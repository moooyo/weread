//! Renderer-owned Linux framebuffer inversion with an explicit hardware opt-in.

use std::io;

use super::ffi::{FbFixScreeninfo, FbVarScreeninfo};

#[derive(Clone, Copy, Debug)]
pub(super) struct ModeSnapshot {
    pub(super) variable: FbVarScreeninfo,
    pub(super) fixed: FbFixScreeninfo,
}

impl ModeSnapshot {
    fn kernel_inverse(self) -> bool {
        self.variable.bits_per_pixel == 8 && self.variable.grayscale == 2
    }

    fn hardware_supported(self) -> bool {
        self.variable.bits_per_pixel == 8 && matches!(self.variable.grayscale, 1 | 2)
    }

    fn same_layout(self, other: Self) -> bool {
        let current = self.variable;
        let expected = other.variable;
        current.xres == expected.xres
            && current.yres == expected.yres
            && current.xres_virtual == expected.xres_virtual
            && current.yres_virtual == expected.yres_virtual
            && current.xoffset == expected.xoffset
            && current.yoffset == expected.yoffset
            && current.bits_per_pixel == expected.bits_per_pixel
            && self.fixed.line_length == other.fixed.line_length
            && self.fixed.smem_start == other.fixed.smem_start
            && self.fixed.smem_len == other.fixed.smem_len
            && self.fixed.type_ == other.fixed.type_
            && self.fixed.visual == other.fixed.visual
    }
}

pub(super) trait ModeIo {
    fn read(&mut self) -> io::Result<ModeSnapshot>;
    fn write(&mut self, variable: &FbVarScreeninfo) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NightModeApplied {
    pub(crate) software_inverse: bool,
    pub(crate) changed: bool,
}

/// The startup value is for process exit; the fresh native value is for sleep.
/// Neither value is inferred from a UI preference or from a successful write.
pub(super) struct NightModeState {
    mapping: ModeSnapshot,
    observed: ModeSnapshot,
    original_grayscale: u32,
    native_grayscale: u32,
    hardware_enabled: bool,
    restore_original: bool,
    app_owns_mode: bool,
    applied_desired: Option<bool>,
    retained_desired: bool,
}

impl NightModeState {
    pub(super) fn new(initial: ModeSnapshot, hardware_enabled: bool) -> Self {
        Self {
            mapping: initial,
            observed: initial,
            original_grayscale: initial.variable.grayscale,
            native_grayscale: initial.variable.grayscale,
            hardware_enabled: hardware_enabled && initial.hardware_supported(),
            restore_original: false,
            app_owns_mode: false,
            applied_desired: None,
            retained_desired: false,
        }
    }

    pub(super) fn native_inverse(&self) -> bool {
        self.mapping.variable.bits_per_pixel == 8 && self.native_grayscale == 2
    }

    /// Current renderer hardware state, separate from the retained native baseline.
    pub(super) fn hardware_inverse(&self) -> bool {
        self.hardware_enabled && self.observed.kernel_inverse()
    }

    pub(super) fn software_inverse(&self, desired: bool) -> bool {
        desired ^ self.observed.kernel_inverse()
    }

    pub(super) fn applied_desired(&self) -> Option<bool> {
        self.applied_desired
    }

    pub(super) fn apply(
        &mut self,
        io: &mut impl ModeIo,
        desired: bool,
    ) -> io::Result<NightModeApplied> {
        // A failed mode write still belongs to the active renderer and requires
        // startup cleanup. Native ownership begins only after a coherent yield.
        self.app_owns_mode = true;
        if self.applied_desired == Some(desired) {
            return Ok(NightModeApplied {
                software_inverse: self.software_inverse(desired),
                changed: false,
            });
        }
        let before = self.read_checked(io)?;
        let observed = if self.hardware_enabled && before.hardware_supported() {
            self.set_grayscale(io, before, if desired { 2 } else { 1 })?
        } else {
            before
        };
        self.observed = observed;
        self.retained_desired = desired;
        self.applied_desired = Some(desired);
        Ok(NightModeApplied {
            software_inverse: self.software_inverse(desired),
            changed: true,
        })
    }

    /// Restore metadata before rewriting canonical pixels for the frame guard.
    /// The caller must complete that rewrite before publishing Suspended.
    pub(super) fn prepare_native_snapshot(&mut self, io: &mut impl ModeIo) -> io::Result<bool> {
        // A reversed wake can return here before apply. Preparing raw pixels
        // owns the handoff until the caller commits its complete rewrite.
        self.app_owns_mode = true;
        let before = self.read_checked(io)?;
        let observed = if self.hardware_enabled {
            self.set_grayscale(io, before, self.native_grayscale)?
        } else {
            before
        };
        self.observed = observed;
        self.applied_desired = None;
        Ok(self.software_inverse(self.retained_desired))
    }

