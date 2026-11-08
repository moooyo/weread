//! Admission leases exercised through real Jobs producers and queued results.

use super::*;
use std::sync::{
    atomic::AtomicUsize,
    mpsc::{self, Receiver, TryRecvError},
};
use tokio::runtime::{Builder, Runtime};

const PAYLOAD_BYTES: usize = 256 * 1024;

fn fixture() -> (Runtime, Jobs, Receiver<JobEvent>) {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let (sender, receiver) = mpsc::channel();
    let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
    (runtime, jobs, receiver)
}

async fn eventually(mut condition: impl FnMut() -> bool, description: &str) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !condition() {
        assert!(Instant::now() < deadline, "Timed out: {description}");
        tokio::task::yield_now().await;
    }
}

async fn next_event(receiver: &Receiver<JobEvent>) -> JobEvent {
    let mut event = None;
    eventually(
        || match receiver.try_recv() {
            Ok(value) => {
                event = Some(value);
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => panic!("The genuine producer lost its receiver"),
        },
        "queued result delivery",
    )
    .await;
    event.unwrap()
}

fn metadata(jobs: &Jobs, request: u64, produced: &Arc<AtomicUsize>) -> Option<JobEvent> {
    let produced = produced.clone();
    jobs.run(request, JobResult::Search, move |_client| async move {
        let book = RemoteBook {
            id: format!("queued-book-{request}"),
            intro: "m".repeat(PAYLOAD_BYTES),
            ..RemoteBook::default()
        };
        produced.fetch_add(1, Ordering::SeqCst);
        Ok(vec![book])
    })
}

fn metadata_bytes(event: &JobEvent) -> usize {
    let JobResult::Search(Ok(books)) = &event.result else {
        panic!("Expected a genuine queued metadata payload");
    };
    assert!(
        event.credits.is_some(),
        "The actual producer omitted its lease"
    );
    books.iter().map(|book| book.intro.len()).sum()
}

fn image(
    jobs: &Jobs,
    request: u64,
    document: &str,
    produced: &Arc<AtomicUsize>,
) -> Option<JobEvent> {
    let article_id = document.to_owned();
    let produced = produced.clone();
    jobs.run_public_image(
        request,
        move |result| JobResult::ArticleImage {
            article_id,
            image_id: format!("image-{request}"),
            result,
        },
        move |_client| async move {
            let payload = vec![b'i'; PAYLOAD_BYTES];
            produced.fetch_add(1, Ordering::SeqCst);
            Ok(payload)
        },
    )
}

fn login_flow(jobs: &Jobs) -> LoginFlow {
    jobs.login.lock().unwrap().clone()
}

struct DropProbe(Arc<AtomicUsize>);
impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn assert_sync_search_busy(event: Option<JobEvent>, request: u64) {
    let event = event.expect("Saturation must return a synchronous result");
    assert_eq!(event.request, request);
    assert!(event.credits.is_none());
    assert!(matches!(
        event.result,
        JobResult::Search(Err(SourceError::Busy))
    ));
}

#[test]
fn queued_metadata_keeps_eight_credits_across_account_resets_and_sync_rejections() {
    let (runtime, jobs, receiver) = fixture();
    runtime.block_on(async {
        let produced = Arc::new(AtomicUsize::new(0));
        for request in 0..MAX_METADATA_REQUESTS as u64 {
            assert!(metadata(&jobs, request, &produced).is_none());
        }
        // No receiver operation has occurred: all eight large results remain queued.
        eventually(
            || produced.load(Ordering::SeqCst) == MAX_METADATA_REQUESTS,
            "metadata producers",
        )
        .await;
        assert_eq!(jobs.metadata_capacity(), 0);
        jobs.cancel_all();
        jobs.replace_session(AuthSession {
            api_key: "queued-fixture-replacement".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(jobs.metadata_capacity(), 0);
        for request in 100..1_124 {
            assert_sync_search_busy(
                jobs.search(request, format!("different-document-{request}")),
                request,
            );
        }
        assert_eq!(produced.load(Ordering::SeqCst), MAX_METADATA_REQUESTS);
        let events = (0..MAX_METADATA_REQUESTS)
            .map(|_| receiver.try_recv().unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(
            events.iter().map(metadata_bytes).sum::<usize>(),
            MAX_METADATA_REQUESTS * PAYLOAD_BYTES
        );
        assert!(events.iter().all(|event| !jobs.accepts(event)));
        // Dequeue alone transfers ownership; it must not create new admission.
        assert_eq!(jobs.metadata_capacity(), 0);
        drop(events);
        assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS);
        assert!(metadata(&jobs, 2_000, &produced).is_none());
        let recovered = next_event(&receiver).await;
        assert!(jobs.accepts(&recovered));
        assert_eq!(metadata_bytes(&recovered), PAYLOAD_BYTES);
        assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS - 1);
        drop(recovered);
        assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS);
    });
}

fn fail_owned_metadata(event: JobEvent, budget: &Semaphore) -> source::Result<()> {
    assert_eq!(metadata_bytes(&event), PAYLOAD_BYTES);
    assert_eq!(budget.available_permits(), 0);
    // An early handler error drops the actual event and its lease together.
    Err(SourceError::CheckpointFailed)
}

#[test]
fn handler_failure_and_events_outliving_jobs_release_metadata_without_a_credit_cycle() {
    let (runtime, jobs, receiver) = fixture();
    let budget = jobs.metadata_requests.clone();
    let weak = Arc::downgrade(&budget);
    let held = runtime.block_on(async {
        let produced = Arc::new(AtomicUsize::new(0));
        for request in 0..MAX_METADATA_REQUESTS as u64 {
            assert!(metadata(&jobs, request, &produced).is_none());
        }
        eventually(
            || produced.load(Ordering::SeqCst) == MAX_METADATA_REQUESTS,
            "queued handler inputs",
        )
        .await;
        let first = receiver.try_recv().unwrap();
        assert_eq!(budget.available_permits(), 0);
        assert!(matches!(
            fail_owned_metadata(first, &budget),
            Err(SourceError::CheckpointFailed)
        ));
        assert_eq!(budget.available_permits(), 1);
        (1..MAX_METADATA_REQUESTS)
            .map(|_| receiver.try_recv().unwrap())
            .collect::<Vec<_>>()
    });
    drop(jobs);
    assert_eq!(
        budget.available_permits(),
        1,
        "Owner cancellation returned queued leases prematurely"
    );
    assert_eq!(
        held.iter().map(metadata_bytes).sum::<usize>(),
        (MAX_METADATA_REQUESTS - 1) * PAYLOAD_BYTES
    );
    drop(held);
    assert_eq!(budget.available_permits(), MAX_METADATA_REQUESTS);
    drop(budget);
    assert!(
        weak.upgrade().is_none(),
        "A released event retained a semaphore cycle"
    );
}

#[test]
fn receiver_close_and_inflight_cancellation_release_real_metadata_leases() {
    let (runtime, jobs, receiver) = fixture();
    runtime.block_on(async {
        let produced = Arc::new(AtomicUsize::new(0));
        for request in 0..MAX_METADATA_REQUESTS as u64 {
            assert!(metadata(&jobs, request, &produced).is_none());
        }
        eventually(
            || produced.load(Ordering::SeqCst) == MAX_METADATA_REQUESTS,
            "unconsumed receiver inputs",
        )
        .await;
        assert_eq!(jobs.metadata_capacity(), 0);
        drop(receiver);
        assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS);
        let resource = Arc::new(());
        let weak = Arc::downgrade(&resource);
        let destroyed = Arc::new(AtomicUsize::new(0));
        let probe = DropProbe(destroyed.clone());
        let (started, ready) = tokio::sync::oneshot::channel();
        assert!(
            jobs.run(90, JobResult::Search, move |_client| async move {
                let _resource = resource;
                let _probe = probe;
                started.send(()).unwrap();
                std::future::pending::<source::Result<Vec<RemoteBook>>>().await
            })
            .is_none()
        );
        ready.await.unwrap();
        assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS - 1);
        jobs.cancel_all();
        eventually(
            || {
                weak.upgrade().is_none()
                    && destroyed.load(Ordering::SeqCst) == 1
                    && jobs.metadata_capacity() == MAX_METADATA_REQUESTS
            },
            "cancelled metadata destruction",
        )
        .await;
        // Delivery into the closed receiver must also dispose a freshly produced payload.
        assert!(metadata(&jobs, 91, &produced).is_none());
        eventually(
            || {
                produced.load(Ordering::SeqCst) == MAX_METADATA_REQUESTS + 1
                    && jobs.metadata_capacity() == MAX_METADATA_REQUESTS
            },
            "closed-receiver send cleanup",
        )
        .await;
    });
}

