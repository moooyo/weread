//! Borrowed outline admission before any font reaches Slint's shared registry.

use super::font_budget::{FontBudget, FontLimitError, GlyphWorkKind, MAX_VARIATION_AXES};

type Result<T> = std::result::Result<T, FontLimitError>;

const MAX_COMPONENT_DEPTH: usize = 32;
const MAX_SCALER_POINTS: usize = 65_535;
const PHANTOM_POINTS: usize = 4;
const MAX_GLYPH_ROOTS: usize = u16::MAX as usize + 1;

#[derive(Default)]
struct Tables<'a> {
    head: Option<&'a [u8]>,
    maxp: Option<&'a [u8]>,
    loca: Option<&'a [u8]>,
    glyf: Option<&'a [u8]>,
    gvar: Option<&'a [u8]>,
    cff: Option<&'a [u8]>,
    cff2: Option<&'a [u8]>,
    varc: Option<&'a [u8]>,
}

pub(super) fn admit_font_glyphs(bytes: &[u8], index: u32, budget: &mut FontBudget) -> Result<()> {
    budget.reserve_glyph_work(32)?;
    let raw =
        owned_ttf_parser::RawFace::parse(bytes, index).map_err(|_| FontLimitError::GlyphProgram)?;
    budget.reserve_glyph_work(
        usize::from(raw.table_records.len())
            .checked_mul(16)
            .ok_or(FontLimitError::GlyphWork)?,
    )?;
    let mut tables = Tables::default();
    for record in raw.table_records {
        let slot = if record.tag == owned_ttf_parser::Tag::from_bytes(b"head") {
            &mut tables.head
        } else if record.tag == owned_ttf_parser::Tag::from_bytes(b"maxp") {
            &mut tables.maxp
        } else if record.tag == owned_ttf_parser::Tag::from_bytes(b"loca") {
            &mut tables.loca
        } else if record.tag == owned_ttf_parser::Tag::from_bytes(b"glyf") {
            &mut tables.glyf
        } else if record.tag == owned_ttf_parser::Tag::from_bytes(b"gvar") {
            &mut tables.gvar
        } else if record.tag == owned_ttf_parser::Tag::from_bytes(b"CFF ") {
            &mut tables.cff
        } else if record.tag == owned_ttf_parser::Tag::from_bytes(b"CFF2") {
            &mut tables.cff2
        } else if record.tag == owned_ttf_parser::Tag::from_bytes(b"VARC") {
            &mut tables.varc
        } else {
            continue;
        };
        let start = record.offset as usize;
        let end = start
            .checked_add(record.length as usize)
            .ok_or(FontLimitError::GlyphProgram)?;
        let table = bytes.get(start..end).ok_or(FontLimitError::GlyphProgram)?;
        if slot.is_some_and(|old| old.as_ptr() != table.as_ptr() || old.len() != table.len()) {
            // Different parsers may choose different duplicate directory rows.
            // Identical borrowed spans are safe; distinct outline sources are not.
            return Err(FontLimitError::GlyphProgram);
        }
        *slot = Some(table);
    }
    if tables.varc.is_some() {
        // Skrifa gives VARC priority over base outlines. Its conditional graph
        // cannot be admitted by silently inspecting only glyf or CFF instead.
        return Err(FontLimitError::GlyphFormat);
    }
    budget.reserve_glyph_work(6)?;
    let glyphs = word(tables.maxp.ok_or(FontLimitError::GlyphProgram)?, 4)? as usize;
    if glyphs == 0 {
        return Err(FontLimitError::GlyphProgram);
    }
    let mut supported = false;
    if let Some(glyf) = tables.glyf {
        budget.reserve_glyph_work(54)?;
        let head = tables.head.ok_or(FontLimitError::GlyphProgram)?;
        let long = match word(head, 50)? {
            0 => false,
            1 => true,
            _ => return Err(FontLimitError::GlyphProgram),
        };
        let (offsets, outline_glyphs) = OffsetArray::loca(
            tables.loca.ok_or(FontLimitError::GlyphProgram)?,
            glyphs,
            long,
            glyf.len(),
            budget,
        )?;
        let variation = tables
            .gvar
            .map(|table| Gvar::new(table, budget))
            .transpose()?;
        let state_bytes = outline_glyphs
            .checked_mul(std::mem::size_of::<State>())
            .ok_or(FontLimitError::GlyphMemory)?;
        budget.reserve_glyph_state(state_bytes)?;
        let mut states = Vec::new();
        states
            .try_reserve_exact(outline_glyphs)
            .map_err(|_| FontLimitError::GlyphMemory)?;
        states.resize(outline_glyphs, State::Unseen);
        #[cfg(test)]
        {
            budget.glyph_state_allocations += 1;
        }
        let mut graph = GlyfGraph {
            glyf,
            offsets,
            declared_glyphs: glyphs,
            variation,
            states,
            budget,
        };
        for glyph in 0..outline_glyphs {
            graph.visit(glyph, 0)?;
        }
        supported = true;
    }
    if let Some(cff) = tables.cff {
        super::glyph_cff::admit_cff(cff, false, glyphs, budget)?;
        supported = true;
    }
    if let Some(cff) = tables.cff2 {
        super::glyph_cff::admit_cff(cff, true, glyphs, budget)?;
        supported = true;
    }
    if !supported {
        return Err(FontLimitError::GlyphFormat);
    }
    Ok(())
}

