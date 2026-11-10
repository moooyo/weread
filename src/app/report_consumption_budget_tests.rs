//! Independent old-evidence allowances precede derived maps, copies, and sorting.

use super::reporting::{AllocationLimits, with_allocation_test_limits};
use super::reporting::{
    ReportInputCapacity, ReportInputLimits, report_input_test_stats, with_report_input_test_limits,
};
use super::underline_projection_tests::{BOOK, CHAPTER, Fixture, isolated};
use super::*;
use crate::jobs::JobResult;
use crate::source::report::{
    AllocationHttpFixture, AllocationRecordedRequest, AllocationReply, ReadReportReceipt,
};
use serde_json::{Value, json};

const OUTBOX: &str = "report.outbox.v1";
const ACTIVE: &str = "report.active.v1";
const STORAGE_ERROR: &str = "report.storage_error.v1";
const SUMMARY: &str = "Keep old summary \u{4e2d} \u{1f642}. ";

fn auth() -> source::AuthSession {
    source::AuthSession {
        api_key: "synthetic-consumption-key".into(),
        user_vid: "synthetic-consumption-account".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "synthetic-consumption-account".into()),
            ("wr_skey".into(), "synthetic-consumption-session".into()),
        ]),
        ..Default::default()
    }
}

fn seed(app: &App, summary: &str, state: &str) -> Result<(String, Value)> {
    let account = source::report::account_key(&auth()).unwrap();
    let run = "consumption-old-run";
    let id = format!("{}-{run}-0-60", &account[..12]);
    let attempted = state != "pending";
    let batch = json!({"request":{"session_id":id,"account_key":account,"book_id":BOOK,
        "started_at":Utc::now().timestamp()-600,"seconds":60,"progress_write_allowed":true,
        "active_reader_intent":false,"position":{"chapter_uid":CHAPTER,"chapter_index":0,
            "chapter_offset":0,"progress":0,"summary":summary,"updated_at":1,"device":"fixture"}},
        "run_id":run,"from":0,"to":60,"state":state,
        "attempted_at":if attempted { json!(Utc::now().timestamp()) } else { Value::Null },
        "retry_after":0,"attempts":u32::from(attempted),
        "error":if state=="uncertain" { json!("source.delivery_uncertain") } else { Value::Null }});
    let mut cursors = serde_json::Map::new();
    cursors.insert(run.to_owned(), json!(0));
    let queue = json!({"batches":[batch],"reported_cursors":cursors,"compacted_runs":{}});
    app.store.set_setting(OUTBOX, &queue)?;
    app.store.set_setting(
        ACTIVE,
        &json!({"id":"consumption-active","account_key":account,"book_id":BOOK,
        "started_at":100,"observed_seconds":125,"eligible_seconds":125,"allocated_seconds":0,
        "last_checkpoint_at":225,"local_chapter":0,"local_anchor":0,"position":null}),
    )?;
    Ok((id, queue))
}

fn snapshot(app: &App) -> Result<Value> {
    Ok(app.store.setting(OUTBOX)?.unwrap())
}

fn refuse_before_derived() -> Result<()> {
    let fixture = Fixture::new("Keep consumption body", Vec::new(), false)?;
    {
        let mut app = fixture.app();
        let (_, original) = seed(&app, &SUMMARY.repeat(64), "pending")?;
        for limits in [
            ReportInputLimits {
                metadata_bytes: 0,
                ..Default::default()
            },
            ReportInputLimits {
                scratch_bytes: 0,
                ..Default::default()
            },
            ReportInputLimits {
                work_bytes: 0,
                ..Default::default()
            },
        ] {
            with_report_input_test_limits(limits, || -> Result<()> {
                let error = app.validate_report_input_for_test().unwrap_err();
                assert!(error.downcast_ref::<ReportInputCapacity>().is_some());
                let stats = report_input_test_stats();
                assert_eq!(stats.validation_builds, 0);
                assert_eq!(stats.interval_builds, 0);
                assert_eq!(stats.compaction_builds, 0);
                assert_eq!(stats.old_request_copies, 0);
                assert_eq!(stats.persistence_encodes, 0);
                app.note_read_report_error(&error);
                assert_ne!(app.store.setting::<bool>(STORAGE_ERROR)?, Some(true));
                assert_eq!(snapshot(&app)?, original);
                assert_eq!(
                    app.store.setting::<Value>(ACTIVE)?.unwrap()["eligible_seconds"],
                    125
                );
                Ok(())
            })?;
        }
    }
    fixture.finish()
}

