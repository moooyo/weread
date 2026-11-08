//! Genuine old publication history, visible Article output and checked style rejection.

use super::auxiliary::{
    ArticleProjectionLimit, ArticleProjectionLimits, article_projection_stats,
    reset_article_projection_stats, with_article_projection_limits,
};
use super::underline_projection_tests::{Fixture, isolated};
use super::*;
use crate::article::{StyleTestLimits, style_budget_tests::ObservationScope};
use crate::statistics;
use serde_json::json;
use slint::Model;

const OWNER: &str = "MP_WXS_article_projection";
const OTHER_OWNER: &str = "MP_WXS_article_projection_other";
const CURRENT: &str = "article-projection-00";
const ARTICLE_BODY: &str =
    "Before\nA retained Article paragraph with Unicode \u{4e2d}\u{1f642}.\nAfter";
const READER_BODY: &str = "A\u{1f600}B retained reader context.";

fn article_id(index: usize) -> String {
    format!("article-projection-{index:02}")
}
fn article_book(id: &str) -> String {
    format!("article:{id}")
}
fn article_html() -> String {
    "<div id='js_content'><p>Before</p><p style='color:blue'>A retained Article paragraph with Unicode \u{4e2d}\u{1f642}.</p><img src='https://example.invalid/held.png'><p>After</p></div>".into()
}

fn records(count: usize) -> Vec<MpArticle> {
    (0..count)
        .map(|index| MpArticle {
            id: article_id(index),
            account_id: OWNER.into(),
            title: if index == 1 {
                String::new()
            } else if index < 2 {
                "Current Article".into()
            } else {
                format!(
                    "Historical title {index} {}",
                    "\u{4e2d}\u{1f642}".repeat(256)
                )
            },
            author: format!("Unused author {index} {}", "a".repeat(256)),
            published_at: 1_700_000_000 - index as i64,
            original_url: format!("https://example.invalid/{index}/{}", "u".repeat(1024)),
            cover_url: format!("https://example.invalid/cover/{index}/{}", "c".repeat(256)),
        })
        .collect()
}

fn store_article(state: &App, id: &str, title: &str, body: Option<&str>, html: &str) -> Result<()> {
    state.store.upsert_book(&Book {
        source: "article".into(),
        on_shelf: false,
        intro: "Retained unrelated Book introduction. ".repeat(4096),
        total_chapters: 1,
        ..Book::new(&article_book(id), title)
    })?;
    state.store.upsert_chapter(&Chapter {
        id: id.into(),
        book_id: article_book(id),
        index: 0,
        title: title.into(),
        body: body.map(str::to_owned),
        content_status: ContentStatus::Preview,
        ..Default::default()
    })?;
    state
        .store
        .set_setting(&format!("article.html:{id}"), &html)
}

fn install_history(state: &mut App, count: usize) -> Result<()> {
    let accounts = vec![
        MpAccount {
            id: OWNER.into(),
            name: "First owner name".into(),
            ..Default::default()
        },
        MpAccount {
            id: OWNER.into(),
            name: "Ignored duplicate owner name".into(),
            ..Default::default()
        },
        MpAccount {
            id: OTHER_OWNER.into(),
            name: "Other owner name".into(),
            ..Default::default()
        },
    ];
    let raw = BTreeMap::from([
        (OWNER.to_owned(), records(count)),
        (
            OTHER_OWNER.to_owned(),
            vec![MpArticle {
                id: "other-article".into(),
                account_id: OTHER_OWNER.into(),
                title: "Other article".into(),
                ..Default::default()
            }],
        ),
    ]);
    // This is the durable settings producer/reader used by the App's legacy
    // MP cache. No result getter or projected row is substituted for the data.
    state.store.set_setting("mp.accounts", &accounts)?;
    state.store.set_setting("mp.articles", &raw)?;
    state.mp_accounts = state.store.setting("mp.accounts")?.unwrap();
    state.mp_articles = state.store.setting("mp.articles")?.unwrap();
    state.mp_account = Some(OWNER.into());
    state.prefs.download_article_images = false;
    store_article(
        state,
        CURRENT,
        "Current Article",
        Some(ARTICLE_BODY),
        &article_html(),
    )?;
    store_article(
        state,
        &article_id(1),
        "",
        Some("Will remain empty TEXT"),
        "",
    )?;
    store_article(state, &article_id(2), "Null body Article", None, "")?;
    store_article(
        state,
        "other-article",
        "Other article",
        Some("Other retained body"),
        "<p>Other retained body</p>",
    )?;
    let connection = rusqlite::Connection::open(state.data_dir.join("library.sqlite"))?;
    connection.execute(
        "UPDATE chapters SET body='' WHERE book_id=?1",
        [article_book(&article_id(1))],
    )?;
    Ok(())
}