#[derive(Clone, Copy, Default)]
struct Summary {
    visits: usize,
    points: usize,
    contours: usize,
    simple_points: usize,
    other_points: usize,
    component_stack: usize,
    variation_work: usize,
    depth: usize,
}

impl Summary {
    fn add_child(&mut self, child: Self) -> Result<()> {
        self.visits = add(self.visits, child.visits)?;
        self.points = add(self.points, child.points)?;
        self.contours = add(self.contours, child.contours)?;
        self.simple_points = self.simple_points.max(child.simple_points);
        self.other_points = self.other_points.max(child.other_points);
        self.component_stack = self.component_stack.max(child.component_stack);
        self.variation_work = add(self.variation_work, child.variation_work)?;
        self.depth = self.depth.max(add(child.depth, 1)?);
        self.check_expansion()
    }

    fn check_expansion(self) -> Result<()> {
        if add(self.points, PHANTOM_POINTS)? > MAX_SCALER_POINTS {
            return Err(FontLimitError::GlyphComplexity);
        }
        Ok(())
    }

    fn memory(self, variations: bool) -> Result<usize> {
        // Locked Skrifa 0.44.0 unhinted glyf layout: scaled points, flags,
        // overlap scratch, contours, and optional variation/IUP scratch. The
        // root has four phantom points once, not four per expanded component.
        let points = add(self.points, PHANTOM_POINTS)?;
        let mut memory = add(mul(points, 9)?, mul(self.other_points, 8)?)?;
        memory = add(memory, mul(self.contours, 2)?)?;
        if variations {
            memory = add(memory, mul(self.simple_points, 16)?)?;
            memory = add(memory, mul(self.component_stack, 8)?)?;
        }
        add(memory, 4)
    }
}

#[derive(Clone, Copy)]
enum State {
    Unseen,
    Visiting,
    Ready(Summary),
}

struct GlyfGraph<'a, 'b> {
    glyf: &'a [u8],
    offsets: OffsetArray<'a>,
    declared_glyphs: usize,
    variation: Option<Gvar<'a>>,
    states: Vec<State>,
    budget: &'b mut FontBudget,
}

