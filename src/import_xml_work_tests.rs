//! Small real EPUBs check all XML consumers before name/namespace work.

use super::*;
use std::{cell::Cell, path::PathBuf};
use xml_work::{Fields, Forecast, Kind, Limits, Usage};

#[derive(Clone, Copy, Debug, Default)]
struct Stage {
    inputs: usize,
    source_bytes: usize,
    fields_seen: usize,
    fields: Fields,
    forecast: Forecast,
    before_fields: Usage,
    admitted: Usage,
    admissions: usize,
    dom_parses: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct Stats {
    stages: [Stage; 4],
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
                "The XML work observation is already active"
            );
            state.set(Some(Observation {
                limits,
                stats: Stats::default(),
            }));
        });
        Self
    }

    fn stats(&self) -> Stats {
        OBSERVATION.with(|state| state.get().expect("The XML work observation ended").stats)
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

fn observe(kind: Kind, operation: impl FnOnce(&mut Stage)) {
    OBSERVATION.with(|state| {
        if let Some(mut observation) = state.get() {
            operation(&mut observation.stats.stages[kind.slot()]);
            state.set(Some(observation));
        }
    });
}

pub(super) fn observe_input(kind: Kind, bytes: usize) {
    observe(kind, |stage| {
        stage.inputs += 1;
        stage.source_bytes = bytes;
    });
}

pub(super) fn observe_input_admitted(kind: Kind, usage: Usage) {
    observe(kind, |stage| stage.before_fields = usage);
}

pub(super) fn observe_fields(kind: Kind, fields: Fields) {
    observe(kind, |stage| {
        stage.fields_seen += 1;
        stage.fields = fields;
    });
}

pub(super) fn observe_forecast(kind: Kind, forecast: Forecast) {
    observe(kind, |stage| stage.forecast = forecast);
}

pub(super) fn observe_admitted(kind: Kind, usage: Usage) {
    observe(kind, |stage| {
        stage.admissions += 1;
        stage.admitted = usage;
    });
}

pub(super) fn observe_dom(kind: Kind) {
    observe(kind, |stage| {
        assert!(
            stage.admissions > 0,
            "The original parser needs complete XML admission"
        );
        stage.dom_parses += 1;
    });
}

const KINDS: [Kind; 4] = [Kind::Container, Kind::Package, Kind::Encryption, Kind::Ncx];
const PREFIX_FIELDS: &str = " common_prefix_abcdefghijkl_a='first' common_prefix_abcdefghijkl_b='second' common_prefix_abcdefghijkl_c='third'";
const BODY: &[u8] =
    b"<html><body><h1>Fallback heading</h1><p>Canonical body &amp; preserved.</p></body></html>";

#[derive(Clone)]
struct Documents {
    xml: [String; 4],
}

impl Documents {
    fn new(kind: Kind, fields: &str, child: &str) -> Self {
        let selected = |candidate| {
            if candidate == kind {
                (fields, child)
            } else {
                ("", "")
            }
        };
        let (container_fields, container_child) = selected(Kind::Container);
        let (package_fields, package_child) = selected(Kind::Package);
        let (encryption_fields, encryption_child) = selected(Kind::Encryption);
        let (ncx_fields, ncx_child) = selected(Kind::Ncx);
        Self {
            xml: [
                format!(
                    "<container{container_fields}>{container_child}<rootfiles><rootfile full-path='OEBPS/book.opf' media-type='application/oebps-package+xml'/></rootfiles></container>"
                ),
                format!(
                    "<package version='2.0'{package_fields}>{package_child}<metadata><title>XML work fixture</title></metadata><manifest><item id='chapter' href='Text/chapter.xhtml' media-type='application/xhtml+xml'/><item id='toc' href='toc.ncx' media-type='application/x-dtbncx+xml'/></manifest><spine toc='toc'><itemref idref='chapter'/></spine></package>"
                ),
                format!(
                    "<encryption{encryption_fields}>{encryption_child}<EncryptedData><CipherData><CipherReference URI='Unused/resource.bin'/></CipherData></EncryptedData></encryption>"
                ),
                format!(
                    "<ncx{ncx_fields}>{ncx_child}<navMap><navPoint><content src='Text/chapter.xhtml'/><navLabel><text>XML quota title</text></navLabel></navPoint></navMap></ncx>"
                ),
            ],
        }
    }

    fn get(&self, kind: Kind) -> &str {
        &self.xml[kind.slot()]
    }
}

fn epub_file(documents: &Documents) -> Result<(tempfile::TempDir, PathBuf, Vec<u8>)> {
    assert!(documents.xml.iter().all(|xml| xml.len() < 12 * 1024));
    let bytes = tests::archive(&[
        ("mimetype", b"application/epub+zip"),
        (
            "META-INF/container.xml",
            documents.get(Kind::Container).as_bytes(),
        ),
        ("OEBPS/book.opf", documents.get(Kind::Package).as_bytes()),
        (
            "META-INF/encryption.xml",
            documents.get(Kind::Encryption).as_bytes(),
        ),
        ("OEBPS/toc.ncx", documents.get(Kind::Ncx).as_bytes()),
        ("OEBPS/Text/chapter.xhtml", BODY),
    ])?;
    assert!(bytes.len() < 16 * 1024);
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("xml-work-fixture.epub");
    std::fs::write(&path, &bytes)?;
    Ok((directory, path, bytes))
}

fn import_fixture(documents: &Documents, limits: Limits) -> Result<(Result<ImportedBook>, Stats)> {
    let (_directory, path, bytes) = epub_file(documents)?;
    let scope = ObservationScope::start(limits);
    let result = import_file(&path);
    assert_eq!(std::fs::read(path)?, bytes);
    Ok((result, scope.stats()))
}

fn assert_import(imported: &ImportedBook, title: &str) {
    assert_eq!(imported.book.title, "XML work fixture");
    assert_eq!(imported.book.source, "local");
    assert_eq!(imported.book.total_chapters, 1);
    assert_eq!(imported.book.cached_chapters, 1);
    assert_eq!(imported.chapters.len(), 1);
    assert_eq!(imported.chapters[0].index, 0);
    assert_eq!(imported.chapters[0].title, title);
    assert!(
        imported.chapters[0]
            .body
            .as_deref()
            .unwrap()
            .contains("Canonical body & preserved.")
    );
}

fn assert_rejected_before_dom(stats: Stats, kind: Kind) {
    let stage = stats.stages[kind.slot()];
    assert_eq!(stage.inputs, 1);
    assert_eq!(stage.fields_seen, 1);
    assert_eq!(stage.admissions, 0);
    assert_eq!(stage.dom_parses, 0);
}

fn parse_target(documents: &Documents, kind: Kind, limits: Limits) -> Result<(Result<()>, Stats)> {
    let scope = ObservationScope::start(limits);
    let mut budget = xml_work::Budget::new();
    let result = if kind == Kind::Ncx {
        ncx_titles_with_xml_budget(
            documents.get(kind),
            "OEBPS/toc.ncx",
            &mut RetainedBudget::new(),
            &mut budget,
        )
        .map(|titles| assert_eq!(titles["OEBPS/Text/chapter.xhtml"], "XML quota title"))
    } else {
        budget.parse(documents.get(kind), kind).map(|document| {
            let expected = match kind {
                Kind::Container => "container",
                Kind::Package => "package",
                Kind::Encryption => "encryption",
                Kind::Ncx => unreachable!(),
            };
            assert_eq!(document.root_element().tag_name().name(), expected);
        })
    };
    Ok((result, scope.stats()))
}

#[test]
fn real_epub_uses_all_four_xml_admission_entries_and_preserves_content() -> Result<()> {
    for kind in KINDS {
        let documents = Documents::new(kind, PREFIX_FIELDS, "");
        let (result, stats) = import_fixture(&documents, Limits::default())?;
        assert_import(&result?, "XML quota title");
        for stage in stats.stages {
            assert_eq!(stage.inputs, 1);
            assert_eq!(stage.admissions, 1);
            assert_eq!(stage.dom_parses, 1);
            assert!(stage.forecast.comparison_bytes > 0);
        }
    }
    Ok(())
}

#[test]
fn real_epub_common_prefix_quota_rejects_each_actual_xml_entry_before_dom() -> Result<()> {
    for kind in KINDS {
        let documents = Documents::new(kind, PREFIX_FIELDS, "");
        let (baseline, stats) = import_fixture(&documents, Limits::default())?;
        assert_import(&baseline?, "XML quota title");
        let stage = stats.stages[kind.slot()];
        assert!(stage.forecast.comparison_bytes > 0);
        // Rebuild the whole import's sticky fee through this actual consumer,
        // including earlier documents and this document's prepaid input scan.
        let mut cumulative = 0usize;
        for earlier in KINDS.iter().take(kind.slot() + 1).copied() {
            let earlier_stage = stats.stages[earlier.slot()];
            assert_eq!(earlier_stage.source_bytes, documents.get(earlier).len());
            cumulative = cumulative
                .checked_add(earlier_stage.source_bytes.checked_mul(3).unwrap())
                .unwrap();
            assert_eq!(earlier_stage.before_fields.work_bytes, cumulative);
            cumulative = cumulative
                .checked_add(earlier_stage.forecast.comparison_bytes)
                .unwrap();
            assert_eq!(earlier_stage.admitted.work_bytes, cumulative);
        }
        assert_eq!(stage.admitted.work_bytes, cumulative);
        let maximum = cumulative.checked_sub(1).unwrap();
        assert!(maximum >= stage.before_fields.work_bytes);
        let (result, rejected) = import_fixture(
            &documents,
            Limits {
                work_bytes: maximum,
                ..Limits::default()
            },
        )?;
        let error = result.unwrap_err();
        assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
        // Container/package/encryption attach an outer anyhow context; inspect
        // the typed inner cause instead of their contextual display strings.
        assert_eq!(
            error.downcast_ref::<ImportLimitExceeded>().unwrap().0,
            "XML name processing exceeds its work allowance",
            "the wrong resource rejected {kind:?}"
        );
        assert_rejected_before_dom(rejected, kind);
        let denied = rejected.stages[kind.slot()];
        assert_eq!(
            denied.before_fields.work_bytes,
            stage.before_fields.work_bytes
        );
        assert_eq!(denied.fields, stage.fields);
        assert_eq!(
            denied.forecast.comparison_bytes,
            stage.forecast.comparison_bytes
        );
        assert_eq!(
            denied
                .before_fields
                .work_bytes
                .checked_add(denied.forecast.comparison_bytes)
                .unwrap(),
            maximum.checked_add(1).unwrap()
        );
        for previous in &rejected.stages[..kind.slot()] {
            assert_eq!(previous.inputs, 1);
            assert_eq!(previous.fields_seen, 1);
            assert_eq!(previous.admissions, 1);
            assert_eq!(previous.dom_parses, 1);
        }
        for later in &rejected.stages[kind.slot() + 1..] {
            assert_eq!(later.dom_parses, 0);
        }
    }
    Ok(())
}

#[test]
fn each_xml_consumer_accepts_exact_work_and_rejects_one_unit_over() -> Result<()> {
    for kind in KINDS {
        let documents = Documents::new(kind, PREFIX_FIELDS, "");
        let (result, baseline) = parse_target(&documents, kind, Limits::default())?;
        result?;
        let required = baseline.stages[kind.slot()].admitted.work_bytes;
        for maximum in [required, required - 1] {
            let (result, stats) = parse_target(
                &documents,
                kind,
                Limits {
                    work_bytes: maximum,
                    ..Limits::default()
                },
            )?;
            if maximum == required {
                result?;
                assert_eq!(stats.stages[kind.slot()].dom_parses, 1);
                assert_eq!(stats.stages[kind.slot()].admitted.work_bytes, required);
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .downcast_ref::<ImportLimitExceeded>()
                        .is_some()
                );
                assert_rejected_before_dom(stats, kind);
            }
        }
    }
    Ok(())
}

