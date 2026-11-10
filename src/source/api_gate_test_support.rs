//! Bounded loopback fixtures for real source requests; no production transport changes.

#[path = "auth_session_budget_report_tests.rs"]
mod auth_session_budget_report_tests;

use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    thread::{self, JoinHandle},
    time::Instant,
};

#[derive(Clone, Debug)]
pub(crate) struct RecordedRequest {
    pub method: String,
    pub target: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl RecordedRequest {
    pub(crate) fn path(&self) -> &str {
        self.target.split('?').next().unwrap()
    }

    pub(crate) fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

pub(crate) struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    pub(crate) fn bytes(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body,
        }
    }

    pub(crate) fn json(value: Value) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: serde_json::to_vec(&value).unwrap(),
        }
    }

    pub(crate) fn text(value: impl Into<String>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: value.into().into_bytes(),
        }
    }

    pub(crate) fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    pub(crate) fn cookie(mut self, cookie: impl Into<String>) -> Self {
        self.headers.push(("Set-Cookie".into(), cookie.into()));
        self
    }
}

pub(crate) struct HttpFixture {
    origin: String,
    stopped: Arc<AtomicBool>,
    records: Arc<Mutex<Vec<RecordedRequest>>>,
    worker: Option<JoinHandle<()>>,
}

impl HttpFixture {
    pub(crate) fn new(handler: impl Fn(&RecordedRequest) -> Reply + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let records = Arc::new(Mutex::new(Vec::new()));
        let captured = records.clone();
        let worker =
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(30);
                while !stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let request = read_request(&mut stream);
                            let reply = handler(&request);
                            let mut records = captured.lock().unwrap();
                            assert!(records.len() < 32, "Unexpected repeated source requests");
                            records.push(request);
                            drop(records);
                            write!(
                            stream,
                            "HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n",
                            reply.status, reply.body.len(),
                        ).unwrap();
                            for (name, value) in reply.headers {
                                write!(stream, "{name}: {value}\r\n").unwrap();
                            }
                            stream.write_all(b"\r\n").unwrap();
                            stream.write_all(&reply.body).unwrap();
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            assert!(
                                Instant::now() < deadline,
                                "The HTTP fixture was not stopped"
                            );
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) => panic!("Fixture listener failed: {error}"),
                    }
                }
            });
        Self {
            origin,
            stopped,
            records,
            worker: Some(worker),
        }
    }

    pub(crate) fn origin(&self) -> &str {
        &self.origin
    }

    pub(crate) fn http(&self) -> Client {
        http_client_builder(FIRST_PARTY_IDLE_PER_HOST)
            .no_proxy()
            .build()
            .unwrap()
    }

    /// Construct clients only from an active Tokio context.
    pub(crate) fn client(&self, auth: AuthSession) -> WeReadClient {
        let mut client = WeReadClient::new(auth).unwrap();
        client.http = self.http();
        client.resource_http = http_client_builder(0).no_proxy().build().unwrap();
        client.test_web_origin = Some(self.origin.clone());
        client.test_gateway_url = Some(format!("{}/gateway", self.origin));
        client
    }

    pub(crate) fn finish(mut self) -> Vec<RecordedRequest> {
        self.stopped.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
        self.records.lock().unwrap().clone()
    }
}

impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}

fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        assert!(
            header.len() < 128 * 1024,
            "Fixture request headers are too large"
        );
        let mut byte = [0u8];
        stream.read_exact(&mut byte).unwrap();
        header.push(byte[0]);
    }
    let header = String::from_utf8(header).unwrap();
    let mut lines = header.split("\r\n");
    let mut start = lines.next().unwrap().split_whitespace();
    let method = start.next().unwrap().into();
    let target = start.next().unwrap().into();
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().into()))
        .collect();
    let length = headers
        .get("content-length")
        .map_or(0, |length| length.parse::<usize>().unwrap());
    assert!(length < 256 * 1024, "Fixture request body is too large");
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).unwrap();
    RecordedRequest {
        method,
        target,
        headers,
        body,
    }
}

pub(crate) fn auth() -> AuthSession {
    AuthSession {
        api_key: "synthetic-api-gate-key".into(),
        user_vid: "synthetic-api-gate-vid".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "synthetic-api-gate-vid".into()),
            ("wr_skey".into(), "synthetic-api-gate-session".into()),
        ]),
        ..AuthSession::default()
    }
}
