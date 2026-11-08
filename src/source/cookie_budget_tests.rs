//! Synthetic cookie collections exercise resource limits without network I/O.

use super::*;
use reqwest::header::{HeaderMap, HeaderValue, SET_COOKIE};

fn response_headers<I, S>(lines: I) -> HeaderMap
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut headers = HeaderMap::new();
    for line in lines {
        headers.append(
            SET_COOKIE,
            HeaderValue::from_bytes(line.as_ref().as_bytes())
                .expect("The synthetic cookie is a valid header"),
        );
    }
    headers
}

fn wire_bytes(cookies: &BTreeMap<String, String>) -> usize {
    cookies
        .iter()
        .map(|(name, value)| name.len() + 1 + value.len())
        .sum::<usize>()
        + cookies.len().saturating_sub(1) * 2
}

fn wire_entry(name: &str, bytes: usize) -> (String, String) {
    let prefix = name.len() + 1;
    assert!(bytes > prefix, "A fixture cookie needs a nonempty value");
    (name.to_owned(), "x".repeat(bytes - prefix))
}

fn count_jar(prefix: &str, count: usize) -> BTreeMap<String, String> {
    (0..count)
        .map(|index| (format!("wr_{prefix}_{index:04}"), "fixture-value".into()))
        .collect()
}

fn total_jar(bytes: usize) -> BTreeMap<String, String> {
    let mut cookies = BTreeMap::new();
    for index in 0..7 {
        let (name, value) = wire_entry(&format!("wr_bytes_{index:02}"), MAX_COOKIE_ENTRY_BYTES);
        cookies.insert(name, value);
    }
    let final_size = bytes - wire_bytes(&cookies) - 2;
    assert!(final_size <= MAX_COOKIE_ENTRY_BYTES);
    let (name, value) = wire_entry("wr_bytes_07", final_size);
    cookies.insert(name, value);
    assert_eq!(wire_bytes(&cookies), bytes);
    cookies
}

fn snapshot(cookies: &BTreeMap<String, String>) -> Vec<u8> {
    serde_json::to_vec(cookies).unwrap()
}

fn expect_cookie_budget<T>(result: Result<T>) {
    let error = match result {
        Ok(_) => panic!("The oversized cookie collection was accepted"),
        Err(error) => error,
    };
    assert!(matches!(
        &error,
        SourceError::InvalidResponse("cookie_budget")
    ));
    assert!(
        !error.retryable(),
        "A local resource refusal must not retry"
    );
    assert!(
        !error.requires_login(),
        "A resource refusal must not invalidate account credentials"
    );
}

fn account(cookies: BTreeMap<String, String>) -> AuthSession {
    AuthSession {
        api_key: "synthetic-cookie-budget-api-key".into(),
        account_name: "Synthetic cookie-budget account".into(),
        user_vid: "synthetic-cookie-account".into(),
        login_time: 42,
        cookies,
        ..AuthSession::default()
    }
}

#[test]
fn independent_responses_stop_at_the_count_limit_without_mutating_the_old_jar() {
    let mut cookies = BTreeMap::new();
    for index in 0..MAX_COOKIE_COUNT {
        merge_response_cookies(
            &mut cookies,
            &response_headers([format!("wr_growth_{index:04}=fixture")]),
        )
        .unwrap();
        assert_eq!(cookies.len(), index + 1);
    }
    validate_cookie_jar(&cookies).unwrap();
    let before = snapshot(&cookies);
    let header = cookie_header(&cookies).unwrap();
    for index in MAX_COOKIE_COUNT..MAX_COOKIE_COUNT + 512 {
        expect_cookie_budget(merge_response_cookies(
            &mut cookies,
            &response_headers([format!("wr_growth_{index:04}=fixture")]),
        ));
        assert_eq!(snapshot(&cookies), before);
        assert_eq!(cookie_header(&cookies).unwrap(), header);
    }
}

