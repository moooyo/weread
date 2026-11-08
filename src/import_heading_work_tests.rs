//! Small fallback-heading forests preserve prefix selection and operation admission.

use super::*;
use nav_work::{Limits, Usage};
use std::{cell::Cell, io::Write};

#[derive(Clone, Copy, Debug, Default)]
struct Stats {
    preflights: usize,
    requested: Usage,
    admitted: usize,
    usage: Usage,
    projection_batches: usize,
    projection_elements: usize,
    projection_visits: usize,
    label_visits: usize,
    label_child_inspections: usize,
    copied_label_bytes: usize,
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
                "The heading observation is already active"
            );
            state.set(Some(Observation {
                limits,
                stats: Stats::default(),
            }));
        });
        Self
    }

    fn stats(&self) -> Stats {
        OBSERVATION.with(|state| state.get().unwrap().stats)
    }
}

impl Drop for ObservationScope {
    fn drop(&mut self) {
        OBSERVATION.with(|state| state.set(None));
    }
}

pub(super) fn configured_limits() -> Option<Limits> {
    OBSERVATION.with(|state| state.get().map(|value| value.limits))
}

fn observe(change: impl FnOnce(&mut Stats)) {
    OBSERVATION.with(|state| {
        if let Some(mut observation) = state.get() {
            change(&mut observation.stats);
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

pub(super) fn observe_projection(elements: usize) {
    observe(|stats| {
        stats.projection_batches += 1;
        stats.projection_elements += elements;
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

fn nested_empty(depth: usize, blank: &str, final_heading: bool) -> String {
    let mut html = String::from("<html><body>");
    for _ in 0..depth {
        html.push_str("<h1><span>");
    }
    html.push_str(blank);
    for _ in 0..depth {
        html.push_str("</span></h1>");
    }
    if final_heading {
        html.push_str("<h2>Chosen</h2>");
    }
    html.push_str("<p>Tail body.</p></body></html>");
    let document = Html::parse_document(&html);
    assert_eq!(
        document.select(&Selector::parse("h1").unwrap()).count(),
        depth,
        "The locked parser repaired the intended heading chain",
    );
    if depth > 0 {
        let selector = Selector::parse("h1").unwrap();
        let first = document.select(&selector).next().unwrap();
        assert_eq!(
            first.select(&selector).count(),
            depth - 1,
            "The fixture must retain shared nested heading suffixes",
        );
    }
    html
}

fn run(
    html: &str,
    need_heading: bool,
    limits: Limits,
    retained: &mut RetainedBudget,
) -> (Result<(String, Option<String>)>, Stats, usize) {
    let scope = ObservationScope::start(limits);
    let mut work = nav_work::Budget::new();
    let result = extract_html_with_work(html, need_heading, retained, &mut work);
    (result, scope.stats(), work.used())
}

fn resource_error(error: &anyhow::Error) {
    assert!(
        error.downcast_ref::<ImportLimitExceeded>().is_some(),
        "{error:#}"
    );
}

fn legacy_heading(html: &str) -> Result<Option<String>> {
    // Only small fixtures use the historical repeated-label algorithm.
    let document = Html::parse_document(html);
    let body = document
        .select(&Selector::parse("body").unwrap())
        .next()
        .unwrap();
    for heading in body.select(&Selector::parse("h1,h2,h3").unwrap()) {
        let mut text = String::new();
        append_element(heading, &mut text, MAX_TITLE_BYTES)?;
        let text = compact_whitespace(&text);
        if !text.is_empty() {
            return Ok(Some(text));
        }
    }
    Ok(None)
}

#[test]
fn nested_empty_headings_share_one_summary_forest_and_keep_readable_tail_body() {
    let html = nested_empty(12, &" <!--ignored-->".repeat(16), false);
    let (result, stats, spent) = run(&html, true, Limits::default(), &mut RetainedBudget::new());
    let (body, heading) = result.unwrap();
    assert_eq!(body, "Tail body.");
    assert!(heading.is_none());
    assert_eq!(stats.usage.candidates, 12);
    assert_eq!(stats.projection_batches, 1);
    assert_eq!(stats.projection_visits, 2 * stats.projection_elements);
    assert!(stats.usage.label_visits > stats.projection_elements);
    assert!(stats.usage.label_child_inspections > 16);
    assert_eq!(
        (
            stats.label_visits,
            stats.label_child_inspections,
            stats.copied_label_bytes
        ),
        (0, 0, 0)
    );
    assert_eq!(spent, stats.usage.work_bytes);
}

#[test]
fn required_heading_prefix_is_completely_prepaid_before_the_first_label_copy() {
    let html = nested_empty(8, &" <!--ignored-->".repeat(12), true);
    let (expected, baseline, _) = run(&html, true, Limits::default(), &mut RetainedBudget::new());
    assert_eq!(expected.unwrap().1.as_deref(), Some("Chosen"));
    assert_eq!(baseline.usage.candidates, 9);
    assert_eq!(baseline.projection_batches, 2);
    assert_eq!(baseline.label_visits, 3);
    let exact = Limits {
        work_bytes: baseline.usage.work_bytes,
        metadata_bytes: baseline.usage.metadata_bytes,
    };
    let (result, stats, _) = run(&html, true, exact, &mut RetainedBudget::new());
    assert_eq!(result.unwrap().1.as_deref(), Some("Chosen"));
    assert_eq!(stats.admitted, 1);
    for limits in [
        Limits {
            work_bytes: exact.work_bytes - 1,
            ..exact
        },
        Limits {
            metadata_bytes: exact.metadata_bytes - 1,
            ..exact
        },
    ] {
        let mut retained = RetainedBudget::new();
        let before = retained.used;
        let (result, stats, _) = run(&html, true, limits, &mut retained);
        resource_error(&result.unwrap_err());
        assert_eq!(stats.admitted, 0);
        assert_eq!((stats.label_visits, stats.copied_label_bytes), (0, 0));
        assert_eq!(retained.used, before);
    }
}

#[test]
fn a_first_nonempty_heading_does_not_admit_or_reject_unvisited_later_heading_labels() {
    let first = "<html><body><h1>First</h1><p>Body</p></body></html>";
    let (_, baseline, _) = run(first, true, Limits::default(), &mut RetainedBudget::new());
    let later = format!(
        "<html><body><h1>First</h1><h2>{}</h2><p>Body</p></body></html>",
        "x".repeat(MAX_TITLE_BYTES + 1),
    );
    let (result, stats, _) = run(
        &later,
        true,
        Limits {
            work_bytes: baseline.usage.work_bytes,
            metadata_bytes: baseline.usage.metadata_bytes,
        },
        &mut RetainedBudget::new(),
    );
    assert_eq!(result.unwrap().1.as_deref(), Some("First"));
    assert_eq!(stats.usage.candidates, 1);
    assert_eq!(stats.usage.nodes, baseline.usage.nodes);
    assert_eq!(stats.projection_batches, 1);
    assert_eq!(stats.label_visits, 3);
}

#[test]
fn ordinary_heading_whitespace_unicode_and_root_suppression_match_existing_selection() {
    for html in [
        "<html><body><h1> </h1><h2>A <span>\u{4e2d}\u{1f642}</span></h2><h3>Later</h3><p>Body</p></body></html>",
        "<html><body><h1 hidden><span><h2>Child title</h2></span></h1><p>Body</p></body></html>",
        "<html><body><h1 aria-hidden='true'>Hidden</h1><h2><script>Skip</script><span aria-hidden='true'>Skip</span>Visible</h2><p>Body</p></body></html>",
        "<html><body><svg><h1>Foreign context</h1></svg><h2>Later</h2><p>Body</p></body></html>",
        "<html><body><h1 style='display:none'>Existing style behavior</h1><p>Body</p></body></html>",
    ] {
        let expected = legacy_heading(html).unwrap();
        let (result, _, _) = run(html, true, Limits::default(), &mut RetainedBudget::new());
        assert_eq!(result.unwrap().1, expected);
    }
    let html = nested_empty(8, " \u{a0}\u{2003}<!--empty-->", false);
    let (body, heading) = extract_html(&html, true).unwrap();
    assert_eq!(body, "Tail body.");
    assert!(heading.is_none());
    let (body, heading) = extract_html(
        "<html><body><h1 hidden><span><h2>Child title</h2></span></h1><p>Body</p></body></html>",
        true,
    )
    .unwrap();
    assert_eq!(body, "Body");
    assert_eq!(heading.as_deref(), Some("Child title"));
}

#[test]
fn raw_title_and_temporary_metadata_refusals_never_become_heading_none() {
    let oversized = format!("<h1>{}</h1><p>Body</p>", " ".repeat(MAX_TITLE_BYTES));
    let (result, stats, _) = run(
        &oversized,
        true,
        Limits::default(),
        &mut RetainedBudget::new(),
    );
    resource_error(&result.unwrap_err());
    assert_eq!((stats.label_visits, stats.copied_label_bytes), (0, 0));
    let html = "<h1>Title</h1><p>Body</p>";
    let (_, baseline, _) = run(html, true, Limits::default(), &mut RetainedBudget::new());
    let mut retained = RetainedBudget {
        used: MAX_RETAINED_BYTES - baseline.usage.metadata_bytes + 1,
    };
    let before = retained.used;
    let (result, stats, _) = run(html, true, Limits::default(), &mut retained);
    resource_error(&result.unwrap_err());
    assert_eq!(stats.projection_batches, 0);
    assert_eq!((stats.label_visits, stats.copied_label_bytes), (0, 0));
    assert_eq!(retained.used, before);
    let (result, _, spent) = run(
        html,
        false,
        Limits {
            work_bytes: 0,
            metadata_bytes: 0,
        },
        &mut RetainedBudget::new(),
    );
    assert!(result.unwrap().1.is_none());
    assert_eq!(spent, 0);
    let (result, _, spent) = run(
        "<p>Body</p>",
        true,
        Limits {
            work_bytes: 0,
            metadata_bytes: 0,
        },
        &mut RetainedBudget::new(),
    );
    assert_eq!(result.unwrap(), (String::from("Body"), None::<String>));
    assert_eq!(spent, 0);
}

#[test]
fn navigation_and_repeated_heading_operations_share_work_without_temporary_refunds() {
    let nav = "<nav><a href='chapter.xhtml'> </a></nav>";
    let html = "<h1>Title</h1><p>Body</p>";
    let mut retained = RetainedBudget::new();
    let mut calibration = nav_work::Budget::new();
    assert!(
        nav_titles_with_work(nav, "OPS/nav.xhtml", &mut retained, &mut calibration)
            .unwrap()
            .is_empty()
    );
    let nav_fee = calibration.used();
    let (_, heading, _) = run(html, true, Limits::default(), &mut RetainedBudget::new());
    let scope = ObservationScope::start(Limits {
        work_bytes: nav_fee + heading.usage.work_bytes - 1,
        ..Limits::default()
    });
    let mut work = nav_work::Budget::new();
    let mut retained = RetainedBudget::new();
    let before = retained.used;
    assert!(
        nav_titles_with_work(nav, "OPS/nav.xhtml", &mut retained, &mut work)
            .unwrap()
            .is_empty()
    );
    assert_eq!(retained.used, before);
    resource_error(&extract_html_with_work(html, true, &mut retained, &mut work).unwrap_err());
    assert_eq!(scope.stats().admitted, 0);
    assert_eq!(scope.stats().label_visits, 0);
    assert!(work.used() >= nav_fee);
    assert_eq!(retained.used, before);
}

fn epub(html: &str, repetitions: usize) -> Vec<u8> {
    let container = br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OPS/package.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
    let spine = "<itemref idref='chapter'/>".repeat(repetitions);
    let package = format!(
        "<package xmlns='http://www.idpf.org/2007/opf' version='3.0'><metadata><title>Heading fixture</title></metadata><manifest><item id='chapter' href='chapter.xhtml' media-type='application/xhtml+xml'/></manifest><spine>{spine}</spine></package>",
    );
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("mimetype", &b"application/epub+zip"[..]),
        ("META-INF/container.xml", &container[..]),
        ("OPS/package.opf", package.as_bytes()),
        ("OPS/chapter.xhtml", html.as_bytes()),
    ] {
        writer
            .start_file(
                name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    assert!(bytes.len() < 32 * 1024);
    bytes
}

#[test]
fn real_no_navigation_epub_uses_nested_empty_heading_fallback_and_preserves_tail_text() {
    let directory = tempfile::tempdir().unwrap();
    let html = nested_empty(12, &" <!--ignored-->".repeat(16), false);
    let path = directory.path().join("heading-fixture.epub");
    std::fs::write(&path, epub(&html, 1)).unwrap();
    let scope = ObservationScope::start(Limits::default());
    let imported = import_file(&path).unwrap();
    assert_eq!(imported.chapters.len(), 1);
    assert_eq!(imported.chapters[0].title, "chapter");
    assert_eq!(imported.chapters[0].body.as_deref(), Some("Tail body."));
    assert_eq!(scope.stats().usage.candidates, 12);
    assert_eq!(scope.stats().projection_batches, 1);
    assert_eq!(scope.stats().copied_label_bytes, 0);
}

#[test]
fn real_repeated_spine_work_refusal_returns_no_partial_import_or_old_storage_changes() {
    let directory = tempfile::tempdir().unwrap();
    let html = nested_empty(6, " <!--ignored-->", true);
    let one = directory.path().join("one.epub");
    let two = directory.path().join("two.epub");
    std::fs::write(&one, epub(&html, 1)).unwrap();
    std::fs::write(&two, epub(&html, 2)).unwrap();
    let first = ObservationScope::start(Limits::default());
    assert_eq!(import_file(&one).unwrap().chapters[0].title, "Chosen");
    let fee = first.stats().usage.work_bytes;
    drop(first);
    let store = crate::store::Store::open(directory.path().join("old.sqlite")).unwrap();
    store
        .upsert_book(&crate::domain::Book::new("old-book", "Stored book"))
        .unwrap();
    store
        .upsert_chapter(&Chapter {
            id: "old-uid".into(),
            book_id: "old-book".into(),
            index: 0,
            title: "Stored title".into(),
            body: Some("Stored body.".into()),
            ..Chapter::default()
        })
        .unwrap();
    let before_book = store.book("old-book").unwrap();
    let before_chapter = store.chapter("old-book", 0).unwrap();
    let scope = ObservationScope::start(Limits {
        work_bytes: 2 * fee - 1,
        ..Limits::default()
    });
    resource_error(&import_file(&two).unwrap_err());
    assert_eq!(scope.stats().admitted, 1);
    assert_eq!(scope.stats().copied_label_bytes, "Chosen".len() + 2);
    assert_eq!(store.book("old-book").unwrap(), before_book);
    assert_eq!(store.chapter("old-book", 0).unwrap(), before_chapter);
    assert_eq!(store.books().unwrap().len(), 1);
}
