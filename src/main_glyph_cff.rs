//! Bounded borrowed Type 2 counting for the locked software outline consumer.

use super::font_budget::{FontBudget, FontLimitError, GlyphWorkKind};
use super::glyph_budget::{Cursor, add, dword, mul, word};

type Result<T> = std::result::Result<T, FontLimitError>;

const MAX_CALL_DEPTH: usize = 10;
const MAX_STACK: usize = 513;
const PROGRAM_SCRATCH: usize = 16 * MAX_STACK + 4096;

#[derive(Clone, Copy)]
enum Number {
    Integer(i32),
    Fixed,
}

struct Stack {
    values: [Number; MAX_STACK],
    len: usize,
    limit: usize,
}

impl Stack {
    fn new() -> Self {
        Self {
            values: [Number::Integer(0); MAX_STACK],
            len: 0,
            limit: MAX_STACK,
        }
    }

    fn push(&mut self, value: Number) -> Result<()> {
        if self.len >= self.limit {
            return Err(FontLimitError::GlyphComplexity);
        }
        self.values[self.len] = value;
        self.len += 1;
        Ok(())
    }

    fn pop(&mut self) -> Result<Number> {
        self.len = self
            .len
            .checked_sub(1)
            .ok_or(FontLimitError::GlyphProgram)?;
        Ok(self.values[self.len])
    }

    fn integer(&mut self) -> Result<i32> {
        match self.pop()? {
            Number::Integer(value) => Ok(value),
            // The locked consumer distinguishes a literal integer from 16.16,
            // including a fixed-point result which happens to be integral.
            Number::Fixed => Err(FontLimitError::GlyphProgram),
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }
}

#[derive(Clone, Copy)]
struct Index<'a> {
    offsets: &'a [u8],
    data: &'a [u8],
    count: usize,
    width: usize,
}

impl<'a> Index<'a> {
    fn empty() -> Self {
        Self {
            offsets: &[],
            data: &[],
            count: 0,
            width: 0,
        }
    }

    fn read(cursor: &mut Cursor<'a>, cff2: bool, budget: &mut FontBudget) -> Result<Self> {
        cursor.work_kind = GlyphWorkKind::IndexRead;
        let count = if cff2 {
            dword(cursor.take(4, budget)?, 0)? as usize
        } else {
            cursor.word(budget)? as usize
        };
        if count == 0 {
            return Ok(Self::empty());
        }
        let width = cursor.byte(budget)? as usize;
        if !(1..=4).contains(&width) {
            return Err(FontLimitError::GlyphProgram);
        }
        let offsets = cursor.take(mul(add(count, 1)?, width)?, budget)?;
        let mut index = Self {
            offsets,
            data: &[],
            count,
            width,
        };
        if index.offset(0)? != 1 {
            return Err(FontLimitError::GlyphProgram);
        }
        let mut previous = 1;
        for element in 1..=count {
            let offset = index.offset(element)?;
            if offset < previous {
                return Err(FontLimitError::GlyphProgram);
            }
            previous = offset;
        }
        index.data = cursor.span(
            previous
                .checked_sub(1)
                .ok_or(FontLimitError::GlyphProgram)?,
            budget,
        )?;
        Ok(index)
    }

    fn offset(self, element: usize) -> Result<usize> {
        let start = mul(element, self.width)?;
        let bytes = self
            .offsets
            .get(start..add(start, self.width)?)
            .ok_or(FontLimitError::GlyphProgram)?;
        bytes.iter().try_fold(0usize, |offset, &byte| {
            offset
                .checked_mul(256)
                .and_then(|offset| offset.checked_add(byte as usize))
                .ok_or(FontLimitError::GlyphProgram)
        })
    }

