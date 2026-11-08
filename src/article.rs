use std::collections::BTreeSet;

#[cfg(test)]
use scraper::Selector;
use scraper::{ElementRef, Html, node::Node};
use sha2::{Digest, Sha256};
use url::Url;

use crate::reader::layout_budget::{LayoutLease, LayoutString, LayoutVec};
use crate::reader::{
    AdmittedLayout, Layout, LayoutBudget, LayoutLimitError, Paginator, TextMetrics,
};

#[path = "article_style_budget.rs"]
mod style_budget;

pub use style_budget::{StyleLimitError, StyleLimitKind};

/// Borrowed eligibility for one immutable HTML operation. No selector class
/// cache, token Vec, or normalized attribute String is materialized.
pub(crate) struct ArticleStyleWork<'a> {
    budget: style_budget::Budget<'a>,
}

impl Default for ArticleStyleWork<'_> {
    fn default() -> Self {
        Self {
            budget: style_budget::Budget::default(),
        }
    }
}

impl<'a> ArticleStyleWork<'a> {
    pub(crate) fn content_container(
        &mut self,
        document: &'a Html,
    ) -> Result<Option<ElementRef<'a>>, StyleLimitError> {
        // Match the exact pinned Html::select storage order, including its
        // attached-parent gate. This is a union, not a global ID priority.
        for node in document.tree.nodes() {
            let Some(element) = ElementRef::wrap(node) else {
                continue;
            };
            if element.parent().is_none() {
                continue;
            }
            let value = element.value();
            // Pinned scraper has_id uses the first local-name ID irrespective
            // of namespace. The fixed ten-byte comparison needs no ID cache.
            let id: Option<&str> = value
                .attrs
                .iter()
                .find(|(name, _)| name.local.as_ref() == "id")
                .map(|(_, value)| &**value);
            if id == Some("js_content") {
                return Ok(Some(element));
            }
            // Selector matching in scraper 0.24.0 fixes NoQuirks, even when
            // Html.quirks_mode differs. Keep exact case-sensitive membership
            // over every local-name class attribute, as Element.classes does.
            for (name, classes) in value.attrs.iter() {
                if name.local.as_ref() == "class" && self.budget.container_class(classes)? {
                    return Ok(Some(element));
                }
            }
        }
        Ok(None)
    }

    pub(crate) fn suppresses(&mut self, style: &'a str) -> Result<bool, StyleLimitError> {
        self.budget.suppresses(style)
    }

    pub(crate) fn suppressed_class(&mut self, classes: &'a str) -> Result<bool, StyleLimitError> {
        self.budget.collector_class(classes)
    }
}

#[path = "article_image_budget.rs"]
mod image_budget;

pub use image_budget::{ImageLimitError, ImageLimitKind};

#[derive(Debug, thiserror::Error)]
pub enum ArticleParseError {
    #[error(transparent)]
    Style(#[from] StyleLimitError),
    #[error(transparent)]
    Image(#[from] ImageLimitError),
    #[error(transparent)]
    Layout(#[from] LayoutLimitError),
}

pub(crate) fn image_work_preflight(images: &[ArticleImage]) -> Result<(), ImageLimitError> {
    image_budget::image_work_preflight(images)
}

#[cfg(test)]
pub(crate) use style_budget::Limits as StyleTestLimits;

#[cfg(test)]
pub(crate) use image_budget::Limits as ImageTestLimits;

/// Article settings are supplied independently from the book reader's settings.
pub type ArticleLayout = Layout;

/// Image frame and caption dimensions in logical pixels, supplied by the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArticleImageGeometry {
    pub placeholder_height: f32,
    pub loaded_height: f32,
    pub caption_height: f32,
    pub caption_gap: f32,
}

impl Default for ArticleImageGeometry {
    fn default() -> Self {
        Self {
            placeholder_height: 180.0,
            loaded_height: 220.0,
            caption_height: 20.0,
            caption_gap: 8.0,
        }
    }
}

/// Quote typography and padding are independent of the body font selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArticleQuoteGeometry {
    pub horizontal_padding: f32,
    pub vertical_padding: f32,
    pub font_scale: f32,
    pub line_height: f32,
}

impl Default for ArticleQuoteGeometry {
    fn default() -> Self {
        Self {
            horizontal_padding: 24.0,
            vertical_padding: 16.0,
            font_scale: 0.95,
            line_height: 1.8,
        }
    }
}

/// Platform-specific dimensions belong to the caller, not the HTML extractor.
/// Supply the serif metrics used by the quote renderer for matching line breaks.
#[derive(Clone, Copy, Default)]
pub struct ArticleLayoutOptions<'a> {
    pub images: ArticleImageGeometry,
    pub quote: ArticleQuoteGeometry,
    pub quote_metrics: Option<&'a TextMetrics>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArticleImage {
    pub id: String,
    pub url: String,
    pub caption: String,
}

/// A DOM image occurrence mapped to a Unicode scalar boundary in cached text.
/// `source` is the decoded attribute identity, not an approved fetch URL.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnchoredImage {
    pub id: String,
    pub source: String,
    pub anchor: Option<u32>,
    pub caption: String,
}

/// A rejected mapping never returns a truncated occurrence list or a chosen
/// caption alternative. These counters describe admitted work, not total heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorLimitKind {
    Occurrences,
    Events,
    Edges,
    MatchingWork,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnchorUsage {
    pub occurrences: usize,
    pub events: usize,
    pub edges: usize,
    pub matching_work: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("image anchor {kind:?} exceeds its budget (limit {limit})")]
pub struct AnchorLimitError {
    pub kind: AnchorLimitKind,
    pub usage: AnchorUsage,
    pub requested: usize,
    pub limit: usize,
}

#[derive(Clone, Copy)]
struct AnchorLimits {
    occurrences: usize,
    events: usize,
    edges: usize,
    matching_work: usize,
}

impl Default for AnchorLimits {
    fn default() -> Self {
        Self {
            // Keep the existing chapter-manifest occurrence policy. Event and
            // edge limits independently cover long runs of standalone captions.
            occurrences: 2048,
            events: 16 * 1024,
            edges: 256 * 1024,
            // Unicode scalar units include canonical normalization and full
            // attempted slice lengths, even when comparison can fail early.
            matching_work: 64 * 1024 * 1024,
        }
    }
}

#[derive(Default)]
struct AnchorBudget {
    limits: AnchorLimits,
    usage: AnchorUsage,
}

impl AnchorBudget {
    fn new(limits: AnchorLimits) -> Self {
        Self {
            limits,
            usage: AnchorUsage::default(),
        }
    }

    fn admit(&mut self, kind: AnchorLimitKind, amount: usize) -> Result<(), AnchorLimitError> {
        let (used, limit) = match kind {
            AnchorLimitKind::Occurrences => (self.usage.occurrences, self.limits.occurrences),
            AnchorLimitKind::Events => (self.usage.events, self.limits.events),
            AnchorLimitKind::Edges => (self.usage.edges, self.limits.edges),
            AnchorLimitKind::MatchingWork => (self.usage.matching_work, self.limits.matching_work),
        };
        let Some(next) = used.checked_add(amount).filter(|next| *next <= limit) else {
            return Err(AnchorLimitError {
                kind,
                usage: self.usage,
                requested: amount,
                limit,
            });
        };
        match kind {
            AnchorLimitKind::Occurrences => self.usage.occurrences = next,
            AnchorLimitKind::Events => self.usage.events = next,
            AnchorLimitKind::Edges => self.usage.edges = next,
            AnchorLimitKind::MatchingWork => self.usage.matching_work = next,
        }
        Ok(())
    }

    fn style_error(&self, error: StyleLimitError) -> AnchorLimitError {
        AnchorLimitError {
            kind: AnchorLimitKind::MatchingWork,
            usage: self.usage,
            requested: error.requested,
            limit: error.limit,
        }
    }

    fn image_error(&self, error: ImageLimitError) -> AnchorLimitError {
        AnchorLimitError {
            kind: AnchorLimitKind::MatchingWork,
            usage: self.usage,
            requested: error.requested,
            limit: error.limit,
        }
    }
}

/// Locate book images without changing the canonical text used by positions.
///
/// Text is matched from the beginning in DOM order; an unmatched prefix makes
/// subsequent positions unknown. Whitespace is ignored for matching only, and
/// every returned offset still addresses the original Unicode scalar stream.
/// Figure captions may be present or absent in that stream. Their alternatives
/// are resolved by later text when possible; ambiguous positions remain unknown.
/// Occurrence, event, edge, or matching-work exhaustion rejects the whole mapping.
pub fn image_anchors(
    html: &str,
    canonical_plaintext: &str,
) -> Result<Vec<AnchoredImage>, AnchorLimitError> {
    let document = Html::parse_document(html);
    image_anchors_in_document(&document, canonical_plaintext)
}

/// Reuse a worker's parsed document without changing scalar matching semantics.
pub(crate) fn image_anchors_in_document(
    document: &Html,
    canonical_plaintext: &str,
) -> Result<Vec<AnchoredImage>, AnchorLimitError> {
    image_anchors_in_document_with_limits(document, canonical_plaintext, AnchorLimits::default())
}

