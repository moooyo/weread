//! Measured Reader counters exercise bounded allocation and durable prefixes.

use super::reporting::{
    AllocationCapacity, AllocationLimits, allocation_test_stats, with_allocation_test_limits,
};
use super::underline_projection_tests::{BOOK, CHAPTER, Fixture, isolated};
use super::*;
use crate::jobs::JobResult;
use crate::platform::{ManagedSleepState, PlatformError, RenderSink};
use crate::source::report::{AllocationHttpFixture, AllocationRecordedRequest, AllocationReply};
use crate::store::{EncodedSettingsLimits, with_encoded_settings_test_limits};
use serde_json::{Value, json};

const ACTIVE: &str = "report.active.v1";
const OUTBOX: &str = "report.outbox.v1";
const STORAGE_ERROR: &str = "report.storage_error.v1";
const SCHEDULE: &str = "report.next_check.v1";
const DEBT: &str = "report.allocation_debt.v1:";
const OTHER: &str = "allocation-second-book";
const OTHER_UID: &str = "allocation-second-chapter";
const BODY: &str = "Keep this complete allocation body and local note.";

fn auth(vid: &str) -> source::AuthSession {
    source::AuthSession {
        api_key: "synthetic-allocation-key".into(),
        user_vid: vid.into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), vid.into()),
            ("wr_skey".into(), "synthetic-allocation-session".into()),
        ]),
        ..Default::default()
    }
}

fn sql(app: &App) -> Result<rusqlite::Connection> {
    Ok(rusqlite::Connection::open(
        app.data_dir.join("library.sqlite"),
    )?)
}

fn seed_book(app: &App, id: &str, uid: &str) -> Result<()> {
    let mut book = Book::new(id, "Allocation \u{4e2d} \u{1f642} report chapter");
    book.source = "weread".into();
    book.intro = "Keep allocation introduction.".into();
    book.total_chapters = 1;
    app.store.upsert_book(&book)?;
    app.store.upsert_chapter(&Chapter {
        id: uid.into(),
        book_id: id.into(),
        index: 0,
        title: book.title,
        cached: true,
        body: Some(BODY.into()),
        content_status: ContentStatus::Full,
        content_account_key: app.chapter_content_owner().unwrap(),
    })?;
    app.store.set_setting(
        &format!("catalog.remote:{id}"),
        &vec![source::RemoteChapter {
            uid: uid.into(),
            title: "Allocation report chapter".into(),
            index: 0,
            word_count: 20,
            ..Default::default()
        }],
    )
}

fn begin(app: &mut App, id: &str, uid: &str) -> Result<()> {
    seed_book(app, id, uid)?;
    app.reader.book_id = id.into();
    app.reader.chapter_index = 0;
    app.reader.body = BODY.into();
    app.reader.session_started = Some(Utc::now().timestamp() - 600);
    app.reader.session_seconds = 0;
    app.reader.position_confirmed = true;
    app.reader.last_active = Instant::now();
    app.route = Route::Reader(id.into(), 0);
    app.start_read_report_session()
}

fn setup(app: &mut App) -> Result<()> {
    app.reader.session_started = None;
    app.reader.session_seconds = 0;
    app.demo = false;
    app.force_offline = true;
    app.online = false;
    app.prefs.report_time = true;
    app.prefs.push_on_close = false;
    app.prefs.pull_on_open = false;
    app.prefs.report_reader_only = false;
    app.jobs.replace_session(auth("synthetic-allocation-a"))?;
    begin(app, BOOK, CHAPTER)?;
    app.reader.selection = Some((0, 4));
    assert!(app.reader.draft.set_query("Keep allocation draft"));
    app.store.save_annotation(&Annotation {
        id: "allocation-kept-note".into(),
        book_id: BOOK.into(),
        start: 0,
        end: 4,
        quote: "Keep".into(),
        note: "Keep allocation note".into(),
        created_at: 1,
        ..Default::default()
    })
}

fn snapshot(app: &App, key: &str) -> Result<Value> {
    Ok(app.store.setting(key)?.unwrap_or(Value::Null))
}

fn batches(app: &App) -> Result<Vec<Value>> {
    Ok(snapshot(app, OUTBOX)?["batches_v2"]
        .as_array()
        .cloned()
        .unwrap_or_default())
}

