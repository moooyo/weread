//! Requested layout storage and allocation work for one staged operation.

use std::fmt::{self, Write};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

pub const DEFAULT_LAYOUT_METADATA_BYTES: usize = 64 * 1024 * 1024;
pub const DEFAULT_LAYOUT_ALLOCATION_WORK: usize = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutLimits {
    pub metadata_bytes: usize,
    pub allocation_work: usize,
}

impl Default for LayoutLimits {
    fn default() -> Self {
        Self {
            metadata_bytes: DEFAULT_LAYOUT_METADATA_BYTES,
            allocation_work: DEFAULT_LAYOUT_ALLOCATION_WORK,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutLimitKind {
    Metadata,
    AllocationWork,
    Allocation,
    Anchors,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("layout {kind:?} exceeds its allowance (limit {limit})")]
pub struct LayoutLimitError {
    pub kind: LayoutLimitKind,
    pub used: usize,
    pub requested: usize,
    pub limit: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutUsage {
    pub live_metadata: usize,
    pub peak_metadata: usize,
    pub allocation_work: usize,
    pub allocations: usize,
    pub vector_pushes: usize,
    pub text_copies: usize,
    pub rejected: usize,
    pub scan_steps: usize,
}

#[derive(Debug)]
struct State {
    limits: LayoutLimits,
    usage: LayoutUsage,
}

/// The limits describe requested storage and logical allocation work. They do
/// not describe allocator overhead, font execution time, or process RSS.
#[derive(Debug)]
pub struct LayoutBudget {
    state: Arc<Mutex<State>>,
}

impl Default for LayoutBudget {
    fn default() -> Self {
        #[cfg(test)]
        let limits = TEST_LIMITS.with(std::cell::Cell::get).unwrap_or_default();
        #[cfg(not(test))]
        let limits = LayoutLimits::default();
        Self::with_limits(limits)
    }
}

impl LayoutBudget {
    pub fn with_limits(limits: LayoutLimits) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                limits: LayoutLimits {
                    metadata_bytes: limits.metadata_bytes.min(DEFAULT_LAYOUT_METADATA_BYTES),
                    allocation_work: limits.allocation_work.min(DEFAULT_LAYOUT_ALLOCATION_WORK),
                },
                usage: LayoutUsage::default(),
            })),
        }
    }

    pub fn usage(&self) -> LayoutUsage {
        locked(&self.state).usage
    }

    pub(crate) fn lease(&self) -> LayoutLease {
        LayoutLease {
            state: self.state.clone(),
            bytes: 0,
        }
    }

    pub(crate) fn share(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }

    pub(crate) fn hold_lease(&self, lease: &LayoutLease) -> Result<LayoutLease, LayoutLimitError> {
        if Arc::ptr_eq(&self.state, &lease.state) {
            Ok(self.lease())
        } else {
            self.hold(lease.bytes())
        }
    }

    pub(crate) fn adopt_storage(
        &self,
        lease: LayoutLease,
    ) -> Result<LayoutLease, LayoutLimitError> {
        if Arc::ptr_eq(&self.state, &lease.state) {
            Ok(lease)
        } else {
            // Ownership moves without copying data. A different operation must
            // still admit all of that live storage before taking ownership.
            let rebound = self.hold(lease.bytes())?;
            drop(lease);
            Ok(rebound)
        }
    }

    /// Borrowed old output stays live while the replacement is being prepared.
    /// It consumes live storage headroom without recharging earlier work.
    pub(crate) fn hold(&self, bytes: usize) -> Result<LayoutLease, LayoutLimitError> {
        let mut lease = self.lease();
        lease.admit(bytes, 0)?;
        Ok(lease)
    }

    pub(crate) fn copy_storage(&self, bytes: usize) -> Result<LayoutLease, LayoutLimitError> {
        let mut lease = self.lease();
        lease.admit(bytes, bytes)?;
        Ok(lease)
    }

    pub(crate) fn work(&self, bytes: usize) -> Result<(), LayoutLimitError> {
        self.lease().admit(0, bytes)
    }

    /// One actual scalar inspection or measurement uses four logical work
    /// bytes. It shares the existing operation ceiling with allocation work.
    pub(crate) fn scan_step(&self) -> Result<(), LayoutLimitError> {
        self.work(4)?;
        let mut state = locked(&self.state);
        state.usage.scan_steps += 1;
        observe(state.usage);
        Ok(())
    }

    pub(crate) fn preflight(&self, bytes: usize, work: usize) -> Result<(), LayoutLimitError> {
        let mut state = locked(&self.state);
        let result = checked_allowance(
            state.usage.live_metadata,
            bytes,
            state.limits.metadata_bytes,
            LayoutLimitKind::Metadata,
        )
        .and_then(|_| {
            checked_allowance(
                state.usage.allocation_work,
                work,
                state.limits.allocation_work,
                LayoutLimitKind::AllocationWork,
            )
        });
        if let Err(error) = result {
            state.usage.rejected += 1;
            observe(state.usage);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn copy(&self, text: &str) -> Result<(String, LayoutLease), LayoutLimitError> {
        let mut result = LayoutString::with_capacity(self, text.len())?;
        result.value.push_str(text);
        let mut state = locked(&self.state);
        state.usage.text_copies += 1;
        observe(state.usage);
        drop(state);
        Ok(result.into_parts())
    }

    pub(crate) fn format(
        &self,
        args: fmt::Arguments<'_>,
    ) -> Result<(String, LayoutLease), LayoutLimitError> {
        let mut result = LayoutString::new(self);
        if result.write_fmt(args).is_err() {
            return Err(result
                .failure
                .expect("A layout formatter retains its refusal"));
        }
        Ok(result.into_parts())
    }
}

fn locked(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Debug)]
pub(crate) struct LayoutLease {
    state: Arc<Mutex<State>>,
    bytes: usize,
}

