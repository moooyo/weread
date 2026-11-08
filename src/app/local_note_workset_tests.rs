//! Real Reader/Detail selections and scalar statistics over retained local history.

use super::underline_projection_tests::{BOOK, Fixture, isolated};
use super::*;
use crate::input::{InputPurpose, THOUGHT_INPUT_SCALARS};
use crate::statistics;
use crate::store::{AnnotationLimits, AnnotationResourceLimit};
use chrono::DateTime;
use rusqlite::params;
use slint::Model;

const BODY: &str = "Reader workset fixture.";

fn reader_event(fixture: &Fixture, action: &str, value: &str) {
    fixture
        .ui()
        .global::<ReaderModel>()
        .invoke_event(action.into(), value.into());
}

fn save_note(state: &App, id: &str, text: &str, created_at: i64) -> Result<()> {
    state.store.save_annotation(&Annotation {
        id: id.into(),
        book_id: BOOK.into(),
        chapter_index: 0,
        start: 0,
        end: 4,
        quote: "Read".into(),
        note: text.into(),
        created_at,
    })
}

fn populate(state: &App, count: usize) -> Result<()> {
    for index in 0..count {
        save_note(
            state,
            &format!("workset-{index:02}"),
            &format!("Visible note {index}"),
            index as i64 + 1,
        )?;
    }
    Ok(())
}

fn sql(state: &App) -> Result<rusqlite::Connection> {
    Ok(rusqlite::Connection::open(
        state.data_dir.join("library.sqlite"),
    )?)
}

fn retained_editor(
    state: &App,
) -> (
    String,
    String,
    u64,
    Option<(u32, u32)>,
    String,
    Option<source::AuthSession>,
) {
    (
        state.reader.draft.query.clone(),
        state.reader.draft.composition.clone(),
        state.reader.draft.query_revision(),
        state.reader.selection,
        state.reader.body.clone(),
        state.jobs.session(),
    )
}

fn prepare_editor(state: &mut App) {
    assert!(
        state
            .reader
            .draft
            .set_query_for_purpose("Keep the original editor", InputPurpose::Thought)
    );
    for key in "shi".chars() {
        state.reader.draft.press(&key.to_string());
    }
    state.reader.selection = Some((1, 3));
}

fn hidden_geometry() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut state = fixture.app();
        populate(&state, 32)?;
        state.prefs.mine = true;
        let previews = state.reader.thought_preview_reads;
        let layouts = state.reader.thought_quote_layouts;
        state.store.reset_annotation_read_stats();
        for _ in 0..4 {
            state.render_reader()?;
        }
        assert_eq!(state.store.annotation_read_stats(), Default::default());
        assert_eq!(state.reader.thought_preview_reads, previews);
        assert_eq!(state.reader.thought_quote_layouts, layouts);
        assert_eq!(fixture.ui().global::<ReaderModel>().get_note_count(), 32);
        assert_eq!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_thoughts()
                .row_count(),
            0
        );

        let paginator = state.reader.paginator.as_ref().unwrap();
        let layout = paginator.layout;
        let line = paginator
            .current_page()
            .lines
            .iter()
            .find(|line| line.start == 0)
            .unwrap();
        let metrics = &state.fonts[0].metrics;
        let margin = (state.platform.logical_viewport_size().0 as f32 - layout.width) / 2.0;
        let x = (margin
            + (line.x_at_offset(1, &layout, metrics).unwrap()
                + line.x_at_offset(3, &layout, metrics).unwrap())
                / 2.0)
            / state.platform.logical_viewport_size().0 as f32;
        let y = line.y + line.height / 2.0;
        assert_eq!(state.reader_hit_mark(x, y)?, Some("mine:workset-00".into()));
        let reads = state.store.annotation_read_stats();
        assert_eq!(reads.id_copies, 1);
        assert_eq!(reads.quote_copies, 0);
        assert_eq!(reads.note_copies, 0);
        assert_eq!(state.store.annotation_count(BOOK)?, 32);
    }
    fixture.finish()
}

#[test]
fn hidden_reader_and_caret_hit_do_not_materialize_history_text() -> Result<()> {
    isolated(
        "app::local_note_workset_tests::hidden_reader_and_caret_hit_do_not_materialize_history_text",
        hidden_geometry,
    )
}

