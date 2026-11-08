//! Bounded manifest inventories and pre-mutation canonical-body recovery.

use super::*;
use crate::{
    cache::{InventoryLimits, with_inventory_test_limits},
    domain::{Annotation, Book, Chapter, ContentStatus},
};
use std::fs;

type BodyAdmissionHook = Box<dyn FnOnce() -> Result<()>>;
thread_local! {
    static OWNED_BODY_BASELINE: Cell<usize> = const { Cell::new(0) };
    static BODY_ADMISSION_HOOK: std::cell::RefCell<Option<BodyAdmissionHook>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn after_body_admission() -> Result<()> {
    let hook = BODY_ADMISSION_HOOK.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook()?;
    }
    Ok(())
}

fn with_body_admission_hook<T>(hook: BodyAdmissionHook, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<BodyAdmissionHook>);
    impl Drop for Restore {
        fn drop(&mut self) {
            BODY_ADMISSION_HOOK.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(BODY_ADMISSION_HOOK.with(|slot| slot.borrow_mut().replace(hook)));
    run()
}

#[derive(Debug, PartialEq, Eq)]
struct CacheState {
    entries: Vec<(String, String, i64, i64, i64)>,
    pending: Vec<(String, i64)>,
    files: BTreeMap<String, Vec<u8>>,
}

fn cache_state(assets: &BookAssetStore) -> Result<CacheState> {
    let connection = rusqlite::Connection::open(assets.cache.root().join("cache-index.sqlite3"))?;
    let entries = connection
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
    let pending = connection
        .prepare("SELECT filename,size FROM cache_pending_cleanup ORDER BY filename")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut files = BTreeMap::new();
    for item in fs::read_dir(assets.cache.root())? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        if (name.ends_with(".png") || name.starts_with(".tmp-")) && item.file_type()?.is_file() {
            files.insert(name, fs::read(item.path())?);
        }
    }
    Ok(CacheState {
        entries,
        pending,
        files,
    })
}

fn manifests(store: &Store) -> Result<Vec<(String, serde_json::Value)>> {
    store.settings_with_prefix(MANIFEST_PREFIX)
}

fn seed_history(store: &Store) -> Result<()> {
    let mut book = store.book("book")?.unwrap();
    book.intro = "Keep the introduction".into();
    book.progress = 0.25;
    store.upsert_book(&book)?;
    store.save_annotation(&Annotation {
        id: "retained-note".into(),
        book_id: "book".into(),
        start: 0,
        end: 6,
        quote: "Before".into(),
        note: "Keep the personal note".into(),
        created_at: 17,
        ..Default::default()
    })?;
    store.set_setting("inventory.fixture.account", &"owner")?;
    store.set_setting(
        "inventory.fixture.preferences",
        &vec!["Keep", "preferences"],
    )?;
    Ok(())
}

fn assert_history(store: &Store) -> Result<()> {
    let book = store.book("book")?.unwrap();
    assert_eq!(book.intro, "Keep the introduction");
    assert_eq!(book.progress, 0.25);
    assert_eq!(store.chapter_body("book", 0)?.as_deref(), Some(tests::TEXT));
    let notes = store.annotations("book")?;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].id, "retained-note");
    assert_eq!(notes[0].quote, "Before");
    assert_eq!(notes[0].note, "Keep the personal note");
    assert_eq!((notes[0].start, notes[0].end), (0, 6));
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

fn packet(count: usize) -> Result<DownloadedChapterImages> {
    let mut assets = Vec::new();
    let mut source_to_asset = BTreeMap::new();
    for index in 0..count {
        let source = format!("images/{index}.png");
        let asset = tests::asset(&source, index as u8)?;
        source_to_asset.insert(source, asset.id.clone());
        assets.push(asset);
    }
    Ok(DownloadedChapterImages {
        assets,
        source_to_asset,
    })
}

fn html(count: usize) -> String {
    let mut output = "<p>Before</p>".to_owned();
    for index in 0..count {
        output.push_str(&format!("<img src='images/{index}.png'>"));
    }
    output.push_str("<p>After</p>");
    output
}

fn reset(assets: &BookAssetStore, store: &Store) {
    assets.reset_inventory_read_stats();
    crate::store::reset_inventory_test_read_stats();
    OWNED_BODY_BASELINE.with(|slot| slot.set(store.chapter_body_read_count()));
}

