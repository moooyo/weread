use super::super::client::{MAX_COOKIE_COUNT, MAX_COOKIE_ENTRY_BYTES, response_json};
use super::*;
use reqwest::header::{HeaderMap, HeaderValue, SET_COOKIE};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[path = "auth_session_budget_web_tests.rs"]
mod auth_session_budget_tests;

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: &'static str,
}

impl Reply {
    fn json(status: u16, body: &'static str) -> Self {
        Self {
            status,
            headers: vec![],
            body,
        }
    }

    fn rotated(mut self) -> Self {
        self.headers.extend(
            [
                (
                    "Set-Cookie",
                    "wr_skey=fixture-renewed-key; Path=/; HttpOnly",
                ),
                (
                    "Set-Cookie",
                    "wr_rt=fixture-renewed-refresh; Path=/; HttpOnly",
                ),
                ("x-wr-ticket", "fixture-renewed-ticket"),
                ("x-wrpa-0", "fixture-renewed-wrpa"),
            ]
            .map(|(name, value)| (name, value.to_owned())),
        );
        self
    }

    fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

struct Fixture {
    origin: String,
    records: Arc<Mutex<Vec<(usize, RecordedRequest)>>>,
    worker: thread::JoinHandle<()>,
}

impl Fixture {
    fn scripted(replies: Vec<Reply>) -> Self {
        Self::serve(replies.len(), move |_, index| replies[index].clone())
    }

    fn serve<F>(expected: usize, handler: F) -> Self
    where
        F: Fn(&RecordedRequest, usize) -> Reply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let records = Arc::new(Mutex::new(Vec::new()));
        let captured = records.clone();
        let handler = Arc::new(handler);
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut workers = Vec::new();
            while workers.len() < expected {
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
                            let reply = handler(&request, index);
                            let mut response = format!(
                                "HTTP/1.1 {} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                                reply.status,
                                reply.body.len(),
                            );
                            for (name, value) in reply.headers {
                                response.push_str(&format!("{name}: {value}\r\n"));
                            }
                            response.push_str("\r\n");
                            response.push_str(reply.body);
                            stream.write_all(response.as_bytes()).unwrap();
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "Missing synthetic HTTP request");
                        thread::sleep(Duration::from_millis(2));
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
            records,
            worker,
        }
    }

    fn client(&self) -> WeReadClient {
        let mut client = WeReadClient::new(fixture_auth()).unwrap();
        client.test_web_origin = Some(self.origin.clone());
        client.http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        client
    }

    fn finish(self) -> Vec<RecordedRequest> {
        self.worker.join().unwrap();
        let mut records = self.records.lock().unwrap().clone();
        records.sort_by_key(|(index, _)| *index);
        records.into_iter().map(|(_, record)| record).collect()
    }
}

fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    let mut bytes = Vec::new();
    let mut chunk = [0; 2048];
    let header_end = loop {
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "Incomplete synthetic HTTP request");
        bytes.extend_from_slice(&chunk[..read]);
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
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "Incomplete synthetic HTTP body");
        bytes.extend_from_slice(&chunk[..read]);
    }
    RecordedRequest {
        method,
        path,
        headers,
        body: bytes[header_end..header_end + content_length].to_vec(),
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

async fn get_json(client: &WeReadClient, path: &str) -> Result<Value> {
    let request = client.web_request(Method::GET, path)?;
    response_json(client.web_response(request, true).await?).await
}

#[test]
fn ordinary_responses_rotate_cookies_for_the_next_shared_client_request() {
    let fixture = Fixture::scripted(vec![
        Reply::json(200, "{}").rotated(),
        Reply::json(200, "{}"),
    ]);
    let client = fixture.client();
    let shared = client.clone();
    runtime().block_on(async {
        get_json(&client, "/web/reader/fixture-first")
            .await
            .unwrap();
        get_json(&shared, "/web/reader/fixture-second")
            .await
            .unwrap();
    });
    let requests = fixture.finish();
    assert!(requests[0].headers["cookie"].contains("wr_skey=fixture-original-key"));
    assert!(requests[1].headers["cookie"].contains("wr_skey=fixture-renewed-key"));
    assert!(requests[1].headers["cookie"].contains("wr_rt=fixture-renewed-refresh"));
    assert_eq!(requests[1].headers["x-wr-ticket"], "fixture-renewed-ticket");
    assert_eq!(requests[1].headers["x-wrpa-0"], "fixture-renewed-wrpa");
}

#[test]
fn unauthorized_read_renews_once_and_replays_its_original_body_with_new_auth() {
    let mut rejected = Reply::json(401, "{}");
    rejected
        .headers
        .push(("Set-Cookie", "wr_rt=; Path=/".into()));
    let fixture = Fixture::scripted(vec![
        rejected,
        Reply::json(200, r#"{"succ":1}"#).rotated(),
        Reply::json(200, r#"{"chapter":"fixture"}"#),
    ]);
    let client = fixture.client();
    let payload = json!({"fixture": "chapter-body"});
    runtime().block_on(async {
        let request = client
            .web_request(Method::POST, "/web/book/chapter/e_0")
            .unwrap()
            .header("Referer", "https://weread.qq.com/web/reader/fixture")
            .json(&payload);
        let value = response_json(client.web_response(request, true).await.unwrap())
            .await
            .unwrap();
        assert_eq!(value["chapter"], "fixture");
    });
    let requests = fixture.finish();
    assert_eq!(requests[1].path, "/web/login/renewal");
    assert!(requests[1].headers["cookie"].contains("wr_rt=fixture-original-refresh"));
    let renewal: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(renewal, json!({"rq":"%2Fweb%2Fbook%2Fread","ql":false}));
    assert_eq!(requests[0].method, requests[2].method);
    assert_eq!(requests[0].path, requests[2].path);
    assert_eq!(requests[0].body, requests[2].body);
    assert_eq!(
        requests[0].headers["referer"],
        requests[2].headers["referer"]
    );
    assert!(requests[2].headers["cookie"].contains("wr_skey=fixture-renewed-key"));
    assert_eq!(requests[2].headers["x-wr-ticket"], "fixture-renewed-ticket");
    assert_eq!(requests[2].headers["x-wrpa-0"], "fixture-renewed-wrpa");
}

#[test]
fn failed_or_unacknowledged_renewal_never_commits_response_credentials() {
    for (status, body) in [
        (401, "{}"),
        (200, r#"{"succ":0}"#),
        (200, r#"{"errCode":123}"#),
        (200, "{}"),
        (200, "not-json"),
    ] {
        let fixture = Fixture::scripted(vec![Reply::json(status, body).rotated()]);
        let client = fixture.client();
        let before = serde_json::to_value(client.session()).unwrap();
        runtime().block_on(async {
            assert!(client.renew().await.is_err());
        });
        assert_eq!(serde_json::to_value(client.session()).unwrap(), before);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/web/login/renewal");
    }
}

#[test]
fn a_second_unauthorized_response_stops_after_one_renewal() {
    let fixture = Fixture::scripted(vec![
        Reply::json(401, "{}"),
        Reply::json(200, r#"{"succ":1}"#).rotated(),
        Reply::json(401, "{}"),
    ]);
    let client = fixture.client();
    let result = runtime().block_on(get_json(&client, "/web/reader/fixture"));
    assert!(matches!(result, Err(SourceError::Http(401))));
    let requests = fixture.finish();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/web/login/renewal")
            .count(),
        1
    );
}

#[test]
fn concurrent_unauthorized_reads_share_one_acknowledged_renewal() {
    let arrivals = Arc::new((Mutex::new(0), Condvar::new()));
    let fixture = Fixture::serve(5, move |request, _| {
        if request.path == "/web/login/renewal" {
            return Reply::json(200, r#"{"succ":1}"#).rotated();
        }
        if request.headers["cookie"].contains("fixture-original-key") {
            let (count, ready) = &*arrivals;
            let mut count = count.lock().unwrap();
            *count += 1;
            ready.notify_all();
            let (count, timeout) = ready
                .wait_timeout_while(count, Duration::from_secs(3), |count| *count < 2)
                .unwrap();
            assert!(
                !timeout.timed_out() && *count == 2,
                "Both original requests must reach the server"
            );
            return Reply::json(401, "{}");
        }
        assert!(request.headers["cookie"].contains("fixture-renewed-key"));
        Reply::json(200, "{}")
    });
    let client = fixture.client();
    runtime().block_on(async {
        let first = get_json(&client, "/web/reader/fixture-first");
        let second = get_json(&client, "/web/reader/fixture-second");
        let (first, second) = tokio::join!(first, second);
        first.unwrap();
        second.unwrap();
    });
    let requests = fixture.finish();
    assert_eq!(requests.len(), 5);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/web/login/renewal")
            .count(),
        1
    );
}

fn late_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.append(
        SET_COOKIE,
        HeaderValue::from_static("wr_skey=fixture-late-key; Path=/"),
    );
    headers.append(
        SET_COOKIE,
        HeaderValue::from_static("wr_new=fixture-late-cookie; Path=/"),
    );
    headers.insert(
        "x-wr-ticket",
        HeaderValue::from_static("fixture-late-ticket"),
    );
    headers
}

#[test]
fn late_ordinary_response_cannot_roll_back_acknowledged_renewal() {
    let fixture = Fixture::scripted(vec![Reply::json(200, r#"{"succ":1}"#).rotated()]);
    let client = fixture.client();
    let sent = client.session();
    runtime().block_on(async {
        client.renew().await.unwrap();
    });
    let renewed = serde_json::to_value(client.session()).unwrap();
    assert!(
        !client
            .commit_web_headers(&sent, &late_headers(), false)
            .unwrap()
    );
    assert_eq!(serde_json::to_value(client.session()).unwrap(), renewed);
    fixture.finish();
}

#[test]
fn responses_for_an_old_account_cannot_modify_a_replacement_account() {
    let client = WeReadClient::new(fixture_auth()).unwrap();
    let sent = client.session();
    let mut replacement = fixture_auth();
    replacement.user_vid = "fixture-other-account".into();
    replacement.api_key = "fixture-other-api-key".into();
    replacement
        .cookies
        .insert("wr_skey".into(), "fixture-other-key".into());
    client.replace_session(replacement).unwrap();
    let expected = serde_json::to_value(client.session()).unwrap();
    assert!(
        !client
            .commit_web_headers(&sent, &late_headers(), false)
            .unwrap()
    );
    assert!(
        !client
            .commit_web_headers(&sent, &late_headers(), true)
            .unwrap()
    );
    assert_eq!(serde_json::to_value(client.session()).unwrap(), expected);
}

#[test]
fn an_untrusted_response_cannot_update_web_credentials() {
    let fixture = Fixture::scripted(vec![Reply::json(200, "{}").rotated()]);
    let mut client = fixture.client();
    client.test_web_origin = Some("http://127.0.0.1:1".into());
    let sent = client.session();
    let expected = serde_json::to_value(&sent).unwrap();
    runtime().block_on(async {
        let response = client.http.get(&fixture.origin).send().await.unwrap();
        client.observe_web_response(&sent, &response).unwrap();
    });
    assert_eq!(serde_json::to_value(client.session()).unwrap(), expected);
    let requests = fixture.finish();
    assert!(!requests[0].headers.contains_key("cookie"));
}

#[test]
fn untrusted_web_requests_are_rejected_before_network_execution() {
    let client = WeReadClient::new(fixture_auth()).unwrap();
    let result = runtime().block_on(async {
        client
            .web_response(client.http.get("https://cdn.invalid/fixture"), true)
            .await
    });
    assert!(matches!(
        result,
        Err(SourceError::InvalidResponse("web_request_origin"))
    ));
}

#[test]
fn nonreplayable_operations_do_not_renew_or_repeat_after_unauthorized() {
    let fixture = Fixture::scripted(vec![Reply::json(401, "{}").rotated()]);
    let client = fixture.client();
    let expected = serde_json::to_value(client.session()).unwrap();
    let result = runtime().block_on(async {
        let request = client
            .web_request(Method::POST, "/web/book/read")
            .unwrap()
            .json(&json!({"fixture": "single-write"}));
        response_json(client.web_response(request, false).await.unwrap()).await
    });
    assert!(matches!(result, Err(SourceError::Http(401))));
    assert_eq!(serde_json::to_value(client.session()).unwrap(), expected);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/web/book/read");
}

const FIXTURE_CATALOG: &str = r#"{"data":[{"bookId":"fixture-book","updated":[{"chapterUid":"fixture-chapter","chapterIdx":1,"wordCount":1,"title":"Fixture chapter"}]}]}"#;

#[test]
fn catalog_business_auth_errors_renew_once_and_replay_the_same_read() {
    for expired in [
        r#"{"errCode":-2012,"errMsg":"fixture private message"}"#,
        r#"{"errCode":"-2010","errMsg":"fixture private message"}"#,
    ] {
        let mut rejected = Reply::json(200, expired);
        rejected.headers.extend(
            [
                ("Set-Cookie", "wr_rt=; Path=/"),
                ("Set-Cookie", "wr_skey=fixture-rejected-key; Path=/"),
            ]
            .map(|(name, value)| (name, value.to_owned())),
        );
        let fixture = Fixture::scripted(vec![
            rejected,
            Reply::json(200, r#"{"succ":1}"#).rotated(),
            Reply::json(200, FIXTURE_CATALOG),
        ]);
        let client = fixture.client();
        let chapters = runtime().block_on(client.catalog("fixture-book")).unwrap();
        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].uid, "fixture-chapter");
        let requests = fixture.finish();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].path, "/web/book/chapterInfos");
        assert_eq!(requests[1].path, "/web/login/renewal");
        assert!(requests[1].headers["cookie"].contains("wr_rt=fixture-original-refresh"));
        assert!(requests[1].headers["cookie"].contains("wr_skey=fixture-original-key"));
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[2].method, "POST");
        assert_eq!(requests[0].path, requests[2].path);
        assert_eq!(requests[0].body, requests[2].body);
        assert_eq!(
            requests[0].headers["referer"],
            requests[2].headers["referer"]
        );
        assert!(requests[2].headers["cookie"].contains("wr_skey=fixture-renewed-key"));
        assert_eq!(requests[2].headers["x-wr-ticket"], "fixture-renewed-ticket");
        assert_eq!(requests[2].headers["x-wrpa-0"], "fixture-renewed-wrpa");
    }
}

#[test]
fn concurrent_catalog_business_auth_errors_share_one_acknowledged_renewal() {
    for rotate in [false, true] {
        let arrivals = Arc::new((Mutex::new(0), Condvar::new()));
        let fixture = Fixture::serve(5, move |request, index| {
            if request.path == "/web/login/renewal" {
                return if rotate {
                    Reply::json(200, r#"{"succ":1}"#).rotated()
                } else {
                    Reply::json(200, r#"{"succ":1}"#)
                };
            }
            assert_eq!(request.path, "/web/book/chapterInfos");
            if index < 2 {
                assert!(request.headers["cookie"].contains("fixture-original-key"));
                let (count, ready) = &*arrivals;
                let mut count = count.lock().unwrap();
                *count += 1;
                ready.notify_all();
                let (count, timeout) = ready
                    .wait_timeout_while(count, Duration::from_secs(3), |count| *count < 2)
                    .unwrap();
                assert!(!timeout.timed_out() && *count == 2);
                return Reply::json(200, r#"{"errCode":-2012}"#);
            }
            assert!(request.headers["cookie"].contains(if rotate {
                "fixture-renewed-key"
            } else {
                "fixture-original-key"
            }));
            Reply::json(200, FIXTURE_CATALOG)
        });
        let client = fixture.client();
        runtime().block_on(async {
            let (first, second) = tokio::join!(
                client.catalog("fixture-book"),
                client.catalog("fixture-book")
            );
            assert_eq!(first.unwrap().len(), 1);
            assert_eq!(second.unwrap().len(), 1);
        });
        let requests = fixture.finish();
        assert_eq!(requests.len(), 5);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/web/login/renewal")
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/web/book/chapterInfos")
                .count(),
            4
        );
    }
}

#[test]
fn concurrent_failed_renewals_never_authorize_replaying_rejected_reads() {
    for failed in [
        Reply::json(401, "{}"),
        Reply::json(200, r#"{"errCode":-2012}"#),
        Reply::json(200, r#"{"succ":0}"#),
    ] {
        let arrivals = Arc::new((Mutex::new(0), Condvar::new()));
        let fixture = Fixture::serve(4, move |request, index| {
            if request.path == "/web/login/renewal" {
                return failed.clone().rotated();
            }
            assert_eq!(request.path, "/web/book/chapterInfos");
            assert!(
                index < 2,
                "An unacknowledged renewal must not replay the read"
            );
            let (count, ready) = &*arrivals;
            let mut count = count.lock().unwrap();
            *count += 1;
            ready.notify_all();
            let (count, timeout) = ready
                .wait_timeout_while(count, Duration::from_secs(3), |count| *count < 2)
                .unwrap();
            assert!(!timeout.timed_out() && *count == 2);
            Reply::json(200, r#"{"errCode":-2012}"#)
        });
        let client = fixture.client();
        let original = serde_json::to_value(client.session()).unwrap();
        runtime().block_on(async {
            let (first, second) = tokio::join!(
                client.catalog("fixture-book"),
                client.catalog("fixture-book")
            );
            assert!(first.is_err());
            assert!(second.is_err());
        });
        assert_eq!(serde_json::to_value(client.session()).unwrap(), original);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 4);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/web/book/chapterInfos")
                .count(),
            2
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/web/login/renewal")
                .count(),
            2
        );
    }
}

#[test]
fn article_list_reads_recover_business_auth_without_losing_their_query() {
    let fixture = Fixture::scripted(vec![
        Reply::json(200, r#"{"errCode":-2010}"#),
        Reply::json(200, r#"{"succ":1}"#).rotated(),
        Reply::json(200, r#"{"reviews":[]}"#),
    ]);
    let client = fixture.client();
    let articles = runtime()
        .block_on(client.mp_articles("MP_WXS_fixture", 0, 10))
        .unwrap();
    assert!(articles.is_empty());
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].method, "GET");
    assert!(requests[0].path.starts_with("/web/mp/articles?"));
    assert_eq!(requests[1].path, "/web/login/renewal");
    assert_eq!(requests[0].path, requests[2].path);
    assert_eq!(requests[2].method, "GET");
    assert!(requests[2].headers["cookie"].contains("fixture-renewed-key"));
}

#[test]
fn failed_business_auth_renewal_stops_without_committing_rejected_cookies() {
    for renewal in [
        Reply::json(401, "{}"),
        Reply::json(200, r#"{"errCode":-2012}"#),
        Reply::json(200, r#"{"succ":0}"#),
        Reply::json(200, "{}"),
    ] {
        let mut rejected = Reply::json(200, r#"{"errCode":-2012}"#);
        rejected.headers.extend(
            [
                ("Set-Cookie", "wr_rt=; Path=/"),
                ("Set-Cookie", "wr_skey=fixture-rejected-key; Path=/"),
            ]
            .map(|(name, value)| (name, value.to_owned())),
        );
        let fixture = Fixture::scripted(vec![rejected, renewal.rotated()]);
        let client = fixture.client();
        let before = serde_json::to_value(client.session()).unwrap();
        assert!(runtime().block_on(client.catalog("fixture-book")).is_err());
        assert_eq!(serde_json::to_value(client.session()).unwrap(), before);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/web/login/renewal");
        assert!(requests[1].headers["cookie"].contains("wr_rt=fixture-original-refresh"));
    }
}

#[test]
fn catalog_still_expired_after_acknowledgment_never_renews_a_second_time() {
    for rotate in [false, true] {
        let renewal = if rotate {
            Reply::json(200, r#"{"succ":1}"#).rotated()
        } else {
            Reply::json(200, r#"{"succ":1}"#)
        };
        let mut rejected_again = Reply::json(200, r#"{"errCode":-2010}"#);
        rejected_again.headers.push((
            "Set-Cookie",
            "wr_skey=fixture-rejected-again; Path=/".into(),
        ));
        let fixture = Fixture::scripted(vec![
            Reply::json(200, r#"{"errCode":-2012}"#),
            renewal,
            rejected_again,
        ]);
        let client = fixture.client();
        let result = runtime().block_on(client.catalog("fixture-book"));
        assert!(matches!(result, Err(SourceError::Api(code)) if code == "-2010"));
        assert_eq!(
            client.session().cookies["wr_skey"],
            if rotate {
                "fixture-renewed-key"
            } else {
                "fixture-original-key"
            }
        );
        let requests = fixture.finish();
        assert_eq!(requests.len(), 3);
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
fn unreviewed_business_codes_and_identity_only_codes_never_trigger_renewal() {
    for (body, expected_code) in [
        (r#"{"errCode":-2013}"#, "-2013"),
        (r#"{"errCode":-20120}"#, "-20120"),
        (r#"{"errCode":-9999}"#, "-9999"),
        (r#"{"errCode":"FIXTURE_DENIED"}"#, "FIXTURE_DENIED"),
    ] {
        let mut reply = Reply::json(200, body);
        if expected_code == "-2013" {
            reply.headers.push((
                "Set-Cookie",
                "wr_skey=fixture-rejected-session; Path=/".into(),
            ));
        }
        let fixture = Fixture::scripted(vec![reply]);
        let client = fixture.client();
        let original = serde_json::to_value(client.session()).unwrap();
        let result = runtime().block_on(client.catalog("fixture-book"));
        assert!(matches!(&result, Err(SourceError::Api(code)) if code == expected_code));
        assert_eq!(
            result.unwrap_err().requires_login(),
            expected_code == "-2013"
        );
        if expected_code == "-2013" {
            assert_eq!(serde_json::to_value(client.session()).unwrap(), original);
        }
        let requests = fixture.finish();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/web/book/chapterInfos");
    }
}

#[test]
fn http_and_business_auth_failures_share_one_read_recovery_budget() {
    for (first, last) in [
        (
            Reply::json(401, "{}"),
            Reply::json(200, r#"{"errCode":-2012}"#),
        ),
        (
            Reply::json(200, r#"{"errCode":-2012}"#),
            Reply::json(401, "{}"),
        ),
    ] {
        let fixture = Fixture::scripted(vec![
            first,
            Reply::json(200, r#"{"succ":1}"#).rotated(),
            last,
        ]);
        let client = fixture.client();
        let result = runtime().block_on(client.catalog("fixture-book"));
        assert!(matches!(
            result,
            Err(SourceError::Http(401) | SourceError::Api(_))
        ));
        let requests = fixture.finish();
        assert_eq!(requests.len(), 3);
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
fn progress_posts_with_business_auth_errors_are_never_renewed_or_replayed() {
    for failed_post in [1, 2] {
        let mut replies = vec![Reply::json(
            200,
            r#"<script>window.__INITIAL_STATE__ = {"reader":{"psvts":"fixture-reader-clock"}};</script>"#,
        )];
        if failed_post == 2 {
            replies.push(Reply::json(200, r#"{"succ":1}"#));
        }
        replies.push(Reply::json(
            200,
            r#"{"errCode":-2012,"errMsg":"fixture private message"}"#,
        ));
        let fixture = Fixture::scripted(replies);
        let client = fixture.client();
        let position = super::super::ReadingPosition {
            chapter_uid: "fixture-chapter".into(),
            ..Default::default()
        };
        let result = runtime().block_on(client.report_progress("fixture-book", &position, 3));
        assert!(matches!(result, Err(SourceError::Api(code)) if code == "-2012"));
        let requests = fixture.finish();
        assert_eq!(requests.len(), failed_post + 1);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/web/book/read")
                .count(),
            failed_post
        );
        assert!(
            !requests
                .iter()
                .any(|request| request.path == "/web/login/renewal")
        );
    }
}

fn over_budget_reply(body: &'static str) -> Reply {
    Reply::json(200, body).rotated().header(
        "Set-Cookie",
        format!(
            "wr_oversized={}; Path=/",
            "x".repeat(MAX_COOKIE_ENTRY_BYTES)
        ),
    )
}

fn auth_with_cookie_count(count: usize) -> AuthSession {
    let mut session = fixture_auth();
    assert!((session.cookies.len()..=MAX_COOKIE_COUNT).contains(&count));
    for index in 0..count - session.cookies.len() {
        session
            .cookies
            .insert(format!("wr_padding_{index}"), "v".into());
    }
    session
}

fn assert_cookie_budget<T>(result: Result<T>) {
    assert!(matches!(
        result,
        Err(SourceError::InvalidResponse("cookie_budget"))
    ));
}

#[test]
fn ordinary_cookie_budget_rejection_keeps_the_entire_session_and_never_renews() {
    let fixture = Fixture::scripted(vec![over_budget_reply("{}")]);
    let client = fixture.client();
    let before = client.session();
    assert_cookie_budget(runtime().block_on(get_json(&client, "/web/reader/fixture-budget")));
    assert_eq!(client.session(), before);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/web/reader/fixture-budget");
}

#[test]
fn parsed_read_cookie_budget_rejection_does_not_spend_auth_recovery() {
    let fixture = Fixture::scripted(vec![over_budget_reply(FIXTURE_CATALOG)]);
    let client = fixture.client();
    let before = client.session();
    assert_cookie_budget(runtime().block_on(client.catalog("fixture-book")));
    assert_eq!(client.session(), before);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/web/book/chapterInfos");
}

#[test]
fn a_shard_pair_propagates_cookie_budget_failure_without_auth_recovery() {
    let fixture = Fixture::scripted(vec![
        Reply::json(200, "fixture-first-shard"),
        over_budget_reply("fixture-second-shard"),
    ]);
    let client = fixture.client();
    let before = client.session();
    let mut auth = WebReadAuth::default();
    let result = runtime().block_on(async {
        let first = client
            .web_request(Method::POST, "/web/book/chapter/e_0")
            .unwrap();
        let second = client
            .web_request(Method::POST, "/web/book/chapter/e_1")
            .unwrap();
        client
            .web_text_pair(first, second, &mut auth)
            .await
            .and_then(|pair| pair.result)
    });
    assert_cookie_budget(result);
    assert!(auth.recovery_available);
    assert!(!auth.restart_reader && !auth.recovery_failed);
    assert_eq!(client.session(), before);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(
        !requests
            .iter()
            .any(|request| request.path == "/web/login/renewal")
    );
}

#[test]
fn a_cookie_budget_failure_wins_over_a_renewable_peer_in_either_shard_order() {
    for budget_first in [true, false] {
        let fixture = Fixture::serve(2, move |request, _| {
            if request.path.ends_with("e_0") == budget_first {
                over_budget_reply("fixture-budget-shard")
            } else {
                Reply::json(200, r#"{"errCode":-2012}"#)
            }
        });
        let client = fixture.client();
        let before = client.session();
        let mut auth = WebReadAuth::default();
        let result = runtime().block_on(async {
            let first = client
                .web_request(Method::POST, "/web/book/chapter/e_0")
                .unwrap();
            let second = client
                .web_request(Method::POST, "/web/book/chapter/e_1")
                .unwrap();
            client
                .web_text_pair(first, second, &mut auth)
                .await
                .and_then(|pair| pair.result)
        });
        assert_cookie_budget(result);
        assert!(auth.recovery_available);
        assert!(!auth.restart_reader && !auth.recovery_failed);
        assert_eq!(client.session(), before);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|request| request.path.starts_with("/web/book/chapter/"))
        );
    }
}

#[test]
fn acknowledged_renewal_with_excess_cookies_is_rejected_without_partial_rotation() {
    let fixture = Fixture::scripted(vec![over_budget_reply(r#"{"succ":1}"#)]);
    let client = fixture.client();
    let before = client.session();
    assert_cookie_budget(runtime().block_on(client.renew()));
    assert_eq!(client.session(), before);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/web/login/renewal");
}

#[test]
fn read_recovery_never_replays_after_an_over_budget_renewal() {
    let fixture = Fixture::scripted(vec![
        Reply::json(401, "{}"),
        over_budget_reply(r#"{"succ":1}"#),
    ]);
    let client = fixture.client();
    let before = client.session();
    assert_cookie_budget(runtime().block_on(get_json(&client, "/web/reader/fixture-expired")));
    assert_eq!(client.session(), before);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/web/reader/fixture-expired");
    assert_eq!(requests[1].path, "/web/login/renewal");
}

#[test]
fn a_rejected_cookie_batch_does_not_discard_successful_earlier_state() {
    let fixture = Fixture::scripted(vec![
        Reply::json(200, "{}").header("Set-Cookie", "wr_retained=first; Path=/"),
        over_budget_reply("{}"),
    ]);
    let client = fixture.client();
    runtime()
        .block_on(get_json(&client, "/web/reader/fixture-first"))
        .unwrap();
    let retained = client.session();
    assert_eq!(retained.cookies["wr_retained"], "first");
    assert_cookie_budget(runtime().block_on(get_json(&client, "/web/reader/fixture-rejected")));
    assert_eq!(client.session(), retained);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].headers["cookie"].contains("wr_retained=first"));
}

#[test]
fn concurrent_cookie_replies_merge_non_auth_cookies_when_the_budget_allows() {
    let arrivals = Arc::new((Mutex::new(0), Condvar::new()));
    let fixture = Fixture::serve(2, move |request, _| {
        let (count, ready) = &*arrivals;
        let mut count = count.lock().unwrap();
        *count += 1;
        ready.notify_all();
        let (count, timeout) = ready
            .wait_timeout_while(count, Duration::from_secs(3), |count| *count < 2)
            .unwrap();
        assert!(!timeout.timed_out() && *count == 2);
        let name = if request.path.ends_with("first") {
            "wr_first"
        } else {
            "wr_second"
        };
        Reply::json(200, "{}").header("Set-Cookie", format!("{name}=accepted; Path=/"))
    });
    let client = fixture.client();
    let original = client.session();
    runtime().block_on(async {
        let (first, second) = tokio::join!(
            get_json(&client, "/web/reader/fixture-first"),
            get_json(&client, "/web/reader/fixture-second"),
        );
        first.unwrap();
        second.unwrap();
    });
    let mut expected = original;
    expected
        .cookies
        .insert("wr_first".into(), "accepted".into());
    expected
        .cookies
        .insert("wr_second".into(), "accepted".into());
    assert_eq!(client.session(), expected);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
}

#[test]
fn concurrent_individually_valid_cookie_batches_cannot_overfill_the_shared_jar() {
    let arrivals = Arc::new((Mutex::new(0), Condvar::new()));
    let fixture = Fixture::serve(2, move |request, _| {
        let (count, ready) = &*arrivals;
        let mut count = count.lock().unwrap();
        *count += 1;
        ready.notify_all();
        let (count, timeout) = ready
            .wait_timeout_while(count, Duration::from_secs(3), |count| *count < 2)
            .unwrap();
        assert!(!timeout.timed_out() && *count == 2);
        let name = if request.path.ends_with("first") {
            "wr_first"
        } else {
            "wr_second"
        };
        Reply::json(200, "{}").header("Set-Cookie", format!("{name}=accepted; Path=/"))
    });
    let client = fixture.client();
    client
        .replace_session(auth_with_cookie_count(MAX_COOKIE_COUNT - 1))
        .unwrap();
    let original = client.session();
    runtime().block_on(async {
        let (first, second) = tokio::join!(
            get_json(&client, "/web/reader/fixture-first"),
            get_json(&client, "/web/reader/fixture-second"),
        );
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        for result in [first, second] {
            if result.is_err() {
                assert_cookie_budget(result);
            }
        }
    });
    let current = client.session();
    assert_eq!(current.cookies.len(), MAX_COOKIE_COUNT);
    assert_ne!(
        current.cookies.contains_key("wr_first"),
        current.cookies.contains_key("wr_second")
    );
    let mut expected = original;
    let accepted_cookie = if current.cookies.contains_key("wr_first") {
        "wr_first"
    } else {
        "wr_second"
    };
    expected
        .cookies
        .insert(accepted_cookie.into(), "accepted".into());
    assert_eq!(current, expected);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
}

#[test]
fn an_over_budget_reply_for_an_old_account_cannot_modify_its_replacement() {
    let released = Arc::new((Mutex::new(false), Condvar::new()));
    let server_release = released.clone();
    let fixture = Fixture::serve(1, move |_, _| {
        let (released, ready) = &*server_release;
        let released = released.lock().unwrap();
        let (released, timeout) = ready
            .wait_timeout_while(released, Duration::from_secs(3), |released| !*released)
            .unwrap();
        assert!(!timeout.timed_out() && *released);
        over_budget_reply("{}")
    });
    let client = fixture.client();
    let mut replacement = fixture_auth();
    replacement.api_key = "fixture-replacement-key".into();
    replacement.user_vid = "fixture-replacement-account".into();
    replacement
        .cookies
        .insert("wr_vid".into(), replacement.user_vid.clone());
    replacement
        .cookies
        .insert("wr_skey".into(), "fixture-replacement-session".into());
    let expected = replacement.clone();
    runtime().block_on(async {
        let incoming = get_json(&client, "/web/reader/fixture-old-account");
        let replace = async {
            let deadline = Instant::now() + Duration::from_secs(3);
            while fixture.records.lock().unwrap().is_empty() {
                assert!(
                    Instant::now() < deadline,
                    "The old-account request was not sent"
                );
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            client.replace_session(replacement).unwrap();
            let (released, ready) = &*released;
            *released.lock().unwrap() = true;
            ready.notify_all();
        };
        let (incoming, ()) = tokio::join!(incoming, replace);
        incoming.unwrap();
    });
    assert_eq!(client.session(), expected);
    assert_eq!(fixture.finish().len(), 1);
}

const FIXTURE_READER_STATE: &str =
    r#"<script>window.__INITIAL_STATE__ = {"reader":{"psvts":"fixture-reader-clock"}};</script>"#;

fn fixture_position() -> super::super::ReadingPosition {
    super::super::ReadingPosition {
        chapter_uid: "fixture-chapter".into(),
        ..Default::default()
    }
}

#[test]
fn an_enter_cookie_budget_failure_prevents_sending_elapsed_time() {
    let fixture = Fixture::scripted(vec![
        Reply::json(200, FIXTURE_READER_STATE),
        over_budget_reply(r#"{"succ":1}"#),
    ]);
    let client = fixture.client();
    let before = client.session();
    assert_cookie_budget(runtime().block_on(client.report_progress(
        "fixture-book",
        &fixture_position(),
        3,
    )));
    assert_eq!(client.session(), before);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/web/book/read");
    let enter: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert!(enter.get("rt").is_none());
    assert!(
        !requests
            .iter()
            .any(|request| request.path == "/web/login/renewal")
    );
}

#[test]
fn an_unacknowledged_elapsed_cookie_failure_remains_uncertain_and_is_not_replayed() {
    let fixture = Fixture::scripted(vec![
        Reply::json(200, FIXTURE_READER_STATE),
        Reply::json(200, r#"{"succ":1}"#),
        over_budget_reply("{}"),
    ]);
    let client = fixture.client();
    let before = client.session();
    let result = runtime().block_on(client.report_progress("fixture-book", &fixture_position(), 3));
    assert!(matches!(result, Err(SourceError::DeliveryUncertain)));
    assert_eq!(client.session(), before);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    let elapsed: Value = serde_json::from_slice(&requests[2].body).unwrap();
    assert_eq!(elapsed["rt"], 3);
    assert!(
        !requests
            .iter()
            .any(|request| request.path == "/web/login/renewal")
    );
}

#[test]
fn an_explicit_elapsed_acknowledgement_survives_rejected_cookie_rotation() {
    for body in [r#"{"succ":1}"#, r#"{"synckey":"fixture-ack"}"#] {
        let fixture = Fixture::scripted(vec![
            Reply::json(200, FIXTURE_READER_STATE),
            Reply::json(200, r#"{"succ":1}"#),
            over_budget_reply(body),
        ]);
        let client = fixture.client();
        let before = client.session();
        runtime()
            .block_on(client.report_progress("fixture-book", &fixture_position(), 3))
            .unwrap();
        assert_eq!(client.session(), before);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/web/book/read")
                .count(),
            2
        );
        assert!(
            !requests
                .iter()
                .any(|request| request.path == "/web/login/renewal")
        );
    }
}
