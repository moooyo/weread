mod article_images;
mod auth;
mod auxiliary;
mod backend;
mod book_images;
mod library;
mod lighting;
mod lighting_worker;
mod managed_lifecycle;
mod managed_sleep;
mod reading;
mod reporting;
mod session_persistence;
mod session_restore;
mod system_settings;
mod task_wake;
mod thoughts;
mod underline_projection;

#[cfg(test)]
mod closing_tests;

#[cfg(test)]
mod terminal_tests;

#[cfg(test)]
mod download_tests;

#[cfg(test)]
mod article_image_tests;

#[cfg(test)]
mod managed_lifecycle_tests;

#[cfg(all(test, target_os = "linux"))]
mod resource_tests;

#[cfg(test)]
mod underline_projection_tests;

#[cfg(test)]
mod thoughts_tests;

#[cfg(test)]
mod cached_metadata_tests;

#[cfg(test)]
mod local_caret_tests;

#[cfg(test)]
mod input_limit_tests;

#[cfg(test)]
mod display_metadata_tests;

#[cfg(test)]
mod mp_projection_tests;

#[cfg(test)]
mod publication_filter_budget_tests;

#[cfg(test)]
mod book_metadata_workset_tests;

#[cfg(test)]
mod selection_workset_tests;

#[cfg(test)]
mod catalog_settlement_tests;

#[cfg(test)]
mod screen_shelf_budget_tests;

#[cfg(test)]
mod chapter_access_consumer_tests;

#[cfg(test)]
mod local_mark_output_tests;

#[cfg(test)]
mod article_projection_budget_tests;

#[cfg(test)]
mod local_statistics_work_tests;

#[cfg(test)]
mod local_note_workset_tests;

#[cfg(test)]
mod bulk_integrity_tests;

#[cfg(test)]
mod request_context_tests;

#[cfg(test)]
mod report_summary_tests;

#[cfg(test)]
mod report_allocation_budget_tests;

#[cfg(test)]
mod report_consumption_budget_tests;

#[cfg(test)]
mod native_edit_tests;

#[cfg(test)]
mod fresh_checkpoint_ack_tests;

#[cfg(test)]
mod incoming_article_prepublication_tests;

#[cfg(test)]
mod article_url_budget_tests;

#[cfg(test)]
mod layout_admission_tests;

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use anyhow::Result;
use chrono::Utc;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::{
    cache::FileCache,
    domain::*,
    i18n::{self, text as tr},
    input::PinyinInput,
    jobs::{JobEvent, Jobs},
    platform::{DeviceProfile, Platform, RefreshKind, RefreshPlan, RefreshReason},
    preferences::{AppPreferences, ReaderPreferences},
    reader::{Paginator, TextMetrics},
    session::SessionFile,
    source::{self, MpAccount, MpArticle, RemoteReview, RemoteUnderline},
    store::Store,
    ui::*,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Login,
    Library,
    Search,
    Detail(String),
    Reader(String, u32),
    Publications,
    Statistics,
    Settings,
    Article(String),
}

impl Route {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Library => "library",
            Self::Search => "search",
            Self::Detail(_) => "detail",
            Self::Reader(..) => "reader",
            Self::Publications => "mp",
            Self::Statistics => "stats",
            Self::Settings => "settings",
            Self::Article(_) => "article",
        }
    }
}

pub struct FontRecord {
    pub id: String,
    pub label: String,
    pub family: String,
    pub metrics: TextMetrics,
    pub weights: BTreeMap<u16, TextMetrics>,
    pub available: bool,
}

impl FontRecord {
    pub fn metrics_for_weight(&self, weight: u32) -> &TextMetrics {
        self.weights
            .iter()
            .min_by_key(|(candidate, _)| i32::from(**candidate).abs_diff(weight as i32))
            .map(|(_, metrics)| metrics)
            .unwrap_or(&self.metrics)
    }
}

pub struct AppOptions {
    pub data_dir: PathBuf,
    pub profile: DeviceProfile,
    pub demo: bool,
    pub offline: bool,
    pub initial_screen: String,
    pub fonts: Vec<FontRecord>,
}

struct ReaderState {
    book_id: String,
    chapter_index: u32,
    body: String,
    paginator: Option<Paginator>,
    preferences: ReaderPreferences,
    notes_panel: bool,
    toc_page: usize,
    toc_tab: String,
    underlines: Vec<RemoteUnderline>,
    underline_body_revision: u64,
    underline_revision: u64,
    underline_projection: underline_projection::UnderlineProjection,
    thought_id: String,
    // Local read-only view ownership is separate from the retained draft.
    thought_edit_blocked: bool,
    thought_page: usize,
    remote_thoughts: Vec<RemoteReview>,
    #[cfg(test)]
    thought_preview_reads: usize,
    #[cfg(test)]
    thought_quote_layouts: usize,
    #[cfg(test)]
    caret_work_limit: usize,
    #[cfg(test)]
    caret_stats: Cell<crate::reader::CaretStats>,
    thought_request: u64,
    thought_context: Option<reading::ThoughtRequestContext>,
    note_edit_context: Option<reading::LocalNoteEditContext>,
    #[cfg(test)]
    selection_work_stats: Cell<reading::SelectionWorkStats>,
    #[cfg(test)]
    mark_output_stats: Cell<reading::MarkOutputStats>,
    underline_request: u64,
    selection: Option<(u32, u32)>,
    press: Option<reading::ReaderPress>,
    suppress_tap: bool,
    position_confirmed: bool,
    cloud: Option<source::ReadingPosition>,
    draft: PinyinInput,
    session_started: Option<i64>,
    session_seconds: u64,
    last_active: Instant,
}

impl ReaderState {
    fn set_context(&mut self, book_id: &str, chapter_index: u32) {
        if self.book_id != book_id || self.chapter_index != chapter_index {
            if self.book_id != book_id {
                self.book_id = book_id.to_owned();
            }
            self.chapter_index = chapter_index;
            self.underline_body_revision = self.underline_body_revision.wrapping_add(1);
            self.underline_projection.clear();
            self.thought_context = None;
            self.note_edit_context = None;
        }
    }