fn assert_kept(app: &App, account: &Option<source::AuthSession>) -> Result<()> {
    assert_eq!(&app.jobs.session(), account);
    assert_eq!(app.reader.body, BODY);
    assert_eq!(app.reader.draft.query, "Keep allocation draft");
    assert_eq!(app.reader.selection, Some((0, 4)));
    assert_eq!(app.store.chapter_body(BOOK, 0)?.as_deref(), Some(BODY));
    assert_eq!(
        app.store.book(BOOK)?.unwrap().intro,
        "Keep allocation introduction."
    );
    let note = app
        .store
        .annotation_by_id(BOOK, "allocation-kept-note")?
        .unwrap();
    assert_eq!(
        (note.quote.as_str(), note.note.as_str()),
        ("Keep", "Keep allocation note")
    );
    Ok(())
}

fn assert_unique_intervals(app: &App, seconds: u64) -> Result<()> {
    let rows = batches(app)?;
    let mut ids = BTreeSet::new();
    let mut intervals = rows
        .iter()
        .map(|row| {
            assert!(ids.insert(row["request"]["session_id"].as_str().unwrap()));
            let from = row["from"].as_u64().unwrap();
            let to = row["to"].as_u64().unwrap();
            assert_eq!(to - from, row["request"]["seconds"].as_u64().unwrap());
            assert!((1..=60).contains(&(to - from)));
            (from, to)
        })
        .collect::<Vec<_>>();
    intervals.sort_unstable();
    let mut end = 0;
    for (from, to) in intervals {
        assert_eq!(from, end);
        end = to;
    }
    assert_eq!(end, seconds);
    Ok(())
}

fn storage_recovery() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        setup(&mut app)?;
        let account = app.jobs.session();
        let connection = sql(&app)?;
        connection.execute_batch("CREATE TRIGGER allocation_outbox_fault BEFORE INSERT ON settings
            WHEN NEW.key='report.outbox.v1' BEGIN SELECT RAISE(ABORT,'synthetic allocation failure'); END;")?;
        app.reader.session_seconds = 60;
        app.tick_read_reports()?;
        assert_eq!(app.store.setting::<bool>(STORAGE_ERROR)?, Some(true));
        for seconds in [120, 180, 301] {
            app.reader.session_seconds = seconds;
            app.checkpoint_read_reports(false)?;
        }
        let active = snapshot(&app, ACTIVE)?;
        assert_eq!(active["eligible_seconds"], 301);
        assert_eq!(active["allocated_seconds"], 0);
        assert!(batches(&app)?.is_empty());
        app.finish_reading()?;
        assert!(snapshot(&app, ACTIVE)?.is_null());
        let key = app
            .store
            .first_setting_key_with_prefix(DEBT, None)?
            .unwrap();
        assert_eq!(snapshot(&app, &key)?["run"]["eligible_seconds"], 301);
        connection.execute_batch("DROP TRIGGER allocation_outbox_fault")?;
        with_allocation_test_limits(
            AllocationLimits {
                batches: 2,
                ..Default::default()
            },
            || -> Result<()> {
                app.recover_read_reports()?;
                let stats = allocation_test_stats();
                assert_eq!(stats.remaining_batches, 6);
                assert_eq!(stats.batch_copies, 2);
                assert_eq!(batches(&app)?.len(), 2);
                assert_eq!(snapshot(&app, &key)?["run"]["allocated_seconds"], 120);
                Ok(())
            },
        )?;
        for _ in 0..2 {
            with_allocation_test_limits(
                AllocationLimits {
                    batches: 2,
                    ..Default::default()
                },
                || -> Result<()> {
                    let before = batches(&app)?.len();
                    app.tick_read_reports()?;
                    let stats = allocation_test_stats();
                    assert!(stats.batch_copies <= 2);
                    assert!(stats.existing_comparisons <= before * stats.batch_copies);
                    assert!(stats.validation_batch_visits <= 8 * (before + 2));
                    Ok(())
                },
            )?;
        }
        assert_unique_intervals(&app, 301)?;
        assert!(
            app.store
                .first_setting_key_with_prefix(DEBT, None)?
                .is_none()
        );
        assert_ne!(app.store.setting::<bool>(STORAGE_ERROR)?, Some(true));
        assert_kept(&app, &account)?;
    }
    fixture.finish()
}

