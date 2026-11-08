//! Repeated offline use within one process, including actual software rendering.

use super::*;

use std::{
    io::Read,
    path::Path,
    process::{Child, Command, Stdio},
    rc::Weak,
    sync::atomic::{AtomicU8, Ordering},
};

use anyhow::{Context, ensure};
use slint::{
    Model, Rgb8Pixel,
    platform::{
        Platform as SlintPlatform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};

use crate::platform::{DeviceControl, PlatformCapabilities, PlatformError};

const CASE: &str =
    "app::resource_tests::repeated_offline_navigation_and_application_drop_release_resources";
const CHILD_CASE: &str = "WEREAD_RESOURCE_TEST_CHILD";
const COMPLETED: &str = "WEREAD_RESOURCE_TEST_COMPLETED";
const CHILD_TIMEOUT: Duration = Duration::from_secs(120);
const RSS_GROWTH_ALLOWANCE_KIB: usize = 16 * 1024;

/// A test platform must not itself keep every created window alive.
struct ResourceUiPlatform {
    last_window: Rc<RefCell<Weak<MinimalSoftwareWindow>>>,
}

impl SlintPlatform for ResourceUiPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.last_window.replace(Rc::downgrade(&window));
        Ok(window)
    }
}

/// Exercise the real lighting thread without executing any hardware command.
#[derive(Default)]
struct ResourceDevice {
    brightness: AtomicU8,
    warmth: AtomicU8,
}

impl DeviceControl for ResourceDevice {
    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities {
            brightness: true,
            warmth: true,
            ..PlatformCapabilities::default()
        }
    }

    fn brightness(&self) -> Result<u8, PlatformError> {
        Ok(self.brightness.load(Ordering::Acquire))
    }

    fn set_brightness(&self, value: u8) -> Result<(), PlatformError> {
        self.brightness.store(value, Ordering::Release);
        Ok(())
    }

    fn warmth(&self) -> Result<u8, PlatformError> {
        Ok(self.warmth.load(Ordering::Acquire))
    }

    fn set_warmth(&self, value: u8) -> Result<(), PlatformError> {
        self.warmth.store(value, Ordering::Release);
        Ok(())
    }

    fn battery_percentage(&self) -> Result<u8, PlatformError> {
        Err(PlatformError::Unsupported("fixture_battery"))
    }

    fn set_inverted(&self, _: bool) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("fixture_hardware_inversion"))
    }

    fn reset_idle_timer(&self) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("fixture_keep_awake"))
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
struct ResourceSample {
    descriptors: usize,
    threads: usize,
    rss_kib: usize,
}

impl ResourceSample {
    fn read() -> Result<Self> {
        let status = std::fs::read_to_string("/proc/self/status")?;
        let field = |key: &str| -> Result<usize> {
            status
                .lines()
                .find_map(|line| line.strip_prefix(key))
                .and_then(|value| value.split_whitespace().next())
                .with_context(|| format!("The resource fixture is missing {key}"))?
                .parse()
                .with_context(|| format!("The resource fixture cannot parse {key}"))
        };
        let descriptors = std::fs::read_dir("/proc/self/fd")?
            .collect::<std::io::Result<Vec<_>>>()?
            .len();
        Ok(Self {
            descriptors,
            threads: field("Threads:")?,
            rss_kib: field("VmRSS:")?,
        })
    }

    fn emit(self, stage: &str) {
        println!(
            "resource-sample {}",
            serde_json::json!({"stage": stage, "sample": self})
        );
    }

    fn assert_handles_within(self, baseline: Self, stage: &str) {
        assert!(
            self.descriptors <= baseline.descriptors,
            "{stage}: file descriptors grew: {baseline:?} -> {self:?}"
        );
        assert!(
            self.threads <= baseline.threads,
            "{stage}: threads grew: {baseline:?} -> {self:?}"
        );
    }

    fn assert_rss_plateau(self, baseline: Self, stage: &str) {
        // Allocators and font/image caches may keep a bounded high-water mark.
        // Compare equal workloads after warming all routes, not against startup.
        assert!(
            self.rss_kib <= baseline.rss_kib + RSS_GROWTH_ALLOWANCE_KIB,
            "{stage}: resident memory did not plateau after warmup: {baseline:?} -> {self:?}"
        );
    }
}

fn fixture_metrics() -> Result<TextMetrics> {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../assets/fonts/NotoSerifSC-Regular.otf.gz")[..],
    )
    .read_to_end(&mut bytes)?;
    let metrics = TextMetrics::from_font_owned(bytes.clone(), 0).map_err(anyhow::Error::msg)?;
    let registered = slint::fontique_010::shared_collection().register_fonts(
        slint::fontique_010::fontique::Blob::new(Arc::new(bytes)),
        None,
    );
    ensure!(
        registered
            .into_iter()
            .any(|(_, fonts)| fonts.into_iter().next().is_some()),
        "The resource fixture font was not registered"
    );
    Ok(metrics)
}

