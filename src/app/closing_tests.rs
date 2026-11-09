use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Result;
use slint::{
    ComponentHandle, EventLoopError, LogicalPosition, Rgb8Pixel,
    platform::{
        EventLoopProxy, Platform as SlintPlatform, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};

use super::{App, AppOptions, Route};
use crate::{
    i18n::text as tr,
    jobs::{JobEvent, JobResult},
    platform::{
        self, DeviceControl, DeviceProfile, Platform, PlatformCapabilities, PlatformError,
        RefreshKind, RefreshPlan, RefreshSink, RenderSink,
    },
    preferences::AppPreferences,
    session::SessionFile,
    source::{AuthSession, QrLoginChallenge, QrLoginState},
    store::Store,
    ui::{AppWindow, AuxiliaryModel, LoginModel, ShellModel, Theme},
};

#[derive(Default)]
struct LoopState {
    terminated: AtomicBool,
    reject_quit: AtomicBool,
    quit_attempts: AtomicUsize,
    queued: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
}

struct TestUiPlatform {
    loop_state: Arc<LoopState>,
    window: Rc<RefCell<Option<Rc<MinimalSoftwareWindow>>>>,
}

impl TestUiPlatform {
    fn new(loop_state: Arc<LoopState>) -> Self {
        Self {
            loop_state,
            window: Rc::new(RefCell::new(None)),
        }
    }
}

impl SlintPlatform for TestUiPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.window.replace(Some(window.clone()));
        Ok(window)
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(TestEventLoopProxy(self.loop_state.clone())))
    }
}

struct TestEventLoopProxy(Arc<LoopState>);

impl EventLoopProxy for TestEventLoopProxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        self.0.quit_attempts.fetch_add(1, Ordering::SeqCst);
        if self.0.reject_quit.load(Ordering::SeqCst) {
            return Err(EventLoopError::EventLoopTerminated);
        }
        self.0.terminated.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn invoke_from_event_loop(
        &self,
        event: Box<dyn FnOnce() + Send>,
    ) -> Result<(), EventLoopError> {
        if self.0.terminated.load(Ordering::SeqCst) {
            return Err(EventLoopError::EventLoopTerminated);
        }
        // Native status work may enqueue a callback, but this test never runs a UI loop.
        self.0.queued.lock().unwrap().push(event);
        Ok(())
    }
}

struct TestDeviceControl;

