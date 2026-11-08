//! Elapsed-report response handling over an isolated, bounded HTTP transport.

use super::*;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

type FixtureResult<T> = std::result::Result<T, String>;

struct RecordedPost {
    method: String,
    path: String,
    body: Vec<u8>,
}

struct SinglePostFixture {
    origin: String,
    cancelled: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<FixtureResult<Option<RecordedPost>>>>,
}

impl SinglePostFixture {
    fn new(status: u16, body: &'static str, reject_cookies: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if worker_cancelled.load(Ordering::Acquire) {
                    return Ok(None);
                }
                if Instant::now() >= deadline {
                    return Err(
                        "The synthetic elapsed POST did not arrive before its deadline".into(),
                    );
                }
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_millis(50)))
                            .map_err(|error| error.to_string())?;
                        stream
                            .set_write_timeout(Some(Duration::from_millis(500)))
                            .map_err(|error| error.to_string())?;
                        let post = read_post(&mut stream, &worker_cancelled, deadline)?;
                        let mut response = format!(
                            "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                            body.len(),
                        );
                        if reject_cookies {
                            // A valid rotation preceding the oversized entry must
                            // also remain uncommitted when the batch is rejected.
                            response
                                .push_str("Set-Cookie: wr_skey=fixture-rotated-key; Path=/\r\n");
                            response.push_str("x-wr-ticket: fixture-rotated-ticket\r\n");
                            response.push_str(&format!(
                                "Set-Cookie: wr_oversized={}; Path=/\r\n",
                                "x".repeat(super::super::client::MAX_COOKIE_ENTRY_BYTES),
                            ));
                        }
                        response.push_str("\r\n");
                        response.push_str(body);
                        stream
                            .write_all(response.as_bytes())
                            .map_err(|error| error.to_string())?;
                        return Ok(Some(post));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
        });
        Self {
            origin,
            cancelled,
            worker: Some(worker),
        }
    }

    fn client(&self) -> WeReadClient {
        let mut client = WeReadClient::new(AuthSession {
            api_key: "fixture-api-key".into(),
            user_vid: "fixture-account".into(),
            cookies: BTreeMap::from([
                ("wr_vid".into(), "fixture-account".into()),
                ("wr_skey".into(), "fixture-original-key".into()),
            ]),
            ..Default::default()
        })
        .unwrap();
        client.test_web_origin = Some(self.origin.clone());
        client.http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        client
    }

    fn finish(mut self) -> RecordedPost {
        self.worker
            .take()
            .unwrap()
            .join()
            .expect("The synthetic report worker panicked")
            .expect("The synthetic report transport failed")
            .expect("The synthetic report was cancelled before its POST")
    }
}

impl Drop for SinglePostFixture {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            // Listener polling, reads, and writes all have finite deadlines, so
            // a failed assertion cannot detach a fixture thread or retain a socket.
            let _ = worker.join();
        }
    }
}

fn read_post(
    stream: &mut TcpStream,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> FixtureResult<RecordedPost> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 2048];
    let header_end = loop {
        read_chunk(stream, &mut bytes, &mut chunk, cancelled, deadline)?;
        if bytes.len() > 64 * 1024 {
            return Err("The synthetic report request exceeded its byte limit".into());
        }
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let header = std::str::from_utf8(&bytes[..header_end]).map_err(|error| error.to_string())?;
    let mut lines = header.split("\r\n");
    let mut request_line = lines
        .next()
        .ok_or("Missing synthetic request line")?
        .split_whitespace();
    let method = request_line
        .next()
        .ok_or("Missing synthetic request method")?
        .to_owned();
    let path = request_line
        .next()
        .ok_or("Missing synthetic request path")?
        .to_owned();
    let length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .ok_or("Missing synthetic report length")?
        .1
        .trim()
        .parse::<usize>()
        .map_err(|error| error.to_string())?;
    if length > 64 * 1024 - header_end {
        return Err("The synthetic report body exceeded its byte limit".into());
    }
    while bytes.len() < header_end + length {
        read_chunk(stream, &mut bytes, &mut chunk, cancelled, deadline)?;
    }
    Ok(RecordedPost {
        method,
        path,
        body: bytes[header_end..header_end + length].to_vec(),
    })
}

