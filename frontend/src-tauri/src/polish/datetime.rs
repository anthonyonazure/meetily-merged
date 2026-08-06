//! Spoken dates and times to one consistent written form.
//!
//! A transcript writes the same moment five ways: "three thirty p m", "3:30pm",
//! "3:30 p.m.", "half three", "fifteen thirty". Notes and deliverables should
//! write it once, the same way: `3:30 PM`.
//!
//! As with numbers, the discipline is that every conversion needs an anchor that
//! rules out the alternative reading:
//!
//! - A **meridiem** (am/pm in any spelling) anchors a time. Without one,
//!   "three thirty" could be a score, a version, or a room number, so it is left
//!   alone.
//! - **"o'clock"** anchors an hour; nothing else follows it.
//! - A **month name** anchors a day. "August fifth" is a date; a bare "fifth" is
//!   not.
//!
//! Nothing here guesses a year, and nothing converts a bare pair of numbers.

use once_cell::sync::Lazy;
use regex::Regex;

const MONTHS: &[&str] = &[
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// Spoken ordinals for days of the month.
const ORDINALS: &[(&str, u32)] = &[
    ("first", 1),
    ("second", 2),
    ("third", 3),
    ("fourth", 4),
    ("fifth", 5),
    ("sixth", 6),
    ("seventh", 7),
    ("eighth", 8),
    ("ninth", 9),
    ("tenth", 10),
    ("eleventh", 11),
    ("twelfth", 12),
    ("thirteenth", 13),
    ("fourteenth", 14),
    ("fifteenth", 15),
    ("sixteenth", 16),
    ("seventeenth", 17),
    ("eighteenth", 18),
    ("nineteenth", 19),
    ("twentieth", 20),
    ("twenty-first", 21),
    ("twenty-second", 22),
    ("twenty-third", 23),
    ("twenty-fourth", 24),
    ("twenty-fifth", 25),
    ("twenty-sixth", 26),
    ("twenty-seventh", 27),
    ("twenty-eighth", 28),
    ("twenty-ninth", 29),
    ("thirtieth", 30),
    ("thirty-first", 31),
];

const HOUR_WORDS: &[(&str, u32)] = &[
    ("one", 1),
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
];

/// Minute words that can follow an hour: "oh five", "fifteen", "thirty",
/// "forty five". Written as the whole spoken minute phrase so the match is exact
/// rather than arithmetic on ambiguous fragments.
const MINUTE_PHRASES: &[(&str, u32)] = &[
    ("oh one", 1),
    ("oh two", 2),
    ("oh three", 3),
    ("oh four", 4),
    ("oh five", 5),
    ("oh six", 6),
    ("oh seven", 7),
    ("oh eight", 8),
    ("oh nine", 9),
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
    ("thirteen", 13),
    ("fourteen", 14),
    ("fifteen", 15),
    ("sixteen", 16),
    ("seventeen", 17),
    ("eighteen", 18),
    ("nineteen", 19),
    ("twenty", 20),
    ("twenty one", 21),
    ("twenty-one", 21),
    ("twenty five", 25),
    ("twenty-five", 25),
    ("thirty", 30),
    ("thirty five", 35),
    ("thirty-five", 35),
    ("forty", 40),
    ("forty five", 45),
    ("forty-five", 45),
    ("fifty", 50),
    ("fifty five", 55),
    ("fifty-five", 55),
];

/// The meridiem tail, shared by both time patterns.
///
/// `m(?:\.\B|\b)` rather than `m\.?\b`: the trailing `\b` alone cannot follow a
/// consumed full stop (two non-word characters are not a boundary), so
/// "ten a.m." would match only "a.m" and leave a stray dot behind. `\.\B`
/// consumes the abbreviation's dot, while the plain `\b` alternative still refuses
/// to match inside a word like "amp".
const MERIDIEM_TAIL: &str = r"([ap])\.?\s*m(?:\.\B|\b)";

/// `3:30pm`, `3:30 p.m.`, `10 AM` — already digits, inconsistent presentation.
static DIGIT_TIME: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"(?i)\b(\d{{1,2}})(?::([0-5]\d))?\s*{MERIDIEM_TAIL}"
    ))
    .expect("valid regex")
});

/// `three thirty p m`, `three p.m.`, `eleven forty five pm`.
static SPOKEN_TIME: Lazy<Regex> = Lazy::new(|| {
    let hours = HOUR_WORDS
        .iter()
        .map(|(w, _)| *w)
        .collect::<Vec<_>>()
        .join("|");
    // Longest minute phrases first so "twenty five" wins over "twenty".
    let mut minute_phrases: Vec<&str> = MINUTE_PHRASES.iter().map(|(w, _)| *w).collect();
    minute_phrases.sort_by_key(|p| std::cmp::Reverse(p.len()));
    let minutes = minute_phrases.join("|");
    Regex::new(&format!(
        r"(?i)\b({hours})(?:\s+({minutes}))?\s+{MERIDIEM_TAIL}"
    ))
    .expect("valid regex")
});

