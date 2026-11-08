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
}

impl RenderSink for Display {
    fn set_dark_mode(&self, enabled: bool) -> std::result::Result<(), PlatformError> {
        if self.fail.load(Ordering::Acquire) {
            return Err(PlatformError::Operation("fixture render failure".into()));
        }
        self.dark.store(enabled, Ordering::Release);
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
            demo: false,
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
