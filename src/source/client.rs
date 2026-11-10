use super::content::{ReaderSession, html_to_text, normalize_catalog_checked};
use super::protocol::{
    SKILL_VERSION, USER_AGENT, content_payload_checked as content_payload, normalize_cover_url,
    now_seconds, read_payload_checked as read_payload, reader_url_checked as reader_url,
};
use super::reader_admission::{DownloadReaderContext, ReaderTier};
use super::web_session::{WebReadAuth, web_session_changed};
use super::*;
use crate::download_diagnostics::{DownloadDiagnostics, ResponsePhase};
use reqwest::{Client, ClientBuilder, Method, RequestBuilder, Response};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

const WEB_ORIGIN: &str = "https://weread.qq.com";
const GATEWAY_URL: &str = "https://i.weread.qq.com/api/agent/gateway";
const MAX_RESPONSE_BYTES: usize = 24 * 1024 * 1024;
const MAX_PREFETCH_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_PREFETCH_CONTENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_API_CODE_BYTES: usize = 64;
pub(crate) const MAX_COOKIE_COUNT: usize = 256;
pub(crate) const MAX_COOKIE_TOTAL_BYTES: usize = 64 * 1024;
pub(crate) const MAX_COOKIE_ENTRY_BYTES: usize = 8 * 1024;
const MAX_COOKIE_UPDATE_NAMES: usize = MAX_COOKIE_COUNT * 2;
const FIRST_PARTY_IDLE_PER_HOST: usize = 4;
const HTTP_POOL_IDLE_SECONDS: u64 = 90;
const MAX_REVIEW_OUTPUT_RECORDS: usize = 4096;
const MAX_REVIEW_ID_BYTES: usize = 4096;
const MAX_REVIEW_AUTHOR_BYTES: usize = 4096;
const MAX_REVIEW_TEXT_BYTES: usize = 64 * 1024;
const MAX_REVIEW_OUTPUT_WORK: usize = 64 * 1024 * 1024;
// Include HTML/entity scans, normalized text, and worst-case temporary line
// vectors before converting even the first selected review text.
const REVIEW_TEXT_WORK_MULTIPLIER: usize = 32;
const MAX_BOOK_OUTPUT_RECORDS: usize = 8192;
const MAX_BOOK_ID_BYTES: usize = 4096;
const MAX_BOOK_SHORT_FIELD_BYTES: usize = 16 * 1024;
const MAX_BOOK_COVER_BYTES: usize = 4096;
const MAX_BOOK_INTRO_BYTES: usize = 1024 * 1024;
const MAX_BOOK_OUTPUT_WORK: usize = 64 * 1024 * 1024;
const MAX_STATISTICS_OUTPUT_RECORDS: usize = MAX_BOOK_OUTPUT_RECORDS;
const MAX_STATISTICS_KEY_BYTES: usize = MAX_BOOK_ID_BYTES;
const MAX_STATISTICS_METADATA_BYTES: usize = MAX_BOOK_OUTPUT_WORK;
const MAX_STATISTICS_OUTPUT_WORK: usize = MAX_BOOK_OUTPUT_WORK;
const MAX_BOOK_RAW_DEPTH: usize = 128;
// These are logical operation forecasts, not exact allocator/RSS or CPU caps.
// The raw forecast includes container slack, key ownership and two tree copies.
const BOOK_RAW_VALUE_BYTES: usize = 128;
const BOOK_RAW_OBJECT_BYTES: usize = 1024;
const BOOK_RAW_ARRAY_BYTES: usize = 128;
const BOOK_RAW_KEY_BYTES: usize = 128;
// Project allowances for opaque publication owners and normalized metadata,
// independent of the request's count hint and the transport's body allowance.
const MAX_MP_ACCOUNT_ID_BYTES: usize = 4096;
const MAX_MP_ACCOUNT_RECORDS: usize = 8192;
const MAX_MP_ACCOUNT_METADATA_BYTES: usize = 64 * 1024 * 1024;
const MAX_MP_ARTICLE_ID_BYTES: usize = 4096;
const MAX_MP_ARTICLE_RECORDS: usize = 8192;
const MAX_MP_ARTICLE_METADATA_BYTES: usize = 64 * 1024 * 1024;
const MP_ORIGINAL_URL_FIELDS: &[&str] = &[
    "originalUrl",
    "original_url",
    "url",
    "contentUrl",
    "content_url",
];
const MP_COVER_URL_FIELDS: &[&str] = &["pic_url", "picUrl", "cover"];
// Application allowances for retained highlight metadata, not asserted
// upstream format limits. Inspect selected borrowed fields before mapping.
const MAX_UNDERLINE_RECORDS: usize = 4096;
const MAX_UNDERLINE_QUOTE_BYTES: usize = 16 * 1024;
const MAX_UNDERLINE_RANGE_BYTES: usize = 4 * 1024;
const MAX_UNDERLINE_TOTAL_BYTES: usize = 4 * 1024 * 1024;
const UNDERLINE_TEXT_FIELDS: &[&str] = &[
    "markText",
    "bookmarkText",
    "rangeText",
    "abstract",
    "content",
    "text",
];
const CACHED_UNDERLINE_TEXT_FIELDS: &[&str] = &[
    "text",
    "markText",
    "bookmarkText",
    "rangeText",
    "abstract",
    "content",
];
const CACHED_UNDERLINE_THOUGHT_FIELDS: &[&str] = &["thoughts", "reviewCount", "reviewsCount"];

#[derive(Clone, Copy)]
struct UnderlineLimits {
    records: usize,
    quote_bytes: usize,
    range_bytes: usize,
    total_bytes: usize,
}

#[derive(Clone, Copy)]
struct MpArticleLimits {
    owner_bytes: usize,
    id_bytes: usize,
    records: usize,
    metadata_bytes: usize,
}

#[derive(Clone, Copy)]
struct MpAccountLimits {
    owner_bytes: usize,
    records: usize,
    metadata_bytes: usize,
}

impl Default for MpAccountLimits {
    fn default() -> Self {
        Self {
            owner_bytes: MAX_MP_ACCOUNT_ID_BYTES,
            records: MAX_MP_ACCOUNT_RECORDS,
            metadata_bytes: MAX_MP_ACCOUNT_METADATA_BYTES,
        }
    }
}

impl Default for MpArticleLimits {
    fn default() -> Self {
        Self {
            owner_bytes: MAX_MP_ACCOUNT_ID_BYTES,
            id_bytes: MAX_MP_ARTICLE_ID_BYTES,
            records: MAX_MP_ARTICLE_RECORDS,
            metadata_bytes: MAX_MP_ARTICLE_METADATA_BYTES,
        }
    }
}

impl Default for UnderlineLimits {
    fn default() -> Self {
        Self {
            records: MAX_UNDERLINE_RECORDS,
            quote_bytes: MAX_UNDERLINE_QUOTE_BYTES,
            range_bytes: MAX_UNDERLINE_RANGE_BYTES,
            total_bytes: MAX_UNDERLINE_TOTAL_BYTES,
        }
    }
}

#[derive(Clone, Copy)]
struct ReviewOutputLimits {
    records: usize,
    id_bytes: usize,
    author_bytes: usize,
    text_bytes: usize,
    work_bytes: usize,
}

impl Default for ReviewOutputLimits {
    fn default() -> Self {
        Self {
            records: MAX_REVIEW_OUTPUT_RECORDS,
            id_bytes: MAX_REVIEW_ID_BYTES,
            author_bytes: MAX_REVIEW_AUTHOR_BYTES,
            text_bytes: MAX_REVIEW_TEXT_BYTES,
            work_bytes: MAX_REVIEW_OUTPUT_WORK,
        }
    }
}

struct ReviewFields<'a> {
    node: &'a Value,
    id: Cow<'a, str>,
    author: Cow<'a, str>,
    text: Cow<'a, str>,
}

fn review_fields(value: &Value) -> ReviewFields<'_> {
    let mut node = value;
    for _ in 0..3 {
        if let Some(inner) = node.get("review").filter(|value| value.is_object()) {
            node = inner;
        } else {
            break;
        }
    }
    let author = node
        .get("author")
        .or_else(|| node.get("user"))
        .unwrap_or(node);
    let author = author.as_str().map_or_else(
        || borrowed_text_at(author, &["nick", "name", "userName", "nickname"]),
        Cow::Borrowed,
    );
    let text = ["content", "htmlContent", "text"]
        .into_iter()
        .map(|field| borrowed_text_at(node, &[field]))
        .find(|text| !text.is_empty())
        .unwrap_or(Cow::Borrowed(""));
    ReviewFields {
        node,
        id: borrowed_text_at(node, &["reviewId", "id"]),
        author,
        text,
    }
}

fn review_scalar_work_bytes(node: &Value) -> Result<usize> {
    // Numeric JSON values have bounded representations. Numeric strings may
    // still require scanning their full borrowed span; include all possible
    // scalar fallbacks without parsing them merely to decide admission.
    let likes = ["likesCount", "likeCount", "likes"]
        .into_iter()
        .find_map(|field| node.get(field))
        .and_then(Value::as_str)
        .map_or(0, str::len);
    ["createTime", "time", "star", "rating"]
        .into_iter()
        .filter_map(|field| node.get(field).and_then(Value::as_str))
        .try_fold(likes, |used, text| used.checked_add(text.len()))
        .and_then(|bytes| bytes.checked_mul(2))
        .ok_or(SourceError::ResponseTooLarge)
}

struct ReviewOutputBudget {
    limits: ReviewOutputLimits,
    records: usize,
    work_bytes: usize,
}

impl ReviewOutputBudget {
    fn new(limits: ReviewOutputLimits) -> Self {
        Self {
            limits: ReviewOutputLimits {
                records: limits.records.min(MAX_REVIEW_OUTPUT_RECORDS),
                id_bytes: limits.id_bytes.min(MAX_REVIEW_ID_BYTES),
                author_bytes: limits.author_bytes.min(MAX_REVIEW_AUTHOR_BYTES),
                text_bytes: limits.text_bytes.min(MAX_REVIEW_TEXT_BYTES),
                work_bytes: limits.work_bytes.min(MAX_REVIEW_OUTPUT_WORK),
            },
            records: 0,
            work_bytes: 0,
        }
    }

    fn admit(&mut self, value: &Value) -> Result<()> {
        self.records = self
            .records
            .checked_add(1)
            .filter(|records| *records <= self.limits.records)
            .ok_or(SourceError::ResponseTooLarge)?;
        let fields = review_fields(value);
        if fields.id.len() > self.limits.id_bytes
            || fields.author.len() > self.limits.author_bytes
            || fields.text.len() > self.limits.text_bytes
        {
            return Err(SourceError::ResponseTooLarge);
        }
        let fee = fields
            .id
            .len()
            .checked_add(fields.author.len())
            .and_then(|bytes| bytes.checked_mul(4))
            .and_then(|bytes| {
                fields
                    .text
                    .len()
                    .checked_mul(REVIEW_TEXT_WORK_MULTIPLIER)
                    .and_then(|text| bytes.checked_add(text))
            })
            .and_then(|bytes| bytes.checked_add(4 * std::mem::size_of::<RemoteReview>() + 128))
            .ok_or(SourceError::ResponseTooLarge)?;
        let fee = fee
            .checked_add(review_scalar_work_bytes(fields.node)?)
            .ok_or(SourceError::ResponseTooLarge)?;
        reserve_output_work(&mut self.work_bytes, fee, self.limits.work_bytes)
    }
}

fn review_records(value: &Value) -> &[Value] {
    value
        .get("reviews")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn thought_review_records(value: &Value) -> impl Iterator<Item = &Value> + Clone {
    review_records(value).iter().flat_map(|entry| {
        let pages = entry.get("pageReviews").and_then(Value::as_array);
        pages
            .into_iter()
            .flatten()
            .chain(pages.is_none().then_some(entry))
    })
}

fn normalize_review_candidates<'a>(
    records: impl Iterator<Item = &'a Value> + Clone,
    budget: &mut ReviewOutputBudget,
) -> Result<Vec<RemoteReview>> {
    let mut count = 0usize;
    // An immutable borrowed prepass prices the complete response before Vec
    // allocation, any selected string copy, or HTML conversion.
    for record in records.clone() {
        budget.admit(record)?;
        count += 1;
    }
    let mut output = Vec::with_capacity(count);
    for record in records {
        output.push(materialize_review(record));
    }
    Ok(output)
}

#[derive(Clone, Copy)]
struct BookOutputLimits {
    records: usize,
    id_bytes: usize,
    short_field_bytes: usize,
    cover_bytes: usize,
    intro_bytes: usize,
    work_bytes: usize,
}

impl Default for BookOutputLimits {
    fn default() -> Self {
        Self {
            records: MAX_BOOK_OUTPUT_RECORDS,
            id_bytes: MAX_BOOK_ID_BYTES,
            short_field_bytes: MAX_BOOK_SHORT_FIELD_BYTES,
            cover_bytes: MAX_BOOK_COVER_BYTES,
            intro_bytes: MAX_BOOK_INTRO_BYTES,
            work_bytes: MAX_BOOK_OUTPUT_WORK,
        }
    }
}

struct BookFields<'a> {
    id: Cow<'a, str>,
    title: Cow<'a, str>,
    author: Cow<'a, str>,
    cover: Cow<'a, str>,
    category: Cow<'a, str>,
    intro: Cow<'a, str>,
    publisher: Cow<'a, str>,
    isbn: Cow<'a, str>,
}

fn book_fields(value: &Value) -> BookFields<'_> {
    let info = book_info_node(value);
    BookFields {
        id: borrowed_text_at(info, &["bookId", "book_id"]),
        title: borrowed_text_at(info, &["title"]),
        author: borrowed_text_at(info, &["author"]),
        cover: borrowed_text_at(info, &["cover", "coverUrl"]),
        category: borrowed_text_at(info, &["category", "categoryName"]),
        intro: borrowed_text_at(info, &["intro", "description"]),
        publisher: borrowed_text_at(info, &["publisher"]),
        isbn: borrowed_text_at(info, &["isbn"]),
    }
}

