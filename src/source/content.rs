//! Reader-session extraction and integrity-checked chapter decoding.
use super::access::ReaderAccess;
use super::client::borrowed_scalar_text;
use super::{RemoteChapter, Result, SourceError};
use base64::Engine;
use scraper::{Html, Selector};
use serde_json::Value;
use std::{borrow::Cow, collections::HashSet};

pub(crate) const MAX_CATALOG_CHAPTERS: usize = 8192;
const MAX_CATALOG_UID_BYTES: usize = 4096;
const MAX_CATALOG_OWNER_BYTES: usize = 4096;
const MAX_CATALOG_METADATA_BYTES: usize = 64 * 1024 * 1024;

fn valid_catalog_owner(book_id: &str) -> bool {
    !book_id.is_empty()
        && book_id.len() <= MAX_CATALOG_OWNER_BYTES
        && !book_id.chars().any(char::is_control)
}

/// Reserve retained source metadata plus its two owner-side Chapter copies.
/// Body payloads remain in SQL; owner-key copying is separately admitted from
/// scalar Store evidence before old headers are read. No ID format is assumed.
pub fn catalog_metadata_bytes(book_id: &str, chapters: &[RemoteChapter]) -> Result<usize> {
    catalog_metadata_bytes_with_limit(book_id, chapters, MAX_CATALOG_METADATA_BYTES)
}

fn catalog_metadata_bytes_with_limit(
    book_id: &str,
    chapters: &[RemoteChapter],
    maximum: usize,
) -> Result<usize> {
    if !valid_catalog_owner(book_id) {
        return Err(SourceError::InvalidResponse("catalog_owner"));
    }
    validate_catalog(chapters)?;
    let mut used = 0usize;
    for chapter in chapters {
        reserve_catalog_fields(
            &mut used,
            book_id,
            chapter.uid.len(),
            chapter.title.len(),
            &chapter.resource_url,
            maximum,
        )?;
    }
    Ok(used)
}

fn reserve_catalog_fields(
    used: &mut usize,
    book_id: &str,
    uid_bytes: usize,
    title_bytes: usize,
    resource_url: &str,
    maximum: usize,
) -> Result<()> {
    super::protocol::admit_resource_url(
        resource_url,
        super::protocol::ResourceUrlStage::Raw,
        "https://weread.qq.com".len(),
    )?;
    let resource_bytes = resource_url.len();
    let fixed = 2 * std::mem::size_of::<crate::domain::Chapter>()
        + 2 * std::mem::size_of::<RemoteChapter>();
    for part in [
        book_id.len().checked_mul(2),
        uid_bytes.checked_mul(3),
        title_bytes.checked_mul(3),
        Some(resource_bytes),
        Some(fixed),
    ] {
        *used = part
            .and_then(|part| used.checked_add(part))
            .filter(|bytes| *bytes <= maximum)
            .ok_or(SourceError::ResponseTooLarge)?;
    }
    Ok(())
}

fn borrowed_scalar(value: &Value) -> Option<Cow<'_, str>> {
    match value {
        Value::String(text) => Some(Cow::Borrowed(text)),
        Value::Number(number) => Some(Cow::Owned(number.to_string())),
        _ => None,
    }
}

fn catalog_records(payload: &Value) -> &[Value] {
    let records = payload.get("data").unwrap_or(payload);
    if records.get("bookId").is_some() || records.get("updated").is_some() {
        std::slice::from_ref(records)
    } else {
        records.as_array().map(Vec::as_slice).unwrap_or_default()
    }
}

fn valid_catalog_uid(uid: &str) -> bool {
    !uid.is_empty() && uid.len() <= MAX_CATALOG_UID_BYTES && !uid.chars().any(char::is_control)
}

/// Admission for typed results too: owner-side callers must check before
/// consulting cached bodies or deriving a replacement catalog from them.
pub fn validate_catalog(chapters: &[RemoteChapter]) -> Result<()> {
    if chapters.len() > MAX_CATALOG_CHAPTERS {
        return Err(SourceError::ResponseTooLarge);
    }
    let mut seen = HashSet::new();
    for chapter in chapters {
        if !valid_catalog_uid(&chapter.uid) || !seen.insert(chapter.uid.as_str()) {
            return Err(SourceError::InvalidResponse("catalog_uid"));
        }
    }
    Ok(())
}

/// Inspect borrowed, readable UID fields before allocating their normalized
/// metadata. Limits are application allowances, not upstream format claims.
pub fn normalize_catalog_checked(payload: &Value, book_id: &str) -> Result<Vec<RemoteChapter>> {
    #[cfg(not(test))]
    let maximum = MAX_CATALOG_METADATA_BYTES;
    #[cfg(test)]
    let maximum = catalog_field_gate_tests::maximum();
    normalize_catalog_checked_with_limit(payload, book_id, maximum)
}

