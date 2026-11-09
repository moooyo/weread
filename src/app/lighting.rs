//! Serialized Reader lighting status and control.

use super::lighting_worker::{
    LightKind, LightRequest, LightResult, LightWorker, WorkerCompletion, join_worker,
};
use super::*;
use crate::platform::PlatformError;
use std::sync::atomic::{AtomicBool, Ordering};

const LIGHT_STOP_DEADLINE: Duration = Duration::from_millis(2250);

/// Application appearance is persisted independently of the native output base.
pub(super) fn restore_dark_mode(platform: &Platform, prefs: &AppPreferences) -> Result<()> {
    if platform.capabilities().application_dark_mode {
        platform.set_dark_mode(prefs.dark_mode)?;
    }
    Ok(())
}

struct PendingLight {
    id: u64,
}

struct BatteryRead {
    cancelled: Arc<AtomicBool>,
    reply: mpsc::Receiver<std::result::Result<u8, PlatformError>>,
    finished: mpsc::Receiver<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl BatteryRead {
    fn start(platform: Arc<Platform>) -> std::result::Result<Self, PlatformError> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stopped = cancelled.clone();
        let (sender, reply) = mpsc::channel();
        let (completed, finished) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("weread-battery-status".into())
            .spawn(move || {
                let _completed = WorkerCompletion::new(completed);
                if stopped.load(Ordering::Acquire) {
                    return;
                }
                let value = platform.battery_percentage();
                if !stopped.load(Ordering::Acquire) {
                    let _ = sender.send(value);
                }
            })?;
        Ok(Self {
            cancelled,
            reply,
            finished,
            thread: Some(thread),
        })
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn shutdown(&mut self, timeout: Duration) -> std::result::Result<(), PlatformError> {
        self.cancel();
        join_worker(
            &mut self.thread,
            &self.finished,
            timeout,
            "battery-status worker",
        )
    }

    fn is_finished(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(|thread| thread.is_finished())
    }
}

impl Drop for BatteryRead {
    fn drop(&mut self) {
        // Ordinary polling and destruction never wait on a device command.
        // Managed lifecycle callers require shutdown before dropping this handle.
        self.cancel();
    }
}

#[derive(Default)]
pub(super) struct LightingState {
    brightness: Option<u8>,
    warmth: Option<u8>,
    desired_brightness: Option<u8>,
    desired_warmth: Option<u8>,
    pending_target: Option<(LightKind, u8)>,
    refresh_requested: bool,
    sequence: u64,
    pending: Option<PendingLight>,
    worker: Option<LightWorker>,
    stopping: bool,
    battery: Option<BatteryRead>,
    status: String,
}
impl LightingState {
    #[cfg(test)]
    fn busy(&self) -> bool {
        self.pending.is_some()
            || self.desired_brightness.is_some()
            || self.desired_warmth.is_some()
            || self.refresh_requested
            || self.stopping
            || self.battery.is_some()
    }
    fn value(&self, kind: LightKind) -> Option<u8> {
        match kind {
            LightKind::Brightness => self.brightness,
            LightKind::Warmth => self.warmth,
        }
    }
    fn target(&self, kind: LightKind) -> Option<u8> {
        let queued = match kind {
            LightKind::Brightness => self.desired_brightness,
            LightKind::Warmth => self.desired_warmth,
        };
        queued
            .or_else(|| {
                self.pending_target
                    .filter(|(pending_kind, _)| *pending_kind == kind)
                    .map(|(_, value)| value)
            })
            .or_else(|| self.value(kind))
    }
}

impl App {
    pub(super) fn toggle_dark_mode(&mut self) -> Result<()> {
        if !self.platform.capabilities().application_dark_mode {
            self.show_toast("application_dark_mode_unavailable");
            return Ok(());
        }
        let previous = self.prefs.dark_mode;
        let enabled = !previous;
        self.platform.set_dark_mode(enabled)?;
        self.prefs.dark_mode = enabled;
        if let Err(error) = self.save_preferences() {
            self.prefs.dark_mode = previous;
            if let Err(rollback) = self.platform.set_dark_mode(previous) {
                crate::diagnostic!(
                    "Restore application dark mode after a save failure failed: {rollback:#}"
                );
            }
            return Err(error);
        }
        self.show_toast(if enabled {
            "dark_mode_on"
        } else {
            "dark_mode_off"
        });
        Ok(())
    }

    /// Observe the current native base without changing the saved application
    /// preference. Stage the same desired appearance before wake reacquires input.
    pub(super) fn synchronize_native_output_base(&mut self, inverted: bool) -> Result<()> {
        self.platform.set_native_output_inverted(inverted)?;
        self.platform.set_dark_mode(self.prefs.dark_mode)?;
        self.render()
    }

    pub(super) fn reader_is_active_page(&self) -> bool {
        matches!(self.route, Route::Reader(..))
    }
    pub(super) fn app_status_bar_height(&self) -> u32 {
        28
    }
    pub(super) fn search_viewport_size(&self) -> (u32, u32) {
        self.platform.logical_viewport_size()
    }

