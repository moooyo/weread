//! Catalog-body conservation and bounded cached-view reads in one Store.

use super::*;
use serde::Deserialize;

fn store() -> Result<Store> {
    let store = Store::open(":memory:")?;
    let mut book = Book::new("resource-book", "Resource fixture book");
    book.source = "local".into();
    store.upsert_book(&book)?;
    Ok(store)
}

fn chapter(index: u32, uid: &str, body: Option<&str>) -> Chapter {
    Chapter {
        id: uid.into(),
        book_id: "resource-book".into(),
        index,
        title: format!("Title {uid}"),
        body: body.map(str::to_owned),
        cached: body.is_some(),
        content_status: if body.is_some() {
            ContentStatus::Full
        } else {
            ContentStatus::Unknown
        },
        ..Default::default()
    }
}

#[test]
fn catalog_reindex_preserves_large_sql_bodies_without_rust_materialization() -> Result<()> {
    let store = store()?;
    let body = "x".repeat(256 * 1024);
    for index in 0..4 {
        store.upsert_chapter(&chapter(index, &format!("uid-{index}"), Some(&body)))?;
    }
    store.save_position(&ReadingPosition {
        book_id: "resource-book".into(),
        chapter_index: 0,
        char_offset: 4,
        ..Default::default()
    })?;
    store.save_annotation(&Annotation {
        id: "note".into(),
        book_id: "resource-book".into(),
        chapter_index: 0,
        start: 0,
        end: 1,
        quote: "x".into(),
        note: "Keep local note".into(),
        created_at: 0,
    })?;
    let task = DownloadTask {
        id: "resource-task".into(),
        book_id: "resource-book".into(),
        chapters: vec![0, 1, 2, 3],
        status: DownloadStatus::Running,
        next_index: 1,
        ..Default::default()
    };
    store.save_task(&task)?;
    let headers = store.chapter_headers("resource-book")?;
    assert_eq!(headers.len(), 4);
    assert!(
        headers
            .iter()
            .all(|header| header.body.is_none() && header.cached)
    );
    let replacement = [
        chapter(0, "uid-2", None),
        chapter(1, "uid-1", None),
        chapter(2, "uid-0", None),
        chapter(3, "uid-3", None),
    ];
    store.replace_catalog("resource-book", &replacement)?;
    assert_eq!(store.materialized_chapter_bytes(), 0);
    for index in 0..4 {
        assert_eq!(
            store.chapter_body("resource-book", index)?.as_deref(),
            Some(body.as_str())
        );
    }
    assert_eq!(store.position("resource-book")?.unwrap().chapter_index, 2);
    assert_eq!(store.annotations("resource-book")?[0].chapter_index, 2);
    assert_eq!(store.task("resource-task")?.unwrap().chapters, [2, 1, 0, 3]);
    assert_eq!(store.book("resource-book")?.unwrap().cached_chapters, 4);
    assert!(
        store
            .chapter_metadata("resource-book")?
            .iter()
            .all(|chapter| chapter.body.is_none() && !chapter.title.is_empty())
    );
    Ok(())
}

