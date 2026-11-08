use super::*;
use crate::domain::ContentStatus;
use serde_json::{Value, json};

const NOW: i64 = 2_000_000_000;
const BOOK: &str = "fixture-book";
const UID: &str = "1000";
const ACCOUNT: &str = "fixture-account";
const TRIAL_EDITION: u64 = 1 << 19;

fn fixture() -> Value {
    let chapter = json!({
        "chapterUid": UID,
        "chapterIdx": 10,
        "price": 10,
        "paid": 0,
        "wordCount": 1000
    });
    json!({
        "user": { "vid": ACCOUNT },
        "reader": {
            "bookId": BOOK,
            "bookInfo": {
                "bookId": BOOK,
                "paid": 0,
                "payType": 1,
                "payingStatus": 2,
                "maxFreeChapter": 0,
                "shortTimeRead": { "active": 0 }
            },
            "chapterInfos": [chapter.clone()],
            "currentChapter": chapter,
            "memberCardSummary": {
                "isPaying": 0,
                "expired": 1,
                "expiredTime": NOW - 1,
                "freeBookIds": []
            }
        }
    })
}

fn access(state: &Value) -> ReaderAccess {
    ReaderAccess::from_state(state, BOOK, UID, Some(ACCOUNT), NOW)
}

fn complete_text() -> String {
    "A complete synthetic passage with enough content. ".repeat(100)
}

fn status(state: &Value) -> ContentStatus {
    access(state).status_for_text(&complete_text())
}

fn edit_chapter(state: &mut Value, edit: impl FnOnce(&mut Value)) {
    let mut chapter = state["reader"]["chapterInfos"][0].clone();
    edit(&mut chapter);
    state["reader"]["chapterInfos"] = json!([chapter.clone()]);
    state["reader"]["currentChapter"] = chapter;
}

fn valid_member(state: &mut Value, paying: bool) {
    state["reader"]["memberCardSummary"] = json!({
        "isPaying": if paying { 1 } else { 0 },
        "expired": 0,
        "expiredTime": NOW + 1000,
        "freeBookIds": []
    });
}

#[test]
fn an_unpurchased_book_can_have_an_explicit_valid_member_grant() {
    for paying_status in [1, 3] {
        for paying in [false, true] {
            let mut state = fixture();
            state["reader"]["bookInfo"]["payingStatus"] = json!(paying_status);
            valid_member(&mut state, paying);
            assert_eq!(status(&state), ContentStatus::Full);
            assert_eq!(state["reader"]["bookInfo"]["paid"], 0);
        }
    }
}

#[test]
fn an_expired_member_does_not_authorize_an_unpurchased_chapter() {
    for expiry in [NOW - 1, NOW] {
        let mut state = fixture();
        state["reader"]["bookInfo"]["payingStatus"] = json!(1);
        state["reader"]["memberCardSummary"]["expiredTime"] = json!(expiry);
        let access = access(&state);
        assert_eq!(
            access.status_for_text(&complete_text()),
            ContentStatus::Preview
        );
        assert_eq!(access.status_for_text(""), ContentStatus::Locked);
    }
}

#[test]
fn contradictory_paid_membership_stays_unknown() {
    let mut state = fixture();
    state["reader"]["bookInfo"]["payingStatus"] = json!(1);
    state["reader"]["memberCardSummary"]["isPaying"] = json!(1);
    assert_eq!(status(&state), ContentStatus::Unknown);
    assert_eq!(access(&state).status_for_text(""), ContentStatus::Unknown);
}

#[test]
fn missing_member_summary_is_not_proof_that_membership_is_absent() {
    for summary in [json!({}), Value::Null, json!([])] {
        let mut state = fixture();
        state["reader"]["memberCardSummary"] = summary;
        assert_eq!(status(&state), ContentStatus::Unknown);
    }
    let mut state = fixture();
    state["reader"]
        .as_object_mut()
        .unwrap()
        .remove("memberCardSummary");
    assert_eq!(status(&state), ContentStatus::Unknown);
}

