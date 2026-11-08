//! Small NCX projections and real EPUB imports measure admitted navigation work.

use super::*;
use std::{cell::Cell, path::PathBuf};

#[derive(Clone, Copy, Debug, Default)]
struct NcxWorkStats {
    nodes: usize,
    requested_node_visits: usize,
    requested_projection_bytes: usize,
    reserved_node_visits: usize,
    reserved_projection_bytes: usize,
    projection_allocations: usize,
    projection_nodes: usize,
    projection_capacity: usize,
    node_visits: usize,
    text_element_reads: usize,
    admitted_labels: usize,
    admitted_label_work: usize,
    copied_label_bytes: usize,
    retained_titles: usize,
}

#[derive(Clone, Copy)]
struct Observation {
    limits: NcxWorkLimits,
    stats: NcxWorkStats,
}

thread_local! {
    static OBSERVATION: Cell<Option<Observation>> = const { Cell::new(None) };
}

struct ObservationScope;

impl ObservationScope {
    fn start(limits: NcxWorkLimits) -> Self {
        OBSERVATION.with(|state| {
            assert!(
                state.get().is_none(),
                "The NCX observation is already active"
            );
            state.set(Some(Observation {
                limits,
                stats: NcxWorkStats::default(),
            }));
        });
        Self
    }

    fn stats(&self) -> NcxWorkStats {
        OBSERVATION.with(|state| state.get().expect("The NCX observation ended").stats)
    }
}

impl Drop for ObservationScope {
    fn drop(&mut self) {
        OBSERVATION.with(|state| state.set(None));
    }
}

pub(super) fn configured_limits() -> Option<NcxWorkLimits> {
    OBSERVATION.with(|state| state.get().map(|observation| observation.limits))
}

fn observe(operation: impl FnOnce(&mut NcxWorkStats)) {
    OBSERVATION.with(|state| {
        if let Some(mut observation) = state.get() {
            operation(&mut observation.stats);
            state.set(Some(observation));
        }
    });
}

pub(super) fn observe_preflight(nodes: usize, visits: usize, bytes: usize) {
    observe(|stats| {
        stats.nodes = nodes;
        stats.requested_node_visits = visits;
        stats.requested_projection_bytes = bytes;
    });
}

pub(super) fn observe_reservation(visits: usize, bytes: usize) {
    observe(|stats| {
        stats.reserved_node_visits = visits;
        stats.reserved_projection_bytes = bytes;
    });
}

pub(super) fn observe_projection(nodes: usize, capacity: usize) {
    observe(|stats| {
        stats.projection_allocations += 1;
        stats.projection_nodes = nodes;
        stats.projection_capacity = capacity;
    });
}

pub(super) fn observe_visit() {
    observe(|stats| stats.node_visits += 1);
}

pub(super) fn observe_text_read() {
    observe(|stats| stats.text_element_reads += 1);
}

pub(super) fn observe_label_admission(bytes: usize, fee: usize) {
    observe(|stats| {
        assert_eq!(fee, bytes * 3);
        stats.admitted_labels += 1;
        stats.admitted_label_work += fee;
    });
}

pub(super) fn observe_label_copy(bytes: usize) {
    observe(|stats| stats.copied_label_bytes += bytes);
}

pub(super) fn observe_title() {
    observe(|stats| stats.retained_titles += 1);
}

fn nested_pairs(depth: usize, label: Option<&str>, targets: bool) -> String {
    let mut xml = String::from("<ncx><navMap>");
    for ordinal in 0..depth {
        xml.push_str("<navPoint>");
        if targets {
            xml.push_str(&format!("<content src='target-{ordinal}.xhtml'/>"));
        }
        xml.push_str("<navLabel>");
    }
    if let Some(label) = label {
        xml.push_str("<text>");
        xml.push_str(label);
        xml.push_str("</text>");
    }
    for _ in 0..depth {
        xml.push_str("</navLabel></navPoint>");
    }
    xml.push_str("</navMap></ncx>");
    xml
}

