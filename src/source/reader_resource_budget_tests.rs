//! Small genuine Source producers and tighten-only operation observations.

use super::api_gate_test_support::{HttpFixture, Reply, auth};
use super::*;
use crate::source::content::{ReaderSession, TextHtmlLimits, TextHtmlUsage};
use crate::source::protocol::{ReaderLimits, ResourceUrlLimits, ResourceUrlUsage};
use std::cell::Cell;
use std::io::{Cursor, Write};

#[derive(Clone, Copy, Default)]
struct Limits {
    text: TextHtmlLimits,
    reader: ReaderLimits,
    url: ResourceUrlLimits,
}

#[derive(Clone, Copy, Debug, Default)]
struct Stats {
    text_preflights: usize,
    text_outputs: usize,
    html_bytes: usize,
    text: TextHtmlUsage,
    fields: usize,
    scalar_copies: usize,
    scalar_bytes: usize,
    state_decodes: usize,
    script_copies: usize,
    payloads: usize,
    reader_urls: usize,
    token_hashes: usize,
    id_hashes: usize,
    url_preflights: usize,
    url_parses: usize,
    url_copies: usize,
    url_bytes: usize,
    catalog_records: usize,
    catalog_url_bytes: usize,
    url: ResourceUrlUsage,
    input_values: usize,
    input_readers: usize,
    input_occurrences: usize,
    input_metadata: usize,
    input_work: usize,
}

thread_local! {
    static LIMITS: Cell<Option<Limits>> = const { Cell::new(None) };
    static STATS: Cell<Stats> = Cell::new(Stats::default());
}

struct Scope {
    limits: Option<Limits>,
    stats: Stats,
}

impl Scope {
    fn start(limits: Limits) -> Self {
        Self {
            limits: LIMITS.with(|slot| slot.replace(Some(limits))),
            stats: STATS.with(|slot| slot.replace(Stats::default())),
        }
    }

    fn stats(&self) -> Stats {
        STATS.with(Cell::get)
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        LIMITS.with(|slot| slot.set(self.limits));
        STATS.with(|slot| slot.set(self.stats));
    }
}

fn observe(change: impl FnOnce(&mut Stats)) {
    STATS.with(|slot| {
        let mut stats = slot.get();
        change(&mut stats);
        slot.set(stats);
    });
}

pub(crate) fn reader_limits() -> ReaderLimits {
    LIMITS.with(Cell::get).unwrap_or_default().reader
}
pub(crate) fn url_limits() -> ResourceUrlLimits {
    LIMITS.with(Cell::get).unwrap_or_default().url
}
pub(crate) fn text_limits() -> TextHtmlLimits {
    LIMITS.with(Cell::get).unwrap_or_default().text
}
pub(crate) fn observe_fields(_: usize, _: usize) {
    observe(|stats| stats.fields += 1);
}
pub(crate) fn observe_scalar_copy(bytes: usize) {
    observe(|stats| {
        stats.scalar_copies += 1;
        stats.scalar_bytes += bytes;
    });
}
pub(crate) fn observe_state_decode() {
    observe(|stats| stats.state_decodes += 1);
}
pub(crate) fn observe_script_copy() {
    observe(|stats| stats.script_copies += 1);
}
pub(crate) fn observe_payload() {
    observe(|stats| stats.payloads += 1);
}
pub(crate) fn observe_reader_url() {
    observe(|stats| stats.reader_urls += 1);
}
pub(crate) fn observe_token_hash() {
    observe(|stats| stats.token_hashes += 1);
}
pub(crate) fn observe_id_hash() {
    observe(|stats| stats.id_hashes += 1);
}
pub(crate) fn observe_url_parse() {
    observe(|stats| stats.url_parses += 1);
}
pub(crate) fn observe_url_copy(bytes: usize) {
    observe(|stats| {
        stats.url_copies += 1;
        stats.url_bytes += bytes;
    });
}
pub(crate) fn observe_url_preflight(usage: ResourceUrlUsage) {
    observe(|stats| {
        stats.url_preflights += 1;
        stats.url = usage;
    });
}
pub(crate) fn observe_catalog_copy(bytes: usize) {
    observe(|stats| {
        stats.catalog_records += 1;
        stats.catalog_url_bytes += bytes;
    });
}
pub(crate) fn observe_text_preflight(usage: TextHtmlUsage) {
    observe(|stats| {
        stats.text_preflights += 1;
        stats.text = usage;
    });
}
pub(crate) fn observe_text_output(bytes: usize) {
    observe(|stats| {
        stats.text_outputs += 1;
        stats.html_bytes += bytes;
    });
}
pub(crate) fn observe_input(
    values: usize,
    readers: usize,
    occurrences: usize,
    metadata: usize,
    work: usize,
) {
    observe(|stats| {
        stats.input_values = values;
        stats.input_readers = readers;
        stats.input_occurrences = occurrences;
        stats.input_metadata = metadata;
        stats.input_work = work;
    });
}

