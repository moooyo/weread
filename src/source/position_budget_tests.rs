//! Genuine progress transport and persisted-position admission observations.

use super::api_gate_test_support::{HttpFixture, Reply, auth};
use super::*;
use crate::source::{ReadingPositionLimits, ReadingPositionSettingError, ReadingPositionUsage};
use crate::store::{Store, StoredSettingReadLimits};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stats {
    pub request_constructors: usize,
    pub search_nodes: usize,
    pub book_comparisons: usize,
    pub number_formats: usize,
    pub numeric_parses: usize,
    pub materializations: usize,
    pub field_copies: usize,
    pub copied_bytes: usize,
    pub setting_scans: usize,
    pub setting_decodes: usize,
    pub usage: ReadingPositionUsage,
}

thread_local! {
    static LIMITS: Cell<Option<ReadingPositionLimits>> = const { Cell::new(None) };
    static STATS: Cell<Stats> = Cell::new(Stats::default());
}

pub(crate) struct ObservationScope {
    limits: Option<ReadingPositionLimits>,
    stats: Stats,
}

impl ObservationScope {
    pub(crate) fn start(limits: ReadingPositionLimits) -> Self {
        Self {
            limits: LIMITS.with(|slot| slot.replace(Some(limits))),
            stats: STATS.with(|slot| slot.replace(Stats::default())),
        }
    }

    pub(crate) fn stats(&self) -> Stats {
        STATS.with(Cell::get)
    }
}

impl Drop for ObservationScope {
    fn drop(&mut self) {
        LIMITS.with(|slot| slot.set(self.limits));
        STATS.with(|slot| slot.set(self.stats));
    }
}

pub(crate) fn limits() -> ReadingPositionLimits {
    LIMITS.with(Cell::get).unwrap_or_default()
}

fn observe(update: impl FnOnce(&mut Stats)) {
    if LIMITS.with(Cell::get).is_some() {
        STATS.with(|slot| {
            let mut stats = slot.get();
            update(&mut stats);
            slot.set(stats);
        });
    }
}

pub(crate) fn observe_usage(usage: ReadingPositionUsage) {
    observe(|stats| stats.usage = usage);
}
pub(crate) fn observe_request_constructor() {
    observe(|stats| stats.request_constructors += 1);
}
pub(crate) fn observe_search_node() {
    observe(|stats| stats.search_nodes += 1);
}
pub(crate) fn observe_book_comparison() {
    observe(|stats| stats.book_comparisons += 1);
}
pub(crate) fn observe_number_format() {
    observe(|stats| stats.number_formats += 1);
}
pub(crate) fn observe_numeric_parse() {
    observe(|stats| stats.numeric_parses += 1);
}
pub(crate) fn observe_materialization() {
    observe(|stats| stats.materializations += 1);
}
pub(crate) fn observe_setting_scan() {
    observe(|stats| stats.setting_scans += 1);
}
pub(crate) fn observe_setting_decode() {
    observe(|stats| stats.setting_decodes += 1);
}
pub(crate) fn observe_field_copy(bytes: usize) {
    observe(|stats| {
        stats.field_copies += 1;
        stats.copied_bytes += bytes;
    });
}

fn progress_value() -> Value {
    json!({
        "bookId":"book", "chapterUid":"中🙂", "chapterIdx":7,
        "chapterOffset":18, "progress":"22.75", "summary":"中🙂".repeat(32),
        "updateTime":"90", "deviceName":"terminal中"
    })
}

fn progress_fixture(value: Value) -> HttpFixture {
    HttpFixture::new(move |request| {
        assert_eq!(request.path(), "/gateway");
        assert_eq!(request.method, "POST");
        assert_eq!(request.json()["api_name"], "/book/getprogress");
        Reply::json(value.clone())
    })
}

