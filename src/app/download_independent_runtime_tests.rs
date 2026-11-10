//! Genuine controller dispatch keeps later chapters moving after a rejected read.

use super::*;
use crate::app::underline_projection_tests::{Fixture, isolated};
use crate::domain::DownloadChapterStatus;
use crate::source::report::{AllocationHttpFixture, AllocationReply};
use serde_json::json;

const BOOK: &str = "independent-worker-book";
const TASK: &str = "independent-real-worker-task";
const ACCOUNT: &str = "independent-worker-account";
const TEXT: &str = "Reader fixture.";
const SHARD: &str = "DF1103CA0CD3800B1FA3F842EB367223aPGh0bWw+PJvGZHk+T4A+UmVhZGVyIGZpeHR1cwPPHC9mUjwvYm9keuP8L2h0bWw+";

fn chapter_uid(index: u32) -> String {
    format!("independent-worker-chapter-{index}")
}

fn worker_auth() -> source::AuthSession {
    source::AuthSession {
        api_key: "synthetic-independent-worker-key".into(),
        user_vid: ACCOUNT.into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), ACCOUNT.into()),
            (
                "wr_skey".into(),
                "synthetic-independent-worker-session".into(),
            ),
        ]),
        ..Default::default()
    }
}

fn reader_page(index: u32) -> String {
    let uid = chapter_uid(index);
    let state = json!({
        "user": { "vid": ACCOUNT },
        "reader": {
            "bookId": BOOK,
            "bookInfo": { "bookId": BOOK, "paid": 1, "payType": 1 },
            "chapterInfos": [{ "chapterUid": uid, "chapterIdx": index + 1, "wordCount": 20 }],
            "currentChapter": { "chapterUid": uid },
            "psvts": format!("independent-worker-clock-{index}"),
            "pclts": "independent-worker-client-clock"
        }
    });
    format!("<script>window.__INITIAL_STATE__={state};</script>")
}

