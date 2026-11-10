//! Durable accounting for acknowledged, non-idempotent reading-time delivery.

mod outbox;

#[cfg(test)]
mod active_budget_tests;

use super::*;
use anyhow::{bail, ensure};
pub(super) use outbox::{AllocationCapacity, AllocationLimits};
use outbox::{
    AllocationExtra, AllocationInput, BatchState, ReportInputPhase, ReportOutbox,
    allocation_encoded_bytes, allocation_limits, observe_report_input, report_input_limits,
    report_settlement_limits,
};
#[cfg(test)]
pub(super) use outbox::{AllocationStats, allocation_test_stats, with_allocation_test_limits};
#[cfg(test)]
use outbox::{ReportBatch, canonical_session_id};
pub(super) use outbox::{ReportInputCapacity, ReportInputLimits};
#[cfg(test)]
pub(super) use outbox::{ReportInputStats, report_input_test_stats, with_report_input_test_limits};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use source::report::{ReadReportFailure, ReadReportReceipt, ReadReportRequest};

const OUTBOX_KEY: &str = "report.outbox.v1";
const ACTIVE_KEY: &str = "report.active.v1";
const SCHEDULE_KEY: &str = "report.next_check.v1";
const STORAGE_ERROR_KEY: &str = "report.storage_error.v1";
const ALLOCATION_DEBT_PREFIX: &str = "report.allocation_debt.v1:";
const ALLOCATION_DEBT_CURSOR_KEY: &str = "report.allocation_debt_cursor.v1";
const UNATTRIBUTED_ACCOUNT: &str = "unattributed";
const BATCH_SECONDS: u64 = 60;
pub(super) const REPORT_SUMMARY_SCALARS: usize = 20;
const DELIVERY_TIMEOUT_SECONDS: i64 = 180;
const RESULT_RECORD_BYTES: usize = 32 * 1024;
// A legacy failure string can overlap the SQL text, decoder scratch, selected
// owned value, and replacement code. Price that whole bounded read phase.
const RESULT_READ_SCRATCH: usize = 4 * RESULT_RECORD_BYTES;
const RESULT_PREFIXES: [&str; 7] = [
    "report.receipt",
    "report.error",
    "report.failure",
    "report.retryable",
    "report.requires_login",
    "report.await_auth",
    "report.uncertain",
];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ActiveRun {
    id: String,
    account_key: String,
    book_id: String,
    started_at: i64,
    observed_seconds: u64,
    eligible_seconds: u64,
    allocated_seconds: u64,
    last_checkpoint_at: i64,
    local_chapter: u32,
    local_anchor: u32,
    position: Option<source::ReadingPosition>,
    /// An account transition can split report ownership without resetting the Reader.
    observer_started_at: i64,
    observer_seconds_base: u64,
    /// Local statistics still use one ID for the original Reader observation.
    local_session_id: String,
}

impl ActiveRun {
    fn reader_started_at(&self) -> i64 {
        if self.observer_started_at == 0 {
            self.started_at
        } else {
            self.observer_started_at
        }
    }
    fn local_session_id(&self) -> &str {
        if self.local_session_id.is_empty() {
            &self.id
        } else {
            &self.local_session_id
        }
    }
    fn belongs_to_reader(&self, app: &App) -> bool {
        self.book_id == app.reader.book_id
            && Some(self.reader_started_at()) == app.reader.session_started
    }
}

/// One closed real Reader run, not one object per elapsed minute.
#[derive(Serialize, Deserialize)]
struct DeferredAllocation {
    run: ActiveRun,
}

/// A read projection cannot be persisted as a complete report run.
#[derive(Deserialize)]
struct ActiveHeader {
    id: String,
    account_key: String,
    book_id: String,
    started_at: i64,
    observed_seconds: u64,
    eligible_seconds: u64,
    allocated_seconds: u64,
    last_checkpoint_at: i64,
    local_chapter: u32,
    local_anchor: u32,
    observer_started_at: i64,
    observer_seconds_base: u64,
    local_session_id: String,
}

#[derive(Deserialize)]
struct DeferredHeader {
    run: ActiveHeader,
}

