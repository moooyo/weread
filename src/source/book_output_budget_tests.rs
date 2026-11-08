//! Borrowed book/raw-tree admission and genuine bounded HTTP list producers.

use super::review_output_budget_tests::{GatewayFixture, assert_gateway, permanent_capacity_error};
use super::*;
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Probe {
    materializations: usize,
    raw_nodes: usize,
    maximum_depth: usize,
}

std::thread_local! {
    static PROBE: Cell<Probe> = Cell::new(Probe::default());
}

pub(super) fn observe_materialization() {
    PROBE.with(|slot| {
        let mut probe = slot.get();
        probe.materializations += 1;
        slot.set(probe);
    });
}

pub(super) fn observe_raw_node() {
    PROBE.with(|slot| {
        let mut probe = slot.get();
        probe.raw_nodes += 1;
        slot.set(probe);
    });
}

pub(super) fn observe_raw_depth(depth: usize) {
    PROBE.with(|slot| {
        let mut probe = slot.get();
        probe.maximum_depth = probe.maximum_depth.max(depth);
        slot.set(probe);
    });
}

fn reset_probe() {
    PROBE.with(|slot| slot.set(Probe::default()));
}

fn probe() -> Probe {
    PROBE.with(Cell::get)
}

fn book(id: &str, title: &str) -> Value {
    json!({"bookId":id,"title":title})
}

#[test]
fn book_aliases_numeric_ids_and_fields_preserve_the_complete_raw_value() {
    let value = json!({
        "bookInfo":{
            "bookId":17,"book_id":"ignored","title":23,"author":31,
            "cover":"https://example.invalid/t31_cover.png","coverUrl":"ignored",
            "categoryName":"Category","description":"Description",
            "publisher":"Publisher","isbn":"ISBN","chapterSize":8
        },
        "book":{"bookId":"ignored","title":"Ignored"},
        "progress":42,"updateTime":123,"unselected":{"items":[null, true, 7]}
    });
    reset_probe();
    let output = normalize_single_book(&value, "fallback", BookOutputLimits::default()).unwrap();
    assert_eq!(output.id, "17");
    assert_eq!(output.title, "23");
    assert_eq!(output.author, "31");
    assert_eq!(output.cover_url, "https://example.invalid/t9_cover.png");
    assert_eq!(output.category, "Category");
    assert_eq!(output.intro, "Description");
    assert_eq!(output.publisher, "Publisher");
    assert_eq!(output.isbn, "ISBN");
    assert_eq!(output.chapter_count, 8);
    assert_eq!(output.progress, 42);
    assert_eq!(output.updated_at, 123);
    assert_eq!(output.raw, value);
    assert_eq!(probe().materializations, 1);
    assert!(probe().raw_nodes > 8);
}

#[test]
fn book_first_empty_alias_stays_empty_and_single_info_prices_the_effective_fallback() {
    let value = json!({"book":{"bookId":"","book_id":"ignored","title":"Title","cover":"","coverUrl":"ignored","intro":"","description":"ignored"}});
    reset_probe();
    let output = normalize_single_book(&value, "fallback", BookOutputLimits::default()).unwrap();
    assert_eq!(output.id, "fallback");
    assert!(output.cover_url.is_empty() && output.intro.is_empty());
    assert_eq!(output.raw, value);
    let mut first = BookOutputBudget::new(BookOutputLimits::default());
    let mut longer = BookOutputBudget::new(BookOutputLimits::default());
    first.admit(&value, Some("a")).unwrap();
    longer.admit(&value, Some("abc")).unwrap();
    assert_eq!(longer.work_bytes - first.work_bytes, 4 * 2);
    reset_probe();
    permanent_capacity_error(
        &normalize_single_book(
            &value,
            "fallback",
            BookOutputLimits {
                id_bytes: 7,
                ..Default::default()
            },
        )
        .unwrap_err(),
    );
    assert_eq!(probe().materializations, 0);
}

