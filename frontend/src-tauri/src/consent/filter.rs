//! Strict-mode filtering for `per_speaker` consent.
//!
//! In `flag_only` the transcript is untouched and unconsented speakers are just
//! marked in the UI. In `strict` the text of an unconsented speaker's segments
//! is withheld everywhere the transcript is *consumed* — summaries, agents,
//! chat context, exports — while the stored transcript itself is left alone.
//! Redacting at the consumption boundary rather than at write time means a
//! later confirmation restores the text without needing a re-transcription.

use crate::database::repositories::consent::ConsentEventsRepository;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::rules::{ConsentLevel, EnforcementMode};
use super::settings;

/// What replaces a withheld segment. Describes the mechanism, nothing more.
pub const REDACTION_MARKER: &str = "[withheld: speaker consent not confirmed]";

/// The speaker label the app gives the operator's own microphone. The operator
/// consents for themselves at every level, so this label is never withheld.
pub const OPERATOR_SPEAKER: &str = "You";

/// Whether and what to withhold for one meeting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionState {
    /// The level in force for this meeting.
    pub level: ConsentLevel,
    pub enforcement: EnforcementMode,
    /// True when text must actually be withheld (per_speaker + strict).
    pub strict: bool,
    /// Speaker labels present in this meeting with no `speaker_confirmed` on
    /// record. Populated regardless of `strict` so `flag_only` can mark them.
    pub unconsented_speakers: Vec<String>,
}

impl RedactionState {
    /// A state that changes nothing, for meetings outside `per_speaker`.
    fn inert(level: ConsentLevel, enforcement: EnforcementMode) -> Self {
        Self {
            level,
            enforcement,
            strict: false,
            unconsented_speakers: Vec::new(),
        }
    }

    pub fn withholds(&self, speaker: Option<&str>) -> bool {
        if !self.strict {
            return false;
        }
        match speaker {
            None => false, // Unlabelled segments predate diarization; keep them.
            Some(label) => self
                .unconsented_speakers
                .iter()
                .any(|s| s.eq_ignore_ascii_case(label.trim())),
        }
    }
}

/// The latest decision recorded for each speaker label, in log order.
/// `true` = confirmed, `false` = declined. Later rows win, which is how a
/// correction (a new row, never an update) takes effect.
pub fn latest_speaker_decisions(events: &[(String, String)]) -> Vec<(String, bool)> {
    let mut decisions: Vec<(String, bool)> = Vec::new();
    for (event_type, subject) in events {
        let confirmed = match event_type.as_str() {
            "speaker_confirmed" => true,
            "speaker_declined" => false,
            _ => continue,
        };
        let subject = subject.trim();
        if subject.is_empty() {
            continue;
        }
        match decisions
            .iter_mut()
            .find(|(label, _)| label.eq_ignore_ascii_case(subject))
        {
            Some(entry) => entry.1 = confirmed,
            None => decisions.push((subject.to_string(), confirmed)),
        }
    }
    decisions
}

/// Speaker labels present in a meeting that have no standing confirmation.
/// The operator's own label is always treated as consented.
pub fn unconsented_speakers(
    present_labels: &[String],
    decisions: &[(String, bool)],
) -> Vec<String> {
    present_labels
        .iter()
        .filter(|label| {
            let label = label.trim();
            if label.is_empty() || label.eq_ignore_ascii_case(OPERATOR_SPEAKER) {
                return false;
            }
            !decisions
                .iter()
                .any(|(decided, confirmed)| *confirmed && decided.eq_ignore_ascii_case(label))
        })
        .map(|label| label.trim().to_string())
        .collect()
}

/// Replaces the text of withheld segments in place. Segments are
/// `(speaker, text)` pairs; the speaker may be absent for pre-diarization rows.
pub fn redact_segments(
    state: &RedactionState,
    segments: &mut [(Option<String>, String)],
) -> usize {
    if !state.strict {
        return 0;
    }
    let mut withheld = 0usize;
    for (speaker, text) in segments.iter_mut() {
        if state.withholds(speaker.as_deref()) {
            *text = REDACTION_MARKER.to_string();
            withheld += 1;
        }
    }
    withheld
}

