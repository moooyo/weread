//! Small borrowed-map and genuine offline App catalog reindexing regressions.

use super::super::underline_projection_tests::{Fixture, isolated};
use super::*;

const BOOK: &str = "catalog-reindex-work-book";
const OTHER: &str = "catalog-reindex-other-book";
const ROWS: u32 = 24;
const RESIDENT: u32 = 3;

fn reset_counters() {
    CATALOG_REINDEX_COUNTERS.with(|slot| slot.set(CatalogReindexCounters::default()));
}

fn counters() -> CatalogReindexCounters {
    CATALOG_REINDEX_COUNTERS.with(Cell::get)
}

struct MetadataAllowance(usize);

impl MetadataAllowance {
    fn new(bytes: usize) -> Self {
        Self(CATALOG_REINDEX_METADATA_ALLOWANCE.with(|slot| slot.replace(bytes)))
    }
}

impl Drop for MetadataAllowance {
    fn drop(&mut self) {
        CATALOG_REINDEX_METADATA_ALLOWANCE.with(|slot| slot.set(self.0));
    }
}

fn uid(index: u32) -> String {
    format!("{}-{index:03}", "catalog-shared-prefix-".repeat(12))
}

fn chapter(index: u32, original: u32) -> Chapter {
    Chapter {
        id: uid(original),
        book_id: BOOK.into(),
        index,
        ..Default::default()
    }
}

fn old_rows(indices: &[u32]) -> BTreeMap<String, Chapter> {
    indices
        .iter()
        .map(|index| {
            let row = chapter(*index, *index);
            (row.id.clone(), row)
        })
        .collect()
}

fn mapped_rows(order: &[u32]) -> Vec<Chapter> {
    order
        .iter()
        .enumerate()
        .map(|(index, original)| chapter(index as u32, *original))
        .collect()
}

fn mapped_index(order: &[u32], original: u32) -> Option<u32> {
    order
        .iter()
        .position(|candidate| *candidate == original)
        .map(|index| index as u32)
}

#[test]
fn borrowed_catalog_mapping_preserves_reordering_removals_and_owned_sources() -> Result<()> {
    let old = old_rows(&[0, 1, 2, 3]);
    let new = mapped_rows(&[3, 10, 1]);
    reset_counters();
    let mapping = CatalogReindex::new(&old, &new)?;
    let retained = mapping.retained_chapter(3).unwrap();
    assert!(std::ptr::eq(retained, &new[0]));
    assert_eq!(retained.id.as_ptr(), new[0].id.as_ptr());
    assert_eq!(mapping.reindex(1), Some(2));
    assert_eq!(mapping.reindex(0), None);
    assert_eq!(mapping.reindex(2), None);
    assert_eq!(mapping.reindex(99), None);
    assert_eq!(old[&uid(3)].index, 3);
    assert_eq!(new[1].id, uid(10));
    assert_eq!(counters().builds, 1);
    assert_eq!(counters().new_uid_visits, new.len());
    assert_eq!(counters().old_uid_visits, old.len());
    assert_eq!(counters().uid_lookups, old.len());
    Ok(())
}

#[test]
fn repeated_catalog_index_lookups_never_rescan_or_copy_uid_fields() -> Result<()> {
    let originals = (0..32).collect::<Vec<_>>();
    let old = old_rows(&originals);
    let order = originals.iter().rev().copied().collect::<Vec<_>>();
    let new = mapped_rows(&order);
    reset_counters();
    let mapping = CatalogReindex::new(&old, &new)?;
    let initial = counters();
    for _ in 0..16 {
        for index in &originals {
            assert_eq!(mapping.reindex(*index), Some(31 - *index));
        }
    }
    let after = counters();
    assert_eq!(after.builds, 1);
    assert_eq!(after.new_uid_visits, initial.new_uid_visits);
    assert_eq!(after.old_uid_visits, initial.old_uid_visits);
    assert_eq!(after.uid_lookups, initial.uid_lookups);
    assert_eq!(after.index_lookups - initial.index_lookups, 16 * old.len());
    assert_eq!(after.new_index_lookups, 0);
    assert_eq!(new[0].id, uid(31));
    Ok(())
}