fn normalize_catalog_checked_with_limit(
    payload: &Value,
    book_id: &str,
    maximum: usize,
) -> Result<Vec<RemoteChapter>> {
    if !valid_catalog_owner(book_id) {
        return Err(SourceError::InvalidResponse("catalog_owner"));
    }
    let mut seen = HashSet::new();
    let mut candidates = 0usize;
    for record in catalog_records(payload) {
        if record
            .get("bookId")
            .and_then(borrowed_scalar_text)
            .as_deref()
            != Some(book_id)
        {
            continue;
        }
        let Some(items) = record
            .get("updated")
            .or_else(|| record.get("chapterInfos"))
            .or_else(|| record.get("chapters"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        candidates = candidates
            .checked_add(items.len())
            .filter(|count| *count <= MAX_CATALOG_CHAPTERS)
            .ok_or(SourceError::ResponseTooLarge)?;
        for item in items {
            if number(item.get("wordCount")) == 0
                || item.get("title").and_then(Value::as_str) == Some("\u{5c01}\u{9762}")
            {
                continue;
            }
            let uid = item
                .get("chapterUid")
                .or_else(|| item.get("chapterId"))
                .and_then(borrowed_scalar_text)
                .ok_or(SourceError::InvalidResponse("catalog_uid"))?;
            if !valid_catalog_uid(&uid) || !seen.insert(uid) {
                return Err(SourceError::InvalidResponse("catalog_uid"));
            }
        }
    }
    let maximum = maximum.min(MAX_CATALOG_METADATA_BYTES);
    let mut used = 0usize;
    // Identity/count rejection retains its existing precedence. This second
    // immutable prepass prices every selected field before normalization can
    // clone any UID, title or resource URL into an output record.
    for record in catalog_records(payload) {
        if record
            .get("bookId")
            .and_then(borrowed_scalar_text)
            .as_deref()
            != Some(book_id)
        {
            continue;
        }
        let Some(items) = record
            .get("updated")
            .or_else(|| record.get("chapterInfos"))
            .or_else(|| record.get("chapters"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for item in items {
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if number(item.get("wordCount")) == 0 || title == "\u{5c01}\u{9762}" {
                continue;
            }
            let uid = item
                .get("chapterUid")
                .or_else(|| item.get("chapterId"))
                .and_then(borrowed_scalar_text)
                .ok_or(SourceError::InvalidResponse("catalog_uid"))?;
            let resource = item.get("tar").and_then(Value::as_str).unwrap_or_default();
            reserve_catalog_fields(
                &mut used,
                book_id,
                uid.len(),
                title.len(),
                resource,
                maximum,
            )?;
        }
    }
    let chapters = normalize_catalog_admitted(payload, book_id);
    catalog_metadata_bytes(book_id, &chapters)?;
    Ok(chapters)
}

#[cfg(test)]
mod catalog_admission_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn duplicate_catalog_uids_are_rejected_before_normalized_output() {
        let value = json!({"bookId":"book","updated":[
            {"chapterUid":"uid","title":"First","wordCount":1},
            {"chapterUid":"uid","title":"Second","wordCount":1},
        ]});
        let error = normalize_catalog_checked(&value, "book").unwrap_err();
        assert!(matches!(
            &error,
            SourceError::InvalidResponse("catalog_uid")
        ));
        assert!(!error.requires_login() && !error.retryable());
    }

    #[test]
    fn readable_catalog_rows_require_basic_bounded_uid_fields() {
        for uid in [
            json!(""),
            json!("bad\nuid"),
            json!("x".repeat(4097)),
            Value::Null,
        ] {
            let value = json!({"bookId":"book","updated":[{"chapterUid":uid,"title":"Fixture","wordCount":1}]});
            assert!(matches!(
                normalize_catalog_checked(&value, "book"),
                Err(SourceError::InvalidResponse("catalog_uid"))
            ));
        }
    }

    #[test]
    fn catalog_record_count_is_admitted_before_readable_mapping() {
        let value = json!({"bookId":"book","updated":vec![json!({"wordCount":0}); MAX_CATALOG_CHAPTERS + 1]});
        assert!(matches!(
            normalize_catalog_checked(&value, "book"),
            Err(SourceError::ResponseTooLarge)
        ));
    }

    #[test]
    fn checked_catalog_keeps_unicode_numeric_alias_and_other_book_contracts() {
        let value = json!({"data":[
            {"bookId":"other","updated":[{"chapterUid":null,"wordCount":1}]},
            {"bookId":"book","chapterInfos":[
                {"chapterUid":12,"chapterIdx":7,"title":"Numeric","wordCount":5},
                {"chapterId":"chapter-\u{4e2d}","title":"Unicode","wordCount":6},
                {"chapterUid":null,"title":"Cover","wordCount":0},
            ]},
        ]});
        let rows = normalize_catalog_checked(&value, "book").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].uid, "12");
        assert_eq!(rows[0].index, 7);
        assert_eq!(rows[1].uid, "chapter-\u{4e2d}");
        validate_catalog(&rows).unwrap();
    }

    #[test]
    fn catalog_metadata_prices_owner_per_new_row_and_exact_small_allowance() {
        let owner = "owner-".repeat(32);
        let rows = (0..3)
            .map(|index| RemoteChapter {
                uid: format!("uid-{index}"),
                title: "Title\u{4e2d}".into(),
                resource_url: "https://example.invalid/resource".into(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let fee = catalog_metadata_bytes(&owner, &rows).unwrap();
        assert_eq!(
            catalog_metadata_bytes_with_limit(&owner, &rows, fee).unwrap(),
            fee
        );
        assert!(matches!(
            catalog_metadata_bytes_with_limit(&owner, &rows, fee - 1),
            Err(SourceError::ResponseTooLarge)
        ));
        let short = catalog_metadata_bytes("o", &rows).unwrap();
        assert_eq!(fee - short, 2 * (owner.len() - 1) * rows.len());
    }

    #[test]
    fn oversized_catalog_owner_is_rejected_before_normalization() {
        let owner = "x".repeat(4097);
        let value = json!({"bookId":owner,"updated":[{"chapterUid":"uid","title":"Fixture","wordCount":1}]});
        assert!(matches!(
            normalize_catalog_checked(&value, &owner),
            Err(SourceError::InvalidResponse("catalog_owner"))
        ));
        let row = RemoteChapter {
            uid: "uid".into(),
            title: "Fixture".into(),
            ..Default::default()
        };
        assert!(matches!(
            catalog_metadata_bytes(&owner, &[row]),
            Err(SourceError::InvalidResponse("catalog_owner"))
        ));
    }
}

#[derive(Clone, Default)]
pub struct ReaderSession {
    pub psvts: String,
    pub pclts: Option<String>,
    pub token: Option<String>,
    pub(super) access: ReaderAccess,
}

impl std::fmt::Debug for ReaderSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReaderSession")
            .field("has_server_clock", &!self.psvts.is_empty())
            .field("has_client_clock", &self.pclts.is_some())
            .field("has_token", &self.token.is_some())
            .field("access", &self.access)
            .finish_non_exhaustive()
    }
}

impl ReaderSession {
    pub fn validate_opaque(&self) -> Result<()> {
        super::protocol::admit_reader_opaque(
            &self.psvts,
            self.pclts.as_deref(),
            self.token.as_deref(),
        )
    }

    /// The reference materializes one fallback clock for the whole enter/read pair.
    pub fn ensure_reader_clock(&mut self, timestamp: i64) -> Result<()> {
        self.validate_opaque()?;
        if self.pclts.as_deref().is_none_or(|value| {
            value.trim().is_empty() || value.trim().parse::<f64>().ok() == Some(0.0)
        }) {
            let timestamp = timestamp.to_string();
            super::protocol::admit_reader_clock_replacement(
                &self.psvts,
                self.token.as_deref(),
                &timestamp,
            )?;
            self.pclts = Some(super::protocol::encoded_id(&timestamp));
        }
        self.validate_opaque()
    }
}

#[derive(Clone, Copy, Default)]
struct JsonReaderFields<'a> {
    fields: [Option<JsonReaderScalar<'a>>; 3],
    input_bytes: [usize; 3],
}

#[derive(Clone, Copy)]
struct JsonReaderScalar<'a> {
    tail: &'a str,
    token: &'a str,
    price: super::protocol::OpaquePrice,
}

#[derive(Clone, Copy)]
enum JsonContext {
    Root,
    Reader,
    ReaderElements,
    Field,
    Other,
}

#[derive(Default)]
struct JsonShape<'a> {
    reader: Option<JsonReaderFields<'a>>,
    scalar: Option<JsonReaderScalar<'a>>,
    input_bytes: usize,
}

/// Selection and decoding ownership are separate: overwritten occurrences
/// still spend their full input/owned/scratch reservations before serde runs.
struct JsonInputBudget {
    limits: super::protocol::ReaderLimits,
    metadata: usize,
    work: usize,
    values: usize,
    readers: usize,
    occurrences: usize,
    opaque_decoded: usize,
    denied: bool,
}

impl JsonInputBudget {
    fn new(input_bytes: usize) -> Result<Self> {
        super::protocol::admit_reader_scan(input_bytes)?;
        let mut budget = Self {
            limits: super::protocol::reader_limits(),
            metadata: 0,
            work: 0,
            values: 0,
            readers: 0,
            occurrences: 0,
            opaque_decoded: 0,
            denied: false,
        };
        let work = input_bytes
            .checked_mul(8)
            .and_then(|bytes| bytes.checked_add(512))
            .ok_or(SourceError::ResponseTooLarge)?;
        let _ = budget.charge(4096, work);
        budget.check()?;
        Ok(budget)
    }

    fn charge(&mut self, metadata: usize, work: usize) -> Option<()> {
        if self.denied {
            return None;
        }
        let Some(metadata) = self.metadata.checked_add(metadata) else {
            self.denied = true;
            return None;
        };
        let Some(work) = self.work.checked_add(work) else {
            self.denied = true;
            return None;
        };
        // Keep spent reservations even when a later token, duplicate or
        // malformed suffix stops this scan.
        self.metadata = metadata;
        self.work = work;
        if metadata > self.limits.metadata_bytes || work > self.limits.work_bytes {
            self.denied = true;
        }
        #[cfg(test)]
        super::client::reader_resource_budget_tests::observe_input(
            self.values,
            self.readers,
            self.occurrences,
            self.metadata,
            self.work,
        );
        (!self.denied).then_some(())
    }

    fn check(&self) -> Result<()> {
        if self.denied {
            Err(SourceError::ResponseTooLarge)
        } else {
            Ok(())
        }
    }

    fn value(&mut self, reader: bool, opaque: bool) -> Option<()> {
        self.values = self.values.checked_add(1).or_else(|| {
            self.denied = true;
            None
        })?;
        if reader {
            self.readers = self.readers.checked_add(1).or_else(|| {
                self.denied = true;
                None
            })?;
        }
        if opaque {
            self.occurrences = self.occurrences.checked_add(1).or_else(|| {
                self.denied = true;
                None
            })?;
            self.opaque_decoded = 0;
        }
        self.charge(2 * std::mem::size_of::<Value>() + 32, 16)
    }

    fn opaque_bytes(&mut self, bytes: usize) -> Option<()> {
        self.opaque_decoded = self.opaque_decoded.checked_add(bytes).or_else(|| {
            self.denied = true;
            None
        })?;
        if self.opaque_decoded > self.limits.field_bytes {
            self.denied = true;
            return None;
        }
        Some(())
    }

    fn raw_string(&mut self, bytes: usize, decoded: usize, opaque: bool) -> Option<()> {
        let metadata = bytes.checked_mul(4).or_else(|| {
            self.denied = true;
            None
        })?;
        let work = bytes.checked_mul(2).or_else(|| {
            self.denied = true;
            None
        })?;
        self.charge(metadata, work)?;
        if opaque {
            self.opaque_bytes(decoded)?;
        }
        Some(())
    }

    fn opaque_input(&mut self, bytes: usize) -> Option<()> {
        let maximum = self
            .limits
            .field_bytes
            .checked_mul(6)
            .and_then(|bytes| bytes.checked_add(32))
            .or_else(|| {
                self.denied = true;
                None
            })?;
        if bytes > maximum {
            self.denied = true;
            return None;
        }
        Some(())
    }

