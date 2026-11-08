use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    io::Read,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use slint::{ComponentHandle, fontique_010::fontique};
use weread::{
    app::{App, AppOptions, FontRecord},
    i18n,
    platform::{self, DeviceProfile, Platform},
    reader::TextMetrics,
    ui::{AppWindow, ShellModel, Theme},
};

#[path = "main_font_budget.rs"]
mod font_budget;
#[path = "main_glyph_budget.rs"]
mod glyph_budget;
#[path = "main_glyph_cff.rs"]
mod glyph_cff;
use font_budget::{
    FontBudget, FontLimitError, MAX_FAMILY_BYTES, MAX_VARIATION_AXES, font_inventory,
    read_font_payload,
};

#[cfg(test)]
#[path = "main_font_budget_tests.rs"]
mod font_budget_tests;

const SANS_REGULAR: &[u8] = include_bytes!("../assets/fonts/NotoSansSC-Regular.otf.gz");
const SANS_MEDIUM: &[u8] = include_bytes!("../assets/fonts/NotoSansSC-Medium.otf.gz");
const SANS_BOLD: &[u8] = include_bytes!("../assets/fonts/NotoSansSC-Bold.otf.gz");
const SERIF_LIGHT: &[u8] = include_bytes!("../assets/fonts/NotoSerifSC-Light.otf.gz");
const SERIF_REGULAR: &[u8] = include_bytes!("../assets/fonts/NotoSerifSC-Regular.otf.gz");
const SERIF_MEDIUM: &[u8] = include_bytes!("../assets/fonts/NotoSerifSC-Medium.otf.gz");
const SERIF_BOLD: &[u8] = include_bytes!("../assets/fonts/NotoSerifSC-Bold.otf.gz");
const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FAILURE_RECORDS: usize = 32;
const MAX_FAILURE_MESSAGE_BYTES: usize = 8 * 1024;
static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static SUSPEND_REQUESTED: AtomicBool = AtomicBool::new(false);
static RESUME_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct FailureLog {
    records: VecDeque<FailureRecord>,
    omitted_occurrences: u64,
}

struct FailureRecord {
    message: Box<str>,
    fingerprint: [u8; 32],
    occurrences: u64,
}

impl FailureLog {
    fn push(&mut self, mut message: String) {
        const TRUNCATED: &str = " [truncated]";
        let fingerprint: [u8; 32] = Sha256::digest(message.as_bytes()).into();
        if message.len() > MAX_FAILURE_MESSAGE_BYTES {
            let mut end = MAX_FAILURE_MESSAGE_BYTES - TRUNCATED.len();
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
            message.push_str(TRUNCATED);
        }
        // Boxed text discards any spare capacity in the formatted error chain.
        let message = message.into_boxed_str();
        if let Some(previous) = self
            .records
            .back_mut()
            .filter(|entry| entry.fingerprint == fingerprint && entry.message == message)
        {
            previous.occurrences = previous.occurrences.saturating_add(1);
            return;
        }
        if self.records.len() == MAX_FAILURE_RECORDS {
            if let Some(previous) = self.records.pop_front() {
                self.omitted_occurrences = self
                    .omitted_occurrences
                    .saturating_add(previous.occurrences);
            }
        }
        self.records.push_back(FailureRecord {
            message,
            fingerprint,
            occurrences: 1,
        });
    }

    fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    fn summary(&self) -> String {
        let mut lines = Vec::with_capacity(self.records.len() + 1);
        if self.omitted_occurrences > 0 {
            lines.push(format!(
                "{} earlier failure occurrences omitted",
                self.omitted_occurrences
            ));
        }
        for entry in &self.records {
            lines.push(if entry.occurrences == 1 {
                entry.message.to_string()
            } else {
                format!("{} (repeated {} times)", entry.message, entry.occurrences)
            });
        }
        lines.join("\n")
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "weread",
    version,
    about = "Independent Kindle reading application"
)]
struct Arguments {
    /// Select design geometry; this does not assert device compatibility.
    #[arg(long, default_value = "pw6")]
    device: DeviceProfile,

    /// Use the native Kindle backend instead of the desktop backend.
    #[arg(long)]
    kindle: bool,

    /// Emit read-only native portrait capability and framebuffer metadata.
    #[arg(long, requires = "kindle", hide = true)]
    probe_native_portrait_framebuffer: bool,

    /// Store application state here instead of the per-user data directory.
    #[arg(long)]
    data_dir: Option<PathBuf>,

    /// Use clearly labeled, isolated demonstration data without remote writes.
    #[arg(long)]
    demo: bool,

    /// Disable network operations and use only locally available content.
    #[arg(long)]
    offline: bool,

    /// Start at a named screen when its local data is available.
    #[arg(long, default_value = "", value_parser = ["", "login", "library", "search", "detail", "reader", "mp", "stats", "settings", "article"])]
    screen: String,

    /// Save a PNG and geometry metadata for the final window before shutting down.
    #[arg(long)]
    capture: Option<PathBuf>,

    /// Shut down after this many milliseconds in the event loop.
    #[arg(long)]
    exit_after_ms: Option<u64>,

    /// Replay a JSON array of {at_ms, section, action, value} application intents.
    #[arg(long)]
    actions: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptAction {
    at_ms: u64,
    section: String,
    action: String,
    #[serde(default)]
    value: String,
}

#[derive(Serialize)]
struct CaptureMetadata<'a> {
    route: String,
    profile: &'a str,
    backend: &'a str,
    design_logical_size: [u32; 2],
    logical_size: [u32; 2],
    window_size: [u32; 2],
    snapshot_size: [u32; 2],
    scale_factor: f32,
}

#[derive(Clone)]
struct RegisteredFace {
    family: String,
    index: u32,
    weight: f32,
}

struct PreparedFont {
    bytes: Arc<Vec<u8>>,
    faces: Vec<RegisteredFace>,
    metrics: BTreeMap<u32, TextMetrics>,
}

struct BundledFonts {
    sans_regular: Arc<Vec<u8>>,
    sans_medium: Arc<Vec<u8>>,
    sans_bold: Arc<Vec<u8>>,
    serif_light: Arc<Vec<u8>>,
    serif_regular: Arc<Vec<u8>>,
    serif_medium: Arc<Vec<u8>>,
    serif_bold: Arc<Vec<u8>>,
}

