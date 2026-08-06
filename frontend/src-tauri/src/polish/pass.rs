//! The two polish entry points, and the speaker-label handling that keeps a
//! `[Speaker 2]` prefix from ever being treated as content.
//!
//! Split out of `mod.rs` so the scratch compile harness can mount the real file
//! with its real tests instead of a copy that could drift from it.

use super::{datetime, filler, numbers};

/// The polish pass for one transcript segment.
///
/// Order matters: fillers are removed first so a number run interrupted by a
/// filler ("twenty five, um, thousand") can still be recognised as one quantity.
pub fn polish_transcript(text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    let cleaned = filler::strip_fillers(text);
    let numbered = numbers::normalize_numbers(&cleaned);
    datetime::normalize_datetimes(&numbered)
}

/// The polish pass for a whole multi-line block, keeping the line structure that
/// the prompt builders and exports rely on.
///
/// Speaker prefixes like `[Speaker 2]` are preserved: the line is split on the
/// closing bracket and only the spoken part is polished, so a label can never be
/// mistaken for content.
pub fn polish_block(block: &str) -> String {
    block
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                return String::new();
            }
            match speaker_prefix(line) {
                Some((prefix, spoken)) => {
                    let polished = polish_transcript(spoken);
                    if polished.is_empty() {
                        // A line that was nothing but filler keeps its label so
                        // the turn-taking structure of the meeting survives.
                        prefix.trim_end().to_string()
                    } else {
                        // The separator is re-added: `spoken` was trimmed off the
                        // prefix, so concatenating without it would produce
                        // "[Speaker 2]300 tickets".
                        format!("{} {}", prefix.trim_end(), polished)
                    }
                }
                None => polish_transcript(line),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Splits a `[Label] spoken text` line into its label and its content.
fn speaker_prefix(line: &str) -> Option<(&str, &str)> {
    if !line.trim_start().starts_with('[') {
        return None;
    }
    let close = line.find(']')?;
    let (prefix, rest) = line.split_at(close + 1);
    Some((prefix, rest.trim_start()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pass_composes_all_three_stages() {
        let raw = "So um, the renewal came to twenty five thousand dollars, and we meet at three thirty p m";
        assert_eq!(
            polish_transcript(raw),
            "So the renewal came to 25,000 dollars, and we meet at 3:30 PM"
        );
    }

    #[test]
    fn a_filler_inside_a_number_run_does_not_break_the_conversion() {
        // The reason fillers are stripped first.
        assert_eq!(
            polish_transcript("about twenty five um thousand seats"),
            "about 25,000 seats"
        );
    }

    #[test]
    fn a_stutter_before_a_quantity_is_collapsed_and_the_number_still_converts() {
        assert_eq!(
            polish_transcript("the the quote was three hundred"),
            "the quote was 300"
        );
    }

    #[test]
    fn product_names_survive_the_whole_pass() {
        assert_eq!(
            polish_transcript("We renewed Office three sixty five and the Catalyst nine thousand"),
            "We renewed Office three sixty five and the Catalyst nine thousand"
        );
    }

    #[test]
    fn a_real_verb_like_survives_the_whole_pass() {
        assert_eq!(
            polish_transcript("I like the plan, it works like a charm"),
            "I like the plan, it works like a charm"
        );
    }

    #[test]
    fn blank_input_stays_blank() {
        assert_eq!(polish_transcript(""), "");
        assert_eq!(polish_transcript("   "), "");
        assert_eq!(polish_block(""), "");
    }

    #[test]
    fn a_block_keeps_its_lines_and_speaker_labels() {
        let block = "[You] So um, we start at ten a m\n[Speaker 2] I think, you know, that works";
        assert_eq!(
            polish_block(block),
            "[You] So we start at 10 AM\n[Speaker 2] I think that works"
        );
    }

    #[test]
    fn a_speaker_label_is_never_treated_as_content() {
        // "[Speaker 2]" contains a digit and a capitalised word; neither may be
        // rewritten.
        let block = "[Speaker 2] three hundred tickets";
        assert_eq!(polish_block(block), "[Speaker 2] 300 tickets");
    }

    #[test]
    fn a_line_that_was_only_filler_keeps_its_label() {
        let block = "[You] Um, uh\n[Speaker 2] The backups ran";
        assert_eq!(polish_block(block), "[You]\n[Speaker 2] The backups ran");
    }

    #[test]
    fn a_block_without_labels_is_polished_line_by_line() {
        let block = "So um one thing\n\nthe the second thing";
        assert_eq!(polish_block(block), "So one thing\n\nthe second thing");
    }

    #[test]
    fn a_withheld_consent_marker_is_left_intact() {
        // The consent filter runs before polish; its marker must survive.
        let marker = "[withheld: speaker consent not confirmed]";
        assert_eq!(polish_block(marker), marker);
    }

    #[test]
    fn the_pass_is_idempotent() {
        let raw = "So um, twenty five thousand dollars at three thirty p m";
        let once = polish_transcript(raw);
        assert_eq!(polish_transcript(&once), once);
    }
}
