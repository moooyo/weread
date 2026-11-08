//! Real small SQLite rows exercise annotation metadata and selected payloads.

use super::*;
use std::cell::{Cell, RefCell};

thread_local! {
    static PREFLIGHT_HOOK: RefCell<Option<Box<dyn FnOnce() -> Result<()>>>> = const { RefCell::new(None) };
    static UPDATE_LIMIT: Cell<Option<usize>> = const { Cell::new(None) };
}

pub(super) fn after_preflight() -> Result<()> {
    let hook = PREFLIGHT_HOOK.with(|hook| hook.borrow_mut().take());
    if let Some(hook) = hook {
        hook()?;
    }
    Ok(())
}

pub(super) fn update_work_limit() -> Option<usize> {
    UPDATE_LIMIT.with(Cell::get)
}

struct HookGuard;

impl HookGuard {
    fn start(hook: impl FnOnce() -> Result<()> + 'static) -> Self {
        PREFLIGHT_HOOK.with(|state| {
            assert!(state.borrow().is_none());
            *state.borrow_mut() = Some(Box::new(hook));
        });
        Self
    }
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        PREFLIGHT_HOOK.with(|state| state.borrow_mut().take());
        UPDATE_LIMIT.with(|limit| limit.set(None));
    }
}

fn fixture() -> Result<(tempfile::TempDir, Store)> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path().join("metadata.sqlite3"))?;
    let mut book = Book::new("book", "Annotation fixture");
    book.source = "local".into();
    store.upsert_book(&book)?;
    store.upsert_chapter(&Chapter {
        id: "uid".into(),
        book_id: "book".into(),
        index: 0,
        title: "Fixture chapter".into(),
        body: Some("012345678901234567890123456789012345678901234567890123456789012345".into()),
        ..Chapter::default()
    })?;
    Ok((directory, store))
}

fn note(
    store: &Store,
    id: &str,
    start: u32,
    end: u32,
    quote: &str,
    text: &str,
    created_at: i64,
) -> Result<()> {
    store.save_annotation(&Annotation {
        id: id.into(),
        book_id: "book".into(),
        chapter_index: 0,
        start,
        end,
        quote: quote.into(),
        note: text.into(),
        created_at,
    })
}

fn key(store: &Store, id: &str) -> Result<AnnotationKey> {
    Ok(store.annotation_header_by_id("book", id)?.unwrap().key)
}

#[test]
fn count_geometry_and_presence_never_materialize_annotation_ids_or_text() -> Result<()> {
    let (_directory, store) = fixture()?;
    note(&store, "a", 0, 2, " ", &"n".repeat(4096), 10)?;
    note(&store, "b", 3, 5, "", &"n".repeat(4096), 20)?;
    note(&store, "c", 8, 9, "outside", "outside", 30)?;
    store.reset_annotation_read_stats();
    assert_eq!(store.annotation_count("book")?, 3);
    assert_eq!(store.annotation_count_for_range("book", 0, 0, 4)?, 2);
    assert!(store.annotation_has_quote_for_range("book", 0, 0, 4)?);
    let geometry = store.annotation_geometry_for_range("book", 0, 0, 4)?;
    assert_eq!(
        geometry
            .iter()
            .map(|note| (note.start, note.end))
            .collect::<Vec<_>>(),
        [(0, 2), (3, 5)]
    );
    assert_eq!(
        store.annotation_read_stats(),
        AnnotationReadStats::default()
    );
    Ok(())
}

#[test]
fn header_paging_admits_only_selected_metadata_and_preserves_the_original_order() -> Result<()> {
    let (_directory, store) = fixture()?;
    note(&store, "first", 0, 2, "quote", &"n".repeat(4096), 10)?;
    note(&store, "second", 0, 3, "quote", &"n".repeat(4096), 10)?;
    note(&store, "third", 4, 5, "quote", "note", 30)?;
    store.reset_annotation_read_stats();
    let page = store.annotation_headers_page("book", 1, 1)?;
    assert_eq!(page.total, 3);
    assert_eq!(page.headers[0].key.id, "second");
    let stats = store.annotation_read_stats();
    assert_eq!(stats.id_copies, 1);
    assert_eq!(stats.quote_copies, 0);
    assert_eq!(stats.note_copies, 0);
    Ok(())
}

