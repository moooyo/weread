//! WeRead wire encoding, independently expressed from the inspected Lua protocol.
//! Reference: moooyo/weread.koplugin at 2943080c2493a1ae262cc97c74908ed62928c935.

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use super::{Result, SourceError};

type PositionResult<T> = std::result::Result<T, super::ReadingPositionLimitError>;

pub(crate) fn reading_position_limits() -> super::ReadingPositionLimits {
    let maximum = super::ReadingPositionLimits::default();
    #[cfg(test)]
    {
        let selected = super::client::position_budget_tests::limits();
        return super::ReadingPositionLimits {
            uid_bytes: selected.uid_bytes.min(maximum.uid_bytes),
            summary_bytes: selected.summary_bytes.min(maximum.summary_bytes),
            device_bytes: selected.device_bytes.min(maximum.device_bytes),
            scalar_bytes: selected.scalar_bytes.min(maximum.scalar_bytes),
            metadata_bytes: selected.metadata_bytes.min(maximum.metadata_bytes),
            work_bytes: selected.work_bytes.min(maximum.work_bytes),
        };
    }
    #[cfg(not(test))]
    maximum
}

/// One position operation owns its ledger. Fees cover selected DTO/queue/JSON
/// output and bounded scalar/parser scratch; they are not a total RSS limit.
pub(crate) struct ReadingPositionBudget {
    limits: super::ReadingPositionLimits,
    usage: super::ReadingPositionUsage,
}

impl ReadingPositionBudget {
    pub(crate) fn new() -> PositionResult<Self> {
        let mut budget = Self {
            limits: reading_position_limits(),
            usage: super::ReadingPositionUsage::default(),
        };
        budget.metadata(16 * 1024 + 8 * std::mem::size_of::<super::ReadingPosition>() + 512)?;
        budget.work(4096)?;
        Ok(budget)
    }

    pub(crate) fn usage(&self) -> super::ReadingPositionUsage {
        self.usage
    }

    fn observe(&self) {
        #[cfg(test)]
        super::client::position_budget_tests::observe_usage(self.usage);
    }

    fn reserve(
        kind: super::ReadingPositionLimitKind,
        used: usize,
        requested: usize,
        limit: usize,
    ) -> PositionResult<usize> {
        used.checked_add(requested)
            .filter(|total| *total <= limit)
            .ok_or(super::ReadingPositionLimitError {
                kind,
                used,
                requested,
                limit,
            })
    }

    fn multiply(
        &self,
        kind: super::ReadingPositionLimitKind,
        bytes: usize,
        factor: usize,
    ) -> PositionResult<usize> {
        bytes
            .checked_mul(factor)
            .ok_or(super::ReadingPositionLimitError {
                kind,
                used: 0,
                requested: usize::MAX,
                limit: if kind == super::ReadingPositionLimitKind::Metadata {
                    self.limits.metadata_bytes
                } else {
                    self.limits.work_bytes
                },
            })
    }

    pub(crate) fn metadata(&mut self, bytes: usize) -> PositionResult<()> {
        self.usage.metadata_bytes = Self::reserve(
            super::ReadingPositionLimitKind::Metadata,
            self.usage.metadata_bytes,
            bytes,
            self.limits.metadata_bytes,
        )?;
        self.observe();
        Ok(())
    }

    pub(crate) fn work(&mut self, bytes: usize) -> PositionResult<()> {
        self.usage.work_bytes = Self::reserve(
            super::ReadingPositionLimitKind::Work,
            self.usage.work_bytes,
            bytes,
            self.limits.work_bytes,
        )?;
        self.observe();
        Ok(())
    }

    pub(crate) fn check_field(
        &self,
        kind: super::ReadingPositionLimitKind,
        bytes: usize,
    ) -> PositionResult<()> {
        let maximum = match kind {
            super::ReadingPositionLimitKind::Uid => self.limits.uid_bytes,
            super::ReadingPositionLimitKind::Summary => self.limits.summary_bytes,
            super::ReadingPositionLimitKind::Device => self.limits.device_bytes,
            _ => self.limits.scalar_bytes,
        };
        Self::reserve(kind, 0, bytes, maximum).map(|_| ())
    }