fn image_anchors_in_document_with_limits(
    document: &Html,
    canonical_plaintext: &str,
    limits: AnchorLimits,
) -> Result<Vec<AnchoredImage>, AnchorLimitError> {
    let mut collector = AnchorCollector {
        budget: AnchorBudget::new(limits),
        ..AnchorCollector::default()
    };
    collector.visit(document.root_element())?;
    collector.flush();
    collector.materialize_images();
    // Reserve the original scalar stream before allocating its normalized
    // characters and scalar ends. The caller already bounds input bytes.
    collector.budget.admit(
        AnchorLimitKind::MatchingWork,
        canonical_plaintext.chars().count(),
    )?;
    let Some(canonical) = CanonicalMatch::new(canonical_plaintext) else {
        return Ok(collector.images);
    };

    let mut cursors = BTreeSet::from([0usize]);
    let mut steps = Vec::new();
    let mut image_has_prior_text = vec![false; collector.images.len()];
    for event in collector.events {
        let mut edges = Vec::new();
        let image_index = match event {
            AnchorEvent::Text(text) => {
                for cursor in &cursors {
                    collector
                        .budget
                        .admit(AnchorLimitKind::MatchingWork, text.len())?;
                    if let Some(end) = canonical.match_at(*cursor, &text) {
                        collector.budget.admit(AnchorLimitKind::Edges, 1)?;
                        edges.push((*cursor, end));
                    }
                }
                None
            }
            AnchorEvent::Caption(text) => {
                for cursor in &cursors {
                    collector.budget.admit(AnchorLimitKind::Edges, 1)?;
                    edges.push((*cursor, *cursor));
                    collector
                        .budget
                        .admit(AnchorLimitKind::MatchingWork, text.len())?;
                    if let Some(end) = canonical.match_at(*cursor, &text) {
                        collector.budget.admit(AnchorLimitKind::Edges, 1)?;
                        edges.push((*cursor, end));
                    }
                }
                None
            }
            AnchorEvent::Image {
                index,
                has_prior_text,
            } => {
                image_has_prior_text[index] = has_prior_text;
                collector.images[index].anchor =
                    canonical.unique_anchor(cursors.iter().copied(), has_prior_text);
                collector
                    .budget
                    .admit(AnchorLimitKind::Edges, cursors.len())?;
                edges.extend(cursors.iter().map(|cursor| (*cursor, *cursor)));
                Some(index)
            }
        };
        cursors = edges.iter().map(|(_, next)| *next).collect();
        // Optional captions can create many alternatives in pathological HTML.
        // Stop locating new images instead of pruning to an arbitrary position.
        if cursors.len() > 128 {
            cursors.clear();
            edges.clear();
        }
        steps.push(AnchorStep { image_index, edges });
    }

    // A complete alignment resolves captions that look like the next paragraph.
    // If later unrelated text differs, retain only the already proven prefixes.
    if cursors.contains(&canonical.characters.len()) {
        let mut viable = BTreeSet::from([canonical.characters.len()]);
        for step in steps.into_iter().rev() {
            let before: BTreeSet<_> = step
                .edges
                .into_iter()
                .filter_map(|(previous, next)| viable.contains(&next).then_some(previous))
                .collect();
            if let Some(index) = step.image_index {
                collector.images[index].anchor =
                    canonical.unique_anchor(before.iter().copied(), image_has_prior_text[index]);
            }
            viable = before;
        }
    }
    Ok(collector.images)
}

struct CanonicalMatch {
    characters: Vec<char>,
    scalar_ends: Vec<u32>,
    original_length: u32,
}

impl CanonicalMatch {
    fn new(text: &str) -> Option<Self> {
        let mut characters = Vec::new();
        let mut scalar_ends = Vec::new();
        let mut original_length = 0;
        for (index, character) in text.chars().enumerate() {
            original_length = u32::try_from(index.checked_add(1)?).ok()?;
            if match_character(character) {
                characters.push(character);
                scalar_ends.push(original_length);
            }
        }
        Some(Self {
            characters,
            scalar_ends,
            original_length,
        })
    }

    fn match_at(&self, cursor: usize, text: &[char]) -> Option<usize> {
        let end = cursor.checked_add(text.len())?;
        (self.characters.get(cursor..end)? == text).then_some(end)
    }

    fn unique_anchor(
        &self,
        cursors: impl Iterator<Item = usize>,
        has_prior_text: bool,
    ) -> Option<u32> {
        let mut positions = BTreeSet::new();
        for cursor in cursors {
            if cursor == 0 && has_prior_text {
                return None;
            }
            let position = if cursor == 0 {
                0
            } else if cursor == self.characters.len() {
                self.original_length
            } else {
                *self.scalar_ends.get(cursor.checked_sub(1)?)?
            };
            positions.insert(position);
        }
        (positions.len() == 1)
            .then(|| positions.first().copied())
            .flatten()
    }
}

fn match_character(character: char) -> bool {
    !character.is_whitespace() && character != '\u{200b}' && character != '\u{feff}'
}

enum AnchorEvent {
    Text(Vec<char>),
    Caption(Vec<char>),
    Image { index: usize, has_prior_text: bool },
}

struct AnchorStep {
    image_index: Option<usize>,
    edges: Vec<(usize, usize)>,
}

#[derive(Default)]
struct AnchorCollector<'a> {
    budget: AnchorBudget,
    images: Vec<AnchoredImage>,
    events: Vec<AnchorEvent>,
    pending: Vec<char>,
    has_visible_text: bool,
    last_image: Option<usize>,
    styles: style_budget::Budget<'a>,
    image_budget: image_budget::Budget,
    image_fields: Vec<AnchorImageFields<'a>>,
}

struct AnchorImageFields<'a> {
    source: &'a str,
    alt: &'a str,
    caption_override: bool,
}

impl<'a> AnchorCollector<'a> {
    fn materialize_images(&mut self) {
        // The complete traversal already admitted every image/caption field
        // and count. Only now may any source clone, normalization, or SHA run.
        for (index, (image, fields)) in self.images.iter_mut().zip(&self.image_fields).enumerate() {
            self.image_budget.hash();
            let digest = hex::encode(Sha256::digest(fields.source.as_bytes()));
            image.id = format!("image-{}-{index}", &digest[..20]);
            self.image_budget.input_copy();
            image.source = fields.source.to_owned();
            if !fields.caption_override {
                image.caption = normalize_text(fields.alt);
            }
        }
    }

    fn flush(&mut self) {
        if !self.pending.is_empty() {
            self.events
                .push(AnchorEvent::Text(std::mem::take(&mut self.pending)));
        }
    }

    fn append(&mut self, text: &str) -> Result<(), AnchorLimitError> {
        let previous = self.pending.len();
        for character in text.chars().filter(|character| match_character(*character)) {
            if self.pending.is_empty() {
                // The pending event is paid for before its first scalar is
                // retained; flush only transfers this already admitted vector.
                self.budget.admit(AnchorLimitKind::Events, 1)?;
            }
            self.pending.push(character);
        }
        if self.pending.len() > previous {
            self.has_visible_text = true;
            self.last_image = None;
        }
        Ok(())
    }

