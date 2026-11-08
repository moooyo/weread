//! Initialization allowances for font payloads, metadata, outline admission and inventories.

use std::{
    fmt, fs,
    io::Read,
    path::{Path, PathBuf},
};

pub(super) const MAX_FAMILY_BYTES: usize = 4096;
pub(super) const MAX_VARIATION_AXES: usize = 64;
// Family/record staging plus conservative FontInfo axis-vector copies. The
// raw table count is checked before either temporary or shared registration.
const METADATA_PER_FACE: usize = MAX_FAMILY_BYTES * 8 + MAX_VARIATION_AXES * 16 * 4 + 4096;

#[derive(Clone, Copy)]
pub(super) struct FontLimits {
    pub per_file_faces: usize,
    pub faces: usize,
    pub payload_and_metrics: usize,
    pub metadata: usize,
    pub read_work: usize,
    pub name_work: usize,
    pub glyph_work: usize,
    pub glyph_visits: usize,
    pub glyph_points: usize,
    pub glyph_state_bytes: usize,
    pub per_glyph_visits: usize,
    pub per_glyph_points: usize,
    pub per_glyph_memory: usize,
    pub inventory_entries: usize,
    pub inventory_path_bytes: usize,
}

impl Default for FontLimits {
    fn default() -> Self {
        Self {
            per_file_faces: 16,
            faces: 128,
            payload_and_metrics: 256 * 1024 * 1024,
            metadata: 8 * 1024 * 1024,
            read_work: 128 * 1024 * 1024,
            name_work: 64 * 1024 * 1024,
            // Include raw inspection and per-glyph CFF DICT/subroutine work.
            // This is independent of the unchanged actual-read allowance.
            glyph_work: 256 * 1024 * 1024,
            glyph_visits: 64 * 1024 * 1024,
            glyph_points: 128 * 1024 * 1024,
            glyph_state_bytes: 8 * 1024 * 1024,
            per_glyph_visits: 65_535,
            per_glyph_points: 65_535,
            per_glyph_memory: 4 * 1024 * 1024,
            inventory_entries: 256,
            inventory_path_bytes: 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct FontUsage {
    pub faces: usize,
    pub payload_and_metrics: usize,
    pub metadata: usize,
    pub read_work: usize,
    pub name_work: usize,
    pub glyph_work: usize,
    pub glyph_visits: usize,
    pub glyph_points: usize,
    pub glyph_state_bytes: usize,
    pub glyph_memory: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FontLimitError {
    Faces,
    Payload,
    Metadata,
    ReadWork,
    NameWork,
    GlyphWork,
    GlyphComplexity,
    GlyphMemory,
    GlyphProgram,
    GlyphFormat,
    Inventory,
}

impl fmt::Display for FontLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Faces => "font face allowance is exhausted",
            Self::Payload => "font payload and metric-copy allowance is exhausted",
            Self::Metadata => "font metadata allowance is exhausted",
            Self::ReadWork => "font initialization read-work allowance is exhausted",
            Self::NameWork => "font name-preflight work allowance is exhausted",
            Self::GlyphWork => "font glyph-preflight work allowance is exhausted",
            Self::GlyphComplexity => "font glyph expansion allowance is exhausted",
            Self::GlyphMemory => "font glyph temporary-memory allowance is exhausted",
            Self::GlyphProgram => "font glyph data is invalid for bounded preflight",
            Self::GlyphFormat => "font glyph program is unsupported by bounded preflight",
            Self::Inventory => "font directory inventory allowance is exhausted",
        })
    }
}

impl std::error::Error for FontLimitError {}

#[derive(Clone, Copy, Debug)]
pub(super) enum GlyphWorkKind {
    General,
    IndexRead,
    IndexSpan,
    DictParse,
    DictProjection,
    Program,
    Operand,
    Output,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct GlyphWorkUsage {
    pub index_read: usize,
    pub index_span: usize,
    pub dict_parse: usize,
    pub dict_projection: usize,
    pub program: usize,
    pub operand: usize,
    pub output: usize,
    pub general: usize,
}

#[cfg(test)]
impl GlyphWorkUsage {
    pub fn total(self) -> usize {
        [
            self.index_read,
            self.index_span,
            self.dict_parse,
            self.dict_projection,
            self.program,
            self.operand,
            self.output,
            self.general,
        ]
        .into_iter()
        .try_fold(0usize, usize::checked_add)
        .expect("Category charges are bounded by the total glyph allowance")
    }

