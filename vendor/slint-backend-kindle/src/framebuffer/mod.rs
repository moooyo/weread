mod ffi;
mod night_mode;

use crate::refresh::{RefreshWaveform, ResolvedRefresh};
use std::cell::Cell;
use std::ops::Range;
use std::os::fd::AsRawFd;

use ffi::{
    EPDC_FLAG_FORCE_MONOCHROME, FBIOGET_FSCREENINFO, FBIOGET_VSCREENINFO, FBIOPUT_VSCREENINFO,
    FbFixScreeninfo, FbVarScreeninfo, MXCFB_SEND_UPDATE, MXCFB_SEND_UPDATE_MTK,
    MXCFB_SEND_UPDATE_REX, MXCFB_SEND_UPDATE_ZELDA, MXCFB_WAIT_FOR_UPDATE_COMPLETE,
    TEMP_USE_AMBIENT, UPDATE_MODE_FULL, UPDATE_MODE_PARTIAL, UpdateMarkerData, UpdateRect,
    UpdateRequest, UpdateRequestMtk, UpdateRequestRex, UpdateRequestZelda, WAVEFORM_MODE_A2_LEGACY,
    WAVEFORM_MODE_A2_MODERN, WAVEFORM_MODE_AUTO, WAVEFORM_MODE_DU, WAVEFORM_MODE_GC16,
    WAVEFORM_MODE_GCK16_MTK,
};
pub(crate) use night_mode::NightModeApplied;
use night_mode::{ModeIo, ModeSnapshot, NightModeState};

/// Which MXCFB update ioctl this kernel accepts, varies with the Kindle model
///
//We probe on first refresh and remember the winner instead of retrying the failing ioctl every frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateVariant {
    /// `MXCFB_SEND_UPDATE` — 72-byte struct, older devices.
    Legacy,
    /// `MXCFB_SEND_UPDATE_REX` — 80-byte struct, Paperwhite 10th gen and newer.
    Rex,
    /// `MXCFB_SEND_UPDATE_ZELDA` — 88-byte struct, KOA2/KOA3 (Oasis 2/3).
    Zelda,
    /// `MXCFB_SEND_UPDATE_MTK` — 96-byte struct, MediaTek devices (Basic 2022,
    /// Paperwhite 5, Scribe).
    Mtk,
    /// No variant was accepted. Cached so a hopeless kernel costs one probe
    /// and one error log total, not three failing ioctls per frame.
    Unsupported,
}

impl UpdateVariant {
    /// Probed oldest ABI first.
    const PROBE_ORDER: [UpdateVariant; 4] = [Self::Legacy, Self::Rex, Self::Zelda, Self::Mtk];
}

enum PackedUpdateRequest {
    Legacy(UpdateRequest),
    Rex(UpdateRequestRex),
    Zelda(UpdateRequestZelda),
    Mtk(UpdateRequestMtk),
}

/// Memory-mapped handle to the Kindle's e-ink framebuffer.
///
/// Pixel format is 8-bit grayscale (one byte per pixel) on most models, or
/// 32 bpp on color models like the Colorsoft. The `stride` may be wider than
/// `width * bytes_per_pixel` due to hardware alignment requirements.
pub(crate) struct Framebuffer {
    file: std::fs::File,
    map: *mut u8,
    len: usize,
    pub(crate) width: u32,
    pub(crate) height: u32,
    stride: usize,
    /// Bytes per pixel in the mmap: 1 (8-bit grayscale, most Kindles) or
    /// 4 (32-bit BGRA/RGBA, e.g. Kindle Colorsoft).
    bytes_per_pixel: usize,
    /// The update ioctl variant this kernel accepts, cached after the first
    /// successful refresh. `None` until then.
    update_variant: Cell<Option<UpdateVariant>>,
    /// Failed attempts consume their identity too; no update marker is reused.
    last_issued_marker: Cell<u32>,
    last_submitted_marker: Cell<Option<u32>>,
    last_completed_marker: Cell<Option<u32>>,
    night_mode: NightModeState,
}

// SAFETY: The mmap is process-wide and we only access it from the event loop thread.
unsafe impl Send for Framebuffer {}

impl Framebuffer {
    /// Open the framebuffer device and query its geometry from the kernel.
    ///
    /// This works on any Kindle model - the resolution and stride are read at
    /// runtime rather than being hardcoded.
    pub(crate) fn open() -> std::io::Result<Self> {
        Self::open_with_hardware_inversion(false)
    }

