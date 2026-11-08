//! Local pointer and note operations with real registered TrueType kerning.

use super::underline_projection_tests::{BOOK, Fixture, isolated};
use super::*;
use crate::reader::{CaretStats, caret_test_font};
use anyhow::Context;
use slint::{Model, fontique_010::fontique};

fn cluster(marks: usize) -> String {
    format!("A{}B", "\u{301}".repeat(marks))
}

fn prepare(state: &mut App) -> Result<()> {
    let bytes = Arc::new(caret_test_font::bytes());
    // Use the same fontique registration and family/index metadata as Main's
    // custom-font loader, not merely an OwnedFace with an isolated kern table.
    let index = {
        let mut collection = slint::fontique_010::shared_collection();
        let registered = collection.register_fonts(fontique::Blob::new(bytes.clone()), None);
        let mut index = None;
        for (family, fonts) in registered {
            let name = collection
                .family_name(family)
                .context("Registered fixture font needs its family name")?;
            if name == caret_test_font::FAMILY {
                for font in fonts {
                    assert!((font.weight().value() - 400.0).abs() < 0.1);
                    index = Some(font.index());
                }
            }
        }
        index.context("The custom-font registration did not accept the fixture family")?
    };
    let metrics = TextMetrics::from_font(bytes.as_slice(), index).map_err(anyhow::Error::msg)?;
    assert!(metrics.is_font_backed());
    assert_eq!(metrics.advance('\u{301}', 20.0), 0.0);
    assert!(metrics.kerning('\u{301}', '\u{301}', 20.0) > 0.0);
    state.fonts = vec![FontRecord {
        id: "custom-caret-fixture".into(),
        label: "Local caret fixture".into(),
        family: caret_test_font::FAMILY.into(),
        metrics: metrics.clone(),
        weights: BTreeMap::from([(400, metrics)]),
        available: true,
    }];
    state.reader.preferences.font = "custom-caret-fixture".into();
    state.reader.preferences.font_size = 20;
    state.reader.preferences.indent = 0;
    state.reader.preferences.justify = false;
    state.prefs.popular = false;
    state.prefs.mine = true;
    state.reader.notes_panel = false;
    state.jobs.replace_session(source::AuthSession {
        user_vid: "local-caret-account".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "local-caret-account".into()),
            ("wr_skey".into(), "synthetic-local-caret-session".into()),
        ]),
        ..Default::default()
    })?;
    state.reflow_reader()?;
    state.reader.caret_stats.set(CaretStats::default());
    Ok(())
}

fn point(state: &App, line_index: usize, local_x: f32) -> (f32, f32) {
    let paginator = state.reader.paginator.as_ref().unwrap();
    let line = &paginator.current_page().lines[line_index];
    let width = state.platform.logical_viewport_size().0 as f32;
    let margin = (width - paginator.layout.width) / 2.0;
    ((local_x + margin) / width, line.y + line.height / 2.0)
}

fn pointer(point: (f32, f32)) -> String {
    format!("{},{}", point.0, point.1)
}

fn note(state: &App, id: &str, end: u32) -> Result<()> {
    state.store.save_annotation(&Annotation {
        id: id.into(),
        book_id: BOOK.into(),
        chapter_index: 0,
        start: 0,
        end,
        quote: state.reader.body.chars().take(end as usize).collect(),
        note: "Retained local note".into(),
        created_at: 0,
    })
}

fn assert_retained(
    state: &App,
    body: &str,
    selection: (u32, u32),
    draft: &str,
    account: &Option<source::AuthSession>,
) {
    assert_eq!(state.reader.body, body);
    assert_eq!(state.reader.selection, Some(selection));
    assert_eq!(state.reader.draft.query, draft);
    assert_eq!(&state.jobs.session(), account);
    assert_eq!(state.reader.book_id, BOOK);
    assert_eq!(state.route, Route::Reader(BOOK.into(), 0));
}