fn results_only_headers() -> Result<()> {
    let fixture = Fixture::new("Keep consumption body", Vec::new(), false)?;
    {
        let app = fixture.app();
        let (id, original) = seed(&app, &SUMMARY.repeat(4096), "uncertain")?;
        app.store
            .set_setting(&format!("report.uncertain:{id}"), &true)?;
        let admitted =
            with_report_input_test_limits(ReportInputLimits::default(), || -> Result<_> {
                app.reconcile_report_input_for_test()?;
                let stats = report_input_test_stats();
                assert_eq!(stats.result_headers, 1);
                assert!(stats.result_header_bytes < 1024);
                assert_eq!(stats.old_request_copies, 0);
                assert_eq!(stats.transport_copies, 0);
                assert_eq!(snapshot(&app)?, original);
                Ok(stats)
            })?;
        for limits in [
            ReportInputLimits {
                metadata_bytes: admitted.metadata_bytes - 1,
                ..Default::default()
            },
            ReportInputLimits {
                scratch_bytes: admitted.scratch_bytes - 1,
                ..Default::default()
            },
            ReportInputLimits {
                work_bytes: admitted.work_bytes - 1,
                ..Default::default()
            },
        ] {
            with_report_input_test_limits(limits, || -> Result<()> {
                let error = app.reconcile_report_input_for_test().unwrap_err();
                assert!(error.downcast_ref::<ReportInputCapacity>().is_some());
                assert_eq!(report_input_test_stats().result_headers, 0);
                assert_eq!(report_input_test_stats().old_request_copies, 0);
                assert_eq!(snapshot(&app)?, original);
                Ok(())
            })?;
        }
    }
    fixture.finish()
}

fn compaction_scratch() -> Result<()> {
    let fixture = Fixture::new("Keep consumption body", Vec::new(), false)?;
    {
        let app = fixture.app();
        let (_, original) = seed(&app, &SUMMARY.repeat(4096), "uncertain")?;
        let validation =
            with_report_input_test_limits(ReportInputLimits::default(), || -> Result<_> {
                app.validate_report_input_for_test()?;
                Ok(report_input_test_stats())
            })?;
        with_report_input_test_limits(
            ReportInputLimits {
                scratch_bytes: validation.scratch_bytes,
                ..Default::default()
            },
            || -> Result<()> {
                let error = app.compact_report_input_for_test().unwrap_err();
                assert!(error.downcast_ref::<ReportInputCapacity>().is_some());
                assert_eq!(report_input_test_stats().compaction_builds, 0);
                assert_eq!(report_input_test_stats().old_request_copies, 0);
                assert_eq!(report_input_test_stats().persistence_encodes, 0);
                assert_eq!(snapshot(&app)?, original);
                Ok(())
            },
        )?;
        with_report_input_test_limits(ReportInputLimits::default(), || -> Result<()> {
            app.compact_report_input_for_test()?;
            let stats = report_input_test_stats();
            assert_eq!(stats.old_request_copies, 1);
            assert!(stats.old_request_bytes >= SUMMARY.len() * 4096);
            assert_eq!(snapshot(&app)?, original);
            Ok(())
        })?;
    }
    fixture.finish()
}

fn invalid_proof() -> Result<()> {
    let fixture = Fixture::new("Keep consumption body", Vec::new(), false)?;
    {
        let app = fixture.app();
        let (_, mut original) = seed(&app, SUMMARY, "uncertain")?;
        original["reported_cursors"]["consumption-old-run"] = json!(60);
        app.store.set_setting(OUTBOX, &original)?;
        with_report_input_test_limits(ReportInputLimits::default(), || -> Result<()> {
            let error = app.compact_report_input_for_test().unwrap_err();
            assert!(error.downcast_ref::<ReportInputCapacity>().is_none());
            assert!(format!("{error:#}").contains("reported cursors"));
            assert_eq!(report_input_test_stats().old_request_copies, 0);
            assert_eq!(report_input_test_stats().persistence_encodes, 0);
            assert_eq!(snapshot(&app)?, original);
            Ok(())
        })?;
        // Restore the valid original only after the invariant rejection assertions.
        original["reported_cursors"]["consumption-old-run"] = json!(0);
        app.store.set_setting(OUTBOX, &original)?;
    }
    fixture.finish()
}

