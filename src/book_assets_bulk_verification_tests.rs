//! Real multi-chapter completeness admission, shared payload proofs and failure snapshots.

use super::*;
use crate::domain::{Annotation, Bookmark, Chapter};
use std::{cell::Cell, fs};

std::thread_local! {
    static LIMITS: Cell<Option<BulkVerificationLimits>> = const { Cell::new(None) };
}

pub(super) fn configured_limits() -> Option<BulkVerificationLimits> {
    LIMITS.with(Cell::get)
}

struct LimitsGuard(Option<BulkVerificationLimits>);

impl LimitsGuard {
    fn new(limits: BulkVerificationLimits) -> Self {
        Self(LIMITS.with(|current| current.replace(Some(limits))))
    }
}

impl Drop for LimitsGuard {
    fn drop(&mut self) {
        LIMITS.with(|current| current.set(self.0));
    }
}

fn html(source: &str) -> String {
    format!("<p>Before</p><img src='{source}'><p>After</p>")
}

fn add_chapter(store: &Store, uid: &str, index: u32) -> Result<()> {
    store.upsert_chapter(&Chapter {
        id: uid.into(),
        book_id: "book".into(),
        index,
        title: format!("Bulk chapter {index}"),
        body: Some(tests::TEXT.into()),
        ..Default::default()
    })
}

fn publish(
    store: &Store,
    assets: &BookAssetStore,
    uid: &str,
    source: &str,
    seed: u8,
) -> Result<ChapterManifest> {
    let asset = tests::asset(source, seed)?;
    let packet = DownloadedChapterImages {
        source_to_asset: BTreeMap::from([(source.to_owned(), asset.id.clone())]),
        assets: vec![asset],
    };
    assets.commit_chapter(
        store,
        "book",
        uid,
        &html(source),
        tests::TEXT,
        Some(&packet),
    )
}

fn personal_state(store: &Store) -> Result<()> {
    store.save_annotation(&Annotation {
        id: "bulk-note".into(),
        book_id: "book".into(),
        chapter_index: 0,
        start: 0,
        end: 6,
        quote: "Before".into(),
        note: "Keep bulk verification note".into(),
        created_at: 1,
    })?;
    store.save_bookmark(&Bookmark {
        id: "bulk-bookmark".into(),
        book_id: "book".into(),
        chapter_index: 0,
        char_offset: 1,
        label: "Keep bulk marker".into(),
        created_at: 1,
    })?;
    Ok(())
}

fn assert_personal_state(store: &Store) -> Result<()> {
    assert_eq!(store.chapter_body("book", 0)?.as_deref(), Some(tests::TEXT));
    assert_eq!(
        store.annotations("book")?[0].note,
        "Keep bulk verification note"
    );
    assert_eq!(store.bookmarks("book")?[0].char_offset, 1);
    Ok(())
}

fn manifests(store: &Store, uids: &[&str]) -> Result<Vec<Option<serde_json::Value>>> {
    uids.iter()
        .map(|uid| store.setting(&manifest_key("book", uid)?))
        .collect()
}

fn access_sequence(assets: &BookAssetStore, key: &str) -> Result<i64> {
    Ok(
        rusqlite::Connection::open(assets.cache.root().join("cache-index.sqlite3"))?.query_row(
            "SELECT last_access FROM cache_entries WHERE key=?1",
            [key],
            |row| row.get(0),
        )?,
    )
}

