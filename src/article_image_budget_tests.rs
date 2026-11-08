//! Small whole-document image admission precedes URL/ID/caption materialization.

use std::cell::Cell;

use super::*;
use image_budget::{Limits, Usage};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Stats {
    pub usage: Usage,
}

#[derive(Clone, Copy)]
struct Observation {
    limits: Limits,
    stats: Stats,
}

thread_local! {
    static OBSERVATION: Cell<Option<Observation>> = const { Cell::new(None) };
}

pub(crate) struct ObservationScope;

impl ObservationScope {
    pub(crate) fn start(limits: ImageTestLimits) -> Self {
        OBSERVATION.with(|state| {
            assert!(
                state.get().is_none(),
                "The image budget observation is already active"
            );
            state.set(Some(Observation {
                limits,
                stats: Stats::default(),
            }));
        });
        Self
    }

    pub(crate) fn stats(&self) -> Stats {
        OBSERVATION.with(|state| state.get().expect("The image observation ended").stats)
    }
}

impl Drop for ObservationScope {
    fn drop(&mut self) {
        OBSERVATION.with(|state| state.set(None));
    }
}

pub(super) fn configured_limits() -> Option<Limits> {
    OBSERVATION.with(|state| state.get().map(|observation| observation.limits))
}

pub(super) fn observe(usage: Usage) {
    OBSERVATION.with(|state| {
        if let Some(mut observation) = state.get() {
            observation.stats.usage = usage;
            state.set(Some(observation));
        }
    });
}

fn no_transforms(stats: Stats) {
    assert_eq!(stats.usage.url_parse_calls, 0);
    assert_eq!(stats.usage.url_input_copies, 0);
    assert_eq!(stats.usage.url_output_copies, 0);
    assert_eq!(stats.usage.url_hashes, 0);
}

fn image_error(error: ArticleParseError) -> ImageLimitError {
    match error {
        ArticleParseError::Image(error) => error,
        other => panic!("Expected an independent image capacity error: {other:?}"),
    }
}

#[test]
fn article_image_count_and_late_url_quota_precede_all_transforms() {
    let html = "<p>A</p><img src='https://fixture.invalid/a.png'><img src='https://fixture.invalid/longer-b.png'>";
    {
        let scope = ObservationScope::start(ImageTestLimits {
            occurrences: 1,
            ..ImageTestLimits::default()
        });
        let error = image_error(ArticleDocument::parse_checked(html, "").unwrap_err());
        assert_eq!(error.kind, ImageLimitKind::Occurrences);
        no_transforms(scope.stats());
    }
    let scope = ObservationScope::start(ImageTestLimits {
        raw_url_bytes: "https://fixture.invalid/a.png".len(),
        ..ImageTestLimits::default()
    });
    let error = image_error(ArticleDocument::parse_checked(html, "").unwrap_err());
    assert_eq!(error.kind, ImageLimitKind::UrlBytes);
    no_transforms(scope.stats());
}

#[test]
fn article_image_exact_work_and_metadata_are_prepaid_before_materialization() {
    let html = "<p>A</p><img src='https://fixture.invalid/a.png' alt='Caption' data-size='1024'>";
    let baseline = {
        let scope = ObservationScope::start(ImageTestLimits::default());
        assert_eq!(
            ArticleDocument::parse_checked(html, "")
                .unwrap()
                .images()
                .len(),
            1
        );
        scope.stats().usage
    };
    let exact = ImageTestLimits {
        work_bytes: baseline.work_bytes,
        metadata_bytes: baseline.metadata_bytes,
        ..ImageTestLimits::default()
    };
    {
        let scope = ObservationScope::start(exact);
        assert_eq!(
            ArticleDocument::parse_checked(html, "")
                .unwrap()
                .images()
                .len(),
            1
        );
        assert_eq!(scope.stats().usage, baseline);
    }
    for (limits, kind) in [
        (
            ImageTestLimits {
                work_bytes: exact.work_bytes - 1,
                ..exact
            },
            ImageLimitKind::Work,
        ),
        (
            ImageTestLimits {
                metadata_bytes: exact.metadata_bytes - 1,
                ..exact
            },
            ImageLimitKind::Metadata,
        ),
    ] {
        let scope = ObservationScope::start(limits);
        assert_eq!(
            image_error(ArticleDocument::parse_checked(html, "").unwrap_err()).kind,
            kind
        );
        no_transforms(scope.stats());
    }
}

