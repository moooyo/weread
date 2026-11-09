//! Renderer-owned night-mode requests and canonical RGB output transforms.

use std::fmt;
use std::ops::Range;
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use slint::Rgb8Pixel;

use crate::managed_sleep::{ManagedSleepControl, ManagedSleepState};
use crate::refresh::PhysicalRect;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderError {
    EventLoopTerminated,
    AlreadyInitialized,
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EventLoopTerminated => "the Kindle event loop has terminated",
            Self::AlreadyInitialized => "night-mode configuration must precede the event loop",
        })
    }
}

impl std::error::Error for RenderError {}

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
    pub(crate) wake: Arc<OwnedFd>,
    pub(crate) quit: Arc<AtomicBool>,
}

impl RenderHandle {
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
        if self.managed_sleep.request(sleeping) {
            crate::wakeup::poke(&self.wake);
        }
        Ok(())
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
        let value = ((77 * u32::from(pixel.r) + 150 * u32::from(pixel.g) + 29 * u32::from(pixel.b))
            >> 8) as u8;
        self.grayscale(value)
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

pub(crate) struct FrameWrite {
    pub(crate) region: Option<PhysicalRect>,
    pub(crate) mode_changed: bool,
}

/// Keeps only a scanline of transformed output. Slint retains the original RGB
/// frame, so partial redraws and repeated full updates cannot double-invert it.
pub(crate) struct GrayRenderer {
    applied_mode: GrayMode,
    line: Vec<u8>,
}

impl GrayRenderer {
    pub(crate) fn new(width: usize, initial_mode: GrayMode) -> Self {
        Self {
            applied_mode: initial_mode,
            line: vec![0; width],
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
                let start = row * width + x;
                let source = &rgb[start..start + span];
                let output = &mut self.line[..span];
                for (destination, source) in output.iter_mut().zip(source) {
                    *destination = mode.pixel(*source);
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
}
