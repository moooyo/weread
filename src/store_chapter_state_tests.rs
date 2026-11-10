//! Durable chapter isolation, out-of-order checkpoints, and restart fences.

use super::*;
use crate::domain::{DownloadChapterRetry, DownloadChapterStatus};

const BOOK: &str = "independent-chapter-book";
const OWNER: &str = "synthetic-independent-owner";
const REJECTION: &str = "source.api.-10102";

fn fixture_at(path: impl AsRef<std::path::Path>) -> Result<(Store, DownloadTask)> {
    let store = Store::open(path)?;
    let mut book = Book::new(BOOK, "Independent chapter fixture");
    book.source = "weread".into();
    book.total_chapters = 3;
    store.upsert_book(&book)?;
    for index in 0..3 {
        store.upsert_chapter(&Chapter {
            id: format!("independent-uid-{index}"),
            book_id: BOOK.into(),
            index,
            title: format!("Chapter {index}"),
            ..Default::default()
        })?;
    }
    let task = DownloadTask {
        id: "independent-task".into(),
        book_id: BOOK.into(),
        account_key: OWNER.into(),
        chapters: vec![0, 1, 2],
        status: DownloadStatus::Running,
        updated_at: 100,
        ..Default::default()
    };
    store.save_task(&task)?;
    store.prepare_download_chapter_states(&task, 100)?;
    let task = reload(&store, &task)?;
    Ok((store, task))
}

fn fixture() -> Result<(Store, DownloadTask)> {
    fixture_at(":memory:")
}

fn reload(store: &Store, task: &DownloadTask) -> Result<DownloadTask> {
    store.task(&task.id)?.context("fixture task disappeared")
}

fn packet(index: u32) -> Chapter {
    Chapter {
        id: format!("independent-uid-{index}"),
        book_id: BOOK.into(),
        index,
        title: format!("Chapter {index}"),
        cached: true,
        body: Some(format!(
            "Chapter {index}: independently verified full content."
        )),
        content_status: ContentStatus::Full,
        content_account_key: OWNER.into(),
    }
}

fn publish_imageless_chapter(
    assets: &crate::book_assets::BookAssetStore,
    store: &Store,
    chapter_uid: &str,
    text: &str,
) -> Result<crate::book_assets::VerifiedChapterPublication> {
    let html = format!("<p>{text}</p>");
    let metadata = crate::book_assets::prepare_chapter_metadata(&html, text)?;
    assets
        .commit_prepared_download_chapter_metadata(
            store,
            BOOK,
            chapter_uid,
            &html,
            text,
            &metadata,
            None,
        )?
        .context("an imageless fixture chapter must publish completely")
}

fn running_mixed_chapter_fixture() -> Result<(Store, DownloadTask)> {
    let (store, mut task) = fixture()?;
    for index in 3..5 {
        store.upsert_chapter(&Chapter {
            id: format!("independent-uid-{index}"),
            book_id: BOOK.into(),
            index,
            title: format!("Chapter {index}"),
            ..Default::default()
        })?;
    }
    let mut book = store.book(BOOK)?.unwrap();
    book.total_chapters = 5;
    store.upsert_book(&book)?;
    task.chapters = (0..5).collect();
    store.save_task(&task)?;
    store.prepare_download_chapter_states(&task, 100)?;
    task = reload(&store, &task)?;
    for now in [101, 161, 281, 521] {
        store.record_download_chapter_failure(
            &task,
            "independent-uid-3",
            3,
            3,
            true,
            REJECTION,
            now,
        )?;
        task = reload(&store, &task)?;
    }
    store.record_download_chapter_failure(
        &task,
        "independent-uid-4",
        4,
        4,
        false,
        "source.content_unverified",
        522,
    )?;
    task = reload(&store, &task)?;
    store.commit_independent_download_chapter(&task, &packet(0), 0, 523)?;
    task = reload(&store, &task)?;
    store.start_download_chapter_attempt(&task, "independent-uid-2", 2, 2, 524)?;
    task = reload(&store, &task)?;
    store.record_download_chapter_failure(
        &task,
        "independent-uid-1",
        1,
        1,
        true,
        REJECTION,
        525,
    )?;
    task = reload(&store, &task)?;
    Ok((store, task))
}

#[test]
fn later_chapters_commit_immediately_while_an_earlier_chapter_waits() -> Result<()> {
    let (store, mut task) = fixture()?;
    assert_eq!(
        store.record_download_chapter_failure(
            &task,
            "independent-uid-0",
            0,
            0,
            true,
            REJECTION,
            101,
        )?,
        DownloadChapterRetry::RetryAt(161),
    );
    task = reload(&store, &task)?;
    let committed = store.commit_independent_download_chapter(&task, &packet(2), 2, 102)?;
    assert!(committed.newly_cached);
    assert_eq!(committed.progress.completed_count, 1);
    assert_eq!(committed.progress.next_index, 0);
    assert_eq!(committed.progress.retry_wait_count, 1);
    assert_eq!(store.chapter_body(BOOK, 2)?, packet(2).body);
    assert_eq!(store.chapter_body(BOOK, 0)?, None);
    assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 1);

    task = reload(&store, &task)?;
    assert_eq!(task.status, DownloadStatus::Running);
    assert_eq!(task.completed_count, 1);
    assert_eq!(task.next_index, 0);
    let states = store.download_chapter_states(&task.id)?;
    assert_eq!(states[0].status, DownloadChapterStatus::RetryWait);
    assert_eq!(states[0].retry_count, 1);
    assert_eq!(states[0].retry_not_before, 161);
    assert_eq!(states[0].error.as_deref(), Some(REJECTION));
    assert_eq!(states[2].status, DownloadChapterStatus::Complete);

    let committed = store.commit_independent_download_chapter(&task, &packet(1), 1, 103)?;
    assert_eq!(committed.progress.completed_count, 2);
    assert_eq!(committed.progress.next_index, 0);
    task = reload(&store, &task)?;
    let committed = store.commit_independent_download_chapter(&task, &packet(0), 0, 161)?;
    assert_eq!(committed.progress.completed_count, 3);
    assert_eq!(committed.progress.next_index, 3);
    assert_eq!(committed.progress.retry_wait_count, 0);
    assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 3);
    Ok(())
}