impl<'a> GlyfGraph<'a, '_> {
    fn glyph(&mut self, glyph: usize) -> Result<&'a [u8]> {
        self.budget
            .reserve_glyph_work(if self.offsets.long { 8 } else { 4 })?;
        let start = self.offsets.get(glyph)?;
        let end = self.offsets.get(add(glyph, 1)?)?;
        if start == end {
            return Ok(&[]);
        }
        if glyph >= self.declared_glyphs && (start > end || end > self.glyf.len()) {
            // Whole-table loca can expose an alignment-padding row. The real
            // consumer fails this range before expanding any outline. Preserve
            // that harmless row instead of rejecting every padded font.
            return Ok(&[]);
        }
        let data = self
            .glyf
            .get(start..end)
            .ok_or(FontLimitError::GlyphProgram)?;
        if glyph >= self.declared_glyphs && data.len() < 12 {
            self.budget.reserve_glyph_work(2)?;
            if data.len() < 10 || word(data, 0)? as i16 >= 0 {
                // A short zero-filled glyf padding span cannot form the
                // locked consumer's twelve-byte SimpleGlyph header.
                return Ok(&[]);
            }
        }
        Ok(data)
    }

    fn visit(&mut self, glyph: usize, depth: usize) -> Result<Summary> {
        self.budget.reserve_glyph_work(1)?;
        if depth > MAX_COMPONENT_DEPTH {
            return Err(FontLimitError::GlyphComplexity);
        }
        match self
            .states
            .get(glyph)
            .copied()
            .ok_or(FontLimitError::GlyphProgram)?
        {
            State::Visiting => return Err(FontLimitError::GlyphProgram),
            State::Ready(summary) => {
                if add(depth, summary.depth)? > MAX_COMPONENT_DEPTH {
                    return Err(FontLimitError::GlyphComplexity);
                }
                return Ok(summary);
            }
            State::Unseen => {}
        }
        self.states[glyph] = State::Visiting;
        #[cfg(test)]
        {
            self.budget.glyph_graph_nodes += 1;
        }
        let data = self.glyph(glyph)?;
        let mut summary = Summary {
            visits: 1,
            ..Summary::default()
        };
        let mut components = 0usize;
        if !data.is_empty() {
            self.budget.reserve_glyph_work(10)?;
            data.get(..10).ok_or(FontLimitError::GlyphProgram)?;
            let contours = word(data, 0)? as i16;
            if contours > 0 {
                let (points, contours) = simple_glyph(data, contours as usize, self.budget)?;
                summary.points = points;
                summary.contours = contours;
                summary.simple_points = add(points, PHANTOM_POINTS)?;
                summary.other_points = summary.simple_points;
            } else if contours == 0 {
                // A present zero-contour SimpleGlyph still owns phantom-point
                // scratch. Only an empty loca span is a missing glyph.
                let mut cursor = Cursor::at(data, 10)?;
                let instructions = cursor.word(self.budget)? as usize;
                cursor.take(instructions, self.budget)?;
                summary.simple_points = PHANTOM_POINTS;
                summary.other_points = PHANTOM_POINTS;
            } else if contours < 0 {
                let mut cursor = Cursor::at(data, 10)?;
                let last_flags = loop {
                    let flags = cursor.word(self.budget)?;
                    let child = cursor.word(self.budget)? as usize;
                    let arguments = if flags & 1 != 0 { 4 } else { 2 };
                    let transform = if flags & 8 != 0 {
                        2
                    } else if flags & 64 != 0 {
                        4
                    } else if flags & 128 != 0 {
                        8
                    } else {
                        0
                    };
                    cursor.take(add(arguments, transform)?, self.budget)?;
                    components = add(components, 1)?;
                    #[cfg(test)]
                    {
                        self.budget.glyph_component_records += 1;
                    }
                    let child = self.visit(child, add(depth, 1)?)?;
                    summary.add_child(child)?;
                    if summary.visits > self.budget.limits.per_glyph_visits
                        || add(summary.points, PHANTOM_POINTS)?
                            > self.budget.limits.per_glyph_points
                    {
                        return Err(FontLimitError::GlyphComplexity);
                    }
                    if flags & 32 == 0 {
                        break flags;
                    }
                };
                summary.component_stack = add(summary.component_stack, add(components, 4)?)?;
                if last_flags & 256 != 0 {
                    let instructions = cursor.word(self.budget)? as usize;
                    cursor.take(instructions, self.budget)?;
                    if instructions != 0 {
                        summary.other_points = summary.other_points.max(add(summary.points, 4)?);
                    }
                }
            }
        }
        summary.check_expansion()?;
        if let Some(variation) = self.variation {
            let local_points = if components == 0 {
                add(summary.points, PHANTOM_POINTS)?
            } else {
                add(components, PHANTOM_POINTS)?
            };
            let local_contours = if components == 0 { summary.contours } else { 0 };
            let work = variation.work(glyph, local_points, local_contours, self.budget)?;
            summary.variation_work = add(summary.variation_work, work)?;
        }
        // Account for all repeated child variation passes before allowing the
        // real scaler to execute even one of them. Raw summaries remain small.
        self.budget.reserve_glyph_work(summary.variation_work)?;
        self.budget.reserve_glyph_summary(
            summary.visits,
            add(summary.points, PHANTOM_POINTS)?,
            summary.memory(self.variation.is_some())?,
        )?;
        self.states[glyph] = State::Ready(summary);
        Ok(summary)
    }
}