impl BundledFonts {
    fn load() -> Result<Self> {
        Ok(Self {
            sans_regular: decompress_font(SANS_REGULAR).context("Load bundled Sans Regular")?,
            sans_medium: decompress_font(SANS_MEDIUM).context("Load bundled Sans Medium")?,
            sans_bold: decompress_font(SANS_BOLD).context("Load bundled Sans Bold")?,
            serif_light: decompress_font(SERIF_LIGHT).context("Load bundled Serif Light")?,
            serif_regular: decompress_font(SERIF_REGULAR).context("Load bundled Serif Regular")?,
            serif_medium: decompress_font(SERIF_MEDIUM).context("Load bundled Serif Medium")?,
            serif_bold: decompress_font(SERIF_BOLD).context("Load bundled Serif Bold")?,
        })
    }
}

fn dispatch_native_portrait_probe(
    arguments: &Arguments,
    probe: impl FnOnce() -> Result<(bool, u32)>,
) -> Option<Result<String>> {
    if !arguments.probe_native_portrait_framebuffer {
        return None;
    }
    Some((|| {
        anyhow::ensure!(
            arguments.kindle,
            "The native portrait probe requires --kindle"
        );
        let (automatic, rotation) = probe()?;
        anyhow::ensure!(
            rotation <= 3,
            "The native portrait probe returned an invalid rotation"
        );
        Ok(format!(
            "{}:{rotation}",
            if automatic { "AUTO" } else { "FIXED" }
        ))
    })())
}

fn query_native_portrait_probe() -> Result<(bool, u32)> {
    #[cfg(all(feature = "kindle", target_os = "linux"))]
    {
        Ok(slint_backend_kindle::probe_native_portrait_framebuffer()?)
    }
    #[cfg(not(all(feature = "kindle", target_os = "linux")))]
    {
        bail!("The native portrait probe requires the native Kindle Linux build")
    }
}

