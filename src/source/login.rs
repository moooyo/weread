use super::client::{
    MAX_COOKIE_ENTRY_BYTES, ScalarText, cookie_header, http_client, merge_response_cookies,
    network_error, response_json, response_text, scalar_text_at, validate_cookie_jar,
    validate_cookie_overlay,
};
use super::protocol::{encode_component, now_seconds};
use super::{AuthSession, QrLoginChallenge, QrLoginState, Result, SourceError};
use reqwest::{Client, Response};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

#[path = "auth_budget.rs"]
pub(crate) mod auth_budget;

use auth_budget::{AuthSessionView, BorrowedCookies, CookieValue};

const ORIGIN: &str = "https://weread.qq.com";
const SKILLS_PAGE: &str = "https://weread.qq.com/r/weread-skills";
const LOGIN_LIFETIME_SECONDS: i64 = 300;
const CONFIRM_PATH: &str = "/web/confirm?uid=";
// A QR exchange retains a short opaque identifier, not document content. Bound
// both the raw identifier and the full QR payload before copying or encoding;
// these application allowances do not prescribe an upstream identifier format.
const MAX_LOGIN_UID_BYTES: usize = 1024;
const MAX_LOGIN_CONFIRM_URL_BYTES: usize = 2 * 1024;

fn checked_login_uid(value: &Value, origin: &str) -> Result<String> {
    let uid = match value.get("uid") {
        Some(Value::String(uid)) => Cow::Borrowed(uid.as_str()),
        // Preserve numeric identifiers. Serde's non-arbitrary-precision Number
        // has a bounded representation, unlike an untrusted string field.
        Some(Value::Number(uid)) => Cow::Owned(uid.to_string()),
        _ => return Err(SourceError::InvalidResponse("login_uid")),
    };
    if uid.is_empty() {
        return Err(SourceError::InvalidResponse("login_uid"));
    }
    let budget_error = || SourceError::InvalidResponse("login_uid_budget");
    if uid.len() > MAX_LOGIN_UID_BYTES {
        return Err(budget_error());
    }
    let prefix_bytes = origin
        .len()
        .checked_add(CONFIRM_PATH.len())
        .filter(|length| *length <= MAX_LOGIN_CONFIRM_URL_BYTES)
        .ok_or_else(budget_error)?;
    // Match protocol::encode_component byte-for-byte, including Unicode's
    // UTF-8 bytes, without allocating the expanded URL merely to measure it.
    uid.bytes().try_fold(prefix_bytes, |length, byte| {
        let encoded_bytes =
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                1
            } else {
                3
            };
        length
            .checked_add(encoded_bytes)
            .filter(|length| *length <= MAX_LOGIN_CONFIRM_URL_BYTES)
            .ok_or_else(budget_error)
    })?;
    Ok(uid.into_owned())
}

struct LoginCredentials<'a> {
    user_vid: ScalarText<'a>,
    access_token: ScalarText<'a>,
    refresh_token: ScalarText<'a>,
}

fn checked_login_credentials<'a>(
    result: &'a Value,
    cookies: &BTreeMap<String, String>,
) -> Result<LoginCredentials<'a>> {
    let user_vid = scalar_text_at(result, &["webLoginVid"]);
    let access_token = scalar_text_at(result, &["accessToken"]);
    if user_vid.is_empty() || access_token.is_empty() {
        return Err(SourceError::InvalidResponse("login_credentials"));
    }
    // Preserve the original first overlay gate. A later refresh replacement
    // cannot repair an earlier rejected VID/access/ql snapshot.
    validate_cookie_overlay(
        cookies,
        &[
            ("wr_vid", user_vid.len()),
            ("wr_skey", access_token.len()),
            ("wr_ql", 1),
        ],
    )?;
    let refresh_token = scalar_text_at(result, &["refreshToken"]);
    if !refresh_token.is_empty() {
        if refresh_token.len() > MAX_COOKIE_ENTRY_BYTES {
            return Err(SourceError::InvalidResponse("cookie_budget"));
        }
        let encoded_bytes = refresh_token.bytes().try_fold(0usize, |bytes, byte| {
            bytes
                .checked_add(
                    if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                        1
                    } else {
                        3
                    },
                )
                .ok_or(SourceError::InvalidResponse("cookie_budget"))
        })?;
        validate_cookie_overlay(
            cookies,
            &[
                ("wr_vid", user_vid.len()),
                ("wr_skey", access_token.len()),
                ("wr_ql", 1),
                ("wr_rt", encoded_bytes),
            ],
        )?;
    }
    Ok(LoginCredentials {
        user_vid,
        access_token,
        refresh_token,
    })
}

