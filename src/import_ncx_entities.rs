//! Borrowed NCX entity admission before the XML parser can expand its DOM.

use super::xml_work::Fields;
use super::{
    ImportLimitExceeded, MAX_ENTRIES, MAX_ENTRY_BYTES, MAX_PATH_BYTES, MAX_RETAINED_BYTES,
    MAX_WORK_BYTES, RetainedBudget,
};
use anyhow::{Result, bail};
use std::collections::HashMap;

const MAX_ENTITY_DEPTH: usize = 10;
const MAX_ENTITY_REFERENCES: usize = 256;

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub source_bytes: usize,
    pub declarations: usize,
    pub name_bytes: usize,
    pub expanded_bytes: usize,
    pub work_bytes: usize,
    pub metadata_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            source_bytes: (MAX_ENTRY_BYTES * 2) as usize,
            declarations: MAX_ENTRIES,
            name_bytes: MAX_PATH_BYTES,
            expanded_bytes: MAX_WORK_BYTES as usize,
            work_bytes: MAX_WORK_BYTES as usize,
            metadata_bytes: MAX_RETAINED_BYTES,
        }
    }
}

/// These are conservative byte forecasts, not a total DOM heap or time limit.
/// The input and every invoked literal replacement remain priced even when
/// their markup is borrowed, normalization is smaller, or parsing later fails.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Usage {
    pub declarations: usize,
    pub definitions: usize,
    pub root_references: usize,
    pub summary_scans: usize,
    pub expanded_bytes: usize,
    pub work_bytes: usize,
    pub metadata_bytes: usize,
    pub fields: Fields,
}

pub(super) fn preflight(xml: &str, budget: &RetainedBudget) -> Result<Usage> {
    #[cfg(test)]
    let limits = super::ncx_entity_budget_tests::configured_limits().unwrap_or_default();
    #[cfg(not(test))]
    let limits = Limits::default();
    preflight_with_limits(xml, budget, limits)
}

pub(super) fn preflight_with_limits(
    xml: &str,
    budget: &RetainedBudget,
    mut limits: Limits,
) -> Result<Usage> {
    limits.metadata_bytes = limits
        .metadata_bytes
        .min(MAX_RETAINED_BYTES.saturating_sub(budget.used));
    let mut gate = Gate {
        limits,
        usage: Usage::default(),
    };
    #[cfg(test)]
    super::ncx_entity_budget_tests::observe_preflight_start();
    let result =
        (|| -> Result<()> {
            if xml.len() > limits.source_bytes {
                return Err(ImportLimitExceeded("NCX input exceeds its source allowance").into());
            }
            gate.usage.expanded_bytes = bounded_add(
                0,
                xml.len(),
                limits.expanded_bytes,
                "NCX entity expansion exceeds its byte allowance",
            )?;
            // Prepay complete input passes before prolog and document scanning.
            // Unique replacement contexts and their downstream copy/lookup work
            // are additionally priced by the summaries below.
            gate.usage.work_bytes = bounded_add(
                0,
                checked_mul(
                    xml.len(),
                    3,
                    "NCX entity processing exceeds its work allowance",
                )?,
                limits.work_bytes,
                "NCX entity processing exceeds its work allowance",
            )?;
            let mut entities = Entities {
                gate: &mut gate,
                definitions: Vec::new(),
                first: HashMap::new(),
                name_bytes: 0,
                frames: Vec::new(),
            };
            let mut prolog = Cursor::new(xml);
            let content_start = prolog.prolog(&mut entities)?;
            entities.index()?;
            entities.gate.usage.fields = super::xml_work::scan_content(&xml[content_start..])?;
            let mut scanner = References::new(
                &xml[content_start..],
                ExpansionContext::Content,
                limits.name_bytes,
            );
            while let Some(reference) = scanner.next()? {
                match reference {
                    Reference::Character(_, _) => {}
                    Reference::Entity(name, context) => {
                        let lookup = entities.lookup_fee(name)?;
                        let id = entities.id(name)?;
                        let summary = entities.summary(id, context)?;
                        entities.gate.usage.root_references =
                            entities.gate.usage.root_references.checked_add(1).ok_or(
                                ImportLimitExceeded("NCX entity reference count overflow"),
                            )?;
                        entities.gate.usage.expanded_bytes = bounded_add(
                            entities.gate.usage.expanded_bytes,
                            summary.bytes,
                            limits.expanded_bytes,
                            "NCX entity expansion exceeds its byte allowance",
                        )?;
                        entities.gate.usage.work_bytes = bounded_add(
                            entities.gate.usage.work_bytes,
                            bounded_add(
                                lookup,
                                summary.work,
                                limits.work_bytes,
                                "NCX entity processing exceeds its work allowance",
                            )?,
                            limits.work_bytes,
                            "NCX entity processing exceeds its work allowance",
                        )?;
                        entities.gate.usage.fields.merge(summary.fields)?;
                    }
                }
            }
            Ok(())
        })();
    // All borrowed indices, cached summaries and frames were dropped with the
    // closure, before the caller starts DOM parsing. No retained-data refund
    // can accidentally release storage still owned by the parser.
    #[cfg(test)]
    super::ncx_entity_budget_tests::observe_preflight_finish(gate.usage, result.is_ok());
    result.map(|_| gate.usage)
}

