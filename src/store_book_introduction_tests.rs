//! Introduction payload admission keeps inactive tabs and raw storage intact.

use super::*;
use std::cell::RefCell;

thread_local! {
    static HOOK: RefCell<Option<Box<dyn FnOnce() -> Result<()>>>> = const { RefCell::new(None) };
}

pub(super) fn after_preflight() -> Result<()> {
    let hook = HOOK.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook()?;
    }
    Ok(())
}

struct HookGuard;
impl Drop for HookGuard {
    fn drop(&mut self) {
        HOOK.with(|slot| slot.borrow_mut().take());
    }
}

fn fixture(encoding: &str) -> Result<(tempfile::TempDir, Store)> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("introduction.sqlite");
    {
        let mut connection = Connection::open(&path)?;
        connection.pragma_update(None, "encoding", encoding)?;
        initialize_schema(&mut connection)?;
    }
    let store = Store::open(path)?;
    store.upsert_book(&Book {
        intro: "Keep introduction".into(),
        source: "local".into(),
        ..Book::new("book", "Book title")
    })?;
    store.reset_book_read_stats();
    store.reset_book_introduction_read_stats();
    Ok((directory, store))
}

fn raw_metadata(store: &Store, raw: &str) -> Result<()> {
    store.connection.execute(
        "INSERT INTO settings(key,value) VALUES ('book.metadata:book',?1)
        ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [raw],
    )?;
    Ok(())
}

fn expected(raw: &str, field: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw).unwrap();
    let value = value
        .get("bookInfo")
        .or_else(|| value.get("book"))
        .unwrap_or(&value);
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[test]
fn nonintroduction_metadata_does_not_project_intro_or_raw_book_setting() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    store
        .connection
        .execute("UPDATE books SET intro=x'80' WHERE id='book'", [])?;
    store.connection.execute(
        "INSERT INTO settings(key,value) VALUES ('book.metadata:book',x'80')",
        [],
    )?;
    let detail = store.book_detail_metadata("book", false)?.unwrap();
    assert_eq!(detail.book.title, "Book title");
    assert!(detail.introduction.is_none());
    assert_eq!(store.book_read_stats().intro_copies, 0);
    assert_eq!(
        store.book_introduction_read_stats(),
        BookIntroductionReadStats::default()
    );
    assert!(
        store
            .book_detail_metadata("book", true)
            .unwrap_err()
            .downcast_ref::<BookMetadataUnavailable>()
            .is_some()
    );
    assert_eq!(store.book_read_stats().intro_copies, 0);
    assert_eq!(
        store.connection.query_row(
            "SELECT typeof(intro) FROM books WHERE id='book'",
            [],
            |row| row.get::<_, String>(0)
        )?,
        "blob"
    );
    Ok(())
}

#[test]
fn projected_metadata_matches_wrapper_shadowing_and_last_duplicate_semantics() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    for raw in [
        r#"{"publisher":"Root","isbn":" root "}"#,
        r#"{"publisher":"Wrong","book":{"publisher":"Book"},"bookInfo":{"publisher":"Info","isbn":" 123 "}}"#,
        r#"{"publisher":"Wrong","bookInfo":null,"book":{"publisher":"Book"}}"#,
        r#"{"book":false,"publisher":"Wrong"}"#,
        r#"{"bookInfo":{"publisher":"First","publisher":"Last","isbn":5}}"#,
        r#"{"bookInfo":{"publisher":"First"},"bookInfo":null}"#,
        r#"{"bookInfo":{"\u0070ublisher":" \u4e2d\u0000 \u3000","isbn":" \u3000 "}}"#,
    ] {
        raw_metadata(&store, raw)?;
        let intro = store.book_introduction("book")?.unwrap();
        assert_eq!(intro.publisher, expected(raw, "publisher"));
        assert_eq!(intro.isbn, expected(raw, "isbn"));
        assert_eq!(intro.intro, "Keep introduction");
    }
    Ok(())
}

#[test]
fn unselected_wrapper_fields_are_skipped_before_needed_field_copy_limits() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    let raw = format!(
        "{{\"publisher\":\"{}\",\"book\":{{\"publisher\":\"{}\"}},\"bookInfo\":{{\"publisher\":\"Info\",\"isbn\":\"123\"}}}}",
        "x".repeat(128),
        "y".repeat(128)
    );
    raw_metadata(&store, &raw)?;
    with_book_introduction_test_limits(
        BookIntroductionLimits {
            field_bytes: 4,
            ..Default::default()
        },
        || -> Result<()> {
            let intro = store.book_introduction("book")?.unwrap();
            assert_eq!(intro.publisher.as_deref(), Some("Info"));
            assert_eq!(intro.isbn.as_deref(), Some("123"));
            Ok(())
        },
    )
}

#[test]
fn metadata_node_and_field_exact_limits_deny_before_any_intro_copy() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    raw_metadata(&store, r#"{"bookInfo":{"publisher":"abc","isbn":"y"}}"#)?;
    let exact = BookIntroductionLimits {
        metadata_nodes: 7,
        field_bytes: 3,
        ..Default::default()
    };
    with_book_introduction_test_limits(exact, || -> Result<()> {
        assert_eq!(
            store
                .book_introduction("book")?
                .unwrap()
                .publisher
                .as_deref(),
            Some("abc")
        );
        assert_eq!(store.book_introduction_read_stats().metadata_nodes, 7);
        Ok(())
    })?;
    for limits in [
        BookIntroductionLimits {
            metadata_nodes: 6,
            ..exact
        },
        BookIntroductionLimits {
            field_bytes: 2,
            ..exact
        },
    ] {
        store.reset_book_read_stats();
        store.reset_book_introduction_read_stats();
        with_book_introduction_test_limits(limits, || -> Result<()> {
            assert!(
                store
                    .book_introduction("book")
                    .unwrap_err()
                    .downcast_ref::<BookMetadataLimit>()
                    .is_some()
            );
            assert_eq!(store.book_read_stats().intro_copies, 0);
            assert_eq!(store.book_introduction_read_stats().metadata_reads, 1);
            Ok(())
        })?;
        assert_eq!(
            store
                .connection
                .query_row("SELECT intro FROM books WHERE id='book'", [], |row| row
                    .get::<_, String>(
                    0
                ))?,
            "Keep introduction"
        );
    }
    Ok(())
}