#[test]
fn an_empty_old_catalog_keeps_pending_fallback_without_inventing_reader_identity() -> Result<()> {
    let new = mapped_rows(&[9, 8, 7]);
    let old = BTreeMap::new();
    reset_counters();
    let mapping = CatalogReindex::new(&old, &new)?;
    assert_eq!(mapping.reindex(0), Some(0));
    assert_eq!(mapping.reindex(2), Some(2));
    assert_eq!(mapping.reindex(3), None);
    assert!(mapping.retained_chapter(0).is_none());
    let mut history = vec![
        Route::Reader(BOOK.into(), 0),
        Route::Reader(OTHER.into(), 0),
    ];
    remap_reader_history(&mut history, BOOK, &mapping);
    assert_eq!(
        history,
        [Route::Detail(BOOK.into()), Route::Reader(OTHER.into(), 0)]
    );
    assert_eq!(counters().uid_lookups, 0);
    assert_eq!(counters().new_uid_visits, new.len());
    assert_eq!(counters().old_uid_visits, 0);
    Ok(())
}

#[test]
fn catalog_map_count_and_metadata_admission_precede_any_table_allocation() -> Result<()> {
    reset_counters();
    for (old_count, new_count) in [
        (MAX_CATALOG_REINDEX_ENTRIES + 1, 0),
        (0, MAX_CATALOG_REINDEX_ENTRIES + 1),
        (usize::MAX, 1),
    ] {
        let error = catalog_reindex_metadata_bytes(old_count, new_count).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<source::SourceError>(),
            Some(source::SourceError::ResponseTooLarge)
        ));
    }
    assert_eq!(catalog_reindex_metadata_bytes(0, 0)?, 0);
    assert_eq!(
        catalog_reindex_metadata_bytes(MAX_CATALOG_REINDEX_ENTRIES, MAX_CATALOG_REINDEX_ENTRIES)?,
        MAX_CATALOG_REINDEX_METADATA_BYTES
    );
    let old = old_rows(&[0, 1]);
    let new = mapped_rows(&[1, 0, 8]);
    let fee = catalog_reindex_metadata_bytes(old.len(), new.len())?;
    {
        let _allowance = MetadataAllowance::new(fee - 1);
        let error = CatalogReindex::new(&old, &new).err().unwrap();
        assert!(matches!(
            error.downcast_ref::<source::SourceError>(),
            Some(source::SourceError::ResponseTooLarge)
        ));
        assert_eq!(counters(), CatalogReindexCounters::default());
    }
    let _allowance = MetadataAllowance::new(fee);
    assert_eq!(CatalogReindex::new(&old, &new)?.reindex(1), Some(0));
    assert_eq!(counters().builds, 1);
    assert_eq!(counters().uid_lookups, old.len());
    Ok(())
}

fn configure(state: &mut App) -> Result<String> {
    state.demo = false;
    state.online = true;
    state.force_offline = true;
    state.reader.session_started = None;
    state.prefs.report_time = false;
    state.prefs.push_on_close = false;
    state.prefs.pull_on_open = false;
    state.prefs.keep_awake = false;
    state.jobs.replace_session(source::AuthSession {
        user_vid: "synthetic-catalog-reindex-owner".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "synthetic-catalog-reindex-owner".into()),
            ("wr_skey".into(), "synthetic-catalog-reindex-session".into()),
        ]),
        ..Default::default()
    })?;
    Ok(state
        .jobs
        .session()
        .as_ref()
        .and_then(super::super::library::download_account_key)
        .unwrap())
}

fn seed(state: &mut App, owner: &str) -> Result<()> {
    let mut book = Book::new(BOOK, "Catalog reindex work fixture");
    book.source = "weread".into();
    book.total_chapters = ROWS;
    state.store.upsert_book(&book)?;
    for index in 0..ROWS {
        let status = match index % 3 {
            0 => ContentStatus::Full,
            1 => ContentStatus::Preview,
            _ => ContentStatus::Unknown,
        };
        state.store.upsert_chapter(&Chapter {
            id: uid(index),
            book_id: BOOK.into(),
            index,
            title: format!("Original title {index}"),
            cached: status == ContentStatus::Full,
            body: (status != ContentStatus::Unknown)
                .then(|| format!("Original source body {index}. Keep its local text.")),
            content_status: status,
            content_account_key: if status == ContentStatus::Unknown {
                String::new()
            } else {
                owner.into()
            },
        })?;
    }
    Ok(())
}

fn remote(order: &[u32]) -> Vec<source::RemoteChapter> {
    order
        .iter()
        .map(|original| source::RemoteChapter {
            uid: uid(*original),
            title: format!("Refreshed title {original}"),
            word_count: 16,
            ..Default::default()
        })
        .collect()
}

