//! Background source work. This module owns no UI objects, SQL, or data files.
use crate::download_diagnostics::{DownloadDiagnostics, DownloadRun, SUMMARY_INTERVAL, Stage};
use crate::preferences::{DEFAULT_DOWNLOAD_CONCURRENCY, MAX_DOWNLOAD_CONCURRENCY};
use crate::{domain, source};
use source::{
    AuthSession, ChapterContent, LoginFlow, QrLoginChallenge, QrLoginState, ReadStatistics,
    RemoteBook, RemoteChapter, RemoteReview, RemoteUnderline, ShelfSnapshot, SourceCapabilities,
    SourceError, StatisticsPeriod, WeReadClient,
};
use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::Sender,
};
use std::time::{Duration, Instant};
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore},
    task::AbortHandle,
};

const MAX_METADATA_REQUESTS: usize = 8;
const MAX_COVER_REQUESTS: usize = 2;
const MAX_PUBLIC_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_DOWNLOAD_CHAPTERS: usize = 8192;
const MAX_DOWNLOAD_IDENTIFIER_BYTES: usize = 4096;
const MAX_DOWNLOAD_METADATA_BYTES: usize = 64 * 1024 * 1024;

/// A checked, bodyless snapshot for only the remaining chapters of one task.
/// Cached text stays in SQL until its owner accepts a credited checkpoint.
#[derive(Debug)]
pub struct DownloadPlan {
    task_id: String,
    book_id: String,
    account_key: String,
    include_images: bool,
    indices: Vec<u32>,
    next_index: usize,
    chapters: BTreeMap<u32, DownloadPlanEntry>,
    states: Vec<DownloadPlanState>,
}

#[derive(Clone, Debug)]
struct DownloadPlanEntry {
    id: String,
    title: String,
    cached_full: bool,
    cached_images: bool,
}

#[derive(Clone, Debug)]
struct DownloadPlanState {
    status: domain::DownloadChapterStatus,
    retry_not_before: i64,
}

impl DownloadPlanEntry {
    fn reusable(&self, include_images: bool) -> bool {
        self.cached_full && (!include_images || self.cached_images)
    }
}

impl DownloadPlan {
    pub fn from_headers(
        task: &domain::DownloadTask,
        headers: &[domain::Chapter],
    ) -> source::Result<Self> {
        Self::from_headers_with_limit(task, headers, MAX_DOWNLOAD_METADATA_BYTES)
    }

    fn from_headers_with_limit(
        task: &domain::DownloadTask,
        headers: &[domain::Chapter],
        maximum_bytes: usize,
    ) -> source::Result<Self> {
        Self::from_headers_and_states(task, headers, None, maximum_bytes)
    }

    /// Use the owner's freshly reconciled per-chapter persistence snapshot.
    pub fn from_headers_with_states(
        task: &domain::DownloadTask,
        headers: &[domain::Chapter],
        states: &[domain::DownloadChapterState],
    ) -> source::Result<Self> {
        Self::from_headers_and_states(task, headers, Some(states), MAX_DOWNLOAD_METADATA_BYTES)
    }

    fn from_headers_and_states(
        task: &domain::DownloadTask,
        headers: &[domain::Chapter],
        persisted: Option<&[domain::DownloadChapterState]>,
        maximum_bytes: usize,
    ) -> source::Result<Self> {
        validate_download_task(task)?;
        if headers.len() > MAX_DOWNLOAD_CHAPTERS {
            return Err(SourceError::ResponseTooLarge);
        }
        if persisted.is_some_and(|states| states.len() != task.chapters.len()) {
            return Err(SourceError::InvalidResponse("download_states"));
        }
        let mut bytes = 0usize;
        // Admit all temporarily retained metadata and the selected copies before
        // building owned index entries. There is never a body-bearing plan.
        for part in [
            task.id.len(),
            task.book_id.len(),
            task.account_key.len(),
            task.error.as_ref().map_or(0, String::len),
            task.chapters
                .len()
                .checked_mul(2 * std::mem::size_of::<u32>())
                .ok_or(SourceError::ResponseTooLarge)?,
            task.chapters
                .len()
                .checked_mul(std::mem::size_of::<DownloadPlanState>() + std::mem::size_of::<bool>())
                .ok_or(SourceError::ResponseTooLarge)?,
            task.chapters.len().div_ceil(u64::BITS as usize) * std::mem::size_of::<u64>(),
            // Borrowed maps and duplicate sets are bounded metadata too.
            headers
                .len()
                .checked_mul(256)
                .ok_or(SourceError::ResponseTooLarge)?,
            std::mem::size_of::<Self>(),
        ] {
            reserve_download_metadata(&mut bytes, part, maximum_bytes)?;
        }
        if let Some(states) = persisted {
            for (ordinal, state) in states.iter().enumerate() {
                if state.ordinal != ordinal
                    || state.chapter_index != task.chapters[ordinal]
                    || !valid_download_identifier(&state.chapter_uid)
                    || state.retry_not_before < 0
                    || state.error.as_ref().is_some_and(|error| {
                        error.len() > MAX_DOWNLOAD_IDENTIFIER_BYTES
                            || error.chars().any(char::is_control)
                    })
                {
                    return Err(SourceError::InvalidResponse("download_states"));
                }
                for part in [
                    state.chapter_uid.len(),
                    state.error.as_ref().map_or(0, String::len),
                    std::mem::size_of::<domain::DownloadChapterState>(),
                ] {
                    reserve_download_metadata(&mut bytes, part, maximum_bytes)?;
                }
            }
        }
        let mut seen_indices = HashSet::new();
        let mut seen_ids = HashSet::new();
        for header in headers {
            if header.body.is_some() {
                return Err(SourceError::InvalidResponse("download_plan_body"));
            }
            if header.book_id != task.book_id
                || !valid_download_identifier(&header.id)
                || header.content_account_key.len() > MAX_DOWNLOAD_IDENTIFIER_BYTES
                || header.content_account_key.chars().any(char::is_control)
                || !seen_indices.insert(header.index)
                || !seen_ids.insert(header.id.as_str())
            {
                return Err(SourceError::InvalidResponse("download_header"));
            }
            for part in [
                header.book_id.len(),
                header.id.len(),
                header.title.len(),
                header.content_account_key.len(),
                std::mem::size_of::<domain::Chapter>(),
            ] {
                reserve_download_metadata(&mut bytes, part, maximum_bytes)?;
            }
        }
        let by_index = headers
            .iter()
            .map(|header| (header.index, header))
            .collect::<BTreeMap<_, _>>();
        let states = task
            .chapters
            .iter()
            .enumerate()
            .map(|(ordinal, index)| {
                let state = persisted.and_then(|states| states.get(ordinal));
                if let Some(state) = state {
                    let header = by_index
                        .get(index)
                        .ok_or(SourceError::InvalidResponse("missing_chapter"))?;
                    if state.chapter_uid != header.id {
                        return Err(SourceError::InvalidResponse("download_states"));
                    }
                    let reusable = header.cached
                        && header.content_status == domain::ContentStatus::Full
                        && header.content_account_key == task.account_key;
                    // The owner reconciles image proofs before supplying these
                    // states. Stale text evidence returns only this chapter to
                    // pending instead of losing the other completions.
                    let status =
                        if state.status == domain::DownloadChapterStatus::Complete && !reusable {
                            domain::DownloadChapterStatus::Pending
                        } else {
                            state.status
                        };
                    Ok(DownloadPlanState {
                        status,
                        retry_not_before: state.retry_not_before,
                    })
                } else {
                    Ok(DownloadPlanState {
                        status: if ordinal < task.next_index {
                            domain::DownloadChapterStatus::Complete
                        } else {
                            domain::DownloadChapterStatus::Pending
                        },
                        retry_not_before: 0,
                    })
                }
            })
            .collect::<source::Result<Vec<_>>>()?;
        let remaining = task
            .chapters
            .iter()
            .zip(&states)
            .filter(|(_, state)| state.status != domain::DownloadChapterStatus::Complete)
            .map(|(index, _)| *index)
            .collect::<HashSet<_>>();
        for header in headers {
            if remaining.contains(&header.index) {
                for part in [
                    header.id.len(),
                    header.title.len(),
                    2 * std::mem::size_of::<DownloadPlanEntry>(),
                ] {
                    reserve_download_metadata(&mut bytes, part, maximum_bytes)?;
                }
            }
        }
        reserve_download_metadata(&mut bytes, task.id.len(), maximum_bytes)?;
        reserve_download_metadata(&mut bytes, task.book_id.len(), maximum_bytes)?;
        reserve_download_metadata(&mut bytes, task.account_key.len(), maximum_bytes)?;

        let mut selected = BTreeMap::new();
        for header in headers {
            if remaining.contains(&header.index) {
                selected.insert(header.index, header);
            }
        }
        if selected.len() != remaining.len() {
            return Err(SourceError::InvalidResponse("missing_chapter"));
        }
        let chapters = selected
            .into_iter()
            .map(|(index, header)| {
                (
                    index,
                    DownloadPlanEntry {
                        id: header.id.clone(),
                        title: header.title.clone(),
                        cached_full: header.cached
                            && header.content_status == domain::ContentStatus::Full
                            && !header.content_account_key.is_empty()
                            && header.content_account_key == task.account_key,
                        cached_images: false,
                    },
                )
            })
            .collect();
        Ok(Self {
            task_id: task.id.clone(),
            book_id: task.book_id.clone(),
            account_key: task.account_key.clone(),
            include_images: task.include_images,
            indices: task.chapters.clone(),
            next_index: task.next_index,
            chapters,
            states,
        })
    }

    /// Record only a fresh, complete image-cache decision for this exact UID.
    /// The UI verifies the actual resources again before acknowledging reuse.
    pub(crate) fn mark_complete_cached_images(
        &mut self,
        index: u32,
        chapter_id: &str,
    ) -> source::Result<()> {
        let entry = self
            .chapters
            .get_mut(&index)
            .ok_or(SourceError::InvalidResponse("cached_download_chapter"))?;
        if !self.include_images || !entry.cached_full || entry.id != chapter_id {
            return Err(SourceError::InvalidResponse("cached_download_chapter"));
        }
        entry.cached_images = true;
        Ok(())
    }