#[test]
fn header_rows_and_sql_cells_reject_before_any_id_copy() -> Result<()> {
    let (_directory, store) = fixture()?;
    note(&store, "a", 0, 1, "q", "n", 10)?;
    note(&store, "b", 2, 3, "q", "n", 20)?;
    for limits in [
        AnnotationLimits {
            rows: 1,
            ..AnnotationLimits::headers()
        },
        AnnotationLimits {
            cells: 7,
            ..AnnotationLimits::headers()
        },
    ] {
        store.reset_annotation_read_stats();
        assert!(
            store
                .annotation_headers_for_chapter_with_limits("book", 0, limits)
                .unwrap_err()
                .downcast_ref::<AnnotationResourceLimit>()
                .is_some()
        );
        assert_eq!(
            store.annotation_read_stats(),
            AnnotationReadStats::default()
        );
    }
    Ok(())
}

#[test]
fn header_working_set_exact_boundary_is_paid_before_ownership() -> Result<()> {
    let (_directory, store) = fixture()?;
    note(&store, "id", 0, 1, "q", "n", 10)?;
    let required = 2
        * (std::mem::size_of::<AnnotationMetadata>() + std::mem::size_of::<AnnotationHeader>())
        + 2;
    for maximum in [required, required - 1] {
        store.reset_annotation_read_stats();
        let result = store.annotation_headers_for_chapter_with_limits(
            "book",
            0,
            AnnotationLimits {
                bytes: maximum,
                ..AnnotationLimits::headers()
            },
        );
        if maximum == required {
            assert_eq!(result?[0].key.id, "id");
            assert_eq!(store.annotation_read_stats().id_bytes, 2);
        } else {
            assert!(
                result
                    .unwrap_err()
                    .downcast_ref::<AnnotationResourceLimit>()
                    .is_some()
            );
            assert_eq!(store.annotation_read_stats().id_copies, 0);
        }
        assert_eq!(store.annotation_read_stats().note_copies, 0);
    }
    Ok(())
}

#[test]
fn oversized_ids_are_unavailable_before_copy_but_do_not_block_geometry() -> Result<()> {
    let (_directory, store) = fixture()?;
    note(&store, &"i".repeat(4097), 0, 1, "q", "n", 10)?;
    store.reset_annotation_read_stats();
    assert!(
        store
            .annotation_headers_for_chapter("book", 0)
            .unwrap_err()
            .downcast_ref::<AnnotationResourceLimit>()
            .is_some()
    );
    assert_eq!(store.annotation_read_stats().id_copies, 0);
    let geometry = store.annotation_geometry_for_range("book", 0, 0, 1)?;
    assert_eq!(geometry.len(), 1);
    assert_eq!(store.annotation_read_stats().quote_copies, 0);
    assert_eq!(store.annotation_read_stats().note_copies, 0);
    Ok(())
}

#[test]
fn selected_record_rows_cells_fields_and_aggregate_reject_before_payload_copy() -> Result<()> {
    let (_directory, store) = fixture()?;
    note(&store, "id", 0, 1, "quote", "private note", 10)?;
    let keys = [key(&store, "id")?];
    let required = annotation_record_fixed(1)?
        + 2 * "id".len()
        + "book".len()
        + "quote".len()
        + "private note".len();
    for limits in [
        AnnotationLimits {
            rows: 0,
            ..AnnotationLimits::records()
        },
        AnnotationLimits {
            cells: 13,
            ..AnnotationLimits::records()
        },
        AnnotationLimits {
            quote_bytes: 4,
            ..AnnotationLimits::records()
        },
        AnnotationLimits {
            note_bytes: 11,
            ..AnnotationLimits::records()
        },
        AnnotationLimits {
            bytes: required - 1,
            ..AnnotationLimits::records()
        },
    ] {
        store.reset_annotation_read_stats();
        assert!(
            store
                .annotation_records_with_limits("book", &keys, limits)
                .unwrap_err()
                .downcast_ref::<AnnotationResourceLimit>()
                .is_some()
        );
        assert_eq!(
            store.annotation_read_stats(),
            AnnotationReadStats::default()
        );
    }
    let records = store.annotation_records_with_limits(
        "book",
        &keys,
        AnnotationLimits {
            bytes: required,
            ..AnnotationLimits::records()
        },
    )?;
    assert_eq!(records[0].note, "private note");
    Ok(())
}