#[test]
fn article_image_aliases_srcset_and_unicode_keep_first_safe_https_selection() {
    let html = concat!(
        "<p>A</p><img data-src='http://unsafe.invalid/a.png'",
        " data-original='https://user@fixture.invalid/a.png'",
        " data-lazy-src='//fixture.invalid/a b.png' src='https://fixture.invalid/ignored.png'",
        " alt='  \u{4e2d}  \u{1f642}  '>",
        "<img src='http://unsafe.invalid/b.png' srcset='http://unsafe.invalid/c.png 1x, //fixture.invalid/set.png 2x'>"
    );
    let images = ArticleDocument::parse_checked(html, "").unwrap().images();
    assert_eq!(images.len(), 2);
    assert_eq!(images[0].url, "https://fixture.invalid/a%20b.png");
    assert_eq!(images[0].caption, "\u{4e2d} \u{1f642}");
    assert_eq!(images[1].url, "https://fixture.invalid/set.png");
}

#[test]
fn article_image_unsafe_skips_keep_caption_association_and_first_seen_inventory() {
    let html = concat!(
        "<p>A</p><img src='https://fixture.invalid/a.png' alt='First'>",
        "<img src='http://unsafe.invalid/a.png'><figcaption>Override</figcaption>",
        "<img src='https://fixture.invalid/a.png' alt='Second'>"
    );
    let document = ArticleDocument::parse_checked(html, "").unwrap();
    let images = document.images();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].caption, "Override");
    let pages = document.paginate(
        &Layout::default(),
        &TextMetrics::default(),
        &BTreeSet::new(),
    );
    let blocks = pages
        .iter()
        .flatten()
        .filter(|block| block.kind == "image")
        .collect::<Vec<_>>();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].text, "Override");
    assert_eq!(blocks[1].text, "Second");
    assert!(
        blocks
            .iter()
            .all(|block| block.image_url.as_deref() == Some("https://fixture.invalid/a.png"))
    );
}

#[test]
fn article_image_caption_and_srcset_limits_reject_before_url_parsing() {
    {
        let scope = ObservationScope::start(ImageTestLimits {
            caption_bytes: 3,
            ..ImageTestLimits::default()
        });
        let html =
            "<p>A</p><img src='https://fixture.invalid/a.png'><figcaption>Too long</figcaption>";
        assert_eq!(
            image_error(ArticleDocument::parse_checked(html, "").unwrap_err()).kind,
            ImageLimitKind::Caption
        );
        no_transforms(scope.stats());
    }
    let scope = ObservationScope::start(ImageTestLimits {
        url_candidates: 1,
        ..ImageTestLimits::default()
    });
    let html =
        "<p>A</p><img srcset='http://unsafe.invalid/a.png 1x, https://fixture.invalid/b.png 2x'>";
    assert_eq!(
        image_error(ArticleDocument::parse_checked(html, "").unwrap_err()).kind,
        ImageLimitKind::Occurrences
    );
    no_transforms(scope.stats());
}

#[test]
fn anchor_image_complete_count_and_selected_field_admission_precede_hash_copy() {
    let html = "<p>A</p><img src='one.png'><img src='two.png'>";
    {
        let scope = ObservationScope::start(ImageTestLimits {
            occurrences: 1,
            ..ImageTestLimits::default()
        });
        assert!(image_anchors(html, "A").is_err());
        no_transforms(scope.stats());
    }
    let scope = ObservationScope::start(ImageTestLimits {
        raw_url_bytes: 3,
        ..ImageTestLimits::default()
    });
    assert!(image_anchors(html, "A").is_err());
    no_transforms(scope.stats());
}

#[test]
fn image_work_legacy_vector_admission_does_not_filter_or_parse_urls() {
    let images = vec![
        ArticleImage {
            id: "opaque".into(),
            url: "http://legacy.invalid/a".into(),
            caption: "Old".into(),
        },
        ArticleImage {
            id: "".into(),
            url: "".into(),
            caption: "".into(),
        },
    ];
    {
        let scope = ObservationScope::start(ImageTestLimits::default());
        image_work_preflight(&images).unwrap();
        assert_eq!(scope.stats().usage.records, 2);
        no_transforms(scope.stats());
    }
    let scope = ObservationScope::start(ImageTestLimits {
        occurrences: 1,
        ..ImageTestLimits::default()
    });
    assert!(image_work_preflight(&images).is_err());
    no_transforms(scope.stats());
}

#[test]
fn article_image_quota_cannot_become_partial_or_fallback_document() {
    let html = "<p>Preserved body</p><img src='https://fixture.invalid/a.png'>";
    let scope = ObservationScope::start(ImageTestLimits {
        work_bytes: 0,
        ..ImageTestLimits::default()
    });
    // Capacity tests use the production Result entry point. The cfg(test)
    // ordinary constructor expects an admitted fixture and cannot return empty.
    for fallback in ["", "Unpublished fallback"] {
        let error = ArticleDocument::parse_checked(html, fallback).unwrap_err();
        let ArticleParseError::Image(error) = error else {
            panic!("The image quota must remain an exact typed image error: {error:?}");
        };
        assert_eq!(error.kind, ImageLimitKind::Work);
        assert_eq!(error.used, 0);
        assert_eq!(error.limit, 0);
        assert!(error.requested > 0);
    }
    no_transforms(scope.stats());
}
