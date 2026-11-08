//! SQL interval geometry shares one whole-page output allowance with public and selection marks.

use super::reading::{MarkOutputLimits, MarkOutputStats, with_mark_output_test_limits};
use super::underline_projection_tests::{BOOK, Fixture, isolated};
use super::*;
use slint::Model;

fn save(state: &App, id: &str, chapter: u32, start: u32, end: u32) -> Result<()> {
    state.store.save_annotation(&Annotation {
        id: id.into(),
        book_id: BOOK.into(),
        chapter_index: chapter,
        start,
        end,
        quote: "Retained scalar quote".into(),
        note: "Retained local output note".into(),
        created_at: i64::from(start),
    })
}

fn reset(state: &App) {
    state
        .reader
        .mark_output_stats
        .set(MarkOutputStats::default());
    state.store.reset_annotation_read_stats();
}

fn marks(fixture: &Fixture) -> usize {
    fixture
        .ui()
        .global::<ReaderModel>()
        .get_lines()
        .iter()
        .map(|line| line.marks.row_count())
        .sum()
}

fn assert_withheld(fixture: &Fixture) {
    let rows = fixture.ui().global::<ReaderModel>().get_lines();
    assert!(rows.iter().all(|line| line.marks.row_count() == 0
        && !line.mine
        && !line.highlight
        && line.count == 0));
}

fn point(fixture: &Fixture, state: &App, offset: u32) -> (f32, f32) {
    let paginator = state.reader.paginator.as_ref().unwrap();
    let line = paginator
        .current_page()
        .lines
        .iter()
        .find(|line| line.start <= offset && offset < line.end)
        .unwrap();
    let left = line
        .x_at_offset(offset, &paginator.layout, &state.fonts[0].metrics)
        .unwrap();
    let right = line
        .x_at_offset(
            (offset + 1).min(line.end),
            &paginator.layout,
            &state.fonts[0].metrics,
        )
        .unwrap();
    let x = (fixture.ui().global::<ReaderModel>().get_margin() as f32 + (left + right) / 2.0)
        / state.platform.logical_viewport_size().0 as f32;
    (x, line.y + line.height / 2.0)
}

fn dense_legal_ranges() -> Result<()> {
    let body = format!("{}\n", "\u{4e2d}".repeat(30)).repeat(10);
    assert!(body.len() < 1024);
    let fixture = Fixture::new_for_profile(&body, Vec::new(), false, DeviceProfile::Scribe)?;
    {
        let mut state = fixture.app();
        state.reader.preferences.font_size = 18;
        state.reader.preferences.line_spacing = 0;
        state.reader.preferences.margin = 0;
        state.reflow_reader()?;
        let page = state.reader.paginator.as_ref().unwrap().current_page();
        assert_eq!(page.lines.len(), 10);
        assert!(page.lines.iter().all(|line| line.end - line.start == 30));
        assert_eq!(state.reader.paginator.as_ref().unwrap().char_count, 310);
        assert_eq!((page.start, page.end), (0, 310));
        let mut caret = crate::reader::CaretBudget::default();
        let mut reserved_positions = 0usize;
        let mut reserved_work = 0usize;
        for (index, line) in page.lines.iter().enumerate() {
            let start = (index * 31) as u32;
            assert_eq!((line.start, line.end), (start, start + 30));
            assert_eq!(line.text.len(), 90);
            assert_eq!(line.text.chars().count(), 30);
            assert!(!line.text.contains('\n'));
            // The newline advances the raw source but is absent from Line.text.
            // Thirty measured characters reserve thirty-one caret positions:
            // the initial boundary plus one push after each character.
            reserved_positions += 31;
            reserved_work += 90 * 91 / 2 + 90 * 8 + 1;
            let measured = caret.stats().measured;
            let geometry = line
                .caret_geometry(
                    &state.reader.paginator.as_ref().unwrap().layout,
                    &state.fonts[0].metrics,
                    &mut caret,
                )
                .unwrap();
            assert_eq!(caret.stats().measured - measured, 30);
            for offset in line.start..=line.end {
                assert!(geometry.x_at_offset(offset).is_finite());
            }
        }
        assert_eq!(reserved_positions, 310);
        assert!(reserved_positions <= 8193);
        assert_eq!(reserved_work, 48160);
        assert!(reserved_work <= crate::reader::DEFAULT_CARET_WORK);
        assert_eq!(caret.stats().admitted, 10);
        assert_eq!(caret.stats().allocations, 10);
        assert_eq!(caret.stats().rejected, 0);
        assert_eq!(caret.stats().measured, 300);
        let last = page.lines.last().unwrap().start;
        for start in 0..16 {
            for end in last + 7..=last + 30 {
                save(&state, &format!("dense-{start}-{end}"), 0, start, end)?;
            }
        }
        assert_eq!(state.store.annotation_count(BOOK)?, 384);
        state.prefs.mine = true;
        state.reader.draft.set_query("Keep dense overlay draft");
        let account = state.jobs.session();
        reset(&state);
        state.render_reader()?;
        assert_withheld(&fixture);
        let planned = state.reader.mark_output_stats.get();
        assert_eq!(planned.local_forecast, 3840);
        assert_eq!(planned.local_outputs, 0);
        assert_eq!(planned.remote_outputs, 0);
        assert_eq!(planned.selection_outputs, 0);
        assert_eq!(planned.rejected, 1);
        assert_eq!(state.store.annotation_read_stats().id_copies, 0);
        assert_eq!(state.store.annotation_read_stats().quote_copies, 0);
        assert_eq!(state.store.annotation_read_stats().note_copies, 0);
        let (x, y) = point(&fixture, &state, 0);
        reset(&state);
        assert_eq!(state.reader_hit_mark(x, y)?, None);
        assert_eq!(state.reader.mark_output_stats.get().local_forecast, 3840);
        assert_eq!(state.store.annotation_read_stats().id_copies, 0);
        assert_eq!(state.store.annotation_read_stats().quote_copies, 0);
        assert_eq!(state.store.annotation_read_stats().note_copies, 0);
        assert_eq!(state.reader.selection, None);
        assert_eq!(state.reader.body, body);
        assert_eq!(state.reader.draft.query, "Keep dense overlay draft");
        assert_eq!(state.jobs.session(), account);
        assert_eq!(state.store.annotation_count(BOOK)?, 384);
    }
    fixture.finish()
}

