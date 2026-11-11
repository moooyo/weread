//! Resident lifecycle regressions without native power or network operations.

use super::*;
use crate::{
    jobs::JobResult,
    platform::{DeviceControl, ManagedSleepState, PlatformCapabilities, PlatformError, RenderSink},
};
use slint::platform::{
    EventLoopProxy, Platform as SlintPlatform, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use slint::{EventLoopError, Model};
use std::{
    path::Path,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

struct LifecycleUi;

impl SlintPlatform for LifecycleUi {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}

struct LifecycleDisplay {
    state: Mutex<ManagedSleepState>,
    dark: AtomicBool,
    quit_attempts: AtomicUsize,
}

impl Default for LifecycleDisplay {
    fn default() -> Self {
        Self {
            state: Mutex::new(ManagedSleepState::Running),
            dark: AtomicBool::new(false),
            quit_attempts: AtomicUsize::new(0),
        }
    }
}

struct LifecycleQuitUi(Arc<LifecycleDisplay>);

impl SlintPlatform for LifecycleQuitUi {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(LifecycleQuitProxy(self.0.clone())))
    }
}

struct LifecycleQuitProxy(Arc<LifecycleDisplay>);

impl EventLoopProxy for LifecycleQuitProxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        self.0.quit_attempts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn invoke_from_event_loop(&self, _: Box<dyn FnOnce() + Send>) -> Result<(), EventLoopError> {
        // This fixture has no running loop; only explicit quit is supported.
        Err(EventLoopError::EventLoopTerminated)
    }
}

impl LifecycleDisplay {
    fn set_state(&self, state: ManagedSleepState) {
        *self.state.lock().unwrap() = state;
    }
}

impl RenderSink for LifecycleDisplay {
    fn set_dark_mode(&self, enabled: bool) -> Result<(), PlatformError> {
        self.dark.store(enabled, Ordering::Release);
        Ok(())
    }

    fn managed_sleep_supported(&self) -> bool {
        true
    }

    fn set_managed_sleep(&self, sleeping: bool) -> Result<(), PlatformError> {
        self.set_state(if sleeping {
            ManagedSleepState::Suspended
        } else {
            ManagedSleepState::Running
        });
        Ok(())
    }

    fn managed_sleep_state(&self) -> Result<ManagedSleepState, PlatformError> {
        Ok(*self.state.lock().unwrap())
    }
}

fn on_lifecycle_thread(
    scenario: fn(&AppWindow, &Rc<RefCell<App>>, &Arc<LifecycleDisplay>, &Path) -> Result<()>,
) -> Result<()> {
    on_lifecycle_thread_with_runtime(move |ui, application, display, path, _| {
        scenario(ui, application, display, path)
    })
}

fn on_lifecycle_thread_with_runtime(
    scenario: impl FnOnce(
        &AppWindow,
        &Rc<RefCell<App>>,
        &Arc<LifecycleDisplay>,
        &Path,
        &tokio::runtime::Runtime,
    ) -> Result<()>
    + Send
    + 'static,
) -> Result<()> {
    on_lifecycle_thread_with_runtime_mode(scenario, false)
}

fn on_lifecycle_thread_with_runtime_mode(
    scenario: impl FnOnce(
        &AppWindow,
        &Rc<RefCell<App>>,
        &Arc<LifecycleDisplay>,
        &Path,
        &tokio::runtime::Runtime,
    ) -> Result<()>
    + Send
    + 'static,
    accept_quit: bool,
) -> Result<()> {
    std::thread::spawn(move || -> Result<()> {
        let display = Arc::new(LifecycleDisplay::default());
        if accept_quit {
            slint::platform::set_platform(Box::new(LifecycleQuitUi(display.clone())))?;
        } else {
            slint::platform::set_platform(Box::new(LifecycleUi))?;
        }
        let directory = tempfile::tempdir()?;
        // Existing scenarios never drive the runtime. Retry scenarios may only
        // drive old work after the lifecycle has fenced its account epoch.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let platform = Arc::new(Platform::new(DeviceProfile::Scribe, false)?);
        platform.set_render_sink(display.clone())?;
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
                    label: "Resident lifecycle fixture font".into(),
                    family: "serif".into(),
                    metrics: TextMetrics::default(),
                    weights: BTreeMap::new(),
                    available: true,
                }],
            },
            platform,
            &runtime,
        )?;
        application.borrow_mut().stop_lighting_hardware()?;
        scenario(&ui, &application, &display, directory.path(), &runtime)?;
        application.borrow_mut().stop_lighting_hardware()?;
        drop(application);
        drop(ui);
        runtime.shutdown_timeout(Duration::from_secs(1));
        Ok(())
    })
    .join()
    .expect("The resident lifecycle fixture thread must not panic")
}

fn fixture_session() -> source::AuthSession {
    source::AuthSession {
        user_vid: "resident-lifecycle-account".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "resident-lifecycle-account".into()),
            ("wr_skey".into(), "synthetic-resident-session".into()),
        ]),
        ..source::AuthSession::default()
    }
}

fn seed_reader(state: &mut App) -> Result<()> {
    state.jobs.replace_session(fixture_session())?;
    let mut book = Book::new("resident-reader", "Resident reader fixture");
    book.total_chapters = 1;
    state.store.upsert_book(&book)?;
    state.store.upsert_chapter(&Chapter {
        id: "resident-reader-chapter".into(),
        book_id: book.id.clone(),
        index: 0,
        title: "A retained chapter".into(),
        cached: true,
        body: Some("A retained paragraph for a resident reading lifecycle. ".repeat(600)),
        content_status: ContentStatus::Full,
        ..Chapter::default()
    })?;
    state.load_reader(&book.id, 0, Some(4000))?;
    state.reader.session_started = Some(Utc::now().timestamp() - 60);
    state.reader.session_seconds = 23;
    state.reader.position_confirmed = true;
    state.last_tick = Instant::now();
    state.render()?;
    Ok(())
}

fn paginator_address(state: &App) -> usize {
    state.reader.paginator.as_ref().unwrap() as *const Paginator as usize
}

fn resident_buffer_addresses(state: &App) -> (usize, usize) {
    (
        state.reader.paginator.as_ref().unwrap().pages.as_ptr() as usize,
        state.reader.body.as_ptr() as usize,
    )
}