    fn get(self, element: usize) -> Result<&'a [u8]> {
        if element >= self.count {
            return Err(FontLimitError::GlyphProgram);
        }
        let start = self
            .offset(element)?
            .checked_sub(1)
            .ok_or(FontLimitError::GlyphProgram)?;
        let end = self
            .offset(add(element, 1)?)?
            .checked_sub(1)
            .ok_or(FontLimitError::GlyphProgram)?;
        self.data
            .get(start..end)
            .ok_or(FontLimitError::GlyphProgram)
    }

    fn subroutine(self, operand: i32) -> Result<&'a [u8]> {
        let bias = if self.count < 1240 {
            107
        } else if self.count < 33900 {
            1131
        } else {
            32768
        };
        let index = operand
            .checked_add(bias)
            .ok_or(FontLimitError::GlyphProgram)?;
        self.get(usize::try_from(index).map_err(|_| FontLimitError::GlyphProgram)?)
    }
}

#[derive(Default)]
struct Dict {
    charstrings: Option<usize>,
    private: Option<(usize, usize)>,
    fd_array: Option<usize>,
    fd_select: Option<usize>,
    subrs: Option<usize>,
    projected_work: usize,
}

fn nonnegative(value: Number) -> Result<usize> {
    match value {
        Number::Integer(value) => usize::try_from(value).map_err(|_| FontLimitError::GlyphProgram),
        Number::Fixed => Err(FontLimitError::GlyphProgram),
    }
}

fn read_number(
    first: u8,
    cursor: &mut Cursor<'_>,
    dictionary: bool,
    budget: &mut FontBudget,
) -> Result<Number> {
    Ok(match first {
        32..=246 => Number::Integer(i32::from(first) - 139),
        247..=250 => {
            Number::Integer((i32::from(first) - 247) * 256 + i32::from(cursor.byte(budget)?) + 108)
        }
        251..=254 => {
            Number::Integer(-(i32::from(first) - 251) * 256 - i32::from(cursor.byte(budget)?) - 108)
        }
        28 => Number::Integer(i32::from(cursor.word(budget)? as i16)),
        29 if dictionary => Number::Integer(dword(cursor.take(4, budget)?, 0)? as i32),
        30 if dictionary => {
            // DICT reals describe matrices and other metadata. Affine
            // transforms do not add vertices; pixel bounds have their own
            // renderer admission. Keep a non-integer marker without a String.
            read_bcd(cursor, budget)?;
            Number::Fixed
        }
        255 if !dictionary => {
            cursor.take(4, budget)?;
            Number::Fixed
        }
        _ => return Err(FontLimitError::GlyphProgram),
    })
}

fn read_bcd(cursor: &mut Cursor<'_>, budget: &mut FontBudget) -> Result<usize> {
    enum Phase {
        Integer,
        Fraction,
        Exponent,
    }
    let start = cursor.offset;
    let mut phase = Phase::Integer;
    let mut exponent = 0usize;
    loop {
        let byte = cursor.byte(budget)?;
        for nibble in [byte >> 4, byte & 0x0f] {
            match phase {
                Phase::Integer => match nibble {
                    0..=9 | 0x0e => {}
                    0x0a => phase = Phase::Fraction,
                    0x0b | 0x0c => phase = Phase::Exponent,
                    _ => return Ok(cursor.offset - start),
                },
                Phase::Fraction => match nibble {
                    0..=9 => {}
                    0x0b | 0x0c => phase = Phase::Exponent,
                    _ => return Ok(cursor.offset - start),
                },
                Phase::Exponent => match nibble {
                    0..=9 => {
                        // The locked decoder checks the prior exponent after
                        // reading this byte, including an early low-nibble exit.
                        if exponent > 1000 {
                            return Ok(cursor.offset - start);
                        }
                        exponent = exponent * 10 + nibble as usize;
                    }
                    _ => return Ok(cursor.offset - start),
                },
            }
        }
    }
}

