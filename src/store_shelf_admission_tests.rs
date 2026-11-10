//! Genuine small SQL shelves admit only their first ID and retain private data.

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

struct Fixture {
    store: Store,
    directory: tempfile::TempDir,
    book: Book,
    chapter: Chapter,
    annotation: Annotation,
    bookmark: Bookmark,
    position: ReadingPosition,
    task: DownloadTask,
}

impl Fixture {
    fn new(encoding: &str) -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("shelf.sqlite");
        {
            let mut connection = Connection::open(&path)?;
            connection.pragma_update(None, "encoding", encoding)?;
            initialize_schema(&mut connection)?;
        }
        let store = Store::open(&path)?;
        for (id, on_shelf) in [("history", false), ("first", true), ("second", true)] {
            store.upsert_book(&Book {
                source: "weread".into(),
                intro: format!("Keep {id} introduction:{}", "x".repeat(4096)),
                on_shelf,
                ..Book::new(id, format!("Shelf fixture {id}"))
            })?;
        }
        store.upsert_chapter(&Chapter {
            id: "history-uid".into(),
            book_id: "history".into(),
            title: "Retained chapter".into(),
            body: Some("Keep retained chapter body".into()),
            cached: true,
            content_status: ContentStatus::Full,
            content_account_key: "synthetic-shelf-account".into(),
            ..Default::default()
        })?;
        let annotation = Annotation {
            id: "history-note".into(),
            book_id: "history".into(),
            start: 0,
            end: 4,
            quote: "Keep".into(),
            note: "Keep retained note".into(),
            created_at: 10,
            ..Default::default()
        };
        store.save_annotation(&annotation)?;
        let bookmark = Bookmark {
            id: "history-mark".into(),
            book_id: "history".into(),
            char_offset: 1,
            label: "Keep retained bookmark".into(),
            created_at: 10,
            ..Default::default()
        };
        store.save_bookmark(&bookmark)?;
        let position = ReadingPosition {
            book_id: "history".into(),
            char_offset: 2,
            updated_at: 10,
            device: "synthetic-shelf-device".into(),
            ..Default::default()
        };
        store.save_position(&position)?;
        let task = DownloadTask {
            id: "history-pending".into(),
            book_id: "history".into(),
            account_key: "synthetic-shelf-account".into(),
            chapters: vec![0],
            status: DownloadStatus::Pending,
            updated_at: 10,
            ..Default::default()
        };
        store.save_task(&task)?;
        store.set_setting("shelf.fixture.account", &"synthetic-shelf-account")?;
        store.set_setting("shelf.fixture.preferences", &vec!["keep", "preferences"])?;
        let book = store.book("history")?.unwrap();
        let chapter = store.chapter("history", 0)?.unwrap();
        store.reset_book_read_stats();
        Ok(Self {
            directory,
            store,
            book,
            chapter,
            annotation,
            bookmark,
            position,
            task,
        })
    }

    fn assert_history(&self) -> Result<()> {
        assert_eq!(self.store.book("history")?, Some(self.book.clone()));
        assert_eq!(
            self.store.chapter("history", 0)?,
            Some(self.chapter.clone())
        );
        assert_eq!(
            self.store.annotations("history")?,
            vec![self.annotation.clone()]
        );
        assert_eq!(
            self.store.bookmarks("history")?,
            vec![self.bookmark.clone()]
        );
        assert_eq!(self.store.position("history")?, Some(self.position.clone()));
        assert_eq!(self.store.task(&self.task.id)?, Some(self.task.clone()));
        assert_eq!(
            self.store.setting::<String>("shelf.fixture.account")?,
            Some("synthetic-shelf-account".into())
        );
        assert_eq!(
            self.store
                .setting::<Vec<String>>("shelf.fixture.preferences")?,
            Some(vec!["keep".into(), "preferences".into()])
        );
        Ok(())
    }

    fn first_rowid(&self) -> Result<i64> {
        Ok(self.store.connection.query_row(
            "SELECT rowid FROM books WHERE on_shelf = 1 ORDER BY rowid LIMIT 1",
            [],
            |row| row.get(0),
        )?)
    }

    fn assert_shelf_introductions(&self) -> Result<()> {
        // Full reads here occur after the API materialization assertions.
        for id in ["first", "second"] {
            assert_eq!(
                self.store.book(id)?.unwrap().intro,
                format!("Keep {id} introduction:{}", "x".repeat(4096))
            );
        }
        Ok(())
    }
}

fn fixed_id_bytes() -> usize {
    2 * (std::mem::size_of::<String>() + std::mem::size_of::<i64>())
}

