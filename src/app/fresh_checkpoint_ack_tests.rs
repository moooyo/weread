//! Real Full worker packets distinguish failed commits from failed post-commit saves.

use super::underline_projection_tests::{Fixture, isolated};
use super::*;
use crate::jobs::{DownloadPlan, JobResult};
use crate::source::report::{AllocationHttpFixture, AllocationReply};
use serde_json::json;

const BOOK: &str = "fixture-book";
const UID: &str = "fixture-chapter";
const TEXT: &str = "Reader fixture.";
const SHARD: &str = "DF1103CA0CD3800B1FA3F842EB367223aPGh0bWw+PJvGZHk+T4A+UmVhZGVyIGZpeHR1cwPPHC9mUjwvYm9keuP8L2h0bWw+";

fn auth() -> source::AuthSession {
    source::AuthSession {
        api_key: "fixture-api-key".into(),
        user_vid: "fixture-account".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "fixture-account".into()),
            ("wr_skey".into(), "fixture-session".into()),
        ]),
        ..Default::default()
    }
}

fn server() -> AllocationHttpFixture {
    AllocationHttpFixture::new(|request| {
        if request.path().starts_with("/web/reader/") {
            let state = json!({ "user": { "vid": "fixture-account" }, "reader": {
                "bookId": BOOK, "bookInfo": { "bookId": BOOK, "paid": 1, "payType": 1 },
                "chapterInfos": [{ "chapterUid": UID, "chapterIdx": 1, "wordCount": 20 }],
                "currentChapter": { "chapterUid": UID },
                "psvts": "fixture-server-clock", "pclts": "fixture-client-clock"
            }});
            AllocationReply::text(format!(
                "<script>window.__INITIAL_STATE__={state};</script>"
            ))
        } else {
            assert_eq!(request.method, "POST");
            match request.path() {
                "/web/book/chapter/e_0" => AllocationReply::text(SHARD),
                "/web/book/chapter/e_1" | "/web/book/chapter/e_3" => AllocationReply::text(""),
                path => panic!("Unexpected checkpoint fixture request: {path}"),
            }
        }
    })
}

fn seed(state: &mut App) -> Result<DownloadTask> {
    let mut book = Book::new(BOOK, "Worker checkpoint book");
    book.source = "weread".into();
    book.total_chapters = 1;
    book.intro = "Retained checkpoint introduction".into();
    state.store.upsert_book(&book)?;
    state.store.upsert_chapter(&Chapter {
        id: UID.into(),
        book_id: BOOK.into(),
        title: "Worker checkpoint chapter".into(),
        body: Some(TEXT.into()),
        content_status: ContentStatus::Unknown,
        ..Default::default()
    })?;
    state.store.save_annotation(&Annotation {
        id: "worker-checkpoint-note".into(),
        book_id: BOOK.into(),
        start: 0,
        end: 6,
        quote: "Reader".into(),
        note: "Preserved worker note".into(),
        created_at: 1,
        ..Default::default()
    })?;
    state.load_local_reader(BOOK, 0, Some(0))?;
    state.reader.selection = Some((0, 6));
    assert!(state.reader.draft.set_query("Preserved worker draft"));
    let owner = state.chapter_content_owner().unwrap();
    let task = DownloadTask {
        id: "fresh-real-worker-task".into(),
        book_id: BOOK.into(),
        account_key: owner,
        chapters: vec![0],
        status: DownloadStatus::Running,
        include_images: false,
        updated_at: 10,
        ..Default::default()
    };
    state.store.save_task(&task)?;
    state.current_download = Some(task.clone());
    state.download_request = state.request();
    state.pending_read = Some((BOOK.into(), 0));
    Ok(task)
}

