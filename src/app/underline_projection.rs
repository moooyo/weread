//! One current chapter's checked remote-underline positions, without a body copy.

use std::{collections::HashMap, fmt};

use crate::{
    reader::Line,
    source::{RemoteUnderline, WeReadClient},
};

const MATCHING_WORK_BYTES: usize = 64 * 1024 * 1024;
const MAX_PAGE_REMOTE_MARKS: usize = 1024;
const MAX_PAGE_LINES: usize = 256;
const PAGE_MARK_WORK_BYTES: usize = 64 * 1024 * 1024;

fn page_mark_work_bytes(bytes: usize) -> Result<usize, ProjectionError> {
    // A nonzero kerning/gap can visit all i existing carets at scalar i.
    // Two endpoints therefore need at most n*(n+1) boundary checks. UTF-8
    // bytes conservatively bound n; add the other linear scans separately.
    bytes
        .checked_add(1)
        .and_then(|next| bytes.checked_mul(next))
        .and_then(|quadratic| {
            bytes
                .checked_mul(8)
                .and_then(|linear| quadratic.checked_add(linear))
        })
        .ok_or(ProjectionError::PageWork)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ProjectionKey {
    pub body_revision: u64,
    pub underline_revision: u64,
    pub account_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ProjectedUnderline {
    pub index: usize,
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProjectionError {
    InputBudget,
    MatchingWork,
    PageMarks,
    PageLines,
    PageWork,
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InputBudget => "remote underline input exceeds its resource budget",
            Self::MatchingWork => "remote underline matching exceeds its work budget",
            Self::PageMarks => "remote underline page marks exceed their output budget",
            Self::PageLines => "remote underline page geometry exceeds its line budget",
            Self::PageWork => "remote underline page marks exceed their geometry work budget",
        })
    }
}

impl std::error::Error for ProjectionError {}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ProjectionCounters {
    pub builds: usize,
    pub matches: usize,
    pub charged_bytes: usize,
}

pub(super) struct UnderlineProjection {
    key: Option<ProjectionKey>,
    ranges: Vec<ProjectedUnderline>,
    failure: Option<ProjectionError>,
    failure_reported: bool,
    work_limit: usize,
    page_mark_limit: usize,
    page_work_limit: usize,
    #[cfg(test)]
    counters: ProjectionCounters,
}

impl Default for UnderlineProjection {
    fn default() -> Self {
        Self {
            key: None,
            ranges: Vec::new(),
            failure: None,
            failure_reported: false,
            work_limit: MATCHING_WORK_BYTES,
            page_mark_limit: MAX_PAGE_REMOTE_MARKS,
            page_work_limit: PAGE_MARK_WORK_BYTES,
            #[cfg(test)]
            counters: ProjectionCounters::default(),
        }
    }
}

impl UnderlineProjection {
    pub fn clear(&mut self) {
        self.key = None;
        self.ranges.clear();
        self.failure = None;
        self.failure_reported = false;
    }

    /// Failed versions are cached too: neither reflow nor an idle redraw retries
    /// the same expensive input. Only explicit document/list/account changes do.
    pub fn ensure(
        &mut self,
        key: ProjectionKey,
        body: &str,
        underlines: &[RemoteUnderline],
    ) -> Result<(), ProjectionError> {
        if self.key == Some(key) {
            return self.failure.map_or(Ok(()), Err);
        }
        self.clear();
        self.key = Some(key);
        #[cfg(test)]
        {
            self.counters.builds += 1;
        }
        match self.build(body, underlines) {
            Ok(ranges) => {
                self.ranges = ranges;
                Ok(())
            }
            Err(error) => {
                self.failure = Some(error);
                Err(error)
            }
        }
    }