#[test]
fn free_and_limit_free_bits_are_independent_positive_grants() {
    for pay_type in [32, 64] {
        let mut state = fixture();
        state["reader"]["bookInfo"]["payType"] = json!(pay_type);
        state["reader"]["bookInfo"]["payingStatus"] = json!(6);
        state["reader"]["memberCardSummary"] = json!({});
        assert_eq!(status(&state), ContentStatus::Full);
    }
}

#[test]
fn normal_free_chapter_boundaries_use_the_target_index_instead_of_its_uid() {
    let mut state = fixture();
    state["reader"]["bookInfo"]["maxFreeChapter"] = json!(3);
    edit_chapter(&mut state, |chapter| {
        chapter["chapterIdx"] = json!(2);
        chapter["price"] = json!(0);
    });
    assert_eq!(status(&state), ContentStatus::Full);

    edit_chapter(&mut state, |chapter| {
        chapter["chapterUid"] = json!("1");
        chapter["chapterIdx"] = json!(10);
    });
    let access = ReaderAccess::from_state(&state, BOOK, "1", Some(ACCOUNT), NOW);
    assert_eq!(
        access.status_for_text(&complete_text()),
        ContentStatus::Preview
    );
}

#[test]
fn explicit_book_and_chapter_purchases_authorize_the_bound_account() {
    let mut book = fixture();
    book["reader"]["bookInfo"]["paid"] = json!(1);
    book["reader"]["memberCardSummary"] = json!({});
    assert_eq!(status(&book), ContentStatus::Full);

    let mut chapter = fixture();
    chapter["reader"]["bookInfo"]["payType"] = json!(2);
    chapter["reader"]["bookInfo"]["payingStatus"] = json!(6);
    chapter["reader"]["memberCardSummary"] = json!({});
    edit_chapter(&mut chapter, |chapter| chapter["paid"] = json!(1));
    assert_eq!(status(&chapter), ContentStatus::Full);
}

#[test]
fn absent_or_non_numeric_paid_values_do_not_create_free_access() {
    for paid in [
        Value::Null,
        json!("0"),
        json!(false),
        json!(true),
        json!(-1),
        json!(2),
    ] {
        let mut state = fixture();
        state["reader"]["bookInfo"]["paid"] = paid;
        assert_eq!(status(&state), ContentStatus::Unknown);
    }
    let mut missing = fixture();
    missing["reader"]["bookInfo"]
        .as_object_mut()
        .unwrap()
        .remove("paid");
    assert_eq!(status(&missing), ContentStatus::Unknown);

    let mut numeric_one = fixture();
    numeric_one["reader"]["bookInfo"]["paid"] = json!(1.0);
    assert_eq!(status(&numeric_one), ContentStatus::Full);
}

#[test]
fn per_chapter_price_zero_is_positive_but_invalid_prices_remain_unknown() {
    let mut state = fixture();
    state["reader"]["bookInfo"]["payType"] = json!(2);
    edit_chapter(&mut state, |chapter| chapter["price"] = json!(0));
    assert_eq!(status(&state), ContentStatus::Full);
    for price in [Value::Null, json!("0"), json!(-1), json!(false)] {
        let mut invalid = fixture();
        invalid["reader"]["bookInfo"]["payType"] = json!(2);
        edit_chapter(&mut invalid, |chapter| chapter["price"] = price);
        assert_eq!(status(&invalid), ContentStatus::Unknown);
    }
    let mut missing = fixture();
    missing["reader"]["bookInfo"]["payType"] = json!(2);
    edit_chapter(&mut missing, |chapter| {
        chapter.as_object_mut().unwrap().remove("price");
    });
    assert_eq!(status(&missing), ContentStatus::Unknown);
}

