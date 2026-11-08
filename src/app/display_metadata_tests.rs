//! Real cold App attachment and article renders with scalar-only display lookups.

use super::underline_projection_tests::isolated;
use super::*;
use crate::reader::caret_test_font;
use anyhow::Context;
use slint::{
    Model,
    fontique_010::fontique,
    platform::{
        Platform as UiPlatform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};

const BOOK: &str = "display-reader-book";
const READER_CHAPTERS: u32 = 6;
const RESUME_CHAPTER: u32 = 3;
const ACCOUNT: &str = "display-publication-account";
const ARTICLES: usize = 15;
const RESUME_KEY: &str = "reader.resume.v1";

struct MetadataUi;

impl UiPlatform for MetadataUi {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

fn session() -> source::AuthSession {
    source::AuthSession {
        api_key: "synthetic-display-metadata-key".into(),
        user_vid: "synthetic-display-metadata-account".into(),
        account_name: "Display metadata fixture".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "synthetic-display-metadata-account".into()),
            (
                "wr_skey".into(),
                "synthetic-display-metadata-session".into(),
            ),
        ]),
        ..Default::default()
    }
}

fn reader_uid(index: u32) -> String {
    format!("display-reader-chapter-{index}")
}
fn article_id(index: usize) -> String {
    format!("display-article-{index:03}")
}
fn article_book(index: usize) -> String {
    format!("article:{}", article_id(index))
}

fn reader_body(index: u32) -> String {
    if index == RESUME_CHAPTER {
        format!("A\u{1f642}\u{4e2d}B\n{}", "AB\n".repeat(160))
    } else if index == 0 {
        "AB retained local Reader.\n".repeat(16)
    } else {
        format!("Hidden reader chapter {index}.\n").repeat(256)
    }
}

fn article_body(index: usize) -> String {
    if index == 0 {
        "AB\n".repeat(160)
    } else {
        format!("Hidden article {index}.\n").repeat(512)
    }
}

struct Fixture {
    directory: tempfile::TempDir,
    runtime: Option<tokio::runtime::Runtime>,
    platform: Arc<Platform>,
    metrics: TextMetrics,
}

struct Instance {
    ui: AppWindow,
    app: Rc<RefCell<App>>,
}

impl Fixture {
    fn new() -> Result<Self> {
        slint::platform::set_platform(Box::new(MetadataUi))?;
        let bytes = Arc::new(caret_test_font::bytes());
        let index = {
            let mut collection = slint::fontique_010::shared_collection();
            let mut index = None;
            for (family, fonts) in
                collection.register_fonts(fontique::Blob::new(bytes.clone()), None)
            {
                if collection.family_name(family) == Some(caret_test_font::FAMILY) {
                    index = fonts.first().map(|font| font.index());
                }
            }
            index.context("The original small font must register for the App fixture")?
        };
        let metrics =
            TextMetrics::from_font(bytes.as_slice(), index).map_err(anyhow::Error::msg)?;
        assert!(metrics.is_font_backed());
        let directory = tempfile::tempdir()?;
        SessionFile::new(directory.path()).save(&session())?;
        let fixture = Self {
            directory,
            runtime: Some(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?,
            ),
            platform: Arc::new(Platform::new(DeviceProfile::Paperwhite6, false)?),
            metrics,
        };
        fixture.seed()?;
        Ok(fixture)
    }

    fn store(&self) -> Result<Store> {
        Store::open(self.directory.path().join("library.sqlite"))
    }

