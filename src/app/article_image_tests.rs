//! Current-article image admission through real App entry points and job fences.
//! Transport futures only receive file URLs, rejected before any HTTP request.

use super::*;
use crate::{
    jobs::JobResult,
    platform::{ManagedSleepState, PlatformError, RenderSink},
};
use anyhow::{Context, bail, ensure};
use slint::platform::{
    Platform as UiPlatform, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::Mutex,
};

const ARTICLE_ID: &str = "fixture-article";
const IMAGE_COUNT: usize = 7;
const CHILD_CASE: &str = "WEREAD_ARTICLE_IMAGE_APP_CASE";
const COMPLETED: &str = "WEREAD_ARTICLE_IMAGE_APP_COMPLETED";
const CHILD_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_CAPTURE_BYTES: u64 = 64 * 1024;

struct ArticleUi;

impl UiPlatform for ArticleUi {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

struct ArticleDisplay(Mutex<ManagedSleepState>);

impl Default for ArticleDisplay {
    fn default() -> Self {
        Self(Mutex::new(ManagedSleepState::Running))
    }
}

impl RenderSink for ArticleDisplay {
    fn set_dark_mode(&self, _: bool) -> Result<(), PlatformError> {
        Ok(())
    }

    fn managed_sleep_supported(&self) -> bool {
        true
    }

    fn set_managed_sleep(&self, sleeping: bool) -> Result<(), PlatformError> {
        *self.0.lock().unwrap() = if sleeping {
            ManagedSleepState::Suspended
        } else {
            ManagedSleepState::Running
        };
        Ok(())
    }

    fn managed_sleep_state(&self) -> Result<ManagedSleepState, PlatformError> {
        Ok(*self.0.lock().unwrap())
    }
}

#[derive(Debug)]
struct ImagePacket {
    request: u64,
    account_epoch: u64,
    key: String,
    id: String,
    credits: RefCell<Option<crate::jobs::JobCredits>>,
}

impl Clone for ImagePacket {
    fn clone(&self) -> Self {
        // A fixture identity replay is not another transport-owned payload.
        Self {
            request: self.request,
            account_epoch: self.account_epoch,
            key: self.key.clone(),
            id: self.id.clone(),
            credits: RefCell::new(None),
        }
    }
}

impl ImagePacket {
    fn event(&self, result: source::Result<Vec<u8>>) -> JobEvent {
        JobEvent {
            credits: self.credits.borrow_mut().take(),
            request: self.request,
            account_epoch: self.account_epoch,
            result: JobResult::ArticleImage {
                article_id: ARTICLE_ID.into(),
                image_id: self.id.clone(),
                result,
            },
        }
    }
}

fn png() -> Result<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(2, 2, image::Luma([204])))
        .write_to(&mut output, image::ImageFormat::Png)?;
    Ok(output.into_inner())
}

fn quiet_periodic_work(app: &mut App) {
    app.last_tick = Instant::now() + Duration::from_secs(3600);
    app.device_poll_at = app.last_tick;
}

