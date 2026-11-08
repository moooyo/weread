//! Small real EPUBs exercise NCX admission before DOM and navigation copies.

use super::*;
use ncx_entities::{Limits, Usage};
use std::{cell::Cell, path::PathBuf};

#[derive(Clone, Copy, Debug, Default)]
struct Stats {
    preflights: usize,
    admitted: usize,
    rejected: usize,
    dom_parses: usize,
    navigation_copy_bytes: usize,
    usage: Usage,
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
                "The NCX entity observation is already active"
            );
            state.set(Some(Observation {
                limits,
                stats: Stats::default(),
            }));
        });
        Self
    }

    fn stats(&self) -> Stats {
        OBSERVATION.with(|state| state.get().expect("The NCX entity observation ended").stats)
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

pub(super) fn observe_preflight_start() {
    observe(|stats| stats.preflights += 1);
}

pub(super) fn observe_preflight_finish(usage: Usage, admitted: bool) {
    observe(|stats| {
        stats.usage = usage;
        if admitted {
            stats.admitted += 1;
        } else {
            stats.rejected += 1;
        }
    });
}

pub(super) fn observe_dom_parse() {
    observe(|stats| {
        assert!(
            stats.admitted > 0,
            "DOM parsing needs complete entity admission"
        );
        stats.dom_parses += 1;
    });
}

pub(super) fn observe_navigation_copy(bytes: usize) {
    observe(|stats| stats.navigation_copy_bytes += bytes);
}

const PACKAGE: &[u8] = br#"<package version='2.0'><metadata><title>NCX entity fixture</title></metadata><manifest><item id='chapter' href='Text/chapter.xhtml' media-type='application/xhtml+xml'/><item id='toc' href='toc.ncx' media-type='application/x-dtbncx+xml'/></manifest><spine toc='toc'><itemref idref='chapter'/></spine></package>"#;
const BODY: &[u8] =
    b"<html><body><h1>Fallback heading</h1><p>Canonical body &amp; preserved.</p></body></html>";

fn navigation(label: &str) -> String {
    format!(
        "<ncx><navMap><navPoint><content src='Text/chapter.xhtml'/><navLabel><text>{label}</text></navLabel></navPoint></navMap></ncx>"
    )
}

fn internal(declarations: &str, content: &str) -> String {
    format!("<!DOCTYPE ncx [{declarations}]>{content}")
}

fn repeated(label_references: usize) -> String {
    internal(
        "<!ENTITY e 'abcdefgh'>",
        &navigation(&"&e;".repeat(label_references)),
    )
}

fn epub_file(ncx: &str) -> Result<(tempfile::TempDir, PathBuf, Vec<u8>)> {
    assert!(
        ncx.len() < 12 * 1024,
        "The NCX regression must remain small"
    );
    let bytes = tests::archive(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", tests::container()),
        ("OEBPS/book.opf", PACKAGE),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/Text/chapter.xhtml", BODY),
    ])?;
    assert!(bytes.len() < 16 * 1024);
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("entity-fixture.epub");
    std::fs::write(&path, &bytes)?;
    Ok((directory, path, bytes))
}

fn assert_import(imported: &ImportedBook, title: &str) {
    assert_eq!(imported.book.title, "NCX entity fixture");
    assert_eq!(imported.book.source, "local");
    assert_eq!(imported.book.total_chapters, 1);
    assert_eq!(imported.book.cached_chapters, 1);
    assert_eq!(imported.chapters.len(), 1);
    assert_eq!(imported.chapters[0].title, title);
    assert!(
        imported.chapters[0]
            .body
            .as_deref()
            .unwrap()
            .contains("Canonical body & preserved.")
    );
    assert!(imported.chapters[0].cached);
    assert_eq!(
        imported.chapters[0].content_status,
        crate::domain::ContentStatus::Full
    );
}

fn assert_before_dom(stats: Stats) {
    assert_eq!(stats.preflights, 1);
    assert_eq!(stats.admitted, 0);
    assert_eq!(stats.rejected, 1);
    assert_eq!(stats.dom_parses, 0);
    assert_eq!(stats.navigation_copy_bytes, 0);
}

