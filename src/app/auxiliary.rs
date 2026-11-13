use super::library::{admit_qr_payload, qr_image};
use super::*;

use crate::{
    article::{ArticleDocument, ArticleImageGeometry, ArticleLayoutOptions, ArticleQuoteGeometry},
    reader::Layout,
    statistics,
};
use chrono::{DateTime, Datelike, FixedOffset};
use slint::Model;

struct StatisticsView {
    period_label: String,
    seconds: Option<u64>,
    metrics: Vec<StatisticMetric>,
    bars: Vec<statistics::Bar>,
    ranking: Vec<statistics::Rank>,
    ranking_max_seconds: Option<u64>,
    calendar: Vec<i32>,
    calendar_offset: usize,
    calendar_label: String,
    preferences: Vec<statistics::Preference>,
    status: String,
    offset: i32,
    range_start: i64,
}

#[cfg(test)]
#[path = "../statistics_year_budget_tests.rs"]
mod statistics_year_budget_tests;

#[cfg(test)]
#[path = "qr_input_budget_tests.rs"]
mod qr_input_budget_tests;

const MAX_PUBLICATION_OWNER_BYTES: usize = 4096;
const MAX_PUBLICATION_ARTICLE_ID_BYTES: usize = 4096;
const MAX_PUBLICATION_ACCOUNT_RECORDS: usize = 8192;
const MAX_PUBLICATION_ACCOUNT_METADATA_BYTES: usize = 64 * 1024 * 1024;
const MAX_PUBLICATION_FILTER_WORK_BYTES: usize = 64 * 1024 * 1024;
const MAX_PUBLICATION_FILTER_SCRATCH_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub(super) struct PublicationFilterLimits {
    pub work_bytes: usize,
    pub scratch_bytes: usize,
}

impl Default for PublicationFilterLimits {
    fn default() -> Self {
        Self {
            work_bytes: MAX_PUBLICATION_FILTER_WORK_BYTES,
            scratch_bytes: MAX_PUBLICATION_FILTER_SCRATCH_BYTES,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error(
    "publication filter {resource} exceeds its allowance (limit {limit}, requested {requested})"
)]
pub(super) struct PublicationFilterLimit {
    pub resource: &'static str,
    pub limit: usize,
    pub requested: usize,
}

#[cfg(test)]
std::thread_local! {
    static PUBLICATION_FILTER_TEST_LIMITS: Cell<Option<PublicationFilterLimits>> = const { Cell::new(None) };
}

#[cfg(test)]
pub(super) fn with_publication_filter_test_limits<T>(
    limits: PublicationFilterLimits,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<PublicationFilterLimits>);
    impl Drop for Restore {
        fn drop(&mut self) {
            PUBLICATION_FILTER_TEST_LIMITS.with(|current| current.set(self.0));
        }
    }
    let _restore =
        Restore(PUBLICATION_FILTER_TEST_LIMITS.with(|current| current.replace(Some(limits))));
    run()
}

fn publication_filter_limits() -> PublicationFilterLimits {
    #[cfg(test)]
    if let Some(limits) = PUBLICATION_FILTER_TEST_LIMITS.with(Cell::get) {
        return limits;
    }
    PublicationFilterLimits::default()
}

struct PublicationFilterBudget {
    limits: PublicationFilterLimits,
    work_bytes: usize,
    peak_scratch_bytes: usize,
    preflight_chars: usize,
}

impl PublicationFilterBudget {
    fn new() -> Self {
        let limits = publication_filter_limits();
        Self {
            limits: PublicationFilterLimits {
                work_bytes: limits.work_bytes.min(MAX_PUBLICATION_FILTER_WORK_BYTES),
                scratch_bytes: limits
                    .scratch_bytes
                    .min(MAX_PUBLICATION_FILTER_SCRATCH_BYTES),
            },
            work_bytes: 0,
            peak_scratch_bytes: 0,
            preflight_chars: 0,
        }
    }

    fn charge_work(&mut self, bytes: usize) -> Result<()> {
        let next = self
            .work_bytes
            .checked_add(bytes)
            .ok_or(PublicationFilterLimit {
                resource: "transform work bytes",
                limit: self.limits.work_bytes,
                requested: usize::MAX,
            })?;
        if next > self.limits.work_bytes {
            return Err(PublicationFilterLimit {
                resource: "transform work bytes",
                limit: self.limits.work_bytes,
                requested: next,
            }
            .into());
        }
        self.work_bytes = next;
        Ok(())
    }

    fn admit(&mut self, first: &str, author: Option<&str>) -> Result<()> {
        let separator = if author.is_some() { " " } else { "" };
        let input_bytes = first
            .len()
            .checked_add(separator.len())
            .and_then(|bytes| bytes.checked_add(author.map_or(0, str::len)))
            .ok_or(PublicationFilterLimit {
                resource: "transform work bytes",
                limit: self.limits.work_bytes,
                requested: usize::MAX,
            })?;
        // Pay the complete borrowed Unicode preflight and input-copy/scan
        // allowance before visiting its first scalar. No String exists yet.
        self.charge_work(input_bytes.checked_mul(4).ok_or(PublicationFilterLimit {
            resource: "transform work bytes",
            limit: self.limits.work_bytes,
            requested: usize::MAX,
        })?)?;
        let mut lowercase_bytes_upper = 0usize;
        for part in [first, separator, author.unwrap_or_default()] {
            for character in part.chars() {
                self.preflight_chars =
                    self.preflight_chars
                        .checked_add(1)
                        .ok_or(PublicationFilterLimit {
                            resource: "transform work bytes",
                            limit: self.limits.work_bytes,
                            requested: usize::MAX,
                        })?;
                // Character mappings price UTF-8 capacity only. They are never
                // used as the matcher: the complete str retains contextual
                // lowercase rules, including Greek final sigma.
                let bytes = character.to_lowercase().count().checked_mul(4).ok_or(
                    PublicationFilterLimit {
                        resource: "transform work bytes",
                        limit: self.limits.work_bytes,
                        requested: usize::MAX,
                    },
                )?;
                self.charge_work(bytes.checked_mul(2).ok_or(PublicationFilterLimit {
                    resource: "transform work bytes",
                    limit: self.limits.work_bytes,
                    requested: usize::MAX,
                })?)?;
                lowercase_bytes_upper =
                    lowercase_bytes_upper
                        .checked_add(bytes)
                        .ok_or(PublicationFilterLimit {
                            resource: "temporary scratch bytes",
                            limit: self.limits.scratch_bytes,
                            requested: usize::MAX,
                        })?;
            }
        }
        // Budget two temporary String capacity bounds for an article matcher,
        // or one for borrowed query/name normalization. This maximum belongs
        // to one examined record, not to the resident batch or total RSS.
        let joined = if author.is_some() {
            input_bytes.max(8).checked_mul(2)
        } else {
            Some(0)
        }
        .ok_or(PublicationFilterLimit {
            resource: "temporary scratch bytes",
            limit: self.limits.scratch_bytes,
            requested: usize::MAX,
        })?;
        let strings = if author.is_some() { 2usize } else { 1usize };
        let scratch = input_bytes
            .max(lowercase_bytes_upper)
            .max(8)
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(joined))
            .and_then(|bytes| {
                strings
                    .checked_mul(std::mem::size_of::<String>())
                    .and_then(|headers| bytes.checked_add(headers))
            })
            .ok_or(PublicationFilterLimit {
                resource: "temporary scratch bytes",
                limit: self.limits.scratch_bytes,
                requested: usize::MAX,
            })?;
        self.peak_scratch_bytes = self.peak_scratch_bytes.max(scratch);
        if scratch > self.limits.scratch_bytes {
            return Err(PublicationFilterLimit {
                resource: "temporary scratch bytes",
                limit: self.limits.scratch_bytes,
                requested: scratch,
            }
            .into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) struct PublicationProjectionLimits {
    pub owner_bytes: usize,
    pub records: usize,
    pub metadata_bytes: usize,
    pub visible_metadata_bytes: usize,
}

impl Default for PublicationProjectionLimits {
    fn default() -> Self {
        Self {
            owner_bytes: MAX_PUBLICATION_OWNER_BYTES,
            records: MAX_PUBLICATION_ACCOUNT_RECORDS,
            metadata_bytes: MAX_PUBLICATION_ACCOUNT_METADATA_BYTES,
            visible_metadata_bytes: MAX_PUBLICATION_ACCOUNT_METADATA_BYTES,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PublicationProjectionStats {
    pub admitted_account_records: usize,
    pub owner_summaries: usize,
    pub article_visits: usize,
    pub unread_reads: usize,
    pub presence_reads: usize,
    pub account_rows: usize,
    pub article_rows: usize,
    pub row_source_bytes: usize,
    pub rejections: usize,
    pub filter_query_transforms: usize,
    pub filter_name_transforms: usize,
    pub filter_article_transforms: usize,
    pub filter_join_copies: usize,
    pub filter_join_source_bytes: usize,
    pub filter_lowercase_source_bytes: usize,
    pub filter_lowercase_result_bytes: usize,
    pub filter_preflight_chars: usize,
    pub filter_work_bytes: usize,
    pub filter_peak_scratch_bytes: usize,
    pub filter_actual_peak_scratch_bytes: usize,
    pub filter_rejections: usize,
}

struct PublicationAccountProjection<'a> {
    account: &'a MpAccount,
    latest: Option<&'a MpArticle>,
    unread: u32,
}

struct PublicationArticleProjection<'a> {
    article: &'a MpArticle,
    cached: bool,
    read: bool,
}

fn validate_publication_owner(owner: &str, maximum: usize) -> source::Result<()> {
    validate_publication_identifier(owner, maximum.min(MAX_PUBLICATION_OWNER_BYTES), "mp_owner")
}

fn validate_publication_identifier(
    value: &str,
    maximum: usize,
    field: &'static str,
) -> source::Result<()> {
    if value.len() > maximum {
        return Err(source::SourceError::ResponseTooLarge);
    }
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(source::SourceError::InvalidResponse(field));
    }
    Ok(())
}

/// A loaded legacy cache can still contain duplicate owners. Admit every raw
/// record before allocating the borrowed index, then keep its first record.
/// This matches account navigation and the source's first-seen owner choice.
fn unique_publication_accounts(
    accounts: &[MpAccount],
    limits: PublicationProjectionLimits,
) -> source::Result<Vec<&MpAccount>> {
    if accounts.len() > limits.records.min(MAX_PUBLICATION_ACCOUNT_RECORDS) {
        return Err(source::SourceError::ResponseTooLarge);
    }
    let mut bytes = 0usize;
    for account in accounts {
        validate_publication_owner(&account.id, limits.owner_bytes)?;
        let fee = account
            .id
            .len()
            .checked_add(account.name.len())
            .and_then(|bytes| bytes.checked_add(account.cover_url.len()))
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<MpAccount>()))
            .and_then(|bytes| bytes.checked_mul(2))
            .and_then(|bytes| bytes.checked_add(account.id.len()))
            .ok_or(source::SourceError::ResponseTooLarge)?;
        bytes = bytes
            .checked_add(fee)
            .filter(|bytes| {
                *bytes
                    <= limits
                        .metadata_bytes
                        .min(MAX_PUBLICATION_ACCOUNT_METADATA_BYTES)
            })
            .ok_or(source::SourceError::ResponseTooLarge)?;
    }
    let mut seen = BTreeSet::new();
    let mut unique = Vec::new();
    for account in accounts {
        if seen.insert(account.id.as_str()) {
            unique.push(account);
        }
    }
    Ok(unique)
}

fn admit_publication_row(
    bytes: &mut usize,
    fields: &[&str],
    record_bytes: usize,
    maximum: usize,
    field: &'static str,
) -> source::Result<()> {
    // Raw owner admission does not price a cached latest title. Reserve the
    // complete visible batch independently before its first field is copied.
    let id = fields[0];
    validate_publication_identifier(
        id,
        if field == "mp_owner" {
            MAX_PUBLICATION_OWNER_BYTES
        } else {
            MAX_PUBLICATION_ARTICLE_ID_BYTES
        },
        field,
    )?;
    let fee = fields
        .iter()
        .try_fold(record_bytes, |used, field| used.checked_add(field.len()))
        .and_then(|bytes| bytes.checked_mul(2))
        .and_then(|bytes| bytes.checked_add(id.len()))
        .ok_or(source::SourceError::ResponseTooLarge)?;
    *bytes = bytes
        .checked_add(fee)
        .filter(|bytes| *bytes <= maximum.min(MAX_PUBLICATION_ACCOUNT_METADATA_BYTES))
        .ok_or(source::SourceError::ResponseTooLarge)?;
    Ok(())
}

fn admit_publication_account_rows<'row, 'data: 'row>(
    rows: impl Iterator<Item = &'row PublicationAccountProjection<'data>>,
    maximum: usize,
) -> source::Result<()> {
    let mut bytes = 0usize;
    for row in rows {
        admit_publication_row(
            &mut bytes,
            &[
                &row.account.id,
                &row.account.name,
                row.latest
                    .map(|article| article.title.as_str())
                    .unwrap_or_default(),
            ],
            std::mem::size_of::<MpAccount>(),
            maximum,
            "mp_owner",
        )?;
    }
    Ok(())
}

fn admit_publication_article_rows<'row, 'data: 'row>(
    rows: impl Iterator<Item = &'row PublicationArticleProjection<'data>>,
    maximum: usize,
) -> source::Result<()> {
    let mut bytes = 0usize;
    for row in rows {
        admit_publication_row(
            &mut bytes,
            &[&row.article.id, &row.article.title, &row.article.author],
            std::mem::size_of::<MpArticle>(),
            maximum,
            "mp_article_id",
        )?;
    }
    Ok(())
}

const MAX_ARTICLE_PROJECTION_WORK: usize = 64 * 1024 * 1024;
const MAX_ARTICLE_PROJECTION_OUTPUT: usize = 64 * 1024 * 1024;
const ARTICLE_DATE_OUTPUT_BYTES: usize = 64;

#[derive(Clone, Copy, Debug)]
pub(super) struct ArticleProjectionLimits {
    pub work_bytes: usize,
    pub output_bytes: usize,
}

