//! Renderer-owned night-mode requests and canonical RGB output transforms.

use std::fmt;
use std::ops::Range;
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use slint::Rgb8Pixel;

use crate::dithering::{
    DitheringControl, DitheringError, PhysicalImageRegion, RawImageRegion, software_gray,
};
use crate::managed_sleep::{ManagedSleepControl, ManagedSleepState};
use crate::refresh::PhysicalRect;
use crate::sleep_presentation::SleepPresentationControl;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderError {
    EventLoopTerminated,
    AlreadyInitialized,
    PresentationUnavailable,
    PresentationSequenceExhausted,
    InvalidPresentationToken,
    InvalidRawImageRegions,
    DitheringUnavailable,
    DitheringSequenceExhausted,
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EventLoopTerminated => "the Kindle event loop has terminated",
            Self::AlreadyInitialized => "night-mode configuration must precede the event loop",
            Self::PresentationUnavailable => "the sleep presentation is unavailable or cancelled",
            Self::PresentationSequenceExhausted => "the sleep presentation sequence is exhausted",
            Self::InvalidPresentationToken => "the sleep presentation token was not issued",
            Self::InvalidRawImageRegions => {
                "the raw-image regions exceed their count or geometry limits"
            }
            Self::DitheringUnavailable => "the frame dithering state is unavailable",
            Self::DitheringSequenceExhausted => "the frame dithering sequence is exhausted",
        })
    }
}

impl std::error::Error for RenderError {}

impl From<DitheringError> for RenderError {
    fn from(error: DitheringError) -> Self {
        match error {
            DitheringError::InvalidRegions => Self::InvalidRawImageRegions,
            DitheringError::SequenceExhausted => Self::DitheringSequenceExhausted,
            DitheringError::StateUnavailable => Self::DitheringUnavailable,
        }
    }
}

/// Hardware support must be approved for the exact model and firmware.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NightModeConfig {
    #[default]
    SoftwareOnly,
    HardwareIfSupported,
}

const HARDWARE_ENABLED: u8 = 1;
const CONFIGURATION_LOCKED: u8 = 2;

pub(crate) struct NightModeControl {
    desired: AtomicBool,
    configuration: AtomicU8,
    native_baseline: AtomicU8,
}

impl Default for NightModeControl {
    fn default() -> Self {
        Self {
            desired: AtomicBool::new(false),
            configuration: AtomicU8::new(0),
            native_baseline: AtomicU8::new(0),
        }
    }
}

impl NightModeControl {
    fn configure(&self, config: NightModeConfig) -> Result<(), RenderError> {
        self.configuration
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                if current & CONFIGURATION_LOCKED != 0 {
                    None
                } else {
                    Some(match config {
                        NightModeConfig::SoftwareOnly => 0,
                        NightModeConfig::HardwareIfSupported => HARDWARE_ENABLED,
                    })
                }
            })
            .map(|_| ())
            .map_err(|_| RenderError::AlreadyInitialized)
    }

    pub(crate) fn lock_configuration(&self) -> NightModeConfig {
        let current = self
            .configuration
            .fetch_or(CONFIGURATION_LOCKED, Ordering::AcqRel);
        if current & HARDWARE_ENABLED != 0 {
            NightModeConfig::HardwareIfSupported
        } else {
            NightModeConfig::SoftwareOnly
        }
    }

    pub(crate) fn desired(&self) -> bool {
        self.desired.load(Ordering::Acquire)
    }

    pub(crate) fn observe_native_baseline(&self, inverted: bool) {
        self.native_baseline
            .store(if inverted { 2 } else { 1 }, Ordering::Release);
    }

    fn native_output_inverted(&self) -> Option<bool> {
        match self.native_baseline.load(Ordering::Acquire) {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        }
    }
}

/// A producer for renderer-owned preferences, with no LIPC or dynconf writes.
/// Success acknowledges a request, not completion of a physical panel update.
#[derive(Clone)]
pub struct RenderHandle {
    pub(crate) night_mode: Arc<NightModeControl>,
    pub(crate) managed_sleep: Arc<ManagedSleepControl>,
    pub(crate) sleep_presentation: Arc<SleepPresentationControl>,
    pub(crate) dithering: Arc<DitheringControl>,
    pub(crate) wake: Arc<OwnedFd>,
    pub(crate) quit: Arc<AtomicBool>,
}

