//! Durable state transitions for a transport without an idempotency key.
//!
//! The controller must persist `Sending` before starting the request. A failed
//! persistence operation must prevent the request from being sent.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::source::report::{ReadReportReceipt, ReadReportRequest};

const MAX_ALLOCATION_BATCHES: usize = 16;
const MAX_ALLOCATION_RETAINED_BATCHES: usize = 8192;
const MAX_ALLOCATION_METADATA_BYTES: usize = 64 * 1024 * 1024;
const MAX_ALLOCATION_ENCODED_BYTES: usize = 16 * 1024 * 1024;
const MAX_ALLOCATION_VALIDATION_WORK: usize = 128 * 1024 * 1024;

/// Existing delivery evidence has its own operation allowances. Refusing a new
/// allocation must never reduce the budget for validating or sending Pending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct ReportInputLimits {
    pub encoded_bytes: usize,
    pub metadata_bytes: usize,
    pub scratch_bytes: usize,
    pub work_bytes: usize,
    pub terminal_headroom: usize,
}

impl Default for ReportInputLimits {
    fn default() -> Self {
        Self {
            encoded_bytes: 32 * 1024 * 1024,
            metadata_bytes: 64 * 1024 * 1024,
            scratch_bytes: 64 * 1024 * 1024,
            work_bytes: 128 * 1024 * 1024,
            terminal_headroom: 32 * 1024,
        }
    }
}

impl ReportInputLimits {
    fn tightened(self) -> Self {
        let maximum = Self::default();
        Self {
            encoded_bytes: self.encoded_bytes.min(maximum.encoded_bytes),
            metadata_bytes: self.metadata_bytes.min(maximum.metadata_bytes),
            scratch_bytes: self.scratch_bytes.min(maximum.scratch_bytes),
            work_bytes: self.work_bytes.min(maximum.work_bytes),
            terminal_headroom: self.terminal_headroom.min(maximum.terminal_headroom),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("report input {resource} exceeds its operation allowance")]
pub(in crate::app) struct ReportInputCapacity {
    pub resource: &'static str,
}

#[derive(Clone, Copy)]
pub(super) enum ReportInputPhase {
    Validation,
    Cursor,
    Compaction,
    Results,
    Transport,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct ReportInputStats {
    pub preflights: usize,
    pub rejections: usize,
    pub validation_builds: usize,
    pub interval_builds: usize,
    pub interval_items: usize,
    pub compaction_builds: usize,
    pub old_request_copies: usize,
    pub old_request_bytes: usize,
    pub result_headers: usize,
    pub result_header_bytes: usize,
    pub transport_copies: usize,
    pub transport_bytes: usize,
    pub persistence_encodes: usize,
    pub persistence_bytes: usize,
    pub metadata_bytes: usize,
    pub scratch_bytes: usize,
    pub peak_scratch_bytes: usize,
    pub work_bytes: usize,
}

#[cfg(test)]
std::thread_local! {
    static REPORT_INPUT_LIMITS: std::cell::Cell<Option<ReportInputLimits>> = const { std::cell::Cell::new(None) };
    static REPORT_INPUT_STATS: std::cell::Cell<Option<ReportInputStats>> = const { std::cell::Cell::new(None) };
}

pub(super) fn report_input_limits() -> ReportInputLimits {
    #[cfg(test)]
    if let Some(limits) = REPORT_INPUT_LIMITS.with(std::cell::Cell::get) {
        return limits.tightened();
    }
    ReportInputLimits::default()
}

pub(super) fn report_settlement_limits() -> ReportInputLimits {
    let limits = report_input_limits();
    if limits != ReportInputLimits::default() {
        return limits;
    }
    // A bounded terminal header reserve supports old already-attempted records
    // as well as this version's Sending admission. Explicit lower input quotas
    // still defer heavy work; their known response is preserved independently.
    ReportInputLimits {
        encoded_bytes: limits.encoded_bytes + limits.terminal_headroom,
        metadata_bytes: limits.metadata_bytes + limits.terminal_headroom,
        scratch_bytes: limits.scratch_bytes + 2 * limits.terminal_headroom,
        work_bytes: limits.work_bytes + 16 * limits.terminal_headroom,
        ..limits
    }
}

#[cfg(test)]
pub(in crate::app) fn with_report_input_test_limits<T>(
    limits: ReportInputLimits,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<ReportInputLimits>, Option<ReportInputStats>);
    impl Drop for Restore {
        fn drop(&mut self) {
            REPORT_INPUT_LIMITS.with(|state| state.set(self.0));
            REPORT_INPUT_STATS.with(|state| state.set(self.1));
        }
    }
    let _restore = Restore(
        REPORT_INPUT_LIMITS.with(|state| state.replace(Some(limits.tightened()))),
        REPORT_INPUT_STATS.with(|state| state.replace(Some(ReportInputStats::default()))),
    );
    run()
}

#[cfg(test)]
pub(in crate::app) fn report_input_test_stats() -> ReportInputStats {
    REPORT_INPUT_STATS.with(|state| state.get().unwrap_or_default())
}

pub(super) fn observe_report_input(observe: impl FnOnce(&mut ReportInputStats)) {
    #[cfg(test)]
    REPORT_INPUT_STATS.with(|state| {
        if let Some(mut stats) = state.get() {
            observe(&mut stats);
            state.set(Some(stats));
        }
    });
    #[cfg(not(test))]
    let _ = observe;
}

fn input_add(left: usize, right: usize) -> Result<usize> {
    left.checked_add(right).ok_or_else(|| {
        ReportInputCapacity {
            resource: "arithmetic",
        }
        .into()
    })
}

fn input_mul(left: usize, right: usize) -> Result<usize> {
    left.checked_mul(right).ok_or_else(|| {
        ReportInputCapacity {
            resource: "arithmetic",
        }
        .into()
    })
}

pub(super) fn report_input_encoded_bytes(value: &impl Serialize, maximum: usize) -> Result<usize> {
    allocation_encoded_bytes(value, maximum).map_err(|error| {
        if error.downcast_ref::<AllocationCapacity>().is_some() {
            ReportInputCapacity {
                resource: "encoded bytes",
            }
            .into()
        } else {
            error
        }
    })
}

/// Allowances belong to one allocation operation, not to retained delivery proof.
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct AllocationLimits {
    pub batches: usize,
    pub retained_batches: usize,
    pub metadata_bytes: usize,
    pub encoded_bytes: usize,
    pub validation_work: usize,
}

impl Default for AllocationLimits {
    fn default() -> Self {
        Self {
            batches: MAX_ALLOCATION_BATCHES,
            retained_batches: MAX_ALLOCATION_RETAINED_BATCHES,
            metadata_bytes: MAX_ALLOCATION_METADATA_BYTES,
            encoded_bytes: MAX_ALLOCATION_ENCODED_BYTES,
            validation_work: MAX_ALLOCATION_VALIDATION_WORK,
        }
    }
}

impl AllocationLimits {
    fn tightened(self) -> Self {
        let maximum = Self::default();
        Self {
            batches: self.batches.min(maximum.batches),
            retained_batches: self.retained_batches.min(maximum.retained_batches),
            metadata_bytes: self.metadata_bytes.min(maximum.metadata_bytes),
            encoded_bytes: self.encoded_bytes.min(maximum.encoded_bytes),
            validation_work: self.validation_work.min(maximum.validation_work),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("report allocation {resource} exceeds its operation allowance")]
pub(in crate::app) struct AllocationCapacity {
    pub resource: &'static str,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct AllocationStats {
    pub remaining_batches: u64,
    pub admitted_batches: usize,
    pub batch_copies: usize,
    pub batch_string_bytes: usize,
    pub old_batch_copies: usize,
    pub old_string_bytes: usize,
    pub validation_calls: usize,
    pub validation_batch_visits: usize,
    pub existing_comparisons: usize,
    pub encoded_inspected_bytes: usize,
    pub metadata_bytes: usize,
    pub encoded_bytes: usize,
    pub validation_work: usize,
}

#[cfg(test)]
std::thread_local! {
    static ALLOCATION_LIMITS: std::cell::Cell<Option<AllocationLimits>> = const { std::cell::Cell::new(None) };
    static ALLOCATION_STATS: std::cell::Cell<Option<AllocationStats>> = const { std::cell::Cell::new(None) };
}

pub(super) fn allocation_limits() -> AllocationLimits {
    #[cfg(test)]
    if let Some(limits) = ALLOCATION_LIMITS.with(std::cell::Cell::get) {
        return limits.tightened();
    }
    AllocationLimits::default()
}

#[cfg(test)]
pub(in crate::app) fn with_allocation_test_limits<T>(
    limits: AllocationLimits,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<AllocationLimits>, Option<AllocationStats>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ALLOCATION_LIMITS.with(|state| state.set(self.0));
            ALLOCATION_STATS.with(|state| state.set(self.1));
        }
    }
    let _restore = Restore(
        ALLOCATION_LIMITS.with(|state| state.replace(Some(limits.tightened()))),
        ALLOCATION_STATS.with(|state| state.replace(Some(AllocationStats::default()))),
    );
    run()
}

#[cfg(test)]
pub(in crate::app) fn allocation_test_stats() -> AllocationStats {
    ALLOCATION_STATS.with(|state| state.get().unwrap_or_default())
}

fn observe_allocation(observe: impl FnOnce(&mut AllocationStats)) {
    #[cfg(test)]
    ALLOCATION_STATS.with(|state| {
        if let Some(mut stats) = state.get() {
            observe(&mut stats);
            state.set(Some(stats));
        }
    });
    #[cfg(not(test))]
    let _ = observe;
}

fn allocation_add(left: usize, right: usize) -> Result<usize> {
    left.checked_add(right).ok_or_else(|| {
        AllocationCapacity {
            resource: "arithmetic",
        }
        .into()
    })
}

fn allocation_mul(left: usize, right: usize) -> Result<usize> {
    left.checked_mul(right).ok_or_else(|| {
        AllocationCapacity {
            resource: "arithmetic",
        }
        .into()
    })
}

fn allocation_admit(resource: &'static str, bytes: usize, limit: usize) -> Result<()> {
    if bytes > limit {
        return Err(AllocationCapacity { resource }.into());
    }
    Ok(())
}

/// Count the exact compact JSON format used by Store, without an owned JSON buffer.
pub(super) fn allocation_encoded_bytes(value: &impl Serialize, maximum: usize) -> Result<usize> {
    struct Counter {
        bytes: usize,
        maximum: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, value: &[u8]) -> std::io::Result<usize> {
            observe_allocation(|stats| {
                stats.encoded_inspected_bytes =
                    stats.encoded_inspected_bytes.saturating_add(value.len());
            });
            let end = self
                .bytes
                .checked_add(value.len())
                .filter(|bytes| *bytes <= self.maximum)
                .ok_or_else(|| std::io::Error::other("report allocation encoded allowance"))?;
            self.bytes = end;
            Ok(value.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { bytes: 0, maximum };
    serde_json::to_writer(&mut counter, value).map_err(|_| AllocationCapacity {
        resource: "encoded bytes",
    })?;
    Ok(counter.bytes)
}

pub(super) struct AllocationInput<'a> {
    pub run_id: &'a str,
    pub account_key: &'a str,
    pub book_id: &'a str,
    pub started_at: i64,
    pub allocated_seconds: u64,
    pub target_seconds: u64,
    pub position: Option<&'a crate::source::ReadingPosition>,
    pub progress_write_allowed: bool,
}

pub(super) struct PreparedAllocation {
    pub replacement: ReportOutbox,
    pub removed: Vec<String>,
    pub added: usize,
    pub cursor: u64,
    pub complete: bool,
    pub encoded_bytes: usize,
}

pub(super) struct AllocationExtra {
    pub metadata_bytes: usize,
    pub encoded_bytes: usize,
    pub cursor: u64,
    pub fixed_encoded_bytes: usize,
    pub omit_at_target: bool,
}

impl AllocationExtra {
    fn encoded_at(&self, cursor: u64, target: u64) -> Result<usize> {
        if self.encoded_bytes == 0 || self.omit_at_target && cursor == target {
            return Ok(self.fixed_encoded_bytes);
        }
        let before = self.cursor.checked_ilog10().unwrap_or(0) as usize + 1;
        let after = cursor.checked_ilog10().unwrap_or(0) as usize + 1;
        let bytes = self
            .encoded_bytes
            .checked_sub(before)
            .ok_or_else(|| anyhow::anyhow!("invalid report allocation encoding baseline"))?;
        allocation_add(allocation_add(bytes, after)?, self.fixed_encoded_bytes)
    }
}

/// Inline strings are bounded admission metadata, never owned request fields.
struct InlineText<const N: usize> {
    bytes: [u8; N],
    length: usize,
}

impl<const N: usize> InlineText<N> {
    fn empty() -> Self {
        Self {
            bytes: [0; N],
            length: 0,
        }
    }
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.length])
            .expect("Inline metadata contains complete UTF-8")
    }
}

