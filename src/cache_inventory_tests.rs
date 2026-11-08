//! Producer-backed inventory admission and mutation-fence regressions.

use super::*;

type PreflightHook = Box<dyn FnOnce() -> Result<()>>;
thread_local! {
    static PREFLIGHT_HOOK: RefCell<Option<PreflightHook>> = const { RefCell::new(None) };
}

pub(super) fn after_preflight() -> Result<()> {
    let hook = PREFLIGHT_HOOK.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook()?;
    }
    Ok(())
}

fn with_preflight_hook<T>(hook: PreflightHook, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<PreflightHook>);
    impl Drop for Restore {
        fn drop(&mut self) {
            PREFLIGHT_HOOK.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(PREFLIGHT_HOOK.with(|slot| slot.borrow_mut().replace(hook)));
    run()
}

fn fixture(maximum: u64) -> Result<(tempfile::TempDir, FileCache)> {
    let directory = tempfile::tempdir()?;
    let cache = FileCache::open(directory.path(), maximum)?;
    Ok((directory, cache))
}

#[derive(Debug, PartialEq, Eq)]
struct DurableState {
    entries: Vec<(String, String, i64, i64, i64)>,
    pending: Vec<(String, i64)>,
    payloads: BTreeMap<String, Vec<u8>>,
}

fn state(cache: &FileCache) -> Result<DurableState> {
    let entries = cache
        .connection
        .prepare("SELECT key,filename,size,last_access,pinned FROM cache_entries ORDER BY key")?
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let pending = cache
        .connection
        .prepare("SELECT filename,size FROM cache_pending_cleanup ORDER BY filename")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut payloads = BTreeMap::new();
    for item in fs::read_dir(cache.root())? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        if valid_cleanup_filename(&name) && item.file_type()?.is_file() {
            payloads.insert(name, fs::read(item.path())?);
        }
    }
    Ok(DurableState {
        entries,
        pending,
        payloads,
    })
}

fn quota(error: &anyhow::Error) -> &InventoryLimitExceeded {
    error
        .downcast_ref::<InventoryLimitExceeded>()
        .expect("Expected an inventory capacity error")
}

fn transparent_png(seed: u8) -> Result<Vec<u8>> {
    let image =
        image::RgbaImage::from_pixel(1, 1, image::Rgba([seed, seed.wrapping_add(1), 93, 0]));
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image).write_to(&mut output, image::ImageFormat::Png)?;
    Ok(output.into_inner())
}

#[test]
fn tiny_valid_png_puts_have_a_real_finite_payload_floor_without_pricing_inventory() -> Result<()> {
    let (_directory, cache) = fixture(crate::book_assets::DEFAULT_IMAGE_BUDGET)?;
    let mut original_digests = BTreeSet::new();
    let mut total = 0_u64;
    let mut smallest = u64::MAX;
    for seed in 0..12 {
        let png = transparent_png(seed)?;
        assert_eq!(image::load_from_memory(&png)?.width(), 1);
        let id = digest(&png);
        assert!(original_digests.insert(id.clone()));
        let key = format!("book-image:v1:{id}");
        let path = cache.put_png(&key, &png)?;
        assert_eq!(fs::read(&path)?, png);
        let bytes = fs::metadata(path)?.len();
        smallest = smallest.min(bytes);
        total += bytes;
    }
    assert_eq!(cache.keys()?.len(), 12);
    assert_eq!(cache.usage()?, total);
    assert!(cache.maximum_bytes() / smallest > InventoryLimits::default().rows as u64);
    let before = state(&cache)?;
    cache.reset_inventory_read_stats();
    let error = with_inventory_test_limits(
        InventoryLimits {
            rows: 11,
            ..Default::default()
        },
        || cache.keys(),
    )
    .unwrap_err();
    assert_eq!(quota(&error).resource, "rows");
    assert_eq!(cache.inventory_read_stats().key_copies, 0);
    assert_eq!(state(&cache)?, before);
    Ok(())
}