    /// Commit only after the caller has completely rewritten compensated raw RGB.
    /// Until then, a failure or unwinding panic retains startup cleanup authority.
    pub(super) fn yield_native_ownership(&mut self) {
        self.app_owns_mode = false;
        self.applied_desired = None;
    }

    /// Native UI may change its own grayscale while this process is suspended.
    pub(super) fn observe_native(&mut self, io: &mut impl ModeIo) -> io::Result<()> {
        let observed = self.read_checked(io)?;
        self.native_grayscale = observed.variable.grayscale;
        self.observed = observed;
        self.applied_desired = None;
        self.app_owns_mode = false;
        Ok(())
    }

    pub(super) fn restore_startup(&mut self, io: &mut impl ModeIo) -> io::Result<()> {
        if !self.app_owns_mode || !self.restore_original {
            return Ok(());
        }
        let before = self.read_checked(io)?;
        self.observed = self.set_grayscale(io, before, self.original_grayscale)?;
        self.restore_original = false;
        Ok(())
    }

    fn read_checked(&self, io: &mut impl ModeIo) -> io::Result<ModeSnapshot> {
        let observed = io.read()?;
        if !observed.same_layout(self.mapping) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "framebuffer layout changed during night-mode ownership",
            ));
        }
        Ok(observed)
    }

    fn set_grayscale(
        &mut self,
        io: &mut impl ModeIo,
        before: ModeSnapshot,
        grayscale: u32,
    ) -> io::Result<ModeSnapshot> {
        if before.variable.grayscale == grayscale {
            return Ok(before);
        }
        if !before.hardware_supported() || !matches!(grayscale, 1 | 2) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "hardware inversion requires an 8-bit grayscale 1 or 2 framebuffer",
            ));
        }
        let mut requested = before.variable;
        requested.grayscale = grayscale;
        // A failed ioctl can still leave hardware state uncertain. Drop must try
        // to restore the original value even when write/readback fails.
        self.restore_original = true;
        io.write(&requested)?;
        let observed = self.read_checked(io)?;
        if !observed.same_layout(before)
            || observed.variable.grayscale != grayscale
            || observed.variable.rotate != before.variable.rotate
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "framebuffer night-mode write did not match its readback",
            ));
        }
        Ok(observed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockFramebuffer {
        current: ModeSnapshot,
        writes: Vec<FbVarScreeninfo>,
        reads: usize,
        fail_write: bool,
        fail_read: Option<usize>,
        ignore_write: bool,
        change_layout: bool,
        reset_rotation: bool,
    }

    impl MockFramebuffer {
        fn new(grayscale: u32) -> Self {
            Self {
                current: ModeSnapshot {
                    variable: FbVarScreeninfo {
                        xres: 3,
                        yres: 2,
                        xres_virtual: 3,
                        yres_virtual: 4,
                        bits_per_pixel: 8,
                        grayscale,
                        rotate: 3,
                        pixclock: 17,
                        reserved: [11, 12, 13, 14],
                        ..Default::default()
                    },
                    fixed: FbFixScreeninfo {
                        line_length: 6,
                        smem_len: 24,
                        ..Default::default()
                    },
                },
                writes: Vec::new(),
                reads: 0,
                fail_write: false,
                fail_read: None,
                ignore_write: false,
                change_layout: false,
                reset_rotation: false,
            }
        }

        fn controller(&self, hardware_enabled: bool) -> NightModeState {
            NightModeState::new(self.current, hardware_enabled)
        }
    }

    impl ModeIo for MockFramebuffer {
        fn read(&mut self) -> io::Result<ModeSnapshot> {
            self.reads += 1;
            if self.fail_read == Some(self.reads) {
                return Err(io::Error::other("Synthetic GETVAR failure"));
            }
            Ok(self.current)
        }

        fn write(&mut self, variable: &FbVarScreeninfo) -> io::Result<()> {
            self.writes.push(*variable);
            if self.fail_write {
                return Err(io::Error::other("Synthetic PUTVAR failure"));
            }
            if !self.ignore_write {
                self.current.variable = *variable;
            }
            if self.change_layout {
                self.current.fixed.line_length += 1;
            }
            if self.reset_rotation {
                self.current.variable.rotate = 0;
            }
            Ok(())
        }
    }

    #[test]
    fn hardware_transitions_preserve_fresh_metadata_and_never_double_invert() {
        let mut io = MockFramebuffer::new(1);
        let original = io.current.variable;
        let mut state = io.controller(true);
        assert_eq!(state.apply(&mut io, false).unwrap().software_inverse, false);
        io.current.variable.rotate = 1;
        let dark = state.apply(&mut io, true).unwrap();
        assert_eq!(
            dark,
            NightModeApplied {
                software_inverse: false,
                changed: true
            }
        );
        let mut expected = original;
        expected.grayscale = 2;
        expected.rotate = 1;
        assert_eq!(io.writes, [expected]);
        assert_eq!(state.apply(&mut io, true).unwrap().changed, false);
        let light = state.apply(&mut io, false).unwrap();
        assert_eq!(
            light,
            NightModeApplied {
                software_inverse: false,
                changed: true
            }
        );
        assert_eq!(io.current.variable.grayscale, 1);
        assert_eq!(io.writes.len(), 2);
        for desired in [false, true] {
            let applied = state.apply(&mut io, desired).unwrap();
            let raw = if applied.software_inverse {
                255 - 210
            } else {
                210
            };
            let physical = if io.current.kernel_inverse() {
                255 - raw
            } else {
                raw
            };
            assert_eq!(physical, if desired { 45 } else { 210 });
        }
    }

    #[test]
    fn software_fallback_compensates_a_preexisting_hardware_inverse() {
        for (depth, grayscale, enabled) in
            [(8, 1, false), (8, 2, false), (8, 0, true), (32, 1, true)]
        {
            let mut io = MockFramebuffer::new(grayscale);
            io.current.variable.bits_per_pixel = depth;
            let mut state = io.controller(enabled);
            for desired in [false, true, false] {
                let applied = state.apply(&mut io, desired).unwrap();
                assert_eq!(
                    applied.software_inverse ^ io.current.kernel_inverse(),
                    desired
                );
            }
            state.restore_startup(&mut io).unwrap();
            assert!(io.writes.is_empty());
        }
    }

    #[test]
    fn startup_inverse_is_restored_using_current_nonmode_fields() {
        let mut io = MockFramebuffer::new(2);
        let mut state = io.controller(true);
        state.apply(&mut io, false).unwrap();
        assert_eq!(io.current.variable.grayscale, 1);
        io.current.variable.rotate = 1;
        state.restore_startup(&mut io).unwrap();
        assert_eq!(io.current.variable.grayscale, 2);
        assert_eq!(io.current.variable.rotate, 1);
        let writes = io.writes.len();
        state.restore_startup(&mut io).unwrap();
        assert_eq!(io.writes.len(), writes);
    }

    #[test]
    fn failed_write_or_readback_never_acknowledges_an_applied_request() {
        for failure in 0..5 {
            let mut io = MockFramebuffer::new(1);
            let mut state = io.controller(true);
            match failure {
                0 => io.fail_write = true,
                1 => io.ignore_write = true,
                2 => io.fail_read = Some(2),
                3 => io.change_layout = true,
                _ => io.reset_rotation = true,
            }
            assert!(state.apply(&mut io, true).is_err());
            assert_eq!(state.applied_desired(), None);
            assert!(state.restore_original);
            io.fail_write = false;
            io.ignore_write = false;
            io.fail_read = None;
            io.current.fixed.line_length = 6;
            io.change_layout = false;
            io.reset_rotation = false;
            state.restore_startup(&mut io).unwrap();
            assert_eq!(io.current.variable.grayscale, 1);
        }
    }

    #[test]
    fn changed_mapping_or_failed_initial_observation_never_reaches_a_mode_write() {
        for changed_layout in [false, true] {
            let mut io = MockFramebuffer::new(1);
            let mut state = io.controller(true);
            if changed_layout {
                io.current.variable.xoffset = 1;
            } else {
                io.fail_read = Some(1);
            }
            assert!(state.apply(&mut io, true).is_err());
            assert!(io.writes.is_empty());
            assert_eq!(state.applied_desired(), None);
        }
    }

    #[test]
    fn canonical_pixels_have_identical_physical_output_in_hardware_and_software_modes() {
        use crate::render_control::{GrayMode, GrayRenderer};
        use slint::Rgb8Pixel;
        let rgb = [255, 0, 127, 200].map(|value| Rgb8Pixel {
            r: value,
            g: value,
            b: value,
        });
        let original = rgb;
        for (initial, enabled) in [(1, true), (2, true), (1, false), (2, false)] {
            let mut io = MockFramebuffer::new(initial);
            let mut state = io.controller(enabled);
            let mut renderer = GrayRenderer::new(4, GrayMode::default());
            let mut raw = [0; 4];
            for desired in [false, true, true, false] {
                let applied = state.apply(&mut io, desired).unwrap();
                renderer.write_frame(
                    &rgb,
                    (4, 1),
                    None,
                    GrayMode {
                        dark_mode: applied.software_inverse,
                        black_and_white: false,
                    },
                    applied.changed,
                    |_, columns, pixels| raw[columns].copy_from_slice(pixels),
                );
                let physical = raw.map(|value| {
                    if io.current.kernel_inverse() {
                        255 - value
                    } else {
                        value
                    }
                });
                assert_eq!(
                    physical,
                    if desired {
                        [0, 255, 128, 55]
                    } else {
                        [255, 0, 127, 200]
                    }
                );
                assert_eq!(rgb, original);
            }
        }
    }

    #[test]
    fn exiting_after_native_yield_never_overwrites_a_new_native_mode() {
        let mut io = MockFramebuffer::new(1);
        let mut state = io.controller(true);
        state.apply(&mut io, true).unwrap();
        state.prepare_native_snapshot(&mut io).unwrap();
        state.yield_native_ownership();
        io.current.variable.grayscale = 2;
        let writes = io.writes.len();
        let reads = io.reads;
        state.restore_startup(&mut io).unwrap();
        assert_eq!(io.writes.len(), writes);
        assert_eq!(io.reads, reads);
        assert_eq!(io.current.variable.grayscale, 2);
        // The lifetime startup value remains available after a later wake.
        state.observe_native(&mut io).unwrap();
        state.apply(&mut io, true).unwrap();
        state.restore_startup(&mut io).unwrap();
        assert_eq!(io.current.variable.grayscale, 1);
    }

    #[test]
    fn failed_baseline_restore_keeps_active_cleanup_authority() {
        let mut io = MockFramebuffer::new(1);
        let mut state = io.controller(true);
        state.apply(&mut io, true).unwrap();
        io.fail_write = true;
        assert!(state.prepare_native_snapshot(&mut io).is_err());
        assert!(state.app_owns_mode && state.restore_original);
        io.fail_write = false;
        state.restore_startup(&mut io).unwrap();
        assert_eq!(io.current.variable.grayscale, 1);
    }

    #[test]
    fn failed_raw_rewrite_before_yield_keeps_startup_cleanup_authority() {
        use crate::render_control::{GrayMode, GrayRenderer};
        use slint::Rgb8Pixel;
        use std::panic::{AssertUnwindSafe, catch_unwind};
        let mut io = MockFramebuffer::new(1);
        let mut state = io.controller(true);
        state.apply(&mut io, true).unwrap();
        state.prepare_native_snapshot(&mut io).unwrap();
        state.yield_native_ownership();
        io.current.variable.grayscale = 2;
        state.observe_native(&mut io).unwrap();
        state.apply(&mut io, false).unwrap();
        let software_inverse = state.prepare_native_snapshot(&mut io).unwrap();
        assert_eq!(io.current.variable.grayscale, 2);
        let mut renderer = GrayRenderer::new(1, GrayMode::default());
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                renderer.write_frame(
                    &[Rgb8Pixel {
                        r: 255,
                        g: 255,
                        b: 255,
                    }],
                    (1, 1),
                    None,
                    GrayMode {
                        dark_mode: software_inverse,
                        black_and_white: false,
                    },
                    true,
                    |_, _, _| panic!("Synthetic raw framebuffer write failure"),
                );
            }))
            .is_err()
        );
        assert!(state.app_owns_mode && state.restore_original);
        state.restore_startup(&mut io).unwrap();
        assert_eq!(io.current.variable.grayscale, 1);
    }

    #[test]
    fn sleep_snapshot_is_coherent_and_wake_retains_the_app_request() {
        let mut io = MockFramebuffer::new(1);
        let mut state = io.controller(true);
        state.apply(&mut io, true).unwrap();
        let canonical = [255u8, 0, 95];
        let physical_before = canonical.map(|value| 255 - value);
        let software_inverse = state.prepare_native_snapshot(&mut io).unwrap();
        assert_eq!(io.current.variable.grayscale, 1);
        let raw_snapshot =
            canonical.map(|value| if software_inverse { 255 - value } else { value });
        assert_eq!(raw_snapshot, physical_before);
        assert_eq!(state.applied_desired(), None);
        state.yield_native_ownership();
        // Native dark mode changes while the application is suspended.
        io.current.variable.grayscale = 2;
        state.observe_native(&mut io).unwrap();
        assert!(state.native_inverse());
        assert_eq!(state.apply(&mut io, true).unwrap().software_inverse, false);
        assert_eq!(io.current.variable.grayscale, 2);
        let software_inverse = state.prepare_native_snapshot(&mut io).unwrap();
        assert!(!software_inverse);
        assert_eq!(io.current.variable.grayscale, 2);
        state.yield_native_ownership();
        state.observe_native(&mut io).unwrap();
        state.apply(&mut io, true).unwrap();
        state.restore_startup(&mut io).unwrap();
        assert_eq!(io.current.variable.grayscale, 1);
    }
}
