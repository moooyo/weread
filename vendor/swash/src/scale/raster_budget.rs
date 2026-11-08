//! Project-local physical raster admission, before outline target allocation.

use alloc::vec::Vec;
use core::fmt;

const MAX_DIMENSION: u32 = 2048;
const MAX_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_BUFFER_BYTES: usize = 16 * 1024 * 1024;
const MAX_WORK_BYTES: usize = 64 * 1024 * 1024;

/// Physical target allowances for one outline glyph render.
/// These are requested-buffer/work limits, not total allocator RSS or CPU time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RasterLimits {
    pub dimension: u32,
    pub pixels: u64,
    pub buffer_bytes: usize,
    pub work_bytes: usize,
}

impl Default for RasterLimits {
    fn default() -> Self {
        Self {
            dimension: MAX_DIMENSION,
            pixels: MAX_PIXELS,
            buffer_bytes: MAX_BUFFER_BYTES,
            work_bytes: MAX_WORK_BYTES,
        }
    }
}

impl RasterLimits {
    fn constrained(self) -> Self {
        Self {
            dimension: self.dimension.min(MAX_DIMENSION),
            pixels: self.pixels.min(MAX_PIXELS),
            buffer_bytes: self.buffer_bytes.min(MAX_BUFFER_BYTES),
            work_bytes: self.work_bytes.min(MAX_WORK_BYTES),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterLimitKind {
    Dimensions,
    Pixels,
    Stride,
    Bytes,
    Work,
    Allocation,
    FillCells,
    FillStorage,
    FillWork,
    FillAllocation,
    FillCoordinates,
    FillTarget,
}

/// A resource refusal never authorizes a partially rendered glyph image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RasterLimitError {
    pub kind: RasterLimitKind,
    pub requested: u64,
    pub limit: u64,
}

impl fmt::Display for RasterLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "glyph raster {:?} exceeds its allowance (requested {}, limit {})",
            self.kind, self.requested, self.limit,
        )
    }
}

impl core::error::Error for RasterLimitError {}

/// Operation-local observations of the actual allocation boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RasterUsage {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub work_bytes: usize,
    pub resize_calls: usize,
    pub growth_allocations: usize,
    pub maximum_buffer_bytes: usize,
    pub rejected: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct Layout {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub bytes: usize,
}

/// Only admission can create a prepaid allocation/work reservation.
pub(crate) struct Reservation {
    remaining: usize,
}

pub(crate) struct Budget {
    pub limits: RasterLimits,
    pub usage: RasterUsage,
    pub error: Option<RasterLimitError>,
}

impl Default for Budget {
    fn default() -> Self {
        Self::new(RasterLimits::default())
    }
}

impl Budget {
    pub fn new(limits: RasterLimits) -> Self {
        Self {
            limits: limits.constrained(),
            usage: RasterUsage::default(),
            error: None,
        }
    }

    pub fn fail(&mut self, kind: RasterLimitKind, requested: u64, limit: u64) -> RasterLimitError {
        let error = RasterLimitError { kind, requested, limit };
        self.usage.rejected = self.usage.rejected.saturating_add(1);
        if self.error.is_none() {
            self.error = Some(error);
        }
        error
    }

    #[cfg(feature = "render")]
    pub fn fill_refusal(&mut self, error: zeno::FillRasterLimitError) {
        let kind = match error.kind {
            zeno::FillRasterLimitKind::Cells => RasterLimitKind::FillCells,
            zeno::FillRasterLimitKind::Storage => RasterLimitKind::FillStorage,
            zeno::FillRasterLimitKind::Work => RasterLimitKind::FillWork,
            zeno::FillRasterLimitKind::Allocation => RasterLimitKind::FillAllocation,
            zeno::FillRasterLimitKind::Coordinates => RasterLimitKind::FillCoordinates,
            zeno::FillRasterLimitKind::Target => RasterLimitKind::FillTarget,
        };
        self.fail(kind, error.requested, error.limit);
    }