fn simple_glyph(data: &[u8], contours: usize, budget: &mut FontBudget) -> Result<(usize, usize)> {
    let mut cursor = Cursor::at(data, 10)?;
    let ends = cursor.take(mul(contours, 2)?, budget)?;
    let mut last = None;
    for endpoint in ends.chunks_exact(2) {
        let endpoint = word(endpoint, 0)? as usize;
        if last.is_some_and(|previous| endpoint <= previous) {
            return Err(FontLimitError::GlyphProgram);
        }
        last = Some(endpoint);
    }
    let points = add(last.ok_or(FontLimitError::GlyphProgram)?, 1)?;
    if add(points, PHANTOM_POINTS)? > MAX_SCALER_POINTS
        || add(points, PHANTOM_POINTS)? > budget.limits.per_glyph_points
    {
        return Err(FontLimitError::GlyphComplexity);
    }
    let instructions = cursor.word(budget)? as usize;
    cursor.take(instructions, budget)?;
    // A repeated flag represents several real point decodes in the scaler.
    // Price that logical work without allocating or reconstructing any point.
    budget.reserve_glyph_work(points)?;
    let mut decoded = 0usize;
    let mut x_bytes = 0usize;
    let mut y_bytes = 0usize;
    while decoded < points {
        let flags = cursor.byte(budget)?;
        let repetitions = if flags & 8 != 0 {
            add(cursor.byte(budget)? as usize, 1)?
        } else {
            1
        };
        decoded = add(decoded, repetitions)?;
        if decoded > points {
            return Err(FontLimitError::GlyphProgram);
        }
        let x = if flags & 2 != 0 {
            1
        } else if flags & 16 == 0 {
            2
        } else {
            0
        };
        let y = if flags & 4 != 0 {
            1
        } else if flags & 32 == 0 {
            2
        } else {
            0
        };
        x_bytes = add(x_bytes, mul(x, repetitions)?)?;
        y_bytes = add(y_bytes, mul(y, repetitions)?)?;
    }
    cursor.take(add(x_bytes, y_bytes)?, budget)?;
    Ok((points, contours))
}

#[derive(Clone, Copy)]
struct OffsetArray<'a> {
    bytes: &'a [u8],
    long: bool,
}

impl<'a> OffsetArray<'a> {
    fn loca(
        bytes: &'a [u8],
        declared: usize,
        long: bool,
        data_len: usize,
        budget: &mut FontBudget,
    ) -> Result<(Self, usize)> {
        // Read-fonts exposes the complete loca table, while Swash and compound
        // records supply u16 GIDs without a maxp count fence. Admit every root
        // with two complete offsets before reserving its state or walking it.
        // Partial alignment bytes do not add another usable offset.
        budget.reserve_glyph_work(bytes.len())?;
        let width = if long { 4 } else { 2 };
        let required = mul(add(declared, 1)?, width)?;
        bytes.get(..required).ok_or(FontLimitError::GlyphProgram)?;
        let roots = (bytes.len() / width).saturating_sub(1).min(MAX_GLYPH_ROOTS);
        let prefix = mul(add(roots, 1)?, width)?;
        let offsets = Self {
            bytes: bytes.get(..prefix).ok_or(FontLimitError::GlyphProgram)?,
            long,
        };
        // Keep the existing declared-prefix integrity check. A later padding
        // row can be empty or unreadable, but a valid later range must be
        // visited even when maxp reports fewer glyphs.
        let mut previous = 0;
        for glyph in 0..=declared {
            let offset = offsets.get(glyph)?;
            if offset < previous || offset > data_len {
                return Err(FontLimitError::GlyphProgram);
            }
            previous = offset;
        }
        Ok((offsets, roots))
    }

    fn new(
        bytes: &'a [u8],
        glyphs: usize,
        long: bool,
        data_len: usize,
        budget: &mut FontBudget,
    ) -> Result<Self> {
        let length = mul(add(glyphs, 1)?, if long { 4 } else { 2 })?;
        budget.reserve_glyph_work(length)?;
        let bytes = bytes.get(..length).ok_or(FontLimitError::GlyphProgram)?;
        let offsets = Self { bytes, long };
        let mut previous = 0;
        for glyph in 0..=glyphs {
            let offset = offsets.get(glyph)?;
            if offset < previous || offset > data_len {
                return Err(FontLimitError::GlyphProgram);
            }
            previous = offset;
        }
        Ok(offsets)
    }

    fn get(self, glyph: usize) -> Result<usize> {
        if self.long {
            Ok(dword(self.bytes, mul(glyph, 4)?)? as usize)
        } else {
            mul(word(self.bytes, mul(glyph, 2)?)? as usize, 2)
        }
    }
}

#[derive(Clone, Copy)]
struct Gvar<'a> {
    data: &'a [u8],
    offsets: OffsetArray<'a>,
    glyphs: usize,
    axes: usize,
    shared_tuples: usize,
}