fn rejected_hold() -> Result<()> {
    let body = cluster(16);
    let fixture = Fixture::new(&body, Vec::new(), false)?;
    {
        let mut state = fixture.app();
        prepare(&mut state)?;
        note(&state, "retained-caret-note", 1)?;
        state.reader.selection = Some((0, 1));
        state
            .reader
            .draft
            .set_query("Draft before the rejected hold");
        fixture
            .ui()
            .global::<ReaderModel>()
            .set_writing_thought(true);
        let account = state.jobs.session();
        state.reader.caret_work_limit = 32;
        let origin = pointer(point(&state, 0, 5.0));
        state.reader_action("press", &origin)?;
        assert_retained(
            &state,
            &body,
            (0, 1),
            "Draft before the rejected hold",
            &account,
        );
        let generation = fixture
            .ui()
            .global::<ReaderModel>()
            .get_press_generation()
            .to_string();
        state.reader_action("hold", &generation)?;
        assert_retained(
            &state,
            &body,
            (0, 1),
            "Draft before the rejected hold",
            &account,
        );
        let moved = pointer(point(&state, 0, 20.0));
        state.reader_action("move", &moved)?;
        assert_retained(
            &state,
            &body,
            (0, 1),
            "Draft before the rejected hold",
            &account,
        );
        state.reader_action("release", &origin)?;
        assert_retained(
            &state,
            &body,
            (0, 1),
            "Draft before the rejected hold",
            &account,
        );
        let stats = state.reader.caret_stats.get();
        assert!(stats.rejected > 0);
        assert_eq!(stats.measured, 0);
        assert_eq!(stats.allocations, 0);
        assert_eq!(stats.boundaries, 0);
        assert_eq!(
            state.store.annotations(BOOK)?[0].note,
            "Retained local note"
        );
    }
    fixture.finish()
}

fn rejected_drag() -> Result<()> {
    let body = format!("AB\n{}", cluster(64));
    let fixture = Fixture::new(&body, Vec::new(), false)?;
    {
        let mut state = fixture.app();
        prepare(&mut state)?;
        assert_eq!(
            state
                .reader
                .paginator
                .as_ref()
                .unwrap()
                .current_page()
                .lines
                .len(),
            2
        );
        let account = state.jobs.session();
        state.reader.caret_work_limit = 128;
        let origin = pointer(point(&state, 0, 5.0));
        state.reader_action("press", &origin)?;
        assert_eq!(state.reader.caret_stats.get(), CaretStats::default());
        let generation = fixture
            .ui()
            .global::<ReaderModel>()
            .get_press_generation()
            .to_string();
        state.reader_action("hold", &generation)?;
        let selection = state
            .reader
            .selection
            .context("The small origin line must be admitted")?;
        assert_eq!(selection, (0, 2));
        // Hold resolves the anchor and then its preview focus. Both are the
        // legal two-scalar origin; neither enters the long destination line.
        assert_eq!(
            state.reader.caret_stats.get(),
            CaretStats {
                admitted: 2,
                allocations: 2,
                measured: 4,
                ..Default::default()
            }
        );
        state
            .reader
            .draft
            .set_query("Draft after the admitted origin");
        fixture
            .ui()
            .global::<ReaderModel>()
            .set_writing_thought(true);
        state.render_reader()?;
        let admitted = state.reader.caret_stats.get();
        // Publishing the selected origin adds one bounded AB snapshot. The
        // unselected destination needs no caret vector during this redraw.
        assert_eq!(
            admitted,
            CaretStats {
                admitted: 3,
                allocations: 3,
                measured: 6,
                ..Default::default()
            }
        );
        let rows = fixture.ui().global::<ReaderModel>().get_lines();
        assert_eq!(rows.row_data(0).unwrap().marks.row_count(), 1);
        assert_eq!(rows.row_data(1).unwrap().marks.row_count(), 0);
        let destination = pointer(point(&state, 1, 5.0));
        state.reader_action("move", &destination)?;
        assert_eq!(
            state.reader.caret_stats.get(),
            CaretStats {
                rejected: 1,
                ..admitted
            }
        );
        assert_retained(
            &state,
            &body,
            selection,
            "Draft after the admitted origin",
            &account,
        );
        state.reader_action("preview", &generation)?;
        assert_eq!(
            state.reader.caret_stats.get(),
            CaretStats {
                rejected: 2,
                ..admitted
            }
        );
        assert_retained(
            &state,
            &body,
            selection,
            "Draft after the admitted origin",
            &account,
        );
        state.reader_action("release", &destination)?;
        assert_retained(
            &state,
            &body,
            selection,
            "Draft after the admitted origin",
            &account,
        );
        let rejected = state.reader.caret_stats.get();
        assert_eq!(
            rejected,
            CaretStats {
                rejected: 3,
                ..admitted
            }
        );
        // A later real selection redraw is legal origin work, distinct from
        // the three rejected target operations. No history scan is admitted.
        state.render_reader()?;
        assert_eq!(
            state.reader.caret_stats.get(),
            CaretStats {
                admitted: 4,
                allocations: 4,
                measured: 8,
                rejected: 3,
                ..Default::default()
            }
        );
        let rows = fixture.ui().global::<ReaderModel>().get_lines();
        assert_eq!(rows.row_data(0).unwrap().marks.row_count(), 1);
        assert_eq!(rows.row_data(1).unwrap().marks.row_count(), 0);
        assert_retained(
            &state,
            &body,
            selection,
            "Draft after the admitted origin",
            &account,
        );
    }
    fixture.finish()
}

