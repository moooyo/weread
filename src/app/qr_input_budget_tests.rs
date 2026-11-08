//! Borrowed QR admission and genuine offline Article/Login consumers.

use super::super::library::{
    MAX_QR_PAYLOAD_BYTES, QrInputWork, qr_input_work, reset_qr_input_work, with_qr_input_test_limit,
};
use super::super::underline_projection_tests::{Fixture, isolated};
use super::*;
use crate::jobs::JobResult;

const OWNER: &str = "MP_WXS_qr-budget-owner";
const ARTICLE: &str = "qr-budget-article";
const ARTICLE_BOOK: &str = "article:qr-budget-article";
const ARTICLE_BODY: &str = "The stored article text remains available.";
const ARTICLE_HTML: &str = "<p>The stored article text remains available.</p>";
const VALID_URL: &str = "https://example.invalid/article?fixture=qr";

fn image_size(image: &slint::Image) -> (u32, u32) {
    let size = image.size();
    (size.width, size.height)
}

fn assert_qr_pixels(image: &slint::Image, selected_uri: &str) -> Result<()> {
    let expected = qrcode::QrCode::new(selected_uri.as_bytes())?;
    let qrcode::Version::Normal(version) = expected.version() else {
        anyhow::bail!("The complete Article URI must use a normal QR version");
    };
    assert_eq!(expected.width(), 17 + 4 * version as usize);
    assert_eq!(expected.error_correction_level(), qrcode::EcLevel::M);
    let pixels = image
        .to_rgb8()
        .expect("The published QR must retain its actual CPU pixel buffer");
    let edge = expected.width() + 8;
    assert_eq!(
        (pixels.width(), pixels.height()),
        (edge as u32, edge as u32)
    );
    for (index, pixel) in pixels.as_slice().iter().enumerate() {
        let (x, y) = (index % edge, index / edge);
        let ink = x >= 4
            && y >= 4
            && x < edge - 4
            && y < edge - 4
            && expected[(x - 4, y - 4)] == qrcode::Color::Dark;
        let channel = if ink { 0 } else { 255 };
        assert_eq!(
            (pixel.r, pixel.g, pixel.b),
            (channel, channel, channel),
            "QR module or quiet-zone mismatch at {x},{y}"
        );
    }
    Ok(())
}

fn qr_pixels(image: &slint::Image) -> Vec<u8> {
    image
        .to_rgb8()
        .expect("The actual QR pixel buffer must remain available")
        .as_bytes()
        .to_vec()
}

fn assert_resource_error(error: &anyhow::Error) {
    let error = error
        .downcast_ref::<source::SourceError>()
        .expect("QR admission retains its typed source resource error");
    assert!(matches!(error, source::SourceError::ResponseTooLarge));
    assert!(!error.requires_login());
    assert!(!error.retryable());
}

#[test]
fn shared_qr_byte_admission_precedes_the_actual_encoder() {
    with_qr_input_test_limit(4, || {
        reset_qr_input_work();
        let image = qr_image("ABCD").unwrap();
        assert!(image.size().width > 0);
        let work = qr_input_work();
        assert_eq!(work.admissions, 1);
        assert_eq!(work.encodes, 1);
        assert_eq!(work.rejections, 0);

        reset_qr_input_work();
        let error = qr_image("ABCDE").unwrap_err();
        assert_resource_error(&error);
        assert_eq!(
            qr_input_work(),
            QrInputWork {
                admissions: 1,
                rejections: 1,
                ..Default::default()
            },
        );
    });
}

#[test]
fn shared_qr_unicode_uses_utf8_bytes_without_truncating_payloads() {
    let exact = "\u{1f980}\u{1f980}";
    assert_eq!(exact.chars().count(), 2);
    assert_eq!(exact.len(), 8);
    with_qr_input_test_limit(8, || {
        reset_qr_input_work();
        let image = qr_image(exact).unwrap();
        assert!(image.size().width > 0);
        assert_eq!(qr_input_work().encodes, 1);
        let oversized = format!("{exact}a");
        reset_qr_input_work();
        let error = qr_image(&oversized).unwrap_err();
        assert_resource_error(&error);
        assert_eq!(oversized, format!("{exact}a"));
        assert_eq!(qr_input_work().encodes, 0);
        assert_eq!(qr_input_work().rejections, 1);
    });
}