    fn selection(&mut self, prices: [super::protocol::OpaquePrice; 3]) -> Result<()> {
        super::protocol::admit_reader_fields(prices)?;
        let mut decoded = 0usize;
        let mut input = 0usize;
        let mut output = 0usize;
        for (index, price) in prices.into_iter().enumerate() {
            decoded = decoded
                .checked_add(price.decoded_bytes)
                .ok_or(SourceError::ResponseTooLarge)?;
            input = input
                .checked_add(price.input_bytes)
                .ok_or(SourceError::ResponseTooLarge)?;
            if index < 2 {
                output = output
                    .checked_add(price.json_bytes)
                    .and_then(|bytes| bytes.checked_add(price.percent_bytes))
                    .ok_or(SourceError::ResponseTooLarge)?;
            }
        }
        let metadata = decoded
            .checked_mul(8)
            .and_then(|bytes| {
                input
                    .checked_mul(2)
                    .and_then(|input| bytes.checked_add(input))
            })
            .and_then(|bytes| bytes.checked_add(4096))
            .ok_or(SourceError::ResponseTooLarge)?;
        let work = input
            .checked_mul(4)
            .and_then(|bytes| {
                output
                    .checked_mul(4)
                    .and_then(|output| bytes.checked_add(output))
            })
            .and_then(|bytes| bytes.checked_add(512))
            .ok_or(SourceError::ResponseTooLarge)?;
        let _ = self.charge(metadata, work);
        self.check()
    }
}

struct JsonFieldsScan<'a> {
    fields: Option<JsonReaderFields<'a>>,
    complete: bool,
}

/// An allocation-free first-value walk. Last-value selection matches serde,
/// while its independent budget accounts every value, key and decoded string.
struct JsonFieldWalk<'a, 'b> {
    text: &'a str,
    at: usize,
    budget: &'b mut JsonInputBudget,
}

impl<'a> JsonFieldWalk<'a, '_> {
    fn whitespace(&mut self) {
        while self
            .text
            .as_bytes()
            .get(self.at)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.at += 1;
        }
    }

    fn string(&mut self, opaque: bool) -> Option<&'a str> {
        let start = self.at;
        if self.text.as_bytes().get(self.at) != Some(&b'"') {
            return None;
        }
        self.budget
            .charge(2 * std::mem::size_of::<String>() + 32, 8)?;
        self.budget.raw_string(1, 0, opaque)?;
        self.at += 1;
        while let Some(byte) = self.text.as_bytes().get(self.at).copied() {
            if byte == b'"' {
                self.budget.raw_string(1, 0, opaque)?;
                self.at += 1;
                return Some(&self.text[start..self.at]);
            }
            let before = self.at;
            let character = json_character(self.text, &mut self.at);
            let bytes = self.at.saturating_sub(before).max(1);
            self.budget
                .raw_string(bytes, character.map_or(0, char::len_utf8), opaque)?;
            character?;
        }
        None
    }

    fn value(&mut self, context: JsonContext, depth: usize) -> Option<JsonShape<'a>> {
        self.value_in(context, depth, false)
    }

    fn value_in(
        &mut self,
        context: JsonContext,
        depth: usize,
        inherited_opaque: bool,
    ) -> Option<JsonShape<'a>> {
        if depth > 128 {
            return None;
        }
        self.whitespace();
        let start = self.at;
        let field_root = matches!(context, JsonContext::Field);
        let opaque = inherited_opaque || field_root;
        self.budget
            .value(matches!(context, JsonContext::Reader), field_root)?;
        let shape = match *self.text.as_bytes().get(self.at)? {
            b'{' => {
                self.at += 1;
                let mut reader = None;
                let mut fields = JsonReaderFields::default();
                self.whitespace();
                if self.text.as_bytes().get(self.at) == Some(&b'}') {
                    self.at += 1;
                } else {
                    loop {
                        self.whitespace();
                        self.budget.charge(
                            2 * (std::mem::size_of::<String>() + std::mem::size_of::<Value>()) + 64,
                            16,
                        )?;
                        let key = self.string(opaque)?;
                        let index = ["psvts", "pclts", "token"]
                            .iter()
                            .position(|name| json_key_is(key, name));
                        self.whitespace();
                        if self.text.as_bytes().get(self.at) != Some(&b':') {
                            return None;
                        }
                        self.at += 1;
                        let next =
                            if matches!(context, JsonContext::Root) && json_key_is(key, "reader") {
                                JsonContext::Reader
                            } else if matches!(
                                context,
                                JsonContext::Reader | JsonContext::ReaderElements
                            ) && index.is_some()
                            {
                                JsonContext::Field
                            } else {
                                JsonContext::Other
                            };
                        let shape = self.value_in(next, depth + 1, opaque)?;
                        if matches!(context, JsonContext::Root) && json_key_is(key, "reader") {
                            reader = shape.reader;
                        }
                        if matches!(context, JsonContext::Reader) {
                            if let Some(index) = index {
                                fields.fields[index] = shape.scalar;
                                fields.input_bytes[index] = shape.input_bytes;
                            }
                        }
                        self.whitespace();
                        match self.text.as_bytes().get(self.at)? {
                            b'}' => {
                                self.at += 1;
                                break;
                            }
                            b',' => self.at += 1,
                            _ => return None,
                        }
                    }
                }
                Some(JsonShape {
                    reader: if matches!(context, JsonContext::Reader) {
                        Some(fields)
                    } else {
                        reader
                    },
                    scalar: None,
                    input_bytes: self.at - start,
                })
            }
            b'[' => {
                self.at += 1;
                self.whitespace();
                if self.text.as_bytes().get(self.at) == Some(&b']') {
                    self.at += 1;
                } else {
                    loop {
                        let next =
                            if matches!(context, JsonContext::Reader | JsonContext::ReaderElements)
                            {
                                JsonContext::ReaderElements
                            } else {
                                JsonContext::Other
                            };
                        self.value_in(next, depth + 1, opaque)?;
                        self.whitespace();
                        match self.text.as_bytes().get(self.at)? {
                            b']' => {
                                self.at += 1;
                                break;
                            }
                            b',' => self.at += 1,
                            _ => return None,
                        }
                    }
                }
                Some(JsonShape {
                    input_bytes: self.at - start,
                    ..Default::default()
                })
            }
            b'"' => {
                let token = self.string(opaque)?;
                let scalar = if matches!(context, JsonContext::Field) {
                    let price = if token.len()
                        > super::protocol::ReaderLimits::default().field_bytes * 6 + 32
                    {
                        // No valid 4 KiB decoded JSON string needs a larger
                        // literal. Reject before decoding its complete span.
                        super::protocol::OpaquePrice {
                            decoded_bytes: token.len(),
                            input_bytes: token.len(),
                            ..Default::default()
                        }
                    } else {
                        json_string_price(token, |_| {})?
                    };
                    Some(JsonReaderScalar {
                        tail: &self.text[start..],
                        token,
                        price,
                    })
                } else {
                    None
                };
                Some(JsonShape {
                    scalar,
                    reader: None,
                    input_bytes: self.at - start,
                })
            }
            b'-' | b'0'..=b'9' => {
                while self.text.as_bytes().get(self.at).is_some_and(|byte| {
                    matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                }) {
                    self.budget.charge(2, 4)?;
                    self.at += 1;
                }
                if opaque {
                    self.budget.opaque_bytes(32)?;
                }
                let scalar = if matches!(context, JsonContext::Field) {
                    // Non-arbitrary-precision Number output is bounded. Raw
                    // numeric scan/decoder scratch remains separately prepaid.
                    Some(JsonReaderScalar {
                        tail: &self.text[start..],
                        token: &self.text[start..self.at],
                        price: super::protocol::OpaquePrice {
                            decoded_bytes: 32,
                            json_bytes: 32,
                            percent_bytes: 96,
                            input_bytes: self.at - start,
                        },
                    })
                } else {
                    None
                };
                Some(JsonShape {
                    scalar,
                    reader: None,
                    input_bytes: self.at - start,
                })
            }
            b't' | b'f' | b'n' => {
                let literal = if self.text[start..].starts_with("true") {
                    "true"
                } else if self.text[start..].starts_with("false") {
                    "false"
                } else if self.text[start..].starts_with("null") {
                    "null"
                } else {
                    return None;
                };
                self.at += literal.len();
                Some(JsonShape {
                    input_bytes: self.at - start,
                    ..Default::default()
                })
            }
            _ => None,
        }?;
        if field_root {
            self.budget.opaque_input(self.at - start)?;
        }
        Some(shape)
    }
}

