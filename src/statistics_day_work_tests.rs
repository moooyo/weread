//! Small clipped ranges exercise local day work before any aggregation map.

use super::*;
use crate::store::Store;

fn at(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn now() -> DateTime<Utc> {
    at("2026-11-03T06:00:00Z")
}

fn session(id: &str, start: i64, end: i64, seconds: u64) -> ReadingSession {
    ReadingSession {
        id: id.into(),
        book_id: "book".into(),
        started_at: start,
        ended_at: end,
        seconds,
    }
}

fn midnight_session() -> ReadingSession {
    session(
        "midnight",
        at("2026-11-02T15:59:30Z").timestamp(),
        at("2026-11-02T16:00:30Z").timestamp(),
        41,
    )
}

fn assert_no_aggregate() {
    let stats = local_statistics_work_stats();
    assert_eq!(stats.admitted_phases, 0);
    assert_eq!(stats.aggregate_calls, 0);
    assert_eq!(stats.day_steps, 0);
    assert_eq!(stats.daily_entries, 0);
    assert_eq!(stats.book_entries, 0);
}

#[test]
fn stored_legal_long_interval_with_one_active_second_rejects_before_day_work() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("library.sqlite")).unwrap();
    store
        .upsert_book(&Book::new("book", "Stored interval fixture"))
        .unwrap();
    let record = session(
        "stored",
        DateTime::<Utc>::MIN_UTC.timestamp(),
        now().timestamp(),
        1,
    );
    store.record_session(&record).unwrap();
    let rows = store.sessions(i64::MIN, i64::MAX).unwrap();
    assert_eq!(rows, [record.clone()]);
    with_local_statistics_test_context(
        LocalStatisticsLimits {
            day_steps: 1,
            ..Default::default()
        },
        || {
            let error = summarize_checked("total", 0, now(), &rows, &[], &[]).unwrap_err();
            assert_eq!(error.kind, LocalStatisticsLimitKind::DaySteps);
            assert!(error.requested > 65536);
            let stats = local_statistics_work_stats();
            assert!(stats.phase_day_forecasts[0] > 65536);
            assert!(stats.phase_day_forecasts[1] > 0);
            assert_no_aggregate();
        },
    );
    assert_eq!(store.sessions(i64::MIN, i64::MAX).unwrap(), [record]);
}

#[test]
fn exact_two_phase_day_work_preserves_midnight_apportioning() {
    let record = midnight_session();
    for maximum in [4, 3] {
        with_local_statistics_test_context(
            LocalStatisticsLimits {
                day_steps: maximum,
                ..Default::default()
            },
            || {
                let result = summarize_checked("week", 0, now(), &[record.clone()], &[], &[]);
                let stats = local_statistics_work_stats();
                assert_eq!(stats.phase_day_forecasts, [2, 2]);
                assert_eq!(stats.day_forecast, 4);
                if maximum == 4 {
                    let report = result.unwrap();
                    assert_eq!(
                        (report.seconds, report.days, report.average_seconds),
                        (41, 2, 20)
                    );
                    assert_eq!((report.bars[0].seconds, report.bars[1].seconds), (20, 21));
                    assert_eq!(report.calendar[1..3], [1, 1]);
                    assert!(report.local_unavailable.is_none());
                    assert_eq!(stats.aggregate_calls, 2);
                    assert_eq!(stats.day_steps, 4);
                    assert_eq!(stats.daily_entries, 4);
                    assert_eq!(stats.book_entries, 2);
                } else {
                    assert_eq!(result.unwrap_err().kind, LocalStatisticsLimitKind::DaySteps);
                    assert_no_aggregate();
                }
            },
        );
    }
}

#[test]
fn separate_main_and_calendar_allowances_cannot_reset_the_operation_budget() {
    with_local_statistics_test_context(
        LocalStatisticsLimits {
            day_steps: 2,
            ..Default::default()
        },
        || {
            let error =
                summarize_checked("week", 0, now(), &[midnight_session()], &[], &[]).unwrap_err();
            assert_eq!((error.limit, error.requested), (2, 4));
            assert_eq!(local_statistics_work_stats().phase_day_forecasts, [2, 2]);
            assert_no_aggregate();
        },
    );
}

