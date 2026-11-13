use super::article_images::ArticleImageOutcome;
use super::*;
use crate::jobs::JobResult;
use slint::Model;
use std::collections::HashMap;

const MAX_CATALOG_REINDEX_ENTRIES: usize = source::content::MAX_CATALOG_CHAPTERS;
// A separate logical fee covers hash buckets and their control bytes. It is
// added to the existing catalog/owner allowance before either map is built.
const CATALOG_REINDEX_METADATA_PER_ENTRY: usize = 128;
const MAX_CATALOG_REINDEX_METADATA_BYTES: usize =
    2 * MAX_CATALOG_REINDEX_ENTRIES * CATALOG_REINDEX_METADATA_PER_ENTRY;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CatalogReindexCounters {
    builds: usize,
    new_uid_visits: usize,
    old_uid_visits: usize,
    uid_lookups: usize,
    index_lookups: usize,
    new_index_lookups: usize,
}

#[cfg(test)]
std::thread_local! {
    static CATALOG_REINDEX_COUNTERS: Cell<CatalogReindexCounters> =
        Cell::new(CatalogReindexCounters::default());
    // Small fixtures may tighten the real pre-map admission without allocating
    // a large catalog. They cannot raise the production entry-derived ceiling.
    static CATALOG_REINDEX_METADATA_ALLOWANCE: Cell<usize> =
        const { Cell::new(MAX_CATALOG_REINDEX_METADATA_BYTES) };
}

fn catalog_reindex_metadata_bytes(old_count: usize, new_count: usize) -> Result<usize> {
    if old_count > MAX_CATALOG_REINDEX_ENTRIES || new_count > MAX_CATALOG_REINDEX_ENTRIES {
        return Err(source::SourceError::ResponseTooLarge.into());
    }
    let bytes = old_count
        .checked_add(new_count)
        .and_then(|count| count.checked_mul(CATALOG_REINDEX_METADATA_PER_ENTRY))
        .ok_or(source::SourceError::ResponseTooLarge)?;
    let maximum = MAX_CATALOG_REINDEX_METADATA_BYTES;
    #[cfg(test)]
    let maximum = maximum.min(CATALOG_REINDEX_METADATA_ALLOWANCE.with(Cell::get));
    if bytes > maximum {
        return Err(source::SourceError::ResponseTooLarge.into());
    }
    Ok(bytes)
}

pub(super) struct CoverRequestContext {
    request: u64,
    account_epoch: u64,
    url: String,
}

/// One admitted catalog operation. UID keys borrow the immutable new rows and
/// are discarded after resolving old indices; every consumer then uses numbers.
struct CatalogReindex<'a> {
    chapters: &'a [Chapter],
    old_to_new: HashMap<u32, Option<&'a Chapter>>,
}

impl<'a> CatalogReindex<'a> {
    fn new(old: &BTreeMap<String, Chapter>, chapters: &'a [Chapter]) -> Result<Self> {
        // The App also adds this fee to its catalog/owner byte allowance. Keep
        // the local entry gate before with_capacity for every constructor call.
        catalog_reindex_metadata_bytes(old.len(), chapters.len())?;
        #[cfg(test)]
        CATALOG_REINDEX_COUNTERS.with(|slot| {
            let mut counters = slot.get();
            counters.builds += 1;
            slot.set(counters);
        });
        let mut new_by_uid = HashMap::with_capacity(chapters.len());
        for chapter in chapters {
            #[cfg(test)]
            CATALOG_REINDEX_COUNTERS.with(|slot| {
                let mut counters = slot.get();
                counters.new_uid_visits += 1;
                slot.set(counters);
            });
            // Incoming duplicate UIDs are rejected by the borrowed source gate.
            // Preserve first-match semantics for the private mapping helper too.
            new_by_uid.entry(chapter.id.as_str()).or_insert(chapter);
        }
        let mut old_to_new = HashMap::with_capacity(old.len());
        for previous in old.values() {
            #[cfg(test)]
            CATALOG_REINDEX_COUNTERS.with(|slot| {
                let mut counters = slot.get();
                counters.old_uid_visits += 1;
                counters.uid_lookups += 1;
                slot.set(counters);
            });
            let target = new_by_uid.get(previous.id.as_str()).copied();
            old_to_new.entry(previous.index).or_insert(target);
        }
        Ok(Self {
            chapters,
            old_to_new,
        })
    }

    /// A missing/deleted original UID cannot authorize a resident Reader or
    /// history entry, even when an empty old catalog permits pending fallback.
    fn retained_chapter(&self, index: u32) -> Option<&'a Chapter> {
        #[cfg(test)]
        CATALOG_REINDEX_COUNTERS.with(|slot| {
            let mut counters = slot.get();
            counters.index_lookups += 1;
            slot.set(counters);
        });
        self.old_to_new.get(&index).copied().flatten()
    }

    fn reindex(&self, index: u32) -> Option<u32> {
        if self.old_to_new.is_empty() {
            self.chapter(index).map(|chapter| chapter.index)
        } else {
            self.retained_chapter(index).map(|chapter| chapter.index)
        }
    }

    /// The producer enumerates new rows into contiguous indices after source
    /// admission. Check the stored index too rather than guessing a target.
    fn chapter(&self, index: u32) -> Option<&'a Chapter> {
        #[cfg(test)]
        CATALOG_REINDEX_COUNTERS.with(|slot| {
            let mut counters = slot.get();
            counters.new_index_lookups += 1;
            slot.set(counters);
        });
        self.chapters
            .get(usize::try_from(index).ok()?)
            .filter(|chapter| chapter.index == index)
    }
}

