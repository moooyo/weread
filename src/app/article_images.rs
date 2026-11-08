//! Image admission metadata and intents for one current article.
//!
//! This state never owns a DOM or a queue for another article. The controller
//! supplies parsed metadata once, checks capacity before taking a candidate,
//! and remains responsible for cache lookup, transport, and rendered images.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::article::ArticleImage;
use crate::reader::layout_budget::{LayoutLease, LayoutVec};
use crate::reader::{AdmittedLayout, LayoutBudget, LayoutLimitError, LayoutLimitKind};

const MAX_ADMITTED_IMAGES: usize = 2;

/// An operation-local vector whose complete index/copy forecast was admitted
/// before a controller retires the current document or its image intents.
pub(super) struct PreparedImageWork {
    images: Vec<ArticleImage>,
    by_id: HashMap<String, usize>,
    by_url: HashMap<String, usize>,
    admission: LayoutLease,
}

impl PreparedImageWork {
    #[cfg(test)]
    pub(super) fn prepare(
        images: Vec<ArticleImage>,
    ) -> Result<Self, crate::article::ImageLimitError> {
        crate::article::image_work_preflight(&images)?;
        let budget = LayoutBudget::default();
        let bytes = images
            .iter()
            .try_fold(
                images
                    .capacity()
                    .checked_mul(std::mem::size_of::<ArticleImage>())
                    .unwrap_or(usize::MAX),
                |used, image| {
                    used.checked_add(image.id.capacity())
                        .and_then(|used| used.checked_add(image.url.capacity()))
                        .and_then(|used| used.checked_add(image.caption.capacity()))
                },
            )
            .expect("The small image fixture storage fits usize");
        let owner = budget.hold(bytes).expect("Hold the ordinary image fixture");
        let buffer = images.capacity() * std::mem::size_of::<ArticleImage>();
        Ok(
            Self::prepare_admitted(AdmittedLayout::from_vec(images, owner, buffer), &budget)
                .expect("Admit the ordinary image fixture indexes"),
        )
    }

    pub(super) fn prepare_admitted(
        images: AdmittedLayout<Vec<ArticleImage>>,
        budget: &LayoutBudget,
    ) -> Result<Self, crate::article::ArticleParseError> {
        crate::article::image_work_preflight(&images)?;
        let count = images.len();
        let (images, input_owner, input_buffer) = images.into_vec_parts();
        let mut input_owner = budget.adopt_storage(input_owner)?;
        let map_bytes = if count == 0 {
            0
        } else {
            // Reserving count entries needs fewer buckets than this checked
            // next power of two of twice the count. Include control bytes and
            // both complete tables before either allocation or key copy.
            count
                .checked_mul(2)
                .and_then(|count| count.max(4).checked_next_power_of_two())
                .and_then(|buckets| buckets.checked_mul(std::mem::size_of::<(String, usize)>() + 1))
                .and_then(|bytes| {
                    bytes.checked_add(16 + std::mem::size_of::<HashMap<String, usize>>())
                })
                .and_then(|bytes| bytes.checked_mul(2))
                .ok_or(LayoutLimitError {
                    kind: LayoutLimitKind::Metadata,
                    used: 0,
                    requested: usize::MAX,
                    limit: crate::reader::layout_budget::DEFAULT_LAYOUT_METADATA_BYTES,
                })?
        };
        let mut map_owner = budget.copy_storage(map_bytes)?;
        let mut by_id = HashMap::new();
        let mut by_url = HashMap::new();
        if by_id.try_reserve(count).is_err() || by_url.try_reserve(count).is_err() {
            return Err(LayoutLimitError {
                kind: LayoutLimitKind::Allocation,
                used: 0,
                requested: map_bytes,
                limit: crate::reader::layout_budget::DEFAULT_LAYOUT_METADATA_BYTES,
            }
            .into());
        }
        // The original owned image vector, the new vector, both index tables,
        // and every copied ID/URL share one live admission during this move.
        let mut admitted = LayoutVec::with_capacity(budget, count)?;
        for image in images {
            if image.id.is_empty() || by_id.contains_key(&image.id) {
                continue;
            }
            let index = admitted.len();
            let (id, owner) = budget.copy(&image.id)?;
            map_owner.merge(owner);
            by_id.insert(id, index);
            if !by_url.contains_key(&image.url) {
                let (url, owner) = budget.copy(&image.url)?;
                map_owner.merge(owner);
                by_url.insert(url, index);
            }
            admitted.push(image)?;
        }
        input_owner.release(input_buffer);
        admitted.adopt(input_owner);
        admitted.adopt(map_owner);
        let (images, admission) = admitted.into_parts();
        Ok(Self {
            images,
            by_id,
            by_url,
            admission,
        })
    }

