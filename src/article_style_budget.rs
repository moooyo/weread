//! Operation-local admission for the existing charwise style suppression rule.

use std::collections::HashMap;

const MAX_STYLE_WORK: usize = 64 * 1024 * 1024;
const MAX_STYLE_METADATA: usize = 8 * 1024 * 1024;
const LOOKUP_WORK: usize = 16;

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub work_bytes: usize,
    pub metadata_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            work_bytes: MAX_STYLE_WORK,
            metadata_bytes: MAX_STYLE_METADATA,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleLimitKind {
    Work,
    Metadata,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("article style {kind:?} exceeds its allowance (limit {limit})")]
pub struct StyleLimitError {
    pub kind: StyleLimitKind,
    pub used: usize,
    pub requested: usize,
    pub limit: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Usage {
    pub lookups: usize,
    pub memo_hits: usize,
    pub style_scans: usize,
    pub input_bytes: usize,
    pub lowered_chars: usize,
    pub normalized_bytes: usize,
    pub work_bytes: usize,
    pub metadata_bytes: usize,
    pub peak_scratch: usize,
    pub class_lookups: usize,
    pub class_memo_hits: usize,
    pub class_scans: usize,
    pub class_input_bytes: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SpanKey {
    address: usize,
    bytes: usize,
    kind: SpanKind,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum SpanKind {
    Style,
    ContainerClass,
    CollectorClass,
}

struct Memo<'a> {
    // Retain the borrow, not just its address. The immutable style cannot be
    // dropped/replaced while this operation's identity evidence remains live.
    style: &'a str,
    suppressed: bool,
}

pub(super) struct Budget<'a> {
    limits: Limits,
    usage: Usage,
    memo: HashMap<SpanKey, Memo<'a>>,
}

impl Default for Budget<'_> {
    fn default() -> Self {
        #[cfg(test)]
        let limits = super::style_budget_tests::configured_limits().unwrap_or_default();
        #[cfg(not(test))]
        let limits = Limits::default();
        let limits = Limits {
            work_bytes: limits.work_bytes.min(MAX_STYLE_WORK),
            metadata_bytes: limits.metadata_bytes.min(MAX_STYLE_METADATA),
        };
        observe_start();
        Self {
            limits,
            usage: Usage::default(),
            memo: HashMap::new(),
        }
    }
}

impl<'a> Budget<'a> {
    pub(super) fn suppresses(&mut self, style: &'a str) -> Result<bool, StyleLimitError> {
        self.matches_span(style, SpanKind::Style)
    }

    pub(super) fn container_class(&mut self, classes: &'a str) -> Result<bool, StyleLimitError> {
        self.matches_span(classes, SpanKind::ContainerClass)
    }

    pub(super) fn collector_class(&mut self, classes: &'a str) -> Result<bool, StyleLimitError> {
        self.matches_span(classes, SpanKind::CollectorClass)
    }

    fn matches_span(&mut self, style: &'a str, kind: SpanKind) -> Result<bool, StyleLimitError> {
        self.work(LOOKUP_WORK)?;
        if kind == SpanKind::Style {
            self.usage.lookups += 1;
        } else {
            self.usage.class_lookups += 1;
        }
        observe(self.usage);
        let key = SpanKey {
            address: style.as_ptr() as usize,
            bytes: style.len(),
            kind,
        };
        if let Some(memo) = self.memo.get(&key) {
            debug_assert_eq!(memo.style.as_ptr(), style.as_ptr());
            debug_assert_eq!(memo.style.len(), style.len());
            let suppressed = memo.suppressed;
            if kind == SpanKind::Style {
                self.usage.memo_hits += 1;
            } else {
                self.usage.class_memo_hits += 1;
            }
            observe(self.usage);
            return Ok(suppressed);
        }
        // The whole borrowed read/lower/matching forecast passes before a
        // character is inspected. No full normalized String is materialized.
        let factor = if kind == SpanKind::Style { 8 } else { 4 };
        let fee = style.len().checked_mul(factor).ok_or(StyleLimitError {
            kind: StyleLimitKind::Work,
            used: self.usage.work_bytes,
            requested: usize::MAX,
            limit: self.limits.work_bytes,
        })?;
        let next_work = admitted(
            self.usage.work_bytes,
            fee,
            self.limits.work_bytes,
            StyleLimitKind::Work,
        )?;
        let scratch = std::mem::size_of::<Matcher>() + 4;
        let slot = std::mem::size_of::<(SpanKey, Memo<'_>)>() + 2;
        let entries = self.memo.len().checked_add(1).ok_or(StyleLimitError {
            kind: StyleLimitKind::Metadata,
            used: self.usage.metadata_bytes,
            requested: usize::MAX,
            limit: self.limits.metadata_bytes,
        })?;
        let metadata = entries
            .checked_mul(2 * slot)
            .and_then(|bytes| bytes.checked_add(4 * slot))
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Self>() + scratch))
            .ok_or(StyleLimitError {
                kind: StyleLimitKind::Metadata,
                used: self.usage.metadata_bytes,
                requested: usize::MAX,
                limit: self.limits.metadata_bytes,
            })?;
        admitted(
            0,
            metadata,
            self.limits.metadata_bytes,
            StyleLimitKind::Metadata,
        )?;
        self.usage.work_bytes = next_work;
        self.usage.metadata_bytes = metadata;
        self.usage.peak_scratch = self.usage.peak_scratch.max(scratch);
        if kind == SpanKind::Style {
            self.usage.style_scans += 1;
            self.usage.input_bytes += style.len();
        } else {
            self.usage.class_scans += 1;
            self.usage.class_input_bytes += style.len();
        }
        observe_admission(self.usage);

