use super::*;

#[path = "download_cache_reuse_tests.rs"]
mod cache_reuse_tests;
#[path = "download_recovery_tests.rs"]
mod recovery_tests;
use crate::{
    book_assets::{PreparedChapterImages, prepare_chapter_metadata, prepare_downloaded},
    jobs::JobResult,
    source::images::{DownloadedChapterImages, ImageAsset},
};
use slint::{
    Model,
    platform::{
        Platform as SlintPlatform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{io::Cursor, path::Path};

struct DownloadTestPlatform;

impl SlintPlatform for DownloadTestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}

fn on_ui_thread(scenario: fn(&AppWindow, &Rc<RefCell<App>>, &Path) -> Result<()>) -> Result<()> {
    std::thread::spawn(move || -> Result<()> {
        slint::platform::set_platform(Box::new(DownloadTestPlatform))?;
        let directory = tempfile::tempdir()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let ui = AppWindow::new()?;
        let platform = Arc::new(Platform::new(DeviceProfile::Scribe, false)?);
        crate::platform::configure_window(ui.window(), &platform)?;
        let application = App::attach(
            &ui,
            AppOptions {
                data_dir: directory.path().to_path_buf(),
                profile: DeviceProfile::Scribe,
                demo: false,
                offline: true,
                initial_screen: "library".into(),
                fonts: vec![FontRecord {
                    id: "song".into(),
                    label: "Download fixture font".into(),
                    family: "serif".into(),
                    metrics: TextMetrics::default(),
                    weights: BTreeMap::new(),
                    available: true,
                }],
            },
            platform,
            &runtime,
        )?;
        application.borrow_mut().stop_lighting_hardware()?;
        scenario(&ui, &application, directory.path())?;
        drop(application);
        drop(ui);
        runtime.shutdown_timeout(Duration::from_secs(1));
        Ok(())
    })
    .join()
    .expect("The download fixture UI thread panicked")
}

fn seed(state: &mut App, count: u32) -> Result<()> {
    let mut book = Book::new("fixture-book", "Download fixture book");
    book.total_chapters = count;
    state.store.upsert_book(&book)?;
    for index in 0..count {
        state.store.upsert_chapter(&Chapter {
            id: format!("fixture-chapter-{index}"),
            book_id: book.id.clone(),
            index,
            title: format!("Fixture chapter {index}"),
            ..Chapter::default()
        })?;
    }
    state.navigate(Route::Library, false)?;
    state.render()?;
    Ok(())
}

fn seed_remote(state: &mut App, count: u32) -> Result<()> {
    seed(state, count)?;
    let mut book = state.store.book("fixture-book")?.unwrap();
    book.source = "weread".into();
    state.store.upsert_book(&book)?;
    state.jobs.replace_session(source::AuthSession {
        user_vid: "fixture-access-reader".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "fixture-access-reader".into()),
            ("wr_skey".into(), "synthetic-access-session".into()),
        ]),
        ..source::AuthSession::default()
    })?;
    Ok(())
}

fn begin_task(state: &mut App, indices: Vec<u32>, include_images: bool) -> Result<DownloadTask> {
    let task = DownloadTask {
        id: unique_id("fixture-download"),
        book_id: "fixture-book".into(),
        account_key: state
            .jobs
            .session()
            .as_ref()
            .and_then(library::download_account_key)
            .unwrap_or_else(|| "fixture-account".into()),
        chapters: indices,
        status: DownloadStatus::Running,
        include_images,
        updated_at: 100,
        ..DownloadTask::default()
    };
    state.store.save_task(&task)?;
    state
        .store
        .prepare_download_chapter_states(&task, task.updated_at)?;
    let task = state.store.task(&task.id)?.unwrap();
    state.current_download = Some(task.clone());
    state.download_request = 51;
    state.download_ui_refresh_at = Instant::now() + Duration::from_secs(60);
    state.last_tick = Instant::now();
    Ok(task)
}

fn packet_event(
    state: &App,
    index: u32,
    text: &str,
    html: Option<&str>,
    images: Option<PreparedChapterImages>,
) -> Result<(JobEvent, tokio::sync::oneshot::Receiver<source::Result<()>>)> {
    let task = state.current_download.as_ref().unwrap();
    let mut chapter = state.store.chapter(&task.book_id, index)?.unwrap();
    chapter.body = Some(text.into());
    chapter.cached = true;
    chapter.content_status = ContentStatus::Full;
    chapter.content_account_key = task.account_key.clone();
    let metadata = html
        .map(|html| prepare_chapter_metadata(html, text))
        .transpose()?;
    let (checkpoint, reply) = tokio::sync::oneshot::channel();
    Ok((
        JobEvent {
            credits: None,
            request: state.download_request,
            account_epoch: state.jobs.account_epoch(),
            result: JobResult::Downloaded {
                task_id: task.id.clone(),
                chapter,
                next_index: task
                    .chapters
                    .iter()
                    .position(|selected| *selected == index)
                    .unwrap()
                    + 1,
                html: html.map(str::to_owned),
                metadata,
                images,
                checkpoint,
            },
        },
        reply,
    ))
}

fn access_packet_event(
    state: &App,
    index: u32,
    text: &str,
    html: Option<&str>,
    status: ContentStatus,
) -> Result<(JobEvent, tokio::sync::oneshot::Receiver<source::Result<()>>)> {
    let (mut packet, reply) = packet_event(state, index, text, html, None)?;
    if let JobResult::Downloaded {
        chapter,
        html,
        metadata,
        ..
    } = &mut packet.result
    {
        chapter.content_status = status;
        chapter.cached = status == ContentStatus::Full;
        if status == ContentStatus::Locked && text.is_empty() {
            chapter.body = None;
            *html = None;
            *metadata = None;
        }
    }
    Ok((packet, reply))
}

