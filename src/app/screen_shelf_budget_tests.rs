//! Genuine offline attachment and initial-screen selection use scalar shelf evidence.

use super::underline_projection_tests::isolated;
use super::*;
use slint::platform::{
    Platform as UiPlatform, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};

const FIRST: &str = "z-first-shelf-row";
const SECOND: &str = "a-second-shelf-row";

struct ScreenUi;

impl UiPlatform for ScreenUi {
    fn create_window_adapter(
        &self,
    ) -> std::result::Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

struct Fixture {
    application: Option<Rc<RefCell<App>>>,
    ui: Option<AppWindow>,
    runtime: Option<tokio::runtime::Runtime>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new(books: bool, configured: bool, screen: &str) -> Result<Self> {
        slint::platform::set_platform(Box::new(ScreenUi))?;
        let directory = tempfile::tempdir()?;
        {
            let store = Store::open(directory.path().join("library.sqlite"))?;
            let mut preferences = AppPreferences::for_device(false);
            preferences.pull_on_open = false;
            preferences.push_on_close = false;
            preferences.report_time = false;
            store.set_setting("preferences", &preferences)?;
            if books {
                for (id, title, body) in [
                    (FIRST, "Z first inserted title", "First row body."),
                    (SECOND, "A second inserted title", "Second row body."),
                ] {
                    let mut book = Book::new(id, title);
                    book.source = "local".into();
                    book.on_shelf = true;
                    book.total_chapters = 1;
                    book.intro = format!("Retained {id} introduction. ").repeat(2048);
                    store.upsert_book(&book)?;
                    store.upsert_chapter(&Chapter {
                        id: format!("{id}-chapter"),
                        book_id: id.into(),
                        title: title.into(),
                        body: Some(body.into()),
                        cached: true,
                        content_status: ContentStatus::Full,
                        ..Default::default()
                    })?;
                }
            }
        }
        if configured {
            SessionFile::new(directory.path()).save(&source::AuthSession {
                api_key: "synthetic-screen-key".into(),
                user_vid: "synthetic-screen-account".into(),
                cookies: BTreeMap::from([
                    ("wr_vid".into(), "synthetic-screen-account".into()),
                    ("wr_skey".into(), "synthetic-screen-session".into()),
                ]),
                ..Default::default()
            })?;
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let platform = Arc::new(Platform::new(DeviceProfile::Paperwhite6, false)?);
        let ui = AppWindow::new()?;
        crate::platform::configure_window(ui.window(), &platform)?;
        let application = App::attach(
            &ui,
            AppOptions {
                data_dir: directory.path().to_path_buf(),
                profile: DeviceProfile::Paperwhite6,
                demo: false,
                offline: true,
                initial_screen: screen.into(),
                fonts: vec![FontRecord {
                    id: "song".into(),
                    label: "Screen fixture font".into(),
                    family: "sans-serif".into(),
                    metrics: TextMetrics::default(),
                    weights: BTreeMap::new(),
                    available: true,
                }],
            },
            platform,
            &runtime,
        )?;
        assert_eq!(application.borrow().jobs.configured(), configured);
        assert!(!application.borrow().demo);
        assert!(application.borrow().force_offline);
        Ok(Self {
            application: Some(application),
            ui: Some(ui),
            runtime: Some(runtime),
            _directory: directory,
        })
    }
    fn app(&self) -> std::cell::RefMut<'_, App> {
        self.application.as_ref().unwrap().borrow_mut()
    }
    fn ui(&self) -> &AppWindow {
        self.ui.as_ref().unwrap()
    }
    fn finish(mut self) -> Result<()> {
        let application = self.application.take().unwrap();
        application.borrow_mut().terminal_shutdown()?;
        let weak = Rc::downgrade(&application);
        drop(application);
        assert!(weak.upgrade().is_none());
        let ui = self.ui.take().unwrap();
        let weak = ui.as_weak();
        ui.hide()?;
        drop(ui);
        slint::platform::update_timers_and_animations();
        assert!(weak.upgrade().is_none());
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
        }
        if let Some(ui) = self.ui.take() {
            let _ = ui.hide();
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }
}

fn sql(state: &App) -> Result<rusqlite::Connection> {
    Ok(rusqlite::Connection::open(
        state.data_dir.join("library.sqlite"),
    )?)
}

fn intros(state: &App) -> Result<Vec<(String, String)>> {
    let database = sql(state)?;
    let mut statement = database.prepare("SELECT id,intro FROM books ORDER BY rowid")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn unconfigured_import() -> Result<()> {
    let fixture = Fixture::new(true, false, "")?;
    {
        let state = fixture.app();
        assert_eq!(state.route, Route::Library);
        let reads = state.store.book_read_stats();
        assert_eq!(reads.intro_copies, 0);
        assert_eq!(reads.intro_bytes, 0);
        assert!(reads.metadata_rows > 0); // The actual Library still renders its admitted metadata.
        let stored = intros(&state)?;
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].0, FIRST);
        assert!(stored.iter().all(|(_, intro)| intro.len() > 16 * 1024));
        assert!(state.jobs.session().is_none());
    }
    fixture.finish()
}