fn partial_fault() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        setup(&mut app)?;
        app.reader.session_seconds = 180;
        with_allocation_test_limits(
            AllocationLimits {
                batches: 1,
                ..Default::default()
            },
            || app.checkpoint_read_reports(true),
        )?;
        let before = snapshot(&app, OUTBOX)?;
        let active = snapshot(&app, ACTIVE)?;
        assert_eq!(active["allocated_seconds"], 60);
        let connection = sql(&app)?;
        connection.execute_batch("CREATE TRIGGER allocation_cursor_fault BEFORE UPDATE ON settings
            WHEN NEW.key='report.active.v1' AND json_extract(NEW.value,'$.allocated_seconds')>json_extract(OLD.value,'$.allocated_seconds')
            BEGIN SELECT RAISE(ABORT,'synthetic allocation cursor failure'); END;")?;
        with_allocation_test_limits(
            AllocationLimits {
                batches: 1,
                ..Default::default()
            },
            || -> Result<()> {
                let error = app.checkpoint_read_reports(true).unwrap_err();
                assert!(error.downcast_ref::<rusqlite::Error>().is_some());
                assert_eq!(allocation_test_stats().batch_copies, 1);
                assert_eq!(snapshot(&app, OUTBOX)?, before);
                assert_eq!(snapshot(&app, ACTIVE)?["allocated_seconds"], 60);
                Ok(())
            },
        )?;
        connection.execute_batch("DROP TRIGGER allocation_cursor_fault")?;
        app.reader.session_started = None;
        with_allocation_test_limits(
            AllocationLimits {
                batches: 1,
                ..Default::default()
            },
            || app.recover_read_reports(),
        )?;
        with_allocation_test_limits(
            AllocationLimits {
                batches: 1,
                ..Default::default()
            },
            || app.tick_read_reports(),
        )?;
        assert_unique_intervals(&app, 180)?;
        assert!(
            app.store
                .first_setting_key_with_prefix(DEBT, None)?
                .is_none()
        );
    }
    fixture.finish()
}

fn exact_limits() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        setup(&mut app)?;
        app.reader.session_seconds = 60;
        let connection = sql(&app)?;
        connection.execute_batch("CREATE TRIGGER allocation_price_fault BEFORE INSERT ON settings
            WHEN NEW.key='report.outbox.v1' BEGIN SELECT RAISE(ABORT,'synthetic allocation pricing'); END;")?;
        let fee = with_allocation_test_limits(
            AllocationLimits {
                batches: 1,
                ..Default::default()
            },
            || -> Result<_> {
                let error = app.checkpoint_read_reports(true).unwrap_err();
                assert!(error.downcast_ref::<rusqlite::Error>().is_some());
                let stats = allocation_test_stats();
                assert_eq!(stats.batch_copies, 1);
                Ok(stats)
            },
        )?;
        connection.execute_batch("DROP TRIGGER allocation_price_fault")?;
        let account = app.jobs.session();
        let before = snapshot(&app, OUTBOX)?;
        let exact = AllocationLimits {
            batches: 1,
            metadata_bytes: fee.metadata_bytes,
            encoded_bytes: fee.encoded_bytes,
            validation_work: fee.validation_work,
            ..Default::default()
        };
        for limits in [
            AllocationLimits {
                metadata_bytes: exact.metadata_bytes - 1,
                ..exact
            },
            AllocationLimits {
                encoded_bytes: exact.encoded_bytes - 1,
                ..exact
            },
            AllocationLimits {
                validation_work: exact.validation_work - 1,
                ..exact
            },
        ] {
            with_allocation_test_limits(limits, || -> Result<()> {
                let error = app.checkpoint_read_reports(true).unwrap_err();
                assert!(error.downcast_ref::<AllocationCapacity>().is_some());
                let stats = allocation_test_stats();
                assert_eq!(stats.batch_copies, 0);
                assert_eq!(stats.old_batch_copies, 0);
                assert_eq!(snapshot(&app, OUTBOX)?, before);
                assert_eq!(snapshot(&app, ACTIVE)?["allocated_seconds"], 0);
                assert_kept(&app, &account)
            })?;
        }
        with_allocation_test_limits(exact, || app.checkpoint_read_reports(true))?;
        assert_unique_intervals(&app, 60)?;
        let actual = serde_json::to_string(&snapshot(&app, OUTBOX)?)?.len()
            + serde_json::to_string(&snapshot(&app, ACTIVE)?)?.len();
        assert_eq!(actual, fee.encoded_bytes);
    }
    fixture.finish()
}

