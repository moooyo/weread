use super::*;
use crate::domain::BookMetadata;
use sha2::{Digest, Sha256};

pub(super) struct BookTileMetadata<'a> {
    id: &'a str,
    title: &'a str,
    author: &'a str,
    cover_url: &'a str,
    source: &'a str,
    progress: f32,
    cached_chapters: u32,
    updated: bool,
    finished: bool,
}

pub(super) trait BookTileFields {
    fn tile_metadata(&self) -> BookTileMetadata<'_>;
}

impl BookTileFields for Book {
    fn tile_metadata(&self) -> BookTileMetadata<'_> {
        BookTileMetadata {
            id: &self.id,
            title: &self.title,
            author: &self.author,
            cover_url: &self.cover_url,
            source: &self.source,
            progress: self.progress,
            cached_chapters: self.cached_chapters,
            updated: self.updated,
            finished: self.finished,
        }
    }
}

impl BookTileFields for BookMetadata {
    fn tile_metadata(&self) -> BookTileMetadata<'_> {
        BookTileMetadata {
            id: &self.id,
            title: &self.title,
            author: &self.author,
            cover_url: &self.cover_url,
            source: &self.source,
            progress: self.progress,
            cached_chapters: self.cached_chapters,
            updated: self.updated,
            finished: self.finished,
        }
    }
}

pub(super) struct ReviewRequestContext {
    request: u64,
    book_id: String,
    latest: bool,
    cache_key: String,
    account_epoch: u64,
}

#[cfg(test)]
std::thread_local! {
    static BULK_IMAGE_TEST_WORK: Cell<Option<u64>> = const { Cell::new(None) };
}

#[cfg(test)]
pub(super) fn with_bulk_image_test_work<T>(work: u64, operation: impl FnOnce() -> T) -> T {
    struct Restore(Option<u64>);
    impl Drop for Restore {
        fn drop(&mut self) {
            BULK_IMAGE_TEST_WORK.with(|limit| limit.set(self.0));
        }
    }
    let _restore = Restore(BULK_IMAGE_TEST_WORK.with(|limit| limit.replace(Some(work))));
    operation()
}

#[cfg(test)]
#[path = "library_download_resource_tests.rs"]
mod download_resource_tests;

#[cfg(test)]
#[path = "shelf_merge_budget_tests.rs"]
mod shelf_merge_budget_tests;

#[cfg(test)]
#[path = "content_owner_snapshot_tests.rs"]
mod content_owner_snapshot_tests;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ContentOwnerWork {
    snapshots: usize,
    snapshot_string_bytes: usize,
    owners: usize,
    identity_bytes: usize,
    cookie_serializations: usize,
    status_checks: usize,
}

#[cfg(test)]
std::thread_local! {
    static CONTENT_OWNER_WORK: std::cell::Cell<ContentOwnerWork> =
        std::cell::Cell::new(ContentOwnerWork::default());
}

#[cfg(test)]
fn observe_content_owner_work(observe: impl FnOnce(&mut ContentOwnerWork)) {
    CONTENT_OWNER_WORK.with(|slot| {
        let mut work = slot.get();
        observe(&mut work);
        slot.set(work);
    });
}

#[cfg(test)]
fn session_string_bytes(session: &source::AuthSession) -> usize {
    let fields = [
        &session.api_key,
        &session.account_name,
        &session.user_vid,
        &session.wr_ticket,
        &session.wr_wrpa,
    ]
    .into_iter()
    .map(String::len)
    .sum::<usize>();
    session.cookies.iter().fold(fields, |bytes, (name, value)| {
        bytes + name.len() + value.len()
    })
}

/// Apply the existing permission rule to an operation's fresh owner snapshot.
/// Metadata presence alone never makes remote content authorized or full.
pub(super) fn chapter_content_status_for_owner(
    book: &Book,
    chapter: &Chapter,
    owner: Option<&str>,
) -> ContentStatus {
    chapter_content_status_for_source(&book.source, chapter, owner)
}

pub(super) fn chapter_content_status_for_metadata_owner(
    book: &BookMetadata,
    chapter: &Chapter,
    owner: Option<&str>,
) -> ContentStatus {
    chapter_content_status_for_source(&book.source, chapter, owner)
}

pub(super) fn chapter_content_status_for_access_owner(
    source: &str,
    entry: &crate::store::ChapterAccessMetadata,
    owner: Option<&str>,
) -> ContentStatus {
    chapter_content_status_for_evidence(
        source,
        entry.chapter.cached,
        entry.locally_readable,
        entry.chapter.content_status,
        &entry.chapter.content_account_key,
        owner,
    )
}

fn chapter_content_status_for_source(
    source: &str,
    chapter: &Chapter,
    owner: Option<&str>,
) -> ContentStatus {
    chapter_content_status_for_evidence(
        source,
        chapter.cached,
        source != "weread"
            && chapter
                .body
                .as_deref()
                .is_some_and(|body| !body.trim().is_empty()),
        chapter.content_status,
        &chapter.content_account_key,
        owner,
    )
}

fn chapter_content_status_for_evidence(
    source: &str,
    cached: bool,
    locally_readable: bool,
    content_status: ContentStatus,
    content_account_key: &str,
    owner: Option<&str>,
) -> ContentStatus {
    #[cfg(test)]
    observe_content_owner_work(|work| work.status_checks += 1);
    if source != "weread" {
        return if cached || locally_readable {
            ContentStatus::Full
        } else {
            ContentStatus::Unknown
        };
    }
    if content_account_key.is_empty() || owner != Some(content_account_key) {
        ContentStatus::Unknown
    } else {
        content_status
    }
}

pub(super) fn chapter_content_label_for_status(
    status: ContentStatus,
    body_present: bool,
    current: bool,
) -> String {
    let state = match status {
        ContentStatus::Preview => "content_preview",
        ContentStatus::Locked => "content_locked",
        ContentStatus::Full => "downloaded",
        ContentStatus::Unknown if body_present => "content_unverified",
        ContentStatus::Unknown => "not_downloaded",
    };
    if current {
        format!("{} · {}", tr("current_chapter"), tr(state))
    } else {
        tr(state)
    }
}

fn detail_page_size(profile: DeviceProfile, tab: &str) -> usize {
    match tab {
        "notes" => {
            if profile.is_scribe() {
                4
            } else {
                2
            }
        }
        "reviews" => {
            if profile.is_scribe() {
                5
            } else {
                3
            }
        }
        _ => profile.detail_chapter_page_size(),
    }
}

fn detail_tab_page(
    tab: &str,
    position: Option<u32>,
    chapter_page_size: usize,
    selecting: &mut bool,
    selected: &mut BTreeSet<u32>,
) -> usize {
    *selecting = false;
    selected.clear();
    if tab == "toc" {
        position.unwrap_or(0) as usize / chapter_page_size.max(1)
    } else {
        0
    }
}

#[cfg(test)]
fn detail_metadata_text(metadata: Option<&serde_json::Value>, key: &str) -> String {
    metadata
        .map(|value| {
            value
                .get("bookInfo")
                .or_else(|| value.get("book"))
                .unwrap_or(value)
        })
        .and_then(|value| value.get(key))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| tr("metadata_unavailable"))
}

fn detail_date(timestamp: i64) -> String {
    (timestamp > 0)
        .then(|| chrono::DateTime::from_timestamp(timestamp, 0))
        .flatten()
        .map(|time| {
            time.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
                .format("%m-%d")
                .to_string()
        })
        .unwrap_or_else(|| tr("metadata_unavailable"))
}

fn detail_note_rows(mut annotations: Vec<Annotation>, chapters: &[Chapter]) -> Vec<DetailReview> {
    annotations.sort_by_key(|note| (note.chapter_index, note.start, note.created_at));
    annotations
        .into_iter()
        .map(|note| DetailReview {
            id: note.id.into(),
            chapter: chapters
                .iter()
                .find(|chapter| chapter.index == note.chapter_index)
                .map(|chapter| chapter.title.clone())
                .unwrap_or_else(|| tr("chapter_unavailable"))
                .into(),
            date: detail_date(note.created_at).into(),
            quote: note.quote.into(),
            thought: note.note.into(),
            ..Default::default()
        })
        .collect()
}

fn known_tab_count(label: &str, count: usize) -> String {
    i18n::format(
        "tab_count",
        &[("label", tr(label)), ("count", count.to_string())],
    )
}

pub(super) fn normalize_book(
    remote: &source::RemoteBook,
    previous: Option<Book>,
    on_shelf: bool,
) -> Book {
    let old = previous.unwrap_or_default();
    Book {
        id: remote.id.clone(),
        title: if remote.title.is_empty() {
            old.title
        } else {
            remote.title.clone()
        },
        author: if remote.author.is_empty() {
            old.author
        } else {
            remote.author.clone()
        },
        category: if remote.category.is_empty() {
            old.category
        } else {
            remote.category.clone()
        },
        intro: if remote.intro.is_empty() {
            old.intro
        } else {
            remote.intro.clone()
        },
        cover_url: if remote.cover_url.is_empty() {
            old.cover_url
        } else {
            remote.cover_url.clone()
        },
        source: "weread".into(),
        progress: old.progress.max(remote.progress.min(100) as f32 / 100.0),
        total_chapters: old.total_chapters.max(remote.chapter_count),
        cached_chapters: old.cached_chapters,
        updated: old.updated,
        finished: old.finished || remote.finished,
        on_shelf,
    }
}

pub(super) fn merge_shelf(
    remote: &[source::RemoteBook],
    existing: &[Book],
    overrides: &BTreeMap<String, bool>,
) -> Vec<Book> {
    merge_shelf_with_observer(remote, existing, overrides, |_, _| {})
}

fn merge_shelf_with_observer(
    remote: &[source::RemoteBook],
    existing: &[Book],
    overrides: &BTreeMap<String, bool>,
    mut before_normalize: impl FnMut(&source::RemoteBook, Option<&Book>),
) -> Vec<Book> {
    let previous: BTreeMap<_, _> = existing
        .iter()
        .map(|book| (book.id.as_str(), book))
        .collect();
    let mut books = Vec::new();
    let mut slots = BTreeMap::new();
    for book in existing.iter().filter(|book| {
        book.on_shelf && (book.source != "weread" || overrides.get(&book.id) == Some(&true))
    }) {
        slots.entry(book.id.as_str()).or_insert(books.len());
        books.push(book.clone());
    }

    // Repeated source rows use the last record's fields, but a new identity
    // keeps its first-seen insertion order. The indices borrow input IDs;
    // remote-row normalization copies old metadata only after this complete
    // identity merge finishes.
    let mut latest = BTreeMap::new();
    let mut first_seen = Vec::new();
    for book in remote {
        if book.id.is_empty()
            || overrides.get(&book.id) == Some(&false)
            || previous
                .get(book.id.as_str())
                .is_some_and(|book| book.source != "weread")
        {
            continue;
        }
        if latest.insert(book.id.as_str(), book).is_none() {
            first_seen.push(book.id.as_str());
        }
    }
    for id in first_seen {
        let book = latest[id];
        let old = previous.get(id).copied();
        before_normalize(book, old);
        let normalized = normalize_book(book, old.cloned(), true);
        if let Some(position) = slots.get(id).copied() {
            books[position] = normalized;
        } else {
            slots.insert(id, books.len());
            books.push(normalized);
        }
    }
    books
}

// A project allowance for the complete UTF-8 QR payload, shared by Login and
// article links. A bounded final matrix does not bound earlier segmentation.
pub(super) const MAX_QR_PAYLOAD_BYTES: usize = 2 * 1024;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct QrInputWork {
    pub admissions: usize,
    pub rejections: usize,
    pub article_lookups: usize,
    pub article_copies: usize,
    pub article_url_copy_bytes: usize,
    pub uri_parses: usize,
    pub encodes: usize,
    pub publications: usize,
}

#[cfg(test)]
std::thread_local! {
    static QR_INPUT_WORK: Cell<QrInputWork> = Cell::new(QrInputWork::default());
    static QR_PAYLOAD_ALLOWANCE: Cell<usize> = const { Cell::new(MAX_QR_PAYLOAD_BYTES) };
}

