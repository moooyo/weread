//! Small immutable DOM style work, exact charwise semantics, and bounded wrappers.

use std::cell::Cell;

use super::*;
use style_budget::{Budget, Limits, Usage};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Stats {
    pub operations: usize,
    pub admissions: usize,
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
    pub(crate) fn start(limits: StyleTestLimits) -> Self {
        OBSERVATION.with(|state| {
            assert!(
                state.get().is_none(),
                "The style observation is already active"
            );
            state.set(Some(Observation {
                limits,
                stats: Stats::default(),
            }));
        });
        Self
    }

    pub(crate) fn stats(&self) -> Stats {
        OBSERVATION.with(|state| state.get().expect("The style observation ended").stats)
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

fn update(operation: impl FnOnce(&mut Stats)) {
    OBSERVATION.with(|state| {
        if let Some(mut observation) = state.get() {
            operation(&mut observation.stats);
            state.set(Some(observation));
        }
    });
}

pub(super) fn observe_start() {
    update(|stats| stats.operations += 1);
}

pub(super) fn observe_admission(usage: Usage) {
    update(|stats| {
        stats.admissions += 1;
        stats.usage = usage;
    });
}

pub(super) fn observe(usage: Usage) {
    update(|stats| stats.usage = usage);
}

pub(crate) fn reconstructed_html(style: &str, following: usize) -> String {
    let mut html = format!("<html><body><p><b style='{style}'>A</p>");
    for _ in 0..following {
        html.push_str("<p>A</p>");
    }
    html.push_str("</body></html>");
    html
}

#[test]
fn borrowed_content_container_preserves_the_pinned_union_selector() {
    for body in [
        "<div class='rich_media_content' data-order='first'>A</div><div id='js_content' data-order='second'>B</div>",
        "<div id='js_content' data-order='first'>A</div><div class='rich_media_content' data-order='second'>B</div>",
        "<div id='JS_CONTENT' class='RICH_MEDIA_CONTENT' data-order='upper'>A</div><p>B</p>",
        "<div class='prefix\u{a0}rich_media_content' data-order='joined'>A</div><div class='prefix\t rich_media_content' data-order='split'>B</div>",
        "<svg><g class='rich_media_content' data-order='foreign'>A</g></svg><div id='js_content' data-order='later'>B</div>",
    ] {
        for doctype in ["", "<!DOCTYPE html>"] {
            let html = format!("{doctype}<html><body>{body}</body></html>");
            let expected_document = Html::parse_document(&html);
            let selector = Selector::parse("#js_content, .rich_media_content").unwrap();
            let expected = expected_document.select(&selector).next().map(|element| {
                (
                    element.value().name().to_owned(),
                    element.attr("data-order").unwrap_or_default().to_owned(),
                )
            });
            let document = Html::parse_document(&html);
            let actual = ArticleStyleWork::default()
                .content_container(&document)
                .unwrap()
                .map(|element| {
                    (
                        element.value().name().to_owned(),
                        element.attr("data-order").unwrap_or_default().to_owned(),
                    )
                });
            assert_eq!(actual, expected);
        }
    }
}

#[test]
fn reconstructed_raw_classes_share_borrowed_scans_without_class_cache_copies() {
    let class = format!("{} readable", "font-credit ".repeat(32));
    let mut html = format!("<html><body><p><b class='{class}'>A</p>");
    for _ in 0..8 {
        html.push_str("<p>A</p>");
    }
    html.push_str("</body></html>");
    let document = Html::parse_document(&html);
    let spans: Vec<_> = document
        .tree
        .nodes()
        .filter_map(ElementRef::wrap)
        .filter(|element| element.value().name() == "b")
        .filter_map(|element| element.attr("class"))
        .collect();
    assert!(spans.len() > 1);
    assert!(spans.iter().all(|span| span.as_ptr() == spans[0].as_ptr()));
    let scope = ObservationScope::start(StyleTestLimits::default());
    let mut work = ArticleStyleWork::default();
    assert!(work.content_container(&document).unwrap().is_none());
    for span in &spans {
        assert!(!work.suppressed_class(span).unwrap());
    }
    assert_eq!(scope.stats().usage.class_scans, 2);
    assert_eq!(scope.stats().usage.class_input_bytes, 2 * class.len());
    assert_eq!(scope.stats().usage.class_memo_hits, 2 * (spans.len() - 1));
}

#[test]
fn raw_class_exact_work_and_one_less_refuse_before_split_or_fallback() {
    let class = format!("{} rich_media_content", "ordinary ".repeat(16));
    let html = format!("<div class='{class}'><p>x</p></div>");
    let document = Html::parse_document(&html);
    let required = {
        let scope = ObservationScope::start(StyleTestLimits::default());
        assert!(
            ArticleStyleWork::default()
                .content_container(&document)
                .unwrap()
                .is_some()
        );
        scope.stats().usage
    };
    {
        let scope = ObservationScope::start(StyleTestLimits {
            work_bytes: required.work_bytes,
            metadata_bytes: required.metadata_bytes,
        });
        assert!(
            ArticleStyleWork::default()
                .content_container(&document)
                .unwrap()
                .is_some()
        );
        assert_eq!(scope.stats().usage.class_scans, 1);
    }
    let scope = ObservationScope::start(StyleTestLimits {
        work_bytes: required.work_bytes - 1,
        ..StyleTestLimits::default()
    });
    assert!(matches!(
        ArticleDocument::parse_checked(&html, "Unpublished fallback"),
        Err(ArticleParseError::Style(_))
    ));
    assert_eq!(scope.stats().usage.class_scans, 0);
    assert_eq!(scope.stats().usage.class_input_bytes, 0);
}

#[test]
fn container_id_and_collector_hidden_shortcuts_do_not_scan_raw_classes() {
    let html = "<div id='js_content'><div hidden class='rich_media_meta'>Hidden</div><script class='rich_media_meta'>Hidden</script><p>Visible</p></div>";
    let scope = ObservationScope::start(StyleTestLimits {
        work_bytes: 0,
        metadata_bytes: 0,
    });
    assert_eq!(
        ArticleDocument::parse_checked(html, "")
            .unwrap()
            .plain_text(),
        "Visible"
    );
    assert_eq!(scope.stats().usage.class_scans, 0);
    assert_eq!(scope.stats().usage.lookups, 0);
}

#[test]
fn style_streaming_matcher_preserves_filter_lower_and_contains_order() {
    for style in [
        "",
        "color:blue",
        "DISPLAY : NONE",
        "v i s i b i l i t y:\tHIDDEN",
        "before-display:none-after",
        "ddisplay:none",
        "vvisibility:hidden",
        "DISPLAY:\u{a0}NONE",
        "dis\u{200b}play:none",
        "DISPLA\u{178}:NONE",
        "\u{130}DISPLAY:NONE",
        "vis\u{130}bility:hidden",
        "\u{1f642} color:Blue",
    ] {
        // The reference is deliberately tiny. Production never builds this
        // normalized string, but must preserve exactly its byte semantics.
        let compact: String = style
            .chars()
            .filter(|character| !character.is_ascii_whitespace())
            .flat_map(char::to_lowercase)
            .collect();
        let expected = compact.contains("display:none") || compact.contains("visibility:hidden");
        let mut budget = Budget::default();
        assert_eq!(budget.suppresses(style).unwrap(), expected, "{style:?}");
    }
}

#[test]
fn reconstructed_formatting_styles_scan_one_borrowed_span_per_operation() {
    let style = "color:Blue; custom-value:abcdefghijklmnopqrstuvwxyz0123456789";
    let html = reconstructed_html(style, 8);
    let document = Html::parse_document(&html);
    let selector = Selector::parse("b[style]").unwrap();
    let copies = document.select(&selector).count();
    assert_eq!(copies, 9);
    {
        let scope = ObservationScope::start(StyleTestLimits::default());
        let parsed = ArticleDocument::parse_checked(&html, "").unwrap();
        assert_eq!(parsed.plain_text().lines().count(), 9);
        assert_eq!(scope.stats().operations, 1);
        assert_eq!(scope.stats().usage.style_scans, 1);
        assert_eq!(scope.stats().usage.memo_hits, copies - 1);
        assert_eq!(scope.stats().usage.input_bytes, style.len());
    }
    let canonical = crate::source::content::html_to_text(&html);
    let scope = ObservationScope::start(StyleTestLimits::default());
    assert!(image_anchors(&html, &canonical).unwrap().is_empty());
    assert_eq!(scope.stats().operations, 1);
    assert_eq!(scope.stats().usage.style_scans, 1);
    assert_eq!(scope.stats().usage.memo_hits, copies - 1);
    assert_eq!(scope.stats().usage.input_bytes, style.len());
}

#[test]
fn style_exact_work_and_metadata_reject_before_lowering() {
    let style = "color:Blue; margin:0";
    let baseline = {
        let scope = ObservationScope::start(StyleTestLimits::default());
        assert!(!Budget::default().suppresses(style).unwrap());
        scope.stats().usage
    };
    let exact = StyleTestLimits {
        work_bytes: baseline.work_bytes,
        metadata_bytes: baseline.metadata_bytes,
    };
    {
        let scope = ObservationScope::start(exact);
        assert!(!Budget::default().suppresses(style).unwrap());
        assert_eq!(scope.stats().usage, baseline);
    }
    for (limits, kind) in [
        (
            StyleTestLimits {
                work_bytes: exact.work_bytes - 1,
                ..exact
            },
            StyleLimitKind::Work,
        ),
        (
            StyleTestLimits {
                metadata_bytes: exact.metadata_bytes - 1,
                ..exact
            },
            StyleLimitKind::Metadata,
        ),
    ] {
        let scope = ObservationScope::start(limits);
        let error = Budget::default().suppresses(style).unwrap_err();
        assert_eq!(error.kind, kind);
        assert_eq!(scope.stats().admissions, 0);
        assert_eq!(scope.stats().usage.style_scans, 0);
        assert_eq!(scope.stats().usage.lowered_chars, 0);
        assert_eq!(scope.stats().usage.normalized_bytes, 0);
    }
}

#[test]
fn style_priority_shortcuts_do_not_read_suppressed_node_attributes() {
    let html = concat!(
        "<div hidden style='display:none'><p>Hidden</p></div>",
        "<p aria-hidden='true' style='visibility:hidden'>Hidden</p>",
        "<script style='color:blue'>Hidden script</script><p>Visible</p>"
    );
    let scope = ObservationScope::start(StyleTestLimits {
        work_bytes: 0,
        metadata_bytes: 0,
    });
    assert_eq!(
        ArticleDocument::parse_checked(html, "")
            .unwrap()
            .plain_text(),
        "Visible"
    );
    assert!(image_anchors(html, "Visible").unwrap().is_empty());
    assert_eq!(scope.stats().usage.lookups, 0);
    assert_eq!(scope.stats().usage.style_scans, 0);
}

#[test]
fn anchor_caption_and_article_quote_styles_share_each_operation_budget() {
    let html = concat!(
        "<p>Before</p><img src='https://fixture.invalid/figure.png'>",
        "<figcaption style='color:Blue'><b style='margin:0'>Caption</b></figcaption>",
        "<blockquote style='padding:0'><b style='font-weight:400'>After</b></blockquote>"
    );
    {
        let scope = ObservationScope::start(StyleTestLimits::default());
        assert_eq!(image_anchors(html, "BeforeCaptionAfter").unwrap().len(), 1);
        assert_eq!(scope.stats().operations, 1);
        assert_eq!(scope.stats().usage.memo_hits, 1);
        assert_eq!(scope.stats().usage.style_scans, 4);
    }
    let baseline = {
        let scope = ObservationScope::start(StyleTestLimits::default());
        assert!(!ArticleDocument::parse_checked(html, "").unwrap().is_empty());
        assert_eq!(scope.stats().operations, 1);
        scope.stats().usage
    };
    let scope = ObservationScope::start(StyleTestLimits {
        work_bytes: baseline.work_bytes - 1,
        ..StyleTestLimits::default()
    });
    assert!(ArticleDocument::parse_checked(html, "").is_err());
    assert_eq!(scope.stats().operations, 1);
}

#[test]
fn checked_article_capacity_never_returns_partial_or_fallback_content() {
    let html = "<p>First visible block</p><p style='color:Blue'>Later block</p>";
    let scope = ObservationScope::start(StyleTestLimits {
        work_bytes: 0,
        ..StyleTestLimits::default()
    });
    // Both fallback choices must fail through the production Result API. No
    // partial document or successful empty ordinary fixture can be produced.
    for fallback in ["", "Unpublished fallback"] {
        let error = ArticleDocument::parse_checked(html, fallback).unwrap_err();
        let ArticleParseError::Style(error) = error else {
            panic!("The style quota must remain an exact typed style error: {error:?}");
        };
        assert_eq!(error.kind, StyleLimitKind::Work);
        assert_eq!(error.used, 0);
        assert_eq!(error.limit, 0);
        assert!(error.requested > 0);
    }
    assert_eq!(scope.stats().usage.style_scans, 0);
    assert_eq!(scope.stats().usage.lowered_chars, 0);
}

#[test]
fn anchor_style_quota_remains_a_strict_capacity_result() {
    let html = "<p style='color:Blue'>A</p><img src='figure.png'>";
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
        let error = image_anchors(html, "A").unwrap_err();
        assert_eq!(error.kind, AnchorLimitKind::MatchingWork);
        assert_eq!(scope.stats().usage.style_scans, 0);
        assert_eq!(scope.stats().usage.lowered_chars, 0);
    }
}