fn visible_pages() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let state = fixture.app();
        populate(&state, 32)?;
        state.store.reset_annotation_read_stats();
    }
    reader_event(&fixture, "reader.toc", "notes");
    let first = fixture.ui().global::<ReaderModel>().get_entries();
    let first_ids = first
        .iter()
        .map(|entry| entry.id.to_string())
        .collect::<Vec<_>>();
    {
        let state = fixture.app();
        let reads = state.store.annotation_read_stats();
        assert!(!first_ids.is_empty() && first_ids.len() <= 16);
        assert_eq!(reads.quote_copies, first_ids.len());
        assert_eq!(reads.note_copies, first_ids.len());
        assert_eq!(first_ids[0], "annotation:workset-00");
        state.store.reset_annotation_read_stats();
    }
    reader_event(&fixture, "reader.toc-page", "1");
    {
        let state = fixture.app();
        let second = fixture.ui().global::<ReaderModel>().get_entries();
        let reads = state.store.annotation_read_stats();
        assert_eq!(reads.quote_copies, second.row_count());
        assert_eq!(reads.note_copies, second.row_count());
        assert!(second.row_count() > 0 && second.row_count() <= 16);
        assert!(
            second
                .iter()
                .all(|entry| !first_ids.contains(&entry.id.to_string()))
        );
        assert_eq!(state.store.annotation_count(BOOK)?, 32);
    }
    reader_event(&fixture, "reader.toc-close", "");
    {
        let mut state = fixture.app();
        state.store.reset_annotation_read_stats();
        state.render_reader()?;
        assert_eq!(state.store.annotation_read_stats(), Default::default());
        assert_eq!(state.store.annotation_count(BOOK)?, 32);
    }
    fixture.finish()
}

#[test]
fn toc_note_pages_read_only_their_visible_selected_payloads() -> Result<()> {
    isolated(
        "app::local_note_workset_tests::toc_note_pages_read_only_their_visible_selected_payloads",
        visible_pages,
    )
}

fn oversized_readonly() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    let text = "historical".repeat(64 * 1024 / 10 + 1);
    let kept = {
        let mut state = fixture.app();
        save_note(&state, "oversized-note", &text, 1)?;
        prepare_editor(&mut state);
        state.store.reset_annotation_read_stats();
        retained_editor(&state)
    };
    reader_event(&fixture, "reader.thought", "mine:oversized-note");
    {
        let state = fixture.app();
        assert_eq!(retained_editor(&state), kept);
        assert!(state.reader.thought_edit_blocked);
        assert_eq!(state.store.annotation_read_stats().note_copies, 0);
        let cards = fixture.ui().global::<ReaderModel>().get_thoughts();
        assert_eq!(cards.row_data(0).unwrap().body, tr("api_error"));
        assert_eq!(cards.row_data(0).unwrap().quote, "Read");
    }
    reader_event(&fixture, "reader.write-thought", "thread");
    reader_event(&fixture, "reader.thought-draft", "Rejected replacement");
    reader_event(&fixture, "reader.save-thought", "");
    reader_event(&fixture, "reader.highlight", "thread");
    {
        let state = fixture.app();
        assert_eq!(retained_editor(&state), kept);
        assert_eq!(state.store.annotation_read_stats().note_copies, 0);
        assert_eq!(state.store.annotation_count(BOOK)?, 1);
        assert_eq!(
            sql(&state)?.query_row(
                "SELECT note FROM annotations WHERE book_id=?1 AND id=?2",
                params![BOOK, "oversized-note"],
                |row| row.get::<_, String>(0)
            )?,
            text
        );
    }
    fixture.finish()
}

#[test]
fn oversized_historical_note_is_readonly_without_copying_or_replacing_it() -> Result<()> {
    isolated(
        "app::local_note_workset_tests::oversized_historical_note_is_readonly_without_copying_or_replacing_it",
        oversized_readonly,
    )
}

fn bounded_noneditable() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    let text = "n".repeat(THOUGHT_INPUT_SCALARS + 1);
    let kept = {
        let mut state = fixture.app();
        save_note(&state, "bounded-readonly", &text, 1)?;
        prepare_editor(&mut state);
        state.store.reset_annotation_read_stats();
        retained_editor(&state)
    };
    reader_event(&fixture, "reader.thought", "mine:bounded-readonly");
    {
        let state = fixture.app();
        assert!(state.reader.thought_edit_blocked);
        assert_eq!(retained_editor(&state), kept);
        assert_eq!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_thoughts()
                .row_data(0)
                .unwrap()
                .body,
            text
        );
        assert_eq!(state.store.annotation_read_stats().note_copies, 2);
        assert_eq!(state.store.annotation_count(BOOK)?, 1);
    }
    fixture.finish()
}

#[test]
fn admitted_display_text_still_obeys_the_separate_editor_scalar_gate() -> Result<()> {
    isolated(
        "app::local_note_workset_tests::admitted_display_text_still_obeys_the_separate_editor_scalar_gate",
        bounded_noneditable,
    )
}