fn assert_preflight_rejection(stats: NcxWorkStats) {
    assert!(stats.nodes > 0 && stats.requested_node_visits > 0);
    assert!(stats.requested_projection_bytes > 0);
    assert_eq!(stats.reserved_node_visits, 0);
    assert_eq!(stats.reserved_projection_bytes, 0);
    assert_eq!(stats.projection_allocations, 0);
    assert_eq!(stats.projection_nodes, 0);
    assert_eq!(stats.node_visits, 0);
    assert_eq!(stats.text_element_reads, 0);
    assert_eq!(stats.admitted_labels, 0);
    assert_eq!(stats.copied_label_bytes, 0);
    assert_eq!(stats.retained_titles, 0);
}

#[test]
fn nested_ncx_labels_without_text_or_content_have_linear_node_work() -> Result<()> {
    let mut earlier = None;
    for depth in [32, 64] {
        let xml = nested_pairs(depth, None, false);
        assert!(xml.len() < 4096);
        let scope = ObservationScope::start(NcxWorkLimits::default());
        let mut budget = RetainedBudget::new();
        let before = budget.used;
        let titles = ncx_titles(&xml, "OEBPS/toc.ncx", &mut budget)?;
        let stats = scope.stats();
        assert!(titles.is_empty());
        assert_eq!(stats.nodes, 2 * depth + 3);
        assert_eq!(stats.projection_allocations, 1);
        assert_eq!(stats.projection_nodes, stats.nodes);
        assert_eq!(stats.projection_capacity, stats.nodes);
        assert!(stats.node_visits >= 2 * stats.nodes);
        assert!(stats.node_visits <= 5 * stats.nodes);
        assert!(stats.node_visits <= stats.reserved_node_visits);
        assert_eq!(stats.admitted_labels, 0);
        assert_eq!(stats.copied_label_bytes, 0);
        assert_eq!(stats.retained_titles, 0);
        assert_eq!(budget.used, before);
        if let Some(previous) = earlier {
            assert!(stats.node_visits > previous);
            assert!(stats.node_visits < previous * 2);
        }
        earlier = Some(stats.node_visits);
    }
    Ok(())
}

#[test]
fn ncx_node_work_exact_allowance_precedes_projection_and_title_copying() -> Result<()> {
    let xml = nested_pairs(8, None, false);
    let nodes = roxmltree::Document::parse(&xml)?.descendants().len();
    let required = nodes * NCX_NODE_VISITS_PER_NODE;
    for allowance in [required, required - 1] {
        let scope = ObservationScope::start(NcxWorkLimits {
            node_visits: allowance,
            ..NcxWorkLimits::default()
        });
        let mut budget = RetainedBudget::new();
        let before = budget.used;
        let result = ncx_titles(&xml, "OEBPS/toc.ncx", &mut budget);
        let stats = scope.stats();
        if allowance == required {
            assert!(result?.is_empty());
            assert_eq!(stats.reserved_node_visits, required);
            assert_eq!(stats.projection_allocations, 1);
            assert!(stats.node_visits > 0);
        } else {
            let error = result.unwrap_err();
            assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
            assert!(error.to_string().contains("NCX navigation node work"));
            assert_preflight_rejection(stats);
        }
        assert_eq!(budget.used, before);
    }
    Ok(())
}

#[test]
fn ncx_projection_metadata_is_reserved_before_its_only_allocation() -> Result<()> {
    let xml = nested_pairs(8, None, false);
    let nodes = roxmltree::Document::parse(&xml)?.descendants().len();
    let required = nodes * std::mem::size_of::<NcxFirstText<'_>>()
        + std::mem::size_of::<Vec<NcxFirstText<'_>>>();
    for available in [required, required - 1] {
        let scope = ObservationScope::start(NcxWorkLimits::default());
        let mut budget = RetainedBudget {
            used: MAX_RETAINED_BYTES - available,
        };
        let before = budget.used;
        let result = ncx_titles(&xml, "OEBPS/toc.ncx", &mut budget);
        let stats = scope.stats();
        if available == required {
            assert!(result?.is_empty());
            assert_eq!(stats.reserved_projection_bytes, required);
            assert_eq!(stats.projection_allocations, 1);
            assert!(stats.node_visits > 0);
        } else {
            assert!(
                result
                    .unwrap_err()
                    .downcast_ref::<ImportLimitExceeded>()
                    .is_some()
            );
            assert_preflight_rejection(stats);
        }
        assert_eq!(budget.used, before);
    }
    Ok(())
}

