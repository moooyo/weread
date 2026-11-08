//! Real Full chapter HTTP and Jobs producers exercise the checked SVG guard.

use super::review_output_budget_tests::{RecordedRequest, read_request};
use super::*;
use crate::{
    book_assets::BookAssetStore,
    domain::{Annotation, Book, Chapter, ContentStatus, DownloadStatus, DownloadTask},
    jobs::{DownloadPlan, JobEvent, JobResult, Jobs},
    source::images::{
        SvgGuardLimits,
        image_svg_guard_tests::{ObservationScope, mathml},
    },
    store::Store,
};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    net::{SocketAddr, TcpListener},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Instant,
};

const BOOK: &str = "svg-guard-worker-book";
const UID: &str = "svg-guard-worker-chapter";
const VID: &str = "synthetic-svg-guard-vid";
const BODY: &str = "A";

#[path = "jobs_style_budget_tests.rs"]
mod style_budget_worker_tests;

#[path = "jobs_image_budget_tests.rs"]
mod image_budget_worker_tests;

fn auth() -> AuthSession {
    AuthSession {
        api_key: "synthetic-svg-guard-key".into(),
        user_vid: VID.into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), VID.into()),
            ("wr_skey".into(), "synthetic-svg-guard-session".into()),
        ]),
        ..Default::default()
    }
}

fn owner() -> String {
    hex::encode(Sha256::digest(
        format!("weread-download-vid:{VID}").as_bytes(),
    ))
}

fn shard(html: &str) -> String {
    // Invert the recorded permutation. Both tests require a real decoder and
    // Full-access calibration before attributing a result to the SVG guard.
    let mut encoded = base64::engine::general_purpose::STANDARD
        .encode(html)
        .into_bytes();
    let length = encoded.len();
    let n = 4.min(length.div_ceil(10));
    let tail = encoded[length - n..]
        .iter()
        .rev()
        .map(|byte| {
            let mut radix_four = 0u64;
            let mut begun = false;
            for bit in (0..8).rev() {
                let digit = (byte >> bit) & 1;
                if digit != 0 {
                    begun = true;
                }
                if begun {
                    radix_four = radix_four * 4 + u64::from(digit);
                }
            }
            radix_four.to_string()
        })
        .collect::<String>();
    let modulus = length - n - 2;
    let step = modulus.to_string().len();
    let mut positions = Vec::new();
    let mut index = 0;
    while positions.len() < 10 && index + step < tail.len() {
        positions.push(tail[index..index + step].parse::<usize>().unwrap() % modulus);
        positions.push(
            tail[index + 1..(index + step + 1).min(tail.len())]
                .parse::<usize>()
                .unwrap()
                % modulus,
        );
        index += step;
    }
    for pair in positions.chunks_exact(2) {
        for offset in 0..=1 {
            encoded.swap(pair[0] + offset, pair[1] + offset);
        }
    }
    let body = format!("a{}", String::from_utf8(encoded).unwrap());
    format!("{:X}{body}", md5::compute(body.as_bytes()))
}

fn reader_state() -> String {
    format!(
        "<script>window.__INITIAL_STATE__ = {};</script>",
        json!({
            "user":{"vid":VID},
            "reader":{
                "bookId":BOOK,"psvts":"svg-worker-clock","pclts":0,
                "isBookInfoError":false,"isBookForbidden":false,
                "bookInfo":{"bookId":BOOK,"payType":1,"paid":1},
                "chapterInfos":[{"chapterUid":UID,"chapterIdx":0,"wordCount":1,"paid":1,"price":0}],
                "currentChapter":{"chapterUid":UID,"chapterIdx":0,"wordCount":1,"paid":1,"price":0}
            }
        })
    )
}

struct Fixture {
    address: SocketAddr,
    stopped: Arc<AtomicBool>,
    records: Arc<Mutex<Vec<RecordedRequest>>>,
    worker: Option<JoinHandle<()>>,
}