impl DeviceControl for TestDeviceControl {
    fn native_viewport(&self) -> Option<(u32, u32)> {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RefreshAttempt {
    plan: RefreshPlan,
    after_termination: bool,
}

struct RecordingSink {
    loop_state: Arc<LoopState>,
    attempts: Mutex<Vec<RefreshAttempt>>,
}

impl RecordingSink {
    fn take(&self) -> Vec<RefreshAttempt> {
        std::mem::take(&mut *self.attempts.lock().unwrap())
    }
}

impl RefreshSink for RecordingSink {
    fn submit(&self, plan: RefreshPlan) -> Result<(), PlatformError> {
        let after_termination = self.loop_state.terminated.load(Ordering::SeqCst);
        self.attempts.lock().unwrap().push(RefreshAttempt {
            plan,
            after_termination,
        });
        if after_termination {
            return Err(PlatformError::Operation(
                "the fixture event loop has terminated".into(),
            ));
        }
        Ok(())
    }
}

fn assert_live_feedback(ui: &AppWindow, application: &Rc<RefCell<App>>, sink: &RecordingSink) {
    sink.take();
    {
        // Theme feedback must remain safe during an application's synchronous callback.
        let _state = application.borrow_mut();
        ui.global::<Theme>().invoke_refresh("press".into());
        ui.global::<Theme>().invoke_refresh("release".into());
    }
    assert_eq!(
        sink.take(),
        vec![
            RefreshAttempt {
                plan: RefreshPlan {
                    kind: RefreshKind::Fast,
                    rect: None,
                },
                after_termination: false,
            },
            RefreshAttempt {
                plan: RefreshPlan {
                    kind: RefreshKind::Ui,
                    rect: None,
                },
                after_termination: false,
            },
        ],
    );
}

fn run_closing_scenario() -> Result<()> {
    let loop_state = Arc::new(LoopState::default());
    slint::platform::set_platform(Box::new(TestUiPlatform::new(loop_state.clone())))?;
    let profile = DeviceProfile::Paperwhite6;
    let platform = Arc::new(Platform::with_control(
        profile,
        true,
        Arc::new(TestDeviceControl),
    ));
    let sink = Arc::new(RecordingSink {
        loop_state: loop_state.clone(),
        attempts: Mutex::new(Vec::new()),
    });
    platform.set_refresh_sink(sink.clone())?;
    let ui = AppWindow::new()?;
    platform::configure_window(ui.window(), &platform)?;
    let directory = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let application = App::attach(
        &ui,
        AppOptions {
            data_dir: directory.path().to_path_buf(),
            profile,
            demo: true,
            offline: true,
            initial_screen: "library".into(),
            fonts: Vec::new(),
        },
        platform,
        &runtime,
    )?;
    assert_live_feedback(&ui, &application, &sink);

    ui.global::<AuxiliaryModel>()
        .invoke_event("settings.exit".into(), "".into());
    assert_eq!(
        ui.global::<ShellModel>().get_dialog_kind().as_str(),
        "confirm"
    );
    ui.global::<ShellModel>()
        .invoke_event("dialog-secondary".into(), "".into());
    assert!(ui.global::<ShellModel>().get_dialog_kind().is_empty());
    assert_eq!(loop_state.quit_attempts.load(Ordering::SeqCst), 0);
    assert_live_feedback(&ui, &application, &sink);

    let database = rusqlite::Connection::open(directory.path().join("demo/library.sqlite"))?;
    // Schema triggers make the application's connection fail only this disposable write.
    database.execute_batch(
        "CREATE TRIGGER reject_preferences_insert BEFORE INSERT ON settings
         WHEN NEW.key = 'preferences'
         BEGIN SELECT RAISE(ABORT, 'fixture preferences write failure'); END;
         CREATE TRIGGER reject_preferences_update BEFORE UPDATE OF value ON settings
         WHEN NEW.key = 'preferences'
         BEGIN SELECT RAISE(ABORT, 'fixture preferences write failure'); END;",
    )?;
    let error = application
        .borrow_mut()
        .request_exit()
        .expect_err("Saving preferences must fail before attempting to quit");
    assert!(format!("{error:#}").contains("fixture preferences write failure"));
    assert_eq!(loop_state.quit_attempts.load(Ordering::SeqCst), 0);
    assert_live_feedback(&ui, &application, &sink);
    database.execute_batch(
        "DROP TRIGGER reject_preferences_insert;
         DROP TRIGGER reject_preferences_update;",
    )?;

    loop_state.reject_quit.store(true, Ordering::SeqCst);
    assert!(application.borrow_mut().request_exit().is_err());
    assert_eq!(loop_state.quit_attempts.load(Ordering::SeqCst), 1);
    assert!(!loop_state.terminated.load(Ordering::SeqCst));
    assert_live_feedback(&ui, &application, &sink);
    loop_state.reject_quit.store(false, Ordering::SeqCst);

    ui.global::<AuxiliaryModel>()
        .invoke_event("settings.exit".into(), "".into());
    assert_eq!(
        ui.global::<ShellModel>().get_dialog_kind().as_str(),
        "confirm"
    );
    ui.global::<ShellModel>()
        .set_route("closing-route-sentinel".into());
    sink.take();
    ui.global::<Theme>().invoke_refresh("press".into());
    // This is the same callback invoked by the confirmation button's clicked handler.
    ui.global::<ShellModel>()
        .invoke_event("dialog-primary".into(), "".into());
    assert!(loop_state.terminated.load(Ordering::SeqCst));
    assert_eq!(loop_state.quit_attempts.load(Ordering::SeqCst), 2);
    assert_eq!(
        ui.global::<ShellModel>().get_route().as_str(),
        "closing-route-sentinel",
    );

    {
        let mut state = application.borrow_mut();
        let route = state.route.clone();
        let old_tick = Instant::now() - Duration::from_secs(2);
        state.last_tick = old_tick;
        state.toast_until = Some(Instant::now() - Duration::from_secs(1));
        ui.global::<ShellModel>()
            .set_toast("closing-toast-sentinel".into());
        ui.global::<Theme>().invoke_refresh("release".into());
        ui.global::<Theme>().invoke_refresh("press".into());
        state.dispatch("shell", "tab", "settings")?;
        state.tick()?;
        assert_eq!(state.route, route);
        assert_eq!(state.last_tick, old_tick);
        assert!(state.toast_until.is_some());
    }
    assert_eq!(
        ui.global::<ShellModel>().get_route().as_str(),
        "closing-route-sentinel",
    );
    assert_eq!(
        ui.global::<ShellModel>().get_toast().as_str(),
        "closing-toast-sentinel",
    );
    assert_eq!(
        sink.take(),
        vec![RefreshAttempt {
            plan: RefreshPlan {
                kind: RefreshKind::Fast,
                rect: None,
            },
            after_termination: false,
        }],
    );
    Ok(())
}

#[test]
fn exit_confirmation_gates_late_theme_refreshes_without_borrowing_app_state() {
    // No framebuffer or firmware is opened; a fresh UI thread owns Slint's platform.
    std::thread::spawn(run_closing_scenario)
        .join()
        .expect("The closing fixture UI thread panicked")
        .expect("The closing fixture failed");
}

fn login_close_target(
    ui: &AppWindow,
    adapter: &MinimalSoftwareWindow,
    profile: DeviceProfile,
) -> LogicalPosition {
    let size = ui.window().size();
    let scale = ui.window().scale_factor();
    let mut pixels = vec![
        Rgb8Pixel {
            r: 255,
            g: 255,
            b: 255
        };
        (size.width * size.height) as usize
    ];
    adapter.request_redraw();
    let mut rendered = false;
    adapter.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, size.width as usize);
        rendered = true;
    });
    assert!(
        rendered,
        "The login fixture must paint before receiving input"
    );
    let margin = if profile.is_scribe() { 32.0 } else { 20.0 };
    let top = 28.0 + if profile.is_scribe() { 16.0 } else { 8.0 };
    let position = LogicalPosition::new(size.width as f32 / scale - margin - 24.0, top + 24.0);
    let mut ink = 0;
    for y in ((position.y - 12.0) * scale) as u32..((position.y + 12.0) * scale) as u32 {
        for x in ((position.x - 12.0) * scale) as u32..((position.x + 12.0) * scale) as u32 {
            let pixel = pixels[(y * size.width + x) as usize];
            if pixel.r < 80 && pixel.g < 80 && pixel.b < 80 {
                ink += 1;
            }
        }
    }
    assert!(
        ink >= 12,
        "The close icon must remain visibly painted in every login state: profile={}, size={}x{}, scale={scale}, target=({}, {}), ink={ink}, step={}, busy={}",
        profile.id(),
        size.width,
        size.height,
        position.x,
        position.y,
        ui.global::<LoginModel>().get_step(),
        ui.global::<LoginModel>().get_busy(),
    );
    position
}