    fn validate_for(&self, task: &domain::DownloadTask) -> source::Result<()> {
        validate_download_task(task)?;
        if self.task_id != task.id
            || self.book_id != task.book_id
            || self.account_key != task.account_key
            || self.include_images != task.include_images
            || self.indices != task.chapters
            || self.next_index != task.next_index
        {
            return Err(SourceError::InvalidResponse("download_plan"));
        }
        Ok(())
    }
}

fn valid_download_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_DOWNLOAD_IDENTIFIER_BYTES
        && !value.chars().any(char::is_control)
}

fn validate_download_task(task: &domain::DownloadTask) -> source::Result<()> {
    if !valid_download_identifier(&task.id)
        || !valid_download_identifier(&task.book_id)
        || !valid_download_identifier(&task.account_key)
    {
        return Err(SourceError::InvalidResponse("download_task"));
    }
    if task.chapters.len() > MAX_DOWNLOAD_CHAPTERS {
        return Err(SourceError::ResponseTooLarge);
    }
    // A caller cannot attach an unpriced old diagnostic after constructing
    // the bodyless plan. Running tasks normally clear this status text.
    if task
        .error
        .as_ref()
        .is_some_and(|error| error.len() > MAX_DOWNLOAD_IDENTIFIER_BYTES)
    {
        return Err(SourceError::ResponseTooLarge);
    }
    if task.next_index > task.chapters.len()
        || task.chapters.iter().copied().collect::<HashSet<_>>().len() != task.chapters.len()
    {
        return Err(SourceError::InvalidResponse("download_cursor"));
    }
    Ok(())
}

fn reserve_download_metadata(used: &mut usize, bytes: usize, maximum: usize) -> source::Result<()> {
    *used = used
        .checked_add(bytes)
        .filter(|bytes| *bytes <= maximum)
        .ok_or(SourceError::ResponseTooLarge)?;
    Ok(())
}

#[derive(Debug)]
pub struct JobEvent {
    pub request: u64,
    /// Captured when the work starts, including anonymous and cancellation work.
    pub account_epoch: u64,
    pub result: JobResult,
    /// Work admission remains owned through queued payload processing or drop.
    pub(crate) credits: Option<JobCredits>,
}

#[derive(Debug)]
pub(crate) struct JobCredits {
    _first: OwnedSemaphorePermit,
    _second: Option<OwnedSemaphorePermit>,
}

impl JobEvent {
    /// Build a synchronous admission/control result without putting it in a queue.
    pub fn new(request: u64, account_epoch: u64, result: JobResult) -> Self {
        Self {
            request,
            account_epoch,
            result,
            credits: None,
        }
    }

    fn retain(mut self, first: OwnedSemaphorePermit, second: Option<OwnedSemaphorePermit>) -> Self {
        self.credits = Some(JobCredits {
            _first: first,
            _second: second,
        });
        self
    }
}

#[derive(Debug)]
pub enum JobResult {
    Shelf(source::Result<ShelfSnapshot>),
    Search(source::Result<Vec<RemoteBook>>),
    BookInfo {
        book_id: String,
        result: source::Result<RemoteBook>,
    },
    Catalog {
        book_id: String,
        result: source::Result<Vec<RemoteChapter>>,
    },
    Reviews {
        book_id: String,
        result: source::Result<Vec<RemoteReview>>,
    },
    Progress {
        book_id: String,
        result: source::Result<source::ReadingPosition>,
    },
    UploadProgress(source::Result<()>),
    ReadReport {
        session_id: String,
        result: source::Result<source::report::ReadReportReceipt>,
    },
    MpArticles {
        account_id: String,
        result: source::Result<Vec<source::MpArticle>>,
    },
    Article {
        article_id: String,
        result: source::Result<ChapterContent>,
    },
    Statistics(source::Result<ReadStatistics>),
    Renew(source::Result<AuthSession>),
    Cover {
        book_id: String,
        result: source::Result<Vec<u8>>,
    },
    ArticleImage {
        article_id: String,
        image_id: String,
        result: source::Result<Vec<u8>>,
    },
    LoginBegin(source::Result<QrLoginChallenge>),
    LoginPoll(source::Result<QrLoginState>),
    Underlines {
        book_id: String,
        chapter_id: String,
        result: source::Result<Vec<RemoteUnderline>>,
    },
    Thoughts {
        book_id: String,
        chapter_id: String,
        result: source::Result<Vec<RemoteReview>>,
    },
    /// Every complete chapter is emitted, even when visual progress is throttled.
    Downloaded {
        task_id: String,
        chapter: domain::Chapter,
        next_index: usize,
        html: Option<String>,
        metadata: Option<crate::book_assets::PreparedChapterMetadata>,
        images: Option<crate::book_assets::PreparedChapterImages>,
        checkpoint: tokio::sync::oneshot::Sender<source::Result<()>>,
    },
    /// Reuse a cached body without transporting it or loading the book corpus.
    /// The receiver verifies current SQL evidence before recording completion.
    CachedDownloaded {
        task_id: String,
        chapter_id: String,
        chapter_index: u32,
        next_index: usize,
        checkpoint: tokio::sync::oneshot::Sender<source::Result<()>>,
    },
    /// Clear a due chapter's stale error before starting its next attempt.
    DownloadChapterStarted {
        task_id: String,
        chapter_id: String,
        chapter_index: u32,
        ordinal: usize,
        checkpoint: tokio::sync::oneshot::Sender<source::Result<()>>,
    },
    /// Failure policy and its deadline become durable before this worker exits.
    DownloadChapterFailed {
        task_id: String,
        chapter_id: String,
        chapter_index: u32,
        ordinal: usize,
        error: SourceError,
        checkpoint: tokio::sync::oneshot::Sender<source::Result<domain::DownloadChapterRetry>>,
    },
    DownloadProgress {
        task_id: String,
        completed: usize,
        waiting: usize,
        blocked: usize,
        total: usize,
        title: String,
    },
    DownloadFinished {
        task_id: String,
    },
    DownloadIncomplete {
        task_id: String,
        completed: usize,
        blocked: usize,
        total: usize,
    },
    DownloadFailed {
        task_id: String,
        error: SourceError,
    },
    Cancelled {
        task_id: String,
    },
}

struct ActiveDownload {
    token: u64,
    request: u64,
    account_epoch: u64,
    task_id: String,
    cancelled: Arc<AtomicBool>,
    image_cache: Arc<crate::book_assets::PreparedImageCache>,
    retries: Arc<DownloadRetryControl>,
    abort: Option<AbortHandle>,
}

/// Owns one bounded chapter task and any independent metadata requests.
pub struct Jobs {
    handle: Handle,
    sender: Sender<JobEvent>,
    client: Arc<Mutex<Option<WeReadClient>>>,
    anonymous: WeReadClient,
    login: Mutex<LoginFlow>,
    account_epoch: Arc<AtomicU64>,
    account_changed: Arc<tokio::sync::Notify>,
    login_epoch: Arc<AtomicU64>,
    login_changed: Arc<tokio::sync::Notify>,
    metadata_requests: Arc<Semaphore>,
    cover_requests: Arc<Semaphore>,
    download_preparation: Arc<Semaphore>,
    download_fetches: Arc<Semaphore>,
    download_reader_pages: Arc<source::ReaderAdmission>,
    download_packets: Arc<Semaphore>,
    download_progress: Arc<Semaphore>,
    login_requests: Arc<Semaphore>,
    download_token: AtomicU64,
    download: Arc<Mutex<Option<ActiveDownload>>>,
}

impl Jobs {
    pub fn new(
        handle: Handle,
        sender: Sender<JobEvent>,
        client: Option<WeReadClient>,
    ) -> source::Result<Self> {
        Ok(Self {
            handle,
            sender,
            client: Arc::new(Mutex::new(client)),
            anonymous: WeReadClient::new(AuthSession::default())?,
            login: Mutex::new(LoginFlow::new()?),
            account_epoch: Arc::new(AtomicU64::new(0)),
            account_changed: Arc::new(tokio::sync::Notify::new()),
            login_epoch: Arc::new(AtomicU64::new(0)),
            login_changed: Arc::new(tokio::sync::Notify::new()),
            metadata_requests: Arc::new(Semaphore::new(MAX_METADATA_REQUESTS)),
            cover_requests: Arc::new(Semaphore::new(MAX_COVER_REQUESTS)),
            download_preparation: Arc::new(Semaphore::new(1)),
            download_fetches: Arc::new(Semaphore::new(MAX_DOWNLOAD_CONCURRENCY as usize)),
            download_reader_pages: Arc::new(source::ReaderAdmission::new()),
            download_packets: Arc::new(Semaphore::new(1)),
            download_progress: Arc::new(Semaphore::new(1)),
            login_requests: Arc::new(Semaphore::new(1)),
            download_token: AtomicU64::new(0),
            download: Arc::new(Mutex::new(None)),
        })
    }

    pub fn configured(&self) -> bool {
        self.session()
            .is_some_and(|session| session.is_configured())
    }

    pub fn account_epoch(&self) -> u64 {
        self.account_epoch.load(Ordering::SeqCst)
    }

    /// Check again at receipt: account changes can happen after an event is queued.
    pub fn accepts(&self, event: &JobEvent) -> bool {
        event.account_epoch == self.account_epoch()
    }

    pub fn session(&self) -> Option<AuthSession> {
        self.client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(WeReadClient::session)
    }

    pub fn capabilities(&self) -> SourceCapabilities {
        self.current_client().capabilities()
    }

    /// Opportunistic work can defer starting until a later UI tick. Admission
    /// remains atomic inside `run`; foreground requests receive a retryable
    /// result when another producer consumes the remaining capacity first.
    pub fn metadata_capacity(&self) -> usize {
        self.metadata_requests.available_permits()
    }

    pub fn cover_capacity(&self) -> usize {
        self.cover_requests
            .available_permits()
            .min(self.metadata_capacity())
    }

    #[cfg(test)]
    pub(crate) fn download_resource_capacity_for_tests(&self) -> (usize, usize) {
        (
            self.download_packets.available_permits(),
            self.download_preparation.available_permits(),
        )
    }

    #[cfg(test)]
    pub(crate) fn download_fetch_capacity_for_tests(&self) -> usize {
        self.download_fetches.available_permits()
    }

    #[cfg(test)]
    pub(crate) fn download_reader_gate_for_tests(&self) -> Arc<source::ReaderAdmission> {
        self.download_reader_pages.clone()
    }