    pub fn dimensions(&mut self, width: u32, height: u32) -> Result<(), RasterLimitError> {
        if width > self.limits.dimension || height > self.limits.dimension {
            return Err(self.fail(
                RasterLimitKind::Dimensions,
                u64::from(width.max(height)),
                u64::from(self.limits.dimension),
            ));
        }
        let pixels = u64::from(width).checked_mul(u64::from(height)).ok_or_else(|| {
            self.fail(RasterLimitKind::Pixels, u64::MAX, self.limits.pixels)
        })?;
        if pixels > self.limits.pixels {
            return Err(self.fail(RasterLimitKind::Pixels, pixels, self.limits.pixels));
        }
        Ok(())
    }

    pub fn bytes(&mut self, bytes: usize) -> Result<(), RasterLimitError> {
        if bytes > self.limits.buffer_bytes {
            return Err(self.fail(
                RasterLimitKind::Bytes,
                u64::try_from(bytes).unwrap_or(u64::MAX),
                u64::try_from(self.limits.buffer_bytes).unwrap_or(u64::MAX),
            ));
        }
        Ok(())
    }

    pub fn layout(
        &mut self,
        width: u32,
        height: u32,
        channels: u32,
    ) -> Result<Layout, RasterLimitError> {
        self.usage.width = width;
        self.usage.height = height;
        self.dimensions(width, height)?;
        if !(1..=4).contains(&channels) {
            return Err(self.fail(RasterLimitKind::Stride, u64::from(channels), 4));
        }
        let stride = u64::from(width).checked_mul(u64::from(channels)).ok_or_else(|| {
            self.fail(RasterLimitKind::Stride, u64::MAX, MAX_BUFFER_BYTES as u64)
        })?;
        let bytes = stride.checked_mul(u64::from(height)).ok_or_else(|| {
            self.fail(RasterLimitKind::Bytes, u64::MAX, MAX_BUFFER_BYTES as u64)
        })?;
        let stride = usize::try_from(stride).map_err(|_| {
            self.fail(RasterLimitKind::Stride, u64::MAX, MAX_BUFFER_BYTES as u64)
        })?;
        let bytes = usize::try_from(bytes).map_err(|_| {
            self.fail(RasterLimitKind::Bytes, u64::MAX, MAX_BUFFER_BYTES as u64)
        })?;
        self.usage.stride = stride;
        self.bytes(bytes)?;
        Ok(Layout { width, height, stride, bytes })
    }

    pub fn prepay(&mut self, bytes: usize) -> Result<Reservation, RasterLimitError> {
        let requested = self.usage.work_bytes.checked_add(bytes).ok_or_else(|| {
            self.fail(RasterLimitKind::Work, u64::MAX, self.limits.work_bytes as u64)
        })?;
        if requested > self.limits.work_bytes {
            return Err(self.fail(
                RasterLimitKind::Work,
                requested as u64,
                self.limits.work_bytes as u64,
            ));
        }
        // Failed or overwritten sources never refund an admitted allocation.
        self.usage.work_bytes = requested;
        Ok(Reservation { remaining: bytes })
    }

    pub fn resize(
        &mut self,
        buffer: &mut Vec<u8>,
        bytes: usize,
        reservation: &mut Reservation,
    ) -> Result<(), RasterLimitError> {
        self.bytes(bytes)?;
        reservation.remaining = reservation.remaining.checked_sub(bytes).ok_or_else(|| {
            self.fail(RasterLimitKind::Work, bytes as u64, reservation.remaining as u64)
        })?;
        if buffer.capacity() < bytes {
            buffer.try_reserve_exact(bytes.saturating_sub(buffer.len())).map_err(|_| {
                self.fail(RasterLimitKind::Allocation, bytes as u64, self.limits.buffer_bytes as u64)
            })?;
            self.usage.growth_allocations = self.usage.growth_allocations.saturating_add(1);
        }
        self.usage.resize_calls = self.usage.resize_calls.saturating_add(1);
        self.usage.maximum_buffer_bytes = self.usage.maximum_buffer_bytes.max(bytes);
        buffer.resize(bytes, 0);
        Ok(())
    }
}