fn event(state: &App, result: JobResult) -> JobEvent {
    JobEvent {
        credits: None,
        request: state.download_request,
        account_epoch: state.jobs.account_epoch(),
        result,
    }
}

fn png_bytes() -> Result<Vec<u8>> {
    let image = image::RgbImage::from_pixel(8, 6, image::Rgb([27, 63, 119]));
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image).write_to(&mut output, image::ImageFormat::Png)?;
    Ok(output.into_inner())
}

fn cover_access_spy(state: &mut App) -> Result<rusqlite::Connection> {
    state.cache.put_png("cover:fixture-book", &png_bytes()?)?;
    state.render()?;
    let connection = rusqlite::Connection::open(state.cache.root().join("cache-index.sqlite3"))?;
    connection.execute_batch(
        "CREATE TABLE fixture_cover_access (key TEXT NOT NULL);
         CREATE TRIGGER record_fixture_cover_access AFTER UPDATE OF last_access ON cache_entries
         BEGIN INSERT INTO fixture_cover_access(key) VALUES(NEW.key); END;",
    )?;
    Ok(connection)
}

fn cover_accesses(connection: &rusqlite::Connection) -> Result<usize> {
    Ok(
        connection.query_row("SELECT COUNT(*) FROM fixture_cover_access", [], |row| {
            row.get(0)
        })?,
    )
}

fn run_background_checkpoints(
    ui: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed(&mut state, 3)?;
    let access = cover_access_spy(&mut state)?;
    let original_rows = ui.global::<LibraryModel>().get_books();
    assert!(original_rows.row_data(0).unwrap().has_cover);
    assert!(!original_rows.row_data(0).unwrap().cached);
    let task = begin_task(&mut state, vec![0, 1, 2], false)?;
    let (sender, receiver) = mpsc::channel();
    state.events = receiver;
    for index in 0..2 {
        let (packet, mut reply) = packet_event(
            &state,
            index,
            "A complete fixture body.",
            Some("<p>A complete fixture body.</p>"),
            None,
        )?;
        sender.send(packet)?;
        state.tick()?;
        assert!(matches!(reply.try_recv(), Ok(Ok(()))));
        assert_eq!(
            state.current_download.as_ref().unwrap().next_index,
            index as usize + 1
        );
        assert_eq!(
            state.store.task(&task.id)?.unwrap().next_index,
            index as usize + 1
        );
        assert!(!original_rows.row_data(0).unwrap().cached);
        assert_eq!(cover_accesses(&access)?, 0);
    }
    sender.send(event(
        &state,
        JobResult::DownloadProgress {
            task_id: task.id,
            completed: 2,
            waiting: 0,
            blocked: 0,
            total: 3,
            title: "Fixture chapter 1".into(),
        },
    ))?;
    state.tick()?;
    assert_eq!(
        ui.global::<LibraryModel>().get_status(),
        ui.global::<ShellModel>().get_task_label()
    );
    assert!(!ui.global::<LibraryModel>().get_status().is_empty());
    assert_eq!(cover_accesses(&access)?, 0);

    let due = state.download_ui_refresh_at;
    state.flush_download_ui(due)?;
    assert!(original_rows.row_data(0).unwrap().cached);
    assert!(original_rows.row_data(0).unwrap().has_cover);
    assert!(
        ui.global::<LibraryModel>()
            .get_books()
            .row_data(0)
            .unwrap()
            .cached
    );
    assert!(state.download_ui_changes.is_empty());
    assert_eq!(cover_accesses(&access)?, 0);
    Ok(())
}

#[test]
fn background_checkpoints_keep_cover_models_and_only_flush_download_fields_when_due() -> Result<()>
{
    on_ui_thread(run_background_checkpoints)
}

fn run_detail_checkpoint(ui: &AppWindow, application: &Rc<RefCell<App>>, _: &Path) -> Result<()> {
    let mut state = application.borrow_mut();
    seed(&mut state, 2)?;
    state.navigate(Route::Detail("fixture-book".into()), false)?;
    let access = cover_access_spy(&mut state)?;
    let original_rows = ui.global::<DetailModel>().get_chapters();
    let task = begin_task(&mut state, vec![0, 1], false)?;
    let (packet, mut reply) = packet_event(
        &state,
        0,
        "Readable detail fixture.",
        Some("<p>Readable detail fixture.</p>"),
        None,
    )?;
    assert!(!state.handle_job(packet)?);
    assert!(matches!(reply.try_recv(), Ok(Ok(()))));
    let due = state.download_ui_refresh_at;
    state.flush_download_ui(due)?;
    assert!(original_rows.row_data(0).unwrap().cached);
    assert!(ui.global::<DetailModel>().get_book().cached);
    assert!(ui.global::<DetailModel>().get_book().has_cover);
    assert!(ui.global::<DetailModel>().get_can_read());
    assert_eq!(state.store.task(&task.id)?.unwrap().next_index, 1);
    assert_eq!(cover_accesses(&access)?, 0);
    Ok(())
}

#[test]
fn detail_download_updates_count_and_chapter_flags_without_loading_its_cover() -> Result<()> {
    on_ui_thread(run_detail_checkpoint)
}