#[test]
fn book_and_target_chapter_binding_mismatches_remain_unknown() {
    let mut original = fixture();
    original["reader"]["bookInfo"]["paid"] = json!(1);
    let mut wrong_reader = original.clone();
    wrong_reader["reader"]["bookId"] = json!("other-fixture-book");
    let mut wrong_book = original.clone();
    wrong_book["reader"]["bookInfo"]["bookId"] = json!("other-fixture-book");
    let mut wrong_target = original.clone();
    wrong_target["reader"]["chapterInfos"][0]["chapterUid"] = json!("other-fixture-chapter");
    let mut wrong_current = original.clone();
    wrong_current["reader"]["currentChapter"]["chapterUid"] = json!("other-fixture-chapter");
    let mut duplicates = original;
    let duplicate = duplicates["reader"]["chapterInfos"][0].clone();
    duplicates["reader"]["chapterInfos"] = json!([duplicate.clone(), duplicate]);
    for state in [
        wrong_reader,
        wrong_book,
        wrong_target,
        wrong_current,
        duplicates,
    ] {
        assert_eq!(status(&state), ContentStatus::Unknown);
    }
}

#[test]
fn purchases_and_membership_cannot_cross_account_bindings() {
    let mut original = fixture();
    original["reader"]["bookInfo"]["paid"] = json!(1);
    for vid in [json!("other-fixture-account"), json!(0), Value::Null] {
        let mut state = original.clone();
        state["user"]["vid"] = vid;
        assert_eq!(status(&state), ContentStatus::Unknown);
    }
    original.as_object_mut().unwrap().remove("user");
    assert_eq!(status(&original), ContentStatus::Unknown);
}

#[test]
fn a_trial_edition_cannot_use_an_index_grant_outside_its_uid_trial_boundary() {
    let mut state = fixture();
    state["reader"]["bookInfo"]["payType"] = json!(1 | TRIAL_EDITION);
    state["reader"]["bookInfo"]["maxFreeChapter"] = json!(3);
    edit_chapter(&mut state, |chapter| {
        chapter["chapterIdx"] = json!(2);
        chapter["price"] = json!(0);
    });
    assert_eq!(status(&state), ContentStatus::Preview);
    let mut allowed_trial = state.clone();
    edit_chapter(&mut allowed_trial, |chapter| {
        chapter["chapterUid"] = json!("2");
        chapter["chapterIdx"] = json!(10);
    });
    let allowed_trial = ReaderAccess::from_state(&allowed_trial, BOOK, "2", Some(ACCOUNT), NOW);
    assert_eq!(
        allowed_trial.status_for_text(&complete_text()),
        ContentStatus::Preview
    );
    state["reader"]["bookInfo"]["paid"] = json!(1);
    assert_eq!(status(&state), ContentStatus::Full);
}

#[test]
fn legitimate_short_chapters_and_ellipsis_endings_are_not_previews_by_themselves() {
    let mut state = fixture();
    state["reader"]["bookInfo"]["paid"] = json!(1);
    edit_chapter(&mut state, |chapter| chapter["wordCount"] = json!(10));
    assert_eq!(
        access(&state).status_for_text("A final note."),
        ContentStatus::Full
    );
    edit_chapter(&mut state, |chapter| chapter["wordCount"] = json!(6));
    assert_eq!(
        access(&state).status_for_text("End..."),
        ContentStatus::Full
    );
    assert_eq!(
        access(&state).status_for_text(" \n"),
        ContentStatus::Unknown
    );
    edit_chapter(&mut state, |chapter| chapter["wordCount"] = json!(120));
    for body in ["x".repeat(105), format!("{}...", "x".repeat(102))] {
        assert_eq!(access(&state).status_for_text(&body), ContentStatus::Full);
    }
}

#[test]
fn an_authorized_but_obviously_short_response_is_unknown_instead_of_locked() {
    let mut state = fixture();
    state["reader"]["bookInfo"]["paid"] = json!(1);
    edit_chapter(&mut state, |chapter| chapter["wordCount"] = json!(5000));
    let short = format!("{}...", "x".repeat(100));
    assert_eq!(
        access(&state).status_for_text(&short),
        ContentStatus::Unknown
    );
    state["reader"]["bookInfo"]["paid"] = json!(0);
    assert_eq!(
        access(&state).status_for_text(&short),
        ContentStatus::Preview
    );
    state["reader"]["bookInfo"]
        .as_object_mut()
        .unwrap()
        .remove("paid");
    assert_eq!(
        access(&state).status_for_text(&short),
        ContentStatus::Unknown
    );
}

