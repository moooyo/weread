//! Genuine chapter HTTP requests exercise independent, bounded chapter work.

use super::*;
use crate::{
    domain::{
        Book, Chapter, ContentStatus, DownloadChapterRetry, DownloadChapterStatus, DownloadStatus,
        DownloadTask,
    },
    jobs::{DownloadPlan, JobEvent, JobResult, Jobs},
    preferences::MAX_DOWNLOAD_CONCURRENCY,
    store::Store,
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Condvar,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const BOOK: &str = "concurrent-book";
const ACCOUNT: &str = "concurrent-account";

#[derive(Default)]
struct FixtureOptions {
    blocked: BTreeSet<usize>,
    failed: BTreeSet<usize>,
    rejected_attempts: BTreeMap<usize, usize>,
    preview: Option<usize>,
    large: BTreeSet<usize>,
}

#[derive(Default)]
struct Observations {
    readers: Vec<usize>,
    completed: BTreeSet<usize>,
    active: usize,
    maximum_active: usize,
    released: BTreeSet<usize>,
    release_all: bool,
    shard_requests: usize,
}

#[derive(Default)]
struct Control {
    state: Mutex<Observations>,
    changed: Condvar,
}

impl Control {
    fn wait(&self, condition: impl Fn(&Observations) -> bool, description: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut state = self.state.lock().unwrap();
        while !condition(&state) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "Timed out: {description}");
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }

    fn release(&self, index: usize) {
        self.state.lock().unwrap().released.insert(index);
        self.changed.notify_all();
    }

    fn release_all(&self) {
        self.state.lock().unwrap().release_all = true;
        self.changed.notify_all();
    }
}

