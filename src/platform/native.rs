use super::native_command::run_command;
use super::{DeviceControl, DeviceProfile, PlatformCapabilities, PlatformError};
use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

/// Linux's stable fb_var_screeninfo layout consists of forty 32-bit words.
/// Visible resolution and panning offsets are checked without writing display state.
#[repr(C)]
struct FramebufferInfo {
    words: [u32; 40],
}

pub(super) struct NativeControl {
    get_property: Option<PathBuf>,
    set_property: Option<PathBuf>,
    native_viewport: (u32, u32),
    native_output_inverse: Option<bool>,
}

impl NativeControl {
    pub fn new(profile: DeviceProfile) -> Result<Self, PlatformError> {
        let firmware = std::env::var("WEREAD_APPROVED_FIRMWARE").unwrap_or_default();
        if std::env::var("WEREAD_APPROVED_PROFILE").ok().as_deref() != Some(profile.id())
            || firmware.trim().is_empty()
        {
            return Err(PlatformError::UnapprovedDevice);
        }
        let experimental_display = match std::env::var("WEREAD_EXPERIMENTAL_DISPLAY") {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(PlatformError::InvalidValue(
                    "the experimental display identifier must be valid Unicode",
                ));
            }
        };
        let expected =
            expected_framebuffer_size(profile, &firmware, experimental_display.as_deref())?;
        let native_viewport = validate_framebuffer(expected)?;
        crate::diagnostic!(
            "weread: native display geometry: expected={}x{}, actual={}x{}, offset=0,0, scale=2, experiment={}",
            expected.0,
            expected.1,
            native_viewport.0,
            native_viewport.1,
            experimental_display.as_deref().unwrap_or("nominal-design"),
        );
        Ok(Self {
            get_property: find_command("lipc-get-prop"),
            set_property: find_command("lipc-set-prop"),
            native_viewport,
            native_output_inverse: super::native_theme::startup_native_output_inverted(),
        })
    }

    fn get_integer(&self, property: &str) -> Result<u8, PlatformError> {
        let program = self
            .get_property
            .as_deref()
            .ok_or(PlatformError::Unsupported("lipc"))?;
        let output = run_command(program, &["com.lab126.powerd", property])?;
        output
            .trim()
            .parse::<u8>()
            .map_err(|_| PlatformError::Operation("powerd returned a nonnumeric value".into()))
    }

    fn set_integer(&self, property: &str, value: u8) -> Result<(), PlatformError> {
        let program = self
            .set_property
            .as_deref()
            .ok_or(PlatformError::Unsupported("lipc"))?;
        run_command(
            program,
            &["-i", "com.lab126.powerd", property, &value.to_string()],
        )?;
        Ok(())
    }
}

impl DeviceControl for NativeControl {
    fn native_viewport(&self) -> Option<(u32, u32)> {
        Some(self.native_viewport)
    }

    fn native_output_inverted(&self) -> Option<bool> {
        self.native_output_inverse
    }

    fn capabilities(&self) -> PlatformCapabilities {
        let writable = self.set_property.is_some();
        PlatformCapabilities {
            brightness: writable && self.get_property.is_some(),
            warmth: writable && self.get_property.is_some(),
            keep_awake: writable,
            // Hardware inversion remains separate from application rendering.
            hardware_inversion: false,
            application_dark_mode: false,
            // Pen input requires an additional reviewed backend interface.
            pen: false,
            explicit_refresh: false,
        }
    }

    fn brightness(&self) -> Result<u8, PlatformError> {
        self.get_integer("flIntensity")
    }

    fn set_brightness(&self, value: u8) -> Result<(), PlatformError> {
        if value > 24 {
            return Err(PlatformError::InvalidValue(
                "brightness must be between 0 and 24",
            ));
        }
        self.set_integer("flIntensity", value)
    }

    fn warmth(&self) -> Result<u8, PlatformError> {
        self.get_integer("currentAmberLevel")
    }

    fn set_warmth(&self, value: u8) -> Result<(), PlatformError> {
        if value > 24 {
            return Err(PlatformError::InvalidValue(
                "warmth must be between 0 and 24",
            ));
        }
        self.set_integer("currentAmberLevel", value)
    }

    fn battery_percentage(&self) -> Result<u8, PlatformError> {
        let value = self.get_integer("battLevel")?;
        if value > 100 {
            return Err(PlatformError::InvalidValue(
                "battery percentage must be in 0..=100",
            ));
        }
        Ok(value)
    }

    fn set_inverted(&self, _: bool) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("hardware_inversion"))
    }

    fn reset_idle_timer(&self) -> Result<(), PlatformError> {
        self.set_integer("touchScreenSaverTimeout", 1)
    }

    fn quiesce_commands(&self, timeout: std::time::Duration) -> Result<(), PlatformError> {
        super::native_command::quiesce_commands(timeout)
    }
}

