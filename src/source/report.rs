//! Reading-time transport observed in read_report.lua and protocol.lua.
//! The wire operation also writes reading position; it has no idempotency key.
use super::{
    AuthSession, ReadingPosition, RemoteChapter, Result, SourceError, WeReadClient, protocol,
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[cfg(test)]
pub(crate) use super::client::api_gate_test_support::{
    HttpFixture as AllocationHttpFixture, RecordedRequest as AllocationRecordedRequest,
    Reply as AllocationReply,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadReportRequest {
    /// Local outbox identity. It is not sent as an invented server parameter.
    pub session_id: String,
    /// Stable account identity, independent of expiring cookie values.
    pub account_key: String,
    pub book_id: String,
    pub started_at: i64,
    /// Actual active Reader seconds in this batch, excluding idle or suspended time.
    pub seconds: u32,
    /// The observed endpoint always includes position; callers must opt into it.
    pub progress_write_allowed: bool,
    /// True only for a currently active Reader whose local choice was confirmed.
    #[serde(default)]
    pub active_reader_intent: bool,
    pub position: Option<ReadingPosition>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadReportReceipt {
    pub accepted_at: i64,
    pub seconds: u32,
    pub has_synckey: bool,
}

/// One atomic failure record prevents a partial write from losing delivery uncertainty.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadReportFailure {
    pub code: String,
    pub retryable: bool,
    pub requires_login: bool,
    pub uncertain: bool,
}

impl From<&SourceError> for ReadReportFailure {
    fn from(error: &SourceError) -> Self {
        Self {
            code: error.to_string(),
            retryable: error.retryable(),
            requires_login: error.requires_login(),
            uncertain: matches!(error, SourceError::DeliveryUncertain),
        }
    }
}

/// A QR-proven VID or an explicitly configured web VID identifies the local outbox.
/// Unknown or conflicting identities cannot borrow another account's reading time.
pub fn account_key(session: &AuthSession) -> Option<String> {
    let account = session.user_vid.trim();
    let cookie = session
        .cookies
        .get("wr_vid")
        .map(|value| value.trim())
        .unwrap_or_default();
    if !account.is_empty() && !cookie.is_empty() && account != cookie {
        return None;
    }
    let vid = if account.is_empty() { cookie } else { account };
    if vid.is_empty()
        || vid.len() > 256
        || !vid
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return None;
    }
    Some(hex::encode(Sha256::digest(
        format!("weread-account\n{vid}").as_bytes(),
    )))
}

impl ReadReportRequest {
    pub fn validate(&self, auth: &AuthSession, now: i64) -> Result<&ReadingPosition> {
        if !auth.has_web_session() {
            return Err(SourceError::WebLoginRequired);
        }
        let current =
            account_key(auth).ok_or(SourceError::Unsupported("account_identity_required"))?;
        if self.account_key != current {
            return Err(SourceError::Unsupported("report_account_mismatch"));
        }
        if !self.progress_write_allowed {
            return Err(SourceError::Unsupported(
                "time_report_requires_progress_write",
            ));
        }
        if !auth.has_api_key() {
            return Err(progress_check_required());
        }
        if self.session_id.is_empty()
            || self.session_id.len() > 512
            || self.session_id.chars().any(char::is_control)
        {
            return Err(SourceError::InvalidResponse("report_session"));
        }
        if self.book_id.is_empty()
            || self.book_id.starts_with("MP_WXS_")
            || self.book_id.len() > 2048
        {
            return Err(SourceError::Unsupported("read_report_book"));
        }
        if self.seconds == 0
            || self.started_at <= 0
            || self.started_at > now
            || u64::from(self.seconds) > now.saturating_sub(self.started_at) as u64
        {
            return Err(SourceError::InvalidResponse("report_elapsed_time"));
        }
        let position = self
            .position
            .as_ref()
            .filter(|position| !position.chapter_uid.is_empty())
            .ok_or(SourceError::Unsupported("read_position_required"))?;
        if position.progress > 100 {
            return Err(SourceError::InvalidResponse("report_progress"));
        }
        Ok(position)
    }
}

fn progress_check_required() -> SourceError {
    SourceError::Unsupported("read_report_progress_check_required")
}

struct ReportPositionProjection<'position, 'catalog> {
    source: &'position ReadingPosition,
    chapter: &'catalog RemoteChapter,
    offset: u32,
    progress: u32,
}

fn project_report_position<'position, 'catalog>(
    position: &'position ReadingPosition,
    coordinates: &std::collections::BTreeMap<&'catalog str, (u64, &'catalog RemoteChapter)>,
    total: u64,
) -> Result<ReportPositionProjection<'position, 'catalog>> {
    #[cfg(test)]
    report_position_gate_tests::observe_projection();
    if position.chapter_uid.is_empty() {
        return Err(progress_check_required());
    }
    let (prefix, chapter) = coordinates
        .get(position.chapter_uid.as_str())
        .ok_or_else(progress_check_required)?;
    let offset = position.chapter_offset.min(chapter.word_count);
    let coordinate = prefix
        .checked_add(u64::from(offset))
        .ok_or_else(progress_check_required)?;
    Ok(ReportPositionProjection {
        source: position,
        chapter,
        offset,
        progress: ((u128::from(coordinate) * 100 / u128::from(total)).min(100)) as u32,
    })
}