fn rejected_local_marks() -> Result<()> {
    let body = cluster(16);
    let fixture = Fixture::new(&body, Vec::new(), false)?;
    {
        let mut state = fixture.app();
        prepare(&mut state)?;
        note(&state, "local-caret-mark", 1)?;
        state.reader.selection = Some((0, 1));
        state
            .reader
            .draft
            .set_query("Draft before rejected local geometry");
        fixture
            .ui()
            .global::<ReaderModel>()
            .set_writing_thought(true);
        state.reader.caret_work_limit = 32;
        let account = state.jobs.session();
        for _ in 0..32 {
            state.render_reader()?;
            assert_retained(
                &state,
                &body,
                (0, 1),
                "Draft before rejected local geometry",
                &account,
            );
        }
        assert!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_lines()
                .iter()
                .all(|line| line.marks.row_count() == 0)
        );
        let (x, y) = point(&state, 0, 5.0);
        assert_eq!(state.reader_hit_mark(x, y)?, None);
        let stats = state.reader.caret_stats.get();
        assert!(stats.rejected >= 33);
        assert_eq!(stats.allocations, 0);
        assert_eq!(stats.measured, 0);
        assert_eq!(stats.boundaries, 0);
        assert_retained(
            &state,
            &body,
            (0, 1),
            "Draft before rejected local geometry",
            &account,
        );
        let stored = state.store.annotations(BOOK)?;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, "local-caret-mark");
        assert_eq!(stored[0].quote, "A");
        assert_eq!(stored[0].note, "Retained local note");
    }
    fixture.finish()
}

fn admitted_snapshot_and_hidden() -> Result<()> {
    let body = cluster(16);
    let fixture = Fixture::new(&body, Vec::new(), false)?;
    {
        let mut state = fixture.app();
        prepare(&mut state)?;
        for _ in 0..16 {
            state.render_reader()?;
        }
        let (x, y) = point(&state, 0, 5.0);
        assert_eq!(state.reader_hit_mark(x, y)?, None);
        assert_eq!(state.reader.caret_stats.get(), CaretStats::default());
        note(&state, "caret-note-a", 1)?;
        note(&state, "caret-note-cluster", 17)?;
        state.reader.selection = Some((0, 17));
        state.render_reader()?;
        let stats = state.reader.caret_stats.get();
        assert_eq!(stats.admitted, 1);
        assert_eq!(stats.allocations, 1);
        assert_eq!(stats.measured, 18);
        assert_eq!(stats.boundaries, 170);
        let marks = fixture
            .ui()
            .global::<ReaderModel>()
            .get_lines()
            .row_data(0)
            .unwrap()
            .marks;
        assert_eq!(marks.row_count(), 3);
        assert!(marks.iter().all(|mark| mark.width > 0.0 && mark.x >= 0.0));
        state.reader.caret_stats.set(CaretStats::default());
        assert_eq!(
            state.reader_hit_mark(x, y)?,
            Some("mine:caret-note-a".into())
        );
        let hit = state.reader.caret_stats.get();
        assert_eq!(hit.allocations, 1);
        assert_eq!(hit.measured, 18);
        assert_eq!(hit.boundaries, 170);
        assert_eq!(state.reader.body, body);
    }
    fixture.finish()
}