#[test]
fn reader_access_is_bound_to_the_local_account_and_session_generation() {
    let mut state = fixture();
    state["reader"]["bookInfo"]["paid"] = json!(1);
    let owner = Some("fixture-owner-key".to_owned());
    let mut access = access(&state);
    assert_eq!(
        access.content_status(&complete_text(), &owner, 7),
        ContentStatus::Unknown
    );
    access.bind_session(owner.clone(), 7, true);
    assert!(!access.needs_refresh());
    assert_eq!(
        access.content_status(&complete_text(), &owner, 7),
        ContentStatus::Full
    );
    assert_eq!(
        access.content_status(&complete_text(), &owner, 8),
        ContentStatus::Unknown
    );
    assert_eq!(
        access.content_status(&complete_text(), &Some("other-owner-key".into()), 7),
        ContentStatus::Unknown
    );
    access.bind_session(owner.clone(), 7, false);
    assert!(access.needs_refresh());
    assert_eq!(
        access.content_status(&complete_text(), &owner, 7),
        ContentStatus::Unknown
    );
}

#[test]
fn public_free_grants_do_not_require_fabricating_an_account_identity() {
    let mut state = fixture();
    state["reader"]["bookInfo"]["payType"] = json!(32);
    state["user"]["vid"] = json!(0);
    let access = ReaderAccess::from_state(&state, BOOK, UID, None, NOW);
    assert_eq!(
        access.status_for_text(&complete_text()),
        ContentStatus::Full
    );
    state["user"]["vid"] = json!("other-fixture-account");
    assert_eq!(status(&state), ContentStatus::Unknown);
}

#[test]
fn explicit_member_exceptions_precede_the_per_chapter_membership_exclusion() {
    for exception in ["status", "short_time", "book_list"] {
        let mut state = fixture();
        state["reader"]["bookInfo"]["payType"] = json!(2);
        state["reader"]["bookInfo"]["payingStatus"] = json!(1);
        valid_member(&mut state, false);
        match exception {
            "status" => state["reader"]["bookInfo"]["payingStatus"] = json!(5),
            "short_time" => state["reader"]["bookInfo"]["shortTimeRead"]["active"] = json!(1),
            "book_list" => state["reader"]["memberCardSummary"]["freeBookIds"] = json!([BOOK]),
            _ => unreachable!(),
        }
        assert_eq!(status(&state), ContentStatus::Full);
    }
    let mut excluded = fixture();
    excluded["reader"]["bookInfo"]["payType"] = json!(2);
    excluded["reader"]["bookInfo"]["payingStatus"] = json!(1);
    valid_member(&mut excluded, false);
    assert_eq!(status(&excluded), ContentStatus::Preview);
}

#[test]
fn independent_purchase_books_and_debug_output_keep_their_boundaries() {
    let mut state = fixture();
    state["reader"]["bookInfo"]["payingStatus"] = json!(6);
    valid_member(&mut state, true);
    assert_eq!(status(&state), ContentStatus::Preview);
    state["reader"]["bookInfo"]["paid"] = json!(1);
    let mut access = access(&state);
    access.bind_session(Some("fixture-private-owner-key".into()), 7, true);
    let debug = format!("{access:?}");
    for private in [BOOK, ACCOUNT, "fixture-private-owner-key"] {
        assert!(!debug.contains(private));
    }
}

#[test]
fn unreviewed_membership_statuses_cannot_create_a_membership_grant() {
    for paying_status in [0, 7, 999, -1] {
        let mut state = fixture();
        state["reader"]["bookInfo"]["payingStatus"] = json!(paying_status);
        valid_member(&mut state, true);
        assert_eq!(status(&state), ContentStatus::Unknown);
    }
}