#[test]
fn selected_records_do_not_read_unselected_oversized_quote_or_note() -> Result<()> {
    let (_directory, store) = fixture()?;
    note(&store, "selected", 0, 1, "q", "n", 10)?;
    note(
        &store,
        "unselected",
        2,
        3,
        &"q".repeat(16 * 1024 + 1),
        &"n".repeat(64 * 1024 + 1),
        20,
    )?;
    let selected = key(&store, "selected")?;
    store.reset_annotation_read_stats();
    let records = store.annotation_records("book", &[selected])?;
    assert_eq!(records[0].id, "selected");
    assert_eq!(store.annotation_read_stats().quote_bytes, 1);
    assert_eq!(store.annotation_read_stats().note_bytes, 1);
    assert_eq!(store.annotation_count("book")?, 2);
    Ok(())
}

#[test]
fn rowid_reuse_and_wrong_book_fail_identity_before_payload_projection() -> Result<()> {
    let (_directory, store) = fixture()?;
    note(&store, "old", 0, 1, "q", "n", 10)?;
    let old = key(&store, "old")?;
    store.connection.execute(
        "UPDATE annotations SET id='new' WHERE rowid=?1",
        [old.rowid],
    )?;
    store.reset_annotation_read_stats();
    assert!(
        store
            .annotation_records("book", &[old])
            .unwrap_err()
            .downcast_ref::<AnnotationUnavailable>()
            .is_some()
    );
    assert_eq!(
        store.annotation_read_stats(),
        AnnotationReadStats::default()
    );
    let key = key(&store, "new")?;
    store.reset_annotation_read_stats();
    assert!(
        store
            .annotation_records("other-book", &[key])
            .unwrap_err()
            .downcast_ref::<AnnotationUnavailable>()
            .is_some()
    );
    assert_eq!(store.annotation_read_stats().note_copies, 0);
    Ok(())
}

#[test]
fn blob_and_invalid_utf8_selected_fields_are_not_owned_or_guessed() -> Result<()> {
    let (_directory, store) = fixture()?;
    note(&store, "id", 0, 1, "q", "n", 10)?;
    let keys = [key(&store, "id")?];
    for sql in [
        "UPDATE annotations SET note=x'80' WHERE id='id'",
        "UPDATE annotations SET note=CAST(x'80' AS TEXT) WHERE id='id'",
    ] {
        store.connection.execute(sql, [])?;
        store.reset_annotation_read_stats();
        assert!(
            store
                .annotation_records("book", &keys)
                .unwrap_err()
                .downcast_ref::<AnnotationUnavailable>()
                .is_some()
        );
        assert_eq!(store.annotation_read_stats().quote_copies, 0);
        assert_eq!(store.annotation_read_stats().note_copies, 0);
    }
    Ok(())
}

#[test]
fn record_preflight_and_payload_read_share_the_same_sql_snapshot() -> Result<()> {
    let (directory, store) = fixture()?;
    note(&store, "id", 0, 1, "old quote", "old note", 10)?;
    let keys = [key(&store, "id")?];
    let other = Connection::open(directory.path().join("metadata.sqlite3"))?;
    let _hook = HookGuard::start(move || {
        other.execute(
            "UPDATE annotations SET quote='new quote',note='new note',created_at=20 WHERE id='id'",
            [],
        )?;
        Ok(())
    });
    let records = store.annotation_records("book", &keys)?;
    assert_eq!(records[0].quote, "old quote");
    assert_eq!(records[0].note, "old note");
    assert_eq!(records[0].created_at, 10);
    assert_eq!(
        store.annotation_by_id("book", "id")?.unwrap().note,
        "new note"
    );
    Ok(())
}

#[test]
fn header_preflight_and_admitted_id_read_share_the_same_snapshot() -> Result<()> {
    let (directory, store) = fixture()?;
    note(&store, "old", 0, 1, "q", "n", 10)?;
    let other = Connection::open(directory.path().join("metadata.sqlite3"))?;
    let _hook = HookGuard::start(move || {
        other.execute("UPDATE annotations SET id='new' WHERE id='old'", [])?;
        Ok(())
    });
    let headers = store.annotation_headers_for_chapter("book", 0)?;
    assert_eq!(headers[0].key.id, "old");
    assert!(
        store
            .annotation_records("book", &[headers[0].key.clone()])
            .unwrap_err()
            .downcast_ref::<AnnotationUnavailable>()
            .is_some()
    );
    Ok(())
}

