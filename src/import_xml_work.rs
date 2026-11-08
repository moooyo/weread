//! Borrowed forecasts for the locked XML parser's name and namespace work.

use super::{ImportLimitExceeded, MAX_ENTRY_BYTES, MAX_NCX_NODES, MAX_PATH_BYTES, MAX_WORK_BYTES};
use anyhow::Result;

const MAX_FIELDS: usize = 64 * 1024;
const MAX_METADATA_BYTES: usize = 64 * 1024 * 1024;
const XML_NAMESPACE_BYTES: usize = "http://www.w3.org/XML/1998/namespace".len();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Container,
    Package,
    Encryption,
    Ncx,
}

impl Kind {
    #[cfg(test)]
    pub(super) fn slot(self) -> usize {
        match self {
            Self::Container => 0,
            Self::Package => 1,
            Self::Encryption => 2,
            Self::Ncx => 3,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub source_bytes: usize,
    pub fields: usize,
    pub name_bytes: usize,
    pub work_bytes: usize,
    pub metadata_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            source_bytes: (MAX_ENTRY_BYTES * 2) as usize,
            fields: MAX_FIELDS,
            name_bytes: MAX_PATH_BYTES,
            work_bytes: MAX_WORK_BYTES as usize,
            metadata_bytes: MAX_METADATA_BYTES,
        }
    }
}

/// Counts are merged in actual entity invocation contexts. Attribute-only
/// replacements supply a URI-length upper bound but never create fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Fields {
    pub elements: usize,
    pub closes: usize,
    pub nodes: usize,
    pub attributes: usize,
    pub attribute_pairs: usize,
    pub namespaces: usize,
    pub namespace_pairs: usize,
    pub namespace_tags: usize,
    pub qualified_attributes: usize,
    pub maximum_name: usize,
    pub namespace_value: usize,
    pub namespace_references: usize,
    pub attribute_replacement: usize,
    pub mixed_groups: bool,
}

