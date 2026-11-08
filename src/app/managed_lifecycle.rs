//! Durable application quiescence while the native process remains resident.

use super::*;
use crate::platform::ManagedSleepState;
use anyhow::ensure;

#[derive(Default)]
pub(super) struct ResidentLifecycle {
    prepared: bool,
    retry_catalogs: BTreeSet<String>,
    retry_books: BTreeSet<String>,
    retry_shelf: bool,
    retry_search: bool,
}

impl ResidentLifecycle {
    /// A completed catalog rejection no longer owns a producer to resume.
    pub(super) fn forget_catalog_retry(&mut self, book_id: &str) {
        self.retry_catalogs.remove(book_id);
    }

    /// Account invalidation clears retry ownership, not the live power handoff.
    pub(super) fn clear_retry_work(&mut self) {
        self.retry_catalogs.clear();
        self.retry_books.clear();
        self.retry_shelf = false;
        self.retry_search = false;
    }
}

#[cfg(test)]
impl ResidentLifecycle {
    pub(super) fn is_prepared(&self) -> bool {
        self.prepared
    }

    pub(super) fn retry_intents(&self) -> (&BTreeSet<String>, &BTreeSet<String>, bool, bool) {
        (
            &self.retry_catalogs,
            &self.retry_books,
            self.retry_shelf,
            self.retry_search,
        )
    }
}

impl App {
    /// Persist local work and stop producers before the backend releases input.
    /// A successful return is only application readiness; the caller must still
    /// wait for the renderer's input/display acknowledgement before freezing.
    pub fn prepare_resident_sleep(&mut self) -> Result<()> {
        let prepared = self.prepare_resident_sleep_inner();
        self.preserve_submission_error(prepared)?;
        self.resident_lifecycle.prepared = true;
        Ok(())
    }

    fn prepare_resident_sleep_inner(&mut self) -> Result<()> {
        ensure!(!self.closing.get(), "the application is already closing");
        self.managed_sleep_blocked = true;
        let writing_thought = self
            .ui
            .upgrade()
            .is_some_and(|ui| ui.global::<ReaderModel>().get_writing_thought());
        let transient_selection = self.reader.press.is_some() && !writing_thought;
        if transient_selection {
            self.reader.selection = None;
            if let Some(ui) = self.ui.upgrade() {
                ui.global::<ReaderModel>().set_selection_open(false);
            }
        }
        self.invalidate_reader_press();
        self.reader.suppress_tap = false;
        if self.resident_lifecycle.prepared {
            return Ok(());
        }
        if transient_selection && matches!(self.route, Route::Reader(..)) {
            // The retained Slint model must also lose the transient underline,
            // so the backend's first restored frame cannot display a ghost.
            self.render_reader()?;
        }

        // A cancelled task retains its stopping guard until it has joined.
        // Include that guard before acknowledging native power ownership.
        self.join_task_wake_guard(task_wake::STOP_TIMEOUT)?;

        self.save_reader_session()?;
        self.save_preferences()?;
        let force_offline = self.force_offline;
        self.force_offline = true;
        let reading_result = self.finish_reading();
        self.force_offline = force_offline;
        reading_result?;
        // Reconcile acknowledged deliveries and retain uncertain POSTs rather
        // than replaying them after the network futures are cancelled.
        self.interrupt_resident_read_reports()?;
        self.pause_resident_download()?;
        self.persist_session_updates()?;

        self.reconcile_resident_read_retries(true);

        // Preserve the shared authenticated client, but fence every queued old
        // result, including late catalog/progress responses that could move UI.
        self.jobs.cancel_all();
        self.discard_cancelled_job_events();
        let invalid_request = self.request();
        self.shelf_request = invalid_request;
        self.search_request = invalid_request;
        self.review_request = invalid_request;
        self.stats_request = invalid_request;
        self.login_request = invalid_request;
        self.download_request = invalid_request;
        self.reader.underline_request = invalid_request;
        self.reader.thought_request = invalid_request;
        self.catalog_requests.clear();
        self.book_requests.clear();
        self.pending_covers.clear();
        self.pending_articles.clear();
        self.pending_article_images.clear();
        self.article.image_work.pause_pending();
        self.download_ui_changes.clear();
        self.busy = false;
        self.login_polling = false;
        if self.resident_lifecycle.retry_shelf {
            self.library_status.clear();
        }
        if self.search_status == tr("searching") {
            self.search_status.clear();
        }
        if self.mp_status == tr("synchronizing") {
            self.mp_status.clear();
        }
        if self.settings_status == tr("synchronizing") {
            self.settings_status.clear();
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<LoginModel>().set_busy(false);
        }

        // This performs bounded joins for both lighting and battery workers.
        // A failure leaves the input guard set and never acknowledges readiness.
        self.stop_lighting_hardware()?;
        self.platform.quiesce_device_commands(Duration::ZERO)?;
        self.persist_session_updates()?;
        Ok(())
    }

    fn pause_resident_download(&mut self) -> Result<()> {
        let Some(previous) = self.current_download.take() else {
            return Ok(());
        };
        if previous.status != DownloadStatus::Running {
            return Ok(());
        }
        let mut paused = previous.clone();
        paused.status = DownloadStatus::Paused;
        paused.updated_at = Utc::now().timestamp();
        match self.store.save_task_if_current(&previous, &paused) {
            Ok(true) => {
                self.downloads.retain(|task| task.id != paused.id);
                self.downloads.push_front(paused);
            }
            // A cancellation, account change or newer plan wins over this
            // snapshot. Do not revive it through the resident resume queue.
            Ok(false) => self.downloads.retain(|task| task.id != previous.id),
            Err(error) => {
                self.current_download = Some(previous);
                return Err(error);
            }
        }
        Ok(())
    }

