//! Completion-order chapter workers with acknowledged, independent recovery.

use super::*;
use source::ChapterReadHints;
use tokio::task::JoinSet;

pub(super) enum DownloadOutcome {
    Complete,
    Incomplete { completed: usize, blocked: usize },
}

/// A task-local, coalescing control surface, never a queue of chapter payloads.
pub(super) struct DownloadRetryControl {
    requested: Mutex<RetryRequests>,
    changed: tokio::sync::Notify,
}

struct RetryRequests {
    bits: Box<[u64]>,
    chapters: usize,
    closed: bool,
}

impl DownloadRetryControl {
    pub(super) fn new(chapters: usize) -> Self {
        // DownloadPlan admission has already checked the chapter bound.
        Self {
            requested: Mutex::new(RetryRequests {
                bits: vec![0; chapters.div_ceil(u64::BITS as usize)].into_boxed_slice(),
                chapters,
                closed: false,
            }),
            changed: tokio::sync::Notify::new(),
        }
    }

    pub(super) fn request(&self, ordinals: &[usize]) -> bool {
        let mut requested = self
            .requested
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if requested.closed
            || ordinals.is_empty()
            || ordinals.len() > MAX_DOWNLOAD_CHAPTERS
            || ordinals
                .iter()
                .any(|ordinal| *ordinal >= requested.chapters)
        {
            return false;
        }
        for ordinal in ordinals {
            requested.bits[*ordinal / u64::BITS as usize] |= 1 << (*ordinal % u64::BITS as usize);
        }
        drop(requested);
        self.changed.notify_one();
        true
    }

    fn apply(&self, states: &mut [DownloadPlanState], admitted: &[bool]) {
        let mut requested = self
            .requested
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (ordinal, state) in states.iter_mut().enumerate() {
            let bit = 1 << (ordinal % u64::BITS as usize);
            let word = &mut requested.bits[ordinal / u64::BITS as usize];
            if *word & bit == 0 {
                continue;
            }
            if admitted[ordinal] {
                // A durable failure ACK can precede this worker's JoinSet
                // settlement. Retain one bit until that result is observed;
                // never interrupt the attempt or allocate a second worker.
                continue;
            }
            *word &= !bit;
            if state.status == domain::DownloadChapterStatus::Blocked {
                // RetryWait with a due deadline selects the existing started
                // ACK path, which revalidates the owner's durable Pending state.
                state.status = domain::DownloadChapterStatus::RetryWait;
                state.retry_not_before = 0;
            }
        }
    }

    fn close_if_idle(&self) -> bool {
        let mut requested = self
            .requested
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if requested.bits.iter().any(|word| *word != 0) {
            return false;
        }
        requested.closed = true;
        true
    }

    pub(super) fn close(&self) {
        let mut requested = self
            .requested
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        requested.closed = true;
        requested.bits.fill(0);
        drop(requested);
        self.changed.notify_one();
    }
}

struct ChapterWork {
    ordinal: usize,
    index: u32,
    entry: DownloadPlanEntry,
    retrying: bool,
}

enum ChapterDisposition {
    Complete,
    RetryAt(i64),
    Blocked,
    Stopped,
}

#[derive(Default)]
struct ChapterPreparation {
    resource_catalog: Option<Vec<RemoteChapter>>,
}

struct DownloadWorkers {
    client: WeReadClient,
    task_id: String,
    book_id: String,
    account_key: String,
    include_images: bool,
    token: u64,
    request: u64,
    epoch: u64,
    live_epoch: Arc<AtomicU64>,
    cancelled: Arc<AtomicBool>,
    stopped: AtomicBool,
    image_cache: Arc<crate::book_assets::PreparedImageCache>,
    preparation: Arc<Semaphore>,
    fetches: Arc<Semaphore>,
    packets: Arc<Semaphore>,
    diagnostics: Arc<DownloadDiagnostics>,
    sender: Sender<JobEvent>,
    active: Arc<Mutex<Option<ActiveDownload>>>,
    hints: Mutex<ChapterReadHints>,
    // One chapter owns images, preparation and durable ACK at a time. This
    // lock has no body-fetch ownership, so an exclusive large body can wait
    // without preventing already fetched chapters from releasing their bytes.
    stage: tokio::sync::Mutex<ChapterPreparation>,
}