fn seed_and_open(app: &mut App, automatic: bool, marker: &str) -> Result<()> {
    app.force_offline = true;
    app.prefs.download_article_images = automatic;
    let book_id = format!("article:{ARTICLE_ID}");
    let mut book = Book::new(&book_id, "Article admission fixture");
    book.source = "article".into();
    book.on_shelf = false;
    book.total_chapters = 1;
    book.cached_chapters = 1;
    app.store.upsert_book(&book)?;
    let mut html = format!("<article id='js_content'><p>{marker}</p>");
    for index in 0..IMAGE_COUNT {
        html.push_str(&format!(
            "<p>Paragraph {index}</p><img src='https://images.example.test/{index}.png' alt='Image {index}'>"
        ));
    }
    html.push_str("<p>Final paragraph</p></article>");
    app.store.upsert_chapter(&Chapter {
        id: ARTICLE_ID.into(),
        book_id,
        index: 0,
        cached: true,
        body: Some(format!("{marker}\nFixture article body")),
        content_status: ContentStatus::Full,
        ..Chapter::default()
    })?;
    app.store
        .set_setting(&format!("article.html:{ARTICLE_ID}"), &html)?;
    let parses = app.article.document_parses;
    if app.route == Route::Article(ARTICLE_ID.into()) {
        app.load_article(ARTICLE_ID)?;
    } else {
        app.navigate(Route::Article(ARTICLE_ID.into()), false)?;
    }
    assert_eq!(app.article.document_parses, parses + 1);
    assert_eq!(app.article.image_work.images().len(), IMAGE_COUNT);
    assert_eq!(app.article.image_work.has_work(), automatic);
    assert!(app.pending_article_images.is_empty());

    // Keep the parsed production IDs and document layout. Only replace the
    // transport URLs, before enabling online admission, with a non-network
    // scheme that the real adapter rejects before constructing a request.
    let mut metadata = app.article.image_work.images().to_vec();
    for (index, image) in metadata.iter_mut().enumerate() {
        image.url = format!("file:///synthetic-article-{index}.png");
    }
    app.article.image_work.reset(ARTICLE_ID);
    app.article.image_work.install(metadata);
    for index in 0..IMAGE_COUNT {
        app.article.image_work.mark_cache_checked(index);
    }
    if automatic {
        app.article.image_work.start_auto();
    }
    app.force_offline = false;
    app.online = true;
    quiet_periodic_work(app);
    Ok(())
}

fn collect_transport(runtime: &tokio::runtime::Runtime, app: &App) -> Result<Vec<ImagePacket>> {
    assert!(
        app.article
            .image_work
            .images()
            .iter()
            .all(|image| image.url.starts_with("file://"))
    );
    let expected = app.pending_article_images.len();
    let mut packets = Vec::new();
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), async {
            while packets.len() < expected {
                match app.events.try_recv() {
                    Ok(event) => {
                        let JobEvent {
                            request,
                            account_epoch,
                            result,
                            credits,
                        } = event;
                        let JobResult::ArticleImage {
                            article_id,
                            image_id,
                            result,
                        } = result
                        else {
                            bail!("The isolated image fixture received an unrelated job result");
                        };
                        ensure!(
                            matches!(
                                result,
                                Err(source::SourceError::InvalidResponse("image_url"))
                            ),
                            "A fixture URL reached unexpected transport behavior"
                        );
                        ensure!(
                            article_id == ARTICLE_ID,
                            "The image result belongs to another article"
                        );
                        packets.push(ImagePacket {
                            request,
                            account_epoch,
                            key: image_processing::article_cache_key(
                                &article_id,
                                &image_id,
                                &app.article
                                    .image_work
                                    .images()
                                    .iter()
                                    .find(|image| image.id == image_id)
                                    .unwrap()
                                    .url,
                            ),
                            id: image_id,
                            credits: RefCell::new(credits),
                        });
                    }
                    Err(mpsc::TryRecvError::Empty) => tokio::task::yield_now().await,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        bail!("The image fixture sender closed")
                    }
                }
            }
            Ok(())
        })
        .await
        .context("The isolated image transport did not publish its queued results")?
    })?;
    Ok(packets)
}