fn run_retained_reader(
    ui: &AppWindow,
    application: &Rc<RefCell<App>>,
    display: &Arc<LifecycleDisplay>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    state.history = vec![Route::Library, Route::Detail("resident-reader".into())];
    state.reader.draft = PinyinInput::new("An unfinished thought remains in memory.");
    let selected_start = state
        .reader
        .paginator
        .as_ref()
        .unwrap()
        .current_page()
        .start
        + 2;
    let selection = Some((selected_start, selected_start + 6));
    state.reader.selection = selection;
    state.reader.cloud = Some(source::ReadingPosition {
        chapter_uid: "resident-reader-chapter".into(),
        chapter_index: 1,
        chapter_offset: 1,
        updated_at: 20,
        ..source::ReadingPosition::default()
    });
    ui.global::<ReaderModel>().set_writing_thought(true);
    state.reader_action("press", "0.4,40")?;
    assert!(state.reader.press.is_some());
    let page = state
        .reader
        .paginator
        .as_ref()
        .unwrap()
        .current_page()
        .clone();
    assert!(page.start > 0);
    let address = paginator_address(&state);
    let buffers = resident_buffer_addresses(&state);
    let body = state.reader.body.clone();
    let route = state.route.clone();
    let history = state.history.clone();
    let preferences = state.reader.preferences.clone();
    let draft = state.reader.draft.query.clone();
    let cloud = state.reader.cloud.as_ref().unwrap().clone();
    let note = Annotation {
        id: "resident-note".into(),
        book_id: "resident-reader".into(),
        chapter_index: 0,
        start: page.start + 2,
        end: page.start + 8,
        quote: body.chars().skip(page.start as usize + 2).take(6).collect(),
        note: "Keep the saved thought".into(),
        created_at: 1,
    };
    let bookmark = Bookmark {
        id: "resident-bookmark".into(),
        book_id: note.book_id.clone(),
        chapter_index: 0,
        char_offset: page.start + 3,
        label: "Keep this anchor".into(),
        created_at: 1,
    };
    state.store.save_annotation(&note)?;
    state.store.save_bookmark(&bookmark)?;
    let pending = source::ReadingPosition {
        chapter_uid: "resident-reader-chapter".into(),
        chapter_index: 1,
        chapter_offset: 3,
        updated_at: 2,
        ..source::ReadingPosition::default()
    };
    state
        .store
        .set_setting("progress.pending:resident-reader", &pending)?;
    state.prefs.dark_mode = true;
    display.set_dark_mode(true)?;
    let epoch = state.jobs.account_epoch();

    state.prepare_resident_sleep()?;
    assert!(state.managed_sleep_blocked);
    assert!(state.resident_lifecycle.is_prepared());
    assert!(state.reader.press.is_none());
    assert_eq!(state.reader.session_started, None);
    assert_eq!(state.reader.session_seconds, 0);
    assert_eq!(state.jobs.account_epoch(), epoch + 1);
    assert_eq!(state.jobs.session(), Some(fixture_session()));
    assert_eq!(
        state
            .store
            .position("resident-reader")?
            .unwrap()
            .char_offset,
        page.start
    );
    assert_eq!(
        state
            .store
            .sessions(i64::MIN, i64::MAX)?
            .iter()
            .map(|session| session.seconds)
            .sum::<u64>(),
        23
    );
    state.prepare_resident_sleep()?;
    assert_eq!(state.jobs.account_epoch(), epoch + 1);
    assert_eq!(state.store.sessions(i64::MIN, i64::MAX)?.len(), 1);

    let old_tick = Instant::now() - Duration::from_secs(3600);
    state.last_tick = old_tick;
    state.reader.last_active = old_tick;
    state.dispatch("reader", "turn", "1")?;
    state.dispatch("shell", "tab", "settings")?;
    state.tick()?;
    assert_eq!(state.last_tick, old_tick);
    assert_eq!(state.route, route);
    assert_eq!(
        state.reader.paginator.as_ref().unwrap().current_page(),
        &page
    );

    state.resume_resident_sleep()?;
    assert!(!state.managed_sleep_blocked);
    assert!(!state.resident_lifecycle.is_prepared());
    assert!(state.last_tick.elapsed() < Duration::from_secs(30));
    assert!(state.reader.last_active.elapsed() < Duration::from_secs(30));
    assert!(state.reader.session_started.is_some());
    assert_eq!(state.reader.session_seconds, 0);
    assert_eq!(paginator_address(&state), address);
    assert_eq!(resident_buffer_addresses(&state), buffers);
    assert_eq!(
        state.reader.paginator.as_ref().unwrap().current_page(),
        &page
    );
    assert_eq!(state.reader.body, body);
    assert_eq!(state.reader.book_id, "resident-reader");
    assert_eq!(state.reader.chapter_index, 0);
    assert_eq!(state.reader.preferences, preferences);
    assert_eq!(state.reader.selection, selection);
    assert_eq!(state.reader.draft.query, draft);
    assert_eq!(state.history, history);
    assert_eq!(state.route, route);
    assert_eq!(
        state.reader.cloud.as_ref().unwrap().chapter_offset,
        cloud.chapter_offset
    );
    assert_eq!(state.store.annotations("resident-reader")?, vec![note]);
    assert_eq!(state.store.bookmarks("resident-reader")?, vec![bookmark]);
    assert_eq!(
        state
            .store
            .setting::<source::ReadingPosition>("progress.pending:resident-reader")?
            .unwrap()
            .chapter_offset,
        pending.chapter_offset
    );
    assert!(ui.global::<ReaderModel>().get_writing_thought());
    assert!(state.prefs.dark_mode && display.dark.load(Ordering::Acquire));
    let resumed_tick = state.last_tick;
    state.resume_resident_sleep()?;
    assert_eq!(state.last_tick, resumed_tick);
    state.tick()?;
    assert!(state.reader.session_seconds <= 1);
    assert_eq!(
        state
            .store
            .sessions(i64::MIN, i64::MAX)?
            .iter()
            .map(|session| session.seconds)
            .sum::<u64>(),
        23
    );
    Ok(())
}

#[test]
fn resident_sleep_retains_the_live_reader_and_excludes_sleep_from_reading_time() -> Result<()> {
    on_lifecycle_thread(run_retained_reader)
}

fn has_selected_mark(ui: &AppWindow) -> bool {
    ui.global::<ReaderModel>()
        .get_lines()
        .iter()
        .any(|line| line.marks.iter().any(|mark| mark.selected))
}

fn run_transient_selection(
    ui: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Arc<LifecycleDisplay>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    let page_start = state
        .reader
        .paginator
        .as_ref()
        .unwrap()
        .current_page()
        .start;
    let range = (page_start + 2, page_start + 8);
    let note = Annotation {
        id: "resident-saved-underline".into(),
        book_id: "resident-reader".into(),
        chapter_index: 0,
        start: range.0,
        end: range.1,
        quote: state
            .reader
            .body
            .chars()
            .skip(range.0 as usize)
            .take(6)
            .collect(),
        note: "A saved underline survives cancellation of a held selection".into(),
        created_at: 1,
    };
    state.store.save_annotation(&note)?;
    state.reader_action("press", "0.4,40")?;
    state.reader.selection = Some(range);
    ui.global::<ReaderModel>().set_writing_thought(false);
    ui.global::<ReaderModel>().set_selection_open(true);
    state.render()?;
    assert!(state.reader.press.is_some());
    assert!(
        has_selected_mark(ui),
        "The fixture must display the transient selection before sleep"
    );
    let press_generation = ui.global::<ReaderModel>().get_press_generation();
    let address = paginator_address(&state);
    let buffers = resident_buffer_addresses(&state);

    state.prepare_resident_sleep()?;
    assert!(state.reader.press.is_none());
    assert!(state.reader.selection.is_none());
    assert!(!ui.global::<ReaderModel>().get_selection_open());
    assert!(!has_selected_mark(ui));
    assert_ne!(
        ui.global::<ReaderModel>().get_press_generation(),
        press_generation
    );
    assert!(
        ui.global::<ReaderModel>()
            .get_lines()
            .iter()
            .any(|line| { line.marks.iter().any(|mark| mark.mine && !mark.selected) }),
        "Only the transient underline is removed from the retained model"
    );

    state.resume_resident_sleep()?;
    // A held-pointer timer queued before suspension must not recreate its
    // selection after input is re-enabled in the same process.
    state.dispatch("reader", "hold", &press_generation.to_string())?;
    assert!(state.reader.press.is_none());
    assert!(state.reader.selection.is_none());
    assert!(!has_selected_mark(ui));
    assert_eq!(paginator_address(&state), address);
    assert_eq!(resident_buffer_addresses(&state), buffers);
    assert_eq!(state.store.annotations("resident-reader")?, vec![note]);
    Ok(())
}

#[test]
fn resident_sleep_removes_a_held_selection_from_the_retained_frame_without_losing_saved_marks()
-> Result<()> {
    on_lifecycle_thread(run_transient_selection)
}

fn seed_downloads(state: &mut App) -> Result<String> {
    let owner = state
        .jobs
        .session()
        .as_ref()
        .and_then(library::download_account_key)
        .unwrap();
    let mut book = Book::new("resident-download-book", "Resident download fixture");
    book.source = "weread".into();
    book.total_chapters = 3;
    state.store.upsert_book(&book)?;
    for index in 0..3 {
        state.store.upsert_chapter(&Chapter {
            id: format!("resident-download-chapter-{index}"),
            book_id: book.id.clone(),
            index,
            title: format!("Download chapter {index}"),
            body: (index == 0).then(|| "An acknowledged complete chapter.".into()),
            cached: index == 0,
            content_status: if index == 0 {
                ContentStatus::Full
            } else {
                ContentStatus::Unknown
            },
            content_account_key: if index == 0 {
                owner.clone()
            } else {
                String::new()
            },
        })?;
    }
    Ok(owner)
}

