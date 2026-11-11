mod native_portrait;
#[cfg(any(all(feature = "kindle", target_os = "linux"), test))]
mod native_theme;
mod profile;
mod refresh;
mod startup;

#[cfg(all(feature = "kindle", target_os = "linux"))]
mod native;
#[cfg(all(target_os = "linux", any(feature = "kindle", test)))]
mod native_command;

pub use native_portrait::{NativePortraitCalibration, NativePortraitDirection};
pub use profile::DeviceProfile;
pub use refresh::{RefreshKind, RefreshPlan, RefreshReason, RefreshRect, RefreshScheduler};
pub use startup::{StartupScreen, StartupStage};

/// Unknown or fixed launcher records leave the existing fixed mapping enabled.
pub fn startup_native_portrait_calibration() -> Option<NativePortraitCalibration> {
    #[cfg(any(all(feature = "kindle", target_os = "linux"), test))]
    {
        native_theme::startup_native_portrait_calibration()
    }
    #[cfg(not(any(all(feature = "kindle", target_os = "linux"), test)))]
    {
        None
    }
}

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("platform capability is unavailable: {0}")]
    Unsupported(&'static str),
    #[error("Kindle launch was not approved by the device/firmware allowlist")]
    UnapprovedDevice,
    #[error(
        "the framebuffer geometry does not match the approved display: expected {expected:?}, actual {actual:?}"
    )]
    GeometryMismatch {
        expected: (u32, u32),
        actual: (u32, u32),
    },
    #[error("the framebuffer offset is unsupported: expected (0, 0), actual {actual:?}")]
    FramebufferOffset { actual: (u32, u32) },
    #[error("invalid hardware value: {0}")]
    InvalidValue(&'static str),
    #[error("device operation failed: {0}")]
    Operation(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlatformCapabilities {
    pub brightness: bool,
    pub warmth: bool,
    pub hardware_inversion: bool,
    /// Application output inversion supplied by the attached renderer.
    pub application_dark_mode: bool,
    pub keep_awake: bool,
    pub pen: bool,
    pub explicit_refresh: bool,
}

/// A request is separate from completed native input and display ownership.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ManagedSleepState {
    #[default]
    Running,
    Suspending,
    Suspended,
    Resuming,
    Terminated,
}

/// Hardware details stay behind a replaceable service boundary.
pub trait DeviceControl: Send + Sync {
    /// Return the approved visible framebuffer dimensions in physical pixels.
    fn native_viewport(&self) -> Option<(u32, u32)> {
        None
    }
    /// A launcher-observed system output mode, never a request to change it.
    fn native_output_inverted(&self) -> Option<bool> {
        None
    }
    /// Permit reviewed framebuffer writes only for an explicitly scoped target.
    fn allows_framebuffer_night_mode(&self) -> bool {
        false
    }
    fn capabilities(&self) -> PlatformCapabilities;
    fn brightness(&self) -> Result<u8, PlatformError>;
    fn set_brightness(&self, value: u8) -> Result<(), PlatformError>;
    fn warmth(&self) -> Result<u8, PlatformError>;
    fn set_warmth(&self, value: u8) -> Result<(), PlatformError>;
    fn battery_percentage(&self) -> Result<u8, PlatformError>;
    fn set_inverted(&self, enabled: bool) -> Result<(), PlatformError>;
    fn reset_idle_timer(&self) -> Result<(), PlatformError>;
    /// Confirm existing device commands are reaped after their producers stop.
    /// Controls without independently tracked commands need no extra work.
    fn quiesce_commands(&self, _: Duration) -> Result<(), PlatformError> {
        Ok(())
    }
}

/// A producer queue acknowledges delivery to the renderer, not panel success.
pub trait RefreshSink: Send + Sync {
    fn submit(&self, plan: RefreshPlan) -> Result<(), PlatformError>;
}

/// Application appearance is owned by the renderer. A Kindle sink can use
/// framebuffer inversion without changing native system preferences.
pub trait RenderSink: Send + Sync {
    fn set_dark_mode(&self, enabled: bool) -> Result<(), PlatformError>;

    /// A renderer that reads native framebuffer metadata on every real resume
    /// does not require the launcher's optional LIPC appearance snapshot.
    fn observes_native_output_on_resume(&self) -> bool {
        false
    }

    fn native_output_inverted(&self) -> Option<bool> {
        None
    }

    fn set_native_output_inverted(&self, _: bool) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("native_output_inversion"))
    }

    fn set_native_portrait_calibration(
        &self,
        _: NativePortraitCalibration,
    ) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("native_portrait_calibration"))
    }

    /// Optional current-cycle preparation before the renderer reacquires input.
    fn prepare_native_portrait_resume(
        &self,
        _: NativePortraitCalibration,
    ) -> Result<(), PlatformError> {
        Ok(())
    }

    fn managed_sleep_supported(&self) -> bool {
        false
    }

    fn set_managed_sleep(&self, _: bool) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("managed_sleep"))
    }

    fn managed_sleep_state(&self) -> Result<ManagedSleepState, PlatformError> {
        Err(PlatformError::Unsupported("managed_sleep"))
    }

    /// Request a complete frame after the application has installed its sleep UI.
    fn request_sleep_presentation(&self) -> Result<u64, PlatformError> {
        Err(PlatformError::Unsupported("sleep_presentation"))
    }

    /// Only actual completed presentation may satisfy the returned token.
    fn sleep_presentation_completed(&self, _: u64) -> Result<bool, PlatformError> {
        Err(PlatformError::Unsupported("sleep_presentation"))
    }
}

/// Canonical UI colors stay unchanged. Only the process-local output transform
/// compensates for the native EPDC mode observed by the trusted supervisor.
#[cfg(test)]
fn software_output_inverted(desired_dark: bool, native_inverse: Option<bool>) -> bool {
    desired_dark ^ native_inverse.unwrap_or(false)
}

#[cfg(test)]
struct OutputCompensation {
    desired_dark: std::sync::atomic::AtomicBool,
    native_inverse: std::sync::atomic::AtomicBool,
    native_known: std::sync::atomic::AtomicBool,
    apply: Mutex<()>,
}

#[cfg(test)]
impl OutputCompensation {
    fn new(native_inverse: Option<bool>) -> Self {
        Self {
            desired_dark: std::sync::atomic::AtomicBool::new(false),
            native_inverse: std::sync::atomic::AtomicBool::new(native_inverse.unwrap_or(false)),
            native_known: std::sync::atomic::AtomicBool::new(native_inverse.is_some()),
            apply: Mutex::new(()),
        }
    }