#[test]
fn epub2_external_public_and_system_doctypes_preserve_unicode_navigation() -> Result<()> {
    for doctype in [
        "<!DOCTYPE ncx PUBLIC '-//NISO//DTD ncx 2005-1//EN' 'http://www.daisy.org/z3986/2005/ncx-2005-1.dtd'>",
        "<!DOCTYPE ncx SYSTEM 'http://www.daisy.org/z3986/2005/ncx-2005-1.dtd'>",
    ] {
        let ncx = format!(
            "<?xml version='1.0' encoding='UTF-8' standalone='yes'?>{doctype}{}",
            navigation("First e\u{301} &amp; \u{1f319}")
        );
        let (_directory, path, bytes) = epub_file(&ncx)?;
        let scope = ObservationScope::start(Limits::default());
        let imported = import_file(&path)?;
        assert_import(&imported, "First e\u{301} & \u{1f319}");
        let stats = scope.stats();
        assert_eq!(stats.dom_parses, 1);
        assert_eq!(stats.usage.definitions, 0);
        assert_eq!(stats.usage.metadata_bytes, 0);
        assert_eq!(std::fs::read(path)?, bytes);
    }
    Ok(())
}

#[test]
fn epub2_internal_literal_and_character_entities_preserve_titles() -> Result<()> {
    let ncx = internal(
        "<!ENTITY chapter\u{b7}name 'Hello &#x1f319; &amp; e&#769;'>",
        &navigation("&chapter\u{b7}name;"),
    );
    let (_directory, path, bytes) = epub_file(&ncx)?;
    let scope = ObservationScope::start(Limits::default());
    assert_import(&import_file(&path)?, "Hello \u{1f319} & e\u{301}");
    let stats = scope.stats();
    assert_eq!(stats.dom_parses, 1);
    assert_eq!(stats.usage.root_references, 1);
    assert_eq!(stats.usage.summary_scans, 1);
    assert_eq!(std::fs::read(path)?, bytes);
    Ok(())
}

#[test]
fn forward_duplicate_and_builtin_entity_names_keep_first_match() -> Result<()> {
    let ncx = internal(
        "<!ENTITY outer '&inner; &amp; kept'><!ENTITY inner 'First'><!ENTITY inner 'Wrong'><!ENTITY amp 'Wrong amp'>",
        &navigation("&outer;"),
    );
    let scope = ObservationScope::start(Limits::default());
    let titles = ncx_titles(&ncx, "OEBPS/toc.ncx", &mut RetainedBudget::new())?;
    assert_eq!(titles["OEBPS/Text/chapter.xhtml"], "First & kept");
    let stats = scope.stats();
    assert_eq!(stats.usage.declarations, 4);
    assert_eq!(stats.usage.definitions, 4);
    assert_eq!(stats.usage.summary_scans, 2);
    assert_eq!(stats.dom_parses, 1);
    Ok(())
}

#[test]
fn small_markup_entities_preserve_navigation_and_attribute_expansion() -> Result<()> {
    let ncx = internal(
        "<!ENTITY point \"<navPoint><content src='&path;'/><navLabel><text>&title;</text></navLabel></navPoint>\"><!ENTITY path 'Text/chapter.xhtml'><!ENTITY title 'Markup \u{1f319}'>",
        "<ncx><navMap>&point;</navMap></ncx>",
    );
    let (_directory, path, _) = epub_file(&ncx)?;
    let scope = ObservationScope::start(Limits::default());
    assert_import(&import_file(&path)?, "Markup \u{1f319}");
    assert_eq!(scope.stats().usage.summary_scans, 3);
    assert_eq!(scope.stats().usage.root_references, 1);
    assert_eq!(scope.stats().dom_parses, 1);
    Ok(())
}

#[test]
fn comments_cdata_and_pi_do_not_expand_reference_lookalikes() -> Result<()> {
    let ncx = internal(
        "<!-- <!ENTITY phantom '&unused;'> --><?fixture &unused;?><!ENTITY unused '&unused;'><!ENTITY visible '<![CDATA[&unused;]]>'>",
        &navigation("&visible;").replace(
            "<navMap>",
            "<navMap><!-- &unused; --><ignored><![CDATA[&unused;]]></ignored><?fixture &unused;?>",
        ),
    );
    let scope = ObservationScope::start(Limits::default());
    let titles = ncx_titles(&ncx, "OEBPS/toc.ncx", &mut RetainedBudget::new())?;
    assert_eq!(titles["OEBPS/Text/chapter.xhtml"], "&unused;");
    let stats = scope.stats();
    assert_eq!(stats.usage.declarations, 2);
    assert_eq!(stats.usage.root_references, 1);
    assert_eq!(stats.usage.summary_scans, 1);
    assert_eq!(stats.dom_parses, 1);
    Ok(())
}

