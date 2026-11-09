use std::cell::RefCell;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use slint::Rgb8Pixel;
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, RenderingRotation, RepaintBufferType,
};
use slint::platform::{Clipboard, EventLoopProxy, Platform, PlatformError, WindowAdapter};

use crate::clipboard::{ClipboardKind, ClipboardStore};
use crate::framebuffer::Framebuffer;
use crate::gyro::GyroObserver;
use crate::managed_sleep::ManagedSleepControl;
use crate::orientation::{NativeOrientationHandle, OrientationRequest, PortraitController};
use crate::power::{arm_wakealarm, find_wakealarm, suspend_to_mem};
use crate::refresh::{PendingRefresh, PhysicalRect, RefreshHandle};
use crate::render_control::{GrayMode, GrayRenderer, RenderHandle};
use crate::touch::{TouchInput, TransitionDrain, has_accelerometer};
use crate::wakeup::{self, KindleEventLoopProxy, Queue, Wakeup};
use crate::{OnWakeCallback, WakeSchedule};

// Animations get redrawn at most ~30 fps. E-ink can't keep up with anything
// faster, so quicker wakes would just waste battery.
const ANIMATION_FRAME: Duration = Duration::from_millis(33);
const ORIENTATION_TIMEOUT: Duration = Duration::from_millis(500);
const ORIENTATION_POLL: Duration = Duration::from_millis(10);

pub(crate) struct KindlePlatform {
    pub(crate) window: Rc<MinimalSoftwareWindow>,
    start: Instant,
    queue: Queue,
    wakeup: Wakeup,
    quit_flag: Arc<AtomicBool>,
    pub(crate) wake_schedule: Arc<Mutex<Option<WakeSchedule>>>,
    pub(crate) on_wake: OnWakeCallback,
    black_and_white: Arc<AtomicBool>,
    dark_mode: Arc<AtomicBool>,
    managed_sleep: Arc<ManagedSleepControl>,
    rotation: Arc<AtomicU32>,
    native_orientation: Arc<Mutex<OrientationRequest>>,
    initialized: Arc<AtomicBool>,
    refresh_pending: Arc<Mutex<PendingRefresh>>,
    clipboard: RefCell<ClipboardStore>,
}

impl KindlePlatform {
    pub(crate) fn new(
        wake_schedule: Arc<Mutex<Option<WakeSchedule>>>,
        on_wake: OnWakeCallback,
        black_and_white: Arc<AtomicBool>,
        rotation: Arc<AtomicU32>,
    ) -> std::io::Result<Self> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        let wakeup = wakeup::make_wakeup()?;
        Ok(Self {
            window,
            start: Instant::now(),
            queue: Arc::new(Mutex::new(Vec::new())),
            wakeup,
            quit_flag: Arc::new(AtomicBool::new(false)),
            wake_schedule,
            on_wake,
            black_and_white,
            dark_mode: Arc::new(AtomicBool::new(false)),
            managed_sleep: Arc::new(ManagedSleepControl::default()),
            rotation,
            native_orientation: Arc::new(Mutex::new(OrientationRequest::default())),
            initialized: Arc::new(AtomicBool::new(false)),
            refresh_pending: Arc::new(Mutex::new(PendingRefresh::default())),
            clipboard: RefCell::new(ClipboardStore::default()),
        })
    }

    pub(crate) fn refresh_handle(&self) -> RefreshHandle {
        RefreshHandle {
            pending: self.refresh_pending.clone(),
            wake: self.wakeup.write.clone(),
            quit: self.quit_flag.clone(),
        }
    }

    pub(crate) fn render_handle(&self) -> RenderHandle {
        RenderHandle {
            dark_mode: self.dark_mode.clone(),
            managed_sleep: self.managed_sleep.clone(),
            wake: self.wakeup.write.clone(),
            quit: self.quit_flag.clone(),
        }
    }

    pub(crate) fn native_orientation_handle(&self) -> NativeOrientationHandle {
        NativeOrientationHandle {
            request: self.native_orientation.clone(),
            initialized: self.initialized.clone(),
            managed_sleep: self.managed_sleep.clone(),
            wake: self.wakeup.write.clone(),
            quit: self.quit_flag.clone(),
        }
    }

    fn output_mode(&self) -> GrayMode {
        GrayMode {
            dark_mode: self.dark_mode.load(Ordering::Acquire),
            black_and_white: self.black_and_white.load(Ordering::Relaxed),
        }
    }

    /// Suspend the device to RAM once it's been idle for `stay_awake` with no
    /// pending work, then arm the wakealarm to bring it back. Returns `true`
    /// if a suspend cycle ran (the caller should restart the event loop).
    fn suspend_if_idle(
        &self,
        frame_buffer: &Framebuffer,
        wakealarm: Option<&Path>,
        last_interaction: &mut Instant,
        applied_mode: GrayMode,
        before_suspend: impl FnOnce(),
    ) -> bool {
        let (Some(schedule), Some(wakealarm_path)) = (
            *self.wake_schedule.lock().expect("wake schedule poisoned"),
            wakealarm,
        ) else {
            return false;
        };

        // Pending Slint timers don't block suspend: they'll just fire on
        // resume (a 1 Hz clock timer would otherwise pin the device awake).
        let nothing_pending = !self.window.has_active_animations()
            && self
                .queue
                .lock()
                .expect("event loop closure queue poisoned")
                .is_empty();
        let nothing_pending = nothing_pending
            && self
                .refresh_pending
                .lock()
                .expect("refresh queue poisoned")
                .is_empty();
        let nothing_pending = nothing_pending && self.output_mode() == applied_mode;
        if last_interaction.elapsed() < schedule.stay_awake || !nothing_pending {
            return false;
        }

        frame_buffer.wait_for_update_complete();

        // If arming fails we still suspend, sleeping to save battery is better than
        // staying awake.
        if let Err(e) = arm_wakealarm(wakealarm_path, schedule.wake_interval) {
            log::error!(
                "failed to arm RTC wakealarm: {e}; device may only wake on user input this cycle"
            );
        }
        before_suspend();
        if let Err(e) = suspend_to_mem() {
            log::error!("suspend-to-RAM failed: {e}");
        }

        // Start a fresh stay_awake window so the consumer's app
        // gets at least that long to react.
        *last_interaction = Instant::now();
        // Fire the consumer's on-wake callback (if any) before any rendering
        // this cycle, so e.g. an HTTP poll runs before the next draw shows
        // stale data.
        if let Some(callback) = self.on_wake.borrow_mut().as_mut() {
            callback();
        }
        true
    }
}

