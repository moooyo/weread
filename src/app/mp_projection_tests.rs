//! Real App projections of duplicated legacy publication owners and cached articles.

use super::auxiliary::{PublicationProjectionLimits, PublicationProjectionStats};
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

const BOOK: &str = "publication-projection-reader";
const OWNERS: usize = 12;
const COPIES: usize = 3;
const ARTICLES: usize = 11;
const TITLE_REPEAT: usize = 256;

struct ProjectionUi;

impl UiPlatform for ProjectionUi {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

fn session() -> source::AuthSession {
    source::AuthSession {
        api_key: "synthetic-publication-projection-key".into(),
        user_vid: "synthetic-publication-projection-account".into(),
        account_name: "Publication projection fixture".into(),
        cookies: BTreeMap::from([
            (
                "wr_vid".into(),
                "synthetic-publication-projection-account".into(),
            ),
            (
                "wr_skey".into(),
                "synthetic-publication-projection-session".into(),
            ),
        ]),
        ..Default::default()
    }
}

fn owner(index: usize) -> String {
    format!("MP_WXS_projection_{index:02}")
}

fn account_name(index: usize) -> String {
    format!("First publication {:02}", OWNERS - index)
}

fn article_id(account: usize, index: usize) -> String {
    format!("projection-article-{account:02}-{index:02}")
}

fn article_book(account: usize, index: usize) -> String {
    format!("article:{}", article_id(account, index))
}

fn article_title(account: usize, index: usize) -> String {
    format!(
        "Retained title {account:02} {index:02} {}",
        "\u{4e2d}\u{1f642}".repeat(TITLE_REPEAT),
    )
}

fn article_author(account: usize, index: usize) -> String {
    format!("Author {account:02} {index:02}")
}

fn stored_accounts() -> Vec<MpAccount> {
    let mut accounts = Vec::new();
    for index in 0..OWNERS {
        accounts.push(MpAccount {
            id: owner(index),
            name: account_name(index),
            cover_url: format!("https://example.invalid/publication-{index:02}.png"),
            updated_at: 1_700_000_100 - index as i64,
            unread_count: 99,
        });
        for copy in 1..COPIES {
            accounts.push(MpAccount {
                id: owner(index),
                name: format!("Ignored duplicate {index:02} {copy}"),
                cover_url: format!("https://example.invalid/ignored-{index:02}-{copy}.png"),
                updated_at: 1_800_000_000 + copy as i64,
                unread_count: 999,
            });
        }
    }
    accounts
}

fn stored_articles(account: usize) -> Vec<MpArticle> {
    (0..ARTICLES)
        .map(|index| MpArticle {
            id: article_id(account, index),
            account_id: owner(account),
            title: article_title(account, index),
            author: article_author(account, index),
            // A tied timestamp exercises the previous last-max summary and
            // the separate stable timestamp/ID ordering of the article list.
            published_at: 1_700_000_000
                + if account == 5 && index + 2 == ARTICLES {
                    ARTICLES - 1
                } else {
                    index
                } as i64,
            original_url: format!("https://example.invalid/article-{account:02}-{index:02}"),
            cover_url: String::new(),
        })
        .collect()
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
        slint::platform::set_platform(Box::new(ProjectionUi))?;
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
            index.context("The small original font must register for publication projection")?
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
        preferences.push_on_close = false;
        preferences.reader.font = "song".into();
        preferences.reader.font_size = 20;
        preferences.reader.indent = 0;
        preferences.reader.justify = false;
        preferences.download_article_images = false;
        store.set_setting("preferences", &preferences)?;
        store.upsert_book(&Book {
            source: "local".into(),
            total_chapters: 1,
            ..Book::new(BOOK, "Retained projection Reader")
        })?;
        store.upsert_chapter(&Chapter {
            id: "projection-reader-chapter".into(),
            book_id: BOOK.into(),
            title: "Retained projection chapter".into(),
            body: Some(format!("A\u{1f642}B\n{}", "AB\n".repeat(16))),
            cached: true,
            content_status: ContentStatus::Full,
            ..Default::default()
        })?;
        store.save_annotation(&Annotation {
            id: "projection-retained-note".into(),
            book_id: BOOK.into(),
            chapter_index: 0,
            start: 0,
            end: 2,
            quote: "A\u{1f642}".into(),
            note: "Stored projection note".into(),
            created_at: 1,
        })?;
        // Use the same real SQL settings writer as the completed Shelf/article
        // consumer. App attachment below must load this old cache itself.
        store.set_setting("mp.accounts", &stored_accounts())?;
        let articles = (0..OWNERS)
            .map(|index| (owner(index), stored_articles(index)))
            .collect::<BTreeMap<_, _>>();
        store.set_setting("mp.articles", &articles)?;
        for account in 0..OWNERS {
            for index in 0..ARTICLES {
                let book_id = article_book(account, index);
                store.upsert_book(&Book {
                    source: "article".into(),
                    on_shelf: false,
                    total_chapters: 1,
                    ..Book::new(&book_id, article_title(account, index))
                })?;
                store.upsert_chapter(&Chapter {
                    id: article_id(account, index),
                    book_id,
                    index: 0,
                    body: (index != 1 && index != 2)
                        .then(|| format!("Hidden article body {account} {index}")),
                    content_status: ContentStatus::Preview,
                    ..Default::default()
                })?;
                store.set_setting(
                    &format!("article.read:{}", article_id(account, index)),
                    &(account % 3 == 0 || index == 1),
                )?;
            }
        }
        let database = rusqlite::Connection::open(self.directory.path().join("library.sqlite"))?;
        for account in 0..OWNERS {
            // Legacy empty TEXT remains present while index two remains NULL.
            database.execute(
                "UPDATE chapters SET body='' WHERE book_id=?1 AND chapter_index=0",
                [article_book(account, 1)],
            )?;
        }
        Ok(())
    }