    pub fn difference(self, previous: Self) -> Self {
        Self {
            index_read: self.index_read - previous.index_read,
            index_span: self.index_span - previous.index_span,
            dict_parse: self.dict_parse - previous.dict_parse,
            dict_projection: self.dict_projection - previous.dict_projection,
            program: self.program - previous.program,
            operand: self.operand - previous.operand,
            output: self.output - previous.output,
            general: self.general - previous.general,
        }
    }
}

pub(super) struct FontBudget {
    pub limits: FontLimits,
    usage: FontUsage,
    #[cfg(test)]
    pub metric_copies: usize,
    #[cfg(test)]
    pub shared_registrations: usize,
    #[cfg(test)]
    pub temporary_registrations: usize,
    #[cfg(test)]
    pub name_root_headers: usize,
    #[cfg(test)]
    pub name_directory_records: usize,
    #[cfg(test)]
    pub name_table_headers: usize,
    #[cfg(test)]
    pub name_record_visits: usize,
    #[cfg(test)]
    pub glyph_state_allocations: usize,
    #[cfg(test)]
    pub glyph_graph_nodes: usize,
    #[cfg(test)]
    pub glyph_component_records: usize,
    #[cfg(test)]
    pub glyph_program_tokens: usize,
    #[cfg(test)]
    pub glyph_subroutine_calls: usize,
    #[cfg(test)]
    pub glyph_work_usage: GlyphWorkUsage,
}

impl Default for FontBudget {
    fn default() -> Self {
        Self::new(FontLimits::default())
    }
}

impl FontBudget {
    pub fn new(limits: FontLimits) -> Self {
        Self {
            limits,
            usage: FontUsage::default(),
            #[cfg(test)]
            metric_copies: 0,
            #[cfg(test)]
            shared_registrations: 0,
            #[cfg(test)]
            temporary_registrations: 0,
            #[cfg(test)]
            name_root_headers: 0,
            #[cfg(test)]
            name_directory_records: 0,
            #[cfg(test)]
            name_table_headers: 0,
            #[cfg(test)]
            name_record_visits: 0,
            #[cfg(test)]
            glyph_state_allocations: 0,
            #[cfg(test)]
            glyph_graph_nodes: 0,
            #[cfg(test)]
            glyph_component_records: 0,
            #[cfg(test)]
            glyph_program_tokens: 0,
            #[cfg(test)]
            glyph_subroutine_calls: 0,
            #[cfg(test)]
            glyph_work_usage: GlyphWorkUsage::default(),
        }
    }

    /// Reserve before parsing metadata, copying any metrics, or registering a
    /// source globally. Failed attempts and overwritten sources are not refunded.
    pub fn reserve_font(&mut self, bytes: usize, raw_faces: usize) -> Result<(), FontLimitError> {
        if raw_faces == 0 || raw_faces > self.limits.per_file_faces {
            return Err(FontLimitError::Faces);
        }
        let faces = self
            .usage
            .faces
            .checked_add(raw_faces)
            .filter(|faces| *faces <= self.limits.faces)
            .ok_or(FontLimitError::Faces)?;
        let copies = raw_faces
            .checked_add(1)
            .and_then(|count| bytes.checked_mul(count))
            .ok_or(FontLimitError::Payload)?;
        let payload_and_metrics = self
            .usage
            .payload_and_metrics
            .checked_add(copies)
            .filter(|bytes| *bytes <= self.limits.payload_and_metrics)
            .ok_or(FontLimitError::Payload)?;
        let metadata = raw_faces
            .checked_mul(METADATA_PER_FACE)
            .and_then(|bytes| self.usage.metadata.checked_add(bytes))
            .filter(|bytes| *bytes <= self.limits.metadata)
            .ok_or(FontLimitError::Metadata)?;
        self.usage = FontUsage {
            faces,
            payload_and_metrics,
            metadata,
            read_work: self.usage.read_work,
            name_work: self.usage.name_work,
            glyph_work: self.usage.glyph_work,
            glyph_visits: self.usage.glyph_visits,
            glyph_points: self.usage.glyph_points,
            glyph_state_bytes: self.usage.glyph_state_bytes,
            glyph_memory: self.usage.glyph_memory,
        };
        Ok(())
    }

