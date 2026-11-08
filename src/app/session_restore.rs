//! Account-scoped recovery of the visible local Reader after a power cycle.

use super::*;
#[cfg(all(feature = "kindle", target_os = "linux"))]
use anyhow::Context;
use anyhow::ensure;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SESSION_KEY: &str = "reader.resume.v1";

pub(super) fn explicit_native_dark_target(
    demo: bool,
    native_base: Option<bool>,
    desired_dark: bool,
) -> Option<bool> {
    if demo {
        None
    } else {
        native_base.map(|_| desired_dark)
    }
}

pub(super) struct LauncherContext {
    directory: PathBuf,
    generation: String,
    token: String,
}

#[derive(Clone, Copy)]
pub(super) enum ManagedMarker {
    SleepRequest,
    SleepReady,
    WakeRequest,
    WakeReady,
}

impl ManagedMarker {
    fn name(self) -> &'static str {
        match self {
            Self::SleepRequest => "managed-sleep-request",
            Self::SleepReady => "managed-sleep-ready",
            Self::WakeRequest => "managed-wake-request",
            Self::WakeReady => "managed-wake-ready",
        }
    }
}

impl LauncherContext {
    pub(super) fn from_environment() -> Result<Option<Self>> {
        if std::env::var_os("WEREAD_LAUNCH_GENERATION").is_none() {
            return Ok(None);
        }
        Self::from_native_environment().map(Some)
    }

    #[cfg(not(all(feature = "kindle", target_os = "linux")))]
    fn from_native_environment() -> Result<Self> {
        anyhow::bail!("A managed launcher context requires the native Kindle build")
    }

    #[cfg(all(feature = "kindle", target_os = "linux"))]
    fn from_native_environment() -> Result<Self> {
        // The environment selects a generation, never an arbitrary output
        // directory. The private lock and process ancestry prove its owner.
        let directory = PathBuf::from("/tmp/weread-launcher.lock");
        ensure!(
            std::env::var_os("WEREAD_LAUNCH_CONTEXT_DIR").as_deref() == Some(directory.as_os_str()),
            "The launcher context directory does not match the display lock"
        );
        validate_private_directory(&directory)?;
        let generation = std::env::var("WEREAD_LAUNCH_GENERATION")?;
        ensure!(
            !generation.is_empty()
                && generation.len() <= 20
                && generation.bytes().all(|byte| byte.is_ascii_digit()),
            "The launcher generation is invalid"
        );
        let token = std::env::var("WEREAD_LAUNCH_TOKEN")?;
        ensure!(
            token.len() == 32 && token.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "The launcher context token is invalid"
        );
        ensure!(
            read_private_record(&directory.join("nonce"))?.as_deref() == Some(token.as_str()),
            "The launcher context token does not match the display lock"
        );
        let context = Self {
            directory,
            generation,
            token,
        };
        ensure!(
            context.matches("generation")?,
            "The launcher generation is stale"
        );
        let launcher = read_private_record(&context.directory.join("pid"))?
            .context("The launcher owner is missing")?
            .parse::<u32>()?;
        let application = read_private_record(&context.directory.join("application-pid"))?
            .context("The launcher application owner is missing")?
            .parse::<u32>()?;
        ensure!(
            application == std::process::id(),
            "The launcher application owner does not match"
        );
        let runner = process_parent(application)?;
        ensure!(
            process_parent(runner)? == launcher,
            "The launcher process ancestry does not match the display lock"
        );
        Ok(context)
    }

    fn record(&self) -> String {
        format!("{}:{}", self.generation, self.token)
    }

    pub(super) fn matches(&self, name: &str) -> Result<bool> {
        Ok(read_private_record(&self.directory.join(name))?
            .is_some_and(|value| value == self.record()))
    }

    pub(super) fn write(&self, name: &str) -> Result<()> {
        self.write_record(name, &self.record())
    }