fn expected_framebuffer_size(
    profile: DeviceProfile,
    firmware: &str,
    experimental_display: Option<&str>,
) -> Result<(u32, u32), PlatformError> {
    match experimental_display {
        None => Ok(profile.physical_size()),
        Some("pw6-5201-emulator")
            if profile == DeviceProfile::Paperwhite6 && firmware == "5.20.1" =>
        {
            // This experiment identifies one firmware emulator, not physical-device support.
            Ok((1272, 1696))
        }
        Some("pw6-5201-emulator") => Err(PlatformError::InvalidValue(
            "pw6-5201-emulator requires approved profile pw6 and firmware 5.20.1",
        )),
        Some(_) => Err(PlatformError::InvalidValue(
            "unknown experimental display identifier",
        )),
    }
}

fn validate_framebuffer(expected: (u32, u32)) -> Result<(u32, u32), PlatformError> {
    let file = OpenOptions::new().read(true).open("/dev/fb0")?;
    let mut information = FramebufferInfo { words: [0; 40] };
    // SAFETY: FBIOGET_VSCREENINFO copies the Linux fb_var_screeninfo structure
    // into an aligned 160-byte writable buffer. It does not retain the pointer.
    let result = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            0x4600,
            &mut information as *mut FramebufferInfo,
        )
    };
    if result == -1 {
        return Err(std::io::Error::last_os_error().into());
    }
    validate_framebuffer_info(&information, expected)
}

fn validate_framebuffer_info(
    information: &FramebufferInfo,
    expected: (u32, u32),
) -> Result<(u32, u32), PlatformError> {
    let actual = (information.words[0], information.words[1]);
    if actual != expected {
        return Err(PlatformError::GeometryMismatch { expected, actual });
    }
    let offset = (information.words[4], information.words[5]);
    if offset != (0, 0) {
        return Err(PlatformError::FramebufferOffset { actual: offset });
    }
    Ok(actual)
}

fn find_command(name: &str) -> Option<PathBuf> {
    ["/usr/bin", "/bin", "/usr/sbin", "/sbin"]
        .into_iter()
        .map(|directory| Path::new(directory).join(name))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framebuffer_info(size: (u32, u32), offset: (u32, u32)) -> FramebufferInfo {
        let mut information = FramebufferInfo { words: [0; 40] };
        information.words[0] = size.0;
        information.words[1] = size.1;
        information.words[2] = size.0;
        information.words[3] = size.1 * 2;
        information.words[4] = offset.0;
        information.words[5] = offset.1;
        information
    }

    #[test]
    fn nominal_display_keeps_the_design_geometry_for_every_profile() {
        for profile in [
            DeviceProfile::Paperwhite5,
            DeviceProfile::Paperwhite6,
            DeviceProfile::Scribe,
            DeviceProfile::Scribe2025,
        ] {
            assert_eq!(
                expected_framebuffer_size(profile, "5.20.1", None).unwrap(),
                profile.physical_size(),
            );
        }
    }

    #[test]
    fn the_emulator_display_requires_its_exact_identifier_profile_and_firmware() {
        assert_eq!(
            expected_framebuffer_size(
                DeviceProfile::Paperwhite6,
                "5.20.1",
                Some("pw6-5201-emulator"),
            )
            .unwrap(),
            (1272, 1696),
        );
        for (profile, firmware, identifier) in [
            (DeviceProfile::Paperwhite5, "5.20.1", "pw6-5201-emulator"),
            (DeviceProfile::Paperwhite6, "5.18.6", "pw6-5201-emulator"),
            (DeviceProfile::Paperwhite6, "5.20.1", "1272x1696"),
            (DeviceProfile::Paperwhite6, "5.20.1", ""),
        ] {
            assert!(matches!(
                expected_framebuffer_size(profile, firmware, Some(identifier)),
                Err(PlatformError::InvalidValue(_)),
            ));
        }
    }

    #[test]
    fn visible_geometry_is_checked_independently_of_virtual_framebuffer_height() {
        let information = framebuffer_info((1272, 1696), (0, 0));
        assert_eq!(
            validate_framebuffer_info(&information, (1272, 1696)).unwrap(),
            (1272, 1696),
        );
        let error = validate_framebuffer_info(&information, (1264, 1680)).unwrap_err();
        assert!(matches!(
            error,
            PlatformError::GeometryMismatch {
                expected: (1264, 1680),
                actual: (1272, 1696),
            },
        ));
    }

    #[test]
    fn nonzero_framebuffer_offsets_are_refused() {
        for offset in [(1, 0), (0, 1696)] {
            let information = framebuffer_info((1272, 1696), offset);
            assert!(matches!(
                validate_framebuffer_info(&information, (1272, 1696)),
                Err(PlatformError::FramebufferOffset { actual }) if actual == offset,
            ));
        }
    }
}
