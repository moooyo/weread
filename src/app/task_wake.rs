//! One owned wake-maintenance worker across task cancellation and replacement.

use super::*;
use crate::platform::{PlatformError, WakeGuard};

pub(super) const STOP_TIMEOUT: Duration = Duration::from_millis(2250);

#[derive(Default)]
pub(super) struct TaskWakeMaintenance {
    guard: Option<WakeGuard>,
    requested: bool,
    stopping: bool,
    failure_reported: bool,
    terminal_cleanup_managed: bool,
}

impl TaskWakeMaintenance {
    /// App owns the one terminal join budget; field Drop must not add another.
    pub(super) fn prepare_terminal_cleanup(&mut self) {
        self.terminal_cleanup_managed = true;
        self.retire();
    }

    fn request_start(&mut self) {
        self.requested = true;
    }

    fn reap_stopping(&mut self, timeout: Duration) -> Result<(), PlatformError> {
        if !self.stopping {
            return Ok(());
        }
        if let Some(guard) = self.guard.as_mut() {
            guard.shutdown(timeout)?;
        }
        self.guard = None;
        self.stopping = false;
        Ok(())
    }

    fn retire(&mut self) {
        self.requested = false;
        self.stopping = self.guard.is_some();
        // A normal cancellation must not block UI dispatch. A timeout keeps the
        // sole handle; a later task cannot replace it before a successful join.
        let _ = self.reap_stopping(Duration::ZERO);
    }

    fn poll(&mut self, platform: &Platform, can_start: bool) -> bool {
        let mut failed = false;
        if self.guard.as_ref().and_then(WakeGuard::failure).is_some() {
            if !self.failure_reported {
                self.failure_reported = true;
                failed = true;
            }
            if !self.stopping {
                self.requested = false;
            }
            self.stopping = true;
        }
        let _ = self.reap_stopping(Duration::ZERO);
        if self.guard.is_none() && self.requested && can_start {
            match platform.hold_awake_deferred() {
                Ok(guard) => {
                    self.guard = Some(guard);
                    self.failure_reported = false;
                }
                Err(error) => {
                    self.requested = false;
                    crate::diagnostic!("Start task wake maintenance failed: {error}");
                    failed = true;
                }
            }
        }
        failed
    }

    fn stop_and_join(&mut self, timeout: Duration) -> Result<(), PlatformError> {
        self.requested = false;
        self.stopping = self.guard.is_some();
        self.reap_stopping(timeout)
    }
}

impl Drop for TaskWakeMaintenance {
    fn drop(&mut self) {
        // A standalone owner gets one bounded join. App-managed destruction
        // only retries immediately after the App's shared terminal budget.
        let timeout = if self.terminal_cleanup_managed {
            Duration::ZERO
        } else {
            STOP_TIMEOUT
        };
        let _ = self.stop_and_join(timeout);
    }
}

impl App {
    fn task_wake_is_eligible(&self) -> bool {
        self.current_download
            .as_ref()
            .is_some_and(|task| task.status == DownloadStatus::Running)
            && self.prefs.keep_awake
            && self.platform.capabilities().keep_awake
            && !self.closing.get()
    }

    pub(super) fn start_task_wake_guard(&mut self) {
        if self.task_wake_is_eligible() {
            self.task_wake.request_start();
        } else {
            self.task_wake.retire();
        }
        self.poll_task_wake_guard();
    }

    pub(super) fn retire_task_wake_guard(&mut self) {
        self.task_wake.retire();
    }

    pub(super) fn poll_task_wake_guard(&mut self) {
        let eligible = self.task_wake_is_eligible();
        if !eligible {
            self.task_wake.retire();
        }
        // A resume may queue its next download before dropping the input guard.
        // Preserve the request, but create no worker until ordinary ticks resume.
        if self.task_wake.poll(
            self.platform.as_ref(),
            eligible && !self.managed_sleep_blocked,
        ) {
            self.show_toast("keep_awake_failed");
        }
    }

    pub(super) fn join_task_wake_guard(&mut self, timeout: Duration) -> Result<()> {
        self.task_wake.stop_and_join(timeout)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{DeviceControl, PlatformCapabilities};
    use std::sync::{
        Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Default)]
    struct ResetGate {
        released: Mutex<bool>,
        changed: Condvar,
        calls: AtomicUsize,
        active: AtomicUsize,
        maximum_active: AtomicUsize,
    }

