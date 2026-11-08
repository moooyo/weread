//! Real App callbacks for Search and Reader draft capacity boundaries.

use super::underline_projection_tests::{BOOK, Fixture, isolated};
use super::*;
use crate::input::{InputLanguage, InputPurpose, SEARCH_INPUT_SCALARS, THOUGHT_INPUT_SCALARS};
use slint::Model;

fn account() -> source::AuthSession {
    source::AuthSession {
        api_key: "synthetic-input-key".into(),
        user_vid: "synthetic-input-account".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "synthetic-input-account".into()),
            ("wr_skey".into(), "synthetic-input-session".into()),
        ]),
        ..Default::default()
    }
}

fn search_event(fixture: &Fixture, action: &str, value: &str) {
    fixture
        .ui()
        .global::<SearchModel>()
        .invoke_event(action.into(), value.into());
}

fn reader_event(fixture: &Fixture, action: &str, value: &str) {
    fixture
        .ui()
        .global::<ReaderModel>()
        .invoke_event(action.into(), value.into());
}

fn prepare_search(state: &mut App, text: &str, maximum: usize) -> Result<()> {
    state.jobs.replace_session(account())?;
    state.query = PinyinInput::new(text);
    state.query.set_test_scalar_limit(maximum);
    state.search_purpose = "books".into();
    state.search_scope = "shelf".into();
    state.navigate(Route::Search, true)?;
    state.render_search()
}

fn english_callbacks() -> Result<()> {
    let fixture = Fixture::new("Reader input fixture.", Vec::new(), false)?;
    {
        prepare_search(&mut fixture.app(), "ABCD", 4)?;
    }
    search_event(&fixture, "key", "language");
    let (request, revision, body, account) = {
        let state = fixture.app();
        assert_eq!(state.query.language, InputLanguage::English);
        (
            state.search_request,
            state.query.query_revision(),
            state.reader.body.clone(),
            state.jobs.session(),
        )
    };
    for _ in 0..16 {
        for key in ["x", "space", "period"] {
            search_event(&fixture, "key", key);
        }
    }
    {
        let state = fixture.app();
        assert_eq!(state.query.query, "ABCD");
        assert!(state.query.composition.is_empty());
        assert_eq!(state.query.query_revision(), revision);
        assert_eq!(state.search_request, request);
        assert_eq!(
            fixture.ui().global::<SearchModel>().get_query().as_str(),
            "ABCD"
        );
        assert_eq!(state.reader.body, body);
        assert_eq!(state.jobs.session(), account);
    }
    fixture.finish()
}

#[test]
fn genuine_search_keyboard_callbacks_cannot_extend_a_full_query() -> Result<()> {
    isolated(
        "app::input_limit_tests::genuine_search_keyboard_callbacks_cannot_extend_a_full_query",
        english_callbacks,
    )
}

fn candidates_paste_and_submit() -> Result<()> {
    let fixture = Fixture::new("Reader input fixture.", Vec::new(), false)?;
    {
        prepare_search(&mut fixture.app(), "FULL", 4)?;
    }
    for character in "shi".chars() {
        search_event(&fixture, "key", &character.to_string());
    }
    let candidate_pages = {
        let state = fixture.app();
        state.query.candidate_page_count()
    };
    if candidate_pages > 1 {
        search_event(&fixture, "key", "candidate-next");
    }
    let (candidate, composition, page, request, history, account) = {
        let state = fixture.app();
        (
            state.query.candidates().into_iter().next().unwrap(),
            state.query.composition.clone(),
            state.query.candidate_page,
            state.search_request,
            state.store.recent_queries(4)?,
            state.jobs.session(),
        )
    };
    search_event(&fixture, "candidate", &candidate);
    fixture.ui().global::<SearchModel>().set_paste_ready(true);
    search_event(&fixture, "prepare-paste", "");
    assert!(!fixture.ui().global::<SearchModel>().get_paste_ready());
    for _ in 0..8 {
        search_event(&fixture, "key", "submit");
    }
    {
        let state = fixture.app();
        assert_eq!(state.query.query, "FULL");
        assert_eq!(state.query.composition, composition);
        assert_eq!(state.query.candidate_page, page);
        assert_eq!(state.search_request, request);
        assert_eq!(state.store.recent_queries(4)?, history);
        assert_eq!(state.jobs.session(), account);
        assert!(fixture.ui().global::<SearchModel>().get_keyboard_visible());
        assert_eq!(
            fixture
                .ui()
                .global::<SearchModel>()
                .get_composition()
                .as_str(),
            composition
        );
    }
    fixture.finish()
}

