use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);
static NEXT_CACHE_ID: AtomicU64 = AtomicU64::new(1);

/// A bounded, single-application-process cache. Payloads use digest-only filenames.
pub struct FileCache {
    root: PathBuf,
    maximum_bytes: u64,
    connection: Connection,
    text_encoding: CacheTextEncoding,
    identity: u64,
    #[cfg(test)]
    inventory_reads: Rc<std::cell::Cell<InventoryReadStats>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    key: String,
    filename: String,
    size: u64,
    pinned: bool,
}

const MAX_INVENTORY_METADATA_BYTES: usize = 64 * 1024 * 1024;
const MAX_INVENTORY_WORK_BYTES: usize = 128 * 1024 * 1024;
const MAX_INVENTORY_KEY_BYTES: usize = 32 * 1024;
const MAX_INVENTORY_FILENAME_UPPER: usize = 512;
const INVENTORY_ROW_SLOT: usize = 4
    * (std::mem::size_of::<Entry>()
        + std::mem::size_of::<(String, InspectedEntry)>()
        + std::mem::size_of::<(String, VerifiedObject)>()
        + std::mem::size_of::<(String, PrepaidObject)>()
        + std::mem::size_of::<(String, Option<ObjectStamp>)>()
        + 4 * std::mem::size_of::<String>());
const MAX_INVENTORY_ROWS: usize = MAX_INVENTORY_METADATA_BYTES / INVENTORY_ROW_SLOT;

#[derive(Clone, Copy, Debug)]
pub(crate) struct InventoryLimits {
    pub rows: usize,
    pub metadata_bytes: usize,
    pub work_bytes: usize,
    pub key_bytes: usize,
}

impl Default for InventoryLimits {
    fn default() -> Self {
        Self {
            rows: MAX_INVENTORY_ROWS,
            metadata_bytes: MAX_INVENTORY_METADATA_BYTES,
            work_bytes: MAX_INVENTORY_WORK_BYTES,
            key_bytes: MAX_INVENTORY_KEY_BYTES,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("cache inventory {resource} exceeds its allowance (limit {limit}, requested {requested})")]
pub struct InventoryLimitExceeded {
    pub resource: &'static str,
    pub limit: usize,
    pub requested: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("cache inventory metadata is unavailable")]
pub struct InventoryUnavailable;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct InventoryReadStats {
    pub scalar_rows: usize,
    pub metadata_bytes: usize,
    pub work_bytes: usize,
    pub peak_scratch_bytes: usize,
    pub entry_copies: usize,
    pub key_copies: usize,
    pub filename_copies: usize,
    pub directory_copies: usize,
    pub inspections: usize,
    pub rejections: usize,
}

#[cfg(test)]
thread_local! {
    static INVENTORY_TEST_LIMITS: std::cell::Cell<Option<InventoryLimits>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_inventory_test_limits<T>(limits: InventoryLimits, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<InventoryLimits>);
    impl Drop for Restore {
        fn drop(&mut self) {
            INVENTORY_TEST_LIMITS.with(|slot| slot.set(self.0));
        }
    }
    let _restore = Restore(INVENTORY_TEST_LIMITS.with(|slot| slot.replace(Some(limits))));
    run()
}

/// One operation's retained collection slots, maximum temporary buffer and
/// cumulative logical processing work. This is not a total-process RSS limit.
pub(crate) struct InventoryBudget {
    pub(crate) limits: InventoryLimits,
    retained: usize,
    scratch: usize,
    work: usize,
    #[cfg(test)]
    observer: Option<Rc<std::cell::Cell<InventoryReadStats>>>,
}

impl InventoryBudget {
    pub(crate) fn new() -> Self {
        let ceiling = InventoryLimits::default();
        #[cfg(test)]
        let limits = INVENTORY_TEST_LIMITS
            .with(std::cell::Cell::get)
            .unwrap_or(ceiling);
        #[cfg(not(test))]
        let limits = ceiling;
        Self {
            limits: InventoryLimits {
                rows: limits.rows.min(ceiling.rows),
                metadata_bytes: limits.metadata_bytes.min(ceiling.metadata_bytes),
                work_bytes: limits.work_bytes.min(ceiling.work_bytes),
                key_bytes: limits.key_bytes.min(ceiling.key_bytes),
            },
            retained: 0,
            scratch: 0,
            work: 0,
            #[cfg(test)]
            observer: None,
        }
    }

    pub(crate) fn add(left: usize, right: usize) -> Result<usize> {
        left.checked_add(right).ok_or_else(|| {
            InventoryLimitExceeded {
                resource: "arithmetic",
                limit: MAX_INVENTORY_METADATA_BYTES,
                requested: usize::MAX,
            }
            .into()
        })
    }

    pub(crate) fn mul(left: usize, right: usize) -> Result<usize> {
        left.checked_mul(right).ok_or_else(|| {
            InventoryLimitExceeded {
                resource: "arithmetic",
                limit: MAX_INVENTORY_METADATA_BYTES,
                requested: usize::MAX,
            }
            .into()
        })
    }

    pub(crate) fn admit(
        &self,
        resource: &'static str,
        requested: usize,
        limit: usize,
    ) -> Result<()> {
        if requested > limit {
            #[cfg(test)]
            if let Some(observer) = &self.observer {
                let mut stats = observer.get();
                stats.rejections += 1;
                observer.set(stats);
            }
            return Err(InventoryLimitExceeded {
                resource,
                limit,
                requested,
            }
            .into());
        }
        Ok(())
    }

    pub(crate) fn rows(&self, count: usize) -> Result<()> {
        self.admit("rows", count, self.limits.rows)
    }

    pub(crate) fn retain(&mut self, bytes: usize) -> Result<()> {
        let next = Self::add(self.retained, bytes)?;
        self.admit(
            "metadata bytes",
            Self::add(next, self.scratch)?,
            self.limits.metadata_bytes,
        )?;
        self.retained = next;
        self.observe();
        Ok(())
    }

    pub(crate) fn scratch(&mut self, bytes: usize) -> Result<()> {
        let next = self.scratch.max(bytes);
        self.admit(
            "metadata bytes",
            Self::add(self.retained, next)?,
            self.limits.metadata_bytes,
        )?;
        self.scratch = next;
        self.observe();
        Ok(())
    }

    pub(crate) fn work(&mut self, bytes: usize) -> Result<()> {
        let next = Self::add(self.work, bytes)?;
        self.admit("work bytes", next, self.limits.work_bytes)?;
        self.work = next;
        self.observe();
        Ok(())
    }

    pub(crate) fn metadata_bytes(&self) -> usize {
        self.retained + self.scratch
    }

    pub(crate) fn remaining_metadata(&self) -> usize {
        self.limits
            .metadata_bytes
            .saturating_sub(self.metadata_bytes())
    }

    fn observe(&self) {
        #[cfg(test)]
        if let Some(observer) = &self.observer {
            let mut stats = observer.get();
            stats.metadata_bytes = stats.metadata_bytes.max(self.metadata_bytes());
            stats.peak_scratch_bytes = stats.peak_scratch_bytes.max(self.scratch);
            stats.work_bytes = stats.work_bytes.max(self.work);
            observer.set(stats);
        }
    }
}

#[derive(Clone, Copy)]
enum InventoryUse {
    Entries,
    Verification,
}

struct DirectoryCandidate {
    filename: String,
    stamp: ObjectStamp,
}

pub(crate) struct RemovalPlan {
    cache_identity: u64,
    entries: Vec<PlannedRemoval>,
    pending: Vec<PlannedCleanup>,
    revision: (u64, u64),
}

struct PlannedRemoval {
    entry: Entry,
    stamp: Option<ObjectStamp>,
}

struct PlannedCleanup {
    filename: String,
    stamp: Option<ObjectStamp>,
}

#[derive(Clone, Copy)]
enum CacheTextEncoding {
    Utf8,
    Utf16,
}

impl CacheTextEncoding {
    fn read(connection: &Connection) -> Result<Self> {
        let encoding: String = connection.pragma_query_value(None, "encoding", |row| row.get(0))?;
        match encoding.as_str() {
            "UTF-8" => Ok(Self::Utf8),
            "UTF-16le" | "UTF-16be" => Ok(Self::Utf16),
            _ => Err(VerificationMetadataLimitExceeded.into()),
        }
    }

    fn utf8_upper(self, encoded: usize) -> Result<usize> {
        match self {
            Self::Utf8 => Ok(encoded),
            Self::Utf16 => encoded
                .checked_add(1)
                .map(|bytes| bytes / 2)
                .and_then(|units| units.checked_mul(3))
                .ok_or_else(|| VerificationMetadataLimitExceeded.into()),
        }
    }

    fn utf8_upper_total(self, encoded: usize, cells: usize) -> Result<usize> {
        match self {
            Self::Utf8 => Ok(encoded),
            Self::Utf16 => encoded
                .checked_add(cells)
                .map(|bytes| bytes / 2)
                .and_then(|units| units.checked_mul(3))
                .ok_or_else(|| InventoryUnavailable.into()),
        }
    }

    fn field_encoded_limit(self, maximum_utf8: usize) -> Result<i64> {
        let maximum = match self {
            Self::Utf8 => maximum_utf8,
            Self::Utf16 => InventoryBudget::mul(maximum_utf8, 2)?,
        };
        i64::try_from(maximum).map_err(|_| InventoryUnavailable.into())
    }

    fn temporary_bytes(self, encoded: usize, upper: usize) -> Result<usize> {
        let raw = InventoryBudget::add(encoded, 3)?;
        let converted = match self {
            Self::Utf8 => upper,
            Self::Utf16 => InventoryBudget::add(InventoryBudget::mul(encoded & !1, 2)?, 1)?,
        };
        InventoryBudget::add(raw, InventoryBudget::add(converted.max(upper), 4096)?)
    }

    fn maximum_encoded(self, maximum_utf8: usize) -> Result<i64> {
        let encoded = match self {
            Self::Utf8 => maximum_utf8,
            Self::Utf16 => (maximum_utf8 / 3)
                .checked_mul(2)
                .ok_or(VerificationMetadataLimitExceeded)?,
        };
        i64::try_from(encoded).map_err(|_| VerificationMetadataLimitExceeded.into())
    }
}

#[derive(Clone, Copy)]
struct EntrySpan {
    key_upper: usize,
    filename_upper: usize,
}

pub(crate) struct KeyInspectionPlan<'a> {
    cache_identity: u64,
    spans: BTreeMap<&'a str, Option<EntrySpan>>,
    metadata_bytes: usize,
    transaction: rusqlite::Transaction<'a>,
    revision: (u64, u64),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct VerificationStats {
    pub charged_bytes: u64,
    pub read_bytes: u64,
    pub hashes: usize,
}

#[derive(Debug, thiserror::Error)]
#[error(
    "cache verification exceeds its work budget (limit {limit}, charged {charged}, requested {requested})"
)]
pub struct VerificationLimitExceeded {
    pub limit: u64,
    pub charged: u64,
    pub requested: u64,
}

#[derive(Debug, thiserror::Error)]
#[error("cache verification metadata exceeds its allowance")]
pub struct VerificationMetadataLimitExceeded;

/// Short-lived evidence for one synchronous operation. This stores fingerprints
/// and digest outcomes, never payload bytes, readers, or background work.
pub(crate) struct VerificationScope {
    limit: u64,
    cache_identity: Option<u64>,
    stats: VerificationStats,
    evidence: HashMap<String, VerifiedObject>,
    prepaid: HashMap<String, PrepaidObject>,
    destinations: Rc<RefCell<DestinationProtection>>,
}

/// Guards share only operation-local metadata, so unwinding cannot bypass the
/// same identity fence used by explicit cleanup. There is no cache back-reference.
#[derive(Default)]
struct DestinationProtection {
    cache_identity: Option<u64>,
    expected: HashMap<String, Option<ObjectStamp>>,
}

impl DestinationProtection {
    fn validate(&self, cache: &FileCache) -> Result<()> {
        self.validate_except(cache, &[])
    }

    fn validate_except(&self, cache: &FileCache, owned: &[&str]) -> Result<()> {
        ensure!(
            self.cache_identity
                .is_none_or(|identity| identity == cache.identity),
            "cache destination protection changed ownership"
        );
        for (filename, expected) in &self.expected {
            if !owned.contains(&filename.as_str()) {
                ensure!(
                    object_stamp(&cache.cleanup_object_path(filename)?)? == *expected,
                    "cache destination appeared or changed after operation preflight"
                );
            }
        }
        Ok(())
    }

    fn remember(
        &mut self,
        cache: &FileCache,
        filename: &str,
        stamp: Option<ObjectStamp>,
    ) -> Result<()> {
        ensure!(
            self.cache_identity == Some(cache.identity),
            "cache destination protection changed ownership"
        );
        if let Some(expected) = self.expected.get(filename) {
            ensure!(
                *expected == stamp,
                "cache destination appeared or changed after operation preflight"
            );
        } else {
            self.expected.insert(filename.to_owned(), stamp);
        }
        Ok(())
    }

    fn check_remembered(
        &self,
        cache: &FileCache,
        filename: &str,
        stamp: &Option<ObjectStamp>,
    ) -> Result<()> {
        ensure!(
            self.cache_identity
                .is_none_or(|identity| identity == cache.identity),
            "cache destination protection changed ownership"
        );
        if let Some(expected) = self.expected.get(filename) {
            ensure!(
                expected == stamp,
                "cache destination appeared or changed after operation preflight"
            );
        }
        Ok(())
    }

    fn removed_by_owner(&mut self, filename: &str) {
        if let Some(expected) = self.expected.get_mut(filename) {
            // A completed unlink authorizes absence, never a later appearance.
            *expected = None;
        }
    }

    fn created_by_owner(&mut self, cache: &FileCache, filename: &str, file: &File) -> Result<()> {
        self.validate_except(cache, &[filename])?;
        ensure!(
            self.expected.get(filename) == Some(&None),
            "cache temporary was not absent before creation"
        );
        let stamp = owned_file_stamp(cache, filename, file)?;
        self.expected.insert(filename.to_owned(), Some(stamp));
        Ok(())
    }

    fn refresh_owned_writer(
        &mut self,
        cache: &FileCache,
        filename: &str,
        file: &File,
    ) -> Result<()> {
        self.validate_except(cache, &[filename])?;
        ensure!(
            self.expected.get(filename).is_some_and(Option::is_some),
            "cache temporary lacks an owned writer"
        );
        // Only the create_new file descriptor may account for its own writes.
        // A path-only refresh would incorrectly adopt a replacement object.
        let stamp = owned_file_stamp(cache, filename, file)?;
        self.expected.insert(filename.to_owned(), Some(stamp));
        Ok(())
    }

    fn renamed_by_owner(
        &mut self,
        cache: &FileCache,
        temporary: &str,
        destination: &str,
        file: &File,
    ) -> Result<ObjectStamp> {
        self.validate_except(cache, &[temporary, destination])?;
        ensure!(
            self.expected.get(temporary).is_some_and(Option::is_some)
                && self.expected.get(destination) == Some(&None)
                && object_stamp(&cache.cleanup_object_path(temporary)?)?.is_none(),
            "cache rename changed its preflight identities"
        );
        let stamp = owned_file_stamp(cache, destination, file)?;
        self.expected.insert(temporary.to_owned(), None);
        self.expected
            .insert(destination.to_owned(), Some(stamp.clone()));
        Ok(stamp)
    }