    fn build(
        &mut self,
        body: &str,
        underlines: &[RemoteUnderline],
    ) -> Result<Vec<ProjectedUnderline>, ProjectionError> {
        WeReadClient::validate_underlines(underlines).map_err(|_| ProjectionError::InputBudget)?;
        let mut quotes = HashMap::new();
        let mut ranges = Vec::new();
        let mut charged = 0usize;
        for (index, underline) in underlines.iter().enumerate() {
            let quote = underline.text.as_str();
            let range = if quote.is_empty() || quote.len() > body.len() {
                None
            } else if let Some(range) = quotes.get(quote) {
                *range
            } else {
                // Prepay conservative byte work for both complete searches,
                // scalar-prefix counting and pattern/scalar processing. This
                // is an application work allowance, not a wall-clock promise.
                let fee = body
                    .len()
                    .checked_mul(5)
                    .and_then(|bytes| {
                        quote
                            .len()
                            .checked_mul(3)
                            .and_then(|quote| bytes.checked_add(quote))
                    })
                    .ok_or(ProjectionError::MatchingWork)?;
                charged = charged
                    .checked_add(fee)
                    .filter(|bytes| *bytes <= self.work_limit)
                    .ok_or(ProjectionError::MatchingWork)?;
                #[cfg(test)]
                {
                    self.counters.matches += 1;
                    self.counters.charged_bytes += fee;
                }
                let range = unique_quote_range(body, quote);
                // Keys borrow the current input only during this build. No
                // quote strings or document are retained in the final cache.
                quotes.insert(quote, range);
                range
            };
            if let Some((start, end)) = range {
                ranges.push(ProjectedUnderline { index, start, end });
            }
        }
        Ok(ranges)
    }

    pub fn range(&self, index: usize) -> Option<(u32, u32)> {
        self.ranges
            .iter()
            .find(|range| range.index == index)
            .map(|range| (range.start, range.end))
    }

    /// Count every actual line mark before creating page output. Excess output
    /// rejects the entire remote overlay; it never yields a partial guessed set.
    pub fn page_ranges(&self, lines: &[Line]) -> Result<Vec<ProjectedUnderline>, ProjectionError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        if lines.len() > MAX_PAGE_LINES {
            return Err(ProjectionError::PageLines);
        }
        let mut visible = Vec::new();
        let mut marks = 0usize;
        let mut work = 0usize;
        for range in &self.ranges {
            let mut intersects_page = false;
            for line in lines {
                if range.start < line.end && line.start < range.end {
                    marks = marks
                        .checked_add(1)
                        .filter(|count| *count <= self.page_mark_limit)
                        .ok_or(ProjectionError::PageMarks)?;
                    // Prepay the real worst-case equal-caret backscan, not an
                    // assumption that all zero-width kerning is a no-op.
                    work = work
                        .checked_add(page_mark_work_bytes(line.text.len())?)
                        .filter(|bytes| *bytes <= self.page_work_limit)
                        .ok_or(ProjectionError::PageWork)?;
                    intersects_page = true;
                }
            }
            if intersects_page {
                visible.push(*range);
            }
        }
        Ok(visible)
    }

    pub fn take_failure_notice(&mut self) -> bool {
        !std::mem::replace(&mut self.failure_reported, true)
    }

    #[cfg(test)]
    pub fn counters(&self) -> ProjectionCounters {
        self.counters
    }

    #[cfg(test)]
    pub fn set_test_work_limit(&mut self, limit: usize) {
        self.work_limit = limit;
        self.clear();
    }
}