fn new_book() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        setup(&mut app)?;
        app.reader.session_seconds = 125;
        with_allocation_test_limits(
            AllocationLimits {
                batches: 0,
                ..Default::default()
            },
            || -> Result<()> {
                let error = app.checkpoint_read_reports(true).unwrap_err();
                assert!(error.downcast_ref::<AllocationCapacity>().is_some());
                Ok(())
            },
        )?;
        let old = snapshot(&app, ACTIVE)?;
        let connection = sql(&app)?;
        connection.execute_batch(
            "CREATE TRIGGER allocation_archive_fault BEFORE INSERT ON settings
            WHEN substr(NEW.key,1,26)='report.allocation_debt.v1:'
            BEGIN SELECT RAISE(ABORT,'synthetic allocation archive failure'); END;",
        )?;
        let result = begin(&mut app, OTHER, OTHER_UID);
        let error = result.unwrap_err();
        assert!(error.downcast_ref::<rusqlite::Error>().is_some());
        assert_eq!(snapshot(&app, ACTIVE)?["id"], old["id"]);
        assert!(
            app.store
                .first_setting_key_with_prefix(DEBT, None)?
                .is_none()
        );
        connection.execute_batch("DROP TRIGGER allocation_archive_fault")?;
        app.start_read_report_session()?;
        app.reader.session_seconds = 20;
        with_allocation_test_limits(
            AllocationLimits {
                batches: 0,
                ..Default::default()
            },
            || app.checkpoint_read_reports(false),
        )?;
        let current = snapshot(&app, ACTIVE)?;
        assert_eq!(current["book_id"], OTHER);
        assert_eq!(current["eligible_seconds"], 20);
        assert_ne!(current["id"], old["id"]);
        let old_key = app
            .store
            .first_setting_key_with_prefix(DEBT, None)?
            .unwrap();
        assert_eq!(snapshot(&app, &old_key)?["run"]["eligible_seconds"], 125);
        with_allocation_test_limits(
            AllocationLimits {
                batches: 0,
                ..Default::default()
            },
            || app.finish_reading(),
        )?;
        for _ in 0..3 {
            with_allocation_test_limits(
                AllocationLimits {
                    batches: 2,
                    ..Default::default()
                },
                || app.tick_read_reports(),
            )?;
        }
        let rows = batches(&app)?;
        let sum = |book: &str| {
            rows.iter()
                .filter(|row| row["request"]["book_id"] == book)
                .map(|row| row["request"]["seconds"].as_u64().unwrap())
                .sum::<u64>()
        };
        assert_eq!((sum(BOOK), sum(OTHER)), (125, 20));
        let mut ids = BTreeSet::new();
        assert!(
            rows.iter()
                .all(|row| ids.insert(row["request"]["session_id"].as_str().unwrap()))
        );
        assert!(
            app.store
                .first_setting_key_with_prefix(DEBT, None)?
                .is_none()
        );
        assert_eq!(app.store.chapter_body(BOOK, 0)?.as_deref(), Some(BODY));
        assert_eq!(
            app.store
                .annotation_by_id(BOOK, "allocation-kept-note")?
                .unwrap()
                .note,
            "Keep allocation note"
        );
    }
    fixture.finish()
}

fn account_split() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        setup(&mut app)?;
        app.reader.session_seconds = 60;
        with_allocation_test_limits(
            AllocationLimits {
                batches: 0,
                ..Default::default()
            },
            || -> Result<()> {
                assert!(
                    app.checkpoint_read_reports(true)
                        .unwrap_err()
                        .downcast_ref::<AllocationCapacity>()
                        .is_some()
                );
                Ok(())
            },
        )?;
        let first = snapshot(&app, ACTIVE)?;
        let local_id = first["id"].as_str().unwrap().to_owned();
        // Thirty newly observed seconds still belong to the old owner at the
        // transition boundary. The new owner begins after that durable baseline.
        app.reader.session_seconds = 90;
        app.jobs.replace_session(auth("synthetic-allocation-b"))?;
        app.start_read_report_session()?;
        let mut next = snapshot(&app, ACTIVE)?;
        assert_eq!(next["observer_seconds_base"], 90);
        assert_eq!(next["local_session_id"], local_id);
        assert_ne!(next["account_key"], first["account_key"]);
        assert!(next["position"].is_null());
        // A small synthetic clock interval admits subsequent real counter delta.
        next["started_at"] = json!(Utc::now().timestamp() - 60);
        next["last_checkpoint_at"] = next["started_at"].clone();
        app.store.set_setting(ACTIVE, &next)?;
        app.reader.session_seconds = 120;
        with_allocation_test_limits(
            AllocationLimits {
                batches: 0,
                ..Default::default()
            },
            || app.checkpoint_read_reports(false),
        )?;
        assert_eq!(snapshot(&app, ACTIVE)?["eligible_seconds"], 30);
        let old_key = app
            .store
            .first_setting_key_with_prefix(DEBT, None)?
            .unwrap();
        assert_eq!(snapshot(&app, &old_key)?["run"]["eligible_seconds"], 90);
        with_allocation_test_limits(
            AllocationLimits {
                batches: 0,
                ..Default::default()
            },
            || app.finish_reading(),
        )?;
        for _ in 0..3 {
            with_allocation_test_limits(
                AllocationLimits {
                    batches: 1,
                    ..Default::default()
                },
                || app.tick_read_reports(),
            )?;
        }
        let rows = batches(&app)?;
        let old_owner = first["account_key"].as_str().unwrap();
        let old_seconds = rows
            .iter()
            .filter(|row| row["request"]["account_key"] == old_owner)
            .map(|row| row["request"]["seconds"].as_u64().unwrap())
            .sum::<u64>();
        let new_seconds = rows
            .iter()
            .filter(|row| row["request"]["account_key"] != old_owner)
            .map(|row| row["request"]["seconds"].as_u64().unwrap())
            .sum::<u64>();
        assert_eq!((old_seconds, new_seconds), (90, 30));
        let saved = app
            .store
            .sessions(i64::MIN, i64::MAX)?
            .into_iter()
            .filter(|session| session.id == local_id)
            .collect::<Vec<_>>();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].seconds, 120);
        assert!(
            app.store
                .first_setting_key_with_prefix(DEBT, None)?
                .is_none()
        );
    }
    fixture.finish()
}

