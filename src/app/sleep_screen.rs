//! Prepare one bounded sleep cover before the power-event critical path.

use super::*;
use anyhow::ensure;
use sha2::{Digest, Sha256};
use std::path::Path;

const MAX_SLEEP_COVER_WIDTH: u32 = 1536;
const MAX_SLEEP_COVER_HEIGHT: u32 = 2048;
const MAX_SLEEP_COVER_URL_BYTES: usize = 8192;
const MAX_SLEEP_TITLE_SCALARS: usize = 256;
const MAX_SLEEP_AUTHOR_SCALARS: usize = 128;

#[derive(Default)]
pub(super) struct SleepScreenState {
    book_id: Option<String>,
    source_key: Option<String>,
    thumbnail_key: Option<String>,
    title: slint::SharedString,
    author: slint::SharedString,
    cover: Option<slint::Image>,
    full_resolution: bool,
}

pub(super) fn cover_key(book_id: &str, url: &str) -> Option<String> {
    if url.is_empty() {
        return None;
    }
    source_key("sleep-cover", book_id, url)
}

pub(super) fn thumbnail_key(book_id: &str, url: &str) -> Option<String> {
    source_key("cover", book_id, url)
}

fn source_key(prefix: &str, book_id: &str, url: &str) -> Option<String> {
    if url.len() > MAX_SLEEP_COVER_URL_BYTES || book_id.len() > 4096 {
        return None;
    }
    let mut digest = Sha256::new();
    digest.update((book_id.len() as u64).to_be_bytes());
    digest.update(book_id.as_bytes());
    digest.update(url.as_bytes());
    Some(format!("{prefix}:{}", hex::encode(digest.finalize())))
}

fn bounded_text(text: &str, maximum: usize) -> slint::SharedString {
    text.chars().take(maximum).collect::<String>().into()
}

fn decode_cover(path: &Path, geometry: (u32, u32)) -> Result<slint::Image> {
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SLEEP_COVER_WIDTH.max(400));
    limits.max_image_height = Some(MAX_SLEEP_COVER_HEIGHT);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let grayscale = reader
        .decode()?
        .resize(
            geometry.0,
            geometry.1,
            image::imageops::FilterType::Triangle,
        )
        .into_luma8();
    let (width, height) = grayscale.dimensions();
    ensure!(
        width <= MAX_SLEEP_COVER_WIDTH && height <= MAX_SLEEP_COVER_HEIGHT,
        "The prepared sleep cover exceeds its pixel budget"
    );
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
    for (destination, source) in pixels.make_mut_slice().iter_mut().zip(grayscale.pixels()) {
        let value = (((u16::from(source[0]) + 8) / 17).min(15) * 17) as u8;
        *destination = slint::Rgb8Pixel {
            r: value,
            g: value,
            b: value,
        };
    }
    Ok(slint::Image::from_rgb8(pixels))
}

impl App {
    /// Residual Reader state never identifies a book outside a live Reader route.
    pub(super) fn current_sleep_book_id(&self) -> Option<&str> {
        let Route::Reader(book_id, chapter) = &self.route else {
            return None;
        };
        (book_id == &self.reader.book_id
            && *chapter == self.reader.chapter_index
            && self.reader.paginator.is_some()
            && !self.reader.body.is_empty())
        .then_some(book_id.as_str())
    }

    pub(super) fn sleep_cover_enabled(&self) -> bool {
        self.prefs.sleep_cover
    }

    pub(super) fn sleep_cover_geometry(&self) -> (u32, u32) {
        let (width, height) = self
            .platform
            .native_viewport_size()
            .unwrap_or_else(|| self.profile.physical_size());
        (
            width.clamp(1, MAX_SLEEP_COVER_WIDTH),
            height.clamp(1, MAX_SLEEP_COVER_HEIGHT),
        )
    }