fn assert_no_owned_body_read(store: &Store) {
    OWNED_BODY_BASELINE.with(|slot| {
        assert_eq!(
            store.chapter_body_read_count(),
            slot.get(),
            "The inventory operation must not materialize a canonical body"
        )
    });
}

fn quota(error: &anyhow::Error) -> &InventoryLimitExceeded {
    error
        .downcast_ref::<InventoryLimitExceeded>()
        .expect("Expected a cache inventory capacity error")
}

#[test]
fn transparent_original_digests_publish_distinct_real_keys_with_identical_tiny_white_pngs()
-> Result<()> {
    let (_directory, store, assets) = tests::setup(DEFAULT_IMAGE_BUDGET)?;
    let mut original_assets = Vec::new();
    let mut source_to_asset = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for seed in 0..12_u8 {
        let image =
            image::RgbaImage::from_pixel(1, 1, image::Rgba([seed, seed.wrapping_add(1), 137, 0]));
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image).write_to(&mut encoded, image::ImageFormat::Png)?;
        let bytes = encoded.into_inner();
        let source = format!("images/{seed}.png");
        let id = digest(&bytes);
        assert!(ids.insert(id.clone()));
        source_to_asset.insert(source.clone(), id.clone());
        original_assets.push(ImageAsset {
            id,
            bytes,
            media_type: "image/png".into(),
            extension: "png".into(),
            source_names: vec![source],
        });
    }
    let downloaded = DownloadedChapterImages {
        assets: original_assets,
        source_to_asset,
    };
    let manifest = assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(12),
        tests::TEXT,
        Some(&downloaded),
    )?;
    assert!(manifest.complete);
    let paths = assets.image_paths(&manifest.images)?;
    let mut unique_paths = BTreeSet::new();
    let mut white_png = None;
    let mut bytes_total = 0_u64;
    for path in paths.into_iter().map(Option::unwrap) {
        assert!(unique_paths.insert(path.clone()));
        let bytes = fs::read(&path)?;
        let raster = image::load_from_memory(&bytes)?.into_luma8();
        assert_eq!(raster.dimensions(), (1, 1));
        assert_eq!(raster.get_pixel(0, 0).0, [255]);
        if let Some(expected) = &white_png {
            assert_eq!(&bytes, expected);
        }
        bytes_total += bytes.len() as u64;
        white_png = Some(bytes);
    }
    assert_eq!(assets.cache.keys()?.len(), 12);
    assert_eq!(assets.cache.pinned_keys()?.len(), 12);
    assert_eq!(assets.usage()?, bytes_total);
    assert!(
        DEFAULT_IMAGE_BUDGET / white_png.unwrap().len() as u64
            > InventoryLimits::default().rows as u64
    );
    assets.recover(&store)?;
    assert!(assets.complete(&store, "book", "uid")?);
    Ok(())
}

#[test]
fn inventory_refusal_preserves_manifests_payloads_pins_notes_and_account_for_recover_and_clear()
-> Result<()> {
    let (_directory, store, assets) = tests::setup(DEFAULT_IMAGE_BUDGET)?;
    assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(2),
        tests::TEXT,
        Some(&packet(2)?),
    )?;
    seed_history(&store)?;
    let before_manifests = manifests(&store)?;
    let before_cache = cache_state(&assets)?;
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
        for operation in 0..3 {
            reset(&assets, &store);
            let error = with_inventory_test_limits(limits, || match operation {
                0 => assets.recover(&store),
                1 => assets.clear_book(&store, "book"),
                _ => assets.clear_all(&store),
            })
            .unwrap_err();
            quota(&error);
            let reads = crate::store::inventory_test_read_stats();
            assert_eq!(reads.key_copies, 0);
            assert_eq!(reads.manifest_borrows, 0);
            assert_eq!(reads.manifest_deserializations, 0);
            assert_eq!(reads.body_borrows, 0);
            assert_eq!(assets.inventory_read_stats().body_reads, 0);
            assert_eq!(assets.cache.inventory_read_stats().inspections, 0);
            assert_eq!(manifests(&store)?, before_manifests);
            assert_eq!(cache_state(&assets)?, before_cache);
            assert_history(&store)?;
        }
    }
    Ok(())
}