fn click_login_close(ui: &AppWindow, position: LogicalPosition) -> Result<()> {
    ui.window()
        .try_dispatch_event(WindowEvent::PointerMoved { position })?;
    ui.window()
        .try_dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        })?;
    ui.window()
        .try_dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        })?;
    Ok(())
}

fn login_close_challenge() -> QrLoginChallenge {
    QrLoginChallenge {
        uid: "login-close-challenge".into(),
        confirm_url: "https://example.invalid/login-close".into(),
        expires_at: chrono::Utc::now().timestamp() + 300,
    }
}

fn run_login_close_scenario(profile: DeviceProfile) -> Result<()> {
    let loop_state = Arc::new(LoopState::default());
    loop_state.reject_quit.store(true, Ordering::SeqCst);
    let ui_platform = TestUiPlatform::new(loop_state.clone());
    let window_slot = ui_platform.window.clone();
    slint::platform::set_platform(Box::new(ui_platform))?;
    let platform = Arc::new(Platform::with_control(
        profile,
        false,
        Arc::new(TestDeviceControl),
    ));
    let ui = AppWindow::new()?;
    platform::configure_window(ui.window(), &platform)?;
    let directory = tempfile::tempdir()?;
    let session = AuthSession {
        api_key: "synthetic-login-close-account".into(),
        user_vid: "login-close-account".into(),
        ..AuthSession::default()
    };
    let session_file = SessionFile::new(directory.path());
    session_file.save(&session)?;
    let saved_account = std::fs::read(directory.path().join("account.json"))?;
    // This runtime is never entered, so login/source work cannot reach the network.
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
            initial_screen: "login".into(),
            fonts: Vec::new(),
        },
        platform,
        &runtime,
    )?;
    ui.show()?;
    let adapter = window_slot.borrow().as_ref().unwrap().clone();
    let mut attempts = 0;
    for step in ["qr", "code", "key"] {
        for (busy, status) in [
            (false, ""),
            (true, "login_connecting"),
            (false, "network_unavailable"),
        ] {
            let stale_request = {
                let mut state = application.borrow_mut();
                state.login_request = state.request();
                state.login_polling = busy;
                state.login_challenge = Some(login_close_challenge());
                state.login_code = "1234".into();
                state.login_key = "synthetic-unsubmitted-key".into();
                let login = ui.global::<LoginModel>();
                login.set_step(step.into());
                login.set_busy(busy);
                login.set_status(if status.is_empty() {
                    "".into()
                } else {
                    tr(status).into()
                });
                state.render()?;
                state.login_request
            };
            let center = login_close_target(&ui, &adapter, profile);
            // The icon itself is 24px, but taps near the edges must hit its 48px button.
            let edge = if attempts % 2 == 0 { -21.0 } else { 21.0 };
            click_login_close(&ui, LogicalPosition::new(center.x + edge, center.y + edge))?;
            attempts += 1;
            assert_eq!(loop_state.quit_attempts.load(Ordering::SeqCst), attempts);
            assert!(!loop_state.terminated.load(Ordering::SeqCst));
            let mut state = application.borrow_mut();
            assert!(!state.closing.get());
            assert_eq!(state.route, Route::Login);
            assert_ne!(state.login_request, stale_request);
            assert!(!state.login_polling);
            assert!(state.login_challenge.is_none());
            assert!(state.login_code.is_empty() && state.login_key.is_empty());
            let login = ui.global::<LoginModel>();
            assert!(!login.get_busy() && !login.get_can_submit());
            assert_eq!(login.get_status().as_str(), tr("operation_failed"));
            let account_epoch = state.jobs.account_epoch();
            assert!(!state.handle_job(JobEvent {
                credits: None,
                request: stale_request,
                account_epoch,
                result: JobResult::LoginBegin(Ok(login_close_challenge())),
            })?);
            assert!(!state.handle_job(JobEvent {
                credits: None,
                request: stale_request,
                account_epoch,
                result: JobResult::LoginPoll(Ok(QrLoginState::Authenticated(AuthSession {
                    api_key: "synthetic-late-account".into(),
                    ..AuthSession::default()
                }))),
            })?);
            assert_eq!(state.jobs.session(), Some(session.clone()));
            assert_eq!(session_file.load()?, Some(session.clone()));
            assert_eq!(
                std::fs::read(directory.path().join("account.json"))?,
                saved_account
            );
        }
    }

    let database = rusqlite::Connection::open(directory.path().join("library.sqlite"))?;
    database.execute_batch(
        "CREATE TRIGGER reject_login_close_save BEFORE UPDATE OF value ON settings
         WHEN NEW.key = 'preferences'
         BEGIN SELECT RAISE(ABORT, 'fixture login exit save failure'); END;",
    )?;
    ui.global::<LoginModel>().set_busy(true);
    let center = login_close_target(&ui, &adapter, profile);
    click_login_close(&ui, center)?;
    assert_eq!(loop_state.quit_attempts.load(Ordering::SeqCst), attempts);
    assert!(!application.borrow().closing.get());
    assert!(!ui.global::<LoginModel>().get_busy());
    assert_eq!(
        ui.global::<LoginModel>().get_status().as_str(),
        tr("operation_failed")
    );
    database.execute_batch("DROP TRIGGER reject_login_close_save;")?;

    loop_state.reject_quit.store(false, Ordering::SeqCst);
    ui.global::<LoginModel>().set_busy(true);
    let center = login_close_target(&ui, &adapter, profile);
    click_login_close(&ui, center)?;
    assert!(loop_state.terminated.load(Ordering::SeqCst));
    assert_eq!(
        loop_state.quit_attempts.load(Ordering::SeqCst),
        attempts + 1
    );
    assert!(application.borrow().closing.get());
    assert_eq!(application.borrow().route, Route::Login);
    assert_eq!(session_file.load()?, Some(session));
    assert_eq!(
        std::fs::read(directory.path().join("account.json"))?,
        saved_account
    );
    let request = application.borrow().login_request;
    ui.global::<LoginModel>()
        .invoke_event("refresh".into(), "".into());
    assert_eq!(application.borrow().login_request, request);
    drop(application);
    drop(ui);
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn login_close_remains_tappable_during_each_step_and_fences_cancelled_account_results() -> Result<()>
{
    run_isolated_closing_scenarios(
        "app::closing_tests::login_close_remains_tappable_during_each_step_and_fences_cancelled_account_results",
        &[
            ("pw6", || {
                run_login_close_scenario(DeviceProfile::Paperwhite6)
            }),
            ("scribe", || run_login_close_scenario(DeviceProfile::Scribe)),
        ],
    )
}

