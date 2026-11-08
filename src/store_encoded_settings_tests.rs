//! Trusted encoded transactions and bounded literal-prefix key rotation.

use super::*;
use std::cell::RefCell;

thread_local! {
    static KEY_HOOK: RefCell<Option<Box<dyn FnOnce() -> Result<()>>>> = const { RefCell::new(None) };
}

pub(super) fn after_key_preflight() -> Result<()> {
    let hook = KEY_HOOK.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook()?;
    }
    Ok(())
}

struct HookGuard;
impl Drop for HookGuard {
    fn drop(&mut self) {
        KEY_HOOK.with(|slot| slot.borrow_mut().take());
    }
}

fn store(encoding: &str) -> Result<(tempfile::TempDir, Store)> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("encoded.sqlite");
    {
        let mut connection = Connection::open(&path)?;
        connection.pragma_update(None, "encoding", encoding)?;
        migrate(&mut connection)?;
    }
    Ok((directory, Store::open(path)?))
}

fn seed(store: &Store) -> Result<Vec<String>> {
    for (key, value) in [
        ("report.outbox", "{\"old\":1}"),
        ("report.active", "{\"allocated\":1}"),
        ("account", "\"synthetic-account\""),
        ("preferences", "{\"keep\":true}"),
    ] {
        store.connection.execute(
            "INSERT INTO settings(key,value) VALUES (?1,?2)",
            params![key, value],
        )?;
    }
    let keys = [
        "receipt",
        "error",
        "failure",
        "retryable",
        "requires_login",
        "await_auth",
        "uncertain",
    ]
    .into_iter()
    .map(|kind| format!("report.{kind}:session"))
    .collect::<Vec<_>>();
    for key in &keys {
        store
            .connection
            .execute("INSERT INTO settings(key,value) VALUES (?1,'true')", [key])?;
    }
    Ok(keys)
}

