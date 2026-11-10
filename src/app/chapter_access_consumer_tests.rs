//! Selected chapter evidence preserves permissions without copying resident SQL bodies.

use super::underline_projection_tests::{BOOK, CHAPTER, Fixture, isolated};
use super::*;
use crate::store::{CachedResourceLimit, ChapterAccessLimits, with_chapter_access_test_limits};
use rusqlite::{params, types::Value};

fn sql(state: &App) -> Result<rusqlite::Connection> {
    Ok(rusqlite::Connection::open(
        state.data_dir.join("library.sqlite"),
    )?)
}

fn account() -> source::AuthSession {
    source::AuthSession {
        api_key: "synthetic-access-key".into(),
        user_vid: "synthetic-access-account".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "synthetic-access-account".into()),
            ("wr_skey".into(), "synthetic-access-session".into()),
        ]),
        ..Default::default()
    }
}

fn set_intro(state: &App) -> Result<String> {
    let intro = "Retained selected metadata introduction. ".repeat(2048);
    sql(state)?.execute(
        "UPDATE books SET intro=?2 WHERE id=?1",
        params![BOOK, intro],
    )?;
    Ok(intro)
}

fn assert_intro(state: &App, intro: &str) -> Result<()> {
    assert_eq!(
        sql(state)?.query_row("SELECT intro FROM books WHERE id=?1", [BOOK], |row| row
            .get::<_, String>(
            0
        ))?,
        intro
    );
    Ok(())
}

fn remote_catalog(state: &App) -> Result<()> {
    state.store.set_setting(
        &format!("catalog.remote:{BOOK}"),
        &vec![
            source::RemoteChapter {
                uid: CHAPTER.into(),
                index: 0,
                word_count: 100,
                ..Default::default()
            },
            source::RemoteChapter {
                uid: "underline-fixture-chapter-1".into(),
                index: 1,
                word_count: 100,
                ..Default::default()
            },
        ],
    )
}

fn current_only() -> Result<()> {
    let fixture = Fixture::new("Selected current body.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let intro = set_intro(&state)?;
        remote_catalog(&state)?;
        let database = sql(&state)?;
        let old_other: Option<String> = database.query_row(
            "SELECT body FROM chapters WHERE book_id=?1 AND chapter_index=1",
            [BOOK],
            |row| row.get(0),
        )?;
        database.execute(
            "UPDATE chapters SET body=x'ff' WHERE book_id=?1 AND chapter_index=1",
            [BOOK],
        )?;
        let owned = state.store.chapter_body_read_count();
        let corpus = state.store.materialized_chapter_bytes();
        state.store.reset_book_read_stats();
        state.store.reset_selected_metadata_read_stats();
        for _ in 0..3 {
            state.reflow_reader()?;
            state.render_reader()?;
            let position = state.reader_upload_position()?.unwrap();
            assert_eq!(position.chapter_uid, CHAPTER);
        }
        assert_eq!(state.store.chapter_body_read_count(), owned);
        assert_eq!(state.store.materialized_chapter_bytes(), corpus);
        assert_eq!(state.store.book_read_stats().intro_copies, 0);
        let metadata = state.store.selected_metadata_read_stats();
        assert_eq!(metadata.body_borrows, 9);
        assert_eq!(metadata.body_scan_bytes, 9 * "Selected current body.".len());
        assert_eq!(state.reader.body, "Selected current body.");
        assert!(
            state
                .reader
                .paginator
                .as_ref()
                .is_some_and(|pages| !pages.current_page().lines.is_empty())
        );
        assert_intro(&state, &intro)?;
        assert_eq!(
            database.query_row(
                "SELECT body FROM chapters WHERE book_id=?1 AND chapter_index=1",
                [BOOK],
                |row| row.get::<_, Value>(0)
            )?,
            Value::Blob(vec![0xff])
        );
        state.store.reset_selected_metadata_read_stats();
        assert_eq!(
            state.store.chapter_title(BOOK, 0)?.as_deref(),
            Some("Underline fixture chapter")
        );
        assert_eq!(state.store.selected_metadata_read_stats().body_borrows, 0);
        database.execute(
            "UPDATE chapters SET body=?2 WHERE book_id=?1 AND chapter_index=1",
            params![BOOK, old_other],
        )?;
    }
    fixture.finish()
}

#[test]
fn current_render_reflow_and_position_use_borrowed_access_and_never_copy_other_blob() -> Result<()>
{
    isolated(
        "app::chapter_access_consumer_tests::current_render_reflow_and_position_use_borrowed_access_and_never_copy_other_blob",
        current_only,
    )
}