    pub(super) fn into_work(
        self,
        article_id: &str,
        budget: &LayoutBudget,
    ) -> Result<ArticleImageWork, LayoutLimitError> {
        let mut admission = budget.adopt_storage(self.admission)?;
        let (article_id, owner) = budget.copy(article_id)?;
        admission.merge(owner);
        Ok(ArticleImageWork {
            article_id,
            ready: true,
            images: self.images,
            by_id: self.by_id,
            by_url: self.by_url,
            layout_admission: Some(admission),
            ..ArticleImageWork::default()
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ArticleImageOutcome {
    Success,
    Failed,
    Busy,
}

#[derive(Clone, Copy)]
struct PendingImage {
    request: u64,
    retry_allowed: bool,
}

#[derive(Default)]
pub(super) struct ArticleImageWork {
    article_id: String,
    ready: bool,
    images: Vec<ArticleImage>,
    by_id: HashMap<String, usize>,
    by_url: HashMap<String, usize>,
    bulk_cursor: Option<usize>,
    explicit: BTreeSet<usize>,
    pending: BTreeMap<usize, PendingImage>,
    failed: BTreeSet<usize>,
    checked: BTreeSet<usize>,
    auto_started: bool,
    layout_admission: Option<LayoutLease>,
}

impl ArticleImageWork {
    pub fn reset(&mut self, article_id: &str) {
        *self = Self {
            article_id: article_id.to_owned(),
            ..Self::default()
        };
    }

    /// Reflow cannot reinstall metadata and reset an existing retry cursor.
    /// A changed document must start a new reset/install generation.
    #[cfg(test)]
    pub fn install(
        &mut self,
        images: Vec<ArticleImage>,
    ) -> Result<(), crate::article::ImageLimitError> {
        if self.ready {
            return Ok(());
        }
        let prepared = PreparedImageWork::prepare(images)?;
        self.install_prepared(prepared);
        Ok(())
    }

    pub(super) fn install_prepared(&mut self, prepared: PreparedImageWork) {
        if self.ready {
            return;
        }
        self.images = prepared.images;
        self.by_id = prepared.by_id;
        self.by_url = prepared.by_url;
        self.layout_admission = Some(prepared.admission);
        self.ready = true;
    }

    pub(super) fn requested_metadata(&self) -> usize {
        self.layout_admission.as_ref().map_or(0, LayoutLease::bytes)
    }

    pub fn is_ready_for(&self, article_id: &str) -> bool {
        self.ready && self.article_id == article_id
    }

    pub fn images(&self) -> &[ArticleImage] {
        &self.images
    }

    pub fn request_all(&mut self, retry: bool) {
        if !self.ready {
            return;
        }
        if retry {
            self.failed.clear();
            self.checked.clear();
        }
        for pending in self.pending.values_mut() {
            pending.retry_allowed = true;
        }
        self.bulk_cursor = (!self.images.is_empty()).then_some(0);
    }

    pub fn request_one(&mut self, id_or_url: &str) -> bool {
        let Some(index) = self.index_for(id_or_url) else {
            return false;
        };
        self.failed.remove(&index);
        self.checked.remove(&index);
        if let Some(pending) = self.pending.get_mut(&index) {
            pending.retry_allowed = true;
        }
        self.explicit.insert(index);
        true
    }

    /// Automatic loading is armed once per document, including after failure,
    /// cancellation, idle ticks, and ordinary layout changes.
    pub fn start_auto(&mut self) {
        if !self.ready || self.auto_started {
            return;
        }
        self.auto_started = true;
        self.request_all(false);
    }

    /// Already admitted requests may still finish after a user cancels intent,
    /// but a late Busy result cannot grant permission for another attempt.
    pub fn cancel_intents(&mut self) {
        self.bulk_cursor = None;
        self.explicit.clear();
        for pending in self.pending.values_mut() {
            pending.retry_allowed = false;
        }
    }

    /// Fence cancelled transport before resume while retaining still-requested
    /// current-document work. A late old request cannot finish a replacement.
    pub fn pause_pending(&mut self) {
        self.explicit.extend(
            self.pending
                .iter()
                .filter_map(|(index, pending)| pending.retry_allowed.then_some(*index)),
        );
        self.pending.clear();
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn has_work(&self) -> bool {
        self.ready && (self.bulk_cursor.is_some() || !self.explicit.is_empty())
    }

    /// Keep a credit until the UI settles its result, not only until network
    /// completion. Queued image payloads still belong to this current document.
    pub fn available_capacity(&self) -> usize {
        MAX_ADMITTED_IMAGES.saturating_sub(self.pending.len())
    }

    /// Call only after transport admission capacity is available. Each bulk
    /// index is consumed once; skips cannot cause an idle rescan of the list.
    pub fn next_candidate(&mut self) -> Option<usize> {
        if !self.ready || self.available_capacity() == 0 {
            return None;
        }
        while let Some(index) = self.explicit.pop_first() {
            if self.selectable(index) {
                return Some(index);
            }
        }
        while let Some(index) = self.bulk_cursor {
            self.bulk_cursor = (index + 1 < self.images.len()).then_some(index + 1);
            if self.selectable(index) {
                return Some(index);
            }
        }
        None
    }

    pub fn image(&self, index: usize) -> Option<&ArticleImage> {
        self.images.get(index)
    }

    pub fn cache_checked(&self, index: usize) -> bool {
        self.checked.contains(&index)
    }

    pub fn mark_cache_checked(&mut self, index: usize) -> bool {
        if index >= self.images.len() {
            return false;
        }
        self.checked.insert(index);
        true
    }

    /// Cache/evidence errors have no network request to finish, but must stop
    /// automatic retry until the user explicitly asks for another attempt.
    pub fn fail(&mut self, index: usize) -> bool {
        if index >= self.images.len() || self.pending.contains_key(&index) {
            return false;
        }
        self.explicit.remove(&index);
        self.failed.insert(index);
        true
    }

    pub fn admit(&mut self, index: usize, request: u64) -> bool {
        if self.available_capacity() == 0 || !self.selectable(index) {
            return false;
        }
        self.explicit.remove(&index);
        self.pending.insert(
            index,
            PendingImage {
                request,
                retry_allowed: true,
            },
        );
        true
    }

    pub fn accepts(&self, image_id: &str, request: u64) -> Option<usize> {
        let index = *self.by_id.get(image_id)?;
        self.pending
            .get(&index)
            .is_some_and(|pending| pending.request == request)
            .then_some(index)
    }

    pub fn request_for(&self, image_id: &str) -> Option<u64> {
        self.pending
            .get(self.by_id.get(image_id)?)
            .map(|pending| pending.request)
    }

    pub fn finish(&mut self, index: usize, request: u64, outcome: ArticleImageOutcome) -> bool {
        let Some(pending) = self
            .pending
            .get(&index)
            .copied()
            .filter(|pending| pending.request == request)
        else {
            return false;
        };
        self.pending.remove(&index);
        self.explicit.remove(&index);
        match outcome {
            ArticleImageOutcome::Success => {
                self.failed.remove(&index);
                self.checked.insert(index);
            }
            ArticleImageOutcome::Failed => {
                self.failed.insert(index);
            }
            ArticleImageOutcome::Busy => {
                if pending.retry_allowed {
                    self.explicit.insert(index);
                }
            }
        }
        true
    }

    fn selectable(&self, index: usize) -> bool {
        index < self.images.len()
            && !self.failed.contains(&index)
            && !self.pending.contains_key(&index)
    }

    fn index_for(&self, id_or_url: &str) -> Option<usize> {
        self.by_id
            .get(id_or_url)
            .or_else(|| self.by_url.get(id_or_url))
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn images(count: usize) -> Vec<ArticleImage> {
        (0..count)
            .map(|index| ArticleImage {
                id: format!("image-{index}"),
                url: format!("https://images.example.test/{index}.png"),
                caption: format!("Fixture caption {index}"),
            })
            .collect()
    }

    fn work(count: usize) -> ArticleImageWork {
        let mut work = ArticleImageWork::default();
        work.reset("article-a");
        work.install(images(count)).unwrap();
        work
    }

    #[test]
    fn queued_results_keep_credits_and_a_full_window_preserves_the_next_candidate() {
        let mut work = work(4);
        work.request_all(false);
        let first = work.next_candidate().unwrap();
        let second = work.next_candidate().unwrap();
        assert!(work.admit(first, 1));
        assert!(work.admit(second, 2));
        assert_eq!(work.available_capacity(), 0);
        for _ in 0..64 {
            assert_eq!(work.next_candidate(), None);
            assert!(!work.admit(2, 99));
        }
        assert!(work.finish(first, 1, ArticleImageOutcome::Success));
        assert_eq!(work.available_capacity(), 1);
        assert_eq!(work.next_candidate(), Some(2));
        assert!(work.admit(2, 3));
        assert_eq!(work.available_capacity(), 0);
        assert!(work.finish(second, 2, ArticleImageOutcome::Busy));
        assert_eq!(work.next_candidate(), Some(second));
        assert!(work.admit(second, 4));
        assert_eq!(work.available_capacity(), 0);
        work.pause_pending();
        assert_eq!(work.available_capacity(), 2);
        assert_eq!(work.accepts("image-1", 4), None);
        assert_eq!(work.accepts("image-2", 3), None);
        let next = work.next_candidate().unwrap();
        assert!(work.admit(next, 5));
        work.reset("replacement");
        assert_eq!(work.available_capacity(), 2);
        assert!(!work.finish(next, 5, ArticleImageOutcome::Failed));
    }

    #[test]
    fn automatic_loading_drains_more_than_two_batches_without_rearming_on_idle_ticks() {
        let mut work = work(8);
        work.start_auto();
        let mut completed = BTreeSet::new();
        for batch in 0..4 {
            let mut active = Vec::new();
            for slot in 0..2 {
                let index = work
                    .next_candidate()
                    .expect("The next admission batch was lost");
                assert!(
                    completed.insert(index),
                    "The bulk cursor revisited a completed image"
                );
                assert!(work.mark_cache_checked(index));
                let request = (batch * 2 + slot + 1) as u64;
                assert!(work.admit(index, request));
                let image = work.image(index).unwrap();
                assert_eq!(work.accepts(&image.id, request), Some(index));
                active.push((index, request));
            }
            for (index, request) in active {
                assert!(work.finish(index, request, ArticleImageOutcome::Success));
            }
        }
        assert_eq!(completed, BTreeSet::from_iter(0..8));
        for _ in 0..1024 {
            work.start_auto();
            assert!(!work.has_work());
            assert_eq!(work.next_candidate(), None);
        }
    }

    #[test]
    fn failed_cache_and_network_images_wait_for_an_explicit_retry() {
        let mut work = work(2);
        work.start_auto();
        assert_eq!(work.next_candidate(), Some(0));
        assert!(work.fail(0));
        assert_eq!(work.next_candidate(), Some(1));
        assert!(work.mark_cache_checked(1));
        assert!(work.admit(1, 10));
        assert!(work.finish(1, 10, ArticleImageOutcome::Failed));
        for _ in 0..64 {
            work.start_auto();
            assert!(!work.has_work());
            assert_eq!(work.next_candidate(), None);
        }
        work.request_all(false);
        assert_eq!(work.next_candidate(), None);
        assert!(!work.has_work());
        assert!(work.request_one("https://images.example.test/1.png"));
        assert!(!work.cache_checked(1));
        assert_eq!(work.next_candidate(), Some(1));
        assert!(work.admit(1, 11));
        assert!(work.finish(1, 11, ArticleImageOutcome::Success));
        assert_eq!(work.next_candidate(), None);
        work.request_all(true);
        assert!(!work.cache_checked(1));
        assert_eq!(work.next_candidate(), Some(0));
        assert_eq!(work.next_candidate(), Some(1));
        assert!(!work.has_work());
    }

    #[test]
    fn busy_defers_one_image_and_an_old_result_cannot_poison_its_replacement() {
        let mut work = work(2);
        work.start_auto();
        assert_eq!(work.next_candidate(), Some(0));
        assert!(work.mark_cache_checked(0));
        assert!(work.admit(0, 20));
        assert!(work.finish(0, 20, ArticleImageOutcome::Busy));
        assert!(work.cache_checked(0));
        assert_eq!(work.request_for("image-0"), None);
        assert_eq!(work.accepts("image-0", 20), None);
        assert_eq!(work.next_candidate(), Some(0));
        assert!(work.admit(0, 21));
        assert!(!work.finish(0, 20, ArticleImageOutcome::Failed));
        assert_eq!(work.accepts("image-0", 21), Some(0));
        assert!(work.finish(0, 21, ArticleImageOutcome::Success));
        assert_eq!(work.next_candidate(), Some(1));
        assert!(work.admit(1, 22));
        assert!(work.finish(1, 22, ArticleImageOutcome::Success));
        assert!(!work.has_work());
    }

    #[test]
    fn paused_requests_resume_only_for_the_current_document_and_use_new_tokens() {
        let mut work = work(4);
        work.start_auto();
        for (index, request) in [(0, 30), (1, 31)] {
            assert_eq!(work.next_candidate(), Some(index));
            assert!(work.admit(index, request));
        }
        work.pause_pending();
        assert_eq!(work.request_for("image-0"), None);
        assert_eq!(work.accepts("image-1", 31), None);
        assert!(!work.finish(0, 30, ArticleImageOutcome::Success));
        for (index, request) in [(0, 32), (1, 33)] {
            assert_eq!(work.next_candidate(), Some(index));
            assert!(work.admit(index, request));
            assert!(work.finish(index, request, ArticleImageOutcome::Success));
        }
        assert_eq!(work.next_candidate(), Some(2));
        assert!(work.admit(2, 34));
        work.reset("article-b");
        assert!(!work.is_ready_for("article-a"));
        assert!(!work.is_ready_for("article-b"));
        assert!(work.images().is_empty());
        assert!(!work.has_work());
        work.install(images(1)).unwrap();
        assert!(work.is_ready_for("article-b"));
        assert_eq!(work.accepts("image-2", 34), None);
        assert!(!work.finish(2, 34, ArticleImageOutcome::Busy));
        assert!(work.request_one("image-0"));
        assert_eq!(work.next_candidate(), Some(0));
        assert!(work.admit(0, 35));
        assert_eq!(work.accepts("image-0", 30), None);
        assert_eq!(work.accepts("image-0", 35), Some(0));
    }

    #[test]
    fn cancelling_intent_preserves_in_flight_completion_but_does_not_restart_auto_loading() {
        let mut work = work(3);
        work.start_auto();
        assert_eq!(work.next_candidate(), Some(0));
        assert!(work.admit(0, 40));
        assert!(work.request_one("image-2"));
        work.cancel_intents();
        assert!(!work.has_work());
        assert_eq!(work.next_candidate(), None);
        assert_eq!(work.accepts("image-0", 40), Some(0));
        assert!(work.finish(0, 40, ArticleImageOutcome::Success));
        work.start_auto();
        assert!(!work.has_work());
        work.request_all(false);
        assert!(work.has_work());
        work.clear();
        assert!(work.images().is_empty());
        assert!(!work.is_ready_for("article-a"));
        assert!(!work.has_work());
        assert_eq!(work.accepts("image-0", 40), None);
    }

    #[test]
    fn a_late_busy_result_cannot_restart_cancelled_image_intent() {
        let mut work = work(1);
        work.start_auto();
        assert_eq!(work.next_candidate(), Some(0));
        assert!(work.admit(0, 60));
        work.cancel_intents();
        assert_eq!(work.accepts("image-0", 60), Some(0));
        assert!(work.finish(0, 60, ArticleImageOutcome::Busy));
        work.start_auto();
        assert!(!work.has_work());
        assert_eq!(work.next_candidate(), None);
    }

    #[test]
    fn a_new_manual_request_can_authorize_busy_retry_after_cancellation() {
        for bulk in [false, true] {
            let mut work = work(1);
            work.start_auto();
            assert_eq!(work.next_candidate(), Some(0));
            assert!(work.admit(0, 70));
            work.cancel_intents();
            if bulk {
                work.request_all(true);
            } else {
                assert!(work.request_one("image-0"));
            }
            assert!(work.finish(0, 70, ArticleImageOutcome::Busy));
            assert_eq!(work.next_candidate(), Some(0));
            assert!(work.admit(0, 71));
            assert!(!work.finish(0, 70, ArticleImageOutcome::Failed));
            assert_eq!(work.accepts("image-0", 71), Some(0));
            assert!(work.finish(0, 71, ArticleImageOutcome::Success));
            assert_eq!(work.request_for("image-0"), None);
        }
    }

    #[test]
    fn pause_does_not_resurrect_cancelled_pending_work() {
        let mut work = work(2);
        work.start_auto();
        for (index, request) in [(0, 80), (1, 81)] {
            assert_eq!(work.next_candidate(), Some(index));
            assert!(work.admit(index, request));
        }
        work.cancel_intents();
        work.pause_pending();
        assert!(!work.has_work());
        assert_eq!(work.next_candidate(), None);
        assert!(!work.finish(0, 80, ArticleImageOutcome::Busy));
        assert_eq!(work.accepts("image-1", 81), None);
        assert!(work.request_one("image-1"));
        assert_eq!(work.next_candidate(), Some(1));
        assert!(work.admit(1, 82));
        work.pause_pending();
        assert_eq!(work.next_candidate(), Some(1));
        assert_eq!(work.accepts("image-1", 82), None);
        assert!(work.admit(1, 83));
    }

    #[test]
    fn repeated_explicit_requests_do_not_duplicate_an_in_flight_image() {
        let mut work = work(1);
        for _ in 0..1024 {
            assert!(work.request_one("image-0"));
            assert!(work.request_one("https://images.example.test/0.png"));
        }
        assert_eq!(work.next_candidate(), Some(0));
        assert!(work.admit(0, 50));
        assert!(work.request_one("image-0"));
        assert_eq!(work.next_candidate(), None);
        assert!(!work.has_work());
        assert_eq!(work.request_for("image-0"), Some(50));
        assert!(!work.admit(0, 51));
        assert!(work.finish(0, 50, ArticleImageOutcome::Success));
    }

    #[test]
    fn reflow_installation_preserves_metadata_and_existing_retry_state() {
        let mut work = work(2);
        work.start_auto();
        assert_eq!(work.next_candidate(), Some(0));
        assert!(work.mark_cache_checked(0));
        assert!(work.fail(0));
        work.install(images(20)).unwrap();
        assert_eq!(work.images().len(), 2);
        assert!(work.cache_checked(0));
        work.start_auto();
        assert_eq!(work.next_candidate(), Some(1));
        assert_eq!(work.next_candidate(), None);
        assert!(!work.has_work());
    }

    #[test]
    fn invalid_indices_and_unknown_ids_cannot_create_work_or_request_records() {
        let mut work = work(1);
        assert_eq!(work.image(usize::MAX), None);
        assert!(!work.mark_cache_checked(usize::MAX));
        assert!(!work.cache_checked(usize::MAX));
        assert!(!work.fail(usize::MAX));
        assert!(!work.admit(usize::MAX, 60));
        assert!(!work.finish(usize::MAX, 60, ArticleImageOutcome::Busy));
        assert!(!work.request_one("unknown"));
        assert_eq!(work.request_for("unknown"), None);
        assert!(!work.has_work());
        assert_eq!(work.next_candidate(), None);
    }
}
