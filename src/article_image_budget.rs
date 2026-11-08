//! Borrowed admission before article image URL parsing, hashing, and owned output.

use super::ArticleImage;

const MAX_RAW_URL_BYTES: usize = 4096;
const MAX_NORMALIZED_URL_BYTES: usize = 16 * 1024;
const MAX_OCCURRENCES: usize = 2048;
const MAX_URL_CANDIDATES: usize = 8192;
const MAX_CAPTION_BYTES: usize = 16 * 1024;
const MAX_ID_BYTES: usize = 4096;
const MAX_WORK_BYTES: usize = 64 * 1024 * 1024;
const MAX_METADATA_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub raw_url_bytes: usize,
    pub normalized_url_bytes: usize,
    pub occurrences: usize,
    pub url_candidates: usize,
    pub caption_bytes: usize,
    pub id_bytes: usize,
    pub work_bytes: usize,
    pub metadata_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            raw_url_bytes: MAX_RAW_URL_BYTES,
            normalized_url_bytes: MAX_NORMALIZED_URL_BYTES,
            occurrences: MAX_OCCURRENCES,
            url_candidates: MAX_URL_CANDIDATES,
            caption_bytes: MAX_CAPTION_BYTES,
            id_bytes: MAX_ID_BYTES,
            work_bytes: MAX_WORK_BYTES,
            metadata_bytes: MAX_METADATA_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageLimitKind {
    UrlBytes,
    Occurrences,
    Caption,
    Work,
    Metadata,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("article image {kind:?} exceeds its allowance (limit {limit})")]
pub struct ImageLimitError {
    pub kind: ImageLimitKind,
    pub used: usize,
    pub requested: usize,
    pub limit: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Usage {
    pub records: usize,
    pub url_candidates: usize,
    pub url_input_bytes: usize,
    pub caption_bytes: usize,
    pub work_bytes: usize,
    pub metadata_bytes: usize,
    pub url_parse_calls: usize,
    pub url_input_copies: usize,
    pub url_output_copies: usize,
    pub url_hashes: usize,
}

pub(super) struct AdmittedUrl<'a> {
    raw: &'a str,
    capacity: usize,
}

impl<'a> AdmittedUrl<'a> {
    pub(super) fn raw(&self) -> &'a str {
        self.raw
    }
}

pub(super) struct Budget {
    limits: Limits,
    usage: Usage,
}

impl Default for Budget {
    fn default() -> Self {
        #[cfg(test)]
        let limits = super::image_budget_tests::configured_limits().unwrap_or_default();
        #[cfg(not(test))]
        let limits = Limits::default();
        let ceiling = Limits::default();
        Self {
            limits: Limits {
                raw_url_bytes: limits.raw_url_bytes.min(ceiling.raw_url_bytes),
                normalized_url_bytes: limits
                    .normalized_url_bytes
                    .min(ceiling.normalized_url_bytes),
                occurrences: limits.occurrences.min(ceiling.occurrences),
                url_candidates: limits.url_candidates.min(ceiling.url_candidates),
                caption_bytes: limits.caption_bytes.min(ceiling.caption_bytes),
                id_bytes: limits.id_bytes.min(ceiling.id_bytes),
                work_bytes: limits.work_bytes.min(ceiling.work_bytes),
                metadata_bytes: limits.metadata_bytes.min(ceiling.metadata_bytes),
            },
            usage: Usage::default(),
        }
    }
}

impl Budget {
    pub(super) fn record(&mut self) -> Result<(), ImageLimitError> {
        let next = admitted(
            self.usage.records,
            1,
            self.limits.occurrences,
            ImageLimitKind::Occurrences,
        )?;
        // Include staged records, actual output slots, ID/index records and
        // Vec/map minima. This is a logical forecast, not allocator RSS.
        self.fee(64, 2 * std::mem::size_of::<ArticleImage>() + 256)?;
        self.usage.records = next;
        observe(self.usage);
        Ok(())
    }

