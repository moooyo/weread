//! Canonical Gray8 book images, separate from evictable cover and article thumbnails.
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, ensure};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    article::{AnchoredImage, image_anchors_in_document},
    cache::{
        CacheInspection, FileCache, InventoryBudget, InventoryLimitExceeded, RemovalPlan,
        VerificationScope,
    },
    source::images::{DownloadedChapterImages, ImageAsset},
    store::Store,
};

pub const DEFAULT_IMAGE_BUDGET: u64 = 512 * 1024 * 1024;
pub const DEFAULT_PREPARED_IMAGE_CACHE_BYTES: usize = 8 * 1024 * 1024;
const MANIFEST_PREFIX: &str = "chapter.assets:";
const CACHE_PREFIX: &str = "book-image:v2:";
const MAX_HTML_BYTES: usize = 8 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;
const MAX_IMAGE_REFERENCES: usize = 2048;
const MAX_IMAGE_ASSETS: usize = 512;
const MAX_PREPARED_PACKET_BYTES: usize = 64 * 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 4096;
const MAX_BULK_CHAPTERS: usize = 8192;
const BULK_BODY_READ_SCRATCH_BYTES: usize = 4 * 1024;
/// One chapter/Reader operation pays for unique image bytes and one overflow
/// probe per possible occurrence. Recovery uses the configured cache allowance.
pub const DEFAULT_IMAGE_VERIFICATION_WORK_BYTES: u64 =
    MAX_PREPARED_PACKET_BYTES as u64 + MAX_IMAGE_REFERENCES as u64;
const OUTPUT_WIDTH: u32 = 1860;
const OUTPUT_HEIGHT: u32 = 2480;
const PREPARED_CONVERSION_VERSION: u32 = 2;
const OUTPUT_GRAY_LEVELS: u16 = 256;
const OUTPUT_ALPHA_BACKGROUND: u8 = 255;

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredImage {
    /// A unique occurrence identity, even when two positions use the same asset.
    pub id: String,
    /// Decoded DOM attribute identity; private runtime metadata, never a cache path.
    pub source: String,
    /// SHA-256 of the acquired original bytes, before grayscale conversion.
    pub asset_id: Option<String>,
    pub cache_key: Option<String>,
    pub anchor: Option<u32>,
    /// Prepared PNG dimensions in physical pixels. Placeholders use zero.
    pub width: u32,
    pub height: u32,
    pub caption: String,
}

impl std::fmt::Debug for StoredImage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredImage")
            .field("id", &self.id)
            .field("asset_id", &self.asset_id)
            .field("anchor", &self.anchor)
            .field("dimensions", &(self.width, self.height))
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChapterManifest {
    pub version: u32,
    pub book_id: String,
    pub chapter_uid: String,
    pub html: String,
    /// Binds scalar image anchors to the exact body committed by the download checkpoint.
    pub text_sha256: String,
    pub images: Vec<StoredImage>,
    /// Unsupported image references keep a text-only manifest incomplete.
    pub unresolved_images: u32,
    /// True only after every referenced raster asset has been decoded, saved and pinned.
    pub complete: bool,
}

/// A single-use result of a fresh body and image verification operation.
/// Consume it immediately at the matching SQL checkpoint; never cache it.
pub(crate) struct VerifiedChapterImages {
    book_id: String,
    chapter_uid: String,
}

impl VerifiedChapterImages {
    pub(crate) fn book_id(&self) -> &str {
        &self.book_id
    }

    pub(crate) fn chapter_uid(&self) -> &str {
        &self.chapter_uid
    }
}

/// A single-use batch proof for an immediate task recovery transaction.
/// The book identity is retained once and the admitted UID set owns no bodies.
pub(crate) struct VerifiedBookImages {
    book_id: String,
    chapter_uids: BTreeSet<String>,
    retained_metadata_bytes: usize,
}

impl VerifiedBookImages {
    pub(crate) fn book_id(&self) -> &str {
        &self.book_id
    }

    pub(crate) fn contains(&self, chapter_uid: &str) -> bool {
        self.chapter_uids.contains(chapter_uid)
    }

    pub(crate) fn len(&self) -> usize {
        self.chapter_uids.len()
    }

    pub(crate) fn retained_metadata_bytes(&self) -> usize {
        self.retained_metadata_bytes
    }
}

/// A completed publication binds the image proof to the exact incoming body.
/// Only successful publication of a complete manifest can construct it.
pub(crate) struct VerifiedChapterPublication {
    book_id: String,
    chapter_uid: String,
    text_sha256: String,
}

impl VerifiedChapterPublication {
    pub(crate) fn matches_body(&self, book: &str, chapter_uid: &str, text: &str) -> bool {
        self.book_id == book
            && self.chapter_uid == chapter_uid
            && digest_matches_text(&Sha256::digest(text.as_bytes()).into(), &self.text_sha256)
    }
}

impl std::fmt::Debug for ChapterManifest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChapterManifest")
            .field("version", &self.version)
            .field("image_count", &self.images.len())
            .field("unresolved_images", &self.unresolved_images)
            .field("complete", &self.complete)
            .finish_non_exhaustive()
    }
}

/// Enumeration deliberately omits HTML so recovery and reference reconciliation
/// retain only image metadata, rather than a second copy of the entire book library.
#[derive(Default, Deserialize)]
#[serde(default)]
struct ManifestIndex {
    version: u32,
    #[serde(deserialize_with = "inventory_identifier")]
    book_id: String,
    #[serde(deserialize_with = "inventory_identifier")]
    chapter_uid: String,
    #[serde(deserialize_with = "inventory_digest")]
    text_sha256: String,
    #[serde(deserialize_with = "inventory_images")]
    images: Vec<StoredImage>,
    unresolved_images: u32,
    complete: bool,
}

#[derive(Clone, Copy)]
struct InventoryParseDenial {
    resource: &'static str,
    limit: usize,
    requested: usize,
}

thread_local! {
    static INVENTORY_PARSE_DENIAL: Cell<Option<InventoryParseDenial>> = const { Cell::new(None) };
}

fn inventory_denied<E: serde::de::Error>(
    resource: &'static str,
    limit: usize,
    requested: usize,
) -> E {
    INVENTORY_PARSE_DENIAL.with(|slot| {
        if slot.get().is_none() {
            slot.set(Some(InventoryParseDenial {
                resource,
                limit,
                requested,
            }));
        }
    });
    E::custom("image inventory field exceeds its allowance")
}

fn parse_inventory<T>(read: impl FnOnce() -> Result<T>) -> Result<T> {
    struct Restore(Option<InventoryParseDenial>);
    impl Drop for Restore {
        fn drop(&mut self) {
            INVENTORY_PARSE_DENIAL.with(|slot| slot.set(self.0));
        }
    }
    let restore = Restore(INVENTORY_PARSE_DENIAL.with(|slot| slot.replace(None)));
    let result = read();
    let denied = INVENTORY_PARSE_DENIAL.with(|slot| slot.take());
    drop(restore);
    if let Some(denied) = denied {
        return Err(InventoryLimitExceeded {
            resource: denied.resource,
            limit: denied.limit,
            requested: denied.requested,
        }
        .into());
    }
    result
}

fn inventory_text<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
    maximum: usize,
    resource: &'static str,
) -> std::result::Result<String, D::Error> {
    struct Text {
        maximum: usize,
        resource: &'static str,
    }
    impl serde::de::Visitor<'_> for Text {
        type Value = String;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("bounded image inventory text")
        }
        fn visit_str<E: serde::de::Error>(self, value: &str) -> std::result::Result<String, E> {
            if value.len() > self.maximum {
                return Err(inventory_denied(self.resource, self.maximum, value.len()));
            }
            Ok(value.to_owned())
        }
        fn visit_string<E: serde::de::Error>(
            self,
            value: String,
        ) -> std::result::Result<String, E> {
            if value.len() > self.maximum {
                return Err(inventory_denied(self.resource, self.maximum, value.len()));
            }
            Ok(value)
        }
    }
    deserializer.deserialize_string(Text { maximum, resource })
}

fn inventory_identifier<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<String, D::Error> {
    inventory_text(d, 4096, "manifest identifier bytes")
}
fn inventory_digest<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<String, D::Error> {
    inventory_text(d, 64, "manifest digest bytes")
}
fn inventory_source<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<String, D::Error> {
    inventory_text(d, MAX_SOURCE_BYTES, "manifest source bytes")
}
fn inventory_caption<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<String, D::Error> {
    inventory_text(d, 16 * 1024, "manifest caption bytes")
}
fn inventory_html<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    inventory_text(d, MAX_HTML_BYTES, "manifest HTML bytes")
}

fn inventory_optional<'de, D: serde::Deserializer<'de>>(
    d: D,
    maximum: usize,
    resource: &'static str,
) -> std::result::Result<Option<String>, D::Error> {
    struct Optional {
        maximum: usize,
        resource: &'static str,
    }
    impl<'de> serde::de::Visitor<'de> for Optional {
        type Value = Option<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("optional bounded image inventory text")
        }
        fn visit_none<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D: serde::Deserializer<'de>>(
            self,
            d: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            inventory_text(d, self.maximum, self.resource).map(Some)
        }
    }
    d.deserialize_option(Optional { maximum, resource })
}
fn inventory_asset_id<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    inventory_optional(d, 64, "manifest asset bytes")
}
fn inventory_cache_key<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    inventory_optional(d, CACHE_PREFIX.len() + 64, "manifest cache key bytes")
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct InventoryImage {
    #[serde(deserialize_with = "inventory_identifier")]
    id: String,
    #[serde(deserialize_with = "inventory_source")]
    source: String,
    #[serde(deserialize_with = "inventory_asset_id")]
    asset_id: Option<String>,
    #[serde(deserialize_with = "inventory_cache_key")]
    cache_key: Option<String>,
    anchor: Option<u32>,
    width: u32,
    height: u32,
    #[serde(deserialize_with = "inventory_caption")]
    caption: String,
}
impl From<InventoryImage> for StoredImage {
    fn from(i: InventoryImage) -> Self {
        Self {
            id: i.id,
            source: i.source,
            asset_id: i.asset_id,
            cache_key: i.cache_key,
            anchor: i.anchor,
            width: i.width,
            height: i.height,
            caption: i.caption,
        }
    }
}

fn inventory_images<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Vec<StoredImage>, D::Error> {
    struct Images;
    impl<'de> serde::de::Visitor<'de> for Images {
        type Value = Vec<StoredImage>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("bounded image inventory records")
        }
        fn visit_seq<S: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: S,
        ) -> std::result::Result<Self::Value, S::Error> {
            struct Image<'a>(&'a mut usize);
            impl<'de> serde::de::DeserializeSeed<'de> for Image<'_> {
                type Value = StoredImage;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    d: D,
                ) -> std::result::Result<Self::Value, D::Error> {
                    let count = self.0.checked_add(1).ok_or_else(|| {
                        inventory_denied::<D::Error>(
                            "manifest image rows",
                            MAX_IMAGE_REFERENCES,
                            usize::MAX,
                        )
                    })?;
                    if count > MAX_IMAGE_REFERENCES {
                        return Err(inventory_denied(
                            "manifest image rows",
                            MAX_IMAGE_REFERENCES,
                            count,
                        ));
                    }
                    *self.0 = count;
                    InventoryImage::deserialize(d).map(Into::into)
                }
            }
            let mut count = 0;
            let mut images = Vec::new();
            while let Some(image) = sequence.next_element_seed(Image(&mut count))? {
                if images.is_empty() {
                    images.reserve_exact(1);
                }
                images.push(image);
            }
            Ok(images)
        }
    }
    d.deserialize_seq(Images)
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct InventoryManifest {
    version: u32,
    #[serde(deserialize_with = "inventory_identifier")]
    book_id: String,
    #[serde(deserialize_with = "inventory_identifier")]
    chapter_uid: String,
    #[serde(deserialize_with = "inventory_html")]
    html: String,
    #[serde(deserialize_with = "inventory_digest")]
    text_sha256: String,
    #[serde(deserialize_with = "inventory_images")]
    images: Vec<StoredImage>,
    unresolved_images: u32,
    complete: bool,
}
impl From<InventoryManifest> for ChapterManifest {
    fn from(m: InventoryManifest) -> Self {
        Self {
            version: m.version,
            book_id: m.book_id,
            chapter_uid: m.chapter_uid,
            html: m.html,
            text_sha256: m.text_sha256,
            images: m.images,
            unresolved_images: m.unresolved_images,
            complete: m.complete,
        }
    }
}

struct InventoryManifestEntry {
    header: crate::store::SettingInventoryHeader,
    key: String,
    index: ManifestIndex,
    retired: bool,
}

struct PreparedInventory {
    manifests: Vec<InventoryManifestEntry>,
    inspected: Option<CacheInspection>,
    removals: RemovalPlan,
    replacements: Vec<Option<ChapterManifest>>,
    revision: (u64, u64),
    cache_revision: (u64, u64),
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct InventoryCount {
    images: CountImages,
}
#[derive(Default)]
struct CountImages(usize);
impl<'de> Deserialize<'de> for CountImages {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Count;
        impl<'de> serde::de::Visitor<'de> for Count {
            type Value = CountImages;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("image inventory count")
            }
            fn visit_seq<S: serde::de::SeqAccess<'de>>(
                self,
                mut sequence: S,
            ) -> std::result::Result<Self::Value, S::Error> {
                let mut count = 0_usize;
                while sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    count = count.checked_add(1).ok_or_else(|| {
                        <S::Error as serde::de::Error>::custom("image count arithmetic overflow")
                    })?;
                }
                Ok(CountImages(count))
            }
        }
        d.deserialize_seq(Count)
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AssetInventoryStats {
    pub headers: usize,
    pub manifest_reads: usize,
    pub full_manifest_reads: usize,
    pub body_metadata: usize,
    pub body_reads: usize,
    pub body_bytes: usize,
    pub body_work: usize,
    pub metadata_bytes: usize,
    pub peak_body_temporary: usize,
    pub image_counts: usize,
    pub projection_requested_bytes: usize,
    pub projection_admitted_bytes: usize,
}

#[derive(Default)]
struct JsonByteCounter {
    bytes: usize,
}
impl std::io::Write for JsonByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("image inventory JSON length overflow"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

const FORECAST_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const FORECAST_IMAGE_KEY: &str = concat!(
    "book-image:v2:",
    "0000000000000000000000000000000000000000000000000000000000000000"
);

#[derive(Serialize)]
struct ManifestOutputForecast<'a> {
    version: u32,
    book_id: &'a str,
    chapter_uid: &'a str,
    html: &'a str,
    text_sha256: &'a str,
    images: ForecastImages<'a>,
    unresolved_images: u32,
    complete: bool,
}
struct ForecastImages<'a>(&'a [AnchoredImage]);
impl Serialize for ForecastImages<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        #[derive(Serialize)]
        struct Image<'a> {
            id: &'a str,
            source: &'a str,
            asset_id: Option<&'a str>,
            cache_key: Option<&'a str>,
            anchor: Option<u32>,
            width: u32,
            height: u32,
            caption: &'a str,
        }
        let mut images = serializer.serialize_seq(Some(self.0.len()))?;
        for image in self.0 {
            // These values price the largest valid serialization only. They
            // are never an asset identity, lookup result or published manifest.
            images.serialize_element(&Image {
                id: &image.id,
                source: &image.source,
                asset_id: Some(FORECAST_DIGEST),
                cache_key: Some(FORECAST_IMAGE_KEY),
                anchor: image.anchor,
                width: OUTPUT_WIDTH,
                height: OUTPUT_HEIGHT,
                caption: &image.caption,
            })?;
        }
        images.end()
    }
}

fn forecast_manifest_output(
    budget: &mut InventoryBudget,
    book: &str,
    uid: &str,
    html: &str,
    metadata: &PreparedChapterMetadata,
) -> Result<()> {
    let mut strings = InventoryBudget::add(
        InventoryBudget::add(book.len(), uid.len())?,
        InventoryBudget::add(html.len(), metadata.text_sha256.len())?,
    )?;
    budget.rows(metadata.anchors.len())?;
    for image in &metadata.anchors {
        budget.admit("manifest identifier bytes", image.id.len(), 4096)?;
        budget.admit(
            "manifest source bytes",
            image.source.len(),
            MAX_SOURCE_BYTES,
        )?;
        budget.admit("manifest caption bytes", image.caption.len(), 16 * 1024)?;
        strings = InventoryBudget::add(
            strings,
            InventoryBudget::add(
                image.id.len(),
                InventoryBudget::add(image.source.len(), image.caption.len())?,
            )?,
        )?;
    }
    budget.work(InventoryBudget::add(
        InventoryBudget::mul(strings, 8)?,
        InventoryBudget::mul(metadata.anchors.len(), 512)?,
    )?)?;
    let mut counter = JsonByteCounter::default();
    serde_json::to_writer(
        &mut counter,
        &ManifestOutputForecast {
            version: 1,
            book_id: book,
            chapter_uid: uid,
            html,
            text_sha256: &metadata.text_sha256,
            images: ForecastImages(&metadata.anchors),
            unresolved_images: metadata.unresolved_images,
            complete: false,
        },
    )?;
    let slots = InventoryBudget::add(
        2 * std::mem::size_of::<ChapterManifest>(),
        InventoryBudget::mul(
            InventoryBudget::add(metadata.anchors.len(), 4)?,
            2 * std::mem::size_of::<StoredImage>() + 4 * (std::mem::size_of::<&str>() + 64),
        )?,
    )?;
    let asset_names = InventoryBudget::mul(metadata.anchors.len(), 64 + CACHE_PREFIX.len() + 64)?;
    budget.retain(InventoryBudget::add(
        slots,
        InventoryBudget::add(strings, asset_names)?,
    )?)?;
    // The body proof is gone before serialization. Only the maximum temporary
    // phase is held, while the counted JSON bounds both encoding and binding.
    budget.scratch(InventoryBudget::add(
        InventoryBudget::mul(counter.bytes, 3)?,
        4096,
    )?)?;
    budget.work(InventoryBudget::mul(counter.bytes, 2)?)?;
    budget.work(InventoryBudget::mul(
        metadata.anchors.len(),
        32 * (CACHE_PREFIX.len() + 64) + 512,
    )?)?;
    Ok(())
}

fn admit_manifest_serialization(
    budget: &mut InventoryBudget,
    manifest: &ChapterManifest,
) -> Result<()> {
    let mut strings = 0_usize;
    for value in [
        &manifest.html,
        &manifest.book_id,
        &manifest.chapter_uid,
        &manifest.text_sha256,
    ] {
        strings = InventoryBudget::add(strings, value.len())?;
    }
    for image in &manifest.images {
        for value in [
            image.id.as_str(),
            image.source.as_str(),
            image.caption.as_str(),
            image.asset_id.as_deref().unwrap_or_default(),
            image.cache_key.as_deref().unwrap_or_default(),
        ] {
            strings = InventoryBudget::add(strings, value.len())?;
        }
    }
    budget.work(InventoryBudget::add(
        InventoryBudget::mul(strings, 8)?,
        InventoryBudget::mul(manifest.images.len(), 512)?,
    )?)?;
    let mut counter = JsonByteCounter::default();
    serde_json::to_writer(&mut counter, manifest)?;
    budget.scratch(InventoryBudget::add(
        InventoryBudget::mul(counter.bytes, 3)?,
        4096,
    )?)?;
    budget.work(InventoryBudget::mul(counter.bytes, 2)?)
}

fn digest_matches_text(digest: &[u8; 32], expected: &str) -> bool {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    expected.len() == 64
        && digest.iter().enumerate().all(|(index, byte)| {
            expected.as_bytes()[index * 2] == HEX[usize::from(byte >> 4)]
                && expected.as_bytes()[index * 2 + 1] == HEX[usize::from(byte & 15)]
        })
}

#[derive(Clone, Copy)]
struct BulkVerificationLimits {
    chapters: usize,
    metadata_bytes: usize,
}

