//! Filler removal and stutter collapsing.
//!
//! The most-cited complaint about locally-transcribed notes is not accuracy, it is
//! that the text reads like a recording: "so, um, I think, you know, we should,
//! we should probably check the, the firewall". Whisper is faithfully reproducing
//! speech. A deliverable does not want that faithfulness.
//!
//! The rule that shapes this whole file: **only remove a word when its filler
//! reading is the only reading.** "um" and "uh" always are. "like", "so", and
//! "right" usually are not, so they are only touched in the narrow positions
//! where they cannot be doing real work. Everything ambiguous is left alone,
//! because a summary missing a word the speaker meant is worse than a summary
//! with one "you know" in it.

/// Words that are filler in every context English uses them in.
const UNAMBIGUOUS_FILLERS: &[&str] = &["um", "uh", "erm", "uhh", "umm", "mmm", "hmm", "er", "ah"];

/// Discourse fillers that are only fillers when a comma brackets them.
///
/// "you know" is filler as an interjection ("we should, you know, check it") and
/// content otherwise ("you know the answer"). Same for "like": bracketed it is
/// filler, bare it is a verb, a preposition, or a comparison. The comma is the
/// evidence, and `strip_phrase_fillers` will not act without it.
const PHRASE_FILLERS: &[&str] = &["you know", "i mean", "sort of", "kind of", "like"];

/// Splits a word from its trailing punctuation so "um," can be matched as "um".
fn split_trailing_punctuation(token: &str) -> (&str, &str) {
    let end = token
        .rfind(|c: char| c.is_alphanumeric() || c == '\'' || c == '’')
        .map(|i| i + token[i..].chars().next().map_or(1, char::len_utf8))
        .unwrap_or(0);
    token.split_at(end)
}

fn normalized(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’')
        .to_lowercase()
}

/// True when a token is a filler sound with no other possible reading.
pub fn is_unambiguous_filler(token: &str) -> bool {
    let word = normalized(token);
    UNAMBIGUOUS_FILLERS.contains(&word.as_str())
}

/// Collapses an immediate single-word repetition: "the, the firewall" becomes
/// "the firewall".
///
/// Only **single-word** repeats are collapsed. A multi-word restart
/// ("we should, we should probably") is left alone: collapsing it would also
/// collapse "I know, I know" and "no, no", where the repetition is the meaning.
/// One word is the shape that is unambiguously a stutter.
///
/// Even for single words, deliberate emphasis is a real construction, so the
/// adverbs and answers below are exempt rather than guessed at.
const EMPHASIS_REPEATS: &[&str] = &["very", "really", "so", "no", "yes", "far", "much", "long"];

pub fn is_stutter_repeat(previous: &str, current: &str) -> bool {
    let a = normalized(previous);
    let b = normalized(current);
    if a.is_empty() || a != b {
        return false;
    }
    !EMPHASIS_REPEATS.contains(&a.as_str())
}

/// Merges a stutter pair into one token: the **first** copy's word (so a sentence's
/// capital survives) with the **last** copy's trailing punctuation (so the comma
/// the speaker restarted on does not linger).
fn merge_stutter(first: &str, last: &str) -> String {
    let (word, _) = split_trailing_punctuation(first);
    let (_, punctuation) = split_trailing_punctuation(last);
    format!("{}{}", word, punctuation)
}

/// Word-level pass: drops unambiguous fillers and collapses stutters.
pub fn strip_word_fillers(text: &str) -> String {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut kept: Vec<String> = Vec::with_capacity(tokens.len());

    for token in &tokens {
        if is_unambiguous_filler(token) {
            continue;
        }
        if let Some(previous) = kept.last() {
            if is_stutter_repeat(previous, token) {
                let merged = merge_stutter(previous, token);
                kept.pop();
                kept.push(merged);
                continue;
            }
        }
        kept.push((*token).to_string());
    }

    kept.join(" ")
}

/// Phrase-level pass: removes a comma-bracketed discourse phrase, and the comma
/// that bracketed it.
///
/// The comma requirement is what keeps "you know the answer" and "I like this plan"
/// intact while removing "we should, you know, check it" and "it was, like, huge".
/// A phrase at the very start of the text followed by a comma is also removed,
/// since that is unambiguously an interjection.
pub fn strip_phrase_fillers(text: &str) -> String {
    let mut out = text.to_string();
    for phrase in PHRASE_FILLERS {
        out = remove_bracketed(&out, phrase);
    }
    out
}