impl Fixture {
    fn new(html: &str) -> Self {
        let encoded = Arc::new(shard(html));
        let reader = Arc::new(reader_state());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let records = Arc::new(Mutex::new(Vec::new()));
        let captured = records.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut children = Vec::new();
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        assert!(children.len() < 24, "Unexpected extra SVG fixture requests");
                        let encoded = encoded.clone();
                        let reader = reader.clone();
                        let records = captured.clone();
                        children.push(thread::spawn(move || {
                            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                            stream.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
                            let request = read_request(&mut stream);
                            let body = match request.path.as_str() {
                                "/web/book/chapterInfos" => serde_json::to_vec(&json!({
                                    "data":[{"bookId":BOOK,"updated":[{
                                        "chapterUid":UID,"chapterIdx":0,"wordCount":1,
                                        "title":"Worker chapter"
                                    }]}]
                                })).unwrap(),
                                "/web/book/chapter/e_0" => encoded.as_bytes().to_vec(),
                                "/web/book/chapter/e_1" | "/web/book/chapter/e_3" => Vec::new(),
                                path if path.starts_with("/web/reader/") => reader.as_bytes().to_vec(),
                                path => panic!("Unexpected SVG fixture path: {path}"),
                            };
                            records.lock().unwrap().push(request);
                            let response = format!(
                                "HTTP/1.1 200 Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            stream.write_all(response.as_bytes()).unwrap();
                            stream.write_all(&body).unwrap();
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "The SVG fixture was not stopped");
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("Synthetic SVG listener failed: {error}"),
                }
            }
            for child in children {
                child.join().unwrap();
            }
        });
        Self {
            address,
            stopped,
            records,
            worker: Some(worker),
        }
    }

    fn client(&self) -> WeReadClient {
        let mut client = WeReadClient::new(auth()).unwrap();
        let origin = format!("http://{}", self.address);
        // Synthetic credentials stay on loopback even if an origin hook stops
        // being used. No remote image URL appears in this MathML fixture.
        client.http = http_client_builder(FIRST_PARTY_IDLE_PER_HOST)
            .no_proxy()
            .resolve("weread.qq.com", self.address)
            .resolve("i.weread.qq.com", self.address)
            .build()
            .unwrap();
        client.resource_http = http_client_builder(0)
            .no_proxy()
            .resolve("image-fixture.invalid", self.address)
            .build()
            .unwrap();
        client.test_web_origin = Some(origin.clone());
        client.test_gateway_url = Some(format!("{origin}/gateway"));
        client.test_resource_origin = Some(origin);
        client
    }

    fn finish(mut self) -> Vec<RecordedRequest> {
        self.stopped.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
        self.records.lock().unwrap().clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let joined = worker.join();
            if !thread::panicking() {
                joined.unwrap();
            }
        }
    }
}

fn state() -> (
    tempfile::TempDir,
    Store,
    BookAssetStore,
    DownloadTask,
    Annotation,
) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("library.sqlite")).unwrap();
    store
        .upsert_book(&Book {
            id: BOOK.into(),
            title: "Original book".into(),
            source: "weread".into(),
            total_chapters: 1,
            ..Default::default()
        })
        .unwrap();
    store
        .upsert_chapter(&Chapter {
            id: UID.into(),
            book_id: BOOK.into(),
            index: 0,
            title: "Original chapter".into(),
            cached: false,
            body: None,
            content_status: ContentStatus::Unknown,
            ..Default::default()
        })
        .unwrap();
    let note = Annotation {
        id: "synthetic-svg-note".into(),
        book_id: BOOK.into(),
        chapter_index: 0,
        start: 0,
        end: 1,
        quote: BODY.into(),
        note: "Preserve the existing note".into(),
        created_at: 1,
    };
    store.save_annotation(&note).unwrap();
    let task = DownloadTask {
        id: "svg-guard-worker-task".into(),
        book_id: BOOK.into(),
        account_key: owner(),
        chapters: vec![0],
        status: DownloadStatus::Running,
        include_images: true,
        next_index: 0,
        updated_at: 17,
        ..Default::default()
    };
    store.save_task(&task).unwrap();
    let assets = BookAssetStore::open(directory.path().join("images"), 128 * 1024).unwrap();
    (directory, store, assets, task, note)
}

