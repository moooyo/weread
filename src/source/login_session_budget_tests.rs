//! Real GET/Poll login responses admit a whole account before owned output.

use super::super::client::api_gate_test_support::{HttpFixture, Reply, auth};
use super::*;
use auth_budget::{reset_work, work};
use std::sync::{Condvar, Mutex};

const VID: &str = "123";
const SKEY: &str = "12345678";
const KEY: &str = "synthetic-enabled-key";
const NAME: &str = "Synthetic account";

fn flow(fixture: &HttpFixture) -> LoginFlow {
    let mut flow = LoginFlow::new().unwrap();
    flow.http = fixture.http();
    flow.test_origin = Some(fixture.origin().into());
    flow
}

fn expected(key: String, name: String) -> AuthSession {
    AuthSession {
        api_key: key,
        cookies: BTreeMap::from([
            ("wr_gid".into(), "synthetic-bootstrap".into()),
            ("wr_poll".into(), "synthetic-poll".into()),
            ("wr_vid".into(), VID.into()),
            ("wr_skey".into(), SKEY.into()),
            ("wr_ql".into(), "0".into()),
        ]),
        account_name: name,
        user_vid: VID.into(),
        login_time: now_seconds(),
        ..Default::default()
    }
}

fn values(field: &str, extra: usize) -> (String, String) {
    let mut session = expected(KEY.into(), NAME.into());
    let selected = if field == "apikey" {
        &mut session.api_key
    } else {
        &mut session.account_name
    };
    selected.push_str("\u{4e2d}\"\\\n\u{1}");
    let padding =
        AuthSession::MAX_ENCODED_BYTES - serde_json::to_vec(&session).unwrap().len() + extra;
    if field == "apikey" {
        session.api_key.push_str(&"x".repeat(padding));
    } else {
        session.account_name.push_str(&"x".repeat(padding));
    }
    (session.api_key, session.account_name)
}

fn fixture(key: Value, name: Value) -> HttpFixture {
    HttpFixture::new(move |request| match request.path() {
        "/r/weread-skills" => Reply::text("Ready").cookie("wr_gid=synthetic-bootstrap; Path=/"),
        "/api/auth/getLoginUid" => Reply::json(serde_json::json!({"uid":"whole-session-uid"})),
        "/api/auth/getLoginInfo" => Reply::json(serde_json::json!({
            "succeed":true,"webLoginVid":123,"accessToken":12345678,"refreshToken":""
        }))
        .cookie("wr_poll=synthetic-poll; Path=/"),
        "/api/userInfo" => Reply::json(serde_json::json!({"name":name})),
        "/api/skills/apikeyGet" => Reply::json(serde_json::json!({"apikey":key})),
        path => panic!("Unexpected whole-session login path: {path}"),
    })
}

