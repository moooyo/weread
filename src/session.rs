use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};

use crate::source::AuthSession;

const MAX_ACCOUNT_FILE_BYTES: u64 = AuthSession::MAX_ENCODED_BYTES as u64;

pub struct SessionFile {
    path: PathBuf,
}

impl SessionFile {
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join("account.json"),
        }
    }

    pub fn load(&self) -> Result<Option<AuthSession>> {
        let Some(metadata) = regular_account_path(&self.path)? else {
            return Ok(None);
        };
        validate_account_file(&metadata)?;
        let mut options = fs::OpenOptions::new();
        options.read(true);
        protect_open(&mut options);
        let file = options
            .open(&self.path)
            .context("Open local account material")?;
        validate_account_file(&file.metadata()?)?;
        let mut bytes = Vec::new();
        file.take(MAX_ACCOUNT_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .context("Read local account material")?;
        ensure!(
            bytes.len() as u64 <= MAX_ACCOUNT_FILE_BYTES,
            "Local account material exceeds its file budget"
        );
        // Serde type errors can embed string values from credential fields.
        // Keep the failure, but never retain the original value-bearing cause.
        let session: AuthSession = serde_json::from_slice(&bytes).map_err(|_| {
            anyhow::anyhow!("Decode local account material: invalid account format")
        })?;
        validate_session(&session)?;
        Ok(Some(session))
    }

    pub fn save(&self, session: &AuthSession) -> Result<()> {
        validate_session(session)?;
        let parent = self.path.parent().context("Account directory")?;
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(error) = fs::set_permissions(parent, fs::Permissions::from_mode(0o700)) {
                if !permission_mode_unavailable(parent, &error) {
                    return Err(error.into());
                }
            }
        }
        regular_account_path(&self.path)?;
        let temporary = self.path.with_extension("pending");
        let mut pending = PendingAccount::create(temporary)?;
        let result = pending
            .write(session)
            .and_then(|()| pending.commit(&self.path));
        if let Err(error) = result {
            if let Err(cleanup) = pending.abort() {
                return Err(error.context(format!(
                    "Clean local account temporary material failed: {cleanup}"
                )));
            }
            return Err(error);
        }
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        for path in [&self.path, &self.path.with_extension("pending")] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}

fn validate_account_file(metadata: &fs::Metadata) -> Result<()> {
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "Local account material is not a regular file"
    );
    ensure!(
        metadata.len() <= MAX_ACCOUNT_FILE_BYTES,
        "Local account material exceeds its file budget"
    );
    Ok(())
}

fn regular_account_path(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "Local account path is not a regular file"
            );
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn protect_open(options: &mut fs::OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Inspect a reparse point itself instead of following it if a path
        // changes after the pre-open no-symlink check.
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    #[cfg(not(any(unix, windows)))]
    let _ = options;
}

fn validate_session(session: &AuthSession) -> Result<()> {
    session.validate_resource_budget()?;
    Ok(())
}

struct BoundedWriter<W> {
    inner: W,
    written: u64,
    limit: u64,
}

impl<W> BoundedWriter<W> {
    fn new(inner: W, limit: u64) -> Self {
        Self {
            inner,
            written: 0,
            limit,
        }
    }
}

impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.limit.saturating_sub(self.written) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Local account material exceeds its encoded file budget",
            ));
        }
        let count = self.inner.write(bytes)?;
        self.written += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A fixed temporary slot belongs to the single application owner. Remove its
/// old regular directory entry before creating a new file, so even a hard-link
/// alias cannot truncate a published account. Directories and symlinks remain.
struct PendingAccount {
    path: PathBuf,
    file: Option<File>,
    armed: bool,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl PendingAccount {
    fn create(path: PathBuf) -> Result<Self> {
        if regular_account_path(&path)?.is_some() {
            fs::remove_file(&path).context("Reclaim local account temporary material")?;
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        protect_open(&mut options);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&path)
            .context("Create local account temporary material")?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "Local account temporary material is not a regular file"
        );
        validate_pending_identity(&metadata)?;
        #[cfg(unix)]
        let identity = pending_file_identity(&file)?;
        Ok(Self {
            path,
            file: Some(file),
            armed: true,
            #[cfg(unix)]
            identity,
        })
    }

