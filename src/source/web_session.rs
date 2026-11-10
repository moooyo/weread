//! Trusted-origin cookie rotation and bounded recovery for read-only requests.
use super::client::{
    cookie_header, merge_response_cookies, network_error, response_json,
    response_text_limited_observed, success, validate_api,
};
use super::login::auth_budget::{AuthSessionView, CookieValue};
use super::reader_admission::DownloadReaderContext;
use super::{AuthSession, Result, SourceError, WeReadClient};
use crate::download_diagnostics::{DownloadDiagnostics, ResponsePhase};
use reqwest::{Method, Request, RequestBuilder, Response, StatusCode, Url, header::HeaderMap};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;

/// One recovery opportunity shared by all requests making up a logical read.
pub(super) struct WebReadAuth {
    recovery_available: bool,
    pub(super) restart_reader: bool,
    pub(super) recovery_failed: bool,
    pub(super) response_limit: Option<usize>,
    pub(super) download_diagnostics: Option<Arc<DownloadDiagnostics>>,
    pub(super) download_reader: Option<DownloadReaderContext>,
}

impl Default for WebReadAuth {
    fn default() -> Self {
        Self {
            recovery_available: true,
            restart_reader: false,
            recovery_failed: false,
            response_limit: None,
            download_diagnostics: None,
            download_reader: None,
        }
    }
}

pub(super) struct WebTextPair {
    pub(super) result: Result<(String, String)>,
    pub(super) session_changed: bool,
}

struct WebTextAttempt {
    result: Result<String>,
    session_changed: bool,
}

impl WeReadClient {
    pub(super) fn web_origin(&self) -> &str {
        #[cfg(test)]
        if let Some(origin) = &self.test_web_origin {
            return origin;
        }
        "https://weread.qq.com"
    }

    fn trusted_web_url(&self, url: &Url) -> bool {
        Url::parse(self.web_origin()).is_ok_and(|origin| {
            url.origin() == origin.origin() && url.username().is_empty() && url.password().is_none()
        })
    }

    /// Only read-only operations opt into replay. Position/time writes do not.
    pub(super) async fn web_response(
        &self,
        request: RequestBuilder,
        retry_auth: bool,
    ) -> Result<Response> {
        let template = request.build().map_err(network_error)?;
        if !self.trusted_web_url(template.url()) {
            return Err(SourceError::InvalidResponse("web_request_origin"));
        }
        let (sent, renewal_generation) = {
            let renewal = self.renewal.lock().await;
            (self.session(), *renewal)
        };
        let response = self.execute_web(&template, &sent).await?;
        self.observe_web_response(&sent, &response)?;
        if response.status() != StatusCode::UNAUTHORIZED || !retry_auth {
            return Ok(response);
        }
        drop(response);

        // A second rejected request reuses the first request's acknowledged
        // renewal instead of rotating the same session concurrently.
        self.renew_if_unchanged(&sent, renewal_generation).await?;
        let refreshed = self.session();
        let response = self.execute_web(&template, &refreshed).await?;
        self.observe_web_response(&refreshed, &response)?;
        Ok(response)
    }

    pub(super) async fn web_json_read(&self, request: RequestBuilder) -> Result<Value> {
        let mut auth = WebReadAuth::default();
        self.web_read(request, &mut auth, false, |text| {
            let value: Value =
                serde_json::from_str(&text).map_err(|_| SourceError::InvalidResponse("json"))?;
            validate_api(&value)?;
            Ok(value)
        })
        .await
    }

    pub(super) async fn web_text_read(
        &self,
        request: RequestBuilder,
        auth: &mut WebReadAuth,
        restart_reader: bool,
    ) -> Result<String> {
        self.web_read(request, auth, restart_reader, parse_web_text)
            .await
    }

