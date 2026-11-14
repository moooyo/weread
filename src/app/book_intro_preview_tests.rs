//! Genuine Detail renders keep the sidebar preview separate from full payloads.

use super::super::underline_projection_tests::{BOOK, Fixture, isolated};
use super::*;
use crate::store::{BOOK_INTRODUCTION_PREVIEW_SCALARS, BookMetadataUnavailable};
use rusqlite::{Connection, OptionalExtension, params};
use slint::Model;

const EMPTY_BOOK: &str = "empty-introduction-preview-book";
const METADATA: &str =
    r#"{"bookInfo":{"publisher":"Preview fixture publisher","isbn":"160-preview"}}"#;

fn introduction() -> String {
    "A\u{1f642}\u{4e2d}e\u{301}\n".repeat(1_200)
}

fn reader_body() -> String {
    "Reader source body retains its character anchor during Detail tabs.\n\n".repeat(256)
}

#[derive(Debug, PartialEq)]
struct ReaderAnchor {
    book_id: String,
    chapter_index: u32,
    page_index: usize,
    start: u32,
    end: u32,
    body: String,
}

fn anchor(state: &App) -> ReaderAnchor {
    let paginator = state.reader.paginator.as_ref().unwrap();
    ReaderAnchor {
        book_id: state.reader.book_id.clone(),
        chapter_index: state.reader.chapter_index,
        page_index: paginator.current_index(),
        start: paginator.current_page().start,
        end: paginator.current_page().end,
        body: state.reader.body.clone(),
    }
}

fn database(fixture: &Fixture) -> Result<Connection> {
    Ok(Connection::open(
        fixture.app().data_dir.join("library.sqlite"),
    )?)
}

fn metadata(connection: &Connection, value: rusqlite::types::Value) -> Result<()> {
    connection.execute(
        "INSERT INTO settings(key,value) VALUES (?1,?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![format!("book.metadata:{BOOK}"), value],
    )?;
    Ok(())
}