#[test]
fn namespace_aliases_and_rebinding_keep_ordinary_xml_semantics() -> Result<()> {
    let fields = " xmlns:p='urn:shared' xmlns:q='urn:shared' p:a='one' q:b='two'";
    let child = "<ignored xmlns:p='urn:rebound' p:c='three'><inner p:d='four'/></ignored>";
    for kind in KINDS {
        let documents = Documents::new(kind, fields, child);
        let (result, stats) = import_fixture(&documents, Limits::default())?;
        assert_import(&result?, "XML quota title");
        let stage = stats.stages[kind.slot()];
        assert_eq!(stage.fields.namespaces, 3);
        assert_eq!(stage.fields.namespace_tags, 2);
        assert!(stage.fields.qualified_attributes >= 4);
        assert_eq!(stage.dom_parses, 1);
    }
    Ok(())
}

#[test]
fn duplicate_expanded_attribute_aliases_keep_the_original_ncx_fallback() -> Result<()> {
    let documents = Documents::new(
        Kind::Ncx,
        " xmlns:p='urn:shared' xmlns:q='urn:shared' p:a='one' q:a='two'",
        "",
    );
    let (result, stats) = import_fixture(&documents, Limits::default())?;
    assert_import(&result?, "Fallback heading");
    assert_eq!(stats.stages[Kind::Ncx.slot()].dom_parses, 1);
    assert_eq!(stats.stages[Kind::Ncx.slot()].admissions, 1);
    Ok(())
}

