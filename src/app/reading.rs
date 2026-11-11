use super::underline_projection::{ProjectedUnderline, ProjectionKey};
use super::*;
use crate::input::InputPurpose;
use crate::reader::{CaretBudget, CaretGeometry, Layout, LayoutBudget, Line, Page};
use crate::store::{
    AnnotationFieldState, AnnotationGeometry, AnnotationHeader, AnnotationLimits,
    AnnotationResourceLimit, AnnotationUnavailable, BookmarkUnavailable, ChapterAccessMetadata,
};
use std::collections::HashMap;

pub(super) struct PreparedReaderLoad {
    book: BookMetadata,
    index: u32,
    body: String,
    preferences: ReaderPreferences,
    paginator: Option<Paginator>,
    changed_book: bool,
    changed_chapter: bool,
    reopening: bool,
    consume_pending_position: bool,
    locked: bool,
}

impl App {
    pub(super) fn pending_reading_position(
        &self,
        book_id: &str,
    ) -> Result<Option<source::ReadingPosition>> {
        self.store.with_bounded_setting_text(
            &format!("progress.pending:{book_id}"),
            crate::store::StoredSettingReadLimits {
                json_passes: 2,
                ..Default::default()
            },
            |raw, _metadata| {
                source::decode_reading_position_setting(raw).map_err(anyhow::Error::from)
            },
        )
    }
}

#[cfg(test)]
#[path = "reading_work_tests.rs"]
mod work_tests;

const READER_HOLD_DELAY: Duration = Duration::from_millis(501);
const READER_PREVIEW_DELAY: Duration = Duration::from_millis(80);
const READER_HOLD_MOVEMENT: f32 = 12.0;
const READER_ACCESS_NOTICE_HEIGHT: f32 = 148.0;
const READER_ACCESS_NOTICE_GAP: f32 = 20.0;
// Application encoded-cache allowance; this accommodates ordinary JSON
// escaping/framing without treating every bounded HTTP response as UI data.
const UNDERLINE_CACHE_BYTES: usize = 16 * 1024 * 1024;
const UNDERLINE_CACHE_RECORDS: usize = 4096;
const BOOK_WEIGHT_METADATA_BYTES: usize = 64 * 1024 * 1024;
const BOOK_WEIGHT_ENTRY_BYTES: usize = 128;
const SELECTION_WORK_BYTES: usize = 64 * 1024 * 1024;
const SELECTION_PREVIEW_BYTES: usize = 16 * 1024;
const SELECTION_LINE_BYTES: usize = 16 * 1024;
const SELECTION_LINE_SCALARS: usize = 8192;
const MAX_PAGE_MARKS: usize = 1024;
const MARK_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
const MARK_OUTPUT_WORK: usize = 64 * 1024 * 1024;
const MARK_RECTANGLE_BYTES: usize = 64;
const MARK_ID_BYTES: usize = 29;

#[derive(Clone, Copy)]
pub(super) struct MarkOutputLimits {
    pub marks: usize,
    pub bytes: usize,
    pub work: usize,
}

impl Default for MarkOutputLimits {
    fn default() -> Self {
        Self {
            marks: MAX_PAGE_MARKS,
            bytes: MARK_OUTPUT_BYTES,
            work: MARK_OUTPUT_WORK,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("page mark {resource} exceeds its allowance (limit {limit}, requested {requested})")]
pub(super) struct MarkOutputLimit {
    pub resource: &'static str,
    pub limit: usize,
    pub requested: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("page mark source intervals are unavailable")]
struct MarkOutputUnavailable;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct MarkOutputStats {
    pub admissions: usize,
    pub endpoint_slots: usize,
    pub local_forecast: usize,
    pub remote_forecast: usize,
    pub selection_forecast: usize,
    pub bytes: usize,
    pub work: usize,
    pub local_outputs: usize,
    pub remote_outputs: usize,
    pub selection_outputs: usize,
    pub rejected: usize,
}

#[cfg(test)]
std::thread_local! {
    static MARK_OUTPUT_TEST_LIMITS: Cell<Option<MarkOutputLimits>> = const { Cell::new(None) };
}

#[cfg(test)]
pub(super) fn with_mark_output_test_limits<T>(
    limits: MarkOutputLimits,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<MarkOutputLimits>);
    impl Drop for Restore {
        fn drop(&mut self) {
            MARK_OUTPUT_TEST_LIMITS.with(|slot| slot.set(self.0));
        }
    }
    let _restore = Restore(MARK_OUTPUT_TEST_LIMITS.with(|slot| slot.replace(Some(limits))));
    run()
}

fn mark_output_limits() -> MarkOutputLimits {
    #[cfg(test)]
    if let Some(limits) = MARK_OUTPUT_TEST_LIMITS.with(Cell::get) {
        return limits;
    }
    MarkOutputLimits::default()
}

struct MarkOutputPlan {
    counts: [[usize; 3]; crate::reader::MAX_CARET_PAGE_LINES],
    local: usize,
    remote: usize,
    selection: usize,
    endpoint_slots: usize,
    bytes: usize,
    work: usize,
}

impl Default for MarkOutputPlan {
    fn default() -> Self {
        Self {
            counts: [[0; 3]; crate::reader::MAX_CARET_PAGE_LINES],
            local: 0,
            remote: 0,
            selection: 0,
            endpoint_slots: 0,
            bytes: 0,
            work: 0,
        }
    }
}

fn mark_output_add(
    left: usize,
    right: usize,
    resource: &'static str,
    limit: usize,
) -> Result<usize> {
    left.checked_add(right).ok_or_else(|| {
        MarkOutputLimit {
            resource,
            limit,
            requested: usize::MAX,
        }
        .into()
    })
}

fn mark_output_mul(
    left: usize,
    right: usize,
    resource: &'static str,
    limit: usize,
) -> Result<usize> {
    left.checked_mul(right).ok_or_else(|| {
        MarkOutputLimit {
            resource,
            limit,
            requested: usize::MAX,
        }
        .into()
    })
}

fn admit_mark_outputs(
    plan: &mut MarkOutputPlan,
    lines: &[Line],
    local: &[AnnotationGeometry],
    remote: &[ProjectedUnderline],
    selection: Option<(u32, u32)>,
    chapter: u32,
) -> Result<()> {
    let requested = mark_output_limits();
    let limits = MarkOutputLimits {
        marks: requested.marks.min(MAX_PAGE_MARKS),
        bytes: requested.bytes.min(MARK_OUTPUT_BYTES),
        work: requested.work.min(MARK_OUTPUT_WORK),
    };
    if lines.len() > crate::reader::MAX_CARET_PAGE_LINES
        || local.len() > 8192
        || remote.len() > MAX_PAGE_MARKS
    {
        return Err(MarkOutputUnavailable.into());
    }
    let input = mark_output_add(local.len(), remote.len(), "work bytes", limits.work)?;
    let input = mark_output_add(
        input,
        usize::from(selection.is_some()),
        "work bytes",
        limits.work,
    )?;
    if input == 0 {
        return Ok(());
    }
    // Prepay validation, source-kind filtering and line traversal before the
    // first metadata visit. These arrays contain only numeric endpoints.
    plan.work = mark_output_mul(
        mark_output_add(input, lines.len(), "work bytes", limits.work)?,
        128,
        "work bytes",
        limits.work,
    )?;
    if plan.work > limits.work {
        return Err(MarkOutputLimit {
            resource: "work bytes",
            limit: limits.work,
            requested: plan.work,
        }
        .into());
    }
    let mut previous = None;
    for line in lines {
        if line.start > line.end
            || previous.is_some_and(|(start, end)| line.start < start || line.end < end)
        {
            return Err(MarkOutputUnavailable.into());
        }
        previous = Some((line.start, line.end));
    }
    let local_count = local
        .iter()
        .filter(|note| note.chapter_index == chapter)
        .count();
    let count = mark_output_add(
        mark_output_add(local_count, remote.len(), "work bytes", limits.work)?,
        usize::from(selection.is_some()),
        "work bytes",
        limits.work,
    )?;
    let mut levels = 0usize;
    let mut remaining = count;
    while remaining > 1 {
        remaining = remaining / 2 + remaining % 2;
        levels += 1;
    }
    let sorting = mark_output_mul(
        mark_output_mul(count, levels.max(1), "work bytes", limits.work)?,
        128,
        "work bytes",
        limits.work,
    )?;
    plan.work = mark_output_add(plan.work, sorting, "work bytes", limits.work)?;
    // Endpoints, fixed per-page counts and both sorts are independently priced
    // before allocation. No annotation ID, quote or body participates.
    plan.endpoint_slots = mark_output_mul(count, 2, "logical bytes", limits.bytes)?;
    plan.bytes = mark_output_add(
        mark_output_mul(
            plan.endpoint_slots,
            std::mem::size_of::<(u32, usize)>(),
            "logical bytes",
            limits.bytes,
        )?,
        std::mem::size_of::<MarkOutputPlan>() + 2 * std::mem::size_of::<Vec<(u32, usize)>>(),
        "logical bytes",
        limits.bytes,
    )?;
    if plan.work > limits.work || plan.bytes > limits.bytes {
        let (resource, limit, requested) = if plan.work > limits.work {
            ("work bytes", limits.work, plan.work)
        } else {
            ("logical bytes", limits.bytes, plan.bytes)
        };
        return Err(MarkOutputLimit {
            resource,
            limit,
            requested,
        }
        .into());
    }
    let mut starts = Vec::with_capacity(count);
    let mut ends = Vec::with_capacity(count);
    let mut push = |start: u32, end: u32, kind: usize| -> Result<()> {
        if start >= end {
            return Err(MarkOutputUnavailable.into());
        }
        starts.push((start, kind));
        ends.push((end, kind));
        Ok(())
    };
    for note in local.iter().filter(|note| note.chapter_index == chapter) {
        push(note.start, note.end, 0)?;
    }
    for range in remote {
        push(range.start, range.end, 1)?;
    }
    if let Some((start, end)) = selection {
        push(start, end, 2)?;
    }
    drop(push);
    starts.sort_unstable_by_key(|entry| entry.0);
    ends.sort_unstable_by_key(|entry| entry.0);
    let mut active = [0usize; 3];
    let mut opened = 0;
    let mut closed = 0;
    for (index, line) in lines.iter().enumerate() {
        while opened < starts.len() && starts[opened].0 < line.end {
            let kind = starts[opened].1;
            active[kind] = active[kind].checked_add(1).ok_or(MarkOutputUnavailable)?;
            opened += 1;
        }
        while closed < ends.len() && ends[closed].0 <= line.start {
            let kind = ends[closed].1;
            active[kind] = active[kind].checked_sub(1).ok_or(MarkOutputUnavailable)?;
            closed += 1;
        }
        plan.counts[index] = active;
        plan.local = mark_output_add(plan.local, active[0], "mark count", limits.marks)?;
        plan.remote = mark_output_add(plan.remote, active[1], "mark count", limits.marks)?;
        plan.selection = mark_output_add(plan.selection, active[2], "mark count", limits.marks)?;
    }
    let marks = mark_output_add(
        mark_output_add(plan.local, plan.remote, "mark count", limits.marks)?,
        plan.selection,
        "mark count",
        limits.marks,
    )?;
    if marks > limits.marks {
        return Err(MarkOutputLimit {
            resource: "mark count",
            limit: limits.marks,
            requested: marks,
        }
        .into());
    }
    let models = plan.counts[..lines.len()]
        .iter()
        .filter(|counts| counts.iter().any(|count| *count > 0))
        .count();
    let model_bytes = mark_output_mul(models, 128, "logical bytes", limits.bytes)?;
    plan.bytes = mark_output_add(plan.bytes, model_bytes, "logical bytes", limits.bytes)?;
    plan.work = mark_output_add(plan.work, model_bytes, "work bytes", limits.work)?;
    for (kind, count) in [plan.local, plan.remote, plan.selection]
        .into_iter()
        .enumerate()
    {
        // Local marks paint one solid rectangle; selections add two anchors.
        // Only public marks create the bounded 128-dash component and label.
        let rectangles = [1usize, 130, 3][kind];
        let value = mark_output_add(
            3 * std::mem::size_of::<ReaderMark>(),
            3 * MARK_ID_BYTES,
            "logical bytes",
            limits.bytes,
        )?;
        let fee = mark_output_add(
            value,
            mark_output_mul(
                rectangles,
                MARK_RECTANGLE_BYTES,
                "logical bytes",
                limits.bytes,
            )?,
            "logical bytes",
            limits.bytes,
        )?;
        plan.bytes = mark_output_add(
            plan.bytes,
            mark_output_mul(count, fee, "logical bytes", limits.bytes)?,
            "logical bytes",
            limits.bytes,
        )?;
        plan.work = mark_output_add(
            plan.work,
            mark_output_mul(
                count,
                mark_output_add(fee, 128, "work bytes", limits.work)?,
                "work bytes",
                limits.work,
            )?,
            "work bytes",
            limits.work,
        )?;
    }
    if plan.bytes > limits.bytes || plan.work > limits.work {
        let (resource, limit, requested) = if plan.bytes > limits.bytes {
            ("logical bytes", limits.bytes, plan.bytes)
        } else {
            ("work bytes", limits.work, plan.work)
        };
        return Err(MarkOutputLimit {
            resource,
            limit,
            requested,
        }
        .into());
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) struct SelectionLimits {
    pub work_bytes: usize,
    pub preview_bytes: usize,
    pub line_bytes: usize,
    pub line_scalars: usize,
}

impl Default for SelectionLimits {
    fn default() -> Self {
        Self {
            work_bytes: SELECTION_WORK_BYTES,
            preview_bytes: SELECTION_PREVIEW_BYTES,
            line_bytes: SELECTION_LINE_BYTES,
            line_scalars: SELECTION_LINE_SCALARS,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("selection {resource} exceeds its allowance (limit {limit}, requested {requested})")]
pub(super) struct SelectionWorkLimit {
    pub resource: &'static str,
    pub limit: usize,
    pub requested: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("selection source anchors are unavailable")]
struct SelectionSourceUnavailable;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct SelectionWorkStats {
    pub source_scalar_visits: usize,
    pub source_bytes: usize,
    pub phrase_scalar_visits: usize,
    pub phrase_vectors: usize,
    pub phrase_copied_scalars: usize,
    pub preview_source_copies: usize,
    pub preview_source_bytes: usize,
    pub new_quote_source_copies: usize,
    pub work_bytes: usize,
    pub rejections: usize,
}

#[cfg(test)]
std::thread_local! {
    static SELECTION_TEST_LIMITS: Cell<Option<SelectionLimits>> = const { Cell::new(None) };
}

#[cfg(test)]
pub(super) fn with_selection_test_limits<T>(limits: SelectionLimits, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<SelectionLimits>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SELECTION_TEST_LIMITS.with(|current| current.set(self.0));
        }
    }
    let _restore = Restore(SELECTION_TEST_LIMITS.with(|current| current.replace(Some(limits))));
    run()
}

fn selection_limits() -> SelectionLimits {
    #[cfg(test)]
    if let Some(limits) = SELECTION_TEST_LIMITS.with(Cell::get) {
        return limits;
    }
    SelectionLimits::default()
}

struct SelectionOperation {
    limits: SelectionLimits,
    work_bytes: usize,
    #[cfg(test)]
    stats: SelectionWorkStats,
}

impl SelectionOperation {
    fn new() -> Self {
        let limits = selection_limits();
        Self {
            limits: SelectionLimits {
                work_bytes: limits.work_bytes.min(SELECTION_WORK_BYTES),
                preview_bytes: limits.preview_bytes.min(SELECTION_PREVIEW_BYTES),
                line_bytes: limits.line_bytes.min(SELECTION_LINE_BYTES),
                line_scalars: limits.line_scalars.min(SELECTION_LINE_SCALARS),
            },
            work_bytes: 0,
            #[cfg(test)]
            stats: SelectionWorkStats::default(),
        }
    }

    fn charge(&mut self, bytes: usize) -> Result<()> {
        let next = self
            .work_bytes
            .checked_add(bytes)
            .ok_or(SelectionWorkLimit {
                resource: "source work bytes",
                limit: self.limits.work_bytes,
                requested: usize::MAX,
            })?;
        if next > self.limits.work_bytes {
            return Err(SelectionWorkLimit {
                resource: "source work bytes",
                limit: self.limits.work_bytes,
                requested: next,
            }
            .into());
        }
        self.work_bytes = next;
        #[cfg(test)]
        {
            self.stats.work_bytes = next;
        }
        Ok(())
    }
}

fn selection_span<'a>(
    text: &'a str,
    start: u32,
    end: u32,
    maximum: usize,
    operation: &mut SelectionOperation,
) -> Result<&'a str> {
    let count = end.checked_sub(start).ok_or(SelectionSourceUnavailable)? as usize;
    if count > maximum {
        return Err(SelectionWorkLimit {
            resource: "preview minimum bytes",
            limit: maximum,
            requested: count,
        }
        .into());
    }
    if end as usize > text.len() {
        return Err(SelectionSourceUnavailable.into());
    }
    if start == 0 && end == 0 {
        return Ok("");
    }
    // UTF-8 has at most four bytes per scalar. Pay a conservative prefix and
    // endpoint lookup bound before scanning; no whole-body char Vec is made.
    let prefix = (end as usize)
        .checked_add(1)
        .and_then(|count| count.checked_mul(4))
        .ok_or(SelectionWorkLimit {
            resource: "source work bytes",
            limit: operation.limits.work_bytes,
            requested: usize::MAX,
        })?
        .min(text.len());
    operation.charge(prefix)?;
    let mut first = None;
    let mut scalar = 0usize;
    let mut last = None;
    for (offset, character) in text.char_indices() {
        #[cfg(test)]
        {
            operation.stats.source_scalar_visits += 1;
            operation.stats.source_bytes += character.len_utf8();
        }
        #[cfg(not(test))]
        let _ = character;
        if scalar == start as usize {
            first = Some(offset);
        }
        if scalar == end as usize {
            last = Some(offset);
            break;
        }
        scalar = scalar.checked_add(1).ok_or(SelectionSourceUnavailable)?;
    }
    if last.is_none() && scalar == end as usize {
        last = Some(text.len());
    }
    let (first, last) = (
        first.ok_or(SelectionSourceUnavailable)?,
        last.ok_or(SelectionSourceUnavailable)?,
    );
    let span = text.get(first..last).ok_or(SelectionSourceUnavailable)?;
    if span.len() > maximum {
        return Err(SelectionWorkLimit {
            resource: "preview UTF-8 bytes",
            limit: maximum,
            requested: span.len(),
        }
        .into());
    }
    Ok(span)
}

pub(super) struct LocalNoteEditContext {
    header: AnnotationHeader,
    book_id: String,
    chapter_id: String,
    body_revision: u64,
    account_epoch: u64,
}

#[cfg(test)]
std::thread_local! {
    static NOTE_HEADER_TEST_LIMITS: Cell<Option<AnnotationLimits>> = const { Cell::new(None) };
    static NOTE_RECORD_TEST_LIMITS: Cell<Option<AnnotationLimits>> = const { Cell::new(None) };
}

#[cfg(test)]
pub(super) fn with_note_read_test_limits<T>(
    headers: AnnotationLimits,
    records: AnnotationLimits,
    operation: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<AnnotationLimits>, Option<AnnotationLimits>);
    impl Drop for Restore {
        fn drop(&mut self) {
            NOTE_HEADER_TEST_LIMITS.with(|limits| limits.set(self.0));
            NOTE_RECORD_TEST_LIMITS.with(|limits| limits.set(self.1));
        }
    }
    let _restore = Restore(
        NOTE_HEADER_TEST_LIMITS.with(|limits| limits.replace(Some(headers))),
        NOTE_RECORD_TEST_LIMITS.with(|limits| limits.replace(Some(records))),
    );
    operation()
}

fn note_header_limits() -> AnnotationLimits {
    #[cfg(test)]
    if let Some(limits) = NOTE_HEADER_TEST_LIMITS.with(Cell::get) {
        return limits;
    }
    AnnotationLimits::headers()
}