fn main() -> std::process::ExitCode {
    let arguments = Arguments::parse();
    // This hidden launcher probe runs before diagnostics, fonts, state, the
    // startup worker, or any framebuffer mapping and touch acquisition.
    if let Some(result) = dispatch_native_portrait_probe(&arguments, query_native_portrait_probe) {
        return match result {
            Ok(output) => {
                println!("{output}");
                std::process::ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("weread: {error:#}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    // Keep diagnostics alive through the final error report. A failed native
    // log configuration disables repeated fallback instead of filling stderr.
    let _diagnostics = if arguments.kindle {
        arguments
            .data_dir
            .clone()
            .map(Ok)
            .unwrap_or_else(default_data_dir)
            .ok()
            .and_then(|path| weread::diagnostics::configure(path.join("logs/application.log")).ok())
    } else {
        None
    };
    match run(arguments) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            weread::diagnostic!("weread: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod native_portrait_probe_tests {
    use super::*;

    #[test]
    fn ordinary_launch_does_not_invoke_the_native_metadata_probe() {
        let arguments = Arguments::try_parse_from(["weread"]).unwrap();
        assert!(
            dispatch_native_portrait_probe(&arguments, || panic!(
                "Ordinary startup must not query native metadata"
            ))
            .is_none()
        );
        assert!(
            Arguments::try_parse_from(["weread", "--probe-native-portrait-framebuffer"]).is_err()
        );
    }

    #[test]
    fn launcher_probe_dispatch_uses_only_its_bounded_native_metadata_result() {
        let arguments = Arguments::try_parse_from([
            "weread",
            "--kindle",
            "--probe-native-portrait-framebuffer",
            "--device",
            "scribe",
        ])
        .unwrap();
        let calls = Cell::new(0);
        let output = dispatch_native_portrait_probe(&arguments, || {
            calls.set(calls.get() + 1);
            Ok((true, 3))
        })
        .unwrap()
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(output, "AUTO:3");
        assert_eq!(
            dispatch_native_portrait_probe(&arguments, || Ok((false, 0)))
                .unwrap()
                .unwrap(),
            "FIXED:0"
        );
        assert!(
            dispatch_native_portrait_probe(&arguments, || Ok((true, 4)))
                .unwrap()
                .is_err()
        );
        assert!(
            dispatch_native_portrait_probe(&arguments, || bail!(
                "Synthetic read-only probe failure"
            ))
            .unwrap()
            .is_err()
        );
    }
}

fn run(arguments: Arguments) -> Result<()> {
    let data_dir = arguments
        .data_dir
        .clone()
        .map(Ok)
        .unwrap_or_else(default_data_dir)?;
    let actions = load_actions(arguments.actions.as_deref())?;
    let deadline = automatic_deadline(&arguments, &actions)?;
    let platform = Arc::new(Platform::new(arguments.device, arguments.kindle)?);
    install_shutdown_signals(arguments.kindle)?;
    let startup_data_dir = if arguments.demo {
        data_dir.join("demo")
    } else {
        data_dir.clone()
    };
    let startup_dark_mode = weread::preferences::startup_dark_mode_hint(&startup_data_dir)
        .unwrap_or_else(|| platform.native_output_inverted().unwrap_or(false));
    let mut startup = platform::StartupScreen::show(&platform, startup_dark_mode)
        .context("Show the native startup screen")?;
    startup.set_stage(platform::StartupStage::Fonts);
    let fonts = BundledFonts::load()?;
    if STOP_REQUESTED.load(Ordering::Relaxed) {
        return Ok(());
    }

    // The raw startup worker owns the framebuffer during font decompression.
    // Join it before the installer changes process environment, then resume it.
    startup.pause().context("Pause native startup feedback")?;
    startup.set_stage(platform::StartupStage::Display);
    #[cfg(all(feature = "kindle", target_os = "linux"))]
    let _kindle_backend = if arguments.kindle {
        Some(platform::install_kindle_backend(
            fonts.sans_regular.as_slice(),
        )?)
    } else {
        None
    };
    #[cfg(all(feature = "kindle", target_os = "linux"))]
    if let Some(backend) = _kindle_backend.as_ref() {
        platform.set_refresh_sink(Arc::new(platform::KindleRefreshSink::new(
            backend.refresh_handle(),
        )))?;
        let render_sink = match platform.native_output_inverted() {
            Some(inverted) => platform::KindleRenderSink::new_with_native_inversion(
                backend.render_handle(),
                inverted,
            ),
            None => platform::KindleRenderSink::new(backend.render_handle()),
        };
        platform.set_render_sink(Arc::new(
            render_sink.with_native_orientation(backend.native_orientation_handle()),
        ))?;
        if let Some(calibration) = platform::startup_native_portrait_calibration() {
            // Optional calibration must not block an otherwise valid startup.
            if let Err(error) = platform.set_native_portrait_calibration(calibration) {
                weread::diagnostic!("Configure native portrait observation failed: {error}");
            }
        }
    }
    #[cfg(not(all(feature = "kindle", target_os = "linux")))]
    if arguments.kindle {
        bail!("The native Kindle backend is unavailable in this build");
    }
    startup.resume().context("Resume native startup feedback")?;

    if !arguments.kindle {
        #[cfg(feature = "desktop")]
        slint::BackendSelector::new()
            .backend_name("winit".into())
            .renderer_name("software".into())
            .select()
            .context("Initialize the desktop rendering backend")?;
        #[cfg(not(feature = "desktop"))]
        bail!("This build requires --kindle; the desktop backend is disabled");
    }

    let ui = AppWindow::new().context("Create the application window")?;
    platform::configure_window(ui.window(), &platform)?;
    let font_records = initialize_fonts(&ui, &fonts, &data_dir)?;
    if STOP_REQUESTED.load(Ordering::Relaxed) {
        return Ok(());
    }
    startup.set_stage(platform::StartupStage::Library);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("weread-network")
        .enable_all()
        .build()
        .context("Create background workers")?;
    App::begin_state_open().context("Mark application state initialization")?;
    let application = App::attach(
        &ui,
        AppOptions {
            data_dir,
            profile: arguments.device,
            demo: arguments.demo,
            offline: arguments.offline,
            initial_screen: if arguments.kindle
                && std::env::var("WEREAD_NATIVE_RETURN").ok().as_deref() == Some("1")
            {
                "native-return".into()
            } else if arguments.kindle
                && std::env::var("WEREAD_RESUME_SESSION").ok().as_deref() == Some("1")
            {
                "resume".into()
            } else {
                arguments.screen
            },
            fonts: font_records,
        },
        platform.clone(),
        &runtime,
    )
    .context("Open application state")?;

    let failures = Rc::new(RefCell::new(FailureLog::default()));
    let quitting = Rc::new(Cell::new(false));
    let close_failures = failures.clone();
    let close_quitting = quitting.clone();
    let close_application = application.clone();
    ui.window().on_close_requested(move || {
        match close_application.borrow_mut().request_exit() {
            Ok(()) => close_quitting.set(true),
            Err(error) => close_failures
                .borrow_mut()
                .push(format!("Close the application failed: {error:#}")),
        }
        // Keep the window alive until its final capture and persistent writes finish.
        slint::CloseRequestResponse::KeepWindowShown
    });

    let timer = slint::Timer::default();
    let timer_application = application.clone();
    let timer_platform = platform.clone();
    let timer_failures = failures.clone();
    let timer_quitting = quitting.clone();
    let timer_ui = ui.as_weak();
    let profile = arguments.device;
    let native = arguments.kindle;
    let fixed_capture_geometry = arguments.capture.is_some();
    let started = Instant::now();
    let mut next_action = 0;
    let mut initialize_geometry = !native;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            if timer_quitting.get() {
                return;
            }
            if STOP_REQUESTED.load(Ordering::Relaxed) {
                request_quit(&timer_quitting, &timer_failures);
                return;
            }
            if native {
                match timer_application.borrow_mut().poll_managed_sleep(
                    SUSPEND_REQUESTED.swap(false, Ordering::AcqRel),
                    RESUME_REQUESTED.swap(false, Ordering::AcqRel),
                ) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => {
                        timer_failures
                            .borrow_mut()
                            .push(format!("Managed sleep handoff failed: {error:#}"));
                        request_quit(&timer_quitting, &timer_failures);
                        return;
                    }
                }
            }
            let Some(ui) = timer_ui.upgrade() else {
                timer_failures
                    .borrow_mut()
                    .push("The application window no longer exists".into());
                request_quit(&timer_quitting, &timer_failures);
                return;
            };
            if !native && (initialize_geometry || fixed_capture_geometry) {
                let expected = expected_pixel_size(ui.window(), &timer_platform);
                let actual = ui.window().size();
                let needs_resize = initialize_geometry
                    || expected.is_ok_and(|(width, height)| {
                        actual.width != width || actual.height != height
                    });
                if needs_resize {
                    if let Err(error) = set_profile_window_size(ui.window(), profile) {
                        timer_failures
                            .borrow_mut()
                            .push(format!("Set the profile geometry failed: {error:#}"));
                        request_quit(&timer_quitting, &timer_failures);
                        return;
                    }
                    initialize_geometry = false;
                }
            }
            let elapsed = started.elapsed();
            while let Some(action) = actions.get(next_action) {
                if elapsed < Duration::from_millis(action.at_ms) {
                    break;
                }
                let result = if action.section == "pointer" {
                    // Pointer dispatch can synchronously invoke application callbacks.
                    dispatch_pointer(ui.window(), &action.action, &action.value)
                } else {
                    timer_application.borrow_mut().dispatch(
                        &action.section,
                        &action.action,
                        &action.value,
                    )
                };
                if let Err(error) = result {
                    timer_failures.borrow_mut().push(format!(
                        "Script action {} at {} ms failed: {error:#}",
                        next_action + 1,
                        action.at_ms,
                    ));
                    request_quit(&timer_quitting, &timer_failures);
                    return;
                }
                next_action += 1;
            }
            if let Err(error) = timer_application.borrow_mut().tick() {
                timer_failures
                    .borrow_mut()
                    .push(format!("Application update failed: {error:#}"));
                request_quit(&timer_quitting, &timer_failures);
                return;
            }
            if STOP_REQUESTED.load(Ordering::Relaxed)
                || deadline.is_some_and(|limit| elapsed >= limit)
            {
                request_quit(&timer_quitting, &timer_failures);
            }
        },
    );

    let loop_result = (|| -> Result<()> {
        // The normal backend opens and writes the panel only after this join.
        startup.finish().context("Finish native startup feedback")?;
        ui.show()?;
        if !native {
            set_profile_window_size(ui.window(), profile)?;
        }
        slint::run_event_loop()?;
        Ok(())
    })();
    timer.stop();
    if let Err(error) = loop_result {
        failures
            .borrow_mut()
            .push(format!("Window event loop failed: {error:#}"));
    }
    if let Err(error) = application.borrow_mut().terminal_shutdown() {
        failures.borrow_mut().push(format!(
            "Complete terminal application cleanup failed: {error:#}"
        ));
    }
    if let Some(path) = arguments.capture.as_deref() {
        if let Err(error) = capture_window(&ui, path, &platform) {
            failures
                .borrow_mut()
                .push(format!("Capture the final window failed: {error:#}"));
        }
    }
    if let Err(error) = ui.hide() {
        failures
            .borrow_mut()
            .push(format!("Close the application window failed: {error}"));
    }
    drop(application);
    runtime.shutdown_timeout(Duration::from_secs(3));

    let messages = failures.borrow();
    if !messages.is_empty() {
        bail!("{}", messages.summary());
    }
    Ok(())
}

fn default_data_dir() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(path) = nonempty_environment_path("APPDATA").filter(|path| path.is_absolute()) {
            return Ok(path.join("weread"));
        }
        if let Some(path) =
            nonempty_environment_path("USERPROFILE").filter(|path| path.is_absolute())
        {
            return Ok(path.join("AppData").join("Roaming").join("weread"));
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(path) =
            nonempty_environment_path("XDG_DATA_HOME").filter(|path| path.is_absolute())
        {
            return Ok(path.join("weread"));
        }
        if let Some(path) = nonempty_environment_path("HOME").filter(|path| path.is_absolute()) {
            return Ok(path.join(".local").join("share").join("weread"));
        }
    }
    bail!("No per-user data directory is available; supply --data-dir explicitly")
}

fn nonempty_environment_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn load_actions(path: Option<&Path>) -> Result<Vec<ScriptAction>> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let mut actions: Vec<ScriptAction> = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("Read actions from {}", path.display()))?,
    )
    .context("Parse the action script")?;
    let sections = [
        "shell",
        "library",
        "detail",
        "login",
        "search",
        "reader",
        "article",
        "auxiliary",
        "pointer",
    ];
    for (index, action) in actions.iter().enumerate() {
        if !sections.contains(&action.section.as_str()) || action.action.trim().is_empty() {
            bail!(
                "Action {} has an unsupported section or an empty action name",
                index + 1
            );
        }
        if action.section == "pointer" {
            if !["tap", "down", "up", "move"].contains(&action.action.as_str()) {
                bail!("Action {} uses an unsupported pointer operation", index + 1);
            }
            pointer_position(&action.value)
                .with_context(|| format!("Action {} has invalid pointer coordinates", index + 1))?;
        }
    }
    actions.sort_by_key(|action| action.at_ms);
    Ok(actions)
}