        let suppressed = if kind == SpanKind::Style {
            let mut matcher = Matcher::default();
            let mut utf8 = [0; 4];
            // Keep exactly the old order: remove ASCII whitespace from the input
            // scalars, then apply char::to_lowercase, then match the resulting bytes.
            for character in style
                .chars()
                .filter(|character| !character.is_ascii_whitespace())
                .flat_map(char::to_lowercase)
            {
                self.usage.lowered_chars += 1;
                for byte in character.encode_utf8(&mut utf8).bytes() {
                    self.usage.normalized_bytes += 1;
                    matcher.feed(byte);
                }
            }
            matcher.matched
        } else {
            // Scraper's class membership sorts/deduplicates the same ASCII
            // tokens. A borrowed membership test needs neither that Vec nor
            // LocalName copies, and preserves its case-sensitive result.
            style.split_ascii_whitespace().any(|token| match kind {
                SpanKind::ContainerClass => token == "rich_media_content",
                SpanKind::CollectorClass => matches!(
                    token,
                    "rich_media_title" | "rich_media_meta" | "rich_media_meta_list",
                ),
                SpanKind::Style => unreachable!(),
            })
        };
        self.memo.insert(key, Memo { style, suppressed });
        observe(self.usage);
        Ok(suppressed)
    }

    fn work(&mut self, amount: usize) -> Result<(), StyleLimitError> {
        self.usage.work_bytes = admitted(
            self.usage.work_bytes,
            amount,
            self.limits.work_bytes,
            StyleLimitKind::Work,
        )?;
        Ok(())
    }
}

#[derive(Default)]
struct Matcher {
    states: [usize; 2],
    matched: bool,
}

impl Matcher {
    fn feed(&mut self, byte: u8) {
        if self.matched {
            return;
        }
        for (index, needle) in [b"display:none".as_slice(), b"visibility:hidden".as_slice()]
            .into_iter()
            .enumerate()
        {
            let state = &mut self.states[index];
            if byte == needle[*state] {
                *state += 1;
                if *state == needle.len() {
                    self.matched = true;
                    return;
                }
            } else {
                // Each fixed needle contains its starting byte only at offset
                // zero, so a mismatch can retain only this one-byte prefix.
                *state = usize::from(byte == needle[0]);
            }
        }
    }
}

fn admitted(
    used: usize,
    requested: usize,
    limit: usize,
    kind: StyleLimitKind,
) -> Result<usize, StyleLimitError> {
    used.checked_add(requested)
        .filter(|bytes| *bytes <= limit)
        .ok_or(StyleLimitError {
            kind,
            used,
            requested,
            limit,
        })
}

fn observe_admission(usage: Usage) {
    #[cfg(test)]
    super::style_budget_tests::observe_admission(usage);
    #[cfg(not(test))]
    let _ = usage;
}

fn observe_start() {
    #[cfg(test)]
    super::style_budget_tests::observe_start();
}

fn observe(usage: Usage) {
    #[cfg(test)]
    super::style_budget_tests::observe(usage);
    #[cfg(not(test))]
    let _ = usage;
}
