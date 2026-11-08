//! Wire position selection borrows both sources and copies only twenty summary scalars.

use super::super::client::api_gate_test_support::{HttpFixture, Reply, auth};
use super::*;
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Work {
    projections: usize,
    materializations: usize,
    summary_scalars: usize,
    summary_bytes: usize,
}

thread_local! {
    static WORK: Cell<Work> = const {
        Cell::new(Work { projections: 0, materializations: 0, summary_scalars: 0, summary_bytes: 0 })
    };
}

pub(super) fn observe_projection() {
    WORK.with(|slot| {
        let mut work = slot.get();
        work.projections += 1;
        slot.set(work);
    });
}

pub(super) fn observe_summary_scalar() {
    WORK.with(|slot| {
        let mut work = slot.get();
        work.summary_scalars += 1;
        slot.set(work);
    });
}

pub(super) fn observe_materialization(bytes: usize) {
    WORK.with(|slot| {
        let mut work = slot.get();
        work.materializations += 1;
        work.summary_bytes += bytes;
        slot.set(work);
    });
}

fn reset() {
    WORK.with(|slot| slot.set(Work::default()));
}

fn work() -> Work {
    WORK.with(Cell::get)
}

fn catalog() -> Vec<RemoteChapter> {
    vec![
        RemoteChapter {
            uid: "short".into(),
            index: 4,
            word_count: 100,
            ..Default::default()
        },
        RemoteChapter {
            uid: "long".into(),
            index: 9,
            word_count: 900,
            ..Default::default()
        },
    ]
}

fn local() -> ReadingPosition {
    ReadingPosition {
        chapter_uid: "short".into(),
        chapter_index: 999,
        chapter_offset: 90,
        progress: 99,
        summary: "\u{4e2d}\u{1f642}".repeat(512),
        updated_at: 80,
        device: "local-device".into(),
    }
}

fn cloud() -> ReadingPosition {
    ReadingPosition {
        chapter_uid: "long".into(),
        chapter_index: 999,
        chapter_offset: 450,
        progress: 1,
        summary: "\u{1f642}".repeat(1024),
        updated_at: 90,
        device: "cloud-device".into(),
    }
}

#[test]
fn only_the_selected_unicode_summary_is_scanned_and_copied_after_coordinate_validation() {
    let local = local();
    let cloud = cloud();
    let before_local = serde_json::to_value(&local).unwrap();
    let before_cloud = serde_json::to_value(&cloud).unwrap();
    reset();
    let selected = checked_report_position(&local, &cloud, &catalog(), false, 100).unwrap();
    assert_eq!(selected.chapter_uid, "long");
    assert_eq!(selected.chapter_index, 9);
    assert_eq!(selected.chapter_offset, 450);
    assert_eq!(selected.progress, 55);
    assert_eq!(selected.summary, "\u{1f642}".repeat(20));
    assert_eq!(selected.updated_at, 90);
    assert_eq!(selected.device, "cloud-device");
    assert_eq!(
        work(),
        Work {
            projections: 2,
            materializations: 1,
            summary_scalars: 20,
            summary_bytes: 80
        }
    );
    assert_eq!(serde_json::to_value(local).unwrap(), before_local);
    assert_eq!(serde_json::to_value(cloud).unwrap(), before_cloud);
}

#[test]
fn active_intent_recency_unknown_time_and_exact_twenty_scalar_prefixes_keep_selection() {
    for (active, local_time, cloud_time, expected_uid) in [
        (true, 0, 200, "short"),
        (false, 95, 90, "short"),
        (false, 90, 90, "long"),
        (false, 0, 90, "long"),
        (false, 200, 90, "long"),
    ] {
        let mut local = local();
        let mut cloud = cloud();
        local.updated_at = local_time;
        cloud.updated_at = cloud_time;
        local.summary = " leading whitespace is preserved ".into();
        reset();
        let selected = checked_report_position(&local, &cloud, &catalog(), active, 100).unwrap();
        assert_eq!(selected.chapter_uid, expected_uid);
        let original = if expected_uid == "short" {
            &local
        } else {
            &cloud
        };
        assert_eq!(
            selected.summary,
            original.summary.chars().take(20).collect::<String>()
        );
        assert_eq!(selected.updated_at, original.updated_at);
        assert_eq!(work().materializations, 1);
        assert_eq!(work().summary_scalars, 20);
    }
    for summary in ["", "Short", "12345678901234567890", "123456789012345678901"] {
        let mut local = local();
        local.summary = summary.into();
        reset();
        let selected = checked_report_position(&local, &cloud(), &catalog(), true, 100).unwrap();
        assert_eq!(
            selected.summary,
            summary.chars().take(20).collect::<String>()
        );
        assert_eq!(work().summary_scalars, summary.chars().count().min(20));
    }
}