#[test]
fn genuine_candidate_paste_and_submit_rejections_preserve_composition_and_history() -> Result<()> {
    isolated(
        "app::input_limit_tests::genuine_candidate_paste_and_submit_rejections_preserve_composition_and_history",
        candidates_paste_and_submit,
    )
}

fn thought_purpose_and_historical_text() -> Result<()> {
    let fixture = Fixture::new("Reader input fixture.", Vec::new(), false)?;
    let initial = "n".repeat(SEARCH_INPUT_SCALARS + 1);
    {
        let mut state = fixture.app();
        state.jobs.replace_session(account())?;
        state.reader.selection = Some((0, 4));
        assert!(state.reader.draft.set_query(&initial));
    }
    reader_event(&fixture, "reader.write-thought", "selection");
    {
        let state = fixture.app();
        assert_eq!(state.route, Route::Search);
        assert_eq!(state.search_purpose, "thought");
        assert_eq!(state.query.purpose(), InputPurpose::Thought);
        assert_eq!(state.query.query, initial);
        assert_eq!(
            fixture.ui().global::<SearchModel>().get_query().as_str(),
            initial
        );
    }
    let edited = "m".repeat(5000);
    search_event(&fixture, "edit-query", &edited);
    for character in "shi".chars() {
        search_event(&fixture, "key", &character.to_string());
    }
    let (composition, account) = {
        let state = fixture.app();
        (state.query.composition.clone(), state.jobs.session())
    };
    let historical = "h".repeat(THOUGHT_INPUT_SCALARS + 1);
    search_event(&fixture, "edit-query", &historical);
    {
        let state = fixture.app();
        assert_eq!(state.query.query, edited);
        assert_eq!(state.query.composition, composition);
    }
    search_event(&fixture, "back", "");
    {
        let state = fixture.app();
        assert_eq!(state.route, Route::Reader(BOOK.into(), 0));
        assert_eq!(state.reader.draft.query, edited);
        assert_eq!(state.reader.draft.composition, composition);
        state.store.save_annotation(&Annotation {
            id: "historical-input-note".into(),
            book_id: BOOK.into(),
            chapter_index: 0,
            start: 0,
            end: 4,
            quote: "Read".into(),
            note: historical.clone(),
            created_at: 1,
        })?;
    }
    reader_event(&fixture, "reader.thought-draft", &historical);
    reader_event(&fixture, "reader.thought", "mine:historical-input-note");
    {
        let state = fixture.app();
        assert_eq!(state.reader.draft.query, edited);
        assert_eq!(state.reader.draft.composition, composition);
        assert_eq!(state.reader.selection, Some((0, 4)));
        assert_eq!(state.jobs.session(), account);
        let saved = state
            .store
            .annotations(BOOK)?
            .into_iter()
            .find(|note| note.id == "historical-input-note")
            .unwrap();
        assert_eq!(saved.note, historical);
        assert_eq!(state.reader.body, "Reader input fixture.");
    }
    fixture.finish()
}

#[test]
fn genuine_note_inputs_keep_the_16k_purpose_and_preserve_oversized_stored_text() -> Result<()> {
    isolated(
        "app::input_limit_tests::genuine_note_inputs_keep_the_16k_purpose_and_preserve_oversized_stored_text",
        thought_purpose_and_historical_text,
    )
}