enum BookRawFrame<'a> {
    Array(std::slice::Iter<'a, Value>),
    Object(serde_json::map::Iter<'a>),
}

impl<'a> BookRawFrame<'a> {
    fn next(&mut self) -> Option<(&'a Value, Option<&'a str>)> {
        match self {
            Self::Array(values) => values.next().map(|value| (value, None)),
            Self::Object(values) => values
                .next()
                .map(|(key, value)| (value, Some(key.as_str()))),
        }
    }
}

struct BookOutputBudget {
    limits: BookOutputLimits,
    records: usize,
    work_bytes: usize,
}

impl BookOutputBudget {
    fn new(limits: BookOutputLimits) -> Self {
        Self {
            limits: BookOutputLimits {
                records: limits.records.min(MAX_BOOK_OUTPUT_RECORDS),
                id_bytes: limits.id_bytes.min(MAX_BOOK_ID_BYTES),
                short_field_bytes: limits.short_field_bytes.min(MAX_BOOK_SHORT_FIELD_BYTES),
                cover_bytes: limits.cover_bytes.min(MAX_BOOK_COVER_BYTES),
                intro_bytes: limits.intro_bytes.min(MAX_BOOK_INTRO_BYTES),
                work_bytes: limits.work_bytes.min(MAX_BOOK_OUTPUT_WORK),
            },
            records: 0,
            work_bytes: 0,
        }
    }

    fn spend(&mut self, bytes: usize) -> Result<()> {
        reserve_output_work(&mut self.work_bytes, bytes, self.limits.work_bytes)
    }

    fn admit(&mut self, value: &Value, fallback: Option<&str>) -> Result<bool> {
        self.records = self
            .records
            .checked_add(1)
            .filter(|records| *records <= self.limits.records)
            .ok_or(SourceError::ResponseTooLarge)?;
        // Empty list IDs still spend count/scan admission, but their unused
        // fields/raw data are no longer copied simply to discard the result.
        self.spend(64)?;
        let fields = book_fields(value);
        if fields.id.is_empty() && fallback.is_none() {
            return Ok(false);
        }
        let id = if fields.id.is_empty() {
            fallback.unwrap_or_default()
        } else {
            fields.id.as_ref()
        };
        if id.len() > self.limits.id_bytes
            || fields.cover.len() > self.limits.cover_bytes
            || fields.intro.len() > self.limits.intro_bytes
            || [
                &fields.title,
                &fields.author,
                &fields.category,
                &fields.publisher,
                &fields.isbn,
            ]
            .iter()
            .any(|field| field.len() > self.limits.short_field_bytes)
        {
            return Err(SourceError::ResponseTooLarge);
        }
        let field_bytes = [
            id.len(),
            fields.title.len(),
            fields.author.len(),
            fields.cover.len(),
            fields.category.len(),
            fields.intro.len(),
            fields.publisher.len(),
            fields.isbn.len(),
        ]
        .into_iter()
        .try_fold(0usize, |used, bytes| used.checked_add(bytes))
        .and_then(|bytes| bytes.checked_mul(4))
        .and_then(|bytes| bytes.checked_add(4 * std::mem::size_of::<RemoteBook>() + 256))
        .ok_or(SourceError::ResponseTooLarge)?;
        self.spend(field_bytes)?;
        self.admit_raw(value)?;
        Ok(true)
    }

    fn admit_raw(&mut self, root: &Value) -> Result<()> {
        let mut frames = Vec::<BookRawFrame<'_>>::new();
        let mut current: Option<(&Value, Option<&str>)> = Some((root, None));
        loop {
            if let Some((value, key)) = current.take() {
                if let Some(key) = key {
                    self.spend(
                        key.len()
                            .checked_mul(2)
                            .and_then(|bytes| bytes.checked_add(2 * BOOK_RAW_KEY_BYTES))
                            .ok_or(SourceError::ResponseTooLarge)?,
                    )?;
                }
                // Pay before inspecting or retaining this node. Each frame
                // owns an iterator, never an array of all unvisited siblings.
                self.spend(2 * BOOK_RAW_VALUE_BYTES)?;
                #[cfg(test)]
                book_output_budget_tests::observe_raw_node();
                let frame = match value {
                    Value::String(text) => {
                        self.spend(
                            text.len()
                                .checked_mul(2)
                                .ok_or(SourceError::ResponseTooLarge)?,
                        )?;
                        None
                    }
                    Value::Array(values) => {
                        self.spend(2 * BOOK_RAW_ARRAY_BYTES)?;
                        Some(BookRawFrame::Array(values.iter()))
                    }
                    Value::Object(values) => {
                        self.spend(2 * BOOK_RAW_OBJECT_BYTES)?;
                        Some(BookRawFrame::Object(values.iter()))
                    }
                    _ => None,
                };
                if let Some(frame) = frame {
                    if frames.len() >= MAX_BOOK_RAW_DEPTH {
                        return Err(SourceError::ResponseTooLarge);
                    }
                    frames.push(frame);
                    #[cfg(test)]
                    book_output_budget_tests::observe_raw_depth(frames.len());
                }
            }
            loop {
                let Some(frame) = frames.last_mut() else {
                    return Ok(());
                };
                if let Some(next) = frame.next() {
                    current = Some(next);
                    break;
                }
                frames.pop();
            }
        }
    }
}

fn reserve_output_work(used: &mut usize, bytes: usize, maximum: usize) -> Result<()> {
    *used = used
        .checked_add(bytes)
        .filter(|bytes| *bytes <= maximum)
        .ok_or(SourceError::ResponseTooLarge)?;
    Ok(())
}

#[derive(Clone, Copy)]
struct StatisticsOutputLimits {
    records: usize,
    key_bytes: usize,
    metadata_bytes: usize,
    work_bytes: usize,
}

impl Default for StatisticsOutputLimits {
    fn default() -> Self {
        Self {
            records: MAX_STATISTICS_OUTPUT_RECORDS,
            key_bytes: MAX_STATISTICS_KEY_BYTES,
            metadata_bytes: MAX_STATISTICS_METADATA_BYTES,
            work_bytes: MAX_STATISTICS_OUTPUT_WORK,
        }
    }
}

/// A complete immutable prepass authorizes only this response's map copies.
struct AdmittedStatistics<'a> {
    buckets: Option<&'a serde_json::Map<String, Value>>,
    #[cfg(test)]
    records: usize,
    #[cfg(test)]
    metadata_bytes: usize,
    #[cfg(test)]
    work_bytes: usize,
}

fn statistics_scalar_work(value: Option<&Value>) -> Result<usize> {
    value
        .and_then(Value::as_str)
        .map_or(Some(32), |text| {
            text.len()
                .checked_mul(2)
                .and_then(|bytes| bytes.checked_add(32))
        })
        .ok_or(SourceError::ResponseTooLarge)
}

fn admit_statistics(
    value: &Value,
    limits: StatisticsOutputLimits,
) -> Result<AdmittedStatistics<'_>> {
    let limits = StatisticsOutputLimits {
        records: limits.records.min(MAX_STATISTICS_OUTPUT_RECORDS),
        key_bytes: limits.key_bytes.min(MAX_STATISTICS_KEY_BYTES),
        metadata_bytes: limits.metadata_bytes.min(MAX_STATISTICS_METADATA_BYTES),
        work_bytes: limits.work_bytes.min(MAX_STATISTICS_OUTPUT_WORK),
    };
    let buckets = value.get("readTimes").and_then(Value::as_object);
    let records = buckets.map_or(0, serde_json::Map::len);
    if records > limits.records {
        return Err(SourceError::ResponseTooLarge);
    }
    let mut metadata_bytes = 0usize;
    let mut work_bytes = 0usize;
    let fixed = 2 * std::mem::size_of::<ReadStatistics>() + 256;
    reserve_output_work(&mut metadata_bytes, fixed, limits.metadata_bytes)?;
    reserve_output_work(&mut work_bytes, fixed, limits.work_bytes)?;
    // Include every selected summary numeric string before number_at/time_at
    // can scan it. Missing and non-string scalars retain their normal fallback.
    for field in [
        "totalReadTime",
        "readDays",
        "dayAverageReadTime",
        "baseTime",
    ] {
        reserve_output_work(
            &mut work_bytes,
            statistics_scalar_work(value.get(field))?,
            limits.work_bytes,
        )?;
    }
    // A record-count-derived logical allowance prepays tree comparison byte
    // visits and owned-key work. At 8,192 rows it reserves 64 comparison passes;
    // this is an operation policy, not an exact allocator/RSS or CPU-time claim.
    let comparison_passes = (records.max(1).ilog2() as usize)
        .checked_add(1)
        .and_then(|levels| levels.checked_mul(4))
        .and_then(|passes| passes.checked_add(8))
        .ok_or(SourceError::ResponseTooLarge)?;
    for (key, scalar) in buckets.into_iter().flatten() {
        #[cfg(test)]
        statistics_output_budget_tests::observe_borrowed_record();
        if key.len() > limits.key_bytes {
            return Err(SourceError::ResponseTooLarge);
        }
        let metadata = key
            .len()
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<(String, u64)>() + 128))
            .ok_or(SourceError::ResponseTooLarge)?;
        reserve_output_work(&mut metadata_bytes, metadata, limits.metadata_bytes)?;
        let key_work = key
            .len()
            .checked_mul(comparison_passes)
            .and_then(|bytes| {
                key.len()
                    .checked_mul(4)
                    .and_then(|copies| bytes.checked_add(copies))
            })
            .ok_or(SourceError::ResponseTooLarge)?;
        let scalar_work = statistics_scalar_work(Some(scalar))?;
        let fee = metadata
            .checked_add(key_work)
            .and_then(|bytes| bytes.checked_add(scalar_work))
            .ok_or(SourceError::ResponseTooLarge)?;
        reserve_output_work(&mut work_bytes, fee, limits.work_bytes)?;
    }
    Ok(AdmittedStatistics {
        buckets,
        #[cfg(test)]
        records,
        #[cfg(test)]
        metadata_bytes,
        #[cfg(test)]
        work_bytes,
    })
}

impl AdmittedStatistics<'_> {
    fn into_daily_seconds(self) -> BTreeMap<String, u64> {
        #[cfg(test)]
        statistics_output_budget_tests::observe_map_admission(
            self.records,
            self.metadata_bytes,
            self.work_bytes,
        );
        let mut output = BTreeMap::new();
        for (key, value) in self.buckets.into_iter().flatten() {
            #[cfg(test)]
            statistics_output_budget_tests::observe_key_copy(
                key.len(),
                self.records,
                self.metadata_bytes,
                self.work_bytes,
            );
            // Numeric strings retain the existing u64 parse/default behavior.
            // No key or count field is materialized before the complete gate.
            let key = key.clone();
            let seconds = value.as_u64().unwrap_or_else(|| {
                value
                    .as_str()
                    .and_then(|text| text.parse().ok())
                    .unwrap_or_default()
            });
            output.insert(key, seconds);
        }
        output
    }
}

fn normalize_statistics(value: Value, limits: StatisticsOutputLimits) -> Result<ReadStatistics> {
    let admitted = admit_statistics(&value, limits)?;
    let daily_seconds = admitted.into_daily_seconds();
    #[cfg(test)]
    statistics_output_budget_tests::observe_summary_materialization();
    Ok(ReadStatistics {
        total_seconds: u64::from(number_at(&value, &["totalReadTime"])),
        read_days: number_at(&value, &["readDays"]),
        average_seconds: u64::from(number_at(&value, &["dayAverageReadTime"])),
        base_time: time_at(&value, &["baseTime"]),
        daily_seconds,
        raw: value,
    })
}

fn ordinary_shelf_records(value: &Value) -> impl Iterator<Item = &Value> + Clone {
    value
        .get("books")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| {
            !borrowed_text_at(book_info_node(item), &["bookId", "book_id"]).starts_with("MP_WXS_")
        })
}

fn search_book_records(value: &Value) -> impl Iterator<Item = &Value> + Clone {
    value
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|group| {
            group
                .get("books")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
}

fn admit_book_candidates<'a>(
    records: impl Iterator<Item = &'a Value>,
    budget: &mut BookOutputBudget,
) -> Result<()> {
    for record in records {
        budget.admit(record, None)?;
    }
    Ok(())
}

fn normalize_single_book(
    value: &Value,
    fallback: &str,
    limits: BookOutputLimits,
) -> Result<RemoteBook> {
    // This is the existing single-book validity rule. An invalid response
    // cannot justify copying its otherwise unused retained raw tree.
    if book_fields(value).title.is_empty() {
        return Err(SourceError::InvalidResponse("book_info"));
    }
    let mut budget = BookOutputBudget::new(limits);
    budget.admit(value, Some(fallback))?;
    let mut book = materialize_book(value);
    if book.id.is_empty() {
        book.id = fallback.into();
    }
    Ok(book)
}

fn admit_underline_bytes(
    total: &mut usize,
    range_bytes: usize,
    quote_bytes: usize,
    limits: UnderlineLimits,
) -> Result<()> {
    if range_bytes > limits.range_bytes || quote_bytes > limits.quote_bytes {
        return Err(SourceError::ResponseTooLarge);
    }
    *total = total
        .checked_add(range_bytes)
        .and_then(|size| size.checked_add(quote_bytes))
        .filter(|size| *size <= limits.total_bytes)
        .ok_or(SourceError::ResponseTooLarge)?;
    Ok(())
}

fn selected_text_bytes(value: &Value, fields: &[&str]) -> usize {
    fields
        .iter()
        .find_map(|field| match value.get(field) {
            Some(Value::String(text)) => Some(text.len()),
            // Number has a bounded representation without arbitrary precision.
            Some(Value::Number(number)) => Some(number.to_string().len()),
            _ => None,
        })
        .unwrap_or(0)
}

fn normalize_underlines(value: &Value, limits: UnderlineLimits) -> Result<Vec<RemoteUnderline>> {
    let Some(records) = value
        .get("underlines")
        .or_else(|| value.get("updated"))
        .and_then(Value::as_array)
    else {
        return Ok(Vec::new());
    };
    normalize_underline_records(
        records,
        limits,
        UNDERLINE_TEXT_FIELDS,
        &["reviewCount", "reviewsCount"],
    )
}

fn normalize_underline_records(
    records: &[Value],
    limits: UnderlineLimits,
    text_fields: &[&str],
    thought_fields: &[&str],
) -> Result<Vec<RemoteUnderline>> {
    if records.len() > limits.records {
        return Err(SourceError::ResponseTooLarge);
    }
    let mut total = 0;
    // Inspect the borrowed selected fields before allocating the mapped vector
    // or cloning any quote. Fallback aliases match text_at exactly.
    for item in records {
        admit_underline_bytes(
            &mut total,
            selected_text_bytes(item, &["range"]),
            selected_text_bytes(item, text_fields),
            limits,
        )?;
    }
    Ok(records
        .iter()
        .map(|item| RemoteUnderline {
            range: text_at(item, &["range"]),
            text: text_at(item, text_fields),
            count: number_at(item, &["count", "underlineCount"]),
            thoughts: number_at(item, thought_fields),
        })
        .collect())
}

