//! Catalog terminal failures settle only their own intents after known commits.

use super::underline_projection_tests::{BOOK, CHAPTER, Fixture, isolated};
use super::*;
use crate::jobs::JobResult;
use crate::platform::{ManagedSleepState, PlatformError, RenderSink};
use rusqlite::params;

const OTHER: &str = "catalog-settlement-other";
const SECOND: &str = "underline-fixture-chapter-1";

struct RunningSink;

impl RenderSink for RunningSink {
    fn set_dark_mode(&self, _: bool) -> Result<(), PlatformError> {
        Ok(())
    }
    fn managed_sleep_supported(&self) -> bool {
        true
    }
    fn managed_sleep_state(&self) -> Result<ManagedSleepState, PlatformError> {
        Ok(ManagedSleepState::Running)
    }
}

fn sql(state: &App) -> Result<rusqlite::Connection> {
    Ok(rusqlite::Connection::open(
        state.data_dir.join("library.sqlite"),
    )?)
}

fn seed(state: &mut App) -> Result<String> {
    state.jobs.replace_session(source::AuthSession {
        user_vid: "synthetic-catalog-settlement-account".into(),
        cookies: BTreeMap::from([
            (
                "wr_vid".into(),
                "synthetic-catalog-settlement-account".into(),
            ),
            (
                "wr_skey".into(),
                "synthetic-catalog-settlement-session".into(),
            ),
        ]),
        ..Default::default()
    })?;
    state.platform.set_render_sink(Arc::new(RunningSink))?;
    state
        .store
        .upsert_book(&Book::new(OTHER, "Other preserved catalog"))?;
    let mut book = state.store.book(BOOK)?.unwrap();
    book.intro = "Retained catalog introduction. ".repeat(2048);
    state.store.upsert_book(&book)?;
    state.store.save_annotation(&Annotation {
        id: "catalog-settlement-note".into(),
        book_id: BOOK.into(),
        start: 0,
        end: 4,
        quote: "Keep".into(),
        note: "Keep catalog note".into(),
        created_at: 1,
        ..Default::default()
    })?;
    state.store.save_bookmark(&Bookmark {
        id: "catalog-settlement-mark".into(),
        book_id: BOOK.into(),
        char_offset: 1,
        label: "Keep catalog anchor".into(),
        created_at: 1,
        ..Default::default()
    })?;
    state.reader.selection = Some((0, 4));
    assert!(state.reader.draft.set_query("Keep catalog draft"));
    state.pending_read = Some((BOOK.into(), 0));
    state
        .pending_catalog_downloads
        .extend([BOOK.into(), OTHER.into()]);
    state.pending_catalog_images.insert(BOOK.into(), true);
    state.pending_catalog_images.insert(OTHER.into(), false);
    state.catalog_requests.insert(BOOK.into(), 401);
    state.catalog_requests.insert(OTHER.into(), 402);
    state.prepare_resident_sleep()?;
    state.resume_resident_sleep()?;
    assert!(state.resident_lifecycle.retry_intents().0.contains(OTHER));
    assert!(state.resident_lifecycle.retry_intents().0.contains(BOOK));
    // Model an independently reissued producer after resident resumption.
    state.catalog_requests.insert(OTHER.into(), 402);
    state.route = Route::Library;
    state.history = vec![Route::Reader(BOOK.into(), 0), Route::Detail(OTHER.into())];
    state.pending_downloads = vec![(BOOK.into(), vec![0]), (OTHER.into(), vec![7])];
    state.detail_id = BOOK.into();
    state.selected_chapters.insert(0);
    Ok(state.chapter_content_owner().unwrap())
}

