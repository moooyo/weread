//! Bounded canonical article-image preparation outside the UI thread.
//!
//! Transport credits travel with the encoded input and prepared output. A
//! cancelled active slot remains owned until its thread and result are reaped.

use super::*;
use crate::{cache::PreparedPng, jobs::JobCredits};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self as worker_mpsc, Receiver, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
};

const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_RETAINED_INPUT_BYTES: usize = 2 * MAX_INPUT_BYTES;
const MAX_REQUESTS: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ArticlePreparationContext {
    pub(super) request: u64,
    pub(super) account_epoch: u64,
    pub(super) article_id: String,
    pub(super) image_id: String,
    pub(super) index: usize,
    pub(super) url: String,
    pub(super) cache_key: String,
    pub(super) geometry: (u32, u32),
    generation: u64,
}

struct Request {
    context: ArticlePreparationContext,
    // Drop payloads before their admission credits, including during unwind.
    bytes: Vec<u8>,
    credits: Option<JobCredits>,
}

struct Completion {
    context: ArticlePreparationContext,
    result: Result<PreparedPng>,
    _credits: Option<JobCredits>,
}

struct Active {
    context: ArticlePreparationContext,
    encoded_bytes: usize,
    cancel: Arc<AtomicBool>,
    replies: Receiver<Completion>,
    thread: Option<JoinHandle<()>>,
    immediate: Option<Completion>,
}

fn prepare_request(request: Request, cancel: &AtomicBool, sender: Sender<Completion>) {
    // Keep the entire request intact while decoder code can unwind. Its field
    // order disposes of encoded input before returning transport admission.
    let result = FileCache::prepare_gray8_png_cancellable(
        &request.bytes,
        request.context.geometry.0,
        request.context.geometry.1,
        cancel,
    );
    let Request {
        context,
        bytes,
        credits,
    } = request;
    drop(bytes);
    let _ = sender.send(Completion {
        context,
        result,
        _credits: credits,
    });
}

#[derive(Default)]
pub(super) struct ArticlePreparationState {
    generation: u64,
    exhausted: bool,
    active: Option<Active>,
    queued: VecDeque<Request>,
    started: u64,
    completed: u64,
    cancellations: u64,
    stale: u64,
    peak_encoded_bytes: usize,
}

impl ArticlePreparationState {
    pub(super) fn is_busy(&self) -> bool {
        self.active.is_some() || !self.queued.is_empty()
    }

    fn encoded_bytes(&self) -> usize {
        self.active
            .as_ref()
            .map_or(0, |active| active.encoded_bytes)
            + self
                .queued
                .iter()
                .map(|request| request.bytes.len())
                .sum::<usize>()
    }

    fn enqueue(
        &mut self,
        mut context: ArticlePreparationContext,
        bytes: Vec<u8>,
        credits: Option<JobCredits>,
    ) -> Result<bool> {
        if bytes.len() > MAX_INPUT_BYTES {
            drop(bytes);
            drop(credits);
            anyhow::bail!("article image input exceeds its allowance");
        }
        if self.exhausted
            || usize::from(self.active.is_some()) + self.queued.len() >= MAX_REQUESTS
            || self.encoded_bytes().saturating_add(bytes.len()) > MAX_RETAINED_INPUT_BYTES
        {
            // bytes is dropped before credits when admission cannot retain it.
            drop(bytes);
            drop(credits);
            return Ok(false);
        }
        context.generation = self.generation;
        self.queued.push_back(Request {
            context,
            bytes,
            credits,
        });
        self.peak_encoded_bytes = self.peak_encoded_bytes.max(self.encoded_bytes());
        self.start_next();
        Ok(true)
    }