fn run_pending_reader(ui: &AppWindow, application: &Rc<RefCell<App>>, _: &Path) -> Result<()> {
    let mut state = application.borrow_mut();
    seed(&mut state, 2)?;
    state.navigate(Route::Reader("fixture-book".into(), 0), false)?;
    state.render()?;
    state.pending_read = Some(("fixture-book".into(), 0));
    begin_task(&mut state, vec![0, 1], false)?;
    let (sender, receiver) = mpsc::channel();
    state.events = receiver;
    let text = "The requested reading chapter is now available. ".repeat(30);
    let (packet, mut reply) = packet_event(
        &state,
        0,
        &text,
        Some("<p>The requested reading chapter is now available.</p>"),
        None,
    )?;
    sender.send(packet)?;
    state.tick()?;
    assert!(matches!(reply.try_recv(), Ok(Ok(()))));
    assert!(state.pending_read.is_none());
    assert_eq!(state.reader.body, text);
    assert!(state.reader.paginator.is_some());
    assert_eq!(state.route, Route::Reader("fixture-book".into(), 0));
    assert!(ui.global::<ReaderModel>().get_lines().row_count() > 0);
    assert!(state.download_ui_changes.is_empty());
    Ok(())
}

#[test]
fn requested_reader_chapter_renders_immediately_before_the_progress_refresh_deadline() -> Result<()>
{
    on_ui_thread(run_pending_reader)
}

fn run_reader_image_complement(
    ui: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    use sha2::{Digest, Sha256};
    let mut state = application.borrow_mut();
    seed(&mut state, 1)?;
    let text = "Before the fixture image.\nAfter the fixture image.";
    let html = "<p>Before the fixture image.</p><img src=\"images/fixture.png\"><p>After the fixture image.</p>";
    state.book_assets.commit_chapter(
        &state.store,
        "fixture-book",
        "fixture-chapter-0",
        html,
        text,
        None,
    )?;
    state.store.cache_chapter_body("fixture-book", 0, text)?;
    state.navigate(Route::Reader("fixture-book".into(), 0), false)?;
    state.render()?;
    assert_eq!(ui.global::<ReaderModel>().get_images().row_count(), 1);
    assert!(
        !ui.global::<ReaderModel>()
            .get_images()
            .row_data(0)
            .unwrap()
            .loaded
    );
    let anchor = state
        .reader
        .paginator
        .as_ref()
        .unwrap()
        .current_page()
        .start;
    let task = begin_task(&mut state, vec![0], true)?;
    let bytes = png_bytes()?;
    let id = hex::encode(Sha256::digest(&bytes));
    let downloaded = DownloadedChapterImages {
        source_to_asset: BTreeMap::from([("images/fixture.png".into(), id.clone())]),
        assets: vec![ImageAsset {
            id,
            media_type: "image/png".into(),
            extension: "png".into(),
            source_names: vec!["images/fixture.png".into()],
            bytes,
        }],
    };
    let prepared = prepare_downloaded(downloaded)?;
    let (packet, mut reply) = packet_event(&state, 0, text, Some(html), Some(prepared))?;
    let (sender, receiver) = mpsc::channel();
    state.events = receiver;
    sender.send(packet)?;
    state.tick()?;
    assert!(matches!(reply.try_recv(), Ok(Ok(()))));
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
    assert_eq!(ui.global::<ReaderModel>().get_images().row_count(), 1);
    assert!(
        ui.global::<ReaderModel>()
            .get_images()
            .row_data(0)
            .unwrap()
            .loaded
    );
    assert!(
        state
            .book_assets
            .complete(&state.store, "fixture-book", "fixture-chapter-0")?
    );
    assert_eq!(
        state.store.book("fixture-book")?.unwrap().cached_chapters,
        1
    );
    assert_eq!(state.store.task(&task.id)?.unwrap().next_index, 1);
    Ok(())
}

#[test]
fn active_reader_image_complement_reflows_immediately_without_double_counting_text() -> Result<()> {
    on_ui_thread(run_reader_image_complement)
}

fn run_incomplete_images_keep_text(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_remote(&mut state, 2)?;
    let task = begin_task(&mut state, vec![0, 1], true)?;
    let text = "A stable paragraph with a missing requested image.";
    let html = format!("<p>{text}</p><img src='images/missing.png'>");
    let (packet, mut reply) = packet_event(&state, 0, text, Some(&html), None)?;
    state.handle_job(packet)?;
    assert!(matches!(
        reply.try_recv(),
        Ok(Err(source::SourceError::ContentUnverified))
    ));
    let body = state.store.chapter("fixture-book", 0)?.unwrap();
    assert_eq!(body.body.as_deref(), Some(text));
    assert!(body.cached);
    assert_eq!(body.content_status, ContentStatus::Full);
    assert_eq!(state.store.task(&task.id)?.unwrap().completed_count, 0);
    assert!(
        !state
            .book_assets
            .chapter(&state.store, "fixture-book", &body.id)?
            .unwrap()
            .complete
    );
    let personal = fixture_personal_records(&state)?;
    let (checkpoint, mut retry) = tokio::sync::oneshot::channel();
    let failed = event(
        &state,
        JobResult::DownloadChapterFailed {
            task_id: task.id.clone(),
            chapter_id: body.id,
            chapter_index: 0,
            ordinal: 0,
            error: source::SourceError::ContentUnverified,
            checkpoint,
        },
    );
    state.handle_job(failed)?;
    assert!(matches!(
        retry.try_recv(),
        Ok(Ok(DownloadChapterRetry::Blocked))
    ));
    let next = "An independently complete image-free chapter.";
    let (packet, mut reply) = packet_event(&state, 1, next, Some(&format!("<p>{next}</p>")), None)?;
    state.handle_job(packet)?;
    assert!(matches!(reply.try_recv(), Ok(Ok(()))));
    let saved = state.store.task(&task.id)?.unwrap();
    assert_eq!(saved.completed_count, 1);
    assert_eq!(saved.next_index, 0);
    assert_eq!(saved.status, DownloadStatus::Running);
    assert_eq!(
        state.store.chapter_body("fixture-book", 0)?.as_deref(),
        Some(text)
    );
    assert_eq!(state.store.annotations("fixture-book")?, vec![personal.0]);
    assert_eq!(state.store.bookmarks("fixture-book")?, vec![personal.1]);
    assert_eq!(state.store.position("fixture-book")?, Some(personal.2));
    Ok(())
}