fn attach_fixture(
    directory: &Path,
    metrics: &TextMetrics,
    runtime: &tokio::runtime::Runtime,
) -> Result<(AppWindow, Rc<RefCell<App>>)> {
    let ui = AppWindow::new()?;
    ui.global::<Theme>().set_ui_font("Noto Serif CJK SC".into());
    ui.global::<Theme>()
        .set_serif_font("Noto Serif CJK SC".into());
    let profile = DeviceProfile::Paperwhite6;
    let platform = Arc::new(Platform::with_control(
        profile,
        false,
        Arc::new(ResourceDevice::default()),
    ));
    crate::platform::configure_window(ui.window(), &platform)?;
    let application = App::attach(
        &ui,
        AppOptions {
            data_dir: directory.to_path_buf(),
            profile,
            demo: true,
            offline: true,
            initial_screen: "library".into(),
            fonts: vec![FontRecord {
                id: "song".into(),
                label: "Resource fixture font".into(),
                family: "Noto Serif CJK SC".into(),
                metrics: metrics.clone(),
                weights: BTreeMap::new(),
                available: true,
            }],
        },
        platform,
        runtime,
    )?;
    ui.show()?;
    Ok((ui, application))
}

fn paint(
    ui: &AppWindow,
    window: &MinimalSoftwareWindow,
    buffer: &mut Vec<Rgb8Pixel>,
) -> Result<()> {
    let size = ui.window().size();
    let white = Rgb8Pixel {
        r: 255,
        g: 255,
        b: 255,
    };
    buffer.resize((size.width * size.height) as usize, white);
    buffer.fill(white);
    slint::platform::update_timers_and_animations();
    window.request_redraw();
    let mut painted = false;
    window.draw_if_needed(|renderer| {
        renderer.render(buffer, size.width as usize);
        painted = true;
    });
    ensure!(painted, "The resource fixture did not paint its frame");
    ensure!(
        buffer.iter().any(|pixel| *pixel != white),
        "The resource fixture painted an empty frame"
    );
    Ok(())
}

fn navigate_cycle(
    ui: &AppWindow,
    application: &Rc<RefCell<App>>,
    window: &MinimalSoftwareWindow,
    buffer: &mut Vec<Rgb8Pixel>,
    cycle: usize,
) -> Result<bool> {
    let mut painted_image = false;
    for route in [
        Route::Detail("demo-0".into()),
        Route::Reader("demo-0".into(), (cycle % 3) as u32),
        Route::Settings,
        Route::Library,
    ] {
        {
            let mut state = application.borrow_mut();
            state.navigate(route, true)?;
            if matches!(state.route, Route::Reader(_, 1)) {
                // Select an image-bearing page explicitly rather than assuming
                // a particular font or device keeps it on the chapter's first page.
                let paginator = state
                    .reader
                    .paginator
                    .as_mut()
                    .context("The image fixture has no paginator")?;
                let image_page = paginator
                    .pages
                    .iter()
                    .position(|page| !page.images.is_empty())
                    .context("The image fixture has no image-bearing page")?;
                paginator.set_page(image_page);
            }
            state.poll_lighting();
            state.render()?;
            ensure!(
                state.force_offline && state.demo,
                "The fixture became online"
            );
            ensure!(!state.jobs.configured(), "The fixture acquired an account");
            ensure!(
                state.history.len() <= 64,
                "Navigation history became unbounded"
            );
            if matches!(state.route, Route::Reader(..)) {
                ensure!(
                    state.reader.paginator.is_some() && !state.reader.body.is_empty(),
                    "The resource fixture did not enter a populated Reader"
                );
                painted_image |= ui
                    .global::<ReaderModel>()
                    .get_images()
                    .iter()
                    .any(|image| image.loaded);
            }
        }
        paint(ui, window, buffer)?;
    }
    Ok(painted_image)
}

fn drop_fixture(ui: AppWindow, application: Rc<RefCell<App>>) -> Result<()> {
    let weak_app = Rc::downgrade(&application);
    application.borrow_mut().shutdown()?;
    drop(application);
    ensure!(
        weak_app.upgrade().is_none(),
        "UI callbacks retained an application after shutdown"
    );
    let weak_ui = ui.as_weak();
    ui.hide()?;
    drop(ui);
    slint::platform::update_timers_and_animations();
    ensure!(weak_ui.upgrade().is_none(), "The closed UI was retained");
    Ok(())
}

