//! Borrowed credential overlays reject before copies, encoding or authenticated GETs.

use super::super::client::api_gate_test_support::{HttpFixture, Reply, auth};
use super::*;
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Work {
    materializations: usize,
    copied_bytes: usize,
    refresh_encodes: usize,
    refresh_input_bytes: usize,
}

thread_local! {
    static WORK: Cell<Work> = const {
        Cell::new(Work { materializations: 0, copied_bytes: 0, refresh_encodes: 0, refresh_input_bytes: 0 })
    };
}

pub(super) fn observe_credential_copy(bytes: usize) {
    WORK.with(|slot| {
        let mut work = slot.get();
        work.materializations += 1;
        work.copied_bytes += bytes;
        slot.set(work);
    });
}

pub(super) fn observe_refresh_encoding(bytes: usize) {
    WORK.with(|slot| {
        let mut work = slot.get();
        work.refresh_encodes += 1;
        work.refresh_input_bytes += bytes;
        slot.set(work);
    });
}

fn reset() {
    WORK.with(|slot| slot.set(Work::default()));
}

fn work() -> Work {
    WORK.with(Cell::get)
}

fn credentials() -> Value {
    serde_json::json!({"webLoginVid":"v","accessToken":"s","refreshToken":""})
}

fn assert_budget<T>(result: Result<T>) {
    assert!(matches!(
        result,
        Err(SourceError::InvalidResponse("cookie_budget"))
    ));
    assert_eq!(work(), Work::default());
}

fn padding(total_entries_bytes: usize) -> BTreeMap<String, String> {
    let mut cookies = BTreeMap::new();
    let mut remaining = total_entries_bytes;
    for index in 0..8 {
        let name = format!("wr_pad_{index}");
        let entry = if index < 7 {
            MAX_COOKIE_ENTRY_BYTES
        } else {
            remaining
        };
        let value_bytes = entry - name.len() - 1;
        cookies.insert(name, "x".repeat(value_bytes));
        remaining -= entry;
    }
    assert_eq!(remaining, 0);
    validate_cookie_jar(&cookies).unwrap();
    cookies
}

#[test]
fn selected_string_fields_are_borrowed_and_numeric_credential_aliases_remain_valid() {
    let value = credentials();
    reset();
    let selected = checked_login_credentials(&value, &BTreeMap::new()).unwrap();
    assert!(matches!(selected.user_vid, ScalarText::Borrowed("v")));
    assert!(matches!(selected.access_token, ScalarText::Borrowed("s")));
    assert_eq!(work(), Work::default());
    let numeric = serde_json::json!({"webLoginVid":123,"accessToken":456,"refreshToken":789});
    let selected = checked_login_credentials(&numeric, &BTreeMap::new()).unwrap();
    assert_eq!(selected.user_vid.as_str(), "123");
    assert_eq!(selected.access_token.as_str(), "456");
    assert_eq!(selected.refresh_token.as_str(), "789");
    assert_eq!(work(), Work::default());
    for value in [
        serde_json::json!(u64::MAX),
        serde_json::json!(i64::MIN),
        serde_json::json!(f64::MAX),
        serde_json::json!(f64::MIN_POSITIVE),
        serde_json::json!(0.0),
        serde_json::json!(-0.0),
    ] {
        let scalar = super::super::client::borrowed_scalar_text(&value).unwrap();
        assert_eq!(scalar.as_str(), value.to_string());
        assert!(scalar.len() < 64);
    }
    for invalid in [Value::Null, serde_json::json!(true), serde_json::json!([])] {
        let mut value = credentials();
        value["webLoginVid"] = invalid;
        assert!(matches!(
            checked_login_credentials(&value, &BTreeMap::new()),
            Err(SourceError::InvalidResponse("login_credentials")),
        ));
    }
}