#[test]
fn empty_list_ids_skip_unused_fields_and_raw_without_skipping_candidate_admission() {
    let mut ignored = Value::Null;
    for _ in 0..MAX_BOOK_RAW_DEPTH + 4 {
        ignored = json!([ignored]);
    }
    let value = json!({"bookInfo":{"bookId":"","title":"Unused long title"},"unselected":ignored});
    reset_probe();
    let mut budget = BookOutputBudget::new(BookOutputLimits {
        short_field_bytes: 1,
        work_bytes: 64,
        ..Default::default()
    });
    assert!(!budget.admit(&value, None).unwrap());
    assert_eq!(budget.records, 1);
    assert_eq!(budget.work_bytes, 64);
    assert_eq!(probe(), Probe::default());
}

#[test]
fn selected_book_field_byte_limits_reject_before_any_field_or_raw_copy() {
    for (field, maximum) in [
        ("bookId", MAX_BOOK_ID_BYTES),
        ("cover", MAX_BOOK_COVER_BYTES),
        ("title", MAX_BOOK_SHORT_FIELD_BYTES),
        ("author", MAX_BOOK_SHORT_FIELD_BYTES),
        ("category", MAX_BOOK_SHORT_FIELD_BYTES),
        ("publisher", MAX_BOOK_SHORT_FIELD_BYTES),
        ("isbn", MAX_BOOK_SHORT_FIELD_BYTES),
    ] {
        let mut value = book("id", "Title");
        value[field] = json!("x".repeat(maximum));
        reset_probe();
        assert!(normalize_single_book(&value, "fallback", BookOutputLimits::default()).is_ok());
        assert_eq!(probe().materializations, 1);
        value[field] = json!("x".repeat(maximum + 1));
        reset_probe();
        permanent_capacity_error(
            &normalize_single_book(&value, "fallback", BookOutputLimits::default()).unwrap_err(),
        );
        assert_eq!(probe().materializations, 0);
        assert_eq!(probe().raw_nodes, 0);
    }
    assert_eq!(MAX_BOOK_INTRO_BYTES, 1024 * 1024);
    for length in [16, 17] {
        let value = json!({"bookId":"id","title":"Title","intro":"x".repeat(length)});
        reset_probe();
        let result = normalize_single_book(
            &value,
            "fallback",
            BookOutputLimits {
                intro_bytes: 16,
                ..Default::default()
            },
        );
        if length == 16 {
            assert!(result.is_ok());
        } else {
            permanent_capacity_error(&result.unwrap_err());
            assert_eq!(probe().materializations, 0);
        }
    }
}

#[test]
fn utf8_book_id_uses_bytes_and_the_selected_empty_alias_does_not_fall_back() {
    let exact = format!("{}x", "\u{4e2d}".repeat(1365));
    assert_eq!(exact.len(), MAX_BOOK_ID_BYTES);
    let value = json!({"bookId":exact.clone(),"title":"Title"});
    assert!(normalize_single_book(&value, "fallback", BookOutputLimits::default()).is_ok());
    let value = json!({"bookId":format!("{exact}\u{4e2d}"),"title":"Title"});
    reset_probe();
    permanent_capacity_error(
        &normalize_single_book(&value, "fallback", BookOutputLimits::default()).unwrap_err(),
    );
    assert_eq!(probe().materializations, 0);
}

#[test]
fn raw_tree_fee_accepts_exact_and_rejects_one_less_before_clone() {
    let value = json!({"bookInfo":{"bookId":"id","title":"Title"},"outer key":{"array":[null, true, {"nested string":"Raw text"}]}});
    let mut calibration = BookOutputBudget::new(BookOutputLimits::default());
    calibration.admit(&value, Some("fallback")).unwrap();
    let fee = calibration.work_bytes;
    for allowance in [fee, fee - 1] {
        reset_probe();
        let result = normalize_single_book(
            &value,
            "fallback",
            BookOutputLimits {
                work_bytes: allowance,
                ..Default::default()
            },
        );
        if allowance == fee {
            assert_eq!(result.unwrap().raw, value);
            assert_eq!(probe().materializations, 1);
        } else {
            permanent_capacity_error(&result.unwrap_err());
            assert_eq!(probe().materializations, 0);
            assert!(probe().raw_nodes > 0);
        }
    }
}