fn capacity<T>(result: Result<T>) {
    assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
    assert!(!SourceError::ResponseTooLarge.requires_login());
    assert!(!SourceError::ResponseTooLarge.retryable());
}

fn reader_html(token: &str, locked: bool) -> String {
    let mut state = json!({
        "user":{"vid":auth().user_vid},
        "reader":{
            "bookId":"book", "psvts":"server-clock", "pclts":0, "token":token,
            "bookInfo":{"bookId":"book", "paid":u8::from(!locked), "payType":1},
            "chapterInfos":[{"chapterUid":"17","chapterIdx":0,"wordCount":1,"paid":u8::from(!locked),"price":if locked { 12 } else { 0 }}],
            "currentChapter":{"chapterUid":"17","chapterIdx":0,"wordCount":1,"paid":u8::from(!locked),"price":if locked { 12 } else { 0 }}
        }
    });
    if locked {
        // Supply the negative grant evidence used by the production checker.
        let reader = &mut state["reader"];
        reader["bookInfo"]["maxFreeChapter"] = json!(0);
        reader["bookInfo"]["payingStatus"] = json!(2);
        reader["bookInfo"]["shortTimeRead"] = json!({"active": 0});
        reader["chapterInfos"][0]["chapterIdx"] = json!(1);
        reader["currentChapter"]["chapterIdx"] = json!(1);
        reader["memberCardSummary"] = json!({
            "isPaying": 0,
            "expired": 1,
            "expiredTime": 0,
            "freeBookIds": []
        });
    }
    format!("<script>window.__INITIAL_STATE__={state};</script>")
}

fn text_fixture(text: &str, token: &str, locked: bool) -> HttpFixture {
    text_fixture_with_clock(text, token, locked, "server-clock")
}

fn text_fixture_with_clock(text: &str, token: &str, locked: bool, clock: &str) -> HttpFixture {
    let reader = reader_html(token, locked).replace(
        "\"psvts\":\"server-clock\"",
        &format!("\"psvts\":{}", serde_json::to_string(clock).unwrap()),
    );
    text_fixture_with_html(text, reader)
}