fn read_dict(bytes: &[u8], budget: &mut FontBudget) -> Result<Dict> {
    let mut cursor = Cursor::at_as(bytes, 0, GlyphWorkKind::DictParse)?;
    let mut operands = [Number::Integer(0); MAX_STACK];
    let mut len = 0;
    let mut dict = Dict::default();
    let mut matrix_projection = 0usize;
    let mut renderer_cursor_pos = 0usize;
    while cursor.offset < bytes.len() {
        let first = cursor.byte(budget)?;
        if first >= 32 || matches!(first, 28..=30) {
            if len == operands.len() {
                return Err(FontLimitError::GlyphComplexity);
            }
            operands[len] = read_number(first, &mut cursor, true, budget)?;
            len += 1;
            continue;
        }
        let operator = if first == 12 {
            0x0c00 | u16::from(cursor.byte(budget)?)
        } else {
            u16::from(first)
        };
        match operator {
            17 => {
                dict.charstrings = Some(nonnegative(
                    *operands
                        .get(len.checked_sub(1).ok_or(FontLimitError::GlyphProgram)?)
                        .ok_or(FontLimitError::GlyphProgram)?,
                )?);
            }
            18 => {
                if len < 2 {
                    return Err(FontLimitError::GlyphProgram);
                }
                dict.private = Some((nonnegative(operands[0])?, nonnegative(operands[1])?));
            }
            19 => {
                dict.subrs = Some(nonnegative(
                    *operands
                        .get(len.checked_sub(1).ok_or(FontLimitError::GlyphProgram)?)
                        .ok_or(FontLimitError::GlyphProgram)?,
                )?);
            }
            0x0c06 => {
                if len != 1 || nonnegative(operands[0])? != 2 {
                    return Err(FontLimitError::GlyphFormat);
                }
            }
            0x0c24 => {
                dict.fd_array = Some(nonnegative(
                    *operands
                        .get(len.checked_sub(1).ok_or(FontLimitError::GlyphProgram)?)
                        .ok_or(FontLimitError::GlyphProgram)?,
                )?);
            }
            0x0c25 => {
                dict.fd_select = Some(nonnegative(
                    *operands
                        .get(len.checked_sub(1).ok_or(FontLimitError::GlyphProgram)?)
                        .ok_or(FontLimitError::GlyphProgram)?,
                )?);
            }
            0x0c07 => {
                let prefix = matrix_prefix_work(bytes, renderer_cursor_pos, budget)?;
                matrix_projection = add(matrix_projection, add(prefix, 64)?)?;
            }
            // CFF2 variation operators can change later stack operands. Do not
            // inspect default coordinates and then publish an unproved program.
            22 | 23 | 24 => return Err(FontLimitError::GlyphFormat),
            _ => {}
        }
        if known_dict_operator(operator) && operator != 0x0c07 {
            // The locked iterator updates this position only after a known
            // ordinary operator. Invalid operators and FontMatrix keep it.
            renderer_cursor_pos = cursor.offset;
        }
        len = 0;
    }
    // The locked DICT iterator reparses earlier operands for FontMatrix before
    // updating cursor_pos. Repeated matrices can therefore re-read a long BCD
    // prefix. Keep one whole-DICT scan and each actual six-number prefix per
    // renderer miss even though preflight caches this borrowed descriptor.
    dict.projected_work = add(bytes.len(), matrix_projection)?;
    Ok(dict)
}

fn known_dict_operator(operator: u16) -> bool {
    // Matches read-fonts 0.41.0 ps/cff/dict.rs from_opcode and
    // from_extended_opcode, including known fields this counter ignores.
    matches!(operator, 0..=11 | 13..=24 | 0x0c00..=0x0c0e | 0x0c11..=0x0c17 | 0x0c1e..=0x0c26)
}

fn matrix_prefix_take<'a>(
    cursor: &mut Cursor<'a>,
    work: &mut usize,
    count: usize,
    budget: &mut FontBudget,
) -> Result<Option<&'a [u8]>> {
    *work = add(*work, count)?;
    match cursor.take(count, budget) {
        Ok(bytes) => Ok(Some(bytes)),
        // A failed numeric reparse makes the real iterator ignore this
        // matrix. Price the attempted read without changing that behavior.
        Err(FontLimitError::GlyphProgram) => Ok(None),
        Err(error) => Err(error),
    }
}

