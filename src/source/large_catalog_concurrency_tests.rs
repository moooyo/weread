//! Large synthetic catalogs must not disable bounded chapter prefetch.
//!
//! Reader pages contain real catalog entries rather than whitespace padding.
//! All accounts, identifiers, titles, and chapter content are synthetic.

use super::*;
use crate::{
    domain::{Chapter, ContentStatus, DownloadStatus, DownloadTask},
    jobs::{DownloadPlan, JobEvent, JobResult, Jobs},
};
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[path = "large_reader_gate_tests.rs"]
mod large_reader_gate_tests;

#[path = "reader_admission_freshness_tests.rs"]
mod reader_admission_freshness_tests;

#[path = "reader_admission_benchmark.rs"]
mod reader_admission_benchmark;

#[path = "reader_admission_transport_tests.rs"]
mod reader_admission_transport_tests;

const BOOK: &str = "fixture-book";
const CATALOG_LEN: usize = 1_863;
const WINDOW: usize = 8;
const SELECTED: usize = WINDOW * 2 + 1;
const SMALL_RESPONSE_LIMIT: usize = 512 * 1024;
const CASE_WAIT: Duration = Duration::from_secs(10);
const SHARD_HOLD_WAIT: Duration = Duration::from_secs(25);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const FIXTURE_LIFETIME: Duration = Duration::from_secs(60);

fn chapter_id(index: usize) -> String {
    format!("large-catalog-chapter-{index:04}")
}

fn reader_path(index: usize) -> String {
    let url = crate::source::protocol::reader_url_checked(BOOK, Some(&chapter_id(index))).unwrap();
    reqwest::Url::parse(&url).unwrap().path().to_owned()
}

fn catalog() -> Vec<Value> {
    (0..CATALOG_LEN)
        .map(|index| {
            json!({
                "chapterUid": chapter_id(index),
                "chapterIdx": index + 1,
                "title": format!(
                    "Fixture chapter {index:04}: {}",
                    "Synthetic catalog metadata ".repeat(12),
                ),
                "wordCount": 1,
                "paid": 1,
                "price": 0,
            })
        })
        .collect()
}

fn large_reader(index: usize, chapters: &[Value]) -> String {
    assert_eq!(chapters.len(), CATALOG_LEN);
    let state = json!({
        "user": { "vid": fixture_auth().user_vid },
        "reader": {
            "bookId": BOOK,
            "bookInfo": { "bookId": BOOK, "paid": 1, "payType": 1 },
            "chapterInfos": chapters,
            "currentChapter": chapters[index].clone(),
            "psvts": format!("large-catalog-clock-{index}"),
            "pclts": 0,
        },
    });
    let html = format!("<script>window.__INITIAL_STATE__={state};</script>");
    assert!(html.len() > SMALL_RESPONSE_LIMIT);
    assert!(html.len() < 2 * 1024 * 1024);
    html
}

fn compressed_reader(index: usize, chapters: &[Value]) -> Vec<u8> {
    let reply = Reply::gzip(large_reader(index, chapters));
    assert!(
        reply.body.len() < SMALL_RESPONSE_LIMIT,
        "The fixture must exercise the decoded Reader size, not the wire size"
    );
    reply.body
}

fn compressed_reply(bytes: &[u8]) -> Reply {
    Reply {
        status: 200,
        headers: vec![("Content-Encoding".into(), "gzip".into())],
        body: bytes.to_vec(),
    }
}

enum ReaderReads {
    Ordinary,
    FirstAdaptive,
    AdaptiveWindow,
}

fn assert_reader_request_count(actual: usize, expected: ReaderReads) {
    match expected {
        ReaderReads::Ordinary => {
            assert_eq!(
                actual, 1,
                "An ordinary Reader was unnecessarily fetched again"
            );
        }
        ReaderReads::FirstAdaptive => {
            assert_eq!(
                actual, 2,
                "The first large Reader needs one probe and one gated read"
            );
        }
        ReaderReads::AdaptiveWindow => {
            // A small Reader probe may precede one gated full-budget read. An
            // already selected large-Reader mode can proceed directly to that read.
            assert!(
                (1..=2).contains(&actual),
                "Expected one Reader acquisition with at most one small-budget probe; got {actual}"
            );
        }
    }
}