impl Platform for KindlePlatform {
    fn set_clipboard_text(&self, text: &str, clipboard: Clipboard) {
        let kind = match clipboard {
            Clipboard::DefaultClipboard => ClipboardKind::Default,
            Clipboard::SelectionClipboard => ClipboardKind::Selection,
            _ => return,
        };
        self.clipboard.borrow_mut().set(kind, text);
    }

    fn clipboard_text(&self, clipboard: Clipboard) -> Option<String> {
        let kind = match clipboard {
            Clipboard::DefaultClipboard => ClipboardKind::Default,
            Clipboard::SelectionClipboard => ClipboardKind::Selection,
            _ => return None,
        };
        self.clipboard.borrow().text(kind)
    }

    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(KindleEventLoopProxy {
            queue: self.queue.clone(),
            write_fd: self.wakeup.write.clone(),
            quit_flag: self.quit_flag.clone(),
        }))
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        struct QuitOnExit(Arc<AtomicBool>, Arc<ManagedSleepControl>);
        impl Drop for QuitOnExit {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
                self.1.terminate();
            }
        }
        let _quit_on_exit = QuitOnExit(self.quit_flag.clone(), self.managed_sleep.clone());
        self.initialized.store(true, Ordering::Release);
        let mut frame_buffer = Framebuffer::open()
            .map_err(|e| PlatformError::Other(format!("failed to open /dev/fb0: {e}")))?;

        let mut touch_input = Some(
            TouchInput::open(frame_buffer.width, frame_buffer.height)
                .map_err(|e| PlatformError::Other(format!("failed to open touch input: {e}")))?,
        );

        let initial_mode = self.output_mode();
        frame_buffer.fill(initial_mode.paper());
        frame_buffer.refresh_full();

        let width = frame_buffer.width as usize;
        let mut rgb_buffer = vec![Rgb8Pixel::default(); width * frame_buffer.height as usize];
        let mut gray_renderer = GrayRenderer::new(width, initial_mode);

        let (panel_width, panel_height) = (frame_buffer.width, frame_buffer.height);
        let mut portrait = PortraitRuntime::new(launch_render_offset());
        let mut input_generation = 1u64;
        portrait.ensure_active(&self.native_orientation, &frame_buffer)?;
        let mut applied_rotation = self.rotation.load(Ordering::Relaxed);
        self.window.set_size(portrait_window_size(
            panel_width,
            panel_height,
            (applied_rotation + portrait.controller.render_offset()) % 360,
        ));
        let wakeup_read_fd = self.wakeup.read.as_raw_fd();
        let wakealarm = find_wakealarm().ok();
        let mut last_interaction = Instant::now();
        let mut restore_full_frame = false;
        let mut presentation_barrier = true;
        let mut post_frame_input = PostFrameInputGate::default();

        loop {
            if self.quit_flag.load(Ordering::SeqCst) {
                break;
            }
            if pause_native_inputs(
                &self.managed_sleep,
                &mut touch_input,
                &mut portrait,
                &mut input_generation,
                &self.window,
            ) {
                restore_full_frame = false;
                presentation_barrier = false;
                post_frame_input.clear();
            } else {
                if resume_native_input(
                    &self.managed_sleep,
                    &mut touch_input,
                    &mut input_generation,
                    panel_width,
                    panel_height,
                )? {
                    restore_full_frame = true;
                    presentation_barrier = true;
                }
                if portrait.ensure_active(&self.native_orientation, &frame_buffer)? {
                    begin_presentation_transition(
                        &mut touch_input,
                        &mut input_generation,
                        &self.window,
                    )?;
                    presentation_barrier = true;
                }
            }
            if !self.managed_sleep.requested()
                && !self.managed_sleep.suspended()
                && !restore_full_frame
                && !presentation_barrier
                && !post_frame_input.waiting
                && self.suspend_if_idle(
                    &frame_buffer,
                    wakealarm.as_deref(),
                    &mut last_interaction,
                    gray_renderer.mode(),
                    || portrait.pause(),
                )
            {
                restore_full_frame = true;
                presentation_barrier = true;
                begin_presentation_transition(
                    &mut touch_input,
                    &mut input_generation,
                    &self.window,
                )?;
                continue;
            }

            let timeout_ms: libc::c_int = if (restore_full_frame || presentation_barrier)
                && !portrait.input_gated()
                && !self.managed_sleep.requested()
            {
                1
            } else {
                match (
                    self.window.has_active_animations(),
                    slint::platform::duration_until_next_timer_update(),
                ) {
                    (true, Some(d)) => duration_to_ms(d.min(ANIMATION_FRAME)),
                    (true, None) => duration_to_ms(ANIMATION_FRAME),
                    (false, Some(d)) => duration_to_ms(d),
                    (false, None) => -1,
                }
            };
            let polled_input_generation = input_generation;
            let polled_portrait_generation = portrait.generation;
            let mut file_descriptors = [
                libc::pollfd {
                    fd: touch_input.as_ref().map_or(-1, TouchInput::fd),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: wakeup_read_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: portrait.observer.as_ref().map_or(-1, GyroObserver::fd),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            // SAFETY: The three descriptors and their backing owners remain live
            // while poll runs. A retired descriptor is checked by generation.
            if unsafe {
                libc::poll(
                    file_descriptors.as_mut_ptr(),
                    file_descriptors.len() as libc::nfds_t,
                    timeout_ms,
                )
            } < 0
            {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(PlatformError::Other(format!("poll failed: {error}")));
            }
            let err_bits = libc::POLLERR | libc::POLLHUP | libc::POLLNVAL;
            if file_descriptors[1].revents & err_bits != 0 {
                return Err(PlatformError::Other(format!(
                    "poll: control fd died (revents={:#x})",
                    file_descriptors[1].revents,
                )));
            }
            if file_descriptors[1].revents & libc::POLLIN != 0 {
                wakeup::drain(&self.wakeup.read);
                let pending: Vec<_> = self
                    .queue
                    .lock()
                    .expect("event loop closure queue poisoned")
                    .drain(..)
                    .collect();
                for callback in pending {
                    if self.quit_flag.load(Ordering::SeqCst) {
                        break;
                    }
                    callback();
                }
            }
            // Control and managed-sleep timers precede both direction and touch.
            slint::platform::update_timers_and_animations();
            if self.quit_flag.load(Ordering::SeqCst) {
                break;
            }
            if pause_native_inputs(
                &self.managed_sleep,
                &mut touch_input,
                &mut portrait,
                &mut input_generation,
                &self.window,
            ) {
                restore_full_frame = false;
                presentation_barrier = false;
                post_frame_input.clear();
                continue;
            }
            if resume_native_input(
                &self.managed_sleep,
                &mut touch_input,
                &mut input_generation,
                panel_width,
                panel_height,
            )? {
                restore_full_frame = true;
                presentation_barrier = true;
            }
            if portrait.ensure_active(&self.native_orientation, &frame_buffer)? {
                begin_presentation_transition(
                    &mut touch_input,
                    &mut input_generation,
                    &self.window,
                )?;
                presentation_barrier = true;
            }
            if polled_portrait_generation == portrait.generation {
                if file_descriptors[2].revents & err_bits != 0 {
                    portrait.retire_observer();
                } else if file_descriptors[2].revents & libc::POLLIN != 0
                    && portrait.poll(&frame_buffer)?
                {
                    begin_presentation_transition(
                        &mut touch_input,
                        &mut input_generation,
                        &self.window,
                    )?;
                    presentation_barrier = true;
                    last_interaction = Instant::now();
                }
            }
            let app_rotation = self.rotation.load(Ordering::Relaxed);
            let rotation_changed = app_rotation != applied_rotation;
            if rotation_changed {
                applied_rotation = app_rotation;
                begin_presentation_transition(
                    &mut touch_input,
                    &mut input_generation,
                    &self.window,
                )?;
                presentation_barrier = true;
            }
            if polled_input_generation == input_generation
                && file_descriptors[0].revents & err_bits != 0
            {
                return Err(PlatformError::Other(format!(
                    "poll: touch fd died (revents={:#x})",
                    file_descriptors[0].revents,
                )));
            }
            if post_frame_input.waiting
                && !presentation_barrier
                && !restore_full_frame
                && polled_input_generation == input_generation
                && (file_descriptors[0].revents | file_descriptors[1].revents) & libc::POLLIN != 0
            {
                let drain = match touch_input.as_mut() {
                    Some(input) => input.continue_presentation_transition().map_err(|error| {
                        PlatformError::Other(format!(
                            "failed to synchronize cancelled input: {error}"
                        ))
                    })?,
                    None => TransitionDrain::Ready,
                };
                post_frame_input.retry(drain, &self.managed_sleep);
                if !post_frame_input.waiting {
                    last_interaction = Instant::now();
                }
            }
            if !presentation_barrier
                && !restore_full_frame
                && !post_frame_input.waiting
                && !portrait.input_gated()
                && !self.managed_sleep.requested()
                && polled_input_generation == input_generation
                && file_descriptors[0].revents & libc::POLLIN != 0
            {
                last_interaction = Instant::now();
                if let Some(input) = touch_input.as_mut() {
                    input
                        .poll(
                            &self.window,
                            (applied_rotation + portrait.controller.touch_offset()) % 360,
                        )
                        .map_err(|error| {
                            PlatformError::Other(format!("failed to read touch input: {error}"))
                        })?;
                }
            }
            // An input callback can request sleep synchronously.
            if pause_native_inputs(
                &self.managed_sleep,
                &mut touch_input,
                &mut portrait,
                &mut input_generation,
                &self.window,
            ) {
                restore_full_frame = false;
                presentation_barrier = false;
                post_frame_input.clear();
                continue;
            }
            if portrait.input_gated() {
                continue;
            }
            let output_mode = self.output_mode();
            let mode_changed = output_mode != gray_renderer.mode();
            let force_full = restore_full_frame
                || presentation_barrier
                || (post_frame_input.waiting && mode_changed);
            let render_rotation = (applied_rotation + portrait.controller.render_offset()) % 360;
            if force_full {
                self.window.set_size(portrait_window_size(
                    panel_width,
                    panel_height,
                    render_rotation,
                ));
            }
            if mode_changed || force_full {
                self.window.request_redraw();
            }
            let pending = self
                .refresh_pending
                .lock()
                .expect("refresh queue poisoned")
                .take();
            if !pending.is_empty() {
                self.window.request_redraw();
            }
            let mut dirty_region = None;
            self.window.draw_if_needed(|renderer| {
                renderer.set_rendering_rotation(match render_rotation {
                    90 => RenderingRotation::Rotate90,
                    180 => RenderingRotation::Rotate180,
                    270 => RenderingRotation::Rotate270,
                    _ => RenderingRotation::NoRotation,
                });
                if force_full {
                    renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
                }
                let dirty = renderer.render(&mut rgb_buffer, width);
                if force_full {
                    renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
                }
                let origin = dirty.bounding_box_origin();
                let size = dirty.bounding_box_size();
                dirty_region = Some(PhysicalRect {
                    x: origin.x,
                    y: origin.y,
                    width: size.width,
                    height: size.height,
                });
            });
            let written = gray_renderer.write_frame(
                &rgb_buffer,
                (panel_width, panel_height),
                dirty_region,
                output_mode,
                force_full,
                |row, columns, pixels| frame_buffer.write_line(row, columns, pixels),
            );
            if force_full {
                frame_buffer.refresh_full_checked().map_err(|error| {
                    PlatformError::Other(format!(
                        "failed to restore the presentation frame: {error}"
                    ))
                })?;
                frame_buffer.wait_for_update_complete();
                let drain = match touch_input.as_mut() {
                    Some(input) => input.finish_presentation_transition().map_err(|error| {
                        PlatformError::Other(format!(
                            "failed to drain the previous presentation input: {error}"
                        ))
                    })?,
                    None => TransitionDrain::Ready,
                };
                presentation_barrier = false;
                post_frame_input.finish_frame(drain, restore_full_frame, &self.managed_sleep);
                restore_full_frame = false;
                last_interaction = Instant::now();
            } else if written.mode_changed {
                frame_buffer.refresh_full();
            } else if let Some(refresh) = crate::refresh::resolve(
                pending,
                written.region,
                panel_width,
                panel_height,
                render_rotation,
            ) {
                frame_buffer.refresh_planned(refresh);
            }
        }
        Ok(())
    }
}

