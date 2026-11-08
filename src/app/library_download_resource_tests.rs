//! Genuine metadata-only Detail, dialog, and serial-plan operations on cached SQL text.

use super::super::underline_projection_tests::{Fixture, isolated};
use super::*;
use slint::Model;

const BOOK: &str = "bodyless-corpus-book";
const UNFETCHED: u32 = 16;

fn configure(state: &mut App) -> Result<String> {
    state.demo = false;
    state.online = true;
    state.force_offline = true;
    state.reader.session_started = None;
    state.prefs.report_time = false;
    state.prefs.push_on_close = false;
    state.prefs.keep_awake = false;
    state.jobs.replace_session(source::AuthSession {
        user_vid: "synthetic-corpus-reader".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "synthetic-corpus-reader".into()),
            ("wr_skey".into(), "synthetic-corpus-session".into()),
        ]),
        ..Default::default()
    })?;
    Ok(state
        .jobs
        .session()
        .as_ref()
        .and_then(download_account_key)
        .unwrap())
}

fn seed_corpus(state: &mut App, owner: &str) -> Result<()> {
    let mut book = Book::new(BOOK, "Cached corpus fixture");
    book.source = "weread".into();
    book.total_chapters = UNFETCHED + 1;
    state.store.upsert_book(&book)?;
    let body = "Unrelated cached text. ".repeat(8192);
    for index in 0..UNFETCHED {
        state.store.upsert_chapter(&Chapter {
            id: format!("corpus-{index}"),
            book_id: BOOK.into(),
            index,
            title: format!("Corpus title {index}"),
            cached: true,
            body: Some(body.clone()),
            content_status: ContentStatus::Full,
            content_account_key: owner.into(),
        })?;
    }
    state.store.upsert_chapter(&Chapter {
        id: format!("corpus-{UNFETCHED}"),
        book_id: BOOK.into(),
        index: UNFETCHED,
        title: "Unfetched final chapter".into(),
        ..Default::default()
    })?;
    state.route = Route::Detail(BOOK.into());
    state.detail_id = BOOK.into();
    Ok(())
}

fn detail_and_dialog_case() -> Result<()> {
    let fixture = Fixture::new("Reader fixture", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let owner = configure(&mut state)?;
        seed_corpus(&mut state, &owner)?;
        let reads = state.store.materialized_chapter_bytes();
        for tab in ["intro", "toc", "notes"] {
            state.detail_tab = tab.into();
            state.render_detail()?;
        }
        assert!(fixture.ui().global::<DetailModel>().get_can_read());
        assert!(
            fixture
                .ui()
                .global::<DetailModel>()
                .get_chapters()
                .row_count()
                > 0
        );
        state.detail_tab = "toc".into();
        state.selecting_chapters = true;
        state.detail_page = UNFETCHED as usize / state.profile.detail_chapter_page_size();
        state.detail_action("select-page", "")?;
        assert_eq!(state.selected_chapters, BTreeSet::from([UNFETCHED]));
        state.detail_action("chapter", &format!("corpus-{UNFETCHED}"))?;
        assert!(state.selected_chapters.is_empty());
        state.detail_action("download", "")?;
        assert_eq!(state.pending_downloads[0].1.len(), (UNFETCHED + 1) as usize);
        state.detail_action("refresh-access", "")?;
        state.offer_download(vec![(BOOK.into(), vec![UNFETCHED])])?;
        assert_eq!(state.pending_downloads, [(BOOK.into(), vec![UNFETCHED])]);
        assert_eq!(state.store.materialized_chapter_bytes(), reads);
        assert_eq!(state.store.book(BOOK)?.unwrap().cached_chapters, UNFETCHED);
    }
    fixture.finish()
}

fn single_chapter_case() -> Result<()> {
    let fixture = Fixture::new("Reader fixture", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let owner = configure(&mut state)?;
        seed_corpus(&mut state, &owner)?;
        state.force_offline = false;
        let reads = state.store.materialized_chapter_bytes();
        // The current-thread runtime is not driven. This runs the actual App
        // planner and admits a real Jobs task without starting its transport.
        state.enqueue_download_options(BOOK, vec![UNFETCHED], false)?;
        let task = state.current_download.as_ref().unwrap().clone();
        assert_eq!(task.chapters, [UNFETCHED]);
        assert_eq!(
            state.jobs.active_download().as_deref(),
            Some(task.id.as_str())
        );
        assert_eq!(state.store.materialized_chapter_bytes(), reads);
        assert_eq!(state.store.book(BOOK)?.unwrap().cached_chapters, UNFETCHED);
        assert!(state.events.try_recv().is_err());
        let mut cancelled = state.current_download.take().unwrap();
        let cancellation = state.jobs.cancel_download(&cancelled.id);
        state.settle_job_submission(cancellation);
        cancelled.status = DownloadStatus::Cancelled;
        state.store.save_task(&cancelled)?;
        state.force_offline = true;
    }
    fixture.finish()
}