    fn write_record(&self, name: &str, value: &str) -> Result<()> {
        use std::io::Write;

        validate_private_directory(&self.directory)?;
        let path = self.directory.join(name);
        let pending = path.with_extension("pending");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&pending)?;
        writeln!(file, "{value}")?;
        file.sync_all()?;
        drop(file);
        #[cfg(windows)]
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        std::fs::rename(pending, path)?;
        Ok(())
    }

    fn validate_managed_owner(&self) -> Result<()> {
        validate_private_directory(&self.directory)?;
        ensure!(
            read_private_record(&self.directory.join("nonce"))?.as_deref()
                == Some(self.token.as_str())
                && self.matches("generation")?,
            "The managed launcher generation or owner has changed"
        );
        let application = read_private_record(&self.directory.join("application-pid"))?
            .ok_or_else(|| anyhow::anyhow!("The managed application owner is missing"))?
            .parse::<u32>()?;
        ensure!(
            application == std::process::id(),
            "The managed application owner does not match"
        );
        #[cfg(all(feature = "kindle", target_os = "linux"))]
        {
            let launcher = read_private_record(&self.directory.join("pid"))?
                .context("The managed launcher owner is missing")?
                .parse::<u32>()?;
            let runner = process_parent(application)?;
            ensure!(
                process_parent(runner)? == launcher,
                "The managed launcher ancestry has changed"
            );
        }
        Ok(())
    }

    pub(super) fn publish_managed_sleep(&self) -> Result<()> {
        self.validate_managed_owner()?;
        self.write("managed-sleep-available")
    }

    pub(super) fn native_output_mode(&self) -> Result<Option<bool>> {
        self.validate_managed_owner()?;
        if self.matches("ordinary-exit")? {
            return Ok(None);
        }
        self.native_mode_record("native-epdc-mode", &format!("{}:", self.record()))
    }

    pub(super) fn managed_native_output_mode(&self, cycle: u64) -> Result<Option<bool>> {
        ensure!(cycle > 0, "The native output cycle must be positive");
        self.validate_managed_owner()?;
        if self.matches("ordinary-exit")?
            || self.managed_cycle(ManagedMarker::WakeRequest)? != Some(cycle)
        {
            return Ok(None);
        }
        self.native_mode_record(
            "managed-native-epdc-mode",
            &format!("{}:{cycle}:", self.record()),
        )
    }

    fn native_mode_record(&self, name: &'static str, prefix: &str) -> Result<Option<bool>> {
        let Some(value) = read_private_record(&self.directory.join(name))? else {
            return Ok(None);
        };
        Ok(match value.strip_prefix(prefix) {
            Some("Y8") => Some(false),
            Some("Y8INV") => Some(true),
            _ => None,
        })
    }

    pub(super) fn managed_native_portrait_calibration(
        &self,
        cycle: u64,
    ) -> Result<Option<crate::platform::NativePortraitCalibration>> {
        ensure!(cycle > 0, "The native portrait cycle must be positive");
        self.validate_managed_owner()?;
        if self.matches("ordinary-exit")?
            || self.managed_cycle(ManagedMarker::WakeRequest)? != Some(cycle)
        {
            return Ok(None);
        }
        let path = self.directory.join("managed-native-portrait-calibration");
        // Optional missing or invalid calibration keeps the previous mapping.
        let Ok(Some(record)) = read_private_record(&path) else {
            return Ok(None);
        };
        let prefix = format!("{}:{cycle}:", self.record());
        let Some(payload) = record.strip_prefix(&prefix) else {
            return Ok(None);
        };
        let Some(calibration) = crate::platform::NativePortraitCalibration::parse_payload(payload)
        else {
            return Ok(None);
        };
        self.validate_managed_owner()?;
        if self.matches("ordinary-exit")?
            || self.managed_cycle(ManagedMarker::WakeRequest)? != Some(cycle)
            || read_private_record(&path).ok().flatten().as_deref() != Some(record.as_str())
        {
            return Ok(None);
        }
        Ok(Some(calibration))
    }

    pub(super) fn native_dark_target(&self) -> Result<Option<bool>> {
        self.validate_managed_owner()?;
        let Some(value) = read_private_record(&self.directory.join("native-dark-target"))? else {
            return Ok(None);
        };
        Ok(match value.strip_prefix(&format!("{}:", self.record())) {
            Some("0") => Some(false),
            Some("1") => Some(true),
            _ => None,
        })
    }

    pub(super) fn mark_explicit_exit(&self, dark_target: Option<bool>) -> Result<()> {
        self.validate_managed_owner()?;
        // A failed previous quit cannot lend its old appearance intent to a new
        // exit whose native base is unknown or whose preference has changed.
        self.remove_native_dark_target()?;
        if let Some(target) = dark_target {
            self.write_record(
                "native-dark-target",
                &format!("{}:{}", self.record(), u8::from(target)),
            )?;
        }
        let result = self.write("ordinary-exit");
        if result.is_err() {
            let _ = self.remove_native_dark_target();
        }
        result
    }

    pub(super) fn clear_explicit_exit(&self) -> Result<()> {
        self.validate_managed_owner()?;
        let ordinary = self.remove_matching("ordinary-exit");
        let native = self.remove_native_dark_target();
        ordinary.and(native)
    }

    fn remove_native_dark_target(&self) -> Result<()> {
        if self.native_dark_target()?.is_some() {
            std::fs::remove_file(self.directory.join("native-dark-target"))?;
        }
        Ok(())
    }

    pub(super) fn managed_cycle(&self, marker: ManagedMarker) -> Result<Option<u64>> {
        self.validate_managed_owner()?;
        let Some(value) = read_private_record(&self.directory.join(marker.name()))? else {
            return Ok(None);
        };
        let prefix = format!("{}:", self.record());
        let Some(cycle) = value.strip_prefix(&prefix) else {
            return Ok(None);
        };
        if cycle.is_empty()
            || cycle.len() > 20
            || !cycle.bytes().all(|byte| byte.is_ascii_digit())
            || cycle.starts_with('0')
        {
            return Ok(None);
        }
        Ok(cycle.parse::<u64>().ok().filter(|cycle| *cycle > 0))
    }

    pub(super) fn write_managed_cycle(&self, marker: ManagedMarker, cycle: u64) -> Result<()> {
        ensure!(cycle > 0, "The managed sleep cycle must be positive");
        self.validate_managed_owner()?;
        let request = match marker {
            ManagedMarker::SleepReady => Some(ManagedMarker::SleepRequest),
            ManagedMarker::WakeReady => Some(ManagedMarker::WakeRequest),
            ManagedMarker::SleepRequest | ManagedMarker::WakeRequest => None,
        };
        if let Some(request) = request {
            ensure!(
                self.managed_cycle(request)? == Some(cycle) && !self.matches("ordinary-exit")?,
                "The managed acknowledgement intent is no longer current"
            );
        }
        self.write_record(marker.name(), &format!("{}:{cycle}", self.record()))
    }

    pub(super) fn acknowledge_managed_sleep(&self, cycle: u64) -> Result<()> {
        ensure!(
            self.managed_cycle(ManagedMarker::SleepRequest)? == Some(cycle)
                && self.matches("sleep-request")?
                && !self.matches("ordinary-exit")?,
            "The managed sleep request is no longer current"
        );
        // The cold-resume fallback requires the same durable save proof as a
        // successful shutdown. It is published only after the backend is paused.
        self.write("resume-ready")?;
        self.write_managed_cycle(ManagedMarker::SleepReady, cycle)
    }

    #[cfg(test)]
    pub(super) fn managed_test_context(directory: PathBuf) -> Result<Self> {
        let context = Self {
            directory,
            generation: "7".into(),
            token: "0123456789abcdef0123456789abcdef".into(),
        };
        context.write_record("nonce", &context.token)?;
        context.write_record("generation", &context.record())?;
        context.write_record("application-pid", &std::process::id().to_string())?;
        #[cfg(all(feature = "kindle", target_os = "linux"))]
        context.write_record(
            "pid",
            &process_parent(process_parent(std::process::id())?)?.to_string(),
        )?;
        Ok(context)
    }

    pub(super) fn remove_matching(&self, name: &str) -> Result<()> {
        validate_private_directory(&self.directory)?;
        if self.matches(name)? {
            std::fs::remove_file(self.directory.join(name))?;
        }
        Ok(())
    }
}

