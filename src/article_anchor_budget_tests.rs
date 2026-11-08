//! Small quota fixtures for occurrence admission and cumulative anchor work.

use super::*;
use anyhow::Result as AnyResult;

fn locate(
    html: &str,
    canonical: &str,
    limits: AnchorLimits,
) -> Result<Vec<AnchoredImage>, AnchorLimitError> {
    let document = Html::parse_document(html);
    image_anchors_in_document_with_limits(&document, canonical, limits)
}

fn caption_prefix(count: usize) -> String {
    "<img src='same.png'><figcaption>A</figcaption>".repeat(count)
}

#[test]
fn repeated_occurrences_are_rejected_before_hashing_or_matching() {
    let error = locate(
        "<img src='same.png'><img src='same.png'><img src='same.png'>",
        "A",
        AnchorLimits {
            occurrences: 2,
            matching_work: 0,
            ..AnchorLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, AnchorLimitKind::Occurrences);
    assert_eq!(error.usage.occurrences, 2);
    assert_eq!(error.usage.events, 2);
    assert_eq!(error.usage.edges, 0);
    assert_eq!(error.usage.matching_work, 0);
    assert_eq!((error.requested, error.limit), (1, 2));
}

#[test]
fn event_admission_precedes_new_pending_text_and_image_data() {
    for html in [
        "<p>A</p><img src='same.png'>",
        "<img src='same.png'><p>A</p>",
    ] {
        let error = locate(
            html,
            "A",
            AnchorLimits {
                events: 1,
                ..AnchorLimits::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.kind, AnchorLimitKind::Events);
        assert_eq!(error.usage.events, 1);
        assert_eq!(error.usage.occurrences, 1);
        assert_eq!(error.usage.matching_work, 0);
    }
}

#[test]
fn visible_caption_is_admitted_before_its_first_text_chunk_is_copied() {
    let document = Html::parse_document(
        "<figcaption>\u{8bf4}\u{660e}<span hidden>suppressed</span></figcaption>",
    );
    let selector = Selector::parse("figcaption").unwrap();
    let caption = document.select(&selector).next().unwrap();
    let mut output = String::new();
    let mut budget = AnchorBudget::new(AnchorLimits {
        events: 0,
        ..AnchorLimits::default()
    });
    let error = caption_text(caption, &mut output, &mut budget).unwrap_err();
    assert_eq!(error.kind, AnchorLimitKind::Events);
    assert_eq!(error.usage.events, 0);
    assert!(output.is_empty());
}

#[test]
fn an_image_edge_batch_is_admitted_without_a_partial_extend() {
    let error = locate(
        &caption_prefix(2),
        "AAA",
        AnchorLimits {
            edges: 4,
            ..AnchorLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, AnchorLimitKind::Edges);
    assert_eq!(error.usage.edges, 3);
    assert_eq!(error.requested, 2);
    assert_eq!(error.limit, 4);
    assert_eq!(error.usage.occurrences, 2);
    assert_eq!(error.usage.matching_work, 4);
}

#[test]
fn standalone_captions_cannot_grow_an_unbounded_alignment_graph() {
    let html = format!(
        "{}{}",
        caption_prefix(4),
        "<figcaption>A</figcaption>".repeat(32)
    );
    let error = locate(
        &html,
        &"A".repeat(36),
        AnchorLimits {
            edges: 45,
            ..AnchorLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, AnchorLimitKind::Edges);
    assert_eq!(error.usage.occurrences, 4);
    assert_eq!(error.usage.events, 40);
    assert_eq!(error.usage.edges, 45);
    assert_eq!(error.usage.matching_work, 62);
    assert_eq!(error.requested, 1);
}

#[test]
fn matching_work_is_reserved_before_the_slice_comparison() {
    let error = locate(
        "<p>ABCD</p><img src='same.png'>",
        "ABCD",
        AnchorLimits {
            matching_work: 7,
            ..AnchorLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, AnchorLimitKind::MatchingWork);
    assert_eq!(error.usage.matching_work, 4);
    assert_eq!(error.usage.edges, 0);
    assert_eq!((error.requested, error.limit), (4, 7));
}

#[test]
fn matching_work_and_anchors_use_unicode_scalars_instead_of_utf8_bytes() {
    let text = "\u{7532}\u{1f642}";
    assert!(text.len() > 4);
    let images = locate(
        "<p>\u{7532}\u{1f642}</p><img src='same.png'>",
        text,
        AnchorLimits {
            matching_work: 4,
            ..AnchorLimits::default()
        },
    )
    .unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].anchor, Some(2));
}

#[test]
fn hidden_images_and_empty_captions_do_not_consume_visible_admission() {
    let limits = AnchorLimits {
        occurrences: 0,
        events: 0,
        edges: 0,
        matching_work: 0,
    };
    let images = locate(
        "<img hidden src='hidden.png'><span style='display:none'><img src='other.png'></span>\
         <figcaption> \u{200b}<span hidden>suppressed</span></figcaption>",
        "",
        limits,
    )
    .unwrap();
    assert!(images.is_empty());
    let error = locate("<img>", "", limits).unwrap_err();
    assert_eq!(error.kind, AnchorLimitKind::Occurrences);
    assert_eq!(error.usage.occurrences, 0);
}

#[test]
fn cursor_overflow_keeps_all_occurrences_and_a_conservative_unknown_suffix() {
    let html = format!("{}<img src='last.png'>", caption_prefix(128));
    let images = image_anchors(&html, &"A".repeat(128)).unwrap();
    assert_eq!(images.len(), 129);
    assert_eq!(images[0].anchor, Some(0));
    assert_eq!(images.last().unwrap().anchor, None);
}

#[test]
fn production_metadata_quota_failure_preserves_existing_body_and_asset_publication() -> AnyResult<()>
{
    use crate::{
        article::image_budget_tests::ObservationScope as ImageScope,
        book_assets::BookAssetStore,
        domain::{Book, Chapter},
        source::SourceError,
        source::images::make_plan,
        store::Store,
    };

    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path().join("metadata.sqlite3"))?;
    store.upsert_book(&Book::new("book", "Fixture book"))?;
    store.upsert_chapter(&Chapter {
        id: "uid".into(),
        book_id: "book".into(),
        index: 0,
        title: "Fixture chapter".into(),
        body: Some("Before".into()),
        ..Chapter::default()
    })?;
    let assets = BookAssetStore::open(directory.path().join("images"), 1024 * 1024)?;
    let previous =
        assets.commit_prepared_chapter(&store, "book", "uid", "<p>Before</p>", "Before", None)?;
    assert!(previous.complete && previous.images.is_empty());
    let reference = "<img src='https://images.example.test/same.png'>";
    // The exact admitted Source boundary still deduplicates repeated URLs.
    // Its independent occurrence gate now precedes the old 2,049-row fixture.
    let admitted_html = reference.repeat(2048);
    let plan = make_plan("book", "uid", &admitted_html, None)?;
    assert_eq!(plan.references.len(), 1);
    assert_eq!(plan.remote_images.len(), 1);
    assert_eq!(
        plan.references[0].source,
        "https://images.example.test/same.png"
    );
    let html = reference.repeat(2049);
    {
        let scope = ImageScope::start(ImageTestLimits::default());
        let source_error = make_plan("book", "uid", &html, None).unwrap_err();
        assert!(matches!(&source_error, SourceError::ResponseTooLarge));
        assert!(!source_error.requires_login() && !source_error.retryable());
        let usage = scope.stats().usage;
        assert_eq!(usage.records, 2048);
        assert_eq!(usage.url_parse_calls, 0);
        assert_eq!(usage.url_input_copies, 0);
        assert_eq!(usage.url_output_copies, 0);
        assert_eq!(usage.url_hashes, 0);
        assert_eq!(
            assets.chapter(&store, "book", "uid")?,
            Some(previous.clone())
        );
        assert_eq!(
            store.chapter_body_by_uid("book", "uid")?,
            Some("Before".into())
        );
        assert_eq!(assets.usage()?, 0);
    }
    // Keep the original strict BookAssets entry and all of its occurrence,
    // graph-work, body-binding, and publication-preservation assertions.
    let error = assets
        .commit_prepared_chapter(&store, "book", "uid", &html, "Before", None)
        .unwrap_err();
    let quota = error.downcast_ref::<AnchorLimitError>().unwrap();
    assert_eq!(quota.kind, AnchorLimitKind::Occurrences);
    assert_eq!(quota.usage.occurrences, 2048);
    assert_eq!(quota.usage.edges, 0);
    assert_eq!(quota.usage.matching_work, 0);
    assert_eq!(assets.chapter(&store, "book", "uid")?, Some(previous));
    assert_eq!(
        store.chapter_body_by_uid("book", "uid")?,
        Some("Before".into())
    );
    assert_eq!(assets.usage()?, 0);
    Ok(())
}