    fn release_temporary(&mut self, filename: &str) {
        if !valid_filename(filename) && self.expected.get(filename) == Some(&None) {
            self.expected.remove(filename);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ObjectStamp {
    size: u64,
    regular: bool,
    symlink: bool,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64, i64, i64),
}

impl ObjectStamp {
    fn of(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            size: metadata.len(),
            regular: metadata.is_file(),
            symlink: metadata.file_type().is_symlink(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            #[cfg(unix)]
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.mtime(),
                metadata.mtime_nsec(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        }
    }

    fn permits_reuse(&self) -> bool {
        // Portable metadata lacks a stable file identity. Those platforms still
        // deduplicate logical keys in bulk, but never reuse a physical hash on
        // fingerprints alone. Unix operations also compare inode and ctime.
        cfg!(unix)
    }
}

struct VerifiedObject {
    stamp: ObjectStamp,
    expected_size: u64,
    digest_matches: bool,
}

struct PrepaidObject {
    stamp: ObjectStamp,
    expected_size: u64,
    reads_left: usize,
}

struct InspectedEntry {
    entry: Option<Entry>,
    stamp: Option<ObjectStamp>,
    path: Option<PathBuf>,
    invalid: bool,
}

pub(crate) struct CacheInspection {
    cache_identity: u64,
    entries: BTreeMap<String, InspectedEntry>,
    protection: Rc<RefCell<DestinationProtection>>,
    metadata_bytes: usize,
    prepaid: bool,
}

impl VerificationScope {
    pub(crate) fn metadata_scope(&self) -> Self {
        Self {
            limit: self.limit,
            cache_identity: self.cache_identity,
            stats: self.stats,
            evidence: HashMap::new(),
            prepaid: HashMap::new(),
            destinations: Rc::clone(&self.destinations),
        }
    }

    pub(crate) fn new(limit: u64) -> Self {
        Self {
            limit,
            cache_identity: None,
            stats: VerificationStats::default(),
            evidence: HashMap::new(),
            prepaid: HashMap::new(),
            destinations: Rc::new(RefCell::new(DestinationProtection::default())),
        }
    }

    #[cfg(test)]
    pub(crate) fn stats(&self) -> VerificationStats {
        self.stats
    }

    fn bind(&mut self, cache: &FileCache) -> Result<()> {
        ensure!(
            self.cache_identity
                .is_none_or(|identity| identity == cache.identity),
            "cache verification scope belongs to another cache instance"
        );
        self.cache_identity = Some(cache.identity);
        self.destinations.borrow_mut().cache_identity = Some(cache.identity);
        self.limit = self.limit.min(cache.verification_work_limit());
        Ok(())
    }

    fn reserve(&mut self, requested: u64) -> Result<()> {
        let Some(charged) = self
            .stats
            .charged_bytes
            .checked_add(requested)
            .filter(|charged| *charged <= self.limit)
        else {
            return Err(VerificationLimitExceeded {
                limit: self.limit,
                charged: self.stats.charged_bytes,
                requested,
            }
            .into());
        };
        self.stats.charged_bytes = charged;
        Ok(())
    }

    /// A bulk owner prepays its bounded manifest/body evidence before reads.
    /// These bytes share the operation allowance with later unique payloads.
    pub(crate) fn reserve_external_work(&mut self, requested: u64) -> Result<()> {
        self.reserve(requested)
    }
}

impl CacheInspection {
    pub(crate) fn path(&self, key: &str) -> Option<&PathBuf> {
        self.entries.get(key)?.path.as_ref()
    }

    pub(crate) fn has_invalid_entries(&self) -> bool {
        self.entries.values().any(|entry| entry.invalid)
    }

    pub(crate) fn metadata_bytes(&self) -> usize {
        self.metadata_bytes
    }

    /// Consume a snapshot proof without a new hash, but recheck the actual
    /// logical row and physical identity for every bulk lookup.
    pub(crate) fn current_path<'a>(
        &'a self,
        cache: &FileCache,
        key: &str,
    ) -> Result<Option<&'a PathBuf>> {
        ensure!(
            cache.identity == self.cache_identity,
            "cache inspection changed ownership"
        );
        let Some(inspected) = self.entries.get(key) else {
            return Ok(None);
        };
        ensure!(
            cache.entry_matches(key, inspected.entry.as_ref())?,
            "cached object changed during verification"
        );
        if let Some(entry) = &inspected.entry {
            ensure!(
                object_stamp(&cache.object_path(&entry.filename)?)? == inspected.stamp,
                "cached file changed during verification"
            );
        }
        Ok(inspected.path.as_ref())
    }

    /// Recheck all identities before the first mutation. A quota rejection during
    /// inspection therefore cannot partially retire data or change its usage.
    pub(crate) fn finish(&self, cache: &FileCache, touch: bool) -> Result<()> {
        ensure!(
            cache.identity == self.cache_identity,
            "cache inspection changed ownership"
        );
        self.protection.borrow().validate(cache)?;
        for (key, inspected) in &self.entries {
            let unchanged = if self.prepaid {
                cache.entry_matches(key, inspected.entry.as_ref())?
            } else {
                let current = cache.entry(key)?;
                match (&current, &inspected.entry) {
                    (None, None) => true,
                    (Some(current), Some(previous)) => {
                        current.key == previous.key
                            && current.filename == previous.filename
                            && current.size == previous.size
                    }
                    _ => false,
                }
            };
            ensure!(unchanged, "cached object changed during verification");
            if let Some(entry) = &inspected.entry {
                ensure!(
                    object_stamp(&cache.object_path(&entry.filename)?)? == inspected.stamp,
                    "cached file changed during verification"
                );
            }
        }
        for inspected in self.entries.values().filter(|entry| entry.invalid) {
            if let Some(entry) = &inspected.entry {
                cache.check_captured_object(
                    &entry.filename,
                    &inspected.stamp,
                    Some(&self.protection),
                )?;
                cache.retire_entry_using_protection(entry, Some(&self.protection))?;
                cache.cleanup_captured_pending_object(
                    &entry.filename,
                    &inspected.stamp,
                    Some(&self.protection),
                )?;
            }
        }
        if touch {
            for (key, inspected) in &self.entries {
                if inspected.path.is_some() {
                    // A logical key is touched once per successful bulk access,
                    // not once for every occurrence of its physical image.
                    self.protection.borrow().validate(cache)?;
                    cache.touch(key)?;
                }
            }
        }
        Ok(())
    }
}

/// A durable cleanup record precedes publication of an unindexed payload. The
/// guard handles unwinding; explicit error paths also surface cleanup failures.
struct PendingObject<'a> {
    cache: &'a FileCache,
    filename: String,
    armed: bool,
    protection: Option<Rc<RefCell<DestinationProtection>>>,
    created: bool,
}

impl<'a> PendingObject<'a> {
    #[cfg(test)]
    fn new(cache: &'a FileCache, filename: &str, size: u64) -> Result<Self> {
        Self::new_using_protection(cache, filename, size, None)
    }

    fn new_using_protection(
        cache: &'a FileCache,
        filename: &str,
        size: u64,
        protection: Option<&Rc<RefCell<DestinationProtection>>>,
    ) -> Result<Self> {
        if let Some(protection) = protection {
            let mut protection = protection.borrow_mut();
            protection.validate(cache)?;
            let stamp = object_stamp(&cache.cleanup_object_path(filename)?)?;
            if !valid_filename(filename) {
                ensure!(stamp.is_none(), "cache temporary appeared before creation");
            }
            protection.remember(cache, filename, stamp)?;
        }
        cache.schedule_cleanup(filename, size)?;
        Ok(Self {
            cache,
            filename: filename.to_owned(),
            armed: true,
            protection: protection.cloned(),
            created: false,
        })
    }

    fn record_created(&mut self, file: &File) -> Result<()> {
        if let Some(protection) = &self.protection {
            protection
                .borrow_mut()
                .created_by_owner(self.cache, &self.filename, file)?;
        }
        self.created = true;
        Ok(())
    }

    fn refresh_created(&self, file: &File) -> Result<()> {
        if self.created {
            if let Some(protection) = &self.protection {
                protection
                    .borrow_mut()
                    .refresh_owned_writer(self.cache, &self.filename, file)?;
            }
        }
        Ok(())
    }

    fn record_rename(&mut self, destination: &str, file: &File) -> Result<ObjectStamp> {
        ensure!(self.created, "cache temporary has no owned writer");
        let stamp = match &self.protection {
            Some(protection) => protection.borrow_mut().renamed_by_owner(
                self.cache,
                &self.filename,
                destination,
                file,
            )?,
            None => owned_file_stamp(self.cache, destination, file)?,
        };
        self.created = false;
        Ok(stamp)
    }

    fn cleanup(&mut self) -> Result<()> {
        self.cache
            .cleanup_pending_object_using_protection(&self.filename, self.protection.as_ref())?;
        self.armed = false;
        Ok(())
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingObject<'_> {
    fn drop(&mut self) {
        if self.armed {
            // A failed unlink keeps its durable record and blocks subsequent
            // writes until cleanup succeeds, rather than losing its budget.
            let _ = self
                .cache
                .cleanup_pending_object_using_protection(&self.filename, self.protection.as_ref());
        }
    }
}

impl FileCache {
    pub fn open(root: impl AsRef<Path>, maximum_bytes: u64) -> Result<Self> {
        ensure!(
            maximum_bytes > 0 && maximum_bytes <= i64::MAX as u64,
            "invalid cache budget"
        );
        fs::create_dir_all(root.as_ref()).context("create file cache directory")?;
        let root = root
            .as_ref()
            .canonicalize()
            .context("resolve file cache directory")?;
        let index = root.join("cache-index.sqlite3");
        if let Ok(metadata) = fs::symlink_metadata(&index) {
            ensure!(
                !metadata.file_type().is_symlink(),
                "cache index must not be a symlink"
            );
        }
        let connection = Connection::open(&index).context("open cache index")?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS cache_entries (
                key TEXT PRIMARY KEY NOT NULL,
                filename TEXT NOT NULL,
                size INTEGER NOT NULL CHECK(size >= 0),
                last_access INTEGER NOT NULL,
                pinned INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS cache_lru ON cache_entries(pinned,last_access);
            CREATE TABLE IF NOT EXISTS cache_pending_cleanup (
                filename TEXT PRIMARY KEY NOT NULL,
                size INTEGER NOT NULL CHECK(size >= 0)
            );
        ",
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
            fs::set_permissions(index, fs::Permissions::from_mode(0o600))?;
        }
        let cache = Self {
            root,
            maximum_bytes,
            text_encoding: CacheTextEncoding::read(&connection)?,
            connection,
            identity: NEXT_CACHE_ID.fetch_add(1, Ordering::Relaxed),
            #[cfg(test)]
            inventory_reads: Rc::new(std::cell::Cell::new(InventoryReadStats::default())),
        };
        cache.recover()?;
        cache.evict_to_budget()?;
        Ok(cache)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn maximum_bytes(&self) -> u64 {
        self.maximum_bytes
    }

    pub(crate) fn inventory_budget(&self) -> InventoryBudget {
        let mut budget = InventoryBudget::new();
        #[cfg(test)]
        {
            budget.observer = Some(Rc::clone(&self.inventory_reads));
        }
        budget
    }

    pub(crate) fn inventory_revision(&self) -> Result<(u64, u64)> {
        let version = self
            .connection
            .pragma_query_value(None, "data_version", |row| row.get::<_, u64>(0))?;
        Ok((version, self.connection.total_changes()))
    }

    pub(crate) fn bind_verification(&self, scope: &mut VerificationScope) -> Result<()> {
        scope.bind(self)
    }

    pub(crate) fn admit_write_inventory(
        &self,
        additions: &[(&str, &[u8])],
        budget: &mut InventoryBudget,
    ) -> Result<()> {
        if additions.is_empty() {
            return Ok(());
        }
        budget.rows(additions.len())?;
        let mut added_bytes = 0_u64;
        let mut added_names = 0_usize;
        for (key, bytes) in additions {
            budget.admit("key bytes", key.len(), budget.limits.key_bytes)?;
            added_bytes = added_bytes
                .checked_add(bytes.len() as u64)
                .context("cache addition size overflow")?;
            added_names = InventoryBudget::add(added_names, key.len())?;
        }
        budget.retain(InventoryBudget::add(
            InventoryBudget::mul(
                additions.len(),
                4 * (std::mem::size_of::<String>() + MAX_INVENTORY_FILENAME_UPPER),
            )?,
            added_names,
        )?)?;
        budget.work(InventoryBudget::add(
            InventoryBudget::mul(added_names, 4)?,
            InventoryBudget::mul(additions.len(), 1024)?,
        )?)?;
        let indexed: i64 = self.connection.query_row(
            "SELECT COALESCE(SUM(size),0) FROM cache_entries",
            [],
            |row| row.get(0),
        )?;
        let indexed = u64::try_from(indexed).map_err(|_| InventoryUnavailable)?;
        if indexed.saturating_add(added_bytes) > self.maximum_bytes {
            let count: i64 =
                self.connection
                    .query_row("SELECT COUNT(*) FROM cache_entries", [], |row| row.get(0))?;
            budget.rows(InventoryBudget::add(
                usize::try_from(count).map_err(|_| InventoryUnavailable)?,
                additions.len(),
            )?)?;
            self.inventory_entries_phase(budget, InventoryUse::Entries, false, false, false)?;
            // A later put may see every newly admitted record in its eviction
            // snapshot. Price that largest future collection before put/pins.
            let key_upper = self.text_encoding.utf8_upper(
                usize::try_from(self.text_encoding.field_encoded_limit(added_names)?)
                    .map_err(|_| InventoryUnavailable)?,
            )?;
            let filenames = InventoryBudget::mul(additions.len(), MAX_INVENTORY_FILENAME_UPPER)?;
            budget.retain(InventoryBudget::add(
                InventoryBudget::mul(
                    additions.len(),
                    4 * std::mem::size_of::<Entry>() + 2 * std::mem::size_of::<String>(),
                )?,
                InventoryBudget::mul(InventoryBudget::add(key_upper, filenames)?, 2)?,
            )?)?;
            budget.work(InventoryBudget::add(
                InventoryBudget::mul(InventoryBudget::add(key_upper, filenames)?, 32)?,
                InventoryBudget::mul(additions.len(), 256)?,
            )?)?;
        }
        self.admit_cleanup_inventory(budget)
    }

    pub(crate) fn plan_removal<'a, I>(
        &'a self,
        keys: I,
        budget: &mut InventoryBudget,
        scope: &mut VerificationScope,
    ) -> Result<RemovalPlan>
    where
        I: ExactSizeIterator<Item = &'a str> + Clone,
    {
        let plan = self.plan_borrowed_key_inspection(keys, budget)?;
        let pending_names = self.pending_cleanup_names(budget)?;
        scope.bind(self)?;
        self.validate_verification(scope)?;
        let mut entries = Vec::with_capacity(plan.spans.len());
        for (key, span) in plan.spans {
            if let Some(entry) = self.bounded_entry(key, span, self.text_encoding)? {
                let stamp = object_stamp(&self.object_path(&entry.filename)?)?;
                scope
                    .destinations
                    .borrow()
                    .check_remembered(self, &entry.filename, &stamp)?;
                entries.push(PlannedRemoval { entry, stamp });
            }
        }
        let pending =
            self.capture_cleanup_names(pending_names, budget, Some(&scope.destinations))?;
        plan.transaction.commit()?;
        ensure!(
            self.inventory_revision()? == plan.revision,
            InventoryUnavailable
        );
        Ok(RemovalPlan {
            cache_identity: self.identity,
            entries,
            pending,
            revision: plan.revision,
        })
    }