    pub fn replace_session(&self, session: AuthSession) -> source::Result<()> {
        let client = WeReadClient::new(session)?;
        self.account_epoch.fetch_add(1, Ordering::SeqCst);
        self.account_changed.notify_waiters();
        let _ = self.cancel_active_download(None);
        self.cancel_login();
        *self
            .client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(client);
        Ok(())
    }

    pub fn clear_session(&self) {
        self.account_epoch.fetch_add(1, Ordering::SeqCst);
        self.account_changed.notify_waiters();
        let _ = self.cancel_active_download(None);
        self.cancel_login();
        *self
            .client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    pub fn sync_shelf(&self, request: u64) -> Option<JobEvent> {
        self.run(request, JobResult::Shelf, move |client| async move {
            client.shelf_snapshot().await
        })
    }

    pub fn search(&self, request: u64, query: String) -> Option<JobEvent> {
        self.run(request, JobResult::Search, move |client| async move {
            client.search(&query).await
        })
    }

    pub fn book_info(&self, request: u64, book_id: String) -> Option<JobEvent> {
        let id = book_id.clone();
        self.run(
            request,
            move |result| JobResult::BookInfo { book_id, result },
            move |client| async move { client.book_info(&id).await },
        )
    }

    pub fn catalog(&self, request: u64, book_id: String) -> Option<JobEvent> {
        let id = book_id.clone();
        self.run(
            request,
            move |result| JobResult::Catalog { book_id, result },
            move |client| async move { client.catalog(&id).await },
        )
    }

    pub fn reviews(&self, request: u64, book_id: String, latest: bool) -> Option<JobEvent> {
        let id = book_id.clone();
        self.run(
            request,
            move |result| JobResult::Reviews { book_id, result },
            move |client| async move { client.reviews(&id, latest).await },
        )
    }

    pub fn progress(&self, request: u64, book_id: String) -> Option<JobEvent> {
        let id = book_id.clone();
        self.run(
            request,
            move |result| JobResult::Progress { book_id, result },
            move |client| async move { client.progress(&id).await },
        )
    }

    pub fn upload_progress(
        &self,
        request: u64,
        book_id: String,
        position: source::ReadingPosition,
        elapsed_seconds: u32,
    ) -> Option<JobEvent> {
        self.run(
            request,
            JobResult::UploadProgress,
            move |client| async move {
                client
                    .report_progress(&book_id, &position, elapsed_seconds)
                    .await
            },
        )
    }

    pub fn report_time(
        &self,
        request: u64,
        batch: source::report::ReadReportRequest,
    ) -> Option<JobEvent> {
        let session_id = batch.session_id.clone();
        self.run(
            request,
            move |result| JobResult::ReadReport { session_id, result },
            move |client| async move { client.report_time(&batch).await },
        )
    }

    pub fn mp_articles(
        &self,
        request: u64,
        account_id: String,
        max_index: u32,
        count: u32,
    ) -> Option<JobEvent> {
        let id = account_id.clone();
        self.run(
            request,
            move |result| JobResult::MpArticles { account_id, result },
            move |client| async move { client.mp_articles(&id, max_index, count).await },
        )
    }

    pub fn article(&self, request: u64, article_id: String) -> Option<JobEvent> {
        let id = article_id.clone();
        self.run(
            request,
            move |result| JobResult::Article { article_id, result },
            move |client| async move {
                let html = client.mp_content(&id).await?;
                let plain_text = source::content::html_to_text(&html);
                if plain_text.trim().is_empty() {
                    return Err(SourceError::InvalidResponse("empty_article"));
                }
                Ok(ChapterContent {
                    html,
                    plain_text,
                    style: None,
                    format: source::ContentFormat::Xhtml,
                    content_status: domain::ContentStatus::Full,
                })
            },
        )
    }

    pub fn statistics(
        &self,
        request: u64,
        period: StatisticsPeriod,
        base_time: Option<i64>,
    ) -> Option<JobEvent> {
        self.run(request, JobResult::Statistics, move |client| async move {
            client.statistics(period, base_time).await
        })
    }

    pub fn renew(&self, request: u64) -> Option<JobEvent> {
        self.run(request, JobResult::Renew, move |client| async move {
            client.renew().await
        })
    }

    pub fn cover(&self, request: u64, book_id: String, url: String) -> Option<JobEvent> {
        self.cover_with_limit(request, book_id, url, MAX_PUBLIC_IMAGE_BYTES)
    }

    pub fn cover_with_limit(
        &self,
        request: u64,
        book_id: String,
        url: String,
        maximum_bytes: usize,
    ) -> Option<JobEvent> {
        self.run_public_image(
            request,
            move |result| JobResult::Cover { book_id, result },
            move |client| async move { client.public_image(&url, maximum_bytes).await },
        )
    }

    /// Fetch current-article media with explicit identity, independent of book IDs.
    pub fn article_image(
        &self,
        request: u64,
        article_id: String,
        image_id: String,
        url: String,
    ) -> Option<JobEvent> {
        self.run_public_image(
            request,
            move |result| JobResult::ArticleImage {
                article_id,
                image_id,
                result,
            },
            move |client| async move { client.public_image(&url, MAX_PUBLIC_IMAGE_BYTES).await },
        )
    }

    fn run_public_image<F, Fut>(
        &self,
        request: u64,
        finish: impl FnOnce(source::Result<Vec<u8>>) -> JobResult + Send + 'static,
        operation: F,
    ) -> Option<JobEvent>
    where
        F: FnOnce(WeReadClient) -> Fut + Send + 'static,
        Fut: Future<Output = source::Result<Vec<u8>>> + Send + 'static,
    {
        let permit = match self.cover_requests.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                return Some(JobEvent::new(
                    request,
                    self.account_epoch(),
                    finish(Err(SourceError::Busy)),
                ));
            }
        };
        self.run_with_permit(request, Some(permit), finish, operation)
    }

    pub fn underlines(
        &self,
        request: u64,
        book_id: String,
        chapter_id: String,
    ) -> Option<JobEvent> {
        let id = book_id.clone();
        let chapter = chapter_id.clone();
        self.run(
            request,
            move |result| JobResult::Underlines {
                book_id,
                chapter_id,
                result,
            },
            move |client| async move { client.underlines(&id, &chapter).await },
        )
    }

    pub fn thoughts(
        &self,
        request: u64,
        book_id: String,
        chapter_id: String,
        ranges: Vec<String>,
    ) -> Option<JobEvent> {
        let id = book_id.clone();
        let chapter = chapter_id.clone();
        self.run(
            request,
            move |result| JobResult::Thoughts {
                book_id,
                chapter_id,
                result,
            },
            move |client| async move { client.thoughts(&id, &chapter, &ranges).await },
        )
    }

    pub fn login_begin(&self, request: u64) -> Option<JobEvent> {
        let account_epoch = self.account_epoch();
        self.cancel_login();
        // Each attempt gets its own flow: a late old begin cannot cancel a new one.
        let flow = match LoginFlow::new() {
            Ok(flow) => flow,
            Err(error) => {
                return Some(JobEvent::new(
                    request,
                    account_epoch,
                    JobResult::LoginBegin(Err(error)),
                ));
            }
        };
        *self
            .login
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = flow.clone();
        self.run_login(
            request,
            flow,
            JobResult::LoginBegin,
            move |flow| async move { flow.begin().await },
        )
    }

    pub fn login_poll(&self, request: u64, uid: String, otp: Option<String>) -> Option<JobEvent> {
        let flow = self
            .login
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        self.run_login(
            request,
            flow,
            JobResult::LoginPoll,
            move |flow| async move { flow.poll(&uid, otp.as_deref()).await },
        )
    }

    fn run_login<T, F, Fut>(
        &self,
        request: u64,
        flow: LoginFlow,
        finish: impl FnOnce(source::Result<T>) -> JobResult + Send + 'static,
        operation: F,
    ) -> Option<JobEvent>
    where
        T: Send + 'static,
        F: FnOnce(LoginFlow) -> Fut + Send + 'static,
        Fut: Future<Output = source::Result<T>> + Send + 'static,
    {
        let account_epoch = self.account_epoch();
        let permit = match self.login_requests.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                return Some(JobEvent::new(
                    request,
                    account_epoch,
                    finish(Err(SourceError::Busy)),
                ));
            }
        };
        let live_account_epoch = self.account_epoch.clone();
        let epoch = self.login_epoch.load(Ordering::SeqCst);
        let live_epoch = self.login_epoch.clone();
        let changed = self.login_changed.clone();
        let sender = self.sender.clone();
        self.handle.spawn(async move {
            let cancelled = changed.notified();
            tokio::pin!(cancelled);
            cancelled.as_mut().enable();
            // Register before checking epochs so cancellation cannot be missed
            // between the initial check and the first network suspension.
            if epoch != live_epoch.load(Ordering::SeqCst)
                || account_epoch != live_account_epoch.load(Ordering::SeqCst)
            {
                return;
            }
            tokio::select! {
                biased;
                _ = &mut cancelled => {}
                result = operation(flow) => {
                    if epoch == live_epoch.load(Ordering::SeqCst)
                        && account_epoch == live_account_epoch.load(Ordering::SeqCst)
                    {
                        let _ = sender.send(JobEvent::new(request, account_epoch, finish(result)).retain(permit, None));
                    }
                }
            }
        });
        None
    }

    pub fn cancel_login(&self) {
        self.login_epoch.fetch_add(1, Ordering::SeqCst);
        self.login_changed.notify_waiters();
        self.login
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancel();
    }

    /// Replace the active task with the default bound. The receiver persists it.
    pub fn download(
        &self,
        request: u64,
        task: domain::DownloadTask,
        plan: DownloadPlan,
    ) -> Option<JobEvent> {
        self.download_with_concurrency(request, task, plan, DEFAULT_DOWNLOAD_CONCURRENCY)
    }

    /// Capture the preference for this start/resume. Running tasks keep their limit.
    pub fn download_with_concurrency(
        &self,
        request: u64,
        task: domain::DownloadTask,
        plan: DownloadPlan,
        concurrency: u32,
    ) -> Option<JobEvent> {
        if let Err(error) = plan.validate_for(&task) {
            return Some(JobEvent::new(
                request,
                self.account_epoch(),
                JobResult::DownloadFailed {
                    task_id: task.id,
                    error,
                },
            ));
        }
        let cancellation = self.cancel_active_download(None);
        let concurrency = concurrency.clamp(1, MAX_DOWNLOAD_CONCURRENCY);
        let diagnostics = DownloadDiagnostics::new(
            concurrency,
            plan.states
                .iter()
                .filter(|state| state.status != domain::DownloadChapterStatus::Complete)
                .count(),
            task.include_images,
        );
        let run = DownloadRun::new(diagnostics.clone());
        let token = self.download_token.fetch_add(1, Ordering::SeqCst) + 1;
        let cancelled = Arc::new(AtomicBool::new(false));
        let image_cache = Arc::new(crate::book_assets::PreparedImageCache::default());
        let retries = Arc::new(DownloadRetryControl::new(task.chapters.len()));
        let epoch = self.account_epoch.load(Ordering::SeqCst);
        let live_epoch = self.account_epoch.clone();
        let client = self.current_client();
        let sender = self.sender.clone();
        let active = self.download.clone();
        let preparation = self.download_preparation.clone();
        let fetches = self.download_fetches.clone();
        let reader_pages = self.download_reader_pages.clone();
        let packets = self.download_packets.clone();
        let progress = self.download_progress.clone();
        *active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ActiveDownload {
            token,
            request,
            account_epoch: epoch,
            task_id: task.id.clone(),
            cancelled: cancelled.clone(),
            image_cache: image_cache.clone(),
            retries: retries.clone(),
            abort: None,
        });
        let active_for_job = active.clone();
        let handle = self.handle.spawn(async move {
            let mut run = run;
            let operation = download_chapters(
                &client,
                &task,
                plan,
                token,
                request,
                epoch,
                &live_epoch,
                &cancelled,
                &image_cache,
                &preparation,
                &fetches,
                &reader_pages,
                concurrency as usize,
                &diagnostics,
                &packets,
                &progress,
                &sender,
                &active_for_job,
                &retries,
            );
            let result = {
                tokio::pin!(operation);
                let mut summary = tokio::time::interval_at(
                    tokio::time::Instant::now() + SUMMARY_INTERVAL,
                    SUMMARY_INTERVAL,
                );
                summary.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        result = &mut operation => break result,
                        _ = summary.tick() => diagnostics.report("periodic"),
                    }
                }
            };
            retries.close();
            if epoch != live_epoch.load(Ordering::SeqCst) || cancelled.load(Ordering::SeqCst) {
                let mut slot = active_for_job
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if slot.as_ref().is_some_and(|active| active.token == token) {
                    image_cache.cancel();
                    slot.take();
                }
                return;
            }
            run.finish(match &result {
                Ok(DownloadOutcome::Complete) => "completed",
                Ok(DownloadOutcome::Incomplete { .. }) => "incomplete",
                Err(_) => "failed",
            });
            // Terminal results also own a credit. There is only one active
            // download, and no mutex or uncredited payload is held while waiting.
            let terminal_credit = match packets.acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => return,
            };
            let mut slot = active_for_job
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.as_ref().is_some_and(|active| active.token == token) {
                image_cache.cancel();
                if epoch == live_epoch.load(Ordering::SeqCst) && !cancelled.load(Ordering::SeqCst) {
                    let result = match result {
                        Ok(DownloadOutcome::Complete) => {
                            JobResult::DownloadFinished { task_id: task.id }
                        }
                        Ok(DownloadOutcome::Incomplete { completed, blocked }) => {
                            JobResult::DownloadIncomplete {
                                task_id: task.id,
                                completed,
                                blocked,
                                total: task.chapters.len(),
                            }
                        }
                        Err(error) => JobResult::DownloadFailed {
                            task_id: task.id,
                            error,
                        },
                    };
                    let _ = sender
                        .send(JobEvent::new(request, epoch, result).retain(terminal_credit, None));
                }
                slot.take();
            }
        });
        let abort = handle.abort_handle();
        let mut slot = active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(active) = slot.as_mut().filter(|active| active.token == token) {
            active.abort = Some(abort);
        } else {
            abort.abort();
        }
        cancellation
    }

    pub fn cancel_download(&self, task_id: &str) -> Option<JobEvent> {
        self.cancel_active_download(Some(task_id))
    }

    /// Wake exact blocked chapters after their owner durably resets them.
    /// Requests coalesce in a fixed-size bitmap for this run and account only.
    pub fn retry_download_chapters(&self, request: u64, task_id: &str, ordinals: &[usize]) -> bool {
        let slot = self
            .download
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(active) = slot.as_ref().filter(|active| {
            active.task_id == task_id
                && active.request == request
                && active.account_epoch == self.account_epoch()
                && !active.cancelled.load(Ordering::SeqCst)
        }) else {
            return false;
        };
        active.retries.request(ordinals)
    }

    pub fn active_download(&self) -> Option<String> {
        self.download
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|active| active.task_id.clone())
    }

    pub fn cancel_all(&self) {
        self.account_epoch.fetch_add(1, Ordering::SeqCst);
        self.account_changed.notify_waiters();
        self.cancel_login();
        let _ = self.cancel_active_download(None);
    }

    fn cancel_active_download(&self, task_id: Option<&str>) -> Option<JobEvent> {
        let mut slot = self
            .download
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !slot
            .as_ref()
            .is_some_and(|active| task_id.is_none_or(|id| active.task_id == id))
        {
            return None;
        }
        if let Some(active) = slot.take() {
            active.cancelled.store(true, Ordering::SeqCst);
            active.retries.close();
            active.image_cache.cancel();
            if let Some(abort) = active.abort {
                abort.abort();
            }
            return Some(JobEvent::new(
                active.request,
                active.account_epoch,
                JobResult::Cancelled {
                    task_id: active.task_id,
                },
            ));
        }
        None
    }

    fn current_client(&self) -> WeReadClient {
        self.client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .unwrap_or_else(|| self.anonymous.clone())
    }

    fn run<T, F, Fut>(
        &self,
        request: u64,
        finish: impl FnOnce(source::Result<T>) -> JobResult + Send + 'static,
        operation: F,
    ) -> Option<JobEvent>
    where
        T: Send + 'static,
        F: FnOnce(WeReadClient) -> Fut + Send + 'static,
        Fut: Future<Output = source::Result<T>> + Send + 'static,
    {
        self.run_with_permit(request, None, finish, operation)
    }

    fn run_with_permit<T, F, Fut>(
        &self,
        request: u64,
        additional_permit: Option<OwnedSemaphorePermit>,
        finish: impl FnOnce(source::Result<T>) -> JobResult + Send + 'static,
        operation: F,
    ) -> Option<JobEvent>
    where
        T: Send + 'static,
        F: FnOnce(WeReadClient) -> Fut + Send + 'static,
        Fut: Future<Output = source::Result<T>> + Send + 'static,
    {
        let epoch = self.account_epoch.load(Ordering::SeqCst);
        let permit = match self.metadata_requests.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                return Some(JobEvent::new(
                    request,
                    epoch,
                    finish(Err(SourceError::Busy)),
                ));
            }
        };
        let live_epoch = self.account_epoch.clone();
        let client = self.current_client();
        let sender = self.sender.clone();
        let account_changed = self.account_changed.clone();
        self.handle.spawn(async move {
            // Transfer admission into the queued result. Parsing, UI processing
            // and discarding its payload all remain covered by the same credit.
            let changed = account_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if epoch != live_epoch.load(Ordering::SeqCst) { return; }
            tokio::select! {
                biased;
                _ = &mut changed => {}
                result = operation(client) => {
                    if epoch == live_epoch.load(Ordering::SeqCst) {
                        let _ = sender.send(JobEvent::new(request, epoch, finish(result)).retain(permit, additional_permit));
                    }
                }
            }
        });
        None
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