fn capacity<T>(result: Result<T>) {
    assert!(matches!(result, Err(SourceError::ResponseTooLarge)));
    assert!(!SourceError::ResponseTooLarge.requires_login());
    assert!(!SourceError::ResponseTooLarge.retryable());
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_progress_exact_unicode_fields_precede_all_position_copies() {
    let value = progress_value();
    let uid = value["chapterUid"].as_str().unwrap();
    let device = value["deviceName"].as_str().unwrap();
    for (uid_bytes, device_bytes, admitted) in [
        (uid.len(), device.len(), true),
        (uid.len() - 1, device.len(), false),
        (uid.len(), device.len() - 1, false),
    ] {
        let fixture = progress_fixture(value.clone());
        let client = fixture.client(auth());
        let before = client.session();
        let scope = ObservationScope::start(ReadingPositionLimits {
            uid_bytes,
            device_bytes,
            ..Default::default()
        });
        let result = client.progress("book").await;
        if admitted {
            let position = result.unwrap();
            assert_eq!(position.chapter_uid, uid);
            assert_eq!(position.device, device);
            assert_eq!(position.summary, "中🙂".repeat(32));
            assert!(position.summary.chars().count() > 20);
            assert_eq!(
                (
                    position.chapter_index,
                    position.chapter_offset,
                    position.progress
                ),
                (7, 18, 22)
            );
            assert_eq!(position.updated_at, 90);
            assert_eq!(scope.stats().materializations, 1);
            assert_eq!(scope.stats().field_copies, 3);
            assert_eq!(
                scope.stats().copied_bytes,
                uid.len() + device.len() + position.summary.len()
            );
        } else {
            capacity(result);
            assert_eq!(scope.stats().materializations, 0);
            assert_eq!(scope.stats().field_copies, 0);
            assert_eq!(scope.stats().copied_bytes, 0);
            assert_eq!(scope.stats().number_formats, 0);
        }
        assert_eq!(scope.stats().request_constructors, 1);
        assert_eq!(client.session(), before);
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_progress_complete_metadata_and_work_are_exact_and_atomic() {
    let value = progress_value();
    let initial = progress_fixture(value.clone());
    let client = initial.client(auth());
    let positive = ObservationScope::start(ReadingPositionLimits::default());
    client.progress("book").await.unwrap();
    let usage = positive.stats().usage;
    assert_eq!(positive.stats().field_copies, 3);
    assert_eq!(initial.finish().len(), 1);
    drop(positive);
    for limits in [
        ReadingPositionLimits {
            metadata_bytes: usage.metadata_bytes,
            work_bytes: usage.work_bytes,
            ..Default::default()
        },
        ReadingPositionLimits {
            metadata_bytes: usage.metadata_bytes - 1,
            ..Default::default()
        },
        ReadingPositionLimits {
            work_bytes: usage.work_bytes - 1,
            ..Default::default()
        },
    ] {
        let fixture = progress_fixture(value.clone());
        let client = fixture.client(auth());
        let before = client.session();
        let scope = ObservationScope::start(limits);
        let result = client.progress("book").await;
        if limits.metadata_bytes == usage.metadata_bytes && limits.work_bytes == usage.work_bytes {
            let position = result.unwrap();
            assert_eq!(position.summary, value["summary"].as_str().unwrap());
            assert_eq!(scope.stats().field_copies, 3);
        } else {
            capacity(result);
            assert_eq!(scope.stats().materializations, 0);
            assert_eq!(scope.stats().field_copies, 0);
            assert_eq!(scope.stats().copied_bytes, 0);
        }
        assert_eq!(client.session(), before);
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_progress_alias_precedence_numeric_text_and_empty_fields_keep_selection() {
    let value = json!({
        "a":{"book_id":"other", "progress":90},
        "b":{
            "book_id":"book", "readingProgress":"invalid", "bookProgress":"25.9",
            "chapterUid":null, "chapterId":17, "chapter_uid":"u".repeat(4097),
            "chapterIdx":null, "chapterIndex":90, "chapterOffset":"invalid", "offset":55,
            "summary":"", "chapterTitle":"unchosen", "updateTime":"invalid", "updatedAt":90,
            "deviceName":8, "device":"d".repeat(4097)
        },
        "c":{"bookId":"book", "progress":99, "chapterUid":"unchosen"}
    });
    let fixture = progress_fixture(value);
    let client = fixture.client(auth());
    let before = client.session();
    let scope = ObservationScope::start(ReadingPositionLimits::default());
    let position = client.progress("book").await.unwrap();
    assert_eq!(position.chapter_uid, "17");
    assert_eq!(position.device, "8");
    assert_eq!(position.summary, "");
    assert_eq!(
        (
            position.chapter_index,
            position.chapter_offset,
            position.progress,
            position.updated_at
        ),
        (0, 0, 25, 90)
    );
    assert_eq!(scope.stats().number_formats, 2);
    assert_eq!(scope.stats().field_copies, 3);
    assert_eq!(scope.stats().copied_bytes, 3);
    assert_eq!(client.session(), before);
    assert_eq!(fixture.finish().len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_progress_prices_visited_book_ids_and_request_ids_before_copy() {
    let response = json!([
        {"bookId":"x".repeat(4097), "progress":1}, progress_value()
    ]);
    let fixture = progress_fixture(response);
    let client = fixture.client(auth());
    let before = client.session();
    let scope = ObservationScope::start(ReadingPositionLimits::default());
    capacity(client.progress("book").await);
    assert_eq!(scope.stats().field_copies, 0);
    assert_eq!(scope.stats().materializations, 0);
    assert_eq!(client.session(), before);
    assert_eq!(fixture.finish().len(), 1);
    drop(scope);
    let fixture = progress_fixture(progress_value());
    let client = fixture.client(auth());
    let before = client.session();
    let scope = ObservationScope::start(ReadingPositionLimits::default());
    capacity(client.progress(&"b".repeat(4097)).await);
    assert_eq!(scope.stats().request_constructors, 0);
    assert_eq!(scope.stats().search_nodes, 0);
    assert_eq!(scope.stats().field_copies, 0);
    assert_eq!(client.session(), before);
    assert!(fixture.finish().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_progress_invalid_values_and_missing_books_keep_the_original_errors() {
    for (value, expected) in [
        (
            json!({"bookId":"book", "progress":"NaN", "readingProgress":12, "chapterUid":"uid"}),
            "progress_value",
        ),
        (
            json!({"bookId":"book", "progress":"NaN", "chapterUid":"u".repeat(4097)}),
            "progress_value",
        ),
        (
            json!({"bookId":"other", "progress":12}),
            "progress_not_found",
        ),
        (
            json!({"bookId":"book", "readingProgress":null}),
            "progress_value",
        ),
    ] {
        let fixture = progress_fixture(value);
        let client = fixture.client(auth());
        let before = client.session();
        let scope = ObservationScope::start(ReadingPositionLimits::default());
        assert!(
            matches!(client.progress("book").await, Err(SourceError::InvalidResponse(actual)) if actual == expected)
        );
        assert_eq!(scope.stats().field_copies, 0);
        assert_eq!(scope.stats().materializations, 0);
        assert_eq!(client.session(), before);
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_progress_numeric_input_is_priced_before_parse_and_materialization() {
    let mut value = progress_value();
    value["progress"] = json!("9".repeat(4097));
    let fixture = progress_fixture(value);
    let client = fixture.client(auth());
    let before = client.session();
    let scope = ObservationScope::start(ReadingPositionLimits::default());
    capacity(client.progress("book").await);
    assert_eq!(scope.stats().numeric_parses, 0);
    assert_eq!(scope.stats().number_formats, 0);
    assert_eq!(scope.stats().field_copies, 0);
    assert_eq!(client.session(), before);
    assert_eq!(fixture.finish().len(), 1);
}

fn stored_position() -> ReadingPosition {
    ReadingPosition {
        chapter_uid: "uid\0中".into(),
        chapter_index: 7,
        chapter_offset: 18,
        progress: 22,
        summary: " full 中🙂 ".repeat(64),
        updated_at: 90,
        device: "device\0中".into(),
    }
}

#[test]
fn genuine_stored_position_exact_fields_preserve_full_summary_and_original_setting()
-> anyhow::Result<()> {
    let store = Store::open(":memory:")?;
    let position = stored_position();
    const KEY: &str = "progress.pending:book";
    store.set_setting(KEY, &position)?;
    for summary_bytes in [position.summary.len(), position.summary.len() - 1] {
        let scope = ObservationScope::start(ReadingPositionLimits {
            summary_bytes,
            ..Default::default()
        });
        let result = store.with_bounded_setting_text(
            KEY,
            StoredSettingReadLimits::default(),
            |text, metadata| {
                assert!(metadata.encoded_bytes > 0);
                assert!(metadata.utf8_bytes_upper >= text.len());
                Ok(crate::source::decode_reading_position_setting(text)?)
            },
        );
        if summary_bytes == position.summary.len() {
            let loaded = result?.unwrap();
            assert_eq!(
                serde_json::to_value(&loaded)?,
                serde_json::to_value(&position)?
            );
            assert!(loaded.summary.chars().count() > 20);
            assert_eq!(scope.stats().setting_scans, 1);
            assert_eq!(scope.stats().setting_decodes, 1);
            assert_eq!(scope.stats().field_copies, 3);
        } else {
            let error = result.unwrap_err();
            assert!(
                error
                    .downcast_ref::<ReadingPositionSettingError>()
                    .is_some_and(ReadingPositionSettingError::is_capacity)
            );
            assert_eq!(scope.stats().setting_decodes, 0);
            assert_eq!(scope.stats().field_copies, 0);
            assert_eq!(scope.stats().copied_bytes, 0);
        }
        drop(scope);
        assert_eq!(
            serde_json::to_value(store.setting::<ReadingPosition>(KEY)?.unwrap())?,
            serde_json::to_value(&position)?
        );
    }
    Ok(())
}

#[test]
fn genuine_stored_position_complete_budget_and_zero_budget_never_copy_partial_fields()
-> anyhow::Result<()> {
    let store = Store::open(":memory:")?;
    let position = stored_position();
    const KEY: &str = "progress.pending:book";
    store.set_setting(KEY, &position)?;
    let positive = ObservationScope::start(ReadingPositionLimits::default());
    store
        .with_bounded_setting_text(KEY, StoredSettingReadLimits::default(), |text, _| {
            Ok(crate::source::decode_reading_position_setting(text)?)
        })?
        .unwrap();
    let usage = positive.stats().usage;
    drop(positive);
    for limits in [
        ReadingPositionLimits {
            metadata_bytes: usage.metadata_bytes,
            work_bytes: usage.work_bytes,
            ..Default::default()
        },
        ReadingPositionLimits {
            metadata_bytes: usage.metadata_bytes - 1,
            ..Default::default()
        },
        ReadingPositionLimits {
            work_bytes: usage.work_bytes - 1,
            ..Default::default()
        },
        ReadingPositionLimits {
            metadata_bytes: 0,
            ..Default::default()
        },
        ReadingPositionLimits {
            work_bytes: 0,
            ..Default::default()
        },
    ] {
        let scope = ObservationScope::start(limits);
        let result =
            store.with_bounded_setting_text(KEY, StoredSettingReadLimits::default(), |text, _| {
                Ok(crate::source::decode_reading_position_setting(text)?)
            });
        if limits.metadata_bytes == usage.metadata_bytes && limits.work_bytes == usage.work_bytes {
            assert_eq!(result?.unwrap().summary, position.summary);
            assert_eq!(scope.stats().field_copies, 3);
        } else {
            assert!(
                result
                    .unwrap_err()
                    .downcast_ref::<ReadingPositionSettingError>()
                    .is_some_and(ReadingPositionSettingError::is_capacity)
            );
            assert_eq!(scope.stats().setting_decodes, 0);
            assert_eq!(scope.stats().field_copies, 0);
        }
        drop(scope);
        assert_eq!(
            store.setting::<ReadingPosition>(KEY)?.unwrap().summary,
            position.summary
        );
    }
    Ok(())
}

#[test]
fn setting_preflight_prices_escaped_and_duplicate_occurrences_without_whole_value_decode() {
    let valid = serde_json::to_string(&stored_position()).unwrap();
    let early = format!("\"{}\"", "u".repeat(4097));
    for input in [
        valid.replace(
            "\"chapter_uid\":",
            &format!("\"chapter_uid\":{early},\"chapter_uid\":"),
        ),
        valid.replace(
            "\"chapter_uid\":",
            &format!("\"chapter_\\u0075id\":{early},\"chapter_uid\":"),
        ),
        format!("{{\"chapter_uid\":{early},broken"),
        format!("{{\"chapter_uid\":\"{}", "u".repeat(4097)),
    ] {
        let scope = ObservationScope::start(ReadingPositionLimits::default());
        assert!(matches!(
            crate::source::decode_reading_position_setting(&input),
            Err(ReadingPositionSettingError::Limit(_))
        ));
        assert_eq!(scope.stats().setting_decodes, 0);
        assert_eq!(scope.stats().field_copies, 0);
        assert_eq!(scope.stats().materializations, 0);
    }
}

#[test]
fn setting_schema_keeps_unknown_fields_empty_strings_and_original_json_failures() {
    let mut position = stored_position();
    position.chapter_uid.clear();
    position.summary.clear();
    let valid = serde_json::to_string(&position).unwrap();
    let input = valid
        .replacen(
            '{',
            &format!("{{\"legacy\":{{\"chapter_uid\":\"{}\"}},", "u".repeat(4097)),
            1,
        )
        .replace("\"device\":", "\"d\\u0065vice\":");
    let scope = ObservationScope::start(ReadingPositionLimits::default());
    let loaded = crate::source::decode_reading_position_setting(&input).unwrap();
    assert_eq!(
        serde_json::to_value(loaded).unwrap(),
        serde_json::to_value(&position).unwrap()
    );
    assert_eq!(scope.stats().setting_decodes, 1);
    drop(scope);
    for input in ["{}", "{\"chapter_uid\":false}", "{broken}"] {
        let scope = ObservationScope::start(ReadingPositionLimits::default());
        assert!(matches!(
            crate::source::decode_reading_position_setting(input),
            Err(ReadingPositionSettingError::Json(_))
        ));
        assert_eq!(scope.stats().materializations, 0);
        assert_eq!(scope.stats().field_copies, 0);
    }
}