#[test]
fn retry_budgets_are_independent_and_exhaustion_does_not_pause_other_chapters() -> Result<()> {
    let (store, mut task) = fixture()?;
    for (index, now, retry_at) in [
        (0, 101, 161),
        (1, 102, 162),
        (0, 201, 321),
        (1, 202, 322),
        (0, 401, 641),
    ] {
        assert_eq!(
            store.record_download_chapter_failure(
                &task,
                &format!("independent-uid-{index}"),
                index,
                index as usize,
                true,
                REJECTION,
                now,
            )?,
            DownloadChapterRetry::RetryAt(retry_at),
        );
        task = reload(&store, &task)?;
        assert_eq!(task.status, DownloadStatus::Running);
    }
    let states = store.download_chapter_states(&task.id)?;
    assert_eq!(states[0].retry_count, 3);
    assert_eq!(states[1].retry_count, 2);
    assert_eq!(states[2].retry_count, 0);
    assert_eq!(states[2].status, DownloadChapterStatus::Pending);

    store.commit_independent_download_chapter(&task, &packet(1), 1, 402)?;
    task = reload(&store, &task)?;
    assert_eq!(
        store.record_download_chapter_failure(
            &task,
            "independent-uid-0",
            0,
            0,
            true,
            REJECTION,
            641,
        )?,
        DownloadChapterRetry::Blocked,
    );
    task = reload(&store, &task)?;
    assert_eq!(task.status, DownloadStatus::Running);
    store.commit_independent_download_chapter(&task, &packet(2), 2, 642)?;
    task = reload(&store, &task)?;
    let states = store.download_chapter_states(&task.id)?;
    assert_eq!(states[0].status, DownloadChapterStatus::Blocked);
    assert_eq!(states[0].retry_count, 4);
    assert_eq!(states[0].retry_not_before, 0);
    assert_eq!(states[1].status, DownloadChapterStatus::Complete);
    assert_eq!(states[2].status, DownloadChapterStatus::Complete);
    assert_eq!(task.completed_count, 2);
    assert_eq!(task.next_index, 0);
    task.status = DownloadStatus::Complete;
    assert!(store.save_task(&task).is_err());
    Ok(())
}

#[test]
fn restart_preserves_out_of_order_success_and_only_the_failed_chapter_budget() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("chapter-state.sqlite");
    let (store, mut task) = fixture_at(&path)?;
    store.record_download_chapter_failure(
        &task,
        "independent-uid-0",
        0,
        0,
        true,
        REJECTION,
        101,
    )?;
    task = reload(&store, &task)?;
    store.commit_independent_download_chapter(&task, &packet(2), 2, 102)?;
    drop(store);

    let store = Store::open(&path)?;
    let mut resumed = reload(&store, &task)?;
    assert_eq!(resumed.status, DownloadStatus::Paused);
    assert_eq!(resumed.completed_count, 1);
    resumed.status = DownloadStatus::Running;
    resumed.updated_at = 110;
    store.save_task(&resumed)?;
    let states = store.prepare_download_chapter_states(&resumed, 110)?;
    assert_eq!(states.len(), 3);
    assert_eq!(states[0].status, DownloadChapterStatus::RetryWait);
    assert_eq!(states[0].retry_count, 1);
    assert_eq!(states[0].retry_not_before, 161);
    assert_eq!(states[1].status, DownloadChapterStatus::Pending);
    assert_eq!(states[2].status, DownloadChapterStatus::Complete);
    let resumed = reload(&store, &resumed)?;
    assert_eq!(resumed.completed_count, 1);
    assert_eq!(resumed.next_index, 0);
    assert_eq!(store.chapter_body(BOOK, 2)?, packet(2).body);
    Ok(())
}

#[test]
fn chapter_commit_rolls_back_content_count_and_state_when_task_update_fails() -> Result<()> {
    let (store, task) = fixture()?;
    let states = store.download_chapter_states(&task.id)?;
    store.connection.execute_batch(
        "CREATE TEMP TRIGGER reject_independent_task BEFORE UPDATE ON download_tasks
         BEGIN SELECT RAISE(ABORT,'synthetic independent checkpoint failure'); END",
    )?;
    assert!(
        store
            .commit_independent_download_chapter(&task, &packet(2), 2, 101)
            .is_err()
    );
    assert_eq!(store.task(&task.id)?.unwrap(), task);
    assert_eq!(store.download_chapter_states(&task.id)?, states);
    assert_eq!(store.chapter_body(BOOK, 2)?, None);
    assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 0);
    store
        .connection
        .execute_batch("DROP TRIGGER reject_independent_task")?;
    store.commit_independent_download_chapter(&task, &packet(2), 2, 101)?;
    assert_eq!(store.task(&task.id)?.unwrap().completed_count, 1);
    Ok(())
}

#[test]
fn failure_does_not_release_a_retry_without_durable_state() -> Result<()> {
    let (store, task) = fixture()?;
    let states = store.download_chapter_states(&task.id)?;
    store.connection.execute_batch(
        "CREATE TEMP TRIGGER reject_independent_failure BEFORE UPDATE ON download_tasks
         BEGIN SELECT RAISE(ABORT,'synthetic chapter retry failure'); END",
    )?;
    assert!(
        store
            .record_download_chapter_failure(
                &task,
                "independent-uid-0",
                0,
                0,
                true,
                REJECTION,
                101,
            )
            .is_err()
    );
    assert_eq!(store.task(&task.id)?.unwrap(), task);
    assert_eq!(store.download_chapter_states(&task.id)?, states);
    Ok(())
}

#[test]
fn independent_commits_reject_changed_scope_account_cancellation_and_packet_identity() -> Result<()>
{
    for mutation in [
        "account",
        "plan",
        "cancel",
        "images",
        "timestamp",
        "packet_uid",
        "packet_index",
        "packet_owner",
        "ordinal",
    ] {
        let (store, task) = fixture()?;
        let mut changed = task.clone();
        let mut chapter = packet(2);
        let mut ordinal = 2;
        match mutation {
            "account" => changed.account_key = "synthetic-other-owner".into(),
            "plan" => changed.chapters.reverse(),
            "cancel" => changed.status = DownloadStatus::Cancelled,
            "images" => changed.include_images = true,
            "timestamp" => changed.updated_at = 101,
            "packet_uid" => chapter.id = "another-chapter-uid".into(),
            "packet_index" => chapter.index = 1,
            "packet_owner" => chapter.content_account_key = "synthetic-other-owner".into(),
            "ordinal" => ordinal = 1,
            _ => unreachable!(),
        }
        if changed != task {
            store.save_task(&changed)?;
        }
        assert!(
            store
                .validate_independent_download_chapter(&task, &chapter, ordinal)
                .is_err(),
            "Preflight must reject {mutation}",
        );
        assert!(
            store
                .commit_independent_download_chapter(&task, &chapter, ordinal, 102)
                .is_err(),
            "Commit must reject {mutation}",
        );
        assert_eq!(store.chapter_body(BOOK, 2)?, None);
        assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 0);
        assert_eq!(store.task(&task.id)?.unwrap(), changed);
    }
    Ok(())
}