impl<const N: usize> std::fmt::Write for InlineText<N> {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        let end = self
            .length
            .checked_add(value.len())
            .filter(|end| *end <= N)
            .ok_or(std::fmt::Error)?;
        self.bytes[self.length..end].copy_from_slice(value.as_bytes());
        self.length = end;
        Ok(())
    }
}

impl<const N: usize> Serialize for InlineText<N> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

fn inline_session_id(
    account_key: &str,
    run_id: &str,
    from: u64,
    to: u64,
) -> Result<InlineText<512>> {
    use std::fmt::Write as _;
    let account = account_key
        .get(..account_key.len().min(12))
        .ok_or_else(|| anyhow::anyhow!("invalid report session namespace"))?;
    let mut id = InlineText::empty();
    write!(&mut id, "{account}-{run_id}-{from}-{to}").map_err(|_| AllocationCapacity {
        resource: "session identity",
    })?;
    Ok(id)
}

#[derive(Serialize)]
struct PositionPreview<'a> {
    chapter_uid: &'a str,
    chapter_index: u32,
    chapter_offset: u32,
    progress: u32,
    summary: InlineText<80>,
    updated_at: i64,
    device: &'a str,
}

impl<'a> PositionPreview<'a> {
    fn new(position: &'a crate::source::ReadingPosition) -> Self {
        use std::fmt::Write as _;
        let mut summary = InlineText::empty();
        for character in position.summary.chars().take(super::REPORT_SUMMARY_SCALARS) {
            summary
                .write_char(character)
                .expect("Twenty Unicode scalars fit the inline summary");
        }
        Self {
            chapter_uid: &position.chapter_uid,
            chapter_index: position.chapter_index,
            chapter_offset: position.chapter_offset,
            progress: position.progress,
            summary,
            updated_at: position.updated_at,
            device: &position.device,
        }
    }
    fn text_bytes(&self) -> Result<usize> {
        allocation_add(
            allocation_add(self.chapter_uid.len(), self.device.len())?,
            self.summary.length,
        )
    }
    fn into_owned(self) -> crate::source::ReadingPosition {
        crate::source::ReadingPosition {
            chapter_uid: self.chapter_uid.to_owned(),
            chapter_index: self.chapter_index,
            chapter_offset: self.chapter_offset,
            progress: self.progress,
            summary: self.summary.as_str().to_owned(),
            updated_at: self.updated_at,
            device: self.device.to_owned(),
        }
    }
}

#[derive(Serialize)]
struct RequestPreview<'a> {
    session_id: InlineText<512>,
    account_key: &'a str,
    book_id: &'a str,
    started_at: i64,
    seconds: u32,
    progress_write_allowed: bool,
    active_reader_intent: bool,
    position: Option<PositionPreview<'a>>,
}

#[derive(Serialize)]
struct BatchPreview<'a> {
    request: RequestPreview<'a>,
    run_id: &'a str,
    from: u64,
    to: u64,
    state: BatchState,
    attempted_at: Option<i64>,
    retry_after: i64,
    attempts: u32,
    error: Option<&'a str>,
}

impl<'a> BatchPreview<'a> {
    fn new(input: &AllocationInput<'a>, from: u64, to: u64) -> Result<Self> {
        Ok(Self {
            request: RequestPreview {
                session_id: inline_session_id(input.account_key, input.run_id, from, to)?,
                account_key: input.account_key,
                book_id: input.book_id,
                started_at: input.started_at,
                seconds: u32::try_from(to - from).map_err(|_| AllocationCapacity {
                    resource: "seconds",
                })?,
                progress_write_allowed: input.progress_write_allowed,
                active_reader_intent: false,
                position: input.position.map(PositionPreview::new),
            },
            run_id: input.run_id,
            from,
            to,
            state: BatchState::Pending,
            attempted_at: None,
            retry_after: 0,
            attempts: 0,
            error: None,
        })
    }
    fn text_bytes(&self) -> Result<usize> {
        let mut bytes = allocation_add(self.request.session_id.length, self.run_id.len())?;
        bytes = allocation_add(bytes, self.request.account_key.len())?;
        bytes = allocation_add(bytes, self.request.book_id.len())?;
        allocation_add(
            bytes,
            self.request
                .position
                .as_ref()
                .map(PositionPreview::text_bytes)
                .transpose()?
                .unwrap_or(0),
        )
    }
    fn into_owned(self) -> ReportBatch {
        let bytes = self
            .text_bytes()
            .expect("An admitted batch has a checked fee");
        observe_allocation(|stats| {
            stats.batch_copies += 1;
            stats.batch_string_bytes = stats.batch_string_bytes.saturating_add(bytes);
        });
        ReportBatch {
            request: ReadReportRequest {
                session_id: self.request.session_id.as_str().to_owned(),
                account_key: self.request.account_key.to_owned(),
                book_id: self.request.book_id.to_owned(),
                started_at: self.request.started_at,
                seconds: self.request.seconds,
                progress_write_allowed: self.request.progress_write_allowed,
                active_reader_intent: false,
                position: self.request.position.map(PositionPreview::into_owned),
            },
            run_id: self.run_id.to_owned(),
            from: self.from,
            to: self.to,
            state: self.state,
            attempted_at: self.attempted_at,
            retry_after: self.retry_after,
            attempts: self.attempts,
            error: None,
        }
    }
}

#[derive(Serialize)]
struct ProofPreview<'a> {
    account_key: &'a str,
    book_id: &'a str,
    started_at: i64,
    confirmed_until: u64,
    confirmed_ranges: Vec<ConfirmedRange>,
}

/// Only numeric range metadata is staged before request-field admission.
struct CompactionPreview<'a> {
    batches: Vec<&'a ReportBatch>,
    reported_cursors: &'a BTreeMap<String, u64>,
    compacted_runs: BTreeMap<&'a str, ProofPreview<'a>>,
    removed: Vec<&'a str>,
}

impl Serialize for CompactionPreview<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ReportOutbox", 3)?;
        state.serialize_field("batches_v2", &self.batches)?;
        state.serialize_field("reported_cursors", &self.reported_cursors)?;
        state.serialize_field("compacted_runs", &self.compacted_runs)?;
        state.end()
    }
}

impl CompactionPreview<'_> {
    fn into_owned(self) -> Result<(ReportOutbox, Vec<String>)> {
        let batches = self
            .batches
            .into_iter()
            .map(|batch| {
                let bytes =
                    batch_text_bytes(batch).expect("An admitted old record has a checked fee");
                observe_allocation(|stats| {
                    stats.old_batch_copies += 1;
                    stats.old_string_bytes = stats.old_string_bytes.saturating_add(bytes);
                });
                observe_report_input(|stats| {
                    stats.old_request_copies += 1;
                    stats.old_request_bytes += bytes;
                });
                batch.clone()
            })
            .collect();
        let compacted_runs = self
            .compacted_runs
            .into_iter()
            .map(|(id, proof)| {
                observe_allocation(|stats| {
                    stats.old_string_bytes = stats
                        .old_string_bytes
                        .saturating_add(id.len() + proof.account_key.len() + proof.book_id.len());
                });
                (
                    id.to_owned(),
                    ConfirmedRun {
                        account_key: proof.account_key.to_owned(),
                        book_id: proof.book_id.to_owned(),
                        started_at: proof.started_at,
                        confirmed_until: proof.confirmed_until,
                        confirmed_ranges: proof.confirmed_ranges,
                    },
                )
            })
            .collect();
        observe_allocation(|stats| {
            stats.old_string_bytes = stats
                .old_string_bytes
                .saturating_add(self.reported_cursors.keys().map(String::len).sum::<usize>());
        });
        let removed = self.removed.into_iter().map(str::to_owned).collect();
        Ok((
            ReportOutbox {
                batches,
                reported_cursors: (*self.reported_cursors).clone(),
                compacted_runs,
            },
            removed,
        ))
    }
}

fn batch_text_bytes(batch: &ReportBatch) -> Result<usize> {
    let request = &batch.request;
    let mut bytes = 0;
    for length in [
        request.session_id.len(),
        request.account_key.len(),
        request.book_id.len(),
        batch.run_id.len(),
        batch.error.as_ref().map_or(0, String::len),
    ] {
        bytes = allocation_add(bytes, length)?;
    }
    if let Some(position) = &request.position {
        for length in [
            position.chapter_uid.len(),
            position.summary.len(),
            position.device.len(),
        ] {
            bytes = allocation_add(bytes, length)?;
        }
    }
    Ok(bytes)
}

