//! Borrowed-field admission and genuine, bounded loopback underline requests.

use super::*;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    thread::{self, JoinHandle},
    time::Instant,
};

fn small_limits() -> UnderlineLimits {
    UnderlineLimits {
        records: 2,
        quote_bytes: 4,
        range_bytes: 3,
        total_bytes: 8,
    }
}

fn assert_resource_error<T>(result: Result<T>) {
    let error = match result {
        Ok(_) => panic!("The oversized underline payload was accepted"),
        Err(error) => error,
    };
    assert!(matches!(&error, SourceError::ResponseTooLarge));
    assert!(!error.retryable() && !error.requires_login());
}

#[test]
fn borrowed_underline_aliases_preserve_numbers_unicode_and_counts() {
    let value = json!({"updated":[
        {"range":42,"markText":false,"bookmarkText":"\u{4e2d}\u{6587}","underlineCount":7,"reviewsCount":3},
        {"range":"opaque","markText":123,"count":2,"reviewCount":1},
    ]});
    let records = normalize_underlines(&value, UnderlineLimits::default()).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].range, "42");
    assert_eq!(records[0].text, "\u{4e2d}\u{6587}");
    assert_eq!((records[0].count, records[0].thoughts), (7, 3));
    assert_eq!(records[1].text, "123");
    WeReadClient::validate_underlines(&records).unwrap();
}

#[test]
fn exact_aggregate_admission_and_one_more_byte_use_the_same_normalizer() {
    let exact = json!({"underlines":[{"range":"a","text":"abc"},{"range":"b","text":"def"}]});
    assert_eq!(
        normalize_underlines(&exact, small_limits()).unwrap().len(),
        2
    );
    let excess = json!({"underlines":[{"range":"a","text":"abc"},{"range":"b","text":"defg"}]});
    assert_resource_error(normalize_underlines(&excess, small_limits()));
}

#[test]
fn record_and_selected_field_budgets_reject_before_mapping() {
    assert_resource_error(normalize_underlines(
        &json!({"underlines":[{}, {}, {}]}),
        small_limits(),
    ));
    assert_resource_error(normalize_underlines(
        &json!({"underlines":[{"range":"abcd","text":"a"}]}),
        small_limits(),
    ));
    assert_resource_error(normalize_underlines(
        &json!({"underlines":[{"range":"a","text":"\u{4e2d}\u{6587}"}]}),
        small_limits(),
    ));
    // An unused fallback is neither selected nor cloned by the real normalizer.
    let unused =
        json!({"underlines":[{"range":"a","markText":"x","content":"ignored oversized fallback"}]});
    assert_eq!(
        normalize_underlines(&unused, small_limits()).unwrap()[0].text,
        "x"
    );
}

#[test]
fn cached_underline_lists_receive_the_same_input_policy() {
    let records = [RemoteUnderline {
        text: "a".repeat(MAX_UNDERLINE_QUOTE_BYTES + 1),
        ..Default::default()
    }];
    assert_resource_error(WeReadClient::validate_underlines(&records));
    let records = vec![RemoteUnderline::default(); MAX_UNDERLINE_RECORDS + 1];
    assert_resource_error(WeReadClient::validate_underlines(&records));
}

struct RecordedRequest {
    path: String,
    authorization: String,
    cookies: Option<String>,
    body: Value,
}

struct Fixture {
    address: SocketAddr,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<std::result::Result<Option<RecordedRequest>, String>>>,
}

impl Fixture {
    fn new(body: Value) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let cancelled = stopped.clone();
        let body = body.to_string();
        assert!(body.len() <= 256 * 1024);
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(6);
            while !cancelled.load(Ordering::Acquire) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_millis(50)))
                            .map_err(|error| error.to_string())?;
                        stream
                            .set_write_timeout(Some(Duration::from_millis(500)))
                            .map_err(|error| error.to_string())?;
                        let request = read_request(&mut stream, &cancelled, deadline)?;
                        let response = format!(
                            "HTTP/1.1 200 Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nSet-Cookie: wr_skey=synthetic-unused-cookie\r\n\r\n{body}",
                            body.len(),
                        );
                        stream
                            .write_all(response.as_bytes())
                            .map_err(|error| error.to_string())?;
                        return Ok(Some(request));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            Ok(None)
        });
        Self {
            address,
            stopped,
            worker: Some(worker),
        }
    }

    fn client(&self) -> WeReadClient {
        let mut client = WeReadClient::new(AuthSession {
            api_key: "synthetic-underline-api-key".into(),
            user_vid: "synthetic-underline-account".into(),
            cookies: BTreeMap::from([
                ("wr_vid".into(), "synthetic-underline-account".into()),
                ("wr_skey".into(), "synthetic-existing-cookie".into()),
            ]),
            ..Default::default()
        })
        .unwrap();
        client.test_gateway_url = Some(format!("http://{}/api/agent/gateway", self.address));
        client.http = Client::builder()
            .no_proxy()
            .resolve("i.weread.qq.com", self.address)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_millis(300))
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        client
    }

    fn finish(mut self) -> RecordedRequest {
        self.worker
            .take()
            .unwrap()
            .join()
            .expect("The underline fixture worker panicked")
            .expect("The underline fixture transport failed")
            .expect("The underline fixture received no request")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            // Accept, reads, writes and cancellation all have finite bounds.
            let _ = worker.join();
        }
    }
}

