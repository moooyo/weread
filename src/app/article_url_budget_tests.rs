//! Genuine Article URL quotas are checked before replacing the current view.

use super::article_images::{ArticleImageWork, PreparedImageWork};
use super::underline_projection_tests::{Fixture, isolated};
use super::*;
use crate::article::{
    ArticleDocument, ArticleImage, ImageLimitError, ImageLimitKind, ImageTestLimits,
    image_budget_tests::ObservationScope,
};
use slint::Model;

const CURRENT: &str = "image-quota-current";
const TARGET: &str = "image-quota-target";
const OWNER: &str = "MP_WXS_image_quota";
const URL: &str = "https://example.invalid/kept.png";
const BODY: &str = "Before\nKept body\nAfter";

fn html(url: &str) -> String {
    format!(
        "<div id='js_content'><p>Before</p><p>Kept body</p><img src='{url}' alt='Kept caption'><p>After</p></div>"
    )
}
fn book(id: &str) -> String {
    format!("article:{id}")
}

fn store_article(state: &App, id: &str, html: &str) -> Result<()> {
    state.store.upsert_book(&Book {
        source: "article".into(),
        on_shelf: false,
        total_chapters: 1,
        ..Book::new(&book(id), "Image quota fixture")
    })?;
    state.store.upsert_chapter(&Chapter {
        id: id.into(),
        book_id: book(id),
        index: 0,
        body: Some(BODY.into()),
        content_status: ContentStatus::Full,
        ..Default::default()
    })?;
    state
        .store
        .set_setting(&format!("article.html:{id}"), &html)
}

fn fixture() -> Result<Fixture> {
    let fixture = Fixture::new("A retained Reader draft and context.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        state.prefs.download_article_images = false;
        let accounts = vec![MpAccount {
            id: OWNER.into(),
            name: "Image quota owner".into(),
            ..Default::default()
        }];
        let articles: BTreeMap<String, Vec<MpArticle>> = BTreeMap::from([(
            OWNER.into(),
            vec![MpArticle {
                id: CURRENT.into(),
                account_id: OWNER.into(),
                title: "Current image quota article".into(),
                ..Default::default()
            }],
        )]);
        state.store.set_setting("mp.accounts", &accounts)?;
        state.store.set_setting("mp.articles", &articles)?;
        state.mp_accounts = state.store.setting("mp.accounts")?.unwrap();
        state.mp_articles = state.store.setting("mp.articles")?.unwrap();
        state.mp_account = Some(OWNER.into());
        store_article(&state, CURRENT, &html(URL))?;
        state.navigate(Route::Article(CURRENT.into()), true)?;
        state.render_article()?;
        state.reader.selection = Some((0, 2));
        state.reader.draft.set_query("Retained URL quota draft");
        state.reader.draft.press("n");
        let id = state.article.image_work.images()[0].id.clone();
        state.article.image_work.request_all(false);
        assert!(state.article.image_work.request_one(&id));
        assert!(state.article.image_work.admit(0, 41));
        state
            .pending_article_images
            .insert(format!("article-image:{CURRENT}:{id}"));
        state.cache.put("retained-url-quota-payload", b"retained")?;
    }
    Ok(fixture)
}

struct Snapshot {
    route: Route,
    history: Vec<Route>,
    article: (String, String, String, usize),
    pages: Vec<Vec<crate::article::PageBlock>>,
    images: Vec<ArticleImage>,
    requests: Vec<Option<u64>>,
    work: bool,
    capacity: usize,
    pending: BTreeSet<String>,
    loaded: BTreeMap<String, PathBuf>,
    draft: (String, String, Option<(u32, u32)>),
    cache_keys: Vec<String>,
    cache_bytes: u64,
    epoch: u64,
    ui: (String, String, i32, i32, Vec<(String, String, String)>),
}

impl Snapshot {
    fn new(state: &App) -> Result<Self> {
        let ui = state
            .ui
            .upgrade()
            .expect("The genuine Article fixture has a current window");
        let global = ui.global::<ArticleModel>();
        Ok(Self {
            route: state.route.clone(),
            history: state.history.clone(),
            article: (
                state.article.id.clone(),
                state.article.body.clone(),
                state.article.html.clone(),
                state.article.page,
            ),
            pages: state.article.pages.clone(),
            images: state.article.image_work.images().to_vec(),
            requests: state
                .article
                .image_work
                .images()
                .iter()
                .map(|image| state.article.image_work.request_for(&image.id))
                .collect(),
            work: state.article.image_work.has_work(),
            capacity: state.article.image_work.available_capacity(),
            pending: state.pending_article_images.clone(),
            loaded: state.article.loaded_images.clone(),
            draft: (
                state.reader.draft.query.clone(),
                state.reader.draft.composition.clone(),
                state.reader.selection,
            ),
            cache_keys: state.cache.keys()?,
            cache_bytes: state.cache.usage()?,
            epoch: state.jobs.account_epoch(),
            ui: (
                global.get_title().to_string(),
                global.get_current_id().to_string(),
                global.get_page(),
                global.get_page_count(),
                global
                    .get_blocks()
                    .iter()
                    .map(|block| {
                        (
                            block.id.to_string(),
                            block.kind.to_string(),
                            block.text.to_string(),
                        )
                    })
                    .collect(),
            ),
        })
    }