    fn set_chapter_index(&mut self, chapter_index: u32) {
        if self.chapter_index != chapter_index {
            self.chapter_index = chapter_index;
            self.underline_body_revision = self.underline_body_revision.wrapping_add(1);
            self.underline_projection.clear();
            self.thought_context = None;
            self.note_edit_context = None;
        }
    }

    fn replace_body(&mut self, body: String) {
        if self.body != body {
            self.underline_body_revision = self.underline_body_revision.wrapping_add(1);
            self.underline_projection.clear();
            self.thought_context = None;
            self.note_edit_context = None;
        }
        self.body = body;
    }

    fn replace_underlines(&mut self, underlines: Vec<RemoteUnderline>) {
        self.underlines = underlines;
        self.underline_revision = self.underline_revision.wrapping_add(1);
        self.underline_projection.clear();
        self.thought_context = None;
    }
}

struct ArticleState {
    id: String,
    account_id: String,
    body: String,
    html: String,
    page: usize,
    pages: Vec<Vec<crate::article::PageBlock>>,
    page_layout: crate::reader::layout_budget::LayoutLease,
    loaded_images: BTreeMap<String, PathBuf>,
    loaded_layout: crate::reader::layout_budget::LayoutLease,
    image_work: article_images::ArticleImageWork,
    #[cfg(test)]
    document_parses: usize,
    #[cfg(test)]
    image_cache_checks: usize,
}

/// Immutable view rows keep admission until the actual last ModelRc owner
/// releases them, including owners held by Slint repeaters or another view.
struct AdmittedViewModel<T> {
    rows: VecModel<T>,
    _storage: crate::reader::layout_budget::LayoutLease,
    _registration: crate::reader::layout_budget::LayoutModelRegistration,
}

impl<T: Clone + 'static> slint::Model for AdmittedViewModel<T> {
    type Data = T;

    fn row_count(&self) -> usize {
        slint::Model::row_count(&self.rows)
    }
    fn row_data(&self, row: usize) -> Option<T> {
        slint::Model::row_data(&self.rows, row)
    }
    fn set_row_data(&self, _row: usize, _data: T) {}
    fn model_tracker(&self) -> &dyn slint::ModelTracker {
        slint::Model::model_tracker(&self.rows)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[derive(Default)]
struct DownloadUiChanges {
    chapters: BTreeMap<String, ContentStatus>,
    newly_cached: usize,
}

pub struct App {
    ui: slint::Weak<AppWindow>,
    data_dir: PathBuf,
    profile: DeviceProfile,
    platform: Arc<Platform>,
    closing: Rc<Cell<bool>>,
    terminal_cleanup_attempted: bool,
    managed_sleep_blocked: bool,
    managed_sleep: managed_sleep::ManagedSleep,
    resident_lifecycle: managed_lifecycle::ResidentLifecycle,
    lighting: lighting::LightingState,
    store: Store,
    cache: FileCache,
    book_assets: crate::book_assets::BookAssetStore,
    session_file: SessionFile,
    persisted_session: Option<source::AuthSession>,
    jobs: Jobs,
    events: mpsc::Receiver<JobEvent>,
    immediate_job_changed: bool,
    immediate_job_error: Option<anyhow::Error>,
    route: Route,
    history: Vec<Route>,
    prefs: AppPreferences,
    fonts: Vec<FontRecord>,
    model_layout: crate::reader::layout_budget::LayoutModelRegistry,
    demo: bool,
    online: bool,
    force_offline: bool,
    busy: bool,
    next_id: u64,
    shelf_request: u64,
    search_request: u64,
    review_request: u64,
    review_context: Option<library::ReviewRequestContext>,
    stats_request: u64,
    catalog_requests: BTreeMap<String, u64>,
    book_requests: BTreeMap<String, u64>,
    login_request: u64,
    download_request: u64,
    download_retry_at: Instant,
    download_inventory_unavailable: bool,
    device_poll_at: Instant,
    library_page: usize,
    library_sort: String,
    library_status: String,
    sync_time: String,
    selected_books: BTreeSet<String>,
    selecting_books: bool,
    detail_id: String,
    detail_tab: String,
    detail_page: usize,
    detail_status: String,
    selected_chapters: BTreeSet<u32>,
    selecting_chapters: bool,
    reviews: Vec<RemoteReview>,
    review_latest: bool,
    query: PinyinInput,
    search_scope: String,
    search_purpose: String,
    search_page: usize,
    search_results: Vec<BookMetadata>,
    search_status: String,
    login_challenge: Option<source::QrLoginChallenge>,
    login_code: String,
    login_key: String,
    login_polling: bool,
    last_login_poll: Instant,
    reader: ReaderState,
    article: ArticleState,
    mp_accounts: Vec<MpAccount>,
    mp_articles: BTreeMap<String, Vec<MpArticle>>,
    mp_account: Option<String>,
    mp_query: String,
    mp_sort: String,
    mp_page: usize,
    mp_status: String,
    #[cfg(test)]
    publication_stats: Cell<auxiliary::PublicationProjectionStats>,
    stats_period: String,
    stats_offset: i32,
    stats_selected: Option<usize>,
    remote_stats: Option<source::ReadStatistics>,
    settings_subpage: String,
    settings_page: usize,
    settings_status: String,
    downloads: VecDeque<DownloadTask>,
    current_download: Option<DownloadTask>,
    download_ui_changes: BTreeMap<String, DownloadUiChanges>,
    download_ui_refresh_at: Instant,
    task_wake: task_wake::TaskWakeMaintenance,
    pending_downloads: Vec<(String, Vec<u32>)>,
    pending_read: Option<(String, u32)>,
    pending_catalog_downloads: BTreeSet<String>,
    pending_catalog_images: BTreeMap<String, bool>,
    pending_covers: BTreeSet<String>,
    deferred_cover_refill: Option<Route>,
    pending_articles: BTreeSet<String>,
    pending_article_requests: BTreeMap<String, u64>,
    pending_article_images: BTreeSet<String>,
    dialog_action: String,
    jump_target: String,
    toast_until: Option<Instant>,
    last_tick: Instant,
}

pub fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    Rc::new(VecModel::from(items)).into()
}

