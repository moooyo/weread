//! Complete borrowed catalog fees precede every output-field materialization.

use super::super::client::api_gate_test_support::{HttpFixture, Reply, auth};
use super::*;
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Work {
    normalizations: usize,
    records: usize,
    copied_bytes: usize,
}

thread_local! {
    static WORK: Cell<Work> = const {
        Cell::new(Work { normalizations: 0, records: 0, copied_bytes: 0 })
    };
    static MAXIMUM: Cell<usize> = const { Cell::new(MAX_CATALOG_METADATA_BYTES) };
}

pub(super) fn observe_normalization() {
    WORK.with(|slot| {
        let mut work = slot.get();
        work.normalizations += 1;
        slot.set(work);
    });
}

pub(super) fn observe_output_copy(bytes: usize) {
    WORK.with(|slot| {
        let mut work = slot.get();
        work.records += 1;
        work.copied_bytes += bytes;
        slot.set(work);
    });
}

pub(super) fn maximum() -> usize {
    MAXIMUM.with(Cell::get).min(MAX_CATALOG_METADATA_BYTES)
}

struct Allowance(usize);

impl Allowance {
    fn new(maximum: usize) -> Self {
        Self(MAXIMUM.with(|slot| slot.replace(maximum.min(MAX_CATALOG_METADATA_BYTES))))
    }
}

impl Drop for Allowance {
    fn drop(&mut self) {
        MAXIMUM.with(|slot| slot.set(self.0));
    }
}

fn reset() {
    WORK.with(|slot| slot.set(Work::default()));
}

fn work() -> Work {
    WORK.with(Cell::get)
}

fn payload() -> Value {
    serde_json::json!({"bookId":"book","updated":[
        {"chapterUid":"first","wordCount":10,"title":"First","tar":"first.zip","chapterIdx":4},
        {"chapterId":17,"wordCount":20,"title":"t".repeat(2048),"tar":"r".repeat(1024),"chapterIdx":9}
    ]})
}

fn calibrate(value: &Value, book_id: &str) -> (Vec<RemoteChapter>, usize) {
    let expected = normalize_catalog(value, book_id);
    let fee = catalog_metadata_bytes(book_id, &expected).unwrap();
    (expected, fee)
}

#[test]
fn exact_complete_field_fee_accepts_and_one_less_rejects_before_any_normalization() {
    let value = payload();
    let before = value.clone();
    let (expected, fee) = calibrate(&value, "book");
    for maximum in [fee, fee - 1] {
        reset();
        let result = normalize_catalog_checked_with_limit(&value, "book", maximum);
        if maximum == fee {
            assert_eq!(
                serde_json::to_value(result.unwrap()).unwrap(),
                serde_json::to_value(&expected).unwrap()
            );
            assert_eq!(work().normalizations, 1);
            assert_eq!(work().records, 2);
            assert!(work().copied_bytes > 3072);
        } else {
            assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
            assert_eq!(work(), Work::default());
        }
        assert_eq!(value, before);
    }
}