fn matrix_prefix_work(bytes: &[u8], start: usize, budget: &mut FontBudget) -> Result<usize> {
    let mut cursor = Cursor::at_as(bytes, start, GlyphWorkKind::DictParse)?;
    let mut work = 0usize;
    // ScaledFontMatrix::parse reparses exactly six dynamically scaled numbers,
    // not the remainder of the DICT. Its scalar normalization stays prepaid.
    for _ in 0..6 {
        let Some(first) = matrix_prefix_take(&mut cursor, &mut work, 1, budget)? else {
            return Ok(work);
        };
        match first[0] {
            32..=246 => {}
            247..=254 => {
                if matrix_prefix_take(&mut cursor, &mut work, 1, budget)?.is_none() {
                    return Ok(work);
                }
            }
            28 => {
                if matrix_prefix_take(&mut cursor, &mut work, 2, budget)?.is_none() {
                    return Ok(work);
                }
            }
            29 => {
                if matrix_prefix_take(&mut cursor, &mut work, 4, budget)?.is_none() {
                    return Ok(work);
                }
            }
            30 => {
                let before = cursor.offset;
                match read_bcd(&mut cursor, budget) {
                    Ok(bytes) => work = add(work, bytes)?,
                    Err(FontLimitError::GlyphProgram) => {
                        return add(work, add(cursor.offset - before, 1)?);
                    }
                    Err(error) => return Err(error),
                }
            }
            _ => return Ok(work),
        }
    }
    Ok(work)
}

#[derive(Clone, Copy)]
enum Select<'a> {
    First,
    Direct(&'a [u8]),
    Ranges {
        bytes: &'a [u8],
        count: usize,
        wide: bool,
    },
}

impl<'a> Select<'a> {
    fn read(
        table: &'a [u8],
        offset: Option<usize>,
        glyphs: usize,
        fonts: usize,
        budget: &mut FontBudget,
    ) -> Result<Self> {
        let Some(offset) = offset else {
            return Ok(Self::First);
        };
        let mut cursor = Cursor::at(table, offset)?;
        match cursor.byte(budget)? {
            0 => {
                let bytes = cursor.take(glyphs, budget)?;
                if bytes.iter().any(|&font| font as usize >= fonts) {
                    return Err(FontLimitError::GlyphProgram);
                }
                Ok(Self::Direct(bytes))
            }
            format @ (3 | 4) => {
                let wide = format == 4;
                let count = if wide {
                    dword(cursor.take(4, budget)?, 0)? as usize
                } else {
                    cursor.word(budget)? as usize
                };
                if count == 0 {
                    return Err(FontLimitError::GlyphProgram);
                }
                let width = if wide { 6 } else { 3 };
                let bytes = cursor.take(mul(count, width)?, budget)?;
                let sentinel = if wide {
                    dword(cursor.take(4, budget)?, 0)? as usize
                } else {
                    cursor.word(budget)? as usize
                };
                if sentinel != glyphs {
                    return Err(FontLimitError::GlyphProgram);
                }
                let select = Self::Ranges { bytes, count, wide };
                let mut previous = None;
                for index in 0..count {
                    let (start, font) = select.range(index)?;
                    if start >= glyphs
                        || font >= fonts
                        || previous.is_some_and(|previous| start <= previous)
                    {
                        return Err(FontLimitError::GlyphProgram);
                    }
                    if index == 0 && start != 0 {
                        return Err(FontLimitError::GlyphProgram);
                    }
                    previous = Some(start);
                }
                Ok(select)
            }
            _ => Err(FontLimitError::GlyphFormat),
        }
    }

