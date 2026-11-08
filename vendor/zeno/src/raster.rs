//! Path rasterizer.

#![allow(clippy::too_many_arguments)]

use super::geometry::{Point, Vector};
use super::fill_raster_budget::{FillRasterBudget, FillRasterLimitKind};
use super::path_builder::PathBuilder;
use super::style::Fill;

use crate::lib::Vec;
use core::fmt;

#[inline(always)]
fn coverage(fill: Fill, mut coverage: i32) -> u8 {
    coverage >>= PIXEL_BITS * 2 + 1 - 8;
    if fill == Fill::EvenOdd {
        coverage &= 511;
        if coverage >= 256 {
            coverage = 511i32.wrapping_sub(coverage);
        }
    } else {
        if coverage < 0 {
            coverage = !coverage;
        }
        if coverage >= 256 {
            coverage = 255;
        }
    }
    coverage as u8
}

pub struct Rasterizer<'a, S: RasterStorage> {
    storage: &'a mut S,
    xmin: i32,
    xmax: i32,
    ymin: i32,
    ymax: i32,
    height: i32,
    shift: Vector,
    start: FixedPoint,
    closed: bool,
    current: Point,
    x: i32,
    y: i32,
    px: i32,
    py: i32,
    cover: i32,
    area: i32,
    invalid: bool,
}

impl<'a, S: RasterStorage> Rasterizer<'a, S> {
    pub fn new(storage: &'a mut S) -> Self {
        Self {
            storage,
            xmin: 0,
            xmax: 0,
            ymin: 0,
            ymax: 0,
            height: 0,
            shift: Vector::ZERO,
            start: FixedPoint::default(),
            closed: false,
            current: Point::ZERO,
            x: 0,
            y: 0,
            px: 0,
            py: 0,
            cover: 0,
            area: 0,
            invalid: false,
        }
    }

    pub fn rasterize(
        &mut self,
        shift: Vector,
        width: u32,
        height: u32,
        apply: &mut impl FnMut(&mut Self),
        fill: Fill,
        buffer: &mut [u8],
        pitch: usize,
        y_up: bool,
    ) {
        let w = width as i32;
        let h = height as i32;
        self.storage
            .reset(FixedPoint { x: 0, y: 0 }, FixedPoint { x: w, y: h });
        if self.storage.failed() {
            return;
        }
        self.shift = shift;
        self.start = FixedPoint::default();
        self.closed = true;
        self.current = Point::ZERO;
        self.xmin = 0;
        self.ymin = 0;
        self.xmax = w;
        self.ymax = h;
        self.height = h;
        self.x = 0;
        self.y = 0;
        self.px = 0;
        self.py = 0;
        self.invalid = true;
        apply(self);
        if self.storage.failed() {
            return;
        }
        if !self.closed {
            self.line_to(self.start);
            if self.storage.failed() {
                return;
            }
        }
        if !self.invalid {
            self.storage.set(self.x, self.y, self.area, self.cover);
        }
        if self.storage.failed() {
            return;
        }
        let indices = self.storage.indices();
        let cells = self.storage.cells();
        let min = FixedPoint::new(self.xmin, self.ymin);
        let max = FixedPoint::new(self.xmax, self.ymax);
        let height = height as usize;
        for i in 0..indices.len() {
            if !self.storage.charge_work(1) {
                return;
            }
            let index = indices[i];
            if index != -1 {
                let row_offset = match self.output_row(i, pitch, height, y_up) {
                    Some(offset) => offset,
                    None => return,
                };
                if self.storage.bounded() && row_offset > buffer.len() {
                    self.storage.refuse_target(row_offset as u64, buffer.len() as u64);
                    return;
                }
                let row = &mut buffer[row_offset..];
                let mut x = min.x;
                let mut cover = 0;
                let mut area;
                let mut index = index;
                loop {
                    if !self.storage.charge_work(1) {
                        return;
                    }
                    let cell = &cells[index as usize];
                    let next_x = match self.output_next_x(cell.x) {
                        Some(x) => x,
                        None => return,
                    };
                    if cover != 0 && cell.x > x {
                        let (xi, count, end) = match self.output_span(x, cell.x) {
                            Some(span) => span,
                            None => return,
                        };
                        if !self.storage.charge_work(count) {
                            return;
                        }
                        if self.storage.bounded() && end > row.len() {
                            self.storage.refuse_target(end as u64, row.len() as u64);
                            return;
                        }
                        let c = coverage(fill, cover);
                        for b in &mut row[xi..end] {
                            *b = c;
                        }
                    }
                    cover = cover.wrapping_add(cell.cover.wrapping_mul(ONE_PIXEL * 2));
                    area = cover.wrapping_sub(cell.area);
                    if area != 0 && cell.x >= min.x {
                        let (xi, count, end) = match self.output_span(cell.x, next_x) {
                            Some(span) => span,
                            None => return,
                        };
                        if !self.storage.charge_work(count) {
                            return;
                        }
                        if self.storage.bounded() && end > row.len() {
                            self.storage.refuse_target(end as u64, row.len() as u64);
                            return;
                        }
                        let c = coverage(fill, area);
                        for b in &mut row[xi..end] {
                            *b = c;
                        }
                    }
                    x = next_x;
                    index = cell.next;
                    if index == -1 {
                        break;
                    }
                }
                if cover != 0 {
                    let (xi, count, end) = match self.output_span(x, max.x) {
                        Some(span) => span,
                        None => return,
                    };
                    if !self.storage.charge_work(count) {
                        return;
                    }
                    if self.storage.bounded() && end > row.len() {
                        self.storage.refuse_target(end as u64, row.len() as u64);
                        return;
                    }
                    let c = coverage(fill, cover);
                    for b in &mut row[xi..end] {
                        *b = c;
                    }
                }
            }
        }
    }