fn pending_post() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let database;
    let debt_key;
    let pending_id;
    let pending_run;
    let pending_summary;
    {
        let mut app = fixture.app();
        setup(&mut app)?;
        app.reader.session_seconds = 60;
        with_allocation_test_limits(
            AllocationLimits {
                batches: 1,
                ..Default::default()
            },
            || app.checkpoint_read_reports(true),
        )?;
        app.finish_reading()?;
        begin(&mut app, BOOK, CHAPTER)?;
        app.reader.session_seconds = 60;
        with_allocation_test_limits(
            AllocationLimits {
                batches: 0,
                ..Default::default()
            },
            || app.finish_reading(),
        )?;
        assert_eq!(batches(&app)?.len(), 1);
        let pending = batches(&app)?.into_iter().next().unwrap();
        pending_id = pending["request"]["session_id"]
            .as_str()
            .unwrap()
            .to_owned();
        pending_run = pending["run_id"].as_str().unwrap().to_owned();
        pending_summary = pending["request"]["position"]["summary"].clone();
        debt_key = app
            .store
            .first_setting_key_with_prefix(DEBT, None)?
            .unwrap();
        database = app.data_dir.join("library.sqlite");
    }
    let server_id = pending_id.clone();
    let server_summary = pending_summary.clone();
    let http = AllocationHttpFixture::new(move |request: &AllocationRecordedRequest| match request
        .path()
    {
        "/gateway" => {
            assert_eq!(request.json()["api_name"], "/book/getprogress");
            AllocationReply::json(json!({"bookId":BOOK,"progress":0,"chapterUid":CHAPTER,
                    "chapterIdx":0,"chapterOffset":0,"updatedAt":Utc::now().timestamp(),"chapterTitle":"Fixture"}))
        }
        "/web/book/chapterInfos" => {
            AllocationReply::json(json!({"data":[{"bookId":BOOK,"updated":[{
                "chapterUid":CHAPTER,"title":"Fixture","chapterIdx":0,"wordCount":20
            }]}]}))
        }
        path if path.starts_with("/web/reader/") => AllocationReply::text(
            "<script>window.__INITIAL_STATE__={\"reader\":{\"psvts\":\"synthetic-report-clock\",\"pclts\":0}};</script>",
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
            assert!(
                queue["batches_v2"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|batch| batch["state"] == "sending")
            );
            let original = queue["batches_v2"]
                .as_array()
                .unwrap()
                .iter()
                .find(|batch| batch["request"]["session_id"].as_str() == Some(server_id.as_str()))
                .expect("The original pending report must be durable before HTTP");
            assert_eq!(original["state"], "sending");
            assert_eq!(original["request"]["position"]["summary"], server_summary);
            if let Some(seconds) = request.json().get("rt").and_then(Value::as_u64) {
                assert_eq!(seconds, 60);
                AllocationReply::json(json!({"succ":1,"synckey":"synthetic-allocation-receipt"}))
            } else {
                AllocationReply::json(json!({"succ":1}))
            }
        }
        path => panic!("Unexpected allocation fixture path: {path}"),
    });
    {
        let mut app = fixture.app();
        let client = {
            let _context = runtime.enter();
            http.client(auth("synthetic-allocation-a"))
        };
        let (sender, receiver) = mpsc::channel();
        app.jobs = Jobs::new(runtime.handle().clone(), sender, Some(client))?;
        app.events = receiver;
        app.force_offline = false;
        app.online = true;
        app.prefs.push_on_close = true;
        app.prefs.report_reader_only = false;
        app.store.set_setting(SCHEDULE, &0_i64)?;
        let account = app.jobs.session();
        with_encoded_settings_test_limits(
            EncodedSettingsLimits {
                bytes: 32,
                key_bytes: 4096,
            },
            || {
                with_allocation_test_limits(
                    AllocationLimits {
                        batches: 0,
                        ..Default::default()
                    },
                    || -> Result<()> {
                        app.tick_read_reports()?;
                        assert_ne!(app.store.setting::<bool>(STORAGE_ERROR)?, Some(true));
                        let sending = batches(&app)?;
                        assert_eq!(sending.len(), 1);
                        assert_eq!(
                            sending[0]["request"]["session_id"].as_str(),
                            Some(pending_id.as_str())
                        );
                        assert_eq!(
                            sending[0]["state"], "sending",
                            "Existing Pending must reach durable Sending while new allocation is refused"
                        );
                        assert_eq!(
                            sending[0]["request"]["position"]["summary"],
                            pending_summary
                        );
                        assert_eq!(snapshot(&app, OUTBOX)?["reported_cursors"][&pending_run], 0);
                        assert_kept(&app, &account)?;
                        let event = runtime.block_on(async {
                            let deadline = Instant::now() + Duration::from_secs(3);
                            loop {
                                if let Ok(event) = app.events.try_recv() {
                                    break event;
                                }
                                assert!(
                                    Instant::now() < deadline,
                                    "The genuine pending report did not finish"
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
                                .setting::<source::report::ReadReportReceipt>(&format!(
                                    "report.receipt:{pending_id}"
                                ))?
                                .unwrap()
                                .seconds,
                            60
                        );
                        app.store.set_setting(SCHEDULE, &0_i64)?;
                        app.tick_read_reports()?;
                        assert!(batches(&app)?.is_empty());
                        assert_eq!(
                            snapshot(&app, OUTBOX)?["reported_cursors"][&pending_run],
                            60
                        );
                        assert!(
                            app.store
                                .setting::<source::report::ReadReportReceipt>(&format!(
                                    "report.receipt:{pending_id}"
                                ))?
                                .is_none()
                        );
                        assert_eq!(snapshot(&app, &debt_key)?["run"]["eligible_seconds"], 60);
                        assert_eq!(allocation_test_stats().batch_copies, 0);
                        assert_eq!(app.read_report_summary(), tr("api_error"));
                        assert_kept(&app, &account)?;
                        for _ in 0..3 {
                            app.tick_read_reports()?;
                        }
                        Ok(())
                    },
                )
            },
        )?;
        assert!(app.read_report_summary().contains('+'));
    }
    let records = http.finish();
    let elapsed = records
        .iter()
        .filter(|request| request.path() == "/web/book/read")
        .filter(|request| request.json().get("rt").is_some())
        .collect::<Vec<_>>();
    assert_eq!(elapsed.len(), 1);
    assert_eq!(records.len(), 5);
    fixture.finish()?;
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

fn partial_then_minute() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        setup(&mut app)?;
        with_allocation_test_limits(
            AllocationLimits {
                batches: 1,
                ..Default::default()
            },
            || -> Result<()> {
                app.reader.session_seconds = 65;
                app.checkpoint_read_reports(true)?;
                app.checkpoint_read_reports(true)?;
                app.reader.session_seconds = 125;
                app.checkpoint_read_reports(false)?;
                let rows = batches(&app)?;
                assert_eq!(
                    rows.iter()
                        .map(|row| row["request"]["seconds"].as_u64().unwrap())
                        .collect::<Vec<_>>(),
                    [60, 5, 60]
                );
                assert_eq!(rows[2]["from"], 65);
                assert_eq!(rows[2]["to"], 125);
                assert_eq!(snapshot(&app, ACTIVE)?["allocated_seconds"], 125);
                assert_eq!(allocation_test_stats().batch_copies, 3);
                assert_unique_intervals(&app, 125)
            },
        )?;
    }
    fixture.finish()
}

struct ResidentSink;

impl RenderSink for ResidentSink {
    fn set_dark_mode(&self, _: bool) -> Result<(), PlatformError> {
        Ok(())
    }
    fn managed_sleep_supported(&self) -> bool {
        true
    }
    fn managed_sleep_state(&self) -> Result<ManagedSleepState, PlatformError> {
        Ok(ManagedSleepState::Running)
    }
}

fn resident_counter() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        setup(&mut app)?;
        app.platform.set_render_sink(Arc::new(ResidentSink))?;
        app.reader.session_seconds = 125;
        with_allocation_test_limits(
            AllocationLimits {
                batches: 1,
                ..Default::default()
            },
            || app.checkpoint_read_reports(true),
        )?;
        let old = snapshot(&app, ACTIVE)?;
        with_allocation_test_limits(
            AllocationLimits {
                batches: 1,
                ..Default::default()
            },
            || -> Result<()> {
                app.prepare_resident_sleep()?;
                app.resume_resident_sleep()?;
                assert_eq!(allocation_test_stats().batch_copies, 1);
                Ok(())
            },
        )?;
        let next = snapshot(&app, ACTIVE)?;
        assert_ne!(next["id"], old["id"]);
        assert_eq!(next["eligible_seconds"], 0);
        assert_eq!(app.reader.session_seconds, 0);
        let key = app
            .store
            .first_setting_key_with_prefix(DEBT, None)?
            .unwrap();
        assert_eq!(snapshot(&app, &key)?["run"]["allocated_seconds"], 120);
        with_allocation_test_limits(
            AllocationLimits {
                batches: 1,
                ..Default::default()
            },
            || app.tick_read_reports(),
        )?;
        assert_unique_intervals(&app, 125)?;
        assert!(
            app.store
                .first_setting_key_with_prefix(DEBT, None)?
                .is_none()
        );
    }
    fixture.finish()
}