fn download_task(id: &str, owner: &str, status: DownloadStatus) -> DownloadTask {
    DownloadTask {
        id: id.into(),
        book_id: "resident-download-book".into(),
        account_key: owner.into(),
        chapters: vec![0, 1, 2],
        next_index: 1,
        status,
        include_images: false,
        updated_at: 100,
        ..DownloadTask::default()
    }
}

fn run_download_preservation(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Arc<LifecycleDisplay>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    let owner = seed_downloads(&mut state)?;
    let active = download_task("resident-active-download", &owner, DownloadStatus::Running);
    let queued = download_task("resident-queued-download", &owner, DownloadStatus::Pending);
    let mut blocked = download_task("resident-locked-download", &owner, DownloadStatus::Paused);
    blocked.error = Some("source.unlock_required".into());
    state.store.save_task(&active)?;
    state.store.save_task(&queued)?;
    state.store.save_task(&blocked)?;
    state.current_download = Some(active.clone());
    state.downloads = VecDeque::from([queued.clone(), blocked.clone()]);
    state.download_request = 71;
    let headers = state.store.chapter_metadata(&active.book_id)?;
    let plan = crate::jobs::DownloadPlan::from_headers(&active, &headers)?;
    state.jobs.download(71, active.clone(), plan);
    assert_eq!(state.jobs.active_download(), Some(active.id.clone()));
    state
        .pending_catalog_downloads
        .insert("resident-future-book".into());
    state
        .pending_catalog_images
        .insert("resident-future-book".into(), true);
    state.pending_downloads = vec![(active.book_id.clone(), vec![1, 2])];
    state.pending_read = Some(("resident-reader".into(), 0));
    state
        .catalog_requests
        .insert("resident-future-book".into(), 72);
    state.book_requests.insert(active.book_id.clone(), 73);
    state.pending_covers.insert(active.book_id.clone());
    state
        .pending_articles
        .insert("resident-pending-article".into());
    let pending_downloads = state.pending_downloads.clone();

    state.prepare_resident_sleep()?;
    let paused = state.store.task(&active.id)?.unwrap();
    assert_eq!(paused.status, DownloadStatus::Paused);
    assert_eq!(paused.next_index, active.next_index);
    assert_eq!(paused.chapters, active.chapters);
    assert_eq!(paused.account_key, active.account_key);
    assert_eq!(paused.include_images, active.include_images);
    assert!(state.current_download.is_none());
    assert!(state.jobs.active_download().is_none());
    assert_eq!(
        state.downloads,
        VecDeque::from([paused.clone(), queued, blocked.clone()])
    );
    assert_eq!(state.pending_downloads, pending_downloads);
    assert_eq!(state.pending_read, Some(("resident-reader".into(), 0)));
    assert!(
        state
            .pending_catalog_downloads
            .contains("resident-future-book")
    );
    assert_eq!(
        state.pending_catalog_images.get("resident-future-book"),
        Some(&true)
    );
    assert!(state.catalog_requests.is_empty() && state.book_requests.is_empty());
    assert!(state.pending_covers.is_empty() && state.pending_articles.is_empty());
    state.resume_resident_sleep()?;
    state.tick()?;
    // The transport's cancellation was emitted under the old epoch. It cannot
    // cancel the durable paused task when the resident process starts ticking.
    assert_eq!(state.store.task(&active.id)?.unwrap(), paused);
    assert_eq!(state.store.task(&blocked.id)?.unwrap(), blocked);
    assert!(state.current_download.is_none());

    let stale = download_task("resident-cas-cancelled", &owner, DownloadStatus::Running);
    state.store.save_task(&stale)?;
    state.current_download = Some(stale.clone());
    state.downloads.push_back(stale.clone());
    let mut cancelled = stale.clone();
    cancelled.status = DownloadStatus::Cancelled;
    state.store.save_task(&cancelled)?;
    state.prepare_resident_sleep()?;
    assert_eq!(state.store.task(&cancelled.id)?.unwrap(), cancelled);
    assert!(state.downloads.iter().all(|task| task.id != stale.id));
    state.resume_resident_sleep()?;
    assert_eq!(
        state.store.task(&cancelled.id)?.unwrap().status,
        DownloadStatus::Cancelled
    );
    state.force_offline = false;
    state.online = true;
    state.prepare_resident_sleep()?;
    state.resume_resident_sleep()?;
    let resumed = state.current_download.as_ref().unwrap();
    assert_eq!(resumed.id, active.id);
    assert_eq!(resumed.status, DownloadStatus::Running);
    assert_eq!(resumed.next_index, active.next_index);
    assert_eq!(resumed.chapters, active.chapters);
    assert_eq!(state.jobs.active_download(), Some(active.id.clone()));
    assert_eq!(state.store.task(&active.id)?.unwrap(), *resumed);
    assert_eq!(
        state.store.task(&cancelled.id)?.unwrap().status,
        DownloadStatus::Cancelled
    );
    Ok(())
}

#[test]
fn resident_sleep_pauses_the_exact_download_and_retains_user_intents_without_reviving_cancellation()
-> Result<()> {
    on_lifecycle_thread(run_download_preservation)
}

fn run_blocked_downloads(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Arc<LifecycleDisplay>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    let owner = seed_downloads(&mut state)?;
    let mut blocked = download_task(
        "resident-permission-blocked",
        &owner,
        DownloadStatus::Paused,
    );
    blocked.error = Some("source.content_unverified".into());
    let cancelled = download_task(
        "resident-permanent-cancel",
        &owner,
        DownloadStatus::Cancelled,
    );
    state.store.save_task(&blocked)?;
    state.store.save_task(&cancelled)?;
    state.downloads = VecDeque::from([blocked.clone(), cancelled.clone()]);
    state.prepare_resident_sleep()?;
    state.force_offline = false;
    state.online = true;
    state.resume_resident_sleep()?;
    assert!(state.current_download.is_none());
    assert!(state.jobs.active_download().is_none());
    assert!(state.downloads.is_empty());
    assert_eq!(state.store.task(&blocked.id)?.unwrap(), blocked);
    assert_eq!(state.store.task(&cancelled.id)?.unwrap(), cancelled);
    Ok(())
}

#[test]
fn resident_resume_does_not_automatically_retry_permission_blocks_or_cancelled_downloads()
-> Result<()> {
    on_lifecycle_thread(run_blocked_downloads)
}

fn run_stale_result(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Arc<LifecycleDisplay>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    let epoch = state.jobs.account_epoch();
    let original_route = state.route.clone();
    let original_page = state.reader.paginator.as_ref().unwrap().current_index();
    state.prepare_resident_sleep()?;
    let original_book = state.store.book("resident-reader")?.unwrap();
    // Match the newest request ID deliberately: only the old epoch must stop
    // this otherwise valid destructive shelf replacement from being accepted.
    let request = state.shelf_request;
    assert!(!state.handle_job(JobEvent {
        credits: None,
        request,
        account_epoch: epoch,
        result: JobResult::Shelf(Ok(source::ShelfSnapshot::default())),
    })?);
    state.resume_resident_sleep()?;
    let shelf_request = state.shelf_request;
    assert!(!state.handle_job(JobEvent {
        credits: None,
        request: shelf_request,
        account_epoch: epoch,
        result: JobResult::Shelf(Ok(source::ShelfSnapshot::default())),
    })?);
    assert_eq!(state.store.book("resident-reader")?, Some(original_book));
    assert_eq!(state.route, original_route);
    assert_eq!(
        state.reader.paginator.as_ref().unwrap().current_index(),
        original_page
    );
    Ok(())
}

#[test]
fn queued_presleep_results_cannot_change_storage_or_the_resumed_reader() -> Result<()> {
    on_lifecycle_thread(run_stale_result)
}