fn remove_bracketed(text: &str, phrase: &str) -> String {
    let lowered = text.to_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0usize;

    while let Some(found) = lowered[cursor..].find(phrase) {
        let start = cursor + found;
        let end = start + phrase.len();

        // Must be a whole-word match on both sides.
        let boundary_before = start == 0
            || !lowered[..start]
                .chars()
                .next_back()
                .map(|c| c.is_alphanumeric())
                .unwrap_or(false);
        let boundary_after = end >= lowered.len()
            || !lowered[end..]
                .chars()
                .next()
                .map(|c| c.is_alphanumeric())
                .unwrap_or(false);

        // Bracketing: a comma immediately after, and either a comma before or the
        // very start of the text.
        let after = lowered[end..].trim_start();
        let comma_after = after.starts_with(',');
        let before_trimmed = lowered[..start].trim_end();
        let comma_before = before_trimmed.ends_with(',') || before_trimmed.is_empty();

        if boundary_before && boundary_after && comma_after && comma_before {
            // Copy up to the phrase, dropping a preceding comma if there was one,
            // then skip the phrase and its trailing comma.
            let keep_to = if before_trimmed.is_empty() {
                0
            } else {
                before_trimmed.len() - 1
            };
            result.push_str(&text[cursor..keep_to.max(cursor)]);
            let skip_to = end + (lowered[end..].len() - after.len()) + 1;
            cursor = skip_to.min(text.len());
        } else {
            result.push_str(&text[cursor..end]);
            cursor = end;
        }
    }
    result.push_str(&text[cursor..]);
    result
}

/// Tidies the spacing and punctuation a removal pass leaves behind: doubled
/// spaces, a space before a comma, a leading comma, doubled commas.
pub fn tidy_spacing(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_was_space = false;

    for ch in text.chars() {
        match ch {
            ' ' | '\t' => {
                if !previous_was_space && !out.is_empty() {
                    out.push(' ');
                }
                previous_was_space = true;
            }
            ',' | '.' | '!' | '?' | ';' | ':' => {
                // Drop the space that a removed word left in front of punctuation.
                while out.ends_with(' ') {
                    out.pop();
                }
                // Collapse a run of the same punctuation mark.
                if out.ends_with(ch) {
                    previous_was_space = false;
                    continue;
                }
                // A comma directly after another punctuation mark is debris.
                if ch == ',' && out.ends_with(['.', '!', '?', ';', ':']) {
                    previous_was_space = false;
                    continue;
                }
                out.push(ch);
                previous_was_space = false;
            }
            other => {
                out.push(other);
                previous_was_space = false;
            }
        }
    }

    let trimmed = out.trim();
    let trimmed = trimmed.trim_start_matches(|c| c == ',' || c == ' ');
    trimmed.trim().to_string()
}

