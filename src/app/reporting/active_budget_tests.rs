//! Stored run admission precedes DTO copies and every maintenance write.

use super::*;
use crate::app::underline_projection_tests::{BOOK, CHAPTER, Fixture, isolated};
use crate::jobs::{JobResult, Jobs};
use crate::source::report::{AllocationHttpFixture, AllocationRecordedRequest, AllocationReply};
use serde_json::{Value, json};

const BODY: &str = "Keep the original active-run body.";
const SUMMARY: &str = "Legacy active summary \u{4e2d}\u{1f642} ";
const DEVICE: &str = "Legacy active device \u{0085} ";

fn auth(vid: &str) -> source::AuthSession {
    source::AuthSession {
        api_key: "synthetic-active-budget-key".into(),
        user_vid: vid.into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), vid.into()),
            ("wr_skey".into(), "synthetic-active-budget-session".into()),
        ]),
        ..Default::default()
    }
}

fn limits<T>(limits: ActiveInputLimits, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<ActiveInputLimits>, Option<ActiveInputStats>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE_INPUT_LIMITS.with(|slot| slot.set(self.0));
            ACTIVE_INPUT_STATS.with(|slot| slot.set(self.1));
        }
    }
    let _restore = Restore(
        ACTIVE_INPUT_LIMITS.with(|slot| slot.replace(Some(limits.tightened()))),
        ACTIVE_INPUT_STATS.with(|slot| slot.replace(Some(ActiveInputStats::default()))),
    );
    run()
}

fn stats() -> ActiveInputStats {
    ACTIVE_INPUT_STATS.with(|slot| slot.get().unwrap_or_default())
}

fn raw(app: &App, key: &str) -> Result<Option<String>> {
    let connection = rusqlite::Connection::open(app.data_dir.join("library.sqlite"))?;
    use rusqlite::OptionalExtension;
    Ok(connection
        .query_row("SELECT value FROM settings WHERE key=?1", [key], |row| {
            row.get(0)
        })
        .optional()?)
}

fn seed(app: &mut App, summary: &str, device: &str) -> Result<Value> {
    app.demo = false;
    app.force_offline = true;
    app.online = false;
    app.prefs.report_time = true;
    app.prefs.push_on_close = false;
    let session = auth("synthetic-active-budget-a");
    app.jobs.replace_session(session.clone())?;
    let account = source::report::account_key(&session).unwrap();
    let mut book = Book::new(BOOK, "Keep active-run book");
    book.source = "weread".into();
    book.intro = "Keep active-run introduction.".into();
    book.total_chapters = 1;
    app.store.upsert_book(&book)?;
    let owner = hex::encode(Sha256::digest(
        format!("weread-download-vid:{}", session.user_vid).as_bytes(),
    ));
    app.store.upsert_chapter(&Chapter {
        id: CHAPTER.into(),
        book_id: BOOK.into(),
        index: 0,
        title: "Keep active-run chapter".into(),
        cached: true,
        body: Some(BODY.into()),
        content_status: ContentStatus::Full,
        content_account_key: owner,
    })?;
    app.store.save_annotation(&Annotation {
        id: "active-budget-note".into(),
        book_id: BOOK.into(),
        start: 0,
        end: 4,
        quote: "Keep".into(),
        note: "Keep active-run note".into(),
        ..Default::default()
    })?;
    let started = Utc::now().timestamp() - 600;
    app.reader.book_id = BOOK.into();
    app.reader.chapter_index = 0;
    app.reader.body = BODY.into();
    app.reader.session_started = Some(started);
    app.reader.session_seconds = 60;
    app.reader.position_confirmed = true;
    app.reader.last_active = Instant::now();
    app.reader.selection = Some((0, 4));
    assert!(app.reader.draft.set_query("Keep active-run draft"));
    app.route = Route::Reader(BOOK.into(), 0);
    let anchor = app
        .reader
        .paginator
        .as_ref()
        .map_or(0, |value| value.current_page().start);
    let value = json!({"id":"active-budget-run","account_key":account,"book_id":BOOK,
        "started_at":started,"observed_seconds":60,"eligible_seconds":60,"allocated_seconds":0,
        "last_checkpoint_at":started+60,"local_chapter":0,"local_anchor":anchor,
        "position":{"chapter_uid":CHAPTER,"chapter_index":0,"chapter_offset":0,"progress":0,
            "summary":summary,"updated_at":1,"device":device},
        "observer_started_at":started,"observer_seconds_base":0,"local_session_id":"active-budget-local"});
    app.store.set_setting(ACTIVE_KEY, &value)?;
    Ok(value)
}

