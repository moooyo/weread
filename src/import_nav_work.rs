//! Shared HTML label forecasts for navigation and fallback heading admission.

use std::collections::HashMap;

use anyhow::{Result, ensure};
use scraper::{ElementRef, Html, Selector};

use super::{
    AppendElementWork, ImportLimitExceeded, MAX_PATH_BYTES, MAX_RETAINED_BYTES, MAX_TITLE_BYTES,
    MAX_WORK_BYTES, RetainedBudget, append_element_spacing, append_element_suppressed,
    append_element_with_observer, check_title_length, compact_whitespace,
};

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub work_bytes: usize,
    pub metadata_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            work_bytes: MAX_WORK_BYTES as usize,
            metadata_bytes: MAX_RETAINED_BYTES,
        }
    }
}

/// Navigation and fallback headings spend one import operation's work allowance.
/// Temporary metadata may be released, but admitted work is never refunded.
pub(super) struct Budget {
    used: usize,
    limit: usize,
}

impl Budget {
    pub(super) fn new() -> Self {
        #[cfg(test)]
        let limit = super::heading_work_tests::configured_limits()
            .map_or(MAX_WORK_BYTES as usize, |limits| limits.work_bytes);
        #[cfg(not(test))]
        let limit = MAX_WORK_BYTES as usize;
        Self {
            used: 0,
            limit: limit.min(MAX_WORK_BYTES as usize),
        }
    }

    fn admit(&mut self, amount: usize) -> Result<()> {
        let next = add(self.used, amount)?;
        ensure!(
            next <= self.limit,
            ImportLimitExceeded("HTML navigation processing exceeds its work allowance")
        );
        self.used = next;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn used(&self) -> usize {
        self.used
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Usage {
    pub nodes: usize,
    pub elements: usize,
    pub candidates: usize,
    pub label_visits: usize,
    pub label_child_inspections: usize,
    pub label_bytes: usize,
    pub path_bytes: usize,
    pub work_bytes: usize,
    pub metadata_bytes: usize,
}

#[derive(Clone, Copy, Default)]
struct Label {
    visits: usize,
    child_inspections: usize,
    bytes: usize,
    has_content: bool,
}

struct Entry<'a> {
    element: ElementRef<'a>,
    candidate: bool,
    label: Label,
}

enum Visit<'a> {
    Enter {
        element: ElementRef<'a>,
        nav_ancestor: bool,
    },
    Exit(ElementRef<'a>),
}

/// The immutable DOM and its insertion order stay borrowed. Parent chains
/// are summarized with one forest walk; no ancestor selector or label text
/// is stored in this table. The counters are logical forecasts, not total RSS.
pub(super) struct Projection<'a> {
    entries: Vec<Entry<'a>>,
    metadata_bytes: usize,
}

impl<'a> Projection<'a> {
    #[cfg(test)]
    pub(super) fn new(
        document: &'a Html,
        nav_path: &str,
        budget: &mut RetainedBudget,
    ) -> Result<Self> {
        #[cfg(test)]
        let limits = super::nav_work_tests::configured_limits().unwrap_or_default();
        #[cfg(not(test))]
        let limits = Limits::default();
        Self::with_limits(document, nav_path, budget, limits)
    }

    pub(super) fn new_with_work(
        document: &'a Html,
        nav_path: &str,
        budget: &mut RetainedBudget,
        work: &mut Budget,
    ) -> Result<Self> {
        #[cfg(test)]
        let limits = super::nav_work_tests::configured_limits().unwrap_or_default();
        #[cfg(not(test))]
        let limits = Limits::default();
        Self::with_limits_and_work(document, nav_path, budget, limits, work)
    }

    #[cfg(test)]
    pub(super) fn with_limits(
        document: &'a Html,
        nav_path: &str,
        budget: &mut RetainedBudget,
        limits: Limits,
    ) -> Result<Self> {
        Self::with_limits_and_work(document, nav_path, budget, limits, &mut Budget::new())
    }

