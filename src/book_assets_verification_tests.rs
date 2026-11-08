//! Bounded unique integrity work without changing durable publication semantics.

use super::*;
use std::fs;

fn packet(two: bool) -> Result<DownloadedChapterImages> {
    let mut assets = vec![tests::asset("images/a.png", 17)?];
    if two {
        assets.push(tests::asset("images/b.png", 143)?);
    }
    let source_to_asset = assets
        .iter()
        .map(|asset| (asset.source_names[0].clone(), asset.id.clone()))
        .collect();
    Ok(DownloadedChapterImages {
        assets,
        source_to_asset,
    })
}

fn html(count: usize, two: bool) -> String {
    let mut html = "<p>Before</p>".to_owned();
    for index in 0..count {
        html.push_str(if two && index % 2 == 1 {
            "<img src='images/b.png'>"
        } else {
            "<img src='images/a.png'>"
        });
    }
    html.push_str("<p>After</p>");
    html
}

fn add_chapter(store: &Store, uid: &str, index: u32) -> Result<()> {
    store.upsert_chapter(&crate::domain::Chapter {
        id: uid.into(),
        book_id: "book".into(),
        index,
        title: "Verification fixture".into(),
        body: Some(tests::TEXT.into()),
        ..crate::domain::Chapter::default()
    })
}

fn durable_manifest(store: &Store, uid: &str) -> Result<serde_json::Value> {
    Ok(store
        .setting::<serde_json::Value>(&manifest_key("book", uid)?)?
        .unwrap())
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

#[test]
fn repeated_occurrences_and_shared_manifests_hash_once_per_complete_reader_and_recovery()
-> Result<()> {
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    let downloaded = packet(false)?;
    let manifest = assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(2048, false),
        tests::TEXT,
        Some(&downloaded),
    )?;
    add_chapter(&store, "shared", 1)?;
    assets.commit_chapter(
        &store,
        "book",
        "shared",
        &html(1, false),
        tests::TEXT,
        Some(&downloaded),
    )?;
    let key = manifest.images[0].cache_key.as_ref().unwrap();
    let path = assets.cache.get_for_commit(key)?.unwrap();
    let size = fs::metadata(&path)?.len();
    let before = access_sequence(&assets, key)?;
    let paths = assets.image_paths(&manifest.images)?;
    assert_eq!(paths.len(), 2048);
    assert!(
        paths
            .iter()
            .all(|candidate| candidate.as_ref() == Some(&path))
    );
    assert_eq!(assets.verification_stats.get().hashes, 1);
    assert_eq!(assets.verification_stats.get().read_bytes, size);
    let after = access_sequence(&assets, key)?;
    assert_eq!(after - before, 1);
    assert!(assets.complete(&store, "book", "uid")?);
    assert_eq!(assets.verification_stats.get().hashes, 1);
    let verified = access_sequence(&assets, key)?;
    assert_eq!(verified, after, "Completeness validation changed recency");
    assets.recover(&store)?;
    assert_eq!(assets.verification_stats.get().hashes, 1);
    assert_eq!(assets.verification_stats.get().read_bytes, size);
    assert_eq!(assets.cache.pinned_state(key)?, Some(true));
    Ok(())
}

#[test]
fn late_verification_refusal_cannot_retire_corruption_or_modify_a_manifest() -> Result<()> {
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    let manifest = assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(2, true),
        tests::TEXT,
        Some(&packet(true)?),
    )?;
    let mut keys = manifest
        .images
        .iter()
        .map(|image| image.cache_key.clone().unwrap())
        .collect::<Vec<_>>();
    keys.sort();
    let path = assets.cache.get_for_commit(&keys[0])?.unwrap();
    let size = fs::metadata(&path)?.len();
    fs::write(&path, vec![b'x'; size as usize])?;
    let before = durable_manifest(&store, "uid")?;
    let usage = assets.usage()?;
    let pins = assets.cache.pinned_keys()?;
    let error = assets
        .complete_with_verification_budget(&store, "book", "uid", size + 1)
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<crate::cache::VerificationLimitExceeded>()
            .is_some()
    );
    assert_eq!(assets.verification_stats.get().hashes, 1);
    assert_eq!(durable_manifest(&store, "uid")?, before);
    assert_eq!(assets.usage()?, usage);
    assert_eq!(assets.cache.pinned_keys()?, pins);
    assert!(path.is_file());
    assert_eq!(store.chapter_body("book", 0)?.as_deref(), Some(tests::TEXT));
    Ok(())
}