#[test]
fn metadata_and_work_refusal_precede_inventory_copies_and_every_mutation() -> Result<()> {
    let (_directory, cache) = fixture(4096)?;
    cache.put("first", b"small")?;
    cache.put("second", b"other")?;
    cache.set_pinned("first", true)?;
    let before = state(&cache)?;
    for limits in [
        InventoryLimits {
            metadata_bytes: 0,
            ..Default::default()
        },
        InventoryLimits {
            work_bytes: 0,
            ..Default::default()
        },
    ] {
        for operation in 0..4 {
            cache.reset_inventory_read_stats();
            let error = with_inventory_test_limits(limits, || match operation {
                0 => cache.keys().map(|_| ()),
                1 => cache.pinned_keys().map(|_| ()),
                2 => cache.clear_unpinned().map(|_| ()),
                _ => cache.recover_with_verification_budget(cache.verification_work_limit()),
            })
            .unwrap_err();
            quota(&error);
            let reads = cache.inventory_read_stats();
            assert_eq!(reads.entry_copies, 0);
            assert_eq!(reads.key_copies, 0);
            assert_eq!(reads.filename_copies, 0);
            assert_eq!(reads.directory_copies, 0);
            assert_eq!(reads.inspections, 0);
            assert_eq!(state(&cache)?, before);
        }
    }
    Ok(())
}

#[test]
fn key_collection_exact_and_one_less_metadata_allowances_gate_before_copy() -> Result<()> {
    let (_directory, cache) = fixture(4096)?;
    cache.put("plain", b"small")?;
    cache.put("unicode:中文💡", b"other")?;
    cache.reset_inventory_read_stats();
    let expected = cache.keys()?;
    let exact = cache.inventory_read_stats().metadata_bytes;
    assert!(exact > 0);
    let before = state(&cache)?;
    cache.reset_inventory_read_stats();
    let error = with_inventory_test_limits(
        InventoryLimits {
            metadata_bytes: exact - 1,
            ..Default::default()
        },
        || cache.keys(),
    )
    .unwrap_err();
    assert_eq!(quota(&error).requested, exact);
    assert_eq!(cache.inventory_read_stats().key_copies, 0);
    cache.reset_inventory_read_stats();
    assert_eq!(
        with_inventory_test_limits(
            InventoryLimits {
                metadata_bytes: exact,
                ..Default::default()
            },
            || cache.keys()
        )?,
        expected
    );
    assert_eq!(cache.inventory_read_stats().key_copies, 2);
    assert_eq!(state(&cache)?, before);
    Ok(())
}

#[test]
fn key_collection_exact_and_one_less_cumulative_work_allowances_gate_before_copy() -> Result<()> {
    let (_directory, cache) = fixture(4096)?;
    cache.put("one", b"small")?;
    cache.put("two", b"other")?;
    cache.reset_inventory_read_stats();
    let expected = cache.keys()?;
    let exact = cache.inventory_read_stats().work_bytes;
    let before = state(&cache)?;
    cache.reset_inventory_read_stats();
    let error = with_inventory_test_limits(
        InventoryLimits {
            work_bytes: exact - 1,
            ..Default::default()
        },
        || cache.keys(),
    )
    .unwrap_err();
    assert_eq!(quota(&error).requested, exact);
    assert_eq!(cache.inventory_read_stats().key_copies, 0);
    assert_eq!(
        with_inventory_test_limits(
            InventoryLimits {
                work_bytes: exact,
                ..Default::default()
            },
            || cache.keys()
        )?,
        expected
    );
    assert_eq!(state(&cache)?, before);
    Ok(())
}