    pub fn rasterize_write(
        &mut self,
        shift: Vector,
        width: u32,
        height: u32,
        apply: &mut impl FnMut(&mut Self),
        fill: Fill,
        pitch: usize,
        y_up: bool,
        write: &mut impl FnMut(usize, usize, usize, u8),
    ) {
        let w = width as i32;
        let h = height as i32;
        self.storage
            .reset(FixedPoint { x: 0, y: 0 }, FixedPoint { x: w, y: h });
        if self.storage.failed() {
            return;
        }
        self.shift = shift;
        self.start = FixedPoint::default();
        self.closed = true;
        self.current = Point::ZERO;
        self.xmin = 0;
        self.ymin = 0;
        self.xmax = w;
        self.ymax = h;
        self.height = h;
        self.x = 0;
        self.y = 0;
        self.px = 0;
        self.py = 0;
        self.invalid = true;
        apply(self);
        if self.storage.failed() {
            return;
        }
        if !self.closed {
            self.line_to(self.start);
            if self.storage.failed() {
                return;
            }
        }
        if !self.invalid {
            self.storage.set(self.x, self.y, self.area, self.cover);
        }
        if self.storage.failed() {
            return;
        }
        let indices = self.storage.indices();
        let cells = self.storage.cells();
        let min = FixedPoint::new(self.xmin, self.ymin);
        let max = FixedPoint::new(self.xmax, self.ymax);
        let height = height as usize;
        for i in 0..indices.len() {
            if !self.storage.charge_work(1) {
                return;
            }
            let index = indices[i];
            if index != -1 {
                let row_offset = match self.output_row(i, pitch, height, y_up) {
                    Some(offset) => offset,
                    None => return,
                };
                let mut x = min.x;
                let mut cover = 0;
                let mut area;
                let mut index = index;
                loop {
                    if !self.storage.charge_work(1) {
                        return;
                    }
                    let cell = &cells[index as usize];
                    let next_x = match self.output_next_x(cell.x) {
                        Some(x) => x,
                        None => return,
                    };
                    if cover != 0 && cell.x > x {
                        let (xi, count, _) = match self.output_span(x, cell.x) {
                            Some(span) => span,
                            None => return,
                        };
                        if !self.storage.charge_work(count) {
                            return;
                        }
                        let c = coverage(fill, cover);
                        write(row_offset, xi, count, c);
                    }
                    cover = cover.wrapping_add(cell.cover.wrapping_mul(ONE_PIXEL * 2));
                    area = cover.wrapping_sub(cell.area);
                    if area != 0 && cell.x >= min.x {
                        let (xi, count, _) = match self.output_span(cell.x, next_x) {
                            Some(span) => span,
                            None => return,
                        };
                        if !self.storage.charge_work(count) {
                            return;
                        }
                        let c = coverage(fill, area);
                        write(row_offset, xi, count, c);
                    }
                    x = next_x;
                    index = cell.next;
                    if index == -1 {
                        break;
                    }
                }
                if cover != 0 {
                    let (xi, count, _) = match self.output_span(x, max.x) {
                        Some(span) => span,
                        None => return,
                    };
                    if !self.storage.charge_work(count) {
                        return;
                    }
                    let c = coverage(fill, cover);
                    write(row_offset, xi, count, c);
                }
            }
        }
    }