    fn seed(&self) -> Result<()> {
        let store = self.store()?;
        let mut preferences = AppPreferences::for_device(false);
        preferences.popular = false;
        preferences.reader.font = "song".into();
        preferences.reader.font_size = 20;
        preferences.reader.indent = 0;
        preferences.reader.justify = false;
        preferences.article.font = "song".into();
        preferences.article.font_size = 20;
        preferences.download_article_images = false;
        store.set_setting("preferences", &preferences)?;
        store.upsert_book(&Book {
            source: "local".into(),
            total_chapters: READER_CHAPTERS,
            on_shelf: true,
            ..Book::new(BOOK, "Retained local Reader")
        })?;
        for index in 0..READER_CHAPTERS {
            store.upsert_chapter(&Chapter {
                id: reader_uid(index),
                book_id: BOOK.into(),
                index,
                title: format!("Reader chapter {index}"),
                body: Some(reader_body(index)),
                cached: true,
                content_status: ContentStatus::Full,
                ..Default::default()
            })?;
        }
        store.save_annotation(&Annotation {
            id: "retained-resume-note".into(),
            book_id: BOOK.into(),
            chapter_index: RESUME_CHAPTER,
            start: 0,
            end: 2,
            quote: "A\u{1f642}".into(),
            note: "Retained local note".into(),
            created_at: 1,
        })?;
        store.save_bookmark(&Bookmark {
            id: "retained-resume-bookmark".into(),
            book_id: BOOK.into(),
            chapter_index: RESUME_CHAPTER,
            char_offset: 1,
            label: "Retained bookmark".into(),
            created_at: 1,
        })?;
        let articles = (0..ARTICLES)
            .map(|index| MpArticle {
                id: article_id(index),
                account_id: ACCOUNT.into(),
                title: format!("Article {index:03}"),
                author: "Fixture author".into(),
                published_at: 1,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        store.set_setting(
            "mp.accounts",
            &vec![MpAccount {
                id: ACCOUNT.into(),
                name: "Fixture publication".into(),
                ..Default::default()
            }],
        )?;
        store.set_setting(
            "mp.articles",
            &BTreeMap::from([(ACCOUNT.to_owned(), articles)]),
        )?;
        for index in 0..ARTICLES {
            let book_id = article_book(index);
            store.upsert_book(&Book {
                source: "article".into(),
                on_shelf: false,
                total_chapters: 1,
                ..Book::new(&book_id, &format!("Article {index:03}"))
            })?;
            // Display presence must remain independent of a full-content flag.
            store.upsert_chapter(&Chapter {
                id: article_id(index),
                book_id,
                index: 0,
                body: (index != 1 && index != 2).then(|| article_body(index)),
                content_status: ContentStatus::Preview,
                ..Default::default()
            })?;
        }
        // A legacy Some(empty TEXT) still counts as present; None does not.
        let database = rusqlite::Connection::open(self.directory.path().join("library.sqlite"))?;
        database.execute(
            "UPDATE chapters SET body='' WHERE book_id=?1 AND chapter_index=0",
            [article_book(1)],
        )?;
        Ok(())
    }

    fn open(&self, screen: &str) -> Result<Instance> {
        let ui = AppWindow::new()?;
        ui.global::<Theme>()
            .set_ui_font(caret_test_font::FAMILY.into());
        ui.global::<Theme>()
            .set_serif_font(caret_test_font::FAMILY.into());
        crate::platform::configure_window(ui.window(), &self.platform)?;
        let app = App::attach(
            &ui,
            AppOptions {
                data_dir: self.directory.path().to_path_buf(),
                profile: DeviceProfile::Paperwhite6,
                demo: false,
                offline: true,
                initial_screen: screen.into(),
                fonts: vec![FontRecord {
                    id: "song".into(),
                    label: "Original display metadata font".into(),
                    family: caret_test_font::FAMILY.into(),
                    metrics: self.metrics.clone(),
                    weights: BTreeMap::from([(400, self.metrics.clone())]),
                    available: true,
                }],
            },
            self.platform.clone(),
            self.runtime.as_ref().unwrap(),
        )?;
        assert!(app.borrow().force_offline && !app.borrow().online);
        Ok(Instance { ui, app })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }
}

fn close(instance: Instance) -> Result<()> {
    instance.app.borrow_mut().terminal_shutdown()?;
    let weak = Rc::downgrade(&instance.app);
    drop(instance.app);
    assert!(weak.upgrade().is_none());
    let weak = instance.ui.as_weak();
    instance.ui.hide()?;
    drop(instance.ui);
    slint::platform::update_timers_and_animations();
    assert!(weak.upgrade().is_none());
    Ok(())
}

fn assert_notes(store: &Store, chapter: u32) -> Result<()> {
    let notes = store.annotations(BOOK)?;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].id, "retained-resume-note");
    assert_eq!(notes[0].chapter_index, chapter);
    assert_eq!(notes[0].quote, "A\u{1f642}");
    assert_eq!(notes[0].note, "Retained local note");
    let bookmarks = store.bookmarks(BOOK)?;
    assert_eq!(bookmarks.len(), 1);
    assert_eq!(bookmarks[0].chapter_index, chapter);
    assert_eq!(bookmarks[0].char_offset, 1);
    Ok(())
}