    pub(crate) fn apply_removal(
        &self,
        plan: &RemovalPlan,
        scope: &VerificationScope,
        remove: impl Fn(&str) -> bool,
    ) -> Result<()> {
        ensure!(
            plan.cache_identity == self.identity,
            "cache removal plan changed ownership"
        );
        ensure!(
            self.inventory_revision()?.0 == plan.revision.0,
            InventoryUnavailable
        );
        self.validate_verification(scope)?;
        // The complete current identity check is read-only. Missing rows can
        // have been retired/evicted by this same admitted operation already.
        for removal in plan
            .entries
            .iter()
            .filter(|removal| remove(&removal.entry.key))
        {
            let entry = &removal.entry;
            if self.entry_matches(&entry.key, None)? {
                continue;
            }
            ensure!(
                self.entry_matches(&entry.key, Some(entry))?,
                "cache removal evidence changed"
            );
            self.check_captured_object(&entry.filename, &removal.stamp, Some(&scope.destinations))?;
        }
        for pending in &plan.pending {
            let still_pending = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM cache_pending_cleanup AS pending WHERE filename=?1
                 AND NOT EXISTS(SELECT 1 FROM cache_entries AS active WHERE active.filename=pending.filename))", [&pending.filename],
                |row| row.get::<_, bool>(0),
            )?;
            if still_pending {
                self.check_captured_object(
                    &pending.filename,
                    &pending.stamp,
                    Some(&scope.destinations),
                )?;
            }
        }
        for pending in &plan.pending {
            self.cleanup_captured_pending_object(
                &pending.filename,
                &pending.stamp,
                Some(&scope.destinations),
            )?;
        }
        for removal in &plan.entries {
            let entry = &removal.entry;
            if !remove(&entry.key) || self.entry_matches(&entry.key, None)? {
                continue;
            }
            ensure!(
                self.inventory_revision()?.0 == plan.revision.0,
                InventoryUnavailable
            );
            ensure!(
                self.entry_matches(&entry.key, Some(entry))?,
                "cache removal evidence changed"
            );
            self.check_captured_object(&entry.filename, &removal.stamp, Some(&scope.destinations))?;
            self.validate_verification(scope)?;
            self.set_pinned_with_verification(&entry.key, false, scope)?;
            self.retire_entry_using_protection(entry, Some(&scope.destinations))?;
            self.cleanup_captured_pending_object(
                &entry.filename,
                &removal.stamp,
                Some(&scope.destinations),
            )?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn inventory_read_stats(&self) -> InventoryReadStats {
        self.inventory_reads.get()
    }

    #[cfg(test)]
    pub(crate) fn reset_inventory_read_stats(&self) {
        self.inventory_reads.set(InventoryReadStats::default());
    }

    #[cfg(test)]
    fn observe_inventory(&self, update: impl FnOnce(&mut InventoryReadStats)) {
        let mut stats = self.inventory_reads.get();
        update(&mut stats);
        self.inventory_reads.set(stats);
    }

    fn inventory_entries(
        &self,
        budget: &mut InventoryBudget,
        usage: InventoryUse,
        pinned_only: bool,
        unpinned_only: bool,
    ) -> Result<Vec<Entry>> {
        self.inventory_entries_phase(budget, usage, pinned_only, unpinned_only, true)
    }

    fn inventory_entries_phase(
        &self,
        budget: &mut InventoryBudget,
        usage: InventoryUse,
        pinned_only: bool,
        unpinned_only: bool,
        materialize: bool,
    ) -> Result<Vec<Entry>> {
        let transaction = if self.connection.is_autocommit() {
            Some(self.connection.unchecked_transaction()?)
        } else {
            None
        };
        let connection = transaction.as_deref().unwrap_or(&self.connection);
        let read_limit = i64::try_from(InventoryBudget::add(budget.limits.rows, 1)?)
            .map_err(|_| InventoryUnavailable)?;
        let (count, key_bytes, filename_bytes, key_cells, filename_cells, largest_key, largest_filename, valid):
            (i64, i64, i64, i64, i64, i64, i64, bool) = connection.query_row(
            "SELECT COUNT(*),COALESCE(SUM(k),0),COALESCE(SUM(f),0),COALESCE(SUM(k>0),0),COALESCE(SUM(f>0),0),
             COALESCE(MAX(k),0),COALESCE(MAX(f),0),COALESCE(MIN(valid),1) FROM
             (SELECT COALESCE(octet_length(key),0) AS k,COALESCE(octet_length(filename),0) AS f,
              typeof(key)='text' AND typeof(filename)='text' AND typeof(size)='integer' AND size>=0
              AND typeof(pinned)='integer' AND typeof(last_access)='integer' AS valid
              FROM cache_entries WHERE (?1=0 OR pinned<>0) AND (?2=0 OR pinned=0) LIMIT ?3)",
            params![pinned_only, unpinned_only, read_limit],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)),
        )?;
        if !valid {
            return Err(InventoryUnavailable.into());
        }
        let count = usize::try_from(count).map_err(|_| InventoryUnavailable)?;
        budget.rows(count)?;
        if count == 0 {
            if let Some(transaction) = transaction {
                transaction.commit()?;
            }
            return Ok(Vec::new());
        }
        let key_encoded = usize::try_from(key_bytes).map_err(|_| InventoryUnavailable)?;
        let filename_encoded = usize::try_from(filename_bytes).map_err(|_| InventoryUnavailable)?;
        let keys = self.text_encoding.utf8_upper_total(
            key_encoded,
            usize::try_from(key_cells).map_err(|_| InventoryUnavailable)?,
        )?;
        let filenames = self.text_encoding.utf8_upper_total(
            filename_encoded,
            usize::try_from(filename_cells).map_err(|_| InventoryUnavailable)?,
        )?;
        budget.admit(
            "key bytes",
            usize::try_from(largest_key).map_err(|_| InventoryUnavailable)?,
            usize::try_from(
                self.text_encoding
                    .field_encoded_limit(budget.limits.key_bytes)?,
            )
            .map_err(|_| InventoryUnavailable)?,
        )?;
        let largest_filename = self
            .text_encoding
            .utf8_upper(usize::try_from(largest_filename).map_err(|_| InventoryUnavailable)?)?;
        budget.admit(
            "filename bytes",
            largest_filename,
            MAX_INVENTORY_FILENAME_UPPER,
        )?;
        let path_bytes = InventoryBudget::mul(
            InventoryBudget::add(self.root.as_os_str().as_encoded_bytes().len(), 1)?,
            2,
        )?;
        let (slots, key_copies, filename_copies) = match usage {
            InventoryUse::Verification => (INVENTORY_ROW_SLOT + path_bytes, 4, 8),
            _ => (
                2 * std::mem::size_of::<Entry>()
                    + 2 * std::mem::size_of::<PlannedRemoval>()
                    + 2 * std::mem::size_of::<String>(),
                2,
                2,
            ),
        };
        budget.retain(InventoryBudget::add(
            InventoryBudget::mul(count, slots)?,
            InventoryBudget::add(
                InventoryBudget::mul(keys, key_copies)?,
                InventoryBudget::mul(filenames, filename_copies)?,
            )?,
        )?)?;
        // The row allowance includes sort comparisons, copying, membership and
        // repeated single-key scalar checks. It is not a SQL wall-clock bound.
        budget.work(InventoryBudget::add(
            InventoryBudget::mul(InventoryBudget::add(keys, filenames)?, 32)?,
            InventoryBudget::mul(
                count,
                InventoryBudget::add(InventoryBudget::mul(path_bytes, 4)?, 256)?,
            )?,
        )?)?;
        budget.scratch(InventoryBudget::add(
            path_bytes,
            2 * MAX_INVENTORY_FILENAME_UPPER,
        )?)?;
        let encoded = InventoryBudget::add(
            usize::try_from(largest_key).map_err(|_| InventoryUnavailable)?,
            usize::try_from(
                self.text_encoding
                    .field_encoded_limit(MAX_INVENTORY_FILENAME_UPPER)?,
            )
            .map_err(|_| InventoryUnavailable)?,
        )?;
        let upper = self.text_encoding.utf8_upper(encoded)?;
        budget.scratch(self.text_encoding.temporary_bytes(encoded, upper)?)?;
        #[cfg(test)]
        self.observe_inventory(|stats| stats.scalar_rows += count);
        if !materialize {
            if let Some(transaction) = transaction {
                transaction.commit()?;
            }
            return Ok(Vec::new());
        }
        #[cfg(test)]
        inventory_tests::after_preflight()?;
        let mut entries = Vec::with_capacity(count);
        {
            let mut statement = connection.prepare(
                "SELECT key,filename,size,pinned FROM cache_entries
                WHERE (?1=0 OR pinned<>0) AND (?2=0 OR pinned=0) ORDER BY last_access,key",
            )?;
            let mut rows = statement.query(params![pinned_only, unpinned_only])?;
            while let Some(row) = rows.next()? {
                let key_value = row.get_ref(0)?;
                let key = key_value.as_str().map_err(|_| InventoryUnavailable)?;
                let filename_value = row.get_ref(1)?;
                let filename = filename_value.as_str().map_err(|_| InventoryUnavailable)?;
                budget.admit("key bytes", key.len(), budget.limits.key_bytes)?;
                budget.admit(
                    "filename bytes",
                    filename.len(),
                    MAX_INVENTORY_FILENAME_UPPER,
                )?;
                let size =
                    u64::try_from(row.get::<_, i64>(2)?).map_err(|_| InventoryUnavailable)?;
                let pinned = row.get(3)?;
                #[cfg(test)]
                self.observe_inventory(|stats| {
                    stats.entry_copies += 1;
                    stats.key_copies += 1;
                    stats.filename_copies += 1;
                });
                entries.push(Entry {
                    key: key.to_owned(),
                    filename: filename.to_owned(),
                    size,
                    pinned,
                });
            }
        }
        ensure!(entries.len() == count, InventoryUnavailable);
        if let Some(transaction) = transaction {
            transaction.commit()?;
        }
        Ok(entries)
    }

    pub(crate) fn inventory_keys_with_budget(
        &self,
        budget: &mut InventoryBudget,
        pinned_only: bool,
    ) -> Result<Vec<String>> {
        let transaction = self.connection.unchecked_transaction()?;
        let limit = i64::try_from(InventoryBudget::add(budget.limits.rows, 1)?)
            .map_err(|_| InventoryUnavailable)?;
        let (count, encoded, cells, maximum, valid): (i64,i64,i64,i64,bool) = transaction.query_row(
            "SELECT COUNT(*),COALESCE(SUM(bytes),0),COALESCE(SUM(bytes>0),0),COALESCE(MAX(bytes),0),COALESCE(MIN(valid),1)
              FROM (SELECT COALESCE(octet_length(key),0) AS bytes,
               typeof(key)='text' AND (?1=0 OR typeof(pinned)='integer') AS valid
              FROM cache_entries WHERE (?1=0 OR pinned<>0) LIMIT ?2)",
            params![pinned_only, limit],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        )?;
        if !valid {
            return Err(InventoryUnavailable.into());
        }
        let count = usize::try_from(count).map_err(|_| InventoryUnavailable)?;
        budget.rows(count)?;
        if count == 0 {
            transaction.commit()?;
            return Ok(Vec::new());
        }
        let encoded = usize::try_from(encoded).map_err(|_| InventoryUnavailable)?;
        let upper = self.text_encoding.utf8_upper_total(
            encoded,
            usize::try_from(cells).map_err(|_| InventoryUnavailable)?,
        )?;
        let maximum = usize::try_from(maximum).map_err(|_| InventoryUnavailable)?;
        budget.admit(
            "key bytes",
            maximum,
            usize::try_from(
                self.text_encoding
                    .field_encoded_limit(budget.limits.key_bytes)?,
            )
            .map_err(|_| InventoryUnavailable)?,
        )?;
        budget.retain(InventoryBudget::add(
            upper,
            InventoryBudget::mul(count, 2 * std::mem::size_of::<String>() + 64)?,
        )?)?;
        budget.work(InventoryBudget::add(
            InventoryBudget::mul(upper, 32)?,
            InventoryBudget::mul(count, 64)?,
        )?)?;
        budget.scratch(
            self.text_encoding
                .temporary_bytes(maximum, self.text_encoding.utf8_upper(maximum)?)?,
        )?;
        #[cfg(test)]
        {
            self.observe_inventory(|stats| stats.scalar_rows += count);
            inventory_tests::after_preflight()?;
        }
        let mut keys = Vec::with_capacity(count);
        {
            let mut statement = transaction.prepare(
                "SELECT key FROM cache_entries WHERE (?1=0 OR pinned<>0) ORDER BY last_access,key",
            )?;
            let mut rows = statement.query([pinned_only])?;
            while let Some(row) = rows.next()? {
                let value = row.get_ref(0)?;
                let key = value.as_str().map_err(|_| InventoryUnavailable)?;
                budget.admit("key bytes", key.len(), budget.limits.key_bytes)?;
                #[cfg(test)]
                self.observe_inventory(|stats| stats.key_copies += 1);
                keys.push(key.to_owned());
            }
        }
        ensure!(keys.len() == count, InventoryUnavailable);
        transaction.commit()?;
        Ok(keys)
    }

    fn pending_cleanup_names(&self, budget: &mut InventoryBudget) -> Result<Vec<String>> {
        self.pending_cleanup_names_phase(budget, true)
    }

    fn pending_cleanup_names_phase(
        &self,
        budget: &mut InventoryBudget,
        materialize: bool,
    ) -> Result<Vec<String>> {
        let transaction = if self.connection.is_autocommit() {
            Some(self.connection.unchecked_transaction()?)
        } else {
            None
        };
        let connection = transaction.as_deref().unwrap_or(&self.connection);
        let limit = i64::try_from(InventoryBudget::add(budget.limits.rows, 1)?)
            .map_err(|_| InventoryUnavailable)?;
        let (count, encoded, cells, largest, valid): (i64,i64,i64,i64,bool) = connection.query_row(
            "SELECT COUNT(*),COALESCE(SUM(bytes),0),COALESCE(SUM(bytes>0),0),COALESCE(MAX(bytes),0),COALESCE(MIN(valid),1)
             FROM (SELECT COALESCE(octet_length(filename),0) AS bytes,
              typeof(filename)='text' AND typeof(size)='integer' AND size>=0 AS valid
              FROM cache_pending_cleanup LIMIT ?1)",
            [limit], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        )?;
        if !valid {
            return Err(InventoryUnavailable.into());
        }
        let count = usize::try_from(count).map_err(|_| InventoryUnavailable)?;
        budget.rows(count)?;
        if count == 0 {
            if let Some(transaction) = transaction {
                transaction.commit()?;
            }
            return Ok(Vec::new());
        }
        let encoded = usize::try_from(encoded).map_err(|_| InventoryUnavailable)?;
        let upper = self.text_encoding.utf8_upper_total(
            encoded,
            usize::try_from(cells).map_err(|_| InventoryUnavailable)?,
        )?;
        let largest = usize::try_from(largest).map_err(|_| InventoryUnavailable)?;
        budget.admit(
            "filename bytes",
            self.text_encoding.utf8_upper(largest)?,
            MAX_INVENTORY_FILENAME_UPPER,
        )?;
        let path = InventoryBudget::add(
            self.root.as_os_str().as_encoded_bytes().len(),
            MAX_INVENTORY_FILENAME_UPPER + 1,
        )?;
        budget.retain(InventoryBudget::add(
            upper,
            InventoryBudget::mul(
                count,
                2 * std::mem::size_of::<String>() + 64 + 2 * std::mem::size_of::<PlannedCleanup>(),
            )?,
        )?)?;
        budget.work(InventoryBudget::add(
            InventoryBudget::mul(upper, 32)?,
            InventoryBudget::mul(
                count,
                InventoryBudget::add(InventoryBudget::mul(path, 4)?, 320)?,
            )?,
        )?)?;
        budget.scratch(
            self.text_encoding
                .temporary_bytes(largest, self.text_encoding.utf8_upper(largest)?)?
                .max(InventoryBudget::mul(path, 2)?),
        )?;
        #[cfg(test)]
        self.observe_inventory(|stats| stats.scalar_rows += count);
        if !materialize {
            if let Some(transaction) = transaction {
                transaction.commit()?;
            }
            return Ok(Vec::new());
        }
        let mut names = Vec::with_capacity(count);
        {
            let mut statement = connection
                .prepare("SELECT filename FROM cache_pending_cleanup ORDER BY filename")?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let value = row.get_ref(0)?;
                let filename = value.as_str().map_err(|_| InventoryUnavailable)?;
                ensure!(
                    valid_cleanup_filename(filename),
                    "invalid pending cache object filename"
                );
                #[cfg(test)]
                self.observe_inventory(|stats| stats.filename_copies += 1);
                names.push(filename.to_owned());
            }
        }
        ensure!(names.len() == count, InventoryUnavailable);
        if let Some(transaction) = transaction {
            transaction.commit()?;
        }
        Ok(names)
    }

    pub(crate) fn admit_cleanup_inventory(&self, budget: &mut InventoryBudget) -> Result<()> {
        // This small receipt is also useful before a caller mutates manifests.
        // No cleanup, path ownership or pin permission follows from admission.
        let names = self.pending_cleanup_names_phase(budget, false)?;
        drop(names);
        Ok(())
    }

    fn capture_cleanup_names(
        &self,
        names: Vec<String>,
        _budget: &mut InventoryBudget,
        protection: Option<&Rc<RefCell<DestinationProtection>>>,
    ) -> Result<Vec<PlannedCleanup>> {
        // The numeric cleanup phase priced this second live vector, every
        // captured stamp and maximum path scratch before the first name copy.
        let mut pending = Vec::with_capacity(names.len());
        for filename in names {
            let stamp = object_stamp(&self.cleanup_object_path(&filename)?)?;
            if let Some(protection) = protection {
                protection
                    .borrow()
                    .check_remembered(self, &filename, &stamp)?;
            }
            pending.push(PlannedCleanup { filename, stamp });
        }
        Ok(pending)
    }

    fn check_cleanup_plan(
        &self,
        pending: &[PlannedCleanup],
        protection: Option<&Rc<RefCell<DestinationProtection>>>,
    ) -> Result<()> {
        for cleanup in pending {
            self.check_captured_object(&cleanup.filename, &cleanup.stamp, protection)?;
        }
        Ok(())
    }

    fn capture_removals(&self, entries: Vec<Entry>) -> Result<Vec<PlannedRemoval>> {
        // inventory_entries has already priced both live vectors, path scratch
        // and the captured stamp work. No payload or owned text is duplicated.
        let mut removals = Vec::with_capacity(entries.len());
        for entry in entries {
            let stamp = object_stamp(&self.object_path(&entry.filename)?)?;
            removals.push(PlannedRemoval { entry, stamp });
        }
        Ok(removals)
    }

    fn paired_inventory(
        &self,
        budget: &mut InventoryBudget,
        usage: InventoryUse,
        unpinned_only: bool,
    ) -> Result<(Vec<Entry>, Vec<String>)> {
        let transaction = self.connection.unchecked_transaction()?;
        if unpinned_only
            && transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM cache_entries WHERE typeof(pinned)<>'integer')",
                [],
                |row| row.get::<_, bool>(0),
            )?
        {
            return Err(InventoryUnavailable.into());
        }
        // Price both complete collections under the same snapshot before the
        // first owned key, filename or Entry vector. The second numeric pass
        // rechecks the receipt; sort/copy passes were already included above.
        self.inventory_entries_phase(budget, usage, false, unpinned_only, false)?;
        self.pending_cleanup_names_phase(budget, false)?;
        let mut emission = InventoryBudget::new();
        emission.limits = budget.limits;
        let entries =
            self.inventory_entries_phase(&mut emission, usage, false, unpinned_only, true)?;
        let names = self.pending_cleanup_names_phase(&mut emission, true)?;
        transaction.commit()?;
        Ok((entries, names))
    }

    /// Allow one full configured cache plus bounded replacement/probe overhead.
    /// Reader and chapter callers can select a smaller operation work allowance.
    pub(crate) fn verification_work_limit(&self) -> u64 {
        self.maximum_bytes.saturating_mul(2)
    }

    /// Retry recorded retirements without losing any preflight identity fence.
    pub(crate) fn retry_pending_cleanup_with_verification(
        &self,
        scope: &VerificationScope,
    ) -> Result<()> {
        self.cleanup_pending_using_protection(Some(&scope.destinations))
    }

    /// Check the complete read-only snapshot before application-owned mutation.
    pub(crate) fn validate_verification(&self, scope: &VerificationScope) -> Result<()> {
        scope.destinations.borrow().validate(self)
    }

    pub fn usage(&self) -> Result<u64> {
        Ok(self.connection.query_row(
            "SELECT (SELECT COALESCE(SUM(size),0) FROM cache_entries)
                  + (SELECT COALESCE(SUM(size),0) FROM cache_pending_cleanup AS pending
                     WHERE NOT EXISTS (SELECT 1 FROM cache_entries AS active
                                       WHERE active.filename=pending.filename))",
            [],
            |row| row.get::<_, i64>(0),
        )? as u64)
    }

    /// Snapshot logical identities for application-owned recovery and reconciliation.
    pub fn keys(&self) -> Result<Vec<String>> {
        self.inventory_keys_with_budget(&mut self.inventory_budget(), false)
    }

    pub fn pinned_keys(&self) -> Result<Vec<String>> {
        self.inventory_keys_with_budget(&mut self.inventory_budget(), true)
    }

    pub fn get(&self, key: &str) -> Result<Option<PathBuf>> {
        let path = self.get_for_commit(key)?;
        if path.is_some() {
            self.touch(key)?;
        }
        Ok(path)
    }

    fn touch(&self, key: &str) -> Result<()> {
        self.connection.execute("UPDATE cache_entries SET last_access=(SELECT COALESCE(MAX(last_access),0)+1 FROM cache_entries) WHERE key=?1", [key])?;
        Ok(())
    }

    /// Validate an object for a download commit without changing its LRU position.
    pub fn get_for_commit(&self, key: &str) -> Result<Option<PathBuf>> {
        let mut scope = VerificationScope::new(self.verification_work_limit());
        let inspected = self.inspect_keys(&[key.to_owned()], &mut scope)?;
        inspected.finish(self, false)?;
        Ok(inspected.path(key).cloned())
    }

    /// Inspection performs no index, pin, payload or recency mutation. Its owner
    /// must finish only after the whole operation passes its verification quota.
    pub(crate) fn inspect_keys(
        &self,
        keys: &[String],
        scope: &mut VerificationScope,
    ) -> Result<CacheInspection> {
        let mut budget = self.inventory_budget();
        let plan = self.plan_key_inspection(keys, &mut budget)?;
        self.inspect_planned_keys(plan, scope)
    }

    pub(crate) fn plan_key_inspection<'a>(
        &'a self,
        keys: &'a [String],
        budget: &mut InventoryBudget,
    ) -> Result<KeyInspectionPlan<'a>> {
        self.plan_borrowed_key_inspection(keys.iter().map(String::as_str), budget)
    }

    /// Price a later subset lookup before an owner starts recovery or publishing.
    /// This receipt owns no Entry, payload proof or mutation permission.
    pub(crate) fn admit_inspection_inventory<'a, I>(
        &'a self,
        keys: I,
        budget: &mut InventoryBudget,
    ) -> Result<()>
    where
        I: ExactSizeIterator<Item = &'a str> + Clone,
    {
        let plan = self.plan_borrowed_key_inspection(keys, budget)?;
        plan.transaction.commit()?;
        ensure!(
            self.inventory_revision()? == plan.revision,
            InventoryUnavailable
        );
        Ok(())
    }

    pub(crate) fn plan_borrowed_key_inspection<'a, I>(
        &'a self,
        keys: I,
        budget: &mut InventoryBudget,
    ) -> Result<KeyInspectionPlan<'a>>
    where
        I: ExactSizeIterator<Item = &'a str> + Clone,
    {
        let count = keys.len();
        budget.rows(count)?;
        for key in keys.clone() {
            budget.admit("key bytes", key.len(), budget.limits.key_bytes)?;
            budget.retain(InventoryBudget::add(
                INVENTORY_ROW_SLOT,
                InventoryBudget::mul(key.len(), 2)?,
            )?)?;
            budget.work(InventoryBudget::add(
                InventoryBudget::mul(key.len(), 32)?,
                128,
            )?)?;
        }
        if count != 0 {
            budget.scratch(32 * 1024)?;
        }
        // The input slots pay for this borrowed numeric staging vector before
        // any uniqueness table or owned cache row is created.
        let revision = self.inventory_revision()?;
        let transaction = self.connection.unchecked_transaction()?;
        let mut numeric = Vec::with_capacity(count);
        for key in keys {
            let span: Option<(bool,bool,i64,i64)> = transaction.query_row(
                "SELECT typeof(key)='text',typeof(filename)='text',COALESCE(octet_length(key),0),
                 COALESCE(octet_length(filename),0) FROM cache_entries WHERE key=?1",
                [key], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
            ).optional()?;
            let span = if let Some((key_text, filename_text, key_bytes, filename_bytes)) = span {
                if !key_text || !filename_text {
                    return Err(InventoryUnavailable.into());
                }
                let key_bytes = usize::try_from(key_bytes).map_err(|_| InventoryUnavailable)?;
                let filename_bytes =
                    usize::try_from(filename_bytes).map_err(|_| InventoryUnavailable)?;
                let key_upper = self.text_encoding.utf8_upper(key_bytes)?;
                let filename_upper = self.text_encoding.utf8_upper(filename_bytes)?;
                budget.admit(
                    "key bytes",
                    key_bytes,
                    usize::try_from(
                        self.text_encoding
                            .field_encoded_limit(budget.limits.key_bytes)?,
                    )
                    .map_err(|_| InventoryUnavailable)?,
                )?;
                budget.admit(
                    "filename bytes",
                    filename_upper,
                    MAX_INVENTORY_FILENAME_UPPER,
                )?;
                let paths = InventoryBudget::mul(
                    InventoryBudget::add(self.root.as_os_str().as_encoded_bytes().len(), 1)?,
                    2,
                )?;
                budget.retain(InventoryBudget::add(
                    paths,
                    InventoryBudget::add(key_upper, InventoryBudget::mul(filename_upper, 6)?)?,
                )?)?;
                budget.work(InventoryBudget::add(
                    InventoryBudget::mul(filename_upper, 16)?,
                    128,
                )?)?;
                let encoded = InventoryBudget::add(key_bytes, filename_bytes)?;
                budget.scratch(self.text_encoding.temporary_bytes(
                    encoded,
                    InventoryBudget::add(key_upper, filename_upper)?,
                )?)?;
                Some(EntrySpan {
                    key_upper,
                    filename_upper,
                })
            } else {
                None
            };
            numeric.push((key, span));
        }
        ensure!(self.inventory_revision()? == revision, InventoryUnavailable);
        let spans = numeric.into_iter().collect();
        Ok(KeyInspectionPlan {
            cache_identity: self.identity,
            spans,
            metadata_bytes: budget.metadata_bytes(),
            transaction,
            revision,
        })
    }

    pub(crate) fn inspect_planned_keys(
        &self,
        plan: KeyInspectionPlan<'_>,
        scope: &mut VerificationScope,
    ) -> Result<CacheInspection> {
        self.inspect_planned_inventory_keys(plan, scope, None)
    }

    pub(crate) fn inspect_planned_removal_keys(
        &self,
        plan: KeyInspectionPlan<'_>,
        scope: &mut VerificationScope,
        inventory: &RemovalPlan,
    ) -> Result<CacheInspection> {
        self.inspect_planned_inventory_keys(plan, scope, Some(inventory))
    }

    fn inspect_planned_inventory_keys(
        &self,
        plan: KeyInspectionPlan<'_>,
        scope: &mut VerificationScope,
        inventory: Option<&RemovalPlan>,
    ) -> Result<CacheInspection> {
        ensure!(
            plan.cache_identity == self.identity,
            "cache inventory plan changed ownership"
        );
        scope.bind(self)?;
        self.validate_verification(scope)?;
        let captured = inventory.map(|inventory| {
            inventory
                .entries
                .iter()
                .map(|removal| (removal.entry.key.as_str(), removal))
                .collect::<BTreeMap<_, _>>()
        });
        if let Some(inventory) = inventory {
            ensure!(
                inventory.cache_identity == self.identity,
                "cache removal plan changed ownership"
            );
            ensure!(
                self.inventory_revision()? == inventory.revision,
                InventoryUnavailable
            );
            self.check_cleanup_plan(&inventory.pending, Some(&scope.destinations))?;
        }
        let mut inspected = BTreeMap::new();
        #[cfg(test)]
        self.observe_inventory(|stats| stats.inspections += 1);
        for (key, span) in plan.spans {
            let Some(entry) = self.bounded_entry(key, span, self.text_encoding)? else {
                inspected.insert(
                    key.to_owned(),
                    InspectedEntry {
                        entry: None,
                        stamp: None,
                        path: None,
                        invalid: false,
                    },
                );
                continue;
            };
            let path = self.object_path(&entry.filename)?;
            let stamp = object_stamp(&path)?;
            scope
                .destinations
                .borrow()
                .check_remembered(self, &entry.filename, &stamp)?;
            if let Some(captured) = &captured {
                let removal = captured.get(key).ok_or(InventoryUnavailable)?;
                ensure!(
                    removal.entry.filename == entry.filename
                        && removal.entry.size == entry.size
                        && removal.stamp == stamp,
                    InventoryUnavailable
                );
            }
            let valid_shape = stamp
                .as_ref()
                .is_some_and(|stamp| stamp.regular && !stamp.symlink && stamp.size == entry.size);
            inspected.insert(
                key.to_owned(),
                InspectedEntry {
                    entry: Some(entry),
                    stamp,
                    path: valid_shape.then_some(path),
                    invalid: !valid_shape,
                },
            );
        }
        // Capture every physical identity before the first streamed hash. Old
        // inventory evidence remains indexed separately from guarded new paths.
        for object in inspected.values_mut() {
            if let Some(path) = &object.path {
                let entry = object
                    .entry
                    .as_ref()
                    .expect("A staged path has an indexed entry");
                let valid = verify_object(path, &entry.filename, entry.size, scope)?;
                ensure!(
                    object_stamp(path)? == object.stamp,
                    "cached file changed during verification"
                );
                object.invalid = !valid;
                if !valid {
                    object.path = None;
                }
            }
        }
        plan.transaction.commit()?;
        ensure!(
            self.inventory_revision()? == plan.revision,
            InventoryUnavailable
        );
        Ok(CacheInspection {
            cache_identity: self.identity,
            entries: inspected,
            protection: Rc::clone(&scope.destinations),
            metadata_bytes: plan.metadata_bytes,
            prepaid: true,
        })
    }

    /// Admit the complete metadata and unique physical read set before the
    /// first payload hash. Legacy single-object callers keep inspect_keys.
    pub(crate) fn inspect_keys_prepaid(
        &self,
        keys: &[String],
        scope: &mut VerificationScope,
        maximum_metadata_bytes: usize,
    ) -> Result<CacheInspection> {
        scope.bind(self)?;
        self.validate_verification(scope)?;
        let slot = 2
            * (std::mem::size_of::<(String, InspectedEntry)>()
                + std::mem::size_of::<(String, VerifiedObject)>()
                + std::mem::size_of::<(String, PrepaidObject)>()
                + std::mem::size_of::<(String, Option<ObjectStamp>)>()
                + std::mem::size_of::<(&str, Option<EntrySpan>)>()
                + std::mem::size_of::<Entry>());
        let path_prefix_bytes = self
            .root
            .as_os_str()
            .as_encoded_bytes()
            .len()
            .checked_add(1)
            .and_then(|bytes| bytes.checked_mul(2))
            .ok_or(VerificationMetadataLimitExceeded)?;
        let mut metadata_bytes = std::mem::size_of::<CacheInspection>() + 4 * slot + 32 * 1024;
        let mut charge = |amount: usize| -> Result<()> {
            metadata_bytes = metadata_bytes
                .checked_add(amount)
                .filter(|bytes| *bytes <= maximum_metadata_bytes)
                .ok_or(VerificationMetadataLimitExceeded)?;
            Ok(())
        };
        // All input slots are priced before the borrowed uniqueness set.
        for key in keys {
            charge(
                key.len()
                    .checked_mul(2)
                    .and_then(|bytes| bytes.checked_add(slot))
                    .ok_or(VerificationMetadataLimitExceeded)?,
            )?;
        }
        charge(0)?;
        let encoding = self.text_encoding;
        let unique = keys.iter().map(String::as_str).collect::<BTreeSet<_>>();
        let mut spans = BTreeMap::new();
        for key in unique {
            let span = self
                .connection
                .query_row(
                    "SELECT typeof(key)='text',typeof(filename)='text',
                    COALESCE(octet_length(key),0),COALESCE(octet_length(filename),0)
                    FROM cache_entries WHERE key=?1",
                    [key],
                    |row| {
                        Ok((
                            row.get::<_, bool>(0)?,
                            row.get::<_, bool>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )
                .optional()?;
            let span = span
                .map(
                    |(key_text, filename_text, key_bytes, filename_bytes)| -> Result<EntrySpan> {
                        if !key_text || !filename_text {
                            return Err(VerificationMetadataLimitExceeded.into());
                        }
                        let key_bytes =
                            usize::try_from(key_bytes).context("invalid cached key length")?;
                        let filename_bytes = usize::try_from(filename_bytes)
                            .context("invalid cached filename length")?;
                        let key_upper = encoding.utf8_upper(key_bytes)?;
                        let filename_upper = encoding.utf8_upper(filename_bytes)?;
                        charge(
                            key_bytes
                                .checked_add(filename_bytes)
                                .and_then(|bytes| bytes.checked_add(key_upper))
                                .and_then(|bytes| bytes.checked_add(filename_upper.checked_mul(4)?))
                                .and_then(|bytes| bytes.checked_add(path_prefix_bytes))
                                .ok_or(VerificationMetadataLimitExceeded)?,
                        )?;
                        Ok(EntrySpan {
                            key_upper,
                            filename_upper,
                        })
                    },
                )
                .transpose()?;
            spans.insert(key, span);
        }
        drop(charge);
        let mut inspected = BTreeMap::new();
        for (key, span) in spans {
            let entry = self.bounded_entry(key, span, encoding)?;
            let (stamp, path, invalid) = if let Some(entry) = &entry {
                let path = self.object_path(&entry.filename)?;
                let stamp = object_stamp(&path)?;
                scope
                    .destinations
                    .borrow_mut()
                    .remember(self, &entry.filename, stamp.clone())?;
                let valid_shape = stamp.as_ref().is_some_and(|stamp| {
                    stamp.regular && !stamp.symlink && stamp.size == entry.size
                });
                if valid_shape {
                    let before = stamp.as_ref().unwrap();
                    let reads = if before.permits_reuse() { 1 } else { 0 };
                    match scope.prepaid.entry(entry.filename.clone()) {
                        std::collections::hash_map::Entry::Occupied(mut prepaid) => {
                            ensure!(
                                prepaid.get().stamp == *before
                                    && prepaid.get().expected_size == entry.size,
                                "cached object changed during bulk admission"
                            );
                            if reads == 0 {
                                let reads_left = prepaid
                                    .get()
                                    .reads_left
                                    .checked_add(1)
                                    .context("bulk verification read count overflow")?;
                                prepaid.get_mut().reads_left = reads_left;
                            }
                        }
                        std::collections::hash_map::Entry::Vacant(prepaid) => {
                            prepaid.insert(PrepaidObject {
                                stamp: before.clone(),
                                expected_size: entry.size,
                                reads_left: 1,
                            });
                        }
                    }
                }
                (stamp, valid_shape.then_some(path), !valid_shape)
            } else {
                (None, None, false)
            };
            inspected.insert(
                key.to_owned(),
                InspectedEntry {
                    entry,
                    stamp,
                    path,
                    invalid,
                },
            );
        }
        let requested = scope.prepaid.values().try_fold(0u64, |bytes, prepaid| {
            prepaid
                .expected_size
                .checked_add(1)
                .and_then(|size| size.checked_mul(prepaid.reads_left as u64))
                .and_then(|size| bytes.checked_add(size))
                .ok_or_else(|| anyhow::anyhow!("bulk verification byte forecast overflow"))
        })?;
        scope.reserve(requested)?;
        for (key, inspected) in &mut inspected {
            ensure!(
                self.entry_matches(key, inspected.entry.as_ref())?,
                "cached object changed during verification"
            );
            if let (Some(entry), Some(path)) = (&inspected.entry, &inspected.path) {
                let valid = verify_object(path, &entry.filename, entry.size, scope)?;
                ensure!(
                    object_stamp(path)? == inspected.stamp,
                    "cached file changed during verification"
                );
                inspected.invalid = !valid;
                if !valid {
                    inspected.path = None;
                }
            }
        }
        scope.prepaid.clear();
        Ok(CacheInspection {
            cache_identity: self.identity,
            entries: inspected,
            protection: Rc::clone(&scope.destinations),
            metadata_bytes,
            prepaid: true,
        })
    }

    fn bounded_entry(
        &self,
        key: &str,
        span: Option<EntrySpan>,
        encoding: CacheTextEncoding,
    ) -> Result<Option<Entry>> {
        let Some(span) = span else {
            ensure!(
                self.entry_matches(key, None)?,
                "cached object appeared during bulk admission"
            );
            return Ok(None);
        };
        let mut statement = self.connection.prepare(
            "SELECT CASE WHEN typeof(key)='text' AND COALESCE(octet_length(key),0)<=?2 THEN key ELSE NULL END,
             CASE WHEN typeof(filename)='text' AND COALESCE(octet_length(filename),0)<=?3 THEN filename ELSE NULL END,
             size,pinned FROM cache_entries WHERE key=?1",
        )?;
        let mut rows = statement.query(params![
            key,
            encoding.maximum_encoded(span.key_upper)?,
            encoding.maximum_encoded(span.filename_upper)?
        ])?;
        let Some(row) = rows.next()? else {
            return Err(VerificationMetadataLimitExceeded.into());
        };
        // Rusqlite receives borrowed UTF-8 after SQLite's encoded gate. Admit
        // that actual conversion and both fields before owning either String.
        let key_value = row.get_ref(0)?;
        let key_text = key_value
            .as_str()
            .map_err(|_| VerificationMetadataLimitExceeded)?;
        let filename_value = row.get_ref(1)?;
        let filename = filename_value
            .as_str()
            .map_err(|_| VerificationMetadataLimitExceeded)?;
        if key_text != key
            || key_text.len() > span.key_upper
            || filename.len() > span.filename_upper
        {
            return Err(VerificationMetadataLimitExceeded.into());
        }
        let size: i64 = row.get(2).map_err(|_| InventoryUnavailable)?;
        let pinned: bool = row.get(3).map_err(|_| InventoryUnavailable)?;
        if size < 0 {
            return Err(InventoryUnavailable.into());
        }
        #[cfg(test)]
        self.observe_inventory(|stats| {
            stats.entry_copies += 1;
            stats.key_copies += 1;
            stats.filename_copies += 1;
        });
        Ok(Some(Entry {
            key: key_text.to_owned(),
            filename: filename.to_owned(),
            size: size as u64,
            pinned,
        }))
    }

    fn entry_matches(&self, key: &str, expected: Option<&Entry>) -> Result<bool> {
        match expected {
            Some(expected) => {
                // A valid digest filename is ASCII. Twice its UTF-8 length
                // covers either database encoding before any text comparison.
                let encoded_limit = expected
                    .filename
                    .len()
                    .checked_mul(2)
                    .and_then(|bytes| i64::try_from(bytes).ok())
                    .ok_or(VerificationMetadataLimitExceeded)?;
                Ok(self.connection.query_row(
                    "SELECT CASE WHEN typeof(filename)='text' AND COALESCE(octet_length(filename),0)<=?4
                        THEN filename IS ?2 AND size=?3 ELSE 0 END FROM cache_entries WHERE key=?1",
                    params![key, expected.filename, expected.size as i64, encoded_limit], |row| row.get(0))
                    .optional()?.unwrap_or(false))
            }
            None => Ok(self.connection.query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM cache_entries WHERE key=?1)",
                [key],
                |row| row.get(0),
            )?),
        }
    }

    /// Read the pin state for one logical object without scanning other entries.
    pub fn pinned_state(&self, key: &str) -> Result<Option<bool>> {
        Ok(self
            .connection
            .query_row(
                "SELECT pinned FROM cache_entries WHERE key=?1",
                [key],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn put(&self, key: &str, bytes: &[u8]) -> Result<PathBuf> {
        self.put_with_extension(key, bytes, "bin", None)
    }

    /// Store a PNG that has already passed the caller's image-decoding policy.
    pub fn put_png(&self, key: &str, bytes: &[u8]) -> Result<PathBuf> {
        ensure!(
            bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "cached PNG has an invalid signature"
        );
        self.put_with_extension(key, bytes, "png", None)
    }

    /// Prepay every physical destination that put may otherwise read even when
    /// the logical index is missing. Call before recovery, pins or publication.
    pub(crate) fn preverify_png_destinations(
        &self,
        candidates: &[(&str, &[u8])],
        scope: &mut VerificationScope,
    ) -> Result<()> {
        scope.bind(self)?;
        self.validate_verification(scope)?;
        let mut seen = HashSet::new();
        for (key, bytes) in candidates {
            ensure!(
                bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
                "cached PNG has an invalid signature"
            );
            ensure!(
                !key.is_empty() && !key.contains('\0') && bytes.len() as u64 <= self.maximum_bytes,
                "invalid PNG destination candidate"
            );
            let filename = format!("{}-{}.png", digest(key.as_bytes()), digest(bytes));
            if !seen.insert(filename.clone()) {
                continue;
            }
            let path = self.object_path(&filename)?;
            let stamp = object_stamp(&path)?;
            scope
                .destinations
                .borrow_mut()
                .remember(self, &filename, stamp.clone())?;
            if stamp.as_ref().is_some_and(|stamp| {
                stamp.regular && !stamp.symlink && stamp.size == bytes.len() as u64
            }) {
                // A false digest is retained as corruption evidence. It is not
                // retired before the entire operation's read admission passes.
                verify_object(&path, &filename, bytes.len() as u64, scope)?;
            }
            ensure!(
                object_stamp(&path)? == stamp,
                "cache destination changed during operation preflight"
            );
        }
        Ok(())
    }

    pub(crate) fn put_png_preverified(
        &self,
        key: &str,
        bytes: &[u8],
        scope: &mut VerificationScope,
    ) -> Result<PathBuf> {
        ensure!(
            bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "cached PNG has an invalid signature"
        );
        scope.bind(self)?;
        self.put_with_extension(key, bytes, "png", Some(scope))
    }

    /// Decode within explicit limits, resize, and quantize to 16 grayscale values.
    pub fn put_cover(&self, key: &str, bytes: &[u8], width: u32, height: u32) -> Result<PathBuf> {
        ensure!(
            width > 0 && height > 0 && width <= 2048 && height <= 2048,
            "invalid thumbnail geometry"
        );
        ensure!(
            bytes.len() <= 16 * 1024 * 1024,
            "cover download exceeds size limit"
        );
        let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        let mut grayscale = reader
            .decode()
            .context("decode upstream cover")?
            .thumbnail(width, height)
            .to_luma8();
        for pixel in grayscale.pixels_mut() {
            pixel[0] = (((pixel[0] as u16 + 8) / 17).min(15) * 17) as u8;
        }
        let mut output = Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(grayscale)
            .write_to(&mut output, image::ImageFormat::Png)?;
        self.put_with_extension(key, output.get_ref(), "png", None)
    }

    /// Keep canonical eight-bit luminance for later display-specific processing.
    /// Cover thumbnails deliberately retain their independent sixteen-level path.
    pub fn put_gray8(&self, key: &str, bytes: &[u8], width: u32, height: u32) -> Result<PathBuf> {
        ensure!(
            width > 0 && height > 0 && width <= 2048 && height <= 2048,
            "invalid canonical image geometry"
        );
        ensure!(
            bytes.len() <= 16 * 1024 * 1024,
            "canonical image download exceeds size limit"
        );
        let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        let mut rgba = reader
            .decode()
            .context("decode canonical image")?
            .thumbnail(width, height)
            .into_rgba8();
        for pixel in rgba.pixels_mut() {
            let alpha = u32::from(pixel[3]);
            for channel in &mut pixel.0[..3] {
                *channel = ((u32::from(*channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
            }
            pixel[3] = 255;
        }
        let grayscale = image::DynamicImage::ImageRgba8(rgba).into_luma8();
        let mut output = Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(grayscale)
            .write_to(&mut output, image::ImageFormat::Png)?;
        self.put_with_extension(key, output.get_ref(), "png", None)
    }

    pub fn set_pinned(&self, key: &str, pinned: bool) -> Result<bool> {
        let Some(current) = self.pinned_state(key)? else {
            return Ok(false);
        };
        if current != pinned {
            self.connection.execute(
                "UPDATE cache_entries SET pinned=?2 WHERE key=?1",
                params![key, pinned],
            )?;
        }
        Ok(true)
    }

    pub(crate) fn set_pinned_with_verification(
        &self,
        key: &str,
        pinned: bool,
        scope: &VerificationScope,
    ) -> Result<bool> {
        self.validate_verification(scope)?;
        self.set_pinned(key, pinned)
    }

    pub fn remove(&self, key: &str) -> Result<bool> {
        self.remove_using_protection(key, None)
    }

    pub(crate) fn remove_with_verification(
        &self,
        key: &str,
        scope: &VerificationScope,
    ) -> Result<bool> {
        self.remove_using_protection(key, Some(&scope.destinations))
    }

    fn remove_using_protection(
        &self,
        key: &str,
        protection: Option<&Rc<RefCell<DestinationProtection>>>,
    ) -> Result<bool> {
        let mut budget = self.inventory_budget();
        let revision = self.inventory_revision()?;
        let entry = self.entry_with_budget(key, &mut budget)?;
        let names = self.pending_cleanup_names(&mut budget)?;
        if let Some(entry) = &entry {
            ensure!(!entry.pinned, "active cache object is pinned");
        }
        let stamp = entry
            .as_ref()
            .map(|entry| object_stamp(&self.object_path(&entry.filename)?))
            .transpose()?;
        let names = self.capture_cleanup_names(names, &mut budget, protection)?;
        ensure!(self.inventory_revision()? == revision, InventoryUnavailable);
        self.check_cleanup_plan(&names, protection)?;
        for cleanup in names {
            self.cleanup_captured_pending_object(&cleanup.filename, &cleanup.stamp, protection)?;
        }
        let Some(entry) = entry else {
            return Ok(false);
        };
        ensure!(
            self.entry_matches(key, Some(&entry))?,
            "cache row changed after removal admission"
        );
        ensure!(
            self.pinned_state(key)? == Some(false),
            "active cache object is pinned"
        );
        let stamp = stamp.expect("An admitted entry has a captured physical state");
        self.check_captured_object(&entry.filename, &stamp, protection)?;
        self.retire_entry_using_protection(&entry, protection)?;
        self.cleanup_captured_pending_object(&entry.filename, &stamp, protection)?;
        Ok(true)
    }

    pub fn clear_unpinned(&self) -> Result<u64> {
        let mut budget = self.inventory_budget();
        let revision = self.inventory_revision()?;
        let (entries, names) = self.paired_inventory(&mut budget, InventoryUse::Entries, true)?;
        let _total = entries.iter().try_fold(0_u64, |total, entry| {
            total
                .checked_add(entry.size)
                .context("cleared cache size overflow")
        })?;
        for entry in &entries {
            ensure!(
                self.entry_matches(&entry.key, Some(entry))?,
                "cache row changed after clear admission"
            );
            ensure!(
                self.pinned_state(&entry.key)? == Some(false),
                "active cache object is pinned"
            );
        }
        let entries = self.capture_removals(entries)?;
        let names = self.capture_cleanup_names(names, &mut budget, None)?;
        ensure!(self.inventory_revision()? == revision, InventoryUnavailable);
        self.check_cleanup_plan(&names, None)?;
        for cleanup in names {
            self.cleanup_captured_pending_object(&cleanup.filename, &cleanup.stamp, None)?;
        }
        let mut removed = 0_u64;
        for removal in entries {
            let entry = removal.entry;
            ensure!(
                self.inventory_revision()?.0 == revision.0,
                InventoryUnavailable
            );
            ensure!(
                self.entry_matches(&entry.key, Some(&entry))?,
                "cache row changed during clear"
            );
            ensure!(
                self.pinned_state(&entry.key)? == Some(false),
                "active cache object is pinned"
            );
            self.check_captured_object(&entry.filename, &removal.stamp, None)?;
            self.retire_entry_using_protection(&entry, None)?;
            self.cleanup_captured_pending_object(&entry.filename, &removal.stamp, None)?;
            removed = removed
                .checked_add(entry.size)
                .context("cleared cache size overflow")?;
        }
        Ok(removed)
    }

    fn put_with_extension(
        &self,
        key: &str,
        bytes: &[u8],
        extension: &str,
        mut verification: Option<&mut VerificationScope>,
    ) -> Result<PathBuf> {
        ensure!(!key.is_empty() && !key.contains('\0'), "invalid cache key");
        let size = bytes.len() as u64;
        ensure!(
            size <= self.maximum_bytes,
            "cache object exceeds the storage budget"
        );
        let mut inventory = self.inventory_budget();
        inventory.admit("key bytes", key.len(), inventory.limits.key_bytes)?;
        inventory.retain(4 * (MAX_INVENTORY_FILENAME_UPPER + std::mem::size_of::<String>()))?;
        inventory.work(InventoryBudget::add(
            InventoryBudget::mul(key.len(), 2)?,
            1024,
        )?)?;
        let old = self.entry_with_budget(key, &mut inventory)?;
        let old_size = old.as_ref().map_or(0, |entry| entry.size);
        // Successful debt cleanup leaves exactly the indexed payload usage.
        // Decide whether an eviction collection is actually needed, and admit
        // it together with debt before any unlink or publication can start.
        let indexed_usage = u64::try_from(self.connection.query_row(
            "SELECT COALESCE(SUM(size),0) FROM cache_entries",
            [],
            |row| row.get::<_, i64>(0),
        )?)
        .context("invalid indexed cache usage")?;
        let indexed_required = indexed_usage.saturating_sub(old_size).saturating_add(size);
        let (eviction_entries, pending_names) = if indexed_required > self.maximum_bytes {
            self.paired_inventory(&mut inventory, InventoryUse::Entries, true)?
        } else {
            (Vec::new(), self.pending_cleanup_names(&mut inventory)?)
        };
        for entry in &eviction_entries {
            ensure!(
                self.entry_matches(&entry.key, Some(entry))?,
                "cache row changed after publication admission"
            );
        }
        let filename = format!("{}-{}.{}", digest(key.as_bytes()), digest(bytes), extension);
        let destination = self.object_path(&filename)?;
        let protection = verification
            .as_deref()
            .map(|scope| Rc::clone(&scope.destinations));
        let pending_names =
            self.capture_cleanup_names(pending_names, &mut inventory, protection.as_ref())?;
        let mut victims = Vec::new();
        let mut remaining = indexed_required;
        for entry in eviction_entries {
            if remaining <= self.maximum_bytes {
                break;
            }
            if entry.key == key || entry.pinned {
                continue;
            }
            let stamp = object_stamp(&self.object_path(&entry.filename)?)?;
            if let Some(protection) = &protection {
                protection
                    .borrow()
                    .check_remembered(self, &entry.filename, &stamp)?;
            }
            remaining = remaining.saturating_sub(entry.size);
            victims.push(PlannedRemoval { entry, stamp });
        }
        ensure!(
            remaining <= self.maximum_bytes,
            "pinned cache objects leave insufficient storage"
        );
        let old_stamp = old
            .as_ref()
            .filter(|entry| entry.filename != filename)
            .map(|entry| object_stamp(&self.object_path(&entry.filename)?))
            .transpose()?;
        self.check_cleanup_plan(&pending_names, protection.as_ref())?;
        if let Some(scope) = verification.as_deref() {
            ensure!(
                scope.destinations.borrow().expected.contains_key(&filename),
                "cache destination was not included in operation preflight"
            );
            // Validate every candidate before old debt can erase a late object
            // and make its path appear absent again.
            self.validate_verification(scope)?;
            if let Some(current) = object_stamp(&destination)? {
                if current.regular && !current.symlink && current.size == size {
                    // Even a debt-backed destination needs its prepaid proof
                    // before cleanup can remove it and permit an owned write.
                    preverified_digest(&destination, &filename, size, scope)?;
                }
            }
        }
        // A persistent unlink failure must prevent further payload publication.
        // Scoped retries consume the original metadata fence before any drain.
        for cleanup in &pending_names {
            self.cleanup_captured_pending_object(
                &cleanup.filename,
                &cleanup.stamp,
                protection.as_ref(),
            )?;
        }
        if let Some(scope) = verification.as_deref_mut() {
            if let Some(current) = object_stamp(&destination)? {
                self.validate_verification(scope)?;
                if current.regular && !current.symlink && current.size == size {
                    // Check the read-free proof before creating a cleanup guard:
                    // an unadmitted valid destination is not this write's orphan.
                    preverified_digest(&destination, &filename, size, scope)?;
                }
            }
        }
        let required = self.usage()?.saturating_sub(old_size).saturating_add(size);
        ensure!(
            required <= indexed_required,
            "cache inventory changed after publication admission"
        );

        let mut pending =
            PendingObject::new_using_protection(self, &filename, size, protection.as_ref())?;
        let publish_result = (|| -> Result<()> {
            if let Some(protection) = &protection {
                protection.borrow().validate(self)?;
            }
            match fs::symlink_metadata(&destination) {
                Ok(metadata) => {
                    ensure!(
                        metadata.is_file() && !metadata.file_type().is_symlink(),
                        "cache destination is not a regular file"
                    );
                    ensure!(
                        metadata.len() == size,
                        "content-addressed cache object has invalid size"
                    );
                    let valid = match verification.as_deref_mut() {
                        Some(scope) => preverified_digest(&destination, &filename, size, scope)?,
                        None => verify_digest(&destination, &filename, size)?,
                    };
                    ensure!(valid, "content-addressed cache object is corrupt");
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    // Track both sides of the atomic rename. Interrupted writes
                    // and failed unlinks remain visible to the storage budget.
                    let (temporary_name, temporary_path) = loop {
                        let name = format!(
                            ".tmp-{}-{}",
                            std::process::id(),
                            NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
                        );
                        let path = self.cleanup_object_path(&name)?;
                        match fs::symlink_metadata(&path) {
                            Ok(_) => continue,
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                break (name, path);
                            }
                            Err(error) => {
                                return Err(error).context("inspect cache temporary path");
                            }
                        }
                    };
                    let mut temporary = PendingObject::new_using_protection(
                        self,
                        &temporary_name,
                        size,
                        protection.as_ref(),
                    )?;
                    let mut output = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&temporary_path)?;
                    let written = (|| -> Result<()> {
                        temporary.record_created(&output)?;
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            output.set_permissions(fs::Permissions::from_mode(0o600))?;
                        }
                        output.write_all(bytes)?;
                        output.sync_all()?;
                        temporary.refresh_created(&output)?;
                        if let Some(protection) = &protection {
                            protection.borrow().validate(self)?;
                        }
                        fs::rename(&temporary_path, &destination).context("commit cache object")?;
                        let stamp = temporary.record_rename(&filename, &output)?;
                        ensure!(stamp.size == size, "owned cache payload changed size");
                        if let Some(scope) = verification.as_deref_mut() {
                            scope.evidence.insert(
                                filename.clone(),
                                VerifiedObject {
                                    stamp,
                                    expected_size: size,
                                    digest_matches: true,
                                },
                            );
                        }
                        Ok(())
                    })();
                    if let Err(error) = written {
                        // Account for this descriptor's partial writes before
                        // cleanup. A replaced path cannot be refreshed from it.
                        if let Err(refresh) = temporary.refresh_created(&output) {
                            return Err(error).context(format!(
                                "cache writer failed; identity recovery remains pending: {refresh:#}"
                            ));
                        }
                        drop(output);
                        if let Err(cleanup) = temporary.cleanup() {
                            return Err(error).context(format!(
                                "cache writer failed; temporary cleanup remains pending: {cleanup:#}"
                            ));
                        }
                        return Err(error);
                    }
                    drop(output);
                    temporary.cleanup()?;
                }
                Err(error) => return Err(error).context("inspect cache destination"),
            }

            if let Some(protection) = &protection {
                protection.borrow().validate(self)?;
            }
            for victim in &victims {
                ensure!(
                    self.entry_matches(&victim.entry.key, Some(&victim.entry))?,
                    "cache eviction evidence changed"
                );
                ensure!(
                    self.pinned_state(&victim.entry.key)? == Some(false),
                    "active cache object is pinned"
                );
                self.check_captured_object(
                    &victim.entry.filename,
                    &victim.stamp,
                    protection.as_ref(),
                )?;
            }
            if let (Some(old), Some(stamp)) = (&old, &old_stamp) {
                self.check_captured_object(&old.filename, stamp, protection.as_ref())?;
            }
            let transaction = self.connection.unchecked_transaction()?;
            ensure!(
                self.entry_matches(key, old.as_ref())?,
                "cache publication evidence changed"
            );
            transaction.execute("INSERT INTO cache_entries (key,filename,size,last_access,pinned) VALUES (?1,?2,?3,(SELECT COALESCE(MAX(last_access),0)+1 FROM cache_entries),?4) ON CONFLICT(key) DO UPDATE SET filename=excluded.filename,size=excluded.size,last_access=excluded.last_access", params![key,filename,size as i64,old.as_ref().is_some_and(|entry| entry.pinned)])?;
            for victim in &victims {
                ensure!(
                    self.entry_matches(&victim.entry.key, Some(&victim.entry))?,
                    "cache eviction evidence changed"
                );
                schedule_cleanup_on(&transaction, &victim.entry.filename, victim.entry.size)?;
                let removed = transaction.execute(
                    "DELETE FROM cache_entries WHERE key=?1 AND filename=?2 AND size=?3",
                    params![
                        victim.entry.key,
                        victim.entry.filename,
                        victim.entry.size as i64
                    ],
                )?;
                ensure!(removed == 1, "cache eviction evidence changed");
            }
            if let Some(old) = old.as_ref().filter(|entry| entry.filename != filename) {
                schedule_cleanup_on(&transaction, &old.filename, old.size)?;
            }
            transaction.execute(
                "DELETE FROM cache_pending_cleanup WHERE filename=?1",
                [&filename],
            )?;
            transaction.commit()?;
            Ok(())
        })();
        if let Err(error) = publish_result {
            if let Err(cleanup_error) = pending.cleanup() {
                return Err(error).context(format!(
                    "cache publication failed; payload cleanup remains pending: {cleanup_error:#}"
                ));
            }
            return Err(error).context("publish cache object");
        }
        pending.disarm();
        for victim in &victims {
            self.cleanup_captured_pending_object(
                &victim.entry.filename,
                &victim.stamp,
                protection.as_ref(),
            )
            .context("cache object committed but retired payload cleanup remains pending")?;
        }
        if let (Some(old), Some(stamp)) = (&old, &old_stamp) {
            self.cleanup_captured_pending_object(&old.filename, stamp, protection.as_ref())
                .context("cache object committed but retired payload cleanup remains pending")?;
        }
        Ok(destination)
    }

    fn entry(&self, key: &str) -> Result<Option<Entry>> {
        self.entry_with_budget(key, &mut self.inventory_budget())
    }

    fn entry_with_budget(&self, key: &str, budget: &mut InventoryBudget) -> Result<Option<Entry>> {
        budget.admit("key bytes", key.len(), budget.limits.key_bytes)?;
        let transaction = self.connection.unchecked_transaction()?;
        let header: Option<(bool, bool, i64, i64)> = transaction
            .query_row(
                "SELECT typeof(key)='text',typeof(filename)='text',COALESCE(octet_length(key),0),
             COALESCE(octet_length(filename),0) FROM cache_entries WHERE key=?1",
                [key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((key_text, filename_text, key_encoded, filename_encoded)) = header else {
            transaction.commit()?;
            return Ok(None);
        };
        if !key_text || !filename_text {
            return Err(InventoryUnavailable.into());
        }
        let key_encoded = usize::try_from(key_encoded).map_err(|_| InventoryUnavailable)?;
        let filename_encoded =
            usize::try_from(filename_encoded).map_err(|_| InventoryUnavailable)?;
        let key_upper = self.text_encoding.utf8_upper(key_encoded)?;
        let filename_upper = self.text_encoding.utf8_upper(filename_encoded)?;
        budget.admit(
            "key bytes",
            key_encoded,
            usize::try_from(
                self.text_encoding
                    .field_encoded_limit(budget.limits.key_bytes)?,
            )
            .map_err(|_| InventoryUnavailable)?,
        )?;
        budget.admit(
            "filename bytes",
            filename_upper,
            MAX_INVENTORY_FILENAME_UPPER,
        )?;
        budget.retain(InventoryBudget::add(
            2 * std::mem::size_of::<PlannedRemoval>(),
            InventoryBudget::add(key_upper, filename_upper)?,
        )?)?;
        budget.work(InventoryBudget::mul(
            InventoryBudget::add(key_upper, filename_upper)?,
            4,
        )?)?;
        budget.scratch(self.text_encoding.temporary_bytes(
            InventoryBudget::add(key_encoded, filename_encoded)?,
            InventoryBudget::add(key_upper, filename_upper)?,
        )?)?;
        let entry = self.bounded_entry(
            key,
            Some(EntrySpan {
                key_upper,
                filename_upper,
            }),
            self.text_encoding,
        )?;
        transaction.commit()?;
        Ok(entry)
    }

    fn entries(&self) -> Result<Vec<Entry>> {
        self.inventory_entries(
            &mut self.inventory_budget(),
            InventoryUse::Entries,
            false,
            false,
        )
    }

    fn object_path(&self, filename: &str) -> Result<PathBuf> {
        ensure!(valid_filename(filename), "invalid cache object filename");
        Ok(self.root.join(filename))
    }

    fn cleanup_object_path(&self, filename: &str) -> Result<PathBuf> {
        ensure!(
            valid_cleanup_filename(filename),
            "invalid pending cache object filename"
        );
        Ok(self.root.join(filename))
    }

    fn schedule_cleanup(&self, filename: &str, size: u64) -> Result<()> {
        schedule_cleanup_on(&self.connection, filename, size)
    }

    fn retire_entry_using_protection(
        &self,
        entry: &Entry,
        protection: Option<&Rc<RefCell<DestinationProtection>>>,
    ) -> Result<()> {
        if let Some(protection) = protection {
            protection.borrow().validate(self)?;
        }
        let transaction = self.connection.unchecked_transaction()?;
        ensure!(
            self.entry_matches(&entry.key, Some(entry))?,
            "cache retirement evidence changed"
        );
        schedule_cleanup_on(&transaction, &entry.filename, entry.size)?;
        let removed = transaction.execute(
            "DELETE FROM cache_entries WHERE key=?1 AND filename=?2 AND size=?3",
            params![entry.key, entry.filename, entry.size as i64],
        )?;
        ensure!(removed == 1, "cache retirement evidence changed");
        transaction.commit()?;
        Ok(())
    }

    fn cleanup_pending(&self) -> Result<()> {
        self.cleanup_pending_using_protection(None)
    }

    fn cleanup_pending_using_protection(
        &self,
        protection: Option<&Rc<RefCell<DestinationProtection>>>,
    ) -> Result<()> {
        if let Some(protection) = protection {
            protection.borrow().validate(self)?;
        }
        let mut budget = self.inventory_budget();
        let filenames = self.pending_cleanup_names(&mut budget)?;
        let pending = self.capture_cleanup_names(filenames, &mut budget, protection)?;
        self.check_cleanup_plan(&pending, protection)?;
        for cleanup in pending {
            self.cleanup_captured_pending_object(&cleanup.filename, &cleanup.stamp, protection)?;
        }
        Ok(())
    }

    fn cleanup_pending_object_using_protection(
        &self,
        filename: &str,
        protection: Option<&Rc<RefCell<DestinationProtection>>>,
    ) -> Result<()> {
        if let Some(protection) = protection {
            protection.borrow().validate(self)?;
        }
        let pending: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM cache_pending_cleanup WHERE filename=?1)",
            [filename],
            |row| row.get(0),
        )?;
        if !pending {
            return Ok(());
        }
        let referenced: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM cache_entries WHERE filename=?1)",
            [filename],
            |row| row.get(0),
        )?;
        if !referenced {
            if let Some(protection) = protection {
                protection.borrow().validate(self)?;
            }
            self.remove_object(filename)?;
            if let Some(protection) = protection {
                protection.borrow_mut().removed_by_owner(filename);
            }
        }
        // A possibly committed payload is protected by its durable index even
        // when a previous COMMIT returned an ambiguous I/O error.
        if let Some(protection) = protection {
            protection.borrow().validate(self)?;
        }
        self.connection.execute(
            "DELETE FROM cache_pending_cleanup WHERE filename=?1",
            [filename],
        )?;
        if let Some(protection) = protection {
            protection.borrow_mut().release_temporary(filename);
        }
        Ok(())
    }

    fn check_captured_object(
        &self,
        filename: &str,
        expected: &Option<ObjectStamp>,
        protection: Option<&Rc<RefCell<DestinationProtection>>>,
    ) -> Result<()> {
        if let Some(protection) = protection {
            let protection = protection.borrow();
            protection.validate(self)?;
            protection.check_remembered(self, filename, expected)?;
        }
        ensure!(
            object_stamp(&self.cleanup_object_path(filename)?)? == *expected,
            "captured cache object appeared or changed after inventory admission"
        );
        Ok(())
    }

    /// Captured legacy filenames are checked individually. They do not enlarge
    /// the shared new-destination fence, whose guards must retain their identity.
    fn cleanup_captured_pending_object(
        &self,
        filename: &str,
        expected: &Option<ObjectStamp>,
        protection: Option<&Rc<RefCell<DestinationProtection>>>,
    ) -> Result<()> {
        if let Some(protection) = protection {
            protection.borrow().validate(self)?;
        }
        let pending = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM cache_pending_cleanup WHERE filename=?1)",
            [filename],
            |row| row.get::<_, bool>(0),
        )?;
        if !pending {
            return Ok(());
        }
        let referenced = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM cache_entries WHERE filename=?1)",
            [filename],
            |row| row.get::<_, bool>(0),
        )?;
        if !referenced {
            self.check_captured_object(filename, expected, protection)?;
            self.remove_object(filename)?;
            if let Some(protection) = protection {
                protection.borrow_mut().removed_by_owner(filename);
            }
        }
        if let Some(protection) = protection {
            protection.borrow().validate(self)?;
        }
        self.connection.execute(
            "DELETE FROM cache_pending_cleanup WHERE filename=?1",
            [filename],
        )?;
        if let Some(protection) = protection {
            protection.borrow_mut().release_temporary(filename);
        }
        Ok(())
    }

    fn remove_object(&self, filename: &str) -> Result<()> {
        let path = self.cleanup_object_path(filename)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("remove stale cache object"),
        }
    }

    fn evict_to_budget(&self) -> Result<()> {
        let mut usage = self.usage()?;
        if usage <= self.maximum_bytes {
            return Ok(());
        }
        let mut budget = self.inventory_budget();
        let revision = self.inventory_revision()?;
        let (entries, names) = self.paired_inventory(&mut budget, InventoryUse::Entries, true)?;
        for entry in &entries {
            ensure!(
                self.entry_matches(&entry.key, Some(entry))?,
                "cache row changed after eviction admission"
            );
            ensure!(
                self.pinned_state(&entry.key)? == Some(false),
                "active cache object is pinned"
            );
        }
        let entries = self.capture_removals(entries)?;
        let names = self.capture_cleanup_names(names, &mut budget, None)?;
        ensure!(self.inventory_revision()? == revision, InventoryUnavailable);
        self.check_cleanup_plan(&names, None)?;
        for cleanup in names {
            self.cleanup_captured_pending_object(&cleanup.filename, &cleanup.stamp, None)?;
        }
        usage = self.usage()?;
        for removal in entries {
            if usage <= self.maximum_bytes {
                break;
            }
            let entry = removal.entry;
            ensure!(
                self.inventory_revision()?.0 == revision.0,
                InventoryUnavailable
            );
            ensure!(
                self.entry_matches(&entry.key, Some(&entry))?,
                "cache eviction evidence changed"
            );
            ensure!(
                self.pinned_state(&entry.key)? == Some(false),
                "active cache object is pinned"
            );
            self.check_captured_object(&entry.filename, &removal.stamp, None)?;
            self.retire_entry_using_protection(&entry, None)?;
            self.cleanup_captured_pending_object(&entry.filename, &removal.stamp, None)?;
            usage = usage.saturating_sub(entry.size);
        }
        ensure!(
            usage <= self.maximum_bytes,
            "pinned cache objects exceed the configured budget"
        );
        Ok(())
    }

    fn recover(&self) -> Result<()> {
        self.recover_with_verification_budget(self.verification_work_limit())
    }

    pub(crate) fn recover_with_verification_budget(&self, maximum_work_bytes: u64) -> Result<()> {
        let mut budget = self.inventory_budget();
        let revision = self.inventory_revision()?;
        let (entries, pending_names) =
            self.paired_inventory(&mut budget, InventoryUse::Entries, false)?;
        let pending = self.capture_cleanup_names(pending_names, &mut budget, None)?;
        let plan = self.plan_borrowed_key_inspection(
            entries.iter().map(|entry| entry.key.as_str()),
            &mut budget,
        )?;
        let mut scope = VerificationScope::new(maximum_work_bytes);
        let inspected = self.inspect_planned_keys(plan, &mut scope)?;
        budget.retain(InventoryBudget::add(
            128,
            InventoryBudget::mul(entries.len(), 4 * (std::mem::size_of::<&str>() + 2))?,
        )?)?;
        let known = inspected
            .entries
            .values()
            .filter_map(|entry| entry.entry.as_ref())
            .map(|entry| entry.filename.as_str())
            .collect::<HashSet<_>>();
        let mut candidates = Vec::new();
        for item in fs::read_dir(&self.root)? {
            let item = item?;
            let name = item.file_name();
            budget.work(InventoryBudget::add(
                InventoryBudget::mul(name.as_encoded_bytes().len(), 2)?,
                32,
            )?)?;
            let Some(filename) = name.to_str() else {
                continue;
            };
            // Opening the single-process cache has no live payload writer.
            // Account for legacy temporary objects even when they are recent
            // and predate the durable cleanup ledger. Unknown names stay put.
            if valid_cleanup_filename(filename) && !known.contains(filename) {
                let count = InventoryBudget::add(candidates.len(), 1)?;
                budget.rows(InventoryBudget::add(pending.len(), count)?)?;
                // Pay before owning a name, growing either collection, or
                // remembering its physical authorization. Partial candidates
                // remain local when a later enumeration allowance fails.
                budget.retain(InventoryBudget::add(
                    InventoryBudget::mul(filename.len(), 3)?,
                    2 * std::mem::size_of::<DirectoryCandidate>()
                        + 2 * std::mem::size_of::<(String, Option<ObjectStamp>)>(),
                )?)?;
                if candidates.is_empty() {
                    budget.retain(4 * std::mem::size_of::<DirectoryCandidate>())?;
                }
                let path_bytes = InventoryBudget::add(
                    self.root.as_os_str().as_encoded_bytes().len(),
                    filename.len() + 1,
                )?;
                budget.work(InventoryBudget::add(
                    InventoryBudget::mul(path_bytes, 4)?,
                    InventoryBudget::add(InventoryBudget::mul(filename.len(), 4)?, 256)?,
                )?)?;
                budget.scratch(InventoryBudget::mul(path_bytes, 2)?)?;
                let stamp = object_stamp(&item.path())?
                    .context("legacy cache object disappeared during recovery")?;
                #[cfg(test)]
                self.observe_inventory(|stats| stats.directory_copies += 1);
                candidates.push(DirectoryCandidate {
                    filename: filename.to_owned(),
                    stamp,
                });
            }
        }
        // Admission, hashing and every legacy filename fence precede the first
        // index retirement, debt mutation or unlink. No quota is corruption.
        ensure!(self.inventory_revision()? == revision, InventoryUnavailable);
        for candidate in &candidates {
            self.check_captured_object(
                &candidate.filename,
                &Some(candidate.stamp.clone()),
                Some(&scope.destinations),
            )?;
        }
        for cleanup in &pending {
            self.check_captured_object(
                &cleanup.filename,
                &cleanup.stamp,
                Some(&scope.destinations),
            )?;
        }
        inspected.finish(self, false)?;
        for candidate in candidates {
            self.check_captured_object(
                &candidate.filename,
                &Some(candidate.stamp.clone()),
                Some(&scope.destinations),
            )?;
            self.schedule_cleanup(&candidate.filename, candidate.stamp.size)?;
            self.cleanup_captured_pending_object(
                &candidate.filename,
                &Some(candidate.stamp),
                Some(&scope.destinations),
            )?;
        }
        for cleanup in pending {
            self.cleanup_captured_pending_object(
                &cleanup.filename,
                &cleanup.stamp,
                Some(&scope.destinations),
            )?;
        }
        Ok(())
    }
}