    fn start_next(&mut self) {
        if self.active.is_some() {
            return;
        }
        let Some(request) = self.queued.pop_front() else {
            return;
        };
        let context = request.context.clone();
        let encoded_bytes = request.bytes.len();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, replies) = worker_mpsc::channel();
        let spawned = thread::Builder::new()
            .name("weread-article-image".into())
            .spawn(move || {
                prepare_request(request, &worker_cancel, sender);
            });
        let (thread, immediate) = match spawned {
            Ok(thread) => {
                self.started = self.started.saturating_add(1);
                (Some(thread), None)
            }
            Err(error) => (
                None,
                Some(Completion {
                    context: context.clone(),
                    result: Err(error.into()),
                    _credits: None,
                }),
            ),
        };
        self.active = Some(Active {
            context,
            encoded_bytes,
            cancel,
            replies,
            thread,
            immediate,
        });
    }

    fn take_completed(&mut self) -> Option<Completion> {
        let active = self.active.as_ref()?;
        if active
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            return None;
        }
        let mut active = self.active.take().expect("The completed slot was present");
        let joined = active.thread.take().map(JoinHandle::join);
        let mut completion = active
            .immediate
            .take()
            .or_else(|| match active.replies.try_recv() {
                Ok(completion) => Some(completion),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
            })
            .unwrap_or_else(|| Completion {
                context: active.context,
                result: Err(anyhow::anyhow!(
                    "The article image worker ended without a result"
                )),
                _credits: None,
            });
        if joined.is_some_and(|result| result.is_err()) {
            completion.result = Err(anyhow::anyhow!("The article image worker panicked"));
        }
        self.completed = self.completed.saturating_add(1);
        Some(completion)
    }

    fn cancel(&mut self) {
        if let Some(active) = self.active.as_ref() {
            active.cancel.store(true, Ordering::Release);
        }
        self.queued.clear();
        self.cancellations = self.cancellations.saturating_add(1);
        match self.generation.checked_add(1) {
            Some(next) => self.generation = next,
            None => self.exhausted = true,
        }
    }

    fn quiesce(&mut self, timeout: Duration) -> Result<()> {
        self.cancel();
        let started = Instant::now();
        while self.active.is_some() {
            if let Some(completion) = self.take_completed() {
                drop(completion);
                continue;
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            anyhow::ensure!(
                !remaining.is_zero(),
                "The article image worker did not stop within its deadline"
            );
            thread::sleep(remaining.min(Duration::from_millis(1)));
        }
        Ok(())
    }

    fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({
            "active": self.active.is_some(),
            "queued": self.queued.len(),
            "encoded_bytes": self.encoded_bytes(),
            "peak_encoded_bytes": self.peak_encoded_bytes,
            "maximum_encoded_bytes": MAX_RETAINED_INPUT_BYTES,
            "started": self.started,
            "completed": self.completed,
            "cancellations": self.cancellations,
            "stale": self.stale,
        })
    }
}

impl Drop for ArticlePreparationState {
    fn drop(&mut self) {
        // A failed bounded lifecycle join cannot make destruction block. The
        // worker owns its payload and credits if its handle must be detached.
        self.cancel();
    }
}

impl App {
    fn article_canonical_geometry(&self) -> (u32, u32) {
        let (width, height) = self.profile.physical_size();
        (width.saturating_sub(128).min(2048), height.min(2048))
    }

    pub(super) fn enqueue_article_preparation(
        &mut self,
        request: u64,
        article_id: &str,
        id: &str,
        index: usize,
        bytes: Vec<u8>,
        credits: Option<JobCredits>,
    ) -> Result<bool> {
        let Some(image) = self.article.image_work.image(index) else {
            drop(bytes);
            drop(credits);
            return Ok(false);
        };
        let context = ArticlePreparationContext {
            request,
            account_epoch: self.jobs.account_epoch(),
            article_id: article_id.to_owned(),
            image_id: id.to_owned(),
            index,
            cache_key: image_processing::article_cache_key(article_id, id, &image.url),
            url: image.url.clone(),
            geometry: self.article_canonical_geometry(),
            generation: 0,
        };
        self.article_preparation.enqueue(context, bytes, credits)
    }

    fn owns_article_preparation_request(&self, context: &ArticlePreparationContext) -> bool {
        !self.article_preparation.exhausted
            && context.generation == self.article_preparation.generation
            && context.account_epoch == self.jobs.account_epoch()
            && context.article_id == self.article.id
            && matches!(&self.route, Route::Article(id) if id == &context.article_id)
            && self.article.image_work.is_ready_for(&context.article_id)
            && self
                .article
                .image_work
                .accepts(&context.image_id, context.request)
                == Some(context.index)
    }

    fn accepts_article_preparation(&self, context: &ArticlePreparationContext) -> bool {
        self.owns_article_preparation_request(context)
            && self
                .article
                .image_work
                .image(context.index)
                .is_some_and(|image| image.url == context.url)
            && self.article_canonical_geometry() == context.geometry
            && image_processing::article_cache_key(
                &context.article_id,
                &context.image_id,
                &context.url,
            ) == context.cache_key
    }

