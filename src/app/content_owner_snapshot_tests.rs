//! Small real-session and App operations prove owner work is per operation.

use super::super::underline_projection_tests::{Fixture, isolated};
use super::*;
use slint::Model;

const BOOK: &str = "owner-snapshot-book";
const BODY: &str = "Preserved owner snapshot text.";

fn work() -> ContentOwnerWork {
    CONTENT_OWNER_WORK.with(std::cell::Cell::get)
}

fn reset_work() {
    CONTENT_OWNER_WORK.with(|slot| slot.set(ContentOwnerWork::default()));
}

fn account(vid: Option<&str>, cookie: &str) -> source::AuthSession {
    let mut cookies = BTreeMap::from([
        ("wr_skey".into(), format!("synthetic-session-{cookie}")),
        ("wr_payload".into(), "x".repeat(2048)),
    ]);
    if let Some(vid) = vid {
        cookies.insert("wr_vid".into(), vid.into());
    }
    source::AuthSession {
        api_key: "synthetic-owner-snapshot-api-key".into(),
        account_name: "Owner snapshot fixture".into(),
        user_vid: vid.unwrap_or_default().into(),
        login_time: 100,
        cookies,
        ..Default::default()
    }
}

fn configure(state: &mut App, session: &source::AuthSession) -> Result<String> {
    state.demo = false;
    state.online = false;
    state.force_offline = true;
    state.reader.session_started = None;
    state.prefs.report_time = false;
    state.prefs.push_on_close = false;
    state.prefs.keep_awake = false;
    state.session_file.save(session)?;
    state.jobs.replace_session(session.clone())?;
    Ok(state.chapter_content_owner().unwrap())
}

fn seed_catalog(
    state: &mut App,
    count: usize,
    owner: &str,
    status: ContentStatus,
    body_present: bool,
) -> Result<()> {
    let mut book = Book::new(BOOK, "Owner snapshot catalog");
    book.source = "weread".into();
    book.total_chapters = count as u32;
    state.store.upsert_book(&book)?;
    for index in 0..count {
        state.store.upsert_chapter(&Chapter {
            id: format!("owner-snapshot-{index}"),
            book_id: BOOK.into(),
            index: index as u32,
            title: format!("Snapshot chapter {index}"),
            cached: status == ContentStatus::Full && body_present,
            body: body_present.then(|| BODY.into()),
            content_status: status,
            content_account_key: owner.into(),
        })?;
    }
    state.route = Route::Detail(BOOK.into());
    state.detail_id = BOOK.into();
    state.detail_tab = "toc".into();
    state.detail_page = 0;
    Ok(())
}

fn assert_one_snapshot(session: &source::AuthSession, serialized_cookies: bool) {
    let observed = work();
    assert_eq!(observed.snapshots, 1);
    assert_eq!(observed.owners, 1);
    assert_eq!(
        observed.snapshot_string_bytes,
        session_string_bytes(session)
    );
    assert!(observed.snapshot_string_bytes >= 2048);
    assert!(observed.identity_bytes > 0);
    assert_eq!(
        observed.cookie_serializations,
        usize::from(serialized_cookies)
    );
}

#[test]
fn remote_status_requires_the_current_nonempty_owner_without_presence_authority() {
    let book = Book {
        source: "weread".into(),
        ..Book::new(BOOK, "Remote permission fixture")
    };
    for status in [
        ContentStatus::Full,
        ContentStatus::Preview,
        ContentStatus::Locked,
        ContentStatus::Unknown,
    ] {
        let mut chapter = Chapter {
            cached: true,
            content_status: status,
            content_account_key: "current-owner".into(),
            ..Default::default()
        };
        assert_eq!(
            chapter_content_status_for_owner(&book, &chapter, Some("current-owner")),
            status
        );
        for owner in [None, Some("wrong-owner"), Some("")] {
            assert_eq!(
                chapter_content_status_for_owner(&book, &chapter, owner),
                ContentStatus::Unknown
            );
        }
        chapter.content_account_key.clear();
        assert_eq!(
            chapter_content_status_for_owner(&book, &chapter, Some("current-owner")),
            ContentStatus::Unknown
        );
        assert_eq!(
            chapter_content_status_for_owner(&book, &chapter, Some("")),
            ContentStatus::Unknown
        );
    }
    assert_eq!(
        chapter_content_label_for_status(ContentStatus::Unknown, true, false),
        tr("content_unverified")
    );
    assert_eq!(
        chapter_content_label_for_status(ContentStatus::Unknown, false, false),
        tr("not_downloaded")
    );
    for (status, label) in [
        (ContentStatus::Full, "downloaded"),
        (ContentStatus::Preview, "content_preview"),
        (ContentStatus::Locked, "content_locked"),
    ] {
        assert_eq!(
            chapter_content_label_for_status(status, false, false),
            tr(label)
        );
        assert_eq!(
            chapter_content_label_for_status(status, false, true),
            format!("{} · {}", tr("current_chapter"), tr(label))
        );
    }
}