    fn write(&mut self, session: &AuthSession) -> Result<()> {
        let file = self
            .file
            .as_mut()
            .context("Account temporary file is closed")?;
        let mut writer = BoundedWriter::new(&mut *file, MAX_ACCOUNT_FILE_BYTES);
        serde_json::to_writer(&mut writer, session).context("Write local account material")?;
        writer.flush()?;
        file.sync_all()
            .context("Synchronize local account material")?;
        Ok(())
    }

    fn commit(&mut self, destination: &Path) -> Result<()> {
        regular_account_path(destination)?;
        drop(self.file.take());
        // rename replaces an existing regular file on both Unix and Windows.
        // Never remove the old account before the replacement has succeeded.
        fs::rename(&self.path, destination).context("Commit local account material")?;
        self.armed = false;
        Ok(())
    }

    fn abort(&mut self) -> io::Result<()> {
        drop(self.file.take());
        if !self.armed {
            return Ok(());
        }
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.armed = false;
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Local account temporary path changed to a non-regular output",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if self.identity != (metadata.dev(), metadata.ino()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Local account temporary path changed ownership",
                ));
            }
        }
        match fs::remove_file(&self.path) {
            Ok(()) => self.armed = false,
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.armed = false,
            Err(error) => return Err(error),
        }
        Ok(())
    }
}

fn validate_pending_identity(metadata: &fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.nlink() == 1,
            "Local account temporary material has multiple hard links"
        );
    }
    #[cfg(not(unix))]
    let _ = metadata;
    Ok(())
}

#[cfg(unix)]
fn pending_file_identity(file: &File) -> Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    validate_pending_identity(&metadata)?;
    Ok((metadata.dev(), metadata.ino()))
}

impl Drop for PendingAccount {
    fn drop(&mut self) {
        let _ = self.abort();
    }
}

#[cfg(unix)]
fn permission_mode_unavailable(path: &Path, error: &std::io::Error) -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        let mut information = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // FAT storage on Kindle cannot enforce POSIX permission bits. Other
        // filesystems still fail closed when private-directory setup fails.
        let result = unsafe { libc::statfs(path.as_ptr(), information.as_mut_ptr()) };
        if result != 0 {
            return false;
        }
        let magic = unsafe { information.assume_init() }.f_type as u64;
        is_permissionless_fat(magic, error.raw_os_error())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, error);
        false
    }
}

