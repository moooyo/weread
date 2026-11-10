//! Actual App dispatch and cache checkpoints retain content, access, and image fences.

use super::*;
use sha2::{Digest, Sha256};

const BOOK: &str = "fixture-book";
const TEXT: &str = "A stable paragraph with enough text for retained personal records.";

fn seed_complete(
    state: &mut App,
    index: u32,
    with_image: bool,
) -> Result<Option<std::path::PathBuf>> {
    let owner = state.chapter_content_owner().unwrap();
    let mut chapter = state.store.chapter(BOOK, index)?.unwrap();
    chapter.body = Some(TEXT.into());
    chapter.cached = true;
    chapter.content_status = ContentStatus::Full;
    chapter.content_account_key = owner;
    state.store.upsert_chapter(&chapter)?;
    let html = if with_image {
        format!("<p>{TEXT}</p><img src='images/cache.png'>")
    } else {
        format!("<p>{TEXT}</p>")
    };
    let downloaded = if with_image {
        let bytes = png_bytes()?;
        let id = hex::encode(Sha256::digest(&bytes));
        Some(DownloadedChapterImages {
            source_to_asset: BTreeMap::from([("images/cache.png".into(), id.clone())]),
            assets: vec![ImageAsset {
                id,
                media_type: "image/png".into(),
                extension: "png".into(),
                source_names: vec!["images/cache.png".into()],
                bytes,
            }],
        })
    } else {
        None
    };
    let manifest = state.book_assets.commit_chapter(
        &state.store,
        BOOK,
        &chapter.id,
        &html,
        TEXT,
        downloaded.as_ref(),
    )?;
    assert!(manifest.complete);
    manifest
        .images
        .first()
        .map(|image| state.book_assets.image_path(image))
        .transpose()
        .map(Option::flatten)
}

