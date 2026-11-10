//! Terminal cleanup regressions with owned fake workers and isolated UI state.

use super::*;
use crate::{
    jobs::JobResult,
    platform::{DeviceControl, PlatformCapabilities, PlatformError},
};
use slint::{
    EventLoopError,
    platform::{
        EventLoopProxy, Platform as SlintPlatform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::sync::{
    Condvar, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Default)]
struct LoopState {
    quits: AtomicUsize,
}

struct TestUi(Arc<LoopState>);
struct TestProxy(Arc<LoopState>);

impl SlintPlatform for TestUi {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(TestProxy(self.0.clone())))
    }
}

impl EventLoopProxy for TestProxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        self.0.quits.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn invoke_from_event_loop(&self, _: Box<dyn FnOnce() + Send>) -> Result<(), EventLoopError> {
        Err(EventLoopError::EventLoopTerminated)
    }
}

#[derive(Default)]
struct GateState {
    entered: bool,
    released: bool,
}

#[derive(Default)]
struct Gate {
    enabled: AtomicBool,
    active: AtomicUsize,
    completions: AtomicUsize,
    state: Mutex<GateState>,
    changed: Condvar,
}

impl Gate {
    fn arm(&self) {
        self.enabled.store(true, Ordering::Release);
    }

    fn enter(&self) {
        if !self.enabled.load(Ordering::Acquire) {
            return;
        }
        self.active.fetch_add(1, Ordering::AcqRel);
        let mut state = self.state.lock().unwrap();
        state.entered = true;
        self.changed.notify_all();
        while !state.released {
            let (next, timeout) = self
                .changed
                .wait_timeout(state, Duration::from_secs(10))
                .unwrap();
            state = next;
            assert!(
                !timeout.timed_out(),
                "The owned terminal worker gate was not released"
            );
        }
        self.active.fetch_sub(1, Ordering::AcqRel);
        self.completions.fetch_add(1, Ordering::AcqRel);
    }

    fn wait_entered(&self) {
        let state = self.state.lock().unwrap();
        let (state, timeout) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(3), |state| !state.entered)
            .unwrap();
        assert!(
            state.entered && !timeout.timed_out(),
            "The fake terminal worker did not enter"
        );
    }

    fn release(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .released = true;
        self.changed.notify_all();
    }
}

#[derive(Default)]
struct Control {
    light: Gate,
    battery: Gate,
    awake: Gate,
    quiescence_calls: AtomicUsize,
    quiescence_while_active: AtomicUsize,
}

impl Control {
    fn release(&self) {
        self.light.release();
        self.battery.release();
        self.awake.release();
    }
}

impl DeviceControl for Control {
    fn native_viewport(&self) -> Option<(u32, u32)> {
        Some(DeviceProfile::Scribe.physical_size())
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities {
            brightness: true,
            keep_awake: self.awake.enabled.load(Ordering::Acquire),
            ..Default::default()
        }
    }

    fn brightness(&self) -> Result<u8, PlatformError> {
        self.light.enter();
        Ok(8)
    }

    fn set_brightness(&self, _: u8) -> Result<(), PlatformError> {
        self.light.enter();
        Ok(())
    }

    fn warmth(&self) -> Result<u8, PlatformError> {
        Err(PlatformError::Unsupported("fixture_warmth"))
    }

    fn set_warmth(&self, _: u8) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("fixture_warmth"))
    }

    fn battery_percentage(&self) -> Result<u8, PlatformError> {
        self.battery.enter();
        Ok(57)
    }

    fn set_inverted(&self, _: bool) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("fixture_inversion"))
    }

    fn reset_idle_timer(&self) -> Result<(), PlatformError> {
        self.awake.enter();
        Ok(())
    }

    fn quiesce_commands(&self, timeout: Duration) -> Result<(), PlatformError> {
        self.quiescence_calls.fetch_add(1, Ordering::AcqRel);
        assert!(
            timeout.is_zero(),
            "Terminal cleanup must observe existing command ownership"
        );
        if self.light.active.load(Ordering::Acquire)
            + self.battery.active.load(Ordering::Acquire)
            + self.awake.active.load(Ordering::Acquire)
            > 0
        {
            self.quiescence_while_active.fetch_add(1, Ordering::AcqRel);
            return Err(PlatformError::Operation(
                "fixture native command ownership is active".into(),
            ));
        }
        Ok(())
    }
}