#[test]
fn eviction_and_new_put_refusal_preserve_prior_payloads_pins_and_cleanup_debt() -> Result<()> {
    let (_directory, cache) = fixture(9)?;
    cache.put("one", b"111")?;
    cache.put("two", b"222")?;
    cache.put("pin", b"333")?;
    cache.set_pinned("pin", true)?;
    let before = state(&cache)?;
    cache.reset_inventory_read_stats();
    let error = with_inventory_test_limits(
        InventoryLimits {
            rows: 1,
            ..Default::default()
        },
        || cache.put("next", b"444"),
    )
    .unwrap_err();
    assert_eq!(quota(&error).resource, "rows");
    assert_eq!(cache.inventory_read_stats().entry_copies, 0);
    assert_eq!(state(&cache)?, before);
    let path = cache.put("next", b"444")?;
    assert_eq!(fs::read(path)?, b"444");
    assert_eq!(cache.pinned_state("pin")?, Some(true));
    assert!(cache.get_for_commit("one")?.is_none());
    assert_eq!(cache.usage()?, 9);
    Ok(())
}

#[test]
fn key_only_and_pin_only_queries_do_not_read_unneeded_filename_fields() -> Result<()> {
    let (_directory, cache) = fixture(4096)?;
    cache.put("kept", b"small")?;
    cache.set_pinned("kept", true)?;
    cache.connection.execute(
        "UPDATE cache_entries SET filename=zeroblob(2048) WHERE key='kept'",
        [],
    )?;
    cache.reset_inventory_read_stats();
    assert_eq!(cache.keys()?, vec!["kept"]);
    assert_eq!(cache.pinned_keys()?, vec!["kept"]);
    assert_eq!(cache.inventory_read_stats().filename_copies, 0);
    let before = cache.connection.total_changes();
    let error = cache.recover_with_verification_budget(8192).unwrap_err();
    assert!(error.downcast_ref::<InventoryUnavailable>().is_some());
    assert_eq!(cache.connection.total_changes(), before);
    Ok(())
}

#[test]
fn utf16_keys_pay_conversion_scratch_and_preserve_original_unicode() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let index = directory.path().join("cache-index.sqlite3");
    let connection = Connection::open(&index)?;
    connection.pragma_update(None, "encoding", "UTF-16le")?;
    connection.execute_batch("CREATE TABLE encoding_seed(value INTEGER)")?;
    drop(connection);
    let cache = FileCache::open(directory.path(), 4096)?;
    let key = "unicode:中文💡";
    cache.put(key, b"small")?;
    cache.set_pinned(key, true)?;
    cache.reset_inventory_read_stats();
    assert_eq!(cache.keys()?, vec![key]);
    let exact = cache.inventory_read_stats().metadata_bytes;
    assert!(cache.inventory_read_stats().peak_scratch_bytes > key.len());
    let before = state(&cache)?;
    cache.reset_inventory_read_stats();
    let error = with_inventory_test_limits(
        InventoryLimits {
            metadata_bytes: exact - 1,
            ..Default::default()
        },
        || cache.keys(),
    )
    .unwrap_err();
    quota(&error);
    assert_eq!(cache.inventory_read_stats().key_copies, 0);
    assert_eq!(state(&cache)?, before);
    cache.recover_with_verification_budget(8192)?;
    assert_eq!(cache.pinned_state(key)?, Some(true));
    assert_eq!(state(&cache)?, before);
    Ok(())
}

#[test]
fn inventory_snapshot_never_copies_a_late_larger_key_set() -> Result<()> {
    let (_directory, cache) = fixture(4096)?;
    cache.put("before", b"small")?;
    let writer = Connection::open(cache.root().join("cache-index.sqlite3"))?;
    let filename = cache.entry("before")?.unwrap().filename;
    cache.reset_inventory_read_stats();
    let keys = with_preflight_hook(
        Box::new(move || {
            writer.execute("INSERT INTO cache_entries(key,filename,size,last_access,pinned) VALUES (?1,?2,5,2,0)",
            params!["later-key".repeat(16), filename])?;
            Ok(())
        }),
        || cache.keys(),
    )?;
    assert_eq!(keys, vec!["before"]);
    assert_eq!(cache.inventory_read_stats().key_copies, 1);
    assert_eq!(cache.keys()?.len(), 2);
    Ok(())
}