/// Verified format and transport hints that belong to one download task.
/// Create a fresh value for every task, including resumed tasks.
#[derive(Clone, Default)]
pub struct ChapterReadHints {
    scope: Option<ChapterHintScope>,
    confirmed_text: bool,
    serial_epub: Arc<AtomicBool>,
    download_diagnostics: Option<Arc<DownloadDiagnostics>>,
    download_reader: Option<DownloadReaderContext>,
}

#[derive(Clone)]
struct ChapterHintScope {
    client: Weak<Mutex<AuthSession>>,
    account_key: String,
    book_id: String,
}

impl ChapterReadHints {
    pub(crate) fn set_download_diagnostics(&mut self, diagnostics: Arc<DownloadDiagnostics>) {
        self.download_diagnostics = Some(diagnostics);
    }

    pub(crate) fn set_download_reader_gate(&mut self, admission: Arc<ReaderAdmission>) {
        self.download_reader = Some(DownloadReaderContext::new(admission));
    }

    pub(crate) fn for_book(client: &WeReadClient, book_id: &str) -> Self {
        let mut hints = Self::default();
        hints.bind(client, book_id);
        hints
    }

    /// Ordered results update the format hint. Credential-sensitive shard
    /// fallback is shared immediately by every clone of this task's scope.
    pub(crate) fn merge(&mut self, returned: Self) {
        let same_scope = self
            .scope
            .as_ref()
            .zip(returned.scope.as_ref())
            .is_some_and(|(current, returned)| {
                Weak::ptr_eq(&current.client, &returned.client)
                    && current.account_key == returned.account_key
                    && current.book_id == returned.book_id
            });
        if same_scope && Arc::ptr_eq(&self.serial_epub, &returned.serial_epub) {
            self.confirmed_text = returned.confirmed_text;
        }
    }

    fn bind(&mut self, client: &WeReadClient, book_id: &str) {
        let identity = super::report::account_key(&client.session());
        let same_scope = self.scope.as_ref().is_some_and(|scope| {
            Weak::ptr_eq(&scope.client, &Arc::downgrade(&client.auth))
                && identity.as_ref() == Some(&scope.account_key)
                && scope.book_id == book_id
        });
        if !same_scope {
            let download_diagnostics = self.download_diagnostics.take();
            let download_reader = self.download_reader.take();
            *self = Self::default();
            self.download_diagnostics = download_diagnostics;
            if let Some(context) = download_reader {
                self.set_download_reader_gate(context.admission);
            }
            self.scope = identity.map(|account_key| ChapterHintScope {
                client: Arc::downgrade(&client.auth),
                account_key,
                book_id: book_id.into(),
            });
        }
    }
}

enum ChapterAttempt {
    Complete(ChapterContent),
    ProbeFormat,
    RetrySerial,
    RefreshAccess,
}

#[derive(Clone)]
pub struct WeReadClient {
    pub(super) http: Client,
    // Arbitrary resource authorities must not retain idle sockets after their
    // job credit is released. One shared client avoids per-request construction.
    resource_http: Client,
    pub(super) auth: Arc<Mutex<AuthSession>>,
    pub(super) renewal: Arc<tokio::sync::Mutex<u64>>,
    #[cfg(test)]
    pub(super) test_web_origin: Option<String>,
    #[cfg(test)]
    test_gateway_url: Option<String>,
    #[cfg(test)]
    test_resource_origin: Option<String>,
}

impl WeReadClient {
    pub fn new(auth: AuthSession) -> Result<Self> {
        validate_cookie_jar(&auth.cookies)?;
        Ok(Self {
            http: http_client()?,
            resource_http: unpooled_http_client()?,
            auth: Arc::new(Mutex::new(auth)),
            renewal: Arc::new(tokio::sync::Mutex::new(0)),
            #[cfg(test)]
            test_web_origin: None,
            #[cfg(test)]
            test_gateway_url: None,
            #[cfg(test)]
            test_resource_origin: None,
        })
    }