    pub(crate) fn fields(
        &mut self,
        uid: usize,
        summary: usize,
        device: usize,
    ) -> PositionResult<()> {
        self.check_field(super::ReadingPositionLimitKind::Uid, uid)?;
        self.check_field(super::ReadingPositionLimitKind::Summary, summary)?;
        self.check_field(super::ReadingPositionLimitKind::Device, device)?;
        let bytes = uid
            .checked_add(summary)
            .and_then(|bytes| bytes.checked_add(device))
            .ok_or(super::ReadingPositionLimitError {
                kind: super::ReadingPositionLimitKind::Metadata,
                used: self.usage.field_bytes,
                requested: usize::MAX,
                limit: self.limits.metadata_bytes,
            })?;
        self.field_bytes(bytes)
    }

    fn field_bytes(&mut self, bytes: usize) -> PositionResult<()> {
        self.metadata(self.multiply(super::ReadingPositionLimitKind::Metadata, bytes, 8)?)?;
        self.work(self.multiply(super::ReadingPositionLimitKind::Work, bytes, 16)?)?;
        self.usage.field_bytes = Self::reserve(
            super::ReadingPositionLimitKind::Metadata,
            self.usage.field_bytes,
            bytes,
            self.limits.metadata_bytes,
        )?;
        self.observe();
        Ok(())
    }

    pub(crate) fn field(
        &mut self,
        kind: super::ReadingPositionLimitKind,
        bytes: usize,
    ) -> PositionResult<()> {
        self.check_field(kind, bytes)?;
        self.field_bytes(bytes)
    }

    pub(crate) fn node(&mut self) -> PositionResult<()> {
        self.work(1024)?;
        self.usage.nodes = Self::reserve(
            super::ReadingPositionLimitKind::Work,
            self.usage.nodes,
            1,
            self.limits.work_bytes,
        )?;
        self.observe();
        Ok(())
    }

    pub(crate) fn scalar(&mut self, bytes: usize) -> PositionResult<()> {
        self.check_field(super::ReadingPositionLimitKind::Scalar, bytes)?;
        self.work(self.multiply(super::ReadingPositionLimitKind::Work, bytes, 8)?)?;
        Ok(())
    }

    pub(crate) fn json_input(&mut self, bytes: usize) -> PositionResult<()> {
        self.metadata(self.multiply(super::ReadingPositionLimitKind::Metadata, bytes, 2)?)?;
        self.work(self.multiply(super::ReadingPositionLimitKind::Work, bytes, 16)?)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReaderLimits {
    pub field_bytes: usize,
    pub aggregate_bytes: usize,
    pub request_bytes: usize,
    pub metadata_bytes: usize,
    pub work_bytes: usize,
}

impl Default for ReaderLimits {
    fn default() -> Self {
        Self {
            field_bytes: 4096,
            aggregate_bytes: 16 * 1024,
            request_bytes: 64 * 1024,
            metadata_bytes: 64 * 1024 * 1024,
            work_bytes: 128 * 1024 * 1024,
        }
    }
}

pub(super) fn reader_limits() -> ReaderLimits {
    let maximum = ReaderLimits::default();
    #[cfg(test)]
    {
        let selected = super::client::reader_resource_budget_tests::reader_limits();
        return ReaderLimits {
            field_bytes: selected.field_bytes.min(maximum.field_bytes),
            aggregate_bytes: selected.aggregate_bytes.min(maximum.aggregate_bytes),
            request_bytes: selected.request_bytes.min(maximum.request_bytes),
            metadata_bytes: selected.metadata_bytes.min(maximum.metadata_bytes),
            work_bytes: selected.work_bytes.min(maximum.work_bytes),
        };
    }
    #[cfg(not(test))]
    maximum
}

fn add(left: usize, right: usize) -> Result<usize> {
    left.checked_add(right).ok_or(SourceError::ResponseTooLarge)
}

fn multiply(value: usize, factor: usize) -> Result<usize> {
    value
        .checked_mul(factor)
        .ok_or(SourceError::ResponseTooLarge)
}

fn admit(value: usize, maximum: usize) -> Result<()> {
    if value > maximum {
        Err(SourceError::ResponseTooLarge)
    } else {
        Ok(())
    }
}

/// A borrowed scalar forecast, including JSON decoding before a String exists.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct OpaquePrice {
    pub decoded_bytes: usize,
    pub json_bytes: usize,
    pub percent_bytes: usize,
    pub input_bytes: usize,
}

impl OpaquePrice {
    pub(super) fn text(value: &str) -> Result<Self> {
        let mut price = Self {
            input_bytes: value.len(),
            json_bytes: 2,
            ..Self::default()
        };
        for character in value.chars() {
            price.character(character)?;
        }
        Ok(price)
    }