fn debt(assets: &BookAssetStore) -> Result<Vec<(String, i64)>> {
    let connection = rusqlite::Connection::open(assets.cache.root().join("cache-index.sqlite3"))?;
    let mut statement =
        connection.prepare("SELECT filename,size FROM cache_pending_cleanup ORDER BY filename")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[test]
fn real_many_uid_completeness_hashes_one_shared_png_and_reproves_the_next_operation() -> Result<()>
{
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    personal_state(&store)?;
    let mut committed = Vec::new();
    for (index, uid) in ["uid", "second", "third"].into_iter().enumerate() {
        if index != 0 {
            add_chapter(&store, uid, index as u32)?;
        }
        committed.push(publish(&store, &assets, uid, "images/shared.png", 51)?);
    }
    let key = committed[0].images[0].cache_key.as_ref().unwrap();
    let path = assets.cache.get_for_commit(key)?.unwrap();
    let size = fs::metadata(&path)?.len();
    let before = access_sequence(&assets, key)?;
    let original = manifests(&store, &["uid", "second", "third"])?;
    let body_reads = store.chapter_body_read_count();
    let body_copies = (0..3)
        .map(|index| store.chapter_body_materialization_stats("book", index))
        .collect::<Vec<_>>();
    assert_eq!(
        assets.complete_many(&store, "book", &["uid", "second", "uid", "third"])?,
        vec![true; 4]
    );
    let first = assets.bulk_verification_stats();
    assert_eq!(first.scope_count, 1);
    assert_eq!(first.unique_chapters, 3);
    assert_eq!(first.manifest_reads, 3);
    assert_eq!(first.body_reads, 3);
    assert_eq!(first.lookups, 3);
    assert_eq!(first.unique_keys, 1);
    assert_eq!(first.hashes, 1);
    assert_eq!(first.read_bytes, size);
    assert_eq!(store.chapter_body_read_count(), body_reads + 3);
    for index in 0..3 {
        assert_eq!(
            store.chapter_body_materialization_stats("book", index as u32),
            (
                body_copies[index].0 + 1,
                body_copies[index].1 + tests::TEXT.len()
            )
        );
    }
    assert_eq!(access_sequence(&assets, key)?, before);
    assert_eq!(assets.cache.pinned_state(key)?, Some(true));
    assert_eq!(
        assets.complete_many(&store, "book", &["third", "second", "uid"])?,
        vec![true; 3]
    );
    let next = assets.bulk_verification_stats();
    assert_eq!(next.scope_count, 1);
    assert_eq!(next.hashes, 1);
    assert_eq!(next.read_bytes, size);
    assert_eq!(store.chapter_body_read_count(), body_reads + 6);
    assert_eq!(manifests(&store, &["uid", "second", "third"])?, original);
    assert_personal_state(&store)
}

#[test]
fn a_late_unique_payload_quota_preserves_corruption_pins_debt_and_all_durable_data() -> Result<()> {
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    personal_state(&store)?;
    let first = publish(&store, &assets, "uid", "images/a.png", 61)?;
    add_chapter(&store, "second", 1)?;
    publish(&store, &assets, "second", "images/b.png", 171)?;
    assert_eq!(
        assets.complete_many(&store, "book", &["uid", "second"])?,
        vec![true; 2]
    );
    let charged = assets.bulk_verification_stats().charged_bytes;
    let key = first.images[0].cache_key.as_ref().unwrap();
    let path = assets.cache.get_for_commit(key)?.unwrap();
    let damaged = vec![b'x'; fs::metadata(&path)?.len() as usize];
    fs::write(&path, &damaged)?;
    let cleanup_path = assets.cache.root().join(".tmp-99-1");
    fs::write(&cleanup_path, b"pending")?;
    rusqlite::Connection::open(assets.cache.root().join("cache-index.sqlite3"))?.execute(
        "INSERT INTO cache_pending_cleanup(filename,size) VALUES('.tmp-99-1',7)",
        [],
    )?;
    let original = manifests(&store, &["uid", "second"])?;
    let keys = assets.cache.keys()?;
    let pins = assets.cache.pinned_keys()?;
    let usage = assets.usage()?;
    let pending = debt(&assets)?;
    let error = assets
        .complete_many_with_verification_budget(&store, "book", &["uid", "second"], charged - 1)
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<crate::cache::VerificationLimitExceeded>()
            .is_some()
    );
    let stats = assets.bulk_verification_stats();
    assert_eq!(stats.scope_count, 1);
    assert_eq!(stats.hashes, 0);
    assert_eq!(stats.read_bytes, 0);
    assert_eq!(assets.cache.keys()?, keys);
    assert_eq!(assets.cache.pinned_keys()?, pins);
    assert_eq!(assets.usage()?, usage);
    assert_eq!(debt(&assets)?, pending);
    assert_eq!(fs::read(&path)?, damaged);
    assert_eq!(fs::read(&cleanup_path)?, b"pending");
    assert_eq!(manifests(&store, &["uid", "second"])?, original);
    assert_personal_state(&store)
}

#[test]
fn bulk_metadata_refusal_reads_no_manifest_body_or_png_and_keeps_shared_state() -> Result<()> {
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    personal_state(&store)?;
    publish(&store, &assets, "uid", "images/shared.png", 71)?;
    add_chapter(&store, "zz-late", 1)?;
    let mut second = publish(&store, &assets, "zz-late", "images/shared.png", 71)?;
    // Ignored HTML still enters the encoded-read forecast before any JSON or
    // selected ManifestIndex field can be copied from an earlier small cell.
    second.html = "Ignored bulk HTML ".repeat(8 * 1024);
    store.set_setting(&manifest_key("book", "zz-late")?, &second)?;
    let original = manifests(&store, &["uid", "zz-late"])?;
    let keys = assets.cache.keys()?;
    let pins = assets.cache.pinned_keys()?;
    let usage = assets.usage()?;
    let body_reads = store.chapter_body_read_count();
    for metadata_bytes in [1, 128 * 1024] {
        let _limits = LimitsGuard::new(BulkVerificationLimits {
            metadata_bytes,
            ..Default::default()
        });
        let error = assets
            .complete_many(&store, "book", &["uid", "zz-late"])
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<BulkVerificationLimitExceeded>()
                .is_some()
        );
        let stats = assets.bulk_verification_stats();
        assert_eq!(stats.manifest_reads, 0);
        assert_eq!(stats.body_reads, 0);
        assert_eq!(stats.hashes, 0);
        assert_eq!(store.chapter_body_read_count(), body_reads);
        assert_eq!(assets.cache.keys()?, keys);
        assert_eq!(assets.cache.pinned_keys()?, pins);
        assert_eq!(assets.usage()?, usage);
        assert_eq!(manifests(&store, &["uid", "zz-late"])?, original);
    }
    assert_personal_state(&store)
}