fn automatic_deadline(arguments: &Arguments, actions: &[ScriptAction]) -> Result<Option<Duration>> {
    let milliseconds = match arguments.exit_after_ms {
        Some(milliseconds) => Some(milliseconds),
        None if arguments.actions.is_some() => Some(
            actions
                .last()
                .map_or(0, |action| action.at_ms)
                .checked_add(750)
                .context("The final scripted action time is too large")?,
        ),
        None if arguments.capture.is_some() => Some(1_200),
        None => None,
    };
    Ok(milliseconds.map(Duration::from_millis))
}

fn decompress_font(bytes: &[u8]) -> Result<Arc<Vec<u8>>> {
    let mut decoded = Vec::new();
    GzDecoder::new(bytes)
        .take(MAX_FONT_BYTES + 1)
        .read_to_end(&mut decoded)?;
    if decoded.len() as u64 > MAX_FONT_BYTES {
        bail!("A bundled font exceeds the font size limit");
    }
    Ok(Arc::new(decoded))
}

fn font_face_count(bytes: &[u8]) -> Result<usize> {
    if bytes.len() as u64 > MAX_FONT_BYTES {
        return Err(FontLimitError::Payload.into());
    }
    if bytes.starts_with(b"ttcf") {
        let count = owned_ttf_parser::fonts_in_collection(bytes)
            .context("The font collection header is incomplete")?;
        usize::try_from(count).context("The font collection count is invalid")
    } else {
        Ok(1)
    }
}

fn prepare_font(bytes: Arc<Vec<u8>>, budget: &mut FontBudget) -> Result<PreparedFont> {
    // Bundled/in-memory sources have already been read or decompressed. File
    // sources use the charged reader and enter the next helper directly.
    budget.record_read(bytes.len())?;
    prepare_font_after_read(bytes, budget)
}

