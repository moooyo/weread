//! Conservative chapter access evidence from a fresh, request-bound Reader state.
use crate::domain::ContentStatus;
use serde_json::{Map, Value};

const FREE_BOOK_BIT: u64 = 0x20;
const LIMITED_FREE_BOOK_BIT: u64 = 0x40;
const WHOLE_PURCHASE_BIT: u64 = 0x1;
const CHAPTER_PURCHASE_BIT: u64 = 0x2;
const TRIAL_EDITION_BIT: u64 = 1 << 19;

#[derive(Clone, Copy, Default)]
enum Evidence {
    Granted,
    Restricted,
    /// An unpaid trial edition is limited even when one sample chapter is readable.
    TrialEdition,
    #[default]
    Unknown,
}

#[derive(Clone)]
struct SessionBinding {
    account_key: Option<String>,
    generation: u64,
}

#[derive(Clone, Default)]
pub(super) struct ReaderAccess {
    evidence: Evidence,
    expected_words: Option<u64>,
    binding: Option<SessionBinding>,
}

impl std::fmt::Debug for ReaderAccess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReaderAccess")
            .field("has_word_count", &self.expected_words.is_some())
            .field("session_bound", &self.binding.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Truth {
    Yes,
    No,
    Unknown,
}

impl ReaderAccess {
    pub(super) fn from_state(
        state: &Value,
        book_id: &str,
        chapter_uid: &str,
        expected_vid: Option<&str>,
        now: i64,
    ) -> Self {
        let Some(reader) = state.get("reader").and_then(Value::as_object) else {
            return Self::default();
        };
        if ["isBookInfoError", "isBookForbidden"].iter().any(|field| {
            reader
                .get(*field)
                .is_some_and(|value| value != &Value::Bool(false))
        }) {
            return Self::default();
        }
        let Some(book) = reader.get("bookInfo").and_then(Value::as_object) else {
            return Self::default();
        };
        if book_id.is_empty()
            || chapter_uid.is_empty()
            || identifier(reader.get("bookId")).as_deref() != Some(book_id)
            || identifier(book.get("bookId")).as_deref() != Some(book_id)
        {
            return Self::default();
        }
        let Some(chapters) = reader.get("chapterInfos").and_then(Value::as_array) else {
            return Self::default();
        };
        let mut targets = chapters.iter().filter(|chapter| {
            identifier(chapter.get("chapterUid")).as_deref() == Some(chapter_uid)
        });
        let Some(chapter) = targets.next().and_then(Value::as_object) else {
            return Self::default();
        };
        if targets.next().is_some() {
            return Self::default();
        }
        let Some(current) = reader.get("currentChapter").and_then(Value::as_object) else {
            return Self::default();
        };
        if identifier(current.get("chapterUid")).as_deref() != Some(chapter_uid)
            || ["chapterIdx", "paid", "price", "wordCount"]
                .iter()
                .any(|field| match (chapter.get(*field), current.get(*field)) {
                    (Some(target), Some(current)) => !same_scalar(target, current),
                    _ => false,
                })
        {
            return Self::default();
        }

        let expected_vid = expected_vid.filter(|vid| !vid.is_empty() && *vid != "0");
        let actual_vid = identifier(state.get("user").and_then(|user| user.get("vid")));
        if actual_vid
            .as_deref()
            .filter(|vid| *vid != "0")
            .is_some_and(|vid| Some(vid) != expected_vid)
        {
            return Self::default();
        }
        let account_verified = expected_vid.is_some() && actual_vid.as_deref() == expected_vid;
        let evidence = access_evidence(
            book,
            chapter,
            reader.get("memberCardSummary"),
            account_verified,
            now,
        );
        Self {
            evidence,
            expected_words: integer(chapter.get("wordCount")),
            binding: None,
        }
    }

    pub(super) fn bind_session(
        &mut self,
        account_key: Option<String>,
        generation: u64,
        stable: bool,
    ) {
        self.binding = stable.then_some(SessionBinding {
            account_key,
            generation,
        });
    }

    /// Call after binding a fresh Reader response. Missing permission fields
    /// remain a stable Unknown; only an unstable account/generation needs a read.
    pub(super) fn needs_refresh(&self) -> bool {
        self.binding.is_none()
    }

    pub(super) fn is_current(&self, account_key: &Option<String>, generation: u64) -> bool {
        self.binding.as_ref().is_some_and(|binding| {
            binding.account_key == *account_key && binding.generation == generation
        })
    }

    pub(super) fn content_status(
        &self,
        text: &str,
        account_key: &Option<String>,
        generation: u64,
    ) -> ContentStatus {
        if !self.is_current(account_key, generation) {
            return ContentStatus::Unknown;
        }
        self.status_for_text(text)
    }

    pub(super) fn status_for_text(&self, text: &str) -> ContentStatus {
        let visible = text
            .chars()
            .filter(|ch| !ch.is_whitespace() && !ch.is_control())
            .count();
        match self.evidence {
            Evidence::Restricted | Evidence::TrialEdition => {
                if visible == 0 {
                    ContentStatus::Locked
                } else {
                    ContentStatus::Preview
                }
            }
            Evidence::Granted
                if visible > 0 && !obviously_short(text, visible, self.expected_words) =>
            {
                ContentStatus::Full
            }
            Evidence::Granted | Evidence::Unknown => ContentStatus::Unknown,
        }
    }
}

fn access_evidence(
    book: &Map<String, Value>,
    chapter: &Map<String, Value>,
    summary: Option<&Value>,
    account_verified: bool,
    now: i64,
) -> Evidence {
    let paid = flag(book.get("paid"));
    if paid == Some(true) {
        return if account_verified {
            Evidence::Granted
        } else {
            Evidence::Unknown
        };
    }
    let Some(pay_type) = integer(book.get("payType")) else {
        return Evidence::Unknown;
    };
    if pay_type & TRIAL_EDITION_BIT != 0 {
        // ChapterContent has no separate edition scope. Preserve the known
        // edition limit as Preview so an entire visible sample cannot become
        // a claim that the full book has been acquired.
        return if paid == Some(false) {
            Evidence::TrialEdition
        } else {
            Evidence::Unknown
        };
    }
    if pay_type & (FREE_BOOK_BIT | LIMITED_FREE_BOOK_BIT) != 0 {
        return Evidence::Granted;
    }
    let free_prefix = match (
        integer(book.get("maxFreeChapter")),
        integer(chapter.get("chapterIdx")),
    ) {
        (Some(limit), Some(index)) if limit >= index => Truth::Yes,
        (Some(_), Some(_)) => Truth::No,
        _ => Truth::Unknown,
    };
    if free_prefix == Truth::Yes {
        return Evidence::Granted;
    }
    let chapter_purchase = pay_type & CHAPTER_PURCHASE_BIT != 0;
    let price = nonnegative_number(chapter.get("price"));
    let chapter_paid = flag(chapter.get("paid"));
    if chapter_purchase && price == Some(0.0) {
        return Evidence::Granted;
    }
    if chapter_purchase && chapter_paid == Some(true) {
        return if account_verified {
            Evidence::Granted
        } else {
            Evidence::Unknown
        };
    }
    let member = membership_access(book, summary, account_verified, now);
    if member == Truth::Yes {
        return Evidence::Granted;
    }
    let chapter_charge_known = pay_type & WHOLE_PURCHASE_BIT != 0
        || (chapter_purchase
            && price.is_some_and(|price| price > 0.0)
            && chapter_paid == Some(false));
    if paid == Some(false)
        && free_prefix == Truth::No
        && member == Truth::No
        && chapter_charge_known
    {
        Evidence::Restricted
    } else {
        Evidence::Unknown
    }
}

fn membership_access(
    book: &Map<String, Value>,
    summary: Option<&Value>,
    account_verified: bool,
    now: i64,
) -> Truth {
    let Some(summary) = summary
        .and_then(Value::as_object)
        .filter(|summary| !summary.is_empty())
    else {
        // The official SSR fallback may produce an empty summary after a timeout.
        return Truth::Unknown;
    };
    if !account_verified {
        return Truth::Unknown;
    }
    let paying_status = integer(book.get("payingStatus"));
    let short_time = match book.get("shortTimeRead") {
        None => Truth::No,
        Some(value) => active_flag(value.get("active")),
    };
    if short_time == Truth::Yes {
        return Truth::Yes;
    }
    let free_list = match summary.get("freeBookIds").and_then(Value::as_array) {
        Some(ids)
            if book
                .get("bookId")
                .is_some_and(|book_id| ids.iter().any(|id| same_scalar(id, book_id))) =>
        {
            Truth::Yes
        }
        Some(_) => Truth::No,
        None => Truth::Unknown,
    };
    if free_list == Truth::Yes {
        return Truth::Yes;
    }
    let no_special_grant = short_time == Truth::No && free_list == Truth::No;
    let Some(pay_type) = integer(book.get("payType")) else {
        return Truth::Unknown;
    };
    let Some(paying_status) = paying_status else {
        return Truth::Unknown;
    };
    let is_paying = flag(summary.get("isPaying"));
    let expired = flag(summary.get("expired"));
    let expiry = nonnegative_number(summary.get("expiredTime"));
    if (summary.contains_key("isPaying") && is_paying.is_none())
        || (summary.contains_key("expired") && expired.is_none())
        || (summary.contains_key("expiredTime") && expiry.is_none())
    {
        return Truth::Unknown;
    }
    if is_paying == Some(true) {
        // A paid-card flag contradicting explicit expiry evidence is not a
        // sufficient basis for granting access from an inconsistent snapshot.
        if expired == Some(true) || expiry.is_some_and(|expiry| expiry <= now as f64) {
            return Truth::Unknown;
        }
    }
    let member_valid = match (expired, expiry) {
        (Some(false), Some(expiry)) if expiry > 0.0 && (now as f64) < expiry => Truth::Yes,
        (Some(true), _) => Truth::No,
        (_, Some(expiry)) if expiry == 0.0 || (now as f64) >= expiry => Truth::No,
        _ => Truth::Unknown,
    };
    if paying_status == 5 {
        // The official shortcut does not validate membership fields itself.
        // This client requires coherent, active evidence before using it.
        return if member_valid == Truth::Yes {
            Truth::Yes
        } else {
            Truth::Unknown
        };
    }
    if pay_type & CHAPTER_PURCHASE_BIT != 0 || paying_status == 6 {
        return if no_special_grant {
            Truth::No
        } else {
            Truth::Unknown
        };
    }
    // Zero is the official Unknown state; unreviewed values cannot grant access.
    if !matches!(paying_status, 1..=4) {
        return Truth::Unknown;
    }
    if is_paying == Some(true) {
        return if member_valid == Truth::Yes {
            Truth::Yes
        } else {
            Truth::Unknown
        };
    }
    if matches!(paying_status, 1 | 3) && member_valid == Truth::Yes {
        return Truth::Yes;
    }
    if is_paying == Some(false)
        && (matches!(paying_status, 2 | 4) || member_valid == Truth::No)
        && no_special_grant
    {
        Truth::No
    } else {
        Truth::Unknown
    }
}

fn active_flag(value: Option<&Value>) -> Truth {
    match value {
        Some(Value::Bool(true)) => Truth::Yes,
        Some(Value::Bool(false)) => Truth::No,
        value if flag(value) == Some(true) => Truth::Yes,
        value if flag(value) == Some(false) => Truth::No,
        _ => Truth::Unknown,
    }
}

fn nonnegative_number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn integer(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    if let Some(value) = value.as_u64() {
        return Some(value);
    }
    // JSON integers keep their exact identity. Only safe integral floating
    // values may be normalized, including flags written as 1.0.
    nonnegative_number(Some(value))
        .filter(|value| value.fract() == 0.0 && *value <= 9_007_199_254_740_991.0)
        .map(|value| value as u64)
}

fn flag(value: Option<&Value>) -> Option<bool> {
    match nonnegative_number(value)? {
        0.0 => Some(false),
        1.0 => Some(true),
        _ => None,
    }
}

fn identifier(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) if !value.is_empty() && !value.chars().any(char::is_control) => {
            Some(value.clone())
        }
        value @ Value::Number(_) => integer(Some(value)).map(|value| value.to_string()),
        _ => None,
    }
}

