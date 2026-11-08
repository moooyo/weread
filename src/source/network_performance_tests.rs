use super::*;
use flate2::{Compression, write::GzEncoder};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread;
use std::time::Instant;

const SHARDS: [&str; 3] = [
    "136D662D62D94B3AF8CEFB644E5DA3D8aSGVsWR",
    "F3BB6E9F13A09F4EC190181148A2520F8sIlYHJ",
    "66AF2A4DC1BFE79515769F2DB6AA08F9GlbciE=",
];
const TXT_SHARD: &str = "69323AAA4B1AF35468CB1604CB26CFCDaSGVsWR8sIlYHJGlbciE=";

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn text(body: impl AsRef<[u8]>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.as_ref().to_vec(),
        }
    }

    fn gzip(body: impl AsRef<[u8]>) -> Self {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(body.as_ref()).unwrap();
        Self {
            status: 200,
            headers: vec![("Content-Encoding".into(), "gzip".into())],
            body: encoder.finish().unwrap(),
        }
    }

    fn status(status: u16) -> Self {
        Self {
            status,
            ..Self::text("")
        }
    }

    fn disconnect() -> Self {
        // Status zero is a fixture action, never an HTTP response.
        Self::status(0)
    }

    fn cookie(mut self, name: &str, value: &str) -> Self {
        self.headers.push((
            "Set-Cookie".into(),
            format!("{name}={value}; Path=/; HttpOnly"),
        ));
        self
    }
}