fn finish_all(runtime: &tokio::runtime::Runtime, app: &mut App) -> Result<usize> {
    let bytes = png()?;
    let mut accepted = 0;
    for _ in 0..IMAGE_COUNT + 2 {
        app.tick()?;
        let packets = collect_transport(runtime, app)?;
        if packets.is_empty() {
            ensure!(
                !app.article.image_work.has_work() && app.pending_article_images.is_empty(),
                "Current-article work stalled without an admitted transport"
            );
            return Ok(accepted);
        }
        for packet in packets {
            assert_eq!(
                app.article.image_work.request_for(&packet.id),
                Some(packet.request)
            );
            assert_eq!(packet.account_epoch, app.jobs.account_epoch());
            assert!(app.handle_job(packet.event(Ok(bytes.clone())))?);
            app.continue_article_image_work()?;
            assert!(
                app.pending_article_images.len() <= 2,
                "Transport completion released a credit before the UI settled its payload"
            );
            accepted += 1;
            ensure!(
                accepted <= IMAGE_COUNT,
                "An image was automatically requested more than once"
            );
        }
    }
    bail!("The bounded article fixture did not drain its image work")
}

fn assert_idle(app: &mut App) -> Result<()> {
    assert!(!app.article.image_work.has_work());
    assert!(app.pending_article_images.is_empty());
    assert_eq!(app.jobs.cover_capacity(), 2);
    let parses = app.article.document_parses;
    let checks = app.article.image_cache_checks;
    quiet_periodic_work(app);
    for _ in 0..32 {
        app.render()?;
        app.tick()?;
    }
    assert_eq!(
        app.article.document_parses, parses,
        "Idle rendering reparsed article HTML"
    );
    assert_eq!(
        app.article.image_cache_checks, checks,
        "Idle rendering rescanned cached images"
    );
    assert!(app.events.try_recv().is_err());
    Ok(())
}

fn bulk_case(runtime: &tokio::runtime::Runtime, app: &mut App, automatic: bool) -> Result<()> {
    seed_and_open(app, automatic, "Original document")?;
    let parses = app.article.document_parses;
    let checks = app.article.image_cache_checks;
    if automatic {
        app.tick()?;
    } else {
        app.dispatch("article", "article.load-all-images", "")?;
        assert!(!app.prefs.download_article_images);
    }
    assert_eq!(app.pending_article_images.len(), 2);
    assert_eq!(
        app.article.document_parses, parses,
        "Batch admission reparsed the document"
    );
    // An explicit load-all retry rechecks each candidate once. Automatic
    // admission reuses the initial document cache checks.
    assert_eq!(
        app.article.image_cache_checks,
        checks + if automatic { 0 } else { 2 }
    );
    // Both transports have completed, but their queued payloads still own the
    // global permits. Popping them into this fixture must not release admission.
    let packets = collect_transport(runtime, app)?;
    assert_eq!(packets.len(), 2);
    assert_eq!(app.jobs.cover_capacity(), 0);
    assert_eq!(app.article.image_work.available_capacity(), 0);
    let mut packets = packets.into_iter();
    assert!(app.handle_job(packets.next().unwrap().event(Ok(png()?)))?);
    app.continue_article_image_work()?;
    assert_eq!(app.pending_article_images.len(), 2);
    assert_eq!(app.article.image_work.available_capacity(), 0);
    assert!(app.handle_job(packets.next().unwrap().event(Ok(png()?)))?);
    app.continue_article_image_work()?;
    assert!(app.pending_article_images.len() <= 2);
    assert_eq!(finish_all(runtime, app)?, IMAGE_COUNT - 2);
    assert_eq!(app.article.loaded_images.len(), IMAGE_COUNT);
    assert_eq!(
        app.article.document_parses,
        parses + IMAGE_COUNT,
        "Each completed image may reflow once, but admission must not parse per candidate"
    );
    assert_eq!(
        app.article.image_cache_checks,
        checks + if automatic { 0 } else { IMAGE_COUNT }
    );
    assert_idle(app)
}