fn raw_introduction(connection: &Connection) -> Result<(String, String, Option<(String, String)>)> {
    let (kind, bytes) = connection.query_row(
        "SELECT typeof(intro),hex(intro) FROM books WHERE id=?1",
        [BOOK],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let setting = connection
        .query_row(
            "SELECT typeof(value),hex(value) FROM settings WHERE key=?1",
            [format!("book.metadata:{BOOK}")],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok((kind, bytes, setting))
}

fn reset_reads(state: &App) {
    state.store.reset_book_read_stats();
    state.store.reset_book_introduction_read_stats();
}

fn assert_no_full_introduction(state: &App) {
    assert_eq!(state.store.book_read_stats().intro_copies, 0);
    assert_eq!(state.store.book_read_stats().intro_bytes, 0);
    assert_eq!(state.store.book_introduction_read_stats().metadata_reads, 0);
}

fn prepare(fixture: &Fixture, intro: &str) -> Result<()> {
    let mut state = fixture.app();
    let mut book = state.store.book(BOOK)?.unwrap();
    book.intro = intro.into();
    state.store.upsert_book(&book)?;
    state.store.upsert_book(&Book {
        source: "local".into(),
        on_shelf: true,
        ..Book::new(EMPTY_BOOK, "Empty introduction book")
    })?;
    let paginator = state.reader.paginator.as_mut().unwrap();
    assert!(paginator.pages.len() > 1);
    paginator.set_page(1);
    assert!(paginator.current_page().start > 0);
    state.persist_reader_position()?;
    state.dispatch("reader", "detail", "toc")
}

fn assert_scribe_prefix(fixture: &Fixture, full: &str) -> String {
    let detail = fixture.ui().global::<DetailModel>();
    let shown = detail.get_intro_preview().to_string();
    assert!(detail.get_intro_available());
    assert!(!shown.is_empty());
    assert!(shown.chars().count() <= BOOK_INTRODUCTION_PREVIEW_SCALARS);
    let prefix = shown.strip_suffix('\u{2026}').unwrap_or(&shown);
    assert!(full.starts_with(prefix));
    assert!(prefix.chars().count() >= BOOK_INTRODUCTION_PREVIEW_SCALARS - 1);
    assert!(prefix.len() > prefix.chars().count());
    assert_ne!(shown, full);
    shown
}

fn full_tab(fixture: &Fixture, full: &str, expected_preview: &str) -> Result<()> {
    let mut state = fixture.app();
    let previous_route = state.route.clone();
    let previous_anchor = anchor(&state);
    reset_reads(&state);
    // This is the same Detail tab action used by the preview's full-text entry.
    state.dispatch("detail", "tab", "intro")?;
    let detail = fixture.ui().global::<DetailModel>();
    assert_eq!(state.route, previous_route);
    assert_eq!(anchor(&state), previous_anchor);
    assert_eq!(state.detail_tab, "intro");
    assert_eq!(detail.get_tab().as_str(), "intro");
    assert_eq!(detail.get_intro().as_str(), full);
    assert_eq!(detail.get_intro_preview().as_str(), expected_preview);
    assert_eq!(detail.get_publisher().as_str(), "Preview fixture publisher");
    assert_eq!(state.store.book_read_stats().intro_copies, 1);
    assert_eq!(state.store.book_read_stats().intro_bytes, full.len());
    assert_eq!(state.store.book_introduction_read_stats().metadata_reads, 1);
    Ok(())
}

fn empty_book_clears_fields(fixture: &Fixture) -> Result<()> {
    let mut state = fixture.app();
    let previous_anchor = anchor(&state);
    state.open_detail(EMPTY_BOOK)?;
    state.detail_tab = "toc".into();
    reset_reads(&state);
    state.render_detail()?;
    let detail = fixture.ui().global::<DetailModel>();
    assert_eq!(state.route, Route::Detail(EMPTY_BOOK.into()));
    assert_eq!(detail.get_book().id.as_str(), EMPTY_BOOK);
    assert!(detail.get_intro_preview().is_empty());
    assert!(!detail.get_intro_available());
    assert!(detail.get_intro().is_empty());
    assert_eq!(anchor(&state), previous_anchor);
    assert_no_full_introduction(&state);
    Ok(())
}

fn scribe_preview() -> Result<()> {
    let full = introduction();
    let fixture =
        Fixture::new_for_profile(&reader_body(), Vec::new(), false, DeviceProfile::Scribe)?;
    prepare(&fixture, &full)?;
    let connection = database(&fixture)?;
    // Inactive tabs and the preview must not parse this invalid metadata blob.
    metadata(&connection, rusqlite::types::Value::Blob(vec![0x80]))?;
    let note = Annotation {
        id: "preserved-introduction-preview-note".into(),
        book_id: BOOK.into(),
        chapter_index: 0,
        start: 0,
        end: 6,
        quote: "Reader".into(),
        note: "Keep the existing private fixture note".into(),
        created_at: 1,
    };
    fixture.app().store.save_annotation(&note)?;
    let mut preview = String::new();
    for tab in ["toc", "notes", "reviews"] {
        let mut state = fixture.app();
        let previous_anchor = anchor(&state);
        reset_reads(&state);
        state.dispatch("detail", "tab", tab)?;
        assert_eq!(state.route, Route::Detail(BOOK.into()));
        assert_eq!(fixture.ui().global::<DetailModel>().get_tab().as_str(), tab);
        assert!(fixture.ui().global::<DetailModel>().get_intro().is_empty());
        assert_no_full_introduction(&state);
        assert_eq!(anchor(&state), previous_anchor);
        let shown = assert_scribe_prefix(&fixture, &full);
        if preview.is_empty() {
            preview = shown;
        } else {
            assert_eq!(shown, preview);
        }
    }
    metadata(&connection, rusqlite::types::Value::Text(METADATA.into()))?;
    full_tab(&fixture, &full, &preview)?;
    assert!(fixture.ui().global::<DetailModel>().get_intro_available());
    fixture.app().dispatch("detail", "tab", "toc")?;
    assert_eq!(assert_scribe_prefix(&fixture, &full), preview);

    let spaced = "  First paragraph.\r\n\r\n  Second paragraph.\n \nThird paragraph.\u{2028}\u{2029}Fourth paragraph.  ";
    let compact = "First paragraph.\nSecond paragraph.\nThird paragraph.\nFourth paragraph.";
    connection.execute(
        "UPDATE books SET intro=?1 WHERE id=?2",
        params![spaced, BOOK],
    )?;
    let spaced_raw = raw_introduction(&connection)?;
    fixture.app().dispatch("detail", "tab", "toc")?;
    assert_eq!(
        fixture
            .ui()
            .global::<DetailModel>()
            .get_intro_preview()
            .as_str(),
        compact
    );
    full_tab(&fixture, spaced, compact)?;
    assert_eq!(raw_introduction(&connection)?, spaced_raw);

    connection.execute("UPDATE books SET intro=x'80' WHERE id=?1", [BOOK])?;
    metadata(&connection, rusqlite::types::Value::Blob(vec![0x81]))?;
    let raw = raw_introduction(&connection)?;
    {
        let mut state = fixture.app();
        let error = state.store.book_introduction_preview(BOOK).unwrap_err();
        assert!(error.downcast_ref::<BookMetadataUnavailable>().is_some());
        let previous_anchor = anchor(&state);
        reset_reads(&state);
        state.dispatch("detail", "tab", "notes")?;
        let detail = fixture.ui().global::<DetailModel>();
        assert_eq!(state.route, Route::Detail(BOOK.into()));
        assert_eq!(detail.get_book().id.as_str(), BOOK);
        assert_eq!(detail.get_book().title.as_str(), "Underline fixture book");
        assert_eq!(
            detail.get_intro_preview().as_str(),
            tr("introduction_unavailable")
        );
        assert_ne!(detail.get_intro_preview().as_str(), preview);
        assert!(!detail.get_intro_available());
        assert!(detail.get_intro().is_empty());
        assert_eq!(detail.get_reviews().row_count(), 1);
        assert_eq!(state.store.annotations(BOOK)?, vec![note]);
        assert_eq!(anchor(&state), previous_anchor);
        assert_no_full_introduction(&state);
    }
    assert_eq!(raw_introduction(&connection)?, raw);
    connection.execute("UPDATE books SET intro=?1 WHERE id=?2", params![full, BOOK])?;
    metadata(&connection, rusqlite::types::Value::Text(METADATA.into()))?;
    fixture.app().dispatch("detail", "tab", "toc")?;
    assert_eq!(assert_scribe_prefix(&fixture, &full), preview);
    empty_book_clears_fields(&fixture)?;
    drop(connection);
    fixture.finish()
}

fn paperwhite_preview() -> Result<()> {
    let full = introduction();
    let fixture = Fixture::new_for_profile(
        &reader_body(),
        Vec::new(),
        false,
        DeviceProfile::Paperwhite6,
    )?;
    prepare(&fixture, &full)?;
    let connection = database(&fixture)?;
    metadata(&connection, rusqlite::types::Value::Blob(vec![0x80]))?;
    for invalid_intro in [false, true] {
        if invalid_intro {
            connection.execute("UPDATE books SET intro=x'80' WHERE id=?1", [BOOK])?;
        }
        let raw = raw_introduction(&connection)?;
        let mut state = fixture.app();
        let previous_anchor = anchor(&state);
        reset_reads(&state);
        state.dispatch("detail", "tab", "toc")?;
        let detail = fixture.ui().global::<DetailModel>();
        assert_eq!(state.route, Route::Detail(BOOK.into()));
        assert_eq!(detail.get_book().id.as_str(), BOOK);
        assert_eq!(detail.get_book().title.as_str(), "Underline fixture book");
        assert!(detail.get_chapters().row_count() > 0);
        assert!(detail.get_intro_preview().is_empty());
        assert!(!detail.get_intro_available());
        assert!(detail.get_intro().is_empty());
        assert_no_full_introduction(&state);
        assert_eq!(anchor(&state), previous_anchor);
        assert_eq!(raw_introduction(&connection)?, raw);
    }
    connection.execute("UPDATE books SET intro=?1 WHERE id=?2", params![full, BOOK])?;
    metadata(&connection, rusqlite::types::Value::Text(METADATA.into()))?;
    full_tab(&fixture, &full, "")?;
    assert!(!fixture.ui().global::<DetailModel>().get_intro_available());
    empty_book_clears_fields(&fixture)?;
    drop(connection);
    fixture.finish()
}

#[test]
fn scribe_detail_preview_is_bounded_and_full_intro_remains_on_demand() -> Result<()> {
    isolated(
        "app::library::introduction_preview_tests::scribe_detail_preview_is_bounded_and_full_intro_remains_on_demand",
        scribe_preview,
    )
}

#[test]
fn paperwhite_detail_skips_preview_and_inactive_intro_payloads() -> Result<()> {
    isolated(
        "app::library::introduction_preview_tests::paperwhite_detail_skips_preview_and_inactive_intro_payloads",
        paperwhite_preview,
    )
}