    /// Hardware inversion is restricted by both application approval and GETVAR.
    pub(crate) fn open_with_hardware_inversion(enabled: bool) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/fb0")?;

        let fd = file.as_raw_fd();

        let snapshot = FramebufferModeIo(&file).read()?;
        let vinfo = snapshot.variable;
        let finfo = snapshot.fixed;

        let width = vinfo.xres;
        let height = vinfo.yres;
        let stride = finfo.line_length as usize;

        // Most Kindles expose an 8-bit grayscale framebuffer, but color models
        // (Kindle Colorsoft) expose a 32 bpp one. The render path produces one
        // gray byte per pixel; for 32 bpp we replicate it into all three color
        // channels, so the byte order (BGRA vs RGBA) doesn't matter.
        let bytes_per_pixel = match vinfo.bits_per_pixel {
            8 => 1,
            32 => 4,
            other => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unsupported framebuffer depth: {other} bpp (expected 8 or 32)"),
                ));
            }
        };

        if width == 0 || height == 0 || stride < width as usize * bytes_per_pixel {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "invalid framebuffer geometry: {width}x{height}, stride={stride}, bpp={}",
                    vinfo.bits_per_pixel
                ),
            ));
        }

        let len = stride * height as usize;

        let map = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };

        if map == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }

        Ok(Self {
            file,
            map: map as *mut u8,
            len,
            width,
            height,
            stride,
            bytes_per_pixel,
            update_variant: Cell::new(None),
            last_issued_marker: Cell::new(0),
            last_submitted_marker: Cell::new(None),
            last_completed_marker: Cell::new(None),
            night_mode: NightModeState::new(snapshot, enabled),
        })
    }

    pub(crate) fn apply_night_mode(&mut self, desired: bool) -> std::io::Result<NightModeApplied> {
        self.night_mode
            .apply(&mut FramebufferModeIo(&self.file), desired)
    }

    pub(crate) fn applied_night_mode(&self) -> Option<bool> {
        self.night_mode.applied_desired()
    }

    pub(crate) fn native_output_inverted(&self) -> bool {
        self.night_mode.native_inverse()
    }

    pub(crate) fn software_inversion(&self, desired: bool) -> bool {
        self.night_mode.software_inverse(desired)
    }

    /// Returns the compensation for the retained, already presented app mode.
    pub(crate) fn prepare_native_snapshot(&mut self) -> std::io::Result<bool> {
        self.night_mode
            .prepare_native_snapshot(&mut FramebufferModeIo(&self.file))
    }

    pub(crate) fn yield_native_display(&mut self) {
        self.night_mode.yield_native_ownership();
    }

    pub(crate) fn observe_native_night_mode(&mut self) -> std::io::Result<()> {
        self.night_mode
            .observe_native(&mut FramebufferModeIo(&self.file))
    }

    /// Read current scanout rotation without changing native display settings.
    /// Geometry changes cannot be applied to the retained pixel buffer.
    pub(crate) fn portrait_rotation(&self) -> std::io::Result<u32> {
        let info = read_variable_info(&self.file)?;
        if info.xres != self.width
            || info.yres != self.height
            || info.bits_per_pixel as usize != self.bytes_per_pixel * 8
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "native framebuffer geometry changed during portrait observation",
            ));
        }
        validated_portrait_rotation(info.rotate)
    }

    /// Write a horizontal span of grayscale pixels into the mmap at row `y`.
    pub(crate) fn write_line(&mut self, y: usize, x_range: Range<usize>, pixels: &[u8]) {
        let bpp = self.bytes_per_pixel;
        let row_span = unsafe {
            std::slice::from_raw_parts_mut(
                self.map.add(y * self.stride + x_range.start * bpp),
                pixels.len() * bpp,
            )
        };
        match bpp {
            1 => row_span.copy_from_slice(pixels),
            _ => {
                for (dst, &g) in row_span.chunks_exact_mut(4).zip(pixels) {
                    dst.copy_from_slice(&[g, g, g, 0xff]);
                }
            }
        }
    }

    /// Fill the entire visible area with a single grayscale value (0x00 = black, 0xff = white).
    pub(crate) fn fill(&mut self, value: u8) {
        let bpp = self.bytes_per_pixel;
        for y in 0..self.height as usize {
            let row = unsafe {
                std::slice::from_raw_parts_mut(
                    self.map.add(y * self.stride),
                    self.width as usize * bpp,
                )
            };
            match bpp {
                1 => row.fill(value),
                _ => {
                    for px in row.chunks_exact_mut(4) {
                        px.copy_from_slice(&[value, value, value, 0xff]);
                    }
                }
            }
        }
    }

    /// Ask the EPDC to refresh a region of the e-ink panel.
    ///
    /// On the first call we probe which update ioctl the kernel accepts and
    /// cache it, so later frames issue exactly one ioctl instead of retrying a
    /// known-failing one every refresh.
    fn send_update(&self, region: UpdateRect, waveform: u32, mode: u32) {
        let _ = self.send_update_checked(region, waveform, mode);
    }

    fn send_update_checked(&self, region: UpdateRect, waveform: u32, mode: u32) -> bool {
        self.submit_update_with(|variant, marker| {
            self.send_update_variant(variant, region, waveform, mode, marker)
        })
    }

    fn submit_update_with(&self, mut send: impl FnMut(UpdateVariant, u32) -> bool) -> bool {
        let Some(marker) = self.last_issued_marker.get().checked_add(1) else {
            return false;
        };
        // Consume failed identities as well. A later successful update cannot
        // accidentally reuse a marker from an uncertain rejected request.
        self.last_issued_marker.set(marker);
        let accepted = match self.update_variant.get() {
            Some(variant) => send(variant, marker),
            None => {
                let accepted = UpdateVariant::PROBE_ORDER
                    .into_iter()
                    .find(|&variant| send(variant, marker));
                if accepted.is_none() {
                    log::error!(
                        "EPDC refresh failed: no known MXCFB_SEND_UPDATE variant \
                         was accepted; the screen will not update"
                    );
                }
                self.update_variant
                    .set(Some(accepted.unwrap_or(UpdateVariant::Unsupported)));
                accepted.is_some()
            }
        };
        if accepted {
            self.last_submitted_marker.set(Some(marker));
        }
        accepted
    }

    /// Issue one specific update ioctl variant. Returns whether it succeeded.
    fn send_update_variant(
        &self,
        variant: UpdateVariant,
        region: UpdateRect,
        waveform: u32,
        mode: u32,
        marker: u32,
    ) -> bool {
        match self.pack_update_request(variant, region, waveform, mode, marker) {
            Some(PackedUpdateRequest::Legacy(update)) => {
                self.send_update_ioctl(MXCFB_SEND_UPDATE, &update)
            }
            Some(PackedUpdateRequest::Rex(update)) => {
                self.send_update_ioctl(MXCFB_SEND_UPDATE_REX, &update)
            }
            Some(PackedUpdateRequest::Zelda(update)) => {
                self.send_update_ioctl(MXCFB_SEND_UPDATE_ZELDA, &update)
            }
            Some(PackedUpdateRequest::Mtk(update)) => {
                self.send_update_ioctl(MXCFB_SEND_UPDATE_MTK, &update)
            }
            None => false,
        }
    }

    fn pack_update_request(
        &self,
        variant: UpdateVariant,
        region: UpdateRect,
        waveform: u32,
        mode: u32,
        marker: u32,
    ) -> Option<PackedUpdateRequest> {
        let flags = if waveform == WAVEFORM_MODE_DU {
            EPDC_FLAG_FORCE_MONOCHROME
        } else {
            0
        };
        match variant {
            UpdateVariant::Legacy => Some(PackedUpdateRequest::Legacy(UpdateRequest {
                update_region: region,
                waveform_mode: waveform,
                update_mode: mode,
                update_marker: marker,
                temperature: TEMP_USE_AMBIENT,
                flags,
                ..Default::default()
            })),
            UpdateVariant::Rex => Some(PackedUpdateRequest::Rex(UpdateRequestRex {
                update_region: region,
                waveform_mode: waveform,
                update_mode: mode,
                update_marker: marker,
                temperature: TEMP_USE_AMBIENT,
                flags,
                ..Default::default()
            })),
            UpdateVariant::Zelda => Some(PackedUpdateRequest::Zelda(UpdateRequestZelda {
                update_region: region,
                waveform_mode: waveform,
                update_mode: mode,
                update_marker: marker,
                temperature: TEMP_USE_AMBIENT,
                flags,
                ..Default::default()
            })),
            UpdateVariant::Mtk => Some(PackedUpdateRequest::Mtk(
                self.mtk_update_request(region, waveform, mode, marker),
            )),
            UpdateVariant::Unsupported => None,
        }
    }

    /// Hand one of the `UpdateRequest*` structs to the kernel. Returns whether
    /// the ioctl succeeded.
    fn send_update_ioctl<T>(&self, request: libc::c_ulong, update: &T) -> bool {
        // SAFETY: `update` outlives the ioctl and matches the kernel's struct.
        unsafe { libc::ioctl(self.file.as_raw_fd(), request as _, update as *const _) != -1 }
    }

    fn mtk_update_request(
        &self,
        region: UpdateRect,
        waveform: u32,
        mode: u32,
        marker: u32,
    ) -> UpdateRequestMtk {
        // Map at final packing so the uncached first MTK probe uses GCK16 too.
        // Global grayscale inversion is already active; no EPDC inversion flag is added.
        // https://github.com/koreader/koreader-base/blob/1c9682506757d5fd30cfa01c5698929dcc35340f/ffi/framebuffer_mxcfb.lua#L433-L455
        let waveform = if self.night_mode.hardware_inverse()
            && waveform == WAVEFORM_MODE_GC16
            && mode == UPDATE_MODE_FULL
        {
            WAVEFORM_MODE_GCK16_MTK
        } else {
            waveform
        };
        UpdateRequestMtk {
            update_region: region,
            waveform_mode: waveform,
            update_mode: mode,
            update_marker: marker,
            temperature: TEMP_USE_AMBIENT,
            flags: if waveform == WAVEFORM_MODE_DU {
                EPDC_FLAG_FORCE_MONOCHROME
            } else {
                0
            },
            ..Default::default()
        }
    }

    /// Apply a request resolved against the current framebuffer bounds.
    pub(crate) fn refresh_planned(&self, refresh: ResolvedRefresh) {
        if refresh.waveform == RefreshWaveform::Auto && !refresh.flashing {
            self.refresh_region(
                slint::PhysicalPosition::new(refresh.region.x, refresh.region.y),
                slint::PhysicalSize::new(refresh.region.width, refresh.region.height),
            );
            return;
        }
        let waveform = waveform_for_driver(refresh.waveform, self.update_variant.get());
        self.send_update(
            UpdateRect {
                top: refresh.region.y as u32,
                left: refresh.region.x as u32,
                width: refresh.region.width,
                height: refresh.region.height,
            },
            waveform,
            if refresh.flashing {
                UPDATE_MODE_FULL
            } else {
                UPDATE_MODE_PARTIAL
            },
        );
    }

    /// Full-screen GC16 refresh
    pub(crate) fn refresh_full(&self) {
        self.send_update(
            UpdateRect {
                top: 0,
                left: 0,
                width: self.width,
                height: self.height,
            },
            WAVEFORM_MODE_GC16,
            UPDATE_MODE_FULL,
        );
    }

    /// Require acceptance of a full-frame presentation or resident wake update.
    /// This does not add an unbounded update-completion wait to the power handshake.
    pub(crate) fn refresh_full_checked(&self) -> std::io::Result<()> {
        if self.last_issued_marker.get() == u32::MAX {
            return Err(std::io::Error::other(
                "the EPDC update marker sequence is exhausted",
            ));
        }
        if self.send_update_checked(
            UpdateRect {
                top: 0,
                left: 0,
                width: self.width,
                height: self.height,
            },
            WAVEFORM_MODE_GC16,
            UPDATE_MODE_FULL,
        ) {
            Ok(())
        } else {
            Err(std::io::Error::other(
                "the EPDC rejected the full-frame update",
            ))
        }
    }

    /// Block until the EPDC has applied the latest successfully submitted update.
    ///
    /// Used before suspending to RAM so the panel doesn't latch mid-refresh.
    /// Best-effort: a failing ioctl is ignored, since this is purely defensive.
    pub(crate) fn wait_for_update_complete(&self) {
        let _ = self.wait_for_update_complete_checked();
    }

    /// Require completion evidence before acknowledging a sleep presentation.
    pub(crate) fn wait_for_update_complete_checked(&self) -> std::io::Result<()> {
        self.wait_for_update_complete_with(|marker| {
            // SAFETY: the framebuffer descriptor is live and marker has the ABI
            // expected by MXCFB_WAIT_FOR_UPDATE_COMPLETE for this synchronous call.
            let result = unsafe {
                libc::ioctl(
                    self.file.as_raw_fd(),
                    MXCFB_WAIT_FOR_UPDATE_COMPLETE as _,
                    marker as *mut _,
                )
            };
            if result < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        })
    }

    /// Keep a preceding asynchronous update from reading a rewritten framebuffer.
    /// With no submitted update there is no preceding DMA operation to await.
    pub(crate) fn wait_for_pending_update_complete_checked(&self) -> std::io::Result<()> {
        if self.last_submitted_marker.get().is_none() {
            return Ok(());
        }
        self.wait_for_update_complete_checked()
    }

    fn wait_for_update_complete_with(
        &self,
        wait: impl FnOnce(&mut UpdateMarkerData) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        let submitted = self
            .last_submitted_marker
            .get()
            .ok_or_else(|| std::io::Error::other("no EPDC update was successfully submitted"))?;
        if self.last_completed_marker.get() == Some(submitted) {
            return Ok(());
        }
        let mut marker = UpdateMarkerData {
            update_marker: submitted,
            collision_test: 0,
        };
        wait(&mut marker)?;
        self.last_completed_marker.set(Some(submitted));
        Ok(())
    }

    /// Partial refresh of a dirty rectangle
    pub(crate) fn refresh_region(
        &self,
        origin: slint::PhysicalPosition,
        size: slint::PhysicalSize,
    ) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.send_update(
            UpdateRect {
                top: origin.y as u32,
                left: origin.x as u32,
                width: size.width,
                height: size.height,
            },
            WAVEFORM_MODE_AUTO,
            UPDATE_MODE_PARTIAL,
        );
    }
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        if let Err(error) = self
            .night_mode
            .restore_startup(&mut FramebufferModeIo(&self.file))
        {
            log::error!("failed to restore startup framebuffer grayscale: {error}");
        }
        unsafe { libc::munmap(self.map as *mut libc::c_void, self.len) };
    }
}

