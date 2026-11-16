//! Prepare bounded cover pixels on one worker outside the UI and power paths.

use super::*;
use anyhow::{Context, ensure};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    thread::JoinHandle,
};

const MAX_SLEEP_COVER_WIDTH: u32 = 1536;
const MAX_SLEEP_COVER_HEIGHT: u32 = 2048;
const MAX_SLEEP_COVER_URL_BYTES: usize = 8192;
const MAX_SLEEP_TITLE_SCALARS: usize = 256;
const MAX_SLEEP_AUTHOR_SCALARS: usize = 128;
const MAX_COVER_INPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_RETAINED_COVER_INPUT_BYTES: usize = 2 * MAX_COVER_INPUT_BYTES;
const MAX_COVER_WORK: usize = 3;
const READER_COVER_IDLE: Duration = Duration::from_secs(4);
const COVER_RETRY_DELAY: Duration = Duration::from_secs(60);

type CoverPixels = slint::SharedPixelBuffer<slint::Rgb8Pixel>;

#[derive(Default)]
pub(super) struct SleepScreenState {
    book_id: Option<String>,
    source_key: Option<String>,
    thumbnail_key: Option<String>,
    title: slint::SharedString,
    author: slint::SharedString,
    cover: Option<slint::Image>,
    full_resolution: bool,
    reload_needed: bool,
    skip_full_cache: bool,
    skip_thumbnail_cache: bool,
    preparation: CoverPreparationState,
}

#[derive(Clone)]
struct CoverIdentity {
    request: u64,
    account_epoch: u64,
    book_id: String,
    url: String,
    geometry: (u32, u32),
    route: Route,
}

#[derive(Clone)]
enum CoverInput {
    Thumbnail(Arc<CoverSource>),
    SleepOriginal(Arc<CoverSource>),
    SleepCached {
        path: PathBuf,
        full_resolution: bool,
    },
}

struct CoverSource {
    // Field order keeps source storage alive only while its admission is owned.
    bytes: Vec<u8>,
    _credits: Option<crate::jobs::JobCredits>,
}

impl CoverInput {
    fn original_bytes(&self) -> usize {
        match self {
            Self::Thumbnail(source) | Self::SleepOriginal(source) => source.bytes.len(),
            Self::SleepCached { .. } => 0,
        }
    }

    fn is_sleep(&self) -> bool {
        !matches!(self, Self::Thumbnail(_))
    }

    fn source_guard(&self) -> Option<Arc<CoverSource>> {
        match self {
            Self::Thumbnail(source) | Self::SleepOriginal(source) => Some(source.clone()),
            Self::SleepCached { .. } => None,
        }
    }
}

struct CoverWork {
    identity: CoverIdentity,
    input: CoverInput,
}

enum PreparedCover {
    Thumbnail {
        png: crate::cache::PreparedPng,
        pixels: CoverPixels,
    },
    SleepOriginal {
        png: crate::cache::PreparedPng,
        pixels: CoverPixels,
    },
    SleepCached {
        pixels: CoverPixels,
        full_resolution: bool,
    },
}

struct ActiveCoverWork {
    work: CoverWork,
    cancelled: Arc<AtomicBool>,
    receiver: mpsc::Receiver<CoverCompletion>,
    handle: JoinHandle<()>,
}

struct CoverCompletion {
    request: u64,
    elapsed_us: u64,
    result: Result<PreparedCover>,
    // Results drop before this guard can return the payload's admission.
    _source_guard: Option<Arc<CoverSource>>,
}

#[derive(Default)]
struct CoverPreparationState {
    active: Option<ActiveCoverWork>,
    queued: VecDeque<CoverWork>,
    reader_idle_until: Option<Instant>,
    retry_at: Option<Instant>,
    worker_retry_at: Option<Instant>,
    started: u64,
    cancelled: u64,
    completed: u64,
    stale: u64,
    last_worker_us: u64,
    last_ui_commit_us: u64,
}

