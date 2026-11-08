use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Local account material. Debug deliberately never prints credentials.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthSession {
    pub api_key: String,
    pub cookies: BTreeMap<String, String>,
    pub account_name: String,
    pub user_vid: String,
    pub login_time: i64,
    pub wr_ticket: String,
    pub wr_wrpa: String,
}

impl AuthSession {
    pub(crate) fn validate_cookie_budget(&self) -> super::Result<()> {
        super::client::validate_cookie_jar(&self.cookies)
    }

    pub fn has_api_key(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    pub fn has_web_session(&self) -> bool {
        self.cookies.get("wr_skey").is_some_and(|s| s.len() >= 8)
            || self.cookies.get("wr_gid").is_some_and(|s| s.len() >= 5)
    }

    pub fn is_configured(&self) -> bool {
        self.has_api_key() || self.has_web_session()
    }
}

impl std::fmt::Debug for AuthSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthSession")
            .field("api_key_present", &self.has_api_key())
            .field("web_session_present", &self.has_web_session())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RemoteBook {
    pub id: String,
    pub title: String,
    pub author: String,
    pub cover_url: String,
    pub category: String,
    pub intro: String,
    pub publisher: String,
    pub isbn: String,
    pub chapter_count: u32,
    pub progress: u32,
    pub finished: bool,
    pub updated_at: i64,
    #[serde(default)]
    pub raw: Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RemoteChapter {
    pub uid: String,
    pub title: String,
    pub index: u32,
    pub word_count: u32,
    /// Resource archive reference from the upstream catalog's `tar` field.
    #[serde(default)]
    pub resource_url: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShelfSnapshot {
    pub books: Vec<RemoteBook>,
    pub mp_accounts: Vec<MpAccount>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MpAccount {
    pub id: String,
    pub name: String,
    pub cover_url: String,
    pub updated_at: i64,
    pub unread_count: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MpArticle {
    pub id: String,
    pub account_id: String,
    pub title: String,
    pub author: String,
    pub published_at: i64,
    pub original_url: String,
    pub cover_url: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReadingPosition {
    pub chapter_uid: String,
    pub chapter_index: u32,
    pub chapter_offset: u32,
    pub progress: u32,
    pub summary: String,
    pub updated_at: i64,
    pub device: String,
}

/// Full position metadata remains distinct from the twenty-scalar read wire.
#[derive(Clone, Copy, Debug)]
pub struct ReadingPositionLimits {
    pub uid_bytes: usize,
    pub summary_bytes: usize,
    pub device_bytes: usize,
    pub scalar_bytes: usize,
    pub metadata_bytes: usize,
    pub work_bytes: usize,
}

impl Default for ReadingPositionLimits {
    fn default() -> Self {
        Self {
            uid_bytes: 4096,
            summary_bytes: 4 * 1024 * 1024,
            device_bytes: 4096,
            scalar_bytes: 4096,
            metadata_bytes: 64 * 1024 * 1024,
            work_bytes: 128 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadingPositionLimitKind {
    Uid,
    Summary,
    Device,
    Scalar,
    Metadata,
    Work,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("source.position_limit.{kind:?}: {used} + {requested} exceeds {limit}")]
pub struct ReadingPositionLimitError {
    pub kind: ReadingPositionLimitKind,
    pub used: usize,
    pub requested: usize,
    pub limit: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadingPositionUsage {
    pub field_bytes: usize,
    pub metadata_bytes: usize,
    pub work_bytes: usize,
    pub nodes: usize,
}

#[cfg(test)]
pub(crate) use super::client::position_budget_tests::{
    ObservationScope as ReadingPositionObservationScope, Stats as ReadingPositionObservationStats,
};

#[derive(Debug, thiserror::Error)]
pub enum ReadingPositionSettingError {
    #[error(transparent)]
    Limit(#[from] ReadingPositionLimitError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl ReadingPositionSettingError {
    pub fn is_capacity(&self) -> bool {
        matches!(self, Self::Limit(_))
    }
}

/// Call with borrowed decoded lengths before owning any selected field.
pub fn admit_reading_position_fields(
    uid_bytes: usize,
    summary_bytes: usize,
    device_bytes: usize,
) -> std::result::Result<ReadingPositionUsage, ReadingPositionLimitError> {
    let mut budget = super::protocol::ReadingPositionBudget::new()?;
    budget.fields(uid_bytes, summary_bytes, device_bytes)?;
    Ok(budget.usage())
}

/// The setting reader supplies an admitted SQLite/UTF-8 snapshot. This second
/// gate prices every relevant JSON occurrence before deserializing the DTO.
pub fn decode_reading_position_setting(
    input: &str,
) -> std::result::Result<ReadingPosition, ReadingPositionSettingError> {
    super::content::decode_reading_position_setting(input)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ContentFormat {
    Xhtml,
    Text,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChapterContent {
    pub html: String,
    pub plain_text: String,
    pub style: Option<String>,
    pub format: ContentFormat,
    /// Transport integrity alone does not establish access to a complete chapter.
    #[serde(default)]
    pub content_status: crate::domain::ContentStatus,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RemoteReview {
    pub id: String,
    pub author: String,
    pub text: String,
    pub likes: u32,
    pub published_at: i64,
    pub recommended: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RemoteUnderline {
    pub range: String,
    pub text: String,
    pub count: u32,
    pub thoughts: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReadStatistics {
    pub total_seconds: u64,
    pub read_days: u32,
    pub average_seconds: u64,
    pub base_time: i64,
    pub daily_seconds: BTreeMap<String, u64>,
    pub raw: Value,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum StatisticsPeriod {
    Week,
    #[default]
    Month,
    Year,
    Total,
}

impl StatisticsPeriod {
    pub fn api_mode(self) -> &'static str {
        match self {
            Self::Week => "weekly",
            Self::Month => "monthly",
            Self::Year => "annually",
            Self::Total => "overall",
        }
    }
}

#[derive(Clone, Debug)]
pub struct QrLoginChallenge {
    pub uid: String,
    pub confirm_url: String,
    pub expires_at: i64,
}

#[derive(Clone, Debug)]
pub enum QrLoginState {
    Pending,
    NeedCode,
    Expired,
    CodeMismatch,
    Authenticated(AuthSession),
}

/// Availability depends on configured credentials, not on live endpoint probes.
#[derive(Clone, Debug, Default)]
pub struct SourceCapabilities {
    pub catalog: bool,
    pub search: bool,
    pub chapter_content: bool,
    pub remote_progress: bool,
    pub statistics: bool,
    pub mp_articles: bool,
    pub remote_shelf_mutation: bool,
    pub remote_annotation_write: bool,
    pub reading_time: bool,
    pub chapter_images: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("source.api_key_required")]
    ApiKeyRequired,
    #[error("source.web_login_required")]
    WebLoginRequired,
    #[error("source.login_expired")]
    LoginExpired,
    #[error("source.http.{0}")]
    Http(u16),
    #[error("source.resource_http.{0}")]
    ResourceHttp(u16),
    #[error("source.api.{0}")]
    Api(String),
    #[error("source.timeout")]
    Timeout,
    #[error("source.network_unavailable")]
    Network,
    #[error("source.busy")]
    Busy,
    #[error("source.invalid_response.{0}")]
    InvalidResponse(&'static str),
    #[error("source.integrity_check_failed")]
    Integrity,
    #[error("source.response_too_large")]
    ResponseTooLarge,
    #[error("source.delivery_uncertain")]
    DeliveryUncertain,
    #[error("source.local_checkpoint_failed")]
    CheckpointFailed,
    #[error("source.unlock_required")]
    UnlockRequired,
    #[error("source.content_unverified")]
    ContentUnverified,
    #[error("source.content_changed")]
    ContentChanged,
    #[error("source.unsupported.{0}")]
    Unsupported(&'static str),
}

impl SourceError {
    pub fn requires_login(&self) -> bool {
        match self {
            Self::ApiKeyRequired
            | Self::WebLoginRequired
            | Self::LoginExpired
            | Self::Http(401) => true,
            Self::Api(code) => matches!(code.as_str(), "-2010" | "-2012" | "-2013"),
            _ => false,
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Timeout
                | Self::Network
                | Self::Busy
                | Self::Http(429 | 500..=599)
                | Self::ResourceHttp(429 | 500..=599)
        )
    }
}

pub type Result<T> = std::result::Result<T, SourceError>;

#[cfg(test)]
mod tests {
    use super::SourceError;

    #[test]
    fn local_request_budget_exhaustion_is_retryable_without_requiring_login() {
        let error = SourceError::Busy;
        assert!(error.retryable());
        assert!(!error.requires_login());
        assert_eq!(error.to_string(), "source.busy");
        let failure = super::super::report::ReadReportFailure::from(&error);
        assert!(failure.retryable);
        assert!(!failure.requires_login);
        assert!(!failure.uncertain);
    }

    #[test]
    fn content_access_failures_do_not_revoke_credentials_or_retry_automatically() {
        for error in [
            SourceError::UnlockRequired,
            SourceError::ContentUnverified,
            SourceError::ContentChanged,
        ] {
            assert!(!error.requires_login());
            assert!(!error.retryable());
        }
        let old_content: super::ChapterContent = serde_json::from_value(serde_json::json!({
            "html":"<p>Fixture</p>", "plain_text":"Fixture", "style":null, "format":"Text"
        }))
        .unwrap();
        assert_eq!(
            old_content.content_status,
            crate::domain::ContentStatus::Unknown
        );
    }

    #[test]
    fn resource_denials_do_not_require_account_login() {
        assert!(SourceError::Http(401).requires_login());
        assert!(!SourceError::Http(403).requires_login());
        for status in [401, 403, 404, 429, 500, 503] {
            let error = SourceError::ResourceHttp(status);
            assert!(!error.requires_login());
            assert_eq!(error.to_string(), format!("source.resource_http.{status}"));
            assert_eq!(error.retryable(), status == 429 || status >= 500);
        }
        assert!(!SourceError::ResourceHttp(599).requires_login());
        assert!(SourceError::ResourceHttp(599).retryable());
        assert!(!SourceError::ResourceHttp(600).retryable());
    }

    #[test]
    fn official_identity_codes_are_distinct_from_unknown_api_failures() {
        for code in ["-2010", "-2012", "-2013"] {
            let error = SourceError::Api(code.into());
            assert!(error.requires_login());
            assert!(!error.retryable());
            assert_eq!(error.to_string(), format!("source.api.{code}"));
            assert!(!matches!(error, SourceError::DeliveryUncertain));
        }
        for code in ["-2011", "-12013", "2012", "fixture-unknown"] {
            assert!(!SourceError::Api(code.into()).requires_login());
        }
    }
}
