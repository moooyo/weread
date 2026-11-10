//! Genuine cached-metadata operations in the bounded, offline UI fixture.

use super::underline_projection_tests::{BOOK, CHAPTER, Fixture, isolated};
use super::*;
use crate::{
    jobs::JobResult,
    platform::{ManagedSleepState, PlatformError, RenderSink},
};
use anyhow::Context;
use rusqlite::params;
use serde_json::json;

fn underline(text: &str) -> RemoteUnderline {
    RemoteUnderline {
        text: text.into(),
        range: "opaque-range".into(),
        count: 2,
        thoughts: 3,
    }
}

fn session(account: &str) -> source::AuthSession {
    source::AuthSession {
        user_vid: account.into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), account.into()),
            ("wr_skey".into(), "synthetic-cached-metadata-session".into()),
        ]),
        ..Default::default()
    }
}

fn cache_sql(state: &App) -> Result<rusqlite::Connection> {
    Ok(rusqlite::Connection::open(
        state.data_dir.join("library.sqlite"),
    )?)
}

fn cache_key() -> String {
    format!("underlines:{BOOK}:{CHAPTER}")
}

fn run_underline_sql_rejection() -> Result<()> {
    let fixture = Fixture::new("keep body", vec![underline("keep")], true)?;
    {
        let mut state = fixture.app();
        state
            .jobs
            .replace_session(session("cached-underline-account"))?;
        state.reader.draft.set_query("Pending local draft");
        state.reader.selection = Some((0, 4));
        fixture
            .ui()
            .global::<ReaderModel>()
            .set_writing_thought(true);
        let account = state.jobs.session();
        let body = state.reader.body.clone();
        let revision = state.reader.underline_revision;
        let reads = state.store.chapter_body_read_count();
        let connection = cache_sql(&state)?;
        let bad_values = [
            rusqlite::types::Value::Text(format!("[{{\"text\":\"{}\"}}]", "x".repeat(96))),
            rusqlite::types::Value::Text("[{}, {}, {}]".into()),
            rusqlite::types::Value::Text("{\"text\":\"wrong container\"}".into()),
            rusqlite::types::Value::Text("[".into()),
            rusqlite::types::Value::Blob(b"[]".to_vec()),
        ];
        for bad in bad_values {
            connection.execute(
                "UPDATE settings SET value=?2 WHERE key=?1",
                params![cache_key(), bad],
            )?;
            state.load_reader_underlines_with_test_allowance(64, 2)?;
            assert_eq!(state.reader.underline_revision, revision);
            assert_eq!(state.reader.underlines[0].text, "keep");
            assert_eq!(state.reader.body, body);
            assert_eq!(state.reader.draft.query, "Pending local draft");
            assert_eq!(state.reader.selection, Some((0, 4)));
            assert!(fixture.ui().global::<ReaderModel>().get_writing_thought());
            assert_eq!(state.jobs.session(), account);
            assert_eq!(state.store.chapter_body_read_count(), reads);
            let saved: rusqlite::types::Value = connection.query_row(
                "SELECT value FROM settings WHERE key=?1",
                [cache_key()],
                |row| row.get(0),
            )?;
            assert_eq!(saved, bad);
            assert_eq!(state.route, Route::Reader(BOOK.into(), 0));
        }
    }
    fixture.finish()
}

fn run_underline_field_and_serde_rejection() -> Result<()> {
    let fixture = Fixture::new("keep body", vec![underline("keep")], true)?;
    {
        let mut state = fixture.app();
        let revision = state.reader.underline_revision;
        let reads = state.store.chapter_body_read_count();
        let quote = "x".repeat(16 * 1024 + 1);
        let data = vec![json!({"text":quote,"range":"opaque","thoughts":4})];
        state.store.set_setting(&cache_key(), &data)?;
        state.load_reader_underlines(false)?;
        assert_eq!(state.reader.underline_revision, revision);
        assert_eq!(state.reader.underlines[0].text, "keep");
        assert_eq!(
            state
                .store
                .setting::<Vec<serde_json::Value>>(&cache_key())?,
            Some(data)
        );
        let deep = format!(
            "[{{\"text\":\"keep\",\"ignored\":{}0{}}}]",
            "[".repeat(150),
            "]".repeat(150)
        );
        let connection = cache_sql(&state)?;
        connection.execute(
            "UPDATE settings SET value=?2 WHERE key=?1",
            params![cache_key(), deep],
        )?;
        state.load_reader_underlines(false)?;
        assert_eq!(state.reader.underline_revision, revision);
        let saved: String = connection.query_row(
            "SELECT value FROM settings WHERE key=?1",
            [cache_key()],
            |row| row.get(0),
        )?;
        assert_eq!(saved, deep);
        assert_eq!(state.store.chapter_body_read_count(), reads);
        assert_eq!(state.reader.body, "keep body");
        assert!(state.jobs.session().is_none());
    }
    fixture.finish()
}

