//! Task remapping, guarded transitions and old-run maintenance keep one commit.

use super::*;

fn fixture() -> Result<Store> {
    let s = Store::open(":memory:")?;
    s.upsert_book(&Book {
        source: "local".into(),
        intro: "Keep intro".into(),
        ..Book::new("book", "Book")
    })?;
    for (index, id, body) in [(0, "one", "Body one"), (1, "two", "Body two")] {
        s.upsert_chapter(&Chapter {
            id: id.into(),
            book_id: "book".into(),
            index,
            body: Some(body.into()),
            ..Default::default()
        })?;
    }
    s.set_setting("transition.account", &"keep account")?;
    Ok(s)
}
fn task(id: &str, owner: &str) -> DownloadTask {
    DownloadTask {
        id: id.into(),
        book_id: "book".into(),
        account_key: owner.into(),
        chapters: vec![0, 1],
        status: DownloadStatus::Running,
        next_index: 1,
        updated_at: 10,
        ..Default::default()
    }
}

#[test]
fn catalog_task_inventory_rejects_before_dto_and_preserves_all_owners_on_remap() -> Result<()> {
    let s = fixture()?;
    let first = task("first", "owner");
    let other = task("other", "foreign");
    s.save_task(&first)?;
    s.save_task(&other)?;
    let catalog = vec![
        Chapter {
            id: "two".into(),
            book_id: "book".into(),
            index: 0,
            title: "Two".into(),
            ..Default::default()
        },
        Chapter {
            id: "one".into(),
            book_id: "book".into(),
            index: 1,
            title: "One".into(),
            ..Default::default()
        },
    ];
    reset_inventory_test_read_stats();
    with_store_inventory_test_limits(
        StoreInventoryLimits {
            encoded_bytes: 1,
            ..Default::default()
        },
        || -> Result<()> {
            assert!(
                s.replace_catalog("book", &catalog)
                    .unwrap_err()
                    .downcast_ref::<StoreInventoryLimit>()
                    .is_some()
            );
            assert_eq!(inventory_test_read_stats().task_deserializations, 0);
            assert_eq!(s.chapter_body_read_count(), 0);
            Ok(())
        },
    )?;
    assert_eq!(s.task("first")?, Some(first));
    assert_eq!(s.task("other")?, Some(other));
    s.replace_catalog("book", &catalog)?;
    for id in ["first", "other"] {
        let t = s.task(id)?.unwrap();
        assert_eq!(t.chapters, vec![1, 0]);
        assert_eq!(t.next_index, 1);
        assert_eq!(t.status, DownloadStatus::Paused);
    }
    assert_eq!(s.chapter_body("book", 0)?.as_deref(), Some("Body two"));
    assert_eq!(s.chapter_body("book", 1)?.as_deref(), Some("Body one"));
    assert_eq!(
        s.setting::<String>("transition.account")?.as_deref(),
        Some("keep account")
    );
    Ok(())
}

#[test]
fn guarded_task_rejection_precedes_payload_and_stale_scalar_never_overwrites() -> Result<()> {
    let s = fixture()?;
    let old = task("current", "owner");
    s.save_task(&old)?;
    let mut next = old.clone();
    next.status = DownloadStatus::Paused;
    next.error = Some("new".into());
    reset_inventory_test_read_stats();
    with_store_inventory_test_limits(
        StoreInventoryLimits {
            encoded_bytes: 1,
            ..Default::default()
        },
        || -> Result<()> {
            assert!(
                s.save_task_if_current(&old, &next)
                    .unwrap_err()
                    .downcast_ref::<StoreInventoryLimit>()
                    .is_some()
            );
            assert_eq!(inventory_test_read_stats().task_deserializations, 0);
            Ok(())
        },
    )?;
    assert_eq!(s.task("current")?, Some(old.clone()));
    s.connection.execute(
        "UPDATE download_tasks SET status='paused',payload=?1 WHERE id='current'",
        ["unparsed stale payload"],
    )?;
    reset_inventory_test_read_stats();
    assert!(!s.save_task_if_current(&old, &next)?);
    assert_eq!(inventory_test_read_stats().task_deserializations, 0);
    assert_eq!(
        s.connection.query_row(
            "SELECT payload FROM download_tasks WHERE id='current'",
            [],
            |r| r.get::<_, String>(0)
        )?,
        "unparsed stale payload"
    );
    s.connection.execute(
        "UPDATE download_tasks SET status='running',updated_at=x'80' WHERE id='current'",
        [],
    )?;
    assert!(
        s.save_task_if_current(&old, &next)
            .unwrap_err()
            .downcast_ref::<StoreInventoryUnavailable>()
            .is_some()
    );
    assert_eq!(inventory_test_read_stats().task_deserializations, 0);
    Ok(())
}

