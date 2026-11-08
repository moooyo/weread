//! Synchronous, bounded application diagnostics without a producer queue.

use std::{
    fmt,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

pub const MAX_LOG_BYTES: u64 = 1024 * 1024;
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024;
pub const BACKUP_COUNT: usize = 2;

const LOG_NAME: &str = "application.log";
const TRUNCATED: &str = " [truncated]";

static DIAGNOSTICS: OnceLock<Arc<Mutex<Registry>>> = OnceLock::new();

/// Emit one diagnostic using the same formatting syntax as `eprintln!`.
#[macro_export]
macro_rules! diagnostic {
    () => {
        $crate::diagnostics::emit(format_args!(""))
    };
    ($($argument:tt)+) => {
        $crate::diagnostics::emit(format_args!($($argument)+))
    };
}

enum Output {
    Stderr,
    Rolling(RollingSink),
    Disabled,
}

struct Registry {
    generation: u64,
    output: Output,
    warned: bool,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            generation: 0,
            output: Output::Stderr,
            warned: false,
        }
    }
}

impl Registry {
    fn disable(&mut self) -> bool {
        // Replacing the variant closes the file, including after a partial write.
        self.output = Output::Disabled;
        !std::mem::replace(&mut self.warned, true)
    }

    fn emit(&mut self, arguments: fmt::Arguments<'_>) -> Option<io::Error> {
        let result = match &mut self.output {
            Output::Stderr => {
                let mut stderr = io::stderr().lock();
                let _ = stderr.write_fmt(arguments);
                let _ = stderr.write_all(b"\n");
                return None;
            }
            Output::Disabled => return None,
            Output::Rolling(sink) => sink.append(&bounded_message(arguments)),
        };
        if let Err(error) = result {
            if self.disable() {
                return Some(error);
            }
        }
        None
    }
}

/// Owns the current configuration; an older guard cannot revoke a replacement.
/// Keep this guard alive through the application's final diagnostic.
#[must_use]
pub struct DiagnosticGuard {
    registry: Arc<Mutex<Registry>>,
    generation: u64,
}

impl Drop for DiagnosticGuard {
    fn drop(&mut self) {
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if registry.generation == self.generation {
            registry.output = Output::Stderr;
        }
    }
}

fn global_registry() -> &'static Arc<Mutex<Registry>> {
    DIAGNOSTICS.get_or_init(|| Arc::new(Mutex::new(Registry::default())))
}

/// Replace the process-local sink with `application.log` and two fixed backups.
/// The parent directory is created privately when absent. Existing symlinks,
/// non-regular files, and paths containing parent traversal are rejected.
/// A configuration failure disables output until a later configuration succeeds.
pub fn configure(path: impl AsRef<Path>) -> io::Result<DiagnosticGuard> {
    configure_registry(global_registry().clone(), path.as_ref(), MAX_LOG_BYTES)
}

fn configure_registry(
    registry: Arc<Mutex<Registry>>,
    path: &Path,
    maximum_bytes: u64,
) -> io::Result<DiagnosticGuard> {
    let mut state = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.generation = state.generation.wrapping_add(1);
    let generation = state.generation;
    // Close a replaced file before inspecting or rotating its names.
    state.output = Output::Disabled;
    match RollingSink::open(path, maximum_bytes) {
        Ok(sink) => {
            state.output = Output::Rolling(sink);
            drop(state);
            Ok(DiagnosticGuard {
                registry,
                generation,
            })
        }
        Err(error) => {
            let warn = state.disable();
            drop(state);
            if warn {
                warn_once(&error);
            }
            Err(error)
        }
    }
}

/// Write one diagnostic immediately. Unconfigured desktop callers retain stderr.
/// A configured sink failure closes it and suppresses further messages instead
/// of creating a retry queue or flooding the launcher's stderr file.
pub fn emit(arguments: fmt::Arguments<'_>) {
    emit_registry(global_registry(), arguments);
}