#[test]
fn a_cookie_entry_budget_includes_its_name_equals_sign_and_value() {
    let (name, value) = wire_entry("wr_entry_edge", MAX_COOKIE_ENTRY_BYTES);
    let exact = BTreeMap::from([(name.clone(), value.clone())]);
    validate_cookie_jar(&exact).unwrap();
    assert_eq!(cookie_header(&exact).unwrap().len(), MAX_COOKIE_ENTRY_BYTES);

    let mut oversized = exact;
    oversized.get_mut(&name).unwrap().push('x');
    expect_cookie_budget(validate_cookie_jar(&oversized));
    expect_cookie_budget(cookie_header(&oversized));
    let mut empty = BTreeMap::new();
    expect_cookie_budget(merge_response_cookies(
        &mut empty,
        &response_headers([format!("{name}={value}x")]),
    ));
    assert!(empty.is_empty());

    let long_name = format!("wr_{}", "n".repeat(MAX_COOKIE_ENTRY_BYTES - 4));
    let name_overflow = BTreeMap::from([(long_name, "x".into())]);
    assert_eq!(wire_bytes(&name_overflow), MAX_COOKIE_ENTRY_BYTES + 1);
    expect_cookie_budget(validate_cookie_jar(&name_overflow));
    expect_cookie_budget(cookie_header(&name_overflow));
}

#[test]
fn total_wire_budget_counts_every_separator_and_accepts_the_exact_boundary() {
    for size in [MAX_COOKIE_TOTAL_BYTES - 1, MAX_COOKIE_TOTAL_BYTES] {
        let cookies = total_jar(size);
        assert!(cookies.values().all(|value| !value.is_empty()));
        validate_cookie_jar(&cookies).unwrap();
        assert_eq!(cookie_header(&cookies).unwrap().len(), size);
        let mut merged = BTreeMap::new();
        merge_response_cookies(
            &mut merged,
            &response_headers(
                cookies
                    .iter()
                    .map(|(name, value)| format!("{name}={value}")),
            ),
        )
        .unwrap();
        assert_eq!(merged, cookies);
    }
    let oversized = total_jar(MAX_COOKIE_TOTAL_BYTES + 1);
    assert!(oversized.len() < MAX_COOKIE_COUNT);
    assert!(
        oversized
            .iter()
            .all(|(name, value)| name.len() + 1 + value.len() <= MAX_COOKIE_ENTRY_BYTES)
    );
    expect_cookie_budget(validate_cookie_jar(&oversized));
    expect_cookie_budget(cookie_header(&oversized));
    let mut merged = BTreeMap::new();
    expect_cookie_budget(merge_response_cookies(
        &mut merged,
        &response_headers(
            oversized
                .iter()
                .map(|(name, value)| format!("{name}={value}")),
        ),
    ));
    assert!(merged.is_empty());
}

#[test]
fn replacing_and_deleting_near_the_limit_reuses_the_recovered_budget() {
    let mut cookies = total_jar(MAX_COOKIE_TOTAL_BYTES);
    let replacement = "y".repeat(cookies["wr_bytes_07"].len());
    merge_response_cookies(
        &mut cookies,
        &response_headers([format!("wr_bytes_07={replacement}")]),
    )
    .unwrap();
    assert_eq!(wire_bytes(&cookies), MAX_COOKIE_TOTAL_BYTES);
    assert_eq!(cookies["wr_bytes_07"], replacement);

    let before = snapshot(&cookies);
    expect_cookie_budget(merge_response_cookies(
        &mut cookies,
        &response_headers([format!("wr_bytes_07={replacement}x")]),
    ));
    assert_eq!(snapshot(&cookies), before);
    merge_response_cookies(&mut cookies, &response_headers(["wr_bytes_00=; Max-Age=0"])).unwrap();
    assert!(!cookies.contains_key("wr_bytes_00"));
    merge_response_cookies(
        &mut cookies,
        &response_headers([format!("wr_bytes_07={replacement}x")]),
    )
    .unwrap();
    let (name, value) = wire_entry("wr_reused", MAX_COOKIE_ENTRY_BYTES - 1);
    merge_response_cookies(&mut cookies, &response_headers([format!("{name}={value}")])).unwrap();
    assert_eq!(
        cookie_header(&cookies).unwrap().len(),
        MAX_COOKIE_TOTAL_BYTES
    );
}

#[test]
fn duplicate_names_keep_the_final_update_or_delete_in_the_response() {
    let mut cookies = BTreeMap::from([("wr_skey".into(), "fixture-old-token".into())]);
    merge_response_cookies(
        &mut cookies,
        &response_headers([
            "wr_skey=fixture-first-token; Path=/",
            "wr_new=first",
            "wr_skey=fixture-final-token; Path=/",
            "wr_new=last",
        ]),
    )
    .unwrap();
    assert_eq!(cookies["wr_skey"], "fixture-final-token");
    assert_eq!(cookies["wr_new"], "last");
    merge_response_cookies(
        &mut cookies,
        &response_headers([
            "wr_skey=fixture-rotated-token",
            "wr_skey=; Max-Age=0",
            "wr_new=; Max-Age=0",
            "wr_new=restored",
        ]),
    )
    .unwrap();
    assert!(!cookies.contains_key("wr_skey"));
    assert_eq!(cookies["wr_new"], "restored");
}

