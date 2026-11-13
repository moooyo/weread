use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const DEFAULT_DOWNLOAD_CONCURRENCY: u32 = 8;
pub const MAX_DOWNLOAD_CONCURRENCY: u32 = 16;

/// A requested image policy, separate from a device's effective processing.
/// Off preserves the application's existing sixteen-level, non-Bayer default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageDithering {
    #[default]
    Off,
    Hardware,
    #[serde(other)]
    Software,
}

impl ImageDithering {
    pub fn parse(value: &str) -> Self {
        match value {
            "off" => Self::Off,
            "hardware" => Self::Hardware,
            _ => Self::Software,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Software => "software",
            Self::Hardware => "hardware",
        }
    }
}

/// Read a best-effort startup appearance hint before mutable application state
/// is opened. The existing database is opened read-only without migration or
/// creation. SQLite may maintain its required WAL shared-memory bookkeeping.
/// The normal Store remains authoritative and loads preferences after startup.
pub fn startup_dark_mode(data_dir: &Path) -> bool {
    startup_dark_mode_hint(data_dir).unwrap_or(false)
}

/// Preserve the distinction between an explicit light preference and first use.
pub fn startup_dark_mode_hint(data_dir: &Path) -> Option<bool> {
    read_startup_dark_mode(data_dir)
}

fn read_startup_dark_mode(data_dir: &Path) -> Option<bool> {
    let connection = open_startup_preferences(data_dir)?;
    let stored: String = connection
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            ["preferences"],
            |row| row.get(0),
        )
        .ok()?;
    Some(
        serde_json::from_str::<AppPreferences>(&stored)
            .ok()?
            .dark_mode,
    )
}

fn open_startup_preferences(data_dir: &Path) -> Option<rusqlite::Connection> {
    let connection = rusqlite::Connection::open_with_flags(
        data_dir.join("library.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    connection.busy_timeout(Duration::ZERO).ok()?;
    Some(connection)
}

/// Device-independent preferences. Display values are rendered by the locale layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppPreferences {
    pub view_mode: String,
    pub dark_mode: bool,
    pub reader: ReaderPreferences,
    pub article: ArticlePreferences,
    pub pull_on_open: bool,
    pub push_on_close: bool,
    pub ask_on_conflict: bool,
    pub report_time: bool,
    pub report_reader_only: bool,
    pub popular: bool,
    pub mine: bool,
    pub edge_guard: bool,
    pub edge_region: u32,
    pub popup_height: u32,
    pub popup_position: String,
    pub popup_turn: bool,
    pub download_images: bool,
    pub download_concurrency: u32,
    pub download_article_images: bool,
    pub prefetch: bool,
    pub prefetch_notes: bool,
    pub flash_on_navigation: bool,
    pub keep_awake: bool,
    pub wifi_idle: bool,
    pub sleep_cover: bool,
    pub image_dithering: ImageDithering,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            view_mode: "cover".into(),
            dark_mode: false,
            reader: ReaderPreferences::default(),
            article: ArticlePreferences::default(),
            pull_on_open: true,
            push_on_close: true,
            ask_on_conflict: true,
            report_time: true,
            report_reader_only: true,
            popular: true,
            mine: true,
            edge_guard: true,
            edge_region: 20,
            popup_height: 62,
            popup_position: "center".into(),
            popup_turn: false,
            download_images: true,
            download_concurrency: DEFAULT_DOWNLOAD_CONCURRENCY,
            download_article_images: false,
            prefetch: false,
            prefetch_notes: false,
            flash_on_navigation: true,
            keep_awake: true,
            wifi_idle: true,
            sleep_cover: true,
            image_dithering: ImageDithering::Off,
        }
    }
}

/// Reading typography is separate from article typography and uses logical dp.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReaderPreferences {
    pub font: String,
    pub font_size: u32,
    pub line_spacing: u32,
    pub margin: u32,
    pub weight: u32,
    pub indent: u32,
    pub gap: u32,
    pub justify: bool,
    pub tap_mode: String,
    pub refresh_interval: u32,
    pub book_only: bool,
}

