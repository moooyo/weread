//! Read startup display compensation only from the owning launcher's private records.

#[cfg(target_os = "linux")]
pub(super) fn startup_native_portrait_calibration() -> Option<super::NativePortraitCalibration> {
    let generation = std::env::var("WEREAD_LAUNCH_GENERATION").ok()?;
    let nonce = std::env::var("WEREAD_LAUNCH_TOKEN").ok()?;
    // SAFETY: geteuid has no pointer arguments and only reads the process identity.
    let owner = unsafe { libc::geteuid() };
    linux::read_validated_portrait(
        std::path::Path::new("/tmp/weread-launcher.lock"),
        std::path::Path::new("/proc"),
        &generation,
        &nonce,
        std::process::id(),
        owner,
    )
}

#[cfg(not(target_os = "linux"))]
pub(super) fn startup_native_portrait_calibration() -> Option<super::NativePortraitCalibration> {
    None
}

#[cfg(target_os = "linux")]
pub(super) fn startup_native_output_inverted() -> Option<bool> {
    let generation = std::env::var("WEREAD_LAUNCH_GENERATION").ok()?;
    let nonce = std::env::var("WEREAD_LAUNCH_TOKEN").ok()?;
    // SAFETY: geteuid has no pointer arguments and only reads the process identity.
    let owner = unsafe { libc::geteuid() };
    linux::read_validated_mode(
        std::path::Path::new("/tmp/weread-launcher.lock"),
        std::path::Path::new("/proc"),
        &generation,
        &nonce,
        std::process::id(),
        owner,
    )
}