fn run_isolated_closing_scenarios(
    case: &str,
    scenarios: &[(&str, fn() -> Result<()>)],
) -> Result<()> {
    const CHILD_CASE: &str = "WEREAD_CLOSING_TEST_CHILD_CASE";
    const CHILD_SCENARIO: &str = "WEREAD_CLOSING_TEST_CHILD_SCENARIO";
    const COMPLETED: &str = "WEREAD_CLOSING_TEST_COMPLETED";
    if let Some(selected) = std::env::var_os(CHILD_CASE) {
        anyhow::ensure!(
            selected.as_os_str() == std::ffi::OsStr::new(case),
            "The closing child selected a different test case"
        );
        let selected_scenario = std::env::var(CHILD_SCENARIO)?;
        let body = scenarios
            .iter()
            .find(|(name, _)| *name == selected_scenario.as_str())
            .map(|(_, body)| *body)
            .ok_or_else(|| anyhow::anyhow!("The closing child selected an unknown scenario"))?;
        std::thread::spawn(body)
            .join()
            .expect("The isolated closing fixture UI thread panicked")?;
        println!("\n{COMPLETED}:{case}:{selected_scenario}");
        return Ok(());
    }
    // Slint's event-loop proxy is process-global. Each scenario therefore needs
    // both a fresh process and a fresh UI thread, even with serial test execution.
    for (name, _) in scenarios {
        let output = std::process::Command::new(std::env::current_exe()?)
            .args(["--exact", case, "--nocapture", "--test-threads=1"])
            .env(CHILD_CASE, case)
            .env(CHILD_SCENARIO, name)
            .env_remove("WEREAD_API_KEY")
            .env_remove("WEREAD_LAUNCH_GENERATION")
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let marker = format!("{COMPLETED}:{case}:{name}");
        anyhow::ensure!(
            output.status.success()
                && stdout
                    .lines()
                    .filter(|line| *line == marker.as_str())
                    .count()
                    == 1,
            "The isolated closing scenario did not complete exactly once: {case}:{name}\nstatus: {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status,
        );
    }
    Ok(())
}

