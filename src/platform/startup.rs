//! Early native startup feedback without Slint or runtime font initialization.

use super::{Platform, PlatformError};

#[derive(Clone, Copy)]
pub enum StartupStage {
    Fonts,
    Display,
    Library,
}

/// Desktop launch does not need the native panel's pre-event-loop display owner.
pub struct StartupScreen {
    #[cfg(all(feature = "kindle", target_os = "linux"))]
    display: Option<slint_backend_kindle::StartupSplash>,
}

impl StartupScreen {
    pub fn show(platform: &Platform, dark_mode: bool) -> Result<Self, PlatformError> {
        #[cfg(all(feature = "kindle", target_os = "linux"))]
        {
            use slint_backend_kindle::{StartupImage, StartupImages, StartupSplash};

            let display = if platform.is_kindle {
                let viewport = platform.native_viewport_size().ok_or_else(|| {
                    PlatformError::Operation("The native display geometry was not approved".into())
                })?;
                // Regenerate these tiny assets with assets/startup/prepare.py when
                // the startup translations change. Runtime fonts remain separate.
                let images = StartupImages {
                    title: StartupImage::from_encoded(include_bytes!(
                        "../../assets/startup/app-name.gray"
                    ))?,
                    starting: StartupImage::from_encoded(include_bytes!(
                        "../../assets/startup/starting.gray"
                    ))?,
                    stages: [
                        StartupImage::from_encoded(include_bytes!(
                            "../../assets/startup/fonts.gray"
                        ))?,
                        StartupImage::from_encoded(include_bytes!(
                            "../../assets/startup/display.gray"
                        ))?,
                        StartupImage::from_encoded(include_bytes!(
                            "../../assets/startup/library.gray"
                        ))?,
                    ],
                };
                let software_inverse = startup_output_inverted(platform, dark_mode);
                Some(StartupSplash::show(images, viewport, software_inverse)?)
            } else {
                None
            };
            Ok(Self { display })
        }
        #[cfg(not(all(feature = "kindle", target_os = "linux")))]
        {
            let _ = (platform, dark_mode);
            Ok(Self {})
        }
    }

    pub fn set_stage(&self, stage: StartupStage) {
        #[cfg(all(feature = "kindle", target_os = "linux"))]
        if let Some(display) = self.display.as_ref() {
            display.set_stage(match stage {
                StartupStage::Fonts => 0,
                StartupStage::Display => 1,
                StartupStage::Library => 2,
            });
        }
        #[cfg(not(all(feature = "kindle", target_os = "linux")))]
        let _ = stage;
    }

    pub fn pause(&mut self) -> Result<(), PlatformError> {
        #[cfg(all(feature = "kindle", target_os = "linux"))]
        if let Some(display) = self.display.as_mut() {
            display.pause()?;
        }
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), PlatformError> {
        #[cfg(all(feature = "kindle", target_os = "linux"))]
        if let Some(display) = self.display.as_mut() {
            display.resume()?;
        }
        Ok(())
    }

    pub fn finish(self) -> Result<(), PlatformError> {
        #[cfg(all(feature = "kindle", target_os = "linux"))]
        if let Some(display) = self.display {
            display.finish()?;
        }
        Ok(())
    }
}

#[cfg(any(all(feature = "kindle", target_os = "linux"), test))]
pub(super) fn startup_output_inverted(platform: &Platform, desired_dark: bool) -> bool {
    super::software_output_inverted(desired_dark, platform.native_output_inverted())
}
