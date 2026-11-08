//! Small Total-year admission and genuine App/model-retirement regressions.

use super::*;
use crate::jobs::JobResult;
use crate::statistics::{
    with_year_projection_test_context, year_projection_stats, year_span_admitted_for_test,
};
use chrono::{Datelike, NaiveDate, Utc};
use serde_json::{Value, json};
use slint::platform::{
    Platform as UiPlatform, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::rc::Weak;

const TOTAL_BARS: usize = 512;
const BAR_WORK_BYTES: usize = 512;
const TOTAL_WORK_BYTES: usize = TOTAL_BARS * BAR_WORK_BYTES;

fn now() -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(2026, 11, 7)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
}

fn timestamp(year: i32, month: u32, day: u32) -> i64 {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp()
}

fn remote(read_times: Value) -> source::ReadStatistics {
    source::ReadStatistics {
        raw: json!({
            "totalReadTime": 900,
            "readDays": 2,
            "dayAverageReadTime": 128,
            "readTimes": read_times,
            "readStat": [
                {"stat": "Completed", "counts": "4 books"},
                {"stat": "Notes", "counts": "2 notes"}
            ]
        }),
        ..Default::default()
    }
}

fn read_times(entries: &[(i64, Value)]) -> Value {
    Value::Object(
        entries
            .iter()
            .map(|(timestamp, value)| (timestamp.to_string(), value.clone()))
            .collect(),
    )
}

#[test]
fn chrono_minimum_single_bucket_rejects_before_any_annual_output() {
    let minimum = DateTime::<Utc>::MIN_UTC;
    // These values follow the locked Chrono root/date constants, rather than
    // assuming a service date format or materializing the implied year range.
    assert_eq!(minimum.timestamp(), -8_334_601_228_800);
    assert_eq!(minimum.year(), (i32::MIN >> 13) + 1);
    let implied = i64::from(now().year()) - i64::from(minimum.year()) + 1;
    assert_eq!(implied, 264_170);
    let key = minimum.timestamp().to_string();
    let raw = json!({"readTimes": {key: 1}});
    assert_eq!(serde_json::to_vec(&raw).unwrap().len(), 34);
    let response = source::ReadStatistics {
        raw,
        ..Default::default()
    };
    for period in ["total", "all"] {
        with_year_projection_test_context(TOTAL_BARS, TOTAL_WORK_BYTES, now(), || {
            let report = statistics::from_remote(&response, period, 0, now());
            assert!(report.bars.is_empty());
            assert!(report.bars_unavailable && report.missing_details);
            let stats = year_projection_stats();
            assert_eq!(stats.admission_checks, 1);
            assert_eq!(stats.rejections, 1);
            assert_eq!(stats.admitted_bars, 0);
            assert_eq!(stats.year_buckets, 0);
            assert_eq!(stats.year_visits, 0);
            assert_eq!(stats.bars_created, 0);
            assert_eq!(stats.ui_rows, 0);
        });
    }
}

#[test]
fn exact_year_and_work_boundaries_keep_complete_gaps_and_last_bucket() {
    let response = remote(read_times(&[
        (timestamp(2024, 6, 1), json!(60)),
        (timestamp(2026, 6, 1), json!(120)),
        (timestamp(2027, 1, 1), json!(999)),
    ]));
    let original = response.raw.clone();
    with_year_projection_test_context(3, 3 * BAR_WORK_BYTES, now(), || {
        let report = statistics::from_remote(&response, "total", -9, now());
        assert!(!report.bars_unavailable);
        assert_eq!(report.bars.len(), 3);
        assert_eq!(report.bars[0].label, "2024");
        assert_eq!(report.bars[1].label, "2025");
        assert_eq!(report.bars[2].label, "2026");
        assert!(report.bars[0].available);
        assert!(!report.bars[1].available);
        assert!(report.bars[2].available);
        assert_eq!(report.bars[2].seconds, 120);
        assert!(report.bars.iter().all(|bar| !bar.future));
        assert_eq!(report.offset, 0);
        assert_eq!(report.seconds, Some(900));
        assert_eq!(report.days, Some(2));
        assert_eq!(report.average_seconds, Some(128));
        assert_eq!(year_projection_stats().bars_created, 3);
        assert_eq!(year_projection_stats().year_buckets, 2);
    });
    for (bars, bytes) in [(2, 3 * BAR_WORK_BYTES), (3, 3 * BAR_WORK_BYTES - 1)] {
        with_year_projection_test_context(bars, bytes, now(), || {
            let report = statistics::from_remote(&response, "total", 0, now());
            assert!(report.bars.is_empty());
            assert!(report.bars_unavailable && report.missing_details);
            assert_eq!(report.seconds, Some(900));
            assert_eq!(report.days, Some(2));
            assert_eq!(report.average_seconds, Some(128));
            assert_eq!(report.summary.len(), 2);
            assert_eq!(report.summary[0].value, "4 books");
            assert_eq!(report.period_label, "2024 – 2026");
            assert_eq!(year_projection_stats().bars_created, 0);
            assert_eq!(year_projection_stats().year_buckets, 0);
            assert_eq!(year_projection_stats().rejections, 1);
        });
    }
    let one_year_over = remote(read_times(&[
        (timestamp(2023, 6, 1), json!(60)),
        (timestamp(2026, 6, 1), json!(120)),
    ]));
    with_year_projection_test_context(3, 3 * BAR_WORK_BYTES, now(), || {
        let report = statistics::from_remote(&one_year_over, "total", 0, now());
        assert!(report.bars_unavailable && report.bars.is_empty());
        assert_eq!(report.period_label, "2023 – 2026");
        assert_eq!(year_projection_stats().admitted_bars, 0);
        assert_eq!(year_projection_stats().year_visits, 0);
    });
    assert_eq!(response.raw, original);
}

