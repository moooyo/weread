//! Selected SQL metadata preserves raw fields without owned body/intro copies.

use super::*;
use std::cell::RefCell;

thread_local! {
    static BOOK_HOOK: RefCell<Option<Box<dyn FnOnce() -> Result<()>>>> = const { RefCell::new(None) };
    static CHAPTER_HOOK: RefCell<Option<Box<dyn FnOnce() -> Result<()>>>> = const { RefCell::new(None) };
}

pub(super) fn after_book_preflight() -> Result<()> {
    let hook = BOOK_HOOK.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook()?;
    }
    Ok(())
}

pub(super) fn after_chapter_preflight() -> Result<()> {
    let hook = CHAPTER_HOOK.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook()?;
    }
    Ok(())
}

struct HookGuard;

impl Drop for HookGuard {
    fn drop(&mut self) {
        BOOK_HOOK.with(|slot| slot.borrow_mut().take());
        CHAPTER_HOOK.with(|slot| slot.borrow_mut().take());
    }
}

struct Fixture {
    store: Store,
    directory: tempfile::TempDir,
    book: Book,
    chapter: Chapter,
    annotation: Annotation,
    task: DownloadTask,
}

impl Fixture {
    fn new(encoding: &str) -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("selected.sqlite");
        {
            let mut connection = Connection::open(&path)?;
            connection.pragma_update(None, "encoding", encoding)?;
            initialize_schema(&mut connection)?;
        }
        let store = Store::open(&path)?;
        store.upsert_book(&Book {
            source: "weread".into(),
            intro: format!("Keep introduction:{}", "x".repeat(4096)),
            progress: 0.25,
            finished: true,
            ..Book::new("book", "Book title")
        })?;
        store.upsert_book(&Book {
            source: "local".into(),
            intro: "Keep other introduction".into(),
            ..Book::new("other", "Other title")
        })?;
        let chapter = Chapter {
            id: "selected-uid".into(),
            book_id: "book".into(),
            index: 0,
            title: "Chapter title".into(),
            body: Some("Selected body".into()),
            cached: true,
            content_status: ContentStatus::Full,
            content_account_key: "selected-owner".into(),
        };
        store.upsert_chapter(&chapter)?;
        store.upsert_chapter(&Chapter {
            id: "unrelated-uid".into(),
            book_id: "book".into(),
            index: 1,
            title: "Unrelated title".into(),
            body: Some("Unrelated body.".repeat(256)),
            content_status: ContentStatus::Preview,
            content_account_key: "other-owner".into(),
            ..Default::default()
        })?;
        store.upsert_chapter(&Chapter {
            id: "selected-uid".into(),
            book_id: "other".into(),
            index: 0,
            title: "Other book chapter".into(),
            body: Some("Keep other body".into()),
            ..Default::default()
        })?;
        let annotation = Annotation {
            id: "retained-note".into(),
            book_id: "book".into(),
            start: 0,
            end: 3,
            quote: "Sel".into(),
            note: "Keep selected note".into(),
            created_at: 10,
            ..Default::default()
        };
        let annotation_borrows = annotation_body_budget_tests::borrowed_count();
        store.save_annotation(&annotation)?;
        let task = DownloadTask {
            id: "retained-task".into(),
            book_id: "book".into(),
            account_key: "selected-owner".into(),
            chapters: vec![0],
            status: DownloadStatus::Pending,
            updated_at: 10,
            ..Default::default()
        };
        store.save_task(&task)?;
        store.set_setting("selected.fixture.account", &"selected-owner")?;
        store.set_setting("selected.fixture.preferences", &vec!["keep", "preferences"])?;
        let book = store.book("book")?.unwrap();
        // save_annotation now validates its range with an admitted borrowed body.
        // Establish that boundary before measuring the metadata API phase.
        assert_eq!(store.chapter_body_read_count(), 0);
        assert_eq!(store.chapter_body_materialization_stats("book", 0), (0, 0));
        assert_eq!(
            annotation_body_budget_tests::borrowed_count(),
            annotation_borrows + 1
        );
        assert_eq!(store.materialized_chapter_bytes(), 0);
        assert_eq!(store.book_read_stats().intro_copies, 1);
        assert_eq!(store.book_read_stats().intro_bytes, book.intro.len());
        assert_eq!(
            store.selected_metadata_read_stats(),
            SelectedMetadataReadStats::default()
        );
        store.chapter_body_reads.set(0);
        store.body_materializations.borrow_mut().clear();
        store.reset_book_read_stats();
        store.reset_selected_metadata_read_stats();
        assert_no_owned_payload(&store);
        Ok(Self {
            store,
            directory,
            book,
            chapter,
            annotation,
            task,
        })
    }

    fn assert_retained(&self) -> Result<()> {
        // Intentional full reads occur after the metadata API assertions.
        assert_eq!(self.store.book("book")?, Some(self.book.clone()));
        assert_eq!(self.store.chapter("book", 0)?, Some(self.chapter.clone()));
        assert_eq!(
            self.store.annotations("book")?,
            vec![self.annotation.clone()]
        );
        assert_eq!(self.store.task(&self.task.id)?, Some(self.task.clone()));
        assert_eq!(
            self.store.setting::<String>("selected.fixture.account")?,
            Some("selected-owner".into())
        );
        assert_eq!(
            self.store
                .setting::<Vec<String>>("selected.fixture.preferences")?,
            Some(vec!["keep".into(), "preferences".into()])
        );
        Ok(())
    }
}

