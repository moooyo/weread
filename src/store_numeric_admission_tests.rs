//! Non-STRICT numeric columns are admitted before ordered/materializing reads.

use super::*;

fn fixture() -> Result<Store> {
    let s = Store::open(":memory:")?;
    s.upsert_book(&Book {
        source: "local".into(),
        intro: "Keep intro".into(),
        ..Book::new("book", "Book")
    })?;
    s.upsert_chapter(&Chapter {
        id: "uid".into(),
        book_id: "book".into(),
        body: Some("Text".into()),
        ..Default::default()
    })?;
    s.set_setting("numeric.account", &"Keep account")?;
    s.reset_book_read_stats();
    Ok(s)
}

#[test]
fn book_numeric_projection_preserves_real_integer_and_nonzero_boolean_semantics() -> Result<()> {
    let s = fixture()?;
    s.connection.execute("UPDATE books SET progress=0,total_chapters=1,cached_chapters=1,updated=-2,finished=2,on_shelf=-1 WHERE id='book'",[])?;
    let b = s.book_metadata_by_id("book")?.unwrap();
    assert_eq!(b.progress, 0.0);
    assert!(b.updated && b.finished && b.on_shelf);
    s.connection
        .execute("UPDATE books SET progress=0.5 WHERE id='book'", [])?;
    assert_eq!(s.book_metadata_by_id("book")?.unwrap().progress, 0.5);
    assert!(s.book_statistics_metadata()?[0].finished);
    assert_eq!(s.book_read_stats().intro_copies, 0);
    Ok(())
}

#[test]
fn malformed_book_scalar_type_rejects_before_owned_metadata_and_keeps_row() -> Result<()> {
    let s = fixture()?;
    s.connection.execute(
        "UPDATE books SET total_chapters=?1 WHERE id='book'",
        [format!("notnumeric{}", "x".repeat(4096))],
    )?;
    assert!(
        s.book_metadata()
            .unwrap_err()
            .downcast_ref::<BookMetadataUnavailable>()
            .is_some()
    );
    assert_eq!(s.book_read_stats().metadata_rows, 0);
    assert_eq!(s.book_read_stats().intro_copies, 0);
    s.connection.execute(
        "UPDATE books SET total_chapters=1,finished=x'80' WHERE id='book'",
        [],
    )?;
    assert!(
        s.book_statistics_metadata()
            .unwrap_err()
            .downcast_ref::<BookMetadataUnavailable>()
            .is_some()
    );
    assert_eq!(s.book_read_stats().metadata_rows, 0);
    assert_eq!(
        s.connection.query_row(
            "SELECT typeof(finished) FROM books WHERE id='book'",
            [],
            |r| r.get::<_, String>(0)
        )?,
        "blob"
    );
    assert_eq!(
        s.setting::<String>("numeric.account")?.as_deref(),
        Some("Keep account")
    );
    Ok(())
}

#[test]
fn chapter_index_type_gate_precedes_batch_order_and_preserves_body() -> Result<()> {
    let s = fixture()?;
    s.connection.execute(
        "UPDATE chapters SET chapter_index=?1 WHERE book_id='book'",
        [format!("notnumeric{}", "x".repeat(4096))],
    )?;
    assert!(
        s.chapter_headers("book")
            .unwrap_err()
            .downcast_ref::<CachedResourceLimit>()
            .is_some()
    );
    assert_eq!(s.chapter_body_read_count(), 0);
    assert_eq!(s.materialized_chapter_bytes(), 0);
    assert_eq!(
        s.connection.query_row(
            "SELECT typeof(chapter_index) FROM chapters WHERE book_id='book'",
            [],
            |r| r.get::<_, String>(0)
        )?,
        "text"
    );
    assert_eq!(
        s.connection
            .query_row("SELECT body FROM chapters WHERE book_id='book'", [], |r| {
                r.get::<_, String>(0)
            })?,
        "Text"
    );
    assert_eq!(
        s.setting::<String>("numeric.account")?.as_deref(),
        Some("Keep account")
    );
    Ok(())
}

#[test]
fn annotation_offset_and_time_types_refuse_before_id_payload_or_timestamp_stream() -> Result<()> {
    let s = fixture()?;
    s.save_annotation(&Annotation {
        id: "note".into(),
        book_id: "book".into(),
        start: 0,
        end: 4,
        quote: "Text".into(),
        note: "Keep note".into(),
        ..Default::default()
    })?;
    s.reset_annotation_read_stats();
    // BLOB end_offset satisfies the non-STRICT comparison CHECK, but cannot
    // be interpreted as a Rust offset or used as a guessed range.
    s.connection.execute(
        "UPDATE annotations SET end_offset=x'80' WHERE id='note'",
        [],
    )?;
    assert!(
        s.annotation_count_for_range("book", 0, 0, 4)
            .unwrap_err()
            .downcast_ref::<AnnotationUnavailable>()
            .is_some()
    );
    assert!(
        s.annotation_geometry_for_range("book", 0, 0, 4)
            .unwrap_err()
            .downcast_ref::<AnnotationUnavailable>()
            .is_some()
    );
    assert_eq!(s.annotation_read_stats(), AnnotationReadStats::default());
    s.connection.execute(
        "UPDATE annotations SET end_offset=4,created_at=x'80' WHERE id='note'",
        [],
    )?;
    let called = std::cell::Cell::new(false);
    assert!(
        s.for_each_annotation_timestamp_count(|_, _| {
            called.set(true);
            Ok(())
        })
        .unwrap_err()
        .downcast_ref::<AnnotationUnavailable>()
        .is_some()
    );
    assert!(!called.get());
    assert_eq!(s.annotation_read_stats(), AnnotationReadStats::default());
    assert_eq!(
        s.connection
            .query_row("SELECT note FROM annotations WHERE id='note'", [], |r| {
                r.get::<_, String>(0)
            })?,
        "Keep note"
    );
    Ok(())
}

#[test]
fn catalog_position_numeric_gate_retains_device_and_rejects_legacy_blob() -> Result<()> {
    let s = fixture()?;
    let value = ReadingPosition {
        book_id: "book".into(),
        chapter_index: 0,
        char_offset: 3,
        updated_at: -7,
        device: "  device\0text  ".into(),
    };
    s.save_position(&value)?;
    assert_eq!(s.position("book")?, Some(value));
    s.connection.execute(
        "UPDATE positions SET char_offset=x'80' WHERE book_id='book'",
        [],
    )?;
    assert!(
        s.position("book")
            .unwrap_err()
            .downcast_ref::<CachedResourceLimit>()
            .is_some()
    );
    assert_eq!(
        s.connection.query_row(
            "SELECT device FROM positions WHERE book_id='book'",
            [],
            |r| r.get::<_, String>(0)
        )?,
        "  device\0text  "
    );
    assert_eq!(s.chapter_body_read_count(), 0);
    assert_eq!(
        s.setting::<String>("numeric.account")?.as_deref(),
        Some("Keep account")
    );
    Ok(())
}