fn waveform_for_driver(waveform: RefreshWaveform, variant: Option<UpdateVariant>) -> u32 {
    match waveform {
        RefreshWaveform::Auto => WAVEFORM_MODE_AUTO,
        RefreshWaveform::Gc16 => WAVEFORM_MODE_GC16,
        RefreshWaveform::Du => WAVEFORM_MODE_DU,
        RefreshWaveform::A2 => match variant {
            Some(UpdateVariant::Legacy) => WAVEFORM_MODE_A2_LEGACY,
            Some(UpdateVariant::Rex | UpdateVariant::Zelda | UpdateVariant::Mtk) => {
                WAVEFORM_MODE_A2_MODERN
            }
            _ => WAVEFORM_MODE_AUTO,
        },
    }
}

fn read_variable_info(file: &std::fs::File) -> std::io::Result<FbVarScreeninfo> {
    let mut info = FbVarScreeninfo::default();
    // SAFETY: The initialized structure matches FBIOGET_VSCREENINFO and lives
    // for the read-only ioctl. The kernel does not retain its pointer.
    if unsafe { libc::ioctl(file.as_raw_fd(), FBIOGET_VSCREENINFO as _, &mut info) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(info)
}

struct FramebufferModeIo<'a>(&'a std::fs::File);

impl ModeIo for FramebufferModeIo<'_> {
    fn read(&mut self) -> std::io::Result<ModeSnapshot> {
        let variable = read_variable_info(self.0)?;
        let mut fixed = FbFixScreeninfo::default();
        // SAFETY: The initialized structure matches FBIOGET_FSCREENINFO and
        // remains live for the ioctl; the kernel does not retain its pointer.
        if unsafe { libc::ioctl(self.0.as_raw_fd(), FBIOGET_FSCREENINFO as _, &mut fixed) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(ModeSnapshot { variable, fixed })
    }

    fn write(&mut self, variable: &FbVarScreeninfo) -> std::io::Result<()> {
        // The Linux ioctl may normalize its argument. Only the paired fresh
        // GETVAR/GETFIX result is accepted as the observed hardware state.
        let mut requested = *variable;
        // SAFETY: The complete Linux fb_var_screeninfo layout is initialized
        // and remains live for FBIOPUT_VSCREENINFO, which retains no pointer.
        if unsafe { libc::ioctl(self.0.as_raw_fd(), FBIOPUT_VSCREENINFO as _, &mut requested) } < 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

fn validated_portrait_rotation(rotation: u32) -> std::io::Result<u32> {
    if rotation <= 3 {
        Ok(rotation)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "native framebuffer rotation is outside 0..=3",
        ))
    }
}