impl Drop for CoverPreparationState {
    fn drop(&mut self) {
        if let Some(active) = &self.active {
            active.cancelled.store(true, Ordering::SeqCst);
        }
    }
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

fn check_cancelled(cancelled: &AtomicBool) -> Result<()> {
    ensure!(
        !cancelled.load(Ordering::SeqCst),
        crate::cache::PreparationCancelled
    );
    Ok(())
}

fn rgb_pixels(gray: &image::GrayImage, cancelled: &AtomicBool) -> Result<CoverPixels> {
    check_cancelled(cancelled)?;
    ensure!(
        gray.width() > 0 && gray.height() > 0,
        "The prepared cover is empty"
    );
    let mut pixels = CoverPixels::new(gray.width(), gray.height());
    for (row, source) in pixels
        .make_mut_bytes()
        .chunks_exact_mut(gray.width() as usize * 3)
        .zip(gray.as_raw().chunks_exact(gray.width() as usize))
    {
        check_cancelled(cancelled)?;
        for (destination, source) in row.chunks_exact_mut(3).zip(source) {
            let value = (((u16::from(*source) + 8) / 17).min(15) * 17) as u8;
            destination.fill(value);
        }
    }
    check_cancelled(cancelled)?;
    Ok(pixels)
}

fn sleep_pixels(
    gray: image::GrayImage,
    geometry: (u32, u32),
    cancelled: &AtomicBool,
) -> Result<CoverPixels> {
    check_cancelled(cancelled)?;
    // Preserve the established Triangle sizing, including thumbnail upscaling.
    let grayscale = image::DynamicImage::ImageLuma8(gray)
        .resize(
            geometry.0,
            geometry.1,
            image::imageops::FilterType::Triangle,
        )
        .into_luma8();
    check_cancelled(cancelled)?;
    ensure!(
        grayscale.width() <= MAX_SLEEP_COVER_WIDTH && grayscale.height() <= MAX_SLEEP_COVER_HEIGHT,
        "The prepared sleep cover exceeds its pixel budget"
    );
    rgb_pixels(&grayscale, cancelled)
}

fn decode_cached_cover(
    path: &Path,
    geometry: (u32, u32),
    cancelled: &AtomicBool,
) -> Result<CoverPixels> {
    check_cancelled(cancelled)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "Invalid cached sleep cover");
    ensure!(
        metadata.len() <= 16 * 1024 * 1024,
        "Cached sleep cover exceeds its byte limit"
    );
    let mut encoded = Vec::with_capacity(usize::try_from(metadata.len())?);
    let mut digest = Sha256::new();
    let mut chunk = [0_u8; 32 * 1024];
    loop {
        check_cancelled(cancelled)?;
        let remaining = (16 * 1024 * 1024 + 1_usize).saturating_sub(encoded.len());
        ensure!(remaining > 0, "Cached sleep cover exceeds its byte limit");
        let count = file.read(&mut chunk[..remaining.min(32 * 1024)])?;
        if count == 0 {
            break;
        }
        ensure!(
            count <= (16 * 1024 * 1024_usize).saturating_sub(encoded.len()),
            "Cached sleep cover exceeds its byte limit"
        );
        if encoded.capacity().saturating_sub(encoded.len()) < count {
            encoded.try_reserve_exact(count)?;
        }
        encoded.extend_from_slice(&chunk[..count]);
        digest.update(&chunk[..count]);
        ensure!(
            encoded.len() <= 16 * 1024 * 1024,
            "Cached sleep cover exceeds its byte limit"
        );
    }
    let expected = path
        .file_stem()
        .and_then(|value| value.to_str())
        .and_then(|value| value.rsplit_once('-').map(|(_, digest)| digest))
        .context("The cached sleep cover has no admitted digest")?;
    ensure!(
        hex::encode(digest.finalize()) == expected,
        "Cached sleep cover changed after admission"
    );
    check_cancelled(cancelled)?;
    let mut reader =
        image::ImageReader::new(std::io::Cursor::new(encoded)).with_guessed_format()?;
    ensure!(
        reader.format() == Some(image::ImageFormat::Png),
        "The cached sleep cover is not PNG"
    );
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SLEEP_COVER_WIDTH.max(400));
    limits.max_image_height = Some(MAX_SLEEP_COVER_HEIGHT);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let grayscale = reader.decode()?.into_luma8();
    check_cancelled(cancelled)?;
    sleep_pixels(grayscale, geometry, cancelled)
}

