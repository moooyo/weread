//! Chapter resources observed in content.lua at the recorded reference revision.
//! Catalog `tar` archives and chapter `img src` values are independent of covers.
use super::{Result, SourceError, WeReadClient, protocol::reader_url_checked as reader_url};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    future::Future,
    io::{Cursor, Read},
};
use url::Url;

#[path = "image_svg_guard.rs"]
mod image_svg_guard;

#[cfg(test)]
pub(crate) use image_svg_guard::Limits as SvgGuardLimits;

/// Count the locked selector's SVG/image relation under complete resource
/// admission, without searching every image's ancestor chain again.
pub(crate) fn count_svg_images(document: &Html) -> Result<usize> {
    image_svg_guard::count_svg_images(document)
}

pub const MAX_ARCHIVE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_EXPANDED_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_REMOTE_FETCH_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_COMPRESSED_WORK_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_IMAGE_COUNT: usize = 512;
pub const MAX_ARCHIVE_ENTRIES: usize = 4096;
const MAX_IMAGE_EDGE: u32 = 8192;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;
const MAX_SOURCE_NAMES: usize = MAX_ARCHIVE_ENTRIES + MAX_IMAGE_COUNT;
const MAX_SOURCE_NAME_BYTES: usize = MAX_SOURCE_NAMES * 4096;
const MAX_SOURCE_NAME_WORK: usize = 128 * 1024 * 1024;
const MAX_ASSET_HASH_WORK: usize = MAX_EXPANDED_BYTES + MAX_REMOTE_FETCH_BYTES;

#[derive(Clone, Copy)]
struct MemberLimits {
    names: usize,
    name_bytes: usize,
    name_work: usize,
    hash_work: usize,
}

impl Default for MemberLimits {
    fn default() -> Self {
        Self {
            names: MAX_SOURCE_NAMES,
            name_bytes: MAX_SOURCE_NAME_BYTES,
            name_work: MAX_SOURCE_NAME_WORK,
            hash_work: MAX_ASSET_HASH_WORK,
        }
    }
}

#[derive(Default)]
struct MemberUsage {
    names: usize,
    name_bytes: usize,
    name_work: usize,
    hash_work: usize,
}

/// Only names admitted before copying can reach the membership operation.
struct AdmittedName(String);

/// One decoder/acquisition's index. Output aliases stay in first-seen order;
/// membership never rescans a digest's entire growing alias vector.
#[derive(Default)]
struct AssetAccumulator {
    assets: Vec<ImageAsset>,
    by_digest: BTreeMap<String, usize>,
    members: Vec<HashSet<String>>,
    retained_bytes: usize,
    limits: MemberLimits,
    usage: MemberUsage,
}

impl AssetAccumulator {
    fn admit_name(&mut self, parts: &[&str]) -> Result<AdmittedName> {
        let length = parts
            .iter()
            .try_fold(0usize, |length, part| length.checked_add(part.len()))
            .filter(|length| *length > 0 && *length <= 4096)
            .ok_or(SourceError::ResponseTooLarge)?;
        let names = self
            .usage
            .names
            .checked_add(1)
            .filter(|count| *count <= self.limits.names)
            .ok_or(SourceError::ResponseTooLarge)?;
        let name_bytes = self
            .usage
            .name_bytes
            .checked_add(length)
            .filter(|bytes| *bytes <= self.limits.name_bytes)
            .ok_or(SourceError::ResponseTooLarge)?;
        // Prepay copying and membership hash/equality byte passes. This is an
        // operation allowance, not a claim about allocator or wall-clock cost.
        let fee = length
            .checked_mul(6)
            .and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<String>()))
            .ok_or(SourceError::ResponseTooLarge)?;
        let name_work = self
            .usage
            .name_work
            .checked_add(fee)
            .filter(|bytes| *bytes <= self.limits.name_work)
            .ok_or(SourceError::ResponseTooLarge)?;
        self.usage.names = names;
        self.usage.name_bytes = name_bytes;
        self.usage.name_work = name_work;
        let mut name = String::with_capacity(length);
        for part in parts {
            name.push_str(part);
        }
        Ok(AdmittedName(name))
    }

    fn admit_binding(&mut self, name: &str) -> Result<()> {
        if name.is_empty() || name.len() > 4096 {
            return Err(SourceError::ResponseTooLarge);
        }
        self.usage.name_work = self
            .usage
            .name_work
            .checked_add(name.len())
            .filter(|bytes| *bytes <= self.limits.name_work)
            .ok_or(SourceError::ResponseTooLarge)?;
        Ok(())
    }

    fn add_image(&mut self, bytes: Vec<u8>, names: Vec<AdmittedName>) -> Result<Option<String>> {
        self.usage.hash_work = self
            .usage
            .hash_work
            .checked_add(bytes.len())
            .filter(|bytes| *bytes <= self.limits.hash_work)
            .ok_or(SourceError::ResponseTooLarge)?;
        let Some(mut asset) = image_asset(bytes, Vec::new())? else {
            return Ok(None);
        };
        let id = asset.id.clone();
        if let Some(index) = self.by_digest.get(&id).copied() {
            let members = &mut self.members[index];
            let output = &mut self.assets[index].source_names;
            for AdmittedName(name) in names {
                if !members.contains(&name) {
                    members.insert(name.clone());
                    output.push(name);
                }
            }
        } else {
            let retained = self
                .retained_bytes
                .checked_add(asset.bytes.len())
                .filter(|bytes| *bytes <= MAX_EXPANDED_BYTES)
                .ok_or(SourceError::ResponseTooLarge)?;
            if self.assets.len() >= MAX_IMAGE_COUNT {
                return Err(SourceError::ResponseTooLarge);
            }
            let mut members = HashSet::new();
            for AdmittedName(name) in names {
                if members.insert(name.clone()) {
                    asset.source_names.push(name);
                }
            }
            self.by_digest.insert(id.clone(), self.assets.len());
            self.retained_bytes = retained;
            self.assets.push(asset);
            self.members.push(members);
        }
        Ok(Some(id))
    }

    fn retain(&mut self, mut keep: impl FnMut(&ImageAsset) -> bool) {
        let assets = std::mem::take(&mut self.assets);
        let members = std::mem::take(&mut self.members);
        self.by_digest.clear();
        self.retained_bytes = 0;
        // Move existing sets with their assets. Pruning does not rehash or
        // recopy every alias, and processed-byte/work reservations stay spent.
        for (asset, members) in assets.into_iter().zip(members) {
            if keep(&asset) {
                self.by_digest.insert(asset.id.clone(), self.assets.len());
                self.retained_bytes += asset.bytes.len();
                self.assets.push(asset);
                self.members.push(members);
            }
        }
    }

    fn into_assets(self) -> Vec<ImageAsset> {
        self.assets
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ImageRequest {
    pub url: String,
    pub referer: String,
    pub source: String,
}

impl std::fmt::Debug for ImageRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageRequest")
            .field(
                "origin",
                &Url::parse(&self.url)
                    .ok()
                    .map(|url| url.origin().ascii_serialization()),
            )
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageReference {
    pub source: String,
    /// Full canonical archive reference, including its directory and query.
    pub archive_name: Option<String>,
    pub remote_url: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ChapterImagePlan {
    /// Converter namespace scoped to this plan's book, never a global filename fallback.
    #[serde(default)]
    pub archive_namespace: Option<String>,
    pub archive: Option<ImageRequest>,
    pub remote_images: Vec<ImageRequest>,
    pub references: Vec<ImageReference>,
}

impl std::fmt::Debug for ChapterImagePlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChapterImagePlan")
            .field(
                "archive_namespace_present",
                &self.archive_namespace.is_some(),
            )
            .field("archive_present", &self.archive.is_some())
            .field("remote_count", &self.remote_images.len())
            .field("reference_count", &self.references.len())
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ImageAsset {
    /// Content digest, suitable for a path-safe cache identity.
    pub id: String,
    pub media_type: String,
    pub extension: String,
    pub source_names: Vec<String>,
    #[serde(skip)]
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for ImageAsset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageAsset")
            .field("id", &self.id)
            .field("media_type", &self.media_type)
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct DownloadedChapterImages {
    pub assets: Vec<ImageAsset>,
    /// Decoded DOM attribute to acquired raw-byte digest; URLs are private metadata.
    /// Raster readiness is established by the downstream CPU preparation stage.
    pub source_to_asset: BTreeMap<String, String>,
}

impl std::fmt::Debug for DownloadedChapterImages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DownloadedChapterImages")
            .field("asset_count", &self.assets.len())
            .field("reference_count", &self.source_to_asset.len())
            .finish()
    }
}

/// Plan only observed `img src` and catalog archive contracts. Unknown formats fail.
pub fn make_plan(
    book_id: &str,
    chapter_uid: &str,
    html: &str,
    archive_url: Option<&str>,
) -> Result<ChapterImagePlan> {
    let archive_url = archive_url.filter(|url| !url.trim().is_empty());
    let document = Html::parse_document(html);
    if count_svg_images(&document)? != 0 {
        return Err(SourceError::Unsupported("svg_book_image"));
    }
    crate::article::check_anchor_preparation_work(&document)
        .map_err(|_| SourceError::ResponseTooLarge)?;
    let images = Selector::parse("img").expect("Static chapter image selector");
    let mut plan = ChapterImagePlan {
        archive_namespace: (!book_id.is_empty()).then(|| format!("CB_{book_id}_")),
        ..Default::default()
    };
    let mut seen_sources = BTreeSet::new();
    let mut seen_urls = BTreeSet::new();
    for image in document.select(&images) {
        let Some(source) = image
            .value()
            .attr("src")
            .map(str::trim)
            .filter(|source| !source.is_empty())
        else {
            if image.value().attr("data-src").is_some() {
                return Err(SourceError::Unsupported("lazy_book_image"));
            }
            continue;
        };
        if source.len() > 4096 || source.chars().any(char::is_control) {
            return Err(SourceError::InvalidResponse("image_reference"));
        }
        if !seen_sources.insert(source.to_owned()) {
            continue;
        }
        if plan.references.len() >= MAX_IMAGE_COUNT {
            return Err(SourceError::ResponseTooLarge);
        }
        if source.starts_with("data:") {
            return Err(SourceError::Unsupported("embedded_book_image"));
        }
        if source.starts_with("https://")
            || source.starts_with("http://")
            || source.starts_with("//")
        {
            let url = normalize_url(source, false)?;
            if seen_urls.insert(url.clone()) {
                plan.remote_images.push(ImageRequest {
                    url: url.clone(),
                    referer: "https://weread.qq.com/".into(),
                    source: source.into(),
                });
            }
            plan.references.push(ImageReference {
                source: source.into(),
                archive_name: None,
                remote_url: Some(url),
            });
        } else {
            if source.contains(':') || source.starts_with('#') {
                return Err(SourceError::Unsupported("book_image_reference"));
            }
            let name = canonical_reference(source)?;
            if name
                .split('?')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .ends_with(".svg")
            {
                return Err(SourceError::Unsupported("svg_book_image"));
            }
            if archive_url.is_none() {
                return Err(SourceError::Unsupported("missing_chapter_archive"));
            }
            plan.references.push(ImageReference {
                source: source.into(),
                archive_name: Some(name),
                remote_url: None,
            });
        }
    }
    if plan
        .references
        .iter()
        .any(|reference| reference.archive_name.is_some())
    {
        plan.archive = archive_url
            .map(|raw| {
                let url = normalize_url(raw, true)?;
                Ok(ImageRequest {
                    url,
                    referer: reader_url(book_id, Some(chapter_uid))?,
                    source: String::new(),
                })
            })
            .transpose()?;
    }
    Ok(plan)
}

fn normalize_url(raw: &str, archive: bool) -> Result<String> {
    // Include the largest supported relative prefix without scanning a field
    // before its raw byte gate. Exact URL rules remain below admission.
    super::protocol::admit_resource_url(
        raw,
        super::protocol::ResourceUrlStage::Raw,
        "https://weread.qq.com".len(),
    )?;
    let raw = raw.trim();
    let absolute = if raw.starts_with("//") {
        format!("https:{raw}")
    } else if archive && raw.starts_with('/') {
        format!("https://weread.qq.com{raw}")
    } else {
        raw.to_owned()
    };
    #[cfg(test)]
    super::client::reader_resource_budget_tests::observe_url_parse();
    let mut url = Url::parse(&absolute).map_err(|_| SourceError::InvalidResponse("image_url"))?;
    if url.scheme() != "https" {
        return Err(SourceError::Unsupported("insecure_book_image"));
    }
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        return Err(SourceError::InvalidResponse("image_url"));
    }
    url.set_fragment(None);
    super::protocol::check_normalized_resource_url(url.as_str())?;
    #[cfg(test)]
    super::client::reader_resource_budget_tests::observe_url_copy(url.as_str().len());
    Ok(url.to_string())
}