fn kept(app: &App, session: &Option<source::AuthSession>) -> Result<()> {
    assert_eq!(&app.jobs.session(), session);
    assert_eq!(app.reader.body, BODY);
    assert_eq!(app.reader.draft.query, "Keep active-run draft");
    assert_eq!(app.reader.selection, Some((0, 4)));
    assert_eq!(app.store.chapter_body(BOOK, 0)?.as_deref(), Some(BODY));
    let note = app
        .store
        .annotation_by_id(BOOK, "active-budget-note")?
        .unwrap();
    assert_eq!(
        (note.quote.as_str(), note.note.as_str()),
        ("Keep", "Keep active-run note")
    );
    Ok(())
}

fn rejected_before_dto() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        seed(&mut app, &SUMMARY.repeat(1024), &DEVICE.repeat(1024))?;
        let original = raw(&app, ACTIVE_KEY)?.unwrap();
        let session = app.jobs.session();
        for budget in [
            ActiveInputLimits {
                encoded_bytes: 0,
                ..Default::default()
            },
            ActiveInputLimits {
                metadata_bytes: 0,
                ..Default::default()
            },
            ActiveInputLimits {
                scratch_bytes: 0,
                ..Default::default()
            },
            ActiveInputLimits {
                work_bytes: 0,
                ..Default::default()
            },
        ] {
            limits(budget, || -> Result<()> {
                let error = app.checkpoint_read_reports(true).unwrap_err();
                assert!(report_capacity(&error));
                assert_eq!(
                    (
                        stats().full_deserializations,
                        stats().encode_calls,
                        stats().write_attempts
                    ),
                    (0, 0, 0)
                );
                assert_eq!(raw(&app, ACTIVE_KEY)?.as_deref(), Some(original.as_str()));
                assert_eq!(
                    app.store.setting::<Value>(ACTIVE_KEY)?.unwrap()["eligible_seconds"],
                    60
                );
                app.note_read_report_error(&error);
                assert_ne!(app.store.setting::<bool>(STORAGE_ERROR_KEY)?, Some(true));
                kept(&app, &session)?;
                Ok(())
            })?;
        }
    }
    fixture.finish()
}

fn idle_headers() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let http = AllocationHttpFixture::new(|request: &AllocationRecordedRequest| {
        panic!(
            "Idle report inspection unexpectedly dispatched HTTP: {}",
            request.path()
        )
    });
    {
        let mut app = fixture.app();
        seed(&mut app, &SUMMARY.repeat(1024), &DEVICE.repeat(1024))?;
        let client = {
            let _context = runtime.enter();
            http.client(auth("synthetic-active-budget-a"))
        };
        let (sender, receiver) = mpsc::channel();
        app.jobs = Jobs::new(runtime.handle().clone(), sender, Some(client))?;
        app.events = receiver;
        app.prefs.push_on_close = true;
        assert!(app.jobs.capabilities().remote_progress);
        assert!(app.force_offline && !app.online);
        let original = raw(&app, ACTIVE_KEY)?.unwrap();
        limits(ActiveInputLimits::default(), || -> Result<()> {
            for _ in 0..3 {
                app.checkpoint_read_reports(false)?;
            }
            assert!(app.read_report_summary().contains('+'));
            assert_eq!(
                app.active_read_report_session_id().as_deref(),
                Some("active-budget-local")
            );
            let observed = stats();
            assert!(observed.header_deserializations >= 5);
            assert_eq!(
                (
                    observed.full_deserializations,
                    observed.encode_calls,
                    observed.write_attempts
                ),
                (0, 0, 0)
            );
            assert!(observed.selected_string_bytes < 4096);
            assert_eq!(raw(&app, ACTIVE_KEY)?.as_deref(), Some(original.as_str()));
            Ok(())
        })?;
    }
    assert!(
        http.finish().is_empty(),
        "Idle header inspection must not send a request"
    );
    fixture.finish()?;
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

