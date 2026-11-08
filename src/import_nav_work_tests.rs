//! Small navigation projections and public EPUB3 imports exercise complete admission.

use std::{cell::Cell, io::Write};

use super::*;
use nav_work::{Limits, Usage};

#[derive(Clone, Copy, Debug, Default)]
struct Stats {
    preflights: usize,
    requested: Usage,
    admitted: usize,
    usage: Usage,
    projection_allocations: usize,
    projection_elements: usize,
    projection_capacity: usize,
    projection_visits: usize,
    label_visits: usize,
    label_child_inspections: usize,
    copied_label_bytes: usize,
    retained_titles: usize,
}

#[derive(Clone, Copy)]
struct Observation {
    limits: Limits,
    stats: Stats,
}

thread_local! {
    static OBSERVATION: Cell<Option<Observation>> = const { Cell::new(None) };
}

struct ObservationScope;

impl ObservationScope {
    fn start(limits: Limits) -> Self {
        OBSERVATION.with(|state| {
            assert!(
                state.get().is_none(),
                "The navigation observation is already active"
            );
            state.set(Some(Observation {
                limits,
                stats: Stats::default(),
            }));
        });
        Self
    }

    fn stats(&self) -> Stats {
        OBSERVATION.with(|state| state.get().expect("The navigation observation ended").stats)
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

pub(super) fn observe_preflight(usage: Usage) {
    observe(|stats| {
        stats.preflights += 1;
        stats.requested = usage;
    });
}

pub(super) fn observe_projection(elements: usize, capacity: usize) {
    observe(|stats| {
        stats.projection_allocations += 1;
        stats.projection_elements = elements;
        stats.projection_capacity = capacity;
    });
}

pub(super) fn observe_projection_visit() {
    observe(|stats| stats.projection_visits += 1);
}

pub(super) fn observe_admitted(usage: Usage) {
    observe(|stats| {
        stats.admitted += 1;
        stats.usage = usage;
    });
}

pub(super) fn observe_label_visit() {
    observe(|stats| stats.label_visits += 1);
}

pub(super) fn observe_label_child_inspection() {
    observe(|stats| stats.label_child_inspections += 1);
}

pub(super) fn observe_label_copy(bytes: usize) {
    observe(|stats| stats.copied_label_bytes += bytes);
}

pub(super) fn observe_title() {
    observe(|stats| stats.retained_titles += 1);
}

fn nested_svg(depth: usize, href: &str, text: &str) -> String {
    let mut html = String::from("<html><body><nav><svg>");
    for _ in 0..depth {
        html.push_str(&format!("<a href='{href}'>"));
    }
    html.push_str(text);
    for _ in 0..depth {
        html.push_str("</a>");
    }
    html.push_str("</svg></nav></body></html>");
    html
}

fn run(
    html: &str,
    limits: Limits,
    budget: &mut RetainedBudget,
) -> (Result<HashMap<String, String>>, Stats) {
    let scope = ObservationScope::start(limits);
    let result = nav_titles(html, "OPS/nav.xhtml", budget);
    (result, scope.stats())
}

fn resource_error(error: &anyhow::Error) {
    assert!(
        error.downcast_ref::<ImportLimitExceeded>().is_some(),
        "{error:#}"
    );
}

#[test]
fn html_nav_foreign_anchors_keep_candidates_without_css_parent_scans() {
    let html = nested_svg(8, "chapter.xhtml", "Word");
    let (titles, stats) = run(&html, Limits::default(), &mut RetainedBudget::new());
    assert_eq!(
        titles.unwrap().get("OPS/chapter.xhtml").map(String::as_str),
        Some("Word")
    );
    assert_eq!(stats.usage.candidates, 8);
    assert_eq!(stats.projection_visits, 2 * stats.usage.elements);
    assert_eq!(stats.projection_elements, stats.usage.elements);
    assert_eq!(stats.projection_capacity, stats.usage.elements);
    // Only the first nonempty path needs a label walk. Its eight ancestors
    // and one borrowed text child use nine metered visits.
    assert_eq!(stats.label_visits, 9);
    assert_eq!(stats.retained_titles, 1);
}

#[test]
fn html_nav_empty_duplicate_paths_spend_complete_work_before_any_label_walk() {
    let html = nested_svg(8, "chapter.xhtml", " ");
    let (titles, baseline) = run(&html, Limits::default(), &mut RetainedBudget::new());
    assert!(titles.unwrap().is_empty());
    assert_eq!(baseline.usage.candidates, 8);
    assert_eq!(baseline.usage.label_visits, 44);
    assert_eq!(baseline.usage.label_bytes, 8);
    assert_eq!(baseline.label_visits, 44);
    assert_eq!(baseline.retained_titles, 0);

    let mut budget = RetainedBudget::new();
    let before = budget.used;
    let (result, denied) = run(
        &html,
        Limits {
            work_bytes: baseline.usage.work_bytes - 1,
            ..Limits::default()
        },
        &mut budget,
    );
    resource_error(&result.unwrap_err());
    assert_eq!(denied.projection_allocations, 1);
    assert_eq!(denied.admitted, 0);
    assert_eq!((denied.label_visits, denied.copied_label_bytes), (0, 0));
    assert_eq!(budget.used, before);
}

#[test]
fn html_nav_comment_children_are_admitted_before_any_empty_label_walk() {
    let inner = " <!--ignored-->".repeat(12);
    let html = nested_svg(4, "chapter.xhtml", &inner);
    let (titles, baseline) = run(&html, Limits::default(), &mut RetainedBudget::new());
    assert!(titles.unwrap().is_empty());
    assert_eq!(baseline.label_visits, 58);
    assert_eq!(baseline.usage.label_visits, 58);
    assert_eq!(baseline.label_child_inspections, 102);
    assert_eq!(baseline.usage.label_child_inspections, 102);
    assert_eq!(baseline.usage.label_bytes, 48);
    assert_eq!(baseline.retained_titles, 0);

    // A limit covering every old token/text/path charge still omits all of
    // these child inspections. Complete admission must reject before label 1.
    let omitted_child_work = 6 * baseline.usage.label_child_inspections;
    let mut budget = RetainedBudget::new();
    let before = budget.used;
    let (result, denied) = run(
        &html,
        Limits {
            work_bytes: baseline.usage.work_bytes - omitted_child_work,
            ..Limits::default()
        },
        &mut budget,
    );
    resource_error(&result.unwrap_err());
    assert_eq!(denied.admitted, 0);
    assert_eq!(
        (
            denied.label_visits,
            denied.label_child_inspections,
            denied.copied_label_bytes
        ),
        (0, 0, 0)
    );
    assert_eq!(budget.used, before);

    let visible = nested_svg(4, "chapter.xhtml", "A<!--ignored--> B");
    let (titles, stats) = run(&visible, Limits::default(), &mut RetainedBudget::new());
    assert_eq!(
        titles.unwrap().get("OPS/chapter.xhtml").map(String::as_str),
        Some("A B")
    );
    assert!(stats.label_child_inspections <= stats.usage.label_child_inspections);
}

#[test]
fn html_nav_exact_work_and_metadata_allowances_are_admitted() {
    let html = "<nav><a href='a.xhtml'>Alpha</a><a href='b.xhtml'>Beta</a></nav>";
    let (expected, baseline) = run(html, Limits::default(), &mut RetainedBudget::new());
    let exact = Limits {
        work_bytes: baseline.usage.work_bytes,
        metadata_bytes: baseline.usage.metadata_bytes,
    };
    let (result, exact_stats) = run(html, exact, &mut RetainedBudget::new());
    assert_eq!(result.unwrap(), expected.unwrap());
    assert_eq!(exact_stats.admitted, 1);
    assert_eq!(exact_stats.usage, baseline.usage);

    for (limits, allocations) in [
        (
            Limits {
                work_bytes: exact.work_bytes - 1,
                ..exact
            },
            1,
        ),
        (
            Limits {
                metadata_bytes: exact.metadata_bytes - 1,
                ..exact
            },
            0,
        ),
    ] {
        let mut budget = RetainedBudget::new();
        let before = budget.used;
        let (result, denied) = run(html, limits, &mut budget);
        resource_error(&result.unwrap_err());
        assert_eq!(denied.projection_allocations, allocations);
        assert_eq!((denied.label_visits, denied.copied_label_bytes), (0, 0));
        assert_eq!(budget.used, before);
    }
}

#[test]
fn html_nav_temporary_metadata_reservation_preserves_retained_capacity() {
    let html = "<nav><a href='chapter.xhtml'>Stored title</a></nav>";
    let (_, baseline) = run(html, Limits::default(), &mut RetainedBudget::new());
    let mut budget = RetainedBudget {
        used: MAX_RETAINED_BYTES - baseline.usage.metadata_bytes + 1,
    };
    let before = budget.used;
    let (result, denied) = run(html, Limits::default(), &mut budget);
    resource_error(&result.unwrap_err());
    assert_eq!(denied.projection_allocations, 0);
    assert_eq!((denied.label_visits, denied.copied_label_bytes), (0, 0));
    assert_eq!(budget.used, before);
}

#[test]
fn html_nav_text_child_worklist_is_prepaid_before_projection_or_copy() {
    let html = format!(
        "<nav><a href='chapter.xhtml'>{}</a></nav>",
        " <!--separator-->".repeat(12)
    );
    let (result, baseline) = run(&html, Limits::default(), &mut RetainedBudget::new());
    assert!(result.unwrap().is_empty());
    let label_worklist = 2 * baseline.usage.nodes * std::mem::size_of::<AppendElementWork<'_>>();
    let (result, denied) = run(
        &html,
        Limits {
            metadata_bytes: baseline.usage.metadata_bytes - label_worklist,
            ..Limits::default()
        },
        &mut RetainedBudget::new(),
    );
    resource_error(&result.unwrap_err());
    assert_eq!(denied.projection_allocations, 0);
    assert_eq!((denied.label_visits, denied.copied_label_bytes), (0, 0));
}

#[test]
fn html_nav_preserves_spacing_hidden_labels_and_first_nonempty_paths() {
    let html = concat!(
        "<nav><a href='chapter.xhtml'> \r\n </a>",
        "<a href='chapter.xhtml'><span hidden>Hidden</span><b>Visible</b><br>Tail",
        "<span aria-hidden='true'>Secret</span></a><a href='chapter.xhtml'>Later</a>",
        "<a href='second.xhtml' aria-hidden='true'>Hidden root</a>",
        "<a href='second.xhtml'><i>Second</i><script>Hidden source</script></a>",
        "<a href='unicode.xhtml'>\u{4e2d}&nbsp; \u{1f642}</a>",
        "</nav>"
    );
    let (titles, stats) = run(html, Limits::default(), &mut RetainedBudget::new());
    let titles = titles.unwrap();
    assert_eq!(
        titles.get("OPS/chapter.xhtml").map(String::as_str),
        Some("Visible Tail")
    );
    assert_eq!(
        titles.get("OPS/second.xhtml").map(String::as_str),
        Some("Second")
    );
    assert_eq!(
        titles.get("OPS/unicode.xhtml").map(String::as_str),
        Some("\u{4e2d} \u{1f642}")
    );
    assert_eq!(stats.retained_titles, 3);
    assert!(stats.label_visits <= stats.usage.label_visits);
}

#[test]
fn html_nav_matches_any_namespace_types_and_empty_namespace_href() {
    let html = concat!(
        "<svg xmlns:xlink='http://www.w3.org/1999/xlink'><nav>",
        "<a xlink:href='ignored.xhtml'>Ignored</a>",
        "<a href='unicode%20name.xhtml?query#fragment' xlink:href='other.xhtml'>",
        "\u{4e2d}</a></nav></svg>"
    );
    let (titles, stats) = run(html, Limits::default(), &mut RetainedBudget::new());
    assert_eq!(
        titles.unwrap(),
        HashMap::from([("OPS/unicode name.xhtml".into(), "\u{4e2d}".into())])
    );
    assert_eq!(stats.usage.candidates, 1);
}

#[test]
fn html_nav_keeps_selector_storage_order_after_parser_reparenting() {
    let html = concat!(
        "<nav><table><tr><td><a href='chapter.xhtml'>First</a></td></tr>",
        "<a href='chapter.xhtml'>Second</a></table></nav>"
    );
    let document = Html::parse_document(html);
    let selector = Selector::parse("nav a[href]").unwrap();
    // A small reference selector checks the locked parser's node order. The
    // production projection must retain it even after foster reparenting.
    let expected = document
        .select(&selector)
        .map(|element| element.id())
        .collect::<Vec<_>>();
    assert_eq!(expected.len(), 2);
    let mut budget = RetainedBudget::new();
    let projection = nav_work::Projection::new(&document, "OPS/nav.xhtml", &mut budget).unwrap();
    let actual = projection
        .candidates()
        .map(|element| element.id())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    let temporary = projection.metadata_bytes();
    drop(projection);
    budget.used -= temporary;
    let (titles, _) = run(html, Limits::default(), &mut RetainedBudget::new());
    assert_eq!(
        titles.unwrap().get("OPS/chapter.xhtml").map(String::as_str),
        Some("First")
    );
}

const CONTAINER: &[u8] = br#"<?xml version="1.0"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
 <rootfiles><rootfile full-path="OPS/package.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

const PACKAGE: &[u8] = br#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
 <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Navigation fixture</dc:title></metadata>
 <manifest>
  <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
  <item id="first" href="Text/first.xhtml" media-type="application/xhtml+xml"/>
  <item id="second" href="Text/second.xhtml" media-type="application/xhtml+xml"/>
 </manifest>
 <spine><itemref idref="first"/><itemref idref="second"/></spine>
</package>"#;

fn epub(nav: &str) -> Result<tempfile::NamedTempFile> {
    let entries: Vec<(&str, &[u8])> = vec![
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER),
        ("OPS/package.opf", PACKAGE),
        ("OPS/nav.xhtml", nav.as_bytes()),
        (
            "OPS/Text/first.xhtml",
            b"<html><body><h1>First heading</h1><p>First body.</p></body></html>",
        ),
        (
            "OPS/Text/second.xhtml",
            b"<html><body><h1>Second heading</h1><p>Second body.</p></body></html>",
        ),
    ];
    let bytes = tests::archive(&entries)?;
    let mut file = tempfile::Builder::new().suffix(".epub").tempfile()?;
    file.write_all(&bytes)?;
    file.flush()?;
    Ok(file)
}

#[test]
fn html_nav_genuine_epub3_foreign_navigation_titles_are_preserved() -> Result<()> {
    let nav = concat!(
        "<html><body><nav><svg><a href='Text/first.xhtml'>Outer ",
        "<a href='Text/second.xhtml'>\u{4e2d}</a></a></svg></nav></body></html>"
    );
    let file = epub(nav)?;
    let scope = ObservationScope::start(Limits::default());
    let imported = import_file(file.path())?;
    assert_eq!(imported.book.title, "Navigation fixture");
    assert_eq!(imported.chapters.len(), 2);
    assert_eq!(imported.chapters[0].title, "Outer \u{4e2d}");
    assert_eq!(imported.chapters[1].title, "\u{4e2d}");
    assert_eq!(scope.stats().usage.candidates, 2);
    assert_eq!(scope.stats().retained_titles, 2);
    Ok(())
}

#[test]
fn html_nav_genuine_epub3_empty_duplicate_labels_stay_readable() -> Result<()> {
    let nav = nested_svg(8, "Text/first.xhtml", " \r\n\t ");
    let file = epub(&nav)?;
    let scope = ObservationScope::start(Limits::default());
    let imported = import_file(file.path())?;
    assert_eq!(imported.chapters.len(), 2);
    assert_eq!(imported.chapters[0].title, "First heading");
    assert_eq!(imported.chapters[1].title, "Second heading");
    assert_eq!(scope.stats().usage.candidates, 8);
    assert_eq!(scope.stats().retained_titles, 0);
    assert_eq!(scope.stats().label_visits, scope.stats().usage.label_visits);
    Ok(())
}

#[test]
fn html_nav_genuine_epub3_quota_failure_does_not_become_heading_fallback() -> Result<()> {
    let nav = nested_svg(8, "Text/first.xhtml", " ");
    let file = epub(&nav)?;
    let required = {
        let scope = ObservationScope::start(Limits::default());
        import_file(file.path())?;
        scope.stats().usage.work_bytes
    };
    let scope = ObservationScope::start(Limits {
        work_bytes: required - 1,
        ..Limits::default()
    });
    let error = import_file(file.path()).unwrap_err();
    resource_error(&error);
    assert_eq!(scope.stats().admitted, 0);
    assert_eq!(
        (scope.stats().label_visits, scope.stats().copied_label_bytes),
        (0, 0)
    );
    Ok(())
}

#[test]
fn html_nav_genuine_epub3_comment_inspections_preserve_exact_admission() -> Result<()> {
    let inner = " <!--ignored-->".repeat(12);
    let nav = nested_svg(4, "Text/first.xhtml", &inner);
    let file = epub(&nav)?;
    let required = {
        let scope = ObservationScope::start(Limits::default());
        let imported = import_file(file.path())?;
        assert_eq!(imported.chapters[0].title, "First heading");
        assert_eq!(scope.stats().label_visits, 58);
        assert_eq!(scope.stats().label_child_inspections, 102);
        assert_eq!(
            scope.stats().label_child_inspections,
            scope.stats().usage.label_child_inspections
        );
        scope.stats().usage.work_bytes
    };
    {
        let scope = ObservationScope::start(Limits {
            work_bytes: required,
            ..Limits::default()
        });
        let imported = import_file(file.path())?;
        assert_eq!(imported.chapters[0].title, "First heading");
        assert_eq!(imported.chapters[1].title, "Second heading");
        assert_eq!(scope.stats().usage.work_bytes, required);
        assert_eq!(scope.stats().label_visits, 58);
        assert_eq!(scope.stats().label_child_inspections, 102);
    }
    let scope = ObservationScope::start(Limits {
        work_bytes: required - 1,
        ..Limits::default()
    });
    let error = import_file(file.path()).unwrap_err();
    resource_error(&error);
    assert_eq!(scope.stats().admitted, 0);
    assert_eq!(
        (
            scope.stats().label_visits,
            scope.stats().label_child_inspections,
            scope.stats().copied_label_bytes
        ),
        (0, 0, 0)
    );
    Ok(())
}