const MAX_SAFE_ATTEMPTS: u32 = 5;
const INITIAL_RETRY_SECONDS: i64 = 5;
const MAX_RETRY_SECONDS: i64 = 300;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BatchState {
    Pending,
    Sending,
    Uncertain,
    Confirmed,
    Blocked,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ReportBatch {
    pub request: ReadReportRequest,
    /// A locally allocated Reader run, scoped to one account and one book.
    pub run_id: String,
    /// Half-open offsets in the run's active-reading seconds, not wall time.
    pub from: u64,
    pub to: u64,
    pub state: BatchState,
    pub attempted_at: Option<i64>,
    pub retry_after: i64,
    pub attempts: u32,
    /// A stable, non-sensitive error code supplied by the controller.
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct ReportOutbox {
    /// Keep unresolved and legacy records until their proof can be represented losslessly.
    /// Old readers must reject range proof instead of silently ignoring it.
    #[serde(rename = "batches_v2")]
    pub batches: Vec<ReportBatch>,
    /// The contiguous confirmed offset for each run, distinct from allocation.
    pub reported_cursors: BTreeMap<String, u64>,
    /// Canonical confirmed ranges replace per-minute receipts, including after gaps.
    pub compacted_runs: BTreeMap<String, ConfirmedRun>,
}

impl<'de> Deserialize<'de> for ReportOutbox {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct StoredProof {
            account_key: String,
            book_id: String,
            started_at: i64,
            confirmed_until: u64,
            #[serde(default, deserialize_with = "deserialize_present")]
            confirmed_ranges: Option<Vec<ConfirmedRange>>,
        }

        #[derive(Deserialize)]
        struct Snapshot {
            #[serde(default, deserialize_with = "deserialize_present")]
            batches_v2: Option<Vec<ReportBatch>>,
            #[serde(default, deserialize_with = "deserialize_present")]
            batches: Option<Vec<ReportBatch>>,
            reported_cursors: BTreeMap<String, u64>,
            #[serde(default, deserialize_with = "deserialize_present")]
            compacted_runs: Option<BTreeMap<String, StoredProof>>,
        }

        let snapshot = Snapshot::deserialize(deserializer)?;
        let (batches, stored_proofs, requires_ranges) =
            match (snapshot.batches_v2, snapshot.batches) {
                (Some(batches), None) => (
                    batches,
                    snapshot.compacted_runs.ok_or_else(|| {
                        <D::Error as serde::de::Error>::missing_field("compacted_runs")
                    })?,
                    true,
                ),
                (None, Some(batches)) => {
                    (batches, snapshot.compacted_runs.unwrap_or_default(), false)
                }
                (Some(_), Some(_)) => {
                    return Err(<D::Error as serde::de::Error>::custom(
                        "duplicate batch field versions",
                    ));
                }
                (None, None) => {
                    return Err(<D::Error as serde::de::Error>::missing_field("batches_v2"));
                }
            };
        let mut compacted_runs = BTreeMap::new();
        for (run_id, proof) in stored_proofs {
            let confirmed_ranges = if requires_ranges {
                proof.confirmed_ranges.ok_or_else(|| {
                    <D::Error as serde::de::Error>::missing_field("confirmed_ranges")
                })?
            } else {
                proof.confirmed_ranges.unwrap_or_default()
            };
            compacted_runs.insert(
                run_id,
                ConfirmedRun {
                    account_key: proof.account_key,
                    book_id: proof.book_id,
                    started_at: proof.started_at,
                    confirmed_until: proof.confirmed_until,
                    confirmed_ranges,
                },
            );
        }
        Ok(Self {
            batches,
            reported_cursors: snapshot.reported_cursors,
            compacted_runs,
        })
    }
}

fn deserialize_present<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    // Missing legacy fields are allowed; an explicitly null proof is never one.
    T::deserialize(deserializer).map(Some)
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ConfirmedRun {
    pub account_key: String,
    pub book_id: String,
    pub started_at: i64,
    /// Retain the original prefix field so older persisted proof remains readable.
    pub confirmed_until: u64,
    /// Sorted, disjoint, nonadjacent ranges strictly beyond the contiguous prefix.
    pub confirmed_ranges: Vec<ConfirmedRange>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct ConfirmedRange {
    pub from: u64,
    pub to: u64,
}

impl ConfirmedRun {
    fn intervals(&self) -> impl Iterator<Item = ConfirmedRange> + '_ {
        (self.confirmed_until > 0)
            .then_some(ConfirmedRange {
                from: 0,
                to: self.confirmed_until,
            })
            .into_iter()
            .chain(self.confirmed_ranges.iter().copied())
    }

    /// Only call after validating the old snapshot and all newly added ranges.
    fn merge_intervals(&mut self) -> Result<()> {
        let mut intervals = self.intervals().collect::<Vec<_>>();
        intervals.sort_unstable_by_key(|interval| interval.from);
        let mut merged: Vec<ConfirmedRange> = Vec::new();
        for interval in intervals {
            if let Some(previous) = merged.last_mut() {
                ensure!(
                    interval.from >= previous.to,
                    "confirmed proof ranges overlap"
                );
                if interval.from == previous.to {
                    previous.to = interval.to;
                    continue;
                }
            }
            merged.push(interval);
        }
        self.confirmed_until = if merged.first().is_some_and(|range| range.from == 0) {
            merged.remove(0).to
        } else {
            0
        };
        self.confirmed_ranges = merged;
        Ok(())
    }
}

impl ReportOutbox {
    pub fn admit_input(&self, phase: ReportInputPhase, extra_scratch: usize) -> Result<()> {
        self.admit_input_with_work(phase, extra_scratch, 0)
    }

    pub fn admit_input_with_work(
        &self,
        phase: ReportInputPhase,
        extra_scratch: usize,
        extra_work: usize,
    ) -> Result<()> {
        let limits = self.input_operation_limits();
        self.admit_input_with_limits(phase, extra_scratch, extra_work, limits)
    }

    fn input_operation_limits(&self) -> ReportInputLimits {
        // A proven safe failure may restore Pending or Blocked while retaining
        // its real attempt header. It still needs the bounded settlement path;
        // a later retry independently passes Sending's stricter admission.
        if self.batches.iter().any(|batch| batch.attempts > 0) {
            report_settlement_limits()
        } else {
            report_input_limits()
        }
    }

    pub fn validate_loaded_input(&self) -> Result<()> {
        self.admit_input(ReportInputPhase::Validation, 0)?;
        report_input_encoded_bytes(self, self.input_operation_limits().encoded_bytes)?;
        self.validate()
    }

    fn admit_input_with_limits(
        &self,
        phase: ReportInputPhase,
        extra_scratch: usize,
        extra_work: usize,
        limits: ReportInputLimits,
    ) -> Result<()> {
        let mut payload = 0usize;
        let mut identities = 0usize;
        let mut run_keys = 0usize;
        let mut ranges = 0usize;
        let mut maximum_ranges = 0usize;
        let mut maximum_identity = 1usize;
        let mut maximum_key = 1usize;
        for batch in &self.batches {
            payload = input_add(payload, batch_text_bytes(batch)?)?;
            for bytes in [
                batch.request.session_id.len(),
                batch.request.account_key.len(),
                batch.request.book_id.len(),
                batch.run_id.len(),
            ] {
                identities = input_add(identities, bytes)?;
                maximum_identity = maximum_identity.max(bytes);
            }
            run_keys = input_add(run_keys, batch.run_id.len())?;
            maximum_key = maximum_key
                .max(batch.run_id.len())
                .max(batch.request.session_id.len());
        }
        for (id, proof) in &self.compacted_runs {
            for bytes in [id.len(), proof.account_key.len(), proof.book_id.len()] {
                payload = input_add(payload, bytes)?;
                identities = input_add(identities, bytes)?;
                maximum_identity = maximum_identity.max(bytes);
            }
            run_keys = input_add(run_keys, id.len())?;
            maximum_key = maximum_key.max(id.len());
            let count = input_add(
                proof.confirmed_ranges.len(),
                usize::from(proof.confirmed_until > 0),
            )?;
            ranges = input_add(ranges, count)?;
            maximum_ranges = maximum_ranges.max(count);
        }
        for id in self.reported_cursors.keys() {
            payload = input_add(payload, id.len())?;
            run_keys = input_add(run_keys, id.len())?;
            maximum_key = maximum_key.max(id.len());
        }
        let batches = self.batches.len();
        let proofs = self.compacted_runs.len();
        let rows = input_add(
            input_add(batches, proofs)?,
            input_add(ranges, self.reported_cursors.len())?,
        )?;
        let mut metadata = input_add(payload, input_mul(rows, 128)?)?;
        let reference_scratch = input_add(
            4096,
            input_add(input_mul(rows, 256)?, input_mul(run_keys, 2)?)?,
        )?;
        let mut scratch = reference_scratch;
        let scans = input_mul(
            batches,
            input_add(
                1,
                input_add(
                    input_mul(batches, 2)?,
                    input_add(input_mul(proofs, 2)?, ranges)?,
                )?,
            )?,
        )?;
        let comparison_bytes = input_mul(scans, input_add(16, input_mul(maximum_identity, 2)?)?)?;
        let levels = input_add(rows, 1)?.checked_ilog2().unwrap_or(0) as usize + 1;
        let ordered_bytes = input_mul(
            input_mul(rows, levels)?,
            input_add(16, input_mul(maximum_key, 2)?)?,
        )?;
        let interval_count = input_add(ranges, batches)?;
        let interval_levels = input_add(input_add(maximum_ranges, batches)?, 1)?
            .checked_ilog2()
            .unwrap_or(0) as usize
            + 1;
        let numeric_work = input_mul(input_mul(interval_count, interval_levels)?, 32)?;
        let mut work = input_add(
            input_mul(
                input_add(comparison_bytes, input_add(ordered_bytes, numeric_work)?)?,
                2,
            )?,
            input_mul(identities, 4)?,
        )?;
        // JSON can escape one text byte into six output bytes. Include that
        // bounded traversal before measuring an old compact encoding or copying
        // its fields; a long summary cannot bypass a lower work allowance.
        work = input_add(work, input_mul(payload, 8)?)?;
        match phase {
            ReportInputPhase::Compaction => {
                // The original and replacement snapshots coexist until commit.
                // A new proof/range row is covered before any old field is owned.
                metadata = input_mul(metadata, 2)?;
                scratch = input_add(scratch, input_add(payload, input_mul(rows, 128)?)?)?;
                work = input_add(input_mul(work, 2)?, input_mul(payload, 3)?)?;
            }
            ReportInputPhase::Results => {
                // Bounded side values may replace error codes in the retained
                // queue. Their aggregate bytes are prepaid before value reads.
                metadata = input_add(metadata, extra_scratch)?;
                scratch = input_add(scratch, input_add(identities, input_mul(batches, 64)?)?)?;
                // Every receipt may refresh its run's sorted numeric intervals.
                // A proof-only snapshot performs no batch cursor updates.
                let cursor_updates = input_mul(
                    batches,
                    input_add(
                        numeric_work,
                        input_mul(interval_count, input_add(32, input_mul(maximum_key, 2)?)?)?,
                    )?,
                )?;
                work = input_add(work, input_add(cursor_updates, input_mul(identities, 2)?)?)?;
            }
            ReportInputPhase::Cursor => {
                scratch = input_add(4096, input_add(input_mul(interval_count, 48)?, run_keys)?)?;
                work = input_add(
                    numeric_work,
                    input_add(
                        input_mul(batches, input_add(maximum_key, 16)?)?,
                        input_mul(identities, 2)?,
                    )?,
                )?;
            }
            ReportInputPhase::Transport => {
                work = input_add(work, input_mul(extra_scratch, 3)?)?;
            }
            ReportInputPhase::Validation => {}
        }
        scratch = input_add(scratch, extra_scratch)?;
        work = input_add(work, extra_work)?;
        observe_report_input(|stats| {
            stats.preflights += 1;
            stats.metadata_bytes = metadata;
            stats.scratch_bytes = scratch;
            stats.peak_scratch_bytes = stats.peak_scratch_bytes.max(scratch);
            stats.work_bytes = work;
        });
        for (resource, requested, maximum) in [
            ("metadata bytes", metadata, limits.metadata_bytes),
            ("scratch bytes", scratch, limits.scratch_bytes),
            ("work bytes", work, limits.work_bytes),
        ] {
            if requested > maximum {
                observe_report_input(|stats| stats.rejections += 1);
                return Err(ReportInputCapacity { resource }.into());
            }
        }
        Ok(())
    }

    /// Prepare one admitted contiguous prefix without touching the old snapshot.
    /// The complete remaining demand stays numeric; it never sizes a vector.
    pub fn prepare_allocation(
        &self,
        input: AllocationInput<'_>,
        extra: AllocationExtra,
    ) -> Result<Option<PreparedAllocation>> {
        let limits = allocation_limits();
        let cursor = self.allocation_cursor(input.run_id);
        ensure!(
            cursor >= input.allocated_seconds,
            "durable report allocation history was lost"
        );
        ensure!(
            cursor <= input.target_seconds,
            "allocated reading time exceeds observed eligibility"
        );
        let remaining = input.target_seconds - cursor;
        let demand =
            remaining / super::BATCH_SECONDS + u64::from(remaining % super::BATCH_SECONDS != 0);
        observe_allocation(|stats| stats.remaining_batches = demand);
        if demand == 0 {
            return Ok(None);
        }
        ensure!(
            valid_identifier(input.run_id, 512)
                && valid_identifier(input.account_key, 512)
                && valid_identifier(input.book_id, 2048)
                && input.started_at > 0,
            "invalid report allocation identity"
        );
        let capacity = limits
            .batches
            .min(usize::try_from(demand.min(MAX_ALLOCATION_BATCHES as u64))?);
        if capacity == 0 {
            return Err(AllocationCapacity {
                resource: "batch slots",
            }
            .into());
        }

        // Complete borrowed pricing precedes validation's range vectors, the
        // compaction preview, request copies, and Store's compact JSON buffers.
        let mut text_bytes = 0usize;
        let mut id_bytes = 0usize;
        let mut ranges = 0usize;
        let mut maximum_ranges = 0usize;
        let mut maximum_run_key = input.run_id.len();
        let mut maximum_identity = input
            .run_id
            .len()
            .max(input.book_id.len())
            .max(input.account_key.len());
        for batch in &self.batches {
            text_bytes = allocation_add(text_bytes, batch_text_bytes(batch)?)?;
            id_bytes = allocation_add(id_bytes, batch.request.session_id.len())?;
            maximum_identity = maximum_identity
                .max(batch.request.session_id.len())
                .max(batch.run_id.len())
                .max(batch.request.account_key.len())
                .max(batch.request.book_id.len());
            maximum_run_key = maximum_run_key
                .max(batch.run_id.len())
                .max(batch.request.session_id.len());
        }
        for (id, proof) in &self.compacted_runs {
            for bytes in [id.len(), proof.account_key.len(), proof.book_id.len()] {
                text_bytes = allocation_add(text_bytes, bytes)?;
                maximum_identity = maximum_identity.max(bytes);
            }
            let count = allocation_add(
                proof.confirmed_ranges.len(),
                usize::from(proof.confirmed_until > 0),
            )?;
            ranges = allocation_add(ranges, count)?;
            maximum_ranges = maximum_ranges.max(count);
            maximum_run_key = maximum_run_key.max(id.len());
        }
        for id in self.reported_cursors.keys() {
            text_bytes = allocation_add(text_bytes, id.len())?;
            maximum_identity = maximum_identity.max(id.len());
            maximum_run_key = maximum_run_key.max(id.len());
        }
        let rows = allocation_add(
            allocation_add(self.batches.len(), self.compacted_runs.len())?,
            allocation_add(ranges, self.reported_cursors.len())?,
        )?;
        let fixed = allocation_add(
            4096,
            allocation_mul(capacity, std::mem::size_of::<BatchPreview<'_>>())?,
        )?;
        let base_metadata = allocation_add(
            extra.metadata_bytes,
            allocation_add(
                fixed,
                allocation_add(
                    allocation_mul(text_bytes, 3)?,
                    allocation_add(allocation_mul(id_bytes, 8)?, allocation_mul(rows, 512)?)?,
                )?,
            )?,
        )?;
        allocation_admit("metadata bytes", base_metadata, limits.metadata_bytes)?;
        let work_for = |added: usize, new_text: usize| -> Result<usize> {
            let count = allocation_add(rows, added)?;
            let batches = allocation_add(self.batches.len(), added)?;
            let proofs = allocation_add(self.compacted_runs.len(), self.batches.len())?;
            let intervals = allocation_add(ranges, batches)?;
            // Full batch scans and namespace/range comparisons depend on B.
            // Proof-only runs do not introduce a quadratic P-by-P scan.
            let scans = allocation_mul(
                batches,
                allocation_add(
                    1,
                    allocation_add(
                        allocation_mul(batches, 2)?,
                        allocation_add(allocation_mul(proofs, 2)?, intervals)?,
                    )?,
                )?,
            )?;
            let compare_bytes = allocation_mul(
                scans,
                allocation_add(16, allocation_mul(maximum_identity, 2)?)?,
            )?;
            let levels = allocation_add(count, 1)?.checked_ilog2().unwrap_or(0) as usize + 1;
            let ordered = allocation_mul(
                allocation_mul(count, levels)?,
                allocation_add(16, allocation_mul(maximum_run_key, 2)?)?,
            )?;
            let range_levels = allocation_add(allocation_add(maximum_ranges, batches)?, 1)?
                .checked_ilog2()
                .unwrap_or(0) as usize
                + 1;
            let range_work = allocation_mul(allocation_mul(intervals, range_levels)?, 32)?;
            let text = allocation_add(allocation_add(text_bytes, new_text)?, extra.metadata_bytes)?;
            allocation_add(
                allocation_mul(
                    allocation_add(compare_bytes, allocation_add(ordered, range_work)?)?,
                    4,
                )?,
                allocation_mul(text, 8)?,
            )
        };
        allocation_admit("validation work", work_for(0, 0)?, limits.validation_work)?;
        self.validate()?;
        self.ensure_run_identity(
            input.run_id,
            input.account_key,
            input.book_id,
            input.started_at,
        )?;
        let preview = self.compaction_preview()?;
        let capacity = capacity.min(
            limits
                .retained_batches
                .saturating_sub(preview.batches.len()),
        );
        if capacity == 0 {
            return Err(AllocationCapacity {
                resource: "retained batch slots",
            }
            .into());
        }
        let mut encoded = allocation_encoded_bytes(&preview, limits.encoded_bytes)?;
        let new_cursor = !self.reported_cursors.contains_key(input.run_id);
        if new_cursor {
            encoded = allocation_add(
                encoded,
                allocation_encoded_bytes(&input.run_id, limits.encoded_bytes)?,
            )?;
            encoded = allocation_add(encoded, 2 + usize::from(!self.reported_cursors.is_empty()))?;
        }
        let mut previews = Vec::with_capacity(capacity);
        let mut end = cursor;
        let mut metadata = base_metadata;
        let mut new_text = 0;
        let mut admitted_encoded = 0;
        let mut admitted_work = 0;
        let mut rejected_resource = "batch slots";
        for _ in 0..capacity {
            let next = end
                .checked_add((input.target_seconds - end).min(super::BATCH_SECONDS))
                .ok_or_else(|| anyhow::anyhow!("report allocation overflow"))?;
            let batch = BatchPreview::new(&input, end, next)?;
            let bytes = batch.text_bytes()?;
            let candidate_metadata = allocation_add(
                metadata,
                allocation_add(
                    allocation_mul(bytes, 4)?,
                    2 * std::mem::size_of::<ReportBatch>(),
                )?,
            )?;
            if candidate_metadata > limits.metadata_bytes {
                rejected_resource = "metadata bytes";
                break;
            }
            let candidate_text = allocation_add(new_text, bytes)?;
            let work = work_for(previews.len() + 1, candidate_text)?;
            if work > limits.validation_work {
                rejected_resource = "validation work";
                break;
            }
            let batch_encoded = allocation_encoded_bytes(&batch, limits.encoded_bytes)?;
            let candidate_encoded = allocation_add(
                encoded,
                allocation_add(
                    batch_encoded,
                    usize::from(!preview.batches.is_empty() || !previews.is_empty()),
                )?,
            )?;
            let total_encoded = allocation_add(
                candidate_encoded,
                extra.encoded_at(next, input.target_seconds)?,
            )?;
            if total_encoded > limits.encoded_bytes {
                rejected_resource = "encoded bytes";
                break;
            }
            // A legacy identity may alias this canonical namespace. Compare
            // admitted borrowed IDs before owning even the first new request.
            for existing in &self.batches {
                observe_allocation(|stats| stats.existing_comparisons += 1);
                ensure!(
                    existing.request.session_id != batch.request.session_id.as_str(),
                    "duplicate report session identity"
                );
            }
            ensure!(
                !self.contains_compacted_session(batch.request.session_id.as_str()),
                "duplicate compacted report session identity"
            );
            previews.push(batch);
            end = next;
            metadata = candidate_metadata;
            new_text = candidate_text;
            encoded = candidate_encoded;
            admitted_encoded = total_encoded;
            admitted_work = work;
        }
        if previews.is_empty() {
            return Err(AllocationCapacity {
                resource: rejected_resource,
            }
            .into());
        }
        let added = previews.len();
        observe_allocation(|stats| {
            stats.admitted_batches = stats.admitted_batches.saturating_add(added);
            stats.metadata_bytes = metadata;
            stats.encoded_bytes = admitted_encoded;
            stats.validation_work = admitted_work;
        });
        let (mut replacement, removed) = preview.into_owned()?;
        if new_cursor {
            replacement
                .reported_cursors
                .insert(input.run_id.to_owned(), 0);
        }
        // A single extend follows one validated old snapshot. Each interval is
        // contiguous beyond its allocation cursor, so no repeated whole-queue
        // append validation or growing existing-record scan is necessary.
        replacement
            .batches
            .extend(previews.into_iter().map(BatchPreview::into_owned));
        replacement.validate()?;
        ensure!(
            allocation_encoded_bytes(&replacement, limits.encoded_bytes)? == encoded,
            "report allocation JSON forecast does not match its saved format"
        );
        Ok(Some(PreparedAllocation {
            replacement,
            removed,
            added,
            cursor: end,
            complete: end == input.target_seconds,
            encoded_bytes: admitted_encoded,
        }))
    }

    fn compaction_preview(&self) -> Result<CompactionPreview<'_>> {
        observe_report_input(|stats| stats.compaction_builds += 1);
        let mut proofs = BTreeMap::new();
        for (id, proof) in &self.compacted_runs {
            proofs.insert(
                id.as_str(),
                ProofPreview {
                    account_key: &proof.account_key,
                    book_id: &proof.book_id,
                    started_at: proof.started_at,
                    confirmed_until: proof.confirmed_until,
                    confirmed_ranges: proof.confirmed_ranges.clone(),
                },
            );
        }
        let mut batches = Vec::with_capacity(self.batches.len());
        let mut removed = Vec::new();
        for batch in &self.batches {
            let canonical = inline_session_id(
                &batch.request.account_key,
                &batch.run_id,
                batch.from,
                batch.to,
            )
            .ok();
            if batch.state != BatchState::Confirmed
                || canonical.as_ref().map(InlineText::as_str)
                    != Some(batch.request.session_id.as_str())
            {
                batches.push(batch);
                continue;
            }
            let proof = proofs
                .entry(batch.run_id.as_str())
                .or_insert_with(|| ProofPreview {
                    account_key: &batch.request.account_key,
                    book_id: &batch.request.book_id,
                    started_at: batch.request.started_at,
                    confirmed_until: 0,
                    confirmed_ranges: Vec::new(),
                });
            proof.confirmed_ranges.push(ConfirmedRange {
                from: batch.from,
                to: batch.to,
            });
            removed.push(batch.request.session_id.as_str());
        }
        for proof in proofs.values_mut() {
            let mut intervals = Vec::with_capacity(proof.confirmed_ranges.len() + 1);
            if proof.confirmed_until > 0 {
                intervals.push(ConfirmedRange {
                    from: 0,
                    to: proof.confirmed_until,
                });
            }
            intervals.extend(proof.confirmed_ranges.iter().copied());
            intervals.sort_unstable_by_key(|interval| interval.from);
            let mut merged: Vec<ConfirmedRange> = Vec::with_capacity(intervals.len());
            for interval in intervals {
                if let Some(previous) = merged.last_mut() {
                    ensure!(
                        interval.from >= previous.to,
                        "confirmed proof ranges overlap"
                    );
                    if interval.from == previous.to {
                        previous.to = interval.to;
                        continue;
                    }
                }
                merged.push(interval);
            }
            proof.confirmed_until = if merged.first().is_some_and(|range| range.from == 0) {
                merged.remove(0).to
            } else {
                0
            };
            proof.confirmed_ranges = merged;
        }
        Ok(CompactionPreview {
            batches,
            reported_cursors: &self.reported_cursors,
            compacted_runs: proofs,
            removed,
        })
    }

    /// Validate every loaded or saved snapshot without repairing or discarding it.
    /// Callers must fail closed if any retained delivery history is inconsistent.
    pub fn validate(&self) -> Result<()> {
        self.admit_input(ReportInputPhase::Validation, 0)?;
        observe_report_input(|stats| stats.validation_builds += 1);
        observe_allocation(|stats| stats.validation_calls += 1);
        let mut sessions = BTreeSet::new();
        let mut runs: BTreeMap<&str, Vec<&ReportBatch>> = BTreeMap::new();
        let mut sending = 0;
        let mut derived_cursors = BTreeMap::new();
        for (run_id, proof) in &self.compacted_runs {
            ensure!(
                valid_identifier(run_id, 512),
                "invalid compacted report run"
            );
            ensure!(
                valid_identifier(&proof.account_key, 512)
                    && valid_identifier(&proof.book_id, 2048)
                    && proof.started_at > 0
                    && (proof.confirmed_until > 0 || !proof.confirmed_ranges.is_empty())
                    && canonical_session_namespace(&proof.account_key, run_id).is_some(),
                "invalid compacted report proof"
            );
            let mut previous_end = proof.confirmed_until;
            for range in &proof.confirmed_ranges {
                ensure!(
                    range.from > previous_end && range.to > range.from,
                    "compacted ranges are empty, overlapping, adjacent or unsorted"
                );
                previous_end = range.to;
            }
            runs.entry(run_id.as_str()).or_default();
        }
        for batch in &self.batches {
            observe_allocation(|stats| stats.validation_batch_visits += 1);
            validate_batch_shape(batch)?;
            ensure!(
                sessions.insert(batch.request.session_id.as_str())
                    && !self.contains_compacted_session(&batch.request.session_id),
                "duplicate report session identity"
            );
            sending += usize::from(batch.state == BatchState::Sending);
            ensure!(sending <= 1, "multiple report deliveries are in flight");
            runs.entry(batch.run_id.as_str()).or_default().push(batch);
        }

        for (run_id, batches) in runs {
            if let Some(first) = batches.first() {
                self.ensure_run_identity(
                    run_id,
                    &first.request.account_key,
                    &first.request.book_id,
                    first.request.started_at,
                )?;
            }
            let mut cursor = 0;
            let mut previous_end = None;
            let mut contiguous = true;
            for (from, to, confirmed) in self.run_intervals(run_id) {
                ensure!(
                    previous_end.is_none_or(|end| from >= end),
                    "report intervals overlap"
                );
                previous_end = Some(to);
                if contiguous && from == cursor && confirmed {
                    cursor = to;
                } else {
                    contiguous = false;
                }
            }
            derived_cursors.insert(run_id.to_owned(), cursor);
        }
        ensure!(
            self.reported_cursors == derived_cursors,
            "reported cursors do not match retained confirmed intervals"
        );
        Ok(())
    }

    pub fn transport_request(&self, index: usize) -> Result<ReadReportRequest> {
        self.admit_sending()?;
        let request = &self
            .batches
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("pending report disappeared"))?
            .request;
        let mut id = InlineText::<512>::empty();
        std::fmt::Write::write_str(&mut id, &request.session_id).map_err(|_| {
            ReportInputCapacity {
                resource: "session identity",
            }
        })?;
        let position = request.position.as_ref().map(PositionPreview::new);
        let mut bytes = input_add(
            input_add(request.session_id.len(), request.account_key.len())?,
            request.book_id.len(),
        )?;
        bytes = input_add(
            bytes,
            position
                .as_ref()
                .map(PositionPreview::text_bytes)
                .transpose()?
                .unwrap_or(0),
        )?;
        self.admit_input(
            ReportInputPhase::Transport,
            input_add(bytes, std::mem::size_of::<ReadReportRequest>())?,
        )?;
        observe_report_input(|stats| {
            stats.transport_copies += 1;
            stats.transport_bytes += bytes;
        });
        Ok(ReadReportRequest {
            session_id: id.as_str().to_owned(),
            account_key: request.account_key.clone(),
            book_id: request.book_id.clone(),
            started_at: request.started_at,
            seconds: request.seconds,
            progress_write_allowed: request.progress_write_allowed,
            active_reader_intent: request.active_reader_intent,
            position: position.map(PositionPreview::into_owned),
        })
    }

    fn admit_sending(&self) -> Result<()> {
        let limits = report_input_limits();
        let reserve = limits.terminal_headroom;
        let reserved = ReportInputLimits {
            encoded_bytes: limits.encoded_bytes.saturating_sub(reserve),
            metadata_bytes: limits.metadata_bytes.saturating_sub(reserve),
            scratch_bytes: limits.scratch_bytes.saturating_sub(reserve),
            work_bytes: limits.work_bytes.saturating_sub(input_mul(reserve, 16)?),
            terminal_headroom: reserve,
        };
        // A persisted Sending must still fit after receipt confirmation or a
        // bounded uncertain/failure header. Existing payloads remain unchanged.
        self.admit_input_with_limits(ReportInputPhase::Compaction, 0, 0, reserved)?;
        let preview = self.compaction_preview()?;
        let encoded = report_input_encoded_bytes(&preview, reserved.encoded_bytes)?;
        let mut keys = 0usize;
        let mut bytes = 0usize;
        for id in &preview.removed {
            for prefix in super::RESULT_PREFIXES {
                keys = input_add(keys, 1)?;
                bytes = input_add(bytes, input_add(input_add(prefix.len(), 1)?, id.len())?)?;
            }
        }
        let scratch = input_add(
            encoded,
            input_add(bytes, input_mul(keys, std::mem::size_of::<String>())?)?,
        )?;
        self.admit_input_with_limits(
            ReportInputPhase::Compaction,
            scratch,
            input_add(input_mul(bytes, 3)?, input_mul(encoded, 3)?)?,
            reserved,
        )
    }

    pub fn append(&mut self, batch: ReportBatch) -> Result<()> {
        self.validate()?;
        validate_batch_shape(&batch)?;
        self.ensure_run_identity(
            &batch.run_id,
            &batch.request.account_key,
            &batch.request.book_id,
            batch.request.started_at,
        )?;
        ensure!(
            !self.contains_compacted_session(&batch.request.session_id),
            "duplicate compacted report session identity"
        );
        if let Some(proof) = self.compacted_runs.get(&batch.run_id) {
            ensure!(
                proof
                    .intervals()
                    .all(|range| batch.to <= range.from || batch.from >= range.to),
                "report interval overlaps compacted confirmed proof"
            );
        }
        ensure!(
            batch.state == BatchState::Pending
                && batch.attempts == 0
                && batch.attempted_at.is_none()
                && batch.error.is_none()
                && batch.retry_after >= 0,
            "new report batch must be an unattempted pending interval"
        );
        ensure!(
            batch.from
                >= self
                    .reported_cursors
                    .get(&batch.run_id)
                    .copied()
                    .unwrap_or(0),
            "report interval was already credited"
        );

        for existing in &self.batches {
            ensure!(
                existing.request.session_id != batch.request.session_id,
                "duplicate report session identity"
            );
            if existing.run_id == batch.run_id {
                ensure!(
                    existing.request.account_key == batch.request.account_key
                        && existing.request.book_id == batch.request.book_id
                        && existing.request.started_at == batch.request.started_at,
                    "report run cannot span accounts, books or start times"
                );
                ensure!(
                    batch.to <= existing.from || batch.from >= existing.to,
                    "report intervals overlap"
                );
            }
        }

        self.reported_cursors
            .entry(batch.run_id.clone())
            .or_default();
        self.batches.push(batch);
        Ok(())
    }

    #[cfg(test)]
    pub fn compacted_until(&self, run_id: &str) -> u64 {
        self.compacted_runs
            .get(run_id)
            .map_or(0, |prefix| prefix.confirmed_until)
    }

    pub fn allocation_cursor(&self, run_id: &str) -> u64 {
        self.batches
            .iter()
            .filter(|batch| batch.run_id == run_id)
            .map(|batch| batch.to)
            .max()
            .unwrap_or(0)
            .max(
                self.compacted_runs
                    .get(run_id)
                    .into_iter()
                    .flat_map(ConfirmedRun::intervals)
                    .map(|range| range.to)
                    .max()
                    .unwrap_or(0),
            )
    }

    pub fn ensure_run_identity(
        &self,
        run_id: &str,
        account_key: &str,
        book_id: &str,
        started_at: i64,
    ) -> Result<()> {
        if let Some(prefix) = self.compacted_runs.get(run_id) {
            ensure!(
                prefix.account_key == account_key
                    && prefix.book_id == book_id
                    && prefix.started_at == started_at,
                "compacted report run identity changed"
            );
        }
        for batch in self.batches.iter().filter(|batch| batch.run_id == run_id) {
            ensure!(
                batch.request.account_key == account_key
                    && batch.request.book_id == book_id
                    && batch.request.started_at == started_at,
                "report run identity changed"
            );
        }
        Ok(())
    }

    /// Return a validated replacement without changing the caller before commit.
    pub fn compacted(&self) -> Result<(Self, Vec<String>)> {
        self.admit_input(ReportInputPhase::Compaction, 0)?;
        self.validate()?;
        let preview = self.compaction_preview()?;
        let encoded =
            report_input_encoded_bytes(&preview, self.input_operation_limits().encoded_bytes)?;
        let mut cleanup_bytes = 0usize;
        let mut cleanup_keys = 0usize;
        for id in &preview.removed {
            for prefix in super::RESULT_PREFIXES {
                cleanup_keys = input_add(cleanup_keys, 1)?;
                cleanup_bytes = input_add(
                    cleanup_bytes,
                    input_add(input_add(prefix.len(), 1)?, id.len())?,
                )?;
            }
        }
        let cleanup_scratch = input_add(
            cleanup_bytes,
            input_mul(cleanup_keys, std::mem::size_of::<String>())?,
        )?;
        self.admit_input_with_work(
            ReportInputPhase::Compaction,
            input_add(encoded, cleanup_scratch)?,
            input_add(input_mul(cleanup_bytes, 3)?, input_mul(encoded, 3)?)?,
        )?;
        let (replacement, removed) = preview.into_owned()?;
        // A retained legacy session may alias a canonical namespace from another
        // run. Never weaken global session uniqueness while compressing proof.
        replacement.validate()?;
        Ok((replacement, removed))
    }

    pub fn encode_persistence(&self) -> Result<String> {
        self.admit_input(ReportInputPhase::Compaction, 0)?;
        let encoded =
            report_input_encoded_bytes(self, self.input_operation_limits().encoded_bytes)?;
        self.admit_input_with_work(
            ReportInputPhase::Compaction,
            encoded,
            input_mul(encoded, 3)?,
        )?;
        // Exact compact output was measured while borrowed. Reserve its actual
        // length once instead of letting a growing serializer double capacity.
        let mut bytes = Vec::with_capacity(encoded);
        serde_json::to_writer(&mut bytes, self)?;
        ensure!(
            bytes.len() == encoded,
            "report persistence encoding changed after admission"
        );
        observe_report_input(|stats| {
            stats.persistence_encodes += 1;
            stats.persistence_bytes += bytes.len();
        });
        Ok(String::from_utf8(bytes)?)
    }

    fn contains_compacted_session(&self, session_id: &str) -> bool {
        self.compacted_runs.iter().any(|(run_id, proof)| {
            let Some(namespace) = canonical_session_namespace(&proof.account_key, run_id) else {
                return false;
            };
            let Some(offsets) = session_id.strip_prefix(&namespace) else {
                return false;
            };
            let Some((from, to)) = offsets.split_once('-') else {
                return false;
            };
            let (Ok(from), Ok(to)) = (from.parse::<u64>(), to.parse::<u64>()) else {
                return false;
            };
            from < to
                && proof
                    .intervals()
                    .any(|range| range.from <= from && to <= range.to)
                && canonical_session_id(&proof.account_key, run_id, from, to).as_deref()
                    == Some(session_id)
        })
    }

    /// This mutation must be saved durably before any network side effect.
    pub fn mark_sending(&mut self, session_id: &str, now: i64) -> Result<()> {
        ensure!(
            !self
                .batches
                .iter()
                .any(|batch| batch.state == BatchState::Sending),
            "another report delivery is already in flight"
        );
        let batch = self.batch_mut(session_id)?;
        ensure!(
            batch.state == BatchState::Pending,
            "report batch is not pending"
        );
        ensure!(now >= batch.retry_after, "report retry is not due");
        ensure!(
            batch.attempts < MAX_SAFE_ATTEMPTS,
            "report retry budget is exhausted"
        );
        batch.state = BatchState::Sending;
        batch.attempted_at = Some(now);
        batch.attempts += 1;
        batch.retry_after = 0;
        batch.error = None;
        Ok(())
    }

    /// A late explicit acknowledgement may resolve an uncertain delivery.
    /// Neither a timer nor a local estimate is sufficient to call this method.
    pub fn confirm(&mut self, session_id: &str, receipt: &ReadReportReceipt) -> Result<()> {
        self.admit_input(ReportInputPhase::Cursor, 0)?;
        let batch = self.batch_mut(session_id)?;
        ensure!(
            receipt.seconds == batch.request.seconds,
            "report acknowledgement has a different elapsed count"
        );
        ensure!(
            matches!(
                batch.state,
                BatchState::Sending | BatchState::Uncertain | BatchState::Confirmed
            ),
            "report batch has no delivery to acknowledge"
        );
        batch.state = BatchState::Confirmed;
        batch.retry_after = 0;
        batch.error = None;
        let run_id = batch.run_id.clone();
        self.refresh_reported_cursor(&run_id);
        Ok(())
    }

    pub fn mark_uncertain(&mut self, session_id: &str, code: &str) -> Result<()> {
        let batch = self.batch_mut(session_id)?;
        ensure!(
            matches!(batch.state, BatchState::Sending | BatchState::Uncertain),
            "only an attempted report can have uncertain delivery"
        );
        batch.state = BatchState::Uncertain;
        batch.retry_after = 0;
        batch.error = Some(code.to_owned());
        Ok(())
    }

    /// Only call this when the transport proves no elapsed report was sent.
    /// A timeout, disconnected response, or ambiguous acknowledgement is unsafe.
    pub fn mark_safe_failure(
        &mut self,
        session_id: &str,
        code: &str,
        now: i64,
        retryable: bool,
    ) -> Result<()> {
        ensure!(now >= 0, "invalid report failure time");
        let batch = self.batch_mut(session_id)?;
        ensure!(
            batch.state == BatchState::Sending,
            "report batch is not sending"
        );
        batch.error = Some(code.to_owned());
        if retryable && batch.attempts < MAX_SAFE_ATTEMPTS {
            let exponent = batch.attempts.saturating_sub(1).min(30);
            let delay = INITIAL_RETRY_SECONDS
                .saturating_mul(1_i64 << exponent)
                .min(MAX_RETRY_SECONDS);
            batch.state = BatchState::Pending;
            batch.retry_after = now.saturating_add(delay);
        } else {
            batch.state = BatchState::Blocked;
            batch.retry_after = 0;
        }
        Ok(())
    }

    /// A restart cannot establish whether a previously started request arrived.
    /// Persist the recovered queue before scheduling another network operation.
    pub fn recover_sending(&mut self) -> usize {
        let mut recovered = 0;
        for batch in &mut self.batches {
            if batch.state == BatchState::Sending {
                batch.state = BatchState::Uncertain;
                batch.retry_after = 0;
                batch.error = Some("interrupted_delivery".into());
                recovered += 1;
            }
        }
        recovered
    }

    pub fn next_pending(&self, account_key: &str, now: i64) -> Option<String> {
        if self
            .batches
            .iter()
            .any(|batch| batch.state == BatchState::Sending)
        {
            return None;
        }
        self.batches
            .iter()
            .find(|batch| {
                batch.request.account_key == account_key
                    && batch.state == BatchState::Pending
                    && batch.attempts < MAX_SAFE_ATTEMPTS
                    && batch.retry_after <= now
            })
            .map(|batch| batch.request.session_id.clone())
    }

    fn batch_mut(&mut self, session_id: &str) -> Result<&mut ReportBatch> {
        match self
            .batches
            .iter_mut()
            .find(|batch| batch.request.session_id == session_id)
        {
            Some(batch) => Ok(batch),
            None => bail!("report batch does not exist"),
        }
    }

    fn refresh_reported_cursor(&mut self, run_id: &str) {
        let mut cursor = 0;
        for (from, to, confirmed) in self.run_intervals(run_id) {
            if from != cursor || !confirmed {
                break;
            }
            cursor = to;
        }
        self.reported_cursors.insert(run_id.to_owned(), cursor);
    }

    fn run_intervals(&self, run_id: &str) -> Vec<(u64, u64, bool)> {
        // validate and confirm admit the complete caller operation before this
        // numeric builder; never repeat full-input pricing inside each run.
        observe_report_input(|stats| stats.interval_builds += 1);
        let mut intervals = self
            .compacted_runs
            .get(run_id)
            .into_iter()
            .flat_map(ConfirmedRun::intervals)
            .map(|range| (range.from, range.to, true))
            .chain(
                self.batches
                    .iter()
                    .filter(|batch| batch.run_id == run_id)
                    .map(|batch| (batch.from, batch.to, batch.state == BatchState::Confirmed)),
            )
            .collect::<Vec<_>>();
        intervals.sort_unstable_by_key(|interval| interval.0);
        observe_report_input(|stats| stats.interval_items += intervals.len());
        intervals
    }
}

