//! A small paid Full HTTP producer rejects style capacity without publishing a checkpoint.

use super::*;
use crate::article::{StyleTestLimits, style_budget_tests::ObservationScope as StyleScope};

#[test]
fn actual_full_jobs_style_quota_preserves_state_and_recovers_checkpoint_credit() {
    let style = "color:Blue; custom-value:abcdefghijklmnopqrstuvwxyz0123456789";
    let html = format!(
        "<html><body><p><b style='{style}'>A</p>{}</body></html>",
        "<p> </p>".repeat(8)
    );
    let fixture = Fixture::new(&html);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = calibrated_client(&runtime, &fixture, &html);
    let before_session = client.session();
    let (_directory, store, assets, mut task, note) = state();
    let before_chapter = store.chapter(BOOK, 0).unwrap().unwrap();
    let before_book = store.book(BOOK).unwrap().unwrap();
    let (sender, receiver) = mpsc::channel();
    let jobs = {
        let _context = runtime.enter();
        Jobs::new(runtime.handle().clone(), sender, Some(client.clone())).unwrap()
    };
    for (request, limits) in [
        (
            91,
            StyleTestLimits {
                work_bytes: 0,
                ..StyleTestLimits::default()
            },
        ),
        (
            92,
            StyleTestLimits {
                metadata_bytes: 0,
                ..StyleTestLimits::default()
            },
        ),
    ] {
        let scope = StyleScope::start(limits);
        let plan =
            DownloadPlan::from_headers(&task, &store.chapter_headers(BOOK).unwrap()).unwrap();
        assert!(jobs.download(request, task.clone(), plan).is_none());
        let terminal = next_terminal(&runtime, &receiver);
        assert!(jobs.accepts(&terminal));
        assert_eq!(terminal.request, request);
        assert!(matches!(&terminal.result,
            JobResult::DownloadFailed { task_id, error: SourceError::ResponseTooLarge }
            if task_id == &task.id));
        assert!(terminal.credits.is_some());
        assert_eq!(jobs.download_resource_capacity_for_tests(), (0, 1));
        assert_eq!(scope.stats().usage.style_scans, 0);
        assert_eq!(scope.stats().usage.lowered_chars, 0);
        assert_eq!(scope.stats().usage.normalized_bytes, 0);
        assert_eq!(store.task(&task.id).unwrap().unwrap(), task);
        assert_eq!(store.chapter(BOOK, 0).unwrap().unwrap(), before_chapter);
        assert_eq!(store.book(BOOK).unwrap().unwrap(), before_book);
        assert_eq!(store.annotations(BOOK).unwrap(), vec![note.clone()]);
        assert!(assets.chapter(&store, BOOK, UID).unwrap().is_none());
        assert_eq!(assets.usage().unwrap(), 0);
        assert_eq!(client.session(), before_session);
        assert_eq!(jobs.session().unwrap(), before_session);
        drop(terminal);
        assert_eq!(jobs.download_resource_capacity_for_tests(), (1, 1));
    }
    let scope = StyleScope::start(StyleTestLimits::default());
    let plan = DownloadPlan::from_headers(&task, &store.chapter_headers(BOOK).unwrap()).unwrap();
    assert!(jobs.download(93, task.clone(), plan).is_none());
    let packet = next_event(&runtime, &receiver);
    assert!(jobs.accepts(&packet));
    assert!(packet.credits.is_some());
    let JobEvent {
        result, credits, ..
    } = packet;
    match result {
        JobResult::Downloaded {
            chapter,
            next_index,
            html,
            metadata,
            images,
            checkpoint,
            ..
        } => {
            assert_eq!(chapter.content_status, ContentStatus::Full);
            assert_eq!(chapter.body.as_deref(), Some(BODY));
            assert_eq!(chapter.content_account_key, owner());
            assets
                .commit_prepared_chapter_metadata(
                    &store,
                    BOOK,
                    UID,
                    html.as_deref().unwrap(),
                    BODY,
                    metadata.as_ref().unwrap(),
                    images.as_ref(),
                )
                .unwrap();
            store
                .commit_download_checkpoint(&task, &chapter, next_index, 31)
                .unwrap();
            task.next_index = next_index;
            task.updated_at = 31;
            checkpoint.send(Ok(())).unwrap();
            drop((chapter, html, metadata, images));
        }
        other => panic!(
            "The style-recovered actual Full producer did not publish a fresh packet: {other:?}"
        ),
    }
    drop(credits);
    let terminal = next_terminal(&runtime, &receiver);
    assert!(
        matches!(&terminal.result, JobResult::DownloadFinished { task_id } if task_id == &task.id)
    );
    assert_eq!(store.task(&task.id).unwrap().unwrap().next_index, 1);
    assert_eq!(
        store.chapter(BOOK, 0).unwrap().unwrap().body.as_deref(),
        Some(BODY)
    );
    assert_eq!(store.annotations(BOOK).unwrap(), vec![note]);
    assert_eq!(scope.stats().usage.style_scans, 1);
    assert_eq!(scope.stats().usage.input_bytes, style.len());
    assert!(scope.stats().usage.memo_hits > 0);
    assert_eq!(client.session(), before_session);
    assert_eq!(jobs.session().unwrap(), before_session);
    drop(terminal);
    assert_eq!(jobs.download_resource_capacity_for_tests(), (1, 1));
    request_contract(&fixture.finish(), 3);
}