    fn output_row(&self, index: usize, pitch: usize, height: usize, y_up: bool) -> Option<usize> {
        if !self.storage.bounded() {
            let y = ((index as i32) - self.ymin) as usize;
            return Some(if y_up { pitch * (height - 1 - y) } else { pitch * y });
        }
        let offset = i32::try_from(index).ok()
            .and_then(|index| index.checked_sub(self.ymin))
            .and_then(|y| usize::try_from(y).ok())
            .filter(|y| *y < height)
            .and_then(|y| {
                if y_up {
                    height.checked_sub(1).and_then(|last| last.checked_sub(y))
                } else {
                    Some(y)
                }
            })
            .and_then(|y| pitch.checked_mul(y));
        if offset.is_none() {
            self.storage.refuse_target(u64::MAX, height as u64);
        }
        offset
    }

    fn output_next_x(&self, x: i32) -> Option<i32> {
        if !self.storage.bounded() {
            return Some(x + 1);
        }
        let next = x.checked_add(1).filter(|next| *next <= self.xmax);
        if next.is_none() {
            self.storage.refuse_target(u64::MAX, self.xmax as u64);
        }
        next
    }

    fn output_span(&self, start: i32, end: i32) -> Option<(usize, usize, usize)> {
        if !self.storage.bounded() {
            let count = (end - start) as usize;
            let start = start as usize;
            return Some((start, count, start + count));
        }
        let width = usize::try_from(self.xmax).ok();
        let span = usize::try_from(start).ok().and_then(|start_index| {
            end.checked_sub(start)
                .and_then(|count| usize::try_from(count).ok())
                .and_then(|count| start_index.checked_add(count).map(|end| (start_index, count, end)))
        }).filter(|(_, _, end)| width.is_some_and(|width| *end <= width));
        if span.is_none() {
            self.storage.refuse_target(u64::MAX, self.xmax as u64);
        }
        span
    }

    #[inline(always)]
    fn set_cell(&mut self, x: i32, y: i32) {
        // Clipped transitions also consume work; they must not keep a line loop alive.
        if !self.storage.charge_work(1) {
            return;
        }
        if !self.invalid && (self.area != 0 || self.cover != 0) {
            self.storage.set(self.x, self.y, self.area, self.cover);
            if self.storage.failed() {
                return;
            }
        }
        self.area = 0;
        self.cover = 0;
        self.x = if x > (self.xmin - 1) {
            x
        } else {
            self.xmin - 1
        };
        self.y = y;
        self.invalid = y >= self.ymax || y < self.ymin || x >= self.xmax;
    }

    fn move_to(&mut self, to: FixedPoint) {
        self.set_cell(trunc(to.x), trunc(to.y));
        if self.storage.failed() {
            return;
        }
        self.px = to.x;
        self.py = to.y;
    }