fn run_storage_failures(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Arc<LifecycleDisplay>,
    directory: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    state.force_offline = false;
    state.online = false;
    let connection = rusqlite::Connection::open(directory.join("library.sqlite"))?;
    for target in ["preferences", "position"] {
        let ddl = if target == "preferences" {
            "CREATE TRIGGER resident_reject_insert BEFORE INSERT ON settings
             WHEN NEW.key='preferences' BEGIN SELECT RAISE(ABORT,'fixture preferences checkpoint failure'); END;
             CREATE TRIGGER resident_reject_update BEFORE UPDATE OF value ON settings
             WHEN NEW.key='preferences' BEGIN SELECT RAISE(ABORT,'fixture preferences checkpoint failure'); END;"
        } else {
            "CREATE TRIGGER resident_reject_insert BEFORE INSERT ON positions
             WHEN NEW.book_id='resident-reader' BEGIN SELECT RAISE(ABORT,'fixture position checkpoint failure'); END;
             CREATE TRIGGER resident_reject_update BEFORE UPDATE ON positions
             WHEN NEW.book_id='resident-reader' BEGIN SELECT RAISE(ABORT,'fixture position checkpoint failure'); END;"
        };
        connection.execute_batch(ddl)?;
        let error = state
            .prepare_resident_sleep()
            .expect_err("A failed checkpoint cannot acknowledge resident sleep readiness");
        assert!(format!("{error:#}").contains(&format!("fixture {target} checkpoint failure")));
        assert!(state.managed_sleep_blocked);
        assert!(!state.resident_lifecycle.is_prepared());
        assert!(
            !state.force_offline,
            "The temporary no-upload flag must always be restored"
        );
        assert!(!state.online);
        assert!(state.resume_resident_sleep().is_err());
        assert!(state.managed_sleep_blocked);
        connection.execute_batch(
            "DROP TRIGGER resident_reject_insert; DROP TRIGGER resident_reject_update;",
        )?;
        state.prepare_resident_sleep()?;
        state.resume_resident_sleep()?;
        assert!(!state.managed_sleep_blocked);
    }
    Ok(())
}

#[test]
fn failed_resident_checkpoints_keep_input_blocked_and_restore_the_offline_flag() -> Result<()> {
    on_lifecycle_thread(run_storage_failures)
}

fn run_renderer_gate(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    display: &Arc<LifecycleDisplay>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    let address = paginator_address(&state);
    state.prepare_resident_sleep()?;
    for phase in [
        ManagedSleepState::Suspending,
        ManagedSleepState::Suspended,
        ManagedSleepState::Resuming,
        ManagedSleepState::Terminated,
    ] {
        display.set_state(phase);
        assert!(state.resume_resident_sleep().is_err());
        assert!(state.managed_sleep_blocked);
        assert!(state.resident_lifecycle.is_prepared());
        assert!(state.reader.session_started.is_none());
        assert_eq!(paginator_address(&state), address);
    }
    display.set_state(ManagedSleepState::Running);
    state.resume_resident_sleep()?;
    assert!(!state.managed_sleep_blocked);
    state.prepare_resident_sleep()?;
    state.closing.set(true);
    assert!(state.resume_resident_sleep().is_err());
    assert!(state.managed_sleep_blocked);
    assert!(state.reader.session_started.is_none());
    state.closing.set(false);
    state.resume_resident_sleep()?;
    Ok(())
}

#[test]
fn resident_resume_requires_a_restored_renderer_and_cannot_reopen_a_closing_application()
-> Result<()> {
    on_lifecycle_thread(run_renderer_gate)
}

fn run_report_queue_failure(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Arc<LifecycleDisplay>,
    directory: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    let connection = rusqlite::Connection::open(directory.join("library.sqlite"))?;
    connection.execute_batch(
        "CREATE TRIGGER resident_reject_report_insert BEFORE INSERT ON settings
         WHEN NEW.key='report.outbox.v1'
         BEGIN SELECT RAISE(ABORT,'fixture report queue checkpoint failure'); END;
         CREATE TRIGGER resident_reject_report_update BEFORE UPDATE OF value ON settings
         WHEN NEW.key='report.outbox.v1'
         BEGIN SELECT RAISE(ABORT,'fixture report queue checkpoint failure'); END;",
    )?;
    let error = state.prepare_resident_sleep().expect_err(
        "A report-queue storage failure must prevent a resident readiness acknowledgement",
    );
    assert!(format!("{error:#}").contains("fixture report queue checkpoint failure"));
    assert!(state.managed_sleep_blocked);
    assert!(!state.resident_lifecycle.is_prepared());
    assert_eq!(
        state.store.setting::<bool>("report.storage_error.v1")?,
        Some(true)
    );
    assert!(state.resume_resident_sleep().is_err());
    connection.execute_batch(
        "DROP TRIGGER resident_reject_report_insert;
         DROP TRIGGER resident_reject_report_update;",
    )?;
    state.shutdown()?;
    assert!(!state.resident_lifecycle.is_prepared());
    assert!(state.reader.session_started.is_none());
    Ok(())
}

#[test]
fn a_report_queue_checkpoint_failure_cannot_ack_resident_sleep_but_allows_fallback_shutdown()
-> Result<()> {
    on_lifecycle_thread(run_report_queue_failure)
}

fn run_earlier_report_finish_failure(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Arc<LifecycleDisplay>,
    directory: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    let mut book = state.store.book("resident-reader")?.unwrap();
    book.source = "weread".into();
    state.store.upsert_book(&book)?;
    let mut chapter = state.store.chapter(&book.id, 0)?.unwrap();
    chapter.content_account_key = state
        .jobs
        .session()
        .as_ref()
        .and_then(library::download_account_key)
        .unwrap();
    state.store.upsert_chapter(&chapter)?;
    state.store.set_setting(
        "catalog.remote:resident-reader",
        &vec![source::RemoteChapter {
            uid: chapter.id.clone(),
            title: chapter.title.clone(),
            index: 1,
            word_count: 6000,
            ..source::RemoteChapter::default()
        }],
    )?;
    state.prefs.report_time = true;
    state.start_read_report_session()?;
    assert!(
        state
            .store
            .setting::<serde_json::Value>("report.active.v1")?
            .is_some()
    );
    let connection = rusqlite::Connection::open(directory.join("library.sqlite"))?;
    connection.execute_batch(
        "CREATE TRIGGER resident_reject_active_report BEFORE UPDATE OF value ON settings
         WHEN NEW.key='report.active.v1'
         BEGIN SELECT RAISE(ABORT,'fixture active report finish failure'); END;",
    )?;
    let error = state.prepare_resident_sleep().expect_err(
        "A finish-reading failure must not be hidden by a later successful interrupt checkpoint",
    );
    assert!(format!("{error:#}").contains("reading report persistence is unavailable"));
    assert!(
        state.reader.session_started.is_none(),
        "The ordinary finish path caught the first error"
    );
    assert!(
        state
            .store
            .setting::<serde_json::Value>("report.outbox.v1")?
            .is_some()
    );
    assert_eq!(
        state.store.setting::<bool>("report.storage_error.v1")?,
        Some(true)
    );
    assert!(state.managed_sleep_blocked);
    assert!(!state.resident_lifecycle.is_prepared());
    assert!(state.resume_resident_sleep().is_err());
    connection.execute_batch("DROP TRIGGER resident_reject_active_report;")?;
    state.shutdown()?;
    assert!(!state.resident_lifecycle.is_prepared());
    Ok(())
}

#[test]
fn resident_readiness_rejects_an_earlier_report_finish_error_even_when_interrupt_writes_succeed()
-> Result<()> {
    on_lifecycle_thread(run_earlier_report_finish_failure)
}

#[derive(Default)]
struct WakeResetGateState {
    entered: usize,
    released: bool,
}

#[derive(Default)]
struct WakeResetGate {
    state: Mutex<WakeResetGateState>,
    changed: Condvar,
}