fn capacity_case(runtime: &tokio::runtime::Runtime, app: &mut App) -> Result<()> {
    seed_and_open(app, false, "Capacity document")?;
    let capacity = app.jobs.metadata_capacity();
    assert_eq!(capacity, 8);
    for request in 0..capacity {
        assert!(
            app.jobs
                .search(1000 + request as u64, "Never polled fixture query".into())
                .is_none()
        );
    }
    assert_eq!(app.jobs.cover_capacity(), 0);
    let parses = app.article.document_parses;
    let checks = app.article.image_cache_checks;
    app.dispatch("article", "article.load-all-images", "")?;
    assert!(app.article.image_work.has_work());
    assert!(app.pending_article_images.is_empty());
    // Cancel metadata before its first poll: no non-file source operation is
    // driven, and a real epoch change releases the occupied request budget.
    app.jobs.cancel_all();
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), async {
            while app.jobs.metadata_capacity() != capacity {
                tokio::task::yield_now().await;
            }
        })
        .await
        .context("Cancelled metadata did not release capacity")
    })?;
    assert!(app.events.try_recv().is_err());
    app.tick()?;
    assert_eq!(app.pending_article_images.len(), 2);
    assert_eq!(app.article.document_parses, parses);
    assert_eq!(app.article.image_cache_checks, checks + 2);
    assert_eq!(finish_all(runtime, app)?, IMAGE_COUNT);
    assert_eq!(app.article.image_cache_checks, checks + IMAGE_COUNT);
    assert_idle(app)
}

fn failure_case(runtime: &tokio::runtime::Runtime, app: &mut App) -> Result<()> {
    seed_and_open(app, true, "Failure document")?;
    app.tick()?;
    let mut packets = collect_transport(runtime, app)?;
    assert_eq!(packets.len(), 2);
    let failed = packets.remove(0);
    let parses = app.article.document_parses;
    assert!(app.handle_job(failed.event(Err(source::SourceError::Network)))?);
    assert_eq!(app.article.document_parses, parses);
    assert!(!app.online);
    for packet in packets {
        assert!(app.handle_job(packet.event(Ok(png()?)))?);
    }
    assert_eq!(app.article.loaded_images.len(), 1);
    assert!(app.article.image_work.has_work());
    assert!(app.pending_article_images.is_empty());
    assert_eq!(app.article.image_work.request_for(&failed.id), None);
    let checks = app.article.image_cache_checks;
    let paused_parses = app.article.document_parses;
    for _ in 0..32 {
        app.render()?;
        app.tick()?;
    }
    assert!(!app.online);
    assert!(app.events.try_recv().is_err());
    assert!(app.pending_article_images.is_empty());
    assert_eq!(app.article.document_parses, paused_parses);
    assert_eq!(app.article.image_cache_checks, checks);
    app.reflow_article()?;
    assert!(app.pending_article_images.is_empty());
    assert_eq!(app.article.image_cache_checks, checks);
    app.dispatch("article", "article.auto-images", "")?;
    assert!(!app.prefs.download_article_images);
    assert!(!app.online);
    app.dispatch("article", "article.auto-images", "")?;
    assert!(app.prefs.download_article_images);
    assert!(
        app.online,
        "Explicit auto enable did not restore network admission intent"
    );
    assert_eq!(app.pending_article_images.len(), 2);
    assert_eq!(app.article.image_work.request_for(&failed.id), None);
    assert_eq!(finish_all(runtime, app)?, IMAGE_COUNT - 2);
    assert_eq!(app.article.loaded_images.len(), IMAGE_COUNT - 1);
    assert_idle(app)?;
    app.dispatch("article", "article.load-image", &failed.id)?;
    let retries = collect_transport(runtime, app)?;
    assert_eq!(retries.len(), 1);
    let current = &retries[0];
    assert_eq!(current.id, failed.id);
    assert_ne!(current.request, failed.request);
    assert!(!app.handle_job(failed.event(Ok(png()?)))?);
    assert_eq!(app.article.loaded_images.len(), IMAGE_COUNT - 1);
    assert!(app.handle_job(current.event(Ok(png()?)))?);
    assert_eq!(app.article.loaded_images.len(), IMAGE_COUNT);
    assert_idle(app)
}

