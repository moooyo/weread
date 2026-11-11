//! Completion tokens for application-owned sleep frames.

use std::sync::Mutex;

use crate::render_control::RenderError;

#[derive(Default)]
struct State {
    requested: u64,
    completed: u64,
    cancelled_through: u64,
    terminated: bool,
}

/// A token describes a full frame submitted after its request. Requests issued
/// before the same draw may coalesce; a later request needs a later draw.
#[derive(Default)]
pub(crate) struct SleepPresentationControl {
    state: Mutex<State>,
}

impl SleepPresentationControl {
    pub(crate) fn request(&self) -> Result<u64, RenderError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RenderError::PresentationUnavailable)?;
        if state.terminated {
            return Err(RenderError::EventLoopTerminated);
        }
        state.requested = state
            .requested
            .checked_add(1)
            .ok_or(RenderError::PresentationSequenceExhausted)?;
        Ok(state.requested)
    }

    pub(crate) fn pending(&self) -> Result<Option<u64>, RenderError> {
        let state = self
            .state
            .lock()
            .map_err(|_| RenderError::PresentationUnavailable)?;
        if state.terminated {
            return Err(RenderError::EventLoopTerminated);
        }
        Ok(
            (state.requested > state.completed && state.requested > state.cancelled_through)
                .then_some(state.requested),
        )
    }

    pub(crate) fn completed(&self, token: u64) -> Result<bool, RenderError> {
        let state = self
            .state
            .lock()
            .map_err(|_| RenderError::PresentationUnavailable)?;
        if state.terminated {
            return Err(RenderError::EventLoopTerminated);
        }
        if token == 0 || token > state.requested {
            return Err(RenderError::InvalidPresentationToken);
        }
        if token <= state.cancelled_through {
            return Err(RenderError::PresentationUnavailable);
        }
        Ok(token <= state.completed)
    }

    /// Only call after Slint rendered the captured token's frame and the EPDC
    /// accepted and completed its full update. Never sample a newer token here.
    pub(crate) fn acknowledge(&self, token: u64) -> Result<(), RenderError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RenderError::PresentationUnavailable)?;
        if state.terminated {
            return Err(RenderError::EventLoopTerminated);
        }
        if token == 0 || token > state.requested {
            return Err(RenderError::InvalidPresentationToken);
        }
        if token <= state.cancelled_through {
            return Err(RenderError::PresentationUnavailable);
        }
        state.completed = state.completed.max(token);
        Ok(())
    }

    /// A native handoff must not allow its later wake frame to complete an
    /// unfinished sleep presentation. Completed requests remain observable when
    /// the handoff did not interrupt a newer pending request.
    pub(crate) fn cancel_pending(&self) {
        if let Ok(mut state) = self.state.lock() {
            if state.requested > state.completed {
                state.cancelled_through = state.requested;
            }
        }
    }

    pub(crate) fn terminate(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.terminated = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_coalesce_only_requests_issued_before_the_captured_frame() {
        let control = SleepPresentationControl::default();
        assert_eq!(control.pending().unwrap(), None);
        let first = control.request().unwrap();
        let second = control.request().unwrap();
        assert_eq!((first, second), (1, 2));
        assert!(!control.completed(first).unwrap());
        let captured = control.pending().unwrap().unwrap();
        let third = control.request().unwrap();
        control.acknowledge(captured).unwrap();
        assert!(control.completed(first).unwrap());
        assert!(control.completed(second).unwrap());
        assert!(!control.completed(third).unwrap());
        assert_eq!(control.pending().unwrap(), Some(third));
        control.acknowledge(third).unwrap();
        assert!(control.completed(third).unwrap());
        assert_eq!(control.pending().unwrap(), None);
    }

    #[test]
    fn an_interrupted_presentation_cannot_be_acknowledged_by_the_wake_frame() {
        let control = SleepPresentationControl::default();
        let interrupted = control.request().unwrap();
        control.cancel_pending();
        assert_eq!(control.pending().unwrap(), None);
        assert_eq!(
            control.acknowledge(interrupted),
            Err(RenderError::PresentationUnavailable)
        );
        assert_eq!(
            control.completed(interrupted),
            Err(RenderError::PresentationUnavailable)
        );
        let fresh = control.request().unwrap();
        assert!(fresh > interrupted);
        control.acknowledge(fresh).unwrap();
        assert!(control.completed(fresh).unwrap());
    }

    #[test]
    fn invalid_tokens_overflow_and_termination_never_report_success() {
        let control = SleepPresentationControl::default();
        assert_eq!(
            control.completed(0),
            Err(RenderError::InvalidPresentationToken)
        );
        assert_eq!(
            control.completed(1),
            Err(RenderError::InvalidPresentationToken)
        );
        control.state.lock().unwrap().requested = u64::MAX;
        assert_eq!(
            control.request(),
            Err(RenderError::PresentationSequenceExhausted)
        );
        assert_eq!(control.pending().unwrap(), Some(u64::MAX));
        control.acknowledge(u64::MAX).unwrap();
        assert!(control.completed(u64::MAX).unwrap());
        control.terminate();
        assert_eq!(control.request(), Err(RenderError::EventLoopTerminated));
        assert_eq!(control.pending(), Err(RenderError::EventLoopTerminated));
        assert_eq!(
            control.completed(u64::MAX),
            Err(RenderError::EventLoopTerminated)
        );
        assert_eq!(
            control.acknowledge(u64::MAX),
            Err(RenderError::EventLoopTerminated)
        );
    }
}
