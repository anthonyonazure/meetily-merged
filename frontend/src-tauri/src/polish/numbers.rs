//! Spoken numbers to digits.
//!
//! "twenty five thousand" is unreadable in a client deliverable and useless in a
//! figure a summary is supposed to carry forward. "25,000" is both.
//!
//! The whole difficulty is that most spoken number words are also parts of names:
//! "Office three sixty five", "Windows Eleven", "Catalyst nine thousand". So this
//! module converts a run of number words only when two things are true:
//!
//! 1. **The run is a quantity, not a label.** It has to contain a scale word
//!    (hundred, thousand, million, billion) or a spoken decimal ("two point five").
//!    That is what distinguishes "twenty five thousand tickets" from
//!    "three sixty five".
//! 2. **Nothing about it looks like a name.** A run preceded by a capitalised word
//!    mid-sentence is a product name until proven otherwise, and so is a run whose
//!    own words are capitalised mid-sentence.
//!
//! Anything failing either test is left exactly as spoken. A missed conversion is
//! a cosmetic loss; a mangled product name is a wrong document.

/// One..nineteen.
const UNITS: &[(&str, i64)] = &[
    ("zero", 0),
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
    ("thirteen", 13),
    ("fourteen", 14),
    ("fifteen", 15),
    ("sixteen", 16),
    ("seventeen", 17),
    ("eighteen", 18),
    ("nineteen", 19),
];

const TENS: &[(&str, i64)] = &[
    ("twenty", 20),
    ("thirty", 30),
    ("forty", 40),
    ("fourty", 40), // a common transcription of "forty"
    ("fifty", 50),
    ("sixty", 60),
    ("seventy", 70),
    ("eighty", 80),
    ("ninety", 90),
];

const SCALES: &[(&str, i64)] = &[
    ("hundred", 100),
    ("thousand", 1_000),
    ("million", 1_000_000),
    ("billion", 1_000_000_000),
];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Word {
    Unit(i64),
    Ten(i64),
    Scale(i64),
    And,
    Point,
}

fn classify(word: &str) -> Option<Word> {
    let lowered = word.to_lowercase();
    if let Some((_, v)) = UNITS.iter().find(|(w, _)| *w == lowered) {
        return Some(Word::Unit(*v));
    }
    if let Some((_, v)) = TENS.iter().find(|(w, _)| *w == lowered) {
        return Some(Word::Ten(*v));
    }
    if let Some((_, v)) = SCALES.iter().find(|(w, _)| *w == lowered) {
        return Some(Word::Scale(*v));
    }
    match lowered.as_str() {
        "and" => Some(Word::And),
        "point" => Some(Word::Point),
        _ => None,
    }
}

/// Strips punctuation from both ends, returning the core word plus what was
/// removed, so "thousand," can be classified and then re-punctuated.
fn peel(token: &str) -> (&str, &str, &str) {
    let start = token
        .find(|c: char| c.is_alphanumeric())
        .unwrap_or(token.len());
    let end = token
        .rfind(|c: char| c.is_alphanumeric())
        .map(|i| i + token[i..].chars().next().map_or(1, char::len_utf8))
        .unwrap_or(token.len());
    if start >= end {
        return ("", token, "");
    }
    (&token[start..end], &token[..start], &token[end..])
}

/// A token is part of a number run if it, or every hyphen-separated part of it, is
/// a number word. This is what lets "twenty-five thousand" work.
fn number_words_in(token: &str) -> Option<Vec<Word>> {
    let (core, _, _) = peel(token);
    if core.is_empty() {
        return None;
    }
    let parts: Vec<&str> = core.split(['-', '\u{2011}']).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    let mut words = Vec::with_capacity(parts.len());
    for part in parts {
        words.push(classify(part)?);
    }
    Some(words)
}

/// True when a token is capitalised in a way that suggests a name rather than a
/// sentence start.
fn looks_like_a_name(token: &str) -> bool {
    let (core, _, _) = peel(token);
    let mut chars = core.chars();
    match chars.next() {
        Some(first) => first.is_uppercase() && chars.any(|c| c.is_lowercase()),
        None => false,
    }
}

/// True when a token closes a clause. A capital after one of these is a sentence
/// start, and a *label* ending in one ("Cost:", "Total:") is not a brand, so
/// neither should suppress a conversion.
fn ends_sentence(token: &str) -> bool {
    token.trim_end().ends_with(['.', '!', '?', ':', ';'])
}

/// True when the token immediately before a run is a name, which makes the run its
/// model number rather than a quantity.
fn is_name_before_run(token: &str) -> bool {
    looks_like_a_name(token) && !ends_sentence(token)
}

/// True when a run's own token is capitalised as a name rather than as the first
/// word of a sentence.
fn is_name_in_run(tokens: &[&str], index: usize) -> bool {
    if !looks_like_a_name(tokens[index]) {
        return false;
    }
    if index == 0 {
        return false;
    }
    !ends_sentence(tokens[index - 1])
}

