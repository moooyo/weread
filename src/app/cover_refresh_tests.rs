//! Cover arrivals update stable visible rows and use one non-flashing batch.

use super::underline_projection_tests::isolated;
use super::*;
use crate::{
    jobs::JobResult,
    platform::{PlatformError, RefreshSink},
};
use slint::{
    Model,
    platform::{
        Platform as UiPlatform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::sync::Mutex;

struct CoverUi;

impl UiPlatform for CoverUi {
    fn create_window_adapter(
        &self,
    ) -> std::result::Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

#[derive(Default)]
struct RefreshRecorder(Mutex<Vec<RefreshPlan>>);

struct ReadonlyRows<T> {
    rows: Vec<T>,
    notify: slint::ModelNotify,
}

impl<T: Clone> Model for ReadonlyRows<T> {
    type Data = T;

    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn row_data(&self, row: usize) -> Option<T> {
        self.rows.get(row).cloned()
    }

    fn model_tracker(&self) -> &dyn slint::ModelTracker {
        &self.notify
    }
}

impl RefreshSink for RefreshRecorder {
    fn submit(&self, plan: RefreshPlan) -> std::result::Result<(), PlatformError> {
        self.0.lock().unwrap().push(plan);
        Ok(())
    }
}

impl RefreshRecorder {
    fn take(&self) -> Vec<RefreshPlan> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

struct Fixture {
    application: Rc<RefCell<App>>,
    ui: AppWindow,
    recorder: Arc<RefreshRecorder>,
    runtime: Option<tokio::runtime::Runtime>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Result<Self> {
        slint::platform::set_platform(Box::new(CoverUi))?;
        let directory = tempfile::tempdir()?;
        {
            let store = Store::open(directory.path().join("library.sqlite"))?;
            for index in 0..40 {
                let mut book = Book::new(format!("cover-{index:02}"), format!("Cover {index:02}"));
                book.source = "local".into();
                book.on_shelf = true;
                store.upsert_book(&book)?;
            }
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let platform = Arc::new(Platform::new(DeviceProfile::Paperwhite6, false)?);
        let recorder = Arc::new(RefreshRecorder::default());
        platform.set_refresh_sink(recorder.clone())?;
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
            app.last_tick = Instant::now() + Duration::from_secs(3600);
            app.device_poll_at = app.last_tick;
            app.tick()?;
        }
        recorder.take();
        Ok(Self {
            application,
            ui,
            recorder,
            runtime: Some(runtime),
            _directory: directory,
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.application.borrow_mut().terminal_shutdown();
        let _ = self.ui.hide();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }
}

fn deliver_cover(app: &mut App, id: &str) -> Result<()> {
    let mut image = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2).write_to(&mut image, image::ImageFormat::Png)?;
    app.pending_covers.insert(id.into());
    let epoch = app.jobs.account_epoch();
    assert!(!app.handle_job(JobEvent::new(
        1,
        epoch,
        JobResult::Cover {
            book_id: id.into(),
            result: Ok(image.into_inner()),
        },
    ))?);
    assert!(!app.pending_covers.contains(id));
    Ok(())
}

fn assert_one_nonflashing_batch(recorder: &RefreshRecorder) {
    assert_eq!(
        recorder.take(),
        vec![RefreshPlan {
            kind: RefreshKind::Ui,
            rect: None,
        }]
    );
}

fn stable_library_batch() -> Result<()> {
    let fixture = Fixture::new()?;
    let state = fixture.ui.global::<LibraryModel>();
    let mut app = fixture.application.borrow_mut();
    app.prefs.view_mode = "list".into();
    app.library_sort = "title".into();
    app.library_page = 1;
    app.selecting_books = true;
    app.render_library()?;
    let selected = state.get_books().row_data(0).unwrap().id.to_string();
    app.selected_books.insert(selected);
    app.render_library()?;
    let rows = state.get_books();
    let before: Vec<_> = rows.iter().collect();
    let page = state.get_page();
    let pages = state.get_page_count();
    assert!(page > 1 && before.len() >= 2);

    for tile in before.iter().take(2) {
        deliver_cover(&mut app, tile.id.as_str())?;
    }
    assert!(fixture.recorder.take().is_empty());
    assert!(!rows.row_data(0).unwrap().has_cover);
    app.tick()?;

    assert_eq!(state.get_books(), rows);
    assert_eq!(state.get_page(), page);
    assert_eq!(state.get_page_count(), pages);
    assert_eq!(state.get_selected_count(), 1);
    assert!(state.get_selecting());
    assert_eq!(state.get_sort(), "title");
    for (index, previous) in before.iter().enumerate() {
        let tile = rows.row_data(index).unwrap();
        assert_eq!(tile.id, previous.id);
        assert_eq!(tile.title, previous.title);
        assert_eq!(tile.glyphs, previous.glyphs);
        assert_eq!(tile.selected, previous.selected);
        assert_eq!((tile.row, tile.column), (previous.row, previous.column));
        assert_eq!(tile.has_cover, index < 2);
    }
    assert_one_nonflashing_batch(&fixture.recorder);
    assert!(app.cover_ui_changes.is_empty());
    app.tick()?;
    assert_eq!(state.get_books(), rows);
    assert!(fixture.recorder.take().is_empty());
    Ok(())
}

fn late_and_cached_covers() -> Result<()> {
    let fixture = Fixture::new()?;
    let state = fixture.ui.global::<LibraryModel>();
    let mut app = fixture.application.borrow_mut();
    let old_id = state.get_books().row_data(0).unwrap().id.to_string();
    app.library_page = 1;
    app.render_library()?;
    let rows = state.get_books();
    assert!(!rows.iter().any(|tile| tile.id.as_str() == old_id));
    deliver_cover(&mut app, &old_id)?;
    app.tick()?;
    assert_eq!(state.get_books(), rows);
    assert!(app.cover_ui_changes.is_empty());
    assert!(fixture.recorder.take().is_empty());

    app.library_page = 0;
    app.render_library()?;
    assert!(state.get_books().row_data(0).unwrap().has_cover);
    app.tick()?;
    assert!(fixture.recorder.take().is_empty());

    app.route = Route::Reader("cover-00".into(), 0);
    deliver_cover(&mut app, "cover-01")?;
    app.tick()?;
    assert!(app.cover_ui_changes.is_empty());
    assert!(fixture.recorder.take().is_empty());
    Ok(())
}

fn search_detail_and_readonly_models() -> Result<()> {
    let fixture = Fixture::new()?;
    let mut app = fixture.application.borrow_mut();
    let source_rows = fixture.ui.global::<LibraryModel>().get_books();
    let first = source_rows.row_data(0).unwrap();
    let second = source_rows.row_data(1).unwrap();
    app.route = Route::Search;
    app.search_purpose = "books".into();
    app.search_scope = "store".into();
    app.search_results = [first.id.as_str(), second.id.as_str()]
        .into_iter()
        .map(|id| app.store.book_metadata_by_id(id).map(Option::unwrap))
        .collect::<Result<Vec<_>>>()?;
    app.search_results[0].author = "Cover author".into();
    app.search_results[0].category = "Cover category".into();
    app.render_search()?;
    let search = fixture.ui.global::<SearchModel>();
    let rows = search.get_results();
    let first_result = rows.row_data(0).unwrap();
    let second_result = rows.row_data(1).unwrap();
    assert_eq!(first_result.cover.size().width, 0);
    deliver_cover(&mut app, first.id.as_str())?;
    app.tick()?;
    assert_eq!(search.get_results(), rows);
    let loaded = rows.row_data(0).unwrap();
    assert!(loaded.cover.size().width > 0);
    assert_eq!(loaded.id, first_result.id);
    assert_eq!(loaded.title, first_result.title);
    assert_eq!(loaded.author, first_result.author);
    assert_eq!(loaded.category, first_result.category);
    assert_eq!(loaded.initial, first_result.initial);
    assert_eq!(loaded.on_shelf, first_result.on_shelf);
    assert_eq!(loaded.tone, first_result.tone);
    assert_eq!(rows.row_data(1).unwrap().cover.size().width, 0);
    assert_one_nonflashing_batch(&fixture.recorder);

    // Read-only models fall back without changing the displayed order.
    let readonly: ModelRc<SearchResult> = Rc::new(ReadonlyRows {
        rows: vec![second_result, loaded],
        notify: slint::ModelNotify::default(),
    })
    .into();
    search.set_results(readonly.clone());
    deliver_cover(&mut app, second.id.as_str())?;
    app.tick()?;
    assert_ne!(search.get_results(), readonly);
    assert_eq!(search.get_results().row_data(0).unwrap().id, second.id);
    assert!(search.get_results().row_data(0).unwrap().cover.size().width > 0);
    assert_eq!(search.get_results().row_data(1).unwrap().id, first.id);
    assert_one_nonflashing_batch(&fixture.recorder);

    app.route = Route::Detail(second.id.to_string());
    let detail = fixture.ui.global::<DetailModel>();
    detail.set_book(second.clone());
    deliver_cover(&mut app, second.id.as_str())?;
    app.tick()?;
    assert!(detail.get_book().has_cover);
    assert_eq!(detail.get_book().id, second.id);
    assert_one_nonflashing_batch(&fixture.recorder);
    Ok(())
}

#[test]
fn cover_batches_preserve_library_rows_selection_and_pagination() -> Result<()> {
    isolated(
        "app::cover_refresh_tests::cover_batches_preserve_library_rows_selection_and_pagination",
        stable_library_batch,
    )
}

#[test]
fn late_cover_arrivals_do_not_refresh_other_pages_or_reader() -> Result<()> {
    isolated(
        "app::cover_refresh_tests::late_cover_arrivals_do_not_refresh_other_pages_or_reader",
        late_and_cached_covers,
    )
}

#[test]
fn visible_search_and_detail_covers_update_without_rebuilding() -> Result<()> {
    isolated(
        "app::cover_refresh_tests::visible_search_and_detail_covers_update_without_rebuilding",
        search_detail_and_readonly_models,
    )
}
