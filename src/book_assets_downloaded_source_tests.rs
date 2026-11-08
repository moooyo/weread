//! Bounded borrowed downloaded-source projections through real ZIP acquisition and commits.

use super::*;
use crate::{
    domain::{Annotation, Bookmark},
    source::images::{self, zip_timestamp_budget_tests as archive_fixture},
};
use std::{cell::Cell, fs};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Work {
    indexes: usize,
    usage: DownloadedSourceUsage,
    key_normalizations: usize,
    query_normalizations: usize,
    normalized_bytes: usize,
    requests: usize,
    lookups: usize,
}

std::thread_local! {
    static LIMITS: Cell<Option<DownloadedSourceLimits>> = const { Cell::new(None) };
    static WORK: Cell<Work> = Cell::new(Work::default());
}

pub(super) fn configured_limits() -> Option<DownloadedSourceLimits> {
    LIMITS.with(Cell::get)
}

pub(super) fn observe_admission(usage: DownloadedSourceUsage) {
    WORK.with(|work| {
        let mut observed = work.get();
        observed.indexes += 1;
        observed.usage = usage;
        work.set(observed);
    });
}

pub(super) fn observe_normalization(query: bool, bytes: usize) {
    WORK.with(|work| {
        let mut observed = work.get();
        if query {
            observed.query_normalizations += 1;
        } else {
            observed.key_normalizations += 1;
        }
        observed.normalized_bytes += bytes;
        work.set(observed);
    });
}

pub(super) fn observe_projection(requests: usize) {
    WORK.with(|work| {
        let mut observed = work.get();
        observed.requests += requests;
        work.set(observed);
    });
}

pub(super) fn observe_lookup() {
    WORK.with(|work| {
        let mut observed = work.get();
        observed.lookups += 1;
        work.set(observed);
    });
}

fn reset_work() {
    WORK.with(|work| work.set(Work::default()));
}
fn work() -> Work {
    WORK.with(Cell::get)
}

struct LimitsGuard(Option<DownloadedSourceLimits>);

impl LimitsGuard {
    fn new(limits: DownloadedSourceLimits) -> Self {
        Self(LIMITS.with(|current| current.replace(Some(limits))))
    }
}

impl Drop for LimitsGuard {
    fn drop(&mut self) {
        LIMITS.with(|current| current.set(self.0));
    }
}

fn requests(sources: &[&str]) -> Vec<AnchoredImage> {
    sources
        .iter()
        .enumerate()
        .map(|(index, source)| AnchoredImage {
            id: format!("downloaded-request-{index}"),
            source: (*source).into(),
            anchor: Some(0),
            caption: String::new(),
        })
        .collect()
}

fn source_map(entries: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
    entries
        .iter()
        .map(|(source, ids)| {
            (
                (*source).to_owned(),
                ids.iter().map(|id| (*id).to_owned()).collect(),
            )
        })
        .collect()
}

fn html(images: &str) -> String {
    format!("<p>Before</p>{images}<p>After</p>")
}

fn acquire_prepare(
    html: &str,
    entries: &[(String, Vec<u8>)],
) -> Result<(PreparedChapterMetadata, PreparedChapterImages)> {
    let bytes = archive_fixture::archive_assets(entries);
    assert!(bytes.len() < 32 * 1024);
    let plan = images::make_plan(
        "book",
        "uid",
        html,
        Some("https://weread.qq.com/resources/fixture.zip"),
    )?;
    assert!(plan.remote_images.is_empty());
    let fetches = Cell::new(0usize);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let downloaded = runtime.block_on(archive_fixture::acquire_for_test(
        &plan,
        |request, maximum| {
            fetches.set(fetches.get() + 1);
            assert_eq!(request.url, "https://weread.qq.com/resources/fixture.zip");
            assert_eq!(maximum, images::MAX_ARCHIVE_BYTES);
            let bytes = bytes.clone();
            async move { Ok(bytes) }
        },
    ))?;
    assert_eq!(fetches.get(), 1);
    let metadata = prepare_chapter_metadata(html, tests::TEXT)?;
    let prepared = prepare_downloaded(downloaded)?;
    Ok((metadata, prepared))
}