#[test]
fn one_active_second_does_not_make_zero_output_day_visits_free() {
    let record = session(
        "sparse-active",
        at("2000-01-01T00:00:00Z").timestamp(),
        at("2100-01-01T00:00:00Z").timestamp(),
        1,
    );
    for maximum in [5, 4] {
        with_local_statistics_test_context(
            LocalStatisticsLimits {
                day_steps: maximum,
                ..Default::default()
            },
            || {
                let result = summarize_checked("week", 0, now(), &[record.clone()], &[], &[]);
                assert_eq!(local_statistics_work_stats().phase_day_forecasts, [2, 3]);
                if maximum == 5 {
                    let report = result.unwrap();
                    assert_eq!((report.seconds, report.days), (0, 0));
                    assert_eq!(local_statistics_work_stats().day_steps, 5);
                    assert_eq!(local_statistics_work_stats().daily_entries, 0);
                } else {
                    assert!(result.is_err());
                    assert_no_aggregate();
                }
            },
        );
    }
}

#[test]
fn invalid_first_duplicate_shadows_later_work_but_not_its_original_raw_range() {
    let old = at("2024-06-01T00:00:00Z").timestamp();
    let recent = now().timestamp() - 60;
    let records = [
        session("same", recent, recent, 1),
        session("same", old, old + 60, 60),
        session("", recent, recent + 60, 60),
        session("", recent, recent + 60, 60),
        session("zero", at("2020-01-01T00:00:00Z").timestamp(), old, 0),
    ];
    let facts = AnnotationStatistics::new("total", 0, now());
    let range = facts.range_checked(&records).unwrap();
    assert_eq!(
        range.0,
        midnight(
            NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(),
            FixedOffset::east_opt(8 * 3600).unwrap()
        )
    );
    with_local_statistics_test_context(LocalStatisticsLimits::default(), || {
        let report = facts.report_checked(&records, &[]).unwrap();
        assert_eq!((report.seconds, report.days), (120, 1));
        assert_eq!(report.range_start, range.0);
        assert_eq!(local_statistics_work_stats().unique_sessions, 4);
        assert_eq!(local_statistics_work_stats().day_steps, 4);
    });
}