fn cached_packet(
    state: &App,
    chapter_id: &str,
    index: u32,
) -> (JobEvent, tokio::sync::oneshot::Receiver<source::Result<()>>) {
    let task = state.current_download.as_ref().unwrap();
    let (checkpoint, reply) = tokio::sync::oneshot::channel();
    (
        event(
            state,
            JobResult::CachedDownloaded {
                task_id: task.id.clone(),
                chapter_id: chapter_id.into(),
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

fn run_complete_checkpoints(_: &AppWindow, application: &Rc<RefCell<App>>, _: &Path) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_remote(&mut state, 2)?;
    seed_complete(&mut state, 0, true)?;
    seed_complete(&mut state, 1, false)?;
    let records = fixture_personal_records(&state)?;
    let task = begin_task(&mut state, vec![0, 1], true)?;
    let count = state.store.book(BOOK)?.unwrap().cached_chapters;
    assert!(
        state
            .store
            .commit_cached_download_checkpoint(
                &task,
                "fixture-chapter-0",
                0,
                1,
                &task.account_key,
                101,
            )
            .is_err(),
        "The text-only Store entry must keep rejecting image tasks"
    );
    for index in 0..2 {
        let (packet, mut reply) = cached_packet(&state, &format!("fixture-chapter-{index}"), index);
        state.handle_job(packet)?;
        assert!(matches!(reply.try_recv(), Ok(Ok(()))));
        assert_eq!(
            state.current_download.as_ref().unwrap().next_index,
            index as usize + 1
        );
        assert_eq!(
            state.store.task(&task.id)?.unwrap().next_index,
            index as usize + 1
        );
        assert_eq!(
            state.store.chapter_body(BOOK, index)?.as_deref(),
            Some(TEXT)
        );
    }
    assert_eq!(state.store.book(BOOK)?.unwrap().cached_chapters, count);
    assert_eq!(state.store.annotations(BOOK)?, vec![records.0]);
    assert_eq!(state.store.bookmarks(BOOK)?, vec![records.1]);
    assert_eq!(state.store.position(BOOK)?, Some(records.2));
    Ok(())
}

#[test]
fn complete_image_and_empty_image_checkpoints_preserve_cache_and_personal_records() -> Result<()> {
    on_ui_thread(run_complete_checkpoints)
}

fn run_cache_fences(_: &AppWindow, application: &Rc<RefCell<App>>, _: &Path) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_remote(&mut state, 1)?;
    for mutation in 0..8 {
        let image = seed_complete(&mut state, 0, true)?.unwrap();
        let task = begin_task(&mut state, vec![0], true)?;
        // All ordinary content mutations use the same Store path as account or
        // catalog replacement. Byte corruption is applied to the actual PNG.
        match mutation {
            0 => {
                std::fs::write(&image, b"broken cached image")?;
            }
            1 => {
                std::fs::remove_file(&image)?;
            }
            2 => {
                let mut chapter = state.store.chapter(BOOK, 0)?.unwrap();
                chapter.content_account_key = "another-account".into();
                state.store.upsert_chapter(&chapter)?;
            }
            3 => {
                let mut chapter = state.store.chapter(BOOK, 0)?.unwrap();
                chapter.content_status = ContentStatus::Preview;
                chapter.cached = false;
                state.store.upsert_chapter(&chapter)?;
            }
            4 => {
                let mut chapter = state.store.chapter(BOOK, 0)?.unwrap();
                chapter.content_status = ContentStatus::Unknown;
                chapter.cached = false;
                state.store.upsert_chapter(&chapter)?;
            }
            5 => {
                let mut chapter = state.store.chapter(BOOK, 0)?.unwrap();
                chapter.body =
                    Some("A different cached body that has no matching image anchors.".into());
                state.store.upsert_chapter(&chapter)?;
            }
            6 => {
                let mut chapter = state.store.chapter(BOOK, 0)?.unwrap();
                chapter.id = "replacement-uid".into();
                state.store.upsert_chapter(&chapter)?;
            }
            7 => {}
            _ => unreachable!(),
        }
        let chapter_id = if mutation == 7 {
            "wrong-uid"
        } else {
            "fixture-chapter-0"
        };
        let (packet, mut reply) = cached_packet(&state, chapter_id, 0);
        state.handle_job(packet)?;
        assert!(matches!(
            reply.try_recv(),
            Ok(Err(source::SourceError::ContentChanged))
        ));
        assert_eq!(state.current_download.as_ref(), Some(&task));
        assert_eq!(state.store.task(&task.id)?, Some(task));
        // Restore the catalog identity before the next independently verified
        // fixture publication, including after a UID replacement.
        let mut chapter = state.store.chapter(BOOK, 0)?.unwrap();
        chapter.id = "fixture-chapter-0".into();
        state.store.upsert_chapter(&chapter)?;
    }
    Ok(())
}

#[test]
fn cached_image_events_recheck_resources_account_status_and_uid_before_advancing() -> Result<()> {
    on_ui_thread(run_cache_fences)
}

fn run_queued_and_resumed_dispatch(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()?;
    let mut state = application.borrow_mut();
    seed_remote(&mut state, 2)?;
    let session = state.jobs.session().unwrap();
    let owner = state.chapter_content_owner().unwrap();
    let client = source::client_with_test_web_origin(session, "http://127.0.0.1:1")?;
    let (sender, receiver) = mpsc::channel();
    state.jobs = Jobs::new(runtime.handle().clone(), sender, Some(client))?;
    state.events = receiver;
    state.prefs.download_concurrency = 8;
    state.force_offline = false;
    state.online = true;
    for resumed in [false, true] {
        let task = DownloadTask {
            id: format!("cache-dispatch-{resumed}"),
            book_id: BOOK.into(),
            account_key: owner.clone(),
            chapters: vec![0, 1],
            status: if resumed {
                DownloadStatus::Paused
            } else {
                DownloadStatus::Pending
            },
            next_index: usize::from(resumed) * 2,
            completed_count: usize::from(resumed) * 2,
            include_images: true,
            updated_at: 100,
            ..Default::default()
        };
        state.store.save_task(&task)?;
        state.downloads.push_back(task.clone());
        // Content becomes complete after the original intent was persisted.
        seed_complete(&mut state, 0, true)?;
        seed_complete(&mut state, 1, false)?;
        state.start_next_download()?;
        assert_eq!(state.current_download.as_ref().unwrap().completed_count, 2);
        assert_eq!(state.store.task(&task.id)?.unwrap().completed_count, 2);
        let mut cached = Vec::new();
        loop {
            let packet = state.events.recv_timeout(Duration::from_secs(3))?;
            let finished = matches!(&packet.result, JobResult::DownloadFinished { .. });
            match &packet.result {
                JobResult::CachedDownloaded { chapter_index, .. } => cached.push(*chapter_index),
                JobResult::DownloadProgress { .. } | JobResult::DownloadFinished { .. } => {}
                other => panic!("A freshly complete task used a network path: {other:?}"),
            }
            state.handle_job(packet)?;
            if finished {
                break;
            }
        }
        assert!(
            cached.is_empty(),
            "Fresh batch verification already completed both chapters"
        );
        let persisted = state.store.task(&task.id)?.unwrap();
        assert_eq!(persisted.status, DownloadStatus::Complete);
        assert_eq!(persisted.next_index, 2);
        assert!(state.current_download.is_none());
    }
    drop(state);
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn queued_and_resumed_image_tasks_keep_verified_completion_without_replaying_checkpoints()
-> Result<()> {
    on_ui_thread(run_queued_and_resumed_dispatch)
}