    fn retained(&self, state: &App) -> Result<()> {
        assert_eq!(state.route, self.route);
        assert_eq!(state.history, self.history);
        assert_eq!(
            (
                state.article.id.clone(),
                state.article.body.clone(),
                state.article.html.clone(),
                state.article.page
            ),
            self.article
        );
        assert_eq!(state.article.pages, self.pages);
        assert_eq!(state.article.image_work.images(), self.images.as_slice());
        assert_eq!(
            state
                .article
                .image_work
                .images()
                .iter()
                .map(|image| state.article.image_work.request_for(&image.id))
                .collect::<Vec<_>>(),
            self.requests
        );
        assert_eq!(state.article.image_work.has_work(), self.work);
        assert_eq!(state.article.image_work.available_capacity(), self.capacity);
        assert_eq!(state.pending_article_images, self.pending);
        assert_eq!(state.article.loaded_images, self.loaded);
        assert_eq!(
            (
                state.reader.draft.query.clone(),
                state.reader.draft.composition.clone(),
                state.reader.selection
            ),
            self.draft
        );
        assert_eq!(state.cache.keys()?, self.cache_keys);
        assert_eq!(state.cache.usage()?, self.cache_bytes);
        assert_eq!(state.jobs.account_epoch(), self.epoch);
        let ui = state
            .ui
            .upgrade()
            .expect("The current Article window must remain available");
        let global = ui.global::<ArticleModel>();
        assert_eq!(
            (
                global.get_title().to_string(),
                global.get_current_id().to_string(),
                global.get_page(),
                global.get_page_count(),
                global
                    .get_blocks()
                    .iter()
                    .map(|block| (
                        block.id.to_string(),
                        block.kind.to_string(),
                        block.text.to_string()
                    ))
                    .collect::<Vec<_>>()
            ),
            self.ui
        );
        Ok(())
    }
}

fn no_transforms(observation: &ObservationScope) {
    let usage = observation.stats().usage;
    assert_eq!(usage.url_parse_calls, 0);
    assert_eq!(usage.url_input_copies, 0);
    assert_eq!(usage.url_output_copies, 0);
    assert_eq!(usage.url_hashes, 0);
}

fn raw_url_rejection() -> Result<()> {
    let fixture = fixture()?;
    {
        let mut state = fixture.app();
        let retained = Snapshot::new(&state)?;
        assert!(!state.prefs.download_article_images);
        for url in [
            format!("https://example.invalid/{}", "a".repeat(4097)),
            format!("data:image/png;base64,{}", "a".repeat(4097)),
        ] {
            let target = html(&url);
            store_article(&state, TARGET, &target)?;
            let observation = ObservationScope::start(ImageTestLimits::default());
            state.navigate(Route::Article(TARGET.into()), true)?;
            no_transforms(&observation);
            retained.retained(&state)?;
            assert_eq!(
                state
                    .store
                    .setting::<String>(&format!("article.html:{TARGET}"))?
                    .as_deref(),
                Some(target.as_str())
            );
            assert_eq!(
                state.store.chapter_body(&book(TARGET), 0)?.as_deref(),
                Some(BODY)
            );
        }
    }
    fixture.finish()
}

fn all_closed_view_quotas() -> Result<()> {
    let fixture = fixture()?;
    {
        let mut state = fixture.app();
        let retained = Snapshot::new(&state)?;
        for limits in [
            ImageTestLimits {
                raw_url_bytes: URL.len() - 1,
                ..Default::default()
            },
            ImageTestLimits {
                occurrences: 0,
                ..Default::default()
            },
            ImageTestLimits {
                work_bytes: 0,
                ..Default::default()
            },
            ImageTestLimits {
                metadata_bytes: 0,
                ..Default::default()
            },
        ] {
            let observation = ObservationScope::start(limits);
            let error = state.reflow_article().unwrap_err();
            assert!(error.downcast_ref::<ImageLimitError>().is_some());
            no_transforms(&observation);
            retained.retained(&state)?;
            assert!(!fixture.ui().global::<ArticleModel>().get_list_open());
            assert!(!state.prefs.download_article_images);
        }
    }
    fixture.finish()
}