#[cfg(target_os = "linux")]
fn is_permissionless_fat(magic: u64, error: Option<i32>) -> bool {
    matches!(magic, 0x4d44 | 0x2011bab0)
        && error.is_some_and(|code| matches!(code, libc::EPERM | libc::EOPNOTSUPP))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_capacity_refusal_is_typed_and_preserves_the_published_account_and_retry() {
        let directory = tempfile::tempdir().unwrap();
        let file = SessionFile::new(directory.path());
        let original = account();
        file.save(&original).unwrap();
        let bytes = fs::read(&file.path).unwrap();
        let oversized = AuthSession {
            account_name: "\u{1}".repeat(AuthSession::MAX_ENCODED_BYTES / 5),
            ..original.clone()
        };
        let error = file.save(&oversized).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<crate::source::SourceError>(),
            Some(crate::source::SourceError::ResponseTooLarge)
        ));
        assert_eq!(fs::read(&file.path).unwrap(), bytes);
        assert_only_account(directory.path());
        let mut accepted = original;
        accepted.account_name = "A valid retry after capacity refusal".into();
        file.save(&accepted).unwrap();
        assert_eq!(file.load().unwrap(), Some(accepted));
    }

    #[test]
    fn valid_source_admission_does_not_hide_a_real_account_directory_io_failure() {
        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("regular-file-parent");
        fs::write(&parent, b"Preserved regular file").unwrap();
        let file = SessionFile::new(&parent);
        let error = file.save(&account()).unwrap_err();
        assert!(error.downcast_ref::<std::io::Error>().is_some());
        assert!(error.downcast_ref::<crate::source::SourceError>().is_none());
        assert_eq!(fs::read(&parent).unwrap(), b"Preserved regular file");
    }

    fn account() -> AuthSession {
        AuthSession {
            api_key: "fixture-only-secret".into(),
            account_name: "Fixture account".into(),
            ..Default::default()
        }
    }

    fn assert_only_account(directory: &Path) {
        let names = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(names, [std::ffi::OsString::from("account.json")]);
    }

    #[test]
    fn account_material_round_trips_and_is_removed_on_logout() {
        let directory = tempfile::tempdir().unwrap();
        let file = SessionFile::new(directory.path());
        assert!(file.load().unwrap().is_none());
        let account = AuthSession {
            api_key: "fixture-only-secret".into(),
            ..Default::default()
        };
        file.save(&account).unwrap();
        assert_eq!(file.load().unwrap().unwrap().api_key, account.api_key);
        assert!(!format!("{account:?}").contains("fixture-only-secret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&file.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        file.clear().unwrap();
        file.clear().unwrap();
        assert!(file.load().unwrap().is_none());
    }

    #[test]
    fn oversized_existing_account_material_is_rejected_without_modification() {
        let directory = tempfile::tempdir().unwrap();
        let file = SessionFile::new(directory.path());
        let oversized = vec![b' '; MAX_ACCOUNT_FILE_BYTES as usize + 1];
        fs::write(&file.path, &oversized).unwrap();
        let error = file.load().unwrap_err();
        assert!(format!("{error:#}").contains("file budget"));
        assert_eq!(
            fs::metadata(&file.path).unwrap().len(),
            oversized.len() as u64
        );
        assert!(fs::read(&file.path).unwrap() == oversized);
        assert_only_account(directory.path());
    }

    #[test]
    fn loaded_cookie_overflow_is_rejected_without_rewriting_account_material() {
        let directory = tempfile::tempdir().unwrap();
        let file = SessionFile::new(directory.path());
        let mut oversized = account();
        for index in 0..257 {
            oversized
                .cookies
                .insert(format!("fixture_{index}"), "fixture-value".into());
        }
        let bytes = serde_json::to_vec(&oversized).unwrap();
        fs::write(&file.path, &bytes).unwrap();
        let error = file.load().unwrap_err();
        assert!(format!("{error:#}").contains("cookie_budget"));
        assert!(fs::read(&file.path).unwrap() == bytes);
        assert_only_account(directory.path());
    }

    #[test]
    fn malformed_account_types_never_include_credential_values_in_errors() {
        let directory = tempfile::tempdir().unwrap();
        let file = SessionFile::new(directory.path());
        let secret = "fixture-credential-type-confusion-secret";
        for value in [
            serde_json::json!({"cookies": secret}),
            serde_json::json!({"login_time": secret}),
        ] {
            let bytes = serde_json::to_vec(&value).unwrap();
            fs::write(&file.path, &bytes).unwrap();
            let error = file.load().unwrap_err();
            let report = format!("{error:#}");
            assert!(report.contains("invalid account format"));
            assert!(!report.contains(secret));
            assert!(fs::read(&file.path).unwrap() == bytes);
            assert_only_account(directory.path());
        }
    }

    #[test]
    fn repeated_field_and_cookie_overflow_never_changes_the_previous_account() {
        let directory = tempfile::tempdir().unwrap();
        let file = SessionFile::new(directory.path());
        file.save(&account()).unwrap();
        let previous = fs::read(&file.path).unwrap();
        let large = "x".repeat(MAX_ACCOUNT_FILE_BYTES as usize + 1);
        let mut invalid = Vec::new();
        for index in 0..5 {
            let mut session = account();
            match index {
                0 => session.api_key = large.clone(),
                1 => session.account_name = large.clone(),
                2 => session.user_vid = large.clone(),
                3 => session.wr_ticket = large.clone(),
                _ => session.wr_wrpa = large.clone(),
            }
            invalid.push(session);
        }
        let mut aggregate = AuthSession::default();
        aggregate.api_key = "a".repeat(MAX_ACCOUNT_FILE_BYTES as usize / 2 + 1);
        aggregate.account_name = "b".repeat(MAX_ACCOUNT_FILE_BYTES as usize / 2 + 1);
        invalid.push(aggregate);
        let mut cookie = account();
        cookie.cookies.insert("fixture".into(), "x".repeat(8193));
        invalid.push(cookie);
        let mut total_cookies = account();
        for index in 0..9 {
            total_cookies
                .cookies
                .insert(format!("fixture_{index}"), "x".repeat(8000));
        }
        invalid.push(total_cookies);

        for _ in 0..8 {
            for session in &invalid {
                assert!(file.save(session).is_err());
                assert!(fs::read(&file.path).unwrap() == previous);
                assert_only_account(directory.path());
            }
        }
    }

    #[test]
    fn escaped_json_cannot_expand_past_the_encoded_account_budget() {
        let directory = tempfile::tempdir().unwrap();
        let file = SessionFile::new(directory.path());
        file.save(&account()).unwrap();
        let previous = fs::read(&file.path).unwrap();
        let escaped = AuthSession {
            account_name: "\u{0001}".repeat(MAX_ACCOUNT_FILE_BYTES as usize / 2),
            ..Default::default()
        };
        assert!(matches!(
            validate_session(&escaped)
                .unwrap_err()
                .downcast_ref::<crate::source::SourceError>(),
            Some(crate::source::SourceError::ResponseTooLarge)
        ));
        for _ in 0..8 {
            let mut writer = BoundedWriter::new(io::sink(), MAX_ACCOUNT_FILE_BYTES);
            assert!(serde_json::to_writer(&mut writer, &escaped).is_err());
            assert!(writer.written <= MAX_ACCOUNT_FILE_BYTES);
            assert!(file.save(&escaped).is_err());
            assert!(fs::read(&file.path).unwrap() == previous);
            assert_only_account(directory.path());
        }
    }

    #[test]
    fn an_interrupted_regular_pending_slot_does_not_block_the_next_save() {
        let directory = tempfile::tempdir().unwrap();
        let file = SessionFile::new(directory.path());
        file.save(&account()).unwrap();
        fs::write(
            file.path.with_extension("pending"),
            b"interrupted fixture output",
        )
        .unwrap();
        let next = AuthSession {
            account_name: "Replacement account fixture".into(),
            ..account()
        };
        file.save(&next).unwrap();
        assert!(file.load().unwrap() == Some(next));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&file.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_only_account(directory.path());
    }

    #[test]
    fn write_and_rename_failures_reclaim_owned_pending_without_changing_the_account() {
        let directory = tempfile::tempdir().unwrap();
        let file = SessionFile::new(directory.path());
        file.save(&account()).unwrap();
        let previous = fs::read(&file.path).unwrap();
        for _ in 0..8 {
            {
                let mut pending =
                    PendingAccount::create(file.path.with_extension("pending")).unwrap();
                let mut writer = BoundedWriter::new(pending.file.as_mut().unwrap(), 16);
                assert!(serde_json::to_writer(&mut writer, &account()).is_err());
                assert!(writer.written <= 16);
            }
            assert!(fs::read(&file.path).unwrap() == previous);
            assert_only_account(directory.path());
            {
                let mut pending =
                    PendingAccount::create(file.path.with_extension("pending")).unwrap();
                pending.write(&account()).unwrap();
                // This is a real rename error after the owned file was synced.
                // No destination parent exists, and the published account stays intact.
                assert!(
                    pending
                        .commit(&directory.path().join("missing-parent/account.json"))
                        .is_err()
                );
            }
            assert!(fs::read(&file.path).unwrap() == previous);
            assert_only_account(directory.path());
        }
    }

    #[test]
    fn a_pending_directory_is_preserved_instead_of_deleted_or_followed() {
        let directory = tempfile::tempdir().unwrap();
        let file = SessionFile::new(directory.path());
        file.save(&account()).unwrap();
        let previous = fs::read(&file.path).unwrap();
        let pending = file.path.with_extension("pending");
        fs::create_dir(&pending).unwrap();
        let foreign = pending.join("foreign-output");
        fs::write(&foreign, b"fixture output").unwrap();
        for _ in 0..8 {
            assert!(file.save(&account()).is_err());
            assert!(fs::read(&file.path).unwrap() == previous);
            assert_eq!(fs::read(&foreign).unwrap(), b"fixture output");
        }
        assert!(pending.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn a_pending_symlink_never_changes_its_target_or_the_published_account() {
        let directory = tempfile::tempdir().unwrap();
        let file = SessionFile::new(directory.path());
        file.save(&account()).unwrap();
        let previous = fs::read(&file.path).unwrap();
        let target = directory.path().join("foreign-output");
        fs::write(&target, b"fixture output").unwrap();
        let pending = file.path.with_extension("pending");
        std::os::unix::fs::symlink(&target, &pending).unwrap();
        assert!(file.save(&account()).is_err());
        assert!(
            fs::symlink_metadata(&pending)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&target).unwrap(), b"fixture output");
        assert!(fs::read(&file.path).unwrap() == previous);
    }

    #[cfg(unix)]
    #[test]
    fn a_stale_pending_hard_link_never_truncates_the_account_before_commit() {
        let directory = tempfile::tempdir().unwrap();
        let file = SessionFile::new(directory.path());
        file.save(&account()).unwrap();
        let previous = fs::read(&file.path).unwrap();
        let pending = file.path.with_extension("pending");
        fs::hard_link(&file.path, &pending).unwrap();
        let invalid = AuthSession {
            api_key: "x".repeat(MAX_ACCOUNT_FILE_BYTES as usize + 1),
            ..account()
        };
        assert!(file.save(&invalid).is_err());
        assert!(fs::read(&file.path).unwrap() == previous);
        assert!(fs::read(&pending).unwrap() == previous);

        {
            let mut output = PendingAccount::create(pending.clone()).unwrap();
            let mut writer = BoundedWriter::new(output.file.as_mut().unwrap(), 16);
            assert!(serde_json::to_writer(&mut writer, &account()).is_err());
        }
        assert!(fs::read(&file.path).unwrap() == previous);
        assert_only_account(directory.path());

        fs::hard_link(&file.path, &pending).unwrap();
        {
            let mut output = PendingAccount::create(pending.clone()).unwrap();
            output.write(&account()).unwrap();
            assert!(fs::read(&file.path).unwrap() == previous);
            assert!(
                output
                    .commit(&directory.path().join("missing-parent/account.json"))
                    .is_err()
            );
        }
        assert!(fs::read(&file.path).unwrap() == previous);
        assert_only_account(directory.path());

        fs::hard_link(&file.path, &pending).unwrap();
        let next = AuthSession {
            account_name: "Replacement account fixture".into(),
            ..account()
        };
        file.save(&next).unwrap();
        assert!(file.load().unwrap() == Some(next));
        assert_only_account(directory.path());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn only_permissionless_fat_accepts_unsupported_mode_changes() {
        assert!(is_permissionless_fat(0x4d44, Some(libc::EPERM)));
        assert!(is_permissionless_fat(0x2011bab0, Some(libc::EOPNOTSUPP)));
        assert!(!is_permissionless_fat(0xef53, Some(libc::EPERM)));
        assert!(!is_permissionless_fat(0x4d44, Some(libc::EIO)));
        assert!(!is_permissionless_fat(0x4d44, Some(libc::EACCES)));
    }
}