fn quota_before_mutation() -> Result<()> {
    let fixture = Fixture::new("Keep visible paginator.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        state.reader.selection = Some((0, 4));
        assert!(state.reader.draft.set_query("Keep access draft"));
        let body = state.reader.body.clone();
        let account = state.jobs.session();
        let pages = state.reader.paginator.as_ref().unwrap().pages.as_ptr();
        let owned = state.store.chapter_body_read_count();
        for limits in [
            ChapterAccessLimits {
                metadata_bytes: 1,
                ..Default::default()
            },
            ChapterAccessLimits {
                work_bytes: 1,
                ..Default::default()
            },
            ChapterAccessLimits {
                temporary_bytes: 1,
                ..Default::default()
            },
        ] {
            state.store.reset_selected_metadata_read_stats();
            let error =
                with_chapter_access_test_limits(limits, || state.reflow_reader()).unwrap_err();
            assert!(error.downcast_ref::<CachedResourceLimit>().is_some());
            assert_eq!(
                state.reader.paginator.as_ref().unwrap().pages.as_ptr(),
                pages
            );
            assert_eq!(state.reader.body, body);
            assert_eq!(state.reader.selection, Some((0, 4)));
            assert_eq!(state.reader.draft.query, "Keep access draft");
            assert_eq!(state.jobs.session(), account);
            assert_eq!(state.store.chapter_body_read_count(), owned);
            assert_eq!(state.store.selected_metadata_read_stats().body_borrows, 0);
            let error =
                with_chapter_access_test_limits(limits, || state.render_reader()).unwrap_err();
            assert!(error.downcast_ref::<CachedResourceLimit>().is_some());
            let error = with_chapter_access_test_limits(limits, || state.reader_upload_position())
                .unwrap_err();
            assert!(error.downcast_ref::<CachedResourceLimit>().is_some());
            assert_eq!(
                state.reader.paginator.as_ref().unwrap().pages.as_ptr(),
                pages
            );
            assert_eq!(state.reader.body, body);
            assert_eq!(state.reader.draft.query, "Keep access draft");
        }
        assert_eq!(
            state.store.chapter_body(BOOK, 0)?.as_deref(),
            Some(body.as_str())
        );
    }
    fixture.finish()
}

#[test]
fn selected_access_quota_rejects_before_body_borrow_and_keeps_visible_state() -> Result<()> {
    isolated(
        "app::chapter_access_consumer_tests::selected_access_quota_rejects_before_body_borrow_and_keeps_visible_state",
        quota_before_mutation,
    )
}

fn presence_and_local_rules() -> Result<()> {
    let fixture = Fixture::new("Retained resident text.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        state.jobs.replace_session(account())?;
        let owner = state.chapter_content_owner().unwrap();
        let database = sql(&state)?;
        let owned = state.store.chapter_body_read_count();
        for (value, present, readable) in [
            (Value::Text(String::new()), true, false),
            (Value::Null, false, false),
            (
                Value::Text("\t\n\u{85}\u{a0}\u{2000}\u{3000}".into()),
                true,
                false,
            ),
            (Value::Text("\0".into()), true, true),
            (Value::Text(" \u{4e2d} ".into()), true, true),
        ] {
            database.execute(
                "UPDATE chapters SET body=?2,content_status='preview',content_account_key=?3
                 WHERE book_id=?1 AND chapter_index=0",
                params![BOOK, value, owner],
            )?;
            let access = state.store.chapter_access_metadata(BOOK, 0)?.unwrap();
            assert!(access.chapter.body.is_none());
            assert_eq!(access.body_present, present);
            assert_eq!(access.locally_readable, readable);
            assert!(!access.chapter.cached);
            assert_eq!(
                state.chapter_access_content_status("local", &access),
                if readable {
                    ContentStatus::Full
                } else {
                    ContentStatus::Unknown
                }
            );
            assert_eq!(
                state.chapter_access_content_status("weread", &access),
                ContentStatus::Preview
            );
            assert_eq!(
                super::library::chapter_content_status_for_access_owner(
                    "weread",
                    &access,
                    Some("different-owner")
                ),
                ContentStatus::Unknown
            );
        }
        assert_eq!(state.store.chapter_body_read_count(), owned);
        assert_eq!(state.reader.body, "Retained resident text.");
        database.execute("UPDATE chapters SET body='',content_status='unknown' WHERE book_id=?1 AND chapter_index=0",
            [BOOK])?;
        database.execute("UPDATE books SET source='weread' WHERE id=?1", [BOOK])?;
        state.render_reader()?;
        assert_eq!(
            fixture.ui().global::<ReaderModel>().get_access_label(),
            tr("content_unverified")
        );
        assert_eq!(state.reader.body, "Retained resident text.");
        database.execute("UPDATE chapters SET body='Retained resident text.',content_status='full' WHERE book_id=?1 AND chapter_index=0",
            [BOOK])?;
        database.execute("UPDATE books SET source='local' WHERE id=?1", [BOOK])?;
    }
    fixture.finish()
}

#[test]
fn text_empty_null_unicode_whitespace_and_nul_keep_original_presence_and_permissions() -> Result<()>
{
    isolated(
        "app::chapter_access_consumer_tests::text_empty_null_unicode_whitespace_and_nul_keep_original_presence_and_permissions",
        presence_and_local_rules,
    )
}