#[test]
fn actual_array_count_prices_selected_slots_before_deserialization_at_exact_and_one_less()
-> Result<()> {
    let (_directory, store, assets) = tests::setup(DEFAULT_IMAGE_BUDGET)?;
    assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(8),
        tests::TEXT,
        Some(&packet(8)?),
    )?;
    reset(&assets, &store);
    let index = assets.indices(&store)?;
    assert_eq!(index[0].1.images.len(), 8);
    let exact = assets.inventory_read_stats().projection_requested_bytes;
    assert_eq!(
        assets.inventory_read_stats().projection_admitted_bytes,
        exact
    );
    let before = cache_state(&assets)?;
    reset(&assets, &store);
    let error = with_inventory_test_limits(
        InventoryLimits {
            metadata_bytes: exact - 1,
            ..Default::default()
        },
        || assets.indices(&store),
    )
    .err()
    .expect("Selected slots must be refused before DTO materialization");
    assert_eq!(quota(&error).requested, exact);
    assert_eq!(assets.inventory_read_stats().image_counts, 1);
    assert_eq!(
        crate::store::inventory_test_read_stats().manifest_borrows,
        1
    );
    assert_eq!(
        crate::store::inventory_test_read_stats().manifest_deserializations,
        0
    );
    assert_eq!(assets.inventory_read_stats().manifest_reads, 0);
    assert_eq!(assets.inventory_read_stats().body_reads, 0);
    reset(&assets, &store);
    let index = with_inventory_test_limits(
        InventoryLimits {
            metadata_bytes: exact,
            ..Default::default()
        },
        || assets.indices(&store),
    )?;
    assert_eq!(index.len(), 1);
    assert_eq!(index[0].1.images.len(), 8);
    assert_eq!(
        crate::store::inventory_test_read_stats().manifest_deserializations,
        1
    );
    assert_eq!(cache_state(&assets)?, before);
    Ok(())
}

#[test]
fn reference_projection_skips_historical_html_and_rejects_before_any_full_manifest_read()
-> Result<()> {
    let (_directory, store, assets) = tests::setup(DEFAULT_IMAGE_BUDGET)?;
    let manifest = assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(1),
        tests::TEXT,
        Some(&packet(1)?),
    )?;
    let key = manifest_key("book", "uid")?;
    let mut historical = serde_json::to_value(&manifest)?;
    historical["html"] = serde_json::Value::String("<p>Historical view</p>".repeat(256));
    store.set_setting(&key, &historical)?;
    reset(&assets, &store);
    let index = assets.indices(&store)?;
    assert_eq!(index.len(), 1);
    assert_eq!(index[0].1.images, manifest.images);
    assert_eq!(assets.inventory_read_stats().full_manifest_reads, 0);
    assert_eq!(
        crate::store::inventory_test_read_stats().manifest_borrows,
        2
    );
    assert_eq!(
        crate::store::inventory_test_read_stats().manifest_deserializations,
        1
    );
    assert_eq!(crate::store::inventory_test_read_stats().body_borrows, 0);
    let before = cache_state(&assets)?;
    reset(&assets, &store);
    let error = with_inventory_test_limits(
        InventoryLimits {
            work_bytes: 1,
            ..Default::default()
        },
        || assets.recover(&store),
    )
    .unwrap_err();
    quota(&error);
    assert_eq!(
        crate::store::inventory_test_read_stats().manifest_borrows,
        0
    );
    assert_eq!(assets.inventory_read_stats().full_manifest_reads, 0);
    assert_eq!(store.setting::<serde_json::Value>(&key)?, Some(historical));
    assert_eq!(cache_state(&assets)?, before);
    Ok(())
}

#[test]
fn zero_body_verification_allowance_rejects_before_borrow_hash_or_cache_repair() -> Result<()> {
    let (_directory, store, assets) = tests::setup(DEFAULT_IMAGE_BUDGET)?;
    let manifest = assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(1),
        tests::TEXT,
        Some(&packet(1)?),
    )?;
    seed_history(&store)?;
    let key = manifest.images[0].cache_key.as_ref().unwrap();
    let path = assets.cache.get_for_commit(key)?.unwrap();
    let length = fs::metadata(&path)?.len() as usize;
    fs::write(&path, vec![b'x'; length])?;
    let before_manifests = manifests(&store)?;
    let before_cache = cache_state(&assets)?;
    reset(&assets, &store);
    let error = assets
        .recover_with_verification_budget(&store, 0)
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<crate::cache::VerificationLimitExceeded>()
            .is_some()
    );
    assert_eq!(
        assets.verification_stats.get(),
        crate::cache::VerificationStats::default()
    );
    assert_eq!(assets.inventory_read_stats().body_reads, 0);
    assert_eq!(crate::store::inventory_test_read_stats().body_borrows, 0);
    assert_no_owned_body_read(&store);
    assert_eq!(manifests(&store)?, before_manifests);
    assert_eq!(cache_state(&assets)?, before_cache);
    assert_history(&store)?;
    Ok(())
}