fn utf16_exact_working_fee() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    let directory = tempfile::tempdir()?;
    let database = directory.path().join("library.sqlite");
    let connection = rusqlite::Connection::open(&database)?;
    connection
        .execute_batch("PRAGMA encoding='UTF-16le'; CREATE TABLE encoding_stamp(value TEXT)")?;
    drop(connection);
    {
        let mut app = fixture.app();
        app.store = Store::open(&database)?;
        app.data_dir = directory.path().to_path_buf();
        seed(
            &mut app,
            &format!("{} quoted \\\" legacy", SUMMARY.repeat(64)),
            &DEVICE.repeat(64),
        )?;
        let original = raw(&app, ACTIVE_KEY)?.unwrap();
        let demand = limits(
            ActiveInputLimits::default(),
            || -> Result<ActiveInputStats> {
                let active = read_active_run(&app.store, ACTIVE_KEY, false)?.unwrap();
                assert!(active.position.unwrap().summary.contains("quoted"));
                assert_eq!(stats().full_deserializations, 1);
                Ok(stats())
            },
        )?;
        assert!(demand.scratch_demand > demand.metadata_demand);
        for budget in [
            ActiveInputLimits {
                encoded_bytes: demand.encoded_demand - 1,
                ..Default::default()
            },
            ActiveInputLimits {
                metadata_bytes: demand.metadata_demand - 1,
                ..Default::default()
            },
            ActiveInputLimits {
                scratch_bytes: demand.scratch_demand - 1,
                ..Default::default()
            },
            ActiveInputLimits {
                work_bytes: demand.work_demand - 1,
                ..Default::default()
            },
            ActiveInputLimits {
                nodes: 1,
                ..Default::default()
            },
            ActiveInputLimits {
                depth: 0,
                ..Default::default()
            },
        ] {
            limits(budget, || -> Result<()> {
                let error = read_active_run(&app.store, ACTIVE_KEY, false).unwrap_err();
                assert!(report_capacity(&error));
                assert_eq!(stats().full_deserializations, 0);
                assert_eq!((stats().encode_calls, stats().write_attempts), (0, 0));
                assert_eq!(raw(&app, ACTIVE_KEY)?.as_deref(), Some(original.as_str()));
                Ok(())
            })?;
        }
        limits(
            ActiveInputLimits {
                encoded_bytes: demand.encoded_demand,
                metadata_bytes: demand.metadata_demand,
                scratch_bytes: demand.scratch_demand,
                work_bytes: demand.work_demand,
                ..Default::default()
            },
            || -> Result<()> {
                assert!(read_active_run(&app.store, ACTIVE_KEY, false)?.is_some());
                assert_eq!(stats().full_deserializations, 1);
                Ok(())
            },
        )?;
    }
    fixture.finish()
}

fn checkpoint_preserves_cold_fields() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        let old = seed(&mut app, &SUMMARY.repeat(1024), &DEVICE.repeat(1024))?;
        let session = app.jobs.session();
        app.reader.session_seconds = 120;
        limits(ActiveInputLimits::default(), || -> Result<()> {
            let error = outbox::with_allocation_test_limits(
                outbox::AllocationLimits {
                    batches: 0,
                    ..Default::default()
                },
                || app.checkpoint_read_reports(true),
            )
            .unwrap_err();
            assert!(error.downcast_ref::<outbox::AllocationCapacity>().is_some());
            let current = app.store.setting::<Value>(ACTIVE_KEY)?.unwrap();
            assert_eq!(
                (
                    current["observed_seconds"].as_u64(),
                    current["eligible_seconds"].as_u64(),
                    current["allocated_seconds"].as_u64()
                ),
                (Some(120), Some(120), Some(0))
            );
            assert_eq!(current["position"]["summary"], old["position"]["summary"]);
            assert_eq!(current["position"]["device"], old["position"]["device"]);
            assert!(
                stats().full_deserializations > 0
                    && stats().encode_calls > 0
                    && stats().write_commits > 0
            );
            kept(&app, &session)?;
            Ok(())
        })?;
    }
    fixture.finish()
}