impl DownloadWorkers {
    fn check_current(&self) -> source::Result<()> {
        if self.cancelled.load(Ordering::SeqCst)
            || self.epoch != self.live_epoch.load(Ordering::SeqCst)
        {
            return Err(SourceError::Unsupported("download_cancelled"));
        }
        Ok(())
    }

    fn send(
        &self,
        result: JobResult,
        packet: OwnedSemaphorePermit,
        body: Option<OwnedSemaphorePermit>,
    ) -> source::Result<()> {
        let slot = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.check_current()?;
        if !slot
            .as_ref()
            .is_some_and(|active| active.token == self.token)
        {
            return Err(SourceError::Unsupported("download_cancelled"));
        }
        self.sender
            .send(JobEvent::new(self.request, self.epoch, result).retain(packet, body))
            .map_err(|_| SourceError::Unsupported("event_receiver_closed"))
    }

    async fn packet(&self) -> source::Result<OwnedSemaphorePermit> {
        let _wait = self.diagnostics.stage(Stage::Ack);
        let permit = self
            .packets
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| SourceError::Unsupported("download_cancelled"))?;
        self.check_current()?;
        Ok(permit)
    }
}

struct FetchedChapter {
    content: ChapterContent,
    credit: OwnedSemaphorePermit,
    exclusive: Option<ExclusiveBodyMode>,
}

struct ExclusiveBodyMode(Arc<DownloadDiagnostics>);

impl Drop for ExclusiveBodyMode {
    fn drop(&mut self) {
        self.0.resume_parallel();
    }
}

async fn fetch_chapter(
    workers: &DownloadWorkers,
    chapter_id: &str,
) -> source::Result<FetchedChapter> {
    let mut credit = workers
        .fetches
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| SourceError::Unsupported("download_cancelled"))?;
    workers.check_current()?;
    let mut hints = workers
        .hints
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let _wait = workers.diagnostics.stage(Stage::TextWait);
    let mut result = {
        let _read = workers.diagnostics.chapter_read();
        workers
            .client
            .chapter_prefetch_with_hints(&workers.book_id, chapter_id, &mut hints)
            .await
    };
    let mut exclusive = None;
    if matches!(result, Err(SourceError::ResponseTooLarge)) {
        // Never upgrade a held permit. The fair whole-pool waiter owns no
        // packet or preparation gate; peers can commit and drop their bodies.
        // Neither successful peers nor their HTTP requests are discarded.
        drop(credit);
        credit = workers
            .fetches
            .clone()
            .acquire_many_owned(MAX_DOWNLOAD_CONCURRENCY)
            .await
            .map_err(|_| SourceError::Unsupported("download_cancelled"))?;
        workers.check_current()?;
        workers.diagnostics.serial_fallback();
        exclusive = Some(ExclusiveBodyMode(workers.diagnostics.clone()));
        result = {
            let _read = workers.diagnostics.chapter_read();
            workers
                .client
                .chapter_with_hints(&workers.book_id, chapter_id, &mut hints)
                .await
        };
    }
    workers
        .hints
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .merge(hints);
    let content = match result {
        Ok(content) => content,
        Err(SourceError::UnlockRequired) => ChapterContent {
            html: String::new(),
            plain_text: String::new(),
            style: None,
            format: source::ContentFormat::Text,
            content_status: domain::ContentStatus::Locked,
        },
        Err(error) => return Err(error),
    };
    Ok(FetchedChapter {
        content,
        credit,
        exclusive,
    })
}

struct PreparedPacket {
    chapter: domain::Chapter,
    html: Option<String>,
    metadata: Option<crate::book_assets::PreparedChapterMetadata>,
    images: Option<crate::book_assets::PreparedChapterImages>,
    packet_credit: OwnedSemaphorePermit,
    fetch_credit: OwnedSemaphorePermit,
}

