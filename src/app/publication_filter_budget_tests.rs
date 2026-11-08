//! Persistent publication caches drive genuine offline UI filtering and denial.

use super::auxiliary::{
    PublicationFilterLimit, PublicationFilterLimits, PublicationProjectionStats,
    with_publication_filter_test_limits,
};
use super::underline_projection_tests::isolated;
use super::*;
use slint::{
    Model,
    platform::{
        Platform as UiPlatform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};

const OWNER: &str = "MP_WXS_filter_owner";
const BOOK: &str = "filter-retained-reader";

struct FilterUi;

impl UiPlatform for FilterUi {
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
    fn new(name: &str, articles: Vec<MpArticle>) -> Result<Self> {
        slint::platform::set_platform(Box::new(FilterUi))?;
        let directory = tempfile::tempdir()?;
        let account = source::AuthSession {
            api_key: "synthetic-filter-key".into(),
            user_vid: "synthetic-filter-account".into(),
            cookies: BTreeMap::from([
                ("wr_vid".into(), "synthetic-filter-account".into()),
                ("wr_skey".into(), "synthetic-filter-session".into()),
            ]),
            ..Default::default()
        };
        SessionFile::new(directory.path()).save(&account)?;
        {
            let store = Store::open(directory.path().join("library.sqlite"))?;
            let mut preferences = AppPreferences::for_device(false);
            preferences.popular = false;
            preferences.pull_on_open = false;
            preferences.push_on_close = false;
            preferences.report_time = false;
            store.set_setting("preferences", &preferences)?;
            store.upsert_book(&Book {
                source: "local".into(),
                total_chapters: 1,
                on_shelf: true,
                ..Book::new(BOOK, "Retained filter Reader")
            })?;
            store.upsert_chapter(&Chapter {
                id: "filter-reader-chapter".into(),
                book_id: BOOK.into(),
                body: Some("Retained reader.".into()),
                cached: true,
                content_status: ContentStatus::Full,
                ..Default::default()
            })?;
            store.save_annotation(&Annotation {
                id: "filter-note".into(),
                book_id: BOOK.into(),
                start: 0,
                end: 8,
                quote: "Retained".into(),
                note: "Keep local note".into(),
                created_at: 1,
                ..Default::default()
            })?;
            store.save_bookmark(&Bookmark {
                id: "filter-bookmark".into(),
                book_id: BOOK.into(),
                char_offset: 1,
                label: "Keep local bookmark".into(),
                ..Default::default()
            })?;
            store.set_setting(
                "mp.accounts",
                &vec![MpAccount {
                    id: OWNER.into(),
                    name: name.into(),
                    updated_at: 100,
                    ..Default::default()
                }],
            )?;
            store.set_setting(
                "mp.articles",
                &BTreeMap::from([(OWNER.to_owned(), articles)]),
            )?;
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
                initial_screen: "library".into(),
                fonts: vec![FontRecord {
                    id: "song".into(),
                    label: "Filter fixture font".into(),
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
            assert_eq!(state.mp_accounts.len(), 1);
            assert_eq!(state.mp_articles.len(), 1);
            assert!(state.force_offline && !state.online && state.jobs.configured());
            state.load_local_reader(BOOK, 0, Some(0))?;
            state.reader.selection = Some((0, 8));
            assert!(state.reader.draft.set_query("Keep filter draft"));
        }
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
    fn show(&self, article_list: bool) {
        self.ui()
            .global::<ShellModel>()
            .invoke_event("tab".into(), "mp".into());
        if article_list {
            self.event("mp.account", OWNER);
        }
    }
    fn event(&self, action: &str, value: &str) {
        self.ui()
            .global::<AuxiliaryModel>()
            .invoke_event(action.into(), value.into());
    }
    fn reset_stats(&self) {
        self.app()
            .publication_stats
            .set(PublicationProjectionStats::default());
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

fn article(id: &str, title: &str, author: &str, published_at: i64) -> MpArticle {
    MpArticle {
        id: id.into(),
        account_id: OWNER.into(),
        title: title.into(),
        author: author.into(),
        published_at,
        ..Default::default()
    }
}

#[derive(Debug, PartialEq)]
struct View {
    title: String,
    subtitle: String,
    query: String,
    sort: String,
    owner: String,
    page: i32,
    pages: i32,
    busy: bool,
    status: String,
    rows: Vec<(String, String, String, String, String, i32, bool)>,
}

fn view(fixture: &Fixture) -> View {
    let global = fixture.ui().global::<AuxiliaryModel>();
    View {
        title: global.get_publications_title().to_string(),
        subtitle: global.get_publications_subtitle().to_string(),
        query: global.get_publications_query().to_string(),
        sort: global.get_publications_sort().to_string(),
        owner: global.get_publications_account_id().to_string(),
        page: global.get_publications_page(),
        pages: global.get_publications_page_count(),
        busy: global.get_publications_busy(),
        status: global.get_publications_status().to_string(),
        rows: global
            .get_publications_rows()
            .iter()
            .map(|row| {
                (
                    row.id.to_string(),
                    row.title.to_string(),
                    row.subtitle.to_string(),
                    row.meta.to_string(),
                    row.initial.to_string(),
                    row.unread,
                    row.cached,
                )
            })
            .collect(),
    }
}

#[derive(Debug, PartialEq)]
struct Retained {
    raw_accounts: serde_json::Value,
    raw_articles: serde_json::Value,
    cache: Vec<(String, String)>,
    prefs: serde_json::Value,
    owner: Option<String>,
    body: String,
    draft: String,
    selection: Option<(u32, u32)>,
    account: Option<source::AuthSession>,
    history: Vec<Route>,
}

fn retained(state: &App) -> Result<Retained> {
    let database = rusqlite::Connection::open(state.data_dir.join("library.sqlite"))?;
    let mut statement = database.prepare(
        "SELECT key,value FROM settings WHERE key IN ('mp.accounts','mp.articles') ORDER BY key",
    )?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(Retained {
        raw_accounts: serde_json::to_value(&state.mp_accounts)?,
        raw_articles: serde_json::to_value(&state.mp_articles)?,
        cache: rows.collect::<rusqlite::Result<Vec<_>>>()?,
        prefs: serde_json::to_value(&state.prefs)?,
        owner: state.mp_account.clone(),
        body: state.reader.body.clone(),
        draft: state.reader.draft.query.clone(),
        selection: state.reader.selection,
        account: state.jobs.session(),
        history: state.history.clone(),
    })
}

fn assert_retained(state: &App, previous: &Retained, query: &str) -> Result<()> {
    assert_eq!(&retained(state)?, previous);
    assert_eq!(state.mp_query, query);
    assert!(!state.closing.get());
    assert_eq!(
        state
            .store
            .annotation_by_id(BOOK, "filter-note")?
            .unwrap()
            .note,
        "Keep local note"
    );
    assert_eq!(state.store.bookmarks(BOOK)?[0].char_offset, 1);
    Ok(())
}

fn small_article_scratch() -> usize {
    // "AB CD": five input scalars, each reserving four UTF-8 output bytes.
    16 + 40 + 2 * std::mem::size_of::<String>()
}

fn exact_and_one_less() -> Result<()> {
    let fixture = Fixture::new("Publisher", vec![article("small", "AB", "CD", 2)])?;
    fixture.show(true);
    let raw = retained(&fixture.app())?;
    for limits in [
        PublicationFilterLimits {
            work_bytes: 71,
            scratch_bytes: small_article_scratch(),
        },
        PublicationFilterLimits {
            work_bytes: 72,
            scratch_bytes: small_article_scratch() - 1,
        },
    ] {
        fixture.event("mp.edit-query", "");
        let previous = view(&fixture);
        fixture.reset_stats();
        with_publication_filter_test_limits(limits, || fixture.event("mp.edit-query", "z"));
        let state = fixture.app();
        let stats = state.publication_stats.get();
        assert_eq!(stats.filter_query_transforms, 1);
        assert_eq!(stats.filter_article_transforms, 0);
        assert_eq!(stats.filter_join_copies, 0);
        assert_eq!(stats.row_source_bytes, 0);
        assert_eq!(stats.filter_rejections, 1);
        assert_eq!(stats.rejections, 1);
        assert_eq!(view(&fixture), previous);
        assert_eq!(
            fixture.ui().global::<ShellModel>().get_toast(),
            tr("api_error")
        );
        assert_retained(&state, &raw, "z")?;
    }
    fixture.event("mp.edit-query", "");
    fixture.reset_stats();
    with_publication_filter_test_limits(
        PublicationFilterLimits {
            work_bytes: 72,
            scratch_bytes: small_article_scratch(),
        },
        || fixture.event("mp.edit-query", "z"),
    );
    {
        let state = fixture.app();
        let stats = state.publication_stats.get();
        assert_eq!(stats.filter_work_bytes, 72);
        assert_eq!(stats.filter_preflight_chars, 6);
        assert_eq!(stats.filter_article_transforms, 1);
        assert_eq!(stats.filter_join_copies, 1);
        assert_eq!(stats.filter_join_source_bytes, 5);
        assert_eq!(stats.filter_lowercase_result_bytes, 6);
        assert_eq!(stats.filter_peak_scratch_bytes, small_article_scratch());
        assert!(stats.filter_actual_peak_scratch_bytes <= stats.filter_peak_scratch_bytes);
        assert_eq!(stats.filter_rejections, 0);
        assert_eq!(view(&fixture).rows.len(), 0);
        assert_retained(&state, &raw, "z")?;
    }
    fixture.finish()
}

#[test]
fn exact_matcher_work_and_scratch_admit_while_one_less_copies_no_article() -> Result<()> {
    isolated(
        "app::publication_filter_budget_tests::exact_matcher_work_and_scratch_admit_while_one_less_copies_no_article",
        exact_and_one_less,
    )
}

fn filtered_out_tail() -> Result<()> {
    let fixture = Fixture::new(
        "Publisher",
        vec![
            article("matched-first", "AZ", "Q", 2),
            article("filtered-tail", &"x".repeat(1024), "None", 1),
        ],
    )?;
    fixture.show(true);
    let previous = view(&fixture);
    let raw = retained(&fixture.app())?;
    fixture.reset_stats();
    with_publication_filter_test_limits(
        PublicationFilterLimits {
            work_bytes: 100,
            ..Default::default()
        },
        || fixture.event("mp.edit-query", "z"),
    );
    {
        let state = fixture.app();
        let stats = state.publication_stats.get();
        assert_eq!(stats.article_visits, 2);
        assert_eq!(stats.filter_work_bytes, 60);
        assert_eq!(stats.filter_preflight_chars, 5);
        assert_eq!(stats.filter_article_transforms, 1);
        assert_eq!(stats.filter_join_source_bytes, 4);
        assert_eq!(stats.presence_reads, 1);
        assert_eq!(stats.unread_reads, 1);
        assert_eq!(stats.row_source_bytes, 0);
        assert_eq!(stats.filter_rejections, 1);
        assert_eq!(view(&fixture), previous);
        assert_retained(&state, &raw, "z")?;
    }
    fixture.finish()
}

#[test]
fn filtered_out_long_tail_is_prepaid_and_late_denial_discards_partial_rows() -> Result<()> {
    isolated(
        "app::publication_filter_budget_tests::filtered_out_long_tail_is_prepaid_and_late_denial_discards_partial_rows",
        filtered_out_tail,
    )
}

fn unicode_separator() -> Result<()> {
    let title = "\u{130} \u{39f}\u{3a3}";
    let fixture = Fixture::new("Publisher", vec![article("unicode", title, "A", 2)])?;
    fixture.show(true);
    let raw = retained(&fixture.app())?;
    fixture.reset_stats();
    fixture.event("mp.edit-query", "\u{3bf}\u{3c2} a");
    {
        let state = fixture.app();
        let shown = view(&fixture);
        assert_eq!(shown.rows.len(), 1);
        assert_eq!(shown.rows[0].0, "unicode");
        assert_eq!(shown.rows[0].1, title);
        let stats = state.publication_stats.get();
        assert_eq!(stats.filter_article_transforms, 1);
        assert_eq!(stats.filter_join_source_bytes, 9);
        assert_eq!(stats.filter_lowercase_result_bytes, 16);
        assert!(stats.filter_actual_peak_scratch_bytes <= stats.filter_peak_scratch_bytes);
        assert!(stats.row_source_bytes > 0);
        assert_retained(&state, &raw, "\u{3bf}\u{3c2} a")?;
    }
    fixture.event("mp.edit-query", "\u{39f}\u{3a3} A");
    assert_eq!(view(&fixture).rows[0].0, "unicode");
    fixture.finish()
}

#[test]
fn whole_joined_unicode_lowercase_and_cross_separator_needles_keep_their_semantics() -> Result<()> {
    isolated(
        "app::publication_filter_budget_tests::whole_joined_unicode_lowercase_and_cross_separator_needles_keep_their_semantics",
        unicode_separator,
    )
}

fn owner_short_circuit() -> Result<()> {
    let fixture = Fixture::new(
        "Owner z",
        vec![
            article("first", &"x".repeat(1024), "A", 1),
            article("second", &"y".repeat(1024), "B", 2),
        ],
    )?;
    fixture.show(false);
    let raw = retained(&fixture.app())?;
    fixture.reset_stats();
    with_publication_filter_test_limits(
        PublicationFilterLimits {
            work_bytes: 96,
            scratch_bytes: 2 * 28 + std::mem::size_of::<String>(),
        },
        || fixture.event("mp.edit-query", "z"),
    );
    {
        let state = fixture.app();
        let stats = state.publication_stats.get();
        assert_eq!(stats.filter_work_bytes, 96);
        assert_eq!(stats.filter_name_transforms, 1);
        assert_eq!(stats.filter_article_transforms, 0);
        assert_eq!(stats.filter_join_copies, 0);
        assert_eq!(stats.article_visits, 2);
        assert_eq!(stats.unread_reads, 2);
        assert_eq!(view(&fixture).rows.len(), 1);
        assert!(stats.row_source_bytes > 1024);
        assert_retained(&state, &raw, "z")?;
    }
    fixture.reset_stats();
    with_publication_filter_test_limits(
        PublicationFilterLimits {
            work_bytes: 0,
            scratch_bytes: 0,
        },
        || fixture.event("mp.edit-query", ""),
    );
    {
        let state = fixture.app();
        let stats = state.publication_stats.get();
        assert_eq!(stats.filter_query_transforms, 0);
        assert_eq!(stats.filter_name_transforms, 0);
        assert_eq!(stats.filter_article_transforms, 0);
        assert_eq!(stats.filter_work_bytes, 0);
        assert_eq!(stats.filter_preflight_chars, 0);
        assert_eq!(stats.filter_rejections, 0);
        assert_eq!(view(&fixture).rows.len(), 1);
        assert_retained(&state, &raw, "")?;
    }
    fixture.finish()
}

#[test]
fn matched_owner_and_empty_query_skip_article_transforms_and_their_fees() -> Result<()> {
    isolated(
        "app::publication_filter_budget_tests::matched_owner_and_empty_query_skip_article_transforms_and_their_fees",
        owner_short_circuit,
    )
}

fn first_article_short_circuit() -> Result<()> {
    let fixture = Fixture::new(
        "Owner",
        vec![
            article("first", "Z", "A", 1),
            article("skipped-tail", &"y".repeat(1024), "B", 2),
        ],
    )?;
    fixture.show(false);
    let raw = retained(&fixture.app())?;
    fixture.reset_stats();
    with_publication_filter_test_limits(
        PublicationFilterLimits {
            work_bytes: 108,
            ..Default::default()
        },
        || fixture.event("mp.edit-query", "z"),
    );
    {
        let state = fixture.app();
        let stats = state.publication_stats.get();
        assert_eq!(stats.filter_work_bytes, 108);
        assert_eq!(stats.filter_name_transforms, 1);
        assert_eq!(stats.filter_article_transforms, 1);
        assert_eq!(stats.filter_join_source_bytes, 3);
        assert_eq!(stats.article_visits, 2);
        assert_eq!(stats.unread_reads, 2);
        assert_eq!(view(&fixture).rows.len(), 1);
        assert_retained(&state, &raw, "z")?;
    }
    fixture.finish()
}

#[test]
fn overview_first_article_match_skips_later_matchers_without_losing_summary_visits() -> Result<()> {
    isolated(
        "app::publication_filter_budget_tests::overview_first_article_match_skips_later_matchers_without_losing_summary_visits",
        first_article_short_circuit,
    )
}

fn focus_and_inline() -> Result<()> {
    let fixture = Fixture::new("Publisher", vec![article("joined", "AB", "CD", 2)])?;
    fixture.show(false);
    let previous = view(&fixture);
    let raw = retained(&fixture.app())?;
    fixture.event("mp.focus", "");
    assert_eq!(fixture.app().route, Route::Search);
    assert_eq!(fixture.app().search_purpose, "publications");
    fixture
        .ui()
        .global::<SearchModel>()
        .invoke_event("edit-query".into(), "b c".into());
    fixture.reset_stats();
    with_publication_filter_test_limits(
        PublicationFilterLimits {
            work_bytes: 50,
            ..Default::default()
        },
        || {
            fixture
                .ui()
                .global::<SearchModel>()
                .invoke_event("key".into(), "submit".into())
        },
    );
    {
        let state = fixture.app();
        assert_eq!(state.route, Route::Publications);
        assert_eq!(view(&fixture), previous);
        assert_eq!(state.publication_stats.get().filter_join_copies, 0);
        assert_eq!(state.publication_stats.get().filter_rejections, 1);
        assert_eq!(state.publication_stats.get().row_source_bytes, 0);
        assert_retained(&state, &raw, "b c")?;
    }
    fixture.event("mp.edit-query", "");
    let previous = view(&fixture);
    fixture.reset_stats();
    with_publication_filter_test_limits(
        PublicationFilterLimits {
            work_bytes: 50,
            ..Default::default()
        },
        || fixture.event("mp.edit-query", "b c"),
    );
    {
        let state = fixture.app();
        assert_eq!(view(&fixture), previous);
        assert_eq!(state.publication_stats.get().filter_join_copies, 0);
        assert_eq!(state.publication_stats.get().filter_rejections, 1);
        assert_retained(&state, &raw, "b c")?;
    }
    fixture.finish()
}

#[test]
fn persistent_cache_focus_search_and_inline_query_denials_preserve_raw_and_old_model() -> Result<()>
{
    isolated(
        "app::publication_filter_budget_tests::persistent_cache_focus_search_and_inline_query_denials_preserve_raw_and_old_model",
        focus_and_inline,
    )
}

fn cumulative_and_peak() -> Result<()> {
    let fixture = Fixture::new(
        "Publisher",
        vec![
            article("first", "AB", "CD", 2),
            article("second", "AB", "CD", 1),
        ],
    )?;
    fixture.show(true);
    let previous = view(&fixture);
    let raw = retained(&fixture.app())?;
    fixture.reset_stats();
    with_publication_filter_test_limits(
        PublicationFilterLimits {
            work_bytes: 131,
            scratch_bytes: small_article_scratch(),
        },
        || fixture.event("mp.edit-query", "z"),
    );
    {
        let state = fixture.app();
        let stats = state.publication_stats.get();
        assert_eq!(stats.filter_join_copies, 1);
        assert_eq!(stats.filter_work_bytes, 124);
        assert_eq!(stats.filter_preflight_chars, 11);
        assert_eq!(stats.row_source_bytes, 0);
        assert_eq!(view(&fixture), previous);
        assert_retained(&state, &raw, "z")?;
    }
    fixture.event("mp.edit-query", "");
    fixture.reset_stats();
    with_publication_filter_test_limits(
        PublicationFilterLimits {
            work_bytes: 132,
            scratch_bytes: small_article_scratch(),
        },
        || fixture.event("mp.edit-query", "z"),
    );
    {
        let state = fixture.app();
        let stats = state.publication_stats.get();
        assert_eq!(stats.filter_work_bytes, 132);
        assert_eq!(stats.filter_join_copies, 2);
        assert_eq!(stats.filter_join_source_bytes, 10);
        assert_eq!(stats.filter_peak_scratch_bytes, small_article_scratch());
        assert!(stats.filter_actual_peak_scratch_bytes <= stats.filter_peak_scratch_bytes);
        assert_eq!(stats.filter_rejections, 0);
        assert_eq!(view(&fixture).rows.len(), 0);
        assert_retained(&state, &raw, "z")?;
    }
    fixture.finish()
}

#[test]
fn multiple_matchers_accumulate_work_but_take_the_maximum_single_temporary_peak() -> Result<()> {
    isolated(
        "app::publication_filter_budget_tests::multiple_matchers_accumulate_work_but_take_the_maximum_single_temporary_peak",
        cumulative_and_peak,
    )
}

fn typed_and_sql_errors() -> Result<()> {
    let fixture = Fixture::new("Publisher", vec![article("small", "AB", "CD", 2)])?;
    fixture.show(true);
    let previous = view(&fixture);
    let raw = retained(&fixture.app())?;
    {
        let mut state = fixture.app();
        state.auxiliary_action("mp.edit-query", "z")?;
        let error = with_publication_filter_test_limits(
            PublicationFilterLimits {
                work_bytes: 0,
                scratch_bytes: 0,
            },
            || state.render_publications_filter_inner_for_test(),
        )
        .unwrap_err();
        assert!(error.downcast_ref::<PublicationFilterLimit>().is_some());
        assert!(error.downcast_ref::<rusqlite::Error>().is_none());
        assert_eq!(view(&fixture), previous);
        assert_retained(&state, &raw, "z")?;

        state.auxiliary_action("mp.edit-query", "b")?;
        let database = rusqlite::Connection::open(state.data_dir.join("library.sqlite"))?;
        database.execute_batch("ALTER TABLE chapters RENAME TO filter_hidden_chapters")?;
        let error = state.render_publications().unwrap_err();
        assert!(error.downcast_ref::<rusqlite::Error>().is_some());
        assert!(error.downcast_ref::<PublicationFilterLimit>().is_none());
        database.execute_batch("ALTER TABLE filter_hidden_chapters RENAME TO chapters")?;
        assert_eq!(view(&fixture), previous);
        assert_retained(&state, &raw, "b")?;
    }
    fixture.finish()
}

#[test]
fn typed_filter_denial_and_real_sql_failure_keep_distinct_causes_and_old_projection() -> Result<()>
{
    isolated(
        "app::publication_filter_budget_tests::typed_filter_denial_and_real_sql_failure_keep_distinct_causes_and_old_projection",
        typed_and_sql_errors,
    )
}