#[test]
fn queued_public_images_keep_two_credits_across_document_and_account_replacement() {
    let (runtime, jobs, receiver) = fixture();
    runtime.block_on(async {
        let produced = Arc::new(AtomicUsize::new(0));
        assert!(image(&jobs, 1, "old-document", &produced).is_none());
        assert!(image(&jobs, 2, "old-document", &produced).is_none());
        eventually(|| produced.load(Ordering::SeqCst) == MAX_COVER_REQUESTS, "queued image payloads").await;
        assert_eq!(jobs.cover_capacity(), 0);
        assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS - MAX_COVER_REQUESTS);
        jobs.cancel_all();
        jobs.replace_session(AuthSession { api_key: "image-fixture-account".into(), ..Default::default() }).unwrap();
        for request in 100..1_124 {
            let event = if request % 2 == 0 {
                jobs.cover(request, format!("article-image:opaque-book-{request}"), "file:///unused.png".into())
            } else {
                jobs.article_image(request, format!("replacement-document-{request}"), "replacement-image".into(), "file:///unused.png".into())
            }.expect("Queued images must reject new work synchronously");
            assert!(event.credits.is_none());
            assert!(matches!(event.result, JobResult::Cover { result: Err(SourceError::Busy), .. }
                | JobResult::ArticleImage { result: Err(SourceError::Busy), .. }));
        }
        let old = receiver.try_recv().unwrap();
        assert!(old.credits.is_some());
        assert!(matches!(&old.result, JobResult::ArticleImage { result: Ok(bytes), .. } if bytes.len() == PAYLOAD_BYTES));
        assert!(!jobs.accepts(&old));
        assert_eq!(jobs.cover_capacity(), 0);
        drop(old);
        assert_eq!(jobs.cover_capacity(), 1);
        assert!(image(&jobs, 3, "replacement-document", &produced).is_none());
        eventually(|| produced.load(Ordering::SeqCst) == 3, "reused image credit").await;
        assert_eq!(jobs.cover_capacity(), 0);
        let old = receiver.try_recv().unwrap();
        assert!(!jobs.accepts(&old));
        drop(old);
        let fresh = receiver.try_recv().unwrap();
        assert!(jobs.accepts(&fresh));
        assert!(matches!(&fresh.result, JobResult::ArticleImage { article_id, result: Ok(bytes), .. }
            if article_id == "replacement-document" && bytes.len() == PAYLOAD_BYTES));
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        drop(fresh);
        assert_eq!(jobs.cover_capacity(), MAX_COVER_REQUESTS);
        assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS);
    });
}