#[test]
fn tiny_legal_sql_ranges_can_pass_carets_but_reject_all_3840_local_fragments() -> Result<()> {
    isolated(
        "app::local_mark_output_tests::tiny_legal_sql_ranges_can_pass_carets_but_reject_all_3840_local_fragments",
        dense_legal_ranges,
    )
}

fn exact_shared_limits() -> Result<()> {
    let fixture = Fixture::new(
        "A \u{4e2d} B note.",
        vec![RemoteUnderline {
            text: "\u{4e2d}".into(),
            range: "mark-shared-range".into(),
            count: 3,
            thoughts: 1,
        }],
        true,
    )?;
    {
        let mut state = fixture.app();
        for end in 4..8 {
            save(&state, &format!("exact-{end}"), 0, 0, end)?;
        }
        state.prefs.mine = true;
        state.reader.selection = Some((0, 1));
        state.reader.draft.set_query("Keep shared output draft");
        let body = state.reader.body.clone();
        let account = state.jobs.session();
        reset(&state);
        state.render_reader()?;
        let exact = state.reader.mark_output_stats.get();
        assert_eq!(exact.local_forecast, 4);
        assert_eq!(exact.remote_forecast, 1);
        assert_eq!(exact.selection_forecast, 1);
        assert_eq!(marks(&fixture), 6);
        let (x, y) = point(&fixture, &state, 0);
        for limits in [
            MarkOutputLimits {
                marks: 5,
                ..Default::default()
            },
            MarkOutputLimits {
                marks: 6,
                bytes: exact.bytes - 1,
                work: exact.work,
            },
            MarkOutputLimits {
                marks: 6,
                bytes: exact.bytes,
                work: exact.work - 1,
            },
        ] {
            reset(&state);
            with_mark_output_test_limits(limits, || state.render_reader())?;
            assert_withheld(&fixture);
            let rejected = state.reader.mark_output_stats.get();
            assert_eq!(
                rejected.local_outputs + rejected.remote_outputs + rejected.selection_outputs,
                0
            );
            assert_eq!(rejected.rejected, 1);
            assert_eq!(state.store.annotation_read_stats().id_copies, 0);
            assert_eq!(state.store.annotation_read_stats().quote_copies, 0);
            assert_eq!(state.store.annotation_read_stats().note_copies, 0);
            assert_eq!(
                with_mark_output_test_limits(limits, || state.reader_hit_mark(x, y))?,
                None
            );
            assert_eq!(state.store.annotation_read_stats().id_copies, 0);
            assert_eq!(state.reader.body, body);
            assert_eq!(state.reader.selection, Some((0, 1)));
            assert_eq!(state.reader.draft.query, "Keep shared output draft");
            assert_eq!(state.jobs.session(), account);
        }
        let limits = MarkOutputLimits {
            marks: 6,
            bytes: exact.bytes,
            work: exact.work,
        };
        reset(&state);
        with_mark_output_test_limits(limits, || state.render_reader())?;
        assert_eq!(marks(&fixture), 6);
        assert_eq!(state.store.annotation_read_stats().id_copies, 0);
        reset(&state);
        assert_eq!(
            with_mark_output_test_limits(limits, || state.reader_hit_mark(x, y))?,
            Some("mine:exact-4".into())
        );
        assert_eq!(state.store.annotation_read_stats().id_copies, 1);
        assert_eq!(state.store.annotation_read_stats().quote_copies, 0);
        assert_eq!(state.store.annotation_read_stats().note_copies, 0);
        assert_eq!(state.store.annotation_count(BOOK)?, 4);
    }
    fixture.finish()
}

