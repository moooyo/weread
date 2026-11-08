use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Instant;

const GOLDEN_SHARD: &str = "DF1103CA0CD3800B1FA3F842EB367223aPGh0bWw+PJvGZHk+T4A+UmVhZGVyIGZpeHR1cwPPHC9mUjwvYm9keuP8L2h0bWw+";
const GOLDEN_HTML: &str = "<html><body><p>Reader fixture.</p></body></html>";

struct Reply {
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body: String,
}

#[derive(Debug)]
struct RequestRecord {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

struct Fixture {
    origin: String,
    worker: thread::JoinHandle<Vec<RequestRecord>>,
}

impl Fixture {
    fn new(replies: Vec<Reply>) -> Self {
        Self::new_with_hook(replies, |_, _| {})
    }

    fn new_with_hook<F>(replies: Vec<Reply>, mut before_reply: F) -> Self
    where
        F: FnMut(usize, &RequestRecord) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut requests = Vec::new();
            for reply in replies {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                Instant::now() < deadline,
                                "Missing synthetic chapter request"
                            );
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => panic!("Synthetic listener failed: {error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let request = read_request(&mut stream);
                before_reply(requests.len(), &request);
                requests.push(request);
                let mut headers = format!(
                    "HTTP/1.1 {} Fixture\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n",
                    reply.status,
                    reply.body.len(),
                );
                for (name, value) in reply.headers {
                    headers.push_str(&format!("{name}: {value}\r\n"));
                }
                headers.push_str("\r\n");
                stream.write_all(headers.as_bytes()).unwrap();
                stream.write_all(reply.body.as_bytes()).unwrap();
            }
            requests
        });
        Self { origin, worker }
    }

    fn client(&self) -> WeReadClient {
        let mut client = WeReadClient::new(AuthSession {
            api_key: "fixture-api-key".into(),
            user_vid: "fixture-account".into(),
            cookies: BTreeMap::from([("wr_skey".into(), "fixture-session".into())]),
            ..Default::default()
        })
        .unwrap();
        client.test_web_origin = Some(self.origin.clone());
        client.http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        client
    }

    fn finish(self) -> Vec<RequestRecord> {
        self.worker.join().unwrap()
    }
}

