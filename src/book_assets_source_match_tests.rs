//! Small source projections and real text-only checkpoints preserve image ownership.

use super::*;
use crate::domain::{
    Annotation, Book, Bookmark, Chapter, ContentStatus, DownloadStatus, DownloadTask,
    ReadingPosition,
};

fn occurrence(source: &str, ordinal: usize) -> AnchoredImage {
    AnchoredImage {
        id: format!("occurrence-{ordinal}"),
        source: source.into(),
        anchor: Some(0),
        ..AnchoredImage::default()
    }
}

fn stored(source: &str, ordinal: usize, digest_digit: char) -> StoredImage {
    let asset_id = digest_digit.to_string().repeat(64);
    StoredImage {
        id: format!("stored-{ordinal}"),
        source: source.into(),
        cache_key: Some(asset_cache_key(&asset_id).unwrap()),
        asset_id: Some(asset_id),
        width: 8,
        height: 6,
        ..StoredImage::default()
    }
}

#[test]
fn repeated_long_sources_are_projected_once_without_cloning_old_occurrences() -> Result<()> {
    let source = format!("images/{}/a.png", "p".repeat(1800));
    let manifest = ChapterManifest {
        images: (0..64)
            .map(|ordinal| stored(&source, ordinal, 'a'))
            .collect(),
        ..ChapterManifest::default()
    };
    let anchors = (0..128)
        .map(|ordinal| occurrence(&source, ordinal))
        .collect::<Vec<_>>();
    let index = PreviousImageIndex::new(&manifest, &anchors)?;
    assert_eq!(index.stats.old_occurrences, 64);
    assert_eq!(index.stats.old_unique_sources, 1);
    assert_eq!(index.stats.old_normalizations, 1);
    assert_eq!(index.stats.new_unique_sources, 1);
    assert_eq!(index.stats.new_normalizations, 0);
    let stats = index.stats;
    for anchor in anchors.iter().chain(&anchors) {
        let image = index.image(&anchor.source)?.unwrap();
        assert!(std::ptr::eq(image, &manifest.images[0]));
    }
    assert_eq!(index.stats, stats);
    assert_eq!(manifest.images.len(), 64);
    Ok(())
}

#[test]
fn exact_priority_first_seen_aliases_and_deferred_ambiguity_keep_the_original_contract()
-> Result<()> {
    let mut placeholder = StoredImage {
        id: "placeholder".into(),
        source: "images/a.png".into(),
        ..StoredImage::default()
    };
    placeholder.caption = "An uncached occurrence is not an exact candidate".into();
    let manifest = ChapterManifest {
        images: vec![
            placeholder,
            stored("images/a.png", 1, 'a'),
            stored("images/./a.png", 2, 'b'),
            stored("./figures/b.png#old", 3, 'a'),
            stored("figures/b.png", 4, 'a'),
            stored("unused/c.png", 5, 'a'),
            stored("unused/c.png", 6, 'b'),
            stored("unrequested/d.png", 7, 'a'),
            stored("unrequested/d.png", 8, 'b'),
        ],
        ..ChapterManifest::default()
    };
    let anchors = [
        occurrence("images/a.png", 0),
        occurrence("./images/a.png#new", 1),
        occurrence("figures/./b.png#new", 2),
        occurrence("unused/c.png", 3),
        occurrence("missing.png", 4),
    ];
    // Neither an unrequested ambiguous old source nor an ambiguous projected
    // source rejects construction. Errors arise at their original lookup.
    let index = PreviousImageIndex::new(&manifest, &anchors)?;
    assert!(std::ptr::eq(
        index.image("images/a.png")?.unwrap(),
        &manifest.images[1],
    ));
    assert!(std::ptr::eq(
        index.image("figures/./b.png#new")?.unwrap(),
        &manifest.images[3],
    ));
    assert!(index.image("./images/a.png#new").is_err());
    assert!(index.image("unused/c.png").is_err());
    assert!(index.image("missing.png")?.is_none());
    Ok(())
}

