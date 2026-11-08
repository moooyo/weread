//! Same-operation integrity evidence and pre-read work admission regressions.

use super::*;

fn fixture() -> Result<(tempfile::TempDir, FileCache)> {
    let directory = tempfile::tempdir()?;
    let cache = FileCache::open(directory.path(), 4096)?;
    Ok((directory, cache))
}

#[test]
fn duplicate_keys_use_one_streamed_hash_and_one_successful_recency_touch() -> Result<()> {
    let (_directory, cache) = fixture()?;
    let path = cache.put("shared", b"small")?;
    let keys = vec!["shared".to_owned(); 2048];
    let mut scope = VerificationScope::new(12);
    let before: i64 = cache
        .connection
        .query_row("SELECT total_changes()", [], |row| row.get(0))?;
    let inspected = cache.inspect_keys(&keys, &mut scope)?;
    inspected.finish(&cache, true)?;
    assert_eq!(inspected.path("shared"), Some(&path));
    assert_eq!(
        scope.stats(),
        VerificationStats {
            charged_bytes: 6,
            read_bytes: 5,
            hashes: 1
        }
    );
    let after: i64 = cache
        .connection
        .query_row("SELECT total_changes()", [], |row| row.get(0))?;
    assert_eq!(after - before, 1);
    cache
        .inspect_keys(&keys, &mut scope)?
        .finish(&cache, false)?;
    let repeat_hashes = if cfg!(unix) { 1 } else { 2 };
    assert_eq!(scope.stats().hashes, repeat_hashes);
    assert_eq!(scope.stats().read_bytes, 5 * repeat_hashes as u64);
    // A new operation must establish new evidence, even for unchanged bytes.
    let mut next = VerificationScope::new(6);
    cache
        .inspect_keys(&keys, &mut next)?
        .finish(&cache, false)?;
    assert_eq!(next.stats().hashes, 1);
    assert_eq!(next.stats().read_bytes, 5);
    Ok(())
}

#[test]
fn quota_is_paid_before_read_and_is_not_corruption_evidence() -> Result<()> {
    let (_directory, cache) = fixture()?;
    let path = cache.put("valid", b"small")?;
    cache.set_pinned("valid", true)?;
    let usage = cache.usage()?;
    let mut scope = VerificationScope::new(5);
    let error = cache
        .inspect_keys(&["valid".into()], &mut scope)
        .err()
        .unwrap();
    assert!(error.downcast_ref::<VerificationLimitExceeded>().is_some());
    assert_eq!(scope.stats(), VerificationStats::default());
    assert_eq!(cache.usage()?, usage);
    assert_eq!(cache.pinned_state("valid")?, Some(true));
    assert_eq!(fs::read(path)?, b"small");
    Ok(())
}

#[test]
fn late_quota_refusal_keeps_a_corrupt_probe_indexed_until_the_whole_batch_passes() -> Result<()> {
    let (_directory, cache) = fixture()?;
    let corrupt = cache.put("first", b"small")?;
    let valid = cache.put("second", b"other")?;
    cache.set_pinned("first", true)?;
    fs::write(&corrupt, b"bad!!")?;
    let mut scope = VerificationScope::new(6);
    let error = cache
        .inspect_keys(&["first".into(), "second".into()], &mut scope)
        .err()
        .unwrap();
    assert!(error.downcast_ref::<VerificationLimitExceeded>().is_some());
    assert_eq!(scope.stats().hashes, 1);
    assert_eq!(scope.stats().read_bytes, 5);
    assert_eq!(cache.usage()?, 10);
    assert_eq!(cache.pinned_state("first")?, Some(true));
    assert_eq!(fs::read(&corrupt)?, b"bad!!");
    assert_eq!(fs::read(&valid)?, b"other");
    assert!(cache.get_for_commit("first")?.is_none());
    assert_eq!(cache.usage()?, 5);
    Ok(())
}

