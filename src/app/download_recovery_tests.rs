//! Chapter failures retain independent durable retry state and completed work.

use super::*;
use crate::domain::{DownloadChapterRetry, DownloadChapterState, DownloadChapterStatus};

#[path = "download_independent_runtime_tests.rs"]
mod runtime_tests;

const NOTICE_KEY: &str = "download.notice.v1";
const REJECTION_ERROR: &str = "source.api.-10102";

fn commit_recovery_chapter(state: &mut App, index: u32) -> Result<()> {
    let text = format!("A complete recovery fixture chapter {index}.");
    let html = format!("<p>{text}</p>");
    let (packet, mut reply) = packet_event(state, index, &text, Some(&html), None)?;
    state.handle_job(packet)?;
    assert!(matches!(reply.try_recv(), Ok(Ok(()))));
    assert_eq!(
        state.store.chapter_body("fixture-book", index)?.as_deref(),
        Some(text.as_str())
    );
    Ok(())
}

fn chapter_retry_state(state: &App, task: &str, index: u32) -> Result<DownloadChapterState> {
    state
        .store
        .download_chapter_states(task)?
        .into_iter()
        .find(|chapter| chapter.chapter_index == index)
        .ok_or_else(|| anyhow::anyhow!("Missing fixture chapter retry state"))
}

fn fail_recovery_chapter(
    state: &mut App,
    index: u32,
    error: source::SourceError,
) -> Result<(DownloadTask, DownloadChapterRetry)> {
    let task = state.current_download.as_ref().unwrap();
    let id = task.id.clone();
    let ordinal = task
        .chapters
        .iter()
        .position(|chapter| *chapter == index)
        .unwrap();
    let chapter_id = state.store.chapter_uid(&task.book_id, index)?.unwrap();
    let (checkpoint, mut reply) = tokio::sync::oneshot::channel();
    let failed = event(
        state,
        JobResult::DownloadChapterFailed {
            task_id: id.clone(),
            chapter_id,
            chapter_index: index,
            ordinal,
            error,
            checkpoint,
        },
    );
    state.handle_job(failed)?;
    let retry = match reply.try_recv() {
        Ok(Ok(retry)) => retry,
        result => anyhow::bail!("A chapter failure did not receive a durable ACK: {result:?}"),
    };
    let saved = state.store.task(&id)?.unwrap();
    assert_eq!(state.current_download.as_ref(), Some(&saved));
    assert_eq!(saved.status, DownloadStatus::Running);
    Ok((saved, retry))
}

fn reject_recovery_read(
    state: &mut App,
    index: u32,
) -> Result<(DownloadTask, DownloadChapterRetry)> {
    fail_recovery_chapter(state, index, source::SourceError::Api("-10102".into()))
}

fn make_retry_due(state: &App, task: &str, index: u32) -> Result<()> {
    let before = state.store.task(task)?.unwrap();
    let chapter = chapter_retry_state(state, task, index)?;
    let connection = rusqlite::Connection::open(state.data_dir.join("library.sqlite"))?;
    assert_eq!(
        connection.execute(
            "UPDATE task_chapter_states SET retry_not_before=?1
             WHERE task_id=?2 AND ordinal=?3 AND status='retry_wait'",
            rusqlite::params![Utc::now().timestamp() - 1, task, chapter.ordinal],
        )?,
        1
    );
    // The fixture only moves this chapter's timer. The task fence is unchanged.
    assert_eq!(state.store.task(task)?.unwrap(), before);
    Ok(())
}

fn start_recovery_chapter(state: &mut App, index: u32) -> Result<()> {
    let task = state.current_download.as_ref().unwrap();
    let id = task.id.clone();
    let before = chapter_retry_state(state, &id, index)?;
    assert_eq!(before.status, DownloadChapterStatus::RetryWait);
    let (checkpoint, mut reply) = tokio::sync::oneshot::channel();
    let started = event(
        state,
        JobResult::DownloadChapterStarted {
            task_id: id.clone(),
            chapter_id: before.chapter_uid.clone(),
            chapter_index: index,
            ordinal: before.ordinal,
            checkpoint,
        },
    );
    state.handle_job(started)?;
    assert!(matches!(reply.try_recv(), Ok(Ok(()))));
    let running = chapter_retry_state(state, &id, index)?;
    assert_eq!(running.status, DownloadChapterStatus::Pending);
    assert_eq!(running.retry_count, before.retry_count);
    assert_eq!(running.retry_not_before, 0);
    assert!(running.error.is_none());
    assert_eq!(
        state.current_download.as_ref(),
        state.store.task(&id)?.as_ref()
    );
    Ok(())
}

