use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Datelike, Duration, FixedOffset, Months, NaiveDate, Utc};

use crate::domain::{Annotation, Book, BookStatisticsMetadata, ReadingSession};
use crate::source::ReadStatistics;
use serde_json::Value;

#[cfg(test)]
#[path = "statistics_remote_projection_tests.rs"]
mod remote_projection_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalStatisticsLimitKind {
    DaySteps,
    ScratchBytes,
    IdentifierWorkBytes,
    DateBoundary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("local statistics {kind:?} is unavailable (limit {limit}, requested {requested})")]
pub struct LocalStatisticsUnavailable {
    pub kind: LocalStatisticsLimitKind,
    pub limit: usize,
    pub requested: usize,
}

/// Logical date visits and temporary collection forecasts. These project
/// policies do not bound total process RSS or an absolute wall-clock interval.
#[derive(Clone, Copy, Debug)]
pub struct LocalStatisticsLimits {
    pub day_steps: usize,
    pub scratch_bytes: usize,
    pub identifier_work_bytes: usize,
}

impl Default for LocalStatisticsLimits {
    fn default() -> Self {
        Self {
            day_steps: 65536,
            scratch_bytes: 64 * 1024 * 1024,
            identifier_work_bytes: 64 * 1024 * 1024,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LocalStatisticsWorkStats {
    pub admission_checks: usize,
    pub raw_sessions: usize,
    pub unique_sessions: usize,
    pub day_forecast: usize,
    pub scratch_forecast: usize,
    pub phase_day_forecasts: [usize; 2],
    pub admitted_phases: usize,
    pub aggregate_calls: usize,
    pub day_steps: usize,
    pub daily_entries: usize,
    pub book_entries: usize,
}

#[cfg(test)]
thread_local! {
    static LOCAL_WORK: std::cell::RefCell<Option<(LocalStatisticsLimits, LocalStatisticsWorkStats)>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn local_statistics_work_stats() -> LocalStatisticsWorkStats {
    LOCAL_WORK.with(|state| {
        state
            .borrow()
            .as_ref()
            .map_or(LocalStatisticsWorkStats::default(), |(_, stats)| *stats)
    })
}

#[cfg(test)]
pub(crate) fn with_local_statistics_test_context<T>(
    limits: LocalStatisticsLimits,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<(LocalStatisticsLimits, LocalStatisticsWorkStats)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            LOCAL_WORK.with(|state| *state.borrow_mut() = self.0.take());
        }
    }
    let previous =
        LOCAL_WORK.with(|state| state.replace(Some((limits, LocalStatisticsWorkStats::default()))));
    let _restore = Restore(previous);
    run()
}

#[cfg(test)]
fn observe_local_work(update: impl FnOnce(&mut LocalStatisticsWorkStats)) {
    LOCAL_WORK.with(|state| {
        if let Some((_, stats)) = state.borrow_mut().as_mut() {
            update(stats);
        }
    });
}

fn local_statistics_limits() -> LocalStatisticsLimits {
    let maximum = LocalStatisticsLimits::default();
    #[cfg(test)]
    if let Some(limits) =
        LOCAL_WORK.with(|state| state.borrow().as_ref().map(|(limits, _)| *limits))
    {
        return LocalStatisticsLimits {
            day_steps: limits.day_steps.min(maximum.day_steps),
            scratch_bytes: limits.scratch_bytes.min(maximum.scratch_bytes),
            identifier_work_bytes: limits
                .identifier_work_bytes
                .min(maximum.identifier_work_bytes),
        };
    }
    maximum
}

fn local_limit(
    kind: LocalStatisticsLimitKind,
    requested: usize,
    limit: usize,
) -> Result<(), LocalStatisticsUnavailable> {
    if requested > limit {
        Err(LocalStatisticsUnavailable {
            kind,
            limit,
            requested,
        })
    } else {
        Ok(())
    }
}

fn local_add(
    left: usize,
    right: usize,
    kind: LocalStatisticsLimitKind,
    limit: usize,
) -> Result<usize, LocalStatisticsUnavailable> {
    left.checked_add(right).ok_or(LocalStatisticsUnavailable {
        kind,
        limit,
        requested: usize::MAX,
    })
}

fn local_mul(
    left: usize,
    right: usize,
    kind: LocalStatisticsLimitKind,
    limit: usize,
) -> Result<usize, LocalStatisticsUnavailable> {
    left.checked_mul(right).ok_or(LocalStatisticsUnavailable {
        kind,
        limit,
        requested: usize::MAX,
    })
}

fn local_date_error() -> LocalStatisticsUnavailable {
    LocalStatisticsUnavailable {
        kind: LocalStatisticsLimitKind::DateBoundary,
        limit: 0,
        requested: 0,
    }
}

struct LocalSessions<'a> {
    sessions: Vec<&'a ReadingSession>,
    scratch: usize,
}

fn prepare_local_sessions(
    sessions: &[ReadingSession],
    limits: LocalStatisticsLimits,
) -> Result<LocalSessions<'_>, LocalStatisticsUnavailable> {
    #[cfg(test)]
    observe_local_work(|stats| {
        stats.admission_checks += 1;
        stats.raw_sessions = sessions.len();
    });
    let scratch = if sessions.is_empty() {
        0
    } else {
        local_add(
            local_mul(
                sessions.len(),
                64,
                LocalStatisticsLimitKind::ScratchBytes,
                limits.scratch_bytes,
            )?,
            512,
            LocalStatisticsLimitKind::ScratchBytes,
            limits.scratch_bytes,
        )?
    };
    local_limit(
        LocalStatisticsLimitKind::ScratchBytes,
        scratch,
        limits.scratch_bytes,
    )?;
    let mut identifier_work = 0;
    for session in sessions {
        identifier_work = local_add(
            identifier_work,
            local_add(
                session.id.len(),
                session.book_id.len(),
                LocalStatisticsLimitKind::IdentifierWorkBytes,
                limits.identifier_work_bytes,
            )?,
            LocalStatisticsLimitKind::IdentifierWorkBytes,
            limits.identifier_work_bytes,
        )?;
    }
    identifier_work = local_mul(
        identifier_work,
        3,
        LocalStatisticsLimitKind::IdentifierWorkBytes,
        limits.identifier_work_bytes,
    )?;
    local_limit(
        LocalStatisticsLimitKind::IdentifierWorkBytes,
        identifier_work,
        limits.identifier_work_bytes,
    )?;
    let mut seen = HashSet::with_capacity(sessions.len());
    let mut unique = Vec::with_capacity(sessions.len());
    for session in sessions {
        // Match aggregate's original ordering: the first nonempty ID is seen
        // before elapsed/active validation, so an invalid first record shadows
        // any later duplicate. Empty IDs remain independent records.
        if !session.id.is_empty() && !seen.insert(session.id.as_str()) {
            continue;
        }
        unique.push(session);
    }
    #[cfg(test)]
    observe_local_work(|stats| stats.unique_sessions = unique.len());
    Ok(LocalSessions {
        sessions: unique,
        scratch,
    })
}

fn local_session_days(
    session: &ReadingSession,
    start: i64,
    end: i64,
    timezone: FixedOffset,
) -> Result<(usize, usize), LocalStatisticsUnavailable> {
    let Some(elapsed) = session.ended_at.checked_sub(session.started_at) else {
        return Ok((0, 0));
    };
    if elapsed <= 0 || session.seconds == 0 {
        return Ok((0, 0));
    }
    let cursor = session.started_at.max(start);
    let upper = session.ended_at.min(end);
    if cursor >= upper {
        return Ok((0, 0));
    }
    let Some(instant) = DateTime::from_timestamp(cursor, 0) else {
        // The original loop attempts this cursor and breaks without a bucket.
        return Ok((1, 0));
    };
    let date = local_date(instant, timezone).ok_or_else(local_date_error)?;
    if date.succ_opt().is_none() {
        return Ok((1, 0));
    }
    // i64 seconds plus an i32 fixed offset fit i128 without narrowing. The
    // final upper bound is exclusive, including negative Unix timestamps.
    let offset = i128::from(timezone.local_minus_utc());
    let first = (i128::from(cursor) + offset).div_euclid(86400);
    let last = (i128::from(upper) - 1 + offset).div_euclid(86400);
    let days = usize::try_from(
        last.checked_sub(first)
            .and_then(|days| days.checked_add(1))
            .ok_or_else(local_date_error)?,
    )
    .map_err(|_| local_date_error())?;
    let positive = u64::try_from(days)
        .map_err(|_| local_date_error())?
        .min(session.seconds.min(elapsed as u64));
    Ok((
        days,
        usize::try_from(positive).map_err(|_| local_date_error())?,
    ))
}

