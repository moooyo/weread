//! Scalar plans prepay unique SQLite TEXT length work without changing truth.

use super::*;
use std::cell::RefCell;

thread_local! { static HOOK: RefCell<Option<Box<dyn FnOnce() -> Result<()>>>> = const { RefCell::new(None) }; }
pub(super) fn before_evaluation() -> Result<()> {
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

fn fixture(encoding: &str) -> Result<(tempfile::TempDir, Store, Book)> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("length.sqlite");
    {
        let mut connection = Connection::open(&path)?;
        connection.pragma_update(None, "encoding", encoding)?;
        migrate(&mut connection)?;
    }
    let store = Store::open(path)?;
    let book = Book {
        source: "weread".into(),
        intro: "Keep original introduction".into(),
        ..Book::new("book", "Length fixture")
    };
    store.upsert_book(&book)?;
    for index in 0..2 {
        store.upsert_chapter(&Chapter {
            id: format!("uid-{index}"),
            book_id: "book".into(),
            index,
            body: Some(format!("Body {index}")),
            content_status: ContentStatus::Full,
            content_account_key: "owner".into(),
            ..Default::default()
        })?;
    }
    store.connection.execute("INSERT INTO annotations(id,book_id,chapter_index,start_offset,end_offset,quote,note,created_at)
        VALUES ('note','book',0,0,1,'B','Keep note',10)", [])?;
    store.set_setting("length.account", &"owner")?;
    store.set_setting("length.preferences", &vec!["keep"])?;
    reset_body_length_read_stats();
    store.reset_book_read_stats();
    Ok((directory, store, book))
}

fn raw_bodies(store: &Store) -> Result<Vec<(i64, rusqlite::types::Value, String, String)>> {
    let mut statement = store.connection.prepare("SELECT chapter_index,body,content_status,content_account_key FROM chapters ORDER BY chapter_index")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn completed(store: &Store, id: &str, chapters: Vec<u32>, owner: &str) -> Result<DownloadTask> {
    let task = DownloadTask {
        id: id.into(),
        book_id: "book".into(),
        account_key: owner.into(),
        next_index: chapters.len(),
        chapters,
        status: DownloadStatus::Complete,
        ..Default::default()
    };
    store.save_task(&task)?;
    Ok(task)
}

#[test]
fn summary_preserves_sql_length_null_empty_nul_blob_unicode_and_utf16_truth() -> Result<()> {
    for encoding in ["UTF-8", "UTF-16le", "UTF-16be"] {
        let (_directory, store, _) = fixture(encoding)?;
        for body in [
            None,
            Some(""),
            Some(" \n\u{3000}"),
            Some("\0tail"),
            Some("head\0tail"),
            Some("\u{4e2d}"),
            Some("\u{feff}\0tail"),
        ] {
            store
                .connection
                .execute("UPDATE chapters SET body=?1 WHERE chapter_index=0", [body])?;
            reset_body_length_read_stats();
            let summary = store.summary()?;
            let expected: u64 = store.connection.query_row("SELECT COUNT(*) FROM chapters WHERE body IS NOT NULL AND length(body)>0 AND content_status='full'", [], |row| row.get(0))?;
            assert_eq!(summary.cached_chapters, expected);
            assert_eq!(store.chapter_body_read_count(), 0);
            assert_eq!(store.book_read_stats().intro_copies, 0);
        }
        store
            .connection
            .execute("UPDATE chapters SET body=x'00' WHERE chapter_index=0", [])?;
        reset_body_length_read_stats();
        assert_eq!(store.summary()?.cached_chapters, 2);
        assert_eq!(body_length_read_stats().text_evaluations, 1);
        let large_bom = format!("\u{feff}{}", "x".repeat(4096));
        store.connection.execute(
            "UPDATE chapters SET body=?1 WHERE chapter_index=0",
            [&large_bom],
        )?;
        reset_body_length_read_stats();
        let expected: u64 = store.connection.query_row(
            "SELECT COUNT(*) FROM chapters WHERE length(body)>0 AND content_status='full'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(store.summary()?.cached_chapters, expected);
        assert_eq!(body_length_read_stats().text_evaluations, 2);
    }
    Ok(())
}

#[test]
fn shelf_and_duplicate_book_upsert_share_each_unique_body_evaluation() -> Result<()> {
    let (_directory, store, mut book) = fixture("UTF-8")?;
    book.title = "Fresh title".into();
    book.on_shelf = false;
    reset_body_length_read_stats();
    store.replace_shelf(&[book.clone(), book.clone()])?;
    assert_eq!(
        (
            body_length_read_stats().unique_rows,
            body_length_read_stats().text_evaluations
        ),
        (2, 2)
    );
    assert_eq!(store.chapter_body_read_count(), 0);
    assert_eq!(store.book_read_stats().intro_copies, 0);
    let stored = store.book("book")?.unwrap();
    assert_eq!(stored.intro, book.intro);
    assert_eq!(stored.title, "Fresh title");
    assert!(stored.on_shelf);
    assert_eq!(stored.cached_chapters, 2);
    reset_body_length_read_stats();
    store.upsert_books(&[book.clone(), book])?;
    assert_eq!(body_length_read_stats().text_evaluations, 2);
    Ok(())
}

#[test]
fn low_work_summary_and_shelf_refuse_before_first_text_evaluation_and_keep_rows() -> Result<()> {
    let (_directory, store, book) = fixture("UTF-8")?;
    let original = raw_bodies(&store)?;
    let before_book = store.book("book")?.unwrap();
    store.reset_book_read_stats();
    let _guard = HookGuard;
    HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(|| panic!("unadmitted body evaluation"))));
    with_body_length_test_limits(
        BodyLengthLimits {
            work_bytes: 1,
            ..Default::default()
        },
        || -> Result<()> {
            assert!(
                store
                    .summary()
                    .unwrap_err()
                    .downcast_ref::<CachedResourceLimit>()
                    .is_some()
            );
            assert!(
                store
                    .replace_shelf(&[book])
                    .unwrap_err()
                    .downcast_ref::<CachedResourceLimit>()
                    .is_some()
            );
            assert_eq!(body_length_read_stats().text_evaluations, 0);
            assert_eq!(store.chapter_body_read_count(), 0);
            assert_eq!(store.book_read_stats().intro_copies, 0);
            Ok(())
        },
    )?;
    assert_eq!(raw_bodies(&store)?, original);
    assert_eq!(store.book("book")?, Some(before_book));
    assert_eq!(store.annotations("book")?.len(), 1);
    assert_eq!(
        store.setting::<String>("length.account")?,
        Some("owner".into())
    );
    Ok(())
}

#[test]
fn complete_plan_exact_and_one_less_gate_all_unique_text_work() -> Result<()> {
    let (_directory, store, _) = fixture("UTF-8")?;
    let sql = format!("SELECT {BODY_LENGTH_FIELDS} FROM chapters ORDER BY rowid");
    let mut original = BodyLengthOperation::new(store.text_encoding);
    original.collect(&store.connection, &sql, [])?;
    let exact = BodyLengthLimits {
        rows: original.rows.len(),
        work_bytes: original.work,
        metadata_bytes: original.metadata,
        temporary_bytes: original.metadata + original.largest_temporary,
    };
    reset_body_length_read_stats();
    with_body_length_test_limits(exact, || -> Result<()> {
        let mut operation = BodyLengthOperation::new(store.text_encoding);
        operation.collect(&store.connection, &sql, [])?;
        operation.evaluate(&store.connection)?;
        assert_eq!(body_length_read_stats().text_evaluations, 2);
        Ok(())
    })?;
    for limits in [
        BodyLengthLimits {
            rows: exact.rows - 1,
            ..exact
        },
        BodyLengthLimits {
            work_bytes: exact.work_bytes - 1,
            ..exact
        },
        BodyLengthLimits {
            metadata_bytes: exact.metadata_bytes - 1,
            ..exact
        },
        BodyLengthLimits {
            temporary_bytes: exact.temporary_bytes - 1,
            ..exact
        },
    ] {
        reset_body_length_read_stats();
        let _guard = HookGuard;
        HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(|| panic!("unadmitted complete plan")))
        });
        with_body_length_test_limits(limits, || -> Result<()> {
            let mut operation = BodyLengthOperation::new(store.text_encoding);
            let result = operation
                .collect(&store.connection, &sql, [])
                .and_then(|_| operation.evaluate(&store.connection));
            assert!(
                result
                    .unwrap_err()
                    .downcast_ref::<CachedResourceLimit>()
                    .is_some()
            );
            assert_eq!(body_length_read_stats().text_evaluations, 0);
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn opening_overlapping_completed_tasks_uses_one_plan_and_preserves_valid_proof() -> Result<()> {
    let (directory, store, _) = fixture("UTF-8")?;
    let first = completed(&store, "first", vec![0], "owner")?;
    let second = completed(&store, "second", vec![0, 1], "owner")?;
    drop(store);
    reset_body_length_read_stats();
    let reopened = Store::open(directory.path().join("length.sqlite"))?;
    assert_eq!(body_length_read_stats().text_evaluations, 2);
    assert_eq!(reopened.task("first")?, Some(first));
    assert_eq!(reopened.task("second")?, Some(second));
    assert_eq!(reopened.chapter_body_read_count(), 0);
    Ok(())
}

#[test]
fn ownership_refresh_shares_count_and_complete_prefix_evidence_then_rolls_back_denial() -> Result<()>
{
    let (_directory, store, _) = fixture("UTF-8")?;
    store.connection.execute(
        "UPDATE chapters SET content_account_key='new-owner' WHERE chapter_index=1",
        [],
    )?;
    let task = completed(&store, "current", vec![1], "new-owner")?;
    reset_body_length_read_stats();
    store.invalidate_content_ownership(Some("new-owner"))?;
    assert_eq!(body_length_read_stats().text_evaluations, 1);
    assert_eq!(store.book("book")?.unwrap().cached_chapters, 1);
    assert_eq!(store.task("current")?, Some(task));
    let original = raw_bodies(&store)?;
    reset_body_length_read_stats();
    with_body_length_test_limits(
        BodyLengthLimits {
            work_bytes: 1,
            ..Default::default()
        },
        || -> Result<()> {
            assert!(
                store
                    .invalidate_content_ownership(Some("new-owner"))
                    .unwrap_err()
                    .downcast_ref::<CachedResourceLimit>()
                    .is_some()
            );
            assert_eq!(body_length_read_stats().text_evaluations, 0);
            Ok(())
        },
    )?;
    assert_eq!(raw_bodies(&store)?, original);
    Ok(())
}

#[test]
fn first_nul_completed_prefix_pauses_at_original_index_and_keeps_remaining_data() -> Result<()> {
    let (directory, store, _) = fixture("UTF-16le")?;
    let task = completed(&store, "completed", vec![0, 1], "owner")?;
    store.connection.execute(
        "UPDATE chapters SET body=?1 WHERE chapter_index=0",
        ["\0tail"],
    )?;
    let original = raw_bodies(&store)?;
    drop(store);
    reset_body_length_read_stats();
    let reopened = Store::open(directory.path().join("length.sqlite"))?;
    let paused = reopened.task(&task.id)?.unwrap();
    assert_eq!(
        (paused.status, paused.next_index),
        (DownloadStatus::Paused, 0)
    );
    assert_eq!(paused.account_key, task.account_key);
    assert_eq!(paused.chapters, task.chapters);
    assert_eq!(raw_bodies(&reopened)?, original);
    assert_eq!(reopened.annotations("book")?.len(), 1);
    Ok(())
}

#[test]
fn sqlite_write_failure_rolls_back_shelf_after_prepaid_count_without_body_copy() -> Result<()> {
    let (_directory, store, mut book) = fixture("UTF-8")?;
    let original = store.book("book")?.unwrap();
    store.reset_book_read_stats();
    reset_body_length_read_stats();
    store.connection.execute_batch(
        "CREATE TEMP TRIGGER reject_book BEFORE UPDATE ON books
        WHEN NEW.title='Rejected title' BEGIN SELECT RAISE(ABORT,'fixture shelf failure'); END",
    )?;
    book.title = "Rejected title".into();
    let error = store.replace_shelf(&[book]).unwrap_err();
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
    assert_eq!(body_length_read_stats().text_evaluations, 2);
    assert_eq!(store.chapter_body_read_count(), 0);
    assert_eq!(store.book("book")?, Some(original));
    Ok(())
}

#[test]
fn modern_download_checkpoint_keeps_delta_counter_without_legacy_recount() -> Result<()> {
    let (_directory, store, _) = fixture("UTF-8")?;
    store.upsert_chapter(&Chapter {
        id: "fresh".into(),
        book_id: "book".into(),
        index: 2,
        ..Default::default()
    })?;
    let task = DownloadTask {
        id: "delta".into(),
        book_id: "book".into(),
        account_key: "owner".into(),
        chapters: vec![2],
        status: DownloadStatus::Running,
        ..Default::default()
    };
    store.save_task(&task)?;
    let packet = Chapter {
        id: "fresh".into(),
        book_id: "book".into(),
        index: 2,
        body: Some("Fresh body".into()),
        cached: true,
        content_status: ContentStatus::Full,
        content_account_key: "owner".into(),
        ..Default::default()
    };
    reset_body_length_read_stats();
    assert!(store.commit_download_checkpoint(&task, &packet, 1, 10)?);
    assert_eq!(body_length_read_stats().text_evaluations, 0);
    assert_eq!(body_length_read_stats().metadata_rows, 0);
    let checkpoint = store.task("delta")?.unwrap();
    assert_eq!(
        (
            checkpoint.status,
            checkpoint.next_index,
            checkpoint.updated_at
        ),
        (DownloadStatus::Running, 1, 10)
    );
    assert_eq!(store.book("book")?.unwrap().cached_chapters, 3);
    assert_eq!(store.chapter("book", 2)?, Some(packet));
    // The worker's Finished event performs this separately fenced transition;
    // a durable packet cursor does not itself certify terminal delivery.
    let mut finished = checkpoint.clone();
    finished.status = DownloadStatus::Complete;
    finished.updated_at = 11;
    reset_body_length_read_stats();
    assert!(store.save_task_if_current(&checkpoint, &finished)?);
    assert_eq!(store.task("delta")?, Some(finished));
    assert_eq!(body_length_read_stats().unique_rows, 1);
    assert_eq!(body_length_read_stats().text_evaluations, 1);
    Ok(())
}

#[test]
fn legacy_utf16_migration_keeps_body_bytes_and_original_nonempty_classification() -> Result<()> {
    let (directory, store, _) = fixture("UTF-16le")?;
    store
        .connection
        .execute("UPDATE books SET source='local' WHERE id='book'", [])?;
    store.connection.execute(
        "UPDATE chapters SET body=?1 WHERE chapter_index=0",
        ["\0tail"],
    )?;
    store.connection.execute(
        "UPDATE chapters SET body=?1 WHERE chapter_index=1",
        ["\u{3000} "],
    )?;
    let mut statement = store
        .connection
        .prepare("SELECT body FROM chapters ORDER BY chapter_index")?;
    let original = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    // Recreate the actual version-one column boundary in this private fixture.
    store.connection.execute_batch(
        "ALTER TABLE chapters DROP COLUMN content_account_key;
        ALTER TABLE chapters DROP COLUMN content_status; PRAGMA user_version=1",
    )?;
    drop(store);
    reset_body_length_read_stats();
    let reopened = Store::open(directory.path().join("length.sqlite"))?;
    assert_eq!(body_length_read_stats().text_evaluations, 2);
    assert_eq!(reopened.book("book")?.unwrap().cached_chapters, 1);
    let mut statement = reopened
        .connection
        .prepare("SELECT body,content_status FROM chapters ORDER BY chapter_index")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(
        rows.iter().map(|row| row.0.clone()).collect::<Vec<_>>(),
        original
    );
    assert_eq!(
        (rows[0].1.as_str(), rows[1].1.as_str()),
        ("unknown", "full")
    );
    assert_eq!(reopened.annotations("book")?.len(), 1);
    assert_eq!(
        reopened.setting::<String>("length.account")?,
        Some("owner".into())
    );
    Ok(())
}