struct AbortTaskOnDrop(AbortHandle);

impl Drop for AbortTaskOnDrop {
    fn drop(&mut self) {
        // Tokio can cancel a blocking task before it starts. A running task
        // keeps its permit until its closure has actually released its data.
        self.0.abort();
    }
}

fn classify_download_preparation_error(error: anyhow::Error) -> SourceError {
    // Preserve the precise quota causes through anyhow context. Other storage,
    // I/O, and integrity failures retain their existing checkpoint category.
    if error
        .downcast_ref::<crate::article::StyleLimitError>()
        .is_some()
        || error
            .downcast_ref::<crate::article::AnchorLimitError>()
            .is_some()
        || error
            .downcast_ref::<crate::reader::LayoutLimitError>()
            .is_some()
    {
        SourceError::ResponseTooLarge
    } else {
        SourceError::CheckpointFailed
    }
}

async fn run_download_preparation<T, F>(
    permit: OwnedSemaphorePermit,
    cancelled: Arc<AtomicBool>,
    live_epoch: Arc<AtomicU64>,
    epoch: u64,
    operation: F,
) -> source::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    if cancelled.load(Ordering::SeqCst) || epoch != live_epoch.load(Ordering::SeqCst) {
        return Err(SourceError::Unsupported("download_cancelled"));
    }
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if cancelled.load(Ordering::SeqCst) || epoch != live_epoch.load(Ordering::SeqCst) {
            return Err(SourceError::Unsupported("download_cancelled"));
        }
        operation().map_err(classify_download_preparation_error)
    });
    let _abort_on_drop = AbortTaskOnDrop(task.abort_handle());
    task.await.map_err(|_| SourceError::CheckpointFailed)?
}

#[path = "jobs_download_scheduler.rs"]
mod download_scheduler;
use download_scheduler::{DownloadOutcome, DownloadRetryControl, download_chapters};

#[cfg(test)]
#[path = "jobs_queued_credit_tests.rs"]
mod queued_credit_tests;

#[cfg(test)]
#[path = "jobs_download_plan_tests.rs"]
mod download_plan_tests;

