use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
};

const PAGE_SIZE: usize = 5;
const MAX_COMPOSITION: usize = 64;
const MAX_CANDIDATES: usize = 128;
const BEAM_WIDTH: usize = 12;

pub const SEARCH_INPUT_SCALARS: usize = 4096;
pub const THOUGHT_INPUT_SCALARS: usize = 16_384;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputPurpose {
    #[default]
    Search,
    Thought,
}

impl InputPurpose {
    pub fn maximum_scalars(self) -> usize {
        match self {
            Self::Search => SEARCH_INPUT_SCALARS,
            Self::Thought => THOUGHT_INPUT_SCALARS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the input exceeds its allowance of {maximum_scalars} Unicode scalars")]
pub struct InputLimitError {
    pub maximum_scalars: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputLanguage {
    #[default]
    Chinese,
    English,
    Numeric,
}

impl InputLanguage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chinese => "zh",
            Self::English => "en",
            Self::Numeric => "numeric",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputIntent {
    None,
    Changed,
    Submit,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub text: String,
    /// Number of original ASCII composition characters consumed by this choice.
    pub consumed: usize,
    pub spelling: String,
}

#[derive(Clone, Copy)]
struct Entry {
    text: &'static str,
    rank: u32,
}

struct Dictionary {
    exact: HashMap<&'static str, Vec<Entry>>,
    keys: Vec<&'static str>,
    max_key_length: usize,
}

impl Dictionary {
    fn bundled() -> &'static Self {
        static DICTIONARY: OnceLock<Dictionary> = OnceLock::new();
        DICTIONARY.get_or_init(|| {
            let mut exact: HashMap<&'static str, Vec<Entry>> = HashMap::new();
            let mut max_key_length = 1;
            for line in include_str!("../assets/pinyin/dictionary.tsv").lines() {
                let mut fields = line.split('\t');
                let (Some(key), Some(text), Some(rank)) =
                    (fields.next(), fields.next(), fields.next())
                else {
                    continue;
                };
                if key.is_empty() || text.is_empty() || !key.bytes().all(|c| c.is_ascii_lowercase())
                {
                    continue;
                }
                let Ok(rank) = rank.parse::<u32>() else {
                    continue;
                };
                max_key_length = max_key_length.max(key.len());
                exact.entry(key).or_default().push(Entry { text, rank });
            }
            for entries in exact.values_mut() {
                entries.sort_by_key(|entry| entry.rank);
                entries.dedup_by_key(|entry| entry.text);
            }
            let mut keys: Vec<_> = exact.keys().copied().collect();
            keys.sort_unstable();
            Self {
                exact,
                keys,
                max_key_length,
            }
        })
    }

    fn suggestions(&self, composition: &str) -> Vec<Candidate> {
        let normalized: String = composition
            .chars()
            .filter(char::is_ascii_alphabetic)
            .map(|character| character.to_ascii_lowercase())
            .collect();
        if normalized.is_empty() {
            return Vec::new();
        }
        let mut results: Vec<RankedCandidate> = Vec::new();
        if let Some(entries) = self.exact.get(normalized.as_str()) {
            for entry in entries.iter().take(MAX_CANDIDATES) {
                results.push(RankedCandidate::new(
                    entry.text.to_owned(),
                    normalized.len(),
                    0,
                    entry.rank as u64,
                ));
            }
        }

        // A small beam composes real dictionary words instead of requiring a canned phrase.
        let mut beams: Vec<Vec<Segment>> = vec![Vec::new(); normalized.len() + 1];
        beams[0].push(Segment {
            text: String::new(),
            score: 0,
            words: 0,
        });
        for start in 0..normalized.len() {
            if beams[start].is_empty() {
                continue;
            }
            let previous = beams[start].clone();
            for end in start + 1..=(start + self.max_key_length).min(normalized.len()) {
                // This buffer contains only ASCII letters, so these are also scalar boundaries.
                let Some(entries) = self.exact.get(&normalized[start..end]) else {
                    continue;
                };
                for prefix in &previous {
                    for entry in entries.iter().take(4) {
                        if prefix.words == 0 && end == normalized.len() {
                            continue;
                        }
                        let score =
                            prefix.score + ((entry.rank as f64 + 1.0).log2() * 100.0) as u64 + 800;
                        let mut text = prefix.text.clone();
                        text.push_str(entry.text);
                        beams[end].push(Segment {
                            text,
                            score,
                            words: prefix.words + 1,
                        });
                    }
                }
                retain_best(&mut beams[end]);
            }
        }
        for segment in &beams[normalized.len()] {
            results.push(RankedCandidate::new(
                segment.text.clone(),
                normalized.len(),
                1,
                segment.score,
            ));
        }

        // Prefix completion handles an unfinished final syllable without a network service.
        let first = self.keys.partition_point(|key| *key < normalized.as_str());
        let mut completions: Vec<(&str, Entry)> = self
            .keys
            .iter()
            .skip(first)
            .take_while(|key| key.starts_with(&normalized))
            .flat_map(|key| {
                self.exact[*key]
                    .iter()
                    .take(3)
                    .map(move |entry| (*key, *entry))
            })
            .collect();
        completions.sort_by_key(|(_, entry)| entry.rank);
        for (key, entry) in completions.into_iter().take(24) {
            if key != normalized {
                results.push(RankedCandidate::new(
                    entry.text.to_owned(),
                    normalized.len(),
                    1,
                    entry.rank as u64 + 10_000,
                ));
            }
        }

        // Leading words can be committed while the unconverted suffix stays editable.
        for end in (1..normalized.len()).rev() {
            if let Some(entries) = self.exact.get(&normalized[..end]) {
                for entry in entries.iter().take(8) {
                    results.push(RankedCandidate::new(
                        entry.text.to_owned(),
                        end,
                        2,
                        ((normalized.len() - end) as u64) * 100_000 + entry.rank as u64,
                    ));
                }
            }
        }
        results.push(RankedCandidate::new(
            normalized.clone(),
            normalized.len(),
            3,
            0,
        ));
        results.sort_by(|left, right| {
            (left.priority, left.score, &left.text).cmp(&(right.priority, right.score, &right.text))
        });
        let mut seen = HashSet::new();
        results
            .into_iter()
            .filter(|candidate| seen.insert(candidate.text.clone()))
            .take(MAX_CANDIDATES)
            .map(|candidate| Candidate {
                text: candidate.text,
                consumed: original_consumed(composition, candidate.consumed),
                spelling: normalized.chars().take(candidate.consumed).collect(),
            })
            .collect()
    }
}

#[derive(Clone)]
struct Segment {
    text: String,
    score: u64,
    words: usize,
}

fn retain_best(segments: &mut Vec<Segment>) {
    if segments.len() <= BEAM_WIDTH {
        return;
    }
    segments.sort_by(|left, right| (left.score, &left.text).cmp(&(right.score, &right.text)));
    segments.dedup_by(|left, right| left.text == right.text);
    segments.truncate(BEAM_WIDTH);
}

struct RankedCandidate {
    text: String,
    consumed: usize,
    priority: u8,
    score: u64,
}

impl RankedCandidate {
    fn new(text: String, consumed: usize, priority: u8, score: u64) -> Self {
        Self {
            text,
            consumed,
            priority,
            score,
        }
    }
}

fn original_consumed(composition: &str, normalized_count: usize) -> usize {
    let mut letters = 0;
    for (index, character) in composition.chars().enumerate() {
        if character.is_ascii_alphabetic() {
            letters += 1;
        }
        if letters == normalized_count {
            return index + 1;
        }
    }
    composition.chars().count()
}

#[derive(Clone, Debug)]
pub struct PinyinInput {
    pub query: String,
    pub composition: String,
    pub language: InputLanguage,
    pub caps: bool,
    pub candidate_page: usize,
    previous_language: InputLanguage,
    purpose: InputPurpose,
    scalar_limit: usize,
    query_revision: u64,
    initialization_rejected: bool,
}

impl Default for PinyinInput {
    fn default() -> Self {
        Self::empty(InputPurpose::Search)
    }
}

struct CompositionCommit {
    first: Option<Candidate>,
    suffix_start: usize,
    scalars: usize,
}

impl PinyinInput {
    /// Existing Search callers use the smaller purpose allowance. A rejected
    /// initialization owns no copy of the oversized source and cannot submit
    /// an empty replacement until an explicit clear or admitted edit repairs it.
    pub fn new(query: impl AsRef<str>) -> Self {
        Self::for_purpose(query.as_ref(), InputPurpose::Search).unwrap_or_else(|_| Self {
            initialization_rejected: true,
            ..Self::default()
        })
    }

    pub fn empty(purpose: InputPurpose) -> Self {
        Self {
            query: String::new(),
            composition: String::new(),
            language: InputLanguage::default(),
            caps: false,
            candidate_page: 0,
            previous_language: InputLanguage::default(),
            purpose,
            scalar_limit: purpose.maximum_scalars(),
            query_revision: 0,
            initialization_rejected: false,
        }
    }

    /// Inspect a borrowed source before materializing the editable field.
    pub fn for_purpose(query: &str, purpose: InputPurpose) -> Result<Self, InputLimitError> {
        let mut input = Self::empty(purpose);
        if !input.set_query(query) {
            return Err(InputLimitError {
                maximum_scalars: input.scalar_limit,
            });
        }
        Ok(input)
    }

    pub fn purpose(&self) -> InputPurpose {
        self.purpose
    }

    pub fn set_purpose(&mut self, purpose: InputPurpose) -> bool {
        let maximum = if self.purpose == purpose {
            self.scalar_limit
        } else {
            purpose.maximum_scalars()
        };
        if self.initialization_rejected || checked_scalar_count(&self.query, maximum).is_none() {
            return false;
        }
        self.purpose = purpose;
        self.scalar_limit = maximum;
        true
    }

    pub fn is_admitted(&self) -> bool {
        !self.initialization_rejected
            && checked_scalar_count(&self.query, self.scalar_limit).is_some()
    }

    pub fn query_revision(&self) -> u64 {
        self.query_revision
    }

    /// Rejection keeps the previous draft, composition and candidate page.
    pub fn set_query(&mut self, value: impl AsRef<str>) -> bool {
        self.set_query_for_purpose(value.as_ref(), self.purpose)
    }

    pub fn set_query_for_purpose(&mut self, value: &str, purpose: InputPurpose) -> bool {
        let maximum = if self.purpose == purpose {
            self.scalar_limit
        } else {
            purpose.maximum_scalars()
        };
        if checked_scalar_count(value, maximum).is_none() {
            return false;
        }
        if self.query != value {
            self.query = value.to_owned();
            self.query_revision = self.query_revision.wrapping_add(1);
        }
        self.composition.clear();
        self.candidate_page = 0;
        self.purpose = purpose;
        self.scalar_limit = maximum;
        self.initialization_rejected = false;
        true
    }

    pub fn clear(&mut self) {
        if !self.query.is_empty() {
            self.query_revision = self.query_revision.wrapping_add(1);
        }
        self.query.clear();
        self.composition.clear();
        self.candidate_page = 0;
        self.initialization_rejected = false;
    }

    pub fn all_candidates(&self) -> Vec<Candidate> {
        if self.composition.is_empty() || self.composition.len() > MAX_COMPOSITION {
            return Vec::new();
        }
        Dictionary::bundled().suggestions(&self.composition)
    }

    pub fn candidates(&self) -> Vec<String> {
        self.all_candidates()
            .into_iter()
            .skip(self.candidate_page * PAGE_SIZE)
            .take(PAGE_SIZE)
            .map(|candidate| candidate.text)
            .collect()
    }

    pub fn candidate_page_count(&self) -> usize {
        self.all_candidates().len().div_ceil(PAGE_SIZE).max(1)
    }

    pub fn choose(&mut self, text: &str) -> bool {
        let Some(candidate) = self
            .all_candidates()
            .into_iter()
            .find(|candidate| candidate.text == text)
        else {
            return false;
        };
        if !self.can_append(candidate.text.chars().count()) {
            return false;
        }
        self.query.push_str(&candidate.text);
        self.query_revision = self.query_revision.wrapping_add(1);
        self.composition = self.composition.chars().skip(candidate.consumed).collect();
        while self.composition.starts_with('\'') {
            self.composition.remove(0);
        }
        self.candidate_page = 0;
        true
    }

    pub fn commit_first(&mut self) -> bool {
        self.commit_with_character(None, false)
    }

    fn can_append(&self, scalars: usize) -> bool {
        !self.initialization_rejected
            && checked_scalar_count(&self.query, self.scalar_limit)
                .and_then(|current| current.checked_add(scalars))
                .is_some_and(|total| total <= self.scalar_limit)
    }

    fn composition_commit(&self) -> Option<CompositionCommit> {
        if self.composition.len() > MAX_COMPOSITION {
            return None;
        }
        if self.composition.is_empty() {
            return Some(CompositionCommit {
                first: None,
                suffix_start: 0,
                scalars: 0,
            });
        }
        let first = self.all_candidates().into_iter().next();
        let consumed = first.as_ref().map_or(0, |first| first.consumed);
        let suffix_start = self
            .composition
            .char_indices()
            .nth(consumed)
            .map_or(self.composition.len(), |(byte, _)| byte);
        let scalars = first
            .as_ref()
            .map_or(0, |first| first.text.chars().count())
            .checked_add(self.composition[suffix_start..].chars().count())?;
        Some(CompositionCommit {
            first,
            suffix_start,
            scalars,
        })
    }

    /// Price the complete candidate, unknown suffix and expanded uppercase
    /// character together. A failed period/mode/submit action consumes nothing.
    fn commit_with_character(&mut self, character: Option<char>, uppercase: bool) -> bool {
        let Some(commit) = self.composition_commit() else {
            return false;
        };
        let added = character.map_or(0, |character| {
            if uppercase {
                character.to_uppercase().count()
            } else {
                1
            }
        });
        let Some(scalars) = commit.scalars.checked_add(added) else {
            return false;
        };
        if !self.can_append(scalars) {
            return false;
        }
        if let Some(first) = commit.first {
            self.query.push_str(&first.text);
        }
        // Preserve the original unconvertible spelling only after admitting
        // its entire suffix. No successful submit represents truncated input.
        self.query
            .push_str(&self.composition[commit.suffix_start..]);
        if let Some(character) = character {
            if uppercase {
                self.query.extend(character.to_uppercase());
            } else {
                self.query.push(character);
            }
        }
        if scalars > 0 {
            self.query_revision = self.query_revision.wrapping_add(1);
        }
        self.composition.clear();
        self.candidate_page = 0;
        true
    }

    #[cfg(test)]
    pub(crate) fn set_test_scalar_limit(&mut self, maximum: usize) {
        self.scalar_limit = maximum.min(self.purpose.maximum_scalars());
    }

    pub fn press(&mut self, key: &str) -> InputIntent {
        match key {
            "backspace" | "delete" => {
                if !self.composition.is_empty() {
                    self.composition.pop();
                } else if self.query.pop().is_some() {
                    self.query_revision = self.query_revision.wrapping_add(1);
                }
                self.candidate_page = 0;
            }
            "shift" => {
                self.caps = !self.caps;
            }
            "language" => {
                if !self.commit_first() {
                    return InputIntent::Rejected;
                }
                self.language = if self.language == InputLanguage::English {
                    InputLanguage::Chinese
                } else {
                    InputLanguage::English
                };
                self.caps = false;
            }
            "numeric" | "numbers" => {
                if !self.commit_first() {
                    return InputIntent::Rejected;
                }
                if self.language == InputLanguage::Numeric {
                    self.language = self.previous_language;
                } else {
                    self.previous_language = self.language;
                    self.language = InputLanguage::Numeric;
                }
                self.caps = false;
            }
            "space" => {
                if self.composition.is_empty() {
                    if !self.commit_with_character(Some(' '), false) {
                        return InputIntent::Rejected;
                    }
                } else if let Some(candidate) = self.candidates().into_iter().next() {
                    if !self.choose(&candidate) {
                        return InputIntent::Rejected;
                    }
                }
            }
            "period" => {
                let character = if self.language == InputLanguage::Chinese {
                    '\u{3002}'
                } else {
                    '.'
                };
                if !self.commit_with_character(Some(character), false) {
                    return InputIntent::Rejected;
                }
            }
            "submit" | "enter" | "search" => {
                return if self.commit_first() {
                    InputIntent::Submit
                } else {
                    InputIntent::Rejected
                };
            }
            "candidate-next" | "next_candidates" => {
                if self.candidate_page + 1 < self.candidate_page_count() {
                    self.candidate_page += 1;
                }
            }
            "candidate-prev" | "previous_candidates" => {
                self.candidate_page = self.candidate_page.saturating_sub(1);
            }
            "clear" => self.clear(),
            _ => {
                let mut characters = key.chars();
                let Some(character) = characters.next() else {
                    return InputIntent::None;
                };
                if characters.next().is_some() {
                    return InputIntent::None;
                }
                if self.language == InputLanguage::Chinese
                    && !self.caps
                    && (character.is_ascii_alphabetic()
                        || character == '\''
                        || character == '\u{00fc}')
                {
                    if !self.is_admitted() || self.composition.len() >= MAX_COMPOSITION {
                        return InputIntent::Rejected;
                    }
                    self.composition.push(if character == '\u{00fc}' {
                        'v'
                    } else {
                        character.to_ascii_lowercase()
                    });
                } else {
                    if !self.commit_with_character(Some(character), self.caps) {
                        return InputIntent::Rejected;
                    }
                }
                self.candidate_page = 0;
            }
        }
        InputIntent::Changed
    }
}

fn checked_scalar_count(text: &str, maximum: usize) -> Option<usize> {
    let count = text.chars().take(maximum.saturating_add(1)).count();
    (count <= maximum).then_some(count)
}

#[cfg(test)]
#[path = "input_limit_tests.rs"]
mod input_limit_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn type_pinyin(value: &str) -> PinyinInput {
        let mut input = PinyinInput::default();
        for character in value.chars() {
            input.press(&character.to_string());
        }
        input
    }

    #[test]
    fn bundled_dictionary_is_substantive_and_candidates_are_not_a_mock_table() {
        assert!(Dictionary::bundled().exact.len() > 10_000);
        for pinyin in ["zhongguo", "dushu", "tushuguan", "weixin", "shijie"] {
            let input = type_pinyin(pinyin);
            assert!(
                input.candidates().iter().any(|text| !text.is_ascii()),
                "{pinyin}"
            );
        }
    }

    #[test]
    fn choosing_a_candidate_preserves_a_continuous_unconverted_suffix() {
        let mut input = type_pinyin("zhongguoxyz");
        let candidate = input
            .all_candidates()
            .into_iter()
            .find(|candidate| candidate.consumed == 8 && !candidate.text.is_ascii())
            .expect("Leading dictionary phrase");
        assert!(input.choose(&candidate.text));
        assert_eq!(input.composition, "xyz");
        assert!(!input.query.is_empty());
    }

    #[test]
    fn backspace_removes_composition_before_committed_unicode() {
        let mut input = PinyinInput::new("\u{4e66}");
        input.press("a");
        input.press("backspace");
        assert_eq!(input.query, "\u{4e66}");
        assert!(input.composition.is_empty());
        input.press("backspace");
        assert!(input.query.is_empty());
    }

    #[test]
    fn english_numeric_and_caps_keys_have_real_effects() {
        let mut input = PinyinInput::default();
        input.press("language");
        input.press("shift");
        input.press("a");
        input.press("numeric");
        input.press("1");
        input.press("@");
        input.press("numeric");
        assert_eq!(input.language, InputLanguage::English);
        assert_eq!(input.query, "A1@");
    }

    #[test]
    fn submission_commits_candidates_and_retains_unknown_spelling() {
        let mut input = type_pinyin("zhongguo");
        assert_eq!(input.press("submit"), InputIntent::Submit);
        assert!(input.composition.is_empty());
        assert!(!input.query.is_ascii());
        let mut unknown = type_pinyin("qzxqzx");
        assert_eq!(unknown.press("submit"), InputIntent::Submit);
        assert!(!unknown.query.is_empty());
    }

    #[test]
    fn incomplete_syllables_produce_completion_candidates_and_can_page() {
        let mut input = type_pinyin("zhongg");
        assert!(
            input
                .candidates()
                .iter()
                .any(|candidate| !candidate.is_ascii())
        );
        input.set_query("");
        input.press("s");
        assert!(input.candidate_page_count() > 1);
        let first = input.candidates();
        input.press("candidate-next");
        assert_ne!(input.candidates(), first);
    }
}