    /// Restart application work only after native input and its retained frame
    /// are restored. The live route, paginator, font data and drafts stay intact.
    pub fn resume_resident_sleep(&mut self) -> Result<()> {
        let resumed = self.resume_resident_sleep_inner();
        self.preserve_submission_error(resumed)?;
        self.resident_lifecycle.prepared = false;
        self.managed_sleep_blocked = false;
        Ok(())
    }

    fn resume_resident_sleep_inner(&mut self) -> Result<()> {
        ensure!(!self.closing.get(), "the application is already closing");
        if !self.managed_sleep_blocked {
            return Ok(());
        }
        ensure!(
            self.resident_lifecycle.prepared,
            "resident sleep preparation did not complete"
        );
        ensure!(
            self.platform.managed_sleep_state()? == ManagedSleepState::Running,
            "the native display and input are not restored"
        );

        let now = Instant::now();
        self.last_tick = now;
        self.reader.last_active = now;
        self.last_login_poll = now;
        self.device_poll_at = now + Duration::from_secs(60);
        self.download_ui_refresh_at = now;
        self.resume_reader_after_failed_exit();
        self.request_device_status();
        self.restart_resident_reads()?;
        self.start_next_download()?;
        self.render_clock();
        if matches!(self.route, Route::Reader(..)) {
            self.render_lighting();
            if let Some(ui) = self.ui.upgrade() {
                ui.global::<ReaderModel>().set_download_busy(
                    self.current_download.as_ref().is_some_and(|task| {
                        task.book_id == self.reader.book_id
                            && task.chapters.contains(&self.reader.chapter_index)
                    }),
                );
            }
        } else {
            self.render()?;
        }
        Ok(())
    }

    fn restart_resident_reads(&mut self) -> Result<()> {
        // Offline wake must discard stale page identities too. Returning before
        // this projection would accumulate every cancelled page across sleeps.
        self.reconcile_resident_read_retries(false);
        if self.demo || self.force_offline || !self.online || !self.jobs.configured() {
            return Ok(());
        }
        let catalogs = std::mem::take(&mut self.resident_lifecycle.retry_catalogs);
        for id in catalogs {
            let visible_detail = matches!(&self.route, Route::Detail(visible) if visible == &id);
            let empty_reader = self.reader.body.is_empty()
                && matches!(&self.route, Route::Reader(visible, _) if visible == &id)
                && self
                    .pending_read
                    .as_ref()
                    .is_some_and(|(book, _)| book == &id);
            if self.pending_catalog_downloads.contains(&id) || visible_detail || empty_reader {
                self.request_catalog(&id);
            }
        }
        let books = std::mem::take(&mut self.resident_lifecycle.retry_books);
        for id in books {
            if matches!(&self.route, Route::Detail(visible) if visible == &id) {
                let request = self.request();
                self.book_requests.insert(id.clone(), request);
                self.settle_job_submission(self.jobs.book_info(request, id));
            }
        }
        if std::mem::take(&mut self.resident_lifecycle.retry_shelf) && self.route == Route::Library
        {
            self.sync_shelf();
        }
        if std::mem::take(&mut self.resident_lifecycle.retry_search)
            && self.route == Route::Search
            && self.search_purpose == "books"
            && self.search_scope != "shelf"
            && self.jobs.capabilities().search
        {
            let request = self.request();
            self.search_request = request;
            self.search_status = tr("searching");
            self.settle_job_submission(
                self.jobs
                    .search(request, self.query.query.trim().to_owned()),
            );
        }
        if let Route::Article(id) = self.route.clone() {
            if self.article.body.is_empty() {
                self.request_article(&id)?;
            }
        }
        Ok(())
    }

    /// Keep metadata retries scoped to current UI and explicit acquisition intent.
    fn reconcile_resident_read_retries(&mut self, snapshot_pending: bool) {
        let detail = match &self.route {
            Route::Detail(id) => Some(id.as_str()),
            _ => None,
        };
        let empty_reader = match (&self.route, self.pending_read.as_ref()) {
            (Route::Reader(id, _), Some((pending, _)))
                if self.reader.body.is_empty() && id == pending =>
            {
                Some(id.as_str())
            }
            _ => None,
        };
        let explicit_catalogs = &self.pending_catalog_downloads;
        let wants_catalog = |id: &String| {
            explicit_catalogs.contains(id)
                || detail == Some(id.as_str())
                || empty_reader == Some(id.as_str())
        };
        let wants_book = |id: &String| detail == Some(id.as_str());
        let lifecycle = &mut self.resident_lifecycle;

        // Prune the previous snapshot before adding current in-flight work.
        // Download intent is held by its own existing application collections.
        lifecycle.retry_catalogs.retain(|id| wants_catalog(id));
        lifecycle.retry_books.retain(|id| wants_book(id));
        if snapshot_pending {
            lifecycle.retry_catalogs.extend(
                self.catalog_requests
                    .keys()
                    .filter(|id| wants_catalog(id))
                    .cloned(),
            );
            lifecycle
                .retry_catalogs
                .extend(explicit_catalogs.iter().cloned());
            lifecycle.retry_books.extend(
                self.book_requests
                    .keys()
                    .filter(|id| wants_book(id))
                    .cloned(),
            );
            lifecycle.retry_shelf |= self.busy;
            lifecycle.retry_search |= self.search_status == tr("searching");
        }
        lifecycle.retry_shelf &= self.route == Route::Library;
        lifecycle.retry_search &= self.route == Route::Search
            && self.search_purpose == "books"
            && self.search_scope != "shelf"
            && !self.query.query.trim().is_empty();
    }
}