    pub(super) fn character(&mut self, character: char) -> Result<()> {
        let bytes = character.len_utf8();
        self.decoded_bytes = add(self.decoded_bytes, bytes)?;
        let quoted = match character {
            '"' | '\\' | '\u{08}' | '\u{0c}' | '\n' | '\r' | '\t' => 2,
            character if character <= '\u{1f}' => 6,
            _ => bytes,
        };
        self.json_bytes = add(self.json_bytes, quoted)?;
        let percent =
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '~') {
                1
            } else {
                multiply(bytes, 3)?
            };
        self.percent_bytes = add(self.percent_bytes, percent)?;
        Ok(())
    }
}

pub(super) fn admit_reader_scan(bytes: usize) -> Result<()> {
    // Price the assignment, borrowed JSON walk and three possible loose-field
    // searches. This is logical scanning work, not a total DOM/RSS policy.
    admit(add(multiply(bytes, 8)?, 512)?, reader_limits().work_bytes)
}

pub(super) fn admit_reader_script_copy(bytes: usize) -> Result<()> {
    let limits = reader_limits();
    admit(add(multiply(bytes, 4)?, 4096)?, limits.metadata_bytes)?;
    admit(add(multiply(bytes, 8)?, 512)?, limits.work_bytes)
}

pub(super) fn admit_reader_fields(fields: [OpaquePrice; 3]) -> Result<()> {
    let limits = reader_limits();
    let mut decoded = 0;
    let mut json = 0;
    let mut percent = 0;
    let mut input = 0;
    for (index, field) in fields.into_iter().enumerate() {
        admit(field.decoded_bytes, limits.field_bytes)?;
        admit(
            field.input_bytes,
            add(multiply(limits.field_bytes, 6)?, 32)?,
        )?;
        decoded = add(decoded, field.decoded_bytes)?;
        if index < 2 {
            json = add(json, field.json_bytes)?;
            percent = add(percent, field.percent_bytes)?;
        }
        input = add(input, field.input_bytes)?;
    }
    admit(decoded, limits.aggregate_bytes)?;
    // Both clock fields reach signed JSON/query output. Token is independently
    // priced for its decoded retention and SHA input, not as an unused query.
    admit(add(json, 2048)?, limits.request_bytes)?;
    admit(add(percent, 2048)?, limits.request_bytes)?;
    let metadata = add(add(multiply(decoded, 8)?, multiply(input, 2)?)?, 4096)?;
    let work = add(
        add(multiply(input, 4)?, multiply(add(json, percent)?, 4)?)?,
        512,
    )?;
    admit(metadata, limits.metadata_bytes)?;
    admit(work, limits.work_bytes)?;
    #[cfg(test)]
    super::client::reader_resource_budget_tests::observe_fields(metadata, work);
    Ok(())
}

