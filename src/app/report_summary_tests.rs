//! Actual offline report allocation with bounded new positions and untouched old proof.

use super::underline_projection_tests::{BOOK, CHAPTER, Fixture, isolated};
use super::*;
use serde_json::{Value, json};
use source::report::ReadReportRequest;

const OUTBOX: &str = "report.outbox.v1";
const ACTIVE: &str = "report.active.v1";

fn long_title() -> String {
    format!("{}{}", "\u{1f600}".repeat(20), "t".repeat(64 * 1024 - 80))
}

fn seed(state: &mut App, title: &str) -> Result<i64> {
    state.jobs.replace_session(source::AuthSession {
        api_key: "synthetic-report-key".into(),
        user_vid: "synthetic-summary-account".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "synthetic-summary-account".into()),
            ("wr_skey".into(), "synthetic-summary-session".into()),
        ]),
        ..Default::default()
    })?;
    let owner = state.chapter_content_owner().unwrap();
    let mut book = state.store.book(BOOK)?.unwrap();
    book.source = "weread".into();
    state.store.upsert_book(&book)?;
    state.store.upsert_chapter(&Chapter {
        id: CHAPTER.into(),
        book_id: BOOK.into(),
        index: 0,
        title: title.into(),
        body: Some("Reader summary fixture.".into()),
        cached: true,
        content_status: ContentStatus::Full,
        content_account_key: owner,
        ..Default::default()
    })?;
    state.store.set_setting(
        &format!("catalog.remote:{BOOK}"),
        &vec![source::RemoteChapter {
            uid: CHAPTER.into(),
            title: title.into(),
            index: 0,
            word_count: 100,
            ..Default::default()
        }],
    )?;
    state.demo = false;
    state.force_offline = true;
    state.prefs.report_time = true;
    state.prefs.push_on_close = true;
    state.reader.position_confirmed = true;
    let started = Utc::now().timestamp() - 240;
    state.reader.session_started = Some(started);
    state.reader.session_seconds = 240;
    Ok(started)
}

fn assert_wire_equal(position: &source::ReadingPosition, title: &str) {
    assert_eq!(
        position.summary.chars().count(),
        super::reporting::REPORT_SUMMARY_SCALARS
    );
    assert!(position.summary.len() <= 80);
    let mut original = position.clone();
    original.summary = title.into();
    let clock = source::content::ReaderSession::default();
    assert_eq!(
        source::protocol::read_payload_with_clock(
            BOOK,
            position,
            &clock,
            Some(60),
            1_788_000_000,
            1_788_000_000_123,
            7
        ),
        source::protocol::read_payload_with_clock(
            BOOK,
            &original,
            &clock,
            Some(60),
            1_788_000_000,
            1_788_000_000_123,
            7
        )
    );
}

fn new_four_batches() -> Result<()> {
    let fixture = Fixture::new("Reader summary fixture.", Vec::new(), false)?;
    let title = long_title();
    {
        let mut state = fixture.app();
        seed(&mut state, &title)?;
        let position = state.reader_upload_position()?.unwrap();
        assert_wire_equal(&position, &title);
        state.start_read_report_session()?;
        state.checkpoint_read_reports(false)?;
        let queue = state.store.setting::<Value>(OUTBOX)?.unwrap();
        let batches = queue["batches_v2"].as_array().unwrap();
        assert_eq!(batches.len(), 4);
        for (index, batch) in batches.iter().enumerate() {
            let request: ReadReportRequest = serde_json::from_value(batch["request"].clone())?;
            assert_eq!(request.seconds, 60);
            assert_eq!(batch["from"].as_u64(), Some(index as u64 * 60));
            assert_eq!(batch["to"].as_u64(), Some((index as u64 + 1) * 60));
            assert_eq!(batch["state"], "pending");
            assert_eq!(batch["attempts"], 0);
            assert_wire_equal(request.position.as_ref().unwrap(), &title);
        }
        let active = state.store.setting::<Value>(ACTIVE)?.unwrap();
        assert_eq!(active["position"]["summary"].as_str().unwrap().len(), 80);
        assert_eq!(active["allocated_seconds"], 240);
        assert_eq!(state.store.chapter(BOOK, 0)?.unwrap().title, title);
        assert!(state.events.try_recv().is_err());
    }
    fixture.finish()
}

