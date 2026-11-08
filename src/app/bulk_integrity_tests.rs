//! Real Library enqueue/prefix consumers of one aligned image verification scope.

use super::underline_projection_tests::{BOOK, CHAPTER, Fixture, isolated};
use super::*;
use crate::source::images::{DownloadedChapterImages, ImageAsset};
use sha2::{Digest, Sha256};
use std::io::Cursor;

const SECOND: &str = "underline-fixture-chapter-1";
const TEXT: &str = "Before\nAfter";
const HTML: &str = "<p>Before</p><img src='images/shared.png'><p>After</p>";

fn account() -> source::AuthSession {
    source::AuthSession {
        user_vid: "synthetic-bulk-account".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "synthetic-bulk-account".into()),
            ("wr_skey".into(), "synthetic-bulk-session".into()),
        ]),
        ..Default::default()
    }
}

fn tiny_packet() -> Result<DownloadedChapterImages> {
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([31, 79, 113])))
        .write_to(&mut output, image::ImageFormat::Png)?;
    let bytes = output.into_inner();
    let id = hex::encode(Sha256::digest(&bytes));
    Ok(DownloadedChapterImages {
        source_to_asset: BTreeMap::from([("images/shared.png".into(), id.clone())]),
        assets: vec![ImageAsset {
            id,
            media_type: "image/png".into(),
            extension: "png".into(),
            source_names: vec!["images/shared.png".into()],
            bytes,
        }],
    })
}

fn seed(state: &mut App, publish_second: bool) -> Result<String> {
    state.jobs.replace_session(account())?;
    state.demo = false;
    state.force_offline = true;
    let owner = state
        .jobs
        .session()
        .as_ref()
        .and_then(library::download_account_key)
        .unwrap();
    let mut book = state.store.book(BOOK)?.unwrap();
    book.source = "weread".into();
    book.on_shelf = false;
    state.store.upsert_book(&book)?;
    let packet = tiny_packet()?;
    for (index, uid) in [CHAPTER, SECOND].into_iter().enumerate() {
        state.store.upsert_chapter(&Chapter {
            id: uid.into(),
            book_id: BOOK.into(),
            index: index as u32,
            title: format!("Bulk fixture chapter {index}"),
            body: Some(TEXT.into()),
            cached: true,
            content_status: ContentStatus::Full,
            content_account_key: owner.clone(),
            ..Default::default()
        })?;
        if index == 0 || publish_second {
            state
                .book_assets
                .commit_chapter(&state.store, BOOK, uid, HTML, TEXT, Some(&packet))?;
        }
    }
    state.store.save_annotation(&Annotation {
        id: "bulk-ui-note".into(),
        book_id: BOOK.into(),
        chapter_index: 0,
        start: 0,
        end: 6,
        quote: "Before".into(),
        note: "Retained local note".into(),
        created_at: 1,
    })?;
    state.store.save_bookmark(&Bookmark {
        id: "bulk-ui-mark".into(),
        book_id: BOOK.into(),
        chapter_index: 0,
        char_offset: 1,
        label: "Retained local bookmark".into(),
        created_at: 1,
    })?;
    Ok(owner)
}

fn assert_retained(state: &App, session: &Option<source::AuthSession>, body: &str) -> Result<()> {
    assert_eq!(&state.jobs.session(), session);
    assert_eq!(state.reader.body, body);
    assert_eq!(state.store.chapter_body(BOOK, 0)?.as_deref(), Some(TEXT));
    assert_eq!(
        state.store.annotations(BOOK)?[0].note,
        "Retained local note"
    );
    assert_eq!(state.store.bookmarks(BOOK)?[0].char_offset, 1);
    Ok(())
}

fn shared_enqueue() -> Result<()> {
    let fixture = Fixture::new(TEXT, Vec::new(), false)?;
    {
        let mut state = fixture.app();
        seed(&mut state, true)?;
        let session = state.jobs.session();
        let body = state.reader.body.clone();
        state.enqueue_download_options(BOOK, vec![0, 1], true)?;
        let work = state.book_assets.bulk_verification_stats();
        assert_eq!(work.scope_count, 1);
        assert_eq!(work.unique_chapters, 2);
        assert_eq!(work.manifest_reads, 2);
        assert_eq!(work.body_reads, 2);
        assert_eq!(work.hashes, 1);
        assert_eq!(work.unique_keys, 1);
        assert!(state.downloads.is_empty() && state.current_download.is_none());
        assert_eq!(
            fixture.ui().global::<ShellModel>().get_toast(),
            tr("already_cached")
        );
        assert_retained(&state, &session, &body)?;
    }
    fixture.finish()
}

#[test]
fn enqueue_verifies_shared_png_for_many_uids_in_one_scope() -> Result<()> {
    isolated(
        "app::bulk_integrity_tests::enqueue_verifies_shared_png_for_many_uids_in_one_scope",
        shared_enqueue,
    )
}