    fn range(self, index: usize) -> Result<(usize, usize)> {
        let Self::Ranges { bytes, count, wide } = self else {
            return Err(FontLimitError::GlyphProgram);
        };
        if index >= count {
            return Err(FontLimitError::GlyphProgram);
        }
        let start = mul(index, if wide { 6 } else { 3 })?;
        Ok(if wide {
            (
                dword(bytes, start)? as usize,
                word(bytes, add(start, 4)?)? as usize,
            )
        } else {
            (
                word(bytes, start)? as usize,
                *bytes
                    .get(add(start, 2)?)
                    .ok_or(FontLimitError::GlyphProgram)? as usize,
            )
        })
    }

    fn font(self, glyph: usize, budget: &mut FontBudget) -> Result<usize> {
        budget.reserve_glyph_work(1)?;
        match self {
            Self::First => Ok(0),
            Self::Direct(bytes) => {
                Ok(*bytes.get(glyph).ok_or(FontLimitError::GlyphProgram)? as usize)
            }
            Self::Ranges { count, .. } => {
                let mut low = 0;
                let mut high = count;
                while low < high {
                    budget.reserve_glyph_work_as(GlyphWorkKind::IndexRead, 8)?;
                    let middle = low + (high - low) / 2;
                    if self.range(middle)?.0 <= glyph {
                        low = middle + 1;
                    } else {
                        high = middle;
                    }
                }
                self.range(low.checked_sub(1).ok_or(FontLimitError::GlyphProgram)?)
                    .map(|(_, font)| font)
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Local<'a> {
    subrs: Index<'a>,
    projected_work: usize,
}

fn private_subrs<'a>(
    table: &'a [u8],
    private: Option<(usize, usize)>,
    cff2: bool,
    budget: &mut FontBudget,
) -> Result<Local<'a>> {
    let Some((length, start)) = private else {
        return Ok(Local {
            subrs: Index::empty(),
            projected_work: 0,
        });
    };
    let bytes = table
        .get(start..add(start, length)?)
        .ok_or(FontLimitError::GlyphProgram)?;
    let dict = read_dict(bytes, budget)?;
    let subrs = if let Some(offset) = dict.subrs {
        Index::read(&mut Cursor::at(table, add(start, offset)?)?, cff2, budget)?
    } else {
        Index::empty()
    };
    Ok(Local {
        subrs,
        projected_work: dict.projected_work,
    })
}

pub(super) fn admit_cff(
    table: &[u8],
    cff2: bool,
    glyphs: usize,
    budget: &mut FontBudget,
) -> Result<()> {
    if PROGRAM_SCRATCH + 4 * 64 > budget.limits.per_glyph_memory {
        return Err(FontLimitError::GlyphMemory);
    }
    let mut cursor = Cursor::at(table, 0)?;
    let major = cursor.byte(budget)?;
    let minor = cursor.byte(budget)?;
    let header = cursor.byte(budget)? as usize;
    if minor != 0 || major != if cff2 { 2 } else { 1 } {
        return Err(FontLimitError::GlyphProgram);
    }
    let (top, globals) = if cff2 {
        let length = cursor.word(budget)? as usize;
        if header < 5 {
            return Err(FontLimitError::GlyphProgram);
        }
        cursor.take(header - 5, budget)?;
        let top = cursor.span(length, budget)?;
        let globals = Index::read(&mut cursor, true, budget)?;
        (top, globals)
    } else {
        let width = cursor.byte(budget)?;
        if header < 4 || !(1..=4).contains(&width) {
            return Err(FontLimitError::GlyphProgram);
        }
        cursor.take(header - 4, budget)?;
        let _names = Index::read(&mut cursor, false, budget)?;
        let top = Index::read(&mut cursor, false, budget)?;
        if top.count != 1 {
            return Err(FontLimitError::GlyphFormat);
        }
        let _strings = Index::read(&mut cursor, false, budget)?;
        let globals = Index::read(&mut cursor, false, budget)?;
        budget.reserve_glyph_work_as(GlyphWorkKind::IndexRead, 8)?;
        (top.get(0)?, globals)
    };
    let top = read_dict(top, budget)?;
    let charstrings = Index::read(
        &mut Cursor::at(table, top.charstrings.ok_or(FontLimitError::GlyphProgram)?)?,
        cff2,
        budget,
    )?;
    if charstrings.count != glyphs {
        return Err(FontLimitError::GlyphProgram);
    }
    let fonts = if let Some(offset) = top.fd_array {
        Some(Index::read(&mut Cursor::at(table, offset)?, cff2, budget)?)
    } else {
        None
    };
    let font_count = fonts.map_or(1, |fonts| fonts.count);
    if font_count == 0 {
        return Err(FontLimitError::GlyphProgram);
    }
    let select = Select::read(table, top.fd_select, glyphs, font_count, budget)?;
    let state_bytes = add(
        mul(font_count, std::mem::size_of::<Option<Local<'_>>>())?,
        std::mem::size_of::<Program<'_>>(),
    )?;
    budget.reserve_glyph_state(state_bytes)?;
    let mut locals = Vec::new();
    locals
        .try_reserve_exact(font_count)
        .map_err(|_| FontLimitError::GlyphMemory)?;
    locals.resize(font_count, None);
    #[cfg(test)]
    {
        budget.glyph_state_allocations += 1;
    }
    for glyph in 0..glyphs {
        let font = select.font(glyph, budget)?;
        let slot = locals.get_mut(font).ok_or(FontLimitError::GlyphProgram)?;
        let local = if let Some(local) = *slot {
            local
        } else {
            let (private, fd_work) = if let Some(fonts) = fonts {
                budget.reserve_glyph_work_as(GlyphWorkKind::IndexRead, 8)?;
                let fd = read_dict(fonts.get(font)?, budget)?;
                (fd.private, fd.projected_work)
            } else {
                (top.private, 0)
            };
            let mut local = private_subrs(table, private, cff2, budget)?;
            local.projected_work = add(local.projected_work, fd_work)?;
            *slot = Some(local);
            local
        };
        // Scaler construction repeats fixed CFF/index/table lookups on each
        // miss as well as the Top/FD/Private scans priced by the descriptors.
        budget.reserve_glyph_work_as(
            GlyphWorkKind::DictProjection,
            add(add(top.projected_work, local.projected_work)?, 64)?,
        )?;
        budget.reserve_glyph_work_as(GlyphWorkKind::IndexRead, 8)?;
        let mut state = Program::new(local.subrs, globals);
        #[cfg(test)]
        {
            budget.glyph_graph_nodes += 1;
        }
        let seen_endchar = state.walk(charstrings.get(glyph)?, 0, budget)?;
        if !seen_endchar {
            state.endchar(budget)?;
        }
        state.close(budget)?;
        budget.reserve_glyph_summary(state.visits, add(state.points, 4)?, state.memory()?)?;
    }
    Ok(())
}

struct Program<'a> {
    stack: Stack,
    local: Index<'a>,
    global: Index<'a>,
    width_seen: bool,
    hints: usize,
    points: usize,
    verbs: usize,
    visits: usize,
    contour: bool,
}

impl<'a> Program<'a> {
    fn new(local: Index<'a>, global: Index<'a>) -> Self {
        Self {
            stack: Stack::new(),
            local,
            global,
            width_seen: false,
            hints: 0,
            points: 0,
            verbs: 0,
            visits: 1,
            contour: false,
        }
    }