#[test]
fn long_sql_title_forms_four_wire_equal_small_pending_summaries_for_240_seconds() -> Result<()> {
    isolated(
        "app::report_summary_tests::long_sql_title_forms_four_wire_equal_small_pending_summaries_for_240_seconds",
        new_four_batches,
    )
}

fn old_proof_and_new_projection() -> Result<()> {
    let fixture = Fixture::new("Reader summary fixture.", Vec::new(), false)?;
    let title = long_title();
    {
        let mut state = fixture.app();
        let started = seed(&mut state, &title)?;
        let now = Utc::now().timestamp();
        let account = source::report::account_key(&state.jobs.session().unwrap()).unwrap();
        let mut position = state.reader_upload_position()?.unwrap();
        position.summary = title.clone();
        let old_request = ReadReportRequest {
            session_id: "legacy-unresolved-summary-request".into(),
            account_key: account.clone(),
            book_id: BOOK.into(),
            started_at: started - 60,
            seconds: 60,
            progress_write_allowed: true,
            active_reader_intent: false,
            position: Some(position.clone()),
        };
        let old_batch = json!({
            "request": old_request, "run_id": "legacy-uncertain-run", "from": 0, "to": 60,
            "state": "uncertain", "attempted_at": started, "retry_after": 0, "attempts": 1,
            "error": "source.delivery_uncertain"
        });
        let proof = json!({
            "account_key": account, "book_id": BOOK, "started_at": started - 120,
            "confirmed_until": 60, "confirmed_ranges": []
        });
        state.store.set_setting(
            OUTBOX,
            &json!({
                "batches_v2": [old_batch],
                "reported_cursors": { "legacy-uncertain-run": 0, "retained-proof-run": 60 },
                "compacted_runs": { "retained-proof-run": proof }
            }),
        )?;
        state.store.set_setting(
            ACTIVE,
            &json!({
                "id": "legacy-active-summary-run", "account_key": account, "book_id": BOOK,
                "started_at": started, "observed_seconds": 240, "eligible_seconds": 240,
                "allocated_seconds": 0, "last_checkpoint_at": now, "local_chapter": 0,
                "local_anchor": 0, "position": position
            }),
        )?;
        let capacity = state.jobs.metadata_capacity();
        state.checkpoint_read_reports(true)?;
        state.tick_read_reports()?;
        let queue = state.store.setting::<Value>(OUTBOX)?.unwrap();
        let batches = queue["batches_v2"].as_array().unwrap();
        assert_eq!(batches.len(), 5);
        assert_eq!(batches[0], old_batch);
        assert_eq!(queue["compacted_runs"]["retained-proof-run"], proof);
        assert_eq!(queue["reported_cursors"]["retained-proof-run"], 60);
        assert_eq!(queue["reported_cursors"]["legacy-uncertain-run"], 0);
        for batch in &batches[1..] {
            let request: ReadReportRequest = serde_json::from_value(batch["request"].clone())?;
            assert_eq!(request.seconds, 60);
            assert_eq!(batch["state"], "pending");
            assert_eq!(batch["attempts"], 0);
            assert_wire_equal(request.position.as_ref().unwrap(), &title);
        }
        let active = state.store.setting::<Value>(ACTIVE)?.unwrap();
        assert_eq!(active["position"]["summary"], title);
        assert_eq!(state.store.chapter(BOOK, 0)?.unwrap().title, title);
        assert_eq!(state.jobs.metadata_capacity(), capacity);
        assert!(state.events.try_recv().is_err());
    }
    fixture.finish()
}

#[test]
fn new_batches_project_legacy_active_metadata_without_rewriting_uncertain_request_or_proof()
-> Result<()> {
    isolated(
        "app::report_summary_tests::new_batches_project_legacy_active_metadata_without_rewriting_uncertain_request_or_proof",
        old_proof_and_new_projection,
    )
}