fn catalog_event(state: &mut App, rows: Vec<source::RemoteChapter>) -> JobEvent {
    let request = state.request();
    state.catalog_requests.insert(BOOK.into(), request);
    JobEvent::new(
        request,
        state.jobs.account_epoch(),
        JobResult::Catalog {
            book_id: BOOK.into(),
            result: Ok(rows),
        },
    )
}

fn paused_task(state: &mut App, owner: &str) -> Result<DownloadTask> {
    let task = DownloadTask {
        id: "catalog-reindex-paused-task".into(),
        book_id: BOOK.into(),
        account_key: owner.into(),
        chapters: vec![RESIDENT, 1, 2, ROWS - 1],
        status: DownloadStatus::Paused,
        include_images: false,
        next_index: 1,
        updated_at: 17,
        ..Default::default()
    };
    state.store.save_task(&task)?;
    Ok(task)
}

fn history_fixture() -> Vec<Route> {
    (0..64)
        .map(|index| match index % 4 {
            0 => Route::Reader(OTHER.into(), 1),
            1 => Route::Reader(BOOK.into(), 2),
            2 => Route::Reader(BOOK.into(), RESIDENT),
            _ => Route::Library,
        })
        .collect()
}

fn assert_retained_rows(state: &App, previous: &[Chapter], order: &[u32]) -> Result<()> {
    for (index, original) in order.iter().enumerate() {
        let saved = state.store.chapter(BOOK, index as u32)?.unwrap();
        assert_eq!(saved.id, uid(*original));
        if let Some(old) = previous.iter().find(|row| row.id == saved.id) {
            assert_eq!(saved.body, old.body);
            assert_eq!(saved.cached, old.cached);
            assert_eq!(saved.content_status, old.content_status);
            assert_eq!(saved.content_account_key, old.content_account_key);
        } else {
            assert!(saved.body.is_none());
            assert!(!saved.cached);
            assert_eq!(saved.content_status, ContentStatus::Unknown);
            assert!(saved.content_account_key.is_empty());
        }
    }
    Ok(())
}

