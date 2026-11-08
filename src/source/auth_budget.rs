//! Borrowed admission for the existing encoded account-file resource policy.

use crate::source::client::{
    MAX_COOKIE_COUNT, MAX_COOKIE_ENTRY_BYTES, MAX_COOKIE_TOTAL_BYTES, validate_cookie_jar,
};
use crate::source::{AuthSession, Result, SourceError, protocol};
use serde::{Serialize, Serializer, ser::SerializeMap};
use std::{collections::BTreeMap, fmt, io};

#[derive(Clone, Copy)]
pub(crate) enum CookieValue<'a> {
    Text(&'a str),
    PercentEncoded(&'a str),
}

impl CookieValue<'_> {
    fn encoded_len(self) -> Result<usize> {
        match self {
            Self::Text(value) => Ok(value.len()),
            Self::PercentEncoded(value) => {
                if value.len() > MAX_COOKIE_ENTRY_BYTES {
                    return Err(SourceError::InvalidResponse("cookie_budget"));
                }
                value.bytes().try_fold(0usize, |bytes, byte| {
                    bytes
                        .checked_add(if component_byte_is_literal(byte) {
                            1
                        } else {
                            3
                        })
                        .ok_or(SourceError::InvalidResponse("cookie_budget"))
                })
            }
        }
    }

    fn into_owned(self) -> String {
        match self {
            Self::Text(value) => value.to_owned(),
            Self::PercentEncoded(value) => protocol::encode_component(value),
        }
    }
}

fn component_byte_is_literal(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
}

struct EncodedComponent<'a>(&'a str);

impl fmt::Display for EncodedComponent<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0.bytes() {
            if component_byte_is_literal(byte) {
                write!(formatter, "{}", char::from(byte))?;
            } else {
                write!(formatter, "%{byte:02X}")?;
            }
        }
        Ok(())
    }
}

impl Serialize for CookieValue<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Text(value) => serializer.serialize_str(value),
            Self::PercentEncoded(value) => serializer.collect_str(&EncodedComponent(value)),
        }
    }
}

/// Only references to an already admitted jar and selected replacement values.
/// Fixed login overlays and response CAS use the same sorted map semantics.
pub(crate) struct BorrowedCookies<'a> {
    entries: BTreeMap<&'a str, CookieValue<'a>>,
}

impl<'a> BorrowedCookies<'a> {
    pub(crate) fn from_jar(cookies: &'a BTreeMap<String, String>) -> Result<Self> {
        validate_cookie_jar(cookies)?;
        Ok(Self::borrow_valid_jar(cookies))
    }

    fn borrow_valid_jar(cookies: &'a BTreeMap<String, String>) -> Self {
        Self {
            entries: cookies
                .iter()
                .map(|(name, value)| (name.as_str(), CookieValue::Text(value.as_str())))
                .collect(),
        }
    }

    pub(crate) fn set(&mut self, name: &'a str, value: CookieValue<'a>) {
        self.entries.insert(name, value);
    }

    pub(crate) fn remove(&mut self, name: &str) {
        self.entries.remove(name);
    }

    fn validate(&self) -> Result<()> {
        let error = || SourceError::InvalidResponse("cookie_budget");
        if self.entries.len() > MAX_COOKIE_COUNT {
            return Err(error());
        }
        let mut bytes = self
            .entries
            .len()
            .saturating_sub(1)
            .checked_mul(2)
            .ok_or_else(error)?;
        for (name, value) in &self.entries {
            let value_bytes = value.encoded_len()?;
            let entry = name
                .len()
                .checked_add(1)
                .and_then(|bytes| bytes.checked_add(value_bytes))
                .filter(|bytes| *bytes <= MAX_COOKIE_ENTRY_BYTES)
                .ok_or_else(error)?;
            bytes = bytes.checked_add(entry).ok_or_else(error)?;
            if bytes > MAX_COOKIE_TOTAL_BYTES {
                return Err(error());
            }
        }
        Ok(())
    }
}

impl Serialize for BorrowedCookies<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.entries.len()))?;
        for (name, value) in &self.entries {
            map.serialize_entry(name, value)?;
        }
        map.end()
    }
}

/// Field order, names and string encoding match AuthSession's Serialize derive.
/// No selected payload is copied merely to prove this proposal is admissible.
#[derive(Serialize)]
pub(crate) struct AuthSessionView<'a> {
    pub(crate) api_key: &'a str,
    pub(crate) cookies: BorrowedCookies<'a>,
    pub(crate) account_name: &'a str,
    pub(crate) user_vid: &'a str,
    pub(crate) login_time: i64,
    pub(crate) wr_ticket: &'a str,
    pub(crate) wr_wrpa: &'a str,
}