fn schedule_cleanup_on(connection: &Connection, filename: &str, size: u64) -> Result<()> {
    ensure!(
        valid_cleanup_filename(filename),
        "invalid pending cache object filename"
    );
    ensure!(size <= i64::MAX as u64, "pending cache object is too large");
    connection.execute(
        "INSERT INTO cache_pending_cleanup (filename,size) VALUES (?1,?2)
         ON CONFLICT(filename) DO UPDATE SET size=excluded.size",
        params![filename, size as i64],
    )?;
    Ok(())
}

fn read_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        key: row.get("key")?,
        filename: row.get("filename")?,
        size: row.get::<_, i64>("size")? as u64,
        pinned: row.get("pinned")?,
    })
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn verify_digest(path: &Path, filename: &str, size: u64) -> Result<bool> {
    let maximum = size
        .checked_add(1)
        .context("cache verification size overflow")?;
    verify_object(path, filename, size, &mut VerificationScope::new(maximum))
}

fn object_stamp(path: &Path) -> Result<Option<ObjectStamp>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(ObjectStamp::of(&metadata))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("inspect cached object identity"),
    }
}

#[cfg(test)]
pub(crate) fn verification_stamp_for_test(path: &Path) -> Result<impl std::fmt::Debug + PartialEq> {
    object_stamp(path)
}

