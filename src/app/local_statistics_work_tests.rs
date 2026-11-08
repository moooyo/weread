//! Genuine offline App statistics keep their old view on day-work rejection.

use super::underline_projection_tests::isolated;
use super::*;
use crate::statistics::{
    self, LocalStatisticsLimits, LocalStatisticsUnavailable, local_statistics_work_stats,
    with_local_statistics_test_context, with_year_projection_test_context,
};
use chrono::{DateTime, NaiveDate, Utc};
use slint::{
    Model,
    platform::{
        Platform as UiPlatform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};

const BOOK: &str = "local-day-work-book";

fn now() -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(2026, 11, 3)
        .unwrap()
        .and_hms_opt(6, 0, 0)
        .unwrap()
        .and_utc()
}

fn context<T>(limits: LocalStatisticsLimits, run: impl FnOnce() -> T) -> T {
    with_year_projection_test_context(512, 512 * 512, now(), || {
        with_local_statistics_test_context(limits, run)
    })
}

fn session(id: &str, start: i64, end: i64, seconds: u64) -> ReadingSession {
    ReadingSession {
        id: id.into(),
        book_id: BOOK.into(),
        started_at: start,
        ended_at: end,
        seconds,
    }
}

struct StatsUi;
impl UiPlatform for StatsUi {
    fn create_window_adapter(
        &self,
    ) -> std::result::Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

struct Fixture {
    app: Option<Rc<RefCell<App>>>,
    ui: Option<AppWindow>,
    runtime: Option<tokio::runtime::Runtime>,
    directory: tempfile::TempDir,
}

impl Fixture {
    fn new(records: &[ReadingSession]) -> Result<Self> {
        slint::platform::set_platform(Box::new(StatsUi))?;
        let directory = tempfile::tempdir()?;
        SessionFile::new(directory.path()).save(&source::AuthSession {
            api_key: "synthetic-local-day-work-key".into(),
            user_vid: "synthetic-local-day-work-account".into(),
            cookies: BTreeMap::from([
                ("wr_vid".into(), "synthetic-local-day-work-account".into()),
                ("wr_skey".into(), "synthetic-local-day-work-session".into()),
            ]),
            ..Default::default()
        })?;
        {
            let store = Store::open(directory.path().join("library.sqlite"))?;
            let mut preferences = AppPreferences::for_device(false);
            preferences.pull_on_open = false;
            preferences.push_on_close = false;
            preferences.report_time = false;
            preferences.popular = false;
            store.set_setting("preferences", &preferences)?;
            let mut book = Book::new(BOOK, "Retained local statistics Reader");
            book.source = "local".into();
            book.category = "Fixture".into();
            book.total_chapters = 1;
            store.upsert_book(&book)?;
            store.upsert_chapter(&Chapter {
                id: "local-day-chapter".into(),
                book_id: BOOK.into(),
                body: Some("Retained reading body.".into()),
                ..Default::default()
            })?;
            store.save_annotation(&Annotation {
                id: "local-day-note".into(),
                book_id: BOOK.into(),
                start: 0,
                end: 8,
                quote: "Retained".into(),
                note: "Keep local note".into(),
                created_at: now().timestamp() - 60,
                ..Default::default()
            })?;
            store.save_bookmark(&Bookmark {
                id: "local-day-bookmark".into(),
                book_id: BOOK.into(),
                char_offset: 1,
                label: "Keep bookmark".into(),
                ..Default::default()
            })?;
            for record in records {
                store.record_session(record)?;
            }
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let platform = Arc::new(Platform::new(DeviceProfile::Paperwhite6, false)?);
        let ui = AppWindow::new()?;
        crate::platform::configure_window(ui.window(), &platform)?;
        let app = App::attach(
            &ui,
            AppOptions {
                data_dir: directory.path().to_path_buf(),
                profile: DeviceProfile::Paperwhite6,
                demo: false,
                offline: true,
                initial_screen: "library".into(),
                fonts: vec![FontRecord {
                    id: "song".into(),
                    label: "Local statistics fixture font".into(),
                    family: "sans-serif".into(),
                    metrics: TextMetrics::default(),
                    weights: BTreeMap::new(),
                    available: true,
                }],
            },
            platform,
            &runtime,
        )?;
        {
            let mut state = app.borrow_mut();
            state.load_local_reader(BOOK, 0, Some(0))?;
            state.reader.selection = Some((0, 8));
            assert!(state.reader.draft.set_query("Keep local statistics draft"));
            state.stats_period = "total".into();
            state.stats_offset = 0;
            state.remote_stats = None;
            assert!(state.force_offline && !state.online && state.jobs.configured());
        }
        Ok(Self {
            app: Some(app),
            ui: Some(ui),
            runtime: Some(runtime),
            directory,
        })
    }

    fn state(&self) -> std::cell::RefMut<'_, App> {
        self.app.as_ref().unwrap().borrow_mut()
    }
    fn ui(&self) -> &AppWindow {
        self.ui.as_ref().unwrap()
    }
    fn finish(mut self) -> Result<()> {
        let app = self.app.take().unwrap();
        app.borrow_mut().terminal_shutdown()?;
        let weak = Rc::downgrade(&app);
        drop(app);
        assert!(weak.upgrade().is_none());
        let ui = self.ui.take().unwrap();
        let weak = ui.as_weak();
        ui.hide()?;
        drop(ui);
        slint::platform::update_timers_and_animations();
        assert!(weak.upgrade().is_none());
        self.runtime
            .take()
            .unwrap()
            .shutdown_timeout(Duration::from_secs(1));
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(app) = self.app.take() {
            let _ = app.borrow_mut().terminal_shutdown();
        }
        if let Some(ui) = self.ui.take() {
            let _ = ui.hide();
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }
}

#[derive(Debug, PartialEq)]
struct View {
    period: String,
    label: String,
    summary: String,
    selected: String,
    calendar_label: String,
    calendar_offset: i32,
    metrics: Vec<(String, String)>,
    bars: Vec<(String, f32, bool, bool)>,
    ranking: Vec<(String, String, String, f32)>,
    calendar: Vec<i32>,
}

fn view(fixture: &Fixture) -> View {
    let global = fixture.ui().global::<AuxiliaryModel>();
    View {
        period: global.get_stats_period().into(),
        label: global.get_stats_period_label().into(),
        summary: global.get_stats_summary().into(),
        selected: global.get_stats_selected_label().into(),
        calendar_label: global.get_stats_calendar_label().into(),
        calendar_offset: global.get_stats_calendar_offset(),
        metrics: global
            .get_stats_metrics()
            .iter()
            .map(|entry| (entry.label.into(), entry.value.into()))
            .collect(),
        bars: global
            .get_stats_bars()
            .iter()
            .map(|entry| {
                (
                    entry.label.into(),
                    entry.value,
                    entry.selected,
                    entry.future,
                )
            })
            .collect(),
        ranking: global
            .get_stats_ranking()
            .iter()
            .map(|entry| {
                (
                    entry.id.into(),
                    entry.title.into(),
                    entry.subtitle.into(),
                    entry.value,
                )
            })
            .collect(),
        calendar: global.get_stats_calendar().iter().collect(),
    }
}

fn assert_before_maps() {
    let stats = local_statistics_work_stats();
    assert_eq!(stats.aggregate_calls, 0);
    assert_eq!(stats.day_steps, 0);
    assert_eq!(stats.daily_entries, 0);
    assert_eq!(stats.book_entries, 0);
}

fn retained_view() -> Result<()> {
    let fixture = Fixture::new(&[session(
        "normal",
        now().timestamp() - 60,
        now().timestamp(),
        30,
    )])?;
    context(LocalStatisticsLimits::default(), || {
        fixture.state().render_statistics()
    })?;
    let original = view(&fixture);
    let selected = fixture.state().stats_selected;
    let offset = fixture.state().stats_offset;
    let epoch = fixture.state().jobs.account_epoch();
    let request = fixture.state().stats_request;
    let preferences = serde_json::to_value(&fixture.state().prefs)?;
    let account = serde_json::to_value(&fixture.state().persisted_session)?;
    let records = fixture.state().store.sessions(i64::MIN, i64::MAX)?;
    let notes = fixture.state().store.annotations(BOOK)?;
    let bookmarks = fixture.state().store.bookmarks(BOOK)?;
    context(
        LocalStatisticsLimits {
            day_steps: 0,
            ..Default::default()
        },
        || -> Result<()> {
            let error = fixture.state().local_statistics_for_test().unwrap_err();
            assert!(error.downcast_ref::<LocalStatisticsUnavailable>().is_some());
            assert_before_maps();
            fixture.state().render_statistics()?;
            assert_before_maps();
            assert_eq!(view(&fixture), original);
            assert_eq!(
                fixture.ui().global::<AuxiliaryModel>().get_stats_status(),
                tr("api_error")
            );
            assert_eq!(
                fixture.ui().global::<ShellModel>().get_toast(),
                tr("api_error")
            );
            Ok(())
        },
    )?;
    let state = fixture.state();
    assert_eq!(
        (state.stats_selected, state.stats_offset),
        (selected, offset)
    );
    assert_eq!(state.jobs.account_epoch(), epoch);
    assert_eq!(state.stats_request, request);
    assert_eq!(state.reader.selection, Some((0, 8)));
    assert_eq!(state.reader.draft.query, "Keep local statistics draft");
    assert_eq!(serde_json::to_value(&state.prefs)?, preferences);
    assert_eq!(serde_json::to_value(&state.persisted_session)?, account);
    assert_eq!(state.store.sessions(i64::MIN, i64::MAX)?, records);
    assert_eq!(state.store.annotations(BOOK)?, notes);
    assert_eq!(state.store.bookmarks(BOOK)?, bookmarks);
    assert_eq!(
        state.store.chapter_body(BOOK, 0)?,
        Some("Retained reading body.".into())
    );
    drop(state);
    fixture.finish()
}

fn stored_extreme() -> Result<()> {
    let record = session(
        "stored-extreme",
        DateTime::<Utc>::MIN_UTC.timestamp(),
        now().timestamp(),
        1,
    );
    let fixture = Fixture::new(&[record.clone()])?;
    context(
        LocalStatisticsLimits {
            day_steps: 1,
            ..Default::default()
        },
        || -> Result<()> {
            let error = fixture.state().local_statistics_for_test().unwrap_err();
            let cause = error.downcast_ref::<LocalStatisticsUnavailable>().unwrap();
            assert_eq!(cause.kind, statistics::LocalStatisticsLimitKind::DaySteps);
            assert!(cause.requested > 65536);
            assert_before_maps();
            Ok(())
        },
    )?;
    assert_eq!(
        fixture.state().store.sessions(i64::MIN, i64::MAX)?,
        [record]
    );
    fixture.finish()
}

fn cumulative_phases() -> Result<()> {
    let start = DateTime::parse_from_rfc3339("2026-11-02T15:59:30Z")
        .unwrap()
        .timestamp();
    let fixture = Fixture::new(&[session("midnight", start, start + 60, 41)])?;
    fixture.state().stats_period = "week".into();
    context(
        LocalStatisticsLimits {
            day_steps: 4,
            ..Default::default()
        },
        || -> Result<()> {
            let report = fixture.state().local_statistics_for_test()?;
            assert_eq!((report.seconds, report.days), (41, 2));
            assert_eq!(local_statistics_work_stats().phase_day_forecasts, [2, 2]);
            fixture.state().render_statistics()?;
            Ok(())
        },
    )?;
    let original = view(&fixture);
    context(
        LocalStatisticsLimits {
            day_steps: 3,
            ..Default::default()
        },
        || -> Result<()> {
            fixture.state().render_statistics()?;
            assert_before_maps();
            assert_eq!(local_statistics_work_stats().phase_day_forecasts, [2, 2]);
            assert_eq!(view(&fixture), original);
            Ok(())
        },
    )?;
    fixture.finish()
}

fn sql_cause() -> Result<()> {
    let fixture = Fixture::new(&[session(
        "normal",
        now().timestamp() - 60,
        now().timestamp(),
        30,
    )])?;
    context(LocalStatisticsLimits::default(), || {
        fixture.state().render_statistics()
    })?;
    let original = view(&fixture);
    let status = fixture.ui().global::<AuxiliaryModel>().get_stats_status();
    let other = rusqlite::Connection::open(fixture.directory.path().join("library.sqlite"))?;
    other.execute_batch("ALTER TABLE reading_sessions RENAME TO retained_reading_sessions")?;
    let result = context(LocalStatisticsLimits::default(), || {
        fixture.state().render_statistics()
    });
    other.execute_batch("ALTER TABLE retained_reading_sessions RENAME TO reading_sessions")?;
    let error = result.unwrap_err();
    assert!(error.downcast_ref::<LocalStatisticsUnavailable>().is_none());
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
    assert_eq!(view(&fixture), original);
    assert_eq!(
        fixture.ui().global::<AuxiliaryModel>().get_stats_status(),
        status
    );
    fixture.finish()
}

#[test]
fn genuine_local_day_rejection_keeps_the_previous_statistics_and_reader_state() -> Result<()> {
    isolated(
        "app::local_statistics_work_tests::genuine_local_day_rejection_keeps_the_previous_statistics_and_reader_state",
        retained_view,
    )
}

#[test]
fn genuine_stored_long_interval_is_rejected_before_first_day_and_map() -> Result<()> {
    isolated(
        "app::local_statistics_work_tests::genuine_stored_long_interval_is_rejected_before_first_day_and_map",
        stored_extreme,
    )
}

#[test]
fn genuine_main_and_calendar_share_one_exact_operation_budget() -> Result<()> {
    isolated(
        "app::local_statistics_work_tests::genuine_main_and_calendar_share_one_exact_operation_budget",
        cumulative_phases,
    )
}

#[test]
fn genuine_local_statistics_sql_failure_is_not_reclassified_as_capacity() -> Result<()> {
    isolated(
        "app::local_statistics_work_tests::genuine_local_statistics_sql_failure_is_not_reclassified_as_capacity",
        sql_cause,
    )
}