fn cold_resume() -> Result<()> {
    let fixture = Fixture::new()?;
    let original = fixture.open("library")?;
    let (anchor, confirmed) = {
        let mut state = original.app.borrow_mut();
        state.load_local_reader(BOOK, RESUME_CHAPTER, Some(0))?;
        let paginator = state.reader.paginator.as_mut().unwrap();
        assert!(paginator.pages.len() > 1);
        paginator.set_page(1);
        let anchor = paginator.current_page().start;
        state.save_reader_session()?;
        (anchor, state.reader.position_confirmed)
    };
    close(original)?;
    let store = fixture.store()?;
    let mut catalog = store.chapter_metadata(BOOK)?;
    catalog.swap(0, RESUME_CHAPTER as usize);
    for (index, chapter) in catalog.iter_mut().enumerate() {
        chapter.index = index as u32;
    }
    store.replace_catalog(BOOK, &catalog)?;
    drop(store);
    // This is a new App and Store, consuming a checkpoint saved by the prior
    // App. Reordering must resolve the UID without loading any other body.
    let resumed = fixture.open("resume")?;
    {
        let state = resumed.app.borrow();
        assert_eq!(state.route, Route::Reader(BOOK.into(), 0));
        assert_eq!(state.reader.body, reader_body(RESUME_CHAPTER));
        assert_eq!(state.reader.chapter_index, 0);
        assert_eq!(
            state
                .reader
                .paginator
                .as_ref()
                .unwrap()
                .current_page()
                .start,
            anchor
        );
        assert_eq!(state.reader.position_confirmed, confirmed);
        assert_eq!(state.reader.selection, None);
        assert!(state.reader.draft.query.is_empty());
        assert_eq!(state.jobs.session(), Some(session()));
        assert_eq!(state.store.materialized_chapter_bytes(), 0);
        let (reads, bytes) = state.store.chapter_body_materialization_stats(BOOK, 0);
        assert!(reads > 0);
        assert_eq!(bytes, reads * reader_body(RESUME_CHAPTER).len());
        assert!(
            (1..READER_CHAPTERS)
                .all(|index| state.store.chapter_body_materialization_stats(BOOK, index) == (0, 0))
        );
        assert_notes(&state.store, 0)?;
    }
    close(resumed)
}

