//! Genuine local-note editing and pointer selection keep derived buffers bounded.

use super::reading::{SelectionLimits, SelectionWorkStats, with_selection_test_limits};
use super::underline_projection_tests::{BOOK, Fixture, isolated};
use super::*;
use crate::store::AnnotationUnavailable;

fn event(fixture: &Fixture, action: &str, value: &str) {
    fixture
        .ui()
        .global::<ReaderModel>()
        .invoke_event(action.into(), value.into());
}

fn large_note(state: &App, quote: &str) -> Result<()> {
    state.store.save_annotation(&Annotation {
        id: "selection-large-note".into(),
        book_id: BOOK.into(),
        chapter_index: 0,
        start: 0,
        end: quote.chars().count() as u32,
        quote: quote.into(),
        note: "Small editable note".into(),
        created_at: 1,
    })
}

fn sql(state: &App) -> Result<rusqlite::Connection> {
    Ok(rusqlite::Connection::open(
        state.data_dir.join("library.sqlite"),
    )?)
}

fn stored_note(state: &App) -> Result<(String, String)> {
    Ok(sql(state)?.query_row(
        "SELECT quote,note FROM annotations WHERE id='selection-large-note'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?)
}

fn large_quote_edit() -> Result<()> {
    let body = "A ".repeat(12 * 1024);
    let fixture = Fixture::new(&body, Vec::new(), false)?;
    {
        let state = fixture.app();
        large_note(&state, &body)?;
        state
            .reader
            .selection_work_stats
            .set(SelectionWorkStats::default());
        state.store.reset_annotation_read_stats();
    }
    with_selection_test_limits(
        SelectionLimits {
            preview_bytes: 4,
            ..Default::default()
        },
        || event(&fixture, "reader.thought", "mine:selection-large-note"),
    );
    {
        let state = fixture.app();
        assert_eq!(
            state.reader.selection,
            Some((0, body.chars().count() as u32))
        );
        assert_eq!(state.reader.body, body);
        assert_eq!(state.reader.draft.query, "Small editable note");
        assert!(!state.reader.thought_edit_blocked);
        assert!(state.reader.note_edit_context.is_some());
        assert_eq!(
            state
                .reader
                .selection_work_stats
                .get()
                .preview_source_copies,
            0
        );
        assert_eq!(
            state.reader.selection_work_stats.get().source_scalar_visits,
            0
        );
        assert_eq!(state.store.annotation_read_stats().quote_copies, 0);
        assert_eq!(
            fixture.ui().global::<ReaderModel>().get_selection_text(),
            tr("api_error")
        );
        assert_eq!(
            stored_note(&state)?,
            (body.clone(), "Small editable note".into())
        );
        state.store.reset_annotation_read_stats();
        state
            .reader
            .selection_work_stats
            .set(SelectionWorkStats::default());
    }
    event(&fixture, "reader.thought-draft", "Updated admitted note");
    event(&fixture, "reader.save-thought", "");
    {
        let state = fixture.app();
        assert_eq!(
            stored_note(&state)?,
            (body.clone(), "Updated admitted note".into())
        );
        assert_eq!(state.store.annotation_read_stats().quote_copies, 0);
        assert_eq!(
            state
                .reader
                .selection_work_stats
                .get()
                .new_quote_source_copies,
            0
        );
        assert_eq!(state.reader.body, body);
        assert_eq!(state.store.annotation_count(BOOK)?, 1);
    }
    fixture.finish()
}

#[test]
fn old_large_quote_keeps_small_note_editable_without_resident_preview_or_quote_copy() -> Result<()>
{
    isolated(
        "app::selection_workset_tests::old_large_quote_keeps_small_note_editable_without_resident_preview_or_quote_copy",
        large_quote_edit,
    )
}

fn phrase_local_line() -> Result<()> {
    let body = format!(
        "A measured token.\n{}",
        "Other retained line.\n".repeat(512)
    );
    let fixture = Fixture::new(&body, Vec::new(), false)?;
    let (x, y, line_scalars) = {
        let mut state = fixture.app();
        state.reader.selection = Some((0, 1));
        assert!(state.reader.draft.set_query("Keep the prior draft"));
        let paginator = state.reader.paginator.as_ref().unwrap();
        let line = &paginator.current_page().lines[0];
        let x = (fixture.ui().global::<ReaderModel>().get_margin() as f32
            + (line
                .x_at_offset(3, &paginator.layout, &state.fonts[0].metrics)
                .unwrap()
                + line
                    .x_at_offset(4, &paginator.layout, &state.fonts[0].metrics)
                    .unwrap())
                / 2.0)
            / state.platform.logical_viewport_size().0 as f32;
        (
            x,
            line.y + line.height / 2.0,
            (line.end - line.start) as usize,
        )
    };
    event(&fixture, "reader.press", &format!("{x},0,{y}"));
    fixture
        .app()
        .reader
        .selection_work_stats
        .set(SelectionWorkStats::default());
    let generation = fixture
        .ui()
        .global::<ReaderModel>()
        .get_press_generation()
        .to_string();
    with_selection_test_limits(
        SelectionLimits {
            work_bytes: 0,
            ..Default::default()
        },
        || event(&fixture, "reader.hold", &generation),
    );
    {
        let state = fixture.app();
        let stats = state.reader.selection_work_stats.get();
        assert_eq!(stats.phrase_vectors, 0);
        assert_eq!(stats.phrase_scalar_visits, 0);
        assert_eq!(stats.preview_source_copies, 0);
        assert_eq!(state.reader.selection, Some((0, 1)));
        assert_eq!(state.reader.draft.query, "Keep the prior draft");
        assert_eq!(state.reader.body, body);
    }
    event(&fixture, "reader.cancel", "");
    event(&fixture, "reader.press", &format!("{x},0,{y}"));
    fixture
        .app()
        .reader
        .selection_work_stats
        .set(SelectionWorkStats::default());
    let generation = fixture
        .ui()
        .global::<ReaderModel>()
        .get_press_generation()
        .to_string();
    event(&fixture, "reader.hold", &generation);
    event(&fixture, "reader.release", &format!("{x},0,{y}"));
    {
        let state = fixture.app();
        let stats = state.reader.selection_work_stats.get();
        assert_eq!(stats.phrase_vectors, 1);
        assert_eq!(stats.phrase_scalar_visits, line_scalars);
        assert_eq!(stats.phrase_copied_scalars, line_scalars);
        assert!(stats.phrase_copied_scalars < body.chars().count() / 100);
        assert_eq!(state.reader.selection, Some((2, 10)));
        assert_eq!(
            fixture.ui().global::<ReaderModel>().get_selection_text(),
            "measured"
        );
        assert_eq!(state.reader.body, body);
    }
    fixture.finish()
}

#[test]
fn english_hold_uses_one_local_line_and_zero_quota_preserves_old_selection_and_draft() -> Result<()>
{
    isolated(
        "app::selection_workset_tests::english_hold_uses_one_local_line_and_zero_quota_preserves_old_selection_and_draft",
        phrase_local_line,
    )
}

fn prefix_work() -> Result<()> {
    let fixture = Fixture::new(
        &format!("abcdef{}", "retained".repeat(128)),
        Vec::new(),
        false,
    )?;
    {
        let mut state = fixture.app();
        state.reader.selection = Some((2, 4));
        assert!(state.reader.draft.set_query("Keep prefix draft"));
        let body = state.reader.body.clone();
        state
            .reader
            .selection_work_stats
            .set(SelectionWorkStats::default());
        with_selection_test_limits(
            SelectionLimits {
                work_bytes: 19,
                preview_bytes: 2,
                ..Default::default()
            },
            || state.render_reader(),
        )?;
        let denied = state.reader.selection_work_stats.get();
        assert_eq!(denied.source_scalar_visits, 0);
        assert_eq!(denied.preview_source_copies, 0);
        assert_eq!(state.reader.selection, Some((2, 4)));
        assert_eq!(state.reader.draft.query, "Keep prefix draft");
        assert_eq!(state.reader.body, body);
        state
            .reader
            .selection_work_stats
            .set(SelectionWorkStats::default());
        with_selection_test_limits(
            SelectionLimits {
                work_bytes: 20,
                preview_bytes: 2,
                ..Default::default()
            },
            || state.render_reader(),
        )?;
        let admitted = state.reader.selection_work_stats.get();
        assert_eq!(admitted.work_bytes, 20);
        assert_eq!(admitted.source_scalar_visits, 5);
        assert_eq!(admitted.source_bytes, 5);
        assert_eq!(admitted.preview_source_copies, 1);
        assert_eq!(admitted.preview_source_bytes, 2);
        assert_eq!(
            fixture.ui().global::<ReaderModel>().get_selection_text(),
            "cd"
        );
    }
    fixture.finish()
}

#[test]
fn preview_prefix_and_endpoint_work_is_prepaid_before_scanning_or_copying() -> Result<()> {
    isolated(
        "app::selection_workset_tests::preview_prefix_and_endpoint_work_is_prepaid_before_scanning_or_copying",
        prefix_work,
    )
}

fn utf8_and_invalid() -> Result<()> {
    let fixture = Fixture::new("A\u{1f600}Z", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        state.reader.selection = Some((1, 2));
        state.reader.draft.set_query("Keep UTF-8 draft");
        state
            .reader
            .selection_work_stats
            .set(SelectionWorkStats::default());
        with_selection_test_limits(
            SelectionLimits {
                preview_bytes: 3,
                ..Default::default()
            },
            || state.render_reader(),
        )?;
        assert_eq!(
            state
                .reader
                .selection_work_stats
                .get()
                .preview_source_copies,
            0
        );
        assert_eq!(state.reader.selection, Some((1, 2)));
        assert_eq!(state.reader.draft.query, "Keep UTF-8 draft");
        assert_eq!(
            fixture.ui().global::<ReaderModel>().get_selection_text(),
            tr("api_error")
        );
        state
            .reader
            .selection_work_stats
            .set(SelectionWorkStats::default());
        with_selection_test_limits(
            SelectionLimits {
                preview_bytes: 4,
                ..Default::default()
            },
            || state.render_reader(),
        )?;
        assert_eq!(
            state.reader.selection_work_stats.get().preview_source_bytes,
            4
        );
        assert_eq!(
            fixture.ui().global::<ReaderModel>().get_selection_text(),
            "\u{1f600}"
        );
        state.reader.selection = Some((90, 91));
        state
            .reader
            .selection_work_stats
            .set(SelectionWorkStats::default());
        state.render_reader()?;
        assert_eq!(state.reader.selection, Some((90, 91)));
        assert_eq!(
            state
                .reader
                .selection_work_stats
                .get()
                .preview_source_copies,
            0
        );
        assert_eq!(
            state.reader.selection_work_stats.get().source_scalar_visits,
            0
        );
        assert_eq!(state.reader.body, "A\u{1f600}Z");
        assert_eq!(
            fixture.ui().global::<ReaderModel>().get_selection_text(),
            tr("api_error")
        );
    }
    fixture.finish()
}

#[test]
fn utf8_preview_byte_gate_and_unavailable_anchors_never_guess_an_empty_success() -> Result<()> {
    isolated(
        "app::selection_workset_tests::utf8_preview_byte_gate_and_unavailable_anchors_never_guess_an_empty_success",
        utf8_and_invalid,
    )
}

fn stale_note_fence() -> Result<()> {
    let body = "A ".repeat(12 * 1024);
    let fixture = Fixture::new(&body, Vec::new(), false)?;
    large_note(&fixture.app(), &body)?;
    event(&fixture, "reader.thought", "mine:selection-large-note");
    event(
        &fixture,
        "reader.thought-draft",
        "Keep pending fenced update",
    );
    {
        let mut state = fixture.app();
        sql(&state)?.execute(
            "UPDATE annotations SET created_at=created_at+1 WHERE id='selection-large-note'",
            [],
        )?;
        let account = state.jobs.session();
        let error = state.reader_action("save-thought", "").unwrap_err();
        assert!(error.downcast_ref::<AnnotationUnavailable>().is_some());
        assert_eq!(state.reader.draft.query, "Keep pending fenced update");
        assert_eq!(
            state.reader.selection,
            Some((0, body.chars().count() as u32))
        );
        assert_eq!(state.reader.body, body);
        assert_eq!(state.jobs.session(), account);
        assert_eq!(
            stored_note(&state)?,
            (body.clone(), "Small editable note".into())
        );
    }
    fixture.finish()
}

#[test]
fn late_row_revision_rejects_note_update_without_replacing_legacy_quote_or_pending_editor()
-> Result<()> {
    isolated(
        "app::selection_workset_tests::late_row_revision_rejects_note_update_without_replacing_legacy_quote_or_pending_editor",
        stale_note_fence,
    )
}

fn readonly_book_projection() -> Result<()> {
    let fixture = Fixture::new("Retained book body.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let intro = "Retained original introduction. ".repeat(2048);
        let mut book = state.store.book(BOOK)?.unwrap();
        book.intro = intro.clone();
        book.progress = 0.25;
        state.store.upsert_book(&book)?;
        let account = state.jobs.session();
        state.render_reader()?;
        state.reader_upload_position()?;
        let stored = state.store.book(BOOK)?.unwrap();
        assert_eq!(stored.intro, intro);
        assert_eq!(stored.progress, 0.25);
        assert_eq!(state.reader.body, "Retained book body.");
        assert_eq!(state.jobs.session(), account);
        assert_eq!(stored.title, book.title);
    }
    fixture.finish()
}

#[test]
fn reader_readonly_projection_preserves_real_intro_progress_body_and_account() -> Result<()> {
    isolated(
        "app::selection_workset_tests::reader_readonly_projection_preserves_real_intro_progress_body_and_account",
        readonly_book_projection,
    )
}