pub(super) fn admit_reader_clock_replacement(
    psvts: &str,
    token: Option<&str>,
    timestamp: &str,
) -> Result<()> {
    let upper = encoded_id_upper(timestamp)?;
    admit_reader_fields([
        OpaquePrice::text(psvts)?,
        OpaquePrice {
            decoded_bytes: upper,
            json_bytes: add(upper, 2)?,
            percent_bytes: upper,
            input_bytes: timestamp.len(),
        },
        OpaquePrice::text(token.unwrap_or_default())?,
    ])
}

pub(super) fn admit_reader_opaque(
    psvts: &str,
    pclts: Option<&str>,
    token: Option<&str>,
) -> Result<()> {
    let limits = reader_limits();
    for field in [psvts, pclts.unwrap_or_default(), token.unwrap_or_default()] {
        admit(field.len(), limits.field_bytes)?;
    }
    admit_reader_fields([
        OpaquePrice::text(psvts)?,
        OpaquePrice::text(pclts.unwrap_or_default())?,
        OpaquePrice::text(token.unwrap_or_default())?,
    ])
}

#[derive(Clone, Copy)]
struct RequestAdmission {
    query_bytes: usize,
}

fn encoded_id_upper(value: &str) -> Result<usize> {
    // Hex output, numeric chunks/separators and the fixed hashes are covered
    // before encoded_id allocates or hashes an opaque ID.
    add(multiply(value.len(), 2)?, 64)
}

fn admit_reader_request(
    book_id: &str,
    chapter_uid: &str,
    fields: [OpaquePrice; 3],
    summary: &str,
) -> Result<RequestAdmission> {
    admit_reader_fields(fields)?;
    admit(book_id.len(), 4096)?;
    admit(chapter_uid.len(), 4096)?;
    let limits = reader_limits();
    let ids = add(encoded_id_upper(book_id)?, encoded_id_upper(chapter_uid)?)?;
    let summary = OpaquePrice::text(summary)?;
    let query = add(
        add(ids, summary.percent_bytes)?,
        add(add(fields[0].percent_bytes, fields[1].percent_bytes)?, 2048)?,
    )?;
    let json = add(
        add(ids, summary.json_bytes)?,
        add(add(fields[0].json_bytes, fields[1].json_bytes)?, 2048)?,
    )?;
    admit(query, limits.request_bytes)?;
    admit(json, limits.request_bytes)?;
    let bytes = add(add(ids, query)?, add(json, fields[2].decoded_bytes)?)?;
    admit(add(multiply(bytes, 8)?, 4096)?, limits.metadata_bytes)?;
    admit(add(multiply(bytes, 16)?, 4096)?, limits.work_bytes)?;
    Ok(RequestAdmission { query_bytes: query })
}