struct RecordingThemeSink {
    native_base: Option<bool>,
    desired: Mutex<Vec<bool>>,
}

impl RenderSink for RecordingThemeSink {
    fn set_dark_mode(&self, desired_dark: bool) -> Result<(), PlatformError> {
        self.desired.lock().unwrap().push(desired_dark);
        Ok(())
    }

    fn native_output_inverted(&self) -> Option<bool> {
        self.native_base
    }
}

fn run_native_dark_default_exit_scenario(saved_dark: Option<bool>) -> Result<()> {
    let loop_state = Arc::new(LoopState::default());
    let ui_platform = TestUiPlatform::new(loop_state.clone());
    let window_slot = ui_platform.window.clone();
    slint::platform::set_platform(Box::new(ui_platform))?;
    let profile = DeviceProfile::Scribe;
    let platform = Arc::new(Platform::with_control(
        profile,
        true,
        Arc::new(TestDeviceControl),
    ));
    let display = Arc::new(RecordingThemeSink {
        native_base: Some(true),
        desired: Mutex::new(Vec::new()),
    });
    platform.set_render_sink(display.clone())?;
    let directory = tempfile::tempdir()?;
    let expected_dark = true;
    let mut expected_preferences = AppPreferences::for_device(true);
    expected_preferences.dark_mode = expected_dark;
    if saved_dark.is_some() {
        expected_preferences.view_mode = "list".into();
        expected_preferences.pull_on_open = false;
    }
    let database = Store::open(&directory.path().join("library.sqlite"))?;
    if let Some(saved_dark) = saved_dark {
        let mut saved_preferences = expected_preferences.clone();
        saved_preferences.dark_mode = saved_dark;
        database.set_setting("preferences", &saved_preferences)?;
        assert_eq!(
            database.setting::<AppPreferences>("preferences")?,
            Some(saved_preferences)
        );
    } else {
        assert!(database.setting::<AppPreferences>("preferences")?.is_none());
    }
    drop(database);
    let session = AuthSession {
        api_key: "synthetic-native-theme-account".into(),
        user_vid: "native-theme-account".into(),
        ..AuthSession::default()
    };
    let account_file = SessionFile::new(directory.path());
    account_file.save(&session)?;
    let account_bytes = std::fs::read(directory.path().join("account.json"))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let ui = AppWindow::new()?;
    platform::configure_window(ui.window(), &platform)?;
    let application = App::attach(
        &ui,
        AppOptions {
            data_dir: directory.path().to_path_buf(),
            profile,
            demo: false,
            offline: true,
            initial_screen: if saved_dark.is_none() {
                "login"
            } else {
                "settings"
            }
            .into(),
            fonts: Vec::new(),
        },
        platform.clone(),
        &runtime,
    )?;
    assert_eq!(application.borrow().prefs, expected_preferences);
    assert_eq!(*display.desired.lock().unwrap(), vec![expected_dark]);
    assert_eq!(platform.native_output_inverted(), Some(true));

    if saved_dark.is_none() {
        ui.show()?;
        let adapter = window_slot.borrow().as_ref().unwrap().clone();
        let center = login_close_target(&ui, &adapter, profile);
        click_login_close(&ui, center)?;
    } else {
        ui.global::<AuxiliaryModel>()
            .invoke_event("settings.exit".into(), "".into());
        assert_eq!(
            ui.global::<ShellModel>().get_dialog_kind().as_str(),
            "confirm"
        );
        ui.global::<ShellModel>()
            .invoke_event("dialog-primary".into(), "".into());
    }
    assert!(application.borrow().closing.get());
    assert!(loop_state.terminated.load(Ordering::SeqCst));
    assert_eq!(loop_state.quit_attempts.load(Ordering::SeqCst), 1);
    assert_eq!(application.borrow().prefs, expected_preferences);
    assert_eq!(
        application
            .borrow()
            .store
            .setting::<AppPreferences>("preferences")?,
        Some(expected_preferences),
    );
    assert_eq!(*display.desired.lock().unwrap(), vec![expected_dark]);
    assert_eq!(platform.native_output_inverted(), Some(true));
    assert_eq!(account_file.load()?, Some(session));
    assert_eq!(
        std::fs::read(directory.path().join("account.json"))?,
        account_bytes
    );
    drop(application);
    drop(ui);
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn first_login_and_settings_exit_follow_native_dark_without_erasing_other_preferences() -> Result<()>
{
    run_isolated_closing_scenarios(
        "app::closing_tests::first_login_and_settings_exit_follow_native_dark_without_erasing_other_preferences",
        &[
            ("first-use", || run_native_dark_default_exit_scenario(None)),
            ("saved-light", || {
                run_native_dark_default_exit_scenario(Some(false))
            }),
        ],
    )
}