#[test]
fn unknown_namespaces_and_malformed_fields_leave_format_errors_to_roxmltree() -> Result<()> {
    let documents = Documents::new(Kind::Container, " missing:field='value'", "");
    let (result, stats) = import_fixture(&documents, Limits::default())?;
    assert!(
        result
            .unwrap_err()
            .downcast_ref::<ImportLimitExceeded>()
            .is_none()
    );
    assert_eq!(stats.stages[Kind::Container.slot()].dom_parses, 1);
    for fields in [" missing:field='value'", " malformed=unquoted"] {
        let documents = Documents::new(Kind::Ncx, fields, "");
        let (result, stats) = import_fixture(&documents, Limits::default())?;
        assert_import(&result?, "Fallback heading");
        assert_eq!(stats.stages[Kind::Ncx.slot()].dom_parses, 1);
    }
    Ok(())
}

fn entity_document(value: &str, replacement: &str) -> Documents {
    let mut documents = Documents::new(Kind::Ncx, "", "");
    let content =
        documents.xml[Kind::Ncx.slot()].replacen("<navMap>", &format!("<navMap>{replacement}"), 1);
    documents.xml[Kind::Ncx.slot()] =
        format!("<!DOCTYPE ncx [<!ENTITY generated \"{value}\">]>{content}");
    documents
}