    fn visit(&mut self, element: ElementRef<'a>) -> Result<(), AnchorLimitError> {
        enum Work<'node> {
            Element(ElementRef<'node>),
            Text(&'node str),
        }
        let mut pending = vec![Work::Element(element)];
        while let Some(work) = pending.pop() {
            let element = match work {
                Work::Text(text) => {
                    self.append(text)?;
                    continue;
                }
                Work::Element(element) => element,
            };
            if anchor_suppressed(element, &mut self.styles)
                .map_err(|error| self.budget.style_error(error))?
            {
                continue;
            }
            let node = element.value();
            if node.name() == "img" {
                // Repeated sources are separate visible occurrences. Admission
                // precedes hashing and cloning, not the later manifest check.
                self.budget.admit(AnchorLimitKind::Occurrences, 1)?;
                self.image_budget
                    .record()
                    .map_err(|error| self.budget.image_error(error))?;
                self.flush();
                self.last_image = None;
                // Book resource plans use src as their primary identity. Unlike the
                // article renderer, this mapper also accepts raw archive references.
                let source = admitted_anchor_source(node, &mut self.image_budget)
                    .map_err(|error| self.budget.image_error(error))?;
                if let Some(source) = source {
                    self.image_budget
                        .caption(node.attr("alt").map_or(0, str::len))
                        .map_err(|error| self.budget.image_error(error))?;
                    self.budget.admit(AnchorLimitKind::Events, 1)?;
                    let index = self.images.len();
                    self.images.push(AnchoredImage::default());
                    self.image_fields.push(AnchorImageFields {
                        source,
                        alt: node.attr("alt").unwrap_or_default(),
                        caption_override: false,
                    });
                    self.events.push(AnchorEvent::Image {
                        index,
                        has_prior_text: self.has_visible_text,
                    });
                    self.last_image = Some(index);
                }
                continue;
            }
            if node.name() == "figcaption" {
                self.flush();
                let mut raw = String::new();
                let image_budget = self.last_image.is_some().then_some(&mut self.image_budget);
                if !caption_text_with_styles(
                    element,
                    &mut raw,
                    &mut self.budget,
                    &mut self.styles,
                    image_budget,
                )? {
                    continue;
                }
                let caption = normalize_text(&raw);
                let characters: Vec<_> = raw.chars().filter(|c| match_character(*c)).collect();
                if !characters.is_empty() {
                    if let Some(index) = self.last_image.take() {
                        self.images[index].caption = caption;
                        self.image_fields[index].caption_override = true;
                        self.events.push(AnchorEvent::Caption(characters));
                    } else {
                        self.events.push(AnchorEvent::Text(characters));
                    }
                    self.has_visible_text = true;
                }
                continue;
            }
            // Reverse only the iterator; do not copy all siblings into another
            // temporary collection. The LIFO work list preserves DOM order.
            for child in element.children().rev() {
                match child.value() {
                    Node::Text(text) => pending.push(Work::Text(text)),
                    Node::Element(_) => {
                        if let Some(element) = ElementRef::wrap(child) {
                            pending.push(Work::Element(element));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

/// The old isolated caption test entry still uses the same bounded policy.
/// Production always passes its complete anchor operation's shared budget.
#[cfg(test)]
fn caption_text(
    element: ElementRef<'_>,
    output: &mut String,
    budget: &mut AnchorBudget,
) -> Result<bool, AnchorLimitError> {
    caption_text_with_styles(
        element,
        output,
        budget,
        &mut style_budget::Budget::default(),
        None,
    )
}

fn caption_text_with_styles<'a>(
    element: ElementRef<'a>,
    output: &mut String,
    budget: &mut AnchorBudget,
    styles: &mut style_budget::Budget<'a>,
    mut images: Option<&mut image_budget::Budget>,
) -> Result<bool, AnchorLimitError> {
    enum Work<'node> {
        Element(ElementRef<'node>),
        Text(&'node str),
    }
    let mut pending = vec![Work::Element(element)];
    let mut has_matching_text = false;
    while let Some(work) = pending.pop() {
        let element = match work {
            Work::Text(text) => {
                if let Some(images) = images.as_deref_mut() {
                    images
                        .caption_fragment(output.len(), text.len())
                        .map_err(|error| budget.image_error(error))?;
                }
                if !has_matching_text && text.chars().any(match_character) {
                    // Empty/suppressed captions consume no event. A visible
                    // caption is admitted before copying its first text chunk.
                    budget.admit(AnchorLimitKind::Events, 1)?;
                    has_matching_text = true;
                }
                output.push_str(text);
                continue;
            }
            Work::Element(element) => element,
        };
        if anchor_suppressed(element, styles).map_err(|error| budget.style_error(error))?
            || element.value().name() == "img"
        {
            continue;
        }
        for child in element.children().rev() {
            match child.value() {
                Node::Text(text) => pending.push(Work::Text(text)),
                Node::Element(_) => {
                    if let Some(element) = ElementRef::wrap(child) {
                        pending.push(Work::Element(element));
                    }
                }
                _ => {}
            }
        }
    }
    Ok(has_matching_text)
}

fn anchor_suppressed<'a>(
    element: ElementRef<'a>,
    styles: &mut style_budget::Budget<'a>,
) -> Result<bool, StyleLimitError> {
    let node = element.value();
    if matches!(
        node.name(),
        "head"
            | "script"
            | "style"
            | "noscript"
            | "template"
            | "svg"
            | "canvas"
            | "video"
            | "audio"
            | "iframe"
            | "object"
            | "embed"
            | "mp-video"
            | "mpvoice"
    ) || node.attr("hidden").is_some()
        || node.attr("aria-hidden") == Some("true")
    {
        return Ok(true);
    }
    node.attr("style")
        .map(|style| styles.suppresses(style))
        .transpose()
        .map(|suppressed| suppressed.unwrap_or(false))
}

fn admitted_anchor_source<'a>(
    node: &'a scraper::node::Element,
    images: &mut image_budget::Budget,
) -> Result<Option<&'a str>, ImageLimitError> {
    for name in ["src", "data-src", "data-original", "data-lazy-src"] {
        if let Some(raw) = node.attr(name) {
            // Admit the original borrowed span before trim can scan it.
            images.url(raw)?;
            let source = raw.trim();
            if !source.is_empty() {
                return Ok(Some(source));
            }
        }
    }
    Ok(None)
}

/// Source planning prepays the same actual anchor style reads before image
/// acquisition. It neither creates anchors nor changes reference visibility.
pub(crate) fn check_anchor_preparation_work(document: &Html) -> Result<(), ArticleParseError> {
    let mut styles = style_budget::Budget::default();
    let mut images = image_budget::Budget::default();
    let root = *document.root_element();
    let mut current = Some(root);
    while let Some(node) = current {
        let skip_children = if let Some(element) = ElementRef::wrap(node) {
            if anchor_suppressed(element, &mut styles)? {
                true
            } else if element.value().name() == "img" {
                images.record()?;
                let value = element.value();
                let source = admitted_anchor_source(value, &mut images)?;
                if let Some(source) = source {
                    images.caption(value.attr("alt").map_or(0, str::len))?;
                }
                true
            } else {
                false
            }
        } else {
            true
        };
        if !skip_children {
            if let Some(child) = node.first_child() {
                current = Some(child);
                continue;
            }
        }
        // Keep constant scratch. Across this complete walk each parent edge
        // is followed once, rather than restarting an ancestor search per node.
        let mut finished = node;
        current = loop {
            if finished == root {
                break None;
            }
            if let Some(sibling) = finished.next_sibling() {
                break Some(sibling);
            }
            finished = finished
                .parent()
                .expect("A borrowed anchor style node remains below its root");
        };
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageBlock {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub meta: String,
    pub image_url: Option<String>,
    pub height: u32,
    pub loaded: bool,
    /// Extra advance between visible Unicode scalars on a justified text line.
    pub letter_spacing: f32,
    pub content_height: f32,
    pub inset_horizontal: f32,
    pub inset_top: f32,
    pub inset_bottom: f32,
    pub frame_height: f32,
    pub caption_height: f32,
    pub caption_gap: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TextKind {
    #[default]
    Paragraph,
    Heading,
    Quote,
}

#[derive(Clone, Debug)]
enum SourceBlock {
    Text { kind: TextKind, text: String },
    Image { image: ArticleImage, size: String },
}

enum StagedBlock<'a> {
    Text {
        kind: TextKind,
        text: String,
    },
    Image {
        element: ElementRef<'a>,
        ordinal: usize,
    },
    Caption {
        element: ElementRef<'a>,
        kind: TextKind,
    },
}

struct ImageInput<'a> {
    urls: Vec<image_budget::AdmittedUrl<'a>>,
    alt: &'a str,
    size: &'a str,
}

struct ReadyImage {
    url: String,
    caption: String,
    size: String,
}

#[derive(Debug)]
pub struct ArticleDocument {
    blocks: Vec<SourceBlock>,
    /// Media is intentionally left to the original article on the phone.
    pub media_count: usize,
    admission: LayoutLease,
}

pub type ArticlePages = AdmittedLayout<Vec<Vec<PageBlock>>>;

impl Default for ArticleDocument {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            media_count: 0,
            admission: LayoutBudget::default().lease(),
        }
    }
}

impl ArticleDocument {
    /// Extract document flow once, without re-reading every nested paragraph.
    #[cfg(test)]
    pub fn parse(html: &str, fallback_text: &str) -> Self {
        Self::parse_checked(html, fallback_text).expect("Admit the ordinary article fixture")
    }

    /// A style capacity error publishes no partial document or fallback text.
    pub fn parse_checked(html: &str, fallback_text: &str) -> Result<Self, ArticleParseError> {
        Self::parse_checked_in(html, fallback_text, &LayoutBudget::default())
    }

    pub(crate) fn parse_checked_in(
        html: &str,
        fallback_text: &str,
        budget: &LayoutBudget,
    ) -> Result<Self, ArticleParseError> {
        budget.work(html.len())?;
        let document = Html::parse_document(html);
        let mut styles = ArticleStyleWork::default();
        let container = styles
            .content_container(&document)?
            .unwrap_or_else(|| document.root_element());
        let mut collector = Collector::new(budget, styles);
        collector.visit(container, TextKind::Paragraph)?;
        collector.flush()?;
        let media_count = collector.media_count;
        let mut blocks = collector.finish_images()?;
        budget.work(fallback_text.len())?;
        if blocks.is_empty() && !fallback_text.trim().is_empty() {
            for paragraph in fallback_text.split('\n') {
                blocks.reserve_one()?;
                let (text, owner) = normalize_text_checked(paragraph, budget)?;
                if !text.is_empty() {
                    blocks.push_owned(
                        SourceBlock::Text {
                            kind: TextKind::Paragraph,
                            text,
                        },
                        owner,
                    )?;
                }
            }
        }
        let (blocks, admission) = blocks.into_parts();
        Ok(Self {
            blocks,
            media_count,
            admission,
        })
    }

    pub fn requested_metadata(&self) -> usize {
        self.admission.bytes()
    }

    pub fn try_images(&self) -> Result<AdmittedLayout<Vec<ArticleImage>>, LayoutLimitError> {
        self.try_images_in(&LayoutBudget::default())
    }

    pub(crate) fn try_images_in(
        &self,
        budget: &LayoutBudget,
    ) -> Result<AdmittedLayout<Vec<ArticleImage>>, LayoutLimitError> {
        let _document = budget.hold_lease(&self.admission)?;
        let mut seen: LayoutVec<&str> = LayoutVec::new(budget);
        let mut images = LayoutVec::new(budget);
        for block in &self.blocks {
            let SourceBlock::Image { image, .. } = block else {
                continue;
            };
            let mut duplicate = false;
            for &prior in seen.iter() {
                budget.work(image.id.len().checked_add(prior.len()).ok_or(
                    LayoutLimitError {
                        kind: crate::reader::LayoutLimitKind::AllocationWork,
                        used: 0,
                        requested: usize::MAX,
                        limit: crate::reader::layout_budget::DEFAULT_LAYOUT_ALLOCATION_WORK,
                    },
                )?)?;
                if prior == image.id.as_str() {
                    duplicate = true;
                    break;
                }
            }
            if duplicate {
                continue;
            }
            images.reserve_one()?;
            seen.push(image.id.as_str())?;
            let (id, mut owner) = budget.copy(&image.id)?;
            let (url, next) = budget.copy(&image.url)?;
            owner.merge(next);
            let (caption, next) = budget.copy(&image.caption)?;
            owner.merge(next);
            images.push_owned(ArticleImage { id, url, caption }, owner)?;
        }
        let buffer = images.buffer_bytes();
        let (images, owner) = images.into_parts();
        Ok(AdmittedLayout::from_vec(images, owner, buffer))
    }

    #[cfg(test)]
    pub fn images(&self) -> Vec<ArticleImage> {
        self.try_images()
            .expect("Admit the ordinary article image fixture")
            .into_parts()
            .0
    }

    pub fn image(&self, id_or_url: &str) -> Option<&ArticleImage> {
        self.blocks.iter().find_map(|block| match block {
            SourceBlock::Image { image, .. } if image.id == id_or_url || image.url == id_or_url => {
                Some(image)
            }
            _ => None,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn try_plain_text(&self) -> Result<AdmittedLayout<String>, LayoutLimitError> {
        let budget = LayoutBudget::default();
        let _document = budget.hold_lease(&self.admission)?;
        let mut bytes = 0usize;
        let mut count = 0usize;
        for block in &self.blocks {
            if let SourceBlock::Text { text, .. } = block {
                bytes = bytes
                    .checked_add(text.len())
                    .and_then(|bytes| bytes.checked_add(usize::from(count > 0)))
                    .ok_or(LayoutLimitError {
                        kind: crate::reader::LayoutLimitKind::Metadata,
                        used: bytes,
                        requested: text.len(),
                        limit: crate::reader::layout_budget::DEFAULT_LAYOUT_METADATA_BYTES,
                    })?;
                count += 1;
            }
        }
        let mut result = LayoutString::with_capacity(&budget, bytes)?;
        let mut first = true;
        for block in &self.blocks {
            if let SourceBlock::Text { text, .. } = block {
                if !first {
                    result.push('\n')?;
                }
                result.append(text)?;
                first = false;
            }
        }
        let (text, owner) = result.into_parts();
        Ok(AdmittedLayout::new(text, owner))
    }

    #[cfg(test)]
    pub fn plain_text(&self) -> String {
        self.try_plain_text()
            .expect("Admit the ordinary article text fixture")
            .into_parts()
            .0
    }

    pub fn try_paginate(
        &self,
        layout: &ArticleLayout,
        metrics: &TextMetrics,
        loaded_images: &BTreeSet<String>,
    ) -> Result<ArticlePages, LayoutLimitError> {
        self.try_paginate_with_first_page_height(layout, metrics, loaded_images, layout.height)
    }

    /// The caller can reserve a separate title and metadata area on page one.
    pub fn try_paginate_with_first_page_height(
        &self,
        layout: &ArticleLayout,
        metrics: &TextMetrics,
        loaded_images: &BTreeSet<String>,
        first_page_height: f32,
    ) -> Result<ArticlePages, LayoutLimitError> {
        self.try_paginate_with_options(
            layout,
            metrics,
            loaded_images,
            first_page_height,
            &ArticleLayoutOptions::default(),
        )
    }

    /// Paginate with the renderer's explicit image and serif quote geometry.
    /// A frame that cannot fit page one is moved intact to a regular page.
    pub fn try_paginate_with_options(
        &self,
        layout: &ArticleLayout,
        metrics: &TextMetrics,
        loaded_images: &BTreeSet<String>,
        first_page_height: f32,
        options: &ArticleLayoutOptions<'_>,
    ) -> Result<ArticlePages, LayoutLimitError> {
        self.try_paginate_with_options_in(
            layout,
            metrics,
            |image| loaded_images.contains(&image.id) || loaded_images.contains(&image.url),
            first_page_height,
            options,
            &LayoutBudget::default(),
        )
    }

    pub(crate) fn try_paginate_with_options_in(
        &self,
        layout: &ArticleLayout,
        metrics: &TextMetrics,
        loaded_images: impl Fn(&ArticleImage) -> bool,
        first_page_height: f32,
        options: &ArticleLayoutOptions<'_>,
        budget: &LayoutBudget,
    ) -> Result<ArticlePages, LayoutLimitError> {
        let _document = budget.hold_lease(&self.admission)?;
        if self.is_empty() {
            let mut pages = LayoutVec::new(budget);
            pages.push(Vec::new())?;
            let (pages, owner) = pages.into_parts();
            return Ok(AdmittedLayout::new(pages, owner));
        }
        let layout = sanitized_layout(*layout);
        let image_geometry = sanitized_image_geometry(options.images);
        let quote_geometry = sanitized_quote_geometry(options.quote, layout.width);
        let fallback_quote_metrics = TextMetrics::default();
        let quote_metrics = options.quote_metrics.unwrap_or(&fallback_quote_metrics);
        let mut units = LayoutVec::new(budget);
        for (index, source) in self.blocks.iter().enumerate() {
            match source {
                SourceBlock::Text { kind, text } => {
                    let mut block_layout = layout;
                    block_layout.height = 1_000_000.0;
                    block_layout.indent = 0;
                    block_layout.paragraph_gap = 0.0;
                    block_layout.justify = layout.justify && *kind != TextKind::Heading;
                    match kind {
                        TextKind::Heading => {
                            block_layout.font_size *= 1.1;
                        }
                        TextKind::Quote => {
                            block_layout.width = (block_layout.width
                                - quote_geometry.horizontal_padding * 2.0)
                                .max(1.0);
                            block_layout.font_size *= quote_geometry.font_scale;
                            block_layout.line_height = quote_geometry.line_height;
                        }
                        TextKind::Paragraph => {}
                    }
                    block_layout.font_size = block_layout.font_size.clamp(8.0, 96.0);
                    let block_metrics = if *kind == TextKind::Quote {
                        quote_metrics
                    } else {
                        metrics
                    };
                    let pagination =
                        Paginator::try_new_in(text, &block_layout, block_metrics, budget)?;
                    let count = pagination
                        .pages
                        .iter()
                        .map(|page| page.lines.len())
                        .sum::<usize>();
                    let (pages, mut line_owner, line_buffers) = pagination.into_pages();
                    for (line_index, line) in
                        pages.into_iter().flat_map(|page| page.lines).enumerate()
                    {
                        units.reserve_one()?;
                        let final_line = line_index + 1 == count;
                        let gap = if final_line {
                            layout.font_size * layout.paragraph_gap
                        } else {
                            0.0
                        };
                        let content_height = block_layout.line_advance();
                        let letter_spacing = justified_letter_spacing(
                            &line.text,
                            line.width,
                            &block_layout,
                            block_metrics,
                            line.paragraph_end,
                        );
                        let (id, mut owner) =
                            budget.format(format_args!("text-{index}-{}", line.start))?;
                        let (kind_label, next) = budget.copy(match kind {
                            TextKind::Heading => "heading",
                            TextKind::Quote => "quote",
                            TextKind::Paragraph if final_line => "paragraph",
                            TextKind::Paragraph => "line",
                        })?;
                        owner.merge(next);
                        units.push_owned(
                            FlowUnit {
                                block: PageBlock {
                                    id,
                                    kind: kind_label,
                                    text: line.text,
                                    height: pixel_height(content_height),
                                    letter_spacing,
                                    content_height,
                                    inset_horizontal: if *kind == TextKind::Quote {
                                        quote_geometry.horizontal_padding
                                    } else {
                                        0.0
                                    },
                                    ..PageBlock::default()
                                },
                                source_index: index,
                                quote: *kind == TextKind::Quote,
                                gap_after: pixel_height_or_zero(gap),
                            },
                            owner,
                        )?;
                    }
                    line_owner.release(line_buffers);
                    units.adopt(line_owner);
                }
                SourceBlock::Image { image, size } => {
                    let loaded = loaded_images(image);
                    let frame_height = if loaded {
                        image_geometry.loaded_height
                    } else {
                        image_geometry.placeholder_height
                    };
                    units.reserve_one()?;
                    let (id, mut owner) = budget.copy(&image.id)?;
                    let (kind, next) = budget.copy("image")?;
                    owner.merge(next);
                    let (caption, next) = budget.copy(&image.caption)?;
                    owner.merge(next);
                    let (size, next) = budget.copy(size)?;
                    owner.merge(next);
                    let (url, next) = budget.copy(&image.url)?;
                    owner.merge(next);
                    units.push_owned(
                        FlowUnit {
                            block: PageBlock {
                                id,
                                kind,
                                text: caption,
                                meta: size,
                                image_url: Some(url),
                                height: pixel_height(
                                    frame_height
                                        + image_geometry.caption_height
                                        + image_geometry.caption_gap,
                                ),
                                loaded,
                                frame_height,
                                caption_height: image_geometry.caption_height,
                                caption_gap: image_geometry.caption_gap,
                                ..PageBlock::default()
                            },
                            source_index: index,
                            quote: false,
                            gap_after: 0,
                        },
                        owner,
                    )?;
                }
            }
        }
        units.reserve_one()?;
        let (id, mut owner) = budget.copy("article-end")?;
        let (kind, next) = budget.copy("end")?;
        owner.merge(next);
        units.push_owned(
            FlowUnit {
                block: PageBlock {
                    id,
                    kind,
                    height: 180,
                    ..PageBlock::default()
                },
                source_index: self.blocks.len(),
                quote: false,
                gap_after: 0,
            },
            owner,
        )?;
        pack_units(
            units,
            layout.height,
            first_page_height,
            quote_geometry.vertical_padding,
            budget,
        )
    }

    #[cfg(test)]
    pub fn paginate(
        &self,
        layout: &ArticleLayout,
        metrics: &TextMetrics,
        loaded_images: &BTreeSet<String>,
    ) -> Vec<Vec<PageBlock>> {
        self.try_paginate(layout, metrics, loaded_images)
            .expect("Admit the ordinary article pagination fixture")
            .into_parts()
            .0
    }

    #[cfg(test)]
    pub fn paginate_with_first_page_height(
        &self,
        layout: &ArticleLayout,
        metrics: &TextMetrics,
        loaded_images: &BTreeSet<String>,
        first_page_height: f32,
    ) -> Vec<Vec<PageBlock>> {
        self.try_paginate_with_first_page_height(layout, metrics, loaded_images, first_page_height)
            .expect("Admit the ordinary article first-page fixture")
            .into_parts()
            .0
    }

    #[cfg(test)]
    pub fn paginate_with_options(
        &self,
        layout: &ArticleLayout,
        metrics: &TextMetrics,
        loaded_images: &BTreeSet<String>,
        first_page_height: f32,
        options: &ArticleLayoutOptions<'_>,
    ) -> Vec<Vec<PageBlock>> {
        self.try_paginate_with_options(layout, metrics, loaded_images, first_page_height, options)
            .expect("Admit the ordinary article geometry fixture")
            .into_parts()
            .0
    }
}

#[cfg(test)]
impl Clone for ArticleDocument {
    fn clone(&self) -> Self {
        let budget = LayoutBudget::default();
        let _old = budget
            .hold(self.requested_metadata())
            .expect("Hold the article fixture");
        let admission = budget
            .copy_storage(self.requested_metadata())
            .expect("Admit the article fixture copy");
        Self {
            blocks: self.blocks.clone(),
            media_count: self.media_count,
            admission,
        }
    }
}

fn sanitized_layout(mut layout: Layout) -> Layout {
    if !layout.width.is_finite() || layout.width < 1.0 {
        layout.width = 568.0;
    }
    if !layout.height.is_finite() || layout.height < 1.0 {
        layout.height = 720.0;
    }
    if !layout.font_size.is_finite() {
        layout.font_size = 19.0;
    }
    layout.font_size = layout.font_size.clamp(8.0, 96.0);
    if !layout.line_height.is_finite() {
        layout.line_height = 1.85;
    }
    layout.line_height = layout.line_height.clamp(1.0, 4.0);
    if !layout.paragraph_gap.is_finite() {
        layout.paragraph_gap = 0.9;
    }
    layout.paragraph_gap = layout.paragraph_gap.clamp(0.0, 4.0);
    layout
}

fn finite_dimension(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1_000_000.0)
    } else {
        fallback
    }
}

fn sanitized_image_geometry(value: ArticleImageGeometry) -> ArticleImageGeometry {
    let defaults = ArticleImageGeometry::default();
    ArticleImageGeometry {
        placeholder_height: finite_dimension(value.placeholder_height, defaults.placeholder_height)
            .max(1.0),
        loaded_height: finite_dimension(value.loaded_height, defaults.loaded_height).max(1.0),
        caption_height: finite_dimension(value.caption_height, defaults.caption_height),
        caption_gap: finite_dimension(value.caption_gap, defaults.caption_gap),
    }
}

fn sanitized_quote_geometry(value: ArticleQuoteGeometry, width: f32) -> ArticleQuoteGeometry {
    let defaults = ArticleQuoteGeometry::default();
    ArticleQuoteGeometry {
        horizontal_padding: finite_dimension(value.horizontal_padding, defaults.horizontal_padding)
            .min(((width - 1.0) / 2.0).max(0.0)),
        vertical_padding: finite_dimension(value.vertical_padding, defaults.vertical_padding),
        font_scale: finite_dimension(value.font_scale, defaults.font_scale).clamp(0.1, 4.0),
        line_height: finite_dimension(value.line_height, defaults.line_height).clamp(1.0, 4.0),
    }
}

fn pixel_height(value: f32) -> u32 {
    value.ceil().max(1.0) as u32
}

fn pixel_height_or_zero(value: f32) -> u32 {
    value.ceil().max(0.0) as u32
}

fn justified_letter_spacing(
    text: &str,
    measured_width: f32,
    layout: &Layout,
    metrics: &TextMetrics,
    paragraph_end: bool,
) -> f32 {
    if !layout.justify || paragraph_end {
        return 0.0;
    }
    let gaps = text
        .chars()
        .filter(|character| metrics.advance(*character, layout.font_size) > 0.0)
        .count()
        .saturating_sub(1);
    if gaps == 0 {
        0.0
    } else {
        (layout.width - measured_width).max(0.0) / gaps as f32
    }
}

struct FlowUnit {
    block: PageBlock,
    source_index: usize,
    quote: bool,
    gap_after: u32,
}

fn pack_units(
    units: LayoutVec<FlowUnit>,
    regular_height: f32,
    first_page_height: f32,
    quote_vertical_padding: f32,
    budget: &LayoutBudget,
) -> Result<ArticlePages, LayoutLimitError> {
    let regular_capacity = regular_height.floor().max(1.0) as u32;
    let first_capacity = if first_page_height.is_finite() {
        first_page_height
            .floor()
            .clamp(1.0, regular_height.floor().max(1.0)) as u32
    } else {
        regular_capacity
    };
    let quote_padding = pixel_height_or_zero(quote_vertical_padding);
    let mut pages = LayoutVec::new(budget);
    let mut page = LayoutVec::new(budget);
    let mut used = 0u32;
    let mut last_quote = None;
    let unit_buffer = units.buffer_bytes();
    let (units, mut unit_owner) = units.into_parts();
    let mut queue = units.into_iter().peekable();
    // At most one spacing continuation precedes the immutable source flow.
    // It needs no second queue allocation or unpriced push_front growth.
    let mut pending_spacing = None;
    while let Some(mut unit) = pending_spacing.take().or_else(|| queue.next()) {
        let mut continue_quote = unit.quote && last_quote == Some(unit.source_index);
        let mut growth = unit
            .block
            .height
            .saturating_add(if unit.quote && !continue_quote {
                quote_padding.saturating_mul(2)
            } else {
                0
            });
        let capacity = if pages.is_empty() {
            first_capacity
        } else {
            regular_capacity
        };
        let following_height = if unit.block.kind == "heading" {
            queue
                .peek()
                .map(|next| {
                    unit.gap_after
                        .saturating_add(next.block.height)
                        .saturating_add(if next.quote {
                            quote_padding.saturating_mul(2)
                        } else {
                            0
                        })
                })
                .unwrap_or(0)
        } else {
            0
        };
        if !page.is_empty()
            && (used.saturating_add(growth) > capacity
                || used.saturating_add(growth).saturating_add(following_height) > capacity)
        {
            push_article_page(&mut pages, page)?;
            page = LayoutVec::new(budget);
            used = 0;
            continue_quote = false;
        } else if page.is_empty()
            && pages.is_empty()
            && first_capacity < regular_capacity
            && (growth > first_capacity
                || (growth.saturating_add(following_height) > first_capacity
                    && growth.saturating_add(following_height) <= regular_capacity))
        {
            // The caller still renders the reserved heading on this empty page.
            pages.push(Vec::new())?;
        }
        let capacity = if pages.is_empty() {
            first_capacity
        } else {
            regular_capacity
        };
        if unit.block.height > capacity {
            if unit.block.kind == "image" {
                fit_image_to_capacity(&mut unit.block, capacity);
            } else if unit.block.kind == "spacing" {
                let remainder = unit.block.height - capacity;
                pending_spacing = Some(spacing_unit(
                    format_args!("{}-continued", unit.block.id),
                    remainder,
                    unit.source_index,
                    budget,
                    &mut unit_owner,
                )?);
                unit.block.height = capacity;
            } else {
                // Degenerate viewports may be shorter than a single glyph line.
                unit.block.height = capacity;
                unit.block.content_height = unit.block.content_height.min(capacity as f32);
            }
        }

        let mut removed_padding = 0;
        if unit.quote {
            let padding = if continue_quote {
                page.last()
                    .map(|previous| pixel_height_or_zero(previous.inset_bottom))
                    .unwrap_or(0)
            } else {
                quote_padding.min(capacity.saturating_sub(unit.block.height) / 2)
            };
            if continue_quote {
                if let Some(previous) = page.last_mut() {
                    removed_padding = pixel_height_or_zero(previous.inset_bottom);
                    previous.inset_bottom = 0.0;
                    previous.height -= removed_padding;
                }
            } else {
                unit.block.inset_top = padding as f32;
            }
            unit.block.inset_bottom = padding as f32;
            unit.block.height = unit
                .block
                .height
                .saturating_add(pixel_height_or_zero(unit.block.inset_top))
                .saturating_add(padding);
        }
        growth = unit.block.height.saturating_sub(removed_padding);
        let allocated_gap = unit
            .gap_after
            .min(capacity.saturating_sub(used.saturating_add(growth)));
        unit.block.height += allocated_gap;
        used = used.saturating_add(growth).saturating_add(allocated_gap);
        if allocated_gap < unit.gap_after {
            pending_spacing = Some(spacing_unit(
                format_args!("spacing-{}", unit.block.id),
                unit.gap_after - allocated_gap,
                unit.source_index,
                budget,
                &mut unit_owner,
            )?);
        }
        last_quote = if unit.quote {
            Some(unit.source_index)
        } else {
            None
        };
        page.push(unit.block)?;
    }
    if !page.is_empty() || pages.is_empty() {
        push_article_page(&mut pages, page)?;
    }
    drop(queue);
    unit_owner.release(unit_buffer);
    pages.adopt(unit_owner);
    let (pages, owner) = pages.into_parts();
    Ok(AdmittedLayout::new(pages, owner))
}

fn push_article_page(
    pages: &mut LayoutVec<Vec<PageBlock>>,
    page: LayoutVec<PageBlock>,
) -> Result<(), LayoutLimitError> {
    pages.reserve_one()?;
    let (page, owner) = page.into_parts();
    pages.push_owned(page, owner)
}

fn spacing_unit(
    id: std::fmt::Arguments<'_>,
    height: u32,
    source_index: usize,
    budget: &LayoutBudget,
    owner: &mut LayoutLease,
) -> Result<FlowUnit, LayoutLimitError> {
    budget.work(std::mem::size_of::<FlowUnit>())?;
    let (id, next) = budget.format(id)?;
    owner.merge(next);
    let (kind, next) = budget.copy("spacing")?;
    owner.merge(next);
    Ok(FlowUnit {
        block: PageBlock {
            id,
            kind,
            height,
            ..PageBlock::default()
        },
        source_index,
        quote: false,
        gap_after: 0,
    })
}

fn fit_image_to_capacity(block: &mut PageBlock, capacity: u32) {
    let available = capacity as f32;
    let caption = block.caption_height + block.caption_gap;
    if caption < available {
        block.frame_height = (available - caption).max(0.0);
    } else {
        // Keep the entire slot self-consistent even in an unusually short view.
        let total = block.frame_height + caption;
        let scale = available / total.max(1.0);
        block.caption_height *= scale;
        block.caption_gap *= scale;
        block.frame_height = (available - block.caption_height - block.caption_gap).max(0.0);
    }
    block.height = capacity;
}

struct Collector<'a> {
    blocks: LayoutVec<StagedBlock<'a>>,
    pending: LayoutString,
    pending_kind: TextKind,
    media_count: usize,
    styles: ArticleStyleWork<'a>,
    image_budget: image_budget::Budget,
    layout: LayoutBudget,
}

impl<'a> Collector<'a> {
    fn new(layout: &LayoutBudget, styles: ArticleStyleWork<'a>) -> Self {
        Self {
            blocks: LayoutVec::new(layout),
            pending: LayoutString::new(layout),
            pending_kind: TextKind::default(),
            media_count: 0,
            styles,
            image_budget: image_budget::Budget::default(),
            layout: layout.share(),
        }
    }

    fn flush(&mut self) -> Result<(), LayoutLimitError> {
        if self.pending.trim().is_empty() {
            self.pending.clear();
            return Ok(());
        }
        self.blocks.reserve_one()?;
        let (text, owner) = self.layout.copy(self.pending.trim())?;
        self.pending.clear();
        self.blocks.push_owned(
            StagedBlock::Text {
                kind: self.pending_kind,
                text,
            },
            owner,
        )?;
        Ok(())
    }

    fn append(&mut self, text: &str, kind: TextKind) -> Result<(), LayoutLimitError> {
        if kind != self.pending_kind {
            self.flush()?;
            self.pending_kind = kind;
        }
        for character in text.chars() {
            if character.is_whitespace() || character == '\u{00a0}' {
                if !self.pending.is_empty() && !self.pending.ends_with(' ') {
                    self.pending.push(' ')?;
                }
            } else if character != '\u{200b}' && character != '\u{feff}' {
                self.pending.push(character)?;
            }
        }
        Ok(())
    }

    fn visit(
        &mut self,
        element: ElementRef<'a>,
        inherited: TextKind,
    ) -> Result<(), ArticleParseError> {
        enum Work<'node> {
            Element(ElementRef<'node>, TextKind),
            Text(&'node str, TextKind),
            Flush,
        }
        let mut pending = LayoutVec::new(&self.layout);
        pending.push(Work::Element(element, inherited))?;
        while let Some(work) = pending.pop() {
            let (element, inherited) = match work {
                Work::Text(text, kind) => {
                    self.append(text, kind)?;
                    continue;
                }
                Work::Flush => {
                    self.flush()?;
                    continue;
                }
                Work::Element(element, inherited) => (element, inherited),
            };
            let node = element.value();
            let tag = node.name();
            if matches!(
                tag,
                "head" | "script" | "style" | "noscript" | "template" | "svg" | "canvas"
            ) || node.attr("hidden").is_some()
                || node.attr("aria-hidden") == Some("true")
                || node.attr("id") == Some("activity-name")
            {
                continue;
            }
            if let Some(classes) = node.attr("class") {
                if self.styles.suppressed_class(classes)? {
                    continue;
                }
            }
            if let Some(style) = node.attr("style") {
                if self.styles.suppresses(style)? {
                    continue;
                }
            }
            if matches!(
                tag,
                "video" | "audio" | "iframe" | "object" | "embed" | "mp-video" | "mpvoice"
            ) {
                self.media_count += 1;
                continue;
            }
            if tag == "img" {
                self.flush()?;
                self.add_image(element)?;
                continue;
            }
            if tag == "figcaption" {
                self.flush()?;
                self.blocks.push(StagedBlock::Caption {
                    element,
                    kind: inherited,
                })?;
                continue;
            }
            if tag == "br" {
                self.flush()?;
                continue;
            }
            let block = matches!(
                tag,
                "address"
                    | "article"
                    | "aside"
                    | "blockquote"
                    | "dd"
                    | "details"
                    | "div"
                    | "dl"
                    | "dt"
                    | "fieldset"
                    | "figure"
                    | "footer"
                    | "form"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "header"
                    | "hr"
                    | "li"
                    | "main"
                    | "nav"
                    | "ol"
                    | "p"
                    | "pre"
                    | "section"
                    | "summary"
                    | "table"
                    | "td"
                    | "th"
                    | "tr"
                    | "ul"
            );
            if block {
                self.flush()?;
                // This exit action runs after the complete subtree. Keeping it
                // on the heap preserves block boundaries without Rust recursion.
                pending.push(Work::Flush)?;
            }
            let kind = if matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                TextKind::Heading
            } else if tag == "blockquote" || inherited == TextKind::Quote {
                TextKind::Quote
            } else {
                inherited
            };
            for child in element.children().rev() {
                match child.value() {
                    Node::Text(text) => pending.push(Work::Text(text, kind))?,
                    Node::Element(_) => {
                        if let Some(element) = ElementRef::wrap(child) {
                            pending.push(Work::Element(element, kind))?;
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn add_image(&mut self, element: ElementRef<'a>) -> Result<(), ArticleParseError> {
        self.blocks.reserve_one()?;
        let ordinal = self.image_budget.records();
        self.image_budget.record()?;
        self.blocks.push(StagedBlock::Image { element, ordinal })?;
        Ok(())
    }

    fn finish_images(mut self) -> Result<LayoutVec<SourceBlock>, ArticleParseError> {
        // The final source slots are admitted while the complete staged flow
        // and pending text are still live, before image or caption outputs.
        let mut blocks = LayoutVec::with_capacity(&self.layout, self.blocks.len())?;
        let mut inputs = LayoutVec::with_capacity(&self.layout, self.image_budget.records())?;
        let mut potential_image = false;
        // Inspect all borrowed candidate fields before any URI parser, URL
        // clone, caption conversion, SHA, or output/index materialization.
        for block in self.blocks.iter() {
            match block {
                StagedBlock::Text { .. } => potential_image = false,
                StagedBlock::Image { element, .. } => {
                    potential_image = true;
                    let node = element.value();
                    let mut urls = LayoutVec::new(&self.layout);
                    for name in ["data-src", "data-original", "data-lazy-src", "src"] {
                        if let Some(raw) = node.attr(name) {
                            urls.reserve_one()?;
                            urls.push(self.image_budget.url(raw)?)?;
                        }
                    }
                    if let Some(set) = node.attr("srcset") {
                        self.image_budget.srcset(set.len())?;
                        for raw in set
                            .split(',')
                            .filter_map(|part| part.split_whitespace().next())
                        {
                            urls.reserve_one()?;
                            urls.push(self.image_budget.url(raw)?)?;
                        }
                    }
                    let alt = node.attr("alt").unwrap_or_default();
                    let size = node.attr("data-size").unwrap_or_default();
                    self.image_budget.caption(alt.len())?;
                    self.image_budget.conversion(size.len())?;
                    let (urls, owner) = urls.into_parts();
                    inputs.push_owned(ImageInput { urls, alt, size }, owner)?;
                }
                StagedBlock::Caption { element, .. } if potential_image => {
                    let bytes = element
                        .text()
                        .try_fold(0usize, |used, text| used.checked_add(text.len()))
                        .ok_or(ImageLimitError {
                            kind: ImageLimitKind::Caption,
                            used: 0,
                            requested: usize::MAX,
                            limit: 16 * 1024,
                        })?;
                    self.image_budget.caption(bytes)?;
                }
                StagedBlock::Caption { .. } => {}
            }
        }
        let mut ready = LayoutVec::with_capacity(&self.layout, inputs.len())?;
        let (inputs, input_owner) = inputs.into_parts();
        for input in inputs {
            let mut selected = None;
            for url in input.urls {
                if let Some(url) = safe_image_url(&url, &mut self.image_budget, &self.layout)? {
                    selected = Some(url);
                    break;
                }
            }
            if let Some((url, mut owner)) = selected {
                let (caption, next) = normalize_text_checked(input.alt, &self.layout)?;
                owner.merge(next);
                let (size, next) =
                    if let Some(size) = input.size.parse::<u64>().ok().filter(|size| *size > 0) {
                        self.layout
                            .format(format_args!("{} KB", size.div_ceil(1024)))?
                    } else {
                        (String::new(), self.layout.lease())
                    };
                owner.merge(next);
                ready.push_owned(Some(ReadyImage { url, caption, size }), owner)?;
            } else {
                ready.push(None)?;
            }
        }
        // Every URL candidate vector has now been consumed and dropped.
        drop(input_owner);
        let staged_buffer = self.blocks.buffer_bytes();
        let (staged, mut staged_owner) = self.blocks.into_parts();
        for block in staged {
            match block {
                StagedBlock::Text { kind, text } => {
                    blocks.push(SourceBlock::Text { kind, text })?
                }
                StagedBlock::Image { ordinal, .. } => {
                    let Some(image) = ready[ordinal].take() else {
                        continue;
                    };
                    self.image_budget.hash();
                    let _digest = self.layout.copy_storage(64)?;
                    let digest = hex::encode(Sha256::digest(image.url.as_bytes()));
                    let (id, owner) = self
                        .layout
                        .format(format_args!("image-{}", &digest[..20]))?;
                    blocks.push_owned(
                        SourceBlock::Image {
                            image: ArticleImage {
                                id,
                                url: image.url,
                                caption: image.caption,
                            },
                            size: image.size,
                        },
                        owner,
                    )?;
                }
                StagedBlock::Caption { element, kind } => {
                    let mut raw = LayoutString::new(&self.layout);
                    for text in element.text() {
                        raw.append(text)?;
                    }
                    let (caption, owner) = normalize_text_checked(&raw, &self.layout)?;
                    if let Some(SourceBlock::Image { image, .. }) = blocks.last_mut() {
                        if !caption.is_empty() {
                            image.caption = caption;
                            blocks.adopt(owner);
                        }
                    } else if !caption.is_empty() {
                        blocks.push_owned(
                            SourceBlock::Text {
                                kind,
                                text: caption,
                            },
                            owner,
                        )?;
                    }
                }
            }
        }
        staged_owner.release(staged_buffer);
        blocks.adopt(staged_owner);
        let ready_buffer = ready.buffer_bytes();
        let (ready, mut ready_owner) = ready.into_parts();
        drop(ready);
        ready_owner.release(ready_buffer);
        blocks.adopt(ready_owner);
        Ok(blocks)
    }
}

fn safe_image_url(
    token: &image_budget::AdmittedUrl<'_>,
    budget: &mut image_budget::Budget,
    layout: &LayoutBudget,
) -> Result<Option<(String, LayoutLease)>, ArticleParseError> {
    let value = token.raw().trim();
    if value.is_empty() {
        return Ok(None);
    }
    budget.input_copy();
    let (normalized, _normalized_owner) = if value.starts_with("//") {
        layout.format(format_args!("https:{value}"))?
    } else {
        layout.copy(value)?
    };
    budget.parse();
    let Ok(parsed) = Url::parse(&normalized) else {
        return Ok(None);
    };
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Ok(None);
    }
    budget.normalized(parsed.as_str(), token)?;
    budget.output_copy();
    Ok(Some(layout.copy(parsed.as_str())?))
}

fn normalize_text_checked(
    text: &str,
    budget: &LayoutBudget,
) -> Result<(String, LayoutLease), LayoutLimitError> {
    budget.work(text.len())?;
    let mut result = LayoutString::new(budget);
    for character in text.chars() {
        if character.is_whitespace() || character == '\u{00a0}' {
            if !result.is_empty() && !result.ends_with(' ') {
                result.push(' ')?;
            }
        } else if character != '\u{200b}' && character != '\u{feff}' {
            result.push(character)?;
        }
    }
    budget.copy(result.trim())
}

fn normalize_text(text: &str) -> String {
    let mut result = String::new();
    for character in text.chars() {
        if character.is_whitespace() || character == '\u{00a0}' {
            if !result.is_empty() && !result.ends_with(' ') {
                result.push(' ');
            }
        } else if character != '\u{200b}' && character != '\u{feff}' {
            result.push(character);
        }
    }
    result.trim().to_owned()
}

#[cfg(test)]
#[path = "article_deep_tests.rs"]
mod deep_tests;

#[cfg(test)]
#[path = "article_anchor_budget_tests.rs"]
mod anchor_budget_tests;

#[cfg(test)]
#[path = "article_style_budget_tests.rs"]
pub(crate) mod style_budget_tests;

#[cfg(test)]
#[path = "article_image_budget_tests.rs"]
pub(crate) mod image_budget_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout {
            width: 240.0,
            height: 300.0,
            font_size: 19.0,
            line_height: 1.85,
            indent: 0,
            paragraph_gap: 0.9,
            justify: false,
        }
    }

    #[test]
    fn nested_blocks_are_extracted_once_in_document_order() {
        let document = ArticleDocument::parse(
            "<div><p>First <strong>paragraph</strong>.</p><div><p>Second paragraph.</p></div></div>",
            "unused fallback",
        );
        assert_eq!(document.plain_text(), "First paragraph.\nSecond paragraph.");
    }

    #[test]
    fn images_stay_between_their_text_blocks_and_load_only_when_supplied() {
        let document = ArticleDocument::parse(
            "<p>Before</p><figure><img data-src='https://example.com/image.jpg' alt='An image'><figcaption>A caption</figcaption></figure><p>After</p>",
            "",
        );
        let images = document.images();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].caption, "A caption");
        let pages = document.paginate(
            &Layout {
                height: 1000.0,
                ..layout()
            },
            &TextMetrics::default(),
            &BTreeSet::new(),
        );
        assert_eq!(pages[0][0].text, "Before");
        assert_eq!(pages[0][1].kind, "image");
        assert_eq!(pages[0][2].text, "After");
        assert!(!pages[0][1].loaded);
        let loaded = BTreeSet::from([images[0].id.clone()]);
        let loaded_pages = document.paginate(
            &Layout {
                height: 1000.0,
                ..layout()
            },
            &TextMetrics::default(),
            &loaded,
        );
        assert!(loaded_pages[0][1].loaded);
        assert_eq!(pages[0][1].height, 208);
        assert_eq!(loaded_pages[0][1].height, 248);
        assert_eq!(pages[0][1].frame_height, 180.0);
        assert_eq!(loaded_pages[0][1].frame_height, 220.0);
        assert_eq!(loaded_pages[0][1].id, pages[0][1].id);
    }

    #[test]
    fn image_geometry_is_supplied_by_the_caller_and_includes_the_caption() {
        let document = ArticleDocument::parse(
            "<img src='https://example.com/image.png' alt='A caption'>",
            "",
        );
        let image = &document.images()[0];
        let large_layout = Layout {
            height: 1000.0,
            ..layout()
        };
        let metrics = TextMetrics::default();
        for (images, expected) in [
            (ArticleImageGeometry::default(), [208, 248]),
            (
                ArticleImageGeometry {
                    placeholder_height: 220.0,
                    loaded_height: 300.0,
                    caption_height: 24.0,
                    caption_gap: 10.0,
                },
                [254, 334],
            ),
        ] {
            for (loaded, expected_height) in [false, true].into_iter().zip(expected) {
                let loaded_images = if loaded {
                    BTreeSet::from([image.id.clone()])
                } else {
                    BTreeSet::new()
                };
                let pages = document.paginate_with_options(
                    &large_layout,
                    &metrics,
                    &loaded_images,
                    large_layout.height,
                    &ArticleLayoutOptions {
                        images,
                        ..ArticleLayoutOptions::default()
                    },
                );
                let block = &pages[0][0];
                assert_eq!(block.height, expected_height);
                assert_eq!(block.loaded, loaded);
                assert_eq!(block.id, image.id);
                assert_eq!(block.image_url.as_deref(), Some(image.url.as_str()));
                assert_eq!(
                    block.frame_height + block.caption_height + block.caption_gap,
                    block.height as f32
                );
            }
        }
    }

    #[test]
    fn images_move_past_a_short_first_page_before_the_frame_is_resized() {
        let document = ArticleDocument::parse(
            "<img src='https://example.com/image.png' alt='A caption'>",
            "",
        );
        let metrics = TextMetrics::default();
        for (regular_height, expected_frame) in [(300.75, 180.0), (180.75, 152.0)] {
            let image_layout = Layout {
                height: regular_height,
                ..layout()
            };
            let pages = document.paginate_with_first_page_height(
                &image_layout,
                &metrics,
                &BTreeSet::new(),
                100.75,
            );
            assert!(pages[0].is_empty());
            let image = &pages[1][0];
            assert_eq!(image.kind, "image");
            assert_eq!(image.frame_height, expected_frame);
            assert_eq!(image.caption_height, 20.0);
            assert_eq!(image.caption_gap, 8.0);
            assert!(
                image.frame_height + image.caption_height + image.caption_gap
                    <= image.height as f32
            );
            for (index, page) in pages.iter().enumerate() {
                let capacity = if index == 0 {
                    100
                } else {
                    regular_height.floor() as u32
                };
                assert!(page.iter().map(|block| block.height).sum::<u32>() <= capacity);
            }
            assert_eq!(
                pages
                    .iter()
                    .flatten()
                    .filter(|block| block.kind == "image")
                    .count(),
                1
            );
        }
    }

    #[test]
    fn justification_changes_only_nonfinal_text_lines_without_losing_unicode() {
        let text = "\u{7532}\u{4e59}e\u{0301}\u{1f642}\u{fe0f} narrow and wide WWW ".repeat(12);
        let document = ArticleDocument::parse(&format!("<h2>A heading</h2><p>{text}</p>"), "");
        let metrics = TextMetrics::default();
        let justified_layout = Layout {
            width: 111.0,
            height: 1000.0,
            justify: true,
            ..layout()
        };
        let justified = document.paginate(&justified_layout, &metrics, &BTreeSet::new());
        let ragged = document.paginate(
            &Layout {
                justify: false,
                ..justified_layout
            },
            &metrics,
            &BTreeSet::new(),
        );
        let text_blocks = |pages: &[Vec<PageBlock>]| {
            pages
                .iter()
                .flatten()
                .filter(|block| block.id.starts_with("text-"))
                .map(|block| (block.id.clone(), block.text.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(text_blocks(&justified), text_blocks(&ragged));
        assert!(
            justified
                .iter()
                .flatten()
                .any(|block| block.letter_spacing > 0.0)
        );
        for block in justified.iter().flatten() {
            assert!(block.letter_spacing.is_finite() && block.letter_spacing >= 0.0);
            if block.kind == "paragraph" || block.kind == "heading" {
                assert_eq!(block.letter_spacing, 0.0);
            }
        }
        assert!(
            ragged
                .iter()
                .flatten()
                .all(|block| block.letter_spacing == 0.0)
        );
        let rendered = justified
            .iter()
            .flatten()
            .filter(|block| block.id.starts_with("text-1-"))
            .map(|block| block.text.as_str())
            .collect::<String>();
        assert_eq!(rendered, text.trim());
        assert_eq!(
            justified_letter_spacing("e\u{0301}", 10.0, &justified_layout, &metrics, false),
            0.0
        );
    }

    #[test]
    fn reflow_preserves_source_text_and_height_only_changes_keep_line_ids() {
        let paragraph = "\u{7532}\u{1f642}e\u{0301} text with punctuation, and spaces. ".repeat(14);
        let quotation = "A quoted sentence stays in its original order. ".repeat(12);
        let document = ArticleDocument::parse(
            &format!(
                "<p>{paragraph}</p><blockquote>{quotation}</blockquote><img src='https://example.com/image.png'><p>After the image.</p>"
            ),
            "",
        );
        let metrics = TextMetrics::default();
        let mut baseline = None;
        for (width, height) in [(240.0, 300.0), (240.0, 150.0), (170.0, 220.0)] {
            let flow_layout = Layout {
                width,
                height,
                justify: true,
                ..layout()
            };
            let pages = document.paginate_with_first_page_height(
                &flow_layout,
                &metrics,
                &BTreeSet::new(),
                120.0,
            );
            let text_blocks: Vec<_> = pages
                .iter()
                .flatten()
                .filter(|block| block.id.starts_with("text-"))
                .map(|block| (block.id.clone(), block.text.clone()))
                .collect();
            if width == 240.0 {
                if let Some(expected) = &baseline {
                    assert_eq!(&text_blocks, expected);
                } else {
                    baseline = Some(text_blocks.clone());
                }
            }
            for (source_index, source) in document.blocks.iter().enumerate() {
                let SourceBlock::Text { text, .. } = source else {
                    continue;
                };
                let prefix = format!("text-{source_index}-");
                let mut actual = String::new();
                for (id, line) in text_blocks.iter().filter(|(id, _)| id.starts_with(&prefix)) {
                    let offset: usize = id.strip_prefix(prefix.as_str()).unwrap().parse().unwrap();
                    assert_eq!(offset, actual.chars().count());
                    actual.push_str(line);
                }
                assert_eq!(&actual, text);
            }
            for (index, page) in pages.iter().enumerate() {
                assert!(
                    page.iter().map(|block| block.height).sum::<u32>()
                        <= if index == 0 { 120 } else { height as u32 }
                );
            }
        }
    }

    #[test]
    fn paragraph_spacing_is_retained_when_the_remaining_gap_crosses_a_page() {
        let document = ArticleDocument::parse("<p>A</p><p>B</p>", "");
        let metrics = TextMetrics::default();
        let mut allocated = Vec::new();
        for height in [300.0, 40.75] {
            let pages =
                document.paginate(&Layout { height, ..layout() }, &metrics, &BTreeSet::new());
            allocated.push(
                pages
                    .iter()
                    .flatten()
                    .filter(|block| block.kind != "end")
                    .map(|block| block.height)
                    .sum::<u32>(),
            );
            assert_eq!(
                pages
                    .iter()
                    .flatten()
                    .filter(|block| block.kind == "paragraph")
                    .map(|block| block.text.as_str())
                    .collect::<String>(),
                "AB"
            );
            assert!(
                pages
                    .iter()
                    .all(|page| page.iter().map(|block| block.height).sum::<u32>()
                        <= height.floor() as u32)
            );
        }
        assert_eq!(allocated[0], allocated[1]);
    }

    #[test]
    fn quotes_use_the_supplied_metrics_and_each_page_fragment_has_its_own_padding() {
        use std::io::Read;

        let font = |name| {
            let bytes = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("assets/fonts")
                    .join(name),
            )
            .expect("Read a bundled font");
            let mut decoded = Vec::new();
            flate2::read::GzDecoder::new(bytes.as_slice())
                .read_to_end(&mut decoded)
                .expect("Decompress a bundled font");
            TextMetrics::from_font_owned(decoded, 0).expect("Parse a bundled font")
        };
        let body_metrics = font("NotoSansSC-Regular.otf.gz");
        let quote_metrics = font("NotoSerifSC-Regular.otf.gz");
        assert_ne!(
            body_metrics.measure("Wiii", 19.0),
            quote_metrics.measure("Wiii", 19.0)
        );
        let text = "A minimum of wide WWW letters and narrow iii letters. ".repeat(20);
        let document = ArticleDocument::parse(&format!("<blockquote>{text}</blockquote>"), "");
        let quote_layout = Layout {
            height: 200.0,
            justify: true,
            ..layout()
        };
        let options = ArticleLayoutOptions {
            quote: ArticleQuoteGeometry {
                horizontal_padding: 36.0,
                vertical_padding: 22.0,
                font_scale: 0.95,
                line_height: 1.85,
            },
            quote_metrics: Some(&quote_metrics),
            ..ArticleLayoutOptions::default()
        };
        let pages = document.paginate_with_options(
            &quote_layout,
            &body_metrics,
            &BTreeSet::new(),
            160.0,
            &options,
        );
        let expected_layout = Layout {
            width: quote_layout.width - 72.0,
            height: 1_000_000.0,
            font_size: quote_layout.font_size * 0.95,
            line_height: 1.85,
            paragraph_gap: 0.0,
            ..quote_layout
        };
        let expected: Vec<_> = Paginator::new(text.trim(), &expected_layout, &quote_metrics)
            .pages
            .into_iter()
            .flat_map(|page| page.lines)
            .map(|line| line.text)
            .collect();
        let actual: Vec<_> = pages
            .iter()
            .flatten()
            .filter(|block| block.kind == "quote")
            .map(|block| block.text.clone())
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(actual.concat(), text.trim());
        for (index, page) in pages.iter().enumerate() {
            assert!(
                page.iter().map(|block| block.height).sum::<u32>()
                    <= if index == 0 { 160 } else { 200 }
            );
            let quotes: Vec<_> = page.iter().filter(|block| block.kind == "quote").collect();
            if quotes.is_empty() {
                continue;
            }
            assert_eq!(quotes.first().unwrap().inset_top, 22.0);
            assert_eq!(quotes.last().unwrap().inset_bottom, 22.0);
            for (line_index, block) in quotes.iter().enumerate() {
                assert_eq!(block.inset_horizontal, 36.0);
                assert_eq!(block.content_height, expected_layout.line_advance());
                if line_index > 0 {
                    assert_eq!(block.inset_top, 0.0);
                }
                if line_index + 1 < quotes.len() {
                    assert_eq!(block.inset_bottom, 0.0);
                }
                assert!(
                    block.content_height + block.inset_top + block.inset_bottom
                        <= block.height as f32
                );
            }
        }
    }

    #[test]
    fn executable_hidden_and_media_content_is_not_rendered() {
        let document = ArticleDocument::parse(
            "<script>secret()</script><style>.x{}</style><p hidden>hidden</p><p style='display: none'>hidden2</p><video>video fallback</video><audio>audio fallback</audio><p>Visible &amp; readable.</p>",
            "",
        );
        assert_eq!(document.plain_text(), "Visible & readable.");
        assert_eq!(document.media_count, 2);
    }

    #[test]
    fn content_container_excludes_the_web_page_shell() {
        let document = ArticleDocument::parse(
            "<h1 id='activity-name'>Title handled by app</h1><p>web chrome</p><div id='js_content'><h2>A heading</h2><p>Body</p><blockquote><p>A quotation</p></blockquote></div><footer>web footer</footer>",
            "",
        );
        assert_eq!(document.plain_text(), "A heading\nBody\nA quotation");
        let blocks: Vec<_> = document
            .paginate(&layout(), &TextMetrics::default(), &BTreeSet::new())
            .into_iter()
            .flatten()
            .collect();
        assert!(blocks.iter().any(|block| block.kind == "heading"));
        assert!(blocks.iter().any(|block| block.kind == "quote"));
    }

    #[test]
    fn unsafe_images_are_rejected_and_protocol_relative_https_is_supported() {
        let document = ArticleDocument::parse(
            "<img src='javascript:alert(1)'><img src='file:///etc/passwd'><img src='data:image/png;base64,abc'><img src='http://example.com/insecure.png'><img data-src='//example.com/secure.png'>",
            "",
        );
        assert_eq!(document.images().len(), 1);
        assert_eq!(document.images()[0].url, "https://example.com/secure.png");
    }

    #[test]
    fn pagination_keeps_order_and_respects_first_page_capacity() {
        let text = (0..80)
            .map(|index| format!("Sentence {index}. "))
            .collect::<String>();
        let document = ArticleDocument::parse(&format!("<p>{text}</p>"), "");
        let pages = document.paginate_with_first_page_height(
            &layout(),
            &TextMetrics::default(),
            &BTreeSet::new(),
            120.0,
        );
        assert!(pages.len() > 2);
        for (index, page) in pages.iter().enumerate() {
            let capacity = if index == 0 { 120 } else { 300 };
            assert!(page.iter().map(|block| block.height).sum::<u32>() <= capacity);
        }
        let rendered = pages
            .iter()
            .flatten()
            .filter(|block| block.kind != "end")
            .map(|block| block.text.as_str())
            .collect::<String>();
        assert_eq!(rendered, text.trim());
    }

    #[test]
    fn malformed_or_empty_html_uses_plain_text_without_losing_unicode() {
        let fallback = "\u{6843}\u{82b1}\u{6e90}\u{8bb0}\n\u{6b63}\u{6587}\u{3002}";
        let document = ArticleDocument::parse("<script>only script</script>", fallback);
        assert_eq!(document.plain_text(), fallback);
        assert!(ArticleDocument::parse("", "").is_empty());
    }

    #[test]
    fn long_quotes_can_continue_across_pages_without_overflow_or_duplication() {
        let text = "A quotation with words that must remain in their original order. ".repeat(30);
        let document =
            ArticleDocument::parse(&format!("<blockquote><p>{text}</p></blockquote>"), "");
        let pages = document.paginate_with_first_page_height(
            &layout(),
            &TextMetrics::default(),
            &BTreeSet::new(),
            160.0,
        );
        assert!(pages.len() > 2);
        for (index, page) in pages.iter().enumerate() {
            assert!(
                page.iter().map(|block| block.height).sum::<u32>()
                    <= if index == 0 { 160 } else { 300 }
            );
        }
        let actual = pages
            .iter()
            .flatten()
            .filter(|block| block.kind == "quote")
            .flat_map(|block| block.text.split('\n'))
            .collect::<String>();
        assert_eq!(actual, text.trim());
    }

    #[test]
    fn image_anchor_offsets_address_original_unicode_scalars() {
        let text = "\u{7532} & \u{1f642}\u{4e59}";
        let images = image_anchors(
            "<p>\u{7532}<span> &amp; </span>\u{1f642}<img src='../images/a&amp;b.png'>\u{4e59}</p>",
            text,
        )
        .unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].source, "../images/a&b.png");
        assert_eq!(images[0].anchor, Some(5));
        assert_eq!(text.chars().nth(5), Some('\u{4e59}'));
    }

    #[test]
    fn image_anchor_keeps_raw_sources_without_weakening_article_url_rules() {
        let html = "<img src='../archive/plot.png?x=1&amp;y=2' data-src='https://example.com/other.png'><img data-src='relative-lazy.png'>";
        let images = image_anchors(html, "").unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].source, "../archive/plot.png?x=1&y=2");
        assert_eq!(images[1].source, "relative-lazy.png");
        assert!(images.iter().all(|image| image.anchor == Some(0)));
        let article = ArticleDocument::parse("<img src='../archive/plot.png'>", "");
        assert!(article.images().is_empty());
    }

    #[test]
    fn image_anchor_leading_and_trailing_occurrences_have_distinct_ids() {
        let text = " \nA\u{1f642} \n";
        let html = "<img src='same.png'><p>A\u{1f642}</p><img src='same.png'>";
        let images = image_anchors(html, text).unwrap();
        assert_eq!(images[0].anchor, Some(0));
        assert_eq!(images[1].anchor, Some(text.chars().count() as u32));
        assert_eq!(images[0].source, images[1].source);
        assert_ne!(images[0].id, images[1].id);
        assert_eq!(images, image_anchors(html, text).unwrap());
    }

    #[test]
    fn image_anchor_repeated_paragraphs_advance_monotonically() {
        let images = image_anchors(
            "<p>Same</p><img src='first.png'><p>Same</p><img src='last.png'>",
            "Same\nSame",
        )
        .unwrap();
        assert_eq!(images[0].anchor, Some(4));
        assert_eq!(images[1].anchor, Some(9));
    }

    #[test]
    fn image_anchor_never_searches_ahead_after_a_missing_prefix() {
        let images = image_anchors(
            "<p>Missing prefix</p><img src='first.png'><p>Known</p><img src='last.png'>",
            "Known",
        )
        .unwrap();
        assert!(images.iter().all(|image| image.anchor.is_none()));
        assert_eq!(
            image_anchors("<p>Before</p><img src='last.png'>", "").unwrap()[0].anchor,
            None
        );
    }

    #[test]
    fn image_anchor_retains_proven_prefixes_when_later_text_differs() {
        let images = image_anchors(
            "<p>Before</p><img src='first.png'><p>Changed</p><img src='last.png'>",
            "Before\nAfter",
        )
        .unwrap();
        assert_eq!(images[0].anchor, Some(6));
        assert_eq!(images[1].anchor, None);
    }

    #[test]
    fn image_anchor_captions_can_be_included_or_omitted_from_canonical_text() {
        let html = "<p>Before</p><figure><img src='first.png' alt='Fallback'><figcaption>A <em>caption</em></figcaption></figure><p>After</p><img src='last.png'>";
        for text in ["Before\nA caption\nAfter", "Before\nAfter"] {
            let images = image_anchors(html, text).unwrap();
            assert_eq!(images[0].anchor, Some(6));
            assert_eq!(images[0].caption, "A caption");
            assert_eq!(images[1].anchor, Some(text.chars().count() as u32));
        }
    }

    #[test]
    fn image_anchor_later_text_disambiguates_repeated_caption_content() {
        let html = "<p>A</p><figure><img src='first.png'><figcaption>B</figcaption></figure><img src='middle.png'><p>B</p><img src='last.png'>";
        let omitted = image_anchors(html, "AB").unwrap();
        assert_eq!(omitted[1].anchor, Some(1));
        assert_eq!(omitted[2].anchor, Some(2));
        let included = image_anchors(html, "ABB").unwrap();
        assert_eq!(included[1].anchor, Some(2));
        assert_eq!(included[2].anchor, Some(3));
    }

    #[test]
    fn image_anchor_does_not_choose_an_ambiguous_caption_path() {
        let images = image_anchors(
            "<p>A</p><figure><img src='first.png'><figcaption>B</figcaption></figure><img src='middle.png'><p>Missing</p>",
            "AB",
        ).unwrap();
        assert_eq!(images[0].anchor, Some(1));
        assert_eq!(images[1].anchor, None);
    }

    #[test]
    fn image_anchor_omits_nonvisible_text_and_hidden_images() {
        let images = image_anchors(
            "<head><title>Shell</title></head><script>hidden</script><p hidden>Hidden</p><img hidden src='hidden.png'><p>Visible</p><img src='visible.png'>",
            "Visible",
        ).unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].source, "visible.png");
        assert_eq!(images[0].anchor, Some(7));
    }
}
