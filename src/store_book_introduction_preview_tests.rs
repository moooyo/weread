//! Bounded introduction prefixes use genuine SQLite storage and strict encodings.

use super::*;

fn fixture(encoding: &str, introduction: &str) -> Result<(tempfile::TempDir, Store)> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("preview.sqlite");
    {
        let mut connection = Connection::open(&path)?;
        connection.pragma_update(None, "encoding", encoding)?;
        initialize_schema(&mut connection)?;
    }
    let store = Store::open(&path)?;
    store.upsert_book(&Book {
        intro: introduction.into(),
        source: "local".into(),
        ..Book::new("book", "Preview fixture")
    })?;
    store.reset_book_read_stats();
    store.reset_book_introduction_read_stats();
    Ok((directory, store))
}

fn assert_no_full_introduction_or_metadata(store: &Store) {
    assert_eq!(store.book_read_stats().intro_copies, 0);
    assert_eq!(store.book_read_stats().intro_bytes, 0);
    assert_eq!(store.book_read_stats().metadata_rows, 0);
    assert_eq!(store.book_read_stats().metadata_string_bytes, 0);
    assert_eq!(
        store.book_introduction_read_stats(),
        BookIntroductionReadStats::default()
    );
}

struct SqliteTextOutputLimit<'a> {
    connection: &'a Connection,
    previous: i32,
}

impl<'a> SqliteTextOutputLimit<'a> {
    fn new(connection: &'a Connection, maximum: i32) -> Result<Self> {
        let previous =
            connection.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH, maximum)?;
        Ok(Self {
            connection,
            previous,
        })
    }
}

impl Drop for SqliteTextOutputLimit<'_> {
    fn drop(&mut self) {
        let _ = self
            .connection
            .set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH, self.previous);
    }
}

fn overwrite_encoded_prefix(store: &Store, offset: i32, bytes: &[u8]) -> Result<()> {
    let rowid: i64 =
        store
            .connection
            .query_row("SELECT rowid FROM books WHERE id='book'", [], |row| {
                row.get(0)
            })?;
    // Damage only this temporary TEXT cell's bytes, preserving its type.
    let mut blob = store
        .connection
        .blob_open("main", "books", "intro", rowid, false)?;
    std::io::Seek::seek(&mut blob, std::io::SeekFrom::Start(u64::try_from(offset)?))?;
    std::io::Write::write_all(&mut blob, bytes)?;
    blob.close()?;
    Ok(())
}

#[test]
fn introduction_preview_distinguishes_short_empty_whitespace_and_missing_books() -> Result<()> {
    for encoding in ["UTF-8", "UTF-16le", "UTF-16be"] {
        let (_directory, store) = fixture(encoding, "  A short introduction.\n")?;
        assert_eq!(
            store.book_introduction_preview("book")?,
            Some(BookIntroductionPreview {
                text: "A short introduction.".into(),
                has_more: false,
            })
        );
        for text in ["", " \t\r\n\u{3000}"] {
            store
                .connection
                .execute("UPDATE books SET intro=?1 WHERE id='book'", [text])?;
            assert_eq!(
                store.book_introduction_preview("book")?,
                Some(BookIntroductionPreview::default())
            );
        }
        let before = store.book_introduction_preview_reads.get();
        assert_eq!(store.book_introduction_preview("missing")?, None);
        assert_eq!(store.book_introduction_preview_reads.get(), before);
        assert_no_full_introduction_or_metadata(&store);
    }
    Ok(())
}