fn canonical_reference(source: &str) -> Result<String> {
    let without_fragment = source.split('#').next().unwrap_or_default();
    let (path, query) = without_fragment
        .split_once('?')
        .map_or((without_fragment, None), |(path, query)| {
            (path, Some(query))
        });
    let path = path
        .split('/')
        .filter(|component| *component != ".")
        .collect::<Vec<_>>()
        .join("/");
    if path.is_empty() || path.ends_with('/') || matches!(path.rsplit('/').next(), Some("." | ".."))
    {
        return Err(SourceError::InvalidResponse("image_reference"));
    }
    // No chapter document base is established by the protocol; never guess what `..` means.
    Ok(match query {
        Some(query) => format!("{path}?{query}"),
        None => path,
    })
}

fn reference_basename(source: &str) -> Option<String> {
    let (path, query) = source
        .split_once('?')
        .map_or((source, None), |(path, query)| (path, Some(query)));
    let name = path
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")?;
    Some(match query {
        Some(query) => format!("{name}?{query}"),
        None => name.into(),
    })
}

#[derive(Default)]
struct ArchiveIndex {
    exact: BTreeMap<String, BTreeSet<String>>,
    basename_paths: BTreeMap<String, BTreeSet<String>>,
    scoped_alias_paths: BTreeMap<String, BTreeSet<String>>,
}

impl ArchiveIndex {
    fn new(assets: &[ImageAsset]) -> Result<Self> {
        let mut index = Self::default();
        for asset in assets {
            for source in &asset.source_names {
                let canonical = canonical_reference(source)?;
                index
                    .exact
                    .entry(canonical.clone())
                    .or_default()
                    .insert(asset.id.clone());
                if let Some(name) = reference_basename(&canonical) {
                    index
                        .basename_paths
                        .entry(name)
                        .or_default()
                        .insert(canonical);
                }
            }
        }
        Ok(index)
    }

    fn with_namespace(assets: &[ImageAsset], namespace: Option<&str>) -> Result<Self> {
        let mut index = Self::new(assets)?;
        let Some(namespace) = namespace.filter(|namespace| !namespace.is_empty()) else {
            return Ok(index);
        };
        for path in index.exact.keys() {
            // Only a root member can carry the observed converter namespace.
            // Archive directories remain authoritative for ordinary resources.
            if path.split('?').next().unwrap_or_default().contains('/') {
                continue;
            }
            if let Some(alias) = path
                .strip_prefix(namespace)
                .filter(|alias| !alias.is_empty())
            {
                index
                    .scoped_alias_paths
                    .entry(alias.to_owned())
                    .or_default()
                    .insert(path.clone());
            }
        }
        for (alias, paths) in &mut index.scoped_alias_paths {
            if let Some(ordinary_paths) = index.basename_paths.get(alias) {
                // Equal bytes do not establish that different resource paths
                // are interchangeable. Count the raw canonical paths first.
                paths.extend(ordinary_paths.iter().cloned());
            }
        }
        Ok(index)
    }