#[test]
fn a_completed_chapter_rejects_repeated_success_and_late_failure() -> Result<()> {
    let (store, task) = fixture()?;
    store.commit_independent_download_chapter(&task, &packet(2), 2, 101)?;
    let current = reload(&store, &task)?;
    assert!(
        store
            .commit_independent_download_chapter(&current, &packet(2), 2, 102)
            .is_err()
    );
    assert!(
        store
            .record_download_chapter_failure(
                &current,
                "independent-uid-2",
                2,
                2,
                true,
                REJECTION,
                102,
            )
            .is_err()
    );
    assert!(
        store.record_download_chapter_failure(
            &task,
            "independent-uid-0",
            0,
            0,
            true,
            REJECTION,
            102,
        ).is_err()
    );
    assert_eq!(store.task(&task.id)?.unwrap(), current);
    assert_eq!(
        store.download_chapter_states(&task.id)?[2].status,
        DownloadChapterStatus::Complete
    );
    assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 1);
    Ok(())
}

#[test]
fn limited_content_blocks_only_its_chapter_and_cannot_complete_the_book() -> Result<()> {
    for status in [
        ContentStatus::Preview,
        ContentStatus::Unknown,
        ContentStatus::Locked,
    ] {
        let (store, mut task) = fixture()?;
        let mut limited = packet(0);
        limited.cached = false;
        limited.content_status = status;
        limited.body = (status != ContentStatus::Locked).then(|| "Readable excerpt.".into());
        let committed = store.commit_independent_download_chapter(&task, &limited, 0, 101)?;
        assert!(!committed.newly_cached);
        assert_eq!(committed.progress.completed_count, 0);
        assert_eq!(committed.progress.next_index, 0);
        task = reload(&store, &task)?;
        assert_eq!(
            store.record_download_chapter_failure(
                &task,
                &limited.id,
                0,
                0,
                false,
                "source.content_unverified",
                102,
            )?,
            DownloadChapterRetry::Blocked,
        );
        task = reload(&store, &task)?;
        assert_eq!(task.status, DownloadStatus::Running);
        store.commit_independent_download_chapter(&task, &packet(2), 2, 103)?;
        task = reload(&store, &task)?;
        assert_eq!(task.completed_count, 1);
        assert_eq!(task.next_index, 0);
        let saved = store.chapter(BOOK, 0)?.unwrap();
        assert_eq!(saved.content_status, status);
        assert_eq!(saved.body, limited.body);
        assert!(!saved.cached);
        assert_eq!(
            store.download_chapter_states(&task.id)?[0].status,
            DownloadChapterStatus::Blocked
        );
        task.status = DownloadStatus::Complete;
        task.next_index = task.chapters.len();
        assert!(store.save_task(&task).is_err());
    }
    Ok(())
}

#[test]
fn cached_completion_checks_real_body_owner_and_preserves_bodyless_transport() -> Result<()> {
    let (store, task) = fixture()?;
    store.upsert_chapter(&packet(2))?;
    let progress =
        store.commit_cached_download_chapter(&task, "independent-uid-2", 2, 2, OWNER, 101)?;
    assert_eq!(progress.completed_count, 1);
    assert_eq!(progress.next_index, 0);
    assert_eq!(store.chapter_body_read_count(), 0);
    assert_eq!(store.materialized_chapter_bytes(), 0);
    assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 1);

    use rusqlite::types::Value;
    for (body, status, owner) in [
        (Value::Null, "full", OWNER),
        (Value::Text(" \t\n\u{3000}".into()), "full", OWNER),
        (Value::Blob(b"Readable blob".to_vec()), "full", OWNER),
        (Value::Text("Readable excerpt".into()), "preview", OWNER),
        (Value::Text("Unverified content".into()), "unknown", OWNER),
        (
            Value::Text("Readable content".into()),
            "full",
            "another-owner",
        ),
    ] {
        let (store, task) = fixture()?;
        store.upsert_chapter(&packet(2))?;
        store.connection.execute(
            "UPDATE chapters SET body=?2,content_status=?3,content_account_key=?4
             WHERE book_id=?1 AND chapter_index=2",
            params![BOOK, body, status, owner],
        )?;
        assert!(
            store
                .commit_cached_download_chapter(&task, "independent-uid-2", 2, 2, OWNER, 101)
                .is_err()
        );
        assert_eq!(store.task(&task.id)?.unwrap(), task);
        assert_eq!(
            store.download_chapter_states(&task.id)?[2].status,
            DownloadChapterStatus::Pending
        );
        assert_eq!(store.chapter_body_read_count(), 0);
        assert_eq!(store.materialized_chapter_bytes(), 0);
    }
    Ok(())
}

#[test]
fn text_only_cached_entry_cannot_acknowledge_an_image_download() -> Result<()> {
    let (store, mut task) = fixture()?;
    task.include_images = true;
    store.save_task(&task)?;
    store.prepare_download_chapter_states(&task, 100)?;
    task = reload(&store, &task)?;
    store.upsert_chapter(&packet(2))?;
    assert!(
        store
            .commit_cached_download_chapter(&task, "independent-uid-2", 2, 2, OWNER, 101)
            .is_err()
    );
    assert_eq!(store.task(&task.id)?.unwrap(), task);
    assert_eq!(
        store.download_chapter_states(&task.id)?[2].status,
        DownloadChapterStatus::Pending
    );
    Ok(())
}

#[test]
fn cached_completion_authenticates_the_uid_ordinal_and_current_account() -> Result<()> {
    let (store, task) = fixture()?;
    store.upsert_chapter(&packet(2))?;
    for (uid, index, ordinal, owner) in [
        ("another-uid", 2, 2, OWNER),
        ("independent-uid-2", 1, 2, OWNER),
        ("independent-uid-2", 2, 1, OWNER),
        ("independent-uid-2", 2, 2, "another-owner"),
        ("independent-uid-2", 2, 2, ""),
    ] {
        assert!(
            store
                .commit_cached_download_chapter(&task, uid, index, ordinal, owner, 101)
                .is_err()
        );
        assert_eq!(store.task(&task.id)?.unwrap(), task);
    }
    store.connection.execute(
        "UPDATE chapters SET source_id='replacement-uid' WHERE book_id=?1 AND chapter_index=2",
        [BOOK],
    )?;
    assert!(
        store
            .commit_cached_download_chapter(&task, "independent-uid-2", 2, 2, OWNER, 101)
            .is_err()
    );
    assert_eq!(store.task(&task.id)?.unwrap(), task);
    assert_eq!(store.chapter_body_read_count(), 0);
    assert_eq!(store.materialized_chapter_bytes(), 0);
    Ok(())
}

