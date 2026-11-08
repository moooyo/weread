//! Exact encoding and borrowed admission are independent of transport payload limits.

use super::*;
use crate::source::client::scalar_text_at;

fn session() -> AuthSession {
    AuthSession {
        api_key: "fixture-key".into(),
        account_name: "Fixture account".into(),
        user_vid: "fixture-vid".into(),
        login_time: 1_800_000_000,
        wr_ticket: "fixture-ticket".into(),
        wr_wrpa: "fixture-wrpa".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "fixture-vid".into()),
            ("wr_skey".into(), "fixture-session".into()),
        ]),
    }
}

fn assert_capacity<T>(result: Result<T>) {
    assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
    assert_eq!(work().materializations, 0);
    assert_eq!(work().field_copy_bytes, 0);
}

#[test]
fn borrowed_session_encoding_matches_owned_serde_for_every_field_and_cookie_escape() {
    let mut session = session();
    let text = "\"\\\n\r\t\u{8}\u{c}\u{1}\u{4e2d}\u{1f600}/";
    session.api_key = text.into();
    session.account_name = text.into();
    session.user_vid = text.into();
    session.wr_ticket = text.into();
    session.wr_wrpa = text.into();
    session.login_time = i64::MIN;
    session.cookies.insert("wr_escaped".into(), text.into());
    let encoded = serde_json::to_vec(&session).unwrap();
    reset_work();
    let admitted = AuthSessionView::from_session(&session)
        .unwrap()
        .admit()
        .unwrap();
    assert_eq!(work().encoded_bytes, encoded.len());
    assert_eq!(work().materializations, 0);
    let copied = admitted.into_owned();
    assert_eq!(copied, session);
    assert_eq!(serde_json::to_vec(&copied).unwrap(), encoded);
    assert_eq!(work().materializations, 1);
}

#[test]
fn complete_encoded_budget_accepts_exact_limit_and_rejects_one_byte_without_copying() {
    for field in ["api_key", "account_name", "wr_ticket", "wr_wrpa"] {
        let mut session = session();
        let size = serde_json::to_vec(&session).unwrap().len();
        let padding = "x".repeat(AuthSession::MAX_ENCODED_BYTES - size);
        let selected = match field {
            "api_key" => &mut session.api_key,
            "account_name" => &mut session.account_name,
            "wr_ticket" => &mut session.wr_ticket,
            _ => &mut session.wr_wrpa,
        };
        selected.push_str(&padding);
        assert_eq!(
            serde_json::to_vec(&session).unwrap().len(),
            AuthSession::MAX_ENCODED_BYTES
        );
        reset_work();
        assert!(session.validate_resource_budget().is_ok());
        assert_eq!(work().encoded_bytes, AuthSession::MAX_ENCODED_BYTES);
        assert_eq!(work().materializations, 0);
        let selected = match field {
            "api_key" => &mut session.api_key,
            "account_name" => &mut session.account_name,
            "wr_ticket" => &mut session.wr_ticket,
            _ => &mut session.wr_wrpa,
        };
        selected.push('x');
        reset_work();
        assert_capacity(session.validate_resource_budget());
    }
}

#[test]
fn escaping_expansion_and_combined_fields_are_rejected_from_borrowed_data() {
    let mut session = session();
    session.account_name = "\u{1}".repeat(AuthSession::MAX_ENCODED_BYTES / 5);
    assert!(session.account_name.len() < AuthSession::MAX_ENCODED_BYTES);
    reset_work();
    assert_capacity(session.validate_resource_budget());
    session.api_key = "x".repeat(AuthSession::MAX_ENCODED_BYTES / 2);
    session.account_name = "y".repeat(AuthSession::MAX_ENCODED_BYTES / 2);
    reset_work();
    assert_capacity(session.validate_resource_budget());
}

#[test]
fn scalar_numeric_aliases_keep_exact_text_and_do_not_materialize_before_admission() {
    for number in [
        serde_json::json!(u64::MAX),
        serde_json::json!(i64::MIN),
        serde_json::json!(f64::MAX),
        serde_json::json!(-0.0),
    ] {
        let value = serde_json::json!({"apikey":number,"name":730});
        let key = scalar_text_at(&value, &["apikey"]);
        let name = scalar_text_at(&value, &["name"]);
        let session = session();
        let mut proposal = AuthSessionView::from_session(&session).unwrap();
        proposal.api_key = key.as_str();
        proposal.account_name = name.as_str();
        reset_work();
        let admitted = proposal.admit().unwrap();
        assert_eq!(work().materializations, 0);
        let result = admitted.into_owned();
        assert_eq!(result.api_key, value["apikey"].to_string());
        assert_eq!(result.account_name, "730");
    }
}

#[test]
fn percent_cookie_overlay_encoding_matches_the_protocol_without_preflight_copy() {
    let session = session();
    let refresh = "a b/\u{4e2d}\"";
    let mut proposal = AuthSessionView::from_session(&session).unwrap();
    proposal
        .cookies
        .set("wr_rt", CookieValue::PercentEncoded(refresh));
    proposal
        .cookies
        .set("wr_skey", CookieValue::Text("replacement-session"));
    let mut expected = session.clone();
    expected
        .cookies
        .insert("wr_rt".into(), protocol::encode_component(refresh));
    expected
        .cookies
        .insert("wr_skey".into(), "replacement-session".into());
    reset_work();
    let admitted = proposal.admit().unwrap();
    assert_eq!(
        work().encoded_bytes,
        serde_json::to_vec(&expected).unwrap().len()
    );
    assert_eq!(work().materializations, 0);
    assert_eq!(admitted.into_owned(), expected);
}

#[test]
fn cookie_capacity_remains_atomic_and_is_not_reclassified_as_authentication_failure() {
    let session = session();
    let mut proposal = AuthSessionView::from_session(&session).unwrap();
    proposal.cookies.set("wr_extra", CookieValue::Text("x"));
    let over = "x".repeat(MAX_COOKIE_ENTRY_BYTES);
    proposal.cookies.set("wr_skey", CookieValue::Text(&over));
    reset_work();
    assert!(matches!(
        proposal.admit(),
        Err(SourceError::InvalidResponse("cookie_budget"))
    ));
    assert_eq!(work().materializations, 0);
    assert_eq!(
        session.cookies.get("wr_skey").map(String::as_str),
        Some("fixture-session")
    );
    assert!(!SourceError::ResponseTooLarge.requires_login());
    assert!(!SourceError::ResponseTooLarge.retryable());
}