#[test]
fn cached_only_and_image_free_publications_preserve_old_state_when_verification_is_refused()
-> Result<()> {
    for image_free in [false, true] {
        let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
        assets.commit_chapter(
            &store,
            "book",
            "uid",
            &html(2, true),
            tests::TEXT,
            Some(&packet(true)?),
        )?;
        let before = durable_manifest(&store, "uid")?;
        let usage = assets.usage()?;
        let pins = assets.cache.pinned_keys()?;
        let next_html = if image_free {
            "<p>Before</p><p>After</p>".to_owned()
        } else {
            html(2, true)
        };
        if image_free {
            assets.reconcile_required.set(true);
        }
        let metadata = prepare_chapter_metadata(&next_html, tests::TEXT)?;
        let error = assets
            .commit_prepared_chapter_metadata_with_verification_budget(
                &store,
                "book",
                "uid",
                &next_html,
                tests::TEXT,
                &metadata,
                None,
                0,
            )
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::cache::VerificationLimitExceeded>()
                .is_some()
        );
        assert_eq!(assets.verification_stats.get().read_bytes, 0);
        assert_eq!(durable_manifest(&store, "uid")?, before);
        assert_eq!(assets.usage()?, usage);
        assert_eq!(assets.cache.pinned_keys()?, pins);
        assert!(assets.reconcile_required.get());
    }
    Ok(())
}

#[test]
fn recovery_budget_failure_keeps_existing_unlink_debt_and_shared_pins() -> Result<()> {
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    let manifest = assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(1, false),
        tests::TEXT,
        Some(&packet(false)?),
    )?;
    let blocked = assets.cache.put("temporary-debt", b"debt")?;
    fs::remove_file(&blocked)?;
    fs::create_dir(&blocked)?;
    assert!(assets.cache.remove("temporary-debt").is_err());
    let before = durable_manifest(&store, "uid")?;
    let usage = assets.usage()?;
    let error = assets
        .recover_with_verification_budget(&store, 0)
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<crate::cache::VerificationLimitExceeded>()
            .is_some()
    );
    assert_eq!(durable_manifest(&store, "uid")?, before);
    assert_eq!(assets.usage()?, usage);
    assert!(blocked.is_dir());
    assert_eq!(
        assets
            .cache
            .pinned_state(manifest.images[0].cache_key.as_ref().unwrap())?,
        Some(true)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn recovery_and_cached_publication_share_one_operation_digest_proof() -> Result<()> {
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(1, false),
        tests::TEXT,
        Some(&packet(false)?),
    )?;
    assets.reconcile_required.set(true);
    assets.commit_chapter(&store, "book", "uid", &html(1, false), tests::TEXT, None)?;
    assert_eq!(assets.verification_stats.get().hashes, 1);
    assert!(!assets.reconcile_required.get());
    Ok(())
}

#[test]
fn repaired_shared_reference_survives_a_failed_new_manifest_after_read_invalidation() -> Result<()>
{
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    let downloaded = packet(false)?;
    let original = assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(1, false),
        tests::TEXT,
        Some(&downloaded),
    )?;
    add_chapter(&store, "shared", 1)?;
    assets.commit_chapter(
        &store,
        "book",
        "shared",
        &html(1, false),
        tests::TEXT,
        Some(&downloaded),
    )?;
    let key = original.images[0].cache_key.as_ref().unwrap();
    let path = assets.cache.get_for_commit(key)?.unwrap();
    let size = fs::metadata(&path)?.len();
    fs::write(&path, vec![b'x'; size as usize])?;
    assert!(assets.image_paths(&original.images)?[0].is_none());
    assert!(assets.reconcile_required.get());
    add_chapter(&store, "new", 2)?;
    let fault = rusqlite::Connection::open(
        assets
            .cache
            .root()
            .parent()
            .unwrap()
            .join("metadata.sqlite3"),
    )?;
    fault.execute_batch("CREATE TRIGGER reject_new_image_manifest BEFORE INSERT ON settings WHEN NEW.key='chapter.assets:book:new' BEGIN SELECT RAISE(ABORT,'fixture publication failure'); END")?;
    assert!(
        assets
            .commit_chapter(
                &store,
                "book",
                "new",
                &html(1, false),
                tests::TEXT,
                Some(&downloaded)
            )
            .is_err()
    );
    assert!(
        store
            .setting::<ChapterManifest>(&manifest_key("book", "new")?)?
            .is_none()
    );
    assert!(assets.cache.get_for_commit(key)?.is_some());
    assert_eq!(assets.cache.pinned_state(key)?, Some(true));
    assert!(assets.reconcile_required.get());
    assert_eq!(store.chapter_body("book", 0)?.as_deref(), Some(tests::TEXT));
    Ok(())
}