fn prepare_font_after_read(bytes: Arc<Vec<u8>>, budget: &mut FontBudget) -> Result<PreparedFont> {
    let raw_faces = font_face_count(bytes.as_slice())?;
    budget.reserve_font(bytes.len(), raw_faces)?;
    // Both parsers see the same immutable data. Borrowed parsing validates
    // every admitted offset/table without cloning the collection payload.
    for index in 0..raw_faces {
        owned_ttf_parser::Face::parse(bytes.as_slice(), index as u32)
            .map_err(|error| anyhow!("The font face is invalid: {error}"))?;
        admit_font_axes(bytes.as_slice(), index as u32)?;
        admit_font_names(bytes.as_slice(), index as u32, budget)?;
        glyph_budget::admit_font_glyphs(bytes.as_slice(), index as u32, budget)?;
    }
    let mut temporary = fontique::Collection::new(fontique::CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    #[cfg(test)]
    {
        budget.temporary_registrations += 1;
    }
    let registered = temporary.register_fonts(fontique::Blob::new(bytes.clone()), None);
    let mut faces = Vec::new();
    for (family, fonts) in registered {
        let family = temporary
            .family_name(family)
            .context("Registered font has no family name")?;
        if family.len() > MAX_FAMILY_BYTES {
            return Err(FontLimitError::Metadata.into());
        }
        for font in fonts {
            if faces.len() >= raw_faces
                || font.index() as usize >= raw_faces
                || !font.weight().value().is_finite()
            {
                bail!("The font registration metadata is inconsistent");
            }
            faces.push(RegisteredFace {
                family: family.to_owned(),
                index: font.index(),
                weight: font.weight().value(),
            });
        }
    }
    if faces.is_empty() {
        bail!("The font contains no supported font faces");
    }
    // Resolve an existing canonical spelling before registration changes any
    // shared state. The software renderer uses these same family identities.
    {
        let mut shared = slint::fontique_010::shared_collection();
        for face in &mut faces {
            if let Some(id) = shared.family_id(&face.family) {
                let canonical = shared
                    .family_name(id)
                    .context("Known font has no family name")?;
                if canonical.len() > MAX_FAMILY_BYTES {
                    return Err(FontLimitError::Metadata.into());
                }
                face.family = canonical.to_owned();
            }
        }
    }
    let mut metrics = BTreeMap::new();
    for face in &faces {
        if !metrics.contains_key(&face.index) {
            #[cfg(test)]
            {
                budget.metric_copies += 1;
            }
            metrics.insert(
                face.index,
                TextMetrics::from_font(bytes.as_slice(), face.index)
                    .map_err(|error| anyhow!(error))?,
            );
        }
    }
    Ok(PreparedFont {
        bytes,
        faces,
        metrics,
    })
}

fn admit_font_axes(bytes: &[u8], index: u32) -> Result<()> {
    let raw = owned_ttf_parser::RawFace::parse(bytes, index)
        .map_err(|error| anyhow!("The font directory is invalid: {error}"))?;
    // Inspect every raw fvar record, including duplicate tags. An optional
    // parser rejecting a malformed table must not hide another parser's count.
    for record in raw.table_records {
        if record.tag != owned_ttf_parser::Tag::from_bytes(b"fvar") {
            continue;
        }
        let start = record.offset as usize;
        let end = start
            .checked_add(record.length as usize)
            .ok_or(FontLimitError::Metadata)?;
        let table = bytes
            .get(start..end)
            .context("The font variation table is out of bounds")?;
        if let Some(count) = table.get(8..10) {
            let axes = u16::from_be_bytes(count.try_into().unwrap()) as usize;
            if axes > MAX_VARIATION_AXES {
                return Err(FontLimitError::Metadata.into());
            }
        }
    }
    Ok(())
}

fn admit_font_names(bytes: &[u8], index: u32, budget: &mut FontBudget) -> Result<()> {
    // These are logical preflight units, not a bound on parser/raster CPU time.
    // Admit the fixed root and complete directory before touching its records.
    budget.reserve_name_work(32)?;
    #[cfg(test)]
    {
        budget.name_root_headers += 1;
    }
    let raw = owned_ttf_parser::RawFace::parse(bytes, index)
        .map_err(|error| anyhow!("The font directory is invalid: {error}"))?;
    // Every raw row pays for its four words and tag/offset checks.
    let directory_work = usize::from(raw.table_records.len())
        .checked_mul(8)
        .ok_or(FontLimitError::NameWork)?;
    budget.reserve_name_work(directory_work)?;
    for record in raw.table_records {
        #[cfg(test)]
        {
            budget.name_directory_records += 1;
        }
        if record.tag != owned_ttf_parser::Tag::from_bytes(b"name") {
            continue;
        }
        // Include fixed header and span checks before data-dependent searches.
        budget.reserve_name_work(32)?;
        #[cfg(test)]
        {
            budget.name_table_headers += 1;
        }
        let start = record.offset as usize;
        let end = start
            .checked_add(record.length as usize)
            .ok_or(FontLimitError::Metadata)?;
        let table = bytes
            .get(start..end)
            .context("The font name table is out of bounds")?;
        let count = name_word(table, 2)? as usize;
        let string_base = name_word(table, 4)? as usize;
        let records_end = count
            .checked_mul(12)
            .and_then(|bytes| bytes.checked_add(6))
            .ok_or(FontLimitError::Metadata)?;
        let records = table
            .get(6..records_end)
            .context("The font name records are incomplete")?;
        // Each twelve-byte record can be searched for both fallback IDs. Price
        // twelve units per search for field reads/comparisons, even if English
        // ends one search early. Duplicate/overlapping tables and TTC aliases
        // use the same cumulative initialization allowance before any search.
        budget.reserve_name_work(
            records
                .len()
                .checked_mul(2)
                .ok_or(FontLimitError::NameWork)?,
        )?;
        // Match the locked memory scanner's physical-index/language ranking.
        // Actual decoding and the final choice remain Fontique's. Prepay both
        // possible fallback spans since unsupported encodings can fall back.
        let mut spans = Vec::with_capacity(2);
        for name_id in [16, 1] {
            let mut selected = None;
            let mut best_rank = -1;
            for (physical_index, record) in records.chunks_exact(12).enumerate() {
                #[cfg(test)]
                {
                    budget.name_record_visits += 1;
                }
                if name_word(record, 6)? != name_id {
                    continue;
                }
                let language = name_word(record, 4)?;
                let rank = if language == 0x0409 {
                    3
                } else if language == 0 {
                    2
                } else if physical_index == 0 {
                    1
                } else {
                    continue;
                };
                if rank > best_rank {
                    best_rank = rank;
                    selected = Some((
                        name_word(record, 10)? as usize,
                        name_word(record, 8)? as usize,
                    ));
                }
                if language == 0x0409 {
                    break;
                }
            }
            if let Some((offset, length)) = selected {
                let start = string_base
                    .checked_add(offset)
                    .ok_or(FontLimitError::Metadata)?;
                let end = start.checked_add(length).ok_or(FontLimitError::Metadata)?;
                table
                    .get(start..end)
                    .context("The selected font name is out of bounds")?;
                if !spans.contains(&(start, length)) {
                    spans.push((start, length));
                }
            }
        }
        for (_, length) in spans {
            // Conservative UTF-16/Mac/UTF-8 decoding, scratch, normalized name
            // keys and temporary family ownership, before any large String.
            budget.reserve_metadata(length.checked_mul(64).ok_or(FontLimitError::Metadata)?)?;
        }
    }
    Ok(())
}

fn name_word(bytes: &[u8], offset: usize) -> Result<u16> {
    let word = bytes
        .get(offset..offset + 2)
        .context("The font name field is incomplete")?;
    Ok(u16::from_be_bytes(word.try_into().unwrap()))
}

fn publish_font(font: &PreparedFont, budget: &mut FontBudget) {
    #[cfg(test)]
    {
        budget.shared_registrations += 1;
    }
    #[cfg(not(test))]
    let _ = budget;
    // Metadata and metrics have already been validated and admitted. This
    // deterministic, infallible API is this source's first shared registration.
    let mut shared = slint::fontique_010::shared_collection();
    let _ = shared.register_fonts(fontique::Blob::new(font.bytes.clone()), None);
}

fn register_font_data(bytes: Arc<Vec<u8>>, budget: &mut FontBudget) -> Result<PreparedFont> {
    let font = prepare_font(bytes, budget)?;
    publish_font(&font, budget);
    Ok(font)
}

fn register_bundled_weight(
    bytes: Arc<Vec<u8>>,
    family: &str,
    weight: f32,
    budget: &mut FontBudget,
) -> Result<(PreparedFont, RegisteredFace)> {
    let font = prepare_font(bytes, budget)?;
    let face = font
        .faces
        .iter()
        .find(|face| face.family == family && face.weight == weight)
        .cloned()
        .with_context(|| {
            format!(
                "The bundled {family} weight {weight} did not register under its typographic family"
            )
        })?;
    publish_font(&font, budget);
    Ok((font, face))
}

fn registered_metrics(font: &PreparedFont, face: &RegisteredFace) -> Result<TextMetrics> {
    font.metrics
        .get(&face.index)
        .cloned()
        .context("Registered font has no prepared metrics")
}

fn initialize_fonts(
    ui: &AppWindow,
    fonts: &BundledFonts,
    data_dir: &Path,
) -> Result<Vec<FontRecord>> {
    // The software renderer reads this same shared Fontique collection.
    let mut budget = FontBudget::default();
    let sans_font = register_font_data(fonts.sans_regular.clone(), &mut budget)?;
    let serif_font = register_font_data(fonts.serif_regular.clone(), &mut budget)?;
    let sans = sans_font
        .faces
        .first()
        .context("Bundled Sans font has no face")?;
    let serif = serif_font
        .faces
        .first()
        .context("Bundled Serif font has no face")?;
    let (sans_medium_font, sans_medium) =
        register_bundled_weight(fonts.sans_medium.clone(), &sans.family, 500.0, &mut budget)?;
    let (sans_bold_font, sans_bold) =
        register_bundled_weight(fonts.sans_bold.clone(), &sans.family, 700.0, &mut budget)?;
    let (serif_light_font, serif_light) =
        register_bundled_weight(fonts.serif_light.clone(), &serif.family, 300.0, &mut budget)?;
    let (serif_medium_font, serif_medium) = register_bundled_weight(
        fonts.serif_medium.clone(),
        &serif.family,
        500.0,
        &mut budget,
    )?;
    let (serif_bold_font, serif_bold) =
        register_bundled_weight(fonts.serif_bold.clone(), &serif.family, 700.0, &mut budget)?;
    let sans_metrics = registered_metrics(&sans_font, sans)?;
    let serif_metrics = registered_metrics(&serif_font, serif)?;
    ui.global::<Theme>().set_ui_font(sans.family.clone().into());
    ui.global::<Theme>()
        .set_serif_font(serif.family.clone().into());
    let mut records = vec![
        FontRecord {
            id: "song".into(),
            label: i18n::text("song_font"),
            family: serif.family.clone(),
            metrics: serif_metrics.clone(),
            weights: BTreeMap::from([
                (300, registered_metrics(&serif_light_font, &serif_light)?),
                (400, serif_metrics),
                (500, registered_metrics(&serif_medium_font, &serif_medium)?),
                (700, registered_metrics(&serif_bold_font, &serif_bold)?),
            ]),
            available: true,
        },
        FontRecord {
            id: "kai".into(),
            label: i18n::text("kai_font"),
            family: String::new(),
            metrics: TextMetrics::default(),
            weights: BTreeMap::new(),
            available: false,
        },
        FontRecord {
            id: "hei".into(),
            label: i18n::text("hei_font"),
            family: sans.family.clone(),
            metrics: sans_metrics.clone(),
            weights: BTreeMap::from([
                (400, sans_metrics),
                (500, registered_metrics(&sans_medium_font, &sans_medium)?),
                (700, registered_metrics(&sans_bold_font, &sans_bold)?),
            ]),
            available: true,
        },
    ];
    load_custom_fonts(&data_dir.join("fonts"), &mut records, &mut budget)?;
    Ok(records)
}

fn load_custom_fonts(
    directory: &Path,
    records: &mut Vec<FontRecord>,
    budget: &mut FontBudget,
) -> Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    let paths = match font_inventory(directory, budget.limits) {
        Ok(paths) => paths,
        Err(error) => {
            weread::diagnostic!(
                "Skip custom font inventory {}: {error:#}",
                directory.display()
            );
            return Ok(());
        }
    };
    let mut chosen_weights = BTreeMap::new();
    for path in paths {
        if !path.is_file()
            || !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("ttf") || extension.eq_ignore_ascii_case("otf")
                })
        {
            continue;
        }
        if let Err(error) = load_custom_font(&path, records, &mut chosen_weights, budget) {
            weread::diagnostic!("Skip custom font {}: {error:#}", path.display());
        }
    }
    Ok(())
}