#[test]
fn independent_body_commit_preserves_the_canonical_text_and_annotation_fence() -> Result<()> {
    let (store, task) = fixture()?;
    store.upsert_chapter(&packet(2))?;
    let annotation = Annotation {
        id: "independent-canonical-note".into(),
        book_id: BOOK.into(),
        chapter_index: 2,
        start: 0,
        end: 7,
        quote: "Chapter".into(),
        note: "Preserve the canonical text".into(),
        ..Default::default()
    };
    store.save_annotation(&annotation)?;
    let mut replacement = packet(2);
    replacement.body = Some("Different full text with incompatible anchors.".into());
    let error = store
        .validate_independent_download_chapter(&task, &replacement, 2)
        .unwrap_err();
    assert!(error.downcast_ref::<ContentConflict>().is_some());
    assert!(
        store
            .commit_independent_download_chapter(&task, &replacement, 2, 101)
            .is_err()
    );
    assert_eq!(store.chapter_body(BOOK, 2)?, packet(2).body);
    assert_eq!(store.annotations(BOOK)?, vec![annotation]);
    assert_eq!(store.task(&task.id)?.unwrap(), task);
    Ok(())
}

#[test]
fn resume_rechecks_access_and_body_instead_of_trusting_completed_state() -> Result<()> {
    for mutation in ["body", "status", "owner"] {
        let (store, mut task) = fixture()?;
        store.commit_independent_download_chapter(&task, &packet(2), 2, 101)?;
        task = reload(&store, &task)?;
        match mutation {
            "body" => store.connection.execute(
                "UPDATE chapters SET body=NULL WHERE book_id=?1 AND chapter_index=2", [BOOK],
            )?,
            "status" => store.connection.execute(
                "UPDATE chapters SET content_status='unknown' WHERE book_id=?1 AND chapter_index=2", [BOOK],
            )?,
            "owner" => store.connection.execute(
                "UPDATE chapters SET content_account_key='another-owner' WHERE book_id=?1 AND chapter_index=2", [BOOK],
            )?,
            _ => unreachable!(),
        };
        let states = store.prepare_download_chapter_states(&task, 102)?;
        assert_eq!(states[2].status, DownloadChapterStatus::Pending);
        task = reload(&store, &task)?;
        assert_eq!(task.completed_count, 0);
        assert_eq!(task.next_index, 0);
    }
    Ok(())
}

#[test]
fn catalog_reindex_preserves_uid_state_and_removes_unselected_rows() -> Result<()> {
    let (store, mut task) = fixture()?;
    store.record_download_chapter_failure(
        &task,
        "independent-uid-0",
        0,
        0,
        true,
        REJECTION,
        101,
    )?;
    task = reload(&store, &task)?;
    store.commit_independent_download_chapter(&task, &packet(2), 2, 102)?;
    let replacement: Vec<_> = [2, 0, 1]
        .into_iter()
        .enumerate()
        .map(|(index, uid)| Chapter {
            id: format!("independent-uid-{uid}"),
            book_id: BOOK.into(),
            index: index as u32,
            title: format!("Reindexed chapter {uid}"),
            ..Default::default()
        })
        .collect();
    store.replace_catalog(BOOK, &replacement)?;
    task = reload(&store, &task)?;
    assert_eq!(task.chapters, vec![1, 2, 0]);
    task.status = DownloadStatus::Running;
    task.updated_at = 110;
    store.save_task(&task)?;
    let states = store.prepare_download_chapter_states(&task, 110)?;
    assert_eq!(states[0].chapter_uid, "independent-uid-0");
    assert_eq!(states[0].chapter_index, 1);
    assert_eq!(states[0].status, DownloadChapterStatus::RetryWait);
    assert_eq!(states[0].retry_count, 1);
    assert_eq!(states[2].chapter_uid, "independent-uid-2");
    assert_eq!(states[2].chapter_index, 0);
    assert_eq!(states[2].status, DownloadChapterStatus::Complete);

    store.replace_catalog(BOOK, &replacement[..2])?;
    task = reload(&store, &task)?;
    task.status = DownloadStatus::Running;
    task.updated_at = 120;
    store.save_task(&task)?;
    let states = store.prepare_download_chapter_states(&task, 120)?;
    assert_eq!(states.len(), 2);
    assert!(
        states
            .iter()
            .all(|state| state.chapter_uid != "independent-uid-1")
    );
    assert_eq!(states[0].retry_count, 1);
    assert_eq!(states[1].status, DownloadChapterStatus::Complete);
    assert_eq!(store.chapter_body(BOOK, 0)?, packet(2).body);
    Ok(())
}

#[test]
fn terminal_lifecycle_releases_normalized_chapter_rows() -> Result<()> {
    let (store, mut task) = fixture()?;
    store.record_download_chapter_failure(
        &task,
        "independent-uid-0",
        0,
        0,
        true,
        REJECTION,
        101,
    )?;
    task = reload(&store, &task)?;
    task.status = DownloadStatus::Cancelled;
    store.save_task(&task)?;
    assert!(store.download_chapter_states(&task.id)?.is_empty());

    let (store, mut task) = fixture()?;
    for (attempt, index) in [2, 0, 1].into_iter().enumerate() {
        store.commit_independent_download_chapter(
            &task,
            &packet(index),
            index as usize,
            101 + attempt as i64,
        )?;
        task = reload(&store, &task)?;
    }
    assert_eq!(task.completed_count, 3);
    assert_eq!(task.next_index, 3);
    task.status = DownloadStatus::Complete;
    store.save_task(&task)?;
    assert!(store.download_chapter_states(&task.id)?.is_empty());
    assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 3);
    Ok(())
}

