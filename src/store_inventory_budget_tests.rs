//! Real SQL inventory gates and borrowed body/manifest snapshot witnesses.

use super::*;
use std::cell::Cell;

thread_local! {
    static BOOL_READS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}
pub(super) fn observe_setting_bool_borrow(bytes: usize) {
    BOOL_READS.with(|slot| {
        let (count, total) = slot.get();
        slot.set((count + 1, total + bytes));
    });
}
fn bool_reads() -> (usize, usize) {
    BOOL_READS.with(Cell::get)
}
fn reset_bool_reads() {
    BOOL_READS.with(|slot| slot.set((0, 0)));
}

thread_local! {
    static STATS: Cell<InventoryReadStats> = const { Cell::new(InventoryReadStats {
        key_copies: 0, key_bytes: 0, manifest_borrows: 0, manifest_bytes: 0,
        manifest_deserializations: 0, body_borrows: 0, body_bytes: 0,
        task_deserializations: 0,
    }) };
}

pub(super) fn read_stats() -> InventoryReadStats {
    STATS.with(Cell::get)
}
pub(super) fn reset() {
    STATS.with(|slot| slot.set(InventoryReadStats::default()));
}
fn update(change: impl FnOnce(&mut InventoryReadStats)) {
    STATS.with(|slot| {
        let mut value = slot.get();
        change(&mut value);
        slot.set(value);
    });
}
pub(super) fn observe_key_copy(bytes: usize) {
    update(|s| {
        s.key_copies += 1;
        s.key_bytes += bytes;
    });
}
pub(super) fn observe_manifest_borrow(bytes: usize) {
    update(|s| {
        s.manifest_borrows += 1;
        s.manifest_bytes += bytes;
    });
}
pub(super) fn observe_manifest_read() {
    update(|s| s.manifest_deserializations += 1);
}
pub(super) fn observe_body_borrow(bytes: usize) {
    update(|s| {
        s.body_borrows += 1;
        s.body_bytes += bytes;
    });
}
pub(super) fn observe_task_read() {
    update(|s| s.task_deserializations += 1);
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
        body: Some("Body\0tail".into()),
        ..Default::default()
    })?;
    store.set_setting("inventory.fixture.account", &"owner")?;
    store.set_setting(
        "inventory.fixture.preferences",
        &vec!["Keep", "preferences"],
    )?;
    Ok(store)
}