fn cancel_busy_case(runtime: &tokio::runtime::Runtime, app: &mut App) -> Result<()> {
    seed_and_open(app, true, "Cancelled document")?;
    app.tick()?;
    let mut packets = collect_transport(runtime, app)?;
    assert_eq!(packets.len(), 2);
    app.dispatch("article", "article.auto-images", "")?;
    assert!(!app.prefs.download_article_images);
    let busy = packets.remove(0);
    assert!(app.handle_job(busy.event(Err(source::SourceError::Busy)))?);
    assert!(!app.article.image_work.has_work());
    for packet in packets {
        assert!(app.handle_job(packet.event(Ok(png()?)))?);
    }
    assert_eq!(app.article.loaded_images.len(), 1);
    assert_idle(app)?;
    app.dispatch("article", "article.auto-images", "")?;
    assert!(app.prefs.download_article_images);
    assert_eq!(finish_all(runtime, app)?, IMAGE_COUNT - 1);
    assert_eq!(app.article.loaded_images.len(), IMAGE_COUNT);
    assert_idle(app)
}

fn stale_document_case(runtime: &tokio::runtime::Runtime, app: &mut App) -> Result<()> {
    seed_and_open(app, false, "Old document marker")?;
    app.dispatch("article", "article.load-all-images", "")?;
    let old = collect_transport(runtime, app)?;
    assert_eq!(old.len(), 2);
    app.navigate(Route::Library, false)?;
    assert!(!app.article.image_work.has_work());
    assert!(app.pending_article_images.is_empty());
    for packet in &old {
        assert!(!app.handle_job(packet.event(Ok(png()?)))?);
        assert!(app.cache.get_for_commit(&packet.key)?.is_none());
    }
    seed_and_open(app, false, "New document marker")?;
    app.dispatch("article", "article.load-all-images", "")?;
    let current = collect_transport(runtime, app)?;
    assert_eq!(current.len(), 2);
    assert!(app.article.pages.len() > 1);
    app.article.page = 1;
    let parses = app.article.document_parses;
    let checks = app.article.image_cache_checks;
    for packet in &old {
        assert!(!app.handle_job(packet.event(Ok(png()?)))?);
    }
    assert!(app.article.html.contains("New document marker"));
    assert!(app.article.loaded_images.is_empty());
    assert_eq!(app.article.page, 1);
    assert_eq!(app.article.document_parses, parses);
    assert_eq!(app.article.image_cache_checks, checks);
    for packet in current {
        assert_ne!(
            Some(packet.request),
            old.iter()
                .find(|old| old.id == packet.id)
                .map(|old| old.request)
        );
        assert!(app.handle_job(packet.event(Ok(png()?)))?);
    }
    assert_eq!(finish_all(runtime, app)?, IMAGE_COUNT - 2);
    assert_eq!(app.article.loaded_images.len(), IMAGE_COUNT);
    assert_idle(app)
}

fn resident_case(runtime: &tokio::runtime::Runtime, app: &mut App) -> Result<()> {
    seed_and_open(app, true, "Resident document")?;
    app.tick()?;
    let old = collect_transport(runtime, app)?;
    assert_eq!(old.len(), 2);
    assert!(app.article.pages.len() > 1);
    app.article.page = 1;
    let html = app.article.html.clone();
    let parses = app.article.document_parses;
    let checks = app.article.image_cache_checks;
    app.prepare_resident_sleep()?;
    assert!(app.managed_sleep_blocked);
    assert!(app.resident_lifecycle.is_prepared());
    assert!(app.pending_article_images.is_empty());
    assert!(app.article.image_work.has_work());
    for packet in &old {
        assert_eq!(
            app.article.image_work.accepts(&packet.id, packet.request),
            None
        );
        assert!(!app.handle_job(packet.event(Ok(png()?)))?);
    }
    app.resume_resident_sleep()?;
    assert!(!app.managed_sleep_blocked);
    assert_eq!(app.article.html, html);
    assert_eq!(app.article.page, 1);
    assert_eq!(app.article.document_parses, parses);
    assert_eq!(app.article.image_cache_checks, checks);
    quiet_periodic_work(app);
    app.tick()?;
    let replacements = collect_transport(runtime, app)?;
    assert_eq!(replacements.len(), 2);
    for packet in &old {
        assert!(!app.handle_job(packet.event(Ok(png()?)))?);
    }
    assert!(app.article.loaded_images.is_empty());
    for packet in replacements {
        assert_ne!(
            Some(packet.request),
            old.iter()
                .find(|old| old.id == packet.id)
                .map(|old| old.request)
        );
        assert!(app.handle_job(packet.event(Ok(png()?)))?);
    }
    assert_eq!(finish_all(runtime, app)?, IMAGE_COUNT - 2);
    assert_eq!(app.article.loaded_images.len(), IMAGE_COUNT);
    assert_idle(app)
}