    fn line_to(&mut self, to: FixedPoint) {
        if !self.storage.charge_work(1) {
            return;
        }
        let to_x = to.x;
        let to_y = to.y;
        let mut ey1 = trunc(self.py);
        let ey2 = trunc(to_y);
        if (ey1 >= self.ymax && ey2 >= self.ymax) || (ey1 < self.ymin && ey2 < self.ymin) {
            self.px = to_x;
            self.py = to_y;
            return;
        }
        let mut ex1 = trunc(self.px);
        let ex2 = trunc(to_x);
        let mut fx1 = fract(self.px);
        let mut fy1 = fract(self.py);
        let dx = to_x - self.px;
        let dy = to_y - self.py;
        if ex1 == ex2 && ey1 == ey2 {
            // empty
        } else if dy == 0 {
            self.set_cell(ex2, ey2);
            if self.storage.failed() {
                return;
            }
            self.px = to_x;
            self.py = to_y;
            return;
        } else if dx == 0 {
            if dy > 0 {
                loop {
                    if !self.storage.charge_work(1) {
                        return;
                    }
                    let fy2 = ONE_PIXEL;
                    self.cover += fy2 - fy1;
                    self.area += (fy2 - fy1) * fx1 * 2;
                    fy1 = 0;
                    ey1 += 1;
                    self.set_cell(ex1, ey1);
                    if self.storage.failed() {
                        return;
                    }
                    if ey1 == ey2 {
                        break;
                    }
                }
            } else {
                loop {
                    if !self.storage.charge_work(1) {
                        return;
                    }
                    let fy2 = 0;
                    self.cover += fy2 - fy1;
                    self.area += (fy2 - fy1) * fx1 * 2;
                    fy1 = ONE_PIXEL;
                    ey1 -= 1;
                    self.set_cell(ex1, ey1);
                    if self.storage.failed() {
                        return;
                    }
                    if ey1 == ey2 {
                        break;
                    }
                }
            }
        } else {
            let mut prod = dx * fy1 - dy * fx1;
            let dx_r = if ex1 != ex2 { (0x00FFFFFF) / dx } else { 0 };
            let dy_r = if ey1 != ey2 { (0x00FFFFFF) / dy } else { 0 };
            fn udiv(a: i32, b: i32) -> i32 {
                ((a as u64 * b as u64) >> (4 * 8 - PIXEL_BITS)) as i32
            }
            loop {
                if !self.storage.charge_work(1) {
                    return;
                }
                if prod <= 0 && prod - dx * ONE_PIXEL > 0 {
                    let fx2 = 0;
                    let fy2 = udiv(-prod, -dx_r);
                    prod -= dy * ONE_PIXEL;
                    self.cover += fy2 - fy1;
                    self.area += (fy2 - fy1) * (fx1 + fx2);
                    fx1 = ONE_PIXEL;
                    fy1 = fy2;
                    ex1 -= 1;
                } else if prod - dx * ONE_PIXEL <= 0 && prod - dx * ONE_PIXEL + dy * ONE_PIXEL > 0 {
                    prod -= dx * ONE_PIXEL;
                    let fx2 = udiv(-prod, dy_r);
                    let fy2 = ONE_PIXEL;
                    self.cover += fy2 - fy1;
                    self.area += (fy2 - fy1) * (fx1 + fx2);
                    fx1 = fx2;
                    fy1 = 0;
                    ey1 += 1;
                } else if prod - dx * ONE_PIXEL + dy * ONE_PIXEL <= 0 && prod + dy * ONE_PIXEL >= 0
                {
                    prod += dy * ONE_PIXEL;
                    let fx2 = ONE_PIXEL;
                    let fy2 = udiv(prod, dx_r);
                    self.cover += fy2 - fy1;
                    self.area += (fy2 - fy1) * (fx1 + fx2);
                    fx1 = 0;
                    fy1 = fy2;
                    ex1 += 1;
                } else {
                    let fx2 = udiv(prod, -dy_r);
                    let fy2 = 0;
                    prod += dx * ONE_PIXEL;
                    self.cover += fy2 - fy1;
                    self.area += (fy2 - fy1) * (fx1 + fx2);
                    fx1 = fx2;
                    fy1 = ONE_PIXEL;
                    ey1 -= 1;
                }
                self.set_cell(ex1, ey1);
                if self.storage.failed() {
                    return;
                }
                if ex1 == ex2 && ey1 == ey2 {
                    break;
                }
            }
        }
        let fx2 = fract(to_x);
        let fy2 = fract(to_y);
        self.cover += fy2 - fy1;
        self.area += (fy2 - fy1) * (fx1 + fx2);
        self.px = to_x;
        self.py = to_y;
    }

    #[allow(clippy::uninit_assumed_init, invalid_value)]
    fn quad_to(&mut self, control: FixedPoint, to: FixedPoint) {
        if !self.storage.charge_work(1) {
            return;
        }
        let mut arc = [FixedPoint::default(); 16 * 2 + 1];
        arc[0].x = to.x;
        arc[0].y = to.y;
        arc[1].x = control.x;
        arc[1].y = control.y;
        arc[2].x = self.px;
        arc[2].y = self.py;
        if (trunc(arc[0].y) >= self.ymax
            && trunc(arc[1].y) >= self.ymax
            && trunc(arc[2].y) >= self.ymax)
            || (trunc(arc[0].y) < self.ymin
                && trunc(arc[1].y) < self.ymin
                && trunc(arc[2].y) < self.ymin)
        {
            self.px = arc[0].x;
            self.py = arc[0].y;
            return;
        }
        let mut dx = (arc[2].x + arc[0].x - 2 * arc[1].x).abs();
        let dy = (arc[2].y + arc[0].y - 2 * arc[1].y).abs();
        if dx < dy {
            dx = dy;
        }
        let mut draw = 1;
        while dx > ONE_PIXEL / 4 {
            if !self.storage.charge_work(1) {
                return;
            }
            dx >>= 2;
            draw <<= 1;
        }
        let mut a = 0;
        loop {
            if !self.storage.charge_work(1) {
                return;
            }
            let mut split = draw & (-draw);
            loop {
                if !self.storage.charge_work(1) {
                    return;
                }
                split >>= 1;
                if split == 0 {
                    break;
                }
                split_quad(&mut arc[a..]);
                a += 2;
            }
            let p = arc[a];
            self.line_to(p);
            if self.storage.failed() {
                return;
            }
            draw -= 1;
            if draw == 0 {
                break;
            }
            a -= 2;
        }
    }