impl<'a> Gvar<'a> {
    fn new(table: &'a [u8], budget: &mut FontBudget) -> Result<Self> {
        budget.reserve_glyph_work(20)?;
        if word(table, 0)? != 1 {
            return Err(FontLimitError::GlyphProgram);
        }
        let axes = word(table, 4)? as usize;
        if axes > MAX_VARIATION_AXES {
            return Err(FontLimitError::GlyphComplexity);
        }
        let shared_tuples = word(table, 6)? as usize;
        let shared_start = dword(table, 8)? as usize;
        let shared_len = mul(mul(shared_tuples, axes)?, 2)?;
        budget.reserve_glyph_work(shared_len)?;
        table
            .get(shared_start..add(shared_start, shared_len)?)
            .ok_or(FontLimitError::GlyphProgram)?;
        let glyphs = word(table, 12)? as usize;
        let long = word(table, 14)? & 1 != 0;
        let start = dword(table, 16)? as usize;
        let data = table.get(start..).ok_or(FontLimitError::GlyphProgram)?;
        let offsets = OffsetArray::new(
            table.get(20..).ok_or(FontLimitError::GlyphProgram)?,
            glyphs,
            long,
            data.len(),
            budget,
        )?;
        Ok(Self {
            data,
            offsets,
            glyphs,
            axes,
            shared_tuples,
        })
    }

    fn work(
        self,
        glyph: usize,
        points: usize,
        contours: usize,
        budget: &mut FontBudget,
    ) -> Result<usize> {
        if glyph >= self.glyphs {
            return Ok(0);
        }
        let start = self.offsets.get(glyph)?;
        let end = self.offsets.get(add(glyph, 1)?)?;
        let data = self
            .data
            .get(start..end)
            .ok_or(FontLimitError::GlyphProgram)?;
        if data.is_empty() {
            return Ok(0);
        }
        budget.reserve_glyph_work(data.len())?;
        let tuple_flags = word(data, 0)?;
        let tuples = tuple_flags as usize & 0x0fff;
        let payload_start = word(data, 2)? as usize;
        let headers = data
            .get(..payload_start)
            .ok_or(FontLimitError::GlyphProgram)?;
        let mut cursor = Cursor::at(headers, 4)?;
        let mut payload = Cursor::at(data, payload_start)?;
        let shared_points = if tuple_flags & 0x8000 != 0 {
            Some(packed_points(&mut payload, points, budget)?)
        } else {
            None
        };
        let mut missing_point_work = 0;
        for _ in 0..tuples {
            let length = cursor.word(budget)? as usize;
            let index = cursor.word(budget)?;
            if index & 0x8000 != 0 {
                cursor.take(mul(self.axes, 2)?, budget)?;
            } else if (index as usize & 0x0fff) >= self.shared_tuples {
                return Err(FontLimitError::GlyphProgram);
            }
            if index & 0x4000 != 0 {
                cursor.take(mul(self.axes, 4)?, budget)?;
            }
            let body = payload.take(length, budget)?;
            let mut deltas = Cursor::at(body, 0)?;
            let point_set = if index & 0x2000 != 0 {
                Some(packed_points(&mut deltas, points, budget)?)
            } else {
                shared_points
            };
            if point_set.is_none() {
                // Missing point lists are sparse, not dense. The locked
                // composite iterator consumes all packed deltas and can visit
                // more indices than the actual component count before u16 ends.
                let decoded = packed_delta_count(&mut deltas, budget)?;
                let possible_points = (decoded / 2).min(65_536);
                missing_point_work = add(
                    missing_point_work,
                    mul(possible_points.saturating_sub(points), 8)?,
                )?;
            }
        }
        // Include every possible tuple and sparse-point/IUP pass. No default
        // variation coordinate can hide a branch. Composite tuples address
        // component positions, while simple tuples address their raw points.
        let tuple_work = add(
            add(mul(self.axes, 3)?, mul(points, 8)?)?,
            add(mul(contours, 8)?, 32)?,
        )?;
        add(
            add(data.len(), mul(tuples, tuple_work)?)?,
            missing_point_work,
        )
    }
}

#[derive(Clone, Copy)]
enum PointSet {
    Dense,
    Sparse,
}