#[test]
fn external_inventory_change_refuses_clear_before_any_owned_retirement() -> Result<()> {
    let (_directory, cache) = fixture(4096)?;
    let path = cache.put("before", b"small")?;
    let writer = Connection::open(cache.root().join("cache-index.sqlite3"))?;
    let filename = cache.entry("before")?.unwrap().filename;
    let own_changes = cache.connection.total_changes();
    let result = with_preflight_hook(
        Box::new(move || {
            writer.execute("INSERT INTO cache_entries(key,filename,size,last_access,pinned) VALUES ('later',?1,5,2,0)", [filename])?;
            Ok(())
        }),
        || cache.clear_unpinned(),
    );
    assert!(result.is_err());
    assert_eq!(cache.connection.total_changes(), own_changes);
    assert_eq!(cache.keys()?.len(), 2);
    assert_eq!(fs::read(path)?, b"small");
    assert_eq!(state(&cache)?.pending, Vec::<(String, i64)>::new());
    Ok(())
}

#[test]
fn late_legacy_directory_quota_refusal_keeps_all_prior_files_and_index_rows() -> Result<()> {
    let (_directory, cache) = fixture(4096)?;
    cache.put("live", b"kept")?;
    cache.set_pinned("live", true)?;
    for index in 0..3 {
        let key = format!("orphan-{index}");
        cache.put(&key, b"old")?;
        cache
            .connection
            .execute("DELETE FROM cache_entries WHERE key=?1", [&key])?;
    }
    let before = state(&cache)?;
    cache.reset_inventory_read_stats();
    let error = with_inventory_test_limits(
        InventoryLimits {
            rows: 1,
            ..Default::default()
        },
        || cache.recover_with_verification_budget(8192),
    )
    .unwrap_err();
    assert_eq!(quota(&error).resource, "rows");
    assert_eq!(cache.inventory_read_stats().directory_copies, 1);
    assert_eq!(state(&cache)?, before);
    cache.recover_with_verification_budget(8192)?;
    assert_eq!(state(&cache)?.payloads.len(), 1);
    assert_eq!(cache.pinned_state("live")?, Some(true));
    Ok(())
}

#[test]
fn legacy_removal_receipts_do_not_grow_the_shared_destination_scan_set() -> Result<()> {
    let (_directory, cache) = fixture(4096)?;
    for index in 0..24 {
        let key = format!("item-{index:02}");
        cache.put(&key, b"small")?;
        cache.set_pinned(&key, true)?;
    }
    let keys = cache.keys()?;
    let mut scope = VerificationScope::new(8192);
    let plan = cache.plan_removal(
        keys.iter().map(String::as_str),
        &mut cache.inventory_budget(),
        &mut scope,
    )?;
    assert!(scope.destinations.borrow().expected.is_empty());
    cache.apply_removal(&plan, &scope, |key| key < "item-12")?;
    assert_eq!(cache.keys()?.len(), 12);
    assert_eq!(cache.pinned_keys()?.len(), 12);
    assert_eq!(cache.usage()?, 60);
    assert!(scope.destinations.borrow().expected.is_empty());
    assert!(state(&cache)?.pending.is_empty());
    Ok(())
}

#[test]
fn a_replaced_captured_file_cannot_be_unpinned_retired_or_unlinked() -> Result<()> {
    let (_directory, cache) = fixture(4096)?;
    let path = cache.put("kept", b"small")?;
    cache.set_pinned("kept", true)?;
    let keys = vec!["kept".to_owned()];
    let mut scope = VerificationScope::new(8192);
    let plan = cache.plan_removal(
        keys.iter().map(String::as_str),
        &mut cache.inventory_budget(),
        &mut scope,
    )?;
    fs::write(&path, b"replacement")?;
    let before = state(&cache)?;
    assert!(cache.apply_removal(&plan, &scope, |_| true).is_err());
    assert_eq!(cache.pinned_state("kept")?, Some(true));
    assert_eq!(fs::read(path)?, b"replacement");
    assert_eq!(state(&cache)?, before);
    Ok(())
}