    #[allow(clippy::uninit_assumed_init, invalid_value)]
    fn curve_to(&mut self, control1: FixedPoint, control2: FixedPoint, to: FixedPoint) {
        if !self.storage.charge_work(1) {
            return;
        }
        let mut arc = [FixedPoint::default(); 16 * 8 + 1];
        arc[0].x = to.x;
        arc[0].y = to.y;
        arc[1].x = control2.x;
        arc[1].y = control2.y;
        arc[2].x = control1.x;
        arc[2].y = control1.y;
        arc[3].x = self.px;
        arc[3].y = self.py;
        if (trunc(arc[0].y) >= self.ymax
            && trunc(arc[1].y) >= self.ymax
            && trunc(arc[2].y) >= self.ymax
            && trunc(arc[3].y) >= self.ymax)
            || (trunc(arc[0].y) < self.ymin
                && trunc(arc[1].y) < self.ymin
                && trunc(arc[2].y) < self.ymin
                && trunc(arc[3].y) < self.ymin)
        {
            self.px = arc[0].x;
            self.py = arc[0].y;
            return;
        }
        let mut a = 0;
        loop {
            if !self.storage.charge_work(1) {
                return;
            }
            if (2 * arc[a].x - 3 * arc[a + 1].x + arc[a + 3].x).abs() > ONE_PIXEL / 2
                || (2 * arc[a].y - 3 * arc[a + 1].y + arc[a + 3].y).abs() > ONE_PIXEL / 2
                || (arc[a].x - 3 * arc[a + 2].x + 2 * arc[a + 3].x).abs() > ONE_PIXEL / 2
                || (arc[a].y - 3 * arc[a + 2].y + 2 * arc[a + 3].y).abs() > ONE_PIXEL / 2
            {
                let buf = &mut arc[a..];
                // if buf.len() < 7 {
                //     return;
                // }
                if buf.len() >= 7 {
                    split_cubic(buf);
                    a += 3;
                    continue;
                } else {
                    self.line_to(to);
                    return;
                }
            }
            let p = arc[a];
            self.line_to(p);
            if self.storage.failed() {
                return;
            }
            if a == 0 {
                return;
            }
            a -= 3;
        }
    }
}

impl<S: RasterStorage> PathBuilder for Rasterizer<'_, S> {
    fn current_point(&self) -> Point {
        self.current + self.shift
    }

    #[inline(always)]
    fn move_to(&mut self, to: impl Into<Point>) -> &mut Self {
        if !self.storage.charge_work(1) {
            return self;
        }
        if !self.closed {
            self.line_to(self.start);
            if self.storage.failed() {
                return self;
            }
        }
        let to = to.into();
        let p = FixedPoint::from_point(to + self.shift);
        self.move_to(p);
        if self.storage.failed() {
            return self;
        }
        self.closed = false;
        self.start = p;
        self.current = to;
        self
    }

    #[inline(always)]
    fn line_to(&mut self, to: impl Into<Point>) -> &mut Self {
        if !self.storage.charge_work(1) {
            return self;
        }
        let to = to.into();
        self.current = to;
        self.closed = false;
        self.line_to(FixedPoint::from_point(to + self.shift));
        self
    }

    #[inline(always)]
    fn quad_to(&mut self, control: impl Into<Point>, to: impl Into<Point>) -> &mut Self {
        if !self.storage.charge_work(1) {
            return self;
        }
        let to = to.into();
        self.current = to;
        self.closed = false;
        self.quad_to(
            FixedPoint::from_point(control.into() + self.shift),
            FixedPoint::from_point(to + self.shift),
        );
        self
    }

    #[inline(always)]
    fn curve_to(
        &mut self,
        control1: impl Into<Point>,
        control2: impl Into<Point>,
        to: impl Into<Point>,
    ) -> &mut Self {
        if !self.storage.charge_work(1) {
            return self;
        }
        let to = to.into();
        self.current = to;
        self.closed = false;
        self.curve_to(
            FixedPoint::from_point(control1.into() + self.shift),
            FixedPoint::from_point(control2.into() + self.shift),
            FixedPoint::from_point(to + self.shift),
        );
        self
    }

    #[inline(always)]
    fn close(&mut self) -> &mut Self {
        if !self.storage.charge_work(1) {
            return self;
        }
        self.line_to(self.start);
        if self.storage.failed() {
            return self;
        }
        self.closed = true;
        self
    }
}

#[derive(Copy, Clone, Default)]
pub struct Cell {
    x: i32,
    cover: i32,
    area: i32,
    next: i32,
}

pub trait RasterStorage {
    fn reset(&mut self, min: FixedPoint, max: FixedPoint);
    fn cells(&self) -> &[Cell];
    fn indices(&self) -> &[i32];
    fn set(&mut self, x: i32, y: i32, area: i32, cover: i32);

