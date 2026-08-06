//! Secret masking for the copy of a transcript that leaves the app.
//!
//! Scope, stated honestly: this is a small set of regex matchers for things
//! that have a checkable shape or an explicit spoken cue. It finds
//!
//! - card-shaped digit runs that pass a Luhn check,
//! - US SSN-shaped groups (3-2-4) in a valid number range, plus bare nine-digit
//!   runs only when an "SSN" / "social security" cue sits in front of them,
//! - well-known API key prefixes (`sk-`, `ghp_`, `xoxb-`, `AKIA`, …),
//! - the value after a keyword cue such as "password is", "the key is",
//!   "pin is".
//!
//! It does NOT find names, addresses, phone numbers, dates of birth, account
//! numbers, or anything else that needs a model or a dictionary to recognise.
//! It is a filter for obvious secrets read aloud in a meeting, not PII
//! detection, and nothing here should be described as making a transcript safe.
//!
//! The stored transcript is never modified. Callers redact the copy they are
//! about to hand to a model, an export, or a share action.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;

pub const MASK_CARD: &str = "[redacted: card]";
pub const MASK_SSN: &str = "[redacted: ssn]";
pub const MASK_KEY: &str = "[redacted: key]";
pub const MASK_SECRET: &str = "[redacted: secret]";

/// What a redaction pass masked, for the "N items masked" line in the UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RedactionReport {
    pub cards: usize,
    pub ssns: usize,
    pub keys: usize,
    pub secrets: usize,
}

impl RedactionReport {
    pub fn total(&self) -> usize {
        self.cards + self.ssns + self.keys + self.secrets
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

// Card shapes, as an alternation rather than one loose digit-and-separator run.
// `\b` on both ends is what keeps a match from starting or ending in the middle
// of a longer digit run (Rust's regex crate has no look-around, and does not
// need it here: digits are word characters).
//
// Order matters: grouped forms come first so a leftmost-first engine does not
// settle for a shorter contiguous match.
static CARD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"\b(?:[0-9]{4}[ -][0-9]{4}[ -][0-9]{4}[ -][0-9]{4}[ -][0-9]{1,3}|[0-9]{4}[ -][0-9]{4}[ -][0-9]{4}[ -][0-9]{4}|[0-9]{4}[ -][0-9]{6}[ -][0-9]{5}|[0-9]{13,19})\b",
    )
    .expect("card regex")
});

// 3-2-4 with one separator style throughout. Written as two alternatives
// because the regex crate has no backreferences, which also means a mixed
// "123-45 6789" is correctly left alone.
static SSN_DASHED: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(?:([0-9]{3})-([0-9]{2})-([0-9]{4})|([0-9]{3}) ([0-9]{2}) ([0-9]{4}))\b")
        .expect("ssn regex")
});

// A bare nine-digit run only counts when the sentence says what it is.
static SSN_CUED: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(ssn|social security(?:\s+number)?)\b([^0-9\n]{0,20})([0-9]{9})\b")
        .expect("cued ssn regex")
});

// Vendor key shapes that are unambiguous on their own.
static KEY_PREFIX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"\b(?:sk-[A-Za-z0-9_-]{16,}|sk_live_[A-Za-z0-9]{10,}|rk_live_[A-Za-z0-9]{10,}|ghp_[A-Za-z0-9]{20,}|gho_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{30,})\b",
    )
    .expect("key prefix regex")
});

// "the password is hunter2", "pin: 4821", "api key = abc123".
//
// The value is a single run of non-space characters, so a sentence like
// "the password is in 1Password" masks one word: it reads oddly and leaks
// nothing, which is the trade this matcher makes. The same trade means the
// English idiom "the key is to ship early" masks the word "to". Multi-word
// nouns come first in the alternation so "api key" wins over bare "key".
static CUED_SECRET: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b((?:the\s+|my\s+|our\s+|your\s+)?(?:api\s*key|secret\s*key|access\s*key|password|passwd|passphrase|passcode|pin|secret|token|credentials?|key)\s*(?:is|are|was|=|:)\s*)(?:['\x22]?)([^\s'\x22]{3,120})",
    )
    .expect("cued secret regex")
});