fn fixture(profile: DeviceProfile) -> Result<Fixture> {
    let fixture = Fixture::new_for_profile(READER_BODY, Vec::new(), false, profile)?;
    install_history(&mut fixture.app(), 17)?;
    Ok(fixture)
}

fn row_snapshot(ui: &AppWindow) -> Vec<(String, String, String, bool)> {
    ui.global::<ArticleModel>()
        .get_articles()
        .iter()
        .map(|row| {
            (
                row.id.to_string(),
                row.title.to_string(),
                row.subtitle.to_string(),
                row.cached,
            )
        })
        .collect()
}

struct Snapshot {
    route: Route,
    history: Vec<Route>,
    article: (String, String, String, String, usize),
    pages: Vec<Vec<crate::article::PageBlock>>,
    loaded: BTreeMap<String, PathBuf>,
    images: Vec<crate::article::ArticleImage>,
    image_requests: Vec<Option<u64>>,
    image_ready: bool,
    image_work: bool,
    image_capacity: usize,
    pending: BTreeSet<String>,
    reader: (String, String, String, Option<(u32, u32)>),
    note_context: Option<usize>,
    thought_context: Option<usize>,
    epoch: u64,
    raw: serde_json::Value,
    ui: (
        String,
        String,
        String,
        String,
        bool,
        String,
        i32,
        i32,
        Vec<(String, String, String, bool)>,
    ),
}

impl Snapshot {
    fn capture(state: &App, ui: &AppWindow) -> Result<Self> {
        let global = ui.global::<ArticleModel>();
        Ok(Self {
            route: state.route.clone(),
            history: state.history.clone(),
            article: (
                state.article.id.clone(),
                state.article.account_id.clone(),
                state.article.body.clone(),
                state.article.html.clone(),
                state.article.page,
            ),
            pages: state.article.pages.clone(),
            loaded: state.article.loaded_images.clone(),
            images: state.article.image_work.images().to_vec(),
            image_requests: state
                .article
                .image_work
                .images()
                .iter()
                .map(|image| state.article.image_work.request_for(&image.id))
                .collect(),
            image_ready: state.article.image_work.is_ready_for(&state.article.id),
            image_work: state.article.image_work.has_work(),
            image_capacity: state.article.image_work.available_capacity(),
            pending: state.pending_article_images.clone(),
            reader: (
                state.reader.body.clone(),
                state.reader.draft.query.clone(),
                state.reader.draft.composition.clone(),
                state.reader.selection,
            ),
            note_context: state
                .reader
                .note_edit_context
                .as_ref()
                .map(|value| value as *const _ as usize),
            thought_context: state
                .reader
                .thought_context
                .as_ref()
                .map(|value| value as *const _ as usize),
            epoch: state.jobs.account_epoch(),
            raw: state.store.setting("mp.articles")?.unwrap(),
            ui: (
                global.get_title().to_string(),
                global.get_current_id().to_string(),
                global.get_meta().to_string(),
                global.get_next_title().to_string(),
                global.get_has_next(),
                global.get_list_summary().to_string(),
                global.get_page(),
                global.get_page_count(),
                row_snapshot(ui),
            ),
        })
    }