fn owned_file_stamp(cache: &FileCache, filename: &str, file: &File) -> Result<ObjectStamp> {
    let stamp = ObjectStamp::of(&file.metadata()?);
    ensure!(
        stamp.regular
            && !stamp.symlink
            && object_stamp(&cache.cleanup_object_path(filename)?)?.as_ref() == Some(&stamp),
        "owned cache writer no longer matches its path"
    );
    Ok(stamp)
}

fn open_verification_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    Ok(options
        .open(path)
        .context("open cached object for verification")?)
}

fn verify_object(
    path: &Path,
    filename: &str,
    size: u64,
    scope: &mut VerificationScope,
) -> Result<bool> {
    ensure!(
        valid_filename(filename),
        "invalid verification object filename"
    );
    let before = object_stamp(path)?.context("cached verification object disappeared")?;
    ensure!(
        before.regular && !before.symlink && before.size == size,
        "cached verification object changed shape"
    );
    let mut file = open_verification_file(path)?;
    ensure!(
        ObjectStamp::of(&file.metadata()?) == before,
        "cached object changed before verification"
    );
    if before.permits_reuse() {
        if let Some(evidence) = scope.evidence.get(filename) {
            if evidence.expected_size == size && evidence.stamp == before {
                ensure!(
                    object_stamp(path)?.as_ref() == Some(&before),
                    "cached object changed while reusing verification"
                );
                return Ok(evidence.digest_matches);
            }
        }
    }
    let maximum = size
        .checked_add(1)
        .context("cache verification size overflow")?;
    // Pay for the complete expected payload and its overflow probe before the
    // first read. Repeated unchanged identities consume no further read work.
    if let Some(prepaid) = scope.prepaid.get_mut(filename) {
        ensure!(
            prepaid.stamp == before && prepaid.expected_size == size && prepaid.reads_left > 0,
            "cached object changed after bulk work admission"
        );
        prepaid.reads_left -= 1;
    } else {
        scope.reserve(maximum)?;
    }
    scope.stats.hashes = scope
        .stats
        .hashes
        .checked_add(1)
        .context("cache verification count overflow")?;
    let mut hash = Sha256::new();
    let mut read_bytes = 0_u64;
    let mut buffer = [0_u8; 32 * 1024];
    {
        let mut reader = (&mut file).take(maximum);
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            read_bytes += count as u64;
            scope.stats.read_bytes = scope
                .stats
                .read_bytes
                .checked_add(count as u64)
                .context("cache verification read count overflow")?;
            hash.update(&buffer[..count]);
        }
    }
    let after = ObjectStamp::of(&file.metadata()?);
    ensure!(
        after == before && object_stamp(path)?.as_ref() == Some(&after),
        "cached object changed during verification"
    );
    let digest_matches =
        read_bytes == size && format!("{:x}", hash.finalize()) == filename[65..129];
    scope.evidence.insert(
        filename.to_owned(),
        VerifiedObject {
            stamp: after,
            expected_size: size,
            digest_matches,
        },
    );
    Ok(digest_matches)
}