#[test]
fn local_remote_and_selection_share_exact_count_bytes_work_and_hit_gate() -> Result<()> {
    isolated(
        "app::local_mark_output_tests::local_remote_and_selection_share_exact_count_bytes_work_and_hit_gate",
        exact_shared_limits,
    )
}

fn unrelated_notes() -> Result<()> {
    let fixture = Fixture::new("Current note.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        save(&state, "current-only", 0, 0, 4)?;
        for index in 0..32 {
            save(&state, &format!("other-chapter-{index}"), 1, 0, 6)?;
        }
        state.prefs.mine = true;
        reset(&state);
        with_mark_output_test_limits(
            MarkOutputLimits {
                marks: 1,
                ..Default::default()
            },
            || state.render_reader(),
        )?;
        assert_eq!(state.reader.mark_output_stats.get().local_forecast, 1);
        assert_eq!(state.reader.mark_output_stats.get().local_outputs, 1);
        assert_eq!(marks(&fixture), 1);
        assert_eq!(state.store.annotation_read_stats().id_copies, 0);
        assert_eq!(state.store.annotation_count(BOOK)?, 33);
        assert_eq!(state.reader.body, "Current note.");
    }
    fixture.finish()
}

#[test]
fn unrelated_legal_history_does_not_consume_current_page_fragment_count() -> Result<()> {
    isolated(
        "app::local_mark_output_tests::unrelated_legal_history_does_not_consume_current_page_fragment_count",
        unrelated_notes,
    )
}

fn source_gap_without_fragments() -> Result<()> {
    let fixture = Fixture::new("AB\nCD", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        save(&state, "newline-only", 0, 2, 3)?;
        state.prefs.mine = true;
        reset(&state);
        with_mark_output_test_limits(
            MarkOutputLimits {
                marks: 0,
                ..Default::default()
            },
            || state.render_reader(),
        )?;
        let stats = state.reader.mark_output_stats.get();
        assert_eq!(stats.endpoint_slots, 2);
        assert_eq!(stats.local_forecast, 0);
        assert_eq!(stats.local_outputs, 0);
        assert_eq!(stats.rejected, 0);
        assert_eq!(marks(&fixture), 0);
        assert_eq!(state.store.annotation_read_stats().id_copies, 0);
        assert_eq!(state.store.annotation_count(BOOK)?, 1);
        assert_eq!(state.reader.body, "AB\nCD");
    }
    fixture.finish()
}

#[test]
fn a_legal_newline_only_range_is_not_blindly_multiplied_by_all_page_lines() -> Result<()> {
    isolated(
        "app::local_mark_output_tests::a_legal_newline_only_range_is_not_blindly_multiplied_by_all_page_lines",
        source_gap_without_fragments,
    )
}