fn invalid_plan_case() -> Result<()> {
    let fixture = Fixture::new("Reader fixture", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let owner = configure(&mut state)?;
        seed_corpus(&mut state, &owner)?;
        for index in 0..2 {
            state.store.upsert_chapter(&Chapter {
                id: "duplicate-corpus-uid".into(),
                book_id: BOOK.into(),
                index,
                title: "Duplicate fixture".into(),
                cached: true,
                body: Some("Keep cached text".into()),
                content_status: ContentStatus::Full,
                content_account_key: owner.clone(),
            })?;
        }
        let task = DownloadTask {
            id: "invalid-plan-task".into(),
            book_id: BOOK.into(),
            account_key: owner,
            chapters: vec![0, 1],
            status: DownloadStatus::Pending,
            include_images: false,
            ..Default::default()
        };
        state.store.save_task(&task)?;
        state.downloads.push_back(task.clone());
        state.force_offline = false;
        let reads = state.store.materialized_chapter_bytes();
        state.start_next_download()?;
        assert!(state.current_download.is_none() && state.jobs.active_download().is_none());
        let saved = state.store.task(&task.id)?.unwrap();
        assert_eq!(saved.status, DownloadStatus::Failed);
        assert_eq!(saved.next_index, 0);
        assert_eq!(
            saved.error.as_deref(),
            Some("source.invalid_response.download_header")
        );
        assert_eq!(state.store.materialized_chapter_bytes(), reads);
        state.force_offline = true;
    }
    fixture.finish()
}

fn detail_presence_case() -> Result<()> {
    let fixture = Fixture::new("Reader fixture", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let owner = configure(&mut state)?;
        let mut book = Book::new(BOOK, "Presence fixture");
        book.source = "weread".into();
        state.store.upsert_book(&book)?;
        state.store.upsert_chapter(&Chapter {
            id: "presence-chapter".into(),
            book_id: BOOK.into(),
            title: "Presence title".into(),
            ..Default::default()
        })?;
        state.route = Route::Detail(BOOK.into());
        state.detail_id = BOOK.into();
        state.detail_tab = "toc".into();
        state.online = false;
        let reads = state.store.materialized_chapter_bytes();
        state.render_detail()?;
        assert!(!fixture.ui().global::<DetailModel>().get_can_read());
        state.store.upsert_chapter(&Chapter {
            id: "presence-chapter".into(),
            book_id: BOOK.into(),
            title: "Presence title".into(),
            body: Some("Retained trial text".into()),
            content_status: ContentStatus::Preview,
            content_account_key: owner,
            ..Default::default()
        })?;
        state.render_detail()?;
        let model = fixture.ui().global::<DetailModel>();
        assert!(model.get_can_read() && model.get_access_needs_refresh());
        assert!(!model.get_chapters().row_data(0).unwrap().cached);
        assert!(
            model
                .get_chapters()
                .row_data(0)
                .unwrap()
                .state
                .contains(tr("content_preview").as_str())
        );
        assert_eq!(state.store.materialized_chapter_bytes(), reads);
    }
    fixture.finish()
}

#[test]
fn detail_and_download_dialog_do_not_materialize_cached_corpus_bodies() -> Result<()> {
    isolated(
        "app::library::download_resource_tests::detail_and_download_dialog_do_not_materialize_cached_corpus_bodies",
        detail_and_dialog_case,
    )
}

#[test]
fn single_chapter_download_planning_leaves_unrelated_corpus_in_sql() -> Result<()> {
    isolated(
        "app::library::download_resource_tests::single_chapter_download_planning_leaves_unrelated_corpus_in_sql",
        single_chapter_case,
    )
}

#[test]
fn invalid_metadata_plans_fail_before_running_without_reading_bodies() -> Result<()> {
    isolated(
        "app::library::download_resource_tests::invalid_metadata_plans_fail_before_running_without_reading_bodies",
        invalid_plan_case,
    )
}

#[test]
fn metadata_only_details_keep_trial_readability_and_access_flags() -> Result<()> {
    isolated(
        "app::library::download_resource_tests::metadata_only_details_keep_trial_readability_and_access_flags",
        detail_presence_case,
    )
}