fn run_selected_pending_and_hidden_reader() -> Result<()> {
    let fixture = Fixture::new("Initial offline fixture", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let owner = configure(&mut state)?;
        seed(&mut state, &owner)?;
        let previous = state.store.chapters(BOOK)?;
        let task = paused_task(&mut state, &owner)?;
        state.load_local_reader(BOOK, RESIDENT, None)?;
        state.reader.selection = Some((0, 8));
        state.reader.draft.set_query("Keep the hidden local draft");
        fixture
            .ui()
            .global::<ReaderModel>()
            .set_writing_thought(true);
        state.navigate(Route::Detail(BOOK.into()), true)?;
        state.selecting_chapters = true;
        let size = state.profile.detail_chapter_page_size();
        for page in 0..(ROWS as usize).div_ceil(size) {
            state.detail_page = page;
            state.detail_action("select-page", "")?;
        }
        let selected = state.selected_chapters.clone();
        assert_eq!(selected.len(), 16);
        state.detail_action("download", "")?;
        assert_eq!(
            state.pending_downloads,
            [(BOOK.into(), (0..ROWS).collect::<Vec<_>>())]
        );
        state.pending_downloads[0].1.extend([RESIDENT, RESIDENT]);
        state.pending_downloads.push((OTHER.into(), vec![7, 9]));
        let pending = state.pending_downloads[0].1.clone();
        state.pending_read = Some((BOOK.into(), 1));
        state.history = history_fixture();
        let body = state.reader.body.clone();
        let account = state.jobs.session();
        let reads = state.store.chapter_body_read_count();
        let whole_body_bytes = state.store.materialized_chapter_bytes();
        let mut order = vec![100];
        order.extend(
            (0..ROWS)
                .rev()
                .filter(|index| ![2, ROWS - 1].contains(index)),
        );
        reset_counters();
        let event = catalog_event(&mut state, remote(&order));
        assert!(state.handle_job(event)?);
        assert_eq!(state.store.chapter_body_read_count(), reads);
        assert_eq!(state.store.materialized_chapter_bytes(), whole_body_bytes);
        assert_eq!(state.jobs.session(), account);
        assert_eq!(state.reader.body, body);
        assert_eq!(state.reader.selection, Some((0, 8)));
        assert_eq!(state.reader.draft.query, "Keep the hidden local draft");
        assert_eq!(
            state.reader.chapter_index,
            mapped_index(&order, RESIDENT).unwrap()
        );
        assert_eq!(state.route, Route::Detail(BOOK.into()));
        assert_eq!(
            state.selected_chapters,
            selected
                .iter()
                .filter_map(|index| mapped_index(&order, *index))
                .collect::<BTreeSet<_>>()
        );
        assert_eq!(
            state.pending_downloads[0].1,
            pending
                .iter()
                .filter_map(|index| mapped_index(&order, *index))
                .collect::<Vec<_>>()
        );
        assert_eq!(state.pending_downloads[1], (OTHER.into(), vec![7, 9]));
        assert_eq!(
            state.pending_read,
            Some((BOOK.into(), mapped_index(&order, 1).unwrap()))
        );
        let expected_history = history_fixture()
            .into_iter()
            .map(|route| match route {
                Route::Reader(id, index) if id == BOOK => mapped_index(&order, index)
                    .map_or(Route::Detail(id.clone()), |index| Route::Reader(id, index)),
                other => other,
            })
            .collect::<Vec<_>>();
        assert_eq!(state.history, expected_history);
        let built = counters();
        assert_eq!(built.builds, 1);
        assert_eq!(built.new_uid_visits, order.len());
        assert_eq!(built.old_uid_visits, ROWS as usize);
        assert_eq!(built.uid_lookups, ROWS as usize);
        assert_eq!(
            built.index_lookups,
            1 + selected.len() + pending.len() + 1 + 32
        );
        assert_eq!(built.new_index_lookups, 1);
        let saved_task = state.store.task(&task.id)?.unwrap();
        assert_eq!(saved_task.account_key, owner);
        assert_eq!(
            saved_task.chapters,
            [
                mapped_index(&order, RESIDENT).unwrap(),
                mapped_index(&order, 1).unwrap()
            ]
        );
        assert_eq!(saved_task.next_index, 1);
        assert_eq!(saved_task.status, DownloadStatus::Paused);
        assert_eq!(saved_task.error.as_deref(), Some("catalog_changed"));
        assert_eq!(saved_task.updated_at, task.updated_at);
        assert_eq!(saved_task.include_images, task.include_images);
        assert_retained_rows(&state, &previous, &order)?;
        for _ in 0..4 {
            state.load_detail(BOOK)?;
            state.render_detail()?;
        }
        state.load_local_reader(BOOK, mapped_index(&order, RESIDENT).unwrap(), None)?;
        assert_eq!(counters(), built);
        assert_eq!(state.reader.body, body);
        assert_eq!(state.reader.draft.query, "Keep the hidden local draft");
        // A second real result gets one fresh map; ordinary rereads above did
        // not rebuild it or repeat a UID lookup for each retained selection.
        reset_counters();
        let event = catalog_event(&mut state, remote(&order));
        state.handle_job(event)?;
        assert_eq!(counters().builds, 1);
        assert_eq!(counters().new_uid_visits, order.len());
        assert_eq!(counters().old_uid_visits, order.len());
        assert_eq!(counters().uid_lookups, order.len());
        assert_eq!(state.reader.body, body);
        assert_eq!(state.reader.selection, Some((0, 8)));
        assert_eq!(state.jobs.session(), account);
        assert_eq!(state.store.task(&task.id)?.unwrap(), saved_task);
        assert_eq!(state.store.materialized_chapter_bytes(), whole_body_bytes);
        assert!(state.force_offline && !state.demo && state.events.try_recv().is_err());
    }
    fixture.finish()
}