fn assert_reader_window(readers: &[usize], admitted: usize) {
    let mut counts = BTreeMap::<usize, usize>::new();
    for &index in readers {
        *counts.entry(index).or_default() += 1;
    }
    assert_eq!(
        counts.keys().copied().collect::<Vec<_>>(),
        (0..admitted).collect::<Vec<_>>()
    );
    for count in counts.into_values() {
        assert_reader_request_count(count, ReaderReads::AdaptiveWindow);
    }
}

fn bounded_fixture<F>(handler: F) -> Fixture
where
    F: Fn(&RecordedRequest) -> Reply + Send + Sync + 'static,
{
    // Keep the shared fixture's request parser and result shape, but give this
    // deliberately gated two-window case more than its default 15 seconds.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let records = Arc::new(Mutex::new(Vec::new()));
    let captured = records.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let handler = Arc::new(handler);
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + FIXTURE_LIFETIME;
        let mut workers = Vec::new();
        while !stopping.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    assert!(workers.len() < 256, "Unexpected repeated source requests");
                    let index = workers.len();
                    let captured = captured.clone();
                    let handler = handler.clone();
                    workers.push(thread::spawn(move || {
                        stream.set_read_timeout(Some(CASE_WAIT)).unwrap();
                        stream.set_write_timeout(Some(CASE_WAIT)).unwrap();
                        let request = read_request(&mut stream);
                        captured.lock().unwrap().push((index, request.clone()));
                        let reply = handler(&request);
                        if reply.status == 0 {
                            return;
                        }
                        let mut headers = format!(
                            "HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n",
                            reply.status,
                            reply.body.len(),
                        );
                        for (name, value) in reply.headers {
                            headers.push_str(&format!("{name}: {value}\r\n"));
                        }
                        headers.push_str("\r\n");
                        // Failed probes and cancellation can close a response
                        // before the fixture finishes writing its complete body.
                        if stream.write_all(headers.as_bytes()).is_ok() {
                            let _ = stream.write_all(&reply.body);
                        }
                    }));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "Fixture was not stopped");
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("Synthetic listener failed: {error}"),
            }
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });
    Fixture {
        origin,
        stop,
        records,
        worker: Some(worker),
    }
}

fn fixture_client(fixture: &Fixture) -> WeReadClient {
    let mut client = fixture.client();
    client.http = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(HTTP_TIMEOUT)
        .build()
        .unwrap();
    client
}

#[derive(Default)]
struct GateState {
    readers: Vec<usize>,
    catalog_requests: usize,
    t0_arrived: BTreeSet<usize>,
    t0_blocked: BTreeSet<usize>,
    t1_arrived: BTreeSet<usize>,
    released: BTreeSet<usize>,
    release_all: bool,
    peak_blocked_t0: usize,
}

#[derive(Default)]
struct ShardGates {
    state: Mutex<GateState>,
    changed: Condvar,
}

impl ShardGates {
    fn wait_for(&self, condition: impl Fn(&GateState) -> bool, description: &str) {
        let deadline = Instant::now() + CASE_WAIT;
        let mut state = self.state.lock().unwrap();
        while !condition(&state) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "Timed out: {description}");
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }

    fn record_reader(&self, index: usize) {
        self.state.lock().unwrap().readers.push(index);
        self.changed.notify_all();
    }

    fn record_catalog(&self) {
        self.state.lock().unwrap().catalog_requests += 1;
        self.changed.notify_all();
    }

    fn hold_t0(&self, index: usize) {
        let deadline = Instant::now() + SHARD_HOLD_WAIT;
        let mut state = self.state.lock().unwrap();
        assert!(
            state.t0_arrived.insert(index),
            "A small chapter body was fetched more than once"
        );
        state.t0_blocked.insert(index);
        state.peak_blocked_t0 = state.peak_blocked_t0.max(state.t0_blocked.len());
        self.changed.notify_all();
        while !state.release_all && !state.released.contains(&index) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Allow fixture shutdown even if the coordinating assertion fails.
                state.t0_blocked.remove(&index);
                self.changed.notify_all();
                panic!("The coordinator did not release a synthetic shard response");
            }
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
        state.t0_blocked.remove(&index);
        self.changed.notify_all();
    }

    fn record_t1(&self, index: usize) {
        self.state.lock().unwrap().t1_arrived.insert(index);
        self.changed.notify_all();
    }

    fn release(&self, indices: impl IntoIterator<Item = usize>) {
        self.state.lock().unwrap().released.extend(indices);
        self.changed.notify_all();
    }

    fn release_everything(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .release_all = true;
        self.changed.notify_all();
    }
}

