//! Opt-in allowances for the fill rasterizer's real storage and access loops.

use core::cell::Cell as StateCell;
use core::fmt;
use core::mem::size_of;

const MAX_CELLS: usize = 1024 * 1024;
const MAX_STORAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_WORK_ACCESSES: usize = 64 * 1024 * 1024;

/// Cumulative requested capacity reservations and logical work for an operation.
/// Storage also admits the requested old-plus-new buffer peak before growth.
/// These field-byte allowances do not describe allocator RSS or elapsed time.
/// Work counts command fetch/dispatch, raster transition/subdivision steps,
/// row-head/cell accesses, output samples, and allocator request attempts. It
/// also prepays two accesses per potentially relocated byte, for read and write.
/// A cell visit counts one logical access when examining several cell fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FillRasterLimits {
    pub cells: usize,
    /// Both cumulative capacity reservations and requested growth peaks must fit.
    pub storage_bytes: usize,
    /// Raster accesses, reserve attempts, and potential copy read/write units.
    pub work_accesses: usize,
}

impl Default for FillRasterLimits {
    fn default() -> Self {
        Self {
            cells: MAX_CELLS,
            storage_bytes: MAX_STORAGE_BYTES,
            work_accesses: MAX_WORK_ACCESSES,
        }
    }
}

impl FillRasterLimits {
    fn constrained(self) -> Self {
        Self {
            cells: self.cells.min(MAX_CELLS),
            storage_bytes: self.storage_bytes.min(MAX_STORAGE_BYTES),
            work_accesses: self.work_accesses.min(MAX_WORK_ACCESSES),
        }
    }

    /// Prices the actual private cell layout on the compilation target.
    pub const fn cell_bytes() -> usize {
        size_of::<super::raster::Cell>()
    }

    /// Prices the actual signed row-head index layout.
    pub const fn index_bytes() -> usize {
        size_of::<i32>()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRasterLimitKind {
    Cells,
    Storage,
    Work,
    Allocation,
    Coordinates,
    Target,
}

/// A rejected raster attempt cannot authorize a partially rendered image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FillRasterLimitError {
    pub kind: FillRasterLimitKind,
    pub requested: u64,
    pub limit: u64,
}

impl fmt::Display for FillRasterLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "fill raster {:?} exceeds its allowance (requested {}, limit {})",
            self.kind, self.requested, self.limit,
        )
    }
}

impl core::error::Error for FillRasterLimitError {}

/// Observations of admitted attempts at the actual raster access boundaries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FillRasterUsage {
    /// Admitted new-cell requests, including prepaid attempts that later fail.
    pub cells: usize,
    /// Admitted row-head initialization requests across every reset.
    pub indices: usize,
    /// Cumulative reserved Cell slots, including spare slots and reset attempts.
    pub cell_slots: usize,
    /// Cumulative reserved row-head slots, including spare slots and resets.
    pub index_slots: usize,
    /// Sum of admitted slot reservations priced with the actual element layouts.
    pub storage_bytes: usize,
    pub work_accesses: usize,
    /// Admitted upper bounds on old allocation bytes that a reserve may move.
    pub potential_relocation_bytes: usize,
    /// Largest admitted requested resident capacity span for both vectors.
    pub maximum_requested_storage_bytes: usize,
    /// Largest admitted requested old-plus-new span during a growth attempt.
    pub maximum_peak_storage_bytes: usize,
    pub maximum_live_cells: usize,
    pub maximum_live_indices: usize,
    pub cell_reserve_attempts: usize,
    pub index_reserve_attempts: usize,
    pub cell_growths: usize,
    pub index_growths: usize,
    pub rejected: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct State {
    usage: FillRasterUsage,
    error: Option<FillRasterLimitError>,
    cell_capacity: usize,
    index_capacity: usize,
}

/// An operation-local budget reused across every raster pass and attempt.
/// Construct a new budget to begin a new operation. Clearing scratch storage,
/// allocation failure, and denied or overwritten attempts never refund it.
#[derive(Debug)]
pub struct FillRasterBudget {
    limits: FillRasterLimits,
    state: StateCell<State>,
}

impl Default for FillRasterBudget {
    fn default() -> Self {
        Self::new(FillRasterLimits::default())
    }
}

impl FillRasterBudget {
    pub fn new(limits: FillRasterLimits) -> Self {
        Self {
            limits: limits.constrained(),
            state: StateCell::new(State::default()),
        }
    }

    pub fn limits(&self) -> FillRasterLimits {
        self.limits
    }

    pub fn usage(&self) -> FillRasterUsage {
        self.state.get().usage
    }

    pub fn error(&self) -> Option<FillRasterLimitError> {
        self.state.get().error
    }

