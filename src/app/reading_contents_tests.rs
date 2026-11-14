//! Real contents navigation retains Reader pagination and uses catalog order.

use super::super::underline_projection_tests::{BOOK, CHAPTER, Fixture, isolated};
use super::*;
use slint::Model;

const SECOND_CHAPTER: &str = "underline-fixture-chapter-1";
const ENTRY_COUNT: usize = 33;

#[derive(Debug, PartialEq)]
struct ReaderCheckpoint {
    route: Route,
    history: Vec<Route>,
    book_id: String,
    chapter_index: u32,
    body: String,
    page_storage: usize,
    page_ranges: Vec<(u32, u32)>,
    page_index: usize,
    page_start: u32,
    page_end: u32,
    layout: Layout,
    session_started: Option<i64>,
    session_seconds: u64,
    report_session: Option<String>,
    position_confirmed: bool,
    saved_position: Option<ReadingPosition>,
    menu: bool,
    tool: String,
    subtool: String,
}

fn checkpoint(state: &App) -> Result<ReaderCheckpoint> {
    let paginator = state
        .reader
        .paginator
        .as_ref()
        .expect("The contents fixture must retain its real paginator");
    let page = paginator.current_page();
    let ui = state
        .ui
        .upgrade()
        .expect("The contents fixture must retain its window");
    let reader = ui.global::<ReaderModel>();
    Ok(ReaderCheckpoint {
        route: state.route.clone(),
        history: state.history.clone(),
        book_id: state.reader.book_id.clone(),
        chapter_index: state.reader.chapter_index,
        body: state.reader.body.clone(),
        page_storage: paginator.pages.as_ptr() as usize,
        page_ranges: paginator
            .pages
            .iter()
            .map(|page| (page.start, page.end))
            .collect(),
        page_index: paginator.current_index(),
        page_start: page.start,
        page_end: page.end,
        layout: paginator.layout,
        session_started: state.reader.session_started,
        session_seconds: state.reader.session_seconds,
        report_session: state.active_read_report_session_id(),
        position_confirmed: state.reader.position_confirmed,
        saved_position: state.store.position(&state.reader.book_id)?,
        menu: reader.get_menu(),
        tool: reader.get_tool().to_string(),
        subtool: reader.get_subtool().to_string(),
    })
}

fn assert_reader_unchanged(fixture: &Fixture, expected: &ReaderCheckpoint) -> Result<()> {
    let state = fixture.app();
    assert_eq!(
        &checkpoint(&state)?,
        expected,
        "Contents navigation must retain pagination, history, session, position, and menu state"
    );
    Ok(())
}

fn reader_event(fixture: &Fixture, action: &str, value: &str) -> Result<()> {
    fixture.app().dispatch("reader", action, value)
}

fn chapter_body(ordinal: usize) -> String {
    format!("Reader contents chapter {ordinal} preserves distinct source pages.\n").repeat(320)
}

fn prepare_nonfirst_page(state: &mut App, seconds: u64) -> Result<()> {
    let paginator = state.reader.paginator.as_mut().unwrap();
    assert!(
        paginator.pages.len() > 1,
        "Both device profiles must exercise a genuinely paginated chapter"
    );
    paginator.set_page(1);
    assert_eq!(paginator.current_index(), 1);
    assert!(paginator.current_page().start > 0);
    assert!(state.reader.session_started.is_some());
    state.history = vec![Route::Library];
    state.reader.session_seconds = seconds;
    state.reader.position_confirmed = false;
    state.persist_reader_position()?;
    state.render_reader()
}

fn retained_contents_navigation(fixture: &Fixture) -> Result<()> {
    {
        let mut state = fixture.app();
        assert_eq!(state.reader.book_id, BOOK);
        prepare_nonfirst_page(&mut state, 17)?;
    }
    reader_event(fixture, "tool", "type")?;
    reader_event(fixture, "subtool", "font")?;
    let reader = fixture.ui().global::<ReaderModel>();
    for menu in [true, false] {
        reader_event(fixture, "menu", if menu { "true" } else { "false" })?;
        assert_eq!(reader.get_menu(), menu);
        assert_eq!(reader.get_tool().as_str(), "type");
        assert_eq!(reader.get_subtool().as_str(), "font");
        let expected = checkpoint(&fixture.app())?;

        reader_event(fixture, "press", "0.5,0,20")?;
        let stale_generation = fixture.app().reader.press.unwrap().generation;
        reader_event(fixture, "toc", "toc")?;
        reader_event(fixture, "hold", &stale_generation.to_string())?;
        assert!(fixture.app().reader.press.is_none());
        assert!(fixture.app().reader.selection.is_none());
        assert_reader_unchanged(fixture, &expected)?;
        reader_event(fixture, "toc-close", "")?;
        assert_reader_unchanged(fixture, &expected)?;

        reader_event(fixture, "toc", "toc")?;
        assert!(reader.get_toc_open());
        assert_reader_unchanged(fixture, &expected)?;
        reader_event(fixture, "toc-close", "")?;
        assert!(!reader.get_toc_open());
        assert_reader_unchanged(fixture, &expected)?;

        reader_event(fixture, "toc", "toc")?;
        reader_event(fixture, "back", "")?;
        assert!(!reader.get_toc_open());
        assert_reader_unchanged(fixture, &expected)?;

        reader_event(fixture, "toc", "toc")?;
        assert!(
            reader
                .get_chapters()
                .iter()
                .any(|row| row.current && row.id.as_str() == CHAPTER)
        );
        reader_event(fixture, "open-chapter", CHAPTER)?;
        assert!(!reader.get_toc_open());
        assert_reader_unchanged(fixture, &expected)?;
    }
    Ok(())
}