#[test]
fn queued_login_errors_survive_regeneration_and_inflight_cancel_releases_the_new_lease() {
    let (runtime, jobs, receiver) = fixture();
    runtime.block_on(async {
        let produced = Arc::new(AtomicUsize::new(0));
        let completed = produced.clone();
        assert!(
            jobs.run_login(
                1,
                login_flow(&jobs),
                JobResult::LoginPoll,
                move |_flow| async move {
                    completed.fetch_add(1, Ordering::SeqCst);
                    Err::<QrLoginState, _>(SourceError::LoginExpired)
                }
            )
            .is_none()
        );
        eventually(
            || produced.load(Ordering::SeqCst) == 1,
            "queued login error",
        )
        .await;
        assert_eq!(jobs.login_requests.available_permits(), 0);
        jobs.cancel_login();
        for request in 100..228 {
            let busy = jobs
                .login_begin(request)
                .expect("Regeneration cannot discard a queued login buffer");
            assert!(busy.credits.is_none());
            assert!(matches!(
                busy.result,
                JobResult::LoginBegin(Err(SourceError::Busy))
            ));
        }
        let old = receiver.try_recv().unwrap();
        assert!(old.credits.is_some());
        assert!(matches!(
            old.result,
            JobResult::LoginPoll(Err(SourceError::LoginExpired))
        ));
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(jobs.login_requests.available_permits(), 0);
        drop(old);
        assert_eq!(jobs.login_requests.available_permits(), 1);
        let resource = Arc::new(());
        let weak = Arc::downgrade(&resource);
        let destroyed = Arc::new(AtomicUsize::new(0));
        let probe = DropProbe(destroyed.clone());
        let (started, ready) = tokio::sync::oneshot::channel();
        assert!(
            jobs.run_login(
                300,
                login_flow(&jobs),
                JobResult::LoginPoll,
                move |_flow| async move {
                    let _resource = resource;
                    let _probe = probe;
                    started.send(()).unwrap();
                    std::future::pending::<source::Result<QrLoginState>>().await
                }
            )
            .is_none()
        );
        ready.await.unwrap();
        jobs.cancel_login();
        eventually(
            || {
                weak.upgrade().is_none()
                    && destroyed.load(Ordering::SeqCst) == 1
                    && jobs.login_requests.available_permits() == 1
            },
            "cancelled login destruction",
        )
        .await;
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    });
}