struct ReleaseOnDrop(Arc<Control>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct Fixture {
    // Declaration order releases gates before any remaining application owner.
    _release: ReleaseOnDrop,
    application: Option<Rc<RefCell<App>>>,
    _ui: AppWindow,
    _runtime: tokio::runtime::Runtime,
    directory: tempfile::TempDir,
    control: Arc<Control>,
    loop_state: Arc<LoopState>,
}

fn session(identity: &str) -> source::AuthSession {
    source::AuthSession {
        user_vid: format!("terminal-fixture-{identity}"),
        cookies: BTreeMap::from([(
            "wr_skey".into(),
            format!("fixture-terminal-cookie-{identity}"),
        )]),
        ..Default::default()
    }
}

fn fixture() -> Result<Fixture> {
    let loop_state = Arc::new(LoopState::default());
    slint::platform::set_platform(Box::new(TestUi(loop_state.clone())))?;
    let directory = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let control = Arc::new(Control::default());
    let release = ReleaseOnDrop(control.clone());
    SessionFile::new(directory.path()).save(&session("original"))?;
    let platform = Arc::new(Platform::with_control(
        DeviceProfile::Scribe,
        true,
        control.clone(),
    ));
    let ui = AppWindow::new()?;
    crate::platform::configure_window(ui.window(), &platform)?;
    let application = App::attach(
        &ui,
        AppOptions {
            data_dir: directory.path().to_path_buf(),
            profile: DeviceProfile::Scribe,
            demo: false,
            offline: true,
            initial_screen: "library".into(),
            fonts: vec![FontRecord {
                id: "song".into(),
                label: "Terminal fixture font".into(),
                family: "serif".into(),
                metrics: TextMetrics::default(),
                weights: BTreeMap::new(),
                available: true,
            }],
        },
        platform,
        &runtime,
    )?;
    {
        let mut state = application.borrow_mut();
        state.stop_lighting_hardware()?;
        let mut book = Book::new("terminal-book", "Terminal fixture reader");
        book.source = "local".into();
        book.total_chapters = 1;
        state.store.upsert_book(&book)?;
        state.store.upsert_chapter(&Chapter {
            id: "terminal-chapter".into(),
            book_id: book.id.clone(),
            index: 0,
            title: "Terminal fixture chapter".into(),
            cached: true,
            body: Some("An owned terminal reader paragraph. ".repeat(300)),
            content_status: ContentStatus::Full,
            ..Default::default()
        })?;
        state.load_reader(&book.id, 0, Some(0))?;
        state.reader.session_started = Some(Utc::now().timestamp() - 5);
        state.reader.session_seconds = 1;
        state.save_preferences()?;
    }
    Ok(Fixture {
        _release: release,
        application: Some(application),
        _ui: ui,
        _runtime: runtime,
        directory,
        control,
        loop_state,
    })
}

fn arm_workers(fixture: &Fixture, battery: bool) {
    fixture.control.light.arm();
    if battery {
        fixture.control.battery.arm();
    }
    fixture
        .application
        .as_ref()
        .unwrap()
        .borrow_mut()
        .request_device_status();
    fixture.control.light.wait_entered();
    if battery {
        fixture.control.battery.wait_entered();
    }
}

fn queue_checkpoint(state: &mut App) -> tokio::sync::oneshot::Receiver<source::Result<()>> {
    let (sender, events) = mpsc::channel();
    state.events = events;
    let (checkpoint, committed) = tokio::sync::oneshot::channel();
    sender
        .send(JobEvent {
            credits: None,
            request: 907,
            account_epoch: state.jobs.account_epoch(),
            result: JobResult::Downloaded {
                task_id: "queued-terminal-fixture".into(),
                chapter: Chapter {
                    id: "queued-chapter".into(),
                    book_id: "queued-book".into(),
                    index: 0,
                    body: Some("Queued owned text. ".repeat(4096)),
                    ..Default::default()
                },
                next_index: 1,
                html: None,
                metadata: None,
                images: None,
                checkpoint,
            },
        })
        .expect("The fixture owns a live event receiver");
    drop(sender);
    committed
}

fn assert_closed_checkpoint(committed: &mut tokio::sync::oneshot::Receiver<source::Result<()>>) {
    assert!(matches!(
        committed.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    ));
}

fn release_when_started(control: Arc<Control>) -> (mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (sender, started) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _release = ReleaseOnDrop(control.clone());
        let _ = started.recv_timeout(Duration::from_secs(5));
        control.release();
    });
    (sender, worker)
}