fn checked_mul(value: usize, factor: usize, message: &'static str) -> Result<usize> {
    value
        .checked_mul(factor)
        .ok_or_else(|| ImportLimitExceeded(message).into())
}

fn bounded_add(
    previous: usize,
    amount: usize,
    maximum: usize,
    message: &'static str,
) -> Result<usize> {
    previous
        .checked_add(amount)
        .filter(|next| *next <= maximum)
        .ok_or_else(|| ImportLimitExceeded(message).into())
}

struct Gate {
    limits: Limits,
    usage: Usage,
}

impl Gate {
    fn metadata(&mut self, amount: usize) -> Result<()> {
        self.usage.metadata_bytes = bounded_add(
            self.usage.metadata_bytes,
            amount,
            self.limits.metadata_bytes,
            "NCX entity preflight metadata exceeds its retained allowance",
        )?;
        Ok(())
    }

    fn declaration(&mut self) -> Result<()> {
        self.usage.declarations = bounded_add(
            self.usage.declarations,
            1,
            self.limits.declarations,
            "NCX entity declarations exceed their count allowance",
        )?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum ExpansionContext {
    Content,
    Attribute,
}

impl ExpansionContext {
    fn slot(self) -> usize {
        match self {
            Self::Content => 0,
            Self::Attribute => 1,
        }
    }
}

#[derive(Clone, Copy)]
struct Summary {
    bytes: usize,
    work: usize,
    references: usize,
    depth: usize,
    fields: Fields,
}

impl Summary {
    fn literal(length: usize, limits: Limits) -> Result<Self> {
        Ok(Self {
            bytes: bounded_add(
                0,
                length,
                limits.expanded_bytes,
                "NCX entity expansion exceeds its byte allowance",
            )?,
            // Tokenization plus the merged temporary String and owned string
            // copy receive separate passes before any actual DOM allocation.
            work: bounded_add(
                0,
                checked_mul(
                    length,
                    3,
                    "NCX entity processing exceeds its work allowance",
                )?,
                limits.work_bytes,
                "NCX entity processing exceeds its work allowance",
            )?,
            references: 1,
            depth: 1,
            fields: Fields::default(),
        })
    }

    fn add(&mut self, child: Self, lookup: usize, limits: Limits) -> Result<()> {
        self.bytes = bounded_add(
            self.bytes,
            child.bytes,
            limits.expanded_bytes,
            "NCX entity expansion exceeds its byte allowance",
        )?;
        self.work = bounded_add(
            self.work,
            bounded_add(
                lookup,
                child.work,
                limits.work_bytes,
                "NCX entity processing exceeds its work allowance",
            )?,
            limits.work_bytes,
            "NCX entity processing exceeds its work allowance",
        )?;
        self.references = self
            .references
            .checked_add(child.references)
            .ok_or(ImportLimitExceeded("NCX entity reference count overflow"))?;
        self.depth = self.depth.max(
            child
                .depth
                .checked_add(1)
                .ok_or(ImportLimitExceeded("NCX entity depth overflow"))?,
        );
        // These match the locked parser's root-reference expansion limits.
        // Malformed navigation retains the ordinary heading fallback.
        if self.references > MAX_ENTITY_REFERENCES || self.depth > MAX_ENTITY_DEPTH {
            bail!("NCX entity reference exceeds the XML parser's nesting allowance");
        }
        self.fields.merge(child.fields)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
enum SummaryState {
    #[default]
    Empty,
    Computing,
    Done(Summary),
}

struct Definition<'a> {
    name: &'a str,
    value: &'a str,
    summaries: [SummaryState; 2],
}

struct Frame<'a> {
    id: usize,
    context: ExpansionContext,
    scanner: References<'a>,
    summary: Summary,
    waiting_lookup: Option<usize>,
}

struct Entities<'a, 'gate> {
    gate: &'gate mut Gate,
    definitions: Vec<Definition<'a>>,
    first: HashMap<&'a str, usize>,
    name_bytes: usize,
    frames: Vec<Frame<'a>>,
}

impl<'a> Entities<'a, '_> {
    fn literal(&mut self, name: &'a str, value: &'a str) -> Result<()> {
        // Price geometric Vec spare slots before retaining borrowed spans.
        // Duplicates still consume declaration, metadata and lookup forecasts.
        self.gate
            .metadata(2 * std::mem::size_of::<Definition<'_>>())?;
        self.name_bytes = self
            .name_bytes
            .checked_add(name.len())
            .ok_or(ImportLimitExceeded("NCX entity name bytes overflow"))?;
        if self.definitions.is_empty() {
            // Avoid Vec's minimum four-slot first growth before four records
            // have been admitted. Later geometric growth fits the doubled fee.
            self.definitions.reserve_exact(1);
        }
        self.definitions.push(Definition {
            name,
            value,
            summaries: [SummaryState::Empty; 2],
        });
        self.gate.usage.definitions = self
            .gate
            .usage
            .definitions
            .checked_add(1)
            .ok_or(ImportLimitExceeded("NCX entity definition count overflow"))?;
        Ok(())
    }

    fn index(&mut self) -> Result<()> {
        if self.definitions.is_empty() {
            // Plain/external-DTD navigation needs no entity heap reservation,
            // preserving the existing projection's retained-data allowance.
            return Ok(());
        }
        // Include spare buckets/control bytes and the small-map minimum.
        let slot = std::mem::size_of::<(&str, usize)>() + 2;
        let fixed = std::mem::size_of::<Entities<'_, '_>>()
            + MAX_ENTITY_DEPTH * std::mem::size_of::<Frame<'_>>()
            + 4 * slot;
        let map = checked_mul(
            self.definitions.len(),
            2 * slot,
            "NCX entity preflight metadata exceeds its retained allowance",
        )?;
        self.gate.metadata(bounded_add(
            fixed,
            map,
            self.gate.limits.metadata_bytes,
            "NCX entity preflight metadata exceeds its retained allowance",
        )?)?;
        // Count first, reserve once, and then hash each borrowed name once.
        // Growing the map incrementally would repeatedly hash long old names.
        self.first = HashMap::with_capacity(self.definitions.len());
        self.frames = Vec::with_capacity(MAX_ENTITY_DEPTH);
        for (id, definition) in self.definitions.iter().enumerate() {
            // roxmltree's entity table resolves the first matching declaration.
            self.first.entry(definition.name).or_insert(id);
        }
        Ok(())
    }

    fn lookup_fee(&self, name: &str) -> Result<usize> {
        if name.len() > self.gate.limits.name_bytes {
            return Err(ImportLimitExceeded("NCX entity name exceeds its byte allowance").into());
        }
        // The actual parser scans its declaration Vec for every reference.
        // Conservatively include every retained name, not only an early match.
        bounded_add(
            self.name_bytes,
            self.definitions
                .len()
                .checked_add(checked_mul(
                    name.len(),
                    2,
                    "NCX entity processing exceeds its work allowance",
                )?)
                .ok_or(ImportLimitExceeded(
                    "NCX entity processing exceeds its work allowance",
                ))?,
            self.gate.limits.work_bytes,
            "NCX entity processing exceeds its work allowance",
        )
    }

    fn id(&self, name: &str) -> Result<usize> {
        self.first
            .get(name)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("NCX references an undefined internal entity"))
    }

    fn push(
        &mut self,
        frames: &mut Vec<Frame<'a>>,
        id: usize,
        context: ExpansionContext,
    ) -> Result<()> {
        if frames.len() >= MAX_ENTITY_DEPTH {
            bail!("NCX entity reference exceeds the XML parser's nesting allowance");
        }
        let definition = &mut self.definitions[id];
        definition.summaries[context.slot()] = SummaryState::Computing;
        let mut summary = Summary::literal(definition.value.len(), self.gate.limits)?;
        if matches!(context, ExpansionContext::Content) {
            summary.fields = super::xml_work::scan_content(definition.value)?;
        }
        self.gate.usage.summary_scans = self
            .gate
            .usage
            .summary_scans
            .checked_add(1)
            .ok_or(ImportLimitExceeded("NCX entity summary count overflow"))?;
        frames.push(Frame {
            id,
            context,
            scanner: References::new(definition.value, context, self.gate.limits.name_bytes),
            summary,
            waiting_lookup: None,
        });
        Ok(())
    }

    fn summary(&mut self, id: usize, context: ExpansionContext) -> Result<Summary> {
        if let SummaryState::Done(summary) = self.definitions[id].summaries[context.slot()] {
            return Ok(summary);
        }
        let mut frames = std::mem::take(&mut self.frames);
        let result = (|| -> Result<Summary> {
            self.push(&mut frames, id, context)?;
            loop {
                let next = frames
                    .last_mut()
                    .expect("An entity frame is active")
                    .scanner
                    .next()?;
                match next {
                    None => {
                        let mut frame = frames.pop().expect("An entity frame is active");
                        if matches!(frame.context, ExpansionContext::Attribute) {
                            frame.summary.fields.attribute_replacement = frame
                                .summary
                                .fields
                                .attribute_replacement
                                .max(frame.summary.bytes);
                        }
                        self.definitions[frame.id].summaries[frame.context.slot()] =
                            SummaryState::Done(frame.summary);
                        if let Some(parent) = frames.last_mut() {
                            let lookup = parent
                                .waiting_lookup
                                .take()
                                .expect("A parent reference is waiting");
                            parent
                                .summary
                                .add(frame.summary, lookup, self.gate.limits)?;
                        } else {
                            return Ok(frame.summary);
                        }
                    }
                    Some(Reference::Character(character, character_context)) => {
                        if matches!(character_context, ExpansionContext::Attribute)
                            && character == '<'
                        {
                            bail!("NCX entity contains an invalid expanded attribute character");
                        }
                    }
                    Some(Reference::Entity(name, next_context)) => {
                        let lookup = self.lookup_fee(name)?;
                        let next_id = self.id(name)?;
                        match self.definitions[next_id].summaries[next_context.slot()] {
                            SummaryState::Done(summary) => frames.last_mut().unwrap().summary.add(
                                summary,
                                lookup,
                                self.gate.limits,
                            )?,
                            SummaryState::Computing => bail!("NCX entity references form a cycle"),
                            SummaryState::Empty => {
                                frames.last_mut().unwrap().waiting_lookup = Some(lookup);
                                self.push(&mut frames, next_id, next_context)?;
                            }
                        }
                    }
                }
            }
        })();
        frames.clear();
        self.frames = frames;
        result
    }
}

/// The scanner recognizes references only in text and quoted attributes.
/// Comments, CDATA, PI and quoted external identifiers never expand entities.
/// It preserves borrowed spans and moves forward without searching a tail
/// again for each ampersand or keeping a Vec of every dependency edge.
struct References<'a> {
    cursor: Cursor<'a>,
    context: ExpansionContext,
    in_tag: bool,
    quote: Option<u8>,
    maximum_name: usize,
}