fn proof_only() -> Result<()> {
    let fixture = Fixture::new("Keep consumption body", Vec::new(), false)?;
    {
        let app = fixture.app();
        let account = source::report::account_key(&auth()).unwrap();
        let mut cursors = serde_json::Map::new();
        let mut proofs = serde_json::Map::new();
        for index in 0..161 {
            let id = format!("input-proof-{index}");
            cursors.insert(id.clone(), json!(60));
            proofs.insert(
                id,
                json!({"account_key":account,"book_id":BOOK,"started_at":100+index,
                "confirmed_until":60,"confirmed_ranges":[]}),
            );
        }
        let original = json!({"batches":[],"reported_cursors":cursors,"compacted_runs":proofs});
        app.store.set_setting(OUTBOX, &original)?;
        with_report_input_test_limits(ReportInputLimits::default(), || -> Result<()> {
            app.compact_report_input_for_test()?;
            assert!(
                report_input_test_stats().work_bytes <= ReportInputLimits::default().work_bytes
            );
            assert_eq!(report_input_test_stats().old_request_copies, 0);
            assert_eq!(snapshot(&app)?, original);
            Ok(())
        })?;
    }
    fixture.finish()
}

fn pending_long_summary() -> Result<()> {
    let fixture = Fixture::new("Keep consumption body", Vec::new(), false)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let database;
    let id;
    let summary = SUMMARY.repeat(4096);
    let summary_bytes = summary.len();
    {
        let mut app = fixture.app();
        app.demo = false;
        app.force_offline = true;
        app.reader.session_started = None;
        app.route = Route::Settings;
        let mut book = Book::new(BOOK, "Kept old report book");
        book.source = "weread".into();
        app.store.upsert_book(&book)?;
        (id, _) = seed(&app, &summary, "pending")?;
        app.store.delete_setting(ACTIVE)?;
        database = app.data_dir.join("library.sqlite");
    }
    let http = AllocationHttpFixture::new(move |request: &AllocationRecordedRequest| match request
        .path()
    {
        "/gateway" => {
            AllocationReply::json(json!({"bookId":BOOK,"progress":0,"chapterUid":CHAPTER,
            "chapterIdx":0,"chapterOffset":0,"updatedAt":1,"chapterTitle":"Cloud"}))
        }
        "/web/book/chapterInfos" => {
            AllocationReply::json(json!({"data":[{"bookId":BOOK,"updated":[{
                "chapterUid":CHAPTER,"chapterIdx":0,"wordCount":20,"title":"Kept chapter"
            }]}]}))
        }
        path if path.starts_with("/web/reader/") => AllocationReply::text(
            "<script>window.__INITIAL_STATE__={\"reader\":{\"psvts\":\"synthetic-input-clock\",\"pclts\":0}};</script>",
        ),
        "/web/book/read" => {
            let connection = rusqlite::Connection::open(&database).unwrap();
            let encoded: String = connection
                .query_row(
                    "SELECT value FROM settings WHERE key='report.outbox.v1'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let queue: Value = serde_json::from_str(&encoded).unwrap();
            let batch = &queue["batches"][0];
            assert_eq!(batch["state"], "sending");
            assert_eq!(
                batch["request"]["position"]["summary"]
                    .as_str()
                    .unwrap()
                    .len(),
                summary_bytes
            );
            let payload = request.json();
            assert!(payload["sm"].as_str().unwrap().chars().count() <= 20);
            if let Some(seconds) = payload["rt"].as_u64() {
                assert_eq!(seconds, 60);
            }
            AllocationReply::json(json!({"succ":1,"synckey":"synthetic-input-receipt"}))
        }
        path => panic!("Unexpected old-input fixture path: {path}"),
    });
    {
        let mut app = fixture.app();
        let client = {
            let _context = runtime.enter();
            http.client(auth())
        };
        let (sender, receiver) = mpsc::channel();
        app.jobs = Jobs::new(runtime.handle().clone(), sender, Some(client))?;
        app.events = receiver;
        app.force_offline = false;
        app.online = true;
        app.prefs.report_time = true;
        app.prefs.push_on_close = true;
        app.prefs.report_reader_only = false;
        app.store.set_setting("report.next_check.v1", &0_i64)?;
        with_allocation_test_limits(
            AllocationLimits {
                batches: 0,
                metadata_bytes: 32,
                encoded_bytes: 32,
                validation_work: 32,
                ..Default::default()
            },
            || app.tick_read_reports(),
        )?;
        let event = runtime.block_on(async {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if let Ok(event) = app.events.try_recv() {
                    break event;
                }
                assert!(
                    Instant::now() < deadline,
                    "The old Pending report did not finish"
                );
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });
        assert!(matches!(
            &event.result,
            JobResult::ReadReport { result: Ok(_), .. }
        ));
        with_report_input_test_limits(
            ReportInputLimits {
                scratch_bytes: 0,
                ..Default::default()
            },
            || -> Result<()> {
                app.handle_job(event)?;
                assert_eq!(report_input_test_stats().validation_builds, 0);
                assert_eq!(report_input_test_stats().old_request_copies, 0);
                assert_eq!(report_input_test_stats().persistence_encodes, 0);
                assert_eq!(
                    app.store
                        .setting::<ReadReportReceipt>(&format!("report.receipt:{id}"))?
                        .unwrap()
                        .seconds,
                    60
                );
                assert_eq!(snapshot(&app)?["batches"][0]["state"], "sending");
                app.tick_read_reports()?;
                assert_ne!(app.store.setting::<bool>(STORAGE_ERROR)?, Some(true));
                Ok(())
            },
        )?;
        app.store.set_setting("report.next_check.v1", &0_i64)?;
        app.tick_read_reports()?;
        assert!(snapshot(&app)?["batches"].as_array().unwrap().is_empty());
        assert_eq!(
            snapshot(&app)?["reported_cursors"]["consumption-old-run"],
            60
        );
        assert_eq!(
            app.store.chapter(BOOK, 0)?.unwrap().body.as_deref(),
            Some("Keep consumption body")
        );
        assert_eq!(app.jobs.session().unwrap(), auth());
        for _ in 0..3 {
            app.tick_read_reports()?;
        }
    }
    let records = http.finish();
    assert_eq!(
        records
            .iter()
            .filter(
                |request| request.path() == "/web/book/read" && request.json().get("rt").is_some()
            )
            .count(),
        1
    );
    assert_eq!(records.len(), 5);
    fixture.finish()?;
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

fn cleanup_prepaid() -> Result<()> {
    let fixture = Fixture::new("Keep consumption body", Vec::new(), false)?;
    {
        let app = fixture.app();
        let account = source::report::account_key(&auth()).unwrap();
        let mut rows = Vec::new();
        let mut cursors = serde_json::Map::new();
        let mut ids = Vec::new();
        for index in 0..4 {
            let run = format!("{index}{}", "r".repeat(449));
            let id = format!("{}-{run}-0-60", &account[..12]);
            ids.push(id.clone());
            cursors.insert(run.clone(), json!(60));
            rows.push(json!({"request":{"session_id":id,"account_key":account,"book_id":"book",
                "started_at":100,"seconds":60,"progress_write_allowed":true,"active_reader_intent":false,
                "position":{"chapter_uid":"u","chapter_index":0,"chapter_offset":0,"progress":0,
                    "summary":"","updated_at":0,"device":"device"}},"run_id":run,"from":0,"to":60,
                "state":"confirmed","attempted_at":100,"retry_after":0,"attempts":1,"error":null}));
        }
        let original = json!({"batches":rows,"reported_cursors":cursors,"compacted_runs":{}});
        app.store.set_setting(OUTBOX, &original)?;
        for id in &ids {
            for prefix in [
                "report.receipt",
                "report.error",
                "report.failure",
                "report.retryable",
                "report.requires_login",
                "report.await_auth",
                "report.uncertain",
            ] {
                app.store
                    .set_setting(&format!("{prefix}:{id}"), &json!({"keep":true}))?;
            }
        }
        let scratch =
            with_report_input_test_limits(ReportInputLimits::default(), || -> Result<_> {
                app.compact_report_input_for_test()?;
                Ok(report_input_test_stats().peak_scratch_bytes)
            })?;
        with_report_input_test_limits(
            ReportInputLimits {
                scratch_bytes: scratch - 1,
                ..Default::default()
            },
            || -> Result<()> {
                let error = app.persist_report_input_for_test().unwrap_err();
                assert!(error.downcast_ref::<ReportInputCapacity>().is_some());
                assert_eq!(report_input_test_stats().old_request_copies, 0);
                assert_eq!(report_input_test_stats().persistence_encodes, 0);
                assert_eq!(snapshot(&app)?, original);
                assert!(
                    app.store
                        .setting::<Value>(&format!("report.receipt:{}", ids[0]))?
                        .is_some()
                );
                Ok(())
            },
        )?;
        with_report_input_test_limits(
            ReportInputLimits {
                scratch_bytes: scratch,
                ..Default::default()
            },
            || -> Result<()> {
                app.persist_report_input_for_test()?;
                let stats = report_input_test_stats();
                assert_eq!(stats.persistence_encodes, 1);
                assert!(stats.persistence_bytes > 0);
                Ok(())
            },
        )?;
        assert_eq!(
            snapshot(&app)?["compacted_runs"].as_object().unwrap().len(),
            4
        );
        for id in ids {
            for prefix in [
                "report.receipt",
                "report.error",
                "report.failure",
                "report.retryable",
                "report.requires_login",
                "report.await_auth",
                "report.uncertain",
            ] {
                assert!(
                    app.store
                        .setting::<Value>(&format!("{prefix}:{id}"))?
                        .is_none()
                );
            }
        }
    }
    fixture.finish()
}

fn terminal_headroom() -> Result<()> {
    let fixture = Fixture::new("Keep consumption body", Vec::new(), false)?;
    {
        let mut app = fixture.app();
        let (_, mut original) = seed(&app, &SUMMARY.repeat(16), "pending")?;
        let id = "legacy-noncanonical-delivery";
        original["batches"][0]["request"]["session_id"] = json!(id);
        app.store.set_setting(OUTBOX, &original)?;
        app.store.delete_setting(ACTIVE)?;
        let encoded = serde_json::to_string(&original)?.len();
        let reserve = 64;
        with_report_input_test_limits(
            ReportInputLimits {
                encoded_bytes: encoded + reserve - 1,
                terminal_headroom: reserve,
                ..Default::default()
            },
            || -> Result<()> {
                let error = app.persist_report_sending_for_test(id).unwrap_err();
                assert!(error.downcast_ref::<ReportInputCapacity>().is_some());
                assert_eq!(report_input_test_stats().transport_copies, 0);
                assert_eq!(report_input_test_stats().persistence_encodes, 0);
                assert_eq!(snapshot(&app)?, original);
                Ok(())
            },
        )?;
        with_report_input_test_limits(
            ReportInputLimits {
                encoded_bytes: encoded + reserve,
                terminal_headroom: reserve,
                ..Default::default()
            },
            || -> Result<()> {
                app.persist_report_sending_for_test(id)?;
                super::reporting::store_report_result(
                    &app.store,
                    id,
                    &Ok(ReadReportReceipt {
                        accepted_at: Utc::now().timestamp(),
                        seconds: 60,
                        has_synckey: true,
                    }),
                )?;
                app.recover_read_reports()?;
                let saved = snapshot(&app)?;
                assert_eq!(saved["batches"][0]["state"], "confirmed");
                assert_eq!(
                    saved["batches"][0]["request"]["position"]["summary"],
                    original["batches"][0]["request"]["position"]["summary"]
                );
                assert_eq!(saved["reported_cursors"]["consumption-old-run"], 60);
                Ok(())
            },
        )?;
    }
    fixture.finish()
}

#[test]
fn old_input_quota_refuses_before_maps_intervals_or_request_copies() -> Result<()> {
    isolated(
        "app::report_consumption_budget_tests::old_input_quota_refuses_before_maps_intervals_or_request_copies",
        refuse_before_derived,
    )
}
#[test]
fn old_result_reconciliation_copies_only_delivery_headers() -> Result<()> {
    isolated(
        "app::report_consumption_budget_tests::old_result_reconciliation_copies_only_delivery_headers",
        results_only_headers,
    )
}
#[test]
fn old_compaction_scratch_refusal_preserves_long_summary_and_unresolved_proof() -> Result<()> {
    isolated(
        "app::report_consumption_budget_tests::old_compaction_scratch_refusal_preserves_long_summary_and_unresolved_proof",
        compaction_scratch,
    )
}
#[test]
fn old_invalid_proof_keeps_its_cause_and_never_becomes_capacity_or_empty() -> Result<()> {
    isolated(
        "app::report_consumption_budget_tests::old_invalid_proof_keeps_its_cause_and_never_becomes_capacity_or_empty",
        invalid_proof,
    )
}
#[test]
fn old_confirmed_only_input_has_no_unconditional_quadratic_proof_fee() -> Result<()> {
    isolated(
        "app::report_consumption_budget_tests::old_confirmed_only_input_has_no_unconditional_quadratic_proof_fee",
        proof_only,
    )
}
#[test]
fn old_pending_long_summary_sends_under_zero_new_allocation_and_retains_receipt_under_input_refusal()
-> Result<()> {
    isolated(
        "app::report_consumption_budget_tests::old_pending_long_summary_sends_under_zero_new_allocation_and_retains_receipt_under_input_refusal",
        pending_long_summary,
    )
}
#[test]
fn old_canonical_cleanup_keys_and_containers_are_prepaid_before_copy_or_commit() -> Result<()> {
    isolated(
        "app::report_consumption_budget_tests::old_canonical_cleanup_keys_and_containers_are_prepaid_before_copy_or_commit",
        cleanup_prepaid,
    )
}
#[test]
fn old_legacy_pending_reserves_terminal_header_before_sending_and_keeps_raw_summary() -> Result<()>
{
    isolated(
        "app::report_consumption_budget_tests::old_legacy_pending_reserves_terminal_header_before_sending_and_keeps_raw_summary",
        terminal_headroom,
    )
}