#[test]
fn elapsed_overflow_raw_record_retains_earliest_range_without_day_work() {
    let old = NaiveDate::from_ymd_opt(-1, 6, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let records = [
        session("overflow", old, i64::MAX, 1),
        session("normal", now().timestamp() - 60, now().timestamp(), 60),
    ];
    with_local_statistics_test_context(LocalStatisticsLimits::default(), || {
        let report = summarize_checked("total", 0, now(), &records, &[], &[]).unwrap();
        assert_eq!(
            report.range_start,
            midnight(
                NaiveDate::from_ymd_opt(-1, 6, 1).unwrap(),
                FixedOffset::east_opt(8 * 3600).unwrap()
            )
        );
        assert_eq!((report.seconds, report.days), (60, 1));
        assert_eq!(local_statistics_work_stats().day_steps, 2);
    });
}

#[test]
fn input_metadata_and_identifier_work_are_paid_before_projection_or_aggregation() {
    let record = midnight_session();
    for limits in [
        LocalStatisticsLimits {
            scratch_bytes: 575,
            ..Default::default()
        },
        LocalStatisticsLimits {
            identifier_work_bytes: 1,
            ..Default::default()
        },
    ] {
        with_local_statistics_test_context(limits, || {
            let error =
                summarize_checked("week", 0, now(), &[record.clone()], &[], &[]).unwrap_err();
            assert!(matches!(
                error.kind,
                LocalStatisticsLimitKind::ScratchBytes
                    | LocalStatisticsLimitKind::IdentifierWorkBytes
            ));
            assert_eq!(local_statistics_work_stats().unique_sessions, 0);
            assert_no_aggregate();
        });
    }
}

#[test]
fn complete_scratch_forecast_exact_boundary_precedes_both_maps() {
    let record = midnight_session();
    let mut required = 0;
    with_local_statistics_test_context(LocalStatisticsLimits::default(), || {
        summarize_checked("week", 0, now(), &[record.clone()], &[], &[]).unwrap();
        required = local_statistics_work_stats().scratch_forecast;
    });
    assert!(required > 576 && required < 4096);
    for maximum in [required, required - 1] {
        with_local_statistics_test_context(
            LocalStatisticsLimits {
                scratch_bytes: maximum,
                ..Default::default()
            },
            || {
                let result = summarize_checked("week", 0, now(), &[record.clone()], &[], &[]);
                assert_eq!(local_statistics_work_stats().scratch_forecast, required);
                if maximum == required {
                    assert_eq!(result.unwrap().seconds, 41);
                } else {
                    assert_eq!(
                        result.unwrap_err().kind,
                        LocalStatisticsLimitKind::ScratchBytes
                    );
                    assert_no_aggregate();
                }
            },
        );
    }
}

#[test]
fn checked_rejection_and_legacy_report_never_claim_a_new_available_zero_range() {
    let record = midnight_session();
    with_local_statistics_test_context(
        LocalStatisticsLimits {
            day_steps: 0,
            ..Default::default()
        },
        || {
            assert!(summarize_checked("week", 0, now(), &[record.clone()], &[], &[]).is_err());
            let legacy = summarize("week", 0, now(), &[record], &[], &[]);
            assert!(legacy.local_unavailable.is_some());
            assert!(
                legacy.bars.is_empty() && legacy.ranking.is_empty() && legacy.calendar.is_empty()
            );
            assert_eq!((legacy.range_start, legacy.range_end), (0, 0));
            assert_no_aggregate();
        },
    );
}

#[test]
fn chrono_minimum_local_midnight_and_an_unrepresentable_first_cursor_keep_old_behavior() {
    let minimum = DateTime::<Utc>::MIN_UTC;
    let fixed_now = minimum.checked_add_signed(Duration::days(2)).unwrap();
    let record = session(
        "min-valid",
        minimum.timestamp(),
        minimum.timestamp() + 60,
        60,
    );
    let report = summarize_checked("total", 0, fixed_now, &[record], &[], &[]).unwrap();
    assert_eq!((report.seconds, report.days), (60, 1));
    assert!(report.range_start < minimum.timestamp());
    let local_minimum = midnight(NaiveDate::MIN, FixedOffset::east_opt(8 * 3600).unwrap());
    let record = session(
        "min-first-invalid",
        local_minimum,
        minimum.timestamp() + 60,
        60,
    );
    let annotation = Annotation {
        created_at: minimum.timestamp(),
        ..Default::default()
    };
    with_local_statistics_test_context(
        LocalStatisticsLimits {
            day_steps: 2,
            ..Default::default()
        },
        || {
            let report =
                summarize_checked("total", 0, fixed_now, &[record], &[], &[annotation]).unwrap();
            assert_eq!((report.seconds, report.days, report.annotations), (0, 0, 1));
            assert_eq!(local_statistics_work_stats().day_steps, 2);
            assert_eq!(local_statistics_work_stats().daily_entries, 0);
        },
    );
}

#[test]
fn invalid_local_upper_boundary_is_typed_and_test_overrides_cannot_raise_the_ceiling() {
    assert_eq!(
        AnnotationStatistics::new("total", 0, DateTime::<Utc>::MAX_UTC)
            .range_checked(&[])
            .unwrap_err()
            .kind,
        LocalStatisticsLimitKind::DateBoundary
    );
    let record = session(
        "long",
        DateTime::<Utc>::MIN_UTC.timestamp(),
        now().timestamp(),
        1,
    );
    with_local_statistics_test_context(
        LocalStatisticsLimits {
            day_steps: usize::MAX,
            scratch_bytes: usize::MAX,
            identifier_work_bytes: usize::MAX,
        },
        || {
            let error = summarize_checked("total", 0, now(), &[record], &[], &[]).unwrap_err();
            assert_eq!(error.limit, 65536);
            assert_eq!(error.kind, LocalStatisticsLimitKind::DaySteps);
            assert_no_aggregate();
        },
    );
}

#[test]
fn normal_rankings_preferences_and_future_clipping_keep_integer_allocation() {
    let current = now().timestamp();
    let mut first = Book::new("a", "First");
    first.category = "Essay".into();
    first.finished = true;
    let mut second = Book::new("b", "Second");
    second.category = "Novel".into();
    let records = [
        ReadingSession {
            book_id: "a".into(),
            ..session("a", current - 60, current, 60)
        },
        ReadingSession {
            book_id: "b".into(),
            ..session("b", current - 60, current + 60, 120)
        },
    ];
    let report = summarize_checked("month", 5, now(), &records, &[first, second], &[]).unwrap();
    assert_eq!(
        (report.seconds, report.days, report.finished, report.offset),
        (120, 1, 1, 0)
    );
    assert_eq!(
        report
            .ranking
            .iter()
            .map(|rank| (rank.book_id.as_str(), rank.seconds))
            .collect::<Vec<_>>(),
        [("a", 60), ("b", 60)]
    );
    assert_eq!(
        report
            .preferences
            .iter()
            .map(|preference| preference.percent)
            .collect::<Vec<_>>(),
        [50, 50]
    );
    assert!(report.local_unavailable.is_none());
}