#[test]
fn membership_claims_require_consistent_typed_expiration_even_for_special_status() {
    for paying_status in [1, 3, 5] {
        for (field, value) in [
            ("expired", json!(1)),
            ("expired", json!("0")),
            ("expired", json!(false)),
            ("expiredTime", json!(0)),
            ("expiredTime", json!(NOW - 1)),
            ("expiredTime", json!((NOW + 1000).to_string())),
        ] {
            let mut state = fixture();
            state["reader"]["bookInfo"]["payingStatus"] = json!(paying_status);
            valid_member(&mut state, true);
            state["reader"]["memberCardSummary"][field] = value;
            assert_eq!(status(&state), ContentStatus::Unknown);
        }
        for field in ["expired", "expiredTime"] {
            let mut state = fixture();
            state["reader"]["bookInfo"]["payingStatus"] = json!(paying_status);
            valid_member(&mut state, true);
            state["reader"]["memberCardSummary"]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert_eq!(status(&state), ContentStatus::Unknown);
        }
    }
}

#[test]
fn a_missing_reviewed_billing_mode_is_unknown_instead_of_locked() {
    for pay_type in [json!(0), json!(0x1000), json!("1"), Value::Null] {
        let mut state = fixture();
        state["reader"]["bookInfo"]["payType"] = pay_type;
        assert_eq!(status(&state), ContentStatus::Unknown);
        assert_eq!(access(&state).status_for_text(""), ContentStatus::Unknown);
    }
    let mut missing = fixture();
    missing["reader"]["bookInfo"]
        .as_object_mut()
        .unwrap()
        .remove("payType");
    assert_eq!(status(&missing), ContentStatus::Unknown);
}

#[test]
fn reader_error_and_forbidden_flags_override_purchase_metadata() {
    let mut state = fixture();
    state["reader"]["bookInfo"]["paid"] = json!(1);
    assert_eq!(status(&state), ContentStatus::Full);
    state["reader"]["isBookInfoError"] = json!(false);
    state["reader"]["isBookForbidden"] = json!(false);
    assert_eq!(status(&state), ContentStatus::Full);
    for flag in ["isBookInfoError", "isBookForbidden"] {
        for value in [json!(true), json!("false"), json!(1), Value::Null] {
            let mut invalid = state.clone();
            invalid["reader"][flag] = value;
            assert_eq!(status(&invalid), ContentStatus::Unknown);
        }
    }
}

#[test]
fn ellipsis_only_placeholders_are_not_full_even_without_a_word_count_conflict() {
    for word_count in [None, Some(1), Some(6)] {
        let mut state = fixture();
        state["reader"]["bookInfo"]["paid"] = json!(1);
        edit_chapter(&mut state, |chapter| {
            if let Some(word_count) = word_count {
                chapter["wordCount"] = json!(word_count);
            } else {
                chapter.as_object_mut().unwrap().remove("wordCount");
            }
        });
        for placeholder in ["...", "…", "⋯", " \n...… ⋯.\t"] {
            assert_eq!(
                access(&state).status_for_text(placeholder),
                ContentStatus::Unknown
            );
        }
    }
}

#[test]
fn adjacent_large_numeric_identifiers_are_never_the_same_identity() {
    let first = 9_007_199_254_740_992u64;
    let next = first + 1;

    let mut book = fixture();
    book["reader"]["bookInfo"]["paid"] = json!(1);
    book["reader"]["bookId"] = json!(first);
    book["reader"]["bookInfo"]["bookId"] = json!(next);
    let book_access = ReaderAccess::from_state(&book, &first.to_string(), UID, Some(ACCOUNT), NOW);
    assert_eq!(
        book_access.status_for_text(&complete_text()),
        ContentStatus::Unknown
    );

    let mut chapter = fixture();
    chapter["reader"]["bookInfo"]["paid"] = json!(1);
    edit_chapter(&mut chapter, |chapter| chapter["chapterUid"] = json!(first));
    let chapter_access =
        ReaderAccess::from_state(&chapter, BOOK, &next.to_string(), Some(ACCOUNT), NOW);
    assert_eq!(
        chapter_access.status_for_text(&complete_text()),
        ContentStatus::Unknown
    );

    let mut account = fixture();
    account["reader"]["bookInfo"]["paid"] = json!(1);
    account["user"]["vid"] = json!(first);
    let expected = next.to_string();
    let account_access = ReaderAccess::from_state(&account, BOOK, UID, Some(&expected), NOW);
    assert_eq!(
        account_access.status_for_text(&complete_text()),
        ContentStatus::Unknown
    );
}