#[test]
fn incomplete_image_publication_keeps_full_text_without_completing_or_stopping_later_chapters()
-> Result<()> {
    on_ui_thread(run_incomplete_images_keep_text)
}

fn run_rejected_checkpoint(ui: &AppWindow, application: &Rc<RefCell<App>>, _: &Path) -> Result<()> {
    let mut state = application.borrow_mut();
    seed(&mut state, 2)?;
    for mismatch in ["account", "cursor", "plan", "cancelled", "updated_at"] {
        let task = begin_task(&mut state, vec![0, 1], false)?;
        let mut newer = task.clone();
        match mismatch {
            "account" => newer.account_key = "different-fixture-account".into(),
            "cursor" => newer.next_index = 1,
            "plan" => newer.chapters = vec![1, 0],
            "cancelled" => newer.status = DownloadStatus::Cancelled,
            "updated_at" => newer.updated_at = 101,
            _ => unreachable!(),
        }
        state.store.save_task(&newer)?;
        let (packet, mut reply) = packet_event(
            &state,
            0,
            "Rejected fixture body.",
            Some("<p>Rejected fixture body.</p>"),
            None,
        )?;
        assert!(state.handle_job(packet)?);
        assert!(matches!(
            reply.try_recv(),
            Ok(Err(source::SourceError::CheckpointFailed))
        ));
        assert_eq!(state.current_download.as_ref().unwrap(), &task);
        assert_eq!(state.store.task(&task.id)?.unwrap(), newer);
        assert_eq!(state.store.chapter_body("fixture-book", 0)?, None);
        state.dialog_action = "downloading".into();
        ui.global::<ShellModel>()
            .set_task_label("Rejected fixture download".into());
        ui.global::<ShellModel>()
            .set_dialog_kind("downloading".into());
        let failed = event(
            &state,
            JobResult::DownloadFailed {
                task_id: task.id.clone(),
                error: source::SourceError::CheckpointFailed,
            },
        );
        assert!(state.handle_job(failed)?);
        assert!(state.current_download.is_none());
        assert_eq!(state.store.task(&task.id)?.unwrap(), newer);
        assert_eq!(state.store.chapter_body("fixture-book", 0)?, None);
        assert!(ui.global::<ShellModel>().get_task_label().is_empty());
        assert!(ui.global::<ShellModel>().get_dialog_kind().is_empty());
        assert!(state.download_ui_changes.is_empty());
    }
    Ok(())
}

#[test]
fn rejected_persistent_checkpoint_does_not_advance_the_in_memory_task_or_acknowledge_success()
-> Result<()> {
    on_ui_thread(run_rejected_checkpoint)
}

fn run_terminal_feedback(ui: &AppWindow, application: &Rc<RefCell<App>>, _: &Path) -> Result<()> {
    let mut state = application.borrow_mut();
    seed(&mut state, 1)?;
    for terminal in ["finish", "cancel", "network"] {
        let mut task = begin_task(&mut state, vec![0], false)?;
        if terminal == "finish" {
            let (packet, mut reply) = packet_event(
                &state,
                0,
                "A completed terminal fixture.",
                Some("<p>A completed terminal fixture.</p>"),
                None,
            )?;
            state.handle_job(packet)?;
            assert!(matches!(reply.try_recv(), Ok(Ok(()))));
            task = state.current_download.as_ref().unwrap().clone();
        }
        state.online = true;
        state.dialog_action = "downloading".into();
        ui.global::<ShellModel>()
            .set_dialog_kind("downloading".into());
        ui.global::<ShellModel>()
            .set_task_label("Fixture download in progress".into());
        let result = match terminal {
            "finish" => JobResult::DownloadFinished {
                task_id: task.id.clone(),
            },
            "cancel" => JobResult::Cancelled {
                task_id: task.id.clone(),
            },
            "network" => JobResult::DownloadFailed {
                task_id: task.id.clone(),
                error: source::SourceError::Network,
            },
            _ => unreachable!(),
        };
        let (sender, receiver) = mpsc::channel();
        state.events = receiver;
        sender.send(event(&state, result))?;
        state.tick()?;
        assert!(state.current_download.is_none());
        assert!(ui.global::<ShellModel>().get_task_label().is_empty());
        assert!(ui.global::<ShellModel>().get_dialog_kind().is_empty());
        let expected = match terminal {
            "finish" => DownloadStatus::Complete,
            "cancel" => DownloadStatus::Cancelled,
            "network" => DownloadStatus::Paused,
            _ => unreachable!(),
        };
        assert_eq!(state.store.task(&task.id)?.unwrap().status, expected);
        if terminal == "network" {
            assert!(!ui.global::<ShellModel>().get_online());
        }
    }
    Ok(())
}

#[test]
fn completion_cancellation_and_failure_refresh_the_shell_without_waiting_for_download_throttling()
-> Result<()> {
    on_ui_thread(run_terminal_feedback)
}

fn run_old_account_checkpoint(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed(&mut state, 1)?;
    let task = begin_task(&mut state, vec![0], false)?;
    let (packet, mut reply) = packet_event(
        &state,
        0,
        "Old account fixture.",
        Some("<p>Old account fixture.</p>"),
        None,
    )?;
    state.jobs.clear_session();
    assert!(!state.handle_job(packet)?);
    assert!(matches!(
        reply.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    ));
    assert_eq!(state.store.task(&task.id)?.unwrap().next_index, 0);
    assert_eq!(state.store.chapter_body("fixture-book", 0)?, None);
    let chapter = state.store.chapter("fixture-book", 0)?.unwrap();
    assert_eq!(chapter.content_status, ContentStatus::Unknown);
    assert!(chapter.content_account_key.is_empty());
    assert!(!chapter.cached);
    assert_eq!(
        state.store.book("fixture-book")?.unwrap().cached_chapters,
        0
    );
    assert!(
        state
            .book_assets
            .chapter(&state.store, "fixture-book", "fixture-chapter-0")?
            .is_none()
    );
    assert!(state.download_ui_changes.is_empty());
    Ok(())
}