    /// Normal Reader rendering already owns this admitted metadata projection.
    /// Power-event handling must never call this preparation or read the cache.
    pub(super) fn prepare_sleep_screen_for_book(&mut self, book: &BookMetadata) {
        if self.managed_sleep_blocked {
            return;
        }
        if !self.prefs.sleep_cover || self.current_sleep_book_id() != Some(book.id.as_str()) {
            self.clear_prepared_sleep_screen();
            return;
        }
        let source_key = cover_key(&book.id, &book.cover_url);
        let thumbnail_key = thumbnail_key(&book.id, &book.cover_url);
        let changed = self.sleep_screen.book_id.as_deref() != Some(book.id.as_str())
            || self.sleep_screen.source_key != source_key
            || self.sleep_screen.thumbnail_key != thumbnail_key;
        self.sleep_screen.title = bounded_text(&book.title, MAX_SLEEP_TITLE_SCALARS);
        self.sleep_screen.author = bounded_text(&book.author, MAX_SLEEP_AUTHOR_SCALARS);
        if changed {
            self.sleep_screen.book_id = Some(book.id.clone());
            self.sleep_screen.source_key = source_key;
            self.sleep_screen.thumbnail_key = thumbnail_key;
            self.sleep_screen.cover = None;
            self.sleep_screen.full_resolution = false;
            self.reload_prepared_sleep_cover();
        }
        if !self.sleep_screen.full_resolution && self.sleep_screen.source_key.is_some() {
            self.schedule_visible_covers();
        }
    }

    /// Download completions are account-fenced before reaching this method.
    /// Keep a sharper derivative only for the currently open book.
    pub(super) fn prepare_downloaded_sleep_cover(
        &mut self,
        book_id: &str,
        url: &str,
        bytes: &[u8],
    ) -> bool {
        if !self.prefs.sleep_cover || self.current_sleep_book_id() != Some(book_id) {
            return true;
        }
        if self.sleep_screen.book_id.as_deref() != Some(book_id)
            || self.sleep_screen.thumbnail_key != thumbnail_key(book_id, url)
        {
            return true;
        }
        let stored = if let Some(key) = self.sleep_screen.source_key.as_deref() {
            let geometry = self.sleep_cover_geometry();
            self.cache
                .put_cover(key, bytes, geometry.0, geometry.1)
                .is_ok()
        } else {
            true
        };
        self.reload_prepared_sleep_cover();
        stored
    }

    fn reload_prepared_sleep_cover(&mut self) {
        if self.sleep_screen.book_id.is_none() {
            return;
        }
        let geometry = self.sleep_cover_geometry();
        let full = self
            .sleep_screen
            .source_key
            .as_deref()
            .and_then(|key| self.cache.get(key).ok().flatten())
            .and_then(|path| decode_cover(&path, geometry).ok());
        let full_resolution = full.is_some();
        let cover = full.or_else(|| {
            self.sleep_screen
                .thumbnail_key
                .as_deref()
                .and_then(|key| self.cache.get(key).ok().flatten())
                .and_then(|path| decode_cover(&path, geometry).ok())
        });
        if cover.is_some() {
            self.sleep_screen.cover = cover;
            self.sleep_screen.full_resolution = full_resolution;
        }
    }

    /// Account invalidation and route changes release the previous book's pixels.
    pub(super) fn clear_prepared_sleep_screen(&mut self) {
        self.sleep_screen = SleepScreenState::default();
        if let Some(ui) = self.ui.upgrade() {
            let state = ui.global::<SleepScreenModel>();
            state.set_active(false);
            state.set_cover(slint::Image::default());
            state.set_has_cover(false);
            state.set_title("".into());
            state.set_author("".into());
        }
    }

    /// This hot path only publishes already decoded pixels and bounded strings.
    pub(super) fn show_sleep_screen(&mut self) -> Result<()> {
        let ui = self
            .ui
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("The sleep presentation window is unavailable"))?;
        let state = ui.global::<SleepScreenModel>();
        let prepared = self.prefs.sleep_cover
            && self.current_sleep_book_id().is_some()
            && self.current_sleep_book_id() == self.sleep_screen.book_id.as_deref();
        if prepared {
            state.set_title(self.sleep_screen.title.clone());
            state.set_author(self.sleep_screen.author.clone());
            state.set_has_cover(self.sleep_screen.cover.is_some());
            state.set_cover(self.sleep_screen.cover.clone().unwrap_or_default());
        } else {
            state.set_title("WeRead for Kindle".into());
            state.set_author("".into());
            state.set_has_cover(false);
            state.set_cover(slint::Image::default());
        }
        state.set_active(true);
        Ok(())
    }

    /// Hiding before the wake request lets the first full frame be the Reader.
    pub(super) fn hide_sleep_screen(&mut self) {
        if let Some(ui) = self.ui.upgrade() {
            let state = ui.global::<SleepScreenModel>();
            state.set_active(false);
            // Only the prepared snapshot owns pixels while awake. A hidden
            // overlay must not retain an older source after a later refresh.
            state.set_cover(slint::Image::default());
            state.set_has_cover(false);
            state.set_title("".into());
            state.set_author("".into());
        }
    }
}

#[cfg(test)]
#[path = "sleep_screen_tests.rs"]
mod tests;