#[cfg(not(target_os = "linux"))]
pub(super) fn startup_native_output_inverted() -> Option<bool> {
    None
}

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::CString;
    use std::fs::{File, Metadata, OpenOptions};
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::Path;

    const MAX_RECORD_BYTES: usize = 128;
    const MAX_STATUS_BYTES: usize = 64 * 1024;

    pub(super) fn read_validated_mode(
        lock_root: &Path,
        proc_root: &Path,
        generation: &str,
        nonce: &str,
        process: u32,
        owner: u32,
    ) -> Option<bool> {
        read_validated_payload(
            lock_root,
            proc_root,
            generation,
            nonce,
            process,
            owner,
            "native-epdc-mode",
            |record| parse_mode_record(record, generation, nonce),
        )
    }

    pub(super) fn read_validated_portrait(
        lock_root: &Path,
        proc_root: &Path,
        generation: &str,
        nonce: &str,
        process: u32,
        owner: u32,
    ) -> Option<crate::platform::NativePortraitCalibration> {
        read_validated_payload(
            lock_root,
            proc_root,
            generation,
            nonce,
            process,
            owner,
            "native-portrait-calibration",
            |record| {
                let payload = record.strip_prefix(&format!("{generation}:{nonce}:"))?;
                crate::platform::NativePortraitCalibration::parse_payload(payload)
            },
        )
    }

    fn read_validated_payload<T>(
        lock_root: &Path,
        proc_root: &Path,
        generation: &str,
        nonce: &str,
        process: u32,
        owner: u32,
        name: &str,
        parse: impl FnOnce(&str) -> Option<T>,
    ) -> Option<T> {
        if !valid_generation(generation) || !valid_nonce(nonce) || process == 0 {
            return None;
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(lock_root)
            .ok()?;
        let directory_metadata = directory.metadata().ok()?;
        if !private_directory(&directory_metadata, owner)
            || !directory_matches(lock_root, &directory_metadata, owner)
        {
            return None;
        }

        let generation_record = format!("{generation}:{nonce}");
        if read_private_record(&directory, "generation", owner)? != generation_record
            || read_private_record(&directory, "nonce", owner)? != nonce
        {
            return None;
        }
        let master_record = read_private_record(&directory, "pid", owner)?;
        let master = parse_pid(&master_record)?;
        let payload_record = read_private_record(&directory, name, owner)?;
        let payload = parse(&payload_record)?;

        // The application-pid marker is written after spawn and can legitimately
        // be absent during this early read. The kernel's ancestry is authoritative.
        let runner = process_parent(proc_root, process)?;
        if process == runner
            || process == master
            || runner == master
            || process_parent(proc_root, runner)? != master
            || process_parent(proc_root, process)? != runner
        {
            return None;
        }

        // Do not mix records across a new generation or a replaced lock directory.
        if read_private_record(&directory, "generation", owner)? != generation_record
            || read_private_record(&directory, "nonce", owner)? != nonce
            || read_private_record(&directory, "pid", owner)? != master_record
            || read_private_record(&directory, name, owner)? != payload_record
            || !directory_matches(lock_root, &directory_metadata, owner)
        {
            return None;
        }
        Some(payload)
    }

    fn valid_generation(value: &str) -> bool {
        !value.is_empty() && value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit())
    }

    fn valid_nonce(value: &str) -> bool {
        value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }

    fn parse_mode_record(record: &str, generation: &str, nonce: &str) -> Option<bool> {
        let mut fields = record.split(':');
        if fields.next()? != generation || fields.next()? != nonce {
            return None;
        }
        let mode = fields.next()?;
        if fields.next().is_some() {
            return None;
        }
        match mode {
            "Y8" => Some(false),
            "Y8INV" => Some(true),
            _ => None,
        }
    }

    fn private_directory(metadata: &Metadata, owner: u32) -> bool {
        metadata.is_dir() && metadata.uid() == owner && metadata.mode() & 0o077 == 0
    }

    fn private_regular(metadata: &Metadata, owner: u32) -> bool {
        metadata.is_file() && metadata.uid() == owner && metadata.mode() & 0o077 == 0
    }

    fn directory_matches(root: &Path, opened: &Metadata, owner: u32) -> bool {
        std::fs::symlink_metadata(root).is_ok_and(|current| {
            private_directory(&current, owner)
                && current.dev() == opened.dev()
                && current.ino() == opened.ino()
        })
    }

    fn read_private_record(directory: &File, name: &str, owner: u32) -> Option<String> {
        let name = CString::new(name).ok()?;
        // SAFETY: directory owns a valid directory fd, and name is a live,
        // NUL-terminated static leaf name. O_NONBLOCK prevents a FIFO from hanging
        // before its file type can be checked; O_NOFOLLOW rejects symlinks.
        let descriptor = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return None;
        }
        // SAFETY: openat returned a new descriptor which is now owned exactly once.
        let mut file = unsafe { File::from_raw_fd(descriptor) };
        let before = file.metadata().ok()?;
        if !private_regular(&before, owner) || before.len() > MAX_RECORD_BYTES as u64 {
            return None;
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take((MAX_RECORD_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .ok()?;
        let after = file.metadata().ok()?;
        if bytes.len() > MAX_RECORD_BYTES
            || !private_regular(&after, owner)
            || before.len() != after.len()
            || after.len() != bytes.len() as u64
        {
            return None;
        }
        let text = std::str::from_utf8(&bytes).ok()?;
        let text = text.strip_suffix('\n').unwrap_or(text);
        let text = text.strip_suffix('\r').unwrap_or(text);
        if text.is_empty() || text.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
            return None;
        }
        Some(text.to_owned())
    }

    fn parse_pid(value: &str) -> Option<u32> {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        value.parse::<u32>().ok().filter(|pid| *pid != 0)
    }

    fn process_parent(proc_root: &Path, process: u32) -> Option<u32> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(proc_root.join(process.to_string()).join("status"))
            .ok()?;
        if !file.metadata().ok()?.is_file() {
            return None;
        }
        let mut bytes = Vec::new();
        file.take((MAX_STATUS_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > MAX_STATUS_BYTES {
            return None;
        }
        let status = std::str::from_utf8(&bytes).ok()?;
        let mut parents = status.lines().filter_map(|line| line.strip_prefix("PPid:"));
        let parent = parse_pid(parents.next()?.trim())?;
        if parents.next().is_some() {
            return None;
        }
        Some(parent)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::{PermissionsExt, symlink};
        use std::path::PathBuf;

        const GENERATION: &str = "17";
        const NONCE: &str = "0123456789abcdef0123456789abcdef";
        const PROCESS: u32 = 303;
        const RUNNER: u32 = 302;
        const MASTER: u32 = 301;

        struct Fixture {
            _directory: tempfile::TempDir,
            lock: PathBuf,
            proc_root: PathBuf,
            owner: u32,
        }

        impl Fixture {
            fn new() -> Self {
                let directory = tempfile::tempdir().unwrap();
                let lock = directory.path().join("launcher");
                let proc_root = directory.path().join("proc");
                std::fs::create_dir(&lock).unwrap();
                std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o700)).unwrap();
                std::fs::create_dir(&proc_root).unwrap();
                let fixture = Self {
                    _directory: directory,
                    lock,
                    proc_root,
                    // SAFETY: geteuid only reads the test process identity.
                    owner: unsafe { libc::geteuid() },
                };
                fixture.record("generation", &format!("{GENERATION}:{NONCE}\n"));
                fixture.record("nonce", &format!("{NONCE}\n"));
                fixture.record("pid", &format!("{MASTER}\n"));
                fixture.record("native-epdc-mode", &format!("{GENERATION}:{NONCE}:Y8INV\n"));
                fixture.parent(PROCESS, RUNNER);
                fixture.parent(RUNNER, MASTER);
                fixture
            }

            fn record(&self, name: &str, value: &str) {
                let path = self.lock.join(name);
                std::fs::write(&path, value).unwrap();
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }

            fn parent(&self, process: u32, parent: u32) {
                let directory = self.proc_root.join(process.to_string());
                std::fs::create_dir_all(&directory).unwrap();
                std::fs::write(
                    directory.join("status"),
                    format!("Name:\tfixture\nPPid:\t{parent}\n"),
                )
                .unwrap();
            }

            fn read(&self) -> Option<bool> {
                read_validated_mode(
                    &self.lock,
                    &self.proc_root,
                    GENERATION,
                    NONCE,
                    PROCESS,
                    self.owner,
                )
            }
        }

        #[test]
        fn owned_early_startup_accepts_both_modes_without_an_application_pid_marker() {
            let fixture = Fixture::new();
            assert!(!fixture.lock.join("application-pid").exists());
            assert_eq!(fixture.read(), Some(true));
            fixture.record("native-epdc-mode", &format!("{GENERATION}:{NONCE}:Y8\n"));
            assert_eq!(fixture.read(), Some(false));
            // An untrusted or not-yet-written application marker cannot be the proof.
            fixture.record("application-pid", "99999\n");
            assert_eq!(fixture.read(), Some(false));
        }

        #[test]
        fn owned_early_startup_portrait_uses_the_same_private_ancestry_gate() {
            let fixture = Fixture::new();
            fixture.record(
                "native-portrait-calibration",
                &format!("{GENERATION}:{NONCE}:D:1\n"),
            );
            let read = || {
                read_validated_portrait(
                    &fixture.lock,
                    &fixture.proc_root,
                    GENERATION,
                    NONCE,
                    PROCESS,
                    fixture.owner,
                )
            };
            assert_eq!(
                read(),
                Some(crate::platform::NativePortraitCalibration {
                    direction: crate::platform::NativePortraitDirection::Down,
                    framebuffer_rotation: 1
                })
            );
            fixture.parent(RUNNER, MASTER + 1);
            assert_eq!(read(), None);
        }

        #[test]
        fn malformed_stale_or_nonprivate_portrait_records_remain_fixed() {
            let fixture = Fixture::new();
            let read = || {
                read_validated_portrait(
                    &fixture.lock,
                    &fixture.proc_root,
                    GENERATION,
                    NONCE,
                    PROCESS,
                    fixture.owner,
                )
            };
            for record in [
                format!("{GENERATION}:{NONCE}:FIXED:0\n"),
                format!("{GENERATION}:{NONCE}:U:4\n"),
                format!("{GENERATION}:{NONCE}:U:1:extra\n"),
                format!("16:{NONCE}:U:0\n"),
                format!("{GENERATION}:ffffffffffffffffffffffffffffffff:U:0\n"),
            ] {
                fixture.record("native-portrait-calibration", &record);
                assert_eq!(read(), None);
            }
            fixture.record(
                "native-portrait-calibration",
                &format!("{GENERATION}:{NONCE}:U:0\n"),
            );
            std::fs::set_permissions(
                fixture.lock.join("native-portrait-calibration"),
                std::fs::Permissions::from_mode(0o644),
            )
            .unwrap();
            assert_eq!(read(), None);
        }

        #[test]
        fn unsupported_or_ambiguous_modes_and_incomplete_records_remain_unknown() {
            let fixture = Fixture::new();
            for mode in [
                "1",
                "0",
                "RGB32INV",
                "Y8INV:extra",
                "Y8INV extra",
                "y8inv",
                "",
            ] {
                fixture.record(
                    "native-epdc-mode",
                    &format!("{GENERATION}:{NONCE}:{mode}\n"),
                );
                assert_eq!(fixture.read(), None);
            }
            std::fs::remove_file(fixture.lock.join("native-epdc-mode")).unwrap();
            assert_eq!(fixture.read(), None);
        }

        #[test]
        fn generation_nonce_and_record_scope_must_all_match() {
            for name in ["generation", "nonce", "native-epdc-mode"] {
                let fixture = Fixture::new();
                fixture.record(name, "wrong-generation-or-token\n");
                assert_eq!(fixture.read(), None);
            }
            let fixture = Fixture::new();
            assert_eq!(
                read_validated_mode(
                    &fixture.lock,
                    &fixture.proc_root,
                    "18",
                    NONCE,
                    PROCESS,
                    fixture.owner
                ),
                None
            );
            assert_eq!(
                read_validated_mode(
                    &fixture.lock,
                    &fixture.proc_root,
                    GENERATION,
                    "fedcba9876543210fedcba9876543210",
                    PROCESS,
                    fixture.owner
                ),
                None
            );
            assert_eq!(
                read_validated_mode(
                    &fixture.lock,
                    &fixture.proc_root,
                    "",
                    NONCE,
                    PROCESS,
                    fixture.owner
                ),
                None
            );
            assert_eq!(
                read_validated_mode(
                    &fixture.lock,
                    &fixture.proc_root,
                    GENERATION,
                    "not-a-token",
                    PROCESS,
                    fixture.owner
                ),
                None
            );
        }

        #[test]
        fn recorded_master_must_be_the_actual_grandparent() {
            let fixture = Fixture::new();
            fixture.parent(RUNNER, MASTER + 9);
            assert_eq!(fixture.read(), None);
            fixture.parent(RUNNER, MASTER);
            fixture.record("pid", &format!("{RUNNER}\n"));
            assert_eq!(fixture.read(), None);
            fixture.record("pid", &format!("{PROCESS}\n"));
            assert_eq!(fixture.read(), None);
            fixture.record("pid", "0\n");
            assert_eq!(fixture.read(), None);
        }

        #[test]
        fn ambiguous_or_missing_kernel_ancestry_cannot_authorize_compensation() {
            let fixture = Fixture::new();
            let status = fixture.proc_root.join(PROCESS.to_string()).join("status");
            std::fs::write(&status, format!("PPid: {RUNNER}\nPPid: {RUNNER}\n")).unwrap();
            assert_eq!(fixture.read(), None);
            std::fs::write(&status, "Name: fixture\n").unwrap();
            assert_eq!(fixture.read(), None);
            std::fs::remove_file(status).unwrap();
            assert_eq!(fixture.read(), None);
        }

        #[test]
        fn wrong_owner_or_public_permissions_are_rejected() {
            let fixture = Fixture::new();
            assert_eq!(
                read_validated_mode(
                    &fixture.lock,
                    &fixture.proc_root,
                    GENERATION,
                    NONCE,
                    PROCESS,
                    fixture.owner.wrapping_add(1)
                ),
                None
            );
            std::fs::set_permissions(&fixture.lock, std::fs::Permissions::from_mode(0o701))
                .unwrap();
            assert_eq!(fixture.read(), None);
            std::fs::set_permissions(&fixture.lock, std::fs::Permissions::from_mode(0o700))
                .unwrap();
            std::fs::set_permissions(
                fixture.lock.join("nonce"),
                std::fs::Permissions::from_mode(0o644),
            )
            .unwrap();
            assert_eq!(fixture.read(), None);
        }

        #[test]
        fn symbolic_lock_directories_and_record_files_are_rejected() {
            let fixture = Fixture::new();
            let alias = fixture._directory.path().join("launcher-alias");
            symlink(&fixture.lock, &alias).unwrap();
            assert_eq!(
                read_validated_mode(
                    &alias,
                    &fixture.proc_root,
                    GENERATION,
                    NONCE,
                    PROCESS,
                    fixture.owner
                ),
                None
            );
            for name in ["generation", "nonce", "pid", "native-epdc-mode"] {
                let fixture = Fixture::new();
                let path = fixture.lock.join(name);
                let target = fixture._directory.path().join("external-record");
                std::fs::rename(&path, &target).unwrap();
                symlink(&target, &path).unwrap();
                assert_eq!(fixture.read(), None);
            }
        }

        #[test]
        fn oversized_and_non_regular_records_are_rejected_without_reading_them() {
            let fixture = Fixture::new();
            fixture.record("native-epdc-mode", &"x".repeat(MAX_RECORD_BYTES + 1));
            assert_eq!(fixture.read(), None);
            let path = fixture.lock.join("native-epdc-mode");
            std::fs::remove_file(&path).unwrap();
            std::fs::create_dir(&path).unwrap();
            assert_eq!(fixture.read(), None);
        }
    }
}