fn snapshot(store: &Store) -> Result<Vec<(String, String)>> {
    let mut statement = store
        .connection
        .prepare("SELECT key,value FROM settings ORDER BY key")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[test]
fn encoded_unicode_bytes_commit_exactly_with_both_state_rows_and_seven_cleanup_keys() -> Result<()>
{
    for encoding in ["UTF-8", "UTF-16le", "UTF-16be"] {
        let (_directory, store) = store(encoding)?;
        let removed = seed(&store)?;
        let outbox =
            serde_json::to_string(&serde_json::json!({"label":"\u{4e2d}\0text","segments":[1,2]}))?;
        let active = serde_json::to_string(&serde_json::json!({"allocated":2,"debt":3}))?;
        store.set_encoded_settings_and_delete(
            &[("report.outbox", &outbox), ("report.active", &active)],
            &removed,
        )?;
        let stored_outbox: String = store.connection.query_row(
            "SELECT value FROM settings WHERE key='report.outbox'",
            [],
            |row| row.get(0),
        )?;
        let stored_active: String = store.connection.query_row(
            "SELECT value FROM settings WHERE key='report.active'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(stored_outbox.as_bytes(), outbox.as_bytes());
        assert_eq!(stored_active.as_bytes(), active.as_bytes());
        for key in &removed {
            assert_eq!(
                store.connection.query_row(
                    "SELECT COUNT(*) FROM settings WHERE key=?1",
                    [key],
                    |row| row.get::<_, i64>(0)
                )?,
                0
            );
        }
        assert_eq!(
            store.setting::<String>("account")?,
            Some("synthetic-account".into())
        );
        assert_eq!(
            store.setting::<serde_json::Value>("preferences")?,
            Some(serde_json::json!({"keep":true}))
        );
    }
    Ok(())
}

#[test]
fn second_replacement_trigger_rolls_back_outbox_active_and_all_old_proofs() -> Result<()> {
    let (_directory, store) = store("UTF-8")?;
    let removed = seed(&store)?;
    let original = snapshot(&store)?;
    store.connection.execute_batch(
        "CREATE TEMP TRIGGER reject_active BEFORE UPDATE ON settings
        WHEN NEW.key='report.active' BEGIN SELECT RAISE(ABORT,'fixture active failure'); END",
    )?;
    let error = store
        .set_encoded_settings_and_delete(
            &[
                ("report.outbox", "{\"new\":2}"),
                ("report.active", "{\"allocated\":2}"),
            ],
            &removed,
        )
        .unwrap_err();
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
    assert_eq!(snapshot(&store)?, original);
    Ok(())
}

#[test]
fn cleanup_trigger_rolls_back_both_replacements_and_prior_deletions() -> Result<()> {
    let (_directory, store) = store("UTF-8")?;
    let removed = seed(&store)?;
    let original = snapshot(&store)?;
    store.connection.execute_batch("CREATE TEMP TRIGGER reject_cleanup BEFORE DELETE ON settings
        WHEN OLD.key='report.failure:session' BEGIN SELECT RAISE(ABORT,'fixture cleanup failure'); END")?;
    let error = store
        .set_encoded_settings_and_delete(
            &[
                ("report.outbox", "{\"new\":2}"),
                ("report.active", "{\"allocated\":2}"),
            ],
            &removed,
        )
        .unwrap_err();
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
    assert_eq!(snapshot(&store)?, original);
    Ok(())
}

#[test]
fn all_setting_key_errors_precede_transaction_and_parameter_binding() -> Result<()> {
    let (_directory, store) = store("UTF-8")?;
    seed(&store)?;
    let original = snapshot(&store)?;
    let outer = store.connection.unchecked_transaction()?;
    for (replacements, removed) in [
        (vec![("", "null")], vec![]),
        (vec![("bad\0key", "null")], vec![]),
        (vec![("same", "1"), ("same", "2")], vec![]),
        (vec![("same", "1")], vec!["same".into()]),
        (vec![("valid", "1")], vec![" \u{3000}".into()]),
    ] {
        let error = store
            .set_encoded_settings_and_delete(&replacements, &removed)
            .unwrap_err();
        assert!(
            error.downcast_ref::<rusqlite::Error>().is_none(),
            "validation must precede nested BEGIN"
        );
        assert!(!store.connection.is_autocommit());
    }
    let long_key = "k".repeat(4097);
    assert!(
        store
            .set_encoded_settings_and_delete(&[(&long_key, "null")], &[])
            .unwrap_err()
            .downcast_ref::<CachedResourceLimit>()
            .is_some()
    );
    outer.rollback()?;
    assert_eq!(snapshot(&store)?, original);
    Ok(())
}

#[test]
fn encoded_envelope_exact_and_one_less_do_not_reparse_or_replace_old_state() -> Result<()> {
    let (_directory, store) = store("UTF-8")?;
    let removed = seed(&store)?;
    let replacements = [
        ("report.outbox", "{\"segments\":[1]}"),
        ("report.active", "{\"allocated\":1}"),
    ];
    let keys = replacements.iter().map(|(key, _)| key.len()).sum::<usize>()
        + removed.iter().map(String::len).sum::<usize>();
    let encoded = replacements
        .iter()
        .map(|(_, value)| value.len())
        .sum::<usize>();
    let required = encoded
        + 4 * keys
        + encoded_setting_key_slots(replacements.len())?
        + std::mem::size_of::<HashSet<&str>>();
    let original = snapshot(&store)?;
    with_encoded_settings_test_limits(
        EncodedSettingsLimits {
            bytes: required - 1,
            ..Default::default()
        },
        || -> Result<()> {
            assert!(
                store
                    .set_encoded_settings_and_delete(&replacements, &removed)
                    .unwrap_err()
                    .downcast_ref::<CachedResourceLimit>()
                    .is_some()
            );
            assert_eq!(snapshot(&store)?, original);
            Ok(())
        },
    )?;
    with_encoded_settings_test_limits(
        EncodedSettingsLimits {
            bytes: required,
            ..Default::default()
        },
        || -> Result<()> { store.set_encoded_settings_and_delete(&replacements, &removed) },
    )?;
    assert_eq!(
        store.connection.query_row(
            "SELECT value FROM settings WHERE key='report.outbox'",
            [],
            |row| row.get::<_, String>(0)
        )?,
        replacements[0].1
    );
    Ok(())
}

#[test]
fn prefix_lookup_is_literal_rotates_after_marker_and_never_projects_value() -> Result<()> {
    let (_directory, store) = store("UTF-8")?;
    for key in ["p%_a", "p%_\u{4e2d}", "px_a", "pZZa"] {
        // A value projection into String would fail on every row.
        store
            .connection
            .execute("INSERT INTO settings(key,value) VALUES (?1,x'80')", [key])?;
    }
    assert_eq!(
        store.first_setting_key_with_prefix("p%_", None)?,
        Some("p%_a".into())
    );
    assert_eq!(
        store.first_setting_key_with_prefix("p%_", Some("p%_a"))?,
        Some("p%_\u{4e2d}".into())
    );
    assert_eq!(
        store.first_setting_key_with_prefix("p%_", Some("p%_\u{4e2d}"))?,
        Some("p%_a".into())
    );
    assert_eq!(store.first_setting_key_with_prefix("missing:", None)?, None);
    assert_eq!(store.encoded_setting_read_stats().key_materializations, 3);
    assert_eq!(store.book_read_stats().intro_copies, 0);
    assert_eq!(store.chapter_body_read_count(), 0);
    Ok(())
}

#[test]
fn prefix_utf16_and_exact_scratch_fee_gate_the_only_key_copy() -> Result<()> {
    for encoding in ["UTF-8", "UTF-16le", "UTF-16be"] {
        let (_directory, store) = store(encoding)?;
        let key = "p:\u{4e2d}";
        store
            .connection
            .execute("INSERT INTO settings(key,value) VALUES (?1,x'80')", [key])?;
        let upper = if encoding == "UTF-8" { 5 } else { 9 };
        let fixed = 2 * (std::mem::size_of::<String>() + std::mem::size_of::<i64>());
        let required = (6 * upper + 16 + "p:".len())
            .max(2 * upper + fixed)
            .max(upper + fixed);
        with_encoded_settings_test_limits(
            EncodedSettingsLimits {
                bytes: required,
                key_bytes: upper,
            },
            || -> Result<()> {
                assert_eq!(
                    store.first_setting_key_with_prefix("p:", None)?,
                    Some(key.into())
                );
                assert_eq!(
                    store.encoded_setting_read_stats(),
                    EncodedSettingReadStats {
                        key_materializations: 1,
                        key_bytes: key.len()
                    }
                );
                Ok(())
            },
        )?;
        store.reset_encoded_setting_read_stats();
        let _guard = HookGuard;
        KEY_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(|| panic!("unadmitted key copy"))));
        with_encoded_settings_test_limits(
            EncodedSettingsLimits {
                bytes: required - 1,
                ..Default::default()
            },
            || -> Result<()> {
                assert!(
                    store
                        .first_setting_key_with_prefix("p:", None)
                        .unwrap_err()
                        .downcast_ref::<CachedResourceLimit>()
                        .is_some()
                );
                assert_eq!(
                    store.encoded_setting_read_stats(),
                    EncodedSettingReadStats::default()
                );
                Ok(())
            },
        )?;
    }
    Ok(())
}

#[test]
fn invalid_prefix_cursor_and_unavailable_key_never_become_absence() -> Result<()> {
    let (_directory, store) = store("UTF-8")?;
    let outer = store.connection.unchecked_transaction()?;
    for (prefix, after) in [
        ("", None),
        ("p:\0", None),
        ("p:", Some("other:marker")),
        ("p:", Some("p:\0marker")),
    ] {
        let error = store
            .first_setting_key_with_prefix(prefix, after)
            .unwrap_err();
        assert!(error.downcast_ref::<rusqlite::Error>().is_none());
        assert!(!store.connection.is_autocommit());
    }
    outer.rollback()?;
    for key in ["p:\0invalid".to_owned(), format!("p:{}", "k".repeat(4097))] {
        store.connection.execute("DELETE FROM settings", [])?;
        store
            .connection
            .execute("INSERT INTO settings(key,value) VALUES (?1,'null')", [&key])?;
        assert!(
            store
                .first_setting_key_with_prefix("p:", None)
                .unwrap_err()
                .downcast_ref::<CachedResourceLimit>()
                .is_some()
        );
        assert_eq!(
            store.encoded_setting_read_stats(),
            EncodedSettingReadStats::default()
        );
    }
    store.connection.execute("DELETE FROM settings", [])?;
    store
        .connection
        .execute("INSERT INTO settings(key,value) VALUES ('','null')", [])?;
    assert!(
        store
            .first_setting_key_with_prefix("p:", None)
            .unwrap_err()
            .downcast_ref::<CachedResourceLimit>()
            .is_some()
    );
    Ok(())
}

#[test]
fn key_snapshot_and_sql_failure_preserve_original_encoded_rows() -> Result<()> {
    let (directory, store) = store("UTF-8")?;
    store
        .connection
        .execute("INSERT INTO settings(key,value) VALUES ('p:old',x'80')", [])?;
    let other = Connection::open(directory.path().join("encoded.sqlite"))?;
    let _guard = HookGuard;
    KEY_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            other.execute("UPDATE settings SET key='p:new' WHERE key='p:old'", [])?;
            Ok(())
        }))
    });
    assert_eq!(
        store.first_setting_key_with_prefix("p:", None)?,
        Some("p:old".into())
    );
    assert_eq!(
        store.first_setting_key_with_prefix("p:", None)?,
        Some("p:new".into())
    );
    store
        .connection
        .execute_batch("ALTER TABLE settings RENAME TO retained_settings")?;
    let error = store.first_setting_key_with_prefix("p:", None).unwrap_err();
    store
        .connection
        .execute_batch("ALTER TABLE retained_settings RENAME TO settings")?;
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
    assert!(error.downcast_ref::<CachedResourceLimit>().is_none());
    assert_eq!(
        store.connection.query_row(
            "SELECT typeof(value) FROM settings WHERE key='p:new'",
            [],
            |row| row.get::<_, String>(0)
        )?,
        "blob"
    );
    Ok(())
}