impl Default for ArticleProjectionLimits {
    fn default() -> Self {
        Self {
            work_bytes: MAX_ARTICLE_PROJECTION_WORK,
            output_bytes: MAX_ARTICLE_PROJECTION_OUTPUT,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error(
    "article projection {resource} exceeds its allowance (limit {limit}, requested {requested})"
)]
pub(super) struct ArticleProjectionLimit {
    pub resource: &'static str,
    pub limit: usize,
    pub requested: usize,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ArticleProjectionStats {
    pub metadata_visits: usize,
    pub owner_visits: usize,
    pub presence_reads: usize,
    pub cache_keys: usize,
    pub cache_key_bytes: usize,
    pub rows: usize,
    pub row_source_bytes: usize,
    pub selected_field_copies: usize,
    pub selected_field_bytes: usize,
    pub work_bytes: usize,
    pub output_bytes: usize,
    pub rejections: usize,
}

#[cfg(test)]
std::thread_local! {
    static ARTICLE_PROJECTION_LIMITS: Cell<Option<ArticleProjectionLimits>> = const { Cell::new(None) };
    static ARTICLE_PROJECTION_STATS: Cell<ArticleProjectionStats> = const { Cell::new(ArticleProjectionStats {
        metadata_visits: 0, owner_visits: 0, presence_reads: 0, cache_keys: 0, cache_key_bytes: 0,
        rows: 0, row_source_bytes: 0, selected_field_copies: 0, selected_field_bytes: 0,
        work_bytes: 0, output_bytes: 0, rejections: 0,
    }) };
}

#[cfg(test)]
pub(super) fn with_article_projection_limits<T>(
    limits: ArticleProjectionLimits,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<ArticleProjectionLimits>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ARTICLE_PROJECTION_LIMITS.with(|value| value.set(self.0));
        }
    }
    let _restore = Restore(ARTICLE_PROJECTION_LIMITS.with(|value| value.replace(Some(limits))));
    run()
}

#[cfg(test)]
pub(super) fn reset_article_projection_stats() {
    ARTICLE_PROJECTION_STATS.with(|value| value.set(ArticleProjectionStats::default()));
}

#[cfg(test)]
pub(super) fn article_projection_stats() -> ArticleProjectionStats {
    ARTICLE_PROJECTION_STATS.with(Cell::get)
}

#[cfg(test)]
fn observe_article_projection(observe: impl FnOnce(&mut ArticleProjectionStats)) {
    ARTICLE_PROJECTION_STATS.with(|value| {
        let mut stats = value.get();
        observe(&mut stats);
        value.set(stats);
    });
}

struct ArticleProjectionBudget {
    limits: ArticleProjectionLimits,
    work: usize,
    output: usize,
}

impl ArticleProjectionBudget {
    fn new() -> Self {
        let maximum = ArticleProjectionLimits::default();
        #[cfg(test)]
        let limits = ARTICLE_PROJECTION_LIMITS.with(Cell::get).unwrap_or(maximum);
        #[cfg(not(test))]
        let limits = maximum;
        Self {
            limits: ArticleProjectionLimits {
                work_bytes: limits.work_bytes.min(maximum.work_bytes),
                output_bytes: limits.output_bytes.min(maximum.output_bytes),
            },
            work: 0,
            output: 0,
        }
    }

    fn sum(left: usize, right: usize) -> Result<usize> {
        left.checked_add(right).ok_or_else(|| {
            ArticleProjectionLimit {
                resource: "forecast bytes",
                limit: MAX_ARTICLE_PROJECTION_WORK,
                requested: usize::MAX,
            }
            .into()
        })
    }

    fn work(&mut self, bytes: usize) -> Result<()> {
        let next = Self::sum(self.work, bytes)?;
        if next > self.limits.work_bytes {
            #[cfg(test)]
            observe_article_projection(|stats| stats.rejections += 1);
            return Err(ArticleProjectionLimit {
                resource: "work bytes",
                limit: self.limits.work_bytes,
                requested: next,
            }
            .into());
        }
        self.work = next;
        #[cfg(test)]
        observe_article_projection(|stats| stats.work_bytes += bytes);
        Ok(())
    }

    fn output(&mut self, fields: &[&str], fixed: usize, copies: usize) -> Result<()> {
        let bytes = fields
            .iter()
            .try_fold(fixed, |used, field| Self::sum(used, field.len()))?;
        let fee = bytes
            .checked_mul(copies)
            .and_then(|bytes| bytes.checked_mul(2))
            .ok_or(ArticleProjectionLimit {
                resource: "output bytes",
                limit: self.limits.output_bytes,
                requested: usize::MAX,
            })?;
        let next = Self::sum(self.output, fee)?;
        if next > self.limits.output_bytes {
            #[cfg(test)]
            observe_article_projection(|stats| stats.rejections += 1);
            return Err(ArticleProjectionLimit {
                resource: "output bytes",
                limit: self.limits.output_bytes,
                requested: next,
            }
            .into());
        }
        self.output = next;
        #[cfg(test)]
        observe_article_projection(|stats| stats.output_bytes += fee);
        self.work(
            bytes
                .checked_mul(copies)
                .and_then(|bytes| bytes.checked_mul(2))
                .ok_or(ArticleProjectionLimit {
                    resource: "field work bytes",
                    limit: self.limits.work_bytes,
                    requested: usize::MAX,
                })?,
        )
    }

    fn identifier(id: &str) -> Result<()> {
        validate_publication_identifier(id, MAX_PUBLICATION_ARTICLE_ID_BYTES, "mp_article_id")?;
        Ok(())
    }

    fn record(&mut self, id: &str, compared: &str) -> Result<()> {
        let bytes = id.len().checked_mul(4).ok_or(ArticleProjectionLimit {
            resource: "record work bytes",
            limit: self.limits.work_bytes,
            requested: usize::MAX,
        })?;
        self.work(Self::sum(32, Self::sum(bytes, compared.len())?)?)?;
        Self::identifier(id)
    }

