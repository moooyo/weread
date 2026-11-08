//! Genuine Reader entry points for image-anchor and catalog-weight work.

use super::super::underline_projection_tests::{BOOK, CHAPTER, Fixture, isolated};
use super::*;
use crate::reader::{image_anchor_work, reset_image_anchor_work};

fn remote(uid: &str, index: u32, words: u32) -> source::RemoteChapter {
    source::RemoteChapter {
        uid: uid.into(),
        index,
        word_count: words,
        title: format!("Chapter {index}"),
        ..Default::default()
    }
}

fn header(uid: &str, index: u32) -> Chapter {
    Chapter {
        id: uid.into(),
        book_id: BOOK.into(),
        index,
        title: format!("Chapter {index}"),
        ..Default::default()
    }
}

fn reset_weight_work() {
    BOOK_WEIGHT_WORK.with(|work| work.set(BookWeightWork::default()));
}

fn weight_work() -> BookWeightWork {
    BOOK_WEIGHT_WORK.with(Cell::get)
}

struct WeightAllowance(usize);

impl WeightAllowance {
    fn new(maximum: usize) -> Self {
        Self(BOOK_WEIGHT_ALLOWANCE.with(|allowance| allowance.replace(maximum)))
    }
}

impl Drop for WeightAllowance {
    fn drop(&mut self) {
        BOOK_WEIGHT_ALLOWANCE.with(|allowance| allowance.set(self.0));
    }
}

#[test]
fn borrowed_weights_keep_first_uid_match_and_local_chapter_order() {
    let chapters = [header("a", 10), header("b", 20), header("c", 30)];
    let catalog = [
        remote("b", 100, 30),
        remote("a", 101, 10),
        remote("b", 102, 200),
        remote("c", 103, 60),
    ];
    reset_weight_work();
    assert_eq!(
        weighted_book_fraction(&chapters, &catalog, 20, 10, 20),
        Some(0.25)
    );
    assert_eq!(
        weighted_book_fraction(&chapters, &catalog, 20, 100, 20),
        Some(0.4)
    );
    assert_eq!(
        weighted_book_fraction(&chapters, &catalog, 20, 100, 0),
        Some(0.1)
    );
    let work = weight_work();
    assert_eq!(work.builds, 3);
    assert_eq!(work.uid_inserts, catalog.len() * 3);
    assert_eq!(work.chapter_lookups, chapters.len() * 3);
}

#[test]
fn missing_uids_zero_totals_and_large_word_counts_keep_previous_fraction_rules() {
    let chapters = [header("a", 10), header("b", 20), header("c", 30)];
    assert_eq!(
        weighted_book_fraction(&chapters, &[remote("a", 0, 1)], 20, 0, 10),
        None,
    );
    let zero = [remote("a", 0, 0), remote("b", 1, 0), remote("c", 2, 0)];
    assert_eq!(weighted_book_fraction(&chapters, &zero, 20, 0, 10), None);
    let large = [
        remote("a", 0, u32::MAX),
        remote("b", 1, u32::MAX),
        remote("c", 2, u32::MAX),
    ];
    assert_eq!(
        weighted_book_fraction(&chapters, &large, 20, 5, 10),
        Some(0.5)
    );
    reset_weight_work();
    assert_eq!(weighted_book_fraction(&chapters, &large, 99, 0, 10), None);
    assert_eq!(weight_work(), BookWeightWork::default());
}

fn html_manifest_reader() -> Result<()> {
    let body = format!("{}Tail.", "\u{200b}".repeat(96));
    let html = format!(
        "<body>{}<p>{body}</p></body>",
        "<img src='images/shared.png'>".repeat(16),
    );
    assert_eq!(source::content::html_to_text(&html), body);
    let fixture = Fixture::new(&body, Vec::new(), false)?;
    {
        let mut state = fixture.app();
        let prepared = crate::book_assets::prepare_chapter_metadata(&html, &body)?;
        let manifest = state.book_assets.commit_prepared_chapter_metadata(
            &state.store,
            BOOK,
            CHAPTER,
            &html,
            &body,
            &prepared,
            None,
        )?;
        assert_eq!(manifest.images.len(), 16);
        assert!(manifest.images.iter().all(|image| image.anchor == Some(0)));
        let original_ids = manifest
            .images
            .iter()
            .map(|image| image.id.clone())
            .collect::<Vec<_>>();
        reset_image_anchor_work();
        for _ in 0..8 {
            state.reflow_reader()?;
        }
        let work = image_anchor_work();
        assert_eq!(work.projections, 8);
        assert_eq!(work.images, 16 * 8);
        assert_eq!(work.scalar_checks, 97 * 8);
        assert!(work.scalar_checks <= (body.chars().count() + 16) * 8);
        let paginator = state.reader.paginator.as_ref().unwrap();
        let placed = paginator
            .pages
            .iter()
            .flat_map(|page| &page.images)
            .collect::<Vec<_>>();
        assert_eq!(placed.len(), original_ids.len());
        assert_eq!(
            placed.iter().map(|image| &image.id).collect::<Vec<_>>(),
            original_ids.iter().collect::<Vec<_>>()
        );
        assert!(placed.iter().all(|image| image.anchor == 96));
        assert_eq!(paginator.char_count as usize, body.chars().count());
        assert_eq!(state.reader.body, body);
        state.render_reader()?;
        assert_eq!(image_anchor_work(), work);
    }
    fixture.finish()
}