impl RenderHandle {
    /// Eligibility is based on reviewed hardware identity and native metadata.
    /// It does not claim that a Y4 update was accepted or completed by the panel.
    pub fn hardware_y4_available(&self) -> bool {
        !self.quit.load(Ordering::SeqCst) && self.dithering.hardware_y4_available()
    }

    /// Publish the raw images belonging to the UI model already installed in
    /// the scene. Coordinates are physical window pixels before render rotation.
    /// An empty list means no raw images; it does not change a future preference.
    pub fn set_hardware_y4_for_frame(
        &self,
        regions: Vec<RawImageRegion>,
    ) -> Result<(), RenderError> {
        if self.quit.load(Ordering::SeqCst)
            || self.managed_sleep.state() == ManagedSleepState::Terminated
        {
            return Err(RenderError::EventLoopTerminated);
        }
        self.dithering.publish(regions)?;
        crate::wakeup::poke(&self.wake);
        Ok(())
    }

    /// Configure the renderer before its event loop opens the framebuffer.
    /// An opt-in still requires actual 8-bpp grayscale 1/2 metadata at runtime.
    pub fn configure_night_mode(&self, config: NightModeConfig) -> Result<(), RenderError> {
        if self.quit.load(Ordering::SeqCst) {
            return Err(RenderError::EventLoopTerminated);
        }
        self.night_mode.configure(config)
    }

    /// The latest native baseline observed at startup or a real resident wake.
    /// This does not report the renderer's temporary hardware inversion mode.
    pub fn native_output_inverted(&self) -> Option<bool> {
        self.night_mode.native_output_inverted()
    }

    /// Ask the event loop to release input and display ownership without exiting.
    /// The caller must wait for Suspended before allowing native UI to draw.
    pub fn set_managed_sleep(&self, sleeping: bool) -> Result<(), RenderError> {
        if self.quit.load(Ordering::SeqCst)
            || self.managed_sleep.state() == ManagedSleepState::Terminated
        {
            return Err(RenderError::EventLoopTerminated);
        }
        let changed = self.managed_sleep.request(sleeping);
        if sleeping {
            self.sleep_presentation.cancel_pending();
        }
        if changed {
            crate::wakeup::poke(&self.wake);
        }
        Ok(())
    }

    /// Request a complete frame after the caller has installed its sleep UI.
    /// The token is acknowledged only after actual rendering and EPDC completion.
    /// Several requests before one draw may share that draw; requests received
    /// during the draw remain pending for another complete frame.
    pub fn request_sleep_presentation(&self) -> Result<u64, RenderError> {
        if self.quit.load(Ordering::SeqCst)
            || self.managed_sleep.state() == ManagedSleepState::Terminated
        {
            return Err(RenderError::EventLoopTerminated);
        }
        if self.managed_sleep.state() != ManagedSleepState::Running {
            return Err(RenderError::PresentationUnavailable);
        }
        let token = self.sleep_presentation.request()?;
        if self.quit.load(Ordering::SeqCst)
            || self.managed_sleep.state() != ManagedSleepState::Running
        {
            self.sleep_presentation.cancel_pending();
            return Err(
                if self.quit.load(Ordering::SeqCst)
                    || self.managed_sleep.state() == ManagedSleepState::Terminated
                {
                    RenderError::EventLoopTerminated
                } else {
                    RenderError::PresentationUnavailable
                },
            );
        }
        crate::wakeup::poke(&self.wake);
        Ok(token)
    }

    /// A completed token proves the requested frame reached the panel. An old
    /// success cannot complete a newer request; termination invalidates all tokens.
    pub fn sleep_presentation_completed(&self, token: u64) -> Result<bool, RenderError> {
        if self.quit.load(Ordering::SeqCst)
            || self.managed_sleep.state() == ManagedSleepState::Terminated
        {
            return Err(RenderError::EventLoopTerminated);
        }
        self.sleep_presentation.completed(token)
    }

    /// Running acknowledges restored input and a complete framebuffer rewrite.
    pub fn managed_sleep_state(&self) -> ManagedSleepState {
        if self.quit.load(Ordering::SeqCst) {
            ManagedSleepState::Terminated
        } else {
            self.managed_sleep.state()
        }
    }