fn report_summary_prefix(summary: &str) -> &str {
    let mut end = 0usize;
    for (index, character) in summary.char_indices().take(20) {
        #[cfg(test)]
        report_position_gate_tests::observe_summary_scalar();
        end = index + character.len_utf8();
    }
    &summary[..end]
}

/// Verify coordinates without interpreting a lower percentage as stale by itself.
/// Active, confirmed local reading is an intent; an old batch needs recency proof.
pub fn checked_report_position(
    local: &ReadingPosition,
    cloud: &ReadingPosition,
    catalog: &[RemoteChapter],
    active_reader_intent: bool,
    now: i64,
) -> Result<ReadingPosition> {
    if catalog.is_empty() {
        return Err(progress_check_required());
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut indices = std::collections::BTreeSet::new();
    let mut before = 0u64;
    let mut coordinates = std::collections::BTreeMap::new();
    for chapter in catalog {
        if chapter.uid.is_empty()
            || chapter.word_count == 0
            || !ids.insert(chapter.uid.as_str())
            || !indices.insert(chapter.index)
        {
            return Err(progress_check_required());
        }
        coordinates.insert(chapter.uid.as_str(), (before, chapter));
        before = before
            .checked_add(u64::from(chapter.word_count))
            .ok_or_else(progress_check_required)?;
    }
    if before == 0 {
        return Err(progress_check_required());
    }
    // Normalize coordinates without copying either source position. Select the
    // same source as before, then materialize only its actual wire summary.
    let local = project_report_position(local, &coordinates, before)?;
    let cloud = project_report_position(cloud, &coordinates, before)?;
    let trusted_times = local.source.updated_at > 0
        && cloud.source.updated_at > 0
        && local.source.updated_at <= now
        && cloud.source.updated_at <= now;
    let selected = if active_reader_intent
        || trusted_times && local.source.updated_at > cloud.source.updated_at
    {
        local
    } else {
        cloud
    };
    let summary = report_summary_prefix(&selected.source.summary);
    #[cfg(test)]
    report_position_gate_tests::observe_materialization(summary.len());
    Ok(ReadingPosition {
        chapter_uid: selected.chapter.uid.clone(),
        chapter_index: selected.chapter.index,
        chapter_offset: selected.offset,
        progress: selected.progress,
        summary: summary.to_owned(),
        updated_at: selected.source.updated_at,
        device: selected.source.device.clone(),
    })
}

#[cfg(test)]
#[path = "report_position_gate_tests.rs"]
mod report_position_gate_tests;

fn acknowledged_body(value: &Value) -> &Value {
    if value.get("succ").is_some() || value.get("synckey").is_some() {
        value
    } else {
        value
            .get("data")
            .filter(|value| value.is_object())
            .or_else(|| value.get("result").filter(|value| value.is_object()))
            .unwrap_or(value)
    }
}

pub(crate) fn acknowledgement(value: &Value) -> Result<bool> {
    let body = acknowledged_body(value);
    super::client::validate_api(body)?;
    let accepted = body.get("succ").is_some_and(|value| {
        value == true || value.as_u64() == Some(1) || value.as_str() == Some("1")
    }) || body.get("synckey").is_some();
    if accepted {
        Ok(body.get("synckey").is_some())
    } else {
        Err(SourceError::DeliveryUncertain)
    }
}

impl WeReadClient {
    pub async fn report_time(&self, request: &ReadReportRequest) -> Result<ReadReportReceipt> {
        let local = request.validate(&self.session(), protocol::now_seconds())?;
        // All preflight requests finish before either position-writing POST starts.
        let (cloud, catalog) = tokio::try_join!(
            self.progress(&request.book_id),
            self.catalog(&request.book_id)
        )
        .map_err(|error| match error {
            SourceError::ApiKeyRequired
            | SourceError::InvalidResponse("progress_not_found" | "progress_value") => {
                progress_check_required()
            }
            _ => error,
        })?;
        let position = checked_report_position(
            local,
            &cloud,
            &catalog,
            request.active_reader_intent,
            protocol::now_seconds(),
        )?;
        let context = self
            .reader_session(&request.book_id, Some(&position.chapter_uid))
            .await?;
        let now = protocol::now_seconds();
        request.validate(&self.session(), now)?;
        let enter =
            protocol::read_payload_checked(&request.book_id, &position, &context, None, now)?;
        let enter_session = self.session();
        let response = self
            .web_request(Method::POST, "/web/book/read")?
            .header(
                "Referer",
                protocol::reader_url_checked(&request.book_id, Some(&position.chapter_uid))?,
            )
            .json(&enter)
            .send()
            .await
            .map_err(super::client::network_error)?;
        self.observe_web_response(&enter_session, &response)?;
        // Enter carries no elapsed time; a failed enter is not an uncertain time report.
        super::client::response_json(response).await?;
        let payload = protocol::read_payload_checked(
            &request.book_id,
            &position,
            &context,
            Some(request.seconds),
            protocol::now_seconds(),
        )?;
        let report_session = self.session();
        let response = self
            .web_request(Method::POST, "/web/book/read")?
            .header(
                "Referer",
                protocol::reader_url_checked(&request.book_id, Some(&position.chapter_uid))?,
            )
            .json(&payload)
            .send()
            .await
            .map_err(|error| {
                if error.is_connect() {
                    super::client::network_error(error)
                } else {
                    SourceError::DeliveryUncertain
                }
            })?;
        let cookie_result = self.observe_web_response(&report_session, &response);
        finish_elapsed_report(response, cookie_result, request.seconds).await
    }
}

/// Once elapsed time was sent, rejecting response credentials is never proof
/// that the delivery was safe to replay. An explicit receipt still confirms it.
async fn finish_elapsed_report(
    response: reqwest::Response,
    cookie_result: Result<bool>,
    seconds: u32,
) -> Result<ReadReportReceipt> {
    let cookie_rejected = cookie_result.is_err();
    let status = response.status();
    if status.is_server_error() {
        return Err(SourceError::DeliveryUncertain);
    }
    let value = super::client::response_json(response)
        .await
        .map_err(|error| match error {
            _ if cookie_rejected => SourceError::DeliveryUncertain,
            SourceError::Http(_) | SourceError::Api(_) => error,
            _ if status.is_success() => SourceError::DeliveryUncertain,
            _ => error,
        })?;
    let has_synckey = acknowledgement(&value).map_err(|error| {
        if cookie_rejected {
            SourceError::DeliveryUncertain
        } else {
            error
        }
    })?;
    if cookie_rejected {
        crate::diagnostic!(
            "weread: source operation=reading_report delivery=acknowledged cookie_update=rejected_cookie_budget"
        );
    }
    Ok(ReadReportReceipt {
        accepted_at: protocol::now_seconds(),
        seconds,
        has_synckey,
    })
}

#[cfg(test)]
#[path = "cookie_report_tests.rs"]
mod cookie_report_tests;

#[cfg(test)]
mod tests {
    use super::super::content::ReaderSession;
    use super::*;
    use std::collections::BTreeMap;

    fn account() -> AuthSession {
        AuthSession {
            api_key: "fixture-api-key".into(),
            user_vid: "fixture-account".into(),
            cookies: BTreeMap::from([
                ("wr_vid".into(), "fixture-account".into()),
                ("wr_skey".into(), "fixture-session".into()),
            ]),
            ..Default::default()
        }
    }

    fn batch() -> ReadReportRequest {
        ReadReportRequest {
            session_id: "fixture-batch".into(),
            account_key: account_key(&account()).unwrap(),
            book_id: "22691208".into(),
            started_at: 40,
            seconds: 60,
            progress_write_allowed: true,
            active_reader_intent: true,
            position: Some(ReadingPosition {
                chapter_uid: "57".into(),
                chapter_index: 2,
                chapter_offset: 389,
                progress: 74,
                summary: "\u{6444}\u{5f71}\u{7b14}\u{8bb0}".into(),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn report_payload_matches_real_reference_lua_golden_vector() {
        let batch = batch();
        let context = ReaderSession {
            psvts: "ps".into(),
            pclts: Some("pc".into()),
            token: Some("token".into()),
            ..Default::default()
        };
        let position = batch.position.as_ref().unwrap();
        let enter = protocol::read_payload_with_clock(
            &batch.book_id,
            position,
            &context,
            None,
            100,
            100001,
            7,
        );
        let report = protocol::read_payload_with_clock(
            &batch.book_id,
            position,
            &context,
            Some(60),
            100,
            100001,
            7,
        );
        assert_eq!(enter["s"], "68a8379c");
        assert!(enter.get("rt").is_none());
        assert_eq!(report["rt"], 60);
        assert_eq!(
            report["sg"],
            "6539713a2d7132d0cf4527875751c8925502883bb2a2c0ce7bddb2ace509163a"
        );
        assert_eq!(report["s"], "664e23a2");
        assert!(report.get("session_id").is_none());
        assert!(report.get("started_at").is_none());
        assert!(report.get("active_reader_intent").is_none());
    }

    #[test]
    fn unknown_accounts_or_progress_opt_out_do_not_reach_transport() {
        assert!(batch().validate(&AuthSession::default(), 100).is_err());
        let mut batch = batch();
        batch.account_key = "another-account".into();
        assert!(matches!(
            batch.validate(&account(), 100),
            Err(SourceError::Unsupported("report_account_mismatch"))
        ));
        batch.account_key = account_key(&account()).unwrap();
        batch.progress_write_allowed = false;
        assert!(matches!(
            batch.validate(&account(), 100),
            Err(SourceError::Unsupported(
                "time_report_requires_progress_write"
            ))
        ));
        batch.progress_write_allowed = true;
        batch.seconds = 61;
        assert!(batch.validate(&account(), 100).is_err());
    }

    #[test]
    fn reports_require_explicit_acknowledgement_and_classify_uncertainty() {
        assert!(!acknowledgement(&serde_json::json!({"succ":1})).unwrap());
        assert!(acknowledgement(&serde_json::json!({"data":{"synckey":7}})).unwrap());
        assert!(!acknowledgement(&serde_json::json!({"result":{"succ":true}})).unwrap());
        assert!(matches!(
            acknowledgement(&serde_json::json!({"data":{"message":"unknown"}})),
            Err(SourceError::DeliveryUncertain)
        ));
        assert!(matches!(
            acknowledgement(&serde_json::json!({"succ":0})),
            Err(SourceError::Api(_))
        ));
        assert!(!SourceError::DeliveryUncertain.retryable());
    }

    #[test]
    fn conflicting_web_vids_cannot_share_an_outbox() {
        let mut session = account();
        session
            .cookies
            .insert("wr_vid".into(), "another-account".into());
        assert!(account_key(&session).is_none());
    }

    #[test]
    fn missing_reader_clock_is_materialized_once_for_the_enter_read_pair() {
        let mut context = ReaderSession {
            psvts: "ps".into(),
            ..Default::default()
        };
        context.ensure_reader_clock(100).unwrap();
        let batch = batch();
        let position = batch.position.as_ref().unwrap();
        let enter = protocol::read_payload_with_clock(
            &batch.book_id,
            position,
            &context,
            None,
            100,
            100001,
            7,
        );
        context.ensure_reader_clock(101).unwrap();
        let report = protocol::read_payload_with_clock(
            &batch.book_id,
            position,
            &context,
            Some(60),
            101,
            101001,
            7,
        );
        assert_eq!(enter["pc"], report["pc"]);
        assert_eq!(enter["pc"], protocol::encoded_id("100"));
    }

    #[test]
    fn catalog_coordinates_are_weighted_and_unknown_time_keeps_fresh_cloud() {
        let catalog = vec![
            RemoteChapter {
                uid: "short".into(),
                index: 4,
                word_count: 100,
                ..Default::default()
            },
            RemoteChapter {
                uid: "long".into(),
                index: 9,
                word_count: 900,
                ..Default::default()
            },
        ];
        let local = ReadingPosition {
            chapter_uid: "short".into(),
            chapter_index: 999,
            chapter_offset: 90,
            progress: 99,
            ..Default::default()
        };
        let cloud = ReadingPosition {
            chapter_uid: "long".into(),
            chapter_index: 0,
            chapter_offset: 450,
            progress: 1,
            ..Default::default()
        };
        let selected = checked_report_position(&local, &cloud, &catalog, false, 100).unwrap();
        assert_eq!(selected.chapter_uid, "long");
        assert_eq!(selected.chapter_index, 9);
        assert_eq!(selected.progress, 55);
        let selected = checked_report_position(&cloud, &local, &catalog, false, 100).unwrap();
        assert_eq!(selected.chapter_uid, "short");
        assert_eq!(selected.progress, 9);
        let selected = checked_report_position(&local, &cloud, &catalog, true, 100).unwrap();
        assert_eq!(selected.chapter_uid, "short");
        assert_eq!(selected.progress, 9);
    }

    #[test]
    fn unknown_uids_or_missing_word_counts_cannot_invent_a_report_position() {
        let mut catalog = vec![RemoteChapter {
            uid: "chapter".into(),
            index: 4,
            word_count: 100,
            ..Default::default()
        }];
        let known = ReadingPosition {
            chapter_uid: "chapter".into(),
            chapter_index: 4,
            chapter_offset: 50,
            ..Default::default()
        };
        let unknown = ReadingPosition {
            chapter_uid: "unknown".into(),
            chapter_index: 4,
            ..Default::default()
        };
        assert!(matches!(
            checked_report_position(&known, &unknown, &catalog, false, 100),
            Err(SourceError::Unsupported(
                "read_report_progress_check_required"
            ))
        ));
        catalog[0].word_count = 0;
        assert!(checked_report_position(&known, &known, &catalog, false, 100).is_err());
        let mut auth = account();
        auth.api_key.clear();
        assert!(matches!(
            batch().validate(&auth, 100),
            Err(SourceError::Unsupported(
                "read_report_progress_check_required"
            ))
        ));
    }

    #[test]
    fn verified_recency_respects_explicit_backtracking_without_guessing_from_percentages() {
        let catalog = vec![RemoteChapter {
            uid: "chapter".into(),
            index: 4,
            word_count: 1000,
            ..Default::default()
        }];
        let local = ReadingPosition {
            chapter_uid: "chapter".into(),
            chapter_offset: 100,
            updated_at: 95,
            ..Default::default()
        };
        let mut cloud = ReadingPosition {
            chapter_uid: "chapter".into(),
            chapter_offset: 900,
            updated_at: 90,
            ..Default::default()
        };
        assert_eq!(
            checked_report_position(&local, &cloud, &catalog, false, 100)
                .unwrap()
                .chapter_offset,
            100
        );
        cloud.updated_at = 99;
        cloud.chapter_offset = 50;
        assert_eq!(
            checked_report_position(&local, &cloud, &catalog, false, 100)
                .unwrap()
                .chapter_offset,
            50
        );
        cloud.updated_at = 0;
        assert_eq!(
            checked_report_position(&local, &cloud, &catalog, false, 100)
                .unwrap()
                .chapter_offset,
            50
        );
        assert_eq!(
            checked_report_position(&local, &cloud, &catalog, true, 100)
                .unwrap()
                .chapter_offset,
            100
        );
    }

    #[tokio::test]
    async fn missing_progress_api_access_is_rejected_before_any_enter_request() {
        let mut auth = account();
        auth.api_key.clear();
        let client = WeReadClient::new(auth).unwrap();
        assert!(matches!(
            client.report_time(&batch()).await,
            Err(SourceError::Unsupported(
                "read_report_progress_check_required"
            ))
        ));
    }
}
