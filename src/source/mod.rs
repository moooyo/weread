//! Replaceable WeRead source adapter. Network contracts are traced in API_REFERENCE.md.
mod access;
mod client;
pub mod content;
pub mod images;
mod login;
pub mod protocol;
mod reader_admission;
pub mod report;
mod types;
mod web_session;

pub use client::{ChapterReadHints, WeReadClient};
pub use login::LoginFlow;
pub(crate) use reader_admission::ReaderAdmission;
pub use types::*;

pub const REFERENCE_REVISION: &str = "2943080c2493a1ae262cc97c74908ed62928c935";

#[cfg(test)]
pub(crate) fn client_with_test_web_origin(
    session: AuthSession,
    origin: &str,
) -> Result<WeReadClient> {
    let mut client = WeReadClient::new(session)?;
    client.test_web_origin = Some(origin.into());
    client.http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(client::network_error)?;
    Ok(client)
}