impl WakeResetGate {
    fn block(&self) -> std::result::Result<(), PlatformError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.entered += 1;
        self.changed.notify_all();
        let (state, timeout) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(10), |state| !state.released)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if timeout.timed_out() && !state.released {
            return Err(PlatformError::Operation(
                "the wake-reset fixture was not released".into(),
            ));
        }
        Ok(())
    }

    fn wait_for_calls(&self, count: usize) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (state, timeout) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(3), |state| state.entered < count)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let ready = !timeout.timed_out() && state.entered >= count;
        drop(state);
        assert!(
            ready,
            "The expected wake-maintenance operation did not start"
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

/// A failed assertion must release any device call before App teardown cancels it.
struct WakeResetRelease(Arc<WakeResetGate>);

impl Drop for WakeResetRelease {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct BlockingWakeControl {
    gate: Arc<WakeResetGate>,
    calls: AtomicUsize,
    active: AtomicUsize,
    maximum_active: AtomicUsize,
}

impl BlockingWakeControl {
    fn new(gate: Arc<WakeResetGate>) -> Self {
        Self {
            gate,
            calls: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            maximum_active: AtomicUsize::new(0),
        }
    }
}

impl DeviceControl for BlockingWakeControl {
    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities {
            keep_awake: true,
            ..PlatformCapabilities::default()
        }
    }

    fn brightness(&self) -> std::result::Result<u8, PlatformError> {
        Err(PlatformError::Unsupported("brightness"))
    }

    fn set_brightness(&self, _: u8) -> std::result::Result<(), PlatformError> {
        Err(PlatformError::Unsupported("brightness"))
    }

    fn warmth(&self) -> std::result::Result<u8, PlatformError> {
        Err(PlatformError::Unsupported("warmth"))
    }

    fn set_warmth(&self, _: u8) -> std::result::Result<(), PlatformError> {
        Err(PlatformError::Unsupported("warmth"))
    }

    fn battery_percentage(&self) -> std::result::Result<u8, PlatformError> {
        Ok(80)
    }

    fn set_inverted(&self, _: bool) -> std::result::Result<(), PlatformError> {
        Err(PlatformError::Unsupported("hardware_inversion"))
    }

    fn reset_idle_timer(&self) -> std::result::Result<(), PlatformError> {
        struct ActiveReset<'a>(&'a AtomicUsize);
        impl Drop for ActiveReset<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }

        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        let _active = ActiveReset(&self.active);
        self.maximum_active.fetch_max(active, Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.gate.block()
    }
}

fn install_blocking_wake_control(
    state: &mut App,
    display: &Arc<LifecycleDisplay>,
    control: Arc<BlockingWakeControl>,
) -> Result<()> {
    let platform = Arc::new(Platform::with_control(
        DeviceProfile::Scribe,
        false,
        control,
    ));
    platform.set_render_sink(display.clone())?;
    state.platform = platform;
    state.prefs.keep_awake = true;
    Ok(())
}

fn cancel_fixture_download(state: &mut App) -> Result<()> {
    state.dialog_action = "downloading".into();
    state.dispatch("shell", "dialog-secondary", "")?;
    assert!(state.current_download.is_none());
    Ok(())
}

