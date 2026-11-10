//! Admission regressions exercise ownership across waits and cancellation.

use super::*;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::sync::{Condvar, Mutex};
use std::task::Poll;
use std::time::Duration;
use tokio::sync::{Barrier, mpsc, oneshot};

const WAIT: Duration = Duration::from_secs(10);
const MIB: usize = 1024 * 1024;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

async fn bounded<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(WAIT, future)
        .await
        .expect("Admission did not finish before its deadline")
}

async fn assert_pending<F: Future>(mut future: Pin<&mut F>) {
    poll_fn(|cx| match future.as_mut().poll(cx) {
        Poll::Pending => Poll::Ready(()),
        Poll::Ready(_) => panic!("An occupied admission quota was bypassed"),
    })
    .await;
}

fn assert_idle(admission: &ReaderAdmission) {
    let snapshot = admission.snapshot();
    assert_eq!(snapshot.available_bytes, snapshot.budget_bytes);
    assert_eq!(snapshot.reserved_bytes, 0);
    assert_eq!(snapshot.response_allowance_bytes, 0);
    assert_eq!(snapshot.pages_active, 0);
    assert_eq!(snapshot.parsers_active, 0);
    assert_eq!(snapshot.parsers_waiting, 0);
    assert_eq!(admission.parsers.available_permits(), READER_PARSER_WORKERS);
    assert!(snapshot.peak_reserved_bytes <= snapshot.budget_bytes);
    assert!(snapshot.peak_parsers_active <= READER_PARSER_WORKERS);
}

#[derive(Default)]
struct BlockingGate {
    released: Mutex<bool>,
    changed: Condvar,
}

impl BlockingGate {
    fn wait(&self) {
        let deadline = Instant::now() + WAIT + WAIT;
        let mut released = self.released.lock().unwrap();
        while !*released {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "The parser gate was not released");
            released = self.changed.wait_timeout(released, remaining).unwrap().0;
        }
    }

    fn release(&self) {
        *self
            .released
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.changed.notify_all();
    }
}

struct ReleaseOnDrop(Arc<BlockingGate>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        // Runtime shutdown waits for blocking workers even after an assertion.
        self.0.release();
    }
}

#[test]
fn every_response_tier_has_an_exact_shared_quota() {
    runtime().block_on(async {
        for (tier, response_bytes, simultaneous) in [
            (ReaderTier(0), 512 * 1024, 32),
            (ReaderTier(1), 2 * MIB, 8),
            (ReaderTier(2), 4 * MIB, 4),
            (ReaderTier(3), 8 * MIB, 2),
            (ReaderTier(4), 16 * MIB, 1),
            (ReaderTier(5), 24 * MIB, 1),
        ] {
            let admission = Arc::new(ReaderAdmission::new());
            let mut owners = Vec::new();
            for _ in 0..simultaneous {
                owners.push(bounded(admission.acquire(tier)).await.unwrap());
            }
            let snapshot = admission.snapshot();
            assert_eq!(snapshot.budget_bytes, 16 * MIB);
            assert_eq!(snapshot.available_bytes, 0);
            assert_eq!(snapshot.reserved_bytes, 16 * MIB);
            assert_eq!(snapshot.pages_active, simultaneous);
            assert_eq!(
                snapshot.response_allowance_bytes,
                response_bytes * simultaneous
            );
            let mut waiter = Box::pin(admission.acquire(tier));
            assert_pending(waiter.as_mut()).await;

            drop(owners.pop());
            let replacement = bounded(waiter).await.unwrap();
            assert_eq!(admission.snapshot().pages_active, simultaneous);
            assert_eq!(admission.snapshot().available_bytes, 0);
            drop((owners, replacement));
            assert_idle(&admission);
            let snapshot = admission.snapshot();
            assert_eq!(snapshot.peak_reserved_bytes, 16 * MIB);
            assert_eq!(snapshot.peak_pages_active, simultaneous);
            assert_eq!(
                snapshot.peak_response_allowance_bytes,
                response_bytes * simultaneous
            );
        }
    });
}