fn validate_private_directory(path: &std::path::Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "The launcher lock is not a directory"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no arguments and does not mutate process state.
        let owner = unsafe { libc::geteuid() };
        ensure!(
            metadata.uid() == owner && metadata.mode() & 0o077 == 0,
            "The launcher lock is not private to this process owner"
        );
    }
    Ok(())
}

fn read_private_record(path: &std::path::Path) -> Result<Option<String>> {
    use std::io::Read;

    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "The launcher record is not a regular file"
    );
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(metadata.len() <= 128, "The launcher record is too large");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no arguments and does not mutate process state.
        let owner = unsafe { libc::geteuid() };
        ensure!(
            metadata.uid() == owner && metadata.mode() & 0o077 == 0,
            "The launcher record is not private to this process owner"
        );
    }
    let mut value = String::new();
    file.read_to_string(&mut value)?;
    Ok(Some(value.trim_end_matches('\n').to_owned()))
}

#[cfg(all(feature = "kindle", target_os = "linux"))]
fn process_parent(pid: u32) -> Result<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status"))?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("PPid:"))
        .context("The launcher process parent is unavailable")?
        .trim()
        .parse()
        .context("The launcher process parent is invalid")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedReaderSession {
    version: u32,
    account_key: String,
    book_id: String,
    chapter_id: String,
    chapter_index: u32,
    anchor: u32,
    body_digest: String,
    position_confirmed: bool,
}

