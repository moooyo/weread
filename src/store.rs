use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, Row, named_params, params};
use serde::{Serialize, de::DeserializeOwned};

use crate::domain::{
    Annotation, Book, BookMetadata, BookStatisticsMetadata, Bookmark, Chapter, ContentStatus,
    DownloadChapterCommit, DownloadChapterProgress, DownloadChapterRetry, DownloadChapterState,
    DownloadChapterStatus, DownloadStatus, DownloadTask, ReadingPosition, ReadingSession,
    StorageSummary,
};

const MAX_CATALOG_HEADERS: usize = 8192;
const MAX_CATALOG_HEADER_BYTES: i64 = 64 * 1024 * 1024;
const CATALOG_INDEX_OFFSET: i64 = u32::MAX as i64 + 1;
const MAX_ENCODED_SETTING_CHANGES: usize = 64 * 1024;
const MAX_ENCODED_SETTING_BYTES: usize = 64 * 1024 * 1024;
// Rust's White_Space set, expressed as a scalar SQL predicate. GLOB inspects
// borrowed SQLite text instead of materializing a trimmed chapter-sized copy.
const NONBLANK_BODY_PATTERN: &str = "*[^\u{9}\u{a}\u{b}\u{c}\u{d}\u{20}\u{85}\u{a0}\u{1680}\u{2000}\u{2001}\u{2002}\u{2003}\u{2004}\u{2005}\u{2006}\u{2007}\u{2008}\u{2009}\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}]*";

#[path = "store_chapter_state.rs"]
mod chapter_state;

#[cfg(test)]
#[path = "store_chapter_state_tests.rs"]
mod chapter_state_tests;

#[cfg(test)]
#[path = "store_resource_tests.rs"]
mod resource_tests;

#[cfg(test)]
#[path = "store_checkpoint_resource_tests.rs"]
mod checkpoint_resource_tests;

#[cfg(test)]
#[path = "store_display_metadata_tests.rs"]
mod display_metadata_tests;

#[cfg(test)]
#[path = "store_annotation_budget_tests.rs"]
mod annotation_budget_tests;

#[cfg(test)]
#[path = "store_byte_admission_tests.rs"]
mod byte_admission_tests;

#[cfg(test)]
#[path = "store_book_metadata_tests.rs"]
mod book_metadata_tests;

#[cfg(test)]
#[path = "store_shelf_admission_tests.rs"]
mod shelf_admission_tests;

#[cfg(test)]
#[path = "store_selected_metadata_tests.rs"]
mod selected_metadata_tests;

#[cfg(test)]
#[path = "store_encoded_settings_tests.rs"]
mod encoded_settings_tests;

#[cfg(test)]
#[path = "store_book_introduction_tests.rs"]
mod book_introduction_tests;

#[cfg(test)]
#[path = "store_body_length_budget_tests.rs"]
mod body_length_budget_tests;

#[cfg(test)]
#[path = "store_annotation_body_budget_tests.rs"]
mod annotation_body_budget_tests;
#[cfg(test)]
#[path = "store_bookmark_budget_tests.rs"]
mod bookmark_budget_tests;
#[cfg(test)]
#[path = "store_inventory_budget_tests.rs"]
mod inventory_budget_tests;
#[cfg(test)]
#[path = "store_search_matcher_budget_tests.rs"]
mod search_matcher_budget_tests;

#[cfg(test)]
#[path = "store_numeric_admission_tests.rs"]
mod numeric_admission_tests;
#[cfg(test)]
#[path = "store_setting_text_budget_tests.rs"]
mod setting_text_budget_tests;
#[cfg(test)]
#[path = "store_task_transition_budget_tests.rs"]
mod task_transition_budget_tests;

/// Storage and parser admission for a borrowed persisted-setting callback.
/// The callback prices selected fields/counts before owning its result.
#[derive(Clone, Copy, Debug)]
pub struct StoredSettingReadLimits {
    pub encoded_bytes: usize,
    pub utf8_bytes: usize,
    pub work_bytes: usize,
    pub scratch_bytes: usize,
    pub json_passes: usize,
}
impl Default for StoredSettingReadLimits {
    fn default() -> Self {
        Self {
            encoded_bytes: 32 * 1024 * 1024,
            utf8_bytes: 32 * 1024 * 1024,
            work_bytes: 128 * 1024 * 1024,
            scratch_bytes: 64 * 1024 * 1024,
            json_passes: 2,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct StoredSettingTextMetadata {
    pub encoded_bytes: usize,
    pub utf8_bytes_upper: usize,
    pub vm_scratch_bytes: usize,
}
#[derive(Debug, thiserror::Error)]
#[error("persisted setting {resource} exceeds its read allowance")]
pub struct StoredSettingReadLimit {
    pub resource: &'static str,
}
#[derive(Debug, thiserror::Error)]
#[error("persisted setting type or snapshot identity is unavailable")]
pub struct StoredSettingUnavailable;

#[derive(Clone, Copy, Debug)]
/// Project admission units for logical inventory work and live field/metadata
/// forecasts. These are not a process RSS limit or a SQLite execution deadline.
pub struct StoreInventoryLimits {
    pub rows: usize,
    pub encoded_bytes: usize,
    pub metadata_bytes: usize,
    pub work_bytes: usize,
    pub scratch_bytes: usize,
}
impl Default for StoreInventoryLimits {
    fn default() -> Self {
        Self {
            rows: 65536,
            encoded_bytes: 64 * 1024 * 1024,
            metadata_bytes: 64 * 1024 * 1024,
            work_bytes: 128 * 1024 * 1024,
            scratch_bytes: 64 * 1024 * 1024,
        }
    }
}
#[derive(Debug, thiserror::Error)]
#[error("store inventory {resource} exceeds its allowance")]
pub struct StoreInventoryLimit {
    pub resource: &'static str,
}
#[derive(Debug, thiserror::Error)]
#[error("store inventory type or identity is unavailable")]
pub struct StoreInventoryUnavailable;
#[derive(Clone, Copy, Debug)]
pub struct SettingInventoryHeader {
    pub rowid: i64,
    pub key_encoded_bytes: usize,
    pub value_encoded_bytes: usize,
    pub key_utf8_bytes_upper: usize,
    pub value_utf8_bytes_upper: usize,
    pub key_is_text: bool,
    pub value_is_text: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct ChapterBodyTextMetadata {
    pub body_present: bool,
    pub encoded_bytes: usize,
    pub utf8_bytes_upper: usize,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct InventoryReadStats {
    pub key_copies: usize,
    pub key_bytes: usize,
    pub manifest_borrows: usize,
    pub manifest_bytes: usize,
    pub manifest_deserializations: usize,
    pub body_borrows: usize,
    pub body_bytes: usize,
    pub task_deserializations: usize,
}
#[cfg(test)]
pub(crate) fn inventory_test_read_stats() -> InventoryReadStats {
    inventory_budget_tests::read_stats()
}
#[cfg(test)]
pub(crate) fn reset_inventory_test_read_stats() {
    inventory_budget_tests::reset()
}
#[cfg(test)]
pub(crate) fn annotation_body_test_borrow_count() -> usize {
    annotation_body_budget_tests::borrowed_count()
}

#[cfg(test)]
thread_local! { static INVENTORY_TEST_LIMITS: std::cell::Cell<Option<StoreInventoryLimits>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
pub(crate) fn with_store_inventory_test_limits<T>(
    limits: StoreInventoryLimits,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<StoreInventoryLimits>);
    impl Drop for Restore {
        fn drop(&mut self) {
            INVENTORY_TEST_LIMITS.with(|slot| slot.set(self.0));
        }
    }
    let _restore = Restore(INVENTORY_TEST_LIMITS.with(|slot| slot.replace(Some(limits))));
    run()
}
fn store_inventory_limits() -> StoreInventoryLimits {
    let maximum = StoreInventoryLimits::default();
    #[cfg(test)]
    if let Some(l) = INVENTORY_TEST_LIMITS.with(std::cell::Cell::get) {
        return StoreInventoryLimits {
            rows: l.rows.min(maximum.rows),
            encoded_bytes: l.encoded_bytes.min(maximum.encoded_bytes),
            metadata_bytes: l.metadata_bytes.min(maximum.metadata_bytes),
            work_bytes: l.work_bytes.min(maximum.work_bytes),
            scratch_bytes: l.scratch_bytes.min(maximum.scratch_bytes),
        };
    }
    maximum
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct EncodedSettingsLimits {
    pub bytes: usize,
    pub key_bytes: usize,
}

impl Default for EncodedSettingsLimits {
    fn default() -> Self {
        Self {
            bytes: MAX_ENCODED_SETTING_BYTES,
            key_bytes: 4096,
        }
    }
}

#[cfg(test)]
thread_local! {
    static ENCODED_SETTINGS_TEST_LIMITS: std::cell::Cell<Option<EncodedSettingsLimits>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_encoded_settings_test_limits<T>(
    limits: EncodedSettingsLimits,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<EncodedSettingsLimits>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ENCODED_SETTINGS_TEST_LIMITS.with(|slot| slot.set(self.0));
        }
    }
    let _restore = Restore(ENCODED_SETTINGS_TEST_LIMITS.with(|slot| slot.replace(Some(limits))));
    run()
}

fn encoded_settings_limits() -> EncodedSettingsLimits {
    let maximum = EncodedSettingsLimits::default();
    #[cfg(test)]
    if let Some(limits) = ENCODED_SETTINGS_TEST_LIMITS.with(std::cell::Cell::get) {
        return EncodedSettingsLimits {
            bytes: limits.bytes.min(maximum.bytes),
            key_bytes: limits.key_bytes.min(maximum.key_bytes),
        };
    }
    maximum
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EncodedSettingReadStats {
    pub key_materializations: usize,
    pub key_bytes: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("cached resource exceeds its read allowance")]
pub struct CachedResourceLimit;

#[derive(Clone, Copy, Debug)]
pub struct BookMetadataLimits {
    pub rows: usize,
    pub sql_cells: usize,
    pub bytes: usize,
    pub identifier_bytes: usize,
}

impl Default for BookMetadataLimits {
    fn default() -> Self {
        Self {
            rows: 8192,
            sql_cells: 8192 * 16,
            bytes: 64 * 1024 * 1024,
            identifier_bytes: 4096,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("book metadata {resource} exceeds its allowance (limit {limit}, requested {requested})")]
pub struct BookMetadataLimit {
    pub resource: &'static str,
    pub limit: usize,
    pub requested: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("book metadata fields are unavailable")]
pub struct BookMetadataUnavailable;

/// Numeric state retained when a fresh full import replaces an existing book.
/// This projection cannot be passed to a full-book persistence operation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BookReadingState {
    pub progress: f32,
    pub finished: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReportBatchDeliveryState {
    Pending,
    Sending,
    Uncertain,
    Confirmed,
    Blocked,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ReportBatchDeliveryMetadata {
    pub sid: String,
    pub state: ReportBatchDeliveryState,
    pub attempted_at: Option<i64>,
    pub attempts: u32,
}

#[derive(Debug, thiserror::Error)]
#[error("report batch metadata {resource} exceeds its independent input allowance")]
pub(crate) struct ReportBatchLookupLimit {
    pub resource: &'static str,
}

#[derive(Debug, thiserror::Error)]
#[error("report batch delivery metadata is unavailable or ambiguous")]
pub(crate) struct ReportBatchLookupUnavailable;

/// The introduction tab's admitted payload. This is not a writable Book.
#[derive(Debug, Default, PartialEq)]
pub struct BookIntroduction {
    pub intro: String,
    pub publisher: Option<String>,
    pub isbn: Option<String>,
}

#[derive(Debug)]
pub struct BookDetailMetadata {
    pub book: BookMetadata,
    pub introduction: Option<BookIntroduction>,
}

#[derive(Clone, Copy, Debug)]
pub struct BookIntroductionLimits {
    pub bytes: usize,
    pub metadata_bytes: usize,
    pub metadata_nodes: usize,
    pub field_bytes: usize,
}

impl Default for BookIntroductionLimits {
    fn default() -> Self {
        Self {
            bytes: 64 * 1024 * 1024,
            metadata_bytes: 4 * 1024 * 1024,
            metadata_nodes: 8192,
            field_bytes: 64 * 1024,
        }
    }
}

#[cfg(test)]
thread_local! {
    static BOOK_INTRODUCTION_TEST_LIMITS: std::cell::Cell<Option<BookIntroductionLimits>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_book_introduction_test_limits<T>(
    limits: BookIntroductionLimits,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<BookIntroductionLimits>);
    impl Drop for Restore {
        fn drop(&mut self) {
            BOOK_INTRODUCTION_TEST_LIMITS.with(|slot| slot.set(self.0));
        }
    }
    let _restore = Restore(BOOK_INTRODUCTION_TEST_LIMITS.with(|slot| slot.replace(Some(limits))));
    run()
}

fn book_introduction_limits() -> BookIntroductionLimits {
    let maximum = BookIntroductionLimits::default();
    #[cfg(test)]
    if let Some(limits) = BOOK_INTRODUCTION_TEST_LIMITS.with(std::cell::Cell::get) {
        return BookIntroductionLimits {
            bytes: limits.bytes.min(maximum.bytes),
            metadata_bytes: limits.metadata_bytes.min(maximum.metadata_bytes),
            metadata_nodes: limits.metadata_nodes.min(maximum.metadata_nodes),
            field_bytes: limits.field_bytes.min(maximum.field_bytes),
        };
    }
    maximum
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BodyLengthLimits {
    pub rows: usize,
    pub work_bytes: usize,
    pub metadata_bytes: usize,
    pub temporary_bytes: usize,
}

impl Default for BodyLengthLimits {
    fn default() -> Self {
        Self {
            rows: 64 * 1024,
            work_bytes: 64 * 1024 * 1024,
            metadata_bytes: 64 * 1024 * 1024,
            temporary_bytes: 64 * 1024 * 1024,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BodyLengthReadStats {
    pub metadata_rows: usize,
    pub unique_rows: usize,
    pub text_evaluations: usize,
    pub encoded_text_bytes: usize,
}

#[cfg(test)]
thread_local! {
    static BODY_LENGTH_TEST_LIMITS: std::cell::Cell<Option<BodyLengthLimits>> = const { std::cell::Cell::new(None) };
    static BODY_LENGTH_READ_STATS: std::cell::Cell<BodyLengthReadStats> = const { std::cell::Cell::new(BodyLengthReadStats {
        metadata_rows: 0, unique_rows: 0, text_evaluations: 0, encoded_text_bytes: 0 }) };
}

#[cfg(test)]
pub(crate) fn with_body_length_test_limits<T>(
    limits: BodyLengthLimits,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<BodyLengthLimits>);
    impl Drop for Restore {
        fn drop(&mut self) {
            BODY_LENGTH_TEST_LIMITS.with(|slot| slot.set(self.0));
        }
    }
    let _restore = Restore(BODY_LENGTH_TEST_LIMITS.with(|slot| slot.replace(Some(limits))));
    run()
}

#[cfg(test)]
pub(crate) fn reset_body_length_read_stats() {
    BODY_LENGTH_READ_STATS.with(|stats| stats.set(BodyLengthReadStats::default()));
}

#[cfg(test)]
pub(crate) fn body_length_read_stats() -> BodyLengthReadStats {
    BODY_LENGTH_READ_STATS.with(std::cell::Cell::get)
}

fn body_length_limits() -> BodyLengthLimits {
    let maximum = BodyLengthLimits::default();
    #[cfg(test)]
    if let Some(limits) = BODY_LENGTH_TEST_LIMITS.with(std::cell::Cell::get) {
        return BodyLengthLimits {
            rows: limits.rows.min(maximum.rows),
            work_bytes: limits.work_bytes.min(maximum.work_bytes),
            metadata_bytes: limits.metadata_bytes.min(maximum.metadata_bytes),
            temporary_bytes: limits.temporary_bytes.min(maximum.temporary_bytes),
        };
    }
    maximum
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BookIntroductionReadStats {
    pub metadata_reads: usize,
    // Nodes from completed admission/projection passes, not a total CPU meter.
    pub metadata_nodes: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct ChapterAccessLimits {
    pub metadata_bytes: usize,
    pub work_bytes: usize,
    pub temporary_bytes: usize,
}

impl Default for ChapterAccessLimits {
    fn default() -> Self {
        Self {
            metadata_bytes: MAX_CATALOG_HEADER_BYTES as usize,
            work_bytes: MAX_CATALOG_HEADER_BYTES as usize,
            temporary_bytes: MAX_CATALOG_HEADER_BYTES as usize,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SelectedMetadataReadStats {
    pub chapter_rows: usize,
    pub chapter_string_bytes: usize,
    pub body_borrows: usize,
    // Input bytes per borrowed TEXT value; validation/trim work is priced separately.
    pub body_scan_bytes: usize,
    pub book_text_rows: usize,
    pub book_text_bytes: usize,
    pub book_scalar_reads: usize,
}

#[cfg(test)]
thread_local! {
    static CHAPTER_ACCESS_TEST_LIMITS: std::cell::Cell<Option<ChapterAccessLimits>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_chapter_access_test_limits<T>(
    limits: ChapterAccessLimits,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<ChapterAccessLimits>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CHAPTER_ACCESS_TEST_LIMITS.with(|slot| slot.set(self.0));
        }
    }
    let _restore = Restore(CHAPTER_ACCESS_TEST_LIMITS.with(|slot| slot.replace(Some(limits))));
    run()
}

fn chapter_access_limits() -> ChapterAccessLimits {
    let maximum = ChapterAccessLimits::default();
    #[cfg(test)]
    if let Some(limits) = CHAPTER_ACCESS_TEST_LIMITS.with(std::cell::Cell::get) {
        return ChapterAccessLimits {
            metadata_bytes: limits.metadata_bytes.min(maximum.metadata_bytes),
            work_bytes: limits.work_bytes.min(maximum.work_bytes),
            temporary_bytes: limits.temporary_bytes.min(maximum.temporary_bytes),
        };
    }
    maximum
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BookReadStats {
    pub metadata_rows: usize,
    pub metadata_string_bytes: usize,
    pub intro_copies: usize,
    pub intro_bytes: usize,
}

#[cfg(test)]
thread_local! {
    static BOOK_METADATA_TEST_LIMITS: std::cell::Cell<Option<BookMetadataLimits>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_book_metadata_test_limits<T>(
    limits: BookMetadataLimits,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<BookMetadataLimits>);
    impl Drop for Restore {
        fn drop(&mut self) {
            BOOK_METADATA_TEST_LIMITS.with(|slot| slot.set(self.0));
        }
    }
    let _restore = Restore(BOOK_METADATA_TEST_LIMITS.with(|slot| slot.replace(Some(limits))));
    run()
}

fn book_metadata_limits() -> BookMetadataLimits {
    let maximum = BookMetadataLimits::default();
    #[cfg(test)]
    if let Some(limits) = BOOK_METADATA_TEST_LIMITS.with(std::cell::Cell::get) {
        return BookMetadataLimits {
            rows: limits.rows.min(maximum.rows),
            sql_cells: limits.sql_cells.min(maximum.sql_cells),
            bytes: limits.bytes.min(maximum.bytes),
            identifier_bytes: limits.identifier_bytes.min(maximum.identifier_bytes),
        };
    }
    maximum
}

#[cfg(test)]
thread_local! {
    static BOOK_METER: std::cell::RefCell<Option<std::rc::Rc<std::cell::Cell<BookReadStats>>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
struct BookMeterGuard {
    previous: Option<std::rc::Rc<std::cell::Cell<BookReadStats>>>,
}

#[cfg(test)]
impl Drop for BookMeterGuard {
    fn drop(&mut self) {
        BOOK_METER.with(|meter| *meter.borrow_mut() = self.previous.take());
    }
}

#[cfg(test)]
fn book_intro_copied(bytes: usize) {
    BOOK_METER.with(|meter| {
        if let Some(meter) = meter.borrow().as_ref() {
            let mut stats = meter.get();
            stats.intro_copies = stats
                .intro_copies
                .checked_add(1)
                .expect("book intro count overflow");
            stats.intro_bytes = stats
                .intro_bytes
                .checked_add(bytes)
                .expect("book intro byte count overflow");
            meter.set(stats);
        }
    });
}

#[cfg(test)]
fn book_metadata_copied(bytes: usize) {
    BOOK_METER.with(|meter| {
        if let Some(meter) = meter.borrow().as_ref() {
            let mut stats = meter.get();
            stats.metadata_rows = stats
                .metadata_rows
                .checked_add(1)
                .expect("book metadata row count overflow");
            stats.metadata_string_bytes = stats
                .metadata_string_bytes
                .checked_add(bytes)
                .expect("book metadata byte count overflow");
            meter.set(stats);
        }
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextEncoding {
    Utf8,
    Utf16,
}

impl TextEncoding {
    fn read(connection: &Connection) -> Result<Self> {
        let encoding: String = connection.pragma_query_value(None, "encoding", |row| row.get(0))?;
        #[cfg(test)]
        byte_admission_tests::observe_encoding_read();
        match encoding.as_str() {
            "UTF-8" => Ok(Self::Utf8),
            "UTF-16le" | "UTF-16be" => Ok(Self::Utf16),
            _ => Err(CachedResourceLimit.into()),
        }
    }

    fn utf8_upper(self, encoded: usize) -> Result<usize> {
        match self {
            Self::Utf8 => Ok(encoded),
            Self::Utf16 => encoded
                .checked_add(1)
                .map(|bytes| bytes / 2)
                .and_then(|units| units.checked_mul(3))
                .ok_or_else(|| CachedResourceLimit.into()),
        }
    }

    fn utf8_upper_total(self, encoded: usize, cells: usize) -> Result<usize> {
        match self {
            Self::Utf8 => Ok(encoded),
            // Rounding each cell separately can cost one code unit per cell.
            // The aggregate bound also covers malformed odd encoded lengths.
            Self::Utf16 => encoded
                .checked_add(cells)
                .map(|bytes| bytes / 2)
                .and_then(|units| units.checked_mul(3))
                .ok_or_else(|| CachedResourceLimit.into()),
        }
    }

    fn maximum_encoded(self, maximum_utf8: usize) -> Result<i64> {
        let encoded = match self {
            Self::Utf8 => maximum_utf8,
            Self::Utf16 => (maximum_utf8 / 3)
                .checked_mul(2)
                .ok_or(CachedResourceLimit)?,
        };
        i64::try_from(encoded).map_err(|_| CachedResourceLimit.into())
    }
}

/// The rowid accelerates bounded selection; the admitted ID fences reuse across
/// separate UI operations. It is never constructed from an unbounded SQL ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnotationKey {
    pub rowid: i64,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnotationHeader {
    pub key: AnnotationKey,
    pub chapter_index: u32,
    pub start: u32,
    pub end: u32,
    pub created_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnnotationGeometry {
    pub rowid: i64,
    pub chapter_index: u32,
    pub start: u32,
    pub end: u32,
    pub created_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnotationFieldState {
    Available { utf8_bytes_upper: usize },
    TooLarge { utf8_bytes_upper: usize },
    Unavailable,
}

#[derive(Debug)]
pub struct AnnotationTextMetadata {
    pub header: AnnotationHeader,
    pub quote: AnnotationFieldState,
    pub note: AnnotationFieldState,
}

#[derive(Clone, Copy, Debug)]
pub struct AnnotationLimits {
    pub rows: usize,
    pub cells: usize,
    pub bytes: usize,
    pub id_bytes: usize,
    pub quote_bytes: usize,
    pub note_bytes: usize,
}

impl AnnotationLimits {
    pub fn headers() -> Self {
        Self {
            rows: 8192,
            cells: 65536,
            bytes: 4 * 1024 * 1024,
            id_bytes: 4096,
            quote_bytes: 16 * 1024,
            note_bytes: 64 * 1024,
        }
    }

    pub fn records() -> Self {
        // Include scalar admission evidence and identity/payload projections.
        Self {
            rows: 16,
            cells: 256,
            ..Self::headers()
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("annotation {resource} exceeds its allowance (limit {limit}, requested {requested})")]
pub struct AnnotationResourceLimit {
    pub resource: &'static str,
    pub limit: usize,
    pub requested: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("annotation metadata or identity is unavailable")]
pub struct AnnotationUnavailable;

#[derive(Debug)]
pub struct AnnotationPage {
    pub total: u64,
    pub headers: Vec<AnnotationHeader>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BookmarkGeometry {
    pub rowid: i64,
    pub chapter_index: u32,
    pub char_offset: u32,
    pub created_at: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookmarkKey {
    pub rowid: i64,
    pub id: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookmarkHeader {
    pub key: BookmarkKey,
    pub chapter_index: u32,
    pub char_offset: u32,
    pub created_at: i64,
}
#[derive(Debug)]
pub struct BookmarkGeometryPage {
    pub total: u64,
    pub geometries: Vec<BookmarkGeometry>,
}
#[derive(Clone, Copy, Debug)]
pub struct BookmarkLimits {
    pub rows: usize,
    pub metadata_bytes: usize,
    pub id_bytes: usize,
    pub label_bytes: usize,
}
impl Default for BookmarkLimits {
    fn default() -> Self {
        Self {
            rows: 8192,
            metadata_bytes: 4 * 1024 * 1024,
            id_bytes: 4096,
            label_bytes: 64 * 1024,
        }
    }
}
#[derive(Debug, thiserror::Error)]
#[error("bookmark {resource} exceeds its allowance")]
pub struct BookmarkResourceLimit {
    pub resource: &'static str,
}
#[derive(Debug, thiserror::Error)]
#[error("bookmark identity or metadata is unavailable")]
pub struct BookmarkUnavailable;
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BookmarkReadStats {
    pub id_copies: usize,
    pub id_bytes: usize,
    pub label_copies: usize,
    pub label_bytes: usize,
}
#[cfg(test)]
pub(crate) fn bookmark_test_read_stats() -> BookmarkReadStats {
    bookmark_budget_tests::read_stats()
}
#[cfg(test)]
pub(crate) fn reset_bookmark_test_read_stats() {
    bookmark_budget_tests::reset()
}
#[cfg(test)]
thread_local! {static BOOKMARK_TEST_LIMITS:std::cell::Cell<Option<BookmarkLimits>>=const{std::cell::Cell::new(None)};}
#[cfg(test)]
pub(crate) fn with_bookmark_test_limits<T>(l: BookmarkLimits, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<BookmarkLimits>);
    impl Drop for Restore {
        fn drop(&mut self) {
            BOOKMARK_TEST_LIMITS.with(|s| s.set(self.0));
        }
    }
    let _r = Restore(BOOKMARK_TEST_LIMITS.with(|s| s.replace(Some(l))));
    run()
}
fn bookmark_limits() -> BookmarkLimits {
    let m = BookmarkLimits::default();
    #[cfg(test)]
    if let Some(l) = BOOKMARK_TEST_LIMITS.with(std::cell::Cell::get) {
        return BookmarkLimits {
            rows: l.rows.min(m.rows),
            metadata_bytes: l.metadata_bytes.min(m.metadata_bytes),
            id_bytes: l.id_bytes.min(m.id_bytes),
            label_bytes: l.label_bytes.min(m.label_bytes),
        };
    }
    m
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AnnotationReadStats {
    pub id_copies: usize,
    pub id_bytes: usize,
    pub quote_copies: usize,
    pub quote_bytes: usize,
    pub note_copies: usize,
    pub note_bytes: usize,
}

#[cfg(test)]
thread_local! {
    static ANNOTATION_METER: std::cell::RefCell<Option<std::rc::Rc<std::cell::Cell<AnnotationReadStats>>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
struct AnnotationMeterGuard {
    previous: Option<std::rc::Rc<std::cell::Cell<AnnotationReadStats>>>,
}

#[cfg(test)]
impl Drop for AnnotationMeterGuard {
    fn drop(&mut self) {
        ANNOTATION_METER.with(|meter| *meter.borrow_mut() = self.previous.take());
    }
}

#[cfg(test)]
fn annotation_copied(id: Option<usize>, quote: Option<usize>, note: Option<usize>) {
    ANNOTATION_METER.with(|meter| {
        if let Some(meter) = meter.borrow().as_ref() {
            let mut stats = meter.get();
            if let Some(bytes) = id {
                stats.id_copies = stats
                    .id_copies
                    .checked_add(1)
                    .expect("annotation ID count overflow");
                stats.id_bytes = stats
                    .id_bytes
                    .checked_add(bytes)
                    .expect("annotation ID bytes overflow");
            }
            if let Some(bytes) = quote {
                stats.quote_copies = stats
                    .quote_copies
                    .checked_add(1)
                    .expect("annotation quote count overflow");
                stats.quote_bytes = stats
                    .quote_bytes
                    .checked_add(bytes)
                    .expect("annotation quote bytes overflow");
            }
            if let Some(bytes) = note {
                stats.note_copies = stats
                    .note_copies
                    .checked_add(1)
                    .expect("annotation note count overflow");
                stats.note_bytes = stats
                    .note_bytes
                    .checked_add(bytes)
                    .expect("annotation note bytes overflow");
            }
            meter.set(stats);
        }
    });
}

/// Bulk verification uses one read snapshot for all scalar admission, JSON
/// metadata, and selected canonical bodies. It owns no downloaded payload.
pub struct StoreResourceSnapshot<'a> {
    transaction: rusqlite::Transaction<'a>,
    text_encoding: TextEncoding,
    revision: (u64, u64),
    #[cfg(test)]
    store: &'a Store,
}

impl StoreResourceSnapshot<'_> {
    pub fn with_bounded_setting_text<T>(
        &self,
        key: &str,
        limits: StoredSettingReadLimits,
        read: impl FnOnce(&str, StoredSettingTextMetadata) -> Result<T>,
    ) -> Result<Option<T>> {
        if self.transaction.total_changes() != self.revision.1 {
            return Err(StoredSettingUnavailable.into());
        }
        with_stored_setting_text_on(&self.transaction, self.text_encoding, key, limits, read)
    }

    pub fn resource_revision(&self) -> (u64, u64) {
        self.revision
    }
    fn require_current_snapshot(&self) -> Result<()> {
        if self.transaction.total_changes() != self.revision.1 {
            return Err(StoreInventoryUnavailable.into());
        }
        Ok(())
    }

    pub fn setting_inventory_headers_with_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<SettingInventoryHeader>> {
        self.require_current_snapshot()?;
        let limits = store_inventory_limits();
        let prefix_price =
            inventory_prefix_preflight(&self.transaction, self.text_encoding, prefix, limits)?;
        let mut statement=self.transaction.prepare("SELECT rowid,typeof(key)='text',typeof(value)='text',COALESCE(octet_length(key),0),COALESCE(octet_length(value),0)
            FROM settings WHERE substr(key,1,length(?1))=?1 ORDER BY key LIMIT ?2")?;
        let mut rows = statement.query(params![
            prefix,
            i64::try_from(
                limits
                    .rows
                    .checked_add(1)
                    .ok_or(StoreInventoryLimit { resource: "rows" })?
            )
            .map_err(|_| StoreInventoryLimit { resource: "rows" })?
        ])?;
        let mut headers = Vec::new();
        let mut encoded = prefix_price.encoded_bytes;
        let mut metadata = 0usize;
        let mut work = prefix_price.work_bytes;
        let mut scratch = prefix_price.scratch_bytes;
        while let Some(row) = rows.next()? {
            inventory_admit(
                "rows",
                headers
                    .len()
                    .checked_add(1)
                    .ok_or(StoreInventoryLimit { resource: "rows" })?,
                limits.rows,
            )?;
            let key_is_text: bool = row.get(1)?;
            let value_is_text: bool = row.get(2)?;
            if !key_is_text || !value_is_text {
                return Err(StoreInventoryUnavailable.into());
            }
            let key =
                usize::try_from(row.get::<_, i64>(3)?).map_err(|_| StoreInventoryUnavailable)?;
            let value =
                usize::try_from(row.get::<_, i64>(4)?).map_err(|_| StoreInventoryUnavailable)?;
            let ku = self
                .text_encoding
                .utf8_upper(key)
                .map_err(|_| StoreInventoryUnavailable)?;
            let vu = self
                .text_encoding
                .utf8_upper(value)
                .map_err(|_| StoreInventoryUnavailable)?;
            // All keys were priced before the literal-prefix predicate. Count
            // a matched value once, and reserve count plus final JSON passes.
            encoded = inventory_sum(&[encoded, value])?;
            metadata = inventory_sum(&[
                metadata,
                ku,
                2 * std::mem::size_of::<SettingInventoryHeader>(),
            ])?;
            work = inventory_sum(&[
                work,
                ku.checked_mul(2).ok_or(StoreInventoryLimit {
                    resource: "work bytes",
                })?,
                vu.checked_mul(3).ok_or(StoreInventoryLimit {
                    resource: "work bytes",
                })?,
            ])?;
            scratch = scratch.max(inventory_vm_scratch(self.text_encoding, value, vu)?);
            scratch = scratch.max(inventory_text_vm_scratch(self.text_encoding, key)?);
            inventory_admit("encoded bytes", encoded, limits.encoded_bytes)?;
            inventory_admit("metadata bytes", metadata, limits.metadata_bytes)?;
            inventory_admit("work bytes", work, limits.work_bytes)?;
            inventory_admit(
                "scratch bytes",
                inventory_sum(&[scratch, metadata])?,
                limits.scratch_bytes,
            )?;
            headers.push(SettingInventoryHeader {
                rowid: row.get(0)?,
                key_is_text,
                value_is_text,
                key_encoded_bytes: key,
                value_encoded_bytes: value,
                key_utf8_bytes_upper: ku,
                value_utf8_bytes_upper: vu,
            });
        }
        Ok(headers)
    }

    pub fn bounded_setting_inventory_key(&self, header: &SettingInventoryHeader) -> Result<String> {
        self.require_current_snapshot()?;
        let limits = store_inventory_limits();
        let (text, raw): (bool, i64) = self.transaction.query_row(
            "SELECT typeof(key)='text',COALESCE(octet_length(key),0) FROM settings WHERE rowid=?1",
            [header.rowid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let raw = usize::try_from(raw).map_err(|_| StoreInventoryUnavailable)?;
        if !text || raw != header.key_encoded_bytes {
            return Err(StoreInventoryUnavailable.into());
        }
        let encoded_max = match self.text_encoding {
            TextEncoding::Utf8 => 32 * 1024,
            TextEncoding::Utf16 => 64 * 1024,
        };
        inventory_admit("key bytes", raw, encoded_max)?;
        let upper = self
            .text_encoding
            .utf8_upper(raw)
            .map_err(|_| StoreInventoryUnavailable)?;
        let output = inventory_sum(&[upper, 2 * std::mem::size_of::<String>()])?;
        inventory_admit("key encoded bytes", raw, limits.encoded_bytes)?;
        inventory_admit("key metadata", output, limits.metadata_bytes)?;
        inventory_admit(
            "key work",
            upper.checked_mul(2).ok_or(StoreInventoryLimit {
                resource: "work bytes",
            })?,
            limits.work_bytes,
        )?;
        inventory_admit(
            "key scratch",
            inventory_sum(&[inventory_text_vm_scratch(self.text_encoding, raw)?, output])?,
            limits.scratch_bytes,
        )?;
        let mut statement = self
            .transaction
            .prepare("SELECT key FROM settings WHERE rowid=?1")?;
        let mut rows = statement.query([header.rowid])?;
        let row = rows.next()?.ok_or(StoreInventoryUnavailable)?;
        let key = row
            .get_ref(0)?
            .as_str()
            .map_err(|_| StoreInventoryUnavailable)?;
        inventory_admit("key bytes", key.len(), 32 * 1024)?;
        validate_identifier(key).map_err(|_| StoreInventoryUnavailable)?;
        #[cfg(test)]
        inventory_budget_tests::observe_key_copy(key.len());
        Ok(key.to_owned())
    }

    /// Decode one admitted boolean without copying its key or encoded JSON.
    pub fn bounded_setting_inventory_bool(&self, header: &SettingInventoryHeader) -> Result<bool> {
        self.require_current_snapshot()?;
        let limits = store_inventory_limits();
        let metadata: Option<(bool, bool, i64, i64)> = self.transaction.query_row(
            "SELECT typeof(key)='text',typeof(value)='text',COALESCE(octet_length(key),0),COALESCE(octet_length(value),0) FROM settings WHERE rowid=?1",
            [header.rowid],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional()?;
        let (key_text, value_text, key_bytes, value_bytes) =
            metadata.ok_or(StoreInventoryUnavailable)?;
        let key_bytes = usize::try_from(key_bytes).map_err(|_| StoreInventoryUnavailable)?;
        let value_bytes = usize::try_from(value_bytes).map_err(|_| StoreInventoryUnavailable)?;
        let key_upper = self
            .text_encoding
            .utf8_upper(key_bytes)
            .map_err(|_| StoreInventoryUnavailable)?;
        let value_upper = self
            .text_encoding
            .utf8_upper(value_bytes)
            .map_err(|_| StoreInventoryUnavailable)?;
        if !key_text
            || !value_text
            || !header.key_is_text
            || !header.value_is_text
            || key_bytes != header.key_encoded_bytes
            || value_bytes != header.value_encoded_bytes
            || key_upper != header.key_utf8_bytes_upper
            || value_upper != header.value_utf8_bytes_upper
        {
            return Err(StoreInventoryUnavailable.into());
        }
        inventory_admit(
            "setting boolean encoded bytes",
            inventory_sum(&[key_bytes, value_bytes])?,
            limits.encoded_bytes,
        )?;
        inventory_admit("setting boolean bytes", value_upper, limits.metadata_bytes)?;
        inventory_admit(
            "setting boolean work",
            value_upper.checked_mul(2).ok_or(StoreInventoryLimit {
                resource: "work bytes",
            })?,
            limits.work_bytes,
        )?;
        inventory_admit(
            "setting boolean scratch",
            inventory_vm_scratch(self.text_encoding, value_bytes, value_upper)?,
            limits.scratch_bytes,
        )?;
        let mut statement = self
            .transaction
            .prepare("SELECT value FROM settings WHERE rowid=?1")?;
        let mut rows = statement.query([header.rowid])?;
        let row = rows.next()?.ok_or(StoreInventoryUnavailable)?;
        let value = row
            .get_ref(0)?
            .as_str()
            .map_err(|_| StoreInventoryUnavailable)?;
        inventory_admit("setting boolean bytes", value.len(), value_upper)?;
        #[cfg(test)]
        inventory_budget_tests::observe_setting_bool_borrow(value.len());
        serde_json::from_str::<bool>(value).context("decode local setting collection")
    }

    /// Inspect the borrowed array before the caller prices selected DTO slots.
    /// No image record or ignored HTML field is owned by this count pass.
    pub fn image_manifest_inventory_metadata(
        &self,
        header: &SettingInventoryHeader,
        maximum_bytes: usize,
        maximum_images: usize,
    ) -> Result<usize> {
        self.require_current_snapshot()?;
        with_inventory_manifest_text_on(
            &self.transaction,
            self.text_encoding,
            header,
            maximum_bytes,
            1,
            |raw| inventory_manifest_image_count(raw, maximum_images),
        )
    }

    /// The caller prices its selected DTO's fields and slots from the metadata
    /// pass. Its deserializer skips unrequested fields instead of owning JSON.
    pub fn bounded_image_manifest_inventory<T: DeserializeOwned>(
        &self,
        header: &SettingInventoryHeader,
        maximum_bytes: usize,
        maximum_images: usize,
    ) -> Result<T> {
        self.require_current_snapshot()?;
        with_inventory_manifest_text_on(
            &self.transaction,
            self.text_encoding,
            header,
            maximum_bytes,
            2,
            |raw| {
                inventory_manifest_image_count(raw, maximum_images)?;
                #[cfg(test)]
                inventory_budget_tests::observe_manifest_read();
                serde_json::from_str(raw).context("decode inventory manifest fields")
            },
        )
    }

    pub fn chapter_body_metadata_by_uid(
        &self,
        book: &str,
        uid: &str,
    ) -> Result<Option<ChapterBodyTextMetadata>> {
        self.require_current_snapshot()?;
        validate_body_uid_inputs(book, uid)?;
        let mut statement = self.transaction.prepare(
            "SELECT body IS NOT NULL,typeof(body)='text',COALESCE(octet_length(body),0)
            FROM chapters WHERE book_id=?1 AND source_id=?2 LIMIT 2",
        )?;
        let mut rows = statement.query(params![book, uid])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let present: bool = row.get(0)?;
        let text: bool = row.get(1)?;
        let raw = usize::try_from(row.get::<_, i64>(2)?).map_err(|_| StoreInventoryUnavailable)?;
        if present && !text {
            return Err(StoreInventoryUnavailable.into());
        }
        if rows.next()?.is_some() {
            return Err(StoreInventoryUnavailable.into());
        }
        Ok(Some(ChapterBodyTextMetadata {
            body_present: present,
            encoded_bytes: raw,
            utf8_bytes_upper: self
                .text_encoding
                .utf8_upper(raw)
                .map_err(|_| StoreInventoryUnavailable)?,
        }))
    }

    pub fn with_bounded_chapter_body_by_uid<T>(
        &self,
        book: &str,
        uid: &str,
        maximum_bytes: usize,
        read: impl FnOnce(&str) -> Result<T>,
    ) -> Result<Option<T>> {
        let Some(meta) = self.chapter_body_metadata_by_uid(book, uid)? else {
            return Ok(None);
        };
        if !meta.body_present {
            return Ok(None);
        }
        inventory_admit("body bytes", meta.utf8_bytes_upper, maximum_bytes)?;
        let vm = match self.text_encoding {
            TextEncoding::Utf8 => meta.encoded_bytes.checked_add(2),
            TextEncoding::Utf16 => meta
                .encoded_bytes
                .checked_mul(3)
                .and_then(|v| v.checked_add(2)),
        }
        .ok_or(StoreInventoryLimit {
            resource: "body scratch",
        })?;
        inventory_admit("body scratch", vm, store_inventory_limits().scratch_bytes)?;
        inventory_admit(
            "body borrowed work",
            meta.utf8_bytes_upper
                .checked_mul(2)
                .ok_or(StoreInventoryLimit {
                    resource: "work bytes",
                })?,
            store_inventory_limits().work_bytes,
        )?;
        let mut statement = self
            .transaction
            .prepare("SELECT body FROM chapters WHERE book_id=?1 AND source_id=?2 LIMIT 2")?;
        let mut rows = statement.query(params![book, uid])?;
        let row = rows.next()?.ok_or(StoreInventoryUnavailable)?;
        let body = row
            .get_ref(0)?
            .as_str()
            .map_err(|_| StoreInventoryUnavailable)?;
        inventory_admit("body bytes", body.len(), maximum_bytes)?;
        #[cfg(test)]
        inventory_budget_tests::observe_body_borrow(body.len());
        read(body).map(Some)
    }
    /// Raw database-encoding bytes. Use utf8_upper_bytes before pricing a Rust
    /// String or JSON parse; UTF-16 storage must not be assumed to be UTF-8.
    pub fn setting_encoded_bytes(&self, key: &str) -> Result<Option<usize>> {
        setting_encoded_bytes_on(&self.transaction, key)
    }

    pub fn utf8_upper_bytes(&self, encoded: usize) -> Result<usize> {
        self.text_encoding.utf8_upper(encoded)
    }

    /// The byte argument is a UTF-8 admission allowance. The returned first
    /// value remains the raw encoded length; JSON is inspected only after it
    /// also fits the connection's conservative UTF-8 conversion bound.
    pub fn image_manifest_metadata(
        &self,
        key: &str,
        maximum_bytes: usize,
        maximum_images: usize,
    ) -> Result<Option<(usize, usize)>> {
        image_manifest_metadata_on(
            &self.transaction,
            self.text_encoding,
            key,
            maximum_bytes,
            maximum_images,
        )
    }

    pub fn bounded_image_manifest_setting<T: DeserializeOwned>(
        &self,
        key: &str,
        maximum_bytes: usize,
        maximum_images: usize,
    ) -> Result<Option<T>> {
        bounded_image_manifest_on(
            &self.transaction,
            self.text_encoding,
            key,
            maximum_bytes,
            maximum_images,
        )
    }

    pub fn chapter_body_bytes_by_uid(
        &self,
        book_id: &str,
        chapter_uid: &str,
    ) -> Result<Option<usize>> {
        chapter_body_encoded_bytes_on(&self.transaction, book_id, chapter_uid)
    }

    pub fn bounded_chapter_body_by_uid(
        &self,
        book_id: &str,
        chapter_uid: &str,
        maximum_bytes: usize,
    ) -> Result<Option<String>> {
        #[cfg(test)]
        self.store
            .chapter_body_reads
            .set(self.store.chapter_body_reads.get() + 1);
        let body = bounded_chapter_body_on(
            &self.transaction,
            self.text_encoding,
            book_id,
            chapter_uid,
            maximum_bytes,
        )?;
        #[cfg(test)]
        if let Some(body) = &body {
            if let Some(index) = self.store.chapter_index_by_uid(book_id, chapter_uid)? {
                self.store
                    .observe_body_materialization(book_id, index, Some(body));
            }
        }
        Ok(body)
    }
}

/// Bounded display metadata and scalar body-presence evidence, without text.
pub struct ChapterMetadata {
    pub chapter: Chapter,
    pub body_present: bool,
}

/// One admitted current row. The embedded Chapter has no body; never use it
/// as a replacement payload. Presence is separate from content authorization.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChapterAccessMetadata {
    pub chapter: Chapter,
    pub body_present: bool,
    pub locally_readable: bool,
}

/// A new body cannot safely replace the current text and its scalar anchors.
#[derive(Debug, thiserror::Error)]
#[error("content_changed")]
pub struct ContentConflict;

/// Preserved notes whose former chapter UID no longer appears in the catalog.
/// This value never names a readable chapter and must not be used for navigation.
pub const ORPHANED_CHAPTER_INDEX: u32 = u32::MAX;

/// Local application metadata. Keep it on its owning thread; source work runs separately.
pub struct Store {
    connection: Connection,
    text_encoding: TextEncoding,
    #[cfg(test)]
    book_reads: std::rc::Rc<std::cell::Cell<BookReadStats>>,
    #[cfg(test)]
    selected_metadata_reads: std::cell::Cell<SelectedMetadataReadStats>,
    #[cfg(test)]
    encoded_setting_reads: std::cell::Cell<EncodedSettingReadStats>,
    #[cfg(test)]
    book_introduction_reads: std::cell::Cell<BookIntroductionReadStats>,
    #[cfg(test)]
    annotation_reads: std::rc::Rc<std::cell::Cell<AnnotationReadStats>>,
    #[cfg(test)]
    materialized_chapter_bytes: std::cell::Cell<usize>,
    #[cfg(test)]
    chapter_header_reads: std::cell::Cell<usize>,
    #[cfg(test)]
    chapter_body_reads: std::cell::Cell<usize>,
    #[cfg(test)]
    bounded_array_reads: std::cell::Cell<usize>,
    #[cfg(test)]
    body_materializations: std::cell::RefCell<HashMap<(String, u32), (usize, usize)>>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path != Path::new(":memory:") {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent).context("create metadata directory")?;
            }
        }
        let mut connection = Connection::open(path).context("open metadata database")?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        let text_encoding = TextEncoding::read(&connection)?;
        initialize_schema(&mut connection)?;
        #[cfg(unix)]
        if path != Path::new(":memory:") {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        let store = Self {
            connection,
            text_encoding,
            #[cfg(test)]
            book_reads: std::rc::Rc::new(std::cell::Cell::new(BookReadStats::default())),
            #[cfg(test)]
            selected_metadata_reads: std::cell::Cell::new(SelectedMetadataReadStats::default()),
            #[cfg(test)]
            encoded_setting_reads: std::cell::Cell::new(EncodedSettingReadStats::default()),
            #[cfg(test)]
            book_introduction_reads: std::cell::Cell::new(BookIntroductionReadStats::default()),
            #[cfg(test)]
            annotation_reads: std::rc::Rc::new(
                std::cell::Cell::new(AnnotationReadStats::default()),
            ),
            #[cfg(test)]
            materialized_chapter_bytes: std::cell::Cell::new(0),
            #[cfg(test)]
            chapter_header_reads: std::cell::Cell::new(0),
            #[cfg(test)]
            chapter_body_reads: std::cell::Cell::new(0),
            #[cfg(test)]
            bounded_array_reads: std::cell::Cell::new(0),
            #[cfg(test)]
            body_materializations: std::cell::RefCell::new(HashMap::new()),
        };
        store.recover_interrupted_tasks()?;
        Ok(store)
    }

    pub fn upsert_book(&self, book: &Book) -> Result<()> {
        validate_identifier(&book.id)?;
        validate_progress(book.progress)?;
        let transaction = self.connection.unchecked_transaction()?;
        let count = *cached_counts_for_books_on(
            &transaction,
            self.text_encoding,
            std::iter::once(book.id.as_str()),
        )?
        .get(book.id.as_str())
        .ok_or(CachedResourceLimit)?;
        upsert_book_on(&transaction, book, count, book.on_shelf)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn resource_verification_snapshot(&self) -> Result<StoreResourceSnapshot<'_>> {
        let before = self.resource_revision()?;
        let transaction = self.connection.unchecked_transaction()?;
        transaction.query_row("SELECT COUNT(*) FROM sqlite_schema", [], |row| {
            row.get::<_, i64>(0)
        })?;
        let revision = self.resource_revision()?;
        if before != revision {
            return Err(StoreInventoryUnavailable.into());
        }
        Ok(StoreResourceSnapshot {
            transaction,
            text_encoding: self.text_encoding,
            revision,
            #[cfg(test)]
            store: self,
        })
    }

    pub fn resource_revision(&self) -> Result<(u64, u64)> {
        let version: i64 = self
            .connection
            .pragma_query_value(None, "data_version", |row| row.get(0))?;
        Ok((
            u64::try_from(version).map_err(|_| StoreInventoryUnavailable)?,
            self.connection.total_changes(),
        ))
    }

    #[cfg(test)]
    fn annotation_meter(&self) -> AnnotationMeterGuard {
        let previous =
            ANNOTATION_METER.with(|meter| meter.replace(Some(self.annotation_reads.clone())));
        AnnotationMeterGuard { previous }
    }

    #[cfg(test)]
    pub(crate) fn annotation_read_stats(&self) -> AnnotationReadStats {
        self.annotation_reads.get()
    }

    #[cfg(test)]
    pub(crate) fn reset_annotation_read_stats(&self) {
        self.annotation_reads.set(AnnotationReadStats::default());
    }

    #[cfg(test)]
    fn book_meter(&self) -> BookMeterGuard {
        let previous = BOOK_METER.with(|meter| meter.replace(Some(self.book_reads.clone())));
        BookMeterGuard { previous }
    }

    #[cfg(test)]
    pub(crate) fn book_read_stats(&self) -> BookReadStats {
        self.book_reads.get()
    }

    #[cfg(test)]
    pub(crate) fn reset_book_read_stats(&self) {
        self.book_reads.set(BookReadStats::default());
    }

    #[cfg(test)]
    pub(crate) fn selected_metadata_read_stats(&self) -> SelectedMetadataReadStats {
        self.selected_metadata_reads.get()
    }

    #[cfg(test)]
    pub(crate) fn reset_selected_metadata_read_stats(&self) {
        self.selected_metadata_reads
            .set(SelectedMetadataReadStats::default());
    }

    #[cfg(test)]
    fn observe_selected_metadata(&self, update: impl FnOnce(&mut SelectedMetadataReadStats)) {
        let mut stats = self.selected_metadata_reads.get();
        update(&mut stats);
        self.selected_metadata_reads.set(stats);
    }

    #[cfg(test)]
    pub(crate) fn encoded_setting_read_stats(&self) -> EncodedSettingReadStats {
        self.encoded_setting_reads.get()
    }

    #[cfg(test)]
    pub(crate) fn reset_encoded_setting_read_stats(&self) {
        self.encoded_setting_reads
            .set(EncodedSettingReadStats::default());
    }

    #[cfg(test)]
    pub(crate) fn book_introduction_read_stats(&self) -> BookIntroductionReadStats {
        self.book_introduction_reads.get()
    }

    #[cfg(test)]
    pub(crate) fn reset_book_introduction_read_stats(&self) {
        self.book_introduction_reads
            .set(BookIntroductionReadStats::default());
    }

    pub fn setting_encoded_bytes(&self, key: &str) -> Result<Option<usize>> {
        self.resource_verification_snapshot()?
            .setting_encoded_bytes(key)
    }

    pub fn image_manifest_metadata(
        &self,
        key: &str,
        maximum_bytes: usize,
        maximum_images: usize,
    ) -> Result<Option<(usize, usize)>> {
        self.resource_verification_snapshot()?
            .image_manifest_metadata(key, maximum_bytes, maximum_images)
    }

    pub fn bounded_image_manifest_setting<T: DeserializeOwned>(
        &self,
        key: &str,
        maximum_bytes: usize,
        maximum_images: usize,
    ) -> Result<Option<T>> {
        self.resource_verification_snapshot()?
            .bounded_image_manifest_setting(key, maximum_bytes, maximum_images)
    }

    pub fn chapter_body_bytes_by_uid(
        &self,
        book_id: &str,
        chapter_uid: &str,
    ) -> Result<Option<usize>> {
        self.resource_verification_snapshot()?
            .chapter_body_bytes_by_uid(book_id, chapter_uid)
    }

    pub fn bounded_chapter_body_by_uid(
        &self,
        book_id: &str,
        chapter_uid: &str,
        maximum_bytes: usize,
    ) -> Result<Option<String>> {
        self.resource_verification_snapshot()?
            .bounded_chapter_body_by_uid(book_id, chapter_uid, maximum_bytes)
    }

    pub fn upsert_books(&self, books: &[Book]) -> Result<()> {
        selected_cached_admit(books.len(), body_length_limits().rows)?;
        let transaction = self.connection.unchecked_transaction()?;
        for book in books {
            validate_identifier(&book.id)?;
            validate_progress(book.progress)?;
        }
        let counts = cached_counts_for_books_on(
            &transaction,
            self.text_encoding,
            books.iter().map(|book| book.id.as_str()),
        )?;
        for book in books {
            let count = *counts.get(book.id.as_str()).ok_or(CachedResourceLimit)?;
            upsert_book_on(&transaction, book, count, book.on_shelf)?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Replace shelf membership atomically without deleting downloaded chapters or notes.
    pub fn replace_shelf(&self, books: &[Book]) -> Result<()> {
        selected_cached_admit(books.len(), body_length_limits().rows)?;
        let transaction = self.connection.unchecked_transaction()?;
        for book in books {
            validate_identifier(&book.id)?;
            validate_progress(book.progress)?;
        }
        let counts = cached_counts_for_books_on(
            &transaction,
            self.text_encoding,
            books.iter().map(|book| book.id.as_str()),
        )?;
        transaction.execute("UPDATE books SET on_shelf = 0", [])?;
        for book in books {
            let count = *counts.get(book.id.as_str()).ok_or(CachedResourceLimit)?;
            upsert_book_on(&transaction, book, count, true)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn books(&self) -> Result<Vec<Book>> {
        self.query_books("SELECT * FROM books ORDER BY rowid DESC", [])
    }

    pub fn book_metadata(&self) -> Result<Vec<BookMetadata>> {
        self.book_metadata_with_limits(book_metadata_limits())
    }

    pub fn book_metadata_with_limits(
        &self,
        limits: BookMetadataLimits,
    ) -> Result<Vec<BookMetadata>> {
        #[cfg(test)]
        let _meter = self.book_meter();
        let transaction = self.connection.unchecked_transaction()?;
        let books = book_metadata_on(&transaction, self.text_encoding, None, limits)?;
        transaction.commit()?;
        Ok(books)
    }

    pub fn book_metadata_by_id(&self, id: &str) -> Result<Option<BookMetadata>> {
        #[cfg(test)]
        let _meter = self.book_meter();
        if id.len() > 4096 {
            return Err(BookMetadataLimit {
                resource: "identifier bytes",
                limit: 4096,
                requested: id.len(),
            }
            .into());
        }
        validate_identifier(id)?;
        let transaction = self.connection.unchecked_transaction()?;
        let books = book_metadata_on(
            &transaction,
            self.text_encoding,
            Some(id),
            book_metadata_limits(),
        )?;
        transaction.commit()?;
        Ok(books.into_iter().next())
    }

    pub fn book_statistics_metadata(&self) -> Result<Vec<BookStatisticsMetadata>> {
        self.book_statistics_metadata_with_limits(book_metadata_limits())
    }

    pub fn book_statistics_metadata_with_limits(
        &self,
        limits: BookMetadataLimits,
    ) -> Result<Vec<BookStatisticsMetadata>> {
        #[cfg(test)]
        let _meter = self.book_meter();
        let transaction = self.connection.unchecked_transaction()?;
        let books = book_statistics_metadata_on(&transaction, self.text_encoding, limits)?;
        transaction.commit()?;
        Ok(books)
    }

    pub fn book_on_shelf(&self, id: &str) -> Result<Option<bool>> {
        if id.len() > 4096 {
            return Err(BookMetadataLimit {
                resource: "identifier bytes",
                limit: 4096,
                requested: id.len(),
            }
            .into());
        }
        validate_identifier(id)?;
        Ok(self
            .connection
            .query_row("SELECT on_shelf FROM books WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// An existence consumer has no need to own any book text.
    pub fn book_exists(&self, id: &str) -> Result<bool> {
        selected_book_id(id)?;
        #[cfg(test)]
        self.observe_selected_metadata(|stats| stats.book_scalar_reads += 1);
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM books WHERE id=?1)",
            [id],
            |row| row.get(0),
        )?)
    }

    pub fn book_source(&self, id: &str) -> Result<Option<String>> {
        self.selected_book_text(id, false)
    }

    pub fn book_title(&self, id: &str) -> Result<Option<String>> {
        self.selected_book_text(id, true)
    }

    /// Read only the introduction tab's fields after encoding/copy/node
    /// admission. Unknown metadata remains borrowed and is never a Value tree.
    pub fn book_introduction(&self, id: &str) -> Result<Option<BookIntroduction>> {
        selected_book_id(id)?;
        #[cfg(test)]
        let _meter = self.book_meter();
        let limits = book_introduction_limits();
        let transaction = self.connection.unchecked_transaction()?;
        let introduction = self.book_introduction_on(&transaction, id, limits)?;
        transaction.commit()?;
        Ok(introduction)
    }

    pub fn book_detail_metadata(
        &self,
        id: &str,
        include_introduction: bool,
    ) -> Result<Option<BookDetailMetadata>> {
        selected_book_id(id)?;
        #[cfg(test)]
        let _meter = self.book_meter();
        let limits = book_metadata_limits();
        let transaction = self.connection.unchecked_transaction()?;
        let Some(book) = book_metadata_on(&transaction, self.text_encoding, Some(id), limits)?
            .into_iter()
            .next()
        else {
            transaction.commit()?;
            return Ok(None);
        };
        let introduction = if include_introduction {
            let strings = book_metadata_add(
                book_metadata_add(book.id.len(), book.title.len())?,
                book_metadata_add(
                    book_metadata_add(book.author.len(), book.category.len())?,
                    book_metadata_add(book.cover_url.len(), book.source.len())?,
                )?,
            )?;
            let used = book_metadata_add(
                strings,
                book_metadata_mul(
                    2,
                    book_metadata_add(
                        std::mem::size_of::<BookMetadata>(),
                        std::mem::size_of::<BookScalarMetadata>(),
                    )?,
                )?,
            )?;
            let remaining = limits
                .bytes
                .checked_sub(used)
                .ok_or(BookMetadataUnavailable)?;
            let mut intro_limits = book_introduction_limits();
            intro_limits.bytes = intro_limits.bytes.min(remaining);
            self.book_introduction_on(&transaction, id, intro_limits)?
        } else {
            None
        };
        transaction.commit()?;
        Ok(Some(BookDetailMetadata { book, introduction }))
    }

    fn book_introduction_on(
        &self,
        connection: &Connection,
        id: &str,
        limits: BookIntroductionLimits,
    ) -> Result<Option<BookIntroduction>> {
        let header: Option<(i64, bool, i64)> = connection.query_row(
            "SELECT rowid,typeof(intro)='text',COALESCE(octet_length(intro),0) FROM books WHERE id=?1", [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        let Some((rowid, valid, intro_encoded)) = header else {
            return Ok(None);
        };
        if !valid {
            return Err(BookMetadataUnavailable.into());
        }
        let intro_encoded = usize::try_from(intro_encoded).map_err(|_| BookMetadataUnavailable)?;
        let intro_upper = self
            .text_encoding
            .utf8_upper(intro_encoded)
            .map_err(|_| BookMetadataUnavailable)?;
        let metadata_key = format!("book.metadata:{id}");
        let metadata: Option<(i64, bool, i64)> = connection.query_row(
            "SELECT rowid,typeof(value)='text',COALESCE(octet_length(value),0) FROM settings WHERE key=?1", [&metadata_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        if metadata.is_some_and(|(_, valid, _)| !valid) {
            return Err(BookMetadataUnavailable.into());
        }
        let metadata_encoded = usize::try_from(metadata.map_or(0, |(_, _, bytes)| bytes))
            .map_err(|_| BookMetadataUnavailable)?;
        let metadata_upper = self
            .text_encoding
            .utf8_upper(metadata_encoded)
            .map_err(|_| BookMetadataUnavailable)?;
        book_metadata_admit(
            "introduction metadata bytes",
            metadata_upper,
            limits.metadata_bytes,
        )?;
        let fixed = book_metadata_mul(2, std::mem::size_of::<BookIntroduction>())?;
        // Include SQLite encoding conversion, Rust copies and UI publication.
        // This logical forecast is not a total RSS or absolute time limit.
        let required = book_metadata_add(
            book_metadata_add(intro_encoded, book_metadata_mul(2, intro_upper)?)?,
            book_metadata_add(
                book_metadata_add(metadata_encoded, book_metadata_mul(2, metadata_upper)?)?,
                book_metadata_add(
                    book_metadata_mul(limits.metadata_nodes, 128)?,
                    book_metadata_add(book_metadata_mul(limits.field_bytes, 4)?, fixed)?,
                )?,
            )?,
        )?;
        book_metadata_admit("introduction working-set bytes", required, limits.bytes)?;
        #[cfg(test)]
        book_introduction_tests::after_preflight()?;
        let fields = if let Some((metadata_rowid, _, _)) = metadata {
            let mut statement = connection.prepare("SELECT value FROM settings WHERE rowid=?1")?;
            let mut rows = statement.query([metadata_rowid])?;
            let row = rows.next()?.ok_or(BookMetadataUnavailable)?;
            let raw = book_metadata_text(row, 0, metadata_upper)?;
            #[cfg(test)]
            {
                let mut stats = self.book_introduction_reads.get();
                stats.metadata_reads = stats
                    .metadata_reads
                    .checked_add(1)
                    .ok_or(BookMetadataUnavailable)?;
                self.book_introduction_reads.set(stats);
            }
            let fields = parse_book_introduction_metadata(raw, limits)?;
            #[cfg(test)]
            {
                let mut stats = self.book_introduction_reads.get();
                stats.metadata_nodes = stats
                    .metadata_nodes
                    .checked_add(fields.nodes)
                    .ok_or(BookMetadataUnavailable)?;
                self.book_introduction_reads.set(stats);
            }
            fields
        } else {
            IntroductionFields::default()
        };
        let intro = {
            let mut statement = connection.prepare("SELECT intro FROM books WHERE rowid=?1")?;
            let mut rows = statement.query([rowid])?;
            let row = rows.next()?.ok_or(BookMetadataUnavailable)?;
            let intro = book_metadata_text(row, 0, intro_upper)?;
            #[cfg(test)]
            book_intro_copied(intro.len());
            intro.to_owned()
        };
        Ok(Some(BookIntroduction {
            intro,
            publisher: fields.publisher,
            isbn: fields.isbn,
        }))
    }

    fn selected_book_text(&self, id: &str, title: bool) -> Result<Option<String>> {
        selected_book_id(id)?;
        #[cfg(test)]
        let _meter = self.book_meter();
        let transaction = self.connection.unchecked_transaction()?;
        let sql = if title {
            "SELECT rowid,typeof(title)='text',COALESCE(octet_length(title),0) FROM books WHERE id=?1"
        } else {
            "SELECT rowid,typeof(source)='text',COALESCE(octet_length(source),0) FROM books WHERE id=?1"
        };
        let header: Option<(i64, bool, i64)> = transaction
            .query_row(sql, [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .optional()?;
        let Some((rowid, valid, encoded)) = header else {
            transaction.commit()?;
            return Ok(None);
        };
        if !valid {
            return Err(BookMetadataUnavailable.into());
        }
        let limits = book_metadata_limits();
        book_metadata_admit("rows", 1, limits.rows)?;
        book_metadata_admit("SQL cells", 4, limits.sql_cells)?;
        let maximum = self
            .text_encoding
            .utf8_upper(usize::try_from(encoded).map_err(|_| BookMetadataUnavailable)?)
            .map_err(|_| BookMetadataUnavailable)?;
        let fixed = book_metadata_mul(
            2,
            book_metadata_add(std::mem::size_of::<String>(), std::mem::size_of::<i64>())?,
        )?;
        book_metadata_admit(
            "working-set bytes",
            book_metadata_add(maximum, fixed)?,
            limits.bytes,
        )?;
        #[cfg(test)]
        selected_metadata_tests::after_book_preflight()?;
        let value = {
            let sql = if title {
                "SELECT title FROM books WHERE rowid=?1"
            } else {
                "SELECT source FROM books WHERE rowid=?1"
            };
            let mut statement = transaction.prepare(sql)?;
            let mut rows = statement.query([rowid])?;
            let row = rows.next()?.ok_or(BookMetadataUnavailable)?;
            let text = book_metadata_text(row, 0, maximum)?;
            #[cfg(test)]
            {
                book_metadata_copied(text.len());
                self.observe_selected_metadata(|stats| {
                    stats.book_text_rows += 1;
                    stats.book_text_bytes += text.len();
                });
            }
            text.to_owned()
        };
        transaction.commit()?;
        Ok(Some(value))
    }

    pub fn book_reading_state(&self, id: &str) -> Result<Option<BookReadingState>> {
        selected_book_id(id)?;
        let transaction = self.connection.unchecked_transaction()?;
        #[cfg(test)]
        self.observe_selected_metadata(|stats| stats.book_scalar_reads += 1);
        let valid = transaction.query_row(
            "SELECT typeof(progress) IN ('real','integer') AND typeof(finished)='integer' FROM books WHERE id=?1",
            [id], |row| row.get::<_, bool>(0),
        ).optional()?;
        let Some(valid) = valid else {
            transaction.commit()?;
            return Ok(None);
        };
        if !valid {
            return Err(BookMetadataUnavailable.into());
        }
        let limits = book_metadata_limits();
        book_metadata_admit("rows", 1, limits.rows)?;
        book_metadata_admit("SQL cells", 3, limits.sql_cells)?;
        book_metadata_admit(
            "working-set bytes",
            book_metadata_mul(2, std::mem::size_of::<BookReadingState>())?,
            limits.bytes,
        )?;
        #[cfg(test)]
        self.observe_selected_metadata(|stats| stats.book_scalar_reads += 1);
        let state = transaction.query_row(
            "SELECT CASE WHEN typeof(progress) IN ('real','integer') THEN progress END,CASE WHEN typeof(finished)='integer' THEN finished END FROM books WHERE id=?1",
            [id],
            |row| {
                Ok(BookReadingState {
                    progress: row.get(0)?,
                    finished: row.get(1)?,
                })
            },
        )?;
        if !state.progress.is_finite() || !(0.0..=1.0).contains(&state.progress) {
            return Err(BookMetadataUnavailable.into());
        }
        transaction.commit()?;
        Ok(Some(state))
    }

    pub fn search_book_metadata(&self, query: &str, shelf_only: bool) -> Result<Vec<BookMetadata>> {
        let limits = store_inventory_limits();
        let trim_work = query.len().checked_mul(2).ok_or(StoreInventoryLimit {
            resource: "search arithmetic",
        })?;
        inventory_admit("search work", trim_work, limits.work_bytes)?;
        let trimmed = query.trim();
        let query_scratch = trimmed.len().checked_mul(6).ok_or(StoreInventoryLimit {
            resource: "search arithmetic",
        })?;
        let mut work = inventory_sum(&[
            trim_work,
            trimmed.len().checked_mul(8).ok_or(StoreInventoryLimit {
                resource: "search arithmetic",
            })?,
        ])?;
        inventory_admit("search work", work, limits.work_bytes)?;
        inventory_admit("search scratch", query_scratch, limits.scratch_bytes)?;
        let query = trimmed.to_lowercase();
        // The lowercase query remains alive while metadata is materialized.
        // Reserve it before the metadata SQL gate permits any field copies.
        let mut metadata_limits = book_metadata_limits();
        let remaining = limits
            .metadata_bytes
            .min(limits.scratch_bytes)
            .checked_sub(query_scratch)
            .ok_or(StoreInventoryLimit {
                resource: "search scratch",
            })?;
        metadata_limits.bytes = metadata_limits.bytes.min(remaining);
        let books = self
            .book_metadata_with_limits(metadata_limits)
            .map_err(|error| {
                if error
                    .downcast_ref::<BookMetadataLimit>()
                    .is_some_and(|limit| limit.resource == "working-set bytes")
                {
                    StoreInventoryLimit {
                        resource: "search metadata and query scratch",
                    }
                    .into()
                } else {
                    error
                }
            })?;
        let held = books.iter().try_fold(0usize, |sum, b| {
            inventory_sum(&[
                sum,
                2 * std::mem::size_of::<BookMetadata>(),
                b.id.len(),
                b.title.len(),
                b.author.len(),
                b.category.len(),
                b.cover_url.len(),
                b.source.len(),
            ])
        })?;
        inventory_admit(
            "search scratch",
            inventory_sum(&[held, query_scratch])?,
            limits.metadata_bytes.min(limits.scratch_bytes),
        )?;
        let mut result = Vec::with_capacity(books.len());
        for book in books {
            if shelf_only && !book.on_shelf {
                continue;
            }
            let mut matched = query.is_empty();
            if !matched {
                for field in [&book.title, &book.author, &book.category] {
                    let expanded = field.len().checked_mul(6).ok_or(StoreInventoryLimit {
                        resource: "search arithmetic",
                    })?;
                    work = inventory_sum(&[
                        work,
                        field.len().checked_mul(8).ok_or(StoreInventoryLimit {
                            resource: "search arithmetic",
                        })?,
                        query.len(),
                    ])?;
                    inventory_admit("search work", work, limits.work_bytes)?;
                    inventory_admit(
                        "search scratch",
                        inventory_sum(&[held, query_scratch, expanded])?,
                        limits.metadata_bytes.min(limits.scratch_bytes),
                    )?;
                    // Keep whole-string Unicode lowercase and its contextual
                    // mappings. Charge only fields reached by short circuit.
                    #[cfg(test)]
                    search_matcher_budget_tests::observe_transform(field.len());
                    let lowered = field.to_lowercase();
                    if lowered.contains(&query) {
                        matched = true;
                        break;
                    }
                }
            }
            if matched {
                result.push(book)
            }
        }
        Ok(result)
    }

    pub fn shelf(&self) -> Result<Vec<Book>> {
        self.query_books("SELECT * FROM books WHERE on_shelf = 1 ORDER BY rowid", [])
    }

    /// Check membership without projecting any book text or allocating an ID.
    pub fn shelf_exists(&self) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM books WHERE on_shelf = 1)",
            [],
            |row| row.get(0),
        )?)
    }

    /// Preserve shelf row order while admitting only the selected identifier.
    pub fn first_shelf_book_id(&self) -> Result<Option<String>> {
        #[cfg(test)]
        let _meter = self.book_meter();
        let transaction = self.connection.unchecked_transaction()?;
        let header: Option<(i64, bool, i64)> = transaction
            .query_row(
                "SELECT rowid,typeof(id)='text',COALESCE(octet_length(id),0)
             FROM books WHERE on_shelf = 1 ORDER BY rowid LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((rowid, is_text, encoded_bytes)) = header else {
            transaction.commit()?;
            return Ok(None);
        };
        if !is_text {
            return Err(BookMetadataUnavailable.into());
        }
        let limits = book_metadata_limits();
        book_metadata_admit("rows", 1, limits.rows)?;
        // Three scalar header fields and one selected SQL text field.
        book_metadata_admit("SQL cells", 4, limits.sql_cells)?;
        let encoded_bytes = usize::try_from(encoded_bytes).map_err(|_| BookMetadataUnavailable)?;
        let maximum_bytes = self
            .text_encoding
            .utf8_upper(encoded_bytes)
            .map_err(|_| BookMetadataUnavailable)?;
        book_metadata_admit("identifier bytes", maximum_bytes, limits.identifier_bytes)?;
        // This is a logical field/slot allowance, not a total SQLite/Rust RSS bound.
        let fixed_bytes = book_metadata_mul(
            2,
            book_metadata_add(std::mem::size_of::<String>(), std::mem::size_of::<i64>())?,
        )?;
        book_metadata_admit(
            "working-set bytes",
            book_metadata_add(maximum_bytes, fixed_bytes)?,
            limits.bytes,
        )?;
        #[cfg(test)]
        shelf_admission_tests::after_preflight()?;
        let id = {
            let mut statement =
                transaction.prepare("SELECT id FROM books WHERE rowid=?1 AND on_shelf = 1")?;
            let mut rows = statement.query([rowid])?;
            let row = rows.next()?.ok_or(BookMetadataUnavailable)?;
            let id = book_metadata_text(row, 0, maximum_bytes)?;
            validate_identifier(id).map_err(|_| BookMetadataUnavailable)?;
            #[cfg(test)]
            book_metadata_copied(id.len());
            id.to_owned()
        };
        transaction.commit()?;
        Ok(Some(id))
    }

    pub fn book(&self, id: &str) -> Result<Option<Book>> {
        #[cfg(test)]
        let _meter = self.book_meter();
        Ok(self
            .connection
            .query_row("SELECT * FROM books WHERE id = ?1", [id], read_book)
            .optional()?)
    }

    /// Substring matching is done in Rust because SQLite's built-in case folding is ASCII-only.
    #[cfg(test)]
    pub fn search_books(&self, query: &str, shelf_only: bool) -> Result<Vec<Book>> {
        let query = query.trim().to_lowercase();
        let books = if shelf_only {
            self.shelf()?
        } else {
            self.books()?
        };
        Ok(books
            .into_iter()
            .filter(|book| {
                query.is_empty()
                    || book.title.to_lowercase().contains(&query)
                    || book.author.to_lowercase().contains(&query)
                    || book.category.to_lowercase().contains(&query)
            })
            .collect())
    }

    pub fn set_on_shelf(&self, id: &str, on_shelf: bool) -> Result<()> {
        ensure!(
            self.connection.execute(
                "UPDATE books SET on_shelf = ?2 WHERE id = ?1",
                params![id, on_shelf]
            )? > 0,
            "book does not exist"
        );
        Ok(())
    }

    pub fn remove_from_shelf(&self, id: &str) -> Result<()> {
        self.set_on_shelf(id, false)
    }

    pub fn set_book_progress(&self, id: &str, progress: f32, finished: bool) -> Result<()> {
        validate_progress(progress)?;
        ensure!(
            self.connection.execute(
                "UPDATE books SET progress = ?2, finished = ?3 WHERE id = ?1",
                params![id, progress, finished]
            )? > 0,
            "book does not exist"
        );
        Ok(())
    }

    pub fn upsert_chapter(&self, chapter: &Chapter) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        upsert_chapter_on(&transaction, chapter)?;
        update_cached_count(&transaction, &chapter.book_id, self.text_encoding)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert_chapters(&self, chapters: &[Chapter]) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut affected = HashSet::new();
        for chapter in chapters {
            upsert_chapter_on(&transaction, chapter)?;
            affected.insert(chapter.book_id.as_str());
        }
        update_cached_counts_on(&transaction, self.text_encoding, affected.into_iter())?;
        transaction.commit()?;
        Ok(())
    }

    /// Atomically replace a complete catalog by stable chapter UID, not ordinal identity.
    ///
    /// The caller must stop any worker using the previous catalog before this call and
    /// discard its late results. New indices must be the contiguous range `0..len`.
    /// Existing cached text cannot be rewritten here: changed text requires a separate
    /// content-replacement use case that can revalidate character anchors.
    /// Deleted-chapter notes and bookmarks remain readable records at
    /// `ORPHANED_CHAPTER_INDEX`; they are not attached to a replacement chapter.
    pub fn replace_catalog(&self, book_id: &str, chapters: &[Chapter]) -> Result<()> {
        validate_identifier(book_id)?;
        ensure!(
            chapters.len() <= MAX_CATALOG_HEADERS,
            "catalog has too many chapters"
        );
        let mut new_indices = HashMap::new();
        let mut indices = HashSet::new();
        for chapter in chapters {
            validate_identifier(&chapter.id)?;
            ensure!(
                chapter.book_id == book_id,
                "catalog contains a chapter from another book"
            );
            ensure!(
                new_indices
                    .insert(chapter.id.as_str(), chapter.index)
                    .is_none(),
                "catalog contains duplicate chapter UIDs"
            );
            ensure!(
                indices.insert(chapter.index),
                "catalog contains duplicate chapter indices"
            );
            ensure!(
                (chapter.index as usize) < chapters.len(),
                "catalog indices must be contiguous and zero-based"
            );
            if let Some(body) = &chapter.body {
                ensure!(
                    !body.trim().is_empty(),
                    "empty chapter body is not a cache entry"
                );
            }
        }
        let input_metadata =
            catalog_input_metadata_bytes(book_id, chapters, MAX_CATALOG_HEADER_BYTES as usize)?;
        self.validate_catalog_metadata_capacity(book_id, input_metadata)?;
        let transaction = self.connection.unchecked_transaction()?;
        validate_catalog_capacity_with_encoding_on(
            &transaction,
            book_id,
            input_metadata,
            MAX_CATALOG_HEADER_BYTES as usize,
            self.text_encoding,
        )?;
        ensure!(
            transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM books WHERE id=?1)",
                [book_id],
                |row| row.get::<_, bool>(0)
            )?,
            "book does not exist"
        );
        let local_content: bool = transaction.query_row(
            "SELECT source<>'weread' FROM books WHERE id=?1",
            [book_id],
            |row| row.get(0),
        )?;
        let old_chapters = chapter_headers_on(&transaction, book_id, false, self.text_encoding)?;
        let old_position = self.position(book_id)?;
        let mut old_by_uid = HashMap::new();
        let mut remapped_indices = HashMap::new();
        for chapter in &old_chapters {
            ensure!(
                old_by_uid
                    .insert(chapter.chapter.id.as_str(), chapter)
                    .is_none(),
                "existing catalog has ambiguous duplicate chapter UIDs"
            );
            if let Some(new_index) = new_indices.get(chapter.chapter.id.as_str()) {
                remapped_indices.insert(chapter.chapter.index, *new_index);
            }
        }
        let replacement: Vec<Chapter> = chapters
            .iter()
            .map(|chapter| -> Result<Chapter> {
                let old = old_by_uid.get(chapter.id.as_str());
                if let (Some(incoming), Some(existing)) =
                    (chapter.body.as_ref(), old.filter(|old| old.body_present))
                {
                    let equal: bool = transaction.query_row(
                        "SELECT body IS ?3 FROM chapters WHERE book_id=?1 AND chapter_index=?2",
                        params![book_id, existing.chapter.index, incoming],
                        |row| row.get(0),
                    )?;
                    ensure!(
                        equal,
                        "catalog replacement cannot rewrite cached text for an existing chapter UID"
                    );
                }
                let body_present =
                    chapter.body.is_some() || old.is_some_and(|old| old.body_present);
                let mut replacement = Chapter {
                    id: chapter.id.clone(),
                    book_id: chapter.book_id.clone(),
                    index: chapter.index,
                    title: chapter.title.clone(),
                    cached: false,
                    body: None,
                    content_status: chapter.content_status,
                    content_account_key: chapter.content_account_key.clone(),
                };
                if let Some(old) = old {
                    // Catalog metadata cannot attest a different content access state.
                    replacement.content_status = old.chapter.content_status;
                    replacement.content_account_key = old.chapter.content_account_key.clone();
                }
                if local_content
                    && body_present
                    && replacement.content_status == ContentStatus::Unknown
                {
                    replacement.content_status = ContentStatus::Full;
                }
                if !body_present && replacement.content_status == ContentStatus::Full {
                    replacement.content_status = ContentStatus::Unknown;
                }
                ensure!(
                    local_content
                        || replacement.content_status != ContentStatus::Full
                        || !replacement.content_account_key.is_empty(),
                    "remote full content has no account owner"
                );
                replacement.cached =
                    replacement.content_status == ContentStatus::Full && body_present;
                Ok(replacement)
            })
            .collect::<Result<_>>()?;
        let cached_indices: HashSet<u32> = replacement
            .iter()
            .filter(|chapter| chapter.cached)
            .map(|chapter| chapter.index)
            .collect();
        let mut catalog_held = input_metadata;
        for old in &old_chapters {
            catalog_held = inventory_sum(&[
                catalog_held,
                2 * std::mem::size_of::<ChapterMetadata>(),
                old.chapter.id.len(),
                old.chapter.book_id.len(),
                old.chapter.title.len(),
                old.chapter.content_account_key.len().checked_mul(2).ok_or(
                    StoreInventoryLimit {
                        resource: "catalog owner bytes",
                    },
                )?,
                192,
            ])?;
        }
        catalog_held = inventory_sum(&[
            catalog_held,
            chapters.len().checked_mul(128).ok_or(StoreInventoryLimit {
                resource: "catalog map bytes",
            })?,
        ])?;
        let mut tasks =
            catalog_task_inventory_on(&transaction, self.text_encoding, book_id, catalog_held)?;
        for task in &mut tasks {
            ensure!(
                task.book_id == book_id,
                "persistent download payload references another book"
            );
            ensure!(
                task.next_index <= task.chapters.len(),
                "persistent download cursor exceeds task length"
            );
            let mut mapped = Vec::new();
            let mut retained_completed = 0;
            for (ordinal, old_index) in task.chapters.iter().enumerate() {
                if let Some(new_index) = remapped_indices.get(old_index) {
                    mapped.push(*new_index);
                    if ordinal < task.next_index {
                        retained_completed += 1;
                    }
                }
            }
            let lost_chapters = mapped.len() != task.chapters.len();
            let cached_prefix = mapped
                .iter()
                .take_while(|index| cached_indices.contains(index))
                .count();
            task.chapters = mapped;
            task.next_index = retained_completed.min(cached_prefix);
            if task.chapters.is_empty() || (lost_chapters && task.next_index == task.chapters.len())
            {
                task.status = DownloadStatus::Cancelled;
                task.error = Some("catalog_changed".into());
            } else if task.next_index == task.chapters.len() {
                task.status = DownloadStatus::Complete;
                task.error = None;
            } else if task.status == DownloadStatus::Running || lost_chapters {
                task.status = DownloadStatus::Paused;
                if lost_chapters {
                    task.error = Some("catalog_changed".into());
                }
            }
        }

        // A temporary mapping prevents index swaps from successively remapping the
        // same record twice. All payload, anchor and count changes share this transaction.
        transaction.execute_batch("CREATE TEMP TABLE weread_catalog_map (old_index INTEGER PRIMARY KEY,new_index INTEGER)")?;
        for chapter in &old_chapters {
            transaction.execute(
                "INSERT INTO weread_catalog_map (old_index,new_index) VALUES (?1,?2)",
                params![
                    chapter.chapter.index,
                    remapped_indices.get(&chapter.chapter.index).copied()
                ],
            )?;
        }
        transaction.execute("UPDATE positions SET chapter_index=COALESCE((SELECT new_index FROM weread_catalog_map WHERE old_index=positions.chapter_index),0), char_offset=CASE WHEN (SELECT new_index FROM weread_catalog_map WHERE old_index=positions.chapter_index) IS NULL THEN 0 ELSE char_offset END WHERE book_id=?1", [book_id])?;
        transaction.execute("UPDATE annotations SET chapter_index=COALESCE((SELECT new_index FROM weread_catalog_map WHERE old_index=annotations.chapter_index),?2) WHERE book_id=?1", params![book_id,ORPHANED_CHAPTER_INDEX])?;
        transaction.execute("UPDATE bookmarks SET chapter_index=COALESCE((SELECT new_index FROM weread_catalog_map WHERE old_index=bookmarks.chapter_index),?2) WHERE book_id=?1", params![book_id,ORPHANED_CHAPTER_INDEX])?;
        // Keep bodies in their existing SQL rows. A disjoint, nonnegative index
        // namespace permits swaps without cloning a corpus into Rust or TEMP.
        transaction.execute("UPDATE chapters SET chapter_index=chapter_index+?2 WHERE book_id=?1
            AND NOT EXISTS (SELECT 1 FROM weread_catalog_map WHERE old_index=chapters.chapter_index AND new_index=chapters.chapter_index)",
            params![book_id, CATALOG_INDEX_OFFSET])?;
        for (chapter, incoming) in replacement.iter().zip(chapters) {
            if let Some(old) = old_by_uid.get(chapter.id.as_str()) {
                let old_index = i64::from(old.chapter.index)
                    + if old.chapter.index == chapter.index {
                        0
                    } else {
                        CATALOG_INDEX_OFFSET
                    };
                let changed = transaction.execute(
                    "UPDATE chapters SET chapter_index=?3,title=?4,body=COALESCE(body,?5),
                     content_status=?6,content_account_key=?7 WHERE book_id=?1 AND chapter_index=?2
                     AND (chapter_index<>?3 OR title IS NOT ?4 OR (body IS NULL AND ?5 IS NOT NULL)
                          OR content_status IS NOT ?6 OR content_account_key IS NOT ?7)",
                    params![
                        book_id,
                        old_index,
                        chapter.index,
                        chapter.title,
                        incoming.body,
                        chapter.content_status.as_str(),
                        chapter.content_account_key
                    ],
                )?;
                ensure!(changed == 1 || transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM chapters WHERE book_id=?1 AND chapter_index=?2 AND source_id=?3)",
                    params![book_id, chapter.index, chapter.id], |row| row.get::<_, bool>(0),
                )?, "catalog row disappeared during reindexing");
            } else {
                transaction.execute(
                    "INSERT INTO chapters (book_id,chapter_index,source_id,title,body,content_status,content_account_key)
                     VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    params![book_id, chapter.index, chapter.id, chapter.title, incoming.body,
                        chapter.content_status.as_str(), chapter.content_account_key],
                )?;
            }
        }
        transaction.execute(
            "DELETE FROM chapters WHERE book_id=?1 AND chapter_index>=?2",
            params![book_id, CATALOG_INDEX_OFFSET],
        )?;
        transaction.execute(
            "UPDATE books SET total_chapters=?2 WHERE id=?1",
            params![book_id, replacement.len() as u32],
        )?;
        update_cached_count(&transaction, book_id, self.text_encoding)?;
        if old_position
            .is_some_and(|position| !remapped_indices.contains_key(&position.chapter_index))
        {
            transaction.execute(
                "UPDATE books SET progress=0,finished=0 WHERE id=?1",
                [book_id],
            )?;
        }
        let task_state_held =
            tasks
                .iter()
                .try_fold(catalog_held, |held, task| -> Result<usize> {
                    let (bytes, _) = task_borrowed_price(task)?;
                    inventory_sum(&[held, bytes])
                })?;
        for mut task in tasks {
            chapter_state::reconcile_catalog_states_on(
                &transaction,
                self.text_encoding,
                &mut task,
                task_state_held,
            )?;
            persist_task_on(&transaction, &task)?;
        }
        transaction.execute_batch("DROP TABLE weread_catalog_map")?;
        transaction.commit()?;
        Ok(())
    }

    pub fn chapters(&self, book_id: &str) -> Result<Vec<Chapter>> {
        let mut statement = self
            .connection
            .prepare("SELECT * FROM chapters WHERE book_id = ?1 ORDER BY chapter_index")?;
        let rows = statement.query_map([book_id], |row| {
            let chapter = read_chapter(row)?;
            #[cfg(test)]
            {
                self.materialized_chapter_bytes.set(
                    self.materialized_chapter_bytes.get()
                        + chapter.body.as_ref().map_or(0, String::len),
                );
                self.observe_body_materialization(
                    &chapter.book_id,
                    chapter.index,
                    chapter.body.as_deref(),
                );
            }
            Ok(chapter)
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Bounded catalog identities and permission flags, without title/body copies.
    pub fn chapter_headers(&self, book_id: &str) -> Result<Vec<Chapter>> {
        #[cfg(test)]
        self.chapter_header_reads
            .set(self.chapter_header_reads.get() + 1);
        let transaction = self.connection.unchecked_transaction()?;
        let headers = chapter_headers_on(&transaction, book_id, false, self.text_encoding)?;
        transaction.commit()?;
        Ok(headers.into_iter().map(|header| header.chapter).collect())
    }

    /// Price old owner-key copies from scalar byte evidence before opening a
    /// header snapshot. No old title/body/account string is copied by this gate.
    pub fn validate_catalog_metadata_capacity(
        &self,
        book_id: &str,
        input_metadata: usize,
    ) -> Result<()> {
        validate_catalog_capacity_with_encoding_on(
            &self.connection,
            book_id,
            input_metadata,
            MAX_CATALOG_HEADER_BYTES as usize,
            self.text_encoding,
        )
    }

    /// Display/navigation metadata retains titles but never reads chapter bodies.
    pub fn chapter_metadata(&self, book_id: &str) -> Result<Vec<Chapter>> {
        let transaction = self.connection.unchecked_transaction()?;
        let headers = chapter_headers_on(&transaction, book_id, true, self.text_encoding)?;
        transaction.commit()?;
        Ok(headers.into_iter().map(|header| header.chapter).collect())
    }

    /// Display callers can distinguish absent/unverified content without
    /// copying body payloads merely to inspect Option<String>. This cheap
    /// presence is not authority to advance a cached download checkpoint.
    pub fn chapter_metadata_with_presence(&self, book_id: &str) -> Result<Vec<ChapterMetadata>> {
        let transaction = self.connection.unchecked_transaction()?;
        let headers = chapter_headers_on(&transaction, book_id, true, self.text_encoding)?;
        transaction.commit()?;
        Ok(headers)
    }

    /// Select only the requested title; no body, UID or owner is projected.
    pub fn chapter_title(&self, book_id: &str, index: u32) -> Result<Option<String>> {
        self.selected_chapter_text(book_id, index, true)
    }

    /// Raw current SQL UID, including legacy empty/NUL UTF8 TEXT. This is not
    /// the stricter cache-key validation performed by chapter_uid.
    pub fn chapter_source_id(&self, book_id: &str, index: u32) -> Result<Option<String>> {
        self.selected_chapter_text(book_id, index, false)
    }

    fn selected_chapter_text(
        &self,
        book_id: &str,
        index: u32,
        title: bool,
    ) -> Result<Option<String>> {
        selected_chapter_id(book_id)?;
        let transaction = self.connection.unchecked_transaction()?;
        let sql = if title {
            "SELECT rowid,typeof(title)='text',COALESCE(octet_length(title),0)
             FROM chapters WHERE book_id=?1 AND chapter_index=?2"
        } else {
            "SELECT rowid,typeof(source_id)='text',COALESCE(octet_length(source_id),0)
             FROM chapters WHERE book_id=?1 AND chapter_index=?2"
        };
        let header: Option<(i64, bool, i64)> = transaction
            .query_row(sql, params![book_id, index], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()?;
        let Some((rowid, valid, encoded)) = header else {
            transaction.commit()?;
            return Ok(None);
        };
        if !valid {
            return Err(CachedResourceLimit.into());
        }
        let maximum = self
            .text_encoding
            .utf8_upper(usize::try_from(encoded).map_err(|_| CachedResourceLimit)?)?;
        if !title {
            selected_cached_admit(maximum, 4096)?;
        }
        let fixed = selected_cached_bytes(&[
            std::mem::size_of::<String>()
                .checked_mul(2)
                .ok_or(CachedResourceLimit)?,
            std::mem::size_of::<i64>()
                .checked_mul(2)
                .ok_or(CachedResourceLimit)?,
        ])?;
        let limits = chapter_access_limits();
        selected_cached_admit(
            selected_cached_bytes(&[maximum, fixed])?,
            limits.metadata_bytes,
        )?;
        selected_cached_admit(maximum, limits.work_bytes)?;
        selected_cached_admit(
            selected_cached_bytes(&[maximum.checked_mul(2).ok_or(CachedResourceLimit)?, fixed])?,
            limits.temporary_bytes,
        )?;
        #[cfg(test)]
        selected_metadata_tests::after_chapter_preflight()?;
        let value = {
            let sql = if title {
                "SELECT title FROM chapters WHERE rowid=?1"
            } else {
                "SELECT source_id FROM chapters WHERE rowid=?1"
            };
            let mut statement = transaction.prepare(sql)?;
            let mut rows = statement.query([rowid])?;
            let row = rows.next()?.ok_or(CachedResourceLimit)?;
            let text = cache_text(row, 0, maximum)?;
            #[cfg(test)]
            self.observe_selected_metadata(|stats| {
                stats.chapter_rows += 1;
                stats.chapter_string_bytes += text.len();
            });
            text.to_owned()
        };
        transaction.commit()?;
        Ok(Some(value))
    }

    /// Preserve current permission/presence and Rust trim semantics without
    /// owning the selected body or treating presence as remote authority.
    pub fn chapter_access_metadata(
        &self,
        book_id: &str,
        index: u32,
    ) -> Result<Option<ChapterAccessMetadata>> {
        selected_chapter_id(book_id)?;
        let transaction = self.connection.unchecked_transaction()?;
        let header: Option<(i64, i64, i64, i64, i64, i64, bool, bool)> = transaction.query_row(
            "SELECT rowid,COALESCE(octet_length(source_id),0),COALESCE(octet_length(title),0),
             COALESCE(octet_length(content_account_key),0),COALESCE(octet_length(content_status),0),
             COALESCE(octet_length(body),0),body IS NOT NULL,
             typeof(source_id)='text' AND typeof(title)='text' AND typeof(content_account_key)='text'
             AND typeof(content_status)='text' AND typeof(chapter_index)='integer'
             AND (body IS NULL OR typeof(body)='text')
             FROM chapters WHERE book_id=?1 AND chapter_index=?2",
            params![book_id, index], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,
                row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?)),
        ).optional()?;
        let Some((rowid, uid, title, owner, status, body, body_present, valid)) = header else {
            transaction.commit()?;
            return Ok(None);
        };
        if !valid {
            return Err(CachedResourceLimit.into());
        }
        let encoded = [uid, title, owner, status, body]
            .map(|bytes| usize::try_from(bytes).map_err(|_| CachedResourceLimit));
        let [uid, title, owner, status, body] = encoded;
        let body_encoded = body?;
        let uid = self.text_encoding.utf8_upper(uid?)?;
        let title = self.text_encoding.utf8_upper(title?)?;
        let owner = self.text_encoding.utf8_upper(owner?)?;
        let status = self.text_encoding.utf8_upper(status?)?;
        let body = self.text_encoding.utf8_upper(body_encoded)?;
        selected_cached_admit(uid, 4096)?;
        selected_cached_admit(owner, 16 * 1024)?;
        selected_cached_admit(status, 32)?;
        let text_bytes = selected_cached_bytes(&[uid, title, owner, status])?;
        let fixed = selected_cached_bytes(&[
            book_id.len(),
            std::mem::size_of::<ChapterAccessMetadata>()
                .checked_mul(2)
                .ok_or(CachedResourceLimit)?,
            std::mem::size_of::<(i64, i64, i64, i64, i64, i64, bool, bool)>(),
        ])?;
        let limits = chapter_access_limits();
        selected_cached_admit(
            selected_cached_bytes(&[text_bytes, fixed])?,
            limits.metadata_bytes,
        )?;
        // UTF8 validation and trim may each inspect the selected body once.
        // Work is prepaid once and is not refunded when temporary values drop.
        selected_cached_admit(
            selected_cached_bytes(&[text_bytes, body.checked_mul(2).ok_or(CachedResourceLimit)?])?,
            limits.work_bytes,
        )?;
        let header_temporary =
            selected_cached_bytes(&[text_bytes.checked_mul(2).ok_or(CachedResourceLimit)?, fixed])?;
        let body_temporary = selected_cached_bytes(&[body_encoded, body, fixed])?;
        selected_cached_admit(header_temporary.max(body_temporary), limits.temporary_bytes)?;
        #[cfg(test)]
        selected_metadata_tests::after_chapter_preflight()?;
        let content_status = {
            let mut statement =
                transaction.prepare("SELECT content_status FROM chapters WHERE rowid=?1")?;
            let mut rows = statement.query([rowid])?;
            let row = rows.next()?.ok_or(CachedResourceLimit)?;
            ContentStatus::parse(cache_text(row, 0, status)?).ok_or(CachedResourceLimit)?
        };
        // Drop this statement and its SQLite conversion before owning headers.
        // These are logical work/scratch limits, not total RSS or wall-clock bounds.
        let locally_readable = if body_present && body_encoded > 0 {
            let mut statement = transaction.prepare("SELECT body FROM chapters WHERE rowid=?1")?;
            let mut rows = statement.query([rowid])?;
            let row = rows.next()?.ok_or(CachedResourceLimit)?;
            let value = row.get_ref(0)?;
            #[cfg(test)]
            self.observe_selected_metadata(|stats| {
                stats.body_borrows += 1;
                if let rusqlite::types::ValueRef::Text(bytes) = &value {
                    stats.body_scan_bytes += bytes.len();
                }
            });
            let text = value.as_str().map_err(|_| CachedResourceLimit)?;
            selected_cached_admit(text.len(), body)?;
            !text.trim().is_empty()
        } else {
            false
        };
        let chapter = {
            let mut statement = transaction.prepare("SELECT source_id,title,content_account_key,chapter_index FROM chapters WHERE rowid=?1")?;
            let mut rows = statement.query([rowid])?;
            let row = rows.next()?.ok_or(CachedResourceLimit)?;
            let id = cache_text(row, 0, uid)?;
            let title = cache_text(row, 1, title)?;
            let owner = cache_text(row, 2, owner)?;
            let current_index: u32 = row.get(3)?;
            if current_index != index {
                return Err(CachedResourceLimit.into());
            }
            #[cfg(test)]
            let owned_bytes =
                selected_cached_bytes(&[id.len(), title.len(), owner.len(), book_id.len()])?;
            #[cfg(test)]
            self.observe_selected_metadata(|stats| {
                stats.chapter_rows += 1;
                stats.chapter_string_bytes += owned_bytes;
            });
            Chapter {
                id: id.to_owned(),
                book_id: book_id.to_owned(),
                index: current_index,
                title: title.to_owned(),
                body: None,
                cached: content_status == ContentStatus::Full && locally_readable,
                content_status,
                content_account_key: owner.to_owned(),
            }
        };
        transaction.commit()?;
        Ok(Some(ChapterAccessMetadata {
            chapter,
            body_present,
            locally_readable,
        }))
    }

    /// Resolve one visible card's cache identity without loading chapter text.
    pub fn chapter_uid(&self, book_id: &str, chapter_index: u32) -> Result<Option<String>> {
        let maximum = self.text_encoding.maximum_encoded(4096)?;
        let mut statement = self.connection.prepare(
            "SELECT CASE WHEN typeof(source_id)='text' AND COALESCE(octet_length(source_id),0) BETWEEN 1 AND ?3
             THEN source_id ELSE NULL END FROM chapters WHERE book_id=?1 AND chapter_index=?2",
        )?;
        let mut rows = statement.query(params![book_id, chapter_index, maximum])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        if matches!(row.get_ref(0)?, rusqlite::types::ValueRef::Null) {
            return Err(CachedResourceLimit.into());
        }
        let uid = cache_text(row, 0, 4096)?;
        #[cfg(test)]
        byte_admission_tests::observe_text_copy("uid", uid.len());
        Ok(Some(uid.to_owned()))
    }

    /// Resolve one unique UID using at most two scalar rows, without catalog
    /// strings or bodies. Ambiguous UIDs and invalid indices remain unavailable.
    pub fn chapter_index_by_uid(&self, book_id: &str, chapter_uid: &str) -> Result<Option<u32>> {
        if book_id.len() > 4096 || chapter_uid.len() > 4096 {
            return Err(CachedResourceLimit.into());
        }
        validate_identifier(book_id)?;
        validate_identifier(chapter_uid)?;
        let mut statement = self.connection.prepare(
            "SELECT CASE WHEN typeof(chapter_index)='integer' AND chapter_index BETWEEN 0 AND ?3
             THEN chapter_index ELSE NULL END FROM chapters WHERE book_id=?1 AND source_id=?2 LIMIT 2",
        )?;
        let mut rows = statement.query(params![book_id, chapter_uid, u32::MAX])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let index = row.get::<_, Option<u32>>(0)?;
        if rows.next()?.is_some() {
            return Ok(None);
        }
        Ok(index)
    }

    /// SQL-only display presence preserves Some(empty TEXT) versus None. This
    /// is not full-content, account-ownership, or download-checkpoint authority.
    pub fn chapter_body_present(&self, book_id: &str, index: u32) -> Result<bool> {
        validate_identifier(book_id)?;
        let value = self
            .connection
            .query_row(
                "SELECT CASE WHEN body IS NULL THEN 0 WHEN typeof(body)='text' THEN 1 ELSE NULL END
             FROM chapters WHERE book_id=?1 AND chapter_index=?2",
                params![book_id, index],
                |row| row.get::<_, Option<bool>>(0),
            )
            .optional()?;
        match value {
            None => Ok(false),
            Some(Some(present)) => Ok(present),
            Some(None) => Err(CachedResourceLimit.into()),
        }
    }

    /// SQLite TEXT length uses Unicode scalar units for validated body strings.
    pub fn chapter_scalar_count(&self, book_id: &str, chapter_index: u32) -> Result<usize> {
        let length = self.connection.query_row(
            "SELECT COALESCE(length(body),0) FROM chapters WHERE book_id=?1 AND chapter_index=?2",
            params![book_id, chapter_index], |row| row.get::<_, i64>(0),
        ).optional()?.unwrap_or(0);
        usize::try_from(length).context("invalid cached chapter scalar count")
    }

    #[cfg(test)]
    pub(crate) fn materialized_chapter_bytes(&self) -> usize {
        self.materialized_chapter_bytes.get()
    }

    #[cfg(test)]
    pub(crate) fn chapter_header_read_count(&self) -> usize {
        self.chapter_header_reads.get()
    }

    #[cfg(test)]
    pub(crate) fn chapter_body_read_count(&self) -> usize {
        self.chapter_body_reads.get()
    }

    #[cfg(test)]
    pub(crate) fn chapter_body_materialization_stats(
        &self,
        book_id: &str,
        index: u32,
    ) -> (usize, usize) {
        self.body_materializations
            .borrow()
            .get(&(book_id.to_owned(), index))
            .copied()
            .unwrap_or_default()
    }

    /// Observe rows returned by the public body-reading APIs, including empty
    /// or absent text. Existing whole-catalog byte counters keep their meaning.
    #[cfg(test)]
    fn observe_body_materialization(&self, book_id: &str, index: u32, body: Option<&str>) {
        let mut materializations = self.body_materializations.borrow_mut();
        let stats = materializations
            .entry((book_id.to_owned(), index))
            .or_default();
        stats.0 = stats.0.saturating_add(1);
        stats.1 = stats.1.saturating_add(body.map_or(0, str::len));
    }

    #[cfg(test)]
    pub(crate) fn bounded_array_read_count(&self) -> usize {
        self.bounded_array_reads.get()
    }

    pub fn chapter(&self, book_id: &str, index: u32) -> Result<Option<Chapter>> {
        #[cfg(test)]
        self.chapter_body_reads
            .set(self.chapter_body_reads.get() + 1);
        Ok(self
            .connection
            .query_row(
                "SELECT * FROM chapters WHERE book_id = ?1 AND chapter_index = ?2",
                params![book_id, index],
                |row| {
                    let chapter = read_chapter(row)?;
                    #[cfg(test)]
                    self.observe_body_materialization(book_id, index, chapter.body.as_deref());
                    Ok(chapter)
                },
            )
            .optional()?)
    }

    pub fn chapter_body(&self, book_id: &str, index: u32) -> Result<Option<String>> {
        #[cfg(test)]
        self.chapter_body_reads
            .set(self.chapter_body_reads.get() + 1);
        Ok(self
            .connection
            .query_row(
                "SELECT body FROM chapters WHERE book_id = ?1 AND chapter_index = ?2",
                params![book_id, index],
                |row| {
                    let body = row.get::<_, Option<String>>(0)?;
                    #[cfg(test)]
                    self.observe_body_materialization(book_id, index, body.as_deref());
                    Ok(body)
                },
            )
            .optional()?
            .flatten())
    }

    /// Fetch one canonical body without loading every cached chapter during image layout.
    pub fn chapter_body_by_uid(&self, book_id: &str, chapter_uid: &str) -> Result<Option<String>> {
        #[cfg(test)]
        self.chapter_body_reads
            .set(self.chapter_body_reads.get() + 1);
        #[cfg(not(test))]
        let query = "SELECT body FROM chapters WHERE book_id=?1 AND source_id=?2 LIMIT 2";
        #[cfg(test)]
        let query =
            "SELECT body,chapter_index FROM chapters WHERE book_id=?1 AND source_id=?2 LIMIT 2";
        let mut statement = self.connection.prepare(query)?;
        let rows = statement.query_map(params![book_id, chapter_uid], |row| {
            let body = row.get::<_, Option<String>>(0)?;
            #[cfg(test)]
            if let Ok(index) = row.get::<_, u32>(1) {
                self.observe_body_materialization(book_id, index, body.as_deref());
            }
            Ok(body)
        })?;
        let bodies = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            bodies.len() <= 1,
            "chapter UID is ambiguous in the local catalog"
        );
        Ok(bodies.into_iter().next().flatten())
    }

    /// Commit a complete chapter body and its cache count in the same transaction.
    pub fn cache_chapter_body(&self, book_id: &str, index: u32, body: &str) -> Result<()> {
        ensure!(
            !body.trim().is_empty(),
            "empty chapter body is not a completed download"
        );
        let transaction = self.connection.unchecked_transaction()?;
        let (source, previous): (String, Option<String>) = transaction.query_row(
            "SELECT b.source,c.body FROM chapters AS c JOIN books AS b ON b.id=c.book_id
             WHERE c.book_id=?1 AND c.chapter_index=?2",
            params![book_id, index],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if source == "weread" && previous.as_deref().is_some_and(|previous| previous != body) {
            return Err(ContentConflict.into());
        }
        ensure!(
            transaction.execute(
                "UPDATE chapters SET body=?3,
                 content_status=CASE WHEN ?4='weread' THEN 'unknown' ELSE 'full' END,
                 content_account_key=''
                 WHERE book_id=?1 AND chapter_index=?2",
                params![book_id, index, body, source]
            )? > 0,
            "chapter does not exist"
        );
        update_cached_count(&transaction, book_id, self.text_encoding)?;
        transaction.commit()?;
        Ok(())
    }

    /// Check task ownership and a safe canonical transition before changing image manifests.
    /// The transaction repeats these checks so a stale preflight cannot authorize a commit.
    /// Validate a packet before image materialization without mutating persistent state.
    pub fn validate_download_chapter(
        &self,
        task: &DownloadTask,
        chapter: &Chapter,
        next_index: usize,
    ) -> Result<()> {
        validate_download_chapter_on(&self.connection, task, chapter, next_index)?;
        Ok(())
    }

    /// Commit content access, text, anchors, count and cursor as one guarded transition.
    /// Persist body, access proof, cache count and cursor as one checkpoint.
    /// Limited content retains its cursor. Returns true only for a newly full body.
    pub fn commit_download_checkpoint(
        &self,
        task: &DownloadTask,
        chapter: &Chapter,
        next_index: usize,
        updated_at: i64,
    ) -> Result<bool> {
        let transaction = self.connection.unchecked_transaction()?;
        let validated = validate_download_chapter_on(&transaction, task, chapter, next_index)?;
        write_download_chapter_on(&transaction, task, chapter, &validated)?;
        let next_cursor =
            i64::try_from(next_index).context("download cursor exceeds SQLite range")?;
        ensure!(
            transaction.execute(
                "UPDATE download_tasks SET status='running',updated_at=?3,
                 payload=json_set(payload,'$.next_index',?2,'$.status','running','$.updated_at',?3)
                 WHERE id=?1 AND status='running'",
                params![task.id, next_cursor, updated_at],
            )? == 1,
            "download checkpoint task disappeared"
        );
        transaction.commit()?;
        Ok(!validated.was_full && validated.is_full)
    }

    /// A bodyless download packet proves only intent. Recheck the durable plan,
    /// current owner and actual readable full body in SQL before acknowledging
    /// its cursor. Existing content, counts and anchors are never rewritten.
    pub fn commit_cached_download_checkpoint(
        &self,
        task: &DownloadTask,
        chapter_uid: &str,
        chapter_index: u32,
        next_index: usize,
        current_account_key: &str,
        updated_at: i64,
    ) -> Result<()> {
        ensure!(
            !task.include_images,
            "an image checkpoint requires freshly verified resources"
        );
        self.commit_cached_checkpoint(
            task,
            chapter_uid,
            chapter_index,
            next_index,
            current_account_key,
            updated_at,
        )
    }

    /// Keep image reuse separate from the text-only cached entry point. The
    /// private proof fields can only come from actual resource verification.
    pub(crate) fn commit_verified_cached_image_download_checkpoint(
        &self,
        task: &DownloadTask,
        proof: crate::book_assets::VerifiedChapterImages,
        chapter_index: u32,
        next_index: usize,
        current_account_key: &str,
        updated_at: i64,
    ) -> Result<()> {
        ensure!(
            task.include_images && proof.book_id() == task.book_id,
            "verified image checkpoint has a mismatched task or book"
        );
        self.commit_cached_checkpoint(
            task,
            proof.chapter_uid(),
            chapter_index,
            next_index,
            current_account_key,
            updated_at,
        )
    }

    fn commit_cached_checkpoint(
        &self,
        task: &DownloadTask,
        chapter_uid: &str,
        chapter_index: u32,
        next_index: usize,
        current_account_key: &str,
        updated_at: i64,
    ) -> Result<()> {
        ensure!(
            task.id.len() <= 4096
                && task.book_id.len() <= 4096
                && chapter_uid.len() <= 4096
                && task.account_key.len() <= 16 * 1024
                && task.chapters.len() <= MAX_CATALOG_HEADERS,
            "cached checkpoint metadata exceeds its allowance"
        );
        validate_identifier(&task.id)?;
        validate_identifier(&task.book_id)?;
        validate_identifier(chapter_uid)?;
        ensure!(
            task.status == DownloadStatus::Running
                && task.next_index < task.chapters.len()
                && task.chapters.get(task.next_index) == Some(&chapter_index)
                && task.next_index.checked_add(1) == Some(next_index)
                && task.chapters.iter().collect::<HashSet<_>>().len() == task.chapters.len()
                && !current_account_key.is_empty()
                && current_account_key == task.account_key,
            "cached checkpoint does not match the running task or current owner"
        );
        let cursor =
            i64::try_from(task.next_index).context("cached cursor exceeds SQLite range")?;
        let next_cursor =
            i64::try_from(next_index).context("cached cursor exceeds SQLite range")?;
        let chapter_path = format!("$.chapters[{}]", task.next_index);
        let plan = serde_json::to_string(&task.chapters)?;
        let transaction = self.connection.unchecked_transaction()?;
        ensure!(
            transaction.execute(
                "UPDATE download_tasks AS t SET status='running',updated_at=:updated_at,
                   payload=json_set(payload,'$.next_index',:next_cursor,'$.status','running',
                     '$.updated_at',:updated_at)
                 WHERE t.id=:task_id AND t.book_id=:book_id AND t.status='running'
                   AND t.updated_at=:old_updated_at
                   AND json_extract(t.payload,'$.id')=:task_id
                   AND json_extract(t.payload,'$.book_id')=:book_id
                   AND json_extract(t.payload,'$.account_key')=:account_key
                   AND json_extract(t.payload,'$.status')='running'
                   AND json_type(t.payload,'$.next_index')='integer'
                   AND json_extract(t.payload,'$.next_index')=:cursor
                   AND json_type(t.payload,'$.chapters')='array'
                   AND json_extract(t.payload,'$.chapters')=json(:plan)
                   AND json_type(t.payload,:chapter_path)='integer'
                   AND json_extract(t.payload,:chapter_path)=:chapter_index
                   AND json_extract(t.payload,'$.include_images')=:include_images
                   AND json_extract(t.payload,'$.updated_at')=:old_updated_at
                   AND json_extract(t.payload,'$.error') IS :error
                   AND EXISTS(SELECT 1 FROM chapters AS c JOIN books AS b ON b.id=c.book_id
                     WHERE c.book_id=t.book_id AND c.chapter_index=:chapter_index
                       AND c.source_id=:chapter_uid AND c.content_status='full'
                       AND c.content_account_key=:account_key AND typeof(c.body)='text'
                       AND CASE WHEN c.body GLOB :nonblank THEN 1
                           ELSE instr(c.body,char(0))>0 END)",
                named_params! {
                    ":task_id": &task.id,
                    ":book_id": &task.book_id,
                    ":account_key": current_account_key,
                    ":old_updated_at": task.updated_at,
                    ":updated_at": updated_at,
                    ":cursor": cursor,
                    ":next_cursor": next_cursor,
                    ":plan": &plan,
                    ":chapter_path": &chapter_path,
                    ":chapter_index": chapter_index,
                    ":chapter_uid": chapter_uid,
                    ":include_images": task.include_images,
                    ":error": task.error.as_deref(),
                    ":nonblank": NONBLANK_BODY_PATTERN,
                },
            )? == 1,
            "cached checkpoint has stale task, owner, catalog or unreadable content"
        );
        transaction.commit()?;
        Ok(())
    }

    /// Remove only replaceable content. Reading positions, annotations and bookmarks survive.
    pub fn clear_book_cache(&self, book_id: &str) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE chapters SET body=NULL,content_status='unknown',content_account_key='' WHERE book_id=?1",
            [book_id],
        )?;
        update_cached_count(&transaction, book_id, self.text_encoding)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn clear_content_cache(&self) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE chapters SET body=NULL,content_status='unknown',content_account_key=''",
            [],
        )?;
        transaction.execute("UPDATE books SET cached_chapters = 0", [])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_position(&self, position: &ReadingPosition) -> Result<()> {
        validate_identifier(&position.book_id)?;
        self.connection.execute("INSERT INTO positions (book_id, chapter_index, char_offset, updated_at, device) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(book_id) DO UPDATE SET chapter_index=excluded.chapter_index, char_offset=excluded.char_offset, updated_at=excluded.updated_at, device=excluded.device", params![position.book_id, position.chapter_index, position.char_offset, position.updated_at, position.device])?;
        Ok(())
    }

    pub fn position(&self, book_id: &str) -> Result<Option<ReadingPosition>> {
        if self.connection.is_autocommit() {
            let transaction = self.connection.unchecked_transaction()?;
            let value = position_metadata_on(&transaction, self.text_encoding, book_id)?;
            transaction.commit()?;
            Ok(value)
        } else {
            position_metadata_on(&self.connection, self.text_encoding, book_id)
        }
    }

    pub fn save_annotation(&self, annotation: &Annotation) -> Result<()> {
        validate_identifier(&annotation.id)?;
        ensure!(
            annotation.start < annotation.end,
            "annotation range must not be empty or reversed"
        );
        if self.connection.is_autocommit() {
            let transaction = self.connection.unchecked_transaction()?;
            save_annotation_on(&transaction, self.text_encoding, annotation)?;
            transaction.commit()?;
            Ok(())
        } else {
            save_annotation_on(&self.connection, self.text_encoding, annotation)
        }
    }

    /// Historical fixture inspection only. Production consumers use bounded
    /// metadata and explicitly selected annotation records below.
    #[cfg(test)]
    pub fn annotations(&self, book_id: &str) -> Result<Vec<Annotation>> {
        let mut statement = self.connection.prepare("SELECT * FROM annotations WHERE book_id = ?1 ORDER BY chapter_index, start_offset, created_at")?;
        let rows = statement.query_map([book_id], |row| {
            Ok(Annotation {
                id: row.get("id")?,
                book_id: row.get("book_id")?,
                chapter_index: row.get("chapter_index")?,
                start: row.get("start_offset")?,
                end: row.get("end_offset")?,
                quote: row.get("quote")?,
                note: row.get("note")?,
                created_at: row.get("created_at")?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn annotation_count(&self, book_id: &str) -> Result<u64> {
        annotation_input_id(book_id, 4096)?;
        annotation_count_on(&self.connection, book_id, None, None, None)
    }

    pub fn annotation_count_for_chapter(&self, book_id: &str, chapter: u32) -> Result<u64> {
        annotation_input_id(book_id, 4096)?;
        annotation_count_on(&self.connection, book_id, Some(chapter), None, None)
    }

    pub fn annotation_count_for_range(
        &self,
        book_id: &str,
        chapter: u32,
        start: u32,
        end: u32,
    ) -> Result<u64> {
        annotation_input_id(book_id, 4096)?;
        ensure!(start <= end, "annotation query range is reversed");
        if start == end {
            return Ok(0);
        }
        annotation_count_on(
            &self.connection,
            book_id,
            Some(chapter),
            Some(start),
            Some(end),
        )
    }

    /// Trim-sensitive counts need actual text. Admit the complete field work
    /// first, then inspect borrowed UTF-8 rows without retaining their payloads.
    pub fn annotation_summary_counts(&self, book_id: &str) -> Result<(u64, u64)> {
        annotation_input_id(book_id, 4096)?;
        let limits = AnnotationLimits::headers();
        let transaction = self.connection.unchecked_transaction()?;
        let (count, bytes, cells, quote_bytes, note_bytes, valid): (i64, i64, i64, i64, i64, bool) = transaction.query_row(
            "SELECT COUNT(*),COALESCE(SUM(COALESCE(octet_length(quote),0)+COALESCE(octet_length(note),0)),0),
                COALESCE(SUM((COALESCE(octet_length(quote),0)>0)+(COALESCE(octet_length(note),0)>0)),0),
                COALESCE(MAX(COALESCE(octet_length(quote),0)),0),COALESCE(MAX(COALESCE(octet_length(note),0)),0),
                COALESCE(MIN(typeof(quote)='text' AND typeof(note)='text'),1) FROM annotations WHERE book_id=?1",
            [book_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )?;
        if !valid {
            return Err(AnnotationUnavailable.into());
        }
        let count = usize::try_from(count).map_err(|_| AnnotationUnavailable)?;
        annotation_admit("rows", count, limits.rows)?;
        annotation_admit(
            "SQL cells",
            annotation_sum(6, annotation_product(count, 2)?)?,
            limits.cells,
        )?;
        annotation_admit(
            "quote bytes",
            self.text_encoding
                .utf8_upper(usize::try_from(quote_bytes).map_err(|_| AnnotationUnavailable)?)?,
            limits.quote_bytes,
        )?;
        annotation_admit(
            "note bytes",
            self.text_encoding
                .utf8_upper(usize::try_from(note_bytes).map_err(|_| AnnotationUnavailable)?)?,
            limits.note_bytes,
        )?;
        let bytes = self.text_encoding.utf8_upper_total(
            usize::try_from(bytes).map_err(|_| AnnotationUnavailable)?,
            usize::try_from(cells).map_err(|_| AnnotationUnavailable)?,
        )?;
        annotation_admit("working-set bytes", bytes, limits.bytes)?;
        let mut highlights = 0_u64;
        let mut thoughts = 0_u64;
        {
            let mut statement =
                transaction.prepare("SELECT quote,note FROM annotations WHERE book_id=?1")?;
            let mut rows = statement.query([book_id])?;
            while let Some(row) = rows.next()? {
                let quote = annotation_text(row, 0, limits.quote_bytes)?;
                let note = annotation_text(row, 1, limits.note_bytes)?;
                if !quote.trim().is_empty() {
                    highlights = highlights.checked_add(1).ok_or(AnnotationUnavailable)?;
                }
                if !note.trim().is_empty() {
                    thoughts = thoughts.checked_add(1).ok_or(AnnotationUnavailable)?;
                }
            }
        }
        transaction.commit()?;
        Ok((highlights, thoughts))
    }

    pub fn annotation_header_by_id(
        &self,
        book_id: &str,
        id: &str,
    ) -> Result<Option<AnnotationHeader>> {
        self.annotation_header_by_id_with_limits(book_id, id, AnnotationLimits::headers())
    }

    pub fn annotation_header_by_id_with_limits(
        &self,
        book_id: &str,
        id: &str,
        limits: AnnotationLimits,
    ) -> Result<Option<AnnotationHeader>> {
        #[cfg(test)]
        let _meter = self.annotation_meter();
        annotation_input_id(book_id, 4096)?;
        annotation_input_id(id, limits.id_bytes)?;
        let transaction = self.connection.unchecked_transaction()?;
        let headers = annotation_headers_on(
            &transaction,
            self.text_encoding,
            AnnotationFilter {
                book_id,
                chapter: None,
                start: None,
                end: None,
                id: Some(id),
                exact: false,
            },
            0,
            1,
            false,
            limits,
        )?;
        transaction.commit()?;
        Ok(headers.into_iter().next())
    }

    pub fn annotation_headers_for_chapter(
        &self,
        book_id: &str,
        chapter: u32,
    ) -> Result<Vec<AnnotationHeader>> {
        self.annotation_headers_for_chapter_with_limits(
            book_id,
            chapter,
            AnnotationLimits::headers(),
        )
    }

    pub fn annotation_headers_for_chapter_with_limits(
        &self,
        book_id: &str,
        chapter: u32,
        limits: AnnotationLimits,
    ) -> Result<Vec<AnnotationHeader>> {
        #[cfg(test)]
        let _meter = self.annotation_meter();
        annotation_input_id(book_id, 4096)?;
        let transaction = self.connection.unchecked_transaction()?;
        let headers = annotation_headers_on(
            &transaction,
            self.text_encoding,
            AnnotationFilter {
                book_id,
                chapter: Some(chapter),
                start: None,
                end: None,
                id: None,
                exact: false,
            },
            0,
            limits.rows,
            true,
            limits,
        )?;
        transaction.commit()?;
        Ok(headers)
    }

    pub fn annotation_headers_for_range(
        &self,
        book_id: &str,
        chapter: u32,
        start: u32,
        end: u32,
    ) -> Result<Vec<AnnotationHeader>> {
        self.annotation_headers_for_range_with_limits(
            book_id,
            chapter,
            start,
            end,
            AnnotationLimits::headers(),
        )
    }

    pub fn annotation_headers_for_range_with_limits(
        &self,
        book_id: &str,
        chapter: u32,
        start: u32,
        end: u32,
        limits: AnnotationLimits,
    ) -> Result<Vec<AnnotationHeader>> {
        #[cfg(test)]
        let _meter = self.annotation_meter();
        annotation_input_id(book_id, 4096)?;
        ensure!(start <= end, "annotation query range is reversed");
        if start == end {
            return Ok(Vec::new());
        }
        let transaction = self.connection.unchecked_transaction()?;
        let headers = annotation_headers_on(
            &transaction,
            self.text_encoding,
            AnnotationFilter {
                book_id,
                chapter: Some(chapter),
                start: Some(start),
                end: Some(end),
                id: None,
                exact: false,
            },
            0,
            limits.rows,
            true,
            limits,
        )?;
        transaction.commit()?;
        Ok(headers)
    }

    pub fn annotation_headers_page(
        &self,
        book_id: &str,
        offset: usize,
        rows: usize,
    ) -> Result<AnnotationPage> {
        self.annotation_headers_page_with_limits(book_id, offset, rows, AnnotationLimits::headers())
    }

    pub fn annotation_headers_page_with_limits(
        &self,
        book_id: &str,
        offset: usize,
        rows: usize,
        limits: AnnotationLimits,
    ) -> Result<AnnotationPage> {
        #[cfg(test)]
        let _meter = self.annotation_meter();
        annotation_input_id(book_id, 4096)?;
        annotation_admit("rows", rows, limits.rows)?;
        let transaction = self.connection.unchecked_transaction()?;
        let total = annotation_count_on(&transaction, book_id, None, None, None)?;
        let headers = annotation_headers_on(
            &transaction,
            self.text_encoding,
            AnnotationFilter {
                book_id,
                chapter: None,
                start: None,
                end: None,
                id: None,
                exact: false,
            },
            offset,
            rows,
            false,
            limits,
        )?;
        transaction.commit()?;
        Ok(AnnotationPage { total, headers })
    }

    pub fn annotation_records(
        &self,
        book_id: &str,
        keys: &[AnnotationKey],
    ) -> Result<Vec<Annotation>> {
        self.annotation_records_with_limits(book_id, keys, AnnotationLimits::records())
    }

    pub fn annotation_records_with_limits(
        &self,
        book_id: &str,
        keys: &[AnnotationKey],
        limits: AnnotationLimits,
    ) -> Result<Vec<Annotation>> {
        #[cfg(test)]
        let _meter = self.annotation_meter();
        annotation_input_id(book_id, 4096)?;
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        annotation_admit("rows", keys.len(), limits.rows)?;
        annotation_admit(
            "SQL cells",
            annotation_product(keys.len(), 14)?,
            limits.cells,
        )?;
        annotation_admit(
            "working-set bytes",
            annotation_record_fixed(keys.len())?,
            limits.bytes,
        )?;
        let mut unique = HashSet::new();
        for key in keys {
            annotation_input_id(&key.id, limits.id_bytes)?;
            ensure!(
                unique.insert(key.rowid),
                "annotation selection contains duplicate rowids"
            );
        }
        let transaction = self.connection.unchecked_transaction()?;
        let records =
            annotation_records_on(&transaction, self.text_encoding, book_id, keys, limits)?;
        transaction.commit()?;
        Ok(records)
    }

    pub fn annotation_by_id(&self, book_id: &str, id: &str) -> Result<Option<Annotation>> {
        self.annotation_by_id_with_limits(book_id, id, AnnotationLimits::records())
    }

    pub fn annotation_by_id_with_limits(
        &self,
        book_id: &str,
        id: &str,
        limits: AnnotationLimits,
    ) -> Result<Option<Annotation>> {
        #[cfg(test)]
        let _meter = self.annotation_meter();
        annotation_input_id(book_id, 4096)?;
        annotation_input_id(id, limits.id_bytes)?;
        let transaction = self.connection.unchecked_transaction()?;
        let headers = annotation_headers_on(
            &transaction,
            self.text_encoding,
            AnnotationFilter {
                book_id,
                chapter: None,
                start: None,
                end: None,
                id: Some(id),
                exact: false,
            },
            0,
            1,
            false,
            limits,
        )?;
        let keys = headers
            .into_iter()
            .map(|header| header.key)
            .collect::<Vec<_>>();
        let records =
            annotation_records_on(&transaction, self.text_encoding, book_id, &keys, limits)?;
        transaction.commit()?;
        Ok(records.into_iter().next())
    }

    pub fn annotation_geometry_for_chapter(
        &self,
        book_id: &str,
        chapter: u32,
    ) -> Result<Vec<AnnotationGeometry>> {
        self.annotation_geometry_for_chapter_with_limits(
            book_id,
            chapter,
            AnnotationLimits::headers(),
        )
    }

    pub fn annotation_header_for_exact_range(
        &self,
        book_id: &str,
        chapter: u32,
        start: u32,
        end: u32,
    ) -> Result<Option<AnnotationHeader>> {
        #[cfg(test)]
        let _meter = self.annotation_meter();
        annotation_input_id(book_id, 4096)?;
        ensure!(start < end, "annotation exact range is empty or reversed");
        let transaction = self.connection.unchecked_transaction()?;
        let headers = annotation_headers_on(
            &transaction,
            self.text_encoding,
            AnnotationFilter {
                book_id,
                chapter: Some(chapter),
                start: Some(start),
                end: Some(end),
                id: None,
                exact: true,
            },
            0,
            1,
            false,
            AnnotationLimits::headers(),
        )?;
        transaction.commit()?;
        Ok(headers.into_iter().next())
    }

    pub fn annotation_geometry_for_chapter_with_limits(
        &self,
        book_id: &str,
        chapter: u32,
        limits: AnnotationLimits,
    ) -> Result<Vec<AnnotationGeometry>> {
        annotation_input_id(book_id, 4096)?;
        let transaction = self.connection.unchecked_transaction()?;
        let geometry = annotation_geometry_on(&transaction, book_id, chapter, None, None, limits)?;
        transaction.commit()?;
        Ok(geometry)
    }

    pub fn annotation_geometry_for_range(
        &self,
        book_id: &str,
        chapter: u32,
        start: u32,
        end: u32,
    ) -> Result<Vec<AnnotationGeometry>> {
        self.annotation_geometry_for_range_with_limits(
            book_id,
            chapter,
            start,
            end,
            AnnotationLimits::headers(),
        )
    }

    pub fn annotation_geometry_for_range_with_limits(
        &self,
        book_id: &str,
        chapter: u32,
        start: u32,
        end: u32,
        limits: AnnotationLimits,
    ) -> Result<Vec<AnnotationGeometry>> {
        annotation_input_id(book_id, 4096)?;
        ensure!(start <= end, "annotation query range is reversed");
        if start == end {
            return Ok(Vec::new());
        }
        let transaction = self.connection.unchecked_transaction()?;
        let geometry = annotation_geometry_on(
            &transaction,
            book_id,
            chapter,
            Some(start),
            Some(end),
            limits,
        )?;
        transaction.commit()?;
        Ok(geometry)
    }

    pub fn annotation_header_for_geometry(
        &self,
        book_id: &str,
        geometry: &AnnotationGeometry,
    ) -> Result<Option<AnnotationHeader>> {
        #[cfg(test)]
        let _meter = self.annotation_meter();
        annotation_input_id(book_id, 4096)?;
        let transaction = self.connection.unchecked_transaction()?;
        let limits = AnnotationLimits::headers();
        let metadata = {
            let mut statement = transaction.prepare(
                "SELECT rowid,CASE WHEN typeof(chapter_index)='integer' THEN chapter_index END,CASE WHEN typeof(start_offset)='integer' THEN start_offset END,CASE WHEN typeof(end_offset)='integer' THEN end_offset END,CASE WHEN typeof(created_at)='integer' THEN created_at END,typeof(id)='text' AND typeof(book_id)='text',
                    COALESCE(octet_length(id),0),COALESCE(octet_length(book_id),0) FROM annotations WHERE rowid=?1",
            )?;
            let mut rows = statement.query([geometry.rowid])?;
            let Some(row) = rows.next()? else {
                return Ok(None);
            };
            if !row.get::<_, bool>(5)? {
                return Err(AnnotationUnavailable.into());
            }
            annotation_admit(
                "identifier bytes",
                annotation_encoded(row, 6, self.text_encoding)?,
                limits.id_bytes,
            )?;
            annotation_admit(
                "book identifier bytes",
                annotation_encoded(row, 7, self.text_encoding)?,
                4096,
            )?;
            annotation_metadata(row)?
        };
        if metadata.chapter_index != geometry.chapter_index
            || metadata.start != geometry.start
            || metadata.end != geometry.end
            || metadata.created_at != geometry.created_at
        {
            return Err(AnnotationUnavailable.into());
        }
        let header = {
            let mut statement =
                transaction.prepare("SELECT id,book_id FROM annotations WHERE rowid=?1")?;
            let mut rows = statement.query([geometry.rowid])?;
            let row = rows.next()?.ok_or(AnnotationUnavailable)?;
            let id = annotation_text(row, 0, limits.id_bytes)?;
            if annotation_text(row, 1, 4096)? != book_id {
                return Err(AnnotationUnavailable.into());
            }
            #[cfg(test)]
            annotation_copied(Some(id.len()), None, None);
            AnnotationHeader {
                key: AnnotationKey {
                    rowid: geometry.rowid,
                    id: id.to_owned(),
                },
                chapter_index: metadata.chapter_index,
                start: metadata.start,
                end: metadata.end,
                created_at: metadata.created_at,
            }
        };
        transaction.commit()?;
        Ok(Some(header))
    }

    pub fn annotation_has_quote_for_range(
        &self,
        book_id: &str,
        chapter: u32,
        start: u32,
        end: u32,
    ) -> Result<bool> {
        annotation_input_id(book_id, 4096)?;
        ensure!(start <= end, "annotation query range is reversed");
        if start == end {
            return Ok(false);
        }
        let (invalid, present): (bool, bool) = self.connection.query_row(
            "SELECT COALESCE(MAX(typeof(quote)<>'text'),0),
                COALESCE(MAX(typeof(quote)='text' AND COALESCE(octet_length(quote),0)>0),0)
                FROM annotations WHERE book_id=?1 AND chapter_index=?2 AND start_offset<?4 AND end_offset>?3",
            params![book_id, chapter, start, end], |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if invalid {
            return Err(AnnotationUnavailable.into());
        }
        Ok(present)
    }

    pub fn annotation_text_metadata(
        &self,
        book_id: &str,
        key: &AnnotationKey,
    ) -> Result<Option<AnnotationTextMetadata>> {
        self.annotation_text_metadata_with_limits(book_id, key, AnnotationLimits::records())
    }

    pub fn annotation_text_metadata_with_limits(
        &self,
        book_id: &str,
        key: &AnnotationKey,
        limits: AnnotationLimits,
    ) -> Result<Option<AnnotationTextMetadata>> {
        #[cfg(test)]
        let _meter = self.annotation_meter();
        annotation_admit("SQL cells", 14, limits.cells)?;
        annotation_input_id(book_id, 4096)?;
        annotation_input_id(&key.id, limits.id_bytes)?;
        let transaction = self.connection.unchecked_transaction()?;
        let Some(header) =
            annotation_header_key_on(&transaction, self.text_encoding, book_id, key, limits)?
        else {
            return Ok(None);
        };
        let value = transaction.query_row(
            "SELECT typeof(quote)='text',COALESCE(octet_length(quote),0),typeof(note)='text',COALESCE(octet_length(note),0)
                FROM annotations WHERE rowid=?1",
            [key.rowid], |row| Ok((row.get::<_, bool>(0)?, row.get::<_, i64>(1)?, row.get::<_, bool>(2)?, row.get::<_, i64>(3)?)),
        )?;
        let quote =
            annotation_field_state(self.text_encoding, value.0, value.1, limits.quote_bytes)?;
        let note = annotation_field_state(self.text_encoding, value.2, value.3, limits.note_bytes)?;
        transaction.commit()?;
        Ok(Some(AnnotationTextMetadata {
            header,
            quote,
            note,
        }))
    }

    pub fn annotation_quote_by_key(
        &self,
        book_id: &str,
        key: &AnnotationKey,
        maximum_bytes: usize,
    ) -> Result<Option<String>> {
        self.annotation_field_by_key(book_id, key, maximum_bytes, false)
    }

    pub fn annotation_note_by_key(
        &self,
        book_id: &str,
        key: &AnnotationKey,
        maximum_bytes: usize,
    ) -> Result<Option<String>> {
        self.annotation_field_by_key(book_id, key, maximum_bytes, true)
    }

    fn annotation_field_by_key(
        &self,
        book_id: &str,
        key: &AnnotationKey,
        maximum_bytes: usize,
        note: bool,
    ) -> Result<Option<String>> {
        #[cfg(test)]
        let _meter = self.annotation_meter();
        annotation_input_id(book_id, 4096)?;
        annotation_input_id(&key.id, 4096)?;
        let transaction = self.connection.unchecked_transaction()?;
        if annotation_header_key_on(
            &transaction,
            self.text_encoding,
            book_id,
            key,
            AnnotationLimits::headers(),
        )?
        .is_none()
        {
            return Ok(None);
        }
        let text = annotation_field_on(
            &transaction,
            self.text_encoding,
            key.rowid,
            maximum_bytes,
            note,
        )?;
        transaction.commit()?;
        Ok(Some(text))
    }

    /// Keep a preexisting private note in SQL when a same-range highlight is
    /// saved. SQLite may read/rewrite its row, but Rust never loads that note.
    pub fn update_annotation_quote_preserving_note(
        &self,
        book_id: &str,
        header: &AnnotationHeader,
        quote: &str,
        created_at: i64,
    ) -> Result<bool> {
        self.update_annotation_field_preserving_other(book_id, header, quote, created_at, false)
    }

    /// Symmetric same-range edit. The existing quote remains in SQL, including
    /// oversized quotes that the current readonly renderer cannot materialize.
    pub fn update_annotation_note_preserving_quote(
        &self,
        book_id: &str,
        header: &AnnotationHeader,
        note: &str,
        created_at: i64,
    ) -> Result<bool> {
        self.update_annotation_field_preserving_other(book_id, header, note, created_at, true)
    }

    fn update_annotation_field_preserving_other(
        &self,
        book_id: &str,
        header: &AnnotationHeader,
        value: &str,
        created_at: i64,
        note: bool,
    ) -> Result<bool> {
        #[cfg(test)]
        let _meter = self.annotation_meter();
        annotation_input_id(book_id, 4096)?;
        annotation_input_id(&header.key.id, 4096)?;
        annotation_admit(
            if note { "note bytes" } else { "quote bytes" },
            value.len(),
            if note {
                AnnotationLimits::records().note_bytes
            } else {
                AnnotationLimits::records().quote_bytes
            },
        )?;
        let transaction = self.connection.unchecked_transaction()?;
        let Some(current) = annotation_header_key_on(
            &transaction,
            self.text_encoding,
            book_id,
            &header.key,
            AnnotationLimits::headers(),
        )?
        else {
            return Ok(false);
        };
        if current != *header {
            return Ok(false);
        }
        let (bytes, cells): (i64, i64) = transaction.query_row(
            "SELECT COALESCE(octet_length(id),0)+COALESCE(octet_length(book_id),0)+COALESCE(octet_length(quote),0)+COALESCE(octet_length(note),0),
                (COALESCE(octet_length(id),0)>0)+(COALESCE(octet_length(book_id),0)>0)+(COALESCE(octet_length(quote),0)>0)+(COALESCE(octet_length(note),0)>0)
                FROM annotations WHERE rowid=?1", [header.key.rowid], |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let bytes = self.text_encoding.utf8_upper_total(
            usize::try_from(bytes).map_err(|_| AnnotationUnavailable)?,
            usize::try_from(cells).map_err(|_| AnnotationUnavailable)?,
        )?;
        let work = annotation_product(annotation_sum(bytes, value.len())?, 2)?;
        #[cfg(test)]
        let maximum_work = annotation_budget_tests::update_work_limit().unwrap_or(64 * 1024 * 1024);
        #[cfg(not(test))]
        let maximum_work = 64 * 1024 * 1024;
        annotation_admit("update work bytes", work, maximum_work)?;
        let sql = if note {
            "UPDATE annotations SET note=?7,created_at=?8 WHERE rowid=?1 AND id=?2 AND book_id=?3
                AND chapter_index=?4 AND start_offset=?5 AND end_offset=?6 AND created_at=?9"
        } else {
            "UPDATE annotations SET quote=?7,created_at=?8 WHERE rowid=?1 AND id=?2 AND book_id=?3
                AND chapter_index=?4 AND start_offset=?5 AND end_offset=?6 AND created_at=?9"
        };
        let changed = transaction.execute(
            sql,
            params![
                header.key.rowid,
                header.key.id,
                book_id,
                header.chapter_index,
                header.start,
                header.end,
                value,
                created_at,
                header.created_at
            ],
        )?;
        transaction.commit()?;
        Ok(changed == 1)
    }

    /// Stream grouped integer timestamps under one snapshot, with no note/quote
    /// projection. The caller retains its existing timezone/date semantics.
    pub fn for_each_annotation_timestamp_count(
        &self,
        mut visit: impl FnMut(i64, u64) -> Result<()>,
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        let (count, valid): (i64, bool) = transaction.query_row(
            "SELECT COUNT(*),COALESCE(MIN(typeof(a.created_at)='integer'),1) FROM annotations AS a NOT INDEXED WHERE EXISTS(SELECT 1 FROM books WHERE id=a.book_id)", [], |row| Ok((row.get(0)?,row.get(1)?)),
        )?;
        if !valid {
            return Err(AnnotationUnavailable.into());
        }
        let count = usize::try_from(count).map_err(|_| AnnotationUnavailable)?;
        let limits = store_inventory_limits();
        annotation_admit("timestamp rows", count, limits.rows)?;
        let sort_bytes = count.checked_mul(128).ok_or(AnnotationUnavailable)?;
        annotation_admit(
            "timestamp working-set bytes",
            sort_bytes,
            limits.metadata_bytes.min(limits.scratch_bytes),
        )?;
        let levels = usize::try_from(usize::BITS - count.max(1).leading_zeros())
            .map_err(|_| AnnotationUnavailable)?;
        let sort_work = count
            .checked_mul(levels)
            .and_then(|n| n.checked_mul(64))
            .ok_or(AnnotationUnavailable)?;
        annotation_admit("timestamp ordering work", sort_work, limits.work_bytes)?;
        {
            let mut statement = transaction.prepare(
                "SELECT CASE WHEN typeof(created_at)='integer' THEN created_at END,COUNT(*) FROM annotations AS a NOT INDEXED
                    WHERE EXISTS(SELECT 1 FROM books WHERE id=a.book_id) GROUP BY created_at ORDER BY created_at",
            )?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let timestamp: i64 = row.get(0).map_err(|_| AnnotationUnavailable)?;
                let count =
                    u64::try_from(row.get::<_, i64>(1)?).map_err(|_| AnnotationUnavailable)?;
                visit(timestamp, count)?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn delete_annotation(&self, id: &str) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM annotations WHERE id = ?1", [id])?
            > 0)
    }

    pub fn save_bookmark(&self, bookmark: &Bookmark) -> Result<()> {
        validate_identifier(&bookmark.id)?;
        self.connection.execute("INSERT INTO bookmarks (id, book_id, chapter_index, char_offset, label, created_at) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET book_id=excluded.book_id, chapter_index=excluded.chapter_index, char_offset=excluded.char_offset, label=excluded.label, created_at=excluded.created_at", params![bookmark.id, bookmark.book_id, bookmark.chapter_index, bookmark.char_offset, bookmark.label, bookmark.created_at])?;
        Ok(())
    }

    #[cfg(test)]
    pub fn bookmarks(&self, book_id: &str) -> Result<Vec<Bookmark>> {
        let mut statement = self.connection.prepare(
            "SELECT * FROM bookmarks WHERE book_id = ?1 ORDER BY chapter_index, char_offset",
        )?;
        let rows = statement.query_map([book_id], |row| {
            Ok(Bookmark {
                id: row.get("id")?,
                book_id: row.get("book_id")?,
                chapter_index: row.get("chapter_index")?,
                char_offset: row.get("char_offset")?,
                label: row.get("label")?,
                created_at: row.get("created_at")?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn bookmark_count(&self, book: &str) -> Result<u64> {
        bookmark_input_id(book)?;
        Ok(u64::try_from(self.connection.query_row(
            "SELECT COUNT(*) FROM bookmarks WHERE book_id=?1",
            [book],
            |r| r.get::<_, i64>(0),
        )?)
        .map_err(|_| BookmarkUnavailable)?)
    }
    pub fn bookmark_geometry_for_range(
        &self,
        book: &str,
        chapter: u32,
        start: u32,
        end: u32,
    ) -> Result<Vec<BookmarkGeometry>> {
        bookmark_input_id(book)?;
        ensure!(start <= end, "bookmark query range is reversed");
        let txn = self.connection.unchecked_transaction()?;
        let values = bookmark_geometries_on(
            &txn,
            book,
            Some((chapter, start, end)),
            0,
            bookmark_limits().rows,
        )?;
        txn.commit()?;
        Ok(values)
    }
    pub fn bookmark_geometry_page(
        &self,
        book: &str,
        offset: usize,
        rows: usize,
    ) -> Result<BookmarkGeometryPage> {
        bookmark_input_id(book)?;
        let txn = self.connection.unchecked_transaction()?;
        let total = u64::try_from(txn.query_row(
            "SELECT COUNT(*) FROM bookmarks WHERE book_id=?1",
            [book],
            |r| r.get::<_, i64>(0),
        )?)
        .map_err(|_| BookmarkUnavailable)?;
        let geometries = bookmark_geometries_on(&txn, book, None, offset, rows)?;
        txn.commit()?;
        Ok(BookmarkGeometryPage { total, geometries })
    }

    /// Resolve the existing canonical entry protocol without loading other
    /// bookmark IDs or labels. The selected result contains only geometry.
    pub fn bookmark_geometry_by_id(
        &self,
        book: &str,
        id: &str,
    ) -> Result<Option<BookmarkGeometry>> {
        bookmark_input_id(book)?;
        bookmark_input_id(id)?;
        let limits = bookmark_limits();
        bookmark_admit(
            "geometry metadata",
            bookmark_sum(&[
                book.len(),
                id.len(),
                2 * std::mem::size_of::<BookmarkGeometry>(),
            ])?,
            limits.metadata_bytes,
        )?;
        let transaction = self.connection.unchecked_transaction()?;
        let geometry = {
            let mut statement=transaction.prepare("SELECT rowid,
                CASE WHEN typeof(chapter_index)='integer' THEN chapter_index END,
                CASE WHEN typeof(char_offset)='integer' THEN char_offset END,
                CASE WHEN typeof(created_at)='integer' THEN created_at END FROM bookmarks WHERE book_id=?1 AND id=?2")?;
            let mut rows = statement.query(params![book, id])?;
            rows.next()?
                .map(|row| -> Result<BookmarkGeometry> {
                    Ok(BookmarkGeometry {
                        rowid: row.get(0).map_err(|_| BookmarkUnavailable)?,
                        chapter_index: row.get(1).map_err(|_| BookmarkUnavailable)?,
                        char_offset: row.get(2).map_err(|_| BookmarkUnavailable)?,
                        created_at: row.get(3).map_err(|_| BookmarkUnavailable)?,
                    })
                })
                .transpose()?
        };
        transaction.commit()?;
        Ok(geometry)
    }

    pub fn bookmark_header_for_geometry(
        &self,
        book: &str,
        g: &BookmarkGeometry,
    ) -> Result<Option<BookmarkHeader>> {
        bookmark_input_id(book)?;
        let txn = self.connection.unchecked_transaction()?;
        let l = bookmark_limits();
        let metadata:Option<(bool,i64)>=txn.query_row("SELECT typeof(id)='text',COALESCE(octet_length(id),0) FROM bookmarks
            WHERE rowid=?1 AND book_id=?2 AND chapter_index=?3 AND char_offset=?4 AND created_at=?5",
            params![g.rowid,book,g.chapter_index,g.char_offset,g.created_at],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((text, raw)) = metadata else {
            txn.commit()?;
            return Ok(None);
        };
        if !text {
            return Err(BookmarkUnavailable.into());
        }
        let raw = usize::try_from(raw).map_err(|_| BookmarkUnavailable)?;
        let encoded_limit = match self.text_encoding {
            TextEncoding::Utf8 => l.id_bytes,
            TextEncoding::Utf16 => l.id_bytes.checked_mul(2).ok_or(BookmarkResourceLimit {
                resource: "id bytes",
            })?,
        };
        bookmark_admit("id encoded bytes", raw, encoded_limit)?;
        let upper = self
            .text_encoding
            .utf8_upper(raw)
            .map_err(|_| BookmarkUnavailable)?;
        let vm = bookmark_vm_scratch(self.text_encoding, raw)?;
        bookmark_admit(
            "header metadata",
            bookmark_sum(&[
                upper,
                vm,
                book.len(),
                2 * std::mem::size_of::<BookmarkHeader>(),
            ])?,
            l.metadata_bytes,
        )?;
        let id = {
            let mut stmt = txn.prepare("SELECT id FROM bookmarks WHERE rowid=?1")?;
            let mut rows = stmt.query([g.rowid])?;
            let row = rows.next()?.ok_or(BookmarkUnavailable)?;
            let id = row.get_ref(0)?.as_str().map_err(|_| BookmarkUnavailable)?;
            bookmark_admit("id bytes", id.len(), l.id_bytes)?;
            validate_identifier(id).map_err(|_| BookmarkUnavailable)?;
            #[cfg(test)]
            bookmark_budget_tests::observe_id(id.len());
            id.to_owned()
        };
        txn.commit()?;
        Ok(Some(BookmarkHeader {
            key: BookmarkKey { rowid: g.rowid, id },
            chapter_index: g.chapter_index,
            char_offset: g.char_offset,
            created_at: g.created_at,
        }))
    }
    pub fn bounded_bookmark_label(
        &self,
        book: &str,
        h: &BookmarkHeader,
        maxbytes: usize,
    ) -> Result<Option<String>> {
        bookmark_input_id(book)?;
        let txn = self.connection.unchecked_transaction()?;
        let l = bookmark_limits();
        let meta = bookmark_label_metadata_on(&txn, self.text_encoding, book, h)?;
        let Some(meta) = meta else {
            txn.commit()?;
            return Ok(None);
        };
        bookmark_admit("label bytes", meta.upper, maxbytes.min(l.label_bytes))?;
        bookmark_admit(
            "label metadata",
            bookmark_sum(&[
                meta.upper,
                meta.vm,
                book.len(),
                h.key.id.len(),
                2 * std::mem::size_of::<String>(),
            ])?,
            l.metadata_bytes,
        )?;
        let label = bookmark_label_on(&txn, book, h, meta.upper)?;
        txn.commit()?;
        Ok(Some(label))
    }
    pub fn bookmark_records(
        &self,
        book: &str,
        headers: &[BookmarkHeader],
    ) -> Result<Vec<Bookmark>> {
        bookmark_input_id(book)?;
        let l = bookmark_limits();
        bookmark_admit("record rows", headers.len(), 16.min(l.rows))?;
        let fixed = headers
            .len()
            .checked_mul(
                2 * std::mem::size_of::<Bookmark>()
                    + 2 * std::mem::size_of::<(&BookmarkHeader, BookmarkTextPrice)>()
                    + 64,
            )
            .ok_or(BookmarkResourceLimit {
                resource: "record metadata",
            })?;
        bookmark_admit("record metadata", fixed, l.metadata_bytes)?;
        let txn = self.connection.unchecked_transaction()?;
        let mut admitted = Vec::new();
        let mut bytes = fixed;
        let mut maximum_vm = 0usize;
        let mut seen = HashSet::with_capacity(headers.len());
        for h in headers {
            if !seen.insert(h.key.rowid) {
                return Err(BookmarkUnavailable.into());
            }
            bookmark_admit("id bytes", h.key.id.len(), l.id_bytes)?;
            let meta = bookmark_label_metadata_on(&txn, self.text_encoding, book, h)?
                .ok_or(BookmarkUnavailable)?;
            bookmark_admit("label bytes", meta.upper, l.label_bytes)?;
            maximum_vm = maximum_vm.max(meta.vm);
            bytes = bookmark_sum(&[bytes, meta.upper, h.key.id.len(), book.len()])?;
            bookmark_admit(
                "record metadata",
                bookmark_sum(&[bytes, maximum_vm])?,
                l.metadata_bytes,
            )?;
            admitted.push((h, meta.upper));
        }
        let mut values = Vec::with_capacity(admitted.len());
        for (h, upper) in admitted {
            let label = bookmark_label_on(&txn, book, h, upper)?;
            #[cfg(test)]
            bookmark_budget_tests::observe_id(h.key.id.len());
            values.push(Bookmark {
                id: h.key.id.clone(),
                book_id: book.to_owned(),
                chapter_index: h.chapter_index,
                char_offset: h.char_offset,
                label,
                created_at: h.created_at,
            });
        }
        txn.commit()?;
        Ok(values)
    }

    pub fn delete_bookmark(&self, id: &str) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM bookmarks WHERE id = ?1", [id])?
            > 0)
    }

    pub fn save_task(&self, task: &DownloadTask) -> Result<()> {
        let (bytes, work) = task_borrowed_price(task)?;
        let limits = store_inventory_limits();
        inventory_admit("task input metadata", bytes, limits.metadata_bytes)?;
        inventory_admit("task input scratch", bytes, limits.scratch_bytes)?;
        inventory_admit("task input work", work, limits.work_bytes)?;
        if self.connection.is_autocommit() {
            let transaction = self.connection.unchecked_transaction()?;
            self.validate_task(task)?;
            persist_task_on(&transaction, task)?;
            transaction.commit()?;
            Ok(())
        } else {
            self.validate_task(task)?;
            persist_task_on(&self.connection, task)
        }
    }

    /// Save a lifecycle transition only while its durable task still matches the
    /// worker snapshot. In particular, a late failure after a rejected checkpoint
    /// must not overwrite a newer account, selection, cancellation, or cursor.
    pub fn save_task_if_current(
        &self,
        previous: &DownloadTask,
        task: &DownloadTask,
    ) -> Result<bool> {
        let (old_bytes, old_work) = task_borrowed_price(previous)?;
        let (new_bytes, new_work) = task_borrowed_price(task)?;
        let held = inventory_sum(&[old_bytes, new_bytes])?;
        let work = inventory_sum(&[old_work, new_work])?;
        let limits = store_inventory_limits();
        inventory_admit("task transition metadata", held, limits.metadata_bytes)?;
        inventory_admit("task transition scratch", held, limits.scratch_bytes)?;
        inventory_admit("task transition work", work, limits.work_bytes)?;
        ensure!(
            previous.id == task.id
                && previous.book_id == task.book_id
                && previous.account_key == task.account_key
                && previous.chapters == task.chapters
                && previous.include_images == task.include_images
                && previous.next_index == task.next_index
                && previous.completed_count == task.completed_count,
            "a guarded task transition cannot change its scope or cursor"
        );
        let transaction = self.connection.unchecked_transaction()?;
        let scalar: Option<(i64, bool, i64, i64, Option<i64>)> = transaction.query_row(
            "SELECT rowid,typeof(book_id)='text' AND typeof(status)='text' AND typeof(updated_at)='integer',COALESCE(octet_length(book_id),0),COALESCE(octet_length(status),0),CASE WHEN typeof(updated_at)='integer' THEN updated_at END FROM download_tasks WHERE id=?1",
            [&previous.id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ).optional()?;
        let Some((rowid, valid, book_bytes, status_bytes, updated_at)) = scalar else {
            return Ok(false);
        };
        if !valid {
            return Err(StoreInventoryUnavailable.into());
        }
        let book_raw = usize::try_from(book_bytes).map_err(|_| StoreInventoryUnavailable)?;
        let status_raw = usize::try_from(status_bytes).map_err(|_| StoreInventoryUnavailable)?;
        let book_bytes = self.text_encoding.utf8_upper(book_raw)?;
        let status_bytes = self.text_encoding.utf8_upper(status_raw)?;
        inventory_admit("task book bytes", book_bytes, 4096)?;
        inventory_admit("task status bytes", status_bytes, 32)?;
        inventory_admit(
            "task transition scalar scratch",
            inventory_sum(&[
                held,
                inventory_text_vm_scratch(self.text_encoding, book_raw)?,
                inventory_text_vm_scratch(self.text_encoding, status_raw)?,
            ])?,
            limits.scratch_bytes,
        )?;
        let work = inventory_sum(&[work, book_bytes, status_bytes])?;
        inventory_admit("task transition scalar work", work, limits.work_bytes)?;
        let scalar_matches = {
            let mut statement =
                transaction.prepare("SELECT book_id,status FROM download_tasks WHERE rowid=?1")?;
            let mut rows = statement.query([rowid])?;
            let row = rows.next()?.ok_or(StoreInventoryUnavailable)?;
            let book = row
                .get_ref(0)?
                .as_str()
                .map_err(|_| StoreInventoryUnavailable)?;
            let status = row
                .get_ref(1)?
                .as_str()
                .map_err(|_| StoreInventoryUnavailable)?;
            book == previous.book_id
                && status == previous.status.as_str()
                && updated_at == Some(previous.updated_at)
        };
        if !scalar_matches {
            return Ok(false);
        }
        let stored = task_inventory_by_id_reserved_on(
            &transaction,
            self.text_encoding,
            &previous.id,
            held,
            work,
        )?
        .ok_or(StoreInventoryUnavailable)?;
        if stored != *previous {
            return Ok(false);
        }
        self.validate_task(task)?;
        persist_task_on(&transaction, task)?;
        transaction.commit()?;
        Ok(true)
    }

    fn validate_task(&self, task: &DownloadTask) -> Result<()> {
        validate_identifier(&task.id)?;
        ensure!(
            !task.chapters.is_empty() || task.status == DownloadStatus::Cancelled,
            "download task has no chapters"
        );
        ensure!(
            task.next_index <= task.chapters.len(),
            "download cursor exceeds task length"
        );
        ensure!(
            task.completed_count <= task.chapters.len(),
            "download completion count exceeds task length"
        );
        ensure!(
            task.chapters.iter().collect::<HashSet<_>>().len() == task.chapters.len(),
            "download task contains duplicate chapters"
        );
        if task.status == DownloadStatus::Complete {
            ensure!(
                task.next_index == task.chapters.len(),
                "incomplete task cannot be marked complete"
            );
            validate_completed_task_on(&self.connection, self.text_encoding, task)?;
            chapter_state::validate_normalized_completion_on(&self.connection, task)?;
        }
        Ok(())
    }

    pub fn task(&self, id: &str) -> Result<Option<DownloadTask>> {
        inventory_admit("task identifier bytes", id.len(), 4096)?;
        validate_identifier(id)?;
        if self.connection.is_autocommit() {
            let transaction = self.connection.unchecked_transaction()?;
            let task = task_inventory_by_id_on(&transaction, self.text_encoding, id)?;
            transaction.commit()?;
            Ok(task)
        } else {
            task_inventory_by_id_on(&self.connection, self.text_encoding, id)
        }
    }

    /// Inspect one recent chapter recovery candidate.
    /// Its identifier is admitted before copying and its payload uses the same
    /// task inventory gate as a selected task; no historical JSON scan occurs.
    pub fn latest_download_notice_candidate(&self) -> Result<Option<DownloadTask>> {
        let transaction = self.connection.unchecked_transaction()?;
        let maximum = self.text_encoding.maximum_encoded(4096)?;
        let selected: Option<Option<String>> = transaction
            .query_row(
                "SELECT CASE WHEN typeof(id)='text' AND COALESCE(octet_length(id),0)<=?1
                        THEN id ELSE NULL END FROM download_tasks AS t
                 WHERE status IN ('pending','running','paused','failed')
                   AND EXISTS(
                     SELECT 1 FROM task_chapter_states AS s WHERE s.task_id=t.id
                       AND s.status IN ('retry_wait','blocked'))
                 ORDER BY updated_at DESC,rowid DESC LIMIT 1",
                [maximum],
                |row| row.get(0),
            )
            .optional()?;
        let result = match selected {
            None => None,
            Some(None) => return Err(StoreInventoryUnavailable.into()),
            Some(Some(id)) => {
                inventory_admit("task notice identifier", id.len(), 4096)?;
                validate_identifier(&id)?;
                task_inventory_by_id_on(&transaction, self.text_encoding, &id)?
            }
        };
        transaction.commit()?;
        Ok(result)
    }

    pub fn tasks(&self) -> Result<Vec<DownloadTask>> {
        self.query_tasks("SELECT payload FROM download_tasks ORDER BY updated_at, rowid")
    }

    pub fn pending_tasks(&self) -> Result<Vec<DownloadTask>> {
        self.query_tasks("SELECT payload FROM download_tasks WHERE status IN ('pending','running','paused') ORDER BY updated_at, rowid")
    }

    pub fn pending_task_count(&self) -> Result<u64> {
        self.scalar("SELECT COUNT(*) FROM download_tasks NOT INDEXED WHERE status IN ('pending','running','paused')")
    }
    pub fn pending_tasks_for_owner(&self, owner: &str) -> Result<Vec<DownloadTask>> {
        inventory_admit("task owner bytes", owner.len(), 16 * 1024)?;
        validate_identifier(owner)?;
        let sql = "SELECT payload FROM download_tasks WHERE status IN ('pending','running','paused') ORDER BY updated_at, rowid";
        if self.connection.is_autocommit() {
            let transaction = self.connection.unchecked_transaction()?;
            let tasks =
                query_task_inventory_on(&transaction, self.text_encoding, sql, Some(owner))?;
            transaction.commit()?;
            Ok(tasks)
        } else {
            query_task_inventory_on(&self.connection, self.text_encoding, sql, Some(owner))
        }
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        validate_identifier(key)?;
        self.connection.execute("INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, serde_json::to_string(value)?])?;
        Ok(())
    }

    /// Lookup delivery membership without owning request/position or compacted
    /// proof. This independent input gate cannot inherit allocation admission.
    /// Complete proof validation remains the reporting consumer's later gate.
    pub(crate) fn report_batch_delivery_state(
        &self,
        outbox_key: &str,
        sid: &str,
    ) -> Result<Option<ReportBatchDeliveryMetadata>> {
        if outbox_key.len() > 4096 || sid.len() > 4096 {
            return Err(ReportBatchLookupLimit {
                resource: "identity bytes",
            }
            .into());
        }
        validate_identifier(outbox_key)?;
        validate_identifier(sid)?;
        ensure!(
            outbox_key == "report.outbox.v1",
            "report metadata lookup requires the current outbox key"
        );
        let transaction = self.connection.unchecked_transaction()?;
        let header: Option<(i64, bool, i64)> = transaction.query_row(
            "SELECT rowid,typeof(value)='text',COALESCE(octet_length(value),0) FROM settings WHERE key=?1", [outbox_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        let Some((rowid, valid, encoded)) = header else {
            transaction.commit()?;
            return Ok(None);
        };
        if !valid {
            return Err(ReportBatchLookupUnavailable.into());
        }
        let encoded = usize::try_from(encoded).map_err(|_| ReportBatchLookupLimit {
            resource: "encoded bytes",
        })?;
        let upper = self
            .text_encoding
            .utf8_upper(encoded)
            .map_err(|_| ReportBatchLookupLimit {
                resource: "encoded bytes",
            })?;
        if upper > 32 * 1024 * 1024 {
            return Err(ReportBatchLookupLimit {
                resource: "encoded bytes",
            }
            .into());
        }
        let parser = upper
            .max(256)
            .checked_next_power_of_two()
            .ok_or(ReportBatchLookupLimit {
                resource: "scratch bytes",
            })?;
        let vm = match self.text_encoding {
            TextEncoding::Utf8 => encoded,
            TextEncoding::Utf16 => encoded
                .checked_mul(3)
                .and_then(|bytes| bytes.checked_add(2))
                .ok_or(ReportBatchLookupLimit {
                    resource: "scratch bytes",
                })?,
        };
        // The parser and SQLite raw value are released before the tiny SID
        // result is owned below. Price those phases with max, not their sum.
        let result_bytes = sid
            .len()
            .checked_add(std::mem::size_of::<ReportBatchDeliveryMetadata>())
            .ok_or(ReportBatchLookupLimit {
                resource: "metadata bytes",
            })?;
        let controls = (2_usize * 1024 * 1024)
            .checked_mul(8)
            .and_then(|bytes| bytes.checked_add(result_bytes))
            .and_then(|bytes| bytes.checked_add(1024))
            .ok_or(ReportBatchLookupLimit {
                resource: "metadata bytes",
            })?;
        if controls > 64 * 1024 * 1024 {
            return Err(ReportBatchLookupLimit {
                resource: "metadata bytes",
            }
            .into());
        }
        // The cursor's raw/conversion storage stays live during parsing.
        let scratch = vm
            .checked_add(parser)
            .ok_or(ReportBatchLookupLimit {
                resource: "scratch bytes",
            })?
            .max(result_bytes);
        if scratch > 64 * 1024 * 1024 {
            return Err(ReportBatchLookupLimit {
                resource: "scratch bytes",
            }
            .into());
        }
        // Borrowed UTF8 validation, token admission and parser decoding are
        // charged as three logical input passes, plus the fixed scratch prime.
        if upper
            .checked_mul(3)
            .and_then(|work| work.checked_add(258 * 3))
            .is_none_or(|work| work > 128 * 1024 * 1024)
        {
            return Err(ReportBatchLookupLimit {
                resource: "work bytes",
            }
            .into());
        }
        let metadata = {
            let mut statement = transaction.prepare("SELECT value FROM settings WHERE rowid=?1")?;
            let mut rows = statement.query([rowid])?;
            let row = rows.next()?.ok_or(ReportBatchLookupUnavailable)?;
            let raw = row
                .get_ref(0)?
                .as_str()
                .map_err(|_| ReportBatchLookupUnavailable)?;
            if raw.len() > upper {
                return Err(ReportBatchLookupLimit {
                    resource: "encoded bytes",
                }
                .into());
            }
            parse_report_batch_delivery_metadata(raw, sid)?
        };
        transaction.commit()?;
        Ok(metadata.map(
            |(state, attempted_at, attempts)| ReportBatchDeliveryMetadata {
                sid: sid.to_owned(),
                state,
                attempted_at,
                attempts,
            },
        ))
    }

    /// Replace durable proof and remove only its superseded side records atomically.
    pub fn set_setting_and_delete<T: Serialize>(
        &self,
        key: &str,
        value: &T,
        removed: &[String],
    ) -> Result<()> {
        validate_identifier(key)?;
        for removed_key in removed {
            validate_identifier(removed_key)?;
            ensure!(
                removed_key != key,
                "replacement setting cannot delete itself"
            );
        }
        let encoded = serde_json::to_string(value)?;
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO settings (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, encoded],
        )?;
        for removed_key in removed {
            transaction.execute("DELETE FROM settings WHERE key=?1", [removed_key])?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Commit trusted, already-admitted compact JSON strings and superseded
    /// records together. Callers must price serialization, output and scratch
    /// before producing these strings; this method does not parse or re-encode
    /// JSON and is not a substitute for that operation-specific admission.
    /// The coarse envelope also prevents an unbudgeted internal caller from
    /// binding unbounded text or allocating an unbounded borrowed-key table.
    pub(crate) fn set_encoded_settings_and_delete(
        &self,
        replacements: &[(&str, &str)],
        removed: &[String],
    ) -> Result<()> {
        self.commit_encoded_settings_and_delete(
            replacements,
            removed,
            encoded_settings_limits(),
            false,
        )
    }

    /// Only current report maintenance may use the independent default
    /// envelope after its ReportInput/exact-encoding admission. Allocation
    /// writes must continue through the generic, dynamically limited method.
    pub(crate) fn set_report_outbox_encoded_and_delete(
        &self,
        encoded: &str,
        removed: &[String],
    ) -> Result<()> {
        self.commit_encoded_settings_and_delete(
            &[("report.outbox.v1", encoded)],
            removed,
            EncodedSettingsLimits::default(),
            true,
        )
    }

    /// Commit already-admitted old-run maintenance within its fixed namespace.
    /// Allocation and outbox/proof writes cannot use this capability.
    pub(crate) fn set_report_active_encoded_and_delete(
        &self,
        replacements: &[(&str, &str)],
        remove_active: bool,
    ) -> Result<()> {
        selected_cached_admit(replacements.len(), 2)?;
        let mut debts = 0usize;
        for &(key, _) in replacements {
            selected_cached_admit(key.len(), EncodedSettingsLimits::default().key_bytes)?;
            if key == "report.active.v1" {
                continue;
            }
            let suffix = key
                .strip_prefix("report.allocation_debt.v1:")
                .ok_or_else(|| {
                    anyhow::anyhow!("active report maintenance key is outside its scope")
                })?;
            let (account, run) = suffix
                .split_once(':')
                .ok_or_else(|| anyhow::anyhow!("invalid report allocation debt key"))?;
            ensure!(
                account == "unattributed"
                    || account.len() == 64 && account.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "invalid report allocation debt account"
            );
            ensure!(
                !run.is_empty() && run.len() <= 256,
                "invalid report allocation debt identity"
            );
            debts = debts.checked_add(1).ok_or(CachedResourceLimit)?;
            ensure!(
                debts <= 1,
                "active report maintenance permits one debt replacement"
            );
        }
        let removed: &[&str] = if remove_active {
            &["report.active.v1"]
        } else {
            &[]
        };
        self.commit_encoded_settings_and_delete(
            replacements,
            removed,
            EncodedSettingsLimits::default(),
            false,
        )
    }

    fn commit_encoded_settings_and_delete<R: AsRef<str>>(
        &self,
        replacements: &[(&str, &str)],
        removed: &[R],
        limits: EncodedSettingsLimits,
        report_only: bool,
    ) -> Result<()> {
        let changes = replacements
            .len()
            .checked_add(removed.len())
            .ok_or(CachedResourceLimit)?;
        selected_cached_admit(changes, MAX_ENCODED_SETTING_CHANGES)?;
        let mut key_bytes = 0_usize;
        let mut encoded_bytes = 0_usize;
        for &(key, value) in replacements {
            selected_cached_admit(key.len(), limits.key_bytes)?;
            key_bytes = key_bytes
                .checked_add(key.len())
                .ok_or(CachedResourceLimit)?;
            encoded_bytes = encoded_bytes
                .checked_add(value.len())
                .ok_or(CachedResourceLimit)?;
        }
        for key in removed {
            let key = key.as_ref();
            selected_cached_admit(key.len(), limits.key_bytes)?;
            key_bytes = key_bytes
                .checked_add(key.len())
                .ok_or(CachedResourceLimit)?;
        }
        let key_slots = encoded_setting_key_slots(replacements.len())?;
        let key_work = key_bytes.checked_mul(4).ok_or(CachedResourceLimit)?;
        selected_cached_admit(
            selected_cached_bytes(&[
                encoded_bytes,
                key_work,
                key_slots,
                std::mem::size_of::<HashSet<&str>>(),
            ])?,
            limits.bytes,
        )?;
        for &(key, _) in replacements {
            validate_identifier(key)?;
        }
        for key in removed {
            validate_identifier(key.as_ref())?;
        }
        if report_only {
            ensure!(
                replacements.len() == 1 && replacements[0].0 == "report.outbox.v1",
                "report maintenance requires the current outbox replacement"
            );
            const PREFIXES: [&str; 7] = [
                "report.receipt:",
                "report.error:",
                "report.failure:",
                "report.retryable:",
                "report.requires_login:",
                "report.await_auth:",
                "report.uncertain:",
            ];
            for key in removed {
                let key = key.as_ref();
                let sid = PREFIXES
                    .iter()
                    .find_map(|prefix| key.strip_prefix(*prefix))
                    .ok_or_else(|| {
                        anyhow::anyhow!("report maintenance cleanup key is outside its scope")
                    })?;
                validate_identifier(sid)?;
            }
        }
        let mut replacement_keys = HashSet::with_capacity(replacements.len());
        for &(key, _) in replacements {
            ensure!(
                replacement_keys.insert(key),
                "duplicate replacement setting key"
            );
        }
        for key in removed {
            ensure!(
                !replacement_keys.contains(key.as_ref()),
                "replacement setting cannot delete itself"
            );
        }
        let transaction = self.connection.unchecked_transaction()?;
        for &(key, value) in replacements {
            transaction.execute(
                "INSERT INTO settings (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )?;
        }
        for key in removed {
            transaction.execute("DELETE FROM settings WHERE key=?1", [key.as_ref()])?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Return one literal-prefix key, rotating after a prior matching marker.
    /// Values stay unprojected. Key metadata prices the prefix/cursor work
    /// before SQL inspects decoded keys; no Rust key list is materialized.
    pub(crate) fn first_setting_key_with_prefix(
        &self,
        prefix: &str,
        after: Option<&str>,
    ) -> Result<Option<String>> {
        let limits = encoded_settings_limits();
        selected_cached_admit(prefix.len(), limits.key_bytes)?;
        validate_identifier(prefix)?;
        if let Some(after) = after {
            selected_cached_admit(after.len(), limits.key_bytes)?;
            validate_identifier(after)?;
            ensure!(
                after.starts_with(prefix),
                "setting cursor is outside the requested prefix"
            );
        }
        let transaction = self.connection.unchecked_transaction()?;
        let (count, encoded, nonempty, maximum, valid): (i64, i64, i64, i64, bool) = transaction.query_row(
            "SELECT COUNT(*),COALESCE(SUM(key_bytes),0),COALESCE(SUM(key_bytes>0),0),
             COALESCE(MAX(key_bytes),0),COALESCE(MIN(valid),1) FROM
             (SELECT COALESCE(octet_length(key),0) AS key_bytes,
              typeof(key)='text' AND COALESCE(octet_length(key),0)>0 AS valid FROM settings LIMIT ?1)",
            [i64::try_from(MAX_ENCODED_SETTING_CHANGES.checked_add(1).ok_or(CachedResourceLimit)?).map_err(|_| CachedResourceLimit)?],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )?;
        if !valid {
            return Err(CachedResourceLimit.into());
        }
        let count = usize::try_from(count).map_err(|_| CachedResourceLimit)?;
        selected_cached_admit(count, MAX_ENCODED_SETTING_CHANGES)?;
        let names = self.text_encoding.utf8_upper_total(
            usize::try_from(encoded).map_err(|_| CachedResourceLimit)?,
            usize::try_from(nonempty).map_err(|_| CachedResourceLimit)?,
        )?;
        let largest = self
            .text_encoding
            .utf8_upper(usize::try_from(maximum).map_err(|_| CachedResourceLimit)?)?;
        let work = selected_cached_bytes(&[
            names.checked_mul(6).ok_or(CachedResourceLimit)?,
            count.checked_mul(16).ok_or(CachedResourceLimit)?,
            prefix.len(),
            after.map_or(0, str::len),
        ])?;
        selected_cached_admit(work, limits.bytes)?;
        let fixed = selected_cached_bytes(&[
            std::mem::size_of::<String>()
                .checked_mul(2)
                .ok_or(CachedResourceLimit)?,
            std::mem::size_of::<i64>()
                .checked_mul(2)
                .ok_or(CachedResourceLimit)?,
        ])?;
        selected_cached_admit(
            selected_cached_bytes(&[largest.checked_mul(2).ok_or(CachedResourceLimit)?, fixed])?,
            limits.bytes,
        )?;
        let sql = "SELECT rowid,typeof(key)='text',COALESCE(octet_length(key),0) FROM settings
                   WHERE substr(key,1,length(?1))=?1 AND (?2 IS NULL OR key>?2) ORDER BY key LIMIT 1";
        let mut header: Option<(i64, bool, i64)> = transaction
            .query_row(sql, params![prefix, after], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()?;
        if header.is_none() && after.is_some() {
            header = transaction
                .query_row(sql, params![prefix, Option::<&str>::None], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .optional()?;
        }
        let Some((rowid, valid, encoded)) = header else {
            transaction.commit()?;
            return Ok(None);
        };
        if !valid {
            return Err(CachedResourceLimit.into());
        }
        let maximum = self
            .text_encoding
            .utf8_upper(usize::try_from(encoded).map_err(|_| CachedResourceLimit)?)?;
        selected_cached_admit(maximum, limits.key_bytes)?;
        selected_cached_admit(selected_cached_bytes(&[maximum, fixed])?, limits.bytes)?;
        #[cfg(test)]
        encoded_settings_tests::after_key_preflight()?;
        let key = {
            let mut statement = transaction.prepare("SELECT key FROM settings WHERE rowid=?1")?;
            let mut rows = statement.query([rowid])?;
            let row = rows.next()?.ok_or(CachedResourceLimit)?;
            let key = cache_text(row, 0, maximum)?;
            validate_identifier(key).map_err(|_| CachedResourceLimit)?;
            #[cfg(test)]
            {
                let mut stats = self.encoded_setting_reads.get();
                stats.key_materializations = stats
                    .key_materializations
                    .checked_add(1)
                    .ok_or(CachedResourceLimit)?;
                stats.key_bytes = stats
                    .key_bytes
                    .checked_add(key.len())
                    .ok_or(CachedResourceLimit)?;
                self.encoded_setting_reads.set(stats);
            }
            key.to_owned()
        };
        transaction.commit()?;
        Ok(Some(key))
    }

    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let value = self
            .connection
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).context("decode local setting"))
            .transpose()
    }

    /// Borrow persisted UTF-8 in one read transaction. The pure callback must
    /// validate/price selected fields and counts before owning any DTO fields.
    /// It must not mutate Store or retain a whole encoded JSON copy.
    pub fn with_bounded_setting_text<T>(
        &self,
        key: &str,
        limits: StoredSettingReadLimits,
        read: impl FnOnce(&str, StoredSettingTextMetadata) -> Result<T>,
    ) -> Result<Option<T>> {
        if self.connection.is_autocommit() {
            let transaction = self.connection.unchecked_transaction()?;
            let value =
                with_stored_setting_text_on(&transaction, self.text_encoding, key, limits, read)?;
            transaction.commit()?;
            Ok(value)
        } else {
            with_stored_setting_text_on(&self.connection, self.text_encoding, key, limits, read)
        }
    }

    /// SQL admission precedes Rust string copying and deserialization. The
    /// original cache is preserved when its type/encoded bytes are unavailable.
    pub fn bounded_setting<T: DeserializeOwned>(
        &self,
        key: &str,
        maximum_bytes: usize,
    ) -> Result<Option<T>> {
        let maximum = self.text_encoding.maximum_encoded(maximum_bytes)?;
        let mut statement = self.connection.prepare(
            "SELECT CASE WHEN typeof(value)='text' AND COALESCE(octet_length(value),0)<=?2
                    THEN value ELSE NULL END FROM settings WHERE key=?1",
        )?;
        let mut rows = statement.query(params![key, maximum])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        if matches!(row.get_ref(0)?, rusqlite::types::ValueRef::Null) {
            return Err(CachedResourceLimit.into());
        }
        let value = cache_text(row, 0, maximum_bytes)?;
        // Deserialization borrows the already admitted UTF-8 SQLite result;
        // no preliminary owned JSON string is needed.
        #[cfg(test)]
        byte_admission_tests::observe_text_copy("setting", 0);
        Ok(Some(
            serde_json::from_str(value).context("decode bounded cached setting")?,
        ))
    }

    /// The JSON array count is evaluated only after its encoded bytes pass;
    /// neither an oversized cache nor excess records are copied into Rust.
    pub fn bounded_array_setting<T: DeserializeOwned>(
        &self,
        key: &str,
        maximum_bytes: usize,
        maximum_records: usize,
    ) -> Result<Option<Vec<T>>> {
        #[cfg(test)]
        self.bounded_array_reads
            .set(self.bounded_array_reads.get() + 1);
        let bytes = self.text_encoding.maximum_encoded(maximum_bytes)?;
        let records = i64::try_from(maximum_records).context("cached array count is too large")?;
        let mut statement = self.connection.prepare(
            "SELECT CASE WHEN typeof(value)='text' AND COALESCE(octet_length(value),0)<=?2 THEN
                CASE WHEN json_valid(value) THEN
                    CASE WHEN json_type(value)='array' AND json_array_length(value)<=?3
                    THEN value ELSE NULL END
                ELSE NULL END
             ELSE NULL END FROM settings WHERE key=?1",
        )?;
        let mut rows = statement.query(params![key, bytes, records])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        if matches!(row.get_ref(0)?, rusqlite::types::ValueRef::Null) {
            return Err(CachedResourceLimit.into());
        }
        let value = cache_text(row, 0, maximum_bytes)?;
        #[cfg(test)]
        byte_admission_tests::observe_text_copy("array", 0);
        let records: Vec<T> = serde_json::from_str(value).context("decode bounded cached array")?;
        ensure!(
            records.len() <= maximum_records,
            "cached array changed beyond its count allowance"
        );
        Ok(Some(records))
    }

    pub fn delete_setting(&self, key: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM settings WHERE key = ?1", [key])?;
        Ok(())
    }

    /// Enumerate a literal key prefix; SQL wildcard characters have no special meaning.
    #[cfg(test)]
    pub fn settings_with_prefix<T: DeserializeOwned>(
        &self,
        prefix: &str,
    ) -> Result<Vec<(String, T)>> {
        validate_identifier(prefix)?;
        let mut statement = self.connection.prepare(
            "SELECT key,value FROM settings WHERE substr(key,1,length(?1))=?1 ORDER BY key",
        )?;
        let rows = statement.query_map([prefix], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut values = Vec::new();
        for row in rows {
            let (key, value) = row?;
            values.push((
                key,
                serde_json::from_str(&value).context("decode local setting collection")?,
            ));
        }
        Ok(values)
    }

    /// Delete an explicit set of settings atomically without touching similarly named keys.
    pub fn delete_settings(&self, keys: &[String]) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        for key in keys {
            transaction.execute("DELETE FROM settings WHERE key=?1", [key])?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn record_session(&self, session: &ReadingSession) -> Result<()> {
        validate_identifier(&session.id)?;
        ensure!(
            session.ended_at >= session.started_at,
            "reading session ends before it starts"
        );
        let elapsed = session
            .ended_at
            .checked_sub(session.started_at)
            .context("reading session range is too large")?;
        ensure!(
            session.seconds <= elapsed as u64,
            "active duration exceeds elapsed session time"
        );
        ensure!(
            session.seconds <= i64::MAX as u64,
            "reading duration is too large"
        );
        self.connection.execute("INSERT INTO reading_sessions (id, book_id, started_at, ended_at, seconds) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET book_id=excluded.book_id, started_at=excluded.started_at, ended_at=excluded.ended_at, seconds=excluded.seconds", params![session.id, session.book_id, session.started_at, session.ended_at, session.seconds as i64])?;
        Ok(())
    }

    /// Sessions overlapping the half-open interval [from, until).
    pub fn sessions(&self, from: i64, until: i64) -> Result<Vec<ReadingSession>> {
        ensure!(from <= until, "invalid reading session range");
        if from == until {
            return Ok(Vec::new());
        }
        let mut statement = self.connection.prepare("SELECT * FROM reading_sessions WHERE started_at < ?2 AND ended_at > ?1 ORDER BY started_at")?;
        let rows = statement.query_map(params![from, until], |row| {
            Ok(ReadingSession {
                id: row.get("id")?,
                book_id: row.get("book_id")?,
                started_at: row.get("started_at")?,
                ended_at: row.get("ended_at")?,
                seconds: row.get::<_, i64>("seconds")? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn remember_query(&self, query: &str) -> Result<()> {
        let query: String = query.trim().chars().take(256).collect();
        if query.is_empty() {
            return Ok(());
        }
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute("INSERT INTO query_history (query, sequence) VALUES (?1, (SELECT COALESCE(MAX(sequence), 0) + 1 FROM query_history)) ON CONFLICT(query) DO UPDATE SET sequence=excluded.sequence", [&query])?;
        transaction.execute("DELETE FROM query_history WHERE query NOT IN (SELECT query FROM query_history ORDER BY sequence DESC LIMIT 20)", [])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn recent_queries(&self, limit: usize) -> Result<Vec<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT query FROM query_history ORDER BY sequence DESC LIMIT ?1")?;
        let rows = statement.query_map([limit.min(20) as i64], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn clear_queries(&self) -> Result<()> {
        self.connection.execute("DELETE FROM query_history", [])?;
        Ok(())
    }

    pub fn summary(&self) -> Result<StorageSummary> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut operation = BodyLengthOperation::new(self.text_encoding);
        let sql = format!(
            "SELECT {BODY_LENGTH_FIELDS} FROM chapters WHERE body IS NOT NULL AND content_status='full' LIMIT ?1"
        );
        let selected = operation.collect(
            &transaction,
            &sql,
            [i64::try_from(
                operation
                    .limits
                    .rows
                    .checked_add(1)
                    .ok_or(CachedResourceLimit)?,
            )
            .map_err(|_| CachedResourceLimit)?],
        )?;
        operation.evaluate(&transaction)?;
        let mut cached_chapters = 0_u64;
        for rowid in selected {
            if operation.positive(rowid)? {
                cached_chapters = cached_chapters.checked_add(1).ok_or(CachedResourceLimit)?;
            }
        }
        let summary = StorageSummary {
            books: self.scalar("SELECT COUNT(*) FROM books")?,
            cached_chapters,
            body_bytes: self.scalar("SELECT COALESCE(SUM(COALESCE(octet_length(body),0)),0) FROM chapters")?,
            annotations: self.scalar("SELECT COUNT(*) FROM annotations")?,
            pending_tasks: self.scalar("SELECT COUNT(*) FROM download_tasks WHERE status IN ('pending','running','paused')")?,
        };
        transaction.commit()?;
        Ok(summary)
    }

    fn query_books<P: rusqlite::Params>(&self, sql: &str, params: P) -> Result<Vec<Book>> {
        #[cfg(test)]
        let _meter = self.book_meter();
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(params, read_book)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn query_tasks(&self, sql: &str) -> Result<Vec<DownloadTask>> {
        if self.connection.is_autocommit() {
            let transaction = self.connection.unchecked_transaction()?;
            let tasks = query_task_inventory_on(&transaction, self.text_encoding, sql, None)?;
            transaction.commit()?;
            Ok(tasks)
        } else {
            query_task_inventory_on(&self.connection, self.text_encoding, sql, None)
        }
    }

    fn scalar(&self, sql: &str) -> Result<u64> {
        let value = self
            .connection
            .query_row(sql, [], |row| row.get::<_, i64>(0))?;
        u64::try_from(value).context("invalid storage scalar")
    }

    fn recover_interrupted_tasks(&self) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut operation = BodyLengthOperation::new(self.text_encoding);
        let completed =
            prepare_completed_tasks_on(&transaction, self.text_encoding, &mut operation)?;
        operation.evaluate(&transaction)?;
        for mut task in
            self.query_tasks("SELECT payload FROM download_tasks WHERE status = 'running'")?
        {
            task.status = DownloadStatus::Paused;
            task.error = None;
            self.save_task(&task)?;
        }
        apply_completed_task_pauses_on(&transaction, &operation, completed)?;
        transaction.commit()?;
        Ok(())
    }

    /// Downgrade another Web account's access proof without deleting its local text or notes.
    pub fn invalidate_content_ownership(&self, account_key: Option<&str>) -> Result<()> {
        let owner = account_key.filter(|owner| !owner.is_empty());
        let transaction = self.connection.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE chapters SET content_status='unknown'
             WHERE book_id IN (SELECT id FROM books WHERE source='weread')
               AND content_status<>'unknown'
               AND (?1 IS NULL OR content_account_key<>?1)",
            [owner],
        )?;
        let mut operation = BodyLengthOperation::new(self.text_encoding);
        let groups = if changed != 0 {
            Some(plan_weread_cached_counts_on(&transaction, &mut operation)?)
        } else {
            None
        };
        let completed =
            prepare_completed_tasks_on(&transaction, self.text_encoding, &mut operation)?;
        operation.evaluate(&transaction)?;
        if let Some(groups) = groups {
            apply_weread_cached_counts_on(&transaction, &operation, groups)?;
        }
        apply_completed_task_pauses_on(&transaction, &operation, completed)?;
        transaction.commit()?;
        Ok(())
    }
}

struct ValidatedDownloadChapter {
    body: Option<String>,
    was_full: bool,
    is_full: bool,
    marker_boundary: Option<u32>,
}

fn has_full_body(chapter: &Chapter) -> bool {
    chapter.content_status == ContentStatus::Full
        && chapter
            .body
            .as_deref()
            .is_some_and(|body| !body.trim().is_empty())
}

const BODY_LENGTH_FIELDS: &str = "rowid,CASE typeof(body) WHEN 'text' THEN 1 WHEN 'blob' THEN 2
    WHEN 'integer' THEN 3 WHEN 'real' THEN 3 ELSE 0 END,COALESCE(octet_length(body),0)";

struct BodyLengthRow {
    kind: u8,
    encoded: usize,
    positive: Option<bool>,
}

struct BodyLengthOperation {
    rows: HashMap<i64, BodyLengthRow>,
    limits: BodyLengthLimits,
    encoding: TextEncoding,
    work: usize,
    metadata: usize,
    largest_temporary: usize,
    evaluated: bool,
}

impl BodyLengthOperation {
    fn new(encoding: TextEncoding) -> Self {
        Self {
            rows: HashMap::new(),
            limits: body_length_limits(),
            encoding,
            work: 0,
            metadata: 4096,
            largest_temporary: 0,
            evaluated: false,
        }
    }

    fn charge_input(&mut self, bytes: usize) -> Result<()> {
        self.work =
            selected_cached_bytes(&[self.work, bytes.checked_mul(2).ok_or(CachedResourceLimit)?])?;
        self.metadata = selected_cached_bytes(&[self.metadata, bytes, 512])?;
        selected_cached_admit(self.work, self.limits.work_bytes)?;
        selected_cached_admit(self.metadata, self.limits.metadata_bytes)
    }

    fn add(&mut self, rowid: i64, kind: u8, encoded: usize) -> Result<()> {
        self.work = selected_cached_bytes(&[self.work, 24])?;
        selected_cached_admit(self.work, self.limits.work_bytes)?;
        #[cfg(test)]
        BODY_LENGTH_READ_STATS.with(|stats| {
            let mut value = stats.get();
            value.metadata_rows += 1;
            stats.set(value);
        });
        if let Some(previous) = self.rows.get(&rowid) {
            if previous.kind != kind || previous.encoded != encoded {
                return Err(CachedResourceLimit.into());
            }
            return Ok(());
        }
        if self.evaluated {
            return Err(CachedResourceLimit.into());
        }
        selected_cached_admit(
            self.rows.len().checked_add(1).ok_or(CachedResourceLimit)?,
            self.limits.rows,
        )?;
        // Cover the borrowed index, scalar rows and caller rowid vectors before
        // inserting their first element. This is a logical scratch forecast.
        self.metadata = selected_cached_bytes(&[self.metadata, 512])?;
        selected_cached_admit(self.metadata, self.limits.metadata_bytes)?;
        let positive = match kind {
            0 => Some(false),
            2 => Some(encoded > 0), // SQLite length(BLOB) uses byte length.
            3 => Some(true),        // Numeric text representations are nonempty.
            1 if encoded == 0 => Some(false),
            1 => {
                let (work, temporary) = match self.encoding {
                    TextEncoding::Utf8 => (
                        encoded.checked_mul(2).ok_or(CachedResourceLimit)?,
                        encoded.checked_add(2).ok_or(CachedResourceLimit)?,
                    ),
                    TextEncoding::Utf16 => (
                        encoded.checked_mul(4).ok_or(CachedResourceLimit)?,
                        encoded
                            .checked_mul(3)
                            .and_then(|bytes| bytes.checked_add(2))
                            .ok_or(CachedResourceLimit)?,
                    ),
                };
                self.work = selected_cached_bytes(&[self.work, work])?;
                selected_cached_admit(self.work, self.limits.work_bytes)?;
                self.largest_temporary = self.largest_temporary.max(temporary);
                None
            }
            _ => return Err(CachedResourceLimit.into()),
        };
        self.rows.insert(
            rowid,
            BodyLengthRow {
                kind,
                encoded,
                positive,
            },
        );
        #[cfg(test)]
        BODY_LENGTH_READ_STATS.with(|stats| {
            let mut value = stats.get();
            value.unique_rows += 1;
            stats.set(value);
        });
        Ok(())
    }

    fn collect<P: rusqlite::Params>(
        &mut self,
        connection: &Connection,
        sql: &str,
        params: P,
    ) -> Result<Vec<i64>> {
        let mut statement = connection.prepare(sql)?;
        let mut rows = statement.query(params)?;
        let mut selected = Vec::new();
        while let Some(row) = rows.next()? {
            let rowid: i64 = row.get(0)?;
            let kind: u8 = row.get(1)?;
            let encoded =
                usize::try_from(row.get::<_, i64>(2)?).map_err(|_| CachedResourceLimit)?;
            self.add(rowid, kind, encoded)?;
            selected.push(rowid);
        }
        Ok(selected)
    }

    fn evaluate(&mut self, connection: &Connection) -> Result<()> {
        selected_cached_admit(
            selected_cached_bytes(&[self.metadata, self.largest_temporary])?,
            self.limits.temporary_bytes,
        )?;
        #[cfg(test)]
        body_length_budget_tests::before_evaluation()?;
        for (rowid, row) in &mut self.rows {
            if row.positive.is_some() {
                continue;
            }
            // Keep the engine's first-NUL, malformed/odd UTF and BOM behavior.
            // Its entire TEXT decode/scan was prepaid for every unique row.
            // This is not a claim of zero VM reads, total RSS or absolute time.
            let positive = connection
                .query_row(
                    "SELECT length(body)>0 FROM chapters WHERE rowid=?1",
                    [rowid],
                    |row| row.get::<_, Option<bool>>(0),
                )?
                .ok_or(CachedResourceLimit)?;
            row.positive = Some(positive);
            #[cfg(test)]
            BODY_LENGTH_READ_STATS.with(|stats| {
                let mut value = stats.get();
                value.text_evaluations += 1;
                value.encoded_text_bytes += row.encoded;
                stats.set(value);
            });
        }
        self.evaluated = true;
        Ok(())
    }

    fn positive(&self, rowid: i64) -> Result<bool> {
        if !self.evaluated {
            return Err(CachedResourceLimit.into());
        }
        self.rows
            .get(&rowid)
            .and_then(|row| row.positive)
            .ok_or_else(|| CachedResourceLimit.into())
    }
}

fn cached_counts_for_books_on<'a>(
    connection: &Connection,
    encoding: TextEncoding,
    books: impl IntoIterator<Item = &'a str>,
) -> Result<HashMap<&'a str, i64>> {
    let mut operation = BodyLengthOperation::new(encoding);
    let mut seen = HashSet::new();
    let mut groups = Vec::new();
    let sql = format!(
        "SELECT {BODY_LENGTH_FIELDS} FROM chapters WHERE book_id=?1 AND content_status='full' AND body IS NOT NULL LIMIT ?2"
    );
    for book in books {
        operation.charge_input(book.len())?;
        validate_identifier(book)?;
        if !seen.insert(book) {
            continue;
        }
        let rows = operation.collect(
            connection,
            &sql,
            params![
                book,
                i64::try_from(
                    operation
                        .limits
                        .rows
                        .checked_add(1)
                        .ok_or(CachedResourceLimit)?
                )
                .map_err(|_| CachedResourceLimit)?
            ],
        )?;
        groups.push((book, rows));
    }
    operation.evaluate(connection)?;
    groups
        .into_iter()
        .map(|(book, rows)| {
            let mut count = 0_i64;
            for rowid in rows {
                if operation.positive(rowid)? {
                    count = count.checked_add(1).ok_or(CachedResourceLimit)?;
                }
            }
            Ok((book, count))
        })
        .collect()
}

fn update_cached_counts_on<'a>(
    connection: &Connection,
    encoding: TextEncoding,
    books: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    for (book, count) in cached_counts_for_books_on(connection, encoding, books)? {
        connection.execute(
            "UPDATE books SET cached_chapters=?2 WHERE id=?1",
            params![book, count],
        )?;
    }
    Ok(())
}

fn plan_weread_cached_counts_on(
    connection: &Connection,
    operation: &mut BodyLengthOperation,
) -> Result<BTreeMap<i64, Vec<i64>>> {
    let mut groups = BTreeMap::new();
    let mut statement = connection.prepare("SELECT c.rowid,CASE typeof(c.body) WHEN 'text' THEN 1 WHEN 'blob' THEN 2
        WHEN 'integer' THEN 3 WHEN 'real' THEN 3 ELSE 0 END,COALESCE(octet_length(c.body),0),b.rowid
        FROM chapters c JOIN books b ON b.id=c.book_id WHERE b.source='weread' AND c.content_status='full'
        AND c.body IS NOT NULL LIMIT ?1")?;
    let mut rows = statement.query([i64::try_from(
        operation
            .limits
            .rows
            .checked_add(1)
            .ok_or(CachedResourceLimit)?,
    )
    .map_err(|_| CachedResourceLimit)?])?;
    while let Some(row) = rows.next()? {
        let rowid: i64 = row.get(0)?;
        operation.add(
            rowid,
            row.get(1)?,
            usize::try_from(row.get::<_, i64>(2)?).map_err(|_| CachedResourceLimit)?,
        )?;
        groups
            .entry(row.get::<_, i64>(3)?)
            .or_insert_with(Vec::new)
            .push(rowid);
    }
    Ok(groups)
}

fn apply_weread_cached_counts_on(
    connection: &Connection,
    operation: &BodyLengthOperation,
    groups: BTreeMap<i64, Vec<i64>>,
) -> Result<()> {
    connection.execute(
        "UPDATE books SET cached_chapters=0 WHERE source='weread'",
        [],
    )?;
    for (book_rowid, rows) in groups {
        let mut count = 0_i64;
        for rowid in rows {
            if operation.positive(rowid)? {
                count = count.checked_add(1).ok_or(CachedResourceLimit)?;
            }
        }
        connection.execute(
            "UPDATE books SET cached_chapters=?2 WHERE rowid=?1",
            params![book_rowid, count],
        )?;
    }
    Ok(())
}

fn completed_body_rows_on(
    connection: &Connection,
    encoding: TextEncoding,
    operation: &mut BodyLengthOperation,
    task: &DownloadTask,
) -> Result<Vec<Option<i64>>> {
    let input = selected_cached_bytes(&[
        task.book_id.len(),
        task.account_key.len(),
        task.chapters
            .len()
            .checked_mul(16)
            .ok_or(CachedResourceLimit)?,
    ])?;
    operation.charge_input(input)?;
    let owner_maximum = encoding.maximum_encoded(16 * 1024)?;
    let fields = BODY_LENGTH_FIELDS
        .replace("rowid", "c.rowid")
        .replace("body", "c.body");
    let sql = format!(
        "SELECT {fields},CASE WHEN typeof(c.content_account_key)<>'text' THEN 0
        WHEN COALESCE(octet_length(c.content_account_key),0)>?4 THEN NULL
        ELSE c.body IS NOT NULL AND c.content_status='full'
        AND c.content_account_key=?3 END FROM chapters c
        WHERE c.book_id=?1 AND c.chapter_index=?2"
    );
    let mut selected = Vec::with_capacity(task.chapters.len());
    for index in &task.chapters {
        let row: Option<(i64, u8, i64, Option<bool>)> = connection
            .query_row(
                &sql,
                params![task.book_id, index, task.account_key, owner_maximum],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((rowid, kind, encoded, eligible)) = row else {
            selected.push(None);
            break;
        };
        let eligible = eligible.ok_or(CachedResourceLimit)?;
        if !eligible {
            selected.push(None);
            break;
        }
        operation.add(
            rowid,
            kind,
            usize::try_from(encoded).map_err(|_| CachedResourceLimit)?,
        )?;
        selected.push(Some(rowid));
    }
    Ok(selected)
}

fn validate_completed_task_on(
    connection: &Connection,
    encoding: TextEncoding,
    task: &DownloadTask,
) -> Result<()> {
    let mut operation = BodyLengthOperation::new(encoding);
    let selected = completed_body_rows_on(connection, encoding, &mut operation, task)?;
    operation.evaluate(connection)?;
    let mut full = selected.len() == task.chapters.len();
    for rowid in selected {
        full &= match rowid {
            Some(rowid) => operation.positive(rowid)?,
            None => false,
        };
    }
    ensure!(
        full,
        "unverified or limited content prevents task completion"
    );
    Ok(())
}

fn prepare_completed_tasks_on(
    connection: &Connection,
    encoding: TextEncoding,
    operation: &mut BodyLengthOperation,
) -> Result<Vec<(DownloadTask, Vec<Option<i64>>)>> {
    let (count, bytes, cells, valid): (i64, i64, i64, bool) = connection.query_row(
        "SELECT COUNT(*),COALESCE(SUM(bytes),0),COALESCE(SUM(bytes>0),0),COALESCE(MIN(valid),1)
         FROM (SELECT COALESCE(octet_length(payload),0) AS bytes,typeof(payload)='text' AS valid
         FROM download_tasks WHERE status='complete' LIMIT ?1)",
        [i64::try_from(
            MAX_CATALOG_HEADERS
                .checked_add(1)
                .ok_or(CachedResourceLimit)?,
        )
        .map_err(|_| CachedResourceLimit)?],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let count = usize::try_from(count).map_err(|_| CachedResourceLimit)?;
    if !valid || count > MAX_CATALOG_HEADERS {
        return Err(CachedResourceLimit.into());
    }
    let upper = encoding.utf8_upper_total(
        usize::try_from(bytes).map_err(|_| CachedResourceLimit)?,
        usize::try_from(cells).map_err(|_| CachedResourceLimit)?,
    )?;
    operation.charge_input(upper.checked_mul(16).ok_or(CachedResourceLimit)?)?;
    let mut tasks = Vec::with_capacity(count);
    {
        let mut statement =
            connection.prepare("SELECT payload FROM download_tasks WHERE status='complete'")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let raw = cache_text(row, 0, upper)?;
            let task = serde_json::from_str::<DownloadTask>(raw)
                .context("decode completed task during content verification")?;
            let selected = completed_body_rows_on(connection, encoding, operation, &task)?;
            tasks.push((task, selected));
        }
    }
    Ok(tasks)
}

fn apply_completed_task_pauses_on(
    connection: &Connection,
    operation: &BodyLengthOperation,
    tasks: Vec<(DownloadTask, Vec<Option<i64>>)>,
) -> Result<()> {
    for (task, selected) in tasks {
        let mut prefix = 0_usize;
        for rowid in selected {
            let Some(rowid) = rowid else {
                break;
            };
            if !operation.positive(rowid)? {
                break;
            }
            prefix = prefix.checked_add(1).ok_or(CachedResourceLimit)?;
        }
        if prefix != task.chapters.len() {
            connection.execute(
                "UPDATE download_tasks SET status='paused',
                 payload=json_set(payload,'$.status','paused','$.next_index',?2,
                   '$.error','source.content_unverified') WHERE id=?1 AND status='complete'",
                params![
                    task.id,
                    i64::try_from(prefix).map_err(|_| CachedResourceLimit)?
                ],
            )?;
        }
    }
    Ok(())
}

fn validate_download_chapter_on(
    connection: &Connection,
    task: &DownloadTask,
    chapter: &Chapter,
    next_index: usize,
) -> Result<ValidatedDownloadChapter> {
    validate_identifier(&task.id)?;
    validate_identifier(&task.book_id)?;
    validate_identifier(&chapter.id)?;
    let is_full = has_full_body(chapter);
    ensure!(
        task.status == DownloadStatus::Running
            && task.next_index < task.chapters.len()
            && task.chapters.get(task.next_index) == Some(&chapter.index)
            && task.book_id == chapter.book_id
            && chapter.content_account_key == task.account_key
            && chapter.cached == is_full,
        "download checkpoint does not match the running task or content owner"
    );
    if chapter.content_status == ContentStatus::Full {
        ensure!(
            is_full && task.next_index.checked_add(1) == Some(next_index),
            "full content must advance the download cursor exactly once"
        );
    } else {
        ensure!(
            next_index == task.next_index,
            "limited content cannot advance the download cursor"
        );
    }
    if chapter.content_status != ContentStatus::Locked || chapter.body.is_some() {
        ensure!(
            chapter
                .body
                .as_deref()
                .is_some_and(|body| !body.trim().is_empty()),
            "download checkpoint has no readable body"
        );
    }
    let cursor = i64::try_from(task.next_index).context("download cursor exceeds SQLite range")?;
    let chapter_path = format!("$.chapters[{}]", task.next_index);
    let plan = serde_json::to_string(&task.chapters)?;
    let old = connection
        .query_row(
            "SELECT c.body,c.content_status,c.content_account_key,b.source
         FROM download_tasks AS t
         JOIN chapters AS c ON c.book_id=t.book_id AND c.chapter_index=:chapter_index
         JOIN books AS b ON b.id=t.book_id
         WHERE t.id=:task_id AND t.book_id=:book_id AND t.status='running'
           AND t.updated_at=:old_updated_at
           AND json_extract(t.payload,'$.id')=:task_id
           AND json_extract(t.payload,'$.book_id')=:book_id
           AND json_extract(t.payload,'$.account_key')=:account_key
           AND json_extract(t.payload,'$.status')='running'
           AND json_type(t.payload,'$.next_index')='integer'
           AND json_extract(t.payload,'$.next_index')=:cursor
           AND json_type(t.payload,'$.chapters')='array'
           AND json_extract(t.payload,'$.chapters')=json(:plan)
           AND json_type(t.payload,:chapter_path)='integer'
           AND json_extract(t.payload,:chapter_path)=:chapter_index
           AND json_extract(t.payload,'$.include_images')=:include_images
           AND json_extract(t.payload,'$.updated_at')=:old_updated_at
           AND json_extract(t.payload,'$.error') IS :error
           AND c.source_id=:chapter_uid",
            named_params! {
                ":task_id": &task.id, ":book_id": &task.book_id, ":account_key": &task.account_key,
                ":old_updated_at": task.updated_at, ":cursor": cursor, ":plan": &plan,
                ":chapter_path": &chapter_path, ":chapter_index": chapter.index,
                ":chapter_uid": &chapter.id, ":include_images": task.include_images,
                ":error": task.error.as_deref(),
            },
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?
        .context("download checkpoint has a stale task, owner, or catalog")?;
    validate_download_content_on(connection, task, chapter, old)
}

fn validate_download_content_on(
    connection: &Connection,
    task: &DownloadTask,
    chapter: &Chapter,
    old: (Option<String>, String, String, String),
) -> Result<ValidatedDownloadChapter> {
    let is_full = has_full_body(chapter);
    ensure!(
        task.book_id == chapter.book_id
            && chapter.content_account_key == task.account_key
            && chapter.cached == is_full,
        "download content does not match its task or owner"
    );
    if chapter.content_status == ContentStatus::Full {
        ensure!(is_full, "full content has no readable body");
    }
    if chapter.content_status != ContentStatus::Locked || chapter.body.is_some() {
        ensure!(
            chapter
                .body
                .as_deref()
                .is_some_and(|body| !body.trim().is_empty()),
            "download checkpoint has no readable body"
        );
    }
    let (old_body, old_status, old_owner, source) = old;
    ensure!(
        source != "weread" || !task.account_key.is_empty(),
        "remote content needs a verified account owner"
    );
    let old_status = ContentStatus::parse(&old_status).context("invalid stored content status")?;
    let was_full = old_status == ContentStatus::Full
        && old_body
            .as_deref()
            .is_some_and(|body| !body.trim().is_empty());
    let mut marker_boundary = None;
    if let (Some(previous), Some(incoming)) = (old_body.as_deref(), chapter.body.as_deref()) {
        if previous != incoming {
            let same_owner = old_owner == task.account_key;
            if !is_full || old_status == ContentStatus::Full || !same_owner {
                return Err(ContentConflict.into());
            }
            if !incoming.starts_with(previous) {
                let prefix = previous
                    .strip_suffix("...")
                    .or_else(|| previous.strip_suffix('…'));
                let Some(prefix) = prefix.filter(|prefix| {
                    !prefix.is_empty()
                        && incoming.starts_with(prefix)
                        && incoming.len() > prefix.len()
                }) else {
                    return Err(ContentConflict.into());
                };
                let boundary = u32::try_from(prefix.chars().count())
                    .context("preview prefix exceeds scalar range")?;
                let intersects_marker: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM annotations
                     WHERE book_id=?1 AND chapter_index=?2 AND end_offset>?3)",
                    params![chapter.book_id, chapter.index, boundary],
                    |row| row.get(0),
                )?;
                if intersects_marker {
                    return Err(ContentConflict.into());
                }
                marker_boundary = Some(boundary);
            }
        }
    }
    Ok(ValidatedDownloadChapter {
        body: chapter.body.clone().or(old_body),
        was_full,
        is_full,
        marker_boundary,
    })
}

fn write_download_chapter_on(
    connection: &Connection,
    task: &DownloadTask,
    chapter: &Chapter,
    validated: &ValidatedDownloadChapter,
) -> Result<()> {
    ensure!(
        connection.execute(
            "UPDATE chapters SET body=?4,content_status=?5,content_account_key=?6
             WHERE book_id=?1 AND chapter_index=?2 AND source_id=?3",
            params![
                chapter.book_id,
                chapter.index,
                chapter.id,
                validated.body,
                chapter.content_status.as_str(),
                chapter.content_account_key
            ],
        )? == 1,
        "download checkpoint chapter disappeared"
    );
    if validated.was_full != validated.is_full {
        let delta = if validated.is_full { 1_i64 } else { -1_i64 };
        ensure!(
            connection.execute(
                "UPDATE books SET cached_chapters=cached_chapters+?2
                 WHERE id=?1 AND cached_chapters+?2>=0",
                params![task.book_id, delta],
            )? == 1,
            "download checkpoint cached count is inconsistent"
        );
    }
    if let Some(boundary) = validated.marker_boundary {
        // The removed preview marker was not authorized source text.
        connection.execute(
            "UPDATE positions SET char_offset=?3
             WHERE book_id=?1 AND chapter_index=?2 AND char_offset>?3",
            params![chapter.book_id, chapter.index, boundary],
        )?;
        connection.execute(
            "UPDATE bookmarks SET char_offset=?3
             WHERE book_id=?1 AND chapter_index=?2 AND char_offset>?3",
            params![chapter.book_id, chapter.index, boundary],
        )?;
    }
    Ok(())
}

fn validate_identifier(id: &str) -> Result<()> {
    ensure!(!id.trim().is_empty(), "identifier must not be empty");
    ensure!(!id.contains('\0'), "identifier contains a null character");
    Ok(())
}

fn selected_book_id(id: &str) -> Result<()> {
    book_metadata_admit("identifier bytes", id.len(), 4096)?;
    validate_identifier(id).map_err(|_| BookMetadataUnavailable.into())
}

fn selected_chapter_id(id: &str) -> Result<()> {
    selected_cached_admit(id.len(), 4096)?;
    validate_identifier(id).map_err(|_| CachedResourceLimit.into())
}

fn selected_cached_bytes(terms: &[usize]) -> Result<usize> {
    terms.iter().try_fold(0_usize, |sum, term| {
        sum.checked_add(*term)
            .ok_or_else(|| CachedResourceLimit.into())
    })
}

fn selected_cached_admit(requested: usize, maximum: usize) -> Result<()> {
    if requested > maximum {
        Err(CachedResourceLimit.into())
    } else {
        Ok(())
    }
}

fn encoded_setting_key_slots(count: usize) -> Result<usize> {
    if count == 0 {
        return Ok(0);
    }
    // Cover power-of-two bucket growth and a control-byte group without
    // owning any setting key. This remains a conservative logical forecast.
    let buckets = count
        .checked_mul(4)
        .and_then(|count| count.checked_add(16))
        .ok_or(CachedResourceLimit)?;
    let slot = std::mem::size_of::<&str>()
        .checked_add(1)
        .ok_or(CachedResourceLimit)?;
    buckets
        .checked_mul(slot)
        .ok_or_else(|| CachedResourceLimit.into())
}

fn parse_report_batch_delivery_metadata(
    raw: &str,
    sid: &str,
) -> Result<Option<(ReportBatchDeliveryState, Option<i64>, u32)>> {
    use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
    use std::{fmt, io::Read as _};
    // Resource policy: current SDK numeric fields fit within 20 bytes. Bound
    // unquoted numeric tokens before serde's optional long-number scratch path.
    // Longer otherwise-valid values are refused, never truncated as success.
    let mut quoted = false;
    let mut escaped = false;
    let mut numeric = 0_usize;
    for byte in raw.bytes() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else if byte == b'"' {
            quoted = true;
            numeric = 0;
        } else if matches!(byte, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
            numeric = numeric.checked_add(1).ok_or(ReportBatchLookupLimit {
                resource: "numeric token bytes",
            })?;
            if numeric > 128 {
                return Err(ReportBatchLookupLimit {
                    resource: "numeric token bytes",
                }
                .into());
            }
        } else {
            numeric = 0;
        }
    }
    struct Budget {
        nodes: usize,
        records: usize,
        denied: Option<&'static str>,
    }
    impl Budget {
        fn tick<E: serde::de::Error>(&mut self) -> std::result::Result<(), E> {
            self.nodes = self
                .nodes
                .checked_add(1)
                .ok_or_else(|| serde::de::Error::custom("metadata node arithmetic"))?;
            if self.nodes > 2 * 1024 * 1024 {
                self.denied = Some("metadata nodes");
                return Err(serde::de::Error::custom("metadata node allowance exceeded"));
            }
            Ok(())
        }
    }
    enum Key {
        Batches,
        Cursors,
        Proofs,
        Request,
        Sid,
        State,
        Attempted,
        Attempts,
        Other,
    }
    struct KeySeed<'a>(&'a mut Budget);
    impl<'de> DeserializeSeed<'de> for KeySeed<'_> {
        type Value = Key;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<Key, D::Error> {
            self.0.tick()?;
            struct Keys;
            impl Visitor<'_> for Keys {
                type Value = Key;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("metadata field name")
                }
                fn visit_str<E: serde::de::Error>(self, key: &str) -> std::result::Result<Key, E> {
                    Ok(match key {
                        "batches" => Key::Batches,
                        "reported_cursors" => Key::Cursors,
                        "compacted_runs" => Key::Proofs,
                        "request" => Key::Request,
                        "session_id" => Key::Sid,
                        "state" => Key::State,
                        "attempted_at" => Key::Attempted,
                        "attempts" => Key::Attempts,
                        _ => Key::Other,
                    })
                }
            }
            deserializer.deserialize_identifier(Keys)
        }
    }
    struct Skip<'a>(&'a mut Budget);
    impl<'de> DeserializeSeed<'de> for Skip<'_> {
        type Value = ();
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<(), D::Error> {
            self.0.tick()?;
            struct SkipVisitor<'a>(&'a mut Budget);
            impl<'de> Visitor<'de> for SkipVisitor<'_> {
                type Value = ();
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("bounded JSON value")
                }
                fn visit_bool<E: serde::de::Error>(self, _: bool) -> std::result::Result<(), E> {
                    Ok(())
                }
                fn visit_i64<E: serde::de::Error>(self, _: i64) -> std::result::Result<(), E> {
                    Ok(())
                }
                fn visit_u64<E: serde::de::Error>(self, _: u64) -> std::result::Result<(), E> {
                    Ok(())
                }
                fn visit_f64<E: serde::de::Error>(self, _: f64) -> std::result::Result<(), E> {
                    Ok(())
                }
                fn visit_str<E: serde::de::Error>(self, _: &str) -> std::result::Result<(), E> {
                    Ok(())
                }
                fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<(), E> {
                    Ok(())
                }
                fn visit_seq<S: SeqAccess<'de>>(
                    self,
                    mut sequence: S,
                ) -> std::result::Result<(), S::Error> {
                    while sequence.next_element_seed(Skip(&mut *self.0))?.is_some() {}
                    Ok(())
                }
                fn visit_map<M: MapAccess<'de>>(
                    self,
                    mut map: M,
                ) -> std::result::Result<(), M::Error> {
                    while map.next_key_seed(KeySeed(&mut *self.0))?.is_some() {
                        map.next_value_seed(Skip(&mut *self.0))?;
                    }
                    Ok(())
                }
            }
            deserializer.deserialize_any(SkipVisitor(self.0))
        }
    }
    struct MapSkip<'a>(&'a mut Budget);
    impl<'de> DeserializeSeed<'de> for MapSkip<'_> {
        type Value = ();
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<(), D::Error> {
            self.0.tick()?;
            struct Maps<'a>(&'a mut Budget);
            impl<'de> Visitor<'de> for Maps<'_> {
                type Value = ();
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("proof metadata object")
                }
                fn visit_map<M: MapAccess<'de>>(
                    self,
                    mut map: M,
                ) -> std::result::Result<(), M::Error> {
                    while map.next_key_seed(KeySeed(&mut *self.0))?.is_some() {
                        map.next_value_seed(Skip(&mut *self.0))?;
                    }
                    Ok(())
                }
            }
            deserializer.deserialize_map(Maps(self.0))
        }
    }
    struct Sid<'a, 'b>(&'a mut Budget, &'b str);
    impl<'de> DeserializeSeed<'de> for Sid<'_, '_> {
        type Value = bool;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<bool, D::Error> {
            self.0.tick()?;
            struct Sids<'a, 'b>(&'a mut Budget, &'b str);
            impl Visitor<'_> for Sids<'_, '_> {
                type Value = bool;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("bounded session identity")
                }
                fn visit_str<E: serde::de::Error>(
                    self,
                    value: &str,
                ) -> std::result::Result<bool, E> {
                    if value.len() > 4096 {
                        self.0.denied = Some("session identity");
                        return Err(serde::de::Error::custom(
                            "session identity allowance exceeded",
                        ));
                    }
                    if value.trim().is_empty() || value.contains('\0') {
                        return Err(serde::de::Error::custom("invalid session identity"));
                    }
                    Ok(value == self.1)
                }
            }
            deserializer.deserialize_str(Sids(self.0, self.1))
        }
    }
    struct Request<'a, 'b>(&'a mut Budget, &'b str);
    impl<'de> DeserializeSeed<'de> for Request<'_, '_> {
        type Value = bool;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<bool, D::Error> {
            self.0.tick()?;
            struct Requests<'a, 'b>(&'a mut Budget, &'b str);
            impl<'de> Visitor<'de> for Requests<'_, '_> {
                type Value = bool;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("report request metadata")
                }
                fn visit_map<M: MapAccess<'de>>(
                    self,
                    mut map: M,
                ) -> std::result::Result<bool, M::Error> {
                    let mut found = None;
                    while let Some(key) = map.next_key_seed(KeySeed(&mut *self.0))? {
                        match key {
                            Key::Sid => {
                                if found.is_some() {
                                    return Err(serde::de::Error::duplicate_field("session_id"));
                                }
                                found = Some(map.next_value_seed(Sid(&mut *self.0, self.1))?);
                            }
                            _ => {
                                map.next_value_seed(Skip(&mut *self.0))?;
                            }
                        }
                    }
                    found.ok_or_else(|| serde::de::Error::missing_field("session_id"))
                }
            }
            deserializer.deserialize_map(Requests(self.0, self.1))
        }
    }
    struct Scalar<'a, T>(&'a mut Budget, std::marker::PhantomData<T>);
    impl<'de, T: serde::Deserialize<'de>> DeserializeSeed<'de> for Scalar<'_, T> {
        type Value = T;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<T, D::Error> {
            self.0.tick()?;
            T::deserialize(deserializer)
        }
    }
    struct Batch<'a, 'b>(&'a mut Budget, &'b str);
    type Evidence = (ReportBatchDeliveryState, Option<i64>, u32);
    impl<'de> DeserializeSeed<'de> for Batch<'_, '_> {
        type Value = Option<Evidence>;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            self.0.tick()?;
            self.0.records = self
                .0
                .records
                .checked_add(1)
                .ok_or_else(|| serde::de::Error::custom("record count arithmetic"))?;
            if self.0.records > 8192 {
                self.0.denied = Some("batch records");
                return Err(serde::de::Error::custom("batch record allowance exceeded"));
            }
            struct Batches<'a, 'b>(&'a mut Budget, &'b str);
            impl<'de> Visitor<'de> for Batches<'_, '_> {
                type Value = Option<Evidence>;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("report batch metadata")
                }
                fn visit_map<M: MapAccess<'de>>(
                    self,
                    mut map: M,
                ) -> std::result::Result<Self::Value, M::Error> {
                    let mut matched = None;
                    let mut state = None;
                    let mut attempts = None;
                    let mut attempted_at = None;
                    let mut attempted_seen = false;
                    while let Some(key) = map.next_key_seed(KeySeed(&mut *self.0))? {
                        match key {
                            Key::Request => {
                                if matched.is_some() {
                                    return Err(serde::de::Error::duplicate_field("request"));
                                }
                                matched = Some(map.next_value_seed(Request(&mut *self.0, self.1))?);
                            }
                            Key::State => {
                                if state.is_some() {
                                    return Err(serde::de::Error::duplicate_field("state"));
                                }
                                state = Some(map.next_value_seed(Scalar::<
                                    ReportBatchDeliveryState,
                                >(
                                    &mut *self.0,
                                    std::marker::PhantomData,
                                ))?);
                            }
                            Key::Attempts => {
                                if attempts.is_some() {
                                    return Err(serde::de::Error::duplicate_field("attempts"));
                                }
                                attempts = Some(map.next_value_seed(Scalar::<u32>(
                                    &mut *self.0,
                                    std::marker::PhantomData,
                                ))?);
                            }
                            Key::Attempted => {
                                if attempted_seen {
                                    return Err(serde::de::Error::duplicate_field("attempted_at"));
                                }
                                attempted_seen = true;
                                attempted_at = map.next_value_seed(Scalar::<Option<i64>>(
                                    &mut *self.0,
                                    std::marker::PhantomData,
                                ))?;
                            }
                            _ => {
                                map.next_value_seed(Skip(&mut *self.0))?;
                            }
                        }
                    }
                    let matched =
                        matched.ok_or_else(|| serde::de::Error::missing_field("request"))?;
                    let state = state.ok_or_else(|| serde::de::Error::missing_field("state"))?;
                    let attempts =
                        attempts.ok_or_else(|| serde::de::Error::missing_field("attempts"))?;
                    if matched
                        && matches!(
                            state,
                            ReportBatchDeliveryState::Sending | ReportBatchDeliveryState::Uncertain
                        )
                        && (!(1..=5).contains(&attempts)
                            || !attempted_at.is_some_and(|time| time >= 0))
                    {
                        return Err(serde::de::Error::custom(
                            "invalid attempted delivery metadata",
                        ));
                    }
                    Ok(matched.then_some((state, attempted_at, attempts)))
                }
            }
            deserializer.deserialize_map(Batches(self.0, self.1))
        }
    }
    struct Array<'a, 'b>(&'a mut Budget, &'b str);
    impl<'de> DeserializeSeed<'de> for Array<'_, '_> {
        type Value = Option<Evidence>;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            self.0.tick()?;
            struct Arrays<'a, 'b>(&'a mut Budget, &'b str);
            impl<'de> Visitor<'de> for Arrays<'_, '_> {
                type Value = Option<Evidence>;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("report batch array")
                }
                fn visit_seq<S: SeqAccess<'de>>(
                    self,
                    mut sequence: S,
                ) -> std::result::Result<Self::Value, S::Error> {
                    let mut selected = None;
                    while let Some(value) =
                        sequence.next_element_seed(Batch(&mut *self.0, self.1))?
                    {
                        if let Some(value) = value {
                            if selected.is_some() {
                                return Err(serde::de::Error::custom("ambiguous session identity"));
                            }
                            selected = Some(value);
                        }
                    }
                    Ok(selected)
                }
            }
            deserializer.deserialize_seq(Arrays(self.0, self.1))
        }
    }
    struct Root<'a, 'b>(&'a mut Budget, &'b str);
    impl<'de> DeserializeSeed<'de> for Root<'_, '_> {
        type Value = Option<Evidence>;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            self.0.tick()?;
            struct Roots<'a, 'b>(&'a mut Budget, &'b str);
            impl<'de> Visitor<'de> for Roots<'_, '_> {
                type Value = Option<Evidence>;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("outbox metadata object")
                }
                fn visit_map<M: MapAccess<'de>>(
                    self,
                    mut map: M,
                ) -> std::result::Result<Self::Value, M::Error> {
                    let mut current = None;
                    let mut cursors = false;
                    let mut proofs = false;
                    while let Some(key) = map.next_key_seed(KeySeed(&mut *self.0))? {
                        match key {
                            Key::Batches => {
                                if current.is_some() {
                                    return Err(serde::de::Error::duplicate_field("batches"));
                                }
                                current = Some(map.next_value_seed(Array(&mut *self.0, self.1))?);
                            }
                            Key::Cursors => {
                                if cursors {
                                    return Err(serde::de::Error::duplicate_field(
                                        "reported_cursors",
                                    ));
                                }
                                map.next_value_seed(MapSkip(&mut *self.0))?;
                                cursors = true;
                            }
                            Key::Proofs => {
                                if proofs {
                                    return Err(serde::de::Error::duplicate_field(
                                        "compacted_runs",
                                    ));
                                }
                                map.next_value_seed(MapSkip(&mut *self.0))?;
                                proofs = true;
                            }
                            _ => {
                                map.next_value_seed(Skip(&mut *self.0))?;
                            }
                        }
                    }
                    if !cursors {
                        return Err(serde::de::Error::missing_field("reported_cursors"));
                    }
                    if !proofs {
                        return Err(serde::de::Error::missing_field("compacted_runs"));
                    }
                    current.ok_or_else(|| serde::de::Error::missing_field("batches"))
                }
            }
            deserializer.deserialize_map(Roots(self.0, self.1))
        }
    }
    let mut budget = Budget {
        nodes: 0,
        records: 0,
        denied: None,
    };
    // Begin at a known power-of-two capacity above admitted numeric tokens.
    // This also covers serde's optional float_roundtrip feature, whose initial
    // significand uses a bulk append. The fixed prelude is metadata-priced.
    let mut prime = [b'a'; 258];
    prime[0] = b'"';
    prime[257] = b'"';
    let reader = prime.as_slice().chain(raw.as_bytes());
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    struct Prime;
    impl Visitor<'_> for Prime {
        type Value = ();
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("fixed scratch prime")
        }
        fn visit_str<E: serde::de::Error>(self, _: &str) -> std::result::Result<(), E> {
            Ok(())
        }
    }
    serde::Deserializer::deserialize_str(&mut deserializer, Prime)
        .context("prime report metadata scratch")?;
    let selected = Root(&mut budget, sid).deserialize(&mut deserializer);
    if let Some(resource) = budget.denied {
        return Err(ReportBatchLookupLimit { resource }.into());
    }
    let selected = selected.context("decode report batch delivery metadata")?;
    deserializer
        .end()
        .context("decode report batch delivery metadata")?;
    Ok(selected)
}

fn book_metadata_admit(resource: &'static str, requested: usize, limit: usize) -> Result<()> {
    if requested > limit {
        Err(BookMetadataLimit {
            resource,
            limit,
            requested,
        }
        .into())
    } else {
        Ok(())
    }
}

fn book_metadata_add(left: usize, right: usize) -> Result<usize> {
    left.checked_add(right)
        .ok_or_else(|| BookMetadataUnavailable.into())
}

fn book_metadata_mul(left: usize, right: usize) -> Result<usize> {
    left.checked_mul(right)
        .ok_or_else(|| BookMetadataUnavailable.into())
}

fn book_metadata_text<'a>(row: &'a Row<'_>, column: usize, maximum: usize) -> Result<&'a str> {
    let value = row.get_ref(column)?;
    let text = value.as_str().map_err(|_| BookMetadataUnavailable)?;
    book_metadata_admit("text bytes", text.len(), maximum)?;
    Ok(text)
}

#[derive(Default)]
struct IntroductionFields {
    publisher: Option<String>,
    isbn: Option<String>,
    nodes: usize,
}

fn parse_book_introduction_metadata(
    raw: &str,
    limits: BookIntroductionLimits,
) -> Result<IntroductionFields> {
    use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
    use std::{cell::Cell, fmt};

    struct NodeBudget {
        nodes: usize,
        maximum: usize,
        denied: bool,
        book_info: bool,
        book: bool,
    }
    struct CountSeed<'a>(&'a mut NodeBudget, bool);
    struct CountVisitor<'a>(&'a mut NodeBudget, bool);
    struct CountKey<'a>(&'a mut NodeBudget);
    impl<'de> DeserializeSeed<'de> for CountKey<'_> {
        type Value = Key;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<Key, D::Error> {
            self.0.nodes = self
                .0
                .nodes
                .checked_add(1)
                .ok_or_else(|| serde::de::Error::custom("metadata node count overflow"))?;
            if self.0.nodes > self.0.maximum {
                self.0.denied = true;
                return Err(serde::de::Error::custom("metadata node allowance exceeded"));
            }
            KeySeed.deserialize(deserializer)
        }
    }
    impl<'de> DeserializeSeed<'de> for CountSeed<'_> {
        type Value = ();
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<(), D::Error> {
            self.0.nodes = self
                .0
                .nodes
                .checked_add(1)
                .ok_or_else(|| serde::de::Error::custom("metadata node count overflow"))?;
            if self.0.nodes > self.0.maximum {
                self.0.denied = true;
                return Err(serde::de::Error::custom("metadata node allowance exceeded"));
            }
            deserializer.deserialize_any(CountVisitor(self.0, self.1))
        }
    }
    impl<'de> Visitor<'de> for CountVisitor<'_> {
        type Value = ();
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("bounded metadata JSON")
        }
        fn visit_bool<E: serde::de::Error>(self, _: bool) -> std::result::Result<(), E> {
            Ok(())
        }
        fn visit_i64<E: serde::de::Error>(self, _: i64) -> std::result::Result<(), E> {
            Ok(())
        }
        fn visit_u64<E: serde::de::Error>(self, _: u64) -> std::result::Result<(), E> {
            Ok(())
        }
        fn visit_f64<E: serde::de::Error>(self, _: f64) -> std::result::Result<(), E> {
            Ok(())
        }
        fn visit_str<E: serde::de::Error>(self, _: &str) -> std::result::Result<(), E> {
            Ok(())
        }
        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<(), E> {
            Ok(())
        }
        fn visit_seq<S: SeqAccess<'de>>(
            self,
            mut sequence: S,
        ) -> std::result::Result<(), S::Error> {
            while sequence
                .next_element_seed(CountSeed(&mut *self.0, false))?
                .is_some()
            {}
            Ok(())
        }
        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> std::result::Result<(), M::Error> {
            while let Some(key) = map.next_key_seed(CountKey(&mut *self.0))? {
                if self.1 {
                    match key {
                        Key::BookInfo => self.0.book_info = true,
                        Key::Book => self.0.book = true,
                        _ => {}
                    }
                }
                map.next_value_seed(CountSeed(&mut *self.0, false))?;
            }
            Ok(())
        }
    }
    let mut budget = NodeBudget {
        nodes: 0,
        maximum: limits.metadata_nodes,
        denied: false,
        book_info: false,
        book: false,
    };
    let mut deserializer = serde_json::Deserializer::from_str(raw);
    let counted = CountSeed(&mut budget, true).deserialize(&mut deserializer);
    if budget.denied {
        return Err(BookMetadataLimit {
            resource: "introduction metadata nodes",
            limit: limits.metadata_nodes,
            requested: budget.nodes,
        }
        .into());
    }
    counted.context("decode introduction metadata")?;
    deserializer.end().context("decode introduction metadata")?;

    enum Key {
        Publisher,
        Isbn,
        BookInfo,
        Book,
        Other,
    }
    struct KeySeed;
    impl<'de> DeserializeSeed<'de> for KeySeed {
        type Value = Key;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<Key, D::Error> {
            struct KeyVisitor;
            impl Visitor<'_> for KeyVisitor {
                type Value = Key;
                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("metadata field name")
                }
                fn visit_str<E: serde::de::Error>(self, key: &str) -> std::result::Result<Key, E> {
                    Ok(match key {
                        "publisher" => Key::Publisher,
                        "isbn" => Key::Isbn,
                        "bookInfo" => Key::BookInfo,
                        "book" => Key::Book,
                        _ => Key::Other,
                    })
                }
            }
            deserializer.deserialize_identifier(KeyVisitor)
        }
    }
    struct TextSeed<'a> {
        maximum: usize,
        denied: &'a Cell<Option<usize>>,
    }
    impl<'de> DeserializeSeed<'de> for TextSeed<'_> {
        type Value = Option<String>;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            struct TextVisitor<'a>(TextSeed<'a>);
            impl<'de> Visitor<'de> for TextVisitor<'_> {
                type Value = Option<String>;
                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("optional bounded metadata text")
                }
                fn visit_str<E: serde::de::Error>(
                    self,
                    text: &str,
                ) -> std::result::Result<Self::Value, E> {
                    if text.len() > self.0.maximum {
                        self.0.denied.set(Some(text.len()));
                        return Err(serde::de::Error::custom(
                            "metadata field allowance exceeded",
                        ));
                    }
                    let text = text.trim();
                    Ok((!text.is_empty()).then(|| text.to_owned()))
                }
                fn visit_bool<E: serde::de::Error>(
                    self,
                    _: bool,
                ) -> std::result::Result<Self::Value, E> {
                    Ok(None)
                }
                fn visit_i64<E: serde::de::Error>(
                    self,
                    _: i64,
                ) -> std::result::Result<Self::Value, E> {
                    Ok(None)
                }
                fn visit_u64<E: serde::de::Error>(
                    self,
                    _: u64,
                ) -> std::result::Result<Self::Value, E> {
                    Ok(None)
                }
                fn visit_f64<E: serde::de::Error>(
                    self,
                    _: f64,
                ) -> std::result::Result<Self::Value, E> {
                    Ok(None)
                }
                fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                    Ok(None)
                }
                fn visit_seq<S: SeqAccess<'de>>(
                    self,
                    mut sequence: S,
                ) -> std::result::Result<Self::Value, S::Error> {
                    while sequence.next_element::<IgnoredAny>()?.is_some() {}
                    Ok(None)
                }
                fn visit_map<M: MapAccess<'de>>(
                    self,
                    mut map: M,
                ) -> std::result::Result<Self::Value, M::Error> {
                    while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                    Ok(None)
                }
            }
            deserializer.deserialize_any(TextVisitor(self))
        }
    }
    struct FieldsSeed<'a> {
        root: bool,
        selection: u8,
        maximum: usize,
        denied: &'a Cell<Option<usize>>,
    }
    impl<'de> DeserializeSeed<'de> for FieldsSeed<'_> {
        type Value = IntroductionFields;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            struct FieldsVisitor<'a>(FieldsSeed<'a>);
            impl<'de> Visitor<'de> for FieldsVisitor<'_> {
                type Value = IntroductionFields;
                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("metadata fields")
                }
                fn visit_map<M: MapAccess<'de>>(
                    self,
                    mut map: M,
                ) -> std::result::Result<Self::Value, M::Error> {
                    let mut fields = IntroductionFields::default();
                    let mut book_info = None;
                    let mut book = None;
                    while let Some(key) = map.next_key_seed(KeySeed)? {
                        match key {
                            Key::Publisher if !self.0.root || self.0.selection == 0 => {
                                fields.publisher = map.next_value_seed(TextSeed {
                                    maximum: self.0.maximum,
                                    denied: self.0.denied,
                                })?
                            }
                            Key::Isbn if !self.0.root || self.0.selection == 0 => {
                                fields.isbn = map.next_value_seed(TextSeed {
                                    maximum: self.0.maximum,
                                    denied: self.0.denied,
                                })?
                            }
                            Key::BookInfo if self.0.root && self.0.selection == 1 => {
                                book_info = Some(map.next_value_seed(FieldsSeed {
                                    root: false,
                                    selection: self.0.selection,
                                    maximum: self.0.maximum,
                                    denied: self.0.denied,
                                })?)
                            }
                            Key::Book if self.0.root && self.0.selection == 2 => {
                                book = Some(map.next_value_seed(FieldsSeed {
                                    root: false,
                                    selection: self.0.selection,
                                    maximum: self.0.maximum,
                                    denied: self.0.denied,
                                })?)
                            }
                            _ => {
                                map.next_value::<IgnoredAny>()?;
                            }
                        }
                    }
                    // Match Value's last duplicate key and bookInfo/book/root
                    // precedence, including null/scalar wrapper shadowing.
                    Ok(book_info.or(book).unwrap_or(fields))
                }
                fn visit_seq<S: SeqAccess<'de>>(
                    self,
                    mut sequence: S,
                ) -> std::result::Result<Self::Value, S::Error> {
                    while sequence.next_element::<IgnoredAny>()?.is_some() {}
                    Ok(IntroductionFields::default())
                }
                fn visit_bool<E: serde::de::Error>(
                    self,
                    _: bool,
                ) -> std::result::Result<Self::Value, E> {
                    Ok(IntroductionFields::default())
                }
                fn visit_i64<E: serde::de::Error>(
                    self,
                    _: i64,
                ) -> std::result::Result<Self::Value, E> {
                    Ok(IntroductionFields::default())
                }
                fn visit_u64<E: serde::de::Error>(
                    self,
                    _: u64,
                ) -> std::result::Result<Self::Value, E> {
                    Ok(IntroductionFields::default())
                }
                fn visit_f64<E: serde::de::Error>(
                    self,
                    _: f64,
                ) -> std::result::Result<Self::Value, E> {
                    Ok(IntroductionFields::default())
                }
                fn visit_str<E: serde::de::Error>(
                    self,
                    _: &str,
                ) -> std::result::Result<Self::Value, E> {
                    Ok(IntroductionFields::default())
                }
                fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                    Ok(IntroductionFields::default())
                }
            }
            deserializer.deserialize_any(FieldsVisitor(self))
        }
    }
    let denied = Cell::new(None);
    let mut deserializer = serde_json::Deserializer::from_str(raw);
    let selection = if budget.book_info {
        1
    } else if budget.book {
        2
    } else {
        0
    };
    let fields = FieldsSeed {
        root: true,
        selection,
        maximum: limits.field_bytes,
        denied: &denied,
    }
    .deserialize(&mut deserializer);
    if let Some(requested) = denied.get() {
        return Err(BookMetadataLimit {
            resource: "introduction metadata field bytes",
            limit: limits.field_bytes,
            requested,
        }
        .into());
    }
    let mut fields = fields.context("decode introduction metadata fields")?;
    deserializer
        .end()
        .context("decode introduction metadata fields")?;
    fields.nodes = budget.nodes;
    Ok(fields)
}

fn book_metadata_preflight(
    connection: &Connection,
    encoding: TextEncoding,
    id: Option<&str>,
    statistics: bool,
    output_size: usize,
    scalar_size: usize,
    limits: BookMetadataLimits,
) -> Result<usize> {
    let sql = if statistics {
        "SELECT COUNT(*),COALESCE(SUM(COALESCE(octet_length(id),0)+COALESCE(octet_length(title),0)+COALESCE(octet_length(category),0)),0),
         COALESCE(SUM((COALESCE(octet_length(id),0)>0)+(COALESCE(octet_length(title),0)>0)+(COALESCE(octet_length(category),0)>0)),0),
         COALESCE(MAX(COALESCE(octet_length(id),0)),0),COALESCE(MIN(typeof(id)='text' AND typeof(title)='text' AND typeof(category)='text' AND typeof(finished)='integer'),1)
         FROM books NOT INDEXED WHERE (?1 IS NULL OR id=?1)"
    } else {
        "SELECT COUNT(*),COALESCE(SUM(COALESCE(octet_length(id),0)+COALESCE(octet_length(title),0)+COALESCE(octet_length(author),0)
          +COALESCE(octet_length(category),0)+COALESCE(octet_length(cover_url),0)+COALESCE(octet_length(source),0)),0),
         COALESCE(SUM((COALESCE(octet_length(id),0)>0)+(COALESCE(octet_length(title),0)>0)+(COALESCE(octet_length(author),0)>0)
          +(COALESCE(octet_length(category),0)>0)+(COALESCE(octet_length(cover_url),0)>0)+(COALESCE(octet_length(source),0)>0)),0),
         COALESCE(MAX(COALESCE(octet_length(id),0)),0),COALESCE(MIN(typeof(id)='text' AND typeof(title)='text' AND typeof(author)='text'
          AND typeof(category)='text' AND typeof(cover_url)='text' AND typeof(source)='text'
          AND typeof(progress) IN ('real','integer') AND typeof(total_chapters)='integer' AND typeof(cached_chapters)='integer'
          AND typeof(updated)='integer' AND typeof(finished)='integer' AND typeof(on_shelf)='integer'),1)
         FROM books NOT INDEXED WHERE (?1 IS NULL OR id=?1)"
    };
    let (count, bytes, cells, id_bytes, valid): (i64, i64, i64, i64, bool) =
        connection.query_row(sql, [id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?;
    if !valid {
        return Err(BookMetadataUnavailable.into());
    }
    let count = usize::try_from(count).map_err(|_| BookMetadataUnavailable)?;
    book_metadata_admit("rows", count, limits.rows)?;
    let projected = if statistics { 5 } else { 13 };
    book_metadata_admit(
        "SQL cells",
        book_metadata_add(5, book_metadata_mul(count, projected)?)?,
        limits.sql_cells,
    )?;
    let id_bytes = encoding
        .utf8_upper(usize::try_from(id_bytes).map_err(|_| BookMetadataUnavailable)?)
        .map_err(|_| BookMetadataUnavailable)?;
    book_metadata_admit("identifier bytes", id_bytes, limits.identifier_bytes)?;
    let bytes = encoding
        .utf8_upper_total(
            usize::try_from(bytes).map_err(|_| BookMetadataUnavailable)?,
            usize::try_from(cells).map_err(|_| BookMetadataUnavailable)?,
        )
        .map_err(|_| BookMetadataUnavailable)?;
    let fixed = book_metadata_mul(
        count,
        book_metadata_mul(2, book_metadata_add(output_size, scalar_size)?)?,
    )?;
    book_metadata_admit(
        "working-set bytes",
        book_metadata_add(bytes, fixed)?,
        limits.bytes,
    )?;
    Ok(count)
}

struct BookScalarMetadata {
    rowid: i64,
    progress: f32,
    total: u32,
    cached: u32,
    updated: bool,
    finished: bool,
    on_shelf: bool,
}

fn book_metadata_on(
    connection: &Connection,
    encoding: TextEncoding,
    id: Option<&str>,
    limits: BookMetadataLimits,
) -> Result<Vec<BookMetadata>> {
    let count = book_metadata_preflight(
        connection,
        encoding,
        id,
        false,
        std::mem::size_of::<BookMetadata>(),
        std::mem::size_of::<BookScalarMetadata>(),
        limits,
    )?;
    let mut scalars = Vec::with_capacity(count);
    {
        let mut statement = connection.prepare("SELECT rowid,CASE WHEN typeof(progress) IN ('real','integer') THEN progress END,
            CASE WHEN typeof(total_chapters)='integer' THEN total_chapters END,CASE WHEN typeof(cached_chapters)='integer' THEN cached_chapters END,
            CASE WHEN typeof(updated)='integer' THEN updated END,CASE WHEN typeof(finished)='integer' THEN finished END,
            CASE WHEN typeof(on_shelf)='integer' THEN on_shelf END FROM books WHERE (?1 IS NULL OR id=?1) ORDER BY rowid DESC")?;
        let mut rows = statement.query([id])?;
        while let Some(row) = rows.next()? {
            let value = BookScalarMetadata {
                rowid: row.get(0).map_err(|_| BookMetadataUnavailable)?,
                progress: row.get(1).map_err(|_| BookMetadataUnavailable)?,
                total: row.get(2).map_err(|_| BookMetadataUnavailable)?,
                cached: row.get(3).map_err(|_| BookMetadataUnavailable)?,
                updated: row.get(4).map_err(|_| BookMetadataUnavailable)?,
                finished: row.get(5).map_err(|_| BookMetadataUnavailable)?,
                on_shelf: row.get(6).map_err(|_| BookMetadataUnavailable)?,
            };
            if !value.progress.is_finite() || !(0.0..=1.0).contains(&value.progress) {
                return Err(BookMetadataUnavailable.into());
            }
            scalars.push(value);
        }
    }
    #[cfg(test)]
    book_metadata_tests::after_preflight()?;
    let mut books = Vec::with_capacity(count);
    for scalar in scalars {
        let mut statement = connection.prepare(
            "SELECT id,title,author,category,cover_url,source FROM books WHERE rowid=?1",
        )?;
        let mut rows = statement.query([scalar.rowid])?;
        let row = rows.next()?.ok_or(BookMetadataUnavailable)?;
        let id = book_metadata_text(row, 0, limits.identifier_bytes)?;
        let title = book_metadata_text(row, 1, limits.bytes)?;
        let author = book_metadata_text(row, 2, limits.bytes)?;
        let category = book_metadata_text(row, 3, limits.bytes)?;
        let cover = book_metadata_text(row, 4, limits.bytes)?;
        let source = book_metadata_text(row, 5, limits.bytes)?;
        #[cfg(test)]
        {
            let mut bytes = 0;
            for value in [id, title, author, category, cover, source] {
                bytes = book_metadata_add(bytes, value.len())?;
            }
            book_metadata_copied(bytes);
        }
        books.push(BookMetadata {
            id: id.to_owned(),
            title: title.to_owned(),
            author: author.to_owned(),
            category: category.to_owned(),
            cover_url: cover.to_owned(),
            source: source.to_owned(),
            progress: scalar.progress,
            total_chapters: scalar.total,
            cached_chapters: scalar.cached,
            updated: scalar.updated,
            finished: scalar.finished,
            on_shelf: scalar.on_shelf,
        });
    }
    Ok(books)
}

fn book_statistics_metadata_on(
    connection: &Connection,
    encoding: TextEncoding,
    limits: BookMetadataLimits,
) -> Result<Vec<BookStatisticsMetadata>> {
    let count = book_metadata_preflight(
        connection,
        encoding,
        None,
        true,
        std::mem::size_of::<BookStatisticsMetadata>(),
        std::mem::size_of::<(i64, bool)>(),
        limits,
    )?;
    let mut scalars = Vec::with_capacity(count);
    {
        let mut statement =
            connection.prepare("SELECT rowid,CASE WHEN typeof(finished)='integer' THEN finished END FROM books ORDER BY rowid DESC")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            scalars.push((
                row.get::<_, i64>(0).map_err(|_| BookMetadataUnavailable)?,
                row.get::<_, bool>(1).map_err(|_| BookMetadataUnavailable)?,
            ));
        }
    }
    #[cfg(test)]
    book_metadata_tests::after_preflight()?;
    let mut books = Vec::with_capacity(count);
    for (rowid, finished) in scalars {
        let mut statement =
            connection.prepare("SELECT id,title,category FROM books WHERE rowid=?1")?;
        let mut rows = statement.query([rowid])?;
        let row = rows.next()?.ok_or(BookMetadataUnavailable)?;
        let id = book_metadata_text(row, 0, limits.identifier_bytes)?;
        let title = book_metadata_text(row, 1, limits.bytes)?;
        let category = book_metadata_text(row, 2, limits.bytes)?;
        #[cfg(test)]
        book_metadata_copied(book_metadata_add(
            book_metadata_add(id.len(), title.len())?,
            category.len(),
        )?);
        books.push(BookStatisticsMetadata {
            id: id.to_owned(),
            title: title.to_owned(),
            category: category.to_owned(),
            finished,
        });
    }
    Ok(books)
}

fn annotation_admit(resource: &'static str, requested: usize, limit: usize) -> Result<()> {
    if requested > limit {
        return Err(AnnotationResourceLimit {
            resource,
            limit,
            requested,
        }
        .into());
    }
    Ok(())
}

fn annotation_input_id(id: &str, maximum: usize) -> Result<()> {
    annotation_admit("identifier bytes", id.len(), maximum)?;
    validate_identifier(id)
}

fn annotation_sum(left: usize, right: usize) -> Result<usize> {
    left.checked_add(right)
        .ok_or_else(|| AnnotationUnavailable.into())
}

fn annotation_product(left: usize, right: usize) -> Result<usize> {
    left.checked_mul(right)
        .ok_or_else(|| AnnotationUnavailable.into())
}

fn annotation_encoded(row: &Row<'_>, column: usize, encoding: TextEncoding) -> Result<usize> {
    let bytes = usize::try_from(row.get::<_, i64>(column)?).map_err(|_| AnnotationUnavailable)?;
    encoding
        .utf8_upper(bytes)
        .map_err(|_| AnnotationUnavailable.into())
}

fn annotation_text<'a>(row: &'a Row<'_>, column: usize, maximum: usize) -> Result<&'a str> {
    let value = row.get_ref(column)?;
    let text = value.as_str().map_err(|_| AnnotationUnavailable)?;
    annotation_admit("text bytes", text.len(), maximum)?;
    Ok(text)
}

fn cache_text<'a>(row: &'a Row<'_>, column: usize, maximum: usize) -> Result<&'a str> {
    let value = row.get_ref(column)?;
    let text = value.as_str().map_err(|_| CachedResourceLimit)?;
    if text.len() > maximum {
        return Err(CachedResourceLimit.into());
    }
    Ok(text)
}

fn inventory_sum(terms: &[usize]) -> Result<usize> {
    terms.iter().try_fold(0usize, |sum, value| {
        sum.checked_add(*value).ok_or_else(|| {
            StoreInventoryLimit {
                resource: "arithmetic",
            }
            .into()
        })
    })
}

fn position_metadata_on(
    connection: &Connection,
    encoding: TextEncoding,
    book_id: &str,
) -> Result<Option<ReadingPosition>> {
    selected_chapter_id(book_id)?;
    let header:Option<(i64,bool,i64,i64)>=connection.query_row(
        "SELECT rowid,typeof(book_id)='text' AND typeof(device)='text' AND typeof(chapter_index)='integer' AND typeof(char_offset)='integer' AND typeof(updated_at)='integer',COALESCE(octet_length(book_id),0),COALESCE(octet_length(device),0) FROM positions WHERE book_id=?1",
        [book_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).optional()?;
    let Some((rowid, valid, book_raw, device_raw)) = header else {
        return Ok(None);
    };
    if !valid {
        return Err(CachedResourceLimit.into());
    }
    let book_raw = usize::try_from(book_raw).map_err(|_| CachedResourceLimit)?;
    let device_raw = usize::try_from(device_raw).map_err(|_| CachedResourceLimit)?;
    let book = encoding.utf8_upper(book_raw)?;
    let device = encoding.utf8_upper(device_raw)?;
    let held = selected_cached_bytes(&[book, device, 2 * std::mem::size_of::<ReadingPosition>()])?;
    let limits = store_inventory_limits();
    selected_cached_admit(held, limits.metadata_bytes)?;
    selected_cached_admit(
        selected_cached_bytes(&[
            held,
            inventory_text_vm_scratch(encoding, book_raw)?,
            inventory_text_vm_scratch(encoding, device_raw)?,
        ])?,
        limits.scratch_bytes,
    )?;
    selected_cached_admit(
        selected_cached_bytes(&[book, device])?
            .checked_mul(2)
            .ok_or(CachedResourceLimit)?,
        limits.work_bytes,
    )?;
    let mut statement=connection.prepare("SELECT book_id,CASE WHEN typeof(chapter_index)='integer' THEN chapter_index END,CASE WHEN typeof(char_offset)='integer' THEN char_offset END,CASE WHEN typeof(updated_at)='integer' THEN updated_at END,device FROM positions WHERE rowid=?1")?;
    let mut rows = statement.query([rowid])?;
    let row = rows.next()?.ok_or(CachedResourceLimit)?;
    let stored_book = cache_text(row, 0, book)?;
    let stored_device = cache_text(row, 4, device)?;
    if stored_book != book_id {
        return Err(CachedResourceLimit.into());
    }
    Ok(Some(ReadingPosition {
        book_id: stored_book.to_owned(),
        chapter_index: row.get(1).map_err(|_| CachedResourceLimit)?,
        char_offset: row.get(2).map_err(|_| CachedResourceLimit)?,
        updated_at: row.get(3).map_err(|_| CachedResourceLimit)?,
        device: stored_device.to_owned(),
    }))
}

fn save_annotation_on(
    connection: &Connection,
    encoding: TextEncoding,
    annotation: &Annotation,
) -> Result<()> {
    let body: Option<(i64, bool, bool, i64)> = connection
        .query_row(
            "SELECT rowid,body IS NOT NULL,typeof(body)='text',COALESCE(octet_length(body),0)
        FROM chapters WHERE book_id=?1 AND chapter_index=?2",
            params![annotation.book_id, annotation.chapter_index],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    if let Some((rowid, true, text, raw)) = body {
        if !text {
            return Err(StoreInventoryUnavailable.into());
        }
        let raw = usize::try_from(raw).map_err(|_| StoreInventoryUnavailable)?;
        let upper = encoding
            .utf8_upper(raw)
            .map_err(|_| StoreInventoryUnavailable)?;
        let limits = store_inventory_limits();
        inventory_admit(
            "annotation body work",
            upper.checked_mul(2).ok_or(StoreInventoryLimit {
                resource: "work bytes",
            })?,
            limits.work_bytes,
        )?;
        inventory_admit(
            "annotation body scratch",
            inventory_text_vm_scratch(encoding, raw)?,
            limits.scratch_bytes,
        )?;
        let mut statement = connection.prepare("SELECT body FROM chapters WHERE rowid=?1")?;
        let mut rows = statement.query([rowid])?;
        let row = rows.next()?.ok_or(StoreInventoryUnavailable)?;
        let body = row
            .get_ref(0)?
            .as_str()
            .map_err(|_| StoreInventoryUnavailable)?;
        inventory_admit("annotation body bytes", body.len(), upper)?;
        #[cfg(test)]
        annotation_body_budget_tests::observe_body_borrow(body.len());
        let end = usize::try_from(annotation.end).map_err(|_| StoreInventoryUnavailable)?;
        ensure!(
            end <= body.chars().count(),
            "annotation exceeds chapter body"
        );
    }
    connection.execute("INSERT INTO annotations (id, book_id, chapter_index, start_offset, end_offset, quote, note, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(id) DO UPDATE SET book_id=excluded.book_id, chapter_index=excluded.chapter_index, start_offset=excluded.start_offset, end_offset=excluded.end_offset, quote=excluded.quote, note=excluded.note, created_at=excluded.created_at",params![annotation.id,annotation.book_id,annotation.chapter_index,annotation.start,annotation.end,annotation.quote,annotation.note,annotation.created_at])?;
    Ok(())
}

fn bookmark_sum(terms: &[usize]) -> Result<usize> {
    terms.iter().try_fold(0usize, |sum, v| {
        sum.checked_add(*v).ok_or_else(|| {
            BookmarkResourceLimit {
                resource: "arithmetic",
            }
            .into()
        })
    })
}
fn bookmark_admit(resource: &'static str, n: usize, max: usize) -> Result<()> {
    if n > max {
        Err(BookmarkResourceLimit { resource }.into())
    } else {
        Ok(())
    }
}
fn bookmark_input_id(id: &str) -> Result<()> {
    bookmark_admit("identifier bytes", id.len(), bookmark_limits().id_bytes)?;
    validate_identifier(id).map_err(|_| BookmarkUnavailable.into())
}
fn bookmark_vm_scratch(encoding: TextEncoding, raw: usize) -> Result<usize> {
    match encoding {
        TextEncoding::Utf8 => raw.checked_add(2),
        TextEncoding::Utf16 => raw.checked_mul(3).and_then(|v| v.checked_add(2)),
    }
    .ok_or_else(|| {
        BookmarkResourceLimit {
            resource: "text scratch",
        }
        .into()
    })
}
#[derive(Clone, Copy)]
struct BookmarkTextPrice {
    upper: usize,
    vm: usize,
}
fn bookmark_geometries_on(
    c: &Connection,
    book: &str,
    range: Option<(u32, u32, u32)>,
    offset: usize,
    rows: usize,
) -> Result<Vec<BookmarkGeometry>> {
    let l = bookmark_limits();
    bookmark_admit("geometry rows", rows, l.rows)?;
    // Raw sort keys must also be numeric before the ordered query can run.
    // typeof observes serial types without owning malformed field payloads.
    let valid:bool=c.query_row("SELECT COALESCE(MIN(typeof(chapter_index)='integer' AND typeof(char_offset)='integer' AND typeof(created_at)='integer'),1) FROM bookmarks NOT INDEXED WHERE book_id=?1",
        [book],|row|row.get(0))?;
    if !valid {
        return Err(BookmarkUnavailable.into());
    }
    let limit = rows
        .checked_add(1)
        .ok_or(BookmarkResourceLimit { resource: "rows" })?;
    let mut stmt=c.prepare("SELECT rowid,CASE WHEN typeof(chapter_index)='integer' THEN chapter_index END,
        CASE WHEN typeof(char_offset)='integer' THEN char_offset END,CASE WHEN typeof(created_at)='integer' THEN created_at END FROM bookmarks WHERE book_id=?1
        AND (?2 IS NULL OR (chapter_index=?2 AND char_offset>=?3 AND char_offset<?4)) ORDER BY chapter_index,char_offset,rowid LIMIT ?5 OFFSET ?6")?;
    let mut source = stmt.query(params![
        book,
        range.map(|r| r.0),
        range.map(|r| r.1),
        range.map(|r| r.2),
        i64::try_from(limit).map_err(|_| BookmarkUnavailable)?,
        i64::try_from(offset).map_err(|_| BookmarkUnavailable)?
    ])?;
    let mut out = Vec::new();
    while let Some(row) = source.next()? {
        if out.len() == rows {
            if range.is_some() {
                return Err(BookmarkResourceLimit {
                    resource: "geometry rows",
                }
                .into());
            } else {
                break;
            }
        }
        bookmark_admit(
            "geometry metadata",
            out.len()
                .checked_add(1)
                .and_then(|n| n.checked_mul(2 * std::mem::size_of::<BookmarkGeometry>()))
                .ok_or(BookmarkResourceLimit {
                    resource: "metadata",
                })?,
            l.metadata_bytes,
        )?;
        out.push(BookmarkGeometry {
            rowid: row.get(0)?,
            chapter_index: row.get(1).map_err(|_| BookmarkUnavailable)?,
            char_offset: row.get(2).map_err(|_| BookmarkUnavailable)?,
            created_at: row.get(3).map_err(|_| BookmarkUnavailable)?,
        });
    }
    Ok(out)
}
fn bookmark_label_metadata_on(
    c: &Connection,
    e: TextEncoding,
    book: &str,
    h: &BookmarkHeader,
) -> Result<Option<BookmarkTextPrice>> {
    bookmark_input_id(&h.key.id)?;
    let m:Option<(bool,i64)>=c.query_row("SELECT typeof(label)='text',COALESCE(octet_length(label),0) FROM bookmarks WHERE rowid=?1
        AND book_id=?2 AND id=?3 AND chapter_index=?4 AND char_offset=?5 AND created_at=?6",
        params![h.key.rowid,book,h.key.id,h.chapter_index,h.char_offset,h.created_at],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((text, raw)) = m else {
        return Ok(None);
    };
    if !text {
        return Err(BookmarkUnavailable.into());
    }
    let raw = usize::try_from(raw).map_err(|_| BookmarkUnavailable)?;
    Ok(Some(BookmarkTextPrice {
        upper: e.utf8_upper(raw).map_err(|_| BookmarkUnavailable)?,
        vm: bookmark_vm_scratch(e, raw)?,
    }))
}
fn bookmark_label_on(c: &Connection, book: &str, h: &BookmarkHeader, max: usize) -> Result<String> {
    let mut stmt=c.prepare("SELECT label FROM bookmarks WHERE rowid=?1 AND book_id=?2 AND id=?3 AND chapter_index=?4 AND char_offset=?5 AND created_at=?6")?;
    let mut rows = stmt.query(params![
        h.key.rowid,
        book,
        h.key.id,
        h.chapter_index,
        h.char_offset,
        h.created_at
    ])?;
    let row = rows.next()?.ok_or(BookmarkUnavailable)?;
    let label = row.get_ref(0)?.as_str().map_err(|_| BookmarkUnavailable)?;
    bookmark_admit("label bytes", label.len(), max)?;
    #[cfg(test)]
    bookmark_budget_tests::observe_label(label.len());
    Ok(label.to_owned())
}
fn inventory_admit(resource: &'static str, requested: usize, limit: usize) -> Result<()> {
    if requested > limit {
        Err(StoreInventoryLimit { resource }.into())
    } else {
        Ok(())
    }
}
fn inventory_text_vm_scratch(encoding: TextEncoding, raw: usize) -> Result<usize> {
    match encoding {
        TextEncoding::Utf8 => raw.checked_add(2),
        TextEncoding::Utf16 => raw.checked_mul(3).and_then(|v| v.checked_add(2)),
    }
    .ok_or_else(|| {
        StoreInventoryLimit {
            resource: "scratch bytes",
        }
        .into()
    })
}
fn inventory_vm_scratch(encoding: TextEncoding, raw: usize, upper: usize) -> Result<usize> {
    let vm = inventory_text_vm_scratch(encoding, raw)?;
    inventory_sum(&[
        vm,
        upper.checked_mul(2).ok_or(StoreInventoryLimit {
            resource: "scratch bytes",
        })?,
        256,
    ])
}

struct InventoryPrefixPrice {
    encoded_bytes: usize,
    work_bytes: usize,
    scratch_bytes: usize,
}
fn inventory_prefix_preflight(
    connection: &Connection,
    encoding: TextEncoding,
    prefix: &str,
    limits: StoreInventoryLimits,
) -> Result<InventoryPrefixPrice> {
    inventory_admit("prefix bytes", prefix.len(), 32 * 1024)?;
    validate_identifier(prefix)?;
    let (count,bytes,cells,largest,valid):(i64,i64,i64,i64,bool)=connection.query_row("SELECT COUNT(*),COALESCE(SUM(bytes),0),COALESCE(SUM(bytes>0),0),COALESCE(MAX(bytes),0),COALESCE(MIN(valid),1)
        FROM (SELECT COALESCE(octet_length(key),0) AS bytes,typeof(key)='text' AS valid FROM settings LIMIT ?1)",
        [i64::try_from(limits.rows.checked_add(1).ok_or(StoreInventoryLimit{resource:"rows"})?).map_err(|_|StoreInventoryLimit{resource:"rows"})?],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    if !valid {
        return Err(StoreInventoryUnavailable.into());
    }
    let count = usize::try_from(count).map_err(|_| StoreInventoryUnavailable)?;
    inventory_admit("key inventory rows", count, limits.rows)?;
    let encoded_bytes = usize::try_from(bytes).map_err(|_| StoreInventoryUnavailable)?;
    inventory_admit(
        "key inventory encoded bytes",
        encoded_bytes,
        limits.encoded_bytes,
    )?;
    let upper = encoding
        .utf8_upper_total(
            encoded_bytes,
            usize::try_from(cells).map_err(|_| StoreInventoryUnavailable)?,
        )
        .map_err(|_| StoreInventoryUnavailable)?;
    let largest = usize::try_from(largest).map_err(|_| StoreInventoryUnavailable)?;
    let largest_upper = encoding
        .utf8_upper(largest)
        .map_err(|_| StoreInventoryUnavailable)?;
    let work_bytes = inventory_sum(&[
        upper.checked_mul(6).ok_or(StoreInventoryLimit {
            resource: "work bytes",
        })?,
        count.checked_mul(16).ok_or(StoreInventoryLimit {
            resource: "work bytes",
        })?,
        prefix.len().checked_mul(2).ok_or(StoreInventoryLimit {
            resource: "work bytes",
        })?,
    ])?;
    // The predicate can convert and copy an unrelated key before any matched
    // header exists. Its largest VM/substring phase remains reserved later.
    let scratch_bytes = inventory_sum(&[
        inventory_text_vm_scratch(encoding, largest)?,
        largest_upper,
        prefix.len(),
        256,
    ])?;
    inventory_admit("prefix work bytes", work_bytes, limits.work_bytes)?;
    inventory_admit("prefix scratch bytes", scratch_bytes, limits.scratch_bytes)?;
    Ok(InventoryPrefixPrice {
        encoded_bytes,
        work_bytes,
        scratch_bytes,
    })
}

fn inventory_manifest_image_count(raw: &str, maximum_images: usize) -> Result<usize> {
    let count = borrowed_manifest_image_count(raw, usize::MAX).map_err(|error| {
        if error.downcast_ref::<CachedResourceLimit>().is_some() {
            StoreInventoryUnavailable.into()
        } else {
            error
        }
    })?;
    inventory_admit(
        "manifest images",
        count,
        maximum_images.min(store_inventory_limits().rows),
    )?;
    Ok(count)
}

fn with_inventory_manifest_text_on<T>(
    connection: &Connection,
    encoding: TextEncoding,
    header: &SettingInventoryHeader,
    maximum_bytes: usize,
    passes: usize,
    read: impl FnOnce(&str) -> Result<T>,
) -> Result<T> {
    let limits = store_inventory_limits();
    let metadata:Option<(bool,i64)>=connection.query_row("SELECT typeof(value)='text',COALESCE(octet_length(value),0) FROM settings WHERE rowid=?1",
        [header.rowid],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    let (text, raw) = metadata.ok_or(StoreInventoryUnavailable)?;
    let raw = usize::try_from(raw).map_err(|_| StoreInventoryUnavailable)?;
    if !text || raw != header.value_encoded_bytes {
        return Err(StoreInventoryUnavailable.into());
    }
    let upper = encoding
        .utf8_upper(raw)
        .map_err(|_| StoreInventoryUnavailable)?;
    inventory_admit("manifest encoded bytes", raw, limits.encoded_bytes)?;
    inventory_admit(
        "manifest bytes",
        upper,
        maximum_bytes.min(limits.metadata_bytes),
    )?;
    inventory_admit(
        "manifest work",
        upper.checked_mul(passes).ok_or(StoreInventoryLimit {
            resource: "work bytes",
        })?,
        limits.work_bytes,
    )?;
    inventory_admit(
        "manifest scratch",
        inventory_vm_scratch(encoding, raw, upper)?,
        limits.scratch_bytes,
    )?;
    let mut statement = connection.prepare("SELECT value FROM settings WHERE rowid=?1")?;
    let mut rows = statement.query([header.rowid])?;
    let row = rows.next()?.ok_or(StoreInventoryUnavailable)?;
    let raw = row
        .get_ref(0)?
        .as_str()
        .map_err(|_| StoreInventoryUnavailable)?;
    inventory_admit("manifest bytes", raw.len(), maximum_bytes)?;
    #[cfg(test)]
    inventory_budget_tests::observe_manifest_borrow(raw.len());
    read(raw)
}

struct TaskInventoryHeader {
    rowid: i64,
    raw: usize,
    upper: usize,
    updated_at: i64,
}

fn query_task_inventory_on(
    connection: &Connection,
    encoding: TextEncoding,
    sql: &str,
    owner: Option<&str>,
) -> Result<Vec<DownloadTask>> {
    let limits = store_inventory_limits();
    // Do not ask SQLite to sort a malformed timestamp before type/byte gates.
    // These are the complete, fixed internal query scopes used by Store.
    let (header_sql, ordered) = match sql {
        "SELECT payload FROM download_tasks ORDER BY updated_at, rowid" => (
            "SELECT rowid,typeof(payload)='text',COALESCE(octet_length(payload),0),CASE WHEN typeof(updated_at)='integer' THEN updated_at END FROM download_tasks NOT INDEXED LIMIT ?1",
            true,
        ),
        "SELECT payload FROM download_tasks WHERE status IN ('pending','running','paused') ORDER BY updated_at, rowid" => {
            (
                "SELECT rowid,typeof(payload)='text',COALESCE(octet_length(payload),0),CASE WHEN typeof(updated_at)='integer' THEN updated_at END FROM download_tasks NOT INDEXED WHERE status IN ('pending','running','paused') LIMIT ?1",
                true,
            )
        }
        "SELECT payload FROM download_tasks WHERE status = 'running'" => (
            "SELECT rowid,typeof(payload)='text',COALESCE(octet_length(payload),0),CASE WHEN typeof(updated_at)='integer' THEN updated_at END FROM download_tasks NOT INDEXED WHERE status = 'running' LIMIT ?1",
            false,
        ),
        _ => bail!("unsupported internal task inventory scope"),
    };
    let limit = i64::try_from(
        limits
            .rows
            .checked_add(1)
            .ok_or(StoreInventoryLimit { resource: "rows" })?,
    )
    .map_err(|_| StoreInventoryLimit { resource: "rows" })?;
    task_inventory_core_on(
        connection,
        encoding,
        header_sql,
        [limit],
        ordered,
        owner,
        0,
        0,
        16,
    )
}

#[allow(clippy::too_many_arguments)]
fn task_inventory_core_on<P: rusqlite::Params>(
    connection: &Connection,
    encoding: TextEncoding,
    header_sql: &str,
    parameters: P,
    ordered: bool,
    owner: Option<&str>,
    held_bytes: usize,
    initial_work: usize,
    output_factor: usize,
) -> Result<Vec<DownloadTask>> {
    let limits = store_inventory_limits();
    inventory_admit("task held metadata", held_bytes, limits.metadata_bytes)?;
    inventory_admit("task held scratch", held_bytes, limits.scratch_bytes)?;
    inventory_admit("task initial work", initial_work, limits.work_bytes)?;
    let mut statement = connection.prepare(header_sql)?;
    let mut rows = statement.query(parameters)?;
    let mut headers = Vec::new();
    let mut raw_total = 0usize;
    let mut work = initial_work;
    let mut metadata = held_bytes;
    let mut maximum_scratch = 0usize;
    while let Some(row) = rows.next()? {
        inventory_admit(
            "task rows",
            headers
                .len()
                .checked_add(1)
                .ok_or(StoreInventoryLimit { resource: "rows" })?,
            limits.rows,
        )?;
        let text: bool = row.get(1)?;
        if !text {
            return Err(StoreInventoryUnavailable.into());
        }
        let raw = usize::try_from(row.get::<_, i64>(2)?).map_err(|_| StoreInventoryUnavailable)?;
        let upper = encoding
            .utf8_upper(raw)
            .map_err(|_| StoreInventoryUnavailable)?;
        raw_total = inventory_sum(&[raw_total, raw])?;
        work = inventory_sum(&[
            work,
            upper
                .checked_mul(if output_factor > 16 { 64 } else { 4 })
                .ok_or(StoreInventoryLimit {
                    resource: "work bytes",
                })?,
        ])?;
        let updated_at = row
            .get::<_, i64>(3)
            .map_err(|_| StoreInventoryUnavailable)?;
        metadata = inventory_sum(&[
            metadata,
            2 * std::mem::size_of::<TaskInventoryHeader>(),
            128,
        ])?;
        inventory_admit("task encoded bytes", raw_total, limits.encoded_bytes)?;
        inventory_admit("task metadata", metadata, limits.metadata_bytes)?;
        maximum_scratch = maximum_scratch.max(inventory_vm_scratch(encoding, raw, upper)?);
        inventory_admit("task parse work", work, limits.work_bytes)?;
        inventory_admit(
            "task scratch",
            inventory_sum(&[maximum_scratch, metadata])?,
            limits.scratch_bytes,
        )?;
        headers.push(TaskInventoryHeader {
            rowid: row.get::<_, i64>(0)?,
            raw,
            upper,
            updated_at,
        });
    }
    drop(rows);
    drop(statement);
    if ordered && headers.len() > 1 {
        let levels =
            usize::try_from(usize::BITS - headers.len().leading_zeros()).map_err(|_| {
                StoreInventoryLimit {
                    resource: "ordering work",
                }
            })?;
        let ordering = headers
            .len()
            .checked_mul(levels)
            .and_then(|n| n.checked_mul(64))
            .ok_or(StoreInventoryLimit {
                resource: "ordering work",
            })?;
        work = inventory_sum(&[work, ordering])?;
        inventory_admit("task ordering and parse work", work, limits.work_bytes)?;
        headers.sort_unstable_by_key(|header| (header.updated_at, header.rowid));
    }
    let mut selected = Vec::new();
    let mut selected_bytes = metadata;
    for TaskInventoryHeader {
        rowid, raw, upper, ..
    } in headers
    {
        let mut statement =
            connection.prepare("SELECT payload FROM download_tasks WHERE rowid=?1")?;
        let mut rows = statement.query([rowid])?;
        let row = rows.next()?.ok_or(StoreInventoryUnavailable)?;
        let payload = row
            .get_ref(0)?
            .as_str()
            .map_err(|_| StoreInventoryUnavailable)?;
        inventory_admit("task utf8 bytes", payload.len(), upper)?;
        if let Some(owner) = owner {
            if !task_owner_matches(payload, owner)? {
                continue;
            }
        }
        selected_bytes = inventory_sum(&[
            selected_bytes,
            upper
                .checked_mul(output_factor)
                .ok_or(StoreInventoryLimit {
                    resource: "task output metadata",
                })?,
            2 * std::mem::size_of::<DownloadTask>(),
        ])?;
        inventory_admit(
            "task output metadata",
            selected_bytes,
            limits.metadata_bytes,
        )?;
        inventory_admit(
            "task selected scratch",
            inventory_sum(&[selected_bytes, maximum_scratch])?,
            limits.scratch_bytes,
        )?;
        selected.push((rowid, raw, upper));
    }
    let mut tasks = Vec::with_capacity(selected.len());
    for (rowid, _raw, upper) in selected {
        let mut statement =
            connection.prepare("SELECT payload FROM download_tasks WHERE rowid=?1")?;
        let mut rows = statement.query([rowid])?;
        let row = rows.next()?.ok_or(StoreInventoryUnavailable)?;
        let payload = row
            .get_ref(0)?
            .as_str()
            .map_err(|_| StoreInventoryUnavailable)?;
        inventory_admit("task utf8 bytes", payload.len(), upper)?;
        #[cfg(test)]
        inventory_budget_tests::observe_task_read();
        tasks.push(
            serde_json::from_str(payload).context("decode admitted persistent download task")?,
        );
    }
    Ok(tasks)
}

fn task_owner_matches(payload: &str, owner: &str) -> Result<bool> {
    use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, Visitor};
    struct Account<'a> {
        owner: &'a str,
        denied: &'a std::cell::Cell<bool>,
    }
    impl<'de> DeserializeSeed<'de> for Account<'_> {
        type Value = bool;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            de: D,
        ) -> std::result::Result<bool, D::Error> {
            struct Text<'a> {
                owner: &'a str,
                denied: &'a std::cell::Cell<bool>,
            }
            impl Visitor<'_> for Text<'_> {
                type Value = bool;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("a bounded task account key")
                }
                fn visit_str<E: serde::de::Error>(
                    self,
                    text: &str,
                ) -> std::result::Result<bool, E> {
                    if text.len() > 16 * 1024 {
                        self.denied.set(true);
                        return Err(E::custom("task account key allowance"));
                    }
                    Ok(text == self.owner)
                }
            }
            de.deserialize_str(Text {
                owner: self.owner,
                denied: self.denied,
            })
        }
    }
    struct Field;
    impl<'de> DeserializeSeed<'de> for Field {
        type Value = bool;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            de: D,
        ) -> std::result::Result<bool, D::Error> {
            struct Name;
            impl Visitor<'_> for Name {
                type Value = bool;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("a task field name")
                }
                fn visit_str<E: serde::de::Error>(
                    self,
                    text: &str,
                ) -> std::result::Result<bool, E> {
                    Ok(text == "account_key")
                }
            }
            de.deserialize_identifier(Name)
        }
    }
    struct Owner<'a> {
        owner: &'a str,
        denied: &'a std::cell::Cell<bool>,
    }
    impl<'de> DeserializeSeed<'de> for Owner<'_> {
        type Value = bool;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            de: D,
        ) -> std::result::Result<bool, D::Error> {
            struct Object<'a> {
                owner: &'a str,
                denied: &'a std::cell::Cell<bool>,
            }
            impl<'de> Visitor<'de> for Object<'_> {
                type Value = bool;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("persistent task fields")
                }
                fn visit_map<A: MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> std::result::Result<bool, A::Error> {
                    let mut matched = None;
                    while let Some(account) = map.next_key_seed(Field)? {
                        if account {
                            if matched.is_some() {
                                return Err(serde::de::Error::duplicate_field("account_key"));
                            }
                            matched = Some(map.next_value_seed(Account {
                                owner: self.owner,
                                denied: self.denied,
                            })?);
                        } else {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                    matched.ok_or_else(|| serde::de::Error::missing_field("account_key"))
                }
            }
            de.deserialize_map(Object {
                owner: self.owner,
                denied: self.denied,
            })
        }
    }
    let denied = std::cell::Cell::new(false);
    let mut deserializer = serde_json::Deserializer::from_str(payload);
    let matched = Owner {
        owner,
        denied: &denied,
    }
    .deserialize(&mut deserializer);
    if denied.get() {
        return Err(StoreInventoryLimit {
            resource: "task owner bytes",
        }
        .into());
    }
    let matched = matched.context("decode task owner metadata")?;
    deserializer.end().context("decode task owner metadata")?;
    Ok(matched)
}

fn task_borrowed_price(task: &DownloadTask) -> Result<(usize, usize)> {
    let strings = inventory_sum(&[
        task.id.len(),
        task.book_id.len(),
        task.account_key.len(),
        task.error.as_ref().map_or(0, String::len),
    ])?;
    let bytes = inventory_sum(&[
        strings.checked_mul(8).ok_or(StoreInventoryLimit {
            resource: "task input bytes",
        })?,
        task.chapters
            .len()
            .checked_mul(64)
            .ok_or(StoreInventoryLimit {
                resource: "task input cells",
            })?,
        2 * std::mem::size_of::<DownloadTask>(),
        1024,
    ])?;
    let work = inventory_sum(&[
        strings.checked_mul(8).ok_or(StoreInventoryLimit {
            resource: "task input work",
        })?,
        task.chapters
            .len()
            .checked_mul(128)
            .ok_or(StoreInventoryLimit {
                resource: "task input work",
            })?,
        512,
    ])?;
    Ok((bytes, work))
}

fn catalog_task_inventory_on(
    connection: &Connection,
    encoding: TextEncoding,
    book: &str,
    held: usize,
) -> Result<Vec<DownloadTask>> {
    let limits = store_inventory_limits();
    let limit = i64::try_from(
        limits
            .rows
            .checked_add(1)
            .ok_or(StoreInventoryLimit { resource: "rows" })?,
    )
    .map_err(|_| StoreInventoryLimit { resource: "rows" })?;
    task_inventory_core_on(
        connection,
        encoding,
        "SELECT rowid,typeof(payload)='text',COALESCE(octet_length(payload),0),CASE WHEN typeof(updated_at)='integer' THEN updated_at END FROM download_tasks NOT INDEXED WHERE book_id=?2 AND status IN ('pending','running','paused','failed') LIMIT ?1",
        params![limit, book],
        true,
        None,
        held,
        held,
        32,
    )
}

fn task_inventory_by_id_on(
    connection: &Connection,
    encoding: TextEncoding,
    id: &str,
) -> Result<Option<DownloadTask>> {
    task_inventory_by_id_reserved_on(connection, encoding, id, 0, 0)
}

fn task_inventory_by_id_reserved_on(
    connection: &Connection,
    encoding: TextEncoding,
    id: &str,
    held_bytes: usize,
    initial_work: usize,
) -> Result<Option<DownloadTask>> {
    let limits = store_inventory_limits();
    inventory_admit("task held metadata", held_bytes, limits.metadata_bytes)?;
    inventory_admit("task held scratch", held_bytes, limits.scratch_bytes)?;
    inventory_admit("task initial work", initial_work, limits.work_bytes)?;
    let metadata: Option<(i64, bool, i64)> = connection
        .query_row(
            "SELECT rowid,typeof(payload)='text',COALESCE(octet_length(payload),0)
        FROM download_tasks WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((rowid, text, raw)) = metadata else {
        return Ok(None);
    };
    if !text {
        return Err(StoreInventoryUnavailable.into());
    }
    inventory_admit("task rows", 1, limits.rows)?;
    let raw = usize::try_from(raw).map_err(|_| StoreInventoryUnavailable)?;
    let upper = encoding
        .utf8_upper(raw)
        .map_err(|_| StoreInventoryUnavailable)?;
    let output = inventory_sum(&[
        held_bytes,
        id.len(),
        upper.checked_mul(16).ok_or(StoreInventoryLimit {
            resource: "task output metadata",
        })?,
        2 * std::mem::size_of::<DownloadTask>(),
        2 * std::mem::size_of::<(i64, usize, usize)>(),
        128,
    ])?;
    inventory_admit("task encoded bytes", raw, limits.encoded_bytes)?;
    inventory_admit("task metadata", output, limits.metadata_bytes)?;
    inventory_admit(
        "task parse work",
        inventory_sum(&[
            initial_work,
            upper.checked_mul(4).ok_or(StoreInventoryLimit {
                resource: "work bytes",
            })?,
        ])?,
        limits.work_bytes,
    )?;
    inventory_admit(
        "task selected scratch",
        inventory_sum(&[output, inventory_vm_scratch(encoding, raw, upper)?])?,
        limits.scratch_bytes,
    )?;
    let mut statement = connection.prepare("SELECT payload FROM download_tasks WHERE rowid=?1")?;
    let mut rows = statement.query([rowid])?;
    let row = rows.next()?.ok_or(StoreInventoryUnavailable)?;
    let payload = row
        .get_ref(0)?
        .as_str()
        .map_err(|_| StoreInventoryUnavailable)?;
    inventory_admit("task utf8 bytes", payload.len(), upper)?;
    #[cfg(test)]
    inventory_budget_tests::observe_task_read();
    Ok(Some(
        serde_json::from_str(payload).context("decode admitted selected download task")?,
    ))
}

fn setting_encoded_bytes_on(connection: &Connection, key: &str) -> Result<Option<usize>> {
    validate_identifier(key)?;
    let value = connection
        .query_row(
            "SELECT CASE WHEN typeof(value)='text' THEN COALESCE(octet_length(value),0) ELSE -1 END
            FROM settings WHERE key=?1",
            [key],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    value
        .map(|value| usize::try_from(value).map_err(|_| CachedResourceLimit.into()))
        .transpose()
}

fn with_stored_setting_text_on<T>(
    connection: &Connection,
    encoding: TextEncoding,
    key: &str,
    limits: StoredSettingReadLimits,
    read: impl FnOnce(&str, StoredSettingTextMetadata) -> Result<T>,
) -> Result<Option<T>> {
    let maximum = StoredSettingReadLimits::default();
    let limits = StoredSettingReadLimits {
        encoded_bytes: limits.encoded_bytes.min(maximum.encoded_bytes),
        utf8_bytes: limits.utf8_bytes.min(maximum.utf8_bytes),
        work_bytes: limits.work_bytes.min(maximum.work_bytes),
        scratch_bytes: limits.scratch_bytes.min(maximum.scratch_bytes),
        json_passes: limits.json_passes,
    };
    let admit = |resource, bytes, allowance| -> Result<()> {
        if bytes > allowance {
            Err(StoredSettingReadLimit { resource }.into())
        } else {
            Ok(())
        }
    };
    admit("key bytes", key.len(), 32 * 1024)?;
    let key_work = key.len().checked_mul(2).ok_or(StoredSettingReadLimit {
        resource: "key work arithmetic",
    })?;
    admit("key work bytes", key_work, limits.work_bytes)?;
    validate_identifier(key)?;
    let changes = connection.total_changes();
    let header: Option<(i64, bool, i64, i64)> = connection.query_row(
        "SELECT rowid,typeof(key)='text' AND typeof(value)='text',COALESCE(octet_length(key),0),COALESCE(octet_length(value),0) FROM settings WHERE key=?1",
        [key], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).optional()?;
    let Some((rowid, valid, key_bytes, raw)) = header else {
        return Ok(None);
    };
    if !valid {
        return Err(StoredSettingUnavailable.into());
    }
    let key_bytes = usize::try_from(key_bytes).map_err(|_| StoredSettingUnavailable)?;
    let raw = usize::try_from(raw).map_err(|_| StoredSettingUnavailable)?;
    let upper = encoding
        .utf8_upper(raw)
        .map_err(|_| StoredSettingReadLimit {
            resource: "UTF-8 arithmetic",
        })?;
    let vm = match encoding {
        TextEncoding::Utf8 => raw.checked_add(2),
        TextEncoding::Utf16 => raw.checked_mul(3).and_then(|n| n.checked_add(2)),
    }
    .ok_or(StoredSettingReadLimit {
        resource: "VM scratch arithmetic",
    })?;
    let scratch = upper
        .checked_mul(2)
        .and_then(|n| n.checked_add(256))
        .and_then(|n| n.checked_add(vm))
        .ok_or(StoredSettingReadLimit {
            resource: "parser scratch arithmetic",
        })?;
    let passes = limits
        .json_passes
        .checked_add(1)
        .ok_or(StoredSettingReadLimit {
            resource: "work arithmetic",
        })?;
    let work = upper
        .checked_mul(passes)
        .and_then(|n| n.checked_add(key_work))
        .ok_or(StoredSettingReadLimit {
            resource: "work arithmetic",
        })?;
    admit("encoded bytes", raw, limits.encoded_bytes)?;
    admit("UTF-8 bytes", upper, limits.utf8_bytes)?;
    admit("work bytes", work, limits.work_bytes)?;
    admit("scratch bytes", scratch, limits.scratch_bytes)?;
    #[cfg(test)]
    setting_text_budget_tests::after_preflight()?;
    if connection.total_changes() != changes {
        return Err(StoredSettingUnavailable.into());
    }
    let mut statement = connection.prepare(
        "SELECT CASE WHEN typeof(key)='text' AND typeof(value)='text' AND COALESCE(octet_length(key),0)=?3 AND COALESCE(octet_length(value),0)=?4 THEN value ELSE NULL END FROM settings WHERE rowid=?1 AND key=?2",
    )?;
    let mut rows = statement.query(params![rowid, key, key_bytes, raw])?;
    let row = rows.next()?.ok_or(StoredSettingUnavailable)?;
    let raw_text = row
        .get_ref(0)?
        .as_str()
        .map_err(|_| StoredSettingUnavailable)?;
    admit("UTF-8 bytes", raw_text.len(), upper.min(limits.utf8_bytes))?;
    #[cfg(test)]
    setting_text_budget_tests::observe_borrow(raw_text.len());
    read(
        raw_text,
        StoredSettingTextMetadata {
            encoded_bytes: raw,
            utf8_bytes_upper: upper,
            vm_scratch_bytes: vm,
        },
    )
    .map(Some)
}

fn image_manifest_metadata_on(
    connection: &Connection,
    encoding: TextEncoding,
    key: &str,
    maximum_bytes: usize,
    maximum_images: usize,
) -> Result<Option<(usize, usize)>> {
    validate_identifier(key)?;
    let maximum = encoding.maximum_encoded(maximum_bytes)?;
    let images = i64::try_from(maximum_images).map_err(|_| CachedResourceLimit)?;
    let value = connection
        .query_row(
            "SELECT COALESCE(octet_length(value),0),
         CASE WHEN typeof(value)='text' AND COALESCE(octet_length(value),0)<=?2 THEN
          CASE WHEN json_valid(value) THEN CASE WHEN json_type(value)='object' THEN
           CASE WHEN json_type(value,'$.images') IS NULL THEN 0
            WHEN json_type(value,'$.images')='array' AND json_array_length(value,'$.images')<=?3
            THEN json_array_length(value,'$.images') ELSE NULL END
           ELSE NULL END ELSE NULL END ELSE NULL END
         FROM settings WHERE key=?1",
            params![key, maximum, images],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
        )
        .optional()?;
    let Some((bytes, count)) = value else {
        return Ok(None);
    };
    let bytes = usize::try_from(bytes).map_err(|_| CachedResourceLimit)?;
    let count =
        usize::try_from(count.ok_or(CachedResourceLimit)?).map_err(|_| CachedResourceLimit)?;
    if encoding.utf8_upper(bytes)? > maximum_bytes || count > maximum_images {
        return Err(CachedResourceLimit.into());
    }
    Ok(Some((bytes, count)))
}

fn bounded_image_manifest_on<T: DeserializeOwned>(
    connection: &Connection,
    encoding: TextEncoding,
    key: &str,
    maximum_bytes: usize,
    maximum_images: usize,
) -> Result<Option<T>> {
    validate_identifier(key)?;
    let maximum = encoding.maximum_encoded(maximum_bytes)?;
    let images = i64::try_from(maximum_images).map_err(|_| CachedResourceLimit)?;
    // Every materializing read repeats byte/type/shape/count admission in this
    // statement. A caller's earlier length snapshot is never blind authority.
    let mut statement = connection.prepare(
        "SELECT CASE WHEN typeof(value)='text' AND COALESCE(octet_length(value),0)<=?2 THEN
          CASE WHEN json_valid(value) THEN CASE WHEN json_type(value)='object' THEN
           CASE WHEN json_type(value,'$.images') IS NULL OR
             (json_type(value,'$.images')='array' AND json_array_length(value,'$.images')<=?3)
             THEN value ELSE NULL END ELSE NULL END ELSE NULL END ELSE NULL END
         FROM settings WHERE key=?1",
    )?;
    let mut rows = statement.query(params![key, maximum, images])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    if matches!(row.get_ref(0)?, rusqlite::types::ValueRef::Null) {
        return Err(CachedResourceLimit.into());
    }
    let value = cache_text(row, 0, maximum_bytes)?;
    // SQL JSON paths select the first duplicate object key. Recheck the
    // borrowed object before generic serde can accept a later, larger images
    // array. This probe skips unknown fields and never builds image records.
    borrowed_manifest_image_count(value, maximum_images)?;
    #[cfg(test)]
    byte_admission_tests::observe_text_copy("manifest", 0);
    Ok(Some(
        serde_json::from_str(value).context("decode admitted image manifest")?,
    ))
}

fn borrowed_manifest_image_count(value: &str, maximum: usize) -> Result<usize> {
    use serde::{
        Deserialize as _,
        de::{DeserializeSeed, Deserializer as _, IgnoredAny, MapAccess, SeqAccess, Visitor},
    };
    struct Key(bool);
    impl<'de> serde::Deserialize<'de> for Key {
        fn deserialize<D: serde::Deserializer<'de>>(
            deserializer: D,
        ) -> std::result::Result<Self, D::Error> {
            struct KeyVisitor;
            impl Visitor<'_> for KeyVisitor {
                type Value = Key;
                fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter.write_str("an image manifest field name")
                }
                fn visit_str<E: serde::de::Error>(
                    self,
                    value: &str,
                ) -> std::result::Result<Key, E> {
                    Ok(Key(value == "images"))
                }
            }
            deserializer.deserialize_identifier(KeyVisitor)
        }
    }
    struct Array {
        maximum: usize,
    }
    impl<'de> DeserializeSeed<'de> for Array {
        type Value = usize;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<usize, D::Error> {
            struct ArrayVisitor {
                maximum: usize,
            }
            impl<'de> Visitor<'de> for ArrayVisitor {
                type Value = usize;
                fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter.write_str("an admitted images array")
                }
                fn visit_seq<S: SeqAccess<'de>>(
                    self,
                    mut sequence: S,
                ) -> std::result::Result<usize, S::Error> {
                    struct Item<'a> {
                        count: &'a mut usize,
                        maximum: usize,
                    }
                    impl<'de> DeserializeSeed<'de> for Item<'_> {
                        type Value = ();
                        fn deserialize<D: serde::Deserializer<'de>>(
                            self,
                            deserializer: D,
                        ) -> std::result::Result<(), D::Error> {
                            *self.count = self
                                .count
                                .checked_add(1)
                                .ok_or_else(|| serde::de::Error::custom("image count overflow"))?;
                            if *self.count > self.maximum {
                                return Err(serde::de::Error::custom(
                                    "image count exceeds allowance",
                                ));
                            }
                            IgnoredAny::deserialize(deserializer).map(|_| ())
                        }
                    }
                    let mut count = 0;
                    while sequence
                        .next_element_seed(Item {
                            count: &mut count,
                            maximum: self.maximum,
                        })?
                        .is_some()
                    {}
                    Ok(count)
                }
            }
            deserializer.deserialize_seq(ArrayVisitor {
                maximum: self.maximum,
            })
        }
    }
    struct Object {
        maximum: usize,
    }
    impl<'de> Visitor<'de> for Object {
        type Value = usize;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an image manifest object")
        }
        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> std::result::Result<usize, M::Error> {
            let mut count = None;
            while let Some(Key(images)) = map.next_key::<Key>()? {
                if images {
                    if count.is_some() {
                        return Err(serde::de::Error::custom("duplicate images field"));
                    }
                    count = Some(map.next_value_seed(Array {
                        maximum: self.maximum,
                    })?);
                } else {
                    map.next_value::<IgnoredAny>()?;
                }
            }
            Ok(count.unwrap_or(0))
        }
    }
    let mut deserializer = serde_json::Deserializer::from_str(value);
    let count = deserializer
        .deserialize_map(Object { maximum })
        .map_err(|_| CachedResourceLimit)?;
    deserializer.end().map_err(|_| CachedResourceLimit)?;
    Ok(count)
}

fn validate_body_uid_inputs(book_id: &str, chapter_uid: &str) -> Result<()> {
    for value in [book_id, chapter_uid] {
        if value.len() > 4096 {
            return Err(CachedResourceLimit.into());
        }
        validate_identifier(value)?;
    }
    Ok(())
}

fn chapter_body_encoded_bytes_on(
    connection: &Connection,
    book_id: &str,
    chapter_uid: &str,
) -> Result<Option<usize>> {
    validate_body_uid_inputs(book_id, chapter_uid)?;
    let mut statement = connection.prepare(
        "SELECT body IS NULL,typeof(body)='text',COALESCE(octet_length(body),0)
            FROM chapters WHERE book_id=?1 AND source_id=?2 LIMIT 2",
    )?;
    let mut rows = statement.query(params![book_id, chapter_uid])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let absent: bool = row.get(0)?;
    let text: bool = row.get(1)?;
    let bytes: i64 = row.get(2)?;
    if rows.next()?.is_some() {
        return Err(CachedResourceLimit.into());
    }
    if absent {
        return Ok(None);
    }
    if !text {
        return Err(CachedResourceLimit.into());
    }
    Ok(Some(
        usize::try_from(bytes).map_err(|_| CachedResourceLimit)?,
    ))
}

fn bounded_chapter_body_on(
    connection: &Connection,
    encoding: TextEncoding,
    book_id: &str,
    chapter_uid: &str,
    maximum_bytes: usize,
) -> Result<Option<String>> {
    let Some(bytes) = chapter_body_encoded_bytes_on(connection, book_id, chapter_uid)? else {
        return Ok(None);
    };
    if encoding.utf8_upper(bytes)? > maximum_bytes {
        return Err(CachedResourceLimit.into());
    }
    let maximum = encoding.maximum_encoded(maximum_bytes)?;
    let mut statement = connection.prepare(
        "SELECT CASE WHEN typeof(body)='text' AND COALESCE(octet_length(body),0)<=?3 THEN body ELSE NULL END
            FROM chapters WHERE book_id=?1 AND source_id=?2 LIMIT 2",
    )?;
    let mut rows = statement.query(params![book_id, chapter_uid, maximum])?;
    let row = rows.next()?.ok_or(CachedResourceLimit)?;
    if matches!(row.get_ref(0)?, rusqlite::types::ValueRef::Null) {
        return Err(CachedResourceLimit.into());
    }
    let body = cache_text(row, 0, maximum_bytes)?;
    #[cfg(test)]
    byte_admission_tests::observe_text_copy("body", body.len());
    Ok(Some(body.to_owned()))
}

fn annotation_numeric_types_on(connection: &Connection, book_id: &str) -> Result<()> {
    // An index can itself contain a legacy oversized sort key. Inspect table
    // serial types before range comparisons or ordered metadata projection.
    let valid: bool = connection.query_row(
        "SELECT COALESCE(MIN(typeof(chapter_index)='integer' AND typeof(start_offset)='integer' AND typeof(end_offset)='integer' AND typeof(created_at)='integer'),1) FROM annotations NOT INDEXED WHERE book_id=?1",
        [book_id], |row| row.get(0),
    )?;
    if !valid {
        return Err(AnnotationUnavailable.into());
    }
    Ok(())
}

fn annotation_count_on(
    connection: &Connection,
    book_id: &str,
    chapter: Option<u32>,
    start: Option<u32>,
    end: Option<u32>,
) -> Result<u64> {
    annotation_numeric_types_on(connection, book_id)?;
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM annotations WHERE book_id=?1 AND (?2 IS NULL OR chapter_index=?2)
         AND (?3 IS NULL OR (start_offset<?4 AND end_offset>?3))",
        params![book_id, chapter, start, end],
        |row| row.get(0),
    )?;
    u64::try_from(count).map_err(|_| AnnotationUnavailable.into())
}

struct AnnotationFilter<'a> {
    book_id: &'a str,
    chapter: Option<u32>,
    start: Option<u32>,
    end: Option<u32>,
    id: Option<&'a str>,
    exact: bool,
}

struct AnnotationMetadata {
    rowid: i64,
    chapter_index: u32,
    start: u32,
    end: u32,
    created_at: i64,
}

fn annotation_metadata(row: &Row<'_>) -> Result<AnnotationMetadata> {
    let metadata = AnnotationMetadata {
        rowid: row.get(0).map_err(|_| AnnotationUnavailable)?,
        chapter_index: row.get(1).map_err(|_| AnnotationUnavailable)?,
        start: row.get(2).map_err(|_| AnnotationUnavailable)?,
        end: row.get(3).map_err(|_| AnnotationUnavailable)?,
        created_at: row.get(4).map_err(|_| AnnotationUnavailable)?,
    };
    if metadata.start >= metadata.end {
        return Err(AnnotationUnavailable.into());
    }
    Ok(metadata)
}

#[allow(clippy::too_many_arguments)]
fn annotation_headers_on(
    connection: &Connection,
    encoding: TextEncoding,
    filter: AnnotationFilter<'_>,
    offset: usize,
    selected_rows: usize,
    complete: bool,
    limits: AnnotationLimits,
) -> Result<Vec<AnnotationHeader>> {
    annotation_numeric_types_on(connection, filter.book_id)?;
    annotation_admit("rows", selected_rows, limits.rows)?;
    let read_limit = if complete {
        selected_rows.checked_add(1).ok_or(AnnotationUnavailable)?
    } else {
        selected_rows
    };
    let read_limit = i64::try_from(read_limit).map_err(|_| AnnotationUnavailable)?;
    let offset = i64::try_from(offset).map_err(|_| AnnotationUnavailable)?;
    let mut metadata = Vec::new();
    let mut bytes = 0usize;
    let mut cells = 0usize;
    {
        let sql = if filter.id.is_some() {
            "SELECT rowid,CASE WHEN typeof(chapter_index)='integer' THEN chapter_index END,CASE WHEN typeof(start_offset)='integer' THEN start_offset END,CASE WHEN typeof(end_offset)='integer' THEN end_offset END,CASE WHEN typeof(created_at)='integer' THEN created_at END,typeof(id)='text',
                COALESCE(octet_length(id),0) FROM annotations WHERE book_id=?1 AND id=?5
                AND (?2 IS NULL OR chapter_index=?2) AND (?3 IS NULL OR (start_offset<?4 AND end_offset>?3))
                AND (NOT ?8 OR (start_offset=?3 AND end_offset=?4))
                ORDER BY chapter_index,start_offset,created_at,rowid LIMIT ?6 OFFSET ?7"
        } else {
            "SELECT rowid,CASE WHEN typeof(chapter_index)='integer' THEN chapter_index END,CASE WHEN typeof(start_offset)='integer' THEN start_offset END,CASE WHEN typeof(end_offset)='integer' THEN end_offset END,CASE WHEN typeof(created_at)='integer' THEN created_at END,typeof(id)='text',
                COALESCE(octet_length(id),0) FROM annotations INDEXED BY annotations_metadata WHERE book_id=?1
                AND (?2 IS NULL OR chapter_index=?2) AND (?3 IS NULL OR (start_offset<?4 AND end_offset>?3))
                AND (NOT ?8 OR (start_offset=?3 AND end_offset=?4))
                AND (?5 IS NULL) ORDER BY chapter_index,start_offset,created_at,rowid LIMIT ?6 OFFSET ?7"
        };
        let mut statement = connection.prepare(sql)?;
        let mut rows = statement.query(params![
            filter.book_id,
            filter.chapter,
            filter.start,
            filter.end,
            filter.id,
            read_limit,
            offset,
            filter.exact
        ])?;
        while let Some(row) = rows.next()? {
            let count = metadata.len().checked_add(1).ok_or(AnnotationUnavailable)?;
            annotation_admit("rows", count, limits.rows)?;
            if !row.get::<_, bool>(5)? {
                return Err(AnnotationUnavailable.into());
            }
            let id_bytes = annotation_encoded(row, 6, encoding)?;
            annotation_admit("identifier bytes", id_bytes, limits.id_bytes)?;
            cells = annotation_sum(cells, 8)?;
            annotation_admit("SQL cells", cells, limits.cells)?;
            let fixed = annotation_product(
                2,
                std::mem::size_of::<AnnotationMetadata>() + std::mem::size_of::<AnnotationHeader>(),
            )?;
            bytes = annotation_sum(bytes, annotation_sum(fixed, id_bytes)?)?;
            annotation_admit("working-set bytes", bytes, limits.bytes)?;
            let value = annotation_metadata(row)?;
            if metadata.is_empty() {
                metadata.reserve_exact(1);
            }
            metadata.push(value);
        }
    }
    #[cfg(test)]
    annotation_budget_tests::after_preflight()?;
    let mut headers = Vec::with_capacity(metadata.len());
    for metadata in metadata {
        let mut statement = connection.prepare("SELECT id FROM annotations WHERE rowid=?1")?;
        let mut rows = statement.query([metadata.rowid])?;
        let row = rows.next()?.ok_or(AnnotationUnavailable)?;
        let id = annotation_text(row, 0, limits.id_bytes)?;
        #[cfg(test)]
        annotation_copied(Some(id.len()), None, None);
        headers.push(AnnotationHeader {
            key: AnnotationKey {
                rowid: metadata.rowid,
                id: id.to_owned(),
            },
            chapter_index: metadata.chapter_index,
            start: metadata.start,
            end: metadata.end,
            created_at: metadata.created_at,
        });
    }
    Ok(headers)
}

fn annotation_records_on(
    connection: &Connection,
    encoding: TextEncoding,
    book_id: &str,
    keys: &[AnnotationKey],
    limits: AnnotationLimits,
) -> Result<Vec<Annotation>> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    annotation_admit("rows", keys.len(), limits.rows)?;
    // Ten scalar admission columns, two identity columns and two payload
    // columns per selected row. These are SQL fields, never display cells.
    let cells = annotation_product(keys.len(), 14)?;
    annotation_admit("SQL cells", cells, limits.cells)?;
    let mut bytes = annotation_record_fixed(keys.len())?;
    annotation_admit("working-set bytes", bytes, limits.bytes)?;
    let mut metadata = Vec::with_capacity(keys.len());
    for key in keys {
        annotation_input_id(&key.id, limits.id_bytes)?;
        let mut statement = connection.prepare(
            "SELECT rowid,CASE WHEN typeof(chapter_index)='integer' THEN chapter_index END,CASE WHEN typeof(start_offset)='integer' THEN start_offset END,CASE WHEN typeof(end_offset)='integer' THEN end_offset END,CASE WHEN typeof(created_at)='integer' THEN created_at END,
                typeof(id)='text' AND typeof(book_id)='text' AND typeof(quote)='text' AND typeof(note)='text',
                COALESCE(octet_length(id),0),COALESCE(octet_length(book_id),0),
                COALESCE(octet_length(quote),0),COALESCE(octet_length(note),0)
                FROM annotations WHERE rowid=?1",
        )?;
        let mut rows = statement.query([key.rowid])?;
        let row = rows.next()?.ok_or(AnnotationUnavailable)?;
        if !row.get::<_, bool>(5)? {
            return Err(AnnotationUnavailable.into());
        }
        let id_bytes = annotation_encoded(row, 6, encoding)?;
        let book_bytes = annotation_encoded(row, 7, encoding)?;
        let quote_bytes = annotation_encoded(row, 8, encoding)?;
        let note_bytes = annotation_encoded(row, 9, encoding)?;
        annotation_admit("identifier bytes", id_bytes, limits.id_bytes)?;
        annotation_admit("book identifier bytes", book_bytes, 4096)?;
        annotation_admit("quote bytes", quote_bytes, limits.quote_bytes)?;
        annotation_admit("note bytes", note_bytes, limits.note_bytes)?;
        for field in [id_bytes, id_bytes, book_bytes, quote_bytes, note_bytes] {
            bytes = annotation_sum(bytes, field)?;
        }
        annotation_admit("working-set bytes", bytes, limits.bytes)?;
        metadata.push(annotation_metadata(row)?);
    }
    #[cfg(test)]
    annotation_budget_tests::after_preflight()?;
    // Validate every admitted identity before projecting any quote or note.
    for key in keys {
        let mut statement =
            connection.prepare("SELECT id,book_id FROM annotations WHERE rowid=?1")?;
        let mut rows = statement.query([key.rowid])?;
        let row = rows.next()?.ok_or(AnnotationUnavailable)?;
        if annotation_text(row, 0, limits.id_bytes)? != key.id
            || annotation_text(row, 1, 4096)? != book_id
        {
            return Err(AnnotationUnavailable.into());
        }
    }
    let mut records = Vec::with_capacity(keys.len());
    for (key, metadata) in keys.iter().zip(metadata) {
        let mut statement =
            connection.prepare("SELECT quote,note FROM annotations WHERE rowid=?1")?;
        let mut rows = statement.query([key.rowid])?;
        let row = rows.next()?.ok_or(AnnotationUnavailable)?;
        let quote = annotation_text(row, 0, limits.quote_bytes)?;
        let note = annotation_text(row, 1, limits.note_bytes)?;
        #[cfg(test)]
        annotation_copied(Some(key.id.len()), Some(quote.len()), Some(note.len()));
        records.push(Annotation {
            id: key.id.clone(),
            book_id: book_id.to_owned(),
            chapter_index: metadata.chapter_index,
            start: metadata.start,
            end: metadata.end,
            quote: quote.to_owned(),
            note: note.to_owned(),
            created_at: metadata.created_at,
        });
    }
    Ok(records)
}

fn annotation_record_fixed(rows: usize) -> Result<usize> {
    let records = annotation_product(
        rows,
        annotation_product(
            2,
            std::mem::size_of::<AnnotationMetadata>() + std::mem::size_of::<Annotation>(),
        )?,
    )?;
    // Include the caller-side duplicate-key membership check before its map
    // allocation, plus conservative bucket/control/minimum spare space.
    let keys = annotation_product(
        rows.checked_add(4).ok_or(AnnotationUnavailable)?,
        2 * (std::mem::size_of::<i64>() + 2),
    )?;
    annotation_sum(records, keys)
}

fn annotation_geometry_on(
    connection: &Connection,
    book_id: &str,
    chapter: u32,
    start: Option<u32>,
    end: Option<u32>,
    limits: AnnotationLimits,
) -> Result<Vec<AnnotationGeometry>> {
    annotation_numeric_types_on(connection, book_id)?;
    let maximum = i64::try_from(limits.rows.checked_add(1).ok_or(AnnotationUnavailable)?)
        .map_err(|_| AnnotationUnavailable)?;
    let mut statement = connection.prepare(
        "SELECT rowid,CASE WHEN typeof(chapter_index)='integer' THEN chapter_index END,CASE WHEN typeof(start_offset)='integer' THEN start_offset END,CASE WHEN typeof(end_offset)='integer' THEN end_offset END,CASE WHEN typeof(created_at)='integer' THEN created_at END FROM annotations INDEXED BY annotations_metadata
            WHERE book_id=?1 AND chapter_index=?2 AND (?3 IS NULL OR (start_offset<?4 AND end_offset>?3))
            ORDER BY chapter_index,start_offset,created_at,rowid LIMIT ?5",
    )?;
    let mut rows = statement.query(params![book_id, chapter, start, end, maximum])?;
    let mut geometry = Vec::new();
    while let Some(row) = rows.next()? {
        let count = geometry.len().checked_add(1).ok_or(AnnotationUnavailable)?;
        annotation_admit("rows", count, limits.rows)?;
        annotation_admit("SQL cells", annotation_product(count, 5)?, limits.cells)?;
        annotation_admit(
            "working-set bytes",
            annotation_product(count, 2 * std::mem::size_of::<AnnotationGeometry>())?,
            limits.bytes,
        )?;
        let metadata = annotation_metadata(row)?;
        if geometry.is_empty() {
            geometry.reserve_exact(1);
        }
        geometry.push(AnnotationGeometry {
            rowid: metadata.rowid,
            chapter_index: metadata.chapter_index,
            start: metadata.start,
            end: metadata.end,
            created_at: metadata.created_at,
        });
    }
    Ok(geometry)
}

fn annotation_header_key_on(
    connection: &Connection,
    encoding: TextEncoding,
    book_id: &str,
    key: &AnnotationKey,
    limits: AnnotationLimits,
) -> Result<Option<AnnotationHeader>> {
    annotation_admit("rows", 1, limits.rows)?;
    annotation_admit("SQL cells", 10, limits.cells)?;
    let metadata = {
        let mut statement = connection.prepare(
            "SELECT rowid,CASE WHEN typeof(chapter_index)='integer' THEN chapter_index END,CASE WHEN typeof(start_offset)='integer' THEN start_offset END,CASE WHEN typeof(end_offset)='integer' THEN end_offset END,CASE WHEN typeof(created_at)='integer' THEN created_at END,typeof(id)='text' AND typeof(book_id)='text',
                COALESCE(octet_length(id),0),COALESCE(octet_length(book_id),0) FROM annotations WHERE rowid=?1",
        )?;
        let mut rows = statement.query([key.rowid])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        if !row.get::<_, bool>(5)? {
            return Err(AnnotationUnavailable.into());
        }
        let id_bytes = annotation_encoded(row, 6, encoding)?;
        let book_bytes = annotation_encoded(row, 7, encoding)?;
        annotation_admit("identifier bytes", id_bytes, limits.id_bytes)?;
        annotation_admit("book identifier bytes", book_bytes, 4096)?;
        let fixed = annotation_product(
            2,
            std::mem::size_of::<AnnotationMetadata>() + std::mem::size_of::<AnnotationHeader>(),
        )?;
        annotation_admit(
            "working-set bytes",
            annotation_sum(fixed, annotation_sum(id_bytes, book_bytes)?)?,
            limits.bytes,
        )?;
        annotation_metadata(row)?
    };
    let mut statement = connection.prepare("SELECT id,book_id FROM annotations WHERE rowid=?1")?;
    let mut rows = statement.query([key.rowid])?;
    let row = rows.next()?.ok_or(AnnotationUnavailable)?;
    if annotation_text(row, 0, limits.id_bytes)? != key.id
        || annotation_text(row, 1, 4096)? != book_id
    {
        return Err(AnnotationUnavailable.into());
    }
    #[cfg(test)]
    annotation_copied(Some(key.id.len()), None, None);
    Ok(Some(AnnotationHeader {
        key: key.clone(),
        chapter_index: metadata.chapter_index,
        start: metadata.start,
        end: metadata.end,
        created_at: metadata.created_at,
    }))
}

fn annotation_field_state(
    encoding: TextEncoding,
    text: bool,
    bytes: i64,
    maximum: usize,
) -> Result<AnnotationFieldState> {
    if !text {
        return Ok(AnnotationFieldState::Unavailable);
    }
    let bytes = usize::try_from(bytes).map_err(|_| AnnotationUnavailable)?;
    let utf8_bytes_upper = encoding.utf8_upper(bytes)?;
    Ok(if utf8_bytes_upper <= maximum {
        AnnotationFieldState::Available { utf8_bytes_upper }
    } else {
        AnnotationFieldState::TooLarge { utf8_bytes_upper }
    })
}

fn annotation_field_on(
    connection: &Connection,
    encoding: TextEncoding,
    rowid: i64,
    maximum_bytes: usize,
    note: bool,
) -> Result<String> {
    let sql = if note {
        "SELECT typeof(note)='text',COALESCE(octet_length(note),0) FROM annotations WHERE rowid=?1"
    } else {
        "SELECT typeof(quote)='text',COALESCE(octet_length(quote),0) FROM annotations WHERE rowid=?1"
    };
    let (text, bytes): (bool, i64) =
        connection.query_row(sql, [rowid], |row| Ok((row.get(0)?, row.get(1)?)))?;
    if !text {
        return Err(AnnotationUnavailable.into());
    }
    let bytes = encoding.utf8_upper(usize::try_from(bytes).map_err(|_| AnnotationUnavailable)?)?;
    annotation_admit(
        if note { "note bytes" } else { "quote bytes" },
        bytes,
        maximum_bytes,
    )?;
    let maximum = encoding.maximum_encoded(maximum_bytes)?;
    let sql = if note {
        "SELECT CASE WHEN typeof(note)='text' AND COALESCE(octet_length(note),0)<=?2 THEN note ELSE NULL END FROM annotations WHERE rowid=?1"
    } else {
        "SELECT CASE WHEN typeof(quote)='text' AND COALESCE(octet_length(quote),0)<=?2 THEN quote ELSE NULL END FROM annotations WHERE rowid=?1"
    };
    let mut statement = connection.prepare(sql)?;
    let mut rows = statement.query(params![rowid, maximum])?;
    let row = rows.next()?.ok_or(AnnotationUnavailable)?;
    let value = annotation_text(row, 0, maximum_bytes)?;
    #[cfg(test)]
    annotation_copied(
        None,
        (!note).then_some(value.len()),
        note.then_some(value.len()),
    );
    Ok(value.to_owned())
}

fn validate_progress(progress: f32) -> Result<()> {
    ensure!(
        progress.is_finite() && (0.0..=1.0).contains(&progress),
        "reading progress must be a finite fraction in 0..=1"
    );
    Ok(())
}

fn upsert_book_on(
    connection: &Connection,
    book: &Book,
    cached_count: i64,
    on_shelf: bool,
) -> Result<()> {
    validate_identifier(&book.id)?;
    validate_progress(book.progress)?;
    connection.execute("INSERT INTO books (id,title,author,category,intro,cover_url,source,progress,total_chapters,cached_chapters,updated,finished,on_shelf) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) ON CONFLICT(id) DO UPDATE SET title=excluded.title, author=excluded.author, category=excluded.category, intro=excluded.intro, cover_url=excluded.cover_url, source=excluded.source, progress=excluded.progress, total_chapters=excluded.total_chapters, cached_chapters=excluded.cached_chapters, updated=excluded.updated, finished=excluded.finished, on_shelf=excluded.on_shelf", params![book.id, book.title, book.author, book.category, book.intro, book.cover_url, book.source, book.progress, book.total_chapters, cached_count, book.updated, book.finished, on_shelf])?;
    Ok(())
}

fn upsert_chapter_on(connection: &Connection, chapter: &Chapter) -> Result<()> {
    validate_identifier(&chapter.book_id)?;
    validate_identifier(&chapter.id)?;
    if let Some(body) = &chapter.body {
        ensure!(
            !body.trim().is_empty(),
            "empty chapter body is not a cache entry"
        );
    }
    let remote: bool = connection.query_row(
        "SELECT source='weread' FROM books WHERE id=?1",
        [&chapter.book_id],
        |row| row.get(0),
    )?;
    let content_status = if chapter.body.is_none() && chapter.content_status == ContentStatus::Full
    {
        ContentStatus::Unknown
    } else if !remote && chapter.body.is_some() && chapter.content_status == ContentStatus::Unknown
    {
        ContentStatus::Full
    } else {
        chapter.content_status
    };
    if content_status == ContentStatus::Full {
        ensure!(
            !remote || !chapter.content_account_key.is_empty(),
            "remote full content has no account owner"
        );
    }
    connection.execute("INSERT INTO chapters (book_id,chapter_index,source_id,title,body,content_status,content_account_key) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(book_id,chapter_index) DO UPDATE SET source_id=excluded.source_id,title=excluded.title,
        body=CASE WHEN chapters.source_id=excluded.source_id THEN COALESCE(excluded.body,chapters.body) ELSE excluded.body END,
        content_status=CASE WHEN chapters.source_id=excluded.source_id AND excluded.body IS NULL THEN chapters.content_status ELSE excluded.content_status END,
        content_account_key=CASE WHEN chapters.source_id=excluded.source_id AND excluded.body IS NULL THEN chapters.content_account_key ELSE excluded.content_account_key END",
        params![chapter.book_id, chapter.index, chapter.id, chapter.title, chapter.body, content_status.as_str(), chapter.content_account_key])?;
    Ok(())
}

fn persist_task_on(connection: &Connection, task: &DownloadTask) -> Result<()> {
    let (bytes, work) = task_borrowed_price(task)?;
    let limits = store_inventory_limits();
    inventory_admit("task persistence metadata", bytes, limits.metadata_bytes)?;
    inventory_admit("task persistence scratch", bytes, limits.scratch_bytes)?;
    inventory_admit("task persistence work", work, limits.work_bytes)?;
    chapter_state::clean_states_before_task_write_on(connection, task)?;
    connection.execute("INSERT INTO download_tasks (id, book_id, status, updated_at, payload) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET book_id=excluded.book_id, status=excluded.status, updated_at=excluded.updated_at, payload=excluded.payload", params![task.id, task.book_id, task.status.as_str(), task.updated_at, serde_json::to_string(task)?])?;
    Ok(())
}

fn update_cached_count(
    connection: &Connection,
    book_id: &str,
    encoding: TextEncoding,
) -> Result<()> {
    update_cached_counts_on(connection, encoding, std::iter::once(book_id))
}

fn read_book(row: &Row<'_>) -> rusqlite::Result<Book> {
    let id = row.get("id")?;
    let title = row.get("title")?;
    let author = row.get("author")?;
    let category = row.get("category")?;
    let intro = row.get::<_, String>("intro")?;
    #[cfg(test)]
    book_intro_copied(intro.len());
    Ok(Book {
        id,
        title,
        author,
        category,
        intro,
        cover_url: row.get("cover_url")?,
        source: row.get("source")?,
        progress: row.get("progress")?,
        total_chapters: row.get("total_chapters")?,
        cached_chapters: row.get("cached_chapters")?,
        updated: row.get("updated")?,
        finished: row.get("finished")?,
        on_shelf: row.get("on_shelf")?,
    })
}

fn catalog_input_metadata_bytes(
    book_id: &str,
    chapters: &[Chapter],
    maximum: usize,
) -> Result<usize> {
    if book_id.is_empty()
        || book_id.len() > 4096
        || book_id.chars().any(char::is_control)
        || chapters.len() > MAX_CATALOG_HEADERS
    {
        return Err(CachedResourceLimit.into());
    }
    let mut bytes = 0usize;
    for chapter in chapters {
        // The caller and replacement retain these strings independently. Body
        // data is borrowed for SQL equality/preservation and is never cloned.
        for part in [
            book_id.len(),
            chapter.id.len(),
            chapter.title.len(),
            chapter.content_account_key.len(),
            std::mem::size_of::<Chapter>(),
        ] {
            bytes = part
                .checked_mul(2)
                .and_then(|part| bytes.checked_add(part))
                .filter(|bytes| *bytes <= maximum)
                .ok_or(CachedResourceLimit)?;
        }
    }
    Ok(bytes)
}

#[cfg(test)]
fn validate_catalog_capacity_on(
    connection: &Connection,
    book_id: &str,
    input_metadata: usize,
    maximum: usize,
) -> Result<()> {
    // Existing pure fixtures create fresh UTF-8 databases. Production always
    // uses the connection encoding captured once by Store::open.
    validate_catalog_capacity_with_encoding_on(
        connection,
        book_id,
        input_metadata,
        maximum,
        TextEncoding::Utf8,
    )
}

fn validate_catalog_capacity_with_encoding_on(
    connection: &Connection,
    book_id: &str,
    input_metadata: usize,
    maximum: usize,
    encoding: TextEncoding,
) -> Result<()> {
    if book_id.is_empty()
        || book_id.len() > 4096
        || book_id.chars().any(char::is_control)
        || input_metadata > maximum
    {
        return Err(CachedResourceLimit.into());
    }
    let (old_owner_bytes, owner_cells): (i64, i64) = connection.query_row(
        "SELECT COALESCE(SUM(COALESCE(octet_length(content_account_key),0)),0),
            COUNT(CASE WHEN COALESCE(octet_length(content_account_key),0)>0 THEN 1 END)
            FROM chapters WHERE book_id=?1",
        [book_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let old_owner_bytes = usize::try_from(old_owner_bytes).map_err(|_| CachedResourceLimit)?;
    let owner_cells = usize::try_from(owner_cells).map_err(|_| CachedResourceLimit)?;
    let old_owner_bytes = encoding.utf8_upper_total(old_owner_bytes, owner_cells)?;
    // Conservatively price all old owner strings, including rows not retained
    // by the new plan. Each selected owner may be copied into two metadata rows.
    if old_owner_bytes
        .checked_mul(2)
        .and_then(|bytes| input_metadata.checked_add(bytes))
        .is_none_or(|bytes| bytes > maximum)
    {
        return Err(CachedResourceLimit.into());
    }
    Ok(())
}

fn chapter_headers_on(
    connection: &Connection,
    book_id: &str,
    include_titles: bool,
    encoding: TextEncoding,
) -> Result<Vec<ChapterMetadata>> {
    let (count, bytes, uid_bytes, owner_bytes, status_bytes, cells, valid): (i64, i64, i64, i64, i64, i64, bool) = connection.query_row(
        "SELECT COUNT(*),COALESCE(SUM(COALESCE(octet_length(source_id),0)+COALESCE(octet_length(content_account_key),0)
         + CASE WHEN ?2 THEN COALESCE(octet_length(title),0) ELSE 0 END),0),
         COALESCE(MAX(COALESCE(octet_length(source_id),0)),0),COALESCE(MAX(COALESCE(octet_length(content_account_key),0)),0),
         COALESCE(MAX(COALESCE(octet_length(content_status),0)),0),
         COALESCE(SUM((COALESCE(octet_length(source_id),0)>0)+(COALESCE(octet_length(content_account_key),0)>0)
             + CASE WHEN ?2 THEN COALESCE(octet_length(title),0)>0 ELSE 0 END),0),
         COALESCE(MIN(typeof(source_id)='text' AND typeof(content_account_key)='text' AND typeof(content_status)='text'
             AND typeof(chapter_index)='integer' AND (NOT ?2 OR typeof(title)='text')),1)
         FROM chapters NOT INDEXED WHERE book_id=?1", params![book_id, include_titles],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
    )?;
    let count = usize::try_from(count).context("invalid chapter header count")?;
    let bytes = encoding.utf8_upper_total(
        usize::try_from(bytes).map_err(|_| CachedResourceLimit)?,
        usize::try_from(cells).map_err(|_| CachedResourceLimit)?,
    )?;
    let fixed = book_id
        .len()
        .checked_add(2 * std::mem::size_of::<Chapter>())
        .and_then(|bytes| bytes.checked_mul(count));
    if count > MAX_CATALOG_HEADERS
        || !valid
        || uid_bytes > encoding.maximum_encoded(4096)?
        || owner_bytes > encoding.maximum_encoded(16 * 1024)?
        || status_bytes > encoding.maximum_encoded(32)?
        || fixed
            .and_then(|fixed| bytes.checked_add(fixed))
            .is_none_or(|bytes| bytes > MAX_CATALOG_HEADER_BYTES as usize)
    {
        return Err(CachedResourceLimit.into());
    }
    // Transactional headers omit titles too. Display callers select titles
    // only after aggregate admission; body/permission checks stay scalar.
    let mut statement = connection.prepare(
        "SELECT source_id,CASE WHEN typeof(chapter_index)='integer' THEN chapter_index END,content_status,content_account_key,
         body IS NOT NULL AND COALESCE(octet_length(body),0)>0, CASE WHEN ?2 THEN title ELSE '' END
         FROM chapters WHERE book_id=?1 ORDER BY chapter_index",
    )?;
    let mut rows = statement.query(params![book_id, include_titles])?;
    #[cfg(test)]
    byte_admission_tests::after_header_preflight()?;
    let mut headers = Vec::with_capacity(count);
    while let Some(row) = rows.next()? {
        let status = cache_text(row, 2, 32)?;
        let content_status = ContentStatus::parse(status).ok_or(CachedResourceLimit)?;
        let body_present: bool = row.get(4)?;
        let id = cache_text(row, 0, 4096)?;
        let owner = cache_text(row, 3, 16 * 1024)?;
        let title = cache_text(row, 5, MAX_CATALOG_HEADER_BYTES as usize)?;
        #[cfg(test)]
        byte_admission_tests::observe_text_copy(
            "header",
            id.len()
                .checked_add(owner.len())
                .and_then(|bytes| bytes.checked_add(title.len()))
                .ok_or(CachedResourceLimit)?,
        );
        headers.push(ChapterMetadata {
            chapter: Chapter {
                id: id.to_owned(),
                book_id: book_id.to_owned(),
                index: row.get(1)?,
                title: title.to_owned(),
                cached: content_status == ContentStatus::Full && body_present,
                body: None,
                content_status,
                content_account_key: owner.to_owned(),
            },
            body_present,
        });
    }
    Ok(headers)
}

fn read_chapter(row: &Row<'_>) -> rusqlite::Result<Chapter> {
    let body: Option<String> = row.get("body")?;
    let status: String = row.get("content_status")?;
    let content_status = ContentStatus::parse(&status).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid content status",
            )),
        )
    })?;
    Ok(Chapter {
        id: row.get("source_id")?,
        book_id: row.get("book_id")?,
        index: row.get("chapter_index")?,
        title: row.get("title")?,
        cached: content_status == ContentStatus::Full
            && body.as_deref().is_some_and(|body| !body.trim().is_empty()),
        body,
        content_status,
        content_account_key: row.get("content_account_key")?,
    })
}

/// Development storage has one current layout and no upgrade path.
fn initialize_schema(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction()?;
    let version: i64 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    ensure!(
        version == 0,
        "versioned development databases are unsupported; use a new data directory"
    );
    transaction.execute_batch("
        CREATE TABLE IF NOT EXISTS books (
            id TEXT PRIMARY KEY NOT NULL CHECK(length(id) > 0),
            title TEXT NOT NULL, author TEXT NOT NULL, category TEXT NOT NULL,
            intro TEXT NOT NULL, cover_url TEXT NOT NULL, source TEXT NOT NULL,
            progress REAL NOT NULL DEFAULT 0 CHECK(progress >= 0 AND progress <= 1),
            total_chapters INTEGER NOT NULL DEFAULT 0,
            cached_chapters INTEGER NOT NULL DEFAULT 0,
            updated INTEGER NOT NULL DEFAULT 0, finished INTEGER NOT NULL DEFAULT 0,
            on_shelf INTEGER NOT NULL DEFAULT 1
        );
        CREATE TABLE IF NOT EXISTS chapters (
            book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            chapter_index INTEGER NOT NULL CHECK(chapter_index >= 0),
            source_id TEXT NOT NULL, title TEXT NOT NULL, body TEXT,
            content_status TEXT NOT NULL DEFAULT 'unknown'
                CHECK(content_status IN ('unknown','full','preview','locked')),
            content_account_key TEXT NOT NULL DEFAULT '',
            PRIMARY KEY(book_id, chapter_index)
        );
        CREATE TABLE IF NOT EXISTS positions (
            book_id TEXT PRIMARY KEY NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            chapter_index INTEGER NOT NULL, char_offset INTEGER NOT NULL,
            updated_at INTEGER NOT NULL, device TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS annotations (
            id TEXT PRIMARY KEY NOT NULL,
            book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            chapter_index INTEGER NOT NULL, start_offset INTEGER NOT NULL,
            end_offset INTEGER NOT NULL CHECK(end_offset > start_offset),
            quote TEXT NOT NULL, note TEXT NOT NULL, created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS annotations_metadata ON annotations(book_id,chapter_index,start_offset,created_at);
        CREATE INDEX IF NOT EXISTS annotations_created_at ON annotations(created_at);
        CREATE TABLE IF NOT EXISTS bookmarks (
            id TEXT PRIMARY KEY NOT NULL,
            book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            chapter_index INTEGER NOT NULL, char_offset INTEGER NOT NULL,
            label TEXT NOT NULL, created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS bookmarks_book ON bookmarks(book_id,chapter_index,char_offset);
        CREATE TABLE IF NOT EXISTS download_tasks (
            id TEXT PRIMARY KEY NOT NULL,
            book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            status TEXT NOT NULL CHECK(status IN ('pending','running','paused','complete','cancelled','failed')),
            updated_at INTEGER NOT NULL, payload TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS tasks_status ON download_tasks(status,updated_at);
        CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY NOT NULL,value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS reading_sessions (
            id TEXT PRIMARY KEY NOT NULL,
            book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            started_at INTEGER NOT NULL, ended_at INTEGER NOT NULL,
            seconds INTEGER NOT NULL CHECK(seconds >= 0 AND ended_at >= started_at)
        );
        CREATE INDEX IF NOT EXISTS sessions_date ON reading_sessions(started_at);
        CREATE TABLE IF NOT EXISTS query_history (query TEXT PRIMARY KEY NOT NULL,sequence INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS task_chapter_states (
                task_id TEXT NOT NULL REFERENCES download_tasks(id) ON DELETE CASCADE,
                chapter_uid TEXT NOT NULL,
                ordinal INTEGER NOT NULL CHECK(ordinal>=0 AND ordinal<8192),
                chapter_index INTEGER NOT NULL CHECK(chapter_index>=0 AND chapter_index<=4294967295),
                status TEXT NOT NULL CHECK(status IN ('pending','complete','retry_wait','blocked')),
                retry_count INTEGER NOT NULL DEFAULT 0 CHECK(retry_count>=0 AND retry_count<=4),
                retry_not_before INTEGER NOT NULL DEFAULT 0,
                error TEXT,
                PRIMARY KEY(task_id,chapter_uid),
                UNIQUE(task_id,ordinal)
             );
             CREATE INDEX IF NOT EXISTS task_chapter_retry ON task_chapter_states(task_id,status,retry_not_before);
    ")?;
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_path(name: &str) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "weread-store-{}-{sequence}-{name}.sqlite",
            std::process::id()
        ))
    }

    fn book() -> Book {
        Book::new("book-1", "Sample book")
    }

    fn chapter(index: u32, body: Option<&str>) -> Chapter {
        Chapter {
            id: format!("chapter-{index}"),
            book_id: "book-1".into(),
            index,
            title: format!("Chapter {index}"),
            cached: body.is_some(),
            body: body.map(str::to_owned),
            content_status: if body.is_some() {
                ContentStatus::Full
            } else {
                ContentStatus::Unknown
            },
            content_account_key: String::new(),
        }
    }

    #[test]
    fn schema_initialization_is_idempotent_and_preserves_state() -> Result<()> {
        let path = temporary_path("initialization");
        {
            let store = Store::open(&path)?;
            store.upsert_book(&book())?;
            store.set_setting("refresh_interval", &6_u32)?;
        }
        let store = Store::open(&path)?;
        assert_eq!(store.shelf()?.len(), 1);
        assert_eq!(store.setting::<u32>("refresh_interval")?, Some(6));
        drop(store);
        let _ = fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn versioned_development_database_is_rejected_without_changing_records() -> Result<()> {
        let path = temporary_path("unsupported-layout");
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "CREATE TABLE retained (value TEXT NOT NULL);
            INSERT INTO retained VALUES ('Keep private development data'); PRAGMA user_version=3;",
        )?;
        drop(connection);
        let error = Store::open(&path)
            .err()
            .context("versioned database was accepted")?;
        assert!(error.to_string().contains("use a new data directory"));
        let connection = Connection::open(&path)?;
        let value: String =
            connection.query_row("SELECT value FROM retained", [], |row| row.get(0))?;
        assert_eq!(value, "Keep private development data");
        let tables: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type='table'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(tables, 1);
        drop(connection);
        let _ = fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn remote_unknown_upserts_do_not_infer_full_access_and_catalogs_preserve_proof_by_uid()
    -> Result<()> {
        let store = Store::open(":memory:")?;
        let mut remote = book();
        remote.source = "weread".into();
        store.upsert_book(&remote)?;
        let unknown = Chapter {
            id: "chapter-0".into(),
            book_id: remote.id.clone(),
            body: Some("Retained unknown body".into()),
            ..Default::default()
        };
        store.upsert_chapter(&unknown)?;
        assert!(!store.chapter(&remote.id, 0)?.unwrap().cached);
        assert_eq!(store.book(&remote.id)?.unwrap().cached_chapters, 0);
        let mut verified = unknown;
        verified.content_status = ContentStatus::Preview;
        verified.content_account_key = "fixture-account".into();
        store.upsert_chapter(&verified)?;
        let mut moved = chapter(1, None);
        moved.id = "chapter-0".into();
        let mut first = chapter(0, None);
        first.id = "new-chapter".into();
        store.replace_catalog(&remote.id, &[first, moved])?;
        let saved = store.chapter(&remote.id, 1)?.unwrap();
        assert_eq!(saved.body, verified.body);
        assert_eq!(saved.content_status, ContentStatus::Preview);
        assert_eq!(saved.content_account_key, verified.content_account_key);
        assert!(!saved.cached);
        let mut local = book();
        local.id = "local-fixture".into();
        store.upsert_book(&local)?;
        store.upsert_chapter(&Chapter {
            id: "local".into(),
            book_id: local.id.clone(),
            body: Some("Local source".into()),
            ..Default::default()
        })?;
        assert!(store.chapter(&local.id, 0)?.unwrap().cached);
        Ok(())
    }

    #[test]
    fn invalid_bulk_write_rolls_back() -> Result<()> {
        let store = Store::open(":memory:")?;
        let mut invalid = Book::new("bad", "Invalid progress");
        invalid.progress = f32::NAN;
        assert!(store.upsert_books(&[book(), invalid]).is_err());
        assert!(store.books()?.is_empty());
        Ok(())
    }

    #[test]
    fn shelf_replacement_preserves_cache_and_rolls_back_on_invalid_item() -> Result<()> {
        let store = Store::open(":memory:")?;
        store.upsert_book(&book())?;
        store.upsert_chapter(&chapter(0, Some("Complete body")))?;
        store.replace_shelf(&[])?;
        assert!(store.shelf()?.is_empty());
        assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 1);
        let invalid = Book::new("", "Invalid");
        assert!(store.replace_shelf(&[book(), invalid]).is_err());
        assert!(store.shelf()?.is_empty());
        assert_eq!(
            store.chapter_body("book-1", 0)?,
            Some("Complete body".into())
        );
        Ok(())
    }

    #[test]
    fn cache_status_requires_a_complete_body_and_survives_metadata_refresh() -> Result<()> {
        let store = Store::open(":memory:")?;
        let mut metadata = book();
        metadata.cached_chapters = 99;
        store.upsert_book(&metadata)?;
        let mut entry = chapter(0, None);
        entry.cached = true;
        store.upsert_chapter(&entry)?;
        assert!(!store.chapter("book-1", 0)?.unwrap().cached);
        assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 0);
        assert!(store.cache_chapter_body("book-1", 0, "  ").is_err());
        store.cache_chapter_body("book-1", 0, "Complete body")?;
        store.upsert_chapter(&entry)?;
        assert!(store.chapter("book-1", 0)?.unwrap().cached);
        assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 1);
        Ok(())
    }

    #[test]
    fn a_new_uid_at_the_same_index_cannot_inherit_another_chapters_body() -> Result<()> {
        let store = Store::open(":memory:")?;
        store.upsert_book(&book())?;
        let original = chapter(0, Some("Original chapter body"));
        store.upsert_chapter(&original)?;
        let mut metadata = original.clone();
        metadata.title = "Updated title".into();
        metadata.body = None;
        store.upsert_chapter(&metadata)?;
        assert_eq!(
            store.chapter_body("book-1", 0)?.as_deref(),
            Some("Original chapter body")
        );
        metadata.id = "different-stable-uid".into();
        store.upsert_chapter(&metadata)?;
        assert_eq!(store.chapter_body("book-1", 0)?, None);
        assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 0);
        Ok(())
    }

    #[test]
    fn catalog_reordering_moves_cached_content_and_all_live_anchors_by_uid() -> Result<()> {
        let store = Store::open(":memory:")?;
        store.upsert_book(&book())?;
        store.upsert_chapters(&[
            chapter(0, Some("Original body zero")),
            chapter(1, None),
            chapter(2, Some("Removed body two")),
        ])?;
        store.save_position(&ReadingPosition {
            book_id: "book-1".into(),
            chapter_index: 0,
            char_offset: 4,
            ..ReadingPosition::default()
        })?;
        store.save_annotation(&Annotation {
            id: "surviving-note".into(),
            book_id: "book-1".into(),
            chapter_index: 0,
            start: 0,
            end: 4,
            quote: "Orig".into(),
            ..Annotation::default()
        })?;
        store.save_annotation(&Annotation {
            id: "removed-note".into(),
            book_id: "book-1".into(),
            chapter_index: 2,
            start: 0,
            end: 7,
            quote: "Removed".into(),
            note: "Keep this personal thought".into(),
            ..Annotation::default()
        })?;
        store.save_bookmark(&Bookmark {
            id: "bookmark-1".into(),
            book_id: "book-1".into(),
            chapter_index: 1,
            char_offset: 9,
            ..Bookmark::default()
        })?;
        store.save_task(&DownloadTask {
            id: "download-1".into(),
            book_id: "book-1".into(),
            chapters: vec![0, 1, 2],
            next_index: 1,
            status: DownloadStatus::Running,
            ..DownloadTask::default()
        })?;
        let mut first = chapter(0, None);
        first.id = "chapter-1".into();
        let mut second = chapter(1, None);
        second.id = "chapter-0".into();
        let mut replacement = chapter(2, None);
        replacement.id = "new-chapter".into();
        store.replace_catalog("book-1", &[first, second, replacement])?;
        assert_eq!(store.chapter_body("book-1", 0)?, None);
        assert_eq!(
            store.chapter_body("book-1", 1)?.as_deref(),
            Some("Original body zero")
        );
        assert_eq!(store.chapter_body("book-1", 2)?, None);
        assert_eq!(store.book("book-1")?.unwrap().total_chapters, 3);
        assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 1);
        let position = store.position("book-1")?.unwrap();
        assert_eq!((position.chapter_index, position.char_offset), (1, 4));
        let notes = store.annotations("book-1")?;
        assert_eq!(
            notes
                .iter()
                .find(|note| note.id == "surviving-note")
                .unwrap()
                .chapter_index,
            1
        );
        let orphan = notes.iter().find(|note| note.id == "removed-note").unwrap();
        assert_eq!(orphan.chapter_index, ORPHANED_CHAPTER_INDEX);
        assert_eq!(orphan.note, "Keep this personal thought");
        assert_eq!(store.bookmarks("book-1")?[0].chapter_index, 0);
        let task = store.task("download-1")?.unwrap();
        assert_eq!(task.chapters, vec![1, 0]);
        assert_eq!(task.next_index, 1);
        assert_eq!(task.status, DownloadStatus::Paused);
        assert_eq!(task.error.as_deref(), Some("catalog_changed"));
        Ok(())
    }

    #[test]
    fn deleting_current_uid_resets_position_and_cancels_an_empty_selection() -> Result<()> {
        let store = Store::open(":memory:")?;
        let mut metadata = book();
        metadata.progress = 0.8;
        store.upsert_book(&metadata)?;
        store.upsert_chapters(&[chapter(0, Some("Keep this chapter")), chapter(1, None)])?;
        store.save_position(&ReadingPosition {
            book_id: "book-1".into(),
            chapter_index: 1,
            char_offset: 100,
            ..ReadingPosition::default()
        })?;
        store.save_bookmark(&Bookmark {
            id: "removed-mark".into(),
            book_id: "book-1".into(),
            chapter_index: 1,
            ..Bookmark::default()
        })?;
        store.save_task(&DownloadTask {
            id: "removed-task".into(),
            book_id: "book-1".into(),
            chapters: vec![1],
            status: DownloadStatus::Pending,
            ..DownloadTask::default()
        })?;
        store.replace_catalog("book-1", &[chapter(0, None)])?;
        let position = store.position("book-1")?.unwrap();
        assert_eq!((position.chapter_index, position.char_offset), (0, 0));
        assert_eq!(store.book("book-1")?.unwrap().progress, 0.0);
        assert_eq!(
            store.bookmarks("book-1")?[0].chapter_index,
            ORPHANED_CHAPTER_INDEX
        );
        let task = store.task("removed-task")?.unwrap();
        assert_eq!(task.status, DownloadStatus::Cancelled);
        assert!(task.chapters.is_empty());
        assert_eq!(task.next_index, 0);
        assert!(store.pending_tasks()?.is_empty());
        store.save_task(&task)?;
        Ok(())
    }

    #[test]
    fn failed_catalog_replacement_rolls_back_content_anchors_tasks_and_counts() -> Result<()> {
        let store = Store::open(":memory:")?;
        store.upsert_book(&book())?;
        store.upsert_chapters(&[chapter(0, Some("Original body")), chapter(1, None)])?;
        store.save_position(&ReadingPosition {
            book_id: "book-1".into(),
            chapter_index: 0,
            char_offset: 3,
            ..ReadingPosition::default()
        })?;
        store.save_annotation(&Annotation {
            id: "note".into(),
            book_id: "book-1".into(),
            chapter_index: 0,
            start: 0,
            end: 4,
            ..Annotation::default()
        })?;
        store.save_task(&DownloadTask {
            id: "task".into(),
            book_id: "book-1".into(),
            chapters: vec![0, 1],
            next_index: 1,
            status: DownloadStatus::Running,
            ..DownloadTask::default()
        })?;
        let before_book = store.book("book-1")?;
        let before_chapters = store.chapters("book-1")?;
        let before_position = store.position("book-1")?;
        let before_notes = store.annotations("book-1")?;
        let before_task = store.task("task")?;
        store.connection.execute_batch("CREATE TEMP TRIGGER reject_catalog_insert BEFORE INSERT ON chapters WHEN NEW.source_id='reject' BEGIN SELECT RAISE(ABORT,'fixture failure'); END")?;
        let mut replacement = chapter(0, None);
        replacement.id = "reject".into();
        assert!(store.replace_catalog("book-1", &[replacement]).is_err());
        assert_eq!(store.book("book-1")?, before_book);
        assert_eq!(store.chapters("book-1")?, before_chapters);
        assert_eq!(store.position("book-1")?, before_position);
        assert_eq!(store.annotations("book-1")?, before_notes);
        assert_eq!(store.task("task")?, before_task);
        store
            .connection
            .execute_batch("DROP TRIGGER reject_catalog_insert")?;
        store.replace_catalog("book-1", &[chapter(0, None), chapter(1, None)])?;
        Ok(())
    }

    #[test]
    fn malformed_catalog_is_rejected_without_affecting_other_books() -> Result<()> {
        let store = Store::open(":memory:")?;
        store.upsert_books(&[book(), Book::new("other-book", "Other")])?;
        store.upsert_chapter(&chapter(0, Some("Original body")))?;
        store.save_position(&ReadingPosition {
            book_id: "other-book".into(),
            chapter_index: 99,
            char_offset: 123,
            ..ReadingPosition::default()
        })?;
        let mut duplicate = chapter(1, None);
        duplicate.id = "chapter-0".into();
        assert!(
            store
                .replace_catalog("book-1", &[chapter(0, None), duplicate])
                .is_err()
        );
        let mut gap = chapter(2, None);
        assert!(store.replace_catalog("book-1", &[gap.clone()]).is_err());
        gap.index = 0;
        gap.book_id = "other-book".into();
        assert!(store.replace_catalog("book-1", &[gap]).is_err());
        store.replace_catalog("book-1", &[chapter(0, None)])?;
        assert_eq!(
            store.chapter_body("book-1", 0)?.as_deref(),
            Some("Original body")
        );
        assert_eq!(store.position("other-book")?.unwrap().char_offset, 123);
        Ok(())
    }

    #[test]
    fn catalog_metadata_cannot_silently_rewrite_text_under_existing_anchors() -> Result<()> {
        let store = Store::open(":memory:")?;
        store.upsert_book(&book())?;
        store.upsert_chapter(&chapter(0, Some("Original annotated chapter")))?;
        store.save_position(&ReadingPosition {
            book_id: "book-1".into(),
            char_offset: 20,
            ..ReadingPosition::default()
        })?;
        store.save_annotation(&Annotation {
            id: "note".into(),
            book_id: "book-1".into(),
            start: 9,
            end: 18,
            quote: "annotated".into(),
            ..Annotation::default()
        })?;
        assert!(
            store
                .replace_catalog("book-1", &[chapter(0, Some("New text"))])
                .is_err()
        );
        assert_eq!(
            store.chapter_body("book-1", 0)?.as_deref(),
            Some("Original annotated chapter")
        );
        assert_eq!(store.position("book-1")?.unwrap().char_offset, 20);
        assert_eq!(store.annotations("book-1")?[0].quote, "annotated");
        store.replace_catalog("book-1", &[chapter(0, None)])?;
        Ok(())
    }

    #[test]
    fn setting_collection_uses_literal_prefix_and_explicit_deletion_is_atomic() -> Result<()> {
        let store = Store::open(":memory:")?;
        let first = "chapter.assets:book_%:first".to_owned();
        let second = "chapter.assets:book_%:second".to_owned();
        let unrelated = "chapter.assets:book-42:other".to_owned();
        for key in [&first, &second, &unrelated] {
            store.set_setting(key, &7_u32)?;
        }
        let selected = store.settings_with_prefix::<u32>("chapter.assets:book_%:")?;
        assert_eq!(selected.len(), 2);
        store.connection.execute_batch("CREATE TEMP TRIGGER reject_setting_delete BEFORE DELETE ON settings WHEN OLD.key='chapter.assets:book_%:second' BEGIN SELECT RAISE(ABORT,'fixture failure'); END")?;
        assert!(
            store
                .delete_settings(&[first.clone(), second.clone()])
                .is_err()
        );
        assert_eq!(store.setting::<u32>(&first)?, Some(7));
        assert_eq!(store.setting::<u32>(&second)?, Some(7));
        store
            .connection
            .execute_batch("DROP TRIGGER reject_setting_delete")?;
        store.delete_settings(&[first.clone(), second.clone()])?;
        assert!(store.setting::<u32>(&first)?.is_none());
        assert!(store.setting::<u32>(&second)?.is_none());
        assert_eq!(store.setting::<u32>(&unrelated)?, Some(7));
        Ok(())
    }

    #[test]
    fn replacing_proof_and_deleting_side_records_rolls_back_as_one_change() -> Result<()> {
        let store = Store::open(":memory:")?;
        let queue = "report.outbox.v1";
        let first = "report.receipt:fixture_%:first".to_owned();
        let second = "report.error:fixture_%:second".to_owned();
        let unrelated = "report.receipt:fixture-42:other";
        store.set_setting(queue, &"old proof")?;
        for key in [first.as_str(), second.as_str(), unrelated] {
            store.set_setting(key, &7_u32)?;
        }
        store.connection.execute_batch("CREATE TEMP TRIGGER reject_proof_delete BEFORE DELETE ON settings WHEN OLD.key='report.error:fixture_%:second' BEGIN SELECT RAISE(FAIL,'fixture failure'); END")?;
        assert!(
            store
                .set_setting_and_delete(queue, &"new proof", &[first.clone(), second.clone()])
                .is_err()
        );
        assert_eq!(
            store.setting::<String>(queue)?.as_deref(),
            Some("old proof")
        );
        assert_eq!(store.setting::<u32>(&first)?, Some(7));
        assert_eq!(store.setting::<u32>(&second)?, Some(7));
        store
            .connection
            .execute_batch("DROP TRIGGER reject_proof_delete")?;
        store.set_setting_and_delete(queue, &"new proof", &[first.clone(), second.clone()])?;
        assert_eq!(
            store.setting::<String>(queue)?.as_deref(),
            Some("new proof")
        );
        assert!(store.setting::<u32>(&first)?.is_none());
        assert!(store.setting::<u32>(&second)?.is_none());
        assert_eq!(store.setting::<u32>(unrelated)?, Some(7));
        assert!(
            store
                .set_setting_and_delete(queue, &"lost proof", &[queue.to_owned()])
                .is_err()
        );
        assert_eq!(
            store.setting::<String>(queue)?.as_deref(),
            Some("new proof")
        );
        Ok(())
    }

    #[test]
    fn chapter_body_uid_lookup_is_book_scoped_and_rejects_ambiguous_identity() -> Result<()> {
        let store = Store::open(":memory:")?;
        store.upsert_books(&[book(), Book::new("other-book", "Other fixture")])?;
        store.upsert_chapter(&chapter(0, Some("First book body")))?;
        let mut other = chapter(0, Some("Other book body"));
        other.book_id = "other-book".into();
        store.upsert_chapter(&other)?;
        assert_eq!(
            store.chapter_body_by_uid("book-1", "chapter-0")?.as_deref(),
            Some("First book body")
        );
        assert_eq!(
            store
                .chapter_body_by_uid("other-book", "chapter-0")?
                .as_deref(),
            Some("Other book body")
        );
        assert!(store.chapter_body_by_uid("book-1", "missing")?.is_none());
        let mut duplicate = chapter(1, Some("Ambiguous body"));
        duplicate.id = "chapter-0".into();
        store.upsert_chapter(&duplicate)?;
        assert!(store.chapter_body_by_uid("book-1", "chapter-0").is_err());
        Ok(())
    }

    #[test]
    fn interrupted_task_recovers_exact_cursor_without_false_completion() -> Result<()> {
        let path = temporary_path("task");
        let mut task = DownloadTask {
            id: "task-1".into(),
            book_id: "book-1".into(),
            chapters: vec![0, 1],
            status: DownloadStatus::Running,
            next_index: 1,
            ..DownloadTask::default()
        };
        {
            let store = Store::open(&path)?;
            store.upsert_book(&book())?;
            store.upsert_chapters(&[chapter(0, Some("First body")), chapter(1, None)])?;
            store.save_task(&task)?;
        }
        let store = Store::open(&path)?;
        let recovered = store.pending_tasks()?.remove(0);
        assert_eq!(recovered.status, DownloadStatus::Paused);
        assert_eq!(recovered.next_index, 1);
        task.status = DownloadStatus::Complete;
        task.next_index = 2;
        assert!(store.save_task(&task).is_err());
        store.cache_chapter_body("book-1", 1, "Second body")?;
        store.save_task(&task)?;
        assert!(store.pending_tasks()?.is_empty());
        drop(store);
        let _ = fs::remove_file(path);
        Ok(())
    }

    fn checkpoint_fixture() -> Result<(Store, DownloadTask, Chapter)> {
        let store = Store::open(":memory:")?;
        store.upsert_book(&book())?;
        store.upsert_chapters(&[chapter(0, None), chapter(1, None)])?;
        let task = DownloadTask {
            id: "checkpoint-task".into(),
            book_id: "book-1".into(),
            account_key: "fixture-account".into(),
            chapters: vec![0, 1],
            status: DownloadStatus::Running,
            include_images: true,
            updated_at: 100,
            ..DownloadTask::default()
        };
        store.save_task(&task)?;
        let mut packet = chapter(0, Some("Complete checkpoint body"));
        packet.cached = true;
        packet.content_account_key = task.account_key.clone();
        Ok((store, task, packet))
    }

    fn access_fixture(
        status: ContentStatus,
        body: Option<&str>,
        owner: &str,
    ) -> Result<(Store, DownloadTask, Chapter)> {
        let (store, task, packet) = checkpoint_fixture()?;
        let mut remote = book();
        remote.source = "weread".into();
        remote.progress = 0.4;
        store.upsert_book(&remote)?;
        let mut previous = chapter(0, body);
        previous.content_status = status;
        previous.content_account_key = owner.into();
        previous.cached = has_full_body(&previous);
        store.upsert_chapter(&previous)?;
        Ok((store, task, packet))
    }

    #[test]
    fn limited_content_is_retained_without_advancing_or_counting_full_chapters() -> Result<()> {
        for status in [ContentStatus::Unknown, ContentStatus::Preview] {
            let (store, task, mut packet) = access_fixture(ContentStatus::Unknown, None, "")?;
            packet.body = Some("A readable limited excerpt...".into());
            packet.content_status = status;
            packet.cached = false;
            store.validate_download_chapter(&task, &packet, 0)?;
            assert!(!store.commit_download_checkpoint(&task, &packet, 0, 101)?);
            let saved = store.chapter("book-1", 0)?.unwrap();
            assert_eq!(saved.body, packet.body);
            assert_eq!(saved.content_status, status);
            assert_eq!(saved.content_account_key, task.account_key);
            assert!(!saved.cached);
            assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 0);
            assert_eq!(store.task(&task.id)?.unwrap().next_index, 0);
            let complete = DownloadTask {
                chapters: vec![0],
                next_index: 1,
                status: DownloadStatus::Complete,
                ..task
            };
            assert!(store.save_task(&complete).is_err());
        }
        Ok(())
    }

    #[test]
    fn locked_packets_preserve_previous_text_and_remove_only_full_access_count() -> Result<()> {
        for previous in [None, Some("An existing authorized body")] {
            let status = if previous.is_some() {
                ContentStatus::Full
            } else {
                ContentStatus::Unknown
            };
            let (store, task, mut packet) = access_fixture(status, previous, "fixture-account")?;
            packet.body = None;
            packet.content_status = ContentStatus::Locked;
            packet.cached = false;
            store.validate_download_chapter(&task, &packet, 0)?;
            assert!(!store.commit_download_checkpoint(&task, &packet, 0, 101)?);
            let saved = store.chapter("book-1", 0)?.unwrap();
            assert_eq!(saved.body.as_deref(), previous);
            assert_eq!(saved.content_status, ContentStatus::Locked);
            assert!(!saved.cached);
            assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 0);
            assert_eq!(store.task(&task.id)?.unwrap().next_index, 0);
        }
        Ok(())
    }

    #[test]
    fn authorized_prefix_extensions_preserve_notes_and_clamp_only_marker_anchors() -> Result<()> {
        for (status, owner) in [
            (ContentStatus::Preview, "fixture-account"),
            (ContentStatus::Unknown, "fixture-account"),
            (ContentStatus::Locked, "fixture-account"),
        ] {
            for marker in ["...", "\u{2026}"] {
                let prefix = "A \u{1f642} stable preview paragraph";
                let boundary = prefix.chars().count() as u32;
                let previous = format!("{prefix}{marker}");
                let (store, task, mut packet) = access_fixture(status, Some(&previous), owner)?;
                let note = Annotation {
                    id: "preserved-note".into(),
                    book_id: "book-1".into(),
                    chapter_index: 0,
                    start: 0,
                    end: 1,
                    quote: "A".into(),
                    note: "Preserve the thought".into(),
                    created_at: 4,
                };
                store.save_annotation(&note)?;
                store.save_position(&ReadingPosition {
                    book_id: "book-1".into(),
                    char_offset: boundary + 1,
                    updated_at: 8,
                    device: "fixture".into(),
                    ..Default::default()
                })?;
                store.save_bookmark(&Bookmark {
                    id: "prefix-bookmark".into(),
                    book_id: "book-1".into(),
                    char_offset: 2,
                    label: "Keep prefix".into(),
                    ..Default::default()
                })?;
                store.save_bookmark(&Bookmark {
                    id: "marker-bookmark".into(),
                    book_id: "book-1".into(),
                    char_offset: boundary + 1,
                    label: "Keep marker bookmark".into(),
                    ..Default::default()
                })?;
                packet.body = Some(format!("{prefix} plus the authorized full chapter text."));
                store.validate_download_chapter(&task, &packet, 1)?;
                assert!(store.commit_download_checkpoint(&task, &packet, 1, 101)?);
                assert_eq!(store.chapter_body("book-1", 0)?, packet.body);
                assert_eq!(store.annotations("book-1")?, vec![note]);
                let position = store.position("book-1")?.unwrap();
                assert_eq!(position.char_offset, boundary);
                assert_eq!(position.updated_at, 8);
                let bookmarks = store.bookmarks("book-1")?;
                assert_eq!(
                    bookmarks
                        .iter()
                        .find(|bookmark| bookmark.id == "prefix-bookmark")
                        .unwrap()
                        .char_offset,
                    2
                );
                assert_eq!(
                    bookmarks
                        .iter()
                        .find(|bookmark| bookmark.id == "marker-bookmark")
                        .unwrap()
                        .char_offset,
                    boundary
                );
                assert_eq!(store.book("book-1")?.unwrap().progress, 0.4);
                assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 1);
                assert_eq!(store.task(&task.id)?.unwrap().next_index, 1);
            }
        }
        Ok(())
    }

    #[test]
    fn marker_annotations_and_unsafe_replacements_are_preserved_as_content_conflicts() -> Result<()>
    {
        let prefix = "A stable prefix";
        let previous = format!("{prefix}...");
        let (store, task, mut packet) =
            access_fixture(ContentStatus::Preview, Some(&previous), "fixture-account")?;
        let boundary = prefix.chars().count() as u32;
        let note = Annotation {
            id: "marker-note".into(),
            book_id: "book-1".into(),
            start: boundary - 1,
            end: boundary + 1,
            quote: "x.".into(),
            note: "Retain this note".into(),
            ..Default::default()
        };
        store.save_annotation(&note)?;
        packet.body = Some(format!("{prefix} additional text"));
        let error = store
            .validate_download_chapter(&task, &packet, 1)
            .unwrap_err();
        assert!(error.downcast_ref::<ContentConflict>().is_some());
        assert!(
            store
                .commit_download_checkpoint(&task, &packet, 1, 101)
                .is_err()
        );
        assert_eq!(
            store.chapter_body("book-1", 0)?.as_deref(),
            Some(previous.as_str())
        );
        assert_eq!(store.annotations("book-1")?, vec![note]);
        assert_eq!(store.task(&task.id)?.unwrap(), task);

        for (old_status, owner, incoming, incoming_status) in [
            (
                ContentStatus::Unknown,
                "",
                "Original prefix extended",
                ContentStatus::Full,
            ),
            (
                ContentStatus::Preview,
                "fixture-account",
                "Unrelated replacement text",
                ContentStatus::Full,
            ),
            (
                ContentStatus::Preview,
                "another-account",
                "Original prefix extended",
                ContentStatus::Full,
            ),
            (
                ContentStatus::Full,
                "fixture-account",
                "Original prefix extended",
                ContentStatus::Full,
            ),
            (
                ContentStatus::Full,
                "fixture-account",
                "Original",
                ContentStatus::Preview,
            ),
        ] {
            let (store, task, mut packet) =
                access_fixture(old_status, Some("Original prefix"), owner)?;
            packet.body = Some(incoming.into());
            packet.content_status = incoming_status;
            packet.cached = incoming_status == ContentStatus::Full;
            let next = if packet.cached { 1 } else { 0 };
            assert!(
                store
                    .validate_download_chapter(&task, &packet, next)
                    .unwrap_err()
                    .downcast_ref::<ContentConflict>()
                    .is_some()
            );
            assert!(
                store
                    .commit_download_checkpoint(&task, &packet, next, 101)
                    .is_err()
            );
            assert_eq!(
                store.chapter_body("book-1", 0)?.as_deref(),
                Some("Original prefix")
            );
            assert_eq!(store.task(&task.id)?.unwrap(), task);
        }
        Ok(())
    }

    #[test]
    fn content_conflicts_and_marker_adjustments_roll_back_with_the_checkpoint() -> Result<()> {
        let previous = "Stable prefix...";
        let (store, task, mut packet) =
            access_fixture(ContentStatus::Preview, Some(previous), "fixture-account")?;
        store.save_position(&ReadingPosition {
            book_id: "book-1".into(),
            char_offset: 14,
            ..Default::default()
        })?;
        store.save_bookmark(&Bookmark {
            id: "bookmark".into(),
            book_id: "book-1".into(),
            char_offset: 14,
            ..Default::default()
        })?;
        packet.body = Some("Stable prefix and full source text".into());
        store.validate_download_chapter(&task, &packet, 1)?;
        store.connection.execute_batch("CREATE TEMP TRIGGER reject_content_checkpoint BEFORE UPDATE ON download_tasks BEGIN SELECT RAISE(ABORT,'fixture failure'); END")?;
        assert!(
            store
                .commit_download_checkpoint(&task, &packet, 1, 101)
                .is_err()
        );
        assert_eq!(store.chapter_body("book-1", 0)?.as_deref(), Some(previous));
        assert_eq!(
            store.chapter("book-1", 0)?.unwrap().content_status,
            ContentStatus::Preview
        );
        assert_eq!(store.position("book-1")?.unwrap().char_offset, 14);
        assert_eq!(store.bookmarks("book-1")?[0].char_offset, 14);
        assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 0);
        assert_eq!(store.task(&task.id)?.unwrap(), task);
        Ok(())
    }

    #[test]
    fn account_switch_invalidates_access_proof_without_removing_content_or_reviving_cancelled_tasks()
    -> Result<()> {
        let (store, task, packet) = access_fixture(ContentStatus::Unknown, None, "")?;
        store.commit_download_checkpoint(&task, &packet, 1, 101)?;
        let complete = DownloadTask {
            id: "finished-task".into(),
            chapters: vec![0],
            next_index: 1,
            status: DownloadStatus::Complete,
            ..task.clone()
        };
        store.save_task(&complete)?;
        let cancelled = DownloadTask {
            id: "cancelled-task".into(),
            status: DownloadStatus::Cancelled,
            ..task
        };
        store.save_task(&cancelled)?;
        let mut local = book();
        local.id = "local-book".into();
        store.upsert_book(&local)?;
        let local_chapter = Chapter {
            id: "local-chapter".into(),
            book_id: local.id.clone(),
            body: Some("Local source text".into()),
            content_status: ContentStatus::Full,
            cached: true,
            ..Default::default()
        };
        store.upsert_chapter(&local_chapter)?;
        store.invalidate_content_ownership(Some("fixture-account"))?;
        assert!(store.chapter("book-1", 0)?.unwrap().cached);
        assert_eq!(
            store.task(&complete.id)?.unwrap().status,
            DownloadStatus::Complete
        );
        store.invalidate_content_ownership(Some("replacement-account"))?;
        let saved = store.chapter("book-1", 0)?.unwrap();
        assert_eq!(saved.body, packet.body);
        assert_eq!(saved.content_status, ContentStatus::Unknown);
        assert_eq!(saved.content_account_key, "fixture-account");
        assert!(!saved.cached);
        assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 0);
        let waiting = store.task(&complete.id)?.unwrap();
        assert_eq!(waiting.status, DownloadStatus::Paused);
        assert_eq!(waiting.next_index, 0);
        assert_eq!(waiting.error.as_deref(), Some("source.content_unverified"));
        assert_eq!(store.task(&cancelled.id)?.unwrap(), cancelled);
        assert!(store.chapter(&local.id, 0)?.unwrap().cached);
        store.invalidate_content_ownership(None)?;
        assert_eq!(store.task(&cancelled.id)?.unwrap(), cancelled);
        Ok(())
    }

    #[test]
    fn checkpoint_commits_body_cursor_and_incremental_count_without_rewriting_plan() -> Result<()> {
        let (store, task, mut packet) = checkpoint_fixture()?;
        store.connection.execute(
            "UPDATE download_tasks SET payload=json_set(payload,'$.fixture_extension','keep') WHERE id=?1",
            [&task.id],
        )?;
        packet.title = "Stale worker title".into();
        store.commit_download_checkpoint(&task, &packet, 1, 101)?;
        let stored = store.task(&task.id)?.unwrap();
        assert_eq!(stored.next_index, 1);
        assert_eq!(stored.updated_at, 101);
        assert_eq!(stored.status, DownloadStatus::Running);
        assert_eq!(stored.chapters, task.chapters);
        assert_eq!(stored.account_key, task.account_key);
        assert_eq!(stored.include_images, task.include_images);
        assert_eq!(store.chapter_body("book-1", 0)?, packet.body);
        assert_eq!(store.chapter("book-1", 0)?.unwrap().title, "Chapter 0");
        assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 1);
        let extension: String = store.connection.query_row(
            "SELECT json_extract(payload,'$.fixture_extension') FROM download_tasks WHERE id=?1",
            [&task.id],
            |row| row.get(0),
        )?;
        assert_eq!(extension, "keep");

        let repeated = DownloadTask {
            id: "checkpoint-image-complement".into(),
            chapters: vec![0],
            ..task
        };
        store.save_task(&repeated)?;
        store.commit_download_checkpoint(&repeated, &packet, 1, 102)?;
        assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 1);
        assert_eq!(store.task(&repeated.id)?.unwrap().next_index, 1);
        Ok(())
    }

    #[test]
    fn checkpoint_failure_rolls_back_body_count_and_cursor_together() -> Result<()> {
        let (store, task, packet) = checkpoint_fixture()?;
        store.connection.execute_batch(
            "CREATE TEMP TRIGGER reject_checkpoint BEFORE UPDATE ON download_tasks
             BEGIN SELECT RAISE(ABORT,'fixture checkpoint failure'); END",
        )?;
        assert!(
            store
                .commit_download_checkpoint(&task, &packet, 1, 101)
                .is_err()
        );
        assert_eq!(store.chapter_body("book-1", 0)?, None);
        assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 0);
        assert_eq!(store.task(&task.id)?.unwrap(), task);
        store
            .connection
            .execute_batch("DROP TRIGGER reject_checkpoint")?;
        store.commit_download_checkpoint(&task, &packet, 1, 101)?;
        assert_eq!(store.chapter_body("book-1", 0)?, packet.body);
        assert_eq!(store.task(&task.id)?.unwrap().next_index, 1);
        Ok(())
    }

    #[test]
    fn checkpoint_rejects_stale_persistent_task_scope_and_plan() -> Result<()> {
        for mismatch in [
            "account",
            "book",
            "status",
            "cursor",
            "selected_chapter",
            "plan_length",
            "include_images",
            "updated_at",
            "payload_status",
            "payload_book",
        ] {
            let (store, task, packet) = checkpoint_fixture()?;
            let mut changed = task.clone();
            match mismatch {
                "account" => changed.account_key = "different-fixture-account".into(),
                "book" => {
                    store.upsert_book(&Book::new("other-book", "Other fixture book"))?;
                    changed.book_id = "other-book".into();
                }
                "status" => changed.status = DownloadStatus::Paused,
                "cursor" => changed.next_index = 1,
                "selected_chapter" => changed.chapters = vec![1, 0],
                "plan_length" => changed.chapters = vec![0],
                "include_images" => changed.include_images = false,
                "updated_at" => changed.updated_at = 101,
                "payload_status" | "payload_book" => {}
                _ => unreachable!(),
            }
            store.save_task(&changed)?;
            if mismatch == "payload_status" {
                store.connection.execute(
                    "UPDATE download_tasks SET payload=json_set(payload,'$.status','paused') WHERE id=?1",
                    [&task.id],
                )?;
            } else if mismatch == "payload_book" {
                store.connection.execute(
                    "UPDATE download_tasks SET payload=json_set(payload,'$.book_id','other-book') WHERE id=?1",
                    [&task.id],
                )?;
            }
            assert!(
                store
                    .commit_download_checkpoint(&task, &packet, 1, 102)
                    .is_err(),
                "A changed {mismatch} must reject a stale worker packet",
            );
            assert_eq!(store.chapter_body("book-1", 0)?, None);
            assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 0);
        }
        Ok(())
    }

    #[test]
    fn checkpoint_rejects_changed_catalog_identity_and_canonical_text() -> Result<()> {
        let (store, task, packet) = checkpoint_fixture()?;
        let mut replacement = chapter(0, None);
        replacement.id = "replacement-chapter".into();
        store.upsert_chapter(&replacement)?;
        assert!(
            store
                .commit_download_checkpoint(&task, &packet, 1, 101)
                .is_err()
        );
        assert_eq!(store.chapter_body("book-1", 0)?, None);
        assert_eq!(store.task(&task.id)?.unwrap().next_index, 0);

        let (store, task, packet) = checkpoint_fixture()?;
        store.cache_chapter_body("book-1", 0, "Preserved canonical chapter")?;
        store.save_annotation(&Annotation {
            id: "checkpoint-note".into(),
            book_id: "book-1".into(),
            start: 0,
            end: 9,
            quote: "Preserved".into(),
            ..Annotation::default()
        })?;
        assert!(
            store
                .commit_download_checkpoint(&task, &packet, 1, 101)
                .is_err()
        );
        assert_eq!(
            store.chapter_body("book-1", 0)?.as_deref(),
            Some("Preserved canonical chapter")
        );
        assert_eq!(store.annotations("book-1")?[0].quote, "Preserved");
        assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 1);
        assert_eq!(store.task(&task.id)?.unwrap().next_index, 0);
        Ok(())
    }

    #[test]
    fn checkpoint_requires_an_exact_next_item_and_nonempty_cached_body() -> Result<()> {
        for mismatch in ["skip", "repeat", "empty", "uncached", "chapter", "book"] {
            let (store, task, mut packet) = checkpoint_fixture()?;
            let mut next_index = 1;
            match mismatch {
                "skip" => next_index = 2,
                "repeat" => next_index = 0,
                "empty" => packet.body = Some(" \n ".into()),
                "uncached" => packet.cached = false,
                "chapter" => packet.index = 1,
                "book" => packet.book_id = "other-book".into(),
                _ => unreachable!(),
            }
            assert!(
                store
                    .commit_download_checkpoint(&task, &packet, next_index, 101)
                    .is_err()
            );
            assert_eq!(store.chapter_body("book-1", 0)?, None);
            assert_eq!(store.task(&task.id)?.unwrap().next_index, 0);
        }
        Ok(())
    }

    #[test]
    #[ignore = "Run this isolated metadata benchmark explicitly on the designated remote environment"]
    fn benchmark_download_checkpoint_metadata() -> Result<()> {
        use std::time::Instant;
        let counts = match std::env::var("WEREAD_DOWNLOAD_BENCH_CHAPTERS") {
            Ok(value) => value
                .split(',')
                .map(|count| count.trim().parse::<usize>())
                .collect::<std::result::Result<Vec<_>, _>>()?,
            Err(_) => vec![111, 1000],
        };
        ensure!(
            !counts.is_empty() && counts.iter().all(|count| matches!(*count, 111 | 1000)),
            "select 111, 1000, or both fixture chapter counts"
        );
        let body = "Synthetic download metadata benchmark text. ".repeat(160);
        for count in counts {
            let directory = tempfile::tempdir()?;
            let baseline = Store::open(directory.path().join("baseline.sqlite"))?;
            let checkpoint = Store::open(directory.path().join("checkpoint.sqlite"))?;
            let mut book = Book::new("benchmark-book", "Isolated synthetic download benchmark");
            book.total_chapters = count as u32;
            let chapters: Vec<_> = (0..count)
                .map(|index| Chapter {
                    id: format!("benchmark-chapter-{index}"),
                    book_id: book.id.clone(),
                    index: index as u32,
                    title: format!("Synthetic chapter {index}"),
                    ..Chapter::default()
                })
                .collect();
            let task = DownloadTask {
                id: "benchmark-task".into(),
                book_id: book.id.clone(),
                account_key: "synthetic-benchmark-account".into(),
                chapters: (0..count as u32).collect(),
                status: DownloadStatus::Running,
                updated_at: 100,
                ..DownloadTask::default()
            };
            for store in [&baseline, &checkpoint] {
                store.upsert_book(&book)?;
                store.upsert_chapters(&chapters)?;
                store.save_task(&task)?;
            }
            let mut baseline_task = task.clone();
            let started = Instant::now();
            for chapter in &chapters {
                let mut chapter = chapter.clone();
                chapter.body = Some(body.clone());
                chapter.cached = true;
                chapter.content_status = ContentStatus::Full;
                chapter.content_account_key = baseline_task.account_key.clone();
                baseline.upsert_chapter(&chapter)?;
                let mut updated = baseline_task.clone();
                updated.next_index += 1;
                updated.updated_at = 100 + updated.next_index as i64;
                baseline.save_task(&updated)?;
                baseline_task = updated;
            }
            let baseline_elapsed = started.elapsed();
            let mut checkpoint_task = task.clone();
            let started = Instant::now();
            for chapter in &chapters {
                let mut chapter = chapter.clone();
                chapter.body = Some(body.clone());
                chapter.cached = true;
                chapter.content_status = ContentStatus::Full;
                chapter.content_account_key = checkpoint_task.account_key.clone();
                let next_index = checkpoint_task.next_index + 1;
                let updated_at = 100 + next_index as i64;
                checkpoint.commit_download_checkpoint(
                    &checkpoint_task,
                    &chapter,
                    next_index,
                    updated_at,
                )?;
                checkpoint_task.next_index = next_index;
                checkpoint_task.updated_at = updated_at;
            }
            let checkpoint_elapsed = started.elapsed();
            assert_eq!(baseline.task(&task.id)?, checkpoint.task(&task.id)?);
            assert_eq!(baseline.task(&task.id)?.unwrap().next_index, count);
            assert_eq!(
                baseline.book(&book.id)?.unwrap().cached_chapters,
                count as u32
            );
            assert_eq!(
                checkpoint.book(&book.id)?.unwrap().cached_chapters,
                count as u32
            );
            assert_eq!(baseline.chapters(&book.id)?, checkpoint.chapters(&book.id)?);
            assert_eq!(baseline.summary()?, checkpoint.summary()?);
            println!(
                "{}",
                serde_json::json!({
                    "fixture": "isolated_download_checkpoint_metadata",
                    "chapters": count,
                    "body_bytes_per_chapter": body.len(),
                    "verified_equivalent": true,
                    "baseline": {
                        "elapsed_ms": baseline_elapsed.as_secs_f64() * 1000.0,
                        "body_writes": count,
                        "task_writes": count,
                        "metadata_mutations": count * 3,
                        "metadata_transactions": count * 2,
                        "rust_plan_clones": count,
                        "rust_plan_serializations": count,
                        "cached_count_rescans": count,
                    },
                    "checkpoint": {
                        "elapsed_ms": checkpoint_elapsed.as_secs_f64() * 1000.0,
                        "body_writes": count,
                        "task_writes": count,
                        "metadata_mutations": count * 3,
                        "metadata_transactions": count,
                        "rust_plan_clones": 0,
                        "rust_plan_serializations": count,
                        "cached_count_rescans": 0,
                    },
                    "operation_count_note": "API and SQL mutation counts, not measured fsync counts",
                })
            );
        }
        Ok(())
    }

    #[test]
    fn clearing_content_keeps_user_notes_and_position() -> Result<()> {
        let store = Store::open(":memory:")?;
        store.upsert_book(&book())?;
        store.upsert_chapter(&chapter(0, Some("An annotation sample.")))?;
        store.save_position(&ReadingPosition {
            book_id: "book-1".into(),
            char_offset: 4,
            ..ReadingPosition::default()
        })?;
        store.save_annotation(&Annotation {
            id: "note-1".into(),
            book_id: "book-1".into(),
            start: 3,
            end: 13,
            quote: "annotation".into(),
            ..Annotation::default()
        })?;
        store.clear_book_cache("book-1")?;
        assert_eq!(store.position("book-1")?.unwrap().char_offset, 4);
        assert_eq!(store.annotations("book-1")?.len(), 1);
        assert_eq!(store.book("book-1")?.unwrap().cached_chapters, 0);
        Ok(())
    }

    #[test]
    fn search_history_is_bounded_and_recency_ordered() -> Result<()> {
        let store = Store::open(":memory:")?;
        for index in 0..30 {
            store.remember_query(&format!("query-{index}"))?;
        }
        store.remember_query("query-12")?;
        let queries = store.recent_queries(100)?;
        assert_eq!(queries.len(), 20);
        assert_eq!(queries[0], "query-12");
        assert!(!queries.contains(&"query-0".to_owned()));
        Ok(())
    }
}