#[test]
fn starting_a_retry_requires_its_deadline_and_retains_the_chapter_budget() -> Result<()> {
    let (store, mut task) = fixture()?;
    store.record_download_chapter_failure(
        &task,
        "independent-uid-0",
        0,
        0,
        true,
        REJECTION,
        101,
    )?;
    task = reload(&store, &task)?;
    let waiting = store.download_chapter_states(&task.id)?;
    assert!(
        store
            .start_download_chapter_attempt(&task, "independent-uid-0", 0, 0, 160)
            .is_err()
    );
    assert_eq!(store.task(&task.id)?.unwrap(), task);
    assert_eq!(store.download_chapter_states(&task.id)?, waiting);

    let progress = store.start_download_chapter_attempt(&task, "independent-uid-0", 0, 0, 161)?;
    assert_eq!(progress.pending_count, 3);
    assert_eq!(progress.retry_wait_count, 0);
    assert_eq!(progress.retry_not_before, None);
    task = reload(&store, &task)?;
    let started = store.download_chapter_states(&task.id)?;
    assert_eq!(started[0].status, DownloadChapterStatus::Pending);
    assert_eq!(started[0].retry_count, 1);
    assert_eq!(started[0].retry_not_before, 0);
    assert_eq!(started[0].error, None);
    assert_eq!(started[1], waiting[1]);
    assert_eq!(started[2], waiting[2]);
    assert_eq!(task.updated_at, 161);
    assert_eq!(task.status, DownloadStatus::Running);

    assert_eq!(
        store.record_download_chapter_failure(
            &task,
            "independent-uid-0",
            0,
            0,
            true,
            REJECTION,
            162,
        )?,
        DownloadChapterRetry::RetryAt(282),
    );
    assert_eq!(store.download_chapter_states(&task.id)?[0].retry_count, 2);
    Ok(())
}

#[test]
fn explicit_resume_resets_only_blocked_chapters() -> Result<()> {
    let (store, mut task) = fixture()?;
    store.commit_independent_download_chapter(&task, &packet(2), 2, 101)?;
    task = reload(&store, &task)?;
    store.record_download_chapter_failure(
        &task,
        "independent-uid-1",
        1,
        1,
        false,
        "source.content_unverified",
        102,
    )?;
    task = reload(&store, &task)?;
    store.record_download_chapter_failure(
        &task,
        "independent-uid-0",
        0,
        0,
        true,
        REJECTION,
        103,
    )?;
    task = reload(&store, &task)?;
    task.status = DownloadStatus::Paused;
    task.updated_at = 104;
    store.save_task(&task)?;
    let before = store.download_chapter_states(&task.id)?;

    let progress = store.resume_download_chapters(&task, 105)?;
    assert_eq!(progress.completed_count, 1);
    assert_eq!(progress.next_index, 0);
    assert_eq!(progress.pending_count, 1);
    assert_eq!(progress.retry_wait_count, 1);
    assert_eq!(progress.blocked_count, 0);
    assert_eq!(progress.retry_not_before, Some(163));
    let after = store.download_chapter_states(&task.id)?;
    assert_eq!(after[0], before[0]);
    assert_eq!(after[2], before[2]);
    assert_eq!(after[1].status, DownloadChapterStatus::Pending);
    assert_eq!(after[1].retry_count, 0);
    assert_eq!(after[1].retry_not_before, 0);
    assert_eq!(after[1].error, None);
    let resumed = reload(&store, &task)?;
    assert_eq!(resumed.status, DownloadStatus::Paused);
    assert_eq!(resumed.completed_count, 1);
    assert_eq!(store.chapter_body(BOOK, 2)?, packet(2).body);
    Ok(())
}

#[test]
fn chapter_state_metadata_bounds_reject_oversized_uid_and_error_without_mutation() -> Result<()> {
    for (field, bytes) in [("chapter_uid", 4097), ("error", 257)] {
        let (store, task) = fixture()?;
        let oversized = "x".repeat(bytes);
        let update =
            format!("UPDATE task_chapter_states SET {field}=?2 WHERE task_id=?1 AND ordinal=0");
        store
            .connection
            .execute(&update, params![task.id, oversized])?;
        let error = store.download_chapter_states(&task.id).unwrap_err();
        assert!(error.downcast_ref::<StoreInventoryLimit>().is_some());
        assert!(store.prepare_download_chapter_states(&task, 101).is_err());
        assert_eq!(store.task(&task.id)?.unwrap(), task);
        let select =
            format!("SELECT {field} FROM task_chapter_states WHERE task_id=?1 AND ordinal=0");
        let retained: String = store
            .connection
            .query_row(&select, [&task.id], |row| row.get(0))?;
        assert_eq!(retained, oversized);
        assert_eq!(store.chapter_body_read_count(), 0);
        assert_eq!(store.materialized_chapter_bytes(), 0);
    }
    Ok(())
}

#[test]
fn chapter_state_type_admission_rejects_malformed_sql_values() -> Result<()> {
    use rusqlite::types::Value;

    for (field, value) in [
        ("chapter_uid", Value::Blob(b"untrusted-uid".to_vec())),
        ("error", Value::Blob(vec![0xff, 0x00])),
        ("retry_not_before", Value::Text("not-an-integer".into())),
    ] {
        let (store, task) = fixture()?;
        let update =
            format!("UPDATE task_chapter_states SET {field}=?2 WHERE task_id=?1 AND ordinal=0");
        store.connection.execute(&update, params![task.id, value])?;
        assert!(store.download_chapter_states(&task.id).is_err());
        assert!(store.prepare_download_chapter_states(&task, 101).is_err());
        assert_eq!(store.task(&task.id)?.unwrap(), task);
        let select =
            format!("SELECT {field} FROM task_chapter_states WHERE task_id=?1 AND ordinal=0");
        let retained: Value = store
            .connection
            .query_row(&select, [&task.id], |row| row.get(0))?;
        assert_eq!(retained, value);
        assert_eq!(store.chapter_body_read_count(), 0);
        assert_eq!(store.materialized_chapter_bytes(), 0);
    }
    Ok(())
}

#[test]
fn chapter_state_reads_honor_row_and_encoded_byte_allowances() -> Result<()> {
    let (store, task) = fixture()?;
    let expected = store.download_chapter_states(&task.id)?;
    for limits in [
        StoreInventoryLimits {
            rows: 2,
            ..StoreInventoryLimits::default()
        },
        StoreInventoryLimits {
            encoded_bytes: 1,
            ..StoreInventoryLimits::default()
        },
    ] {
        let error =
            with_store_inventory_test_limits(limits, || store.download_chapter_states(&task.id))
                .unwrap_err();
        assert!(error.downcast_ref::<StoreInventoryLimit>().is_some());
        assert_eq!(store.download_chapter_states(&task.id)?, expected);
        assert_eq!(store.task(&task.id)?.unwrap(), task);
        assert_eq!(store.chapter_body_read_count(), 0);
    }
    let exact = with_store_inventory_test_limits(
        StoreInventoryLimits {
            rows: 3,
            ..StoreInventoryLimits::default()
        },
        || store.download_chapter_states(&task.id),
    )?;
    assert_eq!(exact, expected);
    Ok(())
}

