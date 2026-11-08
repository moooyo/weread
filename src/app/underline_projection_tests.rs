//! Genuine offline App renders, isolated from Slint's process-global UI state.

use super::*;
use crate::jobs::JobResult;
use slint::{
    Model,
    platform::{
        Platform as SlintPlatform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::RefMut,
    process::{Child, Command, Stdio},
};

pub(super) const BOOK: &str = "underline-fixture-a";
pub(super) const CHAPTER: &str = "underline-fixture-chapter-0";
const BODY: &str = "A\u{1f600}\u{4e2d}B marker. repeat repeat.";
const MOVED_BODY: &str = "AB\u{1f600}\u{4e2d} marker. repeat repeat.";

struct TestUiPlatform;

impl SlintPlatform for TestUiPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

fn underline(text: &str, count: u32) -> RemoteUnderline {
    RemoteUnderline {
        text: text.into(),
        range: "synthetic-opaque-range".into(),
        count,
        thoughts: 1,
    }
}

fn records() -> Vec<RemoteUnderline> {
    vec![
        underline("\u{1f600}\u{4e2d}", 3),
        underline("\u{1f600}\u{4e2d}", 9),
        underline("repeat", 5),
        underline("zzz", 1),
    ]
}

pub(super) struct Fixture {
    application: Option<Rc<RefCell<App>>>,
    ui: Option<AppWindow>,
    runtime: Option<tokio::runtime::Runtime>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    pub(super) fn new(body: &str, underlines: Vec<RemoteUnderline>, popular: bool) -> Result<Self> {
        Self::new_for_profile(body, underlines, popular, DeviceProfile::Paperwhite6)
    }

    pub(super) fn new_for_profile(
        body: &str,
        underlines: Vec<RemoteUnderline>,
        popular: bool,
        profile: DeviceProfile,
    ) -> Result<Self> {
        slint::platform::set_platform(Box::new(TestUiPlatform))?;
        let directory = tempfile::tempdir()?;
        let platform = Arc::new(Platform::new(profile, false)?);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let ui = AppWindow::new()?;
        crate::platform::configure_window(ui.window(), &platform)?;
        let application = App::attach(
            &ui,
            AppOptions {
                data_dir: directory.path().to_path_buf(),
                profile,
                demo: true,
                offline: true,
                initial_screen: "library".into(),
                fonts: vec![FontRecord {
                    id: "song".into(),
                    label: "Underline fixture font".into(),
                    family: "sans-serif".into(),
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
            let mut book = Book::new(BOOK, "Underline fixture book");
            book.source = "local".into();
            book.total_chapters = 2;
            book.cached_chapters = 2;
            book.on_shelf = true;
            state.store.upsert_book(&book)?;
            save_chapter(&state.store, BOOK, CHAPTER, 0, body)?;
            save_chapter(
                &state.store,
                BOOK,
                "underline-fixture-chapter-1",
                1,
                "Second chapter marker.",
            )?;
            state
                .store
                .set_setting(&format!("underlines:{BOOK}:{CHAPTER}"), &underlines)?;
            state.store.set_setting(
                &format!("underlines:{BOOK}:underline-fixture-chapter-1"),
                &vec![underline("Second", 2)],
            )?;
            state.prefs.popular = popular;
            state.load_local_reader(BOOK, 0, None)?;
            state.render_reader()?;
            anyhow::ensure!(
                state.demo && state.force_offline && !state.jobs.configured(),
                "The underline fixture became connected"
            );
        }
        Ok(Self {
            application: Some(application),
            ui: Some(ui),
            runtime: Some(runtime),
            _directory: directory,
        })
    }

    pub(super) fn app(&self) -> RefMut<'_, App> {
        self.application.as_ref().unwrap().borrow_mut()
    }

    pub(super) fn ui(&self) -> &AppWindow {
        self.ui.as_ref().unwrap()
    }

    pub(super) fn finish(mut self) -> Result<()> {
        let application = self.application.take().unwrap();
        application.borrow_mut().terminal_shutdown()?;
        let weak = Rc::downgrade(&application);
        drop(application);
        anyhow::ensure!(
            weak.upgrade().is_none(),
            "An underline fixture callback retained App"
        );
        let ui = self.ui.take().unwrap();
        let weak = ui.as_weak();
        ui.hide()?;
        drop(ui);
        slint::platform::update_timers_and_animations();
        anyhow::ensure!(
            weak.upgrade().is_none(),
            "An underline fixture retained its window"
        );
        self.runtime
            .take()
            .unwrap()
            .shutdown_timeout(Duration::from_secs(1));
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(application) = self.application.take() {
            let _ = application.borrow_mut().terminal_shutdown();
            drop(application);
        }
        if let Some(ui) = self.ui.take() {
            let _ = ui.hide();
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }
}

fn save_chapter(store: &Store, book: &str, id: &str, index: u32, body: &str) -> Result<()> {
    store.upsert_chapter(&Chapter {
        id: id.into(),
        book_id: book.into(),
        index,
        title: "Underline fixture chapter".into(),
        cached: true,
        body: Some(body.into()),
        content_status: ContentStatus::Full,
        ..Default::default()
    })
}

fn remote_marks(ui: &AppWindow) -> usize {
    ui.global::<ReaderModel>()
        .get_lines()
        .iter()
        .map(|line| {
            line.marks
                .iter()
                .filter(|mark| !mark.mine && !mark.selected)
                .count()
        })
        .sum()
}

fn repeated_renders() -> Result<()> {
    let fixture = Fixture::new(BODY, records(), true)?;
    {
        let mut state = fixture.app();
        let before = state.reader.underline_projection.counters();
        assert_eq!(before.matches, 3);
        assert_eq!(state.reader.underline_projection.range(0), Some((1, 3)));
        assert_eq!(state.reader.underline_projection.range(1), Some((1, 3)));
        assert_eq!(state.reader.underline_projection.range(2), None);
        let body_pointer = state.reader.body.as_ptr();
        for _ in 0..64 {
            state.render_reader()?;
        }
        assert_eq!(state.reader.underline_projection.counters(), before);
        assert_eq!(state.reader.body.as_ptr(), body_pointer);
        assert_eq!(remote_marks(fixture.ui()), 2);
        assert!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_lines()
                .iter()
                .any(|line| line.count == 9)
        );

        let paginator = state.reader.paginator.as_ref().unwrap();
        let layout = paginator.layout;
        let line = paginator
            .current_page()
            .lines
            .iter()
            .find(|line| line.start <= 1 && line.end >= 3)
            .unwrap();
        let metrics = &state.fonts[0].metrics;
        let margin = (state.platform.logical_viewport_size().0 as f32 - layout.width) / 2.0;
        let x = (margin
            + (line
                .x_at_offset(1, &layout, metrics)
                .expect("Admit the underline fixture")
                + line
                    .x_at_offset(3, &layout, metrics)
                    .expect("Admit the underline fixture"))
                / 2.0)
            / state.platform.logical_viewport_size().0 as f32;
        let y = line.y + line.height / 2.0;
        assert_eq!(state.reader_hit_mark(x, y)?, Some("remote:0".into()));
        state.reader_action("thought", "remote:0")?;
        assert_eq!(state.reader.selection, Some((1, 3)));
        state.render_reader()?;
        assert_eq!(state.reader.underline_projection.counters(), before);
        assert!(fixture.ui().global::<ReaderModel>().get_thought_open());
    }
    fixture.finish()
}

fn invalidation() -> Result<()> {
    let fixture = Fixture::new(BODY, records(), true)?;
    {
        let mut state = fixture.app();
        let mut builds = state.reader.underline_projection.counters().builds;
        save_chapter(&state.store, BOOK, CHAPTER, 0, MOVED_BODY)?;
        state.load_local_reader(BOOK, 0, None)?;
        state.render_reader()?;
        builds += 1;
        assert_eq!(state.reader.underline_projection.counters().builds, builds);
        assert_eq!(state.reader.underline_projection.range(0), Some((2, 4)));

        let request = state.request();
        state.reader.underline_request = request;
        let epoch = state.jobs.account_epoch();
        state.handle_job(JobEvent::new(
            request,
            epoch,
            JobResult::Underlines {
                book_id: BOOK.into(),
                chapter_id: CHAPTER.into(),
                result: Ok(vec![underline("marker", 4)]),
            },
        ))?;
        builds += 1;
        assert_eq!(state.reader.underline_projection.counters().builds, builds);
        assert_eq!(state.reader.underline_projection.range(0), Some((5, 11)));

        state.load_local_reader(BOOK, 1, None)?;
        state.render_reader()?;
        builds += 1;
        assert_eq!(state.reader.underline_projection.counters().builds, builds);
        assert_eq!(state.reader.underline_projection.range(0), Some((0, 6)));

        let mut book = Book::new("underline-fixture-b", "Other underline fixture book");
        book.source = "local".into();
        book.total_chapters = 2;
        state.store.upsert_book(&book)?;
        save_chapter(
            &state.store,
            &book.id,
            "underline-fixture-b1",
            1,
            "Second chapter marker.",
        )?;
        state.store.set_setting(
            "underlines:underline-fixture-b:underline-fixture-b1",
            &vec![underline("Second", 7)],
        )?;
        state.load_local_reader(&book.id, 1, None)?;
        state.render_reader()?;
        builds += 1;
        assert_eq!(state.reader.underline_projection.counters().builds, builds);
        state.jobs.cancel_all();
        state.discard_cancelled_job_events();
        state.render_reader()?;
        builds += 1;
        assert_eq!(state.reader.underline_projection.counters().builds, builds);
        state.invalidate_account_work()?;
        state.render_reader()?;
        assert!(state.reader.underlines.is_empty());
        assert_eq!(state.reader.underline_projection.range(0), None);
        assert_eq!(
            state.reader.underline_projection.counters().builds,
            builds + 1
        );
    }
    fixture.finish()
}

fn hidden() -> Result<()> {
    let fixture = Fixture::new(BODY, records(), false)?;
    {
        let mut state = fixture.app();
        assert!(state.reader.underlines.is_empty());
        let before = state.reader.underline_projection.counters();
        for _ in 0..64 {
            state.render_reader()?;
        }
        assert_eq!(state.reader.underline_projection.counters(), before);
        assert_eq!(before.builds, 0);
        assert_eq!(remote_marks(fixture.ui()), 0);
        state.reader_action("thought", "remote:0")?;
        assert!(state.reader.selection.is_none());
        assert!(!fixture.ui().global::<ReaderModel>().get_thought_open());
        state.reader_action("popular", "toggle")?;
        state.render_reader()?;
        assert_eq!(state.reader.underline_projection.counters().matches, 3);
        assert_eq!(remote_marks(fixture.ui()), 2);
        state.reader_action("popular", "toggle")?;
        state.render_reader()?;
        let hidden = state.reader.underline_projection.counters();
        for _ in 0..64 {
            state.render_reader()?;
        }
        assert_eq!(state.reader.underline_projection.counters(), hidden);
        assert!(state.reader.underlines.is_empty());
        assert_eq!(remote_marks(fixture.ui()), 0);
        assert_eq!(state.jobs.metadata_capacity(), 8);
    }
    fixture.finish()
}

fn dense() -> Result<()> {
    let fixture = Fixture::new("x starts this page.", vec![underline("x", 1); 1025], true)?;
    {
        let mut state = fixture.app();
        let before = state.reader.underline_projection.counters();
        assert_eq!(before.matches, 1);
        assert_eq!(remote_marks(fixture.ui()), 0);
        assert_eq!(fixture.ui().global::<ReaderModel>().get_thought_count(), 0);
        for _ in 0..32 {
            state.render_reader()?;
        }
        assert_eq!(state.reader.underline_projection.counters(), before);
        state.reader_action("thought", "remote:0")?;
        assert!(state.reader.selection.is_none());
        assert!(!state.reader.body.is_empty() && state.reader.paginator.is_some());
    }
    fixture.finish()
}

fn failed_work() -> Result<()> {
    let fixture = Fixture::new(BODY, records(), true)?;
    {
        let mut state = fixture.app();
        let original_body = state.reader.body.clone();
        let original_session = state.jobs.session();
        let before = state.reader.underline_projection.counters();
        state.reader.underline_projection.set_test_work_limit(1);
        state.render_reader()?;
        let rejected = state.reader.underline_projection.counters();
        assert_eq!(rejected.builds, before.builds + 1);
        assert_eq!(rejected.matches, before.matches);
        assert_eq!(state.reader.underline_projection.range(0), None);
        for _ in 0..64 {
            state.render_reader()?;
        }
        assert_eq!(state.reader.underline_projection.counters(), rejected);
        assert_eq!(state.reader.body, original_body);
        assert_eq!(state.jobs.session(), original_session);
        assert!(fixture.ui().global::<ReaderModel>().get_lines().row_count() > 0);
        assert_eq!(remote_marks(fixture.ui()), 0);
    }
    fixture.finish()
}

fn settle_underlines(state: &mut App, records: Vec<RemoteUnderline>) -> Result<()> {
    let request = state.request();
    state.reader.underline_request = request;
    state.handle_job(JobEvent::new(
        request,
        state.jobs.account_epoch(),
        JobResult::Underlines {
            book_id: BOOK.into(),
            chapter_id: CHAPTER.into(),
            result: Ok(records),
        },
    ))?;
    Ok(())
}

fn opened_view_gate() -> Result<()> {
    let fixture = Fixture::new("x starts this page.", vec![underline("x", 1)], true)?;
    {
        let mut state = fixture.app();
        state.reader_action("thought", "remote:0")?;
        state.render_reader()?;
        assert!(fixture.ui().global::<ReaderModel>().get_thought_open());
        state.reader.draft.set_query("synthetic unsaved draft");
        settle_underlines(&mut state, vec![underline("x", 1); 1025])?;
        assert!(!fixture.ui().global::<ReaderModel>().get_thought_open());
        assert!(state.reader.thought_id.is_empty() && state.reader.selection.is_none());
        assert_eq!(state.reader.draft.query, "synthetic unsaved draft");
        assert_eq!(remote_marks(fixture.ui()), 0);
        assert!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_lines()
                .iter()
                .all(|line| line.marks.iter().all(|mark| !mark.selected))
        );

        settle_underlines(&mut state, vec![underline("x", 1)])?;
        state.reader_action("thought", "remote:0")?;
        state.reader.draft.set_query("synthetic active local draft");
        fixture
            .ui()
            .global::<ReaderModel>()
            .set_writing_thought(true);
        settle_underlines(&mut state, vec![underline("x", 1); 1025])?;
        assert!(!fixture.ui().global::<ReaderModel>().get_thought_open());
        assert!(fixture.ui().global::<ReaderModel>().get_writing_thought());
        assert_eq!(state.reader.selection, Some((0, 1)));
        assert_eq!(state.reader.draft.query, "synthetic active local draft");

        fixture
            .ui()
            .global::<ReaderModel>()
            .set_writing_thought(false);
        state.store.save_annotation(&Annotation {
            id: "synthetic-local-note".into(),
            book_id: BOOK.into(),
            chapter_index: 0,
            start: 0,
            end: 1,
            quote: "x".into(),
            note: "Stored local note".into(),
            created_at: 0,
        })?;
        state.reader_action("thought", "mine:synthetic-local-note")?;
        state.render_reader()?;
        settle_underlines(&mut state, vec![underline("x", 1); 1025])?;
        assert!(fixture.ui().global::<ReaderModel>().get_thought_open());
        assert_eq!(
            fixture.ui().global::<ReaderModel>().get_note_id(),
            "synthetic-local-note"
        );
        assert_eq!(state.reader.thought_id, "mine:synthetic-local-note");
        assert_eq!(state.reader.selection, Some((0, 1)));
        assert_eq!(state.reader.draft.query, "Stored local note");
    }
    fixture.finish()
}

struct BoundedChild(Child);

impl Drop for BoundedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

pub(super) fn isolated(case: &'static str, scenario: fn() -> Result<()>) -> Result<()> {
    const CHILD: &str = "WEREAD_UNDERLINE_PROJECTION_CHILD";
    const COMPLETED: &str = "WEREAD_UNDERLINE_PROJECTION_COMPLETED";
    if std::env::var(CHILD).ok().as_deref() == Some(case) {
        std::thread::spawn(scenario)
            .join()
            .expect("The underline UI fixture panicked")?;
        let marker = std::env::var_os(COMPLETED)
            .ok_or_else(|| anyhow::anyhow!("The underline fixture completion path is missing"))?;
        std::fs::write(marker, case.as_bytes())?;
        return Ok(());
    }
    let completion = tempfile::tempdir()?;
    let marker = completion.path().join("completed");
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--exact", case, "--nocapture", "--test-threads=1"])
        .env(CHILD, case)
        .env(COMPLETED, &marker)
        .env_remove("WEREAD_API_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
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
    let mut child = BoundedChild(command.spawn()?);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.0.try_wait()? {
            anyhow::ensure!(
                status.success() && std::fs::read(&marker).unwrap_or_default() == case.as_bytes(),
                "The isolated underline fixture did not complete: {case} ({status})"
            );
            return Ok(());
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "The isolated underline fixture timed out: {case}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn repeated_underline_renders_and_clicks_reuse_checked_positions() -> Result<()> {
    isolated(
        "app::underline_projection_tests::repeated_underline_renders_and_clicks_reuse_checked_positions",
        repeated_renders,
    )
}

#[test]
fn underline_body_list_chapter_book_and_account_changes_invalidate_projection() -> Result<()> {
    isolated(
        "app::underline_projection_tests::underline_body_list_chapter_book_and_account_changes_invalidate_projection",
        invalidation,
    )
}

#[test]
fn disabled_popular_underlines_do_no_remote_work_or_hidden_derivation() -> Result<()> {
    isolated(
        "app::underline_projection_tests::disabled_popular_underlines_do_no_remote_work_or_hidden_derivation",
        hidden,
    )
}

#[test]
fn dense_underline_page_marks_stay_bounded_without_losing_the_reader() -> Result<()> {
    isolated(
        "app::underline_projection_tests::dense_underline_page_marks_stay_bounded_without_losing_the_reader",
        dense,
    )
}

#[test]
fn failed_underline_matching_is_cached_and_keeps_body_and_account() -> Result<()> {
    isolated(
        "app::underline_projection_tests::failed_underline_matching_is_cached_and_keeps_body_and_account",
        failed_work,
    )
}

#[test]
fn rejected_underline_page_closes_only_the_unavailable_remote_thought() -> Result<()> {
    isolated(
        "app::underline_projection_tests::rejected_underline_page_closes_only_the_unavailable_remote_thought",
        opened_view_gate,
    )
}