#[test]
fn mixed_pages_drain_before_the_full_response_can_enter_exclusively() {
    runtime().block_on(async {
        let admission = Arc::new(ReaderAdmission::new());
        let mut owners = Vec::new();
        for tier in [
            ReaderTier(3),
            ReaderTier(2),
            ReaderTier(1),
            ReaderTier::INITIAL,
            ReaderTier::INITIAL,
            ReaderTier::INITIAL,
            ReaderTier::INITIAL,
        ] {
            owners.push(bounded(admission.acquire(tier)).await.unwrap());
        }
        assert_eq!(admission.snapshot().reserved_bytes, 16 * MIB);
        assert_eq!(admission.snapshot().pages_active, 7);
        let mut exclusive = Box::pin(admission.acquire(ReaderTier::FULL));
        assert_pending(exclusive.as_mut()).await;

        drop(owners.remove(0));
        assert_eq!(admission.snapshot().reserved_bytes, 8 * MIB);
        assert_pending(exclusive.as_mut()).await;
        drop(owners);
        let exclusive = bounded(exclusive).await.unwrap();
        let snapshot = admission.snapshot();
        assert_eq!(snapshot.pages_active, 1);
        assert_eq!(snapshot.reserved_bytes, 16 * MIB);
        assert_eq!(snapshot.response_allowance_bytes, 24 * MIB);
        assert_eq!(snapshot.available_bytes, 0);

        let mut small = Box::pin(admission.acquire(ReaderTier::INITIAL));
        assert_pending(small.as_mut()).await;
        drop(small);
        assert_eq!(admission.snapshot().reserved_bytes, 16 * MIB);
        drop(exclusive);
        assert_idle(&admission);
        assert_eq!(admission.snapshot().peak_response_allowance_bytes, 24 * MIB);
    });
}

#[test]
fn concurrent_expansions_release_old_pages_before_waiting_for_more_credit() {
    runtime().block_on(async {
        let admission = Arc::new(ReaderAdmission::new());
        let context = DownloadReaderContext::new(admission.clone());
        let barrier = Arc::new(Barrier::new(4));
        let (ready, mut arrivals) = mpsc::channel(3);
        let mut workers = Vec::new();
        for tier in [ReaderTier(2), ReaderTier(2), ReaderTier(3)] {
            let context = context.clone();
            let barrier = barrier.clone();
            let ready = ready.clone();
            workers.push(tokio::spawn(async move {
                let old = context.admission.acquire(tier).await.unwrap();
                ready.send(()).await.unwrap();
                barrier.wait().await;
                // Rejected response ownership ends before a larger request queues.
                drop(old);
                let next = context.advance(tier).unwrap();
                let expanded = context.admission.acquire(next).await.unwrap();
                assert!(expanded.response_limit > tier.bytes());
                tokio::task::yield_now().await;
                drop(expanded);
            }));
        }
        drop(ready);
        for _ in 0..3 {
            assert_eq!(bounded(arrivals.recv()).await, Some(()));
        }
        assert_eq!(admission.snapshot().pages_active, 3);
        assert_eq!(admission.snapshot().reserved_bytes, 16 * MIB);
        bounded(barrier.wait()).await;
        for worker in workers {
            bounded(worker).await.unwrap();
        }
        assert_idle(&admission);
        assert_eq!(admission.snapshot().peak_reserved_bytes, 16 * MIB);
        assert_eq!(context.tier(), ReaderTier(4));
    });
}

#[test]
fn cancelling_a_byte_waiter_keeps_the_existing_owner_reserved() {
    runtime().block_on(async {
        let admission = Arc::new(ReaderAdmission::new());
        let owner = bounded(admission.acquire(ReaderTier::FULL)).await.unwrap();
        let mut waiter = Box::pin(admission.acquire(ReaderTier::INITIAL));
        assert_pending(waiter.as_mut()).await;
        drop(waiter);
        let snapshot = admission.snapshot();
        assert_eq!(snapshot.pages_active, 1);
        assert_eq!(snapshot.reserved_bytes, 16 * MIB);
        assert_eq!(snapshot.response_allowance_bytes, 24 * MIB);
        assert_eq!(snapshot.available_bytes, 0);
        drop(owner);
        assert_idle(&admission);
    });
}