#[test]
fn repeated_root_references_reject_before_dom_or_navigation_copy() -> Result<()> {
    let ncx = repeated(3);
    let scope = ObservationScope::start(Limits {
        expanded_bytes: ncx.len() + 3 * 8 - 1,
        ..Limits::default()
    });
    let mut budget = RetainedBudget::new();
    let before = budget.used;
    let error = ncx_titles(&ncx, "OEBPS/toc.ncx", &mut budget).unwrap_err();
    assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
    assert!(error.to_string().contains("entity expansion"));
    assert_before_dom(scope.stats());
    assert_eq!(scope.stats().usage.summary_scans, 1);
    assert_eq!(budget.used, before);
    Ok(())
}

#[test]
fn attribute_references_reject_before_dom() -> Result<()> {
    let ncx = internal(
        "<!ENTITY e 'abcdefgh'>",
        &navigation("Kept").replace("<ncx>", "<ncx data='&e;&e;&e;'>"),
    );
    let scope = ObservationScope::start(Limits {
        expanded_bytes: ncx.len() + 3 * 8 - 1,
        ..Limits::default()
    });
    let error = ncx_titles(&ncx, "OEBPS/toc.ncx", &mut RetainedBudget::new()).unwrap_err();
    assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
    assert_before_dom(scope.stats());
    assert_eq!(scope.stats().usage.summary_scans, 1);
    Ok(())
}

#[test]
fn unused_element_references_reject_before_dom() -> Result<()> {
    let ncx = internal(
        "<!ENTITY e 'abcdefgh'>",
        &navigation("Kept").replace("<navMap>", "<navMap><ignored>&e;&e;&e;</ignored>"),
    );
    let scope = ObservationScope::start(Limits {
        expanded_bytes: ncx.len() + 3 * 8 - 1,
        ..Limits::default()
    });
    let error = ncx_titles(&ncx, "OEBPS/toc.ncx", &mut RetainedBudget::new()).unwrap_err();
    assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
    assert_before_dom(scope.stats());
    Ok(())
}

#[test]
fn expansion_byte_exact_allowance_precedes_dom_and_copy() -> Result<()> {
    let ncx = repeated(3);
    let required = ncx.len() + 3 * 8;
    for maximum in [required, required - 1] {
        let scope = ObservationScope::start(Limits {
            expanded_bytes: maximum,
            ..Limits::default()
        });
        let result = ncx_titles(&ncx, "OEBPS/toc.ncx", &mut RetainedBudget::new());
        let stats = scope.stats();
        if maximum == required {
            assert_eq!(result?["OEBPS/Text/chapter.xhtml"], "abcdefgh".repeat(3));
            assert_eq!(stats.usage.expanded_bytes, required);
            assert_eq!(stats.dom_parses, 1);
            assert_eq!(stats.navigation_copy_bytes, 24);
        } else {
            assert!(
                result
                    .unwrap_err()
                    .downcast_ref::<ImportLimitExceeded>()
                    .is_some()
            );
            assert_before_dom(stats);
        }
    }
    Ok(())
}

#[test]
fn entity_lookup_work_exact_allowance_and_one_over() -> Result<()> {
    let ncx = repeated(3);
    // Three source passes, one-byte name/table entry, two lookup-name passes,
    // and three replacement-byte passes for each actual root reference.
    let required = 3 * ncx.len() + 3 * (1 + 1 + 2 + 3 * 8);
    for maximum in [required, required - 1] {
        let scope = ObservationScope::start(Limits {
            work_bytes: maximum,
            ..Limits::default()
        });
        let result = ncx_titles(&ncx, "OEBPS/toc.ncx", &mut RetainedBudget::new());
        let stats = scope.stats();
        if maximum == required {
            assert_eq!(result?["OEBPS/Text/chapter.xhtml"], "abcdefgh".repeat(3));
            assert_eq!(stats.usage.work_bytes, required);
            assert_eq!(stats.dom_parses, 1);
        } else {
            let error = result.unwrap_err();
            assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
            assert!(error.to_string().contains("entity processing"));
            assert_before_dom(stats);
        }
    }
    Ok(())
}

#[test]
fn entity_metadata_exact_and_remaining_retained_allowance() -> Result<()> {
    let ncx = repeated(1);
    let required =
        ncx_entities::preflight_with_limits(&ncx, &RetainedBudget::new(), Limits::default())?
            .metadata_bytes;
    assert!(required > 0 && required < 16 * 1024);
    for available in [required, required - 1] {
        let scope = ObservationScope::start(Limits::default());
        let mut budget = RetainedBudget {
            used: MAX_RETAINED_BYTES - available,
        };
        let before = budget.used;
        let result = ncx_titles(&ncx, "OEBPS/toc.ncx", &mut budget);
        let stats = scope.stats();
        if available == required {
            assert_eq!(result?["OEBPS/Text/chapter.xhtml"], "abcdefgh");
            assert_eq!(stats.usage.metadata_bytes, required);
            assert_eq!(stats.dom_parses, 1);
            assert!(budget.used > before);
            assert!(budget.used < MAX_RETAINED_BYTES);
        } else {
            let error = result.unwrap_err();
            assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
            assert!(error.to_string().contains("preflight metadata"));
            assert_before_dom(stats);
            assert_eq!(budget.used, before);
        }
    }
    Ok(())
}