    /// Override this request's response budget without changing the chapter's
    /// shard budget or its shared authentication-recovery allowance.
    pub(super) async fn web_text_read_with_limit(
        &self,
        request: RequestBuilder,
        auth: &mut WebReadAuth,
        restart_reader: bool,
        response_limit: Option<usize>,
    ) -> (Result<String>, bool) {
        let mut response_budget_exceeded = false;
        let result = self
            .web_read_with_limit(
                request,
                auth,
                restart_reader,
                parse_web_text,
                response_limit,
                &mut response_budget_exceeded,
            )
            .await;
        (result, response_budget_exceeded)
    }

    /// Two shards share a snapshot and the logical chapter's recovery budget.
    /// These futures are joined in place, so cancellation leaves no workers.
    pub(super) async fn web_text_pair(
        &self,
        first: RequestBuilder,
        second: RequestBuilder,
        auth: &mut WebReadAuth,
    ) -> Result<WebTextPair> {
        let first = first.build().map_err(network_error)?;
        let second = second.build().map_err(network_error)?;
        if !self.trusted_web_url(first.url()) || !self.trusted_web_url(second.url()) {
            return Err(SourceError::InvalidResponse("web_request_origin"));
        }
        let (sent, generation) = self.web_session_snapshot().await;
        let (first, second) = tokio::join!(
            self.web_text_attempt(
                &first,
                &sent,
                auth.response_limit,
                auth.download_diagnostics.as_ref(),
            ),
            self.web_text_attempt(
                &second,
                &sent,
                auth.response_limit,
                auth.download_diagnostics.as_ref(),
            ),
        );
        let session_changed = first.session_changed
            || second.session_changed
            || web_session_changed(&sent, &self.session());
        let mut result = ordered_text_pair(first.result, second.result);
        if result.as_ref().err().is_some_and(renewable_web_auth_error) && auth.recovery_available {
            auth.recovery_available = false;
            // Both responses have been drained before renewal. A late member
            // of the batch cannot commit over the acknowledged replacement.
            match self.renew_if_unchanged(&sent, generation).await {
                Ok(()) => auth.restart_reader = true,
                Err(error) => {
                    auth.recovery_failed = true;
                    result = Err(error);
                }
            }
        }
        Ok(WebTextPair {
            result,
            session_changed,
        })
    }

    async fn web_text_attempt(
        &self,
        request: &Request,
        sent: &AuthSession,
        response_limit: Option<usize>,
        diagnostics: Option<&Arc<DownloadDiagnostics>>,
    ) -> WebTextAttempt {
        let phase = chapter_response_phase(request.url().path());
        let diagnostics = phase.and(diagnostics);
        let http_read = diagnostics.map(|diagnostics| diagnostics.http_read());
        let response = match self.execute_web(request, sent).await {
            Ok(response) => response,
            Err(error) => {
                return WebTextAttempt {
                    result: Err(error),
                    session_changed: false,
                };
            }
        };
        let status = response.status();
        let headers = response.headers().clone();
        let text =
            response_text_limited_observed(response, response_limit, diagnostics, phase).await;
        drop(http_read);
        let result = text.and_then(parse_web_text);
        if let Err(SourceError::Api(_)) = &result {
            let error = result
                .as_ref()
                .err()
                .expect("A rejected shard has an error");
            crate::diagnostic!(
                "weread: source operation=chapter_shard status={} error={error}",
                status.as_u16()
            );
        }
        let may_commit = !result
            .as_ref()
            .err()
            .is_some_and(SourceError::requires_login);
        let session_changed = if may_commit {
            match self.observe_web_headers(sent, request.url(), status, &headers) {
                Ok(changed) => changed,
                Err(error) => {
                    return WebTextAttempt {
                        result: Err(error),
                        session_changed: false,
                    };
                }
            }
        } else {
            false
        };
        WebTextAttempt {
            result,
            session_changed,
        }
    }

    async fn web_read<T, F>(
        &self,
        request: RequestBuilder,
        auth: &mut WebReadAuth,
        restart_reader: bool,
        parse: F,
    ) -> Result<T>
    where
        T: Send,
        F: Fn(String) -> Result<T> + Send,
    {
        let response_limit = auth.response_limit;
        let mut response_budget_exceeded = false;
        self.web_read_with_limit(
            request,
            auth,
            restart_reader,
            parse,
            response_limit,
            &mut response_budget_exceeded,
        )
        .await
    }

