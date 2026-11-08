//! Visible bookmark selection does not own hidden labels.

use super::*;
use std::cell::Cell;

thread_local! {
    static STATS: Cell<BookmarkReadStats> = const { Cell::new(BookmarkReadStats {
        id_copies: 0, id_bytes: 0, label_copies: 0, label_bytes: 0,
    }) };
}
pub(super) fn read_stats() -> BookmarkReadStats {
    STATS.with(Cell::get)
}
pub(super) fn reset() {
    STATS.with(|slot| slot.set(BookmarkReadStats::default()));
}
pub(super) fn observe_id(bytes: usize) {
    STATS.with(|slot| {
        let mut s = slot.get();
        s.id_copies += 1;
        s.id_bytes += bytes;
        slot.set(s);
    });
}
pub(super) fn observe_label(bytes: usize) {
    STATS.with(|slot| {
        let mut s = slot.get();
        s.label_copies += 1;
        s.label_bytes += bytes;
        slot.set(s);
    });
}

fn fixture() -> Result<Store> {
    let store = Store::open(":memory:")?;
    store.upsert_book(&Book {
        source: "local".into(),
        intro: "Keep intro".into(),
        ..Book::new("book", "Book")
    })?;
    for i in 0..3 {
        store.save_bookmark(&Bookmark {
            id: format!("id-{i}"),
            book_id: "book".into(),
            chapter_index: 0,
            char_offset: i,
            label: format!("\"Label {i}\" {}", "x".repeat(2048)),
            created_at: i as i64,
        })?;
    }
    store.set_setting("bookmark.fixture.account", &"Keep account")?;
    reset();
    Ok(store)
}

#[test]
fn bookmark_count_and_range_geometry_do_not_copy_ids_or_labels() -> Result<()> {
    let store = fixture()?;
    assert_eq!(store.bookmark_count("book")?, 3);
    let geometry = store.bookmark_geometry_for_range("book", 0, 1, 2)?;
    assert_eq!(geometry.len(), 1);
    assert_eq!(geometry[0].char_offset, 1);
    assert_eq!(read_stats(), BookmarkReadStats::default());
    assert_eq!(store.book_read_stats().intro_copies, 0);
    Ok(())
}

#[test]
fn visible_bookmark_header_and_records_own_only_selected_canonical_fields() -> Result<()> {
    let store = fixture()?;
    let page = store.bookmark_geometry_page("book", 1, 1)?;
    assert_eq!(page.total, 3);
    let header = store
        .bookmark_header_for_geometry("book", &page.geometries[0])?
        .unwrap();
    assert_eq!(header.key.id, "id-1");
    assert_eq!(read_stats().label_copies, 0);
    let records = store.bookmark_records("book", &[header])?;
    assert_eq!(records.len(), 1);
    assert!(records[0].label.starts_with("\"Label 1\""));
    assert_eq!(read_stats().id_copies, 2);
    assert_eq!(read_stats().label_copies, 1);
    assert_eq!(read_stats().label_bytes, records[0].label.len());
    assert_eq!(store.book_read_stats().intro_copies, 0);
    Ok(())
}

#[test]
fn bookmark_label_quota_and_stale_geometry_keep_all_history() -> Result<()> {
    let store = fixture()?;
    let geometry = store.bookmark_geometry_page("book", 0, 1)?.geometries[0];
    let header = store
        .bookmark_header_for_geometry("book", &geometry)?
        .unwrap();
    with_bookmark_test_limits(
        BookmarkLimits {
            label_bytes: 1,
            ..Default::default()
        },
        || -> Result<()> {
            assert!(
                store
                    .bookmark_records("book", std::slice::from_ref(&header))
                    .unwrap_err()
                    .downcast_ref::<BookmarkResourceLimit>()
                    .is_some()
            );
            assert_eq!(read_stats().id_copies, 1);
            assert_eq!(read_stats().label_copies, 0);
            Ok(())
        },
    )?;
    store
        .connection
        .execute("UPDATE bookmarks SET created_at=99 WHERE id='id-0'", [])?;
    assert!(
        store
            .bookmark_header_for_geometry("book", &geometry)?
            .is_none()
    );
    assert!(
        store
            .bounded_bookmark_label("book", &header, 64 * 1024)?
            .is_none()
    );
    assert_eq!(store.bookmark_count("book")?, 3);
    assert_eq!(
        store
            .setting::<String>("bookmark.fixture.account")?
            .as_deref(),
        Some("Keep account")
    );
    assert!(store.bookmarks("book")?[0].label.starts_with("\"Label 0\""));
    Ok(())
}

#[test]
fn bookmark_sql_failure_is_not_a_capacity_or_empty_success() -> Result<()> {
    let store = fixture()?;
    store
        .connection
        .execute_batch("ALTER TABLE bookmarks RENAME TO held_bookmarks")?;
    let error = store.bookmark_count("book").unwrap_err();
    store
        .connection
        .execute_batch("ALTER TABLE held_bookmarks RENAME TO bookmarks")?;
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
    assert_eq!(store.bookmark_count("book")?, 3);
    Ok(())
}