fn production_prefix_ceiling() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        setup(&mut app)?;
        app.reader.session_started = Some(Utc::now().timestamp() - 3600);
        app.start_read_report_session()?;
        app.reader.session_seconds = 17 * 60;
        let raised = AllocationLimits {
            batches: usize::MAX,
            ..Default::default()
        };
        with_allocation_test_limits(raised, || -> Result<()> {
            app.checkpoint_read_reports(true)?;
            let stats = allocation_test_stats();
            assert_eq!(stats.remaining_batches, 17);
            assert_eq!(stats.batch_copies, 16);
            assert_eq!(batches(&app)?.len(), 16);
            assert_eq!(snapshot(&app, ACTIVE)?["allocated_seconds"], 960);
            assert_eq!(snapshot(&app, ACTIVE)?["eligible_seconds"], 1020);
            Ok(())
        })?;
        with_allocation_test_limits(raised, || -> Result<()> {
            app.checkpoint_read_reports(true)?;
            assert_eq!(allocation_test_stats().batch_copies, 1);
            Ok(())
        })?;
        assert_unique_intervals(&app, 1020)?;
    }
    fixture.finish()
}

fn confirmed_history() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        setup(&mut app)?;
        let account = source::report::account_key(&app.jobs.session().unwrap()).unwrap();
        let mut cursors = serde_json::Map::new();
        let mut proofs = serde_json::Map::new();
        // A small existing history has no pending batch scans. Its necessary
        // proof does not spend a fabricated quadratic proof-by-proof allowance.
        for index in 0..161 {
            let id = format!("confirmed-history-{index}");
            cursors.insert(id.clone(), json!(60));
            proofs.insert(
                id,
                json!({"account_key":account,"book_id":BOOK,
                "started_at":100 + index,"confirmed_until":60,"confirmed_ranges":[]}),
            );
        }
        app.store.set_setting(
            OUTBOX,
            &json!({"batches_v2":[],"reported_cursors":cursors,"compacted_runs":proofs}),
        )?;
        app.reader.session_seconds = 60;
        with_allocation_test_limits(AllocationLimits::default(), || -> Result<()> {
            app.checkpoint_read_reports(true)?;
            assert_eq!(allocation_test_stats().batch_copies, 1);
            assert!(
                allocation_test_stats().validation_work
                    <= AllocationLimits::default().validation_work
            );
            Ok(())
        })?;
        let queue = snapshot(&app, OUTBOX)?;
        assert_eq!(queue["compacted_runs"].as_object().unwrap().len(), 161);
        assert_eq!(queue["reported_cursors"].as_object().unwrap().len(), 162);
        assert_eq!(batches(&app)?.len(), 1);
    }
    fixture.finish()
}