#[cfg(test)]
#[path = "jobs_preparation_error_tests.rs"]
mod preparation_error_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn login_flow(jobs: &Jobs) -> LoginFlow {
        jobs.login.lock().unwrap().clone()
    }

    async fn wait_until_released(resource: &std::sync::Weak<()>) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while resource.upgrade().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelled work retained its resources");
    }

    async fn next_job_event(receiver: &mpsc::Receiver<JobEvent>) -> JobEvent {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match receiver.try_recv() {
                    Ok(event) => return event,
                    Err(mpsc::TryRecvError::Empty) => tokio::task::yield_now().await,
                    Err(mpsc::TryRecvError::Disconnected) => panic!("job event sender closed"),
                }
            }
        })
        .await
        .expect("job did not produce its result")
    }

    struct CountedDrop(Arc<std::sync::atomic::AtomicUsize>);

    impl Drop for CountedDrop {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    const OPAQUE_COVER_BOOK_ID: &str = "article-image:fixture-article:fixture-book";
    const PUBLIC_ARTICLE_ID: &str = "fixture:article-image:article";
    const PUBLIC_ARTICLE_IMAGE_ID: &str = "fixture:image:with-colons";

    #[test]
    fn public_image_entries_keep_distinct_result_types_for_opaque_ids() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        runtime.block_on(async {
            for request in [1, 2] {
                // The actual public adapter rejects file:// before any I/O.
                if request == 1 {
                    assert!(jobs.cover(request, OPAQUE_COVER_BOOK_ID.into(), "file:///fixture.png".into()).is_none());
                } else {
                    assert!(jobs.cover_with_limit(request, OPAQUE_COVER_BOOK_ID.into(), "file:///fixture.png".into(), 1).is_none());
                }
                let event = next_job_event(&receiver).await;
                assert_eq!(event.request, request);
                assert_eq!(event.account_epoch, jobs.account_epoch());
                assert!(jobs.accepts(&event));
                assert!(matches!(
                    event.result,
                    JobResult::Cover { book_id, result: Err(SourceError::InvalidResponse("image_url")) }
                        if book_id == OPAQUE_COVER_BOOK_ID
                ));
            }
            assert!(jobs.article_image(
                3,
                PUBLIC_ARTICLE_ID.into(),
                PUBLIC_ARTICLE_IMAGE_ID.into(),
                "file:///fixture.png".into(),
            ).is_none());
            let event = next_job_event(&receiver).await;
            assert_eq!(event.request, 3);
            assert_eq!(event.account_epoch, jobs.account_epoch());
            assert!(matches!(
                event.result,
                JobResult::ArticleImage { article_id, image_id, result: Err(SourceError::InvalidResponse("image_url")) }
                    if article_id == PUBLIC_ARTICLE_ID && image_id == PUBLIC_ARTICLE_IMAGE_ID
            ));
            assert_eq!(jobs.cover_capacity(), MAX_COVER_REQUESTS - 1);
            drop(event.credits);
            assert_eq!(jobs.cover_capacity(), MAX_COVER_REQUESTS);
            assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS);
            assert!(receiver.try_recv().is_err());
        });
    }

    #[test]
    fn shared_public_image_budget_rejects_with_each_callers_original_identity() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        // Leave the two admitted operations unpolled so their shared image
        // budget is full without starting a transport operation.
        assert!(
            jobs.cover(
                10,
                OPAQUE_COVER_BOOK_ID.into(),
                "file:///fixture.png".into()
            )
            .is_none()
        );
        assert!(
            jobs.article_image(
                11,
                PUBLIC_ARTICLE_ID.into(),
                PUBLIC_ARTICLE_IMAGE_ID.into(),
                "file:///fixture.png".into(),
            )
            .is_none()
        );
        assert_eq!(jobs.cover_capacity(), 0);
        assert_eq!(
            jobs.metadata_capacity(),
            MAX_METADATA_REQUESTS - MAX_COVER_REQUESTS
        );
        let cover = jobs
            .cover(
                12,
                OPAQUE_COVER_BOOK_ID.into(),
                "file:///fixture.png".into(),
            )
            .expect("synchronous rejection");
        assert_eq!(cover.request, 12);
        assert_eq!(cover.account_epoch, jobs.account_epoch());
        assert!(matches!(
            cover.result,
            JobResult::Cover { book_id, result: Err(SourceError::Busy) }
                if book_id == OPAQUE_COVER_BOOK_ID
        ));
        let article = jobs
            .article_image(
                13,
                PUBLIC_ARTICLE_ID.into(),
                PUBLIC_ARTICLE_IMAGE_ID.into(),
                "file:///fixture.png".into(),
            )
            .expect("synchronous rejection");
        assert_eq!(article.request, 13);
        assert_eq!(article.account_epoch, jobs.account_epoch());
        assert!(matches!(
            article.result,
            JobResult::ArticleImage { article_id, image_id, result: Err(SourceError::Busy) }
                if article_id == PUBLIC_ARTICLE_ID && image_id == PUBLIC_ARTICLE_IMAGE_ID
        ));
        assert_eq!(
            jobs.metadata_capacity(),
            MAX_METADATA_REQUESTS - MAX_COVER_REQUESTS
        );
        runtime.block_on(async {
            let mut observed = std::collections::BTreeSet::new();
            for _ in 0..MAX_COVER_REQUESTS {
                let event = next_job_event(&receiver).await;
                assert_eq!(event.account_epoch, jobs.account_epoch());
                assert!(observed.insert(event.request));
                match (event.request, event.result) {
                    (
                        10,
                        JobResult::Cover {
                            book_id,
                            result: Err(SourceError::InvalidResponse("image_url")),
                        },
                    ) => {
                        assert_eq!(book_id, OPAQUE_COVER_BOOK_ID);
                    }
                    (
                        11,
                        JobResult::ArticleImage {
                            article_id,
                            image_id,
                            result: Err(SourceError::InvalidResponse("image_url")),
                        },
                    ) => {
                        assert_eq!(article_id, PUBLIC_ARTICLE_ID);
                        assert_eq!(image_id, PUBLIC_ARTICLE_IMAGE_ID);
                    }
                    other => panic!("Unexpected typed image callback: {other:?}"),
                }
            }
            assert_eq!(observed, std::collections::BTreeSet::from([10, 11]));
            assert_eq!(jobs.cover_capacity(), MAX_COVER_REQUESTS);
            assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS);
            assert!(receiver.try_recv().is_err());
        });
    }

    #[test]
    fn metadata_saturation_reports_typed_image_rejections_without_leaking_cover_permits() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        let occupied = jobs
            .metadata_requests
            .clone()
            .try_acquire_many_owned(MAX_METADATA_REQUESTS as u32)
            .unwrap();
        for request in 100..132 {
            let cover = jobs
                .cover(
                    request,
                    OPAQUE_COVER_BOOK_ID.into(),
                    "file:///fixture.png".into(),
                )
                .expect("synchronous rejection");
            assert_eq!(cover.request, request);
            assert_eq!(cover.account_epoch, jobs.account_epoch());
            assert!(matches!(
                cover.result,
                JobResult::Cover { book_id, result: Err(SourceError::Busy) }
                    if book_id == OPAQUE_COVER_BOOK_ID
            ));
            assert_eq!(jobs.cover_requests.available_permits(), MAX_COVER_REQUESTS);
            let article = jobs
                .article_image(
                    request + 100,
                    PUBLIC_ARTICLE_ID.into(),
                    PUBLIC_ARTICLE_IMAGE_ID.into(),
                    "file:///fixture.png".into(),
                )
                .expect("synchronous rejection");
            assert_eq!(article.request, request + 100);
            assert_eq!(article.account_epoch, jobs.account_epoch());
            assert!(matches!(
                article.result,
                JobResult::ArticleImage { article_id, image_id, result: Err(SourceError::Busy) }
                    if article_id == PUBLIC_ARTICLE_ID && image_id == PUBLIC_ARTICLE_IMAGE_ID
            ));
            assert_eq!(jobs.cover_requests.available_permits(), MAX_COVER_REQUESTS);
            assert_eq!(jobs.metadata_capacity(), 0);
        }
        drop(occupied);
        runtime.block_on(async {
            assert!(
                jobs.cover(
                    300,
                    OPAQUE_COVER_BOOK_ID.into(),
                    "file:///fixture.png".into()
                )
                .is_none()
            );
            assert!(
                jobs.article_image(
                    301,
                    PUBLIC_ARTICLE_ID.into(),
                    PUBLIC_ARTICLE_IMAGE_ID.into(),
                    "file:///fixture.png".into(),
                )
                .is_none()
            );
            let mut observed = std::collections::BTreeSet::new();
            for _ in 0..MAX_COVER_REQUESTS {
                let event = next_job_event(&receiver).await;
                assert!(observed.insert(event.request));
                match (event.request, event.result) {
                    (
                        300,
                        JobResult::Cover {
                            book_id,
                            result: Err(SourceError::InvalidResponse("image_url")),
                        },
                    ) => assert_eq!(book_id, OPAQUE_COVER_BOOK_ID),
                    (
                        301,
                        JobResult::ArticleImage {
                            article_id,
                            image_id,
                            result: Err(SourceError::InvalidResponse("image_url")),
                        },
                    ) => {
                        assert_eq!(article_id, PUBLIC_ARTICLE_ID);
                        assert_eq!(image_id, PUBLIC_ARTICLE_IMAGE_ID);
                    }
                    other => panic!("Unexpected recovered image callback: {other:?}"),
                }
            }
            assert_eq!(observed, std::collections::BTreeSet::from([300, 301]));
            assert_eq!(jobs.cover_capacity(), MAX_COVER_REQUESTS);
            assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS);
            assert!(receiver.try_recv().is_err());
        });
    }

    #[test]
    fn mixed_typed_image_cancellation_drops_resources_and_reuses_all_permits() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        runtime.block_on(async {
            for round in 0..32_u64 {
                let mut retained = Vec::new();
                for article in [false, true] {
                    let request = round * 10 + if article { 1 } else { 0 };
                    let resource = Arc::new(());
                    retained.push(Arc::downgrade(&resource));
                    let dropped = CountedDrop(drops.clone());
                    let (started, ready) = tokio::sync::oneshot::channel();
                    jobs.run_public_image(
                        request,
                        move |result| {
                            if article {
                                JobResult::ArticleImage {
                                    article_id: PUBLIC_ARTICLE_ID.into(),
                                    image_id: PUBLIC_ARTICLE_IMAGE_ID.into(),
                                    result,
                                }
                            } else {
                                JobResult::Cover {
                                    book_id: OPAQUE_COVER_BOOK_ID.into(),
                                    result,
                                }
                            }
                        },
                        move |_client| async move {
                            let _resource = resource;
                            let _dropped = dropped;
                            let _ = started.send(());
                            std::future::pending::<source::Result<Vec<u8>>>().await
                        },
                    );
                    ready.await.unwrap();
                }
                assert_eq!(jobs.cover_capacity(), 0);
                assert_eq!(
                    jobs.metadata_capacity(),
                    MAX_METADATA_REQUESTS - MAX_COVER_REQUESTS
                );
                jobs.cancel_all();
                for resource in &retained {
                    wait_until_released(resource).await;
                }
                assert_eq!(drops.load(Ordering::SeqCst), (round as usize + 1) * 2);
                assert_eq!(jobs.cover_capacity(), MAX_COVER_REQUESTS);
                assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS);
                assert!(receiver.try_recv().is_err());

                let payload = vec![round as u8, 17, 29];
                let cover_payload = payload.clone();
                jobs.run_public_image(
                    round * 10 + 2,
                    move |result| JobResult::Cover {
                        book_id: OPAQUE_COVER_BOOK_ID.into(),
                        result,
                    },
                    move |_client| async move { Ok(cover_payload) },
                );
                jobs.run_public_image(
                    round * 10 + 3,
                    move |result| JobResult::ArticleImage {
                        article_id: PUBLIC_ARTICLE_ID.into(),
                        image_id: PUBLIC_ARTICLE_IMAGE_ID.into(),
                        result,
                    },
                    move |_client| async move { Ok(payload) },
                );
                let mut observed = std::collections::BTreeSet::new();
                for _ in 0..MAX_COVER_REQUESTS {
                    let event = next_job_event(&receiver).await;
                    assert_eq!(event.account_epoch, jobs.account_epoch());
                    assert!(jobs.accepts(&event));
                    assert!(observed.insert(event.request));
                    match event.result {
                        JobResult::Cover {
                            book_id,
                            result: Ok(bytes),
                        } => {
                            assert_eq!(event.request, round * 10 + 2);
                            assert_eq!(book_id, OPAQUE_COVER_BOOK_ID);
                            assert_eq!(bytes, vec![round as u8, 17, 29]);
                        }
                        JobResult::ArticleImage {
                            article_id,
                            image_id,
                            result: Ok(bytes),
                        } => {
                            assert_eq!(event.request, round * 10 + 3);
                            assert_eq!(article_id, PUBLIC_ARTICLE_ID);
                            assert_eq!(image_id, PUBLIC_ARTICLE_IMAGE_ID);
                            assert_eq!(bytes, vec![round as u8, 17, 29]);
                        }
                        other => panic!("Unexpected successful typed image result: {other:?}"),
                    }
                }
                assert_eq!(observed.len(), MAX_COVER_REQUESTS);
                assert_eq!(jobs.cover_capacity(), MAX_COVER_REQUESTS);
                assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS);
                assert!(receiver.try_recv().is_err());
            }
        });
    }

    #[test]
    fn metadata_budget_rejects_without_queueing_and_recovers_after_release() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        runtime.block_on(async {
            let starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mut releases = Vec::new();
            let mut retained = Vec::new();
            for request in 0..MAX_METADATA_REQUESTS as u64 {
                let resource = Arc::new(());
                retained.push(Arc::downgrade(&resource));
                let dropped = CountedDrop(drops.clone());
                let started_count = starts.clone();
                let budget = jobs.metadata_requests.clone();
                let (started, ready) = tokio::sync::oneshot::channel();
                let (release, blocked) = tokio::sync::oneshot::channel();
                releases.push(release);
                jobs.run(
                    request,
                    move |result| {
                        assert_eq!(budget.available_permits(), 0);
                        JobResult::Search(result)
                    },
                    move |_client| async move {
                        let _resource = resource;
                        let _dropped = dropped;
                        started_count.fetch_add(1, Ordering::SeqCst);
                        let _ = started.send(());
                        let _ = blocked.await;
                        Ok(Vec::new())
                    },
                );
                ready.await.unwrap();
            }
            assert_eq!(jobs.metadata_capacity(), 0);
            assert_eq!(starts.load(Ordering::SeqCst), MAX_METADATA_REQUESTS);
            let rejected_drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            for request in 100..164 {
                let resource = Arc::new(());
                let rejected_resource = Arc::downgrade(&resource);
                let dropped = CountedDrop(rejected_drops.clone());
                let started_count = starts.clone();
                let event = jobs
                    .run(request, JobResult::Search, move |_client| async move {
                        let _resource = resource;
                        let _dropped = dropped;
                        started_count.fetch_add(1, Ordering::SeqCst);
                        std::future::pending::<source::Result<Vec<RemoteBook>>>().await
                    })
                    .expect("synchronous budget rejection was lost");
                assert_eq!(event.request, request);
                assert_eq!(event.account_epoch, jobs.account_epoch());
                assert!(matches!(
                    event.result,
                    JobResult::Search(Err(SourceError::Busy))
                ));
                assert!(rejected_resource.upgrade().is_none());
            }
            assert_eq!(rejected_drops.load(Ordering::SeqCst), 64);
            assert_eq!(starts.load(Ordering::SeqCst), MAX_METADATA_REQUESTS);
            assert_eq!(jobs.metadata_capacity(), 0);

            releases.remove(0).send(()).unwrap();
            let finished = next_job_event(&receiver).await;
            assert_eq!(finished.request, 0);
            assert!(matches!(finished.result, JobResult::Search(Ok(_))));
            assert_eq!(
                jobs.metadata_capacity(),
                0,
                "popping an event must not return its credit"
            );
            drop(finished);
            assert_eq!(jobs.metadata_capacity(), 1);
            assert_eq!(drops.load(Ordering::SeqCst), 1);
            let recovered_starts = starts.clone();
            jobs.run(200, JobResult::Search, move |_client| async move {
                recovered_starts.fetch_add(1, Ordering::SeqCst);
                Ok(Vec::new())
            });
            let recovered = next_job_event(&receiver).await;
            assert_eq!(recovered.request, 200);
            assert!(matches!(recovered.result, JobResult::Search(Ok(_))));
            drop(recovered);
            assert_eq!(starts.load(Ordering::SeqCst), MAX_METADATA_REQUESTS + 1);
            jobs.cancel_all();
            for resource in &retained {
                wait_until_released(resource).await;
            }
            assert_eq!(drops.load(Ordering::SeqCst), MAX_METADATA_REQUESTS);
            assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS);
            assert!(receiver.try_recv().is_err());
        });
    }

    #[test]
    fn cover_budget_leaves_foreground_capacity_and_recovers_without_network_access() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        for request in 0..MAX_COVER_REQUESTS as u64 {
            // The adapter rejects this scheme before constructing any request.
            jobs.cover(
                request,
                format!("fixture-{request}"),
                "file:///fixture.png".into(),
            );
        }
        assert_eq!(jobs.cover_capacity(), 0);
        assert_eq!(
            jobs.metadata_capacity(),
            MAX_METADATA_REQUESTS - MAX_COVER_REQUESTS
        );
        let rejected = jobs
            .cover(30, "rejected-cover".into(), "file:///fixture.png".into())
            .expect("synchronous rejection");
        assert_eq!(rejected.request, 30);
        assert_eq!(rejected.account_epoch, jobs.account_epoch());
        assert!(matches!(
            rejected.result,
            JobResult::Cover { book_id, result: Err(SourceError::Busy) } if book_id == "rejected-cover"
        ));
        jobs.run(32, JobResult::Search, |_client| async { Ok(Vec::new()) });
        assert_eq!(
            jobs.metadata_capacity(),
            MAX_METADATA_REQUESTS - MAX_COVER_REQUESTS - 1
        );
        runtime.block_on(async {
            let mut covers = 0;
            let mut foreground = 0;
            for _ in 0..=MAX_COVER_REQUESTS {
                let event = next_job_event(&receiver).await;
                match event.result {
                    JobResult::Cover {
                        result: Err(SourceError::InvalidResponse("image_url")),
                        ..
                    } => covers += 1,
                    JobResult::Search(Ok(_)) if event.request == 32 => foreground += 1,
                    other => panic!("unexpected budget result: {other:?}"),
                }
            }
            assert_eq!(covers, MAX_COVER_REQUESTS);
            assert_eq!(foreground, 1);
            assert_eq!(jobs.cover_capacity(), MAX_COVER_REQUESTS);
            assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS);
            jobs.cover(31, "rejected-cover".into(), "file:///fixture.png".into());
            let recovered = next_job_event(&receiver).await;
            assert_eq!(recovered.request, 31);
            assert!(matches!(
                recovered.result,
                JobResult::Cover {
                    result: Err(SourceError::InvalidResponse("image_url")),
                    ..
                }
            ));
            drop(recovered);
            assert_eq!(jobs.cover_capacity(), MAX_COVER_REQUESTS);
            assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS);
        });
    }

    #[test]
    fn total_budget_rejection_does_not_consume_a_cover_slot() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        runtime.block_on(async {
            let occupied = jobs
                .metadata_requests
                .clone()
                .acquire_many_owned(MAX_METADATA_REQUESTS as u32)
                .await
                .unwrap();
            assert_eq!(jobs.cover_capacity(), 0);
            let rejected = jobs
                .cover(40, "fixture".into(), "file:///fixture.png".into())
                .expect("synchronous rejection");
            assert_eq!(rejected.request, 40);
            assert!(matches!(
                rejected.result,
                JobResult::Cover {
                    result: Err(SourceError::Busy),
                    ..
                }
            ));
            assert_eq!(jobs.cover_requests.available_permits(), MAX_COVER_REQUESTS);
            drop(occupied);
            jobs.cover(41, "fixture".into(), "file:///fixture.png".into());
            let recovered = next_job_event(&receiver).await;
            assert_eq!(recovered.request, 41);
            assert!(matches!(
                recovered.result,
                JobResult::Cover {
                    result: Err(SourceError::InvalidResponse("image_url")),
                    ..
                }
            ));
            drop(recovered);
            assert_eq!(jobs.cover_capacity(), MAX_COVER_REQUESTS);
            assert_eq!(jobs.metadata_capacity(), MAX_METADATA_REQUESTS);
        });
    }

    #[test]
    fn repeated_login_cancellation_drops_in_flight_futures_without_waiting_for_timeout() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        runtime.block_on(async {
            for request in 0..64 {
                let resource = Arc::new(());
                let retained = Arc::downgrade(&resource);
                let (started, ready) = tokio::sync::oneshot::channel();
                jobs.run_login(
                    request,
                    login_flow(&jobs),
                    JobResult::LoginPoll,
                    move |flow| async move {
                        let _flow = flow;
                        let _resource = resource;
                        let _ = started.send(());
                        std::future::pending::<source::Result<QrLoginState>>().await
                    },
                );
                ready.await.unwrap();
                jobs.cancel_login();
                wait_until_released(&retained).await;
                assert!(receiver.try_recv().is_err());
            }
        });
    }

    #[test]
    fn login_cancelled_before_its_first_poll_does_not_start_the_operation() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        let resource = Arc::new(());
        let retained = Arc::downgrade(&resource);
        let starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let starts_in_operation = starts.clone();
        jobs.run_login(1, login_flow(&jobs), JobResult::LoginPoll, move |_flow| {
            starts_in_operation.fetch_add(1, Ordering::SeqCst);
            async move {
                let _resource = resource;
                std::future::pending::<source::Result<QrLoginState>>().await
            }
        });
        jobs.cancel_login();
        runtime.block_on(wait_until_released(&retained));
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn dropping_jobs_cancels_pending_login_work() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        runtime.block_on(async move {
            let resource = Arc::new(());
            let retained = Arc::downgrade(&resource);
            let (started, ready) = tokio::sync::oneshot::channel();
            jobs.run_login(
                1,
                login_flow(&jobs),
                JobResult::LoginPoll,
                move |flow| async move {
                    let _flow = flow;
                    let _resource = resource;
                    let _ = started.send(());
                    std::future::pending::<source::Result<QrLoginState>>().await
                },
            );
            ready.await.unwrap();
            drop(jobs);
            wait_until_released(&retained).await;
            assert!(receiver.try_recv().is_err());
        });
    }

    #[test]
    fn cancelled_running_preparation_keeps_its_permit_until_the_worker_releases_data() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, _receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        runtime.block_on(async {
            let gate = jobs.download_preparation.clone();
            let permit = gate.clone().acquire_owned().await.unwrap();
            let cancelled = Arc::new(AtomicBool::new(false));
            let epoch = jobs.account_epoch.clone();
            let (entered, running) = tokio::sync::oneshot::channel();
            let (release, blocked) = mpsc::channel();
            let resource = Arc::new(());
            let retained = Arc::downgrade(&resource);
            let first = tokio::spawn(run_download_preparation(
                permit,
                cancelled,
                epoch.clone(),
                jobs.account_epoch(),
                move || {
                    let _resource = resource;
                    let _ = entered.send(());
                    blocked.recv_timeout(Duration::from_secs(5))?;
                    Ok(())
                },
            ));
            tokio::time::timeout(Duration::from_secs(2), running)
                .await
                .unwrap()
                .unwrap();
            first.abort();
            assert!(first.await.unwrap_err().is_cancelled());
            assert_eq!(gate.available_permits(), 0);
            assert!(retained.upgrade().is_some());

            // A cancelled async waiter drops its captured payload promptly;
            // the old running decoder must still prevent a second worker.
            let waiting_resource = Arc::new(());
            let waiting_retained = Arc::downgrade(&waiting_resource);
            let waiting_gate = gate.clone();
            let (started, ready) = tokio::sync::oneshot::channel();
            let waiting = tokio::spawn(async move {
                let _resource = waiting_resource;
                let _ = started.send(());
                let _permit = waiting_gate.acquire_owned().await.unwrap();
            });
            ready.await.unwrap();
            assert!(!waiting.is_finished());
            waiting.abort();
            assert!(waiting.await.unwrap_err().is_cancelled());
            wait_until_released(&waiting_retained).await;
            assert_eq!(gate.available_permits(), 0);

            let next_gate = gate.clone();
            let (next_started, next_ready) = tokio::sync::oneshot::channel();
            let next = tokio::spawn(async move {
                let _ = next_started.send(());
                let permit = next_gate.acquire_owned().await.unwrap();
                run_download_preparation(permit, Arc::new(AtomicBool::new(false)), epoch, 0, || {
                    Ok(())
                })
                .await
            });
            next_ready.await.unwrap();
            assert!(!next.is_finished());
            assert_eq!(gate.available_permits(), 0);
            release.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(2), next)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            wait_until_released(&retained).await;
            assert_eq!(gate.available_permits(), 1);
        });
    }

    #[test]
    fn queued_preparation_rechecks_cancellation_before_parsing() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let (entered, running) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let blocker = runtime.spawn_blocking(move || {
            entered.send(()).unwrap();
            blocked.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        running.recv_timeout(Duration::from_secs(2)).unwrap();
        runtime.block_on(async {
            let gate = Arc::new(Semaphore::new(1));
            let permit = gate.clone().acquire_owned().await.unwrap();
            let cancelled = Arc::new(AtomicBool::new(false));
            let cancelled_by_test = cancelled.clone();
            let starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let starts_in_operation = starts.clone();
            let (queued, ready) = tokio::sync::oneshot::channel();
            let worker = tokio::spawn(async move {
                let _ = queued.send(());
                run_download_preparation(
                    permit,
                    cancelled,
                    Arc::new(AtomicU64::new(0)),
                    0,
                    move || {
                        starts_in_operation.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    },
                )
                .await
            });
            ready.await.unwrap();
            cancelled_by_test.store(true, Ordering::SeqCst);
            release.send(()).unwrap();
            let result = tokio::time::timeout(Duration::from_secs(2), worker)
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(
                result,
                Err(SourceError::Unsupported("download_cancelled"))
            ));
            assert_eq!(starts.load(Ordering::SeqCst), 0);
            assert_eq!(gate.available_permits(), 1);
            blocker.await.unwrap();
        });
    }

    #[test]
    fn repeated_download_cancellation_releases_waiters_on_the_shared_fetch_gate() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        runtime.block_on(async {
            let _held = jobs
                .download_fetches
                .clone()
                .acquire_many_owned(MAX_DOWNLOAD_CONCURRENCY)
                .await
                .unwrap();
            let slot = jobs.download.clone();
            for request in 0..64 {
                let id = format!("cancelled-task-{request}");
                let task = domain::DownloadTask {
                    id: id.clone(),
                    book_id: "fixture-book".into(),
                    account_key: "fixture-account".into(),
                    chapters: vec![0],
                    ..Default::default()
                };
                let headers = [domain::Chapter {
                    id: "0".into(),
                    book_id: "fixture-book".into(),
                    index: 0,
                    ..Default::default()
                }];
                let plan = DownloadPlan::from_headers(&task, &headers).unwrap();
                assert!(jobs.download(request, task, plan).is_none());
                tokio::task::yield_now().await;
                assert_eq!(jobs.active_download().as_deref(), Some(id.as_str()));
                assert!(receiver.try_recv().is_err());
                let cancelled = jobs.cancel_download(&id).expect("synchronous cancellation");
                assert!(matches!(cancelled.result, JobResult::Cancelled { .. }));
                tokio::time::timeout(Duration::from_secs(2), async {
                    while Arc::strong_count(&slot) != 2 {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .expect("cancelled download retained its async waiter");
                assert!(jobs.active_download().is_none());
                assert!(receiver.try_recv().is_err());
            }
        });
    }

    #[test]
    fn serial_cached_chapters_emit_all_checkpoints_despite_progress_throttling() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        let task = domain::DownloadTask {
            id: "fixture-task".into(),
            book_id: "fixture-book".into(),
            account_key: "fixture-account".into(),
            chapters: vec![0, 1, 2],
            next_index: 1,
            ..Default::default()
        };
        let chapters = (0..3)
            .map(|index| domain::Chapter {
                id: index.to_string(),
                book_id: "fixture-book".into(),
                index,
                title: format!("Chapter {index}"),
                cached: true,
                content_status: domain::ContentStatus::Full,
                content_account_key: task.account_key.clone(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let plan = DownloadPlan::from_headers(&task, &chapters).unwrap();
        jobs.download_with_concurrency(7, task, plan, 1);
        let mut checkpoints = Vec::new();
        let mut progress = 0;
        loop {
            let event = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(event.request, 7);
            match event.result {
                JobResult::CachedDownloaded {
                    next_index,
                    chapter_index,
                    checkpoint,
                    ..
                } => {
                    assert_eq!(chapter_index as usize + 1, next_index);
                    checkpoints.push(next_index);
                    checkpoint.send(Ok(())).unwrap();
                }
                JobResult::DownloadProgress { .. } => progress += 1,
                JobResult::DownloadFinished { .. } => break,
                other => panic!("unexpected event: {other:?}"),
            }
        }
        assert_eq!(checkpoints, vec![2, 3]);
        assert_eq!(progress, 1);
    }

    #[test]
    fn rejected_checkpoint_cannot_finish_or_advance_to_another_chapter() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        let task = domain::DownloadTask {
            id: "fixture-checkpoint".into(),
            book_id: "fixture-book".into(),
            account_key: "fixture-account".into(),
            chapters: vec![0, 1],
            ..Default::default()
        };
        let headers = vec![
            domain::Chapter {
                id: "0".into(),
                book_id: "fixture-book".into(),
                index: 0,
                cached: true,
                content_status: domain::ContentStatus::Full,
                content_account_key: task.account_key.clone(),
                ..Default::default()
            },
            domain::Chapter {
                id: "1".into(),
                book_id: "fixture-book".into(),
                index: 1,
                cached: true,
                content_status: domain::ContentStatus::Full,
                content_account_key: task.account_key.clone(),
                ..Default::default()
            },
        ];
        let plan = DownloadPlan::from_headers(&task, &headers).unwrap();
        jobs.download_with_concurrency(9, task, plan, 1);
        let event = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        let JobEvent {
            result, credits, ..
        } = event;
        let JobResult::CachedDownloaded {
            checkpoint,
            next_index,
            chapter_id,
            chapter_index,
            task_id,
        } = result
        else {
            panic!("expected one chapter packet");
        };
        assert_eq!(next_index, 1);
        assert_eq!(chapter_index, 0);
        checkpoint.send(Err(SourceError::CheckpointFailed)).unwrap();
        drop((chapter_id, task_id));
        drop(credits);
        let event = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            event.result,
            JobResult::DownloadFailed {
                error: SourceError::CheckpointFailed,
                ..
            }
        ));
    }

    #[test]
    fn stale_cached_evidence_blocks_only_its_chapter_and_allows_its_peer_to_commit() {
        fn receive_data(receiver: &mpsc::Receiver<JobEvent>) -> JobEvent {
            loop {
                let event = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
                if !matches!(&event.result, JobResult::DownloadProgress { .. }) {
                    return event;
                }
            }
        }

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        let task = domain::DownloadTask {
            id: "stale-cached-evidence".into(),
            book_id: "cached-evidence-book".into(),
            account_key: "cached-evidence-account".into(),
            chapters: vec![0, 1],
            ..Default::default()
        };
        let headers = (0..2)
            .map(|index| domain::Chapter {
                id: format!("cached-evidence-{index}"),
                book_id: task.book_id.clone(),
                index,
                cached: true,
                content_status: domain::ContentStatus::Full,
                content_account_key: task.account_key.clone(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let plan = DownloadPlan::from_headers(&task, &headers).unwrap();
        assert!(jobs.download_with_concurrency(10, task, plan, 2).is_none());
        let JobEvent {
            result, credits, ..
        } = receive_data(&receiver);
        let JobResult::CachedDownloaded {
            chapter_index: rejected,
            checkpoint,
            ..
        } = result
        else {
            panic!("Expected the initial cached evidence check");
        };
        checkpoint.send(Err(SourceError::ContentChanged)).unwrap();
        drop(credits);

        let JobEvent {
            result, credits, ..
        } = receive_data(&receiver);
        let JobResult::DownloadChapterFailed {
            chapter_index,
            ordinal,
            error: SourceError::ContentChanged,
            checkpoint,
            ..
        } = result
        else {
            panic!("Stale cached evidence did not reach independent failure handling");
        };
        assert_eq!(chapter_index, rejected);
        assert_eq!(ordinal, rejected as usize);
        assert!(credits.as_ref().unwrap()._second.is_none());
        checkpoint
            .send(Ok(domain::DownloadChapterRetry::Blocked))
            .unwrap();
        drop(credits);

        let JobEvent {
            result, credits, ..
        } = receive_data(&receiver);
        let JobResult::CachedDownloaded {
            chapter_index,
            next_index,
            checkpoint,
            ..
        } = result
        else {
            panic!("The other cached chapter was discarded after a local rejection");
        };
        assert_ne!(chapter_index, rejected);
        assert_eq!(next_index, chapter_index as usize + 1);
        checkpoint.send(Ok(())).unwrap();
        drop(credits);
        let terminal = receive_data(&receiver);
        assert!(matches!(
            &terminal.result,
            JobResult::DownloadIncomplete {
                completed: 1,
                blocked: 1,
                total: 2,
                ..
            }
        ));
        drop(terminal);
        assert_eq!(jobs.download_resource_capacity_for_tests(), (1, 1));
        assert_eq!(
            jobs.download_fetch_capacity_for_tests(),
            MAX_DOWNLOAD_CONCURRENCY as usize
        );
        assert!(jobs.active_download().is_none());
    }

    #[test]
    fn a_retry_start_content_conflict_does_not_stop_the_next_chapter() {
        fn receive_data(receiver: &mpsc::Receiver<JobEvent>) -> JobEvent {
            loop {
                let event = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
                if !matches!(&event.result, JobResult::DownloadProgress { .. }) {
                    return event;
                }
            }
        }

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        let task = domain::DownloadTask {
            id: "retry-start-conflict".into(),
            book_id: "retry-start-book".into(),
            account_key: "retry-start-account".into(),
            chapters: vec![0, 1],
            ..Default::default()
        };
        let headers = (0..2)
            .map(|index| domain::Chapter {
                id: format!("retry-start-{index}"),
                book_id: task.book_id.clone(),
                index,
                cached: true,
                content_status: domain::ContentStatus::Full,
                content_account_key: task.account_key.clone(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let states = headers
            .iter()
            .enumerate()
            .map(|(ordinal, chapter)| domain::DownloadChapterState {
                chapter_uid: chapter.id.clone(),
                chapter_index: chapter.index,
                ordinal,
                status: if ordinal == 0 {
                    domain::DownloadChapterStatus::RetryWait
                } else {
                    domain::DownloadChapterStatus::Pending
                },
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let plan = DownloadPlan::from_headers_with_states(&task, &headers, &states).unwrap();
        assert!(jobs.download_with_concurrency(11, task, plan, 1).is_none());
        let JobEvent {
            result, credits, ..
        } = receive_data(&receiver);
        let JobResult::DownloadChapterStarted {
            chapter_index: 0,
            checkpoint,
            ..
        } = result
        else {
            panic!("A due retry did not ask for durable admission");
        };
        checkpoint.send(Err(SourceError::ContentChanged)).unwrap();
        drop(credits);
        let JobEvent {
            result, credits, ..
        } = receive_data(&receiver);
        let JobResult::DownloadChapterFailed {
            chapter_index: 0,
            error: SourceError::ContentChanged,
            checkpoint,
            ..
        } = result
        else {
            panic!("A retry-start conflict incorrectly stopped the run");
        };
        checkpoint
            .send(Ok(domain::DownloadChapterRetry::Blocked))
            .unwrap();
        drop(credits);
        let JobEvent {
            result, credits, ..
        } = receive_data(&receiver);
        let JobResult::CachedDownloaded {
            chapter_index: 1,
            checkpoint,
            ..
        } = result
        else {
            panic!("The unaffected chapter did not retain its completion path");
        };
        checkpoint.send(Ok(())).unwrap();
        drop(credits);
        let terminal = receive_data(&receiver);
        assert!(matches!(
            &terminal.result,
            JobResult::DownloadIncomplete {
                completed: 1,
                blocked: 1,
                total: 2,
                ..
            }
        ));
        drop(terminal);
        assert_eq!(jobs.download_resource_capacity_for_tests(), (1, 1));
        assert_eq!(
            jobs.download_fetch_capacity_for_tests(),
            MAX_DOWNLOAD_CONCURRENCY as usize
        );
    }

    #[test]
    fn queued_old_account_results_cannot_store_shelf_or_publications() {
        fn receive(jobs: &Jobs, store: &crate::store::Store, event: JobEvent) {
            // This is the same receipt guard used before App::handle_job mutates state.
            if !jobs.accepts(&event) {
                return;
            }
            match event.result {
                JobResult::Shelf(Ok(snapshot)) => {
                    let books = snapshot
                        .books
                        .into_iter()
                        .map(|book| domain::Book {
                            id: book.id,
                            title: book.title,
                            on_shelf: true,
                            source: "weread".into(),
                            ..Default::default()
                        })
                        .collect::<Vec<_>>();
                    store.replace_shelf(&books).unwrap();
                }
                JobResult::MpArticles {
                    account_id,
                    result: Ok(articles),
                } => {
                    store
                        .set_setting(&format!("fixture.mp:{account_id}"), &articles)
                        .unwrap();
                }
                _ => {}
            }
        }
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender.clone(), None).unwrap();
        let old_epoch = jobs.account_epoch();
        sender
            .send(JobEvent {
                credits: None,
                request: 1,
                account_epoch: old_epoch,
                result: JobResult::Shelf(Ok(ShelfSnapshot {
                    books: vec![RemoteBook {
                        id: "old-account-book".into(),
                        title: "Old account fixture".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                })),
            })
            .unwrap();
        sender
            .send(JobEvent {
                credits: None,
                request: 2,
                account_epoch: old_epoch,
                result: JobResult::MpArticles {
                    account_id: "old-account-publication".into(),
                    result: Ok(vec![source::MpArticle {
                        id: "old-account-article".into(),
                        ..Default::default()
                    }]),
                },
            })
            .unwrap();
        jobs.replace_session(AuthSession {
            api_key: "fixture-account-b".into(),
            ..Default::default()
        })
        .unwrap();
        let store = crate::store::Store::open(":memory:").unwrap();
        for event in receiver.try_iter() {
            receive(&jobs, &store, event);
        }
        assert!(store.shelf().unwrap().is_empty());
        assert!(
            store
                .setting::<Vec<source::MpArticle>>("fixture.mp:old-account-publication")
                .unwrap()
                .is_none()
        );
        receive(
            &jobs,
            &store,
            JobEvent {
                credits: None,
                request: 3,
                account_epoch: jobs.account_epoch(),
                result: JobResult::Shelf(Ok(ShelfSnapshot {
                    books: vec![RemoteBook {
                        id: "new-account-book".into(),
                        title: "New account fixture".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                })),
            },
        );
        assert_eq!(store.shelf().unwrap()[0].id, "new-account-book");
    }

    #[test]
    fn cancellation_keeps_the_downloads_original_account_epoch() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        let old_epoch = jobs.account_epoch();
        let task = domain::DownloadTask {
            id: "old-account-task".into(),
            book_id: "fixture-book".into(),
            account_key: "fixture-account".into(),
            ..Default::default()
        };
        let plan = DownloadPlan::from_headers(&task, &[]).unwrap();
        jobs.download(13, task, plan);
        let image_cache = jobs
            .download
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .image_cache
            .clone();
        let event = jobs
            .cancel_download("old-account-task")
            .expect("synchronous cancellation");
        jobs.replace_session(AuthSession {
            api_key: "fixture-new-account".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(event.account_epoch, old_epoch);
        assert_ne!(event.account_epoch, jobs.account_epoch());
        assert!(!jobs.accepts(&event));
        assert!(matches!(event.result, JobResult::Cancelled { .. }));
        assert!(
            receiver.try_recv().is_err(),
            "cancellation controls must not grow the mailbox"
        );
        assert!(
            crate::book_assets::prepare_downloaded_with_cache(
                source::images::DownloadedChapterImages::default(),
                &image_cache,
            )
            .is_err()
        );
    }

    #[test]
    fn replacing_and_cancelling_tasks_closes_only_their_own_image_cache() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, _receiver) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, None).unwrap();
        let task = domain::DownloadTask {
            id: "first-task".into(),
            book_id: "fixture-book".into(),
            account_key: "fixture-account".into(),
            ..Default::default()
        };
        let plan = DownloadPlan::from_headers(&task, &[]).unwrap();
        jobs.download(21, task, plan);
        let first = jobs
            .download
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .image_cache
            .clone();
        let task = domain::DownloadTask {
            id: "second-task".into(),
            book_id: "fixture-book".into(),
            account_key: "fixture-account".into(),
            ..Default::default()
        };
        let plan = DownloadPlan::from_headers(&task, &[]).unwrap();
        jobs.download(22, task, plan);
        let second = jobs
            .download
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .image_cache
            .clone();
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(
            crate::book_assets::prepare_downloaded_with_cache(
                source::images::DownloadedChapterImages::default(),
                &first,
            )
            .is_err()
        );
        assert!(
            crate::book_assets::prepare_downloaded_with_cache(
                source::images::DownloadedChapterImages::default(),
                &second,
            )
            .is_ok()
        );
        jobs.cancel_download("second-task");
        assert!(
            crate::book_assets::prepare_downloaded_with_cache(
                source::images::DownloadedChapterImages::default(),
                &second,
            )
            .is_err()
        );
    }
}