    fn open(&self) -> Result<Instance> {
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
                initial_screen: "reader".into(),
                fonts: vec![FontRecord {
                    id: "song".into(),
                    label: "Original publication projection font".into(),
                    family: caret_test_font::FAMILY.into(),
                    metrics: self.metrics.clone(),
                    weights: BTreeMap::from([(400, self.metrics.clone())]),
                    available: true,
                }],
            },
            self.platform.clone(),
            self.runtime.as_ref().unwrap(),
        )?;
        {
            let mut state = app.borrow_mut();
            assert!(state.force_offline && !state.online);
            assert_eq!(state.mp_accounts.len(), OWNERS * COPIES);
            assert_eq!(state.mp_articles.len(), OWNERS);
            state.reader.selection = Some((0, 2));
            state.reader.draft.set_query("Retained projection draft");
            state.last_tick = Instant::now();
            state.device_poll_at = Instant::now() + Duration::from_secs(3600);
        }
        Ok(Instance { ui, app })
    }

    fn cache_snapshot(&self) -> Result<Vec<(String, String)>> {
        let database = rusqlite::Connection::open(self.directory.path().join("library.sqlite"))?;
        let mut statement = database.prepare(
            "SELECT key,value FROM settings WHERE key IN ('mp.accounts','mp.articles') ORDER BY key")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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

fn rows(ui: &AppWindow) -> Vec<PublicationRow> {
    ui.global::<AuxiliaryModel>()
        .get_publications_rows()
        .iter()
        .collect()
}

fn row_ids(ui: &AppWindow) -> Vec<String> {
    rows(ui).iter().map(|row| row.id.to_string()).collect()
}

fn row_source_bytes(ui: &AppWindow) -> usize {
    rows(ui)
        .iter()
        .map(|row| row.id.len() + row.title.len() + row.subtitle.len())
        .sum()
}

fn view_snapshot(ui: &AppWindow) -> (Vec<(String, String, String, i32, bool)>, String, i32) {
    let global = ui.global::<AuxiliaryModel>();
    (
        rows(ui)
            .into_iter()
            .map(|row| {
                (
                    row.id.to_string(),
                    row.title.to_string(),
                    row.subtitle.to_string(),
                    row.unread,
                    row.cached,
                )
            })
            .collect(),
        global.get_publications_subtitle().to_string(),
        global.get_publications_page(),
    )
}

fn assert_retained(state: &App) -> Result<()> {
    assert_eq!(state.reader.book_id, BOOK);
    assert!(state.reader.body.starts_with("A\u{1f642}B"));
    assert_eq!(state.reader.selection, Some((0, 2)));
    assert_eq!(state.reader.draft.query, "Retained projection draft");
    assert_eq!(state.jobs.session(), Some(session()));
    let notes = state.store.annotations(BOOK)?;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].quote, "A\u{1f642}");
    assert_eq!(notes[0].note, "Stored projection note");
    for account in 0..OWNERS {
        for index in 0..ARTICLES {
            assert_eq!(
                state
                    .store
                    .chapter_body_materialization_stats(&article_book(account, index), 0),
                (0, 0)
            );
        }
    }
    Ok(())
}