    fn memory(&self) -> Result<usize> {
        // Conservative path vectors, their growth/copy overlap, the fixed
        // argument stack, and the bounded call frames. No path is built here.
        add(
            add(mul(add(self.points, 4)?, 64)?, mul(self.verbs, 16)?)?,
            PROGRAM_SCRATCH,
        )
    }

    fn emit(&mut self, points: usize, verbs: usize, budget: &mut FontBudget) -> Result<()> {
        let points = add(self.points, points)?;
        let verbs = add(self.verbs, verbs)?;
        if add(points, 4)? > 65_535 || add(points, 4)? > budget.limits.per_glyph_points {
            return Err(FontLimitError::GlyphComplexity);
        }
        let memory = add(
            add(mul(add(points, 4)?, 64)?, mul(verbs, 16)?)?,
            PROGRAM_SCRATCH,
        )?;
        if memory > budget.limits.per_glyph_memory {
            return Err(FontLimitError::GlyphMemory);
        }
        budget.reserve_glyph_work_as(
            GlyphWorkKind::Output,
            add(points - self.points, verbs - self.verbs)?,
        )?;
        self.points = points;
        self.verbs = verbs;
        Ok(())
    }

    fn close(&mut self, budget: &mut FontBudget) -> Result<()> {
        if self.contour {
            self.emit(0, 1, budget)?;
            self.contour = false;
        }
        Ok(())
    }