fn next_event(runtime: &tokio::runtime::Runtime, fixture: &Fixture) -> Result<JobEvent> {
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let event = { fixture.app().events.try_recv() };
                match event {
                    Ok(event) => return Ok(event),
                    Err(mpsc::TryRecvError::Empty) => {
                        tokio::time::sleep(Duration::from_millis(1)).await
                    }
                    Err(error) => {
                        return Err(anyhow::anyhow!("The checkpoint mailbox closed: {error}"));
                    }
                }
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("The real checkpoint worker timed out"))?
    })
}

fn terminal_event(runtime: &tokio::runtime::Runtime, fixture: &Fixture) -> Result<JobEvent> {
    for _ in 0..4 {
        let event = next_event(runtime, fixture)?;
        if matches!(
            &event.result,
            JobResult::DownloadFinished { .. } | JobResult::DownloadFailed { .. }
        ) {
            return Ok(event);
        }
        assert!(matches!(&event.result, JobResult::DownloadProgress { .. }));
        drop(event);
    }
    Err(anyhow::anyhow!(
        "The bounded checkpoint worker did not finish"
    ))
}

fn checkpoint(save_failure: bool) -> Result<()> {
    checkpoint_mode(save_failure, false)
}

fn checkpoint_mode(save_failure: bool, layout_failure: bool) -> Result<()> {
    let fixture = Fixture::new("Unrelated retained book.", Vec::new(), false)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let http = server();
    let client = {
        let _entered = runtime.enter();
        http.client(auth())
    };
    let task = {
        let mut state = fixture.app();
        let (sender, events) = mpsc::channel();
        state.jobs = Jobs::new(runtime.handle().clone(), sender, Some(client))?;
        state.events = events;
        seed(&mut state)?
    };
    let database = rusqlite::Connection::open(fixture.app().data_dir.join("library.sqlite"))?;
    if save_failure {
        database.execute_batch(
            "CREATE TRIGGER refuse_fresh_resume_save BEFORE INSERT ON settings
             WHEN NEW.key='reader.resume.v1'
             BEGIN SELECT RAISE(FAIL,'synthetic fresh resume save failure'); END;",
        )?;
    } else if !layout_failure {
        database.execute_batch(
            "CREATE TRIGGER refuse_fresh_commit BEFORE UPDATE ON download_tasks
             WHEN NEW.id='fresh-real-worker-task'
             BEGIN SELECT RAISE(ABORT,'synthetic fresh commit failure'); END;",
        )?;
    }
    {
        let state = fixture.app();
        let headers = state.store.chapter_metadata(BOOK)?;
        assert!(
            headers
                .iter()
                .all(|chapter| chapter.body.is_none() && !chapter.cached)
        );
        let plan = DownloadPlan::from_headers(&task, &headers)?;
        assert!(
            state
                .jobs
                .download(state.download_request, task.clone(), plan)
                .is_none()
        );
    }
    let packet = next_event(&runtime, &fixture)?;
    {
        let state = fixture.app();
        assert_eq!(packet.request, state.download_request);
        assert_eq!(packet.account_epoch, state.jobs.account_epoch());
        assert!(packet.credits.is_some());
        match &packet.result {
            JobResult::Downloaded {
                task_id,
                chapter,
                next_index,
                ..
            } => {
                assert_eq!(task_id, &task.id);
                assert_eq!(chapter.id, UID);
                assert_eq!(chapter.book_id, BOOK);
                assert_eq!(chapter.content_status, ContentStatus::Full);
                assert!(chapter.cached);
                assert_eq!(chapter.body.as_deref(), Some(TEXT));
                assert_eq!(*next_index, 1);
            }
            result => panic!("The real Full source did not yield a fresh packet: {result:?}"),
        }
    }
    let account = fixture.app().jobs.session();
    if save_failure || layout_failure {
        let old_pages = fixture
            .app()
            .reader
            .paginator
            .as_ref()
            .unwrap()
            .pages
            .as_ptr();
        let error = if layout_failure {
            crate::reader::layout_budget::with_layout_test_limits(
                crate::reader::LayoutLimits {
                    metadata_bytes: 0,
                    ..Default::default()
                },
                || fixture.app().handle_job(packet),
            )
            .unwrap_err()
        } else {
            fixture.app().handle_job(packet).unwrap_err()
        };
        if layout_failure {
            assert!(
                error
                    .downcast_ref::<crate::reader::LayoutLimitError>()
                    .is_some()
            );
            assert_eq!(
                fixture
                    .app()
                    .reader
                    .paginator
                    .as_ref()
                    .unwrap()
                    .pages
                    .as_ptr(),
                old_pages
            );
        } else {
            assert!(error.downcast_ref::<rusqlite::Error>().is_some());
            assert!(format!("{error:#}").contains("synthetic fresh resume save failure"));
        }
        let state = fixture.app();
        assert_eq!(state.store.task(&task.id)?.unwrap().next_index, 1);
        assert_eq!(state.current_download.as_ref().unwrap().next_index, 1);
        assert_eq!(state.store.chapter_body(BOOK, 0)?.as_deref(), Some(TEXT));
        assert_eq!(
            state.store.chapter_headers(BOOK)?[0].content_status,
            ContentStatus::Full
        );
        assert!(state.pending_read.is_none());
        assert_eq!(state.reader.body, TEXT);
        assert_eq!(state.reader.selection, Some((0, 6)));
        assert_eq!(state.reader.draft.query, "Preserved worker draft");
        assert!(
            state
                .reader
                .paginator
                .as_ref()
                .is_some_and(|pages| !pages.current_page().lines.is_empty())
        );
    } else {
        fixture.app().handle_job(packet)?;
        let state = fixture.app();
        assert_eq!(state.store.task(&task.id)?.unwrap().next_index, 0);
        assert_eq!(state.current_download.as_ref().unwrap().next_index, 0);
        assert_eq!(
            state.store.chapter_headers(BOOK)?[0].content_status,
            ContentStatus::Unknown
        );
        assert_eq!(state.pending_read, Some((BOOK.into(), 0)));
    }
    let terminal = terminal_event(&runtime, &fixture)?;
    if save_failure || layout_failure {
        assert!(
            matches!(&terminal.result, JobResult::DownloadFinished { task_id } if task_id == &task.id)
        );
        if save_failure {
            database.execute_batch("DROP TRIGGER refuse_fresh_resume_save")?;
        }
    } else {
        assert!(
            matches!(&terminal.result, JobResult::DownloadFailed { task_id,
            error: source::SourceError::CheckpointFailed } if task_id == &task.id)
        );
        database.execute_batch("DROP TRIGGER refuse_fresh_commit")?;
    }
    drop(terminal);
    {
        let state = fixture.app();
        assert_eq!(state.jobs.session(), account);
        assert_eq!(
            state
                .store
                .annotation_by_id(BOOK, "worker-checkpoint-note")?
                .unwrap()
                .note,
            "Preserved worker note"
        );
        assert_eq!(
            state.store.book(BOOK)?.unwrap().intro,
            "Retained checkpoint introduction"
        );
    }
    let requests = http.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path() == "/web/book/chapter/e_0")
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path() == "/web/book/chapter/e_1")
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path() == "/web/book/chapter/e_3")
            .count(),
        1
    );
    fixture.finish()?;
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