#[test]
fn a_file_changed_after_inventory_admission_is_never_adopted_as_retirable_corruption() -> Result<()>
{
    let (_directory, store, assets) = tests::setup(DEFAULT_IMAGE_BUDGET)?;
    let manifest = assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(1),
        tests::TEXT,
        Some(&packet(1)?),
    )?;
    seed_history(&store)?;
    let key = manifest.images[0].cache_key.as_ref().unwrap();
    let path = assets.cache.get_for_commit(key)?.unwrap();
    let replacement_path = path.clone();
    let before = cache_state(&assets)?;
    let before_manifests = manifests(&store)?;
    reset(&assets, &store);
    let error = with_body_admission_hook(
        Box::new(move || {
            fs::write(&replacement_path, b"foreign replacement")?;
            Ok(())
        }),
        || assets.recover(&store),
    )
    .unwrap_err();
    assert!(
        error
            .downcast_ref::<crate::cache::InventoryUnavailable>()
            .is_some()
    );
    assert_eq!(assets.verification_stats.get().hashes, 0);
    assert_eq!(
        assets.verification_stats.get().charged_bytes,
        tests::TEXT.len() as u64
    );
    assert_no_owned_body_read(&store);
    let after = cache_state(&assets)?;
    assert_eq!(after.entries, before.entries);
    assert_eq!(after.pending, before.pending);
    assert_eq!(fs::read(path)?, b"foreign replacement");
    assert_eq!(manifests(&store)?, before_manifests);
    assert_history(&store)?;
    Ok(())
}

#[test]
fn normal_shared_recovery_borrows_each_unique_body_and_hashes_one_physical_image() -> Result<()> {
    let (_directory, store, assets) = tests::setup(DEFAULT_IMAGE_BUDGET)?;
    let downloaded = packet(1)?;
    assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(1),
        tests::TEXT,
        Some(&downloaded),
    )?;
    store.upsert_chapter(&Chapter {
        id: "shared".into(),
        book_id: "book".into(),
        index: 1,
        body: Some(tests::TEXT.into()),
        ..Default::default()
    })?;
    assets.commit_chapter(
        &store,
        "book",
        "shared",
        &html(1),
        tests::TEXT,
        Some(&downloaded),
    )?;
    seed_history(&store)?;
    let before = cache_state(&assets)?;
    reset(&assets, &store);
    assets.recover(&store)?;
    assert_eq!(assets.inventory_read_stats().body_metadata, 2);
    assert_eq!(assets.inventory_read_stats().body_reads, 2);
    assert_eq!(
        assets.inventory_read_stats().body_bytes,
        2 * tests::TEXT.len()
    );
    assert_eq!(crate::store::inventory_test_read_stats().body_borrows, 2);
    assert_no_owned_body_read(&store);
    assert_eq!(assets.verification_stats.get().hashes, 1);
    let image_size = before.entries[0].2 as u64;
    assert_eq!(assets.verification_stats.get().read_bytes, image_size);
    assert_eq!(
        assets.verification_stats.get().charged_bytes,
        2 * tests::TEXT.len() as u64 + image_size + 1
    );
    assert_eq!(cache_state(&assets)?, before);
    assert_history(&store)?;
    Ok(())
}