fn read_chunk(
    stream: &mut TcpStream,
    bytes: &mut Vec<u8>,
    chunk: &mut [u8],
    cancelled: &AtomicBool,
    deadline: Instant,
) -> FixtureResult<()> {
    loop {
        if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err("The synthetic report read was cancelled or timed out".into());
        }
        match stream.read(chunk) {
            Ok(0) => return Err("The synthetic report request ended early".into()),
            Ok(count) => {
                bytes.extend_from_slice(&chunk[..count]);
                return Ok(());
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn deliver_once(
    status: u16,
    body: &'static str,
    reject_cookies: bool,
) -> Result<ReadReportReceipt> {
    let fixture = SinglePostFixture::new(status, body, reject_cookies);
    let client = fixture.client();
    let before = client.session();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(async {
        let response = client
            .web_request(Method::POST, "/web/book/read")
            .unwrap()
            .json(&serde_json::json!({"rt": 37, "fixture": "single-elapsed-report"}))
            .send()
            .await
            .unwrap();
        let cookie_result = client.observe_web_response(&before, &response);
        if reject_cookies {
            assert!(matches!(
                &cookie_result,
                Err(SourceError::InvalidResponse("cookie_budget"))
            ));
        } else {
            assert!(cookie_result.is_ok());
        }
        finish_elapsed_report(response, cookie_result, 37).await
    });
    assert_eq!(client.session(), before);
    let post = fixture.finish();
    assert_eq!(post.method, "POST");
    assert_eq!(post.path, "/web/book/read");
    let sent: Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(sent["rt"], 37);
    result
}

#[test]
fn explicit_elapsed_receipts_remain_confirmed_when_cookie_updates_are_rejected() {
    for (body, has_synckey) in [
        (r#"{"succ":1}"#, false),
        (r#"{"succ":true}"#, false),
        (r#"{"synckey":"fixture-ack"}"#, true),
        (r#"{"data":{"synckey":7}}"#, true),
        (r#"{"result":{"succ":true}}"#, false),
    ] {
        let receipt = deliver_once(200, body, true).unwrap();
        assert_eq!(receipt.seconds, 37);
        assert_eq!(receipt.has_synckey, has_synckey);
        assert!(receipt.accepted_at > 0);
    }
}

#[test]
fn missing_or_failed_elapsed_receipts_with_rejected_cookies_never_become_safe_failures() {
    for (status, body) in [
        (200, "{}"),
        (200, "not-json"),
        (200, r#"{"succ":0}"#),
        (200, r#"{"errCode":-2012}"#),
        (200, r#"{"data":{"errCode":123}}"#),
        (400, r#"{"succ":1}"#),
        (403, "{}"),
        (429, "{}"),
        (500, "{}"),
        (503, r#"{"succ":1}"#),
    ] {
        let error = deliver_once(status, body, true).unwrap_err();
        assert!(matches!(error, SourceError::DeliveryUncertain));
        let failure = ReadReportFailure::from(&error);
        assert!(failure.uncertain);
        assert!(!failure.retryable && !failure.requires_login);
    }
}

#[test]
fn accepted_cookie_processing_preserves_existing_elapsed_failure_classification() {
    for status in [400, 401, 403, 429] {
        assert!(
            matches!(deliver_once(status, "{}", false), Err(SourceError::Http(code)) if code == status)
        );
    }
    assert!(matches!(
        deliver_once(200, r#"{"errCode":123}"#, false),
        Err(SourceError::Api(code)) if code == "123"
    ));
    for (status, body) in [
        (200, "{}"),
        (200, "not-json"),
        (500, "{}"),
        (503, r#"{"succ":1}"#),
    ] {
        assert!(matches!(
            deliver_once(status, body, false),
            Err(SourceError::DeliveryUncertain)
        ));
    }
    let receipt = deliver_once(200, r#"{"succ":1}"#, false).unwrap();
    assert_eq!(receipt.seconds, 37);
    assert!(!receipt.has_synckey);
}