fn app_case(scenario: &str) -> Result<()> {
    slint::platform::set_platform(Box::new(ArticleUi))?;
    let directory = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let platform = Arc::new(Platform::new(DeviceProfile::Scribe, false)?);
    platform.set_render_sink(Arc::new(ArticleDisplay::default()))?;
    let ui = AppWindow::new()?;
    crate::platform::configure_window(ui.window(), &platform)?;
    let application = App::attach(
        &ui,
        AppOptions {
            data_dir: directory.path().to_path_buf(),
            profile: DeviceProfile::Scribe,
            demo: false,
            offline: true,
            initial_screen: "library".into(),
            fonts: vec![],
        },
        platform,
        &runtime,
    )?;
    let result = {
        let mut app = application.borrow_mut();
        app.stop_lighting_hardware()?;
        app.prefs.report_time = false;
        app.prefs.push_on_close = false;
        quiet_periodic_work(&mut app);
        match scenario {
            "manual" => bulk_case(&runtime, &mut app, false),
            "automatic" => bulk_case(&runtime, &mut app, true),
            "capacity" => capacity_case(&runtime, &mut app),
            "failure" => failure_case(&runtime, &mut app),
            "cancel-busy" => cancel_busy_case(&runtime, &mut app),
            "stale-document" => stale_document_case(&runtime, &mut app),
            "resident" => resident_case(&runtime, &mut app),
            _ => bail!("Unknown article integration scenario"),
        }
    };
    {
        let mut app = application.borrow_mut();
        app.jobs.cancel_all();
        app.clear_article_image_work();
        app.shutdown()?;
    }
    drop(application);
    drop(ui);
    runtime.shutdown_timeout(Duration::from_secs(1));
    result
}

struct ChildOwner(Option<Child>);

impl ChildOwner {
    fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        self.0
            .as_mut()
            .context("The article fixture child was already reaped")?
            .try_wait()
            .context("Poll the article fixture child")
    }

    fn reap(&mut self) -> Result<ExitStatus> {
        let status = self
            .0
            .as_mut()
            .context("The article fixture child was already reaped")?
            .wait()?;
        self.0 = None;
        Ok(status)
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for ChildOwner {
    fn drop(&mut self) {
        self.stop();
    }
}

fn output(path: &Path) -> Result<String> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_CAPTURE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() as u64 > MAX_CAPTURE_BYTES;
    bytes.truncate(MAX_CAPTURE_BYTES as usize);
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        text.push_str("\n[child output truncated]");
    }
    Ok(text)
}

