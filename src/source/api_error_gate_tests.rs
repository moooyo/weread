//! Bounded diagnostics preserve exact legacy classification through real API stages.

use super::api_gate_test_support::{HttpFixture, Reply, auth};
use super::*;
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Work {
    string_bytes: usize,
    serialized_fragment_bytes: usize,
    serialization_stops: usize,
    retained_bytes: usize,
}

thread_local! {
    static WORK: Cell<Work> = const {
        Cell::new(Work { string_bytes: 0, serialized_fragment_bytes: 0, serialization_stops: 0, retained_bytes: 0 })
    };
}

fn update(change: impl FnOnce(&mut Work)) {
    WORK.with(|slot| {
        let mut work = slot.get();
        change(&mut work);
        slot.set(work);
    });
}

pub(super) fn observe_string_byte() {
    update(|work| work.string_bytes += 1);
}

pub(super) fn observe_serialized_fragment(bytes: usize) {
    update(|work| work.serialized_fragment_bytes += bytes);
}

pub(super) fn observe_serialization_stop() {
    update(|work| work.serialization_stops += 1);
}

pub(super) fn observe_retained_code(bytes: usize) {
    update(|work| work.retained_bytes += bytes);
}

fn reset() {
    WORK.with(|slot| slot.set(Work::default()));
}

fn work() -> Work {
    WORK.with(Cell::get)
}

fn legacy_code(value: &Value) -> Option<String> {
    // Only controlled small values use the historical complete serialization.
    let text = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    if matches!(text.as_str(), "" | "0" | "null") {
        None
    } else {
        Some(
            text.chars()
                .filter(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                })
                .take(MAX_API_CODE_BYTES)
                .collect(),
        )
    }
}

fn assert_api(result: Result<()>, expected: &str) -> SourceError {
    let error = result.unwrap_err();
    assert!(matches!(&error, SourceError::Api(code) if code == expected));
    error
}

#[test]
fn exact_raw_success_precedence_and_succ_keep_the_original_contract() {
    for code in [json!(""), json!("0"), json!("null"), Value::Null, json!(0)] {
        let value = json!({"errCode":code,"errcode":"-2013","succ":true});
        reset();
        assert!(validate_api(&value).is_ok());
        assert_api(
            validate_api(&json!({"errCode":value["errCode"],"succ":false})),
            "rejected",
        );
    }
    for (code, expected) in [
        (json!(" 0"), "0"),
        (json!("null "), "null"),
        (json!("!"), ""),
        (json!([]), ""),
        (json!([0]), "0"),
        (json!([null]), "null"),
        (json!(0.0), "00"),
        (json!(-0.0), "-00"),
    ] {
        assert_api(validate_api(&json!({"errCode":code,"succ":true})), expected);
    }
    assert_api(
        validate_api(&json!({"errcode":"-2012","succ":true})),
        "-2012",
    );
}

#[test]
fn compact_composite_json_prefixes_escaping_order_and_numeric_spelling_are_unchanged() {
    for value in [
        json!({"z":"last","a":"\u{1}\n\t\\\"/\u{4e2d}","n":-1.25e-12,"raw":null}),
        json!([true, false, null, 123, -0.0, {"b":"two","a":"one"}]),
        json!("\u{1}\n\t\\\"/\u{4e2d}"),
        json!({"sorted":"-2010","another":"prefix"}),
        json!([["-2013"]]),
    ] {
        reset();
        assert_eq!(admitted_api_code(&value).unwrap(), legacy_code(&value));
        assert!(work().retained_bytes <= MAX_API_CODE_BYTES);
    }
}

#[test]
fn borrowed_string_prefix_stops_at_capacity_but_sparse_ascii_requires_real_scanning() {
    let value = json!("a".repeat(4096));
    reset();
    assert_eq!(admitted_api_code(&value).unwrap(), Some("a".repeat(64)));
    assert_eq!(work().string_bytes, 64);
    assert_eq!(work().retained_bytes, 64);
    let value = json!(format!("{}-2010", "!".repeat(2048)));
    reset();
    assert_eq!(admitted_api_code(&value).unwrap().as_deref(), Some("-2010"));
    assert_eq!(work().string_bytes, 2053);
    assert_eq!(work().retained_bytes, 5);
}