fn json_character(text: &str, at: &mut usize) -> Option<char> {
    let byte = *text.as_bytes().get(*at)?;
    if byte != b'\\' {
        let character = text[*at..].chars().next()?;
        *at += character.len_utf8();
        return (character > '\u{1f}').then_some(character);
    }
    *at += 1;
    let escape = *text.as_bytes().get(*at)?;
    *at += 1;
    Some(match escape {
        b'"' => '"',
        b'\\' => '\\',
        b'/' => '/',
        b'b' => '\u{08}',
        b'f' => '\u{0c}',
        b'n' => '\n',
        b'r' => '\r',
        b't' => '\t',
        b'u' => {
            let unit = u16::from_str_radix(text.get(*at..*at + 4)?, 16).ok()?;
            *at += 4;
            let scalar = if (0xd800..=0xdbff).contains(&unit) {
                if text.get(*at..*at + 2)? != "\\u" {
                    return None;
                }
                *at += 2;
                let low = u16::from_str_radix(text.get(*at..*at + 4)?, 16).ok()?;
                *at += 4;
                if !(0xdc00..=0xdfff).contains(&low) {
                    return None;
                }
                0x10000 + ((u32::from(unit) - 0xd800) << 10) + u32::from(low) - 0xdc00
            } else {
                u32::from(unit)
            };
            char::from_u32(scalar)?
        }
        _ => return None,
    })
}

fn json_string_price(
    token: &str,
    mut visit: impl FnMut(char),
) -> Option<super::protocol::OpaquePrice> {
    let mut price = super::protocol::OpaquePrice {
        input_bytes: token.len(),
        json_bytes: 2,
        ..Default::default()
    };
    let mut at = 1;
    let end = token.len().checked_sub(1)?;
    while at < end {
        let character = if token.as_bytes()[at] != b'\\' {
            let character = token[at..].chars().next()?;
            at += character.len_utf8();
            character
        } else {
            at += 1;
            let escape = *token.as_bytes().get(at)?;
            at += 1;
            match escape {
                b'"' => '"',
                b'\\' => '\\',
                b'/' => '/',
                b'b' => '\u{08}',
                b'f' => '\u{0c}',
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                b'u' => {
                    let unit = u16::from_str_radix(token.get(at..at + 4)?, 16).ok()?;
                    at += 4;
                    let scalar = if (0xd800..=0xdbff).contains(&unit) {
                        if token.get(at..at + 2)? != "\\u" {
                            return None;
                        }
                        at += 2;
                        let low = u16::from_str_radix(token.get(at..at + 4)?, 16).ok()?;
                        at += 4;
                        if !(0xdc00..=0xdfff).contains(&low) {
                            return None;
                        }
                        0x10000 + ((u32::from(unit) - 0xd800) << 10) + u32::from(low) - 0xdc00
                    } else {
                        u32::from(unit)
                    };
                    char::from_u32(scalar)?
                }
                _ => return None,
            }
        };
        price.character(character).ok()?;
        visit(character);
    }
    (at == end).then_some(price)
}

fn json_key_is(token: &str, name: &str) -> bool {
    let mut at = 0;
    let mut equal = true;
    json_string_price(token, |character| {
        equal &= character.is_ascii() && name.as_bytes().get(at) == Some(&(character as u8));
        at += 1;
    })
    .is_some()
        && equal
        && at == name.len()
}

fn borrowed_json_fields<'a>(
    json: &'a str,
    budget: &mut JsonInputBudget,
) -> Result<JsonFieldsScan<'a>> {
    let shape = JsonFieldWalk {
        text: json,
        at: 0,
        budget,
    }
    .value(JsonContext::Root, 0);
    budget.check()?;
    let complete = shape.is_some();
    Ok(JsonFieldsScan {
        fields: shape.and_then(|shape| shape.reader),
        complete,
    })
}

fn usable_strict_clock(fields: &JsonReaderFields<'_>) -> bool {
    let Some(clock) = fields.fields[0] else {
        return false;
    };
    if clock.token.starts_with('"') {
        return clock.price.decoded_bytes != 0 && !json_key_is(clock.token, "0");
    }
    // Numeric output is the original bounded Number spelling. Integer zero
    // spells "0"; floating zero spells "0.0" (or "-0.0") and remains usable.
    // The borrowed walk already prepaid this scalar's decoder/scratch work.
    serde_json::from_str::<serde_json::Number>(clock.token)
        .is_ok_and(|number| number.is_f64() || number.as_i64() != Some(0))
}

fn loose_scalar<'a>(
    html: &'a str,
    name: &str,
    budget: &mut JsonInputBudget,
) -> Result<Option<JsonReaderScalar<'a>>> {
    let marker = format!("\"{name}\"");
    let Some((_, tail)) = html.split_once(&marker) else {
        return Ok(None);
    };
    let Some((_, value)) = tail.split_once(':') else {
        return Ok(None);
    };
    let value = value.trim_start();
    let shape = JsonFieldWalk {
        text: value,
        at: 0,
        budget,
    }
    .value(JsonContext::Field, 0);
    budget.check()?;
    Ok(shape.and_then(|shape| shape.scalar))
}

fn admit_json_fields(fields: JsonReaderFields<'_>, budget: &mut JsonInputBudget) -> Result<()> {
    let mut prices = fields
        .fields
        .map(|field| field.map_or(Default::default(), |field| field.price));
    for (price, input) in prices.iter_mut().zip(fields.input_bytes) {
        price.input_bytes = price.input_bytes.max(input);
    }
    budget.selection(prices)
}

fn scalar(value: &Value) -> Result<Option<String>> {
    let price = match value {
        Value::String(text) => super::protocol::OpaquePrice::text(text)?,
        Value::Number(_) => super::protocol::OpaquePrice {
            decoded_bytes: 32,
            json_bytes: 32,
            percent_bytes: 96,
            input_bytes: 32,
        },
        _ => return Ok(None),
    };
    super::protocol::admit_reader_fields([price, Default::default(), Default::default()])?;
    let text = match value {
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    };
    #[cfg(test)]
    super::client::reader_resource_budget_tests::observe_scalar_copy(text.len());
    Ok(Some(text))
}

fn find_json_field(html: &str, name: &str, budget: &mut JsonInputBudget) -> Result<Option<String>> {
    let Some(raw) = loose_scalar(html, name, budget)? else {
        return Ok(None);
    };
    budget.selection([raw.price, Default::default(), Default::default()])?;
    let value = serde_json::Deserializer::from_str(raw.tail)
        .into_iter::<Value>()
        .next()
        .and_then(std::result::Result::ok);
    value.as_ref().map(scalar).transpose().map(Option::flatten)
}

pub fn extract_reader_session(html: &str) -> Result<ReaderSession> {
    let mut budget = JsonInputBudget::new(html.len())?;
    extract_reader_session_in(html, &mut budget)
}

fn extract_reader_session_in(html: &str, budget: &mut JsonInputBudget) -> Result<ReaderSession> {
    let json = html
        .split_once("window.__INITIAL_STATE__")
        .and_then(|(_, tail)| tail.split_once('='))
        .map(|(_, json)| json.trim_start());
    let scanned = json
        .map(|json| borrowed_json_fields(json, budget))
        .transpose()?;
    let complete = scanned.as_ref().is_some_and(|scanned| scanned.complete);
    let mut fields = scanned
        .and_then(|scanned| scanned.fields)
        .unwrap_or_default();
    for (index, name) in ["psvts", "pclts", "token"].iter().enumerate() {
        if fields.fields[index].is_none() {
            fields.fields[index] = loose_scalar(html, name, budget)?;
            if let Some(field) = fields.fields[index] {
                fields.input_bytes[index] = field.price.input_bytes;
            }
        }
    }
    admit_json_fields(fields, budget)?;
    // A malformed walk keeps all spent reservations, but cannot authorize a
    // second heavy JSON decode. Original loose scalar fallback remains below.
    let state = json
        .filter(|_| complete)
        .and_then(|json| {
            #[cfg(test)]
            super::client::reader_resource_budget_tests::observe_state_decode();
            serde_json::Deserializer::from_str(json.trim_start())
                .into_iter::<Value>()
                .next()
        })
        .and_then(std::result::Result::ok);
    let reader = state.as_ref().and_then(|s| s.get("reader"));
    let mut get = |name| -> Result<Option<String>> {
        let selected = reader
            .and_then(|reader| reader.get(name))
            .map(scalar)
            .transpose()?
            .flatten();
        if selected.is_some() {
            Ok(selected)
        } else {
            find_json_field(html, name, budget)
        }
    };
    let psvts = get("psvts")?
        .filter(|s| !s.is_empty())
        .ok_or(SourceError::InvalidResponse("reader_session"))?;
    let session = ReaderSession {
        psvts,
        pclts: get("pclts")?,
        token: get("token")?,
        access: ReaderAccess::default(),
    };
    session.validate_opaque()?;
    Ok(session)
}

/// Permission evidence must come from one actual script assignment, never a
/// loose search through book text, JSON strings, or unrelated HTML fields.
pub(super) fn initial_reader_state(html: &str) -> Result<Option<Value>> {
    let mut budget = JsonInputBudget::new(html.len())?;
    initial_reader_state_in(html, &mut budget)
}