#[test]
fn chapter_failure_errors_are_redacted_before_persistence_and_on_legacy_reads() -> Result<()> {
    for (input, expected) in [
        (REJECTION.to_owned(), REJECTION),
        ("source.http.429".into(), "source.http.429"),
        (
            "source.invalid_response.synthetic-private-value".into(),
            "source.invalid_response",
        ),
        (
            "source.unsupported.synthetic-private-value".into(),
            "source.unsupported",
        ),
        (
            "source.api.-10102?cookie=synthetic-private-value".into(),
            "source.local_checkpoint_failed",
        ),
        (
            "source.api.12345678901234567".into(),
            "source.local_checkpoint_failed",
        ),
        (
            "synthetic-private-value".repeat(32),
            "source.local_checkpoint_failed",
        ),
    ] {
        let (store, task) = fixture()?;
        store.record_download_chapter_failure(
            &task,
            "independent-uid-0",
            0,
            0,
            true,
            &input,
            101,
        )?;
        let persisted: String = store.connection.query_row(
            "SELECT error FROM task_chapter_states WHERE task_id=?1 AND ordinal=0",
            [&task.id],
            |row| row.get(0),
        )?;
        assert_eq!(persisted, expected);
        assert_eq!(
            store.download_chapter_states(&task.id)?[0].error.as_deref(),
            Some(expected),
        );
    }

    let (store, task) = fixture()?;
    store.connection.execute(
        "UPDATE task_chapter_states SET error='synthetic-private-value'
         WHERE task_id=?1 AND ordinal=0",
        [&task.id],
    )?;
    let states = store.download_chapter_states(&task.id)?;
    assert_eq!(
        states[0].error.as_deref(),
        Some("source.local_checkpoint_failed"),
    );
    assert_eq!(store.task(&task.id)?.unwrap(), task);
    Ok(())
}

#[test]
fn chapter_retry_budget_survives_restart_without_a_task_error() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("running-chapter-state.sqlite");
    let (store, mut task) = fixture_at(&path)?;
    store.commit_independent_download_chapter(&task, &packet(0), 0, 101)?;
    task = reload(&store, &task)?;
    for now in [200, 380] {
        store.record_download_chapter_failure(
            &task,
            "independent-uid-1",
            1,
            1,
            true,
            REJECTION,
            now,
        )?;
        task = reload(&store, &task)?;
    }
    assert!(task.error.is_none());
    drop(store);

    let store = Store::open(&path)?;
    let mut resumed = reload(&store, &task)?;
    assert_eq!(resumed.status, DownloadStatus::Paused);
    assert_eq!(resumed.error, None);
    resumed.status = DownloadStatus::Running;
    resumed.updated_at = 200;
    store.save_task(&resumed)?;
    let states = store.prepare_download_chapter_states(&resumed, 200)?;
    assert_eq!(states[0].status, DownloadChapterStatus::Complete);
    assert_eq!(states[1].chapter_uid, "independent-uid-1");
    assert_eq!(states[1].status, DownloadChapterStatus::RetryWait);
    assert_eq!(states[1].retry_count, 2);
    assert_eq!(states[1].retry_not_before, 500);
    assert_eq!(states[2].status, DownloadChapterStatus::Pending);
    assert_eq!(states[2].retry_count, 0);
    resumed = reload(&store, &resumed)?;
    assert_eq!(resumed.completed_count, 1);
    assert_eq!(resumed.next_index, 1);
    assert!(
        store
            .start_download_chapter_attempt(&resumed, "independent-uid-1", 1, 1, 499)
            .is_err()
    );
    store.start_download_chapter_attempt(&resumed, "independent-uid-1", 1, 1, 500)?;
    resumed = reload(&store, &resumed)?;
    assert_eq!(
        store.record_download_chapter_failure(
            &resumed,
            "independent-uid-1",
            1,
            1,
            true,
            REJECTION,
            501,
        )?,
        DownloadChapterRetry::RetryAt(741),
    );
    assert_eq!(store.download_chapter_states(&task.id)?[1].retry_count, 3);
    Ok(())
}

#[test]
fn image_download_retains_full_text_without_completing_without_resource_proof() -> Result<()> {
    let (store, mut task) = fixture()?;
    task.include_images = true;
    task.chapters = vec![2];
    store.save_task(&task)?;
    store.prepare_download_chapter_states(&task, 100)?;
    task = reload(&store, &task)?;
    assert!(
        store
            .commit_independent_download_chapter(&task, &packet(2), 0, 101)
            .is_err()
    );
    assert_eq!(store.task(&task.id)?.unwrap(), task);
    assert_eq!(store.chapter_body(BOOK, 2)?, None);
    assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 0);

    let retained = store.commit_pending_image_download_chapter(&task, &packet(2), 0, 101)?;
    assert!(retained.newly_cached);
    assert_eq!(retained.progress.completed_count, 0);
    assert_eq!(retained.progress.next_index, 0);
    assert_eq!(retained.progress.pending_count, 1);
    let saved = store.chapter(BOOK, 2)?.unwrap();
    assert_eq!(saved.body, packet(2).body);
    assert_eq!(saved.content_status, ContentStatus::Full);
    assert!(saved.cached);
    assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 1);
    let current = reload(&store, &task)?;
    assert_eq!(current.completed_count, 0);
    assert_eq!(current.next_index, 0);
    let states = store.download_chapter_states(&task.id)?;
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].chapter_uid, "independent-uid-2");
    assert_eq!(states[0].status, DownloadChapterStatus::Pending);

    let mut forged_complete = current.clone();
    forged_complete.status = DownloadStatus::Complete;
    forged_complete.next_index = 1;
    forged_complete.completed_count = 1;
    assert!(store.save_task(&forged_complete).is_err());
    assert_eq!(store.task(&task.id)?.unwrap(), current);
    assert_eq!(store.download_chapter_states(&task.id)?, states);
    assert_eq!(store.chapter_body(BOOK, 2)?, packet(2).body);
    Ok(())
}

