//! Bounded, anonymous counters for actual download work and stage waits.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) const SUMMARY_INTERVAL: Duration = Duration::from_secs(5);
// This process-local ordinal is unrelated to persisted task or account identity.
static NEXT_RUN: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy)]
pub(crate) enum ResponsePhase {
    Reader,
    Shard,
    Result,
}

impl ResponsePhase {
    fn label(self) -> &'static str {
        match self {
            Self::Reader => "reader",
            Self::Shard => "shard",
            Self::Result => "result",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    TextWait,
    Images,
    Preparation,
    Ack,
}

impl Stage {
    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Default)]
struct Activity {
    active: AtomicUsize,
    peak: AtomicUsize,
    started: AtomicUsize,
    total_ms: AtomicU64,
}

#[derive(Clone, Copy)]
enum ActivityKind {
    Chapter,
    Http,
}

pub(crate) struct DownloadDiagnostics {
    run: u64,
    configured: u32,
    effective: AtomicU32,
    remaining: usize,
    include_images: bool,
    began: Instant,
    chapters: Activity,
    http: Activity,
    reader_responses: AtomicUsize,
    reader_response_max_bytes: AtomicUsize,
    checkpoints: AtomicUsize,
    stages: [AtomicU64; 4],
    active_stage: AtomicUsize,
    stage_started_ms: AtomicU64,
}

impl DownloadDiagnostics {
    pub(crate) fn new(configured: u32, remaining: usize, include_images: bool) -> Arc<Self> {
        Arc::new(Self {
            run: NEXT_RUN.fetch_add(1, Ordering::SeqCst),
            configured,
            effective: AtomicU32::new(configured),
            remaining,
            include_images,
            began: Instant::now(),
            chapters: Activity::default(),
            http: Activity::default(),
            reader_responses: AtomicUsize::new(0),
            reader_response_max_bytes: AtomicUsize::new(0),
            checkpoints: AtomicUsize::new(0),
            stages: std::array::from_fn(|_| AtomicU64::new(0)),
            active_stage: AtomicUsize::new(0),
            stage_started_ms: AtomicU64::new(0),
        })
    }

    fn activity(&self, kind: ActivityKind) -> &Activity {
        match kind {
            ActivityKind::Chapter => &self.chapters,
            ActivityKind::Http => &self.http,
        }
    }

    fn track(self: &Arc<Self>, kind: ActivityKind) -> ActivityGuard {
        let activity = self.activity(kind);
        let active = activity.active.fetch_add(1, Ordering::SeqCst) + 1;
        activity.peak.fetch_max(active, Ordering::SeqCst);
        activity.started.fetch_add(1, Ordering::SeqCst);
        ActivityGuard {
            diagnostics: self.clone(),
            kind,
            began: Instant::now(),
        }
    }

    pub(crate) fn chapter_read(self: &Arc<Self>) -> ActivityGuard {
        self.track(ActivityKind::Chapter)
    }

    pub(crate) fn http_read(self: &Arc<Self>) -> ActivityGuard {
        self.track(ActivityKind::Http)
    }

    pub(crate) fn stage(self: &Arc<Self>, stage: Stage) -> StageGuard {
        self.stage_started_ms
            .store(milliseconds(self.began.elapsed()), Ordering::SeqCst);
        self.active_stage.store(stage.index() + 1, Ordering::SeqCst);
        StageGuard {
            diagnostics: self.clone(),
            stage,
            began: Instant::now(),
        }
    }

