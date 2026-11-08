//! Small public Source plans compare checked SVG ancestry with the locked selector.

use std::cell::Cell;

use scraper::ElementRef;

use super::*;
use image_svg_guard::{Limits, Usage};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Stats {
    pub preflights: usize,
    pub admitted: usize,
    pub rejected: usize,
    pub summary_builds: usize,
    pub summary_elements: usize,
    pub summary_capacity: usize,
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
    pub(crate) fn start(limits: SvgGuardLimits) -> Self {
        OBSERVATION.with(|state| {
            assert!(
                state.get().is_none(),
                "The SVG guard observation is already active"
            );
            state.set(Some(Observation {
                limits,
                stats: Stats::default(),
            }));
        });
        Self
    }

    pub(crate) fn stats(&self) -> Stats {
        OBSERVATION.with(|state| state.get().expect("The SVG guard observation ended").stats)
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

fn observe(operation: impl FnOnce(&mut Stats)) {
    OBSERVATION.with(|state| {
        if let Some(mut observation) = state.get() {
            operation(&mut observation.stats);
            state.set(Some(observation));
        }
    });
}

pub(super) fn observe_preflight(usage: Usage, admitted: bool) {
    observe(|stats| {
        stats.preflights += 1;
        stats.admitted += usize::from(admitted);
        stats.rejected += usize::from(!admitted);
        stats.usage = usage;
    });
}

pub(super) fn observe_build(elements: usize, capacity: usize) {
    observe(|stats| {
        stats.summary_builds += 1;
        stats.summary_elements = elements;
        stats.summary_capacity = capacity;
    });
}

pub(super) fn observe_complete(usage: Usage) {
    observe(|stats| stats.usage = usage);
}

pub(crate) fn mathml(depth: usize, images: usize) -> String {
    let mut html = String::from("<html><body><p>A</p><math>");
    for _ in 0..depth {
        html.push_str("<mrow>");
    }
    for _ in 0..images {
        html.push_str("<image/>");
    }
    for _ in 0..depth {
        html.push_str("</mrow>");
    }
    html.push_str("</math></body></html>");
    html
}

fn resource_error(error: &SourceError) {
    assert!(matches!(error, SourceError::ResponseTooLarge));
    assert!(!error.requires_login());
    assert!(!error.retryable());
}

#[test]
fn svg_guard_mathml_without_svg_uses_one_linear_parent_summary() {
    for (depth, images) in [(4, 4), (8, 8), (12, 12)] {
        let html = mathml(depth, images);
        let scope = ObservationScope::start(SvgGuardLimits::default());
        let plan = make_plan("book", "chapter", &html, None).unwrap();
        assert!(plan.references.is_empty() && plan.remote_images.is_empty());
        assert!(plan.archive.is_none());
        let stats = scope.stats();
        assert_eq!(stats.summary_builds, 1);
        assert_eq!(stats.usage.element_visits, stats.usage.elements);
        assert!(stats.usage.child_inspections <= stats.usage.nodes);
        assert_eq!(stats.usage.work_bytes, 8 * stats.usage.nodes);
        assert_eq!(stats.usage.svg_images, 0);
        assert_eq!(stats.usage.first_match_ordinal, None);
    }
}

#[test]
fn svg_guard_matches_locked_namespace_and_fragment_relationships() {
    let cases = [
        "<svg><image href='figure.png'/></svg>",
        "<svg><foreignObject><math><mrow><image/></mrow></math></foreignObject></svg>",
        "<math><mrow><svg><image/></svg></mrow></math>",
        "<template><svg><image/></svg></template>",
        "<svg><foreignObject><img src='https://fixture.invalid/figure.png'></foreignObject></svg>",
        "<math><mrow><image/></mrow></math>",
        "<svg><image/></svg><svg><image/></svg>",
    ];
    for html in cases {
        let document = Html::parse_document(html);
        let selector = Selector::parse("svg image").unwrap();
        // The reference query is intentionally limited to these tiny trees.
        // The checked production path never calls this descendant selector.
        let expected = document.select(&selector).count();
        let scope = ObservationScope::start(SvgGuardLimits::default());
        assert_eq!(count_svg_images(&document).unwrap(), expected);
        assert_eq!(scope.stats().usage.svg_images, expected);
        assert_eq!(
            scope.stats().usage.element_visits,
            scope.stats().usage.elements
        );
    }
}

#[test]
fn svg_guard_preserves_first_storage_match_after_complete_admission() {
    let document = Html::parse_document(
        "<svg><foreignObject><math><image/></math></foreignObject><image/></svg>",
    );
    let selector = Selector::parse("svg image").unwrap();
    let first = document.select(&selector).next().unwrap();
    let ordinal = document
        .tree
        .nodes()
        .filter_map(ElementRef::wrap)
        .position(|element| element == first)
        .unwrap();
    let scope = ObservationScope::start(SvgGuardLimits::default());
    let matches = count_svg_images(&document).unwrap();
    assert_eq!(matches, 2);
    assert_eq!(scope.stats().usage.first_match_ordinal, Some(ordinal));
}

#[test]
fn svg_guard_exact_work_and_metadata_precede_summary_allocation() {
    let document = Html::parse_document(&mathml(8, 8));
    let baseline = {
        let scope = ObservationScope::start(SvgGuardLimits::default());
        assert_eq!(count_svg_images(&document).unwrap(), 0);
        scope.stats().usage
    };
    let exact = SvgGuardLimits {
        work_bytes: baseline.work_bytes,
        metadata_bytes: baseline.metadata_bytes,
    };
    {
        let scope = ObservationScope::start(exact);
        assert_eq!(count_svg_images(&document).unwrap(), 0);
        assert_eq!(scope.stats().usage, baseline);
        assert_eq!(scope.stats().summary_builds, 1);
    }
    for limits in [
        SvgGuardLimits {
            work_bytes: exact.work_bytes - 1,
            ..exact
        },
        SvgGuardLimits {
            metadata_bytes: exact.metadata_bytes - 1,
            ..exact
        },
    ] {
        let scope = ObservationScope::start(limits);
        resource_error(&count_svg_images(&document).unwrap_err());
        assert_eq!(scope.stats().summary_builds, 0);
        assert_eq!(scope.stats().usage.element_visits, 0);
        assert_eq!(scope.stats().usage.child_inspections, 0);
    }
}

#[test]
fn svg_guard_public_plan_retains_early_unsupported_and_resource_failures() {
    let html = "<svg><image/></svg><img src='http://insecure.invalid/figure.png'>";
    let error = make_plan("book", "chapter", html, None).unwrap_err();
    assert!(matches!(error, SourceError::Unsupported("svg_book_image")));
    let scope = ObservationScope::start(SvgGuardLimits {
        work_bytes: 0,
        ..SvgGuardLimits::default()
    });
    let error = make_plan("book", "chapter", &mathml(4, 4), None).unwrap_err();
    resource_error(&error);
    assert_eq!(scope.stats().summary_builds, 0);
}

#[test]
fn svg_guard_public_plan_keeps_ordinary_and_foreignobject_image_contracts() {
    let archive = "https://fixture.invalid/archive.zip";
    let plan = make_plan(
        "book",
        "chapter",
        "<p>A</p><img src='figure.png'>",
        Some(archive),
    )
    .unwrap();
    assert_eq!(plan.references.len(), 1);
    assert!(plan.archive.is_some());
    assert!(plan.remote_images.is_empty());
    let plan = make_plan(
        "book",
        "chapter",
        "<svg><foreignObject><img src='https://fixture.invalid/figure.png'></foreignObject></svg>",
        None,
    )
    .unwrap();
    assert_eq!(plan.references.len(), 1);
    assert_eq!(plan.remote_images.len(), 1);
    assert!(plan.archive.is_none());
}