fn run_resource_scenario() -> Result<()> {
    let last_window = Rc::new(RefCell::new(Weak::new()));
    slint::platform::set_platform(Box::new(ResourceUiPlatform {
        last_window: last_window.clone(),
    }))?;
    let directory = tempfile::tempdir()?;
    let metrics = fixture_metrics()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (ui, application) = attach_fixture(directory.path(), &metrics, &runtime)?;
    let window = last_window
        .borrow()
        .upgrade()
        .context("The resource fixture has no software window")?;
    let mut buffer = Vec::new();
    let mut samples = Vec::new();
    let mut painted_image = false;
    for (stage, cycles) in [
        ("navigation-warmup", 16),
        ("navigation-first", 32),
        ("navigation-second", 32),
    ] {
        for cycle in 0..cycles {
            painted_image |= navigate_cycle(&ui, &application, &window, &mut buffer, cycle)?;
        }
        let sample = ResourceSample::read()?;
        sample.emit(stage);
        samples.push(sample);
    }
    samples[1].assert_handles_within(samples[0], "navigation-first");
    samples[2].assert_handles_within(samples[0], "navigation-second");
    samples[2].assert_rss_plateau(samples[1], "navigation-second");
    ensure!(painted_image, "No cached image was loaded and painted");
    drop(buffer);
    drop(window);
    drop_fixture(ui, application)?;
    runtime.block_on(tokio::task::yield_now());
    let baseline = ResourceSample::read()?;
    baseline.emit("application-drop-warmup");

    for round in 0..12 {
        let (ui, application) = attach_fixture(directory.path(), &metrics, &runtime)?;
        let window = last_window
            .borrow()
            .upgrade()
            .context("The repeated fixture has no software window")?;
        let mut buffer = Vec::new();
        navigate_cycle(&ui, &application, &window, &mut buffer, round)?;
        drop(buffer);
        drop(window);
        drop_fixture(ui, application)?;
        runtime.block_on(tokio::task::yield_now());
        let sample = ResourceSample::read()?;
        sample.assert_handles_within(baseline, "application-drop");
        if round == 5 || round == 11 {
            sample.emit(if round == 5 {
                "application-drop-first"
            } else {
                "application-drop-second"
            });
        }
        if round == 11 {
            sample.assert_rss_plateau(baseline, "application-drop-second");
        }
    }
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

struct BoundedChild(Child);

impl Drop for BoundedChild {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(Some(_))) {
            return;
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn repeated_offline_navigation_and_application_drop_release_resources() -> Result<()> {
    if let Some(selected) = std::env::var_os(CHILD_CASE) {
        ensure!(
            selected.as_os_str() == std::ffi::OsStr::new(CASE),
            "The resource child selected a different case"
        );
        std::thread::spawn(run_resource_scenario)
            .join()
            .expect("The resource fixture UI thread panicked")?;
        println!("{COMPLETED}:{CASE}");
        return Ok(());
    }

    // Slint keeps process-global event-loop state. Use a fresh process and UI
    // thread, and redirect output to files so a full pipe cannot stall cleanup.
    let output_dir = tempfile::tempdir()?;
    let stdout_path = output_dir.path().join("stdout.log");
    let stderr_path = output_dir.path().join("stderr.log");
    let mut child = BoundedChild(
        Command::new(std::env::current_exe()?)
            .args(["--exact", CASE, "--nocapture", "--test-threads=1"])
            .env(CHILD_CASE, CASE)
            .env_remove("WEREAD_API_KEY")
            .env_remove("WEREAD_LAUNCH_CONTEXT_DIR")
            .env_remove("WEREAD_LAUNCH_GENERATION")
            .env_remove("WEREAD_LAUNCH_TOKEN")
            .env_remove("WEREAD_RESUME_SESSION")
            .env_remove("WEREAD_NATIVE_RETURN")
            .stdout(Stdio::from(std::fs::File::create(&stdout_path)?))
            .stderr(Stdio::from(std::fs::File::create(&stderr_path)?))
            .spawn()?,
    );
    let deadline = Instant::now() + CHILD_TIMEOUT;
    let status = loop {
        if let Some(status) = child.0.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            child.0.kill()?;
            child.0.wait()?;
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = std::fs::read_to_string(stdout_path)?;
    let stderr = std::fs::read_to_string(stderr_path)?;
    let marker = format!("{COMPLETED}:{CASE}");
    ensure!(
        status.is_some_and(|status| status.success())
            && stdout
                .lines()
                .filter(|line| *line == marker.as_str())
                .count()
                == 1,
        "The bounded resource child failed: status={status:?}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    print!("{stdout}");
    Ok(())
}