#[test]
fn local_status_retains_cached_and_unicode_nonblank_body_semantics() {
    let book = Book {
        source: "local".into(),
        ..Book::new(BOOK, "Local permission fixture")
    };
    for (cached, body, expected) in [
        (false, None, ContentStatus::Unknown),
        (true, None, ContentStatus::Full),
        (false, Some("\u{2003}\n"), ContentStatus::Unknown),
        (false, Some("\u{4e2d}\u{1f600}"), ContentStatus::Full),
    ] {
        let chapter = Chapter {
            cached,
            body: body.map(str::to_owned),
            content_status: ContentStatus::Locked,
            content_account_key: "another-owner".into(),
            ..Default::default()
        };
        reset_work();
        assert_eq!(
            chapter_content_status_for_owner(&book, &chapter, None),
            expected
        );
        assert_eq!(
            chapter_content_status_for_owner(&book, &chapter, Some("current-owner")),
            expected
        );
        assert_eq!(work().snapshots, 0);
        assert_eq!(work().owners, 0);
        assert_eq!(work().status_checks, 2);
    }
}

fn full_detail_case() -> Result<()> {
    let fixture = Fixture::new("Reader fixture", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let session = account(Some("snapshot-account-a"), "a");
        let owner = configure(&mut state, &session)?;
        let count = state.profile.detail_chapter_page_size() + 3;
        seed_catalog(&mut state, count, &owner, ContentStatus::Full, true)?;
        let reads = state.store.materialized_chapter_bytes();
        for tab in ["toc", "notes", "toc"] {
            state.detail_tab = tab.into();
            reset_work();
            state.render_detail()?;
            assert_one_snapshot(&session, false);
            assert!(work().status_checks >= count);
            let detail = fixture.ui().global::<DetailModel>();
            assert!(detail.get_can_read());
            assert!(!detail.get_access_needs_refresh());
            assert!(detail.get_chapters().row_count() > 0);
            assert!(detail.get_chapters().iter().all(|row| row.cached));
            assert_eq!(state.store.materialized_chapter_bytes(), reads);
        }
        let book = state.store.book(BOOK)?.unwrap();
        for chapter in state.store.chapter_metadata(BOOK)? {
            assert_eq!(
                chapter_content_status_for_owner(&book, &chapter, Some(&owner)),
                ContentStatus::Full
            );
        }
        let mut local = book;
        local.source = "local".into();
        state.store.upsert_book(&local)?;
        reset_work();
        state.render_detail()?;
        assert_eq!(work().snapshots, 0);
        assert_eq!(work().owners, 0);
        assert!(fixture.ui().global::<DetailModel>().get_can_read());
        assert!(
            !fixture
                .ui()
                .global::<DetailModel>()
                .get_access_needs_refresh()
        );
        assert_eq!(state.store.materialized_chapter_bytes(), reads);
        assert_eq!(state.jobs.session(), Some(session));
    }
    fixture.finish()
}

fn unknown_detail_case() -> Result<()> {
    let fixture = Fixture::new("Reader fixture", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let session = account(None, "no-vid");
        configure(&mut state, &session)?;
        let count = state.profile.detail_chapter_page_size() + 3;
        seed_catalog(&mut state, count, "", ContentStatus::Unknown, false)?;
        let reads = state.store.materialized_chapter_bytes();
        for _ in 0..3 {
            reset_work();
            state.render_detail()?;
            assert_one_snapshot(&session, true);
            assert!(work().status_checks >= count);
            let detail = fixture.ui().global::<DetailModel>();
            assert!(!detail.get_can_read());
            assert!(!detail.get_access_needs_refresh());
            assert!(detail.get_chapters().iter().all(|row| !row.cached));
            assert_eq!(state.store.materialized_chapter_bytes(), reads);
        }
    }
    fixture.finish()
}