#[test]
fn image_publication_proof_is_bound_to_uid_and_canonical_body() -> Result<()> {
    for mismatch in ["uid", "body"] {
        let directory = tempfile::tempdir()?;
        let assets = crate::book_assets::BookAssetStore::open(
            directory.path().join("images"),
            64 * 1024 * 1024,
        )?;
        let (store, mut task) = fixture()?;
        task.include_images = true;
        store.save_task(&task)?;
        store.prepare_download_chapter_states(&task, 100)?;
        task = reload(&store, &task)?;
        let chapter = packet(2);
        let text = chapter.body.as_deref().unwrap();
        let proof_uid = if mismatch == "uid" {
            "independent-uid-1"
        } else {
            chapter.id.as_str()
        };
        let proof_text = if mismatch == "body" {
            "A different canonical body for the same chapter."
        } else {
            text
        };
        let proof = publish_imageless_chapter(&assets, &store, proof_uid, proof_text)?;
        let before = store.download_chapter_states(&task.id)?;
        assert!(
            store
                .commit_verified_image_download_chapter(&task, &chapter, 2, proof, 101)
                .is_err(),
            "A publication proof must reject a mismatched {mismatch}",
        );
        assert_eq!(store.task(&task.id)?.unwrap(), task);
        assert_eq!(store.download_chapter_states(&task.id)?, before);
        assert_eq!(store.chapter_body(BOOK, 2)?, None);
        assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 0);

        let proof = publish_imageless_chapter(&assets, &store, &chapter.id, text)?;
        let committed =
            store.commit_verified_image_download_chapter(&task, &chapter, 2, proof, 102)?;
        assert!(committed.newly_cached);
        assert_eq!(committed.progress.completed_count, 1);
        assert_eq!(committed.progress.next_index, 0);
        assert_eq!(store.chapter_body(BOOK, 2)?, chapter.body);
        assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 1);
        assert_eq!(
            store.download_chapter_states(&task.id)?[2].status,
            DownloadChapterStatus::Complete,
        );
    }
    Ok(())
}

#[test]
fn fresh_image_batch_proof_recovers_and_preserves_completion_across_restart() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let database = directory.path().join("image-recovery.sqlite");
    let image_directory = directory.path().join("images");
    let assets = crate::book_assets::BookAssetStore::open(&image_directory, 64 * 1024 * 1024)?;
    let (store, mut task) = fixture_at(&database)?;
    task.include_images = true;
    task.chapters = vec![2];
    store.save_task(&task)?;
    store.prepare_download_chapter_states(&task, 100)?;
    task = reload(&store, &task)?;
    let chapter = packet(2);
    store.commit_pending_image_download_chapter(&task, &chapter, 0, 101)?;
    task = reload(&store, &task)?;
    drop(publish_imageless_chapter(
        &assets,
        &store,
        &chapter.id,
        chapter.body.as_deref().unwrap(),
    )?);
    let proof = assets.verify_complete_book(&store, BOOK, &[chapter.id.as_str()])?;
    let states = store.prepare_download_chapter_states_with_verified_images(&task, proof, 102)?;
    assert_eq!(states[0].status, DownloadChapterStatus::Complete);
    task = reload(&store, &task)?;
    assert_eq!(task.completed_count, 1);
    assert_eq!(task.next_index, 1);
    drop(assets);
    drop(store);

    let store = Store::open(&database)?;
    let assets = crate::book_assets::BookAssetStore::open(&image_directory, 64 * 1024 * 1024)?;
    let mut resumed = reload(&store, &task)?;
    assert_eq!(resumed.status, DownloadStatus::Paused);
    assert_eq!(resumed.completed_count, 1);
    assert_eq!(resumed.next_index, 1);
    resumed.status = DownloadStatus::Running;
    resumed.updated_at = 110;
    store.save_task(&resumed)?;
    store.connection.execute_batch(
        "CREATE TEMP TRIGGER reject_image_completion_rewind BEFORE UPDATE ON download_tasks
         WHEN json_extract(NEW.payload,'$.completed_count') < json_extract(OLD.payload,'$.completed_count')
         BEGIN SELECT RAISE(ABORT,'synthetic image completion rewind'); END",
    )?;
    let proof = assets.verify_complete_book(&store, BOOK, &[chapter.id.as_str()])?;
    let states =
        store.prepare_download_chapter_states_with_verified_images(&resumed, proof, 111)?;
    assert_eq!(states[0].status, DownloadChapterStatus::Complete);
    let resumed = reload(&store, &resumed)?;
    assert_eq!(resumed.completed_count, 1);
    assert_eq!(resumed.next_index, 1);
    assert_eq!(store.chapter_body(BOOK, 2)?, chapter.body);
    assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 1);
    Ok(())
}

#[test]
fn image_batch_proof_requires_the_manifest_to_match_the_actual_sql_body() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let assets = crate::book_assets::BookAssetStore::open(
        directory.path().join("images"),
        64 * 1024 * 1024,
    )?;
    let (store, mut task) = fixture()?;
    task.include_images = true;
    task.chapters = vec![2];
    store.save_task(&task)?;
    store.prepare_download_chapter_states(&task, 100)?;
    task = reload(&store, &task)?;
    let chapter = packet(2);
    store.commit_pending_image_download_chapter(&task, &chapter, 0, 101)?;
    task = reload(&store, &task)?;
    drop(publish_imageless_chapter(
        &assets,
        &store,
        &chapter.id,
        "Different text that is absent from the durable chapter.",
    )?);
    let proof = assets.verify_complete_book(&store, BOOK, &[chapter.id.as_str()])?;
    let states = store.prepare_download_chapter_states_with_verified_images(&task, proof, 102)?;
    assert_eq!(states[0].status, DownloadChapterStatus::Pending);
    task = reload(&store, &task)?;
    assert_eq!(task.completed_count, 0);
    assert_eq!(task.next_index, 0);
    assert_eq!(store.chapter_body(BOOK, 2)?, chapter.body);

    drop(publish_imageless_chapter(
        &assets,
        &store,
        &chapter.id,
        chapter.body.as_deref().unwrap(),
    )?);
    let proof = assets.verify_complete_book(&store, BOOK, &[chapter.id.as_str()])?;
    let states = store.prepare_download_chapter_states_with_verified_images(&task, proof, 103)?;
    assert_eq!(states[0].status, DownloadChapterStatus::Complete);
    assert_eq!(store.task(&task.id)?.unwrap().completed_count, 1);
    assert_eq!(store.chapter_body(BOOK, 2)?, chapter.body);
    Ok(())
}