const FIRST_TEXT: &str = "First e\u{301} & label";
const SECOND_TEXT: &str = "Second \u{1f319} label";
const FIRST_HTML: &[u8] =
    b"<html><body><h1>First fallback heading</h1><p>First body &amp; preserved.</p></body></html>";
const SECOND_HTML: &[u8] =
    b"<html><body><h1>Second fallback heading</h1><p>Second body preserved.</p></body></html>";
const PACKAGE: &[u8] = br#"<package version='2.0'><metadata><title>NCX work fixture</title></metadata><manifest><item id='first' href='Text/first.xhtml' media-type='application/xhtml+xml'/><item id='second' href='Text/second.xhtml' media-type='application/xhtml+xml'/><item id='ncx' href='toc.ncx' media-type='application/x-dtbncx+xml'/></manifest><spine toc='ncx'><itemref idref='second'/><itemref idref='first'/></spine></package>"#;

fn semantic_ncx() -> String {
    format!(
        "<ncx xmlns:n='urn:fixture'><navMap>\
        <navPoint><content src='Text/first.xhtml#old'/><content src='Text/second.xhtml'/>\
        <navLabel>Ignore this character node<wrap><text> First e\u{301} &amp; label </text></wrap>\
        <text>Wrong later text</text></navLabel><navLabel><text>Wrong second label</text></navLabel></navPoint>\
        <navPoint><content src='Text/first.xhtml#new'/><navLabel><text>Wrong duplicate</text></navLabel></navPoint>\
        <navPoint><content src='Text/second.xhtml'/><navLabel><n:text> {SECOND_TEXT} </n:text></navLabel></navPoint>\
        <navPoint><content/><content src='missing-source.xhtml'/><navLabel><text>Wrong second content</text></navLabel></navPoint>\
        <navPoint><content src='missing-first-label.xhtml'/><navLabel/><navLabel><text>Wrong later label</text></navLabel></navPoint>\
        <navPoint><content src='missing-first-text.xhtml'/><navLabel><text/><text>Wrong later text</text></navLabel></navPoint>\
        <navPoint><content src='nontext-first-child.xhtml'/><navLabel><text><span/>Wrong later character text</text><text>Wrong later element</text></navLabel></navPoint>\
        <navPoint><content src='character-nodes.xhtml'/><navLabel>Character nodes are not text elements</navLabel></navPoint>\
        </navMap></ncx>"
    )
}

#[test]
fn ncx_first_child_and_first_text_element_semantics_remain_exact() -> Result<()> {
    let scope = ObservationScope::start(NcxWorkLimits::default());
    let mut budget = RetainedBudget::new();
    let titles = ncx_titles(&semantic_ncx(), "OEBPS/toc.ncx", &mut budget)?;
    assert_eq!(
        titles,
        HashMap::from([
            ("OEBPS/Text/first.xhtml".into(), FIRST_TEXT.into()),
            ("OEBPS/Text/second.xhtml".into(), SECOND_TEXT.into()),
        ])
    );
    let stats = scope.stats();
    assert_eq!(stats.admitted_labels, 2);
    assert_eq!(stats.retained_titles, 2);
    assert!(stats.copied_label_bytes > FIRST_TEXT.len() + SECOND_TEXT.len());
    assert!(stats.node_visits > 0 && stats.node_visits <= stats.reserved_node_visits);
    Ok(())
}