    fn resolve_with_namespace(&self, source: &str) -> Result<(String, Option<String>)> {
        let canonical = canonical_reference(source)?;
        if let Some(ids) = self.exact.get(&canonical) {
            return Ok((unique_asset_id(ids)?, None));
        }
        let alias = reference_basename(&canonical)
            .ok_or(SourceError::Unsupported("missing_chapter_image"))?;
        if let Some(paths) = self.scoped_alias_paths.get(&alias) {
            if paths.len() != 1 {
                return Err(SourceError::Unsupported("ambiguous_chapter_image"));
            }
            let path = paths
                .first()
                .ok_or(SourceError::Unsupported("missing_chapter_image"))?;
            let ids = self
                .exact
                .get(path)
                .ok_or(SourceError::Unsupported("missing_chapter_image"))?;
            return Ok((unique_asset_id(ids)?, Some(alias)));
        }
        self.resolve(source).map(|id| (id, None))
    }

    fn resolve_references<'a>(
        &self,
        sources: impl IntoIterator<Item = &'a str>,
    ) -> Result<BTreeMap<String, String>> {
        let mut bindings = BTreeMap::new();
        let mut qualified_aliases: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for source in sources {
            let canonical = canonical_reference(source)?;
            let (id, scoped_alias) = self.resolve_with_namespace(&canonical)?;
            if let Some(alias) = scoped_alias {
                if canonical
                    .split('?')
                    .next()
                    .unwrap_or_default()
                    .contains('/')
                {
                    let claims = qualified_aliases.entry(alias).or_default();
                    claims.insert(canonical);
                    if claims.len() != 1 {
                        return Err(SourceError::Unsupported("ambiguous_chapter_image"));
                    }
                }
            }
            bindings.insert(source.to_owned(), id);
        }
        Ok(bindings)
    }

    fn resolve(&self, source: &str) -> Result<String> {
        let canonical = canonical_reference(source)?;
        if let Some(ids) = self.exact.get(&canonical) {
            return unique_asset_id(ids);
        }
        let path = canonical.split('?').next().unwrap_or_default();
        if path.contains('/') {
            return Err(SourceError::Unsupported("missing_chapter_image"));
        }
        // A bare filename may match only one full archive path, never a cross-directory guess.
        let paths = self
            .basename_paths
            .get(&canonical)
            .ok_or(SourceError::Unsupported("missing_chapter_image"))?;
        if paths.len() != 1 {
            return Err(SourceError::Unsupported("ambiguous_chapter_image"));
        }
        let path = paths
            .first()
            .ok_or(SourceError::Unsupported("missing_chapter_image"))?;
        unique_asset_id(
            self.exact
                .get(path)
                .ok_or(SourceError::Unsupported("missing_chapter_image"))?,
        )
    }
}

fn unique_asset_id(ids: &BTreeSet<String>) -> Result<String> {
    if ids.len() != 1 {
        return Err(SourceError::Unsupported("ambiguous_chapter_image"));
    }
    ids.first()
        .cloned()
        .ok_or(SourceError::Unsupported("missing_chapter_image"))
}

/// Acquire referenced resources. Raster readiness requires downstream CPU preparation.
pub async fn acquire(
    client: &WeReadClient,
    plan: &ChapterImagePlan,
) -> Result<DownloadedChapterImages> {
    acquire_with(plan, |request, maximum| async move {
        client
            .chapter_resource(&request.url, &request.referer, maximum)
            .await
    })
    .await
}

async fn acquire_with<F, Fut>(
    plan: &ChapterImagePlan,
    mut fetch: F,
) -> Result<DownloadedChapterImages>
where
    F: FnMut(ImageRequest, usize) -> Fut,
    Fut: Future<Output = Result<Vec<u8>>>,
{
    let mut result = DownloadedChapterImages::default();
    if plan.references.is_empty() {
        return Ok(result);
    }
    if plan.references.len() > MAX_IMAGE_COUNT || plan.remote_images.len() > MAX_IMAGE_COUNT {
        return Err(SourceError::ResponseTooLarge);
    }
    let mut collected = AssetAccumulator::default();
    let requires_archive = plan
        .references
        .iter()
        .any(|reference| reference.archive_name.is_some());
    if requires_archive {
        let archive = plan
            .archive
            .as_ref()
            .ok_or(SourceError::Unsupported("missing_chapter_archive"))?;
        let bytes = fetch(archive.clone(), MAX_ARCHIVE_BYTES).await?;
        collected = decode_archive_accumulator(&bytes)?;
    }
    let archive_index =
        ArchiveIndex::with_namespace(&collected.assets, plan.archive_namespace.as_deref())?;
    let archive_bindings = archive_index.resolve_references(
        plan.references
            .iter()
            .filter_map(|reference| reference.archive_name.as_deref()),
    )?;
    drop(archive_index);
    let referenced_archive_ids = archive_bindings.values().cloned().collect::<BTreeSet<_>>();
    collected.retain(|asset| referenced_archive_ids.contains(&asset.id));
    let referenced_remote_urls = plan
        .references
        .iter()
        .filter_map(|reference| reference.remote_url.as_deref())
        .collect::<BTreeSet<_>>();
    let mut remote_assets = BTreeMap::new();
    let mut fetched_remote_bytes = 0_usize;
    for request in &plan.remote_images {
        if !referenced_remote_urls.contains(request.url.as_str())
            || remote_assets.contains_key(&request.url)
        {
            continue;
        }
        // Retained assets are deduplicated by digest, but distinct URLs still
        // consume transport and hashing work. Bound those bytes independently
        // before fetching or inspecting another response, including duplicates.
        let remaining = MAX_REMOTE_FETCH_BYTES
            .checked_sub(fetched_remote_bytes)
            .ok_or(SourceError::ResponseTooLarge)?;
        let maximum = MAX_IMAGE_BYTES.min(remaining);
        if maximum == 0 {
            return Err(SourceError::ResponseTooLarge);
        }
        let name = collected.admit_name(&[request.source.as_str()])?;
        let bytes = fetch(request.clone(), maximum).await?;
        if bytes.len() > maximum {
            return Err(SourceError::ResponseTooLarge);
        }
        fetched_remote_bytes = fetched_remote_bytes
            .checked_add(bytes.len())
            .filter(|bytes| *bytes <= MAX_REMOTE_FETCH_BYTES)
            .ok_or(SourceError::ResponseTooLarge)?;
        let id = collected
            .add_image(bytes, vec![name])?
            .ok_or(SourceError::Unsupported("book_image_format"))?;
        remote_assets.insert(request.url.clone(), id);
    }
    for reference in &plan.references {
        let id = if let Some(url) = &reference.remote_url {
            remote_assets
                .get(url)
                .cloned()
                .ok_or(SourceError::InvalidResponse("missing_remote_image"))?
        } else if let Some(name) = &reference.archive_name {
            archive_bindings
                .get(name)
                .cloned()
                .ok_or(SourceError::Unsupported("missing_chapter_image"))?
        } else {
            return Err(SourceError::InvalidResponse("image_reference"));
        };
        collected.admit_binding(&reference.source)?;
        result.source_to_asset.insert(reference.source.clone(), id);
    }
    let referenced_ids = result
        .source_to_asset
        .values()
        .cloned()
        .collect::<BTreeSet<_>>();
    collected.retain(|asset| referenced_ids.contains(&asset.id));
    result.assets = collected.into_assets();
    Ok(result)
}

pub fn decode_archive(bytes: &[u8]) -> Result<Vec<ImageAsset>> {
    Ok(decode_archive_accumulator(bytes)?.into_assets())
}

fn decode_archive_accumulator(bytes: &[u8]) -> Result<AssetAccumulator> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(SourceError::ResponseTooLarge);
    }
    if bytes.starts_with(b"PK") {
        decode_zip_accumulator(
            bytes,
            MAX_COMPRESSED_WORK_BYTES as u64,
            MemberLimits::default(),
        )
    } else if bytes.starts_with(&[0x1f, 0x8b]) {
        Err(SourceError::Unsupported("compressed_chapter_archive"))
    } else {
        decode_tar_accumulator(bytes, MemberLimits::default())
    }
}