fn load_custom_font(
    path: &Path,
    records: &mut Vec<FontRecord>,
    chosen_weights: &mut BTreeMap<String, f32>,
    budget: &mut FontBudget,
) -> Result<()> {
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_FONT_BYTES {
        bail!("Custom fonts must not exceed 64 MiB per file");
    }
    if !budget.can_read_file(metadata.len() as usize) {
        return Err(FontLimitError::Payload.into());
    }
    let bytes = Arc::new(read_font_payload(file, MAX_FONT_BYTES, budget)?);
    let font = prepare_font_after_read(bytes, budget)?;
    let digest = hex::encode(Sha256::digest(font.bytes.as_slice()));
    // Stage the complete file before mutating old selections or the shared
    // registry. TextMetrics clones below only clone their prepared Arc owners.
    budget.reserve_metadata(font_record_copy_bytes(records, chosen_weights)?)?;
    let mut staged_records = records.iter().map(copy_font_record).collect();
    let mut staged_weights = chosen_weights.clone();
    apply_custom_font(&font, &digest, &mut staged_records, &mut staged_weights)?;
    publish_font(&font, budget);
    *records = staged_records;
    *chosen_weights = staged_weights;
    Ok(())
}

fn font_record_copy_bytes(
    records: &[FontRecord],
    chosen_weights: &BTreeMap<String, f32>,
) -> Result<usize> {
    let mut bytes = 0usize;
    for record in records {
        for part in [
            record.id.len(),
            record.label.len(),
            record.family.len(),
            std::mem::size_of::<FontRecord>(),
            record
                .weights
                .len()
                .checked_mul(64)
                .context("Font weight metadata is too large")?,
        ] {
            bytes = bytes.checked_add(part).ok_or(FontLimitError::Metadata)?;
        }
    }
    for family in chosen_weights.keys() {
        bytes = bytes
            .checked_add(family.len())
            .and_then(|bytes| bytes.checked_add(64))
            .ok_or(FontLimitError::Metadata)?;
    }
    Ok(bytes)
}

