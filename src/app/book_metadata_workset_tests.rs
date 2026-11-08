//! Small legal EPUB descriptions drive genuine intro-free Library/Stats/Search.

use super::underline_projection_tests::isolated;
use super::*;
use crate::store::{BookMetadataLimit, BookMetadataLimits, with_book_metadata_test_limits};
use chrono::{DateTime, NaiveDate, Utc};
use slint::{
    Model,
    platform::{
        Platform as UiPlatform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::io::{Cursor, Write};
use std::path::Path;

fn now() -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(2026, 11, 8)
        .unwrap()
        .and_hms_opt(6, 0, 0)
        .unwrap()
        .and_utc()
}
fn fixed<T>(run: impl FnOnce() -> T) -> T {
    crate::statistics::with_year_projection_test_context(512, 512 * 512, now(), run)
}

fn imported(path: &Path, ordinal: usize, intro: &str) -> Result<crate::importer::ImportedBook> {
    let package = format!(
        "<package version='2.0'><metadata><title>Metadata fixture {ordinal}</title><creator>Metadata author</creator><subject>Metadata category</subject><description>{intro}</description></metadata><manifest><item id='chapter' href='chapter.xhtml' media-type='application/xhtml+xml'/></manifest><spine><itemref idref='chapter'/></spine></package>"
    );
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in [
        ("mimetype", b"application/epub+zip".as_slice()),
        (
            "META-INF/container.xml",
            b"<container><rootfiles><rootfile full-path='book.opf'/></rootfiles></container>"
                .as_slice(),
        ),
        ("book.opf", package.as_bytes()),
        (
            "chapter.xhtml",
            b"<html><body><h1>Fixture chapter</h1><p>Retained reading body.</p></body></html>"
                .as_slice(),
        ),
    ] {
        writer.start_file(name, options)?;
        writer.write_all(body)?;
    }
    let bytes = writer.finish()?.into_inner();
    assert!(bytes.len() < 8192);
    std::fs::write(path, bytes)?;
    crate::importer::import_file(path)
}

struct MetadataUi;
impl UiPlatform for MetadataUi {
    fn create_window_adapter(
        &self,
    ) -> std::result::Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

struct Fixture {
    app: Option<Rc<RefCell<App>>>,
    ui: Option<AppWindow>,
    runtime: Option<tokio::runtime::Runtime>,
    directory: tempfile::TempDir,
    books: Vec<Book>,
}

impl Fixture {
    fn new() -> Result<Self> {
        slint::platform::set_platform(Box::new(MetadataUi))?;
        let directory = tempfile::tempdir()?;
        let account = source::AuthSession {
            api_key: "synthetic-book-metadata-key".into(),
            user_vid: "synthetic-book-metadata-account".into(),
            cookies: BTreeMap::from([
                ("wr_vid".into(), "synthetic-book-metadata-account".into()),
                ("wr_skey".into(), "synthetic-book-metadata-session".into()),
            ]),
            ..Default::default()
        };
        SessionFile::new(directory.path()).save(&account)?;
        let mut books = Vec::new();
        {
            let store = Store::open(directory.path().join("library.sqlite"))?;
            let mut preferences = AppPreferences::for_device(false);
            preferences.pull_on_open = false;
            preferences.push_on_close = false;
            preferences.report_time = false;
            preferences.popular = false;
            store.set_setting("preferences", &preferences)?;
            for ordinal in 0..3 {
                let intro = format!("Keep introduction {ordinal}:{}", "x".repeat(4096));
                let mut imported = imported(
                    &directory.path().join(format!("fixture-{ordinal}.epub")),
                    ordinal,
                    &intro,
                )?;
                assert_eq!(imported.book.intro, intro);
                imported.book.finished = ordinal == 0;
                store.upsert_book(&imported.book)?;
                store.upsert_chapters(&imported.chapters)?;
                books.push(imported.book);
            }
            store.record_session(&ReadingSession {
                id: "metadata-session".into(),
                book_id: books[0].id.clone(),
                started_at: now().timestamp() - 60,
                ended_at: now().timestamp(),
                seconds: 30,
            })?;
            store.save_annotation(&Annotation {
                id: "metadata-note".into(),
                book_id: books[0].id.clone(),
                start: 0,
                end: 8,
                quote: "Retained".into(),
                note: "Keep metadata note".into(),
                created_at: now().timestamp() - 1,
                ..Default::default()
            })?;
            store.save_bookmark(&Bookmark {
                id: "metadata-bookmark".into(),
                book_id: books[0].id.clone(),
                char_offset: 1,
                label: "Keep bookmark".into(),
                ..Default::default()
            })?;
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let platform = Arc::new(Platform::new(DeviceProfile::Paperwhite6, false)?);
        let ui = AppWindow::new()?;
        crate::platform::configure_window(ui.window(), &platform)?;
        let app = App::attach(
            &ui,
            AppOptions {
                data_dir: directory.path().to_path_buf(),
                profile: DeviceProfile::Paperwhite6,
                demo: false,
                offline: true,
                initial_screen: "library".into(),
                fonts: vec![FontRecord {
                    id: "song".into(),
                    label: "Metadata fixture font".into(),
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
            let mut state = app.borrow_mut();
            state.load_local_reader(&books[0].id, 0, Some(0))?;
            state.reader.selection = Some((0, 8));
            assert!(state.reader.draft.set_query("Keep metadata draft"));
        }
        Ok(Self {
            app: Some(app),
            ui: Some(ui),
            runtime: Some(runtime),
            directory,
            books,
        })
    }
    fn state(&self) -> std::cell::RefMut<'_, App> {
        self.app.as_ref().unwrap().borrow_mut()
    }
    fn ui(&self) -> &AppWindow {
        self.ui.as_ref().unwrap()
    }
    fn finish(mut self) -> Result<()> {
        let app = self.app.take().unwrap();
        app.borrow_mut().terminal_shutdown()?;
        let weak = Rc::downgrade(&app);
        drop(app);
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
        if let Some(app) = self.app.take() {
            let _ = app.borrow_mut().terminal_shutdown();
        }
        if let Some(ui) = self.ui.take() {
            let _ = ui.hide();
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }
}

#[derive(Debug, PartialEq)]
struct LibraryView {
    ids: Vec<String>,
    summary: String,
    page: i32,
    pages: i32,
    sort: String,
    selecting: bool,
}
fn library_view(fixture: &Fixture) -> LibraryView {
    let global = fixture.ui().global::<LibraryModel>();
    LibraryView {
        ids: global
            .get_books()
            .iter()
            .map(|book| book.id.into())
            .collect(),
        summary: global.get_summary().into(),
        page: global.get_page(),
        pages: global.get_page_count(),
        sort: global.get_sort().into(),
        selecting: global.get_selecting(),
    }
}

fn rendering() -> Result<()> {
    let fixture = Fixture::new()?;
    for sort in ["title", "progress", "downloaded", "finished", "notes"] {
        let mut state = fixture.state();
        state.library_sort = sort.into();
        state.store.reset_book_read_stats();
        state.render_library()?;
        assert_eq!(state.store.book_read_stats().intro_copies, 0);
        assert!(state.store.book_read_stats().metadata_rows >= 3);
    }
    {
        let mut state = fixture.state();
        state.stats_period = "total".into();
        state.remote_stats = None;
        state.store.reset_book_read_stats();
        let report = fixed(|| state.local_statistics_for_test())?;
        assert_eq!((report.seconds, report.finished), (30, 1));
        assert_eq!(report.ranking[0].title, fixture.books[0].title);
        fixed(|| state.render_statistics())?;
        assert_eq!(state.store.book_read_stats().intro_copies, 0);
        assert_eq!(state.store.book_read_stats().metadata_rows, 6);
    }
    {
        let mut state = fixture.state();
        state.store.reset_book_read_stats();
        state.load_detail(&fixture.books[0].id)?;
        state.detail_tab = "intro".into();
        state.render_detail()?;
        assert!(state.store.book_read_stats().intro_copies > 0);
        assert_eq!(
            state.store.book_read_stats().intro_bytes,
            fixture.books[0].intro.len()
        );
        assert_eq!(
            fixture.ui().global::<DetailModel>().get_intro().as_str(),
            fixture.books[0].intro.as_str()
        );
        assert_eq!(
            state.store.book(&fixture.books[0].id)?.unwrap().intro,
            fixture.books[0].intro
        );
    }
    fixture.finish()
}

fn quota() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.state().render_library()?;
    let original = library_view(&fixture);
    let epoch = fixture.state().jobs.account_epoch();
    let prefs = serde_json::to_value(&fixture.state().prefs)?;
    let notes = fixture.state().store.annotations(&fixture.books[0].id)?;
    with_book_metadata_test_limits(
        BookMetadataLimits {
            bytes: 1,
            ..Default::default()
        },
        || -> Result<()> {
            let mut state = fixture.state();
            state.store.reset_book_read_stats();
            state.render_library()?;
            assert_eq!(library_view(&fixture), original);
            assert_eq!(state.store.book_read_stats().metadata_rows, 0);
            assert_eq!(state.store.book_read_stats().intro_copies, 0);
            assert_eq!(
                fixture.ui().global::<LibraryModel>().get_status(),
                tr("api_error")
            );
            assert_eq!(
                fixture.ui().global::<ShellModel>().get_toast(),
                tr("api_error")
            );
            let error = fixed(|| state.local_statistics_for_test()).unwrap_err();
            assert!(error.downcast_ref::<BookMetadataLimit>().is_some());
            Ok(())
        },
    )?;
    {
        let state = fixture.state();
        assert_eq!(state.jobs.account_epoch(), epoch);
        assert_eq!(serde_json::to_value(&state.prefs)?, prefs);
        assert_eq!(state.reader.selection, Some((0, 8)));
        assert_eq!(state.reader.draft.query, "Keep metadata draft");
        assert_eq!(state.store.annotations(&fixture.books[0].id)?, notes);
        for book in &fixture.books {
            assert_eq!(state.store.book(&book.id)?.unwrap().intro, book.intro);
        }
    }
    fixture.finish()
}

fn search() -> Result<()> {
    let fixture = Fixture::new()?;
    {
        let mut state = fixture.state();
        state.route = Route::Search;
        state.query = PinyinInput::new("Metadata fixture");
        state.search_purpose = "books".into();
        state.store.reset_book_read_stats();
        state.search_action("scope", "shelf")?;
        state.render_search()?;
        assert_eq!(state.search_results.len(), 3);
        assert_eq!(state.store.book_read_stats().intro_copies, 0);
        state.search_action("add", &fixture.books[0].id)?;
        assert_eq!(state.store.book_read_stats().intro_copies, 0);
        assert_eq!(
            state.store.book(&fixture.books[0].id)?.unwrap().intro,
            fixture.books[0].intro
        );
    }
    fixture.finish()
}

fn sql_error() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.state().render_library()?;
    let original = library_view(&fixture);
    let other = rusqlite::Connection::open(fixture.directory.path().join("library.sqlite"))?;
    other.execute_batch("ALTER TABLE books RENAME TO retained_books")?;
    let result = fixture.state().render_library();
    other.execute_batch("ALTER TABLE retained_books RENAME TO books")?;
    let error = result.unwrap_err();
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
    assert!(error.downcast_ref::<BookMetadataLimit>().is_none());
    assert_eq!(library_view(&fixture), original);
    fixture.finish()
}

#[test]
fn genuine_import_library_and_statistics_skip_intro_while_detail_reads_it_intact() -> Result<()> {
    isolated(
        "app::book_metadata_workset_tests::genuine_import_library_and_statistics_skip_intro_while_detail_reads_it_intact",
        rendering,
    )
}
#[test]
fn genuine_book_metadata_quota_keeps_library_rows_and_private_history() -> Result<()> {
    isolated(
        "app::book_metadata_workset_tests::genuine_book_metadata_quota_keeps_library_rows_and_private_history",
        quota,
    )
}
#[test]
fn genuine_local_search_and_add_use_metadata_without_overwriting_intro() -> Result<()> {
    isolated(
        "app::book_metadata_workset_tests::genuine_local_search_and_add_use_metadata_without_overwriting_intro",
        search,
    )
}
#[test]
fn genuine_book_metadata_sql_failure_remains_an_error_with_old_rows() -> Result<()> {
    isolated(
        "app::book_metadata_workset_tests::genuine_book_metadata_sql_failure_remains_an_error_with_old_rows",
        sql_error,
    )
}
