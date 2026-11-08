//! Same-launch portrait calibration, independent of UI and output polarity.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativePortraitDirection {
    Up,
    Down,
}

/// The native owner pairs a confirmed scanout direction with a read-only
/// framebuffer rotation before taking the window manager's display ownership.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativePortraitCalibration {
    pub direction: NativePortraitDirection,
    pub framebuffer_rotation: u32,
}

impl NativePortraitCalibration {
    pub(crate) fn parse_payload(payload: &str) -> Option<Self> {
        let (direction, rotation) = payload.split_once(':')?;
        let direction = match direction {
            "U" => NativePortraitDirection::Up,
            "D" => NativePortraitDirection::Down,
            // FIXED and unavailable records keep the legacy mapping.
            _ => return None,
        };
        let framebuffer_rotation = match rotation {
            "0" => 0,
            "1" => 1,
            "2" => 2,
            "3" => 3,
            _ => return None,
        };
        Some(Self {
            direction,
            framebuffer_rotation,
        })
    }

    /// Resolve only a portrait transition relative to this exact startup pair.
    /// A raw kernel code has no universal upright/upside-down interpretation.
    pub(crate) fn scanout_at(self, observed_rotation: u32) -> Option<NativePortraitDirection> {
        if observed_rotation > 3 || self.framebuffer_rotation > 3 {
            return None;
        }
        match (observed_rotation + 4 - self.framebuffer_rotation) % 4 {
            0 => Some(self.direction),
            2 => Some(match self.direction {
                NativePortraitDirection::Up => NativePortraitDirection::Down,
                NativePortraitDirection::Down => NativePortraitDirection::Up,
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_records_accept_exact_auto_portrait_pairs_and_preserve_fixed_fallback() {
        for (token, direction) in [
            ("U", NativePortraitDirection::Up),
            ("D", NativePortraitDirection::Down),
        ] {
            for rotation in 0..=3 {
                assert_eq!(
                    NativePortraitCalibration::parse_payload(&format!("{token}:{rotation}")),
                    Some(NativePortraitCalibration {
                        direction,
                        framebuffer_rotation: rotation
                    })
                );
            }
        }
        for payload in [
            "FIXED:0",
            "AUTO:3",
            "U:4",
            "D:-1",
            "D:03",
            "U:1:extra",
            "u:0",
            "U: 0",
            "U:0\n",
        ] {
            assert!(
                NativePortraitCalibration::parse_payload(payload).is_none(),
                "{payload:?}"
            );
        }
    }

    #[test]
    fn scanout_uses_the_startup_pair_and_never_assumes_absolute_rotation_codes() {
        for raw in 0..=3 {
            for direction in [NativePortraitDirection::Up, NativePortraitDirection::Down] {
                let calibration = NativePortraitCalibration {
                    direction,
                    framebuffer_rotation: raw,
                };
                assert_eq!(calibration.scanout_at(raw), Some(direction));
                let reversed = match direction {
                    NativePortraitDirection::Up => NativePortraitDirection::Down,
                    NativePortraitDirection::Down => NativePortraitDirection::Up,
                };
                assert_eq!(calibration.scanout_at((raw + 2) % 4), Some(reversed));
                assert!(calibration.scanout_at((raw + 1) % 4).is_none());
                assert!(calibration.scanout_at((raw + 3) % 4).is_none());
                assert!(calibration.scanout_at(4).is_none());
            }
        }
    }
}