#[test]
fn shared_ncx_labels_charge_empty_and_retained_text_before_copying() -> Result<()> {
    let depth = 8;
    for label in ["  \t  ", "Shared title"] {
        let xml = nested_pairs(depth, Some(label), true);
        let fee = label.len() * 3;
        for allowance in [depth * fee, depth * fee - 1] {
            let scope = ObservationScope::start(NcxWorkLimits {
                label_bytes: allowance,
                ..NcxWorkLimits::default()
            });
            let mut budget = RetainedBudget::new();
            let result = ncx_titles(&xml, "OEBPS/toc.ncx", &mut budget);
            let stats = scope.stats();
            let retained = !label.trim().is_empty();
            if allowance == depth * fee {
                let titles = result?;
                assert_eq!(titles.len(), if retained { depth } else { 0 });
                if retained {
                    assert!(titles.values().all(|title| title == label));
                }
                assert_eq!(stats.admitted_labels, depth);
                assert_eq!(stats.copied_label_bytes, depth * label.len());
                assert_eq!(stats.retained_titles, if retained { depth } else { 0 });
            } else {
                let error = result.unwrap_err();
                assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
                assert!(error.to_string().contains("NCX navigation label work"));
                assert_eq!(stats.admitted_labels, depth - 1);
                assert_eq!(stats.copied_label_bytes, (depth - 1) * label.len());
                assert_eq!(stats.retained_titles, if retained { depth - 1 } else { 0 });
            }
            assert_eq!(stats.admitted_label_work, stats.admitted_labels * fee);
            assert!(stats.admitted_label_work <= allowance);
            assert!(stats.node_visits > 0 && stats.node_visits <= stats.reserved_node_visits);
            assert_eq!(stats.projection_allocations, 1);
            assert_eq!(stats.text_element_reads, 1);
        }
    }
    Ok(())
}

fn epub_file(ncx: &str) -> Result<(tempfile::TempDir, PathBuf, Vec<u8>)> {
    let bytes = tests::archive(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", tests::container()),
        ("OEBPS/book.opf", PACKAGE),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/Text/first.xhtml", FIRST_HTML),
        ("OEBPS/Text/second.xhtml", SECOND_HTML),
    ])?;
    assert!(bytes.len() < 16 * 1024);
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("fixture.epub");
    std::fs::write(&path, &bytes)?;
    Ok((directory, path, bytes))
}

#[test]
fn epub2_ncx_projection_preserves_unicode_titles_body_and_reading_order() -> Result<()> {
    let ncx = format!(
        "<?xml version='1.0'?><!DOCTYPE ncx PUBLIC '-//NISO//DTD ncx 2005-1//EN' 'http://www.daisy.org/z3986/2005/ncx-2005-1.dtd'>{}",
        semantic_ncx()
    );
    let (_directory, path, bytes) = epub_file(&ncx)?;
    let scope = ObservationScope::start(NcxWorkLimits::default());
    let imported = import_file(&path)?;
    assert_eq!(imported.book.title, "NCX work fixture");
    assert_eq!(imported.book.source, "local");
    assert_eq!(imported.book.total_chapters, 2);
    assert_eq!(imported.book.cached_chapters, 2);
    assert_eq!(imported.chapters[0].title, SECOND_TEXT);
    assert_eq!(imported.chapters[1].title, FIRST_TEXT);
    assert!(
        imported.chapters[0]
            .body
            .as_deref()
            .unwrap()
            .contains("Second body preserved.")
    );
    assert!(
        imported.chapters[1]
            .body
            .as_deref()
            .unwrap()
            .contains("First body & preserved.")
    );
    assert!(imported.chapters.iter().all(|chapter| {
        chapter.cached && chapter.content_status == crate::domain::ContentStatus::Full
    }));
    assert_eq!(scope.stats().projection_allocations, 1);
    assert_eq!(scope.stats().retained_titles, 2);
    assert_eq!(std::fs::read(&path)?, bytes);
    Ok(())
}