fn emit_registry(registry: &Arc<Mutex<Registry>>, arguments: fmt::Arguments<'_>) {
    let failure = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .emit(arguments);
    if let Some(error) = failure {
        warn_once(&error);
    }
}

fn warn_once(error: &io::Error) {
    let _ = writeln!(
        io::stderr().lock(),
        "weread: application diagnostics disabled after an I/O error: {error}"
    );
}

struct BoundedMessage {
    text: String,
    truncated: bool,
}

impl fmt::Write for BoundedMessage {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.truncated {
            return Ok(());
        }
        let remaining = (MAX_MESSAGE_BYTES - 1).saturating_sub(self.text.len());
        let mut take = remaining.min(value.len());
        while !value.is_char_boundary(take) {
            take -= 1;
        }
        self.text.push_str(&value[..take]);
        self.truncated |= take < value.len();
        Ok(())
    }
}

fn bounded_message(arguments: fmt::Arguments<'_>) -> Vec<u8> {
    let mut message = BoundedMessage {
        text: String::with_capacity(MAX_MESSAGE_BYTES),
        truncated: false,
    };
    if fmt::write(&mut message, arguments).is_err() {
        let _ = fmt::Write::write_str(&mut message, " [formatting failed]");
    }
    if message.truncated {
        let mut end = message
            .text
            .len()
            .min(MAX_MESSAGE_BYTES - 1 - TRUNCATED.len());
        while !message.text.is_char_boundary(end) {
            end -= 1;
        }
        message.text.truncate(end);
        message.text.push_str(TRUNCATED);
    }
    message.text.push('\n');
    message.text.into_bytes()
}

struct RollingSink {
    paths: [PathBuf; BACKUP_COUNT + 1],
    file: Option<File>,
    maximum_bytes: u64,
}

impl RollingSink {
    fn open(path: &Path, maximum_bytes: u64) -> io::Result<Self> {
        if maximum_bytes < MAX_MESSAGE_BYTES as u64 {
            return Err(invalid(
                "The diagnostic file limit is smaller than one message",
            ));
        }
        if path.file_name() != Some(std::ffi::OsStr::new(LOG_NAME)) {
            return Err(invalid(
                "The diagnostic log filename must be application.log",
            ));
        }
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let parent = absolute
            .parent()
            .ok_or_else(|| invalid("The diagnostic log has no parent directory"))?;
        check_directory_chain(parent, true)?;
        let parent = parent.canonicalize()?;
        let paths = [
            parent.join(LOG_NAME),
            parent.join(format!("{LOG_NAME}.1")),
            parent.join(format!("{LOG_NAME}.2")),
        ];
        for path in &paths {
            check_file(path)?;
        }
        // Existing logs from older versions must also obey the current budget.
        // Keep only their bounded tail; no temporary payload or extra name is used.
        for path in &paths[1..] {
            if check_file(path)?.is_some() {
                let mut file = open_file(path, false)?;
                cap_existing_file(&mut file, maximum_bytes)?;
            }
        }
        let mut file = open_file(&paths[0], true)?;
        cap_existing_file(&mut file, maximum_bytes)?;
        Ok(Self {
            paths,
            file: Some(file),
            maximum_bytes,
        })
    }