#[test]
fn a_full_jar_can_atomically_replace_all_names_in_either_header_order() {
    let original = count_jar("old", MAX_COOKIE_COUNT);
    let replacement = count_jar("new", MAX_COOKIE_COUNT);
    let updates: Vec<_> = replacement
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    let deletions: Vec<_> = original.keys().map(|name| format!("{name}=")).collect();
    for insert_first in [true, false] {
        let mut cookies = original.clone();
        let lines = if insert_first {
            updates.iter().chain(&deletions)
        } else {
            deletions.iter().chain(&updates)
        };
        merge_response_cookies(&mut cookies, &response_headers(lines)).unwrap();
        assert_eq!(cookies, replacement);
        validate_cookie_jar(&cookies).unwrap();
    }
}

#[test]
fn a_refused_batch_never_partially_rotates_a_valid_session_cookie() {
    let mut cookies = count_jar("retained", MAX_COOKIE_COUNT - 1);
    cookies.insert("wr_skey".into(), "fixture-old-session".into());
    let before = snapshot(&cookies);
    expect_cookie_budget(merge_response_cookies(
        &mut cookies,
        &response_headers([
            "wr_skey=fixture-rotated-session",
            "wr_one_cookie_too_many=fixture",
        ]),
    ));
    assert_eq!(snapshot(&cookies), before);
    assert_eq!(cookies["wr_skey"], "fixture-old-session");

    let mut small = BTreeMap::from([("wr_skey".into(), "fixture-old-session".into())]);
    let before = snapshot(&small);
    expect_cookie_budget(merge_response_cookies(
        &mut small,
        &response_headers([
            "wr_skey=fixture-rotated-session".to_owned(),
            format!("wr_oversized={}", "x".repeat(MAX_COOKIE_ENTRY_BYTES)),
        ]),
    ));
    assert_eq!(snapshot(&small), before);
}

#[test]
fn too_many_meaningful_updates_are_refused_before_later_deletes_can_hide_them() {
    let count = MAX_COOKIE_COUNT * 2 + 1;
    let mut lines: Vec<_> = (0..count)
        .map(|index| format!("wr_pending_{index:04}=fixture"))
        .collect();
    lines.extend((0..count).map(|index| format!("wr_pending_{index:04}=")));
    let mut cookies = BTreeMap::new();
    expect_cookie_budget(merge_response_cookies(
        &mut cookies,
        &response_headers(lines),
    ));
    assert!(cookies.is_empty());
}

#[test]
fn deleting_absent_names_does_not_consume_pending_update_capacity() {
    let mut cookies = BTreeMap::from([("wr_skey".into(), "fixture-retained-token".into())]);
    let before = snapshot(&cookies);
    merge_response_cookies(
        &mut cookies,
        &response_headers((0..4096).map(|index| format!("wr_absent_{index:04}=; Max-Age=0"))),
    )
    .unwrap();
    assert_eq!(snapshot(&cookies), before);
}

#[test]
fn client_construction_replacement_and_header_building_refuse_oversized_collections() {
    let original = account(BTreeMap::from([
        ("wr_vid".into(), "synthetic-cookie-account".into()),
        ("wr_skey".into(), "fixture-old-session".into()),
    ]));
    let client = WeReadClient::new(original.clone()).unwrap();
    let shared = client.clone();
    let (name, mut value) = wire_entry("wr_entry", MAX_COOKIE_ENTRY_BYTES);
    value.push('x');
    let invalid_collections = [
        count_jar("invalid_count", MAX_COOKIE_COUNT + 1),
        total_jar(MAX_COOKIE_TOTAL_BYTES + 1),
        BTreeMap::from([(name, value)]),
    ];
    for cookies in invalid_collections {
        expect_cookie_budget(validate_cookie_jar(&cookies));
        expect_cookie_budget(cookie_header(&cookies));
        let mut incoming = account(cookies);
        incoming.api_key = "synthetic-unaccepted-api-key".into();
        incoming.user_vid = "synthetic-unaccepted-account".into();
        expect_cookie_budget(WeReadClient::new(incoming.clone()));
        expect_cookie_budget(client.replace_session(incoming));
        assert_eq!(client.session(), original);
        assert_eq!(shared.session(), original);
    }
}