fn run_cancelled_wake_guard_blocks_resident_readiness(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    display: &Arc<LifecycleDisplay>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    let owner = seed_downloads(&mut state)?;
    let gate = Arc::new(WakeResetGate::default());
    let _release = WakeResetRelease(gate.clone());
    let control = Arc::new(BlockingWakeControl::new(gate.clone()));
    install_blocking_wake_control(&mut state, display, control.clone())?;
    let task = download_task("resident-cancelled-wake", &owner, DownloadStatus::Running);
    state.store.save_task(&task)?;
    state.current_download = Some(task.clone());
    state.start_task_wake_guard();
    gate.wait_for_calls(1);

    // Exercise the same cancellation entry point that clears the task banner.
    // No download future runs in this current-thread, offline fixture.
    cancel_fixture_download(&mut state)?;
    assert_eq!(
        state.store.task(&task.id)?.unwrap().status,
        DownloadStatus::Cancelled
    );
    let error = state
        .prepare_resident_sleep()
        .expect_err("A cancelled task's in-flight wake helper must prevent resident readiness");
    assert!(format!("{error:#}").contains("wake"));
    assert!(state.managed_sleep_blocked);
    assert!(!state.resident_lifecycle.is_prepared());
    assert!(state.join_task_wake_guard(Duration::ZERO).is_err());
    assert!(state.resume_resident_sleep().is_err());
    assert_eq!(control.active.load(Ordering::SeqCst), 1);
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);

    gate.release();
    state.prepare_resident_sleep()?;
    assert!(state.resident_lifecycle.is_prepared());
    assert_eq!(control.active.load(Ordering::SeqCst), 0);
    state.join_task_wake_guard(Duration::ZERO)?;
    state.resume_resident_sleep()?;
    assert!(!state.managed_sleep_blocked);
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
    assert_eq!(control.maximum_active.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn cancelled_wake_maintenance_cannot_ack_resident_sleep_until_its_old_helper_is_joined()
-> Result<()> {
    on_lifecycle_thread(run_cancelled_wake_guard_blocks_resident_readiness)
}

fn run_task_replacement_keeps_one_wake_worker(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    display: &Arc<LifecycleDisplay>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    let owner = seed_downloads(&mut state)?;
    let gate = Arc::new(WakeResetGate::default());
    let _release = WakeResetRelease(gate.clone());
    let control = Arc::new(BlockingWakeControl::new(gate.clone()));
    install_blocking_wake_control(&mut state, display, control.clone())?;
    let first = download_task("resident-wake-first", &owner, DownloadStatus::Running);
    state.store.save_task(&first)?;
    state.current_download = Some(first);
    state.start_task_wake_guard();
    gate.wait_for_calls(1);
    cancel_fixture_download(&mut state)?;

    let replacement = download_task("resident-wake-replacement", &owner, DownloadStatus::Running);
    state.store.save_task(&replacement)?;
    state.current_download = Some(replacement);
    for _ in 0..256 {
        state.start_task_wake_guard();
        state.poll_task_wake_guard();
    }
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
    assert_eq!(control.active.load(Ordering::SeqCst), 1);
    assert_eq!(control.maximum_active.load(Ordering::SeqCst), 1);
    cancel_fixture_download(&mut state)?;
    gate.release();
    state.join_task_wake_guard(Duration::from_secs(2))?;
    assert_eq!(control.active.load(Ordering::SeqCst), 0);

    let fresh = download_task("resident-wake-fresh", &owner, DownloadStatus::Running);
    state.store.save_task(&fresh)?;
    state.current_download = Some(fresh);
    state.start_task_wake_guard();
    gate.wait_for_calls(2);
    cancel_fixture_download(&mut state)?;
    state.join_task_wake_guard(Duration::from_secs(2))?;
    assert_eq!(control.active.load(Ordering::SeqCst), 0);
    assert_eq!(control.calls.load(Ordering::SeqCst), 2);
    assert_eq!(control.maximum_active.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn replacing_cancelled_tasks_cannot_spawn_more_wake_helpers_before_the_old_worker_is_joined()
-> Result<()> {
    on_lifecycle_thread(run_task_replacement_keeps_one_wake_worker)
}

fn run_keep_awake_toggle_waits_for_the_previous_worker(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    display: &Arc<LifecycleDisplay>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    let owner = seed_downloads(&mut state)?;
    let gate = Arc::new(WakeResetGate::default());
    let _release = WakeResetRelease(gate.clone());
    let control = Arc::new(BlockingWakeControl::new(gate.clone()));
    install_blocking_wake_control(&mut state, display, control.clone())?;
    let task = download_task("resident-wake-toggle", &owner, DownloadStatus::Running);
    state.store.save_task(&task)?;
    state.current_download = Some(task);
    state.start_task_wake_guard();
    gate.wait_for_calls(1);

    state.dispatch("shell", "tab", "settings")?;
    state.dispatch("auxiliary", "settings.open", "device")?;
    state.dispatch("auxiliary", "settings.toggle", "keep_awake")?;
    assert!(!state.prefs.keep_awake);
    assert!(state.join_task_wake_guard(Duration::ZERO).is_err());
    state.dispatch("auxiliary", "settings.toggle", "keep_awake")?;
    assert!(state.prefs.keep_awake);
    for _ in 0..256 {
        state.poll_task_wake_guard();
    }
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
    assert_eq!(control.active.load(Ordering::SeqCst), 1);

    gate.release();
    let deadline = Instant::now() + Duration::from_secs(3);
    while control.calls.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        state.poll_task_wake_guard();
        std::thread::sleep(Duration::from_millis(5));
    }
    gate.wait_for_calls(2);
    assert_eq!(control.calls.load(Ordering::SeqCst), 2);
    assert_eq!(control.maximum_active.load(Ordering::SeqCst), 1);
    cancel_fixture_download(&mut state)?;
    state.join_task_wake_guard(Duration::from_secs(2))?;
    assert_eq!(control.active.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn toggling_keep_awake_off_then_on_restarts_only_after_the_previous_helper_is_joined() -> Result<()>
{
    on_lifecycle_thread(run_keep_awake_toggle_waits_for_the_previous_worker)
}

type RetryLifecycleScenario = fn(
    &AppWindow,
    &Rc<RefCell<App>>,
    &Arc<LifecycleDisplay>,
    &Path,
    &tokio::runtime::Runtime,
) -> Result<()>;

fn isolated_retry_lifecycle_case(case: &str, scenario: RetryLifecycleScenario) -> Result<()> {
    isolated_lifecycle_resource_case(case, scenario, false)
}

fn isolated_lifecycle_resource_case(
    case: &str,
    scenario: RetryLifecycleScenario,
    accept_quit: bool,
) -> Result<()> {
    const CHILD: &str = "WEREAD_RESIDENT_RETRY_CHILD_CASE";
    const COMPLETED: &str = "WEREAD_RESIDENT_RETRY_COMPLETED";
    if let Some(selected) = std::env::var_os(CHILD) {
        anyhow::ensure!(
            selected.as_os_str() == std::ffi::OsStr::new(case),
            "The resident retry child selected a different case"
        );
        on_lifecycle_thread_with_runtime_mode(scenario, accept_quit)?;
        println!("\n{COMPLETED}:{case}");
        return Ok(());
    }
    // Each case owns a fresh process and UI thread because Slint's event-loop
    // proxy has process-global state even when the test harness is serialized.
    let output = std::process::Command::new(std::env::current_exe()?)
        .args(["--exact", case, "--nocapture", "--test-threads=1"])
        .env(CHILD, case)
        .env_remove("WEREAD_API_KEY")
        .env_remove("WEREAD_LAUNCH_GENERATION")
        .env_remove("WEREAD_LAUNCH_CONTEXT_DIR")
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
        "The isolated resident retry case did not complete: {case}\nstatus: {}\n{stdout}\n{stderr}",
        output.status,
    );
    Ok(())
}

fn seed_retry_book(state: &mut App, id: &str) -> Result<()> {
    let mut book = Book::new(id, "Resident retry fixture");
    book.source = "weread".into();
    state.store.upsert_book(&book)
}

fn configure_offline_retry_fixture(state: &mut App) -> Result<()> {
    state.jobs.replace_session(fixture_session())?;
    state.force_offline = false;
    state.online = false;
    state.prefs.keep_awake = false;
    assert!(state.jobs.configured());
    assert_eq!(state.jobs.metadata_capacity(), 8);
    Ok(())
}

fn settle_fenced_resident_metadata(
    state: &App,
    runtime: &tokio::runtime::Runtime,
    previous_epoch: u64,
) -> Result<()> {
    anyhow::ensure!(
        state.managed_sleep_blocked && state.resident_lifecycle.is_prepared(),
        "The fixture must prepare resident sleep before polling queued work"
    );
    anyhow::ensure!(
        state.jobs.account_epoch() == previous_epoch + 1,
        "The fixture must cancel the queued request epoch before its first poll"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    runtime.block_on(async {
        while state.jobs.metadata_capacity() != 8 {
            anyhow::ensure!(
                Instant::now() < deadline,
                "Cancelled resident metadata did not release its admission budget"
            );
            // Jobs checks the old account epoch before constructing the HTTP
            // operation. Only already-fenced futures are ever polled here.
            tokio::task::yield_now().await;
        }
        Ok(())
    })
}

fn retry_snapshot(state: &App) -> (BTreeSet<String>, BTreeSet<String>, bool, bool) {
    let (catalogs, books, shelf, search) = state.resident_lifecycle.retry_intents();
    (catalogs.clone(), books.clone(), shelf, search)
}

fn run_offline_detail_retry_cycles(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    display: &Arc<LifecycleDisplay>,
    _: &Path,
    runtime: &tokio::runtime::Runtime,
) -> Result<()> {
    let mut state = application.borrow_mut();
    configure_offline_retry_fixture(&mut state)?;
    let owner = seed_downloads(&mut state)?;
    let queued = download_task("retry-queued-download", &owner, DownloadStatus::Pending);
    let paused = download_task("retry-paused-download", &owner, DownloadStatus::Paused);
    state.store.save_task(&queued)?;
    state.store.save_task(&paused)?;
    let download_queue = VecDeque::from([queued.clone(), paused.clone()]);
    state.downloads = download_queue.clone();
    let explicit = BTreeSet::from([
        "retry-explicit-first".to_owned(),
        "retry-explicit-second".to_owned(),
    ]);
    for id in &explicit {
        seed_retry_book(&mut state, id)?;
    }
    state.pending_catalog_downloads = explicit.clone();
    state.pending_catalog_images = BTreeMap::from([
        ("retry-explicit-first".into(), true),
        ("retry-explicit-second".into(), false),
    ]);
    let image_options = state.pending_catalog_images.clone();
    state.pending_downloads = explicit.iter().map(|id| (id.clone(), vec![0, 1])).collect();
    let pending_downloads = state.pending_downloads.clone();
    for cycle in 0..1_024 {
        let id = format!("offline-resident-detail-{cycle}");
        seed_retry_book(&mut state, &id)?;
        state.navigate(Route::Detail(id.clone()), true)?;
        assert!(state.catalog_requests.contains_key(&id));
        assert!(state.book_requests.contains_key(&id));
        assert_eq!(state.jobs.metadata_capacity(), 6);
        let epoch = state.jobs.account_epoch();
        state.prepare_resident_sleep()?;
        let mut expected_catalogs = explicit.clone();
        expected_catalogs.insert(id.clone());
        assert_eq!(
            retry_snapshot(&state),
            (
                expected_catalogs.clone(),
                BTreeSet::from([id.clone()]),
                false,
                false
            )
        );
        settle_fenced_resident_metadata(&state, runtime, epoch)?;
        assert!(state.events.try_recv().is_err());
        display.set_state(ManagedSleepState::Suspended);
        display.set_state(ManagedSleepState::Running);
        state.resume_resident_sleep()?;
        assert!(!state.managed_sleep_blocked);
        assert!(!state.resident_lifecycle.is_prepared());
        assert_eq!(
            retry_snapshot(&state),
            (expected_catalogs, BTreeSet::from([id]), false, false)
        );
        assert_eq!(state.pending_catalog_downloads, explicit);
        assert_eq!(state.pending_catalog_images, image_options);
        assert_eq!(state.pending_downloads, pending_downloads);
        assert_eq!(state.downloads, download_queue);
        assert!(state.current_download.is_none());
        assert_eq!(state.jobs.metadata_capacity(), 8);
    }
    assert_eq!(state.history.len(), 64);
    assert_eq!(state.store.task(&queued.id)?.unwrap(), queued);
    assert_eq!(state.store.task(&paused.id)?.unwrap(), paused);
    Ok(())
}

#[test]
fn offline_resident_detail_cycles_bound_retry_snapshots_and_preserve_download_intents() -> Result<()>
{
    const CASE: &str = "app::managed_lifecycle_tests::offline_resident_detail_cycles_bound_retry_snapshots_and_preserve_download_intents";
    isolated_retry_lifecycle_case(CASE, run_offline_detail_retry_cycles)
}

fn run_reconnected_retry_scope(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Arc<LifecycleDisplay>,
    _: &Path,
    runtime: &tokio::runtime::Runtime,
) -> Result<()> {
    let mut state = application.borrow_mut();
    configure_offline_retry_fixture(&mut state)?;
    for id in [
        "retry-old-detail",
        "retry-current-detail",
        "retry-explicit-catalog",
    ] {
        seed_retry_book(&mut state, id)?;
    }
    state.navigate(Route::Detail("retry-old-detail".into()), false)?;
    let epoch = state.jobs.account_epoch();
    state.prepare_resident_sleep()?;
    settle_fenced_resident_metadata(&state, runtime, epoch)?;
    state.resume_resident_sleep()?;
    assert!(retry_snapshot(&state).0.contains("retry-old-detail"));

    state.navigate(Route::Detail("retry-current-detail".into()), false)?;
    state
        .pending_catalog_downloads
        .insert("retry-explicit-catalog".into());
    state
        .pending_catalog_images
        .insert("retry-explicit-catalog".into(), true);
    let expected = BTreeSet::from([
        "retry-current-detail".to_owned(),
        "retry-explicit-catalog".to_owned(),
    ]);
    let epoch = state.jobs.account_epoch();
    state.prepare_resident_sleep()?;
    settle_fenced_resident_metadata(&state, runtime, epoch)?;
    assert_eq!(retry_snapshot(&state).0, expected);
    state.online = true;
    state.resume_resident_sleep()?;
    assert_eq!(
        state
            .catalog_requests
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected
    );
    assert_eq!(
        state.book_requests.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from(["retry-current-detail".to_owned()])
    );
    assert_eq!(state.jobs.metadata_capacity(), 5);
    assert_eq!(
        retry_snapshot(&state),
        (BTreeSet::new(), BTreeSet::new(), false, false)
    );
    assert!(
        state
            .pending_catalog_downloads
            .contains("retry-explicit-catalog")
    );
    assert_eq!(
        state.pending_catalog_images.get("retry-explicit-catalog"),
        Some(&true)
    );

    // Online resume admits new work. Fence that new epoch before driving the
    // runtime again, so this assertion never sends a real metadata request.
    state.online = false;
    let epoch = state.jobs.account_epoch();
    state.prepare_resident_sleep()?;
    settle_fenced_resident_metadata(&state, runtime, epoch)?;
    state.resume_resident_sleep()?;
    assert_eq!(retry_snapshot(&state).0, expected);
    assert!(state.events.try_recv().is_err());
    Ok(())
}

#[test]
fn reconnecting_resident_reads_retry_only_the_visible_detail_and_explicit_catalogs() -> Result<()> {
    const CASE: &str = "app::managed_lifecycle_tests::reconnecting_resident_reads_retry_only_the_visible_detail_and_explicit_catalogs";
    isolated_retry_lifecycle_case(CASE, run_reconnected_retry_scope)
}

fn run_unconfigured_empty_reader_scope(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Arc<LifecycleDisplay>,
    _: &Path,
    runtime: &tokio::runtime::Runtime,
) -> Result<()> {
    let mut state = application.borrow_mut();
    configure_offline_retry_fixture(&mut state)?;
    for id in [
        "retry-stale-detail",
        "retry-empty-reader",
        "retry-reader-explicit",
    ] {
        seed_retry_book(&mut state, id)?;
    }
    state.navigate(Route::Detail("retry-stale-detail".into()), false)?;
    let epoch = state.jobs.account_epoch();
    state.prepare_resident_sleep()?;
    settle_fenced_resident_metadata(&state, runtime, epoch)?;
    state.resume_resident_sleep()?;

    state.jobs.clear_session();
    state.online = true;
    state.route = Route::Reader("retry-empty-reader".into(), 0);
    state.reader.book_id = "retry-empty-reader".into();
    state.reader.chapter_index = 0;
    state.reader.body.clear();
    state.reader.paginator = None;
    state.pending_read = Some(("retry-empty-reader".into(), 0));
    state
        .pending_catalog_downloads
        .insert("retry-reader-explicit".into());
    state.request_catalog("retry-empty-reader");
    let expected = BTreeSet::from([
        "retry-empty-reader".to_owned(),
        "retry-reader-explicit".to_owned(),
    ]);
    let epoch = state.jobs.account_epoch();
    state.prepare_resident_sleep()?;
    settle_fenced_resident_metadata(&state, runtime, epoch)?;
    assert!(!state.jobs.configured());
    state.resume_resident_sleep()?;
    assert_eq!(
        retry_snapshot(&state),
        (expected, BTreeSet::new(), false, false)
    );
    assert_eq!(state.pending_read, Some(("retry-empty-reader".into(), 0)));
    assert!(state.catalog_requests.is_empty());

    state.jobs.replace_session(fixture_session())?;
    state.online = false;
    let epoch = state.jobs.account_epoch();
    state.prepare_resident_sleep()?;
    settle_fenced_resident_metadata(&state, runtime, epoch)?;
    // Exercise the resume-time projection when a restored context changes
    // while input remains blocked; forced offline must still prune stale UI.
    state.route = Route::Library;
    state.pending_read = None;
    state.force_offline = true;
    state.resume_resident_sleep()?;
    assert_eq!(
        retry_snapshot(&state),
        (
            BTreeSet::from(["retry-reader-explicit".to_owned()]),
            BTreeSet::new(),
            false,
            false
        )
    );
    Ok(())
}

#[test]
fn unconfigured_and_forced_offline_resumes_keep_only_current_empty_reader_and_download_intent()
-> Result<()> {
    const CASE: &str = "app::managed_lifecycle_tests::unconfigured_and_forced_offline_resumes_keep_only_current_empty_reader_and_download_intent";
    isolated_retry_lifecycle_case(CASE, run_unconfigured_empty_reader_scope)
}

fn run_account_retry_invalidation(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Arc<LifecycleDisplay>,
    _: &Path,
    runtime: &tokio::runtime::Runtime,
) -> Result<()> {
    let mut state = application.borrow_mut();
    configure_offline_retry_fixture(&mut state)?;
    seed_retry_book(&mut state, "retry-account-detail")?;
    for route in [
        Route::Detail("retry-account-detail".into()),
        Route::Library,
        Route::Search,
    ] {
        state.navigate(route.clone(), false)?;
        state
            .pending_catalog_downloads
            .insert("retry-old-account-download".into());
        state.pending_downloads = vec![("retry-old-account-download".into(), vec![0])];
        match route {
            Route::Library => state.sync_shelf(),
            Route::Search => {
                state.search_purpose = "books".into();
                state.search_scope = "store".into();
                state.query = PinyinInput::new("Synthetic retry query");
                state.search_status = tr("searching");
            }
            _ => {}
        }
        let epoch = state.jobs.account_epoch();
        state.prepare_resident_sleep()?;
        settle_fenced_resident_metadata(&state, runtime, epoch)?;
        let before = retry_snapshot(&state);
        assert!(before.0.contains("retry-old-account-download"));
        match route {
            Route::Detail(_) => assert!(before.1.contains("retry-account-detail")),
            Route::Library => assert!(before.2),
            Route::Search => assert!(before.3),
            _ => unreachable!(),
        }
        state.invalidate_account_work()?;
        assert_eq!(
            retry_snapshot(&state),
            (BTreeSet::new(), BTreeSet::new(), false, false)
        );
        assert!(state.resident_lifecycle.is_prepared());
        assert!(state.managed_sleep_blocked);
        // Existing account replacement intentionally clears old-account plans.
        assert!(state.pending_catalog_downloads.is_empty());
        assert!(state.pending_downloads.is_empty());
        state.resume_resident_sleep()?;
        assert!(!state.resident_lifecycle.is_prepared());
        assert!(!state.managed_sleep_blocked);
    }
    Ok(())
}

#[test]
fn account_invalidation_clears_retry_ownership_without_resetting_the_prepared_input_guard()
-> Result<()> {
    const CASE: &str = "app::managed_lifecycle_tests::account_invalidation_clears_retry_ownership_without_resetting_the_prepared_input_guard";
    isolated_retry_lifecycle_case(CASE, run_account_retry_invalidation)
}

struct RetiredNativeControl {
    blocked: AtomicBool,
    checks: AtomicUsize,
    nonzero_timeouts: AtomicUsize,
}

impl RetiredNativeControl {
    fn new(blocked: bool) -> Self {
        Self {
            blocked: AtomicBool::new(blocked),
            checks: AtomicUsize::new(0),
            nonzero_timeouts: AtomicUsize::new(0),
        }
    }
}

impl DeviceControl for RetiredNativeControl {
    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::default()
    }

    fn native_output_inverted(&self) -> Option<bool> {
        Some(false)
    }

    fn brightness(&self) -> std::result::Result<u8, PlatformError> {
        Err(PlatformError::Unsupported("fixture_brightness"))
    }

    fn set_brightness(&self, _: u8) -> std::result::Result<(), PlatformError> {
        Err(PlatformError::Unsupported("fixture_brightness"))
    }

    fn warmth(&self) -> std::result::Result<u8, PlatformError> {
        Err(PlatformError::Unsupported("fixture_warmth"))
    }

    fn set_warmth(&self, _: u8) -> std::result::Result<(), PlatformError> {
        Err(PlatformError::Unsupported("fixture_warmth"))
    }

    fn battery_percentage(&self) -> std::result::Result<u8, PlatformError> {
        Ok(80)
    }

    fn set_inverted(&self, _: bool) -> std::result::Result<(), PlatformError> {
        Err(PlatformError::Unsupported("fixture_inversion"))
    }

    fn reset_idle_timer(&self) -> std::result::Result<(), PlatformError> {
        Err(PlatformError::Unsupported("fixture_keep_awake"))
    }

    fn quiesce_commands(&self, timeout: Duration) -> std::result::Result<(), PlatformError> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        if !timeout.is_zero() {
            self.nonzero_timeouts.fetch_add(1, Ordering::SeqCst);
        }
        if self.blocked.load(Ordering::SeqCst) {
            Err(PlatformError::Operation(
                "synthetic native child remains retired".into(),
            ))
        } else {
            Ok(())
        }
    }
}

fn install_retired_native_control(
    state: &mut App,
    display: &Arc<LifecycleDisplay>,
) -> Result<Arc<RetiredNativeControl>> {
    state.join_task_wake_guard(Duration::ZERO)?;
    state.stop_lighting_hardware()?;
    let control = Arc::new(RetiredNativeControl::new(true));
    let platform = Arc::new(Platform::with_control(
        DeviceProfile::Scribe,
        false,
        control.clone(),
    ));
    platform.set_render_sink(display.clone())?;
    state.platform = platform;
    Ok(control)
}

fn run_retired_native_blocks_sleep(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    display: &Arc<LifecycleDisplay>,
    _: &Path,
    _: &tokio::runtime::Runtime,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    let control = install_retired_native_control(&mut state, display)?;
    let page = state
        .reader
        .paginator
        .as_ref()
        .unwrap()
        .current_page()
        .clone();
    let buffers = resident_buffer_addresses(&state);
    for _ in 0..4 {
        let error = state.prepare_resident_sleep().expect_err(
            "Joined application workers cannot acknowledge a still-retired native producer",
        );
        assert!(format!("{error:#}").contains("synthetic native child remains retired"));
        assert!(state.managed_sleep_blocked);
        assert!(!state.resident_lifecycle.is_prepared());
        assert!(state.resume_resident_sleep().is_err());
        assert_eq!(resident_buffer_addresses(&state), buffers);
    }
    assert_eq!(control.checks.load(Ordering::SeqCst), 4);
    control.blocked.store(false, Ordering::SeqCst);
    state.prepare_resident_sleep()?;
    assert!(state.resident_lifecycle.is_prepared());
    assert!(state.managed_sleep_blocked);
    assert_eq!(control.checks.load(Ordering::SeqCst), 5);
    assert_eq!(control.nonzero_timeouts.load(Ordering::SeqCst), 0);
    state.resume_resident_sleep()?;
    assert!(!state.managed_sleep_blocked);
    assert!(!state.resident_lifecycle.is_prepared());
    assert_eq!(
        state.reader.paginator.as_ref().unwrap().current_page(),
        &page
    );
    assert_eq!(resident_buffer_addresses(&state), buffers);
    Ok(())
}

#[test]
fn resident_sleep_requires_native_producer_quiescence_after_application_workers_join() -> Result<()>
{
    const CASE: &str = "app::managed_lifecycle_tests::resident_sleep_requires_native_producer_quiescence_after_application_workers_join";
    isolated_lifecycle_resource_case(CASE, run_retired_native_blocks_sleep, false)
}

fn run_default_native_quiescence(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Arc<LifecycleDisplay>,
    _: &Path,
    _: &tokio::runtime::Runtime,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    // The existing desktop fixture inherits DeviceControl's default no-op.
    // It must remain usable without constructing any native command runner.
    state.platform.quiesce_device_commands(Duration::ZERO)?;
    state.prepare_resident_sleep()?;
    assert!(state.resident_lifecycle.is_prepared());
    state.resume_resident_sleep()?;
    assert!(!state.managed_sleep_blocked);
    state.shutdown()?;
    assert!(!state.closing.get());
    Ok(())
}

#[test]
fn a_default_control_without_native_producers_completes_resident_and_shutdown_handoffs()
-> Result<()> {
    const CASE: &str = "app::managed_lifecycle_tests::a_default_control_without_native_producers_completes_resident_and_shutdown_handoffs";
    isolated_lifecycle_resource_case(CASE, run_default_native_quiescence, false)
}

fn run_retired_native_blocks_ordinary_exit(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    display: &Arc<LifecycleDisplay>,
    _: &Path,
    _: &tokio::runtime::Runtime,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_reader(&mut state)?;
    let control = install_retired_native_control(&mut state, display)?;
    let buffers = resident_buffer_addresses(&state);
    let error = state.shutdown().expect_err(
        "Ordinary shutdown must retain native ownership while a producer remains retired",
    );
    assert!(format!("{error:#}").contains("synthetic native child remains retired"));
    assert!(!state.closing.get());
    assert_eq!(display.quit_attempts.load(Ordering::SeqCst), 0);
    for _ in 0..4 {
        let error = state
            .request_exit()
            .expect_err("An explicit exit cannot pass the native producer quiescence gate");
        assert!(format!("{error:#}").contains("synthetic native child remains retired"));
        assert!(!state.closing.get());
        assert!(!state.resident_lifecycle.is_prepared());
        assert_eq!(display.quit_attempts.load(Ordering::SeqCst), 0);
        assert!(state.reader.session_started.is_some());
        assert_eq!(resident_buffer_addresses(&state), buffers);
    }
    assert_eq!(control.checks.load(Ordering::SeqCst), 5);
    control.blocked.store(false, Ordering::SeqCst);
    state.request_exit()?;
    assert!(state.closing.get());
    assert_eq!(display.quit_attempts.load(Ordering::SeqCst), 1);
    assert_eq!(control.checks.load(Ordering::SeqCst), 6);
    assert_eq!(control.nonzero_timeouts.load(Ordering::SeqCst), 0);
    // This desktop case observes the actual exit and quit boundary. It has no
    // native launcher context and makes no claim about marker-file publication.
    Ok(())
}

#[test]
fn ordinary_exit_waits_for_native_quiescence_before_closing_or_requesting_quit() -> Result<()> {
    const CASE: &str = "app::managed_lifecycle_tests::ordinary_exit_waits_for_native_quiescence_before_closing_or_requesting_quit";
    isolated_lifecycle_resource_case(CASE, run_retired_native_blocks_ordinary_exit, true)
}

#[path = "sleep_lifecycle_tests.rs"]
mod sleep_lifecycle_tests;