    /// Request the application's physical night appearance from canonical light RGB.
    /// The event loop owns inversion, a complete rewrite, and one full refresh.
    /// This request does not release touch or enter the managed-sleep protocol.
    pub fn set_night_mode(&self, enabled: bool) -> Result<(), RenderError> {
        if self.quit.load(Ordering::SeqCst) {
            return Err(RenderError::EventLoopTerminated);
        }
        if self.night_mode.desired.swap(enabled, Ordering::AcqRel) != enabled {
            crate::wakeup::poke(&self.wake);
        }
        Ok(())
    }

    /// Compatibility alias for [`Self::set_night_mode`].
    pub fn set_dark_mode(&self, enabled: bool) -> Result<(), RenderError> {
        self.set_night_mode(enabled)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct GrayMode {
    pub(crate) dark_mode: bool,
    pub(crate) black_and_white: bool,
}

impl GrayMode {
    pub(crate) fn paper(self) -> u8 {
        self.grayscale(255)
    }

    fn pixel(self, pixel: Rgb8Pixel) -> u8 {
        self.grayscale(luminance(pixel))
    }

    fn image_pixel(self, pixel: Rgb8Pixel, x: usize, y: usize) -> u8 {
        if self.black_and_white {
            return self.pixel(pixel);
        }
        self.grayscale(software_gray(luminance(pixel), x, y))
    }

    pub(crate) fn grayscale(self, value: u8) -> u8 {
        let value = if self.black_and_white {
            if value < 128 { 0 } else { 255 }
        } else {
            value
        };
        if self.dark_mode { 255 - value } else { value }
    }
}

fn luminance(pixel: Rgb8Pixel) -> u8 {
    ((77 * u32::from(pixel.r) + 150 * u32::from(pixel.g) + 29 * u32::from(pixel.b)) >> 8) as u8
}

pub(crate) struct FrameWrite {
    pub(crate) region: Option<PhysicalRect>,
    pub(crate) mode_changed: bool,
}

/// Keeps only a scanline of transformed output. Slint retains the original RGB
/// frame, so partial redraws and repeated full updates cannot double-invert it.
pub(crate) struct GrayRenderer {
    applied_mode: GrayMode,
    line: Vec<u8>,
    image_spans: Vec<ImageRowSpan>,
    image_gaps: Vec<ImageRowSpan>,
}

impl GrayRenderer {
    pub(crate) fn new(width: usize, initial_mode: GrayMode) -> Self {
        Self {
            applied_mode: initial_mode,
            line: vec![0; width],
            image_spans: Vec::new(),
            image_gaps: Vec::new(),
        }
    }

    pub(crate) fn mode(&self) -> GrayMode {
        self.applied_mode
    }

    pub(crate) fn write_frame(
        &mut self,
        rgb: &[Rgb8Pixel],
        panel: (u32, u32),
        dirty: Option<PhysicalRect>,
        mode: GrayMode,
        restore_full: bool,
        write_line: impl FnMut(usize, Range<usize>, &[u8]),
    ) -> FrameWrite {
        self.write_frame_with_image_regions(rgb, panel, dirty, mode, restore_full, &[], write_line)
    }

    pub(crate) fn write_frame_with_image_regions(
        &mut self,
        rgb: &[Rgb8Pixel],
        panel: (u32, u32),
        dirty: Option<PhysicalRect>,
        mode: GrayMode,
        restore_full: bool,
        software_regions: &[PhysicalImageRegion],
        mut write_line: impl FnMut(usize, Range<usize>, &[u8]),
    ) -> FrameWrite {
        let mode_changed = mode != self.applied_mode;
        let region = if mode_changed || restore_full {
            Some(PhysicalRect {
                x: 0,
                y: 0,
                width: panel.0,
                height: panel.1,
            })
        } else {
            dirty.and_then(|dirty| clip(dirty, panel))
        };
        if let Some(region) = region {
            let width = panel.0 as usize;
            let x = region.x as usize;
            let y = region.y as usize;
            let span = region.width as usize;
            self.line.resize(width, 0);
            for row in y..y + region.height as usize {
                image_row_spans(
                    &mut self.image_spans,
                    &mut self.image_gaps,
                    software_regions,
                    row,
                    x..x + span,
                );
                let start = row * width + x;
                let source = &rgb[start..start + span];
                let output = &mut self.line[..span];
                for (destination, source) in output.iter_mut().zip(source) {
                    *destination = mode.pixel(*source);
                }
                for image_span in &self.image_spans {
                    for column in image_span.columns.clone() {
                        if let Some((image_x, image_y)) =
                            software_regions[image_span.image_index].image_local(column, row)
                        {
                            output[column - x] =
                                mode.image_pixel(source[column - x], image_x, image_y);
                        }
                    }
                }
                write_line(row, x..x + span, output);
            }
        }
        // The entire output is written before this mode can be treated as applied.
        self.applied_mode = mode;
        FrameWrite {
            region,
            mode_changed,
        }
    }

    /// A rejected Y4 update may leave several raw images in the retained frame.
    /// Rewrite exactly their union from canonical RGB, never a paragraph, and
    /// never read already transformed framebuffer pixels as another input image.
    pub(crate) fn rewrite_image_regions(
        &mut self,
        rgb: &[Rgb8Pixel],
        panel: (u32, u32),
        regions: &[PhysicalImageRegion],
        mode: GrayMode,
        mut write_line: impl FnMut(usize, Range<usize>, &[u8]),
    ) {
        let width = panel.0 as usize;
        self.line.resize(width, 0);
        for row in 0..panel.1 as usize {
            image_row_spans(
                &mut self.image_spans,
                &mut self.image_gaps,
                regions,
                row,
                0..width,
            );
            for span in &self.image_spans {
                let columns = &span.columns;
                let source = &rgb[row * width + columns.start..row * width + columns.end];
                let output = &mut self.line[..columns.len()];
                for (offset, (destination, source)) in output.iter_mut().zip(source).enumerate() {
                    *destination =
                        match regions[span.image_index].image_local(columns.start + offset, row) {
                            Some((image_x, image_y)) => mode.image_pixel(*source, image_x, image_y),
                            None => mode.pixel(*source),
                        };
                }
                write_line(row, columns.clone(), output);
            }
        }
    }
}

struct ImageRowSpan {
    columns: Range<usize>,
    image_index: usize,
}

fn image_row_spans(
    spans: &mut Vec<ImageRowSpan>,
    gaps: &mut Vec<ImageRowSpan>,
    regions: &[PhysicalImageRegion],
    row: usize,
    bounds: Range<usize>,
) {
    spans.clear();
    for (image_index, image) in regions.iter().enumerate() {
        let region = image.region;
        let bottom = u64::from(region.y) + u64::from(region.height);
        if (row as u64) < u64::from(region.y) || row as u64 >= bottom {
            continue;
        }
        let start = (u64::from(region.x)).max(bounds.start as u64);
        let end = (u64::from(region.x) + u64::from(region.width)).min(bounds.end as u64);
        if start < end {
            let columns = start as usize..end as usize;
            gaps.clear();
            for previous in spans.drain(..) {
                if previous.columns.end <= columns.start || previous.columns.start >= columns.end {
                    gaps.push(previous);
                    continue;
                }
                if previous.columns.start < columns.start {
                    gaps.push(ImageRowSpan {
                        columns: previous.columns.start..columns.start,
                        image_index: previous.image_index,
                    });
                }
                if previous.columns.end > columns.end {
                    gaps.push(ImageRowSpan {
                        columns: columns.end..previous.columns.end,
                        image_index: previous.image_index,
                    });
                }
            }
            gaps.push(ImageRowSpan {
                columns,
                image_index,
            });
            gaps.sort_unstable_by_key(|span| span.columns.start);
            std::mem::swap(spans, gaps);
        }
    }
    // Disjoint spans preserve scene order without repeatedly processing an
    // overlap. The last image owns its crop-local phase, matching paint order.
}

fn clip(rect: PhysicalRect, panel: (u32, u32)) -> Option<PhysicalRect> {
    let x = i64::from(rect.x).clamp(0, i64::from(panel.0));
    let y = i64::from(rect.y).clamp(0, i64::from(panel.1));
    let right = (i64::from(rect.x) + i64::from(rect.width)).clamp(0, i64::from(panel.0));
    let bottom = (i64::from(rect.y) + i64::from(rect.height)).clamp(0, i64::from(panel.1));
    if right <= x || bottom <= y {
        return None;
    }
    Some(PhysicalRect {
        x: x as i32,
        y: y as i32,
        width: (right - x) as u32,
        height: (bottom - y) as u32,
    })
}

#[cfg(test)]
#[path = "render_image_regions_tests.rs"]
mod image_region_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(value: u8) -> Rgb8Pixel {
        Rgb8Pixel {
            r: value,
            g: value,
            b: value,
        }
    }

    #[test]
    fn configuration_is_sealed_at_start_and_native_observation_is_separate_from_desired() {
        let control = NightModeControl::default();
        assert_eq!(control.native_output_inverted(), None);
        control
            .configure(NightModeConfig::HardwareIfSupported)
            .unwrap();
        assert_eq!(
            control.lock_configuration(),
            NightModeConfig::HardwareIfSupported
        );
        assert_eq!(
            control.configure(NightModeConfig::SoftwareOnly),
            Err(RenderError::AlreadyInitialized)
        );
        control.observe_native_baseline(true);
        assert_eq!(control.native_output_inverted(), Some(true));
        assert!(!control.desired());
        control.desired.store(true, Ordering::Release);
        control.observe_native_baseline(false);
        assert_eq!(control.native_output_inverted(), Some(false));
        assert!(control.desired());
    }

    struct Panel {
        width: usize,
        height: usize,
        stride: usize,
        bytes: Vec<u8>,
    }

    impl Panel {
        fn new(width: usize, height: usize) -> Self {
            let stride = width + 3;
            Self {
                width,
                height,
                stride,
                bytes: vec![0x31; stride * height],
            }
        }

        fn draw(
            &mut self,
            renderer: &mut GrayRenderer,
            rgb: &[Rgb8Pixel],
            dirty: Option<PhysicalRect>,
            mode: GrayMode,
        ) -> FrameWrite {
            renderer.write_frame(
                rgb,
                (self.width as u32, self.height as u32),
                dirty,
                mode,
                false,
                |row, columns, pixels| {
                    let start = row * self.stride + columns.start;
                    self.bytes[start..start + pixels.len()].copy_from_slice(pixels);
                },
            )
        }

        fn visible(&self) -> Vec<u8> {
            self.bytes
                .chunks_exact(self.stride)
                .flat_map(|row| row[..self.width].iter().copied())
                .collect()
        }

        fn assert_padding_untouched(&self) {
            for row in self.bytes.chunks_exact(self.stride) {
                assert_eq!(&row[self.width..], &[0x31; 3]);
            }
        }
    }

    fn full(width: u32, height: u32) -> Option<PhysicalRect> {
        Some(PhysicalRect {
            x: 0,
            y: 0,
            width,
            height,
        })
    }

    #[test]
    fn text_antialiasing_and_color_image_pixels_invert_exactly_once() {
        let rgb = [
            gray(255),
            gray(0),
            gray(127),
            gray(128),
            Rgb8Pixel { r: 255, g: 0, b: 0 },
            Rgb8Pixel { r: 0, g: 255, b: 0 },
            Rgb8Pixel { r: 0, g: 0, b: 255 },
        ];
        let original = rgb;
        let mut panel = Panel::new(7, 1);
        let light = GrayMode::default();
        let dark = GrayMode {
            dark_mode: true,
            ..light
        };
        let mut renderer = GrayRenderer::new(7, light);
        panel.draw(&mut renderer, &rgb, full(7, 1), light);
        assert_eq!(panel.visible(), [255, 0, 127, 128, 76, 149, 28]);
        panel.draw(&mut renderer, &rgb, None, dark);
        assert_eq!(panel.visible(), [0, 255, 128, 127, 179, 106, 227]);
        panel.draw(&mut renderer, &rgb, full(7, 1), dark);
        assert_eq!(panel.visible(), [0, 255, 128, 127, 179, 106, 227]);
        assert_eq!(rgb, original, "Canonical RGB pixels must remain unchanged");
        panel.assert_padding_untouched();
    }

    #[test]
    fn transitions_rewrite_every_pixel_and_partial_pages_and_popups_keep_the_mode() {
        let light = GrayMode::default();
        let dark = GrayMode {
            dark_mode: true,
            ..light
        };
        let mut renderer = GrayRenderer::new(3, light);
        let mut panel = Panel::new(3, 2);
        let mut rgb = [
            gray(255),
            gray(0),
            gray(128),
            gray(70),
            gray(255),
            gray(200),
        ];
        panel.draw(&mut renderer, &rgb, full(3, 2), light);
        assert_eq!(panel.visible(), [255, 0, 128, 70, 255, 200]);

        let one_pixel = Some(PhysicalRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        });
        let transition = panel.draw(&mut renderer, &rgb, one_pixel, dark);
        assert_eq!(transition.region, full(3, 2));
        assert!(transition.mode_changed);
        assert_eq!(panel.visible(), [0, 255, 127, 185, 0, 55]);

        rgb[0] = gray(0);
        let popup = panel.draw(&mut renderer, &rgb, one_pixel, dark);
        assert_eq!(popup.region, one_pixel);
        assert!(!popup.mode_changed);
        assert_eq!(panel.visible(), [255, 255, 127, 185, 0, 55]);
        panel.draw(&mut renderer, &rgb, full(3, 2), dark);
        assert_eq!(panel.visible(), [255, 255, 127, 185, 0, 55]);

        rgb = [
            gray(10),
            gray(240),
            gray(90),
            gray(30),
            gray(250),
            gray(110),
        ];
        panel.draw(&mut renderer, &rgb, full(3, 2), dark);
        assert_eq!(panel.visible(), [245, 15, 165, 225, 5, 145]);
        let transition = panel.draw(&mut renderer, &rgb, None, light);
        assert_eq!(transition.region, full(3, 2));
        assert_eq!(panel.visible(), [10, 240, 90, 30, 250, 110]);
        assert!(
            panel
                .draw(&mut renderer, &rgb, None, light)
                .region
                .is_none()
        );
        panel.assert_padding_untouched();
    }