    fn with_limits_and_work(
        document: &'a Html,
        nav_path: &str,
        budget: &mut RetainedBudget,
        limits: Limits,
        work: &mut Budget,
    ) -> Result<Self> {
        let ceiling = Limits::default();
        let limits = Limits {
            work_bytes: limits.work_bytes.min(ceiling.work_bytes),
            metadata_bytes: limits.metadata_bytes.min(ceiling.metadata_bytes),
        };
        let mut usage = Usage::default();
        // This count pass allocates nothing. Charge before examining each row;
        // the six units also prepay initialization and the Enter/Exit walk.
        for node in document.tree.nodes() {
            admit_work(&mut usage, 6, limits, work)?;
            usage.nodes = add(usage.nodes, 1)?;
            if node.value().is_element() {
                usage.elements = add(usage.elements, 1)?;
            }
        }
        let id = document.tree.root().id();
        let map_slot = add(std::mem::size_of_val(&(id, 0usize)), 2)?;
        usage.metadata_bytes = metadata_forecast(usage, map_slot, false)?;
        observe_preflight(usage);
        if usage.metadata_bytes > limits.metadata_bytes {
            return Err(ImportLimitExceeded(
                "HTML navigation projection exceeds its metadata allowance",
            )
            .into());
        }
        budget.reserve(&[usage.metadata_bytes])?;
        let temporary_bytes = usage.metadata_bytes;
        let result = (|| -> Result<Self> {
            let mut entries = Vec::with_capacity(usage.elements);
            let mut by_id = HashMap::with_capacity(usage.elements);
            let mut pending = Vec::with_capacity(add(mul(usage.elements, 2)?, 4)?);
            // Html.select uses tree.nodes(), including element-parent forests
            // separated by document/fragment nodes. Preserve that output order,
            // rather than switching it to preorder after parser reparenting.
            for node in document.tree.nodes() {
                let Some(element) = ElementRef::wrap(node) else {
                    continue;
                };
                by_id.insert(element.id(), entries.len());
                entries.push(Entry {
                    element,
                    candidate: false,
                    label: Label::default(),
                });
                if element.parent().and_then(ElementRef::wrap).is_none() {
                    pending.push(Visit::Enter {
                        element,
                        nav_ancestor: false,
                    });
                }
            }
            observe_projection(entries.len(), entries.capacity());
            while let Some(visit) = pending.pop() {
                observe_projection_visit();
                match visit {
                    Visit::Enter {
                        element,
                        nav_ancestor,
                    } => {
                        let index = *by_id
                            .get(&element.id())
                            .expect("An immutable navigation element has an index");
                        // The locked selector's unprefixed types have any
                        // namespace. Element.attr uses the empty namespace,
                        // exactly like the original [href] existence test.
                        entries[index].candidate = nav_ancestor
                            && element.value().name() == "a"
                            && element.value().attr("href").is_some()
                            && element.parent().is_some();
                        let child_nav = nav_ancestor || element.value().name() == "nav";
                        pending.push(Visit::Exit(element));
                        // Suppressed ancestors still own selectable anchors;
                        // suppression applies from each selected label root.
                        for child in element.children().rev().filter_map(ElementRef::wrap) {
                            pending.push(Visit::Enter {
                                element: child,
                                nav_ancestor: child_nav,
                            });
                        }
                    }
                    Visit::Exit(element) => {
                        let index = *by_id
                            .get(&element.id())
                            .expect("An immutable navigation element has an index");
                        let label = summarize_label(
                            element,
                            |child| {
                                Ok(entries[*by_id
                                    .get(&child.id())
                                    .expect("An immutable navigation child has an index")]
                                .label)
                            },
                            |_| Ok(false),
                        )?;
                        entries[index].label = label;
                    }
                }
            }
            // Every candidate, including empty labels and later duplicate
            // paths, passes complete admission before the first label walk.
            for entry in entries.iter().filter(|entry| entry.candidate) {
                usage.candidates = add(usage.candidates, 1)?;
                usage.label_visits = add(usage.label_visits, entry.label.visits)?;
                usage.label_child_inspections =
                    add(usage.label_child_inspections, entry.label.child_inspections)?;
                usage.label_bytes = add(usage.label_bytes, entry.label.bytes)?;
                let href = entry.element.value().attr("href").unwrap();
                let path_bytes = add(href.len(), nav_path.len())?;
                usage.path_bytes = add(usage.path_bytes, path_bytes)?;
                let fee = sum(&[
                    mul(6, entry.label.visits)?,
                    mul(6, entry.label.child_inspections)?,
                    mul(3, entry.label.bytes)?,
                    mul(4, path_bytes)?,
                ])?;
                admit_work(&mut usage, fee, limits, work)?;
            }
            observe_admitted(usage);
            Ok(Self {
                entries,
                metadata_bytes: temporary_bytes,
            })
        })();
        if result.is_err() {
            // All local tables/work frames were dropped with the closure.
            // Only their temporary metadata reservation can be returned.
            budget.used -= temporary_bytes;
        }
        result
    }