fn run_active_reader_reordering_and_removal() -> Result<()> {
    let fixture = Fixture::new("Initial offline fixture", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let owner = configure(&mut state)?;
        seed(&mut state, &owner)?;
        state.load_local_reader(BOOK, RESIDENT, None)?;
        state.reader.selection = Some((0, 8));
        state.reader.draft.set_query("Keep active Reader draft");
        fixture
            .ui()
            .global::<ReaderModel>()
            .set_writing_thought(true);
        state.pending_read = Some((BOOK.into(), RESIDENT));
        state.history = vec![
            Route::Reader(BOOK.into(), RESIDENT),
            Route::Reader(OTHER.into(), 1),
        ];
        let body = state.reader.body.clone();
        let account = state.jobs.session();
        let whole_body_bytes = state.store.materialized_chapter_bytes();
        let order = [6, RESIDENT, 0, 1, 2, 4, 5];
        reset_counters();
        let event = catalog_event(&mut state, remote(&order));
        state.handle_job(event)?;
        assert_eq!(state.route, Route::Reader(BOOK.into(), 1));
        assert_eq!(state.reader.chapter_index, 1);
        assert_eq!(state.reader.body, body);
        assert_eq!(state.reader.selection, Some((0, 8)));
        assert_eq!(state.reader.draft.query, "Keep active Reader draft");
        assert!(state.pending_read.is_none());
        assert_eq!(state.store.position(BOOK)?.unwrap().chapter_index, 1);
        assert_eq!(
            state.history,
            [
                Route::Reader(BOOK.into(), 1),
                Route::Reader(OTHER.into(), 1)
            ]
        );
        assert_eq!(counters().builds, 1);
        assert_eq!(counters().uid_lookups, ROWS as usize);
        assert_eq!(state.store.materialized_chapter_bytes(), whole_body_bytes);
        let removed_order = [0, 6, 1];
        state.pending_read = Some((BOOK.into(), 1));
        reset_counters();
        let event = catalog_event(&mut state, remote(&removed_order));
        state.handle_job(event)?;
        assert_eq!(state.route, Route::Reader(BOOK.into(), 0));
        assert_eq!(state.reader.chapter_index, 0);
        let relocated = state.store.position(BOOK)?.unwrap();
        assert_eq!((relocated.chapter_index, relocated.char_offset), (0, 0));
        assert_eq!(
            state.reader.body,
            "Original source body 0. Keep its local text."
        );
        assert!(state.reader.selection.is_none());
        assert!(state.reader.draft.query.is_empty());
        assert!(state.pending_read.is_none());
        assert_eq!(
            state.history,
            [Route::Detail(BOOK.into()), Route::Reader(OTHER.into(), 1)]
        );
        assert_eq!(counters().builds, 1);
        assert_eq!(counters().new_uid_visits, removed_order.len());
        assert_eq!(counters().uid_lookups, order.len());
        assert_eq!(state.jobs.session(), account);
        assert_eq!(state.store.materialized_chapter_bytes(), whole_body_bytes);
        assert!(
            state
                .store
                .chapter_index_by_uid(BOOK, &uid(RESIDENT))?
                .is_none()
        );
        let full = state.store.chapter(BOOK, 0)?.unwrap();
        assert_eq!(full.content_status, ContentStatus::Full);
        assert_eq!(full.content_account_key, owner);
        let preview = state.store.chapter(BOOK, 2)?.unwrap();
        assert_eq!(preview.content_status, ContentStatus::Preview);
        assert!(!preview.cached);
        assert!(preview.body.is_some());
        assert_eq!(preview.content_account_key, owner);
        assert!(state.force_offline && state.events.try_recv().is_err());
    }
    fixture.finish()
}