    fn lighting_failed(&mut self) {
        self.lighting.status = tr("hardware_unavailable");
        self.show_toast("hardware_unavailable");
    }

    pub(super) fn request_device_light_refresh(&mut self) {
        if self.closing.get() || self.lighting.stopping {
            return;
        }
        let capabilities = self.platform.capabilities();
        if capabilities.brightness || capabilities.warmth {
            self.lighting.refresh_requested = true;
            self.pump_device_light();
        }
    }

    pub(super) fn request_device_status(&mut self) {
        self.request_device_light_refresh();
        if self.closing.get()
            || self.lighting.stopping
            || !self.platform.is_kindle
            || self.lighting.battery.is_some()
        {
            return;
        }
        // Battery metadata is read-only and does not occupy the lighting queue.
        if let Ok(battery) = BatteryRead::start(self.platform.clone()) {
            self.lighting.battery = Some(battery);
        }
    }

    pub(super) fn change_device_light(&mut self, kind: &str, value: i32, absolute: bool) {
        if self.closing.get() || self.lighting.stopping {
            return;
        }
        let kind = match kind {
            "brightness" => LightKind::Brightness,
            "warmth" => LightKind::Warmth,
            _ => return,
        };
        let capabilities = self.platform.capabilities();
        let supported = match kind {
            LightKind::Brightness => capabilities.brightness,
            LightKind::Warmth => capabilities.warmth,
        };
        let Some(current) = self.lighting.target(kind).filter(|_| supported) else {
            self.lighting_failed();
            return;
        };
        let target = if absolute && (0..=24).contains(&value) {
            value as u8
        } else if !absolute && matches!(value, -1 | 1) {
            (i32::from(current) + value).clamp(0, 24) as u8
        } else {
            self.lighting_failed();
            return;
        };
        match kind {
            LightKind::Brightness => self.lighting.desired_brightness = Some(target),
            LightKind::Warmth => self.lighting.desired_warmth = Some(target),
        }
        self.pump_device_light();
    }

    fn pump_device_light(&mut self) -> bool {
        if self.closing.get() || self.lighting.stopping || self.lighting.pending.is_some() {
            return false;
        }
        let mut request = None;
        for kind in [LightKind::Brightness, LightKind::Warmth] {
            let desired = match kind {
                LightKind::Brightness => self.lighting.desired_brightness.take(),
                LightKind::Warmth => self.lighting.desired_warmth.take(),
            };
            if let Some(value) = desired.filter(|value| Some(*value) != self.lighting.value(kind)) {
                request = Some(LightRequest::Set { kind, value });
                break;
            }
        }
        if request.is_none() && self.lighting.refresh_requested {
            self.lighting.refresh_requested = false;
            let capabilities = self.platform.capabilities();
            if capabilities.brightness || capabilities.warmth {
                request = Some(LightRequest::Query {
                    brightness: capabilities.brightness,
                    warmth: capabilities.warmth,
                });
            }
        }
        let Some(request) = request else {
            return false;
        };
        let started = (|| -> std::result::Result<(), PlatformError> {
            if self.lighting.worker.is_none() {
                self.lighting.worker = Some(LightWorker::new(self.platform.clone())?);
            }
            self.lighting.sequence = self.lighting.sequence.wrapping_add(1);
            let id = self.lighting.sequence;
            self.lighting.worker.as_ref().unwrap().send(id, request)?;
            self.lighting.pending = Some(PendingLight { id });
            self.lighting.pending_target = match request {
                LightRequest::Set { kind, value } => Some((kind, value)),
                _ => None,
            };
            Ok(())
        })();
        if let Err(error) = started {
            crate::diagnostic!("Start device lighting operation failed: {error}");
            self.lighting.worker = None;
            self.lighting_failed();
        } else {
            self.lighting.status.clear();
        }
        true
    }