/// Unique literal matches use Unicode scalar offsets. Overlapping duplicates
/// stay unresolved, and unknown upstream range units never become an anchor.
pub(super) fn unique_quote_range(body: &str, quote: &str) -> Option<(u32, u32)> {
    if quote.is_empty() {
        return None;
    }
    let byte = body.find(quote)?;
    let next_character = byte + body[byte..].chars().next()?.len_utf8();
    if body[next_character..].contains(quote) {
        return None;
    }
    let start = u32::try_from(body[..byte].chars().count()).ok()?;
    let length = u32::try_from(quote.chars().count()).ok()?;
    Some((start, start.checked_add(length)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(body_revision: u64, underline_revision: u64, account_epoch: u64) -> ProjectionKey {
        ProjectionKey {
            body_revision,
            underline_revision,
            account_epoch,
        }
    }

    fn underline(text: &str) -> RemoteUnderline {
        RemoteUnderline {
            text: text.into(),
            range: "opaque-range".into(),
            ..Default::default()
        }
    }

    #[test]
    fn unicode_and_overlapping_duplicates_keep_proven_scalar_positions() {
        assert_eq!(
            unique_quote_range("A\u{1f600}\u{4e2d}B", "\u{1f600}\u{4e2d}"),
            Some((1, 3))
        );
        assert_eq!(unique_quote_range("aaa", "aa"), None);
        assert_eq!(unique_quote_range("repeat repeat", "repeat"), None);
        assert_eq!(unique_quote_range("abc", "missing"), None);
        assert_eq!(unique_quote_range("abc", ""), None);
    }

    #[test]
    fn duplicate_quotes_are_matched_once_and_repeated_versions_reuse_the_projection() {
        let mut projection = UnderlineProjection::default();
        let records = [
            underline("target"),
            underline("target"),
            underline("absent"),
        ];
        projection
            .ensure(key(1, 1, 0), "a target b", &records)
            .unwrap();
        assert_eq!(projection.range(0), Some((2, 8)));
        assert_eq!(projection.range(1), Some((2, 8)));
        assert_eq!(projection.range(2), None);
        let counters = projection.counters();
        assert_eq!(counters.matches, 2);
        for _ in 0..128 {
            projection
                .ensure(key(1, 1, 0), "a target b", &records)
                .unwrap();
            projection
                .page_ranges(&[Line {
                    start: 0,
                    end: 10,
                    ..Default::default()
                }])
                .unwrap();
        }
        assert_eq!(projection.counters(), counters);
    }

    #[test]
    fn work_is_charged_before_matching_and_a_failed_version_is_not_retried() {
        let mut projection = UnderlineProjection {
            work_limit: 17,
            ..Default::default()
        };
        let records = [underline("b"), underline("b")];
        assert_eq!(
            projection.ensure(key(1, 1, 0), "abc", &records),
            Err(ProjectionError::MatchingWork)
        );
        assert_eq!(projection.counters().matches, 0);
        assert!(projection.ranges.is_empty());
        let counters = projection.counters();
        for _ in 0..64 {
            assert_eq!(
                projection.ensure(key(1, 1, 0), "abc", &records),
                Err(ProjectionError::MatchingWork)
            );
        }
        assert_eq!(projection.counters(), counters);
        assert!(projection.take_failure_notice());
        assert!(!projection.take_failure_notice());
        projection.work_limit = 18;
        projection.ensure(key(2, 1, 0), "abc", &records).unwrap();
        assert_eq!(projection.counters().matches, 1);
        assert_eq!(projection.counters().charged_bytes, 18);
        assert_eq!(projection.range(0), Some((1, 2)));
    }

    #[test]
    fn document_list_and_account_versions_replace_the_single_cache() {
        let mut projection = UnderlineProjection::default();
        let records = [underline("x")];
        projection.ensure(key(1, 1, 0), "x a", &records).unwrap();
        assert_eq!(projection.range(0), Some((0, 1)));
        projection.ensure(key(2, 1, 0), "a x", &records).unwrap();
        assert_eq!(projection.range(0), Some((2, 3)));
        projection
            .ensure(key(2, 2, 0), "a x", &[underline("a")])
            .unwrap();
        assert_eq!(projection.range(0), Some((0, 1)));
        projection
            .ensure(key(2, 2, 1), "a x", &[underline("a")])
            .unwrap();
        assert_eq!(projection.counters().builds, 4);
        assert_eq!(projection.ranges.len(), 1);
        projection.clear();
        assert!(projection.key.is_none() && projection.ranges.is_empty());
    }

    #[test]
    fn late_work_exhaustion_keeps_no_partial_projection() {
        let mut projection = UnderlineProjection {
            work_limit: 18,
            ..Default::default()
        };
        let records = [underline("b"), underline("c")];
        assert_eq!(
            projection.ensure(key(1, 1, 0), "abc", &records),
            Err(ProjectionError::MatchingWork)
        );
        assert_eq!(projection.counters().matches, 1);
        assert_eq!(projection.counters().charged_bytes, 18);
        assert!(projection.ranges.is_empty());
        assert_eq!(projection.range(0), None);
    }

    #[test]
    fn dense_page_marks_are_rejected_before_returning_partial_output() {
        let mut projection = UnderlineProjection {
            page_mark_limit: 2,
            ..Default::default()
        };
        projection
            .ensure(key(1, 1, 0), "abcdef", &[underline("abcde")])
            .unwrap();
        let lines = [
            Line {
                start: 0,
                end: 2,
                ..Default::default()
            },
            Line {
                start: 2,
                end: 4,
                ..Default::default()
            },
            Line {
                start: 4,
                end: 6,
                ..Default::default()
            },
        ];
        assert_eq!(
            projection.page_ranges(&lines),
            Err(ProjectionError::PageMarks)
        );
        assert_eq!(projection.page_ranges(&lines[..2]).unwrap().len(), 1);
        assert_eq!(projection.counters().matches, 1);
    }

    #[test]
    fn page_geometry_work_is_prepaid_before_deriving_mark_endpoints() {
        let mut projection = UnderlineProjection {
            page_work_limit: page_mark_work_bytes(32).unwrap() * 2,
            ..Default::default()
        };
        projection
            .ensure(key(1, 1, 0), "abcdef", &[underline("abcde")])
            .unwrap();
        let lines = [
            Line {
                text: "x".repeat(32),
                start: 0,
                end: 2,
                ..Default::default()
            },
            Line {
                text: "x".repeat(32),
                start: 2,
                end: 4,
                ..Default::default()
            },
            Line {
                text: "x".repeat(32),
                start: 4,
                end: 6,
                ..Default::default()
            },
        ];
        assert_eq!(
            projection.page_ranges(&lines),
            Err(ProjectionError::PageWork)
        );
        assert_eq!(projection.page_ranges(&lines[..2]).unwrap().len(), 1);
        assert_eq!(projection.counters().matches, 1);
    }

    #[test]
    fn genuine_combining_line_endpoints_require_worst_case_work_reservation() {
        use crate::reader::{Layout, Paginator, TextMetrics};

        let body = format!("A{}B", "\u{301}".repeat(32));
        let layout = Layout {
            width: 240.0,
            font_size: 20.0,
            indent: 0,
            justify: false,
            ..Default::default()
        };
        let metrics = TextMetrics::default();
        let paginator = Paginator::new(&body, &layout, &metrics);
        assert_eq!(paginator.current_page().lines.len(), 1);
        let line = &paginator.current_page().lines[0];
        let fee = page_mark_work_bytes(line.text.len()).unwrap();
        let mut projection = UnderlineProjection {
            page_work_limit: fee - 1,
            ..Default::default()
        };
        projection
            .ensure(key(1, 1, 0), &body, &[underline("A")])
            .unwrap();
        assert_eq!(
            projection.page_ranges(&paginator.current_page().lines),
            Err(ProjectionError::PageWork)
        );
        projection.page_work_limit = fee;
        let admitted = projection
            .page_ranges(&paginator.current_page().lines)
            .unwrap();
        assert_eq!(admitted.len(), 1);
        let (characters, boundaries) = line.caret_work_for_test(&layout, &metrics);
        assert_eq!(characters, 34);
        assert_eq!(boundaries, 0);
        assert!(2 * (characters + boundaries) <= fee);
        assert_eq!(
            line.x_at_offset(admitted[0].start, &layout, &metrics),
            Some(0.0)
        );
        assert_eq!(
            line.x_at_offset(admitted[0].end, &layout, &metrics),
            Some(metrics.advance('A', layout.font_size))
        );
    }
}