fn malformed_before_copy() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        let original = seed(&mut app, SUMMARY, DEVICE)?;
        let mut missing = original.clone();
        missing.as_object_mut().unwrap().remove("eligible_seconds");
        let mut wrong = original.clone();
        wrong["position"]["device"] = json!(17);
        let mut overflow = original.clone();
        overflow["position"]["chapter_offset"] = json!(u64::MAX);
        for value in [missing, wrong, overflow] {
            app.store.set_setting(ACTIVE_KEY, &value)?;
            let before = raw(&app, ACTIVE_KEY)?.unwrap();
            limits(ActiveInputLimits::default(), || -> Result<()> {
                let error = read_active_run(&app.store, ACTIVE_KEY, false).unwrap_err();
                assert!(!report_capacity(&error));
                assert_eq!(
                    (
                        stats().full_deserializations,
                        stats().header_deserializations,
                        stats().write_attempts
                    ),
                    (0, 0, 0)
                );
                assert_eq!(raw(&app, ACTIVE_KEY)?.as_deref(), Some(before.as_str()));
                Ok(())
            })?;
        }
        app.store.set_setting(ACTIVE_KEY, &original)?;
    }
    fixture.finish()
}

fn deferred_and_marker() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        let active = seed(&mut app, &SUMMARY.repeat(1024), &DEVICE.repeat(1024))?;
        let account = active["account_key"].as_str().unwrap();
        let key = format!("{ALLOCATION_DEBT_PREFIX}{account}:active-budget-run");
        app.store.set_setting(&key, &json!({"run":active}))?;
        app.store.delete_setting(ACTIVE_KEY)?;
        app.reader.session_started = None;
        app.route = Route::Settings;
        let original = raw(&app, &key)?.unwrap();
        limits(
            ActiveInputLimits {
                metadata_bytes: 0,
                ..Default::default()
            },
            || -> Result<()> {
                let error = app.advance_deferred_report_allocation().unwrap_err();
                assert!(report_capacity(&error));
                assert_eq!(stats().full_deserializations, 0);
                assert_eq!(raw(&app, &key)?.as_deref(), Some(original.as_str()));
                assert_eq!(
                    app.store
                        .setting::<String>(ALLOCATION_DEBT_CURSOR_KEY)?
                        .as_deref(),
                    Some(key.as_str())
                );
                Ok(())
            },
        )?;
        app.store
            .set_setting(ALLOCATION_DEBT_CURSOR_KEY, &"x".repeat(8193))?;
        let marker = raw(&app, ALLOCATION_DEBT_CURSOR_KEY)?.unwrap();
        assert!(
            read_debt_marker(&app.store)
                .unwrap_err()
                .downcast_ref::<ActiveInputCapacity>()
                .is_some()
        );
        assert_eq!(
            raw(&app, ALLOCATION_DEBT_CURSOR_KEY)?.as_deref(),
            Some(marker.as_str())
        );
        app.store.set_setting(ALLOCATION_DEBT_CURSOR_KEY, &key)?;
    }
    fixture.finish()
}

fn archive_atomic() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        seed(&mut app, SUMMARY, DEVICE)?;
        let active = read_active_run(&app.store, ACTIVE_KEY, false)?.unwrap();
        let before = raw(&app, ACTIVE_KEY)?.unwrap();
        let debt = format!(
            "{ALLOCATION_DEBT_PREFIX}{}:{}",
            active.account_key, active.id
        );
        let next = ActiveRun {
            id: "active-budget-successor".into(),
            account_key: "b".repeat(64),
            book_id: BOOK.into(),
            started_at: Utc::now().timestamp(),
            last_checkpoint_at: Utc::now().timestamp(),
            observer_started_at: active.started_at,
            observer_seconds_base: 60,
            local_session_id: active.local_session_id.clone(),
            ..Default::default()
        };
        let connection = rusqlite::Connection::open(app.data_dir.join("library.sqlite"))?;
        connection.execute_batch("CREATE TRIGGER reject_active_debt BEFORE INSERT ON settings WHEN NEW.key GLOB 'report.allocation_debt.v1:*' BEGIN SELECT RAISE(FAIL,'active budget fixture commit failure'); END")?;
        let error = app.archive_report_run(&active, Some(&next)).unwrap_err();
        assert!(!report_capacity(&error));
        assert!(format!("{error:#}").contains("active budget fixture commit failure"));
        assert_eq!(raw(&app, ACTIVE_KEY)?.as_deref(), Some(before.as_str()));
        assert!(raw(&app, &debt)?.is_none());
        connection.execute_batch("DROP TRIGGER reject_active_debt")?;
        app.archive_report_run(&active, Some(&next))?;
        assert_eq!(
            app.store.setting::<Value>(&debt)?.unwrap()["run"]["eligible_seconds"],
            60
        );
        let saved = app.store.setting::<Value>(ACTIVE_KEY)?.unwrap();
        assert_eq!(saved["id"], "active-budget-successor");
        assert_eq!(saved["observer_seconds_base"], 60);
        app.reader.session_started = None;
        app.route = Route::Settings;
    }
    fixture.finish()
}