#[test]
fn catalog_sql_reindex_failure_rolls_back_bodies_permissions_and_mapping() -> Result<()> {
    let store = store()?;
    store.upsert_chapter(&chapter(0, "uid-a", Some("Body A")))?;
    store.upsert_chapter(&chapter(1, "uid-b", Some("Body B")))?;
    store.save_position(&ReadingPosition {
        book_id: "resource-book".into(),
        chapter_index: 0,
        char_offset: 2,
        ..Default::default()
    })?;
    store.connection.execute_batch(
        "CREATE TRIGGER refuse_catalog_reindex BEFORE UPDATE OF chapter_index ON chapters
         WHEN NEW.chapter_index=0 AND OLD.chapter_index>=4294967296
         BEGIN SELECT RAISE(FAIL, 'synthetic catalog reindex failure'); END;",
    )?;
    assert!(
        store
            .replace_catalog(
                "resource-book",
                &[chapter(0, "uid-b", None), chapter(1, "uid-a", None)]
            )
            .is_err()
    );
    assert_eq!(store.materialized_chapter_bytes(), 0);
    assert_eq!(
        store.chapter_body("resource-book", 0)?.as_deref(),
        Some("Body A")
    );
    assert_eq!(
        store.chapter_body("resource-book", 1)?.as_deref(),
        Some("Body B")
    );
    assert_eq!(store.position("resource-book")?.unwrap().chapter_index, 0);
    assert_eq!(store.book("resource-book")?.unwrap().cached_chapters, 2);
    let remaining: i64 = store.connection.query_row(
        "SELECT COUNT(*) FROM sqlite_temp_master WHERE name='weread_catalog_map'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(remaining, 0);
    Ok(())
}

#[test]
fn duplicate_catalog_rejection_precedes_any_old_body_read() -> Result<()> {
    let store = store()?;
    store.upsert_chapter(&chapter(0, "uid", Some("Preserved body")))?;
    assert!(
        store
            .replace_catalog(
                "resource-book",
                &[chapter(0, "uid", None), chapter(1, "uid", None)]
            )
            .is_err()
    );
    assert_eq!(store.materialized_chapter_bytes(), 0);
    assert_eq!(
        store.chapter_body("resource-book", 0)?.as_deref(),
        Some("Preserved body")
    );
    Ok(())
}

struct NeverDeserialize;
impl<'de> Deserialize<'de> for NeverDeserialize {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> std::result::Result<Self, D::Error> {
        panic!("A denied cached record must not reach serde")
    }
}

#[test]
fn thought_cache_bytes_type_and_array_count_are_gated_before_serde() -> Result<()> {
    let store = store()?;
    store.connection.execute(
        "INSERT INTO settings(key,value) VALUES ('oversized','[null,null]')",
        [],
    )?;
    let error = store
        .bounded_array_setting::<NeverDeserialize>("oversized", 4, 10)
        .err()
        .unwrap();
    assert!(error.downcast_ref::<CachedResourceLimit>().is_some());
    let error = store
        .bounded_array_setting::<NeverDeserialize>("oversized", 100, 1)
        .err()
        .unwrap();
    assert!(error.downcast_ref::<CachedResourceLimit>().is_some());
    store
        .connection
        .execute("INSERT INTO settings(key,value) VALUES ('blob',x'ff')", [])?;
    assert!(
        store
            .bounded_setting::<String>("blob", 10)
            .err()
            .unwrap()
            .downcast_ref::<CachedResourceLimit>()
            .is_some()
    );
    assert_eq!(
        store
            .setting::<Vec<Option<u8>>>("oversized")?
            .unwrap()
            .len(),
        2
    );
    let kind: String = store.connection.query_row(
        "SELECT typeof(value) FROM settings WHERE key='blob'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(kind, "blob");
    Ok(())
}

#[test]
fn thought_cache_exact_encoded_boundaries_and_unicode_scalar_metadata_hold() -> Result<()> {
    let store = store()?;
    store.set_setting("empty-array", &Vec::<u8>::new())?;
    assert_eq!(
        store.bounded_array_setting::<u8>("empty-array", 2, 0)?,
        Some(Vec::new())
    );
    assert!(
        store
            .bounded_array_setting::<u8>("empty-array", 1, 0)
            .is_err()
    );
    let text = "\u{4e2d}";
    store.set_setting("scalar", &text)?;
    assert_eq!(
        store.bounded_setting::<String>("scalar", 5)?.as_deref(),
        Some(text)
    );
    assert!(store.bounded_setting::<String>("scalar", 4).is_err());
    store.upsert_chapter(&chapter(0, "uid", Some("A\u{1f600}\u{4e2d}")))?;
    assert_eq!(store.chapter_scalar_count("resource-book", 0)?, 3);
    Ok(())
}

#[test]
fn catalog_metadata_small_allowance_prices_old_owner_copies_before_transaction() -> Result<()> {
    let store = store()?;
    let mut old = chapter(0, "uid", Some("Old body"));
    old.content_account_key = "old-owner".into();
    store.upsert_chapter(&old)?;
    let incoming = [chapter(0, "uid", None)];
    let base = catalog_input_metadata_bytes("resource-book", &incoming, usize::MAX)?;
    assert!(catalog_input_metadata_bytes("resource-book", &incoming, base - 1).is_err());
    let fee = base + 2 * old.content_account_key.len();
    validate_catalog_capacity_on(&store.connection, "resource-book", base, fee)?;
    let error = validate_catalog_capacity_on(&store.connection, "resource-book", base, fee - 1)
        .unwrap_err();
    assert!(error.downcast_ref::<CachedResourceLimit>().is_some());
    assert!(store.connection.is_autocommit());
    assert_eq!(store.materialized_chapter_bytes(), 0);
    assert_eq!(
        store.chapter_body("resource-book", 0)?.as_deref(),
        Some("Old body")
    );
    Ok(())
}

#[test]
fn catalog_direct_long_owner_refusal_precedes_headers_and_mutation() -> Result<()> {
    let store = Store::open(":memory:")?;
    let owner = "b".repeat(4097);
    store.upsert_book(&Book::new(&owner, "Long owner fixture"))?;
    store.upsert_chapter(&Chapter {
        id: "uid".into(),
        book_id: owner.clone(),
        index: 0,
        body: Some("Preserved body".into()),
        content_status: ContentStatus::Full,
        ..Default::default()
    })?;
    // Reading this UID into Rust would fail. Owner/metadata refusal happens
    // before consulting any old header or starting the catalog transaction.
    store.connection.execute(
        "UPDATE chapters SET source_id=x'ff' WHERE book_id=?1",
        [&owner],
    )?;
    let incoming = [Chapter {
        id: "uid".into(),
        book_id: owner.clone(),
        index: 0,
        title: "New".into(),
        ..Default::default()
    }];
    let error = store.replace_catalog(&owner, &incoming).unwrap_err();
    assert!(error.downcast_ref::<CachedResourceLimit>().is_some());
    assert!(store.connection.is_autocommit());
    assert_eq!(store.chapter_header_read_count(), 0);
    assert_eq!(store.materialized_chapter_bytes(), 0);
    assert_eq!(
        store.chapter_body(&owner, 0)?.as_deref(),
        Some("Preserved body")
    );
    Ok(())
}