impl SavedReaderSession {
    fn resolve(&self, store: &Store, account_key: &str) -> Result<Option<u32>> {
        if self.version != 1
            || self.account_key != account_key
            || self.book_id.is_empty()
            || self.chapter_id.is_empty()
            || self.book_id.len() > 2048
            || self.chapter_id.len() > 2048
            || self.book_id.chars().any(char::is_control)
            || self.chapter_id.chars().any(char::is_control)
            || !store.book_exists(&self.book_id)?
        {
            return Ok(None);
        }
        // A catalog can move a chapter while preserving its upstream UID.
        // Never reuse an index that now identifies a different chapter.
        let Some(index) = store.chapter_index_by_uid(&self.book_id, &self.chapter_id)? else {
            return Ok(None);
        };
        let Some(body) = store.chapter_body(&self.book_id, index)? else {
            return Ok(None);
        };
        if body.is_empty()
            || self.anchor as usize > body.chars().count()
            || self.body_digest != body_digest(&body)
        {
            return Ok(None);
        }
        Ok(Some(index))
    }
}

impl App {
    /// Call immediately before opening application state. An earlier startup
    /// interruption can retry its original intent without claiming a new save.
    pub fn begin_state_open() -> Result<()> {
        if let Some(context) = LauncherContext::from_environment()? {
            context.write("state-open")?;
        }
        Ok(())
    }

    /// Call only after every shutdown write has succeeded. A zero process exit
    /// alone cannot prove that the current generation saved a resume checkpoint.
    pub(super) fn complete_reader_shutdown(&self) -> Result<()> {
        let Some(context) = LauncherContext::from_environment()? else {
            return Ok(());
        };
        let requested = context.matches("sleep-request")?
            || context.matches("native-wifi-request")?
            || context.matches("native-quick-request")?;
        if requested && !context.matches("ordinary-exit")? {
            context.write("resume-ready")?;
        }
        Ok(())
    }