fn rejected_whole_page() -> Result<()> {
    let body = format!("AB\n{}", cluster(64));
    let fixture = Fixture::new(&body, Vec::new(), false)?;
    {
        let mut state = fixture.app();
        prepare(&mut state)?;
        note(&state, "page-caret-first", 1)?;
        state.store.save_annotation(&Annotation {
            id: "page-caret-second".into(),
            book_id: BOOK.into(),
            chapter_index: 0,
            start: 3,
            end: 4,
            quote: "A".into(),
            note: "Second retained local note".into(),
            created_at: 0,
        })?;
        state.reader.selection = Some((0, 1));
        state
            .reader
            .draft
            .set_query("Draft during a rejected page overlay");
        fixture
            .ui()
            .global::<ReaderModel>()
            .set_writing_thought(true);
        let account = state.jobs.session();
        state.reader.caret_work_limit = 128;
        state.render_reader()?;
        let rows = fixture.ui().global::<ReaderModel>().get_lines();
        assert_eq!(rows.row_count(), 2);
        assert!(rows.iter().all(|row| row.marks.row_count() == 0
            && !row.mine
            && !row.highlight
            && row.count == 0));
        let rendered = state.reader.caret_stats.get();
        assert_eq!(rendered.allocations, 0);
        assert_eq!(rendered.measured, 0);
        assert_eq!(rendered.rejected, 1);
        state.reader.caret_stats.set(CaretStats::default());
        for index in 0..2 {
            let (x, y) = point(&state, index, 5.0);
            assert_eq!(state.reader_hit_mark(x, y)?, None);
        }
        let hit = state.reader.caret_stats.get();
        assert_eq!(hit.allocations, 0);
        assert_eq!(hit.measured, 0);
        assert_eq!(hit.rejected, 2);
        assert_retained(
            &state,
            &body,
            (0, 1),
            "Draft during a rejected page overlay",
            &account,
        );
        assert_eq!(state.store.annotations(BOOK)?.len(), 2);
    }
    fixture.finish()
}

#[test]
fn local_caret_registered_font_rejected_hold_drag_release_keeps_existing_draft() -> Result<()> {
    isolated(
        "app::local_caret_tests::local_caret_registered_font_rejected_hold_drag_release_keeps_existing_draft",
        rejected_hold,
    )
}

#[test]
fn local_caret_registered_font_rejected_drag_keeps_admitted_origin_selection() -> Result<()> {
    isolated(
        "app::local_caret_tests::local_caret_registered_font_rejected_drag_keeps_admitted_origin_selection",
        rejected_drag,
    )
}

#[test]
fn local_caret_registered_font_mark_and_hit_rejections_keep_notes_and_account() -> Result<()> {
    isolated(
        "app::local_caret_tests::local_caret_registered_font_mark_and_hit_rejections_keep_notes_and_account",
        rejected_local_marks,
    )
}

#[test]
fn local_caret_registered_font_reuses_line_snapshot_and_skips_overlay_free_reads() -> Result<()> {
    isolated(
        "app::local_caret_tests::local_caret_registered_font_reuses_line_snapshot_and_skips_overlay_free_reads",
        admitted_snapshot_and_hidden,
    )
}

#[test]
fn local_caret_page_rejection_withholds_all_marks_and_hits_without_losing_draft() -> Result<()> {
    isolated(
        "app::local_caret_tests::local_caret_page_rejection_withholds_all_marks_and_hits_without_losing_draft",
        rejected_whole_page,
    )
}