async fn prepare_chapter(
    workers: &DownloadWorkers,
    work: &ChapterWork,
    content: ChapterContent,
    fetch_credit: OwnedSemaphorePermit,
    packet_credit: OwnedSemaphorePermit,
    stage: &mut ChapterPreparation,
) -> source::Result<PreparedPacket> {
    let mut chapter = domain::Chapter {
        id: work.entry.id.clone(),
        book_id: workers.book_id.clone(),
        index: work.index,
        title: work.entry.title.clone(),
        cached: false,
        body: None,
        content_status: content.content_status,
        content_account_key: workers.account_key.clone(),
    };
    if content.plain_text.trim().is_empty() {
        if content.content_status != domain::ContentStatus::Locked {
            return Err(SourceError::InvalidResponse("empty_chapter"));
        }
        return Ok(PreparedPacket {
            chapter,
            html: None,
            metadata: None,
            images: None,
            packet_credit,
            fetch_credit,
        });
    }
    let permit = {
        let _wait = workers.diagnostics.stage(Stage::Preparation);
        workers
            .preparation
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| SourceError::Unsupported("download_cancelled"))?
    };
    workers.check_current()?;
    let downloaded = if workers.include_images
        && content.content_status == domain::ContentStatus::Full
    {
        let _images = workers.diagnostics.stage(Stage::Images);
        if stage.resource_catalog.is_none() {
            stage.resource_catalog = Some(workers.client.catalog(&workers.book_id).await?);
        }
        let resources = stage
            .resource_catalog
            .as_ref()
            .and_then(|catalog| catalog.iter().find(|remote| remote.uid == work.entry.id))
            .map(|chapter| chapter.resource_url.as_str())
            .filter(|url| !url.is_empty());
        let plan =
            source::images::make_plan(&workers.book_id, &work.entry.id, &content.html, resources)?;
        Some(source::images::acquire(&workers.client, &plan).await?)
    } else {
        None
    };
    let image_cache = workers.image_cache.clone();
    let _wait = workers.diagnostics.stage(Stage::Preparation);
    let (html, canonical, metadata, images, fetch_credit, packet_credit) =
        run_download_preparation(
            permit,
            workers.cancelled.clone(),
            workers.live_epoch.clone(),
            workers.epoch,
            move || {
                // Both queue and body admission follow the actual decoder.
                // Aborting its async owner cannot create another large lane.
                let metadata = crate::book_assets::prepare_chapter_metadata(
                    &content.html,
                    &content.plain_text,
                )?;
                let images = downloaded
                    .map(|downloaded| {
                        crate::book_assets::prepare_downloaded_with_cache(downloaded, &image_cache)
                    })
                    .transpose()?;
                Ok((
                    content.html,
                    content.plain_text,
                    metadata,
                    images,
                    fetch_credit,
                    packet_credit,
                ))
            },
        )
        .await?;
    chapter.body = Some(canonical);
    chapter.cached = chapter.content_status == domain::ContentStatus::Full;
    Ok(PreparedPacket {
        chapter,
        html: Some(html),
        metadata: Some(metadata),
        images,
        packet_credit,
        fetch_credit,
    })
}

fn chapter_local_checkpoint_failure(error: &SourceError) -> bool {
    matches!(
        error,
        SourceError::ContentChanged | SourceError::ContentUnverified | SourceError::UnlockRequired
    )
}

async fn acknowledge_failure(
    workers: &DownloadWorkers,
    work: &ChapterWork,
    error: SourceError,
) -> source::Result<ChapterDisposition> {
    // No chapter body, image payload or fetch permit reaches the cooldown.
    let packet = workers.packet().await?;
    let (checkpoint, committed) = tokio::sync::oneshot::channel();
    workers.send(
        JobResult::DownloadChapterFailed {
            task_id: workers.task_id.clone(),
            chapter_id: work.entry.id.clone(),
            chapter_index: work.index,
            ordinal: work.ordinal,
            error,
            checkpoint,
        },
        packet,
        None,
    )?;
    let _wait = workers.diagnostics.stage(Stage::Ack);
    let disposition = committed
        .await
        .map_err(|_| SourceError::Unsupported("checkpoint_cancelled"))??;
    Ok(match disposition {
        // A malformed or already due ACK must not create an immediate retry
        // loop. The owner persists normal future deadlines and a finite budget.
        domain::DownloadChapterRetry::RetryAt(deadline) if deadline > 0 => {
            ChapterDisposition::RetryAt(deadline.max(unix_seconds().saturating_add(1)))
        }
        domain::DownloadChapterRetry::RetryAt(_) | domain::DownloadChapterRetry::Blocked => {
            ChapterDisposition::Blocked
        }
    })
}