fn assert_retained(store: &Store) -> Result<()> {
    assert_eq!(store.book("book")?.unwrap().intro, "Keep intro");
    assert_eq!(
        store.chapter_body("book", 0)?.as_deref(),
        Some("Body\0tail")
    );
    assert_eq!(
        store
            .setting::<String>("inventory.fixture.account")?
            .as_deref(),
        Some("owner")
    );
    assert_eq!(
        store.setting::<Vec<String>>("inventory.fixture.preferences")?,
        Some(vec!["Keep".into(), "preferences".into()])
    );
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
struct Index {
    #[serde(default)]
    images: Vec<String>,
}

#[test]
fn inventory_headers_are_numeric_and_manifest_projection_skips_unrequested_html() -> Result<()> {
    let store = fixture()?;
    store.set_setting(
        "chapter.assets:book:uid",
        &serde_json::json!({
            "images": ["one"], "html": "x".repeat(4096)
        }),
    )?;
    reset();
    let snapshot = store.resource_verification_snapshot()?;
    let headers = snapshot.setting_inventory_headers_with_prefix("chapter.assets:")?;
    assert_eq!(headers.len(), 1);
    assert_eq!(read_stats(), InventoryReadStats::default());
    let header = &headers[0];
    assert_eq!(
        snapshot.bounded_setting_inventory_key(header)?,
        "chapter.assets:book:uid"
    );
    assert_eq!(
        snapshot.image_manifest_inventory_metadata(header, header.value_utf8_bytes_upper, 1)?,
        1
    );
    assert_eq!(read_stats().manifest_deserializations, 0);
    let index: Index =
        snapshot.bounded_image_manifest_inventory(header, header.value_utf8_bytes_upper, 1)?;
    assert_eq!(index.images, vec!["one"]);
    assert_eq!(read_stats().manifest_borrows, 2);
    assert_eq!(read_stats().manifest_deserializations, 1);
    assert_eq!(store.chapter_body_read_count(), 0);
    assert_eq!(store.book_read_stats().intro_copies, 0);
    drop(snapshot);
    assert_retained(&store)
}

#[test]
fn inventory_low_quota_precedes_any_key_or_value_materialization() -> Result<()> {
    let store = fixture()?;
    store.set_setting(
        "chapter.assets:book:uid",
        &serde_json::json!({ "images": ["one"] }),
    )?;
    reset();
    with_store_inventory_test_limits(
        StoreInventoryLimits {
            encoded_bytes: 1,
            ..Default::default()
        },
        || -> Result<()> {
            let snapshot = store.resource_verification_snapshot()?;
            let error = snapshot
                .setting_inventory_headers_with_prefix("chapter.assets:")
                .unwrap_err();
            assert!(error.downcast_ref::<StoreInventoryLimit>().is_some());
            assert_eq!(read_stats(), InventoryReadStats::default());
            Ok(())
        },
    )?;
    assert!(
        store
            .setting::<serde_json::Value>("chapter.assets:book:uid")?
            .is_some()
    );
    assert_retained(&store)
}

#[test]
fn body_uid_snapshot_borrows_exact_utf8_without_an_owned_body() -> Result<()> {
    let store = fixture()?;
    for (index, uid) in [(1, "empty"), (2, "null")] {
        store.upsert_chapter(&Chapter {
            id: uid.into(),
            book_id: "book".into(),
            index,
            body: None,
            ..Default::default()
        })?;
    }
    // Legacy SQL rows may contain empty TEXT; cache upserts intentionally
    // reject it. Exercise the read boundary without changing that policy.
    assert_eq!(store.connection.execute("UPDATE chapters SET body='' WHERE book_id='book' AND chapter_index=1 AND source_id='empty'", [])?, 1);
    reset();
    let snapshot = store.resource_verification_snapshot()?;
    let metadata = snapshot
        .chapter_body_metadata_by_uid("book", "uid")?
        .unwrap();
    assert!(metadata.body_present);
    assert_eq!(
        snapshot.with_bounded_chapter_body_by_uid(
            "book",
            "uid",
            metadata.utf8_bytes_upper,
            |body| {
                assert_eq!(body, "Body\0tail");
                Ok(body.chars().count())
            }
        )?,
        Some(9)
    );
    assert!(
        snapshot
            .chapter_body_metadata_by_uid("book", "empty")?
            .unwrap()
            .body_present
    );
    assert_eq!(
        snapshot.with_bounded_chapter_body_by_uid("book", "empty", 0, |body| Ok(body.len()))?,
        Some(0)
    );
    assert!(
        !snapshot
            .chapter_body_metadata_by_uid("book", "null")?
            .unwrap()
            .body_present
    );
    assert_eq!(
        snapshot.with_bounded_chapter_body_by_uid("book", "null", 0, |_| -> Result<usize> {
            panic!("NULL body must not be borrowed")
        })?,
        None
    );
    assert!(
        snapshot
            .chapter_body_metadata_by_uid("book", "missing")?
            .is_none()
    );
    assert_eq!(store.chapter_body_read_count(), 0);
    assert_eq!(read_stats().body_borrows, 2);
    assert_eq!(read_stats().body_bytes, 9);
    Ok(())
}

#[test]
fn task_inventory_selects_current_owner_fifo_after_whole_input_admission() -> Result<()> {
    let store = fixture()?;
    let mut original = Vec::new();
    for (id, owner, time, status, cursor) in [
        ("first", "owner", 3, DownloadStatus::Running, 1),
        ("other", "foreign", 2, DownloadStatus::Pending, 0),
        ("third", "owner", 1, DownloadStatus::Paused, 0),
    ] {
        let task = DownloadTask {
            id: id.into(),
            book_id: "book".into(),
            account_key: owner.into(),
            chapters: vec![0, 1],
            updated_at: time,
            status,
            next_index: cursor,
            error: Some("Keep redacted history".repeat(16)),
            ..Default::default()
        };
        store.save_task(&task)?;
        original.push(task);
    }
    reset();
    let transaction = store.connection.unchecked_transaction()?;
    let tasks = store.pending_tasks_for_owner("owner")?;
    transaction.commit()?;
    assert_eq!(tasks, vec![original[2].clone(), original[0].clone()]);
    assert_eq!(read_stats().task_deserializations, 2);
    assert_eq!(store.pending_task_count()?, 3);
    reset();
    with_store_inventory_test_limits(
        StoreInventoryLimits {
            encoded_bytes: 1,
            ..Default::default()
        },
        || -> Result<()> {
            assert!(
                store
                    .pending_tasks_for_owner("owner")
                    .unwrap_err()
                    .downcast_ref::<StoreInventoryLimit>()
                    .is_some()
            );
            assert!(
                store
                    .task("first")
                    .unwrap_err()
                    .downcast_ref::<StoreInventoryLimit>()
                    .is_some()
            );
            assert_eq!(read_stats().task_deserializations, 0);
            Ok(())
        },
    )?;
    assert_eq!(store.task("first")?, Some(original[0].clone()));
    assert_eq!(store.pending_task_count()?, 3);
    assert_retained(&store)
}

#[test]
fn body_invalid_type_and_uid_ambiguity_never_become_missing_content() -> Result<()> {
    let store = fixture()?;
    store
        .connection
        .execute("UPDATE chapters SET body=x'80'", [])?;
    reset();
    let snapshot = store.resource_verification_snapshot()?;
    assert!(
        snapshot
            .chapter_body_metadata_by_uid("book", "uid")
            .unwrap_err()
            .downcast_ref::<StoreInventoryUnavailable>()
            .is_some()
    );
    assert_eq!(read_stats().body_borrows, 0);
    drop(snapshot);
    store
        .connection
        .execute("UPDATE chapters SET body=?1", ["Body\0tail"])?;
    store.connection.execute("INSERT INTO chapters (book_id,chapter_index,source_id,title,body,content_status,content_account_key) VALUES ('book',1,'uid','Duplicate',NULL,'unknown','')", [])?;
    assert!(
        store
            .resource_verification_snapshot()?
            .chapter_body_metadata_by_uid("book", "uid")
            .unwrap_err()
            .downcast_ref::<StoreInventoryUnavailable>()
            .is_some()
    );
    assert_eq!(read_stats().body_borrows, 0);
    assert_eq!(store.chapter_body_read_count(), 0);
    assert_retained(&store)
}

#[test]
fn utf16_pctencoded_key_uses_actual_utf8_contract_and_prepaid_upper() -> Result<()> {
    for encoding in ["UTF-16le", "UTF-16be"] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("db");
        {
            let mut connection = Connection::open(&path)?;
            connection.pragma_update(None, "encoding", encoding)?;
            initialize_schema(&mut connection)?;
        }
        let store = Store::open(&path)?;
        let key = format!("chapter.assets:{}", "%25".repeat(8192));
        assert!(key.len() < 32 * 1024);
        store.set_setting(&key, &serde_json::json!({ "images": [] }))?;
        reset();
        let snapshot = store.resource_verification_snapshot()?;
        let headers = snapshot.setting_inventory_headers_with_prefix("chapter.assets:")?;
        assert!(headers[0].key_utf8_bytes_upper > 32 * 1024);
        assert_eq!(snapshot.bounded_setting_inventory_key(&headers[0])?, key);
        assert_eq!(read_stats().key_bytes, key.len());
        assert_eq!(read_stats().manifest_borrows, 0);
    }
    Ok(())
}