/// A written presentation is retained while its cancelled input stream settles.
/// Waiting does not request another redraw or release the existing input owner.
#[derive(Default)]
struct PostFrameInputGate {
    waiting: bool,
    resume_pending: bool,
}

impl PostFrameInputGate {
    fn clear(&mut self) {
        self.waiting = false;
        self.resume_pending = false;
    }

    fn finish_frame(
        &mut self,
        drain: TransitionDrain,
        resuming: bool,
        control: &ManagedSleepControl,
    ) {
        self.resume_pending |= resuming;
        self.retry(drain, control);
    }

    fn retry(&mut self, drain: TransitionDrain, control: &ManagedSleepControl) {
        self.waiting = matches!(drain, TransitionDrain::Waiting);
        if !self.waiting && self.resume_pending {
            self.resume_pending = false;
            control.acknowledge_running();
        }
    }
}

struct PortraitRuntime {
    controller: PortraitController,
    observer: Option<GyroObserver>,
    attempted: bool,
    snapshot_pending: bool,
    seen_request: u64,
    generation: u64,
}

impl PortraitRuntime {
    fn new(legacy_render_offset: u32) -> Self {
        Self {
            controller: PortraitController::new(legacy_render_offset),
            observer: None,
            attempted: false,
            snapshot_pending: false,
            seen_request: 0,
            generation: 0,
        }
    }