fn changed_owner_case() -> Result<()> {
    let fixture = Fixture::new("Reader fixture", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let first = account(Some("snapshot-account-a"), "a");
        let first_owner = configure(&mut state, &first)?;
        seed_catalog(&mut state, 3, &first_owner, ContentStatus::Full, true)?;
        let note = Annotation {
            id: "owner-snapshot-note".into(),
            book_id: BOOK.into(),
            chapter_index: 0,
            start: 0,
            end: 4,
            quote: "Pres".into(),
            note: "Preserved owner snapshot note".into(),
            created_at: 100,
        };
        state.store.save_annotation(&note)?;
        let parked = DownloadTask {
            id: "owner-snapshot-parked".into(),
            book_id: BOOK.into(),
            account_key: first_owner.clone(),
            chapters: vec![0],
            status: DownloadStatus::Paused,
            updated_at: 100,
            ..Default::default()
        };
        state.store.save_task(&parked)?;
        let chapters = state.store.chapter_metadata(BOOK)?;
        let book = state.store.book(BOOK)?.unwrap();
        let same_vid = account(Some("snapshot-account-a"), "rotated-cookie");
        state.jobs.replace_session(same_vid.clone())?;
        reset_work();
        let stable_owner = state.chapter_content_owner().unwrap();
        assert_one_snapshot(&same_vid, false);
        assert_eq!(stable_owner, first_owner);
        assert!(chapters.iter().all(|chapter| {
            chapter_content_status_for_owner(&book, chapter, Some(&stable_owner))
                == ContentStatus::Full
        }));
        let mut previous_owner = stable_owner;
        for session in [
            account(Some("snapshot-account-b"), "b"),
            account(None, "no-vid-first"),
            account(None, "no-vid-next"),
        ] {
            state.jobs.replace_session(session.clone())?;
            reset_work();
            let owner = state.chapter_content_owner().unwrap();
            assert_one_snapshot(&session, session.user_vid.is_empty());
            assert_ne!(owner, first_owner);
            assert_ne!(owner, previous_owner);
            assert!(chapters.iter().all(|chapter| {
                chapter_content_status_for_owner(&book, chapter, Some(&owner))
                    == ContentStatus::Unknown
            }));
            reset_work();
            state.render_detail()?;
            assert_one_snapshot(&session, session.user_vid.is_empty());
            let detail = fixture.ui().global::<DetailModel>();
            assert!(detail.get_can_read());
            assert!(detail.get_access_needs_refresh());
            assert!(detail.get_chapters().iter().all(|row| !row.cached));
            previous_owner = owner;
        }
        assert_eq!(state.store.chapter_metadata(BOOK)?, chapters);
        assert_eq!(state.store.chapter_body(BOOK, 0)?.as_deref(), Some(BODY));
        assert_eq!(state.store.annotations(BOOK)?, vec![note]);
        assert_eq!(state.store.task(&parked.id)?, Some(parked));
        assert_eq!(state.session_file.load()?, Some(first));
    }
    fixture.finish()
}

fn planning_case() -> Result<()> {
    let fixture = Fixture::new("Reader fixture", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let session = account(None, "planner");
        let owner = configure(&mut state, &session)?;
        let count = state.profile.detail_chapter_page_size() + 3;
        seed_catalog(&mut state, count, &owner, ContentStatus::Full, true)?;
        let chapters = (0..count as u32).collect::<Vec<_>>();
        let reads = state.store.materialized_chapter_bytes();
        reset_work();
        state.enqueue_download_options(BOOK, chapters.clone(), false)?;
        assert_one_snapshot(&session, true);
        assert_eq!(work().status_checks, count);
        assert!(state.current_download.is_none());
        assert!(state.jobs.active_download().is_none());
        assert_eq!(state.store.materialized_chapter_bytes(), reads);

        let task = DownloadTask {
            id: "owner-snapshot-resume".into(),
            book_id: BOOK.into(),
            account_key: owner,
            chapters,
            next_index: count,
            status: DownloadStatus::Paused,
            include_images: false,
            updated_at: 100,
            ..Default::default()
        };
        state.store.save_task(&task)?;
        state.downloads.push_back(task.clone());
        state.force_offline = false;
        state.online = true;
        reset_work();
        // The real current-thread Jobs runtime is not driven. The resumed
        // completed cursor cannot fetch text, even if it is later polled.
        state.start_next_download()?;
        assert_one_snapshot(&session, true);
        assert_eq!(work().status_checks, count);
        let running = state.current_download.as_ref().unwrap();
        assert_eq!(running.id, task.id);
        assert_eq!(running.next_index, count);
        assert_eq!(running.status, DownloadStatus::Running);
        assert_eq!(
            state.jobs.active_download().as_deref(),
            Some(task.id.as_str())
        );
        assert_eq!(state.store.materialized_chapter_bytes(), reads);
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

#[test]
fn repeated_full_details_copy_one_real_session_for_the_complete_catalog() -> Result<()> {
    isolated(
        "app::library::content_owner_snapshot_tests::repeated_full_details_copy_one_real_session_for_the_complete_catalog",
        full_detail_case,
    )
}

#[test]
fn unknown_empty_owner_details_serialize_no_vid_cookies_once_per_render() -> Result<()> {
    isolated(
        "app::library::content_owner_snapshot_tests::unknown_empty_owner_details_serialize_no_vid_cookies_once_per_render",
        unknown_detail_case,
    )
}

#[test]
fn account_replacement_and_cookie_only_owners_are_fresh_for_each_operation() -> Result<()> {
    isolated(
        "app::library::content_owner_snapshot_tests::account_replacement_and_cookie_only_owners_are_fresh_for_each_operation",
        changed_owner_case,
    )
}

#[test]
fn real_download_admission_and_resume_borrow_the_operation_owner() -> Result<()> {
    isolated(
        "app::library::content_owner_snapshot_tests::real_download_admission_and_resume_borrow_the_operation_owner",
        planning_case,
    )
}