#[test]
fn shelf_membership_and_first_id_do_not_project_unrelated_book_fields() -> Result<()> {
    let fixture = Fixture::new("UTF-8")?;
    let expected = fixture.store.shelf()?.first().unwrap().id.clone();
    assert_eq!(expected, "first");
    // Neither consumer needs these fields, even on the selected row. A later
    // malformed ID must not make the whole shelf a prerequisite for selection.
    fixture.store.connection.execute(
        "UPDATE books SET title=x'80',author=x'80',intro=x'80' WHERE id='first'",
        [],
    )?;
    fixture.store.connection.execute(
        "UPDATE books SET id=?1 WHERE id='second'",
        ["z".repeat(4097)],
    )?;
    fixture.store.reset_book_read_stats();
    assert!(fixture.store.shelf_exists()?);
    assert_eq!(fixture.store.book_read_stats(), BookReadStats::default());
    assert_eq!(fixture.store.first_shelf_book_id()?, Some(expected.clone()));
    assert_eq!(
        fixture.store.book_read_stats(),
        BookReadStats {
            metadata_rows: 1,
            metadata_string_bytes: expected.len(),
            ..Default::default()
        }
    );
    fixture.assert_history()
}

#[test]
fn empty_shelf_returns_no_id_without_requiring_identifier_budget() -> Result<()> {
    let empty = Store::open(":memory:")?;
    assert!(!empty.shelf_exists()?);
    assert_eq!(empty.first_shelf_book_id()?, None);
    let fixture = Fixture::new("UTF-8")?;
    fixture
        .store
        .connection
        .execute("UPDATE books SET on_shelf=0", [])?;
    fixture.store.reset_book_read_stats();
    with_book_metadata_test_limits(
        BookMetadataLimits {
            rows: 0,
            sql_cells: 0,
            bytes: 0,
            identifier_bytes: 0,
        },
        || -> Result<()> {
            assert!(!fixture.store.shelf_exists()?);
            assert_eq!(fixture.store.first_shelf_book_id()?, None);
            assert_eq!(fixture.store.book_read_stats(), BookReadStats::default());
            Ok(())
        },
    )?;
    fixture.assert_shelf_introductions()?;
    fixture.assert_history()
}

#[test]
fn first_id_exact_fee_and_one_less_gate_rows_sql_fields_and_bytes_before_copy() -> Result<()> {
    let fixture = Fixture::new("UTF-8")?;
    let required = fixed_id_bytes() + "first".len();
    let exact = BookMetadataLimits {
        rows: 1,
        sql_cells: 4,
        bytes: required,
        identifier_bytes: 5,
    };
    with_book_metadata_test_limits(exact, || -> Result<()> {
        assert_eq!(fixture.store.first_shelf_book_id()?, Some("first".into()));
        assert_eq!(
            fixture.store.book_read_stats(),
            BookReadStats {
                metadata_rows: 1,
                metadata_string_bytes: 5,
                ..Default::default()
            }
        );
        Ok(())
    })?;
    for (limits, resource) in [
        (BookMetadataLimits { rows: 0, ..exact }, "rows"),
        (
            BookMetadataLimits {
                sql_cells: 3,
                ..exact
            },
            "SQL cells",
        ),
        (
            BookMetadataLimits {
                bytes: required - 1,
                ..exact
            },
            "working-set bytes",
        ),
        (
            BookMetadataLimits {
                identifier_bytes: 4,
                ..exact
            },
            "identifier bytes",
        ),
    ] {
        fixture.store.reset_book_read_stats();
        with_book_metadata_test_limits(limits, || -> Result<()> {
            assert!(fixture.store.shelf_exists()?);
            let error = fixture.store.first_shelf_book_id().unwrap_err();
            assert_eq!(
                error.downcast_ref::<BookMetadataLimit>().unwrap().resource,
                resource
            );
            assert_eq!(fixture.store.book_read_stats(), BookReadStats::default());
            Ok(())
        })?;
    }
    fixture.assert_shelf_introductions()?;
    fixture.assert_history()
}

#[test]
fn oversized_first_id_is_rejected_without_selecting_a_later_small_id() -> Result<()> {
    let fixture = Fixture::new("UTF-8")?;
    let rowid = fixture.first_rowid()?;
    for length in [4096, 4097] {
        let id = "i".repeat(length);
        fixture
            .store
            .connection
            .execute("UPDATE books SET id=?1 WHERE rowid=?2", params![id, rowid])?;
        fixture.store.reset_book_read_stats();
        assert!(fixture.store.shelf_exists()?);
        let result = fixture.store.first_shelf_book_id();
        if length == 4096 {
            assert_eq!(result?, Some(id));
            assert_eq!(
                fixture.store.book_read_stats().metadata_string_bytes,
                length
            );
            assert_eq!(fixture.store.book_read_stats().metadata_rows, 1);
        } else {
            let error = result.unwrap_err();
            let limit = error.downcast_ref::<BookMetadataLimit>().unwrap();
            assert_eq!(
                (limit.resource, limit.limit, limit.requested),
                ("identifier bytes", 4096, 4097)
            );
            assert_eq!(fixture.store.book_read_stats(), BookReadStats::default());
        }
        assert_eq!(fixture.store.book_read_stats().intro_copies, 0);
    }
    fixture.assert_history()
}