fn pending_note_submission() -> Result<()> {
    let fixture = Fixture::new("Reader input fixture.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        state.reader.selection = Some((0, 4));
        assert!(state.reader.draft.set_query("ABC"));
        state.reader.draft.set_test_scalar_limit(3);
    }
    reader_event(&fixture, "reader.write-thought", "selection");
    for character in "shi".chars() {
        search_event(&fixture, "key", &character.to_string());
    }
    search_event(&fixture, "back", "");
    let candidate = {
        let state = fixture.app();
        assert_eq!(state.reader.draft.composition, "shi");
        assert!(state.store.annotations(BOOK)?.is_empty());
        state
            .reader
            .draft
            .all_candidates()
            .into_iter()
            .next()
            .unwrap()
    };
    reader_event(&fixture, "reader.save-thought", "");
    {
        let mut state = fixture.app();
        assert_eq!(state.reader.draft.query, "ABC");
        assert_eq!(state.reader.draft.composition, "shi");
        assert_eq!(state.reader.selection, Some((0, 4)));
        assert!(state.store.annotations(BOOK)?.is_empty());
        assert!(fixture.ui().global::<ReaderModel>().get_writing_thought());
        state
            .reader
            .draft
            .set_test_scalar_limit(3 + candidate.text.chars().count());
    }
    reader_event(&fixture, "reader.save-thought", "");
    {
        let state = fixture.app();
        let notes = state.store.annotations(BOOK)?;
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].note, format!("ABC{}", candidate.text));
        assert_eq!(notes[0].quote, "Read");
        assert!(state.reader.draft.query.is_empty());
        assert!(state.reader.draft.composition.is_empty());
    }
    fixture.finish()
}

#[test]
fn a_note_save_never_drops_unconverted_input_after_returning_from_search() -> Result<()> {
    isolated(
        "app::input_limit_tests::a_note_save_never_drops_unconverted_input_after_returning_from_search",
        pending_note_submission,
    )
}

fn retained_editor(
    state: &App,
) -> (
    String,
    String,
    usize,
    InputPurpose,
    u64,
    Option<(u32, u32)>,
    String,
    Option<source::AuthSession>,
) {
    (
        state.reader.draft.query.clone(),
        state.reader.draft.composition.clone(),
        state.reader.draft.candidate_page,
        state.reader.draft.purpose(),
        state.reader.draft.query_revision(),
        state.reader.selection,
        state.reader.body.clone(),
        state.jobs.session(),
    )
}

fn preserve_pending_editor(state: &mut App) {
    state.reader.selection = Some((0, 4));
    assert!(state.reader.draft.set_query("Preserved previous draft"));
    for character in "shi".chars() {
        state.reader.draft.press(&character.to_string());
    }
    state.reader.draft.candidate_page = 1;
}

fn save_local_note(state: &App, id: &str, text: &str, start: u32, end: u32) -> Result<()> {
    state.store.save_annotation(&Annotation {
        id: id.into(),
        book_id: BOOK.into(),
        chapter_index: 0,
        start,
        end,
        quote: state
            .reader
            .body
            .chars()
            .skip(start as usize)
            .take((end - start) as usize)
            .collect(),
        note: text.into(),
        created_at: 1,
    })
}

fn stored_note(state: &App, id: &str) -> Result<String> {
    Ok(state
        .store
        .annotations(BOOK)?
        .into_iter()
        .find(|note| note.id == id)
        .unwrap()
        .note)
}

fn refuse_readonly_editor(fixture: &Fixture) {
    reader_event(fixture, "reader.write-thought", "selection");
    reader_event(
        fixture,
        "reader.thought-draft",
        "Rejected replacement draft",
    );
    reader_event(fixture, "reader.save-thought", "");
    reader_event(fixture, "reader.highlight", "thread");
    let previous_thought_editor = fixture.app().search_purpose == "thought";
    if previous_thought_editor {
        search_event(fixture, "edit-query", "Rejected previous Thought editor");
        search_event(fixture, "key", "submit");
    }
}