    fn assert_retained(&self, state: &App, ui: &AppWindow) -> Result<()> {
        assert_eq!(state.route, self.route);
        assert_eq!(state.history, self.history);
        assert_eq!(
            (
                state.article.id.clone(),
                state.article.account_id.clone(),
                state.article.body.clone(),
                state.article.html.clone(),
                state.article.page
            ),
            self.article
        );
        assert_eq!(state.article.pages, self.pages);
        assert_eq!(state.article.loaded_images, self.loaded);
        assert_eq!(state.article.image_work.images(), self.images.as_slice());
        assert_eq!(
            state
                .article
                .image_work
                .images()
                .iter()
                .map(|image| state.article.image_work.request_for(&image.id))
                .collect::<Vec<_>>(),
            self.image_requests
        );
        assert_eq!(
            state.article.image_work.is_ready_for(&state.article.id),
            self.image_ready
        );
        assert_eq!(state.article.image_work.has_work(), self.image_work);
        assert_eq!(
            state.article.image_work.available_capacity(),
            self.image_capacity
        );
        assert_eq!(state.pending_article_images, self.pending);
        assert_eq!(
            (
                state.reader.body.clone(),
                state.reader.draft.query.clone(),
                state.reader.draft.composition.clone(),
                state.reader.selection
            ),
            self.reader
        );
        assert_eq!(
            state
                .reader
                .note_edit_context
                .as_ref()
                .map(|value| value as *const _ as usize),
            self.note_context
        );
        assert_eq!(
            state
                .reader
                .thought_context
                .as_ref()
                .map(|value| value as *const _ as usize),
            self.thought_context
        );
        assert_eq!(state.jobs.account_epoch(), self.epoch);
        assert_eq!(
            state
                .store
                .setting::<serde_json::Value>("mp.articles")?
                .unwrap(),
            self.raw
        );
        let global = ui.global::<ArticleModel>();
        assert_eq!(
            (
                global.get_title().to_string(),
                global.get_current_id().to_string(),
                global.get_meta().to_string(),
                global.get_next_title().to_string(),
                global.get_has_next(),
                global.get_list_summary().to_string(),
                global.get_page(),
                global.get_page_count(),
                row_snapshot(ui)
            ),
            self.ui
        );
        Ok(())
    }
}

fn hidden_history() -> Result<()> {
    let fixture = fixture(DeviceProfile::Paperwhite6)?;
    {
        let mut state = fixture.app();
        state.navigate(Route::Article(CURRENT.into()), true)?;
        let raw = state
            .store
            .setting::<serde_json::Value>("mp.articles")?
            .unwrap();
        let body_reads = state.store.chapter_body_read_count();
        reset_article_projection_stats();
        state.render_article()?;
        let stats = article_projection_stats();
        assert_eq!(stats.owner_visits, 17);
        assert_eq!(stats.presence_reads, 17);
        assert_eq!(stats.rows, 0);
        assert_eq!(stats.row_source_bytes, 0);
        assert!(row_snapshot(fixture.ui()).is_empty());
        assert!(fixture.ui().global::<ArticleModel>().get_has_next());
        assert_eq!(
            fixture
                .ui()
                .global::<ArticleModel>()
                .get_next_title()
                .as_str(),
            ""
        );
        assert_eq!(
            fixture.ui().global::<ArticleModel>().get_account().as_str(),
            "First owner name"
        );
        assert_eq!(
            fixture
                .ui()
                .global::<ArticleModel>()
                .get_list_summary()
                .as_str(),
            i18n::format(
                "articles_summary",
                &[("count", "17".into()), ("cached", "2".into())]
            )
        );
        assert_eq!(state.store.chapter_body_read_count(), body_reads);
        assert_eq!(
            state
                .store
                .setting::<serde_json::Value>("mp.articles")?
                .unwrap(),
            raw
        );
    }
    fixture.finish()
}

