//! A managed, exclusive-display handoff to the stock Kindle settings UI.

use super::session_restore::{LauncherContext, reader_account_key};
use super::*;
use crate::domain::BookMetadata;
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};

// Serialization is confined to the historical return-context field. The
// application/domain metadata type itself remains non-serializable and cannot
// be passed to Store::upsert_book. No current settings capability is added.
mod search_metadata_snapshot {
    use super::*;
    #[derive(Default, Serialize, Deserialize)]
    #[serde(remote = "BookMetadata", default)]
    struct Snapshot {
        id: String,
        title: String,
        author: String,
        category: String,
        cover_url: String,
        source: String,
        progress: f32,
        total_chapters: u32,
        cached_chapters: u32,
        updated: bool,
        finished: bool,
        on_shelf: bool,
    }
    struct Ref<'a>(&'a BookMetadata);
    impl Serialize for Ref<'_> {
        fn serialize<S: serde::Serializer>(
            &self,
            serializer: S,
        ) -> std::result::Result<S::Ok, S::Error> {
            Snapshot::serialize(self.0, serializer)
        }
    }
    #[derive(Deserialize)]
    struct Owned(#[serde(with = "Snapshot")] BookMetadata);
    pub fn serialize<S: serde::Serializer>(
        books: &[BookMetadata],
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_seq(books.iter().map(Ref))
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Vec<BookMetadata>, D::Error> {
        Vec::<Owned>::deserialize(deserializer)
            .map(|books| books.into_iter().map(|book| book.0).collect())
    }
}

const RETURN_CONTEXT_KEY: &str = "native.return.v1";

#[derive(Clone, Copy)]
enum SettingsKind {
    Wifi,
    Quick,
}

impl SettingsKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "wifi" => Some(Self::Wifi),
            "quick" => Some(Self::Quick),
            _ => None,
        }
    }

    fn capability(self) -> &'static str {
        match self {
            Self::Wifi => "native-wifi-available",
            Self::Quick => "native-quick-available",
        }
    }

    fn request(self) -> &'static str {
        match self {
            Self::Wifi => "native-wifi-request",
            Self::Quick => "native-quick-request",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "screen", rename_all = "snake_case", deny_unknown_fields)]
