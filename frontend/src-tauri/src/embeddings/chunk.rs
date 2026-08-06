//! Turning a meeting's text into the passages that actually get embedded.
//!
//! Pure functions over slices, so the chunking rules are testable without a
//! database. Two shapes are needed:
//!
//! * A transcript arrives as many short rows (one per speech burst). One row is
//!   usually too small to mean anything on its own, so consecutive rows are
//!   glued into passages of roughly `target_chars`, with a one-row overlap so a
//!   sentence that straddles a boundary is still findable from either side.
//! * A summary arrives as one long markdown document, so it is split at
//!   paragraph boundaries instead.

/// One row of source text with the id it should be attributed to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSegment {
    pub id: String,
    pub speaker: Option<String>,
    pub text: String,
}

/// A passage ready to embed. `source_id` is the id of the first row in the
/// passage, which is what a search result links back to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub source_id: String,
    pub text: String,
}

/// Roughly how many characters a transcript passage should hold. Chosen to sit
/// comfortably inside the 256-token window of the embedding model (English
/// averages a little under 4 characters per token, so ~800 characters is ~200
/// tokens) without wasting most of the window on padding.
pub const TRANSCRIPT_TARGET_CHARS: usize = 800;

/// Roughly how many characters a summary passage should hold. Larger than a
/// transcript passage: summary prose is denser and its paragraphs are the natural
/// unit, so splitting them finer would separate a heading from its content.
pub const SUMMARY_TARGET_CHARS: usize = 1_200;

/// Passages shorter than this are dropped. A three-word row ("Yeah.", "Right.")
/// embeds to noise and only makes the ranking worse.
pub const MIN_CHUNK_CHARS: usize = 24;

fn render(segment: &SourceSegment) -> String {
    match segment.speaker.as_deref().map(str::trim) {
        Some(label) if !label.is_empty() => format!("[{}] {}", label, segment.text.trim()),
        _ => segment.text.trim().to_string(),
    }
}

/// Glues consecutive transcript rows into passages of roughly `target_chars`,
/// repeating the last row of each passage at the head of the next so a thought
/// split across a boundary stays retrievable.
pub fn chunk_segments(segments: &[SourceSegment], target_chars: usize) -> Vec<Chunk> {
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut pending: Vec<&SourceSegment> = Vec::new();
    let mut pending_chars = 0usize;

    let flush = |pending: &mut Vec<&SourceSegment>, chunks: &mut Vec<Chunk>| {
        if pending.is_empty() {
            return;
        }
        let text = pending
            .iter()
            .map(|segment| render(segment))
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if text.chars().count() >= MIN_CHUNK_CHARS {
            chunks.push(Chunk {
                source_id: pending[0].id.clone(),
                text,
            });
        }
    };

    for segment in segments {
        if segment.text.trim().is_empty() {
            continue;
        }
        let size = segment.text.chars().count();
        pending.push(segment);
        pending_chars += size;
        if pending_chars < target_chars {
            continue;
        }
        flush(&mut pending, &mut chunks);
        // Carry the final row forward as the overlap. A single row is enough:
        // rows are speech bursts, so one row of context is one full utterance.
        let carry = pending.pop();
        pending.clear();
        if let Some(last) = carry {
            pending.push(last);
            pending_chars = last.text.chars().count();
        } else {
            pending_chars = 0;
        }
    }

    // The trailing partial passage. Skipped when it is only the overlap row that
    // was already embedded as part of the previous passage.
    if pending.len() > 1 || (pending.len() == 1 && chunks.is_empty()) {
        flush(&mut pending, &mut chunks);
    }
    chunks
}

/// Splits one long document at paragraph boundaries into passages of roughly
/// `target_chars`. A single paragraph longer than the target is kept whole rather
/// than cut mid-sentence; the model truncates it, which loses the tail but never
/// invents a false boundary.
pub fn split_document(text: &str, target_chars: usize) -> Vec<String> {
    let mut passages: Vec<String> = Vec::new();
    let mut current = String::new();

    for paragraph in text.split("\n\n") {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            continue;
        }
        if !current.is_empty() && current.chars().count() + paragraph.chars().count() > target_chars
        {
            passages.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(paragraph);
    }
    if !current.is_empty() {
        passages.push(current);
    }
    passages
        .into_iter()
        .filter(|passage| passage.chars().count() >= MIN_CHUNK_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(id: &str, speaker: Option<&str>, text: &str) -> SourceSegment {
        SourceSegment {
            id: id.to_string(),
            speaker: speaker.map(str::to_string),
            text: text.to_string(),
        }
    }

    #[test]
    fn short_rows_are_glued_into_one_passage_with_speaker_labels() {
        let segments = vec![
            segment("t1", Some("You"), "We need the migration done by Friday."),
            segment("t2", Some("Speaker 2"), "Friday is tight but workable."),
        ];
        let chunks = chunk_segments(&segments, 800);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].source_id, "t1");
        assert_eq!(
            chunks[0].text,
            "[You] We need the migration done by Friday.\n[Speaker 2] Friday is tight but workable."
        );
    }

    #[test]
    fn passages_break_at_the_target_and_overlap_by_one_row() {
        let segments = vec![
            segment("t1", None, &"a".repeat(60)),
            segment("t2", None, &"b".repeat(60)),
            segment("t3", None, &"c".repeat(60)),
        ];
        let chunks = chunk_segments(&segments, 100);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].source_id, "t1");
        // t2 closes the first passage and opens the second: that is the overlap.
        assert!(chunks[0].text.contains(&"b".repeat(60)));
        assert_eq!(chunks[1].source_id, "t2");
        assert!(chunks[1].text.contains(&"c".repeat(60)));
    }

    #[test]
    fn a_trailing_overlap_only_remainder_is_not_emitted_twice() {
        let segments = vec![
            segment("t1", None, &"a".repeat(60)),
            segment("t2", None, &"b".repeat(60)),
        ];
        let chunks = chunk_segments(&segments, 100);
        // t2 already appears inside the single emitted passage; it must not come
        // back as a second, duplicate passage of its own.
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn rows_too_short_to_mean_anything_are_dropped() {
        let segments = vec![segment("t1", Some("You"), "Yeah.")];
        assert!(chunk_segments(&segments, 800).is_empty());
    }

    #[test]
    fn blank_rows_are_skipped_without_shifting_attribution() {
        let segments = vec![
            segment("t1", None, "   "),
            segment("t2", None, "The renewal quote lands next Tuesday."),
        ];
        let chunks = chunk_segments(&segments, 800);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].source_id, "t2");
    }

    #[test]
    fn an_empty_transcript_yields_no_passages() {
        assert!(chunk_segments(&[], 800).is_empty());
    }

    #[test]
    fn documents_split_on_paragraph_boundaries() {
        let text = format!("{}\n\n{}", "x".repeat(80), "y".repeat(80));
        let passages = split_document(&text, 100);
        assert_eq!(passages.len(), 2);
        assert_eq!(passages[0], "x".repeat(80));
    }

    #[test]
    fn paragraphs_that_fit_together_stay_together() {
        let text = "First point about the renewal.\n\nSecond point about the invoice.";
        let passages = split_document(text, 1_200);
        assert_eq!(passages.len(), 1);
        assert!(passages[0].contains("First point"));
        assert!(passages[0].contains("Second point"));
    }

    #[test]
    fn an_oversized_paragraph_is_kept_whole_rather_than_cut_mid_sentence() {
        let text = "z".repeat(500);
        let passages = split_document(&text, 100);
        assert_eq!(passages, vec![text]);
    }
}