enum Reference<'a> {
    Character(char, ExpansionContext),
    Entity(&'a str, ExpansionContext),
}

impl<'a> References<'a> {
    fn new(input: &'a str, context: ExpansionContext, maximum_name: usize) -> Self {
        Self {
            cursor: Cursor::new(input),
            context,
            in_tag: false,
            quote: None,
            maximum_name,
        }
    }

    fn next(&mut self) -> Result<Option<Reference<'a>>> {
        while let Some(byte) = self.cursor.byte() {
            if matches!(self.context, ExpansionContext::Attribute) {
                match byte {
                    b'<' => bail!("NCX entity contains an invalid attribute value"),
                    b'&' => return self.reference(ExpansionContext::Attribute).map(Some),
                    _ => self.cursor.position += 1,
                }
            } else if self.in_tag {
                match (self.quote, byte) {
                    (Some(quote), byte) if quote == byte => {
                        self.quote = None;
                        self.cursor.position += 1;
                    }
                    (Some(_), b'&') => {
                        return self.reference(ExpansionContext::Attribute).map(Some);
                    }
                    (Some(_), b'<') => bail!("NCX contains an invalid attribute value"),
                    (None, b'\'' | b'"') => {
                        self.quote = Some(byte);
                        self.cursor.position += 1;
                    }
                    (None, b'>') => {
                        self.in_tag = false;
                        self.cursor.position += 1;
                    }
                    _ => self.cursor.position += 1,
                }
            } else if byte == b'&' {
                return self.reference(ExpansionContext::Content).map(Some);
            } else if byte == b'<' {
                if self.cursor.starts(b"<!--") {
                    self.cursor.comment()?;
                } else if self.cursor.starts(b"<![CDATA[") {
                    self.cursor.cdata()?;
                } else if self.cursor.starts(b"<?") {
                    self.cursor.processing_instruction()?;
                } else if self.cursor.starts(b"<!") {
                    bail!("NCX contains unsupported XML declaration syntax");
                } else {
                    self.in_tag = true;
                    self.cursor.position += 1;
                }
            } else {
                self.cursor.position += 1;
            }
        }
        Ok(None)
    }

    fn reference(&mut self, context: ExpansionContext) -> Result<Reference<'a>> {
        self.cursor.position += 1;
        let reference = if self.cursor.byte() == Some(b'#') {
            self.cursor.position += 1;
            let radix = if self.cursor.byte() == Some(b'x') {
                self.cursor.position += 1;
                16
            } else {
                10
            };
            let start = self.cursor.position;
            while self.cursor.byte().is_some_and(|byte| {
                if radix == 16 {
                    byte.is_ascii_hexdigit()
                } else {
                    byte.is_ascii_digit()
                }
            }) {
                self.cursor.position += 1;
                if self.cursor.position - start > self.maximum_name {
                    return Err(ImportLimitExceeded(
                        "NCX entity reference exceeds its byte allowance",
                    )
                    .into());
                }
            }
            let value = u32::from_str_radix(&self.cursor.input[start..self.cursor.position], radix)
                .map_err(|_| anyhow::anyhow!("NCX has a malformed character reference"))?;
            let character = char::from_u32(value).unwrap_or('\u{fffd}');
            if !xml_character(character) {
                bail!("NCX has an invalid XML character reference");
            }
            Reference::Character(character, context)
        } else {
            let name = self.cursor.name(Some(self.maximum_name))?;
            match name {
                "amp" => Reference::Character('&', context),
                "lt" => Reference::Character('<', context),
                "gt" => Reference::Character('>', context),
                "apos" => Reference::Character('\'', context),
                "quot" => Reference::Character('"', context),
                _ => Reference::Entity(name, context),
            }
        };
        self.cursor.require(b';')?;
        Ok(reference)
    }
}

