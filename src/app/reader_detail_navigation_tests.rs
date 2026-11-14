//! Reader catalog entry uses Book Detail without losing the semantic reading anchor.

use super::super::underline_projection_tests::{BOOK, CHAPTER, Fixture, isolated};
use super::*;
use slint::Model;

const SECOND_CHAPTER: &str = "underline-fixture-chapter-1";
const OTHER_BOOK: &str = "reader-detail-other-book";

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReadingAnchor {
    book_id: String,
    chapter_index: u32,
    char_offset: u32,
}

fn chapter_body(ordinal: usize) -> String {
    format!(
        "Reader detail chapter {ordinal} retains Unicode anchors \u{1f600}\u{4e2d} across catalog navigation.\n"
    )
    .repeat(400)
}

fn reader_event(fixture: &Fixture, action: &str, value: &str) -> Result<()> {
    fixture.app().dispatch("reader", action, value)
}

fn detail_event(fixture: &Fixture, action: &str, value: &str) -> Result<()> {
    fixture.app().dispatch("detail", action, value)
}

fn anchor(state: &App) -> ReadingAnchor {
    let paginator = state
        .reader
        .paginator
        .as_ref()
        .expect("The detail navigation fixture must have a genuine paginator");
    ReadingAnchor {
        book_id: state.reader.book_id.clone(),
        chapter_index: state.reader.chapter_index,
        char_offset: paginator.current_page().start,
    }
}

fn prepare_nonfirst_page(fixture: &Fixture) -> Result<ReadingAnchor> {
    let mut state = fixture.app();
    let paginator = state.reader.paginator.as_mut().unwrap();
    assert!(
        paginator.pages.len() > 1,
        "The chapter must span real pages"
    );
    paginator.set_page(1);
    assert_eq!(paginator.current_index(), 1);
    assert!(paginator.current_page().start > 0);
    state.persist_reader_position()?;
    state.render_reader()?;
    Ok(anchor(&state))
}

fn assert_reader_anchor(fixture: &Fixture, expected: &ReadingAnchor) -> Result<()> {
    let state = fixture.app();
    assert_eq!(
        state.route,
        Route::Reader(expected.book_id.clone(), expected.chapter_index)
    );
    assert_eq!(&anchor(&state), expected);
    let paginator = state.reader.paginator.as_ref().unwrap();
    assert!(
        paginator.current_index() > 0,
        "Returning through details must not silently reopen the first body page"
    );
    assert!(fixture.ui().global::<ReaderModel>().get_page() > 1);
    let saved = state.store.position(&expected.book_id)?.unwrap();
    assert_eq!(saved.chapter_index, expected.chapter_index);
    assert_eq!(saved.char_offset, expected.char_offset);
    Ok(())
}

fn open_font_menu(fixture: &Fixture) -> Result<()> {
    reader_event(fixture, "menu", "true")?;
    reader_event(fixture, "tool", "type")?;
    reader_event(fixture, "subtool", "font")?;
    assert_font_menu(fixture);
    Ok(())
}

fn assert_font_menu(fixture: &Fixture) {
    let reader = fixture.ui().global::<ReaderModel>();
    assert!(reader.get_menu());
    assert_eq!(reader.get_tool().as_str(), "type");
    assert_eq!(reader.get_subtool().as_str(), "font");
}

fn assert_catalog_entry(fixture: &Fixture) {
    let state = fixture.app();
    assert_eq!(state.route, Route::Detail(BOOK.into()));
    assert_eq!(state.detail_id, BOOK);
    assert_eq!(state.detail_tab, "toc");
    assert!(!state.selecting_chapters);
    assert!(state.selected_chapters.is_empty());
    assert!(state.reader.press.is_none());
    assert!(state.reader.selection.is_none());
    let reader = fixture.ui().global::<ReaderModel>();
    assert!(!reader.get_toc_open());
    assert!(!reader.get_selection_open());
    assert_eq!(
        fixture.ui().global::<DetailModel>().get_tab().as_str(),
        "toc"
    );
    assert!(!fixture.ui().global::<DetailModel>().get_selecting());
    assert!(
        fixture
            .ui()
            .global::<ShellModel>()
            .get_dialog_kind()
            .is_empty(),
        "The catalog entry must not implicitly open More"
    );
}