#[test]
fn null_empty_and_nul_body_evidence_keep_distinct_completion_semantics() -> Result<()> {
    let (directory, store, assets) = tests::setup(DEFAULT_IMAGE_BUDGET)?;
    for (index, uid, body) in [
        (1, "null", None),
        (2, "empty", Some("")),
        (3, "nul", Some("A\0B")),
    ] {
        store.upsert_chapter(&Chapter {
            id: uid.into(),
            book_id: "book".into(),
            index,
            body: body.filter(|value| !value.is_empty()).map(str::to_owned),
            content_status: ContentStatus::Full,
            ..Default::default()
        })?;
        if body == Some("") {
            // A legacy TEXT empty value is a present digest input. It does not
            // make a newly downloaded empty body a valid readable cache entry.
            rusqlite::Connection::open(directory.path().join("metadata.sqlite3"))?.execute(
                "UPDATE chapters SET body='',content_status='full' WHERE book_id='book' AND chapter_index=?1", [index],
            )?;
        }
        assets.commit_chapter(
            &store,
            "book",
            uid,
            "<p></p>",
            body.unwrap_or_default(),
            None,
        )?;
    }
    reset(&assets, &store);
    assets.recover(&store)?;
    assert_eq!(assets.inventory_read_stats().body_metadata, 3);
    assert_eq!(assets.inventory_read_stats().body_reads, 2);
    assert_eq!(assets.inventory_read_stats().body_bytes, 3);
    assert_no_owned_body_read(&store);
    assert!(
        !assets
            .raw_chapter(&store, "book", "null")?
            .unwrap()
            .complete
    );
    assert!(
        assets
            .raw_chapter(&store, "book", "empty")?
            .unwrap()
            .complete
    );
    assert!(assets.raw_chapter(&store, "book", "nul")?.unwrap().complete);
    assert_eq!(store.chapter_body("book", 1)?, None);
    assert_eq!(store.chapter_body("book", 2)?, Some(String::new()));
    assert_eq!(store.chapter_body("book", 3)?.as_deref(), Some("A\0B"));
    Ok(())
}

#[test]
fn ambiguous_uid_body_evidence_refuses_before_cache_mutation_or_false_completion() -> Result<()> {
    let (_directory, store, assets) = tests::setup(DEFAULT_IMAGE_BUDGET)?;
    assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(1),
        tests::TEXT,
        Some(&packet(1)?),
    )?;
    store.upsert_chapter(&Chapter {
        id: "uid".into(),
        book_id: "book".into(),
        index: 1,
        body: Some("Different body".into()),
        ..Default::default()
    })?;
    let before_manifests = manifests(&store)?;
    let before_cache = cache_state(&assets)?;
    reset(&assets, &store);
    let error = assets.recover(&store).unwrap_err();
    assert!(
        error
            .downcast_ref::<crate::store::StoreInventoryUnavailable>()
            .is_some()
    );
    assert_eq!(assets.inventory_read_stats().body_reads, 0);
    assert_eq!(crate::store::inventory_test_read_stats().body_borrows, 0);
    assert_eq!(assets.verification_stats.get().hashes, 0);
    assert_eq!(cache_state(&assets)?, before_cache);
    assert_eq!(manifests(&store)?, before_manifests);
    assert_eq!(store.chapter_body("book", 0)?.as_deref(), Some(tests::TEXT));
    assert_eq!(
        store.chapter_body("book", 1)?.as_deref(),
        Some("Different body")
    );
    Ok(())
}

#[test]
fn utf16_body_conversion_peak_is_prepaid_before_any_body_borrow() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let database = directory.path().join("metadata.sqlite3");
    let connection = rusqlite::Connection::open(&database)?;
    connection.pragma_update(None, "encoding", "UTF-16le")?;
    connection.execute_batch("CREATE TABLE encoding_seed(value INTEGER)")?;
    drop(connection);
    let store = Store::open(&database)?;
    store.upsert_book(&Book::new("book", "Book"))?;
    let body = "正文".repeat(512);
    store.upsert_chapter(&Chapter {
        id: "uid".into(),
        book_id: "book".into(),
        body: Some(body.clone()),
        ..Default::default()
    })?;
    let assets = BookAssetStore::open(directory.path().join("images"), DEFAULT_IMAGE_BUDGET)?;
    assets.commit_chapter(&store, "book", "uid", "<p></p>", &body, None)?;
    reset(&assets, &store);
    assets.recover(&store)?;
    let stats = assets.inventory_read_stats();
    assert_eq!(stats.body_reads, 1);
    assert_no_owned_body_read(&store);
    assert!(stats.peak_body_temporary > body.len());
    let before_manifests = manifests(&store)?;
    let before_cache = cache_state(&assets)?;
    reset(&assets, &store);
    let error = with_inventory_test_limits(
        InventoryLimits {
            metadata_bytes: stats.metadata_bytes - 1,
            ..Default::default()
        },
        || assets.recover(&store),
    )
    .unwrap_err();
    quota(&error);
    assert_eq!(assets.inventory_read_stats().body_reads, 0);
    assert_eq!(crate::store::inventory_test_read_stats().body_borrows, 0);
    assert_no_owned_body_read(&store);
    assert_eq!(manifests(&store)?, before_manifests);
    assert_eq!(cache_state(&assets)?, before_cache);
    assert_eq!(store.chapter_body("book", 0)?, Some(body));
    Ok(())
}