fn isolated(test: &'static str, scenario: &'static str) -> Result<()> {
    let module = module_path!()
        .split_once("::")
        .map(|(_, module)| module)
        .context("The article integration test module has no crate prefix")?;
    let case = format!("{module}::{test}");
    if let Some(selected) = std::env::var_os(CHILD_CASE) {
        ensure!(
            selected.as_os_str() == std::ffi::OsStr::new(&case),
            "The child selected another article case"
        );
        #[cfg(unix)]
        {
            let limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            // SAFETY: Only this isolated synthetic child changes its core-file limit.
            if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) } != 0 {
                return Err(std::io::Error::last_os_error()).context("Disable fixture core dumps");
            }
        }
        std::thread::Builder::new()
            .name("weread-article-image-ui".into())
            .spawn(move || app_case(scenario))?
            .join()
            .map_err(|_| anyhow::anyhow!("The article image UI fixture panicked"))??;
        println!("\n{COMPLETED}:{case}");
        return Ok(());
    }
    let directory = tempfile::Builder::new()
        .prefix("weread-article-image-app-")
        .tempdir()?;
    let stdout = directory.path().join("stdout.log");
    let stderr = directory.path().join("stderr.log");
    let child = Command::new(std::env::current_exe()?)
        .args(["--exact", &case, "--nocapture", "--test-threads=1"])
        .env(CHILD_CASE, &case)
        .env_remove("WEREAD_API_KEY")
        .env_remove("WEREAD_LAUNCH_CONTEXT_DIR")
        .env_remove("WEREAD_LAUNCH_TOKEN")
        .env_remove("WEREAD_LAUNCH_GENERATION")
        .env_remove("WEREAD_NATIVE_RETURN")
        .current_dir(directory.path())
        .stdin(Stdio::null())
        .stdout(Stdio::from(File::create(&stdout)?))
        .stderr(Stdio::from(File::create(&stderr)?))
        .spawn()
        .context("Start the isolated article integration child")?;
    let mut owner = ChildOwner(Some(child));
    let deadline = Instant::now() + CHILD_TIMEOUT;
    while owner.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            owner.stop();
            bail!(
                "The article integration child timed out: {case}\n{}\n{}",
                output(&stdout)?,
                output(&stderr)?
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let status = owner.reap()?;
    let stdout = output(&stdout)?;
    let stderr = output(&stderr)?;
    let marker = format!("{COMPLETED}:{case}");
    ensure!(
        status.success()
            && stdout
                .lines()
                .filter(|line| *line == marker.as_str())
                .count()
                == 1,
        "The article integration child failed: {case}\nstatus: {status}\n{stdout}\n{stderr}"
    );
    Ok(())
}

#[test]
fn manual_load_all_continues_after_more_than_two_admission_batches() -> Result<()> {
    isolated(
        "manual_load_all_continues_after_more_than_two_admission_batches",
        "manual",
    )
}

#[test]
fn automatic_images_continue_without_batch_or_idle_reparsing() -> Result<()> {
    isolated(
        "automatic_images_continue_without_batch_or_idle_reparsing",
        "automatic",
    )
}

#[test]
fn released_metadata_capacity_is_observed_by_the_production_tick() -> Result<()> {
    isolated(
        "released_metadata_capacity_is_observed_by_the_production_tick",
        "capacity",
    )
}

#[test]
fn failed_images_require_explicit_retry_and_survive_auto_toggle_without_a_loop() -> Result<()> {
    isolated(
        "failed_images_require_explicit_retry_and_survive_auto_toggle_without_a_loop",
        "failure",
    )
}

#[test]
fn disabling_auto_loading_does_not_requeue_a_late_busy_result() -> Result<()> {
    isolated(
        "disabling_auto_loading_does_not_requeue_a_late_busy_result",
        "cancel-busy",
    )
}

#[test]
fn old_results_cannot_change_another_route_or_a_new_document_with_the_same_id() -> Result<()> {
    isolated(
        "old_results_cannot_change_another_route_or_a_new_document_with_the_same_id",
        "stale-document",
    )
}

#[test]
fn resident_pause_fences_old_image_requests_and_resumes_current_article_work() -> Result<()> {
    isolated(
        "resident_pause_fences_old_image_requests_and_resumes_current_article_work",
        "resident",
    )
}