impl App {
    pub fn attach(
        ui: &AppWindow,
        options: AppOptions,
        platform: Arc<Platform>,
        runtime: &tokio::runtime::Runtime,
    ) -> Result<Rc<RefCell<Self>>> {
        let data_dir = if options.demo {
            options.data_dir.join("demo")
        } else {
            options.data_dir.clone()
        };
        std::fs::create_dir_all(&data_dir)?;
        std::fs::create_dir_all(data_dir.join("imports"))?;
        std::fs::create_dir_all(data_dir.join("fonts"))?;
        let store = Store::open(data_dir.join("library.sqlite"))?;
        if options.demo {
            crate::demo::seed(&store)?;
        }
        let cache = FileCache::open(data_dir.join("covers"), 96 * 1024 * 1024)?;
        let book_assets =
            crate::book_assets::BookAssetStore::open(data_dir.join("images"), 512 * 1024 * 1024)?;
        book_assets.recover(&store)?;
        if options.demo {
            crate::demo::seed_images(&store, &book_assets)?;
        }
        let session_file = SessionFile::new(&data_dir);
        let mut session = if options.demo {
            None
        } else {
            session_file.load()?
        };
        if !options.demo {
            if let Ok(api_key) = std::env::var("WEREAD_API_KEY") {
                if !api_key.trim().is_empty() {
                    let key = api_key.trim().to_owned();
                    session = Some(auth::session_for_api_key(session, &key));
                }
            }
        }
        let configured = session.as_ref().is_some_and(|s| s.is_configured());
        let persisted_session = session.clone().filter(|s| s.is_configured());
        let content_owner = persisted_session
            .as_ref()
            .and_then(library::download_account_key);
        store.invalidate_content_ownership(content_owner.as_deref())?;
        let client = session
            .filter(|s| s.is_configured())
            .map(source::WeReadClient::new)
            .transpose()?;
        let (sender, events) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, client)?;
        let mut prefs = store
            .setting::<AppPreferences>("preferences")?
            .unwrap_or_else(|| AppPreferences::for_device(options.profile.is_scribe()));
        prefs.normalize_for_device(options.profile.is_scribe());
        lighting::restore_dark_mode(&platform, &prefs)?;
        let pending = if let Some(owner) = content_owner.as_deref() {
            store.pending_tasks_for_owner(owner)
        } else {
            Ok(Vec::new())
        };
        let (downloads, download_inventory_unavailable) = match pending {
            Ok(tasks) => (tasks.into_iter().collect(), false),
            Err(error) if Self::inventory_capacity(&error) => (VecDeque::new(), true),
            Err(error) => return Err(error),
        };
        let mp_accounts = store.setting("mp.accounts")?.unwrap_or_default();
        let mp_articles = store.setting("mp.articles")?.unwrap_or_default();
        let sync_time = store.setting("sync.time")?.unwrap_or_default();
        let start = if configured || options.demo || store.shelf_exists()? {
            Route::Library
        } else {
            Route::Login
        };
        let app = Rc::new(RefCell::new(Self {
            ui: ui.as_weak(),
            data_dir,
            profile: options.profile,
            platform,
            closing: Rc::new(Cell::new(false)),
            terminal_cleanup_attempted: false,
            managed_sleep_blocked: false,
            managed_sleep: managed_sleep::ManagedSleep::default(),
            resident_lifecycle: managed_lifecycle::ResidentLifecycle::default(),
            lighting: lighting::LightingState::default(),
            store,
            cache,
            book_assets,
            session_file,
            persisted_session,
            jobs,
            events,
            immediate_job_changed: false,
            immediate_job_error: None,
            route: start,
            history: vec![],
            reader: ReaderState {
                book_id: String::new(),
                chapter_index: 0,
                body: String::new(),
                paginator: None,
                preferences: prefs.reader.clone(),
                notes_panel: false,
                toc_page: 0,
                toc_tab: "toc".into(),
                underlines: vec![],
                underline_body_revision: 0,
                underline_revision: 0,
                underline_projection: underline_projection::UnderlineProjection::default(),
                thought_id: String::new(),
                thought_edit_blocked: false,
                thought_page: 0,
                remote_thoughts: vec![],
                #[cfg(test)]
                thought_preview_reads: 0,
                #[cfg(test)]
                thought_quote_layouts: 0,
                #[cfg(test)]
                caret_work_limit: crate::reader::DEFAULT_CARET_WORK,
                #[cfg(test)]
                caret_stats: Cell::new(crate::reader::CaretStats::default()),
                thought_request: 0,
                thought_context: None,
                note_edit_context: None,
                #[cfg(test)]
                selection_work_stats: Cell::new(reading::SelectionWorkStats::default()),
                #[cfg(test)]
                mark_output_stats: Cell::new(reading::MarkOutputStats::default()),
                underline_request: 0,
                selection: None,
                press: None,
                suppress_tap: false,
                position_confirmed: false,
                cloud: None,
                draft: PinyinInput::empty(crate::input::InputPurpose::Thought),
                session_started: None,
                session_seconds: 0,
                last_active: Instant::now(),
            },
            article: ArticleState {
                id: String::new(),
                account_id: String::new(),
                body: String::new(),
                html: String::new(),
                page: 0,
                pages: Vec::new(),
                page_layout: crate::reader::LayoutBudget::default().lease(),
                loaded_images: BTreeMap::new(),
                loaded_layout: crate::reader::LayoutBudget::default().lease(),
                image_work: article_images::ArticleImageWork::default(),
                #[cfg(test)]
                document_parses: 0,
                #[cfg(test)]
                image_cache_checks: 0,
            },
            prefs,
            fonts: options.fonts,
            model_layout: crate::reader::layout_budget::LayoutModelRegistry::default(),
            demo: options.demo,
            online: !options.offline,
            force_offline: options.offline,
            busy: false,
            next_id: 0,
            shelf_request: 0,
            search_request: 0,
            review_request: 0,
            review_context: None,
            stats_request: 0,
            catalog_requests: BTreeMap::new(),
            book_requests: BTreeMap::new(),
            login_request: 0,
            download_request: 0,
            download_retry_at: Instant::now() + Duration::from_secs(60),
            download_inventory_unavailable,
            device_poll_at: Instant::now() + Duration::from_secs(60),
            library_page: 0,
            library_sort: "default".into(),
            library_status: String::new(),
            sync_time,
            selected_books: BTreeSet::new(),
            selecting_books: false,
            detail_id: String::new(),
            detail_tab: "toc".into(),
            detail_page: 0,
            detail_status: String::new(),
            selected_chapters: BTreeSet::new(),
            selecting_chapters: false,
            reviews: vec![],
            review_latest: false,
            query: PinyinInput::new(""),
            search_scope: "store".into(),
            search_purpose: "books".into(),
            search_page: 0,
            search_results: vec![],
            search_status: String::new(),
            login_challenge: None,
            login_code: String::new(),
            login_key: String::new(),
            login_polling: false,
            last_login_poll: Instant::now() - Duration::from_secs(3),
            mp_accounts,
            mp_articles,
            mp_account: None,
            mp_query: String::new(),
            mp_sort: "recent".into(),
            mp_page: 0,
            mp_status: String::new(),
            #[cfg(test)]
            publication_stats: Cell::new(auxiliary::PublicationProjectionStats::default()),
            stats_period: "week".into(),
            stats_offset: 0,
            stats_selected: None,
            remote_stats: None,
            settings_subpage: String::new(),
            settings_page: 0,
            settings_status: String::new(),
            downloads,
            current_download: None,
            download_ui_changes: BTreeMap::new(),
            download_ui_refresh_at: Instant::now(),
            task_wake: task_wake::TaskWakeMaintenance::default(),
            pending_downloads: vec![],
            pending_read: None,
            pending_catalog_downloads: BTreeSet::new(),
            pending_catalog_images: BTreeMap::new(),
            pending_covers: BTreeSet::new(),
            deferred_cover_refill: None,
            pending_articles: BTreeSet::new(),
            pending_article_requests: BTreeMap::new(),
            pending_article_images: BTreeSet::new(),
            dialog_action: String::new(),
            jump_target: String::new(),
            toast_until: None,
            last_tick: Instant::now(),
        }));
        Self::connect(ui, &app);
        {
            let mut state = app.borrow_mut();
            if state.download_inventory_unavailable {
                state.show_toast("api_error");
            }
            state.recover_read_reports()?;
            if options.initial_screen == "native-return" {
                state.restore_system_return_context()?;
            } else if options.initial_screen == "resume" {
                state.restore_reader_session()?;
            } else if !options.initial_screen.is_empty() {
                state.prepare_screen(&options.initial_screen)?;
            }
            state.render()?;
            state.query_reader_light();
            if configured && !state.force_offline {
                state.sync_shelf();
            }
            if state.route == Route::Login && !state.force_offline {
                state.begin_login();
            }
            state.start_next_download()?;
        }
        Ok(app)
    }

    fn connect(ui: &AppWindow, state: &Rc<RefCell<Self>>) {
        let weak = Rc::downgrade(state);
        ui.window()
            .on_text_input_edit_admission(move |edit: &slint::TextInputEdit<'_>| {
                let Some(state) = weak.upgrade() else {
                    return false;
                };
                let Ok(state) = state.try_borrow() else {
                    return false;
                };
                state.native_text_input_admitted(edit)
            });
        let (platform, closing) = {
            let state = state.borrow();
            (state.platform.clone(), state.closing.clone())
        };
        ui.global::<Theme>().on_refresh(move |kind| {
            if closing.get() {
                return;
            }
            let result = if kind.as_str() == "press" {
                platform.request_refresh(RefreshReason::Press).map(|_| ())
            } else {
                platform.submit_refresh(RefreshPlan {
                    kind: RefreshKind::Ui,
                    rect: None,
                })
            };
            if let Err(error) = result {
                crate::diagnostic!("Display feedback failed: {error}");
            }
        });
        macro_rules! callback {
            ($global:ty, $section:literal) => {{
                let weak = Rc::downgrade(state);
                ui.global::<$global>().on_event(move |action, value| {
                    if let Some(state) = weak.upgrade() {
                        if let Err(error) =
                            state
                                .borrow_mut()
                                .dispatch($section, action.as_str(), value.as_str())
                        {
                            crate::diagnostic!("Application action failed: {error:#}");
                        }
                    }
                });
            }};
        }
        callback!(ShellModel, "shell");
        callback!(LibraryModel, "library");
        callback!(DetailModel, "detail");
        callback!(LoginModel, "login");
        callback!(SearchModel, "search");
        callback!(ReaderModel, "reader");
        callback!(ArticleModel, "article");
        callback!(AuxiliaryModel, "auxiliary");
    }

    /// Admission failures are synchronous controls, never an unbounded mailbox.
    /// Use the existing handler so request/pending bookkeeping stays identical.
    pub(super) fn settle_job_submission(&mut self, rejection: Option<JobEvent>) -> bool {
        let Some(event) = rejection else {
            return false;
        };
        match self.handle_job(event) {
            Ok(changed) => self.immediate_job_changed |= changed,
            Err(error) => {
                if self.immediate_job_error.is_none() {
                    crate::diagnostic!("Settle source request admission failed: {error:#}");
                    self.immediate_job_error = Some(error);
                }
            }
        }
        true
    }

    /// Propagate the first synchronous handler failure before acknowledging a
    /// lifecycle transition. Saving can itself submit and reject another job.
    pub(super) fn preserve_submission_error(&mut self, outcome: Result<()>) -> Result<()> {
        match (self.immediate_job_error.take(), outcome) {
            (Some(error), Err(later)) => Err(error.context(format!(
                "The lifecycle state operation also failed: {later:#}"
            ))),
            (Some(error), Ok(())) => Err(error),
            (None, outcome) => outcome,
        }
    }

    /// Call after fencing all old work and before starting replacement requests.
    /// Drop actual payloads/checkpoint senders; never return credits while their
    /// buffers are still retained by a stale queue item.
    pub(super) fn discard_cancelled_job_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            drop(event);
        }
    }

    pub fn dispatch(&mut self, section: &str, action: &str, value: &str) -> Result<()> {
        if self.closing.get() || self.managed_sleep_blocked {
            return Ok(());
        }
        if self.immediate_job_error.is_some() {
            return Ok(());
        }
        if section == "search"
            && self.search_purpose == "thought"
            && self.reader.thought_edit_blocked
            && self.reader.thought_id.starts_with("mine:")
        {
            // An old Thought editor cannot replace the preserved draft before
            // the Reader's save guard. The read-only view owns current focus.
            if let Some(ui) = self.ui.upgrade() {
                let search = ui.global::<SearchModel>();
                search.set_query("".into());
                search.set_composition("".into());
                search.set_paste_ready(false);
            }
            self.show_toast("api_error");
            return Ok(());
        }
        match section {
            "shell" => self.shell_action(action, value)?,
            "library" => self.library_action(action, value)?,
            "detail" => self.detail_action(action, value)?,
            "login" => self.login_action(action, value)?,
            "search" => self.search_action(action, value)?,
            "reader" => self.reader_action(action, value)?,
            "article" => self.article_action(action, value)?,
            "auxiliary" => self.auxiliary_action(action, value)?,
            _ => {}
        }
        if self.closing.get() {
            return Ok(());
        }
        if section == "reader" && matches!(action, "reader.move" | "move") {
            return Ok(());
        }
        self.render()
    }

    pub fn tick(&mut self) -> Result<()> {
        if self.closing.get() || self.managed_sleep_blocked {
            return Ok(());
        }
        if let Some(error) = self.immediate_job_error.take() {
            return Err(error);
        }
        let mut changed = self.poll_lighting();
        for _ in 0..64 {
            match self.events.try_recv() {
                Ok(event) => {
                    changed |= self.handle_job(event)?;
                    if self.immediate_job_error.is_some() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        if let Some(error) = self.immediate_job_error.take() {
            return Err(error);
        }
        changed |= std::mem::replace(&mut self.immediate_job_changed, false);
        self.persist_session_updates()?;
        self.poll_task_wake_guard();
        if self
            .toast_until
            .is_some_and(|until| Instant::now() >= until)
        {
            if let Some(ui) = self.ui.upgrade() {
                ui.global::<ShellModel>().set_toast("".into());
            }
            self.toast_until = None;
        }
        if self.last_tick.elapsed() >= Duration::from_secs(1) {
            self.last_tick = Instant::now();
            if self.reader_is_active_page()
                && self.reader.session_started.is_some()
                && self.reader.last_active.elapsed() < Duration::from_secs(300)
            {
                self.reader.session_seconds += 1;
            }
            self.poll_login();
            self.tick_read_reports()?;
            self.retry_pending_downloads()?;
            if Instant::now() >= self.device_poll_at {
                self.device_poll_at = Instant::now() + Duration::from_secs(60);
                self.query_reader_light();
            }
            self.render_clock();
        }
        let pending_article_images = self.pending_article_images.len();
        let loaded_article_images = self.continue_article_image_work()?;
        if loaded_article_images {
            self.reflow_article()?;
        }
        changed |=
            loaded_article_images || self.pending_article_images.len() != pending_article_images;
        if changed {
            self.render()?;
        } else {
            self.flush_download_ui(Instant::now())?;
            self.refill_deferred_visible_covers();
        }
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<()> {
        let saved = (|| {
            self.join_task_wake_guard(task_wake::STOP_TIMEOUT)?;
            self.persist_shutdown_state()?;
            // Save the Reader before waiting for device I/O, then require a joined
            // lighting worker before acknowledging sleep or native ownership.
            self.stop_lighting_hardware()?;
            self.platform.quiesce_device_commands(Duration::ZERO)?;
            self.persist_session_updates()
        })();
        self.preserve_submission_error(saved)?;
        self.complete_reader_shutdown()
    }

    fn persist_shutdown_state(&mut self) -> Result<()> {
        let saved = (|| {
            self.save_reader_session()?;
            self.finish_reading()?;
            self.save_preferences()
        })();
        self.preserve_submission_error(saved)
    }

    /// Main's final exit cannot leave resource cleanup conditional on storage.
    /// Recoverable UI close attempts continue to use `request_exit` instead.
    pub fn terminal_shutdown(&mut self) -> Result<()> {
        let saved = self
            .persist_shutdown_state()
            .and_then(|_| self.persist_session_updates());
        let saved = self.preserve_submission_error(saved);
        let resources = self.cleanup_terminal_resources(task_wake::STOP_TIMEOUT);
        match (saved, resources) {
            (Ok(()), Ok(())) => self.complete_reader_shutdown(),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Err(error), Err(cleanup)) => Err(error.context(format!(
                "Terminal resource cleanup also failed: {cleanup:#}"
            ))),
        }
    }

    /// A complete App may unwind before Main reaches its event-loop cleanup.
    /// Never save, acknowledge ownership, or start a producer from this path.
    fn cleanup_terminal_resources(&mut self, timeout: Duration) -> Result<()> {
        let started = Instant::now();
        self.terminal_cleanup_attempted = true;
        self.closing.set(true);
        self.managed_sleep_blocked = true;
        // Notify every hardware producer before spending a shared join budget.
        self.task_wake.prepare_terminal_cleanup();
        self.request_lighting_stop();
        self.jobs.cancel_all();
        // Closing the original receiver drops buffered image packets and their
        // checkpoint senders, including work already queued before the fence.
        let (sender, events) = mpsc::channel();
        drop(sender);
        self.events = events;

        let mut failure = None;
        retain_terminal_cleanup_error(
            &mut failure,
            self.join_task_wake_guard(timeout.saturating_sub(started.elapsed())),
            "wake maintenance",
        );
        retain_terminal_cleanup_error(
            &mut failure,
            self.stop_lighting_hardware_with_timeout(timeout.saturating_sub(started.elapsed())),
            "lighting and battery",
        );
        retain_terminal_cleanup_error(
            &mut failure,
            self.platform
                .quiesce_device_commands(Duration::ZERO)
                .map_err(Into::into),
            "native command ownership",
        );
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Save state and request an explicit user exit through one lifecycle path.
    pub fn request_exit(&mut self) -> Result<()> {
        if let Err(error) = self.shutdown().and_then(|_| self.mark_explicit_exit()) {
            self.resume_reader_after_failed_exit();
            return Err(error);
        }
        // Slint delivers the release feedback after this click requests exit.
        self.closing.set(true);
        if let Err(error) = slint::quit_event_loop() {
            self.closing.set(false);
            self.clear_explicit_exit();
            self.resume_reader_after_failed_exit();
            return Err(error.into());
        }
        Ok(())
    }

    pub(super) fn resume_reader_after_failed_exit(&mut self) {
        self.start_task_wake_guard();
        let Route::Reader(book, _) = &self.route else {
            return;
        };
        if self.reader.session_started.is_some()
            || self.reader.paginator.is_none()
            || self.reader.body.is_empty()
            || &self.reader.book_id != book
        {
            return;
        }
        // A completed run is immutable. Continuing the same visible Reader
        // after an interruption starts a distinct run without loading its page.
        self.reader.session_started = Some(Utc::now().timestamp());
        self.reader.session_seconds = 0;
        self.reader.last_active = Instant::now();
        if let Err(error) = self.start_read_report_session() {
            self.note_read_report_error(&error);
        }
    }

    fn request(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
    fn show_toast(&mut self, key: &str) {
        self.show_message(tr(key));
    }

    fn show_message(&mut self, message: String) {
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<ShellModel>().set_toast(message.into());
            self.toast_until = Some(Instant::now() + Duration::from_secs(2));
        }
    }
    fn save_preferences(&self) -> Result<()> {
        self.store.set_setting("preferences", &self.prefs)
    }

    fn refresh_display(&self, reason: RefreshReason) {
        if self.closing.get() {
            return;
        }
        if let Err(error) = self.platform.request_refresh(reason) {
            crate::diagnostic!("Display update failed: {error}");
        }
    }

    pub(super) fn invalidate_account_work(&mut self) -> Result<()> {
        self.clear_article_image_work();
        self.interrupt_read_reports()?;
        if let Some(mut task) = self.current_download.take() {
            self.settle_job_submission(self.jobs.cancel_download(&task.id));
            task.status = DownloadStatus::Paused;
            task.updated_at = Utc::now().timestamp();
            self.store.save_task(&task)?;
        }
        self.downloads.clear();
        self.download_ui_changes.clear();
        self.retire_task_wake_guard();
        self.pending_covers.clear();
        self.deferred_cover_refill = None;
        self.pending_articles.clear();
        self.pending_article_requests.clear();
        self.pending_article_images.clear();
        self.pending_catalog_downloads.clear();
        self.pending_catalog_images.clear();
        self.pending_downloads.clear();
        self.pending_read = None;
        self.catalog_requests.clear();
        self.book_requests.clear();
        self.resident_lifecycle.clear_retry_work();
        self.search_results.clear();
        self.reviews.clear();
        self.review_context = None;
        self.mp_accounts.clear();
        self.mp_articles.clear();
        self.mp_account = None;
        self.mp_status.clear();
        self.remote_stats = None;
        self.busy = false;
        self.library_status.clear();
        self.history.clear();
        self.reader.selection = None;
        self.reader.draft.clear();
        self.reader.remote_thoughts.clear();
        self.reader.replace_underlines(Vec::new());
        self.reader.thought_id.clear();
        self.reader.thought_context = None;
        self.reader.note_edit_context = None;
        self.reader.thought_edit_blocked = false;
        self.reader.cloud = None;
        self.reader.position_confirmed = false;
        self.dialog_action.clear();
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<ShellModel>().set_task_label("".into());
            ui.global::<ShellModel>().set_dialog_kind("".into());
            ui.global::<ReaderModel>().set_selection_open(false);
            ui.global::<ReaderModel>().set_writing_thought(false);
            ui.global::<ReaderModel>().set_thought_open(false);
        }
        Ok(())
    }

    fn navigate(&mut self, route: Route, remember: bool) -> Result<()> {
        self.navigate_checked(route, remember).map(|_| ())
    }

    fn navigate_checked(&mut self, route: Route, remember: bool) -> Result<bool> {
        if self.route == route {
            return Ok(true);
        }
        let prepared_article = if let Route::Article(id) = &route {
            match self.prepare_article_load(id) {
                Ok(prepared) => Some(prepared),
                Err(error)
                    if Self::view_capacity(&error)
                        || error
                            .downcast_ref::<crate::article::StyleLimitError>()
                            .is_some()
                        || error
                            .downcast_ref::<crate::article::ImageLimitError>()
                            .is_some()
                        || error
                            .downcast_ref::<crate::article::ArticleParseError>()
                            .is_some()
                        || error
                            .downcast_ref::<auxiliary::ArticleProjectionLimit>()
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
            }
        } else {
            None
        };
        let prepared_reader = if let Route::Reader(id, index) = &route {
            if id.is_empty() {
                None
            } else {
                match self.prepare_reader_load(
                    id,
                    *index,
                    None,
                    false,
                    matches!(self.route, Route::Reader(..)),
                    false,
                ) {
                    Ok(prepared) => Some(prepared),
                    Err(error) if Self::view_capacity(&error) => {
                        self.show_toast("api_error");
                        return Ok(false);
                    }
                    Err(error) => return Err(error),
                }
            }
        } else {
            None
        };
        // Leaving a view retires its request identity before another route can
        // reopen a window under the same book/chapter and numeric request.
        if matches!(self.route, Route::Detail(_)) {
            self.review_context = None;
        }
        if matches!(self.route, Route::Reader(..)) {
            self.reader.thought_context = None;
        }
        if !matches!(route, Route::Reader(..))
            && !(route == Route::Search && self.search_purpose == "thought")
        {
            self.reader.note_edit_context = None;
        }
        if matches!(self.route, Route::Reader(..)) {
            self.finish_reading()?;
        }
        if matches!(self.route, Route::Article(_)) {
            self.clear_article_image_work();
        }
        if remember {
            if self.history.len() >= 64 {
                self.history.remove(0);
            }
            self.history.push(self.route.clone());
        }
        self.deferred_cover_refill = None;
        self.route = route.clone();
        match route {
            Route::Detail(id) if id.is_empty() => {
                // An empty shelf used to resolve through an empty full-Book
                // lookup. Preserve that screen without passing a missing ID
                // to the stricter metadata API.
                self.detail_id.clear();
                self.detail_tab = "toc".into();
                self.detail_status.clear();
                self.selecting_chapters = false;
                self.selected_chapters.clear();
                self.reviews.clear();
                self.review_context = None;
                self.review_latest = false;
                self.detail_page = 0;
            }
            Route::Reader(id, _) if id.is_empty() => {
                self.route = Route::Library;
                self.reader.note_edit_context = None;
                self.show_toast("empty_library");
            }
            Route::Detail(id) => self.load_detail(&id)?,
            Route::Reader(_, _) => {
                self.install_reader_load(prepared_reader.ok_or_else(|| {
                    anyhow::anyhow!("The target Reader preparation is unavailable")
                })?)?
            }
            Route::Article(_) => {
                self.install_article_load(prepared_article.ok_or_else(|| {
                    anyhow::anyhow!("The target Article preparation is unavailable")
                })?)?
            }
            Route::Statistics => self.request_statistics()?,
            _ => {}
        }
        self.refresh_display(RefreshReason::EnterScreen);
        Ok(true)
    }

    fn back(&mut self) -> Result<()> {
        let route = self.history.last().cloned().unwrap_or(Route::Library);
        if self.navigate_checked(route, false)? {
            self.history.pop();
        }
        Ok(())
    }

    fn retained_layout_metadata(&self) -> Result<usize> {
        let reader = self
            .reader
            .paginator
            .as_ref()
            .map_or(0, Paginator::requested_metadata);
        reader
            .checked_add(self.article.page_layout.bytes())
            .and_then(|bytes| bytes.checked_add(self.article.loaded_layout.bytes()))
            .and_then(|bytes| bytes.checked_add(self.article.image_work.requested_metadata()))
            .and_then(|bytes| bytes.checked_add(self.model_layout.live_metadata()))
            .ok_or_else(|| {
                crate::reader::LayoutLimitError {
                    kind: crate::reader::LayoutLimitKind::Metadata,
                    used: reader,
                    requested: usize::MAX,
                    limit: crate::reader::layout_budget::DEFAULT_LAYOUT_METADATA_BYTES,
                }
                .into()
            })
    }

    fn admitted_shared_string(
        &self,
        text: &str,
        budget: &crate::reader::LayoutBudget,
    ) -> Result<(
        slint::SharedString,
        crate::reader::layout_budget::LayoutLease,
    )> {
        let (copy, mut owner) = budget.copy(text)?;
        let shared_bytes = text
            .len()
            .checked_add(1)
            .ok_or(crate::reader::LayoutLimitError {
                kind: crate::reader::LayoutLimitKind::Metadata,
                used: 0,
                requested: usize::MAX,
                limit: crate::reader::layout_budget::DEFAULT_LAYOUT_METADATA_BYTES,
            })?;
        let shared_owner = budget.copy_storage(shared_bytes)?;
        let shared = slint::SharedString::from(copy.as_str());
        drop(copy);
        owner.release(text.len());
        owner.merge(shared_owner);
        Ok((shared, owner))
    }

    fn preflight_view_fields<'a, T>(
        &self,
        budget: &crate::reader::LayoutBudget,
        rows: usize,
        fields: impl IntoIterator<Item = &'a str>,
        additional: usize,
    ) -> Result<()> {
        let mut bytes = rows
            .checked_mul(std::mem::size_of::<T>())
            .and_then(|bytes| {
                bytes.checked_add(
                    std::mem::size_of::<AdmittedViewModel<T>>() + 2 * std::mem::size_of::<usize>(),
                )
            })
            .and_then(|bytes| bytes.checked_add(additional))
            .ok_or(crate::reader::LayoutLimitError {
                kind: crate::reader::LayoutLimitKind::Metadata,
                used: 0,
                requested: usize::MAX,
                limit: crate::reader::layout_budget::DEFAULT_LAYOUT_METADATA_BYTES,
            })?;
        for field in fields {
            bytes = field
                .len()
                .checked_mul(2)
                .and_then(|text| text.checked_add(1))
                .and_then(|text| bytes.checked_add(text))
                .ok_or(crate::reader::LayoutLimitError {
                    kind: crate::reader::LayoutLimitKind::Metadata,
                    used: 0,
                    requested: usize::MAX,
                    limit: crate::reader::layout_budget::DEFAULT_LAYOUT_METADATA_BYTES,
                })?;
        }
        budget.preflight(bytes, bytes)?;
        Ok(())
    }

    fn admitted_view_model<T: Clone + 'static>(
        &self,
        items: crate::reader::layout_budget::LayoutVec<T>,
        budget: &crate::reader::LayoutBudget,
    ) -> Result<ModelRc<T>> {
        let (items, mut owner) = items.into_parts();
        owner.merge(budget.copy_storage(
            std::mem::size_of::<AdmittedViewModel<T>>() + 2 * std::mem::size_of::<usize>(),
        )?);
        let registration = self.model_layout.register(owner.bytes())?;
        Ok(Rc::new(AdmittedViewModel {
            rows: VecModel::from(items),
            _storage: owner,
            _registration: registration,
        })
        .into())
    }

    fn view_capacity(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| {
            cause
                .downcast_ref::<crate::reader::LayoutLimitError>()
                .is_some()
                || cause
                    .downcast_ref::<crate::article::StyleLimitError>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::article::ImageLimitError>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::article::ArticleParseError>()
                    .is_some()
                || cause
                    .downcast_ref::<auxiliary::ArticleProjectionLimit>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::store::BookMetadataLimit>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::store::BookMetadataUnavailable>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::store::CachedResourceLimit>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::store::AnnotationResourceLimit>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::store::AnnotationUnavailable>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::store::BookmarkResourceLimit>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::store::BookmarkUnavailable>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::store::StoreInventoryLimit>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::store::StoreInventoryUnavailable>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::store::StoredSettingReadLimit>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::store::StoredSettingUnavailable>()
                    .is_some()
                || cause
                    .downcast_ref::<source::ReadingPositionLimitError>()
                    .is_some()
                || cause
                    .downcast_ref::<source::ReadingPositionSettingError>()
                    .is_some_and(|error| error.is_capacity())
                || cause
                    .downcast_ref::<crate::cache::InventoryLimitExceeded>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::cache::InventoryUnavailable>()
                    .is_some()
                || matches!(
                    cause.downcast_ref::<source::SourceError>(),
                    Some(source::SourceError::ResponseTooLarge)
                )
        })
    }

    fn inventory_capacity(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| {
            cause
                .downcast_ref::<crate::store::StoreInventoryLimit>()
                .is_some()
                || cause
                    .downcast_ref::<crate::store::StoreInventoryUnavailable>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::cache::InventoryLimitExceeded>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::cache::InventoryUnavailable>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::cache::VerificationLimitExceeded>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::cache::VerificationMetadataLimitExceeded>()
                    .is_some()
                || cause
                    .downcast_ref::<crate::book_assets::BulkVerificationLimitExceeded>()
                    .is_some()
        })
    }

    fn render(&mut self) -> Result<()> {
        if !matches!(
            self.route,
            Route::Library | Route::Search | Route::Detail(_)
        ) {
            self.deferred_cover_refill = None;
        }
        let Some(ui) = self.ui.upgrade() else {
            return Ok(());
        };
        ui.global::<Theme>().set_scribe(self.profile.is_scribe());
        let shell = ui.global::<ShellModel>();
        shell.set_route(self.route.name().into());
        shell.set_online(self.online);
        shell.set_demo(self.demo);
        shell.set_selection_count(self.selected_books.len() as i32);
        shell.set_can_download(
            self.jobs.capabilities().chapter_content && self.online && !self.demo,
        );
        shell.set_active_tab(
            match self.route {
                Route::Publications | Route::Article(_) => "mp",
                Route::Statistics => "stats",
                Route::Settings => "settings",
                _ => "library",
            }
            .into(),
        );
        self.render_clock();
        self.render_lighting();
        match self.route {
            Route::Library => self.render_library()?,
            Route::Search => self.render_search()?,
            Route::Login => self.render_login(),
            Route::Detail(_) => self.render_detail()?,
            Route::Reader(..) => self.render_reader()?,
            Route::Publications => self.render_publications()?,
            Route::Statistics => self.render_statistics()?,
            Route::Settings => self.render_settings()?,
            Route::Article(_) => self.render_article()?,
        }
        self.download_ui_changes.clear();
        self.download_ui_refresh_at = Instant::now() + Duration::from_secs(1);
        Ok(())
    }

    fn render_clock(&self) {
        if let Some(ui) = self.ui.upgrade() {
            let clock = Utc::now()
                .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
                .format("%H:%M")
                .to_string();
            ui.global::<ShellModel>().set_clock(clock.clone().into());
            ui.global::<ReaderModel>().set_clock(clock.clone().into());
            ui.global::<ArticleModel>().set_clock(clock.into());
        }
    }

    fn open_detail(&mut self, id: &str) -> Result<()> {
        self.navigate(Route::Detail(id.into()), true)
    }
    fn open_reader(&mut self, id: &str, index: u32) -> Result<()> {
        self.navigate(Route::Reader(id.into(), index), true)
    }
    fn requested_screen_route(&self, screen: &str) -> Result<Route> {
        Ok(match screen {
            "login" => Route::Login,
            "library" => Route::Library,
            "search" => Route::Search,
            "mp" => Route::Publications,
            "stats" => Route::Statistics,
            "settings" => Route::Settings,
            "article" => {
                let first = self
                    .mp_articles
                    .values()
                    .flatten()
                    .next()
                    .map(|article| article.id.clone())
                    .unwrap_or_default();
                Route::Article(first)
            }
            "detail" => Route::Detail(self.store.first_shelf_book_id()?.unwrap_or_default()),
            "reader" => Route::Reader(self.store.first_shelf_book_id()?.unwrap_or_default(), 0),
            _ => Route::Library,
        })
    }

    fn prepare_screen(&mut self, screen: &str) -> Result<()> {
        let route = self.requested_screen_route(screen)?;
        self.navigate(route, false)
    }

    fn error_message(&mut self, error: &source::SourceError) {
        if matches!(error, source::SourceError::Busy) {
            self.show_toast("operation_failed");
        } else if matches!(error, source::SourceError::UnlockRequired) {
            self.show_toast("content_unlock_required");
        } else if matches!(error, source::SourceError::ContentUnverified) {
            self.show_toast("content_verify_required");
        } else if matches!(error, source::SourceError::ContentChanged) {
            self.show_toast("content_changed");
        } else if error.requires_login() {
            self.show_toast("session_expired");
        } else if matches!(
            error,
            source::SourceError::Network | source::SourceError::Timeout
        ) {
            self.online = false;
            self.show_toast("network_unavailable");
        } else {
            self.show_toast("api_error");
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Main already spent the cleanup budget when terminal_shutdown ran.
        // Destruction may retry completed handles, but never add another wait.
        let timeout = if self.terminal_cleanup_attempted {
            Duration::ZERO
        } else {
            task_wake::STOP_TIMEOUT
        };
        if let Err(error) = self.cleanup_terminal_resources(timeout) {
            crate::diagnostic!("Release terminal application resources failed: {error:#}");
        }
    }
}

fn retain_terminal_cleanup_error(
    failure: &mut Option<anyhow::Error>,
    result: Result<()>,
    resource: &'static str,
) {
    if let Err(error) = result {
        *failure = Some(match failure.take() {
            Some(previous) => {
                previous.context(format!("Terminal {resource} cleanup failed: {error:#}"))
            }
            None => error.context(format!("Terminal {resource} cleanup failed")),
        });
    }
}

fn page_count(length: usize, size: usize) -> usize {
    length.div_ceil(size.max(1)).max(1)
}
fn move_page(page: usize, delta: i32, count: usize) -> usize {
    (page as i64 + delta as i64).clamp(0, count.saturating_sub(1) as i64) as usize
}
fn unique_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        Utc::now().timestamp_micros(),
        std::process::id()
    )
}
