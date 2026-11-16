//! One cancellable picture scene worker with UI-owned admission and publication.

use super::image_raster::{self, PictureRequest, PreparedPixels, RasterBudget, RasterLimitError};
use crate::reader::layout_budget::{LayoutLease, LayoutModelRegistration};
use anyhow::{Context, Result, anyhow, ensure};
use std::{
    fmt,
    ops::Deref,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(super) struct SceneRequest {
    pub key: String,
    pub viewport: (u32, u32),
    pub pictures: Vec<PictureRequest>,
    pub storage: LayoutLease,
    pub registration: LayoutModelRegistration,
    pub notify: Option<Box<dyn FnOnce() + Send + 'static>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ImagePreparationStats {
    pub started: u64,
    pub reused: u64,
    // Only successful scenes accepted for the current desired key.
    pub completed: u64,
    pub cancelled: u64,
    pub stale: u64,
    pub latest_worker_us: u64,
    pub latest_wake_ui_us: u64,
    pub wake_ui_us_total: u64,
    pub wake_callbacks: u64,
}

#[derive(Clone, Debug)]
enum PreparationFailure {
    Limit(RasterLimitError),
    Other(String),
}

impl PreparationFailure {
    fn capture(error: anyhow::Error) -> Self {
        error
            .chain()
            .find_map(|cause| cause.downcast_ref::<RasterLimitError>())
            .copied()
            .map(Self::Limit)
            .unwrap_or_else(|| Self::Other(format!("{error:#}")))
    }

    fn error(&self) -> anyhow::Error {
        match self {
            Self::Limit(error) => (*error).into(),
            Self::Other(message) => anyhow!(message.clone()),
        }
    }
}

enum WorkerResult {
    Prepared(Vec<PreparedPixels>),
    Failed(PreparationFailure),
    Cancelled,
}

struct WorkerReply {
    key: String,
    result: WorkerResult,
    prepared: bool,
    elapsed_us: u64,
}

struct ActiveScene {
    key: String,
    cancelled: Arc<AtomicBool>,
    replies: Receiver<WorkerReply>,
    thread: JoinHandle<()>,
    reply: Option<WorkerReply>,
    disconnected: bool,
    started: Instant,
    // Admission never crosses the worker boundary, including on cancellation.
    storage: LayoutLease,
    registration: LayoutModelRegistration,
}

struct ReadyScene {
    key: String,
    pixels: Rc<PreparedScenePixels>,
}

/// Pixel ownership and its admission are released by the same final UI owner.
pub(super) struct PreparedScenePixels {
    pixels: Vec<PreparedPixels>,
    _storage: LayoutLease,
    _registration: LayoutModelRegistration,
}

impl Deref for PreparedScenePixels {
    type Target = [PreparedPixels];

    fn deref(&self) -> &Self::Target {
        &self.pixels
    }
}

impl fmt::Debug for PreparedScenePixels {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedScenePixels")
            .field("count", &self.pixels.len())
            .finish()
    }
}

struct FailedScene {
    key: String,
    error: PreparationFailure,
    reported: bool,
}

/// Rc deliberately keeps this controller on its owning UI thread. A replacement
/// target retains only its key while an obsolete worker is still being drained.
#[derive(Default)]
pub(super) struct ImagePreparationState {
    desired: Option<String>,
    active: Option<ActiveScene>,
    ready: Option<ReadyScene>,
    failure: Option<FailedScene>,
    restart_needed: bool,
    stats: ImagePreparationStats,
}

impl ImagePreparationState {
    pub(super) fn request(
        &mut self,
        request: SceneRequest,
    ) -> Result<Option<Rc<PreparedScenePixels>>> {
        self.request_with(request, prepare_scene)
    }

