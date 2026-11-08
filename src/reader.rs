use std::sync::Arc;

use owned_ttf_parser::{AsFaceRef, OwnedFace};

pub(crate) mod layout_budget;
#[cfg(test)]
#[path = "reader_layout_budget_tests.rs"]
mod layout_budget_tests;
pub use layout_budget::{
    AdmittedLayout, LayoutBudget, LayoutLimitError, LayoutLimitKind, LayoutLimits, LayoutUsage,
};
use layout_budget::{LayoutLease, LayoutString, LayoutVec};

pub const DEFAULT_CARET_WORK: usize = 64 * 1024 * 1024;
const MAX_CARET_INPUT_BYTES: usize = 16 * 1024;
const MAX_CARET_POSITIONS: usize = 8193;
pub(crate) const MAX_CARET_PAGE_LINES: usize = 256;

#[cfg(test)]
#[path = "reader_caret_tests.rs"]
mod caret_tests;

#[cfg(test)]
#[path = "reader_image_anchor_work_tests.rs"]
mod image_anchor_work_tests;

#[cfg(test)]
#[path = "reader_caret_font.rs"]
pub(crate) mod caret_test_font;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaretError {
    #[error("caret text exceeds its input allowance")]
    Input,
    #[error("caret positions exceed their count allowance")]
    Positions,
    #[error("caret geometry exceeds its work allowance")]
    Work,
    #[error("caret text does not match its scalar anchors")]
    Anchors,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CaretStats {
    pub admitted: usize,
    pub allocations: usize,
    pub measured: usize,
    pub boundaries: usize,
    pub rejected: usize,
}

/// One operation's independent allowance. Public configuration can tighten,
/// never bypass, the application ceiling. No chapter or geometry is retained.
pub struct CaretBudget {
    work_left: usize,
    positions_left: usize,
    #[cfg(test)]
    stats: CaretStats,
}

impl Default for CaretBudget {
    fn default() -> Self {
        Self::with_work_limit(DEFAULT_CARET_WORK)
    }
}

impl CaretBudget {
    pub fn with_work_limit(work: usize) -> Self {
        Self {
            work_left: work.min(DEFAULT_CARET_WORK),
            positions_left: MAX_CARET_POSITIONS,
            #[cfg(test)]
            stats: CaretStats::default(),
        }
    }

    pub(crate) fn admit(&mut self, line: &Line) -> Result<CaretReservation, CaretError> {
        let admitted = self.admit_inner(line);
        #[cfg(test)]
        if admitted.is_err() {
            self.stats.rejected += 1;
        }
        admitted
    }

    fn admit_inner(&mut self, line: &Line) -> Result<CaretReservation, CaretError> {
        let bytes = line.text.len();
        if bytes > MAX_CARET_INPUT_BYTES {
            return Err(CaretError::Input);
        }
        let scalars = line
            .end
            .checked_sub(line.start)
            .ok_or(CaretError::Anchors)? as usize;
        let positions = scalars.checked_add(1).ok_or(CaretError::Positions)?;
        if positions > self.positions_left {
            return Err(CaretError::Positions);
        }
        // Prepay each possible equal-caret history visit, plus count, spacing,
        // measurement, push, endpoint lookup and cluster scans. UTF-8 bytes
        // bound scalar count without measuring a glyph or creating a Vec.
        let work = bytes
            .checked_add(1)
            .and_then(|next| bytes.checked_mul(next))
            .map(|quadratic| quadratic / 2)
            .and_then(|quadratic| {
                bytes
                    .checked_mul(8)
                    .and_then(|linear| quadratic.checked_add(linear))
            })
            .and_then(|work| work.checked_add(1))
            .ok_or(CaretError::Work)?;
        if work > self.work_left {
            return Err(CaretError::Work);
        }
        self.work_left -= work;
        if line.text.chars().count() != scalars {
            return Err(CaretError::Anchors);
        }
        self.positions_left -= positions;
        #[cfg(test)]
        {
            self.stats.admitted += 1;
        }
        Ok(CaretReservation {
            positions,
            bytes,
            start: line.start,
            end: line.end,
        })
    }

    fn observe(&mut self, step: CaretStep) {
        #[cfg(test)]
        match step {
            CaretStep::Character => self.stats.measured += 1,
            CaretStep::Boundary => self.stats.boundaries += 1,
        }
        #[cfg(not(test))]
        let _ = step;
    }

    #[cfg(test)]
    pub(crate) fn stats(&self) -> CaretStats {
        self.stats
    }
}

pub(crate) struct CaretReservation {
    positions: usize,
    bytes: usize,
    start: u32,
    end: u32,
}

/// A single admitted line's scalar coordinates, reused within one operation.
/// It owns only a bounded coordinate vector, never the document or its quotes.
#[derive(Debug)]
pub struct CaretGeometry {
    positions: Vec<f32>,
    start: u32,
    end: u32,
    pub spacing: f32,
}

impl CaretGeometry {
    pub fn x_at_offset(&self, offset: u32) -> f32 {
        let index = offset
            .clamp(self.start, self.end)
            .saturating_sub(self.start) as usize;
        self.positions[index]
    }

    pub fn offset_at_x(&self, x: f32) -> Option<u32> {
        if x.is_nan() {
            return None;
        }
        for (index, pair) in self.positions.windows(2).enumerate() {
            if x < (pair[0] + pair[1]) * 0.5 {
                return Some((self.start + index as u32).min(self.end));
            }
        }
        Some(self.end)
    }

    pub fn range_at_x(&self, x: f32) -> Option<(u32, u32)> {
        if x.is_nan() {
            return None;
        }
        if self.start == self.end {
            return Some((self.start, self.start));
        }
        let mut selected = None;
        for (index, pair) in self.positions.windows(2).enumerate() {
            if pair[1] - pair[0] <= 0.001 {
                continue;
            }
            selected = Some(index);
            if x < pair[1] {
                break;
            }
        }
        let Some(start) = selected else {
            return Some((self.start, self.end));
        };
        let mut end = start + 1;
        while end + 1 < self.positions.len()
            && (self.positions[end + 1] - self.positions[end]).abs() <= 0.001
        {
            end += 1;
        }
        Some((
            (self.start + start as u32).min(self.end),
            (self.start + end as u32).min(self.end),
        ))
    }
}

/// Logical-pixel text layout. The platform layer owns physical scaling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub font_size: f32,
    /// A multiplier of `font_size`, rather than a pixel value.
    pub line_height: f32,
    /// First-line indentation, measured in full-width characters.
    pub indent: usize,
    /// Additional paragraph spacing, measured in em.
    pub paragraph_gap: f32,
    pub justify: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            width: 568.0,
            height: 630.0,
            font_size: 22.0,
            line_height: 1.8,
            indent: 2,
            paragraph_gap: 0.0,
            justify: true,
        }
    }
}

impl Layout {
    fn sanitized(self) -> Self {
        Self {
            width: finite_or(self.width, 568.0).max(1.0),
            height: finite_or(self.height, 630.0).max(1.0),
            font_size: finite_or(self.font_size, 22.0).clamp(8.0, 96.0),
            line_height: finite_or(self.line_height, 1.8).clamp(1.0, 4.0),
            indent: self.indent.min(8),
            paragraph_gap: finite_or(self.paragraph_gap, 0.0).clamp(0.0, 4.0),
            justify: self.justify,
        }
    }

    pub fn line_advance(self) -> f32 {
        let value = self.sanitized();
        value.font_size * value.line_height
    }
}

fn finite_or(value: f32, default: f32) -> f32 {
    if value.is_finite() { value } else { default }
}

/// Font-derived advances, with an explicit approximate fallback for missing fonts.
#[derive(Clone, Default)]
pub struct TextMetrics {
    font: Option<Arc<OwnedFace>>,
}

impl TextMetrics {
    pub fn from_font(bytes: impl AsRef<[u8]>, collection_index: u32) -> Result<Self, String> {
        Self::from_font_owned(bytes.as_ref().to_vec(), collection_index)
    }

    /// Retain the supplied bytes without copying or pre-parsing glyph outlines.
    pub fn from_font_owned(bytes: Vec<u8>, collection_index: u32) -> Result<Self, String> {
        let font =
            OwnedFace::from_vec(bytes, collection_index).map_err(|error| error.to_string())?;
        Ok(Self {
            font: Some(Arc::new(font)),
        })
    }

    pub fn is_font_backed(&self) -> bool {
        self.font.is_some()
    }

    pub fn advance(&self, character: char, font_size: f32) -> f32 {
        if is_zero_width(character) {
            return 0.0;
        }
        if character == '\t' {
            return font_size * 2.0;
        }
        if character.is_control() {
            return 0.0;
        }
        if let Some(font) = &self.font {
            let face = font.as_face_ref();
            if let Some(glyph) = face.glyph_index(character).filter(|glyph| glyph.0 != 0) {
                if let Some(advance) = face.glyph_hor_advance(glyph) {
                    return advance as f32 * font_size / face.units_per_em().max(1) as f32;
                }
            }
        }
        approximate_advance(character, font_size)
    }

    pub fn kerning(&self, left: char, right: char, font_size: f32) -> f32 {
        let Some(font) = &self.font else { return 0.0 };
        let face = font.as_face_ref();
        let (Some(left), Some(right), Some(table)) = (
            face.glyph_index(left),
            face.glyph_index(right),
            face.tables().kern,
        ) else {
            return 0.0;
        };
        let mut units = 0i32;
        for subtable in table.subtables {
            if subtable.horizontal && !subtable.variable && !subtable.has_cross_stream {
                units += subtable.glyphs_kerning(left, right).unwrap_or(0) as i32;
            }
        }
        units as f32 * font_size / face.units_per_em().max(1) as f32
    }