fn selected_failure() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut state = fixture.app();
        for index in 0..2 {
            save_note(
                &state,
                &format!("byte-note-{index}"),
                &"b".repeat(512),
                index + 1,
            )?;
        }
        prepare_editor(&mut state);
        let kept = retained_editor(&state);
        let headers = state.store.annotation_headers_page(BOOK, 0, 2)?.headers;
        let mut limits = AnnotationLimits::records();
        let one =
            std::mem::size_of::<Annotation>() + BOOK.len() + headers[0].key.id.len() + 4 + 512;
        limits.bytes = one * 2 - 1;
        state.store.reset_annotation_read_stats();
        let error =
            super::reading::with_note_read_test_limits(AnnotationLimits::headers(), limits, || {
                state.note_display_records(BOOK, &headers)
            })
            .unwrap_err();
        let denial = error.downcast_ref::<AnnotationResourceLimit>().unwrap();
        assert_eq!(denial.resource, "selected bytes");
        assert_eq!(denial.requested, one * 2);
        assert_eq!(state.store.annotation_read_stats().quote_copies, 0);
        assert_eq!(state.store.annotation_read_stats().note_copies, 0);

        fixture.ui().global::<ReaderModel>().set_toc_open(true);
        state.reader.toc_tab = "notes".into();
        limits = AnnotationLimits::records();
        limits.rows = 0;
        state.store.reset_annotation_read_stats();
        let error =
            super::reading::with_note_read_test_limits(AnnotationLimits::headers(), limits, || {
                state.render_reader()
            })
            .unwrap_err();
        assert!(error.downcast_ref::<AnnotationResourceLimit>().is_some());
        assert_eq!(state.store.annotation_read_stats().quote_copies, 0);
        assert_eq!(state.store.annotation_read_stats().note_copies, 0);
        assert_eq!(retained_editor(&state), kept);
        assert_eq!(state.store.annotation_count(BOOK)?, 2);
        fixture.ui().global::<ReaderModel>().set_toc_open(false);
        let mut headers_limit = AnnotationLimits::headers();
        headers_limit.rows = 1;
        state.prefs.mine = true;
        state.store.reset_annotation_read_stats();
        let error = super::reading::with_note_read_test_limits(
            headers_limit,
            AnnotationLimits::records(),
            || state.reader_action("thought", "mine:byte-note-0"),
        )
        .unwrap_err();
        assert!(error.downcast_ref::<AnnotationResourceLimit>().is_some());
        assert_eq!(retained_editor(&state), kept);
        assert_eq!(state.store.annotation_read_stats(), Default::default());
        assert!(
            state
                .reader
                .draft
                .set_query_for_purpose("Keep the rejected new note", InputPurpose::Thought)
        );
        state.reader.selection = Some((0, 4));
        let kept = retained_editor(&state);
        limits = AnnotationLimits::records();
        limits.quote_bytes = 3;
        state.store.reset_annotation_read_stats();
        let error =
            super::reading::with_note_read_test_limits(AnnotationLimits::headers(), limits, || {
                state.reader_action("save-thought", "")
            })
            .unwrap_err();
        let denial = error.downcast_ref::<AnnotationResourceLimit>().unwrap();
        assert_eq!(denial.resource, "new quote bytes");
        assert_eq!(denial.requested, 4);
        assert_eq!(retained_editor(&state), kept);
        assert_eq!(state.store.annotation_count(BOOK)?, 2);
        assert_eq!(state.store.annotation_read_stats(), Default::default());
    }
    fixture.finish()
}

#[test]
fn selected_payload_admission_fails_before_first_copy_and_preserves_editor() -> Result<()> {
    isolated(
        "app::local_note_workset_tests::selected_payload_admission_fails_before_first_copy_and_preserves_editor",
        selected_failure,
    )
}

fn preserving_highlight() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    let text = "h".repeat(64 * 1024 + 1);
    {
        let mut state = fixture.app();
        save_note(&state, "preserved-large-note", &text, 1)?;
        state.reader.selection = Some((0, 4));
        state.store.reset_annotation_read_stats();
        state.reader_action("highlight", "selection")?;
        let reads = state.store.annotation_read_stats();
        assert_eq!(reads.note_copies, 0);
        assert_eq!(reads.quote_copies, 0);
        assert_eq!(state.store.annotation_count(BOOK)?, 1);
        let stored = sql(&state)?.query_row(
            "SELECT id,quote,note,created_at FROM annotations WHERE book_id=?1",
            [BOOK],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )?;
        assert_eq!(stored.0, "preserved-large-note");
        assert_eq!(stored.1, "Read");
        assert_eq!(stored.2, text);
        assert!(stored.3 > 1);
    }
    fixture.finish()
}

#[test]
fn replacing_a_highlight_uses_fenced_sql_without_reading_the_old_note() -> Result<()> {
    isolated(
        "app::local_note_workset_tests::replacing_a_highlight_uses_fenced_sql_without_reading_the_old_note",
        preserving_highlight,
    )
}

