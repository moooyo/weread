//! Frame-bound raw-image regions and reviewed hardware Y4 eligibility helpers.
//!
//! Identity and palette rules follow the reviewed bilicomics implementation at
//! 7241a85fac8aa032b79f4e7f2f6eb29aacdbebd3. Display ownership stays in this backend.

use std::fmt;
use std::io::{self, Read};
use std::sync::{Arc, Mutex};

pub(crate) const MAX_RAW_IMAGE_REGIONS: usize = 128;

/// A raw image in the published, unrotated Slint window's physical pixels.
///
/// The renderer rotates and clips these regions exactly once into framebuffer
/// coordinates. Empty scenes contain no raw images; every region is nonempty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RawImageRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A visible image rectangle with the original, uncropped image coordinate
/// system retained for matching the software-prepared image's Bayer phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PhysicalImageRegion {
    pub region: RawImageRegion,
    pub source: RawImageRegion,
    pub panel: (u32, u32),
    pub rotation: u32,
}

impl PhysicalImageRegion {
    /// Recover final-size image-local coordinates without resetting phase at a
    /// clipped edge or at the rotated rectangle's physical upper-left corner.
    pub(crate) fn image_local(&self, x: usize, y: usize) -> Option<(usize, usize)> {
        if !region_contains(std::slice::from_ref(&self.region), x, y) {
            return None;
        }
        self.source.endpoints()?;
        let x = u32::try_from(x).ok()?;
        let y = u32::try_from(y).ok()?;
        if x >= self.panel.0 || y >= self.panel.1 {
            return None;
        }
        let (source_x, source_y) = match self.rotation % 360 {
            0 => (x, y),
            90 => (y, self.panel.0.checked_sub(x)?.checked_sub(1)?),
            180 => (
                self.panel.0.checked_sub(x)?.checked_sub(1)?,
                self.panel.1.checked_sub(y)?.checked_sub(1)?,
            ),
            270 => (self.panel.1.checked_sub(y)?.checked_sub(1)?, x),
            _ => return None,
        };
        let local_x = source_x.checked_sub(self.source.x)?;
        let local_y = source_y.checked_sub(self.source.y)?;
        (local_x < self.source.width && local_y < self.source.height)
            .then_some((local_x as usize, local_y as usize))
    }
}

impl RawImageRegion {
    fn endpoints(self) -> Option<(u32, u32)> {
        if self.width == 0 || self.height == 0 {
            return None;
        }
        Some((
            self.x.checked_add(self.width)?,
            self.y.checked_add(self.height)?,
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DitheringError {
    InvalidRegions,
    SequenceExhausted,
    StateUnavailable,
}

impl fmt::Display for DitheringError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRegions => "the raw-image regions exceed their count or geometry limits",
            Self::SequenceExhausted => "the frame dithering sequence is exhausted",
            Self::StateUnavailable => "the frame dithering state is unavailable",
        })
    }
}

impl std::error::Error for DitheringError {}

/// An immutable scene snapshot. Later publications or device rejection cannot
/// change the regions associated with a frame already rendered to canonical RGB.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DitheringSnapshot {
    pub(crate) revision: u64,
    pub(crate) regions: Arc<[RawImageRegion]>,
    pub(crate) hardware_available: bool,
}

struct State {
    revision: u64,
    regions: Arc<[RawImageRegion]>,
    available: bool,
    rejected: bool,
    exhausted: bool,
}

impl State {
    fn advance(&mut self) -> Result<(), DitheringError> {
        let Some(next) = self.revision.checked_add(1) else {
            self.available = false;
            self.rejected = true;
            self.exhausted = true;
            return Err(DitheringError::SequenceExhausted);
        };
        self.revision = next;
        Ok(())
    }
}

/// Requested raw regions and actual hardware eligibility are separate. A
/// rejected hardware request retains its raw scene for an immediate CPU fallback.
pub(crate) struct DitheringControl {
    state: Mutex<State>,
}

impl Default for DitheringControl {
    fn default() -> Self {
        Self::new(false)
    }
}