fn same_scalar(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(_), Value::Number(_)) => {
            if let (Some(left), Some(right)) = (integer(Some(left)), integer(Some(right))) {
                return left == right;
            }
            match (left.as_f64(), right.as_f64()) {
                (Some(left), Some(right))
                    if left.abs() <= 9_007_199_254_740_991.0
                        && right.abs() <= 9_007_199_254_740_991.0 =>
                {
                    left == right
                }
                _ => false,
            }
        }
        _ => left == right,
    }
}

fn obviously_short(text: &str, visible: usize, expected: Option<u64>) -> bool {
    if text
        .trim()
        .chars()
        .all(|ch| ch.is_whitespace() || matches!(ch, '.' | '…' | '⋯' | '．'))
    {
        return true;
    }
    let Some(expected) = expected.filter(|expected| *expected >= 400) else {
        return false;
    };
    // Catalog counts are not an equality contract. Only a gross mismatch is
    // suspicious; it can withhold Full but never prove a lock or a preview.
    let severe_mismatch = (visible as u128) * 4 < u128::from(expected);
    let ellipsis = ["...", "…", "⋯"]
        .iter()
        .any(|ending| text.trim_end().ends_with(*ending));
    severe_mismatch || (ellipsis && (visible as u128) * 2 < u128::from(expected))
}

#[cfg(test)]
#[path = "access_tests.rs"]
mod tests;