impl Fields {
    pub(super) fn merge(&mut self, other: Self) -> Result<()> {
        self.elements = add(self.elements, other.elements)?;
        self.closes = add(self.closes, other.closes)?;
        self.nodes = add(self.nodes, other.nodes)?;
        self.attributes = add(self.attributes, other.attributes)?;
        self.attribute_pairs = add(self.attribute_pairs, other.attribute_pairs)?;
        self.namespaces = add(self.namespaces, other.namespaces)?;
        self.namespace_pairs = add(self.namespace_pairs, other.namespace_pairs)?;
        self.namespace_tags = add(self.namespace_tags, other.namespace_tags)?;
        self.qualified_attributes = add(self.qualified_attributes, other.qualified_attributes)?;
        self.maximum_name = self.maximum_name.max(other.maximum_name);
        self.namespace_value = self.namespace_value.max(other.namespace_value);
        self.namespace_references = self.namespace_references.max(other.namespace_references);
        self.attribute_replacement = self.attribute_replacement.max(other.attribute_replacement);
        self.mixed_groups |= other.mixed_groups;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Forecast {
    pub comparison_bytes: usize,
    pub metadata_bytes: usize,
    pub namespace_uri_bytes: usize,
    pub attribute_pairs: usize,
    pub namespace_pairs: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Usage {
    pub source_bytes: usize,
    pub fields: usize,
    pub work_bytes: usize,
    pub metadata_bytes: usize,
}

/// One imported book shares this allowance across all four XML documents.
/// Failed scans/parses do not return consumed work or metadata forecasts.
pub(super) struct Budget {
    limits: Limits,
    usage: Usage,
}

pub(super) struct Input {
    kind: Kind,
    less_than: usize,
    equal: usize,
}

impl Budget {
    pub(super) fn new() -> Self {
        #[cfg(test)]
        let limits = super::xml_work_tests::configured_limits().unwrap_or_default();
        #[cfg(not(test))]
        let limits = Limits::default();
        Self {
            limits,
            usage: Usage::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn with_limits(limits: Limits) -> Self {
        Self {
            limits,
            usage: Usage::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn usage(&self) -> Usage {
        self.usage
    }

    pub(super) fn input(&mut self, xml: &str, kind: Kind) -> Result<Input> {
        #[cfg(test)]
        super::xml_work_tests::observe_input(kind, xml.len());
        if xml.len() > self.limits.source_bytes {
            return Err(ImportLimitExceeded("XML source exceeds its byte allowance").into());
        }
        // The raw capacity counters and the borrowed field/quoted-value scan
        // are admitted before examining their complete input, without a Vec.
        self.usage.work_bytes = admit(
            self.usage.work_bytes,
            mul(xml.len(), 3)?,
            self.limits.work_bytes,
            "XML name processing exceeds its work allowance",
        )?;
        self.usage.source_bytes = add(self.usage.source_bytes, xml.len())?;
        let mut less_than = 0;
        let mut equal = 0;
        for byte in xml.bytes() {
            if byte == b'<' {
                less_than = add(less_than, 1)?;
            }
            if byte == b'=' {
                equal = add(equal, 1)?;
            }
        }
        #[cfg(test)]
        super::xml_work_tests::observe_input_admitted(kind, self.usage);
        Ok(Input {
            kind,
            less_than,
            equal,
        })
    }

    pub(super) fn fields(&mut self, input: Input, fields: Fields) -> Result<()> {
        let kind = input.kind;
        #[cfg(test)]
        super::xml_work_tests::observe_fields(kind, fields);
        #[cfg(not(test))]
        let _ = kind;
        if fields.maximum_name > self.limits.name_bytes {
            return Err(ImportLimitExceeded("XML name exceeds its byte allowance").into());
        }
        self.usage.fields = admit(
            self.usage.fields,
            fields.attributes,
            self.limits.fields,
            "XML fields exceed their count allowance",
        )?;
        let forecast = forecast(&input, fields)?;
        #[cfg(test)]
        super::xml_work_tests::observe_forecast(kind, forecast);
        self.usage.work_bytes = admit(
            self.usage.work_bytes,
            forecast.comparison_bytes,
            self.limits.work_bytes,
            "XML name processing exceeds its work allowance",
        )?;
        self.usage.metadata_bytes = admit(
            self.usage.metadata_bytes,
            forecast.metadata_bytes,
            self.limits.metadata_bytes,
            "XML parser metadata exceeds its forecast allowance",
        )?;
        #[cfg(test)]
        super::xml_work_tests::observe_admitted(kind, self.usage);
        Ok(())
    }

    pub(super) fn parse<'a>(
        &mut self,
        xml: &'a str,
        kind: Kind,
    ) -> Result<roxmltree::Document<'a>> {
        let input = self.input(xml, kind)?;
        let fields = scan_content(xml)?;
        self.fields(input, fields)?;
        observe_dom(kind);
        Ok(roxmltree::Document::parse(xml)?)
    }
}

pub(super) fn observe_dom(kind: Kind) {
    #[cfg(test)]
    super::xml_work_tests::observe_dom(kind);
    #[cfg(not(test))]
    let _ = kind;
}

fn forecast(input: &Input, fields: Fields) -> Result<Forecast> {
    let namespaces = add(fields.namespaces, 1)?;
    let names = add(fields.maximum_name.max(3), 1)?;
    let namespace_uri = if fields.namespaces == 0 {
        XML_NAMESPACE_BYTES
    } else {
        add(
            fields.namespace_value,
            mul(fields.namespace_references, fields.attribute_replacement)?,
        )?
        .max(XML_NAMESPACE_BYTES)
    };
    let expanded_name = sum(&[names, namespace_uri, 2])?;
    // The locked tokenizer can finish an entity fragment at EOF immediately
    // after a start QName or a complete attribute quote, without ElementEnd.
    // Its pending attributes/namespaces can then join the next outer tag.
    // Preserve that permissive behavior with whole-document pair bounds.
    let attribute_pairs = if fields.mixed_groups {
        pairs(fields.attributes)?
    } else {
        fields.attribute_pairs
    };
    let namespace_pairs = if fields.mixed_groups {
        pairs(fields.namespaces)?
    } else {
        fields.namespace_pairs
    };
    let duplicate_attributes = mul(attribute_pairs, expanded_name)?;
    let prefix_queries = add(fields.elements, fields.qualified_attributes)?;
    let prefix_lookup = mul(mul(prefix_queries, namespaces)?, names)?;
    let closing_names = mul(fields.closes, names)?;
    let duplicate_namespaces = mul(namespace_pairs, names)?;
    // resolve_namespaces immediately reuses the parent's range when no local
    // namespace was appended. Otherwise its inherited range and the growing
    // current range each contain at most all declarations plus implicit xml.
    // Namespace-looking fields that add no row are conservatively included.
    let propagation = mul(
        mul(mul(fields.namespace_tags, namespaces)?, namespaces)?,
        names,
    )?;
    // Each namespace registration searches at most all namespace values;
    // tuple comparison includes prefix and URI bytes. The additional four
    // units cover scalar/control work and moving u16 sorted-order indices.
    let sorted_namespaces = mul(
        mul(namespaces, namespaces)?,
        sum(&[names, namespace_uri, 4])?,
    )?;
    let comparison_bytes = sum(&[
        duplicate_attributes,
        prefix_lookup,
        closing_names,
        duplicate_namespaces,
        propagation,
        sorted_namespaces,
    ])?;

    // The locked parser first reserves from every raw '<' and '=', including
    // literals in DTD/quoted values. Conservative 128-byte row slots then
    // include geometric growth, expanded syntax/text rows and namespace data.
    // These are metadata forecasts, not all DOM payloads or allocator RSS.
    let node_slots = sum(&[input.less_than, mul(fields.nodes, 2)?, 4])?;
    let attribute_slots = sum(&[input.equal, mul(fields.attributes, 2)?, 16])?;
    let namespace_rows = mul(namespaces, 2)?;
    let rows = mul(sum(&[node_slots, attribute_slots, namespace_rows])?, 128)?;
    let namespace_refs = add(namespaces, mul(fields.namespace_tags, namespaces)?)?;
    let indices = mul(sum(&[mul(namespace_refs, 2)?, mul(namespaces, 2)?, 4])?, 4)?;
    let prefixes = mul(add(fields.elements, 1)?, 32)?;
    Ok(Forecast {
        comparison_bytes,
        metadata_bytes: sum(&[rows, indices, prefixes])?,
        namespace_uri_bytes: namespace_uri,
        attribute_pairs,
        namespace_pairs,
    })
}

fn add(left: usize, right: usize) -> Result<usize> {
    left.checked_add(right)
        .ok_or_else(|| ImportLimitExceeded("XML work forecast overflow").into())
}

fn mul(left: usize, right: usize) -> Result<usize> {
    left.checked_mul(right)
        .ok_or_else(|| ImportLimitExceeded("XML work forecast overflow").into())
}

fn sum(parts: &[usize]) -> Result<usize> {
    parts.iter().try_fold(0, |used, part| add(used, *part))
}

fn admit(used: usize, amount: usize, maximum: usize, message: &'static str) -> Result<usize> {
    used.checked_add(amount)
        .filter(|next| *next <= maximum)
        .ok_or_else(|| ImportLimitExceeded(message).into())
}

fn pairs(count: usize) -> Result<usize> {
    Ok(mul(count, count.saturating_sub(1))? / 2)
}

/// This is a resource lexer, not a second XML authority. Ordinary malformed
/// syntax returns its bounded prefix forecast; the original parser determines
/// the eventual format error and existing navigation fallback. Quota errors
/// remain strict, including a valid field name above the project allowance.
pub(super) fn scan_content(input: &str) -> Result<Fields> {
    let mut scanner = Scanner {
        input,
        position: 0,
        fields: Fields::default(),
    };
    let result = scanner.scan();
    match result {
        Ok(()) => Ok(scanner.fields),
        Err(error) if error.downcast_ref::<ImportLimitExceeded>().is_some() => Err(error),
        Err(error) if error.downcast_ref::<ParserFormatStop>().is_some() => Ok(scanner.fields),
        Err(_) => Err(ImportLimitExceeded("XML resource lexer cannot bound this syntax").into()),
    }
}

/// Only tokenizer failures proved to precede the remaining tail may use this
/// marker: consume_qname/consume_eq/consume_quote, missing token delimiters,
/// attribute whitespace, raw '<' in a quoted value, and unsupported '<!'.
/// Declarations reach this lexer only in DTD-disabled documents or entity
/// content after the complete RL66 prolog; the original parser rejects them
/// with DtdDetected/UnknownToken before entering their uncounted tail.
#[derive(Debug)]
struct ParserFormatStop;

impl std::fmt::Display for ParserFormatStop {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("XML resource lexer reached a parser format failure")
    }
}

impl std::error::Error for ParserFormatStop {}

fn format_stop() -> anyhow::Error {
    ParserFormatStop.into()
}

struct Scanner<'a> {
    input: &'a str,
    position: usize,
    fields: Fields,
}

impl<'a> Scanner<'a> {
    fn byte(&self) -> Option<u8> {
        self.input.as_bytes().get(self.position).copied()
    }
    fn starts(&self, bytes: &[u8]) -> bool {
        self.input.as_bytes()[self.position..].starts_with(bytes)
    }

    fn spaces(&mut self) -> bool {
        let previous = self.position;
        while self
            .byte()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.position += 1;
        }
        self.position != previous
    }

    fn require(&mut self, byte: u8) -> Result<()> {
        if self.byte() != Some(byte) {
            return Err(format_stop());
        }
        self.position += 1;
        Ok(())
    }

    fn delimited(&mut self, prefix: usize, delimiter: &str) -> Result<()> {
        self.position += prefix;
        let end = self.input[self.position..]
            .find(delimiter)
            .ok_or_else(format_stop)?;
        self.position += end + delimiter.len();
        self.fields.nodes = add(self.fields.nodes, 1)?;
        Ok(())
    }

    fn name(&mut self) -> Result<&'a str> {
        let start = self.position;
        let mut characters = self.input[start..].chars();
        let first = characters
            .next()
            .filter(|character| super::ncx_entities::xml_name_start(*character))
            .ok_or_else(format_stop)?;
        self.position += first.len_utf8();
        for character in characters {
            if !super::ncx_entities::xml_name_character(character) {
                break;
            }
            self.position += character.len_utf8();
            if self.position - start > MAX_PATH_BYTES {
                return Err(ImportLimitExceeded("XML name exceeds its byte allowance").into());
            }
        }
        let name = &self.input[start..self.position];
        if name.len() > MAX_PATH_BYTES {
            return Err(ImportLimitExceeded("XML name exceeds its byte allowance").into());
        }
        let local = name.split_once(':').map_or(name, |(_, local)| local);
        // consume_qname accepts a leading colon as an empty prefix. Preserve
        // that locked-parser behavior rather than stopping before a valid tail.
        if local.contains(':')
            || local
                .chars()
                .next()
                .is_none_or(|character| !super::ncx_entities::xml_name_start(character))
        {
            return Err(format_stop());
        }
        self.fields.maximum_name = self.fields.maximum_name.max(name.len());
        Ok(name)
    }

    fn quoted(&mut self) -> Result<&'a str> {
        let quote = self
            .byte()
            .filter(|byte| matches!(byte, b'\'' | b'"'))
            .ok_or_else(format_stop)?;
        self.position += 1;
        let start = self.position;
        let end = self.input.as_bytes()[start..]
            .iter()
            .position(|byte| *byte == quote)
            .ok_or_else(format_stop)?;
        self.position = start + end + 1;
        let value = &self.input[start..start + end];
        if value.contains('<') {
            return Err(format_stop());
        }
        Ok(value)
    }