fn sparse_catalog(fixture: &Fixture) -> Result<Vec<(String, u32)>> {
    let mut state = fixture.app();
    let size = state.profile.reader_chapter_page_size();
    let count = size * 2 + 1;
    let mut catalog = vec![(CHAPTER.to_owned(), 0), (SECOND_CHAPTER.to_owned(), 1)];
    let additions = (2..count)
        .map(|ordinal| Chapter {
            id: format!("contents-sparse-{ordinal:02}"),
            book_id: BOOK.into(),
            index: 37 + (ordinal as u32 - 2) * 5,
            title: format!("Sparse contents chapter {ordinal}"),
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
    let mut book = state.store.book(BOOK)?.unwrap();
    book.total_chapters = count as u32;
    state.store.upsert_book(&book)?;
    // Reverse insertion makes the visible order depend on the sorted catalog.
    for chapter in additions.iter().rev() {
        state.store.upsert_chapter(chapter)?;
    }
    assert_eq!(
        state
            .store
            .chapter_metadata(BOOK)?
            .into_iter()
            .map(|chapter| (chapter.id, chapter.index))
            .collect::<Vec<_>>(),
        catalog
    );
    let current_ordinal = size + 1;
    state.load_local_reader(BOOK, catalog[current_ordinal].1, Some(0))?;
    prepare_nonfirst_page(&mut state, 23)?;
    for index in 0..ENTRY_COUNT {
        state.store.save_annotation(&Annotation {
            id: format!("contents-note-{index:02}"),
            book_id: BOOK.into(),
            chapter_index: state.reader.chapter_index,
            start: 0,
            end: 6,
            quote: "Reader".into(),
            note: format!("Contents note {index}"),
            created_at: index as i64 + 1,
        })?;
        state.store.save_bookmark(&Bookmark {
            id: format!("contents-mark-{index:02}"),
            book_id: BOOK.into(),
            chapter_index: state.reader.chapter_index,
            char_offset: index as u32,
            label: format!("Contents bookmark {index}"),
            created_at: index as i64 + 1,
        })?;
    }
    state.render_reader()?;
    Ok(catalog)
}

fn assert_chapter_page(
    fixture: &Fixture,
    catalog: &[(String, u32)],
    start: usize,
    count: usize,
    current_index: u32,
) {
    let rows = fixture
        .ui()
        .global::<ReaderModel>()
        .get_chapters()
        .iter()
        .collect::<Vec<_>>();
    let expected = &catalog[start..start + count];
    assert_eq!(rows.len(), count);
    assert_eq!(
        rows.iter()
            .map(|row| row.id.to_string())
            .collect::<Vec<_>>(),
        expected
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>()
    );
    for (row, (_, index)) in rows.iter().zip(expected) {
        assert_eq!(row.current, *index == current_index);
    }
}

fn sparse_contents_navigation(fixture: &Fixture) -> Result<()> {
    let catalog = sparse_catalog(fixture)?;
    let expected = checkpoint(&fixture.app())?;
    let size = fixture.app().profile.reader_chapter_page_size();
    let reader = fixture.ui().global::<ReaderModel>();
    reader_event(fixture, "toc", "toc")?;
    assert_eq!(reader.get_chapter_count(), (size * 2 + 1) as i32);
    assert_eq!(reader.get_chapter_pages(), 3);
    assert_eq!(reader.get_chapter_page(), 2);
    assert_eq!(reader.get_toc_tab().as_str(), "toc");
    assert_chapter_page(fixture, &catalog, size, size, expected.chapter_index);
    assert_reader_unchanged(fixture, &expected)?;

    reader_event(fixture, "toc-page", "1")?;
    assert_eq!(reader.get_chapter_page(), 3);
    assert_chapter_page(fixture, &catalog, size * 2, 1, expected.chapter_index);
    assert_reader_unchanged(fixture, &expected)?;
    reader_event(fixture, "toc-page", "1")?;
    assert_eq!(reader.get_chapter_page(), 3);
    assert_chapter_page(fixture, &catalog, size * 2, 1, expected.chapter_index);
    assert_reader_unchanged(fixture, &expected)?;

    reader_event(fixture, "toc-page", "-99")?;
    assert_eq!(reader.get_chapter_page(), 1);
    assert_chapter_page(fixture, &catalog, 0, size, expected.chapter_index);
    assert_reader_unchanged(fixture, &expected)?;
    reader_event(fixture, "toc-page", "-1")?;
    assert_eq!(reader.get_chapter_page(), 1);
    assert_reader_unchanged(fixture, &expected)?;
    reader_event(fixture, "toc-locate", "")?;
    assert_eq!(reader.get_chapter_page(), 2);
    assert_chapter_page(fixture, &catalog, size, size, expected.chapter_index);
    assert_reader_unchanged(fixture, &expected)?;
    reader_event(fixture, "toc-close", "")?;
    assert!(!reader.get_toc_open());
    assert_reader_unchanged(fixture, &expected)?;

    for tab in ["marks", "notes"] {
        fixture.app().reader.toc_page = usize::MAX;
        reader_event(fixture, "toc", tab)?;
        assert_eq!(reader.get_toc_tab().as_str(), tab);
        assert_eq!(reader.get_chapter_page(), 1);
        assert!(
            reader.get_chapter_pages() > 1,
            "Opening an entry tab must reset its page without relying on a one-page clamp"
        );
        assert!(reader.get_entries().row_count() > 0);
        assert_reader_unchanged(fixture, &expected)?;
        reader_event(fixture, "toc-page", "1")?;
        assert_eq!(reader.get_chapter_page(), 2);
        assert!(reader.get_entries().row_count() > 0);
        assert_reader_unchanged(fixture, &expected)?;
        reader_event(fixture, "toc-locate", "")?;
        assert_eq!(reader.get_toc_tab().as_str(), "toc");
        assert_eq!(reader.get_chapter_page(), 2);
        assert_chapter_page(fixture, &catalog, size, size, expected.chapter_index);
        assert_reader_unchanged(fixture, &expected)?;
        reader_event(fixture, "toc-close", "")?;
        assert!(!reader.get_toc_open());
        assert_reader_unchanged(fixture, &expected)?;
    }

    reader_event(fixture, "toc", "toc")?;
    reader_event(fixture, "open-chapter", &catalog[size + 1].0)?;
    assert!(!reader.get_toc_open());
    assert_reader_unchanged(fixture, &expected)?;

    reader_event(fixture, "menu", "true")?;
    reader_event(fixture, "tool", "type")?;
    reader_event(fixture, "subtool", "font")?;
    reader_event(fixture, "toc", "toc")?;
    assert!(reader.get_menu());
    assert_eq!(reader.get_tool().as_str(), "type");
    assert_eq!(reader.get_subtool().as_str(), "font");
    reader_event(fixture, "toc-page", "-99")?;
    reader_event(fixture, "open-chapter", CHAPTER)?;
    assert!(!reader.get_toc_open());
    assert!(!reader.get_menu());
    assert_eq!(reader.get_tool().as_str(), "type");
    assert!(reader.get_subtool().is_empty());
    {
        let state = fixture.app();
        let paginator = state.reader.paginator.as_ref().unwrap();
        assert_eq!(state.route, Route::Reader(BOOK.into(), 0));
        assert_eq!(state.reader.book_id, BOOK);
        assert_eq!(state.reader.chapter_index, 0);
        assert_eq!(state.reader.body, chapter_body(0));
        assert_eq!(paginator.current_index(), 0);
        assert_eq!(paginator.current_page().start, 0);
        assert!(state.reader.position_confirmed);
        assert_eq!(state.history, expected.history);
        assert_eq!(state.reader.session_started, expected.session_started);
        assert_eq!(state.reader.session_seconds, expected.session_seconds);
        assert_eq!(
            state.active_read_report_session_id(),
            expected.report_session
        );
    }
    reader_event(fixture, "back", "")?;
    {
        let state = fixture.app();
        assert_eq!(state.route, Route::Library);
        assert!(state.history.is_empty());
        assert!(state.reader.session_started.is_none());
    }
    Ok(())
}

fn contents_scenario(profile: DeviceProfile) -> Result<()> {
    let fixture = Fixture::new_for_profile(&chapter_body(0), Vec::new(), false, profile)?;
    retained_contents_navigation(&fixture)?;
    sparse_contents_navigation(&fixture)?;
    fixture.finish()
}

fn paperwhite6_contents() -> Result<()> {
    contents_scenario(DeviceProfile::Paperwhite6)
}

fn scribe_contents() -> Result<()> {
    contents_scenario(DeviceProfile::Scribe)
}

#[test]
fn paperwhite6_contents_preserve_reader_and_sparse_catalog_navigation() -> Result<()> {
    isolated(
        "app::reading::contents_tests::paperwhite6_contents_preserve_reader_and_sparse_catalog_navigation",
        paperwhite6_contents,
    )
}

#[test]
fn scribe_contents_preserve_reader_and_sparse_catalog_navigation() -> Result<()> {
    isolated(
        "app::reading::contents_tests::scribe_contents_preserve_reader_and_sparse_catalog_navigation",
        scribe_contents,
    )
}
