//! Durable chapter recovery and compact download status controls.

use super::*;

pub(super) const REJECTION_CODE: &str = "source.api.-10102";
const NOTICE_KEY: &str = "download.notice.v1";

pub(super) struct DownloadNotice {
    task_id: String,
    book_id: String,
    label: String,
    waiting: bool,
    blocked: bool,
}

pub(super) fn rejected_read(error: &source::SourceError) -> bool {
    matches!(error, source::SourceError::Api(code) if code == "-10102")
}

pub(super) fn recorded_rejection(task: &DownloadTask) -> bool {
    task.error.as_deref() == Some(REJECTION_CODE)
}

/// Legacy task fields are retained only for migration, never active scheduling.
pub(super) fn reset_recovery(task: &mut DownloadTask) {
    task.retry_cursor = 0;
    task.retry_count = 0;
    task.retry_not_before = 0;
}

pub(super) fn chapter_summary_label(
    completed: usize,
    total: usize,
    waiting: usize,
    blocked: usize,
) -> String {
    i18n::format(
        "downloading_chapter_states",
        &[
            ("current", completed.to_string()),
            ("total", total.to_string()),
            ("waiting", waiting.to_string()),
            ("blocked", blocked.to_string()),
        ],
    )
}

impl App {
    /// Settings retain one small identifier; task and chapter admission remain
    /// bounded by Store and scoped to the current authenticated content owner.
    pub(super) fn restore_download_notice(&mut self) {
        let Some(owner) = self.chapter_content_owner() else {
            return;
        };
        let selected = (|| -> Result<Option<DownloadTask>> {
            if let Some(id) = self
                .store
                .bounded_setting::<String>(NOTICE_KEY, 32 * 1024)?
            {
                if id.len() > 4096 {
                    return Ok(None);
                }
                return self.store.task(&id);
            }
            self.store.latest_download_notice_candidate()
        })();
        let Ok(Some(task)) = selected else {
            return;
        };
        if task.account_key == owner
            && !matches!(
                task.status,
                DownloadStatus::Cancelled | DownloadStatus::Complete
            )
        {
            let _ = self.remember_download_notice(&task);
        }
    }

    pub(super) fn remember_download_notice(&mut self, task: &DownloadTask) -> Result<()> {
        let progress = self.store.download_chapter_progress(&task.id)?;
        let legacy =
            recorded_rejection(task) && self.store.download_chapter_states(&task.id)?.is_empty();
        let waiting = progress.retry_wait_count.max(usize::from(
            legacy && task.retry_count > 0 && task.retry_count <= 3,
        ));
        let blocked = progress
            .blocked_count
            .max(usize::from(legacy && waiting == 0));
        self.sync_running_download_actions(blocked);
        if waiting == 0 && blocked == 0 {
            return self.clear_download_notice(&task.id);
        }
        if !self
            .download_notice
            .as_ref()
            .is_some_and(|notice| notice.task_id == task.id)
        {
            self.store.set_setting(NOTICE_KEY, &task.id)?;
        }
        self.download_notice = Some(DownloadNotice {
            task_id: task.id.clone(),
            book_id: task.book_id.clone(),
            label: chapter_summary_label(
                task.completed_count,
                task.chapters.len(),
                waiting,
                blocked,
            ),
            waiting: waiting > 0,
            blocked: blocked > 0,
        });
        self.render_download_notice();
        Ok(())
    }

    pub(super) fn clear_download_notice(&mut self, id: &str) -> Result<()> {
        if self
            .download_notice
            .as_ref()
            .is_some_and(|notice| notice.task_id == id)
        {
            self.download_notice = None;
            self.store.delete_setting(NOTICE_KEY)?;
        }
        self.render_download_notice();
        Ok(())
    }

    pub(super) fn clear_download_notice_for_start(&mut self, id: &str) -> Result<bool> {
        let open_notice = matches!(
            self.dialog_action.as_str(),
            "download-wait" | "download-continue"
        ) && self
            .download_notice
            .as_ref()
            .is_some_and(|notice| notice.task_id == id);
        self.clear_download_notice(id)?;
        Ok(open_notice)
    }

    pub(super) fn reopen_running_download_dialog(&mut self) {
        self.dialog_action = "downloading".into();
        if let Some(ui) = self.ui.upgrade() {
            let shell = ui.global::<ShellModel>();
            shell.set_dialog_kind("downloading".into());
            shell.set_dialog_title(tr("download").into());
            shell.set_dialog_body("".into());
            shell.set_dialog_primary(tr("background_download").into());
            shell.set_dialog_secondary(tr("cancel_download").into());
            shell.set_dialog_destructive(false);
        }
        let blocked = self
            .current_download
            .as_ref()
            .and_then(|task| self.store.download_chapter_progress(&task.id).ok())
            .map_or(0, |progress| progress.blocked_count);
        self.sync_running_download_actions(blocked);
    }