fn total_unread() -> usize {
    (0..OWNERS).filter(|account| account % 3 != 0).count() * (ARTICLES - 1)
}

fn duplicate_owners() -> Result<()> {
    let fixture = Fixture::new()?;
    let cached = fixture.cache_snapshot()?;
    let instance = fixture.open()?;
    {
        let mut state = instance.app.borrow_mut();
        state.dispatch("shell", "tab", "mp")?;
        let visible = rows(&instance.ui).len();
        assert_eq!(visible, 7);
        let stats = state.publication_stats.get();
        assert_eq!(stats.admitted_account_records, OWNERS * COPIES);
        assert_eq!(stats.owner_summaries, OWNERS);
        assert_eq!(stats.article_visits, OWNERS * ARTICLES);
        assert_eq!(stats.unread_reads, OWNERS * ARTICLES);
        assert_eq!(stats.presence_reads, 0);
        assert_eq!(stats.account_rows, visible);
        assert_eq!(stats.article_rows, 0);
        assert_eq!(stats.row_source_bytes, row_source_bytes(&instance.ui));
        assert_eq!(
            row_ids(&instance.ui),
            (0..visible).map(owner).collect::<Vec<_>>()
        );
        for (index, row) in rows(&instance.ui).iter().enumerate() {
            assert_eq!(row.title.as_str(), account_name(index));
            assert_eq!(row.subtitle.as_str(), article_title(index, ARTICLES - 1));
        }
        let expected = format!(
            "{} · {}",
            i18n::format(
                "mp_summary",
                &[
                    ("count", OWNERS.to_string()),
                    ("unread", total_unread().to_string())
                ]
            ),
            tr("local_unread")
        );
        assert_eq!(
            instance
                .ui
                .global::<AuxiliaryModel>()
                .get_publications_subtitle()
                .as_str(),
            expected
        );
        assert_retained(&state)?;
        // A new render gets a new borrowed projection, with no cross-account
        // memo or repeated work proportional to the old duplicate count.
        state.render()?;
        let next = state.publication_stats.get();
        assert_eq!(next.owner_summaries - stats.owner_summaries, OWNERS);
        assert_eq!(next.unread_reads - stats.unread_reads, OWNERS * ARTICLES);
        assert_eq!(next.account_rows - stats.account_rows, visible);
        for _ in 0..8 {
            state.last_tick = Instant::now();
            state.tick()?;
        }
        assert_eq!(state.publication_stats.get(), next);
        assert_retained(&state)?;
    }
    assert_eq!(fixture.cache_snapshot()?, cached);
    close(instance)
}

fn visible_rows() -> Result<()> {
    let fixture = Fixture::new()?;
    let cached = fixture.cache_snapshot()?;
    let instance = fixture.open()?;
    {
        let mut state = instance.app.borrow_mut();
        for _ in 0..3 {
            state.render()?;
        }
        assert_eq!(
            state.publication_stats.get(),
            PublicationProjectionStats::default()
        );
        state.dispatch("shell", "tab", "mp")?;
        let first = state.publication_stats.get();
        state.dispatch("auxiliary", "mp.page", "1")?;
        assert_eq!(
            row_ids(&instance.ui),
            (7..OWNERS).map(owner).collect::<Vec<_>>()
        );
        let second = state.publication_stats.get();
        assert_eq!(second.account_rows - first.account_rows, OWNERS - 7);
        assert_eq!(
            second.row_source_bytes - first.row_source_bytes,
            row_source_bytes(&instance.ui)
        );
        // The article route of the same publication view also pages lightweight
        // descriptors before copying any title or author from the old cache.
        state.dispatch("auxiliary", "mp.account", &owner(4))?;
        let article_first = state.publication_stats.get();
        assert_eq!(article_first.article_rows - second.article_rows, 7);
        assert_eq!(
            article_first.row_source_bytes - second.row_source_bytes,
            row_source_bytes(&instance.ui)
        );
        assert_eq!(
            row_ids(&instance.ui),
            (4..ARTICLES)
                .rev()
                .map(|index| article_id(4, index))
                .collect::<Vec<_>>()
        );
        state.dispatch("auxiliary", "mp.page", "1")?;
        let article_second = state.publication_stats.get();
        assert_eq!(
            row_ids(&instance.ui),
            (0..4)
                .rev()
                .map(|index| article_id(4, index))
                .collect::<Vec<_>>()
        );
        assert_eq!(article_second.article_rows - article_first.article_rows, 4);
        assert_eq!(
            article_second.row_source_bytes - article_first.row_source_bytes,
            row_source_bytes(&instance.ui)
        );
        assert_eq!(
            article_second.presence_reads - article_first.presence_reads,
            ARTICLES
        );
        assert_retained(&state)?;
    }
    assert_eq!(fixture.cache_snapshot()?, cached);
    close(instance)
}