#[cfg(test)]
fn decode_zip_with_compressed_limit(
    bytes: &[u8],
    maximum_compressed_work: u64,
) -> Result<Vec<ImageAsset>> {
    Ok(
        decode_zip_accumulator(bytes, maximum_compressed_work, MemberLimits::default())?
            .into_assets(),
    )
}

fn decode_zip_accumulator(
    bytes: &[u8],
    maximum_compressed_work: u64,
    limits: MemberLimits,
) -> Result<AssetAccumulator> {
    #[cfg(test)]
    let archive =
        image_zip_extra_budget_tests::construct_archive(Cursor::new(bytes), MAX_ARCHIVE_ENTRIES);
    #[cfg(not(test))]
    let archive = zip::ZipArchive::new_with_file_limit(Cursor::new(bytes), MAX_ARCHIVE_ENTRIES);
    let mut archive = archive.map_err(|error| match error {
        zip::result::ZipError::UnsupportedArchive(
            zip::result::ZipError::FILE_COUNT_LIMIT_EXCEEDED
            | zip::result::ZipError::METADATA_LIMIT_EXCEEDED,
        ) => SourceError::ResponseTooLarge,
        _ => SourceError::InvalidResponse("chapter_zip"),
    })?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(SourceError::ResponseTooLarge);
    }
    let mut collected = AssetAccumulator {
        limits,
        ..AssetAccumulator::default()
    };
    let mut expanded = 0usize;
    let mut compressed_work = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| SourceError::InvalidResponse("chapter_zip_entry"))?;
        safe_archive_name(entry.name())?;
        if entry.is_dir() {
            continue;
        }
        let name = collected.admit_name(&[entry.name()])?;
        if entry.enclosed_name().is_none()
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(SourceError::InvalidResponse("archive_path"));
        }
        if entry.size() > MAX_IMAGE_BYTES as u64 {
            return Err(SourceError::ResponseTooLarge);
        }
        expanded = expanded.saturating_add(entry.size() as usize);
        if expanded > MAX_EXPANDED_BYTES {
            return Err(SourceError::ResponseTooLarge);
        }
        // Distinct directory records may revisit the same compressed region,
        // and a legal empty-output stream still requires decompression work.
        // Directories were skipped above because their payload is never read.
        compressed_work = compressed_work
            .checked_add(entry.compressed_size())
            .filter(|bytes| *bytes <= maximum_compressed_work)
            .ok_or(SourceError::ResponseTooLarge)?;
        let mut body = Vec::with_capacity(entry.size() as usize);
        (&mut entry)
            .take(MAX_IMAGE_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|_| SourceError::Integrity)?;
        if body.len() > MAX_IMAGE_BYTES || body.len() as u64 != entry.size() {
            return Err(SourceError::Integrity);
        }
        collected.add_image(body, vec![name])?;
    }
    Ok(collected)
}

fn decode_tar_accumulator(bytes: &[u8], limits: MemberLimits) -> Result<AssetAccumulator> {
    if bytes.len() < 512 {
        return Err(SourceError::InvalidResponse("chapter_tar"));
    }
    let mut collected = AssetAccumulator {
        limits,
        ..AssetAccumulator::default()
    };
    let mut offset = 0usize;
    let mut expanded = 0usize;
    let mut entries = 0usize;
    let mut terminated = false;
    while offset
        .checked_add(512)
        .is_some_and(|end| end <= bytes.len())
    {
        let header = &bytes[offset..offset + 512];
        if header.iter().all(|byte| *byte == 0) {
            terminated = true;
            break;
        }
        entries += 1;
        if entries > MAX_ARCHIVE_ENTRIES {
            return Err(SourceError::ResponseTooLarge);
        }
        let expected = octal(&header[148..156])?;
        let actual = header
            .iter()
            .enumerate()
            .map(|(index, byte)| {
                if (148..156).contains(&index) {
                    32u64
                } else {
                    u64::from(*byte)
                }
            })
            .sum::<u64>();
        if expected != actual {
            return Err(SourceError::Integrity);
        }
        let size = octal(&header[124..136])?;
        if size > MAX_IMAGE_BYTES as u64 {
            return Err(SourceError::ResponseTooLarge);
        }
        let base = tar_string(&header[..100])?;
        let prefix = if header[257..].starts_with(b"ustar") {
            tar_string(&header[345..500])?
        } else {
            ""
        };
        if base.is_empty() && prefix.is_empty() {
            return Err(SourceError::InvalidResponse("archive_path"));
        }
        let name = if prefix.is_empty() {
            collected.admit_name(&[base])?
        } else {
            collected.admit_name(&[prefix, "/", base])?
        };
        safe_archive_name(&name.0)?;
        let typeflag = header[156];
        if matches!(typeflag, b'1' | b'2') {
            return Err(SourceError::InvalidResponse("archive_link"));
        }
        if matches!(typeflag, b'x' | b'g' | b'L' | b'K' | b'S') {
            return Err(SourceError::Unsupported("chapter_tar_extension"));
        }
        let start = offset + 512;
        let end = start
            .checked_add(size as usize)
            .ok_or(SourceError::ResponseTooLarge)?;
        if end > bytes.len() {
            return Err(SourceError::Integrity);
        }
        expanded = expanded.saturating_add(size as usize);
        if expanded > MAX_EXPANDED_BYTES {
            return Err(SourceError::ResponseTooLarge);
        }
        if matches!(typeflag, 0 | b'0') && size > 0 {
            collected.add_image(bytes[start..end].to_vec(), vec![name])?;
        }
        let padding = (512 - size as usize % 512) % 512;
        offset = end
            .checked_add(padding)
            .ok_or(SourceError::ResponseTooLarge)?;
        if offset > bytes.len() {
            return Err(SourceError::Integrity);
        }
    }
    if !terminated && offset != bytes.len() {
        return Err(SourceError::Integrity);
    }
    Ok(collected)
}

fn octal(bytes: &[u8]) -> Result<u64> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| SourceError::InvalidResponse("tar_number"))?
        .trim_matches(['\0', ' ']);
    if value.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(value, 8).map_err(|_| SourceError::InvalidResponse("tar_number"))
}

fn tar_string(bytes: &[u8]) -> Result<&str> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end])
        .map(str::trim_end)
        .map_err(|_| SourceError::InvalidResponse("archive_name"))
}

fn safe_archive_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 4096
        || name.starts_with(['/', '\\'])
        || name.contains(['\\', ':'])
        || name.chars().any(char::is_control)
        || name.split('/').any(|part| part == "..")
    {
        return Err(SourceError::InvalidResponse("archive_path"));
    }
    Ok(())
}

fn image_asset(bytes: Vec<u8>, source_names: Vec<String>) -> Result<Option<ImageAsset>> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(SourceError::ResponseTooLarge);
    }
    let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("png", "image/png"))
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(("jpg", "image/jpeg"))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(("gif", "image/gif"))
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
        Some(("webp", "image/webp"))
    } else {
        None
    };
    let Some((extension, media_type)) = format else {
        return Ok(None);
    };
    let mut reader = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| SourceError::InvalidResponse("image_format"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_EDGE);
    limits.max_image_height = Some(MAX_IMAGE_EDGE);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    // This network stage inspects headers only. Full decoding and grayscale work
    // run once in book_assets::prepare_downloaded on a blocking worker.
    let (width, height) = reader.into_dimensions().map_err(|error| match error {
        image::ImageError::Limits(_) => SourceError::ResponseTooLarge,
        _ => SourceError::InvalidResponse("image_header"),
    })?;
    if width == 0
        || height == 0
        || width > MAX_IMAGE_EDGE
        || height > MAX_IMAGE_EDGE
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return Err(SourceError::ResponseTooLarge);
    }
    Ok(Some(ImageAsset {
        id: hex::encode(Sha256::digest(&bytes)),
        media_type: media_type.into(),
        extension: extension.into(),
        source_names,
        bytes,
    }))
}

#[cfg(test)]
#[path = "image_member_budget_tests.rs"]
mod image_member_budget_tests;