    pub(super) fn sync_running_download_actions(&mut self, blocked: usize) {
        if !matches!(
            self.dialog_action.as_str(),
            "downloading" | "download-retry-active"
        ) {
            return;
        }
        self.dialog_action = if blocked > 0 {
            "download-retry-active"
        } else {
            "downloading"
        }
        .into();
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<ShellModel>().set_dialog_primary(
                tr(if blocked > 0 {
                    "download_retry_chapters"
                } else {
                    "background_download"
                })
                .into(),
            );
            ui.global::<ShellModel>()
                .set_dialog_secondary(tr("cancel_download").into());
        }
    }

    pub(super) fn retry_active_download_chapters(&mut self) -> Result<()> {
        let Some(previous) = self.current_download.as_ref() else {
            return self.continue_download_notice();
        };
        if self.chapter_content_owner().as_deref() != Some(previous.account_key.as_str()) {
            return Ok(());
        }
        let now = Utc::now().timestamp();
        let (progress, ordinals) = self.store.resume_blocked_download_chapters(previous, now)?;
        let task = self.current_download.as_mut().unwrap();
        task.completed_count = progress.completed_count;
        task.next_index = progress.next_index;
        task.updated_at = now;
        let task = task.clone();
        let accepted = ordinals.is_empty()
            || self
                .jobs
                .retry_download_chapters(self.download_request, &task.id, &ordinals);
        self.download_feedback(
            progress.completed_count,
            task.chapters.len(),
            progress.retry_wait_count,
            progress.blocked_count,
        );
        if let Err(error) = self.remember_download_notice(&task) {
            crate::diagnostic!("Refresh active chapter retry notice failed: {error:#}");
        }
        if accepted {
            return Ok(());
        }
        if self.jobs.active_download().is_some() {
            // A cancellation or a different active generation owns this slot.
            // Keep the durable intent; never interrupt it to replay a command.
            return Ok(());
        }
        self.requeue_incomplete_download(&task.id)
    }

    pub(super) fn requeue_incomplete_download(&mut self, task_id: &str) -> Result<()> {
        let Some(task) = self
            .current_download
            .as_ref()
            .filter(|task| task.id == task_id)
            .cloned()
        else {
            return Ok(());
        };
        // The last peer may finish between the click and command. Its terminal
        // packet cannot retire the new durable Pending chapter retry intent.
        let mut pending = task.clone();
        pending.status = DownloadStatus::Pending;
        if self.store.save_task_if_current(&task, &pending)? {
            self.current_download = None;
            self.download_request = self.request();
            self.retire_task_wake_guard();
            self.downloads.retain(|queued| queued.id != pending.id);
            self.downloads.push_front(pending);
            self.start_next_download()?;
        }
        Ok(())
    }

    pub(super) fn reuse_download_notice(
        &mut self,
        book_id: &str,
        requested: &BTreeSet<u32>,
        include_images: bool,
    ) -> Result<bool> {
        let Some(id) = self
            .download_notice
            .as_ref()
            .filter(|notice| notice.book_id == book_id)
            .map(|notice| notice.task_id.clone())
        else {
            return Ok(false);
        };
        let Some(task) = self.store.task(&id)? else {
            return Ok(false);
        };
        if self.chapter_content_owner().as_deref() != Some(task.account_key.as_str())
            || task.include_images != include_images
            || matches!(
                task.status,
                DownloadStatus::Cancelled | DownloadStatus::Complete
            )
        {
            return Ok(false);
        }
        let states = self.store.download_chapter_states(&task.id)?;
        let remaining = if states.is_empty() {
            task.chapters
                .iter()
                .skip(task.next_index)
                .copied()
                .collect::<Vec<_>>()
        } else {
            states
                .iter()
                .filter(|state| state.status != DownloadChapterStatus::Complete)
                .map(|state| state.chapter_index)
                .collect()
        };
        if remaining.is_empty() || !remaining.iter().all(|index| requested.contains(index)) {
            return Ok(false);
        }
        if self.current_download.is_some()
            || states
                .iter()
                .any(|state| state.status == DownloadChapterStatus::RetryWait)
        {
            self.open_download_notice()?;
        } else {
            self.continue_download_notice()?;
        }
        Ok(true)
    }

    pub(super) fn download_notice_visible(&self) -> bool {
        self.current_download.is_some() || self.download_notice.is_some()
    }

    pub(super) fn download_notice_task_for_book(&self, book: &str) -> Result<Option<DownloadTask>> {
        self.download_notice
            .as_ref()
            .filter(|notice| notice.book_id == book)
            .map(|notice| self.store.task(&notice.task_id))
            .transpose()
            .map(Option::flatten)
    }

    pub(super) fn render_download_notice(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        ui.global::<LibraryModel>()
            .set_download_notice(self.download_notice_visible());
        if let Some(task) = &self.current_download {
            let label = ui.global::<ShellModel>().get_task_label();
            ui.global::<LibraryModel>().set_status(label.clone());
            if matches!(&self.route, Route::Detail(id) if *id == task.book_id) {
                ui.global::<DetailModel>().set_status(label);
            }
        } else if let Some(notice) = &self.download_notice {
            ui.global::<ShellModel>()
                .set_task_label(notice.label.clone().into());
            ui.global::<LibraryModel>()
                .set_status(notice.label.clone().into());
            if matches!(&self.route, Route::Detail(id) if *id == notice.book_id) {
                ui.global::<DetailModel>()
                    .set_status(notice.label.clone().into());
            }
        } else {
            ui.global::<ShellModel>().set_task_label("".into());
            ui.global::<LibraryModel>()
                .set_status(self.library_status.clone().into());
            if matches!(&self.route, Route::Detail(_)) {
                ui.global::<DetailModel>()
                    .set_status(self.detail_status.clone().into());
            }
        }
    }

    pub(super) fn open_download_notice(&mut self) -> Result<()> {
        if self.current_download.is_some() {
            self.reopen_running_download_dialog();
            return Ok(());
        }
        let Some(notice) = &self.download_notice else {
            return Ok(());
        };
        self.dialog_action = "download-continue".into();
        if let Some(ui) = self.ui.upgrade() {
            let shell = ui.global::<ShellModel>();
            shell.set_dialog_kind("confirm".into());
            shell.set_dialog_title(
                tr(if notice.blocked {
                    "download_partial"
                } else if notice.waiting {
                    "download_waiting"
                } else {
                    "download_interrupted"
                })
                .into(),
            );
            shell.set_dialog_body(notice.label.clone().into());
            shell.set_dialog_primary(
                tr(if notice.blocked {
                    "download_retry_chapters"
                } else {
                    "continue_download"
                })
                .into(),
            );
            shell.set_dialog_secondary(tr("cancel_download").into());
            shell.set_dialog_destructive(false);
        }
        Ok(())
    }

    pub(super) fn cancel_download_notice(&mut self) -> Result<()> {
        let Some(id) = self
            .download_notice
            .as_ref()
            .map(|notice| notice.task_id.clone())
        else {
            return Ok(());
        };
        let Some(mut task) = self.store.task(&id)? else {
            return self.clear_download_notice(&id);
        };
        if self.chapter_content_owner().as_deref() != Some(task.account_key.as_str()) {
            return self.clear_download_notice(&id);
        }
        let previous = task.clone();
        task.status = DownloadStatus::Cancelled;
        task.error = None;
        task.updated_at = Utc::now().timestamp();
        reset_recovery(&mut task);
        if self.store.save_task_if_current(&previous, &task)? {
            self.downloads.retain(|queued| queued.id != id);
            if self
                .current_download
                .as_ref()
                .is_some_and(|current| current.id == id)
            {
                self.current_download = None;
                self.settle_job_submission(self.jobs.cancel_download(&id));
                self.retire_task_wake_guard();
            }
            self.clear_download_notice(&id)?;
        }
        Ok(())
    }

    pub(super) fn continue_download_notice(&mut self) -> Result<()> {
        if self.current_download.is_some() {
            return self.open_download_notice();
        }
        let Some(id) = self
            .download_notice
            .as_ref()
            .map(|notice| notice.task_id.clone())
        else {
            return Ok(());
        };
        let Some(task) = self.store.task(&id)? else {
            return self.clear_download_notice(&id);
        };
        if self.chapter_content_owner().as_deref() != Some(task.account_key.as_str())
            || matches!(
                task.status,
                DownloadStatus::Cancelled | DownloadStatus::Complete
            )
        {
            return self.clear_download_notice(&id);
        }
        self.queue_explicit_chapter_retry(task)
    }

    pub(super) fn queue_explicit_chapter_retry(&mut self, task: DownloadTask) -> Result<()> {
        let id = task.id.clone();
        if self.chapter_content_owner().as_deref() != Some(task.account_key.as_str())
            || matches!(
                task.status,
                DownloadStatus::Cancelled | DownloadStatus::Complete
            )
        {
            return Ok(());
        }
        let now = Utc::now().timestamp();
        if self.store.download_chapter_states(&task.id)?.is_empty() {
            let headers = self.store.chapter_metadata(&task.book_id)?;
            self.reconcile_download_chapters(&task, &headers, now)?;
        }
        let task = self
            .store
            .task(&id)?
            .ok_or_else(|| anyhow::anyhow!("A download task disappeared"))?;
        // Explicit retry resets blocked chapters only. Completed chapters and
        // future retry deadlines remain durable across repeated button presses.
        self.store.resume_download_chapters(&task, now)?;
        let previous = self
            .store
            .task(&id)?
            .ok_or_else(|| anyhow::anyhow!("A download task disappeared"))?;
        let mut pending = previous.clone();
        pending.status = DownloadStatus::Pending;
        pending.error = None;
        pending.updated_at = now;
        reset_recovery(&mut pending);
        if self.store.save_task_if_current(&previous, &pending)? {
            self.downloads.retain(|queued| queued.id != id);
            self.downloads.push_front(pending);
            self.clear_download_notice(&id)?;
            self.start_next_download()?;
        }
        Ok(())
    }
}