fn read_request(stream: &mut TcpStream) -> RequestRecord {
    let mut bytes = Vec::new();
    let mut chunk = [0; 2048];
    let header_end = loop {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "Incomplete synthetic HTTP request");
        bytes.extend_from_slice(&chunk[..count]);
        assert!(
            bytes.len() <= 65536,
            "Synthetic request exceeded fixture limit"
        );
        if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
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
    let content_length = headers
        .get("content-length")
        .map_or(0, |value| value.parse::<usize>().unwrap());
    assert!(header_end + content_length <= 65536);
    while bytes.len() < header_end + content_length {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "Incomplete synthetic HTTP request body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    RequestRecord {
        method,
        path,
        headers,
        body: bytes[header_end..header_end + content_length].to_vec(),
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn chapter_round(psvts: &str, intact: bool) -> Vec<Reply> {
    let state = format!(
        r#"<script>window.__INITIAL_STATE__ = {{"reader":{{"psvts":"{psvts}"}}}};</script>"#
    );
    let first = if intact {
        GOLDEN_SHARD.to_owned()
    } else {
        format!("{}{}", "0".repeat(32), &GOLDEN_SHARD[32..])
    };
    [state, first, String::new(), String::new()]
        .into_iter()
        .map(|body| Reply {
            status: 200,
            headers: vec![],
            body,
        })
        .collect()
}

#[test]
fn corrupt_chapter_retry_refreshes_reader_state_and_refetches_every_shard() {
    let mut replies = chapter_round("fixture-first-clock", false);
    replies.extend(chapter_round("fixture-second-clock", true));
    let fixture = Fixture::new(replies);
    let client = fixture.client();
    let content = runtime()
        .block_on(client.chapter("fixture-book", "fixture-chapter"))
        .unwrap();
    assert_eq!(content.html, GOLDEN_HTML);
    assert_eq!(content.plain_text, "Reader fixture.");
    let requests = fixture.finish();
    assert_eq!(requests.len(), 8);
    for (round, expected_ps) in ["fixture-first-clock", "fixture-second-clock"]
        .iter()
        .enumerate()
    {
        let requests = &requests[round * 4..round * 4 + 4];
        assert_eq!(requests[0].method, "GET");
        assert!(requests[0].path.starts_with("/web/reader/"));
        assert_eq!(requests[1].path, "/web/book/chapter/e_0");
        assert_eq!(
            requests[2..]
                .iter()
                .map(|request| request.path.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["/web/book/chapter/e_1", "/web/book/chapter/e_3"]),
        );
        for request in &requests[1..] {
            assert_eq!(request.method, "POST");
            let payload: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(payload["ps"].as_str(), Some(*expected_ps));
        }
    }
}

#[test]
fn persistently_corrupt_chapters_stop_after_three_complete_read_attempts() {
    let mut replies = Vec::new();
    for state in [
        "fixture-clock-one",
        "fixture-clock-two",
        "fixture-clock-three",
    ] {
        replies.extend(chapter_round(state, false));
    }
    let fixture = Fixture::new(replies);
    let client = fixture.client();
    let result = runtime().block_on(client.chapter("fixture-book", "fixture-chapter"));
    assert!(matches!(result, Err(SourceError::Integrity)));
    assert!(!SourceError::Integrity.retryable());
    let requests = fixture.finish();
    assert_eq!(requests.len(), 12);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path.starts_with("/web/reader/"))
            .count(),
        3
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/web/book/chapter/e_0")
            .count(),
        3
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/web/book/chapter/e_1")
            .count(),
        3
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/web/book/chapter/e_3")
            .count(),
        3
    );
}

#[test]
fn rejected_authentication_does_not_repeat_the_whole_chapter() {
    for status in [401, 403] {
        let mut replies = vec![Reply {
            status,
            headers: vec![],
            body: "{}".into(),
        }];
        if status == 401 {
            replies.push(Reply {
                status: 401,
                headers: vec![],
                body: "{}".into(),
            });
        }
        let fixture = Fixture::new(replies);
        let client = fixture.client();
        let result = runtime().block_on(client.chapter("fixture-book", "fixture-chapter"));
        assert!(matches!(result, Err(SourceError::Http(code)) if code == status));
        let requests = fixture.finish();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path.starts_with("/web/reader/"))
                .count(),
            1
        );
        if status == 401 {
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[1].path, "/web/login/renewal");
        } else {
            assert_eq!(requests.len(), 1);
        }
    }
}

#[test]
fn chapter_retry_policy_is_local_bounded_and_excludes_permanent_failures() {
    for error in [
        SourceError::Integrity,
        SourceError::Api("fixture-transient".into()),
        SourceError::Network,
        SourceError::Timeout,
        SourceError::Http(429),
        SourceError::Http(503),
        SourceError::InvalidResponse("reader_session"),
        SourceError::InvalidResponse("empty_chapter"),
        SourceError::InvalidResponse("chapter_shard_json"),
        SourceError::InvalidResponse("chapter_base64"),
        SourceError::InvalidResponse("chapter_utf8"),
        SourceError::InvalidResponse("utf8"),
    ] {
        assert!(
            retry_chapter_read(&error),
            "Expected bounded chapter retry for {error}"
        );
    }
    for error in [
        SourceError::ApiKeyRequired,
        SourceError::WebLoginRequired,
        SourceError::LoginExpired,
        SourceError::Api("-2010".into()),
        SourceError::Api("-2012".into()),
        SourceError::Api("-2013".into()),
        SourceError::Http(401),
        SourceError::Http(403),
        SourceError::Http(404),
        SourceError::ResourceHttp(503),
        SourceError::Unsupported("fixture-permanent"),
        SourceError::ResponseTooLarge,
        SourceError::InvalidResponse("resource_url"),
        SourceError::CheckpointFailed,
    ] {
        assert!(
            !retry_chapter_read(&error),
            "Unexpected chapter retry for {error}"
        );
    }
}