fn note_record_limits() -> AnnotationLimits {
    #[cfg(test)]
    if let Some(limits) = NOTE_RECORD_TEST_LIMITS.with(Cell::get) {
        return limits;
    }
    AnnotationLimits::records()
}

fn selected_annotation_quote(
    text: &str,
    start: u32,
    end: u32,
    maximum: usize,
    operation: &mut SelectionOperation,
) -> Result<String> {
    let quote = match selection_span(text, start, end, maximum, operation) {
        Ok(quote) => quote,
        Err(error)
            if error
                .downcast_ref::<SelectionWorkLimit>()
                .is_some_and(|limit| {
                    matches!(
                        limit.resource,
                        "preview minimum bytes" | "preview UTF-8 bytes"
                    )
                }) =>
        {
            let requested = error
                .downcast_ref::<SelectionWorkLimit>()
                .unwrap()
                .requested;
            return Err(AnnotationResourceLimit {
                resource: "new quote bytes",
                limit: maximum,
                requested,
            }
            .into());
        }
        Err(error) => return Err(error),
    };
    if quote.trim().is_empty() {
        return Ok(String::new());
    }
    #[cfg(test)]
    {
        operation.stats.new_quote_source_copies += 1;
    }
    Ok(quote.to_owned())
}

pub(super) struct ThoughtRequestContext {
    request: u64,
    book_id: String,
    chapter_id: String,
    view_id: String,
    range: String,
    cache_key: String,
    body_revision: u64,
    underline_revision: u64,
    account_epoch: u64,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BookWeightWork {
    builds: usize,
    preflight_records: usize,
    uid_inserts: usize,
    chapter_lookups: usize,
    rejections: usize,
}

#[cfg(test)]
std::thread_local! {
    static BOOK_WEIGHT_WORK: Cell<BookWeightWork> = Cell::new(BookWeightWork::default());
    static BOOK_WEIGHT_ALLOWANCE: Cell<usize> = const { Cell::new(BOOK_WEIGHT_METADATA_BYTES) };
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ReaderPress {
    started: Instant,
    origin: (f32, f32),
    latest: (f32, f32),
    generation: i32,
    phase: ReaderPressPhase,
    preview_pending: bool,
}

#[derive(Clone, Copy, Debug)]
enum ReaderPressPhase {
    Pending,
    Rejected,
    Selecting {
        anchor: (u32, u32),
        initial: (u32, u32),
        dragged: bool,
    },
}

impl ReaderPress {
    fn new(x: f32, y: f32, generation: i32) -> Self {
        Self {
            started: Instant::now(),
            origin: (x, y),
            latest: (x, y),
            generation,
            phase: ReaderPressPhase::Pending,
            preview_pending: false,
        }
    }

    fn is_selecting(self) -> bool {
        matches!(self.phase, ReaderPressPhase::Selecting { .. })
    }

    fn exceeds_hold_movement(self, text_width: f32) -> bool {
        let dx = (self.latest.0 - self.origin.0) * text_width;
        let dy = self.latest.1 - self.origin.1;
        dx * dx + dy * dy > READER_HOLD_MOVEMENT * READER_HOLD_MOVEMENT
    }
}

impl App {
    fn page_mark_output_plan(
        &self,
        lines: &[Line],
        local: &[AnnotationGeometry],
        remote: &[ProjectedUnderline],
    ) -> Result<MarkOutputPlan> {
        let mut plan = MarkOutputPlan::default();
        let result = admit_mark_outputs(
            &mut plan,
            lines,
            local,
            remote,
            self.reader.selection,
            self.reader.chapter_index,
        );
        #[cfg(test)]
        {
            let mut stats = self.reader.mark_output_stats.get();
            stats.admissions += 1;
            stats.endpoint_slots += plan.endpoint_slots;
            stats.local_forecast += plan.local;
            stats.remote_forecast += plan.remote;
            stats.selection_forecast += plan.selection;
            stats.bytes += plan.bytes;
            stats.work += plan.work;
            stats.rejected += usize::from(result.is_err());
            self.reader.mark_output_stats.set(stats);
        }
        result.map(|()| plan)
    }

    fn record_mark_output(&self, kind: usize) {
        #[cfg(test)]
        {
            let mut stats = self.reader.mark_output_stats.get();
            match kind {
                0 => stats.local_outputs += 1,
                1 => stats.remote_outputs += 1,
                _ => stats.selection_outputs += 1,
            }
            self.reader.mark_output_stats.set(stats);
        }
        #[cfg(not(test))]
        let _ = kind;
    }

    fn record_selection_operation(&self, operation: &SelectionOperation, rejected: bool) {
        #[cfg(test)]
        {
            let next = operation.stats;
            let mut stats = self.reader.selection_work_stats.get();
            stats.source_scalar_visits += next.source_scalar_visits;
            stats.source_bytes += next.source_bytes;
            stats.phrase_scalar_visits += next.phrase_scalar_visits;
            stats.phrase_vectors += next.phrase_vectors;
            stats.phrase_copied_scalars += next.phrase_copied_scalars;
            stats.preview_source_copies += next.preview_source_copies;
            stats.preview_source_bytes += next.preview_source_bytes;
            stats.new_quote_source_copies += next.new_quote_source_copies;
            stats.work_bytes += next.work_bytes;
            stats.rejections += usize::from(rejected);
            self.reader.selection_work_stats.set(stats);
        }
        #[cfg(not(test))]
        let _ = (operation, rejected);
    }

    fn reader_selection_preview(&self, start: u32, end: u32) -> String {
        let mut operation = SelectionOperation::new();
        let result = selection_span(
            &self.reader.body,
            start,
            end,
            operation.limits.preview_bytes,
            &mut operation,
        )
        .map(|span| {
            #[cfg(test)]
            {
                operation.stats.preview_source_copies += 1;
                operation.stats.preview_source_bytes += span.len();
            }
            span.to_owned()
        });
        self.record_selection_operation(&operation, result.is_err());
        result.unwrap_or_else(|_| tr("api_error"))
    }

    fn reader_initial_range(&self, line: &Line, anchor: (u32, u32)) -> Result<(u32, u32)> {
        let mut operation = SelectionOperation::new();
        let result = initial_reader_range_from_line(line, anchor, &mut operation);
        self.record_selection_operation(&operation, result.is_err());
        result
    }

    fn local_note_edit_header(&self) -> Result<Option<AnnotationHeader>> {
        let Some(id) = self.reader.thought_id.strip_prefix("mine:") else {
            return Ok(None);
        };
        let context = self
            .reader
            .note_edit_context
            .as_ref()
            .ok_or(AnnotationUnavailable)?;
        if context.header.key.id != id
            || context.book_id != self.reader.book_id
            || context.header.chapter_index != self.reader.chapter_index
            || context.body_revision != self.reader.underline_body_revision
            || context.account_epoch != self.jobs.account_epoch()
            || self.reader.selection != Some((context.header.start, context.header.end))
            || self
                .store
                .chapter_uid(&context.book_id, context.header.chapter_index)?
                .as_deref()
                != Some(context.chapter_id.as_str())
        {
            return Err(AnnotationUnavailable.into());
        }
        Ok(Some(context.header.clone()))
    }

    pub(super) fn note_headers_page(
        &self,
        book: &str,
        offset: usize,
        rows: usize,
    ) -> Result<crate::store::AnnotationPage> {
        self.store
            .annotation_headers_page_with_limits(book, offset, rows, note_header_limits())
    }

    fn note_geometry(
        &self,
        book: &str,
        chapter: u32,
        start: u32,
        end: u32,
    ) -> Result<Vec<AnnotationGeometry>> {
        self.store.annotation_geometry_for_range_with_limits(
            book,
            chapter,
            start,
            end,
            note_header_limits(),
        )
    }

    /// Admit one complete visible selection before reading its first payload.
    /// Unavailable historical fields get the existing bounded placeholder; no
    /// rejected note text is copied merely to decide how it should be displayed.
    pub(super) fn note_display_records(
        &self,
        book: &str,
        headers: &[AnnotationHeader],
    ) -> Result<Vec<Annotation>> {
        self.note_display_records_with_limits(book, headers, note_record_limits())
    }

    fn note_display_records_with_limits(
        &self,
        book: &str,
        headers: &[AnnotationHeader],
        limits: AnnotationLimits,
    ) -> Result<Vec<Annotation>> {
        if headers.len() > limits.rows.min(16) {
            return Err(AnnotationResourceLimit {
                resource: "selected rows",
                limit: limits.rows.min(16),
                requested: headers.len(),
            }
            .into());
        }
        let cells = headers
            .len()
            .checked_mul(14)
            .ok_or(AnnotationResourceLimit {
                resource: "selected SQL cells",
                limit: limits.cells,
                requested: usize::MAX,
            })?;
        if cells > limits.cells {
            return Err(AnnotationResourceLimit {
                resource: "selected SQL cells",
                limit: limits.cells,
                requested: cells,
            }
            .into());
        }
        let unavailable = tr("api_error");
        let mut metadata = Vec::with_capacity(headers.len());
        let mut bytes = 0usize;
        for header in headers {
            let item = self
                .store
                .annotation_text_metadata_with_limits(book, &header.key, limits)?
                .ok_or(AnnotationUnavailable)?;
            if item.header != *header {
                return Err(AnnotationUnavailable.into());
            }
            let field_bytes = |field| match field {
                AnnotationFieldState::Available { utf8_bytes_upper } => utf8_bytes_upper,
                _ => unavailable.len(),
            };
            let next = bytes
                .checked_add(std::mem::size_of::<Annotation>())
                .and_then(|bytes| bytes.checked_add(header.key.id.len()))
                .and_then(|bytes| bytes.checked_add(book.len()))
                .and_then(|bytes| bytes.checked_add(field_bytes(item.quote)))
                .and_then(|bytes| bytes.checked_add(field_bytes(item.note)))
                .ok_or(AnnotationResourceLimit {
                    resource: "selected bytes",
                    limit: limits.bytes,
                    requested: usize::MAX,
                })?;
            if next > limits.bytes {
                return Err(AnnotationResourceLimit {
                    resource: "selected bytes",
                    limit: limits.bytes,
                    requested: next,
                }
                .into());
            }
            bytes = next;
            metadata.push(item);
        }
        let keys = metadata
            .iter()
            .filter(|item| {
                matches!(item.quote, AnnotationFieldState::Available { .. })
                    && matches!(item.note, AnnotationFieldState::Available { .. })
            })
            .map(|item| item.header.key.clone())
            .collect::<Vec<_>>();
        let mut available = self
            .store
            .annotation_records_with_limits(book, &keys, limits)?
            .into_iter();
        let mut output = Vec::with_capacity(headers.len());
        for item in metadata {
            let header = item.header;
            let both = matches!(item.quote, AnnotationFieldState::Available { .. })
                && matches!(item.note, AnnotationFieldState::Available { .. });
            let note = if both {
                available.next().ok_or(AnnotationUnavailable)?
            } else {
                Annotation {
                    id: header.key.id.clone(),
                    book_id: book.to_owned(),
                    chapter_index: header.chapter_index,
                    start: header.start,
                    end: header.end,
                    created_at: header.created_at,
                    quote: if matches!(item.quote, AnnotationFieldState::Available { .. }) {
                        self.store
                            .annotation_quote_by_key(book, &header.key, limits.quote_bytes)?
                            .ok_or(AnnotationUnavailable)?
                    } else {
                        unavailable.clone()
                    },
                    note: if matches!(item.note, AnnotationFieldState::Available { .. }) {
                        self.store
                            .annotation_note_by_key(book, &header.key, limits.note_bytes)?
                            .ok_or(AnnotationUnavailable)?
                    } else {
                        unavailable.clone()
                    },
                }
            };
            if note.id != header.key.id
                || note.book_id != book
                || note.chapter_index != header.chapter_index
                || note.start != header.start
                || note.end != header.end
                || note.created_at != header.created_at
            {
                return Err(AnnotationUnavailable.into());
            }
            output.push(note);
        }
        Ok(output)
    }

    pub(super) fn requested_thought_cache_key(
        &self,
        request: u64,
        book_id: &str,
        chapter_id: &str,
    ) -> Option<&str> {
        let context = self.reader.thought_context.as_ref()?;
        let index = context
            .view_id
            .strip_prefix("remote:")?
            .parse::<usize>()
            .ok()?;
        (context.request == request
            && self.reader.thought_request == request
            && context.book_id == book_id
            && self.reader.book_id == context.book_id
            && context.chapter_id == chapter_id
            && self.reader.thought_id == context.view_id
            && context.body_revision == self.reader.underline_body_revision
            && context.underline_revision == self.reader.underline_revision
            && context.account_epoch == self.jobs.account_epoch()
            && self
                .reader
                .underlines
                .get(index)
                .is_some_and(|underline| underline.range == context.range))
        .then_some(context.cache_key.as_str())
    }

    pub(super) fn load_reader(
        &mut self,
        book_id: &str,
        index: u32,
        anchor: Option<u32>,
    ) -> Result<()> {
        self.load_reader_inner(book_id, index, anchor, false)
    }

    pub(super) fn load_local_reader(
        &mut self,
        book_id: &str,
        index: u32,
        anchor: Option<u32>,
    ) -> Result<()> {
        self.load_reader_inner(book_id, index, anchor, true)
    }

    fn load_reader_inner(
        &mut self,
        book_id: &str,
        index: u32,
        anchor: Option<u32>,
        skip_pending_position: bool,
    ) -> Result<()> {
        let prepared =
            self.prepare_reader_load(book_id, index, anchor, skip_pending_position, false, false)?;
        self.install_reader_load(prepared)
    }

    pub(super) fn prepare_reader_load(
        &self,
        book_id: &str,
        index: u32,
        anchor: Option<u32>,
        skip_pending_position: bool,
        will_finish: bool,
        force_context_change: bool,
    ) -> Result<Option<PreparedReaderLoad>> {
        let Some(book) = self.store.book_metadata_by_id(book_id)? else {
            return Ok(None);
        };
        let changed_book = self.reader.book_id != book_id || force_context_change;
        let changed_chapter = changed_book || self.reader.chapter_index != index;
        let editing_thought = self.reader.selection.is_some()
            && self
                .ui
                .upgrade()
                .is_some_and(|ui| ui.global::<ReaderModel>().get_writing_thought());
        let reopening = (self.reader.session_started.is_none() || will_finish) && !editing_thought;
        let old_anchor = (!changed_chapter)
            .then(|| {
                self.reader
                    .paginator
                    .as_ref()
                    .map(|p| p.current_page().start)
            })
            .flatten();
        let mut preferences = if changed_book || reopening {
            let loaded = self
                .store
                .setting::<ReaderPreferences>(&format!("reader.preferences:{book_id}"))?
                .unwrap_or_else(|| self.prefs.reader.clone());
            let mut normalized = self.prefs.clone();
            normalized.reader = loaded;
            normalized.normalize_for_device(self.profile.is_scribe());
            normalized.reader
        } else {
            self.reader.preferences.clone()
        };
        let body = self.store.chapter_body(book_id, index)?.unwrap_or_default();
        let saved = self
            .store
            .position(book_id)?
            .filter(|position| position.chapter_index == index)
            .map(|position| position.char_offset);
        let mut target = anchor.or(old_anchor).or(saved).unwrap_or(0);
        let mut consume_pending_position = false;
        if !body.is_empty() && !skip_pending_position {
            if let Some(position) = self.pending_reading_position(book_id)? {
                let chapters = self.store.chapter_metadata(book_id)?;
                let catalog = self.remote_catalog(book_id)?;
                if cloud_chapter_index(&position, &catalog, &chapters) == Some(index)
                    && chapters
                        .iter()
                        .find(|chapter| chapter.index == index)
                        .is_some_and(|chapter| {
                            self.chapter_metadata_content_status(&book, chapter)
                                == ContentStatus::Full
                        })
                {
                    target = remote_to_local_offset(
                        position.chapter_offset,
                        self.chapter_word_count(book_id, index)?,
                        u32::try_from(body.chars().count()).map_err(|_| {
                            crate::reader::LayoutLimitError {
                                kind: crate::reader::LayoutLimitKind::Anchors,
                                used: 0,
                                requested: usize::MAX,
                                limit: u32::MAX as usize,
                            }
                        })?,
                    );
                    consume_pending_position = true;
                }
            }
        }
        let paginator = if body.is_empty() {
            None
        } else {
            let (paginator, font) = self.prepare_reader_pagination(
                book_id,
                index,
                &body,
                &preferences,
                !(changed_book || reopening) && self.reader.notes_panel,
                target,
                Some(&book.source),
            )?;
            preferences.font = font;
            Some(paginator)
        };
        let locked = self
            .store
            .chapter_access_metadata(book_id, index)?
            .is_some_and(|chapter| {
                self.chapter_access_content_status(&book.source, &chapter) == ContentStatus::Locked
            });
        Ok(Some(PreparedReaderLoad {
            book,
            index,
            body,
            preferences,
            paginator,
            changed_book,
            changed_chapter,
            reopening,
            consume_pending_position,
            locked,
        }))
    }

    /// Prepare against the original UID's body and picture evidence before a
    /// catalog relocation retires that original index. Only the destination
    /// index changes when the admitted result is installed after SQL commit.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_catalog_reader_load(
        &self,
        book_id: &str,
        target: &Chapter,
        source_index: Option<u32>,
        anchor: Option<u32>,
        force_context_change: bool,
        catalog: &[source::RemoteChapter],
        chapters: &[Chapter],
    ) -> Result<Option<PreparedReaderLoad>> {
        let mut prepared = if let Some(source_index) = source_index {
            let Some(prepared) = self.prepare_reader_load(
                book_id,
                source_index,
                anchor,
                true,
                false,
                force_context_change,
            )?
            else {
                return Ok(None);
            };
            prepared
        } else {
            let Some(book) = self.store.book_metadata_by_id(book_id)? else {
                return Ok(None);
            };
            let loaded = self
                .store
                .setting::<ReaderPreferences>(&format!("reader.preferences:{book_id}"))?
                .unwrap_or_else(|| self.prefs.reader.clone());
            let mut preferences = self.prefs.clone();
            preferences.reader = loaded;
            preferences.normalize_for_device(self.profile.is_scribe());
            PreparedReaderLoad {
                book,
                index: target.index,
                body: String::new(),
                preferences: preferences.reader,
                paginator: None,
                changed_book: true,
                changed_chapter: true,
                reopening: true,
                consume_pending_position: false,
                locked: false,
            }
        };
        prepared.index = target.index;
        if let Some(paginator) = prepared.paginator.as_mut() {
            if let Some(position) = self.pending_reading_position(book_id)? {
                if cloud_chapter_index(&position, catalog, chapters) == Some(target.index)
                    && self.chapter_metadata_content_status(&prepared.book, target)
                        == ContentStatus::Full
                {
                    let words = catalog
                        .iter()
                        .find(|chapter| chapter.uid == target.id)
                        .map_or(0, |chapter| chapter.word_count);
                    paginator.seek(remote_to_local_offset(
                        position.chapter_offset,
                        words,
                        paginator.char_count,
                    ));
                    prepared.consume_pending_position = true;
                }
            }
        }
        Ok(Some(prepared))
    }

    pub(super) fn install_reader_load(
        &mut self,
        prepared: Option<PreparedReaderLoad>,
    ) -> Result<()> {
        self.install_reader_load_in(prepared, false)
    }

    pub(super) fn install_catalog_reader_load(
        &mut self,
        prepared: Option<PreparedReaderLoad>,
    ) -> Result<()> {
        self.install_reader_load_in(prepared, true)
    }

    fn install_reader_load_in(
        &mut self,
        prepared: Option<PreparedReaderLoad>,
        catalog_committed: bool,
    ) -> Result<()> {
        let Some(prepared) = prepared else {
            self.route = Route::Library;
            self.show_toast("empty_library");
            return Ok(());
        };
        let PreparedReaderLoad {
            book,
            index,
            body,
            preferences,
            paginator,
            changed_book,
            changed_chapter,
            reopening,
            consume_pending_position,
            locked,
        } = prepared;
        let book_id = book.id.as_str();
        // Every fallible layout and picture stage ended before these old-view
        // persistence operations or any receiver, draft, route, or body change.
        if !catalog_committed
            && changed_book
            && self.reader.session_started.is_some()
            && !self.reader.book_id.is_empty()
        {
            self.finish_reading()?;
        } else if !catalog_committed
            && changed_chapter
            && self.reader.session_started.is_some()
            && !self.reader.book_id.is_empty()
        {
            self.persist_reader_position()?;
        }
        if !catalog_committed && body.is_empty() && self.reader.session_started.is_some() {
            self.finish_reading()?;
        }
        if consume_pending_position && !catalog_committed {
            self.store
                .delete_setting(&format!("progress.pending:{book_id}"))?;
        }
        self.reader.preferences = preferences;
        if changed_book || reopening {
            self.reader.position_confirmed = !self.prefs.pull_on_open;
            self.reader.notes_panel = false;
            self.reader.cloud = None;
            self.reader.selection = None;
            self.reader.draft.clear();
        }
        self.reader.set_context(book_id, index);
        self.reader.replace_body(body);
        self.route = Route::Reader(book_id.to_owned(), index);
        if changed_chapter || reopening {
            self.reader.paginator = None;
            self.reader.replace_underlines(Vec::new());
            self.reader.remote_thoughts.clear();
            self.reader.thought_id.clear();
            self.reader.thought_edit_blocked = false;
            self.reader.thought_page = 0;
            self.reader.toc_tab = "toc".into();
            self.reader.toc_page = index as usize / self.profile.reader_chapter_page_size();
            self.reader.selection = None;
            self.invalidate_reader_press();
            if let Some(ui) = self.ui.upgrade() {
                let state = ui.global::<ReaderModel>();
                state.set_menu(false);
                state.set_toc_open(false);
                state.set_thought_open(false);
                state.set_selection_open(false);
                state.set_writing_thought(false);
                state.set_sync_conflict(false);
                state.set_subtool("".into());
            }
        }
        self.reader.paginator = paginator;
        // After a committed relocation, the new UID/index/body are coherent
        // before any optional persistence can fail. Never save the old index.
        if consume_pending_position && catalog_committed {
            self.store
                .delete_setting(&format!("progress.pending:{book_id}"))?;
        }
        if !self.reader.body.is_empty() {
            if self.reader.session_started.is_none() {
                self.reader.session_started = Some(Utc::now().timestamp());
                self.reader.session_seconds = 0;
                if let Err(error) = self.start_read_report_session() {
                    self.note_read_report_error(&error);
                }
            }
            self.reader.last_active = Instant::now();
        } else {
            self.reader.paginator = None;
            if book.source == "weread"
                && !locked
                && self.online
                && !self.force_offline
                && self.jobs.capabilities().chapter_content
            {
                self.pending_read = Some((book_id.to_owned(), index));
                self.enqueue_download(book_id, vec![index])?;
            }
        }
        self.load_reader_underlines(changed_chapter)?;
        if (changed_book || reopening)
            && self.prefs.pull_on_open
            && !self.demo
            && self.online
            && !self.force_offline
            && self.jobs.capabilities().remote_progress
        {
            let request = self.request();
            self.store
                .set_setting(&format!("progress.request:{book_id}"), &request)?;
            self.settle_job_submission(self.jobs.progress(request, book_id.to_owned()));
        }
        self.platform
            .set_reader_refresh_interval(self.reader.preferences.refresh_interval)?;
        Ok(())
    }

    pub(super) fn load_reader_underlines(&mut self, request_remote: bool) -> Result<()> {
        self.load_reader_underlines_with_allowance(
            request_remote,
            UNDERLINE_CACHE_BYTES,
            UNDERLINE_CACHE_RECORDS,
        )
    }

    #[cfg(test)]
    pub(super) fn load_reader_underlines_with_test_allowance(
        &mut self,
        maximum_bytes: usize,
        maximum_records: usize,
    ) -> Result<()> {
        self.load_reader_underlines_with_allowance(false, maximum_bytes, maximum_records)
    }

    fn load_reader_underlines_with_allowance(
        &mut self,
        request_remote: bool,
        maximum_bytes: usize,
        maximum_records: usize,
    ) -> Result<()> {
        if !self.prefs.popular {
            self.reader.replace_underlines(Vec::new());
            return Ok(());
        }
        let Some(chapter_uid) = self
            .store
            .chapter_uid(&self.reader.book_id, self.reader.chapter_index)?
        else {
            self.reader.replace_underlines(Vec::new());
            return Ok(());
        };
        let cached = self
            .store
            .bounded_array_setting::<serde_json::Value>(
                &format!("underlines:{}:{}", self.reader.book_id, chapter_uid),
                maximum_bytes,
                maximum_records,
            )
            .and_then(|records| {
                source::WeReadClient::normalize_cached_underlines(&records.unwrap_or_default())
                    .map_err(anyhow::Error::from)
            });
        match cached {
            Ok(underlines) => self.reader.replace_underlines(underlines),
            Err(error)
                if error
                    .downcast_ref::<crate::store::CachedResourceLimit>()
                    .is_some()
                    || error.downcast_ref::<serde_json::Error>().is_some()
                    || matches!(
                        error.downcast_ref::<source::SourceError>(),
                        Some(
                            source::SourceError::ResponseTooLarge
                                | source::SourceError::InvalidResponse(_)
                        )
                    ) =>
            {
                // A rejected cache cannot replace an already admitted model,
                // discard a local draft, or turn into an authentication error.
                self.show_toast("api_error");
            }
            Err(error) => return Err(error),
        }
        if request_remote
            && !self.demo
            && self.online
            && !self.force_offline
            && self
                .jobs
                .session()
                .is_some_and(|session| session.has_api_key())
        {
            let request = self.request();
            self.reader.underline_request = request;
            self.settle_job_submission(self.jobs.underlines(
                request,
                self.reader.book_id.clone(),
                chapter_uid,
            ));
        }
        Ok(())
    }

    fn reader_page_underlines(&mut self) -> Vec<ProjectedUnderline> {
        if !self.prefs.popular {
            return Vec::new();
        }
        let key = ProjectionKey {
            body_revision: self.reader.underline_body_revision,
            underline_revision: self.reader.underline_revision,
            account_epoch: self.jobs.account_epoch(),
        };
        let result = self
            .reader
            .underline_projection
            .ensure(key, &self.reader.body, &self.reader.underlines)
            .and_then(|_| {
                self.reader
                    .paginator
                    .as_ref()
                    .map_or(Ok(Vec::new()), |paginator| {
                        self.reader
                            .underline_projection
                            .page_ranges(&paginator.current_page().lines)
                    })
            });
        match result {
            Ok(ranges) => ranges,
            Err(error) => {
                if self.reader.underline_projection.take_failure_notice() {
                    crate::diagnostic!("Remote underline overlay unavailable: {error}");
                    self.show_toast("api_error");
                }
                Vec::new()
            }
        }
    }

    fn reader_geometry(&self) -> (Layout, f32) {
        self.reader_geometry_for(&self.reader.preferences, self.reader.notes_panel)
    }

    fn reader_geometry_for(
        &self,
        preferences: &ReaderPreferences,
        notes_panel: bool,
    ) -> (Layout, f32) {
        let scribe = self.profile.is_scribe();
        let (width, height) = self.platform.logical_viewport_size();
        let margin = reader_margin(scribe, preferences.margin);
        let line_height = (if scribe {
            [1.6, 1.9, 2.15]
        } else {
            [1.55, 1.8, 2.1]
        })[preferences.line_spacing.min(2) as usize];
        let panel = if scribe && notes_panel { 300.0 } else { 0.0 };
        (
            Layout {
                width: (width as f32 - margin * 2.0 - panel).max(48.0),
                height: height as f32 - if scribe { 76.0 + 48.0 } else { 54.0 + 48.0 },
                font_size: preferences.font_size as f32,
                line_height,
                indent: preferences.indent.min(2) as usize,
                paragraph_gap: [0.0, 0.4, 0.9][preferences.gap.min(2) as usize],
                justify: preferences.justify,
            },
            height as f32 - if scribe { 204.0 + 48.0 } else { 164.0 + 48.0 },
        )
    }

    fn reader_font(&self) -> Option<&FontRecord> {
        self.reader_font_for(&self.reader.preferences)
    }

    fn reader_font_for(&self, preferences: &ReaderPreferences) -> Option<&FontRecord> {
        self.fonts
            .iter()
            .find(|font| font.available && font.id == preferences.font)
            .or_else(|| {
                self.fonts
                    .iter()
                    .find(|font| font.available && font.id == "song")
            })
            .or_else(|| self.fonts.iter().find(|font| font.available))
    }

    pub(super) fn reflow_reader(&mut self) -> Result<()> {
        let anchor = self
            .reader
            .paginator
            .as_ref()
            .map(|p| p.current_page().start)
            .unwrap_or(0);
        self.reflow_reader_at(anchor)
    }

    fn reflow_reader_at(&mut self, anchor: u32) -> Result<()> {
        let source = self.store.book_source(&self.reader.book_id)?;
        let (paginator, font) = self.prepare_reader_pagination(
            &self.reader.book_id,
            self.reader.chapter_index,
            &self.reader.body,
            &self.reader.preferences,
            self.reader.notes_panel,
            anchor,
            source.as_deref(),
        )?;
        self.install_reader_pagination(paginator, font);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_reader_pagination(
        &self,
        book_id: &str,
        index: u32,
        body: &str,
        preferences: &ReaderPreferences,
        notes_panel: bool,
        anchor: u32,
        source: Option<&str>,
    ) -> Result<(Paginator, String)> {
        let budget = LayoutBudget::default();
        let _old = budget.hold(self.retained_layout_metadata()?)?;
        let access = self.store.chapter_access_metadata(book_id, index)?;
        let (layout, first_height) = self.reader_geometry_for(preferences, notes_panel);
        let Some(font) = self.reader_font_for(preferences) else {
            anyhow::bail!("No registered reading font is available");
        };
        let metrics = font.metrics_for_weight(preferences.weight).clone();
        let selected_font = font.id.clone();
        let snapshot = self.reader_image_snapshot_for(book_id, index, &budget)?;
        let images = self.reader_inline_images_for(body, &layout, snapshot.as_ref(), &budget)?;
        drop(snapshot);
        let mut paginator = Paginator::try_new_with_images_in(
            body,
            &layout,
            &metrics,
            first_height,
            &images,
            &budget,
        )?;
        let visible_image = (self.reader.book_id == book_id && self.reader.chapter_index == index)
            .then(|| self.reader.paginator.as_ref())
            .flatten()
            .filter(|old| old.current_page().lines.is_empty())
            .and_then(|old| old.current_page().images.first())
            .map(|image| image.id.as_str());
        if !visible_image.is_some_and(|id| paginator.seek_image(id).is_some()) {
            paginator.seek(anchor);
        }
        if let (Some(source), Some(chapter)) = (source, access.as_ref()) {
            if reader_access_hint(
                self.chapter_access_content_status(source, chapter),
                chapter.body_present,
            )
            .is_some()
            {
                append_reader_access_page(&mut paginator, first_height)?;
                if anchor >= paginator.char_count {
                    paginator.seek(anchor);
                }
            }
        }
        Ok((paginator, selected_font))
    }

    fn install_reader_pagination(&mut self, paginator: Paginator, font: String) {
        if self.reader.press.is_some_and(ReaderPress::is_selecting) {
            self.reader.selection = None;
        }
        self.invalidate_reader_press();
        self.reader.preferences.font = font;
        self.reader.paginator = Some(paginator);
    }

    fn apply_reader_layout_preferences(
        &mut self,
        preferences: ReaderPreferences,
        notes_panel: bool,
    ) -> Result<()> {
        let anchor = self
            .reader
            .paginator
            .as_ref()
            .map_or(0, |page| page.current_page().start);
        let source = self.store.book_source(&self.reader.book_id)?;
        let (paginator, font) = self.prepare_reader_pagination(
            &self.reader.book_id,
            self.reader.chapter_index,
            &self.reader.body,
            &preferences,
            notes_panel,
            anchor,
            source.as_deref(),
        )?;
        self.reader.preferences = preferences;
        self.reader.notes_panel = notes_panel;
        self.install_reader_pagination(paginator, font);
        Ok(())
    }

    pub(super) fn render_reader(&mut self) -> Result<()> {
        let _ = self.reader_thought_editor_blocked();
        let Some(ui) = self.ui.upgrade() else {
            return Ok(());
        };
        let Some(book) = self.store.book_metadata_by_id(&self.reader.book_id)? else {
            return Ok(());
        };
        self.prepare_sleep_screen_for_book(&book);
        let Some(access) = self
            .store
            .chapter_access_metadata(&book.id, self.reader.chapter_index)?
        else {
            return Ok(());
        };
        let chapter = &access.chapter;
        let owner = if book.source == "weread" {
            self.chapter_content_owner()
        } else {
            None
        };
        if !self.prefs.popular && self.reader.thought_id.starts_with("remote:") {
            self.retire_unavailable_remote_thought(&ui.global::<ReaderModel>());
        }
        let remote_ranges = self.reader_page_underlines();
        let state = ui.global::<ReaderModel>();
        self.retire_unadmitted_remote_thought(&state, &remote_ranges);
        let preferences = &self.reader.preferences;
        let (layout, _) = self.reader_geometry();
        let page = self.reader.paginator.as_ref().map(|p| p.current_page());
        let page_start = page.map_or(0, |p| p.start);
        let page_end = page.map_or(0, |p| p.end);
        let annotations = if self.prefs.mine {
            self.note_geometry(&book.id, chapter.index, page_start, page_end)?
        } else {
            Vec::new()
        };
        let annotation_count = self.store.annotation_count(&book.id)?;
        let bookmark_count = self.store.bookmark_count(&book.id)?;
        let bookmarks = if let Some(page) = page {
            self.store.bookmark_geometry_for_range(
                &book.id,
                chapter.index,
                page.start,
                page.end.max(page.start.saturating_add(1)),
            )?
        } else {
            Vec::new()
        };
        let view_budget = LayoutBudget::default();
        let _old_view = view_budget.hold(self.retained_layout_metadata()?)?;
        let image_rows = self.prepare_reader_image_rows(&view_budget)?;
        let row_count = page.map_or(0, |page| page.lines.len());
        let additional = MAX_PAGE_MARKS
            .checked_mul(std::mem::size_of::<ReaderMark>() + 64)
            .and_then(|bytes| {
                row_count
                    .checked_mul(std::mem::size_of::<VecModel<ReaderMark>>() + 128)
                    .and_then(|models| bytes.checked_add(models))
            })
            .ok_or(crate::reader::LayoutLimitError {
                kind: crate::reader::LayoutLimitKind::Metadata,
                used: 0,
                requested: usize::MAX,
                limit: crate::reader::layout_budget::DEFAULT_LAYOUT_METADATA_BYTES,
            })?;
        self.preflight_view_fields::<ReaderLine>(
            &view_budget,
            row_count,
            page.into_iter()
                .flat_map(|page| page.lines.iter().map(|line| line.text.as_str())),
            additional,
        )?;
        let mut rows =
            crate::reader::layout_budget::LayoutVec::with_capacity(&view_budget, row_count)?;
        let mut caret_budget = self.reader_caret_budget();
        let mut caret_failed = false;
        let mut reservations: [Option<crate::reader::CaretReservation>;
            crate::reader::MAX_CARET_PAGE_LINES] = std::array::from_fn(|_| None);
        let metrics = self
            .reader_font()
            .map(|font| {
                font.metrics_for_weight(self.reader.preferences.weight)
                    .clone()
            })
            .unwrap_or_default();
        if let Some(page) = page {
            let output_plan = self.page_mark_output_plan(&page.lines, &annotations, &remote_ranges);
            caret_failed =
                page.lines.len() > crate::reader::MAX_CARET_PAGE_LINES || output_plan.is_err();
            if !caret_failed {
                // Reject the whole operation before computing even its first
                // caret vector. Idle redraws of a rejected page cannot repeat
                // an earlier line's admitted quadratic history scan.
                for (index, line) in page.lines.iter().enumerate() {
                    let needed =
                        self.reader.selection.is_some_and(|(start, end)| {
                            intersects(line.start, line.end, start, end)
                        }) || self.prefs.mine
                            && annotations.iter().any(|note| {
                                note.chapter_index == chapter.index
                                    && intersects(line.start, line.end, note.start, note.end)
                            })
                            || remote_ranges.iter().any(|range| {
                                intersects(line.start, line.end, range.start, range.end)
                            });
                    if !needed {
                        continue;
                    }
                    match caret_budget.admit(line) {
                        Ok(admitted) => reservations[index] = Some(admitted),
                        Err(_) => {
                            caret_failed = true;
                            break;
                        }
                    }
                }
            }
            for (index, line) in page.lines.iter().enumerate() {
                let mine = self.prefs.mine
                    && annotations.iter().any(|note| {
                        note.chapter_index == chapter.index
                            && intersects(line.start, line.end, note.start, note.end)
                    });
                let count = remote_ranges
                    .iter()
                    .filter(|range| intersects(line.start, line.end, range.start, range.end))
                    .map(|range| self.reader.underlines[range.index].count)
                    .max()
                    .unwrap_or(0);
                let selection = self
                    .reader
                    .selection
                    .filter(|(start, end)| intersects(line.start, line.end, *start, *end));
                let geometry = if !caret_failed {
                    match reservations[index].take().map(|reservation| {
                        line.caret_geometry_with_reservation(
                            &layout,
                            &metrics,
                            &mut caret_budget,
                            reservation,
                        )
                    }) {
                        Some(Ok(geometry)) => Some(geometry),
                        Some(Err(_)) => {
                            caret_failed = true;
                            None
                        }
                        None => None,
                    }
                } else {
                    None
                };
                let spacing = geometry.as_ref().map_or_else(
                    || line.letter_spacing(&layout, &metrics),
                    |geometry| geometry.spacing,
                );
                let capacity = if caret_failed {
                    0
                } else {
                    output_plan
                        .as_ref()
                        .map(|plan| plan.counts[index].iter().sum())
                        .unwrap_or(0)
                };
                let mut marks =
                    crate::reader::layout_budget::LayoutVec::with_capacity(&view_budget, capacity)?;
                if let Some(geometry) = &geometry {
                    for note in annotations.iter().filter(|note| {
                        self.prefs.mine
                            && note.chapter_index == chapter.index
                            && intersects(line.start, line.end, note.start, note.end)
                    }) {
                        let (mark, owner) = self.admitted_reader_mark(
                            line,
                            &layout,
                            geometry,
                            note.start,
                            note.end,
                            // Drawing needs a short occurrence identity only;
                            // hit testing resolves the fenced header separately.
                            format_args!("mine-row:{}", note.rowid),
                            true,
                            false,
                            0,
                            &view_budget,
                        )?;
                        marks.push_owned(mark, owner)?;
                        self.record_mark_output(0);
                    }
                    for range in &remote_ranges {
                        if intersects(line.start, line.end, range.start, range.end) {
                            let underline = &self.reader.underlines[range.index];
                            let (mark, owner) = self.admitted_reader_mark(
                                line,
                                &layout,
                                geometry,
                                range.start,
                                range.end,
                                format_args!("remote:{}", range.index),
                                false,
                                false,
                                if range.end <= line.end {
                                    underline.count
                                } else {
                                    0
                                },
                                &view_budget,
                            )?;
                            marks.push_owned(mark, owner)?;
                            self.record_mark_output(1);
                        }
                    }
                    if let Some((start, end)) = selection {
                        let (mark, owner) = self.admitted_reader_mark(
                            line,
                            &layout,
                            geometry,
                            start,
                            end,
                            format_args!("selection"),
                            true,
                            true,
                            0,
                            &view_budget,
                        )?;
                        marks.push_owned(mark, owner)?;
                        self.record_mark_output(2);
                    }
                }
                let marks = self.admitted_view_model(marks, &view_budget)?;
                let (text, owner) = self.admitted_shared_string(&line.text, &view_budget)?;
                rows.push_owned(
                    ReaderLine {
                        text,
                        start: line.start as i32,
                        end: line.end as i32,
                        indent: line.indent,
                        highlight: count > 0,
                        mine,
                        count: count.min(i32::MAX as u32) as i32,
                        letter_spacing: spacing,
                        gap_after: line.gap_after,
                        paragraph_end: line.paragraph_end,
                        y: line.y,
                        height: line.height,
                        marks,
                    },
                    owner,
                )?;
            }
        }
        self.record_reader_caret_budget(&caret_budget);
        if caret_failed {
            // A page allowance rejects the whole derived overlay. Source notes,
            // current selection and drafts remain available and unchanged.
            for row in rows.iter_mut() {
                row.marks = ModelRc::default();
                row.highlight = false;
                row.mine = false;
                row.count = 0;
            }
        }
        let rows = self.admitted_view_model(rows, &view_budget)?;
        let page_index = self
            .reader
            .paginator
            .as_ref()
            .map_or(0, |p| p.current_index());
        let pages = self.reader.paginator.as_ref().map_or(1, |p| p.pages.len());
        let fraction = self.local_book_fraction_with_access(
            page_start,
            Some(&book.source),
            Some(&access),
            owner.as_deref(),
        )?;
        let content_status = library::chapter_content_status_for_access_owner(
            &book.source,
            &access,
            owner.as_deref(),
        );
        let access_hint = reader_access_hint(content_status, access.body_present);
        let access_label = if access_hint.is_some() {
            reader_content_label(content_status, access.body_present)
        } else {
            String::new()
        };
        state.set_access_label(access_label.clone().into());
        state.set_access_hint(access_hint.map(tr).unwrap_or_default().into());
        state.set_access_action(
            tr(
                if matches!(
                    content_status,
                    ContentStatus::Preview | ContentStatus::Locked
                ) {
                    "retry_after_unlock"
                } else {
                    "verify_content_access"
                },
            )
            .into(),
        );
        state.set_access_notice(access_hint.is_some() && page_index + 1 == pages);
        state.set_access_offset(page.map_or(READER_ACCESS_NOTICE_GAP, |page| {
            reader_page_bottom(page) + READER_ACCESS_NOTICE_GAP
        }));
        state.set_title(book.title.clone().into());
        state.set_chapter_title(chapter.title.clone().into());
        state.set_chapter_meta(
            format!(
                "{} {} / {}",
                tr("catalog"),
                chapter.index + 1,
                book.total_chapters.max(1)
            )
            .into(),
        );
        state.set_author(
            if book.source == "demo" && book.id == "demo-0" {
                tr(match chapter.index {
                    0 => "sample_chapter_author",
                    1 => "sample_lanting_author",
                    2 => "sample_loushi_author",
                    _ => "sample_author",
                })
            } else {
                book.author.clone()
            }
            .into(),
        );
        state.set_footer(
            i18n::format(
                "page_summary",
                &[
                    ("page", (page_index + 1).to_string()),
                    ("total", pages.to_string()),
                    ("progress", format!("{:.0}", fraction * 100.0)),
                ],
            )
            .into(),
        );
        state.set_show_heading(page_index == 0);
        state.set_lines(rows);
        state.set_status(
            if self.reader.body.is_empty() {
                tr("no_content")
            } else {
                String::new()
            }
            .into(),
        );
        state.set_font_family(
            self.reader_font()
                .map(|font| font.family.clone())
                .unwrap_or_default()
                .into(),
        );
        state.set_font_label(
            self.reader_font()
                .map(|font| font.label.clone())
                .unwrap_or_default()
                .into(),
        );
        state.set_fonts(model(
            self.fonts
                .iter()
                .map(|font| ReaderFont {
                    id: font.id.clone().into(),
                    label: font.label.clone().into(),
                    family: font.family.clone().into(),
                    available: font.available,
                })
                .collect(),
        ));
        state.set_font_size(preferences.font_size as i32);
        state.set_weight(preferences.weight as i32);
        state.set_line_height(layout.line_height);
        state.set_margin(reader_margin(self.profile.is_scribe(), preferences.margin) as i32);
        state.set_font_choice(preferences.font.clone().into());
        state.set_margin_choice(
            ["narrow", "mid", "wide"][preferences.margin.min(2) as usize].into(),
        );
        state.set_line_choice(
            ["tight", "mid", "loose"][preferences.line_spacing.min(2) as usize].into(),
        );
        state.set_indent_choice(preferences.indent.to_string().into());
        state.set_gap_choice(["none", "small", "big"][preferences.gap.min(2) as usize].into());
        state.set_book_only(preferences.book_only);
        state.set_justify(preferences.justify);
        state.set_tap_mode(preferences.tap_mode.clone().into());
        state.set_refresh_choice(preferences.refresh_interval.to_string().into());
        state.set_page(page_index as i32 + 1);
        state.set_page_count(pages as i32);
        state.set_progress(fraction);
        state.set_notes_panel(self.reader.notes_panel);
        state.set_popular(self.prefs.popular);
        state.set_mine(self.prefs.mine);
        state.set_night(self.prefs.dark_mode);
        state.set_thought_position(self.prefs.popup_position.clone().into());
        state.set_thought_height(self.prefs.popup_height as i32);
        let capabilities = self.platform.capabilities();
        state.set_hardware_light(capabilities.brightness && state.get_brightness_known());
        state.set_hardware_warmth(capabilities.warmth && state.get_warmth_known());
        state.set_night_supported(capabilities.application_dark_mode);
        state.set_pen_supported(capabilities.pen);
        state.set_can_download(
            book.source == "weread"
                && !self.demo
                && self.online
                && !self.force_offline
                && self.jobs.capabilities().chapter_content,
        );
        state.set_download_busy(
            self.current_download.as_ref().is_some_and(|task| {
                task.book_id == book.id && task.chapters.contains(&chapter.index)
            }),
        );
        state.set_bookmarked(bookmarks.iter().any(|mark| {
            mark.chapter_index == chapter.index
                && mark.char_offset >= page_start
                && mark.char_offset < page_end.max(page_start + 1)
        }));
        state.set_chapter_summary(
            i18n::format(
                "chapter_summary",
                &[
                    ("total", book.total_chapters.to_string()),
                    ("cached", book.cached_chapters.to_string()),
                ],
            )
            .into(),
        );
        state.set_thought_draft(self.reader.draft.query.clone().into());
        if let Some((start, end)) = self.reader.selection {
            state.set_selection_text(self.reader_selection_preview(start, end).into());
        }
        let selected_notes = self.render_reader_contents(
            &state,
            &book,
            annotation_count,
            bookmark_count,
            owner.as_deref(),
        )?;
        self.render_reader_thoughts(
            &state,
            &annotations,
            page_start,
            page_end,
            &remote_ranges,
            selected_notes,
        )?;
        state.set_images(image_rows);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn admitted_reader_mark(
        &self,
        line: &Line,
        layout: &Layout,
        geometry: &CaretGeometry,
        start: u32,
        end: u32,
        id: std::fmt::Arguments<'_>,
        mine: bool,
        selected: bool,
        count: u32,
        budget: &LayoutBudget,
    ) -> Result<(ReaderMark, crate::reader::layout_budget::LayoutLease)> {
        let (id, mut owner) = budget.format(id)?;
        let owned = owner.bytes();
        let shared = id
            .len()
            .checked_add(1)
            .ok_or(crate::reader::LayoutLimitError {
                kind: crate::reader::LayoutLimitKind::Metadata,
                used: 0,
                requested: usize::MAX,
                limit: crate::reader::layout_budget::DEFAULT_LAYOUT_METADATA_BYTES,
            })?;
        owner.merge(budget.copy_storage(shared)?);
        let mark = reader_mark_with_geometry(
            line, layout, geometry, start, end, id, mine, selected, count,
        );
        owner.release(owned);
        Ok((mark, owner))
    }

    fn render_reader_contents(
        &mut self,
        state: &ReaderModel,
        book: &BookMetadata,
        annotation_count: u64,
        bookmark_count: u64,
        owner: Option<&str>,
    ) -> Result<usize> {
        let size = if self.reader.toc_tab == "toc" {
            self.profile.reader_chapter_page_size()
        } else {
            self.reader_entry_page_size()
        };
        let chapters = self.store.chapter_metadata_with_presence(&book.id)?;
        let toc_open = state.get_toc_open();
        let length = match self.reader.toc_tab.as_str() {
            "toc" => chapters.len(),
            "notes" => usize::try_from(annotation_count).unwrap_or(usize::MAX),
            _ => usize::try_from(bookmark_count).unwrap_or(usize::MAX),
        };
        let count = page_count(length, size);
        let page = self.reader.toc_page.min(count - 1);
        let chapter_size = if toc_open {
            size
        } else {
            self.profile.reader_chapter_page_size()
        };
        let chapter_offset = if toc_open {
            page * chapter_size
        } else {
            let current = chapters
                .iter()
                .position(|entry| entry.chapter.index == self.reader.chapter_index)
                .unwrap_or(0);
            current / chapter_size * chapter_size
        };
        // Lightweight access/current headers remain available with the TOC
        // closed. Presence is display evidence, never Full-content authority.
        let chapter_model = model(
            chapters
                .iter()
                .skip(chapter_offset)
                .take(chapter_size)
                .map(|entry| {
                    let chapter = &entry.chapter;
                    let status =
                        library::chapter_content_status_for_metadata_owner(book, chapter, owner);
                    ReaderChapter {
                        id: chapter.id.clone().into(),
                        title: chapter.title.clone().into(),
                        meta: library::chapter_content_label_for_status(
                            status,
                            entry.body_present,
                            chapter.index == self.reader.chapter_index,
                        )
                        .into(),
                        cached: status == ContentStatus::Full,
                        current: chapter.index == self.reader.chapter_index,
                    }
                })
                .collect(),
        );
        if !toc_open {
            state.set_chapter_count(chapters.len().min(i32::MAX as usize) as i32);
            state.set_mark_count(bookmark_count.min(i32::MAX as u64) as i32);
            state.set_note_count(annotation_count.min(i32::MAX as u64) as i32);
            state.set_chapters(chapter_model);
            state.set_entries(model(Vec::new()));
            return Ok(0);
        }
        let notes = if self.reader.toc_tab == "notes" {
            let page = self.note_headers_page(&book.id, page * size, size)?;
            self.note_display_records(&book.id, &page.headers)?
        } else {
            Vec::new()
        };
        let bookmarks = if self.reader.toc_tab == "marks" {
            let selected = self
                .store
                .bookmark_geometry_page(&book.id, page * size, size)?;
            let mut values = Vec::with_capacity(selected.geometries.len());
            for geometry in &selected.geometries {
                let header = self
                    .store
                    .bookmark_header_for_geometry(&book.id, geometry)?
                    .ok_or(BookmarkUnavailable)?;
                let label = if chapters
                    .iter()
                    .any(|entry| entry.chapter.index == header.chapter_index)
                {
                    String::new()
                } else {
                    self.store
                        .bounded_bookmark_label(&book.id, &header, 64 * 1024)?
                        .ok_or(BookmarkUnavailable)?
                };
                values.push((header, label));
            }
            values
        } else {
            Vec::new()
        };
        let entries: Vec<ReaderEntry> = match self.reader.toc_tab.as_str() {
            "marks" => bookmarks
                .iter()
                .map(|(mark, label)| {
                    let chapter = chapters
                        .iter()
                        .find(|entry| entry.chapter.index == mark.chapter_index)
                        .map(|entry| &entry.chapter);
                    let page_label = (mark.chapter_index == self.reader.chapter_index)
                        .then_some(self.reader.paginator.as_ref())
                        .flatten()
                        .map(|paginator| {
                            i18n::format(
                                "reader_chapter_page",
                                &[(
                                    "page",
                                    (paginator.page_for_offset(mark.char_offset) + 1).to_string(),
                                )],
                            )
                        })
                        .unwrap_or_default();
                    ReaderEntry {
                        id: if chapter.is_none() {
                            String::new()
                        } else {
                            format!("bookmark:{}", mark.key.id)
                        }
                        .into(),
                        title: chapter
                            .map_or_else(|| label.clone(), |chapter| chapter.title.clone())
                            .into(),
                        subtitle: if chapter.is_none() {
                            tr("chapter_removed")
                        } else {
                            stamp(mark.created_at)
                        }
                        .into(),
                        kind: "bookmark".into(),
                        chapter: chapter
                            .map_or_else(|| tr("chapter_removed"), |chapter| chapter.title.clone())
                            .into(),
                        date: stamp(mark.created_at).into(),
                        page_label: page_label.into(),
                        body: String::new().into(),
                        cached: chapter.is_some_and(|chapter| chapter.cached),
                    }
                })
                .collect(),
            "notes" => notes
                .iter()
                .map(|note| {
                    let chapter = chapters
                        .iter()
                        .find(|entry| entry.chapter.index == note.chapter_index)
                        .map(|entry| &entry.chapter);
                    ReaderEntry {
                        id: if chapter.is_none() {
                            String::new()
                        } else {
                            format!("annotation:{}", note.id)
                        }
                        .into(),
                        title: note.quote.clone().into(),
                        subtitle: if chapter.is_none() {
                            tr("chapter_removed")
                        } else {
                            stamp(note.created_at)
                        }
                        .into(),
                        kind: "note".into(),
                        chapter: chapter
                            .map_or_else(|| tr("chapter_removed"), |chapter| chapter.title.clone())
                            .into(),
                        date: stamp(note.created_at).into(),
                        page_label: String::new().into(),
                        body: note.note.clone().into(),
                        cached: chapter.is_some_and(|chapter| chapter.cached),
                    }
                })
                .collect(),
            _ => vec![],
        };
        self.reader.toc_page = page;
        state.set_chapter_count(chapters.len().min(i32::MAX as usize) as i32);
        state.set_mark_count(bookmark_count.min(i32::MAX as u64) as i32);
        state.set_note_count(annotation_count.min(i32::MAX as u64) as i32);
        state.set_chapters(chapter_model);
        state.set_toc_tab(self.reader.toc_tab.clone().into());
        state.set_chapter_page(self.reader.toc_page as i32 + 1);
        state.set_chapter_pages(count as i32);
        state.set_entries(model(entries));
        Ok(notes.len())
    }

    fn retire_unavailable_remote_thought(&mut self, state: &ReaderModel) {
        if !self.reader.thought_id.starts_with("remote:") {
            return;
        }
        self.reader.thought_id.clear();
        self.reader.thought_context = None;
        self.reader.thought_edit_blocked = false;
        self.reader.remote_thoughts.clear();
        // A local editor can still own the selected quote and unsaved draft.
        // Retiring an unavailable public view must not invalidate that editor.
        if !state.get_writing_thought() {
            self.reader.selection = None;
            state.set_selection_open(false);
        }
        state.set_thought_open(false);
        state.set_thought_title("".into());
        state.set_thought_meta("".into());
        state.set_note_id("".into());
    }

    fn retire_unadmitted_remote_thought(
        &mut self,
        state: &ReaderModel,
        remote_ranges: &[ProjectedUnderline],
    ) {
        if state.get_thought_open() && self.reader.thought_id.starts_with("remote:") {
            let admitted = self
                .reader
                .thought_id
                .strip_prefix("remote:")
                .and_then(|index| index.parse::<usize>().ok())
                .is_some_and(|index| remote_ranges.iter().any(|range| range.index == index));
            if !admitted {
                self.retire_unavailable_remote_thought(state);
            }
        }
    }

    fn render_reader_thoughts(
        &mut self,
        state: &ReaderModel,
        geometry: &[AnnotationGeometry],
        start: u32,
        end: u32,
        remote_ranges: &[ProjectedUnderline],
        selected_notes: usize,
    ) -> Result<()> {
        use super::thoughts::{self, CardKind, ThoughtView};
        enum SelectedCard {
            Public(thoughts::ThoughtCard),
            Mine(usize),
        }
        self.retire_unadmitted_remote_thought(state, remote_ranges);
        let opened = state.get_thought_open();
        let mine_view = opened && self.reader.thought_id.starts_with("mine:");
        let opened_note = if mine_view {
            self.store.annotation_header_by_id_with_limits(
                &self.reader.book_id,
                self.reader.thought_id.trim_start_matches("mine:"),
                note_header_limits(),
            )?
        } else {
            None
        };
        let view = if mine_view {
            ThoughtView::Note(0)
        } else if opened {
            ThoughtView::Reviews
        } else {
            ThoughtView::List
        };
        let public = thoughts::cards(
            remote_ranges,
            &[],
            &self.reader.remote_thoughts,
            view,
            self.prefs.popular,
            false,
            self.reader.chapter_index,
            start,
            end,
        );
        let public_count = public.clone().count();
        let private_count = if mine_view {
            if opened_note.is_some() { 1 } else { 0 }
        } else if !opened && self.prefs.mine {
            geometry.len()
        } else {
            0
        };
        let total = public_count
            .checked_add(private_count)
            .ok_or(AnnotationUnavailable)?;
        let mut has_quote = public.clone().any(|card| card.has_quote);
        if let Some(header) = &opened_note {
            let metadata = self
                .store
                .annotation_text_metadata_with_limits(
                    &self.reader.book_id,
                    &header.key,
                    note_record_limits(),
                )?
                .ok_or(AnnotationUnavailable)?;
            has_quote |= !matches!(
                metadata.quote,
                AnnotationFieldState::Available {
                    utf8_bytes_upper: 0
                }
            );
        } else if private_count > 0 && !has_quote {
            has_quote = self.store.annotation_has_quote_for_range(
                &self.reader.book_id,
                self.reader.chapter_index,
                start,
                end,
            )?;
        }
        let scribe = self.profile.is_scribe();
        let side = scribe && self.reader.notes_panel && self.prefs.popup_position == "side";
        let row_height = if total == 0 || has_quote {
            if scribe { 164 } else { 144 }
        } else if scribe {
            112
        } else {
            104
        };
        let height = self.platform.logical_viewport_size().1 as f32;
        let available = if opened {
            if side {
                height - 76.0 - 160.0 - 56.0
            } else {
                height * self.prefs.popup_height as f32 / 100.0 - 160.0 - 48.0 - 4.0
            }
        } else {
            height - 76.0 - 48.0
        };
        let size = ((available / row_height as f32).floor().max(1.0) as usize)
            .min(thoughts::MAX_VISIBLE_CARDS);
        let count = page_count(total, size);
        let page = self.reader.thought_page.min(count - 1);
        if !opened && !self.reader.notes_panel {
            // Hidden views publish counts only. Geometry has no IDs or text.
            self.reader.thought_page = page;
            state.set_note_id("".into());
            state.set_thought_count(total.min(i32::MAX as usize) as i32);
            state.set_thought_page(page as i32 + 1);
            state.set_thought_pages(count.min(i32::MAX as usize) as i32);
            state.set_thoughts(model(Vec::new()));
            return Ok(());
        }
        let first = page.checked_mul(size).ok_or(AnnotationUnavailable)?;
        let last = first
            .checked_add(size)
            .ok_or(AnnotationUnavailable)?
            .min(total);
        let public_selected = public
            .skip(first.min(public_count))
            .take(last.min(public_count).saturating_sub(first))
            .collect::<Vec<_>>();
        let mut plans = public_selected
            .into_iter()
            .map(SelectedCard::Public)
            .collect::<Vec<_>>();
        let mut headers = Vec::new();
        for index in first.max(public_count)..last {
            let local = index - public_count;
            let header = if mine_view {
                opened_note.as_ref().ok_or(AnnotationUnavailable)?.clone()
            } else {
                self.store
                    .annotation_header_for_geometry(
                        &self.reader.book_id,
                        geometry.get(local).ok_or(AnnotationUnavailable)?,
                    )?
                    .ok_or(AnnotationUnavailable)?
            };
            plans.push(SelectedCard::Mine(headers.len()));
            headers.push(header);
        }
        let mut limits = note_record_limits();
        // The TOC and Scribe side view can coexist in one render. Price their
        // combined selected records rather than granting each another 16 rows.
        limits.rows = limits.rows.min(16).saturating_sub(selected_notes);
        if !opened {
            limits.note_bytes = limits.note_bytes.min(thoughts::PREVIEW_BYTES);
        }
        let notes =
            self.note_display_records_with_limits(&self.reader.book_id, &headers, limits)?;
        let chapter_uid = if plans.iter().any(|card| {
            matches!(card,
            SelectedCard::Public(card) if card.kind == CardKind::Remote)
        }) {
            self.store
                .chapter_uid(&self.reader.book_id, self.reader.chapter_index)?
        } else {
            None
        };
        let mut entries = Vec::with_capacity(plans.len());
        for plan in plans {
            let entry = match plan {
                SelectedCard::Mine(index) => {
                    let note = &notes[index];
                    ThoughtEntry {
                        id: format!("mine:{}", note.id).into(),
                        quote: thought_card_text(&note.quote, thoughts::QUOTE_BYTES).into(),
                        quote_lines: Default::default(),
                        meta: if opened {
                            stamp(note.created_at)
                        } else {
                            tr("my_notes")
                        }
                        .into(),
                        body: thought_card_text(
                            &note.note,
                            if opened {
                                thoughts::OPEN_TEXT_BYTES
                            } else {
                                thoughts::PREVIEW_BYTES
                            },
                        )
                        .into(),
                        mine: true,
                    }
                }
                SelectedCard::Public(card) if card.kind == CardKind::Remote => {
                    let underline = &self.reader.underlines[card.index];
                    let quote = thought_card_text(&underline.text, thoughts::QUOTE_BYTES);
                    let meta = format!(
                        "{} {} · {} {}",
                        underline.count,
                        tr("highlight"),
                        underline.thoughts,
                        tr("notes")
                    );
                    let preview = if let Some(uid) = &chapter_uid {
                        self.read_thought_preview(&format!(
                            "thoughts:{}:{}:remote:{}",
                            self.reader.book_id, uid, card.index
                        ))
                    } else {
                        String::new()
                    };
                    ThoughtEntry {
                        id: format!("remote:{}", card.index).into(),
                        quote: quote.into(),
                        quote_lines: Default::default(),
                        meta: meta.into(),
                        body: preview.into(),
                        mine: false,
                    }
                }
                SelectedCard::Public(card) => {
                    let review = &self.reader.remote_thoughts[card.index];
                    ThoughtEntry {
                        id: thought_card_text(&review.id, 4096).into(),
                        quote: "".into(),
                        quote_lines: Default::default(),
                        meta: format!(
                            "{} · {} {}",
                            thought_card_text(&review.author, 4096),
                            tr("likes"),
                            review.likes
                        )
                        .into(),
                        body: thought_card_text(&review.text, thoughts::OPEN_TEXT_BYTES).into(),
                        mine: false,
                    }
                }
            };
            entries.push(entry);
        }
        let quote_width = if side || (scribe && self.reader.notes_panel && !opened) {
            256.0
        } else if scribe {
            504.0
        } else {
            self.platform.logical_viewport_size().0 as f32
                - if self.prefs.popup_position == "bottom" {
                    32.0
                } else {
                    162.0
                }
        };
        let quote_metrics = self
            .fonts
            .iter()
            .find(|font| font.available && font.id == "song")
            .map(|font| font.metrics_for_weight(600).clone())
            .unwrap_or_default();
        for entry in &mut entries {
            if !entry.quote.is_empty() {
                #[cfg(test)]
                {
                    self.reader.thought_quote_layouts += 1;
                }
                entry.quote_lines = thought_quote_lines(
                    &entry.quote,
                    quote_width,
                    if scribe { 17.0 } else { 18.0 },
                    &quote_metrics,
                )?;
            }
        }
        self.reader.thought_page = page;
        state.set_note_id("".into());
        if let Some(note) = notes.first().filter(|_| mine_view) {
            state.set_note_id(note.id.clone().into());
            state.set_thought_title(thought_card_text(&note.quote, thoughts::QUOTE_BYTES).into());
            state.set_thought_meta(tr("my_notes").into());
        } else if opened {
            if let Some(underline) = self
                .reader
                .thought_id
                .strip_prefix("remote:")
                .and_then(|index| index.parse::<usize>().ok())
                .and_then(|index| self.reader.underlines.get(index))
            {
                state.set_thought_title(
                    thought_card_text(&underline.text, thoughts::QUOTE_BYTES).into(),
                );
                state.set_thought_meta(
                    format!(
                        "{} {} · {} {}",
                        underline.count,
                        tr("highlight"),
                        underline.thoughts,
                        tr("notes")
                    )
                    .into(),
                );
            }
        }
        state.set_thought_count(total.min(i32::MAX as usize) as i32);
        state.set_thought_page(page as i32 + 1);
        state.set_thought_pages(count.min(i32::MAX as usize) as i32);
        state.set_thoughts(model(entries));
        Ok(())
    }

    fn read_thought_preview(&mut self, key: &str) -> String {
        #[cfg(test)]
        {
            self.reader.thought_preview_reads += 1;
        }
        match self.store.bounded_array_setting::<RemoteReview>(
            key,
            super::thoughts::CACHE_BYTES,
            super::thoughts::CACHE_RECORDS,
        ) {
            Ok(Some(reviews)) => reviews
                .iter()
                .filter(|review| !review.text.trim().is_empty())
                .max_by_key(|review| review.likes)
                .map(|review| thought_card_text(&review.text, super::thoughts::PREVIEW_BYTES))
                .unwrap_or_default(),
            Ok(None) => String::new(),
            Err(_) => tr("api_error"),
        }
    }

    pub(super) fn reader_action(&mut self, action: &str, value: &str) -> Result<()> {
        let action = action.strip_prefix("reader.").unwrap_or(action);
        self.reader.last_active = Instant::now();
        match action {
            "back" => self.back()?,
            "detail" => {
                let id = self.reader.book_id.clone();
                self.open_detail(&id)?;
            }
            "press" => {
                if let Some((x, y)) = pointer(value) {
                    if self.reader.press.is_some_and(ReaderPress::is_selecting) {
                        self.reader.selection = None;
                    }
                    let generation = self.advance_reader_press_generation();
                    self.reader.press = Some(ReaderPress::new(x, y, generation));
                    self.reader.suppress_tap = false;
                    self.schedule_reader_pointer_event(
                        "reader.hold",
                        generation,
                        READER_HOLD_DELAY,
                    );
                }
            }
            "hold" => {
                if self.activate_reader_selection(value.parse().ok()) {
                    self.refresh_display(RefreshReason::Content);
                }
            }
            "move" => self.move_reader_pointer(value),
            "preview" => {
                if matches!(self.route, Route::Reader(..))
                    && self.reader.press.is_some_and(|press| {
                        press.is_selecting() && value.parse::<i32>().ok() == Some(press.generation)
                    })
                    && self.flush_reader_selection_preview()
                {
                    self.refresh_display(RefreshReason::Content);
                }
            }
            "cancel" => {
                let selecting = self.reader.press.is_some_and(ReaderPress::is_selecting);
                if selecting {
                    self.reader.selection = None;
                }
                if self.reader.press.is_some() {
                    self.reader.suppress_tap = true;
                }
                self.invalidate_reader_press();
                if selecting {
                    self.refresh_display(RefreshReason::Content);
                }
            }
            "release" => self.release_reader_pointer(value)?,
            "tap" => {
                if self.reader.suppress_tap {
                    self.reader.suppress_tap = false;
                    return Ok(());
                }
                let Some(x) = value
                    .parse::<f32>()
                    .ok()
                    .filter(|x| x.is_finite() && (0.0..=1.0).contains(x))
                else {
                    return Ok(());
                };
                let menu = self
                    .ui
                    .upgrade()
                    .is_some_and(|ui| ui.global::<ReaderModel>().get_menu());
                if menu || self.reader.preferences.tap_mode == "swipe" || (0.3..=0.7).contains(&x) {
                    self.set_reader_flag("menu", !menu);
                } else {
                    let mut delta = if x < 0.3 { -1 } else { 1 };
                    if self.reader.preferences.tap_mode == "mirrored" {
                        delta = -delta;
                    }
                    self.turn_reader(delta)?;
                }
            }
            "turn" => {
                // Once selection is active, movement cannot become a page swipe.
                if self.reader.press.is_some_and(ReaderPress::is_selecting) {
                    return Ok(());
                }
                self.invalidate_reader_press();
                self.turn_reader(value.parse().unwrap_or(0))?;
            }
            "chapter" => self.change_reader_chapter(value.parse().unwrap_or(0))?,
            "menu" => self.set_reader_flag("menu", value != "false"),
            "tool" => {
                if matches!(value, "progress" | "notes" | "light" | "type") {
                    if let Some(ui) = self.ui.upgrade() {
                        let state = ui.global::<ReaderModel>();
                        state.set_tool(value.into());
                        state.set_subtool("".into());
                    }
                    if value == "light" {
                        self.query_reader_light();
                    }
                }
            }
            "subtool" => {
                if matches!(value, "" | "font" | "indent" | "turn") {
                    if let Some(ui) = self.ui.upgrade() {
                        ui.global::<ReaderModel>().set_subtool(value.into());
                    }
                }
            }
            "panel" => {
                self.apply_reader_layout_preferences(
                    self.reader.preferences.clone(),
                    self.profile.is_scribe() && value != "false",
                )?;
                self.reader.thought_id.clear();
                self.reader.thought_context = None;
                self.reader.note_edit_context = None;
                self.reader.thought_edit_blocked = false;
                self.reader.thought_page = 0;
                self.reader.selection = None;
                self.reader.draft.clear();
                self.set_reader_flag("thought", false);
                self.set_reader_flag("menu", false);
            }
            "toc" => {
                self.reader.toc_tab = if matches!(value, "toc" | "marks" | "notes") {
                    value
                } else {
                    "toc"
                }
                .into();
                self.reader.toc_page =
                    self.reader.chapter_index as usize / self.profile.reader_chapter_page_size();
                self.set_reader_flag("toc", true);
            }
            "toc-close" => self.set_reader_flag("toc", false),
            "toc-tab" => {
                if matches!(value, "toc" | "marks" | "notes") {
                    self.reader.toc_tab = value.into();
                    self.reader.toc_page = 0;
                }
            }
            "toc-page" => {
                let size = if self.reader.toc_tab == "toc" {
                    self.profile.reader_chapter_page_size()
                } else {
                    self.reader_entry_page_size()
                };
                let length = match self.reader.toc_tab.as_str() {
                    "marks" => usize::try_from(self.store.bookmark_count(&self.reader.book_id)?)
                        .unwrap_or(usize::MAX),
                    "notes" => usize::try_from(self.store.annotation_count(&self.reader.book_id)?)
                        .unwrap_or(usize::MAX),
                    _ => self.store.chapter_metadata(&self.reader.book_id)?.len(),
                };
                self.reader.toc_page = move_page(
                    self.reader.toc_page,
                    value.parse().unwrap_or(0),
                    page_count(length, size),
                );
            }
            "toc-locate" => {
                self.reader.toc_tab = "toc".into();
                self.reader.toc_page =
                    self.reader.chapter_index as usize / self.profile.reader_chapter_page_size();
            }
            "open-chapter" => {
                if let Some(chapter) = self
                    .store
                    .chapter_metadata(&self.reader.book_id)?
                    .into_iter()
                    .find(|chapter| chapter.id == value)
                {
                    let id = self.reader.book_id.clone();
                    self.set_reader_flag("toc", false);
                    self.load_reader(&id, chapter.index, Some(0))?;
                    self.confirm_reader_position()?;
                }
            }
            "open-entry" => self.open_reader_entry(value)?,
            "bookmark" => self.toggle_reader_bookmark()?,
            "selection-close" => {
                self.invalidate_reader_press();
                self.set_reader_flag("selection", false);
                self.reader.selection = None;
                self.reader.draft.clear();
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<ReaderModel>().set_writing_thought(false);
                }
            }
            "highlight" => self.save_reader_annotation(false)?,
            "write-thought" => self.begin_reader_thought()?,
            "thought-draft" => {
                if self.reader_thought_editor_blocked() {
                    self.show_toast("api_error");
                    return Ok(());
                }
                if !self
                    .reader
                    .draft
                    .set_query_for_purpose(value, InputPurpose::Thought)
                {
                    self.show_toast("operation_failed");
                }
            }
            "save-thought" => self.save_reader_annotation(true)?,
            "copy" => self.show_toast("copied"),
            "thought" => self.open_reader_thought(value)?,
            "thought-close" => {
                self.reader.thought_context = None;
                self.reader.note_edit_context = None;
                self.set_reader_flag("thought", false);
                self.reader.thought_page = 0;
                self.reader.remote_thoughts.clear();
                if !self.reader_thought_editor_blocked() {
                    self.reader.thought_id.clear();
                    self.reader.thought_edit_blocked = false;
                    self.reader.selection = None;
                    self.reader.draft.clear();
                }
                // Hiding a read-only view does not authorize its retained old
                // draft. A new selection, admitted note or context retires it.
            }
            "thought-page" => {
                let count = self
                    .ui
                    .upgrade()
                    .map(|ui| ui.global::<ReaderModel>().get_thought_pages())
                    .unwrap_or(1);
                self.reader.thought_page = move_page(
                    self.reader.thought_page,
                    value.parse().unwrap_or(0),
                    count.max(1) as usize,
                );
            }
            "thought-entry" => {}
            "popular" => {
                self.prefs.popular = !self.prefs.popular;
                self.save_preferences()?;
                if self.prefs.popular {
                    self.load_reader_underlines(true)?;
                } else {
                    self.reader.underline_request = self.request();
                    self.reader.replace_underlines(Vec::new());
                }
            }
            "mine" => {
                self.prefs.mine = !self.prefs.mine;
                self.save_preferences()?;
            }
            "thought-position" if matches!(value, "side" | "center" | "bottom") => {
                self.apply_reader_layout_preferences(
                    self.reader.preferences.clone(),
                    self.profile.is_scribe() && value == "side",
                )?;
                self.prefs.popup_position = if value == "side" && !self.profile.is_scribe() {
                    "center"
                } else {
                    value
                }
                .into();
                self.save_preferences()?;
            }
            "font" => {
                if self
                    .fonts
                    .iter()
                    .any(|font| font.id == value && font.available)
                {
                    let mut preferences = self.reader.preferences.clone();
                    preferences.font = value.into();
                    self.apply_reader_layout_preferences(preferences, self.reader.notes_panel)?;
                    self.save_reader_preferences()?;
                } else {
                    self.show_toast("font_unavailable");
                }
            }
            "font-size" => {
                let mut preferences = self.prefs.clone();
                preferences.reader = self.reader.preferences.clone();
                preferences.adjust_for_device(
                    "font_size",
                    value.parse().unwrap_or(0),
                    self.profile.is_scribe(),
                );
                self.apply_reader_layout_preferences(preferences.reader, self.reader.notes_panel)?;
                self.save_reader_preferences()?;
            }
            "font-size-select" => {
                let sizes: &[u32] = if self.profile.is_scribe() {
                    &[18, 20, 22, 24, 26, 28, 30, 32, 34, 36, 38]
                } else {
                    &[16, 18, 20, 22, 24, 26, 28, 30, 32]
                };
                if let Ok(size) = value.parse::<u32>() {
                    if sizes.contains(&size) {
                        let mut preferences = self.reader.preferences.clone();
                        preferences.font_size = size;
                        self.apply_reader_layout_preferences(preferences, self.reader.notes_panel)?;
                        self.save_reader_preferences()?;
                    }
                }
            }
            "weight" => {
                if let Ok(weight) = value.parse::<u32>() {
                    if [300, 400, 500, 700].contains(&weight) {
                        let mut preferences = self.reader.preferences.clone();
                        preferences.weight = weight;
                        self.apply_reader_layout_preferences(preferences, self.reader.notes_panel)?;
                        self.save_reader_preferences()?;
                    }
                }
            }
            "margin" => {
                if let Some(index) = ["narrow", "mid", "wide"]
                    .iter()
                    .position(|choice| *choice == value)
                {
                    let mut preferences = self.reader.preferences.clone();
                    preferences.margin = index as u32;
                    self.apply_reader_layout_preferences(preferences, self.reader.notes_panel)?;
                    self.save_reader_preferences()?;
                }
            }
            "line-spacing" => {
                if let Some(index) = ["tight", "mid", "loose"]
                    .iter()
                    .position(|choice| *choice == value)
                {
                    let mut preferences = self.reader.preferences.clone();
                    preferences.line_spacing = index as u32;
                    self.apply_reader_layout_preferences(preferences, self.reader.notes_panel)?;
                    self.save_reader_preferences()?;
                }
            }
            "indent" => {
                if let Ok(indent) = value.parse::<u32>() {
                    if indent <= 2 {
                        let mut preferences = self.reader.preferences.clone();
                        preferences.indent = indent;
                        self.apply_reader_layout_preferences(preferences, self.reader.notes_panel)?;
                        self.save_reader_preferences()?;
                    }
                }
            }
            "paragraph-gap" => {
                if let Some(index) = ["none", "small", "big"]
                    .iter()
                    .position(|choice| *choice == value)
                {
                    let mut preferences = self.reader.preferences.clone();
                    preferences.gap = index as u32;
                    self.apply_reader_layout_preferences(preferences, self.reader.notes_panel)?;
                    self.save_reader_preferences()?;
                }
            }
            "justify" => {
                let mut preferences = self.reader.preferences.clone();
                preferences.justify = !preferences.justify;
                self.apply_reader_layout_preferences(preferences, self.reader.notes_panel)?;
                self.save_reader_preferences()?;
            }
            "book-only" => {
                self.reader.preferences.book_only = !self.reader.preferences.book_only;
                self.save_reader_preferences()?;
            }
            "tap-mode" if matches!(value, "default" | "mirrored" | "swipe") => {
                self.reader.preferences.tap_mode = value.into();
                self.save_reader_preferences()?;
            }
            "refresh" => {
                if let Ok(interval) = value.parse::<u32>() {
                    if [0, 1, 6, 12].contains(&interval) {
                        self.reader.preferences.refresh_interval = interval;
                        self.platform.set_reader_refresh_interval(interval)?;
                        self.save_reader_preferences()?;
                    }
                }
            }
            "brightness" | "warmth" => {
                self.adjust_reader_light(action, value.parse().unwrap_or(0), false)
            }
            "brightness-select" | "warmth-select" => {
                if let Ok(target) = value.parse::<i32>() {
                    if (0..=24).contains(&target) {
                        self.adjust_reader_light(action.trim_end_matches("-select"), target, true);
                    }
                }
            }
            "night" => self.toggle_dark_mode()?,
            "pen" => self.show_toast("hardware_unavailable"),
            "sync" => {
                if self.online && !self.force_offline && self.jobs.capabilities().remote_progress {
                    let request = self.request();
                    self.store.set_setting(
                        &format!("progress.request:{}", self.reader.book_id),
                        &request,
                    )?;
                    self.settle_job_submission(
                        self.jobs.progress(request, self.reader.book_id.clone()),
                    );
                } else {
                    self.show_toast("network_unavailable");
                }
            }
            "resolve-progress" => {
                if !matches!(value, "local" | "cloud") {
                    return Ok(());
                }
                if value == "cloud" {
                    if let Some(position) = self.reader.cloud.clone() {
                        self.apply_cloud_position(&position)?;
                    }
                }
                self.reader.cloud = None;
                self.confirm_reader_position()?;
                self.set_reader_flag("sync-conflict", false);
            }
            "download" => {
                let id = self.reader.book_id.clone();
                self.pending_read = Some((id.clone(), self.reader.chapter_index));
                self.enqueue_download(&id, vec![self.reader.chapter_index])?;
            }
            "refresh-access" => {
                let id = self.reader.book_id.clone();
                self.refresh_content_access(&id, vec![self.reader.chapter_index])?;
            }
            "load-image" => self.load_reader_image(value)?,
            "delete-annotation" => {
                if self
                    .store
                    .annotation_header_by_id_with_limits(
                        &self.reader.book_id,
                        value,
                        note_header_limits(),
                    )?
                    .is_some()
                {
                    self.confirm_action(
                        &format!("delete-note:{value}"),
                        "delete_note",
                        "delete_note_confirm",
                        "delete_note",
                    );
                    self.set_reader_flag("thought", false);
                    self.reader.selection = None;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn set_reader_flag(&self, flag: &str, enabled: bool) {
        if let Some(ui) = self.ui.upgrade() {
            let state = ui.global::<ReaderModel>();
            match flag {
                "menu" => state.set_menu(enabled),
                "toc" => state.set_toc_open(enabled),
                "thought" => state.set_thought_open(enabled),
                "selection" => state.set_selection_open(enabled),
                "sync-conflict" => state.set_sync_conflict(enabled),
                _ => {}
            }
        }
    }

    fn save_reader_preferences(&mut self) -> Result<()> {
        let key = format!("reader.preferences:{}", self.reader.book_id);
        if self.reader.preferences.book_only {
            self.store.set_setting(&key, &self.reader.preferences)?;
        } else {
            self.store.delete_setting(&key)?;
            self.prefs.reader = self.reader.preferences.clone();
            self.save_preferences()?;
        }
        Ok(())
    }

    fn turn_reader(&mut self, delta: i32) -> Result<()> {
        if delta == 0 {
            return Ok(());
        }
        self.invalidate_reader_press();
        self.reader.selection = None;
        let turned = self.reader.paginator.as_mut().is_some_and(|p| {
            if delta > 0 {
                p.next_page().is_some()
            } else {
                p.previous_page().is_some()
            }
        });
        if turned {
            self.confirm_reader_position()?;
            self.persist_reader_position()?;
            self.refresh_display(RefreshReason::ReaderTurn);
            if self.prefs.prefetch
                && self.online
                && !self.force_offline
                && self.jobs.capabilities().chapter_content
                && self
                    .reader
                    .paginator
                    .as_ref()
                    .is_some_and(|p| p.current_index() + 1 == p.pages.len())
            {
                let chapters = self.store.chapter_metadata(&self.reader.book_id)?;
                if let Some(next) = chapters
                    .iter()
                    .find(|chapter| chapter.index > self.reader.chapter_index && !chapter.cached)
                {
                    let id = self.reader.book_id.clone();
                    self.enqueue_download(&id, vec![next.index])?;
                }
            }
        } else {
            self.change_reader_chapter(delta)?;
        }
        Ok(())
    }

    fn change_reader_chapter(&mut self, delta: i32) -> Result<()> {
        let chapters = self.store.chapter_metadata(&self.reader.book_id)?;
        let Some(current) = chapters
            .iter()
            .position(|chapter| chapter.index == self.reader.chapter_index)
        else {
            return Ok(());
        };
        let next = current as i64 + if delta < 0 { -1 } else { 1 };
        if next < 0 {
            return Ok(());
        }
        let Some(chapter) = chapters.get(next as usize) else {
            if delta > 0 {
                if let Some(book) = self.store.book_metadata_by_id(&self.reader.book_id)? {
                    let status = self.chapter_metadata_content_status(&book, &chapters[current]);
                    if status != ContentStatus::Full {
                        self.show_toast(
                            if matches!(status, ContentStatus::Preview | ContentStatus::Locked) {
                                "content_unlock_required"
                            } else {
                                "content_verify_required"
                            },
                        );
                        return Ok(());
                    }
                }
                self.store
                    .set_book_progress(&self.reader.book_id, 1.0, true)?;
                self.show_toast("finished");
            }
            return Ok(());
        };
        let id = self.reader.book_id.clone();
        self.load_reader(
            &id,
            chapter.index,
            Some(if delta < 0 { u32::MAX } else { 0 }),
        )?;
        self.confirm_reader_position()?;
        self.refresh_display(RefreshReason::ReaderTurn);
        Ok(())
    }

    fn release_reader_pointer(&mut self, value: &str) -> Result<()> {
        if !matches!(self.route, Route::Reader(..)) {
            if self.reader.press.is_some_and(ReaderPress::is_selecting) {
                self.reader.selection = None;
            }
            self.invalidate_reader_press();
            return Ok(());
        }
        let Some(mut press) = self.reader.press else {
            return Ok(());
        };
        press.latest = unbounded_pointer(value).unwrap_or(press.latest);
        self.reader.press = Some(press);
        self.activate_reader_selection(None);
        if self.reader.press.is_some_and(ReaderPress::is_selecting) {
            // The final coordinate bypasses coalescing, including a final move
            // delivered with Up instead of a separate Move event.
            self.flush_reader_selection_preview();
            self.invalidate_reader_press();
            self.reader.suppress_tap = true;
            if let Some(ui) = self.ui.upgrade() {
                ui.global::<ReaderModel>().set_selection_open(true);
            }
            self.refresh_display(RefreshReason::Content);
            return Ok(());
        }
        let press = self.reader.press.unwrap_or(press);
        self.invalidate_reader_press();
        if matches!(press.phase, ReaderPressPhase::Rejected)
            || press.exceeds_hold_movement(self.reader_text_width())
        {
            self.reader.suppress_tap = true;
            return Ok(());
        }
        let edge = self.prefs.edge_region as f32 / 100.0;
        let protected =
            self.prefs.edge_guard && (press.latest.0 < edge || press.latest.0 > 1.0 - edge);
        if !protected {
            if let Some(id) = self.reader_hit_mark(press.latest.0, press.latest.1)? {
                self.open_reader_thought(&id)?;
                self.reader.suppress_tap = true;
            }
        }
        Ok(())
    }

    pub(super) fn invalidate_reader_press(&mut self) {
        self.reader.press = None;
        self.advance_reader_press_generation();
    }

    fn advance_reader_press_generation(&self) -> i32 {
        if let Some(ui) = self.ui.upgrade() {
            let state = ui.global::<ReaderModel>();
            let generation = state.get_press_generation().wrapping_add(1);
            state.set_press_generation(generation);
            generation
        } else {
            0
        }
    }

    fn schedule_reader_pointer_event(&self, event: &'static str, generation: i32, delay: Duration) {
        let weak = self.ui.clone();
        slint::Timer::single_shot(delay, move || {
            if let Some(ui) = weak.upgrade() {
                let state = ui.global::<ReaderModel>();
                if state.get_press_generation() == generation {
                    state.invoke_event(event.into(), generation.to_string().into());
                }
            }
        });
    }

    fn activate_reader_selection(&mut self, timer_generation: Option<i32>) -> bool {
        if !matches!(self.route, Route::Reader(..)) {
            return false;
        }
        let Some(mut press) = self.reader.press else {
            return false;
        };
        // A matching one-shot has already reached its deadline on Slint's clock.
        // Direct release handling also recognizes a hold if the timer is delayed.
        if !matches!(press.phase, ReaderPressPhase::Pending)
            || timer_generation.is_some_and(|generation| generation != press.generation)
            || (timer_generation.is_none() && press.started.elapsed() < Duration::from_millis(500))
        {
            return false;
        }
        if press.exceeds_hold_movement(self.reader_text_width()) {
            press.phase = ReaderPressPhase::Rejected;
            self.reader.press = Some(press);
            self.advance_reader_press_generation();
            return false;
        }
        let Some(anchor) = self.reader_range_at_pointer(press.origin.0, press.origin.1, false)
        else {
            press.phase = ReaderPressPhase::Rejected;
            self.reader.press = Some(press);
            return false;
        };
        let Some(line) = self.reader.paginator.as_ref().and_then(|paginator| {
            paginator
                .current_page()
                .lines
                .iter()
                .find(|line| line.start <= anchor.0 && anchor.0 < line.end)
        }) else {
            return false;
        };
        let initial = match self.reader_initial_range(line, anchor) {
            Ok(initial) => initial,
            Err(_) => {
                press.phase = ReaderPressPhase::Rejected;
                self.reader.press = Some(press);
                self.show_toast("api_error");
                return false;
            }
        };
        press.phase = ReaderPressPhase::Selecting {
            anchor,
            initial,
            dragged: false,
        };
        self.reader.press = Some(press);
        self.reader.thought_id.clear();
        self.reader.note_edit_context = None;
        self.reader.thought_edit_blocked = false;
        self.reader.draft.clear();
        self.reader.suppress_tap = true;
        if let Some(ui) = self.ui.upgrade() {
            let state = ui.global::<ReaderModel>();
            state.set_selection_open(false);
            state.set_writing_thought(false);
            state.set_thought_open(false);
            state.set_menu(false);
        }
        self.flush_reader_selection_preview();
        true
    }

    fn move_reader_pointer(&mut self, value: &str) {
        if !matches!(self.route, Route::Reader(..)) {
            if self.reader.press.is_some_and(ReaderPress::is_selecting) {
                self.reader.selection = None;
            }
            self.invalidate_reader_press();
            return;
        }
        let Some(mut press) = self.reader.press else {
            return;
        };
        let Some(point) = unbounded_pointer(value) else {
            return;
        };
        press.latest = point;
        match press.phase {
            ReaderPressPhase::Pending => {
                if press.exceeds_hold_movement(self.reader_text_width()) {
                    press.phase = ReaderPressPhase::Rejected;
                    self.reader.suppress_tap = true;
                    self.advance_reader_press_generation();
                }
                self.reader.press = Some(press);
            }
            ReaderPressPhase::Rejected => self.reader.press = Some(press),
            ReaderPressPhase::Selecting {
                anchor,
                initial,
                mut dragged,
            } => {
                if let Some(focus) = self.reader_range_at_pointer(point.0, point.1, true) {
                    dragged |= focus != anchor;
                }
                press.phase = ReaderPressPhase::Selecting {
                    anchor,
                    initial,
                    dragged,
                };
                let schedule = !press.preview_pending;
                press.preview_pending = true;
                self.reader.press = Some(press);
                if schedule {
                    self.schedule_reader_pointer_event(
                        "reader.preview",
                        press.generation,
                        READER_PREVIEW_DELAY,
                    );
                }
            }
        }
    }

    fn flush_reader_selection_preview(&mut self) -> bool {
        let Some(mut press) = self.reader.press else {
            return false;
        };
        let ReaderPressPhase::Selecting {
            anchor,
            initial,
            mut dragged,
        } = press.phase
        else {
            return false;
        };
        press.preview_pending = false;
        let Some(focus) = self.reader_range_at_pointer(press.latest.0, press.latest.1, true) else {
            self.reader.press = Some(press);
            return false;
        };
        dragged |= focus != anchor;
        let range = if dragged {
            (anchor.0.min(focus.0), anchor.1.max(focus.1))
        } else {
            initial
        };
        press.phase = ReaderPressPhase::Selecting {
            anchor,
            initial,
            dragged,
        };
        self.reader.press = Some(press);
        if self.reader.selection == Some(range) {
            return false;
        }
        self.reader.selection = Some(range);
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<ReaderModel>()
                .set_selection_text(self.reader_selection_preview(range.0, range.1).into());
        }
        true
    }

    fn reader_text_width(&self) -> f32 {
        self.platform.logical_viewport_size().0 as f32
            - if self.profile.is_scribe() && self.reader.notes_panel {
                300.0
            } else {
                0.0
            }
    }

    fn reader_caret_budget(&self) -> CaretBudget {
        #[cfg(test)]
        let work = self.reader.caret_work_limit;
        #[cfg(not(test))]
        let work = crate::reader::DEFAULT_CARET_WORK;
        CaretBudget::with_work_limit(work)
    }

    fn record_reader_caret_budget(&self, budget: &CaretBudget) {
        #[cfg(test)]
        {
            let mut stats = self.reader.caret_stats.get();
            let next = budget.stats();
            stats.admitted += next.admitted;
            stats.allocations += next.allocations;
            stats.measured += next.measured;
            stats.boundaries += next.boundaries;
            stats.rejected += next.rejected;
            self.reader.caret_stats.set(stats);
        }
        #[cfg(not(test))]
        let _ = budget;
    }

    fn reader_range_at_pointer(&self, x: f32, y: f32, clamp: bool) -> Option<(u32, u32)> {
        let (layout, _) = self.reader_geometry();
        let text_width = self.reader_text_width();
        let paginator = self.reader.paginator.as_ref()?;
        let font = self.reader_font()?;
        let page = paginator.current_page();
        let margin = reader_margin(self.profile.is_scribe(), self.reader.preferences.margin);
        let metrics = font.metrics_for_weight(self.reader.preferences.weight);
        let local_x = x * text_width - margin;
        let mut budget = self.reader_caret_budget();
        let range = if clamp {
            page.text_range_at_point_with_budget(local_x, y, &layout, metrics, &mut budget)
        } else {
            let line = line_at_y(&page.lines, y, layout.line_advance())?;
            line.caret_geometry(&layout, metrics, &mut budget)
                .ok()
                .and_then(|geometry| geometry.range_at_x(local_x))
        };
        self.record_reader_caret_budget(&budget);
        let range = range?;
        (range.0 < range.1).then_some(range)
    }

    pub(super) fn reader_hit_mark(&mut self, x: f32, y: f32) -> Result<Option<String>> {
        let remote_ranges = self.reader_page_underlines();
        let (layout, _) = self.reader_geometry();
        let text_width = self.reader_text_width();
        let Some(paginator) = &self.reader.paginator else {
            return Ok(None);
        };
        if paginator.current_page().lines.len() > crate::reader::MAX_CARET_PAGE_LINES {
            return Ok(None);
        }
        let Some(font) = self.reader_font() else {
            return Ok(None);
        };
        let Some(line) = line_at_y(&paginator.current_page().lines, y, layout.line_advance())
        else {
            return Ok(None);
        };
        let local_x = x * text_width
            - reader_margin(self.profile.is_scribe(), self.reader.preferences.margin);
        let metrics = font.metrics_for_weight(self.reader.preferences.weight);
        let notes = if self.prefs.mine {
            self.note_geometry(
                &self.reader.book_id,
                self.reader.chapter_index,
                paginator.current_page().start,
                paginator.current_page().end,
            )?
        } else {
            Vec::new()
        };
        if self
            .page_mark_output_plan(&paginator.current_page().lines, &notes, &remote_ranges)
            .is_err()
        {
            return Ok(None);
        }
        if !notes.iter().any(|note| {
            note.chapter_index == self.reader.chapter_index
                && intersects(line.start, line.end, note.start, note.end)
        }) && !remote_ranges
            .iter()
            .any(|range| intersects(line.start, line.end, range.start, range.end))
        {
            return Ok(None);
        }
        let mut budget = self.reader_caret_budget();
        let mut reservation = None;
        let mut rejected = false;
        for candidate in &paginator.current_page().lines {
            let needs_geometry =
                self.reader.selection.is_some_and(|(start, end)| {
                    intersects(candidate.start, candidate.end, start, end)
                }) || notes.iter().any(|note| {
                    note.chapter_index == self.reader.chapter_index
                        && intersects(candidate.start, candidate.end, note.start, note.end)
                }) || remote_ranges.iter().any(|range| {
                    intersects(candidate.start, candidate.end, range.start, range.end)
                });
            if !needs_geometry {
                continue;
            }
            match budget.admit(candidate) {
                Ok(admitted) => {
                    if std::ptr::eq(candidate, line) {
                        reservation = Some(admitted);
                    }
                }
                Err(_) => {
                    rejected = true;
                    break;
                }
            }
        }
        let geometry = if rejected {
            None
        } else {
            reservation.and_then(|reservation| {
                line.caret_geometry_with_reservation(&layout, metrics, &mut budget, reservation)
                    .ok()
            })
        };
        self.record_reader_caret_budget(&budget);
        let Some(geometry) = geometry else {
            return Ok(None);
        };
        let hit = |start: u32, end: u32| {
            intersects(line.start, line.end, start, end)
                && local_x >= geometry.x_at_offset(start)
                && local_x <= geometry.x_at_offset(end)
        };
        if self.prefs.mine {
            if let Some(note) = notes.into_iter().find(|note| {
                note.chapter_index == self.reader.chapter_index && hit(note.start, note.end)
            }) {
                let header = self
                    .store
                    .annotation_header_for_geometry(&self.reader.book_id, &note)?
                    .ok_or(AnnotationUnavailable)?;
                return Ok(Some(format!("mine:{}", header.key.id)));
            }
        }
        if self.prefs.popular {
            for range in remote_ranges {
                if hit(range.start, range.end) {
                    return Ok(Some(format!("remote:{}", range.index)));
                }
            }
        }
        Ok(None)
    }

    fn toggle_reader_bookmark(&mut self) -> Result<()> {
        let Some(page) = self.reader.paginator.as_ref().map(|p| p.current_page()) else {
            return Ok(());
        };
        let existing = self
            .store
            .bookmark_geometry_for_range(
                &self.reader.book_id,
                self.reader.chapter_index,
                page.start,
                page.end.max(page.start.saturating_add(1)),
            )?
            .into_iter()
            .next();
        if let Some(mark) = existing {
            let header = self
                .store
                .bookmark_header_for_geometry(&self.reader.book_id, &mark)?
                .ok_or(BookmarkUnavailable)?;
            self.store.delete_bookmark(&header.key.id)?;
            self.show_toast("bookmark_removed");
        } else {
            let label = self
                .store
                .chapter_title(&self.reader.book_id, self.reader.chapter_index)?
                .unwrap_or_default();
            self.store.save_bookmark(&Bookmark {
                id: unique_id("bookmark"),
                book_id: self.reader.book_id.clone(),
                chapter_index: self.reader.chapter_index,
                char_offset: page.start,
                label,
                created_at: Utc::now().timestamp(),
            })?;
            self.show_toast("bookmark_added");
        }
        Ok(())
    }

    fn open_reader_entry(&mut self, value: &str) -> Result<()> {
        let id = self.reader.book_id.clone();
        if let Some(id_value) = value.strip_prefix("bookmark:") {
            if let Some(mark) = self
                .store
                .bookmark_geometry_by_id(&id, id_value)?
                .filter(|mark| mark.chapter_index != u32::MAX)
            {
                self.load_reader(&id, mark.chapter_index, Some(mark.char_offset))?;
                self.set_reader_flag("toc", false);
                self.confirm_reader_position()?;
            }
        }
        if let Some(id_value) = value.strip_prefix("annotation:") {
            if let Some(note) = self
                .store
                .annotation_header_by_id_with_limits(&id, id_value, note_header_limits())?
                .filter(|note| note.chapter_index != u32::MAX)
            {
                self.load_reader(&id, note.chapter_index, Some(note.start))?;
                self.set_reader_flag("toc", false);
                self.confirm_reader_position()?;
            }
        }
        Ok(())
    }

    fn save_reader_annotation(&mut self, with_note: bool) -> Result<()> {
        if self.reader_thought_editor_blocked() {
            self.show_toast("api_error");
            return Ok(());
        }
        let Some((start, end)) = self.reader.selection else {
            return Ok(());
        };
        let mut draft = if with_note {
            if !self.reader.draft.is_admitted() {
                self.show_toast("operation_failed");
                return Ok(());
            }
            Some(self.reader.draft.clone())
        } else {
            None
        };
        if draft
            .as_mut()
            .is_some_and(|draft| !draft.set_purpose(InputPurpose::Thought) || !draft.commit_first())
        {
            self.show_toast("operation_failed");
            return Ok(());
        }
        let note = if with_note {
            draft.as_ref().unwrap().query.trim().to_owned()
        } else {
            String::new()
        };
        if with_note && note.is_empty() {
            return Ok(());
        }
        let editing = if with_note {
            self.local_note_edit_header()?
        } else {
            None
        };
        let created_at = Utc::now().timestamp();
        if let Some(header) = editing {
            // Editing this admitted note changes only its text. The fenced
            // legacy quote can exceed display/preview allowances and must not
            // be copied, guessed from the resident body, or overwritten.
            if !self.store.update_annotation_note_preserving_quote(
                &self.reader.book_id,
                &header,
                &note,
                created_at,
            )? {
                return Err(AnnotationUnavailable.into());
            }
        } else {
            let mut operation = SelectionOperation::new();
            let quote = selected_annotation_quote(
                &self.reader.body,
                start,
                end,
                note_record_limits()
                    .quote_bytes
                    .min(AnnotationLimits::records().quote_bytes)
                    .min(operation.limits.preview_bytes),
                &mut operation,
            );
            self.record_selection_operation(&operation, quote.is_err());
            let quote = quote?;
            if quote.trim().is_empty() {
                return Ok(());
            }
            let existing = self.store.annotation_header_for_exact_range(
                &self.reader.book_id,
                self.reader.chapter_index,
                start,
                end,
            )?;
            if !with_note && existing.is_some() {
                if !self.store.update_annotation_quote_preserving_note(
                    &self.reader.book_id,
                    existing.as_ref().unwrap(),
                    &quote,
                    created_at,
                )? {
                    return Err(AnnotationUnavailable.into());
                }
            } else {
                self.store.save_annotation(&Annotation {
                    id: existing
                        .as_ref()
                        .map(|item| item.key.id.clone())
                        .unwrap_or_else(|| unique_id("note")),
                    book_id: self.reader.book_id.clone(),
                    chapter_index: self.reader.chapter_index,
                    start,
                    end,
                    quote,
                    note,
                    created_at,
                })?;
            }
        }
        self.set_reader_flag("selection", false);
        self.reader.selection = None;
        self.reader.draft.clear();
        self.reader.note_edit_context = None;
        if let Some(ui) = self.ui.upgrade() {
            let state = ui.global::<ReaderModel>();
            state.set_writing_thought(false);
        }
        self.refresh_display(RefreshReason::Content);
        self.show_toast(if with_note {
            "thought_saved"
        } else {
            "annotation_saved"
        });
        Ok(())
    }

    fn begin_reader_thought(&mut self) -> Result<()> {
        if self.reader_thought_editor_blocked() {
            self.show_toast("api_error");
            return Ok(());
        }
        if self.reader.selection.is_none() {
            return Ok(());
        }
        if !self.reader.draft.set_purpose(InputPurpose::Thought) {
            self.show_toast("operation_failed");
            return Ok(());
        }
        if let Some(ui) = self.ui.upgrade() {
            let state = ui.global::<ReaderModel>();
            state.set_selection_open(true);
            state.set_writing_thought(true);
            state.set_thought_open(false);
            ui.global::<SearchModel>().set_keyboard_visible(true);
        }
        self.search_purpose = "thought".into();
        self.reader.thought_context = None;
        self.query = self.reader.draft.clone();
        self.navigate(Route::Search, true)
    }

    fn open_reader_thought(&mut self, value: &str) -> Result<()> {
        if self.prefs.mine {
            if let Some((start, end)) = self
                .reader
                .paginator
                .as_ref()
                .map(|paginator| (paginator.current_page().start, paginator.current_page().end))
            {
                // The following render needs this geometry. Reject its metadata
                // allowance before a new local view can bind the old editor.
                self.note_geometry(&self.reader.book_id, self.reader.chapter_index, start, end)?;
            }
        }
        let remote_range = if value.starts_with("remote:") {
            let Some(index) = value
                .strip_prefix("remote:")
                .and_then(|index| index.parse::<usize>().ok())
            else {
                return Ok(());
            };
            let Some(range) = self
                .reader_page_underlines()
                .into_iter()
                .find(|range| range.index == index)
            else {
                return Ok(());
            };
            Some((range.start, range.end))
        } else {
            None
        };
        let mut note = None;
        let mut note_context = None;
        let mut edit_blocked = false;
        if let Some(id) = value.strip_prefix("mine:") {
            let Some(header) = self.store.annotation_header_by_id_with_limits(
                &self.reader.book_id,
                id,
                note_header_limits(),
            )?
            else {
                return Ok(());
            };
            let metadata = self
                .store
                .annotation_text_metadata_with_limits(
                    &self.reader.book_id,
                    &header.key,
                    note_record_limits(),
                )?
                .ok_or(AnnotationUnavailable)?;
            if metadata.header != header {
                return Err(AnnotationUnavailable.into());
            }
            let chapter_id = self
                .store
                .chapter_uid(&self.reader.book_id, header.chapter_index)?
                .ok_or(AnnotationUnavailable)?;
            if matches!(metadata.note, AnnotationFieldState::Available { .. }) {
                let text = self
                    .store
                    .annotation_note_by_key(
                        &self.reader.book_id,
                        &header.key,
                        super::thoughts::OPEN_TEXT_BYTES,
                    )?
                    .ok_or(AnnotationUnavailable)?;
                edit_blocked = !self
                    .reader
                    .draft
                    .set_query_for_purpose(&text, InputPurpose::Thought);
                if !edit_blocked {
                    note_context = Some(LocalNoteEditContext {
                        header: header.clone(),
                        book_id: self.reader.book_id.clone(),
                        chapter_id,
                        body_revision: self.reader.underline_body_revision,
                        account_epoch: self.jobs.account_epoch(),
                    });
                }
            } else {
                edit_blocked = true;
            }
            note = Some(header);
        }
        if edit_blocked {
            // The existing view applies its separate bounded display policy.
            // Rejected editable text cannot rebind the old selection/draft.
            self.invalidate_reader_press();
            if let Some(ui) = self.ui.upgrade() {
                let state = ui.global::<ReaderModel>();
                state.set_selection_open(false);
                state.set_writing_thought(false);
            }
            self.show_toast("api_error");
        }
        let request = self.request();
        self.reader.thought_request = request;
        self.reader.thought_context = None;
        self.reader.note_edit_context = note_context;
        self.reader.thought_id = value.into();
        self.reader.thought_edit_blocked = edit_blocked;
        self.reader.thought_page = 0;
        self.reader.remote_thoughts.clear();
        self.set_reader_flag("thought", true);
        self.set_reader_flag("menu", false);
        if value.starts_with("mine:") && !edit_blocked {
            if let Some(note) = note {
                self.reader.selection = Some((note.start, note.end));
            }
        } else if let Some(underline) = value
            .strip_prefix("remote:")
            .and_then(|index| index.parse::<usize>().ok())
            .and_then(|index| self.reader.underlines.get(index))
            .cloned()
        {
            self.reader.selection = remote_range;
            if let Some(chapter_id) = self
                .store
                .chapter_uid(&self.reader.book_id, self.reader.chapter_index)?
            {
                let cache_key =
                    format!("thoughts:{}:{}:{}", self.reader.book_id, chapter_id, value);
                self.reader.thought_context = Some(ThoughtRequestContext {
                    request,
                    book_id: self.reader.book_id.clone(),
                    chapter_id: chapter_id.clone(),
                    view_id: value.to_owned(),
                    range: underline.range.clone(),
                    cache_key: cache_key.clone(),
                    body_revision: self.reader.underline_body_revision,
                    underline_revision: self.reader.underline_revision,
                    account_epoch: self.jobs.account_epoch(),
                });
                self.reader.remote_thoughts = match self.store.bounded_array_setting(
                    &cache_key,
                    super::thoughts::CACHE_BYTES,
                    super::thoughts::CACHE_RECORDS,
                ) {
                    Ok(reviews) => reviews.unwrap_or_default(),
                    Err(_) => {
                        self.show_toast("api_error");
                        Vec::new()
                    }
                };
                if self.online
                    && !self.force_offline
                    && !self.demo
                    && self
                        .jobs
                        .session()
                        .is_some_and(|session| session.has_api_key())
                {
                    self.settle_job_submission(self.jobs.thoughts(
                        request,
                        self.reader.book_id.clone(),
                        chapter_id,
                        vec![underline.range],
                    ));
                }
            }
        }
        Ok(())
    }

    fn reader_thought_editor_blocked(&mut self) -> bool {
        // Other context owners can retire the selected ID, including catalog
        // removal. Do not let that old local-view gate block a new document.
        if !self.reader.thought_id.starts_with("mine:") {
            self.reader.thought_edit_blocked = false;
        }
        self.reader.thought_edit_blocked
    }

    pub(super) fn query_reader_light(&mut self) {
        self.request_device_status();
    }

    fn adjust_reader_light(&mut self, kind: &str, value: i32, absolute: bool) {
        self.change_device_light(kind, value, absolute);
    }

    fn remote_catalog(&self, book_id: &str) -> Result<Vec<source::RemoteChapter>> {
        Ok(self
            .store
            .setting(&format!("catalog.remote:{book_id}"))?
            .unwrap_or_default())
    }

    pub(super) fn confirm_reader_position(&mut self) -> Result<()> {
        self.store
            .delete_setting(&format!("progress.request:{}", self.reader.book_id))?;
        self.reader.position_confirmed = true;
        Ok(())
    }

    fn reader_entry_page_size(&self) -> usize {
        let reserved = if self.profile.is_scribe() {
            96 + 52 + 64
        } else {
            96 + 52 + 56
        };
        let row_height = if self.reader.toc_tab == "notes" {
            if self.profile.is_scribe() { 136 } else { 128 }
        } else {
            80
        };
        let (_, height) = self.platform.logical_viewport_size();
        ((height.saturating_sub(reserved) / row_height).max(1) as usize).min(16)
    }

    fn chapter_word_count(&self, book_id: &str, index: u32) -> Result<u32> {
        let Some(uid) = self.store.chapter_uid(book_id, index)? else {
            return Ok(0);
        };
        Ok(self
            .remote_catalog(book_id)?
            .iter()
            .find(|remote| remote.uid == uid)
            .map_or(0, |remote| remote.word_count))
    }

    fn local_book_fraction(&self, offset: u32) -> Result<f32> {
        let source = self.store.book_source(&self.reader.book_id)?;
        let access = self
            .store
            .chapter_access_metadata(&self.reader.book_id, self.reader.chapter_index)?;
        let owner = source
            .as_deref()
            .filter(|source| *source == "weread")
            .and_then(|_| self.chapter_content_owner());
        self.local_book_fraction_with_access(
            offset,
            source.as_deref(),
            access.as_ref(),
            owner.as_deref(),
        )
    }

    fn local_book_fraction_with_access(
        &self,
        offset: u32,
        source: Option<&str>,
        access: Option<&ChapterAccessMetadata>,
        owner: Option<&str>,
    ) -> Result<f32> {
        let chapters = self.store.chapter_metadata(&self.reader.book_id)?;
        let Some(rank) = chapters
            .iter()
            .position(|chapter| chapter.index == self.reader.chapter_index)
        else {
            return Ok(0.0);
        };
        let characters = self.reader.body.chars().count() as u32;
        let complete = source.zip(access).is_some_and(|(source, access)| {
            library::chapter_content_status_for_access_owner(source, access, owner)
                == ContentStatus::Full
        });
        let catalog = self.remote_catalog(&self.reader.book_id)?;
        let denominator = if complete {
            characters
        } else {
            let words = catalog
                .iter()
                .find(|chapter| chapter.uid == chapters[rank].id)
                .map_or(0, |chapter| chapter.word_count);
            if words == 0 {
                0
            } else {
                words.max(characters.saturating_add(1))
            }
        };
        if let Some(fraction) = weighted_book_fraction(
            &chapters,
            &catalog,
            self.reader.chapter_index,
            offset.min(characters),
            denominator,
        ) {
            return Ok(if complete {
                fraction
            } else {
                fraction.min(0.999_999)
            });
        }
        let fraction = if denominator == 0 {
            0.0
        } else {
            (offset.min(characters) as f32 / denominator as f32).clamp(0.0, 1.0)
        };
        Ok(((rank as f32 + fraction) / chapters.len().max(1) as f32)
            .clamp(0.0, if complete { 1.0 } else { 0.999_999 }))
    }

    pub(super) fn persist_reader_position(&self) -> Result<()> {
        if self.reader.book_id.is_empty() || self.reader.body.is_empty() {
            return Ok(());
        }
        let offset = self
            .reader
            .paginator
            .as_ref()
            .map_or(0, |p| p.current_page().start);
        let updated_at = if self.reader.position_confirmed {
            Utc::now().timestamp()
        } else {
            self.store
                .position(&self.reader.book_id)?
                .filter(|previous| {
                    previous.chapter_index == self.reader.chapter_index
                        && previous.char_offset == offset
                })
                .map_or(0, |previous| previous.updated_at)
        };
        self.store.save_position(&ReadingPosition {
            book_id: self.reader.book_id.clone(),
            chapter_index: self.reader.chapter_index,
            char_offset: offset,
            updated_at,
            device: self.profile.name().into(),
        })?;
        let finished = self
            .store
            .book_metadata_by_id(&self.reader.book_id)?
            .is_some_and(|book| book.finished);
        self.store.set_book_progress(
            &self.reader.book_id,
            if finished {
                1.0
            } else {
                self.local_book_fraction(offset)?
            },
            finished,
        )?;
        Ok(())
    }

    pub(super) fn finish_reading(&mut self) -> Result<()> {
        // A later library exit must not make an old, unchanged position newer.
        if self.reader.session_started.is_none() {
            return Ok(());
        }
        self.persist_reader_position()?;
        self.store
            .delete_setting(&format!("progress.request:{}", self.reader.book_id))?;
        let Some(started) = self.reader.session_started else {
            return Ok(());
        };
        let ended = Utc::now().timestamp().max(started);
        let seconds = self
            .reader
            .session_seconds
            .min(ended.saturating_sub(started) as u64);
        let local_session_id = self.active_read_report_session_id();
        let already_saved = match self.finish_read_report_session() {
            Ok(id) => id.is_some(),
            Err(error) => {
                self.note_read_report_error(&error);
                false
            }
        };
        if seconds > 0 && !already_saved {
            self.store.record_session(&ReadingSession {
                id: local_session_id.unwrap_or_else(|| unique_id("reading")),
                book_id: self.reader.book_id.clone(),
                started_at: started,
                ended_at: ended,
                seconds,
            })?;
        }
        self.reader.session_started = None;
        self.reader.session_seconds = 0;
        if self.prefs.push_on_close
            && self.reader.position_confirmed
            && !self.demo
            && self.online
            && !self.force_offline
            && self
                .jobs
                .session()
                .is_some_and(|session| session.has_web_session())
        {
            if let Some(position) = self.reader_upload_position()? {
                let request = self.request();
                self.settle_job_submission(self.jobs.upload_progress(
                    request,
                    self.reader.book_id.clone(),
                    position,
                    0,
                ));
            }
        }
        Ok(())
    }

    pub(super) fn reader_upload_position(&self) -> Result<Option<source::ReadingPosition>> {
        let Some(access) = self
            .store
            .chapter_access_metadata(&self.reader.book_id, self.reader.chapter_index)?
        else {
            return Ok(None);
        };
        let Some(source) = self.store.book_source(&self.reader.book_id)? else {
            return Ok(None);
        };
        let owner = if source == "weread" {
            self.chapter_content_owner()
        } else {
            None
        };
        if library::chapter_content_status_for_access_owner(&source, &access, owner.as_deref())
            != ContentStatus::Full
        {
            return Ok(None);
        }
        let catalog = self.remote_catalog(&self.reader.book_id)?;
        let Some(remote) = catalog
            .iter()
            .find(|remote| remote.uid == access.chapter.id)
        else {
            return Ok(None);
        };
        if remote.word_count == 0 {
            return Ok(None);
        }
        let offset = self
            .reader
            .paginator
            .as_ref()
            .map_or(0, |p| p.current_page().start);
        Ok(Some(source::ReadingPosition {
            chapter_uid: access.chapter.id.clone(),
            chapter_index: remote.index,
            chapter_offset: local_to_remote_offset(
                offset,
                self.reader.body.chars().count() as u32,
                remote.word_count,
            ),
            progress: (self.local_book_fraction_with_access(
                offset,
                Some(&source),
                Some(&access),
                owner.as_deref(),
            )? * 100.0)
                .floor() as u32,
            summary: access
                .chapter
                .title
                .chars()
                .take(reporting::REPORT_SUMMARY_SCALARS)
                .collect(),
            updated_at: if self.reader.position_confirmed {
                Utc::now().timestamp()
            } else {
                0
            },
            device: self.profile.name().into(),
        }))
    }

    pub(super) fn apply_cloud_position(
        &mut self,
        position: &source::ReadingPosition,
    ) -> Result<()> {
        let id = self.reader.book_id.clone();
        let chapters = self.store.chapter_metadata(&id)?;
        let catalog = self.remote_catalog(&id)?;
        let Some(index) = cloud_chapter_index(position, &catalog, &chapters) else {
            self.show_toast("unsupported_content");
            return Ok(());
        };
        let body = self.store.chapter_body(&id, index)?.unwrap_or_default();
        let complete = self.store.book_metadata_by_id(&id)?.is_some_and(|book| {
            chapters
                .iter()
                .find(|chapter| chapter.index == index)
                .is_some_and(|chapter| {
                    self.chapter_metadata_content_status(&book, chapter) == ContentStatus::Full
                })
        });
        if body.is_empty() || !complete {
            self.store
                .set_setting(&format!("progress.pending:{id}"), position)?;
            self.load_reader(&id, index, None)?;
        } else {
            let offset = remote_to_local_offset(
                position.chapter_offset,
                self.chapter_word_count(&id, index)?,
                body.chars().count() as u32,
            );
            self.load_reader(&id, index, Some(offset))?;
            self.persist_reader_position()?;
        }
        self.confirm_reader_position()?;
        Ok(())
    }
}

fn reader_margin(scribe: bool, index: u32) -> f32 {
    (if scribe {
        [40.0, 64.0, 96.0]
    } else {
        [20.0, 32.0, 52.0]
    })[index.min(2) as usize]
}

/// Limit the visible quote to measured lines without changing its source anchor.
fn thought_quote_lines(
    quote: &str,
    width: f32,
    font_size: f32,
    metrics: &TextMetrics,
) -> Result<ModelRc<slint::SharedString>> {
    if quote.is_empty() {
        return Ok(ModelRc::default());
    }
    let layout = Layout {
        width,
        height: 10_000.0,
        font_size,
        line_height: 1.55,
        indent: 0,
        paragraph_gap: 0.0,
        justify: false,
    };
    Ok(model(
        Paginator::try_new(quote, &layout, metrics)?
            .pages
            .into_iter()
            .flat_map(|page| page.lines)
            .take(2)
            .map(|line| line.text.into())
            .collect(),
    ))
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn reader_mark(
    line: &Line,
    layout: &Layout,
    metrics: &TextMetrics,
    start: u32,
    end: u32,
    id: String,
    mine: bool,
    selected: bool,
    count: u32,
) -> Option<ReaderMark> {
    let geometry = line
        .caret_geometry(layout, metrics, &mut CaretBudget::default())
        .ok()?;
    Some(reader_mark_with_geometry(
        line, layout, &geometry, start, end, id, mine, selected, count,
    ))
}

#[allow(clippy::too_many_arguments)]
fn reader_mark_with_geometry(
    line: &Line,
    layout: &Layout,
    geometry: &CaretGeometry,
    start: u32,
    end: u32,
    id: String,
    mine: bool,
    selected: bool,
    count: u32,
) -> ReaderMark {
    let start = start.max(line.start).min(line.end);
    let end = end.min(line.end).max(start);
    let x = geometry.x_at_offset(start);
    let width = (geometry.x_at_offset(end) - x)
        .max(1.0)
        .min((layout.width - x).max(0.0));
    ReaderMark {
        id: id.into(),
        x,
        width,
        mine,
        selected,
        count: count.min(i32::MAX as u32) as i32,
    }
}

fn intersects(a: u32, b: u32, c: u32, d: u32) -> bool {
    a < d && c < b
}

#[cfg(test)]
fn scalar_slice(text: &str, start: u32, end: u32) -> String {
    text.chars()
        .skip(start as usize)
        .take(end.saturating_sub(start) as usize)
        .collect()
}

fn thought_card_text(text: &str, maximum: usize) -> String {
    if text.len() <= maximum {
        text.to_owned()
    } else {
        tr("api_error")
    }
}

#[cfg(test)]
fn quote_range(body: &str, quote: &str) -> Option<(u32, u32)> {
    super::underline_projection::unique_quote_range(body, quote)
}

fn stamp(timestamp: i64) -> String {
    if timestamp <= 0 {
        return String::new();
    }
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|time| {
            time.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
                .format("%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn pointer(value: &str) -> Option<(f32, f32)> {
    unbounded_pointer(value).filter(|(x, _)| (0.0..=1.0).contains(x))
}

fn unbounded_pointer(value: &str) -> Option<(f32, f32)> {
    let values = value
        .split(',')
        .map(str::parse::<f32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    let x = *values.first()?;
    let y = *values.get(2).or_else(|| values.get(1))?;
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

fn line_at_y(lines: &[Line], y: f32, height: f32) -> Option<&Line> {
    if y < 0.0 {
        return None;
    }
    lines.iter().find(|line| {
        y >= line.y
            && y < line.y
                + if line.height > 0.0 {
                    line.height
                } else {
                    height
                }
                + line.gap_after
    })
}

#[cfg(test)]
fn initial_reader_range(
    text: &str,
    anchor: (u32, u32),
    line_start: u32,
    line_end: u32,
) -> (u32, u32) {
    let mut operation = SelectionOperation::new();
    let span = selection_span(
        text,
        line_start,
        line_end,
        SELECTION_LINE_BYTES,
        &mut operation,
    )
    .expect("The original phrase fixture must have admitted source anchors");
    let line = Line {
        text: span.to_owned(),
        start: line_start,
        end: line_end,
        ..Default::default()
    };
    initial_reader_range_from_line(&line, anchor, &mut operation)
        .expect("The original phrase fixture must fit the local line allowance")
}

fn initial_reader_range_from_line(
    line: &Line,
    anchor: (u32, u32),
    operation: &mut SelectionOperation,
) -> Result<(u32, u32)> {
    let scalars = line
        .end
        .checked_sub(line.start)
        .ok_or(SelectionSourceUnavailable)? as usize;
    if line.text.len() > operation.limits.line_bytes {
        return Err(SelectionWorkLimit {
            resource: "phrase line bytes",
            limit: operation.limits.line_bytes,
            requested: line.text.len(),
        }
        .into());
    }
    if scalars > operation.limits.line_scalars {
        return Err(SelectionWorkLimit {
            resource: "phrase line scalars",
            limit: operation.limits.line_scalars,
            requested: scalars,
        }
        .into());
    }
    if scalars == 0 || anchor.0 < line.start || anchor.0 >= anchor.1 || anchor.1 > line.end {
        return Err(SelectionSourceUnavailable.into());
    }
    let work = line
        .text
        .len()
        .checked_mul(2)
        .and_then(|bytes| {
            scalars
                .checked_mul(6)
                .and_then(|scalars| bytes.checked_add(scalars))
        })
        .ok_or(SelectionWorkLimit {
            resource: "source work bytes",
            limit: operation.limits.work_bytes,
            requested: usize::MAX,
        })?;
    operation.charge(work)?;
    // Paginator line construction retains the original raw scalar span. Price
    // and verify that local proof before collecting this one line, never the
    // resident chapter or a prefix skipped without a work allowance.
    let mut observed = 0usize;
    for _ in line.text.chars() {
        observed = observed.checked_add(1).ok_or(SelectionSourceUnavailable)?;
        #[cfg(test)]
        {
            operation.stats.phrase_scalar_visits += 1;
        }
    }
    if observed != scalars {
        return Err(SelectionSourceUnavailable.into());
    }
    let mut chars = Vec::with_capacity(scalars);
    chars.extend(line.text.chars());
    #[cfg(test)]
    {
        operation.stats.phrase_vectors += 1;
        operation.stats.phrase_copied_scalars += chars.len();
    }
    let local = anchor
        .0
        .checked_sub(line.start)
        .ok_or(SelectionSourceUnavailable)? as usize;
    let character = *chars.get(local).ok_or(SelectionSourceUnavailable)?;
    if matches!(character,
        '\u{2e80}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' | '\u{20000}'..='\u{323af}')
    {
        return Ok(anchor);
    }
    let mut start = local;
    let mut end = start + 1;
    let boundary = |c: char| {
        c.is_whitespace()
            || c.is_ascii_punctuation()
            || matches!(
                c,
                '\u{3001}'
                    | '\u{3002}'
                    | '\u{ff0c}'
                    | '\u{ff1b}'
                    | '\u{ff1a}'
                    | '\u{ff01}'
                    | '\u{ff1f}'
                    | '\u{201c}'
                    | '\u{201d}'
            )
    };
    if !boundary(chars[start]) {
        while start > 0 && !boundary(chars[start - 1]) {
            start -= 1;
        }
        while end < chars.len() && !boundary(chars[end]) {
            end += 1;
        }
    }
    let start = line
        .start
        .checked_add(u32::try_from(start).map_err(|_| SelectionSourceUnavailable)?)
        .ok_or(SelectionSourceUnavailable)?;
    let end = line
        .start
        .checked_add(u32::try_from(end).map_err(|_| SelectionSourceUnavailable)?)
        .ok_or(SelectionSourceUnavailable)?;
    Ok((start.min(anchor.0), end.max(anchor.1)))
}

fn reader_access_hint(status: ContentStatus, has_body: bool) -> Option<&'static str> {
    match status {
        ContentStatus::Full => None,
        ContentStatus::Preview => Some("reader_preview_hint"),
        ContentStatus::Locked => Some("reader_locked_hint"),
        ContentStatus::Unknown if has_body => Some("reader_unverified_hint"),
        ContentStatus::Unknown => None,
    }
}

fn reader_content_label(status: ContentStatus, has_body: bool) -> String {
    let key = match status {
        ContentStatus::Preview => "content_preview",
        ContentStatus::Locked => "content_locked",
        ContentStatus::Unknown if has_body => "content_unverified",
        ContentStatus::Full if has_body => "downloaded",
        _ => "not_downloaded",
    };
    tr(key)
}

fn reader_page_bottom(page: &Page) -> f32 {
    page.lines
        .iter()
        .map(|line| line.y + line.height)
        .chain(page.images.iter().map(|image| image.y + image.height))
        .fold(0.0, f32::max)
}

/// A separate notice page keeps status controls from covering the final text or image.
fn append_reader_access_page(paginator: &mut Paginator, first_page_height: f32) -> Result<()> {
    let Some(last) = paginator.pages.last() else {
        return Ok(());
    };
    let available = if paginator.pages.len() == 1 {
        first_page_height
    } else {
        paginator.layout.height
    };
    if reader_page_bottom(last) + READER_ACCESS_NOTICE_GAP + READER_ACCESS_NOTICE_HEIGHT > available
    {
        paginator.try_push_empty_page(paginator.char_count, paginator.char_count)?;
    }
    Ok(())
}

/// Upstream offsets are normalized with catalog word counts, not byte offsets.
fn remote_to_local_offset(offset: u32, words: u32, chars: u32) -> u32 {
    if words == 0 || chars == 0 {
        return 0;
    }
    (u64::from(offset.min(words)) * u64::from(chars) / u64::from(words)) as u32
}

fn local_to_remote_offset(anchor: u32, chars: u32, words: u32) -> u32 {
    if words == 0 || chars == 0 {
        return 0;
    }
    (u64::from(anchor.min(chars)) * u64::from(words) / u64::from(chars)) as u32
}

fn cloud_chapter_index(
    position: &source::ReadingPosition,
    catalog: &[source::RemoteChapter],
    chapters: &[Chapter],
) -> Option<u32> {
    if !position.chapter_uid.is_empty() {
        return chapters
            .iter()
            .find(|chapter| chapter.id == position.chapter_uid)
            .map(|chapter| chapter.index);
    }
    let uid = &catalog
        .iter()
        .find(|chapter| chapter.index == position.chapter_index)?
        .uid;
    chapters
        .iter()
        .find(|chapter| &chapter.id == uid)
        .map(|chapter| chapter.index)
}

fn weighted_book_fraction(
    chapters: &[Chapter],
    catalog: &[source::RemoteChapter],
    current: u32,
    anchor: u32,
    characters: u32,
) -> Option<f32> {
    let rank = chapters
        .iter()
        .position(|chapter| chapter.index == current)?;
    let maximum = BOOK_WEIGHT_METADATA_BYTES;
    #[cfg(test)]
    let maximum = maximum.min(BOOK_WEIGHT_ALLOWANCE.with(Cell::get));
    // The current source policy bounds rows; retain a separate borrowed-map
    // forecast before allocating buckets or hashing an old persisted UID.
    let admitted = catalog.len() <= source::content::MAX_CATALOG_CHAPTERS
        && catalog
            .iter()
            .try_fold(0usize, |used, chapter| {
                #[cfg(test)]
                BOOK_WEIGHT_WORK.with(|work| {
                    let mut observed = work.get();
                    observed.preflight_records += 1;
                    work.set(observed);
                });
                used.checked_add(BOOK_WEIGHT_ENTRY_BYTES)?
                    .checked_add(chapter.uid.len())
                    .filter(|bytes| *bytes <= maximum)
            })
            .is_some();
    if !admitted {
        #[cfg(test)]
        BOOK_WEIGHT_WORK.with(|work| {
            let mut observed = work.get();
            observed.rejections += 1;
            work.set(observed);
        });
        return None;
    }
    #[cfg(test)]
    BOOK_WEIGHT_WORK.with(|work| {
        let mut observed = work.get();
        observed.builds += 1;
        work.set(observed);
    });
    let mut weights = HashMap::with_capacity(catalog.len());
    for remote in catalog {
        #[cfg(test)]
        BOOK_WEIGHT_WORK.with(|work| {
            let mut observed = work.get();
            observed.uid_inserts += 1;
            work.set(observed);
        });
        // Persisted duplicates preserve the old linear search's first match.
        weights
            .entry(remote.uid.as_str())
            .or_insert(u64::from(remote.word_count));
    }
    let mut total = 0_u64;
    let mut before = 0_u64;
    let mut current_weight = 0_u64;
    for (index, chapter) in chapters.iter().enumerate() {
        #[cfg(test)]
        BOOK_WEIGHT_WORK.with(|work| {
            let mut observed = work.get();
            observed.chapter_lookups += 1;
            work.set(observed);
        });
        let weight = *weights.get(chapter.id.as_str())?;
        total = total.checked_add(weight)?;
        if index < rank {
            before = before.checked_add(weight)?;
        } else if index == rank {
            current_weight = weight;
        }
    }
    if total == 0 {
        return None;
    }
    let fraction = if characters == 0 {
        0.0
    } else {
        f64::from(anchor.min(characters)) / f64::from(characters)
    };
    Some(((before as f64 + current_weight as f64 * fraction) / total as f64).clamp(0.0, 1.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_access_labels_distinguish_trial_unknown_and_verified_full_content() {
        assert_eq!(
            reader_content_label(ContentStatus::Preview, true),
            tr("content_preview")
        );
        assert_eq!(
            reader_content_label(ContentStatus::Locked, false),
            tr("content_locked")
        );
        assert_eq!(
            reader_content_label(ContentStatus::Unknown, true),
            tr("content_unverified")
        );
        assert_eq!(
            reader_content_label(ContentStatus::Unknown, false),
            tr("not_downloaded")
        );
        assert_eq!(
            reader_content_label(ContentStatus::Full, true),
            tr("downloaded")
        );
        assert!(reader_access_hint(ContentStatus::Full, true).is_none());
        assert!(reader_access_hint(ContentStatus::Unknown, false).is_none());
        assert_ne!(
            reader_access_hint(ContentStatus::Unknown, true),
            reader_access_hint(ContentStatus::Preview, true)
        );
    }

    #[test]
    fn incomplete_content_notice_preserves_all_paginated_text_and_never_covers_images() {
        let text =
            "Every source character must remain reachable before the access notice. ".repeat(90);
        let layout = Layout {
            width: 260.0,
            height: 240.0,
            font_size: 20.0,
            ..Layout::default()
        };
        let mut paginator = Paginator::new_with_images(
            &text,
            &layout,
            &TextMetrics::default(),
            layout.height,
            &[crate::reader::InlineImage {
                id: "last-access-image".into(),
                anchor: text.chars().count() as u32,
                width: 200.0,
                height: 180.0,
            }],
        );
        let pages = paginator.pages.clone();
        let characters = paginator.char_count;
        append_reader_access_page(&mut paginator, layout.height).unwrap();
        assert_eq!(&paginator.pages[..pages.len()], pages.as_slice());
        assert_eq!(paginator.char_count, characters);
        let notice = paginator.pages.last().unwrap();
        assert!(
            reader_page_bottom(notice) + READER_ACCESS_NOTICE_GAP + READER_ACCESS_NOTICE_HEIGHT
                <= layout.height
        );
        assert_eq!(notice.end, characters);
        let count = paginator.pages.len();
        append_reader_access_page(&mut paginator, layout.height).unwrap();
        assert_eq!(
            paginator.pages.len(),
            count,
            "Reflow must not accumulate notice pages"
        );
    }

    #[test]
    fn source_position_conversion_is_bounded_and_uses_word_count_ratio() {
        assert_eq!(remote_to_local_offset(250, 1000, 8), 2);
        assert_eq!(local_to_remote_offset(2, 8, 1000), 250);
        assert_eq!(
            remote_to_local_offset(u32::MAX, u32::MAX, u32::MAX),
            u32::MAX
        );
        assert_eq!(
            local_to_remote_offset(u32::MAX, u32::MAX, u32::MAX),
            u32::MAX
        );
        assert_eq!(remote_to_local_offset(99, 3, 10), 10);
        assert_eq!(local_to_remote_offset(99, 10, 3), 3);
        assert_eq!(remote_to_local_offset(3, 0, 10), 0);
        assert_eq!(local_to_remote_offset(3, 0, 10), 0);
        assert_eq!(remote_to_local_offset(1, 3, 10), 3);
    }

    #[test]
    fn selection_offsets_are_unicode_scalars_and_pointers_reject_nonfinite_input() {
        let text = "A\u{4e2d}\u{1f600}e\u{301}\r\n\u{5c3e}";
        assert_eq!(text.chars().count(), 8);
        assert_eq!(scalar_slice(text, 2, 3), "\u{1f600}");
        assert_eq!(scalar_slice(text, 3, 5), "e\u{301}");
        assert_eq!(quote_range(text, "\u{1f600}"), Some((2, 3)));
        assert_eq!(quote_range("repeat repeat", "repeat"), None);
        assert_eq!(pointer("NaN,1,1"), None);
        assert_eq!(pointer("0.5,1,inf"), None);
        assert_eq!(pointer("0.5,1,40"), Some((0.5, 40.0)));
        assert_eq!(pointer("1.5,1,-10"), None);
        assert_eq!(unbounded_pointer("1.5,1,-10"), Some((1.5, -10.0)));
        assert_eq!(unbounded_pointer("NaN,1,-10"), None);
    }

    #[test]
    fn stationary_cjk_selection_starts_with_a_cluster_and_latin_keeps_word_semantics() {
        let text = "\u{4e2d}\u{fe0f}\u{6587}\u{4e66}\u{9875} measured words";
        let end = text.chars().count() as u32;
        assert_eq!(initial_reader_range(text, (0, 2), 0, end), (0, 2));
        assert_eq!(initial_reader_range(text, (2, 3), 0, end), (2, 3));
        let word = initial_reader_range(text, (7, 8), 0, end);
        assert_eq!(scalar_slice(text, word.0, word.1), "measured");
    }

    #[test]
    fn persisted_marks_cover_the_same_source_range_after_reflow_and_across_pages() {
        let text = "Exact \u{4e2d}\u{6587} anchors stay with the selected words.\n".repeat(18);
        let metrics = TextMetrics::default();
        let layout = Layout {
            width: 210.0,
            height: 100.0,
            font_size: 16.0,
            line_height: 1.8,
            indent: 2,
            paragraph_gap: 0.4,
            justify: true,
        };
        let initial = Paginator::new(&text, &layout, &metrics);
        let start = initial.pages[1].lines.last().unwrap().start + 1;
        let end = initial.pages[7].lines[0].end - 1;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reader.sqlite");
        {
            let store = Store::open(&path).unwrap();
            store
                .upsert_book(&Book {
                    id: "reader-fixture".into(),
                    title: "Reader fixture".into(),
                    ..Book::default()
                })
                .unwrap();
            store
                .upsert_chapter(&Chapter {
                    id: "reader-fixture-chapter".into(),
                    book_id: "reader-fixture".into(),
                    index: 0,
                    title: "Reader fixture chapter".into(),
                    cached: true,
                    body: Some(text.clone()),
                    content_status: ContentStatus::Full,
                    ..Chapter::default()
                })
                .unwrap();
            store
                .save_annotation(&Annotation {
                    id: "range-fixture".into(),
                    book_id: "reader-fixture".into(),
                    chapter_index: 0,
                    start,
                    end,
                    quote: scalar_slice(&text, start, end),
                    note: String::new(),
                    created_at: 1,
                })
                .unwrap();
        }
        let store = Store::open(path).unwrap();
        let note = store.annotations("reader-fixture").unwrap().remove(0);
        assert_eq!((note.start, note.end), (start, end));
        for (width, font_size) in [(210.0, 16.0), (150.0, 24.0), (290.0, 18.0)] {
            let changed = Layout {
                width,
                font_size,
                ..layout
            };
            let paginator = Paginator::new(&text, &changed, &metrics);
            let mut covered = String::new();
            let mut marked_pages = 0;
            for page in &paginator.pages {
                let mut marked = false;
                for line in page
                    .lines
                    .iter()
                    .filter(|line| intersects(line.start, line.end, note.start, note.end))
                {
                    let mark = reader_mark(
                        line,
                        &changed,
                        &metrics,
                        note.start,
                        note.end,
                        note.id.clone(),
                        true,
                        false,
                        0,
                    )
                    .expect("Admit the normal local mark geometry");
                    assert!(mark.width > 0.0 && mark.x >= 0.0);
                    assert!(mark.x + mark.width <= changed.width + 0.01);
                    assert_eq!(
                        line.offset_at_x(mark.x + 0.001, &changed, &metrics),
                        Some(note.start.max(line.start))
                    );
                    assert_eq!(
                        line.offset_at_x(mark.x + mark.width - 0.001, &changed, &metrics),
                        Some(note.end.min(line.end))
                    );
                    covered.push_str(&scalar_slice(
                        &text,
                        note.start.max(line.start),
                        note.end.min(line.end),
                    ));
                    marked = true;
                }
                marked_pages += usize::from(marked);
            }
            assert!(marked_pages > 1);
            assert_eq!(
                covered,
                note.quote
                    .chars()
                    .filter(|character| !matches!(character, '\r' | '\n'))
                    .collect::<String>()
            );
        }
    }

    #[test]
    fn remote_indices_are_resolved_through_uid_in_filtered_catalogs() {
        let catalog = vec![source::RemoteChapter {
            uid: "chapter-a".into(),
            index: 9,
            ..Default::default()
        }];
        let chapters = vec![Chapter {
            id: "chapter-a".into(),
            index: 0,
            ..Default::default()
        }];
        assert_eq!(
            cloud_chapter_index(
                &source::ReadingPosition {
                    chapter_index: 9,
                    ..Default::default()
                },
                &catalog,
                &chapters
            ),
            Some(0)
        );
        assert_eq!(
            cloud_chapter_index(
                &source::ReadingPosition {
                    chapter_uid: "unknown".into(),
                    chapter_index: 9,
                    ..Default::default()
                },
                &catalog,
                &chapters
            ),
            None
        );
    }

    #[test]
    fn book_percent_uses_word_weights_instead_of_equal_chapters() {
        let chapters = vec![
            Chapter {
                id: "short".into(),
                index: 0,
                ..Default::default()
            },
            Chapter {
                id: "long".into(),
                index: 1,
                ..Default::default()
            },
        ];
        let catalog = vec![
            source::RemoteChapter {
                uid: "short".into(),
                word_count: 100,
                ..Default::default()
            },
            source::RemoteChapter {
                uid: "long".into(),
                word_count: 900,
                ..Default::default()
            },
        ];
        assert_eq!(
            weighted_book_fraction(&chapters, &catalog, 1, 0, 100),
            Some(0.1)
        );
        assert_eq!(
            weighted_book_fraction(&chapters, &catalog, 1, 50, 100),
            Some(0.55)
        );
        assert_eq!(
            weighted_book_fraction(&chapters, &catalog, 1, 100, 100),
            Some(1.0)
        );
        assert_eq!(weighted_book_fraction(&chapters, &[], 1, 0, 100), None);
    }
}

#[cfg(test)]
#[path = "reading_render_tests.rs"]
mod render_tests;