    pub(super) fn metadata_bytes(&self) -> usize {
        self.metadata_bytes
    }

    pub(super) fn candidates(&self) -> impl Iterator<Item = ElementRef<'a>> + '_ {
        self.entries
            .iter()
            .filter(|entry| entry.candidate)
            .map(|entry| entry.element)
    }
}

fn metadata_forecast(usage: Usage, map_slot: usize, heading: bool) -> Result<usize> {
    let row = sum(&[
        std::mem::size_of::<Entry<'_>>(),
        mul(2, std::mem::size_of::<Visit<'_>>())?,
        mul(2, map_slot)?,
    ])?;
    let mut fixed = sum(&[
        std::mem::size_of::<Projection<'_>>(),
        std::mem::size_of::<HashMap<usize, usize>>(),
        mul(4, map_slot)?,
        mul(4, std::mem::size_of::<Visit<'_>>())?,
        mul(4, std::mem::size_of::<AppendElementWork<'_>>())?,
        mul(8, std::mem::size_of::<String>())?,
        mul(4, MAX_TITLE_BYTES)?,
        mul(6, MAX_PATH_BYTES)?,
        mul(MAX_PATH_BYTES, std::mem::size_of::<&str>())?,
    ])?;
    let mut rows = mul(usage.elements, row)?;
    if heading {
        // Lazy roots grow an entry table and the required candidate prefix.
        // Price their geometric spare slots and minima before either grows.
        fixed = add(
            fixed,
            sum(&[
                std::mem::size_of::<Vec<usize>>(),
                mul(4, std::mem::size_of::<Entry<'_>>())?,
                mul(4, std::mem::size_of::<usize>())?,
            ])?,
        )?;
        rows = add(
            rows,
            mul(
                usage.elements,
                sum(&[
                    std::mem::size_of::<Entry<'_>>(),
                    mul(2, std::mem::size_of::<usize>())?,
                ])?,
            )?,
        )?;
    }
    // Skipped comment/other children still consume inspections and work-list
    // space. One raw append token per node has doubled geometric slack.
    let label_worklist = mul(
        mul(usage.nodes, 2)?,
        std::mem::size_of::<AppendElementWork<'_>>(),
    )?;
    sum(&[fixed, rows, label_worklist])
}

fn summarize_label<'a>(
    element: ElementRef<'a>,
    mut child_label: impl FnMut(ElementRef<'a>) -> Result<Label>,
    mut text_has_content: impl FnMut(&str) -> Result<bool>,
) -> Result<Label> {
    let mut label = Label {
        visits: 1,
        ..Label::default()
    };
    if !append_element_suppressed(element) {
        let (block, trailing) = append_element_spacing(element.value().name());
        label.bytes = usize::from(block);
        if let Some(space) = trailing {
            label.visits = add(label.visits, 1)?;
            label.bytes = add(label.bytes, space.len_utf8())?;
        }
        for child in element.children() {
            label.child_inspections = add(label.child_inspections, 1)?;
            if let Some(text) = child.value().as_text() {
                label.visits = add(label.visits, 1)?;
                label.bytes = add(label.bytes, text.len())?;
                label.has_content |= text_has_content(text)?;
            } else if let Some(child) = ElementRef::wrap(child) {
                let child = child_label(child)?;
                label.visits = add(label.visits, child.visits)?;
                label.child_inspections = add(label.child_inspections, child.child_inspections)?;
                label.bytes = add(label.bytes, child.bytes)?;
                label.has_content |= child.has_content;
            }
        }
    }
    Ok(label)
}

/// Preserve ElementRef.select's original heading order. A cached subtree
/// summarizes nested headings once; later sibling roots remain untouched once
/// the first nonempty heading is known. All required label fees precede copying.
pub(super) fn heading_title(
    body: ElementRef<'_>,
    retained: &mut RetainedBudget,
    work: &mut Budget,
) -> Result<Option<String>> {
    #[cfg(test)]
    let limits = super::heading_work_tests::configured_limits().unwrap_or_default();
    #[cfg(not(test))]
    let limits = Limits::default();
    let ceiling = Limits::default();
    let limits = Limits {
        work_bytes: limits.work_bytes.min(ceiling.work_bytes),
        metadata_bytes: limits.metadata_bytes.min(ceiling.metadata_bytes),
    };
    let selector = Selector::parse("h1,h2,h3").expect("constant selector");
    let mut headings = body.select(&selector);
    let Some(first) = headings.next() else {
        return Ok(None);
    };
    let map_slot = add(std::mem::size_of_val(&(first.id(), 0usize)), 2)?;
    let mut usage = Usage::default();
    let mut temporary_bytes = metadata_forecast(usage, map_slot, true)?;
    if temporary_bytes > limits.metadata_bytes {
        return Err(
            ImportLimitExceeded("HTML heading projection exceeds its metadata allowance").into(),
        );
    }
    retained.reserve(&[temporary_bytes])?;
    let result = (|| -> Result<Option<String>> {
        let mut entries: Vec<Entry<'_>> = Vec::new();
        let mut by_id: HashMap<_, usize> = HashMap::new();
        let mut pending: Vec<Visit<'_>> = Vec::new();
        let mut prefix: Vec<usize> = Vec::new();
        let mut selected: Option<usize> = None;
        for heading in std::iter::once(first).chain(headings) {
            admit_work(&mut usage, 6, limits, work)?;
            let index = if let Some(index) = by_id.get(&heading.id()).copied() {
                index
            } else {
                let old_elements = usage.elements;
                // Include the root explicitly. Filtering its identity makes
                // this independent of whether descendants includes self.
                admit_work(&mut usage, 6, limits, work)?;
                usage.nodes = add(usage.nodes, 1)?;
                usage.elements = add(usage.elements, 1)?;
                for node in heading
                    .descendants()
                    .filter(|node| node.id() != heading.id())
                {
                    admit_work(&mut usage, 6, limits, work)?;
                    usage.nodes = add(usage.nodes, 1)?;
                    if node.value().is_element() {
                        usage.elements = add(usage.elements, 1)?;
                    }
                }
                usage.metadata_bytes = metadata_forecast(usage, map_slot, true)?;
                #[cfg(test)]
                super::heading_work_tests::observe_preflight(usage);
                if usage.metadata_bytes > limits.metadata_bytes {
                    return Err(ImportLimitExceeded(
                        "HTML heading projection exceeds its metadata allowance",
                    )
                    .into());
                }
                let additional = usage.metadata_bytes.checked_sub(temporary_bytes).ok_or(
                    ImportLimitExceeded("HTML heading metadata forecast moved backwards"),
                )?;
                retained.reserve(&[additional])?;
                temporary_bytes = usage.metadata_bytes;
                let elements =
                    usage
                        .elements
                        .checked_sub(old_elements)
                        .ok_or(ImportLimitExceeded(
                            "HTML heading element count moved backwards",
                        ))?;
                entries.reserve(elements);
                by_id.reserve(elements);
                pending.reserve(add(mul(elements, 2)?, 4)?);
                let index = entries.len();
                by_id.insert(heading.id(), index);
                entries.push(Entry {
                    element: heading,
                    candidate: false,
                    label: Label::default(),
                });
                pending.push(Visit::Enter {
                    element: heading,
                    nav_ancestor: false,
                });
                for node in heading
                    .descendants()
                    .filter(|node| node.id() != heading.id())
                {
                    let Some(element) = ElementRef::wrap(node) else {
                        continue;
                    };
                    by_id.insert(element.id(), entries.len());
                    entries.push(Entry {
                        element,
                        candidate: false,
                        label: Label::default(),
                    });
                    // Fragment/document-parent forests can contain independently
                    // selectable headings, even under a suppressed element.
                    if element.parent().and_then(ElementRef::wrap).is_none() {
                        pending.push(Visit::Enter {
                            element,
                            nav_ancestor: false,
                        });
                    }
                }
                #[cfg(test)]
                super::heading_work_tests::observe_projection(elements);
                while let Some(visit) = pending.pop() {
                    #[cfg(test)]
                    super::heading_work_tests::observe_projection_visit();
                    match visit {
                        Visit::Enter { element, .. } => {
                            pending.push(Visit::Exit(element));
                            for child in element.children().rev().filter_map(ElementRef::wrap) {
                                pending.push(Visit::Enter {
                                    element: child,
                                    nav_ancestor: false,
                                });
                            }
                        }
                        Visit::Exit(element) => {
                            let index = *by_id
                                .get(&element.id())
                                .expect("An immutable heading element has an index");
                            let label = summarize_label(
                                element,
                                |child| {
                                    Ok(entries[*by_id
                                        .get(&child.id())
                                        .expect("An immutable heading child has an index")]
                                    .label)
                                },
                                |text| {
                                    admit_work(&mut usage, text.len(), limits, work)?;
                                    Ok(text.chars().any(|character| !character.is_whitespace()))
                                },
                            )?;
                            entries[index].label = label;
                        }
                    }
                }
                index
            };
            let label = entries[index].label;
            // Empty headings still had the original raw append byte limit.
            // Apply it only to the candidates the old loop would have visited.
            check_title_length(label.bytes)?;
            prefix.push(index);
            if label.has_content {
                selected = Some(index);
                break;
            }
        }
        for index in prefix {
            let label = entries[index].label;
            usage.candidates = add(usage.candidates, 1)?;
            usage.label_visits = add(usage.label_visits, label.visits)?;
            usage.label_child_inspections =
                add(usage.label_child_inspections, label.child_inspections)?;
            usage.label_bytes = add(usage.label_bytes, label.bytes)?;
            let fee = sum(&[
                mul(6, label.visits)?,
                mul(6, label.child_inspections)?,
                mul(3, label.bytes)?,
            ])?;
            admit_work(&mut usage, fee, limits, work)?;
        }
        #[cfg(test)]
        super::heading_work_tests::observe_admitted(usage);
        let Some(index) = selected else {
            return Ok(None);
        };
        let mut title = String::new();
        append_element_with_observer(
            entries[index].element,
            &mut title,
            MAX_TITLE_BYTES,
            || {
                #[cfg(test)]
                super::heading_work_tests::observe_label_visit();
            },
            || {
                #[cfg(test)]
                super::heading_work_tests::observe_label_child_inspection();
            },
        )?;
        #[cfg(test)]
        super::heading_work_tests::observe_label_copy(title.len());
        let title = compact_whitespace(&title);
        Ok((!title.is_empty()).then_some(title))
    })();
    // Only temporary tables and scratch are released. Failed or skipped label
    // forecasts remain spent in this import operation's work budget.
    retained.used -= temporary_bytes;
    result
}

fn add(left: usize, right: usize) -> Result<usize> {
    left.checked_add(right)
        .ok_or_else(|| ImportLimitExceeded("HTML navigation work forecast overflow").into())
}

fn mul(left: usize, right: usize) -> Result<usize> {
    left.checked_mul(right)
        .ok_or_else(|| ImportLimitExceeded("HTML navigation work forecast overflow").into())
}

fn sum(parts: &[usize]) -> Result<usize> {
    parts.iter().try_fold(0, |used, part| add(used, *part))
}

fn admit_work(usage: &mut Usage, amount: usize, limits: Limits, work: &mut Budget) -> Result<()> {
    let next = add(usage.work_bytes, amount)?;
    ensure!(
        next <= limits.work_bytes,
        ImportLimitExceeded("HTML navigation processing exceeds its work allowance")
    );
    work.admit(amount)?;
    usage.work_bytes = next;
    Ok(())
}

fn observe_preflight(usage: Usage) {
    #[cfg(test)]
    super::nav_work_tests::observe_preflight(usage);
    #[cfg(not(test))]
    let _ = usage;
}

fn observe_projection(nodes: usize, capacity: usize) {
    #[cfg(test)]
    super::nav_work_tests::observe_projection(nodes, capacity);
    #[cfg(not(test))]
    let _ = (nodes, capacity);
}

fn observe_projection_visit() {
    #[cfg(test)]
    super::nav_work_tests::observe_projection_visit();
}

fn observe_admitted(usage: Usage) {
    #[cfg(test)]
    super::nav_work_tests::observe_admitted(usage);
    #[cfg(not(test))]
    let _ = usage;
}

pub(super) fn observe_label_visit() {
    #[cfg(test)]
    super::nav_work_tests::observe_label_visit();
}

pub(super) fn observe_label_child_inspection() {
    #[cfg(test)]
    super::nav_work_tests::observe_label_child_inspection();
}

pub(super) fn observe_label_copy(bytes: usize) {
    #[cfg(test)]
    super::nav_work_tests::observe_label_copy(bytes);
    #[cfg(not(test))]
    let _ = bytes;
}

pub(super) fn observe_title() {
    #[cfg(test)]
    super::nav_work_tests::observe_title();
}