fn prepare_cover_work(
    input: CoverInput,
    geometry: (u32, u32),
    cancelled: &AtomicBool,
) -> Result<PreparedCover> {
    match input {
        CoverInput::Thumbnail(source) => {
            let png = FileCache::prepare_cover_png_cancellable(&source.bytes, 400, 560, cancelled)?;
            let pixels = rgb_pixels(png.gray(), cancelled)?;
            Ok(PreparedCover::Thumbnail { png, pixels })
        }
        CoverInput::SleepOriginal(source) => {
            let png = FileCache::prepare_cover_png_cancellable(
                &source.bytes,
                geometry.0,
                geometry.1,
                cancelled,
            )?;
            let pixels = sleep_pixels(png.gray().clone(), geometry, cancelled)?;
            Ok(PreparedCover::SleepOriginal { png, pixels })
        }
        CoverInput::SleepCached {
            path,
            full_resolution,
        } => {
            let pixels = decode_cached_cover(&path, geometry, cancelled)?;
            Ok(PreparedCover::SleepCached {
                pixels,
                full_resolution,
            })
        }
    }
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

    pub(super) fn note_cover_reading_activity(&mut self) {
        self.sleep_screen.preparation.reader_idle_until = Some(Instant::now() + READER_COVER_IDLE);
        if self
            .sleep_screen
            .preparation
            .active
            .as_ref()
            .is_some_and(|active| !active.cancelled.swap(true, Ordering::SeqCst))
        {
            self.sleep_screen.preparation.cancelled += 1;
        }
    }

    pub(super) fn cover_preparation_diagnostics(&self) -> serde_json::Value {
        let state = &self.sleep_screen.preparation;
        serde_json::json!({
            "ready": self.sleep_screen.cover.is_some(),
            "full_resolution": self.sleep_screen.full_resolution,
            "pending": self.cover_preparation_busy() || self.sleep_screen.reload_needed,
            "active": state.active.is_some(),
            "queued": state.queued.len(),
            "retained_input_bytes": self.retained_cover_input_bytes(),
            "started": state.started,
            "cancelled": state.cancelled,
            "completed": state.completed,
            "stale": state.stale,
            "last_worker_us": state.last_worker_us,
            "last_ui_commit_us": state.last_ui_commit_us,
        })
    }

    pub(super) fn cover_preparation_busy(&self) -> bool {
        self.sleep_screen.preparation.active.is_some()
            || !self.sleep_screen.preparation.queued.is_empty()
    }

    pub(super) fn sleep_cover_admission_ready(&self) -> bool {
        !self.managed_sleep_blocked
            && !self.image_preparation_busy()
            && !self.article_preparation.is_busy()
            && self
                .sleep_screen
                .preparation
                .reader_idle_until
                .is_some_and(|until| Instant::now() >= until)
            && self
                .sleep_screen
                .preparation
                .retry_at
                .is_none_or(|until| Instant::now() >= until)
    }

    pub(super) fn sleep_cover_cached_full_usable(&self) -> bool {
        !self.sleep_screen.skip_full_cache
    }

    pub(super) fn cover_work_pending(&self, book_id: &str, url: &str) -> bool {
        let matches = |work: &CoverWork| {
            work.identity.book_id == book_id
                && work.identity.url == url
                && work.identity.account_epoch == self.jobs.account_epoch()
        };
        self.sleep_screen
            .preparation
            .active
            .as_ref()
            .is_some_and(|active| matches(&active.work))
            || self.sleep_screen.preparation.queued.iter().any(matches)
    }

    pub(super) fn cover_preparation_has_capacity(&self) -> bool {
        let state = &self.sleep_screen.preparation;
        state.queued.len() + usize::from(state.active.is_some()) < MAX_COVER_WORK
            && self.retained_cover_input_bytes() < MAX_RETAINED_COVER_INPUT_BYTES
    }

    fn retained_cover_input_bytes(&self) -> usize {
        self.sleep_screen
            .preparation
            .queued
            .iter()
            .map(|work| work.input.original_bytes())
            .sum::<usize>()
            + self
                .sleep_screen
                .preparation
                .active
                .as_ref()
                .map_or(0, |active| active.work.input.original_bytes())
    }

    fn push_cover_work(&mut self, work: CoverWork) -> bool {
        if !self.cover_preparation_has_capacity()
            || work.input.original_bytes()
                > MAX_RETAINED_COVER_INPUT_BYTES.saturating_sub(self.retained_cover_input_bytes())
        {
            return false;
        }
        self.sleep_screen.preparation.queued.push_back(work);
        true
    }

    pub(super) fn queue_downloaded_cover(
        &mut self,
        request: u64,
        account_epoch: u64,
        book_id: &str,
        url: &str,
        bytes: Vec<u8>,
        credits: Option<crate::jobs::JobCredits>,
    ) -> Result<bool> {
        // Establish payload-before-credit drop order before any admission can fail.
        let source = Arc::new(CoverSource {
            bytes,
            _credits: credits,
        });
        ensure!(
            source.bytes.len() <= MAX_COVER_INPUT_BYTES,
            "Downloaded cover exceeds its byte limit"
        );
        ensure!(
            thumbnail_key(book_id, url).is_some(),
            "Invalid cover source identity"
        );
        let work = CoverWork {
            identity: CoverIdentity {
                request,
                account_epoch,
                book_id: book_id.into(),
                url: url.into(),
                geometry: self.sleep_cover_geometry(),
                route: self.route.clone(),
            },
            input: CoverInput::Thumbnail(source),
        };
        Ok(self.push_cover_work(work))
    }

    /// Reader rendering only records bounded metadata. Pixel work waits for idle.
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
            self.cancel_sleep_cover_work();
            self.sleep_screen.book_id = Some(book.id.clone());
            self.sleep_screen.source_key = source_key;
            self.sleep_screen.thumbnail_key = thumbnail_key;
            self.sleep_screen.cover = None;
            self.sleep_screen.full_resolution = false;
            self.sleep_screen.reload_needed = true;
            self.sleep_screen.skip_full_cache = false;
            self.sleep_screen.skip_thumbnail_cache = false;
            self.sleep_screen.preparation.retry_at = None;
            self.note_cover_reading_activity();
        }
    }

    fn cancel_sleep_cover_work(&mut self) {
        if let Some(active) = &self.sleep_screen.preparation.active {
            if active.work.input.is_sleep() {
                active.cancelled.store(true, Ordering::SeqCst);
            }
        }
        self.sleep_screen
            .preparation
            .queued
            .retain(|work| !work.input.is_sleep());
    }

    pub(super) fn cancel_cover_preparation(&mut self) {
        let had_thumbnail = self
            .sleep_screen
            .preparation
            .active
            .as_ref()
            .is_some_and(|active| !active.work.input.is_sleep())
            || self
                .sleep_screen
                .preparation
                .queued
                .iter()
                .any(|work| !work.input.is_sleep());
        if self
            .sleep_screen
            .preparation
            .active
            .as_ref()
            .is_some_and(|active| !active.cancelled.swap(true, Ordering::SeqCst))
        {
            self.sleep_screen.preparation.cancelled += 1;
        }
        self.sleep_screen.preparation.queued.clear();
        if had_thumbnail {
            self.deferred_cover_refill = Some(self.route.clone());
        }
        // A quiesced cached read has not consumed the book's preparation intent.
        // Wake of the same book must be able to reload its existing full PNG.
        if self.sleep_screen.book_id.is_some() && !self.sleep_screen.full_resolution {
            self.sleep_screen.reload_needed = true;
        }
    }

    pub(super) fn note_cover_source_failure(&mut self, book_id: &str) {
        if self.sleep_screen.book_id.as_deref() == Some(book_id) {
            self.sleep_screen.preparation.retry_at = Some(Instant::now() + COVER_RETRY_DELAY);
        }
    }

    fn record_cover_preparation_failure(&mut self, book_id: &str) -> Result<()> {
        self.store
            .set_setting(&format!("cover.failed:{book_id}"), &Utc::now().timestamp())?;
        self.note_cover_source_failure(book_id);
        Ok(())
    }

    fn cover_work_relevant(&self, work: &CoverWork) -> bool {
        let identity = &work.identity;
        if identity.account_epoch != self.jobs.account_epoch()
            || self.current_cover_url(&identity.book_id).as_deref() != Some(identity.url.as_str())
        {
            return false;
        }
        if !work.input.is_sleep() {
            return true;
        }
        self.prefs.sleep_cover
            && self.current_sleep_book_id() == Some(identity.book_id.as_str())
            && self.sleep_screen.book_id.as_deref() == Some(identity.book_id.as_str())
            && self.sleep_screen.source_key == cover_key(&identity.book_id, &identity.url)
            && self.sleep_screen.thumbnail_key == thumbnail_key(&identity.book_id, &identity.url)
            && identity.geometry == self.sleep_cover_geometry()
            && identity.route == self.route
    }

    fn queue_cached_sleep_cover(&mut self) {
        if !self.sleep_screen.reload_needed || !self.sleep_cover_admission_ready() {
            return;
        }
        let Some(book_id) = self.sleep_screen.book_id.clone() else {
            return;
        };
        let Some(url) = self.current_cover_url(&book_id) else {
            return;
        };
        if self.cover_work_pending(&book_id, &url) {
            return;
        }
        let full = (!self.sleep_screen.skip_full_cache)
            .then_some(())
            .and_then(|_| self.sleep_screen.source_key.as_deref())
            .and_then(|key| self.cache.get(key).ok().flatten());
        let cached = full.map(|path| (path, true)).or_else(|| {
            (!self.sleep_screen.skip_thumbnail_cache)
                .then_some(())
                .and_then(|_| self.sleep_screen.thumbnail_key.as_deref())
                .and_then(|key| self.cache.get(key).ok().flatten())
                .map(|path| (path, false))
        });
        self.sleep_screen.reload_needed = false;
        let Some((path, full_resolution)) = cached else {
            return;
        };
        let request = self.request();
        let work = CoverWork {
            identity: CoverIdentity {
                request,
                account_epoch: self.jobs.account_epoch(),
                book_id,
                url,
                geometry: self.sleep_cover_geometry(),
                route: self.route.clone(),
            },
            input: CoverInput::SleepCached {
                path,
                full_resolution,
            },
        };
        if !self.push_cover_work(work) {
            self.sleep_screen.reload_needed = true;
        }
    }

    fn finish_cover_work(&mut self, work: CoverWork, result: Result<PreparedCover>) -> Result<()> {
        if !self.cover_work_relevant(&work) {
            self.sleep_screen.preparation.stale += 1;
            if work.input.is_sleep()
                && self.current_sleep_book_id() == Some(work.identity.book_id.as_str())
            {
                self.sleep_screen.reload_needed = true;
            }
            return Ok(());
        }
        let prepared = match result {
            Ok(prepared) => prepared,
            Err(error)
                if error
                    .downcast_ref::<crate::cache::PreparationCancelled>()
                    .is_some() =>
            {
                // The old slot is drained before the same bounded input can wait again.
                let _ = self.push_cover_work(work);
                return Ok(());
            }
            Err(_) if matches!(&work.input, CoverInput::SleepCached { .. }) => {
                if matches!(
                    &work.input,
                    CoverInput::SleepCached {
                        full_resolution: true,
                        ..
                    }
                ) {
                    self.sleep_screen.skip_full_cache = true;
                } else {
                    self.sleep_screen.skip_thumbnail_cache = true;
                }
                // A local eviction/corruption window is not a remote-source failure.
                self.sleep_screen.reload_needed = true;
                return Ok(());
            }
            Err(_) => {
                self.record_cover_preparation_failure(&work.identity.book_id)?;
                return Ok(());
            }
        };
        if self.managed_sleep_blocked {
            let _ = self.push_cover_work(work);
            return Ok(());
        }
        let identity = &work.identity;
        match prepared {
            PreparedCover::Thumbnail { png, pixels } => {
                let key =
                    thumbnail_key(&identity.book_id, &identity.url).context("Missing cover key")?;
                if self.cache.put_prepared_png(&key, &png).is_err() {
                    self.record_cover_preparation_failure(&identity.book_id)?;
                    return Ok(());
                }
                self.store
                    .delete_setting(&format!("cover.failed:{}", identity.book_id))?;
                let image = slint::Image::from_rgb8(pixels);
                if self
                    .visible_cover_ids()
                    .iter()
                    .any(|id| id == &identity.book_id)
                {
                    self.cover_ui_changes
                        .insert(identity.book_id.clone(), image.clone());
                }
                if self.prefs.sleep_cover
                    && self.current_sleep_book_id() == Some(identity.book_id.as_str())
                    && self.sleep_screen.source_key == cover_key(&identity.book_id, &identity.url)
                    && !self.sleep_screen.full_resolution
                {
                    if self.sleep_screen.cover.is_none() {
                        self.sleep_screen.cover = Some(image);
                    }
                    if let CoverInput::Thumbnail(source) = &work.input {
                        let request = self.request();
                        let full = CoverWork {
                            identity: CoverIdentity {
                                request,
                                account_epoch: identity.account_epoch,
                                book_id: identity.book_id.clone(),
                                url: identity.url.clone(),
                                geometry: self.sleep_cover_geometry(),
                                route: self.route.clone(),
                            },
                            input: CoverInput::SleepOriginal(source.clone()),
                        };
                        // The derivative now owns both the original bytes and their credit.
                        // Release the old Arc before any rejected admission can drop the credit.
                        drop(work);
                        if !self.push_cover_work(full) {
                            self.sleep_screen.reload_needed = true;
                        }
                    }
                }
            }
            PreparedCover::SleepOriginal { png, pixels } => {
                let key = cover_key(&identity.book_id, &identity.url)
                    .context("Missing sleep cover key")?;
                if self.cache.put_prepared_png(&key, &png).is_err() {
                    self.record_cover_preparation_failure(&identity.book_id)?;
                    return Ok(());
                }
                self.store
                    .delete_setting(&format!("cover.failed:{}", identity.book_id))?;
                self.sleep_screen.cover = Some(slint::Image::from_rgb8(pixels));
                self.sleep_screen.full_resolution = true;
                self.sleep_screen.reload_needed = false;
                self.sleep_screen.skip_full_cache = false;
                self.sleep_screen.preparation.retry_at = None;
            }
            PreparedCover::SleepCached {
                pixels,
                full_resolution,
            } => {
                self.sleep_screen.cover = Some(slint::Image::from_rgb8(pixels));
                self.sleep_screen.full_resolution = full_resolution;
            }
        }
        self.sleep_screen.preparation.completed += 1;
        Ok(())
    }

    /// Completion never requests a Reader render or a display refresh.
    pub(super) fn poll_cover_preparation(&mut self) -> Result<()> {
        if self
            .sleep_screen
            .preparation
            .active
            .as_ref()
            .is_some_and(|active| active.handle.is_finished())
        {
            let active = self.sleep_screen.preparation.active.take().unwrap();
            let cancelled = active.cancelled.load(Ordering::SeqCst);
            let joined = active.handle.join();
            let result = if joined.is_ok() {
                match active.receiver.try_recv() {
                    Ok(completion) if completion.request == active.work.identity.request => {
                        self.sleep_screen.preparation.last_worker_us = completion.elapsed_us;
                        if cancelled {
                            Err(crate::cache::PreparationCancelled.into())
                        } else {
                            completion.result
                        }
                    }
                    _ => Err(anyhow::anyhow!("Cover worker returned no matching result")),
                }
            } else {
                Err(anyhow::anyhow!("Cover worker stopped unexpectedly"))
            };
            let publication_started = Instant::now();
            let finished = self.finish_cover_work(active.work, result);
            self.sleep_screen.preparation.last_ui_commit_us = publication_started
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX))
                as u64;
            finished?;
        }
        if self.managed_sleep_blocked
            || self.image_preparation_busy()
            || self.article_preparation.is_busy()
        {
            return Ok(());
        }
        if self
            .sleep_screen
            .preparation
            .worker_retry_at
            .is_some_and(|until| Instant::now() < until)
        {
            return Ok(());
        }
        let reading = matches!(self.route, Route::Reader(..));
        if reading && !self.sleep_cover_admission_ready() {
            return Ok(());
        }
        self.queue_cached_sleep_cover();
        if self.sleep_screen.preparation.active.is_some() {
            return Ok(());
        }
        let queued = std::mem::take(&mut self.sleep_screen.preparation.queued);
        for work in queued {
            if self.cover_work_relevant(&work) {
                self.sleep_screen.preparation.queued.push_back(work);
            }
        }
        if let Some(index) = self
            .sleep_screen
            .preparation
            .queued
            .iter()
            .position(|work| !work.input.is_sleep())
            .or_else(|| (!self.sleep_screen.preparation.queued.is_empty()).then_some(0))
        {
            let work = self.sleep_screen.preparation.queued.remove(index).unwrap();
            let input = work.input.clone();
            let geometry = work.identity.geometry;
            let request = work.identity.request;
            let cancelled = Arc::new(AtomicBool::new(false));
            let worker_cancelled = cancelled.clone();
            let (sender, receiver) = mpsc::sync_channel(1);
            match std::thread::Builder::new()
                .name("weread-cover".into())
                .spawn(move || {
                    let source_guard = input.source_guard();
                    let started = Instant::now();
                    let result = prepare_cover_work(input, geometry, &worker_cancelled);
                    let elapsed_us = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
                    let _ = sender.send(CoverCompletion {
                        request,
                        elapsed_us,
                        result,
                        _source_guard: source_guard,
                    });
                }) {
                Ok(handle) => {
                    self.sleep_screen.preparation.worker_retry_at = None;
                    self.sleep_screen.preparation.started += 1;
                    self.sleep_screen.preparation.active = Some(ActiveCoverWork {
                        work,
                        cancelled,
                        receiver,
                        handle,
                    });
                }
                Err(_) => {
                    self.sleep_screen.preparation.queued.push_front(work);
                    self.sleep_screen.preparation.worker_retry_at =
                        Some(Instant::now() + READER_COVER_IDLE);
                }
            }
        } else if reading
            && !self.sleep_screen.full_resolution
            && self.sleep_screen.source_key.is_some()
        {
            self.schedule_visible_covers();
        }
        Ok(())
    }

    pub(super) fn quiesce_cover_preparation(&mut self, timeout: Duration) -> Result<()> {
        self.cancel_cover_preparation();
        let deadline = Instant::now() + timeout;
        while self
            .sleep_screen
            .preparation
            .active
            .as_ref()
            .is_some_and(|active| !active.handle.is_finished())
        {
            ensure!(
                Instant::now() < deadline,
                "Cover preparation did not stop before its deadline"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        if let Some(active) = self.sleep_screen.preparation.active.take() {
            let _ = active.handle.join();
            let _ = active.receiver.try_recv();
        }
        Ok(())
    }

    /// Account and route changes release pixels, while a cancelled worker keeps its slot.
    pub(super) fn clear_prepared_sleep_screen(&mut self) {
        self.cancel_sleep_cover_work();
        let preparation = std::mem::take(&mut self.sleep_screen.preparation);
        self.sleep_screen = SleepScreenState {
            preparation,
            ..Default::default()
        };
        if let Some(ui) = self.ui.upgrade() {
            let state = ui.global::<SleepScreenModel>();
            state.set_active(false);
            state.set_cover(slint::Image::default());
            state.set_has_cover(false);
            state.set_title("".into());
            state.set_author("".into());
        }
    }

    /// This hot path only publishes decoded pixels and bounded strings, then cancels optional work.
    pub(super) fn show_sleep_screen(&mut self) -> Result<()> {
        self.note_cover_reading_activity();
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
        self.suspend_image_processing_for_sleep()?;
        Ok(())
    }

    pub(super) fn hide_sleep_screen(&mut self) -> Result<()> {
        if let Some(ui) = self.ui.upgrade() {
            let state = ui.global::<SleepScreenModel>();
            state.set_active(false);
            state.set_cover(slint::Image::default());
            state.set_has_cover(false);
            state.set_title("".into());
            state.set_author("".into());
        }
        self.note_cover_reading_activity();
        self.restore_image_processing_after_sleep()
    }

    #[cfg(test)]
    pub(super) fn drain_cover_preparation_for_test(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.sleep_screen.preparation.reader_idle_until = Some(Instant::now());
            self.sleep_screen.preparation.retry_at = None;
            let _ = self.poll_image_preparation()?;
            self.poll_cover_preparation()?;
            if !self.cover_preparation_busy() && !self.sleep_screen.reload_needed {
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "The cover fixture did not finish"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[cfg(test)]
#[path = "sleep_screen_tests.rs"]
mod tests;