fn start_pending_reader_press(fixture: &Fixture) -> Result<i32> {
    let pointer = {
        let state = fixture.app();
        let paginator = state.reader.paginator.as_ref().unwrap();
        let line = paginator
            .current_page()
            .lines
            .iter()
            .find(|line| line.end.saturating_sub(line.start) > 3)
            .expect("The nonfirst page must contain selectable text");
        let metrics = state
            .reader_font()
            .unwrap()
            .metrics_for_weight(state.reader.preferences.weight);
        let first = line
            .x_at_offset(line.start + 1, &paginator.layout, metrics)
            .unwrap();
        let second = line
            .x_at_offset(line.start + 2, &paginator.layout, metrics)
            .unwrap();
        let margin = reader_margin(state.profile.is_scribe(), state.reader.preferences.margin);
        let x = (margin + (first + second) / 2.0) / state.reader_text_width();
        let y = line.y + line.height / 2.0;
        assert!(state.reader_range_at_pointer(x, y, false).is_some());
        format!("{x},0,{y}")
    };
    reader_event(fixture, "press", &pointer)?;
    assert!(fixture.app().reader.press.is_some());
    Ok(fixture.ui().global::<ReaderModel>().get_press_generation())
}

fn prior_tabs_and_current_chapter(fixture: &Fixture) -> Result<()> {
    let expected = prepare_nonfirst_page(fixture)?;
    for prior_tab in ["intro", "notes"] {
        open_font_menu(fixture)?;
        reader_event(fixture, "detail", "")?;
        detail_event(fixture, "tab", prior_tab)?;
        assert_eq!(
            fixture.ui().global::<DetailModel>().get_tab().as_str(),
            prior_tab
        );
        {
            let mut state = fixture.app();
            // Seed stale catalog selection retained from an earlier detail visit.
            state.selecting_chapters = true;
            state.selected_chapters.insert(expected.chapter_index);
        }
        detail_event(fixture, "back", "")?;
        assert_reader_anchor(fixture, &expected)?;
        assert_font_menu(fixture);

        reader_event(fixture, "toc", "marks")?;
        assert!(fixture.ui().global::<ReaderModel>().get_toc_open());
        let old_generation = start_pending_reader_press(fixture)?;
        reader_event(fixture, "detail", "toc")?;
        assert_catalog_entry(fixture);
        reader_event(fixture, "hold", &old_generation.to_string())?;
        assert_catalog_entry(fixture);

        detail_event(fixture, "back", "")?;
        // Reader route transitions may finish/reopen sessions and rebuild pagination.
        assert_reader_anchor(fixture, &expected)?;
        assert_font_menu(fixture);
        reader_event(fixture, "hold", &old_generation.to_string())?;
        assert!(fixture.app().reader.press.is_none());
        assert!(fixture.app().reader.selection.is_none());
        assert!(!fixture.ui().global::<ReaderModel>().get_selection_open());
        assert_reader_anchor(fixture, &expected)?;

        reader_event(fixture, "detail", "toc")?;
        assert_catalog_entry(fixture);
        assert!(
            fixture
                .ui()
                .global::<DetailModel>()
                .get_chapters()
                .iter()
                .any(|chapter| chapter.current && chapter.id.as_str() == CHAPTER)
        );
        detail_event(fixture, "chapter", CHAPTER)?;
        assert_reader_anchor(fixture, &expected)?;
    }
    Ok(())
}