fn cold_rejections() -> Result<()> {
    let fixture = Fixture::new()?;
    let original = fixture.open("library")?;
    let saved = {
        let mut state = original.app.borrow_mut();
        state.load_local_reader(BOOK, RESUME_CHAPTER, Some(0))?;
        state.save_reader_session()?;
        state
            .store
            .setting::<serde_json::Value>(RESUME_KEY)?
            .unwrap()
    };
    close(original)?;
    let store = fixture.store()?;
    store.upsert_chapter(&Chapter {
        id: reader_uid(RESUME_CHAPTER),
        book_id: BOOK.into(),
        index: READER_CHAPTERS,
        body: Some("Duplicate UID payload".into()),
        ..Default::default()
    })?;
    drop(store);
    let ambiguous = fixture.open("resume")?;
    {
        let state = ambiguous.app.borrow();
        assert_eq!(state.route, Route::Library);
        assert!(state.reader.body.is_empty());
        assert_eq!(state.jobs.session(), Some(session()));
        assert_eq!(state.store.chapter_body_read_count(), 0);
        assert_eq!(state.store.materialized_chapter_bytes(), 0);
        assert!(
            (0..=READER_CHAPTERS)
                .all(|index| state.store.chapter_body_materialization_stats(BOOK, index) == (0, 0))
        );
        assert_eq!(
            state.store.setting::<serde_json::Value>(RESUME_KEY)?,
            Some(saved.clone())
        );
        assert_notes(&state.store, RESUME_CHAPTER)?;
    }
    close(ambiguous)?;
    let database = rusqlite::Connection::open(fixture.directory.path().join("library.sqlite"))?;
    database.execute(
        "DELETE FROM chapters WHERE book_id=?1 AND chapter_index=?2",
        rusqlite::params![BOOK, READER_CHAPTERS],
    )?;
    drop(database);
    let changed_body = format!(
        "{}Changed after the saved checkpoint.",
        reader_body(RESUME_CHAPTER)
    );
    let store = fixture.store()?;
    store.upsert_chapter(&Chapter {
        id: reader_uid(RESUME_CHAPTER),
        book_id: BOOK.into(),
        index: RESUME_CHAPTER,
        body: Some(changed_body.clone()),
        ..Default::default()
    })?;
    store.set_setting(RESUME_KEY, &saved)?;
    drop(store);
    let changed = fixture.open("resume")?;
    {
        let state = changed.app.borrow();
        assert_eq!(state.route, Route::Library);
        assert!(state.reader.body.is_empty());
        assert_eq!(state.store.chapter_body_read_count(), 1);
        assert_eq!(
            state
                .store
                .chapter_body_materialization_stats(BOOK, RESUME_CHAPTER),
            (1, changed_body.len())
        );
        assert!(
            (0..READER_CHAPTERS)
                .filter(|index| *index != RESUME_CHAPTER)
                .all(|index| state.store.chapter_body_materialization_stats(BOOK, index) == (0, 0))
        );
        assert_eq!(state.store.materialized_chapter_bytes(), 0);
        assert_eq!(state.jobs.session(), Some(session()));
        assert_notes(&state.store, RESUME_CHAPTER)?;
    }
    close(changed)?;
    let store = fixture.store()?;
    assert_eq!(
        store.chapter_body(BOOK, RESUME_CHAPTER)?,
        Some(changed_body)
    );
    store.set_setting(RESUME_KEY, &saved)?;
    drop(store);
    let mut other = session();
    other.user_vid = "synthetic-other-display-account".into();
    other
        .cookies
        .insert("wr_vid".into(), other.user_vid.clone());
    SessionFile::new(fixture.directory.path()).save(&other)?;
    let another_account = fixture.open("resume")?;
    {
        let state = another_account.app.borrow();
        assert_eq!(state.route, Route::Library);
        assert!(state.reader.body.is_empty());
        assert_eq!(state.jobs.session(), Some(other));
        assert_eq!(state.store.chapter_body_read_count(), 0);
        assert_eq!(state.store.materialized_chapter_bytes(), 0);
        assert_notes(&state.store, RESUME_CHAPTER)?;
    }
    close(another_account)?;
    // The new 4 KiB scalar helper must not weaken the existing checkpoint's
    // 2 KiB identifier guard. Both saved fields are rejected before body I/O.
    SessionFile::new(fixture.directory.path()).save(&session())?;
    for field in ["book_id", "chapter_id"] {
        let mut oversized = saved.clone();
        oversized[field] = serde_json::Value::String("x".repeat(2049));
        let store = fixture.store()?;
        store.set_setting(RESUME_KEY, &oversized)?;
        drop(store);
        let rejected = fixture.open("resume")?;
        {
            let state = rejected.app.borrow();
            assert_eq!(state.route, Route::Library);
            assert_eq!(state.store.chapter_body_read_count(), 0);
            assert_eq!(state.store.materialized_chapter_bytes(), 0);
            assert_eq!(state.jobs.session(), Some(session()));
            assert_notes(&state.store, RESUME_CHAPTER)?;
        }
        close(rejected)?;
    }
    Ok(())
}