impl LayoutLease {
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn budget(&self) -> LayoutBudget {
        LayoutBudget {
            state: self.state.clone(),
        }
    }

    fn admit(&mut self, bytes: usize, work: usize) -> Result<(), LayoutLimitError> {
        let mut state = locked(&self.state);
        let live = checked_allowance(
            state.usage.live_metadata,
            bytes,
            state.limits.metadata_bytes,
            LayoutLimitKind::Metadata,
        );
        let spent = checked_allowance(
            state.usage.allocation_work,
            work,
            state.limits.allocation_work,
            LayoutLimitKind::AllocationWork,
        );
        let (live, spent) = match (live, spent) {
            (Ok(live), Ok(spent)) => (live, spent),
            (Err(error), _) | (_, Err(error)) => {
                state.usage.rejected += 1;
                observe(state.usage);
                return Err(error);
            }
        };
        self.bytes = self.bytes.checked_add(bytes).ok_or(LayoutLimitError {
            kind: LayoutLimitKind::Metadata,
            used: self.bytes,
            requested: bytes,
            limit: state.limits.metadata_bytes,
        })?;
        state.usage.live_metadata = live;
        state.usage.peak_metadata = state.usage.peak_metadata.max(live);
        state.usage.allocation_work = spent;
        observe(state.usage);
        Ok(())
    }

    pub(crate) fn merge(&mut self, mut other: Self) {
        assert!(Arc::ptr_eq(&self.state, &other.state));
        // Both leases were admitted by the same bounded live counter.
        self.bytes += other.bytes;
        other.bytes = 0;
    }

    pub(crate) fn release(&mut self, bytes: usize) {
        debug_assert!(bytes <= self.bytes);
        let bytes = bytes.min(self.bytes);
        self.bytes -= bytes;
        let mut state = locked(&self.state);
        state.usage.live_metadata -= bytes;
        observe(state.usage);
    }

    pub(crate) fn reserve_vec<T>(
        &mut self,
        values: &mut Vec<T>,
        requested_capacity: &mut usize,
        required: usize,
    ) -> Result<(), LayoutLimitError> {
        if required <= *requested_capacity {
            return Ok(());
        }
        let capacity = next_capacity(*requested_capacity, required)?;
        let new_bytes = capacity_bytes::<T>(capacity)?;
        let old_bytes = capacity_bytes::<T>(*requested_capacity)?;
        let work = new_bytes.checked_add(old_bytes).ok_or(LayoutLimitError {
            kind: LayoutLimitKind::AllocationWork,
            used: 0,
            requested: usize::MAX,
            limit: DEFAULT_LAYOUT_ALLOCATION_WORK,
        })?;
        self.admit(new_bytes, work)?;
        if values.try_reserve_exact(capacity - values.len()).is_err() {
            self.release(new_bytes);
            return Err(LayoutLimitError {
                kind: LayoutLimitKind::Allocation,
                used: old_bytes,
                requested: new_bytes,
                limit: DEFAULT_LAYOUT_METADATA_BYTES,
            });
        }
        self.release(old_bytes);
        *requested_capacity = capacity;
        let mut state = locked(&self.state);
        state.usage.allocations += 1;
        observe(state.usage);
        Ok(())
    }
}