fn filters_and_unread() -> Result<()> {
    let fixture = Fixture::new()?;
    let cached = fixture.cache_snapshot()?;
    let instance = fixture.open()?;
    {
        let mut state = instance.app.borrow_mut();
        state.dispatch("shell", "tab", "mp")?;
        state.dispatch("auxiliary", "mp.sort", "name")?;
        assert_eq!(
            row_ids(&instance.ui),
            (5..OWNERS).rev().map(owner).collect::<Vec<_>>()
        );
        state.dispatch("auxiliary", "mp.sort", "unread")?;
        let unread = (0..OWNERS)
            .filter(|index| index % 3 != 0)
            .take(7)
            .map(owner)
            .collect::<Vec<_>>();
        assert_eq!(row_ids(&instance.ui), unread);
        state.dispatch("auxiliary", "mp.sort", "recent")?;
        state.dispatch("auxiliary", "mp.edit-query", "Ignored duplicate")?;
        assert!(rows(&instance.ui).is_empty());
        let empty = state.publication_stats.get();
        state.render()?;
        let empty_again = state.publication_stats.get();
        assert_eq!(empty_again.account_rows, empty.account_rows);
        assert_eq!(empty_again.row_source_bytes, empty.row_source_bytes);
        state.dispatch("auxiliary", "mp.edit-query", "Author 05 03")?;
        assert_eq!(row_ids(&instance.ui), vec![owner(5)]);
        assert_eq!(
            rows(&instance.ui)[0].subtitle.as_str(),
            article_title(5, ARTICLES - 1)
        );
        assert_eq!(rows(&instance.ui)[0].unread, (ARTICLES - 1) as i32);
        // A real read-mark write must be visible to the next operation. A
        // retained owner memo would keep the earlier unread count here.
        state.store.set_setting(
            &format!("article.read:{}", article_id(5, ARTICLES - 1)),
            &true,
        )?;
        state.render()?;
        assert_eq!(rows(&instance.ui)[0].unread, (ARTICLES - 2) as i32);
        state.dispatch("auxiliary", "mp.edit-query", &account_name(0))?;
        assert_eq!(row_ids(&instance.ui), vec![owner(0)]);
        assert_eq!(rows(&instance.ui)[0].unread, 0);
        assert_retained(&state)?;
    }
    assert_eq!(fixture.cache_snapshot()?, cached);
    close(instance)
}

fn account_switch() -> Result<()> {
    let fixture = Fixture::new()?;
    let cached = fixture.cache_snapshot()?;
    let instance = fixture.open()?;
    {
        let mut state = instance.app.borrow_mut();
        state.dispatch("shell", "tab", "mp")?;
        state.dispatch("auxiliary", "mp.account", &owner(5))?;
        assert_eq!(
            instance
                .ui
                .global::<AuxiliaryModel>()
                .get_publications_title()
                .as_str(),
            account_name(5)
        );
        assert_eq!(row_ids(&instance.ui)[0], article_id(5, ARTICLES - 2));
        let before = state.publication_stats.get();
        state.dispatch("auxiliary", "mp.back", "")?;
        state.dispatch("auxiliary", "mp.account", &owner(8))?;
        assert_eq!(
            instance
                .ui
                .global::<AuxiliaryModel>()
                .get_publications_title()
                .as_str(),
            account_name(8)
        );
        assert!(
            row_ids(&instance.ui)
                .iter()
                .all(|id| id.starts_with("projection-article-08-"))
        );
        let switched = state.publication_stats.get();
        assert_eq!(switched.owner_summaries - before.owner_summaries, OWNERS);
        assert_eq!(switched.presence_reads - before.presence_reads, ARTICLES);
        state.dispatch("auxiliary", "mp.page", "1")?;
        for row in rows(&instance.ui) {
            assert_eq!(row.cached, row.id.as_str() != article_id(8, 2));
        }
        let summary = i18n::format(
            "articles_summary",
            &[
                ("count", ARTICLES.to_string()),
                ("cached", (ARTICLES - 1).to_string()),
            ],
        );
        assert_eq!(
            instance
                .ui
                .global::<AuxiliaryModel>()
                .get_publications_subtitle()
                .as_str(),
            summary
        );
        assert_retained(&state)?;
    }
    assert_eq!(fixture.cache_snapshot()?, cached);
    close(instance)
}