fn business_error(code: &str) -> Reply {
    Reply {
        status: 200,
        headers: vec![],
        body: json!({"errCode": code.parse::<i64>().unwrap(), "errMsg": "fixture private message"})
            .to_string(),
    }
}

fn acknowledged_renewal() -> Reply {
    Reply {
        status: 200,
        headers: vec![
            (
                "Set-Cookie",
                "wr_skey=fixture-renewed-session; Path=/; HttpOnly",
            ),
            (
                "Set-Cookie",
                "wr_rt=fixture-renewed-refresh; Path=/; HttpOnly",
            ),
        ],
        body: r#"{"succ":1}"#.into(),
    }
}

fn assert_shard_clock(request: &RequestRecord, expected: &str) {
    assert_eq!(request.method, "POST");
    let payload: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(payload["ps"].as_str(), Some(expected));
}

#[test]
fn business_expiry_in_a_shard_restarts_the_reader_and_all_shards() {
    for failed_shard in [1, 2] {
        let mut replies = chapter_round("fixture-before-renewal", true);
        // A parallel pair drains both replies before starting renewal.
        let initial_reads = if failed_shard == 1 { 2 } else { 4 };
        replies.truncate(initial_reads);
        replies[failed_shard] = business_error("-2012");
        replies.push(acknowledged_renewal());
        replies.extend(chapter_round("fixture-after-renewal", true));
        let fixture = Fixture::new(replies);
        let client = fixture.client();
        let content = runtime()
            .block_on(client.chapter("fixture-book", "fixture-chapter"))
            .unwrap();
        assert_eq!(content.html, GOLDEN_HTML);
        let requests = fixture.finish();
        assert_eq!(requests.len(), initial_reads + 5);
        assert_eq!(requests[initial_reads].path, "/web/login/renewal");
        let new_reader = initial_reads + 1;
        assert_eq!(requests[new_reader].method, "GET");
        assert_eq!(requests[new_reader].path, requests[0].path);
        assert!(requests[new_reader].headers["cookie"].contains("fixture-renewed-session"));
        for request in &requests[1..initial_reads] {
            assert_shard_clock(request, "fixture-before-renewal");
        }
        assert_eq!(requests[new_reader + 1].path, "/web/book/chapter/e_0");
        assert_eq!(
            requests[new_reader + 2..]
                .iter()
                .map(|request| request.path.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["/web/book/chapter/e_1", "/web/book/chapter/e_3"]),
        );
        for request in &requests[new_reader + 1..] {
            assert_shard_clock(request, "fixture-after-renewal");
            assert!(request.headers["cookie"].contains("fixture-renewed-session"));
        }
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/web/login/renewal")
                .count(),
            1
        );
    }
}

#[test]
fn reader_business_expiry_reloads_the_reader_before_fetching_shards() {
    let mut replies = vec![business_error("-2010"), acknowledged_renewal()];
    replies.extend(
        chapter_round("fixture-unbound-reader-clock", true)
            .into_iter()
            .take(1),
    );
    replies.extend(chapter_round("fixture-renewed-reader-clock", true));
    let fixture = Fixture::new(replies);
    let client = fixture.client();
    let content = runtime()
        .block_on(client.chapter("fixture-book", "fixture-chapter"))
        .unwrap();
    assert_eq!(content.html, GOLDEN_HTML);
    assert_eq!(
        content.content_status,
        crate::domain::ContentStatus::Unknown
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 7);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[1].path, "/web/login/renewal");
    assert_eq!(requests[2].method, "GET");
    assert_eq!(requests[2].path, requests[0].path);
    assert_eq!(requests[3].method, "GET");
    assert_eq!(requests[3].path, requests[0].path);
    for request in &requests[4..] {
        assert_shard_clock(request, "fixture-renewed-reader-clock");
        assert!(request.headers["cookie"].contains("fixture-renewed-session"));
    }
}

