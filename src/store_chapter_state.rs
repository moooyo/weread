//! Bounded, independently durable chapter checkpoints and retry schedules.

use super::*;

const MAX_CHAPTER_STATE_ROWS: usize = 65_536;
const MAX_CHAPTER_ERROR_BYTES: usize = 256;
const CHAPTER_RETRY_DELAYS: [i64; 3] = [60, 120, 240];

impl Store {
    /// Reconcile one selected plan against current UID, body and owner evidence.
    /// Without a fresh batch resource proof, image completions are reopened;
    /// no persisted flag can stand in for the actual cached files.
    pub fn prepare_download_chapter_states(
        &self,
        task: &DownloadTask,
        updated_at: i64,
    ) -> Result<Vec<DownloadChapterState>> {
        let transaction = self.connection.unchecked_transaction()?;
        require_current_task_on(&transaction, self.text_encoding, task, false)?;
        let states = prepare_states_on(&transaction, self.text_encoding, task, None, 0)?;
        let progress = chapter_progress_on(&transaction, &task.id, task.chapters.len())?;
        persist_chapter_progress_on(&transaction, task, progress, updated_at, true)?;
        transaction.commit()?;
        Ok(states)
    }

    /// Keep a restored image completion only after a fresh admitted batch proof.
    pub(crate) fn prepare_download_chapter_states_with_verified_images(
        &self,
        task: &DownloadTask,
        proof: crate::book_assets::VerifiedBookImages,
        updated_at: i64,
    ) -> Result<Vec<DownloadChapterState>> {
        ensure!(
            task.include_images && proof.book_id() == task.book_id,
            "image plan proof does not match the selected task"
        );
        inventory_admit("verified image plan rows", proof.len(), MAX_CATALOG_HEADERS)?;
        let transaction = self.connection.unchecked_transaction()?;
        require_current_task_reserved_on(
            &transaction,
            self.text_encoding,
            task,
            false,
            proof.retained_metadata_bytes(),
        )?;
        let states = prepare_states_on(
            &transaction,
            self.text_encoding,
            task,
            Some(&proof),
            proof.retained_metadata_bytes(),
        )?;
        let progress = chapter_progress_on(&transaction, &task.id, task.chapters.len())?;
        persist_chapter_progress_on(&transaction, task, progress, updated_at, true)?;
        transaction.commit()?;
        Ok(states)
    }

    /// Read admitted task-local metadata without materializing chapter bodies.
    pub fn download_chapter_states(&self, task_id: &str) -> Result<Vec<DownloadChapterState>> {
        validate_state_task_id(task_id)?;
        let transaction = self.connection.unchecked_transaction()?;
        let states = chapter_states_on(&transaction, self.text_encoding, task_id, 0)?;
        transaction.commit()?;
        Ok(states)
    }

    pub fn download_chapter_progress(&self, task_id: &str) -> Result<DownloadChapterProgress> {
        validate_state_task_id(task_id)?;
        let transaction = self.connection.unchecked_transaction()?;
        let task = task_inventory_by_id_on(&transaction, self.text_encoding, task_id)?
            .context("download progress task disappeared")?;
        let progress = match task.status {
            DownloadStatus::Complete => DownloadChapterProgress {
                completed_count: task.chapters.len(),
                next_index: task.chapters.len(),
                ..DownloadChapterProgress::default()
            },
            DownloadStatus::Cancelled => DownloadChapterProgress {
                completed_count: task
                    .completed_count
                    .max(task.next_index)
                    .min(task.chapters.len()),
                next_index: task.next_index.min(task.chapters.len()),
                ..DownloadChapterProgress::default()
            },
            _ => chapter_progress_on(&transaction, task_id, task.chapters.len())?,
        };
        transaction.commit()?;
        Ok(progress)
    }