#[test]
fn raw_array_visits_stop_on_the_shared_fee_without_retaining_all_siblings() {
    let value = json!({"bookId":"id","title":"Title","extra":vec![Value::Null; 1000]});
    reset_probe();
    permanent_capacity_error(
        &normalize_single_book(
            &value,
            "fallback",
            BookOutputLimits {
                work_bytes: 8000,
                ..Default::default()
            },
        )
        .unwrap_err(),
    );
    assert_eq!(probe().materializations, 0);
    assert!(probe().raw_nodes > 2 && probe().raw_nodes < 30);
    assert_eq!(probe().maximum_depth, 2);
}

#[test]
fn raw_tree_iterator_depth_is_bounded_before_copying_a_deep_value() {
    for depth in [MAX_BOOK_RAW_DEPTH - 1, MAX_BOOK_RAW_DEPTH] {
        let mut raw = Value::Null;
        for _ in 0..depth {
            raw = Value::Array(vec![raw]);
        }
        let value = json!({"bookId":"id","title":"Title","extra":raw});
        reset_probe();
        let result = normalize_single_book(&value, "fallback", BookOutputLimits::default());
        if depth == MAX_BOOK_RAW_DEPTH - 1 {
            assert!(result.is_ok());
            assert_eq!(probe().maximum_depth, MAX_BOOK_RAW_DEPTH);
            assert_eq!(probe().materializations, 1);
        } else {
            permanent_capacity_error(&result.unwrap_err());
            assert_eq!(probe().maximum_depth, MAX_BOOK_RAW_DEPTH);
            assert_eq!(probe().materializations, 0);
        }
    }
}

#[test]
fn book_count_includes_skipped_and_duplicate_candidates_before_materialization() {
    let small = json!({"books":[book("same", "First"), book("same", "Last"), Value::Null]});
    reset_probe();
    let mut budget = BookOutputBudget::new(BookOutputLimits {
        records: 2,
        ..Default::default()
    });
    permanent_capacity_error(
        &admit_book_candidates(ordinary_shelf_records(&small), &mut budget).unwrap_err(),
    );
    assert_eq!(budget.records, 2);
    assert_eq!(probe().materializations, 0);
    for count in [MAX_BOOK_OUTPUT_RECORDS, MAX_BOOK_OUTPUT_RECORDS + 1] {
        let value = json!({"books":vec![Value::Null; count]});
        let mut budget = BookOutputBudget::new(BookOutputLimits::default());
        let result = admit_book_candidates(ordinary_shelf_records(&value), &mut budget);
        if count == MAX_BOOK_OUTPUT_RECORDS {
            assert!(result.is_ok());
        } else {
            permanent_capacity_error(&result.unwrap_err());
        }
        assert_eq!(budget.records, MAX_BOOK_OUTPUT_RECORDS);
    }
}

#[test]
fn book_operation_forecast_stays_spent_and_custom_limits_can_only_tighten_defaults() {
    let value = book("id", "Title");
    let mut calibration = BookOutputBudget::new(BookOutputLimits::default());
    calibration.admit(&value, None).unwrap();
    let fee = calibration.work_bytes;
    let mut budget = BookOutputBudget::new(BookOutputLimits {
        work_bytes: fee + fee / 2,
        ..Default::default()
    });
    budget.admit(&value, None).unwrap();
    reset_probe();
    permanent_capacity_error(&budget.admit(&value, None).unwrap_err());
    assert!(budget.work_bytes > fee);
    assert_eq!(budget.records, 2);
    assert_eq!(probe().materializations, 0);
    let raised = BookOutputBudget::new(BookOutputLimits {
        records: usize::MAX,
        id_bytes: usize::MAX,
        short_field_bytes: usize::MAX,
        cover_bytes: usize::MAX,
        intro_bytes: usize::MAX,
        work_bytes: usize::MAX,
    });
    assert_eq!(raised.limits.records, MAX_BOOK_OUTPUT_RECORDS);
    assert_eq!(raised.limits.work_bytes, MAX_BOOK_OUTPUT_WORK);
}

