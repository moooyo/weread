//! Request and acknowledgment state for process-resident managed suspension.

use std::sync::atomic::{AtomicU8, Ordering};

const REQUESTED: u8 = 1;
const ACKNOWLEDGED: u8 = 2;
const TERMINATED: u8 = 4;
const TRANSITIONING: u8 = 8;

/// A request is distinct from the backend's completed input/display handoff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagedSleepState {
    Running,
    Suspending,
    Suspended,
    Resuming,
    Terminated,
}

/// One atomic snapshot prevents requests and backend acknowledgments from
/// erasing each other when a power transition is reversed before it finishes.
pub(crate) struct ManagedSleepControl {
    packed: AtomicU8,
}

impl Default for ManagedSleepControl {
    fn default() -> Self {
        Self {
            packed: AtomicU8::new(0),
        }
    }
}

impl ManagedSleepControl {
    /// Change the desired state. The caller only needs to wake the event loop
    /// when this returns true. Termination never permits a new request.
    pub(crate) fn request(&self, suspended: bool) -> bool {
        let mut current = self.packed.load(Ordering::Acquire);
        loop {
            if current & TERMINATED != 0 {
                return false;
            }
            let next = if suspended {
                current | REQUESTED
            } else {
                current & !REQUESTED
            };
            if next == current {
                return false;
            }
            match self.packed.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(observed) => current = observed,
            }
        }
    }

    pub(crate) fn state(&self) -> ManagedSleepState {
        let packed = self.packed.load(Ordering::Acquire);
        if packed & TERMINATED != 0 {
            return ManagedSleepState::Terminated;
        }
        if packed & TRANSITIONING != 0 {
            return if packed & REQUESTED != 0 {
                ManagedSleepState::Suspending
            } else {
                ManagedSleepState::Resuming
            };
        }
        match packed & (REQUESTED | ACKNOWLEDGED) {
            0 => ManagedSleepState::Running,
            REQUESTED => ManagedSleepState::Suspending,
            ACKNOWLEDGED => ManagedSleepState::Resuming,
            _ => ManagedSleepState::Suspended,
        }
    }

    /// Whether the backend has released touch and stopped framebuffer writes.
    pub(crate) fn suspended(&self) -> bool {
        let packed = self.packed.load(Ordering::Acquire);
        packed & ACKNOWLEDGED != 0 && packed & TRANSITIONING == 0
    }

    pub(crate) fn requested(&self) -> bool {
        self.packed.load(Ordering::Acquire) & REQUESTED != 0
    }

    /// Hide any old acknowledgment before changing touch or display ownership.
    /// The backend must reread the desired state after beginning the transition.
    pub(crate) fn begin_transition(&self) {
        let _ = self
            .packed
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                if current & TERMINATED != 0 {
                    None
                } else {
                    Some(current | TRANSITIONING)
                }
            });
    }

    /// Commit a completed handoff without changing a newer desired state.
    pub(crate) fn acknowledge_suspended(&self) {
        self.acknowledge(true);
    }

    /// Commit completed touch reacquisition and frame restoration, preserving
    /// any newer suspension request that arrived during that work.
    pub(crate) fn acknowledge_running(&self) {
        self.acknowledge(false);
    }

    fn acknowledge(&self, suspended: bool) {
        let _ = self
            .packed
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                if current & TERMINATED != 0 {
                    return None;
                }
                let completed = current & !TRANSITIONING;
                Some(if suspended {
                    completed | ACKNOWLEDGED
                } else {
                    completed & !ACKNOWLEDGED
                })
            });
    }

    pub(crate) fn terminate(&self) {
        self.packed.fetch_or(TERMINATED, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn requests_are_idempotent_and_acknowledgments_complete_each_transition() {
        let control = ManagedSleepControl::default();
        assert_eq!(control.state(), ManagedSleepState::Running);
        assert!(!control.requested());
        assert!(!control.suspended());
        assert!(!control.request(false));

        assert!(control.request(true));
        assert!(!control.request(true));
        assert_eq!(control.state(), ManagedSleepState::Suspending);
        assert!(control.requested());
        assert!(!control.suspended());

        control.acknowledge_suspended();
        control.acknowledge_suspended();
        assert_eq!(control.state(), ManagedSleepState::Suspended);
        assert!(control.requested());
        assert!(control.suspended());

        assert!(control.request(false));
        assert!(!control.request(false));
        assert_eq!(control.state(), ManagedSleepState::Resuming);
        assert!(!control.requested());
        assert!(control.suspended());

        control.acknowledge_running();
        control.acknowledge_running();
        assert_eq!(control.state(), ManagedSleepState::Running);
        assert!(!control.requested());
        assert!(!control.suspended());
    }

    #[test]
    fn a_late_suspend_acknowledgment_preserves_a_reversed_request() {
        let control = ManagedSleepControl::default();
        assert!(control.request(true));
        assert!(control.request(false));
        assert_eq!(control.state(), ManagedSleepState::Running);

        control.acknowledge_suspended();
        assert_eq!(control.state(), ManagedSleepState::Resuming);
        assert!(!control.requested());
        assert!(control.suspended());

        control.acknowledge_running();
        assert_eq!(control.state(), ManagedSleepState::Running);
    }

    #[test]
    fn a_late_resume_acknowledgment_preserves_a_new_suspend_request() {
        let control = ManagedSleepControl::default();
        assert!(control.request(true));
        control.acknowledge_suspended();
        assert!(control.request(false));
        assert!(control.request(true));
        assert_eq!(control.state(), ManagedSleepState::Suspended);

        control.acknowledge_running();
        assert_eq!(control.state(), ManagedSleepState::Suspending);
        assert!(control.requested());
        assert!(!control.suspended());

        control.acknowledge_suspended();
        assert_eq!(control.state(), ManagedSleepState::Suspended);
    }

    #[test]
    fn a_reversed_pause_request_cannot_publish_running_during_ownership_changes() {
        let control = ManagedSleepControl::default();
        assert!(control.request(true));
        control.begin_transition();
        assert_eq!(control.state(), ManagedSleepState::Suspending);
        assert!(!control.suspended());

        assert!(control.request(false));
        assert_eq!(control.state(), ManagedSleepState::Resuming);
        assert!(!control.suspended());

        control.acknowledge_suspended();
        assert_eq!(control.state(), ManagedSleepState::Resuming);
        assert!(control.suspended());
        assert!(!control.requested());

        control.begin_transition();
        assert!(!control.suspended());
        control.acknowledge_running();
        assert_eq!(control.state(), ManagedSleepState::Running);
    }

    #[test]
    fn a_reversed_resume_request_cannot_publish_suspended_during_ownership_changes() {
        let control = ManagedSleepControl::default();
        control.request(true);
        control.acknowledge_suspended();
        assert!(control.request(false));
        control.begin_transition();
        assert_eq!(control.state(), ManagedSleepState::Resuming);
        assert!(!control.suspended());

        assert!(control.request(true));
        assert_eq!(control.state(), ManagedSleepState::Suspending);
        assert!(!control.suspended());

        control.acknowledge_running();
        assert_eq!(control.state(), ManagedSleepState::Suspending);
        assert!(!control.suspended());
        assert!(control.requested());

        control.begin_transition();
        control.acknowledge_suspended();
        assert_eq!(control.state(), ManagedSleepState::Suspended);
        assert!(control.suspended());
    }

    #[test]
    fn termination_prevents_late_transitions_and_acknowledgments() {
        let control = ManagedSleepControl::default();
        control.request(true);
        control.acknowledge_suspended();
        control.request(false);
        control.begin_transition();
        assert!(!control.suspended());

        control.terminate();
        let terminated = control.packed.load(Ordering::Acquire);
        control.begin_transition();
        control.acknowledge_running();
        control.acknowledge_suspended();
        assert!(!control.request(true));
        assert!(!control.request(false));
        assert_eq!(control.packed.load(Ordering::Acquire), terminated);
        assert_eq!(control.state(), ManagedSleepState::Terminated);
    }

    #[test]
    fn termination_is_irreversible_from_every_live_state() {
        for state in [
            ManagedSleepState::Running,
            ManagedSleepState::Suspending,
            ManagedSleepState::Suspended,
            ManagedSleepState::Resuming,
        ] {
            let control = ManagedSleepControl::default();
            if matches!(
                state,
                ManagedSleepState::Suspending | ManagedSleepState::Suspended
            ) {
                control.request(true);
            }
            if matches!(
                state,
                ManagedSleepState::Suspended | ManagedSleepState::Resuming
            ) {
                control.acknowledge_suspended();
            }
            assert_eq!(control.state(), state);
            let requested = control.requested();
            let suspended = control.suspended();

            control.terminate();
            control.terminate();
            assert!(!control.request(true));
            assert!(!control.request(false));
            control.acknowledge_running();
            control.acknowledge_suspended();
            assert_eq!(control.state(), ManagedSleepState::Terminated);
            assert_eq!(control.requested(), requested);
            assert_eq!(control.suspended(), suspended);
        }
    }

    #[test]
    fn concurrent_request_and_acknowledgment_keep_both_bits() {
        let control = Arc::new(ManagedSleepControl::default());
        let barrier = Arc::new(Barrier::new(3));
        let requester = {
            let control = control.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                assert!(control.request(true));
            })
        };
        let acknowledger = {
            let control = control.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                control.acknowledge_suspended();
            })
        };
        barrier.wait();
        requester.join().unwrap();
        acknowledger.join().unwrap();
        assert_eq!(control.state(), ManagedSleepState::Suspended);
        assert!(control.requested());
        assert!(control.suspended());
    }

    #[test]
    fn repeated_cycles_return_to_the_same_running_state() {
        let control = ManagedSleepControl::default();
        for _ in 0..128 {
            assert!(control.request(true));
            assert_eq!(control.state(), ManagedSleepState::Suspending);
            control.acknowledge_suspended();
            assert_eq!(control.state(), ManagedSleepState::Suspended);
            assert!(control.request(false));
            assert_eq!(control.state(), ManagedSleepState::Resuming);
            control.acknowledge_running();
            assert_eq!(control.state(), ManagedSleepState::Running);
        }
    }
}