    /// Price additional staging copies of application-owned names/records.
    pub fn reserve_metadata(&mut self, bytes: usize) -> Result<(), FontLimitError> {
        self.usage.metadata = self
            .usage
            .metadata
            .checked_add(bytes)
            .filter(|bytes| *bytes <= self.limits.metadata)
            .ok_or(FontLimitError::Metadata)?;
        Ok(())
    }

    /// Prepay logical header, directory, and record-search work independently
    /// of retained bytes. Aliases and failed sources never refund prior work.
    pub fn reserve_name_work(&mut self, work: usize) -> Result<(), FontLimitError> {
        self.usage.name_work = self
            .usage
            .name_work
            .checked_add(work)
            .filter(|work| *work <= self.limits.name_work)
            .ok_or(FontLimitError::NameWork)?;
        Ok(())
    }

    /// Price each borrowed glyph header, operand, edge and skipped byte span
    /// before inspecting it. Failed sources keep all work already performed.
    pub fn reserve_glyph_work(&mut self, work: usize) -> Result<(), FontLimitError> {
        self.reserve_glyph_work_as(GlyphWorkKind::General, work)
    }

    pub fn reserve_glyph_work_as(
        &mut self,
        kind: GlyphWorkKind,
        work: usize,
    ) -> Result<(), FontLimitError> {
        self.usage.glyph_work = self
            .usage
            .glyph_work
            .checked_add(work)
            .filter(|work| *work <= self.limits.glyph_work)
            .ok_or(FontLimitError::GlyphWork)?;
        #[cfg(test)]
        {
            let category = match kind {
                GlyphWorkKind::General => &mut self.glyph_work_usage.general,
                GlyphWorkKind::IndexRead => &mut self.glyph_work_usage.index_read,
                GlyphWorkKind::IndexSpan => &mut self.glyph_work_usage.index_span,
                GlyphWorkKind::DictParse => &mut self.glyph_work_usage.dict_parse,
                GlyphWorkKind::DictProjection => &mut self.glyph_work_usage.dict_projection,
                GlyphWorkKind::Program => &mut self.glyph_work_usage.program,
                GlyphWorkKind::Operand => &mut self.glyph_work_usage.operand,
                GlyphWorkKind::Output => &mut self.glyph_work_usage.output,
            };
            *category += work;
        }
        #[cfg(not(test))]
        let _ = kind;
        Ok(())
    }

    /// The operation-local graph cache is freed before the next face. Its peak
    /// is bounded separately, and every allocation also spends cumulative work.
    pub fn reserve_glyph_state(&mut self, bytes: usize) -> Result<(), FontLimitError> {
        if bytes > self.limits.glyph_state_bytes {
            return Err(FontLimitError::GlyphMemory);
        }
        self.reserve_glyph_work(bytes)?;
        self.usage.glyph_state_bytes = self.usage.glyph_state_bytes.max(bytes);
        Ok(())
    }