fn next_event(runtime: &tokio::runtime::Runtime, receiver: &mpsc::Receiver<JobEvent>) -> JobEvent {
    runtime.block_on(async {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Ok(event) = receiver.try_recv() {
                return event;
            }
            assert!(
                Instant::now() < deadline,
                "The real SVG Jobs producer did not finish"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
}

fn next_terminal(
    runtime: &tokio::runtime::Runtime,
    receiver: &mpsc::Receiver<JobEvent>,
) -> JobEvent {
    loop {
        let event = next_event(runtime, receiver);
        match &event.result {
            JobResult::DownloadProgress { .. } => drop(event),
            JobResult::DownloadFinished { .. } | JobResult::DownloadFailed { .. } => return event,
            other => panic!("Unexpected SVG worker terminal sequence: {other:?}"),
        }
    }
}

fn calibrated_client(
    runtime: &tokio::runtime::Runtime,
    fixture: &Fixture,
    html: &str,
) -> WeReadClient {
    assert_eq!(
        super::super::content::decode_shards(&[&shard(html)]).unwrap(),
        html
    );
    assert_eq!(super::super::content::html_to_text(html), BODY);
    let client = {
        let _context = runtime.enter();
        fixture.client()
    };
    let content = runtime.block_on(client.chapter(BOOK, UID)).unwrap();
    assert_eq!(content.content_status, ContentStatus::Full);
    assert_eq!(content.plain_text, BODY);
    assert_eq!(content.html, html);
    client
}

fn request_contract(records: &[RecordedRequest], attempts: usize) {
    assert_eq!(
        records
            .iter()
            .filter(|request| request.path == "/web/book/chapterInfos")
            .count(),
        attempts
    );
    assert_eq!(
        records
            .iter()
            .filter(|request| request.path.starts_with("/web/reader/"))
            .count(),
        attempts + 1
    );
    assert_eq!(records.len(), 4 + 5 * attempts);
    assert!(records.iter().all(|request| {
        request.path.starts_with("/web/reader/")
            || matches!(
                request.path.as_str(),
                "/web/book/chapterInfos"
                    | "/web/book/chapter/e_0"
                    | "/web/book/chapter/e_1"
                    | "/web/book/chapter/e_3"
            )
    }));
}

#[test]
fn full_mathml_jobs_http_worker_commits_checkpoint_and_releases_svg_guard_credits() {
    let html = mathml(8, 8);
    let fixture = Fixture::new(&html);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = calibrated_client(&runtime, &fixture, &html);
    let before_session = client.session();
    let (_directory, store, assets, mut task, note) = state();
    let (sender, receiver) = mpsc::channel();
    let jobs = {
        let _context = runtime.enter();
        Jobs::new(runtime.handle().clone(), sender, Some(client.clone())).unwrap()
    };
    let scope = ObservationScope::start(SvgGuardLimits::default());
    let plan = DownloadPlan::from_headers(&task, &store.chapter_headers(BOOK).unwrap()).unwrap();
    assert!(jobs.download(71, task.clone(), plan).is_none());
    let packet = next_event(&runtime, &receiver);
    assert!(jobs.accepts(&packet));
    assert_eq!(packet.request, 71);
    assert!(packet.credits.is_some());
    assert_eq!(jobs.download_resource_capacity_for_tests(), (0, 1));
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
            assert_eq!(chapter.content_account_key, task.account_key);
            assert_eq!(chapter.body.as_deref(), Some(BODY));
            assert_eq!(next_index, 1);
            assert_eq!(store.task(&task.id).unwrap().unwrap().next_index, 0);
            store
                .validate_download_chapter(&task, &chapter, next_index)
                .unwrap();
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
                .commit_download_checkpoint(&task, &chapter, next_index, 21)
                .unwrap();
            task.next_index = next_index;
            task.updated_at = 21;
            checkpoint.send(Ok(())).unwrap();
            drop((chapter, html, metadata, images));
        }
        other => panic!("The calibrated Full worker did not emit a fresh checkpoint: {other:?}"),
    }
    drop(credits);
    let terminal = next_terminal(&runtime, &receiver);
    assert!(
        matches!(&terminal.result, JobResult::DownloadFinished { task_id } if task_id == &task.id)
    );
    assert!(terminal.credits.is_some());
    assert_eq!(jobs.download_resource_capacity_for_tests(), (0, 1));
    let mut complete = task.clone();
    complete.status = DownloadStatus::Complete;
    assert!(store.save_task_if_current(&task, &complete).unwrap());
    let stored = store.chapter(BOOK, 0).unwrap().unwrap();
    assert_eq!(stored.body.as_deref(), Some(BODY));
    assert_eq!(stored.content_account_key, owner());
    assert_eq!(stored.content_status, ContentStatus::Full);
    assert_eq!(store.task(&task.id).unwrap().unwrap().next_index, 1);
    assert_eq!(store.annotations(BOOK).unwrap(), vec![note]);
    assert_eq!(assets.usage().unwrap(), 0);
    assert!(assets.chapter(&store, BOOK, UID).unwrap().unwrap().complete);
    assert_eq!(scope.stats().summary_builds, 1);
    assert_eq!(
        scope.stats().usage.element_visits,
        scope.stats().usage.elements
    );
    assert_eq!(scope.stats().usage.svg_images, 0);
    assert_eq!(client.session(), before_session);
    assert_eq!(jobs.session().unwrap(), before_session);
    drop(terminal);
    assert_eq!(jobs.download_resource_capacity_for_tests(), (1, 1));
    request_contract(&fixture.finish(), 1);
}

#[test]
fn full_mathml_jobs_guard_quota_rejects_before_worker_and_preserves_store_state() {
    let html = mathml(8, 8);
    let fixture = Fixture::new(&html);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = calibrated_client(&runtime, &fixture, &html);
    let before_session = client.session();
    let (_directory, store, assets, task, note) = state();
    let before_chapter = store.chapter(BOOK, 0).unwrap().unwrap();
    let before_book = store.book(BOOK).unwrap().unwrap();
    let (sender, receiver) = mpsc::channel();
    let jobs = {
        let _context = runtime.enter();
        Jobs::new(runtime.handle().clone(), sender, Some(client.clone())).unwrap()
    };
    for (request, limits) in [
        (
            81,
            SvgGuardLimits {
                work_bytes: 0,
                ..SvgGuardLimits::default()
            },
        ),
        (
            82,
            SvgGuardLimits {
                metadata_bytes: 0,
                ..SvgGuardLimits::default()
            },
        ),
    ] {
        let scope = ObservationScope::start(limits);
        let plan =
            DownloadPlan::from_headers(&task, &store.chapter_headers(BOOK).unwrap()).unwrap();
        assert!(jobs.download(request, task.clone(), plan).is_none());
        let terminal = next_terminal(&runtime, &receiver);
        assert_eq!(terminal.request, request);
        assert!(jobs.accepts(&terminal));
        assert!(terminal.credits.is_some());
        match &terminal.result {
            JobResult::DownloadFailed {
                task_id,
                error: SourceError::ResponseTooLarge,
            } => {
                assert_eq!(task_id, &task.id);
            }
            other => panic!("The real Full SVG guard producer returned another path: {other:?}"),
        }
        assert!(jobs.active_download().is_none());
        assert_eq!(jobs.download_resource_capacity_for_tests(), (0, 1));
        assert_eq!(scope.stats().summary_builds, 0);
        assert_eq!(scope.stats().usage.element_visits, 0);
        assert_eq!(scope.stats().usage.child_inspections, 0);
        assert_eq!(scope.stats().rejected, 1);
        assert_eq!(store.task(&task.id).unwrap().unwrap(), task);
        assert_eq!(store.chapter(BOOK, 0).unwrap().unwrap(), before_chapter);
        assert_eq!(store.book(BOOK).unwrap().unwrap(), before_book);
        assert_eq!(store.annotations(BOOK).unwrap(), vec![note.clone()]);
        assert_eq!(assets.usage().unwrap(), 0);
        assert!(assets.chapter(&store, BOOK, UID).unwrap().is_none());
        assert_eq!(client.session(), before_session);
        assert_eq!(jobs.session().unwrap(), before_session);
        assert!(receiver.try_recv().is_err());
        drop(terminal);
        assert_eq!(jobs.download_resource_capacity_for_tests(), (1, 1));
    }
    request_contract(&fixture.finish(), 2);
}