fn raw_account_fee(accounts: &[MpAccount]) -> usize {
    accounts
        .iter()
        .map(|account| {
            2 * (account.id.len()
                + account.name.len()
                + account.cover_url.len()
                + std::mem::size_of::<MpAccount>())
                + account.id.len()
        })
        .sum()
}

fn visible_account_fee(ui: &AppWindow) -> usize {
    rows(ui)
        .iter()
        .map(|row| {
            2 * (row.id.len()
                + row.title.len()
                + row.subtitle.len()
                + std::mem::size_of::<MpAccount>())
                + row.id.len()
        })
        .sum()
}

fn visible_article_fee(ui: &AppWindow) -> usize {
    rows(ui)
        .iter()
        .map(|row| {
            2 * (row.id.len()
                + row.title.len()
                + row.subtitle.len()
                + std::mem::size_of::<MpArticle>())
                + row.id.len()
        })
        .sum()
}

fn rejection_boundaries() -> Result<()> {
    let fixture = Fixture::new()?;
    let cached = fixture.cache_snapshot()?;
    let instance = fixture.open()?;
    {
        let mut state = instance.app.borrow_mut();
        state.dispatch("shell", "tab", "mp")?;
        let original = view_snapshot(&instance.ui);
        let raw_fee = raw_account_fee(&stored_accounts());
        let visible_fee = visible_account_fee(&instance.ui);
        for limits in [
            PublicationProjectionLimits {
                records: OWNERS * COPIES - 1,
                ..Default::default()
            },
            PublicationProjectionLimits {
                owner_bytes: owner(0).len() - 1,
                ..Default::default()
            },
            PublicationProjectionLimits {
                metadata_bytes: raw_fee - 1,
                ..Default::default()
            },
        ] {
            state
                .publication_stats
                .set(PublicationProjectionStats::default());
            state.render_publications_with_test_limits(limits)?;
            let stats = state.publication_stats.get();
            assert_eq!(stats.rejections, 1);
            assert_eq!(stats.owner_summaries, 0);
            assert_eq!(stats.unread_reads, 0);
            assert_eq!(stats.account_rows, 0);
            assert_eq!(stats.row_source_bytes, 0);
            assert_eq!(view_snapshot(&instance.ui), original);
            assert_retained(&state)?;
        }
        state
            .publication_stats
            .set(PublicationProjectionStats::default());
        state.render_publications_with_test_limits(PublicationProjectionLimits {
            records: OWNERS * COPIES,
            owner_bytes: owner(0).len(),
            metadata_bytes: raw_fee,
            visible_metadata_bytes: visible_fee,
        })?;
        assert_eq!(view_snapshot(&instance.ui), original);
        assert_eq!(state.publication_stats.get().rejections, 0);
        assert_eq!(state.publication_stats.get().account_rows, 7);
        state
            .publication_stats
            .set(PublicationProjectionStats::default());
        state.render_publications_with_test_limits(PublicationProjectionLimits {
            visible_metadata_bytes: visible_fee - 1,
            ..Default::default()
        })?;
        let stats = state.publication_stats.get();
        assert_eq!(stats.rejections, 1);
        assert_eq!(stats.owner_summaries, OWNERS);
        assert_eq!(stats.unread_reads, OWNERS * ARTICLES);
        assert_eq!(stats.account_rows, 0);
        assert_eq!(stats.row_source_bytes, 0);
        assert_eq!(view_snapshot(&instance.ui), original);
        assert_retained(&state)?;
        state.dispatch("auxiliary", "mp.account", &owner(4))?;
        let articles = view_snapshot(&instance.ui);
        let visible_fee = visible_article_fee(&instance.ui);
        state
            .publication_stats
            .set(PublicationProjectionStats::default());
        state.render_publications_with_test_limits(PublicationProjectionLimits {
            visible_metadata_bytes: visible_fee - 1,
            ..Default::default()
        })?;
        let stats = state.publication_stats.get();
        assert_eq!(stats.rejections, 1);
        assert_eq!(stats.presence_reads, ARTICLES);
        assert_eq!(stats.unread_reads, ARTICLES);
        assert_eq!(stats.article_rows, 0);
        assert_eq!(stats.row_source_bytes, 0);
        assert_eq!(view_snapshot(&instance.ui), articles);
        state
            .publication_stats
            .set(PublicationProjectionStats::default());
        state.render_publications_with_test_limits(PublicationProjectionLimits {
            visible_metadata_bytes: visible_fee,
            ..Default::default()
        })?;
        assert_eq!(state.publication_stats.get().rejections, 0);
        assert_eq!(state.publication_stats.get().article_rows, 7);
        assert_eq!(view_snapshot(&instance.ui), articles);
        assert_retained(&state)?;
    }
    assert_eq!(fixture.cache_snapshot()?, cached);
    close(instance)
}