#[test]
fn an_old_account_epoch_cannot_change_download_storage_or_schedule_a_route_render() -> Result<()> {
    on_ui_thread(run_old_account_checkpoint)
}

fn run_inconsistent_metadata(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed(&mut state, 1)?;
    for corruption in ["missing_metadata", "missing_html", "images_without_html"] {
        let task = begin_task(&mut state, vec![0], false)?;
        let (mut packet, mut reply) = packet_event(
            &state,
            0,
            "Invalid packet fixture.",
            Some("<p>Invalid packet fixture.</p>"),
            None,
        )?;
        if let JobResult::Downloaded {
            html,
            metadata,
            images,
            ..
        } = &mut packet.result
        {
            match corruption {
                "missing_metadata" => *metadata = None,
                "missing_html" => *html = None,
                "images_without_html" => {
                    *html = None;
                    *metadata = None;
                    *images = Some(prepare_downloaded(DownloadedChapterImages::default())?);
                }
                _ => unreachable!(),
            }
        }
        assert!(state.handle_job(packet)?);
        assert!(matches!(
            reply.try_recv(),
            Ok(Err(source::SourceError::CheckpointFailed))
        ));
        assert_eq!(state.store.chapter_body("fixture-book", 0)?, None);
        assert_eq!(state.current_download.as_ref().unwrap(), &task);
        assert_eq!(state.store.task(&task.id)?.unwrap().next_index, 0);
    }
    Ok(())
}

#[test]
fn inconsistent_html_metadata_and_image_packets_are_rejected_before_advancing_the_cursor()
-> Result<()> {
    on_ui_thread(run_inconsistent_metadata)
}

fn run_filtered_library(ui: &AppWindow, application: &Rc<RefCell<App>>, _: &Path) -> Result<()> {
    let mut state = application.borrow_mut();
    seed(&mut state, 3)?;
    state.library_sort = "downloaded".into();
    state.render()?;
    assert_eq!(ui.global::<LibraryModel>().get_books().row_count(), 0);
    begin_task(&mut state, vec![0, 1, 2], false)?;
    for index in 0..2 {
        let (packet, mut reply) = packet_event(
            &state,
            index,
            "Filtered fixture body.",
            Some("<p>Filtered fixture body.</p>"),
            None,
        )?;
        assert!(!state.handle_job(packet)?);
        assert!(matches!(reply.try_recv(), Ok(Ok(()))));
    }
    let due = state.download_ui_refresh_at;
    state.flush_download_ui(due)?;
    let rows = ui.global::<LibraryModel>().get_books();
    assert_eq!(rows.row_count(), 1);
    assert!(rows.row_data(0).unwrap().cached);
    assert_eq!(ui.global::<LibraryModel>().get_page_count(), 1);
    let (packet, mut reply) = packet_event(
        &state,
        2,
        "Filtered fixture body.",
        Some("<p>Filtered fixture body.</p>"),
        None,
    )?;
    assert!(!state.handle_job(packet)?);
    assert!(matches!(reply.try_recv(), Ok(Ok(()))));
    let due = state.download_ui_refresh_at;
    state.flush_download_ui(due)?;
    assert_eq!(rows.row_count(), 1);
    assert_eq!(
        state.store.book("fixture-book")?.unwrap().cached_chapters,
        3
    );
    Ok(())
}

#[test]
fn a_book_first_cached_in_a_checkpoint_batch_enters_the_downloaded_filter() -> Result<()> {
    on_ui_thread(run_filtered_library)
}

fn run_list_retry_layout(ui: &AppWindow, application: &Rc<RefCell<App>>, _: &Path) -> Result<()> {
    let mut state = application.borrow_mut();
    seed(&mut state, 2)?;
    state.profile = DeviceProfile::Paperwhite5;
    state.platform = Arc::new(Platform::new(DeviceProfile::Paperwhite5, false)?);
    crate::platform::configure_window(ui.window(), &state.platform)?;
    state.prefs.view_mode = "list".into();
    state.online = true;
    for index in 0..13 {
        state.store.upsert_book(&Book::new(
            format!("fixture-list-book-{index}"),
            format!("Fixture list book {index}"),
        ))?;
    }
    let access = cover_access_spy(&mut state)?;
    assert_eq!(ui.global::<LibraryModel>().get_books().row_count(), 7);
    assert_eq!(ui.global::<LibraryModel>().get_page_count(), 2);
    let task = begin_task(&mut state, vec![0, 1], false)?;
    // A timer-driven retry sets the shell label after the previous library render.
    ui.global::<ShellModel>()
        .set_task_label("Fixture retry in progress".into());
    let (packet, mut reply) = packet_event(
        &state,
        0,
        "Retry fixture body.",
        Some("<p>Retry fixture body.</p>"),
        None,
    )?;
    assert!(!state.handle_job(packet)?);
    assert!(matches!(reply.try_recv(), Ok(Ok(()))));
    let progress = event(
        &state,
        JobResult::DownloadProgress {
            task_id: task.id.clone(),
            completed: 1,
            waiting: 0,
            blocked: 0,
            total: 2,
            title: "Fixture chapter 0".into(),
        },
    );
    assert!(!state.handle_job(progress)?);
    assert_eq!(ui.global::<LibraryModel>().get_books().row_count(), 6);
    assert_eq!(ui.global::<LibraryModel>().get_page_count(), 3);
    let first_accesses = cover_accesses(&access)?;
    assert!(first_accesses > 0);
    let progress = event(
        &state,
        JobResult::DownloadProgress {
            task_id: task.id,
            completed: 1,
            waiting: 0,
            blocked: 0,
            total: 2,
            title: "Fixture chapter 0".into(),
        },
    );
    assert!(!state.handle_job(progress)?);
    assert_eq!(cover_accesses(&access)?, first_accesses);
    assert_eq!(ui.global::<LibraryModel>().get_books().row_count(), 6);
    Ok(())
}