#[test]
fn entity_generated_attributes_reach_the_actual_ncx_work_gate() -> Result<()> {
    let value = "<!-- <fake a='ignored'/> --><?fixture ignored?><![CDATA[<fake b='ignored'/>]]><holder common_prefix_abcdefghijkl_a='first' common_prefix_abcdefghijkl_b='second'/>";
    let documents = entity_document(value, "&generated;&generated;");
    let (result, baseline) = import_fixture(&documents, Limits::default())?;
    assert_import(&result?, "XML quota title");
    let stage = baseline.stages[Kind::Ncx.slot()];
    assert_eq!(stage.fields.attributes, 5);
    assert_eq!(stage.fields.attribute_pairs, 2);
    let (result, stats) = import_fixture(
        &documents,
        Limits {
            work_bytes: stage.admitted.work_bytes - 1,
            ..Limits::default()
        },
    )?;
    assert!(
        result
            .unwrap_err()
            .downcast_ref::<ImportLimitExceeded>()
            .is_some()
    );
    assert_rejected_before_dom(stats, Kind::Ncx);
    Ok(())
}

#[test]
fn entity_generated_namespaces_and_attribute_uri_expansion_are_forecast() -> Result<()> {
    let uri = format!("urn:generated:{}", "r".repeat(48));
    let mut documents = entity_document(
        "<holder xmlns:p='&uri;' p:a='one' p:b='two'><inner xmlns:q='&uri;' q:c='three'/></holder>",
        "&generated;",
    );
    documents.xml[Kind::Ncx.slot()] =
        documents.xml[Kind::Ncx.slot()].replacen("]>", &format!("<!ENTITY uri '{uri}'>]>"), 1);
    let (result, baseline) = import_fixture(&documents, Limits::default())?;
    assert_import(&result?, "XML quota title");
    let stage = baseline.stages[Kind::Ncx.slot()];
    assert_eq!(stage.fields.namespaces, 2);
    assert_eq!(stage.fields.namespace_tags, 2);
    assert!(stage.fields.attribute_replacement >= uri.len());
    assert!(stage.forecast.namespace_uri_bytes > uri.len());
    let (result, stats) = import_fixture(
        &documents,
        Limits {
            work_bytes: stage.admitted.work_bytes - 1,
            ..Limits::default()
        },
    )?;
    assert!(
        result
            .unwrap_err()
            .downcast_ref::<ImportLimitExceeded>()
            .is_some()
    );
    assert_rejected_before_dom(stats, Kind::Ncx);
    Ok(())
}