#[tokio::test(flavor = "current_thread")]
async fn actual_shelf_keeps_ordinary_duplicates_and_mp_first_seen_without_unused_raw_work() {
    let value = json!({"books":[
        {"bookInfo":{"bookId":"same","title":"First ordinary"}},
        {"book":{"book_id":"MP_WXS_one","title":"First MP"},"unselected":vec![Value::Null; 100]},
        {"bookId":"same","title":"Last ordinary"},
        {"bookId":"MP_WXS_one","title":"Later MP"},
        {"bookId":"","title":"Unused empty ID"}
    ]});
    let fixture = GatewayFixture::new(vec![value.clone()]);
    let client = fixture.client();
    let before = client.session();
    reset_probe();
    let output = client.shelf_snapshot().await.unwrap();
    assert_eq!(
        output
            .books
            .iter()
            .map(|book| (book.id.as_str(), book.title.as_str()))
            .collect::<Vec<_>>(),
        [("same", "First ordinary"), ("same", "Last ordinary")]
    );
    assert_eq!(output.books[0].raw, value["books"][0]);
    assert_eq!(output.books[1].raw, value["books"][2]);
    assert_eq!(output.mp_accounts.len(), 1);
    assert_eq!(output.mp_accounts[0].name, "First MP");
    assert_eq!(probe().materializations, 2);
    assert!(
        probe().raw_nodes < 20,
        "An unused MP tree was charged or copied"
    );
    assert_eq!(client.session(), before);
    let records = fixture.finish();
    assert_eq!(records.len(), 1);
    assert_gateway(&records[0], "/shelf/sync");
}