fn zero_width_shared() -> Result<()> {
    let body = "A\u{301}B\nC\u{200d}D";
    let fixture = Fixture::new(
        body,
        vec![RemoteUnderline {
            text: "C\u{200d}D".into(),
            range: "zero-width-public".into(),
            count: 2,
            thoughts: 1,
        }],
        true,
    )?;
    {
        let mut state = fixture.app();
        save(&state, "zero-width-local", 0, 1, 2)?;
        state.prefs.mine = true;
        state.reader.selection = Some((1, 2));
        state.reader.draft.set_query("Keep zero-width draft");
        reset(&state);
        with_mark_output_test_limits(
            MarkOutputLimits {
                marks: 2,
                ..Default::default()
            },
            || state.render_reader(),
        )?;
        assert_withheld(&fixture);
        assert_eq!(state.reader.mark_output_stats.get().local_forecast, 1);
        assert_eq!(state.reader.mark_output_stats.get().remote_forecast, 1);
        assert_eq!(state.reader.mark_output_stats.get().selection_forecast, 1);
        with_mark_output_test_limits(
            MarkOutputLimits {
                marks: 3,
                ..Default::default()
            },
            || state.render_reader(),
        )?;
        assert_eq!(marks(&fixture), 3);
        assert!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_lines()
                .iter()
                .flat_map(|line| line.marks.iter().collect::<Vec<_>>())
                .all(|mark| mark.x.is_finite() && mark.width.is_finite() && mark.width >= 0.0)
        );
        state.reader.preferences.font_size = 22;
        state.reflow_reader()?;
        with_mark_output_test_limits(
            MarkOutputLimits {
                marks: 3,
                ..Default::default()
            },
            || state.render_reader(),
        )?;
        assert_eq!(marks(&fixture), 3);
        assert_eq!(state.reader.selection, Some((1, 2)));
        assert_eq!(state.reader.draft.query, "Keep zero-width draft");
        assert_eq!(state.reader.body, body);
        assert_eq!(state.store.annotation_count(BOOK)?, 1);
    }
    fixture.finish()
}

#[test]
fn zero_width_unicode_counts_real_objects_and_preserves_selection_across_reflow() -> Result<()> {
    isolated(
        "app::local_mark_output_tests::zero_width_unicode_counts_real_objects_and_preserves_selection_across_reflow",
        zero_width_shared,
    )
}

fn page_reflow_mix() -> Result<()> {
    let body = "AB\nCD\nEF\nGH";
    let fixture = Fixture::new(
        body,
        vec![RemoteUnderline {
            text: "EF".into(),
            range: "cross-page-public".into(),
            count: 2,
            thoughts: 1,
        }],
        true,
    )?;
    {
        let mut state = fixture.app();
        save(&state, "across-lines", 0, 0, 11)?;
        state.prefs.mine = true;
        state.reader.selection = Some((0, 11));
        state.reader.draft.set_query("Keep cross-page draft");
        let layout = crate::reader::Layout {
            width: 500.0,
            height: 40.0,
            font_size: 18.0,
            line_height: 1.6,
            indent: 0,
            paragraph_gap: 0.0,
            justify: false,
        };
        state.reader.paginator = Some(Paginator::new(body, &layout, &state.fonts[0].metrics));
        assert_eq!(
            state
                .reader
                .paginator
                .as_ref()
                .unwrap()
                .current_page()
                .lines
                .len(),
            1
        );
        with_mark_output_test_limits(
            MarkOutputLimits {
                marks: 2,
                ..Default::default()
            },
            || state.render_reader(),
        )?;
        assert_eq!(marks(&fixture), 2);
        state.reader.paginator.as_mut().unwrap().seek(6);
        let (x, y) = point(&fixture, &state, 6);
        reset(&state);
        with_mark_output_test_limits(
            MarkOutputLimits {
                marks: 2,
                ..Default::default()
            },
            || state.render_reader(),
        )?;
        assert_withheld(&fixture);
        assert_eq!(
            with_mark_output_test_limits(
                MarkOutputLimits {
                    marks: 2,
                    ..Default::default()
                },
                || state.reader_hit_mark(x, y)
            )?,
            None
        );
        assert_eq!(state.store.annotation_read_stats().id_copies, 0);
        state.reflow_reader()?;
        reset(&state);
        with_mark_output_test_limits(
            MarkOutputLimits {
                marks: 8,
                ..Default::default()
            },
            || state.render_reader(),
        )?;
        assert_withheld(&fixture);
        assert_eq!(state.reader.mark_output_stats.get().local_forecast, 4);
        assert_eq!(state.reader.mark_output_stats.get().selection_forecast, 4);
        assert_eq!(state.reader.mark_output_stats.get().remote_forecast, 1);
        with_mark_output_test_limits(
            MarkOutputLimits {
                marks: 9,
                ..Default::default()
            },
            || state.render_reader(),
        )?;
        assert_eq!(marks(&fixture), 9);
        assert_eq!(state.reader.selection, Some((0, 11)));
        assert_eq!(state.reader.draft.query, "Keep cross-page draft");
        assert_eq!(state.reader.body, body);
        assert_eq!(state.store.annotation_count(BOOK)?, 1);
    }
    fixture.finish()
}

#[test]
fn page_local_gate_is_recomputed_after_seek_and_reflow_without_losing_source_anchors() -> Result<()>
{
    isolated(
        "app::local_mark_output_tests::page_local_gate_is_recomputed_after_seek_and_reflow_without_losing_source_anchors",
        page_reflow_mix,
    )
}