fn capacity_recovery_and_new_context() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        seed(&mut app, &SUMMARY.repeat(1024), &DEVICE.repeat(1024))?;
        let original = raw(&app, ACTIVE_KEY)?.unwrap();
        limits(
            ActiveInputLimits {
                metadata_bytes: 0,
                ..Default::default()
            },
            || -> Result<()> {
                app.recover_read_reports()?;
                assert_eq!(raw(&app, ACTIVE_KEY)?.as_deref(), Some(original.as_str()));
                assert_ne!(app.store.setting::<bool>(STORAGE_ERROR_KEY)?, Some(true));
                assert_eq!(stats().full_deserializations, 0);
                app.reader.book_id = "new-active-budget-book".into();
                let mut book = Book::new("new-active-budget-book", "New context");
                book.source = "weread".into();
                app.store.upsert_book(&book)?;
                let error = app.start_read_report_session().unwrap_err();
                assert!(report_capacity(&error));
                assert_eq!(raw(&app, ACTIVE_KEY)?.as_deref(), Some(original.as_str()));
                Ok(())
            },
        )?;
        app.reader.book_id = BOOK.into();
    }
    fixture.finish()
}

fn pending_http() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let active_original;
    let pending_id;
    let database;
    {
        let mut app = fixture.app();
        let active = seed(&mut app, &SUMMARY.repeat(1024), &DEVICE.repeat(1024))?;
        active_original = raw(&app, ACTIVE_KEY)?.unwrap();
        database = app.data_dir.join("library.sqlite");
        let account = active["account_key"].as_str().unwrap();
        let run = "active-budget-pending";
        pending_id = format!("{}-{run}-0-60", &account[..12]);
        let mut cursors = serde_json::Map::new();
        cursors.insert(run.into(), json!(0));
        app.store.set_setting(OUTBOX_KEY, &json!({"batches_v2":[{"request":{"session_id":pending_id,
            "account_key":account,"book_id":BOOK,"started_at":Utc::now().timestamp()-600,"seconds":60,
            "progress_write_allowed":true,"active_reader_intent":false,"position":{"chapter_uid":CHAPTER,
                "chapter_index":0,"chapter_offset":0,"progress":0,"summary":"Pending proof","updated_at":1,"device":"fixture"}},
            "run_id":run,"from":0,"to":60,"state":"pending","attempted_at":null,"retry_after":0,"attempts":0,"error":null}],
            "reported_cursors":cursors,"compacted_runs":{}}))?;
        app.route = Route::Settings;
        app.reader.session_started = None;
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
            "chapterUid":CHAPTER,"chapterIdx":0,"wordCount":20,"title":"Pending chapter"}]}]}))
        }
        path if path.starts_with("/web/reader/") => AllocationReply::text(
            "<script>window.__INITIAL_STATE__={\"reader\":{\"psvts\":\"synthetic-active-clock\",\"pclts\":0}};</script>",
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
            assert_eq!(queue["batches_v2"][0]["state"], "sending");
            if let Some(seconds) = request.json()["rt"].as_u64() {
                assert_eq!(seconds, 60);
            }
            AllocationReply::json(json!({"succ":1,"synckey":"synthetic-active-receipt"}))
        }
        path => panic!("Unexpected active-run fixture path: {path}"),
    });
    {
        let mut app = fixture.app();
        let client = {
            let _context = runtime.enter();
            http.client(auth("synthetic-active-budget-a"))
        };
        let (sender, receiver) = mpsc::channel();
        app.jobs = Jobs::new(runtime.handle().clone(), sender, Some(client))?;
        app.events = receiver;
        app.force_offline = false;
        app.online = true;
        app.prefs.push_on_close = true;
        app.prefs.report_reader_only = false;
        limits(
            ActiveInputLimits {
                metadata_bytes: 0,
                ..Default::default()
            },
            || -> Result<()> {
                app.store.set_setting(SCHEDULE_KEY, &0_i64)?;
                app.tick_read_reports()?;
                let event = runtime.block_on(async {
                    let deadline = Instant::now() + Duration::from_secs(3);
                    loop {
                        if let Ok(event) = app.events.try_recv() {
                            break event;
                        }
                        assert!(
                            Instant::now() < deadline,
                            "The genuine Pending report did not finish under active refusal"
                        );
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                });
                assert!(matches!(
                    &event.result,
                    JobResult::ReadReport { result: Ok(_), .. }
                ));
                assert!(event.credits.is_some());
                app.handle_job(event)?;
                assert_eq!(
                    app.store
                        .setting::<ReadReportReceipt>(&format!("report.receipt:{pending_id}"))?
                        .unwrap()
                        .seconds,
                    60
                );
                app.store.set_setting(SCHEDULE_KEY, &0_i64)?;
                app.tick_read_reports()?;
                assert_eq!(
                    raw(&app, ACTIVE_KEY)?.as_deref(),
                    Some(active_original.as_str())
                );
                assert_eq!(stats().full_deserializations, 0);
                assert!(
                    app.store.setting::<Value>(OUTBOX_KEY)?.unwrap()["batches_v2"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
                assert_eq!(app.read_report_summary(), tr("api_error"));
                for _ in 0..3 {
                    app.tick_read_reports()?;
                }
                Ok(())
            },
        )?;
    }
    let records = http.finish();
    assert_eq!(records.len(), 5);
    assert_eq!(
        records
            .iter()
            .filter(
                |request| request.path() == "/web/book/read" && request.json().get("rt").is_some()
            )
            .count(),
        1
    );
    fixture.finish()?;
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn stored_active_capacity_refuses_before_dto_or_maintenance_write() -> Result<()> {
    isolated(
        "app::reporting::active_budget_tests::stored_active_capacity_refuses_before_dto_or_maintenance_write",
        rejected_before_dto,
    )
}
#[test]
fn idle_context_summary_and_local_id_do_not_copy_cold_active_fields() -> Result<()> {
    isolated(
        "app::reporting::active_budget_tests::idle_context_summary_and_local_id_do_not_copy_cold_active_fields",
        idle_headers,
    )
}
#[test]
fn utf16_active_exact_working_fees_reject_one_less_before_deserialization() -> Result<()> {
    isolated(
        "app::reporting::active_budget_tests::utf16_active_exact_working_fees_reject_one_less_before_deserialization",
        utf16_exact_working_fee,
    )
}
#[test]
fn actual_checkpoint_preserves_legacy_fields_and_unallocated_seconds() -> Result<()> {
    isolated(
        "app::reporting::active_budget_tests::actual_checkpoint_preserves_legacy_fields_and_unallocated_seconds",
        checkpoint_preserves_cold_fields,
    )
}
#[test]
fn malformed_active_schema_preserves_cause_before_any_owned_record() -> Result<()> {
    isolated(
        "app::reporting::active_budget_tests::malformed_active_schema_preserves_cause_before_any_owned_record",
        malformed_before_copy,
    )
}
#[test]
fn refused_deferred_record_keeps_numeric_evidence_and_bounded_rotation_marker() -> Result<()> {
    isolated(
        "app::reporting::active_budget_tests::refused_deferred_record_keeps_numeric_evidence_and_bounded_rotation_marker",
        deferred_and_marker,
    )
}
#[test]
fn active_debt_successor_transaction_preserves_old_record_on_sql_failure() -> Result<()> {
    isolated(
        "app::reporting::active_budget_tests::active_debt_successor_transaction_preserves_old_record_on_sql_failure",
        archive_atomic,
    )
}
#[test]
fn recovery_and_new_context_do_not_replace_a_refused_old_active_record() -> Result<()> {
    isolated(
        "app::reporting::active_budget_tests::recovery_and_new_context_do_not_replace_a_refused_old_active_record",
        capacity_recovery_and_new_context,
    )
}
#[test]
fn genuine_pending_delivery_survives_active_refusal_without_elapsed_replay() -> Result<()> {
    isolated(
        "app::reporting::active_budget_tests::genuine_pending_delivery_survives_active_refusal_without_elapsed_replay",
        pending_http,
    )
}
