//! Small borrowed cloud working sets and quota rejection before field copies.

use super::*;
use serde_json::json;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-08T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn response() -> ReadStatistics {
    ReadStatistics {
        raw: json!({
            "totalReadTime": 900, "readDays": 2, "dayAverageReadTime": 450,
            "readStat": [{"stat":"", "counts":"skip"}, {"stat":3, "counts":7},
                {"stat":"Selected label", "counts":"Selected value"}, {"stat":"Hidden label", "counts":"x".repeat(4096)}],
            "readLongest": [
                {"book":{"title":""}, "readTime":"x".repeat(4096)},
                {"book":{"bookId":123, "title":"", "author":""}, "albumInfo":{"name":"Album fallback", "authorName":"Unused fallback"}, "readTime":"10"},
                {"book":{"title":0}, "albumInfo":{"authorName":5}, "readTime":20},
                {"book":{"bookId":"hidden-rank", "title":"x".repeat(4096), "author":"y".repeat(4096)}, "readTime":900}
            ],
            "preferCategory":[{"categoryTitle":"Same", "readingTime":1}, {"categoryTitle":"Same", "readingTime":"2"},
                {"categoryTitle":7, "readingTime":3}, {"categoryTitle":"", "readingTime":"x".repeat(4096)}]
        }),
        ..Default::default()
    }
}

fn selected(remote: &ReadStatistics) -> Result<CloudReport, RemoteProjectionUnavailable> {
    from_remote_selected_checked(
        remote,
        "week",
        0,
        now(),
        RemoteProjectionSelection {
            summary: 2,
            ranking: 2,
        },
    )
}

#[test]
fn cloud_selection_preserves_aliases_first_order_hidden_rank_scale_and_duplicate_categories() {
    let remote = response();
    let raw = remote.raw.clone();
    with_remote_projection_limits(RemoteProjectionLimits::default(), || {
        let report = selected(&remote).unwrap();
        assert_eq!(report.summary[0].label, "3");
        assert_eq!(report.summary[0].value, "7");
        assert_eq!(report.summary[1].label, "Selected label");
        assert_eq!(report.ranking[0].book_id, "123");
        assert_eq!(report.ranking[0].title, "Album fallback");
        assert_eq!(report.ranking[0].author, "");
        assert_eq!(report.ranking[1].title, "0");
        assert_eq!(report.ranking[1].author, "5");
        assert_eq!(report.ranking_max_seconds, 900);
        assert_eq!(
            report
                .preferences
                .iter()
                .map(|entry| entry.category.as_str())
                .collect::<Vec<_>>(),
            vec!["Same", "Same", "7"]
        );
        assert_eq!(
            report
                .preferences
                .iter()
                .map(|entry| entry.percent)
                .collect::<Vec<_>>(),
            vec![16, 33, 50]
        );
        let stats = remote_projection_stats();
        assert_eq!((stats.summaries, stats.ranks, stats.categories), (2, 2, 3));
        assert!(stats.text_bytes < 256);
        assert!(stats.work_bytes < 4096);
    });
    assert_eq!(remote.raw, raw);
}

#[test]
fn complete_category_and_selected_field_fees_fail_before_any_output_copy() {
    let remote = response();
    let mut accepted = RemoteProjectionStats::default();
    with_remote_projection_limits(RemoteProjectionLimits::default(), || {
        selected(&remote).unwrap();
        accepted = remote_projection_stats();
    });
    for limits in [
        RemoteProjectionLimits {
            metadata_bytes: accepted.metadata_bytes - 1,
            ..Default::default()
        },
        RemoteProjectionLimits {
            work_bytes: accepted.work_bytes - 1,
            ..Default::default()
        },
        RemoteProjectionLimits {
            records: 6,
            ..Default::default()
        },
    ] {
        with_remote_projection_limits(limits, || {
            assert!(selected(&remote).is_err());
            assert_eq!(remote_projection_stats().text_copies, 0);
            assert_eq!(remote_projection_stats().text_bytes, 0);
        });
    }
    with_remote_projection_limits(
        RemoteProjectionLimits {
            records: 7,
            metadata_bytes: accepted.metadata_bytes,
            work_bytes: accepted.work_bytes,
        },
        || {
            assert!(selected(&remote).is_ok());
        },
    );
}

#[test]
fn zero_category_total_returns_the_original_empty_projection_without_copying_names() {
    let remote = ReadStatistics {
        raw: json!({"preferCategory":[
            {"categoryTitle":"Retained zero category", "readingTime":0}, {"categoryTitle":5, "readingTime":"0"}
        ]}),
        ..Default::default()
    };
    with_remote_projection_limits(
        RemoteProjectionLimits {
            records: 0,
            ..Default::default()
        },
        || {
            let report = selected(&remote).unwrap();
            assert!(report.preferences.is_empty());
            assert_eq!(remote_projection_stats().text_copies, 0);
        },
    );
}

#[test]
fn rejected_compatibility_projection_retains_scalar_summary_and_typed_unavailability() {
    let remote = response();
    let raw = remote.raw.clone();
    with_remote_projection_limits(
        RemoteProjectionLimits {
            metadata_bytes: 0,
            ..Default::default()
        },
        || {
            let report = from_remote(&remote, "week", 0, now());
            assert_eq!(report.seconds, Some(900));
            assert_eq!(report.days, Some(2));
            assert!(report.projection_unavailable.is_some());
            assert!(report.missing_details);
            assert!(
                report.summary.is_empty()
                    && report.ranking.is_empty()
                    && report.preferences.is_empty()
            );
            assert_eq!(remote_projection_stats().text_copies, 0);
        },
    );
    assert_eq!(remote.raw, raw);
}

#[test]
fn annual_span_unavailability_remains_separate_from_selected_detail_admission() {
    let key = DateTime::<Utc>::MIN_UTC.timestamp().to_string();
    let remote = ReadStatistics {
        raw: json!({"totalReadTime":99, "readTimes":{key:1},
        "readStat":[{"stat":"First", "counts":"1"}, {"stat":"Second", "counts":"2"}]}),
        ..Default::default()
    };
    let report = from_remote_selected_checked(
        &remote,
        "total",
        0,
        now(),
        RemoteProjectionSelection {
            summary: 2,
            ranking: 4,
        },
    )
    .unwrap();
    assert!(report.bars_unavailable && report.bars.is_empty());
    assert_eq!(report.seconds, Some(99));
    assert_eq!(report.summary.len(), 2);
    assert!(report.projection_unavailable.is_none());
}