fn unindexed_candidate(
    assets: &BookAssetStore,
) -> Result<(PreparedChapterImages, String, PathBuf, usize)> {
    let original = tests::asset("images/new.png", 213)?;
    let id = original.id.clone();
    let downloaded = DownloadedChapterImages {
        source_to_asset: BTreeMap::from([("images/new.png".into(), id.clone())]),
        assets: vec![original],
    };
    let prepared = prepare_downloaded(downloaded)?;
    let png = &prepared.assets[&id].bytes;
    let key = asset_cache_key(&id)?;
    let filename = format!("{}-{}.png", digest(key.as_bytes()), digest(png));
    let path = assets.cache.root().join(filename);
    fs::write(&path, png)?;
    let size = png.len();
    Ok((prepared, key, path, size))
}

#[test]
fn unindexed_and_shared_alias_destinations_are_admitted_before_any_publication_change() -> Result<()>
{
    for alias in [false, true] {
        let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
        assets.commit_chapter(
            &store,
            "book",
            "uid",
            &html(1, false),
            tests::TEXT,
            Some(&packet(false)?),
        )?;
        assert!(!assets.reconcile_required.get());
        let (prepared, key, candidate, size) = unindexed_candidate(&assets)?;
        if alias {
            let filename = candidate
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            rusqlite::Connection::open(assets.cache.root().join("cache-index.sqlite3"))?.execute(
                "INSERT INTO cache_entries(key,filename,size,last_access,pinned) VALUES ('physical-alias',?1,?2,0,1)",
                rusqlite::params![filename, size as i64],
            )?;
        }
        let before = durable_manifest(&store, "uid")?;
        let usage = assets.usage()?;
        let pins = assets.cache.pinned_keys()?;
        let png_before = fs::read(&candidate)?;
        let next_html = "<p>Before</p><img src='images/new.png'><p>After</p>";
        let metadata = prepare_chapter_metadata(next_html, tests::TEXT)?;
        let error = assets
            .commit_prepared_chapter_metadata_with_verification_budget(
                &store,
                "book",
                "uid",
                next_html,
                tests::TEXT,
                &metadata,
                Some(&prepared),
                0,
            )
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::cache::VerificationLimitExceeded>()
                .is_some()
        );
        assert_eq!(assets.verification_stats.get().read_bytes, 0);
        assert_eq!(assets.verification_stats.get().hashes, 0);
        assert_eq!(durable_manifest(&store, "uid")?, before);
        assert_eq!(store.chapter_body("book", 0)?.as_deref(), Some(tests::TEXT));
        assert_eq!(assets.usage()?, usage);
        assert_eq!(assets.cache.pinned_keys()?, pins);
        assert_eq!(fs::read(&candidate)?, png_before);
        assert!(assets.cache.pinned_state(&key)?.is_none());
        #[cfg(unix)]
        {
            // Recovery reads the other old PNG once; consuming this admitted
            // destination through put must not add a third hidden hash.
            assets.commit_prepared_chapter_metadata(
                &store,
                "book",
                "uid",
                next_html,
                tests::TEXT,
                &metadata,
                Some(&prepared),
            )?;
            assert_eq!(assets.verification_stats.get().hashes, 2);
            assert_eq!(assets.cache.pinned_state(&key)?, Some(true));
        }
    }
    Ok(())
}

#[test]
fn later_recovery_quota_refusal_does_not_remove_a_preflighted_unindexed_destination() -> Result<()>
{
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    assets.commit_chapter(
        &store,
        "book",
        "uid",
        &html(1, false),
        tests::TEXT,
        Some(&packet(false)?),
    )?;
    let (prepared, key, candidate, size) = unindexed_candidate(&assets)?;
    assets.reconcile_required.set(true);
    let before = durable_manifest(&store, "uid")?;
    let usage = assets.usage()?;
    let pins = assets.cache.pinned_keys()?;
    let next_html = "<p>Before</p><img src='images/new.png'><p>After</p>";
    let metadata = prepare_chapter_metadata(next_html, tests::TEXT)?;
    let error = assets
        .commit_prepared_chapter_metadata_with_verification_budget(
            &store,
            "book",
            "uid",
            next_html,
            tests::TEXT,
            &metadata,
            Some(&prepared),
            size as u64 + 1,
        )
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<crate::cache::VerificationLimitExceeded>()
            .is_some()
    );
    assert_eq!(assets.verification_stats.get().hashes, 1);
    assert_eq!(assets.verification_stats.get().read_bytes, size as u64);
    assert_eq!(durable_manifest(&store, "uid")?, before);
    assert_eq!(assets.usage()?, usage);
    assert_eq!(assets.cache.pinned_keys()?, pins);
    assert!(candidate.is_file());
    assert!(assets.cache.pinned_state(&key)?.is_none());
    Ok(())
}

