//! Persisted-setting storage gates precede consumer field ownership.

use super::*;
use std::cell::{Cell, RefCell};

thread_local! {
    static BORROWS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
    static AFTER: RefCell<Option<Box<dyn FnOnce() -> Result<()>>>> = const { RefCell::new(None) };
}
pub(super) fn observe_borrow(bytes: usize) {
    BORROWS.with(|s| {
        let (n, b) = s.get();
        s.set((n + 1, b + bytes));
    });
}
pub(super) fn after_preflight() -> Result<()> {
    let action = AFTER.with(|s| s.borrow_mut().take());
    if let Some(action) = action {
        action()?;
    }
    Ok(())
}
fn reset() {
    BORROWS.with(|s| s.set((0, 0)));
}

#[test]
fn selected_setting_callback_borrows_raw_and_preserves_summary_device() -> Result<()> {
    let store = Store::open(":memory:")?;
    let raw =
        "{\"summary\":\"  Keep\\nsummary  \",\"device\":\" device \",\"ignored\":\"unrequested\"}";
    store.connection.execute(
        "INSERT INTO settings(key,value) VALUES('selected',?1)",
        [raw],
    )?;
    reset();
    let result = store
        .with_bounded_setting_text(
            "selected",
            StoredSettingReadLimits::default(),
            |text, meta| {
                assert_eq!(text, raw);
                assert_eq!(meta.encoded_bytes, raw.len());
                assert_eq!(meta.vm_scratch_bytes, raw.len() + 2);
                #[derive(serde::Deserialize)]
                struct Fields {
                    summary: String,
                    device: String,
                }
                // The genuine small fixture fits its consumer field/cell allowance.
                ensure!(meta.utf8_bytes_upper <= 1024, "fixture field capacity");
                let fields: Fields = serde_json::from_str(text)?;
                Ok((fields.summary, fields.device))
            },
        )?
        .unwrap();
    assert_eq!(result, ("  Keep\nsummary  ".into(), " device ".into()));
    assert_eq!(BORROWS.with(Cell::get), (1, raw.len()));
    assert_eq!(
        store.setting::<serde_json::Value>("selected")?,
        Some(serde_json::from_str(raw)?)
    );
    Ok(())
}

#[test]
fn stored_setting_exact_work_and_scratch_refuse_before_callback() -> Result<()> {
    let store = Store::open(":memory:")?;
    store.set_setting("key", &true)?;
    let raw = "true".len();
    let work = raw * 3 + "key".len() * 2;
    let scratch = raw * 3 + 258;
    for limits in [
        StoredSettingReadLimits {
            work_bytes: work - 1,
            ..Default::default()
        },
        StoredSettingReadLimits {
            scratch_bytes: scratch - 1,
            ..Default::default()
        },
    ] {
        reset();
        let called = Cell::new(false);
        let error = store
            .with_bounded_setting_text("key", limits, |_, _| {
                called.set(true);
                Ok(())
            })
            .unwrap_err();
        assert!(error.downcast_ref::<StoredSettingReadLimit>().is_some());
        assert!(!called.get());
        assert_eq!(BORROWS.with(Cell::get), (0, 0));
    }
    let value = store.with_bounded_setting_text(
        "key",
        StoredSettingReadLimits {
            work_bytes: work,
            scratch_bytes: scratch,
            ..Default::default()
        },
        |raw, _| Ok(serde_json::from_str::<bool>(raw)?),
    )?;
    assert_eq!(value, Some(true));
    assert_eq!(BORROWS.with(Cell::get), (1, 4));
    assert_eq!(store.setting::<bool>("key")?, Some(true));
    Ok(())
}

#[test]
fn stored_setting_utf16_invalid_type_and_missing_keep_distinct_results() -> Result<()> {
    for encoding in ["UTF-16le", "UTF-16be"] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("db");
        {
            let mut c = Connection::open(&path)?;
            c.pragma_update(None, "encoding", encoding)?;
            initialize_schema(&mut c)?;
        }
        let store = Store::open(&path)?;
        store.set_setting("key", &"  Unicode \u{4e2d}\0device  ")?;
        reset();
        let value = store.with_bounded_setting_text(
            "key",
            StoredSettingReadLimits::default(),
            |text, meta| {
                assert_eq!(meta.vm_scratch_bytes, 3 * meta.encoded_bytes + 2);
                Ok(serde_json::from_str::<String>(text)?)
            },
        )?;
        assert_eq!(value.as_deref(), Some("  Unicode \u{4e2d}\0device  "));
        store
            .connection
            .execute("UPDATE settings SET value=x'80' WHERE key='key'", [])?;
        reset();
        assert!(
            store
                .with_bounded_setting_text("key", StoredSettingReadLimits::default(), |_, _| Ok(()))
                .unwrap_err()
                .downcast_ref::<StoredSettingUnavailable>()
                .is_some()
        );
        assert_eq!(BORROWS.with(Cell::get), (0, 0));
        assert_eq!(
            store.with_bounded_setting_text(
                "missing",
                StoredSettingReadLimits::default(),
                |_, _| Ok(())
            )?,
            None
        );
        assert_eq!(
            store.connection.query_row(
                "SELECT typeof(value) FROM settings WHERE key='key'",
                [],
                |r| r.get::<_, String>(0)
            )?,
            "blob"
        );
    }
    Ok(())
}

#[test]
fn stored_setting_transaction_retains_external_snapshot_and_rejects_own_change() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("db");
    let store = std::rc::Rc::new(Store::open(&path)?);
    store.set_setting("key", &"old")?;
    let external = Connection::open(&path)?;
    AFTER.with(|s| {
        *s.borrow_mut() = Some(Box::new(move || {
            external.execute("UPDATE settings SET value='\"new\"' WHERE key='key'", [])?;
            Ok(())
        }))
    });
    assert_eq!(
        store
            .with_bounded_setting_text("key", StoredSettingReadLimits::default(), |raw, _| Ok(
                serde_json::from_str::<String>(raw)?
            ))?
            .as_deref(),
        Some("old")
    );
    assert_eq!(store.setting::<String>("key")?.as_deref(), Some("new"));
    let snapshot = store.resource_verification_snapshot()?;
    let writer = store.clone();
    AFTER.with(|s| *s.borrow_mut() = Some(Box::new(move || writer.set_setting("key", &"own"))));
    reset();
    assert!(
        snapshot
            .with_bounded_setting_text("key", StoredSettingReadLimits::default(), |_, _| Ok(()))
            .unwrap_err()
            .downcast_ref::<StoredSettingUnavailable>()
            .is_some()
    );
    assert_eq!(BORROWS.with(Cell::get), (0, 0));
    drop(snapshot);
    assert_eq!(store.setting::<String>("key")?.as_deref(), Some("new"));
    Ok(())
}