struct Fixture {
    origin: String,
    control: Arc<Control>,
    stopped: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn new(count: usize, options: FixtureOptions) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let control = Arc::new(Control::default());
        let observed = control.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let stopping = stopped.clone();
        let reader_paths = Arc::new(
            (0..count)
                .map(|index| {
                    let url = reader_url(BOOK, Some(&chapter_id(index))).unwrap();
                    (url.strip_prefix(WEB_ORIGIN).unwrap().to_owned(), index)
                })
                .collect::<BTreeMap<_, _>>(),
        );
        let bodies = Arc::new(
            (0..count)
                .map(|index| {
                    let text = if options.large.contains(&index) {
                        format!("{}\n", "A complete large chapter. ".repeat(20)).repeat(2_000)
                    } else {
                        format!("Complete fixture chapter {index}.\n")
                    };
                    crate::source::content::fixture_text_shards(&text)
                })
                .collect::<Vec<_>>(),
        );
        let options = Arc::new(options);
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut workers = Vec::new();
            while !stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let control = observed.clone();
                        let paths = reader_paths.clone();
                        let bodies = bodies.clone();
                        let options = options.clone();
                        workers.push(thread::spawn(move || {
                            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                            stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
                            let Some((path, body)) = read_request(&mut stream) else {
                                return;
                            };
                            let (status, body) = if let Some(&index) = paths.get(&path) {
                                let mut state = control.state.lock().unwrap();
                                state.readers.push(index);
                                state.active += 1;
                                state.maximum_active = state.maximum_active.max(state.active);
                                control.changed.notify_all();
                                while options.blocked.contains(&index)
                                    && !state.release_all
                                    && !state.released.contains(&index)
                                {
                                    state = control.changed.wait(state).unwrap();
                                }
                                if options.failed.contains(&index) {
                                    state.active -= 1;
                                    control.changed.notify_all();
                                    (400, String::new())
                                } else {
                                    (200, reader(index, options.preview == Some(index)))
                                }
                            } else {
                                let value: Value = serde_json::from_slice(&body).unwrap();
                                let index = value["ps"]
                                    .as_str()
                                    .unwrap()
                                    .strip_prefix("concurrent-clock-")
                                    .unwrap()
                                    .parse::<usize>()
                                    .unwrap();
                                let mut state = control.state.lock().unwrap();
                                state.shard_requests += 1;
                                match path.as_str() {
                                    "/web/book/chapter/e_0" => {
                                        (200, format!(r#"{{"bookId":"{BOOK}"}}"#))
                                    }
                                    "/web/book/chapter/t_0" => {
                                        let attempts = state
                                            .readers
                                            .iter()
                                            .filter(|&&read| read == index)
                                            .count();
                                        if options
                                            .rejected_attempts
                                            .get(&index)
                                            .is_some_and(|&rejected| attempts <= rejected)
                                        {
                                            state.active = state.active.saturating_sub(1);
                                            control.changed.notify_all();
                                            (200, r#"{"errCode":-10102}"#.into())
                                        } else {
                                            (200, bodies[index].0.clone())
                                        }
                                    }
                                    "/web/book/chapter/t_1" => {
                                        state.active = state.active.saturating_sub(1);
                                        state.completed.insert(index);
                                        control.changed.notify_all();
                                        (200, bodies[index].1.clone())
                                    }
                                    other => panic!("Unexpected fixture path: {other}"),
                                }
                            };
                            let header = format!(
                                "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            // Cancellation intentionally closes accepted requests.
                            if stream.write_all(header.as_bytes()).is_ok() {
                                let _ = stream.write_all(body.as_bytes());
                            }
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "The fixture was not stopped");
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("Fixture listener failed: {error}"),
                }
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        Self {
            origin,
            control,
            stopped,
            worker: Some(worker),
        }
    }

    fn client(&self) -> WeReadClient {
        let mut client = WeReadClient::new(auth()).unwrap();
        client.test_web_origin = Some(self.origin.clone());
        client.http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        client
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.control.release_all();
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 2048];
    let end = loop {
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= 65536);
        if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let header = std::str::from_utf8(&bytes[..end]).unwrap();
    let path = header.split_whitespace().nth(1).unwrap().to_owned();
    let length = header
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map_or(0, |(_, value)| value.trim().parse::<usize>().unwrap());
    assert!(end + length <= 65536);
    while bytes.len() < end + length {
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    Some((path, bytes[end..end + length].to_vec()))
}

fn auth() -> AuthSession {
    AuthSession {
        api_key: "synthetic-concurrent-api-key".into(),
        user_vid: ACCOUNT.into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), ACCOUNT.into()),
            ("wr_skey".into(), "synthetic-concurrent-session".into()),
        ]),
        ..Default::default()
    }
}

fn chapter_id(index: usize) -> String {
    format!("concurrent-chapter-{index}")
}

fn reader(index: usize, preview: bool) -> String {
    let id = chapter_id(index);
    let state = json!({
        "user": { "vid": ACCOUNT },
        "reader": {
            "bookId": BOOK,
            "bookInfo": {
                "bookId": BOOK,
                "paid": 0,
                "payType": if preview { 1 << 19 } else { 32 }
            },
            "chapterInfos": [{ "chapterUid": id, "chapterIdx": index + 1, "wordCount": 20 }],
            "currentChapter": { "chapterUid": id },
            "psvts": format!("concurrent-clock-{index}"),
            "pclts": "concurrent-client-clock"
        }
    });
    format!("<script>window.__INITIAL_STATE__={state};</script>")
}

fn task(id: &str, count: usize) -> (DownloadTask, DownloadPlan) {
    let task = DownloadTask {
        id: id.into(),
        book_id: BOOK.into(),
        account_key: crate::source::report::account_key(&auth()).unwrap(),
        chapters: (0..count as u32).collect(),
        status: DownloadStatus::Running,
        include_images: false,
        ..Default::default()
    };
    let plan = DownloadPlan::from_headers(&task, &headers(count)).unwrap();
    (task, plan)
}

fn headers(count: usize) -> Vec<Chapter> {
    (0..count)
        .map(|index| Chapter {
            id: chapter_id(index),
            book_id: BOOK.into(),
            index: index as u32,
            title: format!("Fixture chapter {index}"),
            ..Default::default()
        })
        .collect()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

fn jobs(runtime: &tokio::runtime::Runtime, fixture: &Fixture) -> (Jobs, Receiver<JobEvent>) {
    let (sender, receiver) = mpsc::channel();
    (
        Jobs::new(runtime.handle().clone(), sender, Some(fixture.client())).unwrap(),
        receiver,
    )
}

fn receive(receiver: &Receiver<JobEvent>) -> JobEvent {
    receiver
        .recv_timeout(Duration::from_secs(8))
        .expect("Missing genuine Jobs event")
}

fn receive_checkpoint(receiver: &Receiver<JobEvent>) -> JobEvent {
    loop {
        let event = receive(receiver);
        match event.result {
            JobResult::DownloadProgress { .. } => {}
            JobResult::DownloadChapterStarted {
                chapter_id: uid,
                chapter_index,
                ordinal,
                checkpoint,
                ..
            } => {
                assert_eq!(uid, chapter_id(ordinal));
                assert_eq!(chapter_index as usize, ordinal);
                assert!(event.credits.is_some(), "Retry admission must stay bounded");
                checkpoint.send(Ok(())).unwrap();
            }
            _ => return event,
        }
    }
}

fn acknowledge(event: JobEvent, index: usize, full: bool) {
    let JobEvent {
        result, credits, ..
    } = event;
    assert!(credits.is_some(), "The body lost its queue admission lease");
    let JobResult::Downloaded {
        chapter,
        next_index,
        checkpoint,
        html,
        metadata,
        ..
    } = result
    else {
        panic!("Expected a downloaded chapter, received {result:?}");
    };
    assert_eq!(chapter.index, index as u32);
    assert_eq!(next_index, index + 1);
    assert_eq!(
        chapter.content_status,
        if full {
            ContentStatus::Full
        } else {
            ContentStatus::Preview
        }
    );
    assert_eq!(chapter.cached, full);
    assert!(chapter.body.as_ref().is_some_and(|body| !body.is_empty()));
    assert!(html.is_some() && metadata.is_some());
    checkpoint.send(Ok(())).unwrap();
    drop((chapter, html, metadata, credits));
}

fn downloaded_index(event: &JobEvent) -> usize {
    let JobResult::Downloaded { chapter, .. } = &event.result else {
        panic!("Expected a chapter checkpoint, received {:?}", event.result);
    };
    chapter.index as usize
}

fn acknowledge_failure(
    event: JobEvent,
    expected: usize,
    disposition: DownloadChapterRetry,
) -> SourceError {
    let JobEvent {
        result, credits, ..
    } = event;
    let JobResult::DownloadChapterFailed {
        task_id,
        chapter_id: uid,
        chapter_index,
        ordinal,
        error,
        checkpoint,
    } = result
    else {
        panic!("Expected a chapter failure, received {result:?}");
    };
    assert!(
        credits.is_some(),
        "The failure lost its bounded queue credit"
    );
    assert_eq!(chapter_index as usize, expected);
    assert_eq!(ordinal, expected);
    assert_eq!(uid, chapter_id(expected));
    checkpoint.send(Ok(disposition)).unwrap();
    drop((task_id, uid, credits));
    error
}

fn utc_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn commit_full_chapter(event: JobEvent, store: &Store, task: &mut DownloadTask) -> usize {
    let JobEvent {
        result, credits, ..
    } = event;
    let JobResult::Downloaded {
        task_id,
        chapter,
        next_index,
        html,
        metadata,
        images,
        checkpoint,
    } = result
    else {
        panic!("Expected a fresh chapter, received {result:?}");
    };
    let index = chapter.index as usize;
    assert_eq!(task_id, task.id);
    assert!(credits.is_some());
    assert_eq!(chapter.id, chapter_id(index));
    assert_eq!(next_index, index + 1);
    assert_eq!(chapter.content_status, ContentStatus::Full);
    assert!(chapter.cached && chapter.body.as_ref().is_some_and(|body| !body.is_empty()));
    assert!(html.is_some() && metadata.is_some());
    assert!(images.is_none());
    store
        .commit_independent_download_chapter(task, &chapter, next_index - 1, task.updated_at + 1)
        .unwrap();
    *task = store.task(&task.id).unwrap().unwrap();
    checkpoint.send(Ok(())).unwrap();
    drop((task_id, chapter, html, metadata, images, credits));
    index
}

fn assert_resources_released(jobs: &Jobs) {
    eventually(|| {
        let reader = jobs.download_reader_gate_for_tests().snapshot();
        jobs.download_fetch_capacity_for_tests() == MAX_DOWNLOAD_CONCURRENCY as usize
            && jobs.download_resource_capacity_for_tests() == (1, 1)
            && reader.available_bytes == 16 * 1024 * 1024
            && reader.reserved_bytes == 0
            && reader.response_allowance_bytes == 0
            && reader.pages_active == 0
            && reader.parsers_active == 0
    });
    let reader = jobs.download_reader_gate_for_tests().snapshot();
    assert!(reader.peak_reserved_bytes <= 16 * 1024 * 1024);
    assert!(reader.peak_parsers_active <= 2);
}

fn eventually(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(Instant::now() < deadline, "Resources were not released");
        thread::yield_now();
    }
}

#[test]
fn genuine_downloads_honor_serial_default_and_clamped_parallel_limits() {
    for (configured, expected) in [(Some(0), 1usize), (None, 8), (Some(u32::MAX), 16)] {
        let count = expected + 2;
        let fixture = Fixture::new(
            count,
            FixtureOptions {
                blocked: (0..count).collect(),
                ..Default::default()
            },
        );
        let runtime = runtime();
        let (jobs, receiver) = jobs(&runtime, &fixture);
        let (task, plan) = task("parallel-limit", count);
        let result = match configured {
            Some(limit) => jobs.download_with_concurrency(7, task, plan, limit),
            None => jobs.download(7, task, plan),
        };
        assert!(result.is_none());
        fixture.control.wait(
            |state| state.readers.len() >= expected,
            "the configured chapter window",
        );
        assert_eq!(
            fixture.control.state.lock().unwrap().readers.len(),
            expected
        );
        assert_eq!(
            jobs.download_fetch_capacity_for_tests(),
            MAX_DOWNLOAD_CONCURRENCY as usize - expected
        );
        fixture.control.release_all();
        let first = receive_checkpoint(&receiver);
        fixture.control.wait(
            |state| state.completed.len() >= expected,
            "completed speculative chapters",
        );
        // The entire admitted window remains owned until its first durable ACK.
        assert_eq!(
            fixture.control.state.lock().unwrap().readers.len(),
            expected
        );
        let first_index = downloaded_index(&first);
        let mut saved = BTreeSet::from([first_index]);
        acknowledge(first, first_index, true);
        for _ in 1..count {
            let event = receive_checkpoint(&receiver);
            let index = downloaded_index(&event);
            assert!(saved.insert(index), "A completed chapter was emitted twice");
            acknowledge(event, index, true);
        }
        assert_eq!(saved, (0..count).collect());
        assert!(matches!(
            receive_checkpoint(&receiver).result,
            JobResult::DownloadFinished { .. }
        ));
        assert_resources_released(&jobs);
        assert!(fixture.control.state.lock().unwrap().maximum_active <= expected);
    }
}

#[test]
fn a_blocked_first_chapter_does_not_delay_later_checkpoints_or_window_refill() {
    let fixture = Fixture::new(
        5,
        FixtureOptions {
            blocked: BTreeSet::from([0]),
            ..Default::default()
        },
    );
    let runtime = runtime();
    let (jobs, receiver) = jobs(&runtime, &fixture);
    let (task, plan) = task("blocked-first", 5);
    assert!(jobs.download_with_concurrency(9, task, plan, 3).is_none());
    let mut saved = BTreeSet::new();
    for _ in 1..5 {
        let event = receive_checkpoint(&receiver);
        let index = downloaded_index(&event);
        assert_ne!(index, 0, "The blocked Reader cannot produce complete text");
        assert!(saved.insert(index));
        acknowledge(event, index, true);
    }
    assert_eq!(saved, BTreeSet::from([1, 2, 3, 4]));
    assert_eq!(jobs.active_download().as_deref(), Some("blocked-first"));
    {
        let state = fixture.control.state.lock().unwrap();
        assert_eq!(state.readers.len(), 5);
        assert!(!state.completed.contains(&0));
        assert!(state.maximum_active <= 3);
    }
    fixture.control.release(0);
    acknowledge(receive_checkpoint(&receiver), 0, true);
    assert!(matches!(
        receive_checkpoint(&receiver).result,
        JobResult::DownloadFinished { .. }
    ));
    assert_resources_released(&jobs);
}

#[test]
fn chapter_rejection_and_preview_do_not_discard_successful_peers() {
    for preview in [false, true] {
        let fixture = Fixture::new(
            4,
            FixtureOptions {
                blocked: BTreeSet::from([0]),
                failed: if preview {
                    BTreeSet::new()
                } else {
                    BTreeSet::from([1])
                },
                preview: preview.then_some(1),
                ..Default::default()
            },
        );
        let runtime = runtime();
        let (jobs, receiver) = jobs(&runtime, &fixture);
        let (task, plan) = task("independent-failure", 4);
        assert!(jobs.download_with_concurrency(11, task, plan, 4).is_none());
        let mut saved = BTreeSet::new();
        let mut failure = false;
        let mut limited = false;
        while saved.len() < 2 || !failure {
            let event = receive_checkpoint(&receiver);
            match &event.result {
                JobResult::Downloaded { chapter, .. } => {
                    let index = chapter.index as usize;
                    assert_ne!(index, 0);
                    if index == 1 {
                        assert!(preview && !limited);
                        limited = true;
                    } else {
                        assert!(saved.insert(index));
                    }
                    acknowledge(event, index, index != 1);
                }
                JobResult::DownloadChapterFailed { .. } => {
                    assert!(!failure);
                    failure = true;
                    let error = acknowledge_failure(event, 1, DownloadChapterRetry::Blocked);
                    assert!(
                        matches!(error, SourceError::UnlockRequired) && preview
                            || matches!(error, SourceError::Http(400)) && !preview
                    );
                }
                other => panic!("Other runnable chapters were stopped: {other:?}"),
            }
        }
        assert_eq!(saved, BTreeSet::from([2, 3]));
        assert_eq!(limited, preview);
        assert!(jobs.active_download().is_some());
        fixture.control.release(0);
        acknowledge(receive_checkpoint(&receiver), 0, true);
        let terminal = receive_checkpoint(&receiver);
        assert!(matches!(
            &terminal.result,
            JobResult::DownloadIncomplete {
                completed: 3,
                blocked: 1,
                total: 4,
                ..
            }
        ));
        drop(terminal);
        assert_resources_released(&jobs);
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn a_cooling_chapter_releases_its_slot_and_all_payload_resources() {
    let fixture = Fixture::new(
        6,
        FixtureOptions {
            rejected_attempts: BTreeMap::from([(0, 1)]),
            ..Default::default()
        },
    );
    let runtime = runtime();
    let (jobs, receiver) = jobs(&runtime, &fixture);
    let (task, plan) = task("cooling-chapter", 6);
    assert!(jobs.download_with_concurrency(12, task, plan, 2).is_none());
    let mut saved = BTreeSet::new();
    let mut waiting = false;
    while saved.len() < 5 || !waiting {
        let event = receive_checkpoint(&receiver);
        match &event.result {
            JobResult::Downloaded { chapter, .. } => {
                let index = chapter.index as usize;
                assert_ne!(index, 0);
                assert!(saved.insert(index));
                acknowledge(event, index, true);
            }
            JobResult::DownloadChapterFailed { .. } => {
                assert!(
                    !waiting,
                    "The chapter retried before its persisted deadline"
                );
                waiting = true;
                let error = acknowledge_failure(
                    event,
                    0,
                    DownloadChapterRetry::RetryAt(utc_seconds() + 120),
                );
                assert!(matches!(error, SourceError::Api(code) if code == "-10102"));
            }
            other => panic!("A chapter cooldown stopped unrelated work: {other:?}"),
        }
    }
    assert_eq!(saved, BTreeSet::from([1, 2, 3, 4, 5]));
    assert_resources_released(&jobs);
    assert_eq!(jobs.active_download().as_deref(), Some("cooling-chapter"));
    {
        let state = fixture.control.state.lock().unwrap();
        assert_eq!(state.readers.len(), 6);
        assert_eq!(state.readers.iter().filter(|&&index| index == 0).count(), 1);
        assert!(state.maximum_active <= 2);
    }
    for event in receiver.try_iter() {
        assert!(matches!(event.result, JobResult::DownloadProgress { .. }));
    }
    assert!(jobs.cancel_download("cooling-chapter").is_some());
    eventually(|| jobs.active_download().is_none());
    assert_resources_released(&jobs);
    assert!(receiver.try_recv().is_err());
}

#[test]
fn live_manual_retry_preserves_slow_completed_and_cooling_chapters() {
    const REQUEST: u64 = 30;
    const CHAPTERS: usize = 4;
    let fixture = Fixture::new(
        CHAPTERS,
        FixtureOptions {
            blocked: BTreeSet::from([2]),
            rejected_attempts: BTreeMap::from([(0, 1)]),
            ..Default::default()
        },
    );
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("live-manual-retry.sqlite3")).unwrap();
    let mut book = Book::new(BOOK, "Live manual retry fixture");
    book.source = "weread".into();
    book.total_chapters = CHAPTERS as u32;
    store.upsert_book(&book).unwrap();
    for chapter in headers(CHAPTERS) {
        store.upsert_chapter(&chapter).unwrap();
    }
    let (mut task, _) = task("live-manual-retry", CHAPTERS);
    task.updated_at = utc_seconds() - 1_000;
    store.save_task(&task).unwrap();
    store
        .prepare_download_chapter_states(&task, task.updated_at + 1)
        .unwrap();
    task = store.task(&task.id).unwrap().unwrap();

    // Three durable historical attempts make the real HTTP rejection below
    // exhaust this chapter, without fabricating a blocked task or cursor.
    for _ in 0..3 {
        let retry = store
            .record_download_chapter_failure(
                &task,
                &chapter_id(0),
                0,
                0,
                true,
                "source.api.-10102",
                task.updated_at + 1,
            )
            .unwrap();
        task = store.task(&task.id).unwrap().unwrap();
        let DownloadChapterRetry::RetryAt(deadline) = retry else {
            panic!("The historical attempt exhausted the chapter too early");
        };
        store
            .start_download_chapter_attempt(&task, &chapter_id(0), 0, 0, deadline)
            .unwrap();
        task = store.task(&task.id).unwrap().unwrap();
    }
    let future = store
        .record_download_chapter_failure(
            &task,
            &chapter_id(3),
            3,
            3,
            true,
            "source.api.-10102",
            utc_seconds(),
        )
        .unwrap();
    assert!(matches!(future, DownloadChapterRetry::RetryAt(deadline) if deadline > utc_seconds()));
    task = store.task(&task.id).unwrap().unwrap();
    let states = store
        .prepare_download_chapter_states(&task, task.updated_at + 1)
        .unwrap();
    task = store.task(&task.id).unwrap().unwrap();
    assert_eq!(states[0].status, DownloadChapterStatus::Pending);
    assert_eq!(states[0].retry_count, 3);
    assert_eq!(states[3].status, DownloadChapterStatus::RetryWait);
    let future_state = states[3].clone();
    let plan = DownloadPlan::from_headers_with_states(
        &task,
        &store.chapter_headers(BOOK).unwrap(),
        &states,
    )
    .unwrap();
    let (jobs, receiver) = jobs(&runtime, &fixture);
    assert!(
        jobs.download_with_concurrency(REQUEST, task.clone(), plan, 3)
            .is_none()
    );
    fixture.control.wait(
        |state| state.readers.contains(&2),
        "the slow peer entering its original request",
    );

    let mut completed = false;
    let mut blocked = false;
    while !completed || !blocked {
        let event = receive(&receiver);
        assert_eq!(event.request, REQUEST);
        match &event.result {
            JobResult::Downloaded { chapter, .. } => {
                assert_eq!(chapter.index, 1);
                assert!(!completed);
                assert_eq!(commit_full_chapter(event, &store, &mut task), 1);
                completed = true;
            }
            JobResult::DownloadChapterFailed { .. } => {
                assert!(!blocked);
                let JobEvent {
                    result, credits, ..
                } = event;
                let JobResult::DownloadChapterFailed {
                    task_id,
                    chapter_id: uid,
                    chapter_index,
                    ordinal,
                    error,
                    checkpoint,
                } = result
                else {
                    unreachable!()
                };
                assert_eq!(task_id, task.id);
                assert_eq!((chapter_index, ordinal), (0, 0));
                assert_eq!(uid, chapter_id(0));
                assert!(matches!(&error, SourceError::Api(code) if code == "-10102"));
                assert!(credits.is_some());
                let retry = store
                    .record_download_chapter_failure(
                        &task,
                        &uid,
                        chapter_index,
                        ordinal,
                        true,
                        &error.to_string(),
                        task.updated_at + 1,
                    )
                    .unwrap();
                assert_eq!(retry, DownloadChapterRetry::Blocked);
                task = store.task(&task.id).unwrap().unwrap();
                checkpoint.send(Ok(retry)).unwrap();
                drop((task_id, uid, error, credits));
                blocked = true;
            }
            JobResult::DownloadProgress { .. } => {}
            other => panic!("An unrelated chapter stopped before manual retry: {other:?}"),
        }
    }
    let before = store.download_chapter_states(&task.id).unwrap();
    assert_eq!(
        (before[0].status, before[0].retry_count),
        (DownloadChapterStatus::Blocked, 4)
    );
    assert_eq!(before[1].status, DownloadChapterStatus::Complete);
    assert_eq!(before[2].status, DownloadChapterStatus::Pending);
    assert_eq!(before[3], future_state);
    let saved_chapter = store.chapter(BOOK, 1).unwrap().unwrap();
    assert!(!jobs.retry_download_chapters(REQUEST - 1, &task.id, &[0]));
    assert!(!jobs.retry_download_chapters(REQUEST, "obsolete-task", &[0]));
    assert!(!jobs.retry_download_chapters(REQUEST, &task.id, &[CHAPTERS]));

    let (progress, ordinals) = store
        .resume_blocked_download_chapters(&task, task.updated_at + 1)
        .unwrap();
    assert_eq!(ordinals, [0]);
    assert_eq!(
        (
            progress.completed_count,
            progress.retry_wait_count,
            progress.blocked_count
        ),
        (1, 1, 0)
    );
    task = store.task(&task.id).unwrap().unwrap();
    let reset = store.download_chapter_states(&task.id).unwrap();
    assert_eq!(
        (reset[0].status, reset[0].retry_count),
        (DownloadChapterStatus::Pending, 0)
    );
    assert_eq!(reset[0].retry_not_before, 0);
    assert!(reset[0].error.is_none());
    assert_eq!(&reset[1..], &before[1..]);
    assert!(jobs.retry_download_chapters(REQUEST, &task.id, &ordinals));
    // A caller cannot turn complete, admitted or future-waiting chapters into
    // extra attempts, even if it includes their ordinals in a later signal.
    assert!(jobs.retry_download_chapters(REQUEST, &task.id, &[1, 2, 3]));

    let mut started = false;
    loop {
        let event = receive(&receiver);
        assert_eq!(
            event.request, REQUEST,
            "Manual retry replaced the active run"
        );
        match &event.result {
            JobResult::DownloadChapterStarted { .. } => {
                assert!(!started);
                let JobEvent {
                    result, credits, ..
                } = event;
                let JobResult::DownloadChapterStarted {
                    task_id,
                    chapter_id: uid,
                    chapter_index,
                    ordinal,
                    checkpoint,
                } = result
                else {
                    unreachable!()
                };
                assert_eq!(task_id, task.id);
                assert_eq!(uid, chapter_id(0));
                assert_eq!((chapter_index, ordinal), (0, 0));
                assert!(credits.is_some());
                assert_eq!(
                    fixture
                        .control
                        .state
                        .lock()
                        .unwrap()
                        .readers
                        .iter()
                        .filter(|&&index| index == 0)
                        .count(),
                    1,
                    "Retry transport started before its durable admission ACK"
                );
                store
                    .start_download_chapter_attempt(
                        &task,
                        &uid,
                        chapter_index,
                        ordinal,
                        task.updated_at + 1,
                    )
                    .unwrap();
                task = store.task(&task.id).unwrap().unwrap();
                checkpoint.send(Ok(())).unwrap();
                drop((task_id, uid, credits));
                started = true;
            }
            JobResult::Downloaded { chapter, .. } => {
                assert!(started);
                assert_eq!(chapter.index, 0);
                assert_eq!(commit_full_chapter(event, &store, &mut task), 0);
                break;
            }
            JobResult::DownloadProgress { .. } => {}
            other => {
                panic!("Manual retry interrupted a peer or ignored the blocked chapter: {other:?}")
            }
        }
    }
    assert_eq!(jobs.active_download().as_deref(), Some(task.id.as_str()));
    assert_eq!(store.chapter(BOOK, 1).unwrap().unwrap(), saved_chapter);
    assert_eq!(
        store.download_chapter_states(&task.id).unwrap()[3],
        future_state
    );
    {
        let state = fixture.control.state.lock().unwrap();
        assert!(
            !state.completed.contains(&2),
            "The slow peer was cancelled or replaced"
        );
        assert_eq!(state.readers.iter().filter(|&&index| index == 0).count(), 2);
        assert_eq!(state.readers.iter().filter(|&&index| index == 1).count(), 1);
        assert_eq!(state.readers.iter().filter(|&&index| index == 2).count(), 1);
        assert!(!state.readers.contains(&3));
    }
    fixture.control.release(2);
    loop {
        let event = receive(&receiver);
        assert_eq!(event.request, REQUEST);
        if matches!(&event.result, JobResult::DownloadProgress { .. }) {
            continue;
        }
        assert_eq!(downloaded_index(&event), 2);
        assert_eq!(commit_full_chapter(event, &store, &mut task), 2);
        break;
    }
    assert_resources_released(&jobs);
    assert_eq!(task.completed_count, 3);
    assert_eq!(
        store.download_chapter_states(&task.id).unwrap()[3],
        future_state
    );
    assert_eq!(fixture.control.state.lock().unwrap().readers.len(), 4);
    for event in receiver.try_iter() {
        assert!(matches!(event.result, JobResult::DownloadProgress { .. }));
    }
    assert!(jobs.cancel_download(&task.id).is_some());
    assert!(!jobs.retry_download_chapters(REQUEST, &task.id, &[3]));
    assert_resources_released(&jobs);
}

#[test]
fn a_retry_reads_only_its_failed_chapter() {
    let fixture = Fixture::new(
        4,
        FixtureOptions {
            rejected_attempts: BTreeMap::from([(0, 1)]),
            ..Default::default()
        },
    );
    let runtime = runtime();
    let (jobs, receiver) = jobs(&runtime, &fixture);
    let (task, plan) = task("isolated-retry", 4);
    assert!(jobs.download_with_concurrency(14, task, plan, 2).is_none());
    let mut saved = BTreeSet::new();
    let mut failures = 0;
    loop {
        let event = receive_checkpoint(&receiver);
        match &event.result {
            JobResult::Downloaded { chapter, .. } => {
                let index = chapter.index as usize;
                assert!(saved.insert(index), "An acknowledged chapter was retried");
                acknowledge(event, index, true);
            }
            JobResult::DownloadChapterFailed { .. } => {
                failures += 1;
                assert_eq!(failures, 1);
                let error =
                    acknowledge_failure(event, 0, DownloadChapterRetry::RetryAt(utc_seconds() + 1));
                assert!(matches!(error, SourceError::Api(code) if code == "-10102"));
            }
            JobResult::DownloadFinished { .. } => break,
            other => panic!("An isolated retry did not settle: {other:?}"),
        }
    }
    assert_eq!(failures, 1);
    assert_eq!(saved, BTreeSet::from([0, 1, 2, 3]));
    assert_resources_released(&jobs);
    let state = fixture.control.state.lock().unwrap();
    assert_eq!(state.readers.iter().filter(|&&index| index == 0).count(), 2);
    for index in 1..4 {
        assert_eq!(
            state.readers.iter().filter(|&&read| read == index).count(),
            1
        );
    }
}

#[test]
fn exhausted_chapters_finish_with_a_bounded_incomplete_summary() {
    let fixture = Fixture::new(
        3,
        FixtureOptions {
            rejected_attempts: (0..3).map(|index| (index, 4)).collect(),
            ..Default::default()
        },
    );
    let runtime = runtime();
    let (jobs, receiver) = jobs(&runtime, &fixture);
    let (task, plan) = task("exhausted-chapters", 3);
    assert!(jobs.download_with_concurrency(15, task, plan, 2).is_none());
    let mut failures = [0usize; 3];
    loop {
        let event = receive_checkpoint(&receiver);
        match &event.result {
            JobResult::DownloadChapterFailed { ordinal, .. } => {
                let index = *ordinal;
                failures[index] += 1;
                assert!(failures[index] <= 4, "An exhausted chapter retried forever");
                let disposition = if failures[index] < 4 {
                    DownloadChapterRetry::RetryAt(utc_seconds() + 1)
                } else {
                    DownloadChapterRetry::Blocked
                };
                let error = acknowledge_failure(event, index, disposition);
                assert!(matches!(error, SourceError::Api(code) if code == "-10102"));
            }
            JobResult::DownloadIncomplete {
                completed,
                blocked,
                total,
                ..
            } => {
                assert_eq!((*completed, *blocked, *total), (0, 3, 3));
                break;
            }
            other => panic!("Exhausted chapters produced an invalid result: {other:?}"),
        }
    }
    assert_eq!(failures, [4, 4, 4]);
    assert_resources_released(&jobs);
    assert!(jobs.active_download().is_none());
    assert_eq!(fixture.control.state.lock().unwrap().readers.len(), 12);
    assert!(receiver.try_recv().is_err());
}

#[test]
fn nonretryable_failures_fill_every_slot_then_finish_incomplete() {
    const CHAPTERS: usize = 9;
    let fixture = Fixture::new(
        CHAPTERS,
        FixtureOptions {
            failed: (0..CHAPTERS).collect(),
            ..Default::default()
        },
    );
    let runtime = runtime();
    let (jobs, receiver) = jobs(&runtime, &fixture);
    let (task, plan) = task("all-blocked", CHAPTERS);
    assert!(jobs.download_with_concurrency(16, task, plan, 3).is_none());
    let mut blocked = BTreeSet::new();
    for _ in 0..CHAPTERS {
        let event = receive_checkpoint(&receiver);
        let JobResult::DownloadChapterFailed { ordinal, .. } = &event.result else {
            panic!("A chapter failure stopped the remaining work");
        };
        let index = *ordinal;
        assert!(blocked.insert(index));
        assert!(matches!(
            acknowledge_failure(event, index, DownloadChapterRetry::Blocked),
            SourceError::Http(400)
        ));
    }
    let terminal = receive_checkpoint(&receiver);
    assert!(matches!(
        &terminal.result,
        JobResult::DownloadIncomplete {
            completed: 0,
            blocked: CHAPTERS,
            total: CHAPTERS,
            ..
        }
    ));
    drop(terminal);
    assert_eq!(blocked, (0..CHAPTERS).collect());
    assert_resources_released(&jobs);
    assert_eq!(
        fixture.control.state.lock().unwrap().readers.len(),
        CHAPTERS
    );
    assert!(receiver.try_recv().is_err());
}

#[test]
fn out_of_order_durable_success_survives_restart_without_refetching_saved_chapters() {
    const CHAPTERS: usize = 4;
    let fixture = Fixture::new(
        CHAPTERS,
        FixtureOptions {
            blocked: BTreeSet::from([0]),
            ..Default::default()
        },
    );
    let runtime = runtime();
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("independent-chapters.sqlite3");
    let store = Store::open(&database).unwrap();
    let mut book = Book::new(BOOK, "Independent chapter fixture");
    book.source = "weread".into();
    book.total_chapters = CHAPTERS as u32;
    store.upsert_book(&book).unwrap();
    for chapter in headers(CHAPTERS) {
        store.upsert_chapter(&chapter).unwrap();
    }
    let (mut task, _) = task("durable-out-of-order", CHAPTERS);
    task.updated_at = 100;
    store.save_task(&task).unwrap();
    let states = store.prepare_download_chapter_states(&task, 101).unwrap();
    task = store.task(&task.id).unwrap().unwrap();
    let plan = DownloadPlan::from_headers_with_states(
        &task,
        &store.chapter_headers(BOOK).unwrap(),
        &states,
    )
    .unwrap();
    let (first_jobs, first_receiver) = jobs(&runtime, &fixture);
    first_jobs.download_with_concurrency(19, task.clone(), plan, 3);
    let mut saved = BTreeSet::new();
    for _ in 1..CHAPTERS {
        let event = receive_checkpoint(&first_receiver);
        assert_ne!(downloaded_index(&event), 0);
        assert!(saved.insert(commit_full_chapter(event, &store, &mut task)));
    }
    assert_eq!(saved, BTreeSet::from([1, 2, 3]));
    assert_eq!(task.next_index, 0);
    assert_eq!(task.completed_count, 3);
    assert!(first_jobs.cancel_download(&task.id).is_some());
    assert_resources_released(&first_jobs);
    drop((first_jobs, first_receiver, store));

    let store = Store::open(&database).unwrap();
    task = store.task("durable-out-of-order").unwrap().unwrap();
    assert_eq!(task.status, DownloadStatus::Paused);
    assert_eq!((task.next_index, task.completed_count), (0, 3));
    task.status = DownloadStatus::Running;
    task.updated_at += 1;
    store.save_task(&task).unwrap();
    let states = store
        .prepare_download_chapter_states(&task, task.updated_at + 1)
        .unwrap();
    task = store.task(&task.id).unwrap().unwrap();
    assert_eq!(
        states
            .iter()
            .filter(|state| state.status == DownloadChapterStatus::Complete)
            .map(|state| state.chapter_index)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([1, 2, 3])
    );
    let plan = DownloadPlan::from_headers_with_states(
        &task,
        &store.chapter_headers(BOOK).unwrap(),
        &states,
    )
    .unwrap();
    let (resumed_jobs, resumed_receiver) = jobs(&runtime, &fixture);
    resumed_jobs.download_with_concurrency(20, task.clone(), plan, 3);
    fixture.control.wait(
        |state| state.readers.iter().filter(|&&index| index == 0).count() == 2,
        "only the unfinished chapter being resumed",
    );
    fixture.control.release(0);
    let resumed = receive_checkpoint(&resumed_receiver);
    assert_eq!(downloaded_index(&resumed), 0);
    assert_eq!(commit_full_chapter(resumed, &store, &mut task), 0);
    assert!(matches!(
        receive_checkpoint(&resumed_receiver).result,
        JobResult::DownloadFinished { .. }
    ));
    assert_eq!(
        (task.next_index, task.completed_count),
        (CHAPTERS, CHAPTERS)
    );
    assert_resources_released(&resumed_jobs);
    let state = fixture.control.state.lock().unwrap();
    assert_eq!(state.readers.len(), CHAPTERS + 1);
    for index in 1..CHAPTERS {
        assert_eq!(
            state.readers.iter().filter(|&&read| read == index).count(),
            1
        );
    }
    for chapter in store.chapters(BOOK).unwrap() {
        assert!(chapter.cached && chapter.body.is_some());
        assert_eq!(chapter.content_status, ContentStatus::Full);
        assert_eq!(chapter.content_account_key, task.account_key);
    }
}

#[test]
fn a_rejected_durable_checkpoint_closes_the_commit_lane() {
    let fixture = Fixture::new(8, FixtureOptions::default());
    let runtime = runtime();
    let (jobs, receiver) = jobs(&runtime, &fixture);
    let (task, plan) = task("rejected-checkpoint", 8);
    jobs.download(13, task, plan);
    let event = receive_checkpoint(&receiver);
    fixture.control.wait(
        |state| state.completed.len() == 8,
        "all fetched chapters waiting for the first checkpoint",
    );
    let JobEvent {
        result, credits, ..
    } = event;
    let JobResult::Downloaded {
        checkpoint,
        chapter,
        html,
        metadata,
        images,
        ..
    } = result
    else {
        panic!("Expected the first chapter checkpoint");
    };
    checkpoint.send(Err(SourceError::CheckpointFailed)).unwrap();
    drop((chapter, html, metadata, images, credits));
    let terminal = receive_checkpoint(&receiver);
    assert!(matches!(
        terminal.result,
        JobResult::DownloadFailed {
            error: SourceError::CheckpointFailed,
            ..
        }
    ));
    drop(terminal);
    assert_resources_released(&jobs);
    assert!(receiver.try_recv().is_err());
    assert_eq!(jobs.download_resource_capacity_for_tests(), (1, 1));
}

#[test]
fn a_rejected_failure_checkpoint_cannot_retry_or_continue_publishing() {
    let fixture = Fixture::new(
        3,
        FixtureOptions {
            blocked: BTreeSet::from([1, 2]),
            failed: BTreeSet::from([0]),
            ..Default::default()
        },
    );
    let runtime = runtime();
    let (jobs, receiver) = jobs(&runtime, &fixture);
    let (task, plan) = task("rejected-failure-checkpoint", 3);
    assert!(jobs.download_with_concurrency(21, task, plan, 3).is_none());
    let JobEvent {
        result, credits, ..
    } = receive_checkpoint(&receiver);
    let JobResult::DownloadChapterFailed {
        checkpoint,
        chapter_index: 0,
        ..
    } = result
    else {
        panic!("Expected the independent failure checkpoint");
    };
    assert!(credits.is_some());
    checkpoint.send(Err(SourceError::CheckpointFailed)).unwrap();
    drop(credits);
    let terminal = receive_checkpoint(&receiver);
    assert!(matches!(
        &terminal.result,
        JobResult::DownloadFailed {
            error: SourceError::CheckpointFailed,
            ..
        }
    ));
    drop(terminal);
    assert_resources_released(&jobs);
    assert!(jobs.active_download().is_none());
    assert!(receiver.try_recv().is_err());
    assert_eq!(fixture.control.state.lock().unwrap().shard_requests, 0);
    fixture.control.release_all();
}

#[test]
fn cancellation_account_replacement_and_receiver_close_release_inflight_chapters() {
    for action in ["cancel", "replace", "receiver"] {
        let fixture = Fixture::new(
            8,
            FixtureOptions {
                blocked: (0..8).collect(),
                ..Default::default()
            },
        );
        let runtime = runtime();
        let (jobs, receiver) = jobs(&runtime, &fixture);
        let (task, plan) = task("cancel-inflight", 8);
        jobs.download(17, task, plan);
        fixture.control.wait(
            |state| state.readers.len() == 8,
            "all admitted Reader requests",
        );
        assert_eq!(jobs.download_fetch_capacity_for_tests(), 8);
        match action {
            "cancel" => {
                assert!(jobs.cancel_download("cancel-inflight").is_some());
            }
            "replace" => {
                jobs.replace_session(AuthSession {
                    api_key: "synthetic-new-account".into(),
                    ..Default::default()
                })
                .unwrap();
            }
            "receiver" => {
                drop(receiver);
            }
            _ => unreachable!(),
        }
        if action != "receiver" {
            eventually(|| {
                jobs.download_fetch_capacity_for_tests() == MAX_DOWNLOAD_CONCURRENCY as usize
            });
            assert_eq!(fixture.control.state.lock().unwrap().shard_requests, 0);
        }
        fixture.control.release_all();
        eventually(|| {
            jobs.active_download().is_none()
                && jobs.download_fetch_capacity_for_tests() == MAX_DOWNLOAD_CONCURRENCY as usize
                && jobs.download_resource_capacity_for_tests() == (1, 1)
        });
        assert_resources_released(&jobs);
    }
}

#[test]
fn an_old_owned_packet_keeps_its_fetch_lease_after_task_replacement() {
    let fixture = Fixture::new(3, FixtureOptions::default());
    let runtime = runtime();
    let (jobs, receiver) = jobs(&runtime, &fixture);
    let (first, plan) = task("old-packet", 3);
    jobs.download_with_concurrency(23, first, plan, 3);
    let held = receive_checkpoint(&receiver);
    assert!(matches!(&held.result, JobResult::Downloaded { .. }));
    jobs.cancel_download("old-packet").unwrap();
    eventually(|| {
        jobs.download_fetch_capacity_for_tests() == MAX_DOWNLOAD_CONCURRENCY as usize - 1
    });
    assert_eq!(jobs.download_resource_capacity_for_tests(), (0, 1));
    let reads_before = fixture.control.state.lock().unwrap().readers.len();
    let (next, plan) = task("replacement-packet", 3);
    jobs.download_with_concurrency(24, next, plan, 3);
    fixture.control.wait(
        |state| state.readers.len() == reads_before + 3,
        "the replacement's bounded text window",
    );
    assert_eq!(
        fixture.control.state.lock().unwrap().readers.len(),
        reads_before + 3
    );
    assert_eq!(
        jobs.download_fetch_capacity_for_tests(),
        MAX_DOWNLOAD_CONCURRENCY as usize - 4
    );
    assert_eq!(jobs.download_resource_capacity_for_tests().0, 0);
    assert!(
        receiver.try_recv().is_err(),
        "A replacement bypassed the owned packet"
    );
    drop(held);
    let mut saved = BTreeSet::new();
    for _ in 0..3 {
        let event = receive_checkpoint(&receiver);
        let index = downloaded_index(&event);
        assert!(saved.insert(index));
        acknowledge(event, index, true);
    }
    assert_eq!(saved, BTreeSet::from([0, 1, 2]));
    assert!(matches!(
        receive_checkpoint(&receiver).result,
        JobResult::DownloadFinished { .. }
    ));
    assert_resources_released(&jobs);
}

#[test]
fn two_oversized_chapters_wait_exclusively_without_discarding_ordinary_peers() {
    const CHAPTERS: usize = 4;
    let fixture = Fixture::new(
        CHAPTERS,
        FixtureOptions {
            blocked: (0..CHAPTERS).collect(),
            large: BTreeSet::from([0, 1]),
            ..Default::default()
        },
    );
    let runtime = runtime();
    let (jobs, receiver) = jobs(&runtime, &fixture);
    let (task, plan) = task("two-large-chapters", CHAPTERS);
    assert!(jobs.download_with_concurrency(28, task, plan, 4).is_none());
    fixture.control.wait(
        |state| state.readers.len() == CHAPTERS,
        "both oversized and ordinary initial reads",
    );
    fixture.control.release(0);
    fixture.control.release(1);
    eventually(|| jobs.download_fetch_capacity_for_tests() == 0);
    assert_eq!(jobs.download_resource_capacity_for_tests(), (1, 1));
    assert!(receiver.try_recv().is_err());

    // Both upgrades must surrender their initial credits. Otherwise neither
    // exclusive waiter can proceed after these ordinary peers are committed.
    fixture.control.release(2);
    fixture.control.release(3);
    let mut ordinary = BTreeSet::new();
    for _ in 0..2 {
        let event = receive_checkpoint(&receiver);
        let index = downloaded_index(&event);
        assert!(matches!(index, 2 | 3));
        assert!(ordinary.insert(index));
        acknowledge(event, index, true);
    }
    assert_eq!(ordinary, BTreeSet::from([2, 3]));

    let first = receive_checkpoint(&receiver);
    let first_index = downloaded_index(&first);
    assert!(first_index < 2);
    let JobResult::Downloaded { chapter, .. } = &first.result else {
        unreachable!();
    };
    assert!(chapter.body.as_ref().unwrap().len() > 900 * 1024);
    assert_eq!(jobs.download_fetch_capacity_for_tests(), 0);
    {
        let state = fixture.control.state.lock().unwrap();
        assert_eq!(state.readers.len(), CHAPTERS + 1);
        assert_eq!(
            state
                .readers
                .iter()
                .filter(|&&index| index == first_index)
                .count(),
            2
        );
        assert_eq!(
            state
                .readers
                .iter()
                .filter(|&&index| index == 1 - first_index)
                .count(),
            1,
            "The second full-body read overlapped the first owned payload"
        );
    }
    acknowledge(first, first_index, true);

    let second = receive_checkpoint(&receiver);
    let second_index = downloaded_index(&second);
    assert_eq!(second_index, 1 - first_index);
    let JobResult::Downloaded { chapter, .. } = &second.result else {
        unreachable!();
    };
    assert!(chapter.body.as_ref().unwrap().len() > 900 * 1024);
    assert_eq!(jobs.download_fetch_capacity_for_tests(), 0);
    acknowledge(second, second_index, true);
    assert!(matches!(
        receive_checkpoint(&receiver).result,
        JobResult::DownloadFinished { .. }
    ));
    assert_resources_released(&jobs);
    let state = fixture.control.state.lock().unwrap();
    assert_eq!(state.readers.len(), CHAPTERS + 2);
    for index in 0..CHAPTERS {
        assert_eq!(
            state.readers.iter().filter(|&&read| read == index).count(),
            if index < 2 { 2 } else { 1 },
            "Only an oversized chapter may need its one full-budget retry"
        );
    }
}

#[test]
fn oversized_prefetch_preserves_completed_peers_and_restores_the_parallel_window() {
    const WINDOW: usize = 8;
    const CHAPTERS: usize = WINDOW * 2 + 1;
    let fixture = Fixture::new(
        CHAPTERS,
        FixtureOptions {
            blocked: (0..CHAPTERS).collect(),
            large: BTreeSet::from([0]),
            ..Default::default()
        },
    );
    let runtime = runtime();
    let (jobs, receiver) = jobs(&runtime, &fixture);
    let (task, plan) = task("large-chapter", CHAPTERS);
    jobs.download_with_concurrency(29, task, plan, WINDOW as u32);
    fixture.control.wait(
        |state| state.readers.len() == WINDOW,
        "the initial prefetch window",
    );
    fixture.control.release(0);
    // The full-body waiter queues for the entire fair semaphore allowance.
    // It may not own the packet/preparation lane while its peers hold text.
    eventually(|| jobs.download_fetch_capacity_for_tests() == 0);
    assert_eq!(jobs.download_resource_capacity_for_tests(), (1, 1));
    assert!(receiver.try_recv().is_err());
    for index in 1..WINDOW {
        fixture.control.release(index);
    }
    let mut saved = BTreeSet::new();
    for _ in 1..WINDOW {
        let event = receive_checkpoint(&receiver);
        let index = downloaded_index(&event);
        assert!((1..WINDOW).contains(&index));
        assert!(saved.insert(index));
        acknowledge(event, index, true);
    }
    assert_eq!(saved, (1..WINDOW).collect());
    let large = receive_checkpoint(&receiver);
    let JobResult::Downloaded { chapter, .. } = &large.result else {
        panic!("The large chapter was not retried");
    };
    assert_eq!(chapter.index, 0);
    assert!(chapter.body.as_ref().unwrap().len() > 900 * 1024);
    assert_eq!(
        fixture
            .control
            .state
            .lock()
            .unwrap()
            .readers
            .iter()
            .filter(|&&index| index == 0)
            .count(),
        2
    );
    assert_eq!(
        jobs.download_fetch_capacity_for_tests(),
        0,
        "The full-budget body must retain exclusive admission through its ACK"
    );
    {
        let state = fixture.control.state.lock().unwrap();
        for index in 1..WINDOW {
            assert_eq!(
                state.readers.iter().filter(|&&read| read == index).count(),
                1,
                "A completed peer was discarded and fetched again"
            );
        }
        assert_eq!(state.readers.len(), WINDOW + 1);
    }
    acknowledge(large, 0, true);
    fixture.control.wait(
        |state| {
            (WINDOW..WINDOW * 2)
                .all(|index| state.readers.iter().filter(|&&read| read == index).count() == 1)
        },
        "a complete parallel window after the isolated large-chapter retry",
    );
    {
        let state = fixture.control.state.lock().unwrap();
        assert_eq!(state.readers.len(), WINDOW * 2 + 1);
        assert!(!state.readers.contains(&(WINDOW * 2)));
    }
    assert_eq!(
        jobs.download_fetch_capacity_for_tests(),
        MAX_DOWNLOAD_CONCURRENCY as usize - WINDOW,
        "The next ordinary chapters did not regain the configured fetch window"
    );
    fixture.control.release_all();
    for _ in WINDOW..CHAPTERS {
        let event = receive_checkpoint(&receiver);
        let index = downloaded_index(&event);
        assert!((WINDOW..CHAPTERS).contains(&index));
        assert!(saved.insert(index));
        acknowledge(event, index, true);
    }
    assert_eq!(saved, (1..CHAPTERS).collect());
    assert!(matches!(
        receive_checkpoint(&receiver).result,
        JobResult::DownloadFinished { .. }
    ));
    assert_resources_released(&jobs);
    let state = fixture.control.state.lock().unwrap();
    assert_eq!(state.readers.len(), CHAPTERS + 1);
    for index in 1..CHAPTERS {
        assert_eq!(
            state.readers.iter().filter(|&&read| read == index).count(),
            1
        );
    }
}