fn visible_pages(profile: DeviceProfile) -> Result<()> {
    let fixture = fixture(profile)?;
    {
        let mut state = fixture.app();
        state.navigate(Route::Article(CURRENT.into()), true)?;
        let global = fixture.ui().global::<ArticleModel>();
        global.set_list_open(true);
        let size = if profile.is_scribe() { 11 } else { 8 };
        for page in 0..2 {
            global.set_list_page(page + 1);
            reset_article_projection_stats();
            let reads = state.store.chapter_body_read_count();
            state.render_article()?;
            let rows = row_snapshot(fixture.ui());
            let start = page as usize * size;
            assert_eq!(
                rows.iter().map(|row| row.0.clone()).collect::<Vec<_>>(),
                (start..(start + size).min(17))
                    .map(article_id)
                    .collect::<Vec<_>>()
            );
            let expected = state.mp_articles[OWNER][start..(start + size).min(17)]
                .iter()
                .map(|article| article.id.len() + article.title.len())
                .sum::<usize>();
            let stats = article_projection_stats();
            assert_eq!(stats.rows, rows.len());
            assert_eq!(stats.row_source_bytes, expected);
            assert_eq!(stats.presence_reads, 17);
            assert_eq!(state.store.chapter_body_read_count(), reads);
            if page == 0 {
                assert!(rows[1].3);
                assert!(!rows[2].3);
                assert_eq!(rows[1].1, "");
            }
        }
    }
    fixture.finish()
}

fn visible_paperwhite() -> Result<()> {
    visible_pages(DeviceProfile::Paperwhite6)
}
fn visible_scribe() -> Result<()> {
    visible_pages(DeviceProfile::Scribe)
}

fn next_empty_and_owner_switch() -> Result<()> {
    let fixture = fixture(DeviceProfile::Paperwhite6)?;
    {
        let mut state = fixture.app();
        state.navigate(Route::Article(CURRENT.into()), true)?;
        state.render_article()?;
        let raw = state
            .store
            .setting::<serde_json::Value>("mp.articles")?
            .unwrap();
        state.dispatch("article", "article.next", "")?;
        assert_eq!(state.article.id, article_id(1));
        assert_eq!(state.article.body, "");
        assert_eq!(
            fixture.ui().global::<ArticleModel>().get_title().as_str(),
            ""
        );
        assert!(fixture.ui().global::<ArticleModel>().get_has_next());
        assert!(state.pending_articles.is_empty());
        state.navigate(Route::Article("other-article".into()), true)?;
        fixture.ui().global::<ArticleModel>().set_list_open(true);
        reset_article_projection_stats();
        state.render_article()?;
        assert_eq!(row_snapshot(fixture.ui())[0].0, "other-article");
        assert_eq!(article_projection_stats().presence_reads, 1);
        assert_eq!(state.mp_articles[OWNER].len(), 17);
        assert_eq!(
            state
                .store
                .setting::<serde_json::Value>("mp.articles")?
                .unwrap(),
            raw
        );
    }
    fixture.finish()
}

fn output_boundaries() -> Result<()> {
    let fixture = fixture(DeviceProfile::Paperwhite6)?;
    {
        let mut state = fixture.app();
        state.navigate(Route::Article(CURRENT.into()), true)?;
        fixture.ui().global::<ArticleModel>().set_list_open(true);
        reset_article_projection_stats();
        state.render_article()?;
        let fee = article_projection_stats();
        let retained = Snapshot::capture(&state, fixture.ui())?;
        for limits in [
            ArticleProjectionLimits {
                output_bytes: fee.output_bytes - 1,
                ..Default::default()
            },
            ArticleProjectionLimits {
                work_bytes: fee.work_bytes - 1,
                ..Default::default()
            },
        ] {
            reset_article_projection_stats();
            let error =
                with_article_projection_limits(limits, || state.render_article()).unwrap_err();
            assert!(error.downcast_ref::<ArticleProjectionLimit>().is_some());
            assert_eq!(article_projection_stats().rows, 0);
            assert_eq!(article_projection_stats().selected_field_copies, 0);
            retained.assert_retained(&state, fixture.ui())?;
        }
        reset_article_projection_stats();
        with_article_projection_limits(
            ArticleProjectionLimits {
                work_bytes: fee.work_bytes,
                output_bytes: fee.output_bytes,
            },
            || state.render_article(),
        )?;
        retained.assert_retained(&state, fixture.ui())?;
        assert_eq!(article_projection_stats().rows, 8);
    }
    fixture.finish()
}

