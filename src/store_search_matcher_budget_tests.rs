//! Whole-string Unicode lowercase semantics and reached-field work admission.

use super::*;
use std::cell::Cell;

thread_local! { static TRANSFORMS: Cell<usize> = const { Cell::new(0) }; }
pub(super) fn observe_transform(_: usize) {
    TRANSFORMS.with(|slot| slot.set(slot.get() + 1));
}
fn reset() {
    TRANSFORMS.with(|slot| slot.set(0));
}

fn fixture() -> Result<Store> {
    let store = Store::open(":memory:")?;
    store.upsert_book(&Book {
        title: "\u{130} \u{391}\u{3a3}".into(),
        author: "Author".into(),
        category: "Category".into(),
        intro: "Keep intro".into(),
        source: "local".into(),
        ..Book::new("book", "")
    })?;
    reset();
    Ok(store)
}

#[test]
fn search_retains_contextual_sigma_and_expanding_unicode_lowercase() -> Result<()> {
    let store = fixture()?;
    assert_eq!(store.search_book_metadata("\u{3b1}\u{3c2}", true)?.len(), 1);
    assert_eq!(TRANSFORMS.with(Cell::get), 1);
    reset();
    assert_eq!(store.search_book_metadata("i\u{307}", true)?.len(), 1);
    assert_eq!(TRANSFORMS.with(Cell::get), 1);
    assert_eq!(store.book_read_stats().intro_copies, 0);
    Ok(())
}

#[test]
fn empty_search_and_title_short_circuit_do_not_transform_other_fields() -> Result<()> {
    let store = fixture()?;
    assert_eq!(store.search_book_metadata("  ", true)?.len(), 1);
    assert_eq!(TRANSFORMS.with(Cell::get), 0);
    assert_eq!(store.search_book_metadata("author", true)?.len(), 1);
    assert_eq!(TRANSFORMS.with(Cell::get), 2);
    Ok(())
}

#[test]
fn search_quota_refuses_before_transform_and_preserves_book_data() -> Result<()> {
    let store = fixture()?;
    with_store_inventory_test_limits(
        StoreInventoryLimits {
            work_bytes: 1,
            ..Default::default()
        },
        || -> Result<()> {
            assert!(
                store
                    .search_book_metadata("no match", true)
                    .unwrap_err()
                    .downcast_ref::<StoreInventoryLimit>()
                    .is_some()
            );
            assert_eq!(TRANSFORMS.with(Cell::get), 0);
            assert_eq!(store.book_read_stats().metadata_rows, 0);
            Ok(())
        },
    )?;
    assert_eq!(store.book("book")?.unwrap().intro, "Keep intro");
    Ok(())
}

#[test]
fn search_work_is_cumulative_across_exact_reached_fields() -> Result<()> {
    let store = fixture()?;
    let query = "author";
    let exact = 2 * query.len()
        + 8 * query.len()
        + 8 * "\u{130} \u{391}\u{3a3}".len()
        + query.len()
        + 8 * "Author".len()
        + query.len();
    with_store_inventory_test_limits(
        StoreInventoryLimits {
            work_bytes: exact - 1,
            ..Default::default()
        },
        || -> Result<()> {
            let error = store.search_book_metadata(query, true).unwrap_err();
            assert_eq!(
                error
                    .downcast_ref::<StoreInventoryLimit>()
                    .unwrap()
                    .resource,
                "search work"
            );
            assert_eq!(TRANSFORMS.with(Cell::get), 1);
            Ok(())
        },
    )?;
    reset();
    with_store_inventory_test_limits(
        StoreInventoryLimits {
            work_bytes: exact,
            ..Default::default()
        },
        || -> Result<()> {
            let result = store.search_book_metadata(query, true)?;
            assert_eq!(result.len(), 1);
            assert_eq!(result[0].id, "book");
            assert_eq!(TRANSFORMS.with(Cell::get), 2);
            Ok(())
        },
    )?;
    assert_eq!(store.book_read_stats().intro_copies, 0);
    assert_eq!(store.book("book")?.unwrap().intro, "Keep intro");
    Ok(())
}

#[test]
fn search_reserves_live_query_before_any_metadata_copy() -> Result<()> {
    let store = fixture()?;
    let needed = usize::try_from(store.connection.query_row("SELECT COALESCE(octet_length(id),0)+COALESCE(octet_length(title),0)+COALESCE(octet_length(author),0)+COALESCE(octet_length(category),0)+COALESCE(octet_length(cover_url),0)+COALESCE(octet_length(source),0) FROM books WHERE id='book'", [], |row| row.get::<_, i64>(0))?)?;
    let metadata = needed
        + 2 * (std::mem::size_of::<BookMetadata>() + std::mem::size_of::<BookScalarMetadata>());
    let query = "q".repeat(512);
    let combined = metadata + 6 * query.len();
    with_store_inventory_test_limits(
        StoreInventoryLimits {
            scratch_bytes: combined - 1,
            ..Default::default()
        },
        || -> Result<()> {
            let error = store.search_book_metadata(&query, true).unwrap_err();
            assert_eq!(
                error
                    .downcast_ref::<StoreInventoryLimit>()
                    .unwrap()
                    .resource,
                "search metadata and query scratch"
            );
            assert_eq!(store.book_read_stats().metadata_rows, 0);
            assert_eq!(store.book_read_stats().intro_copies, 0);
            assert_eq!(TRANSFORMS.with(Cell::get), 0);
            Ok(())
        },
    )?;
    assert_eq!(store.book("book")?.unwrap().intro, "Keep intro");
    Ok(())
}