/// Formats an integer with thousands separators: 25000 -> "25,000".
pub fn with_thousands_separators(value: i64) -> String {
    let negative = value < 0;
    let digits = value.abs().to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    if negative {
        format!("-{}", grouped)
    } else {
        grouped
    }
}

/// Evaluates a run of number words. Returns None when the run does not add up to
/// anything (a bare "and", a lone "point").
fn evaluate(words: &[Word]) -> Option<String> {
    let mut total: i64 = 0;
    let mut current: i64 = 0;
    let mut saw_number = false;
    let mut fraction: Option<String> = None;

    for word in words {
        match word {
            Word::Unit(v) => {
                if let Some(digits) = fraction.as_mut() {
                    // Only single digits are meaningful after "point".
                    if *v > 9 {
                        return None;
                    }
                    digits.push_str(&v.to_string());
                } else {
                    current += v;
                    saw_number = true;
                }
            }
            Word::Ten(v) => {
                if fraction.is_some() {
                    return None;
                }
                current += v;
                saw_number = true;
            }
            Word::Scale(scale) => {
                if !saw_number {
                    // A bare "thousand" with nothing in front of it is not a
                    // quantity this module will invent a 1 for.
                    return None;
                }
                if *scale == 100 {
                    current = if current == 0 { 100 } else { current * 100 };
                } else {
                    let chunk = if current == 0 { 1 } else { current };
                    match fraction.take() {
                        // "two point five million" -> 2.5 * 1_000_000.
                        Some(digits) if !digits.is_empty() => {
                            let value: f64 = format!("{}.{}", chunk, digits).parse().ok()?;
                            total += (value * *scale as f64).round() as i64;
                        }
                        _ => total += chunk * scale,
                    }
                    current = 0;
                }
            }
            Word::And => {}
            Word::Point => {
                if fraction.is_some() || !saw_number {
                    return None;
                }
                fraction = Some(String::new());
            }
        }
    }

    if !saw_number {
        return None;
    }
    total += current;

    match fraction {
        Some(digits) if !digits.is_empty() => Some(format!(
            "{}.{}",
            with_thousands_separators(total),
            digits
        )),
        // A trailing "point" with no digits is not a number we understood.
        Some(_) => None,
        None => Some(with_thousands_separators(total)),
    }
}

/// True when a run is a quantity rather than a label: it needs a scale word or a
/// spoken decimal. This single condition is what protects "Office three sixty
/// five" and "Windows Eleven".
fn is_quantity(words: &[Word]) -> bool {
    words
        .iter()
        .any(|w| matches!(w, Word::Scale(_) | Word::Point))
}

