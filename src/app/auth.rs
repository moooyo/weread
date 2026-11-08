use super::*;
use crate::domain::BookMetadata;
use crate::input::{InputIntent, InputPurpose};

#[derive(Clone, Copy)]
pub(super) struct JumpEditLimits {
    pub(super) scalars: usize,
    pub(super) bytes: usize,
    pub(super) work_bytes: usize,
}

// Page numbers need at most twenty decimal digits on a 64-bit target. Keep
// room for zero padding and correction without changing native paste syntax.
// These independent edit limits bound clipboard, composition and Undo input.
pub(super) const JUMP_EDIT_LIMITS: JumpEditLimits = JumpEditLimits {
    scalars: 256,
    bytes: 512,
    work_bytes: 1024,
};

fn borrowed_text_edit_parts<'a>(edit: &'a slint::TextInputEdit<'_>) -> Option<[&'a str; 4]> {
    let current = edit.current_text();
    let replacement = edit.replacement_range();
    if replacement.start > replacement.end {
        return None;
    }
    Some([
        current.get(..replacement.start)?,
        current.get(replacement.end..)?,
        edit.inserted_text(),
        edit.preedit_text(),
    ])
}

fn text_parts_within_scalar_limit(parts: [&str; 4], maximum: usize) -> bool {
    let mut remaining = maximum;
    for part in parts {
        let scalars = part.chars().take(remaining.saturating_add(1)).count();
        if scalars > remaining {
            return false;
        }
        remaining -= scalars;
    }
    true
}

pub(super) fn jump_text_edit_admitted(
    edit: &slint::TextInputEdit<'_>,
    limits: JumpEditLimits,
) -> bool {
    let Some(parts) = borrowed_text_edit_parts(edit) else {
        return false;
    };
    // Charge the entire previous text, including a removed selection, before
    // scanning scalars. A replacement cannot hide unbounded removal/Undo work.
    let work = [
        edit.current_text(),
        edit.inserted_text(),
        edit.preedit_text(),
    ]
    .into_iter()
    .try_fold(0usize, |total, part| total.checked_add(part.len()));
    let bytes = parts
        .into_iter()
        .try_fold(0usize, |total, part| total.checked_add(part.len()));
    if work.is_none_or(|work| work > limits.work_bytes)
        || bytes.is_none_or(|bytes| bytes > limits.bytes)
    {
        return false;
    }
    text_parts_within_scalar_limit(parts, limits.scalars)
}

impl App {
    /// Read only the current application scope and the native borrowed edit.
    /// Native admission precedes undo/text/preedit mutation; never dispatch,
    /// render, read the clipboard again, or materialize a candidate here.
    pub(super) fn native_text_input_admitted(&self, edit: &slint::TextInputEdit<'_>) -> bool {
        if self.closing.get()
            || self.managed_sleep_blocked
            || self.immediate_job_error.is_some()
            || !edit.is_enabled()
            || edit.is_read_only()
        {
            return false;
        }
        // Jump edits have their own capacity policy and retain Number's
        // existing key/paste syntax. An obsolete dialog cannot edit a new scope.
        if edit.is_numeric() {
            return self.dialog_action == "jump" && jump_text_edit_admitted(edit, JUMP_EDIT_LIMITS);
        }
        let maximum = if edit.is_password() {
            if self.route != Route::Login {
                return false;
            }
            16_384
        } else if edit.is_single_line() {
            if !matches!(self.route, Route::Search | Route::Publications)
                || self.route == Route::Search && self.search_purpose == "thought"
            {
                return false;
            }
            InputPurpose::Search.maximum_scalars()
        } else {
            if !matches!(self.route, Route::Reader(..))
                && !(self.route == Route::Search && self.search_purpose == "thought")
            {
                return false;
            }
            if self.reader.thought_edit_blocked && self.reader.thought_id.starts_with("mine:") {
                return false;
            }
            InputPurpose::Thought.maximum_scalars()
        };
        let Some(parts) = borrowed_text_edit_parts(edit) else {
            return false;
        };
        text_parts_within_scalar_limit(parts, maximum)
    }

