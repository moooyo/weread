//! Loopback-only login fixtures verify cookie and QR challenge budgets.

use super::*;
use crate::source::client::{MAX_COOKIE_COUNT, WeReadClient};
use serde_json::json;
use std::{
    collections::VecDeque,
    future::Future,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering},
    thread::{self, JoinHandle},
    time::Instant,
};

const FIXTURE_UID: &str = "synthetic-login-budget-uid";
const NEW_VID: &str = "synthetic-new-login-account";
const ACCESS_TOKEN: &str = "synthetic-login-access-token";
const REFRESH_TOKEN: &str = "synthetic-login-refresh-token";

#[derive(Clone, Debug)]
struct RequestRecord {
    path: String,
    headers: BTreeMap<String, String>,
}

struct Reply {
    body: String,
    cookies: Vec<String>,
    release: Option<Arc<AtomicBool>>,
}

impl Reply {
    fn json(value: Value) -> Self {
        Self {
            body: value.to_string(),
            cookies: Vec::new(),
            release: None,
        }
    }

    fn cookie(mut self, cookie: impl Into<String>) -> Self {
        self.cookies.push(cookie.into());
        self
    }

    fn gated(mut self, release: Arc<AtomicBool>) -> Self {
        self.release = Some(release);
        self
    }
}

struct Fixture {
    address: SocketAddr,
    origin: String,
    records: Arc<Mutex<Vec<RequestRecord>>>,
    accepted: Arc<AtomicUsize>,
    stopping: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Fixture {
    fn new(replies: Vec<Reply>) -> Self {
        Self::with_replies(|_| replies)
    }

    fn with_replies(replies: impl FnOnce(&str) -> Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let origin = format!("http://{address}");
        let mut replies = VecDeque::from(replies(&origin));
        let records = Arc::new(Mutex::new(Vec::new()));
        let accepted = Arc::new(AtomicUsize::new(0));
        let stopping = Arc::new(AtomicBool::new(false));
        let captured = records.clone();
        let observed = accepted.clone();
        let cancelled = stopping.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(6);
            while !cancelled.load(AtomicOrdering::Acquire)
                && Instant::now() < deadline
                && observed.load(AtomicOrdering::Acquire) < 16
            {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if cancelled.load(AtomicOrdering::Acquire) {
                            break;
                        }
                        observed.fetch_add(1, AtomicOrdering::AcqRel);
                        if stream
                            .set_read_timeout(Some(Duration::from_millis(50)))
                            .is_err()
                            || stream
                                .set_write_timeout(Some(Duration::from_millis(100)))
                                .is_err()
                        {
                            continue;
                        }
                        let Some(record) = read_request(&mut stream, &cancelled, deadline) else {
                            continue;
                        };
                        captured
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(record);
                        // Unexpected requests still receive a finite local reply.
                        // A missing budget check therefore cannot reach a real API.
                        let reply = replies.pop_front().unwrap_or_else(|| {
                            Reply::json(json!({"name":"unexpected", "apikey":"unexpected"}))
                        });
                        if let Some(release) = reply.release.as_ref() {
                            while !release.load(AtomicOrdering::Acquire)
                                && !cancelled.load(AtomicOrdering::Acquire)
                                && Instant::now() < deadline
                            {
                                thread::sleep(Duration::from_millis(2));
                            }
                            if cancelled.load(AtomicOrdering::Acquire) || Instant::now() >= deadline
                            {
                                break;
                            }
                        }
                        let mut response = format!(
                            "HTTP/1.1 200 Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                            reply.body.len()
                        );
                        for cookie in reply.cookies {
                            response.push_str(&format!("Set-Cookie: {cookie}\r\n"));
                        }
                        response.push_str("\r\n");
                        response.push_str(&reply.body);
                        let _ = stream.write_all(response.as_bytes());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            address,
            origin,
            records,
            accepted,
            stopping,
            worker: Some(worker),
        }
    }

    fn flow(&self) -> LoginFlow {
        let mut flow = LoginFlow::new().unwrap();
        flow.test_origin = Some(self.origin.clone());
        flow.http = Client::builder()
            .no_proxy()
            // Also pin the production hostname to loopback as a safety net if
            // an origin-hook regression accidentally uses a hard-coded URL.
            .resolve("weread.qq.com", self.address)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_millis(300))
            .timeout(Duration::from_millis(700))
            .build()
            .unwrap();
        flow
    }