/// Numeric accounting only. A registry never owns a model, its data, or an App.
#[derive(Clone, Default)]
pub(crate) struct LayoutModelRegistry {
    live: Arc<AtomicUsize>,
}

impl LayoutModelRegistry {
    pub(crate) fn live_metadata(&self) -> usize {
        self.live.load(Ordering::Relaxed)
    }

    pub(crate) fn register(
        &self,
        bytes: usize,
    ) -> Result<LayoutModelRegistration, LayoutLimitError> {
        let used = self.live_metadata();
        self.live
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |live| {
                live.checked_add(bytes)
                    .filter(|next| *next <= DEFAULT_LAYOUT_METADATA_BYTES)
            })
            .map_err(|_| LayoutLimitError {
                kind: LayoutLimitKind::Metadata,
                used,
                requested: bytes,
                limit: DEFAULT_LAYOUT_METADATA_BYTES,
            })?;
        Ok(LayoutModelRegistration {
            live: self.live.clone(),
            bytes,
        })
    }
}

pub(crate) struct LayoutModelRegistration {
    live: Arc<AtomicUsize>,
    bytes: usize,
}

impl Drop for LayoutModelRegistration {
    fn drop(&mut self) {
        self.live.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

impl Drop for LayoutLease {
    fn drop(&mut self) {
        self.release(self.bytes);
    }
}

fn checked_allowance(
    used: usize,
    requested: usize,
    limit: usize,
    kind: LayoutLimitKind,
) -> Result<usize, LayoutLimitError> {
    used.checked_add(requested)
        .filter(|next| *next <= limit)
        .ok_or(LayoutLimitError {
            kind,
            used,
            requested,
            limit,
        })
}

fn capacity_bytes<T>(capacity: usize) -> Result<usize, LayoutLimitError> {
    capacity
        .checked_mul(std::mem::size_of::<T>())
        .ok_or(LayoutLimitError {
            kind: LayoutLimitKind::Metadata,
            used: 0,
            requested: usize::MAX,
            limit: DEFAULT_LAYOUT_METADATA_BYTES,
        })
}

fn next_capacity(current: usize, required: usize) -> Result<usize, LayoutLimitError> {
    current
        .checked_mul(2)
        .map(|doubled| doubled.max(4).max(required))
        .ok_or(LayoutLimitError {
            kind: LayoutLimitKind::Metadata,
            used: current,
            requested: required,
            limit: DEFAULT_LAYOUT_METADATA_BYTES,
        })
}

/// Each growth prepays the entire new buffer while the old buffer and every
/// payload are still live. `try_reserve_exact` uses that requested capacity;
/// allocator rounding is outside this logical storage policy.
pub(crate) struct LayoutVec<T> {
    values: Vec<T>,
    requested_capacity: usize,
    lease: LayoutLease,
}

impl<T> LayoutVec<T> {
    pub(crate) fn new(budget: &LayoutBudget) -> Self {
        Self {
            values: Vec::new(),
            requested_capacity: 0,
            lease: budget.lease(),
        }
    }

    pub(crate) fn with_capacity(
        budget: &LayoutBudget,
        capacity: usize,
    ) -> Result<Self, LayoutLimitError> {
        let mut result = Self::new(budget);
        result.grow(capacity)?;
        Ok(result)
    }

    pub(crate) fn reserve_one(&mut self) -> Result<(), LayoutLimitError> {
        let required = self.values.len().checked_add(1).ok_or(LayoutLimitError {
            kind: LayoutLimitKind::Metadata,
            used: self.values.len(),
            requested: 1,
            limit: DEFAULT_LAYOUT_METADATA_BYTES,
        })?;
        if required > self.requested_capacity {
            self.grow(next_capacity(self.requested_capacity, required)?)?;
        }
        Ok(())
    }

    fn grow(&mut self, capacity: usize) -> Result<(), LayoutLimitError> {
        if capacity <= self.requested_capacity {
            return Ok(());
        }
        let new_bytes = capacity_bytes::<T>(capacity)?;
        let old_bytes = self.buffer_bytes();
        let work = new_bytes.checked_add(old_bytes).ok_or(LayoutLimitError {
            kind: LayoutLimitKind::AllocationWork,
            used: 0,
            requested: usize::MAX,
            limit: DEFAULT_LAYOUT_ALLOCATION_WORK,
        })?;
        self.lease.admit(new_bytes, work)?;
        if self
            .values
            .try_reserve_exact(capacity - self.values.len())
            .is_err()
        {
            self.lease.release(new_bytes);
            return Err(LayoutLimitError {
                kind: LayoutLimitKind::Allocation,
                used: old_bytes,
                requested: new_bytes,
                limit: DEFAULT_LAYOUT_METADATA_BYTES,
            });
        }
        self.lease.release(old_bytes);
        self.requested_capacity = capacity;
        let mut state = locked(&self.lease.state);
        state.usage.allocations += 1;
        observe(state.usage);
        Ok(())
    }

    pub(crate) fn push(&mut self, value: T) -> Result<(), LayoutLimitError> {
        self.reserve_one()?;
        self.values.push(value);
        let mut state = locked(&self.lease.state);
        state.usage.vector_pushes += 1;
        observe(state.usage);
        Ok(())
    }

    pub(crate) fn push_owned(
        &mut self,
        value: T,
        owner: LayoutLease,
    ) -> Result<(), LayoutLimitError> {
        self.push(value)?;
        self.lease.merge(owner);
        Ok(())
    }

    pub(crate) fn extend_reserved(
        &mut self,
        values: impl IntoIterator<Item = T>,
    ) -> Result<(), LayoutLimitError> {
        let start = self.values.len();
        for value in values {
            if self.values.len() == self.requested_capacity {
                return Err(LayoutLimitError {
                    kind: LayoutLimitKind::Metadata,
                    used: self.values.len(),
                    requested: 1,
                    limit: self.requested_capacity,
                });
            }
            self.values.push(value);
        }
        let mut state = locked(&self.lease.state);
        state.usage.vector_pushes += self.values.len() - start;
        observe(state.usage);
        Ok(())
    }

    pub(crate) fn adopt(&mut self, owner: LayoutLease) {
        self.lease.merge(owner);
    }

    pub(crate) fn buffer_bytes(&self) -> usize {
        self.requested_capacity * std::mem::size_of::<T>()
    }

    pub(crate) fn into_parts(self) -> (Vec<T>, LayoutLease) {
        (self.values, self.lease)
    }
}

impl<T> Deref for LayoutVec<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

impl<T> DerefMut for LayoutVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.values
    }
}