impl DitheringControl {
    pub(crate) fn new(available: bool) -> Self {
        Self {
            state: Mutex::new(State {
                revision: 0,
                regions: Arc::from(Vec::<RawImageRegion>::new()),
                available,
                rejected: false,
                exhausted: false,
            }),
        }
    }

    /// Call in the same scene-publication transaction as the matching UI image.
    /// Preparing a future image must not change the currently displayed scene.
    pub(crate) fn publish(&self, regions: Vec<RawImageRegion>) -> Result<(), DitheringError> {
        if regions.len() > MAX_RAW_IMAGE_REGIONS
            || regions.iter().any(|region| region.endpoints().is_none())
        {
            return Err(DitheringError::InvalidRegions);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| DitheringError::StateUnavailable)?;
        if state.exhausted {
            return Err(DitheringError::SequenceExhausted);
        }
        if state.regions.as_ref() != regions.as_slice() {
            state.advance()?;
            state.regions = Arc::from(regions);
        }
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> Result<DitheringSnapshot, DitheringError> {
        let state = self
            .state
            .lock()
            .map_err(|_| DitheringError::StateUnavailable)?;
        if state.exhausted {
            return Err(DitheringError::SequenceExhausted);
        }
        Ok(DitheringSnapshot {
            revision: state.revision,
            regions: state.regions.clone(),
            hardware_available: state.available && !state.rejected,
        })
    }

    pub(crate) fn hardware_y4_available(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.available && !state.rejected && !state.exhausted)
    }

    /// Metadata or runtime policy may remove eligibility temporarily. An explicit
    /// driver rejection remains sticky and cannot be undone by this method.
    pub(crate) fn set_available(&self, available: bool) {
        if let Ok(mut state) = self.state.lock() {
            if state.exhausted {
                return;
            }
            let available = available && !state.rejected;
            if state.available != available && state.advance().is_ok() {
                state.available = available;
            }
        }
    }

    pub(crate) fn reject_hardware(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.rejected = true;
            if state.available {
                state.available = false;
                let _ = state.advance();
            }
        }
    }
}

/// The proc transport may append NUL padding. Read exactly the fixed identity
/// width, without consuming its padding or retaining a complete serial number.
pub(crate) fn read_supported_scribe(mut reader: impl Read) -> io::Result<bool> {
    let mut identity = [0u8; 16];
    reader.read_exact(&mut identity)?;
    Ok(supported_scribe(&identity))
}

fn supported_scribe(identity: &[u8; 16]) -> bool {
    if identity[0] != b'G'
        || !identity
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return false;
    }
    // Public hardware codes follow KOReader 7c38cf104cb9ea89df243e47ec820ef4aa97cab8.
    // They identify only the reviewed monochrome Scribe 2022 and 2024 families.
    matches!(
        &identity[3..6],
        b"27J"
            | b"2BL"
            | b"263"
            | b"227"
            | b"2BM"
            | b"23L"
            | b"23M"
            | b"270"
            | b"3V0"
            | b"3V1"
            | b"3X5"
            | b"3UV"
            | b"3X4"
            | b"3X3"
            | b"41E"
            | b"410"
    )
}

