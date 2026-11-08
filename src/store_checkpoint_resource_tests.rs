//! Bodyless checkpoint authentication and cached-array admission in real SQL.

use super::*;
use serde::{Deserialize, Deserializer};
use serde_json::json;

const BOOK: &str = "cached-checkpoint-book";
const OWNER: &str = "synthetic-checkpoint-owner";

fn fixture() -> Result<(Store, DownloadTask)> {
    let store = Store::open(":memory:")?;
    let mut book = Book::new(BOOK, "Cached checkpoint fixture");
    book.source = "weread".into();
    store.upsert_book(&book)?;
    for index in 0..2 {
        store.upsert_chapter(&Chapter {
            id: format!("uid-{index}"),
            book_id: BOOK.into(),
            index,
            title: format!("Chapter {index}"),
            body: Some(format!("Readable chapter {index}")),
            cached: true,
            content_status: ContentStatus::Full,
            content_account_key: OWNER.into(),
        })?;
    }
    let task = DownloadTask {
        id: "cached-task".into(),
        book_id: BOOK.into(),
        account_key: OWNER.into(),
        chapters: vec![0, 1],
        status: DownloadStatus::Running,
        include_images: false,
        updated_at: 100,
        ..Default::default()
    };
    store.save_task(&task)?;
    Ok((store, task))
}

#[test]
fn cached_checkpoint_advances_without_copying_body_or_incrementing_count() -> Result<()> {
    let (store, mut task) = fixture()?;
    let body = "Readable cached corpus. ".repeat(4096);
    for index in 0..12 {
        store.upsert_chapter(&Chapter {
            id: format!("uid-{index}"),
            book_id: BOOK.into(),
            index,
            title: "Corpus chapter".into(),
            body: Some(body.clone()),
            cached: true,
            content_status: ContentStatus::Full,
            content_account_key: OWNER.into(),
        })?;
    }
    task.chapters = (0..12).collect();
    store.save_task(&task)?;
    let count = store.book(BOOK)?.unwrap().cached_chapters;
    for index in 0..12 {
        store.commit_cached_download_checkpoint(
            &task,
            &format!("uid-{index}"),
            index,
            index as usize + 1,
            OWNER,
            101 + index as i64,
        )?;
        task.next_index += 1;
        task.updated_at = 101 + index as i64;
        assert_eq!(store.task(&task.id)?.unwrap(), task);
        assert_eq!(store.chapter_body_read_count(), 0);
        assert_eq!(store.materialized_chapter_bytes(), 0);
        assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, count);
    }
    assert_eq!(
        store.chapter_body(BOOK, 11)?.as_deref(),
        Some(body.as_str())
    );
    Ok(())
}

#[test]
fn cached_checkpoint_refuses_unreadable_or_unverified_sql_content() -> Result<()> {
    use rusqlite::types::Value;
    let cases = [
        (Value::Null, "full", OWNER),
        (Value::Text(String::new()), "full", OWNER),
        (
            Value::Text(" \t\n\r\u{85}\u{a0}\u{1680}\u{2000}\u{2028}\u{202f}\u{3000}".into()),
            "full",
            OWNER,
        ),
        (Value::Blob(b"Readable blob".to_vec()), "full", OWNER),
        (Value::Text("Readable body".into()), "unknown", OWNER),
        (Value::Text("Readable excerpt".into()), "preview", OWNER),
        (Value::Text("Retained body".into()), "locked", OWNER),
        (Value::Text("Readable body".into()), "full", "other-owner"),
    ];
    for (body, status, owner) in cases {
        let (store, task) = fixture()?;
        store.connection.execute(
            "UPDATE chapters SET body=?2,content_status=?3,content_account_key=?4
             WHERE book_id=?1 AND chapter_index=0",
            params![BOOK, body, status, owner],
        )?;
        let count = store.book(BOOK)?.unwrap().cached_chapters;
        assert!(
            store
                .commit_cached_download_checkpoint(&task, "uid-0", 0, 1, OWNER, 101)
                .is_err()
        );
        assert_eq!(store.task(&task.id)?.unwrap(), task);
        assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, count);
        let saved: Value = store.connection.query_row(
            "SELECT body FROM chapters WHERE book_id=?1 AND chapter_index=0",
            [BOOK],
            |row| row.get(0),
        )?;
        assert_eq!(saved, body);
        assert_eq!(store.chapter_body_read_count(), 0);
    }
    Ok(())
}