/// `three o'clock`, `three oclock`.
static OCLOCK: Lazy<Regex> = Lazy::new(|| {
    let hours = HOUR_WORDS
        .iter()
        .map(|(w, _)| *w)
        .collect::<Vec<_>>()
        .join("|");
    // The apostrophe class is written literally: inside a `format!` template a
    // `\u{...}` escape is read as a positional argument, not as a character.
    Regex::new(&format!(r"(?i)\b({hours})\s+o['’]?\s?clock\b")).expect("valid regex")
});

/// `August fifth`, `August 5th`, `the fifth of August`.
static MONTH_ORDINAL: Lazy<Regex> = Lazy::new(|| {
    let months = MONTHS.join("|");
    let ordinals = ORDINALS
        .iter()
        .map(|(w, _)| *w)
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!(
        r"(?i)\b({months})\s+(?:the\s+)?({ordinals}|\d{{1,2}}(?:st|nd|rd|th)?)\b"
    ))
    .expect("valid regex")
});

static ORDINAL_OF_MONTH: Lazy<Regex> = Lazy::new(|| {
    let months = MONTHS.join("|");
    let ordinals = ORDINALS
        .iter()
        .map(|(w, _)| *w)
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!(
        r"(?i)\b(?:the\s+)?({ordinals}|\d{{1,2}}(?:st|nd|rd|th)?)\s+of\s+({months})\b"
    ))
    .expect("valid regex")
});

fn hour_value(word: &str) -> Option<u32> {
    let lowered = word.to_lowercase();
    HOUR_WORDS
        .iter()
        .find(|(w, _)| *w == lowered)
        .map(|(_, v)| *v)
}

fn minute_value(phrase: &str) -> Option<u32> {
    let lowered = phrase.to_lowercase();
    MINUTE_PHRASES
        .iter()
        .find(|(w, _)| *w == lowered)
        .map(|(_, v)| *v)
}

fn day_value(text: &str) -> Option<u32> {
    let lowered = text.to_lowercase();
    if let Some((_, v)) = ORDINALS.iter().find(|(w, _)| *w == lowered) {
        return Some(*v);
    }
    let digits: String = lowered.chars().take_while(|c| c.is_ascii_digit()).collect();
    let value: u32 = digits.parse().ok()?;
    (1..=31).contains(&value).then_some(value)
}

fn title_case_month(month: &str) -> String {
    let lowered = month.to_lowercase();
    let mut chars = lowered.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => lowered,
    }
}

/// The canonical written time: `3:30 PM`, and `10 AM` when the minutes are zero.
fn format_time(hour: u32, minutes: u32, meridiem: &str) -> String {
    let suffix = if meridiem.eq_ignore_ascii_case("a") {
        "AM"
    } else {
        "PM"
    };
    if minutes == 0 {
        format!("{} {}", hour, suffix)
    } else {
        format!("{}:{:02} {}", hour, minutes, suffix)
    }
}