fn copy_font_record(record: &FontRecord) -> FontRecord {
    FontRecord {
        id: record.id.clone(),
        label: record.label.clone(),
        family: record.family.clone(),
        metrics: record.metrics.clone(),
        weights: record.weights.clone(),
        available: record.available,
    }
}

fn apply_custom_font(
    font: &PreparedFont,
    digest: &str,
    records: &mut Vec<FontRecord>,
    chosen_weights: &mut BTreeMap<String, f32>,
) -> Result<()> {
    let mut faces = font.faces.clone();
    faces.sort_by(|left, right| {
        (left.weight - 400.0)
            .abs()
            .total_cmp(&(right.weight - 400.0).abs())
            .then_with(|| left.index.cmp(&right.index))
    });
    let mut families = BTreeSet::new();
    for face in &faces {
        if !families.insert(face.family.clone()) {
            continue;
        }
        let mut weights = BTreeMap::new();
        for candidate in faces
            .iter()
            .filter(|candidate| candidate.family == face.family)
        {
            let weight = candidate.weight.round() as u16;
            if !weights.contains_key(&weight) {
                weights.insert(weight, registered_metrics(font, candidate)?);
            }
        }
        let existing = records
            .iter()
            .position(|record| record.available && record.family == face.family);
        if let Some(index) = existing {
            records[index].weights.extend(weights);
            if !chosen_weights
                .get(&face.family)
                .is_some_and(|weight| (face.weight - 400.0).abs() < (*weight - 400.0).abs())
            {
                continue;
            }
            weights = records[index].weights.clone();
        }
        let metrics = weights
            .get(&(face.weight.round() as u16))
            .context("The custom font's default face has no metrics")?
            .clone();
        let is_kai = face.family.to_ascii_lowercase().contains("wenkai")
            && !records.iter().any(|record| {
                record.id == "kai" && record.available && record.family != face.family
            });
        chosen_weights.insert(face.family.clone(), face.weight);
        let record = FontRecord {
            id: if is_kai {
                "kai".into()
            } else {
                format!("custom-{}-{}", &digest[..16], face.index)
            },
            label: if is_kai {
                i18n::text("kai_font")
            } else {
                face.family.clone()
            },
            family: face.family.clone(),
            metrics,
            weights,
            available: true,
        };
        if let Some(index) = existing {
            records[index] = record;
        } else if is_kai {
            if let Some(slot) = records.iter_mut().find(|font| font.id == "kai") {
                *slot = record;
            } else {
                records.push(record);
            }
        } else {
            records.push(record);
        }
    }
    Ok(())
}

fn set_profile_window_size(window: &slint::Window, profile: DeviceProfile) -> Result<()> {
    let (width, height) = profile.logical_size();
    let size = slint::LogicalSize::new(width as f32, height as f32);
    window.set_size(size);
    // show() initially evaluates the preferred scene width before native resize completes.
    window.try_dispatch_event(slint::platform::WindowEvent::Resized { size })?;
    Ok(())
}

fn expected_pixel_size(window: &slint::Window, platform: &Platform) -> Result<(u32, u32)> {
    let scale = window.scale_factor();
    if !scale.is_finite() || scale <= 0.0 {
        bail!("The window scale factor is invalid");
    }
    if platform.is_kindle {
        if scale != platform.scale_factor() {
            bail!(
                "The native window scale factor is {scale}; the approved display requires {}",
                platform.scale_factor(),
            );
        }
        return platform
            .native_viewport_size()
            .context("The native display geometry was not approved");
    }
    let (width, height) = platform.logical_viewport_size();
    Ok((
        (width as f32 * scale).round() as u32,
        (height as f32 * scale).round() as u32,
    ))
}

fn pointer_position(value: &str) -> Result<slint::LogicalPosition> {
    let (x, y) = value
        .split_once(',')
        .context("Pointer coordinates must use x,y logical units")?;
    let x = x
        .trim()
        .parse::<f32>()
        .context("Invalid pointer x coordinate")?;
    let y = y
        .trim()
        .parse::<f32>()
        .context("Invalid pointer y coordinate")?;
    if !x.is_finite() || !y.is_finite() {
        bail!("Pointer coordinates must be finite");
    }
    Ok(slint::LogicalPosition::new(x, y))
}

fn dispatch_pointer(window: &slint::Window, action: &str, value: &str) -> Result<()> {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = pointer_position(value)?;
    match action {
        "tap" => {
            window.try_dispatch_event(WindowEvent::PointerMoved { position })?;
            window.try_dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            })?;
            window.try_dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            })?;
        }
        "down" => window.try_dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        })?,
        "up" => window.try_dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        })?,
        "move" => window.try_dispatch_event(WindowEvent::PointerMoved { position })?,
        _ => bail!("Unsupported pointer operation"),
    }
    Ok(())
}