    fn retire_observer(&mut self) {
        if self.observer.take().is_some() {
            self.generation = self.generation.wrapping_add(1);
        }
        self.snapshot_pending = false;
        self.attempted = true;
    }

    fn pause(&mut self) {
        self.retire_observer();
        self.attempted = false;
    }

    fn input_gated(&self) -> bool {
        self.snapshot_pending
            || self
                .observer
                .as_ref()
                .is_some_and(|observer| observer.has_backlog() || !observer.is_synchronized())
    }

    fn ensure_active(
        &mut self,
        shared_request: &Mutex<OrientationRequest>,
        framebuffer: &Framebuffer,
    ) -> Result<bool, PlatformError> {
        let request = *shared_request.lock().map_err(|_| {
            PlatformError::Other("the native orientation request is unavailable".into())
        })?;
        let mut changed = false;
        if request.sequence != self.seen_request {
            self.seen_request = request.sequence;
            match framebuffer.portrait_rotation().and_then(|rotation| {
                self.controller
                    .apply_request(request, rotation)
                    .map_err(io::Error::other)
            }) {
                Ok(applied) => {
                    changed |= applied;
                    self.pause();
                }
                Err(error) => {
                    // Optional calibration failure preserves the old startup map.
                    log::warn!("native portrait calibration was not applied: {error}");
                }
            }
        }
        if self.controller.calibrated() && !self.attempted {
            self.attempted = true;
            match GyroObserver::open() {
                Ok(Some(observer)) => {
                    self.observer = Some(observer);
                    self.generation = self.generation.wrapping_add(1);
                    self.snapshot_pending = true;
                }
                Ok(None) => {}
                Err(error) => log::warn!("native portrait observer is unavailable: {error}"),
            }
        }
        if self.snapshot_pending {
            changed |= self.refresh_snapshot(framebuffer)?;
        }
        Ok(changed)
    }