fn assert_retry_deadline(
    chapter: &DownloadChapterState,
    retry: DownloadChapterRetry,
    before: i64,
    after: i64,
    delay: i64,
) {
    assert_eq!(chapter.status, DownloadChapterStatus::RetryWait);
    assert!(
        (before + delay..=after + delay).contains(&chapter.retry_not_before),
        "The chapter deadline must use the rejection time and expected delay"
    );
    assert!(matches!(retry, DownloadChapterRetry::RetryAt(at) if at == chapter.retry_not_before));
}

fn run_independent_completion(
    ui: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_remote(&mut state, 3)?;
    let original = begin_task(&mut state, vec![0, 1, 2], false)?;
    state.online = true;
    let route = state.route.clone();
    let account_epoch = state.jobs.account_epoch();
    assert!(!source::SourceError::Api("-10102".into()).retryable());
    assert!(!source::SourceError::Api("-10102".into()).requires_login());
    let (_, retry) = reject_recovery_read(&mut state, 0)?;
    assert!(matches!(retry, DownloadChapterRetry::RetryAt(_)));

    commit_recovery_chapter(&mut state, 2)?;
    let after_last = state.store.task(&original.id)?.unwrap();
    assert_eq!(after_last.completed_count, 1);
    assert_eq!(after_last.next_index, 0);
    commit_recovery_chapter(&mut state, 1)?;
    let saved = state.store.task(&original.id)?.unwrap();
    assert_eq!(saved.completed_count, 2);
    assert_eq!(saved.next_index, 0);
    assert_eq!(saved.status, DownloadStatus::Running);
    assert_eq!(saved.chapters, original.chapters);
    assert_eq!(saved.account_key, original.account_key);
    assert_eq!(state.current_download.as_ref(), Some(&saved));
    assert_eq!(state.route, route);
    assert_eq!(state.jobs.account_epoch(), account_epoch);
    assert!(state.online);
    assert_eq!(
        chapter_retry_state(&state, &saved.id, 0)?.status,
        DownloadChapterStatus::RetryWait
    );
    for index in [1, 2] {
        let complete = chapter_retry_state(&state, &saved.id, index)?;
        assert_eq!(complete.status, DownloadChapterStatus::Complete);
        assert!(complete.error.is_none());
    }

    let progress = event(
        &state,
        JobResult::DownloadProgress {
            task_id: saved.id.clone(),
            completed: 2,
            waiting: 1,
            blocked: 0,
            total: 3,
            title: "Independent fixture progress".into(),
        },
    );
    state.handle_job(progress)?;
    let label = i18n::format(
        "downloading_chapter_states",
        &[
            ("current", "2".into()),
            ("total", "3".into()),
            ("waiting", "1".into()),
            ("blocked", "0".into()),
        ],
    );
    assert_eq!(ui.global::<ShellModel>().get_task_label().as_str(), label);
    assert_eq!(ui.global::<LibraryModel>().get_status().as_str(), label);
    assert!((ui.global::<ShellModel>().get_dialog_progress() - 2.0 / 3.0).abs() < 0.001);
    assert!(ui.global::<LibraryModel>().get_download_notice());

    make_retry_due(&state, &saved.id, 0)?;
    start_recovery_chapter(&mut state, 0)?;
    commit_recovery_chapter(&mut state, 0)?;
    let completed = state.store.task(&saved.id)?.unwrap();
    assert_eq!(completed.completed_count, 3);
    assert_eq!(completed.next_index, 3);
    let recovered = chapter_retry_state(&state, &saved.id, 0)?;
    assert_eq!(recovered.status, DownloadChapterStatus::Complete);
    assert_eq!(recovered.retry_count, 0);
    assert_eq!(recovered.retry_not_before, 0);
    assert!(recovered.error.is_none());
    let finished = event(
        &state,
        JobResult::DownloadFinished {
            task_id: saved.id.clone(),
        },
    );
    state.handle_job(finished)?;
    assert!(state.current_download.is_none());
    assert_eq!(
        state.store.task(&saved.id)?.unwrap().status,
        DownloadStatus::Complete
    );
    assert_eq!(
        state.store.book("fixture-book")?.unwrap().cached_chapters,
        3
    );
    Ok(())
}