    pub(crate) fn checkpoint(&self) {
        self.checkpoints.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn reader_response(&self, bytes: usize) {
        self.reader_responses.fetch_add(1, Ordering::SeqCst);
        self.reader_response_max_bytes
            .fetch_max(bytes, Ordering::SeqCst);
    }

    pub(crate) fn serial_fallback(&self) {
        let previous = self.effective.swap(1, Ordering::SeqCst);
        crate::diagnostic!(
            "weread: download run={} event=mode_change scope=chapter reason=prefetch_budget configured={} effective_before={previous} effective_after=1",
            self.run,
            self.configured,
        );
    }

    pub(crate) fn resume_parallel(&self) {
        let previous = self.effective.swap(self.configured, Ordering::SeqCst);
        if previous != self.configured {
            crate::diagnostic!(
                "weread: download run={} event=mode_change scope=chapter reason=checkpoint_acked configured={} effective_before={previous} effective_after={}",
                self.run,
                self.configured,
                self.configured,
            );
        }
    }

    pub(crate) fn budget_exceeded(
        &self,
        phase: ResponsePhase,
        limit_bytes: usize,
        observed_bytes: usize,
        declared_bytes: Option<u64>,
    ) {
        crate::diagnostic!(
            "weread: download run={} event=budget_exceeded phase={} limit_bytes={limit_bytes} observed_bytes={observed_bytes} declared_bytes={} has_declared_bytes={}",
            self.run,
            phase.label(),
            declared_bytes.unwrap_or(0),
            u8::from(declared_bytes.is_some()),
        );
    }

    pub(crate) fn source_budget_exceeded(&self, phase: ResponsePhase, observed_bytes: usize) {
        crate::diagnostic!(
            "weread: download run={} event=budget_exceeded phase={} reason=source_budget limit_bytes=unknown observed_bytes={observed_bytes}",
            self.run,
            phase.label(),
        );
    }

    pub(crate) fn report(&self, outcome: &'static str) {
        let elapsed_ms = milliseconds(self.began.elapsed());
        let active_stage = self.active_stage.load(Ordering::SeqCst);
        let mut stages: [u64; 4] =
            std::array::from_fn(|index| self.stages[index].load(Ordering::SeqCst));
        if let Some(stage) = active_stage
            .checked_sub(1)
            .filter(|stage| *stage < stages.len())
        {
            stages[stage] = stages[stage].saturating_add(
                elapsed_ms.saturating_sub(self.stage_started_ms.load(Ordering::SeqCst)),
            );
        }
        // All timing values are cumulative since task start. HTTP/read sums
        // can exceed elapsed time because their operations overlap.
        crate::diagnostic!(
            "weread: download run={} event=summary outcome={outcome} configured={} effective={} remaining_at_start={} include_images={} elapsed_ms={elapsed_ms} chapter_reads_started={} chapter_reads_active={} chapter_reads_peak={} chapter_http_started={} chapter_http_active={} chapter_http_peak={} checkpoints_acked={} reader_responses={} reader_response_max_bytes={} text_wait_total_ms={} image_stage_total_ms={} preparation_wait_total_ms={} ack_wait_total_ms={} chapter_read_total_ms={} chapter_http_total_ms={}",
            self.run,
            self.configured,
            self.effective.load(Ordering::SeqCst),
            self.remaining,
            u8::from(self.include_images),
            self.chapters.started.load(Ordering::SeqCst),
            self.chapters.active.load(Ordering::SeqCst),
            self.chapters.peak.load(Ordering::SeqCst),
            self.http.started.load(Ordering::SeqCst),
            self.http.active.load(Ordering::SeqCst),
            self.http.peak.load(Ordering::SeqCst),
            self.checkpoints.load(Ordering::SeqCst),
            self.reader_responses.load(Ordering::SeqCst),
            self.reader_response_max_bytes.load(Ordering::SeqCst),
            stages[0],
            stages[1],
            stages[2],
            stages[3],
            self.chapters.total_ms.load(Ordering::SeqCst),
            self.http.total_ms.load(Ordering::SeqCst),
        );
    }

    #[cfg(test)]
    pub(crate) fn activity_snapshot(&self) -> (usize, usize, usize, usize) {
        (
            self.chapters.active.load(Ordering::SeqCst),
            self.chapters.peak.load(Ordering::SeqCst),
            self.http.active.load(Ordering::SeqCst),
            self.http.peak.load(Ordering::SeqCst),
        )
    }
}

pub(crate) struct ActivityGuard {
    diagnostics: Arc<DownloadDiagnostics>,
    kind: ActivityKind,
    began: Instant,
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let activity = self.diagnostics.activity(self.kind);
        activity
            .total_ms
            .fetch_add(milliseconds(self.began.elapsed()), Ordering::SeqCst);
        activity.active.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(crate) struct StageGuard {
    diagnostics: Arc<DownloadDiagnostics>,
    stage: Stage,
    began: Instant,
}

impl Drop for StageGuard {
    fn drop(&mut self) {
        self.diagnostics.stages[self.stage.index()]
            .fetch_add(milliseconds(self.began.elapsed()), Ordering::SeqCst);
        self.diagnostics.active_stage.store(0, Ordering::SeqCst);
    }
}

pub(crate) struct DownloadRun {
    diagnostics: Arc<DownloadDiagnostics>,
    finished: bool,
}

impl DownloadRun {
    pub(crate) fn new(diagnostics: Arc<DownloadDiagnostics>) -> Self {
        crate::diagnostic!(
            "weread: download run={} event=start configured={} remaining_at_start={} include_images={}",
            diagnostics.run,
            diagnostics.configured,
            diagnostics.remaining,
            u8::from(diagnostics.include_images),
        );
        Self {
            diagnostics,
            finished: false,
        }
    }

    pub(crate) fn finish(&mut self, outcome: &'static str) {
        if !self.finished {
            self.diagnostics.report(outcome);
            self.finished = true;
        }
    }
}

impl Drop for DownloadRun {
    fn drop(&mut self) {
        if !self.finished {
            self.diagnostics.report("cancelled");
        }
    }
}

fn milliseconds(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_results_do_not_count_as_active_chapter_or_http_reads() {
        let diagnostics = DownloadDiagnostics::new(8, 20, true);
        let first = diagnostics.chapter_read();
        let second = diagnostics.chapter_read();
        let http = diagnostics.http_read();
        assert_eq!(diagnostics.activity_snapshot(), (2, 2, 1, 1));
        drop(http);
        assert_eq!(diagnostics.activity_snapshot(), (2, 2, 0, 1));
        drop((first, second));
        assert_eq!(diagnostics.activity_snapshot(), (0, 2, 0, 1));
        // The task owner and any finished-result holders may keep the counters
        // alive without being mistaken for network work.
        let retained_result_owner = diagnostics.clone();
        assert_eq!(retained_result_owner.activity_snapshot(), (0, 2, 0, 1));
    }

    #[test]
    fn aborting_a_read_releases_both_activity_guards() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let diagnostics = DownloadDiagnostics::new(8, 20, true);
            let tracked = diagnostics.clone();
            let (started, ready) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                let _chapter = tracked.chapter_read();
                let _http = tracked.http_read();
                started.send(()).unwrap();
                std::future::pending::<()>().await;
            });
            ready.await.unwrap();
            assert_eq!(diagnostics.activity_snapshot(), (1, 1, 1, 1));
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            assert_eq!(diagnostics.activity_snapshot(), (0, 1, 0, 1));
        });
    }
}