#[test]
fn cancelling_a_pending_network_owner_returns_its_credit_to_a_waiter() {
    runtime().block_on(async {
        let admission = Arc::new(ReaderAdmission::new());
        let tracked = admission.clone();
        let (started, ready) = oneshot::channel();
        let owner = tokio::spawn(async move {
            let _reservation = tracked.acquire(ReaderTier(3)).await.unwrap();
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        bounded(ready).await.unwrap();
        assert_eq!(admission.snapshot().reserved_bytes, 8 * MIB);
        let mut waiter = Box::pin(admission.acquire(ReaderTier::FULL));
        assert_pending(waiter.as_mut()).await;
        owner.abort();
        assert!(bounded(owner).await.unwrap_err().is_cancelled());
        let replacement = bounded(waiter).await.unwrap();
        assert_eq!(admission.snapshot().pages_active, 1);
        assert_eq!(admission.snapshot().reserved_bytes, 16 * MIB);
        drop(replacement);
        assert_idle(&admission);
    });
}

#[test]
fn only_two_parsers_run_and_cancelling_a_parser_waiter_releases_its_page() {
    runtime().block_on(async {
        let admission = Arc::new(ReaderAdmission::for_tests(4 * MIB, 2));
        let gate = Arc::new(BlockingGate::default());
        let _release = ReleaseOnDrop(gate.clone());
        let mut workers = Vec::new();
        let mut arrivals = Vec::new();
        for body in ["first synthetic Reader", "second synthetic Reader"] {
            let page = bounded(admission.acquire(ReaderTier::INITIAL))
                .await
                .unwrap()
                .page(body.into());
            let gate = gate.clone();
            let (started, ready) = oneshot::channel();
            arrivals.push(ready);
            workers.push(tokio::spawn(async move {
                page.parse(move |html| {
                    started.send(()).unwrap();
                    gate.wait();
                    Ok(html.len())
                })
                .await
            }));
        }
        for arrival in arrivals {
            bounded(arrival).await.unwrap();
        }
        let third = bounded(admission.acquire(ReaderTier::INITIAL))
            .await
            .unwrap()
            .page("waiting synthetic Reader".into());
        let attempted = Arc::new(AtomicUsize::new(0));
        let observed = attempted.clone();
        let mut waiter = Box::pin(third.parse(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));
        assert_pending(waiter.as_mut()).await;
        let snapshot = admission.snapshot();
        assert_eq!(snapshot.parsers_active, 2);
        assert_eq!(snapshot.parsers_waiting, 1);
        assert_eq!(snapshot.pages_active, 3);
        assert_eq!(snapshot.reserved_bytes, 1536 * 1024);
        drop(waiter);
        let snapshot = admission.snapshot();
        assert_eq!(snapshot.parsers_active, 2);
        assert_eq!(snapshot.parsers_waiting, 0);
        assert_eq!(snapshot.pages_active, 2);
        assert_eq!(snapshot.reserved_bytes, MIB);
        assert_eq!(attempted.load(Ordering::SeqCst), 0);

        gate.release();
        for worker in workers {
            assert!(bounded(worker).await.unwrap().unwrap() > 0);
        }
        assert_idle(&admission);
        assert_eq!(admission.snapshot().peak_parsers_active, 2);
        assert!(admission.snapshot().peak_parsers_waiting >= 1);
    });
}

#[test]
fn aborting_the_async_owner_does_not_release_a_running_blocking_parser() {
    runtime().block_on(async {
        let admission = Arc::new(ReaderAdmission::new());
        let gate = Arc::new(BlockingGate::default());
        let _release = ReleaseOnDrop(gate.clone());
        let body = "synthetic retained Reader body".repeat(1024);
        let body_bytes = body.len();
        let page = bounded(admission.acquire(ReaderTier(1)))
            .await
            .unwrap()
            .page(body);
        let finished_bytes = Arc::new(AtomicUsize::new(0));
        let observed = finished_bytes.clone();
        let blocked = gate.clone();
        let (started, ready) = oneshot::channel();
        let owner = tokio::spawn(async move {
            page.parse(move |html| {
                started.send(()).unwrap();
                blocked.wait();
                observed.store(html.len(), Ordering::SeqCst);
                Ok(())
            })
            .await
        });
        bounded(ready).await.unwrap();
        owner.abort();
        assert!(bounded(owner).await.unwrap_err().is_cancelled());
        let snapshot = admission.snapshot();
        assert_eq!(snapshot.pages_active, 1);
        assert_eq!(snapshot.reserved_bytes, 2 * MIB);
        assert_eq!(snapshot.response_allowance_bytes, 2 * MIB);
        assert_eq!(snapshot.parsers_active, 1);
        assert_eq!(finished_bytes.load(Ordering::SeqCst), 0);

        let mut replacement = Box::pin(admission.acquire(ReaderTier::FULL));
        assert_pending(replacement.as_mut()).await;
        gate.release();
        let replacement = bounded(replacement).await.unwrap();
        bounded(async {
            while admission.snapshot().parsers_active != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert_eq!(finished_bytes.load(Ordering::SeqCst), body_bytes);
        assert_eq!(admission.snapshot().pages_active, 1);
        assert_eq!(admission.snapshot().response_allowance_bytes, 24 * MIB);
        drop(replacement);
        assert_idle(&admission);
    });
}

#[test]
fn parser_errors_and_panics_return_all_page_and_worker_credit() {
    runtime().block_on(async {
        let admission = Arc::new(ReaderAdmission::new());
        let page = bounded(admission.acquire(ReaderTier(1)))
            .await
            .unwrap()
            .page("invalid synthetic Reader".into());
        let result = bounded(
            page.parse::<(), _>(|_| Err(SourceError::InvalidResponse("synthetic_reader_parser"))),
        )
        .await;
        assert!(matches!(
            result,
            Err(SourceError::InvalidResponse("synthetic_reader_parser"))
        ));
        assert_idle(&admission);

        let page = bounded(admission.acquire(ReaderTier::FULL))
            .await
            .unwrap()
            .page("synthetic Reader panic".into());
        let result = bounded(page.parse::<(), _>(|_| panic!("Synthetic parser failure"))).await;
        assert!(matches!(
            result,
            Err(SourceError::InvalidResponse("reader_parse_worker"))
        ));
        assert_idle(&admission);
    });
}

#[test]
fn learned_tiers_are_shared_monotonic_and_reset_with_a_new_context() {
    let admission = Arc::new(ReaderAdmission::new());
    let context = DownloadReaderContext::new(admission.clone());
    let concurrent = context.clone();
    assert_eq!(context.tier(), ReaderTier::INITIAL);
    for (attempted, expected) in [
        (ReaderTier(0), ReaderTier(1)),
        (ReaderTier(1), ReaderTier(2)),
        (ReaderTier(2), ReaderTier(3)),
        (ReaderTier(3), ReaderTier(4)),
        (ReaderTier(4), ReaderTier::FULL),
    ] {
        assert_eq!(context.advance(attempted), Some(expected));
        assert_eq!(concurrent.tier(), expected);
        // A slower chapter's earlier response cannot lower the task's hint.
        assert_eq!(concurrent.advance(ReaderTier::INITIAL), Some(expected));
        assert_eq!(context.tier(), expected);
    }
    assert_eq!(concurrent.advance(ReaderTier::FULL), None);
    let replacement = DownloadReaderContext::new(context.admission.clone());
    assert_eq!(replacement.tier(), ReaderTier::INITIAL);
    assert!(Arc::ptr_eq(&replacement.admission, &admission));
    assert_eq!(context.tier(), ReaderTier::FULL);
    assert_eq!(concurrent.tier(), ReaderTier::FULL);
    assert_idle(&admission);
}