    fn request_with<F>(
        &mut self,
        request: SceneRequest,
        prepare: F,
    ) -> Result<Option<Rc<PreparedScenePixels>>>
    where
        F: FnOnce((u32, u32), Vec<PictureRequest>, &AtomicBool) -> Result<Vec<PreparedPixels>>
            + Send
            + 'static,
    {
        self.restart_needed = false;
        if self.desired.as_deref() != Some(request.key.as_str()) {
            self.desired = Some(request.key.clone());
            self.ready = None;
            self.failure = None;
            if let Some(active) = &self.active {
                active.cancelled.store(true, Ordering::Release);
            }
        }
        self.poll_active()?;
        if let Some(ready) = &self.ready {
            if ready.key == request.key {
                self.stats.reused = self.stats.reused.saturating_add(1);
                return Ok(Some(ready.pixels.clone()));
            }
        }
        if let Some(failure) = self.failure.as_mut() {
            if failure.key == request.key {
                if !failure.reported {
                    failure.reported = true;
                    return Err(failure.error.error());
                }
                return Ok(None);
            }
        }
        if self.active.is_some() {
            // Do not retain the replacement's vector, paths, or metadata lease.
            // A later render supplies a fresh request after this slot is joined.
            return Ok(None);
        }

        let SceneRequest {
            key,
            viewport,
            pictures,
            storage,
            registration,
            notify,
        } = request;
        let worker_key = key.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let (sender, replies) = mpsc::sync_channel(1);
        let started = Instant::now();
        let spawned = thread::Builder::new()
            .name("weread-image-scene".into())
            .spawn(move || {
                let started = Instant::now();
                let mut completed = false;
                let prepared = if worker_cancelled.load(Ordering::Acquire) {
                    Ok(Vec::new())
                } else {
                    let result = prepare(viewport, pictures, &worker_cancelled);
                    completed = result.is_ok();
                    result
                };
                let result = if worker_cancelled.load(Ordering::Acquire) {
                    WorkerResult::Cancelled
                } else {
                    match prepared {
                        Ok(pixels) => WorkerResult::Prepared(pixels),
                        Err(error) => WorkerResult::Failed(PreparationFailure::capture(error)),
                    }
                };
                // A cancellation racing this send is also checked by the UI.
                let result = if worker_cancelled.load(Ordering::Acquire) {
                    WorkerResult::Cancelled
                } else {
                    result
                };
                let _ = sender.send(WorkerReply {
                    key: worker_key,
                    result,
                    prepared: completed,
                    elapsed_us: elapsed_us(started),
                });
                // Queue the result before waking its UI owner. Notification
                // never releases the slot; only a later drain and join can.
                // A dropped receiver still permits a harmless final wakeup.
                if let Some(notify) = notify {
                    notify();
                }
            });
        let worker = match spawned {
            Ok(worker) => worker,
            Err(error) => {
                let error = PreparationFailure::capture(error.into());
                let reported = error.error();
                self.failure = Some(FailedScene {
                    key,
                    error,
                    reported: true,
                });
                return Err(reported);
            }
        };
        self.stats.started = self.stats.started.saturating_add(1);
        self.active = Some(ActiveScene {
            key,
            cancelled,
            replies,
            thread: worker,
            reply: None,
            disconnected: false,
            started,
            storage,
            registration,
        });
        Ok(None)
    }

    /// Receiving the result alone does not release the worker slot or its lease.
    /// Joining happens only after is_finished, so ordinary polling never waits.
    pub(super) fn poll(&mut self) -> Result<bool> {
        let changed = self.poll_active()?;
        if self.active.is_none() && self.restart_needed {
            self.restart_needed = false;
            return Ok(true);
        }
        Ok(changed)
    }

