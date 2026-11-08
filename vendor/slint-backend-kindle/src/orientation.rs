//! Explicit native portrait calibration and process-local presentation requests.

use std::fmt;
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::managed_sleep::ManagedSleepControl;

/// A portrait holding or scanout direction, independent of application rotation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativePortraitOrientation {
    Up,
    Down,
}

impl NativePortraitOrientation {
    pub(crate) fn offset(self) -> u32 {
        match self {
            Self::Up => 0,
            Self::Down => 180,
        }
    }

    fn reversed(self) -> Self {
        match self {
            Self::Up => Self::Down,
            Self::Down => Self::Up,
        }
    }
}

impl From<crate::gyro::NativePortrait> for NativePortraitOrientation {
    fn from(value: crate::gyro::NativePortrait) -> Self {
        match value {
            crate::gyro::NativePortrait::Up => Self::Up,
            crate::gyro::NativePortrait::Down => Self::Down,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeOrientationError {
    EventLoopTerminated,
    ActiveDisplayOwnership,
    InvalidRotation,
    RequestUnavailable,
}

impl fmt::Display for NativeOrientationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EventLoopTerminated => "the Kindle event loop has terminated",
            Self::ActiveDisplayOwnership => "native calibration requires startup or suspended display ownership",
            Self::InvalidRotation => "native framebuffer rotation must be in 0..=3",
            Self::RequestUnavailable => "the native orientation request is unavailable",
        })
    }
}

impl std::error::Error for NativeOrientationError {}

/// A same-launch scanout/GETVAR pair supplied by the native display owner.
/// Absolute kernel rotation codes do not identify a direction across devices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PortraitCalibration {
    pub(crate) holding: NativePortraitOrientation,
    pub(crate) scanout: NativePortraitOrientation,
    pub(crate) raw_rotation: u32,
}