async fn process_chapter(
    workers: &DownloadWorkers,
    work: &ChapterWork,
    fetched: Option<source::Result<FetchedChapter>>,
    stage: &mut ChapterPreparation,
) -> source::Result<ChapterDisposition> {
    let (checkpoint, committed) = tokio::sync::oneshot::channel();
    let content_status;
    let _exclusive;
    match fetched {
        None => {
            let packet = workers.packet().await?;
            workers.send(
                JobResult::CachedDownloaded {
                    task_id: workers.task_id.clone(),
                    chapter_id: work.entry.id.clone(),
                    chapter_index: work.index,
                    next_index: work.ordinal + 1,
                    checkpoint,
                },
                packet,
                None,
            )?;
            content_status = domain::ContentStatus::Full;
            _exclusive = None;
        }
        Some(Err(error)) => return acknowledge_failure(workers, work, error).await,
        Some(Ok(fetched)) => {
            let FetchedChapter {
                content,
                credit,
                exclusive,
            } = fetched;
            _exclusive = exclusive;
            let packet = workers.packet().await?;
            let prepared =
                match prepare_chapter(workers, work, content, credit, packet, stage).await {
                    Ok(prepared) => prepared,
                    Err(error) => return acknowledge_failure(workers, work, error).await,
                };
            content_status = prepared.chapter.content_status;
            workers.send(
                JobResult::Downloaded {
                    task_id: workers.task_id.clone(),
                    chapter: prepared.chapter,
                    next_index: work.ordinal + 1,
                    html: prepared.html,
                    metadata: prepared.metadata,
                    images: prepared.images,
                    checkpoint,
                },
                prepared.packet_credit,
                Some(prepared.fetch_credit),
            )?;
        }
    }
    let acknowledged = {
        let _wait = workers.diagnostics.stage(Stage::Ack);
        committed
            .await
            .map_err(|_| SourceError::Unsupported("checkpoint_cancelled"))?
    };
    if let Err(error) = acknowledged {
        if chapter_local_checkpoint_failure(&error) {
            return acknowledge_failure(workers, work, error).await;
        }
        return Err(error);
    }
    match content_status {
        domain::ContentStatus::Full => {
            workers.diagnostics.checkpoint();
            Ok(ChapterDisposition::Complete)
        }
        domain::ContentStatus::Preview | domain::ContentStatus::Locked => {
            acknowledge_failure(workers, work, SourceError::UnlockRequired).await
        }
        domain::ContentStatus::Unknown => {
            acknowledge_failure(workers, work, SourceError::ContentUnverified).await
        }
    }
}

async fn run_chapter(
    workers: Arc<DownloadWorkers>,
    work: ChapterWork,
) -> source::Result<(usize, ChapterDisposition)> {
    if work.retrying {
        let _stage = workers.stage.lock().await;
        if workers.stopped.load(Ordering::SeqCst) {
            return Ok((work.ordinal, ChapterDisposition::Stopped));
        }
        let result = start_retry_attempt(&workers, &work).await;
        if let Err(error) = result {
            if chapter_local_checkpoint_failure(&error) {
                let outcome = acknowledge_failure(&workers, &work, error).await;
                if outcome.is_err() {
                    workers.stopped.store(true, Ordering::SeqCst);
                }
                return outcome.map(|disposition| (work.ordinal, disposition));
            }
            workers.stopped.store(true, Ordering::SeqCst);
            return Err(error);
        }
    }
    let fetched = if work.entry.reusable(workers.include_images) {
        None
    } else {
        Some(fetch_chapter(&workers, &work.entry.id).await)
    };
    let mut stage = workers.stage.lock().await;
    if workers.stopped.load(Ordering::SeqCst) {
        return Ok((work.ordinal, ChapterDisposition::Stopped));
    }
    workers.check_current()?;
    let outcome = process_chapter(&workers, &work, fetched, &mut stage).await;
    if outcome.is_err() {
        // A failed storage ACK fences this run before another worker obtains
        // the commit lane. Per-chapter failures return a durable disposition.
        workers.stopped.store(true, Ordering::SeqCst);
    }
    outcome.map(|disposition| (work.ordinal, disposition))
}