fn title_and_rank_metadata() -> Result<()> {
    let fixture = fixture(DeviceProfile::Paperwhite6)?;
    {
        let mut state = fixture.app();
        state.navigate(Route::Article(CURRENT.into()), true)?;
        state.mp_articles.clear();
        state.store.reset_book_read_stats();
        state.store.reset_selected_metadata_read_stats();
        state.render_article()?;
        assert_eq!(
            fixture.ui().global::<ArticleModel>().get_title().as_str(),
            "Current Article"
        );
        assert_eq!(state.store.book_read_stats().intro_copies, 0);
        assert_eq!(state.store.selected_metadata_read_stats().book_text_rows, 1);
        let connection = rusqlite::Connection::open(state.data_dir.join("library.sqlite"))?;
        let intro: String = connection.query_row(
            "SELECT intro FROM books WHERE id=?1",
            [article_book(CURRENT)],
            |row| row.get(0),
        )?;
        assert_eq!(intro, "Retained unrelated Book introduction. ".repeat(4096));
        connection.execute(
            "UPDATE books SET title='' WHERE id=?1",
            [article_book(CURRENT)],
        )?;
        state.render_article()?;
        assert_eq!(
            fixture.ui().global::<ArticleModel>().get_title().as_str(),
            ""
        );
        state.store.reset_book_read_stats();
        state.store.reset_selected_metadata_read_stats();
        state.remote_stats = Some(source::ReadStatistics {
            raw: json!({"readLongest": [
                {"book": {"bookId": article_book(CURRENT), "title": "Remote ranking label", "author": "Ranking author"}, "readTime": 20},
                {"book": {"bookId": "", "title": "Non-navigable ranking"}, "readTime": 10}
            ]}),
            ..Default::default()
        });
        state.render_statistics()?;
        assert_eq!(
            state.store.selected_metadata_read_stats().book_scalar_reads,
            1
        );
        assert_eq!(state.store.selected_metadata_read_stats().book_text_rows, 0);
        assert_eq!(state.store.book_read_stats().intro_copies, 0);
        assert_eq!(
            connection.query_row(
                "SELECT intro FROM books WHERE id=?1",
                [article_book(CURRENT)],
                |row| row.get::<_, String>(0)
            )?,
            intro
        );
    }
    fixture.finish()
}

fn style_navigation_rejection() -> Result<()> {
    let fixture = fixture(DeviceProfile::Paperwhite6)?;
    {
        let mut state = fixture.app();
        state.store.save_annotation(&Annotation {
            id: "article-style-note".into(),
            book_id: underline_projection_tests::BOOK.into(),
            chapter_index: 0,
            start: 0,
            end: 2,
            quote: "A\u{1f600}".into(),
            note: "Retained style draft".into(),
            created_at: 1,
        })?;
        state.dispatch("reader", "thought", "mine:article-style-note")?;
        assert!(state.reader.note_edit_context.is_some());
        let retained = Snapshot::capture(&state, fixture.ui())?;
        for limits in [
            StyleTestLimits {
                work_bytes: 0,
                ..Default::default()
            },
            StyleTestLimits {
                metadata_bytes: 0,
                ..Default::default()
            },
        ] {
            let observation = ObservationScope::start(limits);
            state.navigate(Route::Article(CURRENT.into()), true)?;
            assert_eq!(observation.stats().operations, 1);
            assert_eq!(observation.stats().usage.style_scans, 0);
            retained.assert_retained(&state, fixture.ui())?;
        }
    }
    fixture.finish()
}