    fn key(&mut self, prefix: &str, id: &str) -> Result<String> {
        Self::identifier(id)?;
        let bytes = Self::sum(prefix.len(), id.len())?;
        self.work(Self::sum(
            bytes.checked_mul(4).ok_or(ArticleProjectionLimit {
                resource: "cache key bytes",
                limit: self.limits.work_bytes,
                requested: usize::MAX,
            })?,
            2 * std::mem::size_of::<String>(),
        )?)?;
        #[cfg(test)]
        observe_article_projection(|stats| {
            stats.cache_keys += 1;
            stats.cache_key_bytes += bytes;
        });
        Ok(format!("{prefix}{id}"))
    }
}

fn article_projection_owned(value: &str) -> String {
    #[cfg(test)]
    observe_article_projection(|stats| {
        stats.selected_field_copies += 1;
        stats.selected_field_bytes += value.len();
    });
    value.to_owned()
}

pub(super) struct PreparedArticleLoad {
    id: String,
    account_id: String,
    body: String,
    html: String,
    layout: PreparedArticleLayout,
}

pub(super) struct IncomingArticleMetadata {
    pub title: String,
    pub author: String,
    pub cover_url: String,
    pub account_id: String,
}

pub(super) struct PreparedIncomingArticle {
    id: String,
    request: u64,
    account_epoch: u64,
    active_receiver: bool,
    content: source::ChapterContent,
    metadata: Option<IncomingArticleMetadata>,
    layout: PreparedArticleLayout,
}

struct PreparedArticleImages {
    work: super::article_images::ArticleImageWork,
    loaded: BTreeMap<String, PathBuf>,
    loaded_layout: crate::reader::layout_budget::LayoutLease,
    #[cfg(test)]
    cache_checks: usize,
}

struct PreparedArticleLayout {
    pages: crate::article::ArticlePages,
    page: usize,
    images: Option<PreparedArticleImages>,
    _old: crate::reader::layout_budget::LayoutLease,
}

impl PreparedIncomingArticle {
    pub(super) fn content(&self) -> &source::ChapterContent {
        &self.content
    }
    pub(super) fn metadata(&self) -> Option<&IncomingArticleMetadata> {
        self.metadata.as_ref()
    }
    pub(super) fn request(&self) -> u64 {
        self.request
    }
}

fn checked_article_document_in(
    html: &str,
    text: &str,
    budget: &crate::reader::LayoutBudget,
) -> Result<ArticleDocument> {
    ArticleDocument::parse_checked_in(html, text, budget).map_err(article_parse_failure)
}

fn article_parse_failure(error: crate::article::ArticleParseError) -> anyhow::Error {
    match error {
        crate::article::ArticleParseError::Style(error) => error.into(),
        crate::article::ArticleParseError::Image(error) => error.into(),
        crate::article::ArticleParseError::Layout(error) => error.into(),
    }
}

struct ArticleListProjection<'a> {
    articles: &'a [MpArticle],
    next: Option<&'a MpArticle>,
    rows: Vec<(&'a MpArticle, bool)>,
    cached: usize,
    page: usize,
    pages: usize,
}

impl App {
    pub(super) fn auxiliary_action(&mut self, action: &str, value: &str) -> Result<()> {
        match action {
            "mp.focus" => {
                self.query = PinyinInput::new(&self.mp_query);
                self.search_purpose = "publications".into();
                self.search_page = 0;
                self.navigate(Route::Search, true)?;
            }
            "mp.edit-query" | "mp.search" => {
                self.mp_query = value.trim().to_owned();
                self.mp_page = 0;
            }
            "mp.sort" if ["recent", "unread", "name"].contains(&value) => {
                self.mp_sort = value.into();
                self.mp_page = 0;
            }
            "mp.sync" => {
                if self.demo {
                    self.show_toast("demo_notice");
                } else if self.force_offline {
                    self.show_toast("network_unavailable");
                } else if let Some(account) = self.mp_account.clone() {
                    self.sync_publication_articles(&account);
                } else {
                    self.sync_shelf();
                }
            }
            "mp.account" => {
                if self.mp_accounts.iter().any(|account| account.id == value) {
                    self.mp_account = Some(value.into());
                    self.mp_page = 0;
                    self.mp_status.clear();
                    if self.online && !self.demo {
                        self.sync_publication_articles(value);
                    }
                }
            }
            "mp.back" => {
                self.mp_account = None;
                self.mp_page = 0;
                self.mp_status.clear();
            }
            "mp.article" => {
                let mut budget = ArticleProjectionBudget::new();
                if self
                    .article_metadata_with_budget(value, &mut budget)?
                    .is_some()
                {
                    self.navigate(Route::Article(value.into()), true)?;
                }
            }
            "mp.download" => self.request_article(value)?,
            "mp.page" => {
                let count = self
                    .ui
                    .upgrade()
                    .map(|ui| ui.global::<AuxiliaryModel>().get_publications_page_count())
                    .unwrap_or(1);
                self.mp_page = move_page(
                    self.mp_page,
                    value.parse().unwrap_or(0),
                    count.max(1) as usize,
                );
                self.refresh_display(RefreshReason::Content);
            }
            "mp.jump" => self.open_jump("mp", self.mp_page + 1),
            "stats.period" if ["week", "month", "year", "total"].contains(&value) => {
                self.stats_period = value.into();
                self.stats_offset = 0;
                self.stats_selected = None;
                self.request_statistics()?;
            }
            "stats.previous" if self.stats_period != "total" => {
                self.stats_offset = self.stats_offset.saturating_sub(1).max(-1200);
                self.stats_selected = None;
                self.request_statistics()?;
            }
            "stats.next" => {
                self.stats_offset = self.stats_offset.saturating_add(1).min(0);
                self.stats_selected = None;
                self.request_statistics()?;
            }
            "stats.bar" => {
                let report = self.statistics_view()?;
                if let Ok(index) = value.parse::<usize>() {
                    if report
                        .bars
                        .get(index)
                        .is_some_and(|bar| !bar.future && bar.available)
                    {
                        self.stats_selected = Some(index);
                    }
                }
            }
            "stats.bar-previous" | "stats.bar-next" => {
                let report = self.statistics_view()?;
                let available: Vec<_> = report
                    .bars
                    .iter()
                    .enumerate()
                    .filter(|(_, bar)| !bar.future && bar.available)
                    .map(|(index, _)| index)
                    .collect();
                let current = available
                    .iter()
                    .position(|index| Some(*index) == self.stats_selected)
                    .unwrap_or(0);
                self.stats_selected = available
                    .get(move_page(
                        current,
                        if action == "stats.bar-next" { 1 } else { -1 },
                        available.len(),
                    ))
                    .copied();
            }
            "stats.open" if !value.is_empty() => self.open_detail(value)?,
            "settings.open"
                if ["sync", "notes", "storage", "display", "device", "about"].contains(&value) =>
            {
                self.settings_subpage = value.into();
                self.settings_page = 0;
                self.settings_status.clear();
            }
            "settings.back" => {
                self.settings_subpage.clear();
                self.settings_page = 0;
                self.settings_status.clear();
            }
            "settings.page" => {
                let count = self
                    .ui
                    .upgrade()
                    .map(|ui| ui.global::<AuxiliaryModel>().get_settings_page_count())
                    .unwrap_or(1);
                self.settings_page = move_page(
                    self.settings_page,
                    value.parse().unwrap_or(0),
                    count.max(1) as usize,
                );
            }
            "settings.jump" => self.open_jump("settings", self.settings_page + 1),
            "settings.toggle" | "settings.increase" | "settings.decrease" => {
                let enabled = self
                    .setting_rows()?
                    .iter()
                    .any(|entry| entry.id.as_str() == value && entry.enabled);
                if !enabled {
                    return Ok(());
                }
                if value == "night_mode" && action == "settings.toggle" {
                    self.toggle_dark_mode()?;
                    return Ok(());
                }
                if matches!(
                    value,
                    "report_time" | "report_reader_only" | "push_on_close"
                ) {
                    self.checkpoint_read_reports(true)?;
                }
                let changed = if action == "settings.toggle" {
                    self.prefs.apply_toggle(value)
                } else {
                    self.prefs.adjust_for_device(
                        value,
                        if action == "settings.increase" { 1 } else { -1 },
                        self.profile.is_scribe(),
                    )
                };
                if changed {
                    self.prefs.normalize_for_device(self.profile.is_scribe());
                    self.save_preferences()?;
                    if value == "sleep_cover" {
                        self.clear_prepared_sleep_screen();
                        if self.prefs.sleep_cover {
                            if let Some(id) = self.current_sleep_book_id().map(str::to_owned) {
                                if let Some(book) = self.store.book_metadata_by_id(&id)? {
                                    self.prepare_sleep_screen_for_book(&book);
                                }
                            }
                        }
                    }
                    self.platform
                        .set_flash_on_navigation(self.prefs.flash_on_navigation);
                    self.platform
                        .set_reader_refresh_interval(self.prefs.reader.refresh_interval)?;
                    if value == "keep_awake" {
                        self.start_task_wake_guard();
                    }
                    self.settings_status = tr("local_saved");
                }
            }
            "settings.choice" => {
                if let Some((id, choice)) = value.split_once(':') {
                    self.choose_setting(id, choice)?;
                }
            }
            "settings.action" => self.setting_action(value)?,
            "settings.renew" => {
                if self.force_offline {
                    self.show_toast("network_unavailable");
                } else if self.demo
                    || !self
                        .jobs
                        .session()
                        .is_some_and(|session| session.has_web_session())
                {
                    self.show_toast("web_login_required");
                } else {
                    let request = self.request();
                    self.settings_status = tr("synchronizing");
                    self.settle_job_submission(self.jobs.renew(request));
                }
            }
            "settings.exit" => self.confirm_action("exit", "exit_app", "exit_app_hint", "exit_app"),
            "settings.logout" => {
                self.confirm_action("logout", "logout_confirm", "logout_hint", "logout")
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn render_publications(&mut self) -> Result<()> {
        self.render_publications_with_limits(PublicationProjectionLimits::default())
    }

    #[cfg(test)]
    pub(super) fn render_publications_with_test_limits(
        &mut self,
        limits: PublicationProjectionLimits,
    ) -> Result<()> {
        self.render_publications_with_limits(limits)
    }

    #[cfg(test)]
    pub(super) fn render_publications_filter_inner_for_test(&mut self) -> Result<()> {
        self.render_publications_with_limits_inner(PublicationProjectionLimits::default())
    }

    fn render_publications_with_limits(
        &mut self,
        limits: PublicationProjectionLimits,
    ) -> Result<()> {
        let rendered = self.render_publications_with_limits_inner(limits);
        if rendered.as_ref().err().is_some_and(|error| {
            error.downcast_ref::<PublicationFilterLimit>().is_some()
                || matches!(
                    error.downcast_ref::<source::SourceError>(),
                    Some(
                        source::SourceError::ResponseTooLarge
                            | source::SourceError::InvalidResponse(_)
                    )
                )
        }) {
            #[cfg(test)]
            self.observe_publication_projection(|stats| stats.rejections += 1);
            self.show_toast("api_error");
            return Ok(());
        }
        rendered
    }

    fn render_publications_with_limits_inner(
        &mut self,
        limits: PublicationProjectionLimits,
    ) -> Result<()> {
        if self.route != Route::Publications {
            return Ok(());
        }
        let Some(ui) = self.ui.upgrade() else {
            return Ok(());
        };
        let accounts = unique_publication_accounts(&self.mp_accounts, limits)?;
        if let Some(owner) = self.mp_account.as_deref() {
            validate_publication_owner(owner, limits.owner_bytes)?;
        }
        #[cfg(test)]
        self.observe_publication_projection(|stats| {
            stats.admitted_account_records += self.mp_accounts.len();
        });
        let global = ui.global::<AuxiliaryModel>();
        let mut filter = PublicationFilterBudget::new();
        let query = self.publication_filter_lowercase(&mut filter, self.mp_query.trim(), true)?;
        let account_id = self.mp_account.clone();
        let page_size = self.publication_page_size(account_id.is_some());
        let (rows, page, pages, title, subtitle) = if let Some(account_id) = &account_id {
            let account = accounts.iter().find(|account| &account.id == account_id);
            let mut articles = self
                .mp_articles
                .get(account_id)
                .map(|articles| articles.iter().collect::<Vec<_>>())
                .unwrap_or_default();
            articles.sort_by(|left, right| {
                right
                    .published_at
                    .cmp(&left.published_at)
                    .then_with(|| left.id.cmp(&right.id))
            });
            let mut projected = Vec::new();
            for article in articles {
                #[cfg(test)]
                self.observe_publication_projection(|stats| stats.article_visits += 1);
                if !query.is_empty()
                    && !self.publication_filter_article(&mut filter, article, &query)?
                {
                    continue;
                }
                projected.push(PublicationArticleProjection {
                    article,
                    cached: self.publication_body_present(&article.id)?,
                    read: self.publication_article_read(&article.id)?,
                });
            }
            let pages = page_count(projected.len(), page_size);
            let page = self.mp_page.min(pages - 1);
            let subtitle = i18n::format(
                "articles_summary",
                &[
                    ("count", projected.len().to_string()),
                    (
                        "cached",
                        projected
                            .iter()
                            .filter(|entry| entry.cached)
                            .count()
                            .to_string(),
                    ),
                ],
            );
            // Sort and page borrowed descriptors before materializing any
            // title/author text, including when the article list is hidden.
            admit_publication_article_rows(
                projected.iter().skip(page * page_size).take(page_size),
                limits.visible_metadata_bytes,
            )?;
            let rows = projected
                .iter()
                .skip(page * page_size)
                .take(page_size)
                .map(|entry| self.publication_article_row(entry))
                .collect::<Vec<_>>();
            (
                rows,
                page,
                pages,
                account
                    .map(|account| slint::SharedString::from(account.name.as_str()))
                    .unwrap_or_else(|| tr("publications").into()),
                subtitle,
            )
        } else {
            let mut accounts = accounts;
            match self.mp_sort.as_str() {
                "name" => accounts.sort_by(|left, right| left.name.cmp(&right.name)),
                _ => accounts.sort_by(|left, right| {
                    right
                        .updated_at
                        .cmp(&left.updated_at)
                        .then_with(|| left.id.cmp(&right.id))
                }),
            }
            let mut projected = Vec::new();
            for account in accounts {
                #[cfg(test)]
                self.observe_publication_projection(|stats| stats.owner_summaries += 1);
                let articles = self
                    .mp_articles
                    .get(&account.id)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let mut matched = query.is_empty()
                    || self
                        .publication_filter_lowercase(&mut filter, &account.name, false)?
                        .contains(&query);
                let mut latest = None;
                let mut unread = 0_u32;
                let mut unread_error = None;
                for article in articles {
                    #[cfg(test)]
                    self.observe_publication_projection(|stats| stats.article_visits += 1);
                    if !matched && self.publication_filter_article(&mut filter, article, &query)? {
                        matched = true;
                    }
                    if latest.is_none_or(|previous: &MpArticle| {
                        article.published_at >= previous.published_at
                    }) {
                        // max_by_key previously chose the last equal timestamp.
                        latest = Some(article);
                    }
                    if unread_error.is_none() {
                        match self.publication_article_read(&article.id) {
                            Ok(false) => unread = unread.saturating_add(1),
                            Ok(true) => {}
                            Err(error) => unread_error = Some(error),
                        }
                    }
                }
                if !matched {
                    continue;
                }
                // A filtered-out owner did not previously read its unread
                // settings. Do not expose its stored-value failure in this view.
                if let Some(error) = unread_error {
                    return Err(error);
                }
                if self.mp_sort == "unread" && unread == 0 {
                    continue;
                }
                projected.push(PublicationAccountProjection {
                    account,
                    latest,
                    unread,
                });
            }
            let pages = page_count(projected.len(), page_size);
            let page = self.mp_page.min(pages - 1);
            let subtitle = format!(
                "{} · {}",
                i18n::format(
                    "mp_summary",
                    &[
                        ("count", projected.len().to_string()),
                        (
                            "unread",
                            projected
                                .iter()
                                .map(|entry| u64::from(entry.unread.min(i32::MAX as u32)))
                                .sum::<u64>()
                                .to_string(),
                        ),
                    ],
                ),
                tr("local_unread"),
            );
            // The index and summaries borrow current cache data only for this
            // render. Hidden owners never copy their latest titles into rows.
            admit_publication_account_rows(
                projected.iter().skip(page * page_size).take(page_size),
                limits.visible_metadata_bytes,
            )?;
            let rows = projected
                .iter()
                .skip(page * page_size)
                .take(page_size)
                .map(|entry| self.publication_account_row(entry))
                .collect::<Vec<_>>();
            (rows, page, pages, tr("publications").into(), subtitle)
        };
        self.mp_page = page;
        global.set_publications_title(title);
        global.set_publications_subtitle(subtitle.into());
        global.set_publications_rows(model(rows));
        global.set_publications_query(self.mp_query.clone().into());
        global.set_publications_sort(self.mp_sort.clone().into());
        global.set_publications_account_id(account_id.unwrap_or_default().into());
        global.set_publications_page((page + 1) as i32);
        global.set_publications_page_count(pages as i32);
        global.set_publications_busy(self.mp_status == tr("synchronizing"));
        global.set_publications_status(
            if self.online {
                self.mp_status.clone()
            } else {
                tr("offline_banner")
            }
            .into(),
        );
        Ok(())
    }

    fn publication_filter_admit(
        &self,
        budget: &mut PublicationFilterBudget,
        value: &str,
        author: Option<&str>,
    ) -> Result<()> {
        #[cfg(test)]
        let before = (budget.work_bytes, budget.preflight_chars);
        let result = budget.admit(value, author);
        #[cfg(test)]
        self.observe_publication_projection(|stats| {
            stats.filter_work_bytes = stats
                .filter_work_bytes
                .saturating_add(budget.work_bytes - before.0);
            stats.filter_preflight_chars = stats
                .filter_preflight_chars
                .saturating_add(budget.preflight_chars - before.1);
            stats.filter_peak_scratch_bytes = stats
                .filter_peak_scratch_bytes
                .max(budget.peak_scratch_bytes);
            stats.filter_rejections += usize::from(result.is_err());
        });
        result
    }

    fn publication_filter_lowercase(
        &self,
        budget: &mut PublicationFilterBudget,
        value: &str,
        query: bool,
    ) -> Result<String> {
        if value.is_empty() {
            return Ok(String::new());
        }
        self.publication_filter_admit(budget, value, None)?;
        let lowercase = value.to_lowercase();
        #[cfg(test)]
        self.observe_publication_projection(|stats| {
            if query {
                stats.filter_query_transforms += 1;
            } else {
                stats.filter_name_transforms += 1;
            }
            stats.filter_lowercase_source_bytes = stats
                .filter_lowercase_source_bytes
                .saturating_add(value.len());
            stats.filter_lowercase_result_bytes = stats
                .filter_lowercase_result_bytes
                .saturating_add(lowercase.len());
            stats.filter_actual_peak_scratch_bytes = stats.filter_actual_peak_scratch_bytes.max(
                lowercase
                    .capacity()
                    .saturating_add(std::mem::size_of::<String>()),
            );
        });
        #[cfg(not(test))]
        let _ = query;
        Ok(lowercase)
    }

    fn publication_filter_article(
        &self,
        budget: &mut PublicationFilterBudget,
        article: &MpArticle,
        query: &str,
    ) -> Result<bool> {
        self.publication_filter_admit(budget, &article.title, Some(&article.author))?;
        let joined = format!("{} {}", article.title, article.author);
        let lowercase = joined.to_lowercase();
        #[cfg(test)]
        self.observe_publication_projection(|stats| {
            stats.filter_article_transforms += 1;
            stats.filter_join_copies += 1;
            stats.filter_join_source_bytes =
                stats.filter_join_source_bytes.saturating_add(joined.len());
            stats.filter_lowercase_source_bytes = stats
                .filter_lowercase_source_bytes
                .saturating_add(joined.len());
            stats.filter_lowercase_result_bytes = stats
                .filter_lowercase_result_bytes
                .saturating_add(lowercase.len());
            stats.filter_actual_peak_scratch_bytes = stats.filter_actual_peak_scratch_bytes.max(
                joined
                    .capacity()
                    .saturating_add(lowercase.capacity())
                    .saturating_add(2 * std::mem::size_of::<String>()),
            );
        });
        Ok(lowercase.contains(query))
    }

    fn publication_page_size(&self, articles: bool) -> usize {
        let design_page_size = match (articles, self.profile.is_scribe()) {
            (true, true) => 8,
            (false, true) => 9,
            (_, false) => 7,
        };
        let (_, height) = self.platform.logical_viewport_size();
        let navigation = if self.profile.is_scribe() { 76 } else { 64 };
        let status_height = if !self.online || !self.mp_status.is_empty() {
            32
        } else {
            0
        };
        let content_height = height.saturating_sub(
            self.app_status_bar_height() + navigation + 52 + 52 + 48 + 48 + status_height,
        );
        design_page_size.min((content_height / 72).max(1) as usize)
    }

    fn publication_article_read(&self, id: &str) -> Result<bool> {
        validate_publication_identifier(id, MAX_PUBLICATION_ARTICLE_ID_BYTES, "mp_article_id")?;
        #[cfg(test)]
        self.observe_publication_projection(|stats| stats.unread_reads += 1);
        Ok(self
            .store
            .setting::<bool>(&format!("article.read:{id}"))?
            .unwrap_or(false))
    }

    fn publication_body_present(&self, id: &str) -> Result<bool> {
        validate_publication_identifier(id, MAX_PUBLICATION_ARTICLE_ID_BYTES, "mp_article_id")?;
        #[cfg(test)]
        self.observe_publication_projection(|stats| stats.presence_reads += 1);
        self.store.chapter_body_present(&format!("article:{id}"), 0)
    }

    fn publication_row_text(&self, value: &str) -> slint::SharedString {
        #[cfg(test)]
        self.observe_publication_projection(|stats| stats.row_source_bytes += value.len());
        value.into()
    }

    fn publication_account_row(&self, entry: &PublicationAccountProjection<'_>) -> PublicationRow {
        #[cfg(test)]
        self.observe_publication_projection(|stats| stats.account_rows += 1);
        PublicationRow {
            id: self.publication_row_text(&entry.account.id),
            title: self.publication_row_text(&entry.account.name),
            subtitle: self.publication_row_text(
                entry
                    .latest
                    .map(|article| article.title.as_str())
                    .unwrap_or_default(),
            ),
            meta: date_label(entry.account.updated_at).into(),
            initial: entry
                .account
                .name
                .chars()
                .next()
                .map(String::from)
                .unwrap_or_default()
                .into(),
            unread: entry.unread.min(i32::MAX as u32) as i32,
            cached: false,
        }
    }

    fn publication_article_row(&self, entry: &PublicationArticleProjection<'_>) -> PublicationRow {
        #[cfg(test)]
        self.observe_publication_projection(|stats| stats.article_rows += 1);
        PublicationRow {
            id: self.publication_row_text(&entry.article.id),
            title: self.publication_row_text(&entry.article.title),
            subtitle: self.publication_row_text(&entry.article.author),
            meta: format!(
                "{} · {}",
                date_label(entry.article.published_at),
                tr(if entry.cached {
                    "downloaded"
                } else {
                    "not_downloaded"
                })
            )
            .into(),
            initial: "".into(),
            unread: (!entry.read) as i32,
            cached: entry.cached,
        }
    }

    #[cfg(test)]
    fn observe_publication_projection(
        &self,
        observe: impl FnOnce(&mut PublicationProjectionStats),
    ) {
        let mut stats = self.publication_stats.get();
        observe(&mut stats);
        self.publication_stats.set(stats);
    }

    fn sync_publication_articles(&mut self, account_id: &str) {
        if !self.jobs.capabilities().mp_articles {
            self.show_toast("web_login_required");
            return;
        }
        if self.mp_status == tr("synchronizing") {
            return;
        }
        let request = self.request();
        self.mp_status = tr("synchronizing");
        self.settle_job_submission(self.jobs.mp_articles(request, account_id.into(), 0, 100));
    }

    fn local_statistics(&self) -> Result<statistics::Report> {
        let books = self.store.book_statistics_metadata()?;
        let sessions = self.store.sessions(i64::MIN, i64::MAX)?;
        let mut annotations = statistics::AnnotationStatistics::new(
            &self.stats_period,
            self.stats_offset,
            statistics::projection_now(),
        );
        self.store
            .for_each_annotation_timestamp_count(|timestamp, _| {
                annotations.observe_timestamp(timestamp);
                Ok(())
            })?;
        let (start, end) = annotations.range_checked(&sessions)?;
        // Numeric range counting follows the old report exactly, including
        // timestamps near Chrono MIN that precede the local midnight boundary.
        self.store
            .for_each_annotation_timestamp_count(|timestamp, count| {
                if timestamp >= start && timestamp < end {
                    annotations.add_count(count);
                }
                Ok(())
            })?;
        annotations
            .report_metadata_checked(&sessions, &books)
            .map_err(anyhow::Error::from)
    }

    #[cfg(test)]
    pub(super) fn local_statistics_for_test(&self) -> Result<statistics::Report> {
        self.local_statistics()
    }

    pub(super) fn request_statistics(&mut self) -> Result<()> {
        // Invalidate an earlier period even when no new network job can be sent.
        let request = self.request();
        self.stats_request = request;
        self.remote_stats = None;
        let now = Utc::now();
        let base_time = statistics::period_base_time(&self.stats_period, self.stats_offset, now);
        // A persisted cloud cache must be scoped to an acknowledged account identity.
        // Unscoped legacy keys are not restored into a different login session.
        if !self.force_offline && !self.demo && self.jobs.capabilities().statistics {
            let period = match self.stats_period.as_str() {
                "month" => source::StatisticsPeriod::Month,
                "year" => source::StatisticsPeriod::Year,
                "total" => source::StatisticsPeriod::Total,
                _ => source::StatisticsPeriod::Week,
            };
            self.settle_job_submission(self.jobs.statistics(request, period, base_time));
        }
        self.refresh_display(RefreshReason::Content);
        Ok(())
    }

    fn statistics_view(&self) -> Result<StatisticsView> {
        if let Some(remote) = &self.remote_stats {
            let report = statistics::from_remote_selected_checked(
                remote,
                &self.stats_period,
                self.stats_offset,
                statistics::projection_now(),
                statistics::RemoteProjectionSelection {
                    summary: 2,
                    ranking: if self.profile.is_scribe() { 5 } else { 4 },
                },
            )?;
            let mut metrics = vec![
                StatisticMetric {
                    label: tr("reading_days").into(),
                    value: report
                        .days
                        .map(|days| i18n::format("days", &[("count", days.to_string())]))
                        .unwrap_or_else(|| tr("stats_unavailable"))
                        .into(),
                },
                StatisticMetric {
                    label: tr("daily_average").into(),
                    value: optional_duration_label(report.average_seconds).into(),
                },
            ];
            for value in report.summary.into_iter().take(2) {
                metrics.push(StatisticMetric {
                    label: value.label.into(),
                    value: value.value.into(),
                });
            }
            while metrics.len() < 4 {
                metrics.push(StatisticMetric {
                    label: tr(if metrics.len() == 2 {
                        "books_finished"
                    } else {
                        "annotations_count"
                    })
                    .into(),
                    value: tr("stats_unavailable").into(),
                });
            }
            let mut ranking = Vec::new();
            for rank in report.ranking {
                // A known upstream ID makes the ranking navigable without invented metadata.
                if !rank.book_id.is_empty() && !self.store.book_exists(&rank.book_id)? {
                    self.store.upsert_book(&Book {
                        id: rank.book_id.clone(),
                        title: rank.title.clone(),
                        author: rank.author,
                        source: "weread".into(),
                        on_shelf: false,
                        ..Default::default()
                    })?;
                }
                ranking.push(statistics::Rank {
                    book_id: rank.book_id,
                    title: rank.title,
                    seconds: rank.seconds,
                });
            }
            let mut status = if report.bars_unavailable {
                format!("{} · {}", tr("stats_cloud"), tr("stats_unavailable"))
            } else {
                tr(if report.missing_details {
                    "stats_cloud_partial"
                } else {
                    "stats_cloud"
                })
            };
            if !self.online {
                status = format!("{} · {}", status, tr("offline"));
            }
            return Ok(StatisticsView {
                period_label: if report.period_label.is_empty() {
                    tr("total")
                } else {
                    report.period_label
                },
                seconds: report.seconds,
                metrics,
                bars: report.bars,
                ranking,
                ranking_max_seconds: Some(report.ranking_max_seconds),
                calendar: report.calendar,
                calendar_offset: report.calendar_offset,
                calendar_label: report.calendar_label,
                preferences: report.preferences,
                status,
                offset: report.offset,
                range_start: report.range_start,
            });
        }
        let report = self.local_statistics()?;
        Ok(StatisticsView {
            period_label: report.period_label,
            seconds: Some(report.seconds),
            metrics: vec![
                StatisticMetric {
                    label: tr("reading_days").into(),
                    value: i18n::format("days", &[("count", report.days.to_string())]).into(),
                },
                StatisticMetric {
                    label: tr(if matches!(self.stats_period.as_str(), "year" | "total") {
                        "read_day_average"
                    } else {
                        "daily_average"
                    })
                    .into(),
                    value: duration_label(report.average_seconds).into(),
                },
                StatisticMetric {
                    label: tr("stats_finished_current").into(),
                    value: report.finished.to_string().into(),
                },
                StatisticMetric {
                    label: tr("annotations_count").into(),
                    value: report.annotations.to_string().into(),
                },
            ],
            bars: report.bars,
            ranking: report.ranking,
            ranking_max_seconds: None,
            calendar: report.calendar,
            calendar_offset: report.calendar_offset,
            calendar_label: report.calendar_label,
            preferences: report.preferences,
            status: if report.bars_unavailable {
                format!("{} · {}", tr("stats_local"), tr("stats_unavailable"))
            } else {
                tr("stats_local")
            },
            offset: report.offset,
            range_start: report.range_start,
        })
    }

    pub(super) fn render_statistics(&mut self) -> Result<()> {
        let report = match self.statistics_view() {
            Ok(report) => report,
            Err(error)
                if error
                    .downcast_ref::<statistics::LocalStatisticsUnavailable>()
                    .is_some()
                    || error
                        .downcast_ref::<statistics::RemoteProjectionUnavailable>()
                        .is_some()
                    || error
                        .downcast_ref::<crate::store::BookMetadataLimit>()
                        .is_some()
                    || error
                        .downcast_ref::<crate::store::BookMetadataUnavailable>()
                        .is_some() =>
            {
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<AuxiliaryModel>()
                        .set_stats_status(tr("api_error").into());
                }
                self.show_toast("api_error");
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        self.stats_offset = report.offset;
        let Some(ui) = self.ui.upgrade() else {
            return Ok(());
        };
        let global = ui.global::<AuxiliaryModel>();
        let selected = self
            .stats_selected
            .filter(|index| {
                report
                    .bars
                    .get(*index)
                    .is_some_and(|bar| !bar.future && bar.available)
            })
            .or_else(|| {
                report
                    .bars
                    .iter()
                    .enumerate()
                    .filter(|(_, bar)| !bar.future && bar.available)
                    .max_by_key(|(_, bar)| bar.seconds)
                    .map(|(index, _)| index)
            });
        self.stats_selected = selected;
        let maximum = report
            .bars
            .iter()
            .map(|bar| bar.seconds)
            .max()
            .unwrap_or(0)
            .max(1);
        global.set_stats_period(self.stats_period.clone().into());
        global.set_stats_period_label(report.period_label.into());
        global.set_stats_summary(optional_duration_label(report.seconds).into());
        global.set_stats_status(report.status.into());
        global.set_stats_can_next(self.stats_offset < 0 && self.stats_period != "total");
        global.set_stats_chart_title(
            tr(match self.stats_period.as_str() {
                "year" => "monthly_duration",
                "total" => "yearly_duration",
                _ => "daily_duration",
            })
            .into(),
        );
        global.set_stats_selected_label(
            selected
                .and_then(|index| report.bars.get(index).map(|bar| (index, bar)))
                .map(|(index, bar)| {
                    format!(
                        "{} · {}",
                        statistic_selection_label(
                            &self.stats_period,
                            report.range_start,
                            index,
                            &bar.label,
                        ),
                        duration_label(bar.seconds),
                    )
                })
                .unwrap_or_else(|| tr("stats_unavailable"))
                .into(),
        );
        global.set_stats_metrics(model(report.metrics));
        #[cfg(test)]
        statistics::record_statistics_ui_rows(report.bars.len());
        global.set_stats_bars(model(
            report
                .bars
                .iter()
                .enumerate()
                .map(|(index, bar)| StatisticBar {
                    label: if self.stats_period == "month"
                        && ![1, 8, 15, 22, 29].contains(&(index + 1))
                    {
                        String::new()
                    } else {
                        bar.label.clone()
                    }
                    .into(),
                    value: bar.seconds as f32 / maximum as f32,
                    selected: selected == Some(index),
                    future: bar.future || !bar.available,
                })
                .collect(),
        ));
        let top = report
            .ranking_max_seconds
            .unwrap_or_else(|| {
                report
                    .ranking
                    .iter()
                    .map(|rank| rank.seconds)
                    .max()
                    .unwrap_or(0)
            })
            .max(1);
        global.set_stats_ranking(model(
            report
                .ranking
                .into_iter()
                .take(if self.profile.is_scribe() { 5 } else { 4 })
                .map(|rank| RankingEntry {
                    id: rank.book_id.into(),
                    title: rank.title.into(),
                    subtitle: duration_label(rank.seconds).into(),
                    value: rank.seconds as f32 / top as f32,
                })
                .collect(),
        ));
        let calendar_days = report.calendar.iter().filter(|level| **level > 0).count();
        global.set_stats_calendar_days(
            i18n::format(
                if report.calendar.iter().any(|level| *level < 0) {
                    "stats_calendar_days_known"
                } else {
                    "stats_calendar_days"
                },
                &[("count", calendar_days.to_string())],
            )
            .into(),
        );
        let mut calendar = if report.calendar.is_empty() {
            Vec::new()
        } else {
            vec![-1; report.calendar_offset]
        };
        calendar.extend(report.calendar);
        while calendar.len() % 7 != 0 {
            calendar.push(-1);
        }
        global.set_stats_calendar(model(calendar));
        global.set_stats_calendar_offset(report.calendar_offset as i32);
        global.set_stats_calendar_label(statistic_calendar_label(&report.calendar_label).into());
        global.set_stats_weekdays(model(
            [
                "weekday_mon",
                "weekday_tue",
                "weekday_wed",
                "weekday_thu",
                "weekday_fri",
                "weekday_sat",
                "weekday_sun",
            ]
            .iter()
            .map(|key| tr(key).into())
            .collect(),
        ));
        let mut preferences = report.preferences;
        if preferences.len() > 5 {
            let remaining = preferences.split_off(4);
            preferences.push(statistics::Preference {
                category: "other_categories".into(),
                seconds: remaining
                    .iter()
                    .fold(0_u64, |sum, entry| sum.saturating_add(entry.seconds)),
                percent: remaining
                    .iter()
                    .fold(0_u32, |sum, entry| sum.saturating_add(entry.percent)),
            });
        }
        let mut offset = 0.0;
        global.set_stats_preferences(model(
            preferences
                .into_iter()
                .enumerate()
                .map(|(index, preference)| {
                    let value = preference.percent.min(100) as f32 / 100.0;
                    let entry = PreferenceEntry {
                        label: match preference.category.as_str() {
                            "uncategorized" => tr("uncategorized"),
                            "other_categories" => tr("other_categories"),
                            _ => preference.category,
                        }
                        .into(),
                        value,
                        offset,
                        percent: preference.percent.min(100) as i32,
                        tone: index as i32,
                    };
                    offset += value;
                    entry
                })
                .collect(),
        ));
        Ok(())
    }

    pub(super) fn render_settings(&mut self) -> Result<()> {
        let rows = self.setting_rows()?;
        let (_, height) = self.platform.logical_viewport_size();
        let navigation = if self.profile.is_scribe() { 76 } else { 64 };
        let main = self.settings_subpage.is_empty();
        let statusbar = self.app_status_bar_height();
        let capacity = height
            .saturating_sub(
                statusbar + navigation + 56 + 56 + 48 + if main { 104 + 128 } else { 0 },
            )
            .max(72);
        let mut pages: Vec<Vec<SettingEntry>> = vec![vec![]];
        let mut used = 0;
        for row in rows {
            let row_height = setting_row_height(&row, self.profile.is_scribe());
            if used + row_height > capacity && !pages.last().is_some_and(Vec::is_empty) {
                pages.push(vec![]);
                used = 0;
            }
            used += row_height;
            pages
                .last_mut()
                .expect("at least one settings page")
                .push(row);
        }
        self.settings_page = self.settings_page.min(pages.len().saturating_sub(1));
        let Some(ui) = self.ui.upgrade() else {
            return Ok(());
        };
        let global = ui.global::<AuxiliaryModel>();
        let session = self
            .jobs
            .session()
            .filter(|session| session.is_configured());
        let account = session
            .as_ref()
            .map(|session| session.account_name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| {
                tr(if session.is_some() {
                    "account"
                } else {
                    "local_account"
                })
            });
        global.set_settings_account_name(account.clone().into());
        global.set_settings_account_initial(
            account
                .chars()
                .next()
                .map(String::from)
                .unwrap_or_default()
                .into(),
        );
        global.set_settings_account_detail(
            tr(if self.demo {
                "demo_notice"
            } else if session.is_some() {
                "login_local"
            } else {
                "empty_library_hint"
            })
            .into(),
        );
        global.set_settings_account_active(session.is_some() && !self.demo);
        global.set_settings_subpage(self.settings_subpage.clone().into());
        global.set_settings_title(tr(settings_title(&self.settings_subpage)).into());
        global.set_settings_rows(model(pages[self.settings_page].clone()));
        global.set_settings_page((self.settings_page + 1) as i32);
        global.set_settings_page_count(pages.len() as i32);
        global.set_settings_status(
            if self.settings_subpage == "sync" {
                self.read_report_summary()
            } else {
                self.settings_status.clone()
            }
            .into(),
        );
        Ok(())
    }

    fn setting_rows(&self) -> Result<Vec<SettingEntry>> {
        let preferences = &self.prefs;
        let remote = self.jobs.capabilities();
        let hardware = self.platform.capabilities();
        let web = self
            .jobs
            .session()
            .is_some_and(|session| session.has_web_session())
            && !self.demo;
        let source = remote.remote_progress && !self.demo;
        let storage = self.store.summary()?;
        let cache_bytes = storage
            .body_bytes
            .saturating_add(self.cache.usage()?)
            .saturating_add(self.book_assets.usage()?);
        let rows = match self.settings_subpage.as_str() {
            "" => vec![
                setting(
                    "sync",
                    "reading_sync",
                    "link",
                    tr(if preferences.report_time {
                        "enabled"
                    } else {
                        "disabled"
                    }),
                    false,
                    true,
                    "",
                ),
                setting(
                    "notes",
                    "highlights_thoughts",
                    "link",
                    popup_label(&preferences.popup_position),
                    false,
                    true,
                    "",
                ),
                setting(
                    "storage",
                    "downloads_storage",
                    "link",
                    size_label(cache_bytes),
                    false,
                    true,
                    "",
                ),
                setting(
                    "display",
                    "display_refresh",
                    "link",
                    tr(if preferences.view_mode == "list" {
                        "list_view"
                    } else {
                        "cover_view"
                    }),
                    false,
                    true,
                    "",
                ),
                setting(
                    "device",
                    "device_network",
                    "link",
                    self.profile.name().into(),
                    false,
                    true,
                    "",
                ),
                setting(
                    "about",
                    "about",
                    "link",
                    env!("CARGO_PKG_VERSION").into(),
                    false,
                    true,
                    "",
                ),
            ],
            "sync" => vec![
                setting(
                    "pull_on_open",
                    "pull_on_open",
                    "toggle",
                    String::new(),
                    preferences.pull_on_open,
                    source,
                    "",
                ),
                setting(
                    "push_on_close",
                    "push_on_close",
                    "toggle",
                    String::new(),
                    preferences.push_on_close,
                    web,
                    "",
                ),
                setting(
                    "ask_on_conflict",
                    "ask_on_conflict",
                    "toggle",
                    String::new(),
                    preferences.ask_on_conflict,
                    source,
                    "",
                ),
                setting(
                    "report_time",
                    "report_time",
                    "toggle",
                    String::new(),
                    preferences.report_time,
                    web,
                    "",
                ),
                setting(
                    "report_reader_only",
                    "report_reader_only",
                    "toggle",
                    String::new(),
                    preferences.report_reader_only,
                    preferences.report_time && web,
                    "",
                ),
                setting(
                    "report_target",
                    "report_target",
                    "info",
                    tr("automatic"),
                    false,
                    false,
                    "",
                ),
            ],
            "notes" => {
                let mut rows = vec![
                    setting(
                        "popular",
                        "show_popular",
                        "toggle",
                        String::new(),
                        preferences.popular,
                        true,
                        "",
                    ),
                    setting(
                        "mine",
                        "show_mine",
                        "toggle",
                        String::new(),
                        preferences.mine,
                        true,
                        "",
                    ),
                    setting_choice(
                        "popup_position",
                        "notes_position",
                        &preferences.popup_position,
                        if self.profile.is_scribe() {
                            &[
                                ("side", "side_panel"),
                                ("center", "center_popup"),
                                ("bottom", "bottom_popup"),
                            ]
                        } else {
                            &[("center", "center_popup"), ("bottom", "bottom_popup")]
                        },
                        true,
                        "",
                    ),
                    setting(
                        "edge_guard",
                        "edge_guard",
                        "toggle",
                        String::new(),
                        preferences.edge_guard,
                        true,
                        "",
                    ),
                    setting(
                        "edge_region",
                        "edge_region",
                        "stepper",
                        format!("{}%", preferences.edge_region),
                        false,
                        preferences.edge_guard,
                        "",
                    ),
                    setting(
                        "popup_height",
                        "popup_height",
                        "stepper",
                        format!("{}%", preferences.popup_height),
                        false,
                        preferences.popup_position != "side",
                        "",
                    ),
                    setting(
                        "popup_turn",
                        "popup_turn",
                        "toggle",
                        String::new(),
                        false,
                        false,
                        "update_unavailable",
                    ),
                ];
                if self.profile.is_scribe() {
                    rows.insert(
                        3,
                        setting(
                            "pen_highlight",
                            "pen_highlight",
                            "toggle",
                            String::new(),
                            false,
                            false,
                            "pending_verification",
                        ),
                    );
                }
                rows
            }
            "storage" => vec![
                setting(
                    "download_concurrency",
                    "download_concurrency",
                    "stepper",
                    preferences.download_concurrency.to_string(),
                    false,
                    true,
                    "",
                ),
                setting(
                    "download_images",
                    "download_images",
                    "toggle",
                    String::new(),
                    preferences.download_images,
                    true,
                    "",
                ),
                setting(
                    "download_article_images",
                    "download_article_images",
                    "toggle",
                    String::new(),
                    preferences.download_article_images,
                    true,
                    "",
                ),
                setting(
                    "prefetch",
                    "prefetch",
                    "toggle",
                    String::new(),
                    preferences.prefetch,
                    remote.chapter_content && !self.demo,
                    "",
                ),
                setting(
                    "prefetch_notes",
                    "prefetch_notes",
                    "toggle",
                    String::new(),
                    false,
                    false,
                    "update_unavailable",
                ),
                setting(
                    "cache_directory",
                    "cache_directory",
                    "info",
                    self.data_dir.display().to_string(),
                    false,
                    false,
                    "",
                ),
                setting(
                    "clear_cache",
                    "clear_cache",
                    "action",
                    size_label(cache_bytes),
                    false,
                    true,
                    "",
                ),
                setting(
                    "scan_local",
                    "scan_local",
                    "action",
                    String::new(),
                    false,
                    true,
                    "",
                ),
            ],
            "display" => {
                let mut rows = vec![
                    setting_choice(
                        "view_mode",
                        "shelf_view",
                        &preferences.view_mode,
                        if self.profile.is_scribe() {
                            &[("cover", "cover_view"), ("list", "list_view")]
                        } else {
                            &[("list", "list_view"), ("cover", "cover_view")]
                        },
                        true,
                        "",
                    ),
                    setting_choice(
                        "image_dithering",
                        "image_dithering",
                        preferences.image_dithering.as_str(),
                        &[
                            ("off", "image_dithering_off"),
                            ("software", "image_dithering_software"),
                            ("hardware", "image_dithering_hardware"),
                        ],
                        true,
                        if preferences.image_dithering
                            == crate::preferences::ImageDithering::Hardware
                            && !self.platform.hardware_y4_available()
                        {
                            "image_dithering_fallback"
                        } else {
                            ""
                        },
                    ),
                    setting_choice(
                        "refresh_interval",
                        "full_refresh",
                        &preferences.reader.refresh_interval.to_string(),
                        &[
                            ("1", "every_page"),
                            ("6", "six_pages"),
                            ("12", "twelve_pages"),
                            ("0", "never"),
                        ],
                        hardware.explicit_refresh,
                        if hardware.explicit_refresh {
                            ""
                        } else {
                            "pending_verification"
                        },
                    ),
                    setting(
                        "flash_on_navigation",
                        "flash_on_navigation",
                        "toggle",
                        String::new(),
                        preferences.flash_on_navigation && hardware.explicit_refresh,
                        hardware.explicit_refresh,
                        if hardware.explicit_refresh {
                            ""
                        } else {
                            "pending_verification"
                        },
                    ),
                    setting_choice(
                        "tap_mode",
                        "tap_zones",
                        &preferences.reader.tap_mode,
                        &[
                            ("default", "tap_zone_default"),
                            ("mirrored", "tap_zone_mirrored"),
                            ("swipe", "swipe_only"),
                        ],
                        true,
                        "",
                    ),
                    setting(
                        "night_mode",
                        "night_mode",
                        "toggle",
                        String::new(),
                        preferences.dark_mode,
                        hardware.application_dark_mode,
                        if hardware.application_dark_mode {
                            ""
                        } else {
                            "application_dark_mode_unavailable"
                        },
                    ),
                ];
                if self.profile.is_scribe() {
                    rows.insert(
                        1,
                        setting_choice(
                            "screen_orientation",
                            "screen_orientation",
                            "portrait",
                            &[
                                ("portrait", "portrait"),
                                ("landscape", "landscape"),
                                ("auto", "orientation_auto"),
                            ],
                            false,
                            "pending_verification",
                        ),
                    );
                }
                rows
            }
            "device" => vec![
                setting(
                    "keep_awake",
                    "keep_awake",
                    "toggle",
                    String::new(),
                    preferences.keep_awake && hardware.keep_awake,
                    hardware.keep_awake,
                    if hardware.keep_awake {
                        ""
                    } else {
                        "hardware_unavailable"
                    },
                ),
                setting(
                    "sleep_cover",
                    "sleep_cover",
                    "toggle",
                    String::new(),
                    preferences.sleep_cover,
                    true,
                    "",
                ),
            ],
            "about" => vec![
                setting(
                    "version",
                    "version",
                    "info",
                    env!("CARGO_PKG_VERSION").into(),
                    false,
                    false,
                    "",
                ),
                setting(
                    "check_update",
                    "check_update",
                    "action",
                    String::new(),
                    false,
                    false,
                    "no_update_channel",
                ),
                setting(
                    "auto_update",
                    "auto_update",
                    "toggle",
                    String::new(),
                    false,
                    false,
                    "no_update_channel",
                ),
                setting(
                    "update_proxy",
                    "update_proxy",
                    "toggle",
                    String::new(),
                    false,
                    false,
                    "no_update_channel",
                ),
                setting(
                    "license",
                    "license",
                    "info",
                    tr("license_pending"),
                    false,
                    false,
                    "",
                ),
            ],
            _ => Vec::new(),
        };
        Ok(rows)
    }

    fn choose_setting(&mut self, id: &str, choice: &str) -> Result<()> {
        let rows = self.setting_rows()?;
        let Some(row) = rows
            .iter()
            .find(|row| row.id.as_str() == id && row.kind.as_str() == "choice" && row.enabled)
        else {
            return Ok(());
        };
        if !row
            .options
            .iter()
            .any(|option| option.id.as_str() == choice)
            || row.selected_value.as_str() == choice
        {
            return Ok(());
        }
        if !self.prefs.set_choice(id, choice) {
            return Ok(());
        }
        if id == "view_mode" {
            self.library_page = 0;
        }
        self.prefs.normalize_for_device(self.profile.is_scribe());
        self.save_preferences()?;
        if id == "refresh_interval" {
            self.platform
                .set_reader_refresh_interval(self.prefs.reader.refresh_interval)?;
        }
        self.settings_status = tr("local_saved");
        Ok(())
    }

    fn setting_action(&mut self, id: &str) -> Result<()> {
        if id == "login" {
            self.navigate(Route::Login, true)?;
            if self.online && !self.demo {
                self.begin_login();
            }
            return Ok(());
        }
        if !self
            .setting_rows()?
            .iter()
            .any(|entry| entry.id.as_str() == id && entry.enabled)
        {
            return Ok(());
        }
        match id {
            "clear_cache" => {
                self.confirm_action(
                    "clear-cache",
                    "cache_confirm",
                    "cache_confirm",
                    "clear_cache",
                );
                Ok(())
            }
            "scan_local" => self.scan_imports(),
            _ => Ok(()),
        }
    }

    /// Borrow the current metadata when a consumer must admit a selected field
    /// before constructing an owned article snapshot.
    fn publication_article_ref(&self, id: &str) -> Option<&MpArticle> {
        #[cfg(test)]
        super::library::observe_qr_input_work(|work| work.article_lookups += 1);
        self.mp_articles
            .values()
            .flat_map(|articles| articles.iter())
            .find(|article| article.id == id)
    }

    fn article_metadata_with_budget<'a>(
        &'a self,
        id: &str,
        budget: &mut ArticleProjectionBudget,
    ) -> Result<Option<&'a MpArticle>> {
        if id.is_empty() {
            return Ok(None);
        }
        ArticleProjectionBudget::identifier(id)?;
        for article in self
            .mp_articles
            .values()
            .flat_map(|articles| articles.iter())
        {
            budget.record(&article.id, id)?;
            #[cfg(test)]
            observe_article_projection(|stats| stats.metadata_visits += 1);
            if article.id == id {
                return Ok(Some(article));
            }
        }
        Ok(None)
    }

    fn projected_article_title(
        &self,
        id: &str,
        metadata: Option<&MpArticle>,
        budget: &mut ArticleProjectionBudget,
    ) -> Result<String> {
        if let Some(article) = metadata {
            budget.output(&[&article.title], std::mem::size_of::<String>(), 1)?;
            return Ok(article_projection_owned(&article.title));
        }
        let title = if id.is_empty() {
            tr("publications")
        } else {
            let key = budget.key("article:", id)?;
            self.store
                .book_title(&key)?
                .unwrap_or_else(|| tr("publications"))
        };
        // The Store admits its one selected title before ownership. This
        // separate forecast precedes the title's UI/layout copies.
        budget.output(&[&title], std::mem::size_of::<String>(), 1)?;
        Ok(title)
    }

    fn article_list_projection<'a>(
        &'a self,
        global: &ArticleModel,
        budget: &mut ArticleProjectionBudget,
    ) -> Result<ArticleListProjection<'a>> {
        let articles = self
            .mp_articles
            .get(&self.article.account_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let size = if self.profile.is_scribe() { 11 } else { 8 };
        let pages = page_count(articles.len(), size);
        let page = (global.get_list_page().max(1) as usize - 1).min(pages - 1);
        let start = page * size;
        let end = (start + size).min(articles.len());
        let visible = if global.get_list_open() {
            &articles[start..end]
        } else {
            &[]
        };
        // History stays resident and unchanged. The complete borrowed ID work
        // is admitted before any key, title, or row from this operation exists.
        for article in articles {
            budget.record(&article.id, &self.article.id)?;
        }
        let position = articles
            .iter()
            .position(|article| article.id == self.article.id);
        let next = position.and_then(|index| articles.get(index + 1));
        budget.output(
            &[next
                .map(|article| article.title.as_str())
                .unwrap_or_default()],
            std::mem::size_of::<String>(),
            1,
        )?;
        for article in visible {
            budget.output(
                &[&article.id, &article.title],
                std::mem::size_of::<ReaderEntry>() + ARTICLE_DATE_OUTPUT_BYTES + 128,
                1,
            )?;
        }
        let mut rows = Vec::with_capacity(visible.len());
        let mut cached = 0usize;
        for (index, article) in articles.iter().enumerate() {
            let key = budget.key("article:", &article.id)?;
            let available = self.store.chapter_body_present(&key, 0)?;
            cached += usize::from(available);
            #[cfg(test)]
            observe_article_projection(|stats| {
                stats.owner_visits += 1;
                stats.presence_reads += 1;
            });
            if global.get_list_open() && index >= start && index < end {
                rows.push((article, available));
            }
        }
        Ok(ArticleListProjection {
            articles,
            next,
            rows,
            cached,
            page,
            pages,
        })
    }

    pub(super) fn request_article(&mut self, id: &str) -> Result<()> {
        if id.is_empty() || self.pending_articles.contains(id) {
            return Ok(());
        }
        if self
            .store
            .chapter_body_present(&format!("article:{id}"), 0)?
        {
            return Ok(());
        }
        if self.demo {
            self.show_toast("demo_notice");
        } else if self.force_offline {
            self.show_toast("network_unavailable");
        } else if !self.jobs.capabilities().mp_articles {
            self.show_toast("web_login_required");
        } else {
            self.pending_articles.insert(id.into());
            let request = self.request();
            self.pending_article_requests.insert(id.into(), request);
            self.settle_job_submission(self.jobs.article(request, id.into()));
        }
        Ok(())
    }

    pub(super) fn prepare_article_load(&self, id: &str) -> Result<PreparedArticleLoad> {
        let layout_budget = crate::reader::LayoutBudget::default();
        let old = layout_budget.hold(self.retained_layout_metadata()?)?;
        let mut budget = ArticleProjectionBudget::new();
        let metadata = self.article_metadata_with_budget(id, &mut budget)?;
        let account = metadata
            .map(|article| article.account_id.as_str())
            .filter(|id| !id.is_empty())
            .or(self.mp_account.as_deref())
            .unwrap_or_default();
        if !account.is_empty() {
            validate_publication_owner(account, MAX_PUBLICATION_OWNER_BYTES)?;
        }
        budget.output(&[id, account], 2 * std::mem::size_of::<String>(), 1)?;
        let (body, html, page) = if id.is_empty() {
            (String::new(), String::new(), 0)
        } else {
            let book = budget.key("article:", id)?;
            let html_key = budget.key("article.html:", id)?;
            let page_key = budget.key("article.page:", id)?;
            (
                self.store.chapter_body(&book, 0)?.unwrap_or_default(),
                self.store.setting::<String>(&html_key)?.unwrap_or_default(),
                self.store.setting::<usize>(&page_key)?.unwrap_or(0),
            )
        };
        let document = checked_article_document_in(&html, &body, &layout_budget)?;
        let images = super::article_images::PreparedImageWork::prepare_admitted(
            document.try_images_in(&layout_budget)?,
            &layout_budget,
        )
        .map_err(article_parse_failure)?;
        let title = self.projected_article_title(id, metadata, &mut budget)?;
        let layout = self.prepare_article_layout(
            id,
            page,
            false,
            &document,
            &title,
            &self.prefs.article,
            Some(images),
            &layout_budget,
            old,
        )?;
        Ok(PreparedArticleLoad {
            id: article_projection_owned(id),
            account_id: article_projection_owned(account),
            body,
            html,
            layout,
        })
    }

    pub(super) fn load_article(&mut self, id: &str) -> Result<()> {
        let prepared = self.prepare_article_load(id)?;
        self.install_article_load(prepared)
    }

    pub(super) fn prepare_incoming_article(
        &self,
        request: u64,
        id: &str,
        content: source::ChapterContent,
    ) -> Result<PreparedIncomingArticle> {
        let layout_budget = crate::reader::LayoutBudget::default();
        let old = layout_budget.hold(self.retained_layout_metadata()?)?;
        let mut budget = ArticleProjectionBudget::new();
        ArticleProjectionBudget::identifier(id)?;
        let raw_bytes = ArticleProjectionBudget::sum(content.html.len(), content.plain_text.len())?;
        budget.work(raw_bytes.checked_mul(2).ok_or(ArticleProjectionLimit {
            resource: "incoming body work bytes",
            limit: budget.limits.work_bytes,
            requested: usize::MAX,
        })?)?;
        let metadata = self.article_metadata_with_budget(id, &mut budget)?;
        if let Some(metadata) = metadata {
            if !metadata.account_id.is_empty() {
                validate_publication_owner(&metadata.account_id, MAX_PUBLICATION_OWNER_BYTES)?;
            }
            budget.output(
                &[
                    &metadata.title,
                    &metadata.author,
                    &metadata.cover_url,
                    &metadata.account_id,
                ],
                std::mem::size_of::<IncomingArticleMetadata>(),
                2,
            )?;
        }
        budget.output(&[id], std::mem::size_of::<String>(), 1)?;
        // This owns the queued payload without copying it. Both checked stages
        // end before pending work, source status, SQL or resident state changes.
        let document =
            checked_article_document_in(&content.html, &content.plain_text, &layout_budget)?;
        let reuse_images = self.article.id == id
            && self.article.body == content.plain_text
            && self.article.html == content.html
            && self.article.image_work.is_ready_for(id);
        let images = if reuse_images {
            None
        } else {
            Some(
                super::article_images::PreparedImageWork::prepare_admitted(
                    document.try_images_in(&layout_budget)?,
                    &layout_budget,
                )
                .map_err(article_parse_failure)?,
            )
        };
        let title = if let Some(metadata) = metadata {
            article_projection_owned(&metadata.title)
        } else {
            let key = budget.key("article:", id)?;
            let title = self
                .store
                .book_title(&key)?
                .unwrap_or_else(|| id.to_owned());
            budget.output(&[&title], std::mem::size_of::<String>(), 1)?;
            title
        };
        let metadata = metadata.map(|metadata| IncomingArticleMetadata {
            title: article_projection_owned(&metadata.title),
            author: article_projection_owned(&metadata.author),
            cover_url: article_projection_owned(&metadata.cover_url),
            account_id: article_projection_owned(&metadata.account_id),
        });
        let page = if self.article.id == id {
            self.article.page
        } else {
            0
        };
        let layout = self.prepare_article_layout(
            id,
            page,
            self.article.id == id,
            &document,
            &title,
            &self.prefs.article,
            images,
            &layout_budget,
            old,
        )?;
        Ok(PreparedIncomingArticle {
            id: article_projection_owned(id),
            request,
            account_epoch: self.jobs.account_epoch(),
            active_receiver: self.article.id == id
                && matches!(&self.route, Route::Article(current) if current == id),
            content,
            metadata,
            layout,
        })
    }

    pub(super) fn install_incoming_article(
        &mut self,
        prepared: PreparedIncomingArticle,
    ) -> Result<bool> {
        if !prepared.active_receiver
            || prepared.account_epoch != self.jobs.account_epoch()
            || self.article.id != prepared.id
            || !matches!(&self.route, Route::Article(current) if current == &prepared.id)
        {
            return Ok(false);
        }
        let PreparedIncomingArticle {
            id,
            content,
            metadata,
            layout,
            ..
        } = prepared;
        self.article.body = content.plain_text;
        self.article.html = content.html;
        if let Some(metadata) = metadata {
            self.article.account_id = metadata.account_id;
        }
        let _ = id;
        self.install_article_layout(layout);
        self.finish_article_reflow()?;
        Ok(true)
    }

    pub(super) fn install_article_load(&mut self, prepared: PreparedArticleLoad) -> Result<()> {
        let PreparedArticleLoad {
            id,
            account_id,
            body,
            html,
            layout,
        } = prepared;
        self.article.id = id;
        self.article.account_id = account_id;
        self.article.body = body;
        self.article.html = html;
        self.install_article_layout(layout);
        if let Some(ui) = self.ui.upgrade() {
            let global = ui.global::<ArticleModel>();
            global.set_menu(false);
            global.set_list_open(false);
            global.set_qr_open(false);
            global.set_tool("".into());
            global.set_list_page(1);
        }
        self.finish_article_reflow()?;
        if self.article.body.trim().is_empty() && self.article.html.trim().is_empty() {
            let id = self.article.id.clone();
            self.request_article(&id)?;
        }
        Ok(())
    }

    pub(super) fn reflow_article(&mut self) -> Result<()> {
        self.reflow_article_with_preferences(&self.prefs.article.clone())
    }

    fn reflow_article_with_preferences(
        &mut self,
        preferences: &crate::preferences::ArticlePreferences,
    ) -> Result<()> {
        let layout_budget = crate::reader::LayoutBudget::default();
        let old = layout_budget.hold(self.retained_layout_metadata()?)?;
        let document =
            checked_article_document_in(&self.article.html, &self.article.body, &layout_budget)?;
        let images = if self.article.image_work.is_ready_for(&self.article.id) {
            None
        } else {
            Some(
                super::article_images::PreparedImageWork::prepare_admitted(
                    document.try_images_in(&layout_budget)?,
                    &layout_budget,
                )
                .map_err(article_parse_failure)?,
            )
        };
        let title = self.article_title()?;
        let prepared = self.prepare_article_layout(
            &self.article.id,
            self.article.page,
            true,
            &document,
            &title,
            preferences,
            images,
            &layout_budget,
            old,
        )?;
        self.prefs.article = preferences.clone();
        self.install_article_layout(prepared);
        self.finish_article_reflow()
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_article_layout(
        &self,
        id: &str,
        requested_page: usize,
        preserve_current: bool,
        document: &ArticleDocument,
        title: &str,
        preferences: &crate::preferences::ArticlePreferences,
        images: Option<super::article_images::PreparedImageWork>,
        budget: &crate::reader::LayoutBudget,
        old: crate::reader::layout_budget::LayoutLease,
    ) -> Result<PreparedArticleLayout> {
        let previous_anchor = if requested_page == 0 {
            None
        } else if !preserve_current || self.article.id != id || self.article.pages.is_empty() {
            self.store
                .setting::<(usize, u32)>(&format!("article.anchor:{id}"))?
        } else {
            self.article
                .pages
                .get(requested_page)
                .into_iter()
                .flatten()
                .find_map(|block| article_anchor(&block.id))
        };
        let images = if let Some(images) = images {
            let mut work = images.into_work(id, budget)?;
            let mut loaded = BTreeMap::new();
            let mut loaded_layout = budget.lease();
            #[cfg(test)]
            let mut cache_checks = 0;
            // Resolve existing local pictures once per document. Reflow and
            // idle ticks reuse these identities without rehashing every file.
            for index in 0..work.images().len() {
                let image = work.image(index).unwrap();
                let image_id = image.id.as_str();
                budget.work(
                    id.len()
                        .saturating_add(image_id.len())
                        .saturating_add(image.url.len()),
                )?;
                let source_key = image_processing::article_cache_key(id, image_id, &image.url);
                let (key, _key_owner) = budget.copy(&source_key)?;
                #[cfg(test)]
                {
                    cache_checks += 1;
                }
                match self.cache.get(&key) {
                    Ok(Some(path)) => {
                        let bytes = path
                            .as_os_str()
                            .as_encoded_bytes()
                            .len()
                            .checked_add(128)
                            .ok_or(crate::reader::LayoutLimitError {
                                kind: crate::reader::LayoutLimitKind::Metadata,
                                used: 0,
                                requested: usize::MAX,
                                limit: crate::reader::layout_budget::DEFAULT_LAYOUT_METADATA_BYTES,
                            })?;
                        loaded_layout.merge(budget.copy_storage(bytes)?);
                        let (key, owner) = budget.copy(image_id)?;
                        loaded_layout.merge(owner);
                        loaded.insert(key, path);
                    }
                    Ok(None) => {}
                    Err(error) if Self::view_capacity(&error) => return Err(error),
                    Err(_) => {
                        work.fail(index);
                    }
                }
                work.mark_cache_checked(index);
            }
            Some(PreparedArticleImages {
                work,
                loaded,
                loaded_layout,
                #[cfg(test)]
                cache_checks,
            })
        } else {
            None
        };
        if document.is_empty() {
            return Ok(PreparedArticleLayout {
                pages: crate::reader::AdmittedLayout::new(Vec::new(), budget.lease()),
                page: 0,
                images,
                _old: old,
            });
        }
        let (width, height) = self.platform.logical_viewport_size();
        let margin = if self.profile.is_scribe() { 72.0 } else { 32.0 };
        let body_top = if self.profile.is_scribe() { 80.0 } else { 54.0 };
        let footer = if self.profile.is_scribe() { 56.0 } else { 48.0 };
        let layout = Layout {
            width: width as f32 - margin * 2.0,
            height: height as f32 - body_top - footer,
            font_size: preferences.font_size as f32,
            line_height: 1.85,
            indent: 0,
            paragraph_gap: [0.5, 0.9, 1.4][preferences.gap.min(2) as usize],
            justify: true,
        };
        let metrics = self
            .fonts
            .iter()
            .find(|font| font.id == preferences.font && font.available)
            .or_else(|| {
                self.fonts
                    .iter()
                    .find(|font| font.id == "hei" && font.available)
            })
            .map(|font| font.metrics_for_weight(preferences.weight).clone())
            .unwrap_or_default();
        let quote_metrics = self
            .fonts
            .iter()
            .find(|font| font.id == "song" && font.available)
            .map(|font| font.metrics_for_weight(preferences.weight).clone())
            .unwrap_or_default();
        let options = ArticleLayoutOptions {
            images: ArticleImageGeometry {
                placeholder_height: if self.profile.is_scribe() {
                    220.0
                } else {
                    180.0
                },
                loaded_height: if self.profile.is_scribe() {
                    300.0
                } else {
                    220.0
                },
                caption_height: if self.profile.is_scribe() { 24.0 } else { 20.0 },
                caption_gap: if self.profile.is_scribe() { 10.0 } else { 8.0 },
            },
            quote: ArticleQuoteGeometry {
                horizontal_padding: if self.profile.is_scribe() { 36.0 } else { 24.0 },
                vertical_padding: if self.profile.is_scribe() { 22.0 } else { 16.0 },
                font_scale: 0.95,
                line_height: if self.profile.is_scribe() { 1.85 } else { 1.8 },
            },
            quote_metrics: Some(&quote_metrics),
        };
        let heading = self.article_heading_height_in(title, layout.width, budget)?;
        let loaded = images
            .as_ref()
            .map(|images| &images.loaded)
            .unwrap_or(&self.article.loaded_images);
        let pages = document.try_paginate_with_options_in(
            &layout,
            &metrics,
            |image| loaded.contains_key(&image.id) || loaded.contains_key(&image.url),
            (layout.height - heading - 76.0).max(layout.font_size * 1.85),
            &options,
            budget,
        )?;
        let mut selected_page = requested_page;
        if let Some((source, offset)) = previous_anchor {
            if let Some((page, _)) = pages
                .iter()
                .enumerate()
                .flat_map(|(page, blocks)| {
                    blocks.iter().filter_map(move |block| {
                        article_anchor(&block.id).map(|anchor| (page, anchor))
                    })
                })
                .filter(|(_, (candidate_source, candidate_offset))| {
                    *candidate_source == source && *candidate_offset <= offset
                })
                .max_by_key(|(_, (_, candidate_offset))| *candidate_offset)
            {
                selected_page = page;
            }
        }
        selected_page = selected_page.min(pages.len().saturating_sub(1));
        Ok(PreparedArticleLayout {
            pages,
            page: selected_page,
            images,
            _old: old,
        })
    }

    fn install_article_layout(&mut self, prepared: PreparedArticleLayout) {
        let PreparedArticleLayout {
            pages,
            page,
            images,
            _old,
        } = prepared;
        if let Some(images) = images {
            self.pending_article_images.clear();
            self.article.image_work = images.work;
            self.article.loaded_images = images.loaded;
            self.article.loaded_layout = images.loaded_layout;
            #[cfg(test)]
            {
                self.article.image_cache_checks += images.cache_checks;
            }
        }
        let (pages, owner) = pages.into_parts();
        self.article.pages = pages;
        self.article.page_layout = owner;
        self.article.page = page;
        #[cfg(test)]
        {
            self.article.document_parses += 1;
        }
    }

    fn finish_article_reflow(&mut self) -> Result<()> {
        if self.article.pages.is_empty() {
            return Ok(());
        }
        if self.prefs.download_article_images {
            self.article.image_work.start_auto();
        }
        self.continue_article_image_work()?;
        self.store
            .set_setting(&format!("article.read:{}", self.article.id), &true)?;
        self.persist_article_position()?;
        Ok(())
    }

    pub(super) fn render_article(&mut self) -> Result<()> {
        let Some(ui) = self.ui.upgrade() else {
            return Ok(());
        };
        let global = ui.global::<ArticleModel>();
        let mut budget = ArticleProjectionBudget::new();
        let metadata = self.article_metadata_with_budget(&self.article.id, &mut budget)?;
        let title_source = if let Some(article) = metadata {
            std::borrow::Cow::Borrowed(article.title.as_str())
        } else if self.article.id.is_empty() {
            std::borrow::Cow::Owned(tr("publications"))
        } else {
            let key = budget.key("article:", &self.article.id)?;
            std::borrow::Cow::Owned(
                self.store
                    .book_title(&key)?
                    .unwrap_or_else(|| tr("publications")),
            )
        };
        let mut account_source = None;
        for account in &self.mp_accounts {
            budget.record(&account.id, &self.article.account_id)?;
            if account.id == self.article.account_id {
                account_source = Some(account.name.as_str());
                break;
            }
        }
        let fallback_account = account_source.is_none().then(|| tr("publications"));
        let account_source = account_source.unwrap_or_else(|| fallback_account.as_deref().unwrap());
        budget.output(
            &[title_source.as_ref()],
            std::mem::size_of::<String>(),
            if self.article.page == 0 && !self.article.pages.is_empty() {
                2
            } else {
                1
            },
        )?;
        budget.output(&[account_source], std::mem::size_of::<String>() + 4, 1)?;
        budget.output(
            &[metadata
                .map(|article| article.author.as_str())
                .unwrap_or_default()],
            2 * std::mem::size_of::<String>() + ARTICLE_DATE_OUTPUT_BYTES + 3,
            if self.article.page == 0 && !self.article.pages.is_empty() {
                2
            } else {
                1
            },
        )?;
        budget.output(&[&self.article.id], std::mem::size_of::<String>() + 512, 1)?;
        for block in self
            .article
            .pages
            .get(self.article.page)
            .into_iter()
            .flatten()
        {
            budget.output(
                &[&block.id, &block.kind, &block.text, &block.meta],
                std::mem::size_of::<ArticleBlock>(),
                1,
            )?;
            let bytes = ArticleProjectionBudget::sum(
                "article-image::".len(),
                ArticleProjectionBudget::sum(self.article.id.len(), block.id.len())?,
            )?;
            budget.work(bytes.checked_mul(4).ok_or(ArticleProjectionLimit {
                resource: "image key work bytes",
                limit: budget.limits.work_bytes,
                requested: usize::MAX,
            })?)?;
        }
        let projection = self.article_list_projection(&global, &mut budget)?;
        let images = self.article.image_work.images();
        for image in images {
            budget.record(&image.id, &image.id)?;
        }
        let loaded_images = images
            .iter()
            .filter(|image| self.article.loaded_images.contains_key(&image.id))
            .count();
        let starred = if self.article.id.is_empty() {
            false
        } else {
            self.store
                .setting::<bool>(&budget.key("article.star:", &self.article.id)?)?
                .unwrap_or(false)
        };
        let title = article_projection_owned(title_source.as_ref());
        let account = article_projection_owned(account_source);
        let meta = metadata
            .map(|article| format!("{} · {}", article.author, date_label(article.published_at)))
            .unwrap_or_default();
        let next_title = article_projection_owned(
            projection
                .next
                .map(|article| article.title.as_str())
                .unwrap_or_default(),
        );
        let mut entries = Vec::with_capacity(projection.rows.len());
        for (article, cached) in &projection.rows {
            #[cfg(test)]
            observe_article_projection(|stats| {
                stats.rows += 1;
                stats.row_source_bytes += article.id.len() + article.title.len();
            });
            entries.push(ReaderEntry {
                id: article_projection_owned(&article.id).into(),
                title: article_projection_owned(&article.title).into(),
                subtitle: format!(
                    "{} · {}",
                    date_label(article.published_at),
                    tr(if *cached {
                        "downloaded"
                    } else {
                        "not_downloaded"
                    })
                )
                .into(),
                kind: "article".into(),
                cached: *cached,
                ..Default::default()
            });
        }
        let list_summary = i18n::format(
            "articles_summary",
            &[
                ("count", projection.articles.len().to_string()),
                ("cached", projection.cached.to_string()),
            ],
        );
        let image_summary = i18n::format(
            "article_image_summary",
            &[
                ("count", images.len().to_string()),
                ("loaded", loaded_images.to_string()),
            ],
        );
        let mut blocks = Vec::new();
        if !self.article.pages.is_empty() {
            if self.article.page == 0 {
                let (width, _) = self.platform.logical_viewport_size();
                let margin = if self.profile.is_scribe() { 72.0 } else { 32.0 };
                blocks.push(ArticleBlock {
                    id: "article-title".into(),
                    kind: "title".into(),
                    text: article_projection_owned(&title).into(),
                    height: self.article_heading_height(&title, width as f32 - margin * 2.0)?
                        as i32,
                    ..Default::default()
                });
                blocks.push(ArticleBlock {
                    id: "article-meta".into(),
                    kind: "meta".into(),
                    text: article_projection_owned(&meta).into(),
                    height: 76,
                    ..Default::default()
                });
            }
            for block in self
                .article
                .pages
                .get(self.article.page)
                .into_iter()
                .flatten()
            {
                let picture = self.article.loaded_images.get(&block.id);
                let pending = self
                    .pending_article_images
                    .contains(&format!("article-image:{}:{}", self.article.id, block.id));
                blocks.push(ArticleBlock {
                    id: article_projection_owned(&block.id).into(),
                    kind: article_projection_owned(&block.kind).into(),
                    text: article_projection_owned(&block.text).into(),
                    meta: if pending {
                        tr("synchronizing")
                    } else {
                        article_projection_owned(&block.meta)
                    }
                    .into(),
                    loaded: picture.is_some(),
                    picture: slint::Image::default(),
                    height: block.height.min(i32::MAX as u32) as i32,
                    letter_spacing: block.letter_spacing,
                    frame_height: block.frame_height,
                    caption_height: block.caption_height,
                    caption_gap: block.caption_gap,
                    inset_top: block.inset_top,
                    inset_bottom: block.inset_bottom,
                    inset_horizontal: block.inset_horizontal,
                    content_height: block.content_height,
                });
            }
        }
        let (blocks, image_scene) = self.prepare_article_image_scene(blocks)?;
        let font = self
            .fonts
            .iter()
            .find(|font| font.id == self.prefs.article.font && font.available)
            .or_else(|| {
                self.fonts
                    .iter()
                    .find(|font| font.id == "hei" && font.available)
            });
        global.set_font_family(
            font.map(|font| font.family.clone())
                .unwrap_or_else(|| "sans-serif".into())
                .into(),
        );
        global.set_font_choice(self.prefs.article.font.clone().into());
        global.set_fonts(model(
            self.fonts
                .iter()
                .map(|font| ReaderFont {
                    id: font.id.clone().into(),
                    label: font.label.clone().into(),
                    family: font.family.clone().into(),
                    available: font.available,
                })
                .collect(),
        ));
        global.set_font_size(self.prefs.article.font_size as i32);
        global.set_weight(self.prefs.article.weight as i32);
        global.set_line_height(1.85);
        global.set_gap_choice(
            ["tight", "mid", "loose"][self.prefs.article.gap.min(2) as usize].into(),
        );
        global.set_title(title.into());
        global.set_account_initial(
            account
                .chars()
                .next()
                .map(String::from)
                .unwrap_or_default()
                .into(),
        );
        global.set_account(account.into());
        global.set_meta(meta.into());
        global.set_current_id(article_projection_owned(&self.article.id).into());
        global.set_page((self.article.page + 1) as i32);
        global.set_page_count(self.article.pages.len().max(1) as i32);
        global.set_footer(
            format!(
                "{} / {}",
                self.article.page + 1,
                self.article.pages.len().max(1)
            )
            .into(),
        );
        global.set_auto_images(self.prefs.download_article_images);
        global.set_starred(starred);
        global.set_can_download(
            !self.force_offline && !self.demo && self.jobs.capabilities().mp_articles,
        );
        let downloading = self.pending_articles.contains(&self.article.id);
        global.set_download_busy(downloading);
        global.set_status(
            if downloading {
                tr("loading_article")
            } else if !self.online {
                tr("offline")
            } else {
                String::new()
            }
            .into(),
        );
        global.set_has_next(projection.next.is_some());
        global.set_next_title(next_title.into());
        global.set_list_summary(list_summary.into());
        global.set_image_summary(image_summary.into());
        global.set_articles(model(entries));
        global.set_list_page((projection.page + 1) as i32);
        global.set_list_pages(projection.pages as i32);
        self.publish_article_images(blocks, image_scene)
    }

    pub(super) fn article_action(&mut self, action: &str, value: &str) -> Result<()> {
        let Some(ui) = self.ui.upgrade() else {
            return Ok(());
        };
        let global = ui.global::<ArticleModel>();
        let mut reflow = false;
        let mut proposed = self.prefs.clone();
        match action {
            "article.back" => self.back()?,
            "article.download" => self.request_article(&self.article.id.clone())?,
            "article.turn" => {
                self.article.page = move_page(
                    self.article.page,
                    value.parse().unwrap_or(0),
                    self.article.pages.len(),
                );
                self.persist_article_position()?;
                self.refresh_display(RefreshReason::ReaderTurn);
            }
            "article.tap" => {
                if global.get_list_open() || global.get_qr_open() {
                    return Ok(());
                }
                let fraction = value.parse::<f32>().unwrap_or(0.5);
                if global.get_menu() || (0.3..=0.7).contains(&fraction) {
                    global.set_menu(!global.get_menu());
                } else {
                    self.article.page = move_page(
                        self.article.page,
                        if fraction < 0.3 { -1 } else { 1 },
                        self.article.pages.len(),
                    );
                    self.persist_article_position()?;
                    self.refresh_display(RefreshReason::ReaderTurn);
                }
            }
            "article.menu" => global.set_menu(if value == "toggle" {
                !global.get_menu()
            } else {
                value == "true"
            }),
            "article.tool" if ["images", "type"].contains(&value) => {
                global.set_tool(
                    if global.get_tool().as_str() == value {
                        ""
                    } else {
                        value
                    }
                    .into(),
                );
                global.set_menu(true);
            }
            "article.list" => global.set_list_open(value == "true"),
            "article.list-page" => {
                let next = move_page(
                    global.get_list_page().max(1) as usize - 1,
                    value.parse().unwrap_or(0),
                    global.get_list_pages().max(1) as usize,
                );
                global.set_list_page((next + 1) as i32);
            }
            "article.open" => {
                let mut budget = ArticleProjectionBudget::new();
                if self
                    .article_metadata_with_budget(value, &mut budget)?
                    .is_some()
                {
                    self.navigate(Route::Article(value.into()), false)?;
                }
            }
            "article.next" => {
                let mut budget = ArticleProjectionBudget::new();
                let articles = self
                    .mp_articles
                    .get(&self.article.account_id)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                for article in articles {
                    budget.record(&article.id, &self.article.id)?;
                }
                let next = articles
                    .iter()
                    .position(|article| article.id == self.article.id)
                    .and_then(|index| articles.get(index + 1));
                let next = next
                    .map(|article| -> Result<String> {
                        budget.output(&[&article.id], std::mem::size_of::<String>(), 1)?;
                        Ok(article_projection_owned(&article.id))
                    })
                    .transpose()?;
                if let Some(id) = next {
                    self.navigate(Route::Article(id), false)?;
                }
            }
            "article.star" => {
                let key = format!("article.star:{}", self.article.id);
                let starred = self.store.setting::<bool>(&key)?.unwrap_or(false);
                self.store.set_setting(&key, &!starred)?;
                self.show_toast("article_local_star");
            }
            "article.qr" => {
                if value == "false" {
                    global.set_qr_open(false);
                } else if let Some(article) = self.publication_article_ref(&self.article.id) {
                    if let Err(error) = admit_qr_payload(&article.original_url) {
                        self.show_toast("api_error");
                        // Stop this dispatch before its normal render can copy
                        // the same rejected metadata. Existing QR/menu and SQL
                        // state remain available; no truncated link is encoded.
                        return Err(error.into());
                    }
                    #[cfg(test)]
                    super::library::observe_qr_input_work(|work| work.uri_parses += 1);
                    if url::Url::parse(&article.original_url)
                        .is_ok_and(|url| ["http", "https"].contains(&url.scheme()))
                    {
                        let qr = qr_image(&article.original_url)?;
                        #[cfg(test)]
                        super::library::observe_qr_input_work(|work| work.publications += 1);
                        global.set_qr(qr);
                        global.set_qr_open(true);
                        global.set_menu(false);
                    } else {
                        self.show_toast("article_url_missing");
                    }
                } else {
                    self.show_toast("article_url_missing");
                }
            }
            "article.load-image" => {
                reflow = self.queue_article_image(value)?;
            }
            "article.load-all-images" => {
                if self.force_offline {
                    self.show_toast("network_unavailable");
                    return Ok(());
                }
                if !self.demo {
                    self.online = true;
                }
                self.article.image_work.request_all(true);
                reflow = self.continue_article_image_work()?;
            }
            "article.auto-images" => {
                self.prefs.download_article_images = !self.prefs.download_article_images;
                self.save_preferences()?;
                if self.prefs.download_article_images {
                    if !self.force_offline && !self.demo {
                        self.online = true;
                    }
                    self.article.image_work.start_auto();
                    // This is a new explicit enable action, unlike ordinary
                    // reflow. Resume remaining work without clearing failures.
                    self.article.image_work.request_all(false);
                } else {
                    self.article.image_work.cancel_intents();
                }
                reflow = self.continue_article_image_work()?;
            }
            "article.font-size" => {
                proposed.adjust_for_device(
                    "article_font_size",
                    value.parse().unwrap_or(0),
                    self.profile.is_scribe(),
                );
                reflow = true;
            }
            "article.font-size-select" => {
                let sizes: &[u32] = if self.profile.is_scribe() {
                    &[16, 18, 20, 22, 24, 26, 28, 30, 32]
                } else {
                    &[15, 16, 17, 18, 19, 20, 22, 24, 26]
                };
                if let Ok(size) = value.parse::<u32>() {
                    if sizes.contains(&size) && proposed.article.font_size != size {
                        proposed.article.font_size = size;
                        reflow = true;
                    }
                }
            }
            "article.font" => {
                if self
                    .fonts
                    .iter()
                    .any(|font| font.id == value && font.available)
                {
                    proposed.article.font = value.into();
                    reflow = true;
                } else {
                    self.show_toast("font_unavailable");
                }
            }
            "article.weight" => {
                reflow = proposed.set_choice("article_weight", value);
            }
            "article.paragraph-gap" => {
                proposed.article.gap = match value {
                    "tight" => 0,
                    "loose" => 2,
                    _ => 1,
                };
                reflow = true;
            }
            _ => {}
        }
        if reflow {
            proposed.download_article_images = self.prefs.download_article_images;
            proposed.normalize_for_device(self.profile.is_scribe());
            self.reflow_article_with_preferences(&proposed.article)?;
            self.save_preferences()?;
            self.refresh_display(RefreshReason::Content);
        }
        Ok(())
    }

    fn article_title(&self) -> Result<String> {
        let mut budget = ArticleProjectionBudget::new();
        let metadata = self.article_metadata_with_budget(&self.article.id, &mut budget)?;
        self.projected_article_title(&self.article.id, metadata, &mut budget)
    }

    fn article_heading_height(&self, title: &str, width: f32) -> Result<f32> {
        self.article_heading_height_in(title, width, &crate::reader::LayoutBudget::default())
    }

    fn article_heading_height_in(
        &self,
        title: &str,
        width: f32,
        budget: &crate::reader::LayoutBudget,
    ) -> Result<f32> {
        let size = if self.profile.is_scribe() { 38.0 } else { 28.0 };
        let metrics = self
            .fonts
            .iter()
            .find(|font| font.id == "song" && font.available)
            .map(|font| font.metrics_for_weight(700).clone())
            .unwrap_or_default();
        let layout = Layout {
            width,
            height: 10_000.0,
            font_size: size,
            line_height: 1.45,
            indent: 0,
            paragraph_gap: 0.0,
            justify: false,
        };
        let lines = Paginator::try_new_in(title, &layout, &metrics, budget)?
            .pages
            .iter()
            .map(|page| page.lines.len())
            .sum::<usize>()
            .max(1);
        Ok((lines as f32 * size * 1.45 + 12.0).min(240.0))
    }

    fn persist_article_position(&self) -> Result<()> {
        self.store.set_setting(
            &format!("article.page:{}", self.article.id),
            &self.article.page,
        )?;
        let anchor = if self.article.page == 0 {
            None
        } else {
            self.article
                .pages
                .get(self.article.page)
                .into_iter()
                .flatten()
                .find_map(|block| article_anchor(&block.id))
        };
        let key = format!("article.anchor:{}", self.article.id);
        if let Some(anchor) = anchor {
            self.store.set_setting(&key, &anchor)?;
        } else {
            self.store.delete_setting(&key)?;
        }
        Ok(())
    }

    fn queue_article_image(&mut self, image_id: &str) -> Result<bool> {
        if self.force_offline {
            self.show_toast("network_unavailable");
            return Ok(false);
        }
        if !self.demo {
            self.online = true;
        }
        self.article.image_work.request_one(image_id);
        self.continue_article_image_work()
    }

    pub(super) fn clear_article_image_work(&mut self) {
        self.article.image_work.clear();
        self.pending_article_images.clear();
    }

    /// Drain only current-document intent. Admission happens before cache I/O;
    /// no work or no capacity means an idle tick touches no image payloads.
    pub(super) fn continue_article_image_work(&mut self) -> Result<bool> {
        if self.force_offline
            || !self.online
            || self.demo
            || !matches!(&self.route, Route::Article(id) if id == &self.article.id)
            || !self.article.image_work.is_ready_for(&self.article.id)
            || !self.article.image_work.has_work()
        {
            return Ok(false);
        }
        let mut loaded = false;
        while self.jobs.cover_capacity() > 0 && self.article.image_work.available_capacity() > 0 {
            let Some(index) = self.article.image_work.next_candidate() else {
                break;
            };
            let Some(image) = self.article.image_work.image(index) else {
                continue;
            };
            let id = image.id.clone();
            let url = image.url.clone();
            let previous_path = self.article.loaded_images.get(&id).cloned();
            let checked = self.article.image_work.cache_checked(index);
            if checked && previous_path.as_ref().is_some_and(|path| path.is_file()) {
                continue;
            }
            let key = format!("article-image:{}:{id}", self.article.id);
            let cache_key = image_processing::article_cache_key(&self.article.id, &id, &url);
            if !checked || previous_path.is_some() {
                self.article.image_work.mark_cache_checked(index);
                #[cfg(test)]
                {
                    self.article.image_cache_checks += 1;
                }
                match self.cache.get(&cache_key) {
                    Ok(Some(path)) => {
                        loaded |= previous_path.as_ref() != Some(&path);
                        self.article.loaded_images.insert(id, path);
                        continue;
                    }
                    Ok(None) => {
                        loaded |= self.article.loaded_images.remove(&id).is_some();
                    }
                    Err(_) => {
                        loaded |= self.article.loaded_images.remove(&id).is_some();
                        self.article.image_work.fail(index);
                        self.show_toast("image_load_failed");
                        continue;
                    }
                }
            }
            let request = self.request();
            if self.article.image_work.admit(index, request) {
                self.pending_article_images.insert(key.clone());
                if self.settle_job_submission(self.jobs.article_image(
                    request,
                    self.article.id.clone(),
                    id,
                    url,
                )) {
                    break;
                }
            }
        }
        Ok(loaded)
    }
}

fn article_anchor(id: &str) -> Option<(usize, u32)> {
    let (source, offset) = id.strip_prefix("text-")?.split_once('-')?;
    Some((source.parse().ok()?, offset.parse().ok()?))
}

fn setting(
    id: &str,
    title: &str,
    kind: &str,
    value: String,
    checked: bool,
    enabled: bool,
    note: &str,
) -> SettingEntry {
    SettingEntry {
        id: id.into(),
        title: tr(title).into(),
        subtitle: if note.is_empty() {
            String::new()
        } else {
            tr(note)
        }
        .into(),
        value: value.into(),
        kind: kind.into(),
        checked,
        enabled,
        options: Default::default(),
        selected_value: Default::default(),
    }
}

fn setting_choice(
    id: &str,
    title: &str,
    selected: &str,
    options: &[(&str, &str)],
    enabled: bool,
    note: &str,
) -> SettingEntry {
    let mut entry = setting(id, title, "choice", String::new(), false, enabled, note);
    entry.selected_value = selected.into();
    entry.options = model(
        options
            .iter()
            .map(|(id, label)| SegmentedOption {
                id: (*id).into(),
                label: tr(label).into(),
            })
            .collect(),
    );
    entry
}

fn setting_row_height(row: &SettingEntry, scribe: bool) -> u32 {
    if row.kind.as_str() == "choice" {
        let base = if scribe { 104 } else { 100 };
        base + if row.subtitle.is_empty() {
            0
        } else if scribe {
            30
        } else {
            28
        }
    } else if row.subtitle.is_empty() {
        56
    } else {
        72
    }
}

fn settings_title(subpage: &str) -> &str {
    match subpage {
        "sync" => "reading_sync",
        "notes" => "highlights_thoughts",
        "storage" => "downloads_storage",
        "display" => "display_refresh",
        "device" => "device_network",
        "about" => "about",
        _ => "settings",
    }
}

fn popup_label(position: &str) -> String {
    tr(match position {
        "side" => "side_panel",
        "bottom" => "bottom_popup",
        _ => "center_popup",
    })
}

fn date_label(timestamp: i64) -> String {
    if timestamp <= 0 {
        return String::new();
    }
    DateTime::from_timestamp(timestamp, 0)
        .map(|instant| {
            instant
                .with_timezone(&FixedOffset::east_opt(8 * 3600).expect("valid Shanghai offset"))
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_default()
}

fn statistic_selection_label(period: &str, start: i64, index: usize, axis: &str) -> String {
    match period {
        "month" => DateTime::from_timestamp(start, 0)
            .map(|instant| {
                let day = instant.with_timezone(
                    &FixedOffset::east_opt(8 * 3600).expect("valid Shanghai offset"),
                );
                i18n::format(
                    "stats_month_day",
                    &[
                        ("month", day.month().to_string()),
                        ("day", (index + 1).to_string()),
                    ],
                )
            })
            .unwrap_or_else(|| axis.to_owned()),
        "year" => i18n::format("stats_month_name", &[("month", axis.to_owned())]),
        "total" => i18n::format("stats_year_name", &[("year", axis.to_owned())]),
        _ => axis.to_owned(),
    }
}

fn statistic_calendar_label(value: &str) -> String {
    let Some((year, month)) = value.split_once('-') else {
        return value.to_owned();
    };
    let Some(month) = month
        .parse::<u32>()
        .ok()
        .filter(|month| (1..=12).contains(month))
    else {
        return value.to_owned();
    };
    i18n::format(
        "stats_calendar_month",
        &[("year", year.to_owned()), ("month", month.to_string())],
    )
}

fn duration_label(seconds: u64) -> String {
    if seconds >= 3600 {
        i18n::format(
            "hours_minutes",
            &[
                ("hours", (seconds / 3600).to_string()),
                ("minutes", (seconds % 3600 / 60).to_string()),
            ],
        )
    } else if seconds >= 60 {
        i18n::format("minutes", &[("count", (seconds / 60).to_string())])
    } else {
        i18n::format("seconds", &[("count", seconds.to_string())])
    }
}

fn optional_duration_label(seconds: Option<u64>) -> String {
    seconds
        .map(duration_label)
        .unwrap_or_else(|| tr("stats_unavailable"))
}

fn size_label(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}