#[test]
fn attribute_only_entities_never_create_xml_fields_or_namespaces() -> Result<()> {
    let value = "not-a-field='x' xmlns:p='urn:fake'";
    let mut documents = Documents::new(Kind::Ncx, " data='&value;'", "");
    documents.xml[Kind::Ncx.slot()] = format!(
        "<!DOCTYPE ncx [<!ENTITY value \"{value}\">]>{}",
        documents.get(Kind::Ncx)
    );
    let (result, stats) = import_fixture(&documents, Limits::default())?;
    assert_import(&result?, "XML quota title");
    let fields = stats.stages[Kind::Ncx.slot()].fields;
    assert_eq!(fields.attributes, 2);
    assert_eq!(fields.namespaces, 0);
    assert_eq!(fields.attribute_replacement, value.len());
    let document = roxmltree::Document::parse_with_options(
        documents.get(Kind::Ncx),
        roxmltree::ParsingOptions {
            allow_dtd: true,
            nodes_limit: MAX_NCX_NODES,
            entity_resolver: None,
        },
    )?;
    assert_eq!(document.root_element().attribute("data"), Some(value));
    Ok(())
}

#[test]
fn the_four_documents_share_exact_nonrefunded_work_admission() -> Result<()> {
    let documents = Documents::new(Kind::Package, PREFIX_FIELDS, "");
    let (result, baseline) = import_fixture(&documents, Limits::default())?;
    assert_import(&result?, "XML quota title");
    let required = baseline.stages[Kind::Ncx.slot()].admitted.work_bytes;
    for maximum in [required, required - 1] {
        let (result, stats) = import_fixture(
            &documents,
            Limits {
                work_bytes: maximum,
                ..Limits::default()
            },
        )?;
        if maximum == required {
            assert_import(&result?, "XML quota title");
            assert_eq!(stats.stages[Kind::Ncx.slot()].admitted.work_bytes, required);
        } else {
            assert!(
                result
                    .unwrap_err()
                    .downcast_ref::<ImportLimitExceeded>()
                    .is_some()
            );
            assert_rejected_before_dom(stats, Kind::Ncx);
        }
    }
    Ok(())
}

#[test]
fn the_four_documents_share_exact_metadata_forecasts() -> Result<()> {
    let documents = Documents::new(Kind::Package, PREFIX_FIELDS, "");
    let (result, baseline) = import_fixture(&documents, Limits::default())?;
    result?;
    let required = baseline.stages[Kind::Ncx.slot()].admitted.metadata_bytes;
    for maximum in [required, required - 1] {
        let (result, stats) = import_fixture(
            &documents,
            Limits {
                metadata_bytes: maximum,
                ..Limits::default()
            },
        )?;
        if maximum == required {
            assert_import(&result?, "XML quota title");
            assert_eq!(
                stats.stages[Kind::Ncx.slot()].admitted.metadata_bytes,
                required
            );
        } else {
            let error = result.unwrap_err();
            assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
            assert!(error.to_string().contains("XML parser metadata"));
            assert_rejected_before_dom(stats, Kind::Ncx);
        }
    }
    Ok(())
}

#[test]
fn the_four_documents_share_exact_field_count_admission() -> Result<()> {
    let documents = Documents::new(Kind::Package, PREFIX_FIELDS, "");
    let (result, baseline) = import_fixture(&documents, Limits::default())?;
    result?;
    let required = baseline.stages[Kind::Ncx.slot()].admitted.fields;
    for maximum in [required, required - 1] {
        let (result, stats) = import_fixture(
            &documents,
            Limits {
                fields: maximum,
                ..Limits::default()
            },
        )?;
        if maximum == required {
            assert_import(&result?, "XML quota title");
            assert_eq!(stats.stages[Kind::Ncx.slot()].admitted.fields, required);
        } else {
            assert!(
                result
                    .unwrap_err()
                    .downcast_ref::<ImportLimitExceeded>()
                    .is_some()
            );
            assert_rejected_before_dom(stats, Kind::Ncx);
        }
    }
    Ok(())
}