/// Every actual Reader URL producer admits IDs before hex/hash/copy work.
pub fn reader_url_checked(book_id: &str, chapter_uid: Option<&str>) -> Result<String> {
    admit(book_id.len(), 4096)?;
    admit(chapter_uid.map_or(0, str::len), 4096)?;
    let bytes = add(
        add(
            encoded_id_upper(book_id)?,
            chapter_uid.map(encoded_id_upper).transpose()?.unwrap_or(0),
        )?,
        128,
    )?;
    let limits = reader_limits();
    admit(bytes, limits.request_bytes)?;
    admit(add(multiply(bytes, 8)?, 512)?, limits.metadata_bytes)?;
    admit(add(multiply(bytes, 16)?, 512)?, limits.work_bytes)?;
    #[cfg(test)]
    super::client::reader_resource_budget_tests::observe_reader_url();
    Ok(reader_url_admitted(book_id, chapter_uid))
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ResourceUrlLimits {
    pub raw_bytes: usize,
    pub normalized_bytes: usize,
    pub metadata_bytes: usize,
    pub work_bytes: usize,
}

impl Default for ResourceUrlLimits {
    fn default() -> Self {
        Self {
            raw_bytes: 4096,
            normalized_bytes: 16 * 1024,
            metadata_bytes: 64 * 1024 * 1024,
            work_bytes: 128 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum ResourceUrlStage {
    Raw,
    Reused,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ResourceUrlUsage {
    pub normalized_upper: usize,
    pub metadata_bytes: usize,
    pub work_bytes: usize,
}

fn resource_url_limits() -> ResourceUrlLimits {
    let maximum = ResourceUrlLimits::default();
    #[cfg(test)]
    {
        let selected = super::client::reader_resource_budget_tests::url_limits();
        return ResourceUrlLimits {
            raw_bytes: selected.raw_bytes.min(maximum.raw_bytes),
            normalized_bytes: selected.normalized_bytes.min(maximum.normalized_bytes),
            metadata_bytes: selected.metadata_bytes.min(maximum.metadata_bytes),
            work_bytes: selected.work_bytes.min(maximum.work_bytes),
        };
    }
    #[cfg(not(test))]
    maximum
}

/// One URL's normalization/copy allowance, shared by catalog, planning and
/// transport. It does not spend an Article image occurrence or retain a memo.
pub(super) fn admit_resource_url(
    value: &str,
    stage: ResourceUrlStage,
    prefix_bytes: usize,
) -> Result<ResourceUrlUsage> {
    let limits = resource_url_limits();
    admit(
        value.len(),
        match stage {
            ResourceUrlStage::Raw => limits.raw_bytes,
            ResourceUrlStage::Reused => limits.normalized_bytes,
        },
    )?;
    if value.is_empty() {
        return Ok(ResourceUrlUsage::default());
    }
    let value = value.trim();
    let authority = value
        .find("://")
        .map(|start| start + 3)
        .or_else(|| value.starts_with("//").then_some(2));
    let authority_end = authority.map(|start| {
        value[start..]
            .find(['/', '?', '#'])
            .map_or(value.len(), |end| start + end)
    });
    let mut upper = prefix_bytes;
    let mut mapped_host = false;
    for (index, byte) in value.bytes().enumerate() {
        let host_unicode = !byte.is_ascii()
            && authority
                .zip(authority_end)
                .is_some_and(|(start, end)| (start..end).contains(&index));
        mapped_host |= host_unicode;
        // Price host mapping scratch separately from ordinary UTF8 percent
        // expansion. The small raw field policy and this conservative logical
        // allowance precede the URL parser; neither is a total allocator cap.
        let fee = if host_unicode {
            64
        } else if !byte.is_ascii()
            || byte.is_ascii_control()
            || matches!(byte, b' ' | b'"' | b'<' | b'>' | b'\'' | 0x60)
        {
            3
        } else {
            1
        };
        upper = add(upper, fee)?;
    }
    // Host mapping's conservative scratch bound is not the final ASCII host
    // length. Admit that temporary work here, then check the actual normalized
    // field before any serializer/copy or transport reuse.
    if !mapped_host {
        admit(upper, limits.normalized_bytes)?;
    }
    let usage = ResourceUrlUsage {
        normalized_upper: upper,
        metadata_bytes: add(add(multiply(upper, 8)?, multiply(value.len(), 2)?)?, 4096)?,
        work_bytes: add(add(multiply(upper, 16)?, multiply(value.len(), 12)?)?, 512)?,
    };
    admit(usage.metadata_bytes, limits.metadata_bytes)?;
    admit(usage.work_bytes, limits.work_bytes)?;
    #[cfg(test)]
    super::client::reader_resource_budget_tests::observe_url_preflight(usage);
    Ok(usage)
}

pub(super) fn check_normalized_resource_url(value: &str) -> Result<()> {
    admit(value.len(), resource_url_limits().normalized_bytes)
}

pub(super) fn admit_resource_redirect(current: &url::Url, location: &str) -> Result<()> {
    let relative_prefix = if location.starts_with("//") { 6 } else { 0 };
    let next = admit_resource_url(location, ResourceUrlStage::Raw, relative_prefix)?;
    // join may retain a base path while parsing a relative location. Price both
    // before its allocation; check the final normalized field before reuse.
    let storage = add(current.as_str().len(), next.normalized_upper)?;
    let limits = resource_url_limits();
    admit(add(multiply(storage, 8)?, 4096)?, limits.metadata_bytes)?;
    admit(add(multiply(storage, 16)?, 512)?, limits.work_bytes)
}

pub(super) fn admit_reader_referer(value: &str) -> Result<()> {
    let limits = reader_limits();
    admit(value.len(), limits.request_bytes)?;
    admit(add(multiply(value.len(), 8)?, 512)?, limits.metadata_bytes)?;
    admit(add(multiply(value.len(), 16)?, 512)?, limits.work_bytes)
}

pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36 Edg/135.0.0.0";
pub const SKILL_VERSION: &str = "1.0.5";
pub const READER_TOKEN: &str = "3c5c8717f3daf09iop3423zafeqoi";

pub fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn encode_component(value: &str) -> String {
    let mut result = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

pub fn encoded_id(value: &str) -> String {
    #[cfg(test)]
    super::client::reader_resource_budget_tests::observe_id_hash();
    let hash = format!("{:x}", md5::compute(value.as_bytes()));
    let numeric = !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
    let chunks: Vec<String> = if numeric {
        value
            .as_bytes()
            .chunks(9)
            .map(|part| {
                format!(
                    "{:x}",
                    std::str::from_utf8(part)
                        .unwrap_or("0")
                        .parse::<u64>()
                        .unwrap_or(0)
                )
            })
            .collect()
    } else {
        vec![value.bytes().map(|b| format!("{b:x}")).collect()]
    };
    let mut result = format!(
        "{}{}2{}",
        &hash[..3],
        if numeric { '3' } else { '4' },
        &hash[30..]
    );
    for (index, chunk) in chunks.iter().enumerate() {
        if index != 0 {
            result.push('g');
        }
        result.push_str(&format!("{:02x}{chunk}", chunk.len()));
    }
    if result.len() < 20 {
        result.push_str(&hash[..20 - result.len()]);
    }
    let tail = format!("{:x}", md5::compute(result.as_bytes()));
    result.push_str(&tail[..3]);
    result
}

fn sorted_query_admitted(params: &Map<String, Value>, capacity: usize) -> String {
    let ordered: BTreeMap<_, _> = params
        .iter()
        .filter(|(key, _)| key.as_str() != "s")
        .collect();
    let mut output = String::with_capacity(capacity);
    for (key, value) in ordered {
        if !output.is_empty() {
            output.push('&');
        }
        output.push_str(key);
        output.push('=');
        let text = match value {
            Value::String(text) => std::borrow::Cow::Borrowed(text.as_str()),
            Value::Null => std::borrow::Cow::Borrowed("null"),
            _ => std::borrow::Cow::Owned(value.to_string()),
        };
        for byte in text.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                output.push(char::from(byte));
            } else {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                output.push('%');
                output.push(char::from(HEX[usize::from(byte >> 4)]));
                output.push(char::from(HEX[usize::from(byte & 15)]));
            }
        }
    }
    output
}

#[cfg(test)]
pub fn sorted_query(params: &Map<String, Value>) -> String {
    sorted_query_admitted(params, 0)
}

pub fn sign(query: &str) -> String {
    let mut a: u32 = 0x15051505;
    let mut b = a;
    let bytes = query.as_bytes();
    let mut i = bytes.len();
    while i > 1 {
        a = (a ^ (u32::from(bytes[i - 1]) << ((bytes.len() - i + 1) % 30))) & 0x7fff_ffff;
        b = (b ^ (u32::from(bytes[i - 2]) << ((i - 1) % 30))) & 0x7fff_ffff;
        i -= 2;
    }
    format!("{:x}", a.wrapping_add(b))
}

fn signed_admitted(mut params: Map<String, Value>, admission: RequestAdmission) -> Value {
    let signature = sign(&sorted_query_admitted(&params, admission.query_bytes));
    params.insert("s".into(), Value::String(signature));
    Value::Object(params)
}

pub fn content_payload_checked(
    book_id: &str,
    chapter_uid: &str,
    psvts: &str,
    style: bool,
    timestamp: i64,
) -> Result<Value> {
    admit(psvts.len(), reader_limits().field_bytes)?;
    let admission = admit_reader_request(
        book_id,
        chapter_uid,
        [
            OpaquePrice::text(psvts)?,
            OpaquePrice::default(),
            OpaquePrice::default(),
        ],
        "",
    )?;
    #[cfg(test)]
    super::client::reader_resource_budget_tests::observe_payload();
    let timestamp = if encoded_id(&timestamp.to_string()) == psvts {
        timestamp + 1
    } else {
        timestamp
    };
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        % 10_000;
    let params = json!({
        "b": encoded_id(book_id), "c": encoded_id(chapter_uid),
        "r": (u64::from(nonce) * u64::from(nonce)).to_string(),
        "ct": timestamp.to_string(), "ps": psvts,
        "pc": encoded_id(&timestamp.to_string()), "sc": 1,
        "prevChapter": false, "st": u8::from(style),
    });
    let Value::Object(params) = params else {
        unreachable!("Static Reader payload is an object")
    };
    Ok(signed_admitted(params, admission))
}

fn reader_url_admitted(book_id: &str, chapter_uid: Option<&str>) -> String {
    let suffix = chapter_uid
        .map(|uid| format!("k{}", encoded_id(uid)))
        .unwrap_or_default();
    format!(
        "https://weread.qq.com/web/reader/{}{suffix}",
        encoded_id(book_id)
    )
}

#[cfg(test)]
pub fn reader_url(book_id: &str, chapter_uid: Option<&str>) -> String {
    reader_url_checked(book_id, chapter_uid).expect("Fixture Reader IDs are admitted")
}

pub fn web_app_id() -> String {
    let prefix = USER_AGENT
        .split_whitespace()
        .take(12)
        .map(|p| (p.len() % 10).to_string())
        .collect::<String>();
    let hash = USER_AGENT.bytes().fold(0u32, |hash, byte| {
        hash.wrapping_mul(0x83).wrapping_add(u32::from(byte)) & 0x7fff_ffff
    });
    format!("wb{prefix}h{hash}")
}

pub fn read_payload_checked(
    book_id: &str,
    position: &super::ReadingPosition,
    session: &super::content::ReaderSession,
    elapsed: Option<u32>,
    now: i64,
) -> Result<Value> {
    let subsecond = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    read_payload_with_clock_checked(
        book_id,
        position,
        session,
        elapsed,
        now,
        now.saturating_mul(1000)
            .saturating_add(i64::from(subsecond.subsec_millis())),
        subsecond.subsec_nanos() % 1000,
    )
}

/// Deterministic clock inputs also permit comparison with independent wire fixtures.
pub fn read_payload_with_clock_checked(
    book_id: &str,
    position: &super::ReadingPosition,
    session: &super::content::ReaderSession,
    elapsed: Option<u32>,
    now: i64,
    timestamp: i64,
    nonce: u32,
) -> Result<Value> {
    session.validate_opaque()?;
    let end = position
        .summary
        .char_indices()
        .nth(20)
        .map_or(position.summary.len(), |(index, _)| index);
    let summary = &position.summary[..end];
    let admission = admit_reader_request(
        book_id,
        &position.chapter_uid,
        [
            OpaquePrice::text(&session.psvts)?,
            OpaquePrice::text(session.pclts.as_deref().unwrap_or_default())?,
            OpaquePrice::text(session.token.as_deref().unwrap_or_default())?,
        ],
        summary,
    )?;
    #[cfg(test)]
    super::client::reader_resource_budget_tests::observe_payload();
    let params = json!({
        "appId": web_app_id(), "b": encoded_id(book_id),
        "c": encoded_id(&position.chapter_uid), "ci": position.chapter_index,
        "co": position.chapter_offset, "sm": position.summary.chars().take(20).collect::<String>(),
        "pr": position.progress.min(100), "ct": now, "ps": session.psvts,
        "pc": session.pclts.as_ref().filter(|s| !s.is_empty() && s.as_str() != "0").cloned().unwrap_or_else(|| encoded_id(&now.to_string())),
    });
    let Value::Object(mut params) = params else {
        unreachable!("Static Reader payload is an object")
    };
    if let Some(seconds) = elapsed {
        let nonce = nonce % 1000;
        let token = session
            .token
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(READER_TOKEN);
        #[cfg(test)]
        super::client::reader_resource_budget_tests::observe_token_hash();
        let digest = Sha256::digest(format!("{timestamp}{nonce}{token}").as_bytes());
        params.insert("rt".into(), json!(seconds));
        params.insert("ts".into(), json!(timestamp));
        params.insert("rn".into(), json!(nonce));
        params.insert("sg".into(), json!(hex::encode(digest)));
    }
    Ok(signed_admitted(params, admission))
}

// Historical positive wire fixtures in other modules use the old return type.
// Actual producers have only the fallible entry points above.
#[cfg(test)]
pub fn read_payload_with_clock(
    book_id: &str,
    position: &super::ReadingPosition,
    session: &super::content::ReaderSession,
    elapsed: Option<u32>,
    now: i64,
    timestamp: i64,
    nonce: u32,
) -> Value {
    read_payload_with_clock_checked(book_id, position, session, elapsed, now, timestamp, nonce)
        .expect("Fixture Reader fields are admitted")
}

pub fn normalize_cover_url(url: &str) -> String {
    let bytes = url.as_bytes();
    let mut index = 0;
    while index + 3 < bytes.len() {
        if bytes[index] == b'/' && bytes[index + 1] == b't' {
            let mut end = index + 2;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > index + 2 && bytes.get(end) == Some(&b'_') {
                return format!("{}/t9{}", &url[..index], &url[end..]);
            }
        }
        index += 1;
    }
    url.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_ids_match_independent_lua_reference_vectors() {
        for (input, expected) in [
            ("0", "cfc32da010cfcd208495488"),
            ("1", "c4c329b011c4ca4238a0201"),
            ("57", "72b327f023972b32a1f7e2d"),
            ("22691208", "d2c32380715a3d88d2ccb4b"),
            ("fixture-book", "086420c18666978747572652d626f6f6b81c"),
            (
                "MP_WXS_123_fixture",
                "7e5427b244d505f5758535f3132335f666978747572658f5",
            ),
            ("00123", "368329d027b3680c9b1b664"),
            ("1700000000", "24932c107a21fe80g010f72"),
        ] {
            assert_eq!(encoded_id(input), expected, "input={input}");
        }
    }

    #[test]
    fn signatures_match_independent_lua_reference_vectors() {
        for (input, expected) in [
            ("", "2a0a2a0a"),
            ("abc", "2a0a2a50"),
            ("a=hello%20world&b=17", "288b88ca"),
        ] {
            assert_eq!(sign(input), expected);
        }
        let params =
            json!({"z":"last","a":"a b","unicode":"\u{9605}\u{8bfb}","bool":false,"s":"ignored"});
        let query = sorted_query(params.as_object().unwrap());
        assert_eq!(
            query,
            "a=a%20b&bool=false&unicode=%E9%98%85%E8%AF%BB&z=last"
        );
        assert_eq!(sign(&query), "30ab5524");
        assert_eq!(web_app_id(), "wb115321887466h529830856");
    }
}