#[test]
fn inventory_snapshot_revisions_keep_external_view_and_reject_own_changes() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("db");
    let store = Store::open(&path)?;
    let old = serde_json::json!({ "images": ["old"] });
    let key = "chapter.assets:book:uid";
    store.set_setting(key, &old)?;
    let snapshot = store.resource_verification_snapshot()?;
    let revision = snapshot.resource_revision();
    let header = snapshot.setting_inventory_headers_with_prefix("chapter.assets:")?[0];
    let external = Connection::open(&path)?;
    external.execute(
        "UPDATE settings SET value=?1 WHERE key=?2",
        params!["{\"images\":[\"external\",\"two\"]}", key],
    )?;
    reset();
    let index: Index =
        snapshot.bounded_image_manifest_inventory(&header, header.value_utf8_bytes_upper, 1)?;
    assert_eq!(index.images, vec!["old"]);
    drop(snapshot);
    assert_ne!(store.resource_revision()?.0, revision.0);
    assert_eq!(store.resource_revision()?.1, revision.1);
    let snapshot = store.resource_verification_snapshot()?;
    let header = snapshot.setting_inventory_headers_with_prefix("chapter.assets:")?[0];
    store.set_setting(key, &old)?;
    reset();
    let error = snapshot
        .bounded_image_manifest_inventory::<Index>(&header, header.value_utf8_bytes_upper, 2)
        .unwrap_err();
    assert!(error.downcast_ref::<StoreInventoryUnavailable>().is_some());
    assert_eq!(read_stats().manifest_borrows, 0);
    drop(snapshot);
    assert_eq!(
        store.setting::<serde_json::Value>(key)?,
        Some(serde_json::json!({ "images": ["external", "two"] }))
    );
    Ok(())
}