fn style_reflow_and_reload_rejection() -> Result<()> {
    let fixture = fixture(DeviceProfile::Paperwhite6)?;
    {
        let mut state = fixture.app();
        state.navigate(Route::Article(CURRENT.into()), true)?;
        state.render_article()?;
        state.reader.selection = Some((0, 2));
        state.reader.draft.set_query("Held Article draft");
        state.reader.draft.press("n");
        let image_id = state.article.image_work.images()[0].id.clone();
        state.article.image_work.request_all(false);
        assert!(state.article.image_work.request_one(&image_id));
        assert!(state.article.image_work.admit(0, 42));
        state
            .pending_article_images
            .insert(format!("article-image:{CURRENT}:{image_id}"));
        let retained = Snapshot::capture(&state, fixture.ui())?;
        for reload in [false, true] {
            let observation = ObservationScope::start(StyleTestLimits {
                work_bytes: 0,
                ..Default::default()
            });
            let error = if reload {
                state.load_article(CURRENT)
            } else {
                state.reflow_article()
            }
            .unwrap_err();
            assert!(
                error
                    .downcast_ref::<crate::article::StyleLimitError>()
                    .is_some()
            );
            assert_eq!(observation.stats().usage.style_scans, 0);
            retained.assert_retained(&state, fixture.ui())?;
        }
    }
    fixture.finish()
}

fn actual_sql_error_preserves_the_route_and_propagates() -> Result<()> {
    let fixture = fixture(DeviceProfile::Paperwhite6)?;
    {
        let mut state = fixture.app();
        state.navigate(Route::Article(CURRENT.into()), true)?;
        state.render_article()?;
        store_article(
            &state,
            "sql-target",
            "Stored target",
            Some("Target body"),
            "<p>Target body</p>",
        )?;
        let retained = Snapshot::capture(&state, fixture.ui())?;
        let connection = rusqlite::Connection::open(state.data_dir.join("library.sqlite"))?;
        connection.execute_batch("ALTER TABLE books RENAME TO retained_books")?;
        let error = state
            .navigate(Route::Article("sql-target".into()), true)
            .unwrap_err();
        assert!(error.downcast_ref::<rusqlite::Error>().is_some());
        retained.assert_retained(&state, fixture.ui())?;
        connection.execute_batch("ALTER TABLE retained_books RENAME TO books")?;
    }
    fixture.finish()
}

fn remote_statistics_selected_output() -> Result<()> {
    let fixture = fixture(DeviceProfile::Paperwhite6)?;
    {
        let mut state = fixture.app();
        state.stats_period = "week".into();
        let ranks = (0..7).map(|index| json!({"book": {
            "bookId": if index == 0 { article_book(CURRENT) } else { format!("projection-rank-{index}") },
            "title":format!("Rank {index}"), "author":format!("Rank author {index}")},
            "readTime": if index == 6 { 7000 } else { 10 + index }
        })).collect::<Vec<_>>();
        let remote = source::ReadStatistics {
            raw: json!({"totalReadTime":8000, "readDays":2, "dayAverageReadTime":4000,
                "readStat":[{"stat":"First", "counts":"1"}, {"stat":"Second", "counts":"2"}, {"stat":"Unused", "counts":"u".repeat(4096)}],
                "readLongest":ranks, "preferCategory":(1..=6).map(|index| json!({"categoryTitle":format!("Category {index}"), "readingTime":index})).collect::<Vec<_>>()
            }),
            ..Default::default()
        };
        let raw = remote.raw.clone();
        state.remote_stats = Some(remote);
        state.store.reset_book_read_stats();
        state.store.reset_selected_metadata_read_stats();
        statistics::with_remote_projection_limits(
            statistics::RemoteProjectionLimits::default(),
            || -> Result<()> {
                state.render_statistics()?;
                let stats = statistics::remote_projection_stats();
                assert_eq!((stats.summaries, stats.ranks, stats.categories), (2, 4, 6));
                assert!(stats.text_bytes < 512);
                Ok(())
            },
        )?;
        let global = fixture.ui().global::<AuxiliaryModel>();
        assert_eq!(global.get_stats_ranking().row_count(), 4);
        assert!(
            (global.get_stats_ranking().row_data(0).unwrap().value - 10.0 / 7000.0).abs()
                < 0.000001
        );
        assert_eq!(global.get_stats_preferences().row_count(), 5);
        assert_eq!(
            global.get_stats_preferences().row_data(4).unwrap().percent,
            51
        );
        assert_eq!(
            state.store.selected_metadata_read_stats().book_scalar_reads,
            4
        );
        assert_eq!(state.store.book_read_stats().intro_copies, 0);
        assert!(!state.store.book_exists("projection-rank-6")?);
        assert_eq!(state.remote_stats.as_ref().unwrap().raw, raw);
    }
    fixture.finish()
}

