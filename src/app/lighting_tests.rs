use super::super::session_restore::{LauncherContext, ManagedMarker};
use super::*;
use crate::platform::{DeviceControl, ManagedSleepState, PlatformCapabilities, RenderSink};
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
    dark_requests: Mutex<Vec<bool>>,
    fail: AtomicBool,
    native: Mutex<Option<bool>>,
    resume_native: Mutex<Option<bool>>,
    software_inverse: AtomicBool,
    managed: AtomicBool,
    sleep: Mutex<Option<ManagedSleepState>>,
    sleep_requests: Mutex<Vec<bool>>,
    restored_frames: Mutex<Vec<(bool, Option<bool>, bool)>>,
    portrait_resumes: Mutex<Vec<crate::platform::NativePortraitCalibration>>,
}

impl RenderSink for Display {
    fn set_dark_mode(&self, enabled: bool) -> std::result::Result<(), PlatformError> {
        self.dark_requests.lock().unwrap().push(enabled);
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

    fn observes_native_output_on_resume(&self) -> bool {
        self.resume_native.lock().unwrap().is_some()
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
        self.sleep_requests.lock().unwrap().push(sleeping);
        if !self.managed_sleep_supported() {
            return Err(PlatformError::Unsupported("managed_sleep"));
        }
        if sleeping {
            *self.sleep.lock().unwrap() = Some(ManagedSleepState::Suspending);
        } else {
            if let Some(native) = *self.resume_native.lock().unwrap() {
                *self.native.lock().unwrap() = Some(native);
                self.software_inverse.store(
                    self.dark.load(Ordering::Acquire) ^ native,
                    Ordering::Release,
                );
            }
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
    let (ui, application) = attach_with_kind(
        directory.path(),
        &runtime,
        device.clone(),
        Some(display.clone()),
        true,
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
    let (ui, application) = attach_with_kind(
        directory.path(),
        &runtime,
        device.clone(),
        Some(reloaded_display.clone()),
        true,
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
        let requests_before = reloaded_display.dark_requests.lock().unwrap().len();
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
        assert_eq!(
            &reloaded_display.dark_requests.lock().unwrap()[requests_before..],
            &[false, true]
        );
        assert!(device.inversions.lock().unwrap().is_empty());
        assert!(device.writes.lock().unwrap().is_empty());
        assert!(reloaded_display.sleep_requests.lock().unwrap().is_empty());
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
fn saved_app_appearance_is_independent_of_the_native_output_base() -> Result<()> {
    for is_kindle in [false, true] {
        for native_base in [None, Some(false), Some(true)] {
            for desired in [false, true] {
                let directory = tempfile::tempdir()?;
                let store = Store::open(directory.path().join("library.sqlite"))?;
                let saved = AppPreferences {
                    dark_mode: desired,
                    view_mode: "list".into(),
                    ..Default::default()
                };
                store.set_setting("preferences", &saved)?;
                let database = rusqlite::Connection::open(directory.path().join("library.sqlite"))?;
                database.execute_batch("CREATE TRIGGER reject_startup_preferences BEFORE INSERT ON settings WHEN NEW.key = 'preferences' BEGIN SELECT RAISE(FAIL, 'fixture unexpected preference write'); END;")?;
                let display = Arc::new(Display {
                    native: Mutex::new(native_base),
                    ..Default::default()
                });
                let platform = Platform::with_control(
                    DeviceProfile::Paperwhite6,
                    is_kindle,
                    Arc::new(Device::new(false, false)),
                );
                platform.set_render_sink(display.clone())?;
                restore_dark_mode(&platform, &saved)?;
                assert_eq!(display.dark.load(Ordering::Acquire), desired);
                assert_eq!(platform.native_output_inverted(), native_base);
                assert_eq!(
                    display.software_inverse.load(Ordering::Acquire),
                    desired ^ native_base.unwrap_or(false)
                );
                assert_eq!(store.setting::<AppPreferences>("preferences")?, Some(saved));
                assert!(display.sleep_requests.lock().unwrap().is_empty());
            }
        }
    }
    Ok(())
}

fn launcher_context_fixture(directory: &std::path::Path) -> Result<LauncherContext> {
    std::fs::create_dir(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    }
    LauncherContext::managed_test_context(directory.to_path_buf())
}

fn write_launcher_record(directory: &std::path::Path, name: &str, value: &str) -> Result<()> {
    let path = directory.join(name);
    std::fs::write(&path, value)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn launcher_records(directory: &std::path::Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut records = BTreeMap::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        records.insert(
            entry.file_name().to_string_lossy().into_owned(),
            std::fs::read(entry.path())?,
        );
    }
    Ok(records)
}

struct ReleaseGate(Arc<WriteGate>);

impl Drop for ReleaseGate {
    fn drop(&mut self) {
        self.0.release();
    }
}

fn ordinary_night_toggles_preserve_live_work() -> Result<()> {
    let loop_state = Arc::new(UiLoop::default());
    slint::platform::set_platform(Box::new(TestUi(loop_state.clone())))?;
    let directory = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let launcher_directory = directory.path().join("launcher");
    let context = launcher_context_fixture(&launcher_directory)?;
    let device = Arc::new(Device::new(true, true));
    let display = Arc::new(Display {
        native: Mutex::new(Some(true)),
        managed: AtomicBool::new(true),
        sleep: Mutex::new(Some(ManagedSleepState::Running)),
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
        app.managed_sleep = managed_sleep::ManagedSleep::with_test_context(context);
        assert!(!app.poll_managed_sleep(false, false)?);
        let protocol_records = launcher_records(&launcher_directory)?;
        let session = source::AuthSession {
            user_vid: "night-mode-account".into(),
            cookies: BTreeMap::from([
                ("wr_vid".into(), "night-mode-account".into()),
                ("wr_skey".into(), "synthetic-night-mode-session".into()),
            ]),
            ..source::AuthSession::default()
        };
        app.jobs.replace_session(session.clone())?;
        app.session_file.save(&session)?;
        seed_reader(&mut app)?;
        let account_before = std::fs::read(directory.path().join("account.json"))?;
        let active = DownloadTask {
            id: "night-mode-active-download".into(),
            book_id: "book".into(),
            account_key: source::report::account_key(&session).unwrap(),
            chapters: vec![0],
            status: DownloadStatus::Running,
            updated_at: 100,
            ..DownloadTask::default()
        };
        app.store.save_task(&active)?;
        let headers = app.store.chapter_metadata("book")?;
        let plan = crate::jobs::DownloadPlan::from_headers(&active, &headers)?;
        assert!(app.jobs.download(71, active.clone(), plan).is_none());
        app.current_download = Some(active.clone());
        app.download_request = 71;
        app.catalog_requests.insert("retained-catalog".into(), 72);
        app.book_requests.insert("retained-book".into(), 73);
        app.pending_covers.insert("retained-cover".into());
        app.reader.session_started = Some(Utc::now().timestamp() - 20);
        app.reader.session_seconds = 11;
        app.dispatch("reader", "press", "0.5,50")?;
        let page_start = app.reader.paginator.as_ref().unwrap().current_page().start;
        app.reader.selection = Some((page_start + 2, page_start + 8));
        let selection = app.reader.selection;
        let press_generation = ui.global::<ReaderModel>().get_press_generation();
        let route = app.route.clone();
        let body_address = app.reader.body.as_ptr() as usize;
        let pages_address = app.reader.paginator.as_ref().unwrap().pages.as_ptr() as usize;
        let session_started = app.reader.session_started;
        let epoch = app.jobs.account_epoch();
        let battery_gate = Arc::new(WriteGate::default());
        *device.battery_gate.lock().unwrap() = Some(battery_gate.clone());
        app.request_device_status();
        battery_gate.wait_started();
        let release = ReleaseGate(battery_gate);
        let battery = app.lighting.battery.as_ref().unwrap();
        let battery_cancelled = battery.cancelled.clone();
        let battery_thread = battery.thread.as_ref().unwrap().thread().id();
        let light_request = app.lighting.pending.as_ref().map(|pending| pending.id);
        let light_sequence = app.lighting.sequence;
        assert!(app.lighting.worker.is_some());
        assert!(ui.global::<ReaderModel>().get_night_supported());

        for desired in [true, false, true, false] {
            app.dispatch("reader", "night", "toggle")?;
            assert_eq!(app.prefs.dark_mode, desired);
            assert_eq!(ui.global::<ReaderModel>().get_night(), desired);
            assert_eq!(display.dark.load(Ordering::Acquire), desired);
            assert_eq!(app.platform.native_output_inverted(), Some(true));
            assert_eq!(app.route, route);
            assert_eq!(app.reader.body.as_ptr() as usize, body_address);
            assert_eq!(
                app.reader.paginator.as_ref().unwrap().pages.as_ptr() as usize,
                pages_address
            );
            assert_eq!(
                app.reader.paginator.as_ref().unwrap().current_page().start,
                page_start
            );
            assert_eq!(app.reader.session_started, session_started);
            assert_eq!(app.reader.session_seconds, 11);
            assert_eq!(app.reader.selection, selection);
            assert!(app.reader.press.is_some());
            assert_eq!(
                ui.global::<ReaderModel>().get_press_generation(),
                press_generation
            );
            assert!(!app.managed_sleep_blocked);
            assert!(!app.closing.get());
            assert!(!app.resident_lifecycle.is_prepared());
            assert_eq!(app.jobs.account_epoch(), epoch);
            assert_eq!(app.jobs.session(), Some(session.clone()));
            assert_eq!(app.jobs.active_download(), Some(active.id.clone()));
            assert_eq!(app.current_download.as_ref(), Some(&active));
            assert_eq!(
                app.store
                    .tasks()?
                    .into_iter()
                    .find(|task| task.id == active.id),
                Some(active.clone())
            );
            assert_eq!(app.download_request, 71);
            assert_eq!(app.catalog_requests.get("retained-catalog"), Some(&72));
            assert_eq!(app.book_requests.get("retained-book"), Some(&73));
            assert!(app.pending_covers.contains("retained-cover"));
            assert!(app.lighting.worker.is_some());
            assert_eq!(app.lighting.sequence, light_sequence);
            assert_eq!(
                app.lighting.pending.as_ref().map(|pending| pending.id),
                light_request
            );
            assert!(!app.lighting.stopping);
            assert!(!battery_cancelled.load(Ordering::Acquire));
            assert_eq!(
                app.lighting
                    .battery
                    .as_ref()
                    .unwrap()
                    .thread
                    .as_ref()
                    .unwrap()
                    .thread()
                    .id(),
                battery_thread
            );
            assert_eq!(display.managed_sleep_state()?, ManagedSleepState::Running);
            assert!(display.sleep_requests.lock().unwrap().is_empty());
            assert!(display.restored_frames.lock().unwrap().is_empty());
            assert!(device.inversions.lock().unwrap().is_empty());
            assert!(device.writes.lock().unwrap().is_empty());
            assert_eq!(launcher_records(&launcher_directory)?, protocol_records);
            assert_eq!(
                std::fs::read(directory.path().join("account.json"))?,
                account_before
            );
            assert_eq!(
                app.store
                    .setting::<AppPreferences>("preferences")?
                    .unwrap()
                    .dark_mode,
                desired
            );
        }
        release.0.release();
        settle(&mut app);
        app.request_exit()?;
        assert!(app.closing.get());
        assert_eq!(loop_state.quit_attempts.load(Ordering::Acquire), 1);
        assert!(display.sleep_requests.lock().unwrap().is_empty());
        assert_eq!(launcher_records(&launcher_directory)?, protocol_records);
        assert!(!launcher_directory.join("native-dark-target").exists());
    }
    drop(application);
    drop(ui);
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn kindle_night_toggles_preserve_reader_jobs_workers_and_close_without_a_handoff() -> Result<()> {
    run(
        "app::lighting::tests::kindle_night_toggles_preserve_reader_jobs_workers_and_close_without_a_handoff",
        ordinary_night_toggles_preserve_live_work,
    )
}

fn managed_wake_preserves_app_appearance() -> Result<()> {
    slint::platform::set_platform(Box::new(TestUi(Arc::new(UiLoop::default()))))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    for (desired, observe_on_resume) in [(false, false), (true, false), (false, true), (true, true)]
    {
        let directory = tempfile::tempdir()?;
        let saved = AppPreferences {
            dark_mode: desired,
            ..AppPreferences::for_device(false)
        };
        Store::open(directory.path().join("library.sqlite"))?.set_setting("preferences", &saved)?;
        let launcher_directory = directory.path().join("launcher");
        let context = launcher_context_fixture(&launcher_directory)?;
        let device = Arc::new(Device::new(false, false));
        let display = Arc::new(Display {
            native: Mutex::new(Some(desired)),
            resume_native: Mutex::new(observe_on_resume.then_some(!desired)),
            managed: AtomicBool::new(true),
            sleep: Mutex::new(Some(ManagedSleepState::Running)),
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
            let route = app.route.clone();
            let anchor = app.reader.paginator.as_ref().unwrap().current_page().start;
            let body_address = app.reader.body.as_ptr() as usize;
            let pages_address = app.reader.paginator.as_ref().unwrap().pages.as_ptr() as usize;
            app.managed_sleep = managed_sleep::ManagedSleep::with_test_context(context.clone());
            assert_eq!(
                app.platform.observes_native_output_on_resume(),
                observe_on_resume
            );
            assert!(!app.poll_managed_sleep(false, false)?);
            context.write("sleep-request")?;
            context.write_managed_cycle(ManagedMarker::SleepRequest, 1)?;
            assert!(app.poll_managed_sleep(true, false)?);
            assert!(app.managed_sleep_blocked);
            assert!(!launcher_directory.join("managed-sleep-ready").exists());
            *display.sleep.lock().unwrap() = Some(ManagedSleepState::Suspended);
            assert!(app.poll_managed_sleep(false, false)?);
            assert_eq!(context.managed_cycle(ManagedMarker::SleepReady)?, Some(1));
            assert!(context.matches("resume-ready")?);
            let database = rusqlite::Connection::open(directory.path().join("library.sqlite"))?;
            database.execute_batch("CREATE TRIGGER reject_wake_preferences BEFORE INSERT ON settings WHEN NEW.key = 'preferences' BEGIN SELECT RAISE(FAIL, 'fixture unexpected wake preference write'); END;")?;
            let fresh_base = !desired;
            context.write_managed_cycle(ManagedMarker::WakeRequest, 1)?;
            if !observe_on_resume {
                write_launcher_record(
                    &launcher_directory,
                    "managed-native-epdc-mode",
                    &format!(
                        "7:0123456789abcdef0123456789abcdef:1:{}\n",
                        if fresh_base { "Y8INV" } else { "Y8" }
                    ),
                )?;
            } else {
                assert!(!launcher_directory.join("managed-native-epdc-mode").exists());
            }
            assert!(app.poll_managed_sleep(false, true)?);
            assert!(app.managed_sleep_blocked);
            assert_eq!(context.managed_cycle(ManagedMarker::WakeReady)?, None);
            assert_eq!(
                *display.restored_frames.lock().unwrap().last().unwrap(),
                (desired, Some(fresh_base), desired ^ fresh_base)
            );
            assert_eq!(app.prefs.dark_mode, desired);
            assert_eq!(ui.global::<ReaderModel>().get_night(), desired);
            assert_eq!(app.platform.native_output_inverted(), Some(fresh_base));
            assert_ne!(app.lighting.status, tr("api_error"));
            assert_eq!(
                app.store
                    .setting::<AppPreferences>("preferences")?
                    .unwrap()
                    .dark_mode,
                desired
            );
            *display.sleep.lock().unwrap() = Some(ManagedSleepState::Running);
            assert!(!app.poll_managed_sleep(false, false)?);
            assert_eq!(context.managed_cycle(ManagedMarker::WakeReady)?, Some(1));
            assert!(!app.managed_sleep_blocked);
            assert_eq!(app.prefs.dark_mode, desired);
            assert_eq!(display.dark.load(Ordering::Acquire), desired);
            assert_eq!(app.route, route);
            assert_eq!(
                app.reader.paginator.as_ref().unwrap().current_page().start,
                anchor
            );
            assert_eq!(app.reader.body.as_ptr() as usize, body_address);
            assert_eq!(
                app.reader.paginator.as_ref().unwrap().pages.as_ptr() as usize,
                pages_address
            );
            assert_eq!(*display.sleep_requests.lock().unwrap(), vec![true, false]);
            assert!(device.inversions.lock().unwrap().is_empty());
            assert!(device.writes.lock().unwrap().is_empty());
            assert!(!launcher_directory.join("native-theme-request").exists());
            assert!(!launcher_directory.join("native-dark-target").exists());
            database.execute_batch("DROP TRIGGER reject_wake_preferences;")?;
            app.shutdown()?;
        }
        drop(application);
        drop(ui);
    }
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn managed_sleep_restores_the_app_preference_before_acknowledging_an_opposite_native_base()
-> Result<()> {
    run(
        "app::lighting::tests::managed_sleep_restores_the_app_preference_before_acknowledging_an_opposite_native_base",
        managed_wake_preserves_app_appearance,
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