fn add_personal_state(store: &Store) -> Result<()> {
    store.save_annotation(&Annotation {
        id: "downloaded-source-note".into(),
        book_id: "book".into(),
        chapter_index: 0,
        start: 0,
        end: 6,
        quote: "Before".into(),
        note: "Keep the source-index fixture note".into(),
        created_at: 1,
    })?;
    store.save_bookmark(&Bookmark {
        id: "downloaded-source-bookmark".into(),
        book_id: "book".into(),
        chapter_index: 0,
        char_offset: 1,
        label: "Keep the source-index marker".into(),
        created_at: 1,
    })?;
    Ok(())
}

fn assert_personal_state(store: &Store) -> Result<()> {
    assert_eq!(store.chapter_body("book", 0)?.as_deref(), Some(tests::TEXT));
    let notes = store.annotations("book")?;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].note, "Keep the source-index fixture note");
    let bookmarks = store.bookmarks("book")?;
    assert_eq!(bookmarks.len(), 1);
    assert_eq!(bookmarks[0].char_offset, 1);
    Ok(())
}

#[test]
fn actual_zip_acquisition_preparation_and_mixed_sources_use_one_borrowed_projection() -> Result<()>
{
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    add_personal_state(&store)?;
    let body = archive_fixture::raster(111)?;
    let names = ["images/a.png", "images/./a.png", "images/unused.png"];
    let entries = names
        .iter()
        .map(|name| ((*name).to_owned(), body.clone()))
        .collect::<Vec<_>>();
    let image_html = html(&format!(
        "<img src='images/a.png#one'><img src='images/./a.png'>{}",
        "<img data-original='images/./a.png#later'>".repeat(12)
    ));
    let (metadata, prepared) = acquire_prepare(&image_html, &entries)?;
    assert_eq!(metadata.anchors.len(), 14);
    assert_eq!(metadata.unresolved_images, 0);
    assert!(prepared.sources.contains_key("images/unused.png"));
    let keys = prepared.sources.len();
    reset_work();
    let manifest = assets.commit_prepared_chapter_metadata(
        &store,
        "book",
        "uid",
        &image_html,
        tests::TEXT,
        &metadata,
        Some(&prepared),
    )?;
    let observed = work();
    assert_eq!(observed.indexes, 1);
    assert_eq!(observed.key_normalizations, keys);
    assert_eq!(observed.query_normalizations, 1);
    assert_eq!(observed.requests, 3);
    assert_eq!(observed.lookups, 3 * metadata.anchors.len());
    assert!(manifest.complete);
    assert_eq!(manifest.images.len(), metadata.anchors.len());
    assert_eq!(assets.cache.keys()?.len(), 1);
    let digest = manifest.images[0].asset_id.as_ref().unwrap();
    assert!(
        manifest
            .images
            .iter()
            .all(|image| image.asset_id.as_ref() == Some(digest))
    );
    assert_eq!(
        manifest
            .images
            .iter()
            .map(|image| image.source.as_str())
            .collect::<Vec<_>>(),
        metadata
            .anchors
            .iter()
            .map(|image| image.source.as_str())
            .collect::<Vec<_>>()
    );
    assert_personal_state(&store)
}

#[test]
fn repeated_missing_lazy_sources_do_not_repeat_alias_scans_and_rollback_new_payloads() -> Result<()>
{
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    add_personal_state(&store)?;
    let body = archive_fixture::raster(121)?;
    let entries = (0..8)
        .map(|index| (format!("images/{index}/a.png"), body.clone()))
        .collect::<Vec<_>>();
    let image_html = html(&format!(
        "<img src='images/0/a.png'>{}",
        "<img data-original='missing/long-source.png'>".repeat(32)
    ));
    let (metadata, prepared) = acquire_prepare(&image_html, &entries)?;
    assert_eq!(metadata.anchors.len(), 33);
    assert_eq!(prepared.sources.len(), 8);
    reset_work();
    let error = assets
        .commit_prepared_chapter_metadata(
            &store,
            "book",
            "uid",
            &image_html,
            tests::TEXT,
            &metadata,
            Some(&prepared),
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("a referenced chapter image was not acquired"));
    let observed = work();
    assert_eq!(observed.indexes, 1);
    assert_eq!(observed.key_normalizations, 8);
    assert_eq!(observed.query_normalizations, 1);
    assert_eq!(observed.requests, 2);
    assert_eq!(observed.lookups, 2 * metadata.anchors.len() + 2);
    assert!(
        store
            .setting::<ChapterManifest>(&manifest_key("book", "uid")?)?
            .is_none()
    );
    assert!(assets.cache.keys()?.is_empty());
    assert!(assets.cache.pinned_keys()?.is_empty());
    assert_eq!(assets.usage()?, 0);
    assert_personal_state(&store)
}