fn remote_statistics_capacity_preserves_raw_and_existing_summary() -> Result<()> {
    let fixture = fixture(DeviceProfile::Paperwhite6)?;
    {
        let mut state = fixture.app();
        state.remote_stats = Some(source::ReadStatistics {
            raw: json!({"totalReadTime":100, "readDays":1,
            "readStat":[{"stat":"First", "counts":"1"}, {"stat":"Second", "counts":"2"}]}),
            ..Default::default()
        });
        state.render_statistics()?;
        let global = fixture.ui().global::<AuxiliaryModel>();
        let summary = global.get_stats_summary();
        let metrics = global
            .get_stats_metrics()
            .iter()
            .map(|row| (row.label.to_string(), row.value.to_string()))
            .collect::<Vec<_>>();
        let bars = global
            .get_stats_bars()
            .iter()
            .map(|row| (row.label.to_string(), row.value, row.future))
            .collect::<Vec<_>>();
        let raw = json!({"totalReadTime":200, "readDays":2,
            "readStat":[{"stat":"New first", "counts":"1"}, {"stat":"New second", "counts":"2"}],
            "preferCategory":[{"categoryTitle":"One", "readingTime":1}, {"categoryTitle":"Two", "readingTime":2}]});
        state.remote_stats = Some(source::ReadStatistics {
            raw: raw.clone(),
            ..Default::default()
        });
        statistics::with_remote_projection_limits(
            statistics::RemoteProjectionLimits {
                records: 2,
                ..Default::default()
            },
            || -> Result<()> {
                state.render_statistics()?;
                assert_eq!(statistics::remote_projection_stats().text_copies, 0);
                Ok(())
            },
        )?;
        assert_eq!(global.get_stats_summary(), summary);
        assert_eq!(
            global
                .get_stats_metrics()
                .iter()
                .map(|row| (row.label.to_string(), row.value.to_string()))
                .collect::<Vec<_>>(),
            metrics
        );
        assert_eq!(
            global
                .get_stats_bars()
                .iter()
                .map(|row| (row.label.to_string(), row.value, row.future))
                .collect::<Vec<_>>(),
            bars
        );
        assert_eq!(state.remote_stats.as_ref().unwrap().raw, raw);
        assert_eq!(global.get_stats_status().as_str(), tr("api_error"));
    }
    fixture.finish()
}

macro_rules! case {
    ($name:ident, $run:ident) => {
        #[test]
        fn $name() -> Result<()> {
            isolated(
                concat!("app::article_projection_budget_tests::", stringify!($name)),
                $run,
            )
        }
    };
}

case!(
    article_hidden_history_keeps_next_and_cached_summary_without_list_copies,
    hidden_history
);
case!(
    article_visible_history_pages_clone_only_selected_rows_on_paperwhite,
    visible_paperwhite
);
case!(
    article_visible_history_pages_clone_only_selected_rows_on_scribe,
    visible_scribe
);
case!(
    article_next_empty_text_and_owner_switch_preserve_raw_history,
    next_empty_and_owner_switch
);
case!(
    article_projection_exact_and_one_less_limits_preserve_the_entire_view,
    output_boundaries
);
case!(
    article_fallback_title_and_rank_existence_never_copy_retained_book_intro,
    title_and_rank_metadata
);
case!(
    article_style_quota_rejects_navigation_before_old_note_context_is_retired,
    style_navigation_rejection
);
case!(
    article_style_quota_rejects_reflow_and_reload_without_resetting_image_work,
    style_reflow_and_reload_rejection
);
case!(
    article_target_sql_error_keeps_the_old_route_and_is_not_a_capacity_success,
    actual_sql_error_preserves_the_route_and_propagates
);
case!(
    remote_stats_select_before_fields_sql_and_keep_hidden_rank_scale,
    remote_statistics_selected_output
);
case!(
    remote_stats_capacity_retains_raw_summary_and_existing_chart,
    remote_statistics_capacity_preserves_raw_and_existing_summary
);