    pub(super) fn login_action(&mut self, action: &str, value: &str) -> Result<()> {
        match action {
            "close" => {
                self.jobs.cancel_login();
                // Fence results already queued before the login flow was cancelled.
                self.login_request = self.request();
                self.login_polling = false;
                self.login_challenge = None;
                self.login_code.clear();
                self.login_key.clear();
                if let Err(error) = self.request_exit() {
                    if let Some(ui) = self.ui.upgrade() {
                        let login = ui.global::<LoginModel>();
                        login.set_busy(false);
                        login.set_qr(slint::Image::default());
                        login.set_qr_label("".into());
                        login.set_status(tr("operation_failed").into());
                    }
                    self.render_login();
                    return Err(error);
                }
            }
            "refresh" => {
                if !self.force_offline && !self.demo {
                    self.online = true;
                }
                self.begin_login();
            }
            "step" if value == "key" => {
                self.jobs.cancel_login();
                self.login_request = self.request();
                self.login_polling = false;
                self.login_challenge = None;
                self.login_code.clear();
                if let Some(ui) = self.ui.upgrade() {
                    let login = ui.global::<LoginModel>();
                    login.set_step("key".into());
                    login.set_busy(false);
                    login.set_status("".into());
                }
            }
            "step" => self.begin_login(),
            "code-key" => {
                let busy = self
                    .ui
                    .upgrade()
                    .is_some_and(|ui| ui.global::<LoginModel>().get_busy());
                if busy || self.login_polling {
                    return Ok(());
                }
                match value {
                    "rescan" => {
                        if !self.force_offline && !self.demo {
                            self.online = true;
                        }
                        self.begin_login();
                    }
                    "backspace" => {
                        self.login_code.pop();
                    }
                    digit if digit.len() == 1 && digit.as_bytes()[0].is_ascii_digit() => {
                        if self.login_code.len() < 4 {
                            self.login_code.push_str(digit);
                        }
                    }
                    _ => {}
                }
            }
            "submit-code" => {
                if self.login_polling || self.login_code.len() != 4 {
                    return Ok(());
                }
                let Some(challenge) = self.login_challenge.clone() else {
                    self.show_toast("login_expired");
                    return Ok(());
                };
                if challenge.expires_at <= Utc::now().timestamp() {
                    self.poll_login();
                    return Ok(());
                }
                self.login_polling = true;
                self.last_login_poll = Instant::now();
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<LoginModel>().set_busy(true);
                }
                self.settle_job_submission(self.jobs.login_poll(
                    self.login_request,
                    challenge.uid,
                    Some(self.login_code.clone()),
                ));
            }
            "edit-key" => {
                self.login_key = value.chars().take(16_384).collect();
            }
            "api-key-key" => {
                if self
                    .ui
                    .upgrade()
                    .is_some_and(|ui| ui.global::<LoginModel>().get_busy())
                {
                    return Ok(());
                }
                match value {
                    "submit" => return self.login_action("submit-key", ""),
                    "backspace" => {
                        self.login_key.pop();
                    }
                    "space" if self.login_key.len() < 16_384 => self.login_key.push(' '),
                    "period" if self.login_key.len() < 16_384 => self.login_key.push('.'),
                    character
                        if character.chars().count() == 1 && self.login_key.len() < 16_384 =>
                    {
                        self.login_key.push_str(character);
                    }
                    _ => {}
                }
            }
            "submit-key" => {
                let key = self.login_key.trim();
                if key.is_empty() {
                    return Ok(());
                }
                if self.demo {
                    self.show_toast("demo_notice");
                    return Ok(());
                }
                let session = session_for_api_key(self.jobs.session(), key);
                let has_web_session = session.has_web_session();
                self.invalidate_account_work()?;
                self.session_file.save(&session)?;
                self.jobs.replace_session(session)?;
                self.discard_cancelled_job_events();
                self.synchronize_content_owner()?;
                self.login_request = self.request();
                self.login_polling = false;
                self.login_challenge = None;
                self.login_code.clear();
                self.login_key.clear();
                if let Some(ui) = self.ui.upgrade() {
                    let login = ui.global::<LoginModel>();
                    login.set_api_key("".into());
                    login.set_busy(false);
                    login.set_status("".into());
                }
                self.history.clear();
                self.navigate(Route::Library, false)?;
                if has_web_session {
                    self.show_toast("key_saved");
                } else {
                    self.show_message(format!(
                        "{} · {}",
                        tr("key_saved"),
                        tr("web_login_required")
                    ));
                }
                self.sync_shelf();
            }
            "local" => {
                self.jobs.cancel_login();
                self.login_request = self.request();
                self.login_polling = false;
                self.login_challenge = None;
                self.login_key.clear();
                self.history.clear();
                self.navigate(Route::Library, false)?;
            }
            "demo" => {
                self.jobs.cancel_login();
                self.login_request = self.request();
                self.login_polling = false;
                self.login_challenge = None;
                self.login_key.clear();
                self.enter_demo()?;
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn begin_login(&mut self) {
        self.jobs.cancel_login();
        self.login_request = self.request();
        self.login_challenge = None;
        self.login_code.clear();
        self.login_key.clear();
        self.login_polling = false;
        self.last_login_poll = Instant::now() - Duration::from_secs(2);
        let unavailable = self.demo || self.force_offline || !self.online;
        if let Some(ui) = self.ui.upgrade() {
            let login = ui.global::<LoginModel>();
            login.set_step("qr".into());
            login.set_qr(slint::Image::default());
            login.set_code("".into());
            login.set_code_length(0);
            login.set_digits(model(vec!["".into(); 4]));
            login.set_api_key("".into());
            login.set_can_submit(false);
            login.set_busy(!unavailable);
            login.set_status(
                tr(if unavailable {
                    "network_unavailable"
                } else {
                    "login_connecting"
                })
                .into(),
            );
            login.set_qr_label("".into());
        }
        if !unavailable {
            self.settle_job_submission(self.jobs.login_begin(self.login_request));
        }
    }

    pub(super) fn poll_login(&mut self) {
        if self.route != Route::Login {
            return;
        }
        let Some(challenge) = self.login_challenge.clone() else {
            return;
        };
        if challenge.expires_at <= Utc::now().timestamp() {
            self.jobs.cancel_login();
            self.login_request = self.request();
            self.login_challenge = None;
            self.login_polling = false;
            if let Some(ui) = self.ui.upgrade() {
                let login = ui.global::<LoginModel>();
                login.set_step("qr".into());
                login.set_busy(false);
                login.set_status(tr("login_expired").into());
                login.set_qr_label(tr("login_expired").into());
            }
            return;
        }
        self.render_login();
        let step = self
            .ui
            .upgrade()
            .map(|ui| ui.global::<LoginModel>().get_step().to_string())
            .unwrap_or_default();
        if step != "qr"
            || self.force_offline
            || !self.online
            || self.login_polling
            || self.last_login_poll.elapsed() < Duration::from_secs(1)
        {
            return;
        }
        self.login_polling = true;
        self.last_login_poll = Instant::now();
        self.settle_job_submission(
            self.jobs
                .login_poll(self.login_request, challenge.uid, None),
        );
    }

    pub(super) fn render_login(&mut self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let login = ui.global::<LoginModel>();
        login.set_code(self.login_code.clone().into());
        login.set_code_length(self.login_code.chars().count() as i32);
        login.set_digits(model(
            (0..4)
                .map(|index| {
                    self.login_code
                        .chars()
                        .nth(index)
                        .map(|digit| digit.to_string())
                        .unwrap_or_default()
                        .into()
                })
                .collect(),
        ));
        login.set_api_key(self.login_key.clone().into());
        login.set_can_submit(self.login_code.len() == 4);
        if let Some(challenge) = &self.login_challenge {
            let remaining = (challenge.expires_at - Utc::now().timestamp()).max(0);
            let time = format!("{:02}:{:02}", remaining / 60, remaining % 60);
            login.set_qr_label(i18n::format("qr_expiry", &[("time", time)]).into());
        }
    }

    pub(super) fn search_action(&mut self, action: &str, value: &str) -> Result<()> {
        let purpose = self.search_input_purpose();
        if action == "prepare-paste" {
            if let Some(ui) = self.ui.upgrade() {
                ui.global::<SearchModel>().set_paste_ready(false);
            }
        }
        if !matches!(action, "back" | "clear" | "edit-query") && !self.admit_search_input() {
            return Ok(());
        }
        match action {
            "back" => {
                self.search_request = self.request();
                if self.search_purpose == "thought" && self.query.is_admitted() {
                    self.reader.draft = self.query.clone();
                }
                self.back()?;
            }
            "prepare-paste" => {
                if !self.query.commit_first() {
                    self.show_toast("operation_failed");
                    return Ok(());
                }
                self.search_request = self.request();
                self.search_page = 0;
                self.update_search_preview()?;
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<SearchModel>().set_paste_ready(true);
                }
            }
            "focus" => {
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<SearchModel>().set_keyboard_visible(true);
                }
            }
            "clear" => {
                self.query.clear();
                self.query.set_purpose(purpose);
                self.search_request = self.request();
                self.search_page = 0;
                self.search_results.clear();
                self.search_status.clear();
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<SearchModel>().set_keyboard_visible(true);
                }
            }
            "edit-query" => {
                if !self.query.set_query_for_purpose(value, purpose) {
                    self.show_toast("operation_failed");
                    return Ok(());
                }
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<SearchModel>().set_keyboard_visible(true);
                }
                self.search_request = self.request();
                self.search_page = 0;
                self.update_search_preview()?;
            }
            "scope" if matches!(value, "store" | "shelf") => {
                self.search_scope = value.to_owned();
                self.search_page = 0;
                self.search_request = self.request();
                self.run_search(false)?;
            }
            "history" => {
                if !self.query.set_query(value) {
                    self.show_toast("operation_failed");
                    return Ok(());
                }
                self.run_search(true)?;
            }
            "clear-history" => self.store.clear_queries()?,
            "candidate" => {
                if self.query.choose(value) {
                    self.search_request = self.request();
                    self.search_page = 0;
                    self.update_search_preview()?;
                } else {
                    self.show_toast("operation_failed");
                }
            }
            "key" => {
                let previous = self.query.query_revision();
                let intent = self.query.press(value);
                if intent == InputIntent::Rejected {
                    self.show_toast("operation_failed");
                    return Ok(());
                }
                if intent == InputIntent::Submit {
                    self.run_search(true)?;
                } else if previous != self.query.query_revision() {
                    self.search_request = self.request();
                    self.search_page = 0;
                    self.update_search_preview()?;
                }
            }
            "open" if self.search_purpose == "books" => {
                self.search_request = self.request();
                self.open_detail(value)?;
            }
            "add" if self.search_purpose == "books" => {
                // Both local results and genuine remote results already have
                // a complete stored row. Never write a display projection back.
                if self.store.book_on_shelf(value)?.is_some() {
                    self.store
                        .set_setting(&format!("shelf.override:{value}"), &true)?;
                    self.store.set_on_shelf(value, true)?;
                    if let Some(result) = self
                        .search_results
                        .iter_mut()
                        .find(|result| result.id == value)
                    {
                        result.on_shelf = true;
                    }
                    self.show_toast("local_saved");
                }
            }
            "page" => {
                let keyboard = self
                    .ui
                    .upgrade()
                    .is_none_or(|ui| ui.global::<SearchModel>().get_keyboard_visible());
                let count = page_count(
                    self.search_results.len(),
                    search_page_size(self.profile, self.search_viewport_size(), keyboard),
                );
                self.search_page = move_page(self.search_page, value.parse().unwrap_or(0), count);
                self.refresh_display(RefreshReason::Content);
            }
            "jump" => self.open_jump("search", self.search_page + 1),
            _ => {}
        }
        Ok(())
    }

    fn update_search_preview(&mut self) -> Result<()> {
        if self.search_purpose != "books" || self.query.query.trim().is_empty() {
            self.search_results.clear();
            self.search_status.clear();
            return Ok(());
        }
        if self.search_scope == "shelf" || self.demo {
            match local_search_preview(&self.store, &self.query, self.search_scope == "shelf") {
                Ok((results, status)) => {
                    self.search_results = results;
                    self.search_status = status;
                }
                Err(error)
                    if error
                        .downcast_ref::<crate::store::BookMetadataLimit>()
                        .is_some()
                        || error
                            .downcast_ref::<crate::store::BookMetadataUnavailable>()
                            .is_some() =>
                {
                    self.search_status = tr("api_error");
                    self.show_toast("api_error");
                }
                Err(error) => return Err(error),
            }
        } else {
            self.search_results.clear();
            self.search_status.clear();
        }
        Ok(())
    }

    fn search_input_purpose(&self) -> InputPurpose {
        if self.search_purpose == "thought" {
            InputPurpose::Thought
        } else {
            InputPurpose::Search
        }
    }

    fn admit_search_input(&mut self) -> bool {
        let purpose = self.search_input_purpose();
        if self.query.set_purpose(purpose) {
            true
        } else {
            self.show_toast("operation_failed");
            false
        }
    }

    fn run_search(&mut self, submitted: bool) -> Result<()> {
        if !self.admit_search_input() {
            return Ok(());
        }
        if submitted {
            if !self.query.commit_first() {
                self.show_toast("operation_failed");
                return Ok(());
            }
            if let Some(ui) = self.ui.upgrade() {
                ui.global::<SearchModel>().set_keyboard_visible(false);
            }
        }
        self.search_request = self.request();
        self.search_page = 0;
        if self.search_purpose == "publications" {
            if submitted {
                self.mp_query = self.query.query.trim().to_owned();
                self.mp_page = 0;
                self.back()?;
            }
            return Ok(());
        }
        if self.search_purpose == "thought" {
            if submitted {
                self.reader.draft = self.query.clone();
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<ReaderModel>()
                        .set_thought_draft(self.query.query.clone().into());
                }
                self.back()?;
                self.reader_action("save-thought", "")?;
            }
            return Ok(());
        }
        let query = self.query.query.trim().to_owned();
        if query.is_empty() {
            self.search_results.clear();
            self.search_status.clear();
            return Ok(());
        }
        if submitted {
            self.store.remember_query(&query)?;
        }
        if self.search_scope == "shelf" || self.demo {
            return self.update_search_preview();
        }
        self.search_results.clear();
        if self.force_offline || !self.online {
            self.search_status = tr("network_unavailable");
            self.show_toast("network_unavailable");
            return Ok(());
        }
        if !self.jobs.configured() {
            self.search_status = tr("credentials_missing");
            self.show_toast("credentials_missing");
            return Ok(());
        }
        if !self.jobs.capabilities().search {
            self.search_status = tr("api_key_required");
            self.show_toast("api_key_required");
            return Ok(());
        }
        self.search_status = tr("searching");
        self.settle_job_submission(self.jobs.search(self.search_request, query));
        Ok(())
    }

    pub(super) fn render_search(&mut self) -> Result<()> {
        if !self.admit_search_input() {
            if let Some(ui) = self.ui.upgrade() {
                // An old rejected checkpoint remains in SQL. Its oversized
                // editable value must not survive in a two-way Slint binding.
                let search = ui.global::<SearchModel>();
                search.set_query("".into());
                search.set_composition("".into());
                search.set_query_width(0.0);
                search.set_composition_width(0.0);
                search.set_candidates(model(Vec::new()));
            }
            return Ok(());
        }
        let Some(ui) = self.ui.upgrade() else {
            return Ok(());
        };
        let search = ui.global::<SearchModel>();
        let metrics = self
            .fonts
            .iter()
            .find(|font| font.id == "hei" && font.available)
            .map(|font| font.metrics.clone())
            .unwrap_or_default();
        if self.search_purpose == "books" && self.search_scope == "shelf" {
            self.update_search_preview()?;
        }
        let size = search_page_size(
            self.profile,
            self.search_viewport_size(),
            search.get_keyboard_visible(),
        );
        let count = page_count(self.search_results.len(), size);
        self.search_page = self.search_page.min(count - 1);
        let mut results = Vec::new();
        if self.search_purpose == "books" {
            for book in self
                .search_results
                .iter()
                .skip(self.search_page * size)
                .take(size)
            {
                let cover = self
                    .cache
                    .get(&format!("cover:{}", book.id))?
                    .and_then(|path| slint::Image::load_from_path(&path).ok())
                    .unwrap_or_default();
                let on_shelf = self.store.book_on_shelf(&book.id)?.unwrap_or(false);
                let tone = book.title.bytes().fold(0u32, |hash, byte| {
                    hash.wrapping_mul(31).wrapping_add(byte as u32)
                }) % 4;
                results.push(SearchResult {
                    id: book.id.clone().into(),
                    title: book.title.clone().into(),
                    author: book.author.clone().into(),
                    category: book.category.clone().into(),
                    initial: book
                        .title
                        .chars()
                        .next()
                        .map(|value| value.to_string())
                        .unwrap_or_default()
                        .into(),
                    on_shelf,
                    cover,
                    tone: tone as i32,
                });
            }
        }
        search.set_query(self.query.query.clone().into());
        search.set_composition(self.query.composition.clone().into());
        search.set_query_width(metrics.measure(&self.query.query, 18.0));
        search.set_composition_width(metrics.measure(&self.query.composition, 18.0));
        search.set_candidates(model(
            self.query
                .candidates()
                .into_iter()
                .map(Into::into)
                .collect(),
        ));
        let recent = if self.search_purpose == "books" {
            self.store.recent_queries(4)?
        } else {
            vec![]
        };
        let (chips, rows) = layout_recent_queries(
            &recent,
            self.profile,
            self.platform.logical_viewport_size(),
            &metrics,
        );
        search.set_recent(model(recent.into_iter().map(Into::into).collect()));
        search.set_recent_chips(model(chips));
        search.set_recent_rows(rows);
        search.set_results(model(results));
        search.set_scope(self.search_scope.clone().into());
        search.set_purpose(self.search_purpose.clone().into());
        search.set_language(self.query.language.as_str().into());
        search.set_caps(self.query.caps);
        search.set_page(self.search_page as i32 + 1);
        search.set_page_count(count as i32);
        search.set_status(
            if self.search_status.is_empty() && !self.search_results.is_empty() {
                i18n::format(
                    "search_result_count",
                    &[("count", self.search_results.len().to_string())],
                )
            } else {
                self.search_status.clone()
            }
            .into(),
        );
        self.schedule_visible_covers();
        Ok(())
    }
}