fn run_underline_hidden_reload() -> Result<()> {
    let fixture = Fixture::new("A\u{4e2d}\u{1f642} body", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let reads = state.store.bounded_array_read_count();
        let body_reads = state.store.chapter_body_read_count();
        for _ in 0..16 {
            state.load_reader_underlines_with_test_allowance(0, 0)?;
        }
        assert_eq!(state.store.bounded_array_read_count(), reads);
        assert_eq!(state.store.chapter_body_read_count(), body_reads);
        let data = vec![
            json!({"range":17,"text":"\u{4e2d}\u{1f642}","count":2,"thoughts":7}),
            json!({"range":23,"text":93,"count":4,"thoughts":9}),
        ];
        state.store.set_setting(&cache_key(), &data)?;
        state.reader_action("popular", "toggle")?;
        assert_eq!(state.store.bounded_array_read_count(), reads + 1);
        assert_eq!(state.store.chapter_body_read_count(), body_reads);
        assert_eq!(state.reader.underlines[0].range, "17");
        assert_eq!(state.reader.underlines[0].text, "\u{4e2d}\u{1f642}");
        assert_eq!(state.reader.underlines[0].thoughts, 7);
        assert_eq!(state.reader.underlines[1].text, "93");
        assert_eq!(state.reader.underlines[1].thoughts, 9);
        state.reader_action("popular", "toggle")?;
        assert!(state.reader.underlines.is_empty());
        state
            .store
            .set_setting(&cache_key(), &vec![underline("body")])?;
        state.reader_action("popular", "toggle")?;
        assert_eq!(state.store.bounded_array_read_count(), reads + 2);
        assert_eq!(state.reader.underlines[0].text, "body");
        assert_eq!(state.reader.body, "A\u{4e2d}\u{1f642} body");
    }
    fixture.finish()
}

struct RunningResidentSink;