    fn records(&self) -> Vec<RequestRecord> {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    async fn wait_for_requests(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while self.records().len() < count {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("The loopback login request did not arrive before its deadline");
    }

    fn assert_no_requests(&self) {
        // The rejected operation has completed. Allow any mistakenly spawned
        // connector to become observable before asserting the listener stayed idle.
        thread::sleep(Duration::from_millis(30));
        assert_eq!(self.accepted.load(AtomicOrdering::Acquire), 0);
        assert!(self.records().is_empty());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stopping.store(true, AtomicOrdering::Release);
        if let Some(worker) = self.worker.take() {
            // Accept is nonblocking and active reads have a short timeout plus
            // cancellation checks, so cleanup never waits for a missing request.
            let _ = worker.join();
        }
    }
}

fn read_request(
    stream: &mut TcpStream,
    cancelled: &AtomicBool,
    fixture_deadline: Instant,
) -> Option<RequestRecord> {
    let deadline = (Instant::now() + Duration::from_secs(1)).min(fixture_deadline);
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    while !cancelled.load(AtomicOrdering::Acquire) && Instant::now() < deadline {
        match stream.read(&mut buffer) {
            Ok(0) => return None,
            Ok(count) => {
                if bytes.len().saturating_add(count) > 128 * 1024 {
                    return None;
                }
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let header = std::str::from_utf8(&bytes[..end]).ok()?;
                    let mut lines = header.split("\r\n");
                    let path = lines.next()?.split_whitespace().nth(1)?.to_owned();
                    let headers = lines
                        .filter_map(|line| line.split_once(':'))
                        .map(|(name, value)| {
                            (name.trim().to_ascii_lowercase(), value.trim().to_owned())
                        })
                        .collect();
                    return Some(RequestRecord { path, headers });
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return None,
        }
    }
    None
}

async fn bounded<T>(future: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .expect("The synthetic login operation exceeded its deadline")
}

fn cookies(count: usize) -> BTreeMap<String, String> {
    let mut cookies = BTreeMap::from([
        ("wr_vid".into(), "synthetic-old-login-account".into()),
        ("wr_skey".into(), "synthetic-old-login-session".into()),
        ("wr_ql".into(), "0".into()),
        ("wr_rt".into(), "synthetic-old-login-refresh".into()),
    ]);
    assert!(count >= cookies.len());
    for index in 0..count - cookies.len() {
        cookies.insert(format!("wr_padding_{index:04}"), "fixture".into());
    }
    validate_cookie_jar(&cookies).unwrap();
    cookies
}

fn seed(flow: &LoginFlow, cookies: BTreeMap<String, String>) {
    *flow
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = FlowState {
        cookies,
        challenge: Some(QrLoginChallenge {
            uid: FIXTURE_UID.into(),
            confirm_url: format!("{}/web/confirm?uid={FIXTURE_UID}", flow.origin()),
            expires_at: now_seconds() + 300,
        }),
    };
}

type FlowSnapshot = (BTreeMap<String, String>, Option<(String, String, i64)>, u64);

fn snapshot(flow: &LoginFlow) -> FlowSnapshot {
    let state = flow
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    (
        state.cookies.clone(),
        state.challenge.as_ref().map(|challenge| {
            (
                challenge.uid.clone(),
                challenge.confirm_url.clone(),
                challenge.expires_at,
            )
        }),
        flow.generation.load(AtomicOrdering::SeqCst),
    )
}

fn credentials(access: &str, refresh: &str) -> Value {
    json!({
        "succeed": true,
        "webLoginVid": NEW_VID,
        "accessToken": access,
        "refreshToken": refresh,
    })
}

fn expect_budget_error<T>(result: Result<T>) {
    let error = match result {
        Ok(_) => panic!("The oversized login cookie collection was accepted"),
        Err(error) => error,
    };
    assert!(matches!(
        &error,
        SourceError::InvalidResponse("cookie_budget")
    ));
    assert!(!error.retryable());
    assert!(!error.requires_login());
}

fn old_account(cookies: BTreeMap<String, String>) -> WeReadClient {
    WeReadClient::new(AuthSession {
        api_key: "synthetic-retained-api-key".into(),
        account_name: "Synthetic retained account".into(),
        user_vid: "synthetic-old-login-account".into(),
        cookies,
        ..AuthSession::default()
    })
    .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn a_poll_budget_failure_preserves_the_whole_login_flow_and_old_account() {
    let fixture = Fixture::new(vec![
        Reply::json(json!({"logicCode":""}))
            .cookie("wr_skey=synthetic-unaccepted-poll-session")
            .cookie("wr_one_too_many=fixture"),
    ]);
    let flow = fixture.flow();
    let original = cookies(MAX_COOKIE_COUNT);
    let account = old_account(original.clone());
    let previous_account = account.session();
    seed(&flow, original);
    let previous = snapshot(&flow);
    expect_budget_error(bounded(flow.poll(FIXTURE_UID, None)).await);
    assert_eq!(snapshot(&flow), previous);
    assert_eq!(account.session(), previous_account);
    let requests = fixture.records();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].path.starts_with("/api/auth/getLoginInfo?"));
}

#[tokio::test(flavor = "current_thread")]
async fn a_user_info_budget_failure_rolls_back_poll_rotation_without_requesting_an_api_key() {
    let fixture = Fixture::new(vec![
        Reply::json(credentials(ACCESS_TOKEN, REFRESH_TOKEN))
            .cookie("wr_skey=synthetic-unaccepted-poll-session"),
        Reply::json(json!({"name":"Synthetic incoming account"}))
            .cookie("wr_skey=synthetic-unaccepted-user-info-session")
            .cookie("wr_one_too_many=fixture"),
    ]);
    let flow = fixture.flow();
    let original = cookies(MAX_COOKIE_COUNT);
    let account = old_account(original.clone());
    let previous_account = account.session();
    seed(&flow, original);
    let previous = snapshot(&flow);
    expect_budget_error(bounded(flow.poll(FIXTURE_UID, None)).await);
    assert_eq!(snapshot(&flow), previous);
    assert_eq!(account.session(), previous_account);
    let requests = fixture.records();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].path.starts_with("/api/auth/getLoginInfo?"));
    assert!(requests[1].path.starts_with("/api/userInfo?"));
    assert_eq!(requests[1].headers["x-vid"], NEW_VID);
    assert_eq!(requests[1].headers["x-skey"], ACCESS_TOKEN);
    let sent_cookies = &requests[1].headers["cookie"];
    assert!(sent_cookies.contains(&format!("wr_vid={NEW_VID}")));
    assert!(sent_cookies.contains(&format!("wr_skey={ACCESS_TOKEN}")));
    assert!(sent_cookies.contains("wr_ql=0"));
    assert!(sent_cookies.contains(&format!("wr_rt={REFRESH_TOKEN}")));
    assert!(
        requests
            .iter()
            .all(|request| !request.path.contains("apikeyGet"))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn an_oversized_manual_access_token_is_rejected_before_any_request() {
    let fixture = Fixture::new(Vec::new());
    let flow = fixture.flow();
    let original = cookies(4);
    seed(&flow, original.clone());
    let previous = snapshot(&flow);
    let result = credentials(&"a".repeat(MAX_COOKIE_ENTRY_BYTES + 1), REFRESH_TOKEN);
    expect_budget_error(
        bounded(flow.complete(
            &result,
            original,
            flow.generation.load(AtomicOrdering::SeqCst),
        ))
        .await,
    );
    assert_eq!(snapshot(&flow), previous);
    fixture.assert_no_requests();
}

#[tokio::test(flavor = "current_thread")]
async fn percent_encoded_refresh_growth_is_rejected_before_any_request() {
    let fixture = Fixture::new(Vec::new());
    let flow = fixture.flow();
    let original = cookies(4);
    seed(&flow, original.clone());
    let previous = snapshot(&flow);
    let refresh = "%".repeat(MAX_COOKIE_ENTRY_BYTES / 3 + 1);
    assert!(refresh.len() < MAX_COOKIE_ENTRY_BYTES);
    assert!(encode_component(&refresh).len() > MAX_COOKIE_ENTRY_BYTES);
    let result = credentials(ACCESS_TOKEN, &refresh);
    expect_budget_error(
        bounded(flow.complete(
            &result,
            original,
            flow.generation.load(AtomicOrdering::SeqCst),
        ))
        .await,
    );
    assert_eq!(snapshot(&flow), previous);
    fixture.assert_no_requests();
}

#[tokio::test(flavor = "current_thread")]
async fn a_full_login_jar_can_rotate_and_delete_cookies_through_the_local_login_flow() {
    let fixture = Fixture::new(vec![
        Reply::json(json!({})),
        Reply::json(json!({"uid":FIXTURE_UID})),
        Reply::json(credentials(ACCESS_TOKEN, REFRESH_TOKEN))
            .cookie("wr_skey=synthetic-valid-poll-session")
            .cookie("wr_padding_0000=; Max-Age=0")
            .cookie("wr_poll_replacement=fixture"),
        Reply::json(json!({"name":"Synthetic completed account"}))
            .cookie("wr_skey=synthetic-valid-user-info-session")
            .cookie("wr_padding_0001=; Max-Age=0")
            .cookie("wr_info_replacement=fixture"),
        Reply::json(json!({"apikey":"synthetic-completed-api-key"})),
    ]);
    let flow = fixture.flow();
    let challenge = bounded(flow.begin()).await.unwrap();
    assert_eq!(challenge.uid, FIXTURE_UID);
    assert!(challenge.confirm_url.starts_with(&fixture.origin));
    {
        let mut state = flow
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.cookies = cookies(MAX_COOKIE_COUNT);
    }
    let original = snapshot(&flow);
    let session = match bounded(flow.poll(FIXTURE_UID, None)).await.unwrap() {
        QrLoginState::Authenticated(session) => session,
        result => panic!("The valid loopback login did not complete: {result:?}"),
    };
    assert_eq!(session.api_key, "synthetic-completed-api-key");
    assert_eq!(session.account_name, "Synthetic completed account");
    assert_eq!(session.user_vid, NEW_VID);
    assert_eq!(session.cookies.len(), MAX_COOKIE_COUNT);
    assert_eq!(session.cookies["wr_vid"], NEW_VID);
    assert_eq!(session.cookies["wr_rt"], REFRESH_TOKEN);
    assert_eq!(
        session.cookies["wr_skey"],
        "synthetic-valid-user-info-session"
    );
    assert!(!session.cookies.contains_key("wr_padding_0000"));
    assert!(!session.cookies.contains_key("wr_padding_0001"));
    assert!(session.cookies.contains_key("wr_poll_replacement"));
    assert!(session.cookies.contains_key("wr_info_replacement"));
    validate_cookie_jar(&session.cookies).unwrap();
    // The successful flow retains the accepted poll snapshot, as before; the
    // returned account separately owns subsequent credential/response updates.
    let updated = snapshot(&flow);
    assert_eq!(updated.0["wr_skey"], "synthetic-valid-poll-session");
    assert_eq!(updated.1, original.1);
    assert_eq!(updated.2, original.2);
    let requests = fixture.records();
    assert_eq!(requests.len(), 5);
    for (request, path) in requests.iter().zip([
        "/r/weread-skills",
        "/api/auth/getLoginUid",
        "/api/auth/getLoginInfo?",
        "/api/userInfo?",
        "/api/skills/apikeyGet?",
    ]) {
        assert!(request.path.starts_with(path));
        assert_eq!(request.headers["host"], fixture.address.to_string());
    }
}

fn expect_challenge_budget_error<T>(result: Result<T>) {
    let error = match result {
        Ok(_) => panic!("The oversized QR login challenge was accepted"),
        Err(error) => error,
    };
    assert!(matches!(
        &error,
        SourceError::InvalidResponse("login_uid_budget")
    ));
    assert!(!error.retryable());
    assert!(!error.requires_login());
}

fn encoded_boundary_uid(origin: &str, extra_encoded_bytes: usize) -> String {
    let encoded_bytes =
        MAX_LOGIN_CONFIRM_URL_BYTES - origin.len() - CONFIRM_PATH.len() + extra_encoded_bytes;
    let mut uid = "%".repeat(encoded_bytes / 3);
    uid.push_str(&"a".repeat(encoded_bytes % 3));
    assert!(uid.len() <= MAX_LOGIN_UID_BYTES);
    uid
}

#[tokio::test(flavor = "current_thread")]
async fn an_oversized_begin_uid_keeps_no_challenge_or_response_cookies() {
    let uid = "u".repeat(MAX_LOGIN_UID_BYTES + 1);
    let fixture = Fixture::new(vec![
        Reply::json(json!({})).cookie("wr_start=synthetic-start-cookie"),
        Reply::json(json!({"uid":uid})).cookie("wr_uid=synthetic-rejected-cookie"),
    ]);
    let flow = fixture.flow();
    let original = cookies(4);
    let account = old_account(original.clone());
    let previous_account = account.session();
    seed(&flow, original);
    let generation = snapshot(&flow).2;
    expect_challenge_budget_error(bounded(flow.begin()).await);
    assert_eq!(snapshot(&flow), (BTreeMap::new(), None, generation + 1));
    assert_eq!(account.session(), previous_account);
    // Rejection cannot leave a UID that a later poll would use for another request.
    assert!(matches!(
        bounded(flow.poll(&uid, None)).await.unwrap(),
        QrLoginState::Expired
    ));
    let requests = fixture.records();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/r/weread-skills");
    assert_eq!(requests[1].path, "/api/auth/getLoginUid");
}

#[tokio::test(flavor = "current_thread")]
async fn begin_rejects_percent_expansion_before_retaining_the_challenge() {
    let fixture = Fixture::with_replies(|origin| {
        let uid = encoded_boundary_uid(origin, 1);
        assert!(uid.len() < MAX_LOGIN_UID_BYTES);
        assert_eq!(
            origin.len() + CONFIRM_PATH.len() + encode_component(&uid).len(),
            MAX_LOGIN_CONFIRM_URL_BYTES + 1
        );
        vec![
            Reply::json(json!({})),
            Reply::json(json!({"uid":uid})).cookie("wr_uid=synthetic-rejected-cookie"),
        ]
    });
    let flow = fixture.flow();
    let account = old_account(cookies(4));
    let previous_account = account.session();
    expect_challenge_budget_error(bounded(flow.begin()).await);
    assert_eq!(snapshot(&flow), (BTreeMap::new(), None, 1));
    assert_eq!(account.session(), previous_account);
    assert!(matches!(
        bounded(flow.poll(&encoded_boundary_uid(&fixture.origin, 1), None))
            .await
            .unwrap(),
        QrLoginState::Expired
    ));
    assert_eq!(fixture.records().len(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn begin_accepts_the_raw_uid_and_encoded_url_boundaries() {
    let fixture = Fixture::with_replies(|origin| {
        vec![
            Reply::json(json!({})),
            Reply::json(json!({"uid":"a".repeat(MAX_LOGIN_UID_BYTES)})),
            Reply::json(json!({})),
            Reply::json(json!({"uid":encoded_boundary_uid(origin, 0)})),
        ]
    });
    let flow = fixture.flow();
    let raw = bounded(flow.begin()).await.unwrap();
    assert_eq!(raw.uid.len(), MAX_LOGIN_UID_BYTES);
    assert_eq!(
        raw.confirm_url,
        format!("{}{CONFIRM_PATH}{}", fixture.origin, raw.uid)
    );
    let encoded = bounded(flow.begin()).await.unwrap();
    assert_eq!(encoded.uid, encoded_boundary_uid(&fixture.origin, 0));
    assert_eq!(encoded.confirm_url.len(), MAX_LOGIN_CONFIRM_URL_BYTES);
    assert_eq!(
        encoded.confirm_url,
        format!(
            "{}{CONFIRM_PATH}{}",
            fixture.origin,
            encode_component(&encoded.uid)
        )
    );
    let saved = snapshot(&flow);
    assert_eq!(
        saved.1,
        Some((encoded.uid, encoded.confirm_url, encoded.expires_at))
    );
    assert_eq!(saved.2, 2);
    assert_eq!(fixture.records().len(), 4);
}

#[tokio::test(flavor = "current_thread")]
async fn begin_preserves_ordinary_percent_encoding_and_numeric_uids() {
    let uid = "aZ09-_.~% &+/\u{4e2d}\u{6587}\u{1f642}";
    let numbers = [json!(u64::MAX), json!(-42), json!(12.5)];
    let mut replies = vec![Reply::json(json!({})), Reply::json(json!({"uid":uid}))];
    for number in &numbers {
        replies.push(Reply::json(json!({})));
        replies.push(Reply::json(json!({"uid":number})));
    }
    let fixture = Fixture::new(replies);
    let flow = fixture.flow();
    let challenge = bounded(flow.begin()).await.unwrap();
    assert_eq!(challenge.uid, uid);
    assert_eq!(
        challenge.confirm_url,
        format!(
            "{}{CONFIRM_PATH}aZ09-_.~%25%20%26%2B%2F%E4%B8%AD%E6%96%87%F0%9F%99%82",
            fixture.origin
        )
    );
    for number in &numbers {
        let challenge = bounded(flow.begin()).await.unwrap();
        assert_eq!(challenge.uid, number.to_string());
        assert_eq!(
            challenge.confirm_url,
            format!("{}{CONFIRM_PATH}{}", fixture.origin, number)
        );
    }
    assert_eq!(fixture.records().len(), 8);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_an_inflight_begin_fences_its_late_challenge_and_cookies() {
    let release = Arc::new(AtomicBool::new(false));
    let fixture = Fixture::new(vec![
        Reply::json(json!({})).cookie("wr_start=synthetic-old-start-cookie"),
        Reply::json(json!({"uid":"synthetic-old-challenge"}))
            .cookie("wr_uid=synthetic-old-uid-cookie")
            .gated(release.clone()),
        Reply::json(json!({})),
        Reply::json(json!({"uid":"synthetic-new-challenge"}))
            .cookie("wr_uid=synthetic-new-uid-cookie"),
    ]);
    let flow = fixture.flow();
    let account = old_account(cookies(4));
    let previous_account = account.session();
    let (result, ()) = tokio::join!(bounded(flow.begin()), async {
        fixture.wait_for_requests(2).await;
        flow.cancel();
        assert_eq!(snapshot(&flow), (BTreeMap::new(), None, 2));
        release.store(true, AtomicOrdering::Release);
    });
    assert!(matches!(
        result,
        Err(SourceError::Unsupported("login_cancelled"))
    ));
    assert_eq!(snapshot(&flow), (BTreeMap::new(), None, 2));
    assert_eq!(account.session(), previous_account);
    let current = bounded(flow.begin()).await.unwrap();
    assert_eq!(current.uid, "synthetic-new-challenge");
    let saved = snapshot(&flow);
    assert_eq!(
        saved.0,
        BTreeMap::from([("wr_uid".into(), "synthetic-new-uid-cookie".into(),)])
    );
    assert_eq!(
        saved.1,
        Some((current.uid, current.confirm_url, current.expires_at))
    );
    assert_eq!(saved.2, 3);
    assert_eq!(fixture.records().len(), 4);
}