fn local_search_preview(
    store: &Store,
    input: &PinyinInput,
    shelf_only: bool,
) -> Result<(Vec<BookMetadata>, String)> {
    if input.query.trim().is_empty() {
        return Ok((vec![], String::new()));
    }
    let results = store.search_book_metadata(&input.query, shelf_only)?;
    let status = i18n::format(
        "search_result_count",
        &[("count", results.len().to_string())],
    );
    Ok((results, status))
}

fn layout_recent_queries(
    queries: &[String],
    profile: DeviceProfile,
    viewport: (u32, u32),
    metrics: &TextMetrics,
) -> (Vec<RecentSearchChip>, i32) {
    let available = viewport.0.saturating_sub(profile.page_margin() * 2) as f32;
    let (font_size, padding, gap) = if profile.is_scribe() {
        (18.0, 40.0, 12.0)
    } else {
        (16.0, 32.0, 10.0)
    };
    let mut x = 0.0;
    let mut row = 0;
    let mut chips = Vec::new();
    for query in queries {
        let width = (metrics.measure(query, font_size).ceil() + padding)
            .max(48.0)
            .min(available);
        if x > 0.0 && x + width > available {
            x = 0.0;
            row += 1;
        }
        chips.push(RecentSearchChip {
            text: query.clone().into(),
            x,
            width,
            row,
        });
        x += width + gap;
    }
    let rows = if chips.is_empty() { 0 } else { row + 1 };
    (chips, rows)
}