#[test]
fn failed_original_parse_does_not_refund_the_same_xml_budget() -> Result<()> {
    let xml = "<root common_prefix_a='first' common_prefix_a='second'/>";
    let mut baseline = xml_work::Budget::with_limits(Limits::default());
    assert!(baseline.parse(xml, Kind::Container).is_err());
    let required = baseline.usage().work_bytes;
    let scope = ObservationScope::start(Limits::default());
    let mut budget = xml_work::Budget::with_limits(Limits {
        work_bytes: required * 2 - 1,
        ..Limits::default()
    });
    assert!(
        budget
            .parse(xml, Kind::Container)
            .unwrap_err()
            .downcast_ref::<ImportLimitExceeded>()
            .is_none()
    );
    let spent = budget.usage().work_bytes;
    assert_eq!(spent, required);
    assert!(
        budget
            .parse(xml, Kind::Container)
            .unwrap_err()
            .downcast_ref::<ImportLimitExceeded>()
            .is_some()
    );
    assert!(budget.usage().work_bytes > spent);
    assert_eq!(scope.stats().stages[Kind::Container.slot()].dom_parses, 1);
    Ok(())
}

#[test]
fn raw_equals_in_quoted_values_are_priced_before_initial_attribute_capacity() -> Result<()> {
    let mut previous = None;
    for value in ["", "===="] {
        let xml = format!("<root note='{value}'/>");
        let scope = ObservationScope::start(Limits::default());
        let mut budget = xml_work::Budget::new();
        assert_eq!(
            budget
                .parse(&xml, Kind::Container)?
                .root_element()
                .attribute("note"),
            Some(value)
        );
        let metadata = scope.stats().stages[Kind::Container.slot()]
            .forecast
            .metadata_bytes;
        if let Some(previous) = previous {
            assert_eq!(metadata - previous, 4 * 128);
        }
        previous = Some(metadata);
    }
    let xml = "<root note='===='/>";
    let scope = ObservationScope::start(Limits::default());
    let mut budget = xml_work::Budget::with_limits(Limits {
        metadata_bytes: previous.unwrap() - 1,
        ..Limits::default()
    });
    assert!(
        budget
            .parse(xml, Kind::Container)
            .unwrap_err()
            .downcast_ref::<ImportLimitExceeded>()
            .is_some()
    );
    assert_rejected_before_dom(scope.stats(), Kind::Container);
    Ok(())
}

#[test]
fn raw_less_than_in_unused_entity_values_is_priced_before_initial_node_capacity() -> Result<()> {
    let mut required = None;
    let mut target = None;
    for value in ["plain", "<<<<<"] {
        let mut documents = Documents::new(Kind::Ncx, "", "");
        documents.xml[Kind::Ncx.slot()] = format!(
            "<!DOCTYPE ncx [<!ENTITY unused '{value}'>]>{}",
            documents.get(Kind::Ncx)
        );
        let (result, stats) = import_fixture(&documents, Limits::default())?;
        assert_import(&result?, "XML quota title");
        let metadata = stats.stages[Kind::Ncx.slot()].admitted.metadata_bytes;
        if let Some(previous) = required {
            assert_eq!(metadata - previous, 5 * 128);
        }
        required = Some(metadata);
        target = Some(documents);
    }
    let (result, stats) = import_fixture(
        &target.unwrap(),
        Limits {
            metadata_bytes: required.unwrap() - 1,
            ..Limits::default()
        },
    )?;
    assert!(
        result
            .unwrap_err()
            .downcast_ref::<ImportLimitExceeded>()
            .is_some()
    );
    assert_rejected_before_dom(stats, Kind::Ncx);
    Ok(())
}