    fn move_args(&mut self, expected: usize) -> Result<()> {
        let len = self.stack.len;
        if !self.width_seen {
            self.width_seen = true;
        }
        if len < expected {
            return Err(FontLimitError::GlyphProgram);
        }
        self.stack.clear();
        Ok(())
    }

    fn stems(&mut self) -> Result<()> {
        let mut len = self.stack.len;
        if !self.width_seen {
            self.width_seen = true;
            if len % 2 != 0 {
                len -= 1;
            }
        }
        if len % 2 != 0 {
            return Err(FontLimitError::GlyphProgram);
        }
        self.hints = add(self.hints, len / 2)?;
        self.stack.clear();
        Ok(())
    }

    fn path(&mut self, points: usize, verbs: usize, budget: &mut FontBudget) -> Result<()> {
        if !self.contour {
            // The locked consumer opens an implicit contour at (0, 0) before
            // the first line or curve even without an explicit moveto.
            self.emit(1, 1, budget)?;
            self.contour = true;
        }
        self.emit(points, verbs, budget)?;
        self.stack.clear();
        Ok(())
    }

    fn endchar(&mut self, budget: &mut FontBudget) -> Result<()> {
        // read_width() in the real interpreter observes the first operand;
        // it does not pop it. A caller may still use that operand after a
        // child frame's endchar.
        budget.reserve_glyph_work_as(GlyphWorkKind::Program, 1)?;
        self.width_seen = true;
        if self.stack.len > 1 {
            // Explicit and synthesized endchar both reach implicit SEAC in
            // the real consumer. A child endchar does not close the contour
            // or erase its caller's stack and width state.
            return Err(FontLimitError::GlyphFormat);
        }
        Ok(())
    }