    pub fn measure(&self, text: &str, font_size: f32) -> f32 {
        let mut previous = None;
        text.chars().fold(0.0, |width, character| {
            let kern = previous
                .map(|left| self.kerning(left, character, font_size))
                .unwrap_or(0.0);
            previous = Some(character);
            width + kern + self.advance(character, font_size)
        })
    }

    fn measure_with_budget(
        &self,
        text: &str,
        font_size: f32,
        budget: &LayoutBudget,
    ) -> Result<f32, LayoutLimitError> {
        let mut previous = None;
        let mut width = 0.0;
        for character in text.chars() {
            budget.scan_step()?;
            let kern = previous
                .map(|left| self.kerning(left, character, font_size))
                .unwrap_or(0.0);
            previous = Some(character);
            width = width + kern + self.advance(character, font_size);
        }
        Ok(width)
    }
}

fn approximate_advance(character: char, size: f32) -> f32 {
    if character == ' ' || character == '\u{00a0}' {
        size * 0.3
    } else if character.is_ascii() {
        let factor = match character {
            'i' | 'l' | 'I' | '.' | ',' | ':' | ';' | '!' | '\'' | '|' => 0.3,
            'm' | 'w' | 'M' | 'W' | '@' => 0.9,
            _ => 0.58,
        };
        size * factor
    } else {
        size
    }
}

fn is_zero_width(character: char) -> bool {
    matches!(character,
        '\u{0300}'..='\u{036f}' | '\u{1ab0}'..='\u{1aff}' |
        '\u{1dc0}'..='\u{1dff}' | '\u{20d0}'..='\u{20ff}' |
        '\u{fe00}'..='\u{fe0f}' | '\u{fe20}'..='\u{fe2f}' |
        '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' |
        '\u{2060}'..='\u{2064}' | '\u{e0100}'..='\u{e01ef}')
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub text: String,
    /// Half-open offsets into the original chapter, measured in Unicode scalars.
    pub start: u32,
    pub end: u32,
    pub indent: bool,
    pub paragraph_end: bool,
    pub width: f32,
    pub gap_after: f32,
    /// Top and allocated line height within the page's body rectangle.
    pub y: f32,
    pub height: f32,
}

impl Line {
    /// Share the renderer's justification spacing with marks and pointer hit testing.
    pub fn letter_spacing(&self, layout: &Layout, metrics: &TextMetrics) -> f32 {
        let layout = layout.sanitized();
        if layout.justify && !self.paragraph_end {
            let gaps = self
                .text
                .chars()
                .filter(|character| metrics.advance(*character, layout.font_size) > 0.0)
                .count()
                .saturating_sub(1);
            if gaps > 0 {
                let indent = if self.indent {
                    layout.font_size * layout.indent as f32
                } else {
                    0.0
                };
                (layout.width - indent - self.width).max(0.0) / gaps as f32
            } else {
                0.0
            }
        } else {
            0.0
        }
    }

    /// Project a persisted Unicode-scalar anchor into the current line geometry.
    pub fn x_at_offset(&self, offset: u32, layout: &Layout, metrics: &TextMetrics) -> Option<f32> {
        Some(
            self.caret_geometry(layout, metrics, &mut CaretBudget::default())
                .ok()?
                .x_at_offset(offset),
        )
    }

    /// Resolve a visual horizontal coordinate to the nearest source-text anchor.
    pub fn offset_at_x(&self, x: f32, layout: &Layout, metrics: &TextMetrics) -> Option<u32> {
        if x.is_nan() {
            return None;
        }
        self.caret_geometry(layout, metrics, &mut CaretBudget::default())
            .ok()?
            .offset_at_x(x)
    }

    /// Resolve the measured glyph cluster under a pointer, including zero-width scalars.
    pub fn range_at_x(&self, x: f32, layout: &Layout, metrics: &TextMetrics) -> Option<(u32, u32)> {
        if x.is_nan() {
            return None;
        }
        self.caret_geometry(layout, metrics, &mut CaretBudget::default())
            .ok()?
            .range_at_x(x)
    }

    pub fn caret_geometry(
        &self,
        layout: &Layout,
        metrics: &TextMetrics,
        budget: &mut CaretBudget,
    ) -> Result<CaretGeometry, CaretError> {
        let reservation = budget.admit(self)?;
        self.caret_geometry_with_reservation(layout, metrics, budget, reservation)
    }

    /// Operation-local page preflight can transfer one sealed reservation to
    /// the selected line. It cannot authorize another count or changed text.
    pub(crate) fn caret_geometry_with_reservation(
        &self,
        layout: &Layout,
        metrics: &TextMetrics,
        budget: &mut CaretBudget,
        reservation: CaretReservation,
    ) -> Result<CaretGeometry, CaretError> {
        if reservation.bytes != self.text.len()
            || reservation.start != self.start
            || reservation.end != self.end
            || self.text.chars().count() + 1 != reservation.positions
        {
            return Err(CaretError::Anchors);
        }
        let layout = layout.sanitized();
        let spacing = self.letter_spacing(&layout, metrics);
        let positions = self.calculate_carets(
            layout,
            spacing,
            reservation,
            |previous, character| {
                (
                    previous
                        .map(|left| metrics.kerning(left, character, layout.font_size))
                        .unwrap_or(0.0),
                    metrics.advance(character, layout.font_size),
                )
            },
            |step| budget.observe(step),
        );
        #[cfg(test)]
        {
            budget.stats.allocations += 1;
        }
        Ok(CaretGeometry {
            positions,
            start: self.start,
            end: self.end,
            spacing,
        })
    }

    #[cfg(test)]
    fn caret_positions_with(
        &self,
        layout: Layout,
        spacing: f32,
        measure: impl FnMut(Option<char>, char) -> (f32, f32),
        observe: impl FnMut(CaretStep),
    ) -> Vec<f32> {
        let reservation = CaretBudget::default()
            .admit(self)
            .expect("Admit the small caret algorithm fixture");
        self.calculate_carets(layout, spacing, reservation, measure, observe)
    }

    fn calculate_carets(
        &self,
        layout: Layout,
        spacing: f32,
        reservation: CaretReservation,
        mut measure: impl FnMut(Option<char>, char) -> (f32, f32),
        mut observe: impl FnMut(CaretStep),
    ) -> Vec<f32> {
        let mut position = if self.indent {
            layout.font_size * layout.indent as f32
        } else {
            0.0
        };
        let mut positions = Vec::with_capacity(reservation.positions);
        positions.push(position);
        let mut previous = None;
        let mut has_advance = false;
        for character in self.text.chars() {
            observe(CaretStep::Character);
            let (kern, advance) = measure(previous, character);
            let before = kern
                + if advance > 0.0 && has_advance {
                    spacing
                } else {
                    0.0
                };
            // A boundary before the next glyph includes its kerning and inter-glyph gap.
            // Zero-width scalars keep the same caret as their adjoining base glyph.
            // A zero adjustment changes no coordinate. In particular, a long
            // combining sequence with no kern/gap must not rescan its growing
            // equal-caret group on every scalar. Nonzero adjustments retain
            // their original epsilon semantics. Every local/remote caller must
            // hold the generic worst-case reservation before entering this loop.
            if before != 0.0 {
                for boundary in positions.iter_mut().rev() {
                    observe(CaretStep::Boundary);
                    if (*boundary - position).abs() > 0.001 {
                        break;
                    }
                    *boundary += before;
                }
            }
            position += before + advance;
            positions.push(position);
            has_advance |= advance > 0.0;
            previous = Some(character);
        }
        positions
    }

    #[cfg(test)]
    pub(crate) fn caret_work_for_test(
        &self,
        layout: &Layout,
        metrics: &TextMetrics,
    ) -> (usize, usize) {
        let mut budget = CaretBudget::default();
        self.caret_geometry(layout, metrics, &mut budget)
            .expect("Admit the caret work fixture");
        (budget.stats.measured, budget.stats.boundaries)
    }
}