/// Metadata-only launcher probe: no mmap, pixel write or EPDC update.
pub(crate) fn probe_framebuffer_rotation() -> std::io::Result<u32> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open("/dev/fb0")?;
    validated_portrait_rotation(read_variable_info(&file)?.rotate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update_test_framebuffer(night_mode: NightModeState) -> Framebuffer {
        // This is anonymous memory and a non-device file; no Kindle is probed.
        let map = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                4096,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        assert_ne!(map, libc::MAP_FAILED);
        Framebuffer {
            file: std::fs::File::open("/dev/null").unwrap(),
            map: map.cast::<u8>(),
            len: 4096,
            width: 2,
            height: 2,
            stride: 2,
            bytes_per_pixel: 1,
            update_variant: Cell::new(None),
            last_issued_marker: Cell::new(0),
            last_submitted_marker: Cell::new(None),
            last_completed_marker: Cell::new(None),
            night_mode,
        }
    }

    struct ModeFixture(ModeSnapshot);

    impl ModeIo for ModeFixture {
        fn read(&mut self) -> std::io::Result<ModeSnapshot> {
            Ok(self.0)
        }

        fn write(&mut self, variable: &FbVarScreeninfo) -> std::io::Result<()> {
            self.0.variable = *variable;
            Ok(())
        }
    }

    fn observed_night_mode(startup: u32, enabled: bool, desired: bool) -> NightModeState {
        let snapshot = ModeSnapshot {
            variable: FbVarScreeninfo {
                xres: 2,
                yres: 2,
                xres_virtual: 2,
                yres_virtual: 2,
                bits_per_pixel: 8,
                grayscale: startup,
                ..Default::default()
            },
            fixed: FbFixScreeninfo {
                line_length: 2,
                smem_len: 4096,
                ..Default::default()
            },
        };
        let mut mode = NightModeState::new(snapshot, enabled);
        mode.apply(&mut ModeFixture(snapshot), desired).unwrap();
        mode
    }

    #[test]
    fn a_rejected_completion_ioctl_cannot_confirm_a_sleep_frame() {
        let framebuffer = update_test_framebuffer(observed_night_mode(1, false, false));
        framebuffer.last_submitted_marker.set(Some(7));
        assert!(framebuffer.wait_for_update_complete_checked().is_err());
        framebuffer.wait_for_update_complete();
    }

    fn packed_marker(request: PackedUpdateRequest) -> u32 {
        match request {
            PackedUpdateRequest::Legacy(update) => update.update_marker,
            PackedUpdateRequest::Rex(update) => update.update_marker,
            PackedUpdateRequest::Zelda(update) => update.update_marker,
            PackedUpdateRequest::Mtk(update) => update.update_marker,
        }
    }

    #[test]
    fn every_driver_waits_for_the_new_frame_instead_of_an_older_completed_marker() {
        for variant in UpdateVariant::PROBE_ORDER {
            let framebuffer = update_test_framebuffer(observed_night_mode(1, false, false));
            framebuffer.update_variant.set(Some(variant));
            let mut submitted = Vec::new();
            for mode in [UPDATE_MODE_PARTIAL, UPDATE_MODE_FULL] {
                assert!(framebuffer.submit_update_with(|driver, marker| {
                    assert_eq!(driver, variant);
                    let request = framebuffer
                        .pack_update_request(
                            driver,
                            UpdateRect::default(),
                            WAVEFORM_MODE_GC16,
                            mode,
                            marker,
                        )
                        .unwrap();
                    submitted.push(packed_marker(request));
                    true
                }));
            }
            assert_eq!(submitted, [1, 2]);
            let old_completed = submitted[0];
            assert!(
                framebuffer
                    .wait_for_update_complete_with(|record| {
                        assert_eq!(record.update_marker, submitted[1]);
                        if record.update_marker == old_completed {
                            Ok(())
                        } else {
                            Err(std::io::Error::from(std::io::ErrorKind::WouldBlock))
                        }
                    })
                    .is_err()
            );
            assert_eq!(framebuffer.last_completed_marker.get(), None);
            framebuffer
                .wait_for_update_complete_with(|record| {
                    assert_eq!(record.update_marker, submitted[1]);
                    Ok(())
                })
                .unwrap();
            assert_eq!(framebuffer.last_completed_marker.get(), Some(submitted[1]));
        }
    }

    #[test]
    fn failed_submissions_preserve_the_last_success_and_never_reuse_the_failed_identity() {
        let framebuffer = update_test_framebuffer(observed_night_mode(1, false, false));
        framebuffer.update_variant.set(Some(UpdateVariant::Mtk));
        assert!(framebuffer.submit_update_with(|_, marker| {
            assert_eq!(marker, 1);
            true
        }));
        assert!(!framebuffer.submit_update_with(|_, marker| {
            assert_eq!(marker, 2);
            false
        }));
        assert_eq!(framebuffer.last_submitted_marker.get(), Some(1));
        assert!(framebuffer.submit_update_with(|_, marker| {
            assert_eq!(marker, 3);
            true
        }));
        framebuffer
            .wait_for_update_complete_with(|record| {
                assert_eq!(record.update_marker, 3);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn driver_probes_share_one_logical_identity_and_cache_the_first_accepted_abi() {
        let framebuffer = update_test_framebuffer(observed_night_mode(1, false, false));
        let mut probes = Vec::new();
        assert!(framebuffer.submit_update_with(|variant, marker| {
            probes.push((variant, marker));
            assert_eq!(framebuffer.last_submitted_marker.get(), None);
            variant == UpdateVariant::Zelda
        }));
        assert_eq!(
            probes,
            [
                (UpdateVariant::Legacy, 1),
                (UpdateVariant::Rex, 1),
                (UpdateVariant::Zelda, 1),
            ]
        );
        assert_eq!(framebuffer.last_submitted_marker.get(), Some(1));
        assert_eq!(framebuffer.update_variant.get(), Some(UpdateVariant::Zelda));
        assert!(framebuffer.submit_update_with(|variant, marker| {
            assert_eq!((variant, marker), (UpdateVariant::Zelda, 2));
            true
        }));
        assert_eq!(framebuffer.last_submitted_marker.get(), Some(2));
    }

    #[test]
    fn absent_submissions_and_marker_exhaustion_cannot_manufacture_completion() {
        let framebuffer = update_test_framebuffer(observed_night_mode(1, false, false));
        assert!(
            framebuffer
                .wait_for_update_complete_with(|_| panic!("No update was submitted"))
                .is_err()
        );
        framebuffer
            .wait_for_pending_update_complete_checked()
            .unwrap();
        framebuffer.update_variant.set(Some(UpdateVariant::Mtk));
        framebuffer.last_issued_marker.set(u32::MAX - 1);
        assert!(framebuffer.submit_update_with(|_, marker| {
            assert_eq!(marker, u32::MAX);
            true
        }));
        assert!(!framebuffer.submit_update_with(|_, _| panic!("Markers cannot wrap to zero")));
        assert_eq!(framebuffer.last_submitted_marker.get(), Some(u32::MAX));
        assert!(framebuffer.refresh_full_checked().is_err());
        framebuffer
            .wait_for_update_complete_with(|record| {
                assert_eq!(record.update_marker, u32::MAX);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn only_a_successful_wait_is_cached_and_a_new_frame_requires_new_completion() {
        let framebuffer = update_test_framebuffer(observed_night_mode(1, false, false));
        framebuffer.update_variant.set(Some(UpdateVariant::Mtk));
        assert!(framebuffer.submit_update_with(|_, _| true));
        assert!(
            framebuffer
                .wait_for_update_complete_with(|_| Err(std::io::Error::other("interrupted wait")))
                .is_err()
        );
        assert_eq!(framebuffer.last_completed_marker.get(), None);
        framebuffer
            .wait_for_update_complete_with(|record| {
                assert_eq!(record.update_marker, 1);
                Ok(())
            })
            .unwrap();
        framebuffer
            .wait_for_update_complete_with(|_| panic!("A completed marker needs no repeated ioctl"))
            .unwrap();
        assert!(framebuffer.submit_update_with(|_, _| true));
        framebuffer
            .wait_for_update_complete_with(|record| {
                assert_eq!(record.update_marker, 2);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn mtk_night_full_requests_pack_gck16_before_the_driver_probe_is_cached() {
        let framebuffer = update_test_framebuffer(observed_night_mode(1, true, true));
        assert!(framebuffer.update_variant.get().is_none());
        assert!(!framebuffer.native_output_inverted());
        assert!(framebuffer.night_mode.hardware_inverse());
        for cached in [false, true] {
            if cached {
                framebuffer.update_variant.set(Some(UpdateVariant::Mtk));
            }
            let request = framebuffer.mtk_update_request(
                UpdateRect {
                    top: 0,
                    left: 0,
                    width: 2,
                    height: 2,
                },
                WAVEFORM_MODE_GC16,
                UPDATE_MODE_FULL,
                7,
            );
            assert_eq!(request.waveform_mode, 8);
            assert_eq!(request.update_mode, UPDATE_MODE_FULL);
            assert_eq!(
                request.flags, 0,
                "Global inversion must not be applied twice"
            );
            assert_eq!(
                (request.update_region.width, request.update_region.height),
                (2, 2)
            );
        }
    }

    #[test]
    fn mtk_day_and_software_only_full_requests_keep_gc16() {
        for (startup, hardware_enabled, desired) in
            [(2, true, false), (2, false, true), (1, false, true)]
        {
            let framebuffer =
                update_test_framebuffer(observed_night_mode(startup, hardware_enabled, desired));
            let request = framebuffer.mtk_update_request(
                UpdateRect::default(),
                WAVEFORM_MODE_GC16,
                UPDATE_MODE_FULL,
                8,
            );
            assert_eq!(request.waveform_mode, 2);
            assert_eq!(request.flags, 0);
        }
    }

    #[test]
    fn mtk_partial_and_other_waveforms_keep_their_requested_value() {
        let framebuffer = update_test_framebuffer(observed_night_mode(1, true, true));
        for (waveform, mode, flags) in [
            (WAVEFORM_MODE_GC16, UPDATE_MODE_PARTIAL, 0),
            (WAVEFORM_MODE_AUTO, UPDATE_MODE_FULL, 0),
            (
                WAVEFORM_MODE_DU,
                UPDATE_MODE_FULL,
                EPDC_FLAG_FORCE_MONOCHROME,
            ),
        ] {
            let request = framebuffer.mtk_update_request(UpdateRect::default(), waveform, mode, 9);
            assert_eq!(request.waveform_mode, waveform);
            assert_eq!(request.update_mode, mode);
            assert_eq!(request.flags, flags);
        }
    }

    #[test]
    fn a_rejected_wake_update_is_not_acknowledged_as_a_restored_panel() {
        let framebuffer = update_test_framebuffer(NightModeState::new(
            ModeSnapshot {
                variable: FbVarScreeninfo::default(),
                fixed: FbFixScreeninfo::default(),
            },
            false,
        ));
        assert!(framebuffer.refresh_full_checked().is_err());
        assert!(matches!(
            framebuffer.update_variant.get(),
            Some(UpdateVariant::Unsupported)
        ));
        // Ordinary refresh keeps its former best-effort behavior.
        framebuffer.refresh_full();
        assert!(framebuffer.refresh_full_checked().is_err());
    }

    #[test]
    fn waveform_constants_follow_the_detected_driver_family() {
        assert_eq!(
            waveform_for_driver(RefreshWaveform::A2, Some(UpdateVariant::Legacy)),
            4
        );
        for variant in [UpdateVariant::Rex, UpdateVariant::Zelda, UpdateVariant::Mtk] {
            assert_eq!(waveform_for_driver(RefreshWaveform::A2, Some(variant)), 6);
        }
        assert_eq!(
            waveform_for_driver(RefreshWaveform::A2, None),
            WAVEFORM_MODE_AUTO
        );
        assert_eq!(
            waveform_for_driver(RefreshWaveform::A2, Some(UpdateVariant::Unsupported)),
            WAVEFORM_MODE_AUTO
        );
        assert_eq!(
            waveform_for_driver(RefreshWaveform::Du, Some(UpdateVariant::Mtk)),
            1
        );
        assert_eq!(
            waveform_for_driver(RefreshWaveform::Gc16, Some(UpdateVariant::Mtk)),
            2
        );
        for variant in [
            UpdateVariant::Legacy,
            UpdateVariant::Rex,
            UpdateVariant::Zelda,
        ] {
            assert_eq!(waveform_for_driver(RefreshWaveform::Gc16, Some(variant)), 2);
        }
    }
}