    pub(crate) fn refuse(
        &self,
        kind: FillRasterLimitKind,
        requested: u64,
        limit: u64,
    ) -> FillRasterLimitError {
        let mut state = self.state.get();
        if let Some(error) = state.error {
            return error;
        }
        let error = FillRasterLimitError { kind, requested, limit };
        state.error = Some(error);
        state.usage.rejected = 1;
        self.state.set(state);
        error
    }

    pub(crate) fn charge_work(&self, accesses: usize) -> bool {
        let mut state = self.state.get();
        if state.error.is_some() {
            return false;
        }
        let requested = match state.usage.work_accesses.checked_add(accesses) {
            Some(requested) => requested,
            None => {
                self.refuse(FillRasterLimitKind::Work, u64::MAX, as_u64(self.limits.work_accesses));
                return false;
            }
        };
        if requested > self.limits.work_accesses {
            self.refuse(
                FillRasterLimitKind::Work,
                as_u64(requested),
                as_u64(self.limits.work_accesses),
            );
            return false;
        }
        state.usage.work_accesses = requested;
        self.state.set(state);
        true
    }

    pub(crate) fn begin_pass(
        &self,
        cell_capacity: usize,
        index_capacity: usize,
        count: usize,
    ) -> bool {
        let mut state = self.state.get();
        if state.error.is_some() {
            return false;
        }
        let indices = state.usage.indices.checked_add(count);
        let cell_slots = state.usage.cell_slots.checked_add(cell_capacity);
        let index_slots = state.usage.index_slots.checked_add(index_capacity);
        let resident = cell_capacity.checked_mul(FillRasterLimits::cell_bytes())
            .and_then(|cells| index_capacity.checked_mul(FillRasterLimits::index_bytes())
                .and_then(|indices| cells.checked_add(indices)));
        let bytes = resident.and_then(|bytes| state.usage.storage_bytes.checked_add(bytes));
        let (indices, cell_slots, index_slots, resident, bytes) = match (indices, cell_slots, index_slots, resident, bytes) {
            (Some(indices), Some(cell_slots), Some(index_slots), Some(resident), Some(bytes)) => {
                (indices, cell_slots, index_slots, resident, bytes)
            }
            _ => {
                self.refuse(FillRasterLimitKind::Storage, u64::MAX, as_u64(self.limits.storage_bytes));
                return false;
            }
        };
        if bytes > self.limits.storage_bytes {
            self.refuse(FillRasterLimitKind::Storage, as_u64(bytes), as_u64(self.limits.storage_bytes));
            return false;
        }
        state.usage.indices = indices;
        state.usage.cell_slots = cell_slots;
        state.usage.index_slots = index_slots;
        state.usage.storage_bytes = bytes;
        state.usage.maximum_requested_storage_bytes = state.usage.maximum_requested_storage_bytes.max(resident);
        state.cell_capacity = cell_capacity;
        state.index_capacity = index_capacity;
        self.state.set(state);
        true
    }

    pub(crate) fn admit_cell(&self) -> bool {
        let mut state = self.state.get();
        if state.error.is_some() {
            return false;
        }
        let cells = match state.usage.cells.checked_add(1) {
            Some(cells) => cells,
            None => {
                self.refuse(FillRasterLimitKind::Cells, u64::MAX, as_u64(self.limits.cells));
                return false;
            }
        };
        if cells > self.limits.cells {
            self.refuse(FillRasterLimitKind::Cells, as_u64(cells), as_u64(self.limits.cells));
            return false;
        }
        state.usage.cells = cells;
        self.state.set(state);
        true
    }