    async fn web_read_with_limit<T, F>(
        &self,
        request: RequestBuilder,
        auth: &mut WebReadAuth,
        restart_reader: bool,
        parse: F,
        response_limit: Option<usize>,
        response_budget_exceeded: &mut bool,
    ) -> Result<T>
    where
        T: Send,
        F: Fn(String) -> Result<T> + Send,
    {
        *response_budget_exceeded = false;
        let template = request.build().map_err(network_error)?;
        if !self.trusted_web_url(template.url()) {
            return Err(SourceError::InvalidResponse("web_request_origin"));
        }
        let (mut sent, mut renewal_generation) = self.web_session_snapshot().await;
        loop {
            let phase = chapter_response_phase(template.url().path());
            let diagnostics = phase.and(auth.download_diagnostics.as_ref());
            let http_read = diagnostics.map(|diagnostics| diagnostics.http_read());
            let response = self.execute_web(&template, &sent).await?;
            let status = response.status();
            let headers = response.headers().clone();
            let text =
                response_text_limited_observed(response, response_limit, diagnostics, phase).await;
            drop(http_read);
            // Only transport admission selects adaptive Reader acquisition.
            // Parser, Cookie and renewal budget errors retain their own cause.
            *response_budget_exceeded = matches!(&text, Err(SourceError::ResponseTooLarge));
            let result = text.and_then(&parse);
            if result.as_ref().err().is_some_and(renewable_web_auth_error) {
                let error = result
                    .as_ref()
                    .err()
                    .expect("The rejected read has an error");
                crate::diagnostic!(
                    "weread: source operation={} status={} error={error}",
                    web_operation(template.url().path()),
                    status.as_u16()
                );
                if !auth.recovery_available {
                    return result;
                }
                auth.recovery_available = false;
                // Rejected envelopes do not get to clear refresh material.
                // The renewal endpoint alone commits acknowledged replacements.
                if let Err(error) = self.renew_if_unchanged(&sent, renewal_generation).await {
                    auth.recovery_failed = true;
                    return Err(error);
                }
                if restart_reader {
                    auth.restart_reader = true;
                    return result;
                }
                (sent, renewal_generation) = self.web_session_snapshot().await;
                continue;
            }
            if let Err(SourceError::Api(_)) = &result {
                let error = result
                    .as_ref()
                    .err()
                    .expect("The rejected read has an error");
                crate::diagnostic!(
                    "weread: source operation={} status={} error={error}",
                    web_operation(template.url().path()),
                    status.as_u16()
                );
            }
            if !result
                .as_ref()
                .err()
                .is_some_and(SourceError::requires_login)
            {
                if let Err(error) =
                    self.observe_web_headers(&sent, template.url(), status, &headers)
                {
                    *response_budget_exceeded = false;
                    return Err(error);
                }
            }
            return result;
        }
    }

    async fn web_session_snapshot(&self) -> (AuthSession, u64) {
        let renewal = self.renewal.lock().await;
        (self.session(), *renewal)
    }

    async fn renew_if_unchanged(&self, sent: &AuthSession, generation: u64) -> Result<()> {
        let mut renewal = self.renewal.lock().await;
        let current = self.session();
        if current.user_vid != sent.user_vid || current.api_key != sent.api_key {
            return Err(SourceError::Unsupported("renewal_account_changed"));
        }
        if *renewal == generation && same_web_credentials(sent, &current) {
            self.renew_locked().await?;
            *renewal = renewal.wrapping_add(1);
        }
        Ok(())
    }