#[test]
fn unconfigured_non_demo_attachment_uses_local_shelf_presence_without_intro_materialization()
-> Result<()> {
    isolated(
        "app::screen_shelf_budget_tests::unconfigured_non_demo_attachment_uses_local_shelf_presence_without_intro_materialization",
        unconfigured_import,
    )
}

fn unrelated_screens() -> Result<()> {
    let fixture = Fixture::new(true, false, "")?;
    {
        let mut state = fixture.app();
        let original = intros(&state)?;
        let account = state.jobs.session();
        state.history = vec![Route::Search, Route::Publications];
        let history = state.history.clone();
        let database = sql(&state)?;
        database.execute_batch("ALTER TABLE books RENAME TO screen_hidden_books")?;
        for (screen, route) in [
            ("login", Route::Login),
            ("search", Route::Search),
            ("mp", Route::Publications),
            ("stats", Route::Statistics),
            ("settings", Route::Settings),
            ("unknown", Route::Library),
        ] {
            state.store.reset_book_read_stats();
            assert_eq!(state.requested_screen_route(screen)?, route);
            state.prepare_screen(screen)?;
            assert_eq!(state.route, route);
            let reads = state.store.book_read_stats();
            assert_eq!(reads.metadata_rows, 0);
            assert_eq!(reads.metadata_string_bytes, 0);
            assert_eq!(reads.intro_copies, 0);
            assert_eq!(state.jobs.session(), account);
            assert_eq!(state.history, history);
        }
        state.mp_articles = BTreeMap::from([(
            "screen-owner".into(),
            vec![MpArticle {
                id: "screen-article".into(),
                ..Default::default()
            }],
        )]);
        state.store.reset_book_read_stats();
        assert_eq!(
            state.requested_screen_route("article")?,
            Route::Article("screen-article".into())
        );
        assert_eq!(state.store.book_read_stats().metadata_rows, 0);
        database.execute_batch("ALTER TABLE screen_hidden_books RENAME TO books")?;
        assert_eq!(intros(&state)?, original);
    }
    fixture.finish()
}

#[test]
fn unrelated_screen_routes_do_not_touch_even_an_unavailable_books_table() -> Result<()> {
    isolated(
        "app::screen_shelf_budget_tests::unrelated_screen_routes_do_not_touch_even_an_unavailable_books_table",
        unrelated_screens,
    )
}

fn first_id_and_routes() -> Result<()> {
    let fixture = Fixture::new(true, false, "")?;
    {
        let mut state = fixture.app();
        let original = intros(&state)?;
        let account = state.jobs.session();
        state.history = vec![Route::Search, Route::Publications];
        let history = state.history.clone();
        for (screen, route) in [
            ("detail", Route::Detail(FIRST.into())),
            ("reader", Route::Reader(FIRST.into(), 0)),
        ] {
            state.store.reset_book_read_stats();
            assert_eq!(state.requested_screen_route(screen)?, route);
            let selected = state.store.book_read_stats();
            assert_eq!(selected.metadata_rows, 1);
            assert_eq!(selected.metadata_string_bytes, FIRST.len());
            assert_eq!(selected.intro_copies, 0);
            assert_eq!(selected.intro_bytes, 0);
            state.store.reset_book_read_stats();
            state.prepare_screen(screen)?;
            assert_eq!(state.route, route);
            let navigation = state.store.book_read_stats();
            if screen == "detail" {
                // Existing Detail loading may consume its requested full book;
                // the selector above must still own only this one admitted ID.
                assert!(navigation.intro_copies <= 1);
                assert!(navigation.intro_bytes <= original[0].1.len());
            } else {
                assert_eq!(navigation.intro_copies, 0);
                assert_eq!(navigation.intro_bytes, 0);
            }
            assert_eq!(state.history, history);
            assert_eq!(state.jobs.session(), account);
        }
        assert_eq!(state.reader.book_id, FIRST);
        assert_eq!(state.reader.chapter_index, 0);
        assert_eq!(state.reader.body, "First row body.");
        assert_eq!(intros(&state)?, original);
    }
    fixture.finish()
}