#[test]
fn active_maintenance_is_independent_of_allocation_and_strictly_scoped() -> Result<()> {
    let s = fixture()?;
    let debt = "report.allocation_debt.v1:unattributed:old";
    s.set_setting("report.active.v1", &serde_json::json!({"id":"old"}))?;
    with_encoded_settings_test_limits(
        EncodedSettingsLimits {
            bytes: 32,
            ..Default::default()
        },
        || -> Result<()> {
            assert!(
                s.set_encoded_settings_and_delete(&[("report.active.v1", "{}")], &[])
                    .unwrap_err()
                    .downcast_ref::<CachedResourceLimit>()
                    .is_some()
            );
            s.set_report_active_encoded_and_delete(
                &[
                    ("report.active.v1", "{\"id\":\"new\"}"),
                    (debt, "{\"run\":{\"id\":\"old\"}}"),
                ],
                false,
            )?;
            Ok(())
        },
    )?;
    assert_eq!(
        s.setting::<serde_json::Value>("report.active.v1")?,
        Some(serde_json::json!({"id":"new"}))
    );
    assert!(
        s.set_report_active_encoded_and_delete(&[("report.outbox.v1", "{}")], false)
            .is_err()
    );
    assert!(
        s.set_report_active_encoded_and_delete(
            &[
                (debt, "{}"),
                ("report.allocation_debt.v1:unattributed:two", "{}")
            ],
            false
        )
        .is_err()
    );
    assert!(
        s.set_report_active_encoded_and_delete(&[("report.active.v1", "{}")], true)
            .is_err()
    );
    s.set_report_active_encoded_and_delete(&[], true)?;
    assert!(
        s.setting::<serde_json::Value>("report.active.v1")?
            .is_none()
    );
    assert!(s.setting::<serde_json::Value>(debt)?.is_some());
    Ok(())
}

#[test]
fn active_and_debt_second_row_sql_failure_rolls_back_both_records() -> Result<()> {
    let s = fixture()?;
    let debt = "report.allocation_debt.v1:unattributed:old";
    s.set_setting("report.active.v1", &serde_json::json!({"keep":"active"}))?;
    s.set_setting(debt, &serde_json::json!({"keep":"debt"}))?;
    s.connection.execute_batch("CREATE TRIGGER deny_debt BEFORE UPDATE OF value ON settings WHEN OLD.key='report.allocation_debt.v1:unattributed:old' BEGIN SELECT RAISE(ABORT,'fixture debt denied'); END")?;
    let error = s
        .set_report_active_encoded_and_delete(
            &[("report.active.v1", "{\"new\":1}"), (debt, "{\"new\":2}")],
            false,
        )
        .unwrap_err();
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
    assert_eq!(
        s.setting::<serde_json::Value>("report.active.v1")?,
        Some(serde_json::json!({"keep":"active"}))
    );
    assert_eq!(
        s.setting::<serde_json::Value>(debt)?,
        Some(serde_json::json!({"keep":"debt"}))
    );
    assert_eq!(
        s.setting::<String>("transition.account")?.as_deref(),
        Some("keep account")
    );
    Ok(())
}