    fn native_output_inverted(&self) -> Option<bool> {
        use std::sync::atomic::Ordering;
        self.native_known
            .load(Ordering::Acquire)
            .then(|| self.native_inverse.load(Ordering::Acquire))
    }

    fn set_desired(
        &self,
        desired: bool,
        apply: impl FnOnce(bool) -> Result<(), PlatformError>,
    ) -> Result<(), PlatformError> {
        use std::sync::atomic::Ordering;
        let _guard = self.apply.lock().map_err(|_| {
            PlatformError::Operation("output compensation lock was poisoned".into())
        })?;
        apply(software_output_inverted(
            desired,
            self.native_output_inverted(),
        ))?;
        self.desired_dark.store(desired, Ordering::Release);
        Ok(())
    }

    fn set_native(
        &self,
        native: bool,
        apply: impl FnOnce(bool) -> Result<(), PlatformError>,
    ) -> Result<(), PlatformError> {
        use std::sync::atomic::Ordering;
        let _guard = self.apply.lock().map_err(|_| {
            PlatformError::Operation("output compensation lock was poisoned".into())
        })?;
        apply(software_output_inverted(
            self.desired_dark.load(Ordering::Acquire),
            Some(native),
        ))?;
        self.native_inverse.store(native, Ordering::Release);
        self.native_known.store(true, Ordering::Release);
        Ok(())
    }
}

struct DesktopControl;

impl DeviceControl for DesktopControl {
    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::default()
    }
    fn brightness(&self) -> Result<u8, PlatformError> {
        Err(PlatformError::Unsupported("brightness"))
    }
    fn set_brightness(&self, _: u8) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("brightness"))
    }
    fn warmth(&self) -> Result<u8, PlatformError> {
        Err(PlatformError::Unsupported("warmth"))
    }
    fn set_warmth(&self, _: u8) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("warmth"))
    }
    fn battery_percentage(&self) -> Result<u8, PlatformError> {
        Err(PlatformError::Unsupported("battery"))
    }
    fn set_inverted(&self, _: bool) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("hardware_inversion"))
    }
    fn reset_idle_timer(&self) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("keep_awake"))
    }
}

pub struct Platform {
    pub profile: DeviceProfile,
    pub is_kindle: bool,
    control: Arc<dyn DeviceControl>,
    refresh: Mutex<RefreshScheduler>,
    refresh_sink: Mutex<Option<Arc<dyn RefreshSink>>>,
    render_sink: Mutex<Option<Arc<dyn RenderSink>>>,
    started: Instant,
}

impl Platform {
    pub fn new(profile: DeviceProfile, is_kindle: bool) -> Result<Self, PlatformError> {
        let control: Arc<dyn DeviceControl> = if is_kindle {
            #[cfg(all(feature = "kindle", target_os = "linux"))]
            {
                Arc::new(native::NativeControl::new(profile)?)
            }
            #[cfg(not(all(feature = "kindle", target_os = "linux")))]
            {
                return Err(PlatformError::Unsupported("kindle_backend"));
            }
        } else {
            Arc::new(DesktopControl)
        };
        Ok(Self::with_control(profile, is_kindle, control))
    }

    pub fn with_control(
        profile: DeviceProfile,
        is_kindle: bool,
        control: Arc<dyn DeviceControl>,
    ) -> Self {
        Self {
            profile,
            is_kindle,
            control,
            refresh: Mutex::new(RefreshScheduler::default()),
            refresh_sink: Mutex::new(None),
            render_sink: Mutex::new(None),
            started: Instant::now(),
        }
    }

    pub fn capabilities(&self) -> PlatformCapabilities {
        let mut capabilities = self.control.capabilities();
        capabilities.explicit_refresh = self.refresh_sink.lock().is_ok_and(|sink| sink.is_some());
        capabilities.application_dark_mode =
            self.render_sink.lock().is_ok_and(|sink| sink.is_some());
        capabilities
    }

    pub fn native_viewport_size(&self) -> Option<(u32, u32)> {
        if self.is_kindle {
            self.control.native_viewport()
        } else {
            None
        }
    }

    pub fn logical_viewport_size(&self) -> (u32, u32) {
        self.native_viewport_size()
            .map(|(width, height)| (width / 2, height / 2))
            .unwrap_or_else(|| self.profile.logical_size())
    }

    pub fn scale_factor(&self) -> f32 {
        if self.is_kindle { 2.0 } else { 1.0 }
    }

    pub fn brightness(&self) -> Result<u8, PlatformError> {
        self.control.brightness()
    }
    pub fn set_brightness(&self, value: u8) -> Result<(), PlatformError> {
        self.control.set_brightness(value)
    }
    pub fn warmth(&self) -> Result<u8, PlatformError> {
        self.control.warmth()
    }
    pub fn set_warmth(&self, value: u8) -> Result<(), PlatformError> {
        self.control.set_warmth(value)
    }
    pub fn battery_percentage(&self) -> Result<u8, PlatformError> {
        let value = self.control.battery_percentage()?;
        if value > 100 {
            return Err(PlatformError::InvalidValue(
                "battery percentage must be between 0 and 100",
            ));
        }
        Ok(value)
    }

    /// Producer joins and command retirement are separate ownership evidence.
    pub fn quiesce_device_commands(&self, timeout: Duration) -> Result<(), PlatformError> {
        self.control.quiesce_commands(timeout)
    }
    /// Request hardware inversion only when a reviewed device service provides it.
    pub fn set_inverted(&self, enabled: bool) -> Result<(), PlatformError> {
        self.control.set_inverted(enabled)
    }

    pub fn set_render_sink(&self, sink: Arc<dyn RenderSink>) -> Result<(), PlatformError> {
        *self
            .render_sink
            .lock()
            .map_err(|_| PlatformError::Operation("render sink lock was poisoned".into()))? =
            Some(sink);
        Ok(())
    }

    /// Request application appearance without writing native system preferences.
    pub fn set_dark_mode(&self, enabled: bool) -> Result<(), PlatformError> {
        let sink = self
            .render_sink
            .lock()
            .map_err(|_| PlatformError::Operation("render sink lock was poisoned".into()))?
            .clone()
            .ok_or(PlatformError::Unsupported("application_dark_mode"))?;
        sink.set_dark_mode(enabled)
    }

    pub fn allows_framebuffer_night_mode(&self) -> bool {
        self.control.allows_framebuffer_night_mode()
    }

