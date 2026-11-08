//! Public Source/anchor preparation refuses image capacity before publication.

use super::*;
use crate::{
    article::{AnchorLimitError, ImageTestLimits, image_budget_tests::ObservationScope},
    book_assets::{BookAssetStore, prepare_chapter_metadata},
    domain::{Annotation, Book, Chapter, ContentStatus},
    store::Store,
};

#[test]
fn source_article_image_quota_is_resource_typed_before_plan_or_preparation_copy() {
    let html = "<p>A</p><img src='https://fixture.invalid/figure.png'>";
    for limits in [
        ImageTestLimits {
            raw_url_bytes: 8,
            ..ImageTestLimits::default()
        },
        ImageTestLimits {
            work_bytes: 0,
            ..ImageTestLimits::default()
        },
        ImageTestLimits {
            metadata_bytes: 0,
            ..ImageTestLimits::default()
        },
    ] {
        let scope = ObservationScope::start(limits);
        let error = make_plan("book", "chapter", html, None).unwrap_err();
        assert!(matches!(error, SourceError::ResponseTooLarge));
        assert!(!error.requires_login() && !error.retryable());
        let error = prepare_chapter_metadata(html, "A").unwrap_err();
        assert!(error.downcast_ref::<AnchorLimitError>().is_some());
        assert_eq!(scope.stats().usage.url_parse_calls, 0);
        assert_eq!(scope.stats().usage.url_input_copies, 0);
        assert_eq!(scope.stats().usage.url_hashes, 0);
    }
}

#[test]
fn source_article_image_publication_quota_preserves_existing_body_notes_and_manifest() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("library.sqlite")).unwrap();
    store
        .upsert_book(&Book {
            id: "image-quota-book".into(),
            title: "Preserved".into(),
            source: "weread".into(),
            total_chapters: 1,
            ..Default::default()
        })
        .unwrap();
    store
        .upsert_chapter(&Chapter {
            id: "image-quota-chapter".into(),
            book_id: "image-quota-book".into(),
            index: 0,
            title: "Preserved chapter".into(),
            body: Some("A".into()),
            cached: true,
            content_status: ContentStatus::Full,
            content_account_key: "synthetic-image-owner".into(),
        })
        .unwrap();
    let note = Annotation {
        id: "image-quota-note".into(),
        book_id: "image-quota-book".into(),
        chapter_index: 0,
        start: 0,
        end: 1,
        quote: "A".into(),
        note: "Preserved note".into(),
        created_at: 1,
    };
    store.save_annotation(&note).unwrap();
    let assets = BookAssetStore::open(directory.path().join("images"), 64 * 1024).unwrap();
    let original = assets
        .commit_chapter(
            &store,
            "image-quota-book",
            "image-quota-chapter",
            "<p>A</p>",
            "A",
            None,
        )
        .unwrap();
    let chapter = store.chapter("image-quota-book", 0).unwrap();
    let usage = assets.usage().unwrap();
    let scope = ObservationScope::start(ImageTestLimits {
        raw_url_bytes: 3,
        ..ImageTestLimits::default()
    });
    let error = assets
        .commit_prepared_chapter(
            &store,
            "image-quota-book",
            "image-quota-chapter",
            "<p>A</p><img src='figure.png'>",
            "A",
            None,
        )
        .unwrap_err();
    assert!(error.downcast_ref::<AnchorLimitError>().is_some());
    assert_eq!(store.chapter("image-quota-book", 0).unwrap(), chapter);
    assert_eq!(store.annotations("image-quota-book").unwrap(), vec![note]);
    assert_eq!(assets.usage().unwrap(), usage);
    assert_eq!(
        assets
            .chapter(&store, "image-quota-book", "image-quota-chapter")
            .unwrap(),
        Some(original)
    );
    assert_eq!(scope.stats().usage.url_hashes, 0);
}
