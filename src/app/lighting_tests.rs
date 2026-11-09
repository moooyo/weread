use super::*;
use crate::platform::{DeviceControl, PlatformCapabilities, RenderSink};
use slint::{
    EventLoopError,
    platform::{
        EventLoopProxy, Platform as SlintPlatform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
};

#[derive(Default)]
struct UiLoop {
    reject_quit: AtomicBool,
    quit_attempts: AtomicUsize,
}

struct TestUi(Arc<UiLoop>);
struct Proxy(Arc<UiLoop>);

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> std::result::Result<(), EventLoopError> {
        self.0.quit_attempts.fetch_add(1, Ordering::AcqRel);
        if self.0.reject_quit.load(Ordering::Acquire) {
            Err(EventLoopError::EventLoopTerminated)
        } else {
            Ok(())
        }
    }

    fn invoke_from_event_loop(
        &self,
        _: Box<dyn FnOnce() + Send>,
    ) -> std::result::Result<(), EventLoopError> {
        Ok(())
    }
}

impl SlintPlatform for TestUi {
    fn create_window_adapter(
        &self,
    ) -> std::result::Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(self.0.clone())))
    }
}

#[derive(Default)]
struct WriteGate {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}

impl WriteGate {
    fn block(&self) {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.changed.notify_all();
        while !state.1 {
            let (next, timeout) = self
                .changed
                .wait_timeout(state, Duration::from_secs(8))
                .unwrap();
            state = next;
            assert!(!timeout.timed_out(), "The fixture setter was not released");
        }
    }

    fn wait_started(&self) {
        let state = self.state.lock().unwrap();
        let (state, timeout) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(1), |state| !state.0)
            .unwrap();
        assert!(
            state.0 && !timeout.timed_out(),
            "The fixture setter did not start"
        );
    }

    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.changed.notify_all();
    }
}

struct Device {
    light: bool,
    warm: bool,
    brightness: AtomicU8,
    warmth: AtomicU8,
    fail_write: AtomicBool,
    writes: Mutex<Vec<(LightKind, u8)>>,
    gate: Mutex<Option<Arc<WriteGate>>>,
    battery_gate: Mutex<Option<Arc<WriteGate>>>,
    battery_queries: AtomicUsize,
    panic_battery: AtomicBool,
    inversions: Mutex<Vec<bool>>,
}

impl Device {
    fn new(light: bool, warm: bool) -> Self {
        Self {
            light,
            warm,
            brightness: AtomicU8::new(8),
            warmth: AtomicU8::new(10),
            fail_write: AtomicBool::new(false),
            writes: Mutex::new(vec![]),
            gate: Mutex::new(None),
            battery_gate: Mutex::new(None),
            battery_queries: AtomicUsize::new(0),
            panic_battery: AtomicBool::new(false),
            inversions: Mutex::new(vec![]),
        }
    }

    fn write(&self, kind: LightKind, value: u8) -> std::result::Result<(), PlatformError> {
        let gate = self.gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.block();
        }
        if self.fail_write.load(Ordering::Acquire) {
            return Err(PlatformError::Operation("fixture setter failure".into()));
        }
        match kind {
            LightKind::Brightness => self.brightness.store(value, Ordering::Release),
            LightKind::Warmth => self.warmth.store(value, Ordering::Release),
        }
        self.writes.lock().unwrap().push((kind, value));
        Ok(())
    }
}

impl DeviceControl for Device {
    fn native_viewport(&self) -> Option<(u32, u32)> {
        Some(DeviceProfile::Scribe.physical_size())
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities {
            brightness: self.light,
            warmth: self.warm,
            ..Default::default()
        }
    }

    fn brightness(&self) -> std::result::Result<u8, PlatformError> {
        Ok(self.brightness.load(Ordering::Acquire))
    }

    fn warmth(&self) -> std::result::Result<u8, PlatformError> {
        Ok(self.warmth.load(Ordering::Acquire))
    }

    fn set_brightness(&self, value: u8) -> std::result::Result<(), PlatformError> {
        self.write(LightKind::Brightness, value)
    }

    fn set_warmth(&self, value: u8) -> std::result::Result<(), PlatformError> {
        self.write(LightKind::Warmth, value)
    }

    fn battery_percentage(&self) -> std::result::Result<u8, PlatformError> {
        self.battery_queries.fetch_add(1, Ordering::AcqRel);
        if let Some(gate) = self.battery_gate.lock().unwrap().clone() {
            gate.block();
        }
        assert!(
            !self.panic_battery.load(Ordering::Acquire),
            "The fixture battery operation panicked"
        );
        Ok(80)
    }

    fn set_inverted(&self, enabled: bool) -> std::result::Result<(), PlatformError> {
        self.inversions.lock().unwrap().push(enabled);
        Err(PlatformError::Unsupported("hardware_inversion"))
    }

    fn reset_idle_timer(&self) -> std::result::Result<(), PlatformError> {
        Err(PlatformError::Unsupported("keep_awake"))
    }
}

#[derive(Default)]
struct Display {
    dark: AtomicBool,
    fail: AtomicBool,
    native: Mutex<Option<bool>>,
    software_inverse: AtomicBool,
    managed: AtomicBool,
    sleep: Mutex<Option<ManagedSleepState>>,
    restored_frames: Mutex<Vec<(bool, Option<bool>, bool)>>,
    portrait_resumes: Mutex<Vec<crate::platform::NativePortraitCalibration>>,
}