fn independent_server() -> Result<AllocationHttpFixture> {
    let reader_paths = (0..3)
        .map(|index| {
            let url = source::protocol::reader_url_checked(BOOK, Some(&chapter_uid(index)))?;
            Ok((url::Url::parse(&url)?.path().to_owned(), index))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(AllocationHttpFixture::new(move |request| {
        if let Some(&index) = reader_paths.get(request.path()) {
            return AllocationReply::text(reader_page(index));
        }
        assert_eq!(request.method, "POST");
        let payload = request.json();
        let clock = payload["ps"].as_str().unwrap();
        match request.path() {
            "/web/book/chapter/e_0" if clock == "independent-worker-clock-0" => {
                AllocationReply::text(r#"{"errCode":-10102}"#)
            }
            "/web/book/chapter/e_0" => AllocationReply::text(SHARD),
            "/web/book/chapter/e_1" | "/web/book/chapter/e_3" => AllocationReply::text(""),
            path => panic!("Unexpected independent worker request: {path}"),
        }
    }))
}

fn seed_independent_task(state: &mut App) -> Result<(Annotation, Bookmark, ReadingPosition)> {
    let mut book = Book::new(BOOK, "Independent worker book");
    book.source = "weread".into();
    book.total_chapters = 3;
    book.on_shelf = true;
    state.store.upsert_book(&book)?;
    for index in 0..3 {
        state.store.upsert_chapter(&Chapter {
            id: chapter_uid(index),
            book_id: BOOK.into(),
            index,
            title: format!("Independent worker chapter {index}"),
            body: (index == 1).then(|| TEXT.into()),
            content_status: ContentStatus::Unknown,
            ..Default::default()
        })?;
    }
    let note = Annotation {
        id: "independent-worker-note".into(),
        book_id: BOOK.into(),
        chapter_index: 1,
        start: 0,
        end: 6,
        quote: "Reader".into(),
        note: "Retained independent worker note".into(),
        created_at: 20,
    };
    let bookmark = Bookmark {
        id: "independent-worker-bookmark".into(),
        book_id: BOOK.into(),
        chapter_index: 1,
        char_offset: 7,
        label: "Retained independent worker bookmark".into(),
        created_at: 21,
    };
    let position = ReadingPosition {
        book_id: BOOK.into(),
        chapter_index: 1,
        char_offset: 7,
        updated_at: 22,
        device: "synthetic-independent-worker".into(),
    };
    state.store.save_annotation(&note)?;
    state.store.save_bookmark(&bookmark)?;
    state.store.save_position(&position)?;
    let task = DownloadTask {
        id: TASK.into(),
        book_id: BOOK.into(),
        account_key: state.chapter_content_owner().unwrap(),
        chapters: vec![0, 1, 2],
        status: DownloadStatus::Pending,
        include_images: false,
        updated_at: 10,
        ..Default::default()
    };
    state.store.save_task(&task)?;
    state.downloads.clear();
    state.downloads.push_back(task);
    Ok((note, bookmark, position))
}

fn independent_event(runtime: &tokio::runtime::Runtime, fixture: &Fixture) -> Result<JobEvent> {
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let event = { fixture.app().events.try_recv() };
                match event {
                    Ok(event) => return Ok(event),
                    Err(mpsc::TryRecvError::Empty) => {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    Err(error) => {
                        return Err(anyhow::anyhow!("The independent mailbox closed: {error}"));
                    }
                }
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("The independent controller worker timed out"))?
    })
}

fn real_controller_recovery() -> Result<()> {
    let fixture = Fixture::new("Unrelated retained book.", Vec::new(), false)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let http = independent_server()?;
    let client = {
        let _entered = runtime.enter();
        http.client(worker_auth())
    };
    let (note, bookmark, position) = {
        let mut state = fixture.app();
        let (sender, events) = mpsc::channel();
        state.jobs = Jobs::new(runtime.handle().clone(), sender, Some(client))?;
        state.events = events;
        state.demo = false;
        state.force_offline = false;
        state.online = true;
        state.route = Route::Library;
        state.prefs.download_concurrency = 1;
        let personal = seed_independent_task(&mut state)?;
        state.start_next_download()?;
        assert_eq!(state.current_download.as_ref().unwrap().id, TASK);
        assert_eq!(state.jobs.active_download().as_deref(), Some(TASK));
        personal
    };
    let account = fixture.app().jobs.session();
    let epoch = fixture.app().jobs.account_epoch();
    let mut saw_rejection = false;
    let mut saw_independent_commit = false;
    let mut late_packet = None;
    for _ in 0..16 {
        let event = independent_event(&runtime, &fixture)?;
        match &event.result {
            JobResult::DownloadChapterFailed {
                task_id,
                chapter_id,
                chapter_index,
                ordinal,
                error,
                ..
            } => {
                assert!(!saw_rejection);
                assert_eq!(task_id, TASK);
                assert_eq!(chapter_id, &chapter_uid(0));
                assert_eq!((*chapter_index, *ordinal), (0, 0));
                assert!(matches!(error, source::SourceError::Api(code) if code == "-10102"));
                let before = Utc::now().timestamp();
                fixture.app().handle_job(event)?;
                let state = fixture.app();
                let task = state.store.task(TASK)?.unwrap();
                assert_eq!(task.status, DownloadStatus::Running);
                assert_eq!(task.completed_count, 0);
                assert_eq!(task.next_index, 0);
                let chapters = state.store.download_chapter_states(TASK)?;
                assert_eq!(chapters[0].status, DownloadChapterStatus::RetryWait);
                assert_eq!(chapters[0].retry_count, 1);
                assert!(
                    (before + 60..=Utc::now().timestamp() + 60)
                        .contains(&chapters[0].retry_not_before)
                );
                assert_eq!(chapters[1].status, DownloadChapterStatus::Pending);
                assert_eq!(state.current_download.as_ref().unwrap().id, TASK);
                assert_eq!(state.jobs.active_download().as_deref(), Some(TASK));
                assert_eq!(state.jobs.account_epoch(), epoch);
                assert_eq!(state.jobs.session(), account);
                assert_eq!(state.route, Route::Library);
                saw_rejection = true;
            }
            JobResult::Downloaded { chapter, .. } if chapter.index == 1 => {
                assert!(
                    saw_rejection,
                    "One worker slot must continue after its retry ACK"
                );
                assert!(!saw_independent_commit);
                assert_eq!(chapter.content_status, ContentStatus::Full);
                assert!(chapter.cached);
                fixture.app().handle_job(event)?;
                let state = fixture.app();
                let task = state.store.task(TASK)?.unwrap();
                assert_eq!(task.status, DownloadStatus::Running);
                assert_eq!(task.completed_count, 1);
                assert_eq!(task.next_index, 0);
                assert_eq!(state.store.chapter_body(BOOK, 1)?.as_deref(), Some(TEXT));
                let chapters = state.store.download_chapter_states(TASK)?;
                assert_eq!(chapters[0].status, DownloadChapterStatus::RetryWait);
                assert_eq!(chapters[1].status, DownloadChapterStatus::Complete);
                assert_eq!(state.current_download.as_ref().unwrap().completed_count, 1);
                assert_eq!(state.store.position(BOOK)?, Some(position.clone()));
                assert_eq!(state.store.bookmarks(BOOK)?, vec![bookmark.clone()]);
                assert_eq!(
                    state.store.annotation_by_id(BOOK, &note.id)?,
                    Some(note.clone())
                );
                saw_independent_commit = true;
            }
            JobResult::Downloaded { chapter, .. } if chapter.index == 2 => {
                assert!(saw_rejection && saw_independent_commit);
                assert!(event.credits.is_some());
                late_packet = Some(event);
                break;
            }
            JobResult::DownloadProgress {
                completed,
                total,
                waiting,
                blocked,
                ..
            } => {
                assert_eq!(*total, 3);
                assert!(*completed <= 1);
                assert!(*waiting <= 1);
                assert_eq!(*blocked, 0);
                fixture.app().handle_job(event)?;
            }
            result => panic!("A chapter rejection terminated independent work: {result:?}"),
        }
    }
    let late_packet = late_packet.ok_or_else(|| {
        anyhow::anyhow!("The bounded controller did not produce the later chapter packet")
    })?;
    {
        let mut state = fixture.app();
        assert_eq!(state.store.task(TASK)?.unwrap().completed_count, 1);
        state.reopen_running_download_dialog();
        state.shell_action("dialog-secondary", "")?;
        let cancelled = state.store.task(TASK)?.unwrap();
        assert_eq!(cancelled.status, DownloadStatus::Cancelled);
        assert_eq!(cancelled.completed_count, 1);
        assert_eq!(cancelled.next_index, 0);
        assert!(state.current_download.is_none());
        assert!(state.jobs.active_download().is_none());
        assert!(!state.handle_job(late_packet)?);
        assert_eq!(state.store.task(TASK)?.unwrap(), cancelled);
        assert_eq!(state.store.chapter_body(BOOK, 2)?, None);
        assert_eq!(state.store.chapter_body(BOOK, 1)?.as_deref(), Some(TEXT));
        assert_eq!(state.store.position(BOOK)?, Some(position));
        assert_eq!(state.store.bookmarks(BOOK)?, vec![bookmark]);
        assert_eq!(state.store.annotation_by_id(BOOK, &note.id)?, Some(note));
        assert_eq!(state.jobs.session(), account);
        assert_eq!(state.jobs.account_epoch(), epoch);
    }
    let requests = http.finish();
    for index in 0..3 {
        let clock = format!("independent-worker-clock-{index}");
        assert_eq!(
            requests
                .iter()
                .filter(|request| {
                    request.path() == "/web/book/chapter/e_0"
                        && request.json()["ps"].as_str() == Some(clock.as_str())
                })
                .count(),
            1,
            "The controller must not immediately replay chapter {index}"
        );
    }
    assert!(
        requests
            .iter()
            .all(|request| request.path() != "/web/login/renewal")
    );
    fixture.finish()?;
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn real_controller_rejection_releases_a_worker_and_cancel_fences_a_later_packet() -> Result<()> {
    isolated(
        "app::download_tests::recovery_tests::runtime_tests::real_controller_rejection_releases_a_worker_and_cancel_fences_a_later_packet",
        real_controller_recovery,
    )
}

fn live_retry_server() -> Result<AllocationHttpFixture> {
    let reader_paths = (0..4)
        .map(|index| {
            let url = source::protocol::reader_url_checked(BOOK, Some(&chapter_uid(index)))?;
            Ok((url::Url::parse(&url)?.path().to_owned(), index))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let first_attempts = std::sync::atomic::AtomicUsize::new(0);
    Ok(AllocationHttpFixture::new(move |request| {
        if let Some(&index) = reader_paths.get(request.path()) {
            return AllocationReply::text(reader_page(index));
        }
        let payload = request.json();
        let clock = payload["ps"].as_str().unwrap();
        match request.path() {
            "/web/book/chapter/e_0"
                if clock == "independent-worker-clock-0"
                    && first_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 =>
            {
                AllocationReply::text("Synthetic chapter rejection").with_status(400)
            }
            "/web/book/chapter/e_0" if clock == "independent-worker-clock-2" => {
                AllocationReply::text(r#"{"errCode":-10102}"#)
            }
            "/web/book/chapter/e_0" => AllocationReply::text(SHARD),
            "/web/book/chapter/e_1" | "/web/book/chapter/e_3" => AllocationReply::text(""),
            path => panic!("Unexpected active retry request: {path}"),
        }
    }))
}

fn controller_live_retry(terminal_race: bool) -> Result<()> {
    let fixture = Fixture::new("Unrelated retained book.", Vec::new(), false)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let http = live_retry_server()?;
    let client = {
        let _entered = runtime.enter();
        http.client(worker_auth())
    };
    let personal = {
        let mut state = fixture.app();
        let (sender, events) = mpsc::channel();
        state.jobs = Jobs::new(runtime.handle().clone(), sender, Some(client))?;
        state.events = events;
        state.demo = false;
        state.force_offline = false;
        state.online = true;
        state.route = Route::Library;
        state.prefs.download_concurrency = 1;
        let personal = seed_independent_task(&mut state)?;
        let mut book = state.store.book(BOOK)?.unwrap();
        book.total_chapters = 4;
        state.store.upsert_book(&book)?;
        state.store.upsert_chapter(&Chapter {
            id: chapter_uid(3),
            book_id: BOOK.into(),
            index: 3,
            title: "A peer awaiting its durable checkpoint".into(),
            ..Default::default()
        })?;
        let mut task = state.store.task(TASK)?.unwrap();
        task.chapters = if terminal_race {
            vec![0]
        } else {
            vec![0, 1, 2, 3]
        };
        state.store.save_task(&task)?;
        state.downloads = VecDeque::from([task]);
        state.start_next_download()?;
        personal
    };
    let request = fixture.app().download_request;
    let epoch = fixture.app().jobs.account_epoch();
    let account = fixture.app().jobs.session();
    let mut held = None;
    for _ in 0..24 {
        let packet = independent_event(&runtime, &fixture)?;
        if matches!(&packet.result, JobResult::Downloaded { chapter, .. } if chapter.index == 3)
            || terminal_race && matches!(&packet.result, JobResult::DownloadIncomplete { .. })
        {
            held = Some(packet);
            break;
        }
        match &packet.result {
            JobResult::Downloaded { chapter, .. } => assert_eq!(chapter.index, 1),
            JobResult::DownloadChapterFailed { chapter_index, .. } => {
                assert!(*chapter_index == 0 || !terminal_race && *chapter_index == 2);
            }
            JobResult::DownloadProgress { .. } => {}
            result => panic!("Unexpected pre-retry controller event: {result:?}"),
        }
        fixture.app().handle_job(packet)?;
    }
    let held = held.ok_or_else(|| anyhow::anyhow!("The retry boundary packet was not produced"))?;
    let waiting = {
        let mut state = fixture.app();
        let before = state.store.download_chapter_states(TASK)?;
        assert_eq!(before[0].status, DownloadChapterStatus::Blocked);
        if !terminal_race {
            assert_eq!(before[1].status, DownloadChapterStatus::Complete);
            assert_eq!(before[2].status, DownloadChapterStatus::RetryWait);
            assert!(before[2].retry_not_before > Utc::now().timestamp());
            assert_eq!(before[3].status, DownloadChapterStatus::Pending);
            assert!(held.credits.is_some());
        }
        state.open_download_notice()?;
        assert_eq!(state.dialog_action, "download-retry-active");
        assert_eq!(
            fixture
                .ui()
                .global::<ShellModel>()
                .get_dialog_primary()
                .as_str(),
            tr("download_retry_chapters")
        );
        assert_eq!(
            fixture
                .ui()
                .global::<ShellModel>()
                .get_dialog_secondary()
                .as_str(),
            tr("cancel_download")
        );
        state.shell_action("dialog-primary", "")?;
        let after = state.store.download_chapter_states(TASK)?;
        assert_eq!(after[0].status, DownloadChapterStatus::Pending);
        assert_eq!(after[0].retry_count, 0);
        assert_eq!(after[0].retry_not_before, 0);
        assert!(after[0].error.is_none());
        assert_eq!(state.current_download.as_ref().unwrap().id, TASK);
        assert_eq!(state.jobs.active_download().as_deref(), Some(TASK));
        if terminal_race {
            assert_ne!(state.download_request, request);
            assert!(!state.handle_job(held)?);
            None
        } else {
            assert_eq!(state.download_request, request);
            assert_eq!(after[1..], before[1..]);
            assert_eq!(state.current_download.as_ref().unwrap().completed_count, 1);
            state.handle_job(held)?;
            assert_eq!(state.current_download.as_ref().unwrap().completed_count, 2);
            Some(before[2].clone())
        }
    };
    let mut saw_retry_start = false;
    let mut saved_retry = false;
    for _ in 0..24 {
        let packet = independent_event(&runtime, &fixture)?;
        match &packet.result {
            JobResult::DownloadChapterStarted { chapter_index, .. } => {
                assert!(!terminal_race);
                assert_eq!(*chapter_index, 0);
                saw_retry_start = true;
            }
            JobResult::Downloaded { chapter, .. } => {
                assert_eq!(chapter.index, 0);
                assert!(terminal_race || saw_retry_start);
                saved_retry = true;
            }
            JobResult::DownloadFinished { .. } if terminal_race => {
                fixture.app().handle_job(packet)?;
                break;
            }
            JobResult::DownloadProgress { .. } => {}
            result => panic!("An active retry disturbed another chapter: {result:?}"),
        }
        fixture.app().handle_job(packet)?;
        if saved_retry && !terminal_race {
            break;
        }
    }
    assert!(saved_retry);
    {
        let mut state = fixture.app();
        let saved = state.store.task(TASK)?.unwrap();
        assert_eq!(saved.completed_count, if terminal_race { 1 } else { 3 });
        if let Some(waiting) = waiting {
            assert_eq!(state.store.download_chapter_states(TASK)?[2], waiting);
            assert_eq!(state.download_request, request);
            state.reopen_running_download_dialog();
            state.shell_action("dialog-secondary", "")?;
            assert_eq!(
                state.store.task(TASK)?.unwrap().status,
                DownloadStatus::Cancelled
            );
        } else {
            assert_eq!(saved.status, DownloadStatus::Complete);
        }
        assert!(state.current_download.is_none());
        assert_eq!(state.store.position(BOOK)?, Some(personal.2));
        assert_eq!(state.store.bookmarks(BOOK)?, vec![personal.1]);
        assert_eq!(
            state.store.annotation_by_id(BOOK, &personal.0.id)?,
            Some(personal.0.clone())
        );
        assert_eq!(state.jobs.account_epoch(), epoch);
        assert_eq!(state.jobs.session(), account);
    }
    let requests = http.finish();
    for index in 0..4 {
        let clock = format!("independent-worker-clock-{index}");
        let expected = if index == 0 {
            2
        } else {
            usize::from(!terminal_race)
        };
        assert_eq!(
            requests
                .iter()
                .filter(|request| {
                    request.path() == "/web/book/chapter/e_0"
                        && request.json()["ps"].as_str() == Some(clock.as_str())
                })
                .count(),
            expected,
            "A retry re-fetched a completed or waiting peer"
        );
    }
    fixture.finish()?;
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

fn live_retry_case() -> Result<()> {
    controller_live_retry(false)
}
fn terminal_retry_case() -> Result<()> {
    controller_live_retry(true)
}

#[test]
fn live_retry_keeps_a_peer_running_and_preserves_completed_and_waiting_chapters() -> Result<()> {
    isolated(
        "app::download_tests::recovery_tests::runtime_tests::live_retry_keeps_a_peer_running_and_preserves_completed_and_waiting_chapters",
        live_retry_case,
    )
}

#[test]
fn retry_after_terminal_delivery_preserves_new_intent_and_rejects_the_old_terminal() -> Result<()> {
    isolated(
        "app::download_tests::recovery_tests::runtime_tests::retry_after_terminal_delivery_preserves_new_intent_and_rejects_the_old_terminal",
        terminal_retry_case,
    )
}