#[test]
fn qr_test_allowances_cannot_raise_the_complete_production_payload_ceiling() {
    let prefix = "https://example.invalid/?q=";
    let exact = format!(
        "{prefix}{}",
        "a".repeat(MAX_QR_PAYLOAD_BYTES - prefix.len())
    );
    assert_eq!(exact.len(), 2048);
    with_qr_input_test_limit(usize::MAX, || {
        reset_qr_input_work();
        assert!(qr_image(&exact).unwrap().size().width > 0);
        assert_eq!(qr_input_work().encodes, 1);
        let oversized = format!("{exact}a");
        reset_qr_input_work();
        let error = qr_image(&oversized).unwrap_err();
        assert_resource_error(&error);
        assert_eq!(qr_input_work().encodes, 0);
        assert_eq!(qr_input_work().rejections, 1);
    });
    with_qr_input_test_limit(0, || {
        assert!(admit_qr_payload("a").is_err());
    });
    assert!(admit_qr_payload("a").is_ok());
}

fn account() -> source::AuthSession {
    source::AuthSession {
        api_key: "synthetic-qr-budget-key".into(),
        user_vid: "synthetic-qr-budget-account".into(),
        account_name: "QR budget fixture account".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "synthetic-qr-budget-account".into()),
            ("wr_skey".into(), "synthetic-qr-budget-session".into()),
        ]),
        ..Default::default()
    }
}

fn prepare_article(fixture: &Fixture, original_url: &str) -> Result<()> {
    let mut state = fixture.app();
    let account = account();
    state.demo = false;
    state.force_offline = true;
    state.online = false;
    state.reader.session_started = None;
    state.prefs.report_time = false;
    state.prefs.push_on_close = false;
    state.prefs.keep_awake = false;
    state.prefs.download_article_images = false;
    state.session_file.save(&account)?;
    state.jobs.replace_session(account.clone())?;
    state.persisted_session = Some(account);
    let mut book = Book::new(ARTICLE_BOOK, "Stored QR budget article");
    book.source = "article".into();
    book.total_chapters = 1;
    book.cached_chapters = 1;
    state.store.upsert_book(&book)?;
    state.store.upsert_chapter(&Chapter {
        id: ARTICLE.into(),
        book_id: ARTICLE_BOOK.into(),
        title: "Stored QR budget article".into(),
        body: Some(ARTICLE_BODY.into()),
        cached: true,
        content_status: ContentStatus::Full,
        ..Default::default()
    })?;
    state
        .store
        .set_setting(&format!("article.html:{ARTICLE}"), &ARTICLE_HTML)?;
    let accounts = vec![MpAccount {
        id: OWNER.into(),
        name: "QR budget publication".into(),
        ..Default::default()
    }];
    let articles = BTreeMap::from([(
        OWNER.to_owned(),
        vec![MpArticle {
            id: ARTICLE.into(),
            account_id: OWNER.into(),
            title: "Stored QR budget article".into(),
            author: "Fixture author".into(),
            original_url: original_url.into(),
            published_at: 100,
            ..Default::default()
        }],
    )]);
    state.store.set_setting("mp.accounts", &accounts)?;
    state.store.set_setting("mp.articles", &articles)?;
    // Use the persisted metadata shape read by App attachment, then the real
    // Article loader. This consumer has no network or framebuffer dependency.
    state.mp_accounts = state.store.setting("mp.accounts")?.unwrap();
    state.mp_articles = state.store.setting("mp.articles")?.unwrap();
    state.mp_account = Some(OWNER.into());
    state.navigate(Route::Article(ARTICLE.into()), true)?;
    state.render_article()?;
    state.reader.selection = Some((0, 4));
    assert!(state.reader.draft.set_query("Retained QR budget draft"));
    state.reader.draft.press("s");
    assert!(state.jobs.configured());
    assert!(state.force_offline && !state.online);
    Ok(())
}

fn replace_stored_url(fixture: &Fixture, value: &str) -> Result<()> {
    let mut state = fixture.app();
    state.mp_articles.get_mut(OWNER).unwrap()[0].original_url = value.into();
    state.store.set_setting("mp.articles", &state.mp_articles)
}

fn article_event(fixture: &Fixture, value: &str) {
    fixture
        .ui()
        .global::<ArticleModel>()
        .invoke_event("article.qr".into(), value.into());
}