    #[inline(always)]
    fn charge_work(&self, _accesses: usize) -> bool {
        true
    }

    #[inline(always)]
    fn failed(&self) -> bool {
        false
    }

    #[inline(always)]
    fn bounded(&self) -> bool {
        false
    }

    fn refuse_target(&self, _requested: u64, _limit: u64) {}
}

#[derive(Default)]
pub struct HeapStorage {
    min: FixedPoint,
    max: FixedPoint,
    cells: Vec<Cell>,
    indices: Vec<i32>,
}

impl RasterStorage for HeapStorage {
    fn reset(&mut self, min: FixedPoint, max: FixedPoint) {
        self.min = min;
        self.max = max;
        self.cells.clear();
        self.indices.clear();
        self.indices.resize((max.y - min.y) as usize, -1);
    }

    fn cells(&self) -> &[Cell] {
        &self.cells
    }

    fn indices(&self) -> &[i32] {
        &self.indices
    }

    #[inline(always)]
    #[allow(clippy::comparison_chain)]
    fn set(&mut self, x: i32, y: i32, area: i32, cover: i32) {
        let yindex = (y - self.min.y) as usize;
        let mut cell_index = self.indices[yindex];
        let mut last_index = -1;
        while cell_index != -1 {
            let cell = &mut self.cells[cell_index as usize];
            if cell.x > x {
                break;
            } else if cell.x == x {
                cell.area = cell.area.wrapping_add(area);
                cell.cover = cell.cover.wrapping_add(cover);
                return;
            }
            last_index = cell_index;
            cell_index = cell.next;
        }
        let new_index = self.cells.len();
        let cell = Cell {
            x,
            area,
            cover,
            next: cell_index,
        };
        if last_index != -1 {
            self.cells[last_index as usize].next = new_index as i32;
        } else {
            self.indices[yindex] = new_index as i32;
        }
        self.cells.push(cell);
    }
}

const MIN_FILL_CAPACITY: usize = 4;

/// Only the opt-in fallible Mask API constructs this storage adapter.
pub(super) struct BoundedHeapStorage<'s, 'b> {
    storage: &'s mut HeapStorage,
    budget: &'b FillRasterBudget,
}

impl<'s, 'b> BoundedHeapStorage<'s, 'b> {
    pub fn new(storage: &'s mut HeapStorage, budget: &'b FillRasterBudget) -> Self {
        Self { storage, budget }
    }

    fn invalid_index(&self) {
        self.budget.refuse(FillRasterLimitKind::Coordinates, u64::MAX, i32::MAX as u64);
    }

    fn capacity_target(&self, current: usize, required: usize) -> Option<usize> {
        if current >= required {
            return Some(current);
        }
        // A fixed small initial capacity and doubling preserve amortized growth.
        let target = current.checked_mul(2).map(|doubled| doubled.max(required).max(MIN_FILL_CAPACITY));
        if target.is_none() {
            self.capacity_overflow();
        }
        target
    }

    fn capacity_overflow(&self) {
        self.budget.refuse(
            FillRasterLimitKind::Storage,
            u64::MAX,
            u64::try_from(self.budget.limits().storage_bytes).unwrap_or(u64::MAX),
        );
    }
}

impl RasterStorage for BoundedHeapStorage<'_, '_> {
    fn reset(&mut self, min: FixedPoint, max: FixedPoint) {
        if self.failed() {
            return;
        }
        let height = match max.y.checked_sub(min.y).and_then(|height| usize::try_from(height).ok()) {
            Some(height) => height,
            None => {
                self.invalid_index();
                return;
            }
        };
        let old_index_capacity = self.storage.indices.capacity();
        let index_capacity = match self.capacity_target(old_index_capacity, height) {
            Some(capacity) => capacity,
            None => return,
        };
        // Reset consumes a new reservation for retained/spare capacity as well as
        // row-head initialization. Neither credit is refunded by a later reset.
        if !self.budget.begin_pass(self.storage.cells.capacity(), index_capacity, height)
            || !self.charge_work(height)
        {
            return;
        }
        let growing = old_index_capacity < height;
        if growing && !self.budget.prepay_growth(false, old_index_capacity, index_capacity) {
            return;
        }
        self.storage.min = min;
        self.storage.max = max;
        self.storage.cells.clear();
        self.storage.indices.clear();
        if growing {
            let additional = match index_capacity.checked_sub(self.storage.indices.len()) {
                Some(additional) => additional,
                None => {
                    self.capacity_overflow();
                    return;
                }
            };
            if self.storage.indices.try_reserve_exact(additional).is_err() {
                self.budget.allocation_failed();
                return;
            }
            if !self.budget.grew(false)
                || !self.budget.finish_growth(false, old_index_capacity, self.storage.indices.capacity())
            {
                return;
            }
        }
        self.storage.indices.resize(height, -1);
        self.budget.observe_live(0, height);
    }