fn assert_no_owned_payload(store: &Store) {
    assert_eq!(store.book_read_stats().intro_copies, 0);
    assert_eq!(store.book_read_stats().intro_bytes, 0);
    assert_eq!(store.chapter_body_read_count(), 0);
    assert_eq!(store.materialized_chapter_bytes(), 0);
    assert_eq!(store.chapter_body_materialization_stats("book", 0), (0, 0));
}

fn access_fixed_bytes(book_id: &str) -> usize {
    book_id.len()
        + 2 * std::mem::size_of::<ChapterAccessMetadata>()
        + std::mem::size_of::<(i64, i64, i64, i64, i64, i64, bool, bool)>()
}

fn text_fixed_bytes() -> usize {
    2 * (std::mem::size_of::<String>() + std::mem::size_of::<i64>())
}

#[test]
fn selected_access_ignores_other_rows_and_owns_no_body_or_introduction() -> Result<()> {
    let fixture = Fixture::new("UTF-8")?;
    assert_eq!(fixture.store.connection.execute("UPDATE chapters SET source_id=?1,title=x'80',body=x'80',content_status='preview' WHERE book_id='book' AND chapter_index=1", ["u".repeat(4097)])?, 1);
    let entry = fixture.store.chapter_access_metadata("book", 0)?.unwrap();
    let mut expected = fixture.chapter.clone();
    expected.body = None;
    assert_eq!(entry.chapter, expected);
    assert!(entry.body_present && entry.locally_readable && entry.chapter.cached);
    let stats = fixture.store.selected_metadata_read_stats();
    assert_eq!(
        (
            stats.chapter_rows,
            stats.body_borrows,
            stats.body_scan_bytes
        ),
        (1, 1, "Selected body".len())
    );
    assert_eq!(
        stats.chapter_string_bytes,
        "selected-uid".len() + "Chapter title".len() + "selected-owner".len() + "book".len()
    );
    assert_no_owned_payload(&fixture.store);
    fixture.assert_retained()
}