    pub fn observes_native_output_on_resume(&self) -> bool {
        self.render_sink
            .lock()
            .ok()
            .and_then(|sink| sink.clone())
            .is_some_and(|sink| sink.observes_native_output_on_resume())
    }

    /// The renderer retains the newest wake snapshot; early startup uses the
    /// native controller's verified launcher snapshot before a sink exists.
    pub fn native_output_inverted(&self) -> Option<bool> {
        let sink = self.render_sink.lock().ok().and_then(|sink| sink.clone());
        sink.and_then(|sink| sink.native_output_inverted())
            .or_else(|| self.control.native_output_inverted())
    }

    /// Update compensation from an authenticated observation, not a system write.
    pub fn set_native_output_inverted(&self, inverted: bool) -> Result<(), PlatformError> {
        let sink = self
            .render_sink
            .lock()
            .map_err(|_| PlatformError::Operation("render sink lock was poisoned".into()))?
            .clone()
            .ok_or(PlatformError::Unsupported("native_output_inversion"))?;
        sink.set_native_output_inverted(inverted)
    }

    /// Configure the observer from a protected same-launch native scanout pair.
    pub fn set_native_portrait_calibration(
        &self,
        calibration: NativePortraitCalibration,
    ) -> Result<(), PlatformError> {
        let sink = self
            .render_sink
            .lock()
            .map_err(|_| PlatformError::Operation("render sink lock was poisoned".into()))?
            .clone()
            .ok_or(PlatformError::Unsupported("native_portrait_calibration"))?;
        sink.set_native_portrait_calibration(calibration)
    }

    /// A current-cycle record authorizes a fresh metadata observation, without
    /// replacing the retained initial scanout anchor or the user's app rotation.
    pub fn prepare_native_portrait_resume(
        &self,
        observation: NativePortraitCalibration,
    ) -> Result<(), PlatformError> {
        let sink = self
            .render_sink
            .lock()
            .map_err(|_| PlatformError::Operation("render sink lock was poisoned".into()))?
            .clone();
        match sink {
            Some(sink) => sink.prepare_native_portrait_resume(observation),
            None => Ok(()),
        }
    }

    pub fn managed_sleep_supported(&self) -> bool {
        let sink = self.render_sink.lock().ok().and_then(|sink| sink.clone());
        sink.is_some_and(|sink| sink.managed_sleep_supported())
    }

    /// Ask the renderer to hand off or restore ownership; poll its state for ACK.
    pub fn set_managed_sleep(&self, sleeping: bool) -> Result<(), PlatformError> {
        let sink = self
            .render_sink
            .lock()
            .map_err(|_| PlatformError::Operation("render sink lock was poisoned".into()))?
            .clone()
            .ok_or(PlatformError::Unsupported("managed_sleep"))?;
        sink.set_managed_sleep(sleeping)
    }

    pub fn managed_sleep_state(&self) -> Result<ManagedSleepState, PlatformError> {
        let sink = self
            .render_sink
            .lock()
            .map_err(|_| PlatformError::Operation("render sink lock was poisoned".into()))?
            .clone()
            .ok_or(PlatformError::Unsupported("managed_sleep"))?;
        sink.managed_sleep_state()
    }

    pub fn request_sleep_presentation(&self) -> Result<u64, PlatformError> {
        let sink = self
            .render_sink
            .lock()
            .map_err(|_| PlatformError::Operation("render sink lock was poisoned".into()))?
            .clone()
            .ok_or(PlatformError::Unsupported("sleep_presentation"))?;
        sink.request_sleep_presentation()
    }

    pub fn sleep_presentation_completed(&self, token: u64) -> Result<bool, PlatformError> {
        let sink = self
            .render_sink
            .lock()
            .map_err(|_| PlatformError::Operation("render sink lock was poisoned".into()))?
            .clone()
            .ok_or(PlatformError::Unsupported("sleep_presentation"))?;
        sink.sleep_presentation_completed(token)
    }

    pub fn plan_refresh(&self, reason: RefreshReason) -> Option<RefreshPlan> {
        self.refresh
            .lock()
            .ok()?
            .plan(reason, self.started.elapsed())
    }

    pub fn set_refresh_sink(&self, sink: Arc<dyn RefreshSink>) -> Result<(), PlatformError> {
        *self
            .refresh_sink
            .lock()
            .map_err(|_| PlatformError::Operation("refresh sink lock was poisoned".into()))? =
            Some(sink);
        Ok(())
    }

    pub fn submit_refresh(&self, plan: RefreshPlan) -> Result<(), PlatformError> {
        let sink = self
            .refresh_sink
            .lock()
            .map_err(|_| PlatformError::Operation("refresh sink lock was poisoned".into()))?
            .clone();
        match sink {
            Some(sink) => sink.submit(plan),
            None if !self.is_kindle => Ok(()),
            None => Err(PlatformError::Unsupported("explicit_refresh")),
        }
    }

    pub fn request_refresh(
        &self,
        reason: RefreshReason,
    ) -> Result<Option<RefreshPlan>, PlatformError> {
        let plan = self.plan_refresh(reason);
        if let Some(plan) = plan {
            self.submit_refresh(plan)?;
        }
        Ok(plan)
    }

    pub fn set_reader_refresh_interval(&self, pages: u32) -> Result<(), PlatformError> {
        self.refresh
            .lock()
            .map_err(|_| PlatformError::Operation("refresh scheduler lock was poisoned".into()))?
            .set_reader_interval(pages)
            .map_err(PlatformError::InvalidValue)
    }

    pub fn set_flash_on_navigation(&self, enabled: bool) {
        if let Ok(mut scheduler) = self.refresh.lock() {
            scheduler.set_flash_on_navigation(enabled);
        }
    }

    /// Keep a task active without changing persistent power settings.
    ///
    /// The initial reset must succeed before a guard is returned. Subsequent
    /// failures are retained and can be surfaced by the application.
    pub fn hold_awake(&self) -> Result<WakeGuard, PlatformError> {
        self.control.reset_idle_timer()?;
        spawn_wake_guard(self.control.clone(), false, true)
    }

    /// Start task wake maintenance without waiting for any device command.
    ///
    /// The initial reset runs on the worker. Inspect `WakeGuard::failure()` for
    /// an initial or later device error. Dropping this guard requests stop and
    /// detaches; an in-flight bounded command may finish after the drop. Call
    /// `shutdown` before acknowledging a native power handoff.
    pub fn hold_awake_deferred(&self) -> Result<WakeGuard, PlatformError> {
        spawn_wake_guard(self.control.clone(), true, false)
    }
}