enum CaretStep {
    Character,
    Boundary,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InlineImage {
    /// An occurrence identity, distinct from a possibly shared resource digest.
    pub id: String,
    pub anchor: u32,
    /// Intrinsic logical size. Callers convert physical pixels before layout.
    pub width: f32,
    pub height: f32,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ImageAnchorWork {
    pub projections: usize,
    pub images: usize,
    pub scalar_checks: usize,
}

#[cfg(test)]
std::thread_local! {
    static IMAGE_ANCHOR_WORK: std::cell::Cell<ImageAnchorWork> =
        std::cell::Cell::new(ImageAnchorWork::default());
}

#[cfg(test)]
pub(crate) fn reset_image_anchor_work() {
    IMAGE_ANCHOR_WORK.with(|work| work.set(ImageAnchorWork::default()));
}

#[cfg(test)]
pub(crate) fn image_anchor_work() -> ImageAnchorWork {
    IMAGE_ANCHOR_WORK.with(std::cell::Cell::get)
}

/// Project each zero-width run once, without reordering the original records.
/// Sorting only integer ordinals lets the final stable image sort preserve DOM
/// order when different original anchors collapse to the same scalar boundary.
fn normalize_image_anchors(
    characters: &[char],
    images: &mut [(usize, InlineImage)],
    budget: &LayoutBudget,
) -> Result<(), LayoutLimitError> {
    #[cfg(test)]
    IMAGE_ANCHOR_WORK.with(|work| {
        let mut observed = work.get();
        observed.projections += 1;
        observed.images += images.len();
        work.set(observed);
    });
    let mut order = LayoutVec::with_capacity(budget, images.len())?;
    order.extend_reserved(0..images.len())?;
    budget.work(
        images
            .len()
            .checked_mul(images.len().max(1).ilog2() as usize + 1)
            .and_then(|work| work.checked_mul(2 * std::mem::size_of::<usize>()))
            .ok_or(LayoutLimitError {
                kind: LayoutLimitKind::AllocationWork,
                used: 0,
                requested: usize::MAX,
                limit: layout_budget::DEFAULT_LAYOUT_ALLOCATION_WORK,
            })?,
    )?;
    order.sort_unstable_by_key(|index| (images[*index].1.anchor, *index));
    let mut run_end = 0;
    for &index in order.iter() {
        let original = images[index].1.anchor as usize;
        let mut anchor = original.max(run_end);
        if original >= run_end {
            while anchor < characters.len() {
                #[cfg(test)]
                IMAGE_ANCHOR_WORK.with(|work| {
                    let mut observed = work.get();
                    observed.scalar_checks += 1;
                    work.set(observed);
                });
                if !is_zero_width(characters[anchor]) {
                    break;
                }
                anchor += 1;
            }
            run_end = anchor;
        }
        if anchor > 0
            && characters.get(anchor - 1) == Some(&'\r')
            && characters.get(anchor) == Some(&'\n')
        {
            anchor += 1;
        }
        images[index].1.anchor = anchor as u32;
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlacedImage {
    pub id: String,
    pub anchor: u32,
    /// Geometry relative to the page's body rectangle, in logical pixels.
    pub y: f32,
    /// Full slot height, including the fixed caption area after the bitmap.
    pub height: f32,
    /// Fitted bitmap width. The view centers it within the text measure.
    pub width: f32,
}

pub const IMAGE_CAPTION_HEIGHT: f32 = 24.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub start: u32,
    pub end: u32,
    pub lines: Vec<Line>,
    pub images: Vec<PlacedImage>,
}

impl Page {
    /// Clamp a drag to the nearest nonempty text line, including gaps occupied by images.
    pub fn text_range_at_point(
        &self,
        x: f32,
        y: f32,
        layout: &Layout,
        metrics: &TextMetrics,
    ) -> Option<(u32, u32)> {
        self.text_range_at_point_with_budget(x, y, layout, metrics, &mut CaretBudget::default())
    }

    pub fn text_range_at_point_with_budget(
        &self,
        x: f32,
        y: f32,
        layout: &Layout,
        metrics: &TextMetrics,
        budget: &mut CaretBudget,
    ) -> Option<(u32, u32)> {
        if !x.is_finite() || !y.is_finite() || self.lines.len() > MAX_CARET_PAGE_LINES {
            return None;
        }
        let first = self.lines.iter().find(|line| line.start < line.end)?;
        if y < first.y {
            return first
                .caret_geometry(layout, metrics, budget)
                .ok()?
                .range_at_x(f32::NEG_INFINITY);
        }
        let last = self.lines.iter().rev().find(|line| line.start < line.end)?;
        let last_bottom = last.y
            + if last.height > 0.0 {
                last.height
            } else {
                layout.line_advance()
            };
        if y >= last_bottom {
            return last
                .caret_geometry(layout, metrics, budget)
                .ok()?
                .range_at_x(f32::INFINITY);
        }
        let mut closest = None;
        let mut distance = f32::INFINITY;
        for line in self.lines.iter().filter(|line| line.start < line.end) {
            let bottom = line.y
                + if line.height > 0.0 {
                    line.height
                } else {
                    layout.line_advance()
                };
            if y >= line.y && y < bottom {
                return line
                    .caret_geometry(layout, metrics, budget)
                    .ok()?
                    .range_at_x(x);
            }
            let candidate = if y < line.y {
                line.y - y
            } else if y >= bottom {
                y - bottom
            } else {
                0.0
            };
            if candidate < distance {
                closest = Some(line);
                distance = candidate;
            }
        }
        let line = closest?;
        line.caret_geometry(layout, metrics, budget)
            .ok()?
            .range_at_x(x)
    }
}

#[derive(Debug)]
pub struct Paginator {
    pub pages: Vec<Page>,
    pub char_count: u32,
    pub layout: Layout,
    current: usize,
    pages_capacity: usize,
    payload_bytes: usize,
    admission: LayoutLease,
}

impl Paginator {
    pub fn try_new(
        text: &str,
        layout: &Layout,
        metrics: &TextMetrics,
    ) -> Result<Self, LayoutLimitError> {
        Self::try_new_in(text, layout, metrics, &LayoutBudget::default())
    }

    pub(crate) fn try_new_in(
        text: &str,
        layout: &Layout,
        metrics: &TextMetrics,
        budget: &LayoutBudget,
    ) -> Result<Self, LayoutLimitError> {
        Self::try_new_with_first_page_height_in(text, layout, metrics, layout.height, budget)
    }

    /// Reserve chapter-heading space on the first page without shrinking later pages.
    pub fn try_new_with_first_page_height(
        text: &str,
        layout: &Layout,
        metrics: &TextMetrics,
        first_page_height: f32,
    ) -> Result<Self, LayoutLimitError> {
        Self::try_new_with_first_page_height_in(
            text,
            layout,
            metrics,
            first_page_height,
            &LayoutBudget::default(),
        )
    }

    pub(crate) fn try_new_with_first_page_height_in(
        text: &str,
        layout: &Layout,
        metrics: &TextMetrics,
        first_page_height: f32,
        budget: &LayoutBudget,
    ) -> Result<Self, LayoutLimitError> {
        let layout = layout.sanitized();
        let first_page_height =
            finite_or(first_page_height, layout.height).clamp(1.0, layout.height);
        let characters = layout_characters(text, budget)?;
        let char_count = characters.len() as u32;
        let lines = layout_lines(&characters, layout, metrics, budget)?;
        // No later pagination step borrows the scalar stream.
        drop(characters);
        let mut pages = LayoutVec::new(budget);
        let mut page = LayoutPage::new(budget, 0);
        let line_buffer = lines.buffer_bytes();
        let (lines, mut line_owner) = lines.into_parts();
        let mut used_height = 0.0;
        let line_height = layout.font_size * layout.line_height;
        for (mut line, source_end) in lines {
            let capacity = if pages.is_empty() {
                first_page_height
            } else {
                layout.height
            };
            if !page.lines.is_empty() && used_height + line_height > capacity + 0.01 {
                let next_start = page.end;
                push_layout_page(&mut pages, page)?;
                page = LayoutPage::new(budget, next_start);
                used_height = 0.0;
            }
            line.y = used_height;
            line.height = line_height;
            used_height += line_height + line.gap_after;
            page.end = source_end;
            page.lines.push(line)?;
        }
        if !page.lines.is_empty() || pages.is_empty() {
            page.end = char_count;
            push_layout_page(&mut pages, page)?;
        }
        line_owner.release(line_buffer);
        let payload_bytes = line_owner.bytes();
        pages.adopt(line_owner);
        let pages_capacity = pages.buffer_bytes() / std::mem::size_of::<Page>();
        let (pages, admission) = pages.into_parts();
        Ok(Self {
            pages,
            char_count,
            layout,
            current: 0,
            pages_capacity,
            payload_bytes,
            admission,
        })
    }

    /// Insert verified image occurrences into the canonical text's character flow.
    pub fn try_new_with_images(
        text: &str,
        layout: &Layout,
        metrics: &TextMetrics,
        first_page_height: f32,
        images: &[InlineImage],
    ) -> Result<Self, LayoutLimitError> {
        Self::try_new_with_images_in(
            text,
            layout,
            metrics,
            first_page_height,
            images,
            &LayoutBudget::default(),
        )
    }

    pub(crate) fn try_new_with_images_in(
        text: &str,
        layout: &Layout,
        metrics: &TextMetrics,
        first_page_height: f32,
        images: &[InlineImage],
        budget: &LayoutBudget,
    ) -> Result<Self, LayoutLimitError> {
        if images.is_empty() {
            return Self::try_new_with_first_page_height_in(
                text,
                layout,
                metrics,
                first_page_height,
                budget,
            );
        }
        let characters = layout_characters(text, budget)?;
        let char_count = characters.len() as u32;
        let mut admitted_images = LayoutVec::new(budget);
        for (ordinal, image) in images.iter().enumerate() {
            if !image.id.is_empty()
                && image.anchor <= char_count
                && image.width.is_finite()
                && image.height.is_finite()
                && image.width > 0.0
                && image.height > 0.0
            {
                admitted_images.reserve_one()?;
                let (id, owner) = budget.copy(&image.id)?;
                admitted_images.push_owned(
                    (
                        ordinal,
                        InlineImage {
                            id,
                            anchor: image.anchor,
                            width: image.width,
                            height: image.height,
                        },
                    ),
                    owner,
                )?;
            }
        }
        if admitted_images.is_empty() {
            // The fallback builds its own scalar stream. Release this one first.
            drop(characters);
            return Self::try_new_with_first_page_height_in(
                text,
                layout,
                metrics,
                first_page_height,
                budget,
            );
        }
        normalize_image_anchors(&characters, &mut admitted_images, budget)?;
        // An ordinal tie-break preserves DOM order without a stable-sort buffer.
        admitted_images.sort_unstable_by_key(|(ordinal, image)| (image.anchor, *ordinal));
        let layout = layout.sanitized();
        let first_page_height =
            finite_or(first_page_height, layout.height).clamp(1.0, layout.height);
        let mut flow = LayoutVec::new(budget);
        let mut start = 0;
        let image_buffer = admitted_images.buffer_bytes();
        let (images, mut image_owner) = admitted_images.into_parts();
        for (_, image) in images {
            let end = image.anchor as usize;
            append_text_flow(&mut flow, &characters, start, end, layout, metrics, budget)?;
            flow.push(ReaderFlow::Image(image))?;
            start = end;
        }
        append_text_flow(
            &mut flow,
            &characters,
            start,
            characters.len(),
            layout,
            metrics,
            budget,
        )?;
        image_owner.release(image_buffer);
        flow.adopt(image_owner);
        drop(characters);
        let (pages, payload_bytes) =
            paginate_flow(flow, layout, first_page_height, char_count, budget)?;
        let pages_capacity = pages.buffer_bytes() / std::mem::size_of::<Page>();
        let (pages, admission) = pages.into_parts();
        Ok(Self {
            pages,
            char_count,
            layout,
            current: 0,
            pages_capacity,
            payload_bytes,
            admission,
        })
    }

    pub fn requested_metadata(&self) -> usize {
        self.admission.bytes()
    }

    pub(crate) fn into_pages(self) -> (Vec<Page>, LayoutLease, usize) {
        let buffer = self.admission.bytes() - self.payload_bytes;
        (self.pages, self.admission, buffer)
    }

    pub(crate) fn try_push_empty_page(
        &mut self,
        start: u32,
        end: u32,
    ) -> Result<(), LayoutLimitError> {
        let required = self.pages.len().checked_add(1).ok_or(LayoutLimitError {
            kind: LayoutLimitKind::Metadata,
            used: self.pages.len(),
            requested: 1,
            limit: layout_budget::DEFAULT_LAYOUT_METADATA_BYTES,
        })?;
        self.admission
            .reserve_vec(&mut self.pages, &mut self.pages_capacity, required)?;
        self.pages.push(Page {
            start,
            end,
            ..Page::default()
        });
        Ok(())
    }

    pub fn page_for_offset(&self, offset: u32) -> usize {
        let offset = offset.min(self.char_count);
        // A text anchor denotes the position before every image at that boundary.
        // Character offsets alone cannot distinguish consecutive image-only pages.
        if let Some(index) = self
            .pages
            .iter()
            .position(|page| page.images.iter().any(|image| image.anchor == offset))
        {
            return index;
        }
        self.pages
            .partition_point(|page| page.end <= offset)
            .min(self.pages.len().saturating_sub(1))
    }

    pub fn page_for_image(&self, id: &str) -> Option<usize> {
        self.pages
            .iter()
            .position(|page| page.images.iter().any(|image| image.id == id))
    }

    pub fn seek_image(&mut self, id: &str) -> Option<&Page> {
        self.current = self.page_for_image(id)?;
        Some(self.current_page())
    }

    pub fn seek(&mut self, offset: u32) -> &Page {
        self.current = self.page_for_offset(offset);
        self.current_page()
    }

    pub fn current_page(&self) -> &Page {
        &self.pages[self.current]
    }

    pub fn current_index(&self) -> usize {
        self.current
    }

    pub fn set_page(&mut self, index: usize) -> &Page {
        self.current = index.min(self.pages.len().saturating_sub(1));
        self.current_page()
    }

    pub fn next_page(&mut self) -> Option<&Page> {
        if self.current + 1 >= self.pages.len() {
            return None;
        }
        self.current += 1;
        Some(self.current_page())
    }

    pub fn previous_page(&mut self) -> Option<&Page> {
        if self.current == 0 {
            return None;
        }
        self.current -= 1;
        Some(self.current_page())
    }

    /// Reflow around a stable content anchor rather than an old page number.
    pub fn try_reflow(
        &mut self,
        text: &str,
        layout: &Layout,
        metrics: &TextMetrics,
        anchor: u32,
    ) -> Result<(), LayoutLimitError> {
        self.try_reflow_with_first_page_height(text, layout, metrics, layout.height, anchor)
    }

    pub fn try_reflow_with_first_page_height(
        &mut self,
        text: &str,
        layout: &Layout,
        metrics: &TextMetrics,
        first_page_height: f32,
        anchor: u32,
    ) -> Result<(), LayoutLimitError> {
        let budget = LayoutBudget::default();
        let _old = budget.hold(self.requested_metadata())?;
        let mut prepared = Self::try_new_with_first_page_height_in(
            text,
            layout,
            metrics,
            first_page_height,
            &budget,
        )?;
        prepared.seek(anchor);
        *self = prepared;
        Ok(())
    }

    pub fn try_reflow_with_images(
        &mut self,
        text: &str,
        layout: &Layout,
        metrics: &TextMetrics,
        first_page_height: f32,
        images: &[InlineImage],
        anchor: u32,
    ) -> Result<(), LayoutLimitError> {
        let budget = LayoutBudget::default();
        let _old = budget.hold(self.requested_metadata())?;
        let visible_image = self
            .current_page()
            .lines
            .is_empty()
            .then(|| {
                self.current_page()
                    .images
                    .first()
                    .map(|image| image.id.as_str())
            })
            .flatten();
        let mut prepared = Self::try_new_with_images_in(
            text,
            layout,
            metrics,
            first_page_height,
            images,
            &budget,
        )?;
        if let Some(id) = visible_image {
            if prepared.seek_image(id).is_some() {
                *self = prepared;
                return Ok(());
            }
        }
        prepared.seek(anchor);
        *self = prepared;
        Ok(())
    }

    #[cfg(test)]
    pub fn new(text: &str, layout: &Layout, metrics: &TextMetrics) -> Self {
        Self::try_new(text, layout, metrics).expect("Admit the ordinary pagination fixture")
    }

    #[cfg(test)]
    pub fn new_with_first_page_height(
        text: &str,
        layout: &Layout,
        metrics: &TextMetrics,
        first_page_height: f32,
    ) -> Self {
        Self::try_new_with_first_page_height(text, layout, metrics, first_page_height)
            .expect("Admit the ordinary first-page fixture")
    }

    #[cfg(test)]
    pub fn new_with_images(
        text: &str,
        layout: &Layout,
        metrics: &TextMetrics,
        first_page_height: f32,
        images: &[InlineImage],
    ) -> Self {
        Self::try_new_with_images(text, layout, metrics, first_page_height, images)
            .expect("Admit the ordinary image pagination fixture")
    }

    #[cfg(test)]
    pub fn reflow(&mut self, text: &str, layout: &Layout, metrics: &TextMetrics, anchor: u32) {
        self.try_reflow(text, layout, metrics, anchor)
            .expect("Admit the ordinary reflow fixture");
    }

    #[cfg(test)]
    pub fn reflow_with_first_page_height(
        &mut self,
        text: &str,
        layout: &Layout,
        metrics: &TextMetrics,
        first_page_height: f32,
        anchor: u32,
    ) {
        self.try_reflow_with_first_page_height(text, layout, metrics, first_page_height, anchor)
            .expect("Admit the ordinary first-page reflow fixture");
    }

    #[cfg(test)]
    pub fn reflow_with_images(
        &mut self,
        text: &str,
        layout: &Layout,
        metrics: &TextMetrics,
        first_page_height: f32,
        images: &[InlineImage],
        anchor: u32,
    ) {
        self.try_reflow_with_images(text, layout, metrics, first_page_height, images, anchor)
            .expect("Admit the ordinary image reflow fixture");
    }
}

#[cfg(test)]
impl Clone for Paginator {
    fn clone(&self) -> Self {
        let budget = LayoutBudget::default();
        let _old = budget
            .hold(self.requested_metadata())
            .expect("Hold the pagination fixture");
        let admission = budget
            .copy_storage(self.requested_metadata())
            .expect("Admit the pagination fixture copy");
        Self {
            pages: self.pages.clone(),
            char_count: self.char_count,
            layout: self.layout,
            current: self.current,
            pages_capacity: self.pages_capacity,
            payload_bytes: self.payload_bytes,
            admission,
        }
    }
}

fn layout_characters(
    text: &str,
    budget: &LayoutBudget,
) -> Result<LayoutVec<char>, LayoutLimitError> {
    budget.work(text.len())?;
    let count = text.chars().count();
    u32::try_from(count).map_err(|_| LayoutLimitError {
        kind: LayoutLimitKind::Anchors,
        used: 0,
        requested: count,
        limit: u32::MAX as usize,
    })?;
    let mut characters = LayoutVec::with_capacity(budget, count)?;
    characters.extend_reserved(text.chars())?;
    Ok(characters)
}

fn layout_lines(
    characters: &[char],
    layout: Layout,
    metrics: &TextMetrics,
    budget: &LayoutBudget,
) -> Result<LayoutVec<(Line, u32)>, LayoutLimitError> {
    layout_lines_with_indent(characters, layout, metrics, true, budget)
}

fn layout_lines_with_indent(
    characters: &[char],
    layout: Layout,
    metrics: &TextMetrics,
    first_paragraph_indent: bool,
    budget: &LayoutBudget,
) -> Result<LayoutVec<(Line, u32)>, LayoutLimitError> {
    let mut lines = LayoutVec::new(budget);
    let mut paragraph_start = 0;
    while paragraph_start < characters.len() {
        let mut paragraph_end = paragraph_start;
        while paragraph_end < characters.len() {
            budget.scan_step()?;
            if matches!(characters[paragraph_end], '\n' | '\r') {
                break;
            }
            paragraph_end += 1;
        }
        let delimiter_end = if paragraph_end < characters.len() {
            if characters[paragraph_end] == '\r' && characters.get(paragraph_end + 1) == Some(&'\n')
            {
                paragraph_end + 2
            } else {
                paragraph_end + 1
            }
        } else {
            paragraph_end
        };
        if paragraph_end == paragraph_start {
            lines.push((
                Line {
                    start: paragraph_start as u32,
                    end: paragraph_start as u32,
                    paragraph_end: true,
                    gap_after: layout.font_size * layout.paragraph_gap,
                    ..Line::default()
                },
                delimiter_end as u32,
            ))?;
        } else {
            let mut start = paragraph_start;
            let mut first = paragraph_start > 0 || first_paragraph_indent;
            while start < paragraph_end {
                let indent_width = if first {
                    layout.font_size * layout.indent as f32
                } else {
                    0.0
                };
                let available = (layout.width - indent_width).max(layout.font_size);
                let mut end = start;
                let mut width = 0.0;
                let mut previous = None;
                while end < paragraph_end {
                    budget.scan_step()?;
                    let character = characters[end];
                    let kern = previous
                        .map(|left| metrics.kerning(left, character, layout.font_size))
                        .unwrap_or(0.0);
                    let advance = metrics.advance(character, layout.font_size) + kern;
                    if end > start && width + advance > available + 0.01 {
                        break;
                    }
                    width += advance;
                    previous = Some(character);
                    end += 1;
                }
                if end < paragraph_end {
                    end = legal_break(characters, start, end, paragraph_end, budget)?;
                }
                while end < paragraph_end {
                    budget.scan_step()?;
                    if !is_zero_width(characters[end]) {
                        break;
                    }
                    end += 1;
                }
                if end <= start {
                    end = start + 1;
                }
                // Reserve the output slot before its text or tuple is created.
                lines.reserve_one()?;
                let slice = &characters[start..end];
                budget.work(slice.len().checked_mul(4).ok_or(LayoutLimitError {
                    kind: LayoutLimitKind::AllocationWork,
                    used: 0,
                    requested: usize::MAX,
                    limit: layout_budget::DEFAULT_LAYOUT_ALLOCATION_WORK,
                })?)?;
                let bytes = slice
                    .iter()
                    .try_fold(0usize, |used, character| {
                        used.checked_add(character.len_utf8())
                    })
                    .ok_or(LayoutLimitError {
                        kind: LayoutLimitKind::Metadata,
                        used: 0,
                        requested: usize::MAX,
                        limit: layout_budget::DEFAULT_LAYOUT_METADATA_BYTES,
                    })?;
                let mut text = LayoutString::with_capacity(budget, bytes)?;
                for &character in slice {
                    text.push(character)?;
                }
                let (text, text_owner) = text.into_parts();
                let is_last = end == paragraph_end;
                let line = Line {
                    width: metrics.measure_with_budget(&text, layout.font_size, budget)?,
                    text,
                    start: start as u32,
                    end: end as u32,
                    indent: first && layout.indent > 0,
                    paragraph_end: is_last,
                    gap_after: if is_last {
                        layout.font_size * layout.paragraph_gap
                    } else {
                        0.0
                    },
                    y: 0.0,
                    height: layout.font_size * layout.line_height,
                };
                lines.push_owned(
                    (line, if is_last { delimiter_end } else { end } as u32),
                    text_owner,
                )?;
                start = end;
                first = false;
            }
        }
        paragraph_start = delimiter_end;
    }
    Ok(lines)
}

enum ReaderFlow {
    Advance(u32),
    Line(Line, u32),
    Image(InlineImage),
}

fn append_text_flow(
    flow: &mut LayoutVec<ReaderFlow>,
    characters: &[char],
    start: usize,
    end: usize,
    layout: Layout,
    metrics: &TextMetrics,
    budget: &LayoutBudget,
) -> Result<(), LayoutLimitError> {
    if start >= end {
        return Ok(());
    }
    let mut visible_start = start;
    // A delimiter immediately after an inline image still terminates the
    // preceding paragraph; it must not become an extra empty visual line.
    if start > 0
        && !matches!(characters[start - 1], '\n' | '\r')
        && matches!(characters[start], '\n' | '\r')
    {
        visible_start += 1;
        if characters[start] == '\r' && visible_start < end && characters[visible_start] == '\n' {
            visible_start += 1;
        }
        flow.push(ReaderFlow::Advance(visible_start as u32))?;
    }
    if visible_start >= end {
        return Ok(());
    }
    let first_indent = visible_start == 0 || matches!(characters[visible_start - 1], '\n' | '\r');
    let mut lines = layout_lines_with_indent(
        &characters[visible_start..end],
        layout,
        metrics,
        first_indent,
        budget,
    )?;
    if end < characters.len()
        && !matches!(characters[end], '\n' | '\r')
        && !matches!(characters[end - 1], '\n' | '\r')
    {
        if let Some((line, _)) = lines.last_mut() {
            // An image-forced break is a visual paragraph end for justification,
            // but does not insert paragraph spacing into the original paragraph.
            line.paragraph_end = true;
            line.gap_after = 0.0;
        }
    }
    let line_buffer = lines.buffer_bytes();
    let (lines, mut line_owner) = lines.into_parts();
    for (mut line, source_end) in lines {
        line.start += visible_start as u32;
        line.end += visible_start as u32;
        flow.push(ReaderFlow::Line(line, source_end + visible_start as u32))?;
    }
    line_owner.release(line_buffer);
    flow.adopt(line_owner);
    Ok(())
}

struct LayoutPage {
    start: u32,
    end: u32,
    lines: LayoutVec<Line>,
    images: LayoutVec<PlacedImage>,
}

impl LayoutPage {
    fn new(budget: &LayoutBudget, start: u32) -> Self {
        Self {
            start,
            end: start,
            lines: LayoutVec::new(budget),
            images: LayoutVec::new(budget),
        }
    }
}

fn push_layout_page(pages: &mut LayoutVec<Page>, page: LayoutPage) -> Result<(), LayoutLimitError> {
    pages.reserve_one()?;
    let (lines, mut owner) = page.lines.into_parts();
    let (images, image_owner) = page.images.into_parts();
    owner.merge(image_owner);
    pages.push_owned(
        Page {
            start: page.start,
            end: page.end,
            lines,
            images,
        },
        owner,
    )
}

fn has_page_content(page: &LayoutPage) -> bool {
    !page.lines.is_empty() || !page.images.is_empty()
}

fn next_image_geometry(image: &InlineImage, width: f32, capacity: f32) -> (f32, f32) {
    // A normal device body is taller than the caption. The reduced caption
    // fallback only makes invalid/tiny test geometry bounded and non-overlapping.
    let caption = IMAGE_CAPTION_HEIGHT.min(capacity);
    let image_capacity = (capacity - caption).max(f32::EPSILON);
    let scale = (width / image.width)
        .min(image_capacity / image.height)
        .min(1.0);
    (
        image.width * scale,
        (image.height * scale + caption).min(capacity),
    )
}

fn paginate_flow(
    flow: LayoutVec<ReaderFlow>,
    layout: Layout,
    first_page_height: f32,
    char_count: u32,
    budget: &LayoutBudget,
) -> Result<(LayoutVec<Page>, usize), LayoutLimitError> {
    let mut pages = LayoutVec::new(budget);
    let mut page = LayoutPage::new(budget, 0);
    let mut used_height = 0.0;
    let line_height = layout.font_size * layout.line_height;
    let flow_buffer = flow.buffer_bytes();
    let (flow, mut flow_owner) = flow.into_parts();
    for item in flow {
        let capacity = if pages.is_empty() {
            first_page_height
        } else {
            layout.height
        };
        match item {
            ReaderFlow::Advance(offset) => {
                page.end = page.end.max(offset);
            }
            ReaderFlow::Line(mut line, source_end) => {
                if has_page_content(&page) && used_height + line_height > capacity + 0.01 {
                    let next_start = page.end;
                    push_layout_page(&mut pages, page)?;
                    page = LayoutPage::new(budget, next_start);
                    used_height = 0.0;
                }
                line.y = used_height;
                line.height = line_height;
                used_height += line_height + line.gap_after;
                page.end = page.end.max(source_end);
                page.lines.push(line)?;
            }
            ReaderFlow::Image(image) => {
                let (mut width, mut height) = next_image_geometry(&image, layout.width, capacity);
                if has_page_content(&page) && used_height + height > capacity + 0.01 {
                    let next_start = page.end;
                    push_layout_page(&mut pages, page)?;
                    page = LayoutPage::new(budget, next_start);
                    used_height = 0.0;
                    (width, height) = next_image_geometry(&image, layout.width, layout.height);
                }
                page.end = page.end.max(image.anchor);
                page.images.push(PlacedImage {
                    id: image.id,
                    anchor: image.anchor,
                    y: used_height,
                    height,
                    width,
                })?;
                used_height += height;
            }
        }
    }
    if has_page_content(&page) || page.start != page.end || pages.is_empty() {
        page.end = char_count;
        push_layout_page(&mut pages, page)?;
    }
    flow_owner.release(flow_buffer);
    let payload_bytes = flow_owner.bytes();
    pages.adopt(flow_owner);
    Ok((pages, payload_bytes))
}

fn legal_break(
    characters: &[char],
    start: usize,
    tentative: usize,
    paragraph_end: usize,
    budget: &LayoutBudget,
) -> Result<usize, LayoutLimitError> {
    let mut end = tentative;
    // Keep a Latin word together when an earlier whitespace break is available.
    if end < paragraph_end {
        budget.scan_step()?;
        if characters[end - 1].is_ascii_alphanumeric() {
            budget.scan_step()?;
            if characters[end].is_ascii_alphanumeric() {
                for index in (start + 1..end).rev() {
                    budget.scan_step()?;
                    if characters[index - 1].is_whitespace() {
                        end = index;
                        break;
                    }
                }
            }
        }
    }
    // Moving the preceding character prevents a prohibited character at line start.
    while end > start + 1 {
        budget.scan_step()?;
        let closing = is_closing(characters[end]);
        if !closing {
            budget.scan_step()?;
            if !is_opening(characters[end - 1]) {
                break;
            }
        }
        end -= 1;
    }
    if end == start + 1 && end < paragraph_end {
        budget.scan_step()?;
        if is_closing(characters[end]) {
            // A pathologically narrow layout must still make progress without stranding punctuation.
            while end < paragraph_end {
                budget.scan_step()?;
                if !is_closing(characters[end]) {
                    break;
                }
                end += 1;
            }
        }
    }
    Ok(end)
}

fn is_closing(character: char) -> bool {
    matches!(
        character,
        '\u{3001}'
            | '\u{3002}'
            | '\u{ff0c}'
            | '\u{ff0e}'
            | '\u{ff01}'
            | '\u{ff1f}'
            | '\u{ff1b}'
            | '\u{ff1a}'
            | '\u{ff09}'
            | '\u{3011}'
            | '\u{3009}'
            | '\u{300b}'
            | '\u{300d}'
            | '\u{300f}'
            | '\u{3015}'
            | '\u{3017}'
            | '\u{3019}'
            | '\u{301b}'
            | '\u{2019}'
            | '\u{201d}'
            | '\u{2026}'
            | '\u{2030}'
            | '\u{ff05}'
            | ')'
            | ']'
            | '}'
            | ','
            | '.'
            | '!'
            | '?'
            | ';'
            | ':'
            | '%'
    )
}

fn is_opening(character: char) -> bool {
    matches!(
        character,
        '\u{ff08}'
            | '\u{3010}'
            | '\u{3008}'
            | '\u{300a}'
            | '\u{300c}'
            | '\u{300e}'
            | '\u{3014}'
            | '\u{3016}'
            | '\u{3018}'
            | '\u{301a}'
            | '\u{2018}'
            | '\u{201c}'
            | '('
            | '['
            | '{'
    )
}

pub fn text_range(text: &str, start: u32, end: u32) -> String {
    text.chars()
        .skip(start as usize)
        .take(end.saturating_sub(start) as usize)
        .collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TapMode {
    #[default]
    Default,
    Mirrored,
    SwipeOnly,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReaderGesture {
    None,
    Previous,
    Next,
    Menu,
    LongPress { x: f32, y: f32 },
}

#[derive(Clone, Copy, Debug)]
struct TouchStart {
    x: f32,
    y: f32,
    time_ms: u64,
}

#[derive(Clone, Debug)]
pub struct GestureRecognizer {
    pub tap_mode: TapMode,
    pub swipe_distance: f32,
    pub long_press_ms: u64,
    start: Option<TouchStart>,
}

impl Default for GestureRecognizer {
    fn default() -> Self {
        Self {
            tap_mode: TapMode::Default,
            swipe_distance: 64.0,
            long_press_ms: 500,
            start: None,
        }
    }
}

impl GestureRecognizer {
    pub fn press(&mut self, x: f32, y: f32, time_ms: u64) {
        self.start = Some(TouchStart { x, y, time_ms });
    }

    pub fn cancel(&mut self) {
        self.start = None;
    }

    pub fn release(&mut self, x: f32, y: f32, time_ms: u64, width: f32) -> ReaderGesture {
        let Some(start) = self.start.take() else {
            return ReaderGesture::None;
        };
        let dx = x - start.x;
        let dy = y - start.y;
        if dx.abs() >= self.swipe_distance && dx.abs() > dy.abs() * 1.5 {
            return if dx < 0.0 {
                ReaderGesture::Next
            } else {
                ReaderGesture::Previous
            };
        }
        if dx.abs() > 16.0 || dy.abs() > 16.0 {
            return ReaderGesture::None;
        }
        if time_ms.saturating_sub(start.time_ms) >= self.long_press_ms {
            return ReaderGesture::LongPress { x, y };
        }
        if self.tap_mode == TapMode::SwipeOnly {
            return ReaderGesture::Menu;
        }
        let ratio = (x / width.max(1.0)).clamp(0.0, 1.0);
        let gesture = if ratio < 0.3 {
            ReaderGesture::Previous
        } else if ratio >= 0.7 {
            ReaderGesture::Next
        } else {
            ReaderGesture::Menu
        };
        if self.tap_mode == TapMode::Mirrored {
            match gesture {
                ReaderGesture::Previous => ReaderGesture::Next,
                ReaderGesture::Next => ReaderGesture::Previous,
                other => other,
            }
        } else {
            gesture
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compact_layout() -> Layout {
        Layout {
            width: 100.0,
            height: 40.0,
            font_size: 10.0,
            line_height: 2.0,
            indent: 0,
            ..Layout::default()
        }
    }

    #[test]
    fn pagination_covers_unicode_without_losing_source_characters() {
        let text = "\u{6843}\u{82b1}\u{6e90}\u{8bb0}\u{3002}abc\u{1f600}\u{540e}\u{6587}\r\n\u{4e8c}\u{6bb5}\u{ff0c}\u{7ee7}\u{7eed}\u{3002}\n\nend";
        let value = Paginator::new(text, &compact_layout(), &TextMetrics::default());
        assert_eq!(value.pages.first().unwrap().start, 0);
        assert_eq!(
            value.pages.last().unwrap().end as usize,
            text.chars().count()
        );
        for pair in value.pages.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
        for line in value.pages.iter().flat_map(|page| &page.lines) {
            assert_eq!(line.text, text_range(text, line.start, line.end));
        }
    }

    #[test]
    fn reflow_preserves_a_character_anchor_instead_of_a_page_number() {
        let text = "A reliable reading position survives every font change. ".repeat(80);
        let mut value = Paginator::new(&text, &compact_layout(), &TextMetrics::default());
        let anchor = value.pages[4].start + 1;
        let larger = Layout {
            font_size: 18.0,
            ..compact_layout()
        };
        value.reflow(&text, &larger, &TextMetrics::default(), anchor);
        let page = value.current_page();
        assert!(page.start <= anchor && anchor < page.end);
    }

    #[test]
    fn chapter_heading_only_reduces_the_first_pages_capacity() {
        let text = "a\n".repeat(20);
        let layout = Layout {
            height: 80.0,
            ..compact_layout()
        };
        let value =
            Paginator::new_with_first_page_height(&text, &layout, &TextMetrics::default(), 40.0);
        assert_eq!(value.pages[0].lines.len(), 2);
        assert_eq!(value.pages[1].lines.len(), 4);
        for pair in value.pages.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
        let anchor = value.pages[2].start + 1;
        let mut reflowed = value;
        let larger = Layout {
            font_size: 12.0,
            ..layout
        };
        reflowed.reflow_with_first_page_height(
            &text,
            &larger,
            &TextMetrics::default(),
            40.0,
            anchor,
        );
        assert!(reflowed.current_page().start <= anchor && anchor < reflowed.current_page().end);
    }

    #[test]
    fn line_coordinates_resolve_to_original_unicode_scalar_ranges() {
        let layout = Layout {
            indent: 2,
            justify: false,
            ..compact_layout()
        };
        let metrics = TextMetrics::default();
        let text = "\u{7532}\u{4e59}\u{4e19}\u{4e01}\u{620a}";
        let value = Paginator::new(text, &layout, &metrics);
        let line = &value.pages[0].lines[0];
        assert_eq!(line.offset_at_x(0.0, &layout, &metrics), Some(0));
        assert_eq!(line.offset_at_x(31.0, &layout, &metrics), Some(1));
        assert_eq!(
            line.offset_at_x(10_000.0, &layout, &metrics),
            Some(line.end)
        );
        assert_eq!(text_range(text, 1, 3), "\u{4e59}\u{4e19}");
    }

    #[test]
    fn justified_carets_share_visible_glyph_spacing_and_skip_combining_marks() {
        let metrics = TextMetrics::default();
        let layout = Layout {
            width: 200.0,
            font_size: 20.0,
            indent: 2,
            justify: true,
            ..compact_layout()
        };
        let line = Line {
            text: "Ae\u{301}\u{4e2d}Z".into(),
            start: 7,
            end: 12,
            indent: true,
            width: metrics.measure("Ae\u{301}\u{4e2d}Z", layout.font_size),
            ..Line::default()
        };
        let spacing = (layout.width - 40.0 - line.width) / 3.0;
        assert!((line.letter_spacing(&layout, &metrics) - spacing).abs() < 0.001);
        assert!((line.x_at_offset(7, &layout, &metrics).unwrap() - 40.0).abs() < 0.001);
        assert!((line.x_at_offset(12, &layout, &metrics).unwrap() - layout.width).abs() < 0.001);
        assert_eq!(
            line.x_at_offset(9, &layout, &metrics),
            line.x_at_offset(10, &layout, &metrics)
        );
        for offset in [7, 8, 10, 11, 12] {
            let x = line.x_at_offset(offset, &layout, &metrics).unwrap();
            assert_eq!(line.offset_at_x(x, &layout, &metrics), Some(offset));
        }
    }

    #[test]
    fn zero_adjustment_combining_carets_do_not_rescan_the_growing_cluster() {
        let marks = 4096;
        let text = format!("A{}B", "\u{301}".repeat(marks));
        let layout = Layout {
            font_size: 20.0,
            indent: 0,
            justify: false,
            ..compact_layout()
        };
        let metrics = TextMetrics::default();
        let line = Line {
            text,
            start: 7,
            end: 7 + marks as u32 + 2,
            paragraph_end: true,
            ..Default::default()
        };
        assert_eq!(line.caret_work_for_test(&layout, &metrics), (marks + 2, 0));
        let base_end = line.x_at_offset(8, &layout, &metrics).unwrap();
        let cluster_end = 7 + marks as u32 + 1;
        assert_eq!(
            line.x_at_offset(cluster_end, &layout, &metrics),
            Some(base_end)
        );
        assert_eq!(
            line.range_at_x(base_end / 2.0, &layout, &metrics),
            Some((7, cluster_end))
        );
        let final_end = line.x_at_offset(line.end, &layout, &metrics).unwrap();
        assert_eq!(
            line.range_at_x((base_end + final_end) / 2.0, &layout, &metrics),
            Some((cluster_end, line.end))
        );
    }

    #[test]
    fn nonzero_cluster_kerning_and_spacing_preserve_boundary_coordinates() {
        let line = Line {
            text: format!("A{}B", "\u{301}".repeat(16)),
            end: 18,
            ..Default::default()
        };
        let mut visits = 0;
        let positions = line.caret_positions_with(
            Layout::default().sanitized(),
            0.75,
            |previous, character| match character {
                'A' => (0.0, 10.0),
                '\u{301}' => (if previous == Some('A') { 0.125 } else { 0.25 }, 0.0),
                'B' => (-0.5, 9.0),
                _ => unreachable!(),
            },
            |step| {
                if matches!(step, CaretStep::Boundary) {
                    visits += 1;
                }
            },
        );
        let mut expected = vec![0.0];
        expected.extend(std::iter::repeat_n(14.125, 17));
        expected.push(23.125);
        assert_eq!(positions, expected);
        assert_eq!(visits, 170);
        let scalars = line.text.chars().count();
        assert!(visits <= scalars * (scalars + 1) / 2);
        // Nonzero adjustments can still be quadratic. The remote page gate
        // reserves that real upper bound rather than claiming linear work.
        assert!(visits > scalars * 4);
    }

    #[test]
    fn caret_epsilon_group_keeps_its_original_inclusive_boundary_rule() {
        let line = Line {
            text: "..XY".into(),
            end: 4,
            ..Default::default()
        };
        let positions = line.caret_positions_with(
            Layout::default().sanitized(),
            0.0,
            |_, character| match character {
                '.' => (0.0, 0.0005),
                'X' => (0.00025, 1.0),
                'Y' => (-0.001, 1.0),
                _ => unreachable!(),
            },
            |_| {},
        );
        let expected = [0.00025, 0.00075, 0.00125, 1.00025, 2.00025];
        for (actual, expected) in positions.iter().zip(expected) {
            assert!((actual - expected).abs() < 0.000001);
        }
    }

    #[test]
    fn pointer_ranges_follow_measured_clusters_in_justified_unicode_text() {
        let metrics = TextMetrics::default();
        let layout = Layout {
            width: 240.0,
            height: 160.0,
            font_size: 20.0,
            indent: 2,
            justify: true,
            ..compact_layout()
        };
        let paginator = Paginator::new(
            "Ae\u{301}\u{4e2d}\u{fe0f}Z followed by enough text to wrap onto more measured lines.",
            &layout,
            &metrics,
        );
        let line = &paginator.current_page().lines[0];
        assert!(!line.paragraph_end);
        let x = |start, end| {
            (line.x_at_offset(start, &layout, &metrics).unwrap()
                + line.x_at_offset(end, &layout, &metrics).unwrap())
                / 2.0
        };
        assert_eq!(line.range_at_x(x(1, 3), &layout, &metrics), Some((1, 3)));
        assert_eq!(line.range_at_x(x(3, 5), &layout, &metrics), Some((3, 5)));
        assert_eq!(line.range_at_x(-1000.0, &layout, &metrics), Some((0, 1)));
        assert_eq!(
            line.range_at_x(10_000.0, &layout, &metrics),
            Some((line.end - 1, line.end))
        );
    }

    #[test]
    fn active_pointer_ranges_clamp_through_real_image_slots_and_page_edges() {
        let metrics = TextMetrics::default();
        let layout = Layout {
            width: 200.0,
            height: 300.0,
            font_size: 20.0,
            indent: 0,
            ..compact_layout()
        };
        let paginator = Paginator::new_with_images(
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ",
            &layout,
            &metrics,
            300.0,
            &[image("drag-gap", 5, 100.0, 60.0)],
        );
        let page = paginator.current_page();
        let picture = &page.images[0];
        let before = page.lines.iter().find(|line| line.end == 5).unwrap();
        let after = page.lines.iter().find(|line| line.start == 5).unwrap();
        assert_eq!(
            page.text_range_at_point(1.0, picture.y + 1.0, &layout, &metrics),
            Some((before.start, before.start + 1))
        );
        assert_eq!(
            page.text_range_at_point(1.0, picture.y + picture.height - 1.0, &layout, &metrics),
            Some((after.start, after.start + 1))
        );
        assert_eq!(
            page.text_range_at_point(100.0, -1000.0, &layout, &metrics),
            Some((0, 1))
        );
        let last = page.lines.last().unwrap();
        assert_eq!(
            page.text_range_at_point(100.0, 10_000.0, &layout, &metrics),
            Some((last.end - 1, last.end))
        );
        assert_eq!(
            page.text_range_at_point(-1000.0, after.y, &layout, &metrics),
            Some((after.start, after.start + 1))
        );
        assert_eq!(
            page.text_range_at_point(f32::NAN, 1.0, &layout, &metrics),
            None
        );
    }

    fn image(id: &str, anchor: u32, width: f32, height: f32) -> InlineImage {
        InlineImage {
            id: id.into(),
            anchor,
            width,
            height,
        }
    }

    fn assert_image_pages_bounded(pagination: &Paginator, first_height: f32) {
        for (index, page) in pagination.pages.iter().enumerate() {
            let capacity = if index == 0 {
                first_height
            } else {
                pagination.layout.height
            };
            let mut intervals: Vec<_> = page
                .lines
                .iter()
                .map(|line| (line.y, line.y + line.height))
                .chain(
                    page.images
                        .iter()
                        .map(|image| (image.y, image.y + image.height)),
                )
                .collect();
            intervals.sort_by(|left, right| left.0.total_cmp(&right.0));
            for (top, bottom) in &intervals {
                assert!(
                    *top >= -0.01 && *bottom <= capacity + 0.01,
                    "{index}: {top}..{bottom}/{capacity}"
                );
            }
            for pair in intervals.windows(2) {
                assert!(
                    pair[0].1 <= pair[1].0 + 0.01,
                    "overlapping content: {pair:?}"
                );
            }
            for picture in &page.images {
                assert!(picture.width <= pagination.layout.width + 0.01);
                assert!(picture.width > 0.0 && picture.height > 0.0);
            }
        }
        for pair in pagination.pages.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
    }

    #[test]
    fn no_image_constructor_and_empty_image_flow_have_identical_geometry() {
        let text = "First paragraph with enough words to wrap across lines.\nSecond paragraph.";
        let layout = Layout {
            height: 100.0,
            paragraph_gap: 0.9,
            ..compact_layout()
        };
        let normal =
            Paginator::new_with_first_page_height(text, &layout, &TextMetrics::default(), 60.0);
        let images = Paginator::new_with_images(text, &layout, &TextMetrics::default(), 60.0, &[]);
        assert_eq!(normal.pages, images.pages);
        for page in &normal.pages {
            let mut expected_y = 0.0;
            for line in &page.lines {
                assert_eq!(line.y, expected_y);
                assert_eq!(line.height, layout.line_advance());
                expected_y += line.height + line.gap_after;
            }
        }
    }

    #[test]
    fn mid_paragraph_image_splits_the_line_without_stretching_or_reindenting() {
        let text = "abcdefghijklmnop";
        let layout = Layout {
            width: 200.0,
            height: 200.0,
            indent: 2,
            paragraph_gap: 0.9,
            ..compact_layout()
        };
        let pagination = Paginator::new_with_images(
            text,
            &layout,
            &TextMetrics::default(),
            160.0,
            &[image("inline", 3, 80.0, 40.0)],
        );
        let page = &pagination.pages[0];
        assert_eq!(page.lines[0].text, "abc");
        assert!(page.lines[0].paragraph_end);
        assert_eq!(page.lines[0].gap_after, 0.0);
        assert!(page.lines[0].indent);
        assert_eq!(page.lines[1].text, "defghijklmnop");
        assert!(!page.lines[1].indent);
        assert!(page.lines[0].y + page.lines[0].height <= page.images[0].y);
        assert!(page.images[0].y + page.images[0].height <= page.lines[1].y);
        assert_image_pages_bounded(&pagination, 160.0);
    }

    #[test]
    fn leading_trailing_and_multiple_images_retain_document_order() {
        let text = "abcdef";
        let layout = Layout {
            height: 300.0,
            ..compact_layout()
        };
        let pagination = Paginator::new_with_images(
            text,
            &layout,
            &TextMetrics::default(),
            300.0,
            &[
                image("lead", 0, 20.0, 10.0),
                image("middle-a", 3, 20.0, 10.0),
                image("middle-b", 3, 20.0, 10.0),
                image("tail", 6, 20.0, 10.0),
            ],
        );
        let pictures: Vec<_> = pagination
            .pages
            .iter()
            .flat_map(|page| &page.images)
            .map(|picture| picture.id.as_str())
            .collect();
        assert_eq!(pictures, ["lead", "middle-a", "middle-b", "tail"]);
        let rendered = pagination
            .pages
            .iter()
            .flat_map(|page| &page.lines)
            .map(|line| line.text.as_str())
            .collect::<String>();
        assert_eq!(rendered, text);
        assert_image_pages_bounded(&pagination, 300.0);
    }

    #[test]
    fn tall_images_fit_the_body_and_keep_bitmap_aspect_ratio() {
        let layout = Layout {
            width: 200.0,
            height: 200.0,
            ..compact_layout()
        };
        let pagination = Paginator::new_with_images(
            "",
            &layout,
            &TextMetrics::default(),
            100.0,
            &[image("tall", 0, 100.0, 10_000.0)],
        );
        let placed = &pagination.pages[0].images[0];
        assert!((placed.height - 100.0).abs() < 0.01);
        assert!(((placed.height - IMAGE_CAPTION_HEIGHT) / placed.width - 100.0).abs() < 0.01);
        assert_image_pages_bounded(&pagination, 100.0);
    }

    #[test]
    fn unicode_and_paragraph_delimiters_survive_images_at_every_scalar_boundary() {
        let text = "\u{7532}e\u{301}\u{1f600}\n\u{4e59}\r\n\u{4e19}\n\nend";
        let visible: String = text
            .chars()
            .filter(|character| !matches!(character, '\r' | '\n'))
            .collect();
        let layout = Layout {
            height: 120.0,
            paragraph_gap: 0.4,
            ..compact_layout()
        };
        for anchor in 0..=text.chars().count() as u32 {
            let pagination = Paginator::new_with_images(
                text,
                &layout,
                &TextMetrics::default(),
                100.0,
                &[image("boundary", anchor, 20.0, 20.0)],
            );
            let rendered = pagination
                .pages
                .iter()
                .flat_map(|page| &page.lines)
                .map(|line| line.text.as_str())
                .collect::<String>();
            assert_eq!(rendered, visible, "anchor {anchor}");
            for line in pagination.pages.iter().flat_map(|page| &page.lines) {
                assert_eq!(line.text, text_range(text, line.start, line.end));
            }
            assert_image_pages_bounded(&pagination, 100.0);
        }
    }

    #[test]
    fn image_before_a_paragraph_break_does_not_invent_an_empty_line() {
        let text = "a\nb\r\nc\n\nd";
        let layout = Layout {
            height: 300.0,
            paragraph_gap: 0.4,
            ..compact_layout()
        };
        let pagination = Paginator::new_with_images(
            text,
            &layout,
            &TextMetrics::default(),
            300.0,
            &[image("between", 1, 20.0, 20.0)],
        );
        let lines: Vec<_> = pagination
            .pages
            .iter()
            .flat_map(|page| &page.lines)
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(lines, ["a", "b", "c", "", "d"]);
        assert_image_pages_bounded(&pagination, 300.0);
    }

    #[test]
    fn image_only_pages_have_deterministic_text_seek_and_explicit_image_seek() {
        let layout = Layout {
            width: 100.0,
            height: 100.0,
            ..compact_layout()
        };
        let pictures = [
            image("one", 0, 80.0, 60.0),
            image("two", 0, 80.0, 60.0),
            image("three", 0, 80.0, 60.0),
        ];
        let mut pagination =
            Paginator::new_with_images("", &layout, &TextMetrics::default(), 100.0, &pictures);
        assert_eq!(pagination.pages.len(), 3);
        assert!(
            pagination
                .pages
                .iter()
                .all(|page| page.start == 0 && page.end == 0)
        );
        assert_eq!(pagination.page_for_offset(0), 0);
        assert_eq!(pagination.page_for_image("two"), Some(1));
        pagination.seek_image("three").unwrap();
        assert_eq!(pagination.current_index(), 2);
        let smaller = Layout {
            width: 80.0,
            height: 80.0,
            ..layout
        };
        pagination.reflow_with_images("", &smaller, &TextMetrics::default(), 80.0, &pictures, 0);
        assert_eq!(pagination.current_page().images[0].id, "three");
        assert_image_pages_bounded(&pagination, 80.0);
    }

    #[test]
    fn invalid_or_unmapped_image_geometry_does_not_change_the_text() {
        let text = "An unchanged chapter.";
        let layout = compact_layout();
        let normal = Paginator::new(text, &layout, &TextMetrics::default());
        let with_invalid = Paginator::new_with_images(
            text,
            &layout,
            &TextMetrics::default(),
            layout.height,
            &[
                image("bad-anchor", 1000, 20.0, 20.0),
                image("bad-width", 0, f32::NAN, 20.0),
                image("bad-height", 0, 20.0, 0.0),
            ],
        );
        assert_eq!(normal.pages, with_invalid.pages);
    }

    #[test]
    fn chinese_closing_punctuation_does_not_start_a_wrapped_line() {
        let text = "\u{7532}\u{4e59}\u{4e19}\u{4e01}\u{620a}\u{5df1}\u{5e9a}\u{8f9b}\u{58ec}\u{7678}\u{3002}\u{7532}\u{4e59}\u{4e19}\u{300a}\u{4e01}\u{620a}\u{300b}\u{5df1}\u{ff0c}\u{5e9a}\u{8f9b}\u{58ec}";
        let value = Paginator::new(text, &compact_layout(), &TextMetrics::default());
        for line in value.pages.iter().flat_map(|page| &page.lines) {
            assert!(!line.text.chars().next().is_some_and(is_closing));
            assert!(!line.text.chars().last().is_some_and(is_opening));
        }
    }

    #[test]
    fn combining_marks_remain_with_their_base_character() {
        let text = "aaaaaaaaaaaaaaaae\u{301}bbbbbbbbbbbbbbb";
        let value = Paginator::new(text, &compact_layout(), &TextMetrics::default());
        assert!(
            value
                .pages
                .iter()
                .flat_map(|page| &page.lines)
                .all(|line| !line.text.starts_with('\u{301}'))
        );
    }

    #[test]
    fn empty_and_tiny_layouts_remain_navigable() {
        let layout = Layout {
            width: 0.0,
            height: 0.0,
            ..Layout::default()
        };
        let mut empty = Paginator::new("", &layout, &TextMetrics::default());
        assert_eq!(empty.pages.len(), 1);
        assert!(empty.next_page().is_none());
        assert!(empty.previous_page().is_none());
        let narrow = Paginator::new("abc", &layout, &TextMetrics::default());
        assert!(!narrow.pages.is_empty());
        assert_eq!(narrow.pages.last().unwrap().end, 3);
    }

    #[test]
    fn supplied_font_metrics_drive_layout() {
        let Some(path) = std::env::var_os("WEREAD_TEST_FONT") else {
            return;
        };
        let collection_index = std::env::var("WEREAD_TEST_FONT_INDEX")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let bytes = std::fs::read(path).expect("Read the explicitly supplied test font");
        let metrics = TextMetrics::from_font(bytes, collection_index).expect("Parse test font");
        assert!(metrics.is_font_backed());
        assert!(metrics.advance('i', 22.0) < metrics.advance('W', 22.0));
        assert!(metrics.advance('\u{4e2d}', 22.0) > 0.0);
        assert!(
            (metrics.advance('\u{4e2d}', 44.0) - metrics.advance('\u{4e2d}', 22.0) * 2.0).abs()
                < 0.01
        );
        let text = "An actual font controls Latin spacing and CJK text.".repeat(40);
        let value = Paginator::new(&text, &compact_layout(), &metrics);
        assert!(value.pages.len() > 2);
        for line in value.pages.iter().flat_map(|page| &page.lines) {
            assert!(
                (line.width - metrics.measure(&line.text, value.layout.font_size)).abs() < 0.01
            );
        }
    }

    #[test]
    fn invalid_font_data_is_rejected_without_affecting_the_fallback() {
        assert!(TextMetrics::from_font(b"not a font", 0).is_err());
        assert!(TextMetrics::from_font_owned(vec![], 0).is_err());
        assert!(!TextMetrics::default().is_font_backed());
        assert!(TextMetrics::default().advance('\u{4e2d}', 22.0).is_finite());
    }

    #[test]
    fn gestures_enforce_thresholds_and_ignore_vertical_motion() {
        let mut recognizer = GestureRecognizer::default();
        recognizer.press(300.0, 200.0, 0);
        assert_eq!(
            recognizer.release(235.0, 205.0, 200, 632.0),
            ReaderGesture::Next
        );
        recognizer.press(300.0, 200.0, 0);
        assert_eq!(
            recognizer.release(299.0, 290.0, 200, 632.0),
            ReaderGesture::None
        );
        recognizer.press(100.0, 200.0, 0);
        assert!(matches!(
            recognizer.release(101.0, 201.0, 600, 632.0),
            ReaderGesture::LongPress { .. }
        ));
        recognizer.tap_mode = TapMode::Mirrored;
        recognizer.press(10.0, 200.0, 0);
        assert_eq!(
            recognizer.release(10.0, 200.0, 100, 632.0),
            ReaderGesture::Next
        );
    }
}