#[test]
fn oversized_readonly_note_keeps_metadata_and_independent_valid_quote() -> Result<()> {
    let (_directory, store) = fixture()?;
    note(&store, "id", 0, 1, "quote", &"n".repeat(64 * 1024 + 1), 10)?;
    let key = key(&store, "id")?;
    store.reset_annotation_read_stats();
    let metadata = store.annotation_text_metadata("book", &key)?.unwrap();
    assert_eq!(metadata.header.key, key);
    assert_eq!(
        metadata.quote,
        AnnotationFieldState::Available {
            utf8_bytes_upper: 5
        }
    );
    assert_eq!(
        metadata.note,
        AnnotationFieldState::TooLarge {
            utf8_bytes_upper: 64 * 1024 + 1
        }
    );
    assert_eq!(
        store.annotation_quote_by_key("book", &key, 16 * 1024)?,
        Some("quote".into())
    );
    assert!(
        store
            .annotation_note_by_key("book", &key, 64 * 1024)
            .unwrap_err()
            .downcast_ref::<AnnotationResourceLimit>()
            .is_some()
    );
    assert_eq!(store.annotation_read_stats().note_copies, 0);
    Ok(())
}

#[test]
fn same_range_highlight_updates_keep_the_old_large_note_in_sql() -> Result<()> {
    let (_directory, store) = fixture()?;
    let original = "n".repeat(64 * 1024 + 1);
    note(&store, "id", 0, 1, "old quote", &original, 10)?;
    let header = store.annotation_header_by_id("book", "id")?.unwrap();
    store.reset_annotation_read_stats();
    assert!(store.update_annotation_quote_preserving_note("book", &header, "new quote", 20)?);
    assert_eq!(store.annotation_read_stats().note_copies, 0);
    let (quote, text, timestamp): (String, String, i64) = store.connection.query_row(
        "SELECT quote,note,created_at FROM annotations WHERE id='id'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(quote, "new quote");
    assert_eq!(text, original);
    assert_eq!(timestamp, 20);
    assert!(!store.update_annotation_quote_preserving_note("book", &header, "stale quote", 30)?);
    Ok(())
}

#[test]
fn update_work_rejection_keeps_original_note_quote_and_timestamp() -> Result<()> {
    let (_directory, store) = fixture()?;
    note(&store, "id", 0, 1, "old quote", "old note", 10)?;
    let header = store.annotation_header_by_id("book", "id")?.unwrap();
    let _guard = HookGuard;
    UPDATE_LIMIT.with(|limit| limit.set(Some(1)));
    assert!(
        store
            .update_annotation_quote_preserving_note("book", &header, "new quote", 20)
            .unwrap_err()
            .downcast_ref::<AnnotationResourceLimit>()
            .is_some()
    );
    let note = store.annotation_by_id("book", "id")?.unwrap();
    assert_eq!(
        (note.quote.as_str(), note.note.as_str(), note.created_at),
        ("old quote", "old note", 10)
    );
    Ok(())
}

#[test]
fn live_book_timestamp_counts_stream_without_annotation_text_or_time_filtering() -> Result<()> {
    let (_directory, store) = fixture()?;
    for (id, timestamp) in [
        ("negative", -1),
        ("future", i64::MAX),
        ("ancient", i64::MIN),
        ("same-a", 10),
        ("same-b", 10),
    ] {
        note(&store, id, 0, 1, "q", "n", timestamp)?;
    }
    store
        .connection
        .pragma_update(None, "foreign_keys", false)?;
    store.connection.execute(
        "INSERT INTO annotations VALUES ('orphan','missing',0,0,1,'q','n',20)",
        [],
    )?;
    store.connection.pragma_update(None, "foreign_keys", true)?;
    store.reset_annotation_read_stats();
    let mut buckets = Vec::new();
    store.for_each_annotation_timestamp_count(|time, count| {
        buckets.push((time, count));
        Ok(())
    })?;
    assert_eq!(buckets, [(i64::MIN, 1), (-1, 1), (10, 2), (i64::MAX, 1)]);
    assert_eq!(
        store.annotation_read_stats(),
        AnnotationReadStats::default()
    );
    Ok(())
}

#[test]
fn malformed_live_timestamp_types_fail_without_reading_note_text() -> Result<()> {
    let (_directory, store) = fixture()?;
    note(&store, "id", 0, 1, "q", "n", 10)?;
    store
        .connection
        .execute("UPDATE annotations SET created_at=x'80' WHERE id='id'", [])?;
    store.reset_annotation_read_stats();
    assert!(
        store
            .for_each_annotation_timestamp_count(|_, _| panic!("No timestamp may be published"))
            .unwrap_err()
            .downcast_ref::<AnnotationUnavailable>()
            .is_some()
    );
    assert_eq!(
        store.annotation_read_stats(),
        AnnotationReadStats::default()
    );
    Ok(())
}

#[test]
fn selected_geometry_resolves_only_its_admitted_id_and_checks_the_fingerprint() -> Result<()> {
    let (_directory, store) = fixture()?;
    note(&store, "id", 0, 1, "q", "n", 10)?;
    let geometry = store.annotation_geometry_for_range("book", 0, 0, 1)?[0];
    store.reset_annotation_read_stats();
    assert_eq!(
        store
            .annotation_header_for_geometry("book", &geometry)?
            .unwrap()
            .key
            .id,
        "id"
    );
    assert_eq!(store.annotation_read_stats().id_copies, 1);
    assert_eq!(store.annotation_read_stats().note_copies, 0);
    store
        .connection
        .execute("UPDATE annotations SET created_at=20 WHERE id='id'", [])?;
    assert!(
        store
            .annotation_header_for_geometry("book", &geometry)
            .unwrap_err()
            .downcast_ref::<AnnotationUnavailable>()
            .is_some()
    );
    Ok(())
}

#[test]
fn utf16_annotation_fields_use_the_conservative_utf8_bound_before_ownership() -> Result<()> {
    for encoding in ["UTF-16le", "UTF-16be"] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("metadata.sqlite3");
        {
            let mut connection = Connection::open(&path)?;
            connection.pragma_update(None, "encoding", encoding)?;
            migrate(&mut connection)?;
        }
        let store = Store::open(&path)?;
        let mut book = Book::new("book", "Fixture");
        book.source = "local".into();
        store.upsert_book(&book)?;
        store.upsert_chapter(&Chapter {
            id: "uid".into(),
            book_id: "book".into(),
            body: Some("text".into()),
            ..Chapter::default()
        })?;
        note(&store, "id", 0, 1, "\u{6d77}", "\u{4e2d}", 10)?;
        let key = key(&store, "id")?;
        store.reset_annotation_read_stats();
        assert_eq!(
            store.annotation_quote_by_key("book", &key, 3)?,
            Some("\u{6d77}".into())
        );
        assert!(
            store
                .annotation_note_by_key("book", &key, 2)
                .unwrap_err()
                .downcast_ref::<AnnotationResourceLimit>()
                .is_some()
        );
        assert_eq!(store.annotation_read_stats().note_copies, 0);
        assert_eq!(
            store.annotation_text_metadata("book", &key)?.unwrap().note,
            AnnotationFieldState::Available {
                utf8_bytes_upper: 3
            }
        );
    }
    Ok(())
}