fn enqueue_failure() -> Result<()> {
    let fixture = Fixture::new(TEXT, Vec::new(), false)?;
    {
        let mut state = fixture.app();
        seed(&mut state, true)?;
        let original = state.store.book(BOOK)?.unwrap();
        let manifests = state
            .store
            .settings_with_prefix::<serde_json::Value>("chapter.assets:")?;
        let session = state.jobs.session();
        let body = state.reader.body.clone();
        let error = super::library::with_bulk_image_test_work(0, || {
            state.enqueue_download_options(BOOK, vec![0, 1], true)
        })
        .unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::cache::VerificationLimitExceeded>()
                .is_some()
        );
        assert_eq!(state.store.book(BOOK)?.unwrap(), original);
        assert_eq!(
            state
                .store
                .settings_with_prefix::<serde_json::Value>("chapter.assets:")?,
            manifests
        );
        assert!(
            state
                .store
                .setting::<bool>(&format!("shelf.override:{BOOK}"))?
                .is_none()
        );
        assert!(state.downloads.is_empty() && state.current_download.is_none());
        assert!(state.store.pending_tasks()?.is_empty());
        assert_eq!(state.book_assets.bulk_verification_stats().hashes, 0);
        assert_retained(&state, &session, &body)?;
    }
    fixture.finish()
}

#[test]
fn enqueue_quota_error_preserves_shelf_and_does_not_guess_download_need() -> Result<()> {
    isolated(
        "app::bulk_integrity_tests::enqueue_quota_error_preserves_shelf_and_does_not_guess_download_need",
        enqueue_failure,
    )
}

fn prefix_failure() -> Result<()> {
    let fixture = Fixture::new(TEXT, Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let owner = seed(&mut state, true)?;
        let original = DownloadTask {
            id: "bulk-ui-prefix".into(),
            book_id: BOOK.into(),
            account_key: owner,
            chapters: vec![0, 1],
            next_index: 2,
            status: DownloadStatus::Paused,
            include_images: true,
            updated_at: 123,
            ..Default::default()
        };
        state.store.save_task(&original)?;
        state.downloads.push_back(original.clone());
        state.force_offline = false;
        state.online = true;
        let session = state.jobs.session();
        let body = state.reader.body.clone();
        let manifests = state
            .store
            .settings_with_prefix::<serde_json::Value>("chapter.assets:")?;
        let error = super::library::with_bulk_image_test_work(0, || state.start_next_download())
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::cache::VerificationLimitExceeded>()
                .is_some()
        );
        assert_eq!(state.downloads.len(), 1);
        assert_eq!(state.downloads.front(), Some(&original));
        assert_eq!(state.store.task(&original.id)?, Some(original.clone()));
        assert!(state.current_download.is_none());
        assert_eq!(state.book_assets.bulk_verification_stats().scope_count, 1);
        assert_eq!(state.book_assets.bulk_verification_stats().hashes, 0);
        assert_eq!(
            state
                .store
                .settings_with_prefix::<serde_json::Value>("chapter.assets:")?,
            manifests
        );
        assert_retained(&state, &session, &body)?;
        // An internal dispatch cannot retry a latched resource refusal. The
        // same explicit inventory reload used by Sync starts the new operation.
        assert!(state.download_inventory_unavailable);
        state.start_next_download()?;
        assert_eq!(state.downloads.front(), Some(&original));
        assert_eq!(state.store.task(&original.id)?, Some(original.clone()));
        assert!(state.current_download.is_none());
        assert_eq!(state.book_assets.bulk_verification_stats().hashes, 0);
        assert!(state.reload_pending_downloads(true)?);
        assert!(!state.download_inventory_unavailable);
        // The fully proved prefix has no remaining network chapter. The
        // explicit retry still requires its own one-scope shared PNG proof.
        state.start_next_download()?;
        assert!(state.downloads.is_empty());
        assert_eq!(state.current_download.as_ref().unwrap().next_index, 2);
        let work = state.book_assets.bulk_verification_stats();
        assert_eq!(work.scope_count, 1);
        assert_eq!(work.unique_chapters, 2);
        assert_eq!(work.hashes, 1);
        assert_retained(&state, &session, &body)?;
        state.force_offline = true;
    }
    fixture.finish()
}

#[test]
fn prefix_quota_failure_returns_the_original_popped_task_and_cursor() -> Result<()> {
    isolated(
        "app::bulk_integrity_tests::prefix_quota_failure_returns_the_original_popped_task_and_cursor",
        prefix_failure,
    )
}

fn missing_manifest() -> Result<()> {
    let fixture = Fixture::new(TEXT, Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let owner = seed(&mut state, false)?;
        let mut other = Book::new("bulk-other-book", "Already running book");
        other.source = "weread".into();
        state.store.upsert_book(&other)?;
        let running = DownloadTask {
            id: "bulk-other-active".into(),
            book_id: other.id.clone(),
            account_key: owner,
            chapters: vec![0],
            status: DownloadStatus::Running,
            updated_at: 123,
            ..Default::default()
        };
        state.store.save_task(&running)?;
        state.current_download = Some(running.clone());
        let session = state.jobs.session();
        let body = state.reader.body.clone();
        state.enqueue_download_options(BOOK, vec![0, 1], true)?;
        assert_eq!(state.current_download.as_ref(), Some(&running));
        let queued = state.downloads.front().unwrap();
        assert_eq!(queued.chapters, vec![1]);
        assert_eq!(queued.next_index, 0);
        assert!(queued.include_images);
        assert_eq!(queued.status, DownloadStatus::Pending);
        assert_eq!(state.book_assets.bulk_verification_stats().scope_count, 1);
        assert_eq!(state.book_assets.bulk_verification_stats().hashes, 1);
        assert_retained(&state, &session, &body)?;
        state.current_download = None;
    }
    fixture.finish()
}

#[test]
fn a_known_missing_manifest_has_an_aligned_false_result_distinct_from_quota() -> Result<()> {
    isolated(
        "app::bulk_integrity_tests::a_known_missing_manifest_has_an_aligned_false_result_distinct_from_quota",
        missing_manifest,
    )
}