impl<'a> AuthSessionView<'a> {
    pub(crate) fn from_session(session: &'a AuthSession) -> Result<Self> {
        session.validate_cookie_budget()?;
        Ok(Self {
            api_key: &session.api_key,
            cookies: BorrowedCookies::borrow_valid_jar(&session.cookies),
            account_name: &session.account_name,
            user_vid: &session.user_vid,
            login_time: session.login_time,
            wr_ticket: &session.wr_ticket,
            wr_wrpa: &session.wr_wrpa,
        })
    }

    pub(crate) fn login(cookies: BorrowedCookies<'a>, user_vid: &'a str, login_time: i64) -> Self {
        Self {
            api_key: "",
            cookies,
            account_name: "",
            user_vid,
            login_time,
            wr_ticket: "",
            wr_wrpa: "",
        }
    }

    pub(crate) fn admit(self) -> Result<AdmittedAuthSession<'a>> {
        #[cfg(test)]
        observe_work(|work| work.preflights += 1);
        self.cookies.validate()?;
        let mut field_bytes = 0usize;
        for field in [
            self.api_key,
            self.account_name,
            self.user_vid,
            self.wr_ticket,
            self.wr_wrpa,
        ] {
            field_bytes = field_bytes
                .checked_add(field.len())
                .filter(|bytes| *bytes <= AuthSession::MAX_ENCODED_BYTES)
                .ok_or(SourceError::ResponseTooLarge)?;
        }
        let mut counter = EncodedCounter::new(AuthSession::MAX_ENCODED_BYTES);
        if serde_json::to_writer(&mut counter, &self).is_err() {
            return Err(if counter.exceeded {
                SourceError::ResponseTooLarge
            } else {
                SourceError::InvalidResponse("auth_session_encoding")
            });
        }
        #[cfg(test)]
        observe_work(|work| {
            work.admissions += 1;
            work.encoded_bytes += counter.bytes;
        });
        Ok(AdmittedAuthSession(self))
    }
}

/// This token can only be constructed after the complete borrowed proposal passes.
pub(crate) struct AdmittedAuthSession<'a>(AuthSessionView<'a>);

impl AdmittedAuthSession<'_> {
    pub(crate) fn into_owned(self) -> AuthSession {
        let view = self.0;
        #[cfg(test)]
        observe_work(|work| work.materializations += 1);
        AuthSession {
            api_key: copy_field(view.api_key),
            cookies: view
                .cookies
                .entries
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.into_owned()))
                .collect(),
            account_name: copy_field(view.account_name),
            user_vid: copy_field(view.user_vid),
            login_time: view.login_time,
            wr_ticket: copy_field(view.wr_ticket),
            wr_wrpa: copy_field(view.wr_wrpa),
        }
    }
}

fn copy_field(value: &str) -> String {
    #[cfg(test)]
    observe_work(|work| work.field_copy_bytes += value.len());
    value.to_owned()
}

impl AuthSession {
    pub(crate) const MAX_ENCODED_BYTES: usize = 1024 * 1024;

    pub(crate) fn validate_resource_budget(&self) -> Result<()> {
        AuthSessionView::from_session(self)?.admit().map(drop)
    }
}

struct EncodedCounter {
    bytes: usize,
    limit: usize,
    exceeded: bool,
}

impl EncodedCounter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: 0,
            limit,
            exceeded: false,
        }
    }
}

impl io::Write for EncodedCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(next) = self
            .bytes
            .checked_add(bytes.len())
            .filter(|bytes| *bytes <= self.limit)
        else {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Encoded account budget exceeded",
            ));
        };
        self.bytes = next;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AuthBudgetWork {
    pub(crate) preflights: usize,
    pub(crate) admissions: usize,
    pub(crate) encoded_bytes: usize,
    pub(crate) materializations: usize,
    pub(crate) field_copy_bytes: usize,
    pub(crate) adoptions: usize,
}

#[cfg(test)]
std::thread_local! {
    static WORK: std::cell::Cell<AuthBudgetWork> = std::cell::Cell::new(AuthBudgetWork::default());
}

#[cfg(test)]
fn observe_work(observe: impl FnOnce(&mut AuthBudgetWork)) {
    WORK.with(|slot| {
        let mut work = slot.get();
        observe(&mut work);
        slot.set(work);
    });
}

#[cfg(test)]
pub(crate) fn reset_work() {
    WORK.with(|slot| slot.set(AuthBudgetWork::default()));
}

#[cfg(test)]
pub(crate) fn work() -> AuthBudgetWork {
    WORK.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn observe_adoption() {
    observe_work(|work| work.adoptions += 1);
}

#[cfg(test)]
#[path = "auth_budget_tests.rs"]
mod tests;