    fn poll_active(&mut self) -> Result<bool> {
        let Some(active) = self.active.as_mut() else {
            return Ok(false);
        };
        let mut changed = false;
        if active.reply.is_none() && !active.disconnected {
            match active.replies.try_recv() {
                Ok(reply) => {
                    active.reply = Some(reply);
                    changed = true;
                }
                Err(TryRecvError::Disconnected) => {
                    active.disconnected = true;
                    changed = true;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if !active.thread.is_finished() {
            return Ok(changed);
        }
        let mut active = self.active.take().expect("The finished scene was present");
        if active.reply.is_none() {
            active.reply = active.replies.try_recv().ok();
        }
        let joined = active.thread.join();
        self.stats.latest_worker_us = active
            .reply
            .as_ref()
            .map_or_else(|| elapsed_us(active.started), |reply| reply.elapsed_us);
        if active.cancelled.load(Ordering::Acquire)
            || self.desired.as_deref() != Some(active.key.as_str())
            || matches!(
                active.reply.as_ref().map(|reply| &reply.result),
                Some(WorkerResult::Cancelled)
            )
        {
            self.stats.cancelled = self.stats.cancelled.saturating_add(1);
            if active.reply.as_ref().is_some_and(|reply| reply.prepared) {
                self.stats.stale = self.stats.stale.saturating_add(1);
            }
            return Ok(true);
        }
        let result = if joined.is_err() {
            WorkerResult::Failed(PreparationFailure::Other(
                "the image preparation worker panicked".into(),
            ))
        } else {
            match active.reply {
                Some(reply) if reply.key == active.key => reply.result,
                _ => WorkerResult::Failed(PreparationFailure::Other(
                    "the image preparation worker ended without its scene result".into(),
                )),
            }
        };
        match result {
            WorkerResult::Prepared(pixels) => {
                self.stats.completed = self.stats.completed.saturating_add(1);
                self.ready = Some(ReadyScene {
                    key: active.key,
                    pixels: Rc::new(PreparedScenePixels {
                        pixels,
                        _storage: active.storage,
                        _registration: active.registration,
                    }),
                });
            }
            WorkerResult::Failed(error) => {
                self.failure = Some(FailedScene {
                    key: active.key,
                    error,
                    reported: false,
                });
            }
            WorkerResult::Cancelled => unreachable!("Cancelled scenes are retired before publish"),
        }
        Ok(true)
    }

    pub(super) fn cancel(&mut self) {
        self.desired = None;
        self.ready = None;
        self.failure = None;
        self.restart_needed = false;
        if let Some(active) = &self.active {
            active.cancelled.store(true, Ordering::Release);
        }
    }

    /// Pause optional in-flight work without evicting an already ready scene.
    pub(super) fn cancel_pending(&mut self) {
        if let Some(active) = &self.active {
            active.cancelled.store(true, Ordering::Release);
            if self.desired.is_some() {
                self.restart_needed = true;
            }
        }
    }

    pub(super) fn busy(&self) -> bool {
        self.active.is_some()
    }

    pub(super) fn ready(&self) -> bool {
        self.ready.is_some()
    }

    /// Sleep-time renders may reuse pixels, but cannot drain or start a worker.
    pub(super) fn ready_while_paused(&mut self, key: &str) -> Option<Rc<PreparedScenePixels>> {
        if let Some(ready) = &self.ready {
            if ready.key == key {
                self.stats.reused = self.stats.reused.saturating_add(1);
                return Some(ready.pixels.clone());
            }
        }
        if self.desired.as_deref() != Some(key) {
            self.desired = Some(key.into());
        }
        if self.ready.as_ref().is_some_and(|ready| ready.key != key) {
            self.ready = None;
        }
        if self
            .failure
            .as_ref()
            .is_some_and(|failure| failure.key != key)
        {
            self.failure = None;
        }
        self.cancel_pending();
        self.restart_needed = true;
        None
    }

    pub(super) fn restart_needed(&self) -> bool {
        self.restart_needed
    }

    pub(super) fn stats(&self) -> ImagePreparationStats {
        self.stats
    }

    pub(super) fn record_wake(&mut self, elapsed: Duration) {
        let elapsed_us = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
        self.stats.latest_wake_ui_us = elapsed_us;
        self.stats.wake_ui_us_total = self.stats.wake_ui_us_total.saturating_add(elapsed_us);
        self.stats.wake_callbacks = self.stats.wake_callbacks.saturating_add(1);
    }

    /// A timeout retains the active slot and admission until a later drain.
    pub(super) fn quiesce(&mut self, timeout: Duration) -> Result<()> {
        let started = Instant::now();
        self.cancel_pending();
        loop {
            self.poll_active()?;
            if !self.busy() {
                return Ok(());
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            ensure!(
                !remaining.is_zero(),
                "the image preparation worker did not stop within its deadline"
            );
            thread::sleep(remaining.min(Duration::from_millis(1)));
        }
    }
}

impl Drop for ImagePreparationState {
    fn drop(&mut self) {
        // Verified lifecycle cleanup uses quiesce. Destruction only cancels and
        // detaches if a failed bounded shutdown left a worker inside a decoder.
        self.cancel();
    }
}

fn elapsed_us(started: Instant) -> u64 {
    started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

fn prepare_scene(
    viewport: (u32, u32),
    pictures: Vec<PictureRequest>,
    cancelled: &AtomicBool,
) -> Result<Vec<PreparedPixels>> {
    ensure!(
        !cancelled.load(Ordering::Acquire),
        "image preparation was cancelled"
    );
    let mut budget = RasterBudget::new(viewport)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(pictures.len())
        .context("reserve prepared scene pictures")?;
    for picture in &pictures {
        ensure!(
            !cancelled.load(Ordering::Acquire),
            "image preparation was cancelled"
        );
        output.push(image_raster::prepare_cached_picture(
            picture,
            &mut budget,
            cancelled,
        )?);
    }
    ensure!(
        !cancelled.load(Ordering::Acquire),
        "image preparation was cancelled"
    );
    Ok(output)
}

#[cfg(test)]
#[path = "image_preparation_tests.rs"]
mod tests;