    #[test]
    fn bilevel_threshold_is_applied_before_inversion_and_round_trips_cleanly() {
        let rgb = [gray(0), gray(127), gray(128), gray(255)];
        let light = GrayMode {
            black_and_white: true,
            dark_mode: false,
        };
        let dark = GrayMode {
            dark_mode: true,
            ..light
        };
        let mut renderer = GrayRenderer::new(4, light);
        let mut panel = Panel::new(4, 1);
        panel.draw(&mut renderer, &rgb, full(4, 1), light);
        assert_eq!(panel.visible(), [0, 0, 255, 255]);
        panel.draw(&mut renderer, &rgb, None, dark);
        assert_eq!(panel.visible(), [255, 255, 0, 0]);
        panel.draw(&mut renderer, &rgb, None, light);
        assert_eq!(panel.visible(), [0, 0, 255, 255]);
    }

    #[test]
    fn render_requests_wake_the_loop_and_latest_state_controls_actual_pixels() {
        let wakeup = crate::wakeup::make_wakeup().unwrap();
        let handle = RenderHandle {
            night_mode: Arc::new(NightModeControl::default()),
            managed_sleep: Arc::new(ManagedSleepControl::default()),
            sleep_presentation: Arc::new(SleepPresentationControl::default()),
            dithering: Arc::new(DitheringControl::default()),
            wake: wakeup.write.clone(),
            quit: Arc::new(AtomicBool::new(false)),
        };
        handle.set_dark_mode(true).unwrap();
        handle.set_dark_mode(false).unwrap();
        handle.set_dark_mode(true).unwrap();
        let mut descriptor = libc::pollfd {
            fd: std::os::fd::AsRawFd::as_raw_fd(&*wakeup.read),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: descriptor points to one valid pollfd for the call.
        assert_eq!(unsafe { libc::poll(&mut descriptor, 1, 0) }, 1);
        assert_ne!(descriptor.revents & libc::POLLIN, 0);

        let mut renderer = GrayRenderer::new(2, GrayMode::default());
        let mut panel = Panel::new(2, 1);
        let rgb = [gray(255), gray(20)];
        let mode = GrayMode {
            dark_mode: handle.night_mode.desired(),
            black_and_white: false,
        };
        panel.draw(&mut renderer, &rgb, None, mode);
        assert_eq!(panel.visible(), [0, 235]);
        handle.quit.store(true, Ordering::SeqCst);
        assert_eq!(
            handle.set_dark_mode(false),
            Err(RenderError::EventLoopTerminated)
        );
        panel.draw(&mut renderer, &rgb, full(2, 1), mode);
        assert_eq!(panel.visible(), [0, 235]);
    }

    #[test]
    fn a_resident_wake_rewrites_native_pixels_even_when_the_ui_is_not_dirty() {
        for mode in [
            GrayMode::default(),
            GrayMode {
                dark_mode: true,
                black_and_white: false,
            },
        ] {
            let mut renderer = GrayRenderer::new(3, mode);
            let mut panel = Panel::new(3, 2);
            let rgb = [
                gray(10),
                gray(50),
                gray(90),
                gray(130),
                gray(180),
                gray(250),
            ];
            let original = rgb;
            panel.draw(&mut renderer, &rgb, full(3, 2), mode);
            let expected = panel.visible();
            for dirty in [
                None,
                Some(PhysicalRect {
                    x: 1,
                    y: 1,
                    width: 1,
                    height: 1,
                }),
            ] {
                for row in panel.bytes.chunks_exact_mut(panel.stride) {
                    row[..panel.width].fill(0x71);
                }
                let stride = panel.stride;
                let written = renderer.write_frame(
                    &rgb,
                    (3, 2),
                    dirty,
                    mode,
                    true,
                    |row, columns, pixels| {
                        let start = row * stride + columns.start;
                        panel.bytes[start..start + pixels.len()].copy_from_slice(pixels);
                    },
                );
                assert_eq!(written.region, full(3, 2));
                assert!(
                    !written.mode_changed,
                    "Restoration cannot rely on a mode transition"
                );
                assert_eq!(panel.visible(), expected);
                assert_eq!(rgb, original);
                panel.assert_padding_untouched();
            }
        }
    }

    #[test]
    fn sleep_requests_wake_the_backend_and_only_acknowledgment_completes_the_transition() {
        let wakeup = crate::wakeup::make_wakeup().unwrap();
        let handle = RenderHandle {
            night_mode: Arc::new(NightModeControl::default()),
            managed_sleep: Arc::new(ManagedSleepControl::default()),
            sleep_presentation: Arc::new(SleepPresentationControl::default()),
            dithering: Arc::new(DitheringControl::default()),
            wake: wakeup.write.clone(),
            quit: Arc::new(AtomicBool::new(false)),
        };
        assert_eq!(handle.managed_sleep_state(), ManagedSleepState::Running);
        handle.set_managed_sleep(true).unwrap();
        handle.set_managed_sleep(true).unwrap();
        assert_eq!(handle.managed_sleep_state(), ManagedSleepState::Suspending);
        let mut descriptor = libc::pollfd {
            fd: std::os::fd::AsRawFd::as_raw_fd(&*wakeup.read),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(unsafe { libc::poll(&mut descriptor, 1, 0) }, 1);
        crate::wakeup::drain(&wakeup.read);
        assert_eq!(unsafe { libc::poll(&mut descriptor, 1, 0) }, 0);
        handle.set_managed_sleep(true).unwrap();
        assert_eq!(
            unsafe { libc::poll(&mut descriptor, 1, 0) },
            0,
            "A duplicate request must not keep the pipe busy"
        );
        handle.managed_sleep.acknowledge_suspended();
        assert_eq!(handle.managed_sleep_state(), ManagedSleepState::Suspended);
        handle.set_managed_sleep(false).unwrap();
        assert_eq!(handle.managed_sleep_state(), ManagedSleepState::Resuming);
        handle.managed_sleep.acknowledge_running();
        assert_eq!(handle.managed_sleep_state(), ManagedSleepState::Running);
        handle.quit.store(true, Ordering::SeqCst);
        assert_eq!(handle.managed_sleep_state(), ManagedSleepState::Terminated);
        assert_eq!(
            handle.set_managed_sleep(true),
            Err(RenderError::EventLoopTerminated)
        );
    }

    #[test]
    fn sleep_presentations_wake_without_releasing_input_and_require_their_own_ack() {
        let wakeup = crate::wakeup::make_wakeup().unwrap();
        let handle = RenderHandle {
            night_mode: Arc::new(NightModeControl::default()),
            managed_sleep: Arc::new(ManagedSleepControl::default()),
            sleep_presentation: Arc::new(SleepPresentationControl::default()),
            dithering: Arc::new(DitheringControl::default()),
            wake: wakeup.write.clone(),
            quit: Arc::new(AtomicBool::new(false)),
        };
        let first = handle.request_sleep_presentation().unwrap();
        assert_eq!(handle.managed_sleep_state(), ManagedSleepState::Running);
        assert!(!handle.sleep_presentation_completed(first).unwrap());
        let mut descriptor = libc::pollfd {
            fd: std::os::fd::AsRawFd::as_raw_fd(&*wakeup.read),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: descriptor points to one live wake-pipe pollfd for this call.
        assert_eq!(unsafe { libc::poll(&mut descriptor, 1, 0) }, 1);
        handle.sleep_presentation.acknowledge(first).unwrap();
        assert!(handle.sleep_presentation_completed(first).unwrap());
        let next = handle.request_sleep_presentation().unwrap();
        assert!(!handle.sleep_presentation_completed(next).unwrap());
        handle.set_managed_sleep(true).unwrap();
        assert_eq!(
            handle.sleep_presentation_completed(next),
            Err(RenderError::PresentationUnavailable)
        );
        assert_eq!(
            handle.request_sleep_presentation(),
            Err(RenderError::PresentationUnavailable)
        );
        handle.managed_sleep.acknowledge_suspended();
        handle.set_managed_sleep(false).unwrap();
        assert_eq!(
            handle.request_sleep_presentation(),
            Err(RenderError::PresentationUnavailable)
        );
        handle.managed_sleep.acknowledge_running();
        let fresh = handle.request_sleep_presentation().unwrap();
        assert!(fresh > next);
        handle.sleep_presentation.acknowledge(fresh).unwrap();
        handle.quit.store(true, Ordering::SeqCst);
        assert_eq!(
            handle.sleep_presentation_completed(fresh),
            Err(RenderError::EventLoopTerminated)
        );
        assert_eq!(
            handle.request_sleep_presentation(),
            Err(RenderError::EventLoopTerminated)
        );
    }

    #[test]
    fn image_regions_are_scene_bound_and_can_be_staged_before_a_resident_wake() {
        let wake = crate::wakeup::make_wakeup().unwrap();
        let handle = RenderHandle {
            night_mode: Arc::new(NightModeControl::default()),
            managed_sleep: Arc::new(ManagedSleepControl::default()),
            sleep_presentation: Arc::new(SleepPresentationControl::default()),
            dithering: Arc::new(DitheringControl::new(true)),
            wake: wake.write,
            quit: Arc::new(AtomicBool::new(false)),
        };
        assert!(handle.hardware_y4_available());
        let raw = RawImageRegion {
            x: 9,
            y: 7,
            width: 3,
            height: 2,
        };
        handle.set_hardware_y4_for_frame(vec![raw]).unwrap();
        let retained = handle.dithering.snapshot().unwrap();
        assert_eq!(retained.regions.as_ref(), &[raw]);
        handle.set_managed_sleep(true).unwrap();
        handle.managed_sleep.acknowledge_suspended();
        handle.set_hardware_y4_for_frame(Vec::new()).unwrap();
        assert_eq!(handle.managed_sleep_state(), ManagedSleepState::Suspended);
        assert!(handle.dithering.snapshot().unwrap().regions.is_empty());
        assert_eq!(retained.regions.as_ref(), &[raw]);
        assert_eq!(
            handle.set_hardware_y4_for_frame(vec![raw; 129]),
            Err(RenderError::InvalidRawImageRegions)
        );
        handle.dithering.reject_hardware();
        assert!(!handle.hardware_y4_available());
        handle.dithering.set_available(true);
        assert!(!handle.hardware_y4_available());
        handle.quit.store(true, Ordering::SeqCst);
        assert_eq!(
            handle.set_hardware_y4_for_frame(Vec::new()),
            Err(RenderError::EventLoopTerminated)
        );
    }
}