fn event(state: &mut App) -> JobEvent {
    let request = state.request();
    state.catalog_requests.insert(BOOK.into(), request);
    JobEvent::new(
        request,
        state.jobs.account_epoch(),
        JobResult::Catalog {
            book_id: BOOK.into(),
            result: Ok(vec![
                source::RemoteChapter {
                    uid: SECOND.into(),
                    title: "Second now first".into(),
                    word_count: 12,
                    ..Default::default()
                },
                source::RemoteChapter {
                    uid: CHAPTER.into(),
                    title: "Retained UID now second".into(),
                    word_count: 20,
                    ..Default::default()
                },
            ]),
        },
    )
}

fn assert_settled(state: &App) {
    assert!(state.pending_read.is_none());
    assert!(!state.catalog_requests.contains_key(BOOK));
    assert!(!state.pending_catalog_downloads.contains(BOOK));
    assert!(!state.pending_catalog_images.contains_key(BOOK));
    assert!(!state.resident_lifecycle.retry_intents().0.contains(BOOK));
    assert_eq!(state.catalog_requests.get(OTHER), Some(&402));
    assert!(state.pending_catalog_downloads.contains(OTHER));
    assert_eq!(state.pending_catalog_images.get(OTHER), Some(&false));
    assert!(state.resident_lifecycle.retry_intents().0.contains(OTHER));
}