#[test]
fn cached_checkpoint_requires_the_exact_durable_task_scope() -> Result<()> {
    for mutation in 0..9 {
        let (store, task) = fixture()?;
        let mut changed = task.clone();
        match mutation {
            0 => changed.status = DownloadStatus::Cancelled,
            1 => changed.account_key = "other-owner".into(),
            2 => changed.chapters.reverse(),
            3 => changed.next_index = 1,
            4 => changed.include_images = true,
            5 => changed.updated_at += 1,
            6 => changed.error = Some("synthetic-error".into()),
            7 => changed.id = "other-task".into(),
            8 => changed.book_id = "missing-book".into(),
            _ => unreachable!(),
        }
        if mutation == 8 {
            store.upsert_book(&Book::new(&changed.book_id, "Other book"))?;
        }
        store.connection.execute(
            "UPDATE download_tasks SET book_id=?2,status=?3,updated_at=?4,payload=?5 WHERE id=?1",
            params![
                task.id,
                changed.book_id,
                changed.status.as_str(),
                changed.updated_at,
                serde_json::to_string(&changed)?
            ],
        )?;
        assert!(
            store
                .commit_cached_download_checkpoint(&task, "uid-0", 0, 1, OWNER, 101)
                .is_err()
        );
        assert_eq!(store.task(&task.id)?.unwrap(), changed);
        assert_eq!(store.chapter_body_read_count(), 0);
        assert_eq!(
            store.chapter_body(BOOK, 0)?.as_deref(),
            Some("Readable chapter 0")
        );
    }
    Ok(())
}

#[test]
fn cached_checkpoint_refuses_changed_uid_account_or_cursor_before_advance() -> Result<()> {
    let (store, task) = fixture()?;
    for (uid, index, next, owner) in [
        ("other-uid", 0, 1, OWNER),
        ("uid-0", 1, 1, OWNER),
        ("uid-0", 0, 0, OWNER),
        ("uid-0", 0, 2, OWNER),
        ("uid-0", 0, 1, "other-owner"),
        ("uid-0", 0, 1, ""),
    ] {
        assert!(
            store
                .commit_cached_download_checkpoint(&task, uid, index, next, owner, 101)
                .is_err()
        );
        assert_eq!(store.task(&task.id)?.unwrap(), task);
    }
    store.connection.execute(
        "UPDATE chapters SET source_id='replacement-uid' WHERE book_id=?1 AND chapter_index=0",
        [BOOK],
    )?;
    assert!(
        store
            .commit_cached_download_checkpoint(&task, "uid-0", 0, 1, OWNER, 101)
            .is_err()
    );
    assert_eq!(store.task(&task.id)?.unwrap(), task);
    assert_eq!(store.chapter_body_read_count(), 0);
    Ok(())
}

#[test]
fn cached_checkpoint_sql_failure_keeps_cursor_body_count_and_anchors() -> Result<()> {
    let (store, task) = fixture()?;
    store.save_position(&ReadingPosition {
        book_id: BOOK.into(),
        chapter_index: 0,
        char_offset: 3,
        ..Default::default()
    })?;
    store.save_annotation(&Annotation {
        id: "retained-note".into(),
        book_id: BOOK.into(),
        chapter_index: 0,
        start: 0,
        end: 8,
        quote: "Readable".into(),
        note: "Retained note".into(),
        created_at: 0,
    })?;
    let count = store.book(BOOK)?.unwrap().cached_chapters;
    let body_reads = store.chapter_body_read_count();
    store.connection.execute_batch(
        "CREATE TRIGGER refuse_cached_cursor BEFORE UPDATE ON download_tasks
         BEGIN SELECT RAISE(FAIL, 'synthetic cached cursor failure'); END;",
    )?;
    let error = store
        .commit_cached_download_checkpoint(&task, "uid-0", 0, 1, OWNER, 101)
        .unwrap_err();
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
    assert_eq!(store.task(&task.id)?.unwrap(), task);
    assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, count);
    assert_eq!(store.position(BOOK)?.unwrap().char_offset, 3);
    assert_eq!(store.annotations(BOOK)?[0].note, "Retained note");
    assert_eq!(store.chapter_body_read_count(), body_reads);
    assert_eq!(
        store.chapter_body(BOOK, 0)?.as_deref(),
        Some("Readable chapter 0")
    );
    store
        .connection
        .execute_batch("DROP TRIGGER refuse_cached_cursor")?;
    store.commit_cached_download_checkpoint(&task, "uid-0", 0, 1, OWNER, 101)?;
    assert_eq!(store.task(&task.id)?.unwrap().next_index, 1);
    Ok(())
}