fn copy_login_credential(value: ScalarText<'_>) -> String {
    #[cfg(test)]
    login_credential_gate_tests::observe_credential_copy(value.len());
    value.into_owned()
}

fn copy_login_cookie(value: &str) -> String {
    #[cfg(test)]
    login_credential_gate_tests::observe_credential_copy(value.len());
    value.to_owned()
}

#[derive(Clone, Default)]
struct FlowState {
    cookies: BTreeMap<String, String>,
    challenge: Option<QrLoginChallenge>,
}

/// Ephemeral QR exchange. It never writes credentials or changes application state.
#[derive(Clone)]
pub struct LoginFlow {
    http: Client,
    state: Arc<Mutex<FlowState>>,
    generation: Arc<AtomicU64>,
    #[cfg(test)]
    test_origin: Option<String>,
}

impl LoginFlow {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: http_client()?,
            state: Arc::new(Mutex::new(FlowState::default())),
            generation: Arc::new(AtomicU64::new(0)),
            #[cfg(test)]
            test_origin: None,
        })
    }

    fn origin(&self) -> &str {
        #[cfg(test)]
        if let Some(origin) = self.test_origin.as_deref() {
            return origin;
        }
        ORIGIN
    }

    fn trusted_login_url(&self, url: &reqwest::Url) -> bool {
        #[cfg(test)]
        if let Some(origin) = self.test_origin.as_deref() {
            return reqwest::Url::parse(origin).is_ok_and(|origin| url.origin() == origin.origin());
        }
        url.scheme() == "https"
            && url.host_str() == Some("weread.qq.com")
            && url.port_or_known_default() == Some(443)
    }

    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        *self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = FlowState::default();
    }

    pub async fn begin(&self) -> Result<QrLoginChallenge> {
        self.cancel();
        let generation = self.generation.load(Ordering::SeqCst);
        let mut cookies = BTreeMap::new();
        let skills_page = format!("{}/r/weread-skills", self.origin());
        let (response, updated) = self
            .get_with_cookies(&skills_page, &cookies, None, None)
            .await?;
        cookies = updated;
        let _ = response_text(response).await?;
        let (response, updated) = self
            .get_with_cookies(
                &format!("{}/api/auth/getLoginUid", self.origin()),
                &cookies,
                None,
                None,
            )
            .await?;
        cookies = updated;
        let value = response_json(response).await?;
        let uid = checked_login_uid(&value, self.origin())?;
        let challenge = QrLoginChallenge {
            confirm_url: format!("{}{CONFIRM_PATH}{}", self.origin(), encode_component(&uid)),
            uid,
            expires_at: now_seconds() + LOGIN_LIFETIME_SECONDS,
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if generation != self.generation.load(Ordering::SeqCst) {
            return Err(SourceError::Unsupported("login_cancelled"));
        }
        *state = FlowState {
            cookies,
            challenge: Some(challenge.clone()),
        };
        Ok(challenge)
    }

    pub async fn poll(&self, uid: &str, otp: Option<&str>) -> Result<QrLoginState> {
        let generation = self.generation.load(Ordering::SeqCst);
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let Some(challenge) = state
            .challenge
            .as_ref()
            .filter(|challenge| challenge.uid == uid)
        else {
            return Ok(QrLoginState::Expired);
        };
        if now_seconds() >= challenge.expires_at {
            return Ok(QrLoginState::Expired);
        }
        if otp.is_some_and(|code| code.len() != 4 || !code.bytes().all(|b| b.is_ascii_digit())) {
            return Ok(QrLoginState::CodeMismatch);
        }
        let mut url = format!(
            "{}/api/auth/getLoginInfo?uid={}&otp",
            self.origin(),
            encode_component(uid)
        );
        if let Some(code) = otp.filter(|code| !code.is_empty()) {
            url.push('=');
            url.push_str(&encode_component(code));
        }
        let request = self
            .http
            .get(url)
            .header("Accept", "application/json, text/plain, */*")
            .header("Referer", SKILLS_PAGE)
            .header("Cookie", cookie_header(&state.cookies)?)
            .timeout(Duration::from_secs(8));
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) if error.is_timeout() => return Ok(QrLoginState::Pending),
            Err(error) => return Err(network_error(error)),
        };
        let mut cookies = state.cookies;
        merge_response_cookies(&mut cookies, response.headers())?;
        let result = match response_json(response).await {
            Err(SourceError::Timeout) => return Ok(QrLoginState::Pending),
            result => result?,
        };
        if generation != self.generation.load(Ordering::SeqCst) {
            return Ok(QrLoginState::Expired);
        }
        if result.get("succeed") == Some(&Value::Bool(true)) {
            match self.complete(&result, cookies.clone(), generation).await {
                Ok(session) => {
                    self.commit_poll_cookies(generation, cookies)?;
                    return Ok(QrLoginState::Authenticated(session));
                }
                Err(error) => {
                    if !matches!(
                        error,
                        SourceError::InvalidResponse("cookie_budget")
                            | SourceError::ResponseTooLarge
                    ) {
                        // Preserve the established retry state for ordinary
                        // login failures. A resource-budget rejection alone
                        // rolls the entire attempted authentication back.
                        self.commit_poll_cookies(generation, cookies)?;
                    }
                    return Err(error);
                }
            }
        }
        self.commit_poll_cookies(generation, cookies)?;
        match result
            .get("logicCode")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "NEED_OTP" => Ok(QrLoginState::NeedCode),
            "LOGIN_TIMEOUT" | "OTP_EXPIRED" => Ok(QrLoginState::Expired),
            "OTP_NOT_MATCH" => Ok(QrLoginState::CodeMismatch),
            "" => Ok(QrLoginState::Pending),
            _ => Err(SourceError::InvalidResponse("login_state")),
        }
    }

    fn commit_poll_cookies(
        &self,
        generation: u64,
        cookies: BTreeMap<String, String>,
    ) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if generation != self.generation.load(Ordering::SeqCst) {
            return Err(SourceError::Unsupported("login_cancelled"));
        }
        validate_cookie_jar(&cookies)?;
        state.cookies = cookies;
        Ok(())
    }

    async fn complete(
        &self,
        result: &Value,
        mut cookies: BTreeMap<String, String>,
        generation: u64,
    ) -> Result<AuthSession> {
        if generation != self.generation.load(Ordering::SeqCst) {
            return Err(SourceError::Unsupported("login_cancelled"));
        }
        let credentials = checked_login_credentials(result, &cookies)?;
        {
            let mut proposed_cookies = BorrowedCookies::from_jar(&cookies)?;
            proposed_cookies.set("wr_vid", CookieValue::Text(credentials.user_vid.as_str()));
            proposed_cookies.set(
                "wr_skey",
                CookieValue::Text(credentials.access_token.as_str()),
            );
            proposed_cookies.set("wr_ql", CookieValue::Text("0"));
            if !credentials.refresh_token.is_empty() {
                proposed_cookies.set(
                    "wr_rt",
                    CookieValue::PercentEncoded(credentials.refresh_token.as_str()),
                );
            }
            // Admit all known fields before credential copying or refresh
            // encoding. Later responses must pass again with their own fields.
            AuthSessionView::login(
                proposed_cookies,
                credentials.user_vid.as_str(),
                now_seconds(),
            )
            .admit()?;
        }
        let user_vid = copy_login_credential(credentials.user_vid);
        let access_token = copy_login_credential(credentials.access_token);
        cookies.insert("wr_vid".into(), copy_login_cookie(&user_vid));
        cookies.insert("wr_skey".into(), copy_login_cookie(&access_token));
        cookies.insert("wr_ql".into(), "0".into());
        validate_cookie_jar(&cookies)?;
        if !credentials.refresh_token.is_empty() {
            #[cfg(test)]
            login_credential_gate_tests::observe_refresh_encoding(credentials.refresh_token.len());
            cookies.insert("wr_rt".into(), encode_component(&credentials.refresh_token));
        }
        validate_cookie_jar(&cookies)?;
        let user_url = format!(
            "{}/api/userInfo?userVid={}",
            self.origin(),
            encode_component(&user_vid)
        );
        let user_info = self
            .authenticated_json(&user_url, &mut cookies, &user_vid, &access_token)
            .await?;
        if generation != self.generation.load(Ordering::SeqCst) {
            return Err(SourceError::Unsupported("login_cancelled"));
        }
        let account_name = scalar_text_at(&user_info, &["name"]);
        {
            let mut proposal = AuthSessionView::login(
                BorrowedCookies::from_jar(&cookies)?,
                &user_vid,
                now_seconds(),
            );
            proposal.account_name = account_name.as_str();
            // A large account name never needs to be copied or followed by
            // another authenticated GET merely to establish its capacity.
            proposal.admit()?;
        }
        for attempt in 0..3 {
            if generation != self.generation.load(Ordering::SeqCst) {
                return Err(SourceError::Unsupported("login_cancelled"));
            }
            let value = self
                .authenticated_json(
                    &format!("{}/api/skills/apikeyGet?only_show=1", self.origin()),
                    &mut cookies,
                    &user_vid,
                    &access_token,
                )
                .await?;
            if generation != self.generation.load(Ordering::SeqCst) {
                return Err(SourceError::Unsupported("login_cancelled"));
            }
            let api_key = scalar_text_at(&value, &["apikey"]);
            if !api_key.is_empty() {
                let mut proposal = AuthSessionView::login(
                    BorrowedCookies::from_jar(&cookies)?,
                    &user_vid,
                    now_seconds(),
                );
                proposal.api_key = api_key.as_str();
                proposal.account_name = account_name.as_str();
                // Both selected response fields remain borrowed through the
                // exact encoded forecast. Only an admitted session can queue.
                return Ok(proposal.admit()?.into_owned());
            }
            if attempt < 2 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        Err(SourceError::Unsupported("skill_not_enabled"))
    }

    async fn authenticated_json(
        &self,
        url: &str,
        cookies: &mut BTreeMap<String, String>,
        user_vid: &str,
        access_token: &str,
    ) -> Result<Value> {
        for attempt in 0..3 {
            let (response, updated) = self
                .get_with_cookies(url, cookies, Some(user_vid), Some(access_token))
                .await?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED && attempt < 2 {
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
            let value = response_json(response).await?;
            *cookies = updated;
            return Ok(value);
        }
        Err(SourceError::Http(401))
    }

    async fn get_with_cookies(
        &self,
        url: &str,
        cookies: &BTreeMap<String, String>,
        user_vid: Option<&str>,
        access_token: Option<&str>,
    ) -> Result<(Response, BTreeMap<String, String>)> {
        let mut current =
            reqwest::Url::parse(url).map_err(|_| SourceError::InvalidResponse("login_url"))?;
        let mut cookies = cookies.clone();
        for _ in 0..=5 {
            if !self.trusted_login_url(&current) {
                return Err(SourceError::InvalidResponse("login_redirect_origin"));
            }
            let mut request = self
                .http
                .get(current.clone())
                .header("Accept", "application/json, text/plain, */*")
                .header("Referer", SKILLS_PAGE);
            if !cookies.is_empty() {
                request = request.header("Cookie", cookie_header(&cookies)?);
            }
            if let Some(vid) = user_vid {
                request = request.header("X-Vid", vid);
            }
            if let Some(token) = access_token {
                request = request.header("X-Skey", token);
            }
            let response = request.send().await.map_err(network_error)?;
            if response.status().is_redirection() {
                merge_response_cookies(&mut cookies, response.headers())?;
                let next = response
                    .headers()
                    .get("Location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or(SourceError::InvalidResponse("login_redirect"))?;
                current = current
                    .join(next)
                    .map_err(|_| SourceError::InvalidResponse("login_redirect"))?;
                continue;
            }
            merge_response_cookies(&mut cookies, response.headers())?;
            return Ok((response, cookies));
        }
        Err(SourceError::InvalidResponse("too_many_redirects"))
    }
}

#[cfg(test)]
#[path = "login_budget_tests.rs"]
mod login_budget_tests;

#[cfg(test)]
#[path = "login_credential_gate_tests.rs"]
mod login_credential_gate_tests;

#[cfg(test)]
#[path = "login_session_budget_tests.rs"]
mod login_session_budget_tests;