#[test]
fn negative_years_invalid_values_and_future_extrema_preserve_presence() {
    let fixed_now = NaiveDate::from_ymd_opt(1, 6, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc();
    let response = remote(read_times(&[
        (timestamp(-1, 6, 1), json!(60)),
        (timestamp(-1, 7, 1), json!("invalid")),
        (timestamp(1, 5, 1), json!(0)),
        (timestamp(2, 1, 1), json!(999)),
        (DateTime::<Utc>::MAX_UTC.timestamp(), json!(999)),
    ]));
    with_year_projection_test_context(3, 3 * BAR_WORK_BYTES, fixed_now, || {
        let report = statistics::from_remote(&response, "total", 0, fixed_now);
        assert!(!report.bars_unavailable);
        assert_eq!(report.bars.len(), 3);
        assert_eq!(report.bars[0].label, "-1");
        assert!(!report.bars[0].available);
        assert_eq!(report.bars[1].label, "0");
        assert!(!report.bars[1].available);
        assert_eq!(report.bars[2].label, "1");
        assert!(report.bars[2].available);
        assert_eq!(report.bars[2].seconds, 0);
        assert_eq!(year_projection_stats().year_buckets, 2);
    });
}

#[test]
fn other_periods_and_empty_cloud_history_do_not_spend_total_year_credit() {
    let response = remote(read_times(&[(timestamp(2026, 11, 2), json!(60))]));
    with_year_projection_test_context(0, 0, now(), || {
        for (period, count) in [("week", 7), ("month", 30), ("year", 12)] {
            let report = statistics::from_remote(&response, period, 0, now());
            assert_eq!(report.bars.len(), count);
            assert!(!report.bars_unavailable);
            assert_eq!(report.seconds, Some(900));
            assert!(report.bars.last().unwrap().future);
        }
        let empty = statistics::from_remote(&remote(json!({})), "total", 0, now());
        assert!(empty.bars.is_empty());
        assert!(!empty.bars_unavailable);
        assert!(!empty.missing_details);
        assert_eq!(year_projection_stats().admission_checks, 0);
    });
}

#[test]
fn local_total_rejection_keeps_totals_and_minimum_annotation_without_large_bars() {
    let sessions = [
        ReadingSession {
            id: "year-budget-old".into(),
            book_id: "year-budget-book".into(),
            started_at: timestamp(2024, 6, 1),
            ended_at: timestamp(2024, 6, 1) + 60,
            seconds: 60,
        },
        ReadingSession {
            id: "year-budget-last".into(),
            book_id: "year-budget-book".into(),
            started_at: timestamp(2026, 6, 1),
            ended_at: timestamp(2026, 6, 1) + 120,
            seconds: 120,
        },
    ];
    with_year_projection_test_context(3, 3 * BAR_WORK_BYTES, now(), || {
        let report = statistics::summarize("total", 0, now(), &sessions, &[], &[]);
        assert!(!report.bars_unavailable);
        assert_eq!(report.bars.len(), 3);
        assert!(report.bars[1].available);
        assert_eq!(report.bars[1].seconds, 0);
        assert_eq!(report.bars[2].seconds, 120);
    });
    with_year_projection_test_context(2, 3 * BAR_WORK_BYTES, now(), || {
        let report = statistics::summarize("total", 0, now(), &sessions, &[], &[]);
        assert!(report.bars_unavailable && report.bars.is_empty());
        assert_eq!(report.seconds, 180);
        assert_eq!(report.days, 2);
        assert_eq!(report.average_seconds, 90);
        assert_eq!(report.ranking[0].seconds, 180);
        assert_eq!(year_projection_stats().bars_created, 0);
    });
    let annotation = Annotation {
        created_at: DateTime::<Utc>::MIN_UTC.timestamp(),
        ..Default::default()
    };
    with_year_projection_test_context(TOTAL_BARS, TOTAL_WORK_BYTES, now(), || {
        let report = statistics::summarize("total", 0, now(), &[], &[], &[annotation]);
        assert!(report.bars_unavailable && report.bars.is_empty());
        assert_eq!(report.seconds, 0);
        assert_eq!(report.annotations, 1);
        assert_eq!(year_projection_stats().bars_created, 0);
    });
}

#[test]
fn checked_year_arithmetic_and_test_overrides_cannot_raise_the_production_ceiling() {
    with_year_projection_test_context(usize::MAX, usize::MAX, now(), || {
        assert!(!year_span_admitted_for_test(i32::MIN, i32::MAX));
        assert!(!year_span_admitted_for_test(2026, 2025));
        assert!(!year_span_admitted_for_test(2026 - TOTAL_BARS as i32, 2026));
        assert!(year_span_admitted_for_test(
            2026 - TOTAL_BARS as i32 + 1,
            2026
        ));
        assert_eq!(year_projection_stats().bars_created, 0);
        assert_eq!(year_projection_stats().rejections, 3);
    });
}

struct StatisticsUi {
    last_window: Rc<RefCell<Weak<MinimalSoftwareWindow>>>,
}

impl UiPlatform for StatisticsUi {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.last_window.replace(Rc::downgrade(&window));
        Ok(window)
    }
}