fn spawn_wake_guard(
    control: Arc<dyn DeviceControl>,
    reset_on_start: bool,
    join_on_drop: bool,
) -> Result<WakeGuard, PlatformError> {
    let (stop, receiver) = std::sync::mpsc::channel();
    let error = Arc::new(Mutex::new(None));
    let worker_error = error.clone();
    let worker = std::thread::Builder::new()
        .name("weread-wake-guard".into())
        .spawn(move || {
            run_wake_worker(control, receiver, worker_error, reset_on_start);
        })?;
    Ok(WakeGuard {
        stop: Some(stop),
        worker: Some(worker),
        error,
        join_on_drop,
        shutdown_error: None,
    })
}

fn run_wake_worker(
    control: Arc<dyn DeviceControl>,
    receiver: std::sync::mpsc::Receiver<()>,
    error: Arc<Mutex<Option<String>>>,
    reset_on_start: bool,
) {
    let reset = || {
        if let Err(failure) = control.reset_idle_timer() {
            if let Ok(mut slot) = error.lock() {
                *slot = Some(failure.to_string());
            }
            false
        } else {
            true
        }
    };
    if reset_on_start {
        match receiver.try_recv() {
            Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        if !reset() {
            return;
        }
    }
    loop {
        match receiver.recv_timeout(Duration::from_secs(60)) {
            Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if !reset() {
                    return;
                }
            }
        }
    }
}

pub struct WakeGuard {
    stop: Option<std::sync::mpsc::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
    join_on_drop: bool,
    shutdown_error: Option<&'static str>,
}

impl WakeGuard {
    pub fn failure(&self) -> Option<String> {
        self.error.lock().ok()?.clone()
    }

    /// Stop and join maintenance before native power ownership is acknowledged.
    /// A timeout retains the handle so a later call can confirm termination.
    pub fn shutdown(&mut self, timeout: Duration) -> Result<(), PlatformError> {
        if let Some(error) = self.shutdown_error {
            return Err(PlatformError::Operation(error.into()));
        }
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let started = Instant::now();
        while let Some(worker) = self.worker.as_ref() {
            if worker.is_finished() {
                if self.worker.take().unwrap().join().is_err() {
                    let error = "the wake-maintenance worker panicked during shutdown";
                    self.shutdown_error = Some(error);
                    if let Ok(mut failure) = self.error.lock() {
                        *failure = Some(error.into());
                    }
                    return Err(PlatformError::Operation(error.into()));
                }
                return Ok(());
            }
            let elapsed = started.elapsed();
            if elapsed >= timeout {
                return Err(PlatformError::Operation(
                    "the wake-maintenance worker did not stop before its deadline".into(),
                ));
            }
            std::thread::sleep((timeout - elapsed).min(Duration::from_millis(10)));
        }
        Ok(())
    }
}

impl Drop for WakeGuard {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            if self.join_on_drop {
                let _ = worker.join();
            }
        }
    }
}

/// Call after creating the window, before starting its event loop.
pub fn configure_window(
    window: &slint::Window,
    platform: &Platform,
) -> Result<(), slint::PlatformError> {
    window.try_dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
        scale_factor: platform.scale_factor(),
    })?;
    let (width, height) = platform.logical_viewport_size();
    let size = slint::LogicalSize::new(width as f32, height as f32);
    if platform.is_kindle {
        let (width, height) = platform.native_viewport_size().ok_or_else(|| {
            slint::PlatformError::Other("The native display geometry was not approved".into())
        })?;
        window.set_size(slint::PhysicalSize::new(width, height));
    } else {
        window.set_size(size);
    }
    window.try_dispatch_event(slint::platform::WindowEvent::Resized { size })?;
    Ok(())
}

/// Install before creating Tokio runtimes, with any startup display worker joined.
#[cfg(all(feature = "kindle", target_os = "linux"))]
pub fn install_kindle_backend(
    font_bytes: &[u8],
) -> Result<slint_backend_kindle::KindleBackend, slint::PlatformError> {
    slint_backend_kindle::install(font_bytes)
}

#[cfg(all(feature = "kindle", target_os = "linux"))]
pub struct KindleRefreshSink(slint_backend_kindle::RefreshHandle);

#[cfg(all(feature = "kindle", target_os = "linux"))]
pub struct KindleRenderSink {
    handle: slint_backend_kindle::RenderHandle,
    native_observation: Mutex<Option<bool>>,
    orientation: Option<slint_backend_kindle::NativeOrientationHandle>,
    portrait_anchor: Mutex<Option<NativePortraitCalibration>>,
}

#[cfg(all(feature = "kindle", target_os = "linux"))]
impl KindleRenderSink {
    pub fn new(handle: slint_backend_kindle::RenderHandle) -> Self {
        Self {
            handle,
            native_observation: Mutex::new(None),
            orientation: None,
            portrait_anchor: Mutex::new(None),
        }
    }

    /// The desired application appearance is set by App initialization before
    /// the backend event loop fills or draws its first framebuffer.
    pub fn new_with_native_inversion(
        handle: slint_backend_kindle::RenderHandle,
        native_inverse: bool,
    ) -> Self {
        Self {
            handle,
            native_observation: Mutex::new(Some(native_inverse)),
            orientation: None,
            portrait_anchor: Mutex::new(None),
        }
    }

    pub fn with_native_orientation(
        mut self,
        handle: slint_backend_kindle::NativeOrientationHandle,
    ) -> Self {
        self.orientation = Some(handle);
        self
    }
}

#[cfg(all(feature = "kindle", target_os = "linux"))]
impl RenderSink for KindleRenderSink {
    fn request_sleep_presentation(&self) -> Result<u64, PlatformError> {
        self.handle
            .request_sleep_presentation()
            .map_err(|error| PlatformError::Operation(error.to_string()))
    }

    fn sleep_presentation_completed(&self, token: u64) -> Result<bool, PlatformError> {
        self.handle
            .sleep_presentation_completed(token)
            .map_err(|error| PlatformError::Operation(error.to_string()))
    }

    fn observes_native_output_on_resume(&self) -> bool {
        true
    }

    fn set_dark_mode(&self, enabled: bool) -> Result<(), PlatformError> {
        self.handle
            .set_night_mode(enabled)
            .map_err(|error| PlatformError::Operation(error.to_string()))
    }

    fn native_output_inverted(&self) -> Option<bool> {
        self.handle
            .native_output_inverted()
            .or_else(|| self.native_observation.lock().ok().and_then(|value| *value))
    }