/// The whole filler pass.
pub fn strip_fillers(text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    let phrased = strip_phrase_fillers(text);
    let worded = strip_word_fillers(&phrased);
    tidy_spacing(&worded)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- what must be removed -------------------------------------------

    #[test]
    fn um_and_uh_always_go() {
        assert_eq!(strip_fillers("So um I think uh we should check it"), "So I think we should check it");
        assert_eq!(strip_fillers("Um, we should start."), "we should start.");
        assert_eq!(strip_fillers("Erm... hmm, right"), "right");
    }

    #[test]
    fn a_bracketed_you_know_goes() {
        assert_eq!(
            strip_fillers("We should, you know, check the firewall"),
            "We should check the firewall"
        );
    }

    #[test]
    fn a_leading_interjection_goes() {
        assert_eq!(
            strip_fillers("You know, the backups failed"),
            "the backups failed"
        );
        assert_eq!(strip_fillers("I mean, it worked"), "it worked");
    }

    #[test]
    fn a_bracketed_like_goes() {
        assert_eq!(
            strip_fillers("It was, like, completely down"),
            "It was completely down"
        );
    }

    #[test]
    fn an_immediate_stutter_collapses() {
        assert_eq!(
            strip_fillers("Check the the firewall"),
            "Check the firewall"
        );
        assert_eq!(
            strip_fillers("Check the, the firewall"),
            "Check the firewall",
            "the restart comma goes with the repeated word"
        );
    }

    #[test]
    fn a_collapsed_stutter_keeps_the_first_copys_capital() {
        // The first copy carries the sentence's capital; the last carries the
        // punctuation. Keeping the wrong one of each is how a fix like this
        // lowercases the start of every restarted sentence.
        assert_eq!(strip_fillers("We we need a new switch"), "We need a new switch");
        assert_eq!(strip_fillers("The the server"), "The server");
        assert_eq!(strip_fillers("the the server."), "the server.");
    }

    #[test]
    fn a_stutter_match_ignores_case() {
        assert_eq!(strip_fillers("The the server"), "The server");
        assert_eq!(strip_fillers("the The server"), "the server");
    }

    #[test]
    fn a_multi_word_restart_is_deliberately_left_alone() {
        // Conservative by design: the same collapse would flatten "I know, I know"
        // and "no, no", where the repetition carries the meaning.
        assert_eq!(
            strip_fillers("we should, we should probably check it"),
            "we should, we should probably check it"
        );
        assert_eq!(strip_fillers("I know, I know"), "I know, I know");
    }

    // ---- what must NOT be removed ---------------------------------------

    #[test]
    fn like_as_a_verb_survives() {
        assert_eq!(strip_fillers("I like this plan"), "I like this plan");
        assert_eq!(
            strip_fillers("They like the new dashboard"),
            "They like the new dashboard"
        );
    }

    #[test]
    fn like_as_a_preposition_or_comparison_survives() {
        assert_eq!(
            strip_fillers("It behaves like a firewall"),
            "It behaves like a firewall"
        );
        assert_eq!(
            strip_fillers("Something like 25 tickets"),
            "Something like 25 tickets"
        );
        // A comma on one side only is not enough evidence.
        assert_eq!(
            strip_fillers("Tools like, say, Datto"),
            "Tools like, say, Datto"
        );
    }

    #[test]
    fn you_know_as_a_real_clause_survives() {
        assert_eq!(
            strip_fillers("You know the answer already"),
            "You know the answer already"
        );
        assert_eq!(
            strip_fillers("I want to know what you know about it"),
            "I want to know what you know about it"
        );
    }

    #[test]
    fn kind_of_and_sort_of_as_real_phrases_survive() {
        assert_eq!(
            strip_fillers("What kind of licence is it"),
            "What kind of licence is it"
        );
        assert_eq!(
            strip_fillers("It is a sort of gateway"),
            "It is a sort of gateway"
        );
    }

    #[test]
    fn a_word_that_merely_contains_a_filler_survives() {
        // The whole-word boundary check: "umbrella" starts with "um".
        assert_eq!(strip_fillers("Umbrella Corp renewed"), "Umbrella Corp renewed");
        assert_eq!(strip_fillers("The uhlan pattern"), "The uhlan pattern");
        assert_eq!(strip_fillers("Ahmed joined"), "Ahmed joined");
    }

    #[test]
    fn deliberate_emphasis_repetition_survives() {
        assert_eq!(strip_fillers("It was very very slow"), "It was very very slow");
        assert_eq!(strip_fillers("No no, that is wrong"), "No no, that is wrong");
        assert_eq!(strip_fillers("really really important"), "really really important");
    }

    #[test]
    fn a_repeated_word_that_is_not_adjacent_survives() {
        assert_eq!(
            strip_fillers("The server and the switch"),
            "The server and the switch"
        );
    }

    #[test]
    fn a_hyphenated_or_possessive_word_is_not_mangled() {
        assert_eq!(
            strip_fillers("The client's site-to-site tunnel"),
            "The client's site-to-site tunnel"
        );
    }

    // ---- tidying ---------------------------------------------------------

    #[test]
    fn removal_debris_is_tidied_away() {
        assert_eq!(tidy_spacing("We should  , check it"), "We should, check it");
        assert_eq!(tidy_spacing(", leading comma"), "leading comma");
        assert_eq!(tidy_spacing("double,, comma"), "double, comma");
        assert_eq!(tidy_spacing("stop. , then"), "stop. then");
        assert_eq!(tidy_spacing("   spaced   out   "), "spaced out");
    }

    #[test]
    fn an_empty_or_blank_segment_stays_empty() {
        assert_eq!(strip_fillers(""), "");
        assert_eq!(strip_fillers("   "), "");
        // A segment that was nothing but filler collapses to nothing.
        assert_eq!(strip_fillers("um uh erm"), "");
    }

    #[test]
    fn a_realistic_line_reads_cleanly() {
        let raw = "So um, I think, you know, we should probably check the, the firewall, uh, tonight";
        assert_eq!(
            strip_fillers(raw),
            "So I think we should probably check the firewall, tonight"
        );
    }

    #[test]
    fn the_helpers_agree_with_the_pass() {
        assert!(is_unambiguous_filler("Um,"));
        assert!(is_unambiguous_filler("uh"));
        assert!(!is_unambiguous_filler("umbrella"));
        assert!(is_stutter_repeat("the,", "the"));
        assert!(!is_stutter_repeat("very", "very"));
        assert!(!is_stutter_repeat("", ""));
        assert_eq!(merge_stutter("The,", "the"), "The");
        assert_eq!(merge_stutter("the", "the."), "the.");
    }
}