fn shared_manifest_pair(store: &Store, assets: &BookAssetStore) -> Result<ChapterManifest> {
    let downloaded = packet(false)?;
    let original = assets.commit_chapter(
        store,
        "book",
        "uid",
        &html(1, false),
        tests::TEXT,
        Some(&downloaded),
    )?;
    add_chapter(store, "shared", 1)?;
    assets.commit_chapter(
        store,
        "book",
        "shared",
        &html(1, false),
        tests::TEXT,
        Some(&downloaded),
    )?;
    assert!(!assets.reconcile_required.get());
    Ok(original)
}

fn displaced_candidate_debt(
    assets: &BookAssetStore,
) -> Result<(PreparedChapterImages, String, PathBuf, Vec<u8>)> {
    let (prepared, key, path, _size) = unindexed_candidate(assets)?;
    let png = prepared.assets.values().next().unwrap().bytes.clone();
    assert_eq!(assets.cache.put_png(&key, &png)?, path);
    fs::remove_file(&path)?;
    fs::create_dir(&path)?;
    assert!(assets.cache.remove(&key).is_err());
    assert!(assets.cache.pinned_state(&key)?.is_none());
    fs::rename(
        &path,
        assets.cache.root().join("displaced-candidate-directory"),
    )?;
    Ok((prepared, key, path, png))
}

#[derive(Debug, PartialEq, Eq)]
struct DurableCacheState {
    entries: Vec<(String, String, i64, i64, bool)>,
    debt: Vec<(String, i64)>,
    usage: u64,
}