#[test]
fn field_name_exact_allowance_and_one_over_precede_dom() -> Result<()> {
    let xml = "<root common_prefix_a='first' common_prefix_b='second'/>";
    let required = "common_prefix_a".len();
    for maximum in [required, required - 1] {
        let scope = ObservationScope::start(Limits::default());
        let mut budget = xml_work::Budget::with_limits(Limits {
            name_bytes: maximum,
            ..Limits::default()
        });
        let result = budget.parse(xml, Kind::Container);
        if maximum == required {
            assert_eq!(
                result?.root_element().attribute("common_prefix_a"),
                Some("first")
            );
            assert_eq!(scope.stats().stages[Kind::Container.slot()].dom_parses, 1);
        } else {
            assert!(
                result
                    .unwrap_err()
                    .downcast_ref::<ImportLimitExceeded>()
                    .is_some()
            );
            assert_rejected_before_dom(scope.stats(), Kind::Container);
        }
    }
    Ok(())
}

#[test]
fn known_entity_work_and_projection_metadata_exact_cases_keep_their_semantics() -> Result<()> {
    let xml = "<!DOCTYPE ncx [<!ENTITY e 'abcdefgh'>]><ncx><navPoint><content src='chapter.xhtml'/><navLabel><text>&e;&e;&e;</text></navLabel></navPoint></ncx>";
    let required = 3 * xml.len() + 3 * (1 + 1 + 2 + 3 * 8);
    for maximum in [required, required - 1] {
        let result = ncx_entities::preflight_with_limits(
            xml,
            &RetainedBudget::new(),
            ncx_entities::Limits {
                work_bytes: maximum,
                ..ncx_entities::Limits::default()
            },
        );
        if maximum == required {
            let usage = result?;
            assert_eq!(usage.work_bytes, required);
            assert_eq!(usage.summary_scans, 1);
        } else {
            assert!(
                result
                    .unwrap_err()
                    .downcast_ref::<ImportLimitExceeded>()
                    .is_some()
            );
        }
    }
    let xml = "<ncx><navMap><navPoint><navLabel/></navPoint></navMap></ncx>";
    let required =
        5 * std::mem::size_of::<NcxFirstText<'_>>() + std::mem::size_of::<Vec<NcxFirstText<'_>>>();
    for available in [required, required - 1] {
        let mut retained = RetainedBudget {
            used: MAX_RETAINED_BYTES - available,
        };
        let before = retained.used;
        let result = ncx_titles(xml, "OEBPS/toc.ncx", &mut retained);
        if available == required {
            assert!(result?.is_empty());
        } else {
            assert!(
                result
                    .unwrap_err()
                    .downcast_ref::<ImportLimitExceeded>()
                    .is_some()
            );
        }
        assert_eq!(retained.used, before);
    }
    Ok(())
}

#[test]
fn quoted_comment_cdata_and_pi_lookalikes_do_not_create_fields() -> Result<()> {
    let child = "<!-- <fake xmlns:p='urn:fake' a='x'/> --><![CDATA[<fake xmlns:q='urn:fake' b='y'/>]]><?fixture <fake c='z'?>";
    let documents = Documents::new(Kind::Ncx, " note='not-a-field=&quot;value&quot;'", child);
    let (result, stats) = import_fixture(&documents, Limits::default())?;
    assert_import(&result?, "XML quota title");
    let fields = stats.stages[Kind::Ncx.slot()].fields;
    assert_eq!(fields.namespaces, 0);
    assert_eq!(fields.attributes, 2);
    assert_eq!(stats.stages[Kind::Ncx.slot()].dom_parses, 1);
    Ok(())
}