impl Default for BulkVerificationLimits {
    fn default() -> Self {
        Self {
            chapters: MAX_BULK_CHAPTERS,
            metadata_bytes: MAX_PREPARED_PACKET_BYTES,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("bulk chapter image verification exceeds its metadata allowance")]
pub struct BulkVerificationLimitExceeded;

struct BulkMetadataBudget {
    used: usize,
    maximum: usize,
}

impl BulkMetadataBudget {
    fn reserve(&mut self, bytes: usize) -> Result<()> {
        self.used = self
            .used
            .checked_add(bytes)
            .filter(|bytes| *bytes <= self.maximum)
            .ok_or(BulkVerificationLimitExceeded)?;
        Ok(())
    }

    fn remaining(&self) -> usize {
        self.maximum.saturating_sub(self.used)
    }
}

fn bulk_body_temporary_bytes(
    encoded_bytes: usize,
    utf8_upper: usize,
    utf16: bool,
) -> Result<usize> {
    // SQLite 3.50.2 can retain the raw overflow RCStr until cursor teardown.
    // Its UTF-16 converter requests this complete capacity and does not shrink
    // it to the decoded length. The borrowed UTF-8 view aliases that buffer;
    // the Rust owned body can coexist with both allocations inside the getter.
    let raw_cache = encoded_bytes
        .checked_add(3)
        .ok_or(BulkVerificationLimitExceeded)?;
    let conversion = if utf16 {
        (encoded_bytes & !1)
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(1))
            .ok_or(BulkVerificationLimitExceeded)?
    } else {
        0
    };
    let utf8_spans = utf8_upper
        .checked_mul(2)
        .ok_or(BulkVerificationLimitExceeded)?;
    let converted_and_owned = conversion
        .checked_add(utf8_upper)
        .ok_or(BulkVerificationLimitExceeded)?;
    // Keep the original two-span forecast as a floor for UTF-8 and separately
    // admit the larger converter/owned overlap. This is an operation forecast,
    // not an exact allocator or total-process RSS bound.
    raw_cache
        .checked_add(utf8_spans.max(converted_and_owned))
        .and_then(|bytes| bytes.checked_add(BULK_BODY_READ_SCRATCH_BYTES))
        .ok_or_else(|| BulkVerificationLimitExceeded.into())
}

#[derive(Default)]
struct BulkChapter {
    key: String,
    encoded_bytes: Option<usize>,
    encoded_upper: usize,
    images: usize,
    manifest: Option<ManifestIndex>,
    body_bytes: Option<usize>,
    body_upper: usize,
    body_matches: bool,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BulkVerificationStats {
    pub scope_count: usize,
    pub unique_chapters: usize,
    pub manifest_reads: usize,
    pub body_reads: usize,
    pub body_bytes: usize,
    pub body_temporary_bytes: usize,
    pub lookups: usize,
    pub hashes: usize,
    pub read_bytes: u64,
    pub charged_bytes: u64,
    pub metadata_bytes: usize,
    pub unique_keys: usize,
}

pub struct BookAssetStore {
    cache: FileCache,
    reconcile_required: Cell<bool>,
    #[cfg(test)]
    verification_stats: Cell<crate::cache::VerificationStats>,
    #[cfg(test)]
    source_match_stats: Cell<SourceMatchStats>,
    #[cfg(test)]
    bulk_stats: Cell<BulkVerificationStats>,
    #[cfg(test)]
    inventory_stats: Cell<AssetInventoryStats>,
}

struct PinAttempt {
    previous_pin: Option<bool>,
    available_before: bool,
}

struct PreparedAsset {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PreparedImageKey {
    original_sha256: String,
    conversion_version: u32,
    width: u32,
    height: u32,
    gray_levels: u16,
    alpha_background: u8,
}

impl PreparedImageKey {
    fn new(id: &str) -> Self {
        Self {
            original_sha256: id.to_owned(),
            conversion_version: PREPARED_CONVERSION_VERSION,
            width: OUTPUT_WIDTH,
            height: OUTPUT_HEIGHT,
            gray_levels: OUTPUT_GRAY_LEVELS,
            alpha_background: OUTPUT_ALPHA_BACKGROUND,
        }
    }
}

#[derive(Default)]
struct PreparedImageCacheState {
    entries: BTreeMap<PreparedImageKey, Arc<PreparedAsset>>,
    recent: VecDeque<PreparedImageKey>,
    used_bytes: usize,
    closed: bool,
}

/// One download task's bounded cache of checked PNG payloads. The byte budget
/// counts retained PNGs only, not decoder memory, map overhead, or packet Arcs.
/// Short locks never cover image decoding; cancellation closes the cache so an
/// already running blocking worker cannot repopulate it for a superseded task.
pub struct PreparedImageCache {
    maximum_bytes: usize,
    state: Mutex<PreparedImageCacheState>,
}

impl Default for PreparedImageCache {
    fn default() -> Self {
        Self::new(DEFAULT_PREPARED_IMAGE_CACHE_BYTES)
    }
}

impl PreparedImageCache {
    pub fn new(maximum_bytes: usize) -> Self {
        Self {
            maximum_bytes,
            state: Mutex::new(PreparedImageCacheState::default()),
        }
    }

    fn ensure_active(&self) -> Result<()> {
        ensure!(
            !self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .closed,
            "prepared image cache belongs to a cancelled download"
        );
        Ok(())
    }

    fn get(&self, key: &PreparedImageKey) -> Option<Arc<PreparedAsset>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return None;
        }
        let prepared = state.entries.get(key)?.clone();
        state.recent.retain(|recent| recent != key);
        state.recent.push_back(key.clone());
        Some(prepared)
    }

    fn insert(&self, key: PreparedImageKey, prepared: Arc<PreparedAsset>) {
        let size = prepared.bytes.len();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed || size > self.maximum_bytes {
            return;
        }
        if let Some(previous) = state.entries.remove(&key) {
            state.used_bytes -= previous.bytes.len();
            state.recent.retain(|recent| recent != &key);
        }
        while state.used_bytes > self.maximum_bytes - size {
            let Some(oldest) = state.recent.pop_front() else {
                break;
            };
            if let Some(previous) = state.entries.remove(&oldest) {
                state.used_bytes -= previous.bytes.len();
            }
        }
        state.used_bytes += size;
        state.recent.push_back(key.clone());
        state.entries.insert(key, prepared);
    }

    pub(crate) fn cancel(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        state.entries.clear();
        state.recent.clear();
        state.used_bytes = 0;
    }
}

/// Worker-derived image occurrences bound to the exact HTML and canonical text.
/// Private fields prevent callers from supplying unverified scalar metadata.
pub struct PreparedChapterMetadata {
    html_sha256: String,
    text_sha256: String,
    scalar_count: u32,
    anchors: Vec<AnchoredImage>,
    unresolved_images: u32,
}

impl std::fmt::Debug for PreparedChapterMetadata {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedChapterMetadata")
            .field("image_count", &self.anchors.len())
            .field("scalar_count", &self.scalar_count)
            .field("unresolved_images", &self.unresolved_images)
            .finish_non_exhaustive()
    }
}

/// Parse a chapter once on a worker, preserving the independent hidden/SVG
/// source checks and the existing Unicode scalar/caption matching behavior.
pub fn prepare_chapter_metadata(
    html: &str,
    canonical_text: &str,
) -> Result<PreparedChapterMetadata> {
    ensure!(
        html.len() <= MAX_HTML_BYTES,
        "chapter HTML exceeds the image-manifest limit"
    );
    let scalar_count = u32::try_from(canonical_text.chars().count())
        .context("chapter text exceeds the scalar-anchor range")?;
    let document = Html::parse_document(html);
    let anchors = image_anchors_in_document(&document, canonical_text)?;
    ensure!(
        anchors.len() <= MAX_IMAGE_REFERENCES,
        "chapter contains too many image occurrences"
    );
    Ok(PreparedChapterMetadata {
        html_sha256: digest(html.as_bytes()),
        text_sha256: digest(canonical_text.as_bytes()),
        scalar_count,
        anchors,
        unresolved_images: unresolved_images_in_document(&document)?,
    })
}

/// A checked CPU-stage result. Private fields prevent an unverified packet from
/// bypassing original-byte integrity checks or image decoding on its way to a commit.
pub struct PreparedChapterImages {
    assets: BTreeMap<String, Arc<PreparedAsset>>,
    sources: BTreeMap<String, BTreeSet<String>>,
}

impl std::fmt::Debug for PreparedChapterImages {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedChapterImages")
            .field("asset_count", &self.assets.len())
            .field("source_count", &self.sources.len())
            .finish_non_exhaustive()
    }
}

/// Run on a worker (for example `tokio::task::spawn_blocking`), never the UI thread.
/// Unreferenced archive images are discarded; original encoded bytes are consumed.
pub fn prepare_downloaded(downloaded: DownloadedChapterImages) -> Result<PreparedChapterImages> {
    prepare_downloaded_with_cache(downloaded, &PreparedImageCache::new(0))
}

/// Reuse a successful conversion within one task, while validating every new
/// original packet and rebuilding each chapter's source mapping independently.
pub fn prepare_downloaded_with_cache(
    downloaded: DownloadedChapterImages,
    cache: &PreparedImageCache,
) -> Result<PreparedChapterImages> {
    cache.ensure_active()?;
    let DownloadedChapterImages {
        assets,
        source_to_asset,
    } = downloaded;
    ensure!(
        assets.len() <= MAX_IMAGE_ASSETS,
        "chapter contains too many image assets"
    );
    let mut originals = BTreeMap::new();
    let mut sources = BTreeMap::<String, BTreeSet<String>>::new();
    let mut original_bytes = 0_usize;
    for asset in assets {
        cache.ensure_active()?;
        validate_asset(&asset)?;
        original_bytes = original_bytes
            .checked_add(asset.bytes.len())
            .context("chapter image packet size overflow")?;
        ensure!(
            original_bytes <= MAX_PREPARED_PACKET_BYTES,
            "chapter image packet exceeds its memory limit"
        );
        for source in &asset.source_names {
            add_source(&mut sources, source, &asset.id)?;
        }
        originals.insert(asset.id.clone(), asset);
    }
    for (source, id) in &source_to_asset {
        ensure!(
            originals.contains_key(id),
            "image source map references an absent asset"
        );
        add_source(&mut sources, source, id)?;
    }
    let required = if source_to_asset.is_empty() {
        originals.keys().cloned().collect::<BTreeSet<_>>()
    } else {
        source_to_asset.into_values().collect::<BTreeSet<_>>()
    };
    let mut prepared = BTreeMap::new();
    let mut prepared_bytes = 0_usize;
    for (id, asset) in originals {
        if !required.contains(&id) {
            continue;
        }
        cache.ensure_active()?;
        let key = PreparedImageKey::new(&id);
        let image = if let Some(image) = cache.get(&key) {
            image
        } else {
            let image = Arc::new(prepare_asset(&asset)?);
            cache.ensure_active()?;
            cache.insert(key, image.clone());
            image
        };
        prepared_bytes = prepared_bytes
            .checked_add(image.bytes.len())
            .context("prepared chapter image size overflow")?;
        ensure!(
            prepared_bytes <= MAX_PREPARED_PACKET_BYTES,
            "prepared chapter images exceed the memory limit"
        );
        prepared.insert(id, image);
    }
    cache.ensure_active()?;
    Ok(PreparedChapterImages {
        assets: prepared,
        sources,
    })
}

impl BookAssetStore {
    pub fn open(path: impl AsRef<Path>, maximum_bytes: u64) -> Result<Self> {
        Ok(Self {
            cache: FileCache::open(path, maximum_bytes)?,
            reconcile_required: Cell::new(true),
            #[cfg(test)]
            verification_stats: Cell::new(crate::cache::VerificationStats::default()),
            #[cfg(test)]
            source_match_stats: Cell::new(SourceMatchStats::default()),
            #[cfg(test)]
            bulk_stats: Cell::new(BulkVerificationStats::default()),
            #[cfg(test)]
            inventory_stats: Cell::new(AssetInventoryStats::default()),
        })
    }

    pub fn usage(&self) -> Result<u64> {
        self.cache.usage()
    }

    #[cfg(test)]
    pub(crate) fn inventory_read_stats(&self) -> AssetInventoryStats {
        self.inventory_stats.get()
    }
    #[cfg(test)]
    pub(crate) fn reset_inventory_read_stats(&self) {
        self.inventory_stats.set(AssetInventoryStats::default());
        self.cache.reset_inventory_read_stats();
    }
    #[cfg(test)]
    fn observe_inventory(&self, update: impl FnOnce(&mut AssetInventoryStats)) {
        let mut stats = self.inventory_stats.get();
        update(&mut stats);
        self.inventory_stats.set(stats);
    }

    fn json_inventory_temporary(
        snapshot: &crate::store::StoreResourceSnapshot<'_>,
        raw: usize,
        upper: usize,
    ) -> Result<usize> {
        let converted = if snapshot.utf8_upper_bytes(2)? == 3 {
            InventoryBudget::add(InventoryBudget::mul(raw & !1, 2)?, 1)?
        } else {
            upper
        };
        InventoryBudget::add(
            InventoryBudget::add(raw, 3)?,
            InventoryBudget::add(
                converted,
                InventoryBudget::add(InventoryBudget::mul(upper, 2)?, 4096)?,
            )?,
        )
    }

    fn indices_on(
        &self,
        snapshot: &crate::store::StoreResourceSnapshot<'_>,
        budget: &mut InventoryBudget,
    ) -> Result<Vec<InventoryManifestEntry>> {
        let headers = snapshot.setting_inventory_headers_with_prefix(MANIFEST_PREFIX)?;
        budget.rows(headers.len())?;
        budget.retain(InventoryBudget::mul(
            InventoryBudget::add(headers.len(), 4)?,
            2 * (std::mem::size_of::<InventoryManifestEntry>()
                + std::mem::size_of::<crate::store::SettingInventoryHeader>()),
        )?)?;
        // The complete prefix's raw/encoding/processing forecast precedes the
        // first owned key or JSON view. Ignored HTML is temporary input only.
        for header in &headers {
            if !header.key_is_text || !header.value_is_text {
                return Err(crate::store::StoreInventoryUnavailable.into());
            }
            budget.retain(InventoryBudget::add(
                InventoryBudget::mul(header.key_utf8_bytes_upper, 2)?,
                header.value_utf8_bytes_upper,
            )?)?;
            budget.scratch(Self::json_inventory_temporary(
                snapshot,
                header.value_encoded_bytes,
                header.value_utf8_bytes_upper,
            )?)?;
            // One count pass plus the final count/deserialization are three
            // JSON walks. The fourth span covers selected field copying.
            budget.work(InventoryBudget::add(
                InventoryBudget::mul(header.key_utf8_bytes_upper, 32)?,
                InventoryBudget::mul(header.value_utf8_bytes_upper, 4)?,
            )?)?;
        }
        #[cfg(test)]
        self.observe_inventory(|stats| stats.headers += headers.len());
        let mut staged = Vec::with_capacity(headers.len());
        for header in headers {
            let key = snapshot.bounded_setting_inventory_key(&header)?;
            let count = snapshot.image_manifest_inventory_metadata(
                &header,
                header.value_utf8_bytes_upper,
                MAX_IMAGE_REFERENCES,
            )?;
            let slots = InventoryBudget::mul(
                count,
                2 * std::mem::size_of::<StoredImage>()
                    + 4 * (std::mem::size_of::<String>() + CACHE_PREFIX.len() + 64 + 64),
            )?;
            #[cfg(test)]
            let projection_requested = InventoryBudget::add(budget.metadata_bytes(), slots)?;
            #[cfg(test)]
            self.observe_inventory(|stats| {
                stats.image_counts += 1;
                stats.projection_requested_bytes = projection_requested;
            });
            budget.retain(slots)?;
            #[cfg(test)]
            self.observe_inventory(|stats| {
                stats.projection_admitted_bytes = budget.metadata_bytes()
            });
            budget.work(InventoryBudget::mul(
                count,
                32 * (CACHE_PREFIX.len() + 64) + 512,
            )?)?;
            staged.push((header, key));
        }
        let mut manifests = Vec::with_capacity(staged.len());
        for (header, key) in staged {
            let index = parse_inventory(|| {
                snapshot.bounded_image_manifest_inventory::<ManifestIndex>(
                    &header,
                    header.value_utf8_bytes_upper,
                    MAX_IMAGE_REFERENCES,
                )
            })?;
            validate_identity(
                &key,
                index.version,
                &index.book_id,
                &index.chapter_uid,
                &index.text_sha256,
                index.unresolved_images,
                index.complete,
            )?;
            validate_images(&index.images, index.complete)?;
            #[cfg(test)]
            self.observe_inventory(|stats| stats.manifest_reads += 1);
            manifests.push(InventoryManifestEntry {
                header,
                key,
                index,
                retired: false,
            });
        }
        Ok(manifests)
    }

    fn prepare_inventory(
        &self,
        store: &Store,
        scope: &mut VerificationScope,
        budget: &mut InventoryBudget,
        recover: bool,
        excluded_book: Option<&str>,
        exclude_all: bool,
    ) -> Result<PreparedInventory> {
        self.cache.bind_verification(scope)?;
        self.cache.validate_verification(scope)?;
        let cache_revision = self.cache.inventory_revision()?;
        let snapshot = store.resource_verification_snapshot()?;
        let revision = snapshot.resource_revision();
        let mut manifests = self.indices_on(&snapshot, budget)?;
        for manifest in &mut manifests {
            manifest.retired =
                exclude_all || excluded_book == Some(manifest.index.book_id.as_str());
        }
        let all_keys = self.cache.inventory_keys_with_budget(budget, false)?;
        budget.retain(InventoryBudget::mul(
            InventoryBudget::add(all_keys.len(), 4)?,
            2 * std::mem::size_of::<&str>(),
        )?)?;
        let image_keys = all_keys
            .iter()
            .filter(|key| key.starts_with(CACHE_PREFIX))
            .map(String::as_str)
            .collect::<Vec<_>>();
        let removals = self
            .cache
            .plan_removal(image_keys.iter().copied(), budget, scope)?;
        let references = manifests
            .iter()
            .filter(|m| !m.retired)
            .flat_map(|m| m.index.images.iter())
            .filter_map(|image| image.cache_key.as_deref())
            .collect::<BTreeSet<_>>();
        let reference_keys = references.into_iter().collect::<Vec<_>>();
        let inspection_plan = if recover {
            Some(
                self.cache
                    .plan_borrowed_key_inspection(reference_keys.iter().copied(), budget)?,
            )
        } else {
            None
        };
        budget.retain(InventoryBudget::mul(
            InventoryBudget::add(manifests.len(), 4)?,
            2,
        )?)?;
        let mut body_matches = vec![false; manifests.len()];
        if recover {
            let count = manifests.len();
            budget.retain(InventoryBudget::mul(
                InventoryBudget::add(count, 4)?,
                4 * (std::mem::size_of::<(
                    (&str, &str),
                    Option<crate::store::ChapterBodyTextMetadata>,
                )>() + 64),
            )?)?;
            for manifest in &manifests {
                budget.work(InventoryBudget::add(
                    InventoryBudget::mul(
                        InventoryBudget::add(
                            manifest.index.book_id.len(),
                            manifest.index.chapter_uid.len(),
                        )?,
                        32,
                    )?,
                    64,
                )?)?;
            }
            let mut metadata = BTreeMap::new();
            let mut body_work = 0_u64;
            let mut body_temporary = 0usize;
            for manifest in manifests
                .iter()
                .filter(|m| !m.retired && m.index.unresolved_images == 0)
            {
                let identity = (
                    manifest.index.book_id.as_str(),
                    manifest.index.chapter_uid.as_str(),
                );
                if metadata.contains_key(&identity) {
                    continue;
                }
                let body = snapshot.chapter_body_metadata_by_uid(identity.0, identity.1)?;
                #[cfg(test)]
                self.observe_inventory(|stats| stats.body_metadata += 1);
                if let Some(body) = body.filter(|body| body.body_present) {
                    let conversion = if snapshot.utf8_upper_bytes(2)? == 3 {
                        InventoryBudget::add(InventoryBudget::mul(body.encoded_bytes & !1, 2)?, 1)?
                    } else {
                        body.utf8_bytes_upper
                    };
                    let temporary = InventoryBudget::add(
                        InventoryBudget::add(body.encoded_bytes, 3)?,
                        InventoryBudget::add(
                            conversion.max(InventoryBudget::mul(body.utf8_bytes_upper, 2)?),
                            4096,
                        )?,
                    )?;
                    body_temporary = body_temporary.max(temporary);
                    budget.scratch(body_temporary)?;
                    let work = InventoryBudget::add(
                        body.encoded_bytes,
                        InventoryBudget::mul(body.utf8_bytes_upper, 2)?,
                    )?;
                    budget.work(work)?;
                    body_work = body_work
                        .checked_add(body.utf8_bytes_upper as u64)
                        .context("image recovery body work overflow")?;
                    #[cfg(test)]
                    self.observe_inventory(|stats| {
                        stats.body_work += work;
                        stats.peak_body_temporary = body_temporary;
                    });
                }
                metadata.insert(identity, body);
            }
            scope.reserve_external_work(body_work)?;
            #[cfg(test)]
            inventory_tests::after_body_admission()?;
            let mut proofs = BTreeMap::new();
            for (identity, body) in metadata {
                let proof = if let Some(body) = body.filter(|body| body.body_present) {
                    snapshot.with_bounded_chapter_body_by_uid(
                        identity.0,
                        identity.1,
                        body.utf8_bytes_upper,
                        |text| {
                            #[cfg(test)]
                            self.observe_inventory(|stats| {
                                stats.body_reads += 1;
                                stats.body_bytes += text.len();
                            });
                            Ok(<[u8; 32]>::from(Sha256::digest(text.as_bytes())))
                        },
                    )?
                } else {
                    None
                };
                proofs.insert(identity, proof);
            }
            for (index, manifest) in manifests.iter().enumerate() {
                if manifest.retired {
                    continue;
                }
                if let Some(Some(proof)) = proofs.get(&(
                    manifest.index.book_id.as_str(),
                    manifest.index.chapter_uid.as_str(),
                )) {
                    body_matches[index] = digest_matches_text(proof, &manifest.index.text_sha256);
                }
            }
        }
        let inspected = inspection_plan
            .map(|plan| {
                self.cache
                    .inspect_planned_removal_keys(plan, scope, &removals)
            })
            .transpose()?;
        let mut replacements = Vec::with_capacity(manifests.len());
        for (index, manifest) in manifests.iter().enumerate() {
            let available = body_matches[index]
                && manifest.index.images.iter().all(|image| {
                    image
                        .cache_key
                        .as_ref()
                        .is_some_and(|key| inspected.as_ref().and_then(|i| i.path(key)).is_some())
                });
            if recover && !manifest.retired && manifest.index.complete && !available {
                budget.retain(InventoryBudget::add(
                    manifest.header.value_utf8_bytes_upper,
                    2 * std::mem::size_of::<ChapterManifest>(),
                )?)?;
                // This extra full view has its own count, deserialize and field
                // copy walks. Its work is paid before owning the historical HTML.
                budget.work(InventoryBudget::mul(
                    manifest.header.value_utf8_bytes_upper,
                    3,
                )?)?;
                #[cfg(test)]
                self.observe_inventory(|stats| stats.full_manifest_reads += 1);
                let mut replacement: ChapterManifest = parse_inventory(|| {
                    snapshot.bounded_image_manifest_inventory::<InventoryManifest>(
                        &manifest.header,
                        manifest.header.value_utf8_bytes_upper,
                        MAX_IMAGE_REFERENCES,
                    )
                })?
                .into();
                validate_manifest(&manifest.key, &replacement)?;
                replacement.complete = false;
                admit_manifest_serialization(budget, &replacement)?;
                replacements.push(Some(replacement));
            } else {
                replacements.push(None);
            }
        }
        // Release every raw SQLite/conversion/reader buffer before mutation.
        drop(snapshot);
        ensure!(
            store.resource_revision()? == revision,
            crate::store::StoreInventoryUnavailable
        );
        ensure!(
            self.cache.inventory_revision()? == cache_revision,
            crate::cache::InventoryUnavailable
        );
        #[cfg(test)]
        self.observe_inventory(|stats| stats.metadata_bytes = budget.metadata_bytes());
        Ok(PreparedInventory {
            manifests,
            inspected,
            removals,
            replacements,
            revision,
            cache_revision,
        })
    }

