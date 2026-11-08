//! Borrowed cached-field mapping uses the same resource admission as transport.

use super::*;
use serde_json::json;

#[test]
fn cached_underlines_preserve_unicode_numeric_ranges_and_saved_thought_counts() {
    let records = vec![json!({
        "range": 42, "text": "A\u{1f600}\u{4e2d}", "count": 7,
        "thoughts": 9, "reviewCount": 30,
    })];
    let mapped = WeReadClient::normalize_cached_underlines(&records).unwrap();
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].range, "42");
    assert_eq!(mapped[0].text, "A\u{1f600}\u{4e2d}");
    assert_eq!((mapped[0].count, mapped[0].thoughts), (7, 9));
    WeReadClient::validate_underlines(&mapped).unwrap();
    // The original transport contract still uses its review-count aliases.
    let wire =
        normalize_underlines(&json!({"underlines": records}), UnderlineLimits::default()).unwrap();
    assert_eq!(wire[0].thoughts, 30);
}

#[test]
fn cached_fields_reject_count_quote_range_and_total_before_owned_mapping() {
    let limits = UnderlineLimits {
        records: 2,
        quote_bytes: 4,
        range_bytes: 3,
        total_bytes: 6,
    };
    let cases = [
        vec![json!({}), json!({}), json!({})],
        vec![json!({"text":"12345"})],
        vec![json!({"range":"1234"})],
        vec![
            json!({"range":"a", "text":"123"}),
            json!({"range":"b", "text":"12"}),
        ],
    ];
    for records in cases {
        let error = normalize_underline_records(
            &records,
            limits,
            CACHED_UNDERLINE_TEXT_FIELDS,
            CACHED_UNDERLINE_THOUGHT_FIELDS,
        )
        .unwrap_err();
        assert!(matches!(&error, SourceError::ResponseTooLarge));
        assert!(!error.requires_login() && !error.retryable());
    }
    let admitted = normalize_underline_records(
        &[
            json!({"range":"a", "text":"12", "thoughts":3}),
            json!({"range":"b", "text":"34", "thoughts":4}),
        ],
        limits,
        CACHED_UNDERLINE_TEXT_FIELDS,
        CACHED_UNDERLINE_THOUGHT_FIELDS,
    )
    .unwrap();
    assert_eq!(admitted.len(), 2);
    assert_eq!((admitted[0].thoughts, admitted[1].thoughts), (3, 4));
}

#[test]
fn cached_primary_fields_ignore_unused_wire_fallbacks_and_keep_legacy_aliases() {
    let limits = UnderlineLimits {
        records: 2,
        quote_bytes: 5,
        range_bytes: 3,
        total_bytes: 10,
    };
    let records = vec![
        json!({"text":"x", "markText":"unused oversized fallback", "thoughts":2, "reviewCount":90}),
        json!({"bookmarkText":"old", "range":"r", "underlineCount":4, "reviewsCount":5}),
    ];
    let mapped = normalize_underline_records(
        &records,
        limits,
        CACHED_UNDERLINE_TEXT_FIELDS,
        CACHED_UNDERLINE_THOUGHT_FIELDS,
    )
    .unwrap();
    assert_eq!((mapped[0].text.as_str(), mapped[0].thoughts), ("x", 2));
    assert_eq!(
        (mapped[1].text.as_str(), mapped[1].count, mapped[1].thoughts),
        ("old", 4, 5)
    );
}