#[test]
fn repeated_business_expiry_after_reader_restart_is_a_final_failure() {
    let mut replies = chapter_round("fixture-original-reader-clock", true);
    replies.truncate(2);
    replies[1] = business_error("-2012");
    replies.push(acknowledged_renewal());
    let mut second = chapter_round("fixture-restarted-reader-clock", true);
    second.truncate(2);
    second[1] = business_error("-2012");
    replies.extend(second);
    let fixture = Fixture::new(replies);
    let client = fixture.client();
    let result = runtime().block_on(client.chapter("fixture-book", "fixture-chapter"));
    assert!(matches!(result, Err(SourceError::Api(code)) if code == "-2012"));
    let requests = fixture.finish();
    assert_eq!(requests.len(), 5);
    assert_eq!(requests[2].path, "/web/login/renewal");
    assert_eq!(requests[3].method, "GET");
    assert_shard_clock(&requests[4], "fixture-restarted-reader-clock");
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/web/login/renewal")
            .count(),
        1
    );
}

#[test]
fn a_business_expiry_from_renewal_cannot_recurse_or_commit_cookies() {
    let mut replies = chapter_round("fixture-before-failed-renewal", true);
    replies.truncate(2);
    let mut rejected = business_error("-2012");
    rejected
        .headers
        .push(("Set-Cookie", "wr_skey=fixture-rejected-session; Path=/"));
    replies[1] = rejected;
    let mut renewal = business_error("-2012");
    renewal
        .headers
        .push(("Set-Cookie", "wr_skey=fixture-failed-renewal; Path=/"));
    replies.push(renewal);
    let fixture = Fixture::new(replies);
    let client = fixture.client();
    let before = serde_json::to_value(client.session()).unwrap();
    let result = runtime().block_on(client.chapter("fixture-book", "fixture-chapter"));
    assert!(matches!(result, Err(SourceError::Api(code)) if code == "-2012"));
    assert_eq!(serde_json::to_value(client.session()).unwrap(), before);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2].path, "/web/login/renewal");
    assert!(requests[2].headers["cookie"].contains("wr_skey=fixture-session"));
}

#[test]
fn identity_error_without_a_verified_renewal_contract_is_not_replayed() {
    for rejected_step in [0, 1] {
        let mut replies = chapter_round("fixture-final-identity-error", true);
        replies.truncate(rejected_step + 1);
        replies[rejected_step] = business_error("-2013");
        let fixture = Fixture::new(replies);
        let client = fixture.client();
        let result = runtime().block_on(client.chapter("fixture-book", "fixture-chapter"));
        assert!(matches!(result, Err(SourceError::Api(code)) if code == "-2013"));
        let requests = fixture.finish();
        assert_eq!(requests.len(), rejected_step + 1);
        assert!(
            !requests
                .iter()
                .any(|request| request.path == "/web/login/renewal")
        );
    }
}

#[test]
fn reader_and_shards_share_one_business_authentication_recovery_budget() {
    let mut replies = vec![business_error("-2010"), acknowledged_renewal()];
    replies.extend(
        chapter_round("fixture-unbound-reader-clock", true)
            .into_iter()
            .take(1),
    );
    let mut after = chapter_round("fixture-only-renewed-reader-clock", true);
    after.truncate(2);
    after[1] = business_error("-2012");
    replies.extend(after);
    let fixture = Fixture::new(replies);
    let client = fixture.client();
    let result = runtime().block_on(client.chapter("fixture-book", "fixture-chapter"));
    assert!(matches!(result, Err(SourceError::Api(code)) if code == "-2012"));
    let requests = fixture.finish();
    assert_eq!(requests.len(), 5);
    assert_eq!(requests[1].path, "/web/login/renewal");
    assert_eq!(requests[2].method, "GET");
    assert_eq!(requests[3].method, "GET");
    assert_shard_clock(&requests[4], "fixture-only-renewed-reader-clock");
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/web/login/renewal")
            .count(),
        1
    );
}