#[cfg(test)]
#[path = "image_zip_extra_budget_tests.rs"]
mod image_zip_extra_budget_tests;

#[cfg(test)]
pub(crate) fn with_zip_retention_test_limits<T>(
    limits: zip::read::MetadataRetentionLimits,
    payload_size: usize,
    run: impl FnOnce() -> T,
) -> T {
    image_zip_extra_budget_tests::with_retention_limits(limits, payload_size, run)
}

#[cfg(test)]
pub(crate) fn zip_retention_usage_for_test() -> zip::read::MetadataRetentionUsage {
    image_zip_extra_budget_tests::retention_usage()
}

#[cfg(test)]
pub(crate) fn zip_retention_payload_copies_for_test() -> usize {
    image_zip_extra_budget_tests::rejected_payload_copies()
}

#[cfg(test)]
#[path = "zip_timestamp_budget_tests.rs"]
pub(crate) mod zip_timestamp_budget_tests;

#[cfg(test)]
#[path = "image_svg_guard_tests.rs"]
pub(crate) mod image_svg_guard_tests;

#[cfg(test)]
#[path = "image_style_budget_tests.rs"]
mod image_style_budget_tests;

#[cfg(test)]
#[path = "image_article_url_budget_tests.rs"]
mod image_article_url_budget_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn png() -> Vec<u8> {
        png_shade(255)
    }

    fn png_shade(value: u8) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(1, 1, image::Luma([value])))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    fn padded_png(length: usize) -> Vec<u8> {
        let original = png();
        let end = original.len() - 12;
        assert_eq!(&original[end + 4..end + 8], b"IEND");
        let padding = length.checked_sub(original.len() + 12).unwrap();
        let mut bytes = Vec::with_capacity(length);
        bytes.extend_from_slice(&original[..end]);
        bytes.extend_from_slice(&(padding as u32).to_be_bytes());
        let crc_start = bytes.len();
        // A private ancillary, safe-to-copy chunk keeps the fixture a complete
        // PNG rather than relying on a decoder accepting trailing junk.
        bytes.extend_from_slice(b"ruSt");
        bytes.resize(bytes.len() + padding, 0);
        let mut table = [0_u32; 256];
        for (index, entry) in table.iter_mut().enumerate() {
            let mut crc = index as u32;
            for _ in 0..8 {
                crc = if crc & 1 == 0 {
                    crc >> 1
                } else {
                    (crc >> 1) ^ 0xedb8_8320
                };
            }
            *entry = crc;
        }
        let mut crc = !0_u32;
        for byte in &bytes[crc_start..] {
            crc = table[((crc ^ u32::from(*byte)) & 0xff) as usize] ^ (crc >> 8);
        }
        bytes.extend_from_slice(&(!crc).to_be_bytes());
        bytes.extend_from_slice(&original[end..]);
        assert_eq!(bytes.len(), length);
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1, 1));
        bytes
    }

    fn remote_plan(count: usize) -> ChapterImagePlan {
        let html = (0..count)
            .map(|index| format!("<img src='https://cdn.example/image-{index}.png'>"))
            .collect::<String>();
        make_plan("b", "c", &html, None).unwrap()
    }

    #[test]
    fn distinct_urls_with_one_digest_exhaust_actual_remote_fetch_work() {
        let plan = remote_plan(MAX_REMOTE_FETCH_BYTES / MAX_IMAGE_BYTES + 1);
        let image = padded_png(MAX_IMAGE_BYTES);
        let calls = std::cell::Cell::new(0);
        let error = ready(acquire_with(&plan, |_, maximum| {
            calls.set(calls.get() + 1);
            assert_eq!(maximum, MAX_IMAGE_BYTES);
            let bytes = image.clone();
            async move { Ok(bytes) }
        }))
        .unwrap_err();
        assert!(matches!(error, SourceError::ResponseTooLarge));
        assert!(!error.requires_login());
        assert!(!error.retryable());
        assert_eq!(calls.get(), MAX_REMOTE_FETCH_BYTES / MAX_IMAGE_BYTES);
    }

    #[test]
    fn exact_remote_fetch_work_limit_keeps_normal_digest_deduplication() {
        let count = MAX_REMOTE_FETCH_BYTES / MAX_IMAGE_BYTES;
        let plan = remote_plan(count);
        let image = padded_png(MAX_IMAGE_BYTES);
        let calls = std::cell::Cell::new(0);
        let result = ready(acquire_with(&plan, |_, maximum| {
            calls.set(calls.get() + 1);
            assert_eq!(maximum, MAX_IMAGE_BYTES);
            let bytes = image.clone();
            async move { Ok(bytes) }
        }))
        .unwrap();
        assert_eq!(calls.get(), count);
        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.assets[0].bytes.len(), MAX_IMAGE_BYTES);
        assert_eq!(result.source_to_asset.len(), count);
    }

    #[test]
    fn a_fetcher_cannot_ignore_the_smaller_remaining_work_allowance() {
        let plan = remote_plan(9);
        let image = padded_png(MAX_IMAGE_BYTES);
        let half = padded_png(MAX_IMAGE_BYTES / 2);
        let too_much = padded_png(MAX_IMAGE_BYTES / 2 + 1);
        let maxima = std::cell::RefCell::new(Vec::new());
        let error = ready(acquire_with(&plan, |_, maximum| {
            let index = maxima.borrow().len();
            maxima.borrow_mut().push(maximum);
            let bytes = match index {
                0..=6 => image.clone(),
                7 => half.clone(),
                8 => too_much.clone(),
                _ => panic!("The acquisition continued after its work quota failed"),
            };
            async move { Ok(bytes) }
        }))
        .unwrap_err();
        assert!(matches!(error, SourceError::ResponseTooLarge));
        assert_eq!(maxima.borrow().len(), 9);
        assert!(
            maxima.borrow()[..8]
                .iter()
                .all(|maximum| *maximum == MAX_IMAGE_BYTES)
        );
        assert_eq!(maxima.borrow()[8], MAX_IMAGE_BYTES / 2);
        assert!(!error.requires_login());
    }

    #[test]
    fn repeated_url_bindings_reuse_one_fetch_without_virtual_work_charges() {
        let mut plan = make_plan(
            "b", "c",
            "<img src='https://cdn.example/shared.png#first'><img src='https://cdn.example/shared.png#second'>",
            None,
        ).unwrap();
        assert_eq!(plan.references.len(), 2);
        assert_eq!(plan.remote_images.len(), 1);
        let repeated = plan.remote_images[0].clone();
        plan.remote_images.resize(MAX_IMAGE_COUNT, repeated);
        let image = padded_png(MAX_IMAGE_BYTES);
        let calls = std::cell::Cell::new(0);
        let result = ready(acquire_with(&plan, |_, maximum| {
            calls.set(calls.get() + 1);
            assert_eq!(maximum, MAX_IMAGE_BYTES);
            let bytes = image.clone();
            async move { Ok(bytes) }
        }))
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.source_to_asset.len(), 2);
        assert_eq!(
            result
                .source_to_asset
                .values()
                .collect::<BTreeSet<_>>()
                .len(),
            1
        );
    }

    fn image_zip_fixture(body: &[u8]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                "images/fixture.png",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(body).unwrap();
        writer.finish().unwrap().into_inner()
    }

    fn empty_deflate_zip_fixture(name: &str, blocks: usize) -> Vec<u8> {
        assert!(blocks > 0);
        let mut compressed = Vec::new();
        for index in 0..blocks {
            compressed.extend_from_slice(&[u8::from(index + 1 == blocks), 0, 0, 0xff, 0xff]);
        }
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(&compressed).unwrap();
        let mut bytes = writer.finish().unwrap().into_inner();
        let footer = bytes.len() - 22;
        assert_eq!(&bytes[footer..footer + 4], b"PK\x05\x06");
        let central =
            u32::from_le_bytes(bytes[footer + 16..footer + 20].try_into().unwrap()) as usize;
        let local =
            u32::from_le_bytes(bytes[central + 42..central + 46].try_into().unwrap()) as usize;
        assert_eq!(
            u16::from_le_bytes(bytes[local + 6..local + 8].try_into().unwrap()) & 8,
            0
        );
        bytes[local + 4..local + 6].copy_from_slice(&20_u16.to_le_bytes());
        bytes[local + 8..local + 10].copy_from_slice(&8_u16.to_le_bytes());
        bytes[local + 14..local + 18].copy_from_slice(&0_u32.to_le_bytes());
        bytes[local + 22..local + 26].copy_from_slice(&0_u32.to_le_bytes());
        bytes[central + 6..central + 8].copy_from_slice(&20_u16.to_le_bytes());
        bytes[central + 10..central + 12].copy_from_slice(&8_u16.to_le_bytes());
        bytes[central + 16..central + 20].copy_from_slice(&0_u32.to_le_bytes());
        bytes[central + 24..central + 28].copy_from_slice(&0_u32.to_le_bytes());
        bytes
    }

    fn aliased_empty_deflate_zip_fixture() -> Vec<u8> {
        let bytes = empty_deflate_zip_fixture("first.bin", 3);
        let footer = bytes.len() - 22;
        let central =
            u32::from_le_bytes(bytes[footer + 16..footer + 20].try_into().unwrap()) as usize;
        let mut second = bytes[central..footer].to_vec();
        let name_bytes = u16::from_le_bytes(second[28..30].try_into().unwrap()) as usize;
        assert_eq!(name_bytes, "other.bin".len());
        // Both metadata records retain the same local-header offset. The pinned
        // parser permits overlap, so both reads visit the same compressed bytes.
        second[46..46 + name_bytes].copy_from_slice(b"other.bin");
        let mut output = bytes[..footer].to_vec();
        output.extend_from_slice(&second);
        let mut end = bytes[footer..].to_vec();
        end[8..10].copy_from_slice(&2_u16.to_le_bytes());
        end[10..12].copy_from_slice(&2_u16.to_le_bytes());
        end[12..16].copy_from_slice(&u32::try_from(2 * second.len()).unwrap().to_le_bytes());
        output.extend_from_slice(&end);
        output
    }

    #[test]
    fn aliased_empty_zip_members_consume_compressed_work_for_each_read() {
        let bytes = aliased_empty_deflate_zip_fixture();
        let assets = decode_zip_with_compressed_limit(&bytes, 30).unwrap();
        assert!(assets.is_empty());
        let error = decode_zip_with_compressed_limit(&bytes, 29).unwrap_err();
        assert!(matches!(error, SourceError::ResponseTooLarge));
        assert!(!error.requires_login());
        assert!(!error.retryable());
    }

    #[test]
    fn unread_directory_payload_does_not_consume_compressed_work() {
        let bytes = empty_deflate_zip_fixture("ignored/", 3);
        assert!(
            decode_zip_with_compressed_limit(&bytes, 0)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn ordinary_png_zip_accepts_its_exact_compressed_work_allowance() {
        let image = png();
        let bytes = image_zip_fixture(&image);
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let compressed = archive.by_index(0).unwrap().compressed_size();
        assert!(compressed > 0);
        let assets = decode_zip_with_compressed_limit(&bytes, compressed).unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].bytes, image);
        assert!(matches!(
            decode_zip_with_compressed_limit(&bytes, compressed - 1),
            Err(SourceError::ResponseTooLarge)
        ));
    }

    #[test]
    fn chapter_zip_count_rejection_is_a_permanent_resource_error_without_login_expiry() {
        let mut bytes = image_zip_fixture(&png());
        let footer = bytes.len() - 22;
        assert_eq!(&bytes[footer..footer + 4], b"PK\x05\x06");
        let declared_count = (MAX_ARCHIVE_ENTRIES + 1) as u16;
        bytes[footer + 8..footer + 10].copy_from_slice(&declared_count.to_le_bytes());
        bytes[footer + 10..footer + 12].copy_from_slice(&declared_count.to_le_bytes());
        // Only the production bounded decoder sees this declaration. The
        // physical fixture has one image, not thousands of indexed payloads.
        let error = decode_archive(&bytes).unwrap_err();
        assert!(matches!(error, SourceError::ResponseTooLarge));
        assert!(!error.requires_login());
        assert!(!error.retryable());
        let malformed = decode_archive(b"PK malformed fixture").unwrap_err();
        assert!(matches!(
            malformed,
            SourceError::InvalidResponse("chapter_zip")
        ));
        assert!(!malformed.requires_login());
        assert!(!malformed.retryable());
    }

    #[test]
    fn chapter_zip_eocd_fallback_retains_images_after_a_rejected_count_candidate() {
        let image = png();
        let mut bytes = image_zip_fixture(&image);
        let footer = bytes.len() - 22;
        assert_eq!(&bytes[footer..footer + 4], b"PK\x05\x06");
        let mut fake_footer = bytes[footer..].to_vec();
        let declared_count = (MAX_ARCHIVE_ENTRIES + 1) as u16;
        fake_footer[8..10].copy_from_slice(&declared_count.to_le_bytes());
        fake_footer[10..12].copy_from_slice(&declared_count.to_le_bytes());
        bytes[footer + 20..footer + 22].copy_from_slice(&(fake_footer.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&fake_footer);
        let assets = decode_archive(&bytes).unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].bytes, image);
        assert_eq!(assets[0].id, hex::encode(Sha256::digest(&image)));
    }

    fn image_zip_with_failed_directory_candidates(image: &[u8], candidates: usize) -> Vec<u8> {
        let mut bytes = image_zip_fixture(image);
        let footer = bytes.len() - 22;
        assert_eq!(&bytes[footer..footer + 4], b"PK\x05\x06");
        let mut fake_footer = bytes[footer..].to_vec();
        let real_count = u16::from_le_bytes(fake_footer[10..12].try_into().unwrap());
        let fake_count = real_count.checked_add(1).unwrap();
        assert!(usize::from(fake_count) < MAX_ARCHIVE_ENTRIES);
        fake_footer[8..10].copy_from_slice(&fake_count.to_le_bytes());
        fake_footer[10..12].copy_from_slice(&fake_count.to_le_bytes());
        let appended = candidates.checked_mul(fake_footer.len()).unwrap();
        bytes[footer + 20..footer + 22]
            .copy_from_slice(&u16::try_from(appended).unwrap().to_le_bytes());
        for index in 0..candidates {
            let remaining_comment = (candidates - index - 1) * fake_footer.len();
            fake_footer[20..22]
                .copy_from_slice(&u16::try_from(remaining_comment).unwrap().to_le_bytes());
            // Each footer is structurally valid but promises one central record
            // more than exists, so parsing falls back to the previous candidate.
            bytes.extend_from_slice(&fake_footer);
        }
        bytes
    }

    #[test]
    fn chapter_zip_metadata_work_exhaustion_remains_a_resource_error_without_auth_retry() {
        let image = png();
        let ordinary = image_zip_with_failed_directory_candidates(&image, 2);
        assert_eq!(decode_archive(&ordinary).unwrap()[0].bytes, image);

        let bytes = image_zip_with_failed_directory_candidates(&image, 65);
        assert!(
            bytes.len() < 4096,
            "The fallback fixture must remain a small archive"
        );
        let error = decode_archive(&bytes).unwrap_err();
        assert!(matches!(error, SourceError::ResponseTooLarge));
        assert!(
            !error.requires_login(),
            "A local archive quota must not invalidate account credentials"
        );
        assert!(
            !error.retryable(),
            "Repeating the same metadata work cannot repair this archive quota"
        );

        // The real directory remains available behind the failed candidates;
        // a modest explicit work allowance establishes that this is a quota
        // rejection rather than a malformed archive without a usable footer.
        let archive = zip::ZipArchive::new_with_limits(
            Cursor::new(&bytes),
            MAX_ARCHIVE_ENTRIES,
            8 * 1024 * 1024,
            128,
        )
        .unwrap();
        assert_eq!(archive.len(), 1);
    }

    fn png_with_header_dimensions(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = png();
        assert_eq!(&bytes[12..16], b"IHDR");
        bytes[16..20].copy_from_slice(&width.to_be_bytes());
        bytes[20..24].copy_from_slice(&height.to_be_bytes());
        let mut crc = 0xffff_ffffu32;
        for byte in &bytes[12..29] {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        bytes[29..33].copy_from_slice(&(!crc).to_be_bytes());
        bytes
    }

    fn ready<T>(future: impl Future<Output = T>) -> T {
        struct NoopWake;
        impl std::task::Wake for NoopWake {
            fn wake(self: std::sync::Arc<Self>) {}
        }
        let waker = std::task::Waker::from(std::sync::Arc::new(NoopWake));
        let mut context = std::task::Context::from_waker(&waker);
        let mut future = Box::pin(future);
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(result) => result,
            std::task::Poll::Pending => {
                panic!("A fixture transport unexpectedly waited for external work")
            }
        }
    }

    fn tar(name: &str, body: &[u8]) -> Vec<u8> {
        let mut header = vec![0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[124..136].copy_from_slice(format!("{:011o}\0", body.len()).as_bytes());
        header[148..156].fill(b' ');
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        let sum = header.iter().map(|byte| u64::from(*byte)).sum::<u64>();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        header.extend_from_slice(body);
        while header.len() % 512 != 0 {
            header.push(0);
        }
        header.extend_from_slice(&[0; 1024]);
        header
    }

    fn tar_images(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut archive = Vec::new();
        for (name, body) in entries {
            let entry = tar(name, body);
            archive.extend_from_slice(&entry[..entry.len() - 1024]);
        }
        archive.extend_from_slice(&[0; 1024]);
        archive
    }

    fn acquire_fixture_archive(
        plan: &ChapterImagePlan,
        archive: Vec<u8>,
    ) -> Result<DownloadedChapterImages> {
        ready(acquire_with(plan, |_, _| {
            let body = archive.clone();
            async move { Ok(body) }
        }))
    }

    #[test]
    fn image_plan_uses_catalog_archive_and_inline_references() {
        let plan=make_plan("fixture-book","7","<img src='../images/plot.png?x=1'><img src='//cdn.example/remote.jpg?token=fixture'><img src='../images/plot.png?x=1'>",Some("/resources/chapter.tar")).unwrap();
        assert_eq!(
            plan.archive.as_ref().unwrap().url,
            "https://weread.qq.com/resources/chapter.tar"
        );
        assert_eq!(
            plan.archive.as_ref().unwrap().referer,
            reader_url("fixture-book", Some("7")).unwrap()
        );
        assert_eq!(plan.references.len(), 2);
        assert_eq!(
            plan.references[0].archive_name.as_deref(),
            Some("../images/plot.png?x=1")
        );
        assert_eq!(plan.remote_images[0].referer, "https://weread.qq.com/");
        assert!(!format!("{plan:?}").contains("token"));
    }

    #[test]
    fn unsupported_references_do_not_form_a_successful_empty_plan() {
        assert!(make_plan("b", "c", "<img src='123456'>", None).is_err());
        assert!(make_plan("b", "c", "<img src='data:image/png;base64,fixture'>", None).is_err());
        assert!(
            make_plan(
                "b",
                "c",
                "<svg><image href='picture.png'/></svg>",
                Some("/archive")
            )
            .is_err()
        );
        assert!(make_plan("b", "c", "<img src='http://cdn.example/picture.png'>", None).is_err());
        assert!(
            make_plan("b", "c", "<p>Text only.</p>", None)
                .unwrap()
                .references
                .is_empty()
        );
    }

    #[test]
    fn archive_bytes_are_validated_without_extracting_paths() {
        let bytes = png();
        let archive = tar("images/fixture.png", &bytes);
        let assets = decode_archive(&archive).unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].bytes, bytes);
        assert_eq!(assets[0].source_names, ["images/fixture.png"]);
        assert_eq!(assets[0].id, hex::encode(Sha256::digest(&bytes)));
        let mut corrupt = archive.clone();
        corrupt[10] ^= 1;
        assert!(matches!(
            decode_archive(&corrupt),
            Err(SourceError::Integrity)
        ));
        assert!(decode_archive(&tar("../escape.png", &bytes)).is_err());
    }

    #[test]
    fn zip_images_match_tar_identity_and_reject_traversal() {
        fn zip(name: &str, body: &[u8]) -> Vec<u8> {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(body).unwrap();
            writer.finish().unwrap().into_inner()
        }
        let bytes = png();
        let assets = decode_archive(&zip("folder/fixture.png", &bytes)).unwrap();
        assert_eq!(assets[0].id, hex::encode(Sha256::digest(&bytes)));
        assert!(decode_archive(&zip("../escape.png", &bytes)).is_err());
        assert!(
            decode_archive(&zip("folder/not-image.txt", b"Fixture"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn current_book_namespace_binds_a_flat_converter_member_to_its_chapter_reference() {
        let plan = make_plan(
            "fixture-book",
            "chapter",
            "<img src='../Images/note.png'><img src='../Images/./note.png#marker'>",
            Some("/fixture.tar"),
        )
        .unwrap();
        assert_eq!(plan.archive_namespace.as_deref(), Some("CB_fixture-book_"));
        assert!(!format!("{plan:?}").contains("fixture-book"));
        let archive = tar("CB_fixture-book_note.png", &png());
        let result = acquire_fixture_archive(&plan, archive.clone()).unwrap();
        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.source_to_asset.len(), 2);
        assert_eq!(
            result.source_to_asset["../Images/note.png"],
            result.assets[0].id
        );
        assert_eq!(
            result.source_to_asset["../Images/./note.png#marker"],
            result.assets[0].id
        );

        // Previously serialized plans retain strict matching until a current
        // book explicitly supplies the observed converter namespace.
        let mut old_plan = serde_json::to_value(&plan).unwrap();
        old_plan
            .as_object_mut()
            .unwrap()
            .remove("archive_namespace");
        let old_plan: ChapterImagePlan = serde_json::from_value(old_plan).unwrap();
        assert!(old_plan.archive_namespace.is_none());
        assert!(matches!(
            acquire_fixture_archive(&old_plan, archive),
            Err(SourceError::Unsupported("missing_chapter_image"))
        ));
    }

    #[test]
    fn converter_namespaces_never_bind_resources_from_another_book() {
        let plan = make_plan(
            "other-fixture-book",
            "chapter",
            "<img src='../Images/note.png'>",
            Some("/fixture.tar"),
        )
        .unwrap();
        assert!(matches!(
            acquire_fixture_archive(&plan, tar("CB_fixture-book_note.png", &png())),
            Err(SourceError::Unsupported("missing_chapter_image"))
        ));
    }

    #[test]
    fn different_qualified_references_cannot_collapse_into_one_scoped_alias() {
        let plan = make_plan(
            "fixture-book",
            "chapter",
            "<img src='../Images/note.png'><img src='other/note.png'>",
            Some("/fixture.tar"),
        )
        .unwrap();
        assert!(matches!(
            acquire_fixture_archive(&plan, tar("CB_fixture-book_note.png", &png())),
            Err(SourceError::Unsupported("ambiguous_chapter_image"))
        ));
    }

    #[test]
    fn scoped_aliases_count_other_archive_paths_even_when_content_is_identical() {
        let plan = make_plan(
            "fixture-book",
            "chapter",
            "<img src='../Images/note.png'>",
            Some("/fixture.tar"),
        )
        .unwrap();
        for ordinary in [png_shade(255), png_shade(0)] {
            let archive = tar_images(&[
                ("CB_fixture-book_note.png", png_shade(255)),
                ("Images/note.png", ordinary),
            ]);
            assert!(matches!(
                acquire_fixture_archive(&plan, archive),
                Err(SourceError::Unsupported("ambiguous_chapter_image"))
            ));
        }
    }

    #[test]
    fn ordinary_exact_paths_take_precedence_over_a_scoped_converter_alias() {
        let exact = png_shade(0);
        let archive = tar_images(&[
            ("CB_fixture-book_note.png", png_shade(255)),
            ("Images/note.png", exact.clone()),
        ]);
        let plan = make_plan(
            "fixture-book",
            "chapter",
            "<img src='Images/note.png'>",
            Some("/fixture.tar"),
        )
        .unwrap();
        let result = acquire_fixture_archive(&plan, archive).unwrap();
        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.assets[0].bytes, exact);
        assert_eq!(
            result.source_to_asset["Images/note.png"],
            result.assets[0].id
        );
    }

    #[test]
    fn scoped_converter_aliases_preserve_query_and_case_identity() {
        let archive = tar("CB_fixture-book_note.png?version=1", &png());
        let plan = make_plan(
            "fixture-book",
            "chapter",
            "<img src='../Images/note.png?version=1#marker'>",
            Some("/fixture.tar"),
        )
        .unwrap();
        assert!(acquire_fixture_archive(&plan, archive.clone()).is_ok());
        for source in [
            "../Images/note.png?version=2",
            "../Images/note.png",
            "../Images/Note.png?version=1",
        ] {
            let html = format!("<img src='{source}'>");
            let plan = make_plan("fixture-book", "chapter", &html, Some("/fixture.tar")).unwrap();
            assert!(matches!(
                acquire_fixture_archive(&plan, archive.clone()),
                Err(SourceError::Unsupported("missing_chapter_image"))
            ));
        }
    }

    #[test]
    fn archive_bindings_require_exact_directories_for_qualified_sources() {
        let assets = decode_archive(&tar("images/a.png", &png())).unwrap();
        let index = ArchiveIndex::new(&assets).unwrap();
        assert_eq!(
            index.resolve("./images/./a.png#thumb").unwrap(),
            assets[0].id
        );
        assert!(matches!(
            index.resolve("other/a.png"),
            Err(SourceError::Unsupported("missing_chapter_image"))
        ));
        assert!(matches!(
            index.resolve("../images/a.png"),
            Err(SourceError::Unsupported("missing_chapter_image"))
        ));
        let plan = make_plan("b", "c", "<img src='other/a.png'>", Some("/chapter.tar")).unwrap();
        assert!(
            index
                .resolve(plan.references[0].archive_name.as_deref().unwrap())
                .is_err()
        );
    }

    #[test]
    fn bare_sources_require_one_canonical_archive_path_even_for_equal_bytes() {
        let mut assets = decode_archive(&tar("images/a.png", &png())).unwrap();
        assert_eq!(
            ArchiveIndex::new(&assets)
                .unwrap()
                .resolve("a.png")
                .unwrap(),
            assets[0].id
        );
        // Content deduplication must not erase the ambiguity of two different resource paths.
        assets[0].source_names.push("other/a.png".into());
        let index = ArchiveIndex::new(&assets).unwrap();
        assert!(matches!(
            index.resolve("a.png"),
            Err(SourceError::Unsupported("ambiguous_chapter_image"))
        ));
        assert_eq!(index.resolve("images/a.png").unwrap(), assets[0].id);
    }

    #[test]
    fn archive_query_variants_are_distinct_aliases() {
        let assets = decode_archive(&tar("images/a.png?version=1", &png())).unwrap();
        let index = ArchiveIndex::new(&assets).unwrap();
        assert_eq!(
            index.resolve("./images/a.png?version=1#fragment").unwrap(),
            assets[0].id
        );
        assert_eq!(index.resolve("a.png?version=1").unwrap(), assets[0].id);
        assert!(matches!(
            index.resolve("images/a.png?version=2"),
            Err(SourceError::Unsupported("missing_chapter_image"))
        ));
        assert!(matches!(
            index.resolve("a.png"),
            Err(SourceError::Unsupported("missing_chapter_image"))
        ));
        let plan = make_plan(
            "b",
            "c",
            "<img src='./images/./a.png?version=1#fragment'>",
            Some("/chapter.tar"),
        )
        .unwrap();
        assert_eq!(
            plan.references[0].archive_name.as_deref(),
            Some("images/a.png?version=1")
        );
        assert_eq!(
            plan.references[0].source,
            "./images/./a.png?version=1#fragment"
        );
    }

    #[test]
    fn empty_references_never_contact_an_unrelated_archive() {
        let calls = std::cell::Cell::new(0);
        let plan = ChapterImagePlan {
            archive: Some(ImageRequest {
                url: "https://cdn.example/unrelated.tar".into(),
                referer: "https://weread.qq.com/".into(),
                source: String::new(),
            }),
            ..Default::default()
        };
        let result = ready(acquire_with(&plan, |_, _| {
            calls.set(calls.get() + 1);
            async { Err(SourceError::Unsupported("unexpected_fixture_fetch")) }
        }))
        .unwrap();
        assert_eq!(calls.get(), 0);
        assert!(result.assets.is_empty());
        assert!(result.source_to_asset.is_empty());
        assert!(
            make_plan(
                "b",
                "c",
                "<p>Text only</p>",
                Some("http://unused.example/archive.tar")
            )
            .unwrap()
            .archive
            .is_none()
        );
    }

    #[test]
    fn remote_only_references_skip_the_archive_and_unreferenced_requests() {
        let mut plan = make_plan(
            "b",
            "c",
            "<img src='https://cdn.example/used.png'>",
            Some("http://unused.example/archive.tar"),
        )
        .unwrap();
        assert!(plan.archive.is_none());
        plan.archive = Some(ImageRequest {
            url: "https://cdn.example/unrelated.tar".into(),
            referer: "https://weread.qq.com/".into(),
            source: String::new(),
        });
        plan.remote_images.push(ImageRequest {
            url: "https://cdn.example/unused.png".into(),
            referer: "https://weread.qq.com/".into(),
            source: String::new(),
        });
        let calls = std::cell::RefCell::new(Vec::new());
        let image = png();
        let result = ready(acquire_with(&plan, |request, _| {
            calls.borrow_mut().push(request.url.clone());
            let body = image.clone();
            async move {
                if request.url == "https://cdn.example/used.png" {
                    Ok(body)
                } else {
                    Err(SourceError::Unsupported("unexpected_fixture_fetch"))
                }
            }
        }))
        .unwrap();
        assert_eq!(*calls.borrow(), vec!["https://cdn.example/used.png"]);
        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.source_to_asset.len(), 1);
    }

    #[test]
    fn only_assets_referenced_by_this_chapter_are_returned() {
        let used = png_shade(255);
        let unused = png_shade(0);
        let mut archive = tar("images/used.png", &used);
        archive.truncate(archive.len() - 1024);
        archive.extend_from_slice(&tar("images/unused.png", &unused));
        let plan = make_plan(
            "b",
            "c",
            "<img src='images/used.png'>",
            Some("/chapter.tar"),
        )
        .unwrap();
        let result = ready(acquire_with(&plan, |_, _| {
            let body = archive.clone();
            async move { Ok(body) }
        }))
        .unwrap();
        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.assets[0].bytes, used);
        assert_eq!(
            result.source_to_asset.get("images/used.png"),
            Some(&result.assets[0].id)
        );
        assert!(
            result
                .assets
                .iter()
                .all(|asset| asset.id != hex::encode(Sha256::digest(&unused)))
        );
    }

    #[test]
    fn source_image_headers_enforce_side_and_pixel_limits() {
        assert!(matches!(
            image_asset(png_with_header_dimensions(8193, 1), Vec::new()),
            Err(SourceError::ResponseTooLarge)
        ));
        assert!(matches!(
            image_asset(png_with_header_dimensions(4001, 4000), Vec::new()),
            Err(SourceError::ResponseTooLarge)
        ));
    }

    #[test]
    fn acquired_headers_are_not_a_claim_of_decoded_raster_readiness() {
        // The IHDR is valid but the retained IDAT contains only the original 1x1 raster.
        let bytes = png_with_header_dimensions(2, 2);
        assert!(image_asset(bytes.clone(), Vec::new()).unwrap().is_some());
        assert!(
            image::ImageReader::new(Cursor::new(bytes))
                .with_guessed_format()
                .unwrap()
                .decode()
                .is_err()
        );
    }
}