#[test]
fn a_rejected_chapter_keeps_other_completions_durable_and_counts_past_the_first_gap() -> Result<()>
{
    on_ui_thread(run_independent_completion)
}

fn run_independent_retry_budgets(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_remote(&mut state, 3)?;
    let original = begin_task(&mut state, vec![0, 1, 2], false)?;
    commit_recovery_chapter(&mut state, 0)?;
    for (attempt, delay) in [Some(60), Some(120), Some(240), None]
        .into_iter()
        .enumerate()
    {
        if attempt != 0 {
            make_retry_due(&state, &original.id, 1)?;
            start_recovery_chapter(&mut state, 1)?;
        }
        let before = Utc::now().timestamp();
        let (saved, retry) = reject_recovery_read(&mut state, 1)?;
        let after = Utc::now().timestamp();
        let chapter = chapter_retry_state(&state, &saved.id, 1)?;
        assert_eq!(chapter.retry_count, attempt as u8 + 1);
        assert_eq!(chapter.error.as_deref(), Some(REJECTION_ERROR));
        assert_eq!(saved.completed_count, 1);
        assert_eq!(saved.next_index, 1);
        if let Some(delay) = delay {
            assert_retry_deadline(&chapter, retry, before, after, delay);
        } else {
            assert_eq!(chapter.status, DownloadChapterStatus::Blocked);
            assert!(matches!(retry, DownloadChapterRetry::Blocked));
            assert_eq!(chapter.retry_not_before, 0);
        }
    }
    let before = Utc::now().timestamp();
    let (saved, retry) = reject_recovery_read(&mut state, 2)?;
    let after = Utc::now().timestamp();
    let independent = chapter_retry_state(&state, &saved.id, 2)?;
    assert_eq!(independent.retry_count, 1);
    assert_retry_deadline(&independent, retry, before, after, 60);
    assert_eq!(
        chapter_retry_state(&state, &saved.id, 1)?.status,
        DownloadChapterStatus::Blocked
    );
    assert_eq!(
        chapter_retry_state(&state, &saved.id, 0)?.status,
        DownloadChapterStatus::Complete
    );
    assert!(state.store.chapter("fixture-book", 0)?.unwrap().cached);
    assert!(state.downloads.is_empty());
    assert_eq!(
        state.current_download.as_ref().unwrap().status,
        DownloadStatus::Running
    );
    Ok(())
}

#[test]
fn retry_budgets_and_exhaustion_belong_to_each_chapter_without_pausing_the_book() -> Result<()> {
    on_ui_thread(run_independent_retry_budgets)
}

fn run_stale_progress_counts(
    ui: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_remote(&mut state, 4)?;
    let task = begin_task(&mut state, vec![0, 1, 2, 3], false)?;
    reject_recovery_read(&mut state, 0)?;
    fail_recovery_chapter(&mut state, 1, source::SourceError::UnlockRequired)?;
    commit_recovery_chapter(&mut state, 3)?;
    let stale = event(
        &state,
        JobResult::DownloadProgress {
            task_id: task.id.clone(),
            completed: 0,
            waiting: 0,
            blocked: 0,
            total: 4,
            title: "Older fixture progress".into(),
        },
    );
    state.handle_job(stale)?;
    let expected = |waiting: usize| {
        i18n::format(
            "downloading_chapter_states",
            &[
                ("current", "1".into()),
                ("total", "4".into()),
                ("waiting", waiting.to_string()),
                ("blocked", "1".into()),
            ],
        )
    };
    assert_eq!(
        ui.global::<ShellModel>().get_task_label().as_str(),
        expected(1)
    );
    assert_eq!(
        ui.global::<LibraryModel>().get_status().as_str(),
        expected(1)
    );
    assert_eq!(ui.global::<ShellModel>().get_dialog_progress(), 0.25);
    assert_eq!(state.current_download.as_ref().unwrap().completed_count, 1);
    assert_eq!(state.current_download.as_ref().unwrap().next_index, 0);

    make_retry_due(&state, &task.id, 0)?;
    start_recovery_chapter(&mut state, 0)?;
    assert_eq!(
        ui.global::<ShellModel>().get_task_label().as_str(),
        expected(0)
    );
    let before_attempt = event(
        &state,
        JobResult::DownloadProgress {
            task_id: task.id.clone(),
            completed: 0,
            waiting: 1,
            blocked: 1,
            total: 4,
            title: "Superseded retry wait".into(),
        },
    );
    state.handle_job(before_attempt)?;
    assert_eq!(
        ui.global::<ShellModel>().get_task_label().as_str(),
        expected(0)
    );
    assert_eq!(
        ui.global::<LibraryModel>().get_status().as_str(),
        expected(0)
    );
    assert_eq!(ui.global::<ShellModel>().get_dialog_progress(), 0.25);

    let after_cancel = event(
        &state,
        JobResult::DownloadProgress {
            task_id: task.id.clone(),
            completed: 0,
            waiting: 1,
            blocked: 1,
            total: 4,
            title: "Cancelled fixture progress".into(),
        },
    );
    state.reopen_running_download_dialog();
    state.shell_action("dialog-secondary", "")?;
    let cancelled = state.store.task(&task.id)?.unwrap();
    assert_eq!(cancelled.status, DownloadStatus::Cancelled);
    assert!(!state.handle_job(after_cancel)?);
    assert_eq!(state.store.task(&task.id)?.unwrap(), cancelled);
    assert!(ui.global::<ShellModel>().get_task_label().is_empty());
    assert!(!ui.global::<LibraryModel>().get_download_notice());
    assert!(state.download_notice.is_none());
    Ok(())
}