#[test]
fn selected_access_preserves_null_empty_unicode_whitespace_and_nul_cached_semantics() -> Result<()>
{
    for body in [
        None,
        Some(""),
        Some(" \n\u{3000}\u{85}"),
        Some("\0"),
        Some("\u{3000}\0\u{3000}"),
        Some("\u{4e2d}"),
    ] {
        let fixture = Fixture::new("UTF-8")?;
        fixture.store.connection.execute("UPDATE chapters SET body=?1,content_status='full' WHERE book_id='book' AND chapter_index=0", [body])?;
        let entry = fixture.store.chapter_access_metadata("book", 0)?.unwrap();
        let nonblank = body.is_some_and(|body| !body.trim().is_empty());
        assert_eq!(
            (
                entry.body_present,
                entry.locally_readable,
                entry.chapter.cached
            ),
            (body.is_some(), nonblank, nonblank)
        );
        assert!(entry.chapter.body.is_none());
        assert_eq!(
            fixture.store.selected_metadata_read_stats().body_borrows,
            usize::from(body.is_some_and(|body| !body.is_empty()))
        );
        assert_no_owned_payload(&fixture.store);
        let original = fixture.store.chapter("book", 0)?.unwrap();
        assert_eq!(original.cached, entry.chapter.cached);
        assert_eq!(original.body.as_deref(), body);
    }
    Ok(())
}

#[test]
fn title_and_raw_uid_only_reads_ignore_body_owner_status_and_preserve_empty_nul() -> Result<()> {
    let fixture = Fixture::new("UTF-8")?;
    assert_eq!(fixture.store.connection.execute("UPDATE chapters SET body=x'80',content_account_key=x'80',content_status='locked' WHERE book_id='book' AND chapter_index=0", [])?, 1);
    for value in ["", " \u{4e2d}\0value"] {
        fixture.store.connection.execute(
            "UPDATE chapters SET title=?1,source_id=?1 WHERE book_id='book' AND chapter_index=0",
            [value],
        )?;
        fixture.store.reset_selected_metadata_read_stats();
        assert_eq!(fixture.store.chapter_title("book", 0)?, Some(value.into()));
        assert_eq!(
            fixture.store.chapter_source_id("book", 0)?,
            Some(value.into())
        );
        let stats = fixture.store.selected_metadata_read_stats();
        assert_eq!(
            (
                stats.chapter_rows,
                stats.chapter_string_bytes,
                stats.body_borrows
            ),
            (2, 2 * value.len(), 0)
        );
        assert_no_owned_payload(&fixture.store);
    }
    assert_eq!(fixture.store.chapter_title("book", 99)?, None);
    assert_eq!(fixture.store.chapter_source_id("missing", 0)?, None);
    Ok(())
}

#[test]
fn selected_access_exact_and_one_less_admit_all_phases_before_any_text_copy() -> Result<()> {
    let fixture = Fixture::new("UTF-8")?;
    let text = "selected-uid".len() + "Chapter title".len() + "selected-owner".len() + "full".len();
    let body = "Selected body".len();
    let fixed = access_fixed_bytes("book");
    let exact = ChapterAccessLimits {
        metadata_bytes: text + fixed,
        work_bytes: text + 2 * body,
        temporary_bytes: (2 * text + fixed).max(2 * body + fixed),
    };
    with_chapter_access_test_limits(exact, || -> Result<()> {
        assert!(
            fixture
                .store
                .chapter_access_metadata("book", 0)?
                .unwrap()
                .locally_readable
        );
        assert_eq!(fixture.store.selected_metadata_read_stats().body_borrows, 1);
        assert_no_owned_payload(&fixture.store);
        Ok(())
    })?;
    for limits in [
        ChapterAccessLimits {
            metadata_bytes: exact.metadata_bytes - 1,
            ..exact
        },
        ChapterAccessLimits {
            work_bytes: exact.work_bytes - 1,
            ..exact
        },
        ChapterAccessLimits {
            temporary_bytes: exact.temporary_bytes - 1,
            ..exact
        },
    ] {
        fixture.store.reset_selected_metadata_read_stats();
        let _guard = HookGuard;
        CHAPTER_HOOK
            .with(|slot| *slot.borrow_mut() = Some(Box::new(|| panic!("unadmitted text phase"))));
        with_chapter_access_test_limits(limits, || -> Result<()> {
            assert!(
                fixture
                    .store
                    .chapter_access_metadata("book", 0)
                    .unwrap_err()
                    .downcast_ref::<CachedResourceLimit>()
                    .is_some()
            );
            assert_eq!(
                fixture.store.selected_metadata_read_stats(),
                SelectedMetadataReadStats::default()
            );
            assert_no_owned_payload(&fixture.store);
            Ok(())
        })?;
    }
    fixture.assert_retained()
}