    /// Explicit continuation resets exhausted or nonretryable chapter failures.
    /// Unexpired retry deadlines and independently completed chapters survive.
    pub fn resume_download_chapters(
        &self,
        task: &DownloadTask,
        updated_at: i64,
    ) -> Result<DownloadChapterProgress> {
        let transaction = self.connection.unchecked_transaction()?;
        require_current_task_on(&transaction, self.text_encoding, task, false)?;
        let initialized: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_chapter_states WHERE task_id=?1)",
            [&task.id],
            |row| row.get(0),
        )?;
        if !initialized {
            prepare_states_on(&transaction, self.text_encoding, task, None, 0)?;
        }
        reset_blocked_chapters_on(&transaction, self.text_encoding, task)?;
        let progress = chapter_progress_on(&transaction, &task.id, task.chapters.len())?;
        persist_chapter_progress_on(&transaction, task, progress, updated_at, true)?;
        transaction.commit()?;
        Ok(progress)
    }

    /// Retry only the blocked subset of the existing running plan. The caller
    /// signals these admitted ordinals after this transaction has committed.
    pub fn resume_blocked_download_chapters(
        &self,
        task: &DownloadTask,
        updated_at: i64,
    ) -> Result<(DownloadChapterProgress, Vec<usize>)> {
        let transaction = self.connection.unchecked_transaction()?;
        require_current_task_on(&transaction, self.text_encoding, task, true)?;
        let ordinals = reset_blocked_chapters_on(&transaction, self.text_encoding, task)?;
        let progress = chapter_progress_on(&transaction, &task.id, task.chapters.len())?;
        persist_chapter_progress_on(&transaction, task, progress, updated_at, false)?;
        transaction.commit()?;
        Ok((progress, ordinals))
    }

    /// Release only an elapsed chapter deadline while retaining its retry budget.
    pub fn start_download_chapter_attempt(
        &self,
        task: &DownloadTask,
        chapter_uid: &str,
        chapter_index: u32,
        ordinal: usize,
        now: i64,
    ) -> Result<DownloadChapterProgress> {
        let transaction = self.connection.unchecked_transaction()?;
        require_current_task_on(&transaction, self.text_encoding, task, true)?;
        require_pending_chapter_on(
            &transaction,
            task,
            chapter_uid,
            chapter_index,
            ordinal,
            Some(now),
            true,
        )?;
        transaction.execute(
            "UPDATE task_chapter_states SET status='pending',retry_not_before=0,error=NULL
             WHERE task_id=?1 AND chapter_uid=?2 AND chapter_index=?3 AND ordinal=?4",
            params![task.id, chapter_uid, chapter_index, ordinal as i64],
        )?;
        let progress = chapter_progress_on(&transaction, &task.id, task.chapters.len())?;
        persist_chapter_progress_on(&transaction, task, progress, now, false)?;
        transaction.commit()?;
        Ok(progress)
    }

    /// The same canonical-body gate is repeated by the transactional commit.
    pub fn validate_independent_download_chapter(
        &self,
        task: &DownloadTask,
        chapter: &Chapter,
        ordinal: usize,
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        validate_independent_chapter_on(
            &transaction,
            self.text_encoding,
            task,
            chapter,
            ordinal,
            None,
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Commit one result immediately, irrespective of earlier pending chapters.
    /// Limited content remains readable but never advances completed progress.
    pub fn commit_independent_download_chapter(
        &self,
        task: &DownloadTask,
        chapter: &Chapter,
        ordinal: usize,
        updated_at: i64,
    ) -> Result<DownloadChapterCommit> {
        ensure!(
            !task.include_images || chapter.content_status != ContentStatus::Full,
            "a full image chapter requires a fresh publication proof"
        );
        self.commit_independent_body(task, chapter, ordinal, None, updated_at)
    }

    /// Bind newly published image resources to this exact incoming body.
    pub(crate) fn commit_verified_image_download_chapter(
        &self,
        task: &DownloadTask,
        chapter: &Chapter,
        ordinal: usize,
        proof: crate::book_assets::VerifiedChapterPublication,
        updated_at: i64,
    ) -> Result<DownloadChapterCommit> {
        ensure!(
            task.include_images && chapter.content_status == ContentStatus::Full,
            "an image publication proof requires a full image chapter"
        );
        self.commit_independent_body(task, chapter, ordinal, Some(proof), updated_at)
    }

    /// Retain authorized text after an incomplete image publication without
    /// treating the requested text-and-images chapter as completed work.
    pub(crate) fn commit_pending_image_download_chapter(
        &self,
        task: &DownloadTask,
        chapter: &Chapter,
        ordinal: usize,
        updated_at: i64,
    ) -> Result<DownloadChapterCommit> {
        ensure!(
            task.include_images && chapter.content_status == ContentStatus::Full,
            "pending image content requires a full image chapter"
        );
        self.commit_independent_body(task, chapter, ordinal, None, updated_at)
    }

    fn commit_independent_body(
        &self,
        task: &DownloadTask,
        chapter: &Chapter,
        ordinal: usize,
        proof: Option<crate::book_assets::VerifiedChapterPublication>,
        updated_at: i64,
    ) -> Result<DownloadChapterCommit> {
        let transaction = self.connection.unchecked_transaction()?;
        let validated = validate_independent_chapter_on(
            &transaction,
            self.text_encoding,
            task,
            chapter,
            ordinal,
            Some(updated_at),
        )?;
        if let Some(proof) = &proof {
            ensure!(
                proof.matches_body(
                    &task.book_id,
                    &chapter.id,
                    chapter.body.as_deref().unwrap_or_default(),
                ),
                "image publication proof has a stale book, UID or canonical body"
            );
        }
        write_download_chapter_on(&transaction, task, chapter, &validated)?;
        if validated.is_full && (!task.include_images || proof.is_some()) {
            complete_chapter_state_on(&transaction, task, &chapter.id, chapter.index, ordinal)?;
        }
        let progress = chapter_progress_on(&transaction, &task.id, task.chapters.len())?;
        persist_chapter_progress_on(&transaction, task, progress, updated_at, false)?;
        transaction.commit()?;
        Ok(DownloadChapterCommit {
            newly_cached: !validated.was_full && validated.is_full,
            progress,
        })
    }

    /// Text-only reuse checks actual SQL content and owner before completion.
    #[allow(clippy::too_many_arguments)]
    pub fn commit_cached_download_chapter(
        &self,
        task: &DownloadTask,
        chapter_uid: &str,
        chapter_index: u32,
        ordinal: usize,
        current_account_key: &str,
        updated_at: i64,
    ) -> Result<DownloadChapterProgress> {
        ensure!(
            !task.include_images,
            "an image checkpoint requires freshly verified resources"
        );
        self.commit_independent_cached_chapter(
            task,
            chapter_uid,
            chapter_index,
            ordinal,
            current_account_key,
            updated_at,
        )
    }

    /// Consume the resource proof immediately, as in the ordered cached API.
    pub(crate) fn commit_verified_cached_image_download_chapter(
        &self,
        task: &DownloadTask,
        proof: crate::book_assets::VerifiedChapterImages,
        chapter_index: u32,
        ordinal: usize,
        current_account_key: &str,
        updated_at: i64,
    ) -> Result<DownloadChapterProgress> {
        ensure!(
            task.include_images && proof.book_id() == task.book_id,
            "verified image checkpoint has a mismatched task or book"
        );
        self.commit_independent_cached_chapter(
            task,
            proof.chapter_uid(),
            chapter_index,
            ordinal,
            current_account_key,
            updated_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_independent_cached_chapter(
        &self,
        task: &DownloadTask,
        chapter_uid: &str,
        chapter_index: u32,
        ordinal: usize,
        current_account_key: &str,
        updated_at: i64,
    ) -> Result<DownloadChapterProgress> {
        ensure!(
            !current_account_key.is_empty() && current_account_key == task.account_key,
            "cached chapter does not match the current owner"
        );
        let transaction = self.connection.unchecked_transaction()?;
        require_current_task_on(&transaction, self.text_encoding, task, true)?;
        require_pending_chapter_on(
            &transaction,
            task,
            chapter_uid,
            chapter_index,
            ordinal,
            Some(updated_at),
            true,
        )?;
        if !cached_chapter_is_full_on(
            &transaction,
            self.text_encoding,
            task,
            chapter_uid,
            chapter_index,
            admit_chapter_task(task)?,
        )? {
            return Err(ContentConflict.into());
        }
        complete_chapter_state_on(&transaction, task, chapter_uid, chapter_index, ordinal)?;
        let progress = chapter_progress_on(&transaction, &task.id, task.chapters.len())?;
        persist_chapter_progress_on(&transaction, task, progress, updated_at, false)?;
        transaction.commit()?;
        Ok(progress)
    }

    /// Persist the chapter's next action before its worker receives an ACK.
    /// A chapter failure never changes the task's lifecycle state.
    #[allow(clippy::too_many_arguments)]
    pub fn record_download_chapter_failure(
        &self,
        task: &DownloadTask,
        chapter_uid: &str,
        chapter_index: u32,
        ordinal: usize,
        retryable: bool,
        error: &str,
        now: i64,
    ) -> Result<DownloadChapterRetry> {
        let error = safe_chapter_error(error);
        let transaction = self.connection.unchecked_transaction()?;
        require_current_task_on(&transaction, self.text_encoding, task, true)?;
        let previous_count = require_pending_chapter_on(
            &transaction,
            task,
            chapter_uid,
            chapter_index,
            ordinal,
            Some(now),
            false,
        )?;
        let count = previous_count.saturating_add(1).min(4);
        let deadline = retryable
            .then(|| CHAPTER_RETRY_DELAYS.get(usize::from(count.saturating_sub(1))))
            .flatten()
            .map(|delay| now.saturating_add(*delay));
        let status = if deadline.is_some() {
            DownloadChapterStatus::RetryWait
        } else {
            DownloadChapterStatus::Blocked
        };
        ensure!(
            transaction.execute(
                "UPDATE task_chapter_states SET status=?5,retry_count=?6,
                 retry_not_before=?7,error=?8
                 WHERE task_id=?1 AND chapter_uid=?2 AND chapter_index=?3 AND ordinal=?4",
                params![
                    task.id,
                    chapter_uid,
                    chapter_index,
                    ordinal as i64,
                    status.as_str(),
                    count,
                    deadline.unwrap_or(0),
                    error
                ],
            )? == 1,
            "chapter failure state disappeared"
        );
        let progress = chapter_progress_on(&transaction, &task.id, task.chapters.len())?;
        persist_chapter_progress_on(&transaction, task, progress, now, false)?;
        transaction.commit()?;
        Ok(match deadline {
            Some(deadline) => DownloadChapterRetry::RetryAt(deadline),
            None => DownloadChapterRetry::Blocked,
        })
    }
}

fn validate_state_task_id(task_id: &str) -> Result<()> {
    inventory_admit("chapter state task identifier", task_id.len(), 4096)?;
    validate_identifier(task_id)
}

fn admit_chapter_task(task: &DownloadTask) -> Result<usize> {
    validate_state_task_id(&task.id)?;
    inventory_admit("chapter state book identifier", task.book_id.len(), 4096)?;
    inventory_admit("chapter state account", task.account_key.len(), 16 * 1024)?;
    inventory_admit(
        "chapter state plan",
        task.chapters.len(),
        MAX_CATALOG_HEADERS,
    )?;
    inventory_admit(
        "chapter state task error",
        task.error.as_ref().map_or(0, String::len),
        4096,
    )?;
    validate_identifier(&task.book_id)?;
    ensure!(
        !task.chapters.is_empty()
            && task.next_index <= task.chapters.len()
            && task.completed_count <= task.chapters.len(),
        "chapter state task has invalid progress"
    );
    let (bytes, work) = task_borrowed_price(task)?;
    let limits = store_inventory_limits();
    inventory_admit("chapter state task metadata", bytes, limits.metadata_bytes)?;
    inventory_admit("chapter state task scratch", bytes, limits.scratch_bytes)?;
    inventory_admit("chapter state task work", work, limits.work_bytes)?;
    ensure!(
        task.chapters.iter().copied().collect::<HashSet<_>>().len() == task.chapters.len(),
        "chapter state task contains duplicate chapters"
    );
    Ok(bytes)
}

fn require_current_task_on(
    connection: &Connection,
    encoding: TextEncoding,
    task: &DownloadTask,
    running: bool,
) -> Result<()> {
    require_current_task_reserved_on(connection, encoding, task, running, 0)
}

fn require_current_task_reserved_on(
    connection: &Connection,
    encoding: TextEncoding,
    task: &DownloadTask,
    running: bool,
    extra_held: usize,
) -> Result<()> {
    let held = inventory_sum(&[admit_chapter_task(task)?, extra_held])?;
    let limits = store_inventory_limits();
    inventory_admit("chapter task held metadata", held, limits.metadata_bytes)?;
    inventory_admit("chapter task held scratch", held, limits.scratch_bytes)?;
    ensure!(
        if running {
            task.status == DownloadStatus::Running
        } else {
            !matches!(
                task.status,
                DownloadStatus::Cancelled | DownloadStatus::Complete
            )
        },
        "chapter operation does not match the task lifecycle"
    );
    let scalar_matches: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM download_tasks WHERE id=?1 AND book_id=?2
         AND status=?3 AND typeof(updated_at)='integer' AND updated_at=?4)",
        params![task.id, task.book_id, task.status.as_str(), task.updated_at],
        |row| row.get(0),
    )?;
    ensure!(
        scalar_matches,
        "chapter operation has a stale task lifecycle"
    );
    let stored = task_inventory_by_id_reserved_on(connection, encoding, &task.id, held, held)?;
    ensure!(
        stored.as_ref() == Some(task),
        "chapter operation has a stale task or plan"
    );
    Ok(())
}

fn chapter_states_on(
    connection: &Connection,
    encoding: TextEncoding,
    task_id: &str,
    held: usize,
) -> Result<Vec<DownloadChapterState>> {
    let (count, bytes, cells, largest_uid, largest_error, valid):
        (i64, i64, i64, i64, i64, bool) = connection.query_row(
        "SELECT COUNT(*),COALESCE(SUM(uid_bytes+error_bytes),0),
         COALESCE(SUM((uid_bytes>0)+(error_bytes>0)),0),COALESCE(MAX(uid_bytes),0),
         COALESCE(MAX(error_bytes),0),COALESCE(MIN(valid),1) FROM
         (SELECT COALESCE(octet_length(chapter_uid),0) AS uid_bytes,
            COALESCE(octet_length(error),0) AS error_bytes,
            typeof(chapter_uid)='text' AND typeof(ordinal)='integer'
            AND ordinal>=0 AND ordinal<8192 AND typeof(chapter_index)='integer'
            AND chapter_index>=0 AND chapter_index<=4294967295
            AND typeof(status)='text' AND status IN ('pending','complete','retry_wait','blocked')
            AND typeof(retry_count)='integer' AND retry_count>=0 AND retry_count<=4
            AND typeof(retry_not_before)='integer' AND (error IS NULL OR typeof(error)='text') AS valid
          FROM task_chapter_states WHERE task_id=?1 LIMIT 8193)",
        [task_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
    )?;
    let count = usize::try_from(count).map_err(|_| StoreInventoryUnavailable)?;
    ensure!(valid, "chapter state has invalid persisted metadata");
    let limits = store_inventory_limits();
    inventory_admit(
        "chapter state rows",
        count,
        MAX_CATALOG_HEADERS.min(limits.rows),
    )?;
    inventory_admit(
        "chapter state UID bytes",
        usize::try_from(largest_uid).map_err(|_| StoreInventoryUnavailable)?,
        encoding.maximum_encoded(4096)? as usize,
    )?;
    inventory_admit(
        "chapter state error bytes",
        usize::try_from(largest_error).map_err(|_| StoreInventoryUnavailable)?,
        encoding.maximum_encoded(MAX_CHAPTER_ERROR_BYTES)? as usize,
    )?;
    let raw = usize::try_from(bytes).map_err(|_| StoreInventoryUnavailable)?;
    let upper = encoding.utf8_upper_total(
        raw,
        usize::try_from(cells).map_err(|_| StoreInventoryUnavailable)?,
    )?;
    let metadata = inventory_sum(&[
        held,
        upper.checked_mul(2).ok_or(StoreInventoryLimit {
            resource: "chapter state bytes",
        })?,
        count.checked_mul(512).ok_or(StoreInventoryLimit {
            resource: "chapter state rows",
        })?,
    ])?;
    inventory_admit("chapter state encoded bytes", raw, limits.encoded_bytes)?;
    inventory_admit("chapter state metadata", metadata, limits.metadata_bytes)?;
    inventory_admit(
        "chapter state scratch",
        inventory_sum(&[metadata, upper])?,
        limits.scratch_bytes,
    )?;
    inventory_admit(
        "chapter state work",
        inventory_sum(&[
            metadata,
            upper.checked_mul(4).ok_or(StoreInventoryLimit {
                resource: "chapter state work",
            })?,
        ])?,
        limits.work_bytes,
    )?;
    let mut statement = connection.prepare(
        "SELECT chapter_uid,ordinal,chapter_index,status,retry_count,retry_not_before,error
         FROM task_chapter_states WHERE task_id=?1 ORDER BY ordinal",
    )?;
    let mut rows = statement.query([task_id])?;
    let mut states = Vec::with_capacity(count);
    while let Some(row) = rows.next()? {
        ensure!(states.len() < count, "chapter state snapshot changed");
        let chapter_uid = cache_text(row, 0, 4096)?;
        validate_identifier(chapter_uid)?;
        let status = DownloadChapterStatus::parse(cache_text(row, 3, 32)?)
            .context("invalid chapter state status")?;
        let error = if matches!(row.get_ref(6)?, rusqlite::types::ValueRef::Null) {
            None
        } else {
            Some(safe_chapter_error(cache_text(row, 6, MAX_CHAPTER_ERROR_BYTES)?).to_owned())
        };
        states.push(DownloadChapterState {
            chapter_uid: chapter_uid.to_owned(),
            ordinal: row.get(1)?,
            chapter_index: row.get(2)?,
            status,
            retry_count: row.get(4)?,
            retry_not_before: row.get(5)?,
            error,
        });
    }
    Ok(states)
}

fn prepare_states_on(
    connection: &Connection,
    encoding: TextEncoding,
    task: &DownloadTask,
    verified_images: Option<&crate::book_assets::VerifiedBookImages>,
    extra_held: usize,
) -> Result<Vec<DownloadChapterState>> {
    let held = inventory_sum(&[admit_chapter_task(task)?, extra_held])?;
    let old = chapter_states_on(connection, encoding, &task.id, held)?;
    let legacy = old.is_empty();
    let limits = store_inventory_limits();
    let mut metadata = held;
    for state in &old {
        metadata = inventory_sum(&[
            metadata,
            state.chapter_uid.len(),
            state.error.as_ref().map_or(0, String::len),
            512,
        ])?;
    }
    inventory_admit(
        "retained chapter state metadata",
        metadata,
        limits.metadata_bytes,
    )?;
    inventory_admit(
        "retained chapter state scratch",
        metadata,
        limits.scratch_bytes,
    )?;
    let mut selected = Vec::with_capacity(task.chapters.len());
    let mut body_work = 0usize;
    let mut largest_body_scratch = 0usize;
    // Admit the selected UID copies before retaining any catalog strings.
    for (ordinal, index) in task.chapters.iter().copied().enumerate() {
        let header: Option<(i64, bool, i64, i64)> = connection
            .query_row(
                "SELECT rowid,typeof(source_id)='text',COALESCE(octet_length(source_id),0),
               COALESCE(octet_length(body),0)
             FROM chapters WHERE book_id=?1 AND chapter_index=?2",
                params![task.book_id, index],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let (rowid, valid, raw, body_raw) =
            header.context("selected download chapter disappeared")?;
        ensure!(valid, "selected chapter UID is unavailable");
        let raw = usize::try_from(raw).map_err(|_| StoreInventoryUnavailable)?;
        let upper = encoding.utf8_upper(raw)?;
        inventory_admit("selected chapter UID", upper, 4096)?;
        metadata = inventory_sum(&[
            metadata,
            upper.checked_mul(4).ok_or(StoreInventoryLimit {
                resource: "chapter state copies",
            })?,
            1024,
        ])?;
        let body_upper = encoding
            .utf8_upper(usize::try_from(body_raw).map_err(|_| StoreInventoryUnavailable)?)?;
        body_work = inventory_sum(&[
            body_work,
            body_upper.checked_mul(4).ok_or(StoreInventoryLimit {
                resource: "chapter state body work",
            })?,
        ])?;
        largest_body_scratch =
            largest_body_scratch.max(body_upper.checked_mul(3).ok_or(StoreInventoryLimit {
                resource: "chapter state body scratch",
            })?);
        inventory_admit(
            "selected chapter state metadata",
            metadata,
            limits.metadata_bytes,
        )?;
        inventory_admit(
            "selected chapter state scratch",
            inventory_sum(&[metadata, largest_body_scratch])?,
            limits.scratch_bytes,
        )?;
        inventory_admit(
            "selected chapter state work",
            inventory_sum(&[metadata, body_work])?,
            limits.work_bytes,
        )?;
        selected.push((ordinal, index, rowid, upper));
    }
    let retained: i64 = connection.query_row(
        "SELECT COUNT(*) FROM (SELECT 1 FROM task_chapter_states WHERE task_id<>?1 LIMIT 65537)",
        [&task.id],
        |row| row.get(0),
    )?;
    inventory_admit(
        "retained chapter states",
        inventory_sum(&[
            usize::try_from(retained).map_err(|_| StoreInventoryUnavailable)?,
            selected.len(),
        ])?,
        MAX_CHAPTER_STATE_ROWS.min(limits.rows),
    )?;
    let old_by_uid: HashMap<_, _> = old
        .iter()
        .map(|state| (state.chapter_uid.as_str(), state))
        .collect();
    let mut seen = HashSet::new();
    let mut states = Vec::with_capacity(selected.len());
    for (ordinal, index, rowid, upper) in selected {
        let uid: String = connection.query_row(
            "SELECT source_id FROM chapters WHERE rowid=?1",
            [rowid],
            |row| row.get(0),
        )?;
        inventory_admit("selected chapter UID", uid.len(), upper)?;
        validate_identifier(&uid)?;
        ensure!(
            seen.insert(uid.clone()),
            "selected download UIDs are ambiguous"
        );
        let mut state = old_by_uid
            .get(uid.as_str())
            .map(|state| (**state).clone())
            .unwrap_or_else(|| DownloadChapterState {
                chapter_uid: uid.clone(),
                chapter_index: index,
                ordinal,
                ..DownloadChapterState::default()
            });
        state.chapter_index = index;
        state.ordinal = ordinal;
        if legacy
            && task.retry_cursor == task.next_index
            && ordinal == task.retry_cursor
            && task.retry_count > 0
        {
            state.retry_count = task.retry_count.min(4);
            state.retry_not_before = if task.retry_count <= 3 {
                task.retry_not_before
            } else {
                0
            };
            state.status = if task.retry_count <= 3 {
                DownloadChapterStatus::RetryWait
            } else {
                DownloadChapterStatus::Blocked
            };
            state.error = Some("source.api.-10102".into());
        }
        let prior_complete =
            state.status == DownloadChapterStatus::Complete || legacy && ordinal < task.next_index;
        let resources_complete =
            !task.include_images || verified_images.is_some_and(|proof| proof.contains(&uid));
        let complete = resources_complete
            && cached_chapter_is_full_on(connection, encoding, task, &uid, index, metadata)?;
        if complete || prior_complete {
            state.status = if complete {
                DownloadChapterStatus::Complete
            } else {
                DownloadChapterStatus::Pending
            };
            state.retry_count = 0;
            state.retry_not_before = 0;
            state.error = None;
        }
        states.push(state);
    }
    connection.execute(
        "DELETE FROM task_chapter_states WHERE task_id=?1",
        [&task.id],
    )?;
    for state in &states {
        connection.execute(
            "INSERT INTO task_chapter_states(task_id,chapter_uid,ordinal,chapter_index,status,retry_count,retry_not_before,error)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![task.id,state.chapter_uid,state.ordinal as i64,state.chapter_index,
                state.status.as_str(),state.retry_count,state.retry_not_before,state.error],
        )?;
    }
    Ok(states)
}

fn chapter_progress_on(
    connection: &Connection,
    task_id: &str,
    selected_count: usize,
) -> Result<DownloadChapterProgress> {
    inventory_admit(
        "chapter progress selection",
        selected_count,
        MAX_CATALOG_HEADERS,
    )?;
    let (count,complete,pending,waiting,blocked,prefix,deadline,valid):
        (i64,i64,i64,i64,i64,Option<i64>,Option<i64>,bool) = connection.query_row(
        "SELECT COUNT(*),COALESCE(SUM(status='complete'),0),COALESCE(SUM(status='pending'),0),
         COALESCE(SUM(status='retry_wait'),0),COALESCE(SUM(status='blocked'),0),
         MIN(CASE WHEN status<>'complete' THEN ordinal END),
         MIN(CASE WHEN status='retry_wait' THEN retry_not_before END),
         COALESCE(MIN(typeof(ordinal)='integer' AND ordinal>=0 AND ordinal<?2
           AND status IN ('pending','complete','retry_wait','blocked')
           AND typeof(retry_not_before)='integer'),1)
         FROM (SELECT ordinal,status,retry_not_before FROM task_chapter_states WHERE task_id=?1 LIMIT 8193)",
        params![task_id,selected_count as i64],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)),
    )?;
    ensure!(valid, "chapter progress metadata is invalid");
    let count = usize::try_from(count).map_err(|_| StoreInventoryUnavailable)?;
    if count == 0 {
        return Ok(DownloadChapterProgress {
            pending_count: selected_count,
            ..DownloadChapterProgress::default()
        });
    }
    ensure!(
        count == selected_count,
        "chapter states do not match the selected plan"
    );
    Ok(DownloadChapterProgress {
        completed_count: usize::try_from(complete).map_err(|_| StoreInventoryUnavailable)?,
        next_index: usize::try_from(prefix.unwrap_or(count as i64))
            .map_err(|_| StoreInventoryUnavailable)?,
        pending_count: usize::try_from(pending).map_err(|_| StoreInventoryUnavailable)?,
        retry_wait_count: usize::try_from(waiting).map_err(|_| StoreInventoryUnavailable)?,
        blocked_count: usize::try_from(blocked).map_err(|_| StoreInventoryUnavailable)?,
        retry_not_before: deadline,
    })
}