#[test]
fn a_retry_progress_status_row_reflows_list_pagination_once() -> Result<()> {
    on_ui_thread(run_list_retry_layout)
}

fn run_incomplete_content_checkpoints(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_remote(&mut state, 3)?;
    for (index, status) in [
        ContentStatus::Preview,
        ContentStatus::Unknown,
        ContentStatus::Locked,
    ]
    .into_iter()
    .enumerate()
    {
        let task = begin_task(&mut state, vec![index as u32], false)?;
        let text = if status == ContentStatus::Locked {
            ""
        } else {
            "A synthetic readable excerpt..."
        };
        let html = (!text.is_empty()).then_some("<p>A synthetic readable excerpt...</p>");
        let (packet, mut reply) = access_packet_event(&state, index as u32, text, html, status)?;
        state.handle_job(packet)?;
        assert!(matches!(reply.try_recv(), Ok(Ok(()))));
        let chapter = state.store.chapter("fixture-book", index as u32)?.unwrap();
        assert_eq!(chapter.content_status, status);
        assert_eq!(chapter.content_account_key, task.account_key);
        assert!(!chapter.cached);
        assert_eq!(chapter.body.as_deref(), (!text.is_empty()).then_some(text));
        assert_eq!(state.current_download.as_ref().unwrap().next_index, 0);
        assert_eq!(state.store.task(&task.id)?.unwrap().next_index, 0);
        assert_eq!(
            state.store.book("fixture-book")?.unwrap().cached_chapters,
            0
        );
        let mut completed = state.current_download.as_ref().unwrap().clone();
        completed.status = DownloadStatus::Complete;
        completed.next_index = completed.chapters.len();
        assert!(state.store.save_task(&completed).is_err());
        assert_eq!(state.store.task(&task.id)?.unwrap().next_index, 0);
    }
    Ok(())
}

#[test]
fn incomplete_checkpoints_preserve_available_excerpts_without_completing_a_task() -> Result<()> {
    on_ui_thread(run_incomplete_content_checkpoints)
}

fn run_access_terminal_feedback(
    ui: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_remote(&mut state, 1)?;
    for error in [
        source::SourceError::UnlockRequired,
        source::SourceError::ContentUnverified,
        source::SourceError::ContentChanged,
    ] {
        let task = begin_task(&mut state, vec![0], false)?;
        state.online = true;
        let route = state.route.clone();
        let redacted_error = error.to_string();
        state.dialog_action = "downloading".into();
        ui.global::<ShellModel>()
            .set_dialog_kind("downloading".into());
        ui.global::<ShellModel>()
            .set_task_label("Fixture content check in progress".into());
        let (checkpoint, mut retry) = tokio::sync::oneshot::channel();
        let failure = event(
            &state,
            JobResult::DownloadChapterFailed {
                task_id: task.id.clone(),
                chapter_id: "fixture-chapter-0".into(),
                chapter_index: 0,
                ordinal: 0,
                error,
                checkpoint,
            },
        );
        state.handle_job(failure)?;
        assert!(matches!(
            retry.try_recv(),
            Ok(Ok(DownloadChapterRetry::Blocked))
        ));
        assert_eq!(
            state.current_download.as_ref().unwrap().status,
            DownloadStatus::Running
        );
        let terminal = event(
            &state,
            JobResult::DownloadIncomplete {
                task_id: task.id.clone(),
                completed: 0,
                blocked: 1,
                total: 1,
            },
        );
        state.handle_job(terminal)?;
        let saved = state.store.task(&task.id)?.unwrap();
        assert_eq!(saved.status, DownloadStatus::Failed);
        assert_eq!(saved.next_index, 0);
        assert!(saved.error.is_none());
        assert_eq!(
            state.store.download_chapter_states(&task.id)?[0]
                .error
                .as_deref(),
            Some(redacted_error.as_str())
        );
        assert_eq!(state.route, route);
        assert!(state.online);
        assert!(state.current_download.is_none());
        assert!(state.downloads.is_empty());
        assert!(!ui.global::<ShellModel>().get_task_label().is_empty());
        assert_eq!(
            ui.global::<ShellModel>().get_dialog_kind().as_str(),
            "confirm"
        );
        assert_eq!(
            ui.global::<ShellModel>().get_dialog_primary().as_str(),
            tr("download_retry_chapters")
        );
        assert_ne!(
            ui.global::<ShellModel>().get_toast().as_str(),
            tr("session_expired").as_str()
        );
        state.force_offline = false;
        state.download_retry_at = Instant::now() - Duration::from_secs(61);
        state.retry_pending_downloads()?;
        assert!(state.current_download.is_none());
        assert!(state.downloads.is_empty());
        assert_eq!(state.store.task(&task.id)?.unwrap(), saved);
    }
    Ok(())
}

#[test]
fn access_failures_block_only_the_chapter_and_offer_retry_without_expiring_login() -> Result<()> {
    on_ui_thread(run_access_terminal_feedback)
}