impl RenderSink for Display {
    fn set_dark_mode(&self, enabled: bool) -> std::result::Result<(), PlatformError> {
        if self.fail.load(Ordering::Acquire) {
            return Err(PlatformError::Operation("fixture render failure".into()));
        }
        self.dark.store(enabled, Ordering::Release);
        self.software_inverse.store(
            enabled ^ self.native.lock().unwrap().unwrap_or(false),
            Ordering::Release,
        );
        Ok(())
    }

    fn native_output_inverted(&self) -> Option<bool> {
        *self.native.lock().unwrap()
    }

    fn set_native_output_inverted(&self, enabled: bool) -> std::result::Result<(), PlatformError> {
        if self.fail.load(Ordering::Acquire) {
            return Err(PlatformError::Operation(
                "fixture native compensation failure".into(),
            ));
        }
        *self.native.lock().unwrap() = Some(enabled);
        self.software_inverse.store(
            self.dark.load(Ordering::Acquire) ^ enabled,
            Ordering::Release,
        );
        Ok(())
    }

    fn managed_sleep_supported(&self) -> bool {
        self.managed.load(Ordering::Acquire)
    }

    fn set_managed_sleep(&self, sleeping: bool) -> std::result::Result<(), PlatformError> {
        if !self.managed_sleep_supported() {
            return Err(PlatformError::Unsupported("managed_sleep"));
        }
        if sleeping {
            *self.sleep.lock().unwrap() = Some(ManagedSleepState::Suspending);
        } else {
            self.restored_frames.lock().unwrap().push((
                self.dark.load(Ordering::Acquire),
                *self.native.lock().unwrap(),
                self.software_inverse.load(Ordering::Acquire),
            ));
            *self.sleep.lock().unwrap() = Some(ManagedSleepState::Resuming);
        }
        Ok(())
    }

    fn managed_sleep_state(&self) -> std::result::Result<ManagedSleepState, PlatformError> {
        if !self.managed_sleep_supported() {
            return Err(PlatformError::Unsupported("managed_sleep"));
        }
        Ok(self
            .sleep
            .lock()
            .unwrap()
            .unwrap_or(ManagedSleepState::Running))
    }

    fn prepare_native_portrait_resume(
        &self,
        observation: crate::platform::NativePortraitCalibration,
    ) -> std::result::Result<(), PlatformError> {
        if self.managed_sleep_state()? != ManagedSleepState::Suspended {
            return Err(PlatformError::Operation(
                "The fixture portrait observation arrived after input resumed".into(),
            ));
        }
        self.portrait_resumes.lock().unwrap().push(observation);
        Ok(())
    }
}

fn attach(
    directory: &std::path::Path,
    runtime: &tokio::runtime::Runtime,
    device: Arc<Device>,
    display: Option<Arc<Display>>,
) -> Result<(AppWindow, Rc<RefCell<App>>)> {
    attach_with_kind(directory, runtime, device, display, false)
}

fn attach_with_kind(
    directory: &std::path::Path,
    runtime: &tokio::runtime::Runtime,
    device: Arc<Device>,
    display: Option<Arc<Display>>,
    is_kindle: bool,
) -> Result<(AppWindow, Rc<RefCell<App>>)> {
    attach_with_mode(directory, runtime, device, display, is_kindle, false)
}

fn attach_with_mode(
    directory: &std::path::Path,
    runtime: &tokio::runtime::Runtime,
    device: Arc<Device>,
    display: Option<Arc<Display>>,
    is_kindle: bool,
    demo: bool,
) -> Result<(AppWindow, Rc<RefCell<App>>)> {
    let ui = AppWindow::new()?;
    let platform = Arc::new(Platform::with_control(
        DeviceProfile::Paperwhite6,
        is_kindle,
        device,
    ));
    if let Some(display) = display {
        platform.set_render_sink(display)?;
    }
    crate::platform::configure_window(ui.window(), &platform)?;
    let app = App::attach(
        &ui,
        AppOptions {
            data_dir: directory.to_path_buf(),
            profile: DeviceProfile::Paperwhite6,
            demo,
            offline: true,
            initial_screen: "library".into(),
            fonts: vec![FontRecord {
                id: "song".into(),
                label: "Reader lighting fixture font".into(),
                family: "serif".into(),
                metrics: TextMetrics::default(),
                weights: BTreeMap::new(),
                available: true,
            }],
        },
        platform,
        runtime,
    )?;
    Ok((ui, app))
}

fn settle(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        app.poll_lighting();
        if !app.lighting.busy() {
            break;
        }
        assert!(Instant::now() < deadline, "The controller did not settle");
        std::thread::sleep(Duration::from_millis(1));
    }
    app.render().unwrap();
}

fn seed_reader(app: &mut App) -> Result<()> {
    app.store
        .upsert_book(&Book::new("book", "Reader lighting fixture"))?;
    app.store.upsert_chapter(&Chapter {
        id: "chapter".into(),
        book_id: "book".into(),
        index: 0,
        title: "Fixture chapter".into(),
        cached: true,
        body: Some("A readable Reader lighting fixture. ".repeat(500)),
        content_status: ContentStatus::Full,
        ..Chapter::default()
    })?;
    app.navigate(Route::Reader("book".into(), 0), true)?;
    app.render()
}