    fn cells(&self) -> &[Cell] {
        &self.storage.cells
    }

    fn indices(&self) -> &[i32] {
        &self.storage.indices
    }

    #[inline(always)]
    #[allow(clippy::comparison_chain)]
    fn set(&mut self, x: i32, y: i32, area: i32, cover: i32) {
        if !self.charge_work(1) {
            return;
        }
        let yindex = match y.checked_sub(self.storage.min.y).and_then(|y| usize::try_from(y).ok()) {
            Some(yindex) => yindex,
            None => {
                self.invalid_index();
                return;
            }
        };
        let mut cell_index = match self.storage.indices.get(yindex) {
            Some(index) => *index,
            None => {
                self.invalid_index();
                return;
            }
        };
        let mut last_index = None;
        while cell_index != -1 {
            // Charge each examined node, including every repeated prefix scan.
            if !self.charge_work(1) {
                return;
            }
            let index = match usize::try_from(cell_index) {
                Ok(index) => index,
                Err(_) => {
                    self.invalid_index();
                    return;
                }
            };
            let cell = match self.storage.cells.get_mut(index) {
                Some(cell) => cell,
                None => {
                    self.invalid_index();
                    return;
                }
            };
            if cell.x > x {
                break;
            } else if cell.x == x {
                cell.area = cell.area.wrapping_add(area);
                cell.cover = cell.cover.wrapping_add(cover);
                return;
            }
            last_index = Some(index);
            cell_index = cell.next;
        }
        let new_len = match self.storage.cells.len().checked_add(1) {
            Some(new_len) => new_len,
            None => {
                self.invalid_index();
                return;
            }
        };
        let new_index = match i32::try_from(self.storage.cells.len()) {
            Ok(new_index) => new_index,
            Err(_) => {
                self.invalid_index();
                return;
            }
        };
        // New-cell count and writes are distinct from already reserved slots.
        if !self.budget.admit_cell() || !self.charge_work(2) {
            return;
        }
        if self.storage.cells.capacity() < new_len {
            let old_capacity = self.storage.cells.capacity();
            let target = match self.capacity_target(old_capacity, new_len) {
                Some(target) => target,
                None => return,
            };
            let additional = match target.checked_sub(self.storage.cells.len()) {
                Some(additional) => additional,
                None => {
                    self.capacity_overflow();
                    return;
                }
            };
            // Capacity delta/spares, old-plus-new peak, reserve request, and old
            // layout relocation are all prepaid before allocation or link writes.
            if !self.budget.prepay_growth(true, old_capacity, target) {
                return;
            }
            if self.storage.cells.try_reserve_exact(additional).is_err() {
                self.budget.allocation_failed();
                return;
            }
            if !self.budget.grew(true)
                || !self.budget.finish_growth(true, old_capacity, self.storage.cells.capacity())
            {
                return;
            }
        }
        self.storage.cells.push(Cell { x, area, cover, next: cell_index });
        if let Some(index) = last_index {
            self.storage.cells[index].next = new_index;
        } else {
            self.storage.indices[yindex] = new_index;
        }
        self.budget.observe_live(new_len, self.storage.indices.len());
    }

    #[inline(always)]
    fn charge_work(&self, accesses: usize) -> bool {
        self.budget.charge_work(accesses)
    }

    #[inline(always)]
    fn failed(&self) -> bool {
        self.budget.error().is_some()
    }

    #[inline(always)]
    fn bounded(&self) -> bool {
        true
    }

    fn refuse_target(&self, requested: u64, limit: u64) {
        self.budget.refuse(FillRasterLimitKind::Target, requested, limit);
    }
}

const MAX_CELLS: usize = 1024;
const MAX_BAND: usize = 512;

pub struct AdaptiveStorage {
    min: FixedPoint,
    max: FixedPoint,
    height: usize,
    cell_count: usize,
    cells: [Cell; MAX_CELLS],
    heap_cells: Vec<Cell>,
    indices: [i32; MAX_BAND],
    heap_indices: Vec<i32>,
}

impl AdaptiveStorage {
    #[allow(clippy::uninit_assumed_init, invalid_value)]
    pub fn new() -> Self {
        Self {
            min: FixedPoint::default(),
            max: FixedPoint::default(),
            height: 0,
            cell_count: 0,
            cells: [Default::default(); MAX_CELLS],
            heap_cells: Vec::new(),
            indices: [Default::default(); MAX_BAND],
            heap_indices: Vec::new(),
        }
    }
}