enum ReturnScreen {
    Login,
    Library,
    Search,
    Detail { book_id: String },
    Reader { book_id: String, chapter_index: u32 },
    Publications,
    Statistics,
    Settings,
    Article { article_id: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedReturnContext {
    version: u32,
    account_key: String,
    screen: ReturnScreen,
    history: Vec<ReturnScreen>,
    library_page: usize,
    library_sort: String,
    #[serde(default)]
    detail_id: String,
    detail_tab: String,
    detail_page: usize,
    detail_reviews: Vec<RemoteReview>,
    detail_review_latest: bool,
    settings_subpage: String,
    settings_page: usize,
    publication_account: Option<String>,
    publication_query: String,
    publication_sort: String,
    publication_page: usize,
    statistics_period: String,
    statistics_offset: i32,
    statistics_selected: Option<usize>,
    search_query: String,
    search_scope: String,
    search_purpose: String,
    search_page: usize,
    #[serde(with = "search_metadata_snapshot")]
    search_results: Vec<BookMetadata>,
    search_status: String,
    #[serde(default = "default_search_keyboard_visible")]
    search_keyboard_visible: bool,
    #[serde(default)]
    search_pending: Option<bool>,
    article_page: usize,
}

impl ReturnScreen {
    fn from_route(route: &Route) -> Self {
        match route {
            Route::Login => Self::Login,
            Route::Library => Self::Library,
            Route::Search => Self::Search,
            Route::Detail(book_id) => Self::Detail {
                book_id: book_id.clone(),
            },
            Route::Reader(book_id, chapter_index) => Self::Reader {
                book_id: book_id.clone(),
                chapter_index: *chapter_index,
            },
            Route::Publications => Self::Publications,
            Route::Statistics => Self::Statistics,
            Route::Settings => Self::Settings,
            Route::Article(article_id) => Self::Article {
                article_id: article_id.clone(),
            },
        }
    }

    fn route(&self) -> Result<Route> {
        let valid_id =
            |id: &str| !id.is_empty() && id.len() <= 2048 && !id.chars().any(char::is_control);
        Ok(match self {
            Self::Login => Route::Login,
            Self::Library => Route::Library,
            Self::Search => Route::Search,
            Self::Detail { book_id } => {
                ensure!(valid_id(book_id), "The saved detail identifier is invalid");
                Route::Detail(book_id.clone())
            }
            Self::Reader {
                book_id,
                chapter_index,
            } => {
                ensure!(valid_id(book_id), "The saved Reader identifier is invalid");
                Route::Reader(book_id.clone(), *chapter_index)
            }
            Self::Publications => Route::Publications,
            Self::Statistics => Route::Statistics,
            Self::Settings => Route::Settings,
            Self::Article { article_id } => {
                ensure!(
                    valid_id(article_id),
                    "The saved article identifier is invalid"
                );
                Route::Article(article_id.clone())
            }
        })
    }
}

impl SavedReturnContext {
    fn resolve(&self, account_key: &str) -> Result<(Route, Vec<Route>)> {
        ensure!(
            self.version == 1,
            "The native return checkpoint version is unsupported"
        );
        ensure!(
            self.account_key == account_key,
            "The native return checkpoint belongs to a different account"
        );
        ensure!(
            self.history.len() <= 64,
            "The native return history is too large"
        );
        let route = self.screen.route()?;
        let history = self
            .history
            .iter()
            .map(ReturnScreen::route)
            .collect::<Result<Vec<_>>>()?;
        Ok((route, history))
    }

    fn detail_snapshot_id(
        &self,
        store: &Store,
        route: &Route,
        history: &[Route],
    ) -> Result<Option<String>> {
        let id = if self.detail_id.is_empty() {
            // An older checkpoint can identify the currently visible detail
            // through its route, but cannot bind a Reader's hidden detail state.
            match route {
                Route::Detail(id) => id.as_str(),
                _ => return Ok(None),
            }
        } else {
            self.detail_id.as_str()
        };
        ReturnScreen::Detail {
            book_id: id.to_owned(),
        }
        .route()?;
        if matches!(route, Route::Reader(book, _) | Route::Detail(book) if book.as_str() != id) {
            return Ok(None);
        }
        let referenced = matches!(route, Route::Reader(book, _) | Route::Detail(book) if book.as_str() == id)
            || history
                .iter()
                .any(|entry| matches!(entry, Route::Detail(book) if book.as_str() == id));
        if !referenced || store.book(id)?.is_none() {
            return Ok(None);
        }
        Ok(Some(id.to_owned()))
    }

    fn restart_search(&self) -> bool {
        matches!(self.screen, ReturnScreen::Search)
            && self.search_purpose == "books"
            && self.search_scope == "store"
            && !self.search_query.trim().is_empty()
            && self.search_pending.unwrap_or_else(|| {
                // Older checkpoints have no explicit request state. Retrying
                // an empty remote result is safe and also handles a translated
                // pending status that the new process cannot identify.
                self.search_results.is_empty() || self.search_status == tr("searching")
            })
    }
}

fn default_search_keyboard_visible() -> bool {
    true
}

impl App {
    pub(super) fn system_settings_available(&self) -> bool {
        self.native_settings_available(SettingsKind::Quick)
    }

    pub(super) fn wifi_settings_available(&self) -> bool {
        self.native_settings_available(SettingsKind::Wifi)
    }

    fn native_settings_available(&self, kind: SettingsKind) -> bool {
        LauncherContext::from_environment()
            .and_then(|context| {
                context.map_or(Ok(false), |context| context.matches(kind.capability()))
            })
            .unwrap_or(false)
    }

    /// The stock UI is opened only by the supervisor after this process exits.
    /// A valid checkpoint acknowledgment is required before it releases ownership.
    pub(super) fn request_system_settings(&mut self, value: &str) -> Result<()> {
        let Some(kind) = SettingsKind::parse(value) else {
            return Ok(());
        };
        if !self.native_settings_available(kind) {
            self.show_toast("native_settings_unavailable");
            return Ok(());
        }
        if self.native_settings_editing_active() {
            // Restarting the process cannot safely retain an unfinished note,
            // selection or secret input. Keep the editor open for save/cancel.
            self.show_toast("native_settings_finish_editing");
            return Ok(());
        }
        let context = LauncherContext::from_environment()?
            .context("A native settings handoff requires the managed Kindle launcher")?;
        let result = (|| -> Result<()> {
            self.save_system_return_context()?;
            context.write(kind.request())?;
            self.shutdown()?;
            self.closing.set(true);
            slint::quit_event_loop()?;
            Ok(())
        })();
        if let Err(error) = result {
            self.closing.set(false);
            self.resume_reader_after_failed_exit();
            if let Err(cleanup_error) = context.remove_matching(kind.request()) {
                crate::diagnostic!(
                    "Clear an unsuccessful native handoff failed: {cleanup_error:#}"
                );
            }
            if !context.matches("sleep-request").unwrap_or(false) {
                if let Err(cleanup_error) = context.remove_matching("resume-ready") {
                    crate::diagnostic!(
                        "Clear an unsuccessful native acknowledgment failed: {cleanup_error:#}"
                    );
                }
            }
            self.show_toast("native_settings_failed");
            return Err(error);
        }
        Ok(())
    }

    fn native_settings_editing_active(&self) -> bool {
        // Selection modes and their entries can survive navigation to Search
        // or another screen. A process handoff must keep them in this App.
        self.selecting_books
            || !self.selected_books.is_empty()
            || self.selecting_chapters
            || !self.selected_chapters.is_empty()
            || (matches!(self.route, Route::Reader(..)) && self.reader.selection.is_some())
            || (matches!(self.route, Route::Search) && self.search_purpose == "thought")
            || (matches!(self.route, Route::Search) && !self.query.composition.is_empty())
            || (self.route == Route::Login
                && (!self.login_code.is_empty() || !self.login_key.is_empty()))
            || (matches!(self.route, Route::Reader(..) | Route::Search)
                && self
                    .ui
                    .upgrade()
                    .is_some_and(|ui| ui.global::<ReaderModel>().get_writing_thought()))
    }

    fn save_system_return_context(&self) -> Result<()> {
        self.store
            .set_setting(RETURN_CONTEXT_KEY, &self.capture_system_return_context()?)
    }

    fn capture_system_return_context(&self) -> Result<SavedReturnContext> {
        Ok(SavedReturnContext {
            version: 1,
            account_key: reader_account_key(self.demo, self.jobs.session().as_ref())?,
            screen: ReturnScreen::from_route(&self.route),
            history: self.history.iter().map(ReturnScreen::from_route).collect(),
            library_page: self.library_page,
            library_sort: self.library_sort.clone(),
            detail_id: self.detail_id.clone(),
            detail_tab: self.detail_tab.clone(),
            detail_page: self.detail_page,
            detail_reviews: self.reviews.clone(),
            detail_review_latest: self.review_latest,
            settings_subpage: self.settings_subpage.clone(),
            settings_page: self.settings_page,
            publication_account: self.mp_account.clone(),
            publication_query: self.mp_query.clone(),
            publication_sort: self.mp_sort.clone(),
            publication_page: self.mp_page,
            statistics_period: self.stats_period.clone(),
            statistics_offset: self.stats_offset,
            statistics_selected: self.stats_selected,
            search_query: self.query.query.clone(),
            search_scope: self.search_scope.clone(),
            search_purpose: self.search_purpose.clone(),
            search_page: self.search_page,
            search_results: self.search_results.clone(),
            search_status: self.search_status.clone(),
            search_keyboard_visible: self
                .ui
                .upgrade()
                .is_none_or(|ui| ui.global::<SearchModel>().get_keyboard_visible()),
            search_pending: Some(
                matches!(self.route, Route::Search)
                    && self.search_purpose == "books"
                    && self.search_scope == "store"
                    && self.search_status == tr("searching"),
            ),
            article_page: self.article.page,
        })
    }

    /// Normal starts never consume this context. A current launcher generation
    /// must explicitly identify the start as a return from native settings.
    pub(super) fn restore_system_return_context(&mut self) -> Result<()> {
        let result = self.restore_managed_system_return();
        if let Err(error) = result {
            crate::diagnostic!("Restore the native settings return context failed: {error:#}");
            self.show_toast("native_settings_return_failed");
        } else if std::env::var("WEREAD_NATIVE_RETURN_RESULT").as_deref() == Ok("failed") {
            self.show_toast("native_settings_failed");
        }
        Ok(())
    }

    fn restore_managed_system_return(&mut self) -> Result<()> {
        let context = LauncherContext::from_environment()?
            .context("The native return has no managed launcher context")?;
        ensure!(
            context.matches("native-return")?,
            "The native return generation is stale"
        );
        let saved = self
            .store
            .setting::<SavedReturnContext>(RETURN_CONTEXT_KEY)?
            .context("The native return checkpoint is missing")?;
        self.apply_system_return_context(saved)
    }

    fn apply_system_return_context(&mut self, saved: SavedReturnContext) -> Result<()> {
        let owner = reader_account_key(self.demo, self.jobs.session().as_ref())?;
        let (route, history) = saved.resolve(&owner)?;
        let detail_id = saved.detail_snapshot_id(&self.store, &route, &history)?;
        let restart_search = saved.restart_search();
        self.library_page = saved.library_page;
        self.library_sort = saved.library_sort;
        self.settings_subpage = saved.settings_subpage;
        self.settings_page = saved.settings_page;
        self.mp_account = saved.publication_account;
        self.mp_query = saved.publication_query;
        self.mp_sort = saved.publication_sort;
        self.mp_page = saved.publication_page;
        self.stats_period = saved.statistics_period;
        self.stats_offset = saved.statistics_offset;
        self.stats_selected = saved.statistics_selected;
        self.query = PinyinInput::new(saved.search_query);
        self.search_scope = saved.search_scope;
        self.search_purpose = saved.search_purpose;
        self.search_page = saved.search_page;
        self.search_results = saved.search_results;
        self.search_status = saved.search_status;
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<SearchModel>()
                .set_keyboard_visible(saved.search_keyboard_visible);
        }
        if matches!(route, Route::Reader(..)) {
            self.restore_reader_session()?;
        } else {
            self.navigate(route, false)?;
            if matches!(self.route, Route::Article(_)) {
                self.article.page = saved
                    .article_page
                    .min(self.article.pages.len().saturating_sub(1));
            }
        }
        // Loading a different detail initializes its default tab and page.
        // Restore the saved navigation only after that initialization completes.
        if let Some(detail_id) = detail_id {
            self.detail_id = detail_id;
            self.detail_tab = saved.detail_tab;
            self.detail_page = saved.detail_page;
            self.reviews = saved.detail_reviews;
            self.review_latest = saved.detail_review_latest;
        }
        self.history = history;
        if restart_search && self.route == Route::Search {
            // Start a fresh request through the regular capability/account
            // checks without submitting text, changing the keyboard or history.
            self.search_action("scope", &self.search_scope.clone())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::{
        Model,
        platform::{
            Platform as SlintPlatform, WindowAdapter,
            software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
        },
    };

    fn fixture() -> SavedReturnContext {
        SavedReturnContext {
            version: 1,
            account_key: "account:fixture".into(),
            screen: ReturnScreen::Reader {
                book_id: "book-1".into(),
                chapter_index: 3,
            },
            history: vec![ReturnScreen::Detail {
                book_id: "book-1".into(),
            }],
            library_page: 2,
            library_sort: "recent".into(),
            detail_id: "book-1".into(),
            detail_tab: "notes".into(),
            detail_page: 1,
            detail_reviews: vec![],
            detail_review_latest: true,
            settings_subpage: "device".into(),
            settings_page: 0,
            publication_account: None,
            publication_query: String::new(),
            publication_sort: "recent".into(),
            publication_page: 0,
            statistics_period: "week".into(),
            statistics_offset: 0,
            statistics_selected: None,
            search_query: "Fixture".into(),
            search_scope: "store".into(),
            search_purpose: "books".into(),
            search_page: 1,
            search_results: vec![BookMetadata::from(&Book::new("book-1", "Fixture"))],
            search_status: String::new(),
            search_keyboard_visible: false,
            search_pending: Some(false),
            article_page: 0,
        }
    }

    #[test]
    fn native_return_retains_reader_navigation_and_search_results() -> Result<()> {
        let saved: SavedReturnContext = serde_json::from_slice(&serde_json::to_vec(&fixture())?)?;
        let (route, history) = saved.resolve("account:fixture")?;
        assert_eq!(route, Route::Reader("book-1".into(), 3));
        assert_eq!(history, vec![Route::Detail("book-1".into())]);
        assert_eq!(saved.detail_tab, "notes");
        assert_eq!(saved.detail_id, "book-1");
        assert!(saved.detail_review_latest);
        assert_eq!(saved.search_page, 1);
        assert_eq!(saved.search_results[0].id, "book-1");
        assert!(!saved.search_keyboard_visible);
        Ok(())
    }

    #[test]
    fn native_return_rejects_another_account_unknown_schema_and_invalid_history() {
        let mut saved = fixture();
        assert!(saved.resolve("account:another").is_err());
        saved.version = 2;
        assert!(saved.resolve("account:fixture").is_err());
        saved.version = 1;
        saved.history = vec![ReturnScreen::Detail {
            book_id: "book\n1".into(),
        }];
        assert!(saved.resolve("account:fixture").is_err());
    }

    #[test]
    fn detail_snapshot_requires_the_same_existing_book() -> Result<()> {
        let store = Store::open(":memory:")?;
        let mut saved = fixture();
        let (route, history) = saved.resolve("account:fixture")?;
        assert!(
            saved
                .detail_snapshot_id(&store, &route, &history)?
                .is_none()
        );
        store.upsert_book(&Book::new("book-1", "First book"))?;
        store.upsert_book(&Book::new("book-2", "Another book"))?;
        assert_eq!(
            saved
                .detail_snapshot_id(&store, &route, &history)?
                .as_deref(),
            Some("book-1")
        );
        saved.detail_id = "book-2".into();
        let unrelated_history = vec![
            Route::Detail("book-2".into()),
            Route::Detail("book-1".into()),
        ];
        assert!(
            saved
                .detail_snapshot_id(&store, &route, &unrelated_history)?
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn legacy_pending_search_is_retried_without_requerying_completed_results() -> Result<()> {
        let mut saved = fixture();
        saved.screen = ReturnScreen::Search;
        saved.search_status = "A previous process is still searching".into();
        saved.search_results.clear();
        let mut encoded = serde_json::to_value(&saved)?;
        encoded.as_object_mut().unwrap().remove("search_pending");
        let mut restored: SavedReturnContext = serde_json::from_value(encoded)?;
        assert!(restored.restart_search());
        restored.search_pending = Some(false);
        restored.search_status.clear();
        restored.search_results =
            vec![BookMetadata::from(&Book::new("result", "Completed result"))];
        assert!(!restored.restart_search());
        restored.search_pending = Some(true);
        restored.search_purpose = "publications".into();
        assert!(!restored.restart_search());
        Ok(())
    }

    struct ReturnTestPlatform;

    impl SlintPlatform for ReturnTestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
        }
    }

    fn attach_context_fixture(
        directory: &std::path::Path,
        runtime: &tokio::runtime::Runtime,
    ) -> Result<(AppWindow, Rc<RefCell<App>>)> {
        let ui = AppWindow::new()?;
        let platform = Arc::new(Platform::new(DeviceProfile::Paperwhite6, false)?);
        crate::platform::configure_window(ui.window(), &platform)?;
        let application = App::attach(
            &ui,
            AppOptions {
                data_dir: directory.to_path_buf(),
                profile: DeviceProfile::Paperwhite6,
                demo: false,
                offline: true,
                initial_screen: "library".into(),
                fonts: vec![FontRecord {
                    id: "song".into(),
                    label: "Navigation fixture font".into(),
                    family: "serif".into(),
                    metrics: TextMetrics::default(),
                    weights: BTreeMap::new(),
                    available: true,
                }],
            },
            platform,
            runtime,
        )?;
        Ok((ui, application))
    }

    fn run_reader_detail_return() -> Result<()> {
        slint::platform::set_platform(Box::new(ReturnTestPlatform))?;
        let directory = tempfile::tempdir()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (ui, application) = attach_context_fixture(directory.path(), &runtime)?;
        let anchor;
        {
            let mut state = application.borrow_mut();
            state
                .store
                .upsert_book(&Book::new("book-1", "Reader detail fixture"))?;
            state.store.upsert_chapter(&Chapter {
                id: "chapter-1".into(),
                book_id: "book-1".into(),
                index: 0,
                title: "Fixture chapter".into(),
                cached: true,
                body: Some("A readable navigation fixture chapter. ".repeat(1000)),
                content_status: ContentStatus::Full,
                ..Chapter::default()
            })?;
            state.navigate(Route::Detail("book-1".into()), false)?;
            state.detail_tab = "reviews".into();
            state.detail_page = 1;
            state.review_latest = true;
            state.reviews = (0..16)
                .map(|index| RemoteReview {
                    id: format!("review-{index}"),
                    text: format!("Fixture review {index}"),
                    ..Default::default()
                })
                .collect();
            state.navigate(Route::Reader("book-1".into(), 0), true)?;
            state.reader.paginator.as_mut().unwrap().seek(5000);
            anchor = state
                .reader
                .paginator
                .as_ref()
                .unwrap()
                .current_page()
                .start;
            assert!(anchor > 0);
            state.save_system_return_context()?;
            state.save_reader_session()?;
            state.finish_reading()?;
        }
        drop(application);
        drop(ui);
        let (restored_ui, restored) = attach_context_fixture(directory.path(), &runtime)?;
        let saved = restored
            .borrow()
            .store
            .setting::<SavedReturnContext>(RETURN_CONTEXT_KEY)?
            .unwrap();
        {
            let mut state = restored.borrow_mut();
            state.apply_system_return_context(saved)?;
            assert_eq!(state.route, Route::Reader("book-1".into(), 0));
            assert_eq!(
                state
                    .reader
                    .paginator
                    .as_ref()
                    .unwrap()
                    .current_page()
                    .start,
                anchor
            );
            assert_eq!(state.detail_id, "book-1");
            state.back()?;
            state.render()?;
            assert_eq!(state.route, Route::Detail("book-1".into()));
            assert_eq!(state.detail_tab, "reviews");
            assert_eq!(state.detail_page, 1);
            assert!(state.review_latest);
            assert_eq!(state.reviews.len(), 16);
            assert_eq!(state.reviews[0].id, "review-0");
        }
        drop(restored);
        drop(restored_ui);
        runtime.shutdown_timeout(Duration::from_secs(1));
        Ok(())
    }

    #[test]
    fn native_return_reader_back_keeps_detail_tab_page_and_reviews() -> Result<()> {
        std::thread::spawn(run_reader_detail_return)
            .join()
            .expect("The detail return fixture panicked")
    }

    fn run_completed_search_return() -> Result<()> {
        slint::platform::set_platform(Box::new(ReturnTestPlatform))?;
        let directory = tempfile::tempdir()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (ui, application) = attach_context_fixture(directory.path(), &runtime)?;
        let (saved, page_count, first_id, recent);
        {
            let mut state = application.borrow_mut();
            state.route = Route::Search;
            state.history = vec![Route::Library];
            state.query = PinyinInput::new("Completed query");
            state.search_scope = "store".into();
            state.search_purpose = "books".into();
            state.search_page = 1;
            state.search_results = (0..16)
                .map(|index| {
                    BookMetadata::from(&Book::new(
                        format!("result-{index}"),
                        format!("Fixture result {index}"),
                    ))
                })
                .collect();
            state.store.remember_query("Previous query")?;
            ui.global::<SearchModel>().set_keyboard_visible(false);
            state.render()?;
            page_count = ui.global::<SearchModel>().get_page_count();
            first_id = ui
                .global::<SearchModel>()
                .get_results()
                .row_data(0)
                .unwrap()
                .id;
            recent = state.store.recent_queries(8)?;
            saved = state.capture_system_return_context()?;
            assert_eq!(saved.search_pending, Some(false));
        }
        drop(application);
        drop(ui);
        let (restored_ui, restored) = attach_context_fixture(directory.path(), &runtime)?;
        {
            let mut state = restored.borrow_mut();
            let request = state.search_request;
            state.apply_system_return_context(saved)?;
            state.render()?;
            assert!(!restored_ui.global::<SearchModel>().get_keyboard_visible());
            assert_eq!(state.search_page, 1);
            assert_eq!(state.search_request, request);
            assert_eq!(state.search_results.len(), 16);
            assert_eq!(
                restored_ui.global::<SearchModel>().get_page_count(),
                page_count
            );
            assert_eq!(
                restored_ui
                    .global::<SearchModel>()
                    .get_results()
                    .row_data(0)
                    .unwrap()
                    .id,
                first_id
            );
            assert_eq!(state.store.recent_queries(8)?, recent);
        }
        drop(restored);
        drop(restored_ui);
        runtime.shutdown_timeout(Duration::from_secs(1));
        Ok(())
    }

    #[test]
    fn native_return_completed_search_keeps_hidden_keyboard_results_and_page() -> Result<()> {
        std::thread::spawn(run_completed_search_return)
            .join()
            .expect("The search result return fixture panicked")
    }

    fn run_pending_search_return() -> Result<()> {
        slint::platform::set_platform(Box::new(ReturnTestPlatform))?;
        let directory = tempfile::tempdir()?;
        // The current-thread runtime is never driven. Search jobs remain
        // queued, and the fixture delivers their completion without network I/O.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let session = source::AuthSession {
            api_key: "fixture-search-key".into(),
            ..Default::default()
        };
        let (ui, application) = attach_context_fixture(directory.path(), &runtime)?;
        let (saved, recent);
        {
            let mut state = application.borrow_mut();
            state.jobs.replace_session(session.clone())?;
            state.online = true;
            state.force_offline = false;
            state.route = Route::Search;
            state.history = vec![Route::Library];
            state.query = PinyinInput::new("Original pending query");
            state.search_purpose = "books".into();
            state.store.remember_query("Previous query")?;
            ui.global::<SearchModel>().set_keyboard_visible(false);
            state.search_action("scope", "store")?;
            assert_eq!(state.search_status, tr("searching"));
            saved = state.capture_system_return_context()?;
            recent = state.store.recent_queries(8)?;
            assert_eq!(saved.search_pending, Some(true));
        }
        drop(application);
        drop(ui);
        let (restored_ui, restored) = attach_context_fixture(directory.path(), &runtime)?;
        {
            let mut state = restored.borrow_mut();
            state.jobs.replace_session(session.clone())?;
            state.online = true;
            state.force_offline = false;
            let request_before = state.search_request;
            state.apply_system_return_context(saved.clone())?;
            assert!(state.search_request > request_before);
            assert_eq!(state.query.query, "Original pending query");
            assert_eq!(state.search_scope, "store");
            assert_eq!(state.search_purpose, "books");
            assert_eq!(state.history, vec![Route::Library]);
            assert_eq!(state.store.recent_queries(8)?, recent);
            assert!(!restored_ui.global::<SearchModel>().get_keyboard_visible());
            let request = state.search_request;
            let account_epoch = state.jobs.account_epoch();
            state.handle_job(JobEvent {
                credits: None,
                request: request + 1,
                account_epoch,
                result: crate::jobs::JobResult::Search(Ok(vec![])),
            })?;
            assert_eq!(state.search_status, tr("searching"));
            state.handle_job(JobEvent {
                credits: None,
                request,
                account_epoch,
                result: crate::jobs::JobResult::Search(Ok(vec![source::RemoteBook {
                    id: "completed-result".into(),
                    title: "Completed query result".into(),
                    ..Default::default()
                }])),
            })?;
            assert!(state.search_status.is_empty());
            assert_eq!(state.search_results[0].id, "completed-result");
        }
        drop(restored);
        drop(restored_ui);
        let (offline_ui, offline) = attach_context_fixture(directory.path(), &runtime)?;
        {
            let mut state = offline.borrow_mut();
            state.jobs.replace_session(session)?;
            state.apply_system_return_context(saved)?;
            assert_eq!(state.search_status, tr("network_unavailable"));
            assert_ne!(state.search_status, tr("searching"));
            assert_eq!(state.query.query, "Original pending query");
            assert!(!offline_ui.global::<SearchModel>().get_keyboard_visible());
        }
        drop(offline);
        drop(offline_ui);
        runtime.shutdown_timeout(Duration::from_secs(1));
        Ok(())
    }

    #[test]
    fn native_return_pending_search_retries_and_can_complete_or_report_offline() -> Result<()> {
        std::thread::spawn(run_pending_search_return)
            .join()
            .expect("The pending search return fixture panicked")
    }

    fn run_selection_guard() -> Result<()> {
        slint::platform::set_platform(Box::new(ReturnTestPlatform))?;
        let directory = tempfile::tempdir()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (ui, application) = attach_context_fixture(directory.path(), &runtime)?;
        {
            let mut state = application.borrow_mut();
            state.route = Route::Library;
            state.selecting_books = true;
            state.selected_books.insert("book-1".into());
            assert!(state.native_settings_editing_active());
            assert!(state.selected_books.contains("book-1"));
            state.route = Route::Detail("book-1".into());
            state.selecting_chapters = false;
            assert!(state.native_settings_editing_active());
            state.selecting_books = false;
            assert!(state.native_settings_editing_active());
            state.selected_books.clear();
            assert!(!state.native_settings_editing_active());
            state.selecting_chapters = true;
            state.selected_chapters.insert(3);
            assert!(state.native_settings_editing_active());
            assert!(state.selected_chapters.contains(&3));
            state.route = Route::Library;
            assert!(state.native_settings_editing_active());
            state.selecting_chapters = false;
            assert!(state.native_settings_editing_active());
            state.selected_chapters.clear();
            assert!(!state.native_settings_editing_active());
        }
        drop(application);
        drop(ui);
        runtime.shutdown_timeout(Duration::from_secs(1));
        Ok(())
    }

    #[test]
    fn native_handoff_keeps_active_book_and_chapter_selection_in_the_current_app() -> Result<()> {
        std::thread::spawn(run_selection_guard)
            .join()
            .expect("The native selection guard fixture panicked")
    }

    fn run_hidden_selection_guard() -> Result<()> {
        slint::platform::set_platform(Box::new(ReturnTestPlatform))?;
        let directory = tempfile::tempdir()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (ui, application) = attach_context_fixture(directory.path(), &runtime)?;
        {
            let mut state = application.borrow_mut();
            state
                .store
                .upsert_book(&Book::new("book-1", "Selection navigation fixture"))?;
            state.store.upsert_chapter(&Chapter {
                id: "chapter-3".into(),
                book_id: "book-1".into(),
                index: 3,
                title: "Uncached selection fixture".into(),
                cached: false,
                body: None,
                ..Chapter::default()
            })?;
            state.library_action("select", "")?;
            state.library_action("book", "book-1")?;
            state.library_action("search", "store")?;
            assert_eq!(state.route, Route::Search);
            assert!(state.native_settings_editing_active());
            state.search_action("back", "")?;
            assert_eq!(state.route, Route::Library);
            assert!(state.selecting_books);
            assert_eq!(state.selected_books, BTreeSet::from(["book-1".to_owned()]));
            state.library_action("select", "")?;
            assert!(!state.native_settings_editing_active());

            state.navigate(Route::Detail("book-1".into()), true)?;
            state.detail_action("select", "")?;
            state.detail_action("chapter", "chapter-3")?;
            state.back()?;
            assert_eq!(state.route, Route::Library);
            assert!(state.native_settings_editing_active());
            state.navigate(Route::Detail("book-1".into()), true)?;
            assert!(state.selecting_chapters);
            assert_eq!(state.selected_chapters, BTreeSet::from([3]));
            state.detail_action("select", "")?;
            assert!(!state.native_settings_editing_active());
        }
        drop(application);
        drop(ui);
        runtime.shutdown_timeout(Duration::from_secs(1));
        Ok(())
    }

    #[test]
    fn native_handoff_blocks_hidden_book_and_chapter_selection_across_navigation() -> Result<()> {
        std::thread::spawn(run_hidden_selection_guard)
            .join()
            .expect("The hidden native selection fixture panicked")
    }
}
