//! Scalar display lookups never materialize cached chapter payloads.

use super::*;

fn book(store: &Store, id: &str, source: &str) -> Result<()> {
    store.upsert_book(&Book {
        source: source.into(),
        ..Book::new(id, "Display metadata fixture")
    })
}

fn chapter(
    store: &Store,
    book_id: &str,
    uid: &str,
    index: u32,
    body: Option<String>,
) -> Result<()> {
    store.upsert_chapter(&Chapter {
        id: uid.into(),
        book_id: book_id.into(),
        index,
        body,
        title: "Display metadata chapter".into(),
        ..Default::default()
    })
}

#[test]
fn unique_uid_lookup_is_scalar_and_ambiguity_fails_before_body_reads() -> Result<()> {
    let store = Store::open(":memory:")?;
    book(&store, "uid-book", "local")?;
    for index in 0..4 {
        chapter(
            &store,
            "uid-book",
            &format!("uid-{index}"),
            index,
            Some("Hidden cached body.\n".repeat(512)),
        )?;
    }
    assert_eq!(store.chapter_index_by_uid("uid-book", "uid-2")?, Some(2));
    assert_eq!(store.chapter_index_by_uid("uid-book", "missing")?, None);
    book(&store, "other-book", "local")?;
    chapter(
        &store,
        "other-book",
        "uid-2",
        0,
        Some("Another book's body".into()),
    )?;
    assert_eq!(store.chapter_index_by_uid("uid-book", "uid-2")?, Some(2));
    chapter(
        &store,
        "uid-book",
        "uid-2",
        4,
        Some("A duplicate UID's body".into()),
    )?;
    assert_eq!(store.chapter_index_by_uid("uid-book", "uid-2")?, None);
    assert_eq!(store.materialized_chapter_bytes(), 0);
    assert_eq!(store.chapter_body_read_count(), 0);
    assert!(
        (0..5).all(|index| store.chapter_body_materialization_stats("uid-book", index) == (0, 0))
    );
    Ok(())
}

#[test]
fn uid_lookup_bounds_identifiers_and_keeps_invalid_index_rows_ambiguous() -> Result<()> {
    let store = Store::open(":memory:")?;
    book(&store, "index-book", "local")?;
    chapter(
        &store,
        "index-book",
        "invalid-index",
        0,
        Some("Preserved body".into()),
    )?;
    assert!(
        store
            .chapter_index_by_uid(&"b".repeat(4097), "uid")
            .is_err()
    );
    assert!(
        store
            .chapter_index_by_uid("index-book", &"u".repeat(4097))
            .is_err()
    );
    assert!(
        store
            .chapter_index_by_uid("index-book", "invalid\0uid")
            .is_err()
    );
    assert!(store.chapter_index_by_uid("index-book", "").is_err());
    store.connection.execute(
        "UPDATE chapters SET chapter_index=?1 WHERE book_id='index-book'",
        [i64::from(u32::MAX) + 1],
    )?;
    assert_eq!(
        store.chapter_index_by_uid("index-book", "invalid-index")?,
        None
    );
    // A valid duplicate cannot hide an invalid matching row through a WHERE
    // filter. Both records must still make the UID unavailable.
    chapter(
        &store,
        "index-book",
        "invalid-index",
        2,
        Some("Another preserved body".into()),
    )?;
    assert_eq!(
        store.chapter_index_by_uid("index-book", "invalid-index")?,
        None
    );
    assert_eq!(store.materialized_chapter_bytes(), 0);
    assert_eq!(store.chapter_body_read_count(), 0);
    Ok(())
}

#[test]
fn display_presence_preserves_empty_text_without_permission_or_payload_reads() -> Result<()> {
    let store = Store::open(":memory:")?;
    book(&store, "presence-book", "weread")?;
    store.upsert_chapter(&Chapter {
        id: "preview".into(),
        book_id: "presence-book".into(),
        index: 0,
        body: Some("Present preview under another owner".into()),
        content_status: ContentStatus::Preview,
        content_account_key: "other-owner".into(),
        ..Default::default()
    })?;
    for index in 1..5 {
        chapter(
            &store,
            "presence-book",
            &format!("presence-{index}"),
            index,
            None,
        )?;
    }
    // Legacy TEXT can be empty or blank even though new cache writes reject it.
    // Preserve the old Option<String>::is_some display contract for those rows.
    store.connection.execute(
        "UPDATE chapters SET body='' WHERE book_id='presence-book' AND chapter_index=2",
        [],
    )?;
    store.connection.execute(
        "UPDATE chapters SET body=?1 WHERE book_id='presence-book' AND chapter_index=3",
        ["\u{3000} \n"],
    )?;
    assert!(store.chapter_body_present("presence-book", 0)?);
    assert!(!store.chapter_body_present("presence-book", 1)?);
    assert!(store.chapter_body_present("presence-book", 2)?);
    assert!(store.chapter_body_present("presence-book", 3)?);
    assert!(!store.chapter_body_present("presence-book", 99)?);
    assert!(!store.chapter_body_present("missing-book", 0)?);
    let metadata = store.chapter_metadata("presence-book")?;
    assert_eq!(metadata[0].content_status, ContentStatus::Preview);
    assert_eq!(metadata[0].content_account_key, "other-owner");
    assert!(!metadata[0].cached);
    // Invalid non-TEXT storage fails closed, without copying its payload.
    store.connection.execute(
        "UPDATE chapters SET body=X'4142' WHERE book_id='presence-book' AND chapter_index=4",
        [],
    )?;
    assert!(store.chapter_body_present("presence-book", 4).is_err());
    assert_eq!(store.materialized_chapter_bytes(), 0);
    assert_eq!(store.chapter_body_read_count(), 0);
    assert!(
        (0..5).all(
            |index| store.chapter_body_materialization_stats("presence-book", index) == (0, 0)
        )
    );
    Ok(())
}