#[cfg(test)]
pub(super) fn observe_qr_input_work(observe: impl FnOnce(&mut QrInputWork)) {
    QR_INPUT_WORK.with(|slot| {
        let mut work = slot.get();
        observe(&mut work);
        slot.set(work);
    });
}

#[cfg(test)]
pub(super) fn qr_input_work() -> QrInputWork {
    QR_INPUT_WORK.with(Cell::get)
}

#[cfg(test)]
pub(super) fn reset_qr_input_work() {
    QR_INPUT_WORK.with(|slot| slot.set(QrInputWork::default()));
}

#[cfg(test)]
pub(super) fn with_qr_input_test_limit<T>(maximum: usize, run: impl FnOnce() -> T) -> T {
    struct Restore(usize);
    impl Drop for Restore {
        fn drop(&mut self) {
            QR_PAYLOAD_ALLOWANCE.with(|slot| slot.set(self.0));
        }
    }
    let _restore =
        Restore(QR_PAYLOAD_ALLOWANCE.with(|slot| slot.replace(maximum.min(MAX_QR_PAYLOAD_BYTES))));
    run()
}

/// Inspect borrowed bytes before copying a link, parsing its URI, or entering
/// qrcode's unbounded pre-capacity segment collection. Rejection is atomic.
pub(super) fn admit_qr_payload(value: &str) -> source::Result<()> {
    #[cfg(test)]
    observe_qr_input_work(|work| work.admissions += 1);
    let maximum = MAX_QR_PAYLOAD_BYTES;
    #[cfg(test)]
    let maximum = maximum.min(QR_PAYLOAD_ALLOWANCE.with(Cell::get));
    if value.len() > maximum {
        #[cfg(test)]
        observe_qr_input_work(|work| work.rejections += 1);
        return Err(source::SourceError::ResponseTooLarge);
    }
    Ok(())
}

pub(super) fn qr_image(value: &str) -> Result<slint::Image> {
    admit_qr_payload(value)?;
    #[cfg(test)]
    observe_qr_input_work(|work| work.encodes += 1);
    let qr = qrcode::QrCode::new(value.as_bytes())?;
    let quiet = 4;
    let edge = qr.width() + quiet * 2;
    let mut buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(edge as u32, edge as u32);
    for (index, pixel) in buffer.make_mut_slice().iter_mut().enumerate() {
        let x = index % edge;
        let y = index / edge;
        let ink = x >= quiet
            && y >= quiet
            && x < edge - quiet
            && y < edge - quiet
            && qr[(x - quiet, y - quiet)] == qrcode::Color::Dark;
        let value = if ink { 0 } else { 255 };
        *pixel = slint::Rgb8Pixel {
            r: value,
            g: value,
            b: value,
        };
    }
    Ok(slint::Image::from_rgb8(buffer))
}

pub(super) fn download_account_key(session: &source::AuthSession) -> Option<String> {
    if !session.has_web_session() {
        return None;
    }
    let identity = if !session.user_vid.is_empty() {
        format!("weread-download-vid:{}", session.user_vid)
    } else {
        format!(
            "weread-download-session:{}",
            serde_json::to_string(&session.cookies).ok()?
        )
    };
    #[cfg(test)]
    observe_content_owner_work(|work| {
        work.owners += 1;
        work.identity_bytes += identity.len();
        work.cookie_serializations += usize::from(session.user_vid.is_empty());
    });
    Some(hex::encode(Sha256::digest(identity.as_bytes())))
}

fn transient_download_error(error: Option<&str>) -> bool {
    error.is_some_and(|error| {
        matches!(
            error,
            "source.network_unavailable"
                | "source.timeout"
                | "source.http.429"
                | "source.resource_http.429"
        ) || error
            .strip_prefix("source.http.")
            .or_else(|| error.strip_prefix("source.resource_http."))
            .and_then(|code| code.parse::<u16>().ok())
            .is_some_and(|code| (500..600).contains(&code))
    })
}

fn content_download_blocked(error: Option<&str>) -> bool {
    matches!(
        error,
        Some("source.unlock_required" | "source.content_unverified" | "source.content_changed")
    )
}