    /// Summaries count repeated references, even when raw inspection reuses a
    /// cached child. Temporary outline memory is a per-glyph peak, not a copy
    /// retained for every possible glyph at once.
    pub fn reserve_glyph_summary(
        &mut self,
        visits: usize,
        points: usize,
        memory: usize,
    ) -> Result<(), FontLimitError> {
        if visits > self.limits.per_glyph_visits || points > self.limits.per_glyph_points {
            return Err(FontLimitError::GlyphComplexity);
        }
        if memory > self.limits.per_glyph_memory {
            return Err(FontLimitError::GlyphMemory);
        }
        let visits = self
            .usage
            .glyph_visits
            .checked_add(visits)
            .filter(|visits| *visits <= self.limits.glyph_visits)
            .ok_or(FontLimitError::GlyphComplexity)?;
        let points = self
            .usage
            .glyph_points
            .checked_add(points)
            .filter(|points| *points <= self.limits.glyph_points)
            .ok_or(FontLimitError::GlyphComplexity)?;
        self.usage.glyph_visits = visits;
        self.usage.glyph_points = points;
        self.usage.glyph_memory = self.usage.glyph_memory.max(memory);
        Ok(())
    }

    pub fn can_read_file(&self, bytes: usize) -> bool {
        bytes
            .checked_mul(2)
            .and_then(|bytes| self.usage.payload_and_metrics.checked_add(bytes))
            .is_some_and(|bytes| bytes <= self.limits.payload_and_metrics)
            && bytes <= self.remaining_read_work()
    }

    pub fn remaining_read_work(&self) -> usize {
        self.limits.read_work.saturating_sub(self.usage.read_work)
    }

    /// A successful partial read already happened. Keep its charge even when
    /// an excess sentinel, later I/O error, or font preflight rejects the source.
    pub fn record_read(&mut self, bytes: usize) -> Result<(), FontLimitError> {
        self.usage.read_work = self
            .usage
            .read_work
            .checked_add(bytes)
            .unwrap_or(usize::MAX);
        if self.usage.read_work > self.limits.read_work {
            return Err(FontLimitError::ReadWork);
        }
        Ok(())
    }

    /// Even one accepted face needs its source blob and one complete metrics
    /// copy. A growing input must remain within that minimum forecast too.
    pub fn payload_read_limit(&self, maximum: u64) -> u64 {
        ((self
            .limits
            .payload_and_metrics
            .saturating_sub(self.usage.payload_and_metrics)
            / 2) as u64)
            .min(self.remaining_read_work() as u64)
            .min(maximum)
    }

    pub fn usage(&self) -> FontUsage {
        self.usage
    }
}

/// The extra byte distinguishes an exact boundary from a growing/oversized file.
pub(super) fn read_font_payload(
    reader: impl Read,
    maximum: u64,
    budget: &mut FontBudget,
) -> anyhow::Result<Vec<u8>> {
    if budget.remaining_read_work() == 0 {
        return Err(FontLimitError::ReadWork.into());
    }
    let maximum = budget.payload_read_limit(maximum);
    if maximum == 0 {
        return Err(FontLimitError::Payload.into());
    }
    let mut reader = reader.take(maximum.checked_add(1).ok_or(FontLimitError::Payload)?);
    let mut bytes = Vec::new();
    let mut fragment = [0; 8192];
    loop {
        let read = reader.read(&mut fragment)?;
        if read == 0 {
            break;
        }
        budget.record_read(read)?;
        if bytes
            .len()
            .checked_add(read)
            .is_none_or(|length| length as u64 > maximum)
        {
            return Err(FontLimitError::Payload.into());
        }
        bytes.extend_from_slice(&fragment[..read]);
    }
    Ok(bytes)
}

pub(super) fn font_inventory(directory: &Path, limits: FontLimits) -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut bytes = 0usize;
    let prefix_bytes = directory.as_os_str().as_encoded_bytes().len();
    for entry in fs::read_dir(directory)? {
        // Admit before allocating the next owned path or appending it.
        if paths.len() >= limits.inventory_entries {
            return Err(FontLimitError::Inventory.into());
        }
        let entry = entry?;
        let name = entry.file_name();
        let path_bytes = prefix_bytes
            .checked_add(1)
            .and_then(|bytes| bytes.checked_add(name.as_encoded_bytes().len()))
            .ok_or(FontLimitError::Inventory)?;
        bytes = bytes
            .checked_add(path_bytes)
            .filter(|bytes| *bytes <= limits.inventory_path_bytes)
            .ok_or(FontLimitError::Inventory)?;
        paths.push(entry.path());
    }
    paths.sort();
    Ok(paths)
}