fn admit_local_ranges(
    prepared: &LocalSessions<'_>,
    ranges: [(i64, i64); 2],
    timezone: FixedOffset,
    limits: LocalStatisticsLimits,
) -> Result<(), LocalStatisticsUnavailable> {
    let mut days = 0;
    let mut scratch = prepared.scratch;
    for (phase, (start, end)) in ranges.into_iter().enumerate() {
        let mut phase_days = 0;
        let mut positive_phase = false;
        for session in &prepared.sessions {
            let (visits, positive) = local_session_days(session, start, end, timezone)?;
            positive_phase |= positive != 0;
            phase_days = local_add(
                phase_days,
                visits,
                LocalStatisticsLimitKind::DaySteps,
                limits.day_steps,
            )?;
            let day_slots = local_mul(
                positive,
                128,
                LocalStatisticsLimitKind::ScratchBytes,
                limits.scratch_bytes,
            )?;
            let book = if positive == 0 {
                0
            } else {
                local_add(
                    128,
                    local_mul(
                        session.book_id.len(),
                        2,
                        LocalStatisticsLimitKind::ScratchBytes,
                        limits.scratch_bytes,
                    )?,
                    LocalStatisticsLimitKind::ScratchBytes,
                    limits.scratch_bytes,
                )?
            };
            scratch = local_add(
                scratch,
                local_add(
                    day_slots,
                    book,
                    LocalStatisticsLimitKind::ScratchBytes,
                    limits.scratch_bytes,
                )?,
                LocalStatisticsLimitKind::ScratchBytes,
                limits.scratch_bytes,
            )?;
        }
        if positive_phase {
            // BTreeMap and HashMap minimum buckets may allocate several slots
            // at their first entry; pay their fixed spare space per phase.
            scratch = local_add(
                scratch,
                512,
                LocalStatisticsLimitKind::ScratchBytes,
                limits.scratch_bytes,
            )?;
        }
        days = local_add(
            days,
            phase_days,
            LocalStatisticsLimitKind::DaySteps,
            limits.day_steps,
        )?;
        #[cfg(test)]
        observe_local_work(|stats| {
            stats.phase_day_forecasts[phase] = phase_days;
            stats.day_forecast = days;
            stats.scratch_forecast = scratch;
        });
        #[cfg(not(test))]
        let _ = phase;
    }
    local_limit(LocalStatisticsLimitKind::DaySteps, days, limits.day_steps)?;
    local_limit(
        LocalStatisticsLimitKind::ScratchBytes,
        scratch,
        limits.scratch_bytes,
    )?;
    #[cfg(test)]
    observe_local_work(|stats| stats.admitted_phases = 2);
    Ok(())
}

// This project policy prices derived year buckets, labels, Bar storage, and
// the subsequent StatisticBar copies. It is not a total UI/RSS measurement.
const MAX_TOTAL_YEAR_BARS: usize = 512;
const TOTAL_YEAR_BAR_WORK_BYTES: usize = 512;
const MAX_TOTAL_YEAR_WORK_BYTES: usize = MAX_TOTAL_YEAR_BARS * TOTAL_YEAR_BAR_WORK_BYTES;

#[derive(Clone, Copy)]
struct TotalYearLimits {
    bars: usize,
    work_bytes: usize,
}

impl Default for TotalYearLimits {
    fn default() -> Self {
        Self {
            bars: MAX_TOTAL_YEAR_BARS,
            work_bytes: MAX_TOTAL_YEAR_WORK_BYTES,
        }
    }
}

#[derive(Clone, Copy)]
struct TotalYearSpan {
    first: i32,
    count: usize,
}

impl TotalYearSpan {
    fn admit(first: i32, last: i32) -> Option<Self> {
        #[cfg(test)]
        record_year_projection(|stats| stats.admission_checks += 1);
        let count = i64::from(last)
            .checked_sub(i64::from(first))
            .and_then(|span| span.checked_add(1))
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| *count > 0);
        let limits = total_year_limits();
        let admitted = count.filter(|count| {
            *count <= limits.bars.min(MAX_TOTAL_YEAR_BARS)
                && count
                    .checked_mul(TOTAL_YEAR_BAR_WORK_BYTES)
                    .is_some_and(|bytes| bytes <= limits.work_bytes.min(MAX_TOTAL_YEAR_WORK_BYTES))
                && NaiveDate::from_ymd_opt(first, 1, 1).is_some()
                && NaiveDate::from_ymd_opt(last, 1, 1).is_some()
        });
        match admitted {
            Some(count) => {
                #[cfg(test)]
                record_year_projection(|stats| stats.admitted_bars += count);
                Some(Self { first, count })
            }
            None => {
                #[cfg(test)]
                record_year_projection(|stats| stats.rejections += 1);
                None
            }
        }
    }

    fn year(self, index: usize) -> Option<i32> {
        if index >= self.count {
            return None;
        }
        self.first.checked_add(i32::try_from(index).ok()?)
    }

    fn index(self, year: i32) -> Option<usize> {
        i64::from(year)
            .checked_sub(i64::from(self.first))
            .and_then(|index| usize::try_from(index).ok())
            .filter(|index| *index < self.count)
    }

    fn project(self, mut bar: impl FnMut(i32, NaiveDate) -> Bar) -> Option<Vec<Bar>> {
        // Admission precedes range traversal, Vec allocation, and every label.
        let mut bars = Vec::with_capacity(self.count);
        for index in 0..self.count {
            let year = self.year(index)?;
            let day = NaiveDate::from_ymd_opt(year, 1, 1)?;
            #[cfg(test)]
            record_year_projection(|stats| stats.year_visits += 1);
            bars.push(bar(year, day));
            #[cfg(test)]
            record_year_projection(|stats| stats.bars_created += 1);
        }
        Some(bars)
    }
}

fn total_year_limits() -> TotalYearLimits {
    #[cfg(test)]
    {
        return YEAR_PROJECTION_TEST_CONTEXT.with(|context| context.borrow().limits);
    }
    #[cfg(not(test))]
    TotalYearLimits::default()
}

pub(crate) fn projection_now() -> DateTime<Utc> {
    #[cfg(test)]
    if let Some(now) = YEAR_PROJECTION_TEST_CONTEXT.with(|context| context.borrow().now) {
        return now;
    }
    Utc::now()
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct YearProjectionStats {
    pub admission_checks: usize,
    pub admitted_bars: usize,
    pub year_buckets: usize,
    pub year_visits: usize,
    pub bars_created: usize,
    pub ui_rows: usize,
    pub rejections: usize,
}

#[cfg(test)]
#[derive(Default)]
struct YearProjectionTestContext {
    limits: TotalYearLimits,
    now: Option<DateTime<Utc>>,
    stats: YearProjectionStats,
}

#[cfg(test)]
std::thread_local! {
    static YEAR_PROJECTION_TEST_CONTEXT: std::cell::RefCell<YearProjectionTestContext> =
        std::cell::RefCell::new(YearProjectionTestContext::default());
}

#[cfg(test)]
fn record_year_projection(update: impl FnOnce(&mut YearProjectionStats)) {
    YEAR_PROJECTION_TEST_CONTEXT.with(|context| update(&mut context.borrow_mut().stats));
}

#[cfg(test)]
pub(crate) fn record_statistics_ui_rows(rows: usize) {
    record_year_projection(|stats| stats.ui_rows += rows);
}

#[cfg(test)]
pub(crate) fn year_projection_stats() -> YearProjectionStats {
    YEAR_PROJECTION_TEST_CONTEXT.with(|context| context.borrow().stats)
}

#[cfg(test)]
pub(crate) fn year_span_admitted_for_test(first: i32, last: i32) -> bool {
    TotalYearSpan::admit(first, last).is_some()
}

#[cfg(test)]
pub(crate) fn with_year_projection_test_context<T>(
    bars: usize,
    work_bytes: usize,
    now: DateTime<Utc>,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(YearProjectionTestContext);
    impl Drop for Restore {
        fn drop(&mut self) {
            YEAR_PROJECTION_TEST_CONTEXT.with(|context| {
                std::mem::swap(&mut *context.borrow_mut(), &mut self.0);
            });
        }
    }
    let previous = YEAR_PROJECTION_TEST_CONTEXT.with(|context| {
        context.replace(YearProjectionTestContext {
            limits: TotalYearLimits { bars, work_bytes },
            now: Some(now),
            stats: YearProjectionStats::default(),
        })
    });
    let _restore = Restore(previous);
    run()
}

/// A report derived exclusively from local reading sessions and annotations.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub period_label: String,
    pub seconds: u64,
    pub days: u32,
    pub average_seconds: u64,
    /// Currently finished books read in this range; no completion timestamp is stored.
    pub finished: u32,
    pub annotations: u32,
    pub bars: Vec<Bar>,
    /// The complete annual projection exceeded its derived-output allowance.
    /// No prefix of that projection is presented as a complete history.
    pub bars_unavailable: bool,
    pub ranking: Vec<Rank>,
    /// One intensity (0..=3) per actual day of the selected calendar month.
    pub calendar: Vec<i32>,
    /// Monday-based offset of the first calendar day.
    pub calendar_offset: usize,
    pub calendar_label: String,
    pub preferences: Vec<Preference>,
    pub local_only: bool,
    pub source: &'static str,
    pub range_start: i64,
    pub range_end: i64,
    pub offset: i32,
    /// Local day/scratch admission failed. Numeric defaults and range fields
    /// are not a newly available report when this explicit state is present.
    pub local_unavailable: Option<LocalStatisticsUnavailable>,
}