fn run(case: &str, body: fn() -> Result<()>) -> Result<()> {
    const CHILD_CASE: &str = "WEREAD_READER_LIGHTING_CONTROLLER_CHILD_CASE";
    const COMPLETED: &str = "WEREAD_READER_LIGHTING_CONTROLLER_COMPLETED";
    if let Some(selected) = std::env::var_os(CHILD_CASE) {
        anyhow::ensure!(
            selected.as_os_str() == std::ffi::OsStr::new(case),
            "The controller child selected a different test case"
        );
        std::thread::spawn(body)
            .join()
            .expect("The Reader lighting fixture panicked")?;
        println!("\n{COMPLETED}:{case}");
        return Ok(());
    }
    // Slint's event-loop proxy is process-global. Each fixture needs a fresh
    // process as well as a fresh UI thread, including under the full test suite.
    let output = std::process::Command::new(std::env::current_exe()?)
        .args(["--exact", case, "--nocapture", "--test-threads=1"])
        .env(CHILD_CASE, case)
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let marker = format!("{COMPLETED}:{case}");
    anyhow::ensure!(
        output.status.success()
            && stdout
                .lines()
                .filter(|line| *line == marker.as_str())
                .count()
                == 1,
        "The isolated controller case did not complete exactly once: {case}\nstatus: {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status,
    );
    Ok(())
}

fn confirmed_hardware() -> Result<()> {
    slint::platform::set_platform(Box::new(TestUi(Arc::new(UiLoop::default()))))?;
    let directory = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let device = Arc::new(Device::new(true, true));
    let (ui, application) = attach(directory.path(), &runtime, device.clone(), None)?;
    {
        let mut app = application.borrow_mut();
        settle(&mut app);
        seed_reader(&mut app)?;
        app.dispatch("reader", "tool", "light")?;
        settle(&mut app);
        assert_eq!(ui.global::<ReaderModel>().get_brightness(), 8);
        assert!(ui.global::<ReaderModel>().get_brightness_known());
        assert_eq!(ui.global::<ReaderModel>().get_warmth(), 10);
        device.brightness.store(25, Ordering::Release);
        app.request_device_light_refresh();
        settle(&mut app);
        assert!(!ui.global::<ReaderModel>().get_brightness_known());
        assert!(ui.global::<ReaderModel>().get_warmth_known());
        app.dispatch("reader", "brightness-select", "12")?;
        settle(&mut app);
        assert!(device.writes.lock().unwrap().is_empty());
        device.brightness.store(8, Ordering::Release);
        app.request_device_light_refresh();
        settle(&mut app);
        device.fail_write.store(true, Ordering::Release);
        app.dispatch("reader", "brightness-select", "12")?;
        settle(&mut app);
        assert_eq!(app.lighting.value(LightKind::Brightness), Some(8));
        assert_eq!(ui.global::<ReaderModel>().get_brightness(), 8);
        assert_eq!(app.lighting.status, tr("hardware_unavailable"));
        device.fail_write.store(false, Ordering::Release);
        for value in ["25", "-1"] {
            app.dispatch("reader", "brightness-select", value)?;
        }
        app.dispatch("reader", "brightness", "2")?;
        assert!(device.writes.lock().unwrap().is_empty());
        for value in ["0", "24"] {
            app.dispatch("reader", "brightness-select", value)?;
            settle(&mut app);
        }
        app.dispatch("reader", "brightness", "1")?;
        settle(&mut app);
        assert_eq!(
            device.writes.lock().unwrap().as_slice(),
            &[(LightKind::Brightness, 0), (LightKind::Brightness, 24)]
        );
        app.stop_lighting_hardware()?;
    }
    drop(application);
    drop(ui);
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn lighting_unknown_values_failures_and_bounds_never_publish_unconfirmed_success() -> Result<()> {
    run(
        "app::lighting::tests::lighting_unknown_values_failures_and_bounds_never_publish_unconfirmed_success",
        confirmed_hardware,
    )
}

fn shutdown_pending_light() -> Result<()> {
    slint::platform::set_platform(Box::new(TestUi(Arc::new(UiLoop::default()))))?;
    let directory = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let device = Arc::new(Device::new(true, true));
    let (ui, application) = attach(directory.path(), &runtime, device.clone(), None)?;
    {
        let mut app = application.borrow_mut();
        settle(&mut app);
        seed_reader(&mut app)?;
        let gate = Arc::new(WriteGate::default());
        *device.gate.lock().unwrap() = Some(gate.clone());
        app.change_device_light("brightness", 12, true);
        gate.wait_started();
        app.change_device_light("brightness", 14, true);
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            gate.release();
        });
        app.shutdown()?;
        release.join().unwrap();
        assert!(app.lighting.worker.is_none());
        assert!(!app.lighting.busy());
        assert_eq!(
            device.writes.lock().unwrap().as_slice(),
            &[(LightKind::Brightness, 12)]
        );
        let checkpoint: serde_json::Value = app.store.setting("reader.resume.v1")?.unwrap();
        assert_eq!(checkpoint["book_id"], "book");
        app.shutdown()?;
        assert_eq!(device.writes.lock().unwrap().len(), 1);
    }
    drop(application);
    drop(ui);
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn shutdown_saves_reader_then_joins_the_inflight_setter_and_cancels_the_queued_target() -> Result<()>
{
    run(
        "app::lighting::tests::shutdown_saves_reader_then_joins_the_inflight_setter_and_cancels_the_queued_target",
        shutdown_pending_light,
    )
}

