use serde::{Deserialize, Serialize};

/// Application-owned metadata, independent of upstream response formats.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub author: String,
    pub category: String,
    pub intro: String,
    pub cover_url: String,
    pub source: String,
    /// A fraction in the inclusive range 0..=1.
    pub progress: f32,
    pub total_chapters: u32,
    pub cached_chapters: u32,
    pub updated: bool,
    pub finished: bool,
    pub on_shelf: bool,
}

impl Book {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            on_shelf: true,
            ..Self::default()
        }
    }
}

/// Read-only list metadata. Deliberately omits introductions and has no
/// Serialize/Into<Book> implementation: it cannot overwrite a stored book.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BookMetadata {
    pub id: String,
    pub title: String,
    pub author: String,
    pub category: String,
    pub cover_url: String,
    pub source: String,
    pub progress: f32,
    pub total_chapters: u32,
    pub cached_chapters: u32,
    pub updated: bool,
    pub finished: bool,
    pub on_shelf: bool,
}

/// Exact local-statistics book fields. This projection is not a writable Book.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BookStatisticsMetadata {
    pub id: String,
    pub title: String,
    pub category: String,
    pub finished: bool,
}

impl From<&Book> for BookMetadata {
    fn from(book: &Book) -> Self {
        Self {
            id: book.id.clone(),
            title: book.title.clone(),
            author: book.author.clone(),
            category: book.category.clone(),
            cover_url: book.cover_url.clone(),
            source: book.source.clone(),
            progress: book.progress,
            total_chapters: book.total_chapters,
            cached_chapters: book.cached_chapters,
            updated: book.updated,
            finished: book.finished,
            on_shelf: book.on_shelf,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentStatus {
    #[default]
    Unknown,
    Full,
    Preview,
    Locked,
}

impl ContentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Full => "full",
            Self::Preview => "preview",
            Self::Locked => "locked",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "unknown" => Some(Self::Unknown),
            "full" => Some(Self::Full),
            "preview" => Some(Self::Preview),
            "locked" => Some(Self::Locked),
            _ => None,
        }
    }
}

impl std::str::FromStr for ContentStatus {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value).ok_or("invalid content status")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chapter {
    pub id: String,
    pub book_id: String,
    /// A stable, zero-based position in the book's table of contents.
    pub index: u32,
    pub title: String,
    /// True only when a nonempty body is verified as full content.
    pub cached: bool,
    /// Retained text remains locally readable even when its access is unverified or limited.
    pub body: Option<String>,
    #[serde(default)]
    pub content_status: ContentStatus,
    /// Opaque account identity hash; never credentials or a session cookie.
    #[serde(default)]
    pub content_account_key: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadingPosition {
    pub book_id: String,
    pub chapter_index: u32,
    /// Unicode scalar offset, independent of current font and pagination.
    pub char_offset: u32,
    pub updated_at: i64,
    pub device: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Annotation {
    pub id: String,
    pub book_id: String,
    pub chapter_index: u32,
    /// Half-open Unicode scalar range in the extracted chapter text.
    pub start: u32,
    pub end: u32,
    pub quote: String,
    pub note: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bookmark {
    pub id: String,
    pub book_id: String,
    pub chapter_index: u32,
    pub char_offset: u32,
    pub label: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadingSession {
    pub id: String,
    pub book_id: String,
    pub started_at: i64,
    pub ended_at: i64,
    /// Active reading seconds, excluding suspension and inactive routes.
    pub seconds: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    #[default]
    Pending,
    Running,
    Paused,
    Complete,
    Cancelled,
    Failed,
}

impl DownloadStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Complete => "complete",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    pub fn is_pending(self) -> bool {
        matches!(self, Self::Pending | Self::Running | Self::Paused)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DownloadTask {
    pub id: String,
    pub book_id: String,
    /// Opaque account identity hash; credentials are never stored in task metadata.
    pub account_key: String,
    /// Explicit chapter indices; there is no fabricated whole-book format.
    pub chapters: Vec<u32>,
    pub status: DownloadStatus,
    pub include_images: bool,
    /// End of the contiguous completed prefix in `chapters`, not an identifier.
    pub next_index: usize,
    /// A redacted error description suitable for local recovery.
    pub error: Option<String>,
    pub updated_at: i64,
    /// Independently verified chapters, including completions beyond a gap.
    /// `next_index` remains only the contiguous completed prefix.
    pub completed_count: usize,
}

/// Durable task-local state, addressed by the stable source chapter UID.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadChapterStatus {
    #[default]
    Pending,
    Complete,
    RetryWait,
    Blocked,
}

impl DownloadChapterStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Complete => "complete",
            Self::RetryWait => "retry_wait",
            Self::Blocked => "blocked",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "complete" => Some(Self::Complete),
            "retry_wait" => Some(Self::RetryWait),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DownloadChapterState {
    pub chapter_uid: String,
    pub chapter_index: u32,
    pub ordinal: usize,
    pub status: DownloadChapterStatus,
    /// Failed attempts for this UID only; the fourth failure exhausts retries.
    pub retry_count: u8,
    pub retry_not_before: i64,
    /// A bounded, redacted source error code, never a response or credentials.
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadChapterRetry {
    RetryAt(i64),
    Blocked,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DownloadChapterProgress {
    pub completed_count: usize,
    pub next_index: usize,
    pub pending_count: usize,
    pub retry_wait_count: usize,
    pub blocked_count: usize,
    pub retry_not_before: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DownloadChapterCommit {
    pub newly_cached: bool,
    pub progress: DownloadChapterProgress,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StorageSummary {
    pub books: u64,
    pub cached_chapters: u64,
    pub body_bytes: u64,
    pub annotations: u64,
    pub pending_tasks: u64,
}