fn capture_window(ui: &AppWindow, path: &Path, platform: &Platform) -> Result<()> {
    let profile = platform.profile;
    if !platform.is_kindle {
        set_profile_window_size(ui.window(), profile)?;
    }
    let expected = expected_pixel_size(ui.window(), platform)?;
    let actual = ui.window().size();
    if (actual.width, actual.height) != expected {
        bail!(
            "Window geometry is {}x{}; profile {} requires {}x{} pixels",
            actual.width,
            actual.height,
            profile.id(),
            expected.0,
            expected.1
        );
    }
    let snapshot = ui.window().take_snapshot()?;
    if (snapshot.width(), snapshot.height()) != expected {
        bail!(
            "Snapshot geometry is {}x{}; profile {} requires {}x{} pixels",
            snapshot.width(),
            snapshot.height(),
            profile.id(),
            expected.0,
            expected.1
        );
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    image::save_buffer_with_format(
        path,
        snapshot.as_bytes(),
        snapshot.width(),
        snapshot.height(),
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .with_context(|| format!("Write screenshot {}", path.display()))?;
    let (design_width, design_height) = profile.logical_size();
    let (width, height) = platform.logical_viewport_size();
    let metadata = CaptureMetadata {
        route: ui.global::<ShellModel>().get_route().to_string(),
        profile: profile.id(),
        backend: if platform.is_kindle {
            "kindle"
        } else {
            "desktop"
        },
        design_logical_size: [design_width, design_height],
        logical_size: [width, height],
        window_size: [actual.width, actual.height],
        snapshot_size: [snapshot.width(), snapshot.height()],
        scale_factor: ui.window().scale_factor(),
    };
    let metadata_path = path.with_extension("metadata.json");
    fs::write(&metadata_path, serde_json::to_vec_pretty(&metadata)?)
        .with_context(|| format!("Write capture geometry {}", metadata_path.display()))
}

fn request_quit(quitting: &Cell<bool>, failures: &RefCell<FailureLog>) {
    if quitting.replace(true) {
        return;
    }
    if let Err(error) = slint::quit_event_loop() {
        failures
            .borrow_mut()
            .push(format!("Stop the event loop failed: {error}"));
    }
}

#[cfg(unix)]
fn install_shutdown_signals(native: bool) -> Result<()> {
    extern "C" fn stop(_: libc::c_int) {
        STOP_REQUESTED.store(true, Ordering::Relaxed);
    }
    extern "C" fn suspend(_: libc::c_int) {
        SUSPEND_REQUESTED.store(true, Ordering::Relaxed);
    }
    extern "C" fn resume(_: libc::c_int) {
        RESUME_REQUESTED.store(true, Ordering::Relaxed);
    }
    // SAFETY: the handler only stores a lock-free atomic flag; it touches no UI state.
    unsafe {
        if libc::signal(libc::SIGINT, stop as *const () as libc::sighandler_t) == libc::SIG_ERR
            || libc::signal(libc::SIGTERM, stop as *const () as libc::sighandler_t) == libc::SIG_ERR
        {
            return Err(std::io::Error::last_os_error())
                .context("Install shutdown signal handlers");
        }
        if native
            && (libc::signal(libc::SIGUSR1, suspend as *const () as libc::sighandler_t)
                == libc::SIG_ERR
                || libc::signal(libc::SIGUSR2, resume as *const () as libc::sighandler_t)
                    == libc::SIG_ERR)
        {
            return Err(std::io::Error::last_os_error())
                .context("Install managed sleep signal handlers");
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn install_shutdown_signals(_native: bool) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod failure_log_tests {
    use super::*;

    #[test]
    fn repeated_failed_close_attempts_keep_one_record_and_the_total_count() {
        let mut failures = FailureLog::default();
        assert!(failures.is_empty());
        for _ in 0..10_000 {
            failures.push("Close the application failed: storage unavailable".into());
        }
        assert_eq!(failures.records.len(), 1);
        assert_eq!(failures.records[0].occurrences, 10_000);
        assert_eq!(failures.omitted_occurrences, 0);
        assert_eq!(
            failures.summary(),
            "Close the application failed: storage unavailable (repeated 10000 times)"
        );
    }

    #[test]
    fn changing_errors_keep_recent_context_and_account_for_evicted_occurrences() {
        let mut failures = FailureLog::default();
        for attempt in 0..10_000 {
            failures.push(format!("Close attempt {attempt} failed"));
            failures.push(format!("Close attempt {attempt} failed"));
        }
        assert_eq!(failures.records.len(), MAX_FAILURE_RECORDS);
        assert_eq!(failures.omitted_occurrences, 19_936);
        assert_eq!(
            failures.records[0].message.as_ref(),
            "Close attempt 9968 failed"
        );
        assert_eq!(
            failures.records[31].message.as_ref(),
            "Close attempt 9999 failed"
        );
        assert!(
            failures
                .summary()
                .starts_with("19936 earlier failure occurrences omitted\n")
        );
        assert!(
            failures
                .summary()
                .ends_with("Close attempt 9999 failed (repeated 2 times)")
        );
    }

    #[test]
    fn oversized_error_chains_cannot_retain_an_unbounded_string_allocation() {
        let mut failures = FailureLog::default();
        failures.push("\u{1f6d1}".repeat(MAX_FAILURE_MESSAGE_BYTES));
        let record = &failures.records[0];
        assert!(record.message.len() <= MAX_FAILURE_MESSAGE_BYTES);
        assert!(record.message.ends_with(" [truncated]"));
        assert!(record.message.starts_with('\u{1f6d1}'));
        failures.push(format!(
            "{}different tail",
            "\u{1f6d1}".repeat(MAX_FAILURE_MESSAGE_BYTES)
        ));
        assert_eq!(failures.records.len(), 2);
        assert_eq!(failures.records[0].message, failures.records[1].message);
        assert_ne!(
            failures.records[0].fingerprint,
            failures.records[1].fingerprint
        );
    }
}