#[test]
fn valid_bom_unicode_and_prolog_tokens_cannot_hide_late_fields() -> Result<()> {
    let child = format!(
        "<!-- <fake a='ignored'/> --><?fixture ignored?><![CDATA[<fake b='ignored'/>]]><late{PREFIX_FIELDS}/>"
    );
    for kind in [Kind::Container, Kind::Ncx] {
        let mut documents =
            Documents::new(kind, " xmlns:p='urn:unicode' p:name\u{301}='value'", &child);
        let prolog = if kind == Kind::Ncx {
            "<!DOCTYPE ncx PUBLIC '-//NISO//DTD ncx 2005-1//EN' 'urn:fixture' [<!ELEMENT ncx ANY><!ATTLIST ncx ignored CDATA 'value'><!NOTATION ignored SYSTEM 'urn:notation'>]>"
        } else {
            ""
        };
        documents.xml[kind.slot()] = format!(
            "\u{feff}<?xml version='1.0'?><?fixture prolog?><!-- prolog -->{prolog}{}",
            documents.get(kind)
        );
        let (result, baseline) = import_fixture(&documents, Limits::default())?;
        assert_import(&result?, "XML quota title");
        let stage = baseline.stages[kind.slot()];
        assert!(stage.fields.attributes >= 5);
        assert!(stage.fields.attribute_pairs >= 3);
        assert_eq!(stage.dom_parses, 1);
        let (result, rejected) = import_fixture(
            &documents,
            Limits {
                work_bytes: stage.admitted.work_bytes - 1,
                ..Limits::default()
            },
        )?;
        assert!(
            result
                .unwrap_err()
                .downcast_ref::<ImportLimitExceeded>()
                .is_some()
        );
        assert_rejected_before_dom(rejected, kind);
    }
    Ok(())
}

#[test]
fn permissive_entity_eof_groups_receive_global_pair_bounds_before_outer_dom_work() -> Result<()> {
    for (value, later) in [
        (
            "<holder common_prefix_a='first' common_prefix_b='second'",
            "<later common_prefix_c='third'/>",
        ),
        (
            "<holder xmlns:p='urn:ghost' p:a='first'",
            "<later xmlns:q='urn:ghost' q:b='second'/>",
        ),
    ] {
        let replacement = format!("&generated;{later}");
        let documents = entity_document(value, &replacement);
        let (result, baseline) = import_fixture(&documents, Limits::default())?;
        assert_import(&result?, "XML quota title");
        let stage = baseline.stages[Kind::Ncx.slot()];
        assert!(stage.fields.mixed_groups);
        assert!(stage.forecast.attribute_pairs > stage.fields.attribute_pairs);
        let document = roxmltree::Document::parse_with_options(
            documents.get(Kind::Ncx),
            roxmltree::ParsingOptions {
                allow_dtd: true,
                nodes_limit: MAX_NCX_NODES,
                entity_resolver: None,
            },
        )?;
        let element = document
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "later")
            .unwrap();
        if value.contains("xmlns") {
            assert_eq!(element.attribute(("urn:ghost", "a")), Some("first"));
            assert_eq!(element.attribute(("urn:ghost", "b")), Some("second"));
            assert!(stage.forecast.namespace_pairs > stage.fields.namespace_pairs);
        } else {
            assert_eq!(element.attribute("common_prefix_a"), Some("first"));
            assert_eq!(element.attribute("common_prefix_b"), Some("second"));
            assert_eq!(element.attribute("common_prefix_c"), Some("third"));
        }
        let (result, rejected) = import_fixture(
            &documents,
            Limits {
                work_bytes: stage.admitted.work_bytes - 1,
                ..Limits::default()
            },
        )?;
        assert!(
            result
                .unwrap_err()
                .downcast_ref::<ImportLimitExceeded>()
                .is_some()
        );
        assert_rejected_before_dom(rejected, Kind::Ncx);
    }
    Ok(())
}

#[test]
fn locked_leading_colon_names_cannot_hide_valid_late_attribute_work() -> Result<()> {
    let child = format!("<:holder :a='one'{PREFIX_FIELDS}/>");
    for kind in [Kind::Container, Kind::Ncx] {
        let documents = Documents::new(kind, " :early='value'", &child);
        let (result, baseline) = import_fixture(&documents, Limits::default())?;
        assert_import(&result?, "XML quota title");
        let stage = baseline.stages[kind.slot()];
        assert!(stage.fields.attribute_pairs >= 6);
        assert_eq!(stage.dom_parses, 1);
        let (result, rejected) = import_fixture(
            &documents,
            Limits {
                work_bytes: stage.admitted.work_bytes - 1,
                ..Limits::default()
            },
        )?;
        assert!(
            result
                .unwrap_err()
                .downcast_ref::<ImportLimitExceeded>()
                .is_some()
        );
        assert_rejected_before_dom(rejected, kind);
    }
    Ok(())
}