fn fixture_personal_records(state: &App) -> Result<(Annotation, Bookmark, ReadingPosition)> {
    let note = Annotation {
        id: "fixture-access-note".into(),
        book_id: "fixture-book".into(),
        chapter_index: 0,
        start: 2,
        end: 8,
        quote: "stable".into(),
        note: "A synthetic note on the retained prefix.".into(),
        created_at: 21,
    };
    let bookmark = Bookmark {
        id: "fixture-access-bookmark".into(),
        book_id: "fixture-book".into(),
        chapter_index: 0,
        char_offset: 20,
        label: "Synthetic retained bookmark".into(),
        created_at: 22,
    };
    let position = ReadingPosition {
        book_id: "fixture-book".into(),
        chapter_index: 0,
        char_offset: 12,
        updated_at: 23,
        device: "fixture-reader".into(),
    };
    state.store.save_annotation(&note)?;
    state.store.save_bookmark(&bookmark)?;
    state.store.save_position(&position)?;
    Ok((note, bookmark, position))
}

fn run_unlocked_prefix_upgrade(
    ui: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_remote(&mut state, 1)?;
    let prefix = "A stable preview paragraph that includes a bookmark and a note.";
    let preview = format!("{prefix}...");
    let preview_html = format!("<p>{preview}</p>");
    let task = begin_task(&mut state, vec![0], false)?;
    let (packet, mut reply) = access_packet_event(
        &state,
        0,
        &preview,
        Some(&preview_html),
        ContentStatus::Preview,
    )?;
    state.handle_job(packet)?;
    assert!(matches!(reply.try_recv(), Ok(Ok(()))));
    let terminal = event(
        &state,
        JobResult::DownloadFailed {
            task_id: task.id.clone(),
            error: source::SourceError::UnlockRequired,
        },
    );
    state.handle_job(terminal)?;
    let (note, bookmark, position) = fixture_personal_records(&state)?;
    state.navigate(Route::Reader("fixture-book".into(), 0), false)?;
    state.render()?;
    assert_eq!(state.reader.body, preview);
    assert!(ui.global::<ReaderModel>().get_lines().row_count() > 0);
    assert_eq!(
        ui.global::<ReaderModel>().get_access_label().as_str(),
        tr("content_preview").as_str()
    );
    assert_eq!(
        state.store.book("fixture-book")?.unwrap().cached_chapters,
        0
    );

    let full = format!("{prefix} The authorized full chapter continues with its remaining text.");
    let full_html = format!("<p>{full}</p>");
    let task = begin_task(&mut state, vec![0], false)?;
    let (packet, mut reply) = packet_event(&state, 0, &full, Some(&full_html), None)?;
    state.handle_job(packet)?;
    assert!(matches!(reply.try_recv(), Ok(Ok(()))));
    state.render()?;
    let chapter = state.store.chapter("fixture-book", 0)?.unwrap();
    assert_eq!(chapter.body.as_deref(), Some(full.as_str()));
    assert_eq!(chapter.content_status, ContentStatus::Full);
    assert_eq!(chapter.content_account_key, task.account_key);
    assert!(chapter.cached);
    assert_eq!(
        state.chapter_content_status(&state.store.book("fixture-book")?.unwrap(), &chapter),
        ContentStatus::Full
    );
    assert!(ui.global::<ReaderModel>().get_access_label().is_empty());
    assert!(ui.global::<ReaderModel>().get_access_hint().is_empty());
    assert!(!ui.global::<ReaderModel>().get_access_notice());
    assert_eq!(state.reader.body, full);
    assert_eq!(state.route, Route::Reader("fixture-book".into(), 0));
    assert_eq!(state.store.annotations("fixture-book")?, vec![note]);
    assert_eq!(state.store.bookmarks("fixture-book")?, vec![bookmark]);
    assert_eq!(state.store.position("fixture-book")?, Some(position));
    assert_eq!(
        state.store.book("fixture-book")?.unwrap().cached_chapters,
        1
    );
    assert_eq!(state.store.task(&task.id)?.unwrap().next_index, 1);
    let manifest = state
        .book_assets
        .chapter(&state.store, "fixture-book", "fixture-chapter-0")?
        .unwrap();
    assert_eq!(manifest.html, full_html);
    assert!(manifest.complete);
    let terminal = event(
        &state,
        JobResult::DownloadFinished {
            task_id: task.id.clone(),
        },
    );
    state.handle_job(terminal)?;
    assert_eq!(
        state.store.task(&task.id)?.unwrap().status,
        DownloadStatus::Complete
    );
    Ok(())
}

#[test]
fn an_unlocked_prefix_replaces_a_preview_and_rebuilds_its_manifest_without_losing_personal_records()
-> Result<()> {
    on_ui_thread(run_unlocked_prefix_upgrade)
}

fn run_changed_content_preservation(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_remote(&mut state, 1)?;
    let preview = "A stable preview paragraph that includes a bookmark and a note...";
    let preview_html = format!("<p>{preview}</p>");
    begin_task(&mut state, vec![0], false)?;
    let (packet, mut reply) = access_packet_event(
        &state,
        0,
        preview,
        Some(&preview_html),
        ContentStatus::Preview,
    )?;
    state.handle_job(packet)?;
    assert!(matches!(reply.try_recv(), Ok(Ok(()))));
    let (note, bookmark, position) = fixture_personal_records(&state)?;
    let original = state
        .book_assets
        .chapter(&state.store, "fixture-book", "fixture-chapter-0")?
        .unwrap();
    let task = state.current_download.as_ref().unwrap().clone();
    let replacement = "An unrelated revised chapter would invalidate the existing scalar anchors.";
    let replacement_html = format!("<p>{replacement}</p>");
    let (packet, mut reply) = packet_event(&state, 0, replacement, Some(&replacement_html), None)?;
    state.handle_job(packet)?;
    assert!(matches!(
        reply.try_recv(),
        Ok(Err(source::SourceError::ContentChanged))
    ));
    assert_eq!(
        state.store.chapter_body("fixture-book", 0)?.as_deref(),
        Some(preview)
    );
    assert_eq!(
        state
            .store
            .chapter("fixture-book", 0)?
            .unwrap()
            .content_status,
        ContentStatus::Preview
    );
    assert_eq!(
        state
            .book_assets
            .chapter(&state.store, "fixture-book", "fixture-chapter-0")?,
        Some(original)
    );
    assert_eq!(state.store.annotations("fixture-book")?, vec![note]);
    assert_eq!(state.store.bookmarks("fixture-book")?, vec![bookmark]);
    assert_eq!(state.store.position("fixture-book")?, Some(position));
    assert_eq!(state.store.task(&task.id)?.unwrap().next_index, 0);
    assert_eq!(
        state.store.book("fixture-book")?.unwrap().cached_chapters,
        0
    );
    Ok(())
}