#[test]
fn source_count_bytes_and_retained_forecasts_are_admitted_before_projection() -> Result<()> {
    let manifest = ChapterManifest {
        images: vec![
            stored("images/a.png", 0, 'a'),
            stored("images/a.png", 1, 'a'),
        ],
        ..ChapterManifest::default()
    };
    let anchors = [
        occurrence("./images/a.png", 0),
        occurrence("./images/a.png", 1),
    ];
    let admitted = PreviousImageIndex::new(&manifest, &anchors)?;
    let exact = SourceIndexLimits {
        occurrences: 2,
        source_bytes: admitted.stats.source_bytes,
        retained_bytes: admitted.stats.retained_bytes,
    };
    assert!(PreviousImageIndex::with_limits(&manifest, &anchors, exact).is_ok());
    for limits in [
        SourceIndexLimits {
            occurrences: 1,
            ..exact
        },
        SourceIndexLimits {
            source_bytes: exact.source_bytes - 1,
            ..exact
        },
        SourceIndexLimits {
            retained_bytes: exact.retained_bytes - 1,
            ..exact
        },
    ] {
        let error = PreviousImageIndex::with_limits(&manifest, &anchors, limits)
            .err()
            .expect("The source projection quota must reject this operation");
        assert!(error.downcast_ref::<SourceMatchLimitExceeded>().is_some());
    }
    let one_old = ChapterManifest {
        images: vec![manifest.images[0].clone()],
        ..ChapterManifest::default()
    };
    assert!(
        PreviousImageIndex::with_limits(
            &one_old,
            &anchors,
            SourceIndexLimits {
                occurrences: 1,
                ..exact
            },
        )
        .err()
        .unwrap()
        .downcast_ref::<SourceMatchLimitExceeded>()
        .is_some()
    );
    let oversized = ChapterManifest {
        images: vec![stored(&"p".repeat(MAX_SOURCE_BYTES + 1), 0, 'a')],
        ..ChapterManifest::default()
    };
    assert!(
        PreviousImageIndex::new(&oversized, &anchors)
            .err()
            .unwrap()
            .downcast_ref::<SourceMatchLimitExceeded>()
            .is_some()
    );
    assert_eq!(manifest.images.len(), 2);
    assert_eq!(manifest.images[0].source, "images/a.png");
    Ok(())
}

fn remote_fixture(
    occurrences: usize,
) -> Result<(
    tempfile::TempDir,
    Store,
    BookAssetStore,
    String,
    ChapterManifest,
)> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path().join("metadata.sqlite3"))?;
    store.upsert_book(&Book {
        source: "weread".into(),
        ..Book::new("book", "Synthetic source projection")
    })?;
    store.upsert_chapter(&Chapter {
        id: "uid".into(),
        book_id: "book".into(),
        index: 0,
        title: "Fixture chapter".into(),
        body: Some(tests::TEXT.into()),
        cached: true,
        content_status: ContentStatus::Full,
        content_account_key: "old-owner".into(),
    })?;
    store.save_position(&ReadingPosition {
        book_id: "book".into(),
        chapter_index: 0,
        char_offset: 3,
        ..ReadingPosition::default()
    })?;
    store.save_annotation(&Annotation {
        id: "note".into(),
        book_id: "book".into(),
        chapter_index: 0,
        start: 0,
        end: 6,
        quote: "Before".into(),
        note: "Keep the existing personal note".into(),
        ..Annotation::default()
    })?;
    store.save_bookmark(&Bookmark {
        id: "bookmark".into(),
        book_id: "book".into(),
        chapter_index: 0,
        char_offset: 3,
        label: "Keep the existing marker".into(),
        ..Bookmark::default()
    })?;
    let assets = BookAssetStore::open(directory.path().join("images"), 8 * 1024 * 1024)?;
    assets.recover(&store)?;
    let source = format!("images/{}/a.png", "p".repeat(1800));
    let html = format!(
        "<p>Before</p>{}<p>After</p>",
        format!("<img src='{source}'>").repeat(occurrences),
    );
    assert!(html.len() < 256 * 1024);
    let original = tests::asset(&source, 123)?;
    let packet = DownloadedChapterImages {
        source_to_asset: BTreeMap::from([(source.clone(), original.id.clone())]),
        assets: vec![original],
    };
    let manifest =
        assets.commit_chapter(&store, "book", "uid", &html, tests::TEXT, Some(&packet))?;
    assert_eq!(manifest.images.len(), occurrences);
    assert!(manifest.complete && !assets.reconcile_required.get());
    Ok((directory, store, assets, html, manifest))
}