impl App {
    pub(super) fn handle_job(&mut self, event: JobEvent) -> Result<bool> {
        if !self.jobs.accepts(&event) {
            return Ok(false);
        }
        // Keep all admitted credits alive through persistence, decoding and
        // follow-up dispatch, not merely until the mailbox item is popped.
        let JobEvent {
            request,
            result,
            credits: _credits,
            ..
        } = event;
        let mut render = true;
        match result {
            JobResult::Shelf(result) => {
                if request != self.shelf_request {
                    return Ok(false);
                }
                self.busy = false;
                match result {
                    Ok(snapshot) => {
                        self.source_available();
                        let existing = self.store.books()?;
                        let overrides = {
                            let inventory = self.store.resource_verification_snapshot()?;
                            let headers = inventory
                                .setting_inventory_headers_with_prefix("shelf.override:")?;
                            let mut overrides = BTreeMap::new();
                            for header in &headers {
                                let value = inventory.bounded_setting_inventory_bool(header)?;
                                let mut key = inventory.bounded_setting_inventory_key(header)?;
                                // Keep the original repeated-prefix semantics while
                                // retaining only one owned key for the final suffix.
                                let removed =
                                    key.len() - key.trim_start_matches("shelf.override:").len();
                                drop(key.drain(..removed));
                                overrides.insert(key, value);
                            }
                            overrides
                        };
                        let books = library::merge_shelf(&snapshot.books, &existing, &overrides);
                        for remote in &snapshot.books {
                            self.store.set_setting(
                                &format!("book.updated_at:{}", remote.id),
                                &remote.updated_at,
                            )?;
                        }
                        self.store.replace_shelf(&books)?;
                        self.selected_books
                            .retain(|id| books.iter().any(|book| &book.id == id));
                        self.mp_accounts = snapshot.mp_accounts;
                        self.store.set_setting("mp.accounts", &self.mp_accounts)?;
                        let time = Utc::now()
                            .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
                            .format("%H:%M")
                            .to_string();
                        self.sync_time = i18n::format("sync_time", &[("time", time)]);
                        self.store.set_setting("sync.time", &self.sync_time)?;
                        self.library_status.clear();
                        self.start_next_download()?;
                    }
                    Err(error) => {
                        self.library_status = tr("sync_failed");
                        self.job_error(&error, true)?;
                    }
                }
            }
            JobResult::Search(result) => {
                if request != self.search_request
                    || self.route != Route::Search
                    || self.search_scope != "store"
                    || self.search_purpose != "books"
                {
                    return Ok(false);
                }
                match result {
                    Ok(remote_books) => {
                        self.source_available();
                        let mut books = Vec::new();
                        for remote in remote_books {
                            let previous = self.store.book(&remote.id)?;
                            let on_shelf = previous.as_ref().is_some_and(|book| book.on_shelf);
                            let book = library::normalize_book(&remote, previous, on_shelf);
                            if book.id.is_empty() {
                                continue;
                            }
                            self.store.upsert_book(&book)?;
                            books.push(BookMetadata::from(&book));
                        }
                        self.search_results = books;
                        self.search_page = 0;
                        self.search_status = if self.search_results.is_empty() {
                            tr("no_results")
                        } else {
                            String::new()
                        };
                    }
                    Err(error) => {
                        self.search_status = tr("api_error");
                        self.job_error(&error, true)?;
                    }
                }
            }
            JobResult::BookInfo { book_id, result } => {
                if self.book_requests.get(&book_id) != Some(&request) {
                    return Ok(false);
                }
                self.book_requests.remove(&book_id);
                match result {
                    Ok(mut remote) => {
                        if !remote.id.is_empty() && remote.id != book_id {
                            return Ok(false);
                        }
                        self.source_available();
                        remote.id = book_id.clone();
                        let previous = self.store.book(&book_id)?;
                        if previous
                            .as_ref()
                            .is_some_and(|book| book.source != "weread")
                        {
                            return Ok(true);
                        }
                        let on_shelf = previous.as_ref().is_some_and(|book| book.on_shelf);
                        let book = library::normalize_book(&remote, previous, on_shelf);
                        self.store.upsert_book(&book)?;
                        self.store
                            .set_setting(&format!("book.metadata:{book_id}"), &remote.raw)?;
                        self.store.set_setting(
                            &format!("book.updated_at:{book_id}"),
                            &remote.updated_at,
                        )?;
                        if self.detail_id == book_id {
                            self.detail_status.clear();
                        }
                    }
                    Err(error)
                        if self.detail_id == book_id
                            && matches!(&self.route, Route::Detail(id) if id == &book_id) =>
                    {
                        self.detail_status = tr("api_error");
                        self.job_error(&error, true)?;
                    }
                    Err(_) => {}
                }
            }
            JobResult::Catalog { book_id, result } => {
                if self.catalog_requests.get(&book_id) != Some(&request) {
                    return Ok(false);
                }
                self.catalog_requests.remove(&book_id);
                self.catalog_result(&book_id, result)?;
            }
            JobResult::Reviews { book_id, result } => {
                let Some(cache_key) = self
                    .requested_review_cache_key(request, &book_id)
                    .map(str::to_owned)
                else {
                    return Ok(false);
                };
                match result {
                    Ok(reviews) => {
                        self.source_available();
                        self.store.set_setting(&cache_key, &reviews)?;
                        self.reviews = reviews;
                        self.detail_status = if self.reviews.is_empty() {
                            tr("no_reviews")
                        } else {
                            String::new()
                        };
                    }
                    Err(error) => {
                        self.detail_status = tr("api_error");
                        self.job_error(&error, true)?;
                    }
                }
            }
            JobResult::Progress { book_id, result } => {
                let key = format!("progress.request:{book_id}");
                if self.store.setting::<u64>(&key)? != Some(request) {
                    return Ok(false);
                }
                self.store.delete_setting(&key)?;
                if self.reader.book_id != book_id
                    || !matches!(&self.route, Route::Reader(id, _) if id == &book_id)
                {
                    return Ok(false);
                }
                match result {
                    Ok(position) => self.cloud_position(&book_id, position)?,
                    Err(error) => self.job_error(&error, false)?,
                }
            }
            JobResult::UploadProgress(result) => match result {
                Ok(()) => {
                    self.source_available();
                    self.store
                        .set_setting("progress.uploaded_at", &Utc::now().timestamp())?;
                    self.show_toast("upload_success");
                }
                Err(error) => self.job_error(&error, false)?,
            },
            JobResult::ReadReport { session_id, result } => {
                let stored = match reporting::store_report_result(&self.store, &session_id, &result)
                {
                    Ok(stored) => stored,
                    Err(error) if reporting::report_capacity(&error) => {
                        self.note_read_report_error(&error);
                        return Ok(false);
                    }
                    Err(error) => return Err(error),
                };
                if !stored {
                    return Ok(false);
                }
                match result {
                    Ok(_) => self.source_available(),
                    Err(source::SourceError::Network | source::SourceError::Timeout) => {
                        self.online = false;
                    }
                    Err(_) => {}
                }
            }
            JobResult::MpArticles { account_id, result } => match result {
                Ok(articles) => {
                    self.source_available();
                    let previous = self.mp_articles.remove(&account_id).unwrap_or_default();
                    let mut seen = BTreeSet::new();
                    let mut merged = Vec::new();
                    for article in articles.into_iter().chain(previous) {
                        if !article.id.is_empty() && seen.insert(article.id.clone()) {
                            merged.push(article);
                        }
                    }
                    merged.sort_by_key(|article| std::cmp::Reverse(article.published_at));
                    let prefetch = merged
                        .iter()
                        .take(3)
                        .map(|article| article.id.clone())
                        .collect::<Vec<_>>();
                    self.mp_articles.insert(account_id.clone(), merged);
                    self.store.set_setting("mp.articles", &self.mp_articles)?;
                    if self.mp_account.as_deref() == Some(account_id.as_str()) {
                        self.mp_status.clear();
                    }
                    for id in prefetch {
                        self.request_article(&id)?;
                    }
                }
                Err(error) => {
                    if self.mp_account.as_deref() == Some(account_id.as_str()) {
                        self.mp_status.clear();
                    }
                    self.job_error(&error, true)?;
                }
            },
            JobResult::Article { article_id, result } => {
                let completed = self.pending_article_requests.get(&article_id) == Some(&request);
                let outcome = match result {
                    Ok(content) => self.article_result(request, &article_id, content),
                    Err(error) => {
                        if self.article.id == article_id
                            && matches!(&self.route, Route::Article(id) if id == &article_id)
                        {
                            self.job_error(&error, true).map(|_| true)
                        } else {
                            Ok(false)
                        }
                    }
                };
                // Only this captured completed request is retired. A quota or
                // SQL failure cannot leave its source intent permanently busy.
                if completed && self.pending_article_requests.get(&article_id) == Some(&request) {
                    self.pending_article_requests.remove(&article_id);
                    self.pending_articles.remove(&article_id);
                }
                render = outcome?;
            }
            JobResult::Statistics(result) => {
                if request != self.stats_request {
                    return Ok(false);
                }
                match result {
                    Ok(statistics) => {
                        self.source_available();
                        self.store.set_setting(
                            &format!("stats.remote:{}:{}", self.stats_period, self.stats_offset),
                            &statistics,
                        )?;
                        self.remote_stats = Some(statistics);
                    }
                    Err(error) => self.job_error(&error, false)?,
                }
            }
            JobResult::Renew(result) => match result {
                Ok(session) => {
                    let Some(previous) = self.jobs.session() else {
                        return Ok(false);
                    };
                    if !session.user_vid.is_empty()
                        && !previous.user_vid.is_empty()
                        && session.user_vid != previous.user_vid
                    {
                        return Ok(false);
                    }
                    // The shared client may already hold newer response cookies than
                    // this queued renewal event. Persist only its current snapshot.
                    self.persist_session_updates()?;
                    self.source_available();
                    self.settings_status = tr("renew_success");
                    self.show_toast("renew_success");
                }
                Err(error) => {
                    self.settings_status = tr("operation_failed");
                    self.job_error(&error, true)?;
                }
            },
            JobResult::Cover { book_id, result } => {
                self.cover_result(request, &book_id, result)?;
                render = false;
            }
            JobResult::ArticleImage {
                article_id,
                image_id,
                result,
            } => {
                render = self.article_image_result(request, &article_id, &image_id, result)?;
            }
            JobResult::LoginBegin(result) => {
                if request != self.login_request || self.route != Route::Login {
                    return Ok(false);
                }
                self.login_polling = false;
                match result {
                    Ok(challenge) => {
                        self.source_available();
                        let qr = library::qr_image(&challenge.confirm_url)?;
                        self.login_challenge = Some(challenge);
                        self.last_login_poll = Instant::now() - Duration::from_secs(2);
                        if let Some(ui) = self.ui.upgrade() {
                            let login = ui.global::<LoginModel>();
                            login.set_qr(qr);
                            login.set_step("qr".into());
                            login.set_busy(false);
                            login.set_status(tr("login_scan").into());
                        }
                    }
                    Err(error) => {
                        self.login_challenge = None;
                        self.login_failure(&error);
                    }
                }
            }
            JobResult::LoginPoll(result) => {
                if request != self.login_request
                    || self.route != Route::Login
                    || self.login_challenge.is_none()
                {
                    return Ok(false);
                }
                self.login_polling = false;
                self.last_login_poll = Instant::now();
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<LoginModel>().set_busy(false);
                }
                match result {
                    Ok(source::QrLoginState::Pending) => {}
                    Ok(source::QrLoginState::NeedCode) => {
                        if let Some(ui) = self.ui.upgrade() {
                            let login = ui.global::<LoginModel>();
                            if login.get_step() != "code" {
                                self.login_code.clear();
                            }
                            login.set_step("code".into());
                            login.set_status(tr("login_code_hint").into());
                        }
                    }
                    Ok(source::QrLoginState::CodeMismatch) => {
                        if let Some(ui) = self.ui.upgrade() {
                            let login = ui.global::<LoginModel>();
                            login.set_step("code".into());
                            login.set_status(tr("login_code_mismatch").into());
                        }
                    }
                    Ok(source::QrLoginState::Expired) => {
                        self.login_challenge = None;
                        self.jobs.cancel_login();
                        if let Some(ui) = self.ui.upgrade() {
                            let login = ui.global::<LoginModel>();
                            login.set_step("qr".into());
                            login.set_status(tr("login_expired").into());
                            login.set_qr_label(tr("login_expired").into());
                        }
                    }
                    Ok(source::QrLoginState::Authenticated(session)) => {
                        self.login_complete(session)?
                    }
                    Err(error) => self.login_failure(&error),
                }
            }
            JobResult::Underlines {
                book_id,
                chapter_id,
                result,
            } => {
                if request != self.reader.underline_request {
                    return Ok(false);
                }
                match result {
                    Ok(underlines) => {
                        self.store.set_setting(
                            &format!("underlines:{book_id}:{chapter_id}"),
                            &underlines,
                        )?;
                        if self.prefs.popular && self.reader_matches(&book_id, &chapter_id)? {
                            self.reader.replace_underlines(underlines);
                            self.render_reader()?;
                        }
                    }
                    Err(error) if self.reader_matches(&book_id, &chapter_id)? => {
                        self.job_error(&error, false)?
                    }
                    Err(_) => {}
                }
            }
            JobResult::Thoughts {
                book_id,
                chapter_id,
                result,
            } => {
                if !self.reader_matches(&book_id, &chapter_id)? {
                    return Ok(false);
                }
                let Some(cache_key) = self
                    .requested_thought_cache_key(request, &book_id, &chapter_id)
                    .map(str::to_owned)
                else {
                    return Ok(false);
                };
                if !self
                    .ui
                    .upgrade()
                    .is_some_and(|ui| ui.global::<ReaderModel>().get_thought_open())
                {
                    return Ok(false);
                }
                match result {
                    Ok(reviews) => {
                        self.store.set_setting(&cache_key, &reviews)?;
                        self.reader.remote_thoughts = reviews;
                        self.reader.thought_page = 0;
                        self.render_reader()?;
                    }
                    Err(error) => self.job_error(&error, false)?,
                }
            }
            JobResult::Downloaded {
                task_id,
                chapter,
                next_index,
                html,
                metadata,
                images,
                checkpoint,
            } => {
                if request != self.download_request
                    || !self.valid_download_checkpoint(&task_id, &chapter, next_index)
                {
                    let _ = checkpoint.send(Err(source::SourceError::CheckpointFailed));
                    return Ok(false);
                }
                let mut committed = false;
                let mut complete_images = true;
                let result = (|| -> Result<bool> {
                    self.store.validate_independent_download_chapter(
                        self.current_download.as_ref().unwrap(),
                        &chapter,
                        next_index - 1,
                    )?;
                    let publication = match (html, metadata) {
                        (Some(html), Some(metadata)) => {
                            let text = chapter.body.as_deref().ok_or_else(|| {
                                anyhow::anyhow!("A completed chapter packet has no body")
                            })?;
                            self.book_assets.commit_prepared_download_chapter_metadata(
                                &self.store,
                                &chapter.book_id,
                                &chapter.id,
                                &html,
                                text,
                                &metadata,
                                images.as_ref(),
                            )?
                        }
                        (None, None) if images.is_none() => None,
                        _ => anyhow::bail!("A chapter packet has inconsistent prepared metadata"),
                    };
                    complete_images = !self.current_download.as_ref().unwrap().include_images
                        || chapter.content_status != ContentStatus::Full
                        || publication.is_some();
                    self.downloaded_chapter(
                        &task_id,
                        chapter,
                        next_index,
                        publication,
                        &mut committed,
                    )
                })();
                match result {
                    Ok(visible) => {
                        render = visible;
                        let _ = checkpoint.send(if complete_images {
                            Ok(())
                        } else {
                            Err(source::SourceError::ContentUnverified)
                        });
                    }
                    Err(error) => {
                        if committed {
                            // The worker owns a durable checkpoint already.
                            // A later display/session error keeps its cause for
                            // the caller but cannot authorize packet replay.
                            let _ = checkpoint.send(if complete_images {
                                Ok(())
                            } else {
                                Err(source::SourceError::ContentUnverified)
                            });
                            crate::diagnostic!("Post-commit chapter follow-up failed: {error:#}");
                            self.show_toast("operation_failed");
                            return Err(error);
                        }
                        let conflict = error
                            .downcast_ref::<crate::store::ContentConflict>()
                            .is_some();
                        let _ = checkpoint.send(Err(if conflict {
                            source::SourceError::ContentChanged
                        } else {
                            source::SourceError::CheckpointFailed
                        }));
                        self.show_toast(if conflict {
                            "content_changed"
                        } else {
                            "operation_failed"
                        });
                    }
                }
            }
            JobResult::CachedDownloaded {
                task_id,
                chapter_id,
                chapter_index,
                next_index,
                checkpoint,
            } => {
                if request != self.download_request
                    || !self.valid_cached_download_checkpoint(&task_id, chapter_index, next_index)
                {
                    let _ = checkpoint.send(Err(source::SourceError::CheckpointFailed));
                    return Ok(false);
                }
                match self.cached_downloaded_chapter(
                    &task_id,
                    &chapter_id,
                    chapter_index,
                    next_index,
                ) {
                    Ok(visible) => {
                        render = visible;
                        let _ = checkpoint.send(Ok(()));
                    }
                    Err(error) => {
                        let conflict = error
                            .downcast_ref::<crate::store::ContentConflict>()
                            .is_some()
                            || matches!(
                                error.downcast_ref::<source::SourceError>(),
                                Some(source::SourceError::ContentChanged)
                            );
                        let _ = checkpoint.send(Err(if conflict {
                            source::SourceError::ContentChanged
                        } else {
                            source::SourceError::CheckpointFailed
                        }));
                        self.show_toast(if conflict {
                            "content_changed"
                        } else {
                            "operation_failed"
                        });
                        render = false;
                    }
                }
            }
            JobResult::DownloadChapterStarted {
                task_id,
                chapter_id,
                chapter_index,
                ordinal,
                checkpoint,
            } => {
                render = false;
                if request != self.download_request
                    || !self.valid_cached_download_checkpoint(
                        &task_id,
                        chapter_index,
                        ordinal.saturating_add(1),
                    )
                {
                    let _ = checkpoint.send(Err(source::SourceError::CheckpointFailed));
                    return Ok(false);
                }
                let updated_at = Utc::now().timestamp();
                match self.store.start_download_chapter_attempt(
                    self.current_download.as_ref().unwrap(),
                    &chapter_id,
                    chapter_index,
                    ordinal,
                    updated_at,
                ) {
                    Ok(progress) => {
                        let task = self.current_download.as_mut().unwrap();
                        task.updated_at = updated_at;
                        let task = task.clone();
                        let _ = checkpoint.send(Ok(()));
                        self.download_feedback(
                            task.completed_count,
                            task.chapters.len(),
                            progress.retry_wait_count,
                            progress.blocked_count,
                        );
                        self.remember_download_notice(&task)?;
                    }
                    Err(error) => {
                        let conflict = error
                            .downcast_ref::<crate::store::ContentConflict>()
                            .is_some();
                        let _ = checkpoint.send(Err(if conflict {
                            source::SourceError::ContentChanged
                        } else {
                            source::SourceError::CheckpointFailed
                        }));
                        self.show_toast(if conflict {
                            "content_changed"
                        } else {
                            "operation_failed"
                        });
                        crate::diagnostic!("Start chapter retry failed: {error:#}");
                    }
                }
            }
            JobResult::DownloadChapterFailed {
                task_id,
                chapter_id,
                chapter_index,
                ordinal,
                error,
                checkpoint,
            } => {
                render = false;
                if request != self.download_request
                    || !self.valid_cached_download_checkpoint(
                        &task_id,
                        chapter_index,
                        ordinal.saturating_add(1),
                    )
                {
                    let _ = checkpoint.send(Err(source::SourceError::CheckpointFailed));
                    return Ok(false);
                }
                if error.requires_login() {
                    // This is an identity boundary. The controller owns the
                    // original error and fences the worker before opening login.
                    let _ = checkpoint.send(Err(source::SourceError::CheckpointFailed));
                    self.download_terminal(&task_id, Some(error), false)?;
                    return Ok(true);
                }
                let updated_at = Utc::now().timestamp();
                let task = self.current_download.as_ref().unwrap();
                let retry = self.store.record_download_chapter_failure(
                    task,
                    &chapter_id,
                    chapter_index,
                    ordinal,
                    error.retryable() || download_recovery::rejected_read(&error),
                    &error.to_string(),
                    updated_at,
                );
                match retry {
                    Ok(retry) => {
                        let task = self.current_download.as_mut().unwrap();
                        task.updated_at = updated_at;
                        let task = task.clone();
                        // The scheduler may release this chapter's retry wait
                        // only after its deadline or blocked state is durable.
                        let _ = checkpoint.send(Ok(retry));
                        let progress = self.store.download_chapter_progress(&task.id)?;
                        self.download_feedback(
                            task.completed_count,
                            task.chapters.len(),
                            progress.retry_wait_count,
                            progress.blocked_count,
                        );
                        self.remember_download_notice(&task)?;
                    }
                    Err(error) => {
                        let _ = checkpoint.send(Err(source::SourceError::CheckpointFailed));
                        self.show_toast("operation_failed");
                        crate::diagnostic!("Persist chapter retry failed: {error:#}");
                    }
                }
            }
            JobResult::DownloadProgress {
                task_id,
                completed: _,
                total: _,
                title,
                waiting: _,
                blocked: _,
            } => {
                render = false;
                if request == self.download_request
                    && self.current_download.as_ref().is_some_and(|task| {
                        task.id == task_id && task.status == DownloadStatus::Running
                    })
                {
                    let reflow_library = self.route == Route::Library
                        && self.prefs.view_mode == "list"
                        && self.ui.upgrade().is_some_and(|ui| {
                            let library = ui.global::<LibraryModel>();
                            library.get_status().is_empty()
                                || library.get_books().row_count() > self.library_page_size()
                        });
                    if let Some(ui) = self.ui.upgrade() {
                        ui.global::<ShellModel>().set_dialog_chapter(title.into());
                    }
                    let task = self.current_download.as_ref().unwrap();
                    let progress = self.store.download_chapter_progress(&task.id)?;
                    // Progress can trail a different worker's durable failure.
                    // Read one current summary instead of combining snapshots.
                    self.download_feedback(
                        progress.completed_count,
                        task.chapters.len(),
                        progress.retry_wait_count,
                        progress.blocked_count,
                    );
                    self.sync_running_download_actions(progress.blocked_count);
                    if reflow_library {
                        // A status row can reduce list capacity when a retry starts
                        // without a user action. Reflow only this layout transition.
                        self.render_library()?;
                        self.download_ui_changes.clear();
                        self.download_ui_refresh_at = Instant::now() + Duration::from_secs(1);
                    }
                }
            }
            JobResult::DownloadFinished { task_id } => {
                render = request == self.download_request
                    && self
                        .current_download
                        .as_ref()
                        .is_some_and(|task| task.id == task_id);
                if render {
                    self.download_terminal(&task_id, None, false)?;
                }
            }
            JobResult::DownloadIncomplete { task_id, .. } => {
                render = request == self.download_request
                    && self
                        .current_download
                        .as_ref()
                        .is_some_and(|task| task.id == task_id);
                if render {
                    if self
                        .store
                        .download_chapter_progress(&task_id)?
                        .pending_count
                        > 0
                    {
                        // A manual retry can win after the worker took its
                        // terminal snapshot but before this packet is consumed.
                        self.requeue_incomplete_download(&task_id)?;
                    } else {
                        self.download_terminal(&task_id, None, false)?;
                    }
                }
            }
            JobResult::DownloadFailed { task_id, error } => {
                render = request == self.download_request
                    && self
                        .current_download
                        .as_ref()
                        .is_some_and(|task| task.id == task_id);
                if render {
                    self.download_terminal(&task_id, Some(error), false)?;
                }
            }
            JobResult::Cancelled { task_id } => {
                render = request == self.download_request
                    && self
                        .current_download
                        .as_ref()
                        .is_some_and(|task| task.id == task_id);
                if render {
                    self.download_terminal(&task_id, None, true)?;
                }
            }
        }
        Ok(render)
    }

    fn source_available(&mut self) {
        if !self.force_offline {
            self.online = true;
        }
    }

    fn job_error(&mut self, error: &source::SourceError, offer_login: bool) -> Result<()> {
        if matches!(error, source::SourceError::CheckpointFailed) {
            self.show_toast("operation_failed");
            return Ok(());
        }
        self.error_message(error);
        if error.requires_login() {
            self.settings_status = tr("session_expired");
            let reading_cached = matches!(self.route, Route::Reader(..))
                && !self.reader.body.is_empty()
                || matches!(self.route, Route::Article(_)) && !self.article.body.is_empty();
            if offer_login && !reading_cached && !self.demo && self.route != Route::Login {
                self.navigate(Route::Login, true)?;
                self.begin_login();
            }
        }
        Ok(())
    }

    fn catalog_result(
        &mut self,
        book_id: &str,
        result: source::Result<Vec<source::RemoteChapter>>,
    ) -> Result<()> {
        match self.catalog_result_inner(book_id, result) {
            Ok(()) => Ok(()),
            Err(error) => self.reject_catalog_storage(book_id, error),
        }
    }

    fn catalog_result_inner(
        &mut self,
        book_id: &str,
        result: source::Result<Vec<source::RemoteChapter>>,
    ) -> Result<()> {
        self.resident_lifecycle.forget_catalog_retry(book_id);
        match result {
            Ok(remote) => {
                let admission = source::content::catalog_metadata_bytes(book_id, &remote)
                    .map_err(anyhow::Error::from)
                    .and_then(|bytes| {
                        self.store
                            .validate_catalog_metadata_capacity(book_id, bytes)
                            .map(|()| bytes)
                    });
                let input_metadata = admission?;
                self.source_available();
                let headers = self.store.chapter_headers(book_id)?;
                // Price both operation-local maps before allocating them or
                // changing selections, pending work, saved state, or the Store.
                let mapped_admission = catalog_reindex_metadata_bytes(headers.len(), remote.len())
                    .and_then(|bytes| {
                        input_metadata
                            .checked_add(bytes)
                            .ok_or_else(|| source::SourceError::ResponseTooLarge.into())
                    })
                    .and_then(|bytes| {
                        self.store
                            .validate_catalog_metadata_capacity(book_id, bytes)
                    });
                mapped_admission?;
                let old = headers
                    .into_iter()
                    .map(|chapter| (chapter.id.clone(), chapter))
                    .collect::<BTreeMap<_, _>>();
                let chapters = remote
                    .iter()
                    .enumerate()
                    .map(|(index, chapter)| Chapter {
                        id: chapter.uid.clone(),
                        book_id: book_id.into(),
                        index: index as u32,
                        title: chapter.title.clone(),
                        cached: old
                            .get(&chapter.uid)
                            .is_some_and(|previous| previous.cached),
                        content_status: old
                            .get(&chapter.uid)
                            .map_or(ContentStatus::Unknown, |previous| previous.content_status),
                        content_account_key: old
                            .get(&chapter.uid)
                            .map(|previous| previous.content_account_key.clone())
                            .unwrap_or_default(),
                        body: None,
                    })
                    .collect::<Vec<_>>();
                if self.store.book_metadata_by_id(book_id)?.is_none() {
                    self.settle_catalog_waiting(book_id);
                    return Ok(());
                }
                let reindex = CatalogReindex::new(&old, &chapters)?;
                if !chapters.is_empty() {
                    let resident_reader = self.reader.book_id == book_id;
                    let active_reader = resident_reader
                        && matches!(&self.route, Route::Reader(id, _) if id == book_id);
                    let resident_target = resident_reader
                        .then(|| reindex.retained_chapter(self.reader.chapter_index))
                        .flatten();
                    let reader_anchor = resident_reader
                        .then(|| {
                            self.reader
                                .paginator
                                .as_ref()
                                .map(|pages| pages.current_page().start)
                        })
                        .flatten();
                    let prepared_reader = if active_reader {
                        let target = resident_target
                            .or_else(|| chapters.first())
                            .expect("A nonempty admitted catalog has a first chapter");
                        let source_index = old
                            .iter()
                            .find(|(_, chapter)| chapter.id == target.id)
                            .map(|(_, chapter)| chapter.index);
                        Some(self.prepare_catalog_reader_load(
                            book_id,
                            target,
                            source_index,
                            reader_anchor.filter(|_| resident_target.is_some()),
                            resident_target.is_none(),
                            &remote,
                            &chapters,
                        )?)
                    } else {
                        None
                    };
                    if resident_reader
                        && resident_target.is_none()
                        && self.reader.session_started.is_some()
                    {
                        // The old UID is still authoritative here. Finishing
                        // after replace_catalog could save an unrelated index.
                        self.finish_reading()?;
                    } else if active_reader {
                        self.persist_reader_position()?;
                    }
                    let restart_downloads = self
                        .current_download
                        .as_ref()
                        .is_some_and(|task| task.book_id == book_id);
                    if restart_downloads {
                        if let Some(mut task) = self.current_download.as_ref().cloned() {
                            task.status = DownloadStatus::Paused;
                            task.updated_at = Utc::now().timestamp();
                            self.store.save_task(&task)?;
                            self.current_download = None;
                            self.settle_job_submission(self.jobs.cancel_download(&task.id));
                            self.download_request = self.request();
                            self.downloads.push_front(task);
                            self.retire_task_wake_guard();
                        }
                    }
                    self.store.replace_catalog(book_id, &chapters)?;
                    if active_reader {
                        self.install_catalog_reader_load(prepared_reader.ok_or_else(|| {
                            anyhow::anyhow!("The relocated Reader preparation is unavailable")
                        })?)?;
                    }
                    if self.detail_id == book_id {
                        self.selected_chapters = self
                            .selected_chapters
                            .iter()
                            .filter_map(|index| reindex.reindex(*index))
                            .collect();
                    }
                    for (id, indices) in &mut self.pending_downloads {
                        if id == book_id {
                            *indices = indices
                                .iter()
                                .filter_map(|index| reindex.reindex(*index))
                                .collect();
                        }
                    }
                    if let Some((id, index)) =
                        self.pending_read.clone().filter(|(id, _)| id == book_id)
                    {
                        self.pending_read = reindex.reindex(index).map(|index| (id, index));
                    }
                    remap_reader_history(&mut self.history, book_id, &reindex);
                    if resident_reader && !active_reader {
                        if let Some(target) = resident_target {
                            // The UID and body are unchanged, so text anchors and selections remain valid.
                            self.reader.set_context(book_id, target.index);
                            self.reader.toc_page =
                                target.index as usize / self.profile.reader_chapter_page_size();
                        } else {
                            self.reader.set_context("", 0);
                            self.reader.replace_body(String::new());
                            self.reader.paginator = None;
                            self.reader.selection = None;
                            self.reader.press = None;
                            self.reader.draft.clear();
                            self.reader.cloud = None;
                            self.reader.replace_underlines(Vec::new());
                            self.reader.remote_thoughts.clear();
                            self.reader.thought_id.clear();
                            self.reader.thought_page = 0;
                            self.reader.thought_request = self.request();
                            if let Some(ui) = self.ui.upgrade() {
                                let reader = ui.global::<ReaderModel>();
                                reader.set_selection_open(false);
                                reader.set_writing_thought(false);
                                reader.set_thought_open(false);
                                reader.set_sync_conflict(false);
                            }
                            if self.route == Route::Search && self.search_purpose == "thought" {
                                self.search_purpose = "books".into();
                                self.query.clear();
                                self.search_request = self.request();
                                self.back()?;
                                self.show_toast("chapter_removed");
                            }
                        }
                    }
                    self.reload_pending_downloads(false)?;
                    // These in-memory identities follow the committed catalog.
                    // A later cache-key failure must not undo that SQL commit.
                    self.store
                        .set_setting(&format!("catalog.remote:{book_id}"), &remote)?;
                }
                if self.detail_id == book_id {
                    self.detail_status = if chapters.is_empty() {
                        tr("no_content")
                    } else {
                        String::new()
                    };
                }
                if self.pending_catalog_downloads.remove(book_id) {
                    let include_images = self
                        .pending_catalog_images
                        .remove(book_id)
                        .unwrap_or(self.prefs.download_images);
                    if chapters.is_empty() {
                        self.show_toast("unsupported_content");
                    } else {
                        self.enqueue_download_options(
                            book_id,
                            chapters.iter().map(|chapter| chapter.index).collect(),
                            include_images,
                        )?;
                    }
                }
                if let Some((id, index)) = self.pending_read.clone().filter(|(id, _)| id == book_id)
                {
                    if let Some(chapter) = reindex.chapter(index) {
                        if chapter.cached {
                            self.pending_read = None;
                            if matches!(&self.route, Route::Reader(route_id, _) if route_id == &id)
                            {
                                self.load_reader(&id, index, None)?;
                            }
                        } else if !self.chapter_queued(&id, index) {
                            self.enqueue_download(&id, vec![index])?;
                        }
                    } else {
                        self.pending_read = None;
                    }
                }
                self.start_next_download()?;
            }
            Err(error) => {
                let waiting = self.settle_catalog_waiting(book_id);
                if self.detail_id == book_id {
                    self.detail_status = tr("api_error");
                }
                if waiting || matches!(&self.route, Route::Detail(id) if id == book_id) {
                    self.job_error(&error, true)?;
                }
            }
        }
        Ok(())
    }

    fn settle_catalog_waiting(&mut self, book_id: &str) -> bool {
        let reader_waiting = self
            .pending_read
            .as_ref()
            .is_some_and(|(id, _)| id == book_id);
        let download_waiting = self.pending_catalog_downloads.remove(book_id);
        if reader_waiting {
            self.pending_read = None;
        }
        self.pending_catalog_images.remove(book_id);
        self.resident_lifecycle.forget_catalog_retry(book_id);
        reader_waiting || download_waiting
    }

    fn reject_catalog_storage(&mut self, book_id: &str, error: anyhow::Error) -> Result<()> {
        let waiting = self.settle_catalog_waiting(book_id);
        if self.detail_id == book_id {
            self.detail_status = tr("api_error");
        }
        if let Some(source_error) = error.downcast_ref::<source::SourceError>() {
            if waiting || matches!(&self.route, Route::Detail(id) if id == book_id) {
                self.job_error(source_error, false)?;
            } else {
                self.show_toast("api_error");
            }
            return Ok(());
        }
        if error
            .downcast_ref::<crate::store::CachedResourceLimit>()
            .is_some()
            || error
                .downcast_ref::<crate::store::BookMetadataLimit>()
                .is_some()
        {
            self.show_toast("api_error");
            return Ok(());
        }
        // SQLite/I/O failures keep their original typed cause. They are not a
        // capacity rejection, and cannot leave a producer-less waiting intent.
        Err(error.context("apply completed catalog result"))
    }

    fn visible_cover_page_active(&self) -> bool {
        !self.force_offline
            && self.online
            && !self.demo
            && match &self.route {
                Route::Library | Route::Detail(_) => true,
                Route::Search => self.search_purpose == "books",
                Route::Reader(..) => {
                    self.prefs.sleep_cover && self.current_sleep_book_id().is_some()
                }
                _ => false,
            }
    }

    fn visible_cover_ids(&self) -> Vec<String> {
        let Some(ui) = self.ui.upgrade() else {
            return Vec::new();
        };
        match &self.route {
            Route::Library => ui
                .global::<LibraryModel>()
                .get_books()
                .iter()
                .map(|row| row.id.to_string())
                .collect(),
            Route::Search if self.search_purpose == "books" => ui
                .global::<SearchModel>()
                .get_results()
                .iter()
                .map(|row| row.id.to_string())
                .collect(),
            Route::Detail(id)
                if ui.global::<DetailModel>().get_book().id.as_str() == id.as_str() =>
            {
                vec![id.clone()]
            }
            Route::Reader(..) => self
                .current_sleep_book_id()
                .map(|id| vec![id.to_owned()])
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Renderers call this after publishing their clamped, visible rows. Reuse
    /// those identities so list layouts, keyboards, and native viewport sizes
    /// cannot disagree with a second pagination calculation.
    pub(super) fn schedule_visible_covers(&mut self) {
        self.deferred_cover_refill = None;
        if !self.visible_cover_page_active() {
            return;
        }
        for id in self.visible_cover_ids() {
            if id.is_empty() || self.pending_covers.contains(&id) {
                continue;
            }
            let book = if self.route == Route::Search {
                self.search_results
                    .iter()
                    .find(|book| book.id == id)
                    .cloned()
            } else {
                self.store.book_metadata_by_id(&id).ok().flatten()
            };
            let Some(book) = book.filter(|book| !book.cover_url.is_empty()) else {
                continue;
            };
            let cache_key = if matches!(self.route, Route::Reader(..)) {
                let Some(key) = sleep_screen::cover_key(&id, &book.cover_url) else {
                    continue;
                };
                key
            } else {
                let Some(key) = sleep_screen::thumbnail_key(&id, &book.cover_url) else {
                    continue;
                };
                key
            };
            if self
                .store
                .setting::<i64>(&format!("cover.failed:{id}"))
                .ok()
                .flatten()
                .is_some_and(|time| Utc::now().timestamp().saturating_sub(time) < 60)
                || self.cache.get(&cache_key).ok().flatten().is_some()
            {
                continue;
            }
            if self.jobs.cover_capacity() == 0 {
                // One route marker replaces a waiting queue. A later tick can
                // refill it even if unrelated, stale metadata releases capacity.
                self.deferred_cover_refill = Some(self.route.clone());
                break;
            }
            let request = self.request();
            self.register_cover_request(request, &id, &book.cover_url);
            if self.settle_job_submission(self.jobs.cover(request, id, book.cover_url)) {
                self.deferred_cover_refill = Some(self.route.clone());
                break;
            }
        }
    }

    /// Idle ticks perform no SQL, digest reads, or rescans without deferred work.
    pub(super) fn refill_deferred_visible_covers(&mut self) {
        let Some(route) = &self.deferred_cover_refill else {
            return;
        };
        if route != &self.route || !self.visible_cover_page_active() {
            self.deferred_cover_refill = None;
            return;
        }
        if self.jobs.cover_capacity() > 0 {
            self.schedule_visible_covers();
        }
    }

    fn chapter_queued(&self, book_id: &str, index: u32) -> bool {
        self.current_download
            .iter()
            .chain(self.downloads.iter())
            .any(|task| {
                task.book_id == book_id
                    && task.status.is_pending()
                    && task
                        .chapters
                        .iter()
                        .skip(task.next_index)
                        .any(|chapter| *chapter == index)
            })
    }

    fn cloud_position(&mut self, book_id: &str, position: source::ReadingPosition) -> Result<()> {
        self.source_available();
        self.reader.position_confirmed = false;
        self.reader.cloud = None;
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<ReaderModel>().set_sync_conflict(false);
        }
        let chapters = self.store.chapter_metadata(book_id)?;
        let remote_index = chapters
            .iter()
            .find(|chapter| chapter.id == position.chapter_uid)
            .map(|chapter| chapter.index);
        let local = self.store.position(book_id)?;
        let local_index = local
            .as_ref()
            .map(|position| position.chapter_index)
            .unwrap_or(self.reader.chapter_index);
        let local_offset = local
            .as_ref()
            .map(|position| position.char_offset)
            .unwrap_or_else(|| {
                self.reader
                    .paginator
                    .as_ref()
                    .map(|pages| pages.current_page().start)
                    .unwrap_or(0)
            });
        let remote_catalog = self
            .store
            .setting::<Vec<source::RemoteChapter>>(&format!("catalog.remote:{book_id}"))?
            .unwrap_or_default();
        // One cloud comparison needs this one candidate's scalar length, not
        // an allocated copy of every cached chapter on ordinary redraws.
        let local_body_length = self.store.chapter_scalar_count(book_id, local_index)?;
        let local_fraction = if local_body_length == 0 {
            0.0
        } else {
            (local_offset as f64 / local_body_length as f64).clamp(0.0, 1.0)
        };
        let total_words = remote_catalog
            .iter()
            .map(|chapter| u64::from(chapter.word_count))
            .sum::<u64>();
        let local_progress = if total_words > 0 {
            let before = remote_catalog
                .iter()
                .take(local_index as usize)
                .map(|chapter| u64::from(chapter.word_count))
                .sum::<u64>();
            let current = remote_catalog
                .get(local_index as usize)
                .map(|chapter| chapter.word_count)
                .unwrap_or(0);
            (((before as f64 + f64::from(current) * local_fraction) / total_words as f64) * 100.0)
                .clamp(0.0, 100.0)
                .round() as u32
        } else {
            self.store
                .book_metadata_by_id(book_id)?
                .map(|book| (book.progress * 100.0).round() as u32)
                .unwrap_or(0)
        };
        let remote_fraction = remote_catalog
            .iter()
            .find(|chapter| chapter.uid == position.chapter_uid)
            .filter(|chapter| chapter.word_count > 0)
            .map(|chapter| {
                (f64::from(position.chapter_offset) / f64::from(chapter.word_count)).clamp(0.0, 1.0)
            });
        if remote_index.is_none() || remote_fraction.is_none() || total_words == 0 {
            return Ok(());
        }
        let farther = position.progress > local_progress
            || remote_index.is_some_and(|index| index > local_index)
            || remote_index == Some(local_index)
                && position.progress == local_progress
                && remote_fraction.is_some_and(|fraction| fraction > local_fraction);
        let now = Utc::now().timestamp();
        if self.prefs.ask_on_conflict && farther {
            let cloud_title = chapters
                .iter()
                .find(|chapter| chapter.id == position.chapter_uid)
                .map(|chapter| chapter.title.as_str())
                .unwrap_or_default();
            let local_title = chapters
                .iter()
                .find(|chapter| chapter.index == local_index)
                .map(|chapter| chapter.title.as_str())
                .unwrap_or_default();
            if let Some(ui) = self.ui.upgrade() {
                let model = ui.global::<ReaderModel>();
                model.set_cloud_summary(format!("{cloud_title} · {}%", position.progress).into());
                model.set_local_summary(format!("{local_title} · {local_progress}%").into());
                model.set_sync_conflict(true);
            }
            self.reader.cloud = Some(position);
        } else if !self.prefs.ask_on_conflict
            && (local.is_none() && farther
                || position.updated_at > 0
                    && position.updated_at <= now
                    && position.updated_at
                        > local
                            .as_ref()
                            .map(|position| position.updated_at)
                            .unwrap_or(0))
        {
            self.apply_cloud_position(&position)?;
        } else {
            let same_position = local_body_length > 0
                && remote_index == Some(local_index)
                && remote_fraction.is_some_and(|fraction| {
                    (fraction - local_fraction).abs() <= 1.0 / local_body_length as f64
                });
            let newer_local = local.as_ref().is_some_and(|local| {
                local.updated_at > 0
                    && position.updated_at > 0
                    && local.updated_at <= now
                    && position.updated_at <= now
                    && local.updated_at > position.updated_at
            });
            self.reader.position_confirmed = same_position || newer_local;
        }
        Ok(())
    }

    fn article_result(
        &mut self,
        request: u64,
        article_id: &str,
        content: source::ChapterContent,
    ) -> Result<bool> {
        let prepared = match self.prepare_incoming_article(request, article_id, content) {
            Ok(prepared) => prepared,
            Err(error)
                if Self::view_capacity(&error)
                    || error
                        .downcast_ref::<crate::article::ArticleParseError>()
                        .is_some()
                    || error
                        .downcast_ref::<crate::article::StyleLimitError>()
                        .is_some()
                    || error
                        .downcast_ref::<crate::article::ImageLimitError>()
                        .is_some()
                    || error
                        .downcast_ref::<super::auxiliary::ArticleProjectionLimit>()
                        .is_some()
                    || error
                        .downcast_ref::<crate::store::BookMetadataLimit>()
                        .is_some()
                    || error
                        .downcast_ref::<crate::store::CachedResourceLimit>()
                        .is_some()
                    || matches!(
                        error.downcast_ref::<source::SourceError>(),
                        Some(source::SourceError::ResponseTooLarge)
                    ) =>
            {
                self.show_toast("api_error");
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        anyhow::ensure!(
            prepared.request() == request,
            "Incoming Article preparation changed its request identity"
        );
        self.source_available();
        let metadata = prepared.metadata();
        let content = prepared.content();
        let book_id = format!("article:{article_id}");
        let previous = self.store.book(&book_id)?;
        let mut book = previous.unwrap_or_else(|| {
            Book::new(
                &book_id,
                metadata
                    .as_ref()
                    .map(|article| article.title.as_str())
                    .unwrap_or(article_id),
            )
        });
        book.source = "article".into();
        book.on_shelf = false;
        book.total_chapters = 1;
        book.cached_chapters = 1;
        if let Some(metadata) = metadata.as_ref() {
            book.title = metadata.title.clone();
            book.author = metadata.author.clone();
            book.cover_url = metadata.cover_url.clone();
        }
        self.store.upsert_book(&book)?;
        self.store.upsert_chapter(&Chapter {
            id: article_id.into(),
            book_id,
            index: 0,
            title: book.title,
            cached: true,
            body: Some(content.plain_text.clone()),
            content_status: ContentStatus::Full,
            ..Chapter::default()
        })?;
        self.store
            .set_setting(&format!("article.html:{article_id}"), &content.html)?;
        // Consume the same admitted document only after raw SQL publication.
        // The Aux installer rechecks the receiver before retiring its old work.
        self.install_incoming_article(prepared)?;
        Ok(true)
    }

    fn article_image_result(
        &mut self,
        request: u64,
        article_id: &str,
        id: &str,
        result: source::Result<Vec<u8>>,
    ) -> Result<bool> {
        if article_id != self.article.id
            || !matches!(&self.route, Route::Article(id) if id == &self.article.id)
            || !self.article.image_work.is_ready_for(&self.article.id)
        {
            return Ok(false);
        }
        let Some(index) = self.article.image_work.accepts(id, request) else {
            return Ok(false);
        };
        let key = format!("article-image:{article_id}:{id}");
        let url = self
            .article
            .image_work
            .image(index)
            .map(|image| image.url.as_str())
            .unwrap_or("");
        let cache_key = image_processing::article_cache_key(article_id, id, url);
        self.pending_article_images.remove(&key);
        let mut loaded = false;
        let outcome = match result {
            Ok(bytes) => {
                let (width, height) = self.profile.physical_size();
                match self.cache.put_gray8(
                    &cache_key,
                    &bytes,
                    width.saturating_sub(128).min(2048),
                    height.min(2048),
                ) {
                    Ok(path) => {
                        self.article.loaded_images.insert(id.to_owned(), path);
                        loaded = true;
                        ArticleImageOutcome::Success
                    }
                    Err(_) => {
                        self.show_toast("image_load_failed");
                        ArticleImageOutcome::Failed
                    }
                }
            }
            Err(source::SourceError::Busy) => ArticleImageOutcome::Busy,
            Err(error) => {
                self.job_error(&error, false)?;
                ArticleImageOutcome::Failed
            }
        };
        self.article.image_work.finish(index, request, outcome);
        if outcome == ArticleImageOutcome::Busy {
            // Synchronous admission rejection must not recursively re-enter
            // image submission. The next tick continues the retained intent.
            return Ok(true);
        }
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
        Ok(true)
    }

    /// Register before submission because capacity rejection can settle inline.
    pub(super) fn register_cover_request(&mut self, request: u64, book_id: &str, url: &str) {
        self.pending_covers.insert(book_id.to_owned());
        self.pending_cover_requests.insert(
            book_id.to_owned(),
            CoverRequestContext {
                request,
                account_epoch: self.jobs.account_epoch(),
                url: url.to_owned(),
            },
        );
    }

    pub(super) fn clear_pending_cover_requests(&mut self) {
        self.pending_covers.clear();
        self.pending_cover_requests.clear();
        self.cover_ui_changes.clear();
        self.deferred_cover_refill = None;
    }

    fn current_cover_url(&self, book_id: &str) -> Option<String> {
        if self.route == Route::Search && self.search_purpose == "books" {
            if let Some(book) = self.search_results.iter().find(|book| book.id == book_id) {
                return Some(book.cover_url.clone());
            }
        }
        self.store
            .book_metadata_by_id(book_id)
            .ok()
            .flatten()
            .map(|book| book.cover_url)
    }

    fn defer_current_cover(&mut self, book_id: &str) {
        if self.visible_cover_page_active()
            && self.visible_cover_ids().iter().any(|id| id == book_id)
        {
            self.deferred_cover_refill = Some(self.route.clone());
        }
    }

    fn cover_result(
        &mut self,
        request: u64,
        book_id: &str,
        result: source::Result<Vec<u8>>,
    ) -> Result<()> {
        let Some(context) = self.pending_cover_requests.get(book_id) else {
            return Ok(());
        };
        if context.request != request || context.account_epoch != self.jobs.account_epoch() {
            return Ok(());
        }
        let context = self.pending_cover_requests.remove(book_id).unwrap();
        self.pending_covers.remove(book_id);
        if !matches!(&result, Err(source::SourceError::Busy))
            && self.current_cover_url(book_id).as_deref() != Some(context.url.as_str())
        {
            self.defer_current_cover(book_id);
            return Ok(());
        }
        match result {
            Ok(bytes) => {
                let Some(key) = sleep_screen::thumbnail_key(book_id, &context.url) else {
                    return Ok(());
                };
                match self.cache.put_cover(&key, &bytes, 400, 560) {
                    Ok(path) => {
                        self.store
                            .delete_setting(&format!("cover.failed:{book_id}"))?;
                        if !self.prepare_downloaded_sleep_cover(book_id, &context.url, &bytes) {
                            self.store.set_setting(
                                &format!("cover.failed:{book_id}"),
                                &Utc::now().timestamp(),
                            )?;
                        }
                        if self.visible_cover_ids().iter().any(|id| id == book_id) {
                            self.cover_ui_changes.insert(book_id.to_owned(), path);
                        }
                    }
                    Err(_) => {
                        self.store.set_setting(
                            &format!("cover.failed:{book_id}"),
                            &Utc::now().timestamp(),
                        )?;
                    }
                }
            }
            // Admission pressure says nothing about the remote image. Leave it
            // immediately retryable instead of recording the network backoff.
            Err(source::SourceError::Busy) => {
                self.defer_current_cover(book_id);
            }
            Err(_) => {
                self.store
                    .set_setting(&format!("cover.failed:{book_id}"), &Utc::now().timestamp())?;
            }
        }
        Ok(())
    }

    fn updated_cover_tile(
        &self,
        mut tile: BookTile,
        changes: &BTreeMap<String, PathBuf>,
    ) -> Option<BookTile> {
        let path = changes.get(tile.id.as_str())?;
        tile.cover = slint::Image::load_from_path(path).ok()?;
        tile.has_cover = true;
        Some(tile)
    }

    fn updated_search_cover(
        &self,
        mut row: SearchResult,
        changes: &BTreeMap<String, PathBuf>,
    ) -> Option<SearchResult> {
        let path = changes.get(row.id.as_str())?;
        row.cover = slint::Image::load_from_path(path).ok()?;
        Some(row)
    }

    fn updated_cover_rows<T: Clone + 'static>(
        &self,
        rows: ModelRc<T>,
        mut update: impl FnMut(T) -> Option<T>,
    ) -> Option<ModelRc<T>> {
        let updates: Vec<_> = rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| update(row).map(|row| (index, row)))
            .collect();
        if updates.is_empty() {
            return None;
        }
        if rows.as_any().is::<VecModel<T>>() {
            for (index, tile) in updates {
                rows.set_row_data(index, tile);
            }
            Some(rows)
        } else {
            // A read-only replacement model must not silently discard writes.
            // Preserve its published order and metadata in a mutable fallback.
            let mut replacement: Vec<_> = rows.iter().collect();
            for (index, tile) in updates {
                replacement[index] = tile;
            }
            Some(model(replacement))
        }
    }

    /// Apply one mailbox batch without rebuilding the shelf, its pagination,
    /// or its selection state. Offscreen completions remain cache-only.
    pub(super) fn flush_cover_ui(&mut self) {
        if self.cover_ui_changes.is_empty() {
            return;
        }
        let changes = std::mem::take(&mut self.cover_ui_changes);
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let changed = match &self.route {
            Route::Library => {
                let state = ui.global::<LibraryModel>();
                if let Some(rows) = self.updated_cover_rows(state.get_books(), |tile| {
                    self.updated_cover_tile(tile, &changes)
                }) {
                    state.set_books(rows);
                    true
                } else {
                    false
                }
            }
            Route::Search if self.search_purpose == "books" => {
                let state = ui.global::<SearchModel>();
                if let Some(rows) = self.updated_cover_rows(state.get_results(), |row| {
                    self.updated_search_cover(row, &changes)
                }) {
                    state.set_results(rows);
                    true
                } else {
                    false
                }
            }
            Route::Detail(id) => {
                let state = ui.global::<DetailModel>();
                let tile = state.get_book();
                if tile.id.as_str() != id.as_str() {
                    false
                } else if let Some(tile) = self.updated_cover_tile(tile, &changes) {
                    state.set_book(tile);
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if changed {
            self.refresh_display(RefreshReason::CoversLoaded);
        }
    }

    fn login_failure(&mut self, error: &source::SourceError) {
        let key = match error {
            source::SourceError::Unsupported("skill_not_enabled") => "login_skill_required",
            source::SourceError::LoginExpired => "login_expired",
            source::SourceError::Network | source::SourceError::Timeout => "network_unavailable",
            _ => "api_error",
        };
        if matches!(
            error,
            source::SourceError::Network | source::SourceError::Timeout
        ) {
            self.online = false;
        }
        if let Some(ui) = self.ui.upgrade() {
            let login = ui.global::<LoginModel>();
            login.set_busy(false);
            login.set_status(tr(key).into());
        }
        if matches!(error, source::SourceError::Unsupported("skill_not_enabled")) {
            self.login_challenge = None;
        }
    }

    fn login_complete(&mut self, mut session: source::AuthSession) -> Result<()> {
        if session.api_key.is_empty() {
            if let Some(previous) = self.jobs.session().filter(|previous| {
                !previous.user_vid.is_empty()
                    && !session.user_vid.is_empty()
                    && previous.user_vid == session.user_vid
            }) {
                session.api_key = previous.api_key;
            }
        }
        if session.api_key.is_empty() {
            self.login_challenge = None;
            self.jobs.cancel_login();
            self.login_failure(&source::SourceError::Unsupported("skill_not_enabled"));
            return Ok(());
        }
        // Task inventory is independently admitted before replacing a session
        // or retiring its current queue/worker. A quota refusal is not logout.
        let pending = if let Some(owner) = library::download_account_key(&session) {
            self.store.pending_tasks_for_owner(&owner)
        } else {
            Ok(Vec::new())
        };
        let pending: VecDeque<DownloadTask> = match pending {
            Ok(tasks) => tasks.into_iter().collect(),
            Err(error) if Self::inventory_capacity(&error) => {
                self.download_inventory_unavailable = true;
                self.login_failure(&source::SourceError::ResponseTooLarge);
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        self.session_file.save(&session)?;
        self.invalidate_account_work()?;
        self.jobs.replace_session(session)?;
        self.discard_cancelled_job_events();
        self.synchronize_content_owner()?;
        self.login_request = self.request();
        self.login_challenge = None;
        self.login_polling = false;
        self.login_code.clear();
        self.login_key.clear();
        if let Some(ui) = self.ui.upgrade() {
            let login = ui.global::<LoginModel>();
            login.set_api_key("".into());
            login.set_code("".into());
            login.set_busy(false);
            login.set_status("".into());
        }
        self.source_available();
        self.history.clear();
        self.navigate(Route::Library, false)?;
        self.show_toast("login_success");
        self.sync_shelf();
        self.downloads = pending;
        self.download_inventory_unavailable = false;
        self.start_next_download()?;
        Ok(())
    }

    fn reader_matches(&self, book_id: &str, chapter_id: &str) -> Result<bool> {
        Ok(self.reader.book_id == book_id
            && matches!(&self.route, Route::Reader(id, _) if id == book_id)
            && self
                .store
                .chapter_uid(book_id, self.reader.chapter_index)?
                .is_some_and(|uid| uid == chapter_id))
    }

    fn valid_cached_download_checkpoint(
        &self,
        task_id: &str,
        chapter_index: u32,
        next_index: usize,
    ) -> bool {
        let owner = self
            .jobs
            .session()
            .as_ref()
            .and_then(library::download_account_key);
        self.current_download.as_ref().is_some_and(|task| {
            task.id == task_id
                && task.status == DownloadStatus::Running
                && next_index
                    .checked_sub(1)
                    .and_then(|ordinal| task.chapters.get(ordinal))
                    == Some(&chapter_index)
                && owner.as_deref() == Some(task.account_key.as_str())
                && !task.account_key.is_empty()
        })
    }

    fn cached_downloaded_chapter(
        &mut self,
        task_id: &str,
        chapter_id: &str,
        chapter_index: u32,
        next_index: usize,
    ) -> Result<bool> {
        anyhow::ensure!(
            self.valid_cached_download_checkpoint(task_id, chapter_index, next_index),
            "A cached chapter packet does not match its running download",
        );
        let owner = self
            .jobs
            .session()
            .as_ref()
            .and_then(library::download_account_key)
            .ok_or_else(|| anyhow::anyhow!("A cached checkpoint has no current account"))?;
        let updated_at = Utc::now().timestamp();
        let task = self.current_download.as_ref().unwrap();
        let progress = if task.include_images {
            let proof = self
                .book_assets
                .verify_complete_checkpoint(&self.store, &task.book_id, chapter_id)?
                .ok_or(source::SourceError::ContentChanged)?;
            self.store.commit_verified_cached_image_download_chapter(
                task,
                proof,
                chapter_index,
                next_index - 1,
                &owner,
                updated_at,
            )?
        } else {
            self.store.commit_cached_download_chapter(
                task,
                chapter_id,
                chapter_index,
                next_index - 1,
                &owner,
                updated_at,
            )?
        };
        let book_id = task.book_id.clone();
        let task = self.current_download.as_mut().unwrap();
        task.next_index = progress.next_index;
        task.completed_count = progress.completed_count;
        task.updated_at = updated_at;
        let notice_task = task.clone();
        self.download_feedback(
            progress.completed_count,
            self.current_download.as_ref().unwrap().chapters.len(),
            progress.retry_wait_count,
            progress.blocked_count,
        );
        if let Err(error) = self.remember_download_notice(&notice_task) {
            crate::diagnostic!("Refresh committed cached download notice failed: {error:#}");
        }
        self.download_ui_changes
            .entry(book_id.clone())
            .or_default()
            .chapters
            .insert(chapter_id.into(), ContentStatus::Full);
        if self
            .pending_read
            .as_ref()
            .is_some_and(|(id, index)| id == &book_id && *index == chapter_index)
        {
            self.pending_read = None;
        }
        let visible = matches!(&self.route, Route::Reader(id, index)
            if id == &book_id && *index == chapter_index);
        let mut loaded = visible;
        if visible {
            // The event still owns packet credit here. Only a visible chapter
            // may materialize its body; every background checkpoint stays SQL-only.
            if let Err(error) = self.load_reader(&book_id, chapter_index, None) {
                crate::diagnostic!(
                    "Reload Reader after a committed cached checkpoint failed: {error:#}"
                );
                self.show_toast("operation_failed");
                loaded = false;
            }
            if let Err(error) = self.save_reader_session() {
                crate::diagnostic!(
                    "Save Reader after a committed cached checkpoint failed: {error:#}"
                );
                self.show_toast("operation_failed");
            }
        }
        // SQL is already committed and the caller still ACKs this packet. A
        // refused display cannot replace old UI rows with an empty rendering.
        Ok(loaded)
    }

    fn downloaded_chapter(
        &mut self,
        task_id: &str,
        chapter: Chapter,
        next_index: usize,
        publication: Option<crate::book_assets::VerifiedChapterPublication>,
        committed: &mut bool,
    ) -> Result<bool> {
        anyhow::ensure!(
            self.valid_download_checkpoint(task_id, &chapter, next_index),
            "A chapter packet does not match its running download",
        );
        let updated_at = Utc::now().timestamp();
        let task = self.current_download.as_ref().unwrap();
        let committed_chapter =
            if task.include_images && chapter.content_status == ContentStatus::Full {
                if let Some(proof) = publication {
                    self.store.commit_verified_image_download_chapter(
                        task,
                        &chapter,
                        next_index - 1,
                        proof,
                        updated_at,
                    )?
                } else {
                    self.store.commit_pending_image_download_chapter(
                        task,
                        &chapter,
                        next_index - 1,
                        updated_at,
                    )?
                }
            } else {
                self.store.commit_independent_download_chapter(
                    task,
                    &chapter,
                    next_index - 1,
                    updated_at,
                )?
            };
        *committed = true;
        let task = self.current_download.as_mut().unwrap();
        task.next_index = committed_chapter.progress.next_index;
        task.completed_count = committed_chapter.progress.completed_count;
        task.updated_at = updated_at;
        let notice_task = task.clone();
        self.download_feedback(
            committed_chapter.progress.completed_count,
            self.current_download.as_ref().unwrap().chapters.len(),
            committed_chapter.progress.retry_wait_count,
            committed_chapter.progress.blocked_count,
        );
        if let Err(error) = self.remember_download_notice(&notice_task) {
            crate::diagnostic!("Refresh committed download notice failed: {error:#}");
        }
        let changes = self
            .download_ui_changes
            .entry(chapter.book_id.clone())
            .or_default();
        changes
            .chapters
            .insert(chapter.id.clone(), chapter.content_status);
        if committed_chapter.newly_cached {
            changes.newly_cached += 1;
        }
        let requested = self
            .pending_read
            .as_ref()
            .is_some_and(|(id, index)| id == &chapter.book_id && *index == chapter.index);
        if requested {
            self.pending_read = None;
        }
        let visible = matches!(&self.route, Route::Reader(id, index)
            if id == &chapter.book_id && *index == chapter.index);
        if visible {
            let mut followup_error = None;
            let current_anchor = self
                .reader
                .paginator
                .as_ref()
                .map(|p| p.current_page().start);
            let anchor = chapter
                .body
                .as_deref()
                .filter(|body| *body != self.reader.body)
                .and_then(|body| {
                    current_anchor.map(|anchor| {
                        let common = self
                            .reader
                            .body
                            .chars()
                            .zip(body.chars())
                            .take_while(|(old, new)| old == new)
                            .count();
                        anchor.min(common.min(u32::MAX as usize) as u32)
                    })
                });
            // Reloading the same reader preserves its scalar position and reflows
            // new image geometry. A display failure cannot undo a durable commit.
            if let Err(error) = self.load_reader(&chapter.book_id, chapter.index, anchor) {
                crate::diagnostic!("Reload Reader after a committed chapter failed: {error:#}");
                self.show_toast("operation_failed");
                followup_error = Some(error.context("reload Reader after a committed chapter"));
            }
            if let Err(error) = self.save_reader_session() {
                crate::diagnostic!("Save Reader after a committed chapter failed: {error:#}");
                if followup_error.is_none() {
                    followup_error = Some(error.context("save Reader after a committed chapter"));
                }
            }
            if let Some(error) = followup_error {
                return Err(error);
            }
        }
        Ok(visible)
    }

    fn valid_download_checkpoint(
        &self,
        task_id: &str,
        chapter: &Chapter,
        next_index: usize,
    ) -> bool {
        self.current_download.as_ref().is_some_and(|task| {
            task.id == task_id
                && task.status == DownloadStatus::Running
                && chapter.cached == (chapter.content_status == ContentStatus::Full)
                && next_index
                    .checked_sub(1)
                    .and_then(|ordinal| task.chapters.get(ordinal))
                    == Some(&chapter.index)
                && task.book_id == chapter.book_id
                && chapter.content_account_key == task.account_key
                && (chapter.content_status == ContentStatus::Locked && chapter.body.is_none()
                    || chapter
                        .body
                        .as_deref()
                        .is_some_and(|body| !body.trim().is_empty()))
        })
    }

    pub(super) fn download_feedback(
        &self,
        completed: usize,
        total: usize,
        waiting: usize,
        blocked: usize,
    ) {
        if let Some(ui) = self.ui.upgrade() {
            let shell = ui.global::<ShellModel>();
            shell.set_task_label(
                download_recovery::chapter_summary_label(completed, total, waiting, blocked).into(),
            );
            shell.set_dialog_progress(if total == 0 {
                0.0
            } else {
                (completed as f32 / total as f32).clamp(0.0, 1.0)
            });
            if self.route == Route::Library {
                ui.global::<LibraryModel>()
                    .set_status(shell.get_task_label());
            }
            ui.global::<LibraryModel>().set_download_notice(true);
            if self
                .current_download
                .as_ref()
                .is_some_and(|task| matches!(&self.route, Route::Detail(id) if *id == task.book_id))
            {
                ui.global::<DetailModel>()
                    .set_status(shell.get_task_label());
            }
        }
    }

    /// Update download-dependent fields without reading covers or rebuilding the
    /// currently visible route for every checkpoint. Other routes read fresh state
    /// on navigation, and active-reader packets use the immediate render path.
    pub(super) fn flush_download_ui(&mut self, now: Instant) -> Result<()> {
        if self.download_ui_changes.is_empty() || now < self.download_ui_refresh_at {
            return Ok(());
        }
        let changes = std::mem::take(&mut self.download_ui_changes);
        let result = (|| -> Result<()> {
            let Some(ui) = self.ui.upgrade() else {
                return Ok(());
            };
            match &self.route {
                Route::Library => {
                    let mut new_filtered_member = false;
                    let rows = ui.global::<LibraryModel>().get_books();
                    for (id, changes) in &changes {
                        let Some(book) = self.store.book_metadata_by_id(id)? else {
                            continue;
                        };
                        new_filtered_member |= self.library_sort == "downloaded"
                            && changes.newly_cached > 0
                            && book.cached_chapters as usize == changes.newly_cached;
                        for index in 0..rows.row_count() {
                            if let Some(mut row) = rows
                                .row_data(index)
                                .filter(|row| row.id.as_str() == id.as_str())
                            {
                                let cached = book.cached_chapters > 0;
                                if row.cached != cached {
                                    row.cached = cached;
                                    rows.set_row_data(index, row);
                                }
                            }
                        }
                    }
                    if new_filtered_member {
                        // A new member can change filtered pagination. Rebuild once
                        // when the book first becomes cached, rather than per chapter.
                        self.render_library()?;
                    }
                }
                Route::Detail(id) => {
                    if let Some(changes) = changes.get(id) {
                        if let Some(book) = self.store.book_metadata_by_id(id)? {
                            let state = ui.global::<DetailModel>();
                            let mut tile = state.get_book();
                            if tile.id.as_str() == id.as_str()
                                && tile.cached != (book.cached_chapters > 0)
                            {
                                tile.cached = book.cached_chapters > 0;
                                state.set_book(tile);
                            }
                            state.set_summary(
                                i18n::format(
                                    "cache_summary",
                                    &[
                                        ("progress", (book.progress * 100.0).round().to_string()),
                                        ("cached", book.cached_chapters.to_string()),
                                        ("total", book.total_chapters.to_string()),
                                    ],
                                )
                                .into(),
                            );
                            state.set_chapter_summary(download_chapter_summary(&book).into());
                            if book.cached_chapters > 0 {
                                state.set_can_read(true);
                            }
                            let rows = state.get_chapters();
                            for index in 0..rows.row_count() {
                                if let Some(mut row) = rows
                                    .row_data(index)
                                    .filter(|row| changes.chapters.contains_key(row.id.as_str()))
                                {
                                    let content_status = changes.chapters[row.id.as_str()];
                                    row.cached = content_status == ContentStatus::Full;
                                    let label = tr(match content_status {
                                        ContentStatus::Full => "downloaded",
                                        ContentStatus::Preview => "content_preview",
                                        ContentStatus::Locked => "content_locked",
                                        ContentStatus::Unknown => "content_unverified",
                                    });
                                    row.state = if row.current {
                                        format!("{} · {label}", tr("current_chapter"))
                                    } else {
                                        label
                                    }
                                    .into();
                                    if !row.cached {
                                        state.set_access_needs_refresh(true);
                                    }
                                    rows.set_row_data(index, row);
                                }
                            }
                        }
                    }
                }
                Route::Reader(id, _) => {
                    if let Some(changes) = changes.get(id) {
                        if let Some(book) = self.store.book_metadata_by_id(id)? {
                            let state = ui.global::<ReaderModel>();
                            state.set_chapter_summary(download_chapter_summary(&book).into());
                            let rows = state.get_chapters();
                            for index in 0..rows.row_count() {
                                if let Some(mut row) = rows
                                    .row_data(index)
                                    .filter(|row| changes.chapters.contains_key(row.id.as_str()))
                                {
                                    let content_status = changes.chapters[row.id.as_str()];
                                    row.cached = content_status == ContentStatus::Full;
                                    let label = match content_status {
                                        ContentStatus::Full => String::new(),
                                        ContentStatus::Preview => tr("content_preview"),
                                        ContentStatus::Locked => tr("content_locked"),
                                        ContentStatus::Unknown => tr("content_unverified"),
                                    };
                                    row.meta = if row.current {
                                        if label.is_empty() {
                                            tr("current_chapter")
                                        } else {
                                            format!("{} · {label}", tr("current_chapter"))
                                        }
                                    } else {
                                        label
                                    }
                                    .into();
                                    rows.set_row_data(index, row);
                                }
                            }
                        }
                    }
                }
                Route::Settings if matches!(self.settings_subpage.as_str(), "" | "storage") => {
                    self.render_settings()?;
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.download_ui_changes = changes;
            return Err(error);
        }
        self.download_ui_refresh_at = now + Duration::from_secs(1);
        Ok(())
    }

    fn download_terminal(
        &mut self,
        task_id: &str,
        error: Option<source::SourceError>,
        cancelled: bool,
    ) -> Result<()> {
        let Some(mut task) = self.current_download.take() else {
            return Ok(());
        };
        if task.id != task_id {
            self.current_download = Some(task);
            return Ok(());
        }
        let progress = match self.store.download_chapter_progress(&task.id) {
            Ok(progress) => progress,
            Err(error) => {
                self.current_download = Some(task);
                return Err(error);
            }
        };
        let previous = task.clone();
        let network_pause = error.as_ref().is_some_and(|error| {
            matches!(
                error,
                source::SourceError::Network | source::SourceError::Timeout
            )
        });
        let retry_pause = error.as_ref().is_some_and(source::SourceError::retryable);
        let auth_pause = error
            .as_ref()
            .is_some_and(source::SourceError::requires_login);
        let content_pause = error.as_ref().is_some_and(|error| {
            matches!(
                error,
                source::SourceError::UnlockRequired
                    | source::SourceError::ContentUnverified
                    | source::SourceError::ContentChanged
            )
        });
        task.status = if cancelled {
            DownloadStatus::Cancelled
        } else if retry_pause || auth_pause || content_pause {
            DownloadStatus::Paused
        } else if error.is_some() {
            DownloadStatus::Failed
        } else if task.completed_count == task.chapters.len()
            && progress.completed_count == task.chapters.len()
            && progress.next_index == task.chapters.len()
        {
            DownloadStatus::Complete
        } else if progress.blocked_count > 0 {
            DownloadStatus::Failed
        } else {
            DownloadStatus::Paused
        };
        task.error = error.as_ref().map(ToString::to_string);
        task.updated_at = Utc::now().timestamp();
        match self.store.save_task_if_current(&previous, &task) {
            Ok(true) => {}
            Ok(false) => {
                self.settle_job_submission(self.jobs.cancel_download(&task.id));
                self.clear_download_feedback();
                self.download_ui_changes.clear();
                self.show_toast("operation_failed");
                return Ok(());
            }
            Err(error) => {
                self.current_download = Some(previous);
                return Err(error);
            }
        }
        let was_open = matches!(
            self.dialog_action.as_str(),
            "downloading" | "download-retry-active"
        );
        self.clear_download_feedback();
        if !cancelled
            && error.is_none()
            && (progress.retry_wait_count > 0 || progress.blocked_count > 0)
        {
            self.remember_download_notice(&task)?;
            if was_open {
                self.open_download_notice()?;
            }
            self.start_next_download()?;
            return Ok(());
        }
        self.clear_download_notice(&task.id)?;
        if retry_pause {
            if network_pause {
                self.online = false;
            }
            self.downloads.push_front(task.clone());
            self.schedule_download_retry();
            self.show_toast("download_retry_later");
            return Ok(());
        } else if let Some(error) = error {
            self.job_error(&error, true)?;
        } else if task.status == DownloadStatus::Complete {
            self.show_toast("download_complete");
        }
        if auth_pause {
            return Ok(());
        }
        self.start_next_download()?;
        Ok(())
    }

    fn clear_download_feedback(&mut self) {
        self.retire_task_wake_guard();
        if matches!(
            self.dialog_action.as_str(),
            "downloading" | "download-retry-active"
        ) {
            self.dialog_action.clear();
            if let Some(ui) = self.ui.upgrade() {
                ui.global::<ShellModel>().set_dialog_kind("".into());
            }
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<ShellModel>().set_task_label("".into());
        }
    }
}

fn download_chapter_summary(book: &BookMetadata) -> String {
    i18n::format(
        "chapter_summary",
        &[
            ("total", book.total_chapters.to_string()),
            ("cached", book.cached_chapters.to_string()),
        ],
    )
}

fn remap_reader_history(history: &mut [Route], book_id: &str, reindex: &CatalogReindex<'_>) {
    for route in history {
        if let Route::Reader(id, index) = route {
            if id != book_id {
                continue;
            }
            let target = reindex.retained_chapter(*index);
            if let Some(target) = target {
                *index = target.index;
            } else {
                *route = Route::Detail(book_id.into());
            }
        }
    }
}

#[cfg(test)]
#[path = "catalog_reindex_work_tests.rs"]
mod catalog_reindex_work_tests;

#[cfg(test)]
mod tests {
    use super::*;

    struct BusyFixtureUi;

    impl slint::platform::Platform for BusyFixtureUi {
        fn create_window_adapter(
            &self,
        ) -> std::result::Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError>
        {
            Ok(
                slint::platform::software_renderer::MinimalSoftwareWindow::new(
                    slint::platform::software_renderer::RepaintBufferType::NewBuffer,
                ),
            )
        }
    }

    fn settle_cancelled_metadata(
        runtime: &tokio::runtime::Runtime,
        app: &App,
        expected_capacity: usize,
    ) -> Result<()> {
        runtime.block_on(async {
            let deadline = Instant::now() + Duration::from_secs(2);
            while app.jobs.metadata_capacity() != expected_capacity {
                anyhow::ensure!(
                    Instant::now() < deadline,
                    "Cancelled metadata tasks did not release their admission permits"
                );
                tokio::task::yield_now().await;
            }
            Ok(())
        })
    }

    fn busy_event_case() -> Result<()> {
        slint::platform::set_platform(Box::new(BusyFixtureUi))?;
        let directory = tempfile::tempdir()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let platform = Arc::new(Platform::new(DeviceProfile::Paperwhite6, false)?);
        let ui = AppWindow::new()?;
        crate::platform::configure_window(ui.window(), &platform)?;
        let application = App::attach(
            &ui,
            AppOptions {
                data_dir: directory.path().to_path_buf(),
                profile: DeviceProfile::Paperwhite6,
                demo: false,
                offline: true,
                initial_screen: "library".into(),
                fonts: vec![],
            },
            platform,
            &runtime,
        )?;
        {
            let mut app = application.borrow_mut();
            app.stop_lighting_hardware()?;
            app.route = Route::Reader("rejected-catalog".into(), 2);
            app.pending_read = Some(("rejected-catalog".into(), 2));
            app.pending_catalog_downloads
                .insert("rejected-catalog".into());
            app.pending_catalog_images
                .insert("rejected-catalog".into(), true);
            app.catalog_requests.insert("rejected-catalog".into(), 42);
            let epoch = app.jobs.account_epoch();
            assert!(app.handle_job(JobEvent {
                credits: None,
                request: 42,
                account_epoch: epoch,
                result: JobResult::Catalog {
                    book_id: "rejected-catalog".into(),
                    result: Err(source::SourceError::Busy),
                },
            })?);
            assert!(app.pending_read.is_none());
            assert!(!app.pending_catalog_downloads.contains("rejected-catalog"));
            assert!(!app.pending_catalog_images.contains_key("rejected-catalog"));
            assert!(!app.catalog_requests.contains_key("rejected-catalog"));
            assert_eq!(app.route, Route::Reader("rejected-catalog".into(), 2));

            app.pending_read = Some(("another-reader".into(), 7));
            app.catalog_requests.insert("other-rejection".into(), 43);
            assert!(app.handle_job(JobEvent {
                credits: None,
                request: 43,
                account_epoch: epoch,
                result: JobResult::Catalog {
                    book_id: "other-rejection".into(),
                    result: Err(source::SourceError::Busy),
                },
            })?);
            assert_eq!(app.pending_read, Some(("another-reader".into(), 7)));

            app.settings_status = tr("synchronizing");
            assert!(app.handle_job(JobEvent {
                credits: None,
                request: 44,
                account_epoch: epoch,
                result: JobResult::Renew(Err(source::SourceError::Busy)),
            })?);
            assert_eq!(app.settings_status, tr("operation_failed"));
            assert_eq!(app.route, Route::Reader("rejected-catalog".into(), 2));

            app.register_cover_request(45, "busy-cover", "");
            assert!(!app.handle_job(JobEvent {
                credits: None,
                request: 45,
                account_epoch: epoch,
                result: JobResult::Cover {
                    book_id: "busy-cover".into(),
                    result: Err(source::SourceError::Busy),
                },
            })?);
            assert!(!app.pending_covers.contains("busy-cover"));
            assert!(
                app.store
                    .setting::<i64>("cover.failed:busy-cover")?
                    .is_none()
            );

            let image_key = "article-image:fixture:busy-image";
            app.route = Route::Article("fixture".into());
            app.article.id = "fixture".into();
            app.article.image_work.reset("fixture");
            app.article
                .image_work
                .install(vec![crate::article::ArticleImage {
                    id: "busy-image".into(),
                    url: "file:///fixture.png".into(),
                    caption: String::new(),
                }]);
            assert!(app.article.image_work.admit(0, 46));
            app.pending_article_images.insert(image_key.into());
            ui.global::<ShellModel>().set_toast("".into());
            assert!(app.handle_job(JobEvent {
                credits: None,
                request: 46,
                account_epoch: epoch,
                result: JobResult::ArticleImage {
                    article_id: "fixture".into(),
                    image_id: "busy-image".into(),
                    result: Err(source::SourceError::Busy),
                },
            })?);
            assert!(!app.pending_article_images.contains(image_key));
            assert!(ui.global::<ShellModel>().get_toast().is_empty());
            // Opaque book IDs may equal a current article's cache key. The
            // typed purpose, not a string prefix, must select their lifecycle.
            assert!(app.article.image_work.admit(0, 47));
            app.pending_article_images.insert(image_key.into());
            app.store
                .upsert_book(&Book::new(image_key, "Opaque cover fixture"))?;
            app.register_cover_request(48, image_key, "");
            let mut opaque_png = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(2, 2)
                .write_to(&mut opaque_png, image::ImageFormat::Png)?;
            assert!(!app.handle_job(JobEvent {
                credits: None,
                request: 48,
                account_epoch: epoch,
                result: JobResult::Cover {
                    book_id: image_key.into(),
                    result: Ok(opaque_png.into_inner())
                },
            })?);
            assert!(!app.pending_covers.contains(image_key));
            assert!(app.pending_article_images.contains(image_key));
            assert_eq!(app.article.image_work.accepts("busy-image", 47), Some(0));
            assert!(app.article.loaded_images.is_empty());
            assert!(
                app.cache
                    .get_for_commit(&sleep_screen::thumbnail_key(image_key, "").unwrap())?
                    .is_some()
            );
            assert!(app.cache.get_for_commit(image_key)?.is_none());
            app.clear_article_image_work();
            app.store
                .upsert_book(&Book::new("network-cover", "Network cover fixture"))?;
            app.register_cover_request(49, "network-cover", "");
            app.cover_result(49, "network-cover", Err(source::SourceError::Network))?;
            assert!(
                app.store
                    .setting::<i64>("cover.failed:network-cover")?
                    .is_some()
            );

            app.jobs.replace_session(source::AuthSession {
                user_vid: "fixture-ui-waiters".into(),
                cookies: BTreeMap::from([("wr_skey".into(), "fixture-ui-cookie".into())]),
                ..Default::default()
            })?;
            app.force_offline = false;
            app.online = true;
            let slots = app.jobs.metadata_capacity();
            for index in 0..slots {
                assert!(
                    app.jobs
                        .search(900 + index as u64, "Unpolled UI waiter fixture".into())
                        .is_none()
                );
            }
            // Exercise production submissions, not a fabricated callback. The
            // pending label must precede synchronous rejection and settlement.
            app.auxiliary_action("settings.renew", "")?;
            assert_eq!(app.settings_status, tr("operation_failed"));
            app.mp_account = Some("fixture-publication".into());
            app.mp_status.clear();
            app.auxiliary_action("mp.sync", "")?;
            assert!(app.mp_status.is_empty());
            assert!(app.online && app.jobs.configured());
            assert!(app.events.try_recv().is_err());
            app.jobs.cancel_all();
            settle_cancelled_metadata(&runtime, &app, slots)?;
            app.mp_account = None;

            app.route = Route::Search;
            app.search_purpose = "books".into();
            app.query.query = "Visible fixture".into();
            app.force_offline = false;
            app.online = true;
            app.search_results = (0..5)
                .map(|index| {
                    let mut book = Book::new(format!("visible-{index}"), "Visible fixture");
                    book.cover_url = "http://127.0.0.1:9/unused-cover".into();
                    BookMetadata::from(&book)
                })
                .collect();
            // This runtime never polls an admitted network operation. Saturate
            // the two slots synchronously, then cancel before its first poll.
            let metadata_slots = app.jobs.metadata_capacity();
            for index in 0..2 {
                app.jobs.cover(
                    100 + index,
                    format!("budget-{index}"),
                    "http://127.0.0.1:9/unused-cover".into(),
                );
            }
            assert_eq!(app.jobs.cover_capacity(), 0);
            app.render()?;
            assert!(app.pending_covers.is_empty());
            assert_eq!(app.deferred_cover_refill, Some(Route::Search));
            assert!(app.events.try_recv().is_err());
            app.jobs.cancel_all();
            settle_cancelled_metadata(&runtime, &app, metadata_slots)?;
            assert_eq!(app.jobs.cover_capacity(), 2);

            app.register_cover_request(199, "visible-0", "http://127.0.0.1:9/unused-cover");
            app.register_cover_request(200, "visible-1", "http://127.0.0.1:9/unused-cover");
            app.jobs.cover(
                200,
                "visible-1".into(),
                "http://127.0.0.1:9/unused-cover".into(),
            );
            let mut png = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(2, 2).write_to(&mut png, image::ImageFormat::Png)?;
            app.cover_result(199, "visible-0", Ok(png.into_inner()))?;
            app.render()?;
            assert!(
                app.cache
                    .get(
                        &sleep_screen::thumbnail_key(
                            "visible-0",
                            "http://127.0.0.1:9/unused-cover"
                        )
                        .unwrap()
                    )?
                    .is_some()
            );
            assert_eq!(app.pending_covers.len(), 2);
            assert!(app.pending_covers.contains("visible-1"));
            assert!(app.pending_covers.contains("visible-2"));
            assert!(!app.pending_covers.contains("visible-3"));
            assert_eq!(app.jobs.cover_capacity(), 0);
            app.jobs.cancel_all();
            app.stop_lighting_hardware()?;
        }
        drop(application);
        drop(ui);
        runtime.shutdown_timeout(Duration::from_secs(1));
        Ok(())
    }

    #[test]
    fn busy_results_release_ui_waiters_and_defer_cover_admission() -> Result<()> {
        const CASE: &str =
            "app::backend::tests::busy_results_release_ui_waiters_and_defer_cover_admission";
        const CHILD: &str = "WEREAD_BACKEND_BUSY_CHILD";
        const COMPLETED: &str = "WEREAD_BACKEND_BUSY_COMPLETED";
        if std::env::var_os(CHILD).is_some() {
            std::thread::spawn(busy_event_case)
                .join()
                .expect("The busy-event fixture thread panicked")?;
            println!("\n{COMPLETED}");
            return Ok(());
        }
        // Slint's process-global event-loop state needs a separate child process.
        let output = std::process::Command::new(std::env::current_exe()?)
            .args(["--exact", CASE, "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .env_remove("WEREAD_API_KEY")
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::ensure!(
            output.status.success()
                && stdout.lines().filter(|line| *line == COMPLETED).count() == 1,
            "The isolated busy-event fixture did not complete: {}\n{stdout}\n{stderr}",
            output.status,
        );
        Ok(())
    }

    fn non_cover_release_case() -> Result<()> {
        slint::platform::set_platform(Box::new(BusyFixtureUi))?;
        let directory = tempfile::tempdir()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let platform = Arc::new(Platform::new(DeviceProfile::Paperwhite6, false)?);
        let ui = AppWindow::new()?;
        crate::platform::configure_window(ui.window(), &platform)?;
        let application = App::attach(
            &ui,
            AppOptions {
                data_dir: directory.path().to_path_buf(),
                profile: DeviceProfile::Paperwhite6,
                demo: false,
                offline: true,
                initial_screen: "library".into(),
                fonts: vec![],
            },
            platform,
            &runtime,
        )?;
        {
            let mut app = application.borrow_mut();
            app.stop_lighting_hardware()?;
            app.route = Route::Search;
            app.search_purpose = "books".into();
            app.query.query = "Deferred fixture".into();
            app.force_offline = false;
            app.online = true;
            app.search_page = 1;
            ui.global::<SearchModel>().set_keyboard_visible(true);
            app.search_results = (0..10)
                .map(|index| {
                    let mut book = Book::new(format!("deferred-{index}"), "Deferred fixture");
                    book.cover_url = "http://127.0.0.1:9/unused-cover".into();
                    BookMetadata::from(&book)
                })
                .collect();
            let metadata_slots = app.jobs.metadata_capacity();
            assert!(metadata_slots > 0);
            for index in 0..metadata_slots {
                app.jobs
                    .catalog(300 + index as u64, format!("metadata-{index}"));
            }
            assert_eq!(app.jobs.metadata_capacity(), 0);
            assert_eq!(app.jobs.cover_capacity(), 0);
            app.render()?;
            assert!(app.pending_covers.is_empty());
            assert_eq!(app.deferred_cover_refill, Some(Route::Search));
            assert!(app.events.try_recv().is_err());

            // Cancel before the runtime first polls these catalog futures. This
            // releases only metadata permits, without any result or cover event.
            app.jobs.cancel_all();
            settle_cancelled_metadata(&runtime, &app, metadata_slots)?;
            assert_eq!(app.jobs.metadata_capacity(), metadata_slots);
            assert_eq!(app.jobs.cover_capacity(), 2);
            assert!(app.events.try_recv().is_err());

            // Only the production tick observes released capacity; no cover
            // completion or render is available to trigger admission here.
            app.tick()?;
            assert_eq!(app.pending_covers.len(), 2);
            assert!(app.pending_covers.contains("deferred-5"));
            assert!(app.pending_covers.contains("deferred-6"));
            assert!(!app.pending_covers.contains("deferred-0"));
            assert!(!app.pending_covers.contains("deferred-7"));
            assert_eq!(app.jobs.cover_capacity(), 0);
            app.tick()?;
            assert_eq!(app.pending_covers.len(), 2);
            assert!(app.events.try_recv().is_err());
            app.jobs.cancel_all();
            app.stop_lighting_hardware()?;
        }
        drop(application);
        drop(ui);
        runtime.shutdown_timeout(Duration::from_secs(1));
        Ok(())
    }

    #[test]
    fn non_cover_release_refills_deferred_search_covers_without_cover_events() -> Result<()> {
        const CASE: &str = "app::backend::tests::non_cover_release_refills_deferred_search_covers_without_cover_events";
        const CHILD: &str = "WEREAD_BACKEND_DEFERRED_COVER_CHILD";
        const COMPLETED: &str = "WEREAD_BACKEND_DEFERRED_COVER_COMPLETED";
        if std::env::var_os(CHILD).is_some() {
            std::thread::spawn(non_cover_release_case)
                .join()
                .expect("The deferred-cover fixture thread panicked")?;
            println!("\n{COMPLETED}");
            return Ok(());
        }
        let output = std::process::Command::new(std::env::current_exe()?)
            .args(["--exact", CASE, "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .env_remove("WEREAD_API_KEY")
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::ensure!(
            output.status.success()
                && stdout.lines().filter(|line| *line == COMPLETED).count() == 1,
            "The isolated deferred-cover fixture did not complete: {}\n{stdout}\n{stderr}",
            output.status,
        );
        Ok(())
    }

    struct CoverViewportControl;

    impl crate::platform::DeviceControl for CoverViewportControl {
        fn native_viewport(&self) -> Option<(u32, u32)> {
            Some((1272, 1536))
        }

        fn capabilities(&self) -> crate::platform::PlatformCapabilities {
            crate::platform::PlatformCapabilities::default()
        }

        fn brightness(&self) -> Result<u8, crate::platform::PlatformError> {
            Err(crate::platform::PlatformError::Unsupported("fixture_light"))
        }

        fn set_brightness(&self, _: u8) -> Result<(), crate::platform::PlatformError> {
            Err(crate::platform::PlatformError::Unsupported("fixture_light"))
        }

        fn warmth(&self) -> Result<u8, crate::platform::PlatformError> {
            Err(crate::platform::PlatformError::Unsupported("fixture_light"))
        }

        fn set_warmth(&self, _: u8) -> Result<(), crate::platform::PlatformError> {
            Err(crate::platform::PlatformError::Unsupported("fixture_light"))
        }

        fn battery_percentage(&self) -> Result<u8, crate::platform::PlatformError> {
            Err(crate::platform::PlatformError::Unsupported(
                "fixture_battery",
            ))
        }

        fn set_inverted(&self, _: bool) -> Result<(), crate::platform::PlatformError> {
            Err(crate::platform::PlatformError::Unsupported(
                "fixture_inverse",
            ))
        }

        fn reset_idle_timer(&self) -> Result<(), crate::platform::PlatformError> {
            Err(crate::platform::PlatformError::Unsupported("fixture_awake"))
        }
    }

    fn occupy_metadata_without_polling(app: &App) -> usize {
        let capacity = app.jobs.metadata_capacity();
        assert!(capacity > 0);
        for index in 0..capacity {
            assert!(
                app.jobs
                    .search(1000 + index as u64, "unpolled fixture".into())
                    .is_none()
            );
        }
        assert_eq!(app.jobs.metadata_capacity(), 0);
        assert_eq!(app.jobs.cover_capacity(), 0);
        capacity
    }

    fn release_unpolled_cover_work(
        runtime: &tokio::runtime::Runtime,
        app: &mut App,
        capacity: usize,
    ) -> Result<()> {
        // Epoch fencing drops every future before its first network poll.
        app.jobs.cancel_all();
        settle_cancelled_metadata(runtime, app, capacity)?;
        // This fixture releases capacity without invalidating the current view's
        // deferred refill intent. Real lifecycle invalidation clears both.
        app.pending_covers.clear();
        app.pending_cover_requests.clear();
        assert!(app.events.try_recv().is_err());
        Ok(())
    }

    fn integrated_visible_cover_case(scenario: &str) -> Result<()> {
        slint::platform::set_platform(Box::new(BusyFixtureUi))?;
        let directory = tempfile::tempdir()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let profile = if scenario == "library" {
            DeviceProfile::Scribe
        } else {
            DeviceProfile::Paperwhite6
        };
        let platform = Arc::new(if scenario == "short-search" {
            // This injected control only supplies geometry. It opens no native
            // device, and every optional hardware operation is unsupported.
            Platform::with_control(profile, true, Arc::new(CoverViewportControl))
        } else {
            Platform::new(profile, false)?
        });
        let ui = AppWindow::new()?;
        crate::platform::configure_window(ui.window(), &platform)?;
        let application = App::attach(
            &ui,
            AppOptions {
                data_dir: directory.path().to_path_buf(),
                profile,
                demo: false,
                offline: true,
                initial_screen: "library".into(),
                fonts: vec![],
            },
            platform,
            &runtime,
        )?;
        {
            let mut app = application.borrow_mut();
            app.stop_lighting_hardware()?;
            app.force_offline = false;
            app.online = true;
            // Keep unrelated periodic work outside this bounded admission fixture.
            app.last_tick = Instant::now() + Duration::from_secs(3600);
            app.device_poll_at = app.last_tick;
            let capacity = app.jobs.metadata_capacity();
            match scenario {
                "library" => {
                    for index in 0..30 {
                        let mut book =
                            Book::new(format!("book-{index:02}"), format!("Book {index:02}"));
                        book.cover_url = "http://127.0.0.1:9/unused-cover".into();
                        app.store.upsert_book(&book)?;
                    }
                    app.prefs.view_mode = "list".into();
                    app.library_sort = "title".into();
                    app.library_page = 1;
                    occupy_metadata_without_polling(&app);
                    app.render()?;
                    let visible: Vec<_> = ui
                        .global::<LibraryModel>()
                        .get_books()
                        .iter()
                        .map(|row| row.id.to_string())
                        .collect();
                    assert_eq!(visible.len(), 10);
                    assert_eq!(visible[0], "book-10");
                    assert!(app.pending_covers.is_empty());
                    assert_eq!(app.deferred_cover_refill, Some(Route::Library));
                    release_unpolled_cover_work(&runtime, &mut app, capacity)?;
                    // A stale search completion does not request a Library render.
                    let epoch = app.jobs.account_epoch();
                    let stale_request = app.search_request;
                    assert!(!app.handle_job(JobEvent {
                        credits: None,
                        request: stale_request,
                        account_epoch: epoch,
                        result: JobResult::Search(Ok(Vec::new())),
                    })?);
                    app.tick()?;
                    assert_eq!(
                        app.pending_covers,
                        BTreeSet::from([visible[0].clone(), visible[1].clone()])
                    );
                    assert!(!app.pending_covers.contains("book-15"));
                    assert_eq!(app.jobs.cover_capacity(), 0);
                }
                "detail" => {
                    let mut book = Book::new("detail-book", "Detail fixture");
                    book.source = "weread".into();
                    app.store.upsert_book(&book)?;
                    app.open_detail(&book.id)?;
                    app.render()?;
                    assert!(app.deferred_cover_refill.is_none());
                    occupy_metadata_without_polling(&app);
                    app.book_requests.insert(book.id.clone(), 42);
                    let epoch = app.jobs.account_epoch();
                    assert!(app.handle_job(JobEvent {
                        credits: None,
                        request: 42,
                        account_epoch: epoch,
                        result: JobResult::BookInfo {
                            book_id: book.id.clone(),
                            result: Ok(source::RemoteBook {
                                id: book.id.clone(),
                                title: book.title.clone(),
                                cover_url: "http://127.0.0.1:9/unused-cover".into(),
                                ..Default::default()
                            }),
                        },
                    })?);
                    app.render()?;
                    assert!(app.pending_covers.is_empty());
                    assert_eq!(
                        app.deferred_cover_refill,
                        Some(Route::Detail(book.id.clone()))
                    );
                    release_unpolled_cover_work(&runtime, &mut app, capacity)?;
                    app.tick()?;
                    assert_eq!(app.pending_covers, BTreeSet::from([book.id]));
                    assert!(app.deferred_cover_refill.is_none());
                }
                "idle" => {
                    let mut png = std::io::Cursor::new(Vec::new());
                    image::DynamicImage::new_rgb8(2, 2)
                        .write_to(&mut png, image::ImageFormat::Png)?;
                    for index in 0..3 {
                        let mut book =
                            Book::new(format!("cached-{index}"), format!("Cached {index}"));
                        book.cover_url = "http://127.0.0.1:9/unused-cover".into();
                        app.store.upsert_book(&book)?;
                        app.cache.put_cover(
                            &sleep_screen::thumbnail_key(&book.id, &book.cover_url).unwrap(),
                            png.get_ref(),
                            400,
                            560,
                        )?;
                    }
                    app.render()?;
                    assert!(app.deferred_cover_refill.is_none());
                    let index =
                        rusqlite::Connection::open(app.cache.root().join("cache-index.sqlite3"))?;
                    let access = || -> Result<i64> {
                        Ok(index.query_row(
                            "SELECT COALESCE(SUM(last_access),0) FROM cache_entries",
                            [],
                            |row| row.get(0),
                        )?)
                    };
                    let initial = access()?;
                    for _ in 0..256 {
                        app.tick()?;
                    }
                    assert_eq!(access()?, initial);
                    // A previously deferred scan becomes idle after finding
                    // that all current objects were cached by another result.
                    app.deferred_cover_refill = Some(Route::Library);
                    app.tick()?;
                    assert!(app.deferred_cover_refill.is_none());
                    let after_scan = access()?;
                    assert!(after_scan > initial);
                    for _ in 0..256 {
                        app.tick()?;
                    }
                    assert_eq!(access()?, after_scan);
                    assert!(app.pending_covers.is_empty());
                    assert_eq!(app.jobs.cover_capacity(), 2);
                }
                "short-search" => {
                    app.route = Route::Search;
                    app.search_purpose = "books".into();
                    app.query.query = "Short viewport fixture".into();
                    app.search_page = 1;
                    ui.global::<SearchModel>().set_keyboard_visible(true);
                    app.search_results = (0..12)
                        .map(|index| {
                            let mut book =
                                Book::new(format!("short-{index}"), "Short viewport fixture");
                            book.cover_url = "http://127.0.0.1:9/unused-cover".into();
                            BookMetadata::from(&book)
                        })
                        .collect();
                    occupy_metadata_without_polling(&app);
                    app.render()?;
                    let visible: Vec<_> = ui
                        .global::<SearchModel>()
                        .get_results()
                        .iter()
                        .map(|row| row.id.to_string())
                        .collect();
                    assert_eq!(
                        visible,
                        vec![
                            "short-4".to_owned(),
                            "short-5".to_owned(),
                            "short-6".to_owned(),
                            "short-7".to_owned()
                        ]
                    );
                    release_unpolled_cover_work(&runtime, &mut app, capacity)?;
                    app.tick()?;
                    assert_eq!(
                        app.pending_covers,
                        BTreeSet::from([visible[0].clone(), visible[1].clone()])
                    );
                    ui.global::<SearchModel>().set_keyboard_visible(false);
                    app.render()?;
                    let visible: Vec<_> = ui
                        .global::<SearchModel>()
                        .get_results()
                        .iter()
                        .map(|row| row.id.to_string())
                        .collect();
                    assert_eq!(visible[0], "short-7");
                    release_unpolled_cover_work(&runtime, &mut app, capacity)?;
                    app.tick()?;
                    assert_eq!(
                        app.pending_covers,
                        BTreeSet::from([visible[0].clone(), visible[1].clone()])
                    );
                    assert!(!app.pending_covers.contains("short-4"));
                }
                _ => anyhow::bail!("Unknown integrated cover scenario"),
            }
            release_unpolled_cover_work(&runtime, &mut app, capacity)?;
            app.stop_lighting_hardware()?;
        }
        drop(application);
        drop(ui);
        runtime.shutdown_timeout(Duration::from_secs(1));
        Ok(())
    }

    #[test]
    fn visible_cover_refill_tracks_library_detail_pagination_and_idle_ticks() -> Result<()> {
        const CASE: &str = "app::backend::tests::visible_cover_refill_tracks_library_detail_pagination_and_idle_ticks";
        const CHILD: &str = "WEREAD_VISIBLE_COVER_SCENARIO";
        const COMPLETED: &str = "WEREAD_VISIBLE_COVER_COMPLETED";
        if let Ok(scenario) = std::env::var(CHILD) {
            let selected = scenario.clone();
            std::thread::spawn(move || integrated_visible_cover_case(&selected))
                .join()
                .expect("The integrated visible-cover fixture panicked")?;
            println!("\n{COMPLETED}:{scenario}");
            return Ok(());
        }
        for scenario in ["library", "detail", "idle", "short-search"] {
            let output = std::process::Command::new(std::env::current_exe()?)
                .args(["--exact", CASE, "--nocapture", "--test-threads=1"])
                .env(CHILD, scenario)
                .env_remove("WEREAD_API_KEY")
                .env_remove("WEREAD_LAUNCH_CONTEXT_DIR")
                .env_remove("WEREAD_LAUNCH_GENERATION")
                .env_remove("WEREAD_LAUNCH_TOKEN")
                .env_remove("WEREAD_RESUME_SESSION")
                .env_remove("WEREAD_NATIVE_RETURN")
                .output()?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let marker = format!("{COMPLETED}:{scenario}");
            anyhow::ensure!(
                output.status.success()
                    && stdout
                        .lines()
                        .filter(|line| *line == marker.as_str())
                        .count()
                        == 1,
                "The integrated cover fixture failed: {scenario}: {}\n{stdout}\n{stderr}",
                output.status,
            );
        }
        Ok(())
    }

    #[test]
    fn reader_history_tracks_uids_and_removed_chapters_return_to_details() {
        let old = [
            Chapter {
                id: "removed".into(),
                book_id: "book".into(),
                index: 0,
                ..Default::default()
            },
            Chapter {
                id: "retained".into(),
                book_id: "book".into(),
                index: 1,
                ..Default::default()
            },
        ]
        .into_iter()
        .map(|chapter| (chapter.id.clone(), chapter))
        .collect();
        let new = vec![
            Chapter {
                id: "inserted".into(),
                book_id: "book".into(),
                index: 0,
                ..Default::default()
            },
            Chapter {
                id: "other".into(),
                book_id: "book".into(),
                index: 1,
                ..Default::default()
            },
            Chapter {
                id: "retained".into(),
                book_id: "book".into(),
                index: 2,
                ..Default::default()
            },
        ];
        let mut history = vec![
            Route::Library,
            Route::Reader("book".into(), 1),
            Route::Reader("book".into(), 0),
            Route::Reader("another-book".into(), 1),
        ];
        let reindex = CatalogReindex::new(&old, &new).unwrap();
        remap_reader_history(&mut history, "book", &reindex);
        assert_eq!(
            history,
            vec![
                Route::Library,
                Route::Reader("book".into(), 2),
                Route::Detail("book".into()),
                Route::Reader("another-book".into(), 1)
            ]
        );
    }
}