/// Normalizes spoken and inconsistently-written dates and times.
pub fn normalize_datetimes(text: &str) -> String {
    // Spoken times before digit times, so "three thirty p m" becomes "3:30 PM"
    // and is not touched again.
    let out = SPOKEN_TIME.replace_all(text, |caps: &regex::Captures| {
        let Some(hour) = hour_value(&caps[1]) else {
            return caps[0].to_string();
        };
        let minutes = match caps.get(2) {
            Some(m) => match minute_value(m.as_str()) {
                Some(v) => v,
                None => return caps[0].to_string(),
            },
            None => 0,
        };
        format_time(hour, minutes, &caps[3])
    });

    let out = DIGIT_TIME.replace_all(&out, |caps: &regex::Captures| {
        let hour: u32 = match caps[1].parse() {
            Ok(h) if (1..=12).contains(&h) => h,
            // 0 or 13-23 with a meridiem is a transcription artefact, not a time
            // this pass will reinterpret.
            _ => return caps[0].to_string(),
        };
        let minutes: u32 = caps.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        format_time(hour, minutes, &caps[3])
    });

    let out = OCLOCK.replace_all(&out, |caps: &regex::Captures| match hour_value(&caps[1]) {
        Some(hour) => format!("{}:00", hour),
        None => caps[0].to_string(),
    });

    let out = ORDINAL_OF_MONTH.replace_all(&out, |caps: &regex::Captures| {
        match day_value(&caps[1]) {
            Some(day) => format!("{} {}", title_case_month(&caps[2]), day),
            None => caps[0].to_string(),
        }
    });

    let out = MONTH_ORDINAL.replace_all(&out, |caps: &regex::Captures| {
        match day_value(&caps[2]) {
            Some(day) => format!("{} {}", title_case_month(&caps[1]), day),
            None => caps[0].to_string(),
        }
    });

    out.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- times -----------------------------------------------------------

    #[test]
    fn a_spoken_time_with_a_meridiem_becomes_canonical() {
        assert_eq!(normalize_datetimes("at three thirty p m"), "at 3:30 PM");
        assert_eq!(normalize_datetimes("at three thirty pm"), "at 3:30 PM");
        assert_eq!(normalize_datetimes("by ten a.m."), "by 10 AM");
        assert_eq!(
            normalize_datetimes("call at eleven forty five am"),
            "call at 11:45 AM"
        );
        assert_eq!(normalize_datetimes("nine oh five pm"), "9:05 PM");
    }

    #[test]
    fn digit_times_are_normalized_to_one_presentation() {
        assert_eq!(normalize_datetimes("3:30pm"), "3:30 PM");
        assert_eq!(normalize_datetimes("3:30 p.m."), "3:30 PM");
        assert_eq!(normalize_datetimes("10 AM"), "10 AM");
        assert_eq!(normalize_datetimes("10am"), "10 AM");
        assert_eq!(normalize_datetimes("12:00 a m"), "12 AM");
    }

    #[test]
    fn oclock_becomes_a_written_hour() {
        assert_eq!(normalize_datetimes("at three o'clock"), "at 3:00");
        assert_eq!(normalize_datetimes("at three oclock"), "at 3:00");
        assert_eq!(normalize_datetimes("at three o clock"), "at 3:00");
    }

    #[test]
    fn a_time_keeps_its_surrounding_sentence() {
        assert_eq!(
            normalize_datetimes("The window opens at ten p m and closes at two a m"),
            "The window opens at 10 PM and closes at 2 AM"
        );
    }

    // ---- dates -----------------------------------------------------------

    #[test]
    fn a_month_and_ordinal_becomes_month_and_day() {
        assert_eq!(normalize_datetimes("on August fifth"), "on August 5");
        assert_eq!(normalize_datetimes("on august fifth"), "on August 5");
        assert_eq!(normalize_datetimes("on August 5th"), "on August 5");
        assert_eq!(normalize_datetimes("by December twenty-first"), "by December 21");
    }

    #[test]
    fn an_ordinal_before_a_month_is_reordered() {
        assert_eq!(normalize_datetimes("the fifth of August"), "August 5");
        assert_eq!(normalize_datetimes("on the 21st of December"), "on December 21");
    }

    // ---- what must NOT be transformed ------------------------------------

    #[test]
    fn a_bare_pair_of_numbers_is_not_a_time() {
        // No meridiem anchor: could be a score, a ratio, a room.
        assert_eq!(normalize_datetimes("three thirty"), "three thirty");
        assert_eq!(normalize_datetimes("the score was 3:30"), "the score was 3:30");
        assert_eq!(normalize_datetimes("fifteen thirty"), "fifteen thirty");
    }

    #[test]
    fn a_bare_ordinal_is_not_a_date() {
        assert_eq!(normalize_datetimes("the fifth item"), "the fifth item");
        assert_eq!(normalize_datetimes("came third"), "came third");
    }

    #[test]
    fn a_month_word_used_as_a_name_is_not_given_a_day() {
        // "May" with no day after it stays a word.
        assert_eq!(normalize_datetimes("May be worth checking"), "May be worth checking");
        assert_eq!(normalize_datetimes("April joined the call"), "April joined the call");
    }

    #[test]
    fn version_numbers_and_ranges_survive() {
        assert_eq!(normalize_datetimes("version 3:30"), "version 3:30");
        assert_eq!(normalize_datetimes("ports 10 to 20"), "ports 10 to 20");
        assert_eq!(normalize_datetimes("a one to one meeting"), "a one to one meeting");
    }

    #[test]
    fn a_24_hour_reading_with_a_stray_meridiem_is_left_alone() {
        // 15 PM is a transcription artefact. Reinterpreting it would be a guess.
        assert_eq!(normalize_datetimes("at 15 pm"), "at 15 pm");
        assert_eq!(normalize_datetimes("at 0 am"), "at 0 am");
    }

    #[test]
    fn a_word_containing_a_meridiem_letter_is_not_a_time() {
        assert_eq!(normalize_datetimes("the 3 amp fuse"), "the 3 amp fuse");
        assert_eq!(normalize_datetimes("ten among them"), "ten among them");
    }

    #[test]
    fn empty_input_is_handled() {
        assert_eq!(normalize_datetimes(""), "");
    }

    // ---- helpers ---------------------------------------------------------

    #[test]
    fn time_formatting_drops_zero_minutes() {
        assert_eq!(format_time(10, 0, "a"), "10 AM");
        assert_eq!(format_time(3, 30, "p"), "3:30 PM");
        assert_eq!(format_time(9, 5, "P"), "9:05 PM");
    }

    #[test]
    fn day_parsing_accepts_words_and_digits_and_rejects_the_rest() {
        assert_eq!(day_value("fifth"), Some(5));
        assert_eq!(day_value("21st"), Some(21));
        assert_eq!(day_value("31"), Some(31));
        assert_eq!(day_value("32"), None);
        assert_eq!(day_value("banana"), None);
    }
}