impl App {
    fn complete_download_images(&self, book: &str, uids: &[&str]) -> Result<Vec<bool>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        #[cfg(test)]
        if let Some(work) = BULK_IMAGE_TEST_WORK.with(Cell::get) {
            return self.book_assets.complete_many_with_verification_budget(
                &self.store,
                book,
                uids,
                work,
            );
        }
        self.book_assets.complete_many(&self.store, book, uids)
    }

    fn verify_download_images(
        &self,
        book: &str,
        uids: &[&str],
    ) -> Result<crate::book_assets::VerifiedBookImages> {
        #[cfg(test)]
        if let Some(work) = BULK_IMAGE_TEST_WORK.with(Cell::get) {
            return self
                .book_assets
                .verify_complete_book_with_verification_budget(&self.store, book, uids, work);
        }
        self.book_assets
            .verify_complete_book(&self.store, book, uids)
    }

    pub(super) fn reconcile_download_chapters(
        &self,
        task: &DownloadTask,
        headers: &[Chapter],
        updated_at: i64,
    ) -> Result<Vec<DownloadChapterState>> {
        if !task.include_images {
            return self.store.prepare_download_chapter_states(task, updated_at);
        }
        let selected: BTreeSet<_> = task.chapters.iter().copied().collect();
        let image_uids = headers
            .iter()
            .filter(|chapter| {
                selected.contains(&chapter.index)
                    && chapter.cached
                    && chapter.content_status == ContentStatus::Full
                    && chapter.content_account_key == task.account_key
            })
            .map(|chapter| chapter.id.as_str())
            .collect::<Vec<_>>();
        // One fresh operation checks cached content and images for the whole
        // selection. Verified completions never drop out of the durable count.
        let proof = self.verify_download_images(&task.book_id, &image_uids)?;
        self.store
            .prepare_download_chapter_states_with_verified_images(task, proof, updated_at)
    }

    pub(super) fn requested_review_cache_key(&self, request: u64, book_id: &str) -> Option<&str> {
        let context = self.review_context.as_ref()?;
        (context.request == request
            && self.review_request == request
            && context.book_id == book_id
            && self.detail_id == book_id
            && self.review_latest == context.latest
            && self.detail_tab == "reviews"
            && matches!(&self.route, Route::Detail(id) if id == book_id)
            && context.account_epoch == self.jobs.account_epoch())
        .then_some(context.cache_key.as_str())
    }

    pub(super) fn book_tile<B: BookTileFields>(
        &self,
        book: &B,
        row: usize,
        column: usize,
    ) -> BookTile {
        let book = book.tile_metadata();
        let digest = Sha256::digest(book.title.as_bytes());
        let tone = if book.source == "demo" {
            book.id
                .trim_start_matches("demo-")
                .parse::<usize>()
                .ok()
                .map(|n| [2, 3, 0, 1][n % 4])
                .unwrap_or(2)
        } else {
            (digest[0] % 4) as i32
        };
        let chars: Vec<_> = book.title.chars().collect();
        let vertical = chars
            .iter()
            .take(7)
            .map(char::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let extra = chars
            .iter()
            .skip(7)
            .take(7)
            .map(char::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let cover = sleep_screen::thumbnail_key(book.id, book.cover_url)
            .and_then(|key| self.cache.get(&key).ok().flatten())
            .and_then(|path| slint::Image::load_from_path(&path).ok());
        BookTile {
            id: book.id.into(),
            title: book.title.into(),
            author: book.author.into(),
            vertical: vertical.into(),
            vertical_extra: extra.into(),
            glyphs: model(
                chars
                    .iter()
                    .take(16)
                    .map(|character| character.to_string().into())
                    .collect(),
            ),
            initial: book.title.chars().next().unwrap_or(' ').to_string().into(),
            badge: if book.finished {
                tr("finished")
            } else if book.updated {
                tr("updated")
            } else {
                String::new()
            }
            .into(),
            tone,
            progress: book.progress,
            cached: book.cached_chapters > 0,
            selected: self.selected_books.contains(book.id),
            has_cover: cover.is_some(),
            cover: cover.unwrap_or_default(),
            row: row as i32,
            column: column as i32,
        }
    }

    fn visible_books(&self) -> Result<Vec<BookMetadata>> {
        let mut books = Vec::new();
        let mut extra = Vec::new();
        for book in self.store.book_metadata()? {
            if book.on_shelf {
                books.push(book);
            } else if !self.jobs.configured()
                && book.cached_chapters > 0
                && !book.id.starts_with("article:")
                && self
                    .store
                    .setting::<bool>(&format!("shelf.override:{}", book.id))?
                    != Some(false)
            {
                extra.push(book);
            }
        }
        // Shelf order was ascending rowid; offline extra-cache order was
        // descending rowid. One metadata snapshot retains both contracts.
        books.reverse();
        books.extend(extra);
        Ok(books)
    }

    fn filtered_books(&self) -> Result<Vec<BookMetadata>> {
        self.filtered_book_metadata(self.visible_books()?)
    }

    fn filtered_book_metadata(&self, mut books: Vec<BookMetadata>) -> Result<Vec<BookMetadata>> {
        match self.library_sort.as_str() {
            "update" => books.sort_by_key(|book| {
                (
                    !book.updated,
                    std::cmp::Reverse(
                        self.store
                            .setting::<i64>(&format!("book.updated_at:{}", book.id))
                            .ok()
                            .flatten()
                            .unwrap_or(0),
                    ),
                )
            }),
            "progress" => books.sort_by(|a, b| b.progress.total_cmp(&a.progress)),
            "title" => books.sort_by(|a, b| a.title.cmp(&b.title)),
            "downloaded" => books.retain(|book| book.cached_chapters > 0),
            "finished" => books.retain(|book| book.finished),
            "notes" => {
                let mut selected = Vec::new();
                for book in books {
                    if self.store.annotation_count(&book.id)? > 0 {
                        selected.push(book);
                    }
                }
                books = selected;
            }
            _ => {}
        }
        Ok(books)
    }

    pub(super) fn render_library(&mut self) -> Result<()> {
        let Some(ui) = self.ui.upgrade() else {
            return Ok(());
        };
        let (books, visible_count) = match (|| -> Result<_> {
            let visible = self.visible_books()?;
            let count = visible.len();
            Ok((self.filtered_book_metadata(visible)?, count))
        })() {
            Ok(books) => books,
            Err(error)
                if error
                    .downcast_ref::<crate::store::BookMetadataLimit>()
                    .is_some()
                    || error
                        .downcast_ref::<crate::store::BookMetadataUnavailable>()
                        .is_some() =>
            {
                ui.global::<LibraryModel>()
                    .set_status(tr("api_error").into());
                self.show_toast("api_error");
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let size = self.library_page_size();
        let count = page_count(books.len(), size);
        self.library_page = self.library_page.min(count - 1);
        let rows = books
            .iter()
            .skip(self.library_page * size)
            .take(size)
            .enumerate()
            .map(|(i, b)| {
                self.book_tile(
                    b,
                    i / self.profile.cover_columns(),
                    i % self.profile.cover_columns(),
                )
            })
            .collect();
        let state = ui.global::<LibraryModel>();
        state.set_books(model(rows));
        state.set_page(self.library_page as i32 + 1);
        state.set_page_count(count as i32);
        state.set_summary(
            i18n::format(
                "shelf_summary",
                &[
                    ("count", visible_count.to_string()),
                    (
                        "sync",
                        if self.sync_time.is_empty() {
                            tr("local_saved")
                        } else {
                            self.sync_time.clone()
                        },
                    ),
                ],
            )
            .into(),
        );
        let task_status = ui.global::<ShellModel>().get_task_label();
        state.set_status(
            if task_status.is_empty() {
                self.library_status.clone()
            } else {
                task_status.to_string()
            }
            .into(),
        );
        state.set_view_mode(self.prefs.view_mode.clone().into());
        state.set_sort(self.library_sort.clone().into());
        state.set_selecting(self.selecting_books);
        state.set_selected_count(self.selected_books.len() as i32);
        state.set_offline(!self.online);
        state.set_busy(self.busy);
        self.schedule_visible_covers();
        Ok(())
    }

    pub(super) fn library_page_size(&self) -> usize {
        if self.prefs.view_mode != "list" {
            return self.profile.cover_page_size();
        }
        let has_status = !self.library_status.is_empty()
            || self
                .ui
                .upgrade()
                .is_some_and(|ui| !ui.global::<ShellModel>().get_task_label().is_empty());
        let (_, height) = self.platform.logical_viewport_size();
        let navigation = if self.profile.is_scribe() { 76 } else { 64 };
        let available = height.saturating_sub(
            self.app_status_bar_height()
                + navigation
                + 52
                + 52
                + 48
                + 48
                + if self.online { 0 } else { 36 }
                + if self.download_notice_visible() {
                    48
                } else if has_status {
                    32
                } else {
                    0
                },
        );
        self.profile
            .list_page_size()
            .min((available / 72).max(1) as usize)
    }

    pub(super) fn sync_shelf(&mut self) {
        if self.demo {
            self.show_toast("demo_notice");
            return;
        }
        if self.force_offline {
            self.show_toast("network_unavailable");
            return;
        }
        if !self.jobs.configured() {
            self.show_toast("credentials_missing");
            return;
        }
        self.shelf_request = self.request();
        self.busy = true;
        self.library_status = tr("syncing");
        self.settle_job_submission(self.jobs.sync_shelf(self.shelf_request));
    }

    pub(super) fn request_catalog(&mut self, book_id: &str) {
        let request = self.request();
        self.catalog_requests.insert(book_id.into(), request);
        self.settle_job_submission(self.jobs.catalog(request, book_id.into()));
    }

    pub(super) fn library_action(&mut self, action: &str, value: &str) -> Result<()> {
        match action {
            "download-status" if self.download_notice_visible() => self.open_download_notice()?,
            "search" => {
                if self.search_purpose != "books" {
                    self.query.clear();
                    self.search_results.clear();
                    self.search_status.clear();
                    self.search_request = self.request();
                }
                self.search_purpose = "books".into();
                self.search_scope = value.into();
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<SearchModel>().set_keyboard_visible(true);
                }
                self.navigate(Route::Search, true)?;
            }
            "sync" => {
                self.sync_shelf();
                if self.current_download.is_none() {
                    if self.reload_pending_downloads(true)? {
                        self.start_next_download()?;
                    }
                }
            }
            "view" => {
                self.prefs.view_mode = if self.prefs.view_mode == "cover" {
                    "list"
                } else {
                    "cover"
                }
                .into();
                self.library_page = 0;
                self.save_preferences()?;
            }
            "sort" => {
                self.library_sort = value.into();
                self.library_page = 0;
            }
            "select" => {
                self.selecting_books = !self.selecting_books;
                self.selected_books.clear();
                if self.selecting_books {
                    self.prefs.view_mode = "cover".into();
                    self.library_page = 0;
                }
            }
            "book" => {
                if self.selecting_books {
                    if !self.selected_books.remove(value) {
                        self.selected_books.insert(value.into());
                    }
                } else {
                    self.open_detail(value)?;
                }
            }
            "page" => {
                let size = self.library_page_size();
                let books = match self.filtered_books() {
                    Ok(books) => books,
                    Err(error)
                        if error
                            .downcast_ref::<crate::store::BookMetadataLimit>()
                            .is_some()
                            || error
                                .downcast_ref::<crate::store::BookMetadataUnavailable>()
                                .is_some() =>
                    {
                        self.show_toast("api_error");
                        return Ok(());
                    }
                    Err(error) => return Err(error),
                };
                self.library_page = move_page(
                    self.library_page,
                    value.parse().unwrap_or(0),
                    page_count(books.len(), size),
                );
            }
            "jump" => self.open_jump("library", self.library_page + 1),
            "login" => {
                self.navigate(Route::Login, true)?;
                self.begin_login();
            }
            "import" => {
                self.scan_imports()?;
            }
            _ => {}
        }
        self.refresh_display(RefreshReason::Content);
        Ok(())
    }

    pub(super) fn load_detail(&mut self, id: &str) -> Result<()> {
        let source = self.store.book_source(id)?;
        let changed = self.detail_id != id;
        self.detail_id = id.into();
        if changed {
            self.review_context = None;
            self.detail_tab = "toc".into();
            self.selecting_chapters = false;
            self.selected_chapters.clear();
            self.reviews.clear();
            self.review_latest = false;
            self.detail_page = self
                .store
                .position(id)?
                .map(|p| p.chapter_index as usize / self.profile.detail_chapter_page_size())
                .unwrap_or(0);
        }
        self.detail_status.clear();
        if source.as_deref() == Some("weread") && !self.force_offline && self.jobs.configured() {
            let request = self.request();
            self.book_requests.insert(id.into(), request);
            self.settle_job_submission(self.jobs.book_info(request, id.into()));
            self.request_catalog(id);
        }
        Ok(())
    }

    pub(super) fn render_detail(&mut self) -> Result<()> {
        let Some(ui) = self.ui.upgrade() else {
            return Ok(());
        };
        let snapshot = match self
            .store
            .book_detail_metadata(&self.detail_id, self.detail_tab == "intro")
        {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return Ok(()),
            Err(error)
                if error
                    .downcast_ref::<crate::store::BookMetadataLimit>()
                    .is_some()
                    || error
                        .downcast_ref::<crate::store::BookMetadataUnavailable>()
                        .is_some() =>
            {
                ui.global::<DetailModel>()
                    .set_status(tr("api_error").into());
                self.show_toast("api_error");
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let book = snapshot.book;
        let introduction = snapshot.introduction;
        let state = ui.global::<DetailModel>();
        let metadata = self.store.chapter_metadata_with_presence(&book.id)?;
        let readable = metadata.iter().any(|entry| entry.body_present);
        let body_indices = metadata
            .iter()
            .filter(|entry| entry.body_present)
            .map(|entry| entry.chapter.index)
            .collect::<BTreeSet<_>>();
        let chapters = metadata
            .into_iter()
            .map(|entry| entry.chapter)
            .collect::<Vec<_>>();
        // One rendering operation reuses this owner for both the complete
        // access flag and visible rows. The next operation takes a fresh one.
        let owner = if book.source == "weread" {
            self.chapter_content_owner()
        } else {
            None
        };
        state.set_access_needs_refresh(
            book.source == "weread"
                && chapters.iter().any(|chapter| {
                    chapter_content_status_for_metadata_owner(&book, chapter, owner.as_deref())
                        != ContentStatus::Full
                        && (body_indices.contains(&chapter.index)
                            || chapter.content_status == ContentStatus::Locked)
                }),
        );
        let annotation_count = self.store.annotation_count(&book.id)?;
        state.set_book(self.book_tile(&book, 0, 0));
        state.set_author(book.author.clone().into());
        state.set_rating(
            if self.profile.is_scribe() && !book.category.trim().is_empty() {
                format!("{} · {}", tr("rating_unavailable"), book.category)
            } else {
                tr("rating_unavailable")
            }
            .into(),
        );
        state.set_summary(
            i18n::format(
                "cache_summary",
                &[
                    ("progress", (book.progress * 100.0).round().to_string()),
                    ("cached", book.cached_chapters.to_string()),
                    ("total", book.total_chapters.to_string()),
                ],
            )
            .into(),
        );
        state.set_chapter_summary(
            i18n::format(
                "chapter_summary",
                &[
                    ("total", book.total_chapters.to_string()),
                    ("cached", book.cached_chapters.to_string()),
                ],
            )
            .into(),
        );
        if let Some(introduction) = introduction {
            state.set_intro(introduction.intro.into());
            state.set_publisher(
                introduction
                    .publisher
                    .unwrap_or_else(|| tr("metadata_unavailable"))
                    .into(),
            );
            state.set_isbn(
                introduction
                    .isbn
                    .unwrap_or_else(|| tr("metadata_unavailable"))
                    .into(),
            );
        } else {
            state.set_intro(Default::default());
            state.set_publisher(Default::default());
            state.set_isbn(Default::default());
        }
        state.set_metadata(
            if book.category.trim().is_empty() {
                tr("metadata_unavailable")
            } else {
                book.category.clone()
            }
            .into(),
        );
        state.set_published(tr("metadata_unavailable").into());
        // The stored upstream update value can describe reading progress or a new
        // chapter. It is not evidence of when book information was refreshed.
        state.set_update_label(tr("book_update_unavailable").into());
        state.set_catalog_label(
            if book.total_chapters > 0 || !chapters.is_empty() {
                known_tab_count(
                    "catalog",
                    (book.total_chapters as usize).max(chapters.len()),
                )
            } else {
                tr("catalog")
            }
            .into(),
        );
        // The source returns a bounded review batch, without a verified total.
        state.set_reviews_label(tr("reviews").into());
        state.set_notes_label(
            known_tab_count(
                "my_notes",
                usize::try_from(annotation_count).unwrap_or(usize::MAX),
            )
            .into(),
        );
        let note_summary = if matches!(self.detail_tab.as_str(), "intro" | "notes") {
            match self.store.annotation_summary_counts(&book.id) {
                Ok(annotation_summary) => i18n::format(
                    "notes_summary",
                    &[
                        ("highlights", annotation_summary.0.to_string()),
                        ("thoughts", annotation_summary.1.to_string()),
                    ],
                ),
                Err(error)
                    if error
                        .downcast_ref::<crate::store::AnnotationResourceLimit>()
                        .is_some()
                        || error
                            .downcast_ref::<crate::store::AnnotationUnavailable>()
                            .is_some() =>
                {
                    tr("api_error")
                }
                Err(error) => return Err(error),
            }
        } else {
            String::new()
        };
        state.set_note_summary(note_summary.clone().into());
        state.set_notes_summary(
            i18n::format("notes_by_chapter", &[("summary", note_summary)]).into(),
        );
        state.set_tab(self.detail_tab.clone().into());
        state.set_status(self.detail_status.clone().into());
        state.set_selecting(self.selecting_chapters && self.detail_tab == "toc");
        state.set_selected_count(self.selected_chapters.len() as i32);
        state.set_selection_summary(
            i18n::format(
                "selected_chapters",
                &[("count", self.selected_chapters.len().to_string())],
            )
            .into(),
        );
        state.set_can_download(
            book.source == "weread" && self.jobs.capabilities().chapter_content && self.online,
        );
        state.set_can_read(
            readable
                || book.source == "weread"
                    && self.jobs.capabilities().chapter_content
                    && self.online,
        );
        let current = self
            .store
            .position(&book.id)?
            .map(|p| p.chapter_index)
            .unwrap_or(0);
        let size = detail_page_size(self.profile, &self.detail_tab);
        let count = if self.detail_tab == "reviews" {
            page_count(self.reviews.len(), size)
        } else if self.detail_tab == "notes" {
            page_count(
                usize::try_from(annotation_count).unwrap_or(usize::MAX),
                size,
            )
        } else {
            page_count(chapters.len(), size)
        };
        self.detail_page = self.detail_page.min(count - 1);
        state.set_page(self.detail_page as i32 + 1);
        state.set_page_count(count as i32);
        state.set_chapters(model(
            chapters
                .iter()
                .skip(self.detail_page * size)
                .take(size)
                .map(|chapter| {
                    let content_status =
                        chapter_content_status_for_metadata_owner(&book, chapter, owner.as_deref());
                    DetailChapter {
                        id: chapter.id.clone().into(),
                        index: chapter.index as i32,
                        title: chapter.title.clone().into(),
                        state: chapter_content_label_for_status(
                            content_status,
                            body_indices.contains(&chapter.index),
                            current == chapter.index,
                        )
                        .into(),
                        cached: content_status == ContentStatus::Full,
                        current: chapter.index == current,
                        selected: self.selected_chapters.contains(&chapter.index),
                    }
                })
                .collect(),
        ));
        let per_page = detail_page_size(self.profile, &self.detail_tab);
        let rows = if self.detail_tab == "notes" {
            let page = self.note_headers_page(&book.id, self.detail_page * per_page, per_page)?;
            detail_note_rows(
                self.note_display_records(&book.id, &page.headers)?,
                &chapters,
            )
        } else {
            self.reviews
                .iter()
                .skip(self.detail_page * per_page)
                .take(per_page)
                .map(|r| DetailReview {
                    id: r.id.clone().into(),
                    author: r.author.clone().into(),
                    meta: {
                        let mut parts = Vec::new();
                        if r.published_at > 0 {
                            parts.push(detail_date(r.published_at));
                        }
                        if r.likes > 0 {
                            parts.push(i18n::format(
                                "review_likes_count",
                                &[("count", r.likes.to_string())],
                            ));
                        }
                        parts.join(" · ").into()
                    },
                    body: r.text.clone().into(),
                    ..Default::default()
                })
                .collect()
        };
        state.set_reviews(model(rows));
        state.set_review_order(if self.review_latest { "latest" } else { "hot" }.into());
        self.schedule_visible_covers();
        Ok(())
    }

    pub(super) fn detail_action(&mut self, action: &str, value: &str) -> Result<()> {
        let id = self.detail_id.clone();
        match action {
            "back" => self.back()?,
            "more" => {
                self.confirm_action("more", "more", "", "close");
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<ShellModel>().set_dialog_kind("more".into());
                }
            }
            "read" => {
                let index = self
                    .store
                    .position(&id)?
                    .map(|p| p.chapter_index)
                    .unwrap_or(0);
                if self.demo && id != "demo-0" {
                    self.show_toast("demo_content_hint");
                } else {
                    self.open_reader(&id, index)?;
                }
            }
            "tab" => {
                if !matches!(value, "intro" | "toc" | "reviews" | "notes") {
                    return Ok(());
                }
                self.review_context = None;
                self.detail_tab = value.into();
                self.detail_page = detail_tab_page(
                    value,
                    self.store
                        .position(&id)?
                        .map(|position| position.chapter_index),
                    self.profile.detail_chapter_page_size(),
                    &mut self.selecting_chapters,
                    &mut self.selected_chapters,
                );
                if value == "reviews" {
                    self.fetch_reviews();
                }
            }
            "notes" => {
                if self.profile.is_scribe() {
                    self.detail_action("tab", "notes")?;
                } else {
                    let index = self
                        .store
                        .position(&id)?
                        .map(|position| position.chapter_index)
                        .unwrap_or(0);
                    self.open_reader(&id, index)?;
                    self.reader_action("toc", "notes")?;
                    self.reader.toc_page = 0;
                }
            }
            "review-order" => {
                let latest = value == "latest";
                if self.review_latest != latest {
                    self.reviews.clear();
                }
                self.review_latest = latest;
                self.detail_page = 0;
                self.fetch_reviews();
            }
            "select" => {
                if self.detail_tab == "toc" {
                    self.selecting_chapters = !self.selecting_chapters;
                    self.selected_chapters.clear();
                }
            }
            "select-page" => {
                if self.detail_tab != "toc" || !self.selecting_chapters {
                    return Ok(());
                }
                let size = self.profile.detail_chapter_page_size();
                for c in self
                    .store
                    .chapter_metadata(&id)?
                    .iter()
                    .skip(self.detail_page * size)
                    .take(size)
                {
                    if !c.cached {
                        self.selected_chapters.insert(c.index);
                    }
                }
            }
            "chapter" => {
                if let Some(chapter) = self
                    .store
                    .chapter_metadata(&id)?
                    .into_iter()
                    .find(|c| c.id == value)
                {
                    if self.selecting_chapters {
                        if !chapter.cached && !self.selected_chapters.remove(&chapter.index) {
                            self.selected_chapters.insert(chapter.index);
                        }
                    } else {
                        self.open_reader(&id, chapter.index)?;
                    }
                }
            }
            "page" => {
                if !matches!(self.detail_tab.as_str(), "toc" | "reviews" | "notes") {
                    return Ok(());
                }
                let count = self
                    .ui
                    .upgrade()
                    .map(|ui| ui.global::<DetailModel>().get_page_count() as usize)
                    .unwrap_or(1);
                self.detail_page = move_page(self.detail_page, value.parse().unwrap_or(0), count);
            }
            "jump"
                if self.detail_tab == "reviews"
                    || self.detail_tab == "toc" && !self.selecting_chapters =>
            {
                self.open_jump("detail", self.detail_page + 1)
            }
            "download" => {
                let indices = self
                    .store
                    .chapter_metadata(&id)?
                    .into_iter()
                    .map(|c| c.index)
                    .collect();
                self.offer_download(vec![(id, indices)])?;
            }
            "refresh-access" => {
                let indices = self
                    .store
                    .chapter_metadata(&id)?
                    .iter()
                    .map(|chapter| chapter.index)
                    .collect();
                self.refresh_content_access(&id, indices)?;
            }
            "download-selected" => {
                self.offer_download(vec![(id, self.selected_chapters.iter().copied().collect())])?
            }
            "note" => {
                let note = self.store.annotation_header_by_id(&id, value)?;
                if let Some(note) = note {
                    self.open_reader(&id, note.chapter_index)?;
                    self.load_reader(&id, note.chapter_index, Some(note.start))?;
                } else if let Some(review) = self.reviews.iter().find(|r| r.id == value) {
                    let body = review.text.clone();
                    self.confirm_action("info", "reviews", "", "close");
                    if let Some(ui) = self.ui.upgrade() {
                        ui.global::<ShellModel>().set_dialog_body(body.into());
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn fetch_reviews(&mut self) {
        // An order/view change also fences an old result when offline prevents
        // starting its replacement. Cookie updates keep this account epoch.
        self.review_request = self.request();
        self.review_context = Some(ReviewRequestContext {
            request: self.review_request,
            book_id: self.detail_id.clone(),
            latest: self.review_latest,
            cache_key: format!(
                "reviews:{}:{}",
                self.detail_id,
                if self.review_latest { "latest" } else { "hot" }
            ),
            account_epoch: self.jobs.account_epoch(),
        });
        if self.demo || self.force_offline || !self.online || !self.jobs.configured() {
            self.detail_status = tr("network_unavailable");
            return;
        }
        self.settle_job_submission(self.jobs.reviews(
            self.review_request,
            self.detail_id.clone(),
            self.review_latest,
        ));
    }

    pub(super) fn enqueue_download(&mut self, book_id: &str, indices: Vec<u32>) -> Result<()> {
        self.enqueue_download_options(book_id, indices, self.prefs.download_images)
    }

    pub(super) fn chapter_content_status(&self, book: &Book, chapter: &Chapter) -> ContentStatus {
        let owner = if book.source == "weread" {
            self.chapter_content_owner()
        } else {
            None
        };
        chapter_content_status_for_owner(book, chapter, owner.as_deref())
    }

    pub(super) fn chapter_metadata_content_status(
        &self,
        book: &BookMetadata,
        chapter: &Chapter,
    ) -> ContentStatus {
        let owner = if book.source == "weread" {
            self.chapter_content_owner()
        } else {
            None
        };
        chapter_content_status_for_metadata_owner(book, chapter, owner.as_deref())
    }

    pub(super) fn chapter_access_content_status(
        &self,
        source: &str,
        entry: &crate::store::ChapterAccessMetadata,
    ) -> ContentStatus {
        let owner = if source == "weread" {
            self.chapter_content_owner()
        } else {
            None
        };
        chapter_content_status_for_access_owner(source, entry, owner.as_deref())
    }

    pub(super) fn chapter_metadata_content_label(
        &self,
        book: &BookMetadata,
        chapter: &Chapter,
        current: bool,
    ) -> String {
        chapter_content_label_for_status(
            self.chapter_metadata_content_status(book, chapter),
            chapter.body.is_some(),
            current,
        )
    }

    /// Borrow this identity throughout one operation; do not cache it across
    /// account replacement, cookie rotation, navigation or subsequent renders.
    pub(super) fn chapter_content_owner(&self) -> Option<String> {
        let session = self.jobs.session();
        #[cfg(test)]
        observe_content_owner_work(|work| {
            // This bridge measures the actual Jobs snapshot used below, not a
            // replacement getter or an estimate multiplied by directory rows.
            work.snapshots += 1;
            work.snapshot_string_bytes += session.as_ref().map_or(0, session_string_bytes);
        });
        session.as_ref().and_then(download_account_key)
    }

    pub(super) fn synchronize_content_owner(&self) -> Result<()> {
        let owner = self.chapter_content_owner();
        self.store.invalidate_content_ownership(owner.as_deref())
    }

    pub(super) fn chapter_content_label(
        &self,
        book: &Book,
        chapter: &Chapter,
        current: bool,
    ) -> String {
        self.chapter_content_label_with_presence(book, chapter, chapter.body.is_some(), current)
    }

    pub(super) fn chapter_content_label_with_presence(
        &self,
        book: &Book,
        chapter: &Chapter,
        body_present: bool,
        current: bool,
    ) -> String {
        chapter_content_label_for_status(
            self.chapter_content_status(book, chapter),
            body_present,
            current,
        )
    }

    /// Explicitly resume permission-blocked work after the user updates access elsewhere.
    pub(super) fn refresh_content_access(
        &mut self,
        book_id: &str,
        indices: Vec<u32>,
    ) -> Result<()> {
        if self.force_offline {
            self.show_toast("network_unavailable");
            return Ok(());
        }
        if self.demo || !self.jobs.capabilities().chapter_content {
            self.show_toast("web_login_required");
            return Ok(());
        }
        let Some(owner) = self.chapter_content_owner() else {
            self.show_toast("web_login_required");
            return Ok(());
        };
        let selected: BTreeSet<_> = indices.iter().copied().collect();
        if self.current_download.as_ref().is_some_and(|task| {
            task.book_id == book_id
                && task.account_key == owner
                && task.chapters.iter().any(|index| selected.contains(index))
        }) {
            self.show_toast("content_refreshing");
            return Ok(());
        }
        if let Some(task) = self.download_notice_task_for_book(book_id)? {
            if task.account_key == owner
                && self
                    .store
                    .download_chapter_states(&task.id)?
                    .iter()
                    .any(|chapter| {
                        chapter.status == DownloadChapterStatus::Blocked
                            && selected.contains(&chapter.chapter_index)
                    })
            {
                self.online = true;
                self.queue_explicit_chapter_retry(task)?;
                self.show_toast("content_refreshing");
                return Ok(());
            }
        }
        let pending = match self.store.pending_tasks_for_owner(&owner) {
            Ok(tasks) => tasks,
            Err(error) if Self::inventory_capacity(&error) => {
                self.download_inventory_unavailable = true;
                self.show_toast("api_error");
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        self.download_inventory_unavailable = false;
        self.online = true;
        if let Some(task) = pending.into_iter().find(|task| {
            task.book_id == book_id
                && task.account_key == owner
                && content_download_blocked(task.error.as_deref())
                && task
                    .chapters
                    .get(task.next_index)
                    .is_some_and(|index| selected.contains(index))
        }) {
            self.queue_explicit_chapter_retry(task)?;
            self.show_toast("content_refreshing");
        } else {
            self.enqueue_download_options(book_id, indices, self.prefs.download_images)?;
        }
        Ok(())
    }

    pub(super) fn enqueue_download_options(
        &mut self,
        book_id: &str,
        indices: Vec<u32>,
        include_images: bool,
    ) -> Result<()> {
        if self.demo || !self.jobs.capabilities().chapter_content {
            self.show_toast("web_login_required");
            return Ok(());
        }
        let Some(account_key) = self.chapter_content_owner() else {
            self.show_toast("web_login_required");
            return Ok(());
        };
        let chapters = self.store.chapter_metadata(book_id)?;
        let Some(source) = self.store.book_source(book_id)? else {
            return Ok(());
        };
        if chapters.is_empty() {
            self.pending_catalog_downloads.insert(book_id.into());
            self.pending_catalog_images
                .insert(book_id.into(), include_images);
            self.request_catalog(book_id);
            return Ok(());
        }
        let requested: BTreeSet<_> = indices.into_iter().collect();
        if self.reuse_download_notice(book_id, &requested, include_images)? {
            return Ok(());
        }
        let mut scheduled = BTreeSet::new();
        if let Some(task) = self.current_download.as_ref().filter(|task| {
            task.account_key == account_key
                && task.book_id == book_id
                && (!include_images || task.include_images)
                && !content_download_blocked(task.error.as_deref())
        }) {
            scheduled.extend(task.chapters.iter().copied());
        }
        for task in self.downloads.iter().filter(|task| {
            task.account_key == account_key
                && task.book_id == book_id
                && task.status.is_pending()
                && (!include_images || task.include_images)
                && !content_download_blocked(task.error.as_deref())
        }) {
            scheduled.extend(task.chapters.iter().copied());
        }
        let candidates = chapters
            .iter()
            .filter(|chapter| {
                requested.contains(&chapter.index) && !scheduled.contains(&chapter.index)
            })
            .collect::<Vec<_>>();
        let image_uids = if include_images {
            candidates
                .iter()
                .filter(|chapter| {
                    chapter_content_status_for_source(&source, chapter, Some(&account_key))
                        == ContentStatus::Full
                })
                .map(|chapter| chapter.id.as_str())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        // One operation verifies the aligned Full candidates together. Unknown
        // permissions never use a completeness result as download authority.
        let complete = self.complete_download_images(book_id, &image_uids)?;
        anyhow::ensure!(
            complete.len() == image_uids.len(),
            "unaligned bulk image completeness"
        );
        let mut completed = complete.into_iter();
        let mut remaining = Vec::new();
        for chapter in candidates {
            let full = chapter_content_status_for_source(&source, chapter, Some(&account_key))
                == ContentStatus::Full;
            let needed = if !full {
                true
            } else if include_images {
                !completed
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("missing bulk completeness result"))?
            } else {
                !chapter.cached
            };
            if needed {
                remaining.push(chapter.index);
            }
        }
        if remaining.is_empty() {
            self.show_toast("already_cached");
            return Ok(());
        }
        if self
            .store
            .setting::<bool>(&format!("shelf.override:{book_id}"))?
            .is_none()
        {
            self.store.set_on_shelf(book_id, true)?;
            self.store
                .set_setting(&format!("shelf.override:{book_id}"), &true)?;
        }
        let task = DownloadTask {
            id: unique_id("download"),
            book_id: book_id.into(),
            account_key,
            chapters: remaining,
            status: DownloadStatus::Pending,
            include_images,
            updated_at: Utc::now().timestamp(),
            ..Default::default()
        };
        self.store.save_task(&task)?;
        self.downloads.push_back(task);
        self.start_next_download()
    }

    pub(super) fn start_next_download(&mut self) -> Result<()> {
        self.start_download(false)
    }

    fn start_download(&mut self, retry: bool) -> Result<()> {
        if self.current_download.is_some()
            || self.download_inventory_unavailable
            || self.force_offline
            || (!self.online && !retry)
            || !self.jobs.capabilities().chapter_content
        {
            return Ok(());
        }
        let Some(owner) = self.chapter_content_owner() else {
            return Ok(());
        };
        // Chapter deadlines belong to the scheduler. Starting a task never
        // withholds another ready chapter because the first gap is waiting.
        let queued = self.downloads.len();
        for _ in 0..queued {
            let Some(mut task) = self.downloads.pop_front() else {
                break;
            };
            if !task.status.is_pending()
                || task.account_key != owner
                || content_download_blocked(task.error.as_deref())
            {
                continue;
            }
            let original = task.clone();
            let prepared = (|| -> Result<Option<(DownloadTask, crate::jobs::DownloadPlan)>> {
                let chapters = self.store.chapter_metadata(&task.book_id)?;
                let Some(_) = self.store.book_source(&task.book_id)? else {
                    return Ok(None);
                };
                if chapters.is_empty() {
                    task.status = DownloadStatus::Failed;
                    task.error = Some("catalog_missing".into());
                    self.store.save_task(&task)?;
                    return Ok(None);
                }
                // Admit the body-free catalog and selection before durable
                // recovery work, then inspect selected cached images once.
                if let Err(error) = crate::jobs::DownloadPlan::from_headers(&task, &chapters) {
                    if matches!(error, source::SourceError::ResponseTooLarge) {
                        return Err(error.into());
                    }
                    task.status = DownloadStatus::Failed;
                    task.error = Some(error.to_string());
                    task.updated_at = Utc::now().timestamp();
                    self.store.save_task(&task)?;
                    self.error_message(&error);
                    return Ok(None);
                }
                let updated_at = Utc::now().timestamp();
                let states = self.reconcile_download_chapters(&task, &chapters, updated_at)?;
                task = self
                    .store
                    .task(&task.id)?
                    .ok_or_else(|| anyhow::anyhow!("A prepared download task disappeared"))?;
                let plan = match crate::jobs::DownloadPlan::from_headers_with_states(
                    &task, &chapters, &states,
                ) {
                    Ok(plan) => plan,
                    Err(source::SourceError::ResponseTooLarge) => {
                        return Err(source::SourceError::ResponseTooLarge.into());
                    }
                    Err(error) => {
                        task.status = DownloadStatus::Failed;
                        task.error = Some(error.to_string());
                        task.updated_at = Utc::now().timestamp();
                        self.store.save_task(&task)?;
                        self.error_message(&error);
                        return Ok(None);
                    }
                };
                let previous = task.clone();
                task.status = DownloadStatus::Running;
                task.error = None;
                task.updated_at = Utc::now().timestamp();
                anyhow::ensure!(
                    self.store.save_task_if_current(&previous, &task)?,
                    "A download task changed during preparation"
                );
                Ok(Some((task, plan)))
            })();
            let (task, plan) = match prepared {
                Ok(Some(prepared)) => prepared,
                Ok(None) => continue,
                Err(error) => {
                    // The original durable cursor and queue item still own this
                    // intent. No failed validation can discard the popped task.
                    self.downloads
                        .push_front(self.store.task(&original.id)?.unwrap_or(original));
                    if Self::inventory_capacity(&error) || Self::view_capacity(&error) {
                        self.download_inventory_unavailable = true;
                    }
                    return Err(error);
                }
            };
            let notice_reflow = !self.download_notice_visible();
            let reopen_download_dialog = self.clear_download_notice_for_start(&task.id)?;
            let progress = self.store.download_chapter_progress(&task.id)?;
            self.download_feedback(
                task.completed_count,
                task.chapters.len(),
                progress.retry_wait_count,
                progress.blocked_count,
            );
            self.download_request = self.request();
            let cancellation = self.jobs.download_with_concurrency(
                self.download_request,
                task.clone(),
                plan,
                self.prefs.download_concurrency,
            );
            self.current_download = Some(task);
            self.settle_job_submission(cancellation);
            self.start_task_wake_guard();
            if reopen_download_dialog {
                self.reopen_running_download_dialog();
            }
            if notice_reflow && self.route == Route::Library && self.prefs.view_mode == "list" {
                self.render_library()?;
            }
            break;
        }
        self.render_download_notice();
        Ok(())
    }

    pub(super) fn schedule_download_retry(&mut self) {
        self.download_retry_at = Instant::now() + Duration::from_secs(60);
    }

    pub(super) fn retry_pending_downloads(&mut self) -> Result<()> {
        if self.demo
            || self.download_inventory_unavailable
            || self.force_offline
            || self.route == Route::Login
            || self.current_download.is_some()
            || Instant::now() < self.download_retry_at
        {
            return Ok(());
        }
        let Some(owner) = self.chapter_content_owner() else {
            return Ok(());
        };
        let pending = match self.store.pending_tasks_for_owner(&owner) {
            Ok(tasks) => tasks,
            Err(error) if Self::inventory_capacity(&error) => {
                self.download_inventory_unavailable = true;
                self.show_toast("api_error");
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        self.schedule_download_retry();
        let eligible = |task: &DownloadTask| {
            task.account_key == owner
                && (transient_download_error(task.error.as_deref()) || task.error.is_none())
        };
        if !pending.iter().any(eligible) {
            return Ok(());
        }
        let mut queued = self
            .downloads
            .iter()
            .map(|task| task.id.clone())
            .collect::<BTreeSet<_>>();
        for task in pending {
            if eligible(&task) && queued.insert(task.id.clone()) {
                self.downloads.push_back(task);
            }
        }
        self.start_download(true)
    }

    pub(super) fn reload_pending_downloads(&mut self, explicit: bool) -> Result<bool> {
        let pending = if let Some(owner) = self.chapter_content_owner() {
            self.store.pending_tasks_for_owner(&owner)
        } else {
            Ok(Vec::new())
        };
        let mut pending: VecDeque<DownloadTask> = match pending {
            Ok(tasks) => tasks.into_iter().collect(),
            Err(error) if Self::inventory_capacity(&error) => {
                self.download_inventory_unavailable = true;
                self.show_toast("api_error");
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        if let Some(current) = &self.current_download {
            pending.retain(|task| task.id != current.id);
        }
        self.downloads = pending;
        if explicit {
            self.download_inventory_unavailable = false;
        }
        Ok(!self.download_inventory_unavailable)
    }

    fn offer_download(&mut self, items: Vec<(String, Vec<u32>)>) -> Result<()> {
        if !self.online {
            self.show_toast("network_unavailable");
            return Ok(());
        }
        let mut total = 0usize;
        let mut cached = 0usize;
        let mut selected = 0usize;
        for (book_id, indices) in &items {
            let chapters = self.store.chapter_metadata(book_id)?;
            total += chapters.len();
            cached += chapters.iter().filter(|chapter| chapter.cached).count();
            let unique: BTreeSet<_> = indices.iter().copied().collect();
            selected += chapters
                .iter()
                .filter(|chapter| unique.contains(&chapter.index))
                .count();
        }
        self.pending_downloads = items;
        self.confirm_action("download", "download_all", "", "begin_download");
        if let Some(ui) = self.ui.upgrade() {
            let shell = ui.global::<ShellModel>();
            shell.set_dialog_kind("download".into());
            shell.set_dialog_images(self.prefs.download_images);
            shell.set_dialog_summary(
                i18n::format(
                    "download_catalog_summary",
                    &[
                        ("total", total.to_string()),
                        ("cached", cached.to_string()),
                        ("selected", selected.to_string()),
                    ],
                )
                .into(),
            );
            shell.set_dialog_size("".into());
            shell.set_dialog_chapter("".into());
        }
        Ok(())
    }

    pub(super) fn confirm_action(&mut self, action: &str, title: &str, body: &str, primary: &str) {
        self.dialog_action = action.into();
        if let Some(ui) = self.ui.upgrade() {
            let shell = ui.global::<ShellModel>();
            shell.set_dialog_kind("confirm".into());
            shell.set_dialog_title(tr(title).into());
            shell.set_dialog_body(
                if body.is_empty() {
                    String::new()
                } else {
                    tr(body)
                }
                .into(),
            );
            shell.set_dialog_primary(tr(primary).into());
            shell.set_dialog_secondary(
                if action == "info" || action == "more" {
                    String::new()
                } else {
                    tr("cancel")
                }
                .into(),
            );
            shell.set_dialog_destructive(matches!(
                action,
                "logout" | "clear-cache" | "remove-selected" | "remove-book" | "clear-book"
            ));
        }
    }

    pub(super) fn open_jump(&mut self, target: &str, page: usize) {
        self.jump_target = target.into();
        self.confirm_action("jump", "jump_page", "", "confirm");
        if let Some(ui) = self.ui.upgrade() {
            let shell = ui.global::<ShellModel>();
            shell.set_dialog_kind("jump".into());
            shell.set_dialog_input(page.to_string().into());
        }
    }
    fn close_dialog(&mut self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<ShellModel>().set_dialog_kind("".into());
        }
        self.dialog_action.clear();
    }

    pub(super) fn shell_action(&mut self, action: &str, value: &str) -> Result<()> {
        match action {
            "tab" => {
                let route = match value {
                    "mp" => Route::Publications,
                    "stats" => Route::Statistics,
                    "settings" => Route::Settings,
                    _ => Route::Library,
                };
                self.history.clear();
                self.navigate(route, false)?;
            }
            "remove-selected" => self.confirm_action(
                "remove-selected",
                "delete_confirm",
                "keep_downloads",
                "remove",
            ),
            "download-selected" => {
                let mut items = vec![];
                for id in self.selected_books.clone() {
                    let chapters = self.store.chapter_metadata(&id)?;
                    if chapters.is_empty() {
                        self.pending_catalog_downloads.insert(id.clone());
                        self.pending_catalog_images
                            .insert(id.clone(), self.prefs.download_images);
                        self.request_catalog(&id);
                    } else {
                        items.push((id, chapters.into_iter().map(|c| c.index).collect()));
                    }
                }
                if !items.is_empty() {
                    self.offer_download(items)?;
                }
            }
            "dialog-outside" => {
                if let Some(ui) = self.ui.upgrade() {
                    if !ui.global::<ShellModel>().get_dialog_destructive() {
                        self.close_dialog();
                    }
                }
            }
            "dialog-secondary" => {
                if matches!(
                    self.dialog_action.as_str(),
                    "download-wait" | "download-continue"
                ) {
                    self.cancel_download_notice()?;
                }
                if matches!(
                    self.dialog_action.as_str(),
                    "downloading" | "download-retry-active"
                ) {
                    if let Some(task) = self.current_download.take() {
                        self.settle_job_submission(self.jobs.cancel_download(&task.id));
                        let mut task = task;
                        task.status = DownloadStatus::Cancelled;
                        task.error = None;
                        task.updated_at = Utc::now().timestamp();
                        self.store.save_task(&task)?;
                        self.retire_task_wake_guard();
                        self.clear_download_notice(&task.id)?;
                        self.start_next_download()?;
                    }
                }
                self.close_dialog();
            }
            "dialog-images" => {
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<ShellModel>().set_dialog_images(value == "true");
                }
            }
            "dialog-primary" => {
                let intent = self.dialog_action.clone();
                self.close_dialog();
                match intent.as_str() {
                    "download-retry-active" => self.retry_active_download_chapters()?,
                    "download-continue" => self.continue_download_notice()?,
                    "download-wait" => {}
                    value if value.starts_with("delete-note:") => {
                        self.store
                            .delete_annotation(value.trim_start_matches("delete-note:"))?;
                        self.reader.selection = None;
                    }
                    "exit" => {
                        self.request_exit()?;
                    }
                    "logout" => {
                        let books = match self.store.book_metadata() {
                            Ok(books) => books,
                            Err(error)
                                if error
                                    .downcast_ref::<crate::store::BookMetadataLimit>()
                                    .is_some()
                                    || error
                                        .downcast_ref::<crate::store::BookMetadataUnavailable>()
                                        .is_some() =>
                            {
                                self.show_toast("api_error");
                                return Ok(());
                            }
                            Err(error) => return Err(error),
                        };
                        self.finish_reading()?;
                        self.invalidate_account_work()?;
                        self.jobs.clear_session();
                        self.discard_cancelled_job_events();
                        self.synchronize_content_owner()?;
                        self.persisted_session = None;
                        self.session_file.clear()?;
                        self.store.delete_setting("mp.accounts")?;
                        self.store.delete_setting("mp.articles")?;
                        for b in books.into_iter().filter(|book| book.on_shelf) {
                            if b.source == "weread"
                                && self
                                    .store
                                    .setting::<bool>(&format!("shelf.override:{}", b.id))?
                                    != Some(true)
                            {
                                self.store.remove_from_shelf(&b.id)?;
                            }
                        }
                        self.navigate(Route::Login, false)?;
                        self.begin_login();
                    }
                    "remove-selected" => {
                        for id in &self.selected_books {
                            self.store
                                .set_setting(&format!("shelf.override:{id}"), &false)?;
                            self.store.remove_from_shelf(id)?;
                        }
                        self.selected_books.clear();
                        self.selecting_books = false;
                    }
                    "remove-book" => {
                        self.store
                            .set_setting(&format!("shelf.override:{}", self.detail_id), &false)?;
                        self.store.remove_from_shelf(&self.detail_id)?;
                        self.back()?;
                    }
                    "clear-book" => {
                        let id = self.detail_id.clone();
                        let book = self.store.book_metadata_by_id(&id)?;
                        if book.as_ref().is_some_and(|b| b.source == "weread") {
                            self.book_assets.clear_book(&self.store, &id)?;
                            self.store.clear_book_cache(&id)?;
                        } else {
                            self.show_toast("local_cache_preserved");
                        }
                        if let Some(book) = book {
                            if let Some(key) = sleep_screen::thumbnail_key(&id, &book.cover_url) {
                                self.cache.remove(&key)?;
                            }
                            if let Some(key) = sleep_screen::cover_key(&id, &book.cover_url) {
                                self.cache.remove(&key)?;
                            }
                        }
                    }
                    "clear-cache" => {
                        let books = match self.store.book_metadata() {
                            Ok(books) => books,
                            Err(error)
                                if error
                                    .downcast_ref::<crate::store::BookMetadataLimit>()
                                    .is_some()
                                    || error
                                        .downcast_ref::<crate::store::BookMetadataUnavailable>()
                                        .is_some() =>
                            {
                                self.show_toast("api_error");
                                return Ok(());
                            }
                            Err(error) => return Err(error),
                        };
                        for book in books {
                            if matches!(book.source.as_str(), "weread" | "article") {
                                self.book_assets.clear_book(&self.store, &book.id)?;
                                self.store.clear_book_cache(&book.id)?;
                                if let Some(id) = book.id.strip_prefix("article:") {
                                    self.store.delete_setting(&format!("article.html:{id}"))?;
                                }
                            }
                        }
                        self.cache.clear_unpinned()?;
                        self.show_toast("local_saved");
                    }
                    "download" => {
                        let images = self
                            .ui
                            .upgrade()
                            .is_some_and(|ui| ui.global::<ShellModel>().get_dialog_images());
                        let items = std::mem::take(&mut self.pending_downloads);
                        for (id, indices) in items {
                            self.enqueue_download_options(&id, indices, images)?;
                        }
                        self.selecting_chapters = false;
                        self.selected_chapters.clear();
                        if self.current_download.is_some() {
                            self.dialog_action = "downloading".into();
                            if let Some(ui) = self.ui.upgrade() {
                                let shell = ui.global::<ShellModel>();
                                shell.set_dialog_kind("downloading".into());
                                shell.set_dialog_title(tr("download").into());
                                shell.set_dialog_body("".into());
                                shell.set_dialog_primary(tr("background_download").into());
                                shell.set_dialog_secondary(tr("cancel_download").into());
                            }
                        }
                    }
                    "jump" => {
                        let page = self
                            .ui
                            .upgrade()
                            .map(|ui| {
                                ui.global::<ShellModel>()
                                    .get_dialog_input()
                                    .as_str()
                                    .parse::<usize>()
                                    .unwrap_or(1)
                                    .saturating_sub(1)
                            })
                            .unwrap_or(0);
                        match self.jump_target.as_str() {
                            "library" => self.library_page = page,
                            "detail" => self.detail_page = page,
                            "search" => self.search_page = page,
                            "mp" => self.mp_page = page,
                            "settings" => self.settings_page = page,
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            "more-refresh" => {
                self.close_dialog();
                let id = self.detail_id.clone();
                self.load_detail(&id)?;
            }
            "more-import" => {
                self.close_dialog();
                self.scan_imports()?;
            }
            "more-clear" => {
                self.confirm_action("clear-book", "clear_book_cache", "clear_book_hint", "clear")
            }
            "more-remove" => {
                self.confirm_action("remove-book", "remove", "keep_downloads", "remove")
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn enter_demo(&mut self) -> Result<()> {
        if self.demo {
            return self.navigate(Route::Library, false);
        }
        self.invalidate_account_work()?;
        self.jobs.clear_session();
        self.discard_cancelled_job_events();
        self.persisted_session = None;
        self.demo = true;
        self.data_dir = self.data_dir.join("demo");
        std::fs::create_dir_all(&self.data_dir)?;
        self.store = Store::open(self.data_dir.join("library.sqlite"))?;
        crate::demo::seed(&self.store)?;
        self.cache = FileCache::open(self.data_dir.join("covers"), 96 * 1024 * 1024)?;
        self.book_assets = crate::book_assets::BookAssetStore::open(
            self.data_dir.join("images"),
            512 * 1024 * 1024,
        )?;
        self.book_assets.recover(&self.store)?;
        crate::demo::seed_images(&self.store, &self.book_assets)?;
        self.session_file = SessionFile::new(&self.data_dir);
        self.mp_accounts = self.store.setting("mp.accounts")?.unwrap_or_default();
        self.mp_articles = self.store.setting("mp.articles")?.unwrap_or_default();
        self.sync_time.clear();
        self.navigate(Route::Library, false)
    }

    pub(super) fn scan_imports(&mut self) -> Result<()> {
        let directory = self.data_dir.join("imports");
        std::fs::create_dir_all(&directory)?;
        let mut imported = 0;
        let mut failed = false;
        for entry in std::fs::read_dir(&directory)? {
            let path = entry?.path();
            if !path.is_file() {
                continue;
            }
            let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !extension.eq_ignore_ascii_case("txt") && !extension.eq_ignore_ascii_case("epub") {
                continue;
            }
            match crate::importer::import_file(&path) {
                Ok(mut result) => {
                    if let Some(existing) = self.store.book_reading_state(&result.book.id)? {
                        result.book.progress = existing.progress;
                        result.book.finished = existing.finished;
                    }
                    self.store.upsert_book(&result.book)?;
                    self.store.upsert_chapters(&result.chapters)?;
                    imported += 1;
                }
                Err(_) => failed = true,
            }
        }
        if imported > 0 {
            self.show_message(i18n::format(
                "imported_count",
                &[("count", imported.to_string())],
            ));
        } else if failed {
            self.show_toast("import_failed");
        } else {
            self.confirm_action("info", "import_book", "import_hint", "close");
        }
        Ok(())
    }
}

#[cfg(test)]
mod selected_metadata_tests {
    use super::super::underline_projection_tests::isolated;
    use super::*;
    use crate::store::{BookMetadataLimit, BookMetadataLimits, with_book_metadata_test_limits};
    use slint::platform::{
        Platform as UiPlatform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    };
    use std::io::{Cursor, Write};

    struct MetadataUi;
    impl UiPlatform for MetadataUi {
        fn create_window_adapter(
            &self,
        ) -> std::result::Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
        }
    }

    struct Fixture {
        app: Option<Rc<RefCell<App>>>,
        ui: Option<AppWindow>,
        runtime: Option<tokio::runtime::Runtime>,
        _directory: tempfile::TempDir,
        book: Book,
        task: DownloadTask,
        note: Annotation,
    }

    impl Fixture {
        fn new() -> Result<Self> {
            slint::platform::set_platform(Box::new(MetadataUi))?;
            let directory = tempfile::tempdir()?;
            let imports = directory.path().join("imports");
            std::fs::create_dir_all(&imports)?;
            let intro = format!("Keep supplied introduction:{}", "x".repeat(4096));
            let package = format!(
                "<package version='2.0'><metadata><title>Selected metadata fixture</title><description>{intro}</description></metadata><manifest><item id='chapter' href='chapter.xhtml' media-type='application/xhtml+xml'/></manifest><spine><itemref idref='chapter'/></spine></package>"
            );
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, bytes) in [
                ("mimetype", b"application/epub+zip".as_slice()),
                ("META-INF/container.xml", b"<container><rootfiles><rootfile full-path='book.opf'/></rootfiles></container>".as_slice()),
                ("book.opf", package.as_bytes()),
                ("chapter.xhtml", b"<html><body><p>Selected retained reading body.</p></body></html>".as_slice()),
            ] { writer.start_file(name, options)?; writer.write_all(bytes)?; }
            let bytes = writer.finish()?.into_inner();
            assert!(bytes.len() < 8192);
            let path = imports.join("fixture.epub");
            std::fs::write(&path, bytes)?;
            let mut imported = crate::importer::import_file(&path)?;
            assert_eq!(imported.book.intro, intro);
            imported.book.progress = 0.6;
            imported.book.finished = true;
            let note = Annotation {
                id: "selected-library-note".into(),
                book_id: imported.book.id.clone(),
                start: 0,
                end: 3,
                quote: "Sel".into(),
                note: "Keep selected library note".into(),
                created_at: 10,
                ..Default::default()
            };
            let account = source::AuthSession {
                api_key: "synthetic-selected-library-key".into(),
                user_vid: "synthetic-selected-library-account".into(),
                cookies: BTreeMap::from([
                    ("wr_vid".into(), "synthetic-selected-library-account".into()),
                    (
                        "wr_skey".into(),
                        "synthetic-selected-library-session".into(),
                    ),
                ]),
                ..Default::default()
            };
            let task = DownloadTask {
                id: "selected-library-task".into(),
                book_id: imported.book.id.clone(),
                account_key: download_account_key(&account).unwrap(),
                chapters: vec![0],
                status: DownloadStatus::Pending,
                updated_at: 10,
                ..Default::default()
            };
            SessionFile::new(directory.path()).save(&account)?;
            let book = {
                let store = Store::open(directory.path().join("library.sqlite"))?;
                store.upsert_book(&imported.book)?;
                store.upsert_chapters(&imported.chapters)?;
                store.save_annotation(&note)?;
                store.save_task(&task)?;
                let mut preferences = AppPreferences::for_device(false);
                preferences.pull_on_open = false;
                preferences.push_on_close = false;
                preferences.report_time = false;
                preferences.popular = false;
                store.set_setting("preferences", &preferences)?;
                store.book(&imported.book.id)?.unwrap()
            };
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let platform = Arc::new(Platform::new(DeviceProfile::Paperwhite6, false)?);
            let ui = AppWindow::new()?;
            crate::platform::configure_window(ui.window(), &platform)?;
            let app = App::attach(
                &ui,
                AppOptions {
                    data_dir: directory.path().to_path_buf(),
                    profile: DeviceProfile::Paperwhite6,
                    demo: false,
                    offline: true,
                    initial_screen: "library".into(),
                    fonts: vec![FontRecord {
                        id: "song".into(),
                        label: "Selected metadata fixture font".into(),
                        family: "sans-serif".into(),
                        metrics: TextMetrics::default(),
                        weights: BTreeMap::new(),
                        available: true,
                    }],
                },
                platform,
                &runtime,
            )?;
            {
                let mut state = app.borrow_mut();
                state.load_local_reader(&book.id, 0, Some(0))?;
                state.reader.selection = Some((0, 3));
                assert!(state.reader.draft.set_query("Keep selected draft"));
            }
            Ok(Self {
                app: Some(app),
                ui: Some(ui),
                runtime: Some(runtime),
                _directory: directory,
                book,
                task,
                note,
            })
        }

        fn state(&self) -> std::cell::RefMut<'_, App> {
            self.app.as_ref().unwrap().borrow_mut()
        }
        fn finish(mut self) -> Result<()> {
            let app = self.app.take().unwrap();
            app.borrow_mut().terminal_shutdown()?;
            let weak = Rc::downgrade(&app);
            drop(app);
            assert!(weak.upgrade().is_none());
            let ui = self.ui.take().unwrap();
            let weak = ui.as_weak();
            ui.hide()?;
            drop(ui);
            slint::platform::update_timers_and_animations();
            assert!(weak.upgrade().is_none());
            self.runtime
                .take()
                .unwrap()
                .shutdown_timeout(Duration::from_secs(1));
            Ok(())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(app) = self.app.take() {
                let _ = app.borrow_mut().terminal_shutdown();
            }
            if let Some(ui) = self.ui.take() {
                let _ = ui.hide();
            }
            if let Some(runtime) = self.runtime.take() {
                runtime.shutdown_timeout(Duration::from_secs(1));
            }
        }
    }

    fn source_only_detail() -> Result<()> {
        let fixture = Fixture::new()?;
        {
            let mut state = fixture.state();
            state.store.reset_book_read_stats();
            state.store.reset_selected_metadata_read_stats();
            let body_reads = state.store.chapter_body_read_count();
            state.load_detail(&fixture.book.id)?;
            assert_eq!(state.store.book_read_stats().intro_copies, 0);
            assert_eq!(state.store.chapter_body_read_count(), body_reads);
            assert_eq!(
                (
                    state.store.selected_metadata_read_stats().book_text_rows,
                    state.store.selected_metadata_read_stats().book_text_bytes
                ),
                (1, fixture.book.source.len())
            );
            state.detail_id = "held-detail".into();
            state.detail_tab = "intro".into();
            state.detail_page = 2;
            state.selected_chapters = BTreeSet::from([0]);
            state.selecting_chapters = true;
            let epoch = state.jobs.account_epoch();
            let body = state.reader.body.clone();
            let preferences = serde_json::to_value(&state.prefs)?;
            state.store.reset_book_read_stats();
            state.store.reset_selected_metadata_read_stats();
            with_book_metadata_test_limits(
                BookMetadataLimits {
                    bytes: 1,
                    ..Default::default()
                },
                || -> Result<()> {
                    let error = state.load_detail(&fixture.book.id).unwrap_err();
                    assert!(error.downcast_ref::<BookMetadataLimit>().is_some());
                    assert_eq!(
                        (
                            state.detail_id.as_str(),
                            state.detail_tab.as_str(),
                            state.detail_page
                        ),
                        ("held-detail", "intro", 2)
                    );
                    assert_eq!(state.selected_chapters, BTreeSet::from([0]));
                    assert!(state.selecting_chapters);
                    assert_eq!(state.store.book_read_stats().intro_copies, 0);
                    assert_eq!(state.store.chapter_body_read_count(), body_reads);
                    assert_eq!(state.store.selected_metadata_read_stats().book_text_rows, 0);
                    Ok(())
                },
            )?;
            assert_eq!(state.reader.body, body);
            assert_eq!(state.reader.selection, Some((0, 3)));
            assert_eq!(state.reader.draft.query, "Keep selected draft");
            assert_eq!(state.jobs.account_epoch(), epoch);
            assert_eq!(serde_json::to_value(&state.prefs)?, preferences);
            assert_eq!(
                state.store.book(&fixture.book.id)?.unwrap().intro,
                fixture.book.intro
            );
            assert_eq!(
                state.store.annotations(&fixture.book.id)?,
                vec![fixture.note.clone()]
            );
            assert_eq!(
                state.store.task(&fixture.task.id)?,
                Some(fixture.task.clone())
            );
        }
        fixture.finish()
    }

    fn reimport_state() -> Result<()> {
        let fixture = Fixture::new()?;
        {
            let mut state = fixture.state();
            let body = state.reader.body.clone();
            let body_reads = state.store.chapter_body_read_count();
            state.store.reset_book_read_stats();
            state.store.reset_selected_metadata_read_stats();
            state.scan_imports()?;
            assert_eq!(state.store.book_read_stats().intro_copies, 0);
            assert_eq!(state.store.chapter_body_read_count(), body_reads);
            assert_eq!(state.store.selected_metadata_read_stats().book_text_rows, 0);
            assert_eq!(
                state.store.selected_metadata_read_stats().book_scalar_reads,
                2
            );
            let book = state.store.book(&fixture.book.id)?.unwrap();
            assert_eq!(
                (book.progress, book.finished, book.intro.as_str()),
                (0.6, true, fixture.book.intro.as_str())
            );
            assert_eq!(state.reader.body, body);
            assert_eq!(state.reader.selection, Some((0, 3)));
            assert_eq!(state.reader.draft.query, "Keep selected draft");
            assert_eq!(
                state.store.annotations(&fixture.book.id)?,
                vec![fixture.note.clone()]
            );
            assert_eq!(
                state.store.task(&fixture.task.id)?,
                Some(fixture.task.clone())
            );
        }
        fixture.finish()
    }

    fn download_source() -> Result<()> {
        let fixture = Fixture::new()?;
        {
            let mut state = fixture.state();
            assert!(state.force_offline && state.jobs.capabilities().chapter_content);
            let body_reads = state.store.chapter_body_read_count();
            state.store.reset_book_read_stats();
            state.store.reset_selected_metadata_read_stats();
            state.enqueue_download_options(&fixture.book.id, vec![0], false)?;
            assert_eq!(state.store.book_read_stats().intro_copies, 0);
            assert_eq!(state.store.chapter_body_read_count(), body_reads);
            assert_eq!(state.store.selected_metadata_read_stats().book_text_rows, 1);
            let pending = state
                .downloads
                .iter()
                .map(|task| task.id.clone())
                .collect::<Vec<_>>();
            state.store.reset_book_read_stats();
            state.store.reset_selected_metadata_read_stats();
            with_book_metadata_test_limits(
                BookMetadataLimits {
                    bytes: 1,
                    ..Default::default()
                },
                || -> Result<()> {
                    assert!(
                        state
                            .enqueue_download_options(&fixture.book.id, vec![0], false)
                            .unwrap_err()
                            .downcast_ref::<BookMetadataLimit>()
                            .is_some()
                    );
                    assert_eq!(state.store.book_read_stats().intro_copies, 0);
                    assert_eq!(state.store.chapter_body_read_count(), body_reads);
                    assert_eq!(state.store.selected_metadata_read_stats().book_text_rows, 0);
                    Ok(())
                },
            )?;
            assert_eq!(
                state
                    .downloads
                    .iter()
                    .map(|task| task.id.clone())
                    .collect::<Vec<_>>(),
                pending
            );
            assert_eq!(
                state.store.book(&fixture.book.id)?.unwrap().intro,
                fixture.book.intro
            );
            assert_eq!(
                state.store.annotations(&fixture.book.id)?,
                vec![fixture.note.clone()]
            );
            assert_eq!(
                state.store.task(&fixture.task.id)?,
                Some(fixture.task.clone())
            );
        }
        fixture.finish()
    }

    fn detail_payload_tabs() -> Result<()> {
        let fixture = Fixture::new()?;
        let other = rusqlite::Connection::open(fixture._directory.path().join("library.sqlite"))?;
        other.execute(
            "UPDATE books SET intro=x'80' WHERE id=?1",
            [&fixture.book.id],
        )?;
        other.execute(
            "INSERT INTO settings(key,value) VALUES (?1,x'80')",
            [format!("book.metadata:{}", fixture.book.id)],
        )?;
        {
            let mut state = fixture.state();
            state.load_detail(&fixture.book.id)?;
            for tab in ["toc", "reviews", "notes"] {
                state.detail_tab = tab.into();
                state.store.reset_book_read_stats();
                state.store.reset_book_introduction_read_stats();
                let body_reads = state.store.chapter_body_read_count();
                state.render_detail()?;
                assert_eq!(state.store.book_read_stats().intro_copies, 0);
                assert_eq!(
                    state.store.book_introduction_read_stats(),
                    crate::store::BookIntroductionReadStats::default()
                );
                assert_eq!(state.store.chapter_body_read_count(), body_reads);
                assert!(
                    fixture
                        .ui
                        .as_ref()
                        .unwrap()
                        .global::<DetailModel>()
                        .get_intro()
                        .is_empty()
                );
            }
            other.execute(
                "UPDATE books SET intro=?1 WHERE id=?2",
                rusqlite::params![fixture.book.intro, fixture.book.id],
            )?;
            other.execute(
                "UPDATE settings SET value=?1 WHERE key=?2",
                rusqlite::params![
                    r#"{"bookInfo":{"publisher":"Publisher","isbn":"123"}}"#,
                    format!("book.metadata:{}", fixture.book.id)
                ],
            )?;
            state.detail_tab = "intro".into();
            state.store.reset_book_read_stats();
            state.store.reset_book_introduction_read_stats();
            state.render_detail()?;
            let view = fixture.ui.as_ref().unwrap().global::<DetailModel>();
            assert_eq!(view.get_intro().as_str(), fixture.book.intro.as_str());
            assert_eq!(view.get_publisher(), "Publisher");
            assert_eq!(view.get_isbn(), "123");
            assert_eq!(state.store.book_read_stats().intro_copies, 1);
            assert_eq!(
                state.store.book_read_stats().intro_bytes,
                fixture.book.intro.len()
            );
            assert_eq!(state.store.book_introduction_read_stats().metadata_reads, 1);
            state.detail_tab = "toc".into();
            state.store.reset_book_read_stats();
            state.render_detail()?;
            assert!(view.get_intro().is_empty());
            assert_eq!(state.store.book_read_stats().intro_copies, 0);
        }
        drop(other);
        fixture.finish()
    }

    fn intro_quota_keeps_view() -> Result<()> {
        let fixture = Fixture::new()?;
        {
            let mut state = fixture.state();
            state.load_detail(&fixture.book.id)?;
            state.detail_tab = "intro".into();
            state.render_detail()?;
            let view = fixture.ui.as_ref().unwrap().global::<DetailModel>();
            let intro = view.get_intro();
            let summary = view.get_summary();
            let tab = view.get_tab();
            let body = state.reader.body.clone();
            let epoch = state.jobs.account_epoch();
            let preferences = serde_json::to_value(&state.prefs)?;
            state.store.reset_book_read_stats();
            state.store.reset_book_introduction_read_stats();
            crate::store::with_book_introduction_test_limits(
                crate::store::BookIntroductionLimits {
                    bytes: 1,
                    ..Default::default()
                },
                || -> Result<()> {
                    state.render_detail()?;
                    assert_eq!(view.get_intro(), intro);
                    assert_eq!(view.get_summary(), summary);
                    assert_eq!(view.get_tab(), tab);
                    assert_eq!(view.get_status(), tr("api_error"));
                    assert_eq!(state.store.book_read_stats().intro_copies, 0);
                    assert_eq!(state.store.book_introduction_read_stats().metadata_reads, 0);
                    Ok(())
                },
            )?;
            assert_eq!(state.reader.body, body);
            assert_eq!(state.reader.selection, Some((0, 3)));
            assert_eq!(state.reader.draft.query, "Keep selected draft");
            assert_eq!(state.jobs.account_epoch(), epoch);
            assert_eq!(serde_json::to_value(&state.prefs)?, preferences);
            assert_eq!(
                state.store.book(&fixture.book.id)?.unwrap().intro,
                fixture.book.intro
            );
            assert_eq!(
                state.store.annotations(&fixture.book.id)?,
                vec![fixture.note.clone()]
            );
            assert_eq!(
                state.store.task(&fixture.task.id)?,
                Some(fixture.task.clone())
            );
        }
        fixture.finish()
    }

    #[test]
    fn genuine_detail_tabs_project_intro_metadata_only_when_it_is_visible() -> Result<()> {
        isolated(
            "app::library::selected_metadata_tests::genuine_detail_tabs_project_intro_metadata_only_when_it_is_visible",
            detail_payload_tabs,
        )
    }

    #[test]
    fn genuine_intro_payload_quota_preserves_existing_view_and_private_data() -> Result<()> {
        isolated(
            "app::library::selected_metadata_tests::genuine_intro_payload_quota_preserves_existing_view_and_private_data",
            intro_quota_keeps_view,
        )
    }

    #[test]
    fn genuine_detail_source_only_read_and_quota_preserve_the_held_context() -> Result<()> {
        isolated(
            "app::library::selected_metadata_tests::genuine_detail_source_only_read_and_quota_preserve_the_held_context",
            source_only_detail,
        )
    }

    #[test]
    fn genuine_reimport_keeps_numeric_state_and_supplied_full_introduction() -> Result<()> {
        isolated(
            "app::library::selected_metadata_tests::genuine_reimport_keeps_numeric_state_and_supplied_full_introduction",
            reimport_state,
        )
    }

    #[test]
    fn genuine_download_source_metadata_and_denial_keep_the_pending_intent() -> Result<()> {
        isolated(
            "app::library::selected_metadata_tests::genuine_download_source_metadata_and_denial_keep_the_pending_intent",
            download_source,
        )
    }

    #[test]
    fn selected_access_owner_rule_retains_remote_status_without_presence_authority() -> Result<()> {
        let store = Store::open(":memory:")?;
        store.upsert_book(&Book {
            source: "weread".into(),
            intro: "Keep intro".into(),
            ..Book::new("book", "Owner fixture")
        })?;
        store.upsert_chapter(&Chapter {
            id: "uid".into(),
            book_id: "book".into(),
            body: Some("Body".into()),
            content_status: ContentStatus::Preview,
            content_account_key: "owner".into(),
            ..Default::default()
        })?;
        let entry = store.chapter_access_metadata("book", 0)?.unwrap();
        assert_eq!(store.chapter_body_read_count(), 0);
        assert_eq!(
            chapter_content_status_for_access_owner("weread", &entry, Some("owner")),
            ContentStatus::Preview
        );
        for owner in [None, Some("other")] {
            assert_eq!(
                chapter_content_status_for_access_owner("weread", &entry, owner),
                ContentStatus::Unknown
            );
        }
        store.upsert_chapter(&Chapter {
            id: "uid".into(),
            book_id: "book".into(),
            body: Some("Body".into()),
            content_status: ContentStatus::Full,
            content_account_key: "owner".into(),
            ..Default::default()
        })?;
        let full = store.chapter_access_metadata("book", 0)?.unwrap();
        assert_eq!(
            chapter_content_status_for_access_owner("weread", &full, Some("owner")),
            ContentStatus::Full
        );
        assert_eq!(
            chapter_content_status_for_access_owner("weread", &full, Some("other")),
            ContentStatus::Unknown
        );
        assert_eq!(store.chapter_body_read_count(), 0);
        Ok(())
    }

    #[test]
    fn selected_access_nonremote_rule_matches_original_unicode_trim_and_nul() -> Result<()> {
        for body in ["Body", "\0", "\u{3000}\0\u{3000}"] {
            let store = Store::open(":memory:")?;
            let book = Book {
                source: "local".into(),
                ..Book::new("book", "Local fixture")
            };
            store.upsert_book(&book)?;
            store.upsert_chapter(&Chapter {
                id: "uid".into(),
                book_id: "book".into(),
                body: Some(body.into()),
                content_status: ContentStatus::Preview,
                ..Default::default()
            })?;
            let entry = store.chapter_access_metadata("book", 0)?.unwrap();
            assert_eq!(store.chapter_body_read_count(), 0);
            assert!(entry.chapter.body.is_none());
            let expected =
                chapter_content_status_for_owner(&book, &store.chapter("book", 0)?.unwrap(), None);
            assert_eq!(
                chapter_content_status_for_access_owner("local", &entry, None),
                expected
            );
        }
        let entry = crate::store::ChapterAccessMetadata {
            body_present: true,
            ..Default::default()
        };
        assert_eq!(
            chapter_content_status_for_access_owner("local", &entry, None),
            ContentStatus::Unknown
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_tabs_clear_download_selection_and_restore_the_current_catalog_page() {
        let mut selecting = true;
        let mut selected = BTreeSet::from([5, 6, 8]);
        let page = detail_tab_page("intro", Some(37), 7, &mut selecting, &mut selected);
        assert_eq!(page, 0);
        assert!(!selecting);
        assert!(selected.is_empty());

        let page = detail_tab_page("toc", Some(37), 7, &mut selecting, &mut selected);
        assert_eq!(page, 5);
        assert!(!selecting);
        assert!(selected.is_empty());

        selecting = true;
        selected.insert(40);
        assert_eq!(
            detail_tab_page("reviews", Some(37), 12, &mut selecting, &mut selected),
            0
        );
        assert!(!selecting);
        assert!(selected.is_empty());
    }

    #[test]
    fn detail_paging_keeps_rich_notes_separate_from_review_and_catalog_capacity() {
        for profile in [DeviceProfile::Paperwhite6, DeviceProfile::Scribe] {
            let notes = detail_page_size(profile, "notes");
            let reviews = detail_page_size(profile, "reviews");
            assert!(notes < reviews);
            assert_eq!(
                detail_page_size(profile, "toc"),
                profile.detail_chapter_page_size()
            );
            let count = page_count(notes * 2 + 1, notes);
            let first = move_page(0, 1, count);
            let second = move_page(first, 1, count);
            assert_eq!((first, second), (1, 2));
            assert_eq!(move_page(second, 1, count), second);
        }
    }

    #[test]
    fn note_rows_preserve_chapter_order_quote_thought_and_unknown_dates() {
        let rows = detail_note_rows(
            vec![
                Annotation {
                    id: "second".into(),
                    chapter_index: 8,
                    start: 10,
                    quote: "Second quote".into(),
                    ..Default::default()
                },
                Annotation {
                    id: "first".into(),
                    chapter_index: 2,
                    start: 4,
                    quote: "First quote".into(),
                    note: "A personal thought".into(),
                    ..Default::default()
                },
            ],
            &[Chapter {
                index: 2,
                title: "A known chapter".into(),
                ..Default::default()
            }],
        );
        assert_eq!(rows[0].id.as_str(), "first");
        assert_eq!(rows[0].chapter.as_str(), "A known chapter");
        assert_eq!(rows[0].quote.as_str(), "First quote");
        assert_eq!(rows[0].thought.as_str(), "A personal thought");
        assert_eq!(rows[0].date.as_str(), tr("metadata_unavailable"));
        assert_eq!(rows[1].chapter.as_str(), tr("chapter_unavailable"));
    }

    #[test]
    fn detail_metadata_never_fills_missing_fields_from_unrelated_values() {
        let raw = serde_json::json!({
            "publisher": "Unrelated wrapper publisher",
            "bookInfo": {"publisher": "Verified publisher", "isbn": " 9780000000000 "}
        });
        assert_eq!(
            detail_metadata_text(Some(&raw), "publisher"),
            "Verified publisher"
        );
        assert_eq!(detail_metadata_text(Some(&raw), "isbn"), "9780000000000");
        assert_eq!(
            detail_metadata_text(Some(&raw), "published"),
            tr("metadata_unavailable")
        );
        assert_eq!(
            detail_metadata_text(None, "isbn"),
            tr("metadata_unavailable")
        );
    }

    #[test]
    fn download_scope_survives_renewal_but_never_changes_accounts() {
        let session = source::AuthSession {
            user_vid: "fixture-account-a".into(),
            cookies: BTreeMap::from([("wr_skey".into(), "fixture-cookie-a".into())]),
            ..Default::default()
        };
        let owner = download_account_key(&session).unwrap();
        assert_eq!(owner.len(), 64);
        let mut renewed = session.clone();
        renewed
            .cookies
            .insert("wr_skey".into(), "fixture-renewed-cookie-a".into());
        assert_eq!(download_account_key(&renewed).as_ref(), Some(&owner));
        renewed.user_vid = "fixture-account-b".into();
        assert_ne!(download_account_key(&renewed).as_ref(), Some(&owner));
        assert!(
            download_account_key(&source::AuthSession {
                api_key: "fixture-key-only".into(),
                ..Default::default()
            })
            .is_none()
        );
        let anonymous = source::AuthSession {
            cookies: session.cookies.clone(),
            ..Default::default()
        };
        let mut other = anonymous.clone();
        other
            .cookies
            .insert("wr_skey".into(), "fixture-other-session".into());
        assert_ne!(
            download_account_key(&anonymous),
            download_account_key(&other)
        );
    }

    #[test]
    fn only_transient_transport_errors_are_automatically_retried() {
        for error in [
            "source.network_unavailable",
            "source.timeout",
            "source.http.429",
            "source.http.500",
            "source.http.503",
            "source.resource_http.429",
            "source.resource_http.500",
            "source.resource_http.503",
        ] {
            assert!(transient_download_error(Some(error)));
        }
        for error in [
            "source.http.401",
            "source.http.403",
            "source.http.404",
            "source.resource_http.401",
            "source.resource_http.403",
            "source.resource_http.404",
            "source.integrity_check_failed",
            "source.delivery_uncertain",
            "source.unlock_required",
            "source.content_unverified",
            "source.content_changed",
            "source.http.500 invalid",
            "source.resource_http.500 invalid",
        ] {
            assert!(!transient_download_error(Some(error)));
        }
        assert!(!transient_download_error(None));
    }

    #[test]
    fn metadata_timestamps_do_not_claim_unread_content_updates() {
        let remote = source::RemoteBook {
            id: "fixture".into(),
            updated_at: 1_800_000_000,
            ..Default::default()
        };
        assert!(!normalize_book(&remote, None, true).updated);
    }

    #[test]
    fn local_shelf_overrides_survive_a_different_upstream_snapshot() {
        let existing = vec![
            Book {
                source: "weread".into(),
                ..Book::new("hidden", "Hidden")
            },
            Book {
                source: "weread".into(),
                ..Book::new("pinned", "Pinned")
            },
            Book {
                source: "local".into(),
                ..Book::new("local", "Local")
            },
        ];
        let remote = vec![
            source::RemoteBook {
                id: "hidden".into(),
                title: "Hidden".into(),
                ..Default::default()
            },
            source::RemoteBook {
                id: "remote".into(),
                title: "Remote".into(),
                ..Default::default()
            },
        ];
        let overrides = BTreeMap::from([("hidden".into(), false), ("pinned".into(), true)]);
        let merged = merge_shelf(&remote, &existing, &overrides);
        assert_eq!(
            merged
                .iter()
                .map(|book| book.id.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["local", "pinned", "remote"])
        );
    }
}