struct ContentSnapshot {
    reader_body: String,
    draft: String,
    composition: String,
    selection: Option<(u32, u32)>,
    article_body: String,
    article_html: String,
    article_page: usize,
    stored_article_page: Option<usize>,
    stored_article_anchor: Option<(usize, u32)>,
    account: Option<source::AuthSession>,
    persisted_account: Option<source::AuthSession>,
    account_file: Option<source::AuthSession>,
    epoch: u64,
    stored_articles: serde_json::Value,
    cache_bytes: u64,
    asset_bytes: u64,
}

impl ContentSnapshot {
    fn new(fixture: &Fixture) -> Result<Self> {
        let state = fixture.app();
        Ok(Self {
            reader_body: state.reader.body.clone(),
            draft: state.reader.draft.query.clone(),
            composition: state.reader.draft.composition.clone(),
            selection: state.reader.selection,
            article_body: state.article.body.clone(),
            article_html: state.article.html.clone(),
            article_page: state.article.page,
            stored_article_page: state.store.setting(&format!("article.page:{ARTICLE}"))?,
            stored_article_anchor: state.store.setting(&format!("article.anchor:{ARTICLE}"))?,
            account: state.jobs.session(),
            persisted_account: state.persisted_session.clone(),
            account_file: state.session_file.load()?,
            epoch: state.jobs.account_epoch(),
            stored_articles: state.store.setting("mp.articles")?.unwrap(),
            cache_bytes: state.cache.usage()?,
            asset_bytes: state.book_assets.usage()?,
        })
    }

    fn assert_preserved(&self, fixture: &Fixture) -> Result<()> {
        let state = fixture.app();
        assert_eq!(state.route, Route::Article(ARTICLE.into()));
        assert_eq!(state.article.id, ARTICLE);
        assert_eq!(state.reader.body, self.reader_body);
        assert_eq!(state.reader.draft.query, self.draft);
        assert_eq!(state.reader.draft.composition, self.composition);
        assert_eq!(state.reader.selection, self.selection);
        assert_eq!(state.article.body, self.article_body);
        assert_eq!(state.article.html, self.article_html);
        assert_eq!(state.article.page, self.article_page);
        assert_eq!(
            state
                .store
                .setting::<usize>(&format!("article.page:{ARTICLE}"))?,
            self.stored_article_page
        );
        assert_eq!(
            state
                .store
                .setting::<(usize, u32)>(&format!("article.anchor:{ARTICLE}"))?,
            self.stored_article_anchor
        );
        assert_eq!(state.jobs.session(), self.account);
        assert_eq!(state.persisted_session, self.persisted_account);
        assert_eq!(state.session_file.load()?, self.account_file);
        assert_eq!(state.jobs.account_epoch(), self.epoch);
        assert_eq!(
            state
                .store
                .setting::<serde_json::Value>("mp.articles")?
                .unwrap(),
            self.stored_articles,
        );
        assert_eq!(
            state.store.chapter_body(ARTICLE_BOOK, 0)?.as_deref(),
            Some(ARTICLE_BODY)
        );
        assert_eq!(
            state
                .store
                .setting::<String>(&format!("article.html:{ARTICLE}"))?
                .as_deref(),
            Some(ARTICLE_HTML),
        );
        assert_eq!(state.cache.usage()?, self.cache_bytes);
        assert_eq!(state.book_assets.usage()?, self.asset_bytes);
        assert!(state.jobs.configured());
        assert!(state.force_offline && !state.online);
        assert!(state.pending_articles.is_empty());
        assert!(state.pending_article_images.is_empty());
        Ok(())
    }
}