fn store_envelope() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut app = fixture.app();
        setup(&mut app)?;
        app.reader.session_seconds = 60;
        app.checkpoint_read_reports(true)?;
        let before = snapshot(&app, OUTBOX)?;
        let account = app.jobs.session();
        app.reader.session_seconds = 180;
        with_encoded_settings_test_limits(
            EncodedSettingsLimits {
                bytes: 32,
                key_bytes: 4096,
            },
            || -> Result<()> {
                let error = app.checkpoint_read_reports(true).unwrap_err();
                assert!(error.downcast_ref::<AllocationCapacity>().is_some());
                assert!(error.downcast_ref::<rusqlite::Error>().is_none());
                app.note_read_report_error(&error);
                assert_ne!(app.store.setting::<bool>(STORAGE_ERROR)?, Some(true));
                assert_eq!(snapshot(&app, OUTBOX)?, before);
                assert_eq!(snapshot(&app, ACTIVE)?["eligible_seconds"], 180);
                assert_eq!(snapshot(&app, ACTIVE)?["allocated_seconds"], 60);
                assert_kept(&app, &account)
            },
        )?;
    }
    fixture.finish()
}

#[test]
fn genuine_storage_failure_preserves_eligible_time_across_bounded_recovery_prefixes() -> Result<()>
{
    isolated(
        "app::report_allocation_budget_tests::genuine_storage_failure_preserves_eligible_time_across_bounded_recovery_prefixes",
        storage_recovery,
    )
}