#[test]
fn aliases_numeric_ids_defaults_unreadable_rows_and_other_books_keep_their_selection() {
    let value = serde_json::json!({"data":[
        {"bookId":1,"chapterInfos":[
            {"title":"unused".repeat(100),"wordCount":0},
            {"chapterUid":"cover","title":"\u{5c01}\u{9762}","wordCount":1},
            {"chapterId":17,"title":"\u{4e2d}\u{1f642}","wordCount":"2","chapterIdx":"5","tar":123},
            {"chapterUid":"opaque/\u{4e2d}","wordCount":3,"tar":"second.zip"}
        ]},
        {"bookId":"other","updated":[{"wordCount":1,"title":"unused".repeat(100),"tar":"unused"}]},
        {"bookId":1,"updated":null,"chapters":[{"chapterUid":"ignored","wordCount":1}]}
    ]});
    let (expected, fee) = calibrate(&value, "1");
    reset();
    let output = normalize_catalog_checked_with_limit(&value, "1", fee).unwrap();
    assert_eq!(output.len(), 2);
    assert_eq!(output[0].uid, "17");
    assert_eq!(output[0].index, 5);
    assert_eq!(output[0].resource_url, "");
    assert_eq!(output[1].uid, "opaque/\u{4e2d}");
    assert_eq!(output[1].title, "");
    assert_eq!(
        serde_json::to_value(output).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    assert_eq!(work().records, 2);
}

#[test]
fn count_and_uid_errors_keep_precedence_and_empty_outputs_do_not_pay_unused_fields() {
    let duplicate = serde_json::json!({"bookId":"book","updated":[
        {"chapterUid":"same","wordCount":1,"title":"t".repeat(1024)},
        {"chapterId":"same","wordCount":1}
    ]});
    reset();
    assert!(matches!(
        normalize_catalog_checked_with_limit(&duplicate, "book", 0),
        Err(SourceError::InvalidResponse("catalog_uid")),
    ));
    assert_eq!(work(), Work::default());
    let numeric_duplicate = serde_json::json!({"bookId":"book","updated":[
        {"chapterUid":17,"wordCount":1},
        {"chapterId":"17","wordCount":1}
    ]});
    reset();
    assert!(matches!(
        normalize_catalog_checked_with_limit(&numeric_duplicate, "book", 0),
        Err(SourceError::InvalidResponse("catalog_uid")),
    ));
    assert_eq!(work(), Work::default());
    let unreadable = serde_json::json!({"bookId":"book","updated":vec![
        serde_json::json!({"wordCount":0,"title":"Unused","tar":"Unused"});
        MAX_CATALOG_CHAPTERS
    ]});
    reset();
    assert!(
        normalize_catalog_checked_with_limit(&unreadable, "book", 0)
            .unwrap()
            .is_empty()
    );
    assert_eq!(work().normalizations, 1);
    assert_eq!(work().records, 0);
    let overflow =
        serde_json::json!({"bookId":"book","updated":vec![Value::Null; MAX_CATALOG_CHAPTERS + 1]});
    reset();
    assert!(matches!(
        normalize_catalog_checked_with_limit(&overflow, "book", usize::MAX),
        Err(SourceError::ResponseTooLarge),
    ));
    assert_eq!(work(), Work::default());
}

#[test]
fn selected_title_and_resource_utf8_bytes_share_the_same_preflight_and_final_fee() {
    let title = "\u{4e2d}\u{1f642}".repeat(20);
    let resource = "images/\u{4e2d}.zip";
    let value = serde_json::json!({"bookId":"book","chapters":[
        {"chapterUid":"uid","wordCount":1,"title":title,"tar":resource}
    ]});
    let (expected, fee) = calibrate(&value, "book");
    reset();
    let output = normalize_catalog_checked_with_limit(&value, "book", fee).unwrap();
    assert_eq!(catalog_metadata_bytes("book", &output).unwrap(), fee);
    assert_eq!(
        work().copied_bytes,
        expected[0].uid.len() + expected[0].title.len() + expected[0].resource_url.len()
    );
    reset();
    assert!(matches!(
        normalize_catalog_checked_with_limit(&value, "book", fee - 1),
        Err(SourceError::ResponseTooLarge),
    ));
    assert_eq!(work(), Work::default());
}

#[tokio::test(flavor = "current_thread")]
async fn real_catalog_response_is_refused_before_field_copies_and_keeps_authentication() {
    let value = payload();
    let (_, fee) = calibrate(&value, "book");
    let fixture = HttpFixture::new(move |request| {
        assert_eq!(request.method, "POST");
        assert_eq!(request.path(), "/web/book/chapterInfos");
        Reply::json(value.clone())
    });
    let client = fixture.client(auth());
    let before = client.session();
    let _allowance = Allowance::new(fee - 1);
    reset();
    let error = client.catalog("book").await.unwrap_err();
    assert!(matches!(error, SourceError::ResponseTooLarge));
    assert!(!error.requires_login() && !error.retryable());
    assert_eq!(work(), Work::default());
    assert_eq!(client.session(), before);
    let records = fixture.finish();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].json()["bookIds"], serde_json::json!(["book"]));
    assert!(records[0].headers.contains_key("cookie"));
}