#[test]
fn introduction_preview_counts_unicode_scalars_and_caps_lookahead_in_all_encodings() -> Result<()> {
    for encoding in ["UTF-8", "UTF-16le", "UTF-16be"] {
        let exact = "\u{4e2d}\u{1f600}".repeat(80);
        assert_eq!(exact.chars().count(), BOOK_INTRODUCTION_PREVIEW_SCALARS);
        let (_directory, store) = fixture(encoding, &exact)?;
        let preview = store.book_introduction_preview("book")?.unwrap();
        assert_eq!(preview.text, exact);
        assert!(!preview.has_more);
        store.connection.execute(
            "UPDATE books SET intro=?1 WHERE id='book'",
            [format!("{exact}Z")],
        )?;
        let preview = store.book_introduction_preview("book")?.unwrap();
        assert_eq!(preview.text, exact);
        assert!(preview.has_more);

        let astral = "\u{1f600}".repeat(BOOK_INTRODUCTION_PREVIEW_SCALARS + 1);
        store
            .connection
            .execute("UPDATE books SET intro=?1 WHERE id='book'", [&astral])?;
        store
            .book_introduction_preview_reads
            .set(BookIntroductionPreviewReadStats::default());
        let preview = store.book_introduction_preview("book")?.unwrap();
        assert_eq!(preview.text, "\u{1f600}".repeat(160));
        assert!(preview.has_more);
        let stats = store.book_introduction_preview_reads.get();
        assert_eq!(stats.encoded_bytes, BOOK_INTRODUCTION_PREVIEW_ENCODED_BYTES);
        assert_eq!(stats.decoded_scalars, 161);
        assert_eq!(stats.result_bytes, 640);

        store.connection.execute(
            "UPDATE books SET intro=?1 WHERE id='book'",
            [" ".repeat(160) + "Visible only outside the preview"],
        )?;
        let preview = store.book_introduction_preview("book")?.unwrap();
        assert!(preview.text.is_empty());
        assert!(
            preview.has_more,
            "Overflow is determined before trimming the prefix"
        );
        assert_no_full_introduction_or_metadata(&store);
    }
    Ok(())
}

#[test]
fn introduction_preview_reads_huge_text_below_sqlite_text_output_limit() -> Result<()> {
    for encoding in ["UTF-8", "UTF-16le", "UTF-16be"] {
        let introduction = format!(
            "{}{}",
            "Bounded introduction. ".repeat(12),
            "\u{4e2d}\u{1f600}".repeat(600_000)
        );
        let expected = introduction
            .chars()
            .take(160)
            .collect::<String>()
            .trim()
            .to_owned();
        let (_directory, store) = fixture(encoding, &introduction)?;
        let _limit = SqliteTextOutputLimit::new(&store.connection, 2048)?;
        let full =
            store
                .connection
                .query_row("SELECT intro FROM books WHERE id='book'", [], |row| {
                    row.get::<_, String>(0)
                });
        assert_eq!(
            full.unwrap_err().sqlite_error_code(),
            Some(rusqlite::ErrorCode::TooBig)
        );
        let preview = store.book_introduction_preview("book")?.unwrap();
        assert_eq!(preview.text, expected);
        assert!(preview.has_more);
        let stats = store.book_introduction_preview_reads.get();
        assert_eq!(stats.reads, 1);
        assert!(stats.encoded_bytes <= 644);
        assert_eq!(stats.decoded_scalars, 161);
        assert!(stats.result_bytes <= 640);
        assert_no_full_introduction_or_metadata(&store);
    }
    Ok(())
}