#[test]
fn genuine_html_manifest_reader_reflows_share_each_zero_width_run_scan() -> Result<()> {
    isolated(
        "app::reading::work_tests::genuine_html_manifest_reader_reflows_share_each_zero_width_run_scan",
        html_manifest_reader,
    )
}

fn prepare_weighted_reader(state: &mut App, count: usize) -> Result<()> {
    let mut book = state.store.book(BOOK)?.unwrap();
    book.source = "weread".into();
    book.total_chapters = count as u32;
    state.store.upsert_book(&book)?;
    state.demo = false;
    state.prefs.pull_on_open = false;
    state.jobs.replace_session(source::AuthSession {
        user_vid: "synthetic-weight-account".into(),
        cookies: BTreeMap::from([
            ("wr_vid".into(), "synthetic-weight-account".into()),
            ("wr_skey".into(), "synthetic-weight-session".into()),
        ]),
        ..Default::default()
    })?;
    let owner = state.chapter_content_owner().unwrap();
    let mut catalog = Vec::new();
    for index in 0..count {
        let uid = format!("{}-{index:02}", "shared-weight-prefix-".repeat(3));
        let mut chapter = header(&uid, index as u32);
        chapter.cached = true;
        chapter.content_status = ContentStatus::Full;
        chapter.content_account_key = owner.clone();
        chapter.body = Some(format!("Body for chapter {index}."));
        state.store.upsert_chapter(&chapter)?;
        catalog.push(remote(&uid, index as u32, index as u32 + 1));
    }
    source::content::validate_catalog(&catalog)?;
    state
        .store
        .set_setting(&format!("catalog.remote:{BOOK}"), &catalog)?;
    state.load_local_reader(BOOK, (count / 2) as u32, Some(0))?;
    state.reader.selection = Some((0, 4));
    assert!(state.reader.draft.set_query("Keep weighted progress draft"));
    Ok(())
}

fn normal_weighted_progress() -> Result<()> {
    let fixture = Fixture::new("Initial body.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        prepare_weighted_reader(&mut state, 24)?;
        let body = state.reader.body.clone();
        let draft = state.reader.draft.query.clone();
        let account = state.jobs.session();
        reset_weight_work();
        state.render_reader()?;
        let expected = BookWeightWork {
            builds: 1,
            preflight_records: 24,
            uid_inserts: 24,
            chapter_lookups: 24,
            rejections: 0,
        };
        assert_eq!(weight_work(), expected);
        let progress = fixture.ui().global::<ReaderModel>().get_progress();
        assert!((progress - 78.0 / 300.0).abs() < 0.000_001);
        reset_weight_work();
        state.persist_reader_position()?;
        assert_eq!(weight_work(), expected);
        assert_eq!(state.store.position(BOOK)?.unwrap().chapter_index, 12);
        assert_eq!(state.reader.body, body);
        assert_eq!(state.reader.draft.query, draft);
        assert_eq!(state.reader.selection, Some((0, 4)));
        assert_eq!(state.jobs.session(), account);
    }
    fixture.finish()
}

#[test]
fn ordinary_reader_render_and_persistence_build_one_borrowed_weight_projection() -> Result<()> {
    isolated(
        "app::reading::work_tests::ordinary_reader_render_and_persistence_build_one_borrowed_weight_projection",
        normal_weighted_progress,
    )
}

fn rejected_weight_projection() -> Result<()> {
    let fixture = Fixture::new("Initial body.", Vec::new(), false)?;
    {
        let mut state = fixture.app();
        prepare_weighted_reader(&mut state, 8)?;
        let body = state.reader.body.clone();
        let draft = state.reader.draft.query.clone();
        let account = state.jobs.session();
        let _allowance = WeightAllowance::new(1);
        reset_weight_work();
        state.render_reader()?;
        let work = weight_work();
        assert_eq!(work.preflight_records, 1);
        assert_eq!(work.rejections, 1);
        assert_eq!(work.builds, 0);
        assert_eq!(work.uid_inserts, 0);
        assert_eq!(work.chapter_lookups, 0);
        assert_eq!(fixture.ui().global::<ReaderModel>().get_progress(), 0.5);
        assert_eq!(state.reader.body, body);
        assert_eq!(state.reader.draft.query, draft);
        assert_eq!(state.reader.selection, Some((0, 4)));
        assert_eq!(state.jobs.session(), account);
        let expected_uid = format!("{}-04", "shared-weight-prefix-".repeat(3));
        assert_eq!(
            state.store.chapter_uid(BOOK, 4)?.as_deref(),
            Some(expected_uid.as_str())
        );
    }
    fixture.finish()
}

#[test]
fn a_reader_weight_fee_rejection_precedes_table_work_and_preserves_local_state() -> Result<()> {
    isolated(
        "app::reading::work_tests::a_reader_weight_fee_rejection_precedes_table_work_and_preserves_local_state",
        rejected_weight_projection,
    )
}