    fn tag(&mut self) -> Result<()> {
        self.position += 1;
        self.name()?;
        self.fields.elements = add(self.fields.elements, 1)?;
        self.fields.nodes = add(self.fields.nodes, 1)?;
        let mut attributes = 0;
        let mut namespaces = 0;
        let result = (|| -> Result<()> {
            loop {
                if self.byte().is_none() {
                    // parse_start_tag's while !at_end() returns false at this
                    // exact EOF, so it is not a proved parser format failure.
                    self.fields.mixed_groups = true;
                    return Ok(());
                }
                let spaced = self.spaces();
                if self.byte() == Some(b'>') {
                    self.position += 1;
                    return Ok(());
                }
                if self.byte() == Some(b'/') {
                    self.position += 1;
                    self.require(b'>')?;
                    return Ok(());
                }
                if !spaced {
                    return Err(format_stop());
                }
                let name = self.name()?;
                self.spaces();
                self.require(b'=')?;
                self.spaces();
                let value = self.quoted()?;
                attributes = add(attributes, 1)?;
                self.fields.attributes = add(self.fields.attributes, 1)?;
                let (prefix, local) = name
                    .split_once(':')
                    .map_or((None, name), |(prefix, local)| (Some(prefix), local));
                if prefix == Some("xmlns") || local == "xmlns" {
                    namespaces = add(namespaces, 1)?;
                    self.fields.namespaces = add(self.fields.namespaces, 1)?;
                    self.fields.namespace_value = self.fields.namespace_value.max(value.len());
                    let references = value.bytes().filter(|byte| *byte == b'&').count();
                    self.fields.namespace_references =
                        self.fields.namespace_references.max(references);
                } else if prefix.is_some() {
                    self.fields.qualified_attributes = add(self.fields.qualified_attributes, 1)?;
                }
            }
        })();
        self.fields.attribute_pairs = add(self.fields.attribute_pairs, pairs(attributes)?)?;
        self.fields.namespace_pairs = add(self.fields.namespace_pairs, pairs(namespaces)?)?;
        if namespaces != 0 {
            self.fields.namespace_tags = add(self.fields.namespace_tags, 1)?;
        }
        result
    }

    fn scan(&mut self) -> Result<()> {
        while self.byte().is_some() {
            if self.starts(b"<!--") {
                self.delimited(4, "-->")?;
            } else if self.starts(b"<![CDATA[") {
                self.delimited(9, "]]>")?;
            } else if self.starts(b"<?") {
                self.delimited(2, "?>")?;
            } else if self.starts(b"<!") {
                return Err(format_stop());
            } else if self.starts(b"</") {
                self.position += 2;
                self.name()?;
                self.spaces();
                self.require(b'>')?;
                self.fields.closes = add(self.fields.closes, 1)?;
            } else if self.byte() == Some(b'<') {
                self.tag()?;
            } else {
                while self.byte().is_some_and(|byte| byte != b'<') {
                    self.position += 1;
                }
                self.fields.nodes = add(self.fields.nodes, 1)?;
            }
        }
        if self.fields.elements > MAX_NCX_NODES as usize || self.fields.attributes > MAX_FIELDS {
            return Err(
                ImportLimitExceeded("XML syntax records exceed their count allowance").into(),
            );
        }
        Ok(())
    }
}