fn durable_cache_state(assets: &BookAssetStore) -> Result<DurableCacheState> {
    let connection = rusqlite::Connection::open(assets.cache.root().join("cache-index.sqlite3"))?;
    let entries = {
        let mut statement = connection.prepare(
            "SELECT key,filename,size,last_access,pinned FROM cache_entries ORDER BY key",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let debt = {
        let mut statement = connection
            .prepare("SELECT filename,size FROM cache_pending_cleanup ORDER BY filename")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok(DurableCacheState {
        entries,
        debt,
        usage: assets.usage()?,
    })
}

fn assert_same_file(path: &Path, original: &fs::Metadata, bytes: &[u8]) -> Result<()> {
    let current = fs::metadata(path)?;
    assert_eq!(current.len(), original.len());
    assert_eq!(current.modified().ok(), original.modified().ok());
    assert_eq!(current.created().ok(), original.created().ok());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(current.dev(), original.dev());
        assert_eq!(current.ino(), original.ino());
        assert_eq!(current.ctime(), original.ctime());
        assert_eq!(current.ctime_nsec(), original.ctime_nsec());
    }
    assert_eq!(fs::read(path)?, bytes);
    Ok(())
}

#[test]
fn dirty_recovery_rejects_a_late_destination_before_draining_old_debt() -> Result<()> {
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    let original = shared_manifest_pair(&store, &assets)?;
    let (_prepared, key, candidate, png) = displaced_candidate_debt(&assets)?;
    assets.reconcile_required.set(true);
    let mut scope = VerificationScope::new(0);
    assets
        .cache
        .preverify_png_destinations(&[(key.as_str(), png.as_slice())], &mut scope)?;
    let before = durable_cache_state(&assets)?;
    let before_manifest = durable_manifest(&store, "uid")?;
    let before_shared = durable_manifest(&store, "shared")?;
    fs::write(&candidate, &png)?;
    let appeared = fs::metadata(&candidate)?;
    assert!(
        assets
            .recover_using_verification(&store, &mut scope)
            .is_err()
    );
    assert_eq!(scope.stats(), crate::cache::VerificationStats::default());
    assert_eq!(durable_cache_state(&assets)?, before);
    assert_eq!(durable_manifest(&store, "uid")?, before_manifest);
    assert_eq!(durable_manifest(&store, "shared")?, before_shared);
    assert_eq!(store.chapter_body("book", 0)?.as_deref(), Some(tests::TEXT));
    assert_eq!(store.chapter_body("book", 1)?.as_deref(), Some(tests::TEXT));
    assert_eq!(
        assets
            .cache
            .pinned_state(original.images[0].cache_key.as_ref().unwrap())?,
        Some(true)
    );
    assert!(assets.reconcile_required.get());
    assert_same_file(&candidate, &appeared, &png)?;
    Ok(())
}

#[test]
fn publication_cleanup_and_rollback_share_the_original_destination_fence() -> Result<()> {
    for rollback in [false, true] {
        let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
        let original = shared_manifest_pair(&store, &assets)?;
        let old_key = original.images[0].cache_key.as_ref().unwrap();
        let old_path = assets.cache.get_for_commit(old_key)?.unwrap();
        let old_png = fs::read(&old_path)?;
        let orphan_key = asset_cache_key(&digest(b"unreferenced rollback fixture"))?;
        let orphan_path = assets.cache.put_png(&orphan_key, &old_png)?;
        assets.cache.set_pinned(&orphan_key, true)?;
        let (_prepared, key, candidate, png) = displaced_candidate_debt(&assets)?;
        assets.reconcile_required.set(true);
        let mut scope = VerificationScope::new(0);
        assets
            .cache
            .preverify_png_destinations(&[(key.as_str(), png.as_slice())], &mut scope)?;
        let before = durable_cache_state(&assets)?;
        let before_manifest = durable_manifest(&store, "uid")?;
        let before_shared = durable_manifest(&store, "shared")?;
        fs::write(&candidate, &png)?;
        let appeared = fs::metadata(&candidate)?;
        let result = if rollback {
            assets.rollback_attempts(
                &BTreeMap::from([(
                    orphan_key.clone(),
                    PinAttempt {
                        previous_pin: Some(false),
                        available_before: false,
                    },
                )]),
                &scope,
            )
        } else {
            assets.cleanup_unreferenced(&store, &scope)
        };
        assert!(result.is_err());
        assert_eq!(scope.stats(), crate::cache::VerificationStats::default());
        assert_eq!(durable_cache_state(&assets)?, before);
        assert_eq!(durable_manifest(&store, "uid")?, before_manifest);
        assert_eq!(durable_manifest(&store, "shared")?, before_shared);
        assert_eq!(store.chapter_body("book", 0)?.as_deref(), Some(tests::TEXT));
        assert_eq!(assets.cache.pinned_state(&orphan_key)?, Some(true));
        assert_eq!(fs::read(orphan_path)?, old_png);
        assert_eq!(fs::read(old_path)?, old_png);
        assert_same_file(&candidate, &appeared, &png)?;
    }
    Ok(())
}

#[test]
fn stable_destination_debt_recovery_allows_publication_and_preserves_shared_assets() -> Result<()> {
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    let original = shared_manifest_pair(&store, &assets)?;
    let old_key = original.images[0].cache_key.as_ref().unwrap();
    let old_path = assets.cache.get_for_commit(old_key)?.unwrap();
    let old_png = fs::read(&old_path)?;
    let before_manifest = durable_manifest(&store, "uid")?;
    let before_shared = durable_manifest(&store, "shared")?;
    let (prepared, key, candidate, png) = displaced_candidate_debt(&assets)?;
    fs::write(&candidate, &png)?;
    let mut scope = VerificationScope::new(assets.cache.verification_work_limit());
    assets
        .cache
        .preverify_png_destinations(&[(key.as_str(), png.as_slice())], &mut scope)?;
    assets.reconcile_required.set(true);
    assets.recover_using_verification(&store, &mut scope)?;
    assert!(!candidate.exists());
    assert!(durable_cache_state(&assets)?.debt.is_empty());
    assert_eq!(durable_manifest(&store, "uid")?, before_manifest);
    assert_eq!(durable_manifest(&store, "shared")?, before_shared);
    assert_eq!(scope.stats().hashes, 2);
    let verified = scope.stats();
    let next_html = "<p>Before</p><img src='images/new.png'><p>After</p>";
    let metadata = prepare_chapter_metadata(next_html, tests::TEXT)?;
    let manifest = assets.commit_prepared_using_verification(
        &store,
        "book",
        "uid",
        next_html,
        tests::TEXT,
        &metadata,
        Some(&prepared),
        &mut scope,
    )?;
    assert!(manifest.complete);
    assert_eq!(scope.stats(), verified);
    assert_eq!(fs::read(&candidate)?, png);
    assert!(durable_cache_state(&assets)?.debt.is_empty());
    assert_eq!(assets.cache.pinned_state(&key)?, Some(true));
    assert_eq!(assets.cache.pinned_state(old_key)?, Some(true));
    assert_eq!(durable_manifest(&store, "shared")?, before_shared);
    assert_eq!(fs::read(old_path)?, old_png);
    assert_eq!(store.chapter_body("book", 0)?.as_deref(), Some(tests::TEXT));
    assert!(assets.complete(&store, "book", "uid")?);
    assert!(!assets.reconcile_required.get());
    Ok(())
}