#[test]
fn introduction_preview_does_not_read_target_metadata_or_other_book_payloads() -> Result<()> {
    let (_directory, store) = fixture("UTF-8", "Target preview")?;
    store.upsert_book(&Book::new("other", "Unrelated book"))?;
    store.connection.execute_batch(
        "UPDATE books SET title=x'80',author=x'80',cover_url=x'80' WHERE id='book';
         UPDATE books SET intro=CAST(x'80' AS TEXT),title=zeroblob(1048576) WHERE id='other';
         INSERT INTO settings(key,value) VALUES ('book.metadata:book',CAST(x'80' AS TEXT));
         INSERT INTO settings(key,value) VALUES ('book.metadata:other',zeroblob(1048576));",
    )?;
    with_book_introduction_test_limits(
        BookIntroductionLimits {
            bytes: book_introduction_preview_working_set()?,
            metadata_bytes: 0,
            metadata_nodes: 0,
            field_bytes: 0,
        },
        || -> Result<()> {
            let preview = store.book_introduction_preview("book")?.unwrap();
            assert_eq!(preview.text, "Target preview");
            assert!(!preview.has_more);
            assert_no_full_introduction_or_metadata(&store);
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn introduction_preview_rejects_nontext_and_malformed_utf8_without_repairing_data() -> Result<()> {
    let (_directory, store) = fixture("UTF-8", "Keep original")?;
    store
        .connection
        .execute("UPDATE books SET intro=x'80' WHERE id='book'", [])?;
    assert!(
        store
            .book_introduction_preview("book")
            .unwrap_err()
            .downcast_ref::<BookMetadataUnavailable>()
            .is_some()
    );
    assert_eq!(store.book_introduction_preview_reads.get().reads, 0);
    for raw in [
        vec![0x80],
        vec![0xc0, 0xaf],
        vec![0xed, 0xa0, 0x80],
        vec![0xe2, 0x82],
        vec![0xf4, 0x90, 0x80, 0x80],
    ] {
        store.connection.execute(
            "UPDATE books SET intro=CAST(?1 AS TEXT) WHERE id='book'",
            params![&raw],
        )?;
        assert!(
            store
                .book_introduction_preview("book")
                .unwrap_err()
                .downcast_ref::<BookMetadataUnavailable>()
                .is_some()
        );
        let retained: Vec<u8> = store.connection.query_row(
            "SELECT CAST(intro AS BLOB) FROM books WHERE id='book'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(retained, raw);
    }
    assert_no_full_introduction_or_metadata(&store);
    Ok(())
}

#[test]
fn introduction_preview_rejects_utf16_surrogate_damage_and_truncated_lookahead() -> Result<()> {
    for encoding in ["UTF-16le", "UTF-16be"] {
        let (_directory, store) = fixture(encoding, &"x".repeat(161))?;
        let raw = if encoding == "UTF-16le" {
            [0x00, 0xdc]
        } else {
            [0xdc, 0x00]
        };
        overwrite_encoded_prefix(&store, 0, &raw)?;
        assert!(
            store
                .book_introduction_preview("book")
                .unwrap_err()
                .downcast_ref::<BookMetadataUnavailable>()
                .is_some()
        );
        store.connection.execute(
            "UPDATE books SET intro=?1 WHERE id='book'",
            ["x".repeat(161)],
        )?;
        let high = if encoding == "UTF-16le" {
            [0x00, 0xd8]
        } else {
            [0xd8, 0x00]
        };
        overwrite_encoded_prefix(&store, 320, &high)?;
        assert!(
            store
                .book_introduction_preview("book")
                .unwrap_err()
                .downcast_ref::<BookMetadataUnavailable>()
                .is_some()
        );
        assert_no_full_introduction_or_metadata(&store);
    }
    Ok(())
}

#[test]
fn introduction_preview_does_not_validate_or_copy_the_unused_tail() -> Result<()> {
    let (_directory, store) = fixture("UTF-8", "Temporary")?;
    let mut raw = "\u{1f600}".repeat(161).into_bytes();
    raw.extend_from_slice(&[0x80, 0xff, 0xc0]);
    store.connection.execute(
        "UPDATE books SET intro=CAST(?1 AS TEXT) WHERE id='book'",
        params![&raw],
    )?;
    let preview = store.book_introduction_preview("book")?.unwrap();
    assert_eq!(preview.text.chars().count(), 160);
    assert!(preview.has_more);
    assert_eq!(
        store.book_introduction_preview_reads.get().encoded_bytes,
        644
    );
    assert_no_full_introduction_or_metadata(&store);
    Ok(())
}

#[test]
fn introduction_preview_prices_fixed_buffers_before_payload_read() -> Result<()> {
    let (_directory, store) = fixture("UTF-8", "Small preview")?;
    let required = book_introduction_preview_working_set()?;
    with_book_introduction_test_limits(
        BookIntroductionLimits {
            bytes: required - 1,
            ..Default::default()
        },
        || -> Result<()> {
            assert!(
                store
                    .book_introduction_preview("book")
                    .unwrap_err()
                    .downcast_ref::<BookMetadataLimit>()
                    .is_some()
            );
            assert_eq!(store.book_introduction_preview_reads.get().reads, 0);
            assert_eq!(store.book_introduction_preview("missing")?, None);
            Ok(())
        },
    )?;
    assert_eq!(
        store.book_introduction_preview("book")?.unwrap().text,
        "Small preview"
    );
    assert_no_full_introduction_or_metadata(&store);
    Ok(())
}