fn luhn_valid(digits: &str) -> bool {
    let digits: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
    if digits.len() < 13 || digits.len() > 19 {
        return false;
    }
    let mut sum = 0u32;
    for (index, digit) in digits.iter().rev().enumerate() {
        if index % 2 == 1 {
            let doubled = digit * 2;
            sum += if doubled > 9 { doubled - 9 } else { doubled };
        } else {
            sum += digit;
        }
    }
    sum % 10 == 0
}

/// SSN ranges the Social Security Administration never issues. Filtering these
/// keeps ordinary 3-2-4 shaped reference numbers out of the mask.
fn plausible_ssn(area: &str, group: &str, serial: &str) -> bool {
    let area_num: u32 = area.parse().unwrap_or(0);
    if area_num == 0 || area_num == 666 || area_num >= 900 {
        return false;
    }
    group != "00" && serial != "0000"
}

/// Masks obvious secrets in `text`, returning the masked copy and a count of
/// what was replaced. `text` is never mutated in place and the caller's stored
/// copy is untouched.
pub fn redact(text: &str) -> (String, RedactionReport) {
    let mut report = RedactionReport::default();

    // Keys and cued secrets run first: a key can contain a long digit run that
    // would otherwise be nibbled by the card matcher.
    let masked = KEY_PREFIX.replace_all(text, |_: &regex::Captures| {
        report.keys += 1;
        MASK_KEY.to_string()
    });

    let masked = CUED_SECRET.replace_all(&masked, |caps: &regex::Captures| {
        let value = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        // Already masked by an earlier pass, or a sentence that names the thing
        // without giving it ("the password is stored in 1Password" still masks
        // one word; "[redacted: key]" must not be masked twice).
        if value.starts_with("[redacted") {
            return caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default();
        }
        report.secrets += 1;
        format!("{}{}", caps.get(1).map(|m| m.as_str()).unwrap_or(""), MASK_SECRET)
    });

    let masked = SSN_CUED.replace_all(&masked, |caps: &regex::Captures| {
        report.ssns += 1;
        format!(
            "{}{}{}",
            caps.get(1).map(|m| m.as_str()).unwrap_or(""),
            caps.get(2).map(|m| m.as_str()).unwrap_or(" "),
            MASK_SSN
        )
    });

    let masked = SSN_DASHED.replace_all(&masked, |caps: &regex::Captures| {
        // Groups 1-3 are the hyphenated form, 4-6 the spaced one; exactly one
        // alternative participates in any given match.
        let part = |dashed: usize, spaced: usize| {
            caps.get(dashed)
                .or_else(|| caps.get(spaced))
                .map(|m| m.as_str())
                .unwrap_or("")
        };
        let area = part(1, 4);
        let group = part(2, 5);
        let serial = part(3, 6);
        if !plausible_ssn(area, group, serial) {
            return caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default();
        }
        report.ssns += 1;
        MASK_SSN.to_string()
    });

    let masked = CARD.replace_all(&masked, |caps: &regex::Captures| {
        let matched = caps.get(0).map(|m| m.as_str()).unwrap_or("");
        let digits: String = matched.chars().filter(|c| c.is_ascii_digit()).collect();
        if !luhn_valid(&digits) {
            return matched.to_string();
        }
        report.cards += 1;
        MASK_CARD.to_string()
    });

    (masked.into_owned(), report)
}

