//! Replaceable WeRead source adapter. Network contracts are traced in API_REFERENCE.md.
mod access;
mod client;
pub mod content;
pub mod images;
mod login;
pub mod protocol;
pub mod report;
mod types;
mod web_session;

pub use client::{ChapterReadHints, WeReadClient};
pub use login::LoginFlow;
pub use types::*;

pub const REFERENCE_REVISION: &str = "2943080c2493a1ae262cc97c74908ed62928c935";