/// Converts spoken quantities in a line to digits, leaving everything else alone.
pub fn normalize_numbers(text: &str) -> String {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.is_empty() {
        return text.trim().to_string();
    }

    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut index = 0usize;

    while index < tokens.len() {
        let Some(first_words) = number_words_in(tokens[index]) else {
            out.push(tokens[index].to_string());
            index += 1;
            continue;
        };

        // Collect the maximal run of number tokens.
        let run_start = index;
        let mut words = first_words;
        let mut run_end = index + 1;
        while run_end < tokens.len() {
            match number_words_in(tokens[run_end]) {
                Some(more) => {
                    words.extend(more);
                    run_end += 1;
                }
                None => break,
            }
        }
        // A run that ends on a connector ("... and") should not swallow it.
        while matches!(words.last(), Some(Word::And)) && run_end > run_start + 1 {
            words.pop();
            run_end -= 1;
        }

        // Guard 1: is this a quantity at all?
        // Guard 2: is the preceding word a name, making this its model number?
        // Guard 3: are the run's own words capitalised mid-sentence?
        let preceded_by_name = run_start > 0 && is_name_before_run(tokens[run_start - 1]);
        let run_is_capitalised =
            (run_start..run_end).any(|position| is_name_in_run(&tokens, position));

        let converted = if is_quantity(&words) && !preceded_by_name && !run_is_capitalised {
            evaluate(&words)
        } else {
            None
        };

        match converted {
            Some(number) => {
                // Keep the punctuation that bracketed the spoken run.
                let (_, prefix, _) = peel(tokens[run_start]);
                let (_, _, suffix) = peel(tokens[run_end - 1]);
                out.push(format!("{}{}{}", prefix, number, suffix));
            }
            None => {
                for token in &tokens[run_start..run_end] {
                    out.push((*token).to_string());
                }
            }
        }
        index = run_end;
    }

    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- what must be converted ------------------------------------------

    #[test]
    fn a_spoken_quantity_becomes_digits_with_separators() {
        assert_eq!(normalize_numbers("twenty five thousand"), "25,000");
        assert_eq!(normalize_numbers("three hundred"), "300");
        assert_eq!(normalize_numbers("one thousand"), "1,000");
        assert_eq!(normalize_numbers("two million"), "2,000,000");
        assert_eq!(
            normalize_numbers("one hundred and fifty thousand"),
            "150,000"
        );
    }

    #[test]
    fn a_quantity_inside_a_sentence_is_converted_in_place() {
        assert_eq!(
            normalize_numbers("The renewal came to twenty five thousand dollars"),
            "The renewal came to 25,000 dollars"
        );
        assert_eq!(
            normalize_numbers("about three hundred seats, roughly"),
            "about 300 seats, roughly"
        );
    }

    #[test]
    fn hyphenated_spoken_numbers_are_handled() {
        assert_eq!(normalize_numbers("twenty-five thousand"), "25,000");
        assert_eq!(normalize_numbers("one hundred twenty-three thousand"), "123,000");
    }

    #[test]
    fn a_spoken_decimal_is_handled() {
        assert_eq!(normalize_numbers("two point five million"), "2,500,000");
        assert_eq!(normalize_numbers("three point one four"), "3.14");
    }

    #[test]
    fn punctuation_around_a_run_survives() {
        assert_eq!(
            normalize_numbers("the quote was (twenty five thousand),"),
            "the quote was (25,000),"
        );
        assert_eq!(normalize_numbers("Cost: three hundred."), "Cost: 300.");
    }

    #[test]
    fn a_quantity_at_the_start_of_a_sentence_is_still_converted() {
        // Capitalised, but it is the first token, so not a name.
        assert_eq!(
            normalize_numbers("Twenty five thousand was approved"),
            "25,000 was approved"
        );
        assert_eq!(
            normalize_numbers("We agreed. Three hundred seats."),
            "We agreed. 300 seats."
        );
    }

    // ---- what must NOT be converted --------------------------------------

    #[test]
    fn a_product_name_with_numbers_in_it_is_left_alone() {
        // No scale word, so not a quantity.
        assert_eq!(
            normalize_numbers("We renewed Office three sixty five"),
            "We renewed Office three sixty five"
        );
        assert_eq!(normalize_numbers("Windows Eleven rollout"), "Windows Eleven rollout");
    }

    #[test]
    fn a_model_number_after_a_brand_is_left_alone() {
        // "Catalyst nine thousand" has a scale word, so only the name guard saves
        // it. This is the case the guard exists for.
        assert_eq!(
            normalize_numbers("Replacing the Catalyst nine thousand switch"),
            "Replacing the Catalyst nine thousand switch"
        );
        assert_eq!(
            normalize_numbers("a Dell PowerEdge fifteen hundred"),
            "a Dell PowerEdge fifteen hundred"
        );
    }

    #[test]
    fn a_capitalised_run_mid_sentence_is_left_alone() {
        assert_eq!(
            normalize_numbers("the plan is called Twenty Five Thousand"),
            "the plan is called Twenty Five Thousand"
        );
    }

    #[test]
    fn small_standalone_counts_are_left_as_words() {
        // No scale word: converting these is a style opinion, not a fix, and it is
        // where product names hide.
        assert_eq!(normalize_numbers("we have three servers"), "we have three servers");
        assert_eq!(normalize_numbers("all four sites"), "all four sites");
        assert_eq!(normalize_numbers("ten tickets"), "ten tickets");
    }

    #[test]
    fn a_bare_scale_word_is_not_invented_into_a_number() {
        assert_eq!(normalize_numbers("thousands of tickets"), "thousands of tickets");
        assert_eq!(normalize_numbers("a thousand"), "a thousand");
        assert_eq!(normalize_numbers("hundreds more"), "hundreds more");
    }

    #[test]
    fn ordinary_words_that_look_numeric_are_left_alone() {
        assert_eq!(normalize_numbers("point taken"), "point taken");
        assert_eq!(normalize_numbers("and then"), "and then");
        assert_eq!(normalize_numbers("one of one"), "one of one");
    }

    #[test]
    fn existing_digits_are_untouched() {
        assert_eq!(
            normalize_numbers("We saw 25,000 tickets in Q3"),
            "We saw 25,000 tickets in Q3"
        );
    }

    #[test]
    fn a_trailing_connector_is_not_swallowed() {
        assert_eq!(
            normalize_numbers("three hundred and the rest"),
            "300 and the rest"
        );
    }

    #[test]
    fn empty_input_is_handled() {
        assert_eq!(normalize_numbers(""), "");
        assert_eq!(normalize_numbers("   "), "");
    }

    // ---- helpers ---------------------------------------------------------

    #[test]
    fn thousands_separators_group_correctly() {
        assert_eq!(with_thousands_separators(0), "0");
        assert_eq!(with_thousands_separators(999), "999");
        assert_eq!(with_thousands_separators(1_000), "1,000");
        assert_eq!(with_thousands_separators(25_000), "25,000");
        assert_eq!(with_thousands_separators(1_234_567), "1,234,567");
        assert_eq!(with_thousands_separators(-1_500), "-1,500");
    }

    #[test]
    fn name_detection_wants_a_capital_followed_by_lowercase() {
        assert!(looks_like_a_name("Catalyst"));
        assert!(looks_like_a_name("PowerEdge"));
        assert!(!looks_like_a_name("firewall"));
        assert!(!looks_like_a_name("VPN"), "an acronym is not a name marker here");
        assert!(!looks_like_a_name(""));
    }
}