#[test]
fn invalid_needed_chapter_fields_fail_without_owning_payloads() -> Result<()> {
    for assignment in [
        "title=x'80'",
        "source_id=x'80'",
        "content_account_key=x'80'",
        "body=x'80'",
        "body=CAST(x'80' AS TEXT)",
    ] {
        let fixture = Fixture::new("UTF-8")?;
        fixture.store.connection.execute(
            &format!("UPDATE chapters SET {assignment} WHERE book_id='book' AND chapter_index=0"),
            [],
        )?;
        assert!(
            fixture
                .store
                .chapter_access_metadata("book", 0)
                .unwrap_err()
                .downcast_ref::<CachedResourceLimit>()
                .is_some()
        );
        assert_eq!(fixture.store.selected_metadata_read_stats().chapter_rows, 0);
        assert_no_owned_payload(&fixture.store);
    }
    for (column, value) in [
        ("source_id", "u".repeat(4097)),
        ("content_account_key", "o".repeat(16 * 1024 + 1)),
    ] {
        let fixture = Fixture::new("UTF-8")?;
        fixture.store.connection.execute(
            &format!("UPDATE chapters SET {column}=?1 WHERE book_id='book' AND chapter_index=0"),
            [value],
        )?;
        assert!(
            fixture
                .store
                .chapter_access_metadata("book", 0)
                .unwrap_err()
                .downcast_ref::<CachedResourceLimit>()
                .is_some()
        );
        assert_eq!(
            fixture.store.selected_metadata_read_stats(),
            SelectedMetadataReadStats::default()
        );
        assert_no_owned_payload(&fixture.store);
    }
    // The production CHECK prevents invalid status bytes from reaching the
    // reader. Prove that exact write boundary without disabling the schema.
    for invalid in ["invalid".to_owned(), "s".repeat(33)] {
        let fixture = Fixture::new("UTF-8")?;
        let error = fixture
            .store
            .connection
            .execute(
                "UPDATE chapters SET content_status=?1 WHERE book_id='book' AND chapter_index=0",
                [&invalid],
            )
            .unwrap_err();
        match error {
            rusqlite::Error::SqliteFailure(code, _) => {
                assert_eq!(code.extended_code, rusqlite::ffi::SQLITE_CONSTRAINT_CHECK)
            }
            error => panic!("Expected a status CHECK rejection: {error}"),
        }
        let entry = fixture.store.chapter_access_metadata("book", 0)?.unwrap();
        assert_eq!(entry.chapter.content_status, ContentStatus::Full);
        assert!(entry.chapter.cached && entry.locally_readable && entry.body_present);
        assert_no_owned_payload(&fixture.store);
        fixture.assert_retained()?;
    }
    for (status, expected) in [
        ("unknown", ContentStatus::Unknown),
        ("full", ContentStatus::Full),
        ("preview", ContentStatus::Preview),
        ("locked", ContentStatus::Locked),
    ] {
        let fixture = Fixture::new("UTF-8")?;
        assert_eq!(
            fixture.store.connection.execute(
                "UPDATE chapters SET content_status=?1 WHERE book_id='book' AND chapter_index=0",
                [status],
            )?,
            1
        );
        let entry = fixture.store.chapter_access_metadata("book", 0)?.unwrap();
        assert_eq!(entry.chapter.content_status, expected);
        assert_eq!(entry.chapter.cached, expected == ContentStatus::Full);
        assert!(entry.locally_readable && entry.body_present);
        assert_no_owned_payload(&fixture.store);
    }
    Ok(())
}