#[derive(Clone, Copy)]
enum StorageFault {
    Reader,
    Preferences,
    Account,
}

fn terminal_error_description(error: &anyhow::Error) -> String {
    const LIMIT: usize = 8 * 1024;
    const TRUNCATED: &str = " [truncated]";
    let mut description = format!("{error:#}");
    if description.len() > LIMIT {
        let mut end = LIMIT - TRUNCATED.len();
        while !description.is_char_boundary(end) {
            end -= 1;
        }
        description.truncate(end);
        description.push_str(TRUNCATED);
    }
    description
}

fn assert_preserved_storage_error(error: &anyhow::Error, cause: &str, fault: StorageFault) {
    let description = terminal_error_description(error);
    assert!(
        error.chain().any(|entry| entry.to_string().contains(cause)),
        "The original storage cause is missing from the complete chain: {description}",
    );
    if matches!(fault, StorageFault::Reader | StorageFault::Preferences) {
        // SqliteFailure owns the trigger message; its Error source contains
        // only the numeric SQLite failure. root_cause() selects that source.
        let Some(rusqlite::Error::SqliteFailure(code, Some(message))) =
            error.downcast_ref::<rusqlite::Error>()
        else {
            panic!("The original rusqlite error type or trigger message was lost: {description}");
        };
        assert_eq!(
            code.extended_code,
            rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER,
            "The original trigger error code changed: {description}",
        );
        assert!(
            message.contains(cause),
            "The trigger cause changed: {description}"
        );
    }
}

fn install_fault(fixture: &Fixture, fault: StorageFault) -> Result<&'static str> {
    match fault {
        StorageFault::Reader | StorageFault::Preferences => {
            let (key, cause) = match fault {
                StorageFault::Reader => (
                    "reader.resume.v1",
                    "fixture terminal reader checkpoint failure",
                ),
                _ => ("preferences", "fixture terminal preferences failure"),
            };
            rusqlite::Connection::open(fixture.directory.path().join("library.sqlite"))?
                .execute_batch(&format!(
                    "CREATE TRIGGER reject_terminal_storage BEFORE INSERT ON settings
                     WHEN NEW.key='{key}' BEGIN SELECT RAISE(FAIL, '{cause}'); END;"
                ))?;
            Ok(cause)
        }
        StorageFault::Account => {
            fixture
                .application
                .as_ref()
                .unwrap()
                .borrow()
                .jobs
                .replace_session(session("replacement"))?;
            std::fs::create_dir(fixture.directory.path().join("account.pending"))?;
            Ok("Local account path is not a regular file")
        }
    }
}