#[test]
fn canonical_bookmark_lookup_uses_only_selected_geometry_and_rejects_bad_numeric_type() -> Result<()>
{
    let store = fixture()?;
    store.connection.execute(
        "UPDATE bookmarks SET id=?1,label=x'80' WHERE id='id-2'",
        ["u".repeat(4097)],
    )?;
    reset();
    let geometry = store.bookmark_geometry_by_id("book", "id-1")?.unwrap();
    assert_eq!(geometry.char_offset, 1);
    assert_eq!(geometry.created_at, 1);
    assert!(store.bookmark_geometry_by_id("book", "missing")?.is_none());
    assert_eq!(read_stats(), BookmarkReadStats::default());
    store
        .connection
        .execute("UPDATE bookmarks SET char_offset=x'80' WHERE id='id-1'", [])?;
    let error = store.bookmark_geometry_by_id("book", "id-1").unwrap_err();
    assert!(error.downcast_ref::<BookmarkUnavailable>().is_some());
    assert!(
        store
            .bookmark_geometry_page("book", 0, 1)
            .unwrap_err()
            .downcast_ref::<BookmarkUnavailable>()
            .is_some()
    );
    assert_eq!(read_stats(), BookmarkReadStats::default());
    assert_eq!(store.bookmark_count("book")?, 3);
    assert_eq!(
        store.connection.query_row(
            "SELECT typeof(char_offset) FROM bookmarks WHERE id='id-1'",
            [],
            |r| r.get::<_, String>(0)
        )?,
        "blob"
    );
    assert_eq!(
        store
            .setting::<String>("bookmark.fixture.account")?
            .as_deref(),
        Some("Keep account")
    );
    Ok(())
}

#[test]
fn utf16_bookmark_header_keeps_actual_identifier_and_quoted_nul_label() -> Result<()> {
    for encoding in ["UTF-16le", "UTF-16be"] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("db");
        {
            let mut connection = Connection::open(&path)?;
            connection.pragma_update(None, "encoding", encoding)?;
            migrate(&mut connection)?;
        }
        let store = Store::open(&path)?;
        store.upsert_book(&Book {
            source: "local".into(),
            ..Book::new("book", "Book")
        })?;
        let original = Bookmark {
            id: "i".repeat(4096),
            book_id: "book".into(),
            label: "\"\u{4e2d}\0label\"".into(),
            ..Default::default()
        };
        store.save_bookmark(&original)?;
        reset();
        let geometry = store
            .bookmark_geometry_by_id("book", &original.id)?
            .unwrap();
        assert_eq!(read_stats(), BookmarkReadStats::default());
        let header = store
            .bookmark_header_for_geometry("book", &geometry)?
            .unwrap();
        assert_eq!(header.key.id, original.id);
        assert_eq!(
            store
                .bounded_bookmark_label("book", &header, 64 * 1024)?
                .as_deref(),
            Some(original.label.as_str())
        );
        assert_eq!(read_stats().id_bytes, 4096);
        assert_eq!(read_stats().label_bytes, original.label.len());
        assert_eq!(store.bookmark_count("book")?, 1);
    }
    Ok(())
}

#[test]
fn bookmark_label_and_utf16_vm_share_exact_working_set_allowance() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("db");
    {
        let mut connection = Connection::open(&path)?;
        connection.pragma_update(None, "encoding", "UTF-16le")?;
        migrate(&mut connection)?;
    }
    let store = Store::open(&path)?;
    store.upsert_book(&Book {
        source: "local".into(),
        intro: "Keep intro".into(),
        ..Book::new("book", "Book")
    })?;
    let label = "\"Label\"\0tail";
    store.save_bookmark(&Bookmark {
        id: "id".into(),
        book_id: "book".into(),
        label: label.into(),
        ..Default::default()
    })?;
    let geometry = store.bookmark_geometry_by_id("book", "id")?.unwrap();
    let header = store
        .bookmark_header_for_geometry("book", &geometry)?
        .unwrap();
    let encoded = usize::try_from(store.connection.query_row(
        "SELECT octet_length(label) FROM bookmarks WHERE id='id'",
        [],
        |r| r.get::<_, i64>(0),
    )?)?;
    let upper = store.text_encoding.utf8_upper(encoded)?;
    let exact = upper
        + 3 * encoded
        + 2
        + "book".len()
        + header.key.id.len()
        + 2 * std::mem::size_of::<String>();
    reset();
    with_bookmark_test_limits(
        BookmarkLimits {
            metadata_bytes: exact - 1,
            ..Default::default()
        },
        || -> Result<()> {
            let error = store
                .bounded_bookmark_label("book", &header, 64 * 1024)
                .unwrap_err();
            assert_eq!(
                error
                    .downcast_ref::<BookmarkResourceLimit>()
                    .unwrap()
                    .resource,
                "label metadata"
            );
            assert_eq!(read_stats().label_copies, 0);
            Ok(())
        },
    )?;
    with_bookmark_test_limits(
        BookmarkLimits {
            metadata_bytes: exact,
            ..Default::default()
        },
        || -> Result<()> {
            assert_eq!(
                store
                    .bounded_bookmark_label("book", &header, 64 * 1024)?
                    .as_deref(),
                Some(label)
            );
            assert_eq!(read_stats().label_copies, 1);
            Ok(())
        },
    )?;
    assert_eq!(store.bookmarks("book")?[0].label, label);
    assert_eq!(store.book("book")?.unwrap().intro, "Keep intro");
    Ok(())
}