#[test]
fn delayed_progress_cannot_restore_old_counts_after_failure_retry_start_or_cancellation()
-> Result<()> {
    on_ui_thread(run_stale_progress_counts)
}

fn run_reloaded_retry_and_continue(
    ui: &AppWindow,
    application: &Rc<RefCell<App>>,
    directory: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_remote(&mut state, 3)?;
    let original = begin_task(&mut state, vec![0, 1, 2], false)?;
    commit_recovery_chapter(&mut state, 2)?;
    reject_recovery_read(&mut state, 0)?;
    let (_, blocked) = fail_recovery_chapter(&mut state, 1, source::SourceError::UnlockRequired)?;
    assert!(matches!(blocked, DownloadChapterRetry::Blocked));
    let waiting = chapter_retry_state(&state, &original.id, 0)?;
    let complete = chapter_retry_state(&state, &original.id, 2)?;
    let previous = state.current_download.take().unwrap();
    let mut paused = previous.clone();
    paused.status = DownloadStatus::Paused;
    paused.updated_at = Utc::now().timestamp();
    assert!(state.store.save_task_if_current(&previous, &paused)?);
    state.remember_download_notice(&paused)?;

    // Reopen persistence and reconstruct only bounded metadata, as startup does.
    state.downloads.clear();
    state.download_notice = None;
    state.store = Store::open(directory.join("library.sqlite"))?;
    assert!(state.reload_pending_downloads(false)?);
    state.restore_download_notice();
    state.render()?;
    assert!(ui.global::<LibraryModel>().get_download_notice());
    assert_eq!(chapter_retry_state(&state, &original.id, 0)?, waiting);
    state.open_download_notice()?;
    state.continue_download_notice()?;
    let resumed = state.store.task(&original.id)?.unwrap();
    assert_eq!(resumed.id, paused.id);
    assert_eq!(resumed.chapters, paused.chapters);
    assert_eq!(resumed.completed_count, 1);
    assert_eq!(resumed.next_index, 0);
    assert_eq!(resumed.status, DownloadStatus::Pending);
    assert_eq!(chapter_retry_state(&state, &original.id, 0)?, waiting);
    assert_eq!(chapter_retry_state(&state, &original.id, 2)?, complete);
    let reset = chapter_retry_state(&state, &original.id, 1)?;
    assert_eq!(reset.status, DownloadChapterStatus::Pending);
    assert_eq!(reset.retry_count, 0);
    assert_eq!(reset.retry_not_before, 0);
    assert!(reset.error.is_none());
    assert!(state.current_download.is_none());
    assert!(state.force_offline);

    // A resumed scheduler may own future timers while other ready chapters run.
    // The runtime is not driven here; the closed gate also prevents transport.
    state.jobs.download_reader_gate_for_tests().close();
    state.prefs.download_concurrency = 1;
    state.online = true;
    state.force_offline = false;
    assert!(state.reload_pending_downloads(true)?);
    state.start_next_download()?;
    let running = state.current_download.as_ref().unwrap().clone();
    assert_eq!(running.id, original.id);
    assert_eq!(running.completed_count, 1);
    assert_eq!(running.next_index, 0);
    assert_eq!(running.status, DownloadStatus::Running);
    assert_eq!(
        chapter_retry_state(&state, &original.id, 0)?.retry_not_before,
        waiting.retry_not_before
    );
    assert_eq!(
        chapter_retry_state(&state, &original.id, 0)?.retry_count,
        waiting.retry_count
    );
    assert_eq!(chapter_retry_state(&state, &original.id, 2)?, complete);
    assert_eq!(
        state.jobs.active_download().as_deref(),
        Some(original.id.as_str())
    );
    assert!(state.events.try_recv().is_err());
    state.force_offline = true;
    state.open_download_notice()?;
    assert_eq!(state.dialog_action, "downloading");
    state.shell_action("dialog-secondary", "")?;
    let cancelled = state.store.task(&original.id)?.unwrap();
    assert_eq!(cancelled.status, DownloadStatus::Cancelled);
    assert_eq!(cancelled.completed_count, 1);
    assert_eq!(cancelled.next_index, 0);
    assert!(state.current_download.is_none());
    assert!(state.jobs.active_download().is_none());
    assert!(state.download_notice.is_none());
    assert!(state.store.setting::<String>(NOTICE_KEY)?.is_none());
    assert!(!ui.global::<LibraryModel>().get_download_notice());
    assert!(state.reload_pending_downloads(true)?);
    assert!(state.downloads.iter().all(|task| task.id != original.id));
    Ok(())
}