fn persist_chapter_progress_on(
    connection: &Connection,
    task: &DownloadTask,
    progress: DownloadChapterProgress,
    updated_at: i64,
    clear_legacy: bool,
) -> Result<()> {
    let changed = if clear_legacy {
        connection.execute(
            "UPDATE download_tasks SET updated_at=?4,payload=json_set(payload,
             '$.next_index',?2,'$.completed_count',?3,'$.updated_at',?4,
             '$.error',NULL,'$.retry_cursor',0,'$.retry_count',0,'$.retry_not_before',0) WHERE id=?1",
            params![task.id,progress.next_index as i64,progress.completed_count as i64,updated_at],
        )?
    } else {
        connection.execute(
            "UPDATE download_tasks SET updated_at=?4,payload=json_set(payload,
             '$.next_index',?2,'$.completed_count',?3,'$.updated_at',?4) WHERE id=?1",
            params![
                task.id,
                progress.next_index as i64,
                progress.completed_count as i64,
                updated_at
            ],
        )?
    };
    ensure!(changed == 1, "chapter progress task disappeared");
    Ok(())
}

fn reset_blocked_chapters_on(
    connection: &Connection,
    encoding: TextEncoding,
    task: &DownloadTask,
) -> Result<Vec<usize>> {
    let held = admit_chapter_task(task)?;
    let (count, blocked, raw, largest, valid): (i64, i64, i64, i64, bool) =
        connection.query_row(
            "SELECT COUNT(*),COALESCE(SUM(status='blocked'),0),
             COALESCE(SUM(uid_bytes+error_bytes),0),
             COALESCE(MAX(uid_bytes+error_bytes),0),COALESCE(MIN(valid),1) FROM
             (SELECT status,COALESCE(octet_length(chapter_uid),0) AS uid_bytes,
                COALESCE(octet_length(error),0) AS error_bytes,
                typeof(ordinal)='integer' AND ordinal>=0 AND ordinal<?2
                AND typeof(chapter_index)='integer' AND chapter_index>=0
                AND chapter_index<=4294967295 AND typeof(chapter_uid)='text'
                AND COALESCE(octet_length(chapter_uid),0)<=?3
                AND typeof(status)='text' AND status IN ('pending','complete','retry_wait','blocked')
                AND typeof(retry_count)='integer' AND retry_count>=0 AND retry_count<=4
                AND typeof(retry_not_before)='integer'
                AND (error IS NULL OR typeof(error)='text')
                AND COALESCE(octet_length(error),0)<=?4 AS valid
              FROM task_chapter_states WHERE task_id=?1 LIMIT 8193)",
            params![task.id, task.chapters.len() as i64,
                encoding.maximum_encoded(4096)?,
                encoding.maximum_encoded(MAX_CHAPTER_ERROR_BYTES)?],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )?;
    ensure!(valid, "blocked chapter retry has invalid stored metadata");
    let count = usize::try_from(count).map_err(|_| StoreInventoryUnavailable)?;
    ensure!(
        count == task.chapters.len(),
        "blocked chapter retry has a stale plan"
    );
    let blocked = usize::try_from(blocked).map_err(|_| StoreInventoryUnavailable)?;
    let raw = usize::try_from(raw).map_err(|_| StoreInventoryUnavailable)?;
    let largest =
        encoding.utf8_upper(usize::try_from(largest).map_err(|_| StoreInventoryUnavailable)?)?;
    let limits = store_inventory_limits();
    inventory_admit(
        "blocked chapter retry rows",
        count,
        MAX_CATALOG_HEADERS.min(limits.rows),
    )?;
    inventory_admit(
        "blocked chapter retry ordinals",
        blocked,
        task.chapters.len(),
    )?;
    inventory_admit(
        "blocked chapter retry encoded bytes",
        raw,
        limits.encoded_bytes,
    )?;
    let output = inventory_sum(&[
        held,
        blocked
            .checked_mul(2 * std::mem::size_of::<usize>())
            .ok_or(StoreInventoryLimit {
                resource: "blocked chapter retry output",
            })?,
        128,
    ])?;
    inventory_admit(
        "blocked chapter retry metadata",
        output,
        limits.metadata_bytes,
    )?;
    inventory_admit(
        "blocked chapter retry work",
        inventory_sum(&[
            output,
            raw.checked_mul(4).ok_or(StoreInventoryLimit {
                resource: "blocked chapter retry work",
            })?,
        ])?,
        limits.work_bytes,
    )?;
    inventory_admit(
        "blocked chapter retry scratch",
        inventory_sum(&[
            output,
            largest.checked_mul(3).ok_or(StoreInventoryLimit {
                resource: "blocked chapter retry scratch",
            })?,
        ])?,
        limits.scratch_bytes,
    )?;
    let mut ordinals = Vec::with_capacity(blocked);
    {
        let mut statement = connection.prepare(
            "SELECT ordinal,chapter_index FROM task_chapter_states
             WHERE task_id=?1 AND status='blocked' ORDER BY ordinal",
        )?;
        let mut rows = statement.query([&task.id])?;
        while let Some(row) = rows.next()? {
            ensure!(
                ordinals.len() < blocked,
                "blocked chapter retry snapshot changed"
            );
            let ordinal: usize = row.get(0)?;
            let index: u32 = row.get(1)?;
            ensure!(
                task.chapters.get(ordinal) == Some(&index),
                "blocked chapter retry no longer matches the selected chapter"
            );
            ordinals.push(ordinal);
        }
    }
    ensure!(
        ordinals.len() == blocked,
        "blocked chapter retry snapshot changed"
    );
    let reset = connection.execute(
        "UPDATE task_chapter_states SET status='pending',retry_count=0,
         retry_not_before=0,error=NULL WHERE task_id=?1 AND status='blocked'",
        [&task.id],
    )?;
    ensure!(reset == blocked, "blocked chapter retry snapshot changed");
    Ok(ordinals)
}