#[test]
fn utf16_selected_proof_prices_raw_and_converted_body_before_borrow() -> Result<()> {
    for encoding in ["UTF-16le", "UTF-16be"] {
        let fixture = Fixture::new(encoding)?;
        fixture.store.connection.execute(
            "UPDATE chapters SET title=?1,body=?2 WHERE book_id='book' AND chapter_index=0",
            params!["\u{4e2d}", "\u{4e2d}\u{1f600}\0"],
        )?;
        let (raw_text, nonempty, raw_body): (i64, i64, i64) = fixture.store.connection.query_row(
            "SELECT octet_length(source_id)+octet_length(title)+octet_length(content_account_key)+octet_length(content_status),
             (octet_length(source_id)>0)+(octet_length(title)>0)+(octet_length(content_account_key)>0)+(octet_length(content_status)>0),
             octet_length(body) FROM chapters WHERE book_id='book' AND chapter_index=0", [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(raw_body, 8);
        // All fixture cells have even UTF16 byte lengths, so summing the
        // individual ceil(n/2)*3 bounds equals raw_text/2*3 exactly.
        assert_eq!(nonempty, 4);
        let text = usize::try_from(raw_text)? / 2 * 3;
        let body = 12;
        let fixed = access_fixed_bytes("book");
        let exact = ChapterAccessLimits {
            metadata_bytes: text + fixed,
            work_bytes: text + 2 * body,
            temporary_bytes: (2 * text + fixed).max(8 + body + fixed),
        };
        with_chapter_access_test_limits(exact, || -> Result<()> {
            let entry = fixture.store.chapter_access_metadata("book", 0)?.unwrap();
            assert_eq!(entry.chapter.title, "\u{4e2d}");
            assert!(entry.locally_readable && entry.chapter.cached);
            assert_eq!(
                fixture.store.selected_metadata_read_stats().body_scan_bytes,
                "\u{4e2d}\u{1f600}\0".len()
            );
            assert_no_owned_payload(&fixture.store);
            Ok(())
        })?;
        fixture.store.reset_selected_metadata_read_stats();
        with_chapter_access_test_limits(
            ChapterAccessLimits {
                work_bytes: exact.work_bytes - 1,
                ..exact
            },
            || -> Result<()> {
                assert!(
                    fixture
                        .store
                        .chapter_access_metadata("book", 0)
                        .unwrap_err()
                        .downcast_ref::<CachedResourceLimit>()
                        .is_some()
                );
                assert_eq!(
                    fixture.store.selected_metadata_read_stats(),
                    SelectedMetadataReadStats::default()
                );
                Ok(())
            },
        )?;
    }
    Ok(())
}

#[test]
fn selected_metadata_snapshot_does_not_adopt_late_uid_owner_or_body() -> Result<()> {
    let fixture = Fixture::new("UTF-8")?;
    let other = Connection::open(fixture.directory.path().join("selected.sqlite"))?;
    let _guard = HookGuard;
    CHAPTER_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(move || {
        other.execute("UPDATE chapters SET source_id='late-uid',title='Late title',content_account_key='late-owner',body='Late body',content_status='preview' WHERE book_id='book' AND chapter_index=0", [])?;
        Ok(())
    })));
    let entry = fixture.store.chapter_access_metadata("book", 0)?.unwrap();
    assert_eq!(
        (
            entry.chapter.id.as_str(),
            entry.chapter.title.as_str(),
            entry.chapter.content_account_key.as_str(),
            entry.chapter.content_status
        ),
        (
            "selected-uid",
            "Chapter title",
            "selected-owner",
            ContentStatus::Full
        )
    );
    assert_eq!(
        fixture.store.selected_metadata_read_stats().body_scan_bytes,
        "Selected body".len()
    );
    assert_no_owned_payload(&fixture.store);
    let current = fixture.store.chapter_access_metadata("book", 0)?.unwrap();
    assert_eq!(
        (current.chapter.id.as_str(), current.chapter.content_status),
        ("late-uid", ContentStatus::Preview)
    );
    assert!(current.locally_readable && !current.chapter.cached);
    Ok(())
}