#[test]
fn inventory_prefix_prices_unrelated_key_work_before_any_payload() -> Result<()> {
    let store = fixture()?;
    store.set_setting(
        &format!("unrelated:{}", "u".repeat(4096)),
        &"Keep unrelated value",
    )?;
    store.set_setting(
        "chapter.assets:book:uid",
        &serde_json::json!({ "images": [] }),
    )?;
    for (limits, resource) in [
        (
            StoreInventoryLimits {
                work_bytes: 512,
                ..Default::default()
            },
            "prefix work bytes",
        ),
        (
            StoreInventoryLimits {
                scratch_bytes: 512,
                ..Default::default()
            },
            "prefix scratch bytes",
        ),
        (
            StoreInventoryLimits {
                rows: 1,
                ..Default::default()
            },
            "key inventory rows",
        ),
    ] {
        reset();
        with_store_inventory_test_limits(limits, || -> Result<()> {
            let snapshot = store.resource_verification_snapshot()?;
            let error = snapshot
                .setting_inventory_headers_with_prefix("chapter.assets:")
                .unwrap_err();
            assert_eq!(
                error
                    .downcast_ref::<StoreInventoryLimit>()
                    .unwrap()
                    .resource,
                resource
            );
            assert_eq!(read_stats(), InventoryReadStats::default());
            Ok(())
        })?;
    }
    assert_retained(&store)
}