#[test]
fn trim_sensitive_summary_uses_only_admitted_borrowed_fields() -> Result<()> {
    let (_directory, store) = fixture()?;
    note(&store, "a", 0, 1, "\u{2003}", "note", 10)?;
    note(&store, "b", 2, 3, "quote", " \t\n", 20)?;
    note(&store, "c", 4, 5, "\0", "", 30)?;
    store.reset_annotation_read_stats();
    assert_eq!(store.annotation_summary_counts("book")?, (2, 1));
    assert_eq!(
        store.annotation_read_stats(),
        AnnotationReadStats::default()
    );
    store.connection.execute(
        "UPDATE annotations SET note=?1 WHERE id='a'",
        [&"n".repeat(64 * 1024 + 1)],
    )?;
    assert!(
        store
            .annotation_summary_counts("book")
            .unwrap_err()
            .downcast_ref::<AnnotationResourceLimit>()
            .is_some()
    );
    assert_eq!(
        store.annotation_read_stats(),
        AnnotationReadStats::default()
    );
    assert_eq!(store.annotation_count("book")?, 3);
    Ok(())
}

#[test]
fn exact_range_header_selects_the_original_first_match_without_note_payloads() -> Result<()> {
    let (_directory, store) = fixture()?;
    note(&store, "later", 0, 2, "q", "n", 20)?;
    note(&store, "earlier", 0, 2, "q", &"n".repeat(64 * 1024 + 1), 10)?;
    note(&store, "overlap", 1, 3, "q", "n", 5)?;
    store.reset_annotation_read_stats();
    let header = store
        .annotation_header_for_exact_range("book", 0, 0, 2)?
        .unwrap();
    assert_eq!(header.key.id, "earlier");
    assert_eq!(store.annotation_read_stats().id_copies, 1);
    assert_eq!(store.annotation_read_stats().quote_copies, 0);
    assert_eq!(store.annotation_read_stats().note_copies, 0);
    Ok(())
}