    pub fn session(&self) -> AuthSession {
        self.auth
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn replace_session(&self, session: AuthSession) -> Result<()> {
        validate_cookie_jar(&session.cookies)?;
        *self
            .auth
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = session;
        Ok(())
    }

    pub fn capabilities(&self) -> SourceCapabilities {
        let session = self.session();
        SourceCapabilities {
            catalog: session.has_web_session(),
            search: session.has_api_key(),
            chapter_content: session.has_web_session(),
            remote_progress: session.has_api_key(),
            statistics: session.has_api_key(),
            mp_articles: session.has_web_session(),
            remote_shelf_mutation: false,
            remote_annotation_write: false,
            reading_time: session.has_api_key()
                && session.has_web_session()
                && super::report::account_key(&session).is_some(),
            chapter_images: session.has_web_session(),
        }
    }

    pub async fn gateway(&self, api_name: &str, params: Value) -> Result<Value> {
        let response = self
            .gateway_request(api_name, params)?
            .send()
            .await
            .map_err(network_error)?;
        response_json(response).await
    }

    fn gateway_url(&self) -> &str {
        #[cfg(test)]
        if let Some(url) = self.test_gateway_url.as_deref() {
            return url;
        }
        GATEWAY_URL
    }

    fn gateway_request(&self, api_name: &str, params: Value) -> Result<RequestBuilder> {
        let session = self.session();
        if !session.has_api_key() {
            return Err(SourceError::ApiKeyRequired);
        }
        let mut payload = params.as_object().cloned().unwrap_or_default();
        payload.insert("api_name".into(), json!(api_name));
        payload.insert("skill_version".into(), json!(SKILL_VERSION));
        // The gateway uses only its bearer API key. Web cookies never cross here.
        Ok(self
            .http
            .post(self.gateway_url())
            .header("Accept-Encoding", "gzip")
            .bearer_auth(&session.api_key)
            .header("Origin", WEB_ORIGIN)
            .header("Referer", format!("{WEB_ORIGIN}/"))
            .json(&payload))
    }

    pub async fn shelf(&self) -> Result<Vec<RemoteBook>> {
        Ok(self.shelf_snapshot().await?.books)
    }

    pub async fn shelf_snapshot(&self) -> Result<ShelfSnapshot> {
        self.shelf_snapshot_with_mp_limits(MpAccountLimits::default())
            .await
    }

    async fn shelf_snapshot_with_mp_limits(
        &self,
        limits: MpAccountLimits,
    ) -> Result<ShelfSnapshot> {
        self.shelf_snapshot_with_output_limits(limits, BookOutputLimits::default())
            .await
    }

    async fn shelf_snapshot_with_output_limits(
        &self,
        mp_limits: MpAccountLimits,
        book_limits: BookOutputLimits,
    ) -> Result<ShelfSnapshot> {
        let result = self.gateway("/shelf/sync", json!({})).await?;
        let Some(books) = result.get("books").and_then(Value::as_array) else {
            return Err(SourceError::InvalidResponse("shelf_books"));
        };
        let mut book_budget = BookOutputBudget::new(book_limits);
        admit_book_candidates(ordinary_shelf_records(&result), &mut book_budget)?;
        // Complete borrowed MP admission precedes every snapshot copy. MP
        // records never construct a RemoteBook or clone its unused raw data.
        let mut snapshot = ShelfSnapshot {
            mp_accounts: normalize_shelf_mp_accounts(books, mp_limits)?,
            ..ShelfSnapshot::default()
        };
        for item in ordinary_shelf_records(&result) {
            if book_fields(item).id.is_empty() {
                continue;
            }
            snapshot.books.push(materialize_book(item));
        }
        Ok(snapshot)
    }

    pub async fn search(&self, keyword: &str) -> Result<Vec<RemoteBook>> {
        self.search_with_output_limits(keyword, BookOutputLimits::default())
            .await
    }

    async fn search_with_output_limits(
        &self,
        keyword: &str,
        limits: BookOutputLimits,
    ) -> Result<Vec<RemoteBook>> {
        if keyword.trim().is_empty() {
            return Ok(Vec::new());
        }
        let result = self
            .gateway("/store/search", json!({"keyword": keyword, "count": 30}))
            .await?;
        if result.get("results").and_then(Value::as_array).is_none() {
            return Err(SourceError::InvalidResponse("search_results"));
        }
        let mut budget = BookOutputBudget::new(limits);
        admit_book_candidates(search_book_records(&result), &mut budget)?;
        let mut books = Vec::new();
        let mut seen = BTreeSet::new();
        for entry in search_book_records(&result) {
            if book_fields(entry).id.is_empty() {
                continue;
            }
            let book = materialize_book(entry);
            if seen.insert(book.id.clone()) {
                books.push(book);
            }
        }
        Ok(books)
    }

    pub async fn book_info(&self, book_id: &str) -> Result<RemoteBook> {
        self.book_info_with_output_limits(book_id, BookOutputLimits::default())
            .await
    }

    async fn book_info_with_output_limits(
        &self,
        book_id: &str,
        limits: BookOutputLimits,
    ) -> Result<RemoteBook> {
        if book_id.len() > limits.id_bytes.min(MAX_BOOK_ID_BYTES) {
            return Err(SourceError::ResponseTooLarge);
        }
        let result = self
            .gateway("/book/info", json!({"bookId": book_id}))
            .await?;
        normalize_single_book(&result, book_id, limits)
    }

    pub async fn catalog(&self, book_id: &str) -> Result<Vec<RemoteChapter>> {
        let url = reader_url(book_id, None)?;
        let request = self
            .web_request(Method::POST, "/web/book/chapterInfos")?
            .header("Referer", url)
            .json(&json!({"bookIds": [book_id]}));
        let value = self.web_json_read(request).await?;
        normalize_catalog_checked(&value, book_id)
    }

    pub(super) async fn reader_session(
        &self,
        book_id: &str,
        chapter_uid: Option<&str>,
    ) -> Result<ReaderSession> {
        let mut auth = WebReadAuth::default();
        self.reader_session_with_auth(book_id, chapter_uid, &mut auth)
            .await
    }

    async fn reader_session_with_auth(
        &self,
        book_id: &str,
        chapter_uid: Option<&str>,
        auth: &mut WebReadAuth,
    ) -> Result<ReaderSession> {
        let context = auth.download_reader.clone();
        loop {
            // An explicit ordinary read retains the legacy full allowance.
            // Speculative reads learn only a response tier within this task.
            let tier = context.as_ref().map(|context| {
                if auth.response_limit.is_none() {
                    ReaderTier::FULL
                } else {
                    context.tier()
                }
            });
            // Declare admission before HTML. All network error/cancellation
            // paths drop their response buffers before returning this credit.
            let reservation = match (&context, tier) {
                (Some(context), Some(tier)) => Some(context.admission.acquire(tier).await?),
                _ => None,
            };
            if auth.response_limit.is_some()
                && context
                    .as_ref()
                    .zip(tier)
                    .is_some_and(|(context, tier)| context.tier() > tier)
            {
                // A concurrent chapter learned a larger tier while we waited.
                // Release the smaller credit before joining the larger queue.
                drop(reservation);
                continue;
            }
            let (before_account, before_generation) = {
                let generation = self.renewal.lock().await;
                (super::report::account_key(&self.session()), *generation)
            };
            let url = reader_url(book_id, chapter_uid)?;
            let path = url
                .strip_prefix(WEB_ORIGIN)
                .ok_or(SourceError::InvalidResponse("reader_url"))?;
            let request = self.web_request(Method::GET, path)?.header("Referer", &url);
            let response_limit = tier.map(ReaderTier::bytes).or(auth.response_limit);
            let (html, response_budget_exceeded) = self
                .web_text_read_with_limit(request, auth, false, response_limit)
                .await;
            let html = match html {
                Err(SourceError::ResponseTooLarge)
                    if response_budget_exceeded
                        && !auth.recovery_failed
                        && tier.is_some_and(|tier| tier != ReaderTier::FULL) =>
                {
                    // Never wait for more credit while retaining a smaller
                    // reservation: several simultaneous upgrades must progress.
                    drop(reservation);
                    let _ = context
                        .as_ref()
                        .expect("The adaptive Reader has admission")
                        .advance(tier.expect("The adaptive Reader has a tier"));
                    continue;
                }
                result => result?,
            };
            if let Some(diagnostics) = &auth.download_diagnostics {
                diagnostics.reader_response(html.len());
            }
            let (current, generation) = {
                let generation = self.renewal.lock().await;
                (self.session(), *generation)
            };
            let account = super::report::account_key(&current);
            let expected_vid = if current.user_vid.is_empty() {
                current.cookies.get("wr_vid").map(String::as_str)
            } else {
                Some(current.user_vid.as_str())
            };
            let response_bytes = html.len();
            let session = if let Some(reservation) = reservation {
                let book_id = book_id.to_owned();
                let chapter_uid = chapter_uid.map(str::to_owned);
                let expected_vid = expected_vid.map(str::to_owned);
                reservation
                    .page(html)
                    .parse(move |html| {
                        super::content::extract_reader_session_with_access(
                            html,
                            &book_id,
                            chapter_uid.as_deref(),
                            expected_vid.as_deref(),
                            now_seconds(),
                        )
                    })
                    .await
            } else {
                // Ordinary source reads preserve their synchronous parser and
                // its existing operation-budget observations.
                super::content::extract_reader_session_with_access(
                    &html,
                    book_id,
                    chapter_uid,
                    expected_vid,
                    now_seconds(),
                )
            };
            if matches!(&session, Err(SourceError::ResponseTooLarge)) {
                if let Some(diagnostics) = &auth.download_diagnostics {
                    diagnostics.source_budget_exceeded(ResponsePhase::Reader, response_bytes);
                }
            }
            let mut session = session?;
            // Parser queueing and blocking work introduce another renewal
            // window. A proof is fresh only if all three snapshots agree.
            let (after_account, after_generation) = {
                let generation = self.renewal.lock().await;
                (super::report::account_key(&self.session()), *generation)
            };
            let stable = before_account == account
                && before_generation == generation
                && account == after_account
                && generation == after_generation;
            session
                .access
                .bind_session(after_account, after_generation, stable);
            session.ensure_reader_clock(now_seconds())?;
            return Ok(session);
        }
    }

    async fn reader_content_status(
        &self,
        session: &ReaderSession,
        text: &str,
    ) -> crate::domain::ContentStatus {
        let generation = self.renewal.lock().await;
        session.access.content_status(
            text,
            &super::report::account_key(&self.session()),
            *generation,
        )
    }

    async fn completed_chapter_attempt(
        &self,
        session: &ReaderSession,
        content: ChapterContent,
    ) -> ChapterAttempt {
        let generation = self.renewal.lock().await;
        if session
            .access
            .is_current(&super::report::account_key(&self.session()), *generation)
        {
            ChapterAttempt::Complete(content)
        } else {
            // Another chapter may renew the shared session during this read.
            // Revalidate once using the existing access-refresh budget rather
            // than publishing an expired proof or treating it as missing rights.
            ChapterAttempt::RefreshAccess
        }
    }

    async fn shard(
        &self,
        book_id: &str,
        chapter_uid: &str,
        session: &ReaderSession,
        endpoint: &str,
        auth: &mut WebReadAuth,
    ) -> Result<String> {
        session.validate_opaque()?;
        let payload = content_payload(
            book_id,
            chapter_uid,
            &session.psvts,
            endpoint.ends_with("e_2"),
            now_seconds(),
        )?;
        let request = self
            .web_request(Method::POST, endpoint)?
            .header("Referer", reader_url(book_id, Some(chapter_uid))?)
            .json(&payload);
        let text = self.web_text_read(request, auth, true).await?;
        validate_shard_response(endpoint, &text)?;
        Ok(text)
    }

    pub async fn chapter(&self, book_id: &str, chapter_uid: &str) -> Result<ChapterContent> {
        self.chapter_with_hints(book_id, chapter_uid, &mut ChapterReadHints::default())
            .await
    }

    /// Read a chapter while reusing only hints verified in this account/book/task.
    pub async fn chapter_with_hints(
        &self,
        book_id: &str,
        chapter_uid: &str,
        hints: &mut ChapterReadHints,
    ) -> Result<ChapterContent> {
        self.chapter_with_read_limits(book_id, chapter_uid, hints, None)
            .await
    }

    /// Speculation uses a small decoded-response budget. The ordered download
    /// owner can retry an oversized chapter alone with the normal read budget.
    pub(crate) async fn chapter_prefetch_with_hints(
        &self,
        book_id: &str,
        chapter_uid: &str,
        hints: &mut ChapterReadHints,
    ) -> Result<ChapterContent> {
        self.chapter_with_read_limits(
            book_id,
            chapter_uid,
            hints,
            Some(MAX_PREFETCH_RESPONSE_BYTES),
        )
        .await
    }

    async fn chapter_with_read_limits(
        &self,
        book_id: &str,
        chapter_uid: &str,
        hints: &mut ChapterReadHints,
        response_limit: Option<usize>,
    ) -> Result<ChapterContent> {
        hints.bind(self, book_id);
        let account = super::report::account_key(&self.session());
        let mut retries = 0u64;
        let mut auth = WebReadAuth::default();
        auth.response_limit = response_limit;
        auth.download_diagnostics = hints.download_diagnostics.clone();
        auth.download_reader = hints.download_reader.clone();
        let mut access_refreshed = false;
        loop {
            if super::report::account_key(&self.session()) != account {
                hints.bind(self, book_id);
                return Err(SourceError::Unsupported("chapter_account_changed"));
            }
            let result = self
                .chapter_once(book_id, chapter_uid, &mut auth, hints)
                .await;
            if auth.recovery_failed {
                return result.and_then(completed_chapter);
            }
            if std::mem::take(&mut auth.restart_reader) {
                // A renewed shard must acquire a new Reader context, not replay
                // a signed request carrying the previous context's psvts.
                crate::diagnostic!("weread: source operation=chapter_read auth_retry=1");
                continue;
            }
            match result {
                Ok(ChapterAttempt::RefreshAccess) => {
                    if access_refreshed {
                        return Err(SourceError::ContentUnverified);
                    }
                    access_refreshed = true;
                    crate::diagnostic!("weread: source operation=chapter_read access_refresh=1");
                }
                Ok(ChapterAttempt::ProbeFormat) => {
                    crate::diagnostic!("weread: source operation=chapter_read format_probe=1");
                }
                Ok(ChapterAttempt::RetrySerial) => {
                    crate::diagnostic!("weread: source operation=chapter_read serial_fallback=1");
                }
                Ok(ChapterAttempt::Complete(content)) => {
                    let retained_bytes = content
                        .html
                        .len()
                        .checked_add(content.plain_text.len())
                        .and_then(|bytes| {
                            bytes.checked_add(content.style.as_ref().map_or(0, String::len))
                        });
                    if response_limit.is_some()
                        && retained_bytes.is_none_or(|bytes| bytes > MAX_PREFETCH_CONTENT_BYTES)
                    {
                        if let Some(diagnostics) = &auth.download_diagnostics {
                            diagnostics.budget_exceeded(
                                ResponsePhase::Result,
                                MAX_PREFETCH_CONTENT_BYTES,
                                retained_bytes.unwrap_or(usize::MAX),
                                None,
                            );
                        }
                        return Err(SourceError::ResponseTooLarge);
                    }
                    if super::report::account_key(&self.session()) != account {
                        hints.bind(self, book_id);
                        return Err(SourceError::Unsupported("chapter_account_changed"));
                    }
                    hints.confirmed_text =
                        hints.scope.is_some() && matches!(content.format, ContentFormat::Text);
                    return Ok(content);
                }
                Err(error) if retries < 2 && retry_chapter_read(&error) => {
                    retries += 1;
                    crate::diagnostic!(
                        "weread: source operation=chapter_read retry={retries} error={error}"
                    );
                    tokio::time::sleep(Duration::from_millis(800 * retries)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    async fn chapter_once(
        &self,
        book_id: &str,
        chapter_uid: &str,
        auth: &mut WebReadAuth,
        hints: &mut ChapterReadHints,
    ) -> Result<ChapterAttempt> {
        let before = self.session();
        let session = self
            .reader_session_with_auth(book_id, Some(chapter_uid), auth)
            .await?;
        if web_session_changed(&before, &self.session()) {
            hints.serial_epub.store(true, Ordering::SeqCst);
        }
        if session.access.needs_refresh() {
            // Renewal inside the Reader GET can return valid HTML after the
            // captured generation changed. Obtain one stable proof before any
            // content request, sharing this chapter's existing auth budget.
            return Ok(ChapterAttempt::RefreshAccess);
        }
        if hints.confirmed_text {
            return match self
                .text_chapter(book_id, chapter_uid, &session, auth)
                .await
            {
                Ok(content) => Ok(self.completed_chapter_attempt(&session, content).await),
                Err(error) if format_hint_mismatch(&error) && !auth.recovery_failed => {
                    hints.confirmed_text = false;
                    Ok(ChapterAttempt::ProbeFormat)
                }
                Err(error) => Err(error),
            };
        }
        let before = self.session();
        let first = self
            .shard(
                book_id,
                chapter_uid,
                &session,
                "/web/book/chapter/e_0",
                auth,
            )
            .await?;
        if web_session_changed(&before, &self.session()) {
            hints.serial_epub.store(true, Ordering::SeqCst);
        }
        let is_text = first.trim_start().starts_with('{')
            && serde_json::from_str::<Value>(&first)
                .ok()
                .is_some_and(|v| v.get("bookId").is_some());
        if is_text {
            let content = self
                .text_chapter(book_id, chapter_uid, &session, auth)
                .await?;
            Ok(self.completed_chapter_attempt(&session, content).await)
        } else {
            let (second, third) = if hints.serial_epub.load(Ordering::SeqCst) {
                let second = self
                    .shard(
                        book_id,
                        chapter_uid,
                        &session,
                        "/web/book/chapter/e_1",
                        auth,
                    )
                    .await?;
                let third = self
                    .shard(
                        book_id,
                        chapter_uid,
                        &session,
                        "/web/book/chapter/e_3",
                        auth,
                    )
                    .await?;
                (second, third)
            } else {
                let request = |endpoint| -> Result<RequestBuilder> {
                    session.validate_opaque()?;
                    Ok(self
                        .web_request(Method::POST, endpoint)?
                        .header("Referer", reader_url(book_id, Some(chapter_uid))?)
                        .json(&content_payload(
                            book_id,
                            chapter_uid,
                            &session.psvts,
                            false,
                            now_seconds(),
                        )?))
                };
                let pair = self
                    .web_text_pair(
                        request("/web/book/chapter/e_1")?,
                        request("/web/book/chapter/e_3")?,
                        auth,
                    )
                    .await?;
                if pair.session_changed {
                    // Once an endpoint rotates credentials, subsequent chapters
                    // in this task remain serial. The conflicting batch is never
                    // accepted as content or resolved by overwriting newer auth.
                    hints.serial_epub.store(true, Ordering::SeqCst);
                    if !auth.recovery_failed
                        && pair.result.as_ref().err().is_none_or(retry_chapter_read)
                    {
                        return Ok(ChapterAttempt::RetrySerial);
                    }
                }
                let (second, third) = pair.result?;
                validate_shard_response("/web/book/chapter/e_1", &second)?;
                validate_shard_response("/web/book/chapter/e_3", &third)?;
                (second, third)
            };
            let html = super::content::decode_shards(&[&first, &second, &third])?;
            let plain_text = html_to_text(&html);
            let content_status = self.reader_content_status(&session, &plain_text).await;
            if plain_text.trim().is_empty() {
                if content_status == crate::domain::ContentStatus::Locked {
                    return Err(SourceError::UnlockRequired);
                }
                return Err(SourceError::InvalidResponse("empty_chapter"));
            }
            let content = ChapterContent {
                html,
                plain_text,
                style: None,
                format: ContentFormat::Xhtml,
                content_status,
            };
            Ok(self.completed_chapter_attempt(&session, content).await)
        }
    }

    async fn text_chapter(
        &self,
        book_id: &str,
        chapter_uid: &str,
        session: &ReaderSession,
        auth: &mut WebReadAuth,
    ) -> Result<ChapterContent> {
        let t0 = self
            .shard(book_id, chapter_uid, session, "/web/book/chapter/t_0", auth)
            .await?;
        let t1 = match self
            .shard(book_id, chapter_uid, session, "/web/book/chapter/t_1", auth)
            .await
        {
            Ok(text) => text,
            Err(SourceError::Http(404)) => String::new(),
            Err(error) => return Err(error),
        };
        let plain_text = super::content::decode_shards(&[&t0, &t1])?;
        let content_status = self.reader_content_status(session, &plain_text).await;
        if plain_text.trim().is_empty() {
            if content_status == crate::domain::ContentStatus::Locked {
                return Err(SourceError::UnlockRequired);
            }
            return Err(SourceError::InvalidResponse("empty_chapter"));
        }
        let html = super::content::paragraph_html_checked(&plain_text);
        if matches!(&html, Err(SourceError::ResponseTooLarge)) {
            if let Some(diagnostics) = &auth.download_diagnostics {
                diagnostics.source_budget_exceeded(ResponsePhase::Result, plain_text.len());
            }
        }
        let html = html?;
        Ok(ChapterContent {
            html,
            plain_text,
            style: None,
            format: ContentFormat::Text,
            content_status,
        })
    }

    pub async fn progress(&self, book_id: &str) -> Result<ReadingPosition> {
        let mut budget = super::protocol::ReadingPositionBudget::new()
            .map_err(|_| SourceError::ResponseTooLarge)?;
        budget
            .fields(book_id.len(), 0, 0)
            .map_err(|_| SourceError::ResponseTooLarge)?;
        #[cfg(test)]
        position_budget_tests::observe_request_constructor();
        let result = self
            .gateway("/book/getprogress", json!({"bookId": book_id}))
            .await?;
        normalize_position_in(&result, book_id, &mut budget)
    }

    pub async fn report_progress(
        &self,
        book_id: &str,
        position: &ReadingPosition,
        elapsed_seconds: u32,
    ) -> Result<()> {
        let session = self
            .reader_session(book_id, Some(&position.chapter_uid))
            .await?;
        for elapsed in [None, Some(elapsed_seconds)] {
            let payload = read_payload(book_id, position, &session, elapsed, now_seconds())?;
            let sent = self.session();
            let response = self
                .web_request(Method::POST, "/web/book/read")?
                .header("Referer", reader_url(book_id, Some(&position.chapter_uid))?)
                .json(&payload)
                .send()
                .await
                .map_err(network_error)?;
            let cookie_result = self.observe_web_response(&sent, &response);
            let cookie_rejected = match elapsed {
                None => {
                    cookie_result?;
                    false
                }
                Some(_) => cookie_result.is_err(),
            };
            let result = response_json(response).await.map_err(|error| {
                if cookie_rejected {
                    SourceError::DeliveryUncertain
                } else {
                    error
                }
            })?;
            if !success(&result) && result.get("synckey").is_none() {
                return Err(if cookie_rejected {
                    SourceError::DeliveryUncertain
                } else {
                    SourceError::InvalidResponse("read_acknowledgement")
                });
            }
            if cookie_rejected {
                crate::diagnostic!(
                    "weread: source operation=progress delivery=acknowledged cookie_update=rejected_cookie_budget"
                );
            }
        }
        Ok(())
    }

    pub async fn reviews(&self, book_id: &str, latest: bool) -> Result<Vec<RemoteReview>> {
        self.reviews_with_output_limits(book_id, latest, ReviewOutputLimits::default())
            .await
    }

    async fn reviews_with_output_limits(
        &self,
        book_id: &str,
        latest: bool,
        limits: ReviewOutputLimits,
    ) -> Result<Vec<RemoteReview>> {
        let result = self.gateway("/review/list", json!({"bookId": book_id, "reviewListType": if latest { 3 } else { 1 }, "count": 30})).await?;
        let mut budget = ReviewOutputBudget::new(limits);
        normalize_review_candidates(review_records(&result).iter(), &mut budget)
    }

    pub async fn underlines(
        &self,
        book_id: &str,
        chapter_uid: &str,
    ) -> Result<Vec<RemoteUnderline>> {
        let chapter_uid = numeric_or_string(chapter_uid);
        let result = self
            .gateway(
                "/book/underlines",
                json!({"bookId": book_id, "chapterUid": chapter_uid}),
            )
            .await?;
        normalize_underlines(&result, UnderlineLimits::default())
    }

    /// Apply the same resource policy to older locally cached underline lists.
    /// This does no network work and never changes authentication state.
    pub(crate) fn validate_underlines(underlines: &[RemoteUnderline]) -> Result<()> {
        let limits = UnderlineLimits::default();
        if underlines.len() > limits.records {
            return Err(SourceError::ResponseTooLarge);
        }
        let mut total = 0;
        for underline in underlines {
            admit_underline_bytes(
                &mut total,
                underline.range.len(),
                underline.text.len(),
                limits,
            )?;
        }
        Ok(())
    }

    /// Normalize an already SQL-admitted cache without copying its raw records.
    /// Cached `text` and `thoughts` take precedence over optional wire aliases.
    pub(crate) fn normalize_cached_underlines(records: &[Value]) -> Result<Vec<RemoteUnderline>> {
        normalize_underline_records(
            records,
            UnderlineLimits::default(),
            CACHED_UNDERLINE_TEXT_FIELDS,
            CACHED_UNDERLINE_THOUGHT_FIELDS,
        )
    }

    pub async fn thoughts(
        &self,
        book_id: &str,
        chapter_uid: &str,
        ranges: &[String],
    ) -> Result<Vec<RemoteReview>> {
        self.thoughts_with_output_limits(
            book_id,
            chapter_uid,
            ranges,
            ReviewOutputLimits::default(),
        )
        .await
    }

    async fn thoughts_with_output_limits(
        &self,
        book_id: &str,
        chapter_uid: &str,
        ranges: &[String],
        limits: ReviewOutputLimits,
    ) -> Result<Vec<RemoteReview>> {
        let mut reviews = Vec::new();
        // One operation budget survives every response. A later rejection
        // returns an error and drops all earlier operation-local output.
        let mut budget = ReviewOutputBudget::new(limits);
        for batch in ranges.chunks(30) {
            let entries: Vec<Value> = batch
                .iter()
                .map(|range| json!({"range": range,"maxIdx":0,"count":30,"synckey":0}))
                .collect();
            let result = self.gateway("/book/readreviews", json!({"bookId":book_id,"chapterUid":numeric_or_string(chapter_uid),"reviews":entries})).await?;
            let admitted =
                normalize_review_candidates(thought_review_records(&result), &mut budget)?;
            reviews.extend(admitted);
        }
        Ok(reviews)
    }

    pub async fn statistics(
        &self,
        period: StatisticsPeriod,
        base_time: Option<i64>,
    ) -> Result<ReadStatistics> {
        self.statistics_with_output_limits(period, base_time, StatisticsOutputLimits::default())
            .await
    }

    async fn statistics_with_output_limits(
        &self,
        period: StatisticsPeriod,
        base_time: Option<i64>,
        limits: StatisticsOutputLimits,
    ) -> Result<ReadStatistics> {
        let mut params = json!({"mode":period.api_mode()});
        if let Some(time) = base_time.filter(|t| *t > 0) {
            params["baseTime"] = json!(time);
        }
        let result = self.gateway("/readdata/detail", params).await?;
        normalize_statistics(result, limits)
    }

    pub async fn mp_articles(
        &self,
        account_id: &str,
        max_index: u32,
        count: u32,
    ) -> Result<Vec<MpArticle>> {
        self.mp_articles_with_limits(account_id, max_index, count, MpArticleLimits::default())
            .await
    }

    async fn mp_articles_with_limits(
        &self,
        account_id: &str,
        max_index: u32,
        count: u32,
        limits: MpArticleLimits,
    ) -> Result<Vec<MpArticle>> {
        // The borrowed owner passes before query encoding or owned request
        // construction. A 4 KiB input needs at most 12 KiB of percent encoding;
        // no upstream identifier regex or oversized-URI behavior is assumed.
        validate_mp_owner(account_id, limits)?;
        let request = self.web_request(Method::GET, "/web/mp/articles")?.query(&[
            ("bookId", account_id.to_owned()),
            ("maxIdx", max_index.to_string()),
            ("count", count.clamp(1, 100).to_string()),
        ]);
        let result = self.web_json_read(request).await?;
        normalize_mp_articles(&result, account_id, limits)
    }

    pub async fn mp_content(&self, article_id: &str) -> Result<String> {
        let request = self
            .web_request(Method::GET, "/web/mp/content")?
            .query(&[("reviewId", article_id)])
            .header("Accept", "text/html,application/xhtml+xml,*/*");
        let mut auth = WebReadAuth::default();
        self.web_text_read(request, &mut auth, false).await
    }

    /// Downloads a public cover/image without account headers or cross-origin cookies.
    pub async fn public_image(&self, url: &str, maximum_bytes: usize) -> Result<Vec<u8>> {
        super::protocol::admit_resource_url(url, super::protocol::ResourceUrlStage::Reused, 0)?;
        #[cfg(test)]
        reader_resource_budget_tests::observe_url_parse();
        let mut current =
            reqwest::Url::parse(url).map_err(|_| SourceError::InvalidResponse("image_url"))?;
        super::protocol::check_normalized_resource_url(current.as_str())?;
        for _ in 0..=5 {
            if current.scheme() != "https"
                || current.host_str().is_none()
                || current.username() != ""
                || current.password().is_some()
            {
                return Err(SourceError::InvalidResponse("image_url"));
            }
            super::protocol::admit_resource_url(
                current.as_str(),
                super::protocol::ResourceUrlStage::Reused,
                0,
            )?;
            #[cfg(test)]
            reader_resource_budget_tests::observe_url_copy(current.as_str().len());
            let target = current.clone();
            #[cfg(test)]
            let target = self.resource_test_destination(&current)?;
            let response = self
                .resource_http
                .get(target)
                .header("Accept", "image/*")
                .send()
                .await
                .map_err(network_error)?;
            if response.status().is_redirection() {
                let next = response
                    .headers()
                    .get("Location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or(SourceError::InvalidResponse("image_redirect"))?;
                super::protocol::admit_resource_redirect(&current, next)?;
                #[cfg(test)]
                reader_resource_budget_tests::observe_url_parse();
                current = current
                    .join(next)
                    .map_err(|_| SourceError::InvalidResponse("image_redirect"))?;
                super::protocol::check_normalized_resource_url(current.as_str())?;
                continue;
            }
            return response_bytes(response, maximum_bytes.min(MAX_RESPONSE_BYTES))
                .await
                .map_err(resource_error);
        }
        Err(SourceError::InvalidResponse("too_many_redirects"))
    }

    /// Fetch chapter archives or remote image bytes with the reference's Referer.
    /// Cookies are scoped to the reviewed primary and resource WeRead origins.
    pub async fn chapter_resource(
        &self,
        url: &str,
        referer: &str,
        maximum_bytes: usize,
    ) -> Result<Vec<u8>> {
        let mut current = resource_url(url)?;
        let mut credentials_allowed = credentialed_resource_origin(&current);
        let limit = maximum_bytes.min(32 * 1024 * 1024);
        if limit == 0 {
            return Err(SourceError::ResponseTooLarge);
        }
        for _ in 0..=5 {
            let request = self.resource_request(&current, referer, credentials_allowed)?;
            let response = if credentials_allowed && current.host_str() == Some("weread.qq.com") {
                self.web_response(request, true).await?
            } else {
                request.send().await.map_err(network_error)?
            };
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get("Location")
                    .and_then(|header| header.to_str().ok())
                    .ok_or(SourceError::InvalidResponse("resource_redirect"))?;
                super::protocol::admit_resource_redirect(&current, location)?;
                #[cfg(test)]
                reader_resource_budget_tests::observe_url_parse();
                let next = current
                    .join(location)
                    .map_err(|_| SourceError::InvalidResponse("resource_redirect"))?;
                current = resource_url(next.as_str())?;
                if !credentialed_resource_origin(&current) {
                    credentials_allowed = false;
                }
                continue;
            }
            if !response.status().is_success() {
                crate::diagnostic!(
                    "weread: source operation=chapter_resource status={}",
                    response.status().as_u16()
                );
            }
            return response_bytes(response, limit)
                .await
                .map_err(resource_error);
        }
        Err(SourceError::InvalidResponse("too_many_redirects"))
    }

    fn resource_request(
        &self,
        url: &reqwest::Url,
        referer: &str,
        credentials_allowed: bool,
    ) -> Result<RequestBuilder> {
        resource_url(url.as_str())?;
        super::protocol::admit_reader_referer(referer)?;
        #[cfg(test)]
        reader_resource_budget_tests::observe_url_parse();
        let referer_url = reqwest::Url::parse(referer)
            .map_err(|_| SourceError::InvalidResponse("resource_url"))?;
        if referer_url.scheme() != "https"
            || !referer_url.username().is_empty()
            || referer_url.password().is_some()
        {
            return Err(SourceError::InvalidResponse("resource_url"));
        }
        super::protocol::admit_reader_referer(referer_url.as_str())?;
        if referer_url.host_str() != Some("weread.qq.com")
            || referer_url.port_or_known_default() != Some(443)
        {
            return Err(SourceError::InvalidResponse("resource_referer"));
        }
        // Select transport and credential permission from the original URL,
        // including the complete cookie budget, before any test destination.
        let transport = self.resource_transport(url);
        let cookie = if credentials_allowed && credentialed_resource_origin(url) {
            let session = self.session();
            if !session.has_web_session() {
                return Err(SourceError::WebLoginRequired);
            }
            Some(cookie_header(&session.cookies)?)
        } else {
            None
        };
        #[cfg(test)]
        reader_resource_budget_tests::observe_url_copy(url.as_str().len());
        let request_url = url.clone();
        #[cfg(test)]
        let request_url = self.resource_test_destination(url)?;
        let mut request = transport
            .get(request_url)
            .header("Accept", "*/*")
            .header("Referer", referer);
        if let Some(cookie) = cookie {
            request = request.header("Cookie", cookie);
        }
        Ok(request)
    }

    /// Test fixtures still enter through the real HTTPS/reference/credential
    /// policy above. Only the transport destination may become local HTTP;
    /// every production build retains the original validated destination.
    #[cfg(test)]
    fn resource_test_destination(&self, url: &reqwest::Url) -> Result<reqwest::Url> {
        let Some(origin) = self.test_resource_origin.as_deref() else {
            return Ok(url.clone());
        };
        let mut destination = reqwest::Url::parse(origin)
            .map_err(|_| SourceError::InvalidResponse("resource_test_origin"))?;
        let loopback = match destination.host() {
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            _ => false,
        };
        if destination.scheme() != "http"
            || !loopback
            || !destination.username().is_empty()
            || destination.password().is_some()
            || destination.path() != "/"
            || destination.query().is_some()
            || destination.fragment().is_some()
        {
            return Err(SourceError::InvalidResponse("resource_test_origin"));
        }
        destination.set_path(url.path());
        destination.set_query(url.query());
        Ok(destination)
    }

    fn resource_transport(&self, url: &reqwest::Url) -> &Client {
        // Only the two reviewed HTTPS/443 resource authorities can use the
        // first-party pool. Redirects reselect transport on every destination;
        // this does not grant or restore permission to send cookies.
        if credentialed_resource_origin(url) {
            &self.http
        } else {
            &self.resource_http
        }
    }

    pub(super) fn web_request(&self, method: Method, path: &str) -> Result<RequestBuilder> {
        let session = self.session();
        if !session.has_web_session() {
            return Err(SourceError::WebLoginRequired);
        }
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.web_origin()))
            .header(
                "Accept-Encoding",
                if path.starts_with("/web/book/chapter/") {
                    "identity"
                } else {
                    "gzip"
                },
            )
            .header("Origin", WEB_ORIGIN)
            .header("Referer", format!("{WEB_ORIGIN}/"))
            .header("Cookie", cookie_header(&session.cookies)?);
        if !session.wr_ticket.is_empty() {
            request = request.header("x-wr-ticket", session.wr_ticket);
        }
        if !session.wr_wrpa.is_empty() {
            request = request.header("x-wrpa-0", session.wr_wrpa);
        }
        Ok(request)
    }
}

fn completed_chapter(attempt: ChapterAttempt) -> Result<ChapterContent> {
    match attempt {
        ChapterAttempt::Complete(content) => Ok(content),
        ChapterAttempt::RefreshAccess => Err(SourceError::ContentUnverified),
        ChapterAttempt::ProbeFormat | ChapterAttempt::RetrySerial => {
            Err(SourceError::Unsupported("chapter_restart_unavailable"))
        }
    }
}

fn format_hint_mismatch(error: &SourceError) -> bool {
    !error.requires_login()
        && !matches!(error, SourceError::Api(code) if code == "-10102")
        && matches!(
            error,
            SourceError::Integrity
                | SourceError::Api(_)
                | SourceError::Http(404)
                | SourceError::InvalidResponse(
                    "empty_chapter"
                        | "chapter_shard_json"
                        | "chapter_base64"
                        | "chapter_utf8"
                        | "utf8"
                )
        )
}

fn retry_chapter_read(error: &SourceError) -> bool {
    if error.requires_login() || matches!(error, SourceError::Api(code) if code == "-10102") {
        return false;
    }
    matches!(
        error,
        SourceError::Integrity
            | SourceError::Api(_)
            | SourceError::Network
            | SourceError::Timeout
            | SourceError::Http(429 | 500..=599)
            | SourceError::InvalidResponse(
                "reader_session"
                    | "empty_chapter"
                    | "chapter_shard_json"
                    | "chapter_base64"
                    | "chapter_utf8"
                    | "utf8"
            )
    )
}

fn resource_url(value: &str) -> Result<reqwest::Url> {
    super::protocol::admit_resource_url(value, super::protocol::ResourceUrlStage::Reused, 0)?;
    #[cfg(test)]
    reader_resource_budget_tests::observe_url_parse();
    let url =
        reqwest::Url::parse(value).map_err(|_| SourceError::InvalidResponse("resource_url"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(SourceError::InvalidResponse("resource_url"));
    }
    super::protocol::check_normalized_resource_url(url.as_str())?;
    Ok(url)
}

#[cfg(test)]
#[path = "reader_resource_budget_tests.rs"]
pub(crate) mod reader_resource_budget_tests;

#[cfg(test)]
#[path = "position_budget_tests.rs"]
pub(crate) mod position_budget_tests;

#[cfg(test)]
#[path = "chapter_tests.rs"]
mod chapter_tests;

#[cfg(test)]
#[path = "network_performance_tests.rs"]
mod network_performance_tests;

#[cfg(test)]
#[path = "cookie_budget_tests.rs"]
mod cookie_budget_tests;

#[cfg(test)]
#[path = "underline_budget_tests.rs"]
mod underline_budget_tests;

#[cfg(test)]
#[path = "cached_underline_budget_tests.rs"]
mod cached_underline_budget_tests;

#[cfg(test)]
#[path = "../jobs_cached_transport_tests.rs"]
mod cached_download_transport_tests;

#[cfg(test)]
#[path = "../jobs_concurrency_tests.rs"]
mod jobs_concurrency_tests;

#[cfg(test)]
#[path = "review_output_budget_tests.rs"]
mod review_output_budget_tests;

#[cfg(test)]
#[path = "book_output_budget_tests.rs"]
mod book_output_budget_tests;

#[cfg(test)]
#[path = "statistics_output_budget_tests.rs"]
mod statistics_output_budget_tests;

#[cfg(test)]
#[path = "../jobs_zip_timestamp_tests.rs"]
mod jobs_zip_timestamp_tests;

#[cfg(test)]
#[path = "../jobs_svg_guard_tests.rs"]
mod jobs_svg_guard_tests;

fn credentialed_resource_origin(url: &reqwest::Url) -> bool {
    // The pinned upstream authenticates WeRead-owned subdomains. The private
    // trial verified this specific resource origin; other CDNs remain public.
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.host_str(), Some("weread.qq.com" | "res.weread.qq.com"))
}

fn resource_error(error: SourceError) -> SourceError {
    match error {
        SourceError::Http(status) => SourceError::ResourceHttp(status),
        error => error,
    }
}

pub(crate) fn http_client() -> Result<Client> {
    http_client_builder(FIRST_PARTY_IDLE_PER_HOST)
        .build()
        .map_err(network_error)
}

fn unpooled_http_client() -> Result<Client> {
    http_client_builder(0).build().map_err(network_error)
}

fn http_client_builder(max_idle_per_host: usize) -> ClientBuilder {
    Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(25))
        .pool_idle_timeout(Duration::from_secs(HTTP_POOL_IDLE_SECONDS))
        .pool_max_idle_per_host(max_idle_per_host)
        .redirect(reqwest::redirect::Policy::none())
        .default_headers({
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                "Accept-Encoding",
                reqwest::header::HeaderValue::from_static("identity"),
            );
            headers
        })
}

#[cfg(test)]
#[path = "http_pool_budget_tests.rs"]
mod http_pool_budget_tests;

pub(crate) fn network_error(error: reqwest::Error) -> SourceError {
    // Reqwest errors can contain private query parameters, so never retain their text.
    if error.is_timeout() {
        SourceError::Timeout
    } else {
        SourceError::Network
    }
}

pub(crate) async fn response_bytes(response: Response, limit: usize) -> Result<Vec<u8>> {
    response_bytes_observed(response, limit, |_, _| {}).await
}

async fn response_bytes_observed(
    mut response: Response,
    limit: usize,
    mut budget_exceeded: impl FnMut(usize, Option<u64>),
) -> Result<Vec<u8>> {
    if !response.status().is_success() {
        return Err(SourceError::Http(response.status().as_u16()));
    }
    let declared_bytes = response.content_length();
    if declared_bytes.is_some_and(|len| len > limit as u64) {
        budget_exceeded(0, declared_bytes);
        return Err(SourceError::ResponseTooLarge);
    }
    let mut bytes = Vec::new();
    // Reqwest removes compressed Content-Length while decoding gzip. Every
    // chunk is still bounded here after decompression, including small bombs.
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        let observed_bytes = bytes.len().saturating_add(chunk.len());
        if observed_bytes > limit {
            budget_exceeded(observed_bytes, declared_bytes);
            return Err(SourceError::ResponseTooLarge);
        }
        if bytes.capacity() < observed_bytes {
            // Keep geometric growth within the admitted decoded allowance;
            // Vec's default growth can otherwise exceed a non-power-of-two cap.
            let capacity = bytes
                .capacity()
                .saturating_mul(2)
                .max(observed_bytes)
                .min(limit);
            bytes.reserve_exact(capacity - bytes.len());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(crate) async fn response_text(response: Response) -> Result<String> {
    response_text_limited(response, None).await
}

pub(super) async fn response_text_limited(
    response: Response,
    limit: Option<usize>,
) -> Result<String> {
    let maximum = limit.unwrap_or(MAX_RESPONSE_BYTES).min(MAX_RESPONSE_BYTES);
    String::from_utf8(response_bytes(response, maximum).await?)
        .map_err(|_| SourceError::InvalidResponse("utf8"))
}

pub(super) async fn response_text_limited_observed(
    response: Response,
    limit: Option<usize>,
    diagnostics: Option<&Arc<DownloadDiagnostics>>,
    phase: Option<ResponsePhase>,
) -> Result<String> {
    let maximum = limit.unwrap_or(MAX_RESPONSE_BYTES).min(MAX_RESPONSE_BYTES);
    let bytes = response_bytes_observed(response, maximum, |observed, declared| {
        if let (Some(diagnostics), Some(phase)) = (diagnostics, phase) {
            diagnostics.budget_exceeded(phase, maximum, observed, declared);
        }
    })
    .await?;
    String::from_utf8(bytes).map_err(|_| SourceError::InvalidResponse("utf8"))
}

pub(crate) async fn response_json(response: Response) -> Result<Value> {
    let text = response_text(response).await?;
    let value: Value =
        serde_json::from_str(&text).map_err(|_| SourceError::InvalidResponse("json"))?;
    validate_api(&value)?;
    Ok(value)
}

struct ApiCodeWriter {
    code: String,
    raw_bytes: usize,
    raw_zero: bool,
}

impl ApiCodeWriter {
    fn new() -> Self {
        Self {
            code: String::with_capacity(MAX_API_CODE_BYTES),
            raw_bytes: 0,
            raw_zero: true,
        }
    }

    fn complete(&self) -> bool {
        self.code.len() == MAX_API_CODE_BYTES
    }
}

impl Write for ApiCodeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        #[cfg(test)]
        api_error_gate_tests::observe_serialized_fragment(bytes.len());
        for byte in bytes {
            self.raw_zero &= self.raw_bytes == 0 && *byte == b'0';
            self.raw_bytes = self.raw_bytes.saturating_add(1);
            if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_') {
                self.code.push(char::from(*byte));
                if self.complete() {
                    #[cfg(test)]
                    api_error_gate_tests::observe_serialization_stop();
                    return Err(io::Error::other("API error code prefix is complete"));
                }
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn admitted_api_code(value: &Value) -> Result<Option<String>> {
    if let Value::String(text) = value {
        if matches!(text.as_str(), "" | "0" | "null") {
            return Ok(None);
        }
        // Borrow the source span. Few admissible bytes can still require
        // scanning a long input; only retained diagnostic bytes are bounded.
        let code: String = text
            .bytes()
            .inspect(|_| {
                #[cfg(test)]
                api_error_gate_tests::observe_string_byte();
            })
            .filter(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_'))
            .take(MAX_API_CODE_BYTES)
            .map(char::from)
            .collect();
        #[cfg(test)]
        api_error_gate_tests::observe_retained_code(code.len());
        return Ok(Some(code));
    }
    if value.is_null() {
        return Ok(None);
    }
    // Use the same compact JSON serializer as Value::to_string, including
    // escaping, object order and numeric spelling. Its writer retains only
    // the sanitized prefix and stops serialization once that prefix is full.
    // The serializer may scan a complete string fragment before writing it;
    // this is not a constant-input-work or total parser-memory policy.
    let mut writer = ApiCodeWriter::new();
    let serialized = serde_json::to_writer(&mut writer, value);
    if serialized.is_err() && !writer.complete() {
        return Err(SourceError::InvalidResponse("api_error_code"));
    }
    // Only the complete raw number spelling "0" is a success. A composite
    // value whose sanitized output is "0", "null" or empty remains an error.
    if serialized.is_ok() && writer.raw_zero && writer.raw_bytes == 1 {
        return Ok(None);
    }
    #[cfg(test)]
    api_error_gate_tests::observe_retained_code(writer.code.len());
    Ok(Some(writer.code))
}

pub(crate) fn validate_api(value: &Value) -> Result<()> {
    if let Some(code) = value.get("errCode").or_else(|| value.get("errcode")) {
        if let Some(code) = admitted_api_code(code)? {
            return Err(SourceError::Api(code));
        }
    }
    if value
        .get("succ")
        .is_some_and(|v| v != true && v.as_u64() != Some(1) && v.as_str() != Some("1"))
    {
        return Err(SourceError::Api("rejected".into()));
    }
    Ok(())
}

#[cfg(test)]
#[path = "api_error_gate_tests.rs"]
mod api_error_gate_tests;

#[cfg(test)]
#[path = "api_gate_test_support.rs"]
pub(super) mod api_gate_test_support;

fn validate_shard_response(endpoint: &str, text: &str) -> Result<()> {
    let trimmed = text.trim();
    if trimmed == "{}" {
        return Err(SourceError::InvalidResponse("empty_chapter"));
    }
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        // Signed shards remain byte-for-byte unchanged for the strict decoder.
        return Ok(());
    }
    let value: Value = serde_json::from_str(trimmed)
        .map_err(|_| SourceError::InvalidResponse("chapter_shard_json"))?;
    validate_api(&value)?;
    if value.as_object().is_some_and(|object| object.is_empty()) {
        return Err(SourceError::InvalidResponse("empty_chapter"));
    }
    // The reference uses this one metadata response to select TXT shards.
    // A book identifier must not hide an error envelope or make JSON valid
    // on endpoints whose response contract is an encoded content shard.
    if endpoint == "/web/book/chapter/e_0" && !text_at(&value, &["bookId"]).is_empty() {
        return Ok(());
    }
    Err(SourceError::InvalidResponse("chapter_shard_json"))
}

pub(super) fn success(value: &Value) -> bool {
    value
        .get("succ")
        .is_some_and(|v| v == true || v.as_u64() == Some(1) || v.as_str() == Some("1"))
}

pub(crate) fn cookie_header(cookies: &BTreeMap<String, String>) -> Result<String> {
    validate_cookie_jar(cookies)?;
    Ok(cookies
        .iter()
        .filter(|(key, value)| {
            key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && !value.contains(['\r', '\n', ';'])
        })
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("; "))
}

fn cookie_budget_error() -> SourceError {
    SourceError::InvalidResponse("cookie_budget")
}

fn cookie_entry_bytes(name: &str, value: &str) -> Result<usize> {
    cookie_entry_length(name, value.len())
}

fn cookie_entry_length(name: &str, value_bytes: usize) -> Result<usize> {
    name.len()
        .checked_add(1)
        .and_then(|bytes| bytes.checked_add(value_bytes))
        .filter(|bytes| *bytes <= MAX_COOKIE_ENTRY_BYTES)
        .ok_or_else(cookie_budget_error)
}

/// Admit distinct replacement/addition lengths without copying their values
/// or the existing jar. Login uses fixed names and checks both overlay stages.
pub(super) fn validate_cookie_overlay(
    cookies: &BTreeMap<String, String>,
    updates: &[(&str, usize)],
) -> Result<()> {
    for (index, (name, _)) in updates.iter().enumerate() {
        if updates[..index]
            .iter()
            .any(|(previous, _)| previous == name)
        {
            return Err(cookie_budget_error());
        }
    }
    let count = cookies
        .len()
        .checked_add(
            updates
                .iter()
                .filter(|(name, _)| !cookies.contains_key(*name))
                .count(),
        )
        .filter(|count| *count <= MAX_COOKIE_COUNT)
        .ok_or_else(cookie_budget_error)?;
    let mut payload_bytes = 0usize;
    for (name, value) in cookies {
        if !updates.iter().any(|(updated, _)| *updated == name) {
            payload_bytes = payload_bytes
                .checked_add(cookie_entry_bytes(name, value)?)
                .ok_or_else(cookie_budget_error)?;
        }
    }
    for (name, value_bytes) in updates {
        payload_bytes = payload_bytes
            .checked_add(cookie_entry_length(name, *value_bytes)?)
            .ok_or_else(cookie_budget_error)?;
    }
    validate_cookie_totals(count, payload_bytes)
}

fn cookie_payload_bytes(cookies: &BTreeMap<String, String>) -> Result<usize> {
    cookies.iter().try_fold(0usize, |total, (name, value)| {
        total
            .checked_add(cookie_entry_bytes(name, value)?)
            .ok_or_else(cookie_budget_error)
    })
}

fn validate_cookie_totals(count: usize, payload_bytes: usize) -> Result<()> {
    let wire_bytes = count
        .saturating_sub(1)
        .checked_mul(2)
        .and_then(|separators| payload_bytes.checked_add(separators))
        .ok_or_else(cookie_budget_error)?;
    if count > MAX_COOKIE_COUNT || wire_bytes > MAX_COOKIE_TOTAL_BYTES {
        return Err(cookie_budget_error());
    }
    Ok(())
}

/// Bound retained cookie state, including separators in its outgoing header.
/// The policy rejects a whole update instead of evicting authentication cookies.
pub(crate) fn validate_cookie_jar(cookies: &BTreeMap<String, String>) -> Result<()> {
    if cookies.len() > MAX_COOKIE_COUNT {
        return Err(cookie_budget_error());
    }
    validate_cookie_totals(cookies.len(), cookie_payload_bytes(cookies)?)
}

pub(crate) fn merge_response_cookies(
    cookies: &mut BTreeMap<String, String>,
    headers: &reqwest::header::HeaderMap,
) -> Result<()> {
    validate_cookie_jar(cookies)?;
    // Borrow update payloads from the bounded transport response. This map has
    // its own distinct-name limit, so staging a rejected batch cannot grow an
    // unbounded collection before the retained-jar check. Last occurrence wins.
    let mut updates = BTreeMap::new();
    for header in headers.get_all(reqwest::header::SET_COOKIE) {
        let Some((name, value)) = header
            .to_str()
            .ok()
            .and_then(|s| s.split(';').next())
            .and_then(|s| s.split_once('='))
        else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if !name.starts_with("wr_") && !matches!(name, "ptcz" | "RK" | "pgv_pvid") {
            continue;
        }
        cookie_entry_bytes(name, value)?;
        if value.is_empty() && !cookies.contains_key(name) {
            updates.remove(name);
        } else {
            updates.insert(name, value);
        }
        if updates.len() > MAX_COOKIE_UPDATE_NAMES {
            return Err(cookie_budget_error());
        }
    }
    let mut count = cookies.len();
    let mut payload_bytes = cookie_payload_bytes(cookies)?;
    for (name, value) in &updates {
        if let Some(previous) = cookies.get(*name) {
            count -= 1;
            payload_bytes -= cookie_entry_bytes(name, previous)?;
        }
        if !value.is_empty() {
            count = count.checked_add(1).ok_or_else(cookie_budget_error)?;
            payload_bytes = payload_bytes
                .checked_add(cookie_entry_bytes(name, value)?)
                .ok_or_else(cookie_budget_error)?;
        }
    }
    validate_cookie_totals(count, payload_bytes)?;
    if !updates.is_empty() {
        let mut candidate = cookies.clone();
        for (name, value) in updates {
            if value.is_empty() {
                candidate.remove(name);
            } else {
                candidate.insert(name.into(), value.into());
            }
        }
        *cookies = candidate;
    }
    Ok(())
}

fn numeric_or_string(value: &str) -> Value {
    value
        .parse::<u64>()
        .map(Value::from)
        .unwrap_or_else(|_| Value::String(value.into()))
}

pub(crate) fn text_at(value: &Value, fields: &[&str]) -> String {
    fields
        .iter()
        .find_map(|field| {
            value.get(field).and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
        })
        .unwrap_or_default()
}

fn number_at(value: &Value, fields: &[&str]) -> u32 {
    fields
        .iter()
        .find_map(|field| value.get(field).map(|v| super::content::number(Some(v))))
        .unwrap_or_default()
}

fn time_at(value: &Value, fields: &[&str]) -> i64 {
    fields
        .iter()
        .find_map(|field| {
            value.get(field).and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
        })
        .unwrap_or_default()
}

fn book_info_node(value: &Value) -> &Value {
    value
        .get("bookInfo")
        .or_else(|| value.get("book"))
        .unwrap_or(value)
}

/// Materialize only after the complete response's immutable output admission.
fn materialize_book(value: &Value) -> RemoteBook {
    #[cfg(test)]
    book_output_budget_tests::observe_materialization();
    let info = book_info_node(value);
    let fields = book_fields(value);
    let progress = number_at(value, &["progress", "readingProgress"])
        .max(number_at(info, &["progress", "readingProgress"]))
        .min(100);
    RemoteBook {
        id: fields.id.into_owned(),
        title: fields.title.into_owned(),
        author: fields.author.into_owned(),
        cover_url: normalize_cover_url(&fields.cover),
        category: fields.category.into_owned(),
        intro: fields.intro.into_owned(),
        publisher: fields.publisher.into_owned(),
        isbn: fields.isbn.into_owned(),
        chapter_count: number_at(info, &["chapterSize", "chapterCount"]),
        progress,
        finished: progress >= 100 || number_at(info, &["finished"]) == 1,
        updated_at: time_at(value, &["updateTime", "readUpdateTime"])
            .max(time_at(info, &["updateTime", "lastChapterCreateTime"])),
        raw: value.clone(),
    }
}

fn normalize_position(value: &Value, book_id: &str) -> Result<ReadingPosition> {
    let mut budget =
        super::protocol::ReadingPositionBudget::new().map_err(|_| SourceError::ResponseTooLarge)?;
    budget
        .fields(book_id.len(), 0, 0)
        .map_err(|_| SourceError::ResponseTooLarge)?;
    normalize_position_in(value, book_id, &mut budget)
}

fn position_text_node<'a>(value: &'a Value, fields: &[&str]) -> Option<&'a Value> {
    fields.iter().find_map(|field| {
        value
            .get(field)
            .filter(|value| matches!(value, Value::String(_) | Value::Number(_)))
    })
}

fn position_text_bytes(value: Option<&Value>) -> usize {
    match value {
        Some(Value::String(value)) => value.len(),
        // The non-arbitrary-precision Number formatter uses a prepaid inline
        // 64-byte token; its canonical output is copied only after admission.
        Some(Value::Number(_)) => 64,
        _ => 0,
    }
}

fn position_scalar_work(
    value: Option<&Value>,
    budget: &mut super::protocol::ReadingPositionBudget,
) -> Result<()> {
    budget
        .scalar(position_text_bytes(value))
        .map_err(|_| SourceError::ResponseTooLarge)
}

fn position_text_owned(value: Option<&Value>) -> String {
    #[cfg(test)]
    if matches!(value, Some(Value::Number(_))) {
        position_budget_tests::observe_number_format();
    }
    let text = value
        .and_then(borrowed_scalar_text)
        .unwrap_or(ScalarText::Borrowed(""));
    #[cfg(test)]
    position_budget_tests::observe_field_copy(text.len());
    text.into_owned()
}

fn normalize_position_in(
    value: &Value,
    book_id: &str,
    budget: &mut super::protocol::ReadingPositionBudget,
) -> Result<ReadingPosition> {
    fn find_node<'a>(
        value: &'a Value,
        book_id: &str,
        depth: u8,
        budget: &mut super::protocol::ReadingPositionBudget,
    ) -> Result<Option<&'a Value>> {
        if depth > 8 {
            return Ok(None);
        }
        budget.node().map_err(|_| SourceError::ResponseTooLarge)?;
        #[cfg(test)]
        position_budget_tests::observe_search_node();
        let raw_id = position_text_node(value, &["bookId", "book_id"]);
        let id_bytes = position_text_bytes(raw_id);
        budget
            .check_field(super::ReadingPositionLimitKind::Uid, id_bytes)
            .map_err(|_| SourceError::ResponseTooLarge)?;
        budget
            .work(id_bytes.saturating_mul(2))
            .map_err(|_| SourceError::ResponseTooLarge)?;
        let matches_book = {
            #[cfg(test)]
            if matches!(raw_id, Some(Value::Number(_))) {
                position_budget_tests::observe_number_format();
            }
            let id = raw_id
                .and_then(borrowed_scalar_text)
                .unwrap_or(ScalarText::Borrowed(""));
            #[cfg(test)]
            position_budget_tests::observe_book_comparison();
            id.is_empty() || id.as_str() == book_id
        };
        if matches_book
            && (value
                .get("progress")
                .is_some_and(|v| v.is_number() || v.is_string())
                || value.get("readingProgress").is_some()
                || value.get("bookProgress").is_some())
        {
            return Ok(Some(value));
        }
        match value {
            Value::Object(map) => {
                for child in map.values() {
                    if let Some(node) = find_node(child, book_id, depth + 1, budget)? {
                        return Ok(Some(node));
                    }
                }
                Ok(None)
            }
            Value::Array(items) => {
                for child in items {
                    if let Some(node) = find_node(child, book_id, depth + 1, budget)? {
                        return Ok(Some(node));
                    }
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }
    let node = find_node(value, book_id, 0, budget)?
        .ok_or(SourceError::InvalidResponse("progress_not_found"))?;
    budget
        .work(4096)
        .map_err(|_| SourceError::ResponseTooLarge)?;
    let mut progress = None;
    for key in ["progress", "readingProgress", "bookProgress"] {
        if let Some(value) = node.get(key) {
            position_scalar_work(Some(value), budget)?;
            #[cfg(test)]
            position_budget_tests::observe_numeric_parse();
            progress = value
                .as_f64()
                .or_else(|| value.as_str().and_then(|value| value.parse::<f64>().ok()));
            if progress.is_some() {
                break;
            }
        }
    }
    let progress = progress
        .filter(|value| value.is_finite())
        .ok_or(SourceError::InvalidResponse("progress_value"))?;
    let uid = position_text_node(node, &["chapterUid", "chapterId", "chapter_uid"]);
    let summary = position_text_node(node, &["summary", "chapterTitle"]);
    let device = position_text_node(node, &["deviceName", "device"]);
    budget
        .fields(
            position_text_bytes(uid),
            position_text_bytes(summary),
            position_text_bytes(device),
        )
        .map_err(|_| SourceError::ResponseTooLarge)?;
    let numeric_node = |fields: &[&str]| fields.iter().find_map(|field| node.get(field));
    let chapter_index = numeric_node(&["chapterIdx", "chapterIndex", "chapter_idx"]);
    let chapter_offset = numeric_node(&["chapterOffset", "chapterPos", "offset", "chapter_offset"]);
    position_scalar_work(chapter_index, budget)?;
    position_scalar_work(chapter_offset, budget)?;
    let mut updated_at = None;
    for key in ["updateTime", "updatedAt", "update_time"] {
        if let Some(value) = node.get(key) {
            position_scalar_work(Some(value), budget)?;
            #[cfg(test)]
            position_budget_tests::observe_numeric_parse();
            updated_at = value
                .as_i64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()));
            if updated_at.is_some() {
                break;
            }
        }
    }
    #[cfg(test)]
    if matches!(chapter_index, Some(Value::String(_) | Value::Number(_))) {
        position_budget_tests::observe_numeric_parse();
    }
    let chapter_index = super::content::number(chapter_index);
    #[cfg(test)]
    if matches!(chapter_offset, Some(Value::String(_) | Value::Number(_))) {
        position_budget_tests::observe_numeric_parse();
    }
    let chapter_offset = super::content::number(chapter_offset);
    #[cfg(test)]
    position_budget_tests::observe_materialization();
    Ok(ReadingPosition {
        chapter_uid: position_text_owned(uid),
        chapter_index,
        chapter_offset,
        progress: progress.clamp(0.0, 100.0) as u32,
        summary: position_text_owned(summary),
        updated_at: updated_at.unwrap_or_default(),
        device: position_text_owned(device),
    })
}

/// Materialize only after the complete response's immutable output admission.
fn materialize_review(value: &Value) -> RemoteReview {
    #[cfg(test)]
    review_output_budget_tests::observe_materialization();
    let fields = review_fields(value);
    let node = fields.node;
    let star = number_at(node, &["star"]);
    let star = if star > 10 { star / 10 } else { star };
    RemoteReview {
        id: fields.id.into_owned(),
        author: fields.author.into_owned(),
        text: html_to_text(&fields.text),
        likes: number_at(node, &["likesCount", "likeCount", "likes"]),
        published_at: time_at(node, &["createTime", "time"]),
        recommended: star >= 3 || number_at(node, &["rating"]) >= 3,
    }
}

fn validate_mp_owner(account_id: &str, limits: MpArticleLimits) -> Result<()> {
    validate_mp_owner_bytes(account_id, limits.owner_bytes)
}

fn validate_mp_owner_bytes(account_id: &str, maximum_bytes: usize) -> Result<()> {
    if account_id.len() > maximum_bytes {
        return Err(SourceError::ResponseTooLarge);
    }
    if account_id.is_empty() || account_id.chars().any(char::is_control) {
        return Err(SourceError::InvalidResponse("mp_owner"));
    }
    Ok(())
}

fn borrowed_text_at<'a>(value: &'a Value, fields: &[&str]) -> Cow<'a, str> {
    fields
        .iter()
        .find_map(|field| match value.get(field) {
            Some(Value::String(text)) => Some(Cow::Borrowed(text.as_str())),
            // Number's non-arbitrary-precision representation is bounded.
            Some(Value::Number(number)) => Some(Cow::Owned(number.to_string())),
            _ => None,
        })
        .unwrap_or(Cow::Borrowed(""))
}