    fn inventory_references<'a>(
        inventory: &'a PreparedInventory,
        replaced: Option<(&str, &'a ChapterManifest)>,
    ) -> BTreeSet<&'a str> {
        let mut references = BTreeSet::new();
        for manifest in inventory.manifests.iter().filter(|m| !m.retired) {
            if replaced.is_some_and(|(key, _)| key == manifest.key) {
                continue;
            }
            references.extend(
                manifest
                    .index
                    .images
                    .iter()
                    .filter_map(|image| image.cache_key.as_deref()),
            );
        }
        if let Some((_, manifest)) = replaced {
            references.extend(
                manifest
                    .images
                    .iter()
                    .filter_map(|image| image.cache_key.as_deref()),
            );
        }
        references
    }

    fn apply_inventory_recovery(
        &self,
        store: &Store,
        scope: &VerificationScope,
        inventory: &mut PreparedInventory,
    ) -> Result<()> {
        ensure!(
            store.resource_revision()? == inventory.revision,
            crate::store::StoreInventoryUnavailable
        );
        ensure!(
            self.cache.inventory_revision()? == inventory.cache_revision,
            crate::cache::InventoryUnavailable
        );
        let inspected = inventory
            .inspected
            .as_ref()
            .context("image recovery inspection is unavailable")?;
        inspected.finish(&self.cache, false)?;
        inventory.cache_revision = self.cache.inventory_revision()?;
        for (manifest, replacement) in inventory.manifests.iter().zip(&inventory.replacements) {
            if manifest.retired {
                continue;
            }
            for image in &manifest.index.images {
                if let Some(key) = image
                    .cache_key
                    .as_ref()
                    .filter(|key| inspected.path(key).is_some())
                {
                    ensure!(
                        inspected.current_path(&self.cache, key)?.is_some(),
                        crate::cache::InventoryUnavailable
                    );
                    self.cache.set_pinned_with_verification(key, true, scope)?;
                }
            }
            if let Some(replacement) = replacement {
                ensure!(
                    store.resource_revision()? == inventory.revision,
                    crate::store::StoreInventoryUnavailable
                );
                self.cache.validate_verification(scope)?;
                store.set_setting(&manifest.key, replacement)?;
                let revision = store.resource_revision()?;
                ensure!(
                    revision.0 == inventory.revision.0
                        && inventory.revision.1.checked_add(1) == Some(revision.1),
                    crate::store::StoreInventoryUnavailable
                );
                inventory.revision = revision;
            }
        }
        self.cleanup_inventory(store, scope, inventory, None)
    }

    fn cleanup_inventory(
        &self,
        store: &Store,
        scope: &VerificationScope,
        inventory: &PreparedInventory,
        replaced: Option<(&str, &ChapterManifest)>,
    ) -> Result<()> {
        ensure!(
            store.resource_revision()? == inventory.revision,
            crate::store::StoreInventoryUnavailable
        );
        ensure!(
            self.cache.inventory_revision()?.0 == inventory.cache_revision.0,
            crate::cache::InventoryUnavailable
        );
        let referenced = Self::inventory_references(inventory, replaced);
        self.cache.apply_removal(&inventory.removals, scope, |key| {
            key.starts_with(CACHE_PREFIX) && !referenced.contains(key)
        })
    }

    fn record_verification(&self, scope: &VerificationScope) {
        #[cfg(test)]
        self.verification_stats.set(scope.stats());
        #[cfg(not(test))]
        let _ = scope;
    }

    fn lookup_previous_image<'a>(
        &self,
        index: &PreviousImageIndex<'a>,
        source: &str,
    ) -> Result<Option<&'a StoredImage>> {
        #[cfg(test)]
        self.source_match_stats.set(SourceMatchStats {
            lookups: self.source_match_stats.get().lookups + 1,
            ..self.source_match_stats.get()
        });
        index.image(source)
    }

    fn reconcile_before_publication(
        &self,
        store: &Store,
        scope: &mut VerificationScope,
        inventory: &mut Option<PreparedInventory>,
    ) -> Result<BTreeSet<String>> {
        if let Err(error) = self.cache.validate_verification(scope) {
            self.reconcile_required.set(true);
            return Err(error);
        }
        if self.reconcile_required.get() {
            let inventory = inventory
                .as_mut()
                .context("image publication has no admitted recovery inventory")?;
            self.apply_inventory_recovery(store, scope, inventory)
                .context("reconcile chapter image references before publication")?;
            self.reconcile_required.set(false);
            return Ok(Self::inventory_references(inventory, None)
                .into_iter()
                .map(str::to_owned)
                .collect());
        }
        Ok(BTreeSet::new())
    }

    /// Synchronous convenience for small fixtures or non-UI callers. Production source
    /// jobs prepare resources/metadata on a worker and commit them on the owner.
    pub fn commit_chapter(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uid: &str,
        html: &str,
        canonical_text: &str,
        downloaded: Option<&DownloadedChapterImages>,
    ) -> Result<ChapterManifest> {
        let prepared = downloaded.cloned().map(prepare_downloaded).transpose()?;
        self.commit_prepared_chapter(
            store,
            book_id,
            chapter_uid,
            html,
            canonical_text,
            prepared.as_ref(),
        )
    }

    /// Commit prepared resources before the caller commits body/task state and
    /// acknowledges its worker. This stage never decodes or re-encodes a large image.
    pub fn commit_prepared_chapter(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uid: &str,
        html: &str,
        canonical_text: &str,
        downloaded: Option<&PreparedChapterImages>,
    ) -> Result<ChapterManifest> {
        let metadata = prepare_chapter_metadata(html, canonical_text)?;
        self.commit_prepared_chapter_metadata(
            store,
            book_id,
            chapter_uid,
            html,
            canonical_text,
            &metadata,
            downloaded,
        )
    }

    /// Commit worker-derived occurrences after checking their content binding.
    /// This stage hashes the supplied body/HTML but never reparses its DOM.
    #[allow(clippy::too_many_arguments)]
    pub fn commit_prepared_chapter_metadata(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uid: &str,
        html: &str,
        canonical_text: &str,
        metadata: &PreparedChapterMetadata,
        downloaded: Option<&PreparedChapterImages>,
    ) -> Result<ChapterManifest> {
        self.commit_prepared_chapter_metadata_with_verification_budget(
            store,
            book_id,
            chapter_uid,
            html,
            canonical_text,
            metadata,
            downloaded,
            self.cache.verification_work_limit(),
        )
    }

    /// Mint a body-bound proof as part of the existing publication operation.
    /// An incomplete image set remains usable as text, but cannot finish an
    /// image-inclusive download chapter.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn commit_prepared_download_chapter_metadata(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uid: &str,
        html: &str,
        canonical_text: &str,
        metadata: &PreparedChapterMetadata,
        downloaded: Option<&PreparedChapterImages>,
    ) -> Result<Option<VerifiedChapterPublication>> {
        let manifest = self.commit_prepared_chapter_metadata(
            store,
            book_id,
            chapter_uid,
            html,
            canonical_text,
            metadata,
            downloaded,
        )?;
        Ok(manifest.complete.then(|| VerifiedChapterPublication {
            book_id: manifest.book_id,
            chapter_uid: manifest.chapter_uid,
            text_sha256: manifest.text_sha256,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn commit_prepared_chapter_metadata_with_verification_budget(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uid: &str,
        html: &str,
        canonical_text: &str,
        metadata: &PreparedChapterMetadata,
        downloaded: Option<&PreparedChapterImages>,
        maximum_work_bytes: u64,
    ) -> Result<ChapterManifest> {
        #[cfg(test)]
        self.source_match_stats.set(SourceMatchStats::default());
        let mut scope = VerificationScope::new(maximum_work_bytes);
        let result = self.commit_prepared_using_verification(
            store,
            book_id,
            chapter_uid,
            html,
            canonical_text,
            metadata,
            downloaded,
            &mut scope,
        );
        self.record_verification(&scope);
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_prepared_using_verification(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uid: &str,
        html: &str,
        canonical_text: &str,
        metadata: &PreparedChapterMetadata,
        downloaded: Option<&PreparedChapterImages>,
        scope: &mut VerificationScope,
    ) -> Result<ChapterManifest> {
        let key = manifest_key(book_id, chapter_uid)?;
        ensure!(
            html.len() <= MAX_HTML_BYTES,
            "chapter HTML exceeds the image-manifest limit"
        );
        ensure!(
            metadata.html_sha256 == digest(html.as_bytes())
                && metadata.text_sha256 == digest(canonical_text.as_bytes()),
            "prepared chapter metadata does not match its HTML and canonical text"
        );
        let scalar_count = metadata.scalar_count;
        let initial_revision = store.resource_revision()?;
        let initial_cache_revision = self.cache.inventory_revision()?;
        let mut inventory_budget = self.cache.inventory_budget();
        forecast_manifest_output(&mut inventory_budget, book_id, chapter_uid, html, metadata)?;
        // Independent borrowed admission precedes old cache reads, destination
        // verification, debt, pins and publication. Logical source failures are
        // retained in the projection until the original phase requests them.
        let downloaded_sources = if metadata.anchors.is_empty() {
            None
        } else {
            downloaded
                .map(|downloaded| {
                    DownloadedSourceIndex::new(&downloaded.sources, &metadata.anchors, scalar_count)
                })
                .transpose()
                .context("admit downloaded chapter image sources")?
        };
        let previous = match self.raw_chapter_with_budget(
            store,
            book_id,
            chapter_uid,
            &mut inventory_budget,
        ) {
            Ok(previous) => previous,
            Err(error) => {
                self.reconcile_required.set(true);
                return Err(error);
            }
        };
        let unresolved_images = metadata.unresolved_images;
        if downloaded.is_some() {
            ensure!(
                unresolved_images == 0,
                "chapter contains unsupported or missing image sources"
            );
        }
        let mut destinations = BTreeMap::<String, &[u8]>::new();
        if let Some(downloaded) = downloaded {
            for anchor in &metadata.anchors {
                if anchor.id.is_empty()
                    || anchor.source.len() > 4096
                    || anchor.source.chars().any(char::is_control)
                    || anchor.anchor.is_some_and(|anchor| anchor > scalar_count)
                {
                    continue;
                }
                // Unavailable sources never reach put. Leave their original
                // materialization error in sequence so a prior shared repair
                // can still survive rollback of this failed new manifest.
                let Ok(Some(id)) = downloaded_sources
                    .as_ref()
                    .expect("Downloaded occurrences have a source projection")
                    .id(&anchor.source)
                else {
                    continue;
                };
                let Some(asset) = downloaded.assets.get(id) else {
                    continue;
                };
                let Ok(key) = asset_cache_key(id) else {
                    continue;
                };
                destinations.entry(key).or_insert(asset.bytes.as_slice());
            }
        }
        let candidates = destinations
            .iter()
            .map(|(key, bytes)| (key.as_str(), *bytes))
            .collect::<Vec<_>>();
        self.cache
            .admit_write_inventory(&candidates, &mut inventory_budget)?;
        // A valid physical destination may survive without this logical index
        // entry, or be held by another alias. Put would still inspect its bytes.
        // Admit those reads before recovery can retire debt or change any pin.
        if let Err(error) = self.cache.preverify_png_destinations(&candidates, scope) {
            self.reconcile_required.set(true);
            return Err(error).context("verify existing image destinations before publication");
        }
        // Build one borrowed source projection before recovery can change pins
        // or debt. Ambiguity is retained as metadata until the original lookup
        // requests it; an unrelated old source cannot abort a shared repair.
        let previous_images = if downloaded.is_none() && !metadata.anchors.is_empty() {
            match previous
                .as_ref()
                .map(|previous| PreviousImageIndex::new(previous, &metadata.anchors))
                .transpose()
            {
                Ok(index) => index,
                Err(error) => {
                    self.reconcile_required.set(true);
                    return Err(error).context("admit previous chapter image sources");
                }
            }
        } else {
            None
        };
        #[cfg(test)]
        if let Some(index) = &previous_images {
            self.source_match_stats.set(index.stats);
        }
        if !metadata.anchors.is_empty() {
            let count = InventoryBudget::add(
                destinations.len(),
                previous.as_ref().map_or(0, |m| m.images.len()),
            )?;
            inventory_budget.retain(InventoryBudget::mul(
                count,
                2 * (std::mem::size_of::<&str>() + 64),
            )?)?;
            inventory_budget.work(InventoryBudget::mul(count, 512)?)?;
            let mut lookup_keys = destinations
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            if let Some(previous) = &previous {
                for image in &previous.images {
                    if let Some(key) = checked_image_key(image)? {
                        lookup_keys.insert(key);
                    }
                }
            }
            // Recovery can retire old rows, but cannot enlarge this later probe
            // set. Its complete metadata price precedes the first shared repair.
            self.cache
                .admit_inspection_inventory(lookup_keys.iter().copied(), &mut inventory_budget)?;
        }
        let dirty = self.reconcile_required.get();
        let needs_cleanup = previous.as_ref().is_some_and(|previous| {
            previous
                .images
                .iter()
                .any(|image| image.cache_key.is_some())
        });
        let mut inventory = if dirty || needs_cleanup {
            match self.prepare_inventory(store, scope, &mut inventory_budget, dirty, None, false) {
                Ok(inventory) => Some(inventory),
                Err(error) => {
                    self.reconcile_required.set(true);
                    return Err(error);
                }
            }
        } else {
            None
        };
        ensure!(
            store.resource_revision()? == initial_revision,
            crate::store::StoreInventoryUnavailable
        );
        ensure!(
            self.cache.inventory_revision()? == initial_cache_revision,
            crate::cache::InventoryUnavailable
        );
        let recovered_references =
            self.reconcile_before_publication(store, scope, &mut inventory)?;
        let mut manifest = ChapterManifest {
            version: 1,
            book_id: book_id.to_owned(),
            chapter_uid: chapter_uid.to_owned(),
            html: html.to_owned(),
            text_sha256: metadata.text_sha256.clone(),
            unresolved_images,
            ..ChapterManifest::default()
        };
        let mut attempts = BTreeMap::<String, PinAttempt>::new();
        // A panic, ambiguous write, or failed rollback must never let a later
        // publication bypass reconstruction of the durable reference graph.
        self.reconcile_required.set(true);
        let mut probe_keys = BTreeSet::new();
        for anchor in &metadata.anchors {
            if anchor.id.is_empty()
                || anchor.source.len() > 4096
                || anchor.source.chars().any(char::is_control)
                || anchor.anchor.is_some_and(|anchor| anchor > scalar_count)
            {
                continue;
            }
            if let Some(downloaded) = downloaded {
                let Ok(Some(id)) = downloaded_sources
                    .as_ref()
                    .expect("Downloaded occurrences have a source projection")
                    .id(&anchor.source)
                else {
                    continue;
                };
                if !downloaded.assets.contains_key(id) {
                    continue;
                }
                let Ok(key) = asset_cache_key(id) else {
                    continue;
                };
                probe_keys.insert(key);
            } else if let Some(index) = &previous_images {
                if let Some(cached) = self.lookup_previous_image(index, &anchor.source)? {
                    if let Some(key) = checked_image_key(cached)? {
                        probe_keys.insert(key.to_owned());
                    }
                }
            }
        }
        let keys = probe_keys.into_iter().collect::<Vec<_>>();
        let old_pins = keys
            .iter()
            .map(|key| Ok((key.clone(), self.cache.pinned_state(key)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        // Budget the complete reusable set before any pin, payload or manifest
        // mutation. Corrupt-object retirement is deferred until this succeeds.
        ensure!(
            self.cache.inventory_revision()?.0 == initial_cache_revision.0,
            crate::cache::InventoryUnavailable
        );
        let inspected: CacheInspection = self.cache.inspect_keys(&keys, scope)?;
        for key in &keys {
            let required_pin = recovered_references.contains(key)
                || previous.as_ref().is_some_and(|manifest| {
                    manifest
                        .images
                        .iter()
                        .any(|image| image.cache_key.as_ref() == Some(key))
                });
            attempts.insert(
                key.clone(),
                PinAttempt {
                    // Missing/corrupt entries can have lost their old index pin.
                    // Durable references still require a repaired shared payload
                    // to survive rollback of this new manifest.
                    previous_pin: if required_pin {
                        Some(true)
                    } else {
                        old_pins[key]
                    },
                    available_before: inspected.path(key).is_some(),
                },
            );
        }
        inspected.finish(&self.cache, false)?;
        let materialized = (|| -> Result<()> {
            // Preserve every reused object before a new payload can evict an
            // unpinned candidate appearing later in the occurrence list.
            for (key, attempt) in &attempts {
                if attempt.available_before {
                    ensure!(
                        self.cache.set_pinned_with_verification(key, true, scope)?,
                        "verified chapter image disappeared before pinning"
                    );
                }
            }
            let mut prepared_sizes = BTreeMap::<String, (u32, u32)>::new();
            for anchor in &metadata.anchors {
                ensure!(
                    !anchor.id.is_empty()
                        && anchor.source.len() <= 4096
                        && !anchor.source.chars().any(char::is_control),
                    "invalid chapter image occurrence"
                );
                ensure!(
                    anchor.anchor.is_none_or(|anchor| anchor <= scalar_count),
                    "image anchor exceeds canonical text"
                );
                let mut image = StoredImage {
                    id: anchor.id.clone(),
                    source: anchor.source.clone(),
                    anchor: anchor.anchor,
                    caption: anchor.caption.clone(),
                    ..StoredImage::default()
                };
                if let Some(downloaded) = downloaded {
                    let id = downloaded_sources
                        .as_ref()
                        .expect("Downloaded occurrences have a source projection")
                        .id(&image.source)?
                        .context("a referenced chapter image was not acquired")?;
                    let cache_key = asset_cache_key(id)?;
                    let (width, height) = if let Some(dimensions) = prepared_sizes.get(id) {
                        *dimensions
                    } else {
                        let asset = downloaded
                            .assets
                            .get(id)
                            .context("a referenced chapter image is absent")?;
                        // Read the old pin before integrity checking can remove a
                        // corrupt index entry shared with another chapter/book.
                        let existed = attempts[&cache_key].available_before;
                        if !existed {
                            self.cache
                                .put_png_preverified(&cache_key, &asset.bytes, scope)?;
                        }
                        let dimensions = (asset.width, asset.height);
                        ensure!(
                            self.cache
                                .set_pinned_with_verification(&cache_key, true, scope)?,
                            "prepared chapter image disappeared before pinning"
                        );
                        prepared_sizes.insert(id.to_owned(), dimensions);
                        dimensions
                    };
                    image.asset_id = Some(id.to_owned());
                    image.cache_key = Some(cache_key);
                    image.width = width;
                    image.height = height;
                } else if let Some(index) = &previous_images {
                    if let Some(cached) = self.lookup_previous_image(index, &image.source)? {
                        let cache_key = cached
                            .cache_key
                            .as_ref()
                            .context("previous chapter image has no cache identity")?;
                        if inspected.path(cache_key).is_some() {
                            image.asset_id = cached.asset_id.clone();
                            image.cache_key = cached.cache_key.clone();
                            image.width = cached.width;
                            image.height = cached.height;
                            if let Some(cache_key) = &image.cache_key {
                                ensure!(
                                    self.cache
                                        .set_pinned_with_verification(cache_key, true, scope)?,
                                    "previous chapter image disappeared before pinning"
                                );
                            }
                        }
                    }
                }
                manifest.images.push(image);
            }
            manifest.complete = unresolved_images == 0
                && manifest
                    .images
                    .iter()
                    .all(|image| image.asset_id.is_some() && image.cache_key.is_some());
            validate_manifest(&key, &manifest)?;
            self.cache.validate_verification(scope)?;
            Ok(())
        })();
        if let Err(error) = materialized {
            if let Err(rollback) = self.rollback_attempts(&attempts, scope) {
                return Err(error).context(format!(
                    "materialize chapter images; rollback remains pending: {rollback:#}"
                ));
            }
            return Err(error).context("materialize chapter images");
        }
        let committed = (|| {
            self.cache.validate_verification(scope)?;
            store.set_setting(&key, &manifest)
        })();
        if let Err(error) = committed {
            // If durable metadata cannot be read after an I/O failure, preserve pins;
            // the next publication must determine whether the write actually committed.
            if let Ok(current) = store.setting::<ChapterManifest>(&key) {
                if current.as_ref() != Some(&manifest) {
                    if let Err(rollback) = self.rollback_attempts(&attempts, scope) {
                        return Err(error).context(format!(
                            "commit chapter image manifest; rollback remains pending: {rollback:#}"
                        ));
                    }
                }
            }
            return Err(error).context("commit chapter image manifest");
        }
        // Resources and metadata are committed. Maintenance failure cannot turn this
        // successful commit into a reported rollback. A later publication retries it.
        let current_keys = manifest
            .images
            .iter()
            .filter_map(|image| image.cache_key.as_deref())
            .collect::<HashSet<_>>();
        let replaced_assets = previous.as_ref().is_some_and(|previous| {
            previous
                .images
                .iter()
                .filter_map(|image| image.cache_key.as_deref())
                .any(|key| !current_keys.contains(key))
        });
        if replaced_assets {
            let cleanup = (|| -> Result<()> {
                let inventory = inventory
                    .as_mut()
                    .context("committed image maintenance has no admitted inventory")?;
                let revision = store.resource_revision()?;
                ensure!(
                    revision.0 == inventory.revision.0
                        && inventory.revision.1.checked_add(1) == Some(revision.1),
                    crate::store::StoreInventoryUnavailable
                );
                inventory.revision = revision;
                self.cleanup_inventory(store, scope, inventory, Some((&key, &manifest)))
            })();
            if cleanup.is_ok() {
                self.reconcile_required.set(false);
            }
        } else if self.cache.validate_verification(scope).is_ok() {
            self.reconcile_required.set(false);
        }
        Ok(manifest)
    }

    /// Legacy text-only chapters have no manifest. A checkpoint interrupted between
    /// manifest and body commits also returns None rather than misplacing images.
    pub fn chapter(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uid: &str,
    ) -> Result<Option<ChapterManifest>> {
        let result = (|| {
            let Some(manifest) = self.raw_chapter(store, book_id, chapter_uid)? else {
                return Ok(None);
            };
            if !body_matches(store, &manifest)? {
                return Ok(None);
            }
            Ok(Some(manifest))
        })();
        if result.is_err() {
            // The UI can tolerate unavailable pictures, but unreadable durable
            // reference or body-binding evidence must gate later publication.
            self.reconcile_required.set(true);
        }
        result
    }

    pub fn image_path(&self, image: &StoredImage) -> Result<Option<PathBuf>> {
        self.image_path_with_access(image, true)
    }

    /// Resolve one Reader/reflow batch. Paths preserve occurrence order, while
    /// physical verification and successful recency touches use unique keys.
    /// Consume the result in this operation; do not retain it as a cross-turn
    /// integrity cache. A failed batch returns no partially verified paths.
    pub fn image_paths(&self, images: &[StoredImage]) -> Result<Vec<Option<PathBuf>>> {
        self.image_paths_with_verification_budget(images, DEFAULT_IMAGE_VERIFICATION_WORK_BYTES)
    }

    pub fn image_paths_with_verification_budget(
        &self,
        images: &[StoredImage],
        maximum_work_bytes: u64,
    ) -> Result<Vec<Option<PathBuf>>> {
        let mut scope = VerificationScope::new(maximum_work_bytes);
        let result = self.image_paths_using_verification(images, &mut scope, true);
        self.record_verification(&scope);
        result
    }

    fn image_path_with_access(&self, image: &StoredImage, touch: bool) -> Result<Option<PathBuf>> {
        let mut scope = VerificationScope::new(DEFAULT_IMAGE_VERIFICATION_WORK_BYTES);
        let result = self
            .image_paths_using_verification(std::slice::from_ref(image), &mut scope, touch)
            .map(|mut paths| paths.remove(0));
        self.record_verification(&scope);
        result
    }

    fn image_paths_using_verification(
        &self,
        images: &[StoredImage],
        scope: &mut VerificationScope,
        touch: bool,
    ) -> Result<Vec<Option<PathBuf>>> {
        let result = (|| {
            ensure!(
                images.len() <= MAX_IMAGE_REFERENCES,
                "chapter image lookup has too many occurrences"
            );
            let aligned = images
                .iter()
                .map(checked_image_key)
                .collect::<Result<Vec<_>>>()?;
            let keys = aligned
                .iter()
                .flatten()
                .map(|key| (*key).to_owned())
                .collect::<Vec<_>>();
            let inspected = self.cache.inspect_keys(&keys, scope)?;
            if inspected.has_invalid_entries() {
                self.reconcile_required.set(true);
            }
            inspected.finish(&self.cache, touch)?;
            Ok(aligned
                .into_iter()
                .map(|key| key.and_then(|key| inspected.path(key).cloned()))
                .collect())
        })();
        if result.is_err() {
            // Integrity lookup can retire an entry before unlink fails. Keep
            // that ledger debt visible even when a caller renders a placeholder.
            self.reconcile_required.set(true);
        }
        result
    }

    pub fn complete(&self, store: &Store, book_id: &str, chapter_uid: &str) -> Result<bool> {
        self.complete_with_verification_budget(
            store,
            book_id,
            chapter_uid,
            DEFAULT_IMAGE_VERIFICATION_WORK_BYTES,
        )
    }

    pub fn complete_with_verification_budget(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uid: &str,
        maximum_work_bytes: u64,
    ) -> Result<bool> {
        let mut scope = VerificationScope::new(maximum_work_bytes);
        let result = (|| {
            let Some(manifest) = self.chapter(store, book_id, chapter_uid)? else {
                return Ok(false);
            };
            if !manifest.complete || manifest.unresolved_images != 0 {
                return Ok(false);
            }
            // Completeness is validation, not a rendered access. Keep its LRU
            // sequence unchanged and verify each shared image only once.
            let paths = self.image_paths_using_verification(&manifest.images, &mut scope, false)?;
            Ok(paths.iter().all(Option::is_some))
        })();
        self.record_verification(&scope);
        result
    }

    /// One UI acquisition decision verifies all requested chapters together.
    /// Results preserve input order; duplicate UIDs reuse the same proof only
    /// within this operation. The caller still owns current account/access checks.
    pub fn complete_many(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uids: &[&str],
    ) -> Result<Vec<bool>> {
        self.complete_many_with_verification_budget(
            store,
            book_id,
            chapter_uids,
            self.cache.verification_work_limit(),
        )
    }

    /// Mint a checkpoint proof only after revalidating this exact body and all
    /// referenced payloads. Download-plan hints alone cannot mint this proof.
    pub(crate) fn verify_complete_checkpoint(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uid: &str,
    ) -> Result<Option<VerifiedChapterImages>> {
        let complete = self.complete_many(store, book_id, &[chapter_uid])?;
        ensure!(
            complete.len() == 1,
            "unaligned checkpoint image completeness"
        );
        Ok(complete[0].then(|| VerifiedChapterImages {
            book_id: book_id.to_owned(),
            chapter_uid: chapter_uid.to_owned(),
        }))
    }

    /// Reconcile completed image chapters without temporarily dropping their
    /// durable completion count or retaining a separate proof per book copy.
    pub(crate) fn verify_complete_book(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uids: &[&str],
    ) -> Result<VerifiedBookImages> {
        self.verify_complete_book_with_verification_budget(
            store,
            book_id,
            chapter_uids,
            self.cache.verification_work_limit(),
        )
    }

    pub(crate) fn verify_complete_book_with_verification_budget(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uids: &[&str],
        maximum_work_bytes: u64,
    ) -> Result<VerifiedBookImages> {
        #[cfg(test)]
        let limits = bulk_verification_tests::configured_limits().unwrap_or_default();
        #[cfg(not(test))]
        let limits = BulkVerificationLimits::default();
        let mut metadata = BulkMetadataBudget {
            used: 0,
            maximum: limits.metadata_bytes.min(MAX_PREPARED_PACKET_BYTES),
        };
        metadata.reserve(std::mem::size_of::<VerifiedBookImages>())?;
        metadata.reserve(book_id.len())?;
        if chapter_uids.len() > limits.chapters.min(MAX_BULK_CHAPTERS) {
            return Err(BulkVerificationLimitExceeded.into());
        }
        for uid in chapter_uids {
            metadata.reserve(
                uid.len()
                    .checked_add(
                        std::mem::size_of::<String>() + 4 * std::mem::size_of::<usize>() + 1,
                    )
                    .ok_or(BulkVerificationLimitExceeded)?,
            )?;
        }
        // The existing operation pre-admits all manifest, body and PNG work
        // together. It releases that workset before these compact UID copies.
        let complete = self.complete_many_with_verification_budget(
            store,
            book_id,
            chapter_uids,
            maximum_work_bytes,
        )?;
        ensure!(
            complete.len() == chapter_uids.len(),
            "unaligned batch image proof"
        );
        let chapter_uids = chapter_uids
            .iter()
            .zip(complete)
            .filter_map(|(uid, complete)| complete.then(|| (*uid).to_owned()))
            .collect();
        Ok(VerifiedBookImages {
            book_id: book_id.to_owned(),
            chapter_uids,
            retained_metadata_bytes: metadata.used,
        })
    }

    pub fn complete_many_with_verification_budget(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uids: &[&str],
        maximum_work_bytes: u64,
    ) -> Result<Vec<bool>> {
        #[cfg(test)]
        let limits = bulk_verification_tests::configured_limits().unwrap_or_default();
        #[cfg(not(test))]
        let limits = BulkVerificationLimits::default();
        let mut scope =
            VerificationScope::new(maximum_work_bytes.min(self.cache.verification_work_limit()));
        #[cfg(test)]
        self.bulk_stats.set(BulkVerificationStats {
            scope_count: 1,
            ..Default::default()
        });
        let result =
            self.complete_many_using_verification(store, book_id, chapter_uids, limits, &mut scope);
        self.record_verification(&scope);
        #[cfg(test)]
        self.observe_bulk(|stats| {
            stats.hashes = scope.stats().hashes;
            stats.read_bytes = scope.stats().read_bytes;
            stats.charged_bytes = scope.stats().charged_bytes;
        });
        if result.is_err() {
            self.reconcile_required.set(true);
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn bulk_verification_stats(&self) -> BulkVerificationStats {
        self.bulk_stats.get()
    }

    #[cfg(test)]
    fn observe_bulk(&self, observe: impl FnOnce(&mut BulkVerificationStats)) {
        let mut stats = self.bulk_stats.get();
        observe(&mut stats);
        self.bulk_stats.set(stats);
    }

    fn complete_many_using_verification(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uids: &[&str],
        limits: BulkVerificationLimits,
        scope: &mut VerificationScope,
    ) -> Result<Vec<bool>> {
        let limits = BulkVerificationLimits {
            chapters: limits.chapters.min(MAX_BULK_CHAPTERS),
            metadata_bytes: limits.metadata_bytes.min(MAX_PREPARED_PACKET_BYTES),
        };
        ensure!(
            !book_id.is_empty()
                && book_id.len() <= MAX_SOURCE_BYTES
                && !book_id.chars().any(char::is_control),
            "invalid bulk image book identity"
        );
        if chapter_uids.len() > limits.chapters {
            return Err(BulkVerificationLimitExceeded.into());
        }
        if chapter_uids.is_empty() {
            return Ok(Vec::new());
        }
        let mut metadata = BulkMetadataBudget {
            used: 0,
            maximum: limits.metadata_bytes,
        };
        metadata.reserve(std::mem::size_of::<BulkMetadataBudget>())?;
        for uid in chapter_uids {
            ensure!(
                !uid.is_empty()
                    && uid.len() <= MAX_SOURCE_BYTES
                    && !uid.chars().any(char::is_control),
                "invalid bulk image chapter identity"
            );
            let slot = uid
                .len()
                .checked_add(book_id.len())
                .and_then(|bytes| bytes.checked_mul(6))
                .and_then(|bytes| {
                    bytes.checked_add(
                        MANIFEST_PREFIX.len()
                            + 2 * std::mem::size_of::<BulkChapter>()
                            + 2 * std::mem::size_of::<(&str, BulkChapter)>()
                            + 2 * std::mem::size_of::<bool>(),
                    )
                })
                .ok_or(BulkVerificationLimitExceeded)?;
            metadata.reserve(slot)?;
        }
        let mut chapters = BTreeMap::new();
        for uid in chapter_uids {
            if !chapters.contains_key(uid) {
                chapters.insert(
                    *uid,
                    BulkChapter {
                        key: manifest_key(book_id, uid)?,
                        ..Default::default()
                    },
                );
            }
        }
        let snapshot = store.resource_verification_snapshot()?;
        let mut manifest_work = 0u64;
        let mut manifest_temporary = 0usize;
        // This complete phase is scalar/type/octet metadata only: no JSON,
        // ignored HTML copies or body reads may precede aggregate admission.
        for chapter in chapters.values_mut() {
            chapter.encoded_bytes = snapshot.setting_encoded_bytes(&chapter.key)?;
            if let Some(bytes) = chapter.encoded_bytes {
                chapter.encoded_upper = snapshot.utf8_upper_bytes(bytes)?;
                metadata.reserve(
                    bytes
                        .checked_add(chapter.encoded_upper)
                        .ok_or(BulkVerificationLimitExceeded)?,
                )?;
                // Retained selected strings are covered by each cell's upper
                // bound. Only one JSON reader runs at a time; prepay its maximum
                // parser and escaped-string scratch before any JSON inspection.
                let temporary = chapter
                    .encoded_upper
                    .checked_mul(3)
                    .ok_or(BulkVerificationLimitExceeded)?;
                if temporary > manifest_temporary {
                    metadata.reserve(temporary - manifest_temporary)?;
                    manifest_temporary = temporary;
                }
                manifest_work = manifest_work
                    .checked_add(chapter.encoded_upper as u64)
                    .context("bulk manifest byte forecast overflow")?;
            }
        }
        scope.reserve_external_work(manifest_work)?;
        // Only admitted cells can be inspected as JSON. Recheck the same count
        // in the bounded reader before serde can reserve StoredImage vectors.
        for chapter in chapters.values_mut() {
            if let Some(bytes) = chapter.encoded_bytes {
                let Some((current, images)) = snapshot.image_manifest_metadata(
                    &chapter.key,
                    chapter.encoded_upper,
                    MAX_IMAGE_REFERENCES,
                )?
                else {
                    anyhow::bail!("bulk manifest disappeared after metadata admission");
                };
                ensure!(
                    current == bytes,
                    "bulk manifest changed after metadata admission"
                );
                chapter.images = images;
                let slots = images
                    .checked_mul(
                        2 * std::mem::size_of::<StoredImage>()
                            + 2 * std::mem::size_of::<String>()
                            + 2 * (CACHE_PREFIX.len() + 64),
                    )
                    .ok_or(BulkVerificationLimitExceeded)?;
                metadata.reserve(slots)?;
            }
        }
        #[cfg(test)]
        self.observe_bulk(|stats| {
            stats.unique_chapters = chapters.len();
            stats.metadata_bytes = metadata.used;
        });
        for chapter in chapters.values_mut() {
            if chapter.encoded_bytes.is_none() {
                continue;
            }
            chapter.manifest = parse_inventory(|| {
                snapshot.bounded_image_manifest_setting::<ManifestIndex>(
                    &chapter.key,
                    chapter.encoded_upper,
                    chapter.images,
                )
            })?;
            let manifest = chapter
                .manifest
                .as_ref()
                .context("bulk manifest is missing after admission")?;
            ensure!(
                manifest.images.len() == chapter.images,
                "bulk manifest count changed after admission"
            );
            validate_identity(
                &chapter.key,
                manifest.version,
                &manifest.book_id,
                &manifest.chapter_uid,
                &manifest.text_sha256,
                manifest.unresolved_images,
                manifest.complete,
            )?;
            validate_images(&manifest.images, manifest.complete)?;
            #[cfg(test)]
            self.observe_bulk(|stats| stats.manifest_reads += 1);
        }
        let mut body_work = 0u64;
        let mut body_temporary = 0usize;
        // This pure bound uses the snapshot's already cached encoding. It does
        // not inspect another SQL cell or introduce a new database read.
        let body_utf16 = match snapshot.utf8_upper_bytes(2)? {
            2 => false,
            3 => true,
            _ => return Err(BulkVerificationLimitExceeded.into()),
        };
        for (uid, chapter) in &mut chapters {
            let candidate = chapter
                .manifest
                .as_ref()
                .is_some_and(|manifest| manifest.complete && manifest.unresolved_images == 0);
            if !candidate {
                continue;
            }
            chapter.body_bytes = snapshot.chapter_body_bytes_by_uid(book_id, uid)?;
            if let Some(bytes) = chapter.body_bytes {
                chapter.body_upper = snapshot.utf8_upper_bytes(bytes)?;
                // Prepay all simultaneous body-read allocations before any
                // canonical body is loaded. Only one body is retained at a time.
                let temporary = bulk_body_temporary_bytes(bytes, chapter.body_upper, body_utf16)?;
                if temporary > body_temporary {
                    metadata.reserve(temporary - body_temporary)?;
                    body_temporary = temporary;
                }
                body_work = body_work
                    .checked_add(chapter.body_upper as u64)
                    .context("bulk body byte forecast overflow")?;
            }
        }
        #[cfg(test)]
        self.observe_bulk(|stats| stats.body_temporary_bytes = body_temporary);
        scope.reserve_external_work(body_work)?;
        for (uid, chapter) in &mut chapters {
            if chapter.body_bytes.is_none() {
                continue;
            }
            let body = snapshot.bounded_chapter_body_by_uid(book_id, uid, chapter.body_upper)?;
            let body = body.context("bulk body disappeared after metadata admission")?;
            ensure!(
                body.len() <= chapter.body_upper,
                "bulk body changed beyond its read allowance"
            );
            chapter.body_matches =
                digest(body.as_bytes()) == chapter.manifest.as_ref().unwrap().text_sha256;
            #[cfg(test)]
            self.observe_bulk(|stats| {
                stats.body_reads += 1;
                stats.body_bytes += body.len();
            });
        }
        let mut keys = BTreeSet::new();
        for chapter in chapters.values().filter(|chapter| chapter.body_matches) {
            for image in &chapter.manifest.as_ref().unwrap().images {
                if let Some(key) = checked_image_key(image)? {
                    keys.insert(key.to_owned());
                }
            }
        }
        let keys = keys.into_iter().collect::<Vec<_>>();
        if keys.is_empty() {
            #[cfg(test)]
            self.observe_bulk(|stats| stats.metadata_bytes = metadata.used);
            return Ok(chapter_uids
                .iter()
                .map(|uid| chapters[uid].body_matches)
                .collect());
        }
        let inspected = self
            .cache
            .inspect_keys_prepaid(&keys, scope, metadata.remaining())?;
        metadata.reserve(inspected.metadata_bytes())?;
        let mut complete = BTreeMap::new();
        for (uid, chapter) in &chapters {
            let mut ready = chapter.body_matches;
            if ready {
                for image in &chapter.manifest.as_ref().unwrap().images {
                    let key = checked_image_key(image)?
                        .context("complete bulk image has no cache identity")?;
                    ready &= inspected.current_path(&self.cache, key)?.is_some();
                    #[cfg(test)]
                    self.observe_bulk(|stats| stats.lookups += 1);
                }
            }
            complete.insert(*uid, ready);
        }
        // Every source/body/payload and current identity passed before the
        // original corrupt-entry retirement path can perform its first write.
        inspected.finish(&self.cache, false)?;
        if inspected.has_invalid_entries() {
            self.reconcile_required.set(true);
        }
        #[cfg(test)]
        self.observe_bulk(|stats| {
            stats.metadata_bytes = metadata.used;
            stats.unique_keys = keys.len();
        });
        Ok(chapter_uids.iter().map(|uid| complete[uid]).collect())
    }

    /// Include manifests for historical UIDs that no longer appear in today's catalog.
    /// Only asset metadata and payloads are removed; personal notes are untouched.
    pub fn clear_book(&self, store: &Store, book_id: &str) -> Result<()> {
        self.reconcile_required.set(true);
        let mut budget = self.cache.inventory_budget();
        let mut scope = VerificationScope::new(self.cache.verification_work_limit());
        let mut inventory =
            self.prepare_inventory(store, &mut scope, &mut budget, true, Some(book_id), false)?;
        let keys = inventory
            .manifests
            .iter()
            .filter(|m| m.retired)
            .map(|m| m.key.clone())
            .collect::<Vec<_>>();
        ensure!(
            store.resource_revision()? == inventory.revision,
            crate::store::StoreInventoryUnavailable
        );
        store.delete_settings(&keys)?;
        let revision = store.resource_revision()?;
        ensure!(
            revision.0 == inventory.revision.0
                && inventory.revision.1.checked_add(keys.len() as u64) == Some(revision.1),
            crate::store::StoreInventoryUnavailable
        );
        inventory.revision = revision;
        let result = self.apply_inventory_recovery(store, &scope, &mut inventory);
        self.record_verification(&scope);
        if result.is_ok() {
            self.reconcile_required.set(false);
        }
        result
    }

    pub fn clear_all(&self, store: &Store) -> Result<()> {
        self.reconcile_required.set(true);
        let mut budget = self.cache.inventory_budget();
        let mut scope = VerificationScope::new(self.cache.verification_work_limit());
        let mut inventory =
            self.prepare_inventory(store, &mut scope, &mut budget, true, None, true)?;
        let keys = inventory
            .manifests
            .iter()
            .map(|m| m.key.clone())
            .collect::<Vec<_>>();
        ensure!(
            store.resource_revision()? == inventory.revision,
            crate::store::StoreInventoryUnavailable
        );
        store.delete_settings(&keys)?;
        let revision = store.resource_revision()?;
        ensure!(
            revision.0 == inventory.revision.0
                && inventory.revision.1.checked_add(keys.len() as u64) == Some(revision.1),
            crate::store::StoreInventoryUnavailable
        );
        inventory.revision = revision;
        let result = self.apply_inventory_recovery(store, &scope, &mut inventory);
        self.record_verification(&scope);
        if result.is_ok() {
            self.reconcile_required.set(false);
        }
        result
    }

    /// Reconcile missing assets, interrupted checkpoints and orphan pins before
    /// startup or the next publication after an interrupted maintenance attempt.
    pub fn recover(&self, store: &Store) -> Result<()> {
        self.recover_with_verification_budget(store, self.cache.verification_work_limit())
    }

    pub fn recover_with_verification_budget(
        &self,
        store: &Store,
        maximum_work_bytes: u64,
    ) -> Result<()> {
        let mut scope = VerificationScope::new(maximum_work_bytes);
        let result = self
            .recover_using_verification(store, &mut scope)
            .map(|_| ());
        self.record_verification(&scope);
        result
    }

    fn recover_using_verification(
        &self,
        store: &Store,
        scope: &mut VerificationScope,
    ) -> Result<BTreeSet<String>> {
        self.reconcile_required.set(true);
        let mut budget = self.cache.inventory_budget();
        let mut inventory = self.prepare_inventory(store, scope, &mut budget, true, None, false)?;
        self.apply_inventory_recovery(store, scope, &mut inventory)?;
        self.cache.validate_verification(scope)?;
        self.reconcile_required.set(false);
        Ok(Self::inventory_references(&inventory, None)
            .into_iter()
            .map(str::to_owned)
            .collect())
    }

    fn raw_chapter(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uid: &str,
    ) -> Result<Option<ChapterManifest>> {
        self.raw_chapter_with_budget(
            store,
            book_id,
            chapter_uid,
            &mut self.cache.inventory_budget(),
        )
    }

    fn raw_chapter_with_budget(
        &self,
        store: &Store,
        book_id: &str,
        chapter_uid: &str,
        budget: &mut InventoryBudget,
    ) -> Result<Option<ChapterManifest>> {
        let key = manifest_key(book_id, chapter_uid)?;
        let snapshot = store.resource_verification_snapshot()?;
        let Some(raw) = snapshot.setting_encoded_bytes(&key)? else {
            return Ok(None);
        };
        let upper = snapshot.utf8_upper_bytes(raw)?;
        budget.retain(InventoryBudget::add(
            upper,
            2 * std::mem::size_of::<ChapterManifest>(),
        )?)?;
        budget.scratch(Self::json_inventory_temporary(&snapshot, raw, upper)?)?;
        budget.work(InventoryBudget::add(
            InventoryBudget::mul(upper, 4)?,
            InventoryBudget::mul(key.len(), 32)?,
        )?)?;
        let count = parse_inventory(|| {
            snapshot.bounded_image_manifest_setting::<InventoryCount>(
                &key,
                upper,
                MAX_IMAGE_REFERENCES,
            )
        })?
        .context("image manifest disappeared after admission")?
        .images
        .0;
        budget.retain(InventoryBudget::mul(
            InventoryBudget::add(count, 4)?,
            2 * std::mem::size_of::<StoredImage>(),
        )?)?;
        let manifest = parse_inventory(|| {
            snapshot.bounded_image_manifest_setting::<InventoryManifest>(
                &key,
                upper,
                MAX_IMAGE_REFERENCES,
            )
        })?
        .map(ChapterManifest::from);
        if let Some(manifest) = &manifest {
            validate_manifest(&key, manifest)?;
        }
        Ok(manifest)
    }

    fn indices(&self, store: &Store) -> Result<Vec<(String, ManifestIndex)>> {
        let snapshot = store.resource_verification_snapshot()?;
        Ok(self
            .indices_on(&snapshot, &mut self.cache.inventory_budget())?
            .into_iter()
            .map(|m| (m.key, m.index))
            .collect())
    }

    fn cleanup_unreferenced(&self, store: &Store, scope: &VerificationScope) -> Result<()> {
        // Existing narrow callers have a shared fence. Preparation must use
        // that same scope rather than acquiring permission for a late path.
        let mut local_scope = scope.metadata_scope();
        self.cache.validate_verification(scope)?;
        let inventory = self.prepare_inventory(
            store,
            &mut local_scope,
            &mut self.cache.inventory_budget(),
            false,
            None,
            false,
        )?;
        self.cache.validate_verification(scope)?;
        self.cleanup_inventory(store, scope, &inventory, None)
    }

    fn rollback_attempts(
        &self,
        attempts: &BTreeMap<String, PinAttempt>,
        scope: &VerificationScope,
    ) -> Result<()> {
        self.cache.validate_verification(scope)?;
        let mut failure = None;
        for (key, attempt) in attempts {
            let previously_pinned = attempt.previous_pin.unwrap_or(false);
            if let Err(error) =
                self.cache
                    .set_pinned_with_verification(key, previously_pinned, scope)
            {
                if failure.is_none() {
                    failure = Some(error.context("restore chapter image pin state"));
                }
                continue;
            }
            // A repaired asset with a preexisting pin may belong to another book.
            // Preserve it even when its old payload failed integrity validation.
            if !previously_pinned && !attempt.available_before {
                if let Err(error) = self.cache.remove_with_verification(key, scope) {
                    if failure.is_none() {
                        failure = Some(error.context("remove rolled-back chapter image"));
                    }
                }
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

fn checked_image_key(image: &StoredImage) -> Result<Option<&str>> {
    match (&image.asset_id, &image.cache_key) {
        (None, None) => Ok(None),
        (Some(id), Some(key)) => {
            ensure!(
                *key == asset_cache_key(id)?,
                "chapter image cache identity is inconsistent"
            );
            validate_output_dimensions(image.width, image.height)?;
            Ok(Some(key.as_str()))
        }
        _ => anyhow::bail!("chapter image has an incomplete cache identity"),
    }
}

fn manifest_key(book_id: &str, chapter_uid: &str) -> Result<String> {
    for identity in [book_id, chapter_uid] {
        ensure!(
            !identity.is_empty()
                && identity.len() <= 4096
                && !identity.chars().any(char::is_control),
            "invalid chapter image identity"
        );
    }
    let escape = |value: &str| value.replace('%', "%25").replace(':', "%3A");
    Ok(format!(
        "{MANIFEST_PREFIX}{}:{}",
        escape(book_id),
        escape(chapter_uid)
    ))
}

fn validate_manifest(key: &str, manifest: &ChapterManifest) -> Result<()> {
    validate_identity(
        key,
        manifest.version,
        &manifest.book_id,
        &manifest.chapter_uid,
        &manifest.text_sha256,
        manifest.unresolved_images,
        manifest.complete,
    )?;
    ensure!(
        manifest.html.len() <= MAX_HTML_BYTES,
        "chapter image manifest exceeds its HTML limit"
    );
    validate_images(&manifest.images, manifest.complete)
}

fn validate_identity(
    key: &str,
    version: u32,
    book_id: &str,
    chapter_uid: &str,
    text_sha256: &str,
    unresolved_images: u32,
    complete: bool,
) -> Result<()> {
    ensure!(version == 1, "unsupported chapter image manifest version");
    ensure!(
        key == manifest_key(book_id, chapter_uid)?,
        "chapter image manifest identity is inconsistent"
    );
    ensure!(
        valid_digest(text_sha256),
        "chapter image manifest has no canonical body digest"
    );
    ensure!(
        !complete || unresolved_images == 0,
        "unresolved chapter image cannot be complete"
    );
    Ok(())
}

fn validate_images(images: &[StoredImage], complete: bool) -> Result<()> {
    ensure!(
        images.len() <= MAX_IMAGE_REFERENCES,
        "chapter image manifest exceeds its reference limit"
    );
    let mut ids = HashSet::new();
    for image in images {
        ensure!(
            !image.id.is_empty() && ids.insert(image.id.as_str()),
            "chapter image occurrences are not uniquely identified"
        );
        match (&image.asset_id, &image.cache_key) {
            (Some(id), Some(key)) => {
                ensure!(
                    *key == asset_cache_key(id)?,
                    "chapter image identity is inconsistent"
                );
                validate_output_dimensions(image.width, image.height)?;
            }
            (None, None) => ensure!(!complete, "unacquired chapter image cannot be complete"),
            _ => anyhow::bail!("chapter image has an incomplete cache identity"),
        }
    }
    Ok(())
}

fn body_matches(store: &Store, manifest: &ChapterManifest) -> Result<bool> {
    body_matches_parts(
        store,
        &manifest.book_id,
        &manifest.chapter_uid,
        &manifest.text_sha256,
    )
}

fn body_matches_parts(
    store: &Store,
    book_id: &str,
    chapter_uid: &str,
    text_sha256: &str,
) -> Result<bool> {
    Ok(store
        .chapter_body_by_uid(book_id, chapter_uid)?
        .is_some_and(|body| digest(body.as_bytes()) == text_sha256))
}

fn validate_asset(asset: &ImageAsset) -> Result<()> {
    ensure!(
        valid_digest(&asset.id) && digest(&asset.bytes) == asset.id,
        "acquired chapter image digest mismatch"
    );
    ensure!(
        !asset.bytes.is_empty() && asset.bytes.len() <= MAX_IMAGE_BYTES,
        "acquired chapter image exceeds its size limit"
    );
    ensure!(
        matches!(
            asset.media_type.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        ),
        "unsupported chapter image media type"
    );
    Ok(())
}

fn prepare_asset(asset: &ImageAsset) -> Result<PreparedAsset> {
    let mut reader = image::ImageReader::new(Cursor::new(&asset.bytes))
        .with_guessed_format()
        .context("identify chapter image")?;
    ensure!(
        matches!(
            reader.format(),
            Some(
                image::ImageFormat::Png
                    | image::ImageFormat::Jpeg
                    | image::ImageFormat::Gif
                    | image::ImageFormat::WebP
            )
        ),
        "unsupported chapter image format"
    );
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits.clone());
    let (width, height) = reader
        .into_dimensions()
        .context("read chapter image dimensions")?;
    ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_IMAGE_PIXELS,
        "chapter image exceeds the 16 megapixel limit"
    );
    let mut reader = image::ImageReader::new(Cursor::new(&asset.bytes)).with_guessed_format()?;
    reader.limits(limits);
    let decoded = reader.decode().context("decode chapter image")?;
    let rendered = if width <= OUTPUT_WIDTH && height <= OUTPUT_HEIGHT {
        decoded
    } else {
        decoded.thumbnail(OUTPUT_WIDTH, OUTPUT_HEIGHT)
    };
    let mut rgba = rendered.into_rgba8();
    for pixel in rgba.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = ((u32::from(*channel) * alpha
                + u32::from(OUTPUT_ALPHA_BACKGROUND) * (255 - alpha)
                + 127)
                / 255) as u8;
        }
        pixel[3] = 255;
    }
    // Display processing belongs to a later, geometry-bound presentation.
    // Keeping Gray8 here prevents a software or Off choice from destroying
    // source luminance needed by a subsequent hardware-Y4 presentation.
    let grayscale = image::DynamicImage::ImageRgba8(rgba).into_luma8();
    let (width, height) = grayscale.dimensions();
    validate_output_dimensions(width, height)?;
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(grayscale)
        .write_to(&mut encoded, image::ImageFormat::Png)
        .context("encode grayscale chapter image")?;
    Ok(PreparedAsset {
        bytes: encoded.into_inner(),
        width,
        height,
    })
}

fn validate_output_dimensions(width: u32, height: u32) -> Result<()> {
    ensure!(
        width > 0 && height > 0 && width <= OUTPUT_WIDTH && height <= OUTPUT_HEIGHT,
        "prepared chapter image has invalid geometry"
    );
    Ok(())
}

fn asset_cache_key(id: &str) -> Result<String> {
    ensure!(valid_digest(id), "invalid chapter image digest");
    Ok(format!("{CACHE_PREFIX}{id}"))
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn add_source(
    sources: &mut BTreeMap<String, BTreeSet<String>>,
    source: &str,
    id: &str,
) -> Result<()> {
    ensure!(
        !source.is_empty() && source.len() <= 4096 && !source.chars().any(char::is_control),
        "invalid acquired image source identity"
    );
    sources
        .entry(source.to_owned())
        .or_default()
        .insert(id.to_owned());
    Ok(())
}

// Prepared sources include 4,608 archive/remote aliases and up to 512
// source-to-asset binding keys; the acquisition plan alone is not the index.
const MAX_DOWNLOADED_SOURCE_KEYS: usize = 4096 + 512 + 512;
const MAX_DOWNLOADED_SOURCE_BYTES: usize = 20 * 1024 * 1024;
const MAX_DOWNLOADED_SOURCE_WORK: usize = 128 * 1024 * 1024;

#[derive(Clone, Copy)]
struct DownloadedSourceLimits {
    keys: usize,
    associations: usize,
    source_bytes: usize,
    occurrences: usize,
    retained_bytes: usize,
    work_bytes: usize,
}

impl Default for DownloadedSourceLimits {
    fn default() -> Self {
        Self {
            keys: MAX_DOWNLOADED_SOURCE_KEYS,
            associations: MAX_DOWNLOADED_SOURCE_KEYS,
            source_bytes: MAX_DOWNLOADED_SOURCE_BYTES,
            occurrences: MAX_IMAGE_REFERENCES,
            retained_bytes: MAX_PREPARED_PACKET_BYTES,
            work_bytes: MAX_DOWNLOADED_SOURCE_WORK,
        }
    }
}

impl DownloadedSourceLimits {
    fn constrained(self) -> Self {
        let ceiling = Self::default();
        Self {
            keys: self.keys.min(ceiling.keys),
            associations: self.associations.min(ceiling.associations),
            source_bytes: self.source_bytes.min(ceiling.source_bytes),
            occurrences: self.occurrences.min(ceiling.occurrences),
            retained_bytes: self.retained_bytes.min(ceiling.retained_bytes),
            work_bytes: self.work_bytes.min(ceiling.work_bytes),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DownloadedSourceUsage {
    keys: usize,
    associations: usize,
    source_bytes: usize,
    occurrences: usize,
    retained_bytes: usize,
    work_bytes: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("downloaded chapter image sources exceed their metadata or work allowance")]
struct DownloadedSourceLimitExceeded;

#[derive(Clone, Copy, Default)]
struct DownloadedSourceCandidate<'a> {
    id: Option<&'a str>,
    ambiguous: bool,
}

impl<'a> DownloadedSourceCandidate<'a> {
    fn new(ids: &'a BTreeSet<String>) -> Self {
        Self {
            id: ids.first().map(String::as_str),
            ambiguous: ids.len() > 1,
        }
    }

    fn merge(&mut self, other: Self) {
        self.ambiguous |= other.ambiguous;
        match (self.id, other.id) {
            (Some(previous), Some(next)) if previous != next => {
                self.ambiguous = true;
                self.id = Some(previous.min(next));
            }
            (None, Some(next)) => self.id = Some(next),
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
enum DownloadedSourceResolution<'a> {
    Missing,
    Exact(DownloadedSourceCandidate<'a>),
    Normalized(DownloadedSourceCandidate<'a>),
}

/// One commit's requested-source table borrows both the immutable packet and
/// occurrences. Temporary alias indices are dropped before any cache operation.
/// They never copy image IDs or retain an unrequested ambiguity as an error.
struct DownloadedSourceIndex<'a> {
    entries: HashMap<&'a str, DownloadedSourceResolution<'a>>,
}

fn downloaded_source_add(used: &mut usize, amount: usize, maximum: usize) -> Result<()> {
    *used = used
        .checked_add(amount)
        .filter(|used| *used <= maximum)
        .ok_or(DownloadedSourceLimitExceeded)?;
    Ok(())
}

fn downloaded_normalized_capacity(bytes: usize) -> Result<usize> {
    bytes
        .checked_add("https:".len())
        .and_then(|bytes| bytes.checked_mul(3))
        .ok_or_else(|| DownloadedSourceLimitExceeded.into())
}

fn downloaded_normalization_work(bytes: usize) -> Result<usize> {
    // Logical copy, URL decoding/encoding and hash passes. This fee is a
    // project policy, not a bound on allocator RSS, collision cost or time.
    bytes
        .checked_add("https:".len())
        .and_then(|bytes| bytes.checked_mul(12))
        .ok_or_else(|| DownloadedSourceLimitExceeded.into())
}

fn requested_downloaded_source(anchor: &AnchoredImage, scalar_count: u32) -> bool {
    !anchor.id.is_empty()
        && anchor.source.len() <= MAX_SOURCE_BYTES
        && !anchor.source.chars().any(char::is_control)
        && anchor.anchor.is_none_or(|anchor| anchor <= scalar_count)
}

impl<'a> DownloadedSourceIndex<'a> {
    fn new(
        sources: &'a BTreeMap<String, BTreeSet<String>>,
        anchors: &'a [AnchoredImage],
        scalar_count: u32,
    ) -> Result<Self> {
        #[cfg(test)]
        let limits = downloaded_source_tests::configured_limits().unwrap_or_default();
        #[cfg(not(test))]
        let limits = DownloadedSourceLimits::default();
        Self::with_limits(sources, anchors, scalar_count, limits)
    }

    fn forecast(
        sources: &BTreeMap<String, BTreeSet<String>>,
        anchors: &[AnchoredImage],
        scalar_count: u32,
        limits: DownloadedSourceLimits,
    ) -> Result<DownloadedSourceUsage> {
        let limits = limits.constrained();
        if sources.len() > limits.keys || anchors.len() > limits.occurrences {
            return Err(DownloadedSourceLimitExceeded.into());
        }
        let exact_slot = std::mem::size_of::<(&str, DownloadedSourceCandidate<'_>)>();
        let normalized_slot = std::mem::size_of::<(String, DownloadedSourceCandidate<'_>)>();
        let request_slot = std::mem::size_of::<(&str, DownloadedSourceResolution<'_>)>();
        // Include map minima and one normalizer's largest URL/UTF scratch;
        // requested normalized strings are ephemeral, never retained together.
        let fixed = std::mem::size_of::<Self>()
            + 2 * std::mem::size_of::<HashMap<&str, DownloadedSourceCandidate<'_>>>()
            + 4 * (exact_slot + normalized_slot + request_slot)
            + 4 * downloaded_normalized_capacity(MAX_SOURCE_BYTES)?;
        let mut usage = DownloadedSourceUsage {
            keys: sources.len(),
            occurrences: anchors.len(),
            ..Default::default()
        };
        downloaded_source_add(&mut usage.retained_bytes, fixed, limits.retained_bytes)?;
        downloaded_source_add(
            &mut usage.retained_bytes,
            anchors
                .len()
                .checked_mul(2 * request_slot)
                .ok_or(DownloadedSourceLimitExceeded)?,
            limits.retained_bytes,
        )?;
        for (source, ids) in sources {
            if source.len() > MAX_SOURCE_BYTES {
                return Err(DownloadedSourceLimitExceeded.into());
            }
            downloaded_source_add(&mut usage.associations, ids.len(), limits.associations)?;
            // An empty set remains an exact logical ambiguity; it still owns
            // a raw key and cannot bypass the key/byte/metadata admission.
            let names = source
                .len()
                .checked_mul(ids.len().max(1))
                .ok_or(DownloadedSourceLimitExceeded)?;
            downloaded_source_add(&mut usage.source_bytes, names, limits.source_bytes)?;
            let metadata = downloaded_normalized_capacity(source.len())?
                .checked_add(2 * (exact_slot + normalized_slot))
                .and_then(|bytes| {
                    ids.len()
                        .checked_mul(2 * std::mem::size_of::<&str>())
                        .and_then(|ids| bytes.checked_add(ids))
                })
                .ok_or(DownloadedSourceLimitExceeded)?;
            downloaded_source_add(&mut usage.retained_bytes, metadata, limits.retained_bytes)?;
            let work = downloaded_normalization_work(source.len())?
                .checked_add(
                    ids.len()
                        .checked_mul(64)
                        .ok_or(DownloadedSourceLimitExceeded)?,
                )
                .ok_or(DownloadedSourceLimitExceeded)?;
            downloaded_source_add(&mut usage.work_bytes, work, limits.work_bytes)?;
        }
        for anchor in anchors
            .iter()
            .filter(|anchor| requested_downloaded_source(anchor, scalar_count))
        {
            // Duplicate occurrences prepay too, although their unique request
            // is normalized only once and every later lookup is borrowed.
            downloaded_source_add(
                &mut usage.work_bytes,
                downloaded_normalization_work(anchor.source.len())?,
                limits.work_bytes,
            )?;
        }
        Ok(usage)
    }

    fn with_limits(
        sources: &'a BTreeMap<String, BTreeSet<String>>,
        anchors: &'a [AnchoredImage],
        scalar_count: u32,
        limits: DownloadedSourceLimits,
    ) -> Result<Self> {
        let usage = Self::forecast(sources, anchors, scalar_count, limits)?;
        #[cfg(test)]
        downloaded_source_tests::observe_admission(usage);
        #[cfg(not(test))]
        let _ = usage;
        let mut exact = HashMap::with_capacity(sources.len());
        let mut normalized =
            HashMap::<String, DownloadedSourceCandidate<'a>>::with_capacity(sources.len());
        for (source, ids) in sources {
            let candidate = DownloadedSourceCandidate::new(ids);
            exact.insert(source.as_str(), candidate);
            let key = normalized_source(source);
            #[cfg(test)]
            downloaded_source_tests::observe_normalization(
                false,
                key.as_ref().map_or(0, String::len),
            );
            if let Some(key) = key {
                if key.len() > downloaded_normalized_capacity(source.len())? {
                    return Err(DownloadedSourceLimitExceeded.into());
                }
                normalized
                    .entry(key)
                    .and_modify(|previous| previous.merge(candidate))
                    .or_insert(candidate);
            }
        }
        let mut entries = HashMap::with_capacity(anchors.len());
        for anchor in anchors
            .iter()
            .filter(|anchor| requested_downloaded_source(anchor, scalar_count))
        {
            let source = anchor.source.as_str();
            if entries.contains_key(source) {
                continue;
            }
            let resolution = if let Some(candidate) = exact.get(source) {
                DownloadedSourceResolution::Exact(*candidate)
            } else {
                let capacity = downloaded_normalized_capacity(source.len())?;
                let key = normalized_source(source);
                #[cfg(test)]
                downloaded_source_tests::observe_normalization(
                    true,
                    key.as_ref().map_or(0, String::len),
                );
                if key.as_ref().is_some_and(|key| key.len() > capacity) {
                    return Err(DownloadedSourceLimitExceeded.into());
                }
                key.and_then(|key| normalized.get(&key).copied())
                    .map(DownloadedSourceResolution::Normalized)
                    .unwrap_or(DownloadedSourceResolution::Missing)
            };
            entries.insert(source, resolution);
        }
        #[cfg(test)]
        downloaded_source_tests::observe_projection(entries.len());
        Ok(Self { entries })
    }

    fn id(&self, source: &str) -> Result<Option<&'a str>> {
        #[cfg(test)]
        downloaded_source_tests::observe_lookup();
        match self
            .entries
            .get(source)
            .copied()
            .unwrap_or(DownloadedSourceResolution::Missing)
        {
            DownloadedSourceResolution::Missing => Ok(None),
            DownloadedSourceResolution::Exact(candidate) => {
                ensure!(
                    !candidate.ambiguous && candidate.id.is_some(),
                    "ambiguous chapter image source"
                );
                Ok(candidate.id)
            }
            DownloadedSourceResolution::Normalized(candidate) => {
                ensure!(
                    !candidate.ambiguous,
                    "ambiguous normalized chapter image source"
                );
                Ok(candidate.id)
            }
        }
    }
}

/// Compatibility for existing pure source assertions only. Production always
/// builds one bounded table for the complete commit and has no lookup wrapper.
#[cfg(test)]
fn resolve_source(
    sources: &BTreeMap<String, BTreeSet<String>>,
    source: &str,
) -> Result<Option<String>> {
    let anchors = [AnchoredImage {
        id: "source-assertion".into(),
        source: source.into(),
        ..Default::default()
    }];
    let index = DownloadedSourceIndex::new(sources, &anchors, u32::MAX)?;
    Ok(index.id(source)?.map(str::to_owned))
}

#[derive(Clone, Copy)]
struct SourceIndexLimits {
    occurrences: usize,
    source_bytes: usize,
    retained_bytes: usize,
}

impl Default for SourceIndexLimits {
    fn default() -> Self {
        Self {
            occurrences: MAX_IMAGE_REFERENCES,
            source_bytes: 2 * MAX_IMAGE_REFERENCES * MAX_SOURCE_BYTES,
            retained_bytes: MAX_PREPARED_PACKET_BYTES,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("previous chapter image sources exceed their metadata allowance")]
struct SourceMatchLimitExceeded;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SourceMatchStats {
    old_occurrences: usize,
    old_unique_sources: usize,
    old_normalizations: usize,
    new_unique_sources: usize,
    new_normalizations: usize,
    source_bytes: usize,
    retained_bytes: usize,
    lookups: usize,
}

/// Summaries preserve the first matching occurrence and defer ambiguity errors.
/// Missing asset IDs do not create a distinct digest, matching the old lookup.
#[derive(Clone, Copy)]
struct PreviousImageCandidate<'a> {
    first: &'a StoredImage,
    ordinal: usize,
    asset_id: Option<&'a str>,
    ambiguous: bool,
}

impl<'a> PreviousImageCandidate<'a> {
    fn new(first: &'a StoredImage, ordinal: usize) -> Self {
        Self {
            first,
            ordinal,
            asset_id: first.asset_id.as_deref(),
            ambiguous: false,
        }
    }

    fn merge(&mut self, other: Self) {
        if other.ordinal < self.ordinal {
            self.first = other.first;
            self.ordinal = other.ordinal;
        }
        self.ambiguous |= other.ambiguous;
        match (self.asset_id, other.asset_id) {
            (Some(previous), Some(next)) if previous != next => self.ambiguous = true,
            (None, Some(next)) => self.asset_id = Some(next),
            _ => {}
        }
    }
}

/// One commit's source index owns only small metadata and borrows both inputs.
/// Exact/normalized indices are built once, then dropped after projecting the
/// unique requested sources. Probe and materialization share that projection.
struct PreviousImageIndex<'a> {
    entries: HashMap<&'a str, Option<PreviousImageCandidate<'a>>>,
    #[cfg(test)]
    stats: SourceMatchStats,
}

impl<'a> PreviousImageIndex<'a> {
    fn new(manifest: &'a ChapterManifest, anchors: &'a [AnchoredImage]) -> Result<Self> {
        Self::with_limits(manifest, anchors, SourceIndexLimits::default())
    }

    fn with_limits(
        manifest: &'a ChapterManifest,
        anchors: &'a [AnchoredImage],
        limits: SourceIndexLimits,
    ) -> Result<Self> {
        if manifest.images.len() > limits.occurrences || anchors.len() > limits.occurrences {
            return Err(SourceMatchLimitExceeded.into());
        }
        let eligible = |source: &str| {
            source.len() <= MAX_SOURCE_BYTES && !source.chars().any(char::is_control)
        };
        let mut source_bytes = 0usize;
        let mut retained_bytes = std::mem::size_of::<Self>()
            + 2 * std::mem::size_of::<HashMap<&str, PreviousImageCandidate<'_>>>();
        // Price map spare slots and a conservative percent-encoded URL key
        // before normalization or any index insertion. Raw source strings and
        // PNG payloads remain borrowed. Duplicate occurrences do not refund fees.
        for (source, fixed) in manifest
            .images
            .iter()
            .filter(|image| image.cache_key.is_some())
            .map(|image| {
                (
                    image.source.as_str(),
                    2 * std::mem::size_of::<(&str, PreviousImageCandidate<'_>)>()
                        + 2 * std::mem::size_of::<(String, PreviousImageCandidate<'_>)>(),
                )
            })
            .chain(
                anchors
                    .iter()
                    .filter(|anchor| eligible(&anchor.source))
                    .map(|anchor| {
                        (
                            anchor.source.as_str(),
                            2 * std::mem::size_of::<(&str, Option<PreviousImageCandidate<'_>>)>(),
                        )
                    }),
            )
        {
            if source.len() > MAX_SOURCE_BYTES {
                return Err(SourceMatchLimitExceeded.into());
            }
            source_bytes = source_bytes
                .checked_add(source.len())
                .filter(|bytes| *bytes <= limits.source_bytes)
                .ok_or(SourceMatchLimitExceeded)?;
            let fee = source
                .len()
                .checked_add("https:".len())
                .and_then(|bytes| bytes.checked_mul(3))
                .and_then(|bytes| bytes.checked_add(fixed))
                .ok_or(SourceMatchLimitExceeded)?;
            retained_bytes = retained_bytes
                .checked_add(fee)
                .filter(|bytes| *bytes <= limits.retained_bytes)
                .ok_or(SourceMatchLimitExceeded)?;
        }
        if retained_bytes > limits.retained_bytes {
            return Err(SourceMatchLimitExceeded.into());
        }

        let mut exact = HashMap::<&str, PreviousImageCandidate<'a>>::new();
        #[cfg(test)]
        let mut old_occurrences = 0usize;
        for (ordinal, image) in manifest.images.iter().enumerate() {
            if image.cache_key.is_none() {
                continue;
            }
            #[cfg(test)]
            {
                old_occurrences += 1;
            }
            let candidate = PreviousImageCandidate::new(image, ordinal);
            exact
                .entry(image.source.as_str())
                .and_modify(|previous| previous.merge(candidate))
                .or_insert(candidate);
        }
        let mut normalized = HashMap::<String, PreviousImageCandidate<'a>>::new();
        #[cfg(test)]
        let mut old_normalizations = 0usize;
        for (source, candidate) in &exact {
            #[cfg(test)]
            {
                old_normalizations += 1;
            }
            if let Some(source) = normalized_source(source) {
                normalized
                    .entry(source)
                    .and_modify(|previous| previous.merge(*candidate))
                    .or_insert(*candidate);
            }
        }
        let mut entries = HashMap::new();
        #[cfg(test)]
        let mut new_normalizations = 0usize;
        for anchor in anchors.iter().filter(|anchor| eligible(&anchor.source)) {
            let source = anchor.source.as_str();
            if entries.contains_key(source) {
                continue;
            }
            let candidate = if let Some(candidate) = exact.get(source) {
                Some(*candidate)
            } else {
                #[cfg(test)]
                {
                    new_normalizations += 1;
                }
                normalized_source(source).and_then(|source| normalized.get(&source).copied())
            };
            entries.insert(source, candidate);
        }
        Ok(Self {
            #[cfg(test)]
            stats: SourceMatchStats {
                old_occurrences,
                old_unique_sources: exact.len(),
                old_normalizations,
                new_unique_sources: entries.len(),
                new_normalizations,
                source_bytes,
                retained_bytes,
                lookups: 0,
            },
            entries,
        })
    }

    fn image(&self, source: &str) -> Result<Option<&'a StoredImage>> {
        let Some(candidate) = self.entries.get(source).copied().flatten() else {
            return Ok(None);
        };
        ensure!(
            !candidate.ambiguous,
            "ambiguous previously cached chapter image"
        );
        Ok(Some(candidate.first))
    }
}

/// Preserve host, query and full directory identity. A shared basename is never enough.
fn normalized_source(source: &str) -> Option<String> {
    if source.is_empty() || source.chars().any(char::is_control) || source.contains('\\') {
        return None;
    }
    let source = source.split('#').next()?;
    let absolute = if source.starts_with("//") {
        format!("https:{source}")
    } else {
        source.to_owned()
    };
    if let Ok(mut url) = Url::parse(&absolute) {
        if !matches!(url.scheme(), "https" | "http")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return None;
        }
        url.set_fragment(None);
        return Some(url.to_string());
    }
    if absolute.contains(':') || absolute.starts_with('/') {
        return None;
    }
    let (path, query) = absolute
        .split_once('?')
        .map_or((absolute.as_str(), ""), |(path, query)| (path, query));
    let path = path.split('#').next()?;
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." if segments.last().is_some_and(|segment| *segment != "..") => {
                segments.pop();
            }
            _ => segments.push(segment),
        }
    }
    if segments.is_empty() {
        return None;
    }
    Some(if query.is_empty() {
        segments.join("/")
    } else {
        format!(
            "{}?{}",
            segments.join("/"),
            query.split('#').next().unwrap_or_default()
        )
    })
}

fn unresolved_images_in_document(document: &Html) -> Result<u32> {
    // Share the Source guard's operation-local admission and preserve its quota
    // error before an unresolved count can reach publication.
    let svg = crate::source::images::count_svg_images(document)?;
    let images = Selector::parse("img").expect("constant selector");
    let missing = document
        .select(&images)
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
    Ok(u32::try_from(svg.saturating_add(missing)).unwrap_or(u32::MAX))
}

#[cfg(test)]
#[path = "book_assets_verification_tests.rs"]
mod verification_tests;

#[cfg(test)]
#[path = "book_assets_source_match_tests.rs"]
mod source_match_tests;

#[cfg(test)]
#[path = "book_assets_downloaded_source_tests.rs"]
mod downloaded_source_tests;

#[cfg(test)]
#[path = "book_assets_bulk_verification_tests.rs"]
mod bulk_verification_tests;

#[cfg(test)]
#[path = "book_assets_inventory_tests.rs"]
mod inventory_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Annotation, Book, Chapter};

    pub(super) const TEXT: &str = "Before\nAfter";
    const HTML: &str = "<p>Before</p><img src='images/a.png' alt='Fixture figure'><p>After</p>";

    pub(super) fn setup(budget: u64) -> Result<(tempfile::TempDir, Store, BookAssetStore)> {
        let directory = tempfile::tempdir()?;
        let store = Store::open(directory.path().join("metadata.sqlite3"))?;
        store.upsert_book(&Book::new("book", "Fixture book"))?;
        store.upsert_chapter(&Chapter {
            id: "uid".into(),
            book_id: "book".into(),
            index: 0,
            title: "Fixture chapter".into(),
            body: Some(TEXT.into()),
            ..Chapter::default()
        })?;
        let assets = BookAssetStore::open(directory.path().join("images"), budget)?;
        assets.recover(&store)?;
        Ok((directory, store, assets))
    }

    pub(super) fn asset(source: &str, seed: u8) -> Result<ImageAsset> {
        let raster = image::RgbImage::from_fn(8, 6, |x, y| {
            image::Rgb([
                seed.wrapping_add((x * 23) as u8),
                seed.wrapping_add((y * 31) as u8),
                113,
            ])
        });
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(raster).write_to(&mut encoded, image::ImageFormat::Png)?;
        let bytes = encoded.into_inner();
        Ok(ImageAsset {
            id: digest(&bytes),
            media_type: "image/png".into(),
            extension: "png".into(),
            source_names: vec![source.into()],
            bytes,
        })
    }

    fn gray8_ramp_asset() -> Result<ImageAsset> {
        let raster = image::GrayImage::from_fn(256, 1, |x, _| image::Luma([x as u8]));
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(raster).write_to(&mut encoded, image::ImageFormat::Png)?;
        let bytes = encoded.into_inner();
        Ok(ImageAsset {
            id: digest(&bytes),
            media_type: "image/png".into(),
            extension: "png".into(),
            source_names: vec!["images/a.png".into()],
            bytes,
        })
    }

    fn sixteen_level_ramp_png() -> Result<Vec<u8>> {
        let raster = image::GrayImage::from_fn(256, 1, |x, _| {
            image::Luma([(((x + 8) / 17).min(15) * 17) as u8])
        });
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(raster).write_to(&mut encoded, image::ImageFormat::Png)?;
        Ok(encoded.into_inner())
    }

    fn packet(assets: Vec<ImageAsset>) -> DownloadedChapterImages {
        let source_to_asset = assets
            .iter()
            .flat_map(|asset| {
                asset
                    .source_names
                    .iter()
                    .map(|source| (source.clone(), asset.id.clone()))
            })
            .collect();
        DownloadedChapterImages {
            assets,
            source_to_asset,
        }
    }

    fn interrupted_pin_rollback()
    -> Result<(tempfile::TempDir, Store, BookAssetStore, String, String)> {
        let (directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let shared = asset("images/a.png", 7)?;
        let committed = assets.commit_chapter(
            &store,
            "book",
            "uid",
            HTML,
            TEXT,
            Some(&packet(vec![shared.clone()])),
        )?;
        let shared_key = committed.images[0].cache_key.clone().unwrap();
        store.upsert_book(&Book::new("other-book", "Shared fixture book"))?;
        store.upsert_chapter(&Chapter {
            id: "other-uid".into(),
            book_id: "other-book".into(),
            index: 0,
            body: Some(TEXT.into()),
            ..Chapter::default()
        })?;
        assets.commit_chapter(
            &store,
            "other-book",
            "other-uid",
            HTML,
            TEXT,
            Some(&packet(vec![shared.clone()])),
        )?;
        store.upsert_chapter(&Chapter {
            id: "failed-uid".into(),
            book_id: "book".into(),
            index: 1,
            body: Some(TEXT.into()),
            ..Chapter::default()
        })?;
        let orphan = asset("images/b.png", 51)?;
        let orphan_key = asset_cache_key(&orphan.id)?;
        let failed_key = manifest_key("book", "failed-uid")?;
        let metadata_fault = rusqlite::Connection::open(directory.path().join("metadata.sqlite3"))?;
        metadata_fault.execute_batch(&format!(
            "CREATE TRIGGER reject_failed_manifest BEFORE INSERT ON settings
             WHEN NEW.key = '{failed_key}'
             BEGIN SELECT RAISE(ABORT, 'fixture manifest save failure'); END;"
        ))?;
        let pin_fault =
            rusqlite::Connection::open(assets.cache.root().join("cache-index.sqlite3"))?;
        pin_fault.execute_batch(&format!(
            "CREATE TRIGGER reject_orphan_unpin BEFORE UPDATE OF pinned ON cache_entries
             WHEN OLD.key = '{orphan_key}' AND NEW.pinned = 0
             BEGIN SELECT RAISE(ABORT, 'fixture unpin failure'); END;"
        ))?;
        let html = "<p>Before</p><img src='images/a.png'><img src='images/b.png'><p>After</p>";
        let error = assets
            .commit_chapter(
                &store,
                "book",
                "failed-uid",
                html,
                TEXT,
                Some(&packet(vec![shared, orphan])),
            )
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("fixture manifest save failure"));
        assert!(message.contains("fixture unpin failure"));
        assert!(assets.reconcile_required.get());
        assert!(assets.raw_chapter(&store, "book", "failed-uid")?.is_none());
        assert_eq!(assets.cache.pinned_state(&orphan_key)?, Some(true));
        assert_eq!(assets.cache.pinned_state(&shared_key)?, Some(true));
        metadata_fault.execute_batch("DROP TRIGGER reject_failed_manifest")?;
        pin_fault.execute_batch("DROP TRIGGER reject_orphan_unpin")?;
        Ok((directory, store, assets, orphan_key, shared_key))
    }

    #[test]
    fn failed_pin_rollback_is_reconciled_before_same_instance_publishes_again() -> Result<()> {
        let (_directory, store, assets, orphan_key, shared_key) = interrupted_pin_rollback()?;
        let orphan_path = assets.cache.get_for_commit(&orphan_key)?.unwrap();
        let next = asset("images/c.png", 113)?;
        let next_key = asset_cache_key(&next.id)?;
        let publication_order =
            rusqlite::Connection::open(assets.cache.root().join("cache-index.sqlite3"))?;
        publication_order.execute_batch(&format!(
            "CREATE TRIGGER require_orphan_retirement BEFORE INSERT ON cache_entries
             WHEN NEW.key = '{next_key}'
               AND EXISTS(SELECT 1 FROM cache_entries WHERE key = '{orphan_key}')
             BEGIN SELECT RAISE(ABORT, 'orphan must retire before publication'); END;"
        ))?;
        store.upsert_chapter(&Chapter {
            id: "next-uid".into(),
            book_id: "book".into(),
            index: 2,
            body: Some(TEXT.into()),
            ..Chapter::default()
        })?;
        let html = "<p>Before</p><img src='images/c.png'><p>After</p>";
        let current = assets.commit_chapter(
            &store,
            "book",
            "next-uid",
            html,
            TEXT,
            Some(&packet(vec![next])),
        )?;
        assert!(current.complete);
        assert!(!assets.reconcile_required.get());
        assert!(assets.cache.get_for_commit(&orphan_key)?.is_none());
        assert!(!orphan_path.exists());
        assert_eq!(assets.cache.pinned_state(&shared_key)?, Some(true));
        assert_eq!(assets.cache.pinned_state(&next_key)?, Some(true));
        assert!(assets.complete(&store, "book", "uid")?);
        assert!(assets.complete(&store, "other-book", "other-uid")?);
        Ok(())
    }

    #[test]
    fn unreadable_global_manifest_evidence_blocks_new_pins_and_payloads() -> Result<()> {
        let (_directory, store, assets, orphan_key, shared_key) = interrupted_pin_rollback()?;
        let broken_key = manifest_key("unknown-book", "unknown-uid")?;
        store.set_setting(&broken_key, &"not a manifest")?;
        let next = asset("images/c.png", 113)?;
        let next_key = asset_cache_key(&next.id)?;
        let next_packet = packet(vec![next]);
        store.upsert_chapter(&Chapter {
            id: "next-uid".into(),
            book_id: "book".into(),
            index: 2,
            body: Some(TEXT.into()),
            ..Chapter::default()
        })?;
        let before = assets.cache.keys()?.into_iter().collect::<BTreeSet<_>>();
        let usage = assets.usage()?;
        let html = "<p>Before</p><img src='images/c.png'><p>After</p>";
        assert!(
            assets
                .commit_chapter(&store, "book", "next-uid", html, TEXT, Some(&next_packet),)
                .is_err()
        );
        assert!(assets.reconcile_required.get());
        assert_eq!(
            assets.cache.keys()?.into_iter().collect::<BTreeSet<_>>(),
            before
        );
        assert_eq!(assets.usage()?, usage);
        assert!(assets.cache.get_for_commit(&next_key)?.is_none());
        assert!(assets.raw_chapter(&store, "book", "next-uid")?.is_none());
        assert_eq!(assets.cache.pinned_state(&orphan_key)?, Some(true));
        assert_eq!(assets.cache.pinned_state(&shared_key)?, Some(true));
        store.delete_setting(&broken_key)?;
        assets.commit_chapter(&store, "book", "next-uid", html, TEXT, Some(&next_packet))?;
        assert!(!assets.reconcile_required.get());
        assert!(assets.cache.get_for_commit(&orphan_key)?.is_none());
        assert_eq!(assets.cache.pinned_state(&shared_key)?, Some(true));
        Ok(())
    }

    #[test]
    fn a_failed_explicit_recovery_marks_a_previously_clean_store_dirty() -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let shared = assets.commit_chapter(
            &store,
            "book",
            "uid",
            HTML,
            TEXT,
            Some(&packet(vec![asset("images/a.png", 7)?])),
        )?;
        assert!(!assets.reconcile_required.get());
        let broken_key = manifest_key("unknown-book", "unknown-uid")?;
        store.set_setting(&broken_key, &"not a manifest")?;
        assert!(assets.recover(&store).is_err());
        assert!(assets.reconcile_required.get());
        let next = asset("images/c.png", 113)?;
        let next_key = asset_cache_key(&next.id)?;
        let html = "<p>Before</p><img src='images/c.png'><p>After</p>";
        assert!(
            assets
                .commit_chapter(
                    &store,
                    "book",
                    "next-uid",
                    html,
                    TEXT,
                    Some(&packet(vec![next])),
                )
                .is_err()
        );
        assert!(assets.cache.get_for_commit(&next_key)?.is_none());
        assert_eq!(
            assets
                .cache
                .pinned_state(shared.images[0].cache_key.as_ref().unwrap())?,
            Some(true)
        );
        assert!(assets.reconcile_required.get());
        Ok(())
    }

    fn tolerated_lookup_debt_gates_publication(cached_only: bool) -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let damaged = assets.commit_chapter(
            &store,
            "book",
            "uid",
            HTML,
            TEXT,
            Some(&packet(vec![asset("images/a.png", 7)?])),
        )?;
        let shared = asset("images/shared.png", 51)?;
        let shared_html = "<p>Before</p><img src='images/shared.png'><p>After</p>";
        let mut shared_key = String::new();
        for (book_id, uid) in [("shared-book-a", "shared-a"), ("shared-book-b", "shared-b")] {
            store.upsert_book(&Book::new(book_id, "Shared fixture"))?;
            store.upsert_chapter(&Chapter {
                id: uid.into(),
                book_id: book_id.into(),
                index: 0,
                body: Some(TEXT.into()),
                ..Chapter::default()
            })?;
            let current = assets.commit_chapter(
                &store,
                book_id,
                uid,
                shared_html,
                TEXT,
                Some(&packet(vec![shared.clone()])),
            )?;
            shared_key = current.images[0].cache_key.clone().unwrap();
        }
        assert!(!assets.reconcile_required.get());
        let blocked = assets.image_path(&damaged.images[0])?.unwrap();
        std::fs::remove_file(&blocked)?;
        std::fs::create_dir(&blocked)?;
        let lookup = assets.image_path(&damaged.images[0]);
        assert!(lookup.is_err());
        // Reader rendering deliberately tolerates this error as a placeholder.
        // Maintenance must remember it even though no caller propagates it.
        assert!(lookup.ok().flatten().is_none());
        assert!(assets.reconcile_required.get());
        let index = rusqlite::Connection::open(assets.cache.root().join("cache-index.sqlite3"))?;
        let pending = || -> Result<i64> {
            Ok(
                index.query_row("SELECT COUNT(*) FROM cache_pending_cleanup", [], |row| {
                    row.get(0)
                })?,
            )
        };
        assert_eq!(pending()?, 1);
        assert_eq!(assets.cache.pinned_state(&shared_key)?, Some(true));
        let before = assets.cache.keys()?.into_iter().collect::<BTreeSet<_>>();
        let usage = assets.usage()?;
        store.upsert_chapter(&Chapter {
            id: "next-uid".into(),
            book_id: "book".into(),
            index: 1,
            body: Some(TEXT.into()),
            ..Chapter::default()
        })?;
        let image_free_html = "<p>Before</p><p>After</p>";
        let next_html = if cached_only {
            shared_html
        } else {
            image_free_html
        };
        let reused = packet(vec![shared]);
        let downloaded = cached_only.then_some(&reused);
        // Neither variant needs FileCache::put. The asset-store gate itself
        // must retry the ledger before publishing another manifest.
        assert!(
            assets
                .commit_chapter(&store, "book", "next-uid", next_html, TEXT, downloaded,)
                .is_err()
        );
        assert!(assets.reconcile_required.get());
        assert!(assets.raw_chapter(&store, "book", "next-uid")?.is_none());
        assert_eq!(
            assets.cache.keys()?.into_iter().collect::<BTreeSet<_>>(),
            before
        );
        assert_eq!(assets.usage()?, usage);
        assert_eq!(pending()?, 1);
        assert_eq!(assets.cache.pinned_state(&shared_key)?, Some(true));

        std::fs::remove_dir(&blocked)?;
        let current =
            assets.commit_chapter(&store, "book", "next-uid", next_html, TEXT, downloaded)?;
        assert!(current.complete);
        assert!(!assets.reconcile_required.get());
        assert_eq!(pending()?, 0);
        assert!(assets.usage()? < usage);
        assert!(!blocked.exists());
        assert_eq!(assets.cache.pinned_state(&shared_key)?, Some(true));
        assert!(assets.complete(&store, "shared-book-a", "shared-a")?);
        assert!(assets.complete(&store, "shared-book-b", "shared-b")?);
        assert_eq!(
            store.chapter_body_by_uid("book", "uid")?.as_deref(),
            Some(TEXT)
        );
        Ok(())
    }

    #[test]
    fn tolerated_image_lookup_cleanup_failure_blocks_image_free_publication() -> Result<()> {
        tolerated_lookup_debt_gates_publication(false)
    }

    #[test]
    fn tolerated_image_lookup_cleanup_failure_blocks_cached_only_publication() -> Result<()> {
        tolerated_lookup_debt_gates_publication(true)
    }

    #[test]
    fn tolerated_manifest_read_failure_requires_valid_evidence_before_publication() -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let original = assets.commit_chapter(
            &store,
            "book",
            "uid",
            HTML,
            TEXT,
            Some(&packet(vec![asset("images/a.png", 7)?])),
        )?;
        let shared_key = original.images[0].cache_key.as_ref().unwrap();
        assert!(!assets.reconcile_required.get());
        let key = manifest_key("book", "uid")?;
        store.set_setting(&key, &"unreadable manifest fixture")?;
        assert!(
            assets
                .chapter(&store, "book", "uid")
                .ok()
                .flatten()
                .is_none()
        );
        assert!(assets.reconcile_required.get());
        store.upsert_chapter(&Chapter {
            id: "next-uid".into(),
            book_id: "book".into(),
            index: 1,
            body: Some(TEXT.into()),
            ..Chapter::default()
        })?;
        let next = asset("images/b.png", 113)?;
        let next_key = asset_cache_key(&next.id)?;
        let next_packet = packet(vec![next]);
        let next_html = "<p>Before</p><img src='images/b.png'><p>After</p>";
        let before = assets.cache.keys()?.into_iter().collect::<BTreeSet<_>>();
        let usage = assets.usage()?;
        assert!(
            assets
                .commit_chapter(
                    &store,
                    "book",
                    "next-uid",
                    next_html,
                    TEXT,
                    Some(&next_packet),
                )
                .is_err()
        );
        assert!(assets.raw_chapter(&store, "book", "next-uid")?.is_none());
        assert_eq!(
            assets.cache.keys()?.into_iter().collect::<BTreeSet<_>>(),
            before
        );
        assert_eq!(assets.usage()?, usage);
        assert!(assets.cache.get_for_commit(&next_key)?.is_none());
        assert_eq!(assets.cache.pinned_state(shared_key)?, Some(true));
        store.set_setting(&key, &original)?;
        let current = assets.commit_chapter(
            &store,
            "book",
            "next-uid",
            next_html,
            TEXT,
            Some(&next_packet),
        )?;
        assert!(current.complete);
        assert!(!assets.reconcile_required.get());
        assert_eq!(assets.cache.pinned_state(shared_key)?, Some(true));
        assert!(assets.complete(&store, "book", "uid")?);
        Ok(())
    }

    #[test]
    fn acquired_images_are_real_grayscale_pngs_pinned_at_scalar_positions() -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let downloaded = packet(vec![asset("images/a.png", 7)?]);
        let manifest =
            assets.commit_chapter(&store, "book", "uid", HTML, TEXT, Some(&downloaded))?;
        assert!(manifest.complete);
        assert_eq!(manifest.images.len(), 1);
        assert!(manifest.images[0].anchor.is_some());
        let path = assets.image_path(&manifest.images[0])?.unwrap();
        assert_eq!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("png")
        );
        let raster = image::open(&path)?.to_luma8();
        assert_eq!(raster.dimensions(), (8, 6));
        assert_eq!(
            raster,
            image::load_from_memory(&downloaded.assets[0].bytes)?.to_luma8()
        );
        assert_eq!(assets.cache.pinned_keys()?.len(), 1);
        assert!(assets.complete(&store, "book", "uid")?);
        Ok(())
    }

    #[test]
    fn canonical_chapter_images_preserve_every_gray8_luminance() -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let original = gray8_ramp_asset()?;
        let id = original.id.clone();
        let manifest = assets.commit_chapter(
            &store,
            "book",
            "uid",
            HTML,
            TEXT,
            Some(&packet(vec![original])),
        )?;
        assert!(manifest.complete);
        let occurrence = &manifest.images[0];
        assert_eq!(occurrence.asset_id.as_deref(), Some(id.as_str()));
        assert_eq!(occurrence.cache_key, Some(asset_cache_key(&id)?));
        let path = assets.image_path(occurrence)?.unwrap();
        let prepared = image::open(path)?;
        assert_eq!(prepared.color(), image::ColorType::L8);
        assert_eq!(prepared.width(), 256);
        assert_eq!(prepared.height(), 1);
        assert!(prepared.to_luma8().as_raw().iter().copied().eq(0_u8..=255));
        assert_eq!(assets.cache.pinned_keys()?.len(), 1);
        assert!(assets.complete(&store, "book", "uid")?);
        Ok(())
    }

    #[test]
    fn obsolete_quantized_cache_keys_are_rejected_without_erasing_the_payload() -> Result<()> {
        let (_directory, _store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let original = gray8_ramp_asset()?;
        let old_key = format!("book-image:v1:{}", original.id);
        let bytes = sixteen_level_ramp_png()?;
        let old_path = assets.cache.put_png(&old_key, &bytes)?;
        let occurrence = StoredImage {
            id: "old-quantized-occurrence".into(),
            asset_id: Some(original.id.clone()),
            cache_key: Some(old_key.clone()),
            width: 256,
            height: 1,
            ..Default::default()
        };
        assert!(assets.image_path(&occurrence).is_err());
        assert!(
            assets
                .cache
                .get_for_commit(&asset_cache_key(&original.id)?)?
                .is_none()
        );
        assert_eq!(std::fs::read(old_path)?, bytes);
        assert!(assets.cache.get_for_commit(&old_key)?.is_some());
        Ok(())
    }

    #[test]
    fn task_reuse_cannot_substitute_a_quantized_conversion_for_canonical_gray8() -> Result<()> {
        let original = gray8_ramp_asset()?;
        let id = original.id.clone();
        let current_key = PreparedImageKey::new(&id);
        assert_eq!(current_key.conversion_version, 2);
        assert_eq!(current_key.gray_levels, 256);
        let mut previous_key = current_key.clone();
        previous_key.conversion_version = 1;
        previous_key.gray_levels = 16;
        let previous = Arc::new(PreparedAsset {
            bytes: sixteen_level_ramp_png()?,
            width: 256,
            height: 1,
        });
        let cache = PreparedImageCache::default();
        cache.insert(previous_key.clone(), previous.clone());
        assert!(cache.get(&current_key).is_none());
        let prepared = prepare_downloaded_with_cache(packet(vec![original]), &cache)?;
        let current = &prepared.assets[&id];
        assert!(!Arc::ptr_eq(&previous, current));
        let raster = image::load_from_memory(&current.bytes)?.to_luma8();
        assert!(raster.as_raw().iter().copied().eq(0_u8..=255));
        assert!(cache.get(&previous_key).is_some());
        assert!(Arc::ptr_eq(&cache.get(&current_key).unwrap(), current));
        Ok(())
    }

    #[test]
    fn text_only_downloads_preserve_real_placeholders_and_no_image_chapters_can_complete()
    -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let manifest = assets.commit_chapter(&store, "book", "uid", HTML, TEXT, None)?;
        assert!(!manifest.complete);
        assert_eq!(manifest.html, HTML);
        assert_eq!(manifest.images[0].source, "images/a.png");
        assert_eq!(
            (manifest.images[0].width, manifest.images[0].height),
            (0, 0)
        );
        assert!(assets.image_path(&manifest.images[0])?.is_none());
        assert!(!assets.complete(&store, "book", "uid")?);
        let manifest = assets.commit_chapter(
            &store,
            "book",
            "uid",
            "<p>Before</p><p>After</p>",
            TEXT,
            None,
        )?;
        assert!(manifest.images.is_empty() && manifest.complete);
        assert!(assets.complete(&store, "book", "uid")?);
        Ok(())
    }

    #[test]
    fn quota_failure_rolls_back_new_pins_and_preserves_a_prior_manifest() -> Result<()> {
        let first = asset("images/a.png", 7)?;
        let second = asset("images/b.png", 51)?;
        let third = asset("images/c.png", 99)?;
        let first_size = prepare_asset(&first)?.bytes.len() as u64;
        let second_size = prepare_asset(&second)?.bytes.len() as u64;
        let third_size = prepare_asset(&third)?.bytes.len() as u64;
        let (_directory, store, assets) = setup(first_size + second_size + third_size - 1)?;
        let original = assets.commit_chapter(
            &store,
            "book",
            "uid",
            HTML,
            TEXT,
            Some(&packet(vec![first])),
        )?;
        let html = "<p>Before</p><img src='images/a.png'><img src='images/b.png'><img src='images/c.png'><p>After</p>";
        let downloaded = packet(vec![asset("images/a.png", 7)?, second, third]);
        assert!(
            assets
                .commit_chapter(&store, "book", "uid", html, TEXT, Some(&downloaded))
                .is_err()
        );
        assert_eq!(assets.chapter(&store, "book", "uid")?, Some(original));
        assert_eq!(assets.usage()?, first_size);
        assert_eq!(assets.cache.pinned_keys()?.len(), 1);
        assert!(assets.complete(&store, "book", "uid")?);
        Ok(())
    }

    #[test]
    fn invalid_original_digest_or_missing_image_never_marks_a_chapter_complete() -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let mut corrupt = asset("images/a.png", 7)?;
        corrupt.id = "0".repeat(64);
        assert!(
            assets
                .commit_chapter(
                    &store,
                    "book",
                    "uid",
                    HTML,
                    TEXT,
                    Some(&packet(vec![corrupt]))
                )
                .is_err()
        );
        assert!(
            assets
                .commit_chapter(
                    &store,
                    "book",
                    "uid",
                    HTML,
                    TEXT,
                    Some(&DownloadedChapterImages::default())
                )
                .is_err()
        );
        assert_eq!(assets.usage()?, 0);
        assert!(assets.chapter(&store, "book", "uid")?.is_none());
        Ok(())
    }

    #[test]
    fn image_aliases_keep_full_directory_and_query_identity() -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let downloaded = packet(vec![asset("images/a.png", 7)?]);
        let safe = "<p>Before</p><img src='./figures/../images/a.png#figure'><p>After</p>";
        assert!(
            assets
                .commit_chapter(&store, "book", "uid", safe, TEXT, Some(&downloaded))?
                .complete
        );
        for source in ["other/a.png", "images/a.png?variant=other"] {
            let html = format!("<p>Before</p><img src='{source}'><p>After</p>");
            assert!(
                assets
                    .commit_chapter(&store, "book", "uid", &html, TEXT, Some(&downloaded))
                    .is_err()
            );
        }
        assert!(assets.complete(&store, "book", "uid")?);
        Ok(())
    }

    #[test]
    fn repeated_assets_keep_distinct_occurrences_without_duplicate_storage() -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let downloaded = packet(vec![asset("images/a.png", 7)?]);
        let html = "<img src='images/a.png'><p>Before</p><img src='images/a.png'><p>After</p>";
        let manifest =
            assets.commit_chapter(&store, "book", "uid", html, TEXT, Some(&downloaded))?;
        assert_eq!(manifest.images.len(), 2);
        assert_ne!(manifest.images[0].id, manifest.images[1].id);
        assert_eq!(manifest.images[0].cache_key, manifest.images[1].cache_key);
        assert_ne!(manifest.images[0].anchor, manifest.images[1].anchor);
        assert_eq!(assets.cache.keys()?.len(), 1);
        Ok(())
    }

    #[test]
    fn recovery_cleans_orphan_pins_and_downgrades_missing_assets() -> Result<()> {
        let (directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let downloaded = packet(vec![asset("images/a.png", 7)?]);
        let manifest =
            assets.commit_chapter(&store, "book", "uid", HTML, TEXT, Some(&downloaded))?;
        let path = assets.image_path(&manifest.images[0])?.unwrap();
        let orphan = asset("unused.png", 99)?;
        let orphan_key = asset_cache_key(&orphan.id)?;
        assets
            .cache
            .put_png(&orphan_key, &prepare_asset(&orphan)?.bytes)?;
        assets.cache.set_pinned(&orphan_key, true)?;
        std::fs::remove_file(path)?;
        drop(assets);
        let assets = BookAssetStore::open(directory.path().join("images"), DEFAULT_IMAGE_BUDGET)?;
        assets.recover(&store)?;
        assert!(assets.cache.keys()?.is_empty());
        assert!(!assets.chapter(&store, "book", "uid")?.unwrap().complete);
        assert!(!assets.complete(&store, "book", "uid")?);
        Ok(())
    }

    #[test]
    fn body_checkpoint_binding_prevents_images_from_attaching_to_older_text() -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let downloaded = packet(vec![asset("images/a.png", 7)?]);
        let new_text = "Different\nText";
        assets.commit_chapter(
            &store,
            "book",
            "uid",
            "<p>Different</p><img src='images/a.png'><p>Text</p>",
            new_text,
            Some(&downloaded),
        )?;
        assert!(assets.chapter(&store, "book", "uid")?.is_none());
        assert!(!assets.complete(&store, "book", "uid")?);
        store.cache_chapter_body("book", 0, new_text)?;
        assert!(assets.complete(&store, "book", "uid")?);
        Ok(())
    }

    #[test]
    fn clearing_a_book_includes_historical_uids_and_preserves_shared_assets_and_notes() -> Result<()>
    {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let downloaded = packet(vec![asset("images/a.png", 7)?]);
        assets.commit_chapter(&store, "book", "uid", HTML, TEXT, Some(&downloaded))?;
        store.save_annotation(&Annotation {
            id: "personal-note".into(),
            book_id: "book".into(),
            start: 0,
            end: 6,
            quote: "Before".into(),
            ..Annotation::default()
        })?;
        store.upsert_chapter(&Chapter {
            id: "new-uid".into(),
            book_id: "book".into(),
            index: 0,
            body: Some(TEXT.into()),
            ..Chapter::default()
        })?;
        assets.commit_chapter(&store, "book", "new-uid", HTML, TEXT, Some(&downloaded))?;
        store.upsert_book(&Book::new("other-book", "Other fixture"))?;
        store.upsert_chapter(&Chapter {
            id: "other-uid".into(),
            book_id: "other-book".into(),
            index: 0,
            body: Some(TEXT.into()),
            ..Chapter::default()
        })?;
        assets.commit_chapter(
            &store,
            "other-book",
            "other-uid",
            HTML,
            TEXT,
            Some(&downloaded),
        )?;
        assets.clear_book(&store, "book")?;
        assert!(assets.raw_chapter(&store, "book", "uid")?.is_none());
        assert!(assets.raw_chapter(&store, "book", "new-uid")?.is_none());
        assert_eq!(store.annotations("book")?.len(), 1);
        assert_eq!(assets.cache.keys()?.len(), 1);
        assert_eq!(assets.cache.pinned_keys()?.len(), 1);
        assert!(assets.complete(&store, "other-book", "other-uid")?);
        assets.clear_book(&store, "other-book")?;
        assert_eq!(assets.usage()?, 0);
        Ok(())
    }

    #[test]
    fn unknown_anchors_are_retained_without_fabricating_a_text_position() -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let downloaded = packet(vec![asset("images/a.png", 7)?]);
        let manifest = assets.commit_chapter(
            &store,
            "book",
            "uid",
            "<p>Unmatched text</p><img src='images/a.png'>",
            TEXT,
            Some(&downloaded),
        )?;
        assert!(manifest.images[0].anchor.is_none());
        assert!(manifest.complete);
        assert!(assets.image_path(&manifest.images[0])?.is_some());
        Ok(())
    }

    #[test]
    fn transparent_pixels_are_composited_on_white_before_grayscale() -> Result<()> {
        let raster = image::RgbaImage::from_fn(3, 1, |x, _| match x {
            0 => image::Rgba([0, 0, 0, 0]),
            1 => image::Rgba([0, 0, 0, 255]),
            _ => image::Rgba([0, 0, 0, 128]),
        });
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(raster).write_to(&mut encoded, image::ImageFormat::Png)?;
        let bytes = encoded.into_inner();
        let asset = ImageAsset {
            id: digest(&bytes),
            media_type: "image/png".into(),
            extension: "png".into(),
            source_names: vec!["transparent.png".into()],
            bytes,
        };
        let prepared = prepare_asset(&asset)?;
        let raster = image::load_from_memory(&prepared.bytes)?.to_luma8();
        assert_eq!(raster.get_pixel(0, 0)[0], 255);
        assert_eq!(raster.get_pixel(1, 0)[0], 0);
        assert_eq!(raster.get_pixel(2, 0)[0], 127);
        Ok(())
    }

    #[test]
    fn postcommit_cleanup_failure_does_not_report_committed_assets_as_rolled_back() -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let original = assets.commit_chapter(
            &store,
            "book",
            "uid",
            HTML,
            TEXT,
            Some(&packet(vec![asset("images/a.png", 7)?])),
        )?;
        let old_path = assets.image_path(&original.images[0])?.unwrap();
        std::fs::remove_file(&old_path)?;
        std::fs::create_dir(&old_path)?;
        let html = "<p>Before</p><img src='images/b.png'><p>After</p>";
        let replacement = packet(vec![asset("images/b.png", 51)?]);
        let current =
            assets.commit_chapter(&store, "book", "uid", html, TEXT, Some(&replacement))?;
        assert!(current.complete);
        assert_eq!(
            assets.chapter(&store, "book", "uid")?,
            Some(current.clone())
        );
        assert!(assets.complete(&store, "book", "uid")?);
        assert!(assets.reconcile_required.get());
        store.upsert_chapter(&Chapter {
            id: "next-uid".into(),
            book_id: "book".into(),
            index: 1,
            body: Some(TEXT.into()),
            ..Chapter::default()
        })?;
        let usage = assets.usage()?;
        // This publication reuses an existing image. It must still be blocked
        // by a retired payload that only the cleanup ledger can now see.
        assert!(
            assets
                .commit_chapter(&store, "book", "next-uid", html, TEXT, Some(&replacement),)
                .is_err()
        );
        assert!(assets.reconcile_required.get());
        assert!(assets.raw_chapter(&store, "book", "next-uid")?.is_none());
        assert_eq!(assets.chapter(&store, "book", "uid")?, Some(current));
        std::fs::remove_dir(&old_path)?;
        let next =
            assets.commit_chapter(&store, "book", "next-uid", html, TEXT, Some(&replacement))?;
        assert!(next.complete);
        assert!(!assets.reconcile_required.get());
        assert!(assets.usage()? < usage);
        Ok(())
    }

    #[test]
    fn preparation_runs_without_storage_and_discards_unreferenced_archive_images() -> Result<()> {
        let first = asset("images/a.png", 7)?;
        let second = asset("unreferenced/b.png", 51)?;
        let id = first.id.clone();
        let source_to_asset = BTreeMap::from([("images/a.png".to_owned(), id.clone())]);
        let downloaded = DownloadedChapterImages {
            assets: vec![first, second],
            source_to_asset,
        };
        let prepared = std::thread::spawn(move || prepare_downloaded(downloaded))
            .join()
            .expect("preparation worker must not panic")?;
        assert_eq!(prepared.assets.len(), 1);
        assert!(prepared.assets[&id].bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        assert!(
            assets
                .commit_prepared_chapter(&store, "book", "uid", HTML, TEXT, Some(&prepared))?
                .complete
        );
        Ok(())
    }

    #[test]
    fn empty_preparation_cannot_claim_a_referenced_image_is_complete() -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let prepared = prepare_downloaded(DownloadedChapterImages::default())?;
        assert!(
            assets
                .commit_prepared_chapter(&store, "book", "uid", HTML, TEXT, Some(&prepared))
                .is_err()
        );
        assert!(assets.chapter(&store, "book", "uid")?.is_none());
        assert_eq!(assets.usage()?, 0);
        Ok(())
    }

    #[test]
    fn repeated_task_images_reuse_pngs_without_reusing_chapter_sources() -> Result<()> {
        let cache = PreparedImageCache::default();
        let original = asset("chapter-one/a.png", 7)?;
        let id = original.id.clone();
        let first = prepare_downloaded_with_cache(packet(vec![original.clone()]), &cache)?;
        let mut renamed = original.clone();
        renamed.source_names = vec!["chapter-two/same.png".into()];
        let second = prepare_downloaded_with_cache(packet(vec![renamed]), &cache)?;
        assert!(Arc::ptr_eq(&first.assets[&id], &second.assets[&id]));
        assert_eq!(
            resolve_source(&second.sources, "chapter-two/same.png")?,
            Some(id.clone())
        );
        assert!(resolve_source(&second.sources, "chapter-one/a.png")?.is_none());
        let other_task = PreparedImageCache::default();
        let third = prepare_downloaded_with_cache(packet(vec![original]), &other_task)?;
        assert!(!Arc::ptr_eq(&first.assets[&id], &third.assets[&id]));
        Ok(())
    }

    #[test]
    fn cached_conversion_never_bypasses_original_integrity_or_source_checks() -> Result<()> {
        let cache = PreparedImageCache::default();
        let original = asset("images/a.png", 7)?;
        let id = original.id.clone();
        let first = prepare_downloaded_with_cache(packet(vec![original.clone()]), &cache)?;
        let mut corrupt = original.clone();
        corrupt.bytes[0] ^= 1;
        assert!(prepare_downloaded_with_cache(packet(vec![corrupt]), &cache).is_err());
        let mut unsupported = original.clone();
        unsupported.media_type = "image/svg+xml".into();
        assert!(prepare_downloaded_with_cache(packet(vec![unsupported]), &cache).is_err());
        let mut invalid_source = original.clone();
        invalid_source.source_names = vec!["bad\nsource.png".into()];
        assert!(prepare_downloaded_with_cache(packet(vec![invalid_source]), &cache).is_err());
        let second = prepare_downloaded_with_cache(packet(vec![original]), &cache)?;
        assert!(Arc::ptr_eq(&first.assets[&id], &second.assets[&id]));
        Ok(())
    }

    #[test]
    fn prepared_png_cache_evicts_by_bytes_and_recent_reuse() -> Result<()> {
        let originals = [asset("a.png", 7)?, asset("b.png", 51)?, asset("c.png", 99)?];
        let mut sizes = Vec::new();
        for original in &originals {
            sizes.push(prepare_asset(original)?.bytes.len());
        }
        // Every pair fits, while all three payloads exceed the byte budget.
        let budget = [
            sizes[0] + sizes[1],
            sizes[0] + sizes[2],
            sizes[1] + sizes[2],
        ]
        .into_iter()
        .max()
        .unwrap();
        let cache = PreparedImageCache::new(budget);
        let first = prepare_downloaded_with_cache(packet(vec![originals[0].clone()]), &cache)?;
        let _second = prepare_downloaded_with_cache(packet(vec![originals[1].clone()]), &cache)?;
        let hit = prepare_downloaded_with_cache(packet(vec![originals[0].clone()]), &cache)?;
        assert!(Arc::ptr_eq(
            &first.assets[&originals[0].id],
            &hit.assets[&originals[0].id]
        ));
        let _third = prepare_downloaded_with_cache(packet(vec![originals[2].clone()]), &cache)?;
        assert!(
            cache
                .get(&PreparedImageKey::new(&originals[1].id))
                .is_none()
        );
        assert!(
            cache
                .get(&PreparedImageKey::new(&originals[0].id))
                .is_some()
        );
        assert!(
            cache
                .get(&PreparedImageKey::new(&originals[2].id))
                .is_some()
        );
        let state = cache.state.lock().unwrap();
        assert_eq!(state.used_bytes, sizes[0] + sizes[2]);
        assert!(state.used_bytes <= budget);
        Ok(())
    }

    #[test]
    fn image_larger_than_reuse_budget_still_prepares_without_being_retained() -> Result<()> {
        let original = asset("a.png", 7)?;
        let id = original.id.clone();
        let size = prepare_asset(&original)?.bytes.len();
        let cache = PreparedImageCache::new(size - 1);
        let first = prepare_downloaded_with_cache(packet(vec![original.clone()]), &cache)?;
        let second = prepare_downloaded_with_cache(packet(vec![original]), &cache)?;
        assert!(!Arc::ptr_eq(&first.assets[&id], &second.assets[&id]));
        assert!(first.assets[&id].bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(cache.state.lock().unwrap().used_bytes, 0);
        Ok(())
    }

    #[test]
    fn cancellation_clears_pngs_and_rejects_late_worker_insertion() -> Result<()> {
        let cache = Arc::new(PreparedImageCache::default());
        let original = asset("a.png", 7)?;
        let id = original.id.clone();
        let prepared = prepare_downloaded_with_cache(packet(vec![original.clone()]), &cache)?;
        let worker_cache = cache.clone();
        let worker_original = original.clone();
        let (ready, observed) = std::sync::mpsc::channel();
        let release = Arc::new(std::sync::Barrier::new(2));
        let worker_release = release.clone();
        let worker = std::thread::spawn(move || -> Result<()> {
            let late = Arc::new(prepare_asset(&worker_original)?);
            ready.send(()).unwrap();
            worker_release.wait();
            worker_cache.insert(PreparedImageKey::new(&worker_original.id), late);
            Ok(())
        });
        observed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        cache.cancel();
        release.wait();
        worker.join().expect("late image worker must not panic")?;
        let state = cache.state.lock().unwrap();
        assert_eq!(state.used_bytes, 0);
        assert!(state.entries.is_empty());
        drop(state);
        assert!(cache.get(&PreparedImageKey::new(&id)).is_none());
        assert!(prepare_downloaded_with_cache(packet(vec![original]), &cache).is_err());
        // Packets already owned by a receiver remain valid; their Arcs are not
        // the retained cache budget and cannot be reused by a new task.
        assert!(prepared.assets[&id].bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        Ok(())
    }

    #[test]
    fn prepared_metadata_rejects_changed_html_or_text_before_storage() -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let metadata = prepare_chapter_metadata(HTML, TEXT)?;
        let prepared = prepare_downloaded(packet(vec![asset("images/a.png", 7)?]))?;
        for (html, text) in [("<img src='images/a.png'>", TEXT), (HTML, "Changed body")] {
            assert!(
                assets
                    .commit_prepared_chapter_metadata(
                        &store,
                        "book",
                        "uid",
                        html,
                        text,
                        &metadata,
                        Some(&prepared),
                    )
                    .is_err()
            );
            assert_eq!(assets.usage()?, 0);
            assert!(assets.raw_chapter(&store, "book", "uid")?.is_none());
        }
        let manifest = assets.commit_prepared_chapter_metadata(
            &store,
            "book",
            "uid",
            HTML,
            TEXT,
            &metadata,
            Some(&prepared),
        )?;
        assert!(manifest.complete);
        assert_eq!(manifest.images[0].anchor, Some(6));
        Ok(())
    }

    #[test]
    fn shared_document_metadata_preserves_unicode_captions_and_hidden_source_checks() -> Result<()>
    {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let html = "<p>A😀</p><figure><img src='images/a.png'><figcaption>Caption</figcaption></figure><p>B</p><div hidden><img></div><svg><image href='unsupported.svg'></image></svg>";
        for canonical in ["A😀\nB", "A😀\nCaption\nB"] {
            let metadata = prepare_chapter_metadata(html, canonical)?;
            assert_eq!(metadata.anchors.len(), 1);
            assert_eq!(metadata.anchors[0].anchor, Some(2));
            assert_eq!(metadata.anchors[0].caption, "Caption");
            assert_eq!(metadata.unresolved_images, 2);
            let prepared = prepare_downloaded(packet(vec![asset("images/a.png", 7)?]))?;
            assert!(
                assets
                    .commit_prepared_chapter_metadata(
                        &store,
                        "book",
                        "uid",
                        html,
                        canonical,
                        &metadata,
                        Some(&prepared),
                    )
                    .is_err()
            );
            assert_eq!(assets.usage()?, 0);
            let manifest = assets.commit_prepared_chapter_metadata(
                &store, "book", "uid", html, canonical, &metadata, None,
            )?;
            assert!(!manifest.complete);
            assert_eq!(manifest.unresolved_images, 2);
            assert_eq!(manifest.images[0].anchor, Some(2));
        }
        Ok(())
    }

    #[test]
    fn failed_commit_restores_one_prior_unpinned_state_for_repeated_occurrences() -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let original = asset("images/a.png", 7)?;
        let key = asset_cache_key(&original.id)?;
        assets
            .cache
            .put_png(&key, &prepare_asset(&original)?.bytes)?;
        let html = "<p>Before</p><img src='images/a.png'><img src='images/a.png'><img src='missing.png'><p>After</p>";
        let prepared = prepare_downloaded(packet(vec![original]))?;
        let metadata = prepare_chapter_metadata(html, TEXT)?;
        assert!(
            assets
                .commit_prepared_chapter_metadata(
                    &store,
                    "book",
                    "uid",
                    html,
                    TEXT,
                    &metadata,
                    Some(&prepared),
                )
                .is_err()
        );
        assert_eq!(assets.cache.pinned_state(&key)?, Some(false));
        assert!(assets.cache.get_for_commit(&key)?.is_some());
        assert!(assets.raw_chapter(&store, "book", "uid")?.is_none());
        Ok(())
    }

    #[test]
    fn failed_commit_preserves_a_recreated_shared_pin_and_removes_new_assets() -> Result<()> {
        let (_directory, store, assets) = setup(DEFAULT_IMAGE_BUDGET)?;
        let shared = asset("images/a.png", 7)?;
        let original = assets.commit_chapter(
            &store,
            "book",
            "uid",
            HTML,
            TEXT,
            Some(&packet(vec![shared.clone()])),
        )?;
        let shared_key = original.images[0].cache_key.as_ref().unwrap();
        std::fs::remove_file(assets.image_path(&original.images[0])?.unwrap())?;
        store.upsert_book(&Book::new("other-book", "Other fixture book"))?;
        store.upsert_chapter(&Chapter {
            id: "other-uid".into(),
            book_id: "other-book".into(),
            index: 0,
            body: Some(TEXT.into()),
            ..Chapter::default()
        })?;
        let new = asset("images/b.png", 51)?;
        let new_key = asset_cache_key(&new.id)?;
        let html = "<p>Before</p><img src='images/a.png'><img src='images/b.png'><img src='missing.png'><p>After</p>";
        let prepared = prepare_downloaded(packet(vec![shared, new]))?;
        let metadata = prepare_chapter_metadata(html, TEXT)?;
        assert!(
            assets
                .commit_prepared_chapter_metadata(
                    &store,
                    "other-book",
                    "other-uid",
                    html,
                    TEXT,
                    &metadata,
                    Some(&prepared),
                )
                .is_err()
        );
        assert_eq!(assets.cache.pinned_state(shared_key)?, Some(true));
        assert!(assets.complete(&store, "book", "uid")?);
        assert!(assets.cache.get_for_commit(&new_key)?.is_none());
        assert!(
            assets
                .raw_chapter(&store, "other-book", "other-uid")?
                .is_none()
        );
        Ok(())
    }

    #[test]
    #[ignore = "Manual synthetic CPU benchmark; run on the designated verification host."]
    fn benchmark_task_scoped_prepared_image_cache() -> Result<()> {
        use std::time::Instant;

        const WIDTH: u32 = 800;
        const HEIGHT: u32 = 1000;
        const REPETITIONS: usize = 8;
        const SOURCE: &str = "images/synthetic-gradient.png";
        let raster = image::RgbImage::from_fn(WIDTH, HEIGHT, |x, y| {
            image::Rgb([
                (x * 255 / (WIDTH - 1)) as u8,
                (y * 255 / (HEIGHT - 1)) as u8,
                ((x + y) * 255 / (WIDTH + HEIGHT - 2)) as u8,
            ])
        });
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(raster).write_to(&mut encoded, image::ImageFormat::Png)?;
        let bytes = encoded.into_inner();
        let original = ImageAsset {
            id: digest(&bytes),
            media_type: "image/png".into(),
            extension: "png".into(),
            source_names: vec![SOURCE.into()],
            bytes,
        };
        let id = original.id.clone();
        let mut fresh = Vec::with_capacity(REPETITIONS);
        let fresh_start = Instant::now();
        for _ in 0..REPETITIONS {
            fresh.push(prepare_downloaded(packet(vec![original.clone()]))?);
        }
        let fresh_wall = fresh_start.elapsed();

        let cache = PreparedImageCache::default();
        let mut reused = Vec::with_capacity(REPETITIONS);
        let reused_start = Instant::now();
        let cold_start = Instant::now();
        reused.push(prepare_downloaded_with_cache(
            packet(vec![original.clone()]),
            &cache,
        )?);
        let cold_wall = cold_start.elapsed();
        let hit_start = Instant::now();
        for _ in 1..REPETITIONS {
            reused.push(prepare_downloaded_with_cache(
                packet(vec![original.clone()]),
                &cache,
            )?);
        }
        let hit_wall = hit_start.elapsed();
        let reused_wall = reused_start.elapsed();

        // Output checks are outside both measured loops. Keep every packet alive
        // so Arc identity measures reuse without allocator-address recycling.
        let expected_png_digest = digest(&fresh[0].assets[&id].bytes);
        let png = image::load_from_memory(&fresh[0].assets[&id].bytes)?;
        assert_eq!((png.width(), png.height()), (WIDTH, HEIGHT));
        assert_eq!(png.color(), image::ColorType::L8);
        assert_eq!(
            png.to_luma8(),
            image::load_from_memory(&original.bytes)?.to_luma8()
        );
        for prepared in fresh.iter().chain(reused.iter()) {
            let image = &prepared.assets[&id];
            assert_eq!((image.width, image.height), (WIDTH, HEIGHT));
            assert_eq!(digest(&image.bytes), expected_png_digest);
            assert_eq!(resolve_source(&prepared.sources, SOURCE)?, Some(id.clone()));
        }
        let html = format!("<p>Before</p><img src='{SOURCE}'><p>After</p>");
        let metadata = prepare_chapter_metadata(&html, TEXT)?;
        assert_eq!(metadata.html_sha256, digest(html.as_bytes()));
        assert_eq!(metadata.text_sha256, digest(TEXT.as_bytes()));
        assert_eq!(metadata.anchors.len(), 1);
        assert_eq!(metadata.anchors[0].source, SOURCE);
        assert_eq!(metadata.anchors[0].anchor, Some(6));
        assert_eq!(metadata.unresolved_images, 0);

        let fresh_hits = fresh
            .windows(2)
            .filter(|packets| Arc::ptr_eq(&packets[0].assets[&id], &packets[1].assets[&id]))
            .count();
        let reused_hits = reused
            .windows(2)
            .filter(|packets| Arc::ptr_eq(&packets[0].assets[&id], &packets[1].assets[&id]))
            .count();
        assert_eq!(fresh_hits, 0);
        assert_eq!(reused_hits, REPETITIONS - 1);
        println!(
            "prepared_image_benchmark synthetic/non-device width={WIDTH} height={HEIGHT} repetitions={REPETITIONS} raw_bytes={} png_bytes={} fresh_wall_ms={:.3} fresh_cold={} fresh_hits={} reused_wall_ms={:.3} reused_cold={} reused_hits={} reused_cold_wall_ms={:.3} reused_hit_wall_ms={:.3}",
            original.bytes.len(),
            fresh[0].assets[&id].bytes.len(),
            fresh_wall.as_secs_f64() * 1000.0,
            REPETITIONS - fresh_hits,
            fresh_hits,
            reused_wall.as_secs_f64() * 1000.0,
            REPETITIONS - reused_hits,
            reused_hits,
            cold_wall.as_secs_f64() * 1000.0,
            hit_wall.as_secs_f64() * 1000.0,
        );
        Ok(())
    }
}