impl Default for ReaderPreferences {
    fn default() -> Self {
        Self {
            font: "song".into(),
            font_size: 22,
            line_spacing: 1,
            margin: 1,
            weight: 400,
            indent: 2,
            gap: 0,
            justify: true,
            tap_mode: "default".into(),
            refresh_interval: 6,
            book_only: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArticlePreferences {
    pub font: String,
    pub font_size: u32,
    pub weight: u32,
    pub gap: u32,
}

impl Default for ArticlePreferences {
    fn default() -> Self {
        Self {
            font: "hei".into(),
            font_size: 19,
            weight: 400,
            gap: 1,
        }
    }
}

impl AppPreferences {
    pub fn for_device(scribe: bool) -> Self {
        let mut preferences = Self::default();
        if scribe {
            preferences.reader.font_size = 24;
            preferences.article.font_size = 22;
            preferences.popup_position = "side".into();
        }
        preferences
    }

    /// Return false for unknown keys or controls disabled by a parent preference.
    pub fn apply_toggle(&mut self, key: &str) -> bool {
        let key = canonical_key(key);
        let value = match key.as_str() {
            "night_mode" | "dark_mode" => &mut self.dark_mode,
            "pull_on_open" => &mut self.pull_on_open,
            "push_on_close" => &mut self.push_on_close,
            "ask_on_conflict" => &mut self.ask_on_conflict,
            "report_time" => &mut self.report_time,
            "report_reader_only" if self.report_time => &mut self.report_reader_only,
            "popular" => &mut self.popular,
            "mine" => &mut self.mine,
            "edge_guard" => &mut self.edge_guard,
            "popup_turn" => &mut self.popup_turn,
            "download_images" => &mut self.download_images,
            "download_article_images" => &mut self.download_article_images,
            "prefetch" => &mut self.prefetch,
            "prefetch_notes" if self.prefetch => &mut self.prefetch_notes,
            "flash_on_navigation" => &mut self.flash_on_navigation,
            "keep_awake" => &mut self.keep_awake,
            "wifi_idle" => &mut self.wifi_idle,
            "sleep_cover" => &mut self.sleep_cover,
            "reader_justify" | "justify" => &mut self.reader.justify,
            "reader_book_only" | "book_only" => &mut self.reader.book_only,
            _ => return false,
        };
        *value = !*value;
        true
    }

    /// Adjust a numeric control by a number of steps, not by a raw dp amount.
    pub fn adjust(&mut self, key: &str, delta: i32) -> bool {
        let key = canonical_key(key);
        match key.as_str() {
            "reader_font_size" | "font_size" => {
                self.reader.font_size = stepped(self.reader.font_size, delta, 2, 16, 38);
            }
            "article_font_size" => {
                self.article.font_size = step_options(
                    self.article.font_size,
                    delta,
                    &[15, 16, 17, 18, 19, 20, 22, 24, 26, 28, 30, 32],
                );
            }
            "edge_region" => self.edge_region = stepped(self.edge_region, delta, 5, 10, 40),
            "popup_height" => self.popup_height = stepped(self.popup_height, delta, 4, 20, 90),
            "download_concurrency" => {
                self.download_concurrency = stepped(
                    self.download_concurrency,
                    delta,
                    1,
                    1,
                    i64::from(MAX_DOWNLOAD_CONCURRENCY),
                );
            }
            "reader_line_spacing" | "line_spacing" => {
                self.reader.line_spacing = stepped(self.reader.line_spacing, delta, 1, 0, 2);
            }
            "reader_margin" | "margin" => {
                self.reader.margin = stepped(self.reader.margin, delta, 1, 0, 2);
            }
            "reader_indent" | "indent" => {
                self.reader.indent = stepped(self.reader.indent, delta, 1, 0, 2);
            }
            "reader_gap" | "gap" => {
                self.reader.gap = stepped(self.reader.gap, delta, 1, 0, 2);
            }
            "article_gap" => self.article.gap = stepped(self.article.gap, delta, 1, 0, 2),
            "reader_weight" | "weight" => {
                self.reader.weight = step_options(self.reader.weight, delta, &[300, 400, 500, 700]);
            }
            "article_weight" => {
                self.article.weight = step_options(self.article.weight, delta, &[300, 400, 700]);
            }
            _ => return false,
        }
        true
    }

    /// Use the exact size ladder for the active device when editing font sizes.
    pub fn adjust_for_device(&mut self, key: &str, delta: i32, scribe: bool) -> bool {
        match canonical_key(key).as_str() {
            "reader_font_size" | "font_size" => {
                let sizes: &[u32] = if scribe {
                    &[18, 20, 22, 24, 26, 28, 30, 32, 34, 36, 38]
                } else {
                    &[16, 18, 20, 22, 24, 26, 28, 30, 32]
                };
                self.reader.font_size = step_options(self.reader.font_size, delta, sizes);
            }
            "article_font_size" => {
                let sizes: &[u32] = if scribe {
                    &[16, 18, 20, 22, 24, 26, 28, 30, 32]
                } else {
                    &[15, 16, 17, 18, 19, 20, 22, 24, 26]
                };
                self.article.font_size = step_options(self.article.font_size, delta, sizes);
            }
            _ => return self.adjust(key, delta),
        }
        true
    }

    /// Assign a segmented or named option. Unknown values never enter persisted state.
    pub fn set_choice(&mut self, key: &str, value: &str) -> bool {
        let key = canonical_key(key);
        match key.as_str() {
            "view_mode" if ["cover", "list"].contains(&value) => self.view_mode = value.into(),
            "image_dithering" if ["off", "software", "hardware"].contains(&value) => {
                self.image_dithering = ImageDithering::parse(value);
            }
            "popup_position" if ["side", "center", "bottom"].contains(&value) => {
                self.popup_position = value.into();
            }
            "reader_font" | "font" if !value.trim().is_empty() && value.len() <= 256 => {
                self.reader.font = value.into();
            }
            "article_font" if !value.trim().is_empty() && value.len() <= 256 => {
                self.article.font = value.into();
            }
            "reader_tap_mode" | "tap_mode" if ["default", "mirrored", "swipe"].contains(&value) => {
                self.reader.tap_mode = value.into();
            }
            "reader_refresh_interval" | "refresh_interval" => {
                let Ok(interval) = value.parse::<u32>() else {
                    return false;
                };
                if ![0, 1, 6, 12].contains(&interval) {
                    return false;
                }
                self.reader.refresh_interval = interval;
            }
            "reader_line_spacing" | "line_spacing" => {
                let Some(number) = parsed_option(value, &[0, 1, 2]) else {
                    return false;
                };
                self.reader.line_spacing = number;
            }
            "reader_margin" | "margin" => {
                let Some(number) = parsed_option(value, &[0, 1, 2]) else {
                    return false;
                };
                self.reader.margin = number;
            }
            "reader_indent" | "indent" => {
                let Some(number) = parsed_option(value, &[0, 1, 2]) else {
                    return false;
                };
                self.reader.indent = number;
            }
            "reader_gap" | "gap" => {
                let Some(number) = parsed_option(value, &[0, 1, 2]) else {
                    return false;
                };
                self.reader.gap = number;
            }
            "article_gap" => {
                let Some(number) = parsed_option(value, &[0, 1, 2]) else {
                    return false;
                };
                self.article.gap = number;
            }
            "reader_weight" | "weight" => {
                let Some(number) = parsed_option(value, &[300, 400, 500, 700]) else {
                    return false;
                };
                self.reader.weight = number;
            }
            "article_weight" => {
                let Some(number) = parsed_option(value, &[300, 400, 700]) else {
                    return false;
                };
                self.article.weight = number;
            }
            _ => return false,
        }
        true
    }

    /// Raw values are localization-neutral; the UI maps option IDs to readable labels.
    pub fn value(&self, key: &str) -> String {
        let key = canonical_key(key);
        match key.as_str() {
            "view_mode" => self.view_mode.clone(),
            "image_dithering" => self.image_dithering.as_str().into(),
            "night_mode" | "dark_mode" => self.dark_mode.to_string(),
            "reader_font" | "font" => self.reader.font.clone(),
            "reader_font_size" | "font_size" => self.reader.font_size.to_string(),
            "reader_line_spacing" | "line_spacing" => self.reader.line_spacing.to_string(),
            "reader_margin" | "margin" => self.reader.margin.to_string(),
            "reader_weight" | "weight" => self.reader.weight.to_string(),
            "reader_indent" | "indent" => self.reader.indent.to_string(),
            "reader_gap" | "gap" => self.reader.gap.to_string(),
            "reader_justify" | "justify" => self.reader.justify.to_string(),
            "reader_tap_mode" | "tap_mode" => self.reader.tap_mode.clone(),
            "reader_refresh_interval" | "refresh_interval" => {
                self.reader.refresh_interval.to_string()
            }
            "reader_book_only" | "book_only" => self.reader.book_only.to_string(),
            "article_font" => self.article.font.clone(),
            "article_font_size" => self.article.font_size.to_string(),
            "article_weight" => self.article.weight.to_string(),
            "article_gap" => self.article.gap.to_string(),
            "pull_on_open" => self.pull_on_open.to_string(),
            "push_on_close" => self.push_on_close.to_string(),
            "ask_on_conflict" => self.ask_on_conflict.to_string(),
            "report_time" => self.report_time.to_string(),
            "report_reader_only" => self.report_reader_only.to_string(),
            "popular" => self.popular.to_string(),
            "mine" => self.mine.to_string(),
            "edge_guard" => self.edge_guard.to_string(),
            "edge_region" => self.edge_region.to_string(),
            "popup_height" => self.popup_height.to_string(),
            "popup_position" => self.popup_position.clone(),
            "popup_turn" => self.popup_turn.to_string(),
            "download_images" => self.download_images.to_string(),
            "download_concurrency" => self.download_concurrency.to_string(),
            "download_article_images" => self.download_article_images.to_string(),
            "prefetch" => self.prefetch.to_string(),
            "prefetch_notes" => self.prefetch_notes.to_string(),
            "flash_on_navigation" => self.flash_on_navigation.to_string(),
            "keep_awake" => self.keep_awake.to_string(),
            "wifi_idle" => self.wifi_idle.to_string(),
            "sleep_cover" => self.sleep_cover.to_string(),
            _ => String::new(),
        }
    }

    /// Repair invalid stored options without treating configuration as trusted code.
    pub fn normalize(&mut self) {
        if !["cover", "list"].contains(&self.view_mode.as_str()) {
            self.view_mode = "cover".into();
        }
        if !["side", "center", "bottom"].contains(&self.popup_position.as_str()) {
            self.popup_position = "center".into();
        }
        self.edge_region = self.edge_region.clamp(10, 40);
        self.popup_height = self.popup_height.clamp(20, 90);
        self.download_concurrency = self.download_concurrency.clamp(1, MAX_DOWNLOAD_CONCURRENCY);
        self.reader.font_size = nearest(
            self.reader.font_size,
            &[16, 18, 20, 22, 24, 26, 28, 30, 32, 34, 36, 38],
        );
        self.article.font_size = nearest(
            self.article.font_size,
            &[15, 16, 17, 18, 19, 20, 22, 24, 26, 28, 30, 32],
        );
        self.reader.line_spacing = self.reader.line_spacing.min(2);
        self.reader.margin = self.reader.margin.min(2);
        self.reader.weight = nearest(self.reader.weight, &[300, 400, 500, 700]);
        self.reader.indent = self.reader.indent.min(2);
        self.reader.gap = self.reader.gap.min(2);
        self.article.weight = nearest(self.article.weight, &[300, 400, 700]);
        self.article.gap = self.article.gap.min(2);
        if !["default", "mirrored", "swipe"].contains(&self.reader.tap_mode.as_str()) {
            self.reader.tap_mode = "default".into();
        }
        if ![0, 1, 6, 12].contains(&self.reader.refresh_interval) {
            self.reader.refresh_interval = 6;
        }
        if self.reader.font.trim().is_empty() || self.reader.font.len() > 256 {
            self.reader.font = "song".into();
        }
        if self.article.font.trim().is_empty() || self.article.font.len() > 256 {
            self.article.font = "hei".into();
        }
    }

    /// Apply the handoff's size steps after loading or editing device preferences.
    pub fn normalize_for_device(&mut self, scribe: bool) {
        self.normalize();
        let reader_sizes: &[u32] = if scribe {
            &[18, 20, 22, 24, 26, 28, 30, 32, 34, 36, 38]
        } else {
            &[16, 18, 20, 22, 24, 26, 28, 30, 32]
        };
        let article_sizes: &[u32] = if scribe {
            &[16, 18, 20, 22, 24, 26, 28, 30, 32]
        } else {
            &[15, 16, 17, 18, 19, 20, 22, 24, 26]
        };
        self.reader.font_size = nearest(self.reader.font_size, reader_sizes);
        self.article.font_size = nearest(self.article.font_size, article_sizes);
        if !scribe && self.popup_position == "side" {
            self.popup_position = "center".into();
        }
    }
}

fn canonical_key(key: &str) -> String {
    key.replace(['.', '-'], "_")
}

fn parsed_option(value: &str, options: &[u32]) -> Option<u32> {
    value.parse().ok().filter(|number| options.contains(number))
}

fn stepped(value: u32, delta: i32, step: i64, min: i64, max: i64) -> u32 {
    (i64::from(value) + i64::from(delta) * step).clamp(min, max) as u32
}

fn nearest(value: u32, options: &[u32]) -> u32 {
    *options
        .iter()
        .min_by_key(|candidate| candidate.abs_diff(value))
        .expect("nonempty options")
}

fn step_options(value: u32, delta: i32, options: &[u32]) -> u32 {
    let value = nearest(value, options);
    let index = options
        .iter()
        .position(|option| *option == value)
        .expect("normalized option");
    let next = (index as i64 + i64::from(delta)).clamp(0, options.len() as i64 - 1);
    options[next as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_record_snapshot(connection: &rusqlite::Connection) -> (Vec<(String, String)>, i64) {
        let mut statement = connection
            .prepare("SELECT key, value FROM settings ORDER BY key")
            .unwrap();
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let version = connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap();
        (rows, version)
    }

    #[test]
    fn startup_appearance_reads_clean_and_committed_wal_without_changing_user_records() {
        for dark_mode in [false, true] {
            for keep_wal_open in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let connection =
                    rusqlite::Connection::open(directory.path().join("library.sqlite")).unwrap();
                connection
                    .pragma_update(None, "journal_mode", "WAL")
                    .unwrap();
                connection.execute_batch(
                    "CREATE TABLE settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                     INSERT INTO settings VALUES ('unrelated.preference', 'preserved value');
                     PRAGMA user_version=9;",
                ).unwrap();
                let mut preferences = AppPreferences::default();
                preferences.dark_mode = dark_mode;
                connection
                    .execute(
                        "INSERT INTO settings (key, value) VALUES ('preferences', ?1)",
                        [serde_json::to_string(&preferences).unwrap()],
                    )
                    .unwrap();
                let before = user_record_snapshot(&connection);
                let writer = if keep_wal_open {
                    Some(connection)
                } else {
                    drop(connection);
                    None
                };
                assert_eq!(startup_dark_mode(directory.path()), dark_mode);
                let reader = open_startup_preferences(directory.path()).unwrap();
                assert_eq!(user_record_snapshot(&reader), before);
                drop(reader);
                drop(writer);
            }
        }
    }

    #[test]
    fn startup_appearance_falls_back_to_light_without_creating_or_migrating_state() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing");
        assert!(!startup_dark_mode(&missing));
        assert!(!missing.exists());
        assert!(!startup_dark_mode(directory.path()));
        assert!(!directory.path().join("library.sqlite").exists());
        assert!(
            std::fs::read_dir(directory.path())
                .unwrap()
                .next()
                .is_none()
        );

        let database = directory.path().join("library.sqlite");
        std::fs::write(&database, b"invalid SQLite database").unwrap();
        let before = std::fs::read(&database).unwrap();
        assert!(!startup_dark_mode(directory.path()));
        assert_eq!(std::fs::read(&database).unwrap(), before);
    }

    #[test]
    fn startup_appearance_requires_a_valid_boolean_in_the_existing_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let connection =
            rusqlite::Connection::open(directory.path().join("library.sqlite")).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);",
            )
            .unwrap();
        assert!(!startup_dark_mode(directory.path()));
        for stored in [
            "{}",
            r#"{"dark_mode":false}"#,
            r#"{"dark_mode":"true"}"#,
            "invalid JSON",
        ] {
            connection
                .execute(
                    "INSERT INTO settings (key, value) VALUES ('preferences', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    [stored],
                )
                .unwrap();
            let before = user_record_snapshot(&connection);
            assert!(!startup_dark_mode(directory.path()));
            assert_eq!(user_record_snapshot(&connection), before);
        }
    }

    #[test]
    fn startup_preferences_connection_rejects_writes_and_leaves_the_schema_version_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let before =
            {
                let connection =
                    rusqlite::Connection::open(directory.path().join("library.sqlite")).unwrap();
                connection.execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                 PRAGMA user_version=9;",
            ).unwrap();
                let preferences = AppPreferences {
                    dark_mode: true,
                    ..AppPreferences::default()
                };
                connection
                    .execute(
                        "INSERT INTO settings (key,value) VALUES ('preferences',?1)",
                        [serde_json::to_string(&preferences).unwrap()],
                    )
                    .unwrap();
                user_record_snapshot(&connection)
            };
        let connection = open_startup_preferences(directory.path()).unwrap();
        let error = connection
            .execute(
                "UPDATE settings SET value = '{\"dark_mode\":false}' WHERE key = 'preferences'",
                [],
            )
            .unwrap_err();
        assert!(matches!(error, rusqlite::Error::SqliteFailure(code, _)
            if code.code == rusqlite::ErrorCode::ReadOnly));
        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 9);
        let stored: String = connection
            .query_row(
                "SELECT value FROM settings WHERE key = 'preferences'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            serde_json::from_str::<AppPreferences>(&stored)
                .unwrap()
                .dark_mode
        );
        assert!(startup_dark_mode(directory.path()));
        assert_eq!(user_record_snapshot(&connection), before);
    }

    #[test]
    fn application_dark_mode_defaults_to_light_and_survives_preference_round_trips() {
        let mut preferences = AppPreferences::default();
        assert!(!preferences.dark_mode);
        assert!(preferences.apply_toggle("night-mode"));
        assert_eq!(preferences.value("night_mode"), "true");
        let stored = serde_json::to_string(&preferences).unwrap();
        let mut restored: AppPreferences = serde_json::from_str(&stored).unwrap();
        assert!(restored.dark_mode);
        assert!(restored.apply_toggle("dark_mode"));
        let stored = serde_json::to_string(&restored).unwrap();
        let restored: AppPreferences = serde_json::from_str(&stored).unwrap();
        assert!(!restored.dark_mode);
        assert_eq!(restored.value("night_mode"), "false");
    }

    #[test]
    fn download_concurrency_defaults_and_survives_storage() {
        let mut preferences = AppPreferences::default();
        assert_eq!(
            preferences.download_concurrency,
            DEFAULT_DOWNLOAD_CONCURRENCY
        );
        assert!(preferences.adjust("download_concurrency", 1));
        let restored: AppPreferences =
            serde_json::from_str(&serde_json::to_string(&preferences).unwrap()).unwrap();
        assert_eq!(restored.download_concurrency, 9);
        assert_eq!(AppPreferences::for_device(true).download_concurrency, 8);
        assert_eq!(AppPreferences::for_device(false).download_concurrency, 8);
    }

    #[test]
    fn incomplete_preference_snapshots_are_rejected() {
        assert!(serde_json::from_str::<AppPreferences>("{}").is_err());
        let mut snapshot = serde_json::to_value(AppPreferences::default()).unwrap();
        snapshot
            .as_object_mut()
            .unwrap()
            .remove("download_concurrency");
        assert!(serde_json::from_value::<AppPreferences>(snapshot).is_err());
        let mut snapshot = serde_json::to_value(AppPreferences::default()).unwrap();
        snapshot.as_object_mut().unwrap().remove("image_dithering");
        assert!(serde_json::from_value::<AppPreferences>(snapshot).is_err());
    }

    #[test]
    fn image_dithering_is_explicit_and_preserves_the_existing_off_default() {
        let mut preferences = AppPreferences::default();
        assert_eq!(preferences.image_dithering, ImageDithering::Off);
        for value in ["software", "hardware", "off"] {
            assert!(preferences.set_choice("image_dithering", value));
            let restored: AppPreferences =
                serde_json::from_str(&serde_json::to_string(&preferences).unwrap()).unwrap();
            assert_eq!(restored.value("image_dithering"), value);
        }
        assert!(!preferences.set_choice("image_dithering", "unsupported"));
        assert_eq!(preferences.image_dithering, ImageDithering::Off);
        assert_eq!(
            serde_json::from_str::<ImageDithering>("\"future\"").unwrap(),
            ImageDithering::Software
        );
    }

    #[test]
    fn download_concurrency_rejects_unbounded_steps_and_repairs_stored_limits() {
        let mut preferences = AppPreferences::default();
        assert!(preferences.adjust("download_concurrency", i32::MAX));
        assert_eq!(preferences.download_concurrency, MAX_DOWNLOAD_CONCURRENCY);
        assert!(preferences.adjust("download_concurrency", i32::MIN));
        assert_eq!(preferences.download_concurrency, 1);
        for (stored, expected) in [(0, 1), (u32::MAX, MAX_DOWNLOAD_CONCURRENCY)] {
            preferences.download_concurrency = stored;
            preferences.normalize_for_device(true);
            assert_eq!(preferences.download_concurrency, expected);
        }
    }

    #[test]
    fn design_defaults_keep_article_preferences_independent() {
        let mut preferences = AppPreferences::default();
        preferences.reader.font_size = 28;
        assert_eq!(preferences.reader.font_size, 28);
        assert_eq!(preferences.reader.indent, 2);
        assert_eq!(preferences.article.font_size, 19);
        assert!(preferences.adjust("reader.font_size", 1));
        assert_eq!(preferences.reader.font_size, 30);
        assert_eq!(preferences.article.font_size, 19);
        assert_eq!(AppPreferences::for_device(true).reader.font_size, 24);
        assert_eq!(AppPreferences::for_device(true).popup_position, "side");
        preferences.article.font_size = 16;
        preferences.adjust_for_device("article.font_size", 1, true);
        assert_eq!(preferences.article.font_size, 18);
    }

    #[test]
    fn dependent_controls_cannot_be_changed_while_disabled() {
        let mut preferences = AppPreferences::default();
        assert!(!preferences.apply_toggle("prefetch_notes"));
        assert!(!preferences.prefetch_notes);
        assert!(preferences.apply_toggle("prefetch"));
        assert!(preferences.apply_toggle("prefetch_notes"));
        assert!(preferences.prefetch_notes);
        preferences.apply_toggle("report_time");
        assert!(!preferences.apply_toggle("report_reader_only"));
        assert!(preferences.report_reader_only);
    }

    #[test]
    fn options_and_arithmetic_are_bounded_and_invalid_storage_is_repaired() {
        let mut preferences = AppPreferences::default();
        assert!(!preferences.set_choice("refresh_interval", "99"));
        assert!(!preferences.set_choice("view_mode", "invalid"));
        assert!(preferences.adjust("edge_region", i32::MAX));
        assert_eq!(preferences.edge_region, 40);
        preferences.adjust("popup_height", i32::MIN);
        assert_eq!(preferences.popup_height, 20);
        preferences.reader.font_size = u32::MAX;
        preferences.reader.refresh_interval = 8;
        preferences.reader.tap_mode = "invalid".into();
        preferences.normalize();
        assert_eq!(preferences.reader.font_size, 38);
        assert_eq!(preferences.reader.refresh_interval, 6);
        assert_eq!(preferences.reader.tap_mode, "default");
        preferences.normalize_for_device(false);
        assert_eq!(preferences.reader.font_size, 32);
    }
}