    pub(super) fn records(&self) -> usize {
        self.usage.records
    }

    pub(super) fn url<'a>(&mut self, raw: &'a str) -> Result<AdmittedUrl<'a>, ImageLimitError> {
        bound(
            raw.len(),
            self.limits.raw_url_bytes,
            ImageLimitKind::UrlBytes,
        )?;
        let candidates = admitted(
            self.usage.url_candidates,
            1,
            self.limits.url_candidates,
            ImageLimitKind::Occurrences,
        )?;
        let capacity = scaled(
            raw.len().checked_add(6),
            3,
            ImageLimitKind::Metadata,
            self.limits.metadata_bytes,
        )?;
        let work = scaled(
            raw.len().checked_add(6),
            32,
            ImageLimitKind::Work,
            self.limits.work_bytes,
        )?
        .checked_add(64)
        .ok_or(self.overflow(ImageLimitKind::Work))?;
        // URI normalization temporaries and the retained Doc/inventory/index/
        // PageBlock copies are all reserved before the first URI parse or copy.
        let metadata = capacity
            .checked_mul(8)
            .and_then(|bytes| bytes.checked_add(128))
            .ok_or(self.overflow(ImageLimitKind::Metadata))?;
        self.fee(work, metadata)?;
        self.usage.url_candidates = candidates;
        self.usage.url_input_bytes = self
            .usage
            .url_input_bytes
            .checked_add(raw.len())
            .ok_or(self.overflow(ImageLimitKind::Work))?;
        observe(self.usage);
        Ok(AdmittedUrl { raw, capacity })
    }

    pub(super) fn srcset(&mut self, bytes: usize) -> Result<(), ImageLimitError> {
        self.fee(
            bytes
                .checked_mul(8)
                .ok_or(self.overflow(ImageLimitKind::Work))?,
            64,
        )
    }

    pub(super) fn caption(&mut self, bytes: usize) -> Result<(), ImageLimitError> {
        bound(bytes, self.limits.caption_bytes, ImageLimitKind::Caption)?;
        let work = bytes
            .checked_mul(12)
            .and_then(|bytes| bytes.checked_add(64))
            .ok_or(self.overflow(ImageLimitKind::Work))?;
        let metadata = bytes
            .checked_mul(6)
            .and_then(|bytes| bytes.checked_add(128))
            .ok_or(self.overflow(ImageLimitKind::Metadata))?;
        self.fee(work, metadata)?;
        self.usage.caption_bytes = self
            .usage
            .caption_bytes
            .checked_add(bytes)
            .ok_or(self.overflow(ImageLimitKind::Metadata))?;
        observe(self.usage);
        Ok(())
    }

    pub(super) fn caption_fragment(
        &mut self,
        before: usize,
        bytes: usize,
    ) -> Result<(), ImageLimitError> {
        let total = before
            .checked_add(bytes)
            .ok_or(self.overflow(ImageLimitKind::Caption))?;
        bound(total, self.limits.caption_bytes, ImageLimitKind::Caption)?;
        self.caption(bytes)
    }

    pub(super) fn conversion(&mut self, bytes: usize) -> Result<(), ImageLimitError> {
        self.fee(
            bytes
                .checked_mul(8)
                .and_then(|bytes| bytes.checked_add(64))
                .ok_or(self.overflow(ImageLimitKind::Work))?,
            128,
        )
    }

    pub(super) fn normalized(
        &self,
        url: &str,
        token: &AdmittedUrl<'_>,
    ) -> Result<(), ImageLimitError> {
        bound(
            url.len(),
            self.limits.normalized_url_bytes.min(token.capacity),
            ImageLimitKind::UrlBytes,
        )
    }

    pub(super) fn input_copy(&mut self) {
        self.usage.url_input_copies += 1;
        observe(self.usage);
    }

    pub(super) fn parse(&mut self) {
        self.usage.url_parse_calls += 1;
        observe(self.usage);
    }

    pub(super) fn output_copy(&mut self) {
        self.usage.url_output_copies += 1;
        observe(self.usage);
    }

    pub(super) fn hash(&mut self) {
        self.usage.url_hashes += 1;
        observe(self.usage);
    }

    fn fee(&mut self, work: usize, metadata: usize) -> Result<(), ImageLimitError> {
        let next_work = admitted(
            self.usage.work_bytes,
            work,
            self.limits.work_bytes,
            ImageLimitKind::Work,
        )?;
        let next_metadata = admitted(
            self.usage.metadata_bytes,
            metadata,
            self.limits.metadata_bytes,
            ImageLimitKind::Metadata,
        )?;
        self.usage.work_bytes = next_work;
        self.usage.metadata_bytes = next_metadata;
        observe(self.usage);
        Ok(())
    }

    fn overflow(&self, kind: ImageLimitKind) -> ImageLimitError {
        ImageLimitError {
            kind,
            used: if kind == ImageLimitKind::Work {
                self.usage.work_bytes
            } else {
                self.usage.metadata_bytes
            },
            requested: usize::MAX,
            limit: if kind == ImageLimitKind::Work {
                self.limits.work_bytes
            } else {
                self.limits.metadata_bytes
            },
        }
    }
}

