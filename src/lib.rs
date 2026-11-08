pub mod app;
pub mod article;
pub mod book_assets;
pub mod cache;
pub mod demo;
pub mod diagnostics;
pub mod domain;
pub mod i18n;
#[path = "import.rs"]
pub mod importer;
pub mod input;
pub mod jobs;
pub mod platform;
pub mod preferences;
pub mod reader;
pub mod session;
pub mod source;
pub mod statistics;
pub mod store;

pub mod ui {
    slint::include_modules!();
}