fn canonical_session_namespace(account_key: &str, run_id: &str) -> Option<String> {
    // Legacy settings may contain arbitrary Unicode identifiers. Splitting a
    // byte prefix must fail safely instead of panicking on a scalar boundary.
    let account = account_key.get(..account_key.len().min(12))?;
    Some(format!("{account}-{run_id}-"))
}

pub(super) fn canonical_session_id(
    account_key: &str,
    run_id: &str,
    from: u64,
    to: u64,
) -> Option<String> {
    Some(format!(
        "{}{from}-{to}",
        canonical_session_namespace(account_key, run_id)?
    ))
}

fn valid_identifier(value: &str, max_len: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_len && !value.chars().any(char::is_control)
}

fn validate_batch_shape(batch: &ReportBatch) -> Result<()> {
    ensure!(
        valid_identifier(&batch.request.session_id, 512),
        "invalid report session identity"
    );
    ensure!(
        valid_identifier(&batch.request.account_key, 512),
        "invalid report account identity"
    );
    ensure!(
        valid_identifier(&batch.request.book_id, 2048),
        "invalid report book identity"
    );
    ensure!(
        valid_identifier(&batch.run_id, 512),
        "invalid report run identity"
    );
    ensure!(
        batch.to > batch.from && batch.to - batch.from == u64::from(batch.request.seconds),
        "report interval must match positive elapsed seconds"
    );
    ensure!(batch.request.started_at > 0, "invalid report start time");
    ensure!(batch.retry_after >= 0, "invalid report retry time");
    ensure!(
        batch.attempts <= MAX_SAFE_ATTEMPTS,
        "report attempt count exceeds its budget"
    );
    ensure!(
        (batch.attempts == 0) == batch.attempted_at.is_none()
            && batch.attempted_at.is_none_or(|time| time >= 0),
        "report attempt count and timestamp are inconsistent"
    );
    match batch.state {
        BatchState::Pending => {
            ensure!(
                batch.attempts < MAX_SAFE_ATTEMPTS,
                "pending report has exhausted its budget"
            );
            ensure!(
                (batch.attempts == 0) == batch.error.is_none(),
                "pending report has inconsistent failure history"
            );
        }
        BatchState::Sending | BatchState::Confirmed => {
            ensure!(
                batch.attempts > 0 && batch.retry_after == 0 && batch.error.is_none(),
                "active or confirmed report has inconsistent attempt metadata"
            );
        }
        BatchState::Uncertain => {
            ensure!(
                batch.attempts > 0 && batch.retry_after == 0 && batch.error.is_some(),
                "uncertain report has inconsistent failure metadata"
            );
        }
        BatchState::Blocked => {
            ensure!(
                batch.retry_after == 0 && batch.error.is_some(),
                "blocked report has inconsistent failure metadata"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(id: &str, account: &str, run: &str, from: u64, to: u64) -> ReportBatch {
        ReportBatch {
            request: ReadReportRequest {
                session_id: id.into(),
                account_key: account.into(),
                book_id: "book-a".into(),
                started_at: 100,
                seconds: u32::try_from(to.saturating_sub(from)).unwrap(),
                position: Some(crate::source::ReadingPosition {
                    chapter_uid: "chapter-a".into(),
                    ..Default::default()
                }),
                progress_write_allowed: true,
                active_reader_intent: false,
            },
            run_id: run.into(),
            from,
            to,
            state: BatchState::Pending,
            attempted_at: None,
            retry_after: 0,
            attempts: 0,
            error: None,
        }
    }

    fn receipt(seconds: u32) -> ReadReportReceipt {
        ReadReportReceipt {
            accepted_at: 500,
            seconds,
            has_synckey: false,
        }
    }

    #[test]
    fn restarted_sending_becomes_uncertain_and_is_never_replayed() {
        let mut outbox = ReportOutbox::default();
        outbox
            .append(batch("first", "account-a", "run-a", 0, 60))
            .unwrap();
        outbox.mark_sending("first", 200).unwrap();

        let snapshot = serde_json::to_vec(&outbox).unwrap();
        let mut restored: ReportOutbox = serde_json::from_slice(&snapshot).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.recover_sending(), 1);
        assert_eq!(restored.recover_sending(), 0);
        restored.validate().unwrap();
        assert_eq!(restored.batches[0].state, BatchState::Uncertain);
        assert_eq!(restored.batches[0].attempted_at, Some(200));
        assert_eq!(restored.batches[0].attempts, 1);
        assert!(restored.next_pending("account-a", i64::MAX).is_none());
        assert!(restored.mark_sending("first", 300).is_err());
        assert_eq!(restored.reported_cursors["run-a"], 0);
    }

    #[test]
    fn selection_is_account_scoped_and_only_one_send_can_be_in_flight() {
        let mut outbox = ReportOutbox::default();
        outbox
            .append(batch("first", "account-a", "run-a", 0, 60))
            .unwrap();
        outbox
            .append(batch("second", "account-b", "run-b", 0, 30))
            .unwrap();
        assert_eq!(
            outbox.next_pending("account-a", 200).as_deref(),
            Some("first")
        );
        assert_eq!(
            outbox.next_pending("account-b", 200).as_deref(),
            Some("second")
        );
        assert!(outbox.next_pending("unknown", 200).is_none());

        outbox.mark_sending("first", 200).unwrap();
        assert!(outbox.next_pending("account-a", 200).is_none());
        assert!(outbox.next_pending("account-b", 200).is_none());
        assert!(outbox.mark_sending("second", 200).is_err());
        outbox
            .mark_safe_failure("first", "permission_required", 200, false)
            .unwrap();
        assert_eq!(
            outbox.next_pending("account-b", 200).as_deref(),
            Some("second")
        );
    }

    #[test]
    fn duplicates_overlaps_invalid_ranges_and_mixed_run_identities_are_rejected() {
        let mut outbox = ReportOutbox::default();
        outbox
            .append(batch("first", "account-a", "run-a", 0, 60))
            .unwrap();
        assert!(
            outbox
                .append(batch("first", "account-b", "run-b", 0, 60))
                .is_err()
        );
        assert!(
            outbox
                .append(batch("overlap", "account-a", "run-a", 30, 90))
                .is_err()
        );
        assert!(
            outbox
                .append(batch("contained", "account-a", "run-a", 10, 20))
                .is_err()
        );
        assert!(
            outbox
                .append(batch("empty", "account-a", "run-a", 60, 60))
                .is_err()
        );
        assert!(
            outbox
                .append(batch("reversed", "account-a", "run-a", 70, 60))
                .is_err()
        );
        assert!(
            outbox
                .append(batch("mixed-account", "account-b", "run-a", 60, 120))
                .is_err()
        );
        let mut mixed_book = batch("mixed-book", "account-a", "run-a", 60, 120);
        mixed_book.request.book_id = "book-b".into();
        assert!(outbox.append(mixed_book).is_err());
        let mut wrong_seconds = batch("wrong-seconds", "account-a", "run-a", 60, 120);
        wrong_seconds.request.seconds = 59;
        assert!(outbox.append(wrong_seconds).is_err());
        outbox
            .append(batch("adjacent", "account-a", "run-a", 60, 120))
            .unwrap();
        outbox
            .append(batch("other-run", "account-b", "run-b", 0, 60))
            .unwrap();
        assert_eq!(outbox.batches.len(), 3);
    }

    #[test]
    fn only_a_matching_receipt_can_confirm_an_attempted_interval() {
        let mut outbox = ReportOutbox::default();
        outbox
            .append(batch("first", "account-a", "run-a", 0, 60))
            .unwrap();
        assert!(outbox.confirm("first", &receipt(60)).is_err());
        outbox.mark_sending("first", 200).unwrap();
        assert!(outbox.confirm("first", &receipt(59)).is_err());
        assert_eq!(outbox.batches[0].state, BatchState::Sending);
        assert_eq!(outbox.reported_cursors["run-a"], 0);
        outbox.confirm("first", &receipt(60)).unwrap();
        assert_eq!(outbox.batches[0].state, BatchState::Confirmed);
        assert_eq!(outbox.reported_cursors["run-a"], 60);
        assert!(outbox.next_pending("account-a", 500).is_none());
        assert!(
            outbox
                .append(batch("credited", "account-a", "run-a", 0, 60))
                .is_err()
        );
    }

    #[test]
    fn an_uncertain_gap_blocks_cursor_credit_until_an_explicit_late_receipt() {
        let mut outbox = ReportOutbox::default();
        for (id, from, to) in [("first", 0, 60), ("gap", 60, 120), ("last", 120, 180)] {
            outbox
                .append(batch(id, "account-a", "run-a", from, to))
                .unwrap();
        }
        outbox.mark_sending("first", 200).unwrap();
        outbox.confirm("first", &receipt(60)).unwrap();
        outbox.mark_sending("gap", 200).unwrap();
        outbox.mark_uncertain("gap", "delivery_uncertain").unwrap();
        assert_eq!(
            outbox.next_pending("account-a", 200).as_deref(),
            Some("last")
        );
        outbox.mark_sending("last", 200).unwrap();
        outbox.confirm("last", &receipt(60)).unwrap();
        assert_eq!(outbox.reported_cursors["run-a"], 60);
        outbox.validate().unwrap();
        assert!(outbox.next_pending("account-a", 500).is_none());
        outbox.confirm("gap", &receipt(60)).unwrap();
        assert_eq!(outbox.reported_cursors["run-a"], 180);
        outbox.validate().unwrap();
    }

    #[test]
    fn safe_failures_have_a_bounded_backoff_and_retry_budget() {
        let mut outbox = ReportOutbox::default();
        outbox
            .append(batch("first", "account-a", "run-a", 0, 60))
            .unwrap();
        let mut now = 200;
        for attempt in 1..=MAX_SAFE_ATTEMPTS {
            outbox.mark_sending("first", now).unwrap();
            outbox
                .mark_safe_failure("first", "connect_failed", now, true)
                .unwrap();
            outbox.validate().unwrap();
            let stored = &outbox.batches[0];
            assert_eq!(stored.attempts, attempt);
            if attempt < MAX_SAFE_ATTEMPTS {
                let retry_after = stored.retry_after;
                assert_eq!(stored.state, BatchState::Pending);
                assert!((INITIAL_RETRY_SECONDS..=MAX_RETRY_SECONDS).contains(&(retry_after - now)));
                assert!(outbox.next_pending("account-a", retry_after - 1).is_none());
                assert!(outbox.mark_sending("first", retry_after - 1).is_err());
                assert_eq!(
                    outbox.next_pending("account-a", retry_after).as_deref(),
                    Some("first")
                );
                now = retry_after;
            } else {
                assert_eq!(stored.state, BatchState::Blocked);
                assert_eq!(stored.retry_after, 0);
                assert!(outbox.next_pending("account-a", i64::MAX).is_none());
            }
        }
        assert_eq!(outbox.reported_cursors["run-a"], 0);
    }

    #[test]
    fn missing_durable_history_fields_are_rejected_instead_of_defaulted() {
        for snapshot in [r#"{}"#, r#"{"batches":[]}"#, r#"{"reported_cursors":{}}"#] {
            assert!(serde_json::from_str::<ReportOutbox>(snapshot).is_err());
        }
        let mut outbox = ReportOutbox::default();
        outbox
            .append(batch("first", "account-a", "run-a", 0, 60))
            .unwrap();
        let mut snapshot = serde_json::to_value(&outbox).unwrap();
        snapshot["batches_v2"][0]
            .as_object_mut()
            .unwrap()
            .remove("attempts");
        assert!(serde_json::from_value::<ReportOutbox>(snapshot).is_err());
        let empty_snapshot = serde_json::to_vec(&ReportOutbox::default()).unwrap();
        serde_json::from_slice::<ReportOutbox>(&empty_snapshot)
            .unwrap()
            .validate()
            .unwrap();
    }

    #[test]
    fn forged_missing_or_orphan_cursors_are_rejected() {
        let mut outbox = ReportOutbox::default();
        outbox
            .append(batch("first", "account-a", "run-a", 0, 60))
            .unwrap();
        outbox.mark_sending("first", 200).unwrap();
        outbox.confirm("first", &receipt(60)).unwrap();
        outbox.validate().unwrap();

        let mut damaged = outbox.clone();
        damaged.reported_cursors.insert("run-a".into(), 0);
        assert!(damaged.validate().is_err());
        damaged.reported_cursors.insert("run-a".into(), 120);
        assert!(damaged.validate().is_err());
        damaged.reported_cursors.clear();
        assert!(damaged.validate().is_err());
        damaged.reported_cursors = outbox.reported_cursors.clone();
        damaged.reported_cursors.insert("orphan".into(), 0);
        assert!(damaged.validate().is_err());

        outbox
            .append(batch("gap", "account-a", "run-a", 60, 120))
            .unwrap();
        outbox.mark_sending("gap", 200).unwrap();
        outbox.mark_uncertain("gap", "delivery_uncertain").unwrap();
        outbox.validate().unwrap();
        outbox.reported_cursors.insert("run-a".into(), 120);
        assert!(outbox.validate().is_err());
    }

    #[test]
    fn loaded_duplicate_overlapping_or_mixed_runs_are_rejected() {
        let mut outbox = ReportOutbox::default();
        outbox
            .append(batch("first", "account-a", "run-a", 0, 60))
            .unwrap();
        outbox
            .append(batch("second", "account-a", "run-a", 60, 120))
            .unwrap();
        outbox.validate().unwrap();

        let mut damaged = outbox.clone();
        damaged.batches[1].request.session_id = "first".into();
        assert!(damaged.validate().is_err());
        damaged = outbox.clone();
        damaged.batches[1].from = 30;
        damaged.batches[1].to = 90;
        assert!(damaged.validate().is_err());
        damaged = outbox.clone();
        damaged.batches[1].request.account_key = "account-b".into();
        assert!(damaged.validate().is_err());
        damaged = outbox.clone();
        damaged.batches[1].request.book_id = "book-b".into();
        assert!(damaged.validate().is_err());
        damaged = outbox.clone();
        damaged.batches[1].request.seconds = 59;
        assert!(damaged.validate().is_err());
        damaged = outbox.clone();
        damaged.batches[1].run_id = " ".into();
        assert!(damaged.validate().is_err());
    }

    #[test]
    fn loaded_multiple_sends_and_invalid_attempt_metadata_are_rejected() {
        let mut outbox = ReportOutbox::default();
        outbox
            .append(batch("first", "account-a", "run-a", 0, 60))
            .unwrap();
        outbox
            .append(batch("second", "account-b", "run-b", 0, 30))
            .unwrap();
        outbox.mark_sending("first", 200).unwrap();
        outbox.validate().unwrap();

        let mut damaged = outbox.clone();
        damaged.batches[1].state = BatchState::Sending;
        damaged.batches[1].attempts = 1;
        damaged.batches[1].attempted_at = Some(200);
        assert!(damaged.validate().is_err());
        damaged = outbox.clone();
        damaged.batches[0].attempts = MAX_SAFE_ATTEMPTS + 1;
        assert!(damaged.validate().is_err());
        damaged = outbox.clone();
        damaged.batches[0].attempted_at = None;
        assert!(damaged.validate().is_err());
        damaged = outbox.clone();
        damaged.batches[0].retry_after = 1;
        assert!(damaged.validate().is_err());
        damaged = outbox.clone();
        damaged.batches[1].attempts = MAX_SAFE_ATTEMPTS;
        damaged.batches[1].attempted_at = Some(200);
        damaged.batches[1].error = Some("connect_failed".into());
        assert!(damaged.validate().is_err());
        assert!(
            outbox
                .mark_safe_failure("first", "connect_failed", -10, true)
                .is_err()
        );
        outbox.validate().unwrap();
    }

    #[test]
    fn unattributed_history_remains_distinct_from_known_accounts() {
        let mut outbox = ReportOutbox::default();
        outbox
            .append(batch("first", "unattributed", "run-a", 0, 60))
            .unwrap();
        outbox.validate().unwrap();
        assert!(outbox.next_pending("account-a", i64::MAX).is_none());
    }

    #[test]
    fn preflight_block_preserves_zero_attempts_but_uncertainty_requires_a_delivery() {
        let mut outbox = ReportOutbox::default();
        outbox
            .append(batch("first", "account-a", "run-a", 0, 60))
            .unwrap();
        outbox.batches[0].state = BatchState::Blocked;
        outbox.batches[0].error = Some("read_position_required".into());
        outbox.validate().unwrap();
        assert_eq!(outbox.batches[0].attempts, 0);
        assert_eq!(outbox.batches[0].attempted_at, None);
        assert!(outbox.next_pending("account-a", i64::MAX).is_none());
        assert_eq!(outbox.reported_cursors["run-a"], 0);

        let mut uncertain = outbox.clone();
        uncertain.batches[0].state = BatchState::Uncertain;
        assert!(uncertain.validate().is_err());
        let mut fabricated_attempt = outbox;
        fabricated_attempt.batches[0].attempted_at = Some(200);
        assert!(fabricated_attempt.validate().is_err());
    }

    fn canonical_batch(account: &str, run: &str, from: u64, to: u64) -> ReportBatch {
        batch(
            &canonical_session_id(account, run, from, to).unwrap(),
            account,
            run,
            from,
            to,
        )
    }

    fn confirm_batch(outbox: &mut ReportOutbox, id: &str, seconds: u32) {
        outbox.mark_sending(id, 200).unwrap();
        outbox.confirm(id, &receipt(seconds)).unwrap();
    }

    #[test]
    fn long_confirmed_run_retains_one_proof_instead_of_one_record_per_minute() {
        let mut outbox = ReportOutbox::default();
        for minute in 0..2_048 {
            let from = minute * 60;
            let next = canonical_batch("account-a", "run-a", from, from + 60);
            let id = next.request.session_id.clone();
            outbox.append(next).unwrap();
            confirm_batch(&mut outbox, &id, 60);
            let (replacement, removed) = outbox.compacted().unwrap();
            assert_eq!(removed, vec![id]);
            outbox = replacement;
            assert!(outbox.batches.is_empty());
            assert_eq!(outbox.compacted_runs.len(), 1);
            assert_eq!(outbox.allocation_cursor("run-a"), from + 60);
            assert_eq!(outbox.reported_cursors["run-a"], from + 60);
            assert!(serde_json::to_vec(&outbox).unwrap().len() < 1_024);
        }
    }

    #[test]
    fn every_unresolved_state_preserves_the_gap_while_confirmed_suffixes_compact() {
        for state in [
            BatchState::Pending,
            BatchState::Sending,
            BatchState::Uncertain,
            BatchState::Blocked,
        ] {
            let mut outbox = ReportOutbox::default();
            let mut ids = Vec::new();
            for from in [0, 60, 120] {
                let next = canonical_batch("account-a", "run-a", from, from + 60);
                ids.push(next.request.session_id.clone());
                outbox.append(next).unwrap();
            }
            confirm_batch(&mut outbox, &ids[0], 60);
            confirm_batch(&mut outbox, &ids[2], 60);
            if state != BatchState::Pending {
                outbox.mark_sending(&ids[1], 200).unwrap();
            }
            match state {
                BatchState::Uncertain => {
                    outbox.mark_uncertain(&ids[1], "fixture_uncertain").unwrap();
                }
                BatchState::Blocked => {
                    outbox
                        .mark_safe_failure(&ids[1], "fixture_blocked", 200, false)
                        .unwrap();
                }
                _ => {}
            }
            let (mut replacement, removed) = outbox.compacted().unwrap();
            assert_eq!(removed.len(), 2);
            assert!(removed.contains(&ids[0]));
            assert!(removed.contains(&ids[2]));
            assert_eq!(replacement.batches.len(), 1);
            assert_eq!(replacement.batches[0].state, state);
            assert_eq!(replacement.compacted_until("run-a"), 60);
            assert_eq!(
                replacement.compacted_runs["run-a"].confirmed_ranges,
                vec![ConfirmedRange { from: 120, to: 180 }]
            );
            assert_eq!(replacement.reported_cursors["run-a"], 60);
            if state == BatchState::Uncertain {
                replacement.confirm(&ids[1], &receipt(60)).unwrap();
                assert_eq!(replacement.reported_cursors["run-a"], 180);
                let (finished, removed) = replacement.compacted().unwrap();
                assert_eq!(removed, vec![ids[1].clone()]);
                assert!(finished.batches.is_empty());
                assert_eq!(finished.compacted_until("run-a"), 180);
                assert!(finished.compacted_runs["run-a"].confirmed_ranges.is_empty());
            }
        }
    }

    #[test]
    fn a_missing_interval_preserves_the_cursor_while_confirmed_suffixes_compact() {
        let mut outbox = ReportOutbox::default();
        for from in [0, 120] {
            let next = canonical_batch("account-a", "run-a", from, from + 60);
            let id = next.request.session_id.clone();
            outbox.append(next).unwrap();
            confirm_batch(&mut outbox, &id, 60);
        }
        let (replacement, removed) = outbox.compacted().unwrap();
        assert_eq!(removed.len(), 2);
        assert_eq!(replacement.compacted_until("run-a"), 60);
        assert!(replacement.batches.is_empty());
        assert_eq!(
            replacement.compacted_runs["run-a"].confirmed_ranges,
            vec![ConfirmedRange { from: 120, to: 180 }]
        );
        assert_eq!(replacement.allocation_cursor("run-a"), 180);
        assert_eq!(replacement.reported_cursors["run-a"], 60);
    }

    #[test]
    fn compacted_session_identity_cannot_be_reused_in_another_range_or_run() {
        let mut outbox = ReportOutbox::default();
        let first = canonical_batch("account-a", "run-a", 0, 60);
        let old_id = first.request.session_id.clone();
        outbox.append(first).unwrap();
        confirm_batch(&mut outbox, &old_id, 60);
        let (mut compacted, _) = outbox.compacted().unwrap();
        for (account, run, from) in [
            ("account-a", "run-a", 60),
            ("account-a", "run-b", 0),
            ("account-b", "run-b", 0),
        ] {
            assert!(
                compacted
                    .append(batch(&old_id, account, run, from, from + 60))
                    .is_err()
            );
        }
        assert!(compacted.confirm(&old_id, &receipt(60)).is_err());
        let mut forged = compacted.clone();
        forged
            .batches
            .push(batch(&old_id, "account-b", "run-b", 0, 60));
        forged.reported_cursors.insert("run-b".into(), 0);
        assert!(forged.validate().is_err());
        compacted
            .append(canonical_batch("account-a", "run-a", 60, 120))
            .unwrap();
        compacted.validate().unwrap();
    }

    #[test]
    fn compacted_runs_keep_full_identity_and_reject_missing_or_forged_cursors() {
        let account = "same-prefix-account-a";
        let mut outbox = ReportOutbox::default();
        let first = canonical_batch(account, "run-a", 0, 60);
        let id = first.request.session_id.clone();
        outbox.append(first).unwrap();
        confirm_batch(&mut outbox, &id, 60);
        let (compacted, _) = outbox.compacted().unwrap();
        let mut changed = compacted.clone();
        assert!(
            changed
                .append(canonical_batch("same-prefix-account-b", "run-a", 60, 120))
                .is_err()
        );
        let mut next = canonical_batch(account, "run-a", 60, 120);
        next.request.book_id = "other-book".into();
        assert!(changed.append(next).is_err());
        let mut next = canonical_batch(account, "run-a", 60, 120);
        next.request.started_at += 1;
        assert!(changed.append(next).is_err());
        changed.compacted_runs.clear();
        assert!(changed.validate().is_err());
        let mut changed = compacted.clone();
        changed
            .compacted_runs
            .get_mut("run-a")
            .unwrap()
            .confirmed_until += 1;
        assert!(changed.validate().is_err());
        let mut changed = compacted;
        changed.reported_cursors.clear();
        assert!(changed.validate().is_err());
    }

    #[test]
    fn legacy_ids_and_unicode_accounts_are_retained_without_panicking() {
        for account in ["account-a", "legacy-id-\u{1f6d1}-account"] {
            let mut outbox = ReportOutbox::default();
            outbox
                .append(batch("legacy-session", account, "legacy-run", 0, 60))
                .unwrap();
            confirm_batch(&mut outbox, "legacy-session", 60);
            let mut legacy = serde_json::to_value(&outbox).unwrap();
            let fields = legacy.as_object_mut().unwrap();
            fields.remove("compacted_runs");
            let batches = fields.remove("batches_v2").unwrap();
            fields.insert("batches".into(), batches);
            let restored: ReportOutbox = serde_json::from_value(legacy).unwrap();
            restored.validate().unwrap();
            let (replacement, removed) = restored.compacted().unwrap();
            assert!(removed.is_empty());
            assert_eq!(replacement.batches.len(), 1);
            assert!(replacement.compacted_runs.is_empty());
        }
        assert!(canonical_session_id("legacy-id-\u{1f6d1}-account", "run", 0, 60).is_none());
    }

    #[test]
    fn compaction_refuses_a_legacy_alias_of_its_canonical_namespace() {
        let mut outbox = ReportOutbox::default();
        let first = canonical_batch("account-a", "run-a", 0, 60);
        let id = first.request.session_id.clone();
        outbox.append(first).unwrap();
        confirm_batch(&mut outbox, &id, 60);
        let alias = canonical_session_id("account-a", "run-a", 10, 20).unwrap();
        outbox
            .append(batch(&alias, "account-b", "run-b", 0, 10))
            .unwrap();
        let before = serde_json::to_value(&outbox).unwrap();
        assert!(outbox.compacted().is_err());
        assert_eq!(serde_json::to_value(&outbox).unwrap(), before);
    }

    #[test]
    fn confirmed_ranges_preserve_holes_and_reject_numeric_or_session_replays() {
        let mut outbox = ReportOutbox::default();
        let mut ids = Vec::new();
        for from in [0, 60, 120, 180] {
            let next = canonical_batch("account-a", "run-a", from, from + 60);
            ids.push(next.request.session_id.clone());
            outbox.append(next).unwrap();
        }
        outbox.mark_sending(&ids[0], 200).unwrap();
        outbox.mark_uncertain(&ids[0], "fixture_uncertain").unwrap();
        confirm_batch(&mut outbox, &ids[1], 60);
        confirm_batch(&mut outbox, &ids[3], 60);
        let (mut compacted, removed) = outbox.compacted().unwrap();
        assert_eq!(removed.len(), 2);
        assert_eq!(compacted.batches.len(), 2);
        assert_eq!(compacted.reported_cursors["run-a"], 0);
        assert_eq!(compacted.allocation_cursor("run-a"), 240);
        assert_eq!(
            compacted.compacted_runs["run-a"].confirmed_ranges,
            vec![
                ConfirmedRange { from: 60, to: 120 },
                ConfirmedRange { from: 180, to: 240 }
            ]
        );
        for (from, to) in [(90, 100), (50, 70), (110, 190)] {
            assert!(
                compacted
                    .append(batch("new-session", "account-a", "run-a", from, to))
                    .is_err()
            );
        }
        assert!(!compacted.contains_compacted_session(&ids[0]));
        assert!(!compacted.contains_compacted_session(&ids[2]));
        assert!(!compacted.contains_compacted_session(
            &canonical_session_id("account-a", "run-a", 60, 240).unwrap()
        ));
        for (account, run) in [("account-a", "run-a"), ("account-b", "run-b")] {
            assert!(
                compacted
                    .append(batch(&ids[1], account, run, 240, 300))
                    .is_err()
            );
        }
        let mut forged = compacted.clone();
        forged
            .batches
            .push(batch("forged-session", "account-a", "run-a", 90, 100));
        assert!(forged.validate().is_err());

        confirm_batch(&mut compacted, &ids[2], 60);
        assert_eq!(compacted.reported_cursors["run-a"], 0);
        let (mut compacted, _) = compacted.compacted().unwrap();
        assert_eq!(
            compacted.compacted_runs["run-a"].confirmed_ranges,
            vec![ConfirmedRange { from: 60, to: 240 }]
        );
        compacted.confirm(&ids[0], &receipt(60)).unwrap();
        assert_eq!(compacted.reported_cursors["run-a"], 240);
        let (finished, _) = compacted.compacted().unwrap();
        assert!(finished.batches.is_empty());
        assert_eq!(finished.compacted_until("run-a"), 240);
        assert!(finished.compacted_runs["run-a"].confirmed_ranges.is_empty());
    }

    #[test]
    fn malformed_persisted_confirmed_ranges_are_rejected_before_compaction() {
        let mut outbox = ReportOutbox::default();
        for from in [0, 120, 240] {
            let next = canonical_batch("account-a", "run-a", from, from + 60);
            let id = next.request.session_id.clone();
            outbox.append(next).unwrap();
            confirm_batch(&mut outbox, &id, 60);
        }
        let (valid, _) = outbox.compacted().unwrap();
        assert_eq!(valid.compacted_runs["run-a"].confirmed_ranges.len(), 2);
        for variant in 0..7 {
            let mut damaged = valid.clone();
            let proof = damaged.compacted_runs.get_mut("run-a").unwrap();
            match variant {
                0 => proof.confirmed_ranges[0].from = proof.confirmed_until,
                1 => proof.confirmed_ranges[0].to = proof.confirmed_ranges[0].from,
                2 => proof.confirmed_ranges[1].from = proof.confirmed_ranges[0].to,
                3 => proof.confirmed_ranges[1].from = proof.confirmed_ranges[0].to - 1,
                4 => proof.confirmed_ranges.reverse(),
                5 => {
                    proof.confirmed_until = 0;
                    proof.confirmed_ranges.clear();
                }
                6 => {
                    proof.confirmed_until = 0;
                    proof.confirmed_ranges = vec![ConfirmedRange { from: 0, to: 60 }];
                }
                _ => unreachable!(),
            }
            let before = serde_json::to_value(&damaged).unwrap();
            assert!(
                damaged.validate().is_err(),
                "Malformed range variant {variant} passed"
            );
            assert!(damaged.compacted().is_err());
            assert_eq!(serde_json::to_value(&damaged).unwrap(), before);
        }
    }

    #[test]
    fn original_prefix_only_checkpoint_snapshots_remain_readable() {
        let mut outbox = ReportOutbox::default();
        let first = canonical_batch("account-a", "run-a", 0, 60);
        let id = first.request.session_id.clone();
        outbox.append(first).unwrap();
        confirm_batch(&mut outbox, &id, 60);
        let (compacted, _) = outbox.compacted().unwrap();
        let mut legacy = serde_json::to_value(&compacted).unwrap();
        legacy["compacted_runs"]["run-a"]
            .as_object_mut()
            .unwrap()
            .remove("confirmed_ranges");
        let fields = legacy.as_object_mut().unwrap();
        let batches = fields.remove("batches_v2").unwrap();
        fields.insert("batches".into(), batches);
        let restored: ReportOutbox = serde_json::from_value(legacy).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.compacted_until("run-a"), 60);
        assert_eq!(restored.allocation_cursor("run-a"), 60);
        assert!(restored.compacted_runs["run-a"].confirmed_ranges.is_empty());
    }

    #[test]
    fn a_retained_legacy_bridge_advances_the_cursor_without_losing_its_proof() {
        let mut outbox = ReportOutbox::default();
        outbox
            .append(batch("legacy-prefix", "account-a", "run-a", 0, 60))
            .unwrap();
        confirm_batch(&mut outbox, "legacy-prefix", 60);
        for from in [60, 120] {
            let next = canonical_batch("account-a", "run-a", from, from + 60);
            let id = next.request.session_id.clone();
            outbox.append(next).unwrap();
            confirm_batch(&mut outbox, &id, 60);
        }
        let (compacted, removed) = outbox.compacted().unwrap();
        assert_eq!(removed.len(), 2);
        assert_eq!(compacted.batches.len(), 1);
        assert_eq!(compacted.batches[0].request.session_id, "legacy-prefix");
        assert_eq!(compacted.batches[0].state, BatchState::Confirmed);
        assert_eq!(compacted.reported_cursors["run-a"], 180);
        assert_eq!(compacted.compacted_until("run-a"), 0);
        assert_eq!(
            compacted.compacted_runs["run-a"].confirmed_ranges,
            vec![ConfirmedRange { from: 60, to: 180 }]
        );
        compacted.validate().unwrap();
    }

    #[test]
    fn old_readers_reject_range_snapshots_and_new_readers_accept_real_legacy_batches() {
        #[derive(Deserialize)]
        struct LegacyOutbox {
            #[serde(rename = "batches")]
            _batches: Vec<ReportBatch>,
            #[serde(rename = "reported_cursors")]
            _reported_cursors: BTreeMap<String, u64>,
        }

        let mut outbox = ReportOutbox::default();
        let gap = canonical_batch("account-a", "run-a", 0, 60);
        let gap_id = gap.request.session_id.clone();
        outbox.append(gap).unwrap();
        outbox.mark_sending(&gap_id, 200).unwrap();
        outbox.mark_uncertain(&gap_id, "fixture_uncertain").unwrap();
        let suffix = canonical_batch("account-a", "run-a", 60, 120);
        let suffix_id = suffix.request.session_id.clone();
        outbox.append(suffix).unwrap();
        confirm_batch(&mut outbox, &suffix_id, 60);
        let mut legacy = serde_json::to_value(&outbox).unwrap();
        let fields = legacy.as_object_mut().unwrap();
        fields.remove("compacted_runs");
        let batches = fields.remove("batches_v2").unwrap();
        fields.insert("batches".into(), batches);
        assert!(serde_json::from_value::<LegacyOutbox>(legacy.clone()).is_ok());
        let restored: ReportOutbox = serde_json::from_value(legacy).unwrap();
        restored.validate().unwrap();
        let (compacted, _) = restored.compacted().unwrap();
        assert_eq!(compacted.reported_cursors["run-a"], 0);
        assert_eq!(compacted.allocation_cursor("run-a"), 120);
        let new_snapshot = serde_json::to_value(&compacted).unwrap();
        assert!(new_snapshot.get("batches").is_none());
        assert!(new_snapshot.get("batches_v2").is_some());
        assert!(serde_json::from_value::<LegacyOutbox>(new_snapshot.clone()).is_err());
        let mut missing_proof = new_snapshot.clone();
        missing_proof
            .as_object_mut()
            .unwrap()
            .remove("compacted_runs");
        assert!(serde_json::from_value::<ReportOutbox>(missing_proof).is_err());
        let mut null_proof = new_snapshot.clone();
        null_proof["compacted_runs"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<ReportOutbox>(null_proof).is_err());
        let mut missing_ranges = new_snapshot.clone();
        missing_ranges["compacted_runs"]["run-a"]
            .as_object_mut()
            .unwrap()
            .remove("confirmed_ranges");
        assert!(serde_json::from_value::<ReportOutbox>(missing_ranges).is_err());
        let mut null_ranges = new_snapshot.clone();
        null_ranges["compacted_runs"]["run-a"]["confirmed_ranges"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<ReportOutbox>(null_ranges).is_err());
        let mut ambiguous = new_snapshot;
        ambiguous
            .as_object_mut()
            .unwrap()
            .insert("batches".into(), serde_json::json!([]));
        assert!(serde_json::from_value::<ReportOutbox>(ambiguous).is_err());
    }
}