#[test]
fn declaration_and_entity_name_limits_reject_before_dom() -> Result<()> {
    for (ncx, limits) in [
        (
            internal(
                "<!ENTITY e 'First'><!ENTITY e 'Second'>",
                &navigation("&e;"),
            ),
            Limits {
                declarations: 2,
                ..Limits::default()
            },
        ),
        (
            internal("<!ENTITY longer 'First'>", &navigation("&longer;")),
            Limits {
                name_bytes: 6,
                ..Limits::default()
            },
        ),
    ] {
        let scope = ObservationScope::start(limits);
        assert_eq!(
            ncx_titles(&ncx, "OEBPS/toc.ncx", &mut RetainedBudget::new())?["OEBPS/Text/chapter.xhtml"],
            "First"
        );
        assert_eq!(scope.stats().dom_parses, 1);
    }
    for (ncx, limits) in [
        (
            internal(
                "<!ENTITY e 'First'><!ENTITY e 'Second'>",
                &navigation("&e;"),
            ),
            Limits {
                declarations: 1,
                ..Limits::default()
            },
        ),
        (
            internal("<!ENTITY longer 'First'>", &navigation("&longer;")),
            Limits {
                name_bytes: 5,
                ..Limits::default()
            },
        ),
        (
            internal("<!ENTITY e 'First'>", &navigation("&unknown;")),
            Limits {
                name_bytes: 5,
                ..Limits::default()
            },
        ),
    ] {
        let scope = ObservationScope::start(limits);
        let error = ncx_titles(&ncx, "OEBPS/toc.ncx", &mut RetainedBudget::new()).unwrap_err();
        assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
        assert_before_dom(scope.stats());
    }
    Ok(())
}

#[test]
fn nested_replacements_are_memoized_in_each_context() -> Result<()> {
    let ncx = internal(
        "<!ENTITY leaf 'ab'><!ENTITY middle '&leaf;&leaf;'><!ENTITY top '&middle;&middle;'>",
        &navigation("&top;&top;").replace("<ncx>", "<ncx data='&top;&top;'>"),
    );
    let scope = ObservationScope::start(Limits::default());
    let titles = ncx_titles(&ncx, "OEBPS/toc.ncx", &mut RetainedBudget::new())?;
    assert_eq!(titles["OEBPS/Text/chapter.xhtml"], "ab".repeat(8));
    let stats = scope.stats();
    assert_eq!(stats.usage.summary_scans, 6);
    assert_eq!(stats.usage.root_references, 4);
    assert_eq!(stats.dom_parses, 1);
    Ok(())
}

fn depth_declarations(depth: usize) -> String {
    let mut declarations = String::from("<!ENTITY e0 'Title'>");
    for index in 1..depth {
        declarations.push_str(&format!("<!ENTITY e{index} '&e{};'>", index - 1));
    }
    declarations
}

#[test]
fn cycles_unknown_and_excess_depth_keep_ordinary_epub_fallback() -> Result<()> {
    for ncx in [
        internal("<!ENTITY a '&b;'><!ENTITY b '&a;'>", &navigation("&a;")),
        internal("<!ENTITY e 'Unused'>", &navigation("&missing;")),
        internal(&depth_declarations(11), &navigation("&e10;")),
    ] {
        let (_directory, path, bytes) = epub_file(&ncx)?;
        let scope = ObservationScope::start(Limits::default());
        assert_import(&import_file(&path)?, "Fallback heading");
        assert_before_dom(scope.stats());
        assert_eq!(std::fs::read(path)?, bytes);
    }
    Ok(())
}