pub(crate) struct LayoutString {
    value: String,
    requested_capacity: usize,
    lease: LayoutLease,
    failure: Option<LayoutLimitError>,
}

impl LayoutString {
    pub(crate) fn new(budget: &LayoutBudget) -> Self {
        Self {
            value: String::new(),
            requested_capacity: 0,
            lease: budget.lease(),
            failure: None,
        }
    }

    pub(crate) fn with_capacity(
        budget: &LayoutBudget,
        capacity: usize,
    ) -> Result<Self, LayoutLimitError> {
        let mut result = Self::new(budget);
        result.grow(capacity)?;
        Ok(result)
    }

    fn grow(&mut self, capacity: usize) -> Result<(), LayoutLimitError> {
        if capacity <= self.requested_capacity {
            return Ok(());
        }
        let old = self.requested_capacity;
        self.lease.admit(
            capacity,
            capacity.checked_add(old).ok_or(LayoutLimitError {
                kind: LayoutLimitKind::AllocationWork,
                used: old,
                requested: capacity,
                limit: DEFAULT_LAYOUT_ALLOCATION_WORK,
            })?,
        )?;
        if self
            .value
            .try_reserve_exact(capacity - self.value.len())
            .is_err()
        {
            self.lease.release(capacity);
            return Err(LayoutLimitError {
                kind: LayoutLimitKind::Allocation,
                used: old,
                requested: capacity,
                limit: DEFAULT_LAYOUT_METADATA_BYTES,
            });
        }
        self.lease.release(old);
        self.requested_capacity = capacity;
        let mut state = locked(&self.lease.state);
        state.usage.allocations += 1;
        observe(state.usage);
        Ok(())
    }

