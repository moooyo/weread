//! Actual Reader requests share byte admission across task replacement.

use super::*;
use crate::source::reader_admission::{READER_BUDGET_BYTES, ReaderTier};

#[derive(Default)]
struct ReaderState {
    attempts: BTreeMap<usize, usize>,
    held: BTreeSet<usize>,
    released: bool,
}

#[derive(Default)]
struct ReaderHolds {
    state: Mutex<ReaderState>,
    changed: Condvar,
}

impl ReaderHolds {
    fn arrive(&self, index: usize) {
        let mut state = self.state.lock().unwrap();
        let attempt = state.attempts.entry(index).or_default();
        *attempt += 1;
        if *attempt == 1 {
            self.changed.notify_all();
            return;
        }
        state.held.insert(index);
        self.changed.notify_all();
        let deadline = Instant::now() + SHARD_HOLD_WAIT;
        while !state.released {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "The Reader response was not released");
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }

    fn wait_for_full_request(&self, index: usize) {
        let deadline = Instant::now() + CASE_WAIT;
        let mut state = self.state.lock().unwrap();
        while !state.held.contains(&index) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "The gated full Reader request did not start"
            );
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }

    fn release(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .released = true;
        self.changed.notify_all();
    }
}

struct StopAndRelease<'a> {
    jobs: &'a Jobs,
    responses: Arc<ReaderHolds>,
}

impl Drop for StopAndRelease<'_> {
    fn drop(&mut self) {
        self.jobs.cancel_all();
        self.responses.release();
    }
}

fn one_chapter_task(id: &str, index: usize) -> (DownloadTask, DownloadPlan) {
    let task = DownloadTask {
        id: id.into(),
        book_id: BOOK.into(),
        account_key: crate::source::report::account_key(&fixture_auth()).unwrap(),
        chapters: vec![index as u32],
        status: DownloadStatus::Running,
        include_images: false,
        ..Default::default()
    };
    let chapter = Chapter {
        id: chapter_id(index),
        book_id: BOOK.into(),
        index: index as u32,
        title: format!("Synthetic Reader gate chapter {index}"),
        ..Default::default()
    };
    let plan = DownloadPlan::from_headers(&task, &[chapter]).unwrap();
    (task, plan)
}