async fn start_retry_attempt(workers: &DownloadWorkers, work: &ChapterWork) -> source::Result<()> {
    let packet = workers.packet().await?;
    let (checkpoint, committed) = tokio::sync::oneshot::channel();
    workers.send(
        JobResult::DownloadChapterStarted {
            task_id: workers.task_id.clone(),
            chapter_id: work.entry.id.clone(),
            chapter_index: work.index,
            ordinal: work.ordinal,
            checkpoint,
        },
        packet,
        None,
    )?;
    let _wait = workers.diagnostics.stage(Stage::Ack);
    committed
        .await
        .map_err(|_| SourceError::Unsupported("checkpoint_cancelled"))?
}

fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

fn counts(states: &[DownloadPlanState]) -> (usize, usize, usize) {
    let mut completed = 0;
    let mut waiting = 0;
    let mut blocked = 0;
    for state in states {
        match state.status {
            domain::DownloadChapterStatus::Complete => completed += 1,
            domain::DownloadChapterStatus::RetryWait => waiting += 1,
            domain::DownloadChapterStatus::Blocked => blocked += 1,
            domain::DownloadChapterStatus::Pending => {}
        }
    }
    (completed, waiting, blocked)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn download_chapters(
    client: &WeReadClient,
    task: &domain::DownloadTask,
    mut plan: DownloadPlan,
    token: u64,
    request: u64,
    epoch: u64,
    live_epoch: &Arc<AtomicU64>,
    cancelled: &Arc<AtomicBool>,
    image_cache: &Arc<crate::book_assets::PreparedImageCache>,
    preparation: &Arc<Semaphore>,
    fetches: &Arc<Semaphore>,
    reader_pages: &Arc<source::ReaderAdmission>,
    concurrency: usize,
    diagnostics: &Arc<DownloadDiagnostics>,
    packets: &Arc<Semaphore>,
    progress: &Arc<Semaphore>,
    sender: &Sender<JobEvent>,
    active: &Arc<Mutex<Option<ActiveDownload>>>,
    retries: &Arc<DownloadRetryControl>,
) -> source::Result<DownloadOutcome> {
    let mut hints = ChapterReadHints::for_book(client, &task.book_id);
    hints.set_download_diagnostics(diagnostics.clone());
    hints.set_download_reader_gate(reader_pages.clone());
    let workers = Arc::new(DownloadWorkers {
        client: client.clone(),
        task_id: task.id.clone(),
        book_id: task.book_id.clone(),
        account_key: task.account_key.clone(),
        include_images: task.include_images,
        token,
        request,
        epoch,
        live_epoch: live_epoch.clone(),
        cancelled: cancelled.clone(),
        stopped: AtomicBool::new(false),
        image_cache: image_cache.clone(),
        preparation: preparation.clone(),
        fetches: fetches.clone(),
        packets: packets.clone(),
        diagnostics: diagnostics.clone(),
        sender: sender.clone(),
        active: active.clone(),
        hints: Mutex::new(hints),
        stage: tokio::sync::Mutex::new(ChapterPreparation::default()),
    });
    let mut running = JoinSet::new();
    let mut admitted = vec![false; task.chapters.len()];
    let mut last_progress: Option<Instant> = None;
    loop {
        workers.check_current()?;
        let changed = retries.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        retries.apply(&mut plan.states, &admitted);
        if !workers.stopped.load(Ordering::SeqCst) {
            let now = unix_seconds();
            for (ordinal, state) in plan.states.iter_mut().enumerate() {
                if running.len() >= concurrency {
                    break;
                }
                if admitted[ordinal]
                    || state.status == domain::DownloadChapterStatus::Complete
                    || state.status == domain::DownloadChapterStatus::Blocked
                    || (state.status == domain::DownloadChapterStatus::RetryWait
                        && state.retry_not_before > now)
                {
                    continue;
                }
                let index = task.chapters[ordinal];
                let entry = plan
                    .chapters
                    .get(&index)
                    .ok_or(SourceError::InvalidResponse("missing_chapter"))?
                    .clone();
                let retrying = state.status == domain::DownloadChapterStatus::RetryWait;
                admitted[ordinal] = true;
                state.status = domain::DownloadChapterStatus::Pending;
                running.spawn(run_chapter(
                    workers.clone(),
                    ChapterWork {
                        ordinal,
                        index,
                        entry,
                        retrying,
                    },
                ));
            }
        }

        if running.is_empty() {
            let (completed, waiting, blocked) = counts(&plan.states);
            if completed == task.chapters.len() {
                if !retries.close_if_idle() {
                    continue;
                }
                return Ok(DownloadOutcome::Complete);
            }
            if waiting == 0 {
                if !retries.close_if_idle() {
                    continue;
                }
                return Ok(DownloadOutcome::Incomplete { completed, blocked });
            }
            let deadline = plan
                .states
                .iter()
                .filter(|state| state.status == domain::DownloadChapterStatus::RetryWait)
                .map(|state| state.retry_not_before)
                .min()
                .ok_or(SourceError::InvalidResponse("download_retry"))?;
            // This timer belongs to the active run. Its cancellation drops the
            // entire scheduler, and it owns no worker, body or packet permits.
            let wait = deadline.saturating_sub(unix_seconds()).max(0) as u64;
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(wait)) => {}
                _ = &mut changed => {}
            }
            continue;
        }

        let next_retry = if running.len() < concurrency {
            plan.states
                .iter()
                .filter(|state| state.status == domain::DownloadChapterStatus::RetryWait)
                .map(|state| state.retry_not_before)
                .min()
        } else {
            None
        };
        let next = if let Some(deadline) = next_retry {
            let wait = deadline.saturating_sub(unix_seconds()).max(0) as u64;
            tokio::select! {
                result = running.join_next() => result,
                _ = tokio::time::sleep(Duration::from_secs(wait)) => continue,
                _ = &mut changed => continue,
            }
        } else {
            tokio::select! {
                result = running.join_next() => result,
                _ = &mut changed => continue,
            }
        };
        let joined = next
            .ok_or(SourceError::InvalidResponse("download_worker"))?
            .map_err(|_| SourceError::CheckpointFailed)?;
        let (ordinal, disposition) = joined?;
        admitted[ordinal] = false;
        let state = &mut plan.states[ordinal];
        match disposition {
            ChapterDisposition::Complete => {
                state.status = domain::DownloadChapterStatus::Complete;
                state.retry_not_before = 0;
            }
            ChapterDisposition::RetryAt(deadline) => {
                state.status = domain::DownloadChapterStatus::RetryWait;
                state.retry_not_before = deadline;
                if let Some(entry) = plan.chapters.get_mut(&task.chapters[ordinal]) {
                    entry.cached_full = false;
                    entry.cached_images = false;
                }
            }
            ChapterDisposition::Blocked => {
                state.status = domain::DownloadChapterStatus::Blocked;
                state.retry_not_before = 0;
                if let Some(entry) = plan.chapters.get_mut(&task.chapters[ordinal]) {
                    entry.cached_full = false;
                    entry.cached_images = false;
                }
            }
            ChapterDisposition::Stopped => continue,
        }
        if last_progress.is_none_or(|instant| instant.elapsed() >= Duration::from_secs(1)) {
            if let Ok(credit) = progress.clone().try_acquire_owned() {
                let slot = active
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if slot.as_ref().is_some_and(|active| active.token == token)
                    && workers.check_current().is_ok()
                {
                    let (completed, waiting, blocked) = counts(&plan.states);
                    last_progress = Some(Instant::now());
                    let title = plan
                        .chapters
                        .get(&task.chapters[ordinal])
                        .map_or_else(String::new, |entry| entry.title.clone());
                    let _ = sender.send(
                        JobEvent::new(
                            request,
                            epoch,
                            JobResult::DownloadProgress {
                                task_id: task.id.clone(),
                                completed,
                                waiting,
                                blocked,
                                total: task.chapters.len(),
                                title,
                            },
                        )
                        .retain(credit, None),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod retry_control_tests {
    use super::*;

    fn pending() -> DownloadPlanState {
        DownloadPlanState {
            status: domain::DownloadChapterStatus::Pending,
            retry_not_before: 0,
        }
    }

    #[test]
    fn a_request_between_failure_ack_and_join_settlement_is_not_lost() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let retries = DownloadRetryControl::new(3);
            let mut states = vec![pending(); 3];
            let mut running = JoinSet::new();
            let (checkpoint, committed) = tokio::sync::oneshot::channel();
            let (ack_seen, acknowledged) = tokio::sync::oneshot::channel();
            let (release, settle) = tokio::sync::oneshot::channel();
            running.spawn(async move {
                let disposition = committed.await.unwrap();
                ack_seen.send(()).unwrap();
                settle.await.unwrap();
                disposition
            });
            checkpoint
                .send(domain::DownloadChapterRetry::Blocked)
                .unwrap();
            acknowledged.await.unwrap();

            // Hold the real worker after its durable ACK but before JoinSet
            // completion. A user action in this window must survive admission.
            assert!(retries.request(&[0, 1, 2]));
            retries.apply(&mut states, &[true; 3]);
            assert!(running.try_join_next().is_none());
            assert_eq!(retries.requested.lock().unwrap().bits[0], 0b111);
            assert!(
                states
                    .iter()
                    .all(|state| state.status == domain::DownloadChapterStatus::Pending)
            );

            release.send(()).unwrap();
            assert_eq!(
                running.join_next().await.unwrap().unwrap(),
                domain::DownloadChapterRetry::Blocked
            );
            states[0].status = domain::DownloadChapterStatus::Blocked;
            states[1].status = domain::DownloadChapterStatus::Complete;
            states[2].status = domain::DownloadChapterStatus::RetryWait;
            states[2].retry_not_before = 2_000_000_000;
            retries.apply(&mut states, &[false; 3]);
            assert_eq!(states[0].status, domain::DownloadChapterStatus::RetryWait);
            assert_eq!(states[0].retry_not_before, 0);
            assert_eq!(states[1].status, domain::DownloadChapterStatus::Complete);
            assert_eq!(states[2].status, domain::DownloadChapterStatus::RetryWait);
            assert_eq!(states[2].retry_not_before, 2_000_000_000);
            assert!(
                retries
                    .requested
                    .lock()
                    .unwrap()
                    .bits
                    .iter()
                    .all(|word| *word == 0)
            );
        });
    }

    #[test]
    fn repeated_retry_controls_coalesce_inside_one_bounded_bitmap() {
        let retries = DownloadRetryControl::new(MAX_DOWNLOAD_CHAPTERS);
        assert_eq!(retries.requested.lock().unwrap().bits.len(), 128);
        for _ in 0..10_000 {
            assert!(retries.request(&[0, 63, 64, MAX_DOWNLOAD_CHAPTERS - 1]));
        }
        assert!(!retries.request(&[MAX_DOWNLOAD_CHAPTERS]));
        assert!(!retries.request(&[]));
        assert!(!retries.request(&vec![0; MAX_DOWNLOAD_CHAPTERS + 1]));
        let requested = retries.requested.lock().unwrap();
        assert_eq!(requested.bits.len(), 128);
        assert_eq!(
            requested
                .bits
                .iter()
                .map(|word| word.count_ones())
                .sum::<u32>(),
            4
        );
        drop(requested);
        retries.close();
        assert!(!retries.request(&[0]));
        assert!(
            retries
                .requested
                .lock()
                .unwrap()
                .bits
                .iter()
                .all(|word| *word == 0)
        );
    }

    #[test]
    fn terminal_close_cannot_drop_an_already_accepted_retry_request() {
        let retries = DownloadRetryControl::new(1);
        assert!(retries.request(&[0]));
        assert!(!retries.close_if_idle());
        let mut states = [DownloadPlanState {
            status: domain::DownloadChapterStatus::Blocked,
            retry_not_before: 0,
        }];
        retries.apply(&mut states, &[false]);
        assert_eq!(states[0].status, domain::DownloadChapterStatus::RetryWait);
        assert_eq!(states[0].retry_not_before, 0);
        assert!(retries.close_if_idle());
        assert!(!retries.request(&[0]));
    }
}