    impl ResetGate {
        fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.changed.notify_all();
        }

        fn wait_for_calls(&self, count: usize) {
            let deadline = Instant::now() + Duration::from_secs(2);
            while self.calls.load(Ordering::Acquire) < count {
                assert!(
                    Instant::now() < deadline,
                    "The wake worker did not enter the fixture"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    struct ReleaseOnDrop(Arc<ResetGate>);

    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            self.0.release();
        }
    }

    struct FakeControl(Arc<ResetGate>);

    impl DeviceControl for FakeControl {
        fn capabilities(&self) -> PlatformCapabilities {
            PlatformCapabilities {
                keep_awake: true,
                ..PlatformCapabilities::default()
            }
        }

        fn brightness(&self) -> Result<u8, PlatformError> {
            Err(PlatformError::Unsupported("fixture_brightness"))
        }

        fn set_brightness(&self, _: u8) -> Result<(), PlatformError> {
            Err(PlatformError::Unsupported("fixture_brightness"))
        }

        fn warmth(&self) -> Result<u8, PlatformError> {
            Err(PlatformError::Unsupported("fixture_warmth"))
        }

        fn set_warmth(&self, _: u8) -> Result<(), PlatformError> {
            Err(PlatformError::Unsupported("fixture_warmth"))
        }

        fn battery_percentage(&self) -> Result<u8, PlatformError> {
            Err(PlatformError::Unsupported("fixture_battery"))
        }

        fn set_inverted(&self, _: bool) -> Result<(), PlatformError> {
            Err(PlatformError::Unsupported("fixture_inversion"))
        }

        fn reset_idle_timer(&self) -> Result<(), PlatformError> {
            let active = self.0.active.fetch_add(1, Ordering::AcqRel) + 1;
            self.0.maximum_active.fetch_max(active, Ordering::AcqRel);
            self.0.calls.fetch_add(1, Ordering::AcqRel);
            let mut released = self.0.released.lock().unwrap();
            while !*released {
                released = self.0.changed.wait(released).unwrap();
            }
            self.0.active.fetch_sub(1, Ordering::AcqRel);
            Ok(())
        }
    }

    fn fixture(gate: Arc<ResetGate>) -> Platform {
        Platform::with_control(DeviceProfile::Scribe, false, Arc::new(FakeControl(gate)))
    }

    #[test]
    fn repeated_cancellation_keeps_one_worker_and_restarts_only_after_join() {
        let gate = Arc::new(ResetGate::default());
        let platform = fixture(gate.clone());
        let mut wake = TaskWakeMaintenance::default();
        let _release = ReleaseOnDrop(gate.clone());
        wake.request_start();
        assert!(!wake.poll(&platform, true));
        gate.wait_for_calls(1);

        for _ in 0..1024 {
            wake.retire();
            wake.request_start();
            assert!(!wake.poll(&platform, true));
            assert!(wake.guard.is_some() && wake.stopping);
        }
        assert_eq!(gate.calls.load(Ordering::Acquire), 1);
        assert_eq!(gate.maximum_active.load(Ordering::Acquire), 1);
        assert!(wake.stop_and_join(Duration::ZERO).is_err());
        assert!(wake.guard.is_some());

        gate.release();
        wake.stop_and_join(Duration::from_secs(1)).unwrap();
        assert!(wake.guard.is_none());
        assert_eq!(gate.active.load(Ordering::Acquire), 0);
        wake.request_start();
        assert!(!wake.poll(&platform, true));
        gate.wait_for_calls(2);
        wake.stop_and_join(Duration::from_secs(1)).unwrap();
        assert_eq!(gate.maximum_active.load(Ordering::Acquire), 1);
    }

    #[test]
    fn a_resume_request_stays_deferred_until_native_handoff_is_complete() {
        let gate = Arc::new(ResetGate::default());
        let platform = fixture(gate.clone());
        let mut wake = TaskWakeMaintenance::default();
        let _release = ReleaseOnDrop(gate.clone());
        wake.request_start();
        for _ in 0..1024 {
            assert!(!wake.poll(&platform, false));
        }
        assert!(wake.guard.is_none());
        assert_eq!(gate.calls.load(Ordering::Acquire), 0);
        assert!(!wake.poll(&platform, true));
        gate.wait_for_calls(1);
        gate.release();
        wake.stop_and_join(Duration::from_secs(1)).unwrap();
    }
}