#[test]
fn queued_login_success_keeps_its_buffer_after_dequeue_receiver_close_and_account_reset() {
    let (runtime, jobs, receiver) = fixture();
    runtime.block_on(async {
        let produced = Arc::new(AtomicUsize::new(0));
        let completed = produced.clone();
        assert!(jobs.run_login(1, login_flow(&jobs), JobResult::LoginBegin, move |_flow| async move {
            let challenge = QrLoginChallenge { uid: "queued-login".into(), confirm_url: "q".repeat(PAYLOAD_BYTES), expires_at: 9_999 };
            completed.fetch_add(1, Ordering::SeqCst);
            Ok(challenge)
        }).is_none());
        eventually(|| produced.load(Ordering::SeqCst) == 1, "queued login success").await;
        jobs.cancel_all();
        jobs.clear_session();
        let busy = jobs.login_poll(2, "new-login".into(), None).unwrap();
        assert!(matches!(busy.result, JobResult::LoginPoll(Err(SourceError::Busy))));
        assert!(busy.credits.is_none());
        let event = receiver.try_recv().unwrap();
        assert!(matches!(&event.result, JobResult::LoginBegin(Ok(challenge)) if challenge.confirm_url.len() == PAYLOAD_BYTES));
        assert!(!jobs.accepts(&event));
        drop(receiver);
        assert_eq!(jobs.login_requests.available_permits(), 0, "The dequeued buffer still owns its lease");
        drop(event);
        assert_eq!(jobs.login_requests.available_permits(), 1);
        let completed = produced.clone();
        assert!(jobs.run_login(3, login_flow(&jobs), JobResult::LoginPoll, move |_flow| async move {
            completed.fetch_add(1, Ordering::SeqCst);
            Ok(QrLoginState::Authenticated(AuthSession::default()))
        }).is_none());
        eventually(|| produced.load(Ordering::SeqCst) == 2
            && jobs.login_requests.available_permits() == 1, "closed-receiver login delivery").await;
        assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS);
        assert_eq!(jobs.cover_capacity(), MAX_COVER_REQUESTS);
    });
}

fn cached_task(id: &str, count: u32) -> (domain::DownloadTask, DownloadPlan) {
    let task = domain::DownloadTask {
        id: id.to_owned(),
        book_id: "queued-credit-book".into(),
        account_key: "queued-credit-account".into(),
        chapters: (0..count).collect(),
        include_images: false,
        ..domain::DownloadTask::default()
    };
    let chapters = (0..count)
        .map(|index| domain::Chapter {
            id: format!("cached-{index}"),
            book_id: task.book_id.clone(),
            index,
            title: format!("Cached chapter {index}"),
            cached: true,
            content_status: domain::ContentStatus::Full,
            content_account_key: task.account_key.clone(),
            ..domain::Chapter::default()
        })
        .collect::<Vec<_>>();
    let plan = DownloadPlan::from_headers(&task, &chapters).unwrap();
    (task, plan)
}

fn packet_parts(
    event: JobEvent,
) -> (
    String,
    String,
    usize,
    tokio::sync::oneshot::Sender<source::Result<()>>,
    Option<JobCredits>,
) {
    let JobEvent {
        result, credits, ..
    } = event;
    let JobResult::CachedDownloaded {
        task_id,
        chapter_id,
        chapter_index,
        next_index,
        checkpoint,
    } = result
    else {
        panic!("Expected a genuine cached chapter packet");
    };
    assert!(credits.is_some());
    assert_eq!(chapter_id, format!("cached-{chapter_index}"));
    assert_eq!(chapter_index as usize + 1, next_index);
    (task_id, chapter_id, next_index, checkpoint, credits)
}

fn acknowledge_and_drop(
    event: JobEvent,
    expected_task: &str,
    expected_index: usize,
    outcome: source::Result<()>,
) {
    let (task_id, chapter, next_index, checkpoint, credits) = packet_parts(event);
    assert_eq!(task_id, expected_task);
    assert_eq!(next_index, expected_index);
    checkpoint.send(outcome).unwrap();
    // The processing contract drops the payload before releasing its queue lease.
    drop((task_id, chapter));
    drop(credits);
}