#[test]
fn an_incompatible_full_response_is_rejected_before_replacing_the_preview_manifest() -> Result<()> {
    on_ui_thread(run_changed_content_preservation)
}

fn run_refresh_after_cancellation(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_remote(&mut state, 1)?;
    let task = begin_task(&mut state, vec![0], false)?;
    let cancelled = event(
        &state,
        JobResult::Cancelled {
            task_id: task.id.clone(),
        },
    );
    state.handle_job(cancelled)?;
    let original = state.store.task(&task.id)?.unwrap();
    assert_eq!(original.status, DownloadStatus::Cancelled);
    state.force_offline = false;
    let occupied = occupy_download_slot(&mut state);
    state.refresh_content_access("fixture-book", vec![0])?;
    let refreshed = state.downloads.front().unwrap().clone();
    assert_ne!(refreshed.id, original.id);
    assert_eq!(refreshed.chapters, vec![0]);
    assert_eq!(refreshed.next_index, 0);
    assert_eq!(refreshed.status, DownloadStatus::Pending);
    assert_eq!(state.current_download.as_ref(), Some(&occupied));
    assert_eq!(state.store.task(&original.id)?.unwrap(), original);
    assert_eq!(state.store.chapter_body("fixture-book", 0)?, None);
    Ok(())
}

#[test]
fn an_explicit_refresh_creates_a_new_task_without_reviving_a_cancelled_download() -> Result<()> {
    on_ui_thread(run_refresh_after_cancellation)
}

fn occupy_download_slot(state: &mut App) -> DownloadTask {
    let occupied = DownloadTask {
        id: "fixture-unrelated-download".into(),
        book_id: "fixture-unrelated-book".into(),
        status: DownloadStatus::Running,
        chapters: vec![0],
        ..DownloadTask::default()
    };
    state.current_download = Some(occupied.clone());
    occupied
}

fn run_explicit_access_refresh(
    _: &AppWindow,
    application: &Rc<RefCell<App>>,
    _: &Path,
) -> Result<()> {
    let mut state = application.borrow_mut();
    seed_remote(&mut state, 2)?;
    let task = begin_task(&mut state, vec![0, 1], false)?;
    let preview = "A stable preview paragraph that includes a bookmark and a note...";
    let html = format!("<p>{preview}</p>");
    let (packet, mut reply) =
        access_packet_event(&state, 0, preview, Some(&html), ContentStatus::Preview)?;
    state.handle_job(packet)?;
    assert!(matches!(reply.try_recv(), Ok(Ok(()))));
    let (note, bookmark, position) = fixture_personal_records(&state)?;
    let terminal = event(
        &state,
        JobResult::DownloadFailed {
            task_id: task.id.clone(),
            error: source::SourceError::UnlockRequired,
        },
    );
    state.handle_job(terminal)?;
    let paused = state.store.task(&task.id)?.unwrap();
    assert_eq!(paused.status, DownloadStatus::Paused);
    state.force_offline = false;
    state.online = true;
    state.downloads.push_back(paused.clone());
    state.start_next_download()?;
    assert!(state.current_download.is_none());
    assert!(state.downloads.is_empty());
    assert_eq!(state.store.task(&task.id)?.unwrap(), paused);

    // Holding another task in the slot makes this an entirely synchronous
    // scheduling test: no source worker or authenticated request is started.
    let occupied = occupy_download_slot(&mut state);
    state.refresh_content_access("fixture-book", vec![0])?;
    let refreshed = state.store.task(&task.id)?.unwrap();
    assert_eq!(refreshed.id, paused.id);
    assert_eq!(refreshed.account_key, paused.account_key);
    assert_eq!(refreshed.chapters, paused.chapters);
    assert_eq!(refreshed.include_images, paused.include_images);
    assert_eq!(refreshed.next_index, paused.next_index);
    assert_eq!(refreshed.status, DownloadStatus::Pending);
    assert!(refreshed.error.is_none());
    assert_eq!(state.current_download.as_ref(), Some(&occupied));
    assert_eq!(state.downloads.front(), Some(&refreshed));
    assert_eq!(state.downloads.len(), 1);
    assert_eq!(state.store.annotations("fixture-book")?, vec![note]);
    assert_eq!(state.store.bookmarks("fixture-book")?, vec![bookmark]);
    assert_eq!(state.store.position("fixture-book")?, Some(position));
    assert_eq!(
        state.store.chapter_body("fixture-book", 0)?.as_deref(),
        Some(preview)
    );
    assert_eq!(
        state.store.book("fixture-book")?.unwrap().cached_chapters,
        0
    );
    Ok(())
}

#[test]
fn only_an_explicit_access_refresh_requeues_a_blocked_task_and_preserves_its_plan_and_notes()
-> Result<()> {
    on_ui_thread(run_explicit_access_refresh)
}