#[test]
fn complete_downloaded_admission_refuses_before_normalization_lookup_or_cache_mutation()
-> Result<()> {
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    add_personal_state(&store)?;
    let body = archive_fixture::raster(131)?;
    let entries = vec![
        ("images/a.png".into(), body.clone()),
        ("images/./a.png".into(), body),
    ];
    let image_html = html(
        "<img src='images/a.png'><img data-original='images/./a.png#later'><img data-original='images/./a.png#later'>",
    );
    let (metadata, prepared) = acquire_prepare(&image_html, &entries)?;
    let usage = DownloadedSourceIndex::forecast(
        &prepared.sources,
        &metadata.anchors,
        metadata.scalar_count,
        DownloadedSourceLimits::default(),
    )?;
    let previous = assets.commit_prepared_chapter_metadata(
        &store,
        "book",
        "uid",
        &image_html,
        tests::TEXT,
        &metadata,
        Some(&prepared),
    )?;
    let key = previous.images[0].cache_key.as_ref().unwrap();
    let path = assets.cache.get_for_commit(key)?.unwrap();
    let damaged = vec![0; fs::metadata(&path)?.len() as usize];
    fs::write(&path, &damaged)?;
    let pins = assets.cache.pinned_keys()?;
    let keys = assets.cache.keys()?;
    let bytes = assets.usage()?;
    let dirty = assets.reconcile_required.get();
    for limits in [
        DownloadedSourceLimits {
            keys: usage.keys - 1,
            ..Default::default()
        },
        DownloadedSourceLimits {
            associations: usage.associations - 1,
            ..Default::default()
        },
        DownloadedSourceLimits {
            source_bytes: usage.source_bytes - 1,
            ..Default::default()
        },
        DownloadedSourceLimits {
            occurrences: metadata.anchors.len() - 1,
            ..Default::default()
        },
        DownloadedSourceLimits {
            retained_bytes: usage.retained_bytes - 1,
            ..Default::default()
        },
        DownloadedSourceLimits {
            work_bytes: usage.work_bytes - 1,
            ..Default::default()
        },
    ] {
        reset_work();
        let _limits = LimitsGuard::new(limits);
        let error = assets
            .commit_prepared_chapter_metadata(
                &store,
                "book",
                "uid",
                &image_html,
                tests::TEXT,
                &metadata,
                Some(&prepared),
            )
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<DownloadedSourceLimitExceeded>()
                .is_some()
        );
        assert_eq!(work(), Work::default());
        assert_eq!(
            assets.verification_stats.get(),
            crate::cache::VerificationStats::default()
        );
        assert_eq!(assets.cache.pinned_keys()?, pins);
        assert_eq!(assets.cache.keys()?, keys);
        assert_eq!(assets.usage()?, bytes);
        assert_eq!(assets.reconcile_required.get(), dirty);
        assert_eq!(
            store.setting::<ChapterManifest>(&manifest_key("book", "uid")?)?,
            Some(previous.clone())
        );
        assert_eq!(fs::read(&path)?, damaged);
        assert_personal_state(&store)?;
    }
    reset_work();
    let _limits = LimitsGuard::new(DownloadedSourceLimits {
        keys: usage.keys,
        associations: usage.associations,
        source_bytes: usage.source_bytes,
        occurrences: metadata.anchors.len(),
        retained_bytes: usage.retained_bytes,
        work_bytes: usage.work_bytes,
    });
    let manifest = assets.commit_prepared_chapter_metadata(
        &store,
        "book",
        "uid",
        &image_html,
        tests::TEXT,
        &metadata,
        Some(&prepared),
    )?;
    assert!(manifest.complete);
    assert_eq!(work().indexes, 1);
    assert_eq!(work().key_normalizations, prepared.sources.len());
    assert_personal_state(&store)
}