fn recover_failed_exit() -> Result<()> {
    let loop_state = Arc::new(UiLoop::default());
    slint::platform::set_platform(Box::new(TestUi(loop_state.clone())))?;
    let directory = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let device = Arc::new(Device::new(true, true));
    let (ui, application) = attach(directory.path(), &runtime, device.clone(), None)?;
    {
        let mut app = application.borrow_mut();
        settle(&mut app);
        seed_reader(&mut app)?;
        let original_started = Utc::now().timestamp() - 60;
        app.reader.session_started = Some(original_started);
        app.reader.session_seconds = 4;
        loop_state.reject_quit.store(true, Ordering::Release);
        assert!(app.request_exit().is_err());
        assert_eq!(loop_state.quit_attempts.load(Ordering::Acquire), 1);
        assert!(!app.closing.get());
        let resumed_started = app.reader.session_started.unwrap();
        assert!(resumed_started > original_started);
        assert_eq!(app.reader.session_seconds, 0);
        let original = app.store.sessions(i64::MIN, i64::MAX)?;
        assert_eq!(original.len(), 1);
        assert_eq!(original[0].started_at, original_started);
        assert_eq!(original[0].seconds, 4);
        // A real elapsed second makes the resumed local session durable without
        // fabricating a cloud report run or exceeding its elapsed-time budget.
        let deadline = Instant::now() + Duration::from_secs(2);
        while Utc::now().timestamp() <= resumed_started {
            assert!(
                Instant::now() < deadline,
                "The local session clock did not advance"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        app.last_tick = Instant::now() - Duration::from_secs(1);
        app.tick()?;
        assert_eq!(app.reader.session_seconds, 1);
        app.request_device_light_refresh();
        settle(&mut app);
        app.change_device_light("brightness", 12, true);
        settle(&mut app);
        assert_eq!(device.brightness.load(Ordering::Acquire), 12);
        loop_state.reject_quit.store(false, Ordering::Release);
        let database = rusqlite::Connection::open(directory.path().join("library.sqlite"))?;
        database.execute_batch("CREATE TRIGGER reject_preferences BEFORE INSERT ON settings WHEN NEW.key = 'preferences' BEGIN SELECT RAISE(FAIL, 'fixture save failure'); END;")?;
        assert!(app.request_exit().is_err());
        assert_eq!(loop_state.quit_attempts.load(Ordering::Acquire), 1);
        assert!(!app.closing.get());
        assert!(app.reader.session_started.is_some());
        assert_eq!(app.reader.session_seconds, 0);
        let completed = app.store.sessions(i64::MIN, i64::MAX)?;
        assert_eq!(completed.len(), 2);
        assert_eq!(completed[0].id, original[0].id);
        assert_eq!(completed[0].seconds, 4);
        assert_ne!(completed[1].id, original[0].id);
        assert_eq!(completed[1].started_at, resumed_started);
        assert_eq!(completed[1].seconds, 1);
        assert_eq!(
            completed.iter().map(|session| session.seconds).sum::<u64>(),
            5
        );
        app.request_device_light_refresh();
        settle(&mut app);
        app.change_device_light("brightness", 14, true);
        settle(&mut app);
        assert_eq!(device.brightness.load(Ordering::Acquire), 14);
        database.execute_batch("DROP TRIGGER reject_preferences;")?;
        app.shutdown()?;
        app.shutdown()?;
        assert_eq!(loop_state.quit_attempts.load(Ordering::Acquire), 1);
        let final_sessions = app.store.sessions(i64::MIN, i64::MAX)?;
        assert_eq!(final_sessions.len(), 2);
        assert_eq!(
            final_sessions
                .iter()
                .map(|session| session.seconds)
                .sum::<u64>(),
            5
        );
    }
    drop(application);
    drop(ui);
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn rejected_quit_and_failed_preference_save_leave_lighting_recoverable() -> Result<()> {
    run(
        "app::lighting::tests::rejected_quit_and_failed_preference_save_leave_lighting_recoverable",
        recover_failed_exit,
    )
}

fn persist_dark() -> Result<()> {
    slint::platform::set_platform(Box::new(TestUi(Arc::new(UiLoop::default()))))?;
    let directory = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let device = Arc::new(Device::new(false, false));
    let display = Arc::new(Display::default());
    let (ui, application) = attach(
        directory.path(),
        &runtime,
        device.clone(),
        Some(display.clone()),
    )?;
    {
        let mut app = application.borrow_mut();
        seed_reader(&mut app)?;
        assert!(app.platform.capabilities().application_dark_mode);
        assert!(!app.prefs.dark_mode);
        assert!(!ui.global::<ReaderModel>().get_night());
        app.toggle_dark_mode()?;
        app.render()?;
        assert!(display.dark.load(Ordering::Acquire));
        assert!(app.prefs.dark_mode);
        assert!(ui.global::<ReaderModel>().get_night());
        assert!(
            app.store
                .setting::<AppPreferences>("preferences")?
                .unwrap()
                .dark_mode
        );
        app.dispatch("reader", "tool", "light")?;
        assert!(ui.global::<ReaderModel>().get_night());
        assert!(device.inversions.lock().unwrap().is_empty());
        app.shutdown()?;
    }
    drop(application);
    drop(ui);

    // A fresh renderer starts in light mode, so only loading the saved setting
    // can restore dark output after reopening the application.
    let reloaded_display = Arc::new(Display::default());
    assert!(!reloaded_display.dark.load(Ordering::Acquire));
    let (ui, application) = attach(
        directory.path(),
        &runtime,
        device.clone(),
        Some(reloaded_display.clone()),
    )?;
    {
        let mut app = application.borrow_mut();
        assert!(app.prefs.dark_mode);
        assert!(reloaded_display.dark.load(Ordering::Acquire));
        seed_reader(&mut app)?;
        assert!(ui.global::<ReaderModel>().get_night());
        reloaded_display.fail.store(true, Ordering::Release);
        assert!(app.toggle_dark_mode().is_err());
        app.render()?;
        assert!(app.prefs.dark_mode);
        assert!(ui.global::<ReaderModel>().get_night());
        assert!(reloaded_display.dark.load(Ordering::Acquire));
        assert!(
            app.store
                .setting::<AppPreferences>("preferences")?
                .unwrap()
                .dark_mode
        );
        reloaded_display.fail.store(false, Ordering::Release);
        let database = rusqlite::Connection::open(directory.path().join("library.sqlite"))?;
        database.execute_batch("CREATE TRIGGER reject_dark BEFORE INSERT ON settings WHEN NEW.key = 'preferences' BEGIN SELECT RAISE(FAIL, 'fixture persistence failure'); END;")?;
        assert!(app.toggle_dark_mode().is_err());
        app.render()?;
        assert!(app.prefs.dark_mode);
        assert!(ui.global::<ReaderModel>().get_night());
        assert!(reloaded_display.dark.load(Ordering::Acquire));
        assert!(
            app.store
                .setting::<AppPreferences>("preferences")?
                .unwrap()
                .dark_mode
        );
        assert!(device.inversions.lock().unwrap().is_empty());
        database.execute_batch("DROP TRIGGER reject_dark;")?;
        app.shutdown()?;
    }
    drop(application);
    drop(ui);
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn reader_dark_mode_persists_across_app_reopen_and_rolls_back_failures() -> Result<()> {
    run(
        "app::lighting::tests::reader_dark_mode_persists_across_app_reopen_and_rolls_back_failures",
        persist_dark,
    )
}

#[test]
fn known_kindle_system_theme_replaces_stale_saved_appearance_while_desktop_remains_local()
-> Result<()> {
    for is_kindle in [false, true] {
        for native_dark in [false, true] {
            let directory = tempfile::tempdir()?;
            let store = Store::open(directory.path().join("library.sqlite"))?;
            let saved = AppPreferences {
                dark_mode: !native_dark,
                view_mode: "list".into(),
                ..Default::default()
            };
            store.set_setting("preferences", &saved)?;
            let display = Arc::new(Display {
                native: Mutex::new(Some(native_dark)),
                ..Default::default()
            });
            let platform = Platform::with_control(
                DeviceProfile::Paperwhite6,
                is_kindle,
                Arc::new(Device::new(false, false)),
            );
            platform.set_render_sink(display.clone())?;
            let mut restored = saved.clone();
            restore_dark_mode(&platform, &store, &mut restored)?;
            let expected = if is_kindle {
                native_dark
            } else {
                saved.dark_mode
            };
            assert_eq!(restored.dark_mode, expected);
            assert_eq!(display.dark.load(Ordering::Acquire), expected);
            assert_eq!(platform.native_output_inverted(), Some(native_dark));
            assert_eq!(restored.view_mode, saved.view_mode);
            assert_eq!(
                store.setting::<AppPreferences>("preferences")?.unwrap(),
                restored
            );
            if is_kindle {
                assert!(!display.software_inverse.load(Ordering::Acquire));
            }
        }
    }
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path().join("library.sqlite"))?;
    let saved = AppPreferences::default();
    store.set_setting("preferences", &saved)?;
    let database = rusqlite::Connection::open(directory.path().join("library.sqlite"))?;
    database.execute_batch("CREATE TRIGGER reject_startup_mirror BEFORE INSERT ON settings WHEN NEW.key = 'preferences' BEGIN SELECT RAISE(FAIL, 'fixture persistence failure'); END;")?;
    let display = Arc::new(Display {
        native: Mutex::new(Some(true)),
        ..Default::default()
    });
    let platform = Platform::with_control(
        DeviceProfile::Paperwhite6,
        true,
        Arc::new(Device::new(false, false)),
    );
    platform.set_render_sink(display.clone())?;
    let mut restored = saved.clone();
    restore_dark_mode(&platform, &store, &mut restored)?;
    assert!(restored.dark_mode);
    assert!(display.dark.load(Ordering::Acquire));
    assert!(!display.software_inverse.load(Ordering::Acquire));
    assert_eq!(
        store.setting::<AppPreferences>("preferences")?.unwrap(),
        saved
    );
    Ok(())
}

fn confirmed_native_theme_with_failed_preference_mirror() -> Result<()> {
    slint::platform::set_platform(Box::new(TestUi(Arc::new(UiLoop::default()))))?;
    let directory = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let device = Arc::new(Device::new(false, false));
    let display = Arc::new(Display {
        native: Mutex::new(Some(false)),
        ..Default::default()
    });
    let (ui, application) = attach_with_kind(
        directory.path(),
        &runtime,
        device.clone(),
        Some(display.clone()),
        true,
    )?;
    {
        let mut app = application.borrow_mut();
        settle(&mut app);
        seed_reader(&mut app)?;
        app.save_preferences()?;
        assert!(!app.prefs.dark_mode);
        let database = rusqlite::Connection::open(directory.path().join("library.sqlite"))?;
        database.execute_batch("CREATE TRIGGER reject_native_mirror BEFORE INSERT ON settings WHEN NEW.key = 'preferences' BEGIN SELECT RAISE(FAIL, 'fixture persistence failure'); END;")?;

        // This callback carries the launcher's already-confirmed system result.
        // Storage rejection cannot authorize another native transaction.
        app.synchronize_native_dark_mode(true)?;
        app.render()?;
        assert!(app.prefs.dark_mode);
        assert!(ui.global::<ReaderModel>().get_night());
        assert_eq!(app.platform.native_output_inverted(), Some(true));
        assert!(display.dark.load(Ordering::Acquire));
        assert!(!display.software_inverse.load(Ordering::Acquire));
        assert!(device.inversions.lock().unwrap().is_empty());
        assert_eq!(app.lighting.status, tr("api_error"));
        assert!(
            !app.store
                .setting::<AppPreferences>("preferences")?
                .unwrap()
                .dark_mode
        );

        database.execute_batch("DROP TRIGGER reject_native_mirror;")?;
        let mut stale = app.store.setting::<AppPreferences>("preferences")?.unwrap();
        restore_dark_mode(&app.platform, &app.store, &mut stale)?;
        assert!(stale.dark_mode);
        assert!(
            app.store
                .setting::<AppPreferences>("preferences")?
                .unwrap()
                .dark_mode
        );
        assert!(!display.software_inverse.load(Ordering::Acquire));
        app.shutdown()?;
    }
    drop(application);
    drop(ui);
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn confirmed_native_theme_observations_keep_output_in_sync_when_preference_mirroring_fails()
-> Result<()> {
    run(
        "app::lighting::tests::confirmed_native_theme_observations_keep_output_in_sync_when_preference_mirroring_fails",
        confirmed_native_theme_with_failed_preference_mirror,
    )
}

#[cfg(target_os = "linux")]
fn write_theme_record(directory: &std::path::Path, name: &str, value: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join(name);
    std::fs::write(&path, value)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn theme_context_fixture(directory: &std::path::Path) -> Result<(LauncherContext, std::fs::File)> {
    use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt, fs::PermissionsExt};
    std::fs::create_dir(directory)?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    let context = LauncherContext::managed_test_context(directory.to_path_buf())?;
    context.write("native-theme-available")?;
    let pipe_path = directory.join("power-events");
    let pipe_name = std::ffi::CString::new(pipe_path.as_os_str().as_bytes())?;
    // SAFETY: pipe_name is a live NUL-terminated private fixture path.
    anyhow::ensure!(unsafe { libc::mkfifo(pipe_name.as_ptr(), 0o600) } == 0);
    let pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(pipe_path)?;
    Ok((context, pipe))
}

#[cfg(target_os = "linux")]
fn native_theme_demo_handoff() -> Result<()> {
    use std::io::Read;
    const OWNER: &str = "7:0123456789abcdef0123456789abcdef";
    let loop_state = Arc::new(UiLoop::default());
    slint::platform::set_platform(Box::new(TestUi(loop_state.clone())))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    for (confirmed, reject_mirror, requests, deferred_exit) in [
        (true, false, 2_u64, false),
        (false, false, 1, false),
        (true, true, 1, false),
        (true, false, 1, true),
    ] {
        let directory = tempfile::tempdir()?;
        let launcher_directory = directory.path().join("launcher");
        let (context, mut notifications) = theme_context_fixture(&launcher_directory)?;
        let device = Arc::new(Device::new(false, false));
        let display = Arc::new(Display {
            native: Mutex::new(Some(false)),
            managed: AtomicBool::new(true),
            sleep: Mutex::new(Some(ManagedSleepState::Running)),
            ..Default::default()
        });
        let (ui, application) = attach_with_mode(
            directory.path(),
            &runtime,
            device.clone(),
            Some(display.clone()),
            true,
            true,
        )?;
        {
            let mut app = application.borrow_mut();
            settle(&mut app);
            app.navigate(Route::Reader("demo-0".into(), 0), true)?;
            app.render()?;
            app.lighting.native_theme_test_context = Some(context.clone());
            assert!(app.demo && app.force_offline && !app.jobs.configured());
            assert!(app.native_theme_sync_available());
            let route = app.route.clone();
            let anchor = app.reader.paginator.as_ref().unwrap().current_page().start;
            for sequence in 1..=requests {
                let previous = app.prefs.dark_mode;
                let target = !previous;
                let request_path = launcher_directory.join("native-theme-request");
                assert!(!request_path.exists());
                app.toggle_dark_mode()?;
                assert!(app.managed_sleep_blocked);
                assert_eq!(app.prefs.dark_mode, previous);
                assert_eq!(ui.global::<ReaderModel>().get_night(), previous);
                assert!(!request_path.exists());
                assert!(app.poll_native_theme()?);
                assert!(!request_path.exists());
                *display.sleep.lock().unwrap() = Some(ManagedSleepState::Suspended);
                assert!(app.poll_native_theme()?);
                assert_eq!(
                    std::fs::read_to_string(&request_path)?,
                    format!(
                        "{OWNER}:{sequence}:{}\n",
                        if target { "Y8INV" } else { "Y8" }
                    )
                );
                let mut bytes = [0; 128];
                let count = notifications.read(&mut bytes)?;
                assert_eq!(
                    std::str::from_utf8(&bytes[..count])?,
                    format!("_weread_native_theme_request 7 {sequence}\n")
                );
                assert_eq!(app.prefs.dark_mode, previous);
                app.dispatch("reader", "night", "toggle")?;
                assert_eq!(app.lighting.native_theme_sequence, sequence);
                if deferred_exit {
                    app.request_exit()?;
                    assert!(!app.closing.get());
                    assert!(app.lighting.native_theme_exit_requested);
                    assert_eq!(loop_state.quit_attempts.load(Ordering::Acquire), 0);
                }

                let database = rusqlite::Connection::open(
                    directory.path().join("demo").join("library.sqlite"),
                )?;
                if reject_mirror {
                    database.execute_batch("CREATE TRIGGER reject_native_mirror BEFORE INSERT ON settings WHEN NEW.key = 'preferences' BEGIN SELECT RAISE(FAIL, 'fixture persistence failure'); END;")?;
                }
                if sequence == 1 {
                    for stale in [
                        format!("{OWNER}:99:ok:Y8INV\n"),
                        "7:ffffffffffffffffffffffffffffffff:1:ok:Y8INV\n".into(),
                        format!("{OWNER}:1:ok:UNKNOWN\n"),
                    ] {
                        write_theme_record(&launcher_directory, "native-theme-result", &stale)?;
                        assert!(app.poll_native_theme()?);
                        assert_eq!(app.prefs.dark_mode, previous);
                        assert_eq!(display.managed_sleep_state()?, ManagedSleepState::Suspended);
                    }
                }
                let actual = if confirmed { target } else { previous };
                write_theme_record(
                    &launcher_directory,
                    "native-theme-result",
                    &format!(
                        "{OWNER}:{sequence}:{}:{}\n",
                        if confirmed { "ok" } else { "error" },
                        if actual { "Y8INV" } else { "Y8" }
                    ),
                )?;
                let portraits_before = display.portrait_resumes.lock().unwrap().len();
                write_theme_record(
                    &launcher_directory,
                    "native-theme-portrait-calibration",
                    &format!(
                        "{OWNER}:{sequence}:{}\n",
                        if sequence == 1 { "D:1" } else { "FIXED:1" }
                    ),
                )?;
                assert!(app.poll_native_theme()?);
                let portraits = display.portrait_resumes.lock().unwrap();
                if sequence == 1 {
                    assert_eq!(portraits.len(), portraits_before + 1);
                    assert_eq!(
                        portraits.last(),
                        Some(&crate::platform::NativePortraitCalibration {
                            direction: crate::platform::NativePortraitDirection::Down,
                            framebuffer_rotation: 1,
                        })
                    );
                } else {
                    assert_eq!(portraits.len(), portraits_before);
                }
                drop(portraits);
                assert_eq!(app.prefs.dark_mode, actual);
                assert_eq!(ui.global::<ReaderModel>().get_night(), actual);
                assert_eq!(app.platform.native_output_inverted(), Some(actual));
                assert_eq!(
                    *display.restored_frames.lock().unwrap().last().unwrap(),
                    (actual, Some(actual), false)
                );
                assert!(!launcher_directory.join("native-theme-ready").exists());
                assert!(app.managed_sleep_blocked);
                *display.sleep.lock().unwrap() = Some(ManagedSleepState::Running);
                assert!(app.poll_native_theme()?);
                assert!(app.managed_sleep_blocked);
                assert!(app.lighting.native_theme.is_some());
                assert!(!app.closing.get());
                let active_request = std::fs::read_to_string(&request_path)?;
                // A rapid second tap cannot overwrite the first transaction
                // while the server is confirming its own running state.
                app.dispatch("reader", "night", "toggle")?;
                assert_eq!(app.lighting.native_theme_sequence, sequence);
                assert_eq!(std::fs::read_to_string(&request_path)?, active_request);
                assert!(!context.native_theme_committed(sequence)?);
                assert!(app.poll_native_theme()?);
                if deferred_exit {
                    assert_eq!(loop_state.quit_attempts.load(Ordering::Acquire), 0);
                }
                assert_eq!(
                    std::fs::read_to_string(launcher_directory.join("native-theme-ready"))?,
                    format!("{OWNER}:{sequence}\n")
                );
                // This unlink models the owning server's implicit commit.
                std::fs::remove_file(&request_path)?;
                assert!(context.native_theme_committed(sequence)?);
                assert_eq!(app.poll_native_theme()?, deferred_exit);
                assert!(!app.managed_sleep_blocked);
                assert!(app.lighting.native_theme.is_none());
                assert_eq!(app.closing.get(), deferred_exit);
                if deferred_exit {
                    assert_eq!(loop_state.quit_attempts.load(Ordering::Acquire), 1);
                    assert!(!launcher_directory.join("native-dark-target").exists());
                }
                assert_eq!(
                    std::fs::read_to_string(launcher_directory.join("native-theme-ready"))?,
                    format!("{OWNER}:{sequence}\n")
                );
                let persisted = app.store.setting::<AppPreferences>("preferences")?.unwrap();
                assert_eq!(
                    persisted.dark_mode,
                    if reject_mirror { previous } else { actual }
                );
                if reject_mirror {
                    assert_eq!(
                        ui.global::<ShellModel>().get_toast().as_str(),
                        tr("api_error")
                    );
                    database.execute_batch("DROP TRIGGER reject_native_mirror;")?;
                    let mut reloaded = persisted;
                    restore_dark_mode(&app.platform, &app.store, &mut reloaded)?;
                    assert_eq!(reloaded.dark_mode, actual);
                } else if !confirmed {
                    assert_eq!(
                        ui.global::<ShellModel>().get_toast().as_str(),
                        tr("hardware_unavailable")
                    );
                }
                assert_eq!(app.route, route);
                assert_eq!(
                    app.reader.paginator.as_ref().unwrap().current_page().start,
                    anchor
                );
                assert!(device.inversions.lock().unwrap().is_empty());
                if !deferred_exit {
                    settle(&mut app);
                }
            }
            app.shutdown()?;
        }
        drop(application);
        drop(ui);
    }
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn demo_native_theme_toggles_wait_for_protected_results_and_restore_the_same_reader() -> Result<()>
{
    run(
        "app::lighting::tests::demo_native_theme_toggles_wait_for_protected_results_and_restore_the_same_reader",
        native_theme_demo_handoff,
    )
}

fn shutdown_pending_battery_and_light() -> Result<()> {
    slint::platform::set_platform(Box::new(TestUi(Arc::new(UiLoop::default()))))?;
    let directory = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let device = Arc::new(Device::new(true, true));
    let battery_gate = Arc::new(WriteGate::default());
    *device.battery_gate.lock().unwrap() = Some(battery_gate.clone());
    let (ui, application) =
        attach_with_kind(directory.path(), &runtime, device.clone(), None, true)?;
    battery_gate.wait_started();
    {
        let mut app = application.borrow_mut();
        let ready = Instant::now() + Duration::from_secs(1);
        while app.lighting.value(LightKind::Brightness).is_none() {
            app.poll_lighting();
            anyhow::ensure!(
                Instant::now() < ready,
                "The initial lighting query did not finish"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let light_gate = Arc::new(WriteGate::default());
        *device.gate.lock().unwrap() = Some(light_gate.clone());
        app.change_device_light("brightness", 12, true);
        light_gate.wait_started();
        app.change_device_light("brightness", 14, true);
        ui.global::<ShellModel>()
            .set_battery_label("pending".into());

        let started = Instant::now();
        assert!(app.stop_lighting_hardware().is_err());
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "Battery and lighting must share one shutdown deadline"
        );
        assert!(app.lighting.stopping);
        assert!(app.lighting.worker.is_some());
        assert!(app.lighting.battery.is_some());
        assert!(
            app.lighting
                .battery
                .as_ref()
                .unwrap()
                .cancelled
                .load(Ordering::Acquire)
        );
        app.request_device_status();
        app.change_device_light("brightness", 16, true);
        assert_eq!(device.battery_queries.load(Ordering::Acquire), 1);
        app.poll_lighting();
        assert!(app.lighting.stopping);

        light_gate.release();
        let finished = Instant::now() + Duration::from_secs(1);
        while device.writes.lock().unwrap().is_empty() {
            anyhow::ensure!(
                Instant::now() < finished,
                "The cancelled setter did not finish"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        app.poll_lighting();
        assert!(
            app.lighting.stopping,
            "A completed lighting worker cannot hide a live battery read"
        );
        assert!(app.lighting.battery.is_some());
        assert_eq!(
            ui.global::<ShellModel>().get_battery_label().as_str(),
            "pending"
        );

        battery_gate.release();
        app.stop_lighting_hardware()?;
        assert!(!app.lighting.stopping);
        assert!(app.lighting.worker.is_none());
        assert!(app.lighting.battery.is_none());
        assert!(!app.lighting.busy());
        app.poll_lighting();
        assert_eq!(
            ui.global::<ShellModel>().get_battery_label().as_str(),
            "pending"
        );
        assert_eq!(
            device.writes.lock().unwrap().as_slice(),
            &[(LightKind::Brightness, 12)]
        );
        app.stop_lighting_hardware()?;
    }
    drop(application);
    drop(ui);
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn shutdown_joins_battery_and_lighting_against_one_shared_deadline() -> Result<()> {
    run(
        "app::lighting::tests::shutdown_joins_battery_and_lighting_against_one_shared_deadline",
        shutdown_pending_battery_and_light,
    )
}

fn recover_cancelled_battery_without_publishing() -> Result<()> {
    slint::platform::set_platform(Box::new(TestUi(Arc::new(UiLoop::default()))))?;
    let directory = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let device = Arc::new(Device::new(false, false));
    let gate = Arc::new(WriteGate::default());
    *device.battery_gate.lock().unwrap() = Some(gate.clone());
    let (ui, application) =
        attach_with_kind(directory.path(), &runtime, device.clone(), None, true)?;
    gate.wait_started();
    {
        let mut app = application.borrow_mut();
        ui.global::<ShellModel>()
            .set_battery_label("retained".into());
        app.lighting.stopping = true;
        assert!(
            app.join_lighting_workers(Duration::from_millis(10))
                .is_err()
        );
        assert!(app.lighting.battery.is_some());
        app.poll_lighting();
        assert!(app.lighting.stopping);
        app.request_device_status();
        assert_eq!(device.battery_queries.load(Ordering::Acquire), 1);
        gate.release();
        let joined = Instant::now() + Duration::from_secs(1);
        while app.lighting.stopping {
            app.poll_lighting();
            anyhow::ensure!(
                Instant::now() < joined,
                "The stopped battery worker was not joined"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(app.lighting.worker.is_none());
        assert!(app.lighting.battery.is_none());
        assert_eq!(
            ui.global::<ShellModel>().get_battery_label().as_str(),
            "retained"
        );
        app.request_device_status();
        settle(&mut app);
        assert_eq!(device.battery_queries.load(Ordering::Acquire), 2);
        assert_eq!(
            ui.global::<ShellModel>().get_battery_label().as_str(),
            "80%"
        );
        app.stop_lighting_hardware()?;
    }
    drop(application);
    drop(ui);
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn cancelled_battery_reads_finish_before_status_queries_resume_and_never_publish_late_values()
-> Result<()> {
    run(
        "app::lighting::tests::cancelled_battery_reads_finish_before_status_queries_resume_and_never_publish_late_values",
        recover_cancelled_battery_without_publishing,
    )
}

#[test]
fn a_panicked_battery_worker_notifies_shutdown_and_releases_its_join_handle() -> Result<()> {
    let device = Arc::new(Device::new(false, false));
    let gate = Arc::new(WriteGate::default());
    *device.battery_gate.lock().unwrap() = Some(gate.clone());
    device.panic_battery.store(true, Ordering::Release);
    let platform = Arc::new(Platform::with_control(DeviceProfile::Scribe, false, device));
    let mut battery = BatteryRead::start(platform)?;
    gate.wait_started();
    let cancelled = battery.cancelled.clone();
    let release = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(1);
        while !cancelled.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let stopping = cancelled.load(Ordering::Acquire);
        gate.release();
        stopping
    });
    let error = battery.shutdown(Duration::from_secs(1)).unwrap_err();
    assert!(
        release.join().unwrap(),
        "The panic must occur during shutdown's bounded wait"
    );
    assert!(error.to_string().contains("panicked"));
    assert!(battery.thread.is_none());
    battery.shutdown(Duration::ZERO)?;
    Ok(())
}