fn paint_statistics(ui: &AppWindow, last: &Rc<RefCell<Weak<MinimalSoftwareWindow>>>) {
    let window = last
        .borrow()
        .upgrade()
        .expect("The statistics window exists");
    let size = ui.window().size();
    let width = size.width as usize;
    let mut pixels = vec![slint::Rgb8Pixel::default(); width * size.height as usize];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        let _ = renderer.render(&mut pixels, width);
    });
}

fn run_genuine_statistics_year_budget() -> Result<()> {
    use crate::reader::caret_test_font;
    use slint::fontique_010::fontique;

    let last_window = Rc::new(RefCell::new(Weak::new()));
    slint::platform::set_platform(Box::new(StatisticsUi {
        last_window: last_window.clone(),
    }))?;
    let bytes = Arc::new(caret_test_font::bytes());
    let metrics = TextMetrics::from_font(bytes.as_slice(), 0).map_err(anyhow::Error::msg)?;
    slint::fontique_010::shared_collection().register_fonts(fontique::Blob::new(bytes), None);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    for profile in [DeviceProfile::Paperwhite6, DeviceProfile::Scribe] {
        let directory = tempfile::tempdir()?;
        let account = source::AuthSession {
            api_key: "synthetic-statistics-year-budget-key".into(),
            user_vid: "synthetic-statistics-year-budget-account".into(),
            account_name: "Statistics year budget fixture".into(),
            cookies: BTreeMap::from([
                (
                    "wr_vid".into(),
                    "synthetic-statistics-year-budget-account".into(),
                ),
                (
                    "wr_skey".into(),
                    "synthetic-statistics-year-budget-session".into(),
                ),
            ]),
            ..Default::default()
        };
        SessionFile::new(directory.path()).save(&account)?;
        let ui = AppWindow::new()?;
        ui.global::<Theme>()
            .set_ui_font(caret_test_font::FAMILY.into());
        ui.global::<Theme>()
            .set_serif_font(caret_test_font::FAMILY.into());
        let platform = Arc::new(Platform::new(profile, false)?);
        crate::platform::configure_window(ui.window(), &platform)?;
        let app = App::attach(
            &ui,
            AppOptions {
                data_dir: directory.path().to_path_buf(),
                profile,
                demo: false,
                offline: true,
                initial_screen: "stats".into(),
                fonts: vec![FontRecord {
                    id: "song".into(),
                    label: "Statistics fixture font".into(),
                    family: caret_test_font::FAMILY.into(),
                    metrics: metrics.clone(),
                    weights: BTreeMap::from([(400, metrics.clone())]),
                    available: true,
                }],
            },
            platform,
            &runtime,
        )?;
        ui.show()?;
        let global = ui.global::<AuxiliaryModel>();
        let mut state = app.borrow_mut();
        state.dispatch("auxiliary", "stats.period", "total")?;
        let epoch = state.jobs.account_epoch();
        let request = state.stats_request;
        let normal = remote(read_times(&[
            (timestamp(2024, 6, 1), json!(60)),
            (timestamp(2026, 6, 1), json!(120)),
        ]));
        assert!(state.handle_job(JobEvent::new(
            request,
            epoch,
            JobResult::Statistics(Ok(normal)),
        ))?);
        with_year_projection_test_context(3, 3 * BAR_WORK_BYTES, now(), || -> Result<()> {
            state.render_statistics()?;
            assert_eq!(year_projection_stats().ui_rows, 3);
            Ok(())
        })?;
        assert_eq!(global.get_stats_bars().row_count(), 3);
        let rows = Rc::new(slint::VecModel::from(
            global.get_stats_bars().iter().collect::<Vec<_>>(),
        ));
        let old_rows = Rc::downgrade(&rows);
        global.set_stats_bars(rows.clone().into());
        drop(rows);
        paint_statistics(&ui, &last_window);
        assert!(old_rows.upgrade().is_some());

        let rejected = remote(read_times(&[(
            DateTime::<Utc>::MIN_UTC.timestamp(),
            json!(1),
        )]));
        let original = rejected.raw.clone();
        assert!(state.handle_job(JobEvent::new(
            request,
            epoch,
            JobResult::Statistics(Ok(rejected)),
        ))?);
        with_year_projection_test_context(
            TOTAL_BARS,
            TOTAL_WORK_BYTES,
            now(),
            || -> Result<()> {
                state.render_statistics()?;
                assert_eq!(year_projection_stats().bars_created, 0);
                assert_eq!(year_projection_stats().ui_rows, 0);
                assert_eq!(year_projection_stats().rejections, 1);
                Ok(())
            },
        )?;
        paint_statistics(&ui, &last_window);
        assert_eq!(global.get_stats_bars().row_count(), 0);
        assert!(old_rows.upgrade().is_none());
        assert_eq!(state.stats_selected, None);
        assert_eq!(
            global.get_stats_selected_label().as_str(),
            tr("stats_unavailable"),
        );
        assert_eq!(
            global.get_stats_status().as_str(),
            format!(
                "{} · {} · {}",
                tr("stats_cloud"),
                tr("stats_unavailable"),
                tr("offline")
            ),
        );
        assert_eq!(
            global.get_stats_summary().as_str(),
            optional_duration_label(Some(900)),
        );
        assert_eq!(global.get_stats_metrics().row_count(), 4);
        assert_eq!(
            global
                .get_stats_metrics()
                .row_data(2)
                .unwrap()
                .value
                .as_str(),
            "4 books",
        );
        assert_eq!(state.remote_stats.as_ref().unwrap().raw, original);
        assert_eq!(
            state
                .store
                .setting::<source::ReadStatistics>("stats.remote:total:0")?
                .unwrap()
                .raw,
            original,
        );
        assert_eq!(state.jobs.account_epoch(), epoch);
        assert_eq!(state.stats_request, request);
        assert!(state.jobs.configured());
        assert!(state.force_offline && !state.online);
        assert_eq!(
            state.persisted_session.as_ref().unwrap().user_vid,
            account.user_vid,
        );
        state.terminal_shutdown()?;
        drop(state);
        drop(global);
        let weak_app = Rc::downgrade(&app);
        drop(app);
        assert!(weak_app.upgrade().is_none());
        let weak_ui = ui.as_weak();
        ui.hide()?;
        drop(ui);
        slint::platform::update_timers_and_animations();
        assert!(weak_ui.upgrade().is_none());
    }
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

#[test]
fn genuine_statistics_year_budget_preserves_state_and_releases_rows() -> Result<()> {
    crate::app::underline_projection_tests::isolated(
        "app::auxiliary::statistics_year_budget_tests::genuine_statistics_year_budget_preserves_state_and_releases_rows",
        run_genuine_statistics_year_budget,
    )
}