#[test]
fn exact_sources_win_and_unrequested_normalized_ambiguity_stays_logical_metadata() -> Result<()> {
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    let entries = vec![
        ("images/a.png".into(), archive_fixture::raster(141)?),
        ("images//a.png".into(), archive_fixture::raster(151)?),
    ];
    let image_html = html("<img src='images/a.png'><img hidden src='images//a.png'>");
    let (metadata, prepared) = acquire_prepare(&image_html, &entries)?;
    assert_eq!(prepared.assets.len(), 2);
    assert_eq!(metadata.anchors.len(), 1);
    reset_work();
    let manifest = assets.commit_prepared_chapter_metadata(
        &store,
        "book",
        "uid",
        &image_html,
        tests::TEXT,
        &metadata,
        Some(&prepared),
    )?;
    assert!(manifest.complete);
    assert_eq!(
        manifest.images[0].asset_id.as_deref(),
        prepared.sources["images/a.png"].first().map(String::as_str)
    );
    assert_eq!(work().key_normalizations, 2);
    assert_eq!(work().query_normalizations, 0);
    Ok(())
}

#[test]
fn shared_png_repair_survives_a_later_requested_normalized_ambiguity() -> Result<()> {
    let (_directory, store, assets) = tests::setup(8 * 1024 * 1024)?;
    add_personal_state(&store)?;
    let entries = vec![
        ("images/a.png".into(), archive_fixture::raster(161)?),
        ("images//a.png".into(), archive_fixture::raster(171)?),
    ];
    let original_html = html("<img src='images/a.png'><img hidden src='images//a.png'>");
    let (original_metadata, prepared) = acquire_prepare(&original_html, &entries)?;
    let original = assets.commit_prepared_chapter_metadata(
        &store,
        "book",
        "uid",
        &original_html,
        tests::TEXT,
        &original_metadata,
        Some(&prepared),
    )?;
    let key = original.images[0].cache_key.as_ref().unwrap();
    let path = assets.cache.get_for_commit(key)?.unwrap();
    let original_bytes = fs::read(&path)?;
    fs::write(&path, vec![0; original_bytes.len()])?;
    store.upsert_chapter(&crate::domain::Chapter {
        id: "failed-uid".into(),
        book_id: "book".into(),
        index: 1,
        body: Some(tests::TEXT.into()),
        ..Default::default()
    })?;
    let failing_html = html(
        "<img src='images/a.png'><img data-original='images/./a.png'><img hidden src='images//a.png'>",
    );
    let metadata = prepare_chapter_metadata(&failing_html, tests::TEXT)?;
    reset_work();
    let error = assets
        .commit_prepared_chapter_metadata(
            &store,
            "book",
            "failed-uid",
            &failing_html,
            tests::TEXT,
            &metadata,
            Some(&prepared),
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("ambiguous normalized chapter image source"));
    assert_eq!(work().indexes, 1);
    assert_eq!(work().key_normalizations, prepared.sources.len());
    assert_eq!(work().query_normalizations, 1);
    assert_eq!(fs::read(&path)?, original_bytes);
    assert_eq!(assets.cache.pinned_state(key)?, Some(true));
    assert!(assets.complete(&store, "book", "uid")?);
    assert_eq!(
        store.setting::<ChapterManifest>(&manifest_key("book", "uid")?)?,
        Some(original)
    );
    assert!(
        store
            .setting::<ChapterManifest>(&manifest_key("book", "failed-uid")?)?
            .is_none()
    );
    assert_personal_state(&store)
}