#[derive(Clone, Copy)]
enum PositionJsonContext {
    Root,
    Field(super::ReadingPositionLimitKind),
    Other,
}

/// This borrowing pass never constructs a Value, field String, or array. A
/// malformed suffix does not refund already observed field or scratch fees.
struct PositionJsonWalk<'a, 'b> {
    text: &'a str,
    at: usize,
    budget: &'b mut super::protocol::ReadingPositionBudget,
}

impl<'a> PositionJsonWalk<'a, '_> {
    fn whitespace(&mut self) {
        while self
            .text
            .as_bytes()
            .get(self.at)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.at += 1;
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let mut code = 0u32;
        for _ in 0..4 {
            let digit = match *self.text.as_bytes().get(self.at)? {
                b'0'..=b'9' => u32::from(self.text.as_bytes()[self.at] - b'0'),
                b'a'..=b'f' => u32::from(self.text.as_bytes()[self.at] - b'a') + 10,
                b'A'..=b'F' => u32::from(self.text.as_bytes()[self.at] - b'A') + 10,
                _ => return None,
            };
            code = code * 16 + digit;
            self.at += 1;
        }
        Some(code)
    }

    fn string(
        &mut self,
        field: Option<super::ReadingPositionLimitKind>,
    ) -> std::result::Result<Option<&'a str>, super::ReadingPositionLimitError> {
        let start = self.at;
        if self.text.as_bytes().get(self.at) != Some(&b'"') {
            return Ok(None);
        }
        self.at += 1;
        let mut decoded = 0usize;
        loop {
            let Some(byte) = self.text.as_bytes().get(self.at).copied() else {
                return Ok(None);
            };
            if byte == b'"' {
                self.at += 1;
                if let Some(kind) = field {
                    self.budget.field(kind, decoded)?;
                }
                return Ok(Some(&self.text[start..self.at]));
            }
            let width = if byte == b'\\' {
                self.at += 1;
                let Some(escape) = self.text.as_bytes().get(self.at).copied() else {
                    return Ok(None);
                };
                self.at += 1;
                match escape {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => 1,
                    b'u' => {
                        let Some(first) = self.hex4() else {
                            return Ok(None);
                        };
                        let code = if (0xd800..=0xdbff).contains(&first) {
                            if self.text.as_bytes().get(self.at..self.at + 2) != Some(&b"\\u"[..]) {
                                return Ok(None);
                            }
                            self.at += 2;
                            let Some(second) =
                                self.hex4().filter(|code| (0xdc00..=0xdfff).contains(code))
                            else {
                                return Ok(None);
                            };
                            0x10000 + ((first - 0xd800) << 10) + second - 0xdc00
                        } else {
                            first
                        };
                        let Some(character) = char::from_u32(code) else {
                            return Ok(None);
                        };
                        character.len_utf8()
                    }
                    _ => return Ok(None),
                }
            } else {
                if byte < 0x20 {
                    return Ok(None);
                }
                let Some(character) = self.text[self.at..].chars().next() else {
                    return Ok(None);
                };
                self.at += character.len_utf8();
                character.len_utf8()
            };
            decoded = decoded.saturating_add(width);
            if let Some(kind) = field {
                self.budget.check_field(kind, decoded)?;
            }
        }
    }

    fn key_field(token: &str) -> Option<super::ReadingPositionLimitKind> {
        for (key, kind) in [
            ("chapter_uid", super::ReadingPositionLimitKind::Uid),
            ("summary", super::ReadingPositionLimitKind::Summary),
            ("device", super::ReadingPositionLimitKind::Device),
            ("chapter_index", super::ReadingPositionLimitKind::Scalar),
            ("chapter_offset", super::ReadingPositionLimitKind::Scalar),
            ("progress", super::ReadingPositionLimitKind::Scalar),
            ("updated_at", super::ReadingPositionLimitKind::Scalar),
        ] {
            if json_key_is(token, key) {
                return Some(kind);
            }
        }
        None
    }

    fn value(
        &mut self,
        context: PositionJsonContext,
        depth: usize,
    ) -> std::result::Result<Option<()>, super::ReadingPositionLimitError> {
        self.whitespace();
        if depth > 128 {
            return Ok(None);
        }
        self.budget.node()?;
        let Some(byte) = self.text.as_bytes().get(self.at).copied() else {
            return Ok(None);
        };
        match byte {
            b'{' => {
                self.at += 1;
                self.whitespace();
                if self.text.as_bytes().get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Ok(Some(()));
                }
                loop {
                    self.whitespace();
                    let Some(key) = self.string(None)? else {
                        return Ok(None);
                    };
                    self.whitespace();
                    if self.text.as_bytes().get(self.at) != Some(&b':') {
                        return Ok(None);
                    }
                    self.at += 1;
                    let child = if matches!(context, PositionJsonContext::Root) {
                        Self::key_field(key)
                            .map_or(PositionJsonContext::Other, PositionJsonContext::Field)
                    } else {
                        PositionJsonContext::Other
                    };
                    if self.value(child, depth + 1)?.is_none() {
                        return Ok(None);
                    }
                    self.whitespace();
                    match self.text.as_bytes().get(self.at) {
                        Some(b'}') => {
                            self.at += 1;
                            break;
                        }
                        Some(b',') => self.at += 1,
                        _ => return Ok(None),
                    }
                }
                Ok(Some(()))
            }
            b'[' => {
                self.at += 1;
                self.whitespace();
                if self.text.as_bytes().get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(Some(()));
                }
                let mut index = 0usize;
                loop {
                    let child = if matches!(context, PositionJsonContext::Root) && index < 7 {
                        PositionJsonContext::Field(match index {
                            0 => super::ReadingPositionLimitKind::Uid,
                            4 => super::ReadingPositionLimitKind::Summary,
                            6 => super::ReadingPositionLimitKind::Device,
                            _ => super::ReadingPositionLimitKind::Scalar,
                        })
                    } else {
                        PositionJsonContext::Other
                    };
                    if self.value(child, depth + 1)?.is_none() {
                        return Ok(None);
                    }
                    index = index.saturating_add(1);
                    self.whitespace();
                    match self.text.as_bytes().get(self.at) {
                        Some(b']') => {
                            self.at += 1;
                            break;
                        }
                        Some(b',') => self.at += 1,
                        _ => return Ok(None),
                    }
                }
                Ok(Some(()))
            }
            b'"' => {
                let field = if let PositionJsonContext::Field(kind) = context {
                    Some(kind)
                } else {
                    None
                };
                Ok(self.string(field)?.map(|_| ()))
            }
            b'-' | b'0'..=b'9' => {
                let start = self.at;
                while self.text.as_bytes().get(self.at).is_some_and(|byte| {
                    matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                }) {
                    self.at += 1;
                }
                if matches!(context, PositionJsonContext::Field(_)) {
                    self.budget.scalar(self.at - start)?;
                }
                Ok(Some(()))
            }
            b't' | b'f' | b'n' => {
                for literal in ["true", "false", "null"] {
                    if self.text[self.at..].starts_with(literal) {
                        self.at += literal.len();
                        return Ok(Some(()));
                    }
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

pub(super) fn decode_reading_position_setting(
    input: &str,
) -> std::result::Result<super::ReadingPosition, super::ReadingPositionSettingError> {
    let mut budget = super::protocol::ReadingPositionBudget::new()?;
    budget.json_input(input.len())?;
    #[cfg(test)]
    super::client::position_budget_tests::observe_setting_scan();
    let _ = PositionJsonWalk {
        text: input,
        at: 0,
        budget: &mut budget,
    }
    .value(PositionJsonContext::Root, 0)?;
    #[cfg(test)]
    super::client::position_budget_tests::observe_setting_decode();
    let position: super::ReadingPosition = serde_json::from_str(input)?;
    #[cfg(test)]
    {
        super::client::position_budget_tests::observe_materialization();
        for value in [&position.chapter_uid, &position.summary, &position.device] {
            super::client::position_budget_tests::observe_field_copy(value.len());
        }
    }
    Ok(position)
}

fn initial_reader_state_in(html: &str, budget: &mut JsonInputBudget) -> Result<Option<Value>> {
    let document = Html::parse_document(html);
    let scripts = Selector::parse("script").expect("The static script selector is valid");
    let mut state = None;
    for script in document.select(&scripts) {
        if script.value().attr("src").is_some() {
            continue;
        }
        if let Some(kind) = script.value().attr("type") {
            let kind = kind.trim();
            if !["text/javascript", "application/javascript", "module"]
                .iter()
                .any(|expected| kind.eq_ignore_ascii_case(expected))
            {
                continue;
            }
        }
        let mut parts = script.text();
        let Some(first) = parts.next() else {
            continue;
        };
        let text = if parts.next().is_none() {
            Cow::Borrowed(first)
        } else {
            // The HTML sink normally coalesces script text. Preserve the old
            // concatenation contract for multiple segments under a separate
            // complete temporary-copy allowance.
            let bytes = script
                .text()
                .try_fold(0usize, |bytes, part| bytes.checked_add(part.len()))
                .ok_or(SourceError::ResponseTooLarge)?;
            super::protocol::admit_reader_script_copy(bytes)?;
            let metadata = bytes.checked_mul(4).ok_or(SourceError::ResponseTooLarge)?;
            let work = bytes.checked_mul(2).ok_or(SourceError::ResponseTooLarge)?;
            let _ = budget.charge(metadata, work);
            budget.check()?;
            #[cfg(test)]
            super::client::reader_resource_budget_tests::observe_script_copy();
            Cow::Owned(script.text().collect::<String>())
        };
        let Some(after_name) = text.trim_start().strip_prefix("window.__INITIAL_STATE__") else {
            continue;
        };
        let Some(json) = after_name.trim_start().strip_prefix('=') else {
            return Ok(None);
        };
        if state.is_some() {
            return Ok(None);
        }
        let json = json.trim_start();
        let scanned = borrowed_json_fields(json, budget)?;
        if !scanned.complete {
            return Ok(None);
        }
        let fields = scanned.fields.unwrap_or_default();
        if !usable_strict_clock(&fields) {
            // No strong Reader can use this assignment. Enter the existing
            // legacy/loose path before owning an otherwise unused full Value;
            // its actual first loose candidates must pass their own gates.
            return Ok(None);
        }
        admit_json_fields(fields, budget)?;
        #[cfg(test)]
        super::client::reader_resource_budget_tests::observe_state_decode();
        let mut values = serde_json::Deserializer::from_str(json).into_iter::<Value>();
        let Some(parsed) = values.next().and_then(std::result::Result::ok) else {
            return Ok(None);
        };
        let tail = json[values.byte_offset()..].trim_start();
        if (!tail.is_empty() && !tail.starts_with(';')) || tail.contains("window.__INITIAL_STATE__")
        {
            return Ok(None);
        }
        if !parsed.is_object() {
            return Ok(None);
        }
        state = Some(parsed);
    }
    Ok(state)
}

pub(super) fn extract_reader_session_with_access(
    html: &str,
    book_id: &str,
    chapter_uid: Option<&str>,
    expected_vid: Option<&str>,
    now: i64,
) -> Result<ReaderSession> {
    let mut budget = JsonInputBudget::new(html.len())?;
    if let Some(state) = initial_reader_state_in(html, &mut budget)? {
        if let Some(reader) = state.get("reader").and_then(Value::as_object) {
            let psvts = reader
                .get("psvts")
                .map(scalar)
                .transpose()?
                .flatten()
                .filter(|value| !value.is_empty() && value != "0");
            if let Some(psvts) = psvts {
                let raw_client_clock = reader.get("pclts");
                // Public SSR normally starts this client-owned value at zero.
                // Missing/null/blank values use one local fallback; malformed
                // containers cannot authorize content or borrow another script's clock.
                let client_clock_shape_valid = match raw_client_clock {
                    None | Some(Value::Null | Value::String(_)) => true,
                    Some(Value::Number(number)) => number
                        .as_f64()
                        .is_some_and(|number| number.is_finite() && number >= 0.0),
                    _ => false,
                };
                let access = if client_clock_shape_valid {
                    chapter_uid
                        .map(|chapter_uid| {
                            ReaderAccess::from_state(
                                &state,
                                book_id,
                                chapter_uid,
                                expected_vid,
                                now,
                            )
                        })
                        .unwrap_or_default()
                } else {
                    ReaderAccess::default()
                };
                return Ok(ReaderSession {
                    psvts,
                    pclts: raw_client_clock.map(scalar).transpose()?.flatten(),
                    token: reader.get("token").map(scalar).transpose()?.flatten(),
                    access,
                });
            }
        }
    }
    // Legacy clock fallbacks remain usable for reading, but they cannot borrow
    // permissions from a different state or an unrelated script.
    extract_reader_session_in(html, &mut budget)
}

fn normalize_catalog_admitted(payload: &Value, book_id: &str) -> Vec<RemoteChapter> {
    #[cfg(test)]
    catalog_field_gate_tests::observe_normalization();
    let mut chapters = Vec::new();
    for record in catalog_records(payload) {
        if record.get("bookId").and_then(borrowed_scalar).as_deref() != Some(book_id) {
            continue;
        }
        let Some(items) = record
            .get("updated")
            .or_else(|| record.get("chapterInfos"))
            .or_else(|| record.get("chapters"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for (index, item) in items.iter().enumerate() {
            let word_count = number(item.get("wordCount"));
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if word_count == 0 || title == "\u{5c01}\u{9762}" {
                continue;
            }
            let uid = item
                .get("chapterUid")
                .or_else(|| item.get("chapterId"))
                .and_then(borrowed_scalar)
                .map(Cow::into_owned)
                .unwrap_or_default();
            if uid.is_empty() {
                continue;
            }
            #[cfg(test)]
            super::client::reader_resource_budget_tests::observe_catalog_copy(
                item.get("tar").and_then(Value::as_str).map_or(0, str::len),
            );
            #[cfg(test)]
            catalog_field_gate_tests::observe_output_copy(
                uid.len()
                    + title.len()
                    + item.get("tar").and_then(Value::as_str).map_or(0, str::len),
            );
            chapters.push(RemoteChapter {
                uid,
                title: title.into(),
                index: item
                    .get("chapterIdx")
                    .map(|v| number(Some(v)))
                    .unwrap_or(index as u32),
                word_count,
                resource_url: item
                    .get("tar")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
            });
        }
    }
    chapters
}

#[cfg(test)]
pub fn normalize_catalog(payload: &Value, book_id: &str) -> Vec<RemoteChapter> {
    normalize_catalog_admitted(payload, book_id)
}

#[cfg(test)]
#[path = "catalog_field_gate_tests.rs"]
mod catalog_field_gate_tests;

pub(crate) fn number(value: Option<&Value>) -> u32 {
    value
        .and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
        })
        .filter(|number| number.is_finite())
        .unwrap_or_default()
        .clamp(0.0, f64::from(u32::MAX)) as u32
}

fn checked_body(response: &str) -> Result<&str> {
    if response.len() <= 32 {
        return Ok("");
    }
    if !response.is_char_boundary(32) {
        return Err(SourceError::Integrity);
    }
    let (checksum, body) = response.split_at(32);
    if checksum != format!("{:X}", md5::compute(body.as_bytes())) {
        return Err(SourceError::Integrity);
    }
    Ok(body)
}

fn swap_positions(encoded: &[u8]) -> Vec<usize> {
    let length = encoded.len();
    if length < 4 {
        return Vec::new();
    }
    if length < 11 {
        return vec![0, 2];
    }
    let n = 4.min(length.div_ceil(10));
    let tmp: String = encoded[length - n..]
        .iter()
        .rev()
        .map(|byte| {
            let mut radix_four = 0u64;
            let mut begun = false;
            for bit in (0..8).rev() {
                let digit = (byte >> bit) & 1;
                if digit != 0 {
                    begun = true;
                }
                if begun {
                    radix_four = radix_four * 4 + u64::from(digit);
                }
            }
            radix_four.to_string()
        })
        .collect();
    let m = length - n - 2;
    let step = m.to_string().len();
    let mut positions = Vec::new();
    let mut index = 0;
    while positions.len() < 10 && index + step < tmp.len() {
        positions.push(
            tmp[index..index + step]
                .parse::<usize>()
                .unwrap_or_default()
                % m,
        );
        positions.push(
            tmp[index + 1..(index + step + 1).min(tmp.len())]
                .parse::<usize>()
                .unwrap_or_default()
                % m,
        );
        index += step;
    }
    positions
}

fn decode_body(body: &str) -> Result<String> {
    if body.is_empty() {
        return Ok(String::new());
    }
    let mut encoded = body.as_bytes()[1..].to_vec();
    let positions = swap_positions(&encoded);
    for pair in positions.chunks_exact(2).rev() {
        for offset in (0..=1).rev() {
            let left = pair[1] + offset;
            let right = pair[0] + offset;
            if left >= encoded.len() || right >= encoded.len() {
                return Err(SourceError::Integrity);
            }
            encoded.swap(left, right);
        }
    }
    let mut filtered: Vec<u8> = encoded
        .into_iter()
        .filter_map(|b| match b {
            b'-' => Some(b'+'),
            b'_' => Some(b'/'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' | b'=' => Some(b),
            _ => None,
        })
        .collect();
    while filtered.len() % 4 != 0 {
        filtered.push(b'=');
    }
    let data = base64::engine::general_purpose::STANDARD
        .decode(filtered)
        .map_err(|_| SourceError::InvalidResponse("chapter_base64"))?;
    String::from_utf8(data).map_err(|_| SourceError::InvalidResponse("chapter_utf8"))
}

pub fn decode_shards(shards: &[&str]) -> Result<String> {
    let body = shards
        .iter()
        .map(|shard| checked_body(shard))
        .collect::<Result<Vec<_>>>()?
        .join("");
    decode_body(&body)
}

#[cfg(test)]
pub(crate) fn fixture_text_shards(text: &str) -> (String, String) {
    let mut encoded = base64::engine::general_purpose::STANDARD
        .encode(text)
        .into_bytes();
    for pair in swap_positions(&encoded).chunks_exact(2) {
        for offset in 0..=1 {
            encoded.swap(pair[0] + offset, pair[1] + offset);
        }
    }
    let body = format!("a{}", String::from_utf8(encoded).unwrap());
    let (first, second) = body.split_at(body.len() / 2);
    let checksum = |body: &str| format!("{:X}{body}", md5::compute(body.as_bytes()));
    (checksum(first), checksum(second))
}

pub fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TextHtmlLimits {
    pub output_bytes: usize,
    pub metadata_bytes: usize,
    pub work_bytes: usize,
}

impl Default for TextHtmlLimits {
    fn default() -> Self {
        Self {
            output_bytes: 8 * 1024 * 1024,
            metadata_bytes: 64 * 1024 * 1024,
            work_bytes: 128 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TextHtmlUsage {
    pub paragraphs: usize,
    pub output_bytes: usize,
    pub metadata_bytes: usize,
    pub work_bytes: usize,
}

/// Keep the existing lines/blank/escaping contract without per-line Strings,
/// repeated replace buffers, a paragraph Vec, or a second joined output.
pub(super) fn paragraph_html_checked(text: &str) -> Result<String> {
    let maximum = TextHtmlLimits::default();
    #[cfg(test)]
    let selected = super::client::reader_resource_budget_tests::text_limits();
    #[cfg(not(test))]
    let selected = maximum;
    let limits = TextHtmlLimits {
        output_bytes: selected.output_bytes.min(maximum.output_bytes),
        metadata_bytes: selected.metadata_bytes.min(maximum.metadata_bytes),
        work_bytes: selected.work_bytes.min(maximum.work_bytes),
    };
    if text
        .len()
        .checked_mul(4)
        .is_none_or(|bytes| bytes > limits.work_bytes)
        || text
            .len()
            .checked_add(128 + std::mem::size_of::<String>())
            .is_none_or(|bytes| bytes > limits.metadata_bytes)
    {
        return Err(SourceError::ResponseTooLarge);
    }
    let mut usage = TextHtmlUsage::default();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        usage.output_bytes = usage
            .output_bytes
            .checked_add(7 + usize::from(usage.paragraphs != 0))
            .ok_or(SourceError::ResponseTooLarge)?;
        usage.paragraphs = usage
            .paragraphs
            .checked_add(1)
            .ok_or(SourceError::ResponseTooLarge)?;
        for character in line.chars() {
            let bytes = match character {
                '&' => 5,
                '<' | '>' => 4,
                '"' => 6,
                '\'' => 5,
                _ => character.len_utf8(),
            };
            usage.output_bytes = usage
                .output_bytes
                .checked_add(bytes)
                .ok_or(SourceError::ResponseTooLarge)?;
        }
    }
    usage.metadata_bytes = text
        .len()
        .checked_add(usage.output_bytes)
        .and_then(|bytes| bytes.checked_add(128 + std::mem::size_of::<String>()))
        .ok_or(SourceError::ResponseTooLarge)?;
    usage.work_bytes = text
        .len()
        .checked_mul(4)
        .and_then(|bytes| {
            usage
                .output_bytes
                .checked_mul(2)
                .and_then(|output| bytes.checked_add(output))
        })
        .and_then(|bytes| {
            usage
                .paragraphs
                .checked_mul(8)
                .and_then(|paragraphs| bytes.checked_add(paragraphs))
        })
        .ok_or(SourceError::ResponseTooLarge)?;
    #[cfg(test)]
    super::client::reader_resource_budget_tests::observe_text_preflight(usage);
    if usage.output_bytes > limits.output_bytes
        || usage.metadata_bytes > limits.metadata_bytes
        || usage.work_bytes > limits.work_bytes
    {
        return Err(SourceError::ResponseTooLarge);
    }
    let mut html = String::with_capacity(usage.output_bytes);
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        if !html.is_empty() {
            html.push('\n');
        }
        html.push_str("<p>");
        for character in line.chars() {
            match character {
                '&' => html.push_str("&amp;"),
                '<' => html.push_str("&lt;"),
                '>' => html.push_str("&gt;"),
                '"' => html.push_str("&quot;"),
                '\'' => html.push_str("&#39;"),
                _ => html.push(character),
            }
        }
        html.push_str("</p>");
    }
    debug_assert_eq!(html.len(), usage.output_bytes);
    #[cfg(test)]
    super::client::reader_resource_budget_tests::observe_text_output(html.len());
    Ok(html)
}

fn decode_entities(text: &str) -> String {
    decode_entities_with_probe(text, || {})
}

fn decode_entities_with_probe(text: &str, mut probe: impl FnMut()) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some(index) = rest.find('&') {
        result.push_str(&rest[..index]);
        let entity = &rest[index + 1..];
        // The existing entity contract accepts a delimiter at byte 0 through
        // 12. Inspect only those bytes, without slicing a Unicode scalar at
        // the window boundary or searching the complete remaining document.
        let Some(end) = entity.bytes().take(13).position(|byte| {
            probe();
            byte == b';'
        }) else {
            result.push('&');
            rest = entity;
            continue;
        };
        let name = &entity[..end];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            "mdash" => Some('\u{2014}'),
            "hellip" => Some('\u{2026}'),
            _ if name.starts_with("#x") || name.starts_with("#X") => {
                u32::from_str_radix(&name[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if name.starts_with('#') => name[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        if let Some(ch) = ch {
            result.push(ch);
        } else {
            result.push('&');
            result.push_str(name);
            result.push(';');
        }
        rest = &entity[end + 1..];
    }
    result.push_str(rest);
    result
}

/// Extracts reading text, never executes or displays remote scripts.
pub fn html_to_text(html: &str) -> String {
    let mut result = String::new();
    let mut rest = html;
    let mut suppressed: Option<String> = None;
    while let Some(start) = rest.find('<') {
        if suppressed.is_none() {
            result.push_str(&decode_entities(&rest[..start]));
        }
        let Some(end_relative) = rest[start..].find('>') else {
            break;
        };
        let end = start + end_relative;
        let tag = rest[start + 1..end].trim().to_ascii_lowercase();
        let name = tag
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_end_matches('/');
        match name {
            "script" | "style" | "head" => suppressed = Some(name.into()),
            "/script" | "/style" | "/head" if suppressed.as_deref() == Some(&name[1..]) => {
                suppressed = None
            }
            "br" | "/p" | "/div" | "/h1" | "/h2" | "/h3" | "/li" | "/blockquote"
                if suppressed.is_none() =>
            {
                result.push('\n')
            }
            _ => {}
        }
        rest = &rest[end + 1..];
    }
    if suppressed.is_none() {
        result.push_str(&decode_entities(rest));
    }
    result
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "entity_work_budget_tests.rs"]
mod entity_work_budget_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn access_state_fixture() -> Value {
        serde_json::json!({
            "user": { "vid": "fixture-account" },
            "reader": {
                "bookId": "fixture-book",
                "bookInfo": { "bookId": "fixture-book", "paid": 1, "payType": 1 },
                "chapterInfos": [{ "chapterUid": "fixture-chapter", "chapterIdx": 1, "wordCount": 20 }],
                "currentChapter": { "chapterUid": "fixture-chapter" },
                "psvts": "fixture-server-clock",
                "pclts": "fixture-client-clock",
                "token": "fixture-reader-token"
            }
        })
    }

    fn reader_with_access(html: &str) -> ReaderSession {
        extract_reader_session_with_access(
            html,
            "fixture-book",
            Some("fixture-chapter"),
            Some("fixture-account"),
            2_000_000_000,
        )
        .unwrap()
    }

    #[test]
    fn access_and_reader_clocks_share_the_same_executable_state_assignment() {
        let state = access_state_fixture();
        let html = format!(
            "<script type='application/json'>window.__INITIAL_STATE__={{\"reader\":{{\"psvts\":\"fixture-decoy-clock\",\"pclts\":\"fixture-decoy-client\"}}}};</script><script>window.__INITIAL_STATE__={state};</script>"
        );
        let session = reader_with_access(&html);
        assert_eq!(session.psvts, "fixture-server-clock");
        assert_eq!(session.pclts.as_deref(), Some("fixture-client-clock"));
        assert_eq!(session.token.as_deref(), Some("fixture-reader-token"));
        assert_eq!(
            session
                .access
                .status_for_text("A complete synthetic passage."),
            crate::domain::ContentStatus::Full
        );
        let debug = format!("{session:?}");
        assert!(!debug.contains("fixture"));
    }

    #[test]
    fn missing_strict_server_clock_cannot_borrow_permissions_for_legacy_fallbacks() {
        let mut state = access_state_fixture();
        state["reader"].as_object_mut().unwrap().remove("psvts");
        let html = format!(
            "<script>var fixture={{\"psvts\":\"fixture-fallback-clock\"}};</script><script>window.__INITIAL_STATE__={state};</script>"
        );
        let session = reader_with_access(&html);
        assert_eq!(session.psvts, "fixture-fallback-clock");
        assert_eq!(
            session
                .access
                .status_for_text("A complete synthetic passage."),
            crate::domain::ContentStatus::Unknown
        );
    }

    #[test]
    fn normal_optional_client_clock_placeholders_preserve_strict_permission_evidence() {
        for placeholder in [
            None,
            Some(Value::Null),
            Some(serde_json::json!(0)),
            Some(serde_json::json!(0.0)),
            Some(serde_json::json!("")),
            Some(serde_json::json!(" \t")),
            Some(serde_json::json!("0")),
            Some(serde_json::json!("0.0")),
        ] {
            let mut state = access_state_fixture();
            let reader = state["reader"].as_object_mut().unwrap();
            match placeholder {
                Some(value) => {
                    reader.insert("pclts".into(), value);
                }
                None => {
                    reader.remove("pclts");
                }
            }
            reader.insert("token".into(), serde_json::json!(""));
            let html = format!("<script>window.__INITIAL_STATE__={state};</script>");
            let mut session = reader_with_access(&html);
            assert_eq!(session.psvts, "fixture-server-clock");
            assert_eq!(session.token.as_deref(), Some(""));
            assert_eq!(
                session
                    .access
                    .status_for_text("A complete synthetic passage."),
                crate::domain::ContentStatus::Full
            );
            session.ensure_reader_clock(100).unwrap();
            let materialized = super::super::protocol::encoded_id("100");
            assert_eq!(session.pclts.as_deref(), Some(materialized.as_str()));
            session.ensure_reader_clock(101).unwrap();
            assert_eq!(session.pclts.as_deref(), Some(materialized.as_str()));
        }
    }

    #[test]
    fn a_loose_client_clock_cannot_replace_the_strict_states_local_fallback() {
        for placeholder in [None, Some(serde_json::json!(0))] {
            let mut state = access_state_fixture();
            match placeholder {
                Some(value) => {
                    state["reader"]["pclts"] = value;
                }
                None => {
                    state["reader"].as_object_mut().unwrap().remove("pclts");
                }
            }
            let html = format!(
                "<script>var fixture={{\"pclts\":\"fixture-outside-client-clock\"}};</script><script>window.__INITIAL_STATE__={state};</script>"
            );
            let mut session = reader_with_access(&html);
            assert_ne!(
                session.pclts.as_deref(),
                Some("fixture-outside-client-clock")
            );
            session.ensure_reader_clock(100).unwrap();
            assert_eq!(
                session.pclts.as_deref(),
                Some(super::super::protocol::encoded_id("100").as_str())
            );
            assert_eq!(
                session
                    .access
                    .status_for_text("A complete synthetic passage."),
                crate::domain::ContentStatus::Full
            );
        }
    }

    #[test]
    fn malformed_client_clock_fields_do_not_borrow_permission_evidence() {
        for malformed in [
            serde_json::json!({}),
            serde_json::json!([]),
            serde_json::json!(true),
            serde_json::json!(-1),
        ] {
            let mut state = access_state_fixture();
            state["reader"]["pclts"] = malformed;
            let html = format!(
                "<script>var fixture={{\"pclts\":\"fixture-outside-client-clock\"}};</script><script>window.__INITIAL_STATE__={state};</script>"
            );
            let session = reader_with_access(&html);
            assert_eq!(session.psvts, "fixture-server-clock");
            assert_ne!(
                session.pclts.as_deref(),
                Some("fixture-outside-client-clock")
            );
            assert_eq!(
                session
                    .access
                    .status_for_text("A complete synthetic passage."),
                crate::domain::ContentStatus::Unknown
            );
        }
    }

    #[test]
    fn duplicate_or_nonassignment_states_do_not_grant_content_access() {
        let state = access_state_fixture();
        for html in [
            format!(
                "<script>window.__INITIAL_STATE__={state};</script><script>window.__INITIAL_STATE__={state};</script>"
            ),
            format!(
                "<script>window.__INITIAL_STATE__={state};window.__INITIAL_STATE__={state};</script>"
            ),
            format!("<script>window.__INITIAL_STATE__={state} + {{}};</script>"),
        ] {
            assert!(initial_reader_state(&html).unwrap().is_none());
            assert_eq!(
                reader_with_access(&html)
                    .access
                    .status_for_text("A complete synthetic passage."),
                crate::domain::ContentStatus::Unknown
            );
        }
    }

    #[test]
    fn unrelated_html_permission_fields_cannot_fill_a_reader_snapshot() {
        let state = serde_json::json!({"reader": {"psvts":"fixture-server-clock", "pclts":"fixture-client-clock"}});
        let html = format!(
            "<div data-paid='1' data-book='fixture-book' data-vid='fixture-account'></div><script>window.__INITIAL_STATE__={state};</script>"
        );
        assert_eq!(
            reader_with_access(&html)
                .access
                .status_for_text("A complete synthetic passage."),
            crate::domain::ContentStatus::Unknown
        );
    }

    #[test]
    fn shard_integrity_is_enforced() {
        let body = "prefix";
        let shard = format!("{:X}{body}", md5::compute(body));
        assert_eq!(checked_body(&shard).unwrap(), body);
        assert!(checked_body(&("0".repeat(32) + body)).is_err());
    }

    #[test]
    fn reader_session_uses_json_without_executing_script() {
        let html = "<script>window.__INITIAL_STATE__ = {\"reader\":{\"psvts\":\"session\",\"pclts\":\"clock\"}};(function(){})()</script>";
        let state = extract_reader_session(html).unwrap();
        assert_eq!(state.psvts, "session");
        assert_eq!(state.pclts.as_deref(), Some("clock"));
    }

    #[test]
    fn html_text_omits_active_content_and_preserves_paragraphs() {
        assert_eq!(
            html_to_text(
                "<head><style>hidden</style></head><p>A &amp; B</p><script>hidden</script><p>&#x4E2D;</p>"
            ),
            "A & B\n\u{4e2d}"
        );
    }

    #[test]
    fn catalog_excludes_non_readable_chapters() {
        let payload = serde_json::json!({"data":[{"bookId":"book","updated":[{"chapterUid":1,"title":"Cover","wordCount":0},{"chapterUid":2,"title":"Chapter","wordCount":100}]}]});
        let rows = normalize_catalog(&payload, "book");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].uid, "2");
    }

    #[test]
    fn chapter_decoding_matches_independent_lua_reference_vectors() {
        for (shard, expected) in [
            ("D269BC8688903ACA9835F8AFD7260B38ak=SG", "Hi"),
            ("BA02DB5E44BA92C15EF01ABCBE051F2DaVsSGbG8=", "Hello"),
            (
                "69323AAA4B1AF35468CB1604CB26CFCDaSGVsWR8sIlYHJGlbciE=",
                "Hello, reader!",
            ),
            (
                "DF1103CA0CD3800B1FA3F842EB367223aPGh0bWw+PJvGZHk+T4A+UmVhZGVyIGZpeHR1cwPPHC9mUjwvYm9keuP8L2h0bWw+",
                "<html><body><p>Reader fixture.</p></body></html>",
            ),
        ] {
            assert_eq!(decode_shards(&[shard]).unwrap(), expected);
        }
        assert_eq!(
            decode_shards(&[
                "136D662D62D94B3AF8CEFB644E5DA3D8aSGVsWR",
                "F3BB6E9F13A09F4EC190181148A2520F8sIlYHJ",
                "66AF2A4DC1BFE79515769F2DB6AA08F9GlbciE=",
            ])
            .unwrap(),
            "Hello, reader!"
        );
    }
}