fn committed_then_failed_save() -> Result<()> {
    checkpoint(true)
}
fn failed_commit() -> Result<()> {
    checkpoint(false)
}

fn committed_then_refused_layout() -> Result<()> {
    checkpoint_mode(false, true)
}

#[test]
fn real_full_worker_acknowledges_commit_when_reader_layout_is_unavailable() -> Result<()> {
    isolated(
        "app::fresh_checkpoint_ack_tests::real_full_worker_acknowledges_commit_when_reader_layout_is_unavailable",
        committed_then_refused_layout,
    )
}

#[test]
fn real_full_worker_acknowledges_durable_cursor_despite_later_resume_sql_failure() -> Result<()> {
    isolated(
        "app::fresh_checkpoint_ack_tests::real_full_worker_acknowledges_durable_cursor_despite_later_resume_sql_failure",
        committed_then_failed_save,
    )
}

#[test]
fn real_full_worker_nacks_failed_sql_commit_without_advancing_cursor() -> Result<()> {
    isolated(
        "app::fresh_checkpoint_ack_tests::real_full_worker_nacks_failed_sql_commit_without_advancing_cursor",
        failed_commit,
    )
}

fn report_capacity_and_sql_cause() -> Result<()> {
    use super::reporting::{ReportInputLimits, with_report_input_test_limits};
    let fixture = Fixture::new("Preserved report handler body.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let id = "report-handler-pending";
        let run = "report-handler-run";
        let account = source::report::account_key(&auth()).unwrap();
        let original = json!({
            "batches_v2": [{
                "request": { "session_id": id, "account_key": account, "book_id": BOOK,
                    "started_at": 100, "seconds": 60, "progress_write_allowed": true,
                    "active_reader_intent": false, "position": null },
                "run_id": run, "from": 0, "to": 60, "state": "pending",
                "attempted_at": null, "retry_after": 0, "attempts": 0, "error": null
            }],
            "reported_cursors": { "report-handler-run": 0 }, "compacted_runs": {}
        });
        state.store.set_setting("report.outbox.v1", &original)?;
        let result = || JobResult::ReadReport {
            session_id: id.into(),
            result: Ok(source::report::ReadReportReceipt {
                accepted_at: 200,
                seconds: 60,
                has_synckey: true,
            }),
        };
        let epoch = state.jobs.account_epoch();
        let account = state.jobs.session();
        let body = state.reader.body.clone();
        let capacity = state.jobs.metadata_capacity();
        assert!(!with_report_input_test_limits(
            ReportInputLimits {
                scratch_bytes: 0,
                ..Default::default()
            },
            || state.handle_job(JobEvent::new(77, epoch, result()))
        )?);
        assert_eq!(
            state
                .store
                .setting::<serde_json::Value>("report.outbox.v1")?,
            Some(original.clone())
        );
        assert!(
            state
                .store
                .setting::<serde_json::Value>(&format!("report.receipt:{id}"))?
                .is_none()
        );
        assert!(
            !state
                .store
                .setting::<bool>("report.storage_error.v1")?
                .unwrap_or(false)
        );
        assert_eq!(state.settings_status, tr("api_error"));
        assert_eq!(
            fixture.ui().global::<ShellModel>().get_toast(),
            tr("api_error")
        );
        assert_eq!(state.reader.body, body);
        assert_eq!(state.jobs.session(), account);
        assert_eq!(state.jobs.metadata_capacity(), capacity);
        assert!(!state.closing.get());

        let database = rusqlite::Connection::open(state.data_dir.join("library.sqlite"))?;
        database.execute_batch("ALTER TABLE settings RENAME TO report_handler_hidden_settings")?;
        let error = state
            .handle_job(JobEvent::new(78, epoch, result()))
            .unwrap_err();
        assert!(error.downcast_ref::<rusqlite::Error>().is_some());
        assert!(!super::reporting::report_capacity(&error));
        database.execute_batch("ALTER TABLE report_handler_hidden_settings RENAME TO settings")?;
        assert_eq!(
            state
                .store
                .setting::<serde_json::Value>("report.outbox.v1")?,
            Some(original)
        );
        assert!(
            state
                .store
                .setting::<serde_json::Value>(&format!("report.receipt:{id}"))?
                .is_none()
        );
        assert_eq!(state.reader.body, body);
        assert_eq!(state.jobs.session(), account);
    }
    fixture.finish()
}

#[test]
fn real_report_handler_capacity_is_nonfatal_while_unknown_sql_cause_is_preserved() -> Result<()> {
    isolated(
        "app::fresh_checkpoint_ack_tests::real_report_handler_capacity_is_nonfatal_while_unknown_sql_cause_is_preserved",
        report_capacity_and_sql_cause,
    )
}