fn repeated_article_rejections() -> Result<()> {
    let fixture = Fixture::new("Retained Reader QR fixture.", Vec::new(), false)?;
    prepare_article(&fixture, VALID_URL)?;
    article_event(&fixture, "true");
    let global = fixture.ui().global::<ArticleModel>();
    assert!(global.get_qr_open());
    let old_qr = image_size(&global.get_qr());
    assert!(old_qr.0 > 0);
    global.set_menu(true);
    let oversized = format!("https://example.invalid/?q={}", "aA".repeat(1024));
    assert!(oversized.len() > MAX_QR_PAYLOAD_BYTES);
    assert!(oversized.len() < 3 * 1024);
    replace_stored_url(&fixture, &oversized)?;
    let snapshot = ContentSnapshot::new(&fixture)?;
    reset_qr_input_work();
    for attempt in 1..=16 {
        article_event(&fixture, "true");
        assert_eq!(
            qr_input_work(),
            QrInputWork {
                admissions: attempt,
                rejections: attempt,
                article_lookups: attempt,
                ..Default::default()
            },
        );
        assert_eq!(image_size(&global.get_qr()), old_qr);
        assert!(global.get_qr_open());
        assert!(global.get_menu());
        assert_eq!(
            fixture.ui().global::<ShellModel>().get_toast().as_str(),
            tr("api_error")
        );
        snapshot.assert_preserved(&fixture)?;
    }
    let error = fixture
        .app()
        .dispatch("article", "article.qr", "true")
        .unwrap_err();
    assert_resource_error(&error);
    assert_eq!(qr_input_work().encodes, 0);
    assert_eq!(qr_input_work().uri_parses, 0);
    assert_eq!(qr_input_work().article_copies, 0);
    assert_eq!(qr_input_work().article_url_copy_bytes, 0);
    assert_eq!(qr_input_work().publications, 0);
    snapshot.assert_preserved(&fixture)?;
    drop(global);
    fixture.finish()
}

#[test]
fn genuine_article_qr_rejections_precede_copies_uri_parsing_and_encoding() -> Result<()> {
    isolated(
        "app::auxiliary::qr_input_budget_tests::genuine_article_qr_rejections_precede_copies_uri_parsing_and_encoding",
        repeated_article_rejections,
    )
}

fn article_unicode_boundary_and_close() -> Result<()> {
    let fixture = Fixture::new("Retained Reader QR fixture.", Vec::new(), false)?;
    let url = "https://example.invalid/\u{1f980}";
    assert!(url.len() > url.chars().count());
    prepare_article(&fixture, url)?;
    // Copy only the selected URI from the actual durable MP producer for the
    // oracle. The action now borrows metadata instead of cloning an article.
    let selected_uri = {
        let state = fixture.app();
        let stored: BTreeMap<String, Vec<MpArticle>> = state.store.setting("mp.articles")?.unwrap();
        stored[OWNER][0].original_url.clone()
    };
    assert_eq!(selected_uri, url);
    assert_eq!(selected_uri.as_bytes(), url.as_bytes());
    let snapshot = ContentSnapshot::new(&fixture)?;
    with_qr_input_test_limit(url.len(), || -> Result<()> {
        reset_qr_input_work();
        article_event(&fixture, "true");
        let work = qr_input_work();
        assert_eq!(work.admissions, 2);
        assert_eq!(work.rejections, 0);
        assert_eq!(work.uri_parses, 1);
        assert_eq!(work.encodes, 1);
        assert_eq!(work.publications, 1);
        assert_eq!(work.article_copies, 0);
        assert_eq!(work.article_url_copy_bytes, 0);
        assert_qr_pixels(
            &fixture.ui().global::<ArticleModel>().get_qr(),
            &selected_uri,
        )?;
        assert!(fixture.ui().global::<ArticleModel>().get_qr_open());
        assert!(!fixture.ui().global::<ArticleModel>().get_menu());
        snapshot.assert_preserved(&fixture)
    })?;
    let old_qr = image_size(&fixture.ui().global::<ArticleModel>().get_qr());
    let old_pixels = qr_pixels(&fixture.ui().global::<ArticleModel>().get_qr());
    let oversized = format!("{url}a");
    replace_stored_url(&fixture, &oversized)?;
    let snapshot = ContentSnapshot::new(&fixture)?;
    with_qr_input_test_limit(url.len(), || -> Result<()> {
        reset_qr_input_work();
        article_event(&fixture, "true");
        assert_eq!(qr_input_work().rejections, 1);
        assert_eq!(qr_input_work().uri_parses, 0);
        assert_eq!(qr_input_work().encodes, 0);
        assert_eq!(qr_input_work().article_copies, 0);
        assert_eq!(qr_input_work().article_url_copy_bytes, 0);
        assert_eq!(
            image_size(&fixture.ui().global::<ArticleModel>().get_qr()),
            old_qr
        );
        assert_eq!(
            qr_pixels(&fixture.ui().global::<ArticleModel>().get_qr()),
            old_pixels
        );
        snapshot.assert_preserved(&fixture)
    })?;
    with_qr_input_test_limit(0, || {
        reset_qr_input_work();
        article_event(&fixture, "false");
        assert!(!fixture.ui().global::<ArticleModel>().get_qr_open());
        assert_eq!(
            image_size(&fixture.ui().global::<ArticleModel>().get_qr()),
            old_qr
        );
        assert_eq!(qr_input_work().admissions, 0);
        assert_eq!(qr_input_work().uri_parses, 0);
        assert_eq!(qr_input_work().encodes, 0);
        assert_eq!(qr_input_work().publications, 0);
        assert_eq!(qr_input_work().article_copies, 0);
        assert_eq!(qr_input_work().article_url_copy_bytes, 0);
        assert_eq!(
            qr_pixels(&fixture.ui().global::<ArticleModel>().get_qr()),
            old_pixels
        );
    });
    snapshot.assert_preserved(&fixture)?;
    fixture.finish()
}