    async fn execute_web(&self, template: &Request, session: &AuthSession) -> Result<Response> {
        if !session.has_web_session() {
            return Err(SourceError::WebLoginRequired);
        }
        let mut request = template
            .try_clone()
            .ok_or(SourceError::Unsupported("non_replayable_request"))?;
        let headers = request.headers_mut();
        headers.insert(
            reqwest::header::COOKIE,
            cookie_header(&session.cookies)?
                .parse()
                .map_err(|_| SourceError::InvalidResponse("web_cookie"))?,
        );
        for (name, value) in [
            ("x-wr-ticket", &session.wr_ticket),
            ("x-wrpa-0", &session.wr_wrpa),
        ] {
            headers.remove(name);
            if !value.is_empty() {
                headers.insert(
                    name,
                    value
                        .parse()
                        .map_err(|_| SourceError::InvalidResponse("web_header"))?,
                );
            }
        }
        self.http.execute(request).await.map_err(network_error)
    }

    pub(super) fn observe_web_response(
        &self,
        sent: &AuthSession,
        response: &Response,
    ) -> Result<bool> {
        self.observe_web_headers(sent, response.url(), response.status(), response.headers())
    }

    fn observe_web_headers(
        &self,
        sent: &AuthSession,
        url: &Url,
        status: StatusCode,
        headers: &HeaderMap,
    ) -> Result<bool> {
        if !self.trusted_web_url(url) {
            return Ok(false);
        }
        if !status.is_success() {
            crate::diagnostic!(
                "weread: source operation={} status={}",
                web_operation(url.path()),
                status.as_u16()
            );
        }
        // A rejected request must retain its refresh material until the
        // renewal endpoint explicitly acknowledges a replacement session.
        if status != StatusCode::UNAUTHORIZED {
            // An obsolete reply cannot alter or invalidate the replacement
            // account. A parallel read still treats its context as changed.
            return Ok(self
                .commit_web_headers_inner(sent, headers, false)?
                .unwrap_or(true));
        }
        Ok(false)
    }

    fn commit_web_headers(
        &self,
        sent: &AuthSession,
        headers: &HeaderMap,
        authoritative: bool,
    ) -> Result<bool> {
        Ok(self
            .commit_web_headers_inner(sent, headers, authoritative)?
            .is_some())
    }

    /// None means an obsolete reply was ignored before inspecting its updates.
    /// Some reports whether the committed snapshot differs from the sent one.
    fn commit_web_headers_inner(
        &self,
        sent: &AuthSession,
        headers: &HeaderMap,
        authoritative: bool,
    ) -> Result<Option<bool>> {
        let mut current = self
            .auth
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current.user_vid != sent.user_vid || current.api_key != sent.api_key {
            return Ok(None);
        }
        if !authoritative && !same_web_credentials(&current, sent) {
            return Ok(None);
        }
        let mut updated = sent.cookies.clone();
        merge_response_cookies(&mut updated, headers)?;
        let mut proposal = AuthSessionView::from_session(&current)?;
        let names = sent
            .cookies
            .keys()
            .chain(updated.keys())
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        for name in names {
            if updated.get(name) == sent.cookies.get(name)
                || (!authoritative && current.cookies.get(name) != sent.cookies.get(name))
            {
                continue;
            }
            match updated.get(name) {
                Some(value) => {
                    proposal.cookies.set(name, CookieValue::Text(value));
                }
                None => {
                    proposal.cookies.remove(name);
                }
            }
        }
        for (name, current_value, sent_value) in [
            (
                "x-wr-ticket",
                &mut proposal.wr_ticket,
                sent.wr_ticket.as_str(),
            ),
            ("x-wrpa-0", &mut proposal.wr_wrpa, sent.wr_wrpa.as_str()),
        ] {
            if let Some(value) = headers
                .get(name)
                .and_then(|header| header.to_str().ok())
                .filter(|value| !value.is_empty())
            {
                if authoritative || *current_value == sent_value {
                    *current_value = value;
                }
            }
        }
        // A concurrent reply may already have added other valid cookie names.
        // Check cookies and the complete encoded proposal before copying any
        // selected account field or changing current cookies/ticket fields.
        let candidate = proposal.admit()?.into_owned();
        let changed = web_session_changed(sent, &candidate);
        *current = candidate;
        #[cfg(test)]
        super::login::auth_budget::observe_adoption();
        Ok(Some(changed))
    }

    pub async fn renew(&self) -> Result<AuthSession> {
        let mut renewal = self.renewal.lock().await;
        let session = self.renew_locked().await?;
        *renewal = renewal.wrapping_add(1);
        Ok(session)
    }