fn assert_capacity<T>(result: Result<T>) {
    assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
    assert_eq!(work().materializations, 0);
    assert_eq!(work().field_copy_bytes, 0);
    assert_eq!(work().adoptions, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn one_over_key_or_name_is_recoverable_and_keeps_old_account_file_and_flow_cookies() {
    for field in ["apikey", "name"] {
        let (key, name) = values(field, 1);
        let fixture = fixture(serde_json::json!(key), serde_json::json!(name));
        let old = auth();
        let client = fixture.client(old.clone());
        let directory = tempfile::tempdir().unwrap();
        let file = crate::session::SessionFile::new(directory.path());
        file.save(&old).unwrap();
        let encoded = std::fs::read(directory.path().join("account.json")).unwrap();
        let flow = flow(&fixture);
        let challenge = flow.begin().await.unwrap();
        let before = flow.state.lock().unwrap().clone();
        reset_work();
        assert_capacity(flow.poll(&challenge.uid, None).await);
        let after = flow.state.lock().unwrap().clone();
        assert_eq!(after.cookies, before.cookies);
        assert_eq!(after.challenge.as_ref().unwrap().uid, challenge.uid);
        assert_eq!(client.session(), old);
        assert_eq!(
            std::fs::read(directory.path().join("account.json")).unwrap(),
            encoded
        );
        assert!(!SourceError::ResponseTooLarge.requires_login());
        assert!(!SourceError::ResponseTooLarge.retryable());
        let records = fixture.finish();
        assert_eq!(records.len(), 5);
        assert!(records.iter().all(|request| request.method == "GET"));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn exact_encoded_key_or_name_is_admitted_then_materialized_and_saved_without_truncation() {
    for field in ["apikey", "name"] {
        let (key, name) = values(field, 0);
        let fixture = fixture(
            serde_json::json!(key.clone()),
            serde_json::json!(name.clone()),
        );
        let flow = flow(&fixture);
        let challenge = flow.begin().await.unwrap();
        reset_work();
        let QrLoginState::Authenticated(session) = flow.poll(&challenge.uid, None).await.unwrap()
        else {
            panic!("The exact encoded account was not authenticated");
        };
        assert_eq!(session.api_key, key);
        assert_eq!(session.account_name, name);
        assert_eq!(
            serde_json::to_vec(&session).unwrap().len(),
            AuthSession::MAX_ENCODED_BYTES
        );
        assert_eq!(work().materializations, 1);
        assert!(work().field_copy_bytes > 0);
        let directory = tempfile::tempdir().unwrap();
        let file = crate::session::SessionFile::new(directory.path());
        file.save(&session).unwrap();
        assert_eq!(file.load().unwrap(), Some(session));
        let records = fixture.finish();
        assert_eq!(records.len(), 5);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn independently_oversized_user_name_stops_before_api_key_get_or_account_copy() {
    let fixture = HttpFixture::new(|request| match request.path() {
        "/r/weread-skills" => Reply::text("Ready"),
        "/api/auth/getLoginUid" => Reply::json(serde_json::json!({"uid":"whole-session-uid"})),
        "/api/auth/getLoginInfo" => Reply::json(serde_json::json!({
            "succeed":true,"webLoginVid":123,"accessToken":12345678
        })),
        "/api/userInfo" => Reply::json(serde_json::json!({
            "name":"x".repeat(AuthSession::MAX_ENCODED_BYTES + 1)
        })),
        path => panic!("An oversized name reached another request: {path}"),
    });
    let flow = flow(&fixture);
    let challenge = flow.begin().await.unwrap();
    let before = flow.state.lock().unwrap().clone();
    reset_work();
    assert_capacity(flow.poll(&challenge.uid, None).await);
    assert_eq!(flow.state.lock().unwrap().cookies, before.cookies);
    let records = fixture.finish();
    assert_eq!(records.len(), 4);
    assert!(
        records
            .iter()
            .all(|request| request.path() != "/api/skills/apikeyGet")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn numeric_aliases_and_unicode_names_keep_the_original_owned_account_values() {
    for (key, name, key_text, name_text) in [
        (
            serde_json::json!(u64::MAX),
            serde_json::json!(730),
            u64::MAX.to_string(),
            "730".to_owned(),
        ),
        (
            serde_json::json!(KEY),
            serde_json::json!("\u{4e2d}\u{1f600}\n\"\\"),
            KEY.to_owned(),
            "\u{4e2d}\u{1f600}\n\"\\".to_owned(),
        ),
    ] {
        let fixture = fixture(key, name);
        let flow = flow(&fixture);
        let challenge = flow.begin().await.unwrap();
        reset_work();
        let QrLoginState::Authenticated(session) = flow.poll(&challenge.uid, None).await.unwrap()
        else {
            panic!("The admitted scalar account did not authenticate");
        };
        assert_eq!(session.api_key, key_text);
        assert_eq!(session.account_name, name_text);
        assert_eq!(session.user_vid, VID);
        assert_eq!(work().materializations, 1);
        assert!(session.validate_resource_budget().is_ok());
        assert_eq!(fixture.finish().len(), 5);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_fences_a_late_oversized_user_response_before_its_budget_or_copy() {
    let gate = Arc::new((Mutex::new((false, false)), Condvar::new()));
    let server_gate = gate.clone();
    let fixture = HttpFixture::new(move |request| match request.path() {
        "/r/weread-skills" => Reply::text("Ready"),
        "/api/auth/getLoginUid" => Reply::json(serde_json::json!({"uid":"whole-session-uid"})),
        "/api/auth/getLoginInfo" => Reply::json(serde_json::json!({
            "succeed":true,"webLoginVid":123,"accessToken":12345678
        })),
        "/api/userInfo" => {
            let (lock, changed) = &*server_gate;
            let mut state = lock.lock().unwrap();
            state.0 = true;
            changed.notify_all();
            let (state, timeout) = changed
                .wait_timeout_while(state, Duration::from_secs(3), |state| !state.1)
                .unwrap();
            assert!(
                state.1 && !timeout.timed_out(),
                "The login cancellation did not release its reply"
            );
            Reply::json(serde_json::json!({"name":"x".repeat(AuthSession::MAX_ENCODED_BYTES + 1)}))
        }
        path => panic!("A cancelled login reached another request: {path}"),
    });
    let flow = flow(&fixture);
    let challenge = flow.begin().await.unwrap();
    reset_work();
    let cancel = async {
        tokio::time::timeout(Duration::from_secs(3), async {
            while !gate.0.lock().unwrap().0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        flow.cancel();
        gate.0.lock().unwrap().1 = true;
        gate.1.notify_all();
    };
    let (result, ()) = tokio::join!(flow.poll(&challenge.uid, None), cancel);
    assert!(matches!(
        result,
        Err(SourceError::Unsupported("login_cancelled"))
    ));
    // Only the earlier credential overlay was considered, not the obsolete name.
    assert_eq!(work().preflights, 1);
    assert_eq!(work().materializations, 0);
    assert_eq!(work().field_copy_bytes, 0);
    assert!(flow.state.lock().unwrap().challenge.is_none());
    assert_eq!(fixture.finish().len(), 4);
}