fn session_id_and_digest() -> Result<()> {
    let fixture = Fixture::new("Resume \u{1f600} current body.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let intro = set_intro(&state)?;
        let database = sql(&state)?;
        let old_other: Option<String> = database.query_row(
            "SELECT body FROM chapters WHERE book_id=?1 AND chapter_index=1",
            [BOOK],
            |row| row.get(0),
        )?;
        database.execute(
            "UPDATE chapters SET body=x'ff' WHERE book_id=?1 AND chapter_index=1",
            [BOOK],
        )?;
        let body = state.reader.body.clone();
        let owned = state.store.chapter_body_read_count();
        state.store.reset_book_read_stats();
        state.store.reset_selected_metadata_read_stats();
        state.save_reader_session()?;
        assert_eq!(state.store.chapter_body_read_count(), owned);
        assert_eq!(state.store.selected_metadata_read_stats().body_borrows, 0);
        assert_eq!(state.store.book_read_stats().intro_copies, 0);
        let saved = state
            .store
            .setting::<serde_json::Value>("reader.resume.v1")?
            .unwrap();
        assert_eq!(saved["chapter_id"], CHAPTER);
        state.route = Route::Library;
        state.restore_reader_session()?;
        assert_eq!(state.store.chapter_body_read_count() - owned, 2);
        assert_eq!(state.reader.body, body);
        assert_eq!(state.reader.book_id, BOOK);
        assert_eq!(state.reader.chapter_index, 0);
        assert_intro(&state, &intro)?;
        let before_mismatch = state.store.chapter_body_read_count();
        state.route = Route::Library;
        database.execute(
            "UPDATE chapters SET body='Changed SQL body' WHERE book_id=?1 AND chapter_index=0",
            [BOOK],
        )?;
        state.restore_reader_session()?;
        assert_eq!(state.store.chapter_body_read_count() - before_mismatch, 1);
        assert_eq!(state.route, Route::Library);
        assert_eq!(state.reader.body, body);
        assert_eq!(
            fixture.ui().global::<ShellModel>().get_toast(),
            tr("resume_session_unavailable")
        );
        database.execute(
            "UPDATE chapters SET body=?2 WHERE book_id=?1 AND chapter_index=0",
            params![BOOK, body],
        )?;
        database.execute(
            "UPDATE chapters SET body=?2 WHERE book_id=?1 AND chapter_index=1",
            params![BOOK, old_other],
        )?;
    }
    fixture.finish()
}

#[test]
fn session_save_reads_only_raw_uid_while_restore_keeps_real_sql_digest_proof() -> Result<()> {
    isolated(
        "app::chapter_access_consumer_tests::session_save_reads_only_raw_uid_while_restore_keeps_real_sql_digest_proof",
        session_id_and_digest,
    )
}

fn report_source_flags() -> Result<()> {
    let fixture = Fixture::new("Report current body.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        state.jobs.replace_session(account())?;
        let owner = state.chapter_content_owner().unwrap();
        let intro = set_intro(&state)?;
        remote_catalog(&state)?;
        let database = sql(&state)?;
        database.execute("UPDATE books SET source='weread' WHERE id=?1", [BOOK])?;
        database.execute(
            "UPDATE chapters SET content_status='full',content_account_key=?2
            WHERE book_id=?1 AND chapter_index=0",
            params![BOOK, owner],
        )?;
        state.demo = false;
        state.reader.session_started = Some(Utc::now().timestamp() - 60);
        state.reader.session_seconds = 60;
        state.reader.position_confirmed = false;
        state.prefs.report_time = true;
        state.prefs.push_on_close = true;
        state.prefs.report_reader_only = false;
        let owned = state.store.chapter_body_read_count();
        state.store.reset_book_read_stats();
        state.start_read_report_session()?;
        state.checkpoint_read_reports(false)?;
        assert_eq!(state.store.chapter_body_read_count(), owned);
        assert_eq!(state.store.book_read_stats().intro_copies, 0);
        let old = state
            .store
            .setting::<serde_json::Value>("report.outbox.v1")?
            .unwrap();
        assert_eq!(old["batches"].as_array().unwrap().len(), 1);
        database.execute("UPDATE books SET source='local' WHERE id=?1", [BOOK])?;
        state.route = Route::Library;
        state.force_offline = false;
        state.online = true;
        state.store.set_setting("report.next_check.v1", &0_i64)?;
        state.store.reset_book_read_stats();
        let capacity = state.jobs.metadata_capacity();
        state.tick_read_reports()?;
        let stopped = state
            .store
            .setting::<serde_json::Value>("report.outbox.v1")?
            .unwrap();
        assert_eq!(stopped["batches"][0]["state"], "blocked");
        assert_eq!(state.jobs.metadata_capacity(), capacity);
        assert_eq!(state.store.book_read_stats().intro_copies, 0);
        assert_eq!(state.store.chapter_body_read_count(), owned);
        assert_intro(&state, &intro)?;
        assert_eq!(state.reader.body, "Report current body.");
        state.force_offline = true;
    }
    fixture.finish()
}

#[test]
fn report_start_and_queued_source_checks_use_narrow_fields_without_post_or_intro_copy() -> Result<()>
{
    isolated(
        "app::chapter_access_consumer_tests::report_start_and_queued_source_checks_use_narrow_fields_without_post_or_intro_copy",
        report_source_flags,
    )
}