    pub(super) fn poll_lighting(&mut self) -> bool {
        if self.closing.get() {
            return false;
        }
        let mut changed = false;
        if self.lighting.stopping {
            if self.join_lighting_workers(Duration::ZERO).is_ok() {
                self.lighting.pending = None;
                self.lighting.pending_target = None;
                self.lighting.stopping = false;
                self.lighting.brightness = None;
                self.lighting.warmth = None;
                let capabilities = self.platform.capabilities();
                self.lighting.refresh_requested = capabilities.brightness || capabilities.warmth;
                changed = true;
            }
        } else {
            loop {
                let reply = self
                    .lighting
                    .worker
                    .as_ref()
                    .map(|worker| worker.try_reply());
                match reply {
                    Some(Ok(Some(reply))) => {
                        if self
                            .lighting
                            .pending
                            .as_ref()
                            .is_none_or(|pending| pending.id != reply.id)
                        {
                            continue;
                        }
                        self.lighting.pending = None;
                        self.lighting.pending_target = None;
                        match reply.result {
                            LightResult::Query { brightness, warmth } => {
                                let failed =
                                    brightness.as_ref().is_some_and(|result| result.is_err())
                                        || warmth.as_ref().is_some_and(|result| result.is_err());
                                self.lighting.brightness =
                                    brightness.and_then(|result| result.ok());
                                self.lighting.warmth = warmth.and_then(|result| result.ok());
                                if failed {
                                    self.lighting.status = tr("hardware_unavailable");
                                } else {
                                    self.lighting.status.clear();
                                }
                            }
                            LightResult::Set {
                                kind,
                                value,
                                result,
                            } => {
                                if result.is_ok() {
                                    match kind {
                                        LightKind::Brightness => {
                                            self.lighting.brightness = Some(value)
                                        }
                                        LightKind::Warmth => self.lighting.warmth = Some(value),
                                    }
                                    self.lighting.status.clear();
                                } else {
                                    self.lighting_failed();
                                }
                            }
                        }
                        changed = true;
                    }
                    Some(Err(error)) => {
                        crate::diagnostic!("Receive device lighting result failed: {error}");
                        self.lighting.worker = None;
                        self.lighting.pending = None;
                        self.lighting.pending_target = None;
                        self.lighting_failed();
                        changed = true;
                        break;
                    }
                    _ => break,
                }
            }
        }
        if !self.lighting.stopping {
            let result = self.lighting.battery.as_ref().map(|battery| {
                let mut reply = battery.reply.try_recv();
                let finished = battery.is_finished();
                if finished && matches!(&reply, Err(mpsc::TryRecvError::Empty)) {
                    // A reply may arrive between the first read and the exit check.
                    // A terminated sender cannot add another value after this drain.
                    reply = battery.reply.try_recv();
                }
                (finished, battery.cancelled.load(Ordering::Acquire), reply)
            });
            if let Some((finished, cancelled, reply)) = result {
                if let Ok(Ok(value)) = reply {
                    if !cancelled && value <= 100 {
                        if let Some(ui) = self.ui.upgrade() {
                            ui.global::<ShellModel>()
                                .set_battery_label(format!("{value}%").into());
                        }
                    }
                }
                if finished {
                    if let Some(battery) = self.lighting.battery.as_mut() {
                        if let Err(error) = battery.shutdown(Duration::ZERO) {
                            crate::diagnostic!("Join battery-status operation failed: {error}");
                        }
                    }
                    self.lighting.battery = None;
                }
            }
        }
        changed | self.pump_device_light()
    }

    pub(super) fn stop_lighting_hardware(&mut self) -> Result<()> {
        self.stop_lighting_hardware_with_timeout(LIGHT_STOP_DEADLINE)
    }

    pub(super) fn request_lighting_stop(&mut self) {
        self.lighting.desired_brightness = None;
        self.lighting.desired_warmth = None;
        self.lighting.refresh_requested = false;
        self.lighting.stopping = true;
        if let Some(battery) = self.lighting.battery.as_ref() {
            battery.cancel();
        }
        if let Some(worker) = self.lighting.worker.as_ref() {
            worker.request_stop();
        }
    }

    pub(super) fn stop_lighting_hardware_with_timeout(&mut self, timeout: Duration) -> Result<()> {
        self.request_lighting_stop();
        self.join_lighting_workers(timeout)?;
        self.lighting.pending = None;
        self.lighting.pending_target = None;
        self.lighting.stopping = false;
        self.lighting.brightness = None;
        self.lighting.warmth = None;
        Ok(())
    }

    /// Cancel both hardware queues before spending one shared shutdown budget.
    fn join_lighting_workers(
        &mut self,
        timeout: Duration,
    ) -> std::result::Result<(), PlatformError> {
        let started = Instant::now();
        if let Some(battery) = self.lighting.battery.as_ref() {
            battery.cancel();
        }
        let light_error = self.lighting.worker.as_mut().and_then(|worker| {
            worker
                .shutdown(timeout.saturating_sub(started.elapsed()))
                .err()
        });
        let battery_error = self.lighting.battery.as_mut().and_then(|battery| {
            battery
                .shutdown(timeout.saturating_sub(started.elapsed()))
                .err()
        });
        if let Some(error) = light_error.or(battery_error) {
            return Err(error);
        }
        self.lighting.worker = None;
        self.lighting.battery = None;
        Ok(())
    }

    pub(super) fn render_lighting(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let capabilities = self.platform.capabilities();
        let reader = ui.global::<ReaderModel>();
        reader.set_brightness_known(self.lighting.brightness.is_some());
        reader.set_warmth_known(self.lighting.warmth.is_some());
        reader.set_hardware_light(capabilities.brightness && self.lighting.brightness.is_some());
        reader.set_hardware_warmth(capabilities.warmth && self.lighting.warmth.is_some());
        if let Some(value) = self.lighting.brightness {
            reader.set_brightness(i32::from(value));
        }
        if let Some(value) = self.lighting.warmth {
            reader.set_warmth(i32::from(value));
        }
    }
}

#[cfg(test)]
#[path = "lighting_tests.rs"]
mod tests;