#[test]
fn borrowed_resolution_keeps_exact_empty_multidigest_query_and_scope_contracts() -> Result<()> {
    let sources = source_map(&[
        ("images/a.png", &["first"]),
        ("images//a.png", &["second"]),
        ("empty", &[]),
        ("multiple", &["first", "second"]),
        ("https://EXAMPLE.com/plot.png?x=1#old", &["query-one"]),
        ("https://example.com/plot.png?x=2", &["query-two"]),
        ("first-chapter/a.png", &["scoped"]),
    ]);
    let requested = requests(&[
        "images/a.png",
        "images/./a.png",
        "empty",
        "multiple",
        "https://example.com/plot.png?x=1#new",
        "https://example.com/plot.png?x=2",
        "second-chapter/a.png",
        "a.png",
    ]);
    reset_work();
    let index = DownloadedSourceIndex::new(&sources, &requested, 0)?;
    assert_eq!(index.id("images/a.png")?, Some("first"));
    assert!(
        format!("{:#}", index.id("images/./a.png").unwrap_err()).contains("ambiguous normalized")
    );
    assert!(format!("{:#}", index.id("empty").unwrap_err()).contains("ambiguous chapter"));
    assert!(format!("{:#}", index.id("multiple").unwrap_err()).contains("ambiguous chapter"));
    assert_eq!(
        index.id("https://example.com/plot.png?x=1#new")?,
        Some("query-one")
    );
    assert_eq!(
        index.id("https://example.com/plot.png?x=2")?,
        Some("query-two")
    );
    assert_eq!(index.id("second-chapter/a.png")?, None);
    assert_eq!(index.id("a.png")?, None);
    let before = work();
    for _ in 0..16 {
        assert_eq!(
            index.id("https://example.com/plot.png?x=1#new")?,
            Some("query-one")
        );
    }
    assert_eq!(work().key_normalizations, before.key_normalizations);
    assert_eq!(work().query_normalizations, before.query_normalizations);
    Ok(())
}

#[test]
fn downloaded_association_bytes_and_invalid_occurrence_slots_are_prepaid() -> Result<()> {
    let sources = source_map(&[("source", &["one", "two"])]);
    let requested = requests(&["source", "source"]);
    let usage = DownloadedSourceIndex::forecast(
        &sources,
        &requested,
        0,
        DownloadedSourceLimits::default(),
    )?;
    assert_eq!(usage.source_bytes, 2 * "source".len());
    assert_eq!(usage.associations, 2);
    for limits in [
        DownloadedSourceLimits {
            associations: 1,
            ..Default::default()
        },
        DownloadedSourceLimits {
            source_bytes: usage.source_bytes - 1,
            ..Default::default()
        },
        DownloadedSourceLimits {
            retained_bytes: usage.retained_bytes - 1,
            ..Default::default()
        },
        DownloadedSourceLimits {
            work_bytes: usage.work_bytes - 1,
            ..Default::default()
        },
    ] {
        reset_work();
        assert!(DownloadedSourceIndex::with_limits(&sources, &requested, 0, limits).is_err());
        assert_eq!(work(), Work::default());
    }
    let mut invalid = requested.clone();
    for anchor in &mut invalid {
        anchor.id.clear();
    }
    let invalid_usage =
        DownloadedSourceIndex::forecast(&sources, &invalid, 0, DownloadedSourceLimits::default())?;
    assert_eq!(invalid_usage.retained_bytes, usage.retained_bytes);
    reset_work();
    assert!(
        DownloadedSourceIndex::with_limits(
            &sources,
            &invalid,
            0,
            DownloadedSourceLimits {
                retained_bytes: invalid_usage.retained_bytes - 1,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert_eq!(work(), Work::default());
    Ok(())
}

#[test]
fn url_utf_expansion_is_admitted_and_duplicate_requests_share_one_normalization() -> Result<()> {
    let source = "//EXAMPLE.com/\u{4e2d}\u{1f642}.png?x=\u{4e2d}#old";
    let requested_source = "https://example.com/%E4%B8%AD%F0%9F%99%82.png?x=%E4%B8%AD#new";
    let sources = source_map(&[(source, &["utf"])]);
    let requested = requests(&[requested_source, requested_source, requested_source]);
    let usage = DownloadedSourceIndex::forecast(
        &sources,
        &requested,
        0,
        DownloadedSourceLimits::default(),
    )?;
    reset_work();
    let index = DownloadedSourceIndex::with_limits(
        &sources,
        &requested,
        0,
        DownloadedSourceLimits {
            retained_bytes: usage.retained_bytes,
            work_bytes: usage.work_bytes,
            ..Default::default()
        },
    )?;
    assert_eq!(index.id(requested_source)?, Some("utf"));
    assert_eq!(work().key_normalizations, 1);
    assert_eq!(work().query_normalizations, 1);
    assert!(work().normalized_bytes <= usage.retained_bytes);
    Ok(())
}