#[test]
fn utf16_and_exact_copy_forecast_preserve_the_complete_needed_introduction() -> Result<()> {
    for encoding in ["UTF-8", "UTF-16le", "UTF-16be"] {
        let (_directory, store) = fixture(encoding)?;
        store
            .connection
            .execute("UPDATE books SET intro=?1 WHERE id='book'", ["\u{4e2d}"])?;
        let raw: i64 = store.connection.query_row(
            "SELECT octet_length(intro) FROM books WHERE id='book'",
            [],
            |row| row.get(0),
        )?;
        let upper = store.text_encoding.utf8_upper(usize::try_from(raw)?)?;
        let required =
            usize::try_from(raw)? + 2 * upper + 2 * std::mem::size_of::<BookIntroduction>();
        let exact = BookIntroductionLimits {
            bytes: required,
            metadata_nodes: 0,
            field_bytes: 0,
            ..Default::default()
        };
        with_book_introduction_test_limits(exact, || -> Result<()> {
            assert_eq!(store.book_introduction("book")?.unwrap().intro, "\u{4e2d}");
            assert_eq!(store.book_read_stats().intro_bytes, "\u{4e2d}".len());
            Ok(())
        })?;
        store.reset_book_read_stats();
        with_book_introduction_test_limits(
            BookIntroductionLimits {
                bytes: required - 1,
                ..exact
            },
            || -> Result<()> {
                assert!(
                    store
                        .book_introduction("book")
                        .unwrap_err()
                        .downcast_ref::<BookMetadataLimit>()
                        .is_some()
                );
                assert_eq!(store.book_read_stats().intro_copies, 0);
                Ok(())
            },
        )?;
    }
    Ok(())
}

#[test]
fn raw_metadata_byte_quota_and_malformed_json_leave_stored_payload_intact() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    let raw = r#"{"publisher":"abc"}"#;
    raw_metadata(&store, raw)?;
    with_book_introduction_test_limits(
        BookIntroductionLimits {
            metadata_bytes: raw.len() - 1,
            ..Default::default()
        },
        || -> Result<()> {
            assert!(
                store
                    .book_introduction("book")
                    .unwrap_err()
                    .downcast_ref::<BookMetadataLimit>()
                    .is_some()
            );
            assert_eq!(store.book_introduction_read_stats().metadata_reads, 0);
            assert_eq!(store.book_read_stats().intro_copies, 0);
            Ok(())
        },
    )?;
    raw_metadata(&store, "{")?;
    assert!(store.book_detail_metadata("book", false)?.is_some());
    let error = store.book_introduction("book").unwrap_err();
    assert!(error.downcast_ref::<serde_json::Error>().is_some());
    assert_eq!(store.book_read_stats().intro_copies, 0);
    assert_eq!(
        store.connection.query_row(
            "SELECT value FROM settings WHERE key='book.metadata:book'",
            [],
            |row| row.get::<_, String>(0)
        )?,
        "{"
    );
    Ok(())
}

#[test]
fn metadata_and_intro_share_one_snapshot_before_late_book_changes() -> Result<()> {
    let (directory, store) = fixture("UTF-8")?;
    raw_metadata(&store, r#"{"publisher":"Old"}"#)?;
    let other = Connection::open(directory.path().join("introduction.sqlite"))?;
    let _guard = HookGuard;
    HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            other.execute_batch(
                "UPDATE books SET title='Late title',intro='Late introduction' WHERE id='book';
            UPDATE settings SET value='{\"publisher\":\"Late\"}' WHERE key='book.metadata:book'",
            )?;
            Ok(())
        }))
    });
    let detail = store.book_detail_metadata("book", true)?.unwrap();
    assert_eq!(detail.book.title, "Book title");
    let intro = detail.introduction.unwrap();
    assert_eq!(intro.intro, "Keep introduction");
    assert_eq!(intro.publisher.as_deref(), Some("Old"));
    assert_eq!(
        store.book_introduction("book")?.unwrap().intro,
        "Late introduction"
    );
    Ok(())
}

#[test]
fn unavailable_intro_text_and_sql_errors_are_not_missing_books() -> Result<()> {
    let (_directory, store) = fixture("UTF-8")?;
    for value in ["x'80'", "CAST(x'80' AS TEXT)"] {
        store.connection.execute(
            &format!("UPDATE books SET intro={value} WHERE id='book'"),
            [],
        )?;
        assert!(
            store
                .book_introduction("book")
                .unwrap_err()
                .downcast_ref::<BookMetadataUnavailable>()
                .is_some()
        );
        assert_eq!(store.book_read_stats().intro_copies, 0);
    }
    store
        .connection
        .execute_batch("ALTER TABLE books RENAME TO retained_books")?;
    let error = store.book_detail_metadata("book", false).unwrap_err();
    store
        .connection
        .execute_batch("ALTER TABLE retained_books RENAME TO books")?;
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
    assert!(error.downcast_ref::<BookMetadataLimit>().is_none());
    Ok(())
}