struct Fixture {
    origin: String,
    stop: Arc<AtomicBool>,
    records: Arc<Mutex<Vec<(usize, RecordedRequest)>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn serve<F>(handler: F) -> Self
    where
        F: Fn(&RecordedRequest) -> Reply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let records = Arc::new(Mutex::new(Vec::new()));
        let captured = records.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let handler = Arc::new(handler);
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut workers = Vec::new();
            while !stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let index = workers.len();
                        let captured = captured.clone();
                        let handler = handler.clone();
                        workers.push(thread::spawn(move || {
                            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                            stream.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
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
                            // Dropped chapter futures deliberately close these
                            // connections in the cancellation regression.
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
        Self {
            origin,
            stop,
            records,
            worker: Some(worker),
        }
    }

    fn client(&self) -> WeReadClient {
        let mut client = WeReadClient::new(fixture_auth()).unwrap();
        client.test_web_origin = Some(self.origin.clone());
        client.http = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        client
    }

    fn finish(mut self) -> Vec<RecordedRequest> {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
        let mut records = self.records.lock().unwrap().clone();
        records.sort_by_key(|(index, _)| *index);
        records.into_iter().map(|(_, request)| request).collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    let mut bytes = Vec::new();
    let mut chunk = [0; 2048];
    let end = loop {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "Incomplete synthetic HTTP request");
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= 65536);
        if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let header = std::str::from_utf8(&bytes[..end]).unwrap();
    let mut lines = header.split("\r\n");
    let mut request_line = lines.next().unwrap().split_whitespace();
    let method = request_line.next().unwrap().to_owned();
    let path = request_line.next().unwrap().to_owned();
    let headers: BTreeMap<_, _> = lines
        .filter_map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect();
    let length = headers
        .get("content-length")
        .map_or(0, |value| value.parse::<usize>().unwrap());
    assert!(end + length <= 65536);
    while bytes.len() < end + length {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "Incomplete synthetic HTTP body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    RecordedRequest {
        method,
        path,
        headers,
        body: bytes[end..end + length].to_vec(),
    }
}

fn fixture_auth() -> AuthSession {
    AuthSession {
        api_key: "fixture-api-key".into(),
        user_vid: "fixture-account".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "fixture-account".into()),
            ("wr_skey".into(), "fixture-original-key".into()),
            ("wr_rt".into(), "fixture-original-refresh".into()),
        ]),
        ..Default::default()
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn reader(clock: &str) -> String {
    format!(r#"<script>window.__INITIAL_STATE__ = {{"reader":{{"psvts":"{clock}"}}}};</script>"#)
}

fn clock(request: &RecordedRequest) -> String {
    serde_json::from_slice::<Value>(&request.body).unwrap()["ps"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn count(requests: &[RecordedRequest], path: &str) -> usize {
    requests
        .iter()
        .filter(|request| request.path == path)
        .count()
}

#[derive(Default)]
struct PairBarrier {
    count: Mutex<usize>,
    ready: Condvar,
}

impl PairBarrier {
    fn arrive(&self) {
        let mut count = self.count.lock().unwrap();
        *count += 1;
        self.ready.notify_all();
        while *count < 2 {
            let (next, timeout) = self
                .ready
                .wait_timeout(count, Duration::from_secs(2))
                .unwrap();
            assert!(
                !timeout.timed_out(),
                "Both shards must arrive before either reply"
            );
            count = next;
        }
    }
}

fn epub_reply(request: &RecordedRequest, reader_clock: &str) -> Reply {
    if request.path.starts_with("/web/reader/") {
        return Reply::gzip(reader(reader_clock));
    }
    let index = match request.path.as_str() {
        "/web/book/chapter/e_0" => 0,
        "/web/book/chapter/e_1" => 1,
        "/web/book/chapter/e_3" => 2,
        _ => panic!("Unexpected synthetic operation"),
    };
    assert_eq!(clock(request), reader_clock);
    Reply::text(SHARDS[index])
}

fn txt_reply(request: &RecordedRequest) -> Reply {
    if request.path.starts_with("/web/reader/") {
        return Reply::gzip(reader("fixture-txt-clock"));
    }
    match request.path.as_str() {
        "/web/book/chapter/e_0" => Reply::text(r#"{"bookId":"fixture-book"}"#),
        "/web/book/chapter/t_0" => Reply::text(TXT_SHARD),
        "/web/book/chapter/t_1" => Reply::status(404),
        _ => panic!("Unexpected synthetic operation"),
    }
}

#[test]
fn epub_reads_overlap_and_assemble_nonempty_shards_in_endpoint_order() {
    let barrier = Arc::new(PairBarrier::default());
    let paired = barrier.clone();
    let fixture = Fixture::serve(move |request| {
        if matches!(
            request.path.as_str(),
            "/web/book/chapter/e_1" | "/web/book/chapter/e_3"
        ) {
            paired.arrive();
            if request.path.ends_with("e_1") {
                thread::sleep(Duration::from_millis(20));
            }
        }
        epub_reply(request, "fixture-ordered-clock")
    });
    let content = runtime()
        .block_on(fixture.client().chapter("fixture-book", "fixture-chapter"))
        .unwrap();
    assert_eq!(content.html, "Hello, reader!");
    assert_eq!(content.plain_text, "Hello, reader!");
    assert!(matches!(content.format, ContentFormat::Xhtml));
    assert_eq!(*barrier.count.lock().unwrap(), 2);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].headers["accept-encoding"], "gzip");
    assert_eq!(requests[1].path, "/web/book/chapter/e_0");
    for request in &requests[1..] {
        assert_eq!(request.method, "POST");
        assert_eq!(request.headers["accept-encoding"], "identity");
    }
}

#[test]
fn compressed_catalog_business_expiry_retains_bounded_renewal_and_cookie_rotation() {
    let catalog_reads = Arc::new(AtomicUsize::new(0));
    let reads = catalog_reads.clone();
    let fixture = Fixture::serve(move |request| {
        assert_eq!(request.headers["accept-encoding"], "gzip");
        match request.path.as_str() {
            "/web/book/chapterInfos" if reads.fetch_add(1, Ordering::SeqCst) == 0 => {
                Reply::gzip(r#"{"errCode":-2012}"#).cookie("wr_skey", "fixture-rejected-key")
            }
            "/web/book/chapterInfos" => {
                assert!(request.headers["cookie"].contains("fixture-renewed-key"));
                Reply::gzip(
                    r#"{"data":[{"bookId":"fixture-book","updated":[{"chapterUid":1,"title":"Fixture","wordCount":100}]}]}"#,
                )
            }
            "/web/login/renewal" => {
                assert!(request.headers["cookie"].contains("fixture-original-key"));
                Reply::gzip(r#"{"succ":1}"#)
                    .cookie("wr_skey", "fixture-renewed-key")
                    .cookie("wr_rt", "fixture-renewed-refresh")
            }
            _ => panic!("Unexpected synthetic operation"),
        }
    });
    let client = fixture.client();
    let chapters = runtime().block_on(client.catalog("fixture-book")).unwrap();
    assert_eq!(chapters.len(), 1);
    assert_eq!(chapters[0].uid, "1");
    assert_eq!(client.session().cookies["wr_skey"], "fixture-renewed-key");
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(count(&requests, "/web/login/renewal"), 1);
    assert_eq!(requests[0].body, requests[2].body);
}

#[test]
fn compressed_article_html_is_decoded_without_changing_its_content() {
    let fixture = Fixture::serve(|request| {
        assert!(request.path.starts_with("/web/mp/content?"));
        assert_eq!(request.headers["accept-encoding"], "gzip");
        Reply::gzip("<article>Fixture article.</article>")
    });
    let html = runtime()
        .block_on(fixture.client().mp_content("fixture-article"))
        .unwrap();
    assert_eq!(html, "<article>Fixture article.</article>");
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn gzip_expansion_is_limited_after_decoding_even_with_small_wire_length() {
    let fixture = Fixture::serve(|request| {
        assert_eq!(request.headers["accept-encoding"], "gzip");
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        for _ in 0..MAX_RESPONSE_BYTES / 4096 {
            encoder.write_all(&[b'x'; 4096]).unwrap();
        }
        encoder.write_all(b"x").unwrap();
        let body = encoder.finish().unwrap();
        assert!(body.len() < MAX_RESPONSE_BYTES / 100);
        Reply {
            status: 200,
            headers: vec![("Content-Encoding".into(), "gzip".into())],
            body,
        }
    });
    let result = runtime().block_on(fixture.client().reader_session("fixture-book", None));
    assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn txt_format_is_reused_only_after_a_verified_chapter() {
    let fixture = Fixture::serve(txt_reply);
    let client = fixture.client();
    let mut hints = ChapterReadHints::default();
    runtime().block_on(async {
        for chapter in ["fixture-first", "fixture-second"] {
            let content = client
                .chapter_with_hints("fixture-book", chapter, &mut hints)
                .await
                .unwrap();
            assert_eq!(content.plain_text, "Hello, reader!");
            assert!(matches!(content.format, ContentFormat::Text));
        }
    });
    let requests = fixture.finish();
    assert_eq!(requests.len(), 7);
    assert_eq!(count(&requests, "/web/book/chapter/e_0"), 1);
    assert_eq!(count(&requests, "/web/book/chapter/t_0"), 2);
}

#[test]
fn txt_hints_cannot_cross_books_accounts_clients_or_tasks() {
    let fixture = Fixture::serve(txt_reply);
    let client = fixture.client();
    let mut hints = ChapterReadHints::default();
    runtime().block_on(async {
        client
            .chapter_with_hints("fixture-book", "1", &mut hints)
            .await
            .unwrap();
        client
            .chapter_with_hints("fixture-other-book", "2", &mut hints)
            .await
            .unwrap();
        let mut replacement = fixture_auth();
        replacement.user_vid = "fixture-other-account".into();
        replacement
            .cookies
            .insert("wr_vid".into(), "fixture-other-account".into());
        client.replace_session(replacement).unwrap();
        client
            .chapter_with_hints("fixture-other-book", "3", &mut hints)
            .await
            .unwrap();
        fixture
            .client()
            .chapter_with_hints("fixture-other-book", "4", &mut hints)
            .await
            .unwrap();
        client
            .chapter_with_hints("fixture-other-book", "5", &mut ChapterReadHints::default())
            .await
            .unwrap();
    });
    let requests = fixture.finish();
    assert_eq!(requests.len(), 20);
    assert_eq!(count(&requests, "/web/book/chapter/e_0"), 5);
}

#[test]
fn unknown_account_identity_does_not_enable_txt_hints() {
    let fixture = Fixture::serve(txt_reply);
    let client = fixture.client();
    let mut unknown = fixture_auth();
    unknown.user_vid.clear();
    unknown.cookies.remove("wr_vid");
    client.replace_session(unknown).unwrap();
    let mut hints = ChapterReadHints::default();
    runtime().block_on(async {
        for chapter in ["1", "2"] {
            client
                .chapter_with_hints("fixture-book", chapter, &mut hints)
                .await
                .unwrap();
        }
    });
    assert_eq!(count(&fixture.finish(), "/web/book/chapter/e_0"), 2);
}

#[test]
fn stale_txt_hint_reacquires_reader_and_reprobes_format_once() {
    let changed = Arc::new(AtomicBool::new(false));
    let format_changed = changed.clone();
    let fixture = Fixture::serve(move |request| {
        if !format_changed.load(Ordering::SeqCst) {
            return txt_reply(request);
        }
        if request.path == "/web/book/chapter/t_0" {
            return Reply::status(404);
        }
        epub_reply(request, "fixture-new-format-clock")
    });
    let client = fixture.client();
    let mut hints = ChapterReadHints::default();
    runtime().block_on(async {
        client
            .chapter_with_hints("fixture-book", "1", &mut hints)
            .await
            .unwrap();
        changed.store(true, Ordering::SeqCst);
        let content = client
            .chapter_with_hints("fixture-book", "2", &mut hints)
            .await
            .unwrap();
        assert!(matches!(content.format, ContentFormat::Xhtml));
        assert_eq!(content.plain_text, "Hello, reader!");
    });
    let requests = fixture.finish();
    assert_eq!(requests.len(), 10);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path.starts_with("/web/reader/"))
            .count(),
        3
    );
    assert_eq!(count(&requests, "/web/book/chapter/e_0"), 2);
    assert_eq!(count(&requests, "/web/book/chapter/t_0"), 2);
    assert!(!hints.confirmed_text);
}

#[test]
fn invalid_txt_content_clears_a_hint_and_keeps_all_retries_bounded() {
    let corrupt = Arc::new(AtomicBool::new(false));
    let corrupted = corrupt.clone();
    let fixture = Fixture::serve(move |request| {
        if request.path == "/web/book/chapter/t_0" && corrupted.load(Ordering::SeqCst) {
            return Reply::text(format!("{}{}", "0".repeat(32), &TXT_SHARD[32..]));
        }
        txt_reply(request)
    });
    let client = fixture.client();
    let mut hints = ChapterReadHints::default();
    runtime().block_on(async {
        client
            .chapter_with_hints("fixture-book", "1", &mut hints)
            .await
            .unwrap();
        corrupt.store(true, Ordering::SeqCst);
        assert!(matches!(
            client
                .chapter_with_hints("fixture-book", "2", &mut hints)
                .await,
            Err(SourceError::Integrity)
        ));
    });
    let requests = fixture.finish();
    assert_eq!(requests.len(), 19);
    assert_eq!(count(&requests, "/web/book/chapter/e_0"), 4);
    assert!(!hints.confirmed_text);
}

#[test]
fn a_corrupt_parallel_shard_reloads_reader_and_every_shard_before_success() {
    let readers = Arc::new(AtomicUsize::new(0));
    let reader_count = readers.clone();
    let fixture = Fixture::serve(move |request| {
        if request.path.starts_with("/web/reader/") {
            let index = reader_count.fetch_add(1, Ordering::SeqCst);
            return Reply::gzip(reader(if index == 0 {
                "fixture-corrupt-clock"
            } else {
                "fixture-intact-clock"
            }));
        }
        let reader_clock = clock(request);
        if reader_clock == "fixture-corrupt-clock" && request.path.ends_with("e_1") {
            return Reply::text(format!("{}{}", "0".repeat(32), &SHARDS[1][32..]));
        }
        epub_reply(request, &reader_clock)
    });
    let content = runtime()
        .block_on(fixture.client().chapter("fixture-book", "1"))
        .unwrap();
    assert_eq!(content.plain_text, "Hello, reader!");
    let requests = fixture.finish();
    assert_eq!(requests.len(), 8);
    for endpoint in ["e_0", "e_1", "e_3"] {
        assert_eq!(
            count(&requests, &format!("/web/book/chapter/{endpoint}")),
            2
        );
    }
    assert!(requests[4].path.starts_with("/web/reader/"));
    for request in &requests[5..] {
        assert_eq!(clock(request), "fixture-intact-clock");
    }
}

#[test]
fn concurrent_shard_expiry_shares_one_renewal_and_refetches_the_full_chapter() {
    for still_expired in [false, true] {
        let barrier = Arc::new(PairBarrier::default());
        let paired = barrier.clone();
        let fixture = Fixture::serve(move |request| {
            if request.path == "/web/login/renewal" {
                return Reply::gzip(r#"{"succ":1}"#)
                    .cookie("wr_skey", "fixture-renewed-key")
                    .cookie("wr_rt", "fixture-renewed-refresh");
            }
            let renewed = request.headers["cookie"].contains("fixture-renewed-key");
            let reader_clock = if renewed {
                "fixture-renewed-clock"
            } else {
                "fixture-original-clock"
            };
            if matches!(
                request.path.as_str(),
                "/web/book/chapter/e_1" | "/web/book/chapter/e_3"
            ) {
                if !renewed {
                    paired.arrive();
                }
                if !renewed || still_expired {
                    return Reply::text(r#"{"errCode":-2012}"#)
                        .cookie("wr_skey", "fixture-rejected-key");
                }
            }
            epub_reply(request, reader_clock)
        });
        let client = fixture.client();
        let result = runtime().block_on(client.chapter("fixture-book", "fixture-chapter"));
        if still_expired {
            assert!(matches!(result, Err(SourceError::Api(code)) if code == "-2012"));
        } else {
            assert_eq!(result.unwrap().plain_text, "Hello, reader!");
        }
        assert_eq!(client.session().cookies["wr_skey"], "fixture-renewed-key");
        let requests = fixture.finish();
        assert_eq!(requests.len(), 9);
        assert_eq!(count(&requests, "/web/login/renewal"), 1);
        for endpoint in ["e_0", "e_1", "e_3"] {
            assert_eq!(
                count(&requests, &format!("/web/book/chapter/{endpoint}")),
                2
            );
        }
        assert_eq!(requests[4].path, "/web/login/renewal");
        assert!(requests[5].path.starts_with("/web/reader/"));
        for request in &requests[6..] {
            assert_eq!(clock(request), "fixture-renewed-clock");
            assert!(request.headers["cookie"].contains("fixture-renewed-key"));
        }
    }
}

#[test]
fn ordinary_cookie_rotation_cannot_hide_a_parallel_peers_failed_renewal() {
    for failure in ["network", "unknown_api"] {
        let barrier = Arc::new(PairBarrier::default());
        let fixture = Fixture::serve(move |request| {
            if request.path == "/web/login/renewal" {
                assert!(request.headers["cookie"].contains("wr_skey=fixture-original-key"));
                assert!(request.headers["cookie"].contains("RK=fixture-ordinary-rk"));
                return if failure == "network" {
                    Reply::disconnect()
                } else {
                    Reply::gzip(r#"{"errCode":-9999}"#)
                        .cookie("wr_skey", "fixture-unacknowledged-key")
                        .cookie("wr_rt", "fixture-unacknowledged-refresh")
                };
            }
            if matches!(
                request.path.as_str(),
                "/web/book/chapter/e_1" | "/web/book/chapter/e_3"
            ) {
                barrier.arrive();
                if request.path.ends_with("e_1") {
                    return Reply::text(SHARDS[1])
                        .cookie("RK", "fixture-ordinary-rk")
                        .cookie("pgv_pvid", "fixture-ordinary-pvid");
                }
                return Reply::text(r#"{"errCode":-2012}"#)
                    .cookie("wr_skey", "fixture-rejected-key");
            }
            epub_reply(request, "fixture-failed-renewal-clock")
        });
        let client = fixture.client();
        let (result, generation) = runtime().block_on(async {
            let result = client.chapter("fixture-book", "fixture-chapter").await;
            let generation = *client.renewal.lock().await;
            (result, generation)
        });
        if failure == "network" {
            assert!(matches!(result, Err(SourceError::Network)));
        } else {
            assert!(matches!(result, Err(SourceError::Api(code)) if code == "-9999"));
        }
        let session = client.session();
        assert_eq!(session.cookies["wr_skey"], "fixture-original-key");
        assert_eq!(session.cookies["wr_rt"], "fixture-original-refresh");
        assert_eq!(session.cookies["RK"], "fixture-ordinary-rk");
        assert_eq!(session.cookies["pgv_pvid"], "fixture-ordinary-pvid");
        assert_eq!(generation, 0);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 5);
        assert_eq!(count(&requests, "/web/login/renewal"), 1);
        assert_eq!(count(&requests, "/web/book/chapter/e_0"), 1);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path.starts_with("/web/reader/"))
                .count(),
            1,
        );
    }
}

#[test]
fn a_terminal_identity_error_in_either_parallel_shard_prevents_renewal() {
    for terminal_first in [false, true] {
        let barrier = Arc::new(PairBarrier::default());
        let fixture = Fixture::serve(move |request| {
            if matches!(
                request.path.as_str(),
                "/web/book/chapter/e_1" | "/web/book/chapter/e_3"
            ) {
                barrier.arrive();
                return Reply::text(if request.path.ends_with("e_1") == terminal_first {
                    r#"{"errCode":-2013}"#
                } else {
                    r#"{"errCode":-2012}"#
                });
            }
            epub_reply(request, "fixture-identity-clock")
        });
        let client = fixture.client();
        let before = client.session();
        let result = runtime().block_on(client.chapter("fixture-book", "fixture-chapter"));
        assert!(matches!(result, Err(SourceError::Api(code)) if code == "-2013"));
        assert_eq!(client.session(), before);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 4);
        assert_eq!(count(&requests, "/web/login/renewal"), 0);
    }
}

#[test]
fn out_of_order_rotations_force_one_serial_restart_and_remain_serial_for_the_task() {
    let readers = Arc::new(AtomicUsize::new(0));
    let reader_count = readers.clone();
    let barrier = Arc::new(PairBarrier::default());
    let release_late = Arc::new((Mutex::new(false), Condvar::new()));
    let releasing = release_late.clone();
    let fixture = Fixture::serve(move |request| {
        if request.path.starts_with("/web/reader/") {
            let index = reader_count.fetch_add(1, Ordering::SeqCst);
            if index == 0 {
                return Reply::gzip(reader("fixture-parallel-clock"));
            }
            if index == 1 {
                assert!(request.headers["cookie"].contains("wr_skey=fixture-first-arrival-key"));
                assert!(!request.headers["cookie"].contains("fixture-late-arrival-key"));
            }
            return Reply::gzip(reader("fixture-serial-clock"))
                .cookie("wr_skey", "fixture-reader-key");
        }
        let parallel = clock(request) == "fixture-parallel-clock";
        let index = match request.path.as_str() {
            "/web/book/chapter/e_0" => 0,
            "/web/book/chapter/e_1" => 1,
            "/web/book/chapter/e_3" => 2,
            _ => panic!("Unexpected synthetic operation"),
        };
        if parallel && index > 0 {
            barrier.arrive();
            if index == 1 {
                let (lock, ready) = &*releasing;
                let mut released = lock.lock().unwrap();
                while !*released {
                    let (next, timeout) = ready
                        .wait_timeout(released, Duration::from_secs(3))
                        .unwrap();
                    assert!(
                        !timeout.timed_out(),
                        "The first rotation must be observed before the late reply"
                    );
                    released = next;
                }
                return Reply::text(SHARDS[index]).cookie("wr_skey", "fixture-late-arrival-key");
            }
            return Reply::text(SHARDS[index]).cookie("wr_skey", "fixture-first-arrival-key");
        }
        if !parallel {
            assert_eq!(clock(request), "fixture-serial-clock");
            assert!(request.headers["cookie"].contains(if index == 2 {
                "wr_skey=fixture-serial-first-key"
            } else {
                "wr_skey=fixture-reader-key"
            }));
            if index == 1 {
                return Reply::text(SHARDS[index]).cookie("wr_skey", "fixture-serial-first-key");
            }
            if index == 2 {
                return Reply::text(SHARDS[index]).cookie("wr_skey", "fixture-serial-last-key");
            }
        }
        Reply::text(SHARDS[index])
    });
    let client = fixture.client();
    let mut hints = ChapterReadHints::default();
    runtime().block_on(async {
        let mut first = Box::pin(client.chapter_with_hints("fixture-book", "1", &mut hints));
        tokio::select! {
            _ = &mut first => panic!("The late reply must keep the first chapter pending"),
            result = tokio::time::timeout(Duration::from_secs(2), async {
                while client.session().cookies.get("wr_skey").is_none_or(|key| key != "fixture-first-arrival-key") {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            }) => result.unwrap(),
        }
        let (lock, ready) = &*release_late;
        *lock.lock().unwrap() = true;
        ready.notify_all();
        assert_eq!(first.await.unwrap().plain_text, "Hello, reader!");
        let content = client.chapter_with_hints("fixture-book", "2", &mut hints).await.unwrap();
        assert_eq!(content.plain_text, "Hello, reader!");
    });
    assert!(hints.serial_epub);
    assert_eq!(readers.load(Ordering::SeqCst), 3);
    assert_eq!(
        client.session().cookies["wr_skey"],
        "fixture-serial-last-key"
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 12);
    assert_eq!(count(&requests, "/web/login/renewal"), 0);
}

#[test]
fn reader_rotation_selects_serial_shards_before_dispatching_the_pair() {
    let fixture = Fixture::serve(|request| {
        if request.path.starts_with("/web/reader/") {
            return Reply::gzip(reader("fixture-rotating-reader"))
                .cookie("wr_skey", "fixture-reader-key");
        }
        if request.path.ends_with("e_1") {
            assert!(request.headers["cookie"].contains("wr_skey=fixture-reader-key"));
            return Reply::text(SHARDS[1]).cookie("wr_skey", "fixture-first-shard-key");
        }
        if request.path.ends_with("e_3") {
            assert!(request.headers["cookie"].contains("wr_skey=fixture-first-shard-key"));
        }
        epub_reply(request, "fixture-rotating-reader")
    });
    let content = runtime()
        .block_on(fixture.client().chapter("fixture-book", "1"))
        .unwrap();
    assert_eq!(content.plain_text, "Hello, reader!");
    assert_eq!(fixture.finish().len(), 4);
}

#[test]
fn a_rotating_batch_rejected_during_serial_fallback_renews_once_and_refetches_all_shards() {
    let barrier = Arc::new(PairBarrier::default());
    let fixture = Fixture::serve(move |request| {
        if request.path == "/web/login/renewal" {
            assert!(request.headers["cookie"].contains("fixture-pair-"));
            return Reply::gzip(r#"{"succ":1}"#)
                .cookie("wr_skey", "fixture-renewed-key")
                .cookie("wr_rt", "fixture-renewed-refresh");
        }
        let renewed = request.headers["cookie"].contains("fixture-renewed-key");
        if request.path.starts_with("/web/reader/")
            && request.headers["cookie"].contains("fixture-pair-")
        {
            return Reply::gzip(r#"{"errCode":-2012}"#);
        }
        let reader_clock = if renewed {
            "fixture-renewed-clock"
        } else {
            "fixture-rotating-clock"
        };
        if !renewed
            && matches!(
                request.path.as_str(),
                "/web/book/chapter/e_1" | "/web/book/chapter/e_3"
            )
        {
            barrier.arrive();
            return Reply::text(if request.path.ends_with("e_1") {
                SHARDS[1]
            } else {
                SHARDS[2]
            })
            .cookie(
                "wr_skey",
                if request.path.ends_with("e_1") {
                    "fixture-pair-first-key"
                } else {
                    "fixture-pair-second-key"
                },
            );
        }
        epub_reply(request, reader_clock)
    });
    let client = fixture.client();
    let content = runtime()
        .block_on(client.chapter("fixture-book", "1"))
        .unwrap();
    assert_eq!(content.plain_text, "Hello, reader!");
    assert_eq!(client.session().cookies["wr_skey"], "fixture-renewed-key");
    let requests = fixture.finish();
    assert_eq!(requests.len(), 11);
    assert_eq!(count(&requests, "/web/login/renewal"), 1);
    assert_eq!(requests[5].path, "/web/login/renewal");
    assert!(requests[6].path.starts_with("/web/reader/"));
    assert!(requests[7].path.starts_with("/web/reader/"));
    for request in &requests[8..] {
        assert_eq!(clock(request), "fixture-renewed-clock");
        assert!(request.headers["cookie"].contains("fixture-renewed-key"));
    }
    for endpoint in ["e_0", "e_1", "e_3"] {
        assert_eq!(
            count(&requests, &format!("/web/book/chapter/{endpoint}")),
            2
        );
    }
}

#[test]
fn cancelling_a_parallel_chapter_leaves_no_detached_recovery_or_later_reads() {
    let arrived = Arc::new(AtomicUsize::new(0));
    let arrivals = arrived.clone();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let releasing = release.clone();
    let fixture = Fixture::serve(move |request| {
        if matches!(
            request.path.as_str(),
            "/web/book/chapter/e_1" | "/web/book/chapter/e_3"
        ) {
            arrivals.fetch_add(1, Ordering::SeqCst);
            let (lock, ready) = &*releasing;
            let mut released = lock.lock().unwrap();
            while !*released {
                let (next, timeout) = ready
                    .wait_timeout(released, Duration::from_secs(3))
                    .unwrap();
                assert!(
                    !timeout.timed_out(),
                    "Cancellation must release the synthetic replies"
                );
                released = next;
            }
            return Reply::text(r#"{"errCode":-2012}"#).cookie("wr_skey", "fixture-rejected-key");
        }
        epub_reply(request, "fixture-cancelled-clock")
    });
    let client = fixture.client();
    let before = client.session();
    runtime().block_on(async {
        let mut chapter = Box::pin(client.chapter("fixture-book", "fixture-chapter"));
        tokio::select! {
            _ = &mut chapter => panic!("The held replies must keep the chapter pending"),
            result = tokio::time::timeout(Duration::from_secs(2), async {
                while arrived.load(Ordering::SeqCst) < 2 {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            }) => result.unwrap(),
        }
        drop(chapter);
        let (lock, ready) = &*release;
        *lock.lock().unwrap() = true;
        ready.notify_all();
        tokio::time::sleep(Duration::from_millis(80)).await;
    });
    assert_eq!(client.session(), before);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(count(&requests, "/web/login/renewal"), 0);
}

#[test]
#[ignore = "Controlled mock latency benchmark; not a physical-device measurement"]
fn mock_latency_download_network_benchmark() {
    const CHAPTERS: usize = 8;
    const MOCK_LATENCY: Duration = Duration::from_millis(50);

    for serial in [true, false] {
        let fixture = Fixture::serve(|request| {
            thread::sleep(MOCK_LATENCY);
            epub_reply(request, "fixture-benchmark-clock")
        });
        let client = fixture.client();
        let mut hints = ChapterReadHints::default();
        hints.bind(&client, "fixture-book");
        hints.serial_epub = serial;
        let started = Instant::now();
        runtime().block_on(async {
            for chapter in 0..CHAPTERS {
                let content = client
                    .chapter_with_hints("fixture-book", &chapter.to_string(), &mut hints)
                    .await
                    .unwrap();
                assert_eq!(content.plain_text, "Hello, reader!");
            }
        });
        let elapsed = started.elapsed();
        let requests = fixture.finish();
        assert_eq!(requests.len(), CHAPTERS * 4);
        println!(
            "weread-network-benchmark {}",
            json!({
                "environment": "synthetic_http_fixture",
                "device_measurement": false,
                "mock_latency_ms": MOCK_LATENCY.as_millis(),
                "mode": if serial { "epub_serial_reference" } else { "epub_parallel_pair" },
                "chapters": CHAPTERS,
                "requests": requests.len(),
                "wall_time_ms": elapsed.as_millis(),
                "integrity_success": true,
            }),
        );
    }

    for shared in [false, true] {
        let fixture = Fixture::serve(|request| {
            thread::sleep(MOCK_LATENCY);
            txt_reply(request)
        });
        let client = fixture.client();
        let mut task_hints = ChapterReadHints::default();
        let started = Instant::now();
        runtime().block_on(async {
            for chapter in 0..CHAPTERS {
                let mut fresh_hints = ChapterReadHints::default();
                let hints = if shared {
                    &mut task_hints
                } else {
                    &mut fresh_hints
                };
                let content = client
                    .chapter_with_hints("fixture-book", &chapter.to_string(), hints)
                    .await
                    .unwrap();
                assert_eq!(content.plain_text, "Hello, reader!");
            }
        });
        let elapsed = started.elapsed();
        let requests = fixture.finish();
        assert_eq!(
            requests.len(),
            if shared {
                CHAPTERS * 3 + 1
            } else {
                CHAPTERS * 4
            }
        );
        println!(
            "weread-network-benchmark {}",
            json!({
                "environment": "synthetic_http_fixture",
                "device_measurement": false,
                "mock_latency_ms": MOCK_LATENCY.as_millis(),
                "mode": if shared { "txt_task_hints" } else { "txt_fresh_hints_reference" },
                "chapters": CHAPTERS,
                "requests": requests.len(),
                "wall_time_ms": elapsed.as_millis(),
                "integrity_success": true,
            }),
        );
    }
}
