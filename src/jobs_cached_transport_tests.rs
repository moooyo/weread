//! A real proxy proves cached Jobs producers make no first-party HTTP request.

use super::*;
use crate::{
    domain,
    jobs::{DownloadPlan, JobEvent, JobResult, Jobs},
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

fn task(count: u32) -> domain::DownloadTask {
    domain::DownloadTask {
        id: "cached-transport-task".into(),
        book_id: "cached-transport-book".into(),
        account_key: "cached-transport-account".into(),
        chapters: (0..count).collect(),
        status: domain::DownloadStatus::Running,
        include_images: false,
        ..Default::default()
    }
}

fn headers(count: u32, _: bool) -> Vec<domain::Chapter> {
    (0..count)
        .map(|index| domain::Chapter {
            id: format!("uid-{index}"),
            book_id: "cached-transport-book".into(),
            index,
            title: format!("Title {index}"),
            cached: true,
            body: None,
            content_status: domain::ContentStatus::Full,
            content_account_key: "cached-transport-account".into(),
        })
        .collect()
}

struct RejectingProxy {
    address: std::net::SocketAddr,
    stopped: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
    worker: Option<JoinHandle<()>>,
}

impl RejectingProxy {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let stop = stopped.clone();
        let observed = requests.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((stream, _)) => reject_request(stream, &observed),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            address,
            stopped,
            requests,
            worker: Some(worker),
        }
    }
}

fn reject_request(mut stream: TcpStream, requests: &AtomicUsize) {
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut prefix = [0; 1024];
    if stream.read(&mut prefix).unwrap_or(0) > 0 {
        requests.fetch_add(1, Ordering::SeqCst);
    }
    let _ = stream
        .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
}

impl Drop for RejectingProxy {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
}

fn receive(receiver: &Receiver<JobEvent>) -> JobEvent {
    receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("The credited producer did not finish")
}

#[test]
fn a_fully_cached_genuine_task_sends_only_metadata_and_makes_no_http_request() {
    let proxy = RejectingProxy::new();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let _runtime_context = runtime.enter();
    let mut client = WeReadClient::new(AuthSession {
        api_key: "synthetic-cached-api-key".into(),
        user_vid: "synthetic-cached-vid".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "synthetic-cached-vid".into()),
            ("wr_skey".into(), "synthetic-cached-session".into()),
        ]),
        ..Default::default()
    })
    .unwrap();
    client.http = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(format!("http://{}", proxy.address)).unwrap())
        .timeout(Duration::from_secs(1))
        .pool_max_idle_per_host(0)
        .build()
        .unwrap();
    // A real CONNECT request proves this fixture intercepts first-party HTTP.
    assert!(
        runtime
            .block_on(async {
                client
                    .http
                    .get("https://weread.qq.com/web/reader/proxy-fixture")
                    .send()
                    .await
            })
            .is_err()
    );
    assert_eq!(proxy.requests.load(Ordering::SeqCst), 1);
    let (sender, receiver) = mpsc::channel();
    let jobs = Jobs::new(runtime.handle().clone(), sender, Some(client)).unwrap();
    let task = task(3);
    let plan = DownloadPlan::from_headers(&task, &headers(3, true)).unwrap();
    assert!(jobs.download(7, task, plan).is_none());
    let mut checkpoints = 0;
    loop {
        let event = receive(&receiver);
        let JobEvent {
            result, credits, ..
        } = event;
        assert!(credits.is_some());
        match result {
            JobResult::CachedDownloaded {
                chapter_id,
                chapter_index,
                next_index,
                checkpoint,
                ..
            } => {
                assert_eq!(chapter_id, format!("uid-{chapter_index}"));
                assert_eq!(next_index, chapter_index as usize + 1);
                checkpoints += 1;
                checkpoint.send(Ok(())).unwrap();
            }
            JobResult::DownloadProgress { .. } => {}
            JobResult::DownloadFinished { .. } => break,
            other => panic!("A cached task attempted another path: {other:?}"),
        }
        drop(credits);
    }
    assert_eq!(checkpoints, 3);
    assert_eq!(
        proxy.requests.load(Ordering::SeqCst),
        1,
        "Cached work made another HTTP request"
    );
}