#[test]
fn book_scalar_and_selected_text_reads_ignore_unrelated_introductions_and_types() -> Result<()> {
    let fixture = Fixture::new("UTF-8")?;
    fixture.store.connection.execute(
        "UPDATE books SET intro=x'80',author=x'80',category=x'80' WHERE id='book'",
        [],
    )?;
    assert!(fixture.store.book_exists("book")?);
    assert!(!fixture.store.book_exists("missing")?);
    assert_eq!(fixture.store.book_source("book")?, Some("weread".into()));
    assert_eq!(fixture.store.book_title("book")?, Some("Book title".into()));
    assert_eq!(
        fixture.store.book_reading_state("book")?,
        Some(BookReadingState {
            progress: 0.25,
            finished: true
        })
    );
    assert_eq!(fixture.store.book_source("missing")?, None);
    assert_eq!(fixture.store.book_title("missing")?, None);
    assert_eq!(fixture.store.book_reading_state("missing")?, None);
    assert_eq!(
        fixture.store.selected_metadata_read_stats().book_text_rows,
        2
    );
    assert_no_owned_payload(&fixture.store);
    fixture
        .store
        .connection
        .execute("UPDATE books SET source=x'80' WHERE id='book'", [])?;
    assert!(fixture.store.book_exists("book")?);
    assert_eq!(fixture.store.book_title("book")?, Some("Book title".into()));
    assert!(
        fixture
            .store
            .book_source("book")
            .unwrap_err()
            .downcast_ref::<BookMetadataUnavailable>()
            .is_some()
    );
    fixture
        .store
        .connection
        .execute("UPDATE books SET finished='invalid' WHERE id='book'", [])?;
    assert!(
        fixture
            .store
            .book_reading_state("book")
            .unwrap_err()
            .downcast_ref::<BookMetadataUnavailable>()
            .is_some()
    );
    assert_no_owned_payload(&fixture.store);
    Ok(())
}

#[test]
fn selected_book_text_exact_quota_preserves_empty_nul_unicode_and_utf16() -> Result<()> {
    for encoding in ["UTF-8", "UTF-16le", "UTF-16be"] {
        let fixture = Fixture::new(encoding)?;
        for title in ["", "\u{4e2d}\0title"] {
            fixture
                .store
                .connection
                .execute("UPDATE books SET title=?1 WHERE id='book'", [title])?;
            let raw: i64 = fixture.store.connection.query_row(
                "SELECT octet_length(title) FROM books WHERE id='book'",
                [],
                |row| row.get(0),
            )?;
            let upper = fixture
                .store
                .text_encoding
                .utf8_upper(usize::try_from(raw)?)?;
            let required = text_fixed_bytes() + upper;
            fixture.store.reset_selected_metadata_read_stats();
            fixture.store.reset_book_read_stats();
            with_book_metadata_test_limits(
                BookMetadataLimits {
                    rows: 1,
                    sql_cells: 4,
                    bytes: required,
                    ..Default::default()
                },
                || -> Result<()> {
                    assert_eq!(fixture.store.book_title("book")?, Some(title.into()));
                    assert_eq!(
                        fixture.store.selected_metadata_read_stats().book_text_bytes,
                        title.len()
                    );
                    assert_no_owned_payload(&fixture.store);
                    Ok(())
                },
            )?;
            fixture.store.reset_selected_metadata_read_stats();
            fixture.store.reset_book_read_stats();
            let _guard = HookGuard;
            BOOK_HOOK.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(|| panic!("unadmitted book field")))
            });
            with_book_metadata_test_limits(
                BookMetadataLimits {
                    bytes: required - 1,
                    ..Default::default()
                },
                || -> Result<()> {
                    assert!(
                        fixture
                            .store
                            .book_title("book")
                            .unwrap_err()
                            .downcast_ref::<BookMetadataLimit>()
                            .is_some()
                    );
                    assert_eq!(
                        fixture.store.selected_metadata_read_stats(),
                        SelectedMetadataReadStats::default()
                    );
                    assert_eq!(fixture.store.book_read_stats(), BookReadStats::default());
                    Ok(())
                },
            )?;
        }
    }
    Ok(())
}