fn search_page_size(profile: DeviceProfile, viewport: (u32, u32), keyboard: bool) -> usize {
    let maximum = if profile.is_scribe() {
        if keyboard { 7 } else { 11 }
    } else if keyboard {
        5
    } else {
        9
    };
    let reserved = 28 + 60 + 48 + 41 + if keyboard { 300 } else { 48 };
    let available = (viewport.1 as usize).saturating_sub(reserved);
    (available / 72).max(1).min(maximum)
}

pub(super) fn session_for_api_key(
    previous: Option<source::AuthSession>,
    key: &str,
) -> source::AuthSession {
    let mut session = previous
        .filter(|session| session.api_key == key)
        .unwrap_or_default();
    session.api_key = key.to_owned();
    session
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composing_pinyin_keeps_recent_queries_without_an_empty_result_count() -> Result<()> {
        let store = Store::open(":memory:")?;
        store.remember_query("Previous title")?;
        store.upsert_book(&Book {
            id: "matching-book".into(),
            title: "A committed search".into(),
            author: "Fixture author".into(),
            on_shelf: true,
            ..Default::default()
        })?;
        let recent = store.recent_queries(4)?;
        let mut input = PinyinInput::default();
        for character in "shi".chars() {
            input.press(&character.to_string());
            let (books, status) = local_search_preview(&store, &input, true)?;
            assert!(books.is_empty());
            assert!(status.is_empty());
            assert!(input.query.is_empty());
            assert_eq!(store.recent_queries(4)?, recent);
        }
        let candidate = input.candidates().into_iter().next().unwrap();
        store.upsert_book(&Book {
            id: "candidate-book".into(),
            title: candidate.clone(),
            on_shelf: true,
            ..Default::default()
        })?;
        assert!(input.choose(&candidate));
        let (books, status) = local_search_preview(&store, &input, true)?;
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].id, "candidate-book");
        assert_eq!(
            status,
            i18n::format("search_result_count", &[("count", "1".into())])
        );
        assert_eq!(store.recent_queries(4)?, recent);
        Ok(())
    }

    #[test]
    fn a_composition_suffix_does_not_replace_the_committed_search() -> Result<()> {
        let store = Store::open(":memory:")?;
        store.upsert_book(&Book {
            id: "matching-book".into(),
            title: "Committed title".into(),
            on_shelf: true,
            ..Default::default()
        })?;
        let mut input = PinyinInput::new("Committed");
        let before = local_search_preview(&store, &input, true)?;
        input.press("s");
        input.press("h");
        input.press("i");
        let during = local_search_preview(&store, &input, true)?;
        assert_eq!(during.0.len(), before.0.len());
        assert_eq!(during.0[0].id, before.0[0].id);
        assert_eq!(during.1, before.1);
        assert_eq!(input.query, "Committed");
        assert_eq!(input.composition, "shi");
        Ok(())
    }

    #[test]
    fn recent_query_capsules_use_the_bundled_ui_font_and_wrap_within_the_page() {
        use std::io::Read;

        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../assets/fonts/NotoSansSC-Regular.otf.gz")[..],
        )
        .read_to_end(&mut bytes)
        .unwrap();
        let metrics = TextMetrics::from_font(bytes, 0).unwrap();
        assert!(metrics.is_font_backed());
        let queries = vec![
            "iiiiii".into(),
            "WWWWWW".into(),
            "A very long query ".repeat(12),
            "Last query".into(),
        ];
        for profile in [DeviceProfile::Paperwhite5, DeviceProfile::Scribe] {
            let (chips, rows) =
                layout_recent_queries(&queries, profile, profile.logical_size(), &metrics);
            assert_eq!(chips.len(), queries.len());
            assert!(chips[0].width < chips[1].width);
            assert!(rows > 1);
            let available = (profile.logical_size().0 - profile.page_margin() * 2) as f32;
            for chip in &chips {
                assert!(chip.x >= 0.0 && chip.width >= 48.0);
                assert!(chip.x + chip.width <= available);
            }
            assert!(chips.windows(2).all(|pair| {
                pair[1].row > pair[0].row || pair[1].x >= pair[0].x + pair[0].width + 8.0
            }));
        }
    }

    #[test]
    fn search_layout_uses_runtime_viewport_without_changing_the_design_profile() {
        let profile = DeviceProfile::Paperwhite6;
        let viewport = (636, 848);
        let queries = vec!["A long query ".repeat(100)];
        let metrics = TextMetrics::default();
        let (design_chips, _) =
            layout_recent_queries(&queries, profile, profile.logical_size(), &metrics);
        let (viewport_chips, _) = layout_recent_queries(&queries, profile, viewport, &metrics);

        assert_eq!(profile.logical_size(), (632, 840));
        assert_eq!(design_chips[0].width, 592.0);
        assert_eq!(viewport_chips[0].width, 596.0);
        assert!(
            search_page_size(profile, (636, 768), true) < search_page_size(profile, viewport, true)
        );
    }

    #[test]
    fn a_different_api_key_never_inherits_a_web_account() {
        let previous = source::AuthSession {
            api_key: "fixture-account-a-key".into(),
            cookies: BTreeMap::from([("wr_skey".into(), "fixture-account-a-cookie".into())]),
            user_vid: "fixture-account-a".into(),
            account_name: "Fixture account A".into(),
            wr_ticket: "fixture-ticket".into(),
            ..Default::default()
        };
        let same = session_for_api_key(Some(previous.clone()), "fixture-account-a-key");
        assert_eq!(same.cookies, previous.cookies);
        assert_eq!(same.user_vid, previous.user_vid);
        let replacement = session_for_api_key(Some(previous), "fixture-account-b-key");
        assert_eq!(replacement.api_key, "fixture-account-b-key");
        assert!(replacement.cookies.is_empty());
        assert!(replacement.user_vid.is_empty());
        assert!(replacement.account_name.is_empty());
        assert!(replacement.wr_ticket.is_empty());
    }

    #[test]
    fn search_pages_fit_the_reserved_controls_on_every_profile() {
        for profile in [
            DeviceProfile::Paperwhite5,
            DeviceProfile::Paperwhite6,
            DeviceProfile::Scribe,
            DeviceProfile::Scribe2025,
        ] {
            for keyboard in [false, true] {
                let reserved = 28 + 60 + 48 + 41 + if keyboard { 300 } else { 48 };
                assert!(
                    search_page_size(profile, profile.logical_size(), keyboard) * 72 + reserved
                        <= profile.logical_size().1 as usize
                );
            }
        }
        assert!(
            search_page_size(
                DeviceProfile::Paperwhite5,
                DeviceProfile::Paperwhite5.logical_size(),
                true,
            ) < search_page_size(
                DeviceProfile::Paperwhite6,
                DeviceProfile::Paperwhite6.logical_size(),
                true,
            )
        );
    }
}
