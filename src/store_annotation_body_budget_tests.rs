//! Annotation bounds use admitted borrowed Rust Unicode rather than SQL length.

use super::*;
use std::cell::Cell;

thread_local! { static BORROWS: Cell<usize> = const { Cell::new(0) }; }
pub(super) fn observe_body_borrow(_: usize) {
    BORROWS.with(|slot| slot.set(slot.get() + 1));
}
pub(super) fn borrowed_count() -> usize {
    BORROWS.with(Cell::get)
}
fn reset() {
    BORROWS.with(|slot| slot.set(0));
}

fn fixture() -> Result<Store> {
    let store = Store::open(":memory:")?;
    store.upsert_book(&Book {
        source: "local".into(),
        intro: "Keep intro".into(),
        ..Book::new("book", "Book")
    })?;
    store.upsert_chapter(&Chapter {
        id: "uid".into(),
        book_id: "book".into(),
        body: Some("A\0\u{4e2d}Z".into()),
        ..Default::default()
    })?;
    store.set_setting("annotation.body.fixture.account", &"Keep account")?;
    reset();
    Ok(store)
}

fn note(end: u32) -> Annotation {
    Annotation {
        id: "note".into(),
        book_id: "book".into(),
        start: 0,
        end,
        quote: "A".into(),
        note: "Keep".into(),
        ..Default::default()
    }
}

#[test]
fn annotation_borrowed_count_includes_nul_and_unicode_without_owned_body() -> Result<()> {
    let store = fixture()?;
    store.save_annotation(&note(4))?;
    assert_eq!(store.chapter_body_read_count(), 0);
    assert_eq!(borrowed_count(), 1);
    assert!(store.save_annotation(&note(5)).is_err());
    assert_eq!(store.annotations("book")?[0].end, 4);
    Ok(())
}

#[test]
fn annotation_body_quota_refuses_before_borrow_and_keeps_previous_note() -> Result<()> {
    let store = fixture()?;
    store.save_annotation(&note(4))?;
    reset();
    with_store_inventory_test_limits(
        StoreInventoryLimits {
            work_bytes: 1,
            ..Default::default()
        },
        || -> Result<()> {
            assert!(
                store
                    .save_annotation(&note(3))
                    .unwrap_err()
                    .downcast_ref::<StoreInventoryLimit>()
                    .is_some()
            );
            assert_eq!(borrowed_count(), 0);
            Ok(())
        },
    )?;
    assert_eq!(store.annotations("book")?, vec![note(4)]);
    assert_eq!(
        store.chapter_body("book", 0)?.as_deref(),
        Some("A\0\u{4e2d}Z")
    );
    assert_eq!(
        store
            .setting::<String>("annotation.body.fixture.account")?
            .as_deref(),
        Some("Keep account")
    );
    Ok(())
}

#[test]
fn annotation_utf16_uses_rust_count_and_retains_original_body() -> Result<()> {
    for encoding in ["UTF-16le", "UTF-16be"] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("db");
        {
            let mut connection = Connection::open(&path)?;
            connection.pragma_update(None, "encoding", encoding)?;
            initialize_schema(&mut connection)?;
        }
        let store = Store::open(&path)?;
        store.upsert_book(&Book {
            source: "local".into(),
            ..Book::new("book", "Book")
        })?;
        store.upsert_chapter(&Chapter {
            id: "uid".into(),
            book_id: "book".into(),
            body: Some("\u{feff}\0\u{4e2d}".into()),
            ..Default::default()
        })?;
        let original = store.chapter_body("book", 0)?.unwrap();
        let count = u32::try_from(original.chars().count())?;
        let before = store.chapter_body_read_count();
        store.save_annotation(&note(count))?;
        assert_eq!(store.chapter_body_read_count(), before);
        assert_eq!(store.annotations("book")?[0].end, count);
        assert_eq!(
            store.chapter_body("book", 0)?.as_deref(),
            Some(original.as_str())
        );
    }
    Ok(())
}

#[test]
fn annotation_body_exact_work_and_scratch_are_prepaid_before_borrow() -> Result<()> {
    let store = fixture()?;
    store.save_annotation(&note(4))?;
    let bytes = "A\0\u{4e2d}Z".len();
    for (limits, resource) in [
        (
            StoreInventoryLimits {
                work_bytes: 2 * bytes - 1,
                ..Default::default()
            },
            "annotation body work",
        ),
        (
            StoreInventoryLimits {
                scratch_bytes: bytes + 1,
                ..Default::default()
            },
            "annotation body scratch",
        ),
    ] {
        reset();
        with_store_inventory_test_limits(limits, || -> Result<()> {
            let error = store.save_annotation(&note(3)).unwrap_err();
            assert_eq!(
                error
                    .downcast_ref::<StoreInventoryLimit>()
                    .unwrap()
                    .resource,
                resource
            );
            assert_eq!(borrowed_count(), 0);
            assert_eq!(store.chapter_body_read_count(), 0);
            Ok(())
        })?;
    }
    with_store_inventory_test_limits(
        StoreInventoryLimits {
            work_bytes: 2 * bytes,
            scratch_bytes: bytes + 2,
            ..Default::default()
        },
        || -> Result<()> {
            store.save_annotation(&note(4))?;
            assert_eq!(borrowed_count(), 1);
            assert_eq!(store.chapter_body_read_count(), 0);
            Ok(())
        },
    )?;
    assert_eq!(store.annotations("book")?, vec![note(4)]);
    assert_eq!(store.book("book")?.unwrap().intro, "Keep intro");
    Ok(())
}

#[test]
fn annotation_caller_transaction_and_sql_failure_keep_previous_note() -> Result<()> {
    let store = fixture()?;
    store.save_annotation(&note(4))?;
    let transaction = store.connection.unchecked_transaction()?;
    store.save_annotation(&note(3))?;
    drop(transaction);
    assert_eq!(store.annotations("book")?, vec![note(4)]);
    store.connection.execute_batch("CREATE TRIGGER deny_annotation_update BEFORE UPDATE OF note ON annotations WHEN NEW.note='Reject' BEGIN SELECT RAISE(ABORT,'fixture annotation denied'); END")?;
    let mut replacement = note(3);
    replacement.note = "Reject".into();
    reset();
    let error = store.save_annotation(&replacement).unwrap_err();
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
    assert_eq!(borrowed_count(), 1);
    assert_eq!(store.chapter_body_read_count(), 0);
    assert_eq!(store.annotations("book")?, vec![note(4)]);
    assert_eq!(
        store.chapter_body("book", 0)?.as_deref(),
        Some("A\0\u{4e2d}Z")
    );
    assert_eq!(
        store
            .setting::<String>("annotation.body.fixture.account")?
            .as_deref(),
        Some("Keep account")
    );
    Ok(())
}