#[test]
fn unsuccessful_renewal_cannot_fall_back_to_temporary_chapter_retries() {
    for (mut renewal, expected_error) in [
        (
            Reply {
                status: 200,
                headers: vec![],
                body: r#"{"succ":0}"#.into(),
            },
            "source.api.rejected",
        ),
        (business_error("-9999"), "source.api.-9999"),
        (
            Reply {
                status: 200,
                headers: vec![],
                body: "{}".into(),
            },
            "source.invalid_response.renewal_acknowledgement",
        ),
    ] {
        renewal.headers.push((
            "Set-Cookie",
            "wr_skey=fixture-unacknowledged-session; Path=/",
        ));
        let mut replies = chapter_round("fixture-before-unacknowledged-renewal", true);
        replies.truncate(2);
        replies[1] = business_error("-2012");
        replies.push(renewal);
        let fixture = Fixture::new(replies);
        let client = fixture.client();
        let before = serde_json::to_value(client.session()).unwrap();
        let error = runtime()
            .block_on(client.chapter("fixture-book", "fixture-chapter"))
            .unwrap_err();
        assert_eq!(error.to_string(), expected_error);
        assert_eq!(serde_json::to_value(client.session()).unwrap(), before);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].method, "GET");
        assert_shard_clock(&requests[1], "fixture-before-unacknowledged-renewal");
        assert_eq!(requests[2].path, "/web/login/renewal");
        assert!(requests[2].headers["cookie"].contains("wr_skey=fixture-session"));
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path.starts_with("/web/reader/"))
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/web/login/renewal")
                .count(),
            1
        );
    }
}

fn reader_with_access(psvts: &str, chapter_uid: &str) -> Reply {
    let state = json!({
        "user": { "vid": "fixture-account" },
        "reader": {
            "bookId": "fixture-book",
            "bookInfo": { "bookId": "fixture-book", "paid": 1, "payType": 1 },
            "chapterInfos": [{ "chapterUid": chapter_uid, "chapterIdx": 1, "wordCount": 20 }],
            "currentChapter": { "chapterUid": chapter_uid },
            "psvts": psvts,
            "pclts": "fixture-client-clock"
        }
    });
    Reply {
        status: 200,
        headers: vec![],
        body: format!("<script>window.__INITIAL_STATE__={state};</script>"),
    }
}

fn chapter_with_access(psvts: &str, chapter_uid: &str) -> Vec<Reply> {
    let mut replies = vec![reader_with_access(psvts, chapter_uid)];
    replies.extend(chapter_round(psvts, true).into_iter().skip(1));
    replies
}

#[test]
fn reader_auth_recovery_requires_one_stable_access_fetch_before_any_shard() {
    for rejected in [
        Reply {
            status: 401,
            headers: vec![],
            body: "{}".into(),
        },
        business_error("-2012"),
    ] {
        let mut replies = vec![
            rejected,
            acknowledged_renewal(),
            reader_with_access("fixture-unbound-access-clock", "fixture-chapter"),
        ];
        replies.extend(chapter_with_access(
            "fixture-stable-access-clock",
            "fixture-chapter",
        ));
        let fixture = Fixture::new(replies);
        let client = fixture.client();
        let content = runtime()
            .block_on(client.chapter("fixture-book", "fixture-chapter"))
            .unwrap();
        assert_eq!(content.content_status, crate::domain::ContentStatus::Full);
        assert_eq!(content.html, GOLDEN_HTML);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 7);
        assert_eq!(requests[1].path, "/web/login/renewal");
        for index in [0, 2, 3] {
            assert_eq!(requests[index].method, "GET");
            assert_eq!(requests[index].path, requests[0].path);
        }
        assert_eq!(requests[4].path, "/web/book/chapter/e_0");
        assert_eq!(
            requests[5..]
                .iter()
                .map(|request| request.path.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["/web/book/chapter/e_1", "/web/book/chapter/e_3"])
        );
        for request in &requests[4..] {
            assert_shard_clock(request, "fixture-stable-access-clock");
            assert!(request.headers["cookie"].contains("fixture-renewed-session"));
        }
    }
}