    pub(crate) fn push(&mut self, character: char) -> Result<(), LayoutLimitError> {
        let mut utf8 = [0; 4];
        self.append(character.encode_utf8(&mut utf8))
    }

    pub(crate) fn append(&mut self, text: &str) -> Result<(), LayoutLimitError> {
        let required = self
            .value
            .len()
            .checked_add(text.len())
            .ok_or(LayoutLimitError {
                kind: LayoutLimitKind::Metadata,
                used: self.value.len(),
                requested: text.len(),
                limit: DEFAULT_LAYOUT_METADATA_BYTES,
            })?;
        if required > self.requested_capacity {
            self.grow(next_capacity(self.requested_capacity, required)?)?;
        }
        self.value.push_str(text);
        Ok(())
    }

    pub(crate) fn clear(&mut self) {
        self.value.clear();
    }

    pub(crate) fn into_parts(self) -> (String, LayoutLease) {
        (self.value, self.lease)
    }
}

impl Deref for LayoutString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl Write for LayoutString {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.append(text).map_err(|error| {
            self.failure = Some(error);
            fmt::Error
        })
    }
}

/// An output and the sealed ownership of all its requested layout buffers.
#[derive(Debug)]
pub struct AdmittedLayout<T> {
    value: T,
    lease: LayoutLease,
    buffer_bytes: usize,
}

impl<T> AdmittedLayout<T> {
    pub(crate) fn new(value: T, lease: LayoutLease) -> Self {
        Self {
            value,
            lease,
            buffer_bytes: 0,
        }
    }

    pub fn requested_metadata(&self) -> usize {
        self.lease.bytes()
    }

    pub(crate) fn into_parts(self) -> (T, LayoutLease) {
        (self.value, self.lease)
    }
}

impl<T> AdmittedLayout<Vec<T>> {
    pub(crate) fn from_vec(value: Vec<T>, lease: LayoutLease, buffer_bytes: usize) -> Self {
        Self {
            value,
            lease,
            buffer_bytes,
        }
    }

    pub(crate) fn into_vec_parts(self) -> (Vec<T>, LayoutLease, usize) {
        (self.value, self.lease, self.buffer_bytes)
    }
}

impl<T: Default> Default for AdmittedLayout<T> {
    fn default() -> Self {
        Self::new(T::default(), LayoutBudget::default().lease())
    }
}

impl<T> Deref for AdmittedLayout<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> DerefMut for AdmittedLayout<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

#[cfg(test)]
std::thread_local! {
    static TEST_LIMITS: std::cell::Cell<Option<LayoutLimits>> = const {
        std::cell::Cell::new(None)
    };
    static LAST_USAGE: std::cell::Cell<LayoutUsage> = const {
        std::cell::Cell::new(LayoutUsage {
            live_metadata: 0, peak_metadata: 0, allocation_work: 0,
            allocations: 0, vector_pushes: 0, text_copies: 0, rejected: 0, scan_steps: 0,
        })
    };
}

#[cfg(test)]
pub(crate) fn with_layout_test_limits<T>(limits: LayoutLimits, operation: impl FnOnce() -> T) -> T {
    struct Restore(Option<LayoutLimits>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_LIMITS.with(|current| current.set(self.0));
        }
    }
    let _restore = Restore(TEST_LIMITS.with(|current| current.replace(Some(limits))));
    LAST_USAGE.with(|usage| usage.set(LayoutUsage::default()));
    operation()
}

#[cfg(test)]
pub(crate) fn last_layout_usage() -> LayoutUsage {
    LAST_USAGE.with(std::cell::Cell::get)
}

fn observe(usage: LayoutUsage) {
    #[cfg(test)]
    LAST_USAGE.with(|current| current.set(usage));
    #[cfg(not(test))]
    let _ = usage;
}