    fn set_native_output_inverted(&self, inverted: bool) -> Result<(), PlatformError> {
        // The backend observes the actual framebuffer on resume. This trusted
        // launcher observation is an early baseline, never another night request.
        *self.native_observation.lock().map_err(|_| {
            PlatformError::Operation("native observation lock was poisoned".into())
        })? = Some(inverted);
        Ok(())
    }

    fn set_native_portrait_calibration(
        &self,
        calibration: NativePortraitCalibration,
    ) -> Result<(), PlatformError> {
        use slint_backend_kindle::NativePortraitOrientation;
        let handle = self
            .orientation
            .as_ref()
            .ok_or(PlatformError::Unsupported("native_portrait_calibration"))?;
        let mut anchor = self
            .portrait_anchor
            .lock()
            .map_err(|_| PlatformError::Operation("portrait anchor lock was poisoned".into()))?;
        if anchor
            .as_ref()
            .is_some_and(|initial| *initial != calibration)
        {
            return Err(PlatformError::InvalidValue(
                "the initial native portrait calibration is immutable",
            ));
        }
        let direction = match calibration.direction {
            NativePortraitDirection::Up => NativePortraitOrientation::Up,
            NativePortraitDirection::Down => NativePortraitOrientation::Down,
        };
        handle
            .set_calibrated_presentation(direction, direction, calibration.framebuffer_rotation)
            .map_err(|error| PlatformError::Operation(error.to_string()))?;
        // This immutable initial pair remains authoritative when native winmgr
        // still exposes an earlier direction during a later wake.
        if anchor.is_none() {
            *anchor = Some(calibration);
        }
        Ok(())
    }

    fn prepare_native_portrait_resume(
        &self,
        observation: NativePortraitCalibration,
    ) -> Result<(), PlatformError> {
        use slint_backend_kindle::NativePortraitOrientation;
        let Some(handle) = self.orientation.as_ref() else {
            return Ok(());
        };
        let anchor = *self
            .portrait_anchor
            .lock()
            .map_err(|_| PlatformError::Operation("portrait anchor lock was poisoned".into()))?;
        let Some(anchor) = anchor else { return Ok(()) };
        let Ok((true, raw_rotation)) = slint_backend_kindle::probe_native_portrait_framebuffer()
        else {
            return Ok(());
        };
        if raw_rotation != observation.framebuffer_rotation {
            return Ok(());
        }
        let Some(scanout) = anchor.scanout_at(raw_rotation) else {
            return Ok(());
        };
        let Ok(Some(holding)) = slint_backend_kindle::probe_native_portrait_pose() else {
            return Ok(());
        };
        let Ok((true, confirmed_rotation)) =
            slint_backend_kindle::probe_native_portrait_framebuffer()
        else {
            return Ok(());
        };
        if confirmed_rotation != raw_rotation {
            return Ok(());
        }
        let scanout = match scanout {
            NativePortraitDirection::Up => NativePortraitOrientation::Up,
            NativePortraitDirection::Down => NativePortraitOrientation::Down,
        };
        // The protected observation supplies scope and freshness; its winmgr
        // direction does not redefine the initial scanout/GETVAR relationship.
        handle
            .set_calibrated_presentation(holding, scanout, raw_rotation)
            .map_err(|error| PlatformError::Operation(error.to_string()))
    }

    fn managed_sleep_supported(&self) -> bool {
        true
    }

    fn set_managed_sleep(&self, sleeping: bool) -> Result<(), PlatformError> {
        self.handle
            .set_managed_sleep(sleeping)
            .map_err(|error| PlatformError::Operation(error.to_string()))
    }

    fn managed_sleep_state(&self) -> Result<ManagedSleepState, PlatformError> {
        use slint_backend_kindle::ManagedSleepState as NativeState;
        Ok(match self.handle.managed_sleep_state() {
            NativeState::Running => ManagedSleepState::Running,
            NativeState::Suspending => ManagedSleepState::Suspending,
            NativeState::Suspended => ManagedSleepState::Suspended,
            NativeState::Resuming => ManagedSleepState::Resuming,
            NativeState::Terminated => ManagedSleepState::Terminated,
        })
    }
}

#[cfg(all(feature = "kindle", target_os = "linux"))]
impl KindleRefreshSink {
    pub fn new(handle: slint_backend_kindle::RefreshHandle) -> Self {
        Self(handle)
    }
}