    fn walk(&mut self, bytes: &'a [u8], depth: usize, budget: &mut FontBudget) -> Result<bool> {
        if depth > MAX_CALL_DEPTH {
            return Err(FontLimitError::GlyphComplexity);
        }
        let mut cursor = Cursor::at_as(bytes, 0, GlyphWorkKind::Program)?;
        while cursor.offset < bytes.len() {
            let first = cursor.byte(budget)?;
            #[cfg(test)]
            {
                budget.glyph_program_tokens += 1;
            }
            if first >= 32 || matches!(first, 28 | 255) {
                self.stack
                    .push(read_number(first, &mut cursor, false, budget)?)?;
                continue;
            }
            let operator = if first == 12 {
                0x0c00 | u16::from(cursor.byte(budget)?)
            } else {
                u16::from(first)
            };
            let len = self.stack.len;
            // Operand loops, including stems which emit no path vertices,
            // still consume work in the actual interpreter.
            budget.reserve_glyph_work_as(GlyphWorkKind::Operand, len)?;
            match operator {
                1 | 3 | 18 | 23 => self.stems()?,
                19 | 20 => {
                    self.stems()?;
                    cursor.take(add(self.hints, 7)? / 8, budget)?;
                }
                4 | 21 | 22 => {
                    self.move_args(if operator == 21 { 2 } else { 1 })?;
                    self.close(budget)?;
                    self.emit(1, 1, budget)?;
                    self.contour = true;
                }
                5 => {
                    if len < 2 || len % 2 != 0 {
                        return Err(FontLimitError::GlyphProgram);
                    }
                    self.path(len / 2, len / 2, budget)?;
                }
                6 | 7 => {
                    if len == 0 {
                        return Err(FontLimitError::GlyphProgram);
                    }
                    self.path(len, len, budget)?;
                }
                8 => {
                    if len < 6 || len % 6 != 0 {
                        return Err(FontLimitError::GlyphProgram);
                    }
                    self.path(mul(len / 6, 3)?, len / 6, budget)?;
                }
                24 => {
                    if len < 8 || (len - 2) % 6 != 0 {
                        return Err(FontLimitError::GlyphProgram);
                    }
                    self.path(
                        add(mul((len - 2) / 6, 3)?, 1)?,
                        add((len - 2) / 6, 1)?,
                        budget,
                    )?;
                }
                25 => {
                    if len < 8 || (len - 6) % 2 != 0 {
                        return Err(FontLimitError::GlyphProgram);
                    }
                    self.path(add((len - 6) / 2, 3)?, add((len - 6) / 2, 1)?, budget)?;
                }
                26 | 27 | 30 | 31 => {
                    if len < 4 || !matches!(len % 4, 0 | 1) {
                        return Err(FontLimitError::GlyphProgram);
                    }
                    self.path(mul(len / 4, 3)?, len / 4, budget)?;
                }
                0x0c22..=0x0c25 => {
                    let expected = match operator {
                        0x0c22 => 7,
                        0x0c23 => 13,
                        0x0c24 => 9,
                        _ => 11,
                    };
                    if len != expected {
                        return Err(FontLimitError::GlyphProgram);
                    }
                    self.path(6, 2, budget)?;
                }
                10 | 29 => {
                    let operand = self.stack.integer()?;
                    budget.reserve_glyph_work_as(GlyphWorkKind::IndexRead, 8)?;
                    let program = if operator == 10 {
                        self.local
                    } else {
                        self.global
                    }
                    .subroutine(operand)?;
                    self.visits = add(self.visits, 1)?;
                    if self.visits > budget.limits.per_glyph_visits {
                        return Err(FontLimitError::GlyphComplexity);
                    }
                    budget.reserve_glyph_work_as(GlyphWorkKind::Program, 1)?;
                    #[cfg(test)]
                    {
                        budget.glyph_subroutine_calls += 1;
                    }
                    // Parameters, width and hint count belong to the glyph,
                    // not the subroutine ID. Never reuse a context-free memo.
                    let _child_endchar = self.walk(program, add(depth, 1)?, budget)?;
                }
                11 => return Ok(false),
                14 => {
                    self.endchar(budget)?;
                    return Ok(true);
                }
                0x0c0c => {
                    self.stack.pop()?;
                    self.stack.pop()?;
                    self.stack.push(Number::Fixed)?;
                }
                // These legacy operations are no-ops for the actual Type 2
                // consumer. They neither create geometry nor reset arguments.
                0x0c00 | 0x0c07 | 0x0c11 | 0x0c21 | 13 => {}
                0x0c01 | 0x0c02 => self.stack.clear(),
                // SEAC/OtherSubr can create extra glyphs or legacy flex. CFF2
                // blend/vsindex can alter later control operands. Refuse them
                // explicitly until a complete borrowed proof is implemented.
                0x0c06 | 0x0c10 | 15 | 16 => return Err(FontLimitError::GlyphFormat),
                _ => return Err(FontLimitError::GlyphFormat),
            }
        }
        Ok(false)
    }
}