/// Renders the `[Speaker] text` block that the chat and agent prompts are built
/// from, with strict-mode withholding already applied.
///
/// Both prompt builders used to fold the rows themselves; routing them through
/// here is what guarantees the filter cannot be added to one consumer and
/// forgotten in the other.
pub async fn speaker_prefixed_block(
    pool: &SqlitePool,
    meeting_id: &str,
    rows: &[(Option<String>, String)],
) -> String {
    let state = state_for_meeting(pool, meeting_id).await;
    let mut segments = rows.to_vec();
    let withheld = redact_segments(&state, &mut segments);
    if withheld > 0 {
        log::info!(
            "[Consent] withheld {} segment(s) from meeting {} (strict per-speaker consent)",
            withheld,
            meeting_id
        );
    }

    segments
        .into_iter()
        .map(|(speaker, text)| match speaker {
            Some(label) if !label.trim().is_empty() => format!("[{}] {}", label.trim(), text),
            _ => text,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Resolves the redaction state for a meeting from the database.
///
/// Reads the level in force, then the speaker labels actually present in the
/// meeting's transcripts, then the standing decisions from the consent log.
pub async fn state_for_meeting(pool: &SqlitePool, meeting_id: &str) -> RedactionState {
    let settings = settings::load(pool).await;
    let level = settings.consent_level;
    let enforcement = settings.per_speaker_enforcement;

    if level != ConsentLevel::PerSpeaker {
        return RedactionState::inert(level, enforcement);
    }

    let present = match speaker_labels_for_meeting(pool, meeting_id).await {
        Ok(labels) => labels,
        Err(e) => {
            log::warn!(
                "[Consent] could not read speaker labels for {} ({}); withholding nothing",
                meeting_id,
                e
            );
            return RedactionState::inert(level, enforcement);
        }
    };

    let events = match ConsentEventsRepository::for_meeting(pool, meeting_id).await {
        Ok(events) => events
            .into_iter()
            .map(|e| (e.event_type, e.subject.unwrap_or_default()))
            .collect::<Vec<_>>(),
        Err(e) => {
            log::warn!(
                "[Consent] could not read consent log for {} ({}); treating speakers as unconfirmed",
                meeting_id,
                e
            );
            Vec::new()
        }
    };

    let decisions = latest_speaker_decisions(&events);
    let unconsented = unconsented_speakers(&present, &decisions);

    RedactionState {
        level,
        enforcement,
        strict: enforcement == EnforcementMode::Strict && !unconsented.is_empty(),
        unconsented_speakers: unconsented,
    }
}

/// Distinct, non-empty speaker labels used by a meeting's transcripts.
pub async fn speaker_labels_for_meeting(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<Vec<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT speaker FROM transcripts
         WHERE meeting_id = ? AND speaker IS NOT NULL AND TRIM(speaker) <> ''
         ORDER BY speaker ASC",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(label,)| label).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(t, s)| (t.to_string(), s.to_string()))
            .collect()
    }

    fn labels(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    fn strict_state(unconsented: &[&str]) -> RedactionState {
        RedactionState {
            level: ConsentLevel::PerSpeaker,
            enforcement: EnforcementMode::Strict,
            strict: true,
            unconsented_speakers: labels(unconsented),
        }
    }

    #[test]
    fn the_latest_row_for_a_speaker_wins() {
        let log = events(&[
            ("speaker_confirmed", "Speaker 2"),
            ("notice_given", ""),
            ("speaker_declined", "Speaker 2"),
            ("speaker_confirmed", "Speaker 3"),
        ]);
        let decisions = latest_speaker_decisions(&log);
        assert_eq!(
            decisions,
            vec![("Speaker 2".to_string(), false), ("Speaker 3".to_string(), true)]
        );
    }

    #[test]
    fn declined_and_undecided_speakers_are_both_unconsented() {
        let decisions = latest_speaker_decisions(&events(&[
            ("speaker_confirmed", "Speaker 1"),
            ("speaker_declined", "Speaker 2"),
        ]));
        let present = labels(&["You", "Speaker 1", "Speaker 2", "Speaker 3"]);
        // "You" is the operator and is never withheld.
        assert_eq!(
            unconsented_speakers(&present, &decisions),
            labels(&["Speaker 2", "Speaker 3"])
        );
    }

    #[test]
    fn strict_mode_replaces_only_unconsented_text() {
        let state = strict_state(&["Speaker 2"]);
        let mut segments = vec![
            (Some("You".to_string()), "my opening".to_string()),
            (Some("Speaker 2".to_string()), "their answer".to_string()),
            (Some("Speaker 3".to_string()), "another voice".to_string()),
            (None, "unlabelled line".to_string()),
        ];
        let withheld = redact_segments(&state, &mut segments);
        assert_eq!(withheld, 1);
        assert_eq!(segments[0].1, "my opening");
        assert_eq!(segments[1].1, REDACTION_MARKER);
        assert_eq!(segments[2].1, "another voice");
        assert_eq!(segments[3].1, "unlabelled line");
    }

    #[test]
    fn speaker_matching_ignores_case_and_padding() {
        let state = strict_state(&["Speaker 2"]);
        assert!(state.withholds(Some(" speaker 2 ")));
        assert!(!state.withholds(Some("Speaker 20")));
        assert!(!state.withholds(None));
    }

    #[test]
    fn flag_only_never_withholds_anything() {
        let state = RedactionState {
            level: ConsentLevel::PerSpeaker,
            enforcement: EnforcementMode::FlagOnly,
            strict: false,
            unconsented_speakers: labels(&["Speaker 2"]),
        };
        let mut segments = vec![(Some("Speaker 2".to_string()), "kept".to_string())];
        assert_eq!(redact_segments(&state, &mut segments), 0);
        assert_eq!(segments[0].1, "kept");
        assert!(!state.withholds(Some("Speaker 2")));
    }

    #[test]
    fn inert_states_do_nothing_outside_per_speaker() {
        let state = RedactionState::inert(ConsentLevel::Notify, EnforcementMode::Strict);
        assert!(!state.strict);
        assert!(state.unconsented_speakers.is_empty());
        assert!(!state.withholds(Some("Speaker 2")));
    }
}