fn storage_failure(fault: StorageFault) -> Result<()> {
    let fixture = fixture()?;
    let cause = install_fault(&fixture, fault)?;
    let original_account = std::fs::read(fixture.directory.path().join("account.json"))?;
    arm_workers(&fixture, true);
    let application = fixture.application.as_ref().unwrap();
    let old_epoch = application.borrow().jobs.account_epoch();
    let mut committed = queue_checkpoint(&mut application.borrow_mut());
    let (started, release) = release_when_started(fixture.control.clone());
    started.send(()).unwrap();
    let error = application.borrow_mut().terminal_shutdown().unwrap_err();
    release.join().expect("The terminal gate releaser panicked");
    assert_preserved_storage_error(&error, cause, fault);
    assert_eq!(fixture.control.light.active.load(Ordering::Acquire), 0);
    assert_eq!(fixture.control.battery.active.load(Ordering::Acquire), 0);
    assert_eq!(fixture.control.light.completions.load(Ordering::Acquire), 1);
    assert_eq!(
        fixture.control.battery.completions.load(Ordering::Acquire),
        1
    );
    assert_eq!(fixture.control.quiescence_calls.load(Ordering::Acquire), 1);
    assert_eq!(
        fixture
            .control
            .quiescence_while_active
            .load(Ordering::Acquire),
        0
    );
    assert_closed_checkpoint(&mut committed);
    {
        let mut state = application.borrow_mut();
        assert!(state.jobs.account_epoch() > old_epoch);
        assert!(state.closing.get());
        assert!(state.managed_sleep_blocked);
        assert!(matches!(
            state.events.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
        assert!(!state.jobs.accepts(&JobEvent {
            credits: None,
            request: 909,
            account_epoch: old_epoch,
            result: JobResult::Shelf(Ok(source::ShelfSnapshot::default())),
        }));
        state.stop_lighting_hardware_with_timeout(Duration::ZERO)?;
        let route = state.route.clone();
        state.dispatch("shell", "back", "")?;
        state.tick()?;
        state.request_device_status();
        assert_eq!(state.route, route);
    }
    assert_eq!(fixture.loop_state.quits.load(Ordering::Acquire), 0);
    assert_eq!(
        std::fs::read(fixture.directory.path().join("account.json"))?,
        original_account
    );
    // No trusted launcher context is installed. Actual marker publication is
    // covered by the separate managed-protocol fixtures, not claimed here.
    Ok(())
}

fn fail_synchronous_admission(fixture: &Fixture) -> Result<&'static str> {
    const CAUSE: &str = "fixture admission result failure";
    let mut state = fixture.application.as_ref().unwrap().borrow_mut();
    let capacity = state.jobs.metadata_capacity();
    assert_eq!(capacity, 8);
    // The current-thread runtime is deliberately not driven. Each accepted
    // operation owns admission without constructing a transport request.
    for index in 0..capacity {
        assert!(
            state
                .jobs
                .search(2000 + index as u64, "Unpolled admission fixture".into())
                .is_none()
        );
    }
    let request = source::report::ReadReportRequest {
        session_id: "admission-batch".into(),
        account_key: source::report::account_key(&state.jobs.session().unwrap()).unwrap(),
        book_id: "terminal-book".into(),
        started_at: 100,
        seconds: 60,
        progress_write_allowed: true,
        active_reader_intent: false,
        position: None,
    };
    state.store.set_setting(
        "report.outbox.v1",
        &serde_json::json!({
            "batches": [{
                "request": request,
                "run_id": "admission-run",
                "from": 0,
                "to": 60,
                "state": "sending",
                "attempted_at": 200,
                "retry_after": 0,
                "attempts": 1,
                "error": null
            }],
            "reported_cursors": {"admission-run": 0},
            "compacted_runs": {}
        }),
    )?;
    let connection = rusqlite::Connection::open(fixture.directory.path().join("library.sqlite"))?;
    connection.execute_batch(&format!(
        "CREATE TRIGGER reject_admission_result BEFORE INSERT ON settings
         WHEN NEW.key='report.failure:admission-batch'
         BEGIN SELECT RAISE(FAIL, '{CAUSE}'); END;"
    ))?;
    let rejection = state.jobs.report_time(3000, request);
    assert!(matches!(
        &rejection,
        Some(JobEvent {
            result: JobResult::ReadReport {
                result: Err(source::SourceError::Busy),
                ..
            },
            ..
        })
    ));
    assert!(state.settle_job_submission(rejection));
    assert_preserved_storage_error(
        state.immediate_job_error.as_ref().unwrap(),
        CAUSE,
        StorageFault::Preferences,
    );
    // Later saves can succeed. Only the already observed handler failure must
    // prevent this same-turn exit from acknowledging a successful checkpoint.
    connection.execute_batch("DROP TRIGGER reject_admission_result")?;
    Ok(CAUSE)
}

fn same_turn_admission_failure_keeps_recoverable_exit_open() -> Result<()> {
    let fixture = fixture()?;
    let cause = fail_synchronous_admission(&fixture)?;
    let application = fixture.application.as_ref().unwrap();
    let error = application.borrow_mut().request_exit().unwrap_err();
    assert_preserved_storage_error(&error, cause, StorageFault::Preferences);
    assert_eq!(fixture.loop_state.quits.load(Ordering::Acquire), 0);
    {
        let mut state = application.borrow_mut();
        assert!(!state.closing.get());
        assert!(!state.managed_sleep_blocked);
        assert!(
            state.immediate_job_error.is_none(),
            "A returned error must not be reported twice"
        );
        assert!(state.reader.session_started.is_some());
        state.last_tick = Instant::now() + Duration::from_secs(3600);
        state.tick()?;
        state.request_exit()?;
        assert!(state.closing.get());
    }
    assert_eq!(fixture.loop_state.quits.load(Ordering::Acquire), 1);
    // The fixture has no trusted launcher ancestry. It verifies the common
    // success gate and proxy; marker contents remain covered by protocol tests.
    Ok(())
}

fn same_turn_admission_failure_keeps_terminal_cleanup_unconditional() -> Result<()> {
    let fixture = fixture()?;
    let cause = fail_synchronous_admission(&fixture)?;
    let later = install_fault(&fixture, StorageFault::Preferences)?;
    arm_workers(&fixture, true);
    let application = fixture.application.as_ref().unwrap();
    let old_epoch = application.borrow().jobs.account_epoch();
    let mut committed = queue_checkpoint(&mut application.borrow_mut());
    let (started, release) = release_when_started(fixture.control.clone());
    started.send(()).unwrap();
    let error = application.borrow_mut().terminal_shutdown().unwrap_err();
    release
        .join()
        .expect("The admission gate releaser panicked");
    assert_preserved_storage_error(&error, cause, StorageFault::Preferences);
    assert!(
        terminal_error_description(&error).contains(later),
        "The later save failure was omitted from the original handler error"
    );
    assert_eq!(fixture.control.light.active.load(Ordering::Acquire), 0);
    assert_eq!(fixture.control.battery.active.load(Ordering::Acquire), 0);
    assert_eq!(fixture.control.light.completions.load(Ordering::Acquire), 1);
    assert_eq!(
        fixture.control.battery.completions.load(Ordering::Acquire),
        1
    );
    assert_eq!(fixture.control.quiescence_calls.load(Ordering::Acquire), 1);
    assert_eq!(
        fixture
            .control
            .quiescence_while_active
            .load(Ordering::Acquire),
        0
    );
    assert_closed_checkpoint(&mut committed);
    let state = application.borrow();
    assert!(state.closing.get());
    assert!(state.managed_sleep_blocked);
    assert!(state.immediate_job_error.is_none());
    assert!(state.jobs.account_epoch() > old_epoch);
    assert!(matches!(
        state.events.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
    assert_eq!(fixture.loop_state.quits.load(Ordering::Acquire), 0);
    Ok(())
}

fn database_snapshot(path: &std::path::Path) -> Result<serde_json::Value> {
    let connection = rusqlite::Connection::open(path)?;
    let settings = {
        let mut statement = connection.prepare("SELECT key,value FROM settings ORDER BY key")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let sessions: u64 =
        connection.query_row("SELECT COUNT(*) FROM reading_sessions", [], |row| {
            row.get(0)
        })?;
    let positions: u64 =
        connection.query_row("SELECT COUNT(*) FROM positions", [], |row| row.get(0))?;
    Ok(serde_json::json!({"settings": settings, "sessions": sessions, "positions": positions}))
}

fn unexpected_drop() -> Result<()> {
    let mut fixture = fixture()?;
    let application = fixture.application.take().unwrap();
    let _release = ReleaseOnDrop(fixture.control.clone());
    {
        let mut state = application.borrow_mut();
        state.jobs.replace_session(session("unsaved"))?;
        state.prefs.dark_mode = !state.prefs.dark_mode;
        state.store.delete_setting("reader.resume.v1")?;
    }
    fixture.control.light.arm();
    fixture.control.battery.arm();
    application.borrow_mut().request_device_status();
    fixture.control.light.wait_entered();
    fixture.control.battery.wait_entered();
    let mut committed = queue_checkpoint(&mut application.borrow_mut());
    let weak = Rc::downgrade(&application);
    let path = fixture.directory.path().join("library.sqlite");
    let before = database_snapshot(&path)?;
    let account = std::fs::read(fixture.directory.path().join("account.json"))?;
    let (started, release) = release_when_started(fixture.control.clone());
    started.send(()).unwrap();
    drop(application);
    release.join().expect("The drop gate releaser panicked");
    assert!(weak.upgrade().is_none());
    assert_eq!(fixture.control.light.active.load(Ordering::Acquire), 0);
    assert_eq!(fixture.control.battery.active.load(Ordering::Acquire), 0);
    assert_eq!(fixture.control.quiescence_calls.load(Ordering::Acquire), 1);
    assert_eq!(
        fixture
            .control
            .quiescence_while_active
            .load(Ordering::Acquire),
        0
    );
    assert_closed_checkpoint(&mut committed);
    assert_eq!(
        database_snapshot(&path)?,
        before,
        "Unexpected Drop saved application state"
    );
    assert_eq!(
        std::fs::read(fixture.directory.path().join("account.json"))?,
        account
    );
    assert_eq!(fixture.loop_state.quits.load(Ordering::Acquire), 0);
    Ok(())
}

fn cleanup_failure_retains_owner_and_original_cause() -> Result<()> {
    let fixture = fixture()?;
    let cause = install_fault(&fixture, StorageFault::Preferences)?;
    arm_workers(&fixture, false);
    let application = fixture.application.as_ref().unwrap();
    let mut committed = queue_checkpoint(&mut application.borrow_mut());
    let error = application.borrow_mut().terminal_shutdown().unwrap_err();
    assert_preserved_storage_error(&error, cause, StorageFault::Preferences);
    let complete = terminal_error_description(&error);
    assert!(
        complete.contains("lighting and battery"),
        "Lighting cleanup error is missing: {complete}"
    );
    assert!(
        complete.contains("native command ownership"),
        "Command cleanup error is missing: {complete}"
    );
    assert_eq!(fixture.control.light.active.load(Ordering::Acquire), 1);
    assert_eq!(fixture.control.quiescence_calls.load(Ordering::Acquire), 1);
    assert_eq!(
        fixture
            .control
            .quiescence_while_active
            .load(Ordering::Acquire),
        1
    );
    assert_closed_checkpoint(&mut committed);
    // An in-flight join owner cannot disappear just because terminal saving
    // failed. A zero-budget retry must still see the same unfinished worker.
    assert!(
        application
            .borrow_mut()
            .stop_lighting_hardware_with_timeout(Duration::ZERO)
            .is_err()
    );
    fixture.control.release();
    let retried = application.borrow_mut().terminal_shutdown().unwrap_err();
    assert_preserved_storage_error(&retried, cause, StorageFault::Preferences);
    let retried_description = terminal_error_description(&retried);
    assert!(
        !retried_description.contains("Terminal resource cleanup also failed"),
        "Released cleanup still reported an error: {retried_description}"
    );
    assert_eq!(fixture.control.light.active.load(Ordering::Acquire), 0);
    assert_eq!(fixture.control.light.completions.load(Ordering::Acquire), 1);
    assert_eq!(fixture.control.quiescence_calls.load(Ordering::Acquire), 2);
    assert_eq!(
        fixture
            .control
            .quiescence_while_active
            .load(Ordering::Acquire),
        1
    );
    assert_eq!(fixture.loop_state.quits.load(Ordering::Acquire), 0);
    Ok(())
}

fn blocked_wake_uses_one_terminal_budget(terminal_first: bool) -> Result<()> {
    let mut fixture = fixture()?;
    let cause = if terminal_first {
        Some(install_fault(&fixture, StorageFault::Preferences)?)
    } else {
        None
    };
    let application = fixture.application.take().unwrap();
    let _release = ReleaseOnDrop(fixture.control.clone());
    fixture.control.awake.arm();
    {
        let mut state = application.borrow_mut();
        state.prefs.keep_awake = true;
        state.current_download = Some(DownloadTask {
            id: "terminal-wake-task".into(),
            book_id: "terminal-book".into(),
            chapters: vec![0],
            status: DownloadStatus::Running,
            ..Default::default()
        });
        // Only the application wake-maintenance owner starts. No source
        // download or real hardware operation is submitted by this fixture.
        state.start_task_wake_guard();
    }
    fixture.control.awake.wait_entered();
    let path = fixture.directory.path().join("library.sqlite");
    let before = database_snapshot(&path)?;
    let account = std::fs::read(fixture.directory.path().join("account.json"))?;
    let started = Instant::now();
    if let Some(cause) = cause {
        let error = application.borrow_mut().terminal_shutdown().unwrap_err();
        assert_preserved_storage_error(&error, cause, StorageFault::Preferences);
        let complete = terminal_error_description(&error);
        assert!(
            complete.contains("wake maintenance"),
            "Wake cleanup error is missing: {complete}"
        );
        assert_eq!(fixture.control.awake.active.load(Ordering::Acquire), 1);
        let dropping = Instant::now();
        drop(application);
        assert!(
            dropping.elapsed() < Duration::from_millis(1500),
            "App/field Drop spent a second terminal wake budget"
        );
    } else {
        drop(application);
        assert_eq!(database_snapshot(&path)?, before);
        assert_eq!(
            std::fs::read(fixture.directory.path().join("account.json"))?,
            account
        );
    }
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "Terminal destruction repeated the 2250 ms wake join budget"
    );
    let observations = if terminal_first { 2 } else { 1 };
    assert_eq!(
        fixture.control.quiescence_calls.load(Ordering::Acquire),
        observations
    );
    assert_eq!(
        fixture
            .control
            .quiescence_while_active
            .load(Ordering::Acquire),
        observations
    );
    fixture.control.release();
    let deadline = Instant::now() + Duration::from_secs(2);
    while fixture.control.awake.active.load(Ordering::Acquire) != 0
        || fixture.control.awake.completions.load(Ordering::Acquire) == 0
    {
        assert!(
            Instant::now() < deadline,
            "The released fake wake worker did not finish"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(fixture.control.awake.completions.load(Ordering::Acquire), 1);
    assert_eq!(fixture.loop_state.quits.load(Ordering::Acquire), 0);
    Ok(())
}

fn isolated(case: &'static str, scenario: fn() -> Result<()>) -> Result<()> {
    const CHILD: &str = "WEREAD_TERMINAL_CHILD";
    const COMPLETED: &str = "WEREAD_TERMINAL_COMPLETED";
    if std::env::var(CHILD).ok().as_deref() == Some(case) {
        std::thread::spawn(scenario)
            .join()
            .expect("The terminal fixture thread panicked")?;
        println!("\n{COMPLETED}:{case}");
        return Ok(());
    }
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .args(["--exact", case, "--nocapture", "--test-threads=1"])
        .env(CHILD, case)
        .env_remove("WEREAD_API_KEY");
    for name in [
        "WEREAD_LAUNCH_CONTEXT_DIR",
        "WEREAD_LAUNCH_GENERATION",
        "WEREAD_LAUNCH_TOKEN",
        "WEREAD_RESUME_SESSION",
        "WEREAD_NATIVE_RETURN",
        "WEREAD_NATIVE_RETURN_RESULT",
    ] {
        command.env_remove(name);
    }
    let output = command.output()?;
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
        "The isolated terminal fixture did not complete: {}\n{stdout}\n{stderr}",
        output.status
    );
    Ok(())
}

#[test]
fn terminal_reader_write_failure_still_joins_producers_and_closes_queued_packets() -> Result<()> {
    isolated(
        "app::terminal_tests::terminal_reader_write_failure_still_joins_producers_and_closes_queued_packets",
        || storage_failure(StorageFault::Reader),
    )
}

#[test]
fn terminal_preferences_write_failure_still_joins_producers_and_preserves_the_cause() -> Result<()>
{
    isolated(
        "app::terminal_tests::terminal_preferences_write_failure_still_joins_producers_and_preserves_the_cause",
        || storage_failure(StorageFault::Preferences),
    )
}

#[test]
fn terminal_account_write_failure_still_joins_workers_without_replacing_credentials() -> Result<()>
{
    isolated(
        "app::terminal_tests::terminal_account_write_failure_still_joins_workers_without_replacing_credentials",
        || storage_failure(StorageFault::Account),
    )
}

#[test]
fn unexpected_complete_app_drop_releases_producers_without_saving_state() -> Result<()> {
    isolated(
        "app::terminal_tests::unexpected_complete_app_drop_releases_producers_without_saving_state",
        unexpected_drop,
    )
}

#[test]
fn terminal_cleanup_failure_preserves_the_storage_cause_and_the_join_owner() -> Result<()> {
    isolated(
        "app::terminal_tests::terminal_cleanup_failure_preserves_the_storage_cause_and_the_join_owner",
        cleanup_failure_retains_owner_and_original_cause,
    )
}

#[test]
fn terminal_and_field_drop_share_one_deadline_for_a_blocked_wake_worker() -> Result<()> {
    isolated(
        "app::terminal_tests::terminal_and_field_drop_share_one_deadline_for_a_blocked_wake_worker",
        || blocked_wake_uses_one_terminal_budget(true),
    )
}

#[test]
fn unexpected_drop_does_not_repeat_the_blocked_wake_field_join_budget() -> Result<()> {
    isolated(
        "app::terminal_tests::unexpected_drop_does_not_repeat_the_blocked_wake_field_join_budget",
        || blocked_wake_uses_one_terminal_budget(false),
    )
}

#[test]
fn synchronous_admission_failure_before_exit_preserves_the_cause_and_allows_retry() -> Result<()> {
    isolated(
        "app::terminal_tests::synchronous_admission_failure_before_exit_preserves_the_cause_and_allows_retry",
        same_turn_admission_failure_keeps_recoverable_exit_open,
    )
}

#[test]
fn synchronous_admission_failure_before_terminal_exit_still_releases_owned_resources() -> Result<()>
{
    isolated(
        "app::terminal_tests::synchronous_admission_failure_before_terminal_exit_still_releases_owned_resources",
        same_turn_admission_failure_keeps_terminal_cleanup_unconditional,
    )
}