#[cfg(all(feature = "kindle", target_os = "linux"))]
impl RefreshSink for KindleRefreshSink {
    fn submit(&self, plan: RefreshPlan) -> Result<(), PlatformError> {
        use slint_backend_kindle::{PhysicalRect, RefreshArea, RefreshRequest, RefreshWaveform};
        let (waveform, flashing) = match plan.kind {
            RefreshKind::Full | RefreshKind::FlashUi | RefreshKind::FlashPartial => {
                (RefreshWaveform::Gc16, true)
            }
            RefreshKind::Fast => (RefreshWaveform::Du, false),
            RefreshKind::Ui | RefreshKind::Partial => (RefreshWaveform::Auto, false),
        };
        let area = if let Some(rect) = plan.rect {
            RefreshArea::Rect(PhysicalRect {
                x: i32::try_from(rect.x.saturating_mul(2)).unwrap_or(i32::MAX),
                y: i32::try_from(rect.y.saturating_mul(2)).unwrap_or(i32::MAX),
                width: rect.width.saturating_mul(2),
                height: rect.height.saturating_mul(2),
            })
        } else if matches!(plan.kind, RefreshKind::Full | RefreshKind::FlashUi) {
            RefreshArea::Full
        } else {
            RefreshArea::Dirty
        };
        self.0
            .request(RefreshRequest {
                waveform,
                flashing,
                area,
            })
            .map_err(|error| PlatformError::Operation(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Condvar;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct ResetGate {
        state: Mutex<(bool, bool)>,
        changed: Condvar,
    }

    impl ResetGate {
        fn block_reset(&self) {
            let mut state = self.state.lock().unwrap();
            state.0 = true;
            self.changed.notify_all();
            while !state.1 {
                state = self.changed.wait(state).unwrap();
            }
        }

        fn wait_until_entered(&self) -> bool {
            let (state, _) = self
                .changed
                .wait_timeout_while(
                    self.state.lock().unwrap(),
                    Duration::from_secs(2),
                    |state| !state.0,
                )
                .unwrap();
            state.0
        }

        fn release(&self) {
            self.state.lock().unwrap().1 = true;
            self.changed.notify_all();
        }
    }

    #[derive(Default)]
    struct FakeControl {
        resets: AtomicUsize,
        gate: Option<Arc<ResetGate>>,
        fail_reset: bool,
        battery: u8,
        native_viewport: Option<(u32, u32)>,
        native_inverse: Option<bool>,
    }

    impl DeviceControl for FakeControl {
        fn native_viewport(&self) -> Option<(u32, u32)> {
            self.native_viewport
        }
        fn native_output_inverted(&self) -> Option<bool> {
            self.native_inverse
        }

        fn capabilities(&self) -> PlatformCapabilities {
            PlatformCapabilities {
                keep_awake: true,
                ..Default::default()
            }
        }
        fn brightness(&self) -> Result<u8, PlatformError> {
            Err(PlatformError::Unsupported("brightness"))
        }
        fn set_brightness(&self, _: u8) -> Result<(), PlatformError> {
            Err(PlatformError::Unsupported("brightness"))
        }
        fn warmth(&self) -> Result<u8, PlatformError> {
            Err(PlatformError::Unsupported("warmth"))
        }
        fn set_warmth(&self, _: u8) -> Result<(), PlatformError> {
            Err(PlatformError::Unsupported("warmth"))
        }
        fn battery_percentage(&self) -> Result<u8, PlatformError> {
            Ok(self.battery)
        }
        fn set_inverted(&self, _: bool) -> Result<(), PlatformError> {
            Err(PlatformError::Unsupported("hardware_inversion"))
        }
        fn reset_idle_timer(&self) -> Result<(), PlatformError> {
            self.resets.fetch_add(1, Ordering::SeqCst);
            if let Some(gate) = &self.gate {
                gate.block_reset();
            }
            if self.fail_reset {
                Err(PlatformError::Operation("fixture reset failed".into()))
            } else {
                Ok(())
            }
        }
    }

    #[derive(Default)]
    struct RecordingRenderSink {
        transitions: Mutex<Vec<bool>>,
        fail: bool,
    }

    impl RenderSink for RecordingRenderSink {
        fn set_dark_mode(&self, enabled: bool) -> Result<(), PlatformError> {
            if self.fail {
                return Err(PlatformError::Operation("fixture renderer stopped".into()));
            }
            self.transitions.lock().unwrap().push(enabled);
            Ok(())
        }
    }

    struct CompensatingRenderSink {
        output: OutputCompensation,
        writes: Mutex<Vec<bool>>,
    }

    impl CompensatingRenderSink {
        fn new(native_inverse: Option<bool>) -> Self {
            Self {
                output: OutputCompensation::new(native_inverse),
                writes: Mutex::new(Vec::new()),
            }
        }
    }

    impl RenderSink for CompensatingRenderSink {
        fn set_dark_mode(&self, desired: bool) -> Result<(), PlatformError> {
            self.output.set_desired(desired, |software| {
                self.writes.lock().unwrap().push(software);
                Ok(())
            })
        }
        fn native_output_inverted(&self) -> Option<bool> {
            self.output.native_output_inverted()
        }
        fn set_native_output_inverted(&self, native: bool) -> Result<(), PlatformError> {
            self.output.set_native(native, |software| {
                self.writes.lock().unwrap().push(software);
                Ok(())
            })
        }
    }

    #[test]
    fn software_sink_compensation_preserves_the_target_in_both_native_modes() {
        for native in [None, Some(false), Some(true)] {
            let sink = Arc::new(CompensatingRenderSink::new(native));
            for desired in [false, true] {
                sink.set_dark_mode(desired).unwrap();
                let software = *sink.writes.lock().unwrap().last().unwrap();
                assert_eq!(software, desired ^ native.unwrap_or(false));
                // A software-only sink preserves physical appearance too.
                let raw_paper = if software { 0_u8 } else { 255_u8 };
                let physical_paper = if native == Some(true) {
                    255 - raw_paper
                } else {
                    raw_paper
                };
                assert_eq!(physical_paper, if desired { 0 } else { 255 });
            }
            assert_eq!(sink.native_output_inverted(), native);
        }
    }

    #[test]
    fn a_new_native_base_reapplies_the_existing_desired_mode_without_changing_it() {
        let sink = CompensatingRenderSink::new(None);
        sink.set_dark_mode(true).unwrap();
        assert_eq!(sink.native_output_inverted(), None);
        sink.set_native_output_inverted(true).unwrap();
        assert_eq!(sink.native_output_inverted(), Some(true));
        assert!(sink.output.desired_dark.load(Ordering::Acquire));
        sink.set_dark_mode(false).unwrap();
        sink.set_native_output_inverted(false).unwrap();
        assert!(!sink.output.desired_dark.load(Ordering::Acquire));
        assert_eq!(&*sink.writes.lock().unwrap(), &[true, false, true, false]);
    }

    #[test]
    fn failed_render_updates_do_not_commit_a_new_desired_mode_or_native_base() {
        let output = OutputCompensation::new(None);
        output.set_desired(true, |_| Ok(())).unwrap();
        assert!(
            output
                .set_native(true, |_| Err(PlatformError::Operation(
                    "fixture output stopped".into()
                )))
                .is_err()
        );
        assert_eq!(output.native_output_inverted(), None);
        assert!(
            output
                .set_desired(false, |_| Err(PlatformError::Operation(
                    "fixture output stopped".into()
                )))
                .is_err()
        );
        assert!(output.desired_dark.load(Ordering::Acquire));
        let mut applied = None;
        output
            .set_native(true, |software| {
                applied = Some(software);
                Ok(())
            })
            .unwrap();
        assert_eq!(applied, Some(false));
        assert_eq!(output.native_output_inverted(), Some(true));
    }

    #[test]
    fn the_latest_renderer_base_overrides_the_initial_native_snapshot() {
        let platform = Platform::with_control(
            DeviceProfile::Scribe,
            true,
            Arc::new(FakeControl {
                native_inverse: Some(true),
                ..Default::default()
            }),
        );
        assert_eq!(platform.native_output_inverted(), Some(true));
        assert!(matches!(
            platform.set_native_output_inverted(false),
            Err(PlatformError::Unsupported("native_output_inversion"))
        ));
        let sink = Arc::new(CompensatingRenderSink::new(Some(true)));
        platform.set_render_sink(sink.clone()).unwrap();
        platform.set_dark_mode(true).unwrap();
        platform.set_native_output_inverted(false).unwrap();
        assert_eq!(platform.native_output_inverted(), Some(false));
        assert_eq!(&*sink.writes.lock().unwrap(), &[false, true]);
        assert!(sink.output.desired_dark.load(Ordering::Acquire));
    }

    struct ManagedRenderSink(Mutex<ManagedSleepState>);

    struct PresentationRenderSink(Mutex<(u64, u64)>);

    impl RenderSink for PresentationRenderSink {
        fn set_dark_mode(&self, _: bool) -> Result<(), PlatformError> {
            Ok(())
        }

        fn request_sleep_presentation(&self) -> Result<u64, PlatformError> {
            let mut state = self.0.lock().unwrap();
            state.0 += 1;
            Ok(state.0)
        }

        fn sleep_presentation_completed(&self, token: u64) -> Result<bool, PlatformError> {
            let state = self.0.lock().unwrap();
            if token == 0 || token > state.0 {
                return Err(PlatformError::InvalidValue("unknown presentation token"));
            }
            Ok(token <= state.1)
        }
    }

    #[test]
    fn sleep_presentation_requires_an_explicit_sink_and_forwards_independent_tokens() {
        let platform = Platform::new(DeviceProfile::Scribe, false).unwrap();
        assert!(matches!(
            platform.request_sleep_presentation(),
            Err(PlatformError::Unsupported("sleep_presentation"))
        ));
        assert!(matches!(
            platform.sleep_presentation_completed(1),
            Err(PlatformError::Unsupported("sleep_presentation"))
        ));
        platform
            .set_render_sink(Arc::new(RecordingRenderSink::default()))
            .unwrap();
        assert!(matches!(
            platform.request_sleep_presentation(),
            Err(PlatformError::Unsupported("sleep_presentation"))
        ));
        let sink = Arc::new(PresentationRenderSink(Mutex::new((0, 0))));
        platform.set_render_sink(sink.clone()).unwrap();
        let first = platform.request_sleep_presentation().unwrap();
        assert!(!platform.sleep_presentation_completed(first).unwrap());
        sink.0.lock().unwrap().1 = first;
        let second = platform.request_sleep_presentation().unwrap();
        assert!(platform.sleep_presentation_completed(first).unwrap());
        assert!(!platform.sleep_presentation_completed(second).unwrap());
        assert!(!platform.managed_sleep_supported());
    }

    impl RenderSink for ManagedRenderSink {
        fn set_dark_mode(&self, _: bool) -> Result<(), PlatformError> {
            Ok(())
        }
        fn managed_sleep_supported(&self) -> bool {
            true
        }
        fn set_managed_sleep(&self, sleeping: bool) -> Result<(), PlatformError> {
            let mut state = self.0.lock().unwrap();
            if *state == ManagedSleepState::Terminated {
                return Err(PlatformError::Operation(
                    "fixture renderer terminated".into(),
                ));
            }
            if sleeping
                && !matches!(
                    *state,
                    ManagedSleepState::Suspending | ManagedSleepState::Suspended
                )
            {
                *state = ManagedSleepState::Suspending;
            } else if !sleeping
                && !matches!(
                    *state,
                    ManagedSleepState::Running | ManagedSleepState::Resuming
                )
            {
                *state = ManagedSleepState::Resuming;
            }
            Ok(())
        }
        fn managed_sleep_state(&self) -> Result<ManagedSleepState, PlatformError> {
            Ok(*self.0.lock().unwrap())
        }
    }

    #[test]
    fn managed_power_requires_an_explicit_renderer_and_keeps_request_separate_from_ack() {
        let platform = Platform::new(DeviceProfile::Scribe, false).unwrap();
        assert!(!platform.managed_sleep_supported());
        assert!(matches!(
            platform.set_managed_sleep(true),
            Err(PlatformError::Unsupported("managed_sleep"))
        ));
        assert!(matches!(
            platform.managed_sleep_state(),
            Err(PlatformError::Unsupported("managed_sleep"))
        ));
        platform
            .set_render_sink(Arc::new(RecordingRenderSink::default()))
            .unwrap();
        assert!(
            !platform.managed_sleep_supported(),
            "An older render sink cannot silently opt in"
        );
        assert!(matches!(
            platform.set_managed_sleep(true),
            Err(PlatformError::Unsupported("managed_sleep"))
        ));

        let sink = Arc::new(ManagedRenderSink(Mutex::new(ManagedSleepState::Running)));
        platform.set_render_sink(sink.clone()).unwrap();
        assert!(platform.managed_sleep_supported());
        platform.set_managed_sleep(true).unwrap();
        platform.set_managed_sleep(true).unwrap();
        assert_eq!(
            platform.managed_sleep_state().unwrap(),
            ManagedSleepState::Suspending
        );
        *sink.0.lock().unwrap() = ManagedSleepState::Suspended;
        assert_eq!(
            platform.managed_sleep_state().unwrap(),
            ManagedSleepState::Suspended
        );
        platform.set_managed_sleep(false).unwrap();
        assert_eq!(
            platform.managed_sleep_state().unwrap(),
            ManagedSleepState::Resuming
        );
        *sink.0.lock().unwrap() = ManagedSleepState::Running;
        assert_eq!(
            platform.managed_sleep_state().unwrap(),
            ManagedSleepState::Running
        );
        *sink.0.lock().unwrap() = ManagedSleepState::Terminated;
        assert_eq!(
            platform.managed_sleep_state().unwrap(),
            ManagedSleepState::Terminated
        );
        assert!(matches!(
            platform.set_managed_sleep(false),
            Err(PlatformError::Operation(_))
        ));
    }

    #[test]
    fn application_dark_mode_uses_the_render_sink_independently_of_hardware_inversion() {
        let platform = Platform::with_control(
            DeviceProfile::Scribe,
            true,
            Arc::new(FakeControl::default()),
        );
        assert!(matches!(
            platform.set_dark_mode(true),
            Err(PlatformError::Unsupported("application_dark_mode")),
        ));
        assert!(!platform.capabilities().application_dark_mode);
        let sink = Arc::new(RecordingRenderSink::default());
        platform.set_render_sink(sink.clone()).unwrap();
        platform.set_dark_mode(true).unwrap();
        platform.set_dark_mode(false).unwrap();
        assert_eq!(*sink.transitions.lock().unwrap(), [true, false]);
        assert!(platform.capabilities().application_dark_mode);
        assert!(!platform.capabilities().hardware_inversion);
        assert!(matches!(
            platform.set_inverted(true),
            Err(PlatformError::Unsupported("hardware_inversion")),
        ));

        platform
            .set_render_sink(Arc::new(RecordingRenderSink {
                fail: true,
                ..Default::default()
            }))
            .unwrap();
        assert!(matches!(
            platform.set_dark_mode(false),
            Err(PlatformError::Operation(_)),
        ));
    }

    #[test]
    fn native_viewport_is_separate_from_the_desktop_design_geometry() {
        let profile = DeviceProfile::Paperwhite6;
        let control = Arc::new(FakeControl {
            native_viewport: Some((1272, 1696)),
            ..Default::default()
        });
        let native = Platform::with_control(profile, true, control.clone());
        assert_eq!(native.native_viewport_size(), Some((1272, 1696)));
        assert_eq!(native.logical_viewport_size(), (636, 848));
        assert_eq!(native.scale_factor(), 2.0);

        let desktop = Platform::with_control(profile, false, control);
        assert_eq!(desktop.native_viewport_size(), None);
        assert_eq!(desktop.logical_viewport_size(), (632, 840));
        assert_eq!(desktop.scale_factor(), 1.0);
        assert_eq!(profile.logical_size(), (632, 840));
        assert_eq!(profile.physical_size(), (1264, 1680));
    }

    #[test]
    fn deferred_creation_and_drop_do_not_wait_for_a_blocked_device_command() {
        let gate = Arc::new(ResetGate::default());
        let control = Arc::new(FakeControl {
            gate: Some(gate.clone()),
            ..Default::default()
        });
        let platform = Platform::with_control(DeviceProfile::Paperwhite6, true, control.clone());
        let (created, received) = std::sync::mpsc::channel();
        let creator = std::thread::spawn(move || {
            created.send(platform.hold_awake_deferred()).unwrap();
        });
        let guard = match received.recv_timeout(Duration::from_secs(2)) {
            Ok(result) => result.unwrap(),
            Err(error) => {
                gate.release();
                creator.join().unwrap();
                panic!("deferred creation waited for the device: {error}");
            }
        };
        if !gate.wait_until_entered() {
            gate.release();
            drop(guard);
            creator.join().unwrap();
            panic!("the deferred worker did not attempt the initial reset");
        }
        let (dropped, drop_received) = std::sync::mpsc::channel();
        let dropper = std::thread::spawn(move || {
            drop(guard);
            dropped.send(()).unwrap();
        });
        let returned_without_release = drop_received.recv_timeout(Duration::from_secs(1)).is_ok();
        gate.release();
        creator.join().unwrap();
        dropper.join().unwrap();
        assert!(
            returned_without_release,
            "deferred drop joined a blocked device command"
        );
        assert_eq!(control.resets.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancellation_before_worker_start_skips_the_initial_device_command() {
        for explicit_stop in [true, false] {
            let control = Arc::new(FakeControl::default());
            let (stop, receiver) = std::sync::mpsc::channel();
            if explicit_stop {
                stop.send(()).unwrap();
            }
            drop(stop);
            let error = Arc::new(Mutex::new(None));
            run_wake_worker(control.clone(), receiver, error.clone(), true);
            assert_eq!(control.resets.load(Ordering::SeqCst), 0);
            assert!(error.lock().unwrap().is_none());
        }
    }

    #[test]
    fn explicit_wake_guard_shutdown_retains_a_timed_out_worker_until_joined() {
        let gate = Arc::new(ResetGate::default());
        let control = Arc::new(FakeControl {
            gate: Some(gate.clone()),
            ..Default::default()
        });
        let platform = Platform::with_control(DeviceProfile::Scribe, true, control.clone());
        let mut guard = platform.hold_awake_deferred().unwrap();
        if !gate.wait_until_entered() {
            gate.release();
            panic!("the fixture worker never entered its device command");
        }
        let started = Instant::now();
        let timed_out = guard.shutdown(Duration::from_millis(30));
        assert!(matches!(timed_out, Err(PlatformError::Operation(_))));
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(guard.worker.is_some());
        gate.release();
        guard.shutdown(Duration::from_secs(2)).unwrap();
        assert!(guard.worker.is_none());
        guard.shutdown(Duration::ZERO).unwrap();
        assert_eq!(control.resets.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_panicking_wake_guard_cannot_be_acknowledged_as_a_successful_shutdown() {
        let mut guard = WakeGuard {
            stop: None,
            worker: Some(std::thread::spawn(|| panic!("synthetic wake worker panic"))),
            error: Arc::new(Mutex::new(None)),
            join_on_drop: false,
            shutdown_error: None,
        };
        assert!(matches!(
            guard.shutdown(Duration::from_secs(2)),
            Err(PlatformError::Operation(_))
        ));
        assert!(guard.worker.is_none());
        assert!(guard.failure().is_some());
        assert!(matches!(
            guard.shutdown(Duration::ZERO),
            Err(PlatformError::Operation(_))
        ));
    }

    #[test]
    fn an_initial_deferred_failure_is_retained_for_the_application() {
        let control = Arc::new(FakeControl {
            fail_reset: true,
            ..Default::default()
        });
        let platform = Platform::with_control(DeviceProfile::Paperwhite6, true, control.clone());
        let mut guard = platform.hold_awake_deferred().unwrap();
        guard.worker.take().unwrap().join().unwrap();
        assert!(guard.failure().unwrap().contains("fixture reset failed"));
        assert_eq!(control.resets.load(Ordering::SeqCst), 1);
        assert!(!guard.join_on_drop);
    }

    #[test]
    fn the_synchronous_api_still_returns_initial_device_failures_directly() {
        let control = Arc::new(FakeControl {
            fail_reset: true,
            ..Default::default()
        });
        let platform = Platform::with_control(DeviceProfile::Paperwhite6, true, control.clone());
        assert!(matches!(
            platform.hold_awake(),
            Err(PlatformError::Operation(_))
        ));
        assert_eq!(control.resets.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn battery_readings_keep_valid_endpoints_and_reject_invalid_values() {
        for battery in [0, 43, 100] {
            let platform = Platform::with_control(
                DeviceProfile::Paperwhite6,
                true,
                Arc::new(FakeControl {
                    battery,
                    ..Default::default()
                }),
            );
            assert_eq!(platform.battery_percentage().unwrap(), battery);
        }
        let platform = Platform::with_control(
            DeviceProfile::Paperwhite6,
            true,
            Arc::new(FakeControl {
                battery: 101,
                ..Default::default()
            }),
        );
        assert!(matches!(
            platform.battery_percentage(),
            Err(PlatformError::InvalidValue(_))
        ));
        let desktop = Platform::new(DeviceProfile::Paperwhite6, false).unwrap();
        assert!(matches!(
            desktop.battery_percentage(),
            Err(PlatformError::Unsupported("battery"))
        ));
    }
}
