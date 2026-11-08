//! Bounded keyboard admission without truncating drafts or composition.

use super::*;

fn snapshot(
    input: &PinyinInput,
) -> (
    String,
    String,
    InputLanguage,
    bool,
    usize,
    InputLanguage,
    InputPurpose,
    u64,
) {
    (
        input.query.clone(),
        input.composition.clone(),
        input.language,
        input.caps,
        input.candidate_page,
        input.previous_language,
        input.purpose,
        input.query_revision(),
    )
}

fn composing(query: &str, spelling: &str) -> PinyinInput {
    let mut input = PinyinInput::new(query);
    for character in spelling.chars() {
        assert_eq!(input.press(&character.to_string()), InputIntent::Changed);
    }
    input
}

#[test]
fn borrowed_initialization_uses_the_selected_purpose_and_fails_closed() {
    let source = "x".repeat(SEARCH_INPUT_SCALARS + 1);
    assert!(PinyinInput::for_purpose(&source, InputPurpose::Search).is_err());
    let thought = PinyinInput::for_purpose(&source, InputPurpose::Thought).unwrap();
    assert_eq!(thought.query, source);
    assert_eq!(thought.purpose(), InputPurpose::Thought);
    assert!(thought.is_admitted());

    let mut rejected = PinyinInput::new(&source);
    assert!(rejected.query.is_empty());
    assert!(!rejected.is_admitted());
    assert_eq!(rejected.press("submit"), InputIntent::Rejected);
    assert_eq!(source.len(), SEARCH_INPUT_SCALARS + 1);
    assert!(rejected.set_query("Admitted replacement"));
    assert_eq!(rejected.press("submit"), InputIntent::Submit);
    assert_eq!(
        InputPurpose::Thought.maximum_scalars(),
        THOUGHT_INPUT_SCALARS
    );
}

#[test]
fn rejected_edits_and_purpose_changes_preserve_the_complete_previous_input() {
    let mut input = composing("AB", "shi");
    input.set_test_scalar_limit(3);
    input.candidate_page = 1;
    let before = snapshot(&input);
    assert!(!input.set_query("ABCD"));
    assert_eq!(snapshot(&input), before);

    let note = "n".repeat(SEARCH_INPUT_SCALARS + 1);
    assert!(input.set_query_for_purpose(&note, InputPurpose::Thought));
    let before = snapshot(&input);
    assert!(!input.set_purpose(InputPurpose::Search));
    assert_eq!(snapshot(&input), before);
    assert_eq!(input.query, note);
}

#[test]
fn english_space_period_and_repeated_keys_cannot_grow_a_full_query() {
    let mut input = PinyinInput::new("ABC");
    input.language = InputLanguage::English;
    input.set_test_scalar_limit(3);
    let before = snapshot(&input);
    for _ in 0..32 {
        for key in ["x", "space", "period"] {
            assert_eq!(input.press(key), InputIntent::Rejected);
            assert_eq!(snapshot(&input), before);
        }
    }
    input.press("backspace");
    assert_eq!(input.query, "AB");
    assert_eq!(input.press("space"), InputIntent::Changed);
    assert_eq!(input.query, "AB ");
    assert_eq!(input.press("period"), InputIntent::Rejected);
}

#[test]
fn uppercase_expansion_is_admitted_as_all_its_unicode_scalars() {
    let mut input = PinyinInput::new("A");
    input.language = InputLanguage::English;
    input.caps = true;
    input.set_test_scalar_limit(2);
    let before = snapshot(&input);
    assert_eq!(input.press("\u{df}"), InputIntent::Rejected);
    assert_eq!(snapshot(&input), before);
    input.set_test_scalar_limit(3);
    assert_eq!(input.press("\u{df}"), InputIntent::Changed);
    assert_eq!(input.query, "ASS");
    assert_eq!(input.query.chars().count(), 3);
    assert_eq!(input.press("x"), InputIntent::Rejected);
}

#[test]
fn chinese_candidates_and_space_keep_composition_on_capacity_rejection() {
    let mut input = composing("A", "shi");
    let candidate = input.candidates().into_iter().next().unwrap();
    input.set_test_scalar_limit(1);
    let before = snapshot(&input);
    assert!(!input.choose(&candidate));
    assert_eq!(snapshot(&input), before);
    assert_eq!(input.press("space"), InputIntent::Rejected);
    assert_eq!(snapshot(&input), before);
    input.set_test_scalar_limit(1 + candidate.chars().count());
    assert_eq!(input.press("space"), InputIntent::Changed);
    assert_eq!(input.query, format!("A{candidate}"));
    assert!(input.composition.is_empty());
}

#[test]
fn language_numeric_submit_and_direct_characters_do_not_partially_commit() {
    let input = composing("A", "shi");
    for key in [
        "language", "numeric", "numbers", "submit", "enter", "search", "\u{4e2d}",
    ] {
        let mut input = input.clone();
        input.set_test_scalar_limit(1);
        input.candidate_page = 1;
        let before = snapshot(&input);
        assert_eq!(input.press(key), InputIntent::Rejected, "{key}");
        assert_eq!(snapshot(&input), before, "{key}");
    }
}

#[test]
fn a_period_prepays_both_composition_and_punctuation_before_either_commit() {
    let mut input = composing("A", "shi");
    let candidate = input.all_candidates().into_iter().next().unwrap();
    assert_eq!(candidate.consumed, input.composition.chars().count());
    input.set_test_scalar_limit(1 + candidate.text.chars().count());
    let before = snapshot(&input);
    assert_eq!(input.press("period"), InputIntent::Rejected);
    assert_eq!(snapshot(&input), before);
    input.set_test_scalar_limit(2 + candidate.text.chars().count());
    assert_eq!(input.press("period"), InputIntent::Changed);
    assert_eq!(input.query, format!("A{}\u{3002}", candidate.text));
    assert!(input.composition.is_empty());
}

#[test]
fn unconvertible_submission_suffix_is_kept_until_the_complete_spelling_fits() {
    let mut input = composing("AB", "''");
    input.set_test_scalar_limit(3);
    let before = snapshot(&input);
    assert!(!input.commit_first());
    assert_eq!(snapshot(&input), before);
    assert_eq!(input.press("submit"), InputIntent::Rejected);
    assert_eq!(snapshot(&input), before);
    input.set_test_scalar_limit(4);
    assert_eq!(input.press("submit"), InputIntent::Submit);
    assert_eq!(input.query, "AB''");
    assert!(input.composition.is_empty());
}

#[test]
fn the_scalar_allowance_is_not_a_utf8_byte_allowance() {
    let mut input = PinyinInput::new("\u{1f9ea}\u{4e2d}");
    input.language = InputLanguage::English;
    input.set_test_scalar_limit(3);
    assert_eq!(input.press("\u{1f600}"), InputIntent::Changed);
    assert_eq!(input.query.chars().count(), 3);
    assert!(input.query.len() > 3);
    let before = snapshot(&input);
    assert_eq!(input.press("space"), InputIntent::Rejected);
    assert_eq!(snapshot(&input), before);
}