/// Redacts only when the resolved profile asks for it. Returns the text
/// unchanged (and an empty report) otherwise, so callers can wire this in
/// without branching.
pub fn redact_if(enabled: bool, text: &str) -> (String, RedactionReport) {
    if !enabled {
        return (text.to_string(), RedactionReport::default());
    }
    redact(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masked(text: &str) -> String {
        redact(text).0
    }

    // -- cards ------------------------------------------------------------

    #[test]
    fn luhn_valid_cards_are_masked_in_every_grouping() {
        for card in [
            "4111111111111111",
            "4111 1111 1111 1111",
            "4111-1111-1111-1111",
            "5500005555555559",
            "378282246310005",       // 15-digit Amex
            "3056930009020004",      // Diners
            "6011111111111117",      // Discover
        ] {
            let out = masked(&format!("card is {card} thanks"));
            assert!(out.contains(MASK_CARD), "{card} should be masked, got {out}");
            assert!(!out.contains("1111 1111"), "{card} digits survived: {out}");
        }
    }

    #[test]
    fn card_shaped_numbers_that_fail_luhn_are_left_alone() {
        for near_miss in [
            "4111111111111112",        // one digit off
            "1234567890123456",        // sequential
            "1234567890123",           // 13 digits, invalid
            "9999999999999999",
        ] {
            let out = masked(&format!("order {near_miss} shipped"));
            assert!(!out.contains(MASK_CARD), "{near_miss} must not be masked: {out}");
            assert!(out.contains(near_miss), "{near_miss} must survive: {out}");
        }
    }

    #[test]
    fn phone_numbers_and_dates_and_amounts_are_not_cards() {
        for text in [
            "call me on (555) 123-4567",
            "call 555-123-4567 tomorrow",
            "the date is 2026-08-06",
            "invoice 12345 for 1200 dollars",
            "we shipped 100000 units",
            "meeting at 09:30 on 06/08/2026",
        ] {
            let out = masked(text);
            assert_eq!(out, text, "{text} must pass through unchanged");
        }
    }

    #[test]
    fn a_long_digit_run_glued_to_more_digits_is_not_split_into_a_card() {
        // 20 digits: nothing inside should be treated as a card.
        let text = "reference 41111111111111110000 filed";
        assert_eq!(masked(text), text);
    }

    // -- SSNs -------------------------------------------------------------

    #[test]
    fn dashed_and_spaced_ssns_are_masked() {
        assert!(masked("his ssn is 123-45-6789").contains(MASK_SSN));
        assert!(masked("123 45 6789 is the number").contains(MASK_SSN));
        assert!(!masked("his ssn is 123-45-6789").contains("6789"));
    }

    #[test]
    fn impossible_ssn_ranges_are_left_alone() {
        for text in [
            "part 000-45-6789 in stock",
            "part 666-45-6789 in stock",
            "part 900-45-6789 in stock",
            "part 123-00-6789 in stock",
            "part 123-45-0000 in stock",
        ] {
            let out = masked(text);
            assert!(!out.contains(MASK_SSN), "{text} must not be masked: {out}");
        }
    }

    #[test]
    fn bare_nine_digit_runs_need_a_cue() {
        // No cue: an order number stays put.
        let plain = "order 123456789 shipped";
        assert_eq!(masked(plain), plain);
        // With a cue it is masked.
        let cued = masked("SSN 123456789 on file");
        assert!(cued.contains(MASK_SSN), "{cued}");
        let spelled = masked("social security number: 123456789");
        assert!(spelled.contains(MASK_SSN), "{spelled}");
    }

    #[test]
    fn mixed_separator_groups_are_not_treated_as_ssns() {
        let text = "code 123-45 6789 here";
        assert_eq!(masked(text), text);
    }

    // -- keys -------------------------------------------------------------

    #[test]
    fn known_key_shapes_are_masked_without_a_cue() {
        for key in [
            "sk-abcdefghijklmnopqrstuvwx",
            "ghp_abcdefghijklmnopqrstuvwxyz12",
            "xoxb-1234567890-abcdefghijkl",
            "AKIAIOSFODNN7EXAMPLE",
        ] {
            let out = masked(&format!("use {key} for now"));
            assert!(out.contains(MASK_KEY), "{key} should be masked: {out}");
            assert!(!out.contains(key), "{key} survived: {out}");
        }
    }

    #[test]
    fn ordinary_hyphenated_words_are_not_keys() {
        for text in [
            "sk-1 is our sprint name",
            "the ticket is ABC-1234",
            "check-in at nine",
            "part number AKIA12",
        ] {
            assert_eq!(masked(text), text, "{text}");
        }
    }

    // -- cued secrets -----------------------------------------------------

    #[test]
    fn values_after_a_keyword_cue_are_masked() {
        for text in [
            "the password is hunter2",
            "password: hunter2",
            "my passcode is 4821x",
            "the pin is 4821",
            "the key is abc123def",
            "api key = zzz9999zzz",
            "token is abcd-efgh-ijkl",
        ] {
            let out = masked(text);
            assert!(out.contains(MASK_SECRET), "{text} should be masked: {out}");
            assert!(!out.contains("hunter2") || !text.contains("hunter2"), "{out}");
        }
    }

    #[test]
    fn the_cue_word_alone_masks_nothing() {
        for text in [
            "this document is password protected",
            "we should rotate the api key next quarter",
            "the pin badge on the wall",
            "her token of appreciation",
        ] {
            let out = masked(text);
            assert_eq!(out, text, "{text} must pass through unchanged");
        }
    }

    #[test]
    fn a_cue_followed_by_prose_masks_at_most_the_next_word() {
        // Documented limitation, not a bug: the matcher cannot tell a secret
        // from the next word. The three-character minimum keeps the common
        // idioms intact ("the key is to ship early")...
        let idiom = "the key is to ship early";
        assert_eq!(masked(idiom), idiom);
        // ...but a longer following word is masked, and only that word.
        let out = masked("the key is trust between the teams");
        assert!(out.contains(MASK_SECRET), "{out}");
        assert!(out.contains("between the teams"), "only the one word goes: {out}");
    }

    #[test]
    fn the_cue_and_the_verb_survive_so_the_sentence_still_reads() {
        let out = masked("the password is hunter2 and it expires Friday");
        assert!(out.starts_with("the password is [redacted: secret]"), "{out}");
        assert!(out.ends_with("and it expires Friday"), "{out}");
    }

    #[test]
    fn masking_is_idempotent() {
        let once = masked("the password is hunter2, card 4111111111111111, ssn 123-45-6789");
        let twice = masked(&once);
        assert_eq!(once, twice);
    }

    // -- reporting and plumbing -------------------------------------------

    #[test]
    fn the_report_counts_each_kind() {
        let (_, report) = redact(
            "card 4111111111111111 and 5500005555555559, ssn 123-45-6789, \
             the password is hunter2, key sk-abcdefghijklmnopqrstuvwx",
        );
        assert_eq!(report.cards, 2);
        assert_eq!(report.ssns, 1);
        assert_eq!(report.secrets, 1);
        assert_eq!(report.keys, 1);
        assert_eq!(report.total(), 5);
        assert!(!report.is_empty());
    }

    #[test]
    fn clean_text_reports_nothing_and_comes_back_identical() {
        let text = "We agreed to ship on Friday and revisit pricing in Q3.";
        let (out, report) = redact(text);
        assert_eq!(out, text);
        assert!(report.is_empty());
    }

    #[test]
    fn redaction_is_skipped_when_the_profile_does_not_ask_for_it() {
        let text = "card 4111111111111111";
        let (out, report) = redact_if(false, text);
        assert_eq!(out, text);
        assert!(report.is_empty());
        let (out, report) = redact_if(true, text);
        assert!(out.contains(MASK_CARD));
        assert_eq!(report.cards, 1);
    }

    #[test]
    fn multiline_transcripts_keep_their_shape() {
        let text = "Alice: the card is 4111111111111111\nBob: got it\nAlice: ssn 123-45-6789";
        let out = masked(text);
        assert_eq!(out.lines().count(), 3);
        assert!(out.lines().nth(1).unwrap().contains("got it"));
        assert!(out.contains(MASK_CARD));
        assert!(out.contains(MASK_SSN));
    }
}