#[test]
fn non_string_serialization_stops_without_retaining_the_complete_subtree() {
    let value = json!(["a".repeat(4096), {"unused":"later"}]);
    reset();
    assert_eq!(admitted_api_code(&value).unwrap(), Some("a".repeat(64)));
    assert_eq!(work().retained_bytes, 64);
    assert_eq!(work().serialization_stops, 1);
    // The serializer can inspect a complete fragment before giving it to the
    // bounded sink. Do not misstate this as 64 total input-byte visits.
    assert!(work().serialized_fragment_bytes >= 4096);
}

#[test]
fn string_and_composite_login_error_codes_preserve_authentication_and_retry_classification() {
    for code in ["-2010", "-2012", "-2013"] {
        for value in [json!(code), json!([code])] {
            let error = assert_api(validate_api(&json!({"errCode":value})), code);
            assert!(error.requires_login());
            assert!(!error.retryable());
        }
    }
    let error = assert_api(
        validate_api(&json!({"errCode":["-2013","suffix"]})),
        "-2013suffix",
    );
    assert!(!error.requires_login() && !error.retryable());
}

#[tokio::test(flavor = "current_thread")]
async fn real_gateway_and_web_business_errors_keep_the_account_and_exact_request_stage() {
    let fixture = HttpFixture::new(|request| {
        assert!(matches!(request.path(), "/gateway" | "/web/mp/articles"));
        Reply::json(json!({"errCode":["-2013"],"succ":true}))
    });
    let client = fixture.client(auth());
    let before = client.session();
    reset();
    assert!(
        assert_api(
            client
                .gateway("/fixture/business", json!({"selected":"value"}))
                .await
                .map(|_| ()),
            "-2013",
        )
        .requires_login()
    );
    assert!(
        assert_api(
            client.mp_articles("MP_WXS_fixture", 0, 1).await.map(|_| ()),
            "-2013"
        )
        .requires_login()
    );
    assert_eq!(client.session(), before);
    let records = fixture.finish();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].method, "POST");
    assert_eq!(records[0].path(), "/gateway");
    assert_eq!(records[0].json()["api_name"], "/fixture/business");
    assert!(!records[0].headers.contains_key("cookie"));
    assert_eq!(records[1].method, "GET");
    assert_eq!(records[1].path(), "/web/mp/articles");
    assert!(
        records[1]
            .headers
            .get("cookie")
            .unwrap()
            .contains("synthetic-api-gate-session")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn real_reader_shard_rejection_precedes_decoding_without_an_auth_replay() {
    let fixture = HttpFixture::new(|request| {
        if request.path().starts_with("/web/reader/") {
            Reply::text(format!(
                "<script>window.__INITIAL_STATE__={};</script>",
                json!({
                    "user":{"vid":"synthetic-api-gate-vid"},
                    "reader":{
                        "bookId":"book","psvts":"synthetic-clock","pclts":0,
                        "bookInfo":{"bookId":"book","payType":32},
                        "chapterInfos":[{"chapterUid":"uid","chapterIdx":0,"wordCount":1000}],
                        "currentChapter":{"chapterUid":"uid","chapterIdx":0,"wordCount":1000}
                    }
                }),
            ))
        } else {
            assert_eq!(request.path(), "/web/book/chapter/e_0");
            Reply::json(json!({"errCode":["-2013"],"bookId":"book"}))
        }
    });
    let client = fixture.client(auth());
    let before = client.session();
    reset();
    let error = assert_api(client.chapter("book", "uid").await.map(|_| ()), "-2013");
    assert!(error.requires_login());
    assert_eq!(client.session(), before);
    let records = fixture.finish();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].method, "GET");
    assert!(records[0].path().starts_with("/web/reader/"));
    assert_eq!(records[1].method, "POST");
    assert_eq!(records[1].path(), "/web/book/chapter/e_0");
    assert!(
        records
            .iter()
            .all(|record| record.headers.contains_key("cookie"))
    );
}