impl RasterStorage for AdaptiveStorage {
    fn reset(&mut self, min: FixedPoint, max: FixedPoint) {
        self.min = min;
        self.max = max;
        self.height = (max.y - min.y) as usize;
        self.cell_count = 0;
        self.heap_cells.clear();
        self.heap_indices.clear();
        if self.height > MAX_BAND {
            self.heap_indices.resize((max.y - min.y) as usize, -1);
        } else {
            for i in 0..self.height {
                self.indices[i] = -1;
            }
        }
    }

    fn cells(&self) -> &[Cell] {
        if self.cell_count > MAX_CELLS {
            &self.heap_cells
        } else {
            &self.cells
        }
    }

    fn indices(&self) -> &[i32] {
        if self.height > MAX_BAND {
            &self.heap_indices
        } else {
            &self.indices[..self.height]
        }
    }

    #[inline(always)]
    #[allow(clippy::comparison_chain)]
    fn set(&mut self, x: i32, y: i32, area: i32, cover: i32) {
        let yindex = (y - self.min.y) as usize;
        let indices = if self.height > MAX_BAND {
            &mut self.heap_indices[..]
        } else {
            &mut self.indices[..]
        };
        let cells = if !self.heap_cells.is_empty() {
            &mut self.heap_cells[..]
        } else {
            &mut self.cells[..]
        };
        let mut cell_index = indices[yindex];
        let mut last_index = -1;
        while cell_index != -1 {
            let cell = &mut cells[cell_index as usize];
            if cell.x > x {
                break;
            } else if cell.x == x {
                cell.area = cell.area.wrapping_add(area);
                cell.cover = cell.cover.wrapping_add(cover);
                return;
            }
            last_index = cell_index;
            cell_index = cell.next;
        }
        let new_index = self.cell_count;
        self.cell_count += 1;
        let cell = Cell {
            x,
            area,
            cover,
            next: cell_index,
        };
        if last_index != -1 {
            cells[last_index as usize].next = new_index as i32;
        } else {
            indices[yindex] = new_index as i32;
        }
        if new_index < MAX_CELLS {
            cells[new_index] = cell;
        } else {
            if self.heap_cells.is_empty() {
                self.heap_cells.extend_from_slice(&self.cells);
            }
            self.heap_cells.push(cell);
        }
    }
}

const _MAX_DIM: u32 = i16::MAX as u32;

fn split_quad(base: &mut [FixedPoint]) {
    let mut a;
    let mut b;
    base[4].x = base[2].x;
    a = base[0].x + base[1].x;
    b = base[1].x + base[2].x;
    base[3].x = b >> 1;
    base[2].x = (a + b) >> 2;
    base[1].x = a >> 1;
    base[4].y = base[2].y;
    a = base[0].y + base[1].y;
    b = base[1].y + base[2].y;
    base[3].y = b >> 1;
    base[2].y = (a + b) >> 2;
    base[1].y = a >> 1;
}

fn split_cubic(base: &mut [FixedPoint]) {
    let mut a;
    let mut b;
    let mut c;
    base[6].x = base[3].x;
    a = base[0].x + base[1].x;
    b = base[1].x + base[2].x;
    c = base[2].x + base[3].x;
    base[5].x = c >> 1;
    c += b;
    base[4].x = c >> 2;
    base[1].x = a >> 1;
    a += b;
    base[2].x = a >> 2;
    base[3].x = (a + c) >> 3;
    base[6].y = base[3].y;
    a = base[0].y + base[1].y;
    b = base[1].y + base[2].y;
    c = base[2].y + base[3].y;
    base[5].y = c >> 1;
    c += b;
    base[4].y = c >> 2;
    base[1].y = a >> 1;
    a += b;
    base[2].y = a >> 2;
    base[3].y = (a + c) >> 3;
}

#[derive(Copy, Clone, Default)]
pub struct FixedPoint {
    pub x: i32,
    pub y: i32,
}

impl fmt::Debug for FixedPoint {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

impl FixedPoint {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    #[inline(always)]
    pub fn from_point(p: Point) -> Self {
        Self {
            x: to_fixed(p.x),
            y: to_fixed(p.y),
        }
    }
}

#[inline(always)]
fn to_fixed(v: f32) -> i32 {
    unsafe { (v * 256.).to_int_unchecked() }
}

const PIXEL_BITS: i32 = 8;
const ONE_PIXEL: i32 = 1 << PIXEL_BITS;

#[inline(always)]
fn trunc(x: i32) -> i32 {
    x >> PIXEL_BITS
}

#[inline(always)]
fn fract(x: i32) -> i32 {
    x & (ONE_PIXEL - 1)
}