#[test]
fn account_downgrade_text_only_checkpoint_reuses_one_projection_across_both_phases() -> Result<()> {
    let (_directory, store, assets, html, previous) = remote_fixture(32)?;
    let cache_key = previous.images[0].cache_key.as_ref().unwrap();
    let path = assets.cache.get_for_commit(cache_key)?.unwrap();
    let usage = assets.usage()?;
    let position = store.position("book")?;
    let notes = store.annotations("book")?;
    let bookmarks = store.bookmarks("book")?;
    store.invalidate_content_ownership(Some("new-owner"))?;
    let headers = store.chapter_metadata("book")?;
    assert_eq!(headers[0].content_status, ContentStatus::Unknown);
    assert!(!headers[0].cached && headers[0].body.is_none());
    let task = DownloadTask {
        id: "new-owner-task".into(),
        book_id: "book".into(),
        account_key: "new-owner".into(),
        chapters: vec![0],
        status: DownloadStatus::Running,
        updated_at: 100,
        ..DownloadTask::default()
    };
    assert!(!task.include_images);
    let _plan = crate::jobs::DownloadPlan::from_headers(&task, &headers)?;
    store.save_task(&task)?;
    let chapter = Chapter {
        id: "uid".into(),
        book_id: "book".into(),
        index: 0,
        title: "Fixture chapter".into(),
        body: Some(tests::TEXT.into()),
        cached: true,
        content_status: ContentStatus::Full,
        content_account_key: "new-owner".into(),
    };
    store.validate_download_chapter(&task, &chapter, 1)?;
    let metadata = prepare_chapter_metadata(&html, tests::TEXT)?;
    let manifest = assets.commit_prepared_chapter_metadata(
        &store,
        "book",
        "uid",
        &html,
        tests::TEXT,
        &metadata,
        None,
    )?;
    assert_eq!(assets.source_match_stats.get().old_occurrences, 32);
    assert_eq!(assets.source_match_stats.get().old_unique_sources, 1);
    assert_eq!(assets.source_match_stats.get().old_normalizations, 1);
    assert_eq!(assets.source_match_stats.get().new_unique_sources, 1);
    assert_eq!(assets.source_match_stats.get().new_normalizations, 0);
    assert_eq!(assets.source_match_stats.get().lookups, 64);
    assert_eq!(assets.verification_stats.get().hashes, 1);
    store.commit_download_checkpoint(&task, &chapter, 1, 101)?;
    assert_eq!(manifest, previous);
    assert_eq!(store.task(&task.id)?.unwrap().next_index, 1);
    assert_eq!(store.chapter("book", 0)?.unwrap(), chapter);
    assert_eq!(store.book("book")?.unwrap().cached_chapters, 1);
    assert_eq!(store.position("book")?, position);
    assert_eq!(store.annotations("book")?, notes);
    assert_eq!(store.bookmarks("book")?, bookmarks);
    assert_eq!(assets.usage()?, usage);
    assert_eq!(assets.cache.pinned_state(cache_key)?, Some(true));
    assert_eq!(assets.cache.get_for_commit(cache_key)?, Some(path));
    Ok(())
}

#[test]
fn projected_sources_cannot_bypass_verification_or_publish_partial_cached_images() -> Result<()> {
    let (_directory, store, assets, html, previous) = remote_fixture(24)?;
    let cache_key = previous.images[0].cache_key.as_ref().unwrap();
    let usage = assets.usage()?;
    let body = store.chapter("book", 0)?;
    let metadata = prepare_chapter_metadata(&html, tests::TEXT)?;
    let error = assets
        .commit_prepared_chapter_metadata_with_verification_budget(
            &store,
            "book",
            "uid",
            &html,
            tests::TEXT,
            &metadata,
            None,
            0,
        )
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<crate::cache::VerificationLimitExceeded>()
            .is_some()
    );
    assert_eq!(assets.source_match_stats.get().old_unique_sources, 1);
    assert_eq!(assets.source_match_stats.get().old_normalizations, 1);
    assert_eq!(assets.source_match_stats.get().lookups, 24);
    assert_eq!(
        assets.verification_stats.get(),
        crate::cache::VerificationStats::default()
    );
    assert_eq!(
        store.setting::<ChapterManifest>(&manifest_key("book", "uid")?)?,
        Some(previous.clone())
    );
    assert_eq!(store.chapter("book", 0)?, body);
    assert_eq!(assets.usage()?, usage);
    assert_eq!(assets.cache.pinned_state(cache_key)?, Some(true));
    assert!(assets.reconcile_required.get());
    Ok(())
}