fn unavailable_readonly_note() -> Result<()> {
    let fixture = Fixture::new(
        "Reader input fixture.",
        vec![RemoteUnderline {
            text: "Read".into(),
            range: "synthetic-readonly-range".into(),
            count: 1,
            thoughts: 1,
        }],
        true,
    )?;
    {
        fixture.app().jobs.replace_session(account())?;
    }
    reader_event(&fixture, "reader.thought", "remote:0");
    let unavailable = "u".repeat(super::thoughts::OPEN_TEXT_BYTES + 1);
    let kept = {
        let mut state = fixture.app();
        state.reader.remote_thoughts = vec![RemoteReview {
            id: "previous-review".into(),
            text: "Previous bounded review".into(),
            ..Default::default()
        }];
        state.render_reader()?;
        assert_eq!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_thoughts()
                .row_data(0)
                .unwrap()
                .body
                .as_str(),
            "Previous bounded review",
        );
        preserve_pending_editor(&mut state);
        save_local_note(&state, "unavailable-readonly-note", &unavailable, 0, 4)?;
        save_local_note(
            &state,
            "admitted-editable-note",
            "Small existing note",
            7,
            12,
        )?;
        retained_editor(&state)
    };
    reader_event(&fixture, "reader.thought", "mine:unavailable-readonly-note");
    {
        let state = fixture.app();
        assert_eq!(state.reader.thought_id, "mine:unavailable-readonly-note");
        assert!(state.reader.thought_edit_blocked);
        assert_eq!(retained_editor(&state), kept);
        assert!(fixture.ui().global::<ReaderModel>().get_thought_open());
        assert!(!fixture.ui().global::<ReaderModel>().get_writing_thought());
        assert_eq!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_thoughts()
                .row_data(0)
                .unwrap()
                .body
                .as_str(),
            tr("api_error"),
        );
    }
    refuse_readonly_editor(&fixture);
    {
        let state = fixture.app();
        assert_eq!(retained_editor(&state), kept);
        assert_eq!(
            stored_note(&state, "unavailable-readonly-note")?,
            unavailable
        );
    }
    reader_event(&fixture, "reader.thought", "mine:admitted-editable-note");
    {
        let state = fixture.app();
        assert!(!state.reader.thought_edit_blocked);
        assert_eq!(state.reader.draft.query, "Small existing note");
        assert!(state.reader.draft.composition.is_empty());
        assert_eq!(state.reader.selection, Some((7, 12)));
    }
    reader_event(&fixture, "reader.write-thought", "thread");
    search_event(&fixture, "edit-query", "Updated admitted note");
    search_event(&fixture, "key", "submit");
    {
        let state = fixture.app();
        assert_eq!(
            stored_note(&state, "admitted-editable-note")?,
            "Updated admitted note"
        );
        assert_eq!(
            stored_note(&state, "unavailable-readonly-note")?,
            unavailable
        );
        assert_eq!(state.reader.body, "Reader input fixture.");
        assert_eq!(state.jobs.session(), kept.7);
    }
    reader_event(&fixture, "reader.thought", "mine:unavailable-readonly-note");
    {
        let mut state = fixture.app();
        assert!(state.reader.thought_edit_blocked);
        state.invalidate_account_work()?;
        state.jobs.clear_session();
        state.discard_cancelled_job_events();
        assert!(!state.reader.thought_edit_blocked);
        assert!(state.reader.thought_id.is_empty());
        assert!(state.jobs.session().is_none());
        assert_eq!(
            stored_note(&state, "unavailable-readonly-note")?,
            unavailable
        );
    }
    fixture.finish()
}

#[test]
fn unavailable_historical_notes_open_readonly_and_an_admitted_note_restores_editing() -> Result<()>
{
    isolated(
        "app::input_limit_tests::unavailable_historical_notes_open_readonly_and_an_admitted_note_restores_editing",
        unavailable_readonly_note,
    )
}