#[test]
fn chapter_metadata_presence_is_bodyless_and_checkpoint_checks_unicode_trim() -> Result<()> {
    let (store, task) = fixture()?;
    let samples = [
        "",
        " \t\n\r",
        "\u{85}\u{a0}\u{1680}\u{2000}\u{2001}\u{2002}\u{2003}\u{2004}\u{2005}\u{2006}\u{2007}\u{2008}\u{2009}\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}",
        "\u{200b}",
        "\u{4e2d}\u{1f642}",
        "\0",
        " \0 ",
        "\tA\n",
    ];
    for sample in samples {
        store.save_task(&task)?;
        store.connection.execute(
            "UPDATE chapters SET body=?2 WHERE book_id=?1 AND chapter_index=0",
            params![BOOK, sample],
        )?;
        let metadata = store.chapter_metadata_with_presence(BOOK)?;
        assert_eq!(metadata[0].body_present, !sample.is_empty());
        assert_eq!(metadata[0].chapter.cached, !sample.is_empty());
        assert!(metadata.iter().all(|item| item.chapter.body.is_none()));
        assert_eq!(
            store
                .commit_cached_download_checkpoint(&task, "uid-0", 0, 1, OWNER, 101)
                .is_ok(),
            !sample.trim().is_empty(),
        );
        assert_eq!(store.chapter_body_read_count(), 0);
        assert_eq!(store.materialized_chapter_bytes(), 0);
    }
    Ok(())
}

struct NeverDeserialize;

impl<'de> Deserialize<'de> for NeverDeserialize {
    fn deserialize<D: Deserializer<'de>>(_: D) -> std::result::Result<Self, D::Error> {
        panic!("A rejected cached underline must not reach serde")
    }
}

#[test]
fn cached_underline_sql_preflight_refuses_bytes_type_and_count_before_serde() -> Result<()> {
    let (store, _) = fixture()?;
    store.connection.execute(
        "INSERT INTO settings(key,value) VALUES ('cached-underlines','[null,null]')",
        [],
    )?;
    for (bytes, records) in [(4, 2), (128, 1)] {
        let error = store
            .bounded_array_setting::<NeverDeserialize>("cached-underlines", bytes, records)
            .err()
            .unwrap();
        assert!(error.downcast_ref::<CachedResourceLimit>().is_some());
    }
    for bad in [
        rusqlite::types::Value::Blob(b"[]".to_vec()),
        rusqlite::types::Value::Text("{}".into()),
        rusqlite::types::Value::Text("[".into()),
    ] {
        store.connection.execute(
            "UPDATE settings SET value=?1 WHERE key='cached-underlines'",
            [&bad],
        )?;
        let error = store
            .bounded_array_setting::<NeverDeserialize>("cached-underlines", 128, 2)
            .err()
            .unwrap();
        assert!(error.downcast_ref::<CachedResourceLimit>().is_some());
        let saved: rusqlite::types::Value = store.connection.query_row(
            "SELECT value FROM settings WHERE key='cached-underlines'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(saved, bad);
    }
    Ok(())
}

#[test]
fn cached_underline_sql_exact_encoded_boundary_keeps_numeric_unicode_json() -> Result<()> {
    let (store, _) = fixture()?;
    let data = vec![json!({"range":17,"text":"\u{4e2d}\u{1f642}","count":2,"thoughts":3})];
    let encoded = serde_json::to_string(&data)?;
    store.set_setting("cached-underlines", &data)?;
    assert_eq!(
        store.bounded_array_setting::<serde_json::Value>("cached-underlines", encoded.len(), 1)?,
        Some(data.clone()),
    );
    assert!(
        store
            .bounded_array_setting::<serde_json::Value>("cached-underlines", encoded.len() - 1, 1)
            .is_err()
    );
    assert_eq!(
        store.setting::<Vec<serde_json::Value>>("cached-underlines")?,
        Some(data)
    );
    assert_eq!(store.chapter_body_read_count(), 0);
    Ok(())
}