#[test]
fn cached_packets_keep_their_credit_after_ack_and_queued_progress_cannot_block_data() {
    let (runtime, jobs, receiver) = fixture();
    runtime.block_on(async {
        let (task, chapters) = cached_task("first-cached-task", 2);
        assert!(jobs.download(1, task, chapters).is_none());
        let first = next_event(&receiver).await;
        let (task_id, chapter, next_index, checkpoint, credits) = packet_parts(first);
        assert_eq!(next_index, 1);
        assert_eq!(jobs.download_packets.available_permits(), 0);
        assert_eq!(jobs.download_preparation.available_permits(), 1);
        checkpoint.send(Ok(())).unwrap();
        let progress = next_event(&receiver).await;
        assert!(matches!(&progress.result, JobResult::DownloadProgress { completed: 1, .. }));
        assert!(progress.credits.is_some());
        assert_eq!(jobs.download_progress.available_permits(), 0);
        assert_eq!(jobs.download_packets.available_permits(), 0, "ACK returned a still-owned cached packet's credit");
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        drop((task_id, chapter));
        drop(credits);
        acknowledge_and_drop(next_event(&receiver).await, "first-cached-task", 2, Ok(()));
        let finished = next_event(&receiver).await;
        assert!(matches!(&finished.result, JobResult::DownloadFinished { task_id } if task_id == "first-cached-task"));
        assert!(finished.credits.is_some());
        assert_eq!(jobs.download_packets.available_permits(), 0);
        drop(finished);

        // A later task produces and completes data while the old visual progress
        // event remains owned. Progress has a separate one-slot allowance.
        let (task, chapters) = cached_task("later-cached-task", 1);
        assert!(jobs.download(2, task, chapters).is_none());
        acknowledge_and_drop(next_event(&receiver).await, "later-cached-task", 1, Ok(()));
        let finished = next_event(&receiver).await;
        assert!(matches!(&finished.result, JobResult::DownloadFinished { task_id } if task_id == "later-cached-task"));
        assert_eq!(jobs.download_progress.available_permits(), 0);
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        drop(finished);
        drop(progress);
        assert_eq!(jobs.download_progress.available_permits(), 1);
        assert_eq!(jobs.download_packets.available_permits(), 1);
        assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS);
    });
}

#[test]
fn cancelled_cached_packets_nacks_and_receiver_close_preserve_data_backpressure() {
    let (runtime, jobs, receiver) = fixture();
    runtime.block_on(async {
        let (task, chapters) = cached_task("old-cached-task", 2);
        assert!(jobs.download(1, task, chapters).is_none());
        let old = next_event(&receiver).await;
        assert!(old.credits.is_some());
        let cancelled = jobs.cancel_download("old-cached-task").unwrap();
        assert!(cancelled.credits.is_none());
        assert!(matches!(cancelled.result, JobResult::Cancelled { .. }));
        jobs.replace_session(AuthSession { api_key: "cached-fixture-replacement".into(), ..Default::default() }).unwrap();
        assert!(!jobs.accepts(&old));
        let (task, chapters) = cached_task("replacement-cached-task", 2);
        assert!(jobs.download(2, task, chapters).is_none());
        assert_eq!(jobs.download_packets.available_permits(), 0, "Task replacement discarded an old queued buffer's credit");
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        drop(old);
        let replacement = next_event(&receiver).await;
        assert!(jobs.accepts(&replacement));
        acknowledge_and_drop(replacement, "replacement-cached-task", 1, Err(SourceError::CheckpointFailed));
        let failure = next_event(&receiver).await;
        assert!(matches!(&failure.result, JobResult::DownloadFailed { task_id, error: SourceError::CheckpointFailed }
            if task_id == "replacement-cached-task"));
        assert!(failure.credits.is_some());
        // A rejected ACK must emit a terminal failure, never chapter two.
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        drop(failure);
        assert_eq!(jobs.download_packets.available_permits(), 1);
        let (task, chapters) = cached_task("closed-receiver-task", 2);
        assert!(jobs.download(3, task, chapters).is_none());
        // The cached producer performs no await between acquiring credit and
        // queueing its packet. Observe it without receiving or acknowledging it.
        eventually(|| jobs.download_packets.available_permits() == 0, "cached packet queued before receiver close").await;
        drop(receiver);
        eventually(|| jobs.active_download().is_none() && jobs.download_packets.available_permits() == 1,
            "closed receiver rejected the checkpoint and released terminal credit").await;
        assert_eq!(jobs.download_progress.available_permits(), 1);
        assert_eq!(jobs.download_preparation.available_permits(), 1);
    });
}