#[test]
fn current_report_maintenance_uses_default_envelope_while_allocation_stays_refused() -> Result<()> {
    let (_directory, store) = store("UTF-8")?;
    let removed = seed(&store)?;
    store.connection.execute(
        "INSERT INTO settings(key,value) VALUES ('report.outbox.v1','{\"old\":1}')",
        [],
    )?;
    let original = snapshot(&store)?;
    let encoded = "{\"batches_v2\":[],\"reported_cursors\":{},\"compacted_runs\":{}}";
    with_encoded_settings_test_limits(
        EncodedSettingsLimits {
            bytes: 32,
            ..Default::default()
        },
        || -> Result<()> {
            assert!(
                store
                    .set_encoded_settings_and_delete(&[("report.outbox.v1", encoded)], &removed)
                    .unwrap_err()
                    .downcast_ref::<CachedResourceLimit>()
                    .is_some()
            );
            assert_eq!(snapshot(&store)?, original);
            store.set_report_outbox_encoded_and_delete(encoded, &removed)?;
            assert_eq!(
                store.connection.query_row(
                    "SELECT value FROM settings WHERE key='report.outbox.v1'",
                    [],
                    |row| row.get::<_, String>(0)
                )?,
                encoded
            );
            for key in &removed {
                assert_eq!(
                    store.connection.query_row(
                        "SELECT COUNT(*) FROM settings WHERE key=?1",
                        [key],
                        |row| row.get::<_, i64>(0)
                    )?,
                    0
                );
            }
            assert_eq!(
                store.setting::<String>("account")?,
                Some("synthetic-account".into())
            );
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn report_maintenance_scope_and_cleanup_sql_failure_keep_all_old_records() -> Result<()> {
    let (_directory, store) = store("UTF-8")?;
    let removed = seed(&store)?;
    store.connection.execute(
        "INSERT INTO settings(key,value) VALUES ('report.outbox.v1','{\"old\":1}')",
        [],
    )?;
    let original = snapshot(&store)?;
    for key in [
        "preferences",
        "report.receipt:",
        "report.receipt: \u{3000}",
        "report.outbox.v1",
    ] {
        let error = store
            .set_report_outbox_encoded_and_delete("{\"new\":2}", &[key.into()])
            .unwrap_err();
        assert!(error.downcast_ref::<rusqlite::Error>().is_none());
        assert_eq!(snapshot(&store)?, original);
    }
    assert!(
        store
            .commit_encoded_settings_and_delete::<&str>(
                &[("account", "null")],
                &[],
                EncodedSettingsLimits::default(),
                true
            )
            .is_err()
    );
    store.connection.execute_batch("CREATE TEMP TRIGGER reject_report_cleanup BEFORE DELETE ON settings
        WHEN OLD.key='report.failure:session' BEGIN SELECT RAISE(ABORT,'fixture report cleanup failure'); END")?;
    let error = with_encoded_settings_test_limits(
        EncodedSettingsLimits {
            bytes: 32,
            ..Default::default()
        },
        || store.set_report_outbox_encoded_and_delete("{\"new\":2}", &removed),
    )
    .unwrap_err();
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
    assert_eq!(snapshot(&store)?, original);
    Ok(())
}