pub(super) struct InlineScalarNumber {
    bytes: [u8; 64],
    length: usize,
}

impl std::fmt::Write for InlineScalarNumber {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        let end = self
            .length
            .checked_add(value.len())
            .ok_or(std::fmt::Error)?;
        if end > self.bytes.len() {
            return Err(std::fmt::Error);
        }
        self.bytes[self.length..end].copy_from_slice(value.as_bytes());
        self.length = end;
        Ok(())
    }
}

/// Strings stay borrowed. The admitted JSON Number type has a bounded
/// non-arbitrary-precision spelling, formatted into inline token metadata
/// rather than an owned field String before a resource gate.
pub(super) enum ScalarText<'a> {
    Borrowed(&'a str),
    Number(InlineScalarNumber),
}

impl ScalarText<'_> {
    pub(super) fn as_str(&self) -> &str {
        match self {
            Self::Borrowed(value) => value,
            Self::Number(value) => {
                // write_str only appends complete UTF-8 spans.
                std::str::from_utf8(&value.bytes[..value.length]).unwrap_or_default()
            }
        }
    }

    pub(super) fn into_owned(self) -> String {
        self.as_str().to_owned()
    }
}

impl std::ops::Deref for ScalarText<'_> {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq for ScalarText<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for ScalarText<'_> {}