fn text_fixture_with_html(text: &str, reader: String) -> HttpFixture {
    let (first, second) = crate::source::content::fixture_text_shards(text);
    assert_eq!(
        crate::source::content::decode_shards(&[&first, &second]).unwrap(),
        text
    );
    HttpFixture::new(move |request| match request.path() {
        path if path.starts_with("/web/reader/") => Reply::text(reader.clone()),
        "/web/book/chapter/e_0" => Reply::json(json!({"bookId":"book"})),
        "/web/book/chapter/t_0" => Reply::text(first.clone()),
        "/web/book/chapter/t_1" => Reply::text(second.clone()),
        _ => panic!("Unexpected Source text request: {}", request.target),
    })
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_reader_exact_utf8_field_and_one_less_gate_before_any_owned_scalar() {
    let clock = format!("中{}", "x".repeat(4093));
    assert_eq!(clock.len(), 4096);
    for field_bytes in [clock.len(), clock.len() - 1] {
        let fixture = text_fixture_with_clock("Small text", "token", false, &clock);
        let client = fixture.client(auth());
        let before = client.session();
        let scope = Scope::start(Limits {
            reader: ReaderLimits {
                field_bytes,
                ..Default::default()
            },
            ..Default::default()
        });
        let result = client.chapter("book", "17").await;
        if field_bytes == clock.len() {
            assert_eq!(
                result.unwrap().content_status,
                crate::domain::ContentStatus::Full
            );
            assert!(scope.stats().scalar_bytes >= clock.len());
            assert!(scope.stats().payloads > 0);
        } else {
            capacity(result);
            assert_eq!(scope.stats().scalar_copies, 0);
            assert_eq!(scope.stats().state_decodes, 0);
            assert_eq!(scope.stats().payloads, 0);
        }
        assert_eq!(client.session(), before);
        assert_eq!(
            fixture.finish().len(),
            if field_bytes == clock.len() { 4 } else { 1 }
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_text_shards_exact_escaped_unicode_crlf_and_one_less_are_atomic() {
    let plain = " \r\n  &中<>\"'  \r\n\t\r\nSecond\n";
    let expected = "<p>  &amp;中&lt;&gt;&quot;&#39;  </p>\n<p>Second</p>";
    for output_bytes in [expected.len(), expected.len() - 1] {
        let fixture = text_fixture(plain, "token", false);
        let client = fixture.client(auth());
        let before = client.session();
        let scope = Scope::start(Limits {
            text: TextHtmlLimits {
                output_bytes,
                ..Default::default()
            },
            ..Default::default()
        });
        let result = client.chapter("book", "17").await;
        if output_bytes == expected.len() {
            let content = result.unwrap();
            assert_eq!(content.html, expected);
            assert_eq!(content.plain_text, plain);
            assert_eq!(content.content_status, crate::domain::ContentStatus::Full);
            assert_eq!(scope.stats().html_bytes, expected.len());
            assert_eq!(scope.stats().text_outputs, 1);
        } else {
            capacity(result);
            assert_eq!(scope.stats().text_outputs, 0);
            assert_eq!(scope.stats().html_bytes, 0);
        }
        assert_eq!(client.session(), before);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[2].path(), "/web/book/chapter/t_0");
        assert_eq!(requests[3].path(), "/web/book/chapter/t_1");
        assert_eq!(requests[2].json()["ps"], "server-clock");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_text_metadata_and_work_refuse_before_any_html_materialization() {
    let plain = "  A&B  \r\n中";
    let calibration = Scope::start(Limits::default());
    crate::source::content::paragraph_html_checked(plain).unwrap();
    let usage = calibration.stats().text;
    drop(calibration);
    for text in [
        TextHtmlLimits {
            metadata_bytes: usage.metadata_bytes - 1,
            ..Default::default()
        },
        TextHtmlLimits {
            work_bytes: usage.work_bytes - 1,
            ..Default::default()
        },
    ] {
        let fixture = text_fixture(plain, "token", false);
        let client = fixture.client(auth());
        let before = client.session();
        let scope = Scope::start(Limits {
            text,
            ..Default::default()
        });
        capacity(client.chapter("book", "17").await);
        assert_eq!(scope.stats().text_outputs, 0);
        assert_eq!(scope.stats().html_bytes, 0);
        assert_eq!(client.session(), before);
        assert_eq!(fixture.finish().len(), 4);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_empty_locked_text_preserves_unlock_error_without_html() {
    let plain = " \r\n\t";
    let html = reader_html("token", true);
    let account = auth();
    let permission = crate::source::content::extract_reader_session_with_access(
        &html,
        "book",
        Some("17"),
        Some(&account.user_vid),
        now_seconds(),
    )
    .unwrap();
    assert_eq!(
        permission.access.status_for_text(plain),
        crate::domain::ContentStatus::Locked
    );
    assert_eq!(
        permission.access.status_for_text("Visible synthetic text"),
        crate::domain::ContentStatus::Preview
    );
    let fixture = text_fixture_with_html(plain, html);
    let client = fixture.client(account);
    let before = client.session();
    let scope = Scope::start(Limits::default());
    assert!(matches!(
        client.chapter("book", "17").await,
        Err(SourceError::UnlockRequired)
    ));
    assert_eq!(scope.stats().text_outputs, 0);
    assert_eq!(scope.stats().text_preflights, 0);
    assert_eq!(scope.stats().html_bytes, 0);
    assert_eq!(scope.stats().state_decodes, 1);
    assert!(scope.stats().scalar_copies > 0);
    assert_eq!(client.session(), before);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].method, "GET");
    assert!(requests[0].path().starts_with("/web/reader/"));
    for (request, endpoint) in requests[1..].iter().zip([
        "/web/book/chapter/e_0",
        "/web/book/chapter/t_0",
        "/web/book/chapter/t_1",
    ]) {
        assert_eq!(request.method, "POST");
        assert_eq!(request.path(), endpoint);
        assert_eq!(request.json()["ps"], "server-clock");
    }
}

#[test]
fn borrowed_escaped_reader_keys_scalars_and_numeric_aliases_keep_selection() {
    let html = reader_html("17", false).replace(
        "\"psvts\":\"server-clock\"",
        "\"p\\u0073vts\":\"\\u4e2d\\ud83d\\ude42\"",
    );
    let legacy = crate::source::content::extract_reader_session(&html).unwrap();
    let strong = crate::source::content::extract_reader_session_with_access(
        &html,
        "book",
        Some("17"),
        Some(&auth().user_vid),
        now_seconds(),
    )
    .unwrap();
    assert_eq!(legacy.psvts, "中🙂");
    assert_eq!(strong.psvts, legacy.psvts);
    assert_eq!(legacy.pclts.as_deref(), Some("0"));
    let numeric = "<script>window.__INITIAL_STATE__={\"reader\":{\"psvts\":17,\"pclts\":0,\"token\":23}};</script>";
    let session = crate::source::content::extract_reader_session(numeric).unwrap();
    assert_eq!(session.psvts, "17");
    assert_eq!(session.token.as_deref(), Some("23"));
    let loose = crate::source::content::extract_reader_session(
        "<p>legacy</p> \"psvts\":\"clock\", \"token\":\"token\"",
    )
    .unwrap();
    assert_eq!(loose.psvts, "clock");
    assert_eq!(loose.token.as_deref(), Some("token"));
}

#[test]
fn later_reader_field_and_aggregate_refusal_precede_json_and_all_selected_copies() {
    let html = "<script>window.__INITIAL_STATE__={\"reader\":{\"psvts\":\"first\",\"pclts\":\"second\",\"token\":\"third\"}};</script>";
    for reader in [
        ReaderLimits {
            field_bytes: 5,
            ..Default::default()
        },
        ReaderLimits {
            aggregate_bytes: 15,
            ..Default::default()
        },
    ] {
        let scope = Scope::start(Limits {
            reader,
            ..Default::default()
        });
        capacity(crate::source::content::extract_reader_session(html));
        assert_eq!(scope.stats().state_decodes, 0);
        assert_eq!(scope.stats().scalar_copies, 0);
        assert_eq!(scope.stats().script_copies, 0);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_oversized_ssr_token_stops_before_state_decode_or_any_shard() {
    let fixture = text_fixture("Small text", &"t".repeat(4097), false);
    let client = fixture.client(auth());
    let before = client.session();
    let scope = Scope::start(Limits::default());
    capacity(client.chapter("book", "17").await);
    assert_eq!(scope.stats().state_decodes, 0);
    assert_eq!(scope.stats().scalar_copies, 0);
    assert_eq!(scope.stats().payloads, 0);
    assert_eq!(scope.stats().script_copies, 0);
    assert_eq!(client.session(), before);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].path().starts_with("/web/reader/"));
}

#[test]
fn reused_read_and_content_fields_precede_json_uri_and_token_hash_work() {
    let session = ReaderSession {
        psvts: "&中".into(),
        pclts: Some("clock".into()),
        token: Some("token".into()),
        ..Default::default()
    };
    let position = ReadingPosition {
        chapter_uid: "17".into(),
        summary: "中🙂".repeat(12),
        ..Default::default()
    };
    let expected_summary: String = position.summary.chars().take(20).collect();
    let positive = Scope::start(Limits::default());
    let payload = crate::source::protocol::read_payload_with_clock_checked(
        "book",
        &position,
        &session,
        Some(60),
        100,
        100000,
        7,
    )
    .unwrap();
    assert_eq!(payload["ps"], "&中");
    assert_eq!(payload["pc"], "clock");
    assert_eq!(payload["sm"], expected_summary);
    assert_eq!(positive.stats().token_hashes, 1);
    drop(positive);
    for reader in [
        ReaderLimits {
            metadata_bytes: 0,
            ..Default::default()
        },
        ReaderLimits {
            work_bytes: 0,
            ..Default::default()
        },
        ReaderLimits {
            request_bytes: 32,
            ..Default::default()
        },
    ] {
        let scope = Scope::start(Limits {
            reader,
            ..Default::default()
        });
        capacity(crate::source::protocol::content_payload_checked(
            "book",
            "17",
            &session.psvts,
            false,
            100,
        ));
        capacity(crate::source::protocol::read_payload_with_clock_checked(
            "book",
            &position,
            &session,
            Some(60),
            100,
            100000,
            7,
        ));
        assert_eq!(scope.stats().payloads, 0);
        assert_eq!(scope.stats().token_hashes, 0);
        assert_eq!(scope.stats().id_hashes, 0);
        assert_eq!(scope.stats().reader_urls, 0);
    }
    assert_eq!(position.summary, "中🙂".repeat(12));
}

#[test]
fn reader_clock_fallback_refusal_keeps_the_original_reused_session() {
    let mut session = ReaderSession {
        psvts: "clock".into(),
        pclts: Some("0".into()),
        ..Default::default()
    };
    let scope = Scope::start(Limits {
        reader: ReaderLimits {
            metadata_bytes: 0,
            ..Default::default()
        },
        ..Default::default()
    });
    capacity(session.ensure_reader_clock(100));
    assert_eq!(session.pclts.as_deref(), Some("0"));
    drop(scope);
    session.ensure_reader_clock(100).unwrap();
    let original = session.pclts.clone();
    session.ensure_reader_clock(101).unwrap();
    assert_eq!(session.pclts, original);
}

#[test]
fn archive_url_metadata_and_work_are_prepaid_before_normalization_or_copy() {
    let url = "https://res.weread.qq.com/中/archive.zip?key=kept";
    let positive = Scope::start(Limits::default());
    let plan =
        crate::source::images::make_plan("book", "17", "<img src='page.png'>", Some(url)).unwrap();
    assert!(plan.archive.as_ref().unwrap().url.contains("%E4%B8%AD"));
    let usage = positive.stats().url;
    for (raw, expected) in [
        (
            "//res.weread.qq.com/archive.zip",
            "https://res.weread.qq.com/archive.zip",
        ),
        ("/archive.zip", "https://weread.qq.com/archive.zip"),
    ] {
        let relative =
            crate::source::images::make_plan("book", "17", "<img src='page.png'>", Some(raw))
                .unwrap();
        assert_eq!(relative.archive.unwrap().url, expected);
    }
    drop(positive);
    for url_limits in [
        ResourceUrlLimits {
            metadata_bytes: usage.metadata_bytes - 1,
            ..Default::default()
        },
        ResourceUrlLimits {
            work_bytes: usage.work_bytes - 1,
            ..Default::default()
        },
    ] {
        let scope = Scope::start(Limits {
            url: url_limits,
            ..Default::default()
        });
        capacity(crate::source::images::make_plan(
            "book",
            "17",
            "<img src='page.png'>",
            Some(url),
        ));
        assert_eq!(scope.stats().url_parses, 0);
        assert_eq!(scope.stats().url_copies, 0);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_catalog_later_tar_limit_refuses_before_any_uid_title_or_url_output() {
    let payload = json!({"bookId":"book","updated":[
        {"chapterUid":"17","wordCount":1,"title":"First","tar":"https://res.weread.qq.com/first.zip"},
        {"chapterUid":"18","wordCount":1,"title":"Second","tar":format!("https://res.weread.qq.com/{}", "x".repeat(80))}
    ]});
    let raw = payload.clone();
    let fixture = HttpFixture::new(move |_| Reply::json(payload.clone()));
    let client = fixture.client(auth());
    let before = client.session();
    let scope = Scope::start(Limits {
        url: ResourceUrlLimits {
            raw_bytes: 64,
            ..Default::default()
        },
        ..Default::default()
    });
    capacity(client.catalog("book").await);
    assert_eq!(scope.stats().catalog_records, 0);
    assert_eq!(scope.stats().catalog_url_bytes, 0);
    assert_eq!(scope.stats().url_parses, 0);
    assert_eq!(client.session(), before);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].json()["bookIds"], json!(["book"]));
    assert_eq!(raw["updated"][0]["chapterUid"], "17");
    assert_eq!(raw["updated"][1]["title"], "Second");
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_catalog_raw_url_exact_default_and_one_more_precede_all_output() {
    let prefix = "https://res.weread.qq.com/";
    let exact = format!("{prefix}{}", "x".repeat(4096 - prefix.len()));
    for extra in ["", "x"] {
        let raw_url = format!("{exact}{extra}");
        let payload = json!({"bookId":"book","updated":[{"chapterUid":"17","wordCount":1,"title":"Title","tar":raw_url}]});
        let fixture = HttpFixture::new(move |_| Reply::json(payload.clone()));
        let client = fixture.client(auth());
        let before = client.session();
        let scope = Scope::start(Limits {
            url: ResourceUrlLimits {
                raw_bytes: usize::MAX,
                normalized_bytes: usize::MAX,
                ..Default::default()
            },
            ..Default::default()
        });
        let result = client.catalog("book").await;
        if extra.is_empty() {
            let catalog = result.unwrap();
            assert_eq!(catalog[0].uid, "17");
            assert_eq!(catalog[0].resource_url, exact);
            assert_eq!(scope.stats().catalog_records, 1);
            assert_eq!(scope.stats().catalog_url_bytes, 4096);
        } else {
            capacity(result);
            assert_eq!(scope.stats().catalog_records, 0);
            assert_eq!(scope.stats().catalog_url_bytes, 0);
        }
        assert_eq!(scope.stats().url_parses, 0);
        assert_eq!(client.session(), before);
        assert_eq!(fixture.finish().len(), 1);
    }
}

fn small_archive() -> Vec<u8> {
    let mut image_bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(1, 1, image::Luma([17])))
        .write_to(&mut image_bytes, image::ImageFormat::Png)
        .unwrap();
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("page.png", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(image_bytes.get_ref()).unwrap();
    zip.finish().unwrap().into_inner()
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_unicode_tar_catalog_plan_and_zip_fetch_keep_uid_and_original_account() {
    let archive = small_archive();
    let fixture = HttpFixture::new(move |request| match request.path() {
        "/web/book/chapterInfos" => Reply::json(
            json!({"bookId":"book","updated":[{"chapterUid":"17","wordCount":1,"title":"Title","tar":"https://res.weread.qq.com/中/archive.zip"}]}),
        ),
        "/%E4%B8%AD/archive.zip" => Reply::bytes(archive.clone()),
        _ => panic!("Unexpected archive request: {}", request.target),
    });
    let mut client = fixture.client(auth());
    client.test_resource_origin = Some(fixture.origin().to_owned());
    let before = client.session();
    let scope = Scope::start(Limits::default());
    let catalog = client.catalog("book").await.unwrap();
    assert_eq!(scope.stats().catalog_records, 1);
    assert_eq!(
        scope.stats().catalog_url_bytes,
        catalog[0].resource_url.len()
    );
    assert_eq!(catalog[0].uid, "17");
    assert_eq!(
        catalog[0].resource_url,
        "https://res.weread.qq.com/中/archive.zip"
    );
    let plan = crate::source::images::make_plan(
        "book",
        "17",
        "<img src='page.png'>",
        Some(&catalog[0].resource_url),
    )
    .unwrap();
    let downloaded = crate::source::images::acquire(&client, &plan)
        .await
        .unwrap();
    assert_eq!(downloaded.assets.len(), 1);
    assert!(downloaded.source_to_asset.contains_key("page.png"));
    assert!(scope.stats().url_parses > 0);
    assert!(scope.stats().url_copies > 0);
    assert_eq!(client.session(), before);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].headers.contains_key("cookie"));
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_resource_reuse_accepts_reader_referer_and_public_fetch_has_no_credentials() {
    let fixture = HttpFixture::new(|_| Reply::bytes(b"small".to_vec()));
    let mut client = fixture.client(auth());
    client.test_resource_origin = Some(fixture.origin().to_owned());
    let referer =
        crate::source::protocol::reader_url_checked(&"b".repeat(4096), Some(&"c".repeat(4096)))
            .unwrap();
    assert!(referer.len() > 16 * 1024);
    let before = client.session();
    assert_eq!(
        client
            .chapter_resource("https://res.weread.qq.com/fixture.tar", &referer, 64)
            .await
            .unwrap(),
        b"small"
    );
    assert_eq!(
        client
            .public_image("https://res.weread.qq.com/public.png", 64)
            .await
            .unwrap(),
        b"small"
    );
    assert_eq!(client.session(), before);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].headers.contains_key("cookie"));
    assert!(!requests[1].headers.contains_key("cookie"));
    assert!(!requests[1].headers.contains_key("authorization"));
    assert_eq!(requests[0].headers["referer"], referer);
}

fn duplicate_token_html(early: &str, escaped_key: bool) -> String {
    let key = if escaped_key {
        "\"t\\u006fken\""
    } else {
        "\"token\""
    };
    reader_html("last-token", false).replace(
        "\"token\":\"last-token\"",
        &format!("{key}:{early},\"token\":\"last-token\""),
    )
}

fn duplicate_reader_html(early: &str) -> String {
    let template = reader_html("last-token", false);
    let json = template
        .strip_prefix("<script>window.__INITIAL_STATE__=")
        .unwrap()
        .strip_suffix(";</script>")
        .unwrap();
    let value: Value = serde_json::from_str(json).unwrap();
    format!(
        "<script>window.__INITIAL_STATE__={{\"user\":{},\"reader\":{early},\"reader\":{}}};</script>",
        value["user"], value["reader"]
    )
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_overwritten_opaque_occurrences_refuse_before_any_state_decode() {
    let ordinary = serde_json::to_string(&"t".repeat(4097)).unwrap();
    let escaped = format!("\"{}\"", "\\u0074".repeat(4097));
    let container = json!({"held":"t".repeat(4097)}).to_string();
    for (early, escaped_key) in [(ordinary, false), (escaped, true), (container, false)] {
        let html = duplicate_token_html(&early, escaped_key);
        let original = html.clone();
        let fixture = text_fixture_with_html("Small text", html);
        let client = fixture.client(auth());
        let before = client.session();
        let scope = Scope::start(Limits::default());
        capacity(client.chapter("book", "17").await);
        assert_eq!(scope.stats().state_decodes, 0);
        assert_eq!(scope.stats().scalar_copies, 0);
        assert_eq!(scope.stats().payloads, 0);
        assert!(scope.stats().input_occurrences > 0);
        assert_eq!(client.session(), before);
        assert_eq!(fixture.finish().len(), 1);
        assert!(original.contains("last-token"));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_overwritten_reader_objects_and_unsupported_containers_are_priced() {
    for early in [
        json!({"psvts":"old-clock","token":"t".repeat(4097)}),
        json!([{"psvts":"old-clock","token":"t".repeat(4097)}]),
        json!({"token":{"held":"t".repeat(4097)}}),
    ] {
        let html = duplicate_reader_html(&early.to_string());
        let fixture = text_fixture_with_html("Small text", html);
        let client = fixture.client(auth());
        let before = client.session();
        let scope = Scope::start(Limits::default());
        capacity(client.chapter("book", "17").await);
        assert!(scope.stats().input_readers > 0);
        assert_eq!(scope.stats().state_decodes, 0);
        assert_eq!(scope.stats().scalar_copies, 0);
        assert_eq!(client.session(), before);
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_malformed_suffixes_do_not_refund_seen_or_unterminated_large_fields() {
    let early = serde_json::to_string(&"t".repeat(4097)).unwrap();
    let complete_field =
        duplicate_token_html(&early, false).replace("};</script>", ",broken};</script>");
    let unfinished = format!(
        "<script>window.__INITIAL_STATE__={{\"reader\":{{\"psvts\":\"server-clock\",\"token\":\"{}</script>",
        "t".repeat(4097),
    );
    for html in [complete_field, unfinished] {
        let fixture = text_fixture_with_html("Small text", html);
        let client = fixture.client(auth());
        let before = client.session();
        let scope = Scope::start(Limits::default());
        capacity(client.chapter("book", "17").await);
        assert_eq!(scope.stats().state_decodes, 0);
        assert_eq!(scope.stats().scalar_copies, 0);
        assert_eq!(scope.stats().payloads, 0);
        assert_eq!(client.session(), before);
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_small_last_selection_and_malformed_loose_fallback_remain_unchanged() {
    for html in [
        duplicate_token_html("\"first-token\"", false),
        duplicate_reader_html(&json!({"psvts":"old-clock","token":"first-token"}).to_string()),
    ] {
        let fixture = text_fixture_with_html("Small text", html);
        let client = fixture.client(auth());
        let before = client.session();
        let scope = Scope::start(Limits::default());
        let content = client.chapter("book", "17").await.unwrap();
        assert_eq!(content.content_status, crate::domain::ContentStatus::Full);
        assert_eq!(content.plain_text, "Small text");
        assert_eq!(content.html, "<p>Small text</p>");
        assert!(scope.stats().input_occurrences >= 3);
        assert_eq!(client.session(), before);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[2].json()["ps"], "server-clock");
    }
    let malformed =
        duplicate_token_html("\"first-token\"", false).replace("};</script>", ",broken};</script>");
    let fixture = text_fixture_with_html("Small text", malformed);
    let client = fixture.client(auth());
    let before = client.session();
    let scope = Scope::start(Limits::default());
    let session = client.reader_session("book", Some("17")).await.unwrap();
    assert_eq!(session.psvts, "server-clock");
    assert_eq!(session.token.as_deref(), Some("first-token"));
    assert_eq!(scope.stats().state_decodes, 0);
    assert!(scope.stats().scalar_copies > 0);
    assert_eq!(client.session(), before);
    assert_eq!(fixture.finish().len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_cumulative_decode_metadata_and_work_cover_all_overwritten_values() {
    let entries = (0..100)
        .map(|_| {
            format!(
                "\"token\":{},",
                serde_json::to_string(&"t".repeat(128)).unwrap()
            )
        })
        .collect::<String>();
    let html = reader_html("last-token", false).replace(
        "\"token\":\"last-token\"",
        &format!("{entries}\"token\":\"last-token\""),
    );
    let fixture = text_fixture_with_html("Small text", html.clone());
    let client = fixture.client(auth());
    let positive = Scope::start(Limits::default());
    let session = client.reader_session("book", Some("17")).await.unwrap();
    assert_eq!(session.token.as_deref(), Some("last-token"));
    let usage = positive.stats();
    assert!(usage.input_occurrences >= 103);
    assert!(usage.input_metadata > 4096);
    assert!(usage.input_work > 8192);
    assert_eq!(fixture.finish().len(), 1);
    drop(positive);
    for reader in [
        ReaderLimits {
            metadata_bytes: usage.input_metadata - 1,
            ..Default::default()
        },
        ReaderLimits {
            work_bytes: usage.input_work - 1,
            ..Default::default()
        },
    ] {
        let fixture = text_fixture_with_html("Small text", html.clone());
        let client = fixture.client(auth());
        let before = client.session();
        let scope = Scope::start(Limits {
            reader,
            ..Default::default()
        });
        capacity(client.reader_session("book", Some("17")).await);
        assert_eq!(scope.stats().state_decodes, 0);
        assert_eq!(scope.stats().scalar_copies, 0);
        assert_eq!(client.session(), before);
        assert_eq!(fixture.finish().len(), 1);
    }
}

fn strict_with_legacy_html(legacy: Value, clock: Option<Value>) -> String {
    let template = reader_html("last-token", false);
    let json = template
        .strip_prefix("<script>window.__INITIAL_STATE__=")
        .unwrap()
        .strip_suffix(";</script>")
        .unwrap();
    let mut value: Value = serde_json::from_str(json).unwrap();
    let reader = value["reader"].as_object_mut().unwrap();
    if let Some(clock) = clock {
        reader.insert("psvts".into(), clock);
    } else {
        reader.remove("psvts");
    }
    format!(
        "<script>window.__INITIAL_STATE__={{\"legacy\":{legacy},\"reader\":{},\"user\":{}}};</script>",
        value["reader"], value["user"]
    )
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_unusable_strict_clock_prices_actual_legacy_candidate_before_first_decode() {
    for clock in [None, Some(Value::Null)] {
        let html = strict_with_legacy_html(json!({"psvts":"p".repeat(4097)}), clock);
        let fixture = text_fixture_with_html("Small text", html);
        let client = fixture.client(auth());
        let before = client.session();
        let scope = Scope::start(Limits::default());
        capacity(client.reader_session("book", Some("17")).await);
        assert_eq!(scope.stats().state_decodes, 0);
        assert_eq!(scope.stats().scalar_copies, 0);
        assert_eq!(scope.stats().payloads, 0);
        assert_eq!(client.session(), before);
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_root_arrays_price_first_loose_fields_before_any_whole_value_decode() {
    for root in [
        json!([{"psvts":"p".repeat(4097)}, {"token":"t"}]),
        json!([{"psvts":"server-clock"}, {"token":"t".repeat(4097)}]),
    ] {
        let html = format!("<script>window.__INITIAL_STATE__={root};</script>");
        let fixture = text_fixture_with_html("Small text", html);
        let client = fixture.client(auth());
        let before = client.session();
        let scope = Scope::start(Limits::default());
        capacity(client.reader_session("book", Some("17")).await);
        assert_eq!(scope.stats().state_decodes, 0);
        assert_eq!(scope.stats().scalar_copies, 0);
        assert_eq!(client.session(), before);
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_usable_strict_reader_keeps_unknown_large_root_fields_generic() {
    let html = strict_with_legacy_html(
        json!({"psvts":"p".repeat(4097),"token":"t".repeat(4097)}),
        Some(json!("server-clock")),
    );
    let fixture = text_fixture_with_html("Small text", html);
    let client = fixture.client(auth());
    let before = client.session();
    let scope = Scope::start(Limits::default());
    let content = client.chapter("book", "17").await.unwrap();
    assert_eq!(content.content_status, crate::domain::ContentStatus::Full);
    assert_eq!(content.plain_text, "Small text");
    assert_eq!(scope.stats().state_decodes, 1);
    assert!(scope.stats().scalar_bytes < 4097);
    assert_eq!(client.session(), before);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[2].json()["ps"], "server-clock");
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_clock_branch_keeps_zero_numeric_escaped_and_blank_semantics() {
    for (clock, expected, strong) in [
        (json!(17), "17", true),
        (json!(0), "0", false),
        (json!(0.0), "0.0", true),
        (json!("0"), "0", false),
        (json!(" "), " ", true),
    ] {
        let html = strict_with_legacy_html(json!({"unused":"kept"}), Some(clock));
        let fixture = text_fixture_with_html("Small text", html);
        let client = fixture.client(auth());
        let before = client.session();
        let session = client.reader_session("book", Some("17")).await.unwrap();
        assert_eq!(session.psvts, expected);
        assert_eq!(
            client.reader_content_status(&session, "Small text").await
                == crate::domain::ContentStatus::Full,
            strong,
        );
        assert_eq!(client.session(), before);
        assert_eq!(fixture.finish().len(), 1);
    }
    let escaped = strict_with_legacy_html(json!({}), Some(json!("0")))
        .replace("\"psvts\":\"0\"", "\"p\\u0073vts\":\"\\u0030\"");
    let fixture = text_fixture_with_html("Small text", escaped);
    let client = fixture.client(auth());
    let session = client.reader_session("book", Some("17")).await.unwrap();
    assert_eq!(session.psvts, "0");
    assert_ne!(
        client.reader_content_status(&session, "Small text").await,
        crate::domain::ContentStatus::Full
    );
    assert_eq!(fixture.finish().len(), 1);
    let empty = strict_with_legacy_html(json!({}), Some(json!("")));
    let fixture = text_fixture_with_html("Small text", empty);
    let client = fixture.client(auth());
    assert!(matches!(
        client.reader_session("book", Some("17")).await,
        Err(SourceError::InvalidResponse("reader_session"))
    ));
    assert_eq!(fixture.finish().len(), 1);
}