fn read_request(
    stream: &mut TcpStream,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> std::result::Result<RecordedRequest, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 2048];
    let mut body_end = None;
    loop {
        if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err("The underline fixture request timed out".into());
        }
        match stream.read(&mut buffer) {
            Ok(0) => return Err("The underline fixture request ended early".into()),
            Ok(count) => {
                if bytes.len().saturating_add(count) > 16 * 1024 {
                    return Err("The underline fixture request exceeded its byte limit".into());
                }
                bytes.extend_from_slice(&buffer[..count]);
                if body_end.is_none() {
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let header = std::str::from_utf8(&bytes[..end])
                            .map_err(|error| error.to_string())?;
                        let length = header
                            .lines()
                            .filter_map(|line| line.split_once(':'))
                            .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                            .ok_or("Missing underline request length")?
                            .1
                            .trim()
                            .parse::<usize>()
                            .map_err(|error| error.to_string())?;
                        if length > 16 * 1024 - end - 4 {
                            return Err("The underline request body exceeded its limit".into());
                        }
                        body_end = Some((end + 4, end + 4 + length));
                    }
                }
                if let Some((start, end)) = body_end.filter(|(_, end)| bytes.len() >= *end) {
                    let header = std::str::from_utf8(&bytes[..start - 4])
                        .map_err(|error| error.to_string())?;
                    let headers = header
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
                        .collect::<BTreeMap<_, _>>();
                    return Ok(RecordedRequest {
                        path: header
                            .lines()
                            .next()
                            .ok_or("Missing request line")?
                            .split_whitespace()
                            .nth(1)
                            .ok_or("Missing request path")?
                            .into(),
                        authorization: headers.get("authorization").cloned().unwrap_or_default(),
                        cookies: headers.get("cookie").cloned(),
                        body: serde_json::from_slice(&bytes[start..end])
                            .map_err(|error| error.to_string())?,
                    });
                }
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

async fn deliver(body: Value) -> Result<Vec<RemoteUnderline>> {
    let fixture = Fixture::new(body);
    let client = fixture.client();
    let previous = client.session();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        client.underlines("synthetic-book", "42"),
    )
    .await
    .expect("The underline request exceeded its deadline");
    assert_eq!(client.session(), previous);
    let request = fixture.finish();
    assert_eq!(request.path, "/api/agent/gateway");
    assert_eq!(request.authorization, "Bearer synthetic-underline-api-key");
    assert!(request.cookies.is_none());
    assert_eq!(request.body["api_name"], "/book/underlines");
    assert_eq!(request.body["bookId"], "synthetic-book");
    assert_eq!(request.body["chapterUid"], 42);
    result
}

#[tokio::test(flavor = "current_thread")]
async fn loopback_oversized_quote_is_rejected_without_revoking_the_account() {
    assert_resource_error(deliver(json!({"underlines":[{"range":"opaque","text":"x".repeat(MAX_UNDERLINE_QUOTE_BYTES + 1)}]})).await);
}

#[tokio::test(flavor = "current_thread")]
async fn loopback_excess_records_are_rejected_without_partial_results() {
    assert_resource_error(
        deliver(json!({"underlines":vec![json!({"text":"x"}); MAX_UNDERLINE_RECORDS + 1]})).await,
    );
}

#[tokio::test(flavor = "current_thread")]
async fn loopback_normal_unicode_response_preserves_the_gateway_contract() {
    let records = deliver(json!({"updated":[{"range":"opaque","markText":"\u{4e2d}\u{6587}","count":9,"reviewCount":2}]})).await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].text, "\u{4e2d}\u{6587}");
    assert_eq!((records[0].count, records[0].thoughts), (9, 2));
}