struct CancelAndRelease<'a> {
    jobs: &'a Jobs,
    gates: Arc<ShardGates>,
}

impl Drop for CancelAndRelease<'_> {
    fn drop(&mut self) {
        // Fixture::Drop joins its HTTP threads. Open every response gate first
        // so an assertion failure or cancellation cannot strand those threads.
        self.jobs.cancel_all();
        self.gates.release_everything();
    }
}

fn serve_large_catalog(indices: &[usize], gates: Option<Arc<ShardGates>>) -> Fixture {
    let chapters = catalog();
    let pages = indices
        .iter()
        .map(|&index| {
            (
                reader_path(index),
                (index, compressed_reader(index, &chapters)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let catalog_reply =
        Reply::gzip(json!({"data": [{"bookId": BOOK, "updated": chapters}]}).to_string()).body;
    bounded_fixture(move |request| {
        if request.path == "/web/book/chapterInfos" {
            if let Some(gates) = &gates {
                gates.record_catalog();
            }
            return compressed_reply(&catalog_reply);
        }
        if let Some((index, bytes)) = pages.get(&request.path) {
            if let Some(gates) = &gates {
                gates.record_reader(*index);
            }
            return compressed_reply(bytes);
        }
        let index = clock(request)
            .strip_prefix("large-catalog-clock-")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        match request.path.as_str() {
            "/web/book/chapter/e_0" => Reply::text(r#"{"bookId":"fixture-book"}"#),
            "/web/book/chapter/t_0" => {
                if let Some(gates) = &gates {
                    gates.hold_t0(index);
                }
                Reply::text(TXT_SHARD)
            }
            "/web/book/chapter/t_1" => {
                if let Some(gates) = &gates {
                    gates.record_t1(index);
                }
                Reply::status(404)
            }
            _ => panic!("Unexpected synthetic operation"),
        }
    })
}

#[test]
fn large_catalog_normal_read_retains_full_access_to_short_text() {
    // Exercise a target at the end of the catalog, with no synthetic whitespace
    // padding and no fabricated permission shortcut in the source adapter.
    let index = CATALOG_LEN - 1;
    let fixture = serve_large_catalog(&[index], None);
    let client = fixture_client(&fixture);
    let content = runtime()
        .block_on(client.chapter(BOOK, &chapter_id(index)))
        .unwrap();
    assert_eq!(content.plain_text, "Hello, reader!");
    assert_eq!(content.content_status, ContentStatus::Full);
    let requests = fixture.finish();
    assert_reader_request_count(count(&requests, &reader_path(index)), ReaderReads::Ordinary);
    assert_eq!(count(&requests, "/web/book/chapter/t_0"), 1);
    assert_eq!(count(&requests, "/web/book/chapter/t_1"), 1);
    assert_eq!(count(&requests, "/web/login/renewal"), 0);
}

#[test]
fn large_catalog_prefetch_retains_full_access_to_short_text() {
    let index = CATALOG_LEN - 1;
    let fixture = serve_large_catalog(&[index], None);
    let client = fixture_client(&fixture);
    let mut hints = ChapterReadHints::for_book(&client, BOOK);
    let reader_gate = Arc::new(ReaderAdmission::new());
    hints.set_download_reader_gate(reader_gate.clone());
    let content = runtime()
        .block_on(client.chapter_prefetch_with_hints(BOOK, &chapter_id(index), &mut hints))
        .unwrap();
    assert_eq!(content.plain_text, "Hello, reader!");
    assert_eq!(content.content_status, ContentStatus::Full);
    assert_eq!(reader_gate.snapshot().available_bytes, 16 * 1024 * 1024);
    let requests = fixture.finish();
    assert_reader_request_count(
        count(&requests, &reader_path(index)),
        ReaderReads::FirstAdaptive,
    );
    assert_eq!(count(&requests, "/web/book/chapter/t_0"), 1);
    assert_eq!(count(&requests, "/web/book/chapter/t_1"), 1);
    assert_eq!(count(&requests, "/web/login/renewal"), 0);
}

fn selected_task() -> (DownloadTask, DownloadPlan) {
    let task = DownloadTask {
        id: "large-catalog-two-windows".into(),
        book_id: BOOK.into(),
        account_key: crate::source::report::account_key(&fixture_auth()).unwrap(),
        chapters: (0..SELECTED as u32).collect(),
        status: DownloadStatus::Running,
        include_images: true,
        ..Default::default()
    };
    let chapters = (0..SELECTED)
        .map(|index| Chapter {
            id: chapter_id(index),
            book_id: BOOK.into(),
            index: index as u32,
            title: format!("Selected synthetic chapter {index}"),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let plan = DownloadPlan::from_headers(&task, &chapters).unwrap();
    (task, plan)
}

fn next_non_progress(receiver: &Receiver<JobEvent>) -> JobEvent {
    let deadline = Instant::now() + CASE_WAIT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "A chapter checkpoint did not arrive");
        let event = receiver.recv_timeout(remaining).unwrap();
        if !matches!(&event.result, JobResult::DownloadProgress { .. }) {
            return event;
        }
    }
}

fn acknowledge(event: JobEvent, index: usize) {
    let JobEvent {
        result, credits, ..
    } = event;
    assert!(
        credits.is_some(),
        "The queued result lost its admission lease"
    );
    let JobResult::Downloaded {
        chapter,
        next_index,
        html,
        metadata,
        images,
        checkpoint,
        ..
    } = result
    else {
        panic!("Expected a complete chapter, received {result:?}");
    };
    assert_eq!(chapter.index, index as u32);
    assert_eq!(chapter.id, chapter_id(index));
    assert_eq!(next_index, index + 1);
    assert!(chapter.cached);
    assert_eq!(chapter.content_status, ContentStatus::Full);
    assert_eq!(chapter.body.as_deref(), Some("Hello, reader!"));
    assert!(html.is_some() && metadata.is_some());
    // include_images remains enabled, but the synthetic TXT has no image URLs.
    assert!(images.is_some());
    checkpoint.send(Ok(())).unwrap();
    drop((chapter, html, metadata, images, credits));
}

#[test]
fn adaptive_reader_budget_refills_while_the_first_chapter_stays_blocked() {
    // Reader byte credit is returned before the chapter's shard barrier.
    // Independent checkpoints must refill all other workers while index zero
    // stays in its original HTTP request. This is not a process RSS assertion.
    let indices = (0..SELECTED).collect::<Vec<_>>();
    let gates = Arc::new(ShardGates::default());
    let fixture = serve_large_catalog(&indices, Some(gates.clone()));
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
    let cleanup = CancelAndRelease {
        jobs: &jobs,
        gates: gates.clone(),
    };
    let (task, plan) = selected_task();
    assert!(
        jobs.download_with_concurrency(91, task, plan, WINDOW as u32)
            .is_none()
    );

    gates.wait_for(
        |state| (0..WINDOW).all(|index| state.t0_blocked.contains(&index)),
        "the first eight simultaneous chapter shard requests",
    );
    {
        let state = gates.state.lock().unwrap();
        assert_eq!(state.t0_blocked.len(), WINDOW);
        assert_reader_window(&state.readers, WINDOW);
    }
    assert_eq!(
        jobs.download_reader_gate_for_tests()
            .snapshot()
            .available_bytes,
        16 * 1024 * 1024,
        "Completed Reader pages retained their credit during the first shard window"
    );
    gates.release(1..WINDOW);
    let mut completed = BTreeSet::new();
    for _ in 1..WINDOW {
        let event = next_non_progress(&receiver);
        let JobResult::Downloaded { chapter, .. } = &event.result else {
            panic!("A completed suffix did not produce its own checkpoint");
        };
        let index = chapter.index as usize;
        assert!((1..WINDOW).contains(&index));
        assert!(completed.insert(index));
        acknowledge(event, index);
    }

    // Seven independent ACKs refill seven readers beside the still-blocked
    // head. No replay or checkpoint from that head is needed for progress.
    gates.wait_for(
        |state| {
            state.t0_blocked.contains(&0)
                && (WINDOW..WINDOW * 2 - 1).all(|index| state.t0_blocked.contains(&index))
        },
        "seven refilled chapter shard requests beside the blocked first chapter",
    );
    {
        let state = gates.state.lock().unwrap();
        assert_eq!(state.t0_blocked.len(), WINDOW);
        assert_reader_window(&state.readers, WINDOW * 2 - 1);
        assert!(!state.t0_arrived.contains(&(WINDOW * 2 - 1)));
    }
    assert_eq!(
        jobs.download_reader_gate_for_tests()
            .snapshot()
            .available_bytes,
        16 * 1024 * 1024,
        "Completed Reader pages retained their credit during the refilled shard window"
    );
    gates.release(WINDOW..WINDOW * 2 - 1);
    for _ in WINDOW..WINDOW * 2 - 1 {
        let event = next_non_progress(&receiver);
        let JobResult::Downloaded { chapter, .. } = &event.result else {
            panic!("A refilled chapter did not produce its own checkpoint");
        };
        let index = chapter.index as usize;
        assert!((WINDOW..WINDOW * 2 - 1).contains(&index));
        assert!(completed.insert(index));
        acknowledge(event, index);
    }
    gates.wait_for(
        |state| {
            state.t0_blocked.contains(&0)
                && (WINDOW * 2 - 1..SELECTED).all(|index| state.t0_blocked.contains(&index))
        },
        "the final two chapters beside the blocked first chapter",
    );
    gates.release(WINDOW * 2 - 1..SELECTED);
    for _ in WINDOW * 2 - 1..SELECTED {
        let event = next_non_progress(&receiver);
        let JobResult::Downloaded { chapter, .. } = &event.result else {
            panic!("A final chapter did not produce its own checkpoint");
        };
        let index = chapter.index as usize;
        assert!((WINDOW * 2 - 1..SELECTED).contains(&index));
        assert!(completed.insert(index));
        acknowledge(event, index);
    }
    assert_eq!(completed, (1..SELECTED).collect::<BTreeSet<_>>());
    assert_eq!(gates.state.lock().unwrap().t0_blocked, BTreeSet::from([0]));
    gates.release([0]);
    acknowledge(next_non_progress(&receiver), 0);
    let terminal = next_non_progress(&receiver);
    assert!(matches!(
        &terminal.result,
        JobResult::DownloadFinished { .. }
    ));
    drop(terminal);
    {
        let state = gates.state.lock().unwrap();
        assert_reader_window(&state.readers, SELECTED);
        assert_eq!(state.catalog_requests, 1);
        assert_eq!(state.t0_arrived.len(), SELECTED);
        assert_eq!(state.t1_arrived.len(), SELECTED);
        assert_eq!(state.peak_blocked_t0, WINDOW);
        assert!(state.t0_blocked.is_empty());
    }
    drop(cleanup);
    drop(receiver);
    drop(jobs);
    let requests = fixture.finish();
    for index in indices {
        assert_reader_request_count(
            count(&requests, &reader_path(index)),
            ReaderReads::AdaptiveWindow,
        );
    }
    assert_eq!(count(&requests, "/web/book/chapterInfos"), 1);
    assert_eq!(count(&requests, "/web/book/chapter/t_0"), SELECTED);
    assert_eq!(count(&requests, "/web/book/chapter/t_1"), SELECTED);
    assert_eq!(count(&requests, "/web/login/renewal"), 0);
}