fn cached_owner_rejection() -> Result<()> {
    let fixture = Fixture::new()?;
    let mut accounts = stored_accounts();
    accounts.last_mut().unwrap().id = "x".repeat(4097);
    fixture.store()?.set_setting("mp.accounts", &accounts)?;
    let cached = fixture.cache_snapshot()?;
    let instance = fixture.open()?;
    {
        let mut state = instance.app.borrow_mut();
        state.dispatch("shell", "tab", "mp")?;
        let stats = state.publication_stats.get();
        assert_eq!(stats.rejections, 1);
        assert_eq!(stats.admitted_account_records, 0);
        assert_eq!(stats.owner_summaries, 0);
        assert_eq!(stats.unread_reads, 0);
        assert_eq!(stats.account_rows, 0);
        assert_eq!(stats.row_source_bytes, 0);
        assert!(rows(&instance.ui).is_empty());
        assert_eq!(
            instance.ui.global::<ShellModel>().get_toast().as_str(),
            tr("api_error")
        );
        assert_retained(&state)?;
    }
    assert_eq!(fixture.cache_snapshot()?, cached);
    close(instance)
}

#[test]
fn duplicate_publication_owners_project_cached_articles_once_and_keep_the_first_record()
-> Result<()> {
    isolated(
        "app::mp_projection_tests::duplicate_publication_owners_project_cached_articles_once_and_keep_the_first_record",
        duplicate_owners,
    )
}

#[test]
fn publication_projection_materializes_only_visible_account_and_article_rows() -> Result<()> {
    isolated(
        "app::mp_projection_tests::publication_projection_materializes_only_visible_account_and_article_rows",
        visible_rows,
    )
}

#[test]
fn publication_projection_filters_sorts_and_refreshes_unread_from_real_settings() -> Result<()> {
    isolated(
        "app::mp_projection_tests::publication_projection_filters_sorts_and_refreshes_unread_from_real_settings",
        filters_and_unread,
    )
}

#[test]
fn publication_projection_switches_accounts_without_replacing_cached_metadata() -> Result<()> {
    isolated(
        "app::mp_projection_tests::publication_projection_switches_accounts_without_replacing_cached_metadata",
        account_switch,
    )
}

#[test]
fn publication_projection_admits_all_legacy_duplicates_before_rows_and_keeps_the_reader_on_rejection()
-> Result<()> {
    isolated(
        "app::mp_projection_tests::publication_projection_admits_all_legacy_duplicates_before_rows_and_keeps_the_reader_on_rejection",
        rejection_boundaries,
    )
}

#[test]
fn publication_projection_rejects_an_oversized_cached_owner_before_any_summary_or_copy()
-> Result<()> {
    isolated(
        "app::mp_projection_tests::publication_projection_rejects_an_oversized_cached_owner_before_any_summary_or_copy",
        cached_owner_rejection,
    )
}