#[test]
fn cache_replacement_and_instance_changes_reject_stale_inspections() -> Result<()> {
    let (directory, cache) = fixture()?;
    cache.put("key", b"small")?;
    let mut scope = VerificationScope::new(12);
    let old = cache.inspect_keys(&["key".into()], &mut scope)?;
    let replacement = cache.put("key", b"other")?;
    assert!(old.finish(&cache, false).is_err());
    let current = cache.inspect_keys(&["key".into()], &mut scope)?;
    current.finish(&cache, false)?;
    assert_eq!(current.path("key"), Some(&replacement));
    assert_eq!(scope.stats().hashes, 2);
    assert_eq!(scope.stats().read_bytes, 10);
    let reopened = FileCache::open(directory.path(), 4096)?;
    assert!(reopened.inspect_keys(&["key".into()], &mut scope).is_err());
    assert_eq!(fs::read(replacement)?, b"other");
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_replaced_inode_cannot_reuse_same_operation_digest_evidence() -> Result<()> {
    let (directory, cache) = fixture()?;
    let path = cache.put("key", b"small")?;
    let mut scope = VerificationScope::new(12);
    let old = cache.inspect_keys(&["key".into()], &mut scope)?;
    let temporary = directory.path().join("replacement-input");
    fs::write(&temporary, b"bad!!")?;
    fs::rename(&temporary, &path)?;
    assert!(old.finish(&cache, false).is_err());
    assert!(path.is_file());
    let changed = cache.inspect_keys(&["key".into()], &mut scope)?;
    assert!(changed.path("key").is_none());
    assert_eq!(scope.stats().hashes, 2);
    changed.finish(&cache, false)?;
    assert!(!path.exists());
    assert_eq!(cache.usage()?, 0);
    Ok(())
}

#[test]
fn retiring_an_invalid_alias_preserves_the_other_active_filename_reference() -> Result<()> {
    let (_directory, cache) = fixture()?;
    let path = cache.put("shared", b"small")?;
    cache.set_pinned("shared", true)?;
    let filename = path.file_name().unwrap().to_string_lossy().into_owned();
    cache.connection.execute(
        "INSERT INTO cache_entries(key,filename,size,last_access,pinned) VALUES ('alias',?1,3,0,0)",
        [filename.as_str()],
    )?;
    let mut scope = VerificationScope::new(6);
    let inspected = cache.inspect_keys(&["alias".into(), "shared".into()], &mut scope)?;
    inspected.finish(&cache, false)?;
    assert!(cache.entry("alias")?.is_none());
    assert_eq!(cache.pinned_state("shared")?, Some(true));
    assert_eq!(fs::read(path)?, b"small");
    assert_eq!(scope.stats().hashes, 1);
    Ok(())
}

#[test]
fn verification_refusal_does_not_drain_existing_unlink_debt() -> Result<()> {
    let (_directory, cache) = fixture()?;
    let blocked = cache.put("blocked", b"small")?;
    cache.put("valid", b"other")?;
    fs::remove_file(&blocked)?;
    fs::create_dir(&blocked)?;
    assert!(cache.remove("blocked").is_err());
    let usage = cache.usage()?;
    let mut scope = VerificationScope::new(5);
    assert!(cache.inspect_keys(&["valid".into()], &mut scope).is_err());
    assert_eq!(cache.usage()?, usage);
    assert!(blocked.is_dir());
    assert!(cache.put("more", b"extra").is_err());
    Ok(())
}

#[test]
fn a_late_unindexed_destination_is_not_hashed_or_adopted_by_preverified_put() -> Result<()> {
    let (_directory, cache) = fixture()?;
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::new_luma8(2, 2).write_to(&mut encoded, image::ImageFormat::Png)?;
    let bytes = encoded.into_inner();
    let key = "candidate";
    let filename = format!("{}-{}.png", digest(key.as_bytes()), digest(&bytes));
    let path = cache.root().join(filename);
    let mut scope = VerificationScope::new(0);
    cache.preverify_png_destinations(&[(key, bytes.as_slice())], &mut scope)?;
    fs::write(&path, &bytes)?;
    assert!(cache.put_png_preverified(key, &bytes, &mut scope).is_err());
    assert_eq!(scope.stats().read_bytes, 0);
    assert_eq!(scope.stats().hashes, 0);
    assert_eq!(fs::read(&path)?, bytes);
    assert_eq!(cache.usage()?, 0);
    assert!(cache.entry(key)?.is_none());
    // The standalone API still establishes its own bounded integrity proof.
    assert_eq!(cache.put_png(key, &bytes)?, path);
    Ok(())
}

fn encoded_png() -> Result<Vec<u8>> {
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::new_luma8(2, 2).write_to(&mut encoded, image::ImageFormat::Png)?;
    Ok(encoded.into_inner())
}

fn blocked_png_debt(cache: &FileCache, key: &str, bytes: &[u8]) -> Result<(String, PathBuf)> {
    let path = cache.put_png(key, bytes)?;
    let filename = path.file_name().unwrap().to_string_lossy().into_owned();
    fs::remove_file(&path)?;
    fs::create_dir(&path)?;
    assert!(cache.remove(key).is_err());
    assert!(cache.entry(key)?.is_none());
    fs::rename(&path, cache.root().join("displaced-blocking-directory"))?;
    Ok((filename, path))
}

fn cleanup_debt(cache: &FileCache) -> Result<Vec<(String, i64)>> {
    let mut statement = cache
        .connection
        .prepare("SELECT filename,size FROM cache_pending_cleanup ORDER BY filename")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[test]
fn pending_debt_cannot_erase_a_late_destination_before_preverified_put() -> Result<()> {
    let (_directory, cache) = fixture()?;
    let preserved = cache.put("preserved", b"other")?;
    cache.set_pinned("preserved", true)?;
    let bytes = encoded_png()?;
    let key = "candidate-debt";
    let (_filename, path) = blocked_png_debt(&cache, key, &bytes)?;
    let mut scope = VerificationScope::new(0);
    cache.preverify_png_destinations(&[(key, bytes.as_slice())], &mut scope)?;
    let debt = cleanup_debt(&cache)?;
    let entries = cache.entries()?;
    let usage = cache.usage()?;
    fs::write(&path, &bytes)?;
    let appeared = object_stamp(&path)?;
    assert!(cache.put_png_preverified(key, &bytes, &mut scope).is_err());
    assert_eq!(scope.stats(), VerificationStats::default());
    assert_eq!(object_stamp(&path)?, appeared);
    assert_eq!(fs::read(&path)?, bytes);
    assert_eq!(cleanup_debt(&cache)?, debt);
    assert_eq!(cache.entries()?, entries);
    assert_eq!(cache.usage()?, usage);
    assert_eq!(cache.pinned_state("preserved")?, Some(true));
    assert_eq!(fs::read(&preserved)?, b"other");

    // The fixture removes its late object. The original absent identity now
    // permits the stable debt retry, followed by this operation's owned write.
    fs::remove_file(&path)?;
    cache.retry_pending_cleanup_with_verification(&scope)?;
    assert!(cleanup_debt(&cache)?.is_empty());
    assert_eq!(cache.put_png_preverified(key, &bytes, &mut scope)?, path);
    assert_eq!(scope.stats(), VerificationStats::default());
    assert_eq!(fs::read(&path)?, bytes);
    assert_eq!(cache.pinned_state("preserved")?, Some(true));
    assert_eq!(fs::read(preserved)?, b"other");
    Ok(())
}

#[cfg(unix)]
#[test]
fn pending_debt_cannot_hide_a_changed_preflighted_inode() -> Result<()> {
    let (_directory, cache) = fixture()?;
    let bytes = encoded_png()?;
    let key = "changed-debt";
    let (_filename, path) = blocked_png_debt(&cache, key, &bytes)?;
    fs::write(&path, &bytes)?;
    let mut scope = VerificationScope::new(bytes.len() as u64 + 1);
    cache.preverify_png_destinations(&[(key, bytes.as_slice())], &mut scope)?;
    let admitted = scope.stats();
    let replacement = cache.root().join("late-replacement-input");
    fs::write(&replacement, &bytes)?;
    fs::rename(&replacement, &path)?;
    let changed = object_stamp(&path)?;
    let debt = cleanup_debt(&cache)?;
    let usage = cache.usage()?;
    assert!(cache.put_png_preverified(key, &bytes, &mut scope).is_err());
    assert_eq!(scope.stats(), admitted);
    assert_eq!(object_stamp(&path)?, changed);
    assert_eq!(fs::read(&path)?, bytes);
    assert_eq!(cleanup_debt(&cache)?, debt);
    assert_eq!(cache.usage()?, usage);
    assert!(cache.entry(key)?.is_none());
    Ok(())
}

#[test]
fn the_destination_fence_precedes_retirement_pins_and_guard_unwinding() -> Result<()> {
    let (_directory, cache) = fixture()?;
    let preserved = cache.put("preserved", b"other")?;
    let corrupt = cache.put("corrupt", b"small")?;
    fs::write(&corrupt, b"bad")?;
    let bytes = encoded_png()?;
    let key = "guard-debt";
    let (filename, path) = blocked_png_debt(&cache, key, &bytes)?;
    let mut scope = VerificationScope::new(0);
    cache.preverify_png_destinations(&[(key, bytes.as_slice())], &mut scope)?;
    let inspected = cache.inspect_keys(&["corrupt".into()], &mut scope)?;
    let mut guard = PendingObject::new_using_protection(
        &cache,
        &filename,
        bytes.len() as u64,
        Some(&scope.destinations),
    )?;
    fs::write(&path, &bytes)?;
    let appeared = object_stamp(&path)?;
    let debt = cleanup_debt(&cache)?;
    let entries = cache.entries()?;
    let usage = cache.usage()?;
    assert!(inspected.finish(&cache, true).is_err());
    assert!(
        cache
            .set_pinned_with_verification("preserved", true, &scope)
            .is_err()
    );
    assert!(cache.remove_with_verification("preserved", &scope).is_err());
    assert!(
        cache
            .retry_pending_cleanup_with_verification(&scope)
            .is_err()
    );
    assert!(guard.cleanup().is_err());
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = guard;
        panic!("fixture cleanup unwind");
    }));
    assert!(unwound.is_err());
    assert_eq!(scope.stats(), VerificationStats::default());
    assert_eq!(object_stamp(&path)?, appeared);
    assert_eq!(fs::read(&path)?, bytes);
    assert_eq!(cleanup_debt(&cache)?, debt);
    assert_eq!(cache.entries()?, entries);
    assert_eq!(cache.usage()?, usage);
    assert_eq!(cache.pinned_state("preserved")?, Some(false));
    assert_eq!(fs::read(preserved)?, b"other");
    assert_eq!(fs::read(corrupt)?, b"bad");
    Ok(())
}

#[test]
fn an_unchanged_preflighted_debt_can_be_removed_and_recreated_without_another_read() -> Result<()> {
    let (_directory, cache) = fixture()?;
    let preserved = cache.put("preserved", b"other")?;
    cache.set_pinned("preserved", true)?;
    let bytes = encoded_png()?;
    let key = "stable-debt";
    let (_filename, path) = blocked_png_debt(&cache, key, &bytes)?;
    fs::write(&path, &bytes)?;
    let mut scope = VerificationScope::new(bytes.len() as u64 + 1);
    cache.preverify_png_destinations(&[(key, bytes.as_slice())], &mut scope)?;
    let admitted = scope.stats();
    assert_eq!(admitted.hashes, 1);
    assert_eq!(cache.put_png_preverified(key, &bytes, &mut scope)?, path);
    assert_eq!(scope.stats(), admitted);
    assert!(cleanup_debt(&cache)?.is_empty());
    assert_eq!(fs::read(&path)?, bytes);
    assert_eq!(cache.pinned_state(key)?, Some(false));
    assert_eq!(cache.pinned_state("preserved")?, Some(true));
    assert_eq!(fs::read(preserved)?, b"other");
    Ok(())
}