/// Quantize logical gray at its final-size image-local coordinate. Already
/// quantized sixteen-level image pixels remain unchanged in overlapping regions.
pub(crate) fn software_gray(value: u8, x: usize, y: usize) -> u8 {
    const BAYER: [[i16; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let adjusted = (i16::from(value) + BAYER[y % 4][x % 4] - 8).clamp(0, 255) as u16;
    (((adjusted + 8) / 17).min(15) * 17) as u8
}

/// Convert the published window regions once using the same quadrant mapping as
/// refresh rectangles. Invalid geometry or angles fail closed instead of touching
/// pixels outside an image, such as a paragraph or a control.
pub(crate) fn physical_regions(
    regions: &[RawImageRegion],
    panel: (u32, u32),
    rotation: u32,
) -> Vec<RawImageRegion> {
    if regions.len() > MAX_RAW_IMAGE_REGIONS {
        return Vec::new();
    }
    regions
        .iter()
        .filter_map(|region| physical_region(*region, panel, rotation))
        .collect()
}

pub(crate) fn physical_image_regions(
    raw: &[RawImageRegion],
    panel: (u32, u32),
    rotation: u32,
) -> Vec<PhysicalImageRegion> {
    if raw.len() > MAX_RAW_IMAGE_REGIONS {
        return Vec::new();
    }
    raw.iter()
        .filter_map(|source| {
            Some(PhysicalImageRegion {
                region: physical_region(*source, panel, rotation)?,
                source: *source,
                panel,
                rotation: rotation % 360,
            })
        })
        .collect()
}

fn physical_region(
    region: RawImageRegion,
    panel: (u32, u32),
    rotation: u32,
) -> Option<RawImageRegion> {
    let rotation = rotation % 360;
    if panel.0 == 0 || panel.1 == 0 || !matches!(rotation, 0 | 90 | 180 | 270) {
        return None;
    }
    let window = if matches!(rotation, 90 | 270) {
        (panel.1, panel.0)
    } else {
        panel
    };
    let (right, bottom) = region.endpoints()?;
    let x = region.x.min(window.0);
    let y = region.y.min(window.1);
    let right = right.min(window.0);
    let bottom = bottom.min(window.1);
    if right <= x || bottom <= y {
        return None;
    }
    let width = right - x;
    let height = bottom - y;
    Some(match rotation {
        90 => RawImageRegion {
            x: panel.0 - y - height,
            y: x,
            width: height,
            height: width,
        },
        180 => RawImageRegion {
            x: panel.0 - x - width,
            y: panel.1 - y - height,
            width,
            height,
        },
        270 => RawImageRegion {
            x: y,
            y: panel.1 - x - width,
            width: height,
            height: width,
        },
        _ => RawImageRegion {
            x,
            y,
            width,
            height,
        },
    })
}

/// Test half-open image bounds without allowing malformed endpoints to wrap.
pub(crate) fn region_contains(regions: &[RawImageRegion], x: usize, y: usize) -> bool {
    regions.iter().any(|region| {
        region.endpoints().is_some_and(|(right, bottom)| {
            x >= region.x as usize
                && x < right as usize
                && y >= region.y as usize
                && y < bottom as usize
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(x: u32, y: u32, width: u32, height: u32) -> RawImageRegion {
        RawImageRegion {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn identity_transport_reads_only_the_first_sixteen_bytes() {
        for suffix in [
            b"".as_slice(),
            b"\0",
            b"\0\0\n",
            b"\n",
            b"ignored transport suffix",
        ] {
            // This synthetic identity contains only a public family code.
            let mut input = b"G0023L0000000000".to_vec();
            input.extend_from_slice(suffix);
            let mut reader = io::Cursor::new(input);
            assert!(read_supported_scribe(&mut reader).unwrap());
            assert_eq!(reader.position(), 16);
            assert_eq!(&reader.get_ref()[16..], suffix);
        }
    }

    #[test]
    fn identity_transport_rejects_short_or_malformed_fixed_width_identities() {
        let valid = b"G0023L0000000000";
        for length in 0..16 {
            assert_eq!(
                read_supported_scribe(&valid[..length]).unwrap_err().kind(),
                io::ErrorKind::UnexpectedEof
            );
        }
        for position in 0..16 {
            for invalid in [0, b' ', b'\n', b'a', 0xff] {
                let mut input = *valid;
                input[position] = invalid;
                assert!(!read_supported_scribe(input.as_slice()).unwrap());
            }
        }
        for code in ["1PX", "3H3", "ZZZ"] {
            assert!(!read_supported_scribe(format!("G00{code}0000000000").as_bytes()).unwrap());
        }
    }

    struct FragmentedIdentity {
        position: usize,
        fail_after: Option<usize>,
    }

    impl Read for FragmentedIdentity {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            assert!(self.position < 16, "Transport padding must not be read");
            assert!(output.len() <= 16 - self.position);
            if self.fail_after.is_some_and(|limit| self.position >= limit) {
                return Err(io::Error::from_raw_os_error(libc::EIO));
            }
            let limit = self.fail_after.unwrap_or(16);
            let count = output.len().min(3).min(limit - self.position);
            output[..count]
                .copy_from_slice(&b"G0023L0000000000"[self.position..self.position + count]);
            self.position += count;
            Ok(count)
        }
    }

    #[test]
    fn identity_transport_handles_fragmented_reads_and_propagates_io_failures() {
        let mut valid = FragmentedIdentity {
            position: 0,
            fail_after: None,
        };
        assert!(read_supported_scribe(&mut valid).unwrap());
        assert_eq!(valid.position, 16);
        for limit in [0, 6, 15] {
            let mut reader = FragmentedIdentity {
                position: 0,
                fail_after: Some(limit),
            };
            assert_eq!(
                read_supported_scribe(&mut reader)
                    .unwrap_err()
                    .raw_os_error(),
                Some(libc::EIO)
            );
            assert_eq!(reader.position, limit);
        }
    }

    #[test]
    fn every_reviewed_scribe_public_code_is_recognized() {
        for code in [
            "27J", "2BL", "263", "227", "2BM", "23L", "23M", "270", "3V0", "3V1", "3X5", "3UV",
            "3X4", "3X3", "41E", "410",
        ] {
            assert!(read_supported_scribe(format!("G00{code}0000000000").as_bytes()).unwrap());
        }
        assert!(!read_supported_scribe(b"B0023L0000000000".as_slice()).unwrap());
    }

    #[test]
    fn publications_are_bounded_and_invalid_geometry_cannot_replace_the_scene() {
        let control = DitheringControl::new(true);
        let valid = region(1, 2, 3, 4);
        control.publish(vec![valid; MAX_RAW_IMAGE_REGIONS]).unwrap();
        let accepted = control.snapshot().unwrap();
        for invalid in [
            vec![valid; MAX_RAW_IMAGE_REGIONS + 1],
            vec![region(0, 0, 0, 1)],
            vec![region(0, 0, 1, 0)],
            vec![region(u32::MAX, 0, 1, 1)],
            vec![region(0, u32::MAX, 1, 1)],
        ] {
            assert_eq!(
                control.publish(invalid),
                Err(DitheringError::InvalidRegions)
            );
            assert_eq!(control.snapshot().unwrap(), accepted);
        }
        control.publish(Vec::new()).unwrap();
        assert!(control.snapshot().unwrap().regions.is_empty());
    }

    #[test]
    fn snapshots_keep_the_actual_frame_when_a_later_scene_is_published() {
        let control = DitheringControl::new(true);
        let first = region(1, 2, 3, 4);
        let second = region(5, 6, 7, 8);
        control.publish(vec![first]).unwrap();
        let retained = control.snapshot().unwrap();
        control.publish(vec![first]).unwrap();
        assert_eq!(control.snapshot().unwrap().revision, retained.revision);
        control.publish(vec![second]).unwrap();
        control.reject_hardware();
        assert_eq!(retained.regions.as_ref(), &[first]);
        assert!(retained.hardware_available);
        let latest = control.snapshot().unwrap();
        assert_eq!(latest.regions.as_ref(), &[second]);
        assert!(latest.revision > retained.revision);
        assert!(!latest.hardware_available);
    }

    #[test]
    fn rejection_is_sticky_but_retains_raw_regions_until_their_replacement() {
        let control = DitheringControl::new(true);
        let raw = region(1, 2, 3, 4);
        control.publish(vec![raw]).unwrap();
        control.set_available(false);
        assert!(!control.hardware_y4_available());
        control.set_available(true);
        assert!(control.hardware_y4_available());
        control.reject_hardware();
        let rejected = control.snapshot().unwrap();
        control.set_available(true);
        control.publish(vec![raw]).unwrap();
        assert!(!control.hardware_y4_available());
        assert_eq!(control.snapshot().unwrap(), rejected);
        assert_eq!(rejected.regions.as_ref(), &[raw]);
        control.publish(Vec::new()).unwrap();
        assert!(control.snapshot().unwrap().regions.is_empty());
        assert!(!control.hardware_y4_available());
        let initially_unavailable = DitheringControl::new(false);
        initially_unavailable.reject_hardware();
        initially_unavailable.set_available(true);
        assert!(!initially_unavailable.hardware_y4_available());
    }

    #[test]
    fn revision_exhaustion_fails_closed_instead_of_reusing_a_frame_identity() {
        let control = DitheringControl::new(true);
        control.state.lock().unwrap().revision = u64::MAX;
        assert_eq!(
            control.publish(vec![region(0, 0, 1, 1)]),
            Err(DitheringError::SequenceExhausted)
        );
        assert_eq!(control.snapshot(), Err(DitheringError::SequenceExhausted));
        assert!(!control.hardware_y4_available());
        control.set_available(true);
        assert_eq!(control.snapshot(), Err(DitheringError::SequenceExhausted));
    }

    #[test]
    fn publication_snapshots_cannot_mix_regions_from_different_revisions() {
        let control = Arc::new(DitheringControl::new(true));
        let writer = control.clone();
        let worker = std::thread::spawn(move || {
            for x in 1..=100 {
                writer.publish(vec![region(x, 0, 1, 1)]).unwrap();
            }
        });
        for _ in 0..200 {
            let snapshot = control.snapshot().unwrap();
            if let Some(region) = snapshot.regions.first() {
                assert_eq!(snapshot.revision, u64::from(region.x));
            } else {
                assert_eq!(snapshot.revision, 0);
            }
        }
        worker.join().unwrap();
        assert_eq!(control.snapshot().unwrap().revision, 100);
    }

    #[test]
    fn regions_rotate_once_with_the_same_window_geometry_as_refreshes() {
        let input = [region(1, 2, 3, 2)];
        for (rotation, expected) in [
            (0, region(1, 2, 3, 2)),
            (90, region(4, 1, 2, 3)),
            (180, region(4, 2, 3, 2)),
            (270, region(2, 2, 2, 3)),
            (360, region(1, 2, 3, 2)),
        ] {
            assert_eq!(physical_regions(&input, (8, 6), rotation), [expected]);
        }
    }

    #[test]
    fn regions_clip_before_rotation_and_reject_invalid_geometry_or_angles() {
        let clipped = [region(5, 6, 4, 4)];
        assert!(physical_regions(&clipped, (8, 6), 0).is_empty());
        assert!(physical_regions(&clipped, (8, 6), 180).is_empty());
        assert_eq!(physical_regions(&clipped, (8, 6), 90), [region(0, 5, 2, 1)]);
        assert_eq!(
            physical_regions(&clipped, (8, 6), 270),
            [region(6, 0, 2, 1)]
        );
        assert_eq!(
            physical_regions(&[region(7, 5, 3, 3)], (8, 6), 0),
            [region(7, 5, 1, 1)]
        );
        assert_eq!(
            physical_regions(&[region(7, 5, 3, 3)], (8, 6), 180),
            [region(0, 0, 1, 1)]
        );
        for invalid in [
            region(8, 0, 1, 1),
            region(0, 6, 1, 1),
            region(0, 0, 0, 1),
            region(u32::MAX, 0, 1, 1),
        ] {
            assert!(physical_regions(&[invalid], (8, 6), 0).is_empty());
        }
        assert!(physical_regions(&[region(0, 0, 1, 1)], (0, 6), 0).is_empty());
        assert!(physical_regions(&[region(0, 0, 1, 1)], (8, 6), 45).is_empty());
        assert!(physical_regions(&vec![region(0, 0, 1, 1); 129], (8, 6), 0).is_empty());
        assert_eq!(
            physical_regions(&[region(u32::MAX - 1, 0, 1, 1)], (u32::MAX, 1), 0),
            [region(u32::MAX - 1, 0, 1, 1)]
        );
    }

    #[test]
    fn image_membership_is_half_open_and_never_extends_into_a_neighboring_glyph() {
        let images = [region(1, 1, 2, 2), region(5, 0, 1, 1)];
        for y in 0..5 {
            for x in 0..8 {
                let expected = ((1..3).contains(&x) && (1..3).contains(&y)) || (x == 5 && y == 0);
                assert_eq!(region_contains(&images, x, y), expected);
            }
        }
        assert!(!region_contains(
            &[region(u32::MAX, 0, 1, 1)],
            usize::MAX,
            0
        ));
    }

    #[test]
    fn image_local_coordinates_inverse_map_each_presentation_quadrant_once() {
        let source = region(1, 2, 3, 2);
        for (rotation, upper_left, lower_right) in [
            (0, (1, 2), (3, 3)),
            (90, (5, 1), (4, 3)),
            (180, (6, 3), (4, 2)),
            (270, (2, 4), (3, 2)),
        ] {
            let images = physical_image_regions(&[source], (8, 6), rotation);
            assert_eq!(images.len(), 1);
            let image = images[0];
            assert_eq!(image.source, source);
            assert_eq!(image.image_local(upper_left.0, upper_left.1), Some((0, 0)));
            assert_eq!(
                image.image_local(lower_right.0, lower_right.1),
                Some((2, 1))
            );
            assert_eq!(
                image.image_local(
                    (image.region.x + image.region.width) as usize,
                    image.region.y as usize
                ),
                None
            );
            assert_eq!(image.image_local(usize::MAX, usize::MAX), None);
        }
    }

    #[test]
    fn clipped_image_phase_uses_the_original_complete_source_rectangle() {
        let source = region(5, 5, 4, 4);
        let clockwise = physical_image_regions(&[source], (8, 6), 90)[0];
        assert_eq!(clockwise.region, region(0, 5, 3, 1));
        assert_eq!(clockwise.source, source);
        assert_eq!(clockwise.image_local(0, 5), Some((0, 2)));
        assert_eq!(clockwise.image_local(1, 5), Some((0, 1)));
        assert_eq!(clockwise.image_local(2, 5), Some((0, 0)));
        let counterclockwise = physical_image_regions(&[source], (8, 6), 270)[0];
        assert_eq!(counterclockwise.region, region(5, 0, 3, 1));
        assert_eq!(counterclockwise.image_local(5, 0), Some((0, 0)));
        assert_eq!(counterclockwise.image_local(6, 0), Some((0, 1)));
        assert_eq!(counterclockwise.image_local(7, 0), Some((0, 2)));
    }

    #[test]
    fn malformed_physical_mappings_cannot_supply_an_image_phase() {
        let source = region(0, 0, 2, 2);
        let valid = PhysicalImageRegion {
            region: source,
            source,
            panel: (2, 2),
            rotation: 0,
        };
        assert_eq!(valid.image_local(1, 1), Some((1, 1)));
        assert_eq!(
            PhysicalImageRegion {
                rotation: 45,
                ..valid
            }
            .image_local(1, 1),
            None
        );
        assert_eq!(
            PhysicalImageRegion {
                panel: (0, 0),
                ..valid
            }
            .image_local(0, 0),
            None
        );
        assert_eq!(
            PhysicalImageRegion {
                source: region(u32::MAX, 0, 1, 1),
                ..valid
            }
            .image_local(0, 0),
            None
        );
    }

    #[test]
    fn software_fallback_uses_image_local_phase_and_preserves_the_sixteen_level_palette() {
        for y in 0..4 {
            for x in 0..4 {
                for level in 0..=15 {
                    assert_eq!(software_gray(level * 17, x, y), level * 17);
                }
                for value in 0..=255 {
                    assert_eq!(software_gray(value, x, y) % 17, 0);
                    assert_eq!(
                        software_gray(value, x + 4, y + 4),
                        software_gray(value, x, y)
                    );
                }
            }
        }
        let tones = (0..16)
            .map(|phase| software_gray(110, phase % 4, phase / 4))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(tones.into_iter().collect::<Vec<_>>(), [102, 119]);
    }
}