impl std::hash::Hash for ScalarText<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(self.as_str(), state);
    }
}

pub(super) fn borrowed_scalar_text(value: &Value) -> Option<ScalarText<'_>> {
    match value {
        Value::String(value) => Some(ScalarText::Borrowed(value)),
        Value::Number(value) => {
            let mut number = InlineScalarNumber {
                bytes: [0; 64],
                length: 0,
            };
            std::fmt::write(&mut number, format_args!("{value}")).ok()?;
            Some(ScalarText::Number(number))
        }
        _ => None,
    }
}

pub(super) fn scalar_text_at<'a>(value: &'a Value, fields: &[&str]) -> ScalarText<'a> {
    fields
        .iter()
        .find_map(|field| value.get(field).and_then(borrowed_scalar_text))
        .unwrap_or(ScalarText::Borrowed(""))
}

fn mp_account_metadata_fee(owner: &str, info: &Value) -> Result<usize> {
    let text_bytes = owner
        .len()
        .checked_add(selected_text_bytes(info, &["title"]))
        .and_then(|bytes| bytes.checked_add(selected_text_bytes(info, &["cover", "coverUrl"])))
        .ok_or(SourceError::ResponseTooLarge)?;
    // The cover normalization only shortens its selected input. Reserve two
    // normalized snapshots plus the owner membership forecast before copying;
    // this is a project operation allowance, not a transport or total RSS cap.
    text_bytes
        .checked_add(std::mem::size_of::<MpAccount>())
        .and_then(|bytes| bytes.checked_mul(2))
        .and_then(|bytes| bytes.checked_add(owner.len()))
        .ok_or(SourceError::ResponseTooLarge)
}