struct RetainedReader {
    body: String,
    selection: Option<(u32, u32)>,
    draft: String,
    account: Option<source::AuthSession>,
    notes: serde_json::Value,
}

impl RetainedReader {
    fn capture(state: &mut App) -> Result<Self> {
        state.reader.selection = Some((0, 2));
        state
            .reader
            .draft
            .set_query("Unsaved Reader draft during article rendering");
        Ok(Self {
            body: state.reader.body.clone(),
            selection: state.reader.selection,
            draft: state.reader.draft.query.clone(),
            account: state.jobs.session(),
            notes: serde_json::to_value(state.store.annotations(BOOK)?)?,
        })
    }

    fn assert_unchanged(&self, state: &App) -> Result<()> {
        assert_eq!(state.reader.book_id, BOOK);
        assert_eq!(state.reader.chapter_index, 0);
        assert_eq!(state.reader.body, self.body);
        assert_eq!(state.reader.selection, self.selection);
        assert_eq!(state.reader.draft.query, self.draft);
        assert_eq!(state.jobs.session(), self.account);
        assert_eq!(
            serde_json::to_value(state.store.annotations(BOOK)?)?,
            self.notes
        );
        Ok(())
    }
}

fn cached_summary() -> String {
    i18n::format(
        "articles_summary",
        &[
            ("count", ARTICLES.to_string()),
            ("cached", (ARTICLES - 1).to_string()),
        ],
    )
}

fn assert_presence_rows(rows: ModelRc<ReaderEntry>) {
    assert!(rows.row_count() > 0);
    let absent = article_id(2);
    for row in rows.iter() {
        assert_eq!(row.cached, row.id.as_str() != absent.as_str());
    }
}

fn publication_display() -> Result<()> {
    let fixture = Fixture::new()?;
    let instance = fixture.open("reader")?;
    {
        let mut state = instance.app.borrow_mut();
        let retained = RetainedReader::capture(&mut state)?;
        state.navigate(Route::Publications, true)?;
        let reads = state.store.chapter_body_read_count();
        state.dispatch("auxiliary", "mp.account", ACCOUNT)?;
        assert_eq!(
            instance
                .ui
                .global::<AuxiliaryModel>()
                .get_publications_subtitle(),
            cached_summary()
        );
        assert!(
            instance
                .ui
                .global::<AuxiliaryModel>()
                .get_publications_page_count()
                > 1
        );
        let rows = instance
            .ui
            .global::<AuxiliaryModel>()
            .get_publications_rows();
        let visible = rows.row_count();
        assert!(visible >= 3 && visible < ARTICLES);
        assert_eq!(rows.row_data(0).unwrap().id, article_id(0));
        assert!(rows.row_data(1).unwrap().cached);
        assert!(!rows.row_data(2).unwrap().cached);
        let absent = article_id(2);
        for _ in 0..8 {
            state.render()?;
            for row in instance
                .ui
                .global::<AuxiliaryModel>()
                .get_publications_rows()
                .iter()
            {
                assert_eq!(row.cached, row.id.as_str() != absent.as_str());
            }
            assert_eq!(state.store.chapter_body_read_count(), reads);
            retained.assert_unchanged(&state)?;
        }
        state.dispatch("auxiliary", "mp.page", "1")?;
        assert_eq!(
            instance
                .ui
                .global::<AuxiliaryModel>()
                .get_publications_page(),
            2
        );
        assert_eq!(
            instance
                .ui
                .global::<AuxiliaryModel>()
                .get_publications_rows()
                .row_data(0)
                .unwrap()
                .id,
            article_id(visible)
        );
        assert_eq!(state.store.chapter_body_read_count(), reads);
        // Cached and legacy-empty acquisitions also use scalar presence. The
        // absent article remains offline and cannot start a transport producer.
        for index in [0, 1, 2] {
            state.request_article(&article_id(index))?;
        }
        assert_eq!(state.store.chapter_body_read_count(), reads);
        assert!(state.pending_articles.is_empty());
        assert!((0..ARTICLES).all(|index| {
            state
                .store
                .chapter_body_materialization_stats(&article_book(index), 0)
                == (0, 0)
        }));
        assert_eq!(state.store.materialized_chapter_bytes(), 0);
        retained.assert_unchanged(&state)?;
    }
    close(instance)
}