#[test]
fn nontext_malformed_utf8_and_invalid_first_ids_are_unavailable_before_copy() -> Result<()> {
    let fixture = Fixture::new("UTF-8")?;
    let rowid = fixture.first_rowid()?;
    for value in ["x'80'", "CAST(x'80' AS TEXT)", "' '", "'a'||char(0)||'b'"] {
        fixture.store.connection.execute(
            &format!("UPDATE books SET id={value} WHERE rowid=?1"),
            [rowid],
        )?;
        fixture.store.reset_book_read_stats();
        assert!(fixture.store.shelf_exists()?);
        let error = fixture.store.first_shelf_book_id().unwrap_err();
        assert!(error.downcast_ref::<BookMetadataUnavailable>().is_some());
        assert!(error.downcast_ref::<BookMetadataLimit>().is_none());
        assert_eq!(fixture.store.book_read_stats(), BookReadStats::default());
    }
    fixture.assert_history()
}

#[test]
fn utf16_first_id_uses_the_cached_encoding_upper_bound_before_copy() -> Result<()> {
    for encoding in ["UTF-16le", "UTF-16be"] {
        let fixture = Fixture::new(encoding)?;
        let id = "\u{4e2d}\u{1f600}";
        let rowid = fixture.first_rowid()?;
        fixture
            .store
            .connection
            .execute("UPDATE books SET id=?1 WHERE rowid=?2", params![id, rowid])?;
        let encoded: i64 = fixture.store.connection.query_row(
            "SELECT COALESCE(octet_length(id),0) FROM books WHERE rowid=?1",
            [rowid],
            |row| row.get(0),
        )?;
        assert_eq!(encoded, 6);
        let upper = 9;
        let exact = BookMetadataLimits {
            rows: 1,
            sql_cells: 4,
            bytes: fixed_id_bytes() + upper,
            identifier_bytes: upper,
        };
        fixture.store.reset_book_read_stats();
        with_book_metadata_test_limits(exact, || -> Result<()> {
            assert_eq!(fixture.store.first_shelf_book_id()?, Some(id.into()));
            assert_eq!(
                fixture.store.book_read_stats(),
                BookReadStats {
                    metadata_rows: 1,
                    metadata_string_bytes: id.len(),
                    ..Default::default()
                }
            );
            Ok(())
        })?;
        for limits in [
            BookMetadataLimits {
                identifier_bytes: upper - 1,
                ..exact
            },
            BookMetadataLimits {
                bytes: fixed_id_bytes() + upper - 1,
                ..exact
            },
        ] {
            fixture.store.reset_book_read_stats();
            with_book_metadata_test_limits(limits, || -> Result<()> {
                assert!(
                    fixture
                        .store
                        .first_shelf_book_id()
                        .unwrap_err()
                        .downcast_ref::<BookMetadataLimit>()
                        .is_some()
                );
                assert_eq!(fixture.store.book_read_stats(), BookReadStats::default());
                Ok(())
            })?;
        }
        fixture.assert_history()?;
    }
    Ok(())
}

#[test]
fn first_id_snapshot_keeps_preflight_row_when_late_sql_changes_membership() -> Result<()> {
    let fixture = Fixture::new("UTF-8")?;
    let rowid = fixture.first_rowid()?;
    let other = Connection::open(fixture.directory.path().join("shelf.sqlite"))?;
    let _guard = HookGuard;
    HOOK.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(move || {
            other.execute(
                "UPDATE books SET id='late-id',on_shelf=0,intro='Late intro' WHERE rowid=?1",
                [rowid],
            )?;
            Ok(())
        }));
    });
    fixture.store.reset_book_read_stats();
    assert_eq!(fixture.store.first_shelf_book_id()?, Some("first".into()));
    assert_eq!(
        fixture.store.book_read_stats(),
        BookReadStats {
            metadata_rows: 1,
            metadata_string_bytes: 5,
            ..Default::default()
        }
    );
    fixture.store.reset_book_read_stats();
    assert_eq!(fixture.store.first_shelf_book_id()?, Some("second".into()));
    assert_eq!(fixture.store.book_read_stats().intro_copies, 0);
    assert_eq!(fixture.store.book("late-id")?.unwrap().intro, "Late intro");
    fixture.assert_history()
}

#[test]
fn shelf_sql_failure_stays_an_error_and_preserves_private_history() -> Result<()> {
    let fixture = Fixture::new("UTF-8")?;
    fixture
        .store
        .connection
        .execute_batch("ALTER TABLE books RENAME TO retained_books")?;
    fixture.store.reset_book_read_stats();
    let exists = fixture.store.shelf_exists();
    let first = fixture.store.first_shelf_book_id();
    fixture
        .store
        .connection
        .execute_batch("ALTER TABLE retained_books RENAME TO books")?;
    for error in [exists.unwrap_err(), first.unwrap_err()] {
        assert!(error.downcast_ref::<rusqlite::Error>().is_some());
        assert!(error.downcast_ref::<BookMetadataLimit>().is_none());
        assert!(error.downcast_ref::<BookMetadataUnavailable>().is_none());
    }
    assert_eq!(fixture.store.book_read_stats(), BookReadStats::default());
    assert!(fixture.store.shelf_exists()?);
    assert_eq!(fixture.store.first_shelf_book_id()?, Some("first".into()));
    fixture.assert_history()
}