#[test]
fn locked_entity_nesting_reference_boundaries_preserve_small_valid_output() -> Result<()> {
    let ncx = internal(&depth_declarations(10), &navigation("&e9;"));
    {
        let scope = ObservationScope::start(Limits::default());
        assert_eq!(
            ncx_titles(&ncx, "OEBPS/toc.ncx", &mut RetainedBudget::new())?["OEBPS/Text/chapter.xhtml"],
            "Title"
        );
        assert_eq!(scope.stats().dom_parses, 1);
    }
    for references in [255, 256] {
        let declarations = format!(
            "<!ENTITY leaf 'x'><!ENTITY many '{}'>",
            "&leaf;".repeat(references)
        );
        let ncx = internal(&declarations, &navigation("&many;"));
        let (_directory, path, _) = epub_file(&ncx)?;
        let scope = ObservationScope::start(Limits::default());
        let imported = import_file(&path)?;
        if references == 255 {
            assert_import(&imported, &"x".repeat(255));
            assert_eq!(scope.stats().dom_parses, 1);
        } else {
            assert_import(&imported, "Fallback heading");
            assert_before_dom(scope.stats());
        }
    }
    Ok(())
}

#[test]
fn real_epub_resource_rejections_cannot_fall_back_or_materialize_navigation() -> Result<()> {
    for content in [
        navigation("&e;&e;&e;&e;"),
        navigation("Kept").replace("<ncx>", "<ncx data='&e;&e;&e;&e;'>"),
        navigation("Kept").replace("<navMap>", "<navMap><ignored>&e;&e;&e;&e;</ignored>"),
    ] {
        let ncx = internal("<!ENTITY e 'abcdefgh'>", &content);
        let (_directory, path, bytes) = epub_file(&ncx)?;
        let scope = ObservationScope::start(Limits {
            expanded_bytes: ncx.len() + 4 * 8 - 1,
            ..Limits::default()
        });
        let error = import_file(&path).unwrap_err();
        assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
        assert_before_dom(scope.stats());
        assert_eq!(std::fs::read(path)?, bytes);
    }
    Ok(())
}

#[test]
fn malformed_prolog_and_reference_cases_keep_heading_fallback() -> Result<()> {
    for ncx in [
        "<!DOCTYPE ncx [<!ENTITY e 'unclosed>]><ncx/>",
        "<!DOCTYPE ncx PUBLIC 'public-only'><ncx/>",
        "<!DOCTYPE ncx [<!ENTITY e'bad'>]><ncx/>",
        "<!DOCTYPE ncx [<!ENTITY e 'Title'>%e;]><ncx/>",
        "<!DOCTYPE ncx [<!-- malformed--comment -->]><ncx/>",
        "<ncx><navPoint><navLabel><text>&#X41;</text></navLabel></navPoint></ncx>",
        "<ncx>",
    ] {
        let (_directory, path, bytes) = epub_file(ncx)?;
        let scope = ObservationScope::start(Limits::default());
        assert_import(&import_file(&path)?, "Fallback heading");
        assert_eq!(scope.stats().navigation_copy_bytes, 0);
        assert_eq!(std::fs::read(path)?, bytes);
    }
    Ok(())
}

#[test]
fn unused_cycles_external_definitions_and_parameter_literals_remain_deferred() -> Result<()> {
    let ncx = internal(
        "<!ENTITY unused '&unused;'><!ENTITY external SYSTEM 'urn:fixture'><!ENTITY % parameter 'Parameter title'>",
        &navigation("&parameter;"),
    );
    let (_directory, path, _) = epub_file(&ncx)?;
    let scope = ObservationScope::start(Limits::default());
    assert_import(&import_file(&path)?, "Parameter title");
    let stats = scope.stats();
    assert_eq!(stats.usage.declarations, 3);
    assert_eq!(stats.usage.definitions, 2);
    assert_eq!(stats.usage.summary_scans, 1);
    assert_eq!(stats.dom_parses, 1);
    Ok(())
}

#[test]
fn external_ids_and_ignored_dtd_declarations_do_not_bind_lookalikes() -> Result<()> {
    let ncx = format!(
        "<!DOCTYPE ncx SYSTEM \"urn:fixture:<!ENTITY ghost 'Ignored'>\" [<!ELEMENT ncx ANY><!ATTLIST ncx note CDATA '&ghost;'><?fixture &ghost;?><!-- <!ENTITY wrong '&ghost;'> -->]>{}",
        navigation("Title &amp; phrase"),
    );
    let scope = ObservationScope::start(Limits::default());
    let titles = ncx_titles(&ncx, "OEBPS/toc.ncx", &mut RetainedBudget::new())?;
    assert_eq!(titles["OEBPS/Text/chapter.xhtml"], "Title & phrase");
    let stats = scope.stats();
    assert_eq!(stats.usage.declarations, 0);
    assert_eq!(stats.usage.definitions, 0);
    assert_eq!(stats.usage.root_references, 0);
    assert_eq!(stats.usage.metadata_bytes, 0);
    assert_eq!(stats.dom_parses, 1);
    Ok(())
}