fn timestamp_statistics() -> Result<()> {
    let fixture = Fixture::new(BODY, Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let now = DateTime::from_timestamp(1_788_000_000, 0).unwrap();
        let minimum = DateTime::<Utc>::MIN_UTC.timestamp();
        for (index, timestamp) in [
            minimum - 1,
            minimum,
            -3600,
            now.timestamp() - 1,
            now.timestamp() + 1,
            i64::MIN,
            i64::MAX,
        ]
        .into_iter()
        .enumerate()
        {
            save_note(
                &state,
                &format!("timestamp-{index}"),
                "Retained timestamp note",
                timestamp,
            )?;
        }
        state.stats_period = "total".into();
        state.stats_offset = 99;
        state.store.reset_annotation_read_stats();
        let report = statistics::with_year_projection_test_context(512, 512 * 512, now, || {
            state.local_statistics_for_test()
        })?;
        assert_eq!(report.range_start, minimum - 8 * 3600);
        assert_eq!(report.range_end, now.timestamp());
        assert_eq!(report.annotations, 4);
        assert_eq!(report.offset, 0);
        assert!(report.bars_unavailable && report.bars.is_empty());
        assert_eq!(state.store.annotation_read_stats(), Default::default());
        assert_eq!(state.store.annotation_count(BOOK)?, 7);
    }
    fixture.finish()
}

#[test]
fn local_statistics_streams_counts_and_preserves_chrono_minimum_numeric_range() -> Result<()> {
    isolated(
        "app::local_note_workset_tests::local_statistics_streams_counts_and_preserves_chrono_minimum_numeric_range",
        timestamp_statistics,
    )
}

fn combined_selected_views() -> Result<()> {
    let fixture = Fixture::new_for_profile(BODY, Vec::new(), false, DeviceProfile::Scribe)?;
    {
        let mut state = fixture.app();
        populate(&state, 32)?;
        state.prefs.mine = true;
        state.reader.notes_panel = true;
        state.reflow_reader()?;
        state.reader.toc_tab = "notes".into();
        fixture.ui().global::<ReaderModel>().set_toc_open(true);
        state.store.reset_annotation_read_stats();
        state.render_reader()?;
        let toc = fixture
            .ui()
            .global::<ReaderModel>()
            .get_entries()
            .row_count();
        let side = fixture
            .ui()
            .global::<ReaderModel>()
            .get_thoughts()
            .row_count();
        assert!(toc > 0 && side > 0 && toc + side <= 16);
        assert_eq!(state.store.annotation_read_stats().note_copies, toc + side);
        let kept = retained_editor(&state);
        let mut limits = AnnotationLimits::records();
        limits.rows = toc;
        state.store.reset_annotation_read_stats();
        let error =
            super::reading::with_note_read_test_limits(AnnotationLimits::headers(), limits, || {
                state.render_reader()
            })
            .unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<AnnotationResourceLimit>()
                .unwrap()
                .resource,
            "selected rows"
        );
        assert_eq!(state.store.annotation_read_stats().note_copies, toc);
        assert_eq!(retained_editor(&state), kept);
        assert_eq!(state.store.annotation_count(BOOK)?, 32);
        fixture.ui().global::<ReaderModel>().set_toc_open(false);
    }
    reader_event(&fixture, "reader.detail", "");
    fixture.app().store.reset_annotation_read_stats();
    fixture
        .ui()
        .global::<DetailModel>()
        .invoke_event("tab".into(), "notes".into());
    let first = fixture.ui().global::<DetailModel>().get_reviews();
    {
        let state = fixture.app();
        assert_eq!(first.row_count(), 4);
        assert_eq!(state.store.annotation_read_stats().note_copies, 4);
        assert_eq!(state.store.annotation_read_stats().quote_copies, 4);
        state.store.reset_annotation_read_stats();
    }
    fixture
        .ui()
        .global::<DetailModel>()
        .invoke_event("page".into(), "1".into());
    {
        let state = fixture.app();
        let next = fixture.ui().global::<DetailModel>().get_reviews();
        assert_eq!(next.row_count(), 4);
        assert_ne!(first.row_data(0).unwrap().id, next.row_data(0).unwrap().id);
        assert_eq!(state.store.annotation_read_stats().note_copies, 4);
        assert_eq!(state.store.annotation_read_stats().quote_copies, 4);
        assert_eq!(state.store.annotation_count(BOOK)?, 32);
    }
    fixture.finish()
}

#[test]
fn coexisting_toc_and_side_view_share_the_selected_record_allowance() -> Result<()> {
    isolated(
        "app::local_note_workset_tests::coexisting_toc_and_side_view_share_the_selected_record_allowance",
        combined_selected_views,
    )
}