#[test]
fn access_refresh_cannot_spend_an_already_used_auth_recovery_budget() {
    let fixture = Fixture::new(vec![
        business_error("-2012"),
        acknowledged_renewal(),
        reader_with_access("fixture-unbound-access-clock", "fixture-chapter"),
        business_error("-2012"),
    ]);
    let client = fixture.client();
    let result = runtime().block_on(client.chapter("fixture-book", "fixture-chapter"));
    assert!(matches!(result, Err(SourceError::Api(code)) if code == "-2012"));
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[1].path, "/web/login/renewal");
    assert_eq!(requests[2].method, "GET");
    assert_eq!(requests[3].method, "GET");
    assert_eq!(requests[3].path, requests[0].path);
    assert!(
        !requests
            .iter()
            .any(|request| request.path.starts_with("/web/book/chapter/"))
    );
}

#[test]
fn repeatedly_changing_reader_generation_stops_before_fetching_shards() {
    let generation = std::sync::Arc::new(tokio::sync::Mutex::new(0u64));
    let server_generation = generation.clone();
    let fixture = Fixture::new_with_hook(
        vec![
            reader_with_access("fixture-first-unstable-clock", "fixture-chapter"),
            reader_with_access("fixture-second-unstable-clock", "fixture-chapter"),
        ],
        move |_, request| {
            if request.path.starts_with("/web/reader/") {
                // This models another acknowledged session generation while
                // the Reader response is in flight, without a real account.
                let mut generation = server_generation.blocking_lock();
                *generation = generation.wrapping_add(1);
            }
        },
    );
    let mut client = fixture.client();
    client.renewal = generation;
    let result = runtime().block_on(client.chapter("fixture-book", "fixture-chapter"));
    assert!(matches!(result, Err(SourceError::ContentUnverified)));
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.method == "GET"));
    assert_eq!(requests[0].path, requests[1].path);
}

#[test]
fn stable_unknown_access_does_not_trigger_an_additional_reader_fetch() {
    let state = json!({
        "reader": {
            "psvts": "fixture-stable-unknown-clock",
            "pclts": "fixture-client-clock"
        }
    });
    let mut replies = chapter_round("fixture-stable-unknown-clock", true);
    replies[0] = Reply {
        status: 200,
        headers: vec![],
        body: format!("<script>window.__INITIAL_STATE__={state};</script>"),
    };
    let fixture = Fixture::new(replies);
    let client = fixture.client();
    let content = runtime()
        .block_on(client.chapter("fixture-book", "fixture-chapter"))
        .unwrap();
    assert_eq!(
        content.content_status,
        crate::domain::ContentStatus::Unknown
    );
    assert_eq!(content.html, GOLDEN_HTML);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "GET")
            .count(),
        1
    );
    for request in &requests[1..] {
        assert_shard_clock(request, "fixture-stable-unknown-clock");
    }
}