#[test]
fn reload_and_explicit_continue_keep_completed_chapters_and_future_retry_deadlines() -> Result<()> {
    on_ui_thread(run_reloaded_retry_and_continue)
}

fn run_partial_failure_controls(
    ui: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_remote(&mut state, 2)?;
    let original = begin_task(&mut state, vec![0, 1], false)?;
    commit_recovery_chapter(&mut state, 1)?;
    let (_, retry) = fail_recovery_chapter(&mut state, 0, source::SourceError::ContentUnverified)?;
    assert!(matches!(retry, DownloadChapterRetry::Blocked));
    assert_eq!(
        state.current_download.as_ref().unwrap().status,
        DownloadStatus::Running
    );
    let incomplete = event(
        &state,
        JobResult::DownloadIncomplete {
            task_id: original.id.clone(),
            completed: 1,
            blocked: 1,
            total: 2,
        },
    );
    state.handle_job(incomplete)?;
    let failed = state.store.task(&original.id)?.unwrap();
    assert_eq!(failed.status, DownloadStatus::Failed);
    assert_eq!(failed.completed_count, 1);
    assert_eq!(failed.next_index, 0);
    assert!(state.current_download.is_none());
    state.render()?;
    assert!(ui.global::<LibraryModel>().get_download_notice());
    state.open_download_notice()?;
    assert_eq!(state.dialog_action, "download-continue");
    assert_eq!(
        ui.global::<ShellModel>().get_dialog_primary().as_str(),
        tr("download_retry_chapters")
    );
    assert_eq!(
        ui.global::<ShellModel>().get_dialog_secondary().as_str(),
        tr("cancel_download")
    );
    state.shell_action("dialog-primary", "")?;
    let pending = state.store.task(&original.id)?.unwrap();
    assert_eq!(pending.id, failed.id);
    assert_eq!(pending.chapters, failed.chapters);
    assert_eq!(pending.status, DownloadStatus::Pending);
    assert_eq!(pending.completed_count, 1);
    assert_eq!(pending.next_index, 0);
    assert!(pending.error.is_none());
    assert_eq!(
        chapter_retry_state(&state, &original.id, 0)?.status,
        DownloadChapterStatus::Pending
    );
    assert_eq!(
        chapter_retry_state(&state, &original.id, 1)?.status,
        DownloadChapterStatus::Complete
    );
    assert!(state.store.chapter("fixture-book", 1)?.unwrap().cached);
    assert_eq!(
        state.store.book("fixture-book")?.unwrap().cached_chapters,
        1
    );
    assert_eq!(state.downloads.len(), 1);
    assert_eq!(state.downloads.front(), Some(&pending));
    Ok(())
}

#[test]
fn an_incomplete_selection_offers_retry_and_cancel_without_restarting_completed_chapters()
-> Result<()> {
    on_ui_thread(run_partial_failure_controls)
}
