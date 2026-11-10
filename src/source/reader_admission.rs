//! Shared decoded Reader response admission, independent of chapter scheduling.

use super::{Result, SourceError};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Instant;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(crate) const READER_BUDGET_BYTES: usize = 16 * 1024 * 1024;
const READER_CREDIT_BYTES: usize = 64 * 1024;
const READER_PARSER_WORKERS: usize = 2;
const RESPONSE_TIERS: [usize; 6] = [
    512 * 1024,
    2 * 1024 * 1024,
    4 * 1024 * 1024,
    8 * 1024 * 1024,
    16 * 1024 * 1024,
    24 * 1024 * 1024,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ReaderTier(usize);

impl ReaderTier {
    pub(super) const INITIAL: Self = Self(0);
    pub(super) const FULL: Self = Self(RESPONSE_TIERS.len() - 1);

    pub(super) fn bytes(self) -> usize {
        RESPONSE_TIERS[self.0]
    }

    fn next(self) -> Option<Self> {
        (self.0 + 1 < RESPONSE_TIERS.len()).then_some(Self(self.0 + 1))
    }
}

/// Only the response tier is learned. Reader sessions and access evidence are
/// always obtained anew for the requested chapter and current credentials.
#[derive(Clone)]
pub(super) struct DownloadReaderContext {
    pub(super) admission: Arc<ReaderAdmission>,
    learned_tier: Arc<AtomicUsize>,
}

impl DownloadReaderContext {
    pub(super) fn new(admission: Arc<ReaderAdmission>) -> Self {
        Self {
            admission,
            learned_tier: Arc::new(AtomicUsize::new(ReaderTier::INITIAL.0)),
        }
    }

    pub(super) fn tier(&self) -> ReaderTier {
        ReaderTier(self.learned_tier.load(Ordering::SeqCst))
    }

    pub(super) fn advance(&self, attempted: ReaderTier) -> Option<ReaderTier> {
        let next = attempted.next()?;
        let previous = self.learned_tier.fetch_max(next.0, Ordering::SeqCst);
        if previous < next.0 {
            crate::diagnostic!(
                "weread: source operation=reader_response_tier previous_limit_bytes={} next_limit_bytes={} pool_bytes={}",
                ReaderTier(previous).bytes(),
                next.bytes(),
                self.admission.budget_bytes(),
            );
        }
        Some(self.tier())
    }
}

/// This budget bounds admitted decoded responses, not allocator usage or RSS.
/// The legacy 24 MiB response allowance occupies the complete pool exclusively.
/// Parser workspace has its own existing input/work limits and two workers.
pub(crate) struct ReaderAdmission {
    bytes: Arc<Semaphore>,
    parsers: Arc<Semaphore>,
    credits: usize,
    reserved_bytes: AtomicUsize,
    peak_reserved_bytes: AtomicUsize,
    pages_active: AtomicUsize,
    peak_pages_active: AtomicUsize,
    parsers_active: AtomicUsize,
    peak_parsers_active: AtomicUsize,
    parsers_waiting: AtomicUsize,
    peak_parsers_waiting: AtomicUsize,
    response_allowance_bytes: AtomicUsize,
    peak_response_allowance_bytes: AtomicUsize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReaderAdmissionSnapshot {
    pub(crate) budget_bytes: usize,
    pub(crate) available_bytes: usize,
    pub(crate) reserved_bytes: usize,
    pub(crate) peak_reserved_bytes: usize,
    pub(crate) response_allowance_bytes: usize,
    pub(crate) peak_response_allowance_bytes: usize,
    pub(crate) pages_active: usize,
    pub(crate) peak_pages_active: usize,
    pub(crate) parsers_active: usize,
    pub(crate) peak_parsers_active: usize,
    pub(crate) parsers_waiting: usize,
    pub(crate) peak_parsers_waiting: usize,
}

impl ReaderAdmission {
    pub(crate) fn new() -> Self {
        Self::with_budget(READER_BUDGET_BYTES, READER_PARSER_WORKERS)
    }

    fn with_budget(budget_bytes: usize, parser_workers: usize) -> Self {
        assert!(budget_bytes > 0 && budget_bytes % READER_CREDIT_BYTES == 0);
        assert!(budget_bytes <= READER_BUDGET_BYTES);
        assert!(parser_workers > 0 && parser_workers <= READER_PARSER_WORKERS);
        let credits = budget_bytes / READER_CREDIT_BYTES;
        Self {
            bytes: Arc::new(Semaphore::new(credits)),
            parsers: Arc::new(Semaphore::new(parser_workers)),
            credits,
            reserved_bytes: AtomicUsize::new(0),
            peak_reserved_bytes: AtomicUsize::new(0),
            pages_active: AtomicUsize::new(0),
            peak_pages_active: AtomicUsize::new(0),
            parsers_active: AtomicUsize::new(0),
            peak_parsers_active: AtomicUsize::new(0),
            parsers_waiting: AtomicUsize::new(0),
            peak_parsers_waiting: AtomicUsize::new(0),
            response_allowance_bytes: AtomicUsize::new(0),
            peak_response_allowance_bytes: AtomicUsize::new(0),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_tests(budget_bytes: usize, parser_workers: usize) -> Self {
        Self::with_budget(budget_bytes, parser_workers)
    }

    #[cfg(test)]
    pub(crate) fn close(&self) {
        // App recovery fixtures may deny new Reader requests without adding
        // a production shutdown policy or bypassing the actual admission path.
        self.bytes.close();
    }

    fn budget_bytes(&self) -> usize {
        self.credits * READER_CREDIT_BYTES
    }

    pub(crate) fn snapshot(&self) -> ReaderAdmissionSnapshot {
        ReaderAdmissionSnapshot {
            budget_bytes: self.budget_bytes(),
            available_bytes: self.bytes.available_permits() * READER_CREDIT_BYTES,
            reserved_bytes: self.reserved_bytes.load(Ordering::SeqCst),
            peak_reserved_bytes: self.peak_reserved_bytes.load(Ordering::SeqCst),
            response_allowance_bytes: self.response_allowance_bytes.load(Ordering::SeqCst),
            peak_response_allowance_bytes: self
                .peak_response_allowance_bytes
                .load(Ordering::SeqCst),
            pages_active: self.pages_active.load(Ordering::SeqCst),
            peak_pages_active: self.peak_pages_active.load(Ordering::SeqCst),
            parsers_active: self.parsers_active.load(Ordering::SeqCst),
            peak_parsers_active: self.peak_parsers_active.load(Ordering::SeqCst),
            parsers_waiting: self.parsers_waiting.load(Ordering::SeqCst),
            peak_parsers_waiting: self.peak_parsers_waiting.load(Ordering::SeqCst),
        }
    }

    pub(super) async fn acquire(self: &Arc<Self>, tier: ReaderTier) -> Result<ReaderReservation> {
        let began = Instant::now();
        let charged_bytes = tier.bytes().min(self.budget_bytes());
        let credits = (charged_bytes / READER_CREDIT_BYTES) as u32;
        let permit = self
            .bytes
            .clone()
            .acquire_many_owned(credits)
            .await
            .map_err(|_| SourceError::Unsupported("download_cancelled"))?;
        let reserved = self
            .reserved_bytes
            .fetch_add(charged_bytes, Ordering::SeqCst)
            + charged_bytes;
        self.peak_reserved_bytes
            .fetch_max(reserved, Ordering::SeqCst);
        let active = self.pages_active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak_pages_active.fetch_max(active, Ordering::SeqCst);
        let allowance = self
            .response_allowance_bytes
            .fetch_add(tier.bytes(), Ordering::SeqCst)
            + tier.bytes();
        self.peak_response_allowance_bytes
            .fetch_max(allowance, Ordering::SeqCst);
        crate::diagnostic!(
            "weread: source operation=reader_admission response_limit_bytes={} reserved_bytes={reserved} response_allowance_bytes={allowance} pool_bytes={} reader_pages_active={active} wait_ms={}",
            tier.bytes(),
            self.budget_bytes(),
            began.elapsed().as_millis(),
        );
        Ok(ReaderReservation {
            admission: self.clone(),
            charged_bytes,
            response_limit: tier.bytes(),
            _permit: permit,
        })
    }
}

pub(super) struct ReaderReservation {
    admission: Arc<ReaderAdmission>,
    charged_bytes: usize,
    response_limit: usize,
    _permit: OwnedSemaphorePermit,
}

impl ReaderReservation {
    pub(super) fn page(self, html: String) -> ReaderPage {
        ReaderPage {
            html,
            reservation: self,
        }
    }
}

impl Drop for ReaderReservation {
    fn drop(&mut self) {
        // Decrement observations before the semaphore field returns credit.
        self.admission
            .reserved_bytes
            .fetch_sub(self.charged_bytes, Ordering::SeqCst);
        self.admission
            .response_allowance_bytes
            .fetch_sub(self.response_limit, Ordering::SeqCst);
        self.admission.pages_active.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Field order also protects cancellation while waiting for a parser: the raw
/// HTML is destroyed before the reservation is returned to another task.
pub(super) struct ReaderPage {
    html: String,
    reservation: ReaderReservation,
}

impl ReaderPage {
    pub(super) async fn parse<T, F>(self, parse: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&str) -> Result<T> + Send + 'static,
    {
        let began = Instant::now();
        let admission = self.reservation.admission.clone();
        let waiting = ReaderParserWait::new(admission.clone());
        let permit = admission
            .parsers
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| SourceError::Unsupported("download_cancelled"))?;
        drop(waiting);
        let active = admission.parsers_active.fetch_add(1, Ordering::SeqCst) + 1;
        admission
            .peak_parsers_active
            .fetch_max(active, Ordering::SeqCst);
        let parser = ReaderParser {
            admission,
            _permit: permit,
        };
        let wait_ms = began.elapsed().as_millis();
        // A dropped JoinHandle does not stop a running blocking closure. Move
        // both permits with the HTML so cancellation cannot release them early.
        let task = tokio::task::spawn_blocking(move || {
            let parser = parser;
            let page = self;
            let began = Instant::now();
            let result = parse(&page.html);
            let bytes = page.html.len();
            drop(page);
            crate::diagnostic!(
                "weread: source operation=reader_parse response_bytes={bytes} parser_workers_active={} parser_workers_peak={} wait_ms={wait_ms} parse_ms={}",
                parser.admission.parsers_active.load(Ordering::SeqCst),
                parser.admission.peak_parsers_active.load(Ordering::SeqCst),
                began.elapsed().as_millis(),
            );
            drop(parser);
            result
        });
        // Abort can discard work that has not started yet; a running closure
        // still owns its permits and returns them only after destroying HTML.
        let _abort_on_drop = AbortReaderParserOnDrop(task.abort_handle());
        task.await
            .map_err(|_| SourceError::InvalidResponse("reader_parse_worker"))?
    }
}

struct ReaderParser {
    admission: Arc<ReaderAdmission>,
    _permit: OwnedSemaphorePermit,
}

struct AbortReaderParserOnDrop(tokio::task::AbortHandle);

impl Drop for AbortReaderParserOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct ReaderParserWait(Arc<ReaderAdmission>);

impl ReaderParserWait {
    fn new(admission: Arc<ReaderAdmission>) -> Self {
        let waiting = admission.parsers_waiting.fetch_add(1, Ordering::SeqCst) + 1;
        admission
            .peak_parsers_waiting
            .fetch_max(waiting, Ordering::SeqCst);
        Self(admission)
    }
}

impl Drop for ReaderParserWait {
    fn drop(&mut self) {
        self.0.parsers_waiting.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Drop for ReaderParser {
    fn drop(&mut self) {
        self.admission.parsers_active.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
#[path = "reader_admission_tests.rs"]
mod tests;