#[test]
fn access_refresh_budget_is_local_to_each_chapter_when_hints_are_reused() {
    let generation = std::sync::Arc::new(tokio::sync::Mutex::new(0u64));
    let server_generation = generation.clone();
    let mut replies = vec![reader_with_access(
        "fixture-first-unbound",
        "fixture-chapter",
    )];
    replies.extend(chapter_with_access(
        "fixture-first-stable",
        "fixture-chapter",
    ));
    replies.push(reader_with_access(
        "fixture-next-unbound",
        "fixture-next-chapter",
    ));
    replies.extend(chapter_with_access(
        "fixture-next-stable",
        "fixture-next-chapter",
    ));
    let mut reader_fetches = 0;
    let fixture = Fixture::new_with_hook(replies, move |_, request| {
        if request.path.starts_with("/web/reader/") {
            reader_fetches += 1;
            if matches!(reader_fetches, 1 | 3) {
                let mut generation = server_generation.blocking_lock();
                *generation = generation.wrapping_add(1);
            }
        }
    });
    let mut client = fixture.client();
    client.renewal = generation;
    let mut hints = ChapterReadHints::default();
    let (first, next) = runtime().block_on(async {
        let first = client
            .chapter_with_hints("fixture-book", "fixture-chapter", &mut hints)
            .await
            .unwrap();
        let next = client
            .chapter_with_hints("fixture-book", "fixture-next-chapter", &mut hints)
            .await
            .unwrap();
        (first, next)
    });
    assert_eq!(first.content_status, crate::domain::ContentStatus::Full);
    assert_eq!(next.content_status, crate::domain::ContentStatus::Full);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 10);
    for index in [0, 1, 5, 6] {
        assert_eq!(requests[index].method, "GET");
    }
    for request in &requests[2..5] {
        assert_shard_clock(request, "fixture-first-stable");
    }
    for request in &requests[7..] {
        assert_shard_clock(request, "fixture-next-stable");
    }
    assert!(
        !requests
            .iter()
            .any(|request| request.path == "/web/login/renewal")
    );
}

#[test]
fn purchased_and_member_chapters_keep_full_access_with_a_zero_ssr_client_clock() {
    for purchased in [true, false] {
        let mut reader = reader_with_access("fixture-server-clock", "fixture-chapter");
        let state_text = reader
            .body
            .strip_prefix("<script>window.__INITIAL_STATE__=")
            .unwrap()
            .strip_suffix(";</script>")
            .unwrap();
        let mut state: Value = serde_json::from_str(state_text).unwrap();
        state["reader"]["pclts"] = json!(0);
        state["reader"]["token"] = json!("");
        if !purchased {
            state["reader"]["bookInfo"] = json!({
                "bookId": "fixture-book",
                "paid": 0,
                "payType": 1,
                "payingStatus": 1,
                "maxFreeChapter": 0,
                "shortTimeRead": { "active": 0 }
            });
            let chapter = json!({
                "chapterUid": "fixture-chapter",
                "chapterIdx": 1,
                "wordCount": 20,
                "price": 10,
                "paid": 0
            });
            state["reader"]["chapterInfos"] = json!([chapter.clone()]);
            state["reader"]["currentChapter"] = chapter;
            state["reader"]["memberCardSummary"] = json!({
                "isPaying": 0,
                "expired": 0,
                "expiredTime": super::super::protocol::now_seconds() + 600,
                "freeBookIds": []
            });
        }
        reader.body = format!("<script>window.__INITIAL_STATE__={state};</script>");
        let mut replies = vec![reader];
        replies.extend(
            chapter_round("fixture-server-clock", true)
                .into_iter()
                .skip(1),
        );
        let fixture = Fixture::new(replies);
        let client = fixture.client();
        let content = runtime()
            .block_on(client.chapter("fixture-book", "fixture-chapter"))
            .unwrap();
        assert_eq!(content.content_status, crate::domain::ContentStatus::Full);
        assert_eq!(content.html, GOLDEN_HTML);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[0].method, "GET");
        assert!(requests[0].path.starts_with("/web/reader/"));
        assert_eq!(requests[1].path, "/web/book/chapter/e_0");
        assert_eq!(
            requests[2..]
                .iter()
                .map(|request| request.path.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["/web/book/chapter/e_1", "/web/book/chapter/e_3"])
        );
        for request in &requests[1..] {
            assert_shard_clock(request, "fixture-server-clock");
        }
    }
}