struct Cursor<'a> {
    input: &'a str,
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, position: 0 }
    }

    fn byte(&self) -> Option<u8> {
        self.input.as_bytes().get(self.position).copied()
    }

    fn starts(&self, value: &[u8]) -> bool {
        self.input.as_bytes()[self.position..].starts_with(value)
    }

    fn spaces(&mut self) {
        while self
            .byte()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.position += 1;
        }
    }

    fn required_spaces(&mut self) -> Result<()> {
        let previous = self.position;
        self.spaces();
        if previous == self.position {
            bail!("NCX has malformed XML declaration spacing");
        }
        Ok(())
    }

    fn require(&mut self, expected: u8) -> Result<()> {
        if self.byte() != Some(expected) {
            bail!("NCX has malformed XML declaration or reference syntax");
        }
        self.position += 1;
        Ok(())
    }

    fn delimited(&mut self, prefix: usize, delimiter: &str) -> Result<()> {
        self.position += prefix;
        let end = self.input[self.position..]
            .find(delimiter)
            .ok_or_else(|| anyhow::anyhow!("NCX has an unclosed XML declaration"))?;
        self.position += end + delimiter.len();
        Ok(())
    }

    fn comment(&mut self) -> Result<()> {
        let start = self.position + 4;
        self.delimited(4, "-->")?;
        let text = &self.input[start..self.position - 3];
        if text.contains("--")
            || text.ends_with('-')
            || text.chars().any(|character| !xml_character(character))
        {
            bail!("NCX has a malformed XML comment");
        }
        Ok(())
    }

    fn cdata(&mut self) -> Result<()> {
        let start = self.position + 9;
        self.delimited(9, "]]>")?;
        if self.input[start..self.position - 3]
            .chars()
            .any(|character| !xml_character(character))
        {
            bail!("NCX has invalid CDATA characters");
        }
        Ok(())
    }

    fn processing_instruction(&mut self) -> Result<()> {
        if self.starts(b"<?xml ") {
            bail!("NCX has an unexpected XML declaration");
        }
        self.position += 2;
        self.name(None)?;
        self.spaces();
        let start = self.position;
        self.delimited(0, "?>")?;
        if self.input[start..self.position - 2]
            .chars()
            .any(|character| !xml_character(character))
        {
            bail!("NCX has invalid processing instruction characters");
        }
        Ok(())
    }

    fn declaration_attribute(&mut self) -> Result<()> {
        self.name(None)?;
        self.spaces();
        self.require(b'=')?;
        self.spaces();
        let value = self.quoted()?;
        if value.contains('<') || value.chars().any(|character| !xml_character(character)) {
            bail!("NCX has an invalid XML declaration attribute");
        }
        Ok(())
    }

    fn declaration_spacing(&mut self) -> Result<()> {
        if !self.starts(b"?>")
            && !self
                .byte()
                .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        {
            bail!("NCX has malformed XML declaration spacing");
        }
        self.spaces();
        Ok(())
    }

    fn xml_declaration(&mut self) -> Result<()> {
        self.position += 5;
        self.required_spaces()?;
        if !self.starts(b"version") {
            bail!("NCX has no XML declaration version");
        }
        self.declaration_attribute()?;
        self.declaration_spacing()?;
        if self.starts(b"encoding") {
            self.declaration_attribute()?;
            self.declaration_spacing()?;
        }
        if self.starts(b"standalone") {
            self.declaration_attribute()?;
        }
        self.spaces();
        self.require(b'?')?;
        self.require(b'>')
    }

    fn quoted(&mut self) -> Result<&'a str> {
        let quote = self
            .byte()
            .filter(|byte| matches!(byte, b'\'' | b'"'))
            .ok_or_else(|| anyhow::anyhow!("NCX has an unquoted XML declaration value"))?;
        self.position += 1;
        let start = self.position;
        let end = self.input.as_bytes()[start..]
            .iter()
            .position(|byte| *byte == quote)
            .ok_or_else(|| anyhow::anyhow!("NCX has an unclosed XML declaration value"))?;
        self.position = start + end + 1;
        Ok(&self.input[start..start + end])
    }

    fn name(&mut self, maximum: Option<usize>) -> Result<&'a str> {
        let start = self.position;
        let mut characters = self.input[start..].chars();
        let first = characters
            .next()
            .filter(|character| xml_name_start(*character))
            .ok_or_else(|| anyhow::anyhow!("NCX has an invalid XML name"))?;
        self.position += first.len_utf8();
        for character in characters {
            if !xml_name_character(character) {
                break;
            }
            self.position += character.len_utf8();
            if maximum.is_some_and(|maximum| self.position - start > maximum) {
                return Err(
                    ImportLimitExceeded("NCX entity name exceeds its byte allowance").into(),
                );
            }
        }
        if maximum.is_some_and(|maximum| self.position - start > maximum) {
            return Err(ImportLimitExceeded("NCX entity name exceeds its byte allowance").into());
        }
        Ok(&self.input[start..self.position])
    }

    fn misc(&mut self) -> Result<()> {
        loop {
            self.spaces();
            if self.starts(b"<!--") {
                self.comment()?;
            } else if self.starts(b"<?") {
                self.processing_instruction()?;
            } else {
                return Ok(());
            }
        }
    }

    fn external_id(&mut self) -> Result<bool> {
        let public = self.starts(b"PUBLIC");
        if !public && !self.starts(b"SYSTEM") {
            return Ok(false);
        }
        self.position += 6;
        self.required_spaces()?;
        self.quoted()?;
        if public {
            self.required_spaces()?;
            self.quoted()?;
        }
        Ok(true)
    }

    fn prolog(&mut self, entities: &mut Entities<'a, '_>) -> Result<usize> {
        if self.starts(&[0xef, 0xbb, 0xbf]) {
            self.position += 3;
        }
        if self.starts(b"<?xml ") {
            self.xml_declaration()?;
        }
        self.misc()?;
        if !self.starts(b"<!DOCTYPE") {
            return Ok(self.position);
        }
        self.position += 9;
        self.required_spaces()?;
        self.name(None)?;
        self.spaces();
        self.external_id()?;
        self.spaces();
        if self.byte() == Some(b'>') {
            self.position += 1;
            return Ok(self.position);
        }
        self.require(b'[')?;
        loop {
            self.spaces();
            if self.starts(b"<!ENTITY") {
                self.entity(entities)?;
            } else if self.starts(b"<!--") {
                self.comment()?;
            } else if self.starts(b"<?") {
                self.processing_instruction()?;
            } else if self.starts(b"<!ELEMENT")
                || self.starts(b"<!ATTLIST")
                || self.starts(b"<!NOTATION")
            {
                // The locked tokenizer ignores these declarations through the
                // first closing '>'; no entity references are expanded here.
                self.delimited(0, ">")?;
            } else if self.byte() == Some(b']') {
                self.position += 1;
                self.spaces();
                self.require(b'>')?;
                return Ok(self.position);
            } else {
                bail!("NCX has malformed internal DTD syntax");
            }
        }
    }

    fn entity(&mut self, entities: &mut Entities<'a, '_>) -> Result<()> {
        entities.gate.declaration()?;
        self.position += 8;
        self.required_spaces()?;
        let parameter = self.byte() == Some(b'%');
        if parameter {
            self.position += 1;
            self.required_spaces()?;
        }
        let name = self.name(Some(entities.gate.limits.name_bytes))?;
        self.required_spaces()?;
        if self.byte().is_some_and(|byte| matches!(byte, b'\'' | b'"')) {
            let value = self.quoted()?;
            // The locked parser also retains literal parameter declarations
            // in the same first-match table; parameter references in the DTD
            // itself remain unsupported by its tokenizer.
            entities.literal(name, value)?;
        } else if self.external_id()? {
            // entity_resolver=None discards external definitions without
            // fetching their contents or installing a replacement string.
            if !parameter {
                self.spaces();
                if self.starts(b"NDATA") {
                    self.position += 5;
                    self.required_spaces()?;
                    self.name(None)?;
                }
            }
        } else {
            bail!("NCX has an invalid entity definition");
        }
        self.spaces();
        self.require(b'>')
    }
}

pub(super) fn xml_name_start(character: char) -> bool {
    matches!(character,
        ':' | '_' | 'A'..='Z' | 'a'..='z'
        | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}'
        | '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}'
        | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}'
        | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}' | '\u{10000}'..='\u{effff}'
    )
}

pub(super) fn xml_name_character(character: char) -> bool {
    xml_name_start(character)
        || matches!(character, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}')
}

fn xml_character(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
}