fn visible_readonly_note_and_new_selection() -> Result<()> {
    let fixture = Fixture::new("Reader input fixture.", Vec::new(), false)?;
    let visible = "v".repeat(THOUGHT_INPUT_SCALARS + 1);
    assert!(visible.len() <= super::thoughts::OPEN_TEXT_BYTES);
    let kept = {
        let mut state = fixture.app();
        state.jobs.replace_session(account())?;
        preserve_pending_editor(&mut state);
        save_local_note(&state, "visible-readonly-note", &visible, 0, 4)?;
        retained_editor(&state)
    };
    reader_event(&fixture, "reader.write-thought", "selection");
    search_event(&fixture, "back", "");
    {
        let state = fixture.app();
        assert_eq!(state.search_purpose, "thought");
        assert_eq!(retained_editor(&state), kept);
    }
    reader_event(&fixture, "reader.thought", "mine:visible-readonly-note");
    {
        let state = fixture.app();
        assert!(state.reader.thought_edit_blocked);
        assert_eq!(retained_editor(&state), kept);
        assert!(!fixture.ui().global::<ReaderModel>().get_writing_thought());
        assert_eq!(
            fixture
                .ui()
                .global::<ReaderModel>()
                .get_thoughts()
                .row_data(0)
                .unwrap()
                .body
                .as_str(),
            visible,
        );
    }
    refuse_readonly_editor(&fixture);
    reader_event(&fixture, "reader.thought-close", "");
    reader_event(&fixture, "reader.save-thought", "");
    {
        let state = fixture.app();
        assert_eq!(retained_editor(&state), kept);
        assert!(state.reader.thought_edit_blocked);
        assert_eq!(state.query.query, kept.0);
        assert_eq!(state.query.composition, kept.1);
        assert_eq!(stored_note(&state, "visible-readonly-note")?, visible);
        assert!(!fixture.ui().global::<ReaderModel>().get_thought_open());
    }
    reader_event(&fixture, "reader.press", "0.2,8");
    let generation = fixture.ui().global::<ReaderModel>().get_press_generation();
    reader_event(&fixture, "reader.hold", &generation.to_string());
    reader_event(&fixture, "reader.release", "0.2,8");
    {
        let state = fixture.app();
        assert!(!state.reader.thought_edit_blocked);
        assert!(state.reader.thought_id.is_empty());
        assert!(state.reader.draft.query.is_empty());
        assert!(state.reader.draft.composition.is_empty());
        assert!(state.reader.selection.is_some());
        assert_ne!(state.reader.selection, Some((0, 4)));
    }
    reader_event(&fixture, "reader.write-thought", "selection");
    search_event(&fixture, "edit-query", "Fresh selected note");
    search_event(&fixture, "key", "submit");
    {
        let state = fixture.app();
        assert_eq!(stored_note(&state, "visible-readonly-note")?, visible);
        assert!(
            state
                .store
                .annotations(BOOK)?
                .iter()
                .any(|note| note.note == "Fresh selected note")
        );
        assert_eq!(state.reader.body, "Reader input fixture.");
        assert_eq!(state.jobs.session(), kept.7);
    }
    reader_event(&fixture, "reader.thought", "mine:visible-readonly-note");
    {
        let mut state = fixture.app();
        assert!(state.reader.thought_edit_blocked);
        state.load_local_reader(BOOK, 1, None)?;
        assert!(!state.reader.thought_edit_blocked);
        assert!(state.reader.thought_id.is_empty());
        assert_eq!(state.reader.body, "Second chapter marker.");
        assert_eq!(stored_note(&state, "visible-readonly-note")?, visible);
    }
    fixture.finish()
}

#[test]
fn displayable_overlimit_notes_preserve_old_drafts_until_a_new_selection_or_document() -> Result<()>
{
    isolated(
        "app::input_limit_tests::displayable_overlimit_notes_preserve_old_drafts_until_a_new_selection_or_document",
        visible_readonly_note_and_new_selection,
    )
}