#[test]
fn every_credential_entry_accepts_its_exact_key_and_equals_byte_limit_before_copy() {
    for (field, name) in [
        ("webLoginVid", "wr_vid"),
        ("accessToken", "wr_skey"),
        ("refreshToken", "wr_rt"),
    ] {
        let maximum = MAX_COOKIE_ENTRY_BYTES - name.len() - 1;
        let mut value = credentials();
        value[field] = serde_json::json!("x".repeat(maximum));
        reset();
        assert!(checked_login_credentials(&value, &BTreeMap::new()).is_ok());
        assert_eq!(work(), Work::default());
        value[field] = serde_json::json!("x".repeat(maximum + 1));
        assert_budget(checked_login_credentials(&value, &BTreeMap::new()));
    }
}

#[test]
fn refresh_percent_encoding_is_priced_exactly_for_ascii_and_unicode_before_allocation() {
    for (accepted, rejected) in [
        ("!".repeat(2728), "!".repeat(2729)),
        ("\u{4e2d}".repeat(909), "\u{4e2d}".repeat(910)),
    ] {
        let mut value = credentials();
        value["refreshToken"] = serde_json::json!(accepted);
        reset();
        assert!(checked_login_credentials(&value, &BTreeMap::new()).is_ok());
        assert_eq!(work(), Work::default());
        value["refreshToken"] = serde_json::json!(rejected);
        assert_budget(checked_login_credentials(&value, &BTreeMap::new()));
    }
}

#[test]
fn overlays_count_all_keys_and_separators_and_cannot_use_refresh_to_repair_first_stage() {
    // Three new fixed entries cost 24 bytes plus 20 separators at count 11.
    let mut cookies = padding(65_492);
    let value = credentials();
    reset();
    assert!(checked_login_credentials(&value, &cookies).is_ok());
    cookies.get_mut("wr_pad_7").unwrap().push('x');
    let before = cookies.clone();
    assert_budget(checked_login_credentials(&value, &cookies));
    assert_eq!(cookies, before);

    let cookies: BTreeMap<_, _> = (0..253)
        .map(|index| (format!("wr_existing_{index}"), "x".into()))
        .collect();
    reset();
    assert!(checked_login_credentials(&value, &cookies).is_ok());
    let mut overflow = cookies.clone();
    overflow.insert("wr_one_more".into(), "x".into());
    assert_budget(checked_login_credentials(&value, &overflow));

    let mut first_stage = padding(64_500);
    first_stage.insert("wr_rt".into(), "x".repeat(1000));
    validate_cookie_jar(&first_stage).unwrap();
    let mut value = credentials();
    value["refreshToken"] = serde_json::json!("r");
    reset();
    assert_budget(checked_login_credentials(&value, &first_stage));
}

#[test]
fn replacement_lengths_skip_old_values_and_missing_credentials_keep_error_precedence() {
    let mut cookies = BTreeMap::from([
        ("wr_vid".into(), "old".repeat(100)),
        ("wr_skey".into(), "old".repeat(100)),
        ("wr_ql".into(), "1".into()),
        ("wr_rt".into(), "old-refresh".into()),
    ]);
    let before = cookies.clone();
    reset();
    assert!(checked_login_credentials(&credentials(), &cookies).is_ok());
    assert_eq!(cookies, before);
    cookies.extend((0..256).map(|index| (format!("wr_extra_{index}"), "x".into())));
    let mut value = credentials();
    value["webLoginVid"] = serde_json::json!("");
    assert!(matches!(
        checked_login_credentials(&value, &cookies),
        Err(SourceError::InvalidResponse("login_credentials")),
    ));
    assert_eq!(work(), Work::default());
}

fn flow(fixture: &HttpFixture) -> LoginFlow {
    let mut flow = LoginFlow::new().unwrap();
    flow.http = fixture.http();
    flow.test_origin = Some(fixture.origin().into());
    flow
}