#[test]
fn task_inventory_output_and_parser_scratch_share_one_exact_allowance() -> Result<()> {
    let store = fixture()?;
    let mut original = Vec::new();
    for id in ["first", "second"] {
        let task = DownloadTask {
            id: id.into(),
            book_id: "book".into(),
            account_key: "owner".into(),
            chapters: vec![0],
            error: Some("Keep".repeat(8)),
            ..Default::default()
        };
        store.save_task(&task)?;
        original.push(task);
    }
    let bytes = original
        .iter()
        .map(|task| serde_json::to_string(task).map(|s| s.len()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let header_price = original.len() * (2 * std::mem::size_of::<TaskInventoryHeader>() + 128);
    let output_price = header_price
        + bytes
            .iter()
            .map(|n| 16 * n + 2 * std::mem::size_of::<DownloadTask>())
            .sum::<usize>();
    let parser_price = 3 * bytes.iter().copied().max().unwrap() + 258;
    let exact = output_price + parser_price;
    reset();
    with_store_inventory_test_limits(
        StoreInventoryLimits {
            scratch_bytes: exact - 1,
            ..Default::default()
        },
        || -> Result<()> {
            let error = store.pending_tasks_for_owner("owner").unwrap_err();
            assert_eq!(
                error
                    .downcast_ref::<StoreInventoryLimit>()
                    .unwrap()
                    .resource,
                "task selected scratch"
            );
            assert_eq!(read_stats().task_deserializations, 0);
            Ok(())
        },
    )?;
    with_store_inventory_test_limits(
        StoreInventoryLimits {
            scratch_bytes: exact,
            ..Default::default()
        },
        || -> Result<()> {
            assert_eq!(store.pending_tasks_for_owner("owner")?, original);
            assert_eq!(read_stats().task_deserializations, 2);
            Ok(())
        },
    )?;
    assert_eq!(store.pending_task_count()?, 2);
    assert_retained(&store)
}

#[test]
fn task_owner_projection_preserves_current_fields_and_strict_error_causes() -> Result<()> {
    let store = fixture()?;
    let payload = serde_json::to_string(&DownloadTask {
        id: "legacy".into(),
        book_id: "book".into(),
        chapters: vec![0],
        ..Default::default()
    })?;
    store.connection.execute("INSERT INTO download_tasks (id,book_id,status,updated_at,payload) VALUES ('legacy','book','pending',0,?1)", [&payload])?;
    reset();
    assert!(store.pending_tasks_for_owner("owner")?.is_empty());
    assert_eq!(read_stats().task_deserializations, 0);
    assert_eq!(
        store.pending_tasks()?,
        vec![DownloadTask {
            id: "legacy".into(),
            book_id: "book".into(),
            chapters: vec![0],
            ..Default::default()
        }]
    );
    let selected = DownloadTask {
        id: "seq".into(),
        book_id: "book".into(),
        account_key: "owner".into(),
        chapters: vec![0],
        ..Default::default()
    };
    let sequence = serde_json::to_string(&selected)?;
    store.connection.execute("INSERT INTO download_tasks (id,book_id,status,updated_at,payload) VALUES ('seq','book','pending',0,?1)", [&sequence])?;
    assert_eq!(
        store.pending_tasks_for_owner("owner")?,
        vec![selected.clone()]
    );
    store.connection.execute(
        "UPDATE download_tasks SET payload=x'80' WHERE id='legacy'",
        [],
    )?;
    reset();
    assert!(
        store
            .pending_tasks_for_owner("owner")
            .unwrap_err()
            .downcast_ref::<StoreInventoryUnavailable>()
            .is_some()
    );
    assert!(
        store
            .task("legacy")
            .unwrap_err()
            .downcast_ref::<StoreInventoryUnavailable>()
            .is_some()
    );
    assert_eq!(read_stats().task_deserializations, 0);
    store.connection.execute(
        "UPDATE download_tasks SET payload='{' WHERE id='legacy'",
        [],
    )?;
    let error = store.pending_tasks_for_owner("owner").unwrap_err();
    assert!(error.downcast_ref::<serde_json::Error>().is_some());
    assert_eq!(read_stats().task_deserializations, 0);
    store.connection.execute(
        "UPDATE download_tasks SET payload=?1 WHERE id='legacy'",
        [&payload],
    )?;
    store.connection.execute(
        "UPDATE download_tasks SET updated_at=x'80' WHERE id='legacy'",
        [],
    )?;
    let error = store.pending_tasks_for_owner("owner").unwrap_err();
    assert!(error.downcast_ref::<StoreInventoryUnavailable>().is_some());
    assert_eq!(read_stats().task_deserializations, 0);
    store.connection.execute(
        "UPDATE download_tasks SET updated_at=0 WHERE id='legacy'",
        [],
    )?;
    store
        .connection
        .execute_batch("ALTER TABLE download_tasks RENAME TO held_tasks")?;
    let error = store.pending_task_count().unwrap_err();
    store
        .connection
        .execute_batch("ALTER TABLE held_tasks RENAME TO download_tasks")?;
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
    assert_eq!(store.pending_task_count()?, 2);
    assert_eq!(store.task("seq")?, Some(selected));
    assert_retained(&store)
}

#[test]
fn task_payload_requires_current_fields_and_rejects_old_sequences() -> Result<()> {
    let store = fixture()?;
    let task = DownloadTask {
        id: "current-task".into(),
        book_id: "book".into(),
        account_key: "owner".into(),
        chapters: vec![0],
        updated_at: 100,
        ..Default::default()
    };
    store.save_task(&task)?;
    assert_eq!(store.pending_tasks_for_owner("owner")?, vec![task.clone()]);
    let current = serde_json::to_value(&task)?;
    for field in ["account_key", "completed_count", "updated_at", "chapters"] {
        for null in [false, true] {
            let mut damaged = current.clone();
            if null {
                damaged[field] = serde_json::Value::Null;
            } else {
                damaged.as_object_mut().unwrap().remove(field);
            }
            let payload = serde_json::to_string(&damaged)?;
            store.connection.execute(
                "UPDATE download_tasks SET payload=?1 WHERE id=?2",
                params![payload, task.id],
            )?;
            assert!(
                store
                    .task(&task.id)
                    .unwrap_err()
                    .downcast_ref::<serde_json::Error>()
                    .is_some()
            );
            assert!(
                store
                    .pending_tasks_for_owner("owner")
                    .unwrap_err()
                    .downcast_ref::<serde_json::Error>()
                    .is_some()
            );
            let retained: String = store.connection.query_row(
                "SELECT payload FROM download_tasks WHERE id=?1",
                [&task.id],
                |row| row.get(0),
            )?;
            assert_eq!(retained, payload);
        }
    }
    let sequence = r#"["current-task","book","owner",[0],"pending",false,0,null,100]"#;
    store.connection.execute(
        "UPDATE download_tasks SET payload=?1 WHERE id=?2",
        params![sequence, task.id],
    )?;
    assert!(store.task(&task.id).is_err());
    assert!(store.pending_tasks_for_owner("owner").is_err());
    assert_retained(&store)
}

#[test]
fn setting_inventory_bool_preserves_whitespace_false_and_borrowed_key_order() -> Result<()> {
    let store = fixture()?;
    store.set_setting("shelf.override:book", &true)?;
    store.set_setting("shelf.override:other", &false)?;
    let whitespace = format!("\t\r\n{}true{}\n", " ".repeat(128), " ".repeat(128));
    store.connection.execute(
        "UPDATE settings SET value=?1 WHERE key='shelf.override:book'",
        [&whitespace],
    )?;
    reset();
    reset_bool_reads();
    let snapshot = store.resource_verification_snapshot()?;
    let headers = snapshot.setting_inventory_headers_with_prefix("shelf.override:")?;
    assert_eq!(headers.len(), 2);
    assert!(snapshot.bounded_setting_inventory_bool(&headers[0])?);
    assert!(!snapshot.bounded_setting_inventory_bool(&headers[1])?);
    assert_eq!(bool_reads(), (2, whitespace.len() + "false".len()));
    assert_eq!(read_stats(), InventoryReadStats::default());
    assert_eq!(
        snapshot.bounded_setting_inventory_key(&headers[0])?,
        "shelf.override:book"
    );
    assert_eq!(
        snapshot.bounded_setting_inventory_key(&headers[1])?,
        "shelf.override:other"
    );
    assert_eq!(read_stats().key_copies, 2);
    assert_eq!(store.chapter_body_read_count(), 0);
    assert_eq!(store.book_read_stats().intro_copies, 0);
    drop(snapshot);
    assert_eq!(
        store.settings_with_prefix::<bool>("shelf.override:")?,
        vec![
            ("shelf.override:book".into(), true),
            ("shelf.override:other".into(), false)
        ]
    );
    assert_retained(&store)
}

#[test]
fn setting_inventory_bool_exact_gate_and_header_fence_precede_payload() -> Result<()> {
    let store = fixture()?;
    store.set_setting("shelf.override:book", &true)?;
    let snapshot = store.resource_verification_snapshot()?;
    let header = snapshot.setting_inventory_headers_with_prefix("shelf.override:")?[0];
    let bytes = "true".len();
    reset();
    for (limits, resource) in [
        (
            StoreInventoryLimits {
                work_bytes: 2 * bytes - 1,
                ..Default::default()
            },
            "setting boolean work",
        ),
        (
            StoreInventoryLimits {
                scratch_bytes: 3 * bytes + 257,
                ..Default::default()
            },
            "setting boolean scratch",
        ),
    ] {
        reset_bool_reads();
        with_store_inventory_test_limits(limits, || -> Result<()> {
            let error = snapshot
                .bounded_setting_inventory_bool(&header)
                .unwrap_err();
            assert_eq!(
                error
                    .downcast_ref::<StoreInventoryLimit>()
                    .unwrap()
                    .resource,
                resource
            );
            assert_eq!(bool_reads(), (0, 0));
            assert_eq!(read_stats(), InventoryReadStats::default());
            Ok(())
        })?;
    }
    with_store_inventory_test_limits(
        StoreInventoryLimits {
            work_bytes: 2 * bytes,
            scratch_bytes: 3 * bytes + 258,
            ..Default::default()
        },
        || -> Result<()> {
            assert!(snapshot.bounded_setting_inventory_bool(&header)?);
            assert_eq!(bool_reads(), (1, bytes));
            assert_eq!(read_stats(), InventoryReadStats::default());
            Ok(())
        },
    )?;
    let mut mismatch = header;
    mismatch.value_utf8_bytes_upper += 1;
    reset_bool_reads();
    assert!(
        snapshot
            .bounded_setting_inventory_bool(&mismatch)
            .unwrap_err()
            .downcast_ref::<StoreInventoryUnavailable>()
            .is_some()
    );
    assert_eq!(bool_reads(), (0, 0));
    store.set_setting("shelf.override:book", &false)?;
    assert!(
        snapshot
            .bounded_setting_inventory_bool(&header)
            .unwrap_err()
            .downcast_ref::<StoreInventoryUnavailable>()
            .is_some()
    );
    assert_eq!(bool_reads(), (0, 0));
    drop(snapshot);
    assert_eq!(store.setting::<bool>("shelf.override:book")?, Some(true));
    assert_retained(&store)
}

#[test]
fn setting_inventory_bool_utf16_and_decode_sql_errors_keep_original_data() -> Result<()> {
    for encoding in ["UTF-8", "UTF-16le", "UTF-16be"] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("db");
        {
            let mut connection = Connection::open(&path)?;
            connection.pragma_update(None, "encoding", encoding)?;
            initialize_schema(&mut connection)?;
        }
        let store = Store::open(&path)?;
        let key = "shelf.override:book";
        store.set_setting(key, &false)?;
        let value = "\r\n\t false \t\n";
        store.connection.execute(
            "UPDATE settings SET value=?1 WHERE key=?2",
            params![value, key],
        )?;
        let snapshot = store.resource_verification_snapshot()?;
        let header = snapshot.setting_inventory_headers_with_prefix("shelf.override:")?[0];
        reset_bool_reads();
        assert!(!snapshot.bounded_setting_inventory_bool(&header)?);
        assert_eq!(bool_reads(), (1, value.len()));
        drop(snapshot);
        store.set_setting(key, &"false")?;
        let snapshot = store.resource_verification_snapshot()?;
        let header = snapshot.setting_inventory_headers_with_prefix("shelf.override:")?[0];
        let error = snapshot
            .bounded_setting_inventory_bool(&header)
            .unwrap_err();
        assert!(error.downcast_ref::<serde_json::Error>().is_some());
        assert_eq!(error.to_string(), "decode local setting collection");
        drop(snapshot);
        store.set_setting(key, &false)?;
        let snapshot = store.resource_verification_snapshot()?;
        let header = snapshot.setting_inventory_headers_with_prefix("shelf.override:")?[0];
        store
            .connection
            .execute_batch("ALTER TABLE settings RENAME TO held_settings")?;
        reset_bool_reads();
        let error = snapshot
            .bounded_setting_inventory_bool(&header)
            .unwrap_err();
        assert!(error.downcast_ref::<rusqlite::Error>().is_some());
        assert_eq!(bool_reads(), (0, 0));
        drop(snapshot);
        assert_eq!(store.setting::<bool>(key)?, Some(false));
    }
    Ok(())
}