impl Report {
    fn unavailable(error: LocalStatisticsUnavailable) -> Self {
        Self {
            local_unavailable: Some(error),
            bars_unavailable: true,
            local_only: true,
            source: "local_sessions",
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Bar {
    pub label: String,
    pub seconds: u64,
    pub future: bool,
    /// False means the source did not provide a valid bucket, rather than zero reading.
    pub available: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Rank {
    pub book_id: String,
    pub title: String,
    pub seconds: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Preference {
    pub category: String,
    pub seconds: u64,
    pub percent: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Period {
    Week,
    Month,
    Year,
    Total,
}

/// Cloud fields retain presence information lost by default-valued transport fields.
#[derive(Clone, Debug, Default)]
pub struct CloudReport {
    pub period_label: String,
    pub seconds: Option<u64>,
    pub days: Option<u32>,
    pub average_seconds: Option<u64>,
    pub summary: Vec<SummaryValue>,
    pub bars: Vec<Bar>,
    /// Raw cloud totals remain present when the annual chart is unavailable.
    pub bars_unavailable: bool,
    pub ranking: Vec<CloudRank>,
    /// Scale visible ranks against all valid raw ranks without copying them.
    pub ranking_max_seconds: u64,
    pub projection_unavailable: Option<RemoteProjectionUnavailable>,
    pub calendar: Vec<i32>,
    pub calendar_offset: usize,
    pub calendar_label: String,
    pub preferences: Vec<Preference>,
    pub range_start: i64,
    pub missing_details: bool,
    pub offset: i32,
}

#[derive(Clone, Debug, Default)]
pub struct SummaryValue {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, Default)]
pub struct CloudRank {
    pub book_id: String,
    pub title: String,
    pub author: String,
    pub seconds: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct RemoteProjectionSelection {
    pub summary: usize,
    pub ranking: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct RemoteProjectionLimits {
    pub records: usize,
    pub work_bytes: usize,
    pub metadata_bytes: usize,
}

impl Default for RemoteProjectionLimits {
    fn default() -> Self {
        Self {
            records: 8192,
            work_bytes: 64 * 1024 * 1024,
            metadata_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("remote statistics {resource} is unavailable (limit {limit}, requested {requested})")]
pub struct RemoteProjectionUnavailable {
    pub resource: &'static str,
    pub limit: usize,
    pub requested: usize,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RemoteProjectionStats {
    pub raw_visits: usize,
    pub summaries: usize,
    pub ranks: usize,
    pub categories: usize,
    pub text_copies: usize,
    pub text_bytes: usize,
    pub work_bytes: usize,
    pub metadata_bytes: usize,
}

#[cfg(test)]
thread_local! {
    static REMOTE_PROJECTION: std::cell::Cell<Option<(RemoteProjectionLimits, RemoteProjectionStats)>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_remote_projection_limits<T>(
    limits: RemoteProjectionLimits,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<(RemoteProjectionLimits, RemoteProjectionStats)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            REMOTE_PROJECTION.with(|value| value.set(self.0));
        }
    }
    let _restore = Restore(
        REMOTE_PROJECTION
            .with(|value| value.replace(Some((limits, RemoteProjectionStats::default())))),
    );
    run()
}

#[cfg(test)]
pub(crate) fn remote_projection_stats() -> RemoteProjectionStats {
    REMOTE_PROJECTION.with(|value| {
        value
            .get()
            .map_or(RemoteProjectionStats::default(), |(_, stats)| stats)
    })
}

#[cfg(test)]
fn observe_remote_projection(update: impl FnOnce(&mut RemoteProjectionStats)) {
    REMOTE_PROJECTION.with(|value| {
        if let Some((limits, mut stats)) = value.get() {
            update(&mut stats);
            value.set(Some((limits, stats)));
        }
    });
}

#[derive(Clone, Copy)]
enum RemoteText<'a> {
    Text(&'a str),
    Number(&'a serde_json::Number),
}

impl RemoteText<'_> {
    fn bytes(self) -> Result<usize, RemoteProjectionUnavailable> {
        match self {
            Self::Text(value) => Ok(value.len()),
            Self::Number(value) => {
                struct Count(usize);
                impl std::io::Write for Count {
                    fn write(&mut self, value: &[u8]) -> std::io::Result<usize> {
                        self.0 = self.0.checked_add(value.len()).ok_or_else(|| {
                            std::io::Error::other("numeric field length overflow")
                        })?;
                        Ok(value.len())
                    }
                    fn flush(&mut self) -> std::io::Result<()> {
                        Ok(())
                    }
                }
                let mut count = Count(0);
                serde_json::to_writer(&mut count, value).map_err(|_| {
                    RemoteProjectionUnavailable {
                        resource: "numeric metadata bytes",
                        limit: 64 * 1024 * 1024,
                        requested: usize::MAX,
                    }
                })?;
                Ok(count.0)
            }
        }
    }
    fn empty(self) -> bool {
        matches!(self, Self::Text(""))
    }
    fn own(self) -> String {
        let text = match self {
            Self::Text(value) => value.to_owned(),
            Self::Number(value) => value.to_string(),
        };
        #[cfg(test)]
        observe_remote_projection(|stats| {
            stats.text_copies += 1;
            stats.text_bytes += text.len();
        });
        text
    }
}

fn remote_text(value: Option<&Value>) -> Option<RemoteText<'_>> {
    match value? {
        Value::String(value) => Some(RemoteText::Text(value)),
        Value::Number(value) => Some(RemoteText::Number(value)),
        _ => None,
    }
}

fn remote_summary(item: &Value) -> Option<(RemoteText<'_>, RemoteText<'_>)> {
    let label = remote_text(item.get("stat"))?;
    let value = remote_text(item.get("counts"))?;
    (!label.empty() && !value.empty()).then_some((label, value))
}

fn remote_rank(
    item: &Value,
) -> Option<(
    Option<RemoteText<'_>>,
    RemoteText<'_>,
    Option<RemoteText<'_>>,
    u64,
)> {
    let book = item.get("book");
    let album = item.get("albumInfo");
    let title = remote_text(book.and_then(|book| book.get("title")))
        .filter(|title| !title.empty())
        .or_else(|| {
            remote_text(album.and_then(|album| album.get("name"))).filter(|title| !title.empty())
        })?;
    Some((
        remote_text(book.and_then(|book| book.get("bookId"))),
        title,
        remote_text(book.and_then(|book| book.get("author")))
            .or_else(|| remote_text(album.and_then(|album| album.get("authorName")))),
        json_seconds(item.get("readTime"))?,
    ))
}

fn remote_rank_has_title(item: &Value) -> bool {
    remote_text(item.get("book").and_then(|book| book.get("title")))
        .filter(|title| !title.empty())
        .or_else(|| {
            remote_text(item.get("albumInfo").and_then(|album| album.get("name")))
                .filter(|title| !title.empty())
        })
        .is_some()
}

fn remote_category(item: &Value) -> Option<(RemoteText<'_>, u64)> {
    Some((
        remote_text(item.get("categoryTitle")).filter(|value| !value.empty())?,
        json_seconds(item.get("readingTime"))?,
    ))
}

#[derive(Default)]
struct RemoteDetails {
    summary: Vec<SummaryValue>,
    ranking: Vec<CloudRank>,
    maximum: u64,
    preferences: Vec<Preference>,
    unavailable: Option<RemoteProjectionUnavailable>,
}

struct RemoteBudget {
    limits: RemoteProjectionLimits,
    work: usize,
    metadata: usize,
    records: usize,
}

impl RemoteBudget {
    fn new() -> Self {
        let maximum = RemoteProjectionLimits::default();
        #[cfg(test)]
        let limits =
            REMOTE_PROJECTION.with(|value| value.get().map_or(maximum, |(limits, _)| limits));
        #[cfg(not(test))]
        let limits = maximum;
        Self {
            limits: RemoteProjectionLimits {
                records: limits.records.min(maximum.records),
                work_bytes: limits.work_bytes.min(maximum.work_bytes),
                metadata_bytes: limits.metadata_bytes.min(maximum.metadata_bytes),
            },
            work: 0,
            metadata: 0,
            records: 0,
        }
    }

    fn add(
        used: usize,
        bytes: usize,
        resource: &'static str,
        limit: usize,
    ) -> Result<usize, RemoteProjectionUnavailable> {
        let requested = used.checked_add(bytes).ok_or(RemoteProjectionUnavailable {
            resource,
            limit,
            requested: usize::MAX,
        })?;
        if requested > limit {
            return Err(RemoteProjectionUnavailable {
                resource,
                limit,
                requested,
            });
        }
        Ok(requested)
    }

    fn visit(&mut self) -> Result<(), RemoteProjectionUnavailable> {
        self.work = Self::add(self.work, 32, "work bytes", self.limits.work_bytes)?;
        #[cfg(test)]
        observe_remote_projection(|stats| {
            stats.raw_visits += 1;
            stats.work_bytes = self.work;
        });
        Ok(())
    }

    fn numeric(&mut self, value: Option<&Value>) -> Result<(), RemoteProjectionUnavailable> {
        let bytes = value.and_then(Value::as_str).map_or(0, str::len);
        self.work = Self::add(
            self.work,
            bytes.checked_mul(3).ok_or(RemoteProjectionUnavailable {
                resource: "work bytes",
                limit: self.limits.work_bytes,
                requested: usize::MAX,
            })?,
            "work bytes",
            self.limits.work_bytes,
        )?;
        #[cfg(test)]
        observe_remote_projection(|stats| stats.work_bytes = self.work);
        Ok(())
    }

    fn output(
        &mut self,
        fields: &[Option<RemoteText<'_>>],
        record_bytes: usize,
        copies: usize,
    ) -> Result<(), RemoteProjectionUnavailable> {
        self.records = Self::add(self.records, 1, "output records", self.limits.records)?;
        let mut bytes = record_bytes;
        for field in fields.iter().flatten() {
            bytes = Self::add(
                bytes,
                field.bytes()?,
                "metadata bytes",
                self.limits.metadata_bytes,
            )?;
        }
        let bytes = bytes
            .checked_mul(copies)
            .ok_or(RemoteProjectionUnavailable {
                resource: "metadata bytes",
                limit: self.limits.metadata_bytes,
                requested: usize::MAX,
            })?;
        self.metadata = Self::add(
            self.metadata,
            bytes,
            "metadata bytes",
            self.limits.metadata_bytes,
        )?;
        self.work = Self::add(self.work, bytes, "work bytes", self.limits.work_bytes)?;
        #[cfg(test)]
        observe_remote_projection(|stats| {
            stats.work_bytes = self.work;
            stats.metadata_bytes = self.metadata;
        });
        Ok(())
    }
}

fn remote_details(
    remote: &ReadStatistics,
    selection: RemoteProjectionSelection,
) -> Result<RemoteDetails, RemoteProjectionUnavailable> {
    let mut budget = RemoteBudget::new();
    let summaries = remote
        .raw
        .get("readStat")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let ranks = remote
        .raw
        .get("readLongest")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let categories = remote
        .raw
        .get("preferCategory")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut summary_count = 0usize;
    for item in summaries {
        budget.visit()?;
        if let Some((label, value)) = remote_summary(item) {
            if summary_count < selection.summary {
                budget.output(
                    &[Some(label), Some(value)],
                    std::mem::size_of::<SummaryValue>(),
                    2,
                )?;
            }
            summary_count += 1;
        }
    }
    let mut rank_count = 0usize;
    let mut maximum = 0u64;
    for item in ranks {
        budget.visit()?;
        if !remote_rank_has_title(item) {
            continue;
        }
        budget.numeric(item.get("readTime"))?;
        if let Some((id, title, author, seconds)) = remote_rank(item) {
            maximum = maximum.max(seconds);
            // Selected ranks can also create one missing SQL book before their
            // UI row. Admit those bounded inherited-field copies up front.
            if rank_count < selection.ranking {
                budget.output(
                    &[id, Some(title), author],
                    std::mem::size_of::<CloudRank>(),
                    4,
                )?;
            }
            rank_count += 1;
        }
    }
    let mut total = 0u128;
    let mut category_count = 0usize;
    for item in categories {
        budget.visit()?;
        if remote_text(item.get("categoryTitle"))
            .filter(|value| !value.empty())
            .is_none()
        {
            continue;
        }
        budget.numeric(item.get("readingTime"))?;
        if let Some((_, seconds)) = remote_category(item) {
            total += u128::from(seconds);
            category_count += 1;
        }
    }
    if total != 0 {
        for item in categories {
            if let Some((category, _)) = remote_category(item) {
                budget.output(&[Some(category)], std::mem::size_of::<Preference>(), 2)?;
            }
        }
    }
    // No owned output exists until the entire selected and complete-category
    // forecast has succeeded. The original raw response is always borrowed.
    let mut summary = Vec::with_capacity(summary_count.min(selection.summary));
    for (label, value) in summaries
        .iter()
        .filter_map(remote_summary)
        .take(selection.summary)
    {
        summary.push(SummaryValue {
            label: label.own(),
            value: value.own(),
        });
    }
    let mut ranking = Vec::with_capacity(rank_count.min(selection.ranking));
    for (id, title, author, seconds) in ranks.iter().filter_map(remote_rank).take(selection.ranking)
    {
        ranking.push(CloudRank {
            book_id: id.map(RemoteText::own).unwrap_or_default(),
            title: title.own(),
            author: author.map(RemoteText::own).unwrap_or_default(),
            seconds,
        });
    }
    let mut preferences = Vec::with_capacity(if total == 0 { 0 } else { category_count });
    if total != 0 {
        for (category, seconds) in categories.iter().filter_map(remote_category) {
            preferences.push(Preference {
                category: category.own(),
                seconds,
                percent: (u128::from(seconds) * 100 / total) as u32,
            });
        }
    }
    #[cfg(test)]
    observe_remote_projection(|stats| {
        stats.summaries = summary.len();
        stats.ranks = ranking.len();
        stats.categories = preferences.len();
    });
    Ok(RemoteDetails {
        summary,
        ranking,
        maximum,
        preferences,
        unavailable: None,
    })
}

fn parse_period(period: &str) -> Period {
    match period {
        "month" => Period::Month,
        "year" => Period::Year,
        "total" | "all" => Period::Total,
        _ => Period::Week,
    }
}

/// API baseTime is Unix seconds at the natural UTC+8 period boundary.
/// Overall statistics have no bounded baseTime.
pub fn period_base_time(period: &str, offset: i32, now: DateTime<Utc>) -> Option<i64> {
    let period = parse_period(period);
    if period == Period::Total {
        return None;
    }
    let timezone = FixedOffset::east_opt(8 * 3600).expect("valid Shanghai offset");
    let today = now.with_timezone(&timezone).date_naive();
    Some(midnight(
        period_range(period, offset.min(0), today, today).0,
        timezone,
    ))
}

/// Contract reviewed against weread.koplugin at revision
/// 2943080c2493a1ae262cc97c74908ed62928c935, weread/lib/read_stats.lua.
/// readTimes keys and values are Unix seconds and reading seconds respectively.
/// Missing buckets remain unavailable. API totals and averages are never recomputed
/// from sparse buckets; annual/overall buckets cannot produce a daily calendar.
pub fn from_remote(
    remote: &ReadStatistics,
    period: &str,
    offset: i32,
    now: DateTime<Utc>,
) -> CloudReport {
    let selection = RemoteProjectionSelection {
        summary: usize::MAX,
        ranking: usize::MAX,
    };
    let details = remote_details(remote, selection).unwrap_or_else(|error| RemoteDetails {
        unavailable: Some(error),
        ..Default::default()
    });
    from_remote_projected(remote, period, offset, now, details)
}

/// Select the actual UI working set before any cloud field is owned. Annual
/// chart availability retains its separate existing policy in the report.
pub fn from_remote_selected_checked(
    remote: &ReadStatistics,
    period: &str,
    offset: i32,
    now: DateTime<Utc>,
    selection: RemoteProjectionSelection,
) -> Result<CloudReport, RemoteProjectionUnavailable> {
    let details = remote_details(remote, selection)?;
    Ok(from_remote_projected(remote, period, offset, now, details))
}

fn from_remote_projected(
    remote: &ReadStatistics,
    period: &str,
    offset: i32,
    now: DateTime<Utc>,
    details: RemoteDetails,
) -> CloudReport {
    let period = parse_period(period);
    let timezone = FixedOffset::east_opt(8 * 3600).expect("valid Shanghai offset");
    let today = now.with_timezone(&timezone).date_naive();
    let requested = period_range(period, offset.min(0), today, today);
    let canonical = json_timestamp(remote.raw.get("baseTime"))
        .or_else(|| (remote.base_time > 0).then_some(remote.base_time))
        .and_then(|timestamp| DateTime::from_timestamp(timestamp, 0))
        .filter(|timestamp| *timestamp <= now)
        .and_then(|timestamp| local_date(timestamp, timezone));
    let (start, end) = if period == Period::Total {
        requested
    } else {
        canonical
            .map(|day| period_range(period, 0, day, day))
            .unwrap_or(requested)
    };
    let read_times = remote.raw.get("readTimes").and_then(Value::as_object);
    // A sparse borrowed input can imply many absent years. Admit its complete
    // inclusive output span before retaining year buckets or constructing bars.
    // Invalid values still identify an unavailable bucket, as before.
    let total_first_year = if period == Period::Total {
        read_times.and_then(|values| {
            values
                .keys()
                .filter_map(|timestamp| read_times_date(timestamp, timezone))
                .filter(|day| *day <= today)
                .map(|day| day.year())
                .min()
        })
    } else {
        None
    };
    let total_span = total_first_year.and_then(|first| TotalYearSpan::admit(first, today.year()));
    let mut bars_unavailable = total_first_year.is_some() && total_span.is_none();
    let mut buckets = BTreeMap::<NaiveDate, Option<u64>>::new();
    if let Some(values) = read_times.filter(|_| !bars_unavailable) {
        for (timestamp, value) in values {
            let Some(day) = read_times_date(timestamp, timezone) else {
                continue;
            };
            if day > today || (period != Period::Total && (day < start || day >= end)) {
                continue;
            }
            let bucket = match period {
                Period::Week | Period::Month => day,
                Period::Year => month_start(day),
                Period::Total => {
                    NaiveDate::from_ymd_opt(day.year(), 1, 1).expect("valid bucket year")
                }
            };
            let seconds = json_seconds(Some(value));
            #[cfg(test)]
            if period == Period::Total && !buckets.contains_key(&bucket) {
                record_year_projection(|stats| stats.year_buckets += 1);
            }
            buckets
                .entry(bucket)
                .and_modify(|previous| {
                    *previous = previous.and_then(|previous| {
                        seconds.map(|seconds| previous.saturating_add(seconds))
                    });
                })
                .or_insert(seconds);
        }
    }
    let bars: Vec<Bar> = match period {
        Period::Week | Period::Month => start
            .iter_days()
            .take_while(|day| *day < end)
            .map(|day| {
                cloud_bar(
                    if period == Period::Week {
                        day.format("%m-%d").to_string()
                    } else {
                        day.day().to_string()
                    },
                    day > today,
                    buckets.get(&day).copied().flatten(),
                )
            })
            .collect(),
        Period::Year => (1..=12)
            .map(|month| {
                let day =
                    NaiveDate::from_ymd_opt(start.year(), month, 1).expect("valid calendar month");
                cloud_bar(
                    month.to_string(),
                    day > today,
                    buckets.get(&day).copied().flatten(),
                )
            })
            .collect(),
        Period::Total => match total_span {
            Some(span) => match span.project(|year, day| {
                cloud_bar(
                    year.to_string(),
                    false,
                    buckets.get(&day).copied().flatten(),
                )
            }) {
                Some(bars) => bars,
                None => {
                    bars_unavailable = true;
                    Vec::new()
                }
            },
            None => Vec::new(),
        },
    };
    let RemoteDetails {
        summary,
        ranking,
        maximum: ranking_max_seconds,
        preferences,
        unavailable: projection_unavailable,
    } = details;
    let mut calendar = Vec::new();
    let calendar_start = month_start(start);
    if matches!(period, Period::Week | Period::Month) && buckets.values().any(Option::is_some) {
        let calendar_end = add_months(calendar_start, 1);
        calendar = calendar_start
            .iter_days()
            .take_while(|day| *day < calendar_end)
            .map(|day| match buckets.get(&day).copied().flatten() {
                None => -1,
                Some(0) => 0,
                Some(1..=2699) => 1,
                Some(2700..=5999) => 2,
                Some(_) => 3,
            })
            .collect();
    }
    let seconds = json_seconds(remote.raw.get("totalReadTime"));
    let days = json_seconds(remote.raw.get("readDays")).and_then(|days| u32::try_from(days).ok());
    let average_seconds = json_seconds(remote.raw.get("dayAverageReadTime"));
    let missing_details = seconds.is_none()
        || days.is_none()
        || average_seconds.is_none()
        || summary.len() < 2
        || projection_unavailable.is_some()
        || bars_unavailable
        || bars.iter().any(|bar: &Bar| !bar.future && !bar.available);
    CloudReport {
        period_label: match period {
            Period::Week => format!(
                "{} – {}",
                start.format("%Y-%m-%d"),
                end.pred_opt().unwrap_or(start).format("%Y-%m-%d")
            ),
            Period::Month => start.format("%Y-%m").to_string(),
            Period::Year => start.format("%Y").to_string(),
            Period::Total => total_first_year
                .map(|first| format!("{} – {}", first, today.year()))
                .unwrap_or_default(),
        },
        seconds,
        days,
        average_seconds,
        summary,
        bars,
        bars_unavailable,
        ranking,
        ranking_max_seconds,
        projection_unavailable,
        calendar,
        calendar_offset: calendar_start.weekday().num_days_from_monday() as usize,
        calendar_label: calendar_start.format("%Y-%m").to_string(),
        preferences,
        range_start: if period == Period::Total {
            0
        } else {
            midnight(start, timezone)
        },
        missing_details,
        offset: if period == Period::Total {
            0
        } else {
            let current = period_range(period, 0, today, today).0;
            let difference = match period {
                Period::Week => (start - current).num_days() / 7,
                Period::Month => {
                    i64::from(start.year() - current.year()) * 12 + i64::from(start.month())
                        - i64::from(current.month())
                }
                Period::Year => i64::from(start.year() - current.year()),
                Period::Total => 0,
            };
            difference.clamp(i64::from(i32::MIN), 0) as i32
        },
    }
}

fn cloud_bar(label: String, future: bool, seconds: Option<u64>) -> Bar {
    Bar {
        label,
        seconds: seconds.unwrap_or(0),
        future,
        available: seconds.is_some() && !future,
    }
}

fn local_date(instant: DateTime<Utc>, timezone: FixedOffset) -> Option<NaiveDate> {
    // A valid UTC instant can overflow Chrono's local date at its upper edge.
    instant
        .naive_utc()
        .checked_add_signed(Duration::seconds(i64::from(timezone.local_minus_utc())))
        .map(|local| local.date())
}

fn read_times_date(timestamp: &str, timezone: FixedOffset) -> Option<NaiveDate> {
    timestamp
        .parse::<i64>()
        .ok()
        .and_then(|timestamp| DateTime::from_timestamp(timestamp, 0))
        .and_then(|instant| local_date(instant, timezone))
}

fn json_timestamp(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value.as_i64().or_else(|| value.as_str()?.parse().ok())
}

fn json_seconds(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

/// Constant-space annotation facts. Callers can stream timestamps to resolve
/// the exact calendar bounds, then stream grouped counts within those bounds.
/// No quote, note, identifier, or timestamp collection is retained here.
pub struct AnnotationStatistics {
    period: Period,
    offset: i32,
    now: DateTime<Utc>,
    earliest: Option<i64>,
    count: u64,
}

trait LocalBookFields {
    fn id(&self) -> &str;
    fn title(&self) -> &str;
    fn category(&self) -> &str;
    fn finished(&self) -> bool;
}

impl LocalBookFields for Book {
    fn id(&self) -> &str {
        &self.id
    }
    fn title(&self) -> &str {
        &self.title
    }
    fn category(&self) -> &str {
        &self.category
    }
    fn finished(&self) -> bool {
        self.finished
    }
}

impl LocalBookFields for BookStatisticsMetadata {
    fn id(&self) -> &str {
        &self.id
    }
    fn title(&self) -> &str {
        &self.title
    }
    fn category(&self) -> &str {
        &self.category
    }
    fn finished(&self) -> bool {
        self.finished
    }
}

impl AnnotationStatistics {
    pub fn new(period: &str, offset: i32, now: DateTime<Utc>) -> Self {
        let period = parse_period(period);
        Self {
            period,
            offset: if period == Period::Total {
                0
            } else {
                offset.min(0)
            },
            now,
            earliest: None,
            count: 0,
        }
    }

    pub fn observe_timestamp(&mut self, timestamp: i64) {
        if timestamp < self.now.timestamp()
            && DateTime::from_timestamp(timestamp, 0).is_some()
            && self.earliest.is_none_or(|earliest| timestamp < earliest)
        {
            self.earliest = Some(timestamp);
        }
    }

    fn bounds_checked(
        &self,
        sessions: &[ReadingSession],
    ) -> Result<(NaiveDate, NaiveDate, i64, i64), LocalStatisticsUnavailable> {
        let timezone = FixedOffset::east_opt(8 * 60 * 60).expect("valid Shanghai offset");
        let today = local_date(self.now, timezone).ok_or_else(local_date_error)?;
        let earliest = sessions
            .iter()
            .filter(|session| {
                session.seconds > 0
                    && session.ended_at > session.started_at
                    && session.started_at < self.now.timestamp()
            })
            .filter_map(|session| DateTime::from_timestamp(session.started_at, 0))
            .chain(
                self.earliest
                    .iter()
                    .filter_map(|timestamp| DateTime::from_timestamp(*timestamp, 0)),
            )
            .filter_map(|instant| local_date(instant, timezone))
            .min()
            .unwrap_or(today);
        let (start, end) = period_range(self.period, self.offset, today, earliest);
        Ok((
            start,
            end,
            midnight(start, timezone),
            midnight(end, timezone).min(self.now.timestamp()),
        ))
    }

    pub fn range(&self, sessions: &[ReadingSession]) -> (i64, i64) {
        self.range_checked(sessions)
            .expect("use range_checked for an unavailable local date boundary")
    }

    pub fn range_checked(
        &self,
        sessions: &[ReadingSession],
    ) -> Result<(i64, i64), LocalStatisticsUnavailable> {
        let (_, _, start, end) = self.bounds_checked(sessions)?;
        Ok((start, end))
    }

    pub fn add_count(&mut self, count: u64) {
        self.count = self.count.saturating_add(count);
    }

    pub fn report(&self, sessions: &[ReadingSession], books: &[Book]) -> Report {
        self.report_checked(sessions, books)
            .unwrap_or_else(Report::unavailable)
    }

    pub fn report_checked(
        &self,
        sessions: &[ReadingSession],
        books: &[Book],
    ) -> Result<Report, LocalStatisticsUnavailable> {
        summarize_annotation_statistics_checked(self, sessions, books)
    }

    pub fn report_metadata_checked(
        &self,
        sessions: &[ReadingSession],
        books: &[BookStatisticsMetadata],
    ) -> Result<Report, LocalStatisticsUnavailable> {
        summarize_annotation_statistics_checked(self, sessions, books)
    }
}

/// All calendar boundaries use Asia/Shanghai (UTC+08:00), independent of the host.
/// Active seconds within a session are apportioned over its wall-clock interval.
/// Cumulative integer allocation preserves seconds when an interval crosses midnight.
pub fn summarize(
    period: &str,
    offset: i32,
    now: DateTime<Utc>,
    sessions: &[ReadingSession],
    books: &[Book],
    annotations: &[Annotation],
) -> Report {
    summarize_checked(period, offset, now, sessions, books, annotations)
        .unwrap_or_else(Report::unavailable)
}

pub fn summarize_checked(
    period: &str,
    offset: i32,
    now: DateTime<Utc>,
    sessions: &[ReadingSession],
    books: &[Book],
    annotations: &[Annotation],
) -> Result<Report, LocalStatisticsUnavailable> {
    let mut facts = AnnotationStatistics::new(period, offset, now);
    for annotation in annotations {
        facts.observe_timestamp(annotation.created_at);
    }
    let (start, end) = facts.range_checked(sessions)?;
    facts.add_count(
        annotations
            .iter()
            .filter(|annotation| annotation.created_at >= start && annotation.created_at < end)
            .count() as u64,
    );
    facts.report_checked(sessions, books)
}

fn summarize_annotation_statistics_checked<B: LocalBookFields>(
    facts: &AnnotationStatistics,
    sessions: &[ReadingSession],
    books: &[B],
) -> Result<Report, LocalStatisticsUnavailable> {
    let period = facts.period;
    let offset = facts.offset;
    let now = facts.now;
    let timezone = FixedOffset::east_opt(8 * 60 * 60).expect("valid Shanghai offset");
    let today = local_date(now, timezone).ok_or_else(local_date_error)?;
    // Resolve the original raw earliest/range contribution before deduplication.
    // A duplicate or an elapsed-overflow record can still establish that range.
    let (start, end, start_timestamp, end_timestamp) = facts.bounds_checked(sessions)?;
    let calendar_date = if matches!(period, Period::Year | Period::Total) {
        end.pred_opt().unwrap_or(start).min(today)
    } else {
        start
    };
    let calendar_start = month_start(calendar_date);
    let calendar_end = add_months(calendar_start, 1);
    let calendar_start_timestamp = midnight(calendar_start, timezone);
    let calendar_end_timestamp = midnight(calendar_end, timezone).min(now.timestamp());
    let limits = local_statistics_limits();
    let prepared = prepare_local_sessions(sessions, limits)?;
    admit_local_ranges(
        &prepared,
        [
            (start_timestamp, end_timestamp),
            (calendar_start_timestamp, calendar_end_timestamp),
        ],
        timezone,
        limits,
    )?;
    let (daily, per_book) = aggregate(&prepared.sessions, start_timestamp, end_timestamp, timezone);

    let seconds = daily
        .values()
        .fold(0_u64, |sum, value| sum.saturating_add(*value));
    let days = daily.len().min(u32::MAX as usize) as u32;
    let elapsed_days = (today.succ_opt().unwrap_or(today).min(end) - start)
        .num_days()
        .max(1) as u64;
    let average_basis = if matches!(period, Period::Week | Period::Month) {
        elapsed_days
    } else {
        u64::from(days.max(1))
    };
    let catalog: HashMap<_, _> = books.iter().map(|book| (book.id(), book)).collect();
    let mut ranking: Vec<_> = per_book
        .iter()
        .map(|(book_id, seconds)| Rank {
            book_id: book_id.clone(),
            title: catalog
                .get(book_id.as_str())
                .map_or_else(|| book_id.clone(), |book| book.title().to_owned()),
            seconds: *seconds,
        })
        .collect();
    ranking.sort_by(|left, right| {
        right
            .seconds
            .cmp(&left.seconds)
            .then_with(|| left.book_id.cmp(&right.book_id))
    });
    let finished = books
        .iter()
        .filter(|book| book.finished() && per_book.contains_key(book.id()))
        .count()
        .min(u32::MAX as usize) as u32;
    let annotations = facts.count.min(u64::from(u32::MAX)) as u32;
    let mut categories = BTreeMap::<String, u64>::new();
    for (book_id, duration) in &per_book {
        let category = catalog
            .get(book_id.as_str())
            .map(|book| book.category())
            .filter(|category| !category.trim().is_empty())
            .unwrap_or("uncategorized");
        let entry = categories.entry(category.to_owned()).or_default();
        *entry = entry.saturating_add(*duration);
    }
    let mut preferences: Vec<_> = categories
        .into_iter()
        .map(|(category, duration)| Preference {
            category,
            seconds: duration,
            percent: if seconds == 0 {
                0
            } else {
                (u128::from(duration) * 100 / u128::from(seconds)) as u32
            },
        })
        .collect();
    preferences.sort_by(|left, right| {
        right
            .seconds
            .cmp(&left.seconds)
            .then_with(|| left.category.cmp(&right.category))
    });

    let (calendar_daily, _) = aggregate(
        &prepared.sessions,
        calendar_start_timestamp,
        calendar_end_timestamp,
        timezone,
    );
    let calendar = calendar_start
        .iter_days()
        .take_while(|day| *day < calendar_end)
        .map(|day| match calendar_daily.get(&day).copied().unwrap_or(0) {
            0 => 0,
            1..=2699 => 1,
            2700..=5999 => 2,
            _ => 3,
        })
        .collect();

    let (bars, bars_unavailable) = make_bars(period, start, end, today, &daily);
    Ok(Report {
        period_label: match period {
            Period::Week => format!(
                "{} – {}",
                start.format("%Y-%m-%d"),
                end.pred_opt().unwrap_or(start).format("%Y-%m-%d")
            ),
            Period::Month => start.format("%Y-%m").to_string(),
            Period::Year => start.format("%Y").to_string(),
            Period::Total => format!(
                "{} – {}",
                start.format("%Y-%m-%d"),
                today.format("%Y-%m-%d")
            ),
        },
        seconds,
        days,
        average_seconds: seconds / average_basis,
        finished,
        annotations,
        bars,
        bars_unavailable,
        ranking,
        calendar,
        calendar_offset: calendar_start.weekday().num_days_from_monday() as usize,
        calendar_label: calendar_start.format("%Y-%m").to_string(),
        preferences,
        local_only: true,
        source: "local_sessions",
        range_start: start_timestamp,
        range_end: end_timestamp,
        offset,
        local_unavailable: None,
    })
}

fn aggregate(
    sessions: &[&ReadingSession],
    start: i64,
    end: i64,
    timezone: FixedOffset,
) -> (BTreeMap<NaiveDate, u64>, HashMap<String, u64>) {
    #[cfg(test)]
    observe_local_work(|stats| stats.aggregate_calls += 1);
    let mut daily = BTreeMap::<NaiveDate, u64>::new();
    let mut per_book = HashMap::<String, u64>::new();
    for session in sessions {
        let Some(elapsed) = session.ended_at.checked_sub(session.started_at) else {
            continue;
        };
        if elapsed <= 0 || session.seconds == 0 {
            continue;
        }
        let active = session.seconds.min(elapsed as u64);
        let mut cursor = session.started_at.max(start);
        let upper = session.ended_at.min(end);
        let mut session_seconds = 0_u64;
        while cursor < upper {
            #[cfg(test)]
            observe_local_work(|stats| stats.day_steps += 1);
            let Some(instant) = DateTime::from_timestamp(cursor, 0) else {
                break;
            };
            let Some(date) = local_date(instant, timezone) else {
                break;
            };
            let Some(next_date) = date.succ_opt() else {
                break;
            };
            let next = midnight(next_date, timezone).min(upper);
            if next <= cursor {
                break;
            }
            let seconds = allocated(active, elapsed, next - session.started_at)
                .saturating_sub(allocated(active, elapsed, cursor - session.started_at));
            if seconds > 0 {
                #[cfg(test)]
                if !daily.contains_key(&date) {
                    observe_local_work(|stats| stats.daily_entries += 1);
                }
                let entry = daily.entry(date).or_default();
                *entry = entry.saturating_add(seconds);
                session_seconds = session_seconds.saturating_add(seconds);
            }
            cursor = next;
        }
        if session_seconds > 0 {
            // Saturating addition is associative. Price/hash/copy a book key
            // once per session instead of once for each admitted day slice.
            #[cfg(test)]
            if !per_book.contains_key(session.book_id.as_str()) {
                observe_local_work(|stats| stats.book_entries += 1);
            }
            let entry = per_book.entry(session.book_id.clone()).or_default();
            *entry = entry.saturating_add(session_seconds);
        }
    }
    (daily, per_book)
}

fn allocated(active: u64, elapsed: i64, offset: i64) -> u64 {
    (u128::from(active) * offset.max(0) as u128 / elapsed as u128) as u64
}

fn midnight(day: NaiveDate, timezone: FixedOffset) -> i64 {
    // Local midnight at Chrono's minimum date can precede MIN_UTC. Its Unix
    // seconds still fit i64, so compute the boundary without an offset panic.
    day.and_hms_opt(0, 0, 0)
        .expect("valid midnight")
        .and_utc()
        .timestamp()
        .checked_sub(i64::from(timezone.local_minus_utc()))
        .expect("a representable calendar date and fixed offset fit Unix seconds")
}

fn month_start(day: NaiveDate) -> NaiveDate {
    day.with_day(1).expect("valid first day")
}

fn add_months(day: NaiveDate, offset: i32) -> NaiveDate {
    if offset >= 0 {
        day.checked_add_months(Months::new(offset as u32))
            .unwrap_or(day)
    } else {
        day.checked_sub_months(Months::new(offset.unsigned_abs()))
            .unwrap_or(day)
    }
}

fn period_range(
    period: Period,
    offset: i32,
    today: NaiveDate,
    earliest: NaiveDate,
) -> (NaiveDate, NaiveDate) {
    match period {
        Period::Week => {
            let monday = today
                .checked_sub_signed(Duration::days(i64::from(
                    today.weekday().num_days_from_monday(),
                )))
                .unwrap_or(today);
            let start = monday
                .checked_add_signed(Duration::days(i64::from(offset) * 7))
                .unwrap_or(monday);
            (
                start,
                start.checked_add_signed(Duration::days(7)).unwrap_or(start),
            )
        }
        Period::Month => {
            let start = add_months(month_start(today), offset);
            (start, add_months(start, 1))
        }
        Period::Year => {
            let year = today.year().checked_add(offset).unwrap_or(today.year());
            let start = NaiveDate::from_ymd_opt(year, 1, 1).unwrap_or_else(|| {
                NaiveDate::from_ymd_opt(today.year(), 1, 1).expect("valid current year")
            });
            (start, add_months(start, 12))
        }
        Period::Total => (earliest.min(today), today.succ_opt().unwrap_or(today)),
    }
}

fn make_bars(
    period: Period,
    start: NaiveDate,
    end: NaiveDate,
    today: NaiveDate,
    daily: &BTreeMap<NaiveDate, u64>,
) -> (Vec<Bar>, bool) {
    if matches!(period, Period::Week | Period::Month) {
        return (
            start
                .iter_days()
                .take_while(|day| *day < end)
                .map(|day| Bar {
                    label: if period == Period::Week {
                        day.format("%m-%d").to_string()
                    } else {
                        day.day().to_string()
                    },
                    seconds: daily.get(&day).copied().unwrap_or(0),
                    future: day > today,
                    available: true,
                })
                .collect(),
            false,
        );
    }
    if period == Period::Year {
        return (
            (1..=12)
                .map(|month| Bar {
                    label: month.to_string(),
                    seconds: daily
                        .iter()
                        .filter(|(day, _)| day.month() == month)
                        .fold(0_u64, |sum, (_, duration)| sum.saturating_add(*duration)),
                    future: start.year() > today.year()
                        || (start.year() == today.year() && month > today.month()),
                    available: true,
                })
                .collect(),
            false,
        );
    }
    let Some(span) = TotalYearSpan::admit(start.year(), today.year()) else {
        return (Vec::new(), true);
    };
    // Sum each admitted year once; do not rescan all local days for every bar.
    let mut yearly = BTreeMap::<i32, u64>::new();
    for (day, seconds) in daily {
        if span.index(day.year()).is_some() {
            let total = yearly.entry(day.year()).or_default();
            *total = total.saturating_add(*seconds);
        }
    }
    match span.project(|year, _| Bar {
        label: year.to_string(),
        seconds: yearly.get(&year).copied().unwrap_or(0),
        future: false,
        available: true,
    }) {
        Some(bars) => (bars, false),
        None => (Vec::new(), true),
    }
}

#[cfg(test)]
#[path = "statistics_day_work_tests.rs"]
mod day_work_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn session(id: &str, start: &str, end: &str, seconds: u64) -> ReadingSession {
        ReadingSession {
            id: id.into(),
            book_id: "book-1".into(),
            started_at: at(start).timestamp(),
            ended_at: at(end).timestamp(),
            seconds,
        }
    }

    #[test]
    fn shanghai_midnight_splits_duration_without_losing_seconds() {
        let records = [session(
            "one",
            "2026-11-02T15:59:30Z",
            "2026-11-02T16:00:30Z",
            41,
        )];
        let report = summarize("week", 0, at("2026-11-03T05:00:00Z"), &records, &[], &[]);
        assert_eq!(report.seconds, 41);
        assert_eq!(report.days, 2);
        assert_eq!(report.bars.len(), 7);
        assert_eq!(report.bars[0].seconds, 20);
        assert_eq!(report.bars[1].seconds, 21);
        assert_eq!(report.average_seconds, 20);
        assert!(report.local_only);
        assert!(report.bars[2].future);
    }

    #[test]
    fn month_handles_leap_days_and_clips_overlapping_and_future_sessions() {
        let records = [
            session("past", "2024-01-31T15:59:00Z", "2024-01-31T16:01:00Z", 120),
            session("future", "2024-03-01T00:00:00Z", "2024-03-01T00:01:00Z", 60),
        ];
        let report = summarize("month", -1, at("2024-03-01T05:00:00Z"), &records, &[], &[]);
        assert_eq!(report.period_label, "2024-02");
        assert_eq!(report.bars.len(), 29);
        assert_eq!(report.seconds, 60);
        assert_eq!(report.bars[0].seconds, 60);
        assert_eq!(report.calendar.len(), 29);
        assert_eq!(report.calendar_offset, 3);
    }

    #[test]
    fn future_offsets_clamp_and_rankings_use_real_local_sessions() {
        let now = at("2026-11-03T06:00:00Z");
        let record = session("one", "2026-11-03T05:00:00Z", "2026-11-03T05:10:00Z", 600);
        let mut book = Book::new("book-1", "Real local title");
        book.category = "Essays".into();
        book.finished = true;
        let annotation = Annotation {
            created_at: record.started_at,
            ..Annotation::default()
        };
        let report = summarize(
            "month",
            7,
            now,
            &[record.clone(), record],
            &[book],
            &[annotation],
        );
        assert_eq!(report.offset, 0);
        assert_eq!(report.seconds, 600);
        assert_eq!(report.average_seconds, 200);
        assert_eq!(report.ranking.len(), 1);
        assert_eq!(report.ranking[0].title, "Real local title");
        assert_eq!(report.finished, 1);
        assert_eq!(report.annotations, 1);
        assert_eq!(report.preferences[0].percent, 100);
    }

    #[test]
    fn year_and_total_fill_empty_bins_and_empty_reports_do_not_fabricate_history() {
        let now = at("2026-11-03T06:00:00Z");
        let empty = summarize("total", 0, now, &[], &[], &[]);
        assert_eq!(empty.seconds, 0);
        assert!(empty.ranking.is_empty());
        assert_eq!(empty.bars.len(), 1);
        let records = [
            session("old", "2024-06-01T00:00:00Z", "2024-06-01T00:01:00Z", 60),
            session("new", "2026-11-03T05:00:00Z", "2026-11-03T05:01:00Z", 60),
        ];
        let total = summarize("total", 0, now, &records, &[], &[]);
        assert_eq!(total.bars.len(), 3);
        assert_eq!(total.bars[1].seconds, 0);
        let year = summarize("year", 0, now, &records, &[], &[]);
        assert_eq!(year.bars.len(), 12);
        assert_eq!(year.bars[10].seconds, 60);
        assert!(year.bars[11].future);
    }

    #[test]
    fn calendar_uses_the_whole_selected_month_and_total_includes_earlier_annotations() {
        let now = at("2026-11-20T06:00:00Z");
        let records = [session(
            "old",
            "2026-11-02T00:00:00Z",
            "2026-11-02T00:01:00Z",
            60,
        )];
        let week = summarize("week", 0, now, &records, &[], &[]);
        assert_eq!(week.seconds, 0);
        assert_eq!(week.calendar[1], 1);
        let notes = [Annotation {
            created_at: at("2025-03-01T05:00:00Z").timestamp(),
            ..Annotation::default()
        }];
        let total = summarize("total", 0, now, &records, &[], &notes);
        assert_eq!(total.annotations, 1);
        assert!(total.period_label.starts_with("2025-03-01"));
    }

    fn remote(raw: Value) -> ReadStatistics {
        ReadStatistics {
            base_time: json_timestamp(raw.get("baseTime")).unwrap_or(0),
            raw,
            ..ReadStatistics::default()
        }
    }

    #[test]
    fn cloud_fixture_preserves_timestamp_keys_sparse_days_and_independent_totals() {
        // Generated with M.normalize in the pinned Lua module on test-env.
        // Lua returns buckets (1704038400, "1", 60), (1704211200, "3", 120),
        // total=900, days=2, average=128. Jan 2 is absent, not a zero bucket.
        let response = remote(serde_json::json!({
            "baseTime": 1704038400,
            "totalReadTime": 900,
            "readDays": 2,
            "dayAverageReadTime": 128,
            "readTimes": {"1704211200":"120", "1704038400":60, "invalid":999}
        }));
        let report = from_remote(&response, "week", 0, at("2024-01-03T20:00:00Z"));
        assert_eq!(report.range_start, 1704038400);
        assert_eq!(report.seconds, Some(900));
        assert_eq!(report.days, Some(2));
        assert_eq!(report.average_seconds, Some(128));
        assert_eq!(report.bars.len(), 7);
        assert_eq!(report.bars[0].seconds, 60);
        assert!(report.bars[0].available);
        assert!(!report.bars[1].available);
        assert!(!report.bars[1].future);
        assert_eq!(report.bars[2].seconds, 120);
        assert_eq!(report.calendar[0], 1);
        assert_eq!(report.calendar[1], -1);
        assert!(report.missing_details);
    }

    #[test]
    fn cloud_annual_buckets_use_beijing_months_without_recomputing_average() {
        let march = at("2024-02-29T16:00:00Z").timestamp().to_string();
        let response = remote(serde_json::json!({
            "baseTime":1704038400, "totalReadTime":900,
            "readDays":2, "dayAverageReadTime":128,
            "readTimes":{"1704038400":60, march:0}
        }));
        let report = from_remote(&response, "year", 0, at("2024-04-06T00:00:00Z"));
        assert_eq!(report.period_label, "2024");
        assert_eq!(report.bars.len(), 12);
        assert_eq!(report.bars[0].seconds, 60);
        assert!(!report.bars[1].available);
        assert!(report.bars[2].available);
        assert_eq!(report.bars[2].seconds, 0);
        assert_eq!(report.average_seconds, Some(128));
        assert!(report.calendar.is_empty());
        assert!(report.bars[4].future);
    }

    #[test]
    fn cloud_overall_years_keep_unknown_gaps_and_do_not_create_a_daily_calendar() {
        let first = at("2023-05-31T16:00:00Z").timestamp().to_string();
        let last = at("2025-05-31T16:00:00Z").timestamp().to_string();
        let response = remote(serde_json::json!({
            "totalReadTime":900, "readTimes":{first:60, last:120}
        }));
        let now = at("2026-02-02T00:00:00Z");
        let report = from_remote(&response, "total", 0, now);
        assert_eq!(report.range_start, 0);
        assert_eq!(
            report
                .bars
                .iter()
                .map(|bar| bar.label.as_str())
                .collect::<Vec<_>>(),
            ["2023", "2024", "2025", "2026"]
        );
        assert!(report.bars[0].available);
        assert!(!report.bars[1].available);
        assert!(report.bars[2].available);
        assert!(!report.bars[3].available);
        assert!(report.calendar.is_empty());
        let empty = from_remote(&remote(serde_json::json!({})), "total", 0, now);
        assert!(empty.bars.is_empty());
        assert_eq!(empty.seconds, None);
        assert_eq!(empty.days, None);
        assert_eq!(empty.average_seconds, None);
    }

    #[test]
    fn cloud_invalid_values_and_numeric_indices_never_become_zero_reading() {
        let response = remote(serde_json::json!({
            "baseTime":1704038400,
            "totalReadTime":"invalid",
            "readTimes":{"1704038400":"invalid", "1":60, "2":120}
        }));
        let report = from_remote(&response, "week", 0, at("2024-01-04T00:00:00Z"));
        assert_eq!(report.seconds, None);
        assert_eq!(report.days, None);
        assert_eq!(report.average_seconds, None);
        assert!(report.bars.iter().all(|bar| !bar.available));
        assert!(report.calendar.is_empty());
    }

    #[test]
    fn server_canonical_base_and_request_periods_follow_utc_plus_eight() {
        let now = at("2026-11-01T17:00:00Z");
        assert_eq!(
            period_base_time("week", 0, now),
            Some(at("2026-11-01T16:00:00Z").timestamp())
        );
        assert_eq!(
            period_base_time("week", 1, now),
            period_base_time("week", 0, now)
        );
        assert_eq!(
            period_base_time("month", -1, at("2024-03-01T00:00:00Z")),
            Some(at("2024-01-31T16:00:00Z").timestamp())
        );
        assert_eq!(period_base_time("total", -7, now), None);
        let report = from_remote(
            &remote(serde_json::json!({"baseTime":1704038400})),
            "month",
            0,
            now,
        );
        assert_eq!(report.period_label, "2024-01");
        assert_eq!(report.offset, -34);
    }

    #[test]
    fn cloud_ranking_categories_and_summary_use_only_acknowledged_payload_fields() {
        let response = remote(serde_json::json!({
            "readLongest":[
                {"book":{"bookId":"remote-book","title":"Remote title","author":"Remote author"},"readTime":"300"},
                {"albumInfo":{"name":"Remote audio","authorName":"Audio author"},"readTime":100},
                {"book":{"title":"Unknown duration"}}
            ],
            "preferCategory":[
                {"categoryTitle":"Essays","readingTime":80,"val":1},
                {"categoryTitle":"History","readingTime":20,"val":99},
                {"categoryTitle":"Unknown duration","val":100}
            ],
            "readStat":[{"stat":"Completed","counts":"4 books"}]
        }));
        let report = from_remote(&response, "total", 0, at("2026-11-03T00:00:00Z"));
        assert_eq!(report.ranking.len(), 2);
        assert_eq!(report.ranking[0].book_id, "remote-book");
        assert_eq!(report.ranking[0].title, "Remote title");
        assert_eq!(report.ranking[0].seconds, 300);
        assert!(report.ranking[1].book_id.is_empty());
        assert_eq!(report.preferences.len(), 2);
        assert_eq!(report.preferences[0].percent, 80);
        assert_eq!(report.preferences[1].percent, 20);
        assert_eq!(report.summary[0].label, "Completed");
        assert_eq!(report.summary[0].value, "4 books");
        let empty = from_remote(
            &remote(serde_json::json!({})),
            "total",
            0,
            at("2026-11-03T00:00:00Z"),
        );
        assert!(empty.ranking.is_empty());
        assert!(empty.preferences.is_empty());
        assert!(empty.summary.is_empty());
    }
}