fn preverified_digest(
    path: &Path,
    filename: &str,
    size: u64,
    scope: &VerificationScope,
) -> Result<bool> {
    let before = object_stamp(path)?.context("preverified cache destination disappeared")?;
    ensure!(
        before.regular && !before.symlink && before.size == size && before.permits_reuse(),
        "cache destination lacks stable preverified identity"
    );
    let file = open_verification_file(path)?;
    ensure!(
        ObjectStamp::of(&file.metadata()?) == before,
        "cache destination changed after preflight"
    );
    let evidence = scope
        .evidence
        .get(filename)
        .filter(|evidence| evidence.expected_size == size && evidence.stamp == before)
        .context("existing cache destination was not admitted before publication")?;
    ensure!(
        object_stamp(path)?.as_ref() == Some(&before),
        "cache destination changed while consuming verification"
    );
    // This path deliberately cannot reserve or perform a new payload read.
    // A late object change rejects publication instead of escaping its budget.
    Ok(evidence.digest_matches)
}

fn valid_filename(filename: &str) -> bool {
    let Some(stem) = filename
        .strip_suffix(".bin")
        .or_else(|| filename.strip_suffix(".png"))
    else {
        return false;
    };
    stem.len() == 129
        && stem.as_bytes()[64] == b'-'
        && stem.bytes().enumerate().all(|(index, byte)| {
            index == 64 || byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
        })
}

