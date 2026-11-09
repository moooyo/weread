//! Operation-wide admission for metadata retained by central-directory parsing.
//!
//! These prices cover selected metadata and requested vector storage. They are
//! not a process-RSS, allocator-padding, or wall-clock guarantee. Failed
//! candidates keep their admitted charges; a denial is terminal for the search.

use crate::extra_fields::ExtraField;
use crate::result::{ZipError, ZipResult};
use crate::types::ZipFileData;
use std::mem::size_of;

/// Default cumulative allowance for admitted retained metadata.
pub const DEFAULT_METADATA_RETENTION_LIMIT: usize = 64 * 1024 * 1024;
/// Default cumulative number of retained timestamp/NTFS record admissions.
pub const DEFAULT_EXTRA_FIELD_RECORD_LIMIT: usize = 1024 * 1024;

/// Retention allowances for one complete constructor operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetadataRetentionLimits {
    /// Admitted metadata bytes, including temporary old-vector overlap.
    pub metadata_bytes: usize,
    /// Cumulative retained timestamp/NTFS record admissions across candidates.
    pub extra_records: usize,
}

impl Default for MetadataRetentionLimits {
    fn default() -> Self {
        Self {
            metadata_bytes: DEFAULT_METADATA_RETENTION_LIMIT,
            extra_records: DEFAULT_EXTRA_FIELD_RECORD_LIMIT,
        }
    }
}

impl MetadataRetentionLimits {
    pub(crate) fn tightened(self) -> Self {
        let maximum = Self::default();
        Self {
            metadata_bytes: self.metadata_bytes.min(maximum.metadata_bytes),
            extra_records: self.extra_records.min(maximum.extra_records),
        }
    }
}

/// Constructor accounting, including actual extra-vector reserve/push sites.
/// A rejected constructor still returns this small numeric evidence to its
/// caller; it does not publish a partial archive or retained file metadata.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetadataRetentionUsage {
    /// Admitted cumulative fees, never refunded for a failed candidate.
    pub metadata_bytes: usize,
    /// Largest quoted fee including old-vector overlap, even on refusal.
    pub peak_metadata_bytes: usize,
    /// Calls at the real retained-record admission boundary.
    pub extra_preflights: usize,
    /// Successfully admitted record intents, including later parse failures.
    pub extra_records: usize,
    /// Records actually pushed into central-entry extra vectors.
    pub extra_pushes: usize,
    /// Calls at the actual extra-vector reservation site.
    pub vector_reservations: usize,
    /// Sum of full capacities requested at the reservation site.
    pub vector_requested_slots: usize,
    /// Sum of requested capacity increments, charged before reservation.
    pub vector_capacity_delta_slots: usize,
    /// Largest old-vector buffer overlap priced before growth.
    pub maximum_old_buffer_bytes: usize,
    /// Directory attempts sharing this same retention operation.
    pub candidate_attempts: usize,
    /// Whether a retention admission or reservation was refused.
    pub denied: bool,
    /// Attempts to continue a metadata operation after its terminal refusal.
    pub operations_after_denial: usize,
}

pub(crate) struct MetadataRetentionBudget {
    limits: MetadataRetentionLimits,
    usage: MetadataRetentionUsage,
}

#[derive(Clone, Copy)]
pub(crate) struct ExtraReservation {
    new_capacity: Option<usize>,
    capacity_delta: usize,
}

impl MetadataRetentionBudget {
    pub(crate) fn new(limits: MetadataRetentionLimits) -> Self {
        Self {
            limits: limits.tightened(),
            usage: MetadataRetentionUsage::default(),
        }
    }

    pub(crate) fn usage(&self) -> MetadataRetentionUsage {
        self.usage
    }

    fn reject<T>(&mut self) -> ZipResult<T> {
        self.usage.denied = true;
        Err(ZipError::UnsupportedArchive(
            ZipError::METADATA_LIMIT_EXCEEDED,
        ))
    }

    fn active(&mut self) -> ZipResult<()> {
        if self.usage.denied {
            self.usage.operations_after_denial =
                self.usage.operations_after_denial.saturating_add(1);
            return self.reject();
        }
        Ok(())
    }

    fn add(&mut self, left: usize, right: usize) -> ZipResult<usize> {
        left.checked_add(right).ok_or_else(|| {
            self.usage.denied = true;
            ZipError::UnsupportedArchive(ZipError::METADATA_LIMIT_EXCEEDED)
        })
    }

    fn multiply(&mut self, left: usize, right: usize) -> ZipResult<usize> {
        left.checked_mul(right).ok_or_else(|| {
            self.usage.denied = true;
            ZipError::UnsupportedArchive(ZipError::METADATA_LIMIT_EXCEEDED)
        })
    }

    fn charge(&mut self, retained: usize, temporary_overlap: usize) -> ZipResult<()> {
        self.active()?;
        let next = self.add(self.usage.metadata_bytes, retained)?;
        let peak = self.add(next, temporary_overlap)?;
        self.usage.peak_metadata_bytes = self.usage.peak_metadata_bytes.max(peak);
        self.usage.maximum_old_buffer_bytes =
            self.usage.maximum_old_buffer_bytes.max(temporary_overlap);
        if peak > self.limits.metadata_bytes {
            return self.reject();
        }
        self.usage.metadata_bytes = next;
        Ok(())
    }