fn packed_points(
    cursor: &mut Cursor<'_>,
    maximum: usize,
    budget: &mut FontBudget,
) -> Result<PointSet> {
    let first = cursor.byte(budget)?;
    if first == 0 {
        return Ok(PointSet::Dense);
    }
    let count = if first & 0x80 != 0 {
        ((first as usize & 0x7f) << 8) | cursor.byte(budget)? as usize
    } else {
        first as usize
    };
    if count > maximum {
        return Err(FontLimitError::GlyphProgram);
    }
    budget.reserve_glyph_work(count)?;
    let mut decoded = 0;
    let mut point = 0;
    while decoded < count {
        let control = cursor.byte(budget)?;
        let run = add(control as usize & 0x7f, 1)?;
        decoded = add(decoded, run)?;
        if decoded > count {
            return Err(FontLimitError::GlyphProgram);
        }
        for _ in 0..run {
            let delta = if control & 0x80 != 0 {
                cursor.word(budget)? as usize
            } else {
                cursor.byte(budget)? as usize
            };
            point = add(point, delta)?;
            if point >= maximum {
                return Err(FontLimitError::GlyphProgram);
            }
        }
    }
    Ok(PointSet::Sparse)
}

fn packed_delta_count(cursor: &mut Cursor<'_>, budget: &mut FontBudget) -> Result<usize> {
    let mut count = 0;
    while cursor.offset < cursor.bytes.len() {
        let control = cursor.byte(budget)?;
        let run = add(control as usize & 0x3f, 1)?;
        count = add(count, run)?;
        if control & 0xc0 != 0x80 {
            let width = match control & 0xc0 {
                0x40 => 2,
                0xc0 => 4,
                _ => 1,
            };
            cursor.take(mul(run, width)?, budget)?;
        }
    }
    Ok(count)
}

pub(super) struct Cursor<'a> {
    pub(super) bytes: &'a [u8],
    pub(super) offset: usize,
    pub(super) work_kind: GlyphWorkKind,
}

impl<'a> Cursor<'a> {
    pub(super) fn at(bytes: &'a [u8], offset: usize) -> Result<Self> {
        Self::at_as(bytes, offset, GlyphWorkKind::General)
    }

    pub(super) fn at_as(bytes: &'a [u8], offset: usize, work_kind: GlyphWorkKind) -> Result<Self> {
        if offset > bytes.len() {
            return Err(FontLimitError::GlyphProgram);
        }
        Ok(Self {
            bytes,
            offset,
            work_kind,
        })
    }

    pub(super) fn take(&mut self, count: usize, budget: &mut FontBudget) -> Result<&'a [u8]> {
        budget.reserve_glyph_work_as(self.work_kind, count)?;
        let end = add(self.offset, count)?;
        let data = self
            .bytes
            .get(self.offset..end)
            .ok_or(FontLimitError::GlyphProgram)?;
        self.offset = end;
        Ok(data)
    }

    /// Bounds-check and borrow an uninspected payload without pricing its
    /// entire length as a scan. Readers which inspect it still pay through
    /// take/byte/word under their own work category.
    pub(super) fn span(&mut self, count: usize, budget: &mut FontBudget) -> Result<&'a [u8]> {
        budget.reserve_glyph_work_as(GlyphWorkKind::IndexSpan, 4)?;
        let end = add(self.offset, count)?;
        let data = self
            .bytes
            .get(self.offset..end)
            .ok_or(FontLimitError::GlyphProgram)?;
        self.offset = end;
        Ok(data)
    }

    pub(super) fn byte(&mut self, budget: &mut FontBudget) -> Result<u8> {
        Ok(self.take(1, budget)?[0])
    }

    pub(super) fn word(&mut self, budget: &mut FontBudget) -> Result<u16> {
        word(self.take(2, budget)?, 0)
    }
}

pub(super) fn word(bytes: &[u8], offset: usize) -> Result<u16> {
    let bytes = bytes
        .get(offset..add(offset, 2)?)
        .ok_or(FontLimitError::GlyphProgram)?;
    Ok(u16::from_be_bytes(bytes.try_into().unwrap()))
}

pub(super) fn dword(bytes: &[u8], offset: usize) -> Result<u32> {
    let bytes = bytes
        .get(offset..add(offset, 4)?)
        .ok_or(FontLimitError::GlyphProgram)?;
    Ok(u32::from_be_bytes(bytes.try_into().unwrap()))
}

pub(super) fn add(left: usize, right: usize) -> Result<usize> {
    left.checked_add(right)
        .ok_or(FontLimitError::GlyphComplexity)
}

pub(super) fn mul(left: usize, right: usize) -> Result<usize> {
    left.checked_mul(right)
        .ok_or(FontLimitError::GlyphComplexity)
}