#[test]
fn selected_book_snapshot_and_capacity_errors_keep_full_history() -> Result<()> {
    let fixture = Fixture::new("UTF-8")?;
    with_book_metadata_test_limits(
        BookMetadataLimits {
            bytes: 1,
            ..Default::default()
        },
        || -> Result<()> {
            assert!(fixture.store.book_exists("book")?);
            assert!(
                fixture
                    .store
                    .book_source("book")
                    .unwrap_err()
                    .downcast_ref::<BookMetadataLimit>()
                    .is_some()
            );
            assert!(
                fixture
                    .store
                    .book_reading_state("book")
                    .unwrap_err()
                    .downcast_ref::<BookMetadataLimit>()
                    .is_some()
            );
            assert_eq!(
                fixture.store.selected_metadata_read_stats().book_text_rows,
                0
            );
            assert_no_owned_payload(&fixture.store);
            Ok(())
        },
    )?;
    let other = Connection::open(fixture.directory.path().join("selected.sqlite"))?;
    let _guard = HookGuard;
    BOOK_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            other.execute(
                "UPDATE books SET title='Late title',intro='Late intro' WHERE id='book'",
                [],
            )?;
            Ok(())
        }))
    });
    assert_eq!(fixture.store.book_title("book")?, Some("Book title".into()));
    assert_eq!(fixture.store.book_title("book")?, Some("Late title".into()));
    assert_no_owned_payload(&fixture.store);
    assert_eq!(fixture.store.book("book")?.unwrap().intro, "Late intro");
    assert_eq!(fixture.store.chapter("book", 0)?, Some(fixture.chapter));
    assert_eq!(fixture.store.annotations("book")?, vec![fixture.annotation]);
    assert_eq!(fixture.store.task(&fixture.task.id)?, Some(fixture.task));
    Ok(())
}

#[test]
fn selected_sql_failures_remain_sql_errors_without_payload_copies_or_data_loss() -> Result<()> {
    let fixture = Fixture::new("UTF-8")?;
    fixture
        .store
        .connection
        .execute_batch("ALTER TABLE books RENAME TO retained_books")?;
    let failures = vec![
        fixture.store.book_exists("book").unwrap_err(),
        fixture.store.book_source("book").unwrap_err(),
        fixture.store.book_title("book").unwrap_err(),
        fixture.store.book_reading_state("book").unwrap_err(),
    ];
    fixture
        .store
        .connection
        .execute_batch("ALTER TABLE retained_books RENAME TO books")?;
    fixture
        .store
        .connection
        .execute_batch("ALTER TABLE chapters RENAME TO retained_chapters")?;
    let failures = failures.into_iter().chain([
        fixture
            .store
            .chapter_access_metadata("book", 0)
            .unwrap_err(),
        fixture.store.chapter_title("book", 0).unwrap_err(),
        fixture.store.chapter_source_id("book", 0).unwrap_err(),
    ]);
    fixture
        .store
        .connection
        .execute_batch("ALTER TABLE retained_chapters RENAME TO chapters")?;
    for error in failures {
        assert!(error.downcast_ref::<rusqlite::Error>().is_some());
        assert!(error.downcast_ref::<CachedResourceLimit>().is_none());
        assert!(error.downcast_ref::<BookMetadataLimit>().is_none());
        assert!(error.downcast_ref::<BookMetadataUnavailable>().is_none());
    }
    assert_no_owned_payload(&fixture.store);
    fixture.assert_retained()
}