#[test]
fn detail_and_reader_select_only_the_rowid_first_id_before_normal_navigation() -> Result<()> {
    isolated(
        "app::screen_shelf_budget_tests::detail_and_reader_select_only_the_rowid_first_id_before_normal_navigation",
        first_id_and_routes,
    )
}

fn empty_detail() -> Result<()> {
    let fixture = Fixture::new(false, false, "detail")?;
    {
        let mut state = fixture.app();
        assert_eq!(state.route, Route::Detail(String::new()));
        assert!(state.detail_id.is_empty());
        assert_eq!(
            fixture.ui().global::<DetailModel>().get_book().id.as_str(),
            ""
        );
        assert_eq!(state.store.book_read_stats().intro_copies, 0);
        assert_eq!(state.store.book_read_stats().metadata_rows, 0);
        assert!(state.jobs.session().is_none());
        state.history = vec![Route::Search];
        state.prepare_screen("detail")?;
        assert_eq!(state.history, vec![Route::Search]);
        assert_eq!(state.route, Route::Detail(String::new()));
    }
    fixture.finish()
}

#[test]
fn empty_shelf_detail_keeps_the_empty_detail_screen_without_a_metadata_lookup() -> Result<()> {
    isolated(
        "app::screen_shelf_budget_tests::empty_shelf_detail_keeps_the_empty_detail_screen_without_a_metadata_lookup",
        empty_detail,
    )
}

fn empty_reader() -> Result<()> {
    let fixture = Fixture::new(false, false, "reader")?;
    {
        let mut state = fixture.app();
        assert_eq!(state.route, Route::Library);
        assert_eq!(
            fixture.ui().global::<ShellModel>().get_toast(),
            tr("empty_library")
        );
        assert!(state.reader.book_id.is_empty() && state.reader.body.is_empty());
        assert_eq!(state.store.book_read_stats().intro_copies, 0);
        assert_eq!(state.store.book_read_stats().metadata_rows, 0);
        assert!(state.jobs.session().is_none());
        state.history = vec![Route::Search];
        state.prepare_screen("reader")?;
        assert_eq!(state.route, Route::Library);
        assert_eq!(state.history, vec![Route::Search]);
        assert_eq!(
            fixture.ui().global::<ShellModel>().get_toast(),
            tr("empty_library")
        );
    }
    fixture.finish()
}

#[test]
fn empty_shelf_reader_keeps_the_library_and_empty_library_notice() -> Result<()> {
    isolated(
        "app::screen_shelf_budget_tests::empty_shelf_reader_keeps_the_library_and_empty_library_notice",
        empty_reader,
    )
}

fn configured_without_shelf() -> Result<()> {
    let fixture = Fixture::new(false, true, "")?;
    {
        let state = fixture.app();
        assert_eq!(state.route, Route::Library);
        assert!(state.jobs.session().is_some());
        let reads = state.store.book_read_stats();
        assert_eq!(reads.metadata_rows, 0);
        assert_eq!(reads.intro_copies, 0);
        assert_eq!(reads.intro_bytes, 0);
        assert!(state.history.is_empty());
    }
    fixture.finish()
}

#[test]
fn configured_empty_attachment_preserves_the_normal_library_start() -> Result<()> {
    isolated(
        "app::screen_shelf_budget_tests::configured_empty_attachment_preserves_the_normal_library_start",
        configured_without_shelf,
    )
}