fn run_stale_and_preflight_rejections() -> Result<()> {
    let fixture = Fixture::new("Initial offline fixture", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let owner = configure(&mut state)?;
        seed(&mut state, &owner)?;
        let task = paused_task(&mut state, &owner)?;
        state.load_local_reader(BOOK, RESIDENT, None)?;
        state.reader.draft.set_query("Keep rejected mapping draft");
        state.reader.selection = Some((0, 8));
        fixture
            .ui()
            .global::<ReaderModel>()
            .set_writing_thought(true);
        state.navigate(Route::Detail(BOOK.into()), true)?;
        state.selected_chapters = BTreeSet::from([1, 2, 4]);
        state.detail_action("download", "")?;
        let selected = state.selected_chapters.clone();
        let pending = state.pending_downloads.clone();
        let history = state.history.clone();
        let body = state.reader.body.clone();
        let account = state.jobs.session();
        let original = state.store.chapters(BOOK)?;
        let original_position = state.store.position(BOOK)?;
        let rows = remote(&[3, 0, 1]);
        state.catalog_requests.insert(BOOK.into(), 400);
        reset_counters();
        for (request, epoch) in [
            (399, state.jobs.account_epoch()),
            (400, state.jobs.account_epoch() + 1),
        ] {
            assert!(!state.handle_job(JobEvent::new(
                request,
                epoch,
                JobResult::Catalog {
                    book_id: BOOK.into(),
                    result: Ok(rows.clone())
                },
            ))?);
            assert_eq!(state.catalog_requests.get(BOOK), Some(&400));
            assert_eq!(state.pending_downloads, pending);
        }
        assert_eq!(counters(), CatalogReindexCounters::default());
        for duplicate in [true, false] {
            state.pending_read = Some((BOOK.into(), RESIDENT));
            state
                .pending_catalog_downloads
                .extend([BOOK.into(), OTHER.into()]);
            state.pending_catalog_images.insert(BOOK.into(), true);
            state.pending_catalog_images.insert(OTHER.into(), false);
            let payload = if duplicate {
                remote(&[3, 3])
            } else {
                rows.clone()
            };
            let allowance = catalog_reindex_metadata_bytes(ROWS as usize, payload.len())?;
            let _allowance =
                MetadataAllowance::new(if duplicate { allowance } else { allowance - 1 });
            let reads = state.store.chapter_body_read_count();
            let event = catalog_event(&mut state, payload);
            assert!(state.handle_job(event)?);
            assert_eq!(state.store.chapter_body_read_count(), reads);
            assert_eq!(counters(), CatalogReindexCounters::default());
            assert!(state.pending_read.is_none());
            assert!(!state.pending_catalog_downloads.contains(BOOK));
            assert!(!state.pending_catalog_images.contains_key(BOOK));
            assert!(state.pending_catalog_downloads.contains(OTHER));
            assert_eq!(state.pending_catalog_images.get(OTHER), Some(&false));
            assert_eq!(state.selected_chapters, selected);
            assert_eq!(state.pending_downloads, pending);
            assert_eq!(state.history, history);
            assert_eq!(state.reader.body, body);
            assert_eq!(state.reader.selection, Some((0, 8)));
            assert_eq!(state.reader.draft.query, "Keep rejected mapping draft");
            assert_eq!(state.jobs.session(), account);
            assert_eq!(state.store.task(&task.id)?.unwrap(), task);
            assert_eq!(state.store.chapters(BOOK)?, original);
            assert_eq!(state.store.position(BOOK)?, original_position);
            assert!(
                state
                    .store
                    .setting::<Vec<source::RemoteChapter>>(&format!("catalog.remote:{BOOK}"))?
                    .is_none()
            );
            assert_eq!(state.route, Route::Detail(BOOK.into()));
        }
        // A new scalar ownership read must retain an actual SQLite cause. The
        // injected table fault stays active through all in-memory assertions.
        let connection = rusqlite::Connection::open(state.data_dir.join("library.sqlite"))?;
        connection.execute_batch("ALTER TABLE chapters RENAME TO hidden_reindex_chapters")?;
        state.pending_read = Some((BOOK.into(), RESIDENT));
        state.pending_catalog_downloads.insert(BOOK.into());
        let event = catalog_event(&mut state, rows);
        let error = state.handle_job(event).unwrap_err();
        assert!(error.downcast_ref::<rusqlite::Error>().is_some());
        assert!(error.downcast_ref::<source::SourceError>().is_none());
        assert_eq!(counters(), CatalogReindexCounters::default());
        assert!(state.pending_read.is_none());
        assert!(!state.pending_catalog_downloads.contains(BOOK));
        assert_eq!(state.selected_chapters, selected);
        assert_eq!(state.pending_downloads, pending);
        assert_eq!(state.history, history);
        assert_eq!(state.reader.body, body);
        assert_eq!(state.reader.draft.query, "Keep rejected mapping draft");
        assert_eq!(state.jobs.session(), account);
        assert!(
            state
                .store
                .setting::<Vec<source::RemoteChapter>>(&format!("catalog.remote:{BOOK}"))?
                .is_none()
        );
        connection.execute_batch("ALTER TABLE hidden_reindex_chapters RENAME TO chapters")?;
        assert_eq!(state.store.task(&task.id)?.unwrap(), task);
        assert_eq!(state.store.chapters(BOOK)?, original);
        assert_eq!(state.store.position(BOOK)?, original_position);
        assert!(state.force_offline && state.events.try_recv().is_err());
    }
    fixture.finish()
}

#[test]
fn actual_catalog_result_reuses_one_map_for_selected_pending_and_hidden_reader() -> Result<()> {
    isolated(
        "app::backend::catalog_reindex_work_tests::actual_catalog_result_reuses_one_map_for_selected_pending_and_hidden_reader",
        run_selected_pending_and_hidden_reader,
    )
}

#[test]
fn actual_catalog_result_preserves_active_reader_identity_and_deleted_fallback() -> Result<()> {
    isolated(
        "app::backend::catalog_reindex_work_tests::actual_catalog_result_preserves_active_reader_identity_and_deleted_fallback",
        run_active_reader_reordering_and_removal,
    )
}

#[test]
fn actual_catalog_rejections_keep_original_state_without_building_partial_maps() -> Result<()> {
    isolated(
        "app::backend::catalog_reindex_work_tests::actual_catalog_rejections_keep_original_state_without_building_partial_maps",
        run_stale_and_preflight_rejections,
    )
}