    fn refresh_snapshot(&mut self, framebuffer: &Framebuffer) -> Result<bool, PlatformError> {
        let Some(observer) = self.observer.as_mut() else {
            self.snapshot_pending = false;
            return Ok(false);
        };
        let pose = match observer.snapshot() {
            Ok(pose) => {
                self.snapshot_pending = false;
                pose
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => {
                log::warn!("native portrait snapshot is unavailable: {error}");
                self.retire_observer();
                return Ok(false);
            }
        };
        self.observe_pose(pose, framebuffer)
    }

    fn poll(&mut self, framebuffer: &Framebuffer) -> Result<bool, PlatformError> {
        if self.snapshot_pending {
            return self.refresh_snapshot(framebuffer);
        }
        let Some(observer) = self.observer.as_mut() else {
            return Ok(false);
        };
        let pose = match observer.poll() {
            Ok(pose) => pose,
            Err(error) => {
                log::warn!("native portrait event observer was retired: {error}");
                self.retire_observer();
                return Ok(false);
            }
        };
        self.observe_pose(pose, framebuffer)
    }

    fn observe_pose(
        &mut self,
        pose: Option<crate::gyro::NativePortrait>,
        framebuffer: &Framebuffer,
    ) -> Result<bool, PlatformError> {
        self.observe_pose_with(pose, || framebuffer.portrait_rotation())
    }

    fn observe_pose_with(
        &mut self,
        pose: Option<crate::gyro::NativePortrait>,
        rotation: impl FnOnce() -> io::Result<u32>,
    ) -> Result<bool, PlatformError> {
        let Some(pose) = pose else { return Ok(false) };
        let raw_rotation = match rotation() {
            Ok(rotation) => rotation,
            Err(error) => {
                log::warn!("optional native portrait scanout is unavailable: {error}");
                self.retire_observer();
                return Ok(false);
            }
        };
        match self.controller.observe(pose.into(), raw_rotation) {
            Ok(changed) => Ok(changed),
            Err(error) => {
                log::warn!("optional native portrait observation was ignored: {error}");
                self.retire_observer();
                Ok(false)
            }
        }
    }
}

fn portrait_window_size(width: u32, height: u32, render_rotation: u32) -> slint::PhysicalSize {
    if render_rotation.is_multiple_of(180) {
        slint::PhysicalSize::new(width, height)
    } else {
        slint::PhysicalSize::new(height, width)
    }
}

fn begin_presentation_transition(
    input: &mut Option<TouchInput>,
    generation: &mut u64,
    window: &MinimalSoftwareWindow,
) -> Result<(), PlatformError> {
    if let Some(input) = input.as_mut() {
        let _ = input.cancel_and_drain(window).map_err(|error| {
            PlatformError::Other(format!(
                "failed to cancel the previous presentation input: {error}"
            ))
        })?;
    }
    *generation = generation.wrapping_add(1);
    Ok(())
}

fn pause_native_inputs(
    control: &ManagedSleepControl,
    input: &mut Option<TouchInput>,
    portrait: &mut PortraitRuntime,
    generation: &mut u64,
    window: &MinimalSoftwareWindow,
) -> bool {
    let had_input = input.is_some();
    if control.requested() {
        // Close the observer before the input-release acknowledgement can escape.
        portrait.pause();
    }
    let paused = pause_managed_input(control, input, |input| input.cancel(window));
    if had_input && input.is_none() {
        *generation = generation.wrapping_add(1);
    }
    paused
}

fn resume_native_input(
    control: &ManagedSleepControl,
    input: &mut Option<TouchInput>,
    generation: &mut u64,
    width: u32,
    height: u32,
) -> Result<bool, PlatformError> {
    let resumed = begin_managed_resume(control, input, || TouchInput::open(width, height))
        .map_err(|error| PlatformError::Other(format!("failed to restore touch input: {error}")))?;
    if resumed {
        *generation = generation.wrapping_add(1);
    }
    Ok(resumed)
}

/// A completed pause must never retain an input owner. This gate is repeated
/// after callbacks because the main timer can request suspension mid-iteration.
fn pause_managed_input<T>(
    control: &ManagedSleepControl,
    input: &mut Option<T>,
    cancel: impl FnOnce(&mut T),
) -> bool {
    if !control.requested() {
        return false;
    }
    if input.is_some() || !control.suspended() {
        control.begin_transition();
        if !control.requested() {
            if input.is_some() {
                control.acknowledge_running();
            } else {
                control.acknowledge_suspended();
            }
            return false;
        }
        if let Some(mut contact) = input.take() {
            cancel(&mut contact);
            drop(contact);
        }
        control.acknowledge_suspended();
    }
    true
}

/// Keep Resuming observable while input is open but its retained RGB has not
/// yet replaced the native screen. A failed open terminates the handoff.
fn begin_managed_resume<T, E>(
    control: &ManagedSleepControl,
    input: &mut Option<T>,
    open: impl FnOnce() -> Result<T, E>,
) -> Result<bool, E> {
    if control.requested() || !control.suspended() || input.is_some() {
        return Ok(false);
    }
    control.begin_transition();
    if control.requested() {
        control.acknowledge_suspended();
        return Ok(false);
    }
    match open() {
        Ok(contact) => {
            *input = Some(contact);
            Ok(true)
        }
        Err(error) => {
            control.terminate();
            Err(error)
        }
    }
}

fn duration_to_ms(d: Duration) -> libc::c_int {
    // Round up to at least 1 ms. A timeout of 0 makes poll skip the wait
    // entirely, which would spin the CPU if a tiny timer kept re-firing.
    d.as_millis().clamp(1, libc::c_int::MAX as u128) as libc::c_int
}

impl Drop for KindlePlatform {
    fn drop(&mut self) {
        self.quit_flag.store(true, Ordering::SeqCst);
    }
}

/// Devices with an accelerometer let the framework flip its UI 180° when
/// held the other way up. Touch input then already arrives flipped, but the
/// framebuffer does not, so rendering has to compensate by whatever the
/// framework thinks is up when the app starts.
pub(crate) fn launch_render_offset() -> u32 {
    if !has_accelerometer() {
        return 0;
    }
    let mut command = Command::new("lipc-get-prop");
    command.args(["com.lab126.winmgr", "accelerometer"]);
    query_render_offset(&mut command, ORIENTATION_TIMEOUT).unwrap_or(0)
}

fn parse_render_offset(output: &[u8]) -> Option<u32> {
    match output.trim_ascii() {
        b"U" => Some(0),
        b"D" => Some(180),
        _ => None,
    }
}

fn query_render_offset(command: &mut Command, timeout: Duration) -> Option<u32> {
    // This bounds normal userspace protocol polling; it is not a hard deadline
    // on kernel process operations such as spawn, kill, or child reaping.
    let deadline = Instant::now().checked_add(timeout)?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let result = read_render_offset(&mut child, deadline);
    retire_orientation_child(child);
    result
}

fn read_render_offset(child: &mut Child, deadline: Instant) -> Option<u32> {
    let mut stdout = child.stdout.take()?;
    set_orientation_pipe_nonblocking(stdout.as_raw_fd()).ok()?;
    // Reserve one extra byte so an overlong, truncated reply cannot be accepted.
    let mut bytes = [0u8; 17];
    let mut length = 0;
    let mut eof = false;
    loop {
        if Instant::now() >= deadline {
            return None;
        }
        if !eof {
            match stdout.read(&mut bytes[length..]) {
                Ok(0) => eof = true,
                Ok(count) => {
                    length += count;
                    if length == bytes.len() {
                        return None;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            }
        }
        if let Some(status) = child.try_wait().ok()? {
            if !status.success() {
                return None;
            }
            // A descendant can inherit stdout after the direct child exits.
            // Require EOF under the same deadline rather than a blocking read.
            if eof {
                return parse_render_offset(&bytes[..length]);
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        std::thread::sleep(remaining.min(ORIENTATION_POLL));
    }
}

fn set_orientation_pipe_nonblocking(descriptor: RawFd) -> io::Result<()> {
    // SAFETY: descriptor is the owned child stdout pipe and both fcntl requests
    // use integer flags, with no pointer arguments or change in fd ownership.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn retire_orientation_child(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod managed_sleep_tests {
    use super::{begin_managed_resume, pause_managed_input};
    use crate::managed_sleep::{ManagedSleepControl, ManagedSleepState};
    use std::{cell::RefCell, rc::Rc};

    struct InputOwner(Rc<RefCell<Vec<&'static str>>>);
    impl Drop for InputOwner {
        fn drop(&mut self) {
            self.0.borrow_mut().push("release-input");
        }
    }

    #[test]
    fn suspended_input_is_released_before_ack_and_repeated_gates_do_not_draw() {
        let control = ManagedSleepControl::default();
        let observed = Rc::new(RefCell::new(Vec::new()));
        let mut input = Some(InputOwner(observed.clone()));
        control.request(true);
        let paused = pause_managed_input(&control, &mut input, |_| {
            assert_eq!(control.state(), ManagedSleepState::Suspending);
            observed.borrow_mut().push("cancel-pointer");
        });
        assert!(paused);
        assert!(input.is_none());
        assert_eq!(&*observed.borrow(), &["cancel-pointer", "release-input"]);
        assert_eq!(control.state(), ManagedSleepState::Suspended);
        for _ in 0..4 {
            if !pause_managed_input(&control, &mut input, |_| panic!("No contact remains")) {
                observed.borrow_mut().push("draw");
            }
        }
        assert_eq!(&*observed.borrow(), &["cancel-pointer", "release-input"]);
    }

    #[test]
    fn a_reopened_input_waits_for_the_frame_ack_and_can_be_suspended_again() {
        let control = ManagedSleepControl::default();
        let observed = Rc::new(RefCell::new(Vec::new()));
        let mut input = Some(InputOwner(observed.clone()));
        control.request(true);
        pause_managed_input(&control, &mut input, |_| {});
        control.request(false);
        assert!(
            begin_managed_resume(&control, &mut input, || {
                assert_eq!(control.state(), ManagedSleepState::Resuming);
                Ok::<_, ()>(InputOwner(observed.clone()))
            })
            .unwrap()
        );
        assert_eq!(control.state(), ManagedSleepState::Resuming);
        assert!(
            !begin_managed_resume(&control, &mut input, || -> Result<InputOwner, ()> {
                panic!("An input owner must not be opened twice")
            })
            .unwrap()
        );
        // A reversed request cannot claim suspension until the new owner drops.
        control.request(true);
        assert_eq!(control.state(), ManagedSleepState::Suspending);
        pause_managed_input(&control, &mut input, |_| {});
        assert_eq!(control.state(), ManagedSleepState::Suspended);
        assert_eq!(&*observed.borrow(), &["release-input", "release-input"]);
        control.request(false);
        assert!(
            begin_managed_resume(&control, &mut input, || {
                Ok::<_, ()>(InputOwner(observed.clone()))
            })
            .unwrap()
        );
        assert_eq!(control.state(), ManagedSleepState::Resuming);
        control.acknowledge_running();
        assert_eq!(control.state(), ManagedSleepState::Running);
        drop(input);
        assert_eq!(observed.borrow().len(), 3);
    }

    #[test]
    fn input_reacquisition_failure_terminates_without_a_retained_grab() {
        let control = ManagedSleepControl::default();
        let observed = Rc::new(RefCell::new(Vec::new()));
        let mut input = Some(InputOwner(observed.clone()));
        control.request(true);
        pause_managed_input(&control, &mut input, |_| {});
        control.request(false);
        assert_eq!(
            begin_managed_resume(&control, &mut input, || {
                Err::<InputOwner, _>("fixture-grab-failed")
            }),
            Err("fixture-grab-failed")
        );
        assert!(input.is_none());
        assert_eq!(control.state(), ManagedSleepState::Terminated);
        assert_eq!(&*observed.borrow(), &["release-input"]);
        assert!(!control.request(false));
    }
}

#[cfg(test)]
mod active_orientation_runtime_tests {
    use super::*;
    use crate::gyro::NativePortrait;
    use crate::orientation::{NativePortraitOrientation, PortraitCalibration};

    #[test]
    fn a_written_frame_waits_for_synchronized_input_before_resuming() {
        use crate::managed_sleep::ManagedSleepState;
        let control = ManagedSleepControl::default();
        control.request(true);
        control.acknowledge_suspended();
        control.request(false);
        control.begin_transition();
        let mut gate = PostFrameInputGate::default();
        gate.finish_frame(TransitionDrain::Waiting, true, &control);
        assert!(gate.waiting && gate.resume_pending);
        assert_eq!(control.state(), ManagedSleepState::Resuming);
        gate.retry(TransitionDrain::Waiting, &control);
        assert_eq!(control.state(), ManagedSleepState::Resuming);
        gate.retry(TransitionDrain::Ready, &control);
        assert!(!gate.waiting && !gate.resume_pending);
        assert_eq!(control.state(), ManagedSleepState::Running);
        control.request(true);
        control.acknowledge_suspended();
        control.request(false);
        control.begin_transition();
        gate.finish_frame(TransitionDrain::Waiting, true, &control);
        control.request(true);
        gate.retry(TransitionDrain::Ready, &control);
        assert_eq!(control.state(), ManagedSleepState::Suspending);
        gate.clear();
        assert!(!gate.waiting && !gate.resume_pending);
    }

    #[test]
    fn optional_observation_failure_keeps_the_installed_runtime_mapping() {
        let mut runtime = PortraitRuntime::new(180);
        runtime
            .controller
            .apply_request(
                OrientationRequest {
                    sequence: 1,
                    calibration: Some(PortraitCalibration {
                        holding: NativePortraitOrientation::Up,
                        scanout: NativePortraitOrientation::Up,
                        raw_rotation: 3,
                    }),
                },
                3,
            )
            .unwrap();
        assert!(
            runtime
                .observe_pose_with(Some(NativePortrait::Down), || Ok(1))
                .unwrap()
        );
        let installed = (
            runtime.controller.render_offset(),
            runtime.controller.touch_offset(),
        );
        assert_eq!(installed, (0, 180));
        assert_eq!(
            runtime
                .observe_pose_with(Some(NativePortrait::Up), || {
                    Err(io::Error::other("Synthetic optional GETVAR failure"))
                })
                .unwrap(),
            false
        );
        assert_eq!(
            (
                runtime.controller.render_offset(),
                runtime.controller.touch_offset()
            ),
            installed
        );
        assert!(runtime.attempted);
        assert_eq!(
            runtime
                .observe_pose_with(Some(NativePortrait::Up), || Ok(0))
                .unwrap(),
            false
        );
        assert_eq!(
            (
                runtime.controller.render_offset(),
                runtime.controller.touch_offset()
            ),
            installed
        );
        runtime.pause();
        assert!(!runtime.attempted);
        assert!(
            runtime
                .observe_pose_with(Some(NativePortrait::Up), || Ok(3))
                .unwrap()
        );
        assert_eq!(
            (
                runtime.controller.render_offset(),
                runtime.controller.touch_offset()
            ),
            (0, 0)
        );
        assert!(
            !runtime
                .observe_pose_with(None, || panic!("No report must not query scanout"))
                .unwrap()
        );
    }
}

#[cfg(test)]
mod orientation_tests {
    use super::{
        parse_render_offset, query_render_offset, read_render_offset, retire_orientation_child,
        set_orientation_pipe_nonblocking,
    };
    use std::fs;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    #[derive(Clone, Copy, Debug)]
    struct ProcessIdentity {
        pid: u32,
        parent: u32,
        group: i32,
        birth: u64,
        state: u8,
    }

    fn observe_identity(pid: u32) -> Option<ProcessIdentity> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let fields = stat
            .rsplit_once(')')?
            .1
            .split_whitespace()
            .collect::<Vec<_>>();
        Some(ProcessIdentity {
            pid: stat.split_once(' ')?.0.parse().ok()?,
            parent: fields.get(1)?.parse().ok()?,
            group: fields.get(2)?.parse().ok()?,
            birth: fields.get(19)?.parse().ok()?,
            state: *fields.first()?.as_bytes().first()?,
        })
    }

    fn is_original_live_anchor(identity: ProcessIdentity) -> bool {
        // SAFETY: getpgrp has no arguments or owned resources.
        let parent_group = unsafe { libc::getpgrp() };
        observe_identity(identity.pid).is_some_and(|current| {
            current.pid == identity.pid
                && identity.parent == std::process::id()
                && current.parent == identity.parent
                && current.birth == identity.birth
                && current.group == identity.group
                && Some(current.group) == i32::try_from(current.pid).ok()
                && current.group != parent_group
                && !matches!(current.state, b'Z' | b'X' | b'x')
        })
    }

    /// A separate finite anchor pins the group after the reply shell is reaped.
    /// Cleanup authority never comes from that shell or its mutable numeric PID.
    struct FixtureGroup {
        anchor: Option<Child>,
        identity: Option<ProcessIdentity>,
    }

    impl FixtureGroup {
        fn new() -> Self {
            let anchor = Command::new("/bin/sh")
                .args([
                    "-c",
                    r#"
                        anchor_pid=$$
                        IFS= read -r anchor_stat </proc/self/stat || exit 91
                        anchor_tail=${anchor_stat##*) }
                        set -- $anchor_tail
                        [ "$#" -ge 20 ] && [ "$3" = "$anchor_pid" ] || exit 92
                        shift 19
                        anchor_birth=$1
                        sleep 20
                        IFS= read -r anchor_stat </proc/self/stat || exit 93
                        [ "${anchor_stat%% *}" = "$anchor_pid" ] || exit 94
                        anchor_tail=${anchor_stat##*) }
                        set -- $anchor_tail
                        [ "$#" -ge 20 ] && [ "$3" = "$anchor_pid" ] || exit 95
                        shift 19
                        [ "$1" = "$anchor_birth" ] || exit 96
                        kill -KILL "-$anchor_pid"
                    "#,
                ])
                .process_group(0)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("Spawn the finite orientation fixture anchor");
            let pid = anchor.id();
            let mut fixture = Self {
                anchor: Some(anchor),
                identity: None,
            };
            let deadline = Instant::now() + Duration::from_secs(1);
            loop {
                if let Some(identity) = observe_identity(pid) {
                    assert!(is_original_live_anchor(identity));
                    fixture.identity = Some(identity);
                    return fixture;
                }
                assert!(
                    Instant::now() < deadline,
                    "Observe the live orientation anchor"
                );
                thread::sleep(Duration::from_millis(2));
            }
        }

        fn command(&self, script: &str) -> Command {
            let identity = self.identity.expect("The orientation anchor was captured");
            assert!(
                is_original_live_anchor(identity),
                "The orientation anchor expired"
            );
            let mut command = Command::new("/bin/sh");
            command.args(["-c", script]).process_group(identity.group);
            command
        }
    }

    impl Drop for FixtureGroup {
        fn drop(&mut self) {
            let Some(mut anchor) = self.anchor.take() else {
                return;
            };
            if let Some(identity) = self.identity {
                if is_original_live_anchor(identity) {
                    // SAFETY: the original same-birth, live direct anchor pins
                    // this private group even after the reply shell is reaped.
                    unsafe { libc::kill(-identity.group, libc::SIGKILL) };
                }
            } else {
                // No helper can join before construction captures the anchor.
                // Child owns only this direct process; no group is authorized.
                let _ = anchor.kill();
            }
            let deadline = Instant::now() + Duration::from_secs(1);
            loop {
                match anchor.try_wait() {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Ok(None) => break,
                }
            }
        }
    }

    fn fixture(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    #[test]
    fn only_complete_up_or_down_orientation_tokens_are_accepted() {
        assert_eq!(parse_render_offset(b"U\n"), Some(0));
        assert_eq!(parse_render_offset(b" \tD\r\n"), Some(180));
        for output in [
            &b""[..],
            &b"u"[..],
            &b"DD"[..],
            &b"U D"[..],
            &b"orientation D"[..],
            &b"\xffD"[..],
        ] {
            assert_eq!(parse_render_offset(output), None);
        }
    }

    #[test]
    fn child_fixtures_require_success_and_reject_overlong_replies() {
        for (script, expected) in [
            ("printf 'U\\n'", Some(0)),
            ("printf 'D\\n'", Some(180)),
            ("exit 0", None),
            ("printf D; exit 7", None),
            ("printf 'D               '", Some(180)),
            ("printf 'D                '", None),
            ("printf 'U D'", None),
        ] {
            assert_eq!(
                query_render_offset(&mut fixture(script), Duration::from_secs(1)),
                expected,
                "{script}",
            );
        }
    }

    #[test]
    fn a_hung_child_is_killed_and_reaped_after_the_poll_deadline() {
        let started = Instant::now();
        assert_eq!(
            query_render_offset(
                &mut fixture("printf D; exec sleep 5"),
                Duration::from_millis(60),
            ),
            None,
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn inherited_stdout_cannot_hold_the_orientation_query_open() {
        let group = FixtureGroup::new();
        let mut child = group
            .command("sleep 5 & printf D")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let helper = observe_identity(child.id());
        let expected_group = group.identity.unwrap().group;
        let started = Instant::now();
        let result = read_render_offset(&mut child, started + Duration::from_millis(60));
        let shell_exited = matches!(child.try_wait(), Ok(Some(status)) if status.success());
        let pipe_released = child.stdout.is_none();
        retire_orientation_child(child);
        drop(group);
        let helper = helper.expect("Observe the owned reply shell");
        assert_eq!(helper.parent, std::process::id());
        assert_eq!(helper.group, expected_group);
        assert_eq!(result, None);
        assert!(
            shell_exited,
            "The finite descendant retained stdout after shell exit"
        );
        assert!(pipe_released, "The parent reply pipe was released");
        assert!(observe_identity(helper.pid).is_none_or(|current| current.birth != helper.birth));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn expired_or_mismatched_anchors_cannot_authorize_fixture_group_cleanup() {
        let group = FixtureGroup::new();
        let original = group.identity.unwrap();
        assert!(is_original_live_anchor(original));
        assert!(!is_original_live_anchor(ProcessIdentity {
            birth: original.birth + 1,
            ..original
        }));
        assert!(!is_original_live_anchor(ProcessIdentity {
            parent: original.parent + 1,
            ..original
        }));
        let other = FixtureGroup::new();
        assert!(!is_original_live_anchor(ProcessIdentity {
            group: other.identity.unwrap().group,
            ..original
        }));
        let mut expired = Command::new("/bin/sleep")
            .arg("20")
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let expired_identity = observe_identity(expired.id());
        let originally_live = expired_identity.is_some_and(is_original_live_anchor);
        // A scheduler pause can already have exhausted this direct child's TTL.
        // Child still owns the original process; reap it before any assertion.
        let _ = expired.kill();
        expired
            .wait()
            .expect("Reap the owned direct expiry fixture");
        let expired_identity = expired_identity.expect("Capture the direct expiry fixture");
        assert!(originally_live);
        assert!(!is_original_live_anchor(expired_identity));
    }

    #[test]
    fn an_invalid_pipe_descriptor_is_rejected_without_a_read() {
        assert!(set_orientation_pipe_nonblocking(-1).is_err());
    }
}