fn direct_image_work() -> Result<()> {
    let mut work = ArticleImageWork::default();
    let invalid = ArticleImage {
        id: "invalid".into(),
        url: "u".repeat(16 * 1024 + 1),
        ..Default::default()
    };
    {
        let observation = ObservationScope::start(ImageTestLimits::default());
        let error = work.install(vec![invalid]).unwrap_err();
        assert_eq!(error.kind, ImageLimitKind::UrlBytes);
        no_transforms(&observation);
        assert!(!work.is_ready_for(""));
        assert!(work.images().is_empty());
    }
    for limits in [
        ImageTestLimits {
            occurrences: 0,
            ..Default::default()
        },
        ImageTestLimits {
            caption_bytes: 0,
            ..Default::default()
        },
        ImageTestLimits {
            metadata_bytes: 0,
            ..Default::default()
        },
    ] {
        let observation = ObservationScope::start(limits);
        let image = ArticleImage {
            id: "bounded".into(),
            url: URL.into(),
            caption: "Caption".into(),
        };
        assert!(work.install(vec![image]).is_err());
        no_transforms(&observation);
        assert!(work.images().is_empty());
        assert!(!work.is_ready_for(""));
    }
    let alias = "file:///legacy-first.png";
    let first = ArticleImage {
        id: "first".into(),
        url: alias.into(),
        caption: "First caption".into(),
    };
    let second = ArticleImage {
        id: "second".into(),
        url: alias.into(),
        caption: "Second caption".into(),
    };
    let prepared = PreparedImageWork::prepare(vec![first.clone(), second]).unwrap();
    work.install_prepared(prepared);
    assert_eq!(work.images()[0], first);
    assert!(work.request_one(alias));
    assert_eq!(work.next_candidate(), Some(0));
    Ok(())
}

fn incoming_owned_payload() -> Result<()> {
    let fixture = fixture()?;
    {
        let mut state = fixture.app();
        let retained = Snapshot::new(&state)?;
        {
            let observation = ObservationScope::start(ImageTestLimits {
                metadata_bytes: 0,
                ..Default::default()
            });
            let content = source::ChapterContent {
                html: html(URL),
                plain_text: BODY.into(),
                style: None,
                format: source::ContentFormat::Xhtml,
                content_status: ContentStatus::Full,
            };
            let error = state
                .prepare_incoming_article(12, CURRENT, content)
                .err()
                .expect("The whole incoming payload must be rejected");
            assert!(error.downcast_ref::<ImageLimitError>().is_some());
            no_transforms(&observation);
            retained.retained(&state)?;
        }
        let content = source::ChapterContent {
            html: html(URL).replace("Kept body", "New retained body"),
            plain_text: "Before\nNew retained body\nAfter".into(),
            style: None,
            format: source::ContentFormat::Xhtml,
            content_status: ContentStatus::Full,
        };
        let pointers = (content.html.as_ptr(), content.plain_text.as_ptr());
        let prepared = state.prepare_incoming_article(13, CURRENT, content)?;
        assert_eq!(
            (
                prepared.content().html.as_ptr(),
                prepared.content().plain_text.as_ptr()
            ),
            pointers
        );
        assert_eq!(prepared.request(), 13);
        // The real publisher borrows these fields for SQL and then consumes the
        // same opaque payload/document. It never reconstructs a second Doc.
        state.store.upsert_chapter(&Chapter {
            id: CURRENT.into(),
            book_id: book(CURRENT),
            index: 0,
            body: Some(prepared.content().plain_text.clone()),
            content_status: ContentStatus::Full,
            ..Default::default()
        })?;
        state
            .store
            .set_setting(&format!("article.html:{CURRENT}"), &prepared.content().html)?;
        let parses = state.article.document_parses;
        assert!(state.install_incoming_article(prepared)?);
        assert_eq!(state.article.document_parses, parses + 1);
        assert_eq!(state.article.body, "Before\nNew retained body\nAfter");
    }
    fixture.finish()
}

fn url_alias_semantics() -> Result<()> {
    let raw = concat!(
        "<p>Before</p><img data-src='http://unsafe.invalid/p.png' data-original='//example.invalid/a b.png'",
        " src='https://example.invalid/unused.png' alt='  Kept caption  '><p>After</p>"
    );
    let document = ArticleDocument::parse_checked(raw, BODY).unwrap();
    let images = document.images();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].url, "https://example.invalid/a%20b.png");
    assert_eq!(images[0].caption, "Kept caption");
    let mut work = ArticleImageWork::default();
    work.install(images.clone()).unwrap();
    assert_eq!(work.images(), images.as_slice());
    Ok(())
}

macro_rules! case {
    ($name:ident, $run:ident) => {
        #[test]
        fn $name() -> Result<()> {
            isolated(
                concat!("app::article_url_budget_tests::", stringify!($name)),
                $run,
            )
        }
    };
}

case!(
    article_raw_url_and_data_url_quota_preserve_the_closed_current_view,
    raw_url_rejection
);
case!(
    article_image_quotas_gate_closed_nonautomatic_reflow_before_transforms,
    all_closed_view_quotas
);
case!(
    article_image_work_admits_the_entire_vector_before_indexes_and_keeps_first_alias,
    direct_image_work
);
case!(
    article_incoming_preparation_owns_payload_once_and_reuses_checked_document,
    incoming_owned_payload
);
case!(
    article_image_url_aliases_keep_original_https_and_caption_semantics,
    url_alias_semantics
);