    pub(crate) fn candidate(&mut self) -> ZipResult<()> {
        self.active()?;
        self.usage.candidate_attempts = self.add(self.usage.candidate_attempts, 1)?;
        Ok(())
    }

    pub(crate) fn directory(&mut self, files: usize) -> ZipResult<()> {
        // The temporary file vector may grow from zero to less than twice the
        // declared count. The filename map then retains each file and its key.
        // Price both phases before either container is reserved.
        let file_slots = self.multiply(size_of::<ZipFileData>(), 3)?;
        let key_slots = self.multiply(size_of::<Box<str>>(), 2)?;
        let slots = self.add(file_slots, key_slots)?;
        let per_file = self.add(slots, 256)?;
        let requested = self.multiply(files, per_file)?;
        self.charge(requested, 0)
    }

    pub(crate) fn directory_count(&mut self, raw_files: u64) -> ZipResult<usize> {
        self.active()?;
        let files = usize::try_from(raw_files).map_err(|_| {
            self.usage.denied = true;
            ZipError::UnsupportedArchive(ZipError::METADATA_LIMIT_EXCEEDED)
        })?;
        self.directory(files)?;
        Ok(files)
    }

    pub(crate) fn file_fields(&mut self, name: u16, extra: u16, comment: u16) -> ZipResult<()> {
        // A raw filename, its worst-case CP437/lossy UTF-8 representation, and
        // the filename-map key coexist. Comments retain one decoded string;
        // the original extra bytes retain one Arc<Vec<u8>> payload.
        let names = self.multiply(usize::from(name), 7)?;
        let comments = self.multiply(usize::from(comment), 3)?;
        let arc_header = self.multiply(size_of::<usize>(), 2)?;
        let arc = self.add(arc_header, size_of::<Vec<u8>>())?;
        let text = self.add(names, comments)?;
        let fields = self.add(text, usize::from(extra))?;
        let requested = self.add(fields, arc)?;
        self.charge(requested, 0)
    }

    pub(crate) fn bytes(&mut self, bytes: usize) -> ZipResult<()> {
        self.charge(bytes, 0)
    }

    pub(crate) fn unicode(&mut self, length: u16, name: bool) -> ZipResult<()> {
        // Only the declared payload can become the replacement raw name,
        // decoded name and map key, or the replacement comment. Old charges
        // are intentionally retained when another Unicode field replaces it.
        let copies = if name { 3 } else { 1 };
        let requested = self.multiply(usize::from(length), copies)?;
        self.charge(requested, 0)
    }

    pub(crate) fn extra(&mut self, fields: &Vec<ExtraField>) -> ZipResult<ExtraReservation> {
        self.active()?;
        self.usage.extra_preflights = self.add(self.usage.extra_preflights, 1)?;
        let records = self.add(self.usage.extra_records, 1)?;
        if records > self.limits.extra_records {
            return self.reject();
        }
        let needed = self.add(fields.len(), 1)?;
        let old_capacity = fields.capacity();
        let new_capacity = if needed > old_capacity {
            Some(self.multiply(old_capacity, 2)?.max(4).max(needed))
        } else {
            None
        };
        let delta = match new_capacity {
            Some(capacity) => match capacity.checked_sub(old_capacity) {
                Some(delta) => delta,
                None => return self.reject(),
            },
            None => 0,
        };
        let growth = self.multiply(delta, size_of::<ExtraField>())?;
        let old_overlap = if new_capacity.is_some() {
            self.multiply(old_capacity, size_of::<ExtraField>())?
        } else {
            0
        };
        // Charge the logical record once, new requested capacity once, and the
        // old vector's temporary overlap before constructing the enum or making
        // any reservation. A failed parse does not refund this admission.
        let retained = self.add(size_of::<ExtraField>(), growth)?;
        self.charge(retained, old_overlap)?;
        self.usage.extra_records = records;
        Ok(ExtraReservation {
            new_capacity,
            capacity_delta: delta,
        })
    }

    pub(crate) fn reserve(
        &mut self,
        fields: &mut Vec<ExtraField>,
        reservation: ExtraReservation,
    ) -> ZipResult<()> {
        self.active()?;
        if let Some(capacity) = reservation.new_capacity {
            let additional = capacity.checked_sub(fields.len()).ok_or_else(|| {
                self.usage.denied = true;
                ZipError::UnsupportedArchive(ZipError::METADATA_LIMIT_EXCEEDED)
            })?;
            self.usage.vector_reservations = self.add(self.usage.vector_reservations, 1)?;
            self.usage.vector_requested_slots =
                self.add(self.usage.vector_requested_slots, capacity)?;
            self.usage.vector_capacity_delta_slots = self.add(
                self.usage.vector_capacity_delta_slots,
                reservation.capacity_delta,
            )?;
            fields.try_reserve_exact(additional).map_err(|_| {
                self.usage.denied = true;
                ZipError::UnsupportedArchive(ZipError::METADATA_LIMIT_EXCEEDED)
            })?;
        }
        Ok(())
    }

    pub(crate) fn pushed(&mut self) -> ZipResult<()> {
        self.active()?;
        self.usage.extra_pushes = self.add(self.usage.extra_pushes, 1)?;
        Ok(())
    }
}