fn normalize_shelf_mp_accounts(
    records: &[Value],
    limits: MpAccountLimits,
) -> Result<Vec<MpAccount>> {
    normalize_shelf_mp_accounts_with_observer(records, limits, |_| {})
}

fn normalize_shelf_mp_accounts_with_observer(
    records: &[Value],
    limits: MpAccountLimits,
    mut before_copy: impl FnMut(&str),
) -> Result<Vec<MpAccount>> {
    let mut candidates = 0usize;
    let mut metadata_bytes = 0usize;
    // Price every selected raw MP record, including duplicates, before any
    // output field is copied. Empty and other-prefix IDs retain their original
    // Shelf classification and do not become publication candidates.
    for record in records {
        let info = book_info_node(record);
        let owner = borrowed_text_at(info, &["bookId", "book_id"]);
        if !owner.starts_with("MP_WXS_") {
            continue;
        }
        candidates = candidates
            .checked_add(1)
            .filter(|count| *count <= limits.records)
            .ok_or(SourceError::ResponseTooLarge)?;
        validate_mp_owner_bytes(&owner, limits.owner_bytes)?;
        metadata_bytes = metadata_bytes
            .checked_add(mp_account_metadata_fee(&owner, info)?)
            .filter(|bytes| *bytes <= limits.metadata_bytes)
            .ok_or(SourceError::ResponseTooLarge)?;
    }
    let mut accounts = Vec::new();
    let mut seen = BTreeSet::new();
    for record in records {
        let info = book_info_node(record);
        // Only a string can have the existing MP prefix. Borrow it directly
        // so membership never copies a duplicate owner or its metadata.
        let Cow::Borrowed(owner) = borrowed_text_at(info, &["bookId", "book_id"]) else {
            continue;
        };
        if !owner.starts_with("MP_WXS_") || !seen.insert(owner) {
            continue;
        }
        before_copy(owner);
        accounts.push(MpAccount {
            id: owner.into(),
            name: text_at(info, &["title"]),
            cover_url: normalize_cover_url(&text_at(info, &["cover", "coverUrl"])),
            updated_at: time_at(record, &["updateTime", "readUpdateTime"])
                .max(time_at(info, &["updateTime", "lastChapterCreateTime"])),
            unread_count: number_at(record, &["unreadCount", "unread"]),
        });
    }
    Ok(accounts)
}

