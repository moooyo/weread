//! Genuine responses and queued results preserve whole-account capacity fences.

use super::*;
use crate::jobs::{JobResult, Jobs};
use crate::source::login::auth_budget::{reset_work, work};

fn near_limit(mut session: AuthSession, room: usize) -> AuthSession {
    session
        .account_name
        .push_str("\u{4e2d}\u{1f600}\"\\\n\u{1}");
    let bytes = serde_json::to_vec(&session).unwrap().len();
    session
        .account_name
        .push_str(&"x".repeat(AuthSession::MAX_ENCODED_BYTES - bytes - room));
    assert_eq!(
        serde_json::to_vec(&session).unwrap().len(),
        AuthSession::MAX_ENCODED_BYTES - room
    );
    session.validate_resource_budget().unwrap();
    session
}

fn assert_capacity<T>(result: Result<T>) {
    assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
    assert_eq!(work().materializations, 0);
    assert_eq!(work().field_copy_bytes, 0);
    assert_eq!(work().adoptions, 0);
}

#[test]
fn ordinary_header_overflow_rejects_before_account_copy_and_keeps_cookies_and_file() {
    let fixture = Fixture::scripted(vec![
        Reply::json(200, "{}")
            .header("x-wr-ticket", "t".repeat(1024))
            .header("x-wrpa-0", "new-wrpa")
            .header("Set-Cookie", "wr_skey=fixture-rotated-key; Path=/"),
    ]);
    let client = fixture.client();
    let original = near_limit(fixture_auth(), 0);
    client.replace_session(original.clone()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let file = crate::session::SessionFile::new(directory.path());
    file.save(&original).unwrap();
    let stored = std::fs::read(directory.path().join("account.json")).unwrap();
    reset_work();
    assert_capacity(runtime().block_on(get_json(&client, "/web/reader/whole-account-overflow")));
    assert_eq!(client.session(), original);
    assert_eq!(
        std::fs::read(directory.path().join("account.json")).unwrap(),
        stored
    );
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn ordinary_ticket_json_escaping_accepts_exact_encoded_limit_and_rejects_one_over() {
    // Two HTTP bytes become four encoded JSON bytes without changing header filtering.
    for (room, accepted) in [(4, true), (3, false)] {
        let fixture = Fixture::scripted(vec![Reply::json(200, "{}").header("x-wr-ticket", "\\\"")]);
        let client = fixture.client();
        let original = near_limit(fixture_auth(), room);
        client.replace_session(original.clone()).unwrap();
        reset_work();
        let result = runtime().block_on(get_json(&client, "/web/reader/whole-account-exact"));
        if accepted {
            result.unwrap();
            assert_eq!(work().materializations, 1);
            assert_eq!(work().adoptions, 1);
            let current = client.session();
            assert_eq!(current.wr_ticket, "\\\"");
            assert_eq!(current.cookies, original.cookies);
            assert_eq!(
                serde_json::to_vec(&current).unwrap().len(),
                AuthSession::MAX_ENCODED_BYTES
            );
        } else {
            assert_capacity(result);
            assert_eq!(client.session(), original);
        }
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[test]
fn authoritative_renewal_can_reclaim_old_ticket_space_without_freezing_valid_rotation() {
    let fixture = Fixture::scripted(vec![
        Reply::json(200, r#"{"succ":1}"#)
            .header("x-wr-ticket", "acknowledged-ticket")
            .header("Set-Cookie", "wr_skey=new-key; Path=/"),
    ]);
    let client = fixture.client();
    let mut original = fixture_auth();
    original.wr_ticket = "o".repeat(1024);
    let original = near_limit(original, 0);
    client.replace_session(original.clone()).unwrap();
    reset_work();
    let result = runtime().block_on(client.renew()).unwrap();
    assert_eq!(result.account_name, original.account_name);
    assert_eq!(result.wr_ticket, "acknowledged-ticket");
    assert_eq!(
        result.cookies.get("wr_skey").map(String::as_str),
        Some("new-key")
    );
    assert_eq!(work().materializations, 1);
    assert_eq!(work().adoptions, 1);
    assert!(serde_json::to_vec(&result).unwrap().len() < AuthSession::MAX_ENCODED_BYTES);
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/web/login/renewal");
}

#[test]
fn acknowledged_renewal_one_over_whole_budget_keeps_old_account_and_generation() {
    let fixture = Fixture::scripted(vec![
        Reply::json(200, r#"{"succ":1}"#).header("x-wr-ticket", "x"),
    ]);
    let client = fixture.client();
    let original = near_limit(fixture_auth(), 0);
    client.replace_session(original.clone()).unwrap();
    reset_work();
    assert_capacity(runtime().block_on(client.renew()));
    assert_eq!(client.session(), original);
    assert_eq!(*runtime().block_on(client.renewal.lock()), 0);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn obsolete_account_or_web_version_returns_before_unrelated_whole_budget_inspection() {
    let original = near_limit(fixture_auth(), 0);
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-wr-ticket",
        HeaderValue::from_bytes(&vec![b'x'; 1024]).unwrap(),
    );
    headers.insert(
        SET_COOKIE,
        HeaderValue::from_static("wr_extra=new-value; Path=/"),
    );
    for account_changed in [true, false] {
        let client = WeReadClient::new(original.clone()).unwrap();
        let mut replacement = original.clone();
        if account_changed {
            replacement.api_key.push('x');
            replacement.account_name.pop();
        } else {
            replacement
                .cookies
                .insert("wr_skey".into(), "fixture-newer-key".into());
        }
        client.replace_session(replacement.clone()).unwrap();
        reset_work();
        assert_eq!(
            client
                .commit_web_headers_inner(&original, &headers, false)
                .unwrap(),
            None
        );
        assert_eq!(work().preflights, 0);
        assert_eq!(work().materializations, 0);
        assert_eq!(work().adoptions, 0);
        assert_eq!(client.session(), replacement);
    }
}

#[test]
fn queued_source_capacity_error_retains_metadata_credit_until_stale_result_is_dropped() {
    let fixture = Fixture::scripted(vec![
        Reply::json(200, r#"{"succ":1}"#).header("x-wr-ticket", "x"),
    ]);
    let runtime = runtime();
    let original = near_limit(fixture_auth(), 0);
    let (sender, receiver) = std::sync::mpsc::channel();
    let (jobs, client) = {
        let _entered = runtime.enter();
        let client = fixture.client();
        client.replace_session(original.clone()).unwrap();
        let jobs = Jobs::new(runtime.handle().clone(), sender, Some(client.clone())).unwrap();
        (jobs, client)
    };
    reset_work();
    assert!(jobs.renew(81).is_none());
    let event = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(event) = receiver.try_recv() {
                    break event;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap()
    });
    assert!(matches!(
        &event.result,
        JobResult::Renew(Err(SourceError::ResponseTooLarge))
    ));
    assert_eq!(jobs.metadata_capacity(), 7);
    assert_eq!(work().materializations, 0);
    assert_eq!(work().adoptions, 0);
    assert_eq!(client.session(), original);
    jobs.clear_session();
    assert!(!jobs.accepts(&event));
    assert_eq!(jobs.metadata_capacity(), 7);
    drop(event);
    assert_eq!(jobs.metadata_capacity(), 8);
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn shard_pair_whole_budget_error_wins_over_auth_peer_without_attempting_renewal() {
    for reversed in [false, true] {
        let fixture = Fixture::serve(2, move |request, _| {
            let capacity = request.path.ends_with("e_1") ^ reversed;
            if capacity {
                Reply::json(200, "{}").header("x-wr-ticket", "x")
            } else {
                Reply::json(200, r#"{"errCode":-2010}"#)
            }
        });
        let client = fixture.client();
        let original = near_limit(fixture_auth(), 0);
        client.replace_session(original.clone()).unwrap();
        reset_work();
        let result = runtime().block_on(async {
            let mut auth = WebReadAuth::default();
            let pair = client
                .web_text_pair(
                    client
                        .web_request(Method::POST, "/web/book/chapter/e_1")
                        .unwrap(),
                    client
                        .web_request(Method::POST, "/web/book/chapter/e_3")
                        .unwrap(),
                    &mut auth,
                )
                .await
                .unwrap();
            assert!(auth.recovery_available);
            pair.result
        });
        assert_capacity(result);
        assert_eq!(client.session(), original);
        let requests = fixture.finish();
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|request| request.path != "/web/login/renewal")
        );
    }
}