pub(super) fn image_work_preflight(images: &[ArticleImage]) -> Result<(), ImageLimitError> {
    let mut budget = Budget::default();
    // Complete borrowed admission precedes any direct ImageWork Vec/index copy.
    for image in images {
        budget.record()?;
        bound(
            image.url.len(),
            budget.limits.normalized_url_bytes,
            ImageLimitKind::UrlBytes,
        )?;
        bound(
            image.id.len(),
            budget.limits.id_bytes,
            ImageLimitKind::Metadata,
        )?;
        bound(
            image.caption.len(),
            budget.limits.caption_bytes,
            ImageLimitKind::Caption,
        )?;
        let work = image
            .url
            .len()
            .checked_mul(32)
            .and_then(|bytes| {
                image
                    .id
                    .len()
                    .checked_mul(8)
                    .and_then(|id| bytes.checked_add(id))
            })
            .and_then(|bytes| {
                image
                    .caption
                    .len()
                    .checked_mul(12)
                    .and_then(|caption| bytes.checked_add(caption))
            })
            .ok_or(budget.overflow(ImageLimitKind::Work))?;
        let metadata = image
            .url
            .len()
            .checked_mul(8)
            .and_then(|bytes| {
                image
                    .id
                    .len()
                    .checked_mul(4)
                    .and_then(|id| bytes.checked_add(id))
            })
            .and_then(|bytes| {
                image
                    .caption
                    .len()
                    .checked_mul(6)
                    .and_then(|caption| bytes.checked_add(caption))
            })
            .ok_or(budget.overflow(ImageLimitKind::Metadata))?;
        budget.fee(work, metadata)?;
    }
    Ok(())
}

fn admitted(
    used: usize,
    requested: usize,
    limit: usize,
    kind: ImageLimitKind,
) -> Result<usize, ImageLimitError> {
    used.checked_add(requested)
        .filter(|bytes| *bytes <= limit)
        .ok_or(ImageLimitError {
            kind,
            used,
            requested,
            limit,
        })
}

fn bound(bytes: usize, maximum: usize, kind: ImageLimitKind) -> Result<(), ImageLimitError> {
    admitted(0, bytes, maximum, kind).map(|_| ())
}

fn scaled(
    value: Option<usize>,
    factor: usize,
    kind: ImageLimitKind,
    limit: usize,
) -> Result<usize, ImageLimitError> {
    value
        .and_then(|value| value.checked_mul(factor))
        .ok_or(ImageLimitError {
            kind,
            used: 0,
            requested: usize::MAX,
            limit,
        })
}

fn observe(usage: Usage) {
    #[cfg(test)]
    super::image_budget_tests::observe(usage);
    #[cfg(not(test))]
    let _ = usage;
}