fn require_pending_chapter_on(
    connection: &Connection,
    task: &DownloadTask,
    uid: &str,
    index: u32,
    ordinal: usize,
    now: Option<i64>,
    require_catalog: bool,
) -> Result<u8> {
    inventory_admit("chapter checkpoint UID", uid.len(), 4096)?;
    validate_identifier(uid)?;
    ensure!(
        task.chapters.get(ordinal) == Some(&index),
        "chapter packet is outside the task selection"
    );
    let state: Option<(bool, i64, i64, bool)> = connection
        .query_row(
            "SELECT status IN ('pending','retry_wait') AND typeof(retry_count)='integer'
           AND retry_count>=0 AND retry_count<=4 AND typeof(retry_not_before)='integer',
           retry_count,CASE WHEN status='retry_wait' THEN retry_not_before ELSE 0 END,
           EXISTS(SELECT 1 FROM chapters WHERE book_id=?5 AND chapter_index=?3 AND source_id=?2)
         FROM task_chapter_states AS s
         WHERE task_id=?1 AND chapter_uid=?2 AND chapter_index=?3 AND ordinal=?4",
            params![task.id, uid, index, ordinal as i64, task.book_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let (valid, count, deadline, catalog_matches) =
        state.context("chapter packet has stale UID or state")?;
    ensure!(valid, "chapter packet is already complete or blocked");
    if require_catalog && !catalog_matches {
        return Err(ContentConflict.into());
    }
    ensure!(
        now.is_none_or(|now| now >= deadline),
        "chapter retry deadline has not elapsed"
    );
    Ok(u8::try_from(count).context("invalid chapter retry count")?)
}

fn complete_chapter_state_on(
    connection: &Connection,
    task: &DownloadTask,
    uid: &str,
    index: u32,
    ordinal: usize,
) -> Result<()> {
    ensure!(connection.execute(
        "UPDATE task_chapter_states SET status='complete',retry_count=0,retry_not_before=0,error=NULL
         WHERE task_id=?1 AND chapter_uid=?2 AND chapter_index=?3 AND ordinal=?4
           AND status IN ('pending','retry_wait')",
        params![task.id,uid,index,ordinal as i64],
    )? == 1, "chapter completion state disappeared");
    Ok(())
}

fn cached_chapter_is_full_on(
    connection: &Connection,
    encoding: TextEncoding,
    task: &DownloadTask,
    uid: &str,
    index: u32,
    held: usize,
) -> Result<bool> {
    let metadata: Option<(i64, bool, i64, i64, i64)> = connection
        .query_row(
            "SELECT rowid,typeof(content_status)='text' AND typeof(content_account_key)='text'
           AND typeof(body)='text',COALESCE(octet_length(body),0),
           COALESCE(octet_length(content_status),0),COALESCE(octet_length(content_account_key),0)
         FROM chapters WHERE book_id=?1 AND chapter_index=?2 AND source_id=?3",
            params![task.book_id, index, uid],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    let Some((rowid, valid, raw, status_raw, owner_raw)) = metadata else {
        return Ok(false);
    };
    if !valid || task.account_key.is_empty() {
        return Ok(false);
    }
    inventory_admit(
        "cached chapter status",
        encoding.utf8_upper(usize::try_from(status_raw).map_err(|_| StoreInventoryUnavailable)?)?,
        32,
    )?;
    inventory_admit(
        "cached chapter owner",
        encoding.utf8_upper(usize::try_from(owner_raw).map_err(|_| StoreInventoryUnavailable)?)?,
        16 * 1024,
    )?;
    admit_chapter_body_work(
        encoding,
        usize::try_from(raw).map_err(|_| StoreInventoryUnavailable)?,
        held,
    )?;
    Ok(connection.query_row(
        "SELECT content_status='full' AND content_account_key=?3
           AND CASE WHEN body GLOB ?2 THEN 1 ELSE instr(body,char(0))>0 END
         FROM chapters WHERE rowid=?1",
        params![rowid, NONBLANK_BODY_PATTERN, task.account_key],
        |row| row.get(0),
    )?)
}

fn admit_chapter_body_work(encoding: TextEncoding, raw: usize, held: usize) -> Result<usize> {
    let limits = store_inventory_limits();
    let upper = encoding.utf8_upper(raw)?;
    inventory_admit("chapter checkpoint body bytes", raw, limits.encoded_bytes)?;
    inventory_admit(
        "chapter checkpoint body work",
        inventory_sum(&[
            held,
            upper.checked_mul(4).ok_or(StoreInventoryLimit {
                resource: "chapter body work",
            })?,
        ])?,
        limits.work_bytes,
    )?;
    inventory_admit(
        "chapter checkpoint body scratch",
        inventory_sum(&[
            held,
            upper.checked_mul(3).ok_or(StoreInventoryLimit {
                resource: "chapter body scratch",
            })?,
        ])?,
        limits.scratch_bytes,
    )?;
    Ok(upper)
}

fn validate_independent_chapter_on(
    connection: &Connection,
    encoding: TextEncoding,
    task: &DownloadTask,
    chapter: &Chapter,
    ordinal: usize,
    now: Option<i64>,
) -> Result<ValidatedDownloadChapter> {
    require_current_task_on(connection, encoding, task, true)?;
    require_pending_chapter_on(
        connection,
        task,
        &chapter.id,
        chapter.index,
        ordinal,
        now,
        true,
    )?;
    let held = inventory_sum(&[
        admit_chapter_task(task)?,
        chapter
            .body
            .as_ref()
            .map_or(0, String::len)
            .checked_mul(3)
            .ok_or(StoreInventoryLimit {
                resource: "incoming chapter body",
            })?,
        chapter.title.len(),
    ])?;
    let header: (bool, i64, i64, i64, i64) = connection.query_row(
        "SELECT (body IS NULL OR typeof(body)='text') AND typeof(content_status)='text'
           AND typeof(content_account_key)='text' AND typeof(b.source)='text',
           COALESCE(octet_length(c.body),0),COALESCE(octet_length(c.content_status),0),
           COALESCE(octet_length(c.content_account_key),0),COALESCE(octet_length(b.source),0)
         FROM chapters c JOIN books b ON b.id=c.book_id
         WHERE c.book_id=?1 AND c.chapter_index=?2 AND c.source_id=?3",
        params![task.book_id, chapter.index, chapter.id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    ensure!(header.0, "stored chapter evidence is unavailable");
    for (raw, maximum) in [(header.2, 32usize), (header.3, 16 * 1024), (header.4, 4096)] {
        inventory_admit(
            "chapter proof metadata",
            encoding.utf8_upper(usize::try_from(raw).map_err(|_| StoreInventoryUnavailable)?)?,
            maximum,
        )?;
    }
    admit_chapter_body_work(
        encoding,
        usize::try_from(header.1).map_err(|_| StoreInventoryUnavailable)?,
        held,
    )?;
    let old = connection.query_row(
        "SELECT c.body,c.content_status,c.content_account_key,b.source
         FROM chapters c JOIN books b ON b.id=c.book_id
         WHERE c.book_id=?1 AND c.chapter_index=?2 AND c.source_id=?3",
        params![task.book_id, chapter.index, chapter.id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    validate_download_content_on(connection, task, chapter, old)
}

fn safe_chapter_error(error: &str) -> &str {
    const SAFE_ERRORS: &[&str] = &[
        "source.api_key_required",
        "source.web_login_required",
        "source.login_expired",
        "source.timeout",
        "source.network_unavailable",
        "source.busy",
        "source.integrity_check_failed",
        "source.response_too_large",
        "source.delivery_uncertain",
        "source.local_checkpoint_failed",
        "source.unlock_required",
        "source.content_unverified",
        "source.content_changed",
        "source.invalid_response",
        "source.unsupported",
    ];
    let numeric_code = ["source.api.", "source.http.", "source.resource_http."]
        .iter()
        .find_map(|prefix| error.strip_prefix(prefix))
        .is_some_and(|code| {
            let digits = code.strip_prefix('-').unwrap_or(code);
            !digits.is_empty()
                && digits.len() <= 16
                && digits.bytes().all(|byte| byte.is_ascii_digit())
        });
    if error.len() <= MAX_CHAPTER_ERROR_BYTES && (SAFE_ERRORS.contains(&error) || numeric_code) {
        error
    } else if error.starts_with("source.invalid_response.") {
        "source.invalid_response"
    } else if error.starts_with("source.unsupported.") {
        "source.unsupported"
    } else {
        "source.local_checkpoint_failed"
    }
}

/// Called only after the parent task passes its existing write admission.
/// Historical schema fixtures may not have the lazy-state table yet.
pub(super) fn clean_states_before_task_write_on(
    connection: &Connection,
    task: &DownloadTask,
) -> Result<()> {
    let present: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='task_chapter_states')",
        [], |row| row.get(0),
    )?;
    if !present {
        return Ok(());
    }
    if matches!(
        task.status,
        DownloadStatus::Complete | DownloadStatus::Cancelled
    ) {
        connection.execute(
            "DELETE FROM task_chapter_states WHERE task_id=?1",
            [&task.id],
        )?;
    } else {
        let initialized: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_chapter_states WHERE task_id=?1)",
            [&task.id],
            |row| row.get(0),
        )?;
        if initialized {
            let (held, work) = task_borrowed_price(task)?;
            // A guarded lifecycle save can also retain its previous snapshot
            // and the selected durable DTO until this final persistence call.
            let held = held.checked_mul(3).ok_or(StoreInventoryLimit {
                resource: "task cleanup held metadata",
            })?;
            let previous = task_inventory_by_id_reserved_on(
                connection,
                TextEncoding::read(connection)?,
                &task.id,
                held,
                work,
            )?;
            if previous.is_none_or(|previous| {
                previous.book_id != task.book_id
                    || previous.account_key != task.account_key
                    || previous.include_images != task.include_images
            }) {
                connection.execute(
                    "DELETE FROM task_chapter_states WHERE task_id=?1",
                    [&task.id],
                )?;
            }
        }
    }
    Ok(())
}

pub(super) fn validate_normalized_completion_on(
    connection: &Connection,
    task: &DownloadTask,
) -> Result<()> {
    let (count, complete): (i64, i64) = connection.query_row(
        "SELECT COUNT(*),COALESCE(SUM(status='complete'),0)
         FROM (SELECT status FROM task_chapter_states WHERE task_id=?1 LIMIT 8193)",
        [&task.id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if count > 0 {
        ensure!(
            count == task.chapters.len() as i64
                && complete == count
                && task.completed_count == task.chapters.len(),
            "incomplete chapter states prevent task completion"
        );
    }
    Ok(())
}

pub(super) fn reconcile_catalog_states_on(
    connection: &Connection,
    encoding: TextEncoding,
    task: &mut DownloadTask,
    extra_held: usize,
) -> Result<()> {
    task.completed_count = task.completed_count.min(task.chapters.len());
    if matches!(
        task.status,
        DownloadStatus::Complete | DownloadStatus::Cancelled
    ) {
        return Ok(());
    }
    let initialized: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM task_chapter_states WHERE task_id=?1)",
        [&task.id],
        |row| row.get(0),
    )?;
    if initialized {
        prepare_states_on(connection, encoding, task, None, extra_held)?;
        let progress = chapter_progress_on(connection, &task.id, task.chapters.len())?;
        task.next_index = progress.next_index;
        task.completed_count = progress.completed_count;
    }
    Ok(())
}
