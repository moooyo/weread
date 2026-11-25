use std::str::FromStr;

/// A design geometry, not a declaration of hardware compatibility.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DeviceProfile {
    Paperwhite5,
    #[default]
    Paperwhite6,
    Scribe,
    Scribe2025,
}

impl DeviceProfile {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "pw5" | "paperwhite5" => Some(Self::Paperwhite5),
            "pw6" | "paperwhite6" => Some(Self::Paperwhite6),
            "scribe" | "scribe2022" | "scribe2024" => Some(Self::Scribe),
            "scribe2025" => Some(Self::Scribe2025),
            _ => None,
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Paperwhite5 => "pw5",
            Self::Paperwhite6 => "pw6",
            Self::Scribe => "scribe",
            Self::Scribe2025 => "scribe2025",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Paperwhite5 => "Kindle Paperwhite 5",
            Self::Paperwhite6 => "Kindle Paperwhite 6",
            Self::Scribe => "Kindle Scribe (2022 / 2024)",
            Self::Scribe2025 => "Kindle Scribe (2025)",
        }
    }

    pub const fn logical_size(self) -> (u32, u32) {
        match self {
            Self::Paperwhite5 => (618, 824),
            Self::Paperwhite6 => (632, 840),
            Self::Scribe => (930, 1240),
            Self::Scribe2025 => (990, 1320),
        }
    }

    pub const fn physical_size(self) -> (u32, u32) {
        let (width, height) = self.logical_size();
        (width * 2, height * 2)
    }

    pub const fn is_scribe(self) -> bool {
        self.logical_size().0 >= 900
    }

    pub const fn page_margin(self) -> u32 {
        if self.is_scribe() { 32 } else { 20 }
    }

    pub const fn cover_columns(self) -> usize {
        if self.is_scribe() { 5 } else { 3 }
    }

    pub const fn cover_page_size(self) -> usize {
        if self.is_scribe() { 15 } else { 6 }
    }

    /// Upper bound for 72-pixel list rows; the Library fills the viewport up to it.
    pub const fn list_page_size(self) -> usize {
        if self.is_scribe() { 14 } else { 9 }
    }

    /// Fill the current logical detail viewport with bounded pages. Catalog
    /// rows are fixed; longer reviews and notes can scroll within a page.
    /// Temporary status and selection chrome do not change page boundaries.
    pub fn detail_page_size(self, viewport: (u32, u32), tab: &str) -> usize {
        if viewport.0 == 0 {
            return 1;
        }
        let (controls, row_height, maximum) = match tab {
            "reviews" => (if self.is_scribe() { 64 } else { 52 }, 88, 16),
            // The selected annotation reader admits at most sixteen records.
            "notes" => (0, 96, 16),
            _ => (48, 52, 64),
        };
        // Status bar, page header, tabs, tab controls, and the device summary inset.
        let chrome = 28
            + 52
            + 48
            + controls
            + if self.is_scribe() {
                24 // The two-column detail body's top inset.
            } else {
                218 // The stacked book summary and its actions.
            };
        (viewport.1.saturating_sub(chrome) / row_height).clamp(1, maximum) as usize
    }

    pub const fn reader_chapter_page_size(self) -> usize {
        if self.is_scribe() { 16 } else { 11 }
    }
}

impl FromStr for DeviceProfile {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value).ok_or("unknown device profile; use pw5, pw6, scribe, or scribe2025")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn design_units_have_the_same_physical_scale_on_every_profile() {
        for profile in [
            DeviceProfile::Paperwhite5,
            DeviceProfile::Paperwhite6,
            DeviceProfile::Scribe,
            DeviceProfile::Scribe2025,
        ] {
            let logical = profile.logical_size();
            assert_eq!(profile.physical_size(), (logical.0 * 2, logical.1 * 2));
            assert_eq!(profile.is_scribe(), logical.0 >= 900);
        }
    }

    #[test]
    fn unknown_profiles_are_not_silently_assumed_compatible() {
        assert!(DeviceProfile::parse("colorsoft").is_none());
        assert!(DeviceProfile::parse("unknown").is_none());
    }

    #[test]
    fn detail_pages_fill_the_short_row_space_on_every_profile() {
        for (profile, expected) in [
            (DeviceProfile::Paperwhite5, [8, 4, 4]),
            (DeviceProfile::Paperwhite6, [8, 5, 5]),
            (DeviceProfile::Scribe, [20, 11, 11]),
            (DeviceProfile::Scribe2025, [21, 12, 12]),
        ] {
            let actual = ["toc", "reviews", "notes"]
                .map(|tab| profile.detail_page_size(profile.logical_size(), tab));
            assert_eq!(actual, expected, "{}", profile.id());
        }
        assert_eq!(
            DeviceProfile::Scribe.detail_page_size((930, 1240), "toc"),
            20
        );
    }

    #[test]
    fn detail_capacity_uses_runtime_height_and_has_explicit_read_limits() {
        let profile = DeviceProfile::Paperwhite6;
        assert_eq!(profile.detail_page_size((636, 848), "toc"), 8);
        assert_eq!(profile.detail_page_size((636, 1100), "toc"), 13);
        for tab in ["toc", "reviews", "notes"] {
            assert_eq!(profile.detail_page_size((0, u32::MAX), tab), 1);
            assert_eq!(profile.detail_page_size((636, 0), tab), 1);
        }
        assert_eq!(profile.detail_page_size((636, u32::MAX), "toc"), 64);
        assert_eq!(profile.detail_page_size((636, u32::MAX), "reviews"), 16);
        assert_eq!(profile.detail_page_size((636, u32::MAX), "notes"), 16);
    }
}