impl RenderSink for RunningResidentSink {
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

const OTHER: &str = "other-catalog-intent";

fn seed_waiters(state: &mut App) -> Result<()> {
    state
        .jobs
        .replace_session(session("catalog-waiting-account"))?;
    state
        .platform
        .set_render_sink(Arc::new(RunningResidentSink))?;
    state.pending_read = Some((BOOK.into(), 0));
    state
        .pending_catalog_downloads
        .extend([BOOK.into(), OTHER.into()]);
    state.pending_catalog_images.insert(BOOK.into(), true);
    state.pending_catalog_images.insert(OTHER.into(), false);
    state.catalog_requests.insert(BOOK.into(), 401);
    state.catalog_requests.insert(OTHER.into(), 402);
    state.prepare_resident_sleep()?;
    state.resume_resident_sleep()?;
    assert!(state.resident_lifecycle.retry_intents().0.contains(BOOK));
    assert!(state.resident_lifecycle.retry_intents().0.contains(OTHER));
    Ok(())
}

fn catalog_event(state: &mut App, rows: Vec<source::RemoteChapter>) -> JobEvent {
    let request = state.request();
    state.catalog_requests.insert(BOOK.into(), request);
    JobEvent::new(
        request,
        state.jobs.account_epoch(),
        JobResult::Catalog {
            book_id: BOOK.into(),
            result: Ok(rows),
        },
    )
}

fn remote_chapter() -> source::RemoteChapter {
    source::RemoteChapter {
        uid: CHAPTER.into(),
        title: "Catalog row".into(),
        ..Default::default()
    }
}

fn assert_waiters_settled(state: &App) {
    assert!(state.pending_read.is_none());
    assert!(!state.pending_catalog_downloads.contains(BOOK));
    assert!(!state.pending_catalog_images.contains_key(BOOK));
    assert!(!state.catalog_requests.contains_key(BOOK));
    assert!(!state.resident_lifecycle.retry_intents().0.contains(BOOK));
    assert!(state.pending_catalog_downloads.contains(OTHER));
    assert_eq!(state.pending_catalog_images.get(OTHER), Some(&false));
    assert!(state.resident_lifecycle.retry_intents().0.contains(OTHER));
}

fn run_catalog_typed_rejection() -> Result<()> {
    let fixture = Fixture::new("Retained reader body", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        seed_waiters(&mut state)?;
        state.reader.draft.set_query("Retained local draft");
        let account = state.jobs.session();
        let body_reads = state.store.chapter_body_read_count();
        let event = catalog_event(&mut state, vec![remote_chapter(), remote_chapter()]);
        state.handle_job(event)?;
        assert_waiters_settled(&state);
        assert_eq!(state.store.chapter_body_read_count(), body_reads);
        assert_eq!(state.jobs.session(), account);
        assert_eq!(state.reader.body, "Retained reader body");
        assert_eq!(state.reader.draft.query, "Retained local draft");
        for _ in 0..3 {
            state.prepare_resident_sleep()?;
            state.resume_resident_sleep()?;
            assert_waiters_settled(&state);
        }
        assert_eq!(state.route, Route::Reader(BOOK.into(), 0));
    }
    fixture.finish()
}

fn run_catalog_store_rejection() -> Result<()> {
    let fixture = Fixture::new("Retained reader body", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        seed_waiters(&mut state)?;
        let connection = cache_sql(&state)?;
        let original_owners = {
            let mut statement = connection.prepare(
                "SELECT chapter_index,content_account_key FROM chapters WHERE book_id=?1 ORDER BY chapter_index",
            )?;
            let rows = statement.query_map([BOOK], |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let oversized_owner = "o".repeat(16 * 1024 + 1);
        connection.execute(
            "UPDATE chapters SET content_account_key=?2 WHERE book_id=?1",
            params![BOOK, oversized_owner],
        )?;
        let rejection = state
            .store
            .chapter_headers(BOOK)
            .err()
            .context("The synthetic owner must exceed the header allowance")?;
        assert!(
            rejection
                .downcast_ref::<crate::store::CachedResourceLimit>()
                .is_some()
        );
        state
            .reader
            .draft
            .set_query("Retained catalog rejection draft");
        state.reader.selection = Some((0, 8));
        fixture
            .ui()
            .global::<ReaderModel>()
            .set_writing_thought(true);
        let body_reads = state.store.chapter_body_read_count();
        let account = state.jobs.session();
        let event = catalog_event(&mut state, vec![remote_chapter()]);
        state
            .handle_job(event)
            .context("Settle the injected catalog metadata rejection")?;
        assert_waiters_settled(&state);
        assert_eq!(state.store.chapter_body_read_count(), body_reads);
        assert_eq!(state.jobs.session(), account);
        assert_eq!(state.reader.body, "Retained reader body");
        assert_eq!(state.reader.draft.query, "Retained catalog rejection draft");
        assert_eq!(state.reader.selection, Some((0, 8)));
        assert!(fixture.ui().global::<ReaderModel>().get_writing_thought());
        assert_eq!(state.store.chapter_uid(BOOK, 0)?.as_deref(), Some(CHAPTER));
        let saved: String = connection.query_row(
            "SELECT content_account_key FROM chapters WHERE book_id=?1 AND chapter_index=0",
            [BOOK],
            |row| row.get(0),
        )?;
        assert_eq!(saved, oversized_owner);
        let saved_body: String = connection.query_row(
            "SELECT body FROM chapters WHERE book_id=?1 AND chapter_index=0",
            [BOOK],
            |row| row.get(0),
        )?;
        assert_eq!(saved_body, "Retained reader body");
        let still_rejected = state
            .store
            .chapter_metadata(BOOK)
            .err()
            .context("Catalog settlement must not silently repair the synthetic owner")?;
        assert!(
            still_rejected
                .downcast_ref::<crate::store::CachedResourceLimit>()
                .is_some()
        );
        assert_waiters_settled(&state);
        // Keep the injected failure active through all rejection assertions.
        // Restore only the fixture's original owners before the normal terminal
        // saver reads fraction metadata; no production error is swallowed.
        for (index, owner) in &original_owners {
            assert_eq!(connection.execute(
                "UPDATE chapters SET content_account_key=?3 WHERE book_id=?1 AND chapter_index=?2",
                params![BOOK, index, owner],
            )?, 1);
        }
        let restored = state.store.chapter_headers(BOOK)?;
        assert!(restored.iter().all(|chapter| {
            original_owners.iter().any(|(index, owner)| {
                chapter.index == *index && chapter.content_account_key == *owner
            })
        }));
        assert_eq!(state.jobs.session(), account);
        assert_eq!(state.reader.body, "Retained reader body");
        assert_waiters_settled(&state);
    }
    fixture
        .finish()
        .context("Close the catalog fixture after removing its injected owner fault")
}

fn run_catalog_sql_failure() -> Result<()> {
    let fixture = Fixture::new("Retained reader body", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        seed_waiters(&mut state)?;
        let account = state.jobs.session();
        let connection = cache_sql(&state)?;
        connection.execute_batch("ALTER TABLE chapters RENAME TO temporarily_hidden_chapters")?;
        let event = catalog_event(&mut state, vec![remote_chapter()]);
        let error = state.handle_job(event).unwrap_err();
        assert!(error.downcast_ref::<rusqlite::Error>().is_some());
        assert!(
            error
                .downcast_ref::<crate::store::CachedResourceLimit>()
                .is_none()
        );
        assert_waiters_settled(&state);
        assert_eq!(state.jobs.session(), account);
        assert_eq!(state.reader.body, "Retained reader body");
        connection.execute_batch("ALTER TABLE temporarily_hidden_chapters RENAME TO chapters")?;
        assert_eq!(
            state.store.chapter_body(BOOK, 0)?.as_deref(),
            Some("Retained reader body")
        );
    }
    fixture.finish()
}

fn begin_cached_task(state: &mut App) -> Result<DownloadTask> {
    state
        .jobs
        .replace_session(session("cached-download-account"))?;
    let owner = state
        .jobs
        .session()
        .as_ref()
        .and_then(library::download_account_key)
        .unwrap();
    let task = DownloadTask {
        id: "app-cached-task".into(),
        book_id: BOOK.into(),
        account_key: owner,
        chapters: vec![0, 1],
        status: DownloadStatus::Running,
        include_images: false,
        updated_at: 100,
        ..Default::default()
    };
    state.store.save_task(&task)?;
    state
        .store
        .prepare_download_chapter_states(&task, task.updated_at)?;
    let task = state.store.task(&task.id)?.unwrap();
    cache_sql(state)?.execute(
        "UPDATE chapters SET content_status='full',content_account_key=?2 WHERE book_id=?1",
        params![BOOK, task.account_key],
    )?;
    state.download_request = state.request();
    state.current_download = Some(task.clone());
    Ok(task)
}

fn cached_event(
    state: &App,
    uid: &str,
    index: u32,
) -> (JobEvent, tokio::sync::oneshot::Receiver<source::Result<()>>) {
    let task = state.current_download.as_ref().unwrap();
    let (checkpoint, reply) = tokio::sync::oneshot::channel();
    (
        JobEvent::new(
            state.download_request,
            state.jobs.account_epoch(),
            JobResult::CachedDownloaded {
                task_id: task.id.clone(),
                chapter_id: uid.into(),
                chapter_index: index,
                next_index: task
                    .chapters
                    .iter()
                    .position(|selected| *selected == index)
                    .unwrap()
                    + 1,
                checkpoint,
            },
        ),
        reply,
    )
}

fn run_cached_background_checkpoint() -> Result<()> {
    let fixture = Fixture::new("Retained cached body", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let task = begin_cached_task(&mut state)?;
        state.route = Route::Library;
        let reads = state.store.chapter_body_read_count();
        let all_bytes = state.store.materialized_chapter_bytes();
        let count = state.store.book(BOOK)?.unwrap().cached_chapters;
        let (event, mut reply) = cached_event(&state, CHAPTER, 0);
        assert!(!state.handle_job(event)?);
        reply.try_recv()?.map_err(anyhow::Error::from)?;
        assert_eq!(state.current_download.as_ref().unwrap().next_index, 1);
        assert_eq!(state.store.task(&task.id)?.unwrap().next_index, 1);
        assert_eq!(state.store.chapter_body_read_count(), reads);
        assert_eq!(state.store.materialized_chapter_bytes(), all_bytes);
        assert_eq!(state.store.book(BOOK)?.unwrap().cached_chapters, count);
        let (event, mut reply) = cached_event(&state, "underline-fixture-chapter-1", 1);
        assert!(!state.handle_job(event)?);
        reply.try_recv()?.map_err(anyhow::Error::from)?;
        assert_eq!(state.store.task(&task.id)?.unwrap().next_index, 2);
        assert_eq!(state.store.chapter_body_read_count(), reads);
        assert_eq!(state.store.book(BOOK)?.unwrap().cached_chapters, count);
        assert_eq!(state.reader.body, "Retained cached body");
    }
    fixture.finish()
}

fn run_cached_visible_checkpoint() -> Result<()> {
    let fixture = Fixture::new("Retained visible body", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let task = begin_cached_task(&mut state)?;
        let account = state.jobs.session();
        let cached_count = state.store.book(BOOK)?.unwrap().cached_chapters;
        state.pending_read = Some((BOOK.into(), 0));
        let connection = cache_sql(&state)?;
        let other_body: Option<String> = connection.query_row(
            "SELECT body FROM chapters WHERE book_id=?1 AND chapter_index=1",
            [BOOK],
            |row| row.get(0),
        )?;
        connection.execute(
            "UPDATE chapters SET body=x'ff' WHERE book_id=?1 AND chapter_index=1",
            [BOOK],
        )?;
        // A noncurrent BLOB is legal scalar metadata, but any body-bearing
        // Store getter rejects it instead of producing a String. The unchanged
        // corpus counter also rejects an accidental all-body scan from row 0.
        let poisoned = state
            .store
            .chapter_body(BOOK, 1)
            .err()
            .context("The noncurrent body guard must reject String materialization")?;
        assert!(poisoned.downcast_ref::<rusqlite::Error>().is_some());
        let headers = state.store.chapter_metadata(BOOK)?;
        assert!(headers.iter().all(|chapter| chapter.body.is_none()));
        assert!(crate::jobs::DownloadPlan::from_headers(&task, &headers).is_ok());
        let all_bytes = state.store.materialized_chapter_bytes();
        let reflow_start = state.store.chapter_body_read_count();
        state.store.reset_selected_metadata_read_stats();
        state.reflow_reader()?;
        let reflow_reads = state.store.chapter_body_read_count() - reflow_start;
        assert_eq!(reflow_reads, 0);
        let reflow_metadata = state.store.selected_metadata_read_stats();
        assert_eq!(reflow_metadata.body_borrows, 1);
        assert_eq!(reflow_metadata.body_scan_bytes, state.reader.body.len());
        assert_eq!(state.store.materialized_chapter_bytes(), all_bytes);
        let load_start = state.store.chapter_body_read_count();
        state.load_reader(BOOK, 0, None)?;
        let load_reads = state.store.chapter_body_read_count() - load_start;
        // One necessary current body load; reflow borrows its selected proof
        // and never materializes the poisoned noncurrent body or whole corpus.
        assert_eq!(load_reads, 1);
        assert_eq!(state.store.materialized_chapter_bytes(), all_bytes);
        connection.execute_batch(
            "CREATE TRIGGER refuse_cached_resume_save BEFORE INSERT ON settings
             WHEN NEW.key='reader.resume.v1'
             BEGIN SELECT RAISE(FAIL, 'synthetic cached resume save failure'); END;",
        )?;
        let session_start = state.store.chapter_body_read_count();
        state.store.reset_selected_metadata_read_stats();
        let save_error = state.save_reader_session().unwrap_err();
        assert!(save_error.downcast_ref::<rusqlite::Error>().is_some());
        assert!(format!("{save_error:#}").contains("synthetic cached resume save failure"));
        let session_reads = state.store.chapter_body_read_count() - session_start;
        assert_eq!(session_reads, 0);
        assert_eq!(state.store.selected_metadata_read_stats().body_borrows, 0);
        assert!(
            state
                .reader
                .paginator
                .as_ref()
                .is_some_and(|paginator| !paginator.current_page().lines.is_empty())
        );
        assert_eq!(state.store.materialized_chapter_bytes(), all_bytes);
        assert_eq!(state.store.task(&task.id)?.unwrap(), task);
        // Calibration must not stand in for the visible packet's waiting intent.
        assert_eq!(state.route, Route::Reader(BOOK.into(), 0));
        assert_eq!(state.reader.book_id, BOOK);
        assert_eq!(state.reader.chapter_index, 0);
        state.pending_read = Some((BOOK.into(), 0));
        assert_eq!(state.pending_read, Some((BOOK.into(), 0)));
        let reads = state.store.chapter_body_read_count();
        let (event, mut reply) = cached_event(&state, CHAPTER, 0);
        assert!(state.handle_job(event)?);
        reply.try_recv()?.map_err(anyhow::Error::from)?;
        assert_eq!(state.store.task(&task.id)?.unwrap().next_index, 1);
        assert!(state.pending_read.is_none());
        assert_eq!(state.reader.body, "Retained visible body");
        assert_eq!(state.reader.chapter_index, 0);
        assert_eq!(state.reader.book_id, BOOK);
        assert_eq!(state.jobs.session(), account);
        assert_eq!(
            state.store.book(BOOK)?.unwrap().cached_chapters,
            cached_count
        );
        assert!(state.reader.paginator.is_some());
        assert_eq!(state.store.materialized_chapter_bytes(), all_bytes);
        // The packet follows the independently checked loader/reflow/session
        // paths, all under the active noncurrent-body guard: one loader-owned
        // current body and no reflow/session copy, even when session save fails.
        assert_eq!(
            state.store.chapter_body_read_count() - reads,
            load_reads + session_reads
        );
        assert_eq!(load_reads + session_reads, 1);
        let guarded_body: rusqlite::types::Value = connection.query_row(
            "SELECT body FROM chapters WHERE book_id=?1 AND chapter_index=1",
            [BOOK],
            |row| row.get(0),
        )?;
        assert_eq!(guarded_body, rusqlite::types::Value::Blob(vec![0xff]));
        // A later display/session failure cannot NACK an already durable cursor.
        connection.execute_batch("DROP TRIGGER refuse_cached_resume_save")?;
        assert_eq!(
            connection.execute(
                "UPDATE chapters SET body=?2 WHERE book_id=?1 AND chapter_index=1",
                params![BOOK, other_body],
            )?,
            1
        );
    }
    fixture.finish()
}

fn run_cached_event_fences() -> Result<()> {
    let fixture = Fixture::new("Retained cached body", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let task = begin_cached_task(&mut state)?;
        state.reader.draft.set_query("Keep local draft");
        state.reader.selection = Some((0, 8));
        let reads = state.store.chapter_body_read_count();
        let connection = cache_sql(&state)?;
        let whitespace = "\t\n\u{85}\u{a0}\u{2000}\u{3000}";
        connection.execute(
            "UPDATE chapters SET body=?2 WHERE book_id=?1 AND chapter_index=0",
            params![BOOK, whitespace],
        )?;
        assert!(state.store.chapter_headers(BOOK)?[0].cached);
        let (event, mut reply) = cached_event(&state, CHAPTER, 0);
        assert!(!state.handle_job(event)?);
        assert!(matches!(
            reply.try_recv(),
            Ok(Err(source::SourceError::ContentChanged))
        ));
        assert_eq!(state.store.task(&task.id)?.unwrap(), task);
        let saved: String = connection.query_row(
            "SELECT body FROM chapters WHERE book_id=?1 AND chapter_index=0",
            [BOOK],
            |row| row.get(0),
        )?;
        assert_eq!(saved, whitespace);
        connection.execute(
            "UPDATE chapters SET body=?2 WHERE book_id=?1 AND chapter_index=0",
            params![BOOK, &state.reader.body],
        )?;
        let (event, mut reply) = cached_event(&state, "wrong-uid", 0);
        assert!(!state.handle_job(event)?);
        assert!(matches!(
            reply.try_recv(),
            Ok(Err(source::SourceError::CheckpointFailed))
        ));
        assert_eq!(state.store.task(&task.id)?.unwrap(), task);
        let (old_event, mut old_reply) = cached_event(&state, CHAPTER, 0);
        state
            .jobs
            .replace_session(session("replacement-download-account"))?;
        assert!(!state.handle_job(old_event)?);
        assert!(matches!(
            old_reply.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        ));
        let (event, mut reply) = cached_event(&state, CHAPTER, 0);
        assert!(!state.handle_job(event)?);
        assert!(matches!(
            reply.try_recv(),
            Ok(Err(source::SourceError::CheckpointFailed))
        ));
        assert_eq!(state.store.task(&task.id)?.unwrap(), task);
        assert_eq!(state.current_download.as_ref().unwrap(), &task);
        assert_eq!(state.reader.body, "Retained cached body");
        assert_eq!(state.reader.draft.query, "Keep local draft");
        assert_eq!(state.reader.selection, Some((0, 8)));
        assert_eq!(state.store.chapter_body_read_count(), reads);
        assert_eq!(
            state.jobs.session().unwrap().user_vid,
            "replacement-download-account"
        );
    }
    fixture.finish()
}

#[test]
fn cached_underline_sql_rejections_keep_admitted_model_draft_body_and_account() -> Result<()> {
    isolated(
        "app::cached_metadata_tests::cached_underline_sql_rejections_keep_admitted_model_draft_body_and_account",
        run_underline_sql_rejection,
    )
}

#[test]
fn cached_underline_field_and_serde_limits_leave_the_original_cache_untouched() -> Result<()> {
    isolated(
        "app::cached_metadata_tests::cached_underline_field_and_serde_limits_leave_the_original_cache_untouched",
        run_underline_field_and_serde_rejection,
    )
}

#[test]
fn hidden_cached_underlines_do_no_sql_read_and_reload_numeric_unicode_on_enable() -> Result<()> {
    isolated(
        "app::cached_metadata_tests::hidden_cached_underlines_do_no_sql_read_and_reload_numeric_unicode_on_enable",
        run_underline_hidden_reload,
    )
}

#[test]
fn rejected_catalog_settles_only_its_waiters_and_does_not_resume_them_after_sleep() -> Result<()> {
    isolated(
        "app::cached_metadata_tests::rejected_catalog_settles_only_its_waiters_and_does_not_resume_them_after_sleep",
        run_catalog_typed_rejection,
    )
}

#[test]
fn catalog_store_admission_rejection_settles_waiters_without_deleting_content() -> Result<()> {
    isolated(
        "app::cached_metadata_tests::catalog_store_admission_rejection_settles_waiters_without_deleting_content",
        run_catalog_store_rejection,
    )
}

#[test]
fn catalog_sql_failure_keeps_its_typed_cause_and_settles_waiting_intents() -> Result<()> {
    isolated(
        "app::cached_metadata_tests::catalog_sql_failure_keeps_its_typed_cause_and_settles_waiting_intents",
        run_catalog_sql_failure,
    )
}

#[test]
fn cached_background_download_checkpoints_ack_without_reading_any_body() -> Result<()> {
    isolated(
        "app::cached_metadata_tests::cached_background_download_checkpoints_ack_without_reading_any_body",
        run_cached_background_checkpoint,
    )
}

#[test]
fn cached_visible_download_checkpoint_loads_only_its_actual_reader_chapter() -> Result<()> {
    isolated(
        "app::cached_metadata_tests::cached_visible_download_checkpoint_loads_only_its_actual_reader_chapter",
        run_cached_visible_checkpoint,
    )
}

#[test]
fn cached_download_event_catalog_and_account_fences_nack_without_advancing() -> Result<()> {
    isolated(
        "app::cached_metadata_tests::cached_download_event_catalog_and_account_fences_nack_without_advancing",
        run_cached_event_fences,
    )
}