#[test]
fn genuine_partial_allocation_cursor_failure_rolls_back_batches_and_reloads_unique_ids()
-> Result<()> {
    isolated(
        "app::report_allocation_budget_tests::genuine_partial_allocation_cursor_failure_rolls_back_batches_and_reloads_unique_ids",
        partial_fault,
    )
}

#[test]
fn genuine_exact_allocation_fees_reject_one_less_before_owned_batch_fields() -> Result<()> {
    isolated(
        "app::report_allocation_budget_tests::genuine_exact_allocation_fees_reject_one_less_before_owned_batch_fields",
        exact_limits,
    )
}

#[test]
fn genuine_new_book_keeps_closed_debt_and_measures_its_new_reader_counter() -> Result<()> {
    isolated(
        "app::report_allocation_budget_tests::genuine_new_book_keeps_closed_debt_and_measures_its_new_reader_counter",
        new_book,
    )
}

#[test]
fn genuine_account_transition_preserves_eligible_delta_and_one_local_session() -> Result<()> {
    isolated(
        "app::report_allocation_budget_tests::genuine_account_transition_preserves_eligible_delta_and_one_local_session",
        account_split,
    )
}

#[test]
fn genuine_pending_post_survives_allocation_refusal_and_is_not_replayed_after_receipt() -> Result<()>
{
    isolated(
        "app::report_allocation_budget_tests::genuine_pending_post_survives_allocation_refusal_and_is_not_replayed_after_receipt",
        pending_post,
    )
}

#[test]
fn genuine_final_partial_interval_preserves_the_next_regular_sixty_second_batch() -> Result<()> {
    isolated(
        "app::report_allocation_budget_tests::genuine_final_partial_interval_preserves_the_next_regular_sixty_second_batch",
        partial_then_minute,
    )
}

#[test]
fn genuine_resident_cancel_retains_partial_allocation_without_crediting_resume_wall_time()
-> Result<()> {
    isolated(
        "app::report_allocation_budget_tests::genuine_resident_cancel_retains_partial_allocation_without_crediting_resume_wall_time",
        resident_counter,
    )
}

#[test]
fn genuine_numeric_demand_keeps_the_production_sixteen_batch_ceiling() -> Result<()> {
    isolated(
        "app::report_allocation_budget_tests::genuine_numeric_demand_keeps_the_production_sixteen_batch_ceiling",
        production_prefix_ceiling,
    )
}

#[test]
fn genuine_confirmed_history_does_not_create_quadratic_proof_only_allocation_work() -> Result<()> {
    isolated(
        "app::report_allocation_budget_tests::genuine_confirmed_history_does_not_create_quadratic_proof_only_allocation_work",
        confirmed_history,
    )
}

#[test]
fn genuine_store_envelope_refusal_keeps_observed_counter_and_pending_delivery_state() -> Result<()>
{
    isolated(
        "app::report_allocation_budget_tests::genuine_store_envelope_refusal_keeps_observed_counter_and_pending_delivery_state",
        store_envelope,
    )
}