#[test]
fn encoded_cache_metadata_is_admitted_before_hashes_for_both_database_encodings() -> Result<()> {
    for encoding in ["UTF-8", "UTF-16le", "UTF-16be"] {
        let (directory, store, _initial_assets) = tests::setup(8 * 1024 * 1024)?;
        let root = directory.path().join("encoded-images");
        fs::create_dir_all(&root)?;
        let connection = rusqlite::Connection::open(root.join("cache-index.sqlite3"))?;
        connection.pragma_update(None, "encoding", encoding)?;
        // Persist the encoding before FileCache creates its real cache tables.
        connection.execute_batch("CREATE TABLE encoding_fixture(value TEXT)")?;
        drop(connection);
        let assets = BookAssetStore::open(&root, 8 * 1024 * 1024)?;
        assets.recover(&store)?;
        personal_state(&store)?;
        let manifest = publish(&store, &assets, "uid", "images/shared.png", 75)?;
        add_chapter(&store, "second", 1)?;
        publish(&store, &assets, "second", "images/shared.png", 75)?;
        assert_eq!(
            assets.complete_many(&store, "book", &["uid", "second"])?,
            vec![true; 2]
        );
        let admitted = assets.bulk_verification_stats().metadata_bytes;
        assert_eq!(assets.bulk_verification_stats().hashes, 1);
        let key = manifest.images[0].cache_key.as_ref().unwrap();
        let path = assets.cache.get_for_commit(key)?.unwrap();
        let damaged = vec![b'x'; fs::metadata(&path)?.len() as usize];
        fs::write(&path, &damaged)?;
        let original = manifests(&store, &["uid", "second"])?;
        let pins = assets.cache.pinned_keys()?;
        let usage = assets.usage()?;
        let pending = debt(&assets)?;
        let _limits = LimitsGuard::new(BulkVerificationLimits {
            metadata_bytes: admitted - 1,
            ..Default::default()
        });
        let error = assets
            .complete_many(&store, "book", &["uid", "second"])
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::cache::VerificationMetadataLimitExceeded>()
                .is_some()
        );
        assert_eq!(assets.bulk_verification_stats().hashes, 0);
        assert_eq!(assets.bulk_verification_stats().read_bytes, 0);
        assert_eq!(assets.cache.pinned_keys()?, pins);
        assert_eq!(assets.usage()?, usage);
        assert_eq!(debt(&assets)?, pending);
        assert_eq!(fs::read(&path)?, damaged);
        assert_eq!(manifests(&store, &["uid", "second"])?, original);
        assert_personal_state(&store)?;
    }
    Ok(())
}

