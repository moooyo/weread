//! Public Source style gates and actual prepared-publication refusal stay resource typed.

use super::*;
use crate::{
    article::{AnchorLimitError, StyleTestLimits, style_budget_tests::ObservationScope},
    book_assets::{BookAssetStore, prepare_chapter_metadata},
    domain::{Annotation, Book, Chapter, ContentStatus},
    store::Store,
};

#[test]
fn source_style_quota_precedes_plan_and_prepared_publication() {
    let html = "<p style='color:Blue'>A</p><img src='https://fixture.invalid/figure.png'>";
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("library.sqlite")).unwrap();
    store
        .upsert_book(&Book {
            id: "style-book".into(),
            title: "Preserved book".into(),
            source: "weread".into(),
            total_chapters: 1,
            ..Default::default()
        })
        .unwrap();
    store
        .upsert_chapter(&Chapter {
            id: "style-chapter".into(),
            book_id: "style-book".into(),
            index: 0,
            title: "Preserved chapter".into(),
            cached: true,
            body: Some("A".into()),
            content_status: ContentStatus::Full,
            content_account_key: "synthetic-style-owner".into(),
        })
        .unwrap();
    let note = Annotation {
        id: "style-note".into(),
        book_id: "style-book".into(),
        chapter_index: 0,
        start: 0,
        end: 1,
        quote: "A".into(),
        note: "Preserved note".into(),
        created_at: 1,
    };
    store.save_annotation(&note).unwrap();
    let assets = BookAssetStore::open(directory.path().join("images"), 64 * 1024).unwrap();
    let manifest = assets
        .commit_chapter(&store, "style-book", "style-chapter", "<p>A</p>", "A", None)
        .unwrap();
    let before_chapter = store.chapter("style-book", 0).unwrap();
    let before_book = store.book("style-book").unwrap();
    let before_usage = assets.usage().unwrap();
    for limits in [
        StyleTestLimits {
            work_bytes: 0,
            ..StyleTestLimits::default()
        },
        StyleTestLimits {
            metadata_bytes: 0,
            ..StyleTestLimits::default()
        },
    ] {
        let scope = ObservationScope::start(limits);
        let error = make_plan("style-book", "style-chapter", html, None).unwrap_err();
        assert!(matches!(error, SourceError::ResponseTooLarge));
        assert!(!error.requires_login() && !error.retryable());
        let error = prepare_chapter_metadata(html, "A").unwrap_err();
        assert!(error.downcast_ref::<AnchorLimitError>().is_some());
        let error = assets
            .commit_prepared_chapter(&store, "style-book", "style-chapter", html, "A", None)
            .unwrap_err();
        assert!(error.downcast_ref::<AnchorLimitError>().is_some());
        assert_eq!(scope.stats().usage.style_scans, 0);
        assert_eq!(scope.stats().usage.lowered_chars, 0);
        assert_eq!(scope.stats().usage.normalized_bytes, 0);
        assert_eq!(store.chapter("style-book", 0).unwrap(), before_chapter);
        assert_eq!(store.book("style-book").unwrap(), before_book);
        assert_eq!(store.annotations("style-book").unwrap(), vec![note.clone()]);
        assert_eq!(assets.usage().unwrap(), before_usage);
        assert_eq!(
            assets
                .chapter(&store, "style-book", "style-chapter")
                .unwrap(),
            Some(manifest.clone())
        );
    }
}

#[test]
fn source_style_gate_preserves_original_reference_visibility() {
    let html = concat!(
        "<div style='D I S P L A Y : N O N E'>",
        "<img src='https://fixture.invalid/figure.png'></div><p>A</p>"
    );
    let plan = make_plan("book", "chapter", html, None).unwrap();
    // The preflight does not turn Source planning into a visibility filter.
    // The strict anchor consumer still makes its original suppression choice.
    assert_eq!(plan.references.len(), 1);
    assert_eq!(plan.remote_images.len(), 1);
    let scope = ObservationScope::start(StyleTestLimits::default());
    assert!(crate::article::image_anchors(html, "A").unwrap().is_empty());
    assert_eq!(scope.stats().usage.style_scans, 1);
}