    fn append(&mut self, message: &[u8]) -> io::Result<()> {
        check_directory_chain(self.paths[0].parent().unwrap(), false)?;
        let current = check_file(&self.paths[0])?
            .ok_or_else(|| invalid("The active diagnostic file disappeared"))?;
        let file = self
            .file
            .as_ref()
            .ok_or_else(|| invalid("The diagnostic file is closed"))?;
        check_open_file(file)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let opened = file.metadata()?;
            if current.dev() != opened.dev() || current.ino() != opened.ino() {
                return Err(invalid("The active diagnostic file was replaced"));
            }
        }
        if current.len().saturating_add(message.len() as u64) > self.maximum_bytes {
            self.rotate()?;
        }
        self.file
            .as_mut()
            .ok_or_else(|| invalid("The diagnostic file is closed"))?
            .write_all(message)
    }

    fn rotate(&mut self) -> io::Result<()> {
        drop(self.file.take());
        check_directory_chain(self.paths[0].parent().unwrap(), false)?;
        for path in &self.paths {
            if check_file(path)?.is_some() {
                let mut file = open_file(path, false)?;
                cap_existing_file(&mut file, self.maximum_bytes)?;
            }
        }
        if check_file(&self.paths[2])?.is_some() {
            fs::remove_file(&self.paths[2])?;
        }
        if check_file(&self.paths[1])?.is_some() {
            fs::rename(&self.paths[1], &self.paths[2])?;
        }
        if check_file(&self.paths[0])?.is_some() {
            fs::rename(&self.paths[0], &self.paths[1])?;
        }
        self.file = Some(create_file(&self.paths[0])?);
        Ok(())
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn check_directory_chain(path: &Path, create: bool) -> io::Result<()> {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                return Err(invalid(
                    "Diagnostic paths must not contain parent traversal",
                ));
            }
            Component::CurDir => continue,
            _ => prefix.push(component.as_os_str()),
        }
        match fs::symlink_metadata(&prefix) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(invalid("A diagnostic directory is not a real directory")),
            Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&prefix)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&prefix, fs::Permissions::from_mode(0o700))?;
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn check_metadata(metadata: &Metadata) -> io::Result<()> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid("A diagnostic path is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(invalid("A diagnostic file has multiple hard links"));
        }
    }
    Ok(())
}

fn check_file(path: &Path) -> io::Result<Option<Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            check_metadata(&metadata)?;
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn check_open_file(file: &File) -> io::Result<()> {
    check_metadata(&file.metadata()?)
}

fn open_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    options
}

fn open_file(path: &Path, create: bool) -> io::Result<File> {
    let file = open_options().create(create).open(path)?;
    check_open_file(&file)?;
    Ok(file)
}

fn create_file(path: &Path) -> io::Result<File> {
    let file = open_options().create_new(true).open(path)?;
    check_open_file(&file)?;
    Ok(file)
}