#[test]
fn genuine_article_qr_exact_unicode_input_and_close_preserve_cached_content() -> Result<()> {
    isolated(
        "app::auxiliary::qr_input_budget_tests::genuine_article_qr_exact_unicode_input_and_close_preserve_cached_content",
        article_unicode_boundary_and_close,
    )
}

fn invalid_article_url() -> Result<()> {
    let fixture = Fixture::new("Retained Reader QR fixture.", Vec::new(), false)?;
    prepare_article(&fixture, VALID_URL)?;
    article_event(&fixture, "true");
    let global = fixture.ui().global::<ArticleModel>();
    let old_qr = image_size(&global.get_qr());
    global.set_menu(true);
    for invalid in ["", "file:///qr-budget-fixture", "not-a-url"] {
        replace_stored_url(&fixture, invalid)?;
        let snapshot = ContentSnapshot::new(&fixture)?;
        reset_qr_input_work();
        article_event(&fixture, "true");
        assert_eq!(qr_input_work().admissions, 1);
        assert_eq!(qr_input_work().uri_parses, 1);
        assert_eq!(qr_input_work().encodes, 0);
        assert_eq!(qr_input_work().publications, 0);
        assert_eq!(image_size(&global.get_qr()), old_qr);
        assert!(global.get_qr_open());
        assert!(global.get_menu());
        assert_eq!(
            fixture.ui().global::<ShellModel>().get_toast().as_str(),
            tr("article_url_missing")
        );
        snapshot.assert_preserved(&fixture)?;
    }
    drop(global);
    fixture.finish()
}

#[test]
fn genuine_article_invalid_url_keeps_the_previous_qr_without_encoding() -> Result<()> {
    isolated(
        "app::auxiliary::qr_input_budget_tests::genuine_article_invalid_url_keeps_the_previous_qr_without_encoding",
        invalid_article_url,
    )
}

fn login_challenge_uses_shared_qr() -> Result<()> {
    let fixture = Fixture::new("Retained Reader QR fixture.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let body = state.reader.body.clone();
        let account = state.jobs.session();
        state.route = Route::Login;
        state.login_request = state.request();
        let request = state.login_request;
        let epoch = state.jobs.account_epoch();
        let challenge = source::QrLoginChallenge {
            uid: "synthetic-qr-login".into(),
            confirm_url: "https://weread.qq.com/web/confirm?uid=synthetic-qr-login".into(),
            expires_at: 9_999_999_999,
        };
        assert!(challenge.confirm_url.len() <= MAX_QR_PAYLOAD_BYTES);
        reset_qr_input_work();
        assert!(state.handle_job(JobEvent::new(
            request,
            epoch,
            JobResult::LoginBegin(Ok(challenge))
        ))?);
        assert_eq!(qr_input_work().admissions, 1);
        assert_eq!(qr_input_work().encodes, 1);
        assert_eq!(qr_input_work().article_lookups, 0);
        assert_eq!(qr_input_work().article_copies, 0);
        assert_eq!(state.reader.body, body);
        assert_eq!(state.jobs.session(), account);
        let login = fixture.ui().global::<LoginModel>();
        assert_eq!(login.get_step().as_str(), "qr");
        assert!(!login.get_busy());
        assert!(login.get_qr().size().width > 0);
        assert!(state.login_challenge.is_some());
    }
    fixture.finish()
}

#[test]
fn the_existing_login_result_consumer_retains_shared_qr_admission() -> Result<()> {
    isolated(
        "app::auxiliary::qr_input_budget_tests::the_existing_login_result_consumer_retains_shared_qr_admission",
        login_challenge_uses_shared_qr,
    )
}