#[test]
fn selected_book_clear_does_not_read_target_bodies_or_remove_shared_assets_and_notes() -> Result<()>
{
    let (_directory, store, assets) = tests::setup(DEFAULT_IMAGE_BUDGET)?;
    let downloaded = packet(1)?;
    let manifest = assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(1),
        tests::TEXT,
        Some(&downloaded),
    )?;
    seed_history(&store)?;
    store.upsert_book(&Book::new("other", "Other"))?;
    store.upsert_chapter(&Chapter {
        id: "other-uid".into(),
        book_id: "other".into(),
        body: Some(tests::TEXT.into()),
        ..Default::default()
    })?;
    assets.commit_chapter(
        &store,
        "other",
        "other-uid",
        &html(1),
        tests::TEXT,
        Some(&downloaded),
    )?;
    let other = store.setting::<serde_json::Value>(&manifest_key("other", "other-uid")?)?;
    let path = assets
        .cache
        .get_for_commit(manifest.images[0].cache_key.as_ref().unwrap())?
        .unwrap();
    let before_png = fs::read(&path)?;
    reset(&assets, &store);
    assets.clear_book(&store, "book")?;
    assert_eq!(assets.inventory_read_stats().body_metadata, 1);
    assert_eq!(assets.inventory_read_stats().body_reads, 1);
    assert_no_owned_body_read(&store);
    assert!(assets.raw_chapter(&store, "book", "uid")?.is_none());
    assert_eq!(
        store.setting::<serde_json::Value>(&manifest_key("other", "other-uid")?)?,
        other
    );
    assert_eq!(fs::read(path)?, before_png);
    assert_eq!(assets.cache.pinned_keys()?.len(), 1);
    assert_history(&store)?;
    Ok(())
}

#[test]
fn a_known_manifest_commit_retains_success_when_admitted_old_asset_cleanup_hits_sql_error()
-> Result<()> {
    let (_directory, store, assets) = tests::setup(DEFAULT_IMAGE_BUDGET)?;
    let old = assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(1),
        tests::TEXT,
        Some(&packet(1)?),
    )?;
    seed_history(&store)?;
    let old_key = old.images[0].cache_key.as_ref().unwrap();
    let old_path = assets.cache.get_for_commit(old_key)?.unwrap();
    let old_png = fs::read(&old_path)?;
    let fault = rusqlite::Connection::open(assets.cache.root().join("cache-index.sqlite3"))?;
    fault.execute_batch(&format!(
        "CREATE TRIGGER refuse_retirement BEFORE DELETE ON cache_entries
        WHEN OLD.key='{old_key}' BEGIN SELECT RAISE(ABORT,'fixture retirement failure'); END;"
    ))?;
    let source = "images/new.png";
    let replacement = tests::asset(source, 171)?;
    let downloaded = DownloadedChapterImages {
        source_to_asset: BTreeMap::from([(source.into(), replacement.id.clone())]),
        assets: vec![replacement],
    };
    let new_html = "<p>Before</p><img src='images/new.png'><p>After</p>";
    let committed = assets.commit_chapter(
        &store,
        "book",
        "uid",
        new_html,
        tests::TEXT,
        Some(&downloaded),
    )?;
    assert!(committed.complete);
    assert_eq!(
        store.setting::<ChapterManifest>(&manifest_key("book", "uid")?)?,
        Some(committed.clone())
    );
    assert!(assets.reconcile_required.get());
    assert_eq!(fs::read(&old_path)?, old_png);
    assert_history(&store)?;
    fault.execute_batch("DROP TRIGGER refuse_retirement")?;
    assets.recover(&store)?;
    assert!(!old_path.exists());
    assert!(assets.complete(&store, "book", "uid")?);
    assert_history(&store)?;
    Ok(())
}