fn cap_existing_file(file: &mut File, maximum_bytes: u64) -> io::Result<()> {
    let size = file.metadata()?.len();
    if size <= maximum_bytes {
        return Ok(());
    }
    file.seek(SeekFrom::Start(size - maximum_bytes))?;
    let mut tail = Vec::with_capacity(maximum_bytes as usize);
    (&mut *file).take(maximum_bytes).read_to_end(&mut tail)?;
    // A previous UTF-8 message may straddle the retained tail's first byte.
    let start = tail
        .iter()
        .position(|byte| byte & 0b1100_0000 != 0b1000_0000)
        .unwrap_or(tail.len());
    file.set_len(0)?;
    file.write_all(&tail[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_LIMIT: u64 = 2 * MAX_MESSAGE_BYTES as u64;

    fn registry() -> Arc<Mutex<Registry>> {
        Arc::new(Mutex::new(Registry::default()))
    }

    fn assert_budget(root: &Path, maximum_bytes: u64) {
        let entries = fs::read_dir(root)
            .unwrap()
            .collect::<io::Result<Vec<_>>>()
            .unwrap();
        assert!(entries.len() <= BACKUP_COUNT + 1);
        let mut bytes = 0;
        for entry in entries {
            let metadata = entry.metadata().unwrap();
            assert!(metadata.is_file());
            assert!(metadata.len() <= maximum_bytes);
            bytes += metadata.len();
            let text = fs::read_to_string(entry.path()).unwrap();
            assert!(text.ends_with('\n'));
        }
        assert!(bytes <= (BACKUP_COUNT as u64 + 1) * maximum_bytes);
    }

    #[test]
    fn repeated_unicode_messages_rotate_with_fixed_file_and_byte_budgets() {
        let directory = tempfile::tempdir().unwrap();
        let state = registry();
        let guard = configure_registry(state.clone(), &directory.path().join(LOG_NAME), TEST_LIMIT)
            .unwrap();
        let value = "\u{1f4d6}".repeat(MAX_MESSAGE_BYTES);
        for index in 0..128 {
            emit_registry(&state, format_args!("event-{index}: {value}"));
            assert_budget(directory.path(), TEST_LIMIT);
        }
        let current = fs::read_to_string(directory.path().join(LOG_NAME)).unwrap();
        assert!(current.contains("event-127:"));
        assert!(current.contains(TRUNCATED));
        drop(guard);
    }

    #[test]
    fn formatting_is_bounded_even_across_multiple_unicode_fragments() {
        let value = "\u{4e66}".repeat(MAX_MESSAGE_BYTES);
        let message = bounded_message(format_args!("prefix:{value}:{value}:suffix"));
        assert!(message.len() <= MAX_MESSAGE_BYTES);
        let text = std::str::from_utf8(&message).unwrap();
        assert!(text.starts_with("prefix:"));
        assert!(text.ends_with(&format!("{TRUNCATED}\n")));
    }

    #[test]
    fn existing_oversized_fixed_logs_are_capped_before_new_messages() {
        let directory = tempfile::tempdir().unwrap();
        for suffix in ["", ".1", ".2"] {
            let mut value = vec![b'x'; TEST_LIMIT as usize * 4];
            value.push(b'\n');
            fs::write(directory.path().join(format!("{LOG_NAME}{suffix}")), value).unwrap();
        }
        let state = registry();
        let _guard =
            configure_registry(state.clone(), &directory.path().join(LOG_NAME), TEST_LIMIT)
                .unwrap();
        assert_budget(directory.path(), TEST_LIMIT);
        emit_registry(&state, format_args!("new message"));
        assert_budget(directory.path(), TEST_LIMIT);
    }

    #[test]
    fn write_failure_closes_the_sink_and_requests_only_one_warning() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(LOG_NAME);
        let state = registry();
        let _guard = configure_registry(state.clone(), &path, TEST_LIMIT).unwrap();
        {
            let mut state = state.lock().unwrap();
            let Output::Rolling(sink) = &mut state.output else {
                panic!("The fixture did not configure a rolling sink");
            };
            // A read-only FD has the correct identity but write_all must fail.
            sink.file = Some(File::open(&path).unwrap());
        }
        let mut warnings = 0;
        for _ in 0..1024 {
            warnings += usize::from(
                state
                    .lock()
                    .unwrap()
                    .emit(format_args!("repeated I/O failure"))
                    .is_some(),
            );
        }
        assert_eq!(warnings, 1);
        assert!(matches!(state.lock().unwrap().output, Output::Disabled));
        assert_eq!(fs::metadata(path).unwrap().len(), 0);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn initial_configuration_failure_stays_disabled_without_a_guard() {
        struct MustNotFormat;

        impl fmt::Display for MustNotFormat {
            fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
                panic!("A disabled sink must not format or forward more messages");
            }
        }

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(LOG_NAME);
        fs::create_dir(&path).unwrap();
        let state = registry();
        assert!(configure_registry(state.clone(), &path, TEST_LIMIT).is_err());
        assert!(state.lock().unwrap().warned);
        for _ in 0..1024 {
            assert!(
                state
                    .lock()
                    .unwrap()
                    .emit(format_args!("{MustNotFormat}"))
                    .is_none()
            );
        }
        assert!(matches!(state.lock().unwrap().output, Output::Disabled));
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        // A deliberate later configuration can recover; automatic emit cannot.
        fs::remove_dir(&path).unwrap();
        let guard = configure_registry(state.clone(), &path, TEST_LIMIT).unwrap();
        emit_registry(&state, format_args!("recovered configuration"));
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "recovered configuration\n"
        );
        drop(guard);
    }

    #[test]
    fn replacement_guard_cannot_be_revoked_by_an_older_guard() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let state = registry();
        let first_guard =
            configure_registry(state.clone(), &first.path().join(LOG_NAME), TEST_LIMIT).unwrap();
        emit_registry(&state, format_args!("first"));
        let second_guard =
            configure_registry(state.clone(), &second.path().join(LOG_NAME), TEST_LIMIT).unwrap();
        drop(first_guard);
        emit_registry(&state, format_args!("second"));
        assert_eq!(
            fs::read_to_string(first.path().join(LOG_NAME)).unwrap(),
            "first\n"
        );
        assert_eq!(
            fs::read_to_string(second.path().join(LOG_NAME)).unwrap(),
            "second\n"
        );
        drop(second_guard);
        assert!(matches!(state.lock().unwrap().output, Output::Stderr));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn guard_drop_and_reconfiguration_close_the_owned_file_descriptor() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(LOG_NAME);
        let state = registry();
        let references = || {
            fs::read_dir("/proc/self/fd")
                .unwrap()
                .filter_map(Result::ok)
                .filter_map(|entry| fs::read_link(entry.path()).ok())
                .filter(|target| target == &path)
                .count()
        };
        assert_eq!(references(), 0);
        for _ in 0..64 {
            let guard = configure_registry(state.clone(), &path, TEST_LIMIT).unwrap();
            assert_eq!(references(), 1);
            emit_registry(&state, format_args!("bounded descriptor lifetime"));
            drop(guard);
            assert_eq!(references(), 0);
        }
    }

    #[test]
    fn rotation_failure_preserves_unknown_files_and_disables_future_writes() {
        let directory = tempfile::tempdir().unwrap();
        let state = registry();
        let _guard =
            configure_registry(state.clone(), &directory.path().join(LOG_NAME), TEST_LIMIT)
                .unwrap();
        fs::create_dir(directory.path().join(format!("{LOG_NAME}.2"))).unwrap();
        fs::write(directory.path().join("unknown.txt"), b"preserve this file").unwrap();
        let value = "x".repeat(MAX_MESSAGE_BYTES);
        let mut warnings = 0;
        for _ in 0..128 {
            warnings += usize::from(
                state
                    .lock()
                    .unwrap()
                    .emit(format_args!("{value}"))
                    .is_some(),
            );
        }
        assert_eq!(warnings, 1);
        assert!(matches!(state.lock().unwrap().output, Output::Disabled));
        assert!(directory.path().join(format!("{LOG_NAME}.2")).is_dir());
        assert_eq!(
            fs::read(directory.path().join("unknown.txt")).unwrap(),
            b"preserve this file"
        );
        assert!(fs::metadata(directory.path().join(LOG_NAME)).unwrap().len() <= TEST_LIMIT);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 3);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_hardlink_targets_are_rejected_without_touching_other_data() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("unrelated.txt");
        fs::write(&target, b"unrelated data").unwrap();
        let path = directory.path().join(LOG_NAME);
        symlink(&target, &path).unwrap();
        assert!(RollingSink::open(&path, TEST_LIMIT).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"unrelated data");
        fs::remove_file(&path).unwrap();
        fs::hard_link(&target, &path).unwrap();
        assert!(RollingSink::open(&path, TEST_LIMIT).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"unrelated data");

        let linked_parent = directory.path().join("linked-directory");
        symlink(outside.path(), &linked_parent).unwrap();
        assert!(RollingSink::open(&linked_parent.join(LOG_NAME), TEST_LIMIT).is_err());
        assert!(!outside.path().join(LOG_NAME).exists());
    }

    #[test]
    fn concurrent_producers_use_one_bounded_sink_without_a_queue() {
        let directory = tempfile::tempdir().unwrap();
        let state = registry();
        let _guard =
            configure_registry(state.clone(), &directory.path().join(LOG_NAME), TEST_LIMIT)
                .unwrap();
        let mut threads = Vec::new();
        for producer in 0..4 {
            let state = state.clone();
            threads.push(std::thread::spawn(move || {
                let value = "x".repeat(1024);
                for sequence in 0..64 {
                    emit_registry(&state, format_args!("{producer}:{sequence}:{value}"));
                }
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        assert_budget(directory.path(), TEST_LIMIT);
    }
}