#[cfg(test)]
#[path = "mp_account_budget_tests.rs"]
mod mp_account_budget_tests;

#[derive(Clone, Copy)]
struct MpArticleCandidate<'a> {
    outer: &'a Value,
    review: &'a Value,
    info: &'a Value,
}

fn mp_article_id<'a>(candidate: MpArticleCandidate<'a>) -> Cow<'a, str> {
    for (value, field) in [
        (candidate.outer, "reviewId"),
        (candidate.review, "reviewId"),
        (candidate.info, "originalId"),
    ] {
        let id = borrowed_text_at(value, &[field]);
        if !id.is_empty() {
            return id;
        }
    }
    Cow::Borrowed("")
}

fn visit_mp_article_candidates<'a>(
    value: &'a Value,
    depth: u8,
    visitor: &mut impl FnMut(MpArticleCandidate<'a>) -> Result<()>,
) -> Result<()> {
    if depth > 10 {
        return Ok(());
    }
    let node = value
        .get("review")
        .filter(|v| v.is_object())
        .unwrap_or(value);
    if let Some(mp) = node.get("mpInfo").filter(|v| v.is_object()) {
        return visitor(MpArticleCandidate {
            outer: value,
            review: node,
            info: mp,
        });
    }
    match value {
        Value::Object(map) => {
            for child in map.values() {
                visit_mp_article_candidates(child, depth + 1, visitor)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                visit_mp_article_candidates(child, depth + 1, visitor)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn mp_article_metadata_fee(account_id: &str, id: &str, info: &Value) -> Result<usize> {
    let text_bytes = [
        account_id.len(),
        id.len(),
        selected_text_bytes(info, &["title"]),
        selected_text_bytes(info, &["author", "authorName"]),
        selected_text_bytes(info, MP_ORIGINAL_URL_FIELDS),
        selected_text_bytes(info, MP_COVER_URL_FIELDS),
    ]
    .into_iter()
    .try_fold(0usize, |used, bytes| used.checked_add(bytes))
    .ok_or(SourceError::ResponseTooLarge)?;
    // Reserve two normalized record/text forecasts and the membership ID
    // forecast. This is a logical operation policy, not a total RSS budget.
    text_bytes
        .checked_add(std::mem::size_of::<MpArticle>())
        .and_then(|bytes| bytes.checked_mul(2))
        .and_then(|bytes| bytes.checked_add(id.len()))
        .ok_or(SourceError::ResponseTooLarge)
}

fn normalize_mp_articles(
    value: &Value,
    account_id: &str,
    limits: MpArticleLimits,
) -> Result<Vec<MpArticle>> {
    normalize_mp_articles_with_observer(value, account_id, limits, || {})
}

fn normalize_mp_articles_with_observer(
    value: &Value,
    account_id: &str,
    limits: MpArticleLimits,
    mut before_copy: impl FnMut(),
) -> Result<Vec<MpArticle>> {
    validate_mp_owner(account_id, limits)?;
    let mut records = 0usize;
    let mut metadata_bytes = 0usize;
    // Preflight the complete nested candidate set before copying any output
    // owner or field. Empty IDs still spend count admission, and duplicate
    // candidates cannot bypass this forecast by being discarded later.
    visit_mp_article_candidates(value, 0, &mut |candidate| {
        records = records
            .checked_add(1)
            .filter(|records| *records <= limits.records)
            .ok_or(SourceError::ResponseTooLarge)?;
        let id = mp_article_id(candidate);
        if id.is_empty() {
            return Ok(());
        }
        if id.len() > limits.id_bytes {
            return Err(SourceError::ResponseTooLarge);
        }
        if id.chars().any(char::is_control) {
            return Err(SourceError::InvalidResponse("mp_article_id"));
        }
        metadata_bytes = metadata_bytes
            .checked_add(mp_article_metadata_fee(account_id, &id, candidate.info)?)
            .filter(|bytes| *bytes <= limits.metadata_bytes)
            .ok_or(SourceError::ResponseTooLarge)?;
        Ok(())
    })?;
    let mut articles = Vec::new();
    let mut seen = BTreeSet::new();
    visit_mp_article_candidates(value, 0, &mut |candidate| {
        let id = mp_article_id(candidate);
        if id.is_empty() || !seen.insert(id.clone()) {
            return Ok(());
        }
        before_copy();
        articles.push(MpArticle {
            id: id.into_owned(),
            account_id: account_id.into(),
            title: text_at(candidate.info, &["title"]),
            author: text_at(candidate.info, &["author", "authorName"]),
            published_at: time_at(candidate.review, &["createTime", "time"])
                .max(time_at(candidate.info, &["createTime", "publishTime"])),
            original_url: text_at(candidate.info, MP_ORIGINAL_URL_FIELDS),
            cover_url: text_at(candidate.info, MP_COVER_URL_FIELDS),
        });
        Ok(())
    })?;
    Ok(articles)
}

#[cfg(test)]
#[path = "mp_article_budget_tests.rs"]
mod mp_article_budget_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_redacted_from_debug() {
        let session = AuthSession {
            api_key: "private-key".into(),
            cookies: BTreeMap::from([("wr_skey".into(), "private-session".into())]),
            ..Default::default()
        };
        let debug = format!("{session:?}");
        assert!(!debug.contains("private"));
        assert!(debug.contains("present"));
    }

    #[test]
    fn gateway_payload_errors_do_not_include_remote_messages() {
        let error = validate_api(&json!({"errCode":-100,"errMsg":"sensitive value"})).unwrap_err();
        assert_eq!(error.to_string(), "source.api.-100");
    }

    #[test]
    fn shard_json_business_errors_are_classified_before_format_detection() {
        for endpoint in [
            "/web/book/chapter/e_0",
            "/web/book/chapter/e_1",
            "/web/book/chapter/t_0",
        ] {
            for envelope in [
                json!({"errCode": -100, "errMsg": "fixture private message"}),
                json!({"bookId": "fixture-book", "errcode": "DENIED", "message": "fixture private message"}),
                json!({"bookId": "fixture-book", "succ": false, "message": "fixture private message"}),
            ] {
                let error = validate_shard_response(endpoint, &envelope.to_string()).unwrap_err();
                assert!(matches!(error, SourceError::Api(_)));
                assert!(!error.to_string().contains("fixture"));
                assert!(!error.to_string().contains("message"));
                assert!(!matches!(error, SourceError::Integrity));
            }
        }
        assert_eq!(
            validate_shard_response(
                "/web/book/chapter/e_0",
                r#"{"bookId":"fixture-book","errCode":-100,"errMsg":"fixture private message"}"#,
            )
            .unwrap_err()
            .to_string(),
            "source.api.-100"
        );
    }

    #[test]
    fn txt_metadata_is_allowed_only_on_the_initial_epub_probe() {
        for metadata in [
            r#"{"bookId":"fixture-book"}"#,
            " \n{\"bookId\":\"fixture-book\",\"errCode\":0,\"succ\":1}\r\n",
        ] {
            assert!(validate_shard_response("/web/book/chapter/e_0", metadata).is_ok());
            for endpoint in [
                "/web/book/chapter/e_1",
                "/web/book/chapter/e_2",
                "/web/book/chapter/e_3",
                "/web/book/chapter/t_0",
                "/web/book/chapter/t_1",
            ] {
                assert!(matches!(
                    validate_shard_response(endpoint, metadata),
                    Err(SourceError::InvalidResponse("chapter_shard_json"))
                ));
            }
        }
    }

    #[test]
    fn unknown_json_shards_are_rejected_without_exposing_their_payloads() {
        for envelope in [
            r#"{"succ":1,"message":"fixture private message"}"#,
            r#"{"bookId":null}"#,
            r#"{"bookId":""}"#,
            r#"{"bookId":{}}"#,
            r#"[{"bookId":"fixture-book"}]"#,
            "{malformed fixture payload",
        ] {
            let error = validate_shard_response("/web/book/chapter/e_0", envelope).unwrap_err();
            assert_eq!(
                error.to_string(),
                "source.invalid_response.chapter_shard_json"
            );
        }
        for empty in ["{}", " \n{}\r\n", "{ }"] {
            assert!(matches!(
                validate_shard_response("/web/book/chapter/e_0", empty),
                Err(SourceError::InvalidResponse("empty_chapter"))
            ));
        }
    }

    #[test]
    fn opaque_shards_still_require_the_original_integrity_check() {
        let shard = "DF1103CA0CD3800B1FA3F842EB367223aPGh0bWw+PJvGZHk+T4A+UmVhZGVyIGZpeHR1cwPPHC9mUjwvYm9keuP8L2h0bWw+";
        validate_shard_response("/web/book/chapter/e_0", shard).unwrap();
        assert_eq!(
            super::super::content::decode_shards(&[shard]).unwrap(),
            "<html><body><p>Reader fixture.</p></body></html>"
        );
        let corrupted = format!("{}{}", "0".repeat(32), &shard[32..]);
        validate_shard_response("/web/book/chapter/e_0", &corrupted).unwrap();
        assert!(matches!(
            super::super::content::decode_shards(&[&corrupted]),
            Err(SourceError::Integrity)
        ));
    }

    #[test]
    fn shelf_record_keeps_metadata_and_progress() {
        let book = normalize_single_book(
            &json!({"bookInfo":{"bookId":"book","title":"Fixture","cover":"https://cdn.example/t6_fixture"},"progress":42}),
            "book",
            BookOutputLimits::default(),
        ).unwrap();
        assert_eq!(book.id, "book");
        assert_eq!(book.progress, 42);
        assert_eq!(book.cover_url, "https://cdn.example/t9_fixture");
    }

    #[test]
    fn web_cookies_cannot_inject_headers() {
        assert_eq!(
            cookie_header(&BTreeMap::from([
                ("wr_skey".into(), "safe".into()),
                ("bad".into(), "value\r\nInjected: yes".into())
            ]))
            .unwrap(),
            "wr_skey=safe"
        );
    }

    #[test]
    fn remote_writes_are_not_advertised_without_a_contract() {
        let client = WeReadClient::new(AuthSession::default()).unwrap();
        assert!(!client.capabilities().remote_shelf_mutation);
        assert!(!client.capabilities().remote_annotation_write);
    }

    #[test]
    fn gateway_request_uses_exact_reference_contract_without_web_cookies() {
        let client = WeReadClient::new(AuthSession {
            api_key: "fixture-key".into(),
            cookies: BTreeMap::from([("wr_skey".into(), "fixture-cookie".into())]),
            ..Default::default()
        })
        .unwrap();
        let request = client
            .gateway_request("/shelf/sync", json!({}))
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(request.url().as_str(), GATEWAY_URL);
        assert_eq!(request.method(), Method::POST);
        assert_eq!(request.headers().get("Accept-Encoding").unwrap(), "gzip");
        assert_eq!(
            request.headers().get("Authorization").unwrap(),
            "Bearer fixture-key"
        );
        assert!(!request.headers().contains_key("Cookie"));
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"api_name":"/shelf/sync","skill_version":"1.0.5"})
        );
    }

    #[test]
    fn mp_article_keeps_parent_review_identifier() {
        let value = json!({"reviews":[{"subReviews":[{"reviewId":"fixture-article","review":{"mpInfo":{"title":"Fixture article","originalUrl":"https://mp.weixin.qq.com/fixture"}}}]}]});
        let rows =
            normalize_mp_articles(&value, "MP_WXS_fixture", MpArticleLimits::default()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "fixture-article");
    }

    #[test]
    fn chapter_resource_cookies_use_only_reviewed_weread_origins() {
        let client = WeReadClient::new(AuthSession {
            cookies: BTreeMap::from([("wr_skey".into(), "fixture-session".into())]),
            ..Default::default()
        })
        .unwrap();
        let referer = reader_url("fixture-book", Some("1")).unwrap();
        let internal = resource_url("https://weread.qq.com/fixture.tar").unwrap();
        let resource = resource_url("https://res.weread.qq.com/fixture.tar").unwrap();
        let cdn = resource_url("https://cdn.example/fixture.tar").unwrap();
        let trusted = client
            .resource_request(&internal, &referer, true)
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            trusted.headers().get("Cookie").unwrap(),
            "wr_skey=fixture-session"
        );
        let authenticated_resource = client
            .resource_request(&resource, &referer, true)
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            authenticated_resource.headers().get("Cookie").unwrap(),
            "wr_skey=fixture-session"
        );
        assert!(
            !authenticated_resource
                .headers()
                .contains_key("Authorization")
        );
        for value in [
            "https://res.weread.qq.com.attacker.invalid/fixture.tar",
            "https://res.weread.qq.com:444/fixture.tar",
            "https://unreviewed.weread.qq.com/fixture.tar",
        ] {
            let url = resource_url(value).unwrap();
            assert!(!credentialed_resource_origin(&url));
            let request = client
                .resource_request(&url, &referer, true)
                .unwrap()
                .build()
                .unwrap();
            assert!(!request.headers().contains_key("Cookie"));
            assert!(!request.headers().contains_key("Authorization"));
        }
        let external = client
            .resource_request(&cdn, &referer, true)
            .unwrap()
            .build()
            .unwrap();
        assert!(!external.headers().contains_key("Cookie"));
        assert!(!external.headers().contains_key("Authorization"));
        let redirected_back = client
            .resource_request(&internal, &referer, false)
            .unwrap()
            .build()
            .unwrap();
        assert!(!redirected_back.headers().contains_key("Cookie"));
        let redirected_resource = client
            .resource_request(&resource, &referer, false)
            .unwrap()
            .build()
            .unwrap();
        assert!(!redirected_resource.headers().contains_key("Cookie"));
        assert!(resource_url("http://weread.qq.com/fixture.tar").is_err());
    }

    #[test]
    fn progress_uses_reference_aliases_and_rejects_an_explicit_other_book() {
        let value = json!({"book":{"book_id":"fixture-book","readingProgress":22,"chapter_uid":"uid","chapter_idx":7,"chapter_offset":10,"updatedAt":90,"chapterTitle":"Fixture"}});
        let position = normalize_position(&value, "fixture-book").unwrap();
        assert_eq!(position.chapter_uid, "uid");
        assert_eq!(position.chapter_index, 7);
        assert_eq!(position.chapter_offset, 10);
        assert_eq!(position.updated_at, 90);
        assert!(normalize_position(&value, "other-book").is_err());
    }
}