#[test]
fn epub2_ncx_quota_failure_cannot_fall_back_or_materialize_navigation() -> Result<()> {
    let ncx = nested_pairs(8, Some("Shared title"), true).replacen(
        "target-0.xhtml",
        "Text/first.xhtml",
        1,
    );
    let nodes = roxmltree::Document::parse(&ncx)?.descendants().len();
    let (_directory, path, bytes) = epub_file(&ncx)?;
    for limits in [
        NcxWorkLimits {
            node_visits: nodes * NCX_NODE_VISITS_PER_NODE - 1,
            ..NcxWorkLimits::default()
        },
        NcxWorkLimits {
            label_bytes: 0,
            ..NcxWorkLimits::default()
        },
    ] {
        let scope = ObservationScope::start(limits);
        let error = import_file(&path).unwrap_err();
        assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
        let stats = scope.stats();
        if limits.label_bytes == 0 {
            assert!(error.to_string().contains("NCX navigation label work"));
            assert_eq!(stats.projection_allocations, 1);
            assert!(stats.node_visits > 0);
            assert_eq!(stats.admitted_labels, 0);
            assert_eq!(stats.copied_label_bytes, 0);
            assert_eq!(stats.retained_titles, 0);
        } else {
            assert!(error.to_string().contains("NCX navigation node work"));
            assert_preflight_rejection(stats);
        }
        assert_eq!(std::fs::read(&path)?, bytes);
    }
    Ok(())
}

#[test]
fn epub2_missing_and_malformed_ncx_keep_the_legacy_heading_fallback() -> Result<()> {
    let nested = nested_pairs(16, None, false);
    let invalid_path = "<ncx><navPoint><content src='https://invalid.example/chapter.xhtml'/><navLabel><text>Unused title</text></navLabel></navPoint></ncx>";
    for ncx in ["<ncx>", "<ncx/>", nested.as_str(), invalid_path] {
        let (_directory, path, bytes) = epub_file(ncx)?;
        let scope = ObservationScope::start(NcxWorkLimits::default());
        let imported = import_file(&path)?;
        assert_eq!(imported.chapters.len(), 2);
        assert_eq!(imported.chapters[0].title, "Second fallback heading");
        assert_eq!(imported.chapters[1].title, "First fallback heading");
        assert_eq!(scope.stats().retained_titles, 0);
        if ncx != "<ncx>" {
            assert!(scope.stats().node_visits > 0);
            assert_eq!(scope.stats().projection_allocations, 1);
        }
        assert_eq!(std::fs::read(&path)?, bytes);
    }
    Ok(())
}

#[test]
fn unrequested_and_duplicate_ncx_labels_keep_deferred_title_errors() -> Result<()> {
    let long = "T".repeat(MAX_TITLE_BYTES + 1);
    let xml = format!(
        "<ncx><navMap>\
        <navPoint><content src='chapter.xhtml'/><navLabel><text>Kept title</text></navLabel></navPoint>\
        <navPoint><content src='chapter.xhtml#duplicate'/><navLabel><text>{long}</text></navLabel></navPoint>\
        <navPoint><content/><content src='ignored.xhtml'/><navLabel><text>{long}</text></navLabel></navPoint>\
        <navPoint><navLabel><text>{long}</text></navLabel></navPoint>\
        </navMap></ncx>"
    );
    assert!(xml.len() < 64 * 1024);
    let scope = ObservationScope::start(NcxWorkLimits::default());
    let mut budget = RetainedBudget::new();
    assert_eq!(
        ncx_titles(&xml, "OEBPS/toc.ncx", &mut budget)?,
        HashMap::from([("OEBPS/chapter.xhtml".into(), "Kept title".into())]),
    );
    assert_eq!(scope.stats().admitted_labels, 1);
    assert_eq!(scope.stats().admitted_label_work, 3 * "Kept title".len());
    assert_eq!(scope.stats().copied_label_bytes, "Kept title".len());
    assert_eq!(scope.stats().retained_titles, 1);
    Ok(())
}