#[test]
fn utf16_body_conversion_temporary_quota_is_prepaid_before_the_real_body_read() -> Result<()> {
    for encoding in ["UTF-16le", "UTF-16be"] {
        let directory = tempfile::tempdir()?;
        let metadata_path = directory.path().join("metadata.sqlite3");
        let connection = rusqlite::Connection::open(&metadata_path)?;
        connection.pragma_update(None, "encoding", encoding)?;
        connection.execute_batch("CREATE TABLE encoding_fixture(value TEXT)")?;
        drop(connection);
        let store = Store::open(&metadata_path)?;
        store.upsert_book(&crate::domain::Book::new("book", "UTF-16 body fixture"))?;
        add_chapter(&store, "uid", 0)?;
        let assets = BookAssetStore::open(directory.path().join("images"), 8 * 1024 * 1024)?;
        assets.recover(&store)?;
        personal_state(&store)?;
        let shared = publish(&store, &assets, "uid", "images/shared.png", 79)?;
        let shared_key = shared.images[0].cache_key.as_ref().unwrap();
        let shared_path = assets.cache.get_for_commit(shared_key)?.unwrap();
        let shared_bytes = fs::read(&shared_path)?;

        // Each legal BMP scalar occupies two stored UTF-16 bytes and three
        // UTF-8 bytes. The real column is above SQLite's overflow-cache cutoff.
        let body = "\u{6f22}".repeat(4096);
        store.upsert_chapter(&Chapter {
            id: "bmp".into(),
            book_id: "book".into(),
            index: 1,
            title: "BMP body fixture".into(),
            body: Some(body.clone()),
            content_account_key: "fixture-body-owner".into(),
            ..Default::default()
        })?;
        assets.commit_chapter(
            &store,
            "book",
            "bmp",
            &format!("<p>{body}</p>"),
            &body,
            None,
        )?;
        let connection = rusqlite::Connection::open(&metadata_path)?;
        let encoded: i64 = connection.query_row(
            "SELECT octet_length(body) FROM chapters WHERE book_id='book' AND source_id='bmp'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(encoded, 8192);
        assert_eq!(body.len(), 12288);
        let binding: (String, String) = connection.query_row(
            "SELECT content_status,content_account_key FROM chapters WHERE book_id='book' AND source_id='bmp'",
            [], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let pending_path = assets.cache.root().join(".tmp-99-2");
        fs::write(&pending_path, b"pending")?;
        rusqlite::Connection::open(assets.cache.root().join("cache-index.sqlite3"))?.execute(
            "INSERT INTO cache_pending_cleanup(filename,size) VALUES('.tmp-99-2',7)",
            [],
        )?;
        let original = manifests(&store, &["uid", "bmp"])?;
        let keys = assets.cache.keys()?;
        let pins = assets.cache.pinned_keys()?;
        let usage = assets.usage()?;
        let pending = debt(&assets)?;
        let access = access_sequence(&assets, shared_key)?;

        // An image-free current chapter avoids later cache metadata fees. Its
        // exact aggregate allowance therefore ends at body-temporary admission.
        assert_eq!(assets.complete_many(&store, "book", &["bmp"])?, vec![true]);
        // The primary-source allocation forecast is 8195 raw-cache bytes,
        // 16385 conversion-capacity bytes, 12288 owned bytes and 4096 scratch.
        // Observe the actual prepayment, not a result from the pricing helper.
        assert_eq!(
            assets.bulk_verification_stats().body_temporary_bytes,
            40_964
        );
        let exact = assets.bulk_verification_stats().metadata_bytes;
        let body_reads = store.chapter_body_read_count();
        let copies = store.chapter_body_materialization_stats("book", 1);
        {
            let _limits = LimitsGuard::new(BulkVerificationLimits {
                metadata_bytes: exact - 1,
                ..Default::default()
            });
            let error = assets.complete_many(&store, "book", &["bmp"]).unwrap_err();
            assert!(
                error
                    .downcast_ref::<BulkVerificationLimitExceeded>()
                    .is_some()
            );
            assert_eq!(assets.bulk_verification_stats().body_reads, 0);
            assert_eq!(assets.bulk_verification_stats().hashes, 0);
            assert_eq!(store.chapter_body_read_count(), body_reads);
            assert_eq!(store.chapter_body_materialization_stats("book", 1), copies);
            assert_eq!(assets.cache.keys()?, keys);
            assert_eq!(assets.cache.pinned_keys()?, pins);
            assert_eq!(assets.usage()?, usage);
            assert_eq!(debt(&assets)?, pending);
            assert_eq!(access_sequence(&assets, shared_key)?, access);
            assert_eq!(fs::read(&shared_path)?, shared_bytes);
            assert_eq!(fs::read(&pending_path)?, b"pending");
            assert_eq!(manifests(&store, &["uid", "bmp"])?, original);
            assert_eq!(
                connection.query_row(
                    "SELECT body FROM chapters WHERE book_id='book' AND source_id='bmp'",
                    [],
                    |row| row.get::<_, String>(0)
                )?,
                body
            );
            assert_eq!(connection.query_row(
                "SELECT content_status,content_account_key FROM chapters WHERE book_id='book' AND source_id='bmp'",
                [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?, binding);
            assert_eq!(
                store.annotations("book")?[0].note,
                "Keep bulk verification note"
            );
            assert_eq!(store.bookmarks("book")?[0].char_offset, 1);
        }
        {
            let _limits = LimitsGuard::new(BulkVerificationLimits {
                metadata_bytes: exact,
                ..Default::default()
            });
            assert_eq!(assets.complete_many(&store, "book", &["bmp"])?, vec![true]);
            assert_eq!(assets.bulk_verification_stats().metadata_bytes, exact);
            assert_eq!(assets.bulk_verification_stats().body_reads, 1);
            assert_eq!(assets.bulk_verification_stats().body_bytes, body.len());
            assert_eq!(
                assets.bulk_verification_stats().body_temporary_bytes,
                40_964
            );
            assert_eq!(assets.bulk_verification_stats().hashes, 0);
            assert_eq!(store.chapter_body_read_count(), body_reads + 1);
            assert_eq!(
                store.chapter_body_materialization_stats("book", 1),
                (copies.0 + 1, copies.1 + body.len())
            );
        }
        assert_eq!(assets.cache.keys()?, keys);
        assert_eq!(assets.cache.pinned_keys()?, pins);
        assert_eq!(assets.usage()?, usage);
        assert_eq!(debt(&assets)?, pending);
        assert_eq!(access_sequence(&assets, shared_key)?, access);
        assert_eq!(fs::read(&shared_path)?, shared_bytes);
        assert_eq!(fs::read(&pending_path)?, b"pending");
        assert_eq!(manifests(&store, &["uid", "bmp"])?, original);
        assert_eq!(
            store.chapter_body("book", 1)?.as_deref(),
            Some(body.as_str())
        );
        assert_eq!(connection.query_row(
            "SELECT content_status,content_account_key FROM chapters WHERE book_id='book' AND source_id='bmp'",
            [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?, binding);
        assert_personal_state(&store)?;
    }
    Ok(())
}

#[test]
fn missing_and_incomplete_manifests_never_read_hidden_bodies_or_claim_completeness() -> Result<()> {
    let (directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    let complete = publish(&store, &assets, "uid", "images/shared.png", 81)?;
    add_chapter(&store, "incomplete", 1)?;
    let mut incomplete = complete.clone();
    incomplete.chapter_uid = "incomplete".into();
    incomplete.complete = false;
    store.set_setting(&manifest_key("book", "incomplete")?, &incomplete)?;
    add_chapter(&store, "missing", 2)?;
    rusqlite::Connection::open(directory.path().join("metadata.sqlite3"))?.execute(
        "UPDATE chapters SET body=x'fffe' WHERE source_id IN ('incomplete','missing')",
        [],
    )?;
    let body_reads = store.chapter_body_read_count();
    let hidden_copies = [
        store.chapter_body_materialization_stats("book", 1),
        store.chapter_body_materialization_stats("book", 2),
    ];
    assert_eq!(
        assets.complete_many(&store, "book", &["missing", "incomplete", "uid"])?,
        vec![false, false, true]
    );
    let stats = assets.bulk_verification_stats();
    assert_eq!(stats.manifest_reads, 2);
    assert_eq!(stats.body_reads, 1);
    assert_eq!(stats.hashes, 1);
    assert_eq!(store.chapter_body_read_count(), body_reads + 1);
    assert_eq!(
        store.chapter_body_materialization_stats("book", 1),
        hidden_copies[0]
    );
    assert_eq!(
        store.chapter_body_materialization_stats("book", 2),
        hidden_copies[1]
    );
    assert_eq!(assets.cache.pinned_keys()?.len(), 1);
    Ok(())
}

#[test]
fn changed_actual_body_and_ambiguous_uid_cannot_use_a_png_as_full_content_evidence() -> Result<()> {
    let (directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    publish(&store, &assets, "uid", "images/shared.png", 91)?;
    let key = assets.cache.keys()?.pop().unwrap();
    let before = access_sequence(&assets, &key)?;
    rusqlite::Connection::open(directory.path().join("metadata.sqlite3"))?.execute(
        "UPDATE chapters SET body='Different retained actual text' WHERE source_id='uid'",
        [],
    )?;
    assert_eq!(assets.complete_many(&store, "book", &["uid"])?, vec![false]);
    assert_eq!(assets.bulk_verification_stats().body_reads, 1);
    assert_eq!(assets.bulk_verification_stats().hashes, 0);
    assert_eq!(access_sequence(&assets, &key)?, before);
    store.upsert_chapter(&Chapter {
        id: "uid".into(),
        book_id: "book".into(),
        index: 1,
        body: Some(tests::TEXT.into()),
        ..Default::default()
    })?;
    let error = assets.complete_many(&store, "book", &["uid"]).unwrap_err();
    assert!(
        error
            .downcast_ref::<crate::store::CachedResourceLimit>()
            .is_some()
    );
    assert_eq!(assets.bulk_verification_stats().hashes, 0);
    assert_eq!(assets.cache.pinned_state(&key)?, Some(true));
    Ok(())
}

#[test]
fn a_new_operation_cannot_reuse_the_previous_png_stamp_after_replacement() -> Result<()> {
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    let manifest = publish(&store, &assets, "uid", "images/shared.png", 101)?;
    assert_eq!(assets.complete_many(&store, "book", &["uid"])?, vec![true]);
    let key = manifest.images[0].cache_key.as_ref().unwrap();
    let path = assets.cache.get_for_commit(key)?.unwrap();
    let damaged = vec![b'x'; fs::metadata(&path)?.len() as usize];
    fs::write(&path, &damaged)?;
    assert_eq!(assets.complete_many(&store, "book", &["uid"])?, vec![false]);
    assert_eq!(assets.bulk_verification_stats().hashes, 1);
    assert!(assets.reconcile_required.get());
    assert_eq!(store.chapter_body("book", 0)?.as_deref(), Some(tests::TEXT));
    assert_eq!(
        store.setting::<ChapterManifest>(&manifest_key("book", "uid")?)?,
        Some(manifest)
    );
    Ok(())
}

#[test]
fn unknown_manifest_type_json_or_array_count_never_becomes_full_or_retires_shared_data()
-> Result<()> {
    for fault in 0..3 {
        let (directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
        personal_state(&store)?;
        publish(&store, &assets, "uid", "images/shared.png", 105)?;
        add_chapter(&store, "second", 1)?;
        let second = publish(&store, &assets, "second", "images/shared.png", 105)?;
        let key = manifest_key("book", "second")?;
        if fault == 0 {
            rusqlite::Connection::open(directory.path().join("metadata.sqlite3"))?
                .execute("UPDATE settings SET value=x'fffe' WHERE key=?1", [&key])?;
        } else if fault == 1 {
            store.set_setting(&key, &"invalid manifest shape")?;
        } else {
            let mut value = serde_json::to_value(second)?;
            value["images"] = serde_json::Value::Array(vec![serde_json::Value::Null; 2049]);
            store.set_setting(&key, &value)?;
        }
        let pins = assets.cache.pinned_keys()?;
        let keys = assets.cache.keys()?;
        let usage = assets.usage()?;
        let pending = debt(&assets)?;
        let body_reads = store.chapter_body_read_count();
        let error = assets
            .complete_many(&store, "book", &["uid", "second"])
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::store::CachedResourceLimit>()
                .is_some()
        );
        let stats = assets.bulk_verification_stats();
        assert_eq!(stats.manifest_reads, 0);
        assert_eq!(stats.body_reads, 0);
        assert_eq!(stats.hashes, 0);
        assert_eq!(store.chapter_body_read_count(), body_reads);
        assert_eq!(assets.cache.pinned_keys()?, pins);
        assert_eq!(assets.cache.keys()?, keys);
        assert_eq!(assets.usage()?, usage);
        assert_eq!(debt(&assets)?, pending);
        assert_personal_state(&store)?;
    }
    Ok(())
}

#[test]
fn prepaid_cache_snapshot_rechecks_current_identifiers_and_stamps_on_every_lookup() -> Result<()> {
    for replace_index in [false, true] {
        let (directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
        personal_state(&store)?;
        let manifest = publish(&store, &assets, "uid", "images/shared.png", 111)?;
        let key = manifest.images[0].cache_key.as_ref().unwrap();
        let path = assets.cache.get_for_commit(key)?.unwrap();
        let mut scope = VerificationScope::new(assets.cache.verification_work_limit());
        let inspected =
            assets
                .cache
                .inspect_keys_prepaid(&[key.clone()], &mut scope, 64 * 1024 * 1024)?;
        assert!(inspected.current_path(&assets.cache, key)?.is_some());
        let original_stamp = crate::cache::verification_stamp_for_test(&path)?;
        let original_bytes = fs::read(&path)?;
        let original_manifests = manifests(&store, &["uid"])?;
        let index = rusqlite::Connection::open(assets.cache.root().join("cache-index.sqlite3"))?;
        if replace_index {
            let affected = index.execute(
                "UPDATE cache_entries SET filename=?2 WHERE key=?1",
                rusqlite::params![key, "changed-identifier"],
            )?;
            assert_eq!(
                affected, 1,
                "fixture index mutation did not update its current row"
            );
            let filename: String = index.query_row(
                "SELECT filename FROM cache_entries WHERE key=?1",
                [key],
                |row| row.get(0),
            )?;
            assert_eq!(filename, "changed-identifier");
        } else {
            // A same-size in-place overwrite need not advance any observed
            // timestamp. Keep both files live before replacement so Unix has
            // a distinct inode independently of the host clock granularity.
            let replacement = directory.path().join("replacement.png");
            let retained_original = directory.path().join("retained-original.png");
            let replacement_length = original_bytes.len() + if cfg!(unix) { 0 } else { 1 };
            fs::write(&replacement, vec![b'x'; replacement_length])?;
            let original_metadata = fs::symlink_metadata(&path)?;
            let replacement_metadata = fs::symlink_metadata(&replacement)?;
            assert_eq!(replacement_metadata.len(), replacement_length as u64);
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                assert_eq!(replacement_metadata.len(), original_metadata.len());
                assert_ne!(
                    (original_metadata.dev(), original_metadata.ino()),
                    (replacement_metadata.dev(), replacement_metadata.ino()),
                    "fixture replacement reused the still-live original identity"
                );
            }
            #[cfg(not(unix))]
            assert_eq!(replacement_metadata.len(), original_metadata.len() + 1);
            fs::rename(&path, &retained_original)?;
            fs::rename(&replacement, &path)?;
            assert_eq!(fs::read(&retained_original)?, original_bytes);
        }
        let current_stamp = crate::cache::verification_stamp_for_test(&path)?;
        if replace_index {
            assert_eq!(
                current_stamp, original_stamp,
                "index mutation changed the physical fixture"
            );
        } else {
            assert_ne!(
                current_stamp, original_stamp,
                "fixture replacement did not change the actual stamp"
            );
        }
        let current_bytes = fs::read(&path)?;
        let current_filename: String = index.query_row(
            "SELECT filename FROM cache_entries WHERE key=?1",
            [key],
            |row| row.get(0),
        )?;
        let keys = assets.cache.keys()?;
        let pins = assets.cache.pinned_keys()?;
        let usage = assets.usage()?;
        let pending = debt(&assets)?;
        assert!(
            inspected.current_path(&assets.cache, key).is_err(),
            "accepted a stale lookup proof; index mutation: {replace_index}"
        );
        assert!(
            inspected.finish(&assets.cache, false).is_err(),
            "accepted stale retirement evidence; index mutation: {replace_index}"
        );
        assert_eq!(assets.cache.keys()?, keys);
        assert_eq!(assets.cache.pinned_keys()?, pins);
        assert_eq!(assets.usage()?, usage);
        assert_eq!(debt(&assets)?, pending);
        assert_eq!(
            index.query_row(
                "SELECT filename FROM cache_entries WHERE key=?1",
                [key],
                |row| row.get::<_, String>(0)
            )?,
            current_filename
        );
        assert_eq!(
            crate::cache::verification_stamp_for_test(&path)?,
            current_stamp
        );
        assert_eq!(fs::read(&path)?, current_bytes);
        assert_eq!(manifests(&store, &["uid"])?, original_manifests);
        assert!(path.is_file());
        assert_eq!(scope.stats().hashes, 1);
        assert_personal_state(&store)?;
    }
    Ok(())
}

#[test]
fn shared_svg_guard_preserves_unresolved_counts_and_publication_status() -> Result<()> {
    use crate::source::images::{SvgGuardLimits, image_svg_guard_tests::ObservationScope};

    let corpus = [
        "<p>Before</p><svg><g><image href='unsupported.svg'/></g></svg><p>After</p>",
        "<p>Before</p><div hidden><svg><image href='hidden.svg'/></svg><img></div><p>After</p>",
        "<p>Before</p><svg><foreignObject><svg><image href='nested.svg'/></svg><img data-original='lazy.png'><img></foreignObject></svg><p>After</p>",
        "<p>Before</p><math><mi><svg><image href='math.svg'/></svg></mi></math><p>After</p>",
        "<p>Before</p><svg><foreignObject><math><mrow><image href='foreign.svg'/></mrow></math></foreignObject><image href='outer.svg'/></svg><p>After</p>",
        "<p>Before</p><svg xmlns='urn:fixture'><image xmlns='urn:other' href='namespaced.svg'/></svg><p>After</p>",
        "<p>Before</p><svg><foreignObject><s:svg><s:image></s:image></s:svg></foreignObject></svg><p>After</p>",
        "<p>Before</p><template><svg><image href='template.svg'/></svg><img></template><p>After</p>",
        "<p>Before</p><img src=' '><img data-src='lazy.png'><img data-lazy-src='later.png'><img data-original='original.png'><p>After</p>",
        "<p>Before</p><math><mrow><mrow><mrow><image/><image/><image/><image/></mrow></mrow></mrow></math><p>After</p>",
        "<p>Before</p><math><mi>value</mi></math><p>After</p>",
    ];
    for html in corpus {
        let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
        let document = Html::parse_document(html);
        // This small locked-selector oracle checks the consumer's exact DOM
        // relation, including namespace and HTML integration-point transitions.
        let svg = document
            .select(&Selector::parse("svg image").unwrap())
            .count();
        let missing = document
            .select(&Selector::parse("img").unwrap())
            .filter(|image| {
                !["src", "data-src", "data-original", "data-lazy-src"]
                    .iter()
                    .any(|name| {
                        image
                            .value()
                            .attr(name)
                            .is_some_and(|source| !source.trim().is_empty())
                    })
            })
            .count();
        let expected = u32::try_from(svg + missing)?;
        let observation = ObservationScope::start(SvgGuardLimits::default());
        let metadata = prepare_chapter_metadata(html, tests::TEXT)?;
        let measured = observation.stats();
        assert_eq!(metadata.unresolved_images, expected);
        assert_eq!(measured.preflights, 1);
        assert_eq!(measured.admitted, 1);
        assert_eq!(measured.rejected, 0);
        assert_eq!(measured.summary_builds, 1);
        assert_eq!(measured.usage.svg_images, svg);
        assert_eq!(measured.usage.element_visits, measured.usage.elements);
        assert!(measured.usage.child_inspections <= measured.usage.nodes);
        assert_eq!(measured.usage.work_bytes, 8 * measured.usage.nodes);
        assert!(measured.usage.metadata_bytes <= 64 * 1024 * 1024);
        drop(observation);
        let manifest = assets.commit_prepared_chapter_metadata(
            &store,
            "book",
            "uid",
            html,
            tests::TEXT,
            &metadata,
            None,
        )?;
        assert_eq!(manifest.unresolved_images, expected);
        if expected != 0 {
            assert!(!manifest.complete);
        }
        if expected == 0 && metadata.anchors.is_empty() {
            assert!(manifest.complete);
        }
        assert_eq!(store.chapter_body("book", 0)?.as_deref(), Some(tests::TEXT));
    }
    Ok(())
}

#[test]
fn svg_guard_quota_errors_reach_metadata_and_publication_without_mutating_shared_state()
-> Result<()> {
    use crate::source::images::{SvgGuardLimits, image_svg_guard_tests::ObservationScope};

    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    personal_state(&store)?;
    let shared = publish(&store, &assets, "uid", "images/shared.png", 117)?;
    add_chapter(&store, "second", 1)?;
    publish(&store, &assets, "second", "images/shared.png", 117)?;
    let key = shared.images[0].cache_key.as_ref().unwrap();
    let path = assets.cache.get_for_commit(key)?.unwrap();
    let payload = fs::read(&path)?;
    let cleanup_path = assets.cache.root().join(".tmp-99-3");
    fs::write(&cleanup_path, b"pending")?;
    rusqlite::Connection::open(assets.cache.root().join("cache-index.sqlite3"))?.execute(
        "INSERT INTO cache_pending_cleanup(filename,size) VALUES('.tmp-99-3',7)",
        [],
    )?;
    let original = manifests(&store, &["uid", "second"])?;
    let keys = assets.cache.keys()?;
    let pins = assets.cache.pinned_keys()?;
    let usage = assets.usage()?;
    let pending = debt(&assets)?;
    let access = access_sequence(&assets, key)?;
    let verification = assets.verification_stats.get();
    for limits in [
        SvgGuardLimits {
            work_bytes: 0,
            ..Default::default()
        },
        SvgGuardLimits {
            metadata_bytes: 0,
            ..Default::default()
        },
    ] {
        for html in [
            "<p>Before</p><p>After</p>",
            "<p>Before</p><svg><g><image href='blocked.svg'/></g></svg><p>After</p>",
        ] {
            let body_reads = store.chapter_body_read_count();
            let observation = ObservationScope::start(limits);
            let error = prepare_chapter_metadata(html, tests::TEXT).unwrap_err();
            assert!(matches!(
                error.downcast_ref::<crate::source::SourceError>(),
                Some(crate::source::SourceError::ResponseTooLarge)
            ));
            let error = assets
                .commit_prepared_chapter(&store, "book", "uid", html, tests::TEXT, None)
                .unwrap_err();
            assert!(matches!(
                error.downcast_ref::<crate::source::SourceError>(),
                Some(crate::source::SourceError::ResponseTooLarge)
            ));
            let measured = observation.stats();
            assert_eq!(measured.preflights, 2);
            assert_eq!(measured.admitted, 0);
            assert_eq!(measured.rejected, 2);
            assert_eq!(measured.summary_builds, 0);
            assert_eq!(measured.summary_capacity, 0);
            assert_eq!(measured.usage.element_visits, 0);
            assert_eq!(measured.usage.child_inspections, 0);
            assert_eq!(store.chapter_body_read_count(), body_reads);
            assert_eq!(assets.verification_stats.get(), verification);
            assert_eq!(assets.cache.keys()?, keys);
            assert_eq!(assets.cache.pinned_keys()?, pins);
            assert_eq!(assets.usage()?, usage);
            assert_eq!(debt(&assets)?, pending);
            assert_eq!(access_sequence(&assets, key)?, access);
            assert_eq!(fs::read(&path)?, payload);
            assert_eq!(fs::read(&cleanup_path)?, b"pending");
            assert_eq!(manifests(&store, &["uid", "second"])?, original);
            assert_personal_state(&store)?;
        }
    }
    Ok(())
}