impl ActiveHeader {
    fn reader_started_at(&self) -> i64 {
        if self.observer_started_at == 0 {
            self.started_at
        } else {
            self.observer_started_at
        }
    }
    fn local_session_id(&self) -> &str {
        if self.local_session_id.is_empty() {
            &self.id
        } else {
            &self.local_session_id
        }
    }
    fn belongs_to_reader(&self, app: &App) -> bool {
        self.book_id == app.reader.book_id
            && Some(self.reader_started_at()) == app.reader.session_started
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.id.is_empty() && self.id.len() <= 256,
            "invalid active reading identity"
        );
        ensure!(
            !self.book_id.is_empty() && self.book_id.len() <= 2048,
            "invalid active reading book"
        );
        ensure!(
            self.account_key == UNATTRIBUTED_ACCOUNT
                || self.account_key.len() == 64
                    && self
                        .account_key
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit()),
            "invalid active reading account"
        );
        ensure!(
            self.started_at > 0 && self.last_checkpoint_at >= self.started_at,
            "invalid active reading timestamps"
        );
        ensure!(
            self.reader_started_at() > 0
                && self.reader_started_at() <= self.started_at
                && self.observer_seconds_base
                    <= self
                        .last_checkpoint_at
                        .saturating_sub(self.reader_started_at()) as u64
                && (self.local_session_id.is_empty() || self.local_session_id.len() <= 256),
            "invalid active reading observation baseline"
        );
        ensure!(
            self.observed_seconds <= self.last_checkpoint_at.saturating_sub(self.started_at) as u64
                && self.eligible_seconds <= self.observed_seconds
                && self.allocated_seconds <= self.eligible_seconds,
            "active reading counter exceeds measured elapsed time"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct ActiveInputLimits {
    encoded_bytes: usize,
    metadata_bytes: usize,
    scratch_bytes: usize,
    work_bytes: usize,
    nodes: usize,
    depth: usize,
}

impl Default for ActiveInputLimits {
    fn default() -> Self {
        Self {
            encoded_bytes: 32 * 1024 * 1024,
            metadata_bytes: 64 * 1024 * 1024,
            scratch_bytes: 64 * 1024 * 1024,
            work_bytes: 128 * 1024 * 1024,
            nodes: 4096,
            depth: 64,
        }
    }
}

impl ActiveInputLimits {
    fn tightened(self) -> Self {
        let maximum = Self::default();
        Self {
            encoded_bytes: self.encoded_bytes.min(maximum.encoded_bytes),
            metadata_bytes: self.metadata_bytes.min(maximum.metadata_bytes),
            scratch_bytes: self.scratch_bytes.min(maximum.scratch_bytes),
            work_bytes: self.work_bytes.min(maximum.work_bytes),
            nodes: self.nodes.min(maximum.nodes),
            depth: self.depth.min(maximum.depth),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("stored report run {resource} exceeds its independent working allowance")]
struct ActiveInputCapacity {
    resource: &'static str,
}

#[derive(Clone, Copy, Debug, Default)]
struct ActiveInputStats {
    callbacks: usize,
    lexical_bytes: usize,
    lexical_nodes: usize,
    full_deserializations: usize,
    header_deserializations: usize,
    selected_string_bytes: usize,
    encode_calls: usize,
    encoded_bytes: usize,
    write_attempts: usize,
    write_commits: usize,
    rejections: usize,
    encoded_demand: usize,
    metadata_demand: usize,
    scratch_demand: usize,
    work_demand: usize,
}

#[cfg(test)]
std::thread_local! {
    static ACTIVE_INPUT_LIMITS: std::cell::Cell<Option<ActiveInputLimits>> = const { std::cell::Cell::new(None) };
    static ACTIVE_INPUT_STATS: std::cell::Cell<Option<ActiveInputStats>> = const { std::cell::Cell::new(None) };
}

fn active_input_limits() -> ActiveInputLimits {
    #[cfg(test)]
    if let Some(limits) = ACTIVE_INPUT_LIMITS.with(std::cell::Cell::get) {
        return limits.tightened();
    }
    ActiveInputLimits::default()
}

fn observe_active_input(observe: impl FnOnce(&mut ActiveInputStats)) {
    #[cfg(test)]
    ACTIVE_INPUT_STATS.with(|slot| {
        if let Some(mut stats) = slot.get() {
            observe(&mut stats);
            slot.set(Some(stats));
        }
    });
    #[cfg(not(test))]
    let _ = observe;
}

fn active_add(left: usize, right: usize) -> Result<usize> {
    left.checked_add(right).ok_or_else(|| {
        ActiveInputCapacity {
            resource: "arithmetic",
        }
        .into()
    })
}

fn active_mul(left: usize, right: usize) -> Result<usize> {
    left.checked_mul(right).ok_or_else(|| {
        ActiveInputCapacity {
            resource: "arithmetic",
        }
        .into()
    })
}

fn active_admit(resource: &'static str, bytes: usize, maximum: usize) -> Result<()> {
    if bytes > maximum {
        observe_active_input(|stats| stats.rejections += 1);
        return Err(ActiveInputCapacity { resource }.into());
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum ActiveJsonContext {
    Run,
    Position,
    Deferred,
    Other,
}

#[derive(Clone, Copy, PartialEq)]
enum ActiveJsonField {
    Id,
    Account,
    Book,
    LocalId,
    Started,
    Observed,
    Eligible,
    Allocated,
    Last,
    ObserverStart,
    ObserverBase,
    LocalChapter,
    LocalAnchor,
    Position,
    Run,
    ChapterUid,
    ChapterIndex,
    ChapterOffset,
    Progress,
    Summary,
    Updated,
    Device,
    Other,
}

#[derive(Clone, Copy)]
struct ActiveJsonFrame {
    context: ActiveJsonContext,
    field: ActiveJsonField,
    seen: u32,
    object: bool,
    state: u8,
}

#[derive(Clone, Copy, PartialEq)]
enum ActiveJsonKind {
    String,
    Number,
    Object,
    Array,
    Null,
    Bool,
}

fn active_json_expected(frame: ActiveJsonFrame) -> Option<ActiveJsonKind> {
    use ActiveJsonContext as C;
    use ActiveJsonField as F;
    match (frame.context, frame.field) {
        (C::Run, F::Id | F::Account | F::Book | F::LocalId)
        | (C::Position, F::ChapterUid | F::Summary | F::Device) => Some(ActiveJsonKind::String),
        (
            C::Run,
            F::Started
            | F::Observed
            | F::Eligible
            | F::Allocated
            | F::Last
            | F::ObserverStart
            | F::ObserverBase
            | F::LocalChapter
            | F::LocalAnchor,
        )
        | (C::Position, F::ChapterIndex | F::ChapterOffset | F::Progress | F::Updated) => {
            Some(ActiveJsonKind::Number)
        }
        (C::Run, F::Position) | (C::Deferred, F::Run) => Some(ActiveJsonKind::Object),
        _ => None,
    }
}

fn active_json_value(
    frame: ActiveJsonFrame,
    kind: ActiveJsonKind,
    token: Option<&str>,
) -> Result<()> {
    if let Some(expected) = active_json_expected(frame) {
        let optional_position = frame.context == ActiveJsonContext::Run
            && frame.field == ActiveJsonField::Position
            && kind == ActiveJsonKind::Null;
        ensure!(
            expected == kind || optional_position,
            "stored report field has the wrong JSON type"
        );
        if expected == ActiveJsonKind::Number {
            let token = token.expect("Numeric tokens are borrowed before validation");
            ensure!(token.len() <= 20, "stored report counter is too long");
            match frame.field {
                ActiveJsonField::Started
                | ActiveJsonField::Last
                | ActiveJsonField::ObserverStart
                | ActiveJsonField::Updated => {
                    token.parse::<i64>()?;
                }
                ActiveJsonField::LocalChapter
                | ActiveJsonField::LocalAnchor
                | ActiveJsonField::ChapterIndex
                | ActiveJsonField::ChapterOffset
                | ActiveJsonField::Progress => {
                    token.parse::<u32>()?;
                }
                _ => {
                    token.parse::<u64>()?;
                }
            }
        }
    }
    Ok(())
}

fn active_json_complete(frame: ActiveJsonFrame) -> Result<()> {
    use ActiveJsonField::*;
    let required: &[ActiveJsonField] = match frame.context {
        ActiveJsonContext::Run => &[
            Id,
            Account,
            Book,
            Started,
            Observed,
            Eligible,
            Allocated,
            Last,
            LocalChapter,
            LocalAnchor,
        ],
        ActiveJsonContext::Position => &[
            ChapterUid,
            ChapterIndex,
            ChapterOffset,
            Progress,
            Summary,
            Updated,
            Device,
        ],
        ActiveJsonContext::Deferred => &[Run],
        ActiveJsonContext::Other => &[],
    };
    ensure!(
        required
            .iter()
            .all(|field| frame.seen & (1_u32 << (*field as u32)) != 0),
        "stored report JSON is missing a required field"
    );
    Ok(())
}

fn active_json_number(token: &str) -> bool {
    let bytes = token.as_bytes();
    let mut position = usize::from(bytes.first() == Some(&b'-'));
    match bytes.get(position) {
        Some(b'0') => position += 1,
        Some(b'1'..=b'9') => {
            position += 1;
            while bytes.get(position).is_some_and(u8::is_ascii_digit) {
                position += 1;
            }
        }
        _ => return false,
    }
    if bytes.get(position) == Some(&b'.') {
        position += 1;
        let start = position;
        while bytes.get(position).is_some_and(u8::is_ascii_digit) {
            position += 1;
        }
        if position == start {
            return false;
        }
    }
    if matches!(bytes.get(position), Some(b'e' | b'E')) {
        position += 1;
        if matches!(bytes.get(position), Some(b'+' | b'-')) {
            position += 1;
        }
        let start = position;
        while bytes.get(position).is_some_and(u8::is_ascii_digit) {
            position += 1;
        }
        if position == start {
            return false;
        }
    }
    position == bytes.len()
}

#[derive(Default)]
struct ActiveJsonInspection {
    text_bytes: usize,
    header_bytes: usize,
    largest_string: usize,
    largest_header_string: usize,
}

fn active_json_field(name: &str) -> ActiveJsonField {
    use ActiveJsonField::*;
    match name {
        "id" => Id,
        "account_key" => Account,
        "book_id" => Book,
        "local_session_id" => LocalId,
        "started_at" => Started,
        "observed_seconds" => Observed,
        "eligible_seconds" => Eligible,
        "allocated_seconds" => Allocated,
        "last_checkpoint_at" => Last,
        "observer_started_at" => ObserverStart,
        "observer_seconds_base" => ObserverBase,
        "local_chapter" => LocalChapter,
        "local_anchor" => LocalAnchor,
        "position" => Position,
        "run" => Run,
        "chapter_uid" => ChapterUid,
        "chapter_index" => ChapterIndex,
        "chapter_offset" => ChapterOffset,
        "progress" => Progress,
        "summary" => Summary,
        "updated_at" => Updated,
        "device" => Device,
        _ => Other,
    }
}

fn active_json_character(raw: &str, position: &mut usize) -> Result<char> {
    let bytes = raw.as_bytes();
    let byte = *bytes
        .get(*position)
        .ok_or_else(|| anyhow::anyhow!("unfinished stored report JSON string"))?;
    if byte != b'\\' {
        let value = raw[*position..].chars().next().unwrap();
        ensure!(
            value >= '\u{20}',
            "invalid control character in stored report JSON string"
        );
        *position += value.len_utf8();
        return Ok(value);
    }
    *position += 1;
    let escaped = *bytes
        .get(*position)
        .ok_or_else(|| anyhow::anyhow!("unfinished stored report JSON escape"))?;
    *position += 1;
    let value = match escaped {
        b'"' => '"',
        b'\\' => '\\',
        b'/' => '/',
        b'b' => '\u{08}',
        b'f' => '\u{0c}',
        b'n' => '\n',
        b'r' => '\r',
        b't' => '\t',
        b'u' => {
            let end = active_add(*position, 4)?;
            let first = u16::from_str_radix(
                raw.get(*position..end)
                    .ok_or_else(|| anyhow::anyhow!("short stored report Unicode escape"))?,
                16,
            )?;
            *position = end;
            let scalar = if (0xd800..=0xdbff).contains(&first) {
                ensure!(
                    bytes.get(*position..active_add(*position, 2)?) == Some(&b"\\u"[..]),
                    "unpaired stored report Unicode surrogate"
                );
                *position += 2;
                let end = active_add(*position, 4)?;
                let second = u16::from_str_radix(
                    raw.get(*position..end)
                        .ok_or_else(|| anyhow::anyhow!("short stored report Unicode surrogate"))?,
                    16,
                )?;
                ensure!(
                    (0xdc00..=0xdfff).contains(&second),
                    "unpaired stored report Unicode surrogate"
                );
                *position = end;
                0x10000 + (u32::from(first) - 0xd800) * 1024 + u32::from(second) - 0xdc00
            } else {
                u32::from(first)
            };
            char::from_u32(scalar)
                .ok_or_else(|| anyhow::anyhow!("invalid stored report Unicode scalar"))?
        }
        _ => bail!("invalid stored report JSON escape"),
    };
    Ok(value)
}

fn inspect_active_json(
    raw: &str,
    deferred: bool,
    limits: ActiveInputLimits,
) -> Result<ActiveJsonInspection> {
    ensure!(
        raw.trim_start().starts_with('{'),
        "stored report run must be a JSON object"
    );
    active_admit(
        "JSON traversal work",
        active_mul(raw.len(), 4)?,
        limits.work_bytes,
    )?;
    let mut inspection = ActiveJsonInspection::default();
    let mut frames = [ActiveJsonFrame {
        context: ActiveJsonContext::Other,
        field: ActiveJsonField::Other,
        seen: 0,
        object: true,
        state: 0,
    }; 64];
    let bytes = raw.as_bytes();
    let mut position = 0usize;
    let mut depth = 0usize;
    let mut nodes = 0usize;
    let mut started = false;
    while position < bytes.len() {
        match bytes[position] {
            b' ' | b'\n' | b'\r' | b'\t' => {
                position += 1;
                continue;
            }
            b':' => {
                ensure!(
                    depth > 0 && frames[depth - 1].object && frames[depth - 1].state == 1,
                    "unexpected stored report JSON colon"
                );
                frames[depth - 1].state = 2;
                position += 1;
                continue;
            }
            b',' => {
                ensure!(
                    depth > 0 && frames[depth - 1].state == 3,
                    "unexpected stored report JSON comma"
                );
                frames[depth - 1].field = ActiveJsonField::Other;
                frames[depth - 1].state = 4;
                position += 1;
                continue;
            }
            b'{' | b'[' => {
                active_admit("JSON depth", active_add(depth, 1)?, limits.depth)?;
                let context = if depth == 0 {
                    ensure!(!started, "trailing stored report JSON object");
                    started = true;
                    if deferred {
                        ActiveJsonContext::Deferred
                    } else {
                        ActiveJsonContext::Run
                    }
                } else {
                    let parent = frames[depth - 1];
                    ensure!(
                        if parent.object {
                            parent.state == 2
                        } else {
                            matches!(parent.state, 0 | 4)
                        },
                        "unexpected stored report JSON container"
                    );
                    active_json_value(
                        parent,
                        if bytes[position] == b'{' {
                            ActiveJsonKind::Object
                        } else {
                            ActiveJsonKind::Array
                        },
                        None,
                    )?;
                    frames[depth - 1].state = 3;
                    match (
                        frames[depth - 1].context,
                        frames[depth - 1].field,
                        bytes[position],
                    ) {
                        (ActiveJsonContext::Deferred, ActiveJsonField::Run, b'{') => {
                            ActiveJsonContext::Run
                        }
                        (ActiveJsonContext::Run, ActiveJsonField::Position, b'{') => {
                            ActiveJsonContext::Position
                        }
                        _ => ActiveJsonContext::Other,
                    }
                };
                frames[depth] = ActiveJsonFrame {
                    context,
                    field: ActiveJsonField::Other,
                    seen: 0,
                    object: bytes[position] == b'{',
                    state: 0,
                };
                depth += 1;
                position += 1;
            }
            b'}' | b']' => {
                ensure!(depth > 0, "unbalanced stored report JSON");
                let frame = frames[depth - 1];
                ensure!(
                    frame.object == (bytes[position] == b'}') && matches!(frame.state, 0 | 3),
                    "unbalanced or incomplete stored report JSON container"
                );
                active_json_complete(frame)?;
                depth -= 1;
                position += 1;
                continue;
            }
            b'"' => {
                ensure!(depth > 0, "trailing stored report JSON string");
                let parent = frames[depth - 1];
                let key = parent.object && matches!(parent.state, 0 | 4);
                ensure!(
                    key || if parent.object {
                        parent.state == 2
                    } else {
                        matches!(parent.state, 0 | 4)
                    },
                    "unexpected stored report JSON string"
                );
                let start = position;
                position += 1;
                let mut decoded = 0usize;
                let mut name = [0u8; 64];
                let mut name_length = 0usize;
                while bytes.get(position) != Some(&b'"') {
                    let value = active_json_character(raw, &mut position)?;
                    decoded = active_add(decoded, value.len_utf8())?;
                    if name_length < name.len() {
                        let mut encoded = [0u8; 4];
                        let text = value.encode_utf8(&mut encoded);
                        if name_length + text.len() <= name.len() {
                            name[name_length..name_length + text.len()]
                                .copy_from_slice(text.as_bytes());
                        }
                    }
                    name_length = active_add(name_length, value.len_utf8())?;
                }
                position += 1;
                let encoded = position - start;
                inspection.largest_string = inspection.largest_string.max(encoded);
                let mut next = position;
                while bytes.get(next).is_some_and(u8::is_ascii_whitespace) {
                    next += 1;
                }
                if key {
                    ensure!(
                        bytes.get(next) == Some(&b':'),
                        "stored report JSON field is missing its colon"
                    );
                    inspection.largest_header_string =
                        inspection.largest_header_string.max(encoded);
                    let field = if name_length <= name.len() {
                        active_json_field(std::str::from_utf8(&name[..name_length])?)
                    } else {
                        ActiveJsonField::Other
                    };
                    let frame = &mut frames[depth - 1];
                    if field != ActiveJsonField::Other && frame.context != ActiveJsonContext::Other
                    {
                        let bit = 1_u32 << (field as u32);
                        ensure!(frame.seen & bit == 0, "duplicate stored report JSON field");
                        frame.seen |= bit;
                    }
                    frame.field = field;
                    frame.state = 1;
                } else {
                    let frame = frames[depth - 1];
                    active_json_value(frame, ActiveJsonKind::String, None)?;
                    frames[depth - 1].state = 3;
                    let header = frame.context == ActiveJsonContext::Run
                        && matches!(
                            frame.field,
                            ActiveJsonField::Id
                                | ActiveJsonField::Account
                                | ActiveJsonField::Book
                                | ActiveJsonField::LocalId
                        );
                    let selected = header
                        || frame.context == ActiveJsonContext::Position
                            && matches!(
                                frame.field,
                                ActiveJsonField::ChapterUid
                                    | ActiveJsonField::Summary
                                    | ActiveJsonField::Device
                            );
                    if selected {
                        inspection.text_bytes = active_add(inspection.text_bytes, decoded)?;
                    }
                    if header {
                        inspection.header_bytes = active_add(inspection.header_bytes, decoded)?;
                        inspection.largest_header_string =
                            inspection.largest_header_string.max(encoded);
                        let maximum = match frame.field {
                            ActiveJsonField::Book => 2048,
                            ActiveJsonField::Account => 64,
                            _ => 256,
                        };
                        ensure!(decoded <= maximum, "invalid stored active identity field");
                    }
                    if frame.context == ActiveJsonContext::Position
                        && frame.field == ActiveJsonField::ChapterUid
                    {
                        active_admit("chapter UID bytes", decoded, 4096)?;
                    }
                }
            }
            _ => {
                ensure!(depth > 0, "trailing stored report JSON token");
                let frame = frames[depth - 1];
                ensure!(
                    if frame.object {
                        frame.state == 2
                    } else {
                        matches!(frame.state, 0 | 4)
                    },
                    "unexpected stored report JSON token"
                );
                let start = position;
                while bytes.get(position).is_some_and(|byte| {
                    !matches!(byte, b',' | b'}' | b']' | b' ' | b'\n' | b'\r' | b'\t')
                }) {
                    position += 1;
                }
                ensure!(position > start, "invalid stored report JSON token");
                let token = &raw[start..position];
                let kind = match token {
                    "null" => ActiveJsonKind::Null,
                    "true" | "false" => ActiveJsonKind::Bool,
                    _ if active_json_number(token) => ActiveJsonKind::Number,
                    _ => bail!("invalid stored report JSON token"),
                };
                active_json_value(frame, kind, Some(token))?;
                frames[depth - 1].state = 3;
            }
        }
        nodes = active_add(nodes, 1)?;
        active_admit("JSON nodes", nodes, limits.nodes)?;
    }
    ensure!(started && depth == 0, "unbalanced stored report JSON");
    observe_active_input(|stats| {
        stats.lexical_bytes += raw.len();
        stats.lexical_nodes += nodes;
    });
    Ok(inspection)
}

fn active_store_limits(limits: ActiveInputLimits) -> crate::store::StoredSettingReadLimits {
    crate::store::StoredSettingReadLimits {
        encoded_bytes: limits.encoded_bytes,
        utf8_bytes: limits.encoded_bytes,
        work_bytes: limits.work_bytes,
        scratch_bytes: limits.scratch_bytes,
        json_passes: 2,
    }
}

fn active_store_error(error: anyhow::Error) -> anyhow::Error {
    if error
        .downcast_ref::<crate::store::StoredSettingReadLimit>()
        .is_some()
        || error
            .downcast_ref::<crate::store::CachedResourceLimit>()
            .is_some()
    {
        ActiveInputCapacity {
            resource: "Store setting envelope",
        }
        .into()
    } else {
        error
    }
}

fn admit_active_raw(
    raw: &str,
    key: &str,
    metadata: crate::store::StoredSettingTextMetadata,
    deferred: bool,
    header: bool,
) -> Result<ActiveJsonInspection> {
    let limits = active_input_limits();
    active_admit("UTF-8 bytes", raw.len(), limits.encoded_bytes)?;
    let inspection = inspect_active_json(raw, deferred, limits)?;
    let text = if header {
        inspection.header_bytes
    } else {
        inspection.text_bytes
    };
    let fixed = if header {
        std::mem::size_of::<ActiveHeader>()
    } else {
        std::mem::size_of::<ActiveRun>()
    };
    let owned = active_mul(active_add(text, fixed)?, 2)?;
    let largest = if header {
        inspection.largest_header_string
    } else {
        inspection.largest_string
    };
    let parser = largest
        .max(256)
        .checked_next_power_of_two()
        .ok_or(ActiveInputCapacity {
            resource: "parser scratch",
        })?;
    let live = active_add(metadata.vm_scratch_bytes, active_add(parser, owned)?)?;
    let acquisition = active_add(
        metadata.vm_scratch_bytes,
        active_add(active_mul(metadata.utf8_bytes_upper, 2)?, 256)?,
    )?;
    let scratch = live.max(acquisition);
    let callback_work = active_add(active_mul(raw.len(), if header { 4 } else { 8 })?, owned)?;
    let acquisition_work = active_add(
        active_mul(metadata.utf8_bytes_upper, 3)?,
        active_mul(key.len(), 2)?,
    )?;
    let work = callback_work.max(acquisition_work);
    observe_active_input(|stats| {
        stats.encoded_demand = metadata.encoded_bytes.max(metadata.utf8_bytes_upper);
        stats.metadata_demand = owned;
        stats.scratch_demand = scratch;
        stats.work_demand = work;
    });
    active_admit("selected metadata", owned, limits.metadata_bytes)?;
    active_admit("VM/parser/owned scratch", scratch, limits.scratch_bytes)?;
    active_admit("JSON/read/copy work", work, limits.work_bytes)?;
    Ok(inspection)
}

fn read_active_run(store: &Store, key: &str, deferred: bool) -> Result<Option<ActiveRun>> {
    store
        .with_bounded_setting_text(
            key,
            active_store_limits(active_input_limits()),
            |raw, metadata| {
                observe_active_input(|stats| stats.callbacks += 1);
                let inspection = admit_active_raw(raw, key, metadata, deferred, false)?;
                observe_active_input(|stats| {
                    stats.full_deserializations += 1;
                    stats.selected_string_bytes += inspection.text_bytes;
                });
                let active = if deferred {
                    serde_json::from_str::<DeferredAllocation>(raw)?.run
                } else {
                    serde_json::from_str::<ActiveRun>(raw)?
                };
                validate_active(&active)?;
                if deferred {
                    ensure!(
                        key == format!(
                            "{ALLOCATION_DEBT_PREFIX}{}:{}",
                            active.account_key, active.id
                        ),
                        "deferred report key changed its run identity"
                    );
                }
                Ok(active)
            },
        )
        .map_err(active_store_error)
}

fn read_active_header(store: &Store, key: &str, deferred: bool) -> Result<Option<ActiveHeader>> {
    store
        .with_bounded_setting_text(
            key,
            active_store_limits(active_input_limits()),
            |raw, metadata| {
                observe_active_input(|stats| stats.callbacks += 1);
                let inspection = admit_active_raw(raw, key, metadata, deferred, true)?;
                observe_active_input(|stats| {
                    stats.header_deserializations += 1;
                    stats.selected_string_bytes += inspection.header_bytes;
                });
                let active = if deferred {
                    serde_json::from_str::<DeferredHeader>(raw)?.run
                } else {
                    serde_json::from_str::<ActiveHeader>(raw)?
                };
                active.validate()?;
                if deferred {
                    ensure!(
                        key == format!(
                            "{ALLOCATION_DEBT_PREFIX}{}:{}",
                            active.account_key, active.id
                        ),
                        "deferred report key changed its run identity"
                    );
                }
                Ok(active)
            },
        )
        .map_err(active_store_error)
}

fn active_text_bytes(active: &ActiveRun) -> Result<usize> {
    let mut bytes = 0usize;
    for text in [
        &active.id,
        &active.account_key,
        &active.book_id,
        &active.local_session_id,
    ] {
        bytes = active_add(bytes, text.len())?;
    }
    if let Some(position) = &active.position {
        for text in [&position.chapter_uid, &position.summary, &position.device] {
            bytes = active_add(bytes, text.len())?;
        }
    }
    Ok(bytes)
}

fn active_encoded_bytes(value: &impl Serialize, text: usize) -> Result<usize> {
    let limits = active_input_limits();
    active_admit(
        "encoding work",
        active_add(active_mul(text, 8)?, 4096)?,
        limits.work_bytes,
    )?;
    outbox::report_input_encoded_bytes(value, limits.encoded_bytes).map_err(|error| {
        if report_capacity(&error) {
            ActiveInputCapacity {
                resource: "encoded output",
            }
            .into()
        } else {
            error
        }
    })
}

fn encode_active_value(value: &impl Serialize, text: usize, extra: usize) -> Result<String> {
    let limits = active_input_limits();
    let metadata = active_add(
        active_mul(active_add(text, std::mem::size_of::<ActiveRun>())?, 2)?,
        extra,
    )?;
    active_admit("maintenance metadata", metadata, limits.metadata_bytes)?;
    let encoded = active_encoded_bytes(value, text)?;
    active_admit(
        "maintenance/output scratch",
        active_add(metadata, encoded)?,
        limits.scratch_bytes,
    )?;
    // The borrowed compact format is measured before requesting output storage.
    let mut bytes = Vec::with_capacity(encoded);
    serde_json::to_writer(&mut bytes, value)?;
    ensure!(
        bytes.len() == encoded,
        "active report encoding changed after admission"
    );
    observe_active_input(|stats| {
        stats.encode_calls += 1;
        stats.encoded_bytes += encoded;
    });
    Ok(String::from_utf8(bytes)?)
}

fn read_debt_marker(store: &Store) -> Result<Option<String>> {
    store
        .with_bounded_setting_text(
            ALLOCATION_DEBT_CURSOR_KEY,
            crate::store::StoredSettingReadLimits {
                encoded_bytes: 8192,
                utf8_bytes: 8192,
                work_bytes: 128 * 1024,
                scratch_bytes: 64 * 1024,
                json_passes: 2,
            },
            |raw, metadata| {
                let raw = raw.trim();
                ensure!(
                    raw.starts_with('"'),
                    "report debt marker must be a JSON string"
                );
                let mut position = 1usize;
                let mut decoded = 0usize;
                while raw.as_bytes().get(position) != Some(&b'"') {
                    decoded = active_add(
                        decoded,
                        active_json_character(raw, &mut position)?.len_utf8(),
                    )?;
                    active_admit("debt marker bytes", decoded, 4096)?;
                }
                ensure!(
                    raw[position + 1..].trim().is_empty(),
                    "trailing report debt marker JSON"
                );
                let owned = active_mul(decoded, 2)?;
                active_admit(
                    "debt marker scratch",
                    active_add(
                        metadata.vm_scratch_bytes,
                        active_add(active_mul(raw.len(), 2)?, owned)?,
                    )?,
                    64 * 1024,
                )?;
                let marker: String = serde_json::from_str(raw)?;
                ensure!(
                    marker.starts_with(ALLOCATION_DEBT_PREFIX) && !marker.contains('\0'),
                    "invalid report debt marker"
                );
                Ok(marker)
            },
        )
        .map_err(active_store_error)
}

#[derive(Serialize)]
struct ActiveCursorView<'a> {
    id: &'a str,
    account_key: &'a str,
    book_id: &'a str,
    started_at: i64,
    observed_seconds: u64,
    eligible_seconds: u64,
    allocated_seconds: u64,
    last_checkpoint_at: i64,
    local_chapter: u32,
    local_anchor: u32,
    position: &'a Option<source::ReadingPosition>,
    observer_started_at: i64,
    observer_seconds_base: u64,
    local_session_id: &'a str,
}

impl<'a> ActiveCursorView<'a> {
    fn new(active: &'a ActiveRun, cursor: u64) -> Self {
        Self {
            id: &active.id,
            account_key: &active.account_key,
            book_id: &active.book_id,
            started_at: active.started_at,
            observed_seconds: active.observed_seconds,
            eligible_seconds: active.eligible_seconds,
            allocated_seconds: cursor,
            last_checkpoint_at: active.last_checkpoint_at,
            local_chapter: active.local_chapter,
            local_anchor: active.local_anchor,
            position: &active.position,
            observer_started_at: active.observer_started_at,
            observer_seconds_base: active.observer_seconds_base,
            local_session_id: &active.local_session_id,
        }
    }
}

enum AllocationRunView<'a> {
    Active(ActiveCursorView<'a>),
    Deferred(ActiveCursorView<'a>),
}

impl Serialize for AllocationRunView<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Active(run) => run.serialize(serializer),
            Self::Deferred(run) => {
                use serde::ser::SerializeStruct;
                let mut state = serializer.serialize_struct("DeferredAllocation", 1)?;
                state.serialize_field("run", run)?;
                state.end()
            }
        }
    }
}

#[derive(Clone, Copy)]
struct AllocationProgress {
    added: usize,
    cursor: u64,
    complete: bool,
}

impl App {
    /// Call before restoring the initial Reader route, and before starting work.
    pub(super) fn recover_read_reports(&mut self) -> Result<()> {
        match self.recover_read_reports_inner() {
            Ok(()) => self.store.delete_setting(STORAGE_ERROR_KEY)?,
            Err(error) if report_capacity(&error) => {
                self.note_read_report_error(&error);
                self.store.delete_setting(STORAGE_ERROR_KEY)?;
            }
            Err(error) => self.note_read_report_error(&error),
        }
        Ok(())
    }

    fn recover_read_reports_inner(&mut self) -> Result<()> {
        let mut queue = self.load_report_outbox()?;
        self.reconcile_report_results(&mut queue)?;
        queue.recover_sending();
        self.save_report_outbox(&mut queue)?;
        if let Some(active) = read_active_run(&self.store, ACTIVE_KEY, false)? {
            validate_active(&active)?;
            self.archive_report_run(&active, None)?;
        }
        if let Err(error) = self.advance_deferred_report_allocation() {
            if report_capacity(&error) {
                self.note_read_report_error(&error);
            } else {
                return Err(error);
            }
        }
        self.store.set_setting(SCHEDULE_KEY, &0_i64)?;
        Ok(())
    }

    pub(super) fn start_read_report_session(&mut self) -> Result<()> {
        let Some(started_at) = self.reader.session_started else {
            return Ok(());
        };
        if self.demo
            || !self
                .store
                .book_source(&self.reader.book_id)?
                .is_some_and(|source| source == "weread")
        {
            return Ok(());
        }
        let account_key = self
            .current_report_account()
            .unwrap_or_else(|| UNATTRIBUTED_ACCOUNT.into());
        let mut observer_seconds_base = 0;
        let mut local_session_id = String::new();
        let mut run_started_at = started_at;
        let header = read_active_header(&self.store, ACTIVE_KEY, false)?;
        if header.as_ref().is_some_and(|active| {
            active.belongs_to_reader(self) && active.account_key == account_key
        }) {
            return Ok(());
        }
        drop(header);
        let mut previous = read_active_run(&self.store, ACTIVE_KEY, false)?;
        if let Some(active) = previous.as_mut() {
            validate_active(&active)?;
            if active.belongs_to_reader(self) && active.account_key == account_key {
                return Ok(());
            }
            if active.belongs_to_reader(self) {
                let now = Utc::now().timestamp().max(active.started_at);
                let observed = self
                    .reader
                    .session_seconds
                    .saturating_sub(active.observer_seconds_base)
                    .min(now.saturating_sub(active.started_at) as u64);
                observe_active(active, observed, now, self.prefs.report_time)?;
                self.save_active_run(active)?;
                observer_seconds_base = active
                    .observer_seconds_base
                    .checked_add(observed)
                    .ok_or_else(|| anyhow::anyhow!("reading observation baseline overflow"))?;
                local_session_id = active.local_session_id().to_owned();
                run_started_at = Utc::now().timestamp().max(started_at);
            }
            // Closing evidence is independent of allocation capacity. Archive
            // one measured run before a replacement can own ACTIVE_KEY.
        }
        let active = ActiveRun {
            id: unique_id("reading"),
            account_key,
            book_id: self.reader.book_id.clone(),
            started_at: run_started_at,
            observed_seconds: 0,
            eligible_seconds: 0,
            allocated_seconds: 0,
            last_checkpoint_at: run_started_at,
            local_chapter: self.reader.chapter_index,
            local_anchor: self
                .reader
                .paginator
                .as_ref()
                .map_or(0, |paginator| paginator.current_page().start),
            position: self.reader_upload_position()?,
            observer_started_at: started_at,
            observer_seconds_base,
            local_session_id,
        };
        if let Some(previous) = previous {
            self.archive_report_run(&previous, Some(&active))
        } else {
            self.save_active_run(&active)
        }
    }

    /// The caller increments session_seconds only for actual active Reader time.
    pub(super) fn checkpoint_read_reports(&mut self, flush: bool) -> Result<()> {
        self.checkpoint_read_reports_inner(flush).map(|_| ())
    }

    fn checkpoint_read_reports_inner(&mut self, flush: bool) -> Result<usize> {
        if self.demo || self.reader.session_started.is_none() {
            return Ok(0);
        }
        let storage_blocked = self
            .store
            .setting::<bool>(STORAGE_ERROR_KEY)?
            .unwrap_or(false);
        let current = read_active_header(&self.store, ACTIVE_KEY, false)?;
        let account = self
            .current_report_account()
            .unwrap_or_else(|| UNATTRIBUTED_ACCOUNT.into());
        let replace = current
            .as_ref()
            .is_none_or(|run| !run.belongs_to_reader(self) || run.account_key != account);
        if !replace && !flush {
            let header = current.as_ref().expect("A matching report header exists");
            let now = Utc::now().timestamp().max(header.started_at);
            let observed = self
                .reader
                .session_seconds
                .saturating_sub(header.observer_seconds_base)
                .min(now.saturating_sub(header.started_at) as u64);
            ensure!(
                observed >= header.observed_seconds,
                "active reading seconds moved backwards"
            );
            if observed == header.observed_seconds {
                return Ok(0);
            }
        }
        drop(current);
        if replace {
            self.start_read_report_session()?;
        }
        let Some(mut active) = read_active_run(&self.store, ACTIVE_KEY, false)? else {
            return Ok(0);
        };
        ensure!(
            active.belongs_to_reader(self),
            "reading report belongs to another active run"
        );
        validate_active(&active)?;
        let now = Utc::now().timestamp().max(active.started_at);
        let observed = self
            .reader
            .session_seconds
            .saturating_sub(active.observer_seconds_base)
            .min(now.saturating_sub(active.started_at) as u64);
        ensure!(
            observed >= active.observed_seconds,
            "active reading seconds moved backwards"
        );
        let delta = observed - active.observed_seconds;
        if delta == 0 && !flush {
            return Ok(0);
        }
        observe_active(&mut active, observed, now, self.prefs.report_time)?;
        let anchor = self
            .reader
            .paginator
            .as_ref()
            .map_or(0, |paginator| paginator.current_page().start);
        if active.local_chapter != self.reader.chapter_index
            || active.local_anchor != anchor
            || active.position.is_none()
            || active
                .position
                .as_ref()
                .is_some_and(|position| position.updated_at > 0)
                != self.reader.position_confirmed
        {
            active.position = self.reader_upload_position()?;
            active.local_chapter = self.reader.chapter_index;
            active.local_anchor = anchor;
        }
        validate_active(&active)?;
        // The observed counter is durable before allocating its unique intervals.
        // A crash between these writes can lose neither the allocation nor its source.
        self.save_active_run(&active)?;
        self.persist_report_run_session(&active)?;
        let mut added = 0;
        if !storage_blocked
            && (flush
                || active
                    .eligible_seconds
                    .saturating_sub(active.allocated_seconds)
                    >= BATCH_SECONDS)
        {
            let mut queue = self.load_report_outbox()?;
            let progress =
                self.flush_report_run(&mut active, &mut queue, flush, ACTIVE_KEY, false, false)?;
            if progress.added > 0 {
                added = progress.added;
                self.store.set_setting(SCHEDULE_KEY, &0_i64)?;
            }
        }
        Ok(added)
    }

    /// Return the stable local session ID, so the final local upsert is idempotent.
    pub(super) fn finish_read_report_session(&mut self) -> Result<Option<String>> {
        if let Err(error) = self.checkpoint_read_reports(true) {
            if report_capacity(&error) {
                self.note_read_report_error(&error);
            } else {
                return Err(error);
            }
        }
        let Some(active) = read_active_run(&self.store, ACTIVE_KEY, false)? else {
            return Ok(None);
        };
        ensure!(
            active.belongs_to_reader(self),
            "cannot finish another reading report run"
        );
        let id = active.local_session_id().to_owned();
        self.archive_report_run(&active, None)?;
        Ok(Some(id))
    }

    /// Account replacement/cancellation can interrupt a request after it was sent.
    pub(super) fn interrupt_read_reports(&mut self) -> Result<()> {
        if let Err(error) = self.interrupt_read_reports_inner() {
            self.note_read_report_error(&error);
        }
        Ok(())
    }

    /// Native ownership requires a durable report checkpoint, unlike ordinary
    /// best-effort cancellation. Earlier finish failures also leave this flag.
    pub(super) fn interrupt_resident_read_reports(&mut self) -> Result<()> {
        let result = (|| -> Result<()> {
            self.interrupt_read_reports_inner()?;
            ensure!(
                !self
                    .store
                    .setting::<bool>(STORAGE_ERROR_KEY)?
                    .unwrap_or(false),
                "reading report persistence is unavailable"
            );
            Ok(())
        })();
        if let Err(error) = &result {
            self.note_read_report_error(error);
        }
        result
    }

    fn interrupt_read_reports_inner(&mut self) -> Result<()> {
        if let Err(error) = self.checkpoint_read_reports(true) {
            if report_capacity(&error) {
                self.note_read_report_error(&error);
            } else {
                return Err(error);
            }
        }
        let mut queue = self.load_report_outbox()?;
        self.reconcile_report_results(&mut queue)?;
        queue.recover_sending();
        self.save_report_outbox(&mut queue)?;
        self.store.set_setting(SCHEDULE_KEY, &0_i64)
    }

    pub(super) fn tick_read_reports(&mut self) -> Result<()> {
        if let Err(error) = self.tick_read_reports_inner() {
            self.note_read_report_error(&error);
        }
        Ok(())
    }

    fn tick_read_reports_inner(&mut self) -> Result<()> {
        let mut added = 0;
        if matches!(self.route, Route::Reader(..)) {
            match self.checkpoint_read_reports_inner(false) {
                Ok(count) => added = count,
                Err(error) if report_capacity(&error) => self.note_read_report_error(&error),
                Err(error) => return Err(error),
            }
        }
        if self
            .store
            .setting::<bool>(STORAGE_ERROR_KEY)?
            .unwrap_or(false)
        {
            return Ok(());
        }
        if added == 0 {
            if let Err(error) = self.advance_deferred_report_allocation() {
                if report_capacity(&error) {
                    self.note_read_report_error(&error);
                } else {
                    return Err(error);
                }
            }
        }
        let now = Utc::now().timestamp();
        if self
            .store
            .setting::<i64>(SCHEDULE_KEY)?
            .is_some_and(|due| due > now)
        {
            return Ok(());
        }
        let mut queue = self.load_report_outbox()?;
        if self.reconcile_report_results(&mut queue)? {
            self.save_report_outbox(&mut queue)?;
        }
        self.store.set_setting(
            SCHEDULE_KEY,
            &now.saturating_add(
                if queue
                    .batches
                    .iter()
                    .any(|batch| batch.state == BatchState::Sending)
                {
                    1
                } else {
                    5
                },
            ),
        )?;
        if self.demo
            || self.force_offline
            || !self.online
            || !self.prefs.report_time
            || !self.prefs.push_on_close
            || self.prefs.report_reader_only && !self.reader_is_active_page()
            || !self.jobs.capabilities().remote_progress
        {
            return Ok(());
        }
        let Some(session) = self
            .jobs
            .session()
            .filter(|session| session.has_web_session())
        else {
            return Ok(());
        };
        let Some(account) = source::report::account_key(&session) else {
            return Ok(());
        };
        let Some(id) = queue.next_pending(&account, now) else {
            return Ok(());
        };
        let current_auth = auth_revision(&session)?;
        if self
            .store
            .setting::<String>(&format!("report.await_auth:{id}"))?
            .is_some_and(|previous| previous == current_auth)
        {
            return Ok(());
        }
        let index = queue
            .batches
            .iter()
            .position(|batch| batch.request.session_id == id)
            .ok_or_else(|| anyhow::anyhow!("pending report disappeared"))?;
        let request = &mut queue.batches[index].request;
        if self.reader.book_id == request.book_id && self.reader.cloud.is_some() {
            return Ok(());
        }
        request.active_reader_intent = false;
        let current_reader = self.reader_is_active_page()
            && self.reader.session_started.is_some()
            && self.reader.position_confirmed
            && self.reader.book_id == request.book_id
            && matches!(&self.route, Route::Reader(book, _) if book == &request.book_id)
            && self
                .ui
                .upgrade()
                .is_some_and(|ui| !ui.global::<ReaderModel>().get_sync_conflict());
        if current_reader {
            request.position = self.reader_upload_position()?;
            request.active_reader_intent = request.position.is_some();
        }
        if !self
            .store
            .book_source(&request.book_id)?
            .is_some_and(|source| source == "weread")
        {
            queue.batches[index].state = BatchState::Blocked;
            queue.batches[index].error = Some("source.unsupported.read_report_book".into());
            queue.batches[index].retry_after = 0;
            return self.save_report_outbox(&mut queue);
        }
        if request.position.is_none() {
            queue.batches[index].state = BatchState::Blocked;
            queue.batches[index].error = Some("source.unsupported.read_position_required".into());
            queue.batches[index].retry_after = 0;
            return self.save_report_outbox(&mut queue);
        }
        request.progress_write_allowed = true;
        let request = queue.transport_request(index)?;
        if let Err(error) = request.validate(&session, now) {
            queue.batches[index].state = BatchState::Blocked;
            queue.batches[index].error = Some(error.to_string());
            queue.batches[index].retry_after = 0;
            return self.save_report_outbox(&mut queue);
        }
        // Remove stale, definitely unsuccessful attempt metadata before delivery.
        for prefix in [
            "report.error",
            "report.failure",
            "report.retryable",
            "report.requires_login",
            "report.await_auth",
        ] {
            self.store.delete_setting(&format!("{prefix}:{id}"))?;
        }
        queue.mark_sending(&id, now)?;
        self.save_report_outbox(&mut queue)?;
        self.store
            .set_setting(SCHEDULE_KEY, &now.saturating_add(1))?;
        let work = self.request();
        self.settle_job_submission(self.jobs.report_time(work, request));
        Ok(())
    }

    pub(super) fn read_report_summary(&self) -> String {
        if self.demo {
            return tr("report_demo_local_only");
        }
        if self
            .store
            .setting::<bool>(STORAGE_ERROR_KEY)
            .map_or(true, |value| value.unwrap_or(false))
        {
            return tr("report_storage_error");
        }
        let queue = match self.load_report_outbox() {
            Ok(queue) => queue,
            Err(error) if report_capacity(&error) => return tr("api_error"),
            Err(_) => return tr("report_storage_error"),
        };
        let Some(account) = self.current_report_account() else {
            return tr("report_requires_web_identity");
        };
        let own: Vec<_> = queue
            .batches
            .iter()
            .filter(|batch| batch.request.account_key == account)
            .collect();
        let uncertain: u64 = own
            .iter()
            .filter(|batch| batch.state == BatchState::Uncertain)
            .map(|batch| u64::from(batch.request.seconds))
            .sum();
        if uncertain > 0 {
            return i18n::format(
                "report_uncertain_seconds",
                &[("seconds", uncertain.to_string())],
            );
        }
        if !self.prefs.report_time {
            return tr("report_disabled_local");
        }
        if !self.prefs.push_on_close {
            return tr("report_requires_progress_write");
        }
        if !self.jobs.capabilities().remote_progress {
            return tr("report_requires_progress_check");
        }
        if !self
            .jobs
            .session()
            .is_some_and(|session| session.has_web_session())
        {
            return tr("report_requires_web_identity");
        }
        if own.iter().any(|batch| batch.state == BatchState::Blocked) {
            return tr("report_delivery_blocked");
        }
        if self.reader.cloud.is_some()
            && own.iter().any(|batch| {
                batch.request.book_id == self.reader.book_id
                    && matches!(batch.state, BatchState::Pending | BatchState::Sending)
            })
        {
            return tr("sync_conflict");
        }
        let pending: u64 = own
            .iter()
            .filter(|batch| matches!(batch.state, BatchState::Pending | BatchState::Sending))
            .map(|batch| u64::from(batch.request.seconds))
            .sum();
        let active = match read_active_header(&self.store, ACTIVE_KEY, false) {
            Ok(active) => active,
            Err(error) if report_capacity(&error) => return tr("api_error"),
            Err(_) => return tr("report_storage_error"),
        };
        if active
            .as_ref()
            .is_some_and(|active| active.validate().is_err())
        {
            return tr("report_storage_error");
        }
        let unallocated = active
            .filter(|active| active.account_key == account)
            .map_or(0, |active| {
                active
                    .eligible_seconds
                    .saturating_sub(active.allocated_seconds)
            });
        let debt_prefix = format!("{ALLOCATION_DEBT_PREFIX}{account}:");
        let debt = self
            .store
            .first_setting_key_with_prefix(&debt_prefix, None)
            .and_then(|key| {
                key.map(|key| read_active_header(&self.store, &key, true))
                    .transpose()
            })
            .map(|debt| debt.flatten());
        let debt = match debt {
            Ok(debt) => debt,
            Err(_) => return tr("api_error"),
        };
        if debt
            .as_ref()
            .is_some_and(|debt| debt.validate().is_err() || debt.account_key != account)
        {
            return tr("report_storage_error");
        }
        let remaining = debt.as_ref().map_or(0, |debt| {
            debt.eligible_seconds.saturating_sub(debt.allocated_seconds)
        });
        if unallocated > 0 || debt.is_some() {
            // One selected debt is a lower bound, never an assertion that all
            // remaining run records were collected or confirmed.
            let seconds = pending
                .saturating_add(unallocated)
                .saturating_add(remaining);
            return i18n::format(
                "report_pending_seconds",
                &[("seconds", format!("{seconds}+"))],
            );
        }
        if pending > 0 {
            return i18n::format(
                "report_pending_seconds",
                &[("seconds", pending.to_string())],
            );
        }
        if let Ok(Some(timestamp)) = self
            .store
            .setting::<i64>(&format!("report.last_success:{account}"))
        {
            let time = chrono::DateTime::from_timestamp(timestamp, 0)
                .map(|time| {
                    time.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
                        .format("%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_default();
            return i18n::format("report_last_success", &[("time", time)]);
        }
        tr("report_waiting_activity")
    }

    fn current_report_account(&self) -> Option<String> {
        self.jobs
            .session()
            .and_then(|session| source::report::account_key(&session))
    }

    pub(super) fn active_read_report_session_id(&self) -> Option<String> {
        read_active_header(&self.store, ACTIVE_KEY, false)
            .ok()
            .flatten()
            .filter(|active| active.validate().is_ok() && active.belongs_to_reader(self))
            .map(|active| active.local_session_id().to_owned())
    }

    pub(super) fn note_read_report_error(&mut self, error: &anyhow::Error) {
        if report_capacity(error) {
            crate::diagnostic!(
                "Reading-time allocation is deferred within its operation allowance"
            );
            self.settings_status = tr("api_error");
            self.show_toast("api_error");
            return;
        }
        crate::diagnostic!(
            "Reading-time storage/state is unavailable; automatic delivery is disabled"
        );
        self.settings_status = tr("report_storage_error");
        let _ = self.store.set_setting(STORAGE_ERROR_KEY, &true);
    }

    fn load_report_outbox(&self) -> Result<ReportOutbox> {
        load_report_input(&self.store)
    }

    fn save_report_outbox(&self, queue: &mut ReportOutbox) -> Result<()> {
        persist_report_outbox(&self.store, queue)
    }

    #[cfg(test)]
    pub(super) fn validate_report_input_for_test(&self) -> Result<()> {
        self.load_report_outbox().map(|_| ())
    }

    #[cfg(test)]
    pub(super) fn reconcile_report_input_for_test(&self) -> Result<()> {
        let mut queue = self.load_report_outbox()?;
        self.reconcile_report_results(&mut queue).map(|_| ())
    }

    #[cfg(test)]
    pub(super) fn compact_report_input_for_test(&self) -> Result<()> {
        let queue = self.load_report_outbox()?;
        queue.compacted().map(|_| ())
    }

    #[cfg(test)]
    pub(super) fn persist_report_input_for_test(&self) -> Result<()> {
        let mut queue = self.load_report_outbox()?;
        self.save_report_outbox(&mut queue)
    }

    #[cfg(test)]
    pub(super) fn persist_report_sending_for_test(&self, session_id: &str) -> Result<()> {
        let mut queue = self.load_report_outbox()?;
        let index = queue
            .batches
            .iter()
            .position(|batch| batch.request.session_id == session_id)
            .ok_or_else(|| anyhow::anyhow!("pending report disappeared"))?;
        let _request = queue.transport_request(index)?;
        queue.mark_sending(session_id, Utc::now().timestamp())?;
        self.save_report_outbox(&mut queue)
    }

    fn persist_report_run_session(&self, active: &ActiveRun) -> Result<()> {
        validate_active(active)?;
        if active.observed_seconds > 0 {
            self.store.record_session(&ReadingSession {
                id: active.local_session_id().to_owned(),
                book_id: active.book_id.clone(),
                started_at: active.reader_started_at(),
                ended_at: active.last_checkpoint_at,
                seconds: active
                    .observed_seconds
                    .checked_add(active.observer_seconds_base)
                    .ok_or_else(|| anyhow::anyhow!("local reading counter overflow"))?,
            })?;
        }
        Ok(())
    }

    fn commit_active_maintenance(
        &self,
        values: &[(&str, &str)],
        remove_active: bool,
    ) -> Result<()> {
        observe_active_input(|stats| stats.write_attempts += 1);
        self.store
            .set_report_active_encoded_and_delete(values, remove_active)
            .map_err(active_store_error)?;
        observe_active_input(|stats| stats.write_commits += 1);
        Ok(())
    }

    fn save_active_run(&self, active: &ActiveRun) -> Result<()> {
        validate_active(active)?;
        let encoded = encode_active_value(active, active_text_bytes(active)?, 0)?;
        self.commit_active_maintenance(&[(ACTIVE_KEY, encoded.as_str())], false)
    }

    fn flush_report_run(
        &self,
        active: &mut ActiveRun,
        queue: &mut ReportOutbox,
        flush: bool,
        run_key: &str,
        deferred: bool,
        retire_at_target: bool,
    ) -> Result<AllocationProgress> {
        validate_active(active)?;
        validate_active_proof_identity(active, queue)?;
        let cursor = queue.allocation_cursor(&active.id);
        ensure!(
            cursor <= active.eligible_seconds,
            "allocated reading time exceeds observed eligibility"
        );
        let target = if flush {
            active.eligible_seconds
        } else {
            cursor + (active.eligible_seconds - cursor) / BATCH_SECONDS * BATCH_SECONDS
        };
        let limits = allocation_limits();
        let metadata = active_metadata_bytes(active)?;
        if metadata > limits.metadata_bytes {
            return Err(AllocationCapacity {
                resource: "active metadata",
            }
            .into());
        }
        let view = allocation_run_view(active, cursor, deferred);
        let encoded = allocation_encoded_bytes(&view, AllocationLimits::default().encoded_bytes)?;
        let prepared = queue.prepare_allocation(
            AllocationInput {
                run_id: &active.id,
                account_key: &active.account_key,
                book_id: &active.book_id,
                started_at: active.started_at,
                allocated_seconds: active.allocated_seconds,
                target_seconds: target,
                position: active.position.as_ref(),
                progress_write_allowed: self.prefs.push_on_close,
            },
            AllocationExtra {
                metadata_bytes: metadata,
                encoded_bytes: encoded,
                cursor,
                fixed_encoded_bytes: 0,
                omit_at_target: retire_at_target,
            },
        )?;
        let Some(prepared) = prepared else {
            // Existing allocation proof is already durable. A legacy stale
            // counter may catch up without issuing or copying another batch.
            ensure!(
                cursor >= active.allocated_seconds && cursor <= target,
                "durable report allocation history was lost"
            );
            if retire_at_target {
                // Allocation proof is already durable. Only this selected,
                // fully allocated run is retired; no new batch is allocated.
                if run_key == ACTIVE_KEY {
                    self.commit_active_maintenance(&[], true)?;
                } else {
                    ensure!(
                        run_key.starts_with(ALLOCATION_DEBT_PREFIX),
                        "invalid report debt retirement key"
                    );
                    observe_active_input(|stats| stats.write_attempts += 1);
                    self.store.delete_setting(run_key)?;
                    observe_active_input(|stats| stats.write_commits += 1);
                }
            } else if active.allocated_seconds != cursor {
                let encoded = encode_active_value(
                    &allocation_run_view(active, cursor, deferred),
                    active_text_bytes(active)?,
                    0,
                )?;
                self.commit_active_maintenance(&[(run_key, encoded.as_str())], false)?;
            }
            active.allocated_seconds = cursor;
            return Ok(AllocationProgress {
                added: 0,
                cursor,
                complete: cursor == active.eligible_seconds,
            });
        };
        let mut removed = report_result_keys(&prepared.removed);
        let replacement = serde_json::to_string(&prepared.replacement)?;
        let run = if retire_at_target && prepared.complete {
            removed.push(run_key.to_owned());
            None
        } else {
            Some(serde_json::to_string(&allocation_run_view(
                active,
                prepared.cursor,
                deferred,
            ))?)
        };
        ensure!(
            replacement.len() + run.as_ref().map_or(0, String::len) == prepared.encoded_bytes,
            "report allocation saved encoding differs from its admitted format"
        );
        let mut values = vec![(OUTBOX_KEY, replacement.as_str())];
        if let Some(run) = &run {
            values.push((run_key, run.as_str()));
        }
        // Cursor evidence, compacted proof, and all seven side-key families
        // change together. A failed transaction publishes no allocated prefix.
        allocation_store_result(
            self.store
                .set_encoded_settings_and_delete(&values, &removed),
        )?;
        active.allocated_seconds = prepared.cursor;
        *queue = prepared.replacement;
        Ok(AllocationProgress {
            added: prepared.added,
            cursor: prepared.cursor,
            complete: prepared.complete,
        })
    }

    fn archive_report_run(&self, active: &ActiveRun, next: Option<&ActiveRun>) -> Result<()> {
        validate_active(active)?;
        let queue = self.load_report_outbox()?;
        validate_active_proof_identity(active, &queue)?;
        let cursor = queue.allocation_cursor(&active.id);
        ensure!(
            cursor >= active.allocated_seconds && cursor <= active.eligible_seconds,
            "closed report run has lost its allocation proof"
        );
        // Archive local observation once. Deferred allocation must never later
        // overwrite the larger local counter of an account-transition successor.
        self.persist_report_run_session(active)?;
        let debt_key = format!(
            "{ALLOCATION_DEBT_PREFIX}{}:{}",
            active.account_key, active.id
        );
        let debt = if cursor < active.eligible_seconds {
            ensure!(
                self.store.setting_encoded_bytes(&debt_key)?.is_none(),
                "deferred report run already exists"
            );
            Some(AllocationRunView::Deferred(ActiveCursorView::new(
                active, cursor,
            )))
        } else {
            None
        };
        let old_text = active_text_bytes(active)?;
        let old_metadata = active_metadata_bytes(active)?;
        let next_text = next.map(active_text_bytes).transpose()?.unwrap_or(0);
        let next_metadata = next.map(active_metadata_bytes).transpose()?.unwrap_or(0);
        let metadata = active_add(old_metadata, next_metadata)?;
        active_admit(
            "archive metadata",
            metadata,
            active_input_limits().metadata_bytes,
        )?;
        let debt_bytes = debt
            .as_ref()
            .map(|view| active_encoded_bytes(view, old_text))
            .transpose()?
            .unwrap_or(0);
        let successor_bytes = next
            .map(|view| active_encoded_bytes(view, next_text))
            .transpose()?
            .unwrap_or(0);
        let total_encoded = active_add(debt_bytes, successor_bytes)?;
        active_admit(
            "archive encoded output",
            total_encoded,
            active_input_limits().encoded_bytes,
        )?;
        active_admit(
            "archive/output scratch",
            active_add(metadata, total_encoded)?,
            active_input_limits().scratch_bytes,
        )?;
        let debt = debt
            .as_ref()
            .map(|view| encode_active_value(view, old_text, next_metadata))
            .transpose()?;
        let successor = next
            .map(|view| {
                encode_active_value(
                    view,
                    next_text,
                    active_add(old_metadata, debt.as_ref().map_or(0, String::len))?,
                )
            })
            .transpose()?;
        let mut values = Vec::with_capacity(2);
        if let Some(debt) = &debt {
            values.push((debt_key.as_str(), debt.as_str()));
        }
        if let Some(successor) = &successor {
            values.push((ACTIVE_KEY, successor.as_str()));
        }
        self.commit_active_maintenance(&values, next.is_none())
    }

    fn advance_deferred_report_allocation(&mut self) -> Result<usize> {
        let after = read_debt_marker(&self.store)?;
        let Some(key) = allocation_store_result(
            self.store
                .first_setting_key_with_prefix(ALLOCATION_DEBT_PREFIX, after.as_deref()),
        )?
        else {
            return Ok(0);
        };
        let result = (|| {
            let mut debt = read_active_run(&self.store, &key, true)?
                .ok_or_else(|| anyhow::anyhow!("deferred report run disappeared"))?;
            validate_active(&debt)?;
            ensure!(
                key == format!("{ALLOCATION_DEBT_PREFIX}{}:{}", debt.account_key, debt.id),
                "deferred report key changed its run identity"
            );
            let mut queue = self.load_report_outbox()?;
            self.flush_report_run(&mut debt, &mut queue, true, &key, true, true)
        })();
        // A bounded key marker rotates blocked runs without collecting all run
        // records or declaring a refused run empty. It is not allocation credit.
        if result.as_ref().err().is_none_or(report_capacity) {
            // This bounded selector marker is independent of a refused cold
            // run's larger field budget; it can still rotate to another run.
            ensure!(
                key.len() <= 4096 && key.starts_with(ALLOCATION_DEBT_PREFIX) && !key.contains('\0'),
                "invalid report debt marker"
            );
            active_admit(
                "debt marker encoding",
                active_mul(key.len(), 8)?,
                128 * 1024,
            )?;
            self.store.set_setting(ALLOCATION_DEBT_CURSOR_KEY, &key)?;
        }
        result.map(|progress| progress.added)
    }

    fn reconcile_report_results(&self, queue: &mut ReportOutbox) -> Result<bool> {
        if queue.batches.is_empty() {
            return Ok(false);
        }
        queue.admit_input(ReportInputPhase::Results, RESULT_READ_SCRATCH)?;
        let mut side_work = 0usize;
        let mut side_bytes = 0usize;
        for batch in &queue.batches {
            if batch.state == BatchState::Confirmed {
                continue;
            }
            for prefix in [
                "report.receipt",
                "report.uncertain",
                "report.failure",
                "report.error",
            ] {
                let key = format!("{prefix}:{}", batch.request.session_id);
                let bytes = self
                    .store
                    .setting_encoded_bytes(&key)
                    .map_err(report_input_store_error)?
                    .unwrap_or(0);
                side_bytes = side_bytes.checked_add(bytes).ok_or(ReportInputCapacity {
                    resource: "side-record metadata",
                })?;
                // Include conversion, JSON traversal, and a selected failure code
                // copy before opening any value-bearing side record.
                side_work = side_work
                    .checked_add(bytes.checked_mul(16).ok_or(ReportInputCapacity {
                        resource: "side-record work",
                    })?)
                    .ok_or(ReportInputCapacity {
                        resource: "side-record work",
                    })?;
            }
        }
        let scratch = RESULT_READ_SCRATCH
            .checked_add(side_bytes)
            .ok_or(ReportInputCapacity {
                resource: "side-record metadata",
            })?;
        queue.admit_input_with_work(ReportInputPhase::Results, scratch, side_work)?;
        let now = Utc::now().timestamp();
        let mut changed = false;
        for index in 0..queue.batches.len() {
            let batch = &queue.batches[index];
            if batch.state == BatchState::Confirmed {
                continue;
            }
            // Only the result lookup identity and numeric delivery controls
            // outlive this borrow. Position/summary and the full request stay
            // in the immutable queue until an admitted persistence operation.
            observe_report_input(|stats| {
                stats.result_headers += 1;
                stats.result_header_bytes +=
                    batch.request.session_id.len() + batch.request.account_key.len();
            });
            let id = batch.request.session_id.clone();
            let account_key = batch.request.account_key.clone();
            let state = batch.state;
            let attempted_at = batch.attempted_at;
            if let Some(receipt) = self
                .store
                .bounded_setting::<ReadReportReceipt>(
                    &format!("report.receipt:{id}"),
                    RESULT_RECORD_BYTES,
                )
                .map_err(report_input_store_error)?
            {
                queue.confirm(&id, &receipt)?;
                changed = true;
                self.store.set_setting(
                    &format!("report.last_success:{account_key}"),
                    &receipt.accepted_at,
                )?;
                continue;
            }
            if self
                .store
                .bounded_setting::<bool>(&format!("report.uncertain:{id}"), RESULT_RECORD_BYTES)
                .map_err(report_input_store_error)?
                .unwrap_or(false)
            {
                if state == BatchState::Sending {
                    queue.mark_uncertain(&id, "source.delivery_uncertain")?;
                    changed = true;
                }
                continue;
            }
            if state != BatchState::Sending {
                continue;
            }
            if let Some(failure) = self
                .store
                .bounded_setting::<ReadReportFailure>(
                    &format!("report.failure:{id}"),
                    RESULT_RECORD_BYTES,
                )
                .map_err(report_input_store_error)?
            {
                if failure.uncertain {
                    queue.mark_uncertain(&id, &failure.code)?;
                    changed = true;
                    continue;
                }
                queue.mark_safe_failure(
                    &id,
                    &failure.code,
                    now,
                    failure.retryable || failure.requires_login,
                )?;
                changed = true;
                if failure.requires_login {
                    if let Some(session) = self.jobs.session() {
                        self.store.set_setting(
                            &format!("report.await_auth:{id}"),
                            &auth_revision(&session)?,
                        )?;
                    }
                }
            } else if self
                .store
                .bounded_setting::<String>(&format!("report.error:{id}"), RESULT_RECORD_BYTES)
                .map_err(report_input_store_error)?
                .is_some()
            {
                queue.mark_uncertain(&id, "incomplete_delivery_result")?;
                changed = true;
            } else if attempted_at
                .is_some_and(|sent| now.saturating_sub(sent) >= DELIVERY_TIMEOUT_SECONDS)
            {
                queue.mark_uncertain(&id, "source.delivery_timeout")?;
                changed = true;
                self.store
                    .set_setting(&format!("report.uncertain:{id}"), &true)?;
            }
        }
        Ok(changed)
    }
}

fn active_metadata_bytes(active: &ActiveRun) -> Result<usize> {
    let mut bytes = std::mem::size_of::<ActiveRun>() * 2;
    for text in [
        &active.id,
        &active.account_key,
        &active.book_id,
        &active.local_session_id,
    ] {
        bytes = bytes
            .checked_add(text.len().checked_mul(2).ok_or(AllocationCapacity {
                resource: "active metadata",
            })?)
            .ok_or(AllocationCapacity {
                resource: "active metadata",
            })?;
    }
    if let Some(position) = &active.position {
        for text in [&position.chapter_uid, &position.summary, &position.device] {
            bytes = bytes
                .checked_add(text.len().checked_mul(2).ok_or(AllocationCapacity {
                    resource: "active metadata",
                })?)
                .ok_or(AllocationCapacity {
                    resource: "active metadata",
                })?;
        }
    }
    Ok(bytes)
}

fn allocation_store_result<T>(result: Result<T>) -> Result<T> {
    result.map_err(|error| {
        if error
            .downcast_ref::<crate::store::CachedResourceLimit>()
            .is_some()
        {
            AllocationCapacity {
                resource: "Store working set",
            }
            .into()
        } else {
            error
        }
    })
}

fn report_input_store_error(error: anyhow::Error) -> anyhow::Error {
    if error
        .downcast_ref::<crate::store::CachedResourceLimit>()
        .is_some()
        || error
            .downcast_ref::<crate::store::ReportBatchLookupLimit>()
            .is_some()
    {
        ReportInputCapacity {
            resource: "encoded snapshot",
        }
        .into()
    } else {
        error
    }
}

fn load_report_input(store: &Store) -> Result<ReportOutbox> {
    let queue: ReportOutbox = store
        .bounded_setting(OUTBOX_KEY, report_settlement_limits().encoded_bytes)
        .map_err(report_input_store_error)?
        .unwrap_or_default();
    queue.validate_loaded_input()?;
    Ok(queue)
}

pub(super) fn report_capacity(error: &anyhow::Error) -> bool {
    error.downcast_ref::<AllocationCapacity>().is_some()
        || error.downcast_ref::<ReportInputCapacity>().is_some()
        || error.downcast_ref::<ActiveInputCapacity>().is_some()
}

fn allocation_run_view(active: &ActiveRun, cursor: u64, deferred: bool) -> AllocationRunView<'_> {
    let view = ActiveCursorView::new(active, cursor);
    if deferred {
        AllocationRunView::Deferred(view)
    } else {
        AllocationRunView::Active(view)
    }
}

fn report_result_keys(removed: &[String]) -> Vec<String> {
    removed
        .iter()
        .flat_map(|id| {
            RESULT_PREFIXES
                .iter()
                .map(move |prefix| format!("{prefix}:{id}"))
        })
        .collect()
}

fn persist_report_outbox(store: &Store, queue: &mut ReportOutbox) -> Result<()> {
    let (replacement, removed) = queue.compacted()?;
    let keys = report_result_keys(&removed);
    let encoded = replacement.encode_persistence()?;
    // The compressed proof and receipt cleanup are one durable state change.
    // Keep both the caller's snapshot and all existing proof on commit failure.
    // Existing delivery evidence uses its independently admitted maintenance
    // writer; refusing a new allocation cannot prevent durable Sending/results.
    store
        .set_report_outbox_encoded_and_delete(encoded.as_str(), &keys)
        .map_err(report_input_store_error)?;
    *queue = replacement;
    Ok(())
}

pub(super) fn store_report_result(
    store: &Store,
    session_id: &str,
    result: &source::Result<ReadReportReceipt>,
) -> Result<bool> {
    // Preserve a known actual response through a small independently admitted
    // delivery-state projection. Full proof validation remains mandatory before
    // reconciliation can confirm or compact; resource refusal is not evidence
    // that elapsed time can be replayed.
    let state = store
        .report_batch_delivery_state(OUTBOX_KEY, session_id)
        .map_err(report_input_store_error)?
        .map(|metadata| metadata.state);
    if !matches!(
        state,
        Some(
            crate::store::ReportBatchDeliveryState::Sending
                | crate::store::ReportBatchDeliveryState::Uncertain
        )
    ) {
        // No response can be attached to these records. Preserve the original
        // invariant error for a damaged nonmember/compacted snapshot without
        // placing this heavy proof gate before a known attempted response.
        load_report_input(store)?;
        return Ok(false);
    }
    match result {
        Ok(receipt) => {
            store.set_setting(&format!("report.receipt:{session_id}"), receipt)?;
            store.set_setting("report.last_success", &receipt.accepted_at)?;
            for prefix in [
                "report.failure",
                "report.error",
                "report.retryable",
                "report.requires_login",
                "report.uncertain",
            ] {
                store.delete_setting(&format!("{prefix}:{session_id}"))?;
            }
        }
        Err(error) => {
            store.set_setting(
                &format!("report.failure:{session_id}"),
                &ReadReportFailure::from(error),
            )?;
            store.set_setting(&format!("report.error:{session_id}"), &error.to_string())?;
            store.set_setting(
                &format!("report.retryable:{session_id}"),
                &error.retryable(),
            )?;
            store.set_setting(
                &format!("report.requires_login:{session_id}"),
                &error.requires_login(),
            )?;
            if matches!(error, source::SourceError::DeliveryUncertain) {
                store.set_setting(&format!("report.uncertain:{session_id}"), &true)?;
            }
        }
    }
    Ok(true)
}

fn validate_active_proof_identity(active: &ActiveRun, queue: &ReportOutbox) -> Result<()> {
    // A zero allocation demand is idempotent only for the same measured run.
    // Retained intervals still bind its account, book and original start time.
    queue.admit_input(ReportInputPhase::Validation, 0)?;
    queue.ensure_run_identity(
        &active.id,
        &active.account_key,
        &active.book_id,
        active.started_at,
    )
}

fn validate_active(active: &ActiveRun) -> Result<()> {
    ensure!(
        !active.id.is_empty() && active.id.len() <= 256,
        "invalid active reading identity"
    );
    ensure!(
        !active.book_id.is_empty() && active.book_id.len() <= 2048,
        "invalid active reading book"
    );
    ensure!(
        active.account_key == UNATTRIBUTED_ACCOUNT
            || active.account_key.len() == 64
                && active
                    .account_key
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
        "invalid active reading account"
    );
    ensure!(
        active.started_at > 0 && active.last_checkpoint_at >= active.started_at,
        "invalid active reading timestamps"
    );
    ensure!(
        active.reader_started_at() > 0
            && active.reader_started_at() <= active.started_at
            && active.observer_seconds_base
                <= active
                    .last_checkpoint_at
                    .saturating_sub(active.reader_started_at()) as u64
            && (active.local_session_id.is_empty() || active.local_session_id.len() <= 256),
        "invalid active reading observation baseline"
    );
    ensure!(
        active.observed_seconds
            <= active.last_checkpoint_at.saturating_sub(active.started_at) as u64
            && active.eligible_seconds <= active.observed_seconds
            && active.allocated_seconds <= active.eligible_seconds,
        "active reading counter exceeds measured elapsed time"
    );
    Ok(())
}

fn observe_active(
    active: &mut ActiveRun,
    observed: u64,
    now: i64,
    report_enabled: bool,
) -> Result<()> {
    validate_active(active)?;
    ensure!(
        observed >= active.observed_seconds,
        "active reading seconds moved backwards"
    );
    ensure!(
        now >= active.started_at && observed <= now.saturating_sub(active.started_at) as u64,
        "active reading exceeds elapsed wall time"
    );
    let delta = observed - active.observed_seconds;
    if report_enabled {
        active.eligible_seconds = active
            .eligible_seconds
            .checked_add(delta)
            .ok_or_else(|| anyhow::anyhow!("reading report duration overflow"))?;
    }
    active.observed_seconds = observed;
    active.last_checkpoint_at = now;
    validate_active(active)
}

#[cfg(test)]
fn allocate_batches(
    active: &ActiveRun,
    queue: &mut ReportOutbox,
    flush: bool,
    progress_write_allowed: bool,
) -> Result<bool> {
    validate_active(active)?;
    validate_active_proof_identity(active, queue)?;
    let cursor = queue.allocation_cursor(&active.id);
    ensure!(
        cursor <= active.eligible_seconds,
        "allocated reading time exceeds observed eligibility"
    );
    let target = if flush {
        active.eligible_seconds
    } else {
        cursor + (active.eligible_seconds - cursor) / BATCH_SECONDS * BATCH_SECONDS
    };
    let Some(mut prepared) = queue.prepare_allocation(
        AllocationInput {
            run_id: &active.id,
            account_key: &active.account_key,
            book_id: &active.book_id,
            started_at: active.started_at,
            allocated_seconds: active.allocated_seconds,
            target_seconds: target,
            position: active.position.as_ref(),
            progress_write_allowed,
        },
        AllocationExtra {
            metadata_bytes: 0,
            encoded_bytes: 0,
            cursor: 0,
            fixed_encoded_bytes: 0,
            omit_at_target: true,
        },
    )?
    else {
        return Ok(false);
    };
    // Legacy unit callers persist with persist_report_outbox afterwards. Keep
    // their confirmed records until that atomic side-key cleanup can consume
    // them, while still exercising the actual admitted prefix producer.
    let start = prepared.replacement.batches.len() - prepared.added;
    let suffix = prepared.replacement.batches.split_off(start);
    if !queue.reported_cursors.contains_key(&active.id) {
        let (id, cursor) = prepared
            .replacement
            .reported_cursors
            .remove_entry(&active.id)
            .ok_or_else(|| anyhow::anyhow!("new report cursor disappeared"))?;
        queue.reported_cursors.insert(id, cursor);
    }
    queue.batches.extend(suffix);
    queue.validate()?;
    Ok(true)
}

fn auth_revision(session: &source::AuthSession) -> Result<String> {
    // Store only a one-way revision, never credential values in outbox metadata.
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(session)?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovered_activity_never_uses_restart_wall_time_as_reading_time() {
        let active = ActiveRun {
            id: "fixture-run".into(),
            account_key: UNATTRIBUTED_ACCOUNT.into(),
            book_id: "fixture-book".into(),
            started_at: 100,
            observed_seconds: 60,
            eligible_seconds: 45,
            allocated_seconds: 0,
            last_checkpoint_at: 1000,
            local_chapter: 0,
            local_anchor: 0,
            position: None,
            ..Default::default()
        };
        assert!(validate_active(&active).is_ok());
        let mut impossible = active.clone();
        impossible.observed_seconds = 901;
        assert!(validate_active(&impossible).is_err());
        let mut fabricated = active;
        fabricated.eligible_seconds = 61;
        assert!(validate_active(&fabricated).is_err());
    }

    #[test]
    fn credential_revisions_change_without_persisting_the_credentials() {
        let mut session = source::AuthSession::default();
        session
            .cookies
            .insert("wr_skey".into(), "fixture-old-session".into());
        let first = auth_revision(&session).unwrap();
        session
            .cookies
            .insert("wr_skey".into(), "fixture-new-session".into());
        let second = auth_revision(&session).unwrap();
        assert_ne!(first, second);
        assert_eq!(first.len(), 64);
        assert!(!first.contains("fixture"));
    }

    #[test]
    fn disabling_reporting_does_not_credit_the_disabled_interval_later() {
        let mut active = ActiveRun {
            id: "fixture-run".into(),
            account_key: UNATTRIBUTED_ACCOUNT.into(),
            book_id: "fixture-book".into(),
            started_at: 100,
            observed_seconds: 0,
            eligible_seconds: 0,
            allocated_seconds: 0,
            last_checkpoint_at: 100,
            local_chapter: 0,
            local_anchor: 0,
            position: None,
            ..Default::default()
        };
        observe_active(&mut active, 5, 105, true).unwrap();
        observe_active(&mut active, 10, 110, false).unwrap();
        observe_active(&mut active, 15, 115, true).unwrap();
        assert_eq!(active.observed_seconds, 15);
        assert_eq!(active.eligible_seconds, 10);
        observe_active(&mut active, 15, 1000, true).unwrap();
        assert_eq!(active.eligible_seconds, 10);
    }

    #[test]
    fn allocation_survives_a_crash_before_the_active_cursor_checkpoint() {
        let active = ActiveRun {
            id: "fixture-run".into(),
            account_key: UNATTRIBUTED_ACCOUNT.into(),
            book_id: "fixture-book".into(),
            started_at: 100,
            observed_seconds: 65,
            eligible_seconds: 65,
            allocated_seconds: 0,
            last_checkpoint_at: 165,
            local_chapter: 0,
            local_anchor: 0,
            position: None,
            ..Default::default()
        };
        let mut queue = ReportOutbox::default();
        assert!(allocate_batches(&active, &mut queue, false, false).unwrap());
        assert_eq!(queue.batches.len(), 1);
        assert_eq!(queue.batches[0].request.seconds, 60);
        assert!(!allocate_batches(&active, &mut queue, false, true).unwrap());
        assert!(allocate_batches(&active, &mut queue, true, true).unwrap());
        assert_eq!(queue.batches.len(), 2);
        assert_eq!(queue.batches[1].from, 60);
        assert_eq!(queue.batches[1].to, 65);
        assert!(!allocate_batches(&active, &mut queue, true, true).unwrap());
        assert_eq!(queue.reported_cursors["fixture-run"], 0);
        queue.validate().unwrap();
    }

    #[test]
    fn missing_allocation_history_cannot_reissue_previously_allocated_seconds() {
        let active = ActiveRun {
            id: "fixture-run".into(),
            account_key: UNATTRIBUTED_ACCOUNT.into(),
            book_id: "fixture-book".into(),
            started_at: 100,
            observed_seconds: 65,
            eligible_seconds: 65,
            allocated_seconds: 60,
            last_checkpoint_at: 165,
            local_chapter: 0,
            local_anchor: 0,
            position: None,
            ..Default::default()
        };
        let mut missing = ReportOutbox::default();
        assert!(allocate_batches(&active, &mut missing, true, true).is_err());
        assert!(missing.batches.is_empty());
    }

    fn active_fixture(seconds: u64) -> ActiveRun {
        ActiveRun {
            id: "fixture-run".into(),
            account_key: UNATTRIBUTED_ACCOUNT.into(),
            book_id: "fixture-book".into(),
            started_at: 100,
            observed_seconds: seconds,
            eligible_seconds: seconds,
            allocated_seconds: 0,
            last_checkpoint_at: 100 + seconds as i64,
            local_chapter: 0,
            local_anchor: 0,
            position: None,
            ..Default::default()
        }
    }

    fn receipt_fixture(seconds: u32) -> ReadReportReceipt {
        ReadReportReceipt {
            accepted_at: 200,
            seconds,
            has_synckey: false,
        }
    }

    #[test]
    fn repeated_confirmed_checkpoints_remove_all_per_batch_side_records() -> Result<()> {
        let store = Store::open(":memory:")?;
        let mut queue = ReportOutbox::default();
        let mut active = active_fixture(0);
        for minute in 1..=1_024 {
            active.observed_seconds = minute * 60;
            active.eligible_seconds = active.observed_seconds;
            active.last_checkpoint_at = 100 + active.observed_seconds as i64;
            assert!(allocate_batches(&active, &mut queue, false, false)?);
            assert_eq!(queue.batches.len(), 1);
            let id = queue.batches[0].request.session_id.clone();
            queue.mark_sending(&id, 200)?;
            for prefix in RESULT_PREFIXES {
                store.set_setting(&format!("{prefix}:{id}"), &7_u32)?;
            }
            queue.confirm(&id, &receipt_fixture(60))?;
            persist_report_outbox(&store, &mut queue)?;
            active.allocated_seconds = queue.allocation_cursor(&active.id);
            assert_eq!(active.allocated_seconds, active.eligible_seconds);
            assert!(queue.batches.is_empty());
            assert_eq!(queue.compacted_runs.len(), 1);
            for prefix in RESULT_PREFIXES {
                assert!(
                    store
                        .settings_with_prefix::<serde_json::Value>(&format!("{prefix}:"))?
                        .is_empty()
                );
            }
            assert!(serde_json::to_vec(&queue)?.len() < 1_024);
        }
        Ok(())
    }

    #[test]
    fn compacted_proof_survives_an_old_active_cursor_and_rejects_identity_changes() -> Result<()> {
        let store = Store::open(":memory:")?;
        let active = active_fixture(65);
        let mut queue = ReportOutbox::default();
        allocate_batches(&active, &mut queue, false, false)?;
        let first = queue.batches[0].request.session_id.clone();
        queue.mark_sending(&first, 200)?;
        queue.confirm(&first, &receipt_fixture(60))?;
        persist_report_outbox(&store, &mut queue)?;
        queue = store.setting(OUTBOX_KEY)?.unwrap();
        assert_eq!(queue.allocation_cursor(&active.id), 60);
        assert!(!allocate_batches(&active, &mut queue, false, true)?);
        assert!(allocate_batches(&active, &mut queue, true, true)?);
        assert_eq!(queue.batches.len(), 1);
        assert_eq!((queue.batches[0].from, queue.batches[0].to), (60, 65));
        let last = queue.batches[0].request.session_id.clone();
        queue.mark_sending(&last, 200)?;
        queue.confirm(&last, &receipt_fixture(5))?;
        persist_report_outbox(&store, &mut queue)?;
        assert!(!allocate_batches(&active, &mut queue, true, true)?);
        assert!(queue.batches.is_empty());
        for field in ["account", "book", "start"] {
            let mut changed = active.clone();
            match field {
                "account" => changed.account_key = "a".repeat(64),
                "book" => changed.book_id = "other-book".into(),
                "start" => {
                    changed.started_at += 1;
                    changed.last_checkpoint_at += 1;
                }
                _ => unreachable!(),
            }
            assert!(allocate_batches(&changed, &mut queue, true, true).is_err());
        }
        Ok(())
    }

    #[test]
    fn late_or_unknown_results_cannot_recreate_compacted_side_records() -> Result<()> {
        let store = Store::open(":memory:")?;
        let mut queue = ReportOutbox::default();
        allocate_batches(&active_fixture(60), &mut queue, false, true)?;
        let id = queue.batches[0].request.session_id.clone();
        queue.mark_sending(&id, 200)?;
        persist_report_outbox(&store, &mut queue)?;
        assert!(store_report_result(&store, &id, &Ok(receipt_fixture(60)))?);
        queue.confirm(&id, &receipt_fixture(60))?;
        persist_report_outbox(&store, &mut queue)?;
        let before = serde_json::to_value(&queue)?;
        assert!(!store_report_result(&store, &id, &Ok(receipt_fixture(60)))?);
        assert!(!store_report_result(
            &store,
            &id,
            &Err(source::SourceError::Network)
        )?);
        assert!(!store_report_result(
            &store,
            &id,
            &Err(source::SourceError::DeliveryUncertain)
        )?);
        assert!(!store_report_result(
            &store,
            "unknown-session",
            &Ok(receipt_fixture(60))
        )?);
        for prefix in RESULT_PREFIXES {
            assert!(
                store
                    .settings_with_prefix::<serde_json::Value>(&format!("{prefix}:"))?
                    .is_empty()
            );
        }
        assert_eq!(
            store.setting::<serde_json::Value>(OUTBOX_KEY)?.unwrap(),
            before
        );
        let mut corrupt = queue;
        corrupt.compacted_runs.clear();
        store.set_setting(OUTBOX_KEY, &corrupt)?;
        assert!(store_report_result(&store, &id, &Ok(receipt_fixture(60))).is_err());
        assert!(
            store
                .setting::<serde_json::Value>(&format!("report.receipt:{id}"))?
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn failed_compaction_transaction_preserves_the_queue_and_every_side_record() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("reports.sqlite3");
        let store = Store::open(&path)?;
        let mut queue = ReportOutbox::default();
        allocate_batches(&active_fixture(120), &mut queue, false, true)?;
        let gap = queue.batches[0].request.session_id.clone();
        queue.mark_sending(&gap, 200)?;
        queue.mark_uncertain(&gap, "fixture_uncertain")?;
        let id = queue.batches[1].request.session_id.clone();
        queue.mark_sending(&id, 200)?;
        queue.confirm(&id, &receipt_fixture(60))?;
        store.set_setting(OUTBOX_KEY, &queue)?;
        for prefix in RESULT_PREFIXES {
            store.set_setting(&format!("{prefix}:{id}"), &7_u32)?;
        }
        let original = serde_json::to_value(&queue)?;
        let fault = rusqlite::Connection::open(&path)?;
        fault.execute_batch("CREATE TRIGGER reject_report_compaction BEFORE DELETE ON settings WHEN OLD.key GLOB 'report.error:*' BEGIN SELECT RAISE(FAIL,'fixture failure'); END")?;
        assert!(persist_report_outbox(&store, &mut queue).is_err());
        assert_eq!(serde_json::to_value(&queue)?, original);
        assert_eq!(
            store.setting::<serde_json::Value>(OUTBOX_KEY)?.unwrap(),
            original
        );
        for prefix in RESULT_PREFIXES {
            assert_eq!(store.setting::<u32>(&format!("{prefix}:{id}"))?, Some(7));
        }
        fault.execute_batch("DROP TRIGGER reject_report_compaction")?;
        persist_report_outbox(&store, &mut queue)?;
        assert_eq!(queue.batches.len(), 1);
        assert_eq!(queue.batches[0].state, BatchState::Uncertain);
        assert_eq!(queue.compacted_until("fixture-run"), 0);
        assert_eq!(queue.allocation_cursor("fixture-run"), 120);
        for prefix in RESULT_PREFIXES {
            assert!(store.setting::<u32>(&format!("{prefix}:{id}"))?.is_none());
        }
        Ok(())
    }

    #[test]
    fn a_permanent_uncertain_gap_cannot_retain_an_unbounded_confirmed_suffix() -> Result<()> {
        let store = Store::open(":memory:")?;
        let mut queue = ReportOutbox::default();
        let mut active = active_fixture(60);
        allocate_batches(&active, &mut queue, false, true)?;
        let gap = queue.batches[0].request.session_id.clone();
        queue.mark_sending(&gap, 200)?;
        queue.mark_uncertain(&gap, "fixture_uncertain")?;
        persist_report_outbox(&store, &mut queue)?;
        assert!(store_report_result(
            &store,
            &gap,
            &Err(source::SourceError::DeliveryUncertain)
        )?);
        for minute in 1..=2_048 {
            active.observed_seconds = (minute + 1) * 60;
            active.eligible_seconds = active.observed_seconds;
            active.last_checkpoint_at = 100 + active.observed_seconds as i64;
            assert!(allocate_batches(&active, &mut queue, false, true)?);
            assert_eq!(queue.batches.len(), 2);
            let id = queue.batches[1].request.session_id.clone();
            queue.mark_sending(&id, 200)?;
            queue.confirm(&id, &receipt_fixture(60))?;
            for prefix in RESULT_PREFIXES {
                store.set_setting(&format!("{prefix}:{id}"), &7_u32)?;
            }
            persist_report_outbox(&store, &mut queue)?;
            active.allocated_seconds = queue.allocation_cursor(&active.id);
            assert_eq!(queue.batches.len(), 1);
            assert_eq!(queue.batches[0].request.session_id, gap);
            assert_eq!(queue.batches[0].state, BatchState::Uncertain);
            let proof = &queue.compacted_runs[&active.id];
            assert_eq!(proof.confirmed_until, 0);
            assert_eq!(proof.confirmed_ranges.len(), 1);
            assert_eq!(proof.confirmed_ranges[0].from, 60);
            assert_eq!(proof.confirmed_ranges[0].to, active.eligible_seconds);
            assert_eq!(queue.reported_cursors[&active.id], 0);
            assert_eq!(active.allocated_seconds, active.eligible_seconds);
            assert!(serde_json::to_vec(&queue)?.len() < 2_048);
            for prefix in RESULT_PREFIXES {
                let records =
                    store.settings_with_prefix::<serde_json::Value>(&format!("{prefix}:"))?;
                assert!(records.len() <= 1);
                assert!(
                    records
                        .iter()
                        .all(|(key, _)| key == &format!("{prefix}:{gap}"))
                );
            }
            assert_eq!(
                store.setting::<bool>(&format!("report.uncertain:{gap}"))?,
                Some(true)
            );
        }
        assert!(store_report_result(&store, &gap, &Ok(receipt_fixture(60)))?);
        queue.confirm(&gap, &receipt_fixture(60))?;
        assert_eq!(queue.reported_cursors[&active.id], active.eligible_seconds);
        persist_report_outbox(&store, &mut queue)?;
        assert!(queue.batches.is_empty());
        assert_eq!(queue.compacted_until(&active.id), active.eligible_seconds);
        assert!(queue.compacted_runs[&active.id].confirmed_ranges.is_empty());
        for prefix in RESULT_PREFIXES {
            assert!(
                store
                    .settings_with_prefix::<serde_json::Value>(&format!("{prefix}:"))?
                    .is_empty()
            );
        }
        Ok(())
    }

    #[test]
    fn suffix_proof_survives_a_crash_with_an_old_active_allocation_cursor() -> Result<()> {
        let store = Store::open(":memory:")?;
        let mut queue = ReportOutbox::default();
        allocate_batches(&active_fixture(180), &mut queue, false, true)?;
        let gap = queue.batches[0].request.session_id.clone();
        queue.mark_sending(&gap, 200)?;
        queue.mark_uncertain(&gap, "fixture_uncertain")?;
        for index in [1, 2] {
            let id = queue.batches[index].request.session_id.clone();
            queue.mark_sending(&id, 200)?;
            queue.confirm(&id, &receipt_fixture(60))?;
        }
        persist_report_outbox(&store, &mut queue)?;
        let mut restored: ReportOutbox = store.setting(OUTBOX_KEY)?.unwrap();
        assert_eq!(restored.reported_cursors["fixture-run"], 0);
        assert_eq!(restored.allocation_cursor("fixture-run"), 180);
        let old_active = active_fixture(180);
        assert_eq!(old_active.allocated_seconds, 0);
        assert!(!allocate_batches(&old_active, &mut restored, true, true)?);
        assert_eq!(restored.batches.len(), 1);
        let next_active = active_fixture(240);
        assert!(allocate_batches(&next_active, &mut restored, false, true)?);
        assert_eq!(restored.batches.len(), 2);
        assert_eq!(
            (restored.batches[1].from, restored.batches[1].to),
            (180, 240)
        );
        let mut wrong_identity = next_active;
        wrong_identity.book_id = "other-book".into();
        assert!(allocate_batches(&wrong_identity, &mut restored, false, true).is_err());
        Ok(())
    }
}