    pub(super) fn poll_article_preparation(&mut self) -> Result<bool> {
        let Some(completion) = self.article_preparation.take_completed() else {
            self.article_preparation.start_next();
            return Ok(false);
        };
        let Completion {
            context,
            result,
            _credits: credits,
        } = completion;
        if !self.accepts_article_preparation(&context) {
            self.article_preparation.stale = self.article_preparation.stale.saturating_add(1);
            let retry = self.owns_article_preparation_request(&context);
            if retry {
                // A changed target geometry or URL must retire its still-owned
                // request rather than leave an undrainable pending image.
                self.pending_article_images.remove(&format!(
                    "article-image:{}:{}",
                    context.article_id, context.image_id
                ));
                self.article.image_work.finish(
                    context.index,
                    context.request,
                    article_images::ArticleImageOutcome::Busy,
                );
            }
            // Both the prepared output and the original request credit are
            // discarded before the next worker is admitted.
            drop(result);
            drop(credits);
            self.article_preparation.start_next();
            return Ok(retry);
        }
        self.pending_article_images.remove(&format!(
            "article-image:{}:{}",
            context.article_id, context.image_id
        ));
        let mut loaded = false;
        let outcome = match result
            .and_then(|prepared| self.cache.put_prepared_png(&context.cache_key, &prepared))
        {
            Ok(path) => {
                self.article
                    .loaded_images
                    .insert(context.image_id.clone(), path);
                loaded = true;
                article_images::ArticleImageOutcome::Success
            }
            Err(_) => {
                self.show_toast("image_load_failed");
                article_images::ArticleImageOutcome::Failed
            }
        };
        self.article
            .image_work
            .finish(context.index, context.request, outcome);
        self.article_preparation.start_next();
        if loaded || self.continue_article_image_work()? {
            self.reflow_article()?;
            let (width, height) = self.platform.logical_viewport_size();
            self.refresh_display(RefreshReason::ImageLoaded(crate::platform::RefreshRect {
                x: 0,
                y: 0,
                width,
                height,
            }));
        }
        // Persistence, payload disposal, and follow-up dispatch all precede
        // admission release, matching the original synchronous event contract.
        drop(credits);
        Ok(true)
    }

    pub(super) fn cancel_article_preparation(&mut self) {
        self.article_preparation.cancel();
        // An interrupted exit or sleep preparation may return to this live
        // document. Retire old request IDs while retaining permitted retries.
        self.article.image_work.pause_pending();
        self.pending_article_images.clear();
    }

    pub(super) fn quiesce_article_preparation(&mut self, timeout: Duration) -> Result<()> {
        self.article.image_work.pause_pending();
        self.pending_article_images.clear();
        self.article_preparation.quiesce(timeout)
    }

    pub(super) fn article_preparation_diagnostics(&self) -> serde_json::Value {
        self.article_preparation.diagnostics()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn png() -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(2, 2, image::Luma([102])))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    fn context(request: u64) -> ArticlePreparationContext {
        ArticlePreparationContext {
            request,
            account_epoch: 0,
            article_id: "article".into(),
            image_id: request.to_string(),
            index: request as usize,
            url: "file:///fixture.png".into(),
            cache_key: "fixture".into(),
            geometry: (2, 2),
            generation: 0,
        }
    }

    #[test]
    fn queue_remains_bounded_until_completed_work_is_drained() {
        let mut state = ArticlePreparationState::default();
        assert!(state.enqueue(context(1), png(), None).unwrap());
        assert!(state.enqueue(context(2), png(), None).unwrap());
        assert!(!state.enqueue(context(3), png(), None).unwrap());
        assert_eq!(state.queued.len(), 1);
        assert!(state.encoded_bytes() <= MAX_RETAINED_INPUT_BYTES);
        state.quiesce(Duration::from_secs(3)).unwrap();
        assert!(!state.is_busy());
    }

    #[test]
    fn cancellation_keeps_active_slot_until_drain_and_fences_old_generation() {
        let mut state = ArticlePreparationState::default();
        assert!(state.enqueue(context(1), png(), None).unwrap());
        assert!(state.enqueue(context(2), png(), None).unwrap());
        state.cancel();
        assert!(state.active.is_some());
        assert!(state.queued.is_empty());
        let started = Instant::now();
        let completion = loop {
            if let Some(completion) = state.take_completed() {
                break completion;
            }
            assert!(started.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(1));
        };
        assert_ne!(completion.context.generation, state.generation);
        assert!(!state.is_busy());
    }

    #[test]
    fn encoded_input_limit_rejects_before_starting_a_worker() {
        let mut state = ArticlePreparationState::default();
        assert!(
            state
                .enqueue(context(1), vec![0; MAX_INPUT_BYTES + 1], None)
                .is_err()
        );
        assert!(!state.is_busy());
        assert_eq!(state.started, 0);
    }
}