#[tokio::test(flavor = "current_thread")]
async fn real_poll_budget_errors_preserve_challenge_cookies_and_the_old_account_without_user_gets()
{
    for (field, text) in [
        ("webLoginVid", "v".repeat(MAX_COOKIE_ENTRY_BYTES)),
        ("accessToken", "s".repeat(MAX_COOKIE_ENTRY_BYTES)),
        ("refreshToken", "!".repeat(2729)),
    ] {
        let mut response = credentials();
        response["succeed"] = serde_json::json!(true);
        response[field] = serde_json::json!(text);
        let fixture = HttpFixture::new(move |request| match request.path() {
            "/r/weread-skills" => Reply::text("Ready").cookie("wr_gid=synthetic-bootstrap; Path=/"),
            "/api/auth/getLoginUid" => {
                Reply::json(serde_json::json!({"uid":"synthetic-credential-uid"}))
            }
            "/api/auth/getLoginInfo" => {
                Reply::json(response.clone()).cookie("wr_new=synthetic-poll-rotation; Path=/")
            }
            path => panic!("A rejected credential reached an authenticated request: {path}"),
        });
        let old_client = fixture.client(auth());
        let before_account = old_client.session();
        let flow = flow(&fixture);
        let challenge = flow.begin().await.unwrap();
        let before = flow.state.lock().unwrap().clone();
        reset();
        assert_budget(flow.poll(&challenge.uid, None).await);
        let after = flow.state.lock().unwrap().clone();
        assert_eq!(after.cookies, before.cookies);
        assert_eq!(
            after.challenge.as_ref().unwrap().uid,
            before.challenge.as_ref().unwrap().uid
        );
        assert_eq!(old_client.session(), before_account);
        let records = fixture.finish();
        assert_eq!(records.len(), 3);
        assert!(records.iter().all(|record| record.method == "GET"));
        assert_eq!(records[2].path(), "/api/auth/getLoginInfo");
        assert!(
            !records
                .iter()
                .any(|record| record.path() == "/api/userInfo")
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn real_numeric_login_completion_encodes_admitted_refresh_and_keeps_file_contract() {
    let fixture = HttpFixture::new(|request| match request.path() {
        "/r/weread-skills" => Reply::text("Ready"),
        "/api/auth/getLoginUid" => {
            Reply::json(serde_json::json!({"uid":"synthetic-credential-uid"}))
        }
        "/api/auth/getLoginInfo" => Reply::json(serde_json::json!({
            "succeed":true,"webLoginVid":123,"accessToken":456,"refreshToken":"a b/\u{4e2d}"
        })),
        "/api/userInfo" => Reply::json(serde_json::json!({"name":"Synthetic account"})),
        "/api/skills/apikeyGet" => {
            Reply::json(serde_json::json!({"apikey":"synthetic-enabled-key"}))
        }
        path => panic!("Unexpected login path: {path}"),
    });
    let flow = flow(&fixture);
    let challenge = flow.begin().await.unwrap();
    reset();
    let QrLoginState::Authenticated(session) = flow.poll(&challenge.uid, None).await.unwrap()
    else {
        panic!("The admitted synthetic login did not authenticate");
    };
    assert_eq!(session.user_vid, "123");
    assert_eq!(
        session.cookies.get("wr_vid").map(String::as_str),
        Some("123")
    );
    assert_eq!(
        session.cookies.get("wr_skey").map(String::as_str),
        Some("456")
    );
    assert_eq!(
        session.cookies.get("wr_rt"),
        Some(&encode_component("a b/\u{4e2d}"))
    );
    assert_eq!(work().materializations, 4);
    assert_eq!(work().copied_bytes, 12);
    assert_eq!(work().refresh_encodes, 1);
    let directory = tempfile::tempdir().unwrap();
    let file = crate::session::SessionFile::new(directory.path());
    file.save(&session).unwrap();
    assert_eq!(file.load().unwrap(), Some(session));
    let records = fixture.finish();
    assert_eq!(records.len(), 5);
    assert!(records.iter().all(|record| record.method == "GET"));
    assert!(records[3].target.contains("userVid=123"));
    assert_eq!(
        records[3].headers.get("x-vid").map(String::as_str),
        Some("123")
    );
    assert_eq!(
        records[3].headers.get("x-skey").map(String::as_str),
        Some("456")
    );
}