    async fn renew_locked(&self) -> Result<AuthSession> {
        let sent = self.session();
        let request = self
            .web_request(Method::POST, "/web/login/renewal")?
            .json(&json!({"rq":"%2Fweb%2Fbook%2Fread","ql":false}))
            .build()
            .map_err(network_error)?;
        let response = self.execute_web(&request, &sent).await?;
        if !response.status().is_success() {
            crate::diagnostic!(
                "weread: source operation=renewal status={}",
                response.status().as_u16()
            );
        }
        let headers = response.headers().clone();
        let result = response_json(response).await?;
        if !success(&result) {
            return Err(SourceError::InvalidResponse("renewal_acknowledgement"));
        }
        if !self.commit_web_headers(&sent, &headers, true)? {
            return Err(SourceError::Unsupported("renewal_account_changed"));
        }
        crate::diagnostic!("weread: source operation=renewal result=acknowledged");
        Ok(self.session())
    }
}

fn parse_web_text(text: String) -> Result<String> {
    // HTML and encoded shards stay opaque. Only an actual JSON object can
    // carry the official Web session error envelope.
    if text.trim_start().starts_with('{') {
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            validate_api(&value)?;
        }
    }
    Ok(text)
}

fn ordered_text_pair(first: Result<String>, second: Result<String>) -> Result<(String, String)> {
    match (first, second) {
        (Err(error @ SourceError::InvalidResponse("cookie_budget")), _)
        | (_, Err(error @ SourceError::InvalidResponse("cookie_budget")))
        | (Err(error @ SourceError::ResponseTooLarge), _)
        | (_, Err(error @ SourceError::ResponseTooLarge)) => Err(error),
        (Ok(first), Ok(second)) => Ok((first, second)),
        (Err(first), Err(second)) => {
            // An identity-only failure must not become renewable just because
            // its peer happened to return a recoverable expiry code.
            let terminal_identity =
                |error: &SourceError| error.requires_login() && !renewable_web_auth_error(error);
            if terminal_identity(&second)
                || (!terminal_identity(&first) && renewable_web_auth_error(&second))
            {
                Err(second)
            } else {
                Err(first)
            }
        }
        (Err(error), Ok(_)) | (Ok(_), Err(error)) => Err(error),
    }
}

pub(super) fn web_session_changed(left: &AuthSession, right: &AuthSession) -> bool {
    left.api_key != right.api_key
        || left.cookies != right.cookies
        || !same_web_credentials(left, right)
}

fn renewable_web_auth_error(error: &SourceError) -> bool {
    matches!(error, SourceError::Http(401))
        || matches!(error, SourceError::Api(code) if matches!(code.as_str(), "-2010" | "-2012"))
}

fn same_web_credentials(left: &AuthSession, right: &AuthSession) -> bool {
    left.user_vid == right.user_vid
        && left.wr_ticket == right.wr_ticket
        && left.wr_wrpa == right.wr_wrpa
        && ["wr_vid", "wr_skey", "wr_gid", "wr_rt"]
            .iter()
            .all(|name| left.cookies.get(*name) == right.cookies.get(*name))
}

fn chapter_response_phase(path: &str) -> Option<ResponsePhase> {
    if path.starts_with("/web/reader/") {
        Some(ResponsePhase::Reader)
    } else if path.starts_with("/web/book/chapter/") {
        Some(ResponsePhase::Shard)
    } else {
        None
    }
}

fn web_operation(path: &str) -> &'static str {
    match path {
        "/web/book/chapterInfos" => "catalog",
        "/web/login/renewal" => "renewal",
        "/web/book/read" => "reading_report",
        "/web/mp/articles" => "article_list",
        "/web/mp/content" => "article_content",
        path if path.starts_with("/web/reader/") => "reader_session",
        path if path.starts_with("/web/book/chapter/") => "chapter_shard",
        _ => "web_resource",
    }
}

#[cfg(test)]
#[path = "web_session_tests.rs"]
mod tests;