    fn admit_capacity(&self, cells: bool, target: usize, old_capacity: usize) -> bool {
        let mut state = self.state.get();
        if state.error.is_some() {
            return false;
        }
        let (current, price) = if cells {
            (state.cell_capacity, FillRasterLimits::cell_bytes())
        } else {
            (state.index_capacity, FillRasterLimits::index_bytes())
        };
        let cell_capacity = if cells { target } else { state.cell_capacity };
        let index_capacity = if cells { state.index_capacity } else { target };
        let delta = target.checked_sub(current);
        let slots = delta.and_then(|delta| {
            if cells { state.usage.cell_slots.checked_add(delta) } else { state.usage.index_slots.checked_add(delta) }
        });
        let bytes = delta.and_then(|delta| delta.checked_mul(price))
            .and_then(|bytes| state.usage.storage_bytes.checked_add(bytes));
        let resident = cell_capacity.checked_mul(FillRasterLimits::cell_bytes())
            .and_then(|cells| index_capacity.checked_mul(FillRasterLimits::index_bytes())
                .and_then(|indices| cells.checked_add(indices)));
        // The other vector stays live while this old and new buffer may coexist.
        let peak = old_capacity.checked_mul(price)
            .and_then(|old| resident.and_then(|resident| resident.checked_add(old)));
        let (slots, bytes, resident, peak) = match (slots, bytes, resident, peak) {
            (Some(slots), Some(bytes), Some(resident), Some(peak)) => (slots, bytes, resident, peak),
            _ => {
                self.refuse(FillRasterLimitKind::Storage, u64::MAX, as_u64(self.limits.storage_bytes));
                return false;
            }
        };
        if bytes > self.limits.storage_bytes {
            self.refuse(FillRasterLimitKind::Storage, as_u64(bytes), as_u64(self.limits.storage_bytes));
            return false;
        }
        if peak > self.limits.storage_bytes {
            self.refuse(FillRasterLimitKind::Storage, as_u64(peak), as_u64(self.limits.storage_bytes));
            return false;
        }
        if cells {
            state.usage.cell_slots = slots;
        } else {
            state.usage.index_slots = slots;
        }
        state.usage.storage_bytes = bytes;
        state.usage.maximum_requested_storage_bytes = state.usage.maximum_requested_storage_bytes.max(resident);
        state.usage.maximum_peak_storage_bytes = state.usage.maximum_peak_storage_bytes.max(peak);
        state.cell_capacity = cell_capacity;
        state.index_capacity = index_capacity;
        self.state.set(state);
        true
    }

    pub(crate) fn prepay_growth(&self, cells: bool, old_capacity: usize, target: usize) -> bool {
        if !self.admit_capacity(cells, target, old_capacity) {
            return false;
        }
        let state = self.state.get();
        let price = if cells { FillRasterLimits::cell_bytes() } else { FillRasterLimits::index_bytes() };
        let moved = old_capacity.checked_mul(price);
        let relocation = moved.and_then(|bytes| state.usage.potential_relocation_bytes.checked_add(bytes));
        let work = moved.and_then(|bytes| bytes.checked_mul(2)).and_then(|accesses| accesses.checked_add(1));
        let attempts = if cells {
            state.usage.cell_reserve_attempts.checked_add(1)
        } else {
            state.usage.index_reserve_attempts.checked_add(1)
        };
        let (relocation, work, attempts) = match (relocation, work, attempts) {
            (Some(relocation), Some(work), Some(attempts)) => (relocation, work, attempts),
            _ => {
                self.refuse(FillRasterLimitKind::Work, u64::MAX, as_u64(self.limits.work_accesses));
                return false;
            }
        };
        // Count a reserve attempt and both possible copy directions before reserve.
        if !self.charge_work(work) {
            return false;
        }
        let mut state = self.state.get();
        state.usage.potential_relocation_bytes = relocation;
        if cells {
            state.usage.cell_reserve_attempts = attempts;
        } else {
            state.usage.index_reserve_attempts = attempts;
        }
        self.state.set(state);
        true
    }

    pub(crate) fn finish_growth(&self, cells: bool, old_capacity: usize, capacity: usize) -> bool {
        let state = self.state.get();
        if state.error.is_some() {
            return false;
        }
        let expected = if cells { state.cell_capacity } else { state.index_capacity };
        if capacity < expected {
            self.refuse(FillRasterLimitKind::Allocation, as_u64(expected), as_u64(capacity));
            return false;
        }
        // Account any extra capacity reported by Vec before resize or insertion.
        // The allocator request and old-span relocation were already prepaid.
        capacity == expected || self.admit_capacity(cells, capacity, old_capacity)
    }

    pub(crate) fn allocation_failed(&self) {
        self.refuse(
            FillRasterLimitKind::Allocation,
            as_u64(self.usage().storage_bytes),
            as_u64(self.limits.storage_bytes),
        );
    }

    pub(crate) fn grew(&self, cells: bool) -> bool {
        let mut state = self.state.get();
        let count = if cells { &mut state.usage.cell_growths } else { &mut state.usage.index_growths };
        match count.checked_add(1) {
            Some(requested) => *count = requested,
            None => {
                self.refuse(FillRasterLimitKind::Work, u64::MAX, as_u64(self.limits.work_accesses));
                return false;
            }
        }
        self.state.set(state);
        true
    }

    pub(crate) fn observe_live(&self, cells: usize, indices: usize) {
        let mut state = self.state.get();
        state.usage.maximum_live_cells = state.usage.maximum_live_cells.max(cells);
        state.usage.maximum_live_indices = state.usage.maximum_live_indices.max(indices);
        self.state.set(state);
    }
}

fn as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