#[test]
fn a_disappeared_catalog_uid_blocks_only_its_original_chapter_state() -> Result<()> {
    let (store, mut task) = fixture()?;
    store.connection.execute(
        "UPDATE chapters SET source_id='replacement-uid-0' WHERE book_id=?1 AND chapter_index=0",
        [BOOK],
    )?;
    let preflight = store
        .validate_independent_download_chapter(&task, &packet(0), 0)
        .unwrap_err();
    assert!(preflight.downcast_ref::<ContentConflict>().is_some());
    let commit = store
        .commit_independent_download_chapter(&task, &packet(0), 0, 101)
        .unwrap_err();
    assert!(commit.downcast_ref::<ContentConflict>().is_some());
    assert_eq!(store.task(&task.id)?.unwrap(), task);
    assert_eq!(store.chapter_body(BOOK, 0)?, None);

    assert_eq!(
        store.record_download_chapter_failure(
            &task,
            "independent-uid-0",
            0,
            0,
            false,
            "source.content_changed",
            101,
        )?,
        DownloadChapterRetry::Blocked,
    );
    task = reload(&store, &task)?;
    store.commit_independent_download_chapter(&task, &packet(2), 2, 102)?;
    task = reload(&store, &task)?;
    assert_eq!(task.status, DownloadStatus::Running);
    assert_eq!(task.completed_count, 1);
    assert_eq!(task.next_index, 0);
    let states = store.download_chapter_states(&task.id)?;
    assert_eq!(states[0].chapter_uid, "independent-uid-0");
    assert_eq!(states[0].status, DownloadChapterStatus::Blocked);
    assert_eq!(states[0].error.as_deref(), Some("source.content_changed"));
    assert_eq!(states[2].status, DownloadChapterStatus::Complete);
    let replacement = store.chapter(BOOK, 0)?.unwrap();
    assert_eq!(replacement.id, "replacement-uid-0");
    assert_eq!(replacement.body, None);
    assert_eq!(store.chapter_body(BOOK, 2)?, packet(2).body);
    assert_eq!(store.book(BOOK)?.unwrap().cached_chapters, 1);
    Ok(())
}

#[test]
fn running_retry_resets_exactly_the_blocked_ordinals_without_restarting_other_chapters()
-> Result<()> {
    let (store, mut task) = running_mixed_chapter_fixture()?;
    let before = store.download_chapter_states(&task.id)?;
    assert_eq!(before[0].status, DownloadChapterStatus::Complete);
    assert_eq!(before[1].status, DownloadChapterStatus::RetryWait);
    assert_eq!(before[1].retry_not_before, 585);
    assert_eq!(before[2].status, DownloadChapterStatus::Pending);
    assert_eq!(before[3].status, DownloadChapterStatus::Blocked);
    assert_eq!(before[3].retry_count, 4);
    assert_eq!(before[4].status, DownloadChapterStatus::Blocked);
    assert_eq!(before[4].retry_count, 1);
    let body_reads = store.chapter_body_read_count();
    let body_bytes = store.materialized_chapter_bytes();

    let (progress, mut ordinals) = store.resume_blocked_download_chapters(&task, 526)?;
    ordinals.sort_unstable();
    assert_eq!(ordinals, vec![3, 4]);
    assert_eq!(progress.completed_count, 1);
    assert_eq!(progress.next_index, 1);
    assert_eq!(progress.pending_count, 3);
    assert_eq!(progress.retry_wait_count, 1);
    assert_eq!(progress.blocked_count, 0);
    assert_eq!(progress.retry_not_before, Some(585));
    let after = store.download_chapter_states(&task.id)?;
    assert_eq!(&after[..3], &before[..3]);
    for ordinal in [3, 4] {
        assert_eq!(after[ordinal].chapter_uid, before[ordinal].chapter_uid);
        assert_eq!(after[ordinal].chapter_index, before[ordinal].chapter_index);
        assert_eq!(after[ordinal].ordinal, ordinal);
        assert_eq!(after[ordinal].status, DownloadChapterStatus::Pending);
        assert_eq!(after[ordinal].retry_count, 0);
        assert_eq!(after[ordinal].retry_not_before, 0);
        assert_eq!(after[ordinal].error, None);
    }
    assert_eq!(store.chapter_body_read_count(), body_reads);
    assert_eq!(store.materialized_chapter_bytes(), body_bytes);
    task = reload(&store, &task)?;
    assert_eq!(task.status, DownloadStatus::Running);
    assert_eq!(task.updated_at, 526);
    assert_eq!(task.completed_count, 1);
    assert_eq!(task.next_index, 1);
    assert_eq!(task.chapters, vec![0, 1, 2, 3, 4]);

    store.commit_independent_download_chapter(&task, &packet(2), 2, 527)?;
    let continued = store.download_chapter_states(&task.id)?;
    assert_eq!(continued[0], before[0]);
    assert_eq!(continued[1], before[1]);
    assert_eq!(continued[2].status, DownloadChapterStatus::Complete);
    assert_eq!(store.task(&task.id)?.unwrap().completed_count, 2);
    Ok(())
}

#[test]
fn running_retry_rejects_stale_scope_and_rolls_back_all_resets_on_write_failure() -> Result<()> {
    let (store, task) = running_mixed_chapter_fixture()?;
    let before = store.download_chapter_states(&task.id)?;
    let body_reads = store.chapter_body_read_count();
    let body_bytes = store.materialized_chapter_bytes();
    for mutation in ["account", "plan", "status"] {
        let mut stale = task.clone();
        match mutation {
            "account" => stale.account_key = "synthetic-other-owner".into(),
            "plan" => stale.chapters.swap(0, 4),
            "status" => stale.status = DownloadStatus::Paused,
            _ => unreachable!(),
        }
        assert!(
            store.resume_blocked_download_chapters(&stale, 526).is_err(),
            "A running chapter retry must reject stale {mutation}",
        );
        assert_eq!(store.task(&task.id)?.unwrap(), task);
        assert_eq!(store.download_chapter_states(&task.id)?, before);
    }
    let mut paused = task.clone();
    paused.status = DownloadStatus::Paused;
    store.save_task(&paused)?;
    assert!(
        store
            .resume_blocked_download_chapters(&paused, 526)
            .is_err()
    );
    assert_eq!(store.task(&task.id)?.unwrap(), paused);
    assert_eq!(store.download_chapter_states(&task.id)?, before);
    store.save_task(&task)?;
    store.connection.execute_batch(
        "CREATE TEMP TRIGGER reject_running_chapter_retry BEFORE UPDATE ON download_tasks
         BEGIN SELECT RAISE(ABORT,'synthetic running retry checkpoint failure'); END",
    )?;
    assert!(store.resume_blocked_download_chapters(&task, 526).is_err());
    assert_eq!(store.task(&task.id)?.unwrap(), task);
    assert_eq!(store.download_chapter_states(&task.id)?, before);
    assert_eq!(store.chapter_body_read_count(), body_reads);
    assert_eq!(store.materialized_chapter_bytes(), body_bytes);
    store
        .connection
        .execute_batch("DROP TRIGGER reject_running_chapter_retry")?;
    let (_, mut ordinals) = store.resume_blocked_download_chapters(&task, 526)?;
    ordinals.sort_unstable();
    assert_eq!(ordinals, vec![3, 4]);
    Ok(())
}