fn eventually(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + CASE_WAIT;
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "The cancelled Reader retained admission"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn cancelling_and_replacing_jobs_reuses_the_actual_large_reader_gate() {
    let chapters = catalog();
    let pages = (0..2)
        .map(|index| {
            (
                reader_path(index),
                (index, compressed_reader(index, &chapters)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let responses = Arc::new(ReaderHolds::default());
    let observed = responses.clone();
    let fixture = bounded_fixture(move |request| {
        let (index, bytes) = pages
            .get(&request.path)
            .expect("A cancelled Reader must not dispatch chapter shards");
        observed.arrive(*index);
        compressed_reply(bytes)
    });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    let jobs = Jobs::new(
        runtime.handle().clone(),
        sender,
        Some(fixture_client(&fixture)),
    )
    .unwrap();
    let cleanup = StopAndRelease {
        jobs: &jobs,
        responses: responses.clone(),
    };
    let gate = jobs.download_reader_gate_for_tests();
    let fetch_capacity = jobs.download_fetch_capacity_for_tests();
    let resource_capacity = jobs.download_resource_capacity_for_tests();
    for (index, id) in [(0, "first-reader-task"), (1, "replacement-reader-task")] {
        let (task, plan) = one_chapter_task(id, index);
        assert!(
            jobs.download_with_concurrency(index as u64 + 101, task, plan, 8)
                .is_none()
        );
        responses.wait_for_full_request(index);
        assert!(Arc::ptr_eq(&gate, &jobs.download_reader_gate_for_tests()));
        let snapshot = gate.snapshot();
        assert_eq!(snapshot.pages_active, 1);
        assert_eq!(snapshot.reserved_bytes, 2 * 1024 * 1024);
        assert_eq!(
            snapshot.available_bytes,
            READER_BUDGET_BYTES - snapshot.reserved_bytes
        );
        assert!(jobs.cancel_download(id).is_some());
        eventually(|| {
            gate.snapshot().available_bytes == READER_BUDGET_BYTES
                && gate.snapshot().reserved_bytes == 0
                && jobs.download_fetch_capacity_for_tests() == fetch_capacity
                && jobs.download_resource_capacity_for_tests() == resource_capacity
                && jobs.active_download().is_none()
        });
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }
    jobs.replace_session(AuthSession {
        api_key: "synthetic-replacement-account".into(),
        ..Default::default()
    })
    .unwrap();
    assert!(Arc::ptr_eq(&gate, &jobs.download_reader_gate_for_tests()));
    assert_eq!(gate.snapshot().available_bytes, READER_BUDGET_BYTES);
    drop(cleanup);
    drop(jobs);
    drop(receiver);
    let requests = fixture.finish();
    for index in 0..2 {
        assert_eq!(count(&requests, &reader_path(index)), 2);
    }
}

#[test]
fn cancelling_a_large_reader_waiter_does_not_release_another_owners_permit() {
    let index = 0;
    let fixture = serve_large_catalog(&[index], None);
    let client = fixture_client(&fixture);
    let gate = Arc::new(ReaderAdmission::new());
    let mut hints = ChapterReadHints::for_book(&client, BOOK);
    hints.set_download_reader_gate(gate.clone());
    runtime().block_on(async {
        let held = gate.acquire(ReaderTier::FULL).await.unwrap();
        let id = chapter_id(index);
        let mut chapter = Box::pin(client.chapter_prefetch_with_hints(BOOK, &id, &mut hints));
        tokio::select! {
            biased;
            result = &mut chapter => panic!("The occupied Reader gate was bypassed: {result:?}"),
            _ = tokio::task::yield_now() => {},
        }
        assert_eq!(gate.snapshot().available_bytes, 0);
        drop(chapter);
        assert_eq!(gate.snapshot().available_bytes, 0);
        assert_eq!(gate.snapshot().reserved_bytes, READER_BUDGET_BYTES);
        drop(held);
        assert_eq!(gate.snapshot().available_bytes, READER_BUDGET_BYTES);
        // The cancelled waiter never owned credit or issued a Reader request.
        let content = client
            .chapter_prefetch_with_hints(BOOK, &id, &mut hints)
            .await
            .unwrap();
        assert_eq!(content.content_status, ContentStatus::Full);
        assert_eq!(gate.snapshot().available_bytes, READER_BUDGET_BYTES);
        assert_eq!(gate.snapshot().reserved_bytes, 0);
    });
    let requests = fixture.finish();
    assert_eq!(count(&requests, &reader_path(index)), 2);
    assert_eq!(count(&requests, "/web/book/chapter/t_0"), 1);
}

#[test]
fn a_reader_parser_budget_failure_releases_the_full_budget_permit() {
    let index = 0;
    let page = large_reader(index, &catalog()).replace("large-catalog-clock-0", &"p".repeat(4097));
    let fixture = bounded_fixture(move |request| {
        assert_eq!(request.path, reader_path(index));
        Reply::gzip(&page)
    });
    let client = fixture_client(&fixture);
    let gate = Arc::new(ReaderAdmission::new());
    let mut hints = ChapterReadHints::for_book(&client, BOOK);
    hints.set_download_reader_gate(gate.clone());
    let result =
        runtime().block_on(client.chapter_with_hints(BOOK, &chapter_id(index), &mut hints));
    assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
    assert_eq!(gate.snapshot().available_bytes, READER_BUDGET_BYTES);
    assert_eq!(gate.snapshot().reserved_bytes, 0);
    assert_eq!(gate.snapshot().parsers_active, 0);
    assert_eq!(
        gate.snapshot().peak_response_allowance_bytes,
        24 * 1024 * 1024
    );
    assert_eq!(
        hints.download_reader.as_ref().unwrap().tier(),
        ReaderTier::INITIAL
    );
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn a_header_budget_failure_cannot_be_misclassified_as_a_large_reader() {
    let index = 0;
    let page = compressed_reader(index, &catalog());
    let fixture = bounded_fixture(move |request| {
        assert_eq!(request.path, reader_path(index));
        let mut reply = compressed_reply(&page);
        reply
            .headers
            .push(("x-wr-ticket".into(), "extra-ticket".into()));
        reply
    });
    let client = fixture_client(&fixture);
    let mut original = fixture_auth();
    let encoded = serde_json::to_vec(&original).unwrap().len();
    original.account_name = "x".repeat(AuthSession::MAX_ENCODED_BYTES - encoded);
    assert_eq!(
        serde_json::to_vec(&original).unwrap().len(),
        AuthSession::MAX_ENCODED_BYTES
    );
    client.replace_session(original.clone()).unwrap();
    let gate = Arc::new(ReaderAdmission::new());
    let mut hints = ChapterReadHints::for_book(&client, BOOK);
    hints.set_download_reader_gate(gate.clone());
    let result = runtime().block_on(client.chapter_prefetch_with_hints(
        BOOK,
        &chapter_id(index),
        &mut hints,
    ));
    assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
    assert_eq!(client.session(), original);
    assert_eq!(gate.snapshot().available_bytes, READER_BUDGET_BYTES);
    assert_eq!(gate.snapshot().reserved_bytes, 0);
    assert_eq!(gate.snapshot().peak_parsers_active, 0);
    assert_eq!(
        hints.download_reader.as_ref().unwrap().tier(),
        ReaderTier::INITIAL
    );
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn an_adaptive_reader_parser_failure_does_not_upgrade_the_transport_tier() {
    let index = 0;
    let page = large_reader(index, &catalog()).replace("large-catalog-clock-0", &"p".repeat(4097));
    let fixture = bounded_fixture(move |request| {
        assert_eq!(request.path, reader_path(index));
        Reply::gzip(&page)
    });
    let client = fixture_client(&fixture);
    let admission = Arc::new(ReaderAdmission::new());
    let mut hints = ChapterReadHints::for_book(&client, BOOK);
    hints.set_download_reader_gate(admission.clone());
    let result = runtime().block_on(client.chapter_prefetch_with_hints(
        BOOK,
        &chapter_id(index),
        &mut hints,
    ));
    assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
    assert_eq!(
        hints.download_reader.as_ref().unwrap().tier().bytes(),
        2 * 1024 * 1024
    );
    let snapshot = admission.snapshot();
    assert_eq!(snapshot.available_bytes, READER_BUDGET_BYTES);
    assert_eq!(snapshot.reserved_bytes, 0);
    assert_eq!(snapshot.parsers_active, 0);
    assert_eq!(snapshot.peak_parsers_active, 1);
    assert_eq!(fixture.finish().len(), 2);
}