fn article_display() -> Result<()> {
    let fixture = Fixture::new()?;
    let instance = fixture.open("reader")?;
    {
        let mut state = instance.app.borrow_mut();
        let retained = RetainedReader::capture(&mut state)?;
        state.navigate(Route::Article(article_id(0)), true)?;
        assert_eq!(state.article.body, article_body(0));
        assert!(state.article.pages.len() > 1);
        let reads = state.store.chapter_body_read_count();
        let visible = state
            .store
            .chapter_body_materialization_stats(&article_book(0), 0);
        assert_eq!(visible, (1, article_body(0).len()));
        for _ in 0..8 {
            state.render()?;
            assert!(!instance.ui.global::<ArticleModel>().get_list_open());
            assert_eq!(
                instance.ui.global::<ArticleModel>().get_list_summary(),
                cached_summary()
            );
            assert_eq!(state.store.chapter_body_read_count(), reads);
            retained.assert_unchanged(&state)?;
        }
        state.dispatch("article", "article.list", "true")?;
        assert!(instance.ui.global::<ArticleModel>().get_list_open());
        let rows = instance.ui.global::<ArticleModel>().get_articles();
        assert_eq!(rows.row_count(), 8);
        assert_eq!(rows.row_data(0).unwrap().id, article_id(0));
        assert!(rows.row_data(1).unwrap().cached);
        assert!(!rows.row_data(2).unwrap().cached);
        assert_presence_rows(rows);
        state.dispatch("article", "article.list-page", "1")?;
        assert_eq!(instance.ui.global::<ArticleModel>().get_list_page(), 2);
        let rows = instance.ui.global::<ArticleModel>().get_articles();
        assert_eq!(rows.row_count(), ARTICLES - 8);
        assert_eq!(rows.row_data(0).unwrap().id, article_id(8));
        assert_presence_rows(rows);
        state.dispatch("article", "article.list", "false")?;
        assert!(!instance.ui.global::<ArticleModel>().get_list_open());
        state.dispatch("article", "article.turn", "1")?;
        assert_eq!(state.article.page, 1);
        for _ in 0..8 {
            state.render()?;
        }
        assert_eq!(state.article.body, article_body(0));
        assert_eq!(state.store.chapter_body_read_count(), reads);
        assert_eq!(
            state
                .store
                .chapter_body_materialization_stats(&article_book(0), 0),
            visible
        );
        assert!((1..ARTICLES).all(|index| {
            state
                .store
                .chapter_body_materialization_stats(&article_book(index), 0)
                == (0, 0)
        }));
        assert_eq!(state.store.materialized_chapter_bytes(), 0);
        retained.assert_unchanged(&state)?;
    }
    close(instance)
}

#[test]
fn cold_app_resume_loads_only_the_unique_reordered_chapter_body() -> Result<()> {
    isolated(
        "app::display_metadata_tests::cold_app_resume_loads_only_the_unique_reordered_chapter_body",
        cold_resume,
    )
}

#[test]
fn cold_app_resume_rejects_ambiguity_changed_body_account_and_oversized_ids() -> Result<()> {
    isolated(
        "app::display_metadata_tests::cold_app_resume_rejects_ambiguity_changed_body_account_and_oversized_ids",
        cold_rejections,
    )
}

#[test]
fn publication_render_paging_and_acquisition_use_only_body_presence() -> Result<()> {
    isolated(
        "app::display_metadata_tests::publication_render_paging_and_acquisition_use_only_body_presence",
        publication_display,
    )
}

#[test]
fn article_hidden_list_and_page_renders_never_reload_unshown_bodies() -> Result<()> {
    isolated(
        "app::display_metadata_tests::article_hidden_list_and_page_renders_never_reload_unshown_bodies",
        article_display,
    )
}