    /// Explicit UI and window exits win over a simultaneous powerd sleep event.
    /// Signal-driven shutdown must not call this method.
    pub(crate) fn mark_explicit_exit(&self) -> Result<()> {
        if let Some(context) = LauncherContext::from_environment()? {
            let target = explicit_native_dark_target(
                self.demo,
                self.platform.native_output_inverted(),
                self.prefs.dark_mode,
            );
            context.mark_explicit_exit(target)?;
        }
        Ok(())
    }

    pub(super) fn clear_explicit_exit(&self) {
        let result = (|| -> Result<()> {
            if let Some(context) = LauncherContext::from_environment()? {
                context.clear_explicit_exit()?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            crate::diagnostic!("Clear an unsuccessful explicit exit failed: {error:#}");
        }
    }

    /// Save before finishing the Reader and before returning a successful exit.
    /// The application's own database already isolates different data directories.
    pub(super) fn save_reader_session(&self) -> Result<()> {
        let Route::Reader(book_id, chapter_index) = &self.route else {
            self.store.delete_setting(SESSION_KEY)?;
            return Ok(());
        };
        let Some(paginator) = self.reader.paginator.as_ref() else {
            self.store.delete_setting(SESSION_KEY)?;
            return Ok(());
        };
        let Some(chapter_id) = self.store.chapter_source_id(book_id, *chapter_index)? else {
            self.store.delete_setting(SESSION_KEY)?;
            return Ok(());
        };
        if self.reader.book_id != *book_id
            || self.reader.chapter_index != *chapter_index
            || self.reader.body.is_empty()
            || chapter_id.is_empty()
        {
            self.store.delete_setting(SESSION_KEY)?;
            return Ok(());
        }
        let saved = SavedReaderSession {
            version: 1,
            account_key: reader_account_key(self.demo, self.jobs.session().as_ref())?,
            book_id: book_id.clone(),
            chapter_id,
            chapter_index: *chapter_index,
            anchor: paginator.current_page().start,
            body_digest: body_digest(&self.reader.body),
            position_confirmed: self.reader.position_confirmed,
        };
        self.store.set_setting(SESSION_KEY, &saved)
    }

    /// Only a launcher-requested resume calls this method; ordinary starts keep
    /// the configured initial route and never reopen a previous book implicitly.
    pub(super) fn restore_reader_session(&mut self) -> Result<()> {
        let saved = match self.store.setting::<SavedReaderSession>(SESSION_KEY) {
            Ok(Some(saved)) => saved,
            Ok(None) => {
                self.show_toast("resume_session_missing");
                return Ok(());
            }
            Err(error) => {
                crate::diagnostic!("Read saved Reader session failed: {error:#}");
                self.show_toast("resume_session_unavailable");
                return Ok(());
            }
        };
        let owner = reader_account_key(self.demo, self.jobs.session().as_ref())?;
        if saved.account_key != owner {
            self.show_toast("resume_session_account_changed");
            return Ok(());
        }
        let index = match saved.resolve(&self.store, &owner) {
            Ok(Some(index)) => index,
            Ok(None) => {
                self.show_toast("resume_session_unavailable");
                return Ok(());
            }
            Err(error) => {
                crate::diagnostic!("Resolve saved Reader session failed: {error:#}");
                self.show_toast("resume_session_unavailable");
                return Ok(());
            }
        };

        // Resume the exact local anchor without a deferred cloud jump or a new
        // pull-on-open request replacing it during startup. This does not opt
        // an unconfirmed local position into progress writes.
        let fallback = self.route.clone();
        let force_offline = self.force_offline;
        let pull_on_open = self.prefs.pull_on_open;
        self.force_offline = true;
        self.prefs.pull_on_open = !saved.position_confirmed;
        let result = self.load_local_reader(&saved.book_id, index, Some(saved.anchor));
        self.force_offline = force_offline;
        self.prefs.pull_on_open = pull_on_open;
        if let Err(error) = result {
            crate::diagnostic!("Restore saved Reader session failed: {error:#}");
            if let Err(error) = self.finish_reading() {
                crate::diagnostic!("Close an incomplete restored Reader failed: {error:#}");
            }
            self.route = fallback;
            self.show_toast("resume_session_unavailable");
            return Ok(());
        }
        self.history.clear();
        self.refresh_display(RefreshReason::EnterScreen);
        Ok(())
    }
}

pub(super) fn reader_account_key(
    demo: bool,
    session: Option<&source::AuthSession>,
) -> Result<String> {
    if demo {
        return Ok("demo".into());
    }
    let Some(session) = session else {
        return Ok("local".into());
    };
    if let Some(identity) = source::report::account_key(session) {
        return Ok(format!("account:{identity}"));
    }
    // An API-only or otherwise unverified identity can resume only under the
    // same credential context. The saved record never contains credentials.
    let material = serde_json::to_vec(&(
        "weread-reader-resume-v1",
        &session.api_key,
        &session.cookies,
        &session.user_vid,
    ))?;
    Ok(format!("context:{}", hex::encode(Sha256::digest(material))))
}

fn body_digest(body: &str) -> String {
    hex::encode(Sha256::digest(body.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{DeviceControl, PlatformCapabilities, PlatformError};
    use slint::platform::{
        Platform as SlintPlatform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    struct ReaderTestPlatform;

    impl SlintPlatform for ReaderTestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
        }
    }

    struct InterruptibleControl(Arc<AtomicBool>);

    impl DeviceControl for InterruptibleControl {
        fn native_viewport(&self) -> Option<(u32, u32)> {
            assert!(
                !self.0.load(Ordering::Relaxed),
                "fixture Reader loading interrupted"
            );
            Some((1272, 1696))
        }

        fn capabilities(&self) -> PlatformCapabilities {
            PlatformCapabilities::default()
        }

        fn brightness(&self) -> Result<u8, PlatformError> {
            Err(PlatformError::Unsupported("brightness"))
        }

        fn set_brightness(&self, _: u8) -> Result<(), PlatformError> {
            Err(PlatformError::Unsupported("brightness"))
        }

        fn warmth(&self) -> Result<u8, PlatformError> {
            Err(PlatformError::Unsupported("warmth"))
        }

        fn set_warmth(&self, _: u8) -> Result<(), PlatformError> {
            Err(PlatformError::Unsupported("warmth"))
        }

        fn battery_percentage(&self) -> Result<u8, PlatformError> {
            Err(PlatformError::Unsupported("battery"))
        }

        fn set_inverted(&self, _: bool) -> Result<(), PlatformError> {
            Err(PlatformError::Unsupported("hardware_inversion"))
        }

        fn reset_idle_timer(&self) -> Result<(), PlatformError> {
            Err(PlatformError::Unsupported("keep_awake"))
        }
    }

    fn fixture() -> Result<(Store, SavedReaderSession)> {
        let store = Store::open(":memory:")?;
        store.upsert_book(&Book::new("book-1", "Fixture book"))?;
        store.upsert_chapters(&[Chapter {
            id: "chapter-1".into(),
            book_id: "book-1".into(),
            index: 0,
            title: "Fixture chapter".into(),
            cached: true,
            body: Some("A readable fixture chapter.".into()),
            content_status: ContentStatus::Full,
            ..Chapter::default()
        }])?;
        let saved = SavedReaderSession {
            version: 1,
            account_key: "local".into(),
            book_id: "book-1".into(),
            chapter_id: "chapter-1".into(),
            chapter_index: 0,
            anchor: 11,
            body_digest: body_digest("A readable fixture chapter."),
            position_confirmed: false,
        };
        Ok((store, saved))
    }

    #[test]
    fn resume_requires_the_same_account_and_readable_content() -> Result<()> {
        let (store, saved) = fixture()?;
        assert_eq!(saved.resolve(&store, "local")?, Some(0));
        assert_eq!(saved.resolve(&store, "account:another")?, None);
        store.clear_book_cache("book-1")?;
        assert_eq!(saved.resolve(&store, "local")?, None);
        Ok(())
    }

    #[test]
    fn an_anchor_is_not_applied_to_replaced_text_or_another_chapter() -> Result<()> {
        let (store, mut saved) = fixture()?;
        saved.body_digest = body_digest("A different fixture chapter.");
        assert_eq!(saved.resolve(&store, "local")?, None);
        saved.body_digest = body_digest("A readable fixture chapter.");
        saved.chapter_id = "removed-chapter".into();
        assert_eq!(saved.resolve(&store, "local")?, None);
        Ok(())
    }

    #[test]
    fn a_reordered_catalog_restores_by_chapter_uid() -> Result<()> {
        let (store, saved) = fixture()?;
        store.replace_catalog(
            "book-1",
            &[
                Chapter {
                    id: "inserted-chapter".into(),
                    book_id: "book-1".into(),
                    index: 0,
                    title: "Inserted chapter".into(),
                    ..Default::default()
                },
                Chapter {
                    id: "chapter-1".into(),
                    book_id: "book-1".into(),
                    index: 1,
                    title: "Fixture chapter".into(),
                    ..Default::default()
                },
            ],
        )?;
        assert_eq!(saved.chapter_index, 0);
        assert_eq!(saved.resolve(&store, "local")?, Some(1));
        Ok(())
    }

    #[test]
    fn api_only_accounts_and_data_directories_do_not_share_a_resume_record() -> Result<()> {
        let first = source::AuthSession {
            api_key: "fixture-account-a".into(),
            ..Default::default()
        };
        let second = source::AuthSession {
            api_key: "fixture-account-b".into(),
            ..Default::default()
        };
        let first_key = reader_account_key(false, Some(&first))?;
        assert_ne!(first_key, reader_account_key(false, Some(&second))?);
        assert!(!first_key.contains("fixture-account-a"));
        assert_ne!(first_key, reader_account_key(false, None)?);
        let (store, saved) = fixture()?;
        store.set_setting(SESSION_KEY, &saved)?;
        let other_store = Store::open(":memory:")?;
        assert!(
            other_store
                .setting::<SavedReaderSession>(SESSION_KEY)?
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn launcher_markers_are_atomic_and_bound_to_one_generation() -> Result<()> {
        let directory = tempfile::tempdir()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
        }
        let current = LauncherContext {
            directory: directory.path().to_path_buf(),
            generation: "2".into(),
            token: "0123456789abcdef0123456789abcdef".into(),
        };
        let previous = LauncherContext {
            directory: directory.path().to_path_buf(),
            generation: "1".into(),
            token: "0123456789abcdef0123456789abcdef".into(),
        };
        assert!(!current.matches("resume-ready")?);
        previous.write("resume-ready")?;
        assert!(!current.matches("resume-ready")?);
        current.write("resume-ready")?;
        assert!(current.matches("resume-ready")?);
        assert!(!directory.path().join("resume-ready.pending").exists());
        current.write("ordinary-exit")?;
        assert!(current.matches("ordinary-exit")?);
        current.write("state-open")?;
        assert!(current.matches("state-open")?);
        let different_launch = LauncherContext {
            directory: directory.path().to_path_buf(),
            generation: "2".into(),
            token: "fedcba9876543210fedcba9876543210".into(),
        };
        assert!(!different_launch.matches("resume-ready")?);
        Ok(())
    }

    fn pending_survives_local_resume_failures() -> Result<()> {
        slint::platform::set_platform(Box::new(ReaderTestPlatform))?;
        let directory = tempfile::tempdir()?;
        let database_path = directory.path().join("library.sqlite");
        let store = Store::open(&database_path)?;
        store.upsert_book(&Book::new("book-1", "Fixture book"))?;
        store.upsert_chapter(&Chapter {
            id: "chapter-1".into(),
            book_id: "book-1".into(),
            index: 0,
            title: "Fixture chapter".into(),
            cached: true,
            body: Some("A readable fixture chapter.".into()),
            content_status: ContentStatus::Full,
            ..Chapter::default()
        })?;
        let position = source::ReadingPosition {
            chapter_uid: "chapter-1".into(),
            chapter_offset: 37,
            ..Default::default()
        };
        store.set_setting("progress.pending:book-1", &position)?;
        drop(store);
        let database = rusqlite::Connection::open(&database_path)?;
        // A temporary delete-and-restore implementation cannot pass this gate.
        database.execute_batch(
            "CREATE TRIGGER reject_pending_delete BEFORE DELETE ON settings
             WHEN OLD.key = 'progress.pending:book-1'
             BEGIN SELECT RAISE(ABORT, 'unexpected pending deletion'); END;",
        )?;
        let assert_pending = || -> Result<()> {
            let value: String = database.query_row(
                "SELECT value FROM settings WHERE key = 'progress.pending:book-1'",
                [],
                |row| row.get(0),
            )?;
            let restored: source::ReadingPosition = serde_json::from_str(&value)?;
            assert_eq!(restored.chapter_uid, "chapter-1");
            assert_eq!(restored.chapter_offset, 37);
            Ok(())
        };
        let interrupt = Arc::new(AtomicBool::new(false));
        let profile = DeviceProfile::Paperwhite6;
        let platform = Arc::new(Platform::with_control(
            profile,
            true,
            Arc::new(InterruptibleControl(interrupt.clone())),
        ));
        let ui = AppWindow::new()?;
        crate::platform::configure_window(ui.window(), &platform)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let application = App::attach(
            &ui,
            AppOptions {
                data_dir: directory.path().to_path_buf(),
                profile,
                demo: false,
                offline: true,
                initial_screen: "library".into(),
                fonts: vec![],
            },
            platform,
            &runtime,
        )?;
        let error = application
            .borrow_mut()
            .load_local_reader("book-1", 0, Some(11))
            .expect_err("The fixture intentionally has no registered reading font");
        assert!(format!("{error:#}").contains("No registered reading font"));
        assert_pending()?;

        // Admit the real Reader layout before testing a later SQL fault. The
        // no-font failure above must never consume the pending position.
        application.borrow_mut().fonts.push(crate::app::FontRecord {
            id: "song".into(),
            label: "Pending-position fixture font".into(),
            family: "sans-serif".into(),
            metrics: crate::reader::TextMetrics::default(),
            weights: BTreeMap::new(),
            available: true,
        });
        application
            .borrow_mut()
            .load_local_reader("book-1", 0, Some(11))?;
        assert!(application.borrow().reader.paginator.is_some());
        assert_pending()?;

        // Existing callers still apply a pending cloud position normally.
        let error = application
            .borrow_mut()
            .load_reader("book-1", 0, Some(11))
            .expect_err("The ordinary load must reach the pending-delete trigger");
        assert!(error.downcast_ref::<rusqlite::Error>().is_some());
        assert!(format!("{error:#}").contains("unexpected pending deletion"));
        assert_pending()?;

        // Interrupt before pagination without running a Result cleanup path.
        // The independent SQLite connection must still observe the original row.
        interrupt.store(true, Ordering::Relaxed);
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            application
                .borrow_mut()
                .load_local_reader("book-1", 0, Some(11))
        }));
        interrupt.store(false, Ordering::Relaxed);
        let panic = interrupted.expect_err("The actual viewport interruption must unwind");
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or_default();
        assert!(message.contains("fixture Reader loading interrupted"));
        assert_pending()?;
        drop(application);
        assert_pending()?;
        drop(ui);
        runtime.shutdown_timeout(Duration::from_secs(1));
        Ok(())
    }

    #[test]
    fn local_resume_preserves_pending_during_failure_and_interruption() -> Result<()> {
        std::thread::spawn(pending_survives_local_resume_failures)
            .join()
            .expect("The pending-position fixture thread panicked")
    }
}