fn cache_failure(update: bool) -> Result<()> {
    let fixture = Fixture::new("Keep catalog body", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let owner = seed(&mut state)?;
        if update {
            state.route = Route::Reader(BOOK.into(), 0);
        }
        let task = DownloadTask {
            id: "catalog-settlement-task".into(),
            book_id: BOOK.into(),
            account_key: owner,
            chapters: vec![0, 1],
            status: DownloadStatus::Pending,
            updated_at: 10,
            ..Default::default()
        };
        state.store.save_task(&task)?;
        state.downloads.push_back(task.clone());
        let cache_key = format!("catalog.remote:{BOOK}");
        if update {
            state.store.set_setting(
                &cache_key,
                &vec![source::RemoteChapter {
                    uid: CHAPTER.into(),
                    title: "Retained old cache".into(),
                    ..Default::default()
                }],
            )?;
        }
        let old_cache = state.store.setting::<serde_json::Value>(&cache_key)?;
        let old_book = state.store.book(BOOK)?.unwrap();
        let account = state.jobs.session();
        let connection = sql(&state)?;
        let trigger = if update { "UPDATE" } else { "INSERT" };
        connection.execute_batch(&format!(
            "CREATE TRIGGER catalog_cache_fault BEFORE {trigger} ON settings
             WHEN NEW.key='{cache_key}' BEGIN SELECT RAISE(ABORT,'synthetic catalog cache failure'); END;"))?;
        let result = event(&mut state);
        let error = state.handle_job(result).unwrap_err();
        assert!(error.downcast_ref::<rusqlite::Error>().is_some());
        assert!(
            error
                .downcast_ref::<crate::store::CachedResourceLimit>()
                .is_none()
        );
        assert_settled(&state);
        assert_eq!(state.store.chapter_uid(BOOK, 1)?.as_deref(), Some(CHAPTER));
        assert_eq!(
            state.store.chapter_body(BOOK, 1)?.as_deref(),
            Some("Keep catalog body")
        );
        let note = state
            .store
            .annotation_by_id(BOOK, "catalog-settlement-note")?
            .unwrap();
        assert_eq!((note.chapter_index, note.start, note.end), (1, 0, 4));
        assert_eq!(note.quote, "Keep");
        assert_eq!(note.note, "Keep catalog note");
        let mark = state.store.bookmarks(BOOK)?.remove(0);
        assert_eq!((mark.chapter_index, mark.char_offset), (1, 1));
        assert_eq!(state.reader.chapter_index, 1);
        if update {
            assert_eq!(state.route, Route::Reader(BOOK.into(), 1));
        }
        assert_eq!(state.reader.body, "Keep catalog body");
        assert_eq!(state.reader.selection, Some((0, 4)));
        assert_eq!(state.reader.draft.query, "Keep catalog draft");
        assert_eq!(
            state.history,
            vec![Route::Reader(BOOK.into(), 1), Route::Detail(OTHER.into())]
        );
        assert_eq!(
            state.pending_downloads,
            vec![(BOOK.into(), vec![1]), (OTHER.into(), vec![7])]
        );
        assert_eq!(state.selected_chapters, BTreeSet::from([1]));
        assert_eq!(state.store.task(&task.id)?.unwrap().chapters, vec![1, 0]);
        assert_eq!(state.downloads.front().unwrap().chapters, vec![1, 0]);
        assert_eq!(
            state.store.setting::<serde_json::Value>(&cache_key)?,
            old_cache
        );
        assert_eq!(state.store.book(BOOK)?.unwrap().intro, old_book.intro);
        assert_eq!(state.store.book(BOOK)?.unwrap().progress, old_book.progress);
        assert_eq!(state.jobs.session(), account);
        connection.execute_batch("DROP TRIGGER catalog_cache_fault")?;
    }
    fixture.finish()
}

fn inserted_cache_failure() -> Result<()> {
    cache_failure(false)
}
fn updated_cache_failure() -> Result<()> {
    cache_failure(true)
}

#[test]
fn cache_insert_error_after_catalog_commit_settles_own_waiters_and_preserves_other_intents()
-> Result<()> {
    isolated(
        "app::catalog_settlement_tests::cache_insert_error_after_catalog_commit_settles_own_waiters_and_preserves_other_intents",
        inserted_cache_failure,
    )
}

#[test]
fn cache_update_error_keeps_old_cache_and_committed_uid_body_task_and_anchors() -> Result<()> {
    isolated(
        "app::catalog_settlement_tests::cache_update_error_keeps_old_cache_and_committed_uid_body_task_and_anchors",
        updated_cache_failure,
    )
}

fn paused_task_failure() -> Result<()> {
    let fixture = Fixture::new("Keep catalog body", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let owner = seed(&mut state)?;
        let original = DownloadTask {
            id: "catalog-current-task".into(),
            book_id: BOOK.into(),
            account_key: owner,
            chapters: vec![0, 1],
            status: DownloadStatus::Running,
            updated_at: 10,
            ..Default::default()
        };
        state.store.save_task(&original)?;
        state.current_download = Some(original.clone());
        state.download_request = 777;
        let account = state.jobs.session();
        let connection = sql(&state)?;
        connection.execute_batch(
            "CREATE TRIGGER catalog_task_fault BEFORE INSERT ON download_tasks
             WHEN NEW.id='catalog-current-task' BEGIN SELECT RAISE(ABORT,'synthetic paused-task failure'); END;")?;
        let result = event(&mut state);
        let error = state.handle_job(result).unwrap_err();
        assert!(error.downcast_ref::<rusqlite::Error>().is_some());
        assert_settled(&state);
        assert_eq!(state.current_download.as_ref(), Some(&original));
        assert_eq!(state.store.task(&original.id)?, Some(original));
        assert_eq!(state.download_request, 777);
        assert!(state.downloads.is_empty());
        assert_eq!(state.store.chapter_uid(BOOK, 0)?.as_deref(), Some(CHAPTER));
        assert_eq!(state.reader.chapter_index, 0);
        assert_eq!(state.reader.body, "Keep catalog body");
        assert_eq!(state.reader.selection, Some((0, 4)));
        assert_eq!(state.reader.draft.query, "Keep catalog draft");
        assert_eq!(state.jobs.session(), account);
        connection.execute_batch("DROP TRIGGER catalog_task_fault")?;
        state.current_download = None;
        connection.execute(
            "DELETE FROM download_tasks WHERE id=?1",
            params!["catalog-current-task"],
        )?;
    }
    fixture.finish()
}

#[test]
fn paused_task_sql_failure_keeps_the_current_owner_before_cancellation() -> Result<()> {
    isolated(
        "app::catalog_settlement_tests::paused_task_sql_failure_keeps_the_current_owner_before_cancellation",
        paused_task_failure,
    )
}