#[test]
fn clamping_and_invalid_catalog_or_either_position_do_not_copy_a_summary() {
    let mut local = local();
    local.chapter_offset = 999;
    reset();
    let selected = checked_report_position(&local, &cloud(), &catalog(), true, 100).unwrap();
    assert_eq!(selected.chapter_offset, 100);
    assert_eq!(selected.progress, 10);
    let mut unknown = cloud();
    unknown.chapter_uid = "missing".into();
    reset();
    assert!(checked_report_position(&local, &unknown, &catalog(), true, 100).is_err());
    assert_eq!(work().materializations, 0);
    assert_eq!(work().summary_scalars, 0);
    let mut ambiguous = catalog();
    ambiguous[1].uid = "short".into();
    reset();
    assert!(checked_report_position(&local, &cloud(), &ambiguous, true, 100).is_err());
    assert_eq!(work(), Work::default());
}

#[tokio::test(flavor = "current_thread")]
async fn real_report_preflight_and_both_posts_use_the_bounded_summary_without_rewriting_request() {
    let now = protocol::now_seconds();
    let summary = "\u{1f642}".repeat(256);
    let expected_summary = "\u{1f642}".repeat(20);
    let response_summary = summary.clone();
    let fixture = HttpFixture::new(move |request| match request.path() {
        "/gateway" => {
            assert_eq!(request.json()["api_name"], "/book/getprogress");
            Reply::json(serde_json::json!({
                "bookId":"book","chapterUid":"long","chapterIdx":9,
                "chapterOffset":450,"progress":1,"summary":response_summary,
                "updateTime":now - 1,"device":"cloud-device"
            }))
        }
        "/web/book/chapterInfos" => Reply::json(serde_json::json!({
            "bookId":"book","updated":[
                {"chapterUid":"short","chapterIdx":4,"wordCount":100,"title":"Short"},
                {"chapterUid":"long","chapterIdx":9,"wordCount":900,"title":"Long"}
            ]
        })),
        "/web/book/read" => Reply::json(serde_json::json!({"succ":1,"synckey":"synthetic-ack"})),
        path if path.starts_with("/web/reader/") => Reply::text(format!(
            "<script>window.__INITIAL_STATE__={};</script>",
            serde_json::json!({"reader":{"psvts":"synthetic-clock","pclts":0,"token":"synthetic-token"}}),
        )),
        path => panic!("Unexpected report request: {path}"),
    });
    let session = auth();
    let client = fixture.client(session.clone());
    let mut local = local();
    local.updated_at = now - 10;
    let request = ReadReportRequest {
        session_id: "synthetic-wire-projection".into(),
        account_key: account_key(&session).unwrap(),
        book_id: "book".into(),
        started_at: now - 120,
        seconds: 60,
        progress_write_allowed: true,
        active_reader_intent: false,
        position: Some(local),
    };
    let before = serde_json::to_value(&request).unwrap();
    reset();
    let receipt = client.report_time(&request).await.unwrap();
    assert_eq!(receipt.seconds, 60);
    assert_eq!(work().projections, 2);
    assert_eq!(work().materializations, 1);
    assert_eq!(work().summary_scalars, 20);
    assert_eq!(work().summary_bytes, 80);
    assert_eq!(serde_json::to_value(request).unwrap(), before);
    assert_eq!(client.session(), session);
    let records = fixture.finish();
    assert_eq!(records.len(), 5);
    let writes = records
        .iter()
        .filter(|record| record.path() == "/web/book/read")
        .collect::<Vec<_>>();
    assert_eq!(writes.len(), 2);
    for record in &writes {
        assert_eq!(record.method, "POST");
        let payload = record.json();
        assert_eq!(payload["sm"], expected_summary);
        assert_eq!(payload["ci"], 9);
        assert_eq!(payload["co"], 450);
        assert_eq!(payload["pr"], 55);
        assert!(record.headers.contains_key("cookie"));
        assert!(payload.get("session_id").is_none());
    }
    assert!(writes[0].json().get("rt").is_none());
    assert_eq!(writes[1].json()["rt"], 60);
    assert_eq!(
        records
            .iter()
            .filter(|record| record.path() == "/gateway")
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record.path() == "/web/book/chapterInfos")
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record.path().starts_with("/web/reader/"))
            .count(),
        1
    );
}