fn valid_cleanup_filename(filename: &str) -> bool {
    if valid_filename(filename) {
        return true;
    }
    let Some((pid, sequence)) = filename
        .strip_prefix(".tmp-")
        .and_then(|name| name.split_once('-'))
    else {
        return false;
    };
    !pid.is_empty()
        && !sequence.is_empty()
        && pid.bytes().all(|byte| byte.is_ascii_digit())
        && sequence.bytes().all(|byte| byte.is_ascii_digit())
        && pid.parse::<u32>().is_ok()
        && sequence.parse::<u64>().is_ok()
}

#[cfg(test)]
#[path = "cache_verification_tests.rs"]
mod verification_tests;

#[cfg(test)]
#[path = "cache_inventory_tests.rs"]
mod inventory_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "weread-cache-{}-{}",
            std::process::id(),
            NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn payload_footprint(cache: &FileCache) -> Result<(usize, u64)> {
        let mut count = 0;
        let mut bytes = 0;
        for item in fs::read_dir(cache.root())? {
            let item = item?;
            if valid_cleanup_filename(&item.file_name().to_string_lossy()) {
                count += 1;
                bytes += fs::symlink_metadata(item.path())?.len();
            }
        }
        Ok((count, bytes))
    }

    fn pending_count(cache: &FileCache) -> Result<u64> {
        Ok(cache
            .connection
            .query_row("SELECT COUNT(*) FROM cache_pending_cleanup", [], |row| {
                row.get(0)
            })?)
    }

    #[test]
    fn keys_cannot_escape_cache_directory() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 100)?;
        let path = cache.put("../../outside/private.txt", b"cover")?;
        assert_eq!(path.parent(), Some(cache.root()));
        assert!(valid_filename(path.file_name().unwrap().to_str().unwrap()));
        assert!(!root.parent().unwrap().join("outside/private.txt").exists());
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn lru_eviction_respects_accesses_and_pins() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 9)?;
        cache.put("first", b"111")?;
        cache.put("second", b"222")?;
        cache.put("third", b"333")?;
        cache.set_pinned("first", true)?;
        cache.get("second")?;
        cache.put("fourth", b"444")?;
        assert!(cache.get("first")?.is_some());
        assert!(cache.get("second")?.is_some());
        assert!(cache.get("third")?.is_none());
        assert_eq!(cache.usage()?, 9);
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn commit_lookup_preserves_lru_order_without_writing_the_index() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 6)?;
        let first = cache.put("first", b"111")?;
        cache.put("second", b"222")?;
        let before: i64 = cache
            .connection
            .query_row("SELECT total_changes()", [], |row| row.get(0))?;
        assert_eq!(cache.get_for_commit("first")?, Some(first));
        let after: i64 = cache
            .connection
            .query_row("SELECT total_changes()", [], |row| row.get(0))?;
        assert_eq!(after, before);
        cache.put("third", b"333")?;
        assert!(cache.get_for_commit("first")?.is_none());
        assert!(cache.get_for_commit("second")?.is_some());
        assert!(cache.get_for_commit("third")?.is_some());
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn unchanged_pin_updates_report_existing_entries_without_writes() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 100)?;
        cache.put("object", b"111")?;
        assert_eq!(cache.pinned_state("object")?, Some(false));
        let before: i64 = cache
            .connection
            .query_row("SELECT total_changes()", [], |row| row.get(0))?;
        assert!(cache.set_pinned("object", false)?);
        let unchanged: i64 = cache
            .connection
            .query_row("SELECT total_changes()", [], |row| row.get(0))?;
        assert_eq!(unchanged, before);
        assert!(cache.set_pinned("object", true)?);
        assert_eq!(cache.pinned_state("object")?, Some(true));
        let pinned: i64 = cache
            .connection
            .query_row("SELECT total_changes()", [], |row| row.get(0))?;
        assert_eq!(pinned, before + 1);
        assert!(cache.set_pinned("object", true)?);
        let unchanged: i64 = cache
            .connection
            .query_row("SELECT total_changes()", [], |row| row.get(0))?;
        assert_eq!(unchanged, pinned);
        assert_eq!(cache.pinned_state("missing")?, None);
        assert!(!cache.set_pinned("missing", true)?);
        assert!(!cache.set_pinned("missing", false)?);
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn sufficient_budget_does_not_decode_unrelated_eviction_candidates() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 100)?;
        cache.put("unrelated", b"111")?;
        // An unreadable unrelated row makes an unnecessary eviction scan observable.
        cache.connection.execute(
            "UPDATE cache_entries SET pinned='invalid' WHERE key=?1",
            ["unrelated"],
        )?;
        let path = cache.put("current", b"222")?;
        assert_eq!(cache.get_for_commit("current")?, Some(path));
        assert_eq!(cache.pinned_state("current")?, Some(false));
        assert_eq!(cache.usage()?, 6);
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn replacing_a_pinned_object_preserves_its_pin_and_evicts_only_other_objects() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 6)?;
        let old_path = cache.put("replace", b"111")?;
        cache.set_pinned("replace", true)?;
        cache.put("other", b"222")?;
        let new_path = cache.put("replace", b"3333")?;
        assert_ne!(new_path, old_path);
        assert!(!old_path.exists());
        assert_eq!(cache.get_for_commit("replace")?, Some(new_path.clone()));
        assert_eq!(fs::read(new_path)?, b"3333");
        assert_eq!(cache.pinned_state("replace")?, Some(true));
        assert!(cache.get_for_commit("other")?.is_none());
        assert_eq!(cache.usage()?, 4);
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn failed_replacement_preserves_previous_entry() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 6)?;
        cache.put("active", b"111")?;
        cache.set_pinned("active", true)?;
        let old_path = cache.put("replace", b"222")?;
        assert!(cache.put("replace", b"3333").is_err());
        assert_eq!(cache.get("replace")?, Some(old_path.clone()));
        assert_eq!(fs::read(old_path)?, b"222");
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn repeated_index_write_failures_reclaim_new_payloads_in_the_same_process() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 100)?;
        let preserved = cache.put("preserved", b"old")?;
        cache.set_pinned("preserved", true)?;
        cache.connection.execute_batch(
            "CREATE TEMP TRIGGER reject_cache_insert BEFORE INSERT ON cache_entries
             BEGIN SELECT RAISE(ABORT,'fixture cache index failure'); END",
        )?;
        for index in 0..64 {
            let key = if index % 2 == 0 {
                "preserved".to_owned()
            } else {
                format!("new-{index}")
            };
            let bytes = format!("different-payload-{index}");
            assert!(cache.put(&key, bytes.as_bytes()).is_err());
            assert_eq!(cache.usage()?, 3);
            assert_eq!(payload_footprint(&cache)?, (1, 3));
            assert_eq!(pending_count(&cache)?, 0);
            assert_eq!(cache.get_for_commit("preserved")?, Some(preserved.clone()));
            assert_eq!(cache.pinned_state("preserved")?, Some(true));
        }
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn failed_eviction_transaction_reclaims_publication_and_keeps_old_entries() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 6)?;
        let pinned = cache.put("pinned", b"111")?;
        cache.set_pinned("pinned", true)?;
        let victim = cache.put("victim", b"222")?;
        cache.connection.execute_batch(
            "CREATE TEMP TRIGGER reject_cache_delete BEFORE DELETE ON cache_entries
             BEGIN SELECT RAISE(ABORT,'fixture cache eviction failure'); END",
        )?;
        for index in 0..32_u8 {
            assert!(cache.put(&format!("new-{index}"), &[index; 3]).is_err());
            assert_eq!(cache.usage()?, 6);
            assert_eq!(payload_footprint(&cache)?, (2, 6));
            assert_eq!(pending_count(&cache)?, 0);
            assert_eq!(cache.get_for_commit("pinned")?, Some(pinned.clone()));
            assert_eq!(cache.get_for_commit("victim")?, Some(victim.clone()));
            assert_eq!(cache.pinned_state("pinned")?, Some(true));
        }
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn repeated_corrupt_lookups_reclaim_payloads_without_reopening() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 100)?;
        for index in 0..64_u64 {
            let path = cache.put("damaged", &index.to_le_bytes())?;
            fs::write(&path, b"corrupt!")?;
            assert!(cache.get_for_commit("damaged")?.is_none());
            assert!(!path.exists());
            assert_eq!(cache.usage()?, 0);
            assert_eq!(payload_footprint(&cache)?, (0, 0));
            assert_eq!(pending_count(&cache)?, 0);
        }
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn failed_unlink_remains_accounted_and_blocks_new_payloads_until_retried() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 6)?;
        let blocked = cache.put("blocked", b"111")?;
        fs::remove_file(&blocked)?;
        // A directory forces unlink failure even when fixtures run as root. The
        // cache must never recursively delete an unexpected filesystem object.
        fs::create_dir(&blocked)?;
        assert!(cache.remove("blocked").is_err());
        assert!(cache.entry("blocked")?.is_none());
        assert_eq!(cache.usage()?, 3);
        assert_eq!(pending_count(&cache)?, 1);
        for index in 0..32 {
            assert!(cache.put(&format!("new-{index}"), b"222").is_err());
            assert_eq!(payload_footprint(&cache)?.0, 1);
            assert_eq!(cache.usage()?, 3);
            assert_eq!(pending_count(&cache)?, 1);
        }
        fs::remove_dir(&blocked)?;
        cache.put("recovered", b"333")?;
        assert_eq!(pending_count(&cache)?, 0);
        assert_eq!(payload_footprint(&cache)?, (1, 3));
        assert_eq!(cache.usage()?, 3);
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn failed_eviction_unlink_preserves_committed_payload_and_retry_budget() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 6)?;
        let blocked = cache.put("victim", b"111")?;
        cache.put("pinned", b"222")?;
        cache.set_pinned("pinned", true)?;
        fs::remove_file(&blocked)?;
        fs::create_dir(&blocked)?;
        assert!(cache.put("committed", b"333").is_err());
        let committed = cache.get_for_commit("committed")?.unwrap();
        assert_eq!(fs::read(&committed)?, b"333");
        assert_eq!(cache.pinned_state("pinned")?, Some(true));
        assert_eq!(cache.usage()?, 9);
        assert_eq!(pending_count(&cache)?, 1);
        for index in 0..16 {
            assert!(cache.put(&format!("retry-{index}"), b"444").is_err());
            assert_eq!(payload_footprint(&cache)?.0, 3);
            assert!(committed.is_file());
            assert_eq!(cache.usage()?, 9);
        }
        fs::remove_dir(&blocked)?;
        cache.put("recovered", b"555")?;
        assert_eq!(pending_count(&cache)?, 0);
        assert_eq!(payload_footprint(&cache)?, (2, 6));
        assert_eq!(cache.usage()?, 6);
        assert_eq!(cache.pinned_state("pinned")?, Some(true));
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn cleanup_guard_preserves_payload_referenced_by_a_committed_index() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 100)?;
        let path = cache.put("committed", b"111")?;
        cache.set_pinned("committed", true)?;
        let filename = path.file_name().unwrap().to_string_lossy().into_owned();
        {
            let _guard = PendingObject::new(&cache, &filename, 3)?;
            assert_eq!(cache.usage()?, 3);
            assert_eq!(pending_count(&cache)?, 1);
        }
        assert_eq!(pending_count(&cache)?, 0);
        assert_eq!(cache.get_for_commit("committed")?, Some(path.clone()));
        assert_eq!(fs::read(path)?, b"111");
        assert_eq!(cache.pinned_state("committed")?, Some(true));
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn pending_unlink_is_retried_after_reopening() -> Result<()> {
        let root = temporary_root();
        let blocked;
        {
            let cache = FileCache::open(&root, 100)?;
            blocked = cache.put("blocked", b"111")?;
            fs::remove_file(&blocked)?;
            fs::create_dir(&blocked)?;
            assert!(cache.remove("blocked").is_err());
            assert_eq!(pending_count(&cache)?, 1);
        }
        fs::remove_dir(blocked)?;
        let cache = FileCache::open(&root, 100)?;
        assert_eq!(cache.usage()?, 0);
        assert_eq!(pending_count(&cache)?, 0);
        assert_eq!(payload_footprint(&cache)?, (0, 0));
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn interrupted_temporary_payload_is_accounted_and_recovered_without_aging() -> Result<()> {
        let root = temporary_root();
        let temporary_name = format!(
            ".tmp-{}-{}",
            std::process::id(),
            NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
        );
        let temporary_path;
        {
            let cache = FileCache::open(&root, 100)?;
            temporary_path = cache.cleanup_object_path(&temporary_name)?;
            cache.schedule_cleanup(&temporary_name, 3)?;
            fs::write(&temporary_path, b"111")?;
            assert_eq!(cache.usage()?, 3);
            assert_eq!(pending_count(&cache)?, 1);
            assert_eq!(payload_footprint(&cache)?, (1, 3));
        }
        let cache = FileCache::open(&root, 100)?;
        assert!(!temporary_path.exists());
        assert_eq!(cache.usage()?, 0);
        assert_eq!(pending_count(&cache)?, 0);
        assert_eq!(payload_footprint(&cache)?, (0, 0));
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn fresh_legacy_temporary_payload_is_recovered_without_a_cleanup_record() -> Result<()> {
        let root = temporary_root();
        let temporary_name = format!(
            ".tmp-{}-{}",
            std::process::id(),
            NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
        );
        {
            let cache = FileCache::open(&root, 100)?;
            assert_eq!(pending_count(&cache)?, 0);
        }
        let temporary_path = root.join(&temporary_name);
        fs::write(&temporary_path, b"legacy pending payload")?;
        let unknown_file = root.join(".tmp-unrelated");
        let unknown_directory = root.join(".tmp-1-2-unrelated");
        fs::write(&unknown_file, b"preserved unrelated file")?;
        fs::create_dir(&unknown_directory)?;
        fs::write(
            unknown_directory.join("marker"),
            b"preserved unrelated data",
        )?;

        let cache = FileCache::open(&root, 100)?;
        assert!(!temporary_path.exists());
        assert_eq!(cache.usage()?, 0);
        assert_eq!(pending_count(&cache)?, 0);
        assert_eq!(payload_footprint(&cache)?, (0, 0));
        assert_eq!(fs::read(&unknown_file)?, b"preserved unrelated file");
        assert_eq!(
            fs::read(unknown_directory.join("marker"))?,
            b"preserved unrelated data"
        );
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn failed_legacy_temporary_cleanup_is_accounted_and_blocks_new_publication() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 6)?;
        let temporary_name = format!(
            ".tmp-{}-{}",
            std::process::id(),
            NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
        );
        let blocked = root.join(&temporary_name);
        fs::create_dir(&blocked)?;
        fs::write(blocked.join("marker"), b"do not recursively delete")?;
        let blocked_size = fs::symlink_metadata(&blocked)?.len();
        assert_eq!(pending_count(&cache)?, 0);
        assert!(cache.recover().is_err());
        assert_eq!(pending_count(&cache)?, 1);
        assert_eq!(cache.usage()?, blocked_size);
        for index in 0..16 {
            assert!(cache.put(&format!("blocked-{index}"), b"111").is_err());
            assert_eq!(pending_count(&cache)?, 1);
            assert_eq!(cache.usage()?, blocked_size);
            assert_eq!(payload_footprint(&cache)?, (1, blocked_size));
            assert_eq!(
                fs::read(blocked.join("marker"))?,
                b"do not recursively delete"
            );
        }
        fs::remove_file(blocked.join("marker"))?;
        fs::remove_dir(blocked)?;
        cache.put("recovered", b"222")?;
        assert_eq!(pending_count(&cache)?, 0);
        assert_eq!(cache.usage()?, 3);
        assert_eq!(payload_footprint(&cache)?, (1, 3));
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn cleanup_ledger_cannot_escape_the_cache_directory() -> Result<()> {
        let root = temporary_root();
        let external = root.with_extension("external");
        fs::write(&external, b"external")?;
        let cache = FileCache::open(&root, 100)?;
        let malicious = format!("../{}", external.file_name().unwrap().to_string_lossy());
        assert!(cache.schedule_cleanup(&malicious, 8).is_err());
        assert!(cache.cleanup_object_path(".tmp-1-2/../../outside").is_err());
        // Even a corrupted database row cannot bypass the filesystem boundary.
        cache.connection.execute(
            "INSERT INTO cache_pending_cleanup(filename,size) VALUES (?1,8)",
            [&malicious],
        )?;
        assert!(cache.put("blocked", b"111").is_err());
        assert_eq!(fs::read(&external)?, b"external");
        assert_eq!(payload_footprint(&cache)?, (0, 0));
        drop(cache);
        fs::remove_dir_all(root)?;
        fs::remove_file(external)?;
        Ok(())
    }

    #[test]
    fn restart_removes_orphans_and_missing_index_entries() -> Result<()> {
        let root = temporary_root();
        {
            let cache = FileCache::open(&root, 100)?;
            let path = cache.put("missing", b"111")?;
            fs::remove_file(path)?;
            let orphan = format!("{}-{}.bin", digest(b"orphan"), digest(b"body"));
            fs::write(cache.root().join(orphan), b"body")?;
        }
        let cache = FileCache::open(&root, 100)?;
        assert!(cache.get("missing")?.is_none());
        assert_eq!(cache.usage()?, 0);
        let objects = fs::read_dir(cache.root())?
            .filter_map(|entry| entry.ok())
            .filter(|entry| valid_filename(&entry.file_name().to_string_lossy()))
            .count();
        assert_eq!(objects, 0);
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn corrupt_same_size_object_is_not_returned() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 100)?;
        let path = cache.put("cover", b"good")?;
        fs::write(path, b"evil")?;
        assert!(cache.get("cover")?.is_none());
        assert_eq!(cache.usage()?, 0);
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn commit_lookup_rejects_corrupt_truncated_and_missing_objects() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 100)?;
        let corrupt = cache.put("corrupt", b"good")?;
        let truncated = cache.put("truncated", b"good")?;
        let missing = cache.put("missing", b"good")?;
        fs::write(corrupt, b"evil")?;
        fs::write(truncated, b"g")?;
        fs::remove_file(missing)?;
        assert!(cache.get_for_commit("corrupt")?.is_none());
        assert!(cache.get_for_commit("truncated")?.is_none());
        assert!(cache.get_for_commit("missing")?.is_none());
        assert_eq!(cache.usage()?, 0);
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn thumbnails_fit_reserved_geometry_and_use_only_sixteen_gray_levels() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 1024 * 1024)?;
        let source = image::RgbImage::from_fn(6, 4, |x, y| {
            image::Rgb([(x * 41) as u8, (y * 63) as u8, 151])
        });
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(source).write_to(&mut encoded, image::ImageFormat::Png)?;
        let path = cache.put_cover("thumbnail", encoded.get_ref(), 3, 3)?;
        let result = image::open(path)?.to_luma8();
        assert_eq!(result.dimensions(), (3, 2));
        assert!(result.pixels().all(|pixel| pixel[0] % 17 == 0));
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn canonical_gray8_cache_preserves_all_input_luminances() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 1024 * 1024)?;
        let source = image::GrayImage::from_fn(256, 1, |x, _| image::Luma([x as u8]));
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(source).write_to(&mut encoded, image::ImageFormat::Png)?;
        let path = cache.put_gray8("canonical-ramp", encoded.get_ref(), 256, 1)?;
        let result = image::open(path)?.into_luma8();
        assert_eq!(result.dimensions(), (256, 1));
        assert_eq!(result.into_raw(), (0_u8..=255).collect::<Vec<_>>());
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn canonical_gray8_cache_composites_transparency_on_white() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 1024 * 1024)?;
        let source =
            image::RgbaImage::from_raw(3, 1, vec![0, 0, 0, 0, 0, 0, 0, 128, 64, 64, 64, 255])
                .unwrap();
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(source).write_to(&mut encoded, image::ImageFormat::Png)?;
        let path = cache.put_gray8("canonical-alpha", encoded.get_ref(), 3, 1)?;
        let result = image::open(path)?.into_luma8();
        assert_eq!(result.into_raw(), vec![255, 127, 64]);
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn canonical_gray8_limits_reject_without_replacing_cached_objects() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 1024 * 1024)?;
        let retained = cache.put("canonical-limits", b"retained")?;
        let original_usage = cache.usage()?;
        let original_payloads = payload_footprint(&cache)?;
        for (width, height) in [(0, 1), (1, 0), (2049, 1), (1, 2049)] {
            let error = cache
                .put_gray8("canonical-limits", b"not an image", width, height)
                .unwrap_err();
            assert!(error.to_string().contains("geometry"));
        }
        let oversized = vec![0; 16 * 1024 * 1024 + 1];
        let error = cache
            .put_gray8("canonical-limits", &oversized, 1, 1)
            .unwrap_err();
        assert!(error.to_string().contains("size limit"));
        assert!(
            cache
                .put_gray8("canonical-limits", b"not an image", 1, 1)
                .is_err()
        );
        assert_eq!(fs::read(&retained)?, b"retained");
        assert_eq!(cache.get_for_commit("canonical-limits")?, Some(retained));
        assert_eq!(cache.usage()?, original_usage);
        assert_eq!(payload_footprint(&cache)?, original_payloads);
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn canonical_gray8_cache_respects_source_edges_and_target_aspect() -> Result<()> {
        let root = temporary_root();
        let cache = FileCache::open(&root, 1024 * 1024)?;
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::new_luma8(8193, 1).write_to(&mut encoded, image::ImageFormat::Png)?;
        assert!(
            cache
                .put_gray8("canonical-too-wide", encoded.get_ref(), 2048, 1)
                .is_err()
        );
        assert!(cache.get_for_commit("canonical-too-wide")?.is_none());
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::new_luma8(6, 4).write_to(&mut encoded, image::ImageFormat::Png)?;
        let path = cache.put_gray8("canonical-aspect", encoded.get_ref(), 3, 3)?;
        assert_eq!(image::open(path)?.into_luma8().dimensions(), (3, 2));
        drop(cache);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn cache_does_not_follow_object_symlinks() -> Result<()> {
        let root = temporary_root();
        let external = root.with_extension("external");
        fs::write(&external, b"111")?;
        let cache = FileCache::open(&root, 100)?;
        let path = cache.put("key", b"111")?;
        fs::remove_file(&path)?;
        std::os::unix::fs::symlink(&external, &path)?;
        assert!(cache.get("key")?.is_none());
        assert_eq!(fs::read(&external)?, b"111");
        drop(cache);
        fs::remove_dir_all(root)?;
        fs::remove_file(external)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn commit_lookup_does_not_follow_object_symlinks() -> Result<()> {
        let root = temporary_root();
        let external = root.with_extension("external");
        fs::write(&external, b"111")?;
        let cache = FileCache::open(&root, 100)?;
        let path = cache.put("key", b"111")?;
        fs::remove_file(&path)?;
        std::os::unix::fs::symlink(&external, &path)?;
        assert!(cache.get_for_commit("key")?.is_none());
        assert_eq!(cache.usage()?, 0);
        assert_eq!(fs::read(&external)?, b"111");
        drop(cache);
        fs::remove_dir_all(root)?;
        fs::remove_file(external)?;
        Ok(())
    }
}