#[tokio::test(flavor = "current_thread")]
async fn actual_shelf_and_search_count_rejections_precede_any_book_output() {
    for search in [false, true] {
        let records = vec![
            book("same", "First"),
            book("same", "Last"),
            book("unique", "Unique"),
        ];
        let value = if search {
            json!({"results":[{"books":records}]})
        } else {
            json!({"books":records})
        };
        let fixture = GatewayFixture::new(vec![value]);
        let client = fixture.client();
        let before = client.session();
        reset_probe();
        let limits = BookOutputLimits {
            records: 2,
            ..Default::default()
        };
        let error = if search {
            client
                .search_with_output_limits("keyword", limits)
                .await
                .unwrap_err()
        } else {
            client
                .shelf_snapshot_with_output_limits(MpAccountLimits::default(), limits)
                .await
                .unwrap_err()
        };
        permanent_capacity_error(&error);
        assert_eq!(probe().materializations, 0);
        assert_eq!(client.session(), before);
        let captured = fixture.finish();
        assert_eq!(captured.len(), 1);
        let payload = assert_gateway(
            &captured[0],
            if search {
                "/store/search"
            } else {
                "/shelf/sync"
            },
        );
        if search {
            assert_eq!(payload["count"], 30);
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn actual_search_keeps_first_seen_numeric_ids_and_only_retains_unique_raw_records() {
    let value = json!({"results":[{"books":[
        {"bookInfo":{"bookId":17,"title":"First"}},
        {"bookId":17,"title":"Later"},
        {"book":{"book_id":"other","title":"Other"}},
        {"bookId":"","title":"Skipped"}
    ]}]});
    let fixture = GatewayFixture::new(vec![value.clone()]);
    let client = fixture.client();
    reset_probe();
    let output = client.search("keyword").await.unwrap();
    assert_eq!(
        output
            .iter()
            .map(|book| (book.id.as_str(), book.title.as_str()))
            .collect::<Vec<_>>(),
        [("17", "First"), ("other", "Other")]
    );
    assert_eq!(output[0].raw, value["results"][0]["books"][0]);
    assert_eq!(output[1].raw, value["results"][0]["books"][2]);
    assert_eq!(
        probe().materializations,
        3,
        "Duplicate work was not part of this admitted response"
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(assert_gateway(&requests[0], "/store/search")["count"], 30);
}

#[tokio::test(flavor = "current_thread")]
async fn actual_single_book_info_applies_fallback_and_raw_fee_before_copying() {
    let value =
        json!({"bookInfo":{"bookId":"","title":"Fallback title"},"outer":{"metadata":[1,2,3]}});
    let mut calibration = BookOutputBudget::new(BookOutputLimits::default());
    calibration.admit(&value, Some("fallback")).unwrap();
    let fee = calibration.work_bytes;
    for allowance in [fee, fee - 1] {
        let fixture = GatewayFixture::new(vec![value.clone()]);
        let client = fixture.client();
        let before = client.session();
        reset_probe();
        let result = client
            .book_info_with_output_limits(
                "fallback",
                BookOutputLimits {
                    work_bytes: allowance,
                    ..Default::default()
                },
            )
            .await;
        if allowance == fee {
            let book = result.unwrap();
            assert_eq!(book.id, "fallback");
            assert_eq!(book.raw, value);
            assert_eq!(probe().materializations, 1);
        } else {
            permanent_capacity_error(&result.unwrap_err());
            assert_eq!(probe().materializations, 0);
        }
        assert_eq!(client.session(), before);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            assert_gateway(&requests[0], "/book/info")["bookId"],
            "fallback"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn oversized_book_info_request_and_invalid_title_create_no_unused_output() {
    let fixture = GatewayFixture::new(Vec::new());
    let client = fixture.client();
    reset_probe();
    permanent_capacity_error(
        &client
            .book_info(&"x".repeat(MAX_BOOK_ID_BYTES + 1))
            .await
            .unwrap_err(),
    );
    assert_eq!(probe(), Probe::default());
    assert!(fixture.finish().is_empty());
    let fixture = GatewayFixture::new(vec![json!({"bookId":"id","title":"","unselected":[1,2,3]})]);
    let client = fixture.client();
    reset_probe();
    assert!(matches!(
        client.book_info("id").await,
        Err(SourceError::InvalidResponse("book_info"))
    ));
    assert_eq!(probe(), Probe::default());
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn actual_queued_shelf_output_rejection_keeps_metadata_credit_and_auth() {
    use crate::jobs::{JobResult, Jobs};
    use std::sync::mpsc;
    let fixture = GatewayFixture::new(vec![
        json!({"books":[book(&"x".repeat(MAX_BOOK_ID_BYTES + 1), "Title")]}),
    ]);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let client = {
        let _context = runtime.enter();
        fixture.client()
    };
    let before = client.session();
    let (sender, receiver) = mpsc::channel();
    let jobs = {
        let _context = runtime.enter();
        Jobs::new(runtime.handle().clone(), sender, Some(client.clone())).unwrap()
    };
    let available = jobs.metadata_capacity();
    assert!(jobs.sync_shelf(74).is_none());
    let event = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(event.request, 74);
    assert!(event.credits.is_some());
    match &event.result {
        JobResult::Shelf(Err(error)) => permanent_capacity_error(error),
        other => panic!("The genuine Shelf producer did not reject capacity: {other:?}"),
    }
    assert_eq!(jobs.metadata_capacity(), available - 1);
    assert_eq!(client.session(), before);
    drop(event);
    assert_eq!(jobs.metadata_capacity(), available);
    assert!(receiver.try_recv().is_err());
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert_gateway(&requests[0], "/shelf/sync");
}