impl PortraitCalibration {
    fn scanout_at(self, raw_rotation: u32) -> Result<NativePortraitOrientation, NativeOrientationError> {
        if raw_rotation > 3 || self.raw_rotation > 3 {
            return Err(NativeOrientationError::InvalidRotation);
        }
        match (raw_rotation + 4 - self.raw_rotation) % 4 {
            0 => Ok(self.scanout),
            2 => Ok(self.scanout.reversed()),
            _ => Err(NativeOrientationError::InvalidRotation),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct OrientationRequest {
    pub(crate) sequence: u64,
    pub(crate) calibration: Option<PortraitCalibration>,
}

/// Requests an explicitly reviewed native scanout/raw-touch mapping.
/// The setter performs no device I/O and does not acknowledge a physical refresh.
#[derive(Clone)]
pub struct NativeOrientationHandle {
    pub(crate) request: Arc<Mutex<OrientationRequest>>,
    pub(crate) initialized: Arc<AtomicBool>,
    pub(crate) managed_sleep: Arc<ManagedSleepControl>,
    pub(crate) wake: Arc<OwnedFd>,
    pub(crate) quit: Arc<AtomicBool>,
}

impl NativeOrientationHandle {
    /// Enable event-driven portrait changes after a same-launch calibration.
    /// The caller authenticates the holding/scanout pair for its device and
    /// firmware, paired with a fresh read-only GETVAR observation. Updates are
    /// accepted before the event loop starts or while the native owner is asleep.
    pub fn set_calibrated_presentation(
        &self,
        holding: NativePortraitOrientation,
        scanout: NativePortraitOrientation,
        raw_rotation: u32,
    ) -> Result<(), NativeOrientationError> {
        if raw_rotation > 3 {
            return Err(NativeOrientationError::InvalidRotation);
        }
        if self.quit.load(Ordering::SeqCst) {
            return Err(NativeOrientationError::EventLoopTerminated);
        }
        if self.initialized.load(Ordering::Acquire) && !self.managed_sleep.suspended() {
            return Err(NativeOrientationError::ActiveDisplayOwnership);
        }
        let mut request = self.request.lock().map_err(|_| NativeOrientationError::RequestUnavailable)?;
        let calibration = PortraitCalibration { holding, scanout, raw_rotation };
        if request.calibration != Some(calibration) {
            request.calibration = Some(calibration);
            request.sequence = request.sequence.wrapping_add(1);
            crate::wakeup::poke(&self.wake);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PortraitMapping {
    pub(crate) holding: NativePortraitOrientation,
    pub(crate) scanout: NativePortraitOrientation,
}

impl PortraitMapping {
    pub(crate) fn render_offset(self) -> u32 {
        (self.holding.offset() + 360 - self.scanout.offset()) % 360
    }

    pub(crate) fn touch_offset(self) -> u32 {
        self.holding.offset()
    }
}

/// Event-loop-local state. Calibration stays fixed while observed holding and
/// scanout can change; application rotation is never changed by this controller.
pub(crate) struct PortraitController {
    sequence: u64,
    calibration: Option<PortraitCalibration>,
    mapping: Option<PortraitMapping>,
    legacy_render_offset: u32,
}

impl PortraitController {
    pub(crate) fn new(legacy_render_offset: u32) -> Self {
        Self { sequence: 0, calibration: None, mapping: None, legacy_render_offset }
    }

    pub(crate) fn calibrated(&self) -> bool {
        self.calibration.is_some()
    }

    pub(crate) fn render_offset(&self) -> u32 {
        self.mapping.map_or(self.legacy_render_offset, PortraitMapping::render_offset)
    }

    pub(crate) fn touch_offset(&self) -> u32 {
        self.mapping.map_or(0, PortraitMapping::touch_offset)
    }

    pub(crate) fn apply_request(
        &mut self,
        request: OrientationRequest,
        raw_rotation: u32,
    ) -> Result<bool, NativeOrientationError> {
        if request.sequence == self.sequence {
            return Ok(false);
        }
        let Some(calibration) = request.calibration else { return Ok(false) };
        let mapping = PortraitMapping {
            holding: calibration.holding,
            scanout: calibration.scanout_at(raw_rotation)?,
        };
        let changed = self.mapping != Some(mapping);
        self.sequence = request.sequence;
        self.calibration = Some(calibration);
        self.mapping = Some(mapping);
        Ok(changed)
    }

    pub(crate) fn observe(
        &mut self,
        holding: NativePortraitOrientation,
        raw_rotation: u32,
    ) -> Result<bool, NativeOrientationError> {
        let Some(calibration) = self.calibration else { return Ok(false) };
        let mapping = PortraitMapping { holding, scanout: calibration.scanout_at(raw_rotation)? };
        let changed = self.mapping != Some(mapping);
        self.mapping = Some(mapping);
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_same_launch_rotation_anchors_only_resolve_portrait_deltas() {
        for raw_rotation in 0..=3 {
            let calibration = PortraitCalibration {
                holding: NativePortraitOrientation::Up,
                scanout: NativePortraitOrientation::Up,
                raw_rotation,
            };
            assert_eq!(calibration.scanout_at(raw_rotation), Ok(NativePortraitOrientation::Up));
            assert_eq!(calibration.scanout_at((raw_rotation + 2) % 4), Ok(NativePortraitOrientation::Down));
            assert!(calibration.scanout_at((raw_rotation + 1) % 4).is_err());
            assert!(calibration.scanout_at((raw_rotation + 3) % 4).is_err());
            assert!(calibration.scanout_at(4).is_err());
        }
    }

    #[test]
    fn stale_scanout_compensates_pixels_and_raw_touch_independently() {
        let mut controller = PortraitController::new(180);
        assert_eq!((controller.render_offset(), controller.touch_offset()), (180, 0));
        let request = OrientationRequest {
            sequence: 1,
            calibration: Some(PortraitCalibration {
                holding: NativePortraitOrientation::Up,
                scanout: NativePortraitOrientation::Up,
                raw_rotation: 3,
            }),
        };
        assert!(controller.apply_request(request, 3).unwrap());
        assert_eq!((controller.render_offset(), controller.touch_offset()), (0, 0));
        assert!(controller.observe(NativePortraitOrientation::Down, 3).unwrap());
        assert_eq!((controller.render_offset(), controller.touch_offset()), (180, 180));
        assert!(controller.observe(NativePortraitOrientation::Down, 1).unwrap());
        assert_eq!((controller.render_offset(), controller.touch_offset()), (0, 180));
        assert!(!controller.observe(NativePortraitOrientation::Down, 1).unwrap());
        assert!(controller.observe(NativePortraitOrientation::Up, 1).unwrap());
        assert_eq!((controller.render_offset(), controller.touch_offset()), (180, 0));
    }

    #[test]
    fn rejected_landscape_observation_preserves_the_applied_mapping() {
        let mut controller = PortraitController::new(0);
        let request = OrientationRequest {
            sequence: 1,
            calibration: Some(PortraitCalibration {
                holding: NativePortraitOrientation::Down,
                scanout: NativePortraitOrientation::Down,
                raw_rotation: 0,
            }),
        };
        controller.apply_request(request, 0).unwrap();
        assert!(controller.observe(NativePortraitOrientation::Up, 1).is_err());
        assert_eq!((controller.render_offset(), controller.touch_offset()), (0, 180));
        assert!(!controller.apply_request(request, 2).unwrap());
    }
}