fn sparse_catalog(fixture: &Fixture) -> Result<Vec<(String, u32)>> {
    let mut state = fixture.app();
    let size = state.profile.detail_chapter_page_size();
    let count = size * 2 + 1;
    let mut catalog = vec![(CHAPTER.into(), 0), (SECOND_CHAPTER.into(), 1)];
    let additions = (2..count)
        .map(|ordinal| Chapter {
            id: format!("reader-detail-sparse-{ordinal:02}"),
            book_id: BOOK.into(),
            index: 37 + (ordinal as u32 - 2) * 5,
            title: format!("Sparse detail chapter {ordinal}"),
            cached: true,
            body: Some(chapter_body(ordinal)),
            content_status: ContentStatus::Full,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    catalog.extend(
        additions
            .iter()
            .map(|chapter| (chapter.id.clone(), chapter.index)),
    );
    for chapter in additions.iter().rev() {
        state.store.upsert_chapter(chapter)?;
    }
    let mut book = state.store.book(BOOK)?.unwrap();
    book.total_chapters = count as u32;
    book.cached_chapters = count as u32;
    state.store.upsert_book(&book)?;
    assert_eq!(
        state
            .store
            .chapter_metadata(BOOK)?
            .into_iter()
            .map(|chapter| (chapter.id, chapter.index))
            .collect::<Vec<_>>(),
        catalog
    );
    state.load_local_reader(BOOK, catalog[size + 1].1, Some(0))?;
    state.render_reader()?;
    Ok(catalog)
}

fn assert_sparse_detail_page(fixture: &Fixture, catalog: &[(String, u32)]) {
    assert_catalog_entry(fixture);
    let state = fixture.app();
    let size = state.profile.detail_chapter_page_size();
    assert_eq!(
        state.detail_page, 1,
        "Catalog pages use ordinal, not sparse source index"
    );
    let detail = fixture.ui().global::<DetailModel>();
    assert_eq!(detail.get_page(), 2);
    assert_eq!(detail.get_page_count(), 3);
    let rows = detail.get_chapters().iter().collect::<Vec<_>>();
    assert_eq!(rows.len(), size);
    assert_eq!(
        rows.iter()
            .map(|row| row.id.to_string())
            .collect::<Vec<_>>(),
        catalog[size..size * 2]
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>()
    );
    for (row, (_, index)) in rows.iter().zip(&catalog[size..size * 2]) {
        assert_eq!(row.index, *index as i32);
        assert_eq!(row.current, *index == catalog[size + 1].1);
        assert!(!row.selected);
    }
    assert_eq!(rows.iter().filter(|row| row.current).count(), 1);
}

fn sparse_detail_navigation(fixture: &Fixture) -> Result<()> {
    let catalog = sparse_catalog(fixture)?;
    let expected = prepare_nonfirst_page(fixture)?;
    let size = fixture.app().profile.detail_chapter_page_size();
    {
        let mut state = fixture.app();
        let mut other = Book::new(OTHER_BOOK, "Another detail cache owner");
        other.source = "local".into();
        state.store.upsert_book(&other)?;
        state.navigate(Route::Detail(OTHER_BOOK.into()), true)?;
        state.render()?;
    }
    detail_event(fixture, "back", "")?;
    assert_reader_anchor(fixture, &expected)?;
    open_font_menu(fixture)?;

    // A changed detail owner exercises initial catalog location before forcing a tab.
    reader_event(fixture, "detail", "")?;
    assert_sparse_detail_page(fixture, &catalog);
    detail_event(fixture, "tab", "intro")?;
    detail_event(fixture, "tab", "toc")?;
    assert_sparse_detail_page(fixture, &catalog);
    detail_event(fixture, "tab", "notes")?;
    detail_event(fixture, "back", "")?;
    assert_reader_anchor(fixture, &expected)?;
    assert_font_menu(fixture);

    reader_event(fixture, "detail", "toc")?;
    assert_sparse_detail_page(fixture, &catalog);
    detail_event(fixture, "chapter", &catalog[size + 1].0)?;
    assert_reader_anchor(fixture, &expected)?;
    Ok(())
}

fn detail_navigation_scenario(profile: DeviceProfile) -> Result<()> {
    let fixture = Fixture::new_for_profile(&chapter_body(0), Vec::new(), false, profile)?;
    prior_tabs_and_current_chapter(&fixture)?;
    sparse_detail_navigation(&fixture)?;
    fixture.finish()
}

fn scribe_scenario() -> Result<()> {
    detail_navigation_scenario(DeviceProfile::Scribe)
}

fn paperwhite6_scenario() -> Result<()> {
    detail_navigation_scenario(DeviceProfile::Paperwhite6)
}

#[test]
fn scribe_reader_catalog_uses_book_detail_and_preserves_reader_anchor() -> Result<()> {
    isolated(
        "app::reading::detail_navigation_tests::scribe_reader_catalog_uses_book_detail_and_preserves_reader_anchor",
        scribe_scenario,
    )
}

#[test]
fn paperwhite6_reader_catalog_uses_book_detail_and_preserves_reader_anchor() -> Result<()> {
    isolated(
        "app::reading::detail_navigation_tests::paperwhite6_reader_catalog_uses_book_detail_and_preserves_reader_anchor",
        paperwhite6_scenario,
    )
}
