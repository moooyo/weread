//! Resource-checked SVG ancestry equivalent to the locked "svg image" selector.

use std::collections::HashMap;

use scraper::{ElementRef, Html};

use super::{Result, SourceError};

const MAX_GUARD_WORK: usize = 64 * 1024 * 1024;
const MAX_GUARD_METADATA: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub work_bytes: usize,
    pub metadata_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            work_bytes: MAX_GUARD_WORK,
            metadata_bytes: MAX_GUARD_METADATA,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Usage {
    pub nodes: usize,
    pub elements: usize,
    pub work_bytes: usize,
    pub metadata_bytes: usize,
    pub element_visits: usize,
    pub child_inspections: usize,
    pub svg_images: usize,
    pub first_match_ordinal: Option<usize>,
}

struct Entry {
    matched: bool,
}

struct Visit<'a> {
    element: ElementRef<'a>,
    svg_ancestor: bool,
}

pub(super) fn count_svg_images(document: &Html) -> Result<usize> {
    #[cfg(test)]
    let limits = super::image_svg_guard_tests::configured_limits().unwrap_or_default();
    #[cfg(not(test))]
    let limits = Limits::default();
    count_with_limits(document, limits)
}

fn count_with_limits(document: &Html, limits: Limits) -> Result<usize> {
    let ceiling = Limits::default();
    let limits = Limits {
        work_bytes: limits.work_bytes.min(ceiling.work_bytes),
        metadata_bytes: limits.metadata_bytes.min(ceiling.metadata_bytes),
    };
    let mut usage = Usage::default();
    let admitted = (|| -> Result<()> {
        // Counting allocates no summary. Eight logical row units prepay this
        // pass, initialization, fixed-width hashing, and the one forest walk.
        for node in document.tree.nodes() {
            usage.work_bytes = add(usage.work_bytes, 8)?;
            if usage.work_bytes > limits.work_bytes {
                return Err(SourceError::ResponseTooLarge);
            }
            usage.nodes = add(usage.nodes, 1)?;
            if node.value().is_element() {
                usage.elements = add(usage.elements, 1)?;
            }
        }
        let id = document.tree.root().id();
        let map_slot = add(std::mem::size_of_val(&(id, 0usize)), 2)?;
        let per_element = sum(&[
            std::mem::size_of::<Entry>(),
            mul(2, std::mem::size_of::<Visit<'_>>())?,
            mul(2, map_slot)?,
        ])?;
        let fixed = sum(&[
            2 * std::mem::size_of::<Vec<Entry>>(),
            std::mem::size_of::<HashMap<usize, usize>>(),
            mul(4, map_slot)?,
            mul(4, std::mem::size_of::<Visit<'_>>())?,
        ])?;
        usage.metadata_bytes = add(fixed, mul(usage.elements, per_element)?)?;
        if usage.metadata_bytes > limits.metadata_bytes {
            return Err(SourceError::ResponseTooLarge);
        }
        Ok(())
    })();
    observe_preflight(usage, admitted.is_ok());
    admitted?;

    // The immutable DOM stays borrowed. Hash keys are its fixed-width node
    // identities; no attribute, URL, or parent-path strings are retained.
    let mut entries = Vec::with_capacity(usage.elements);
    let mut by_id = HashMap::with_capacity(usage.elements);
    let mut pending = Vec::with_capacity(add(mul(usage.elements, 2)?, 4)?);
    for node in document.tree.nodes() {
        let Some(element) = ElementRef::wrap(node) else {
            continue;
        };
        by_id.insert(element.id(), entries.len());
        entries.push(Entry { matched: false });
        if element.parent().and_then(ElementRef::wrap).is_none() {
            pending.push(Visit {
                element,
                svg_ancestor: false,
            });
        }
    }
    observe_build(entries.len(), entries.capacity());
    while let Some(visit) = pending.pop() {
        usage.element_visits = add(usage.element_visits, 1)?;
        let index = *by_id
            .get(&visit.element.id())
            .expect("An immutable SVG guard element has an index");
        // The original selector's local names have any namespace. Its parent
        // relation stops at non-element document/fragment nodes, exactly like
        // the forest roots above. Namespace transitions do not reset ancestry.
        entries[index].matched = visit.svg_ancestor
            && visit.element.value().name() == "image"
            && visit.element.parent().is_some();
        let svg_ancestor = visit.svg_ancestor || visit.element.value().name() == "svg";
        for child in visit.element.children().rev() {
            usage.child_inspections = add(usage.child_inspections, 1)?;
            if let Some(element) = ElementRef::wrap(child) {
                pending.push(Visit {
                    element,
                    svg_ancestor,
                });
            }
        }
    }
    // Html.select returns storage order. Keep its first-match identity and
    // complete count even when the parser reparented an element in the forest.
    for (ordinal, entry) in entries.iter().enumerate() {
        if entry.matched {
            usage.svg_images = add(usage.svg_images, 1)?;
            usage.first_match_ordinal.get_or_insert(ordinal);
        }
    }
    observe_complete(usage);
    // All vectors/hash tables and DOM references end with this call. This is
    // a logical work/metadata policy, not a total parser RSS or time guarantee.
    Ok(usage.svg_images)
}

fn add(left: usize, right: usize) -> Result<usize> {
    left.checked_add(right).ok_or(SourceError::ResponseTooLarge)
}

fn mul(left: usize, right: usize) -> Result<usize> {
    left.checked_mul(right).ok_or(SourceError::ResponseTooLarge)
}

fn sum(parts: &[usize]) -> Result<usize> {
    parts.iter().try_fold(0, |used, part| add(used, *part))
}

fn observe_preflight(usage: Usage, admitted: bool) {
    #[cfg(test)]
    super::image_svg_guard_tests::observe_preflight(usage, admitted);
    #[cfg(not(test))]
    let _ = (usage, admitted);
}

fn observe_build(elements: usize, capacity: usize) {
    #[cfg(test)]
    super::image_svg_guard_tests::observe_build(elements, capacity);
    #[cfg(not(test))]
    let _ = (elements, capacity);
}

fn observe_complete(usage: Usage) {
    #[cfg(test)]
    super::image_svg_guard_tests::observe_complete(usage);
    #[cfg(not(test))]
    let _ = usage;
}
