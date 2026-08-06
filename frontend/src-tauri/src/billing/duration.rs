//! How long was the meeting?
//!
//! There is no `duration` column on `meetings`. What the app actually stores is
//! per-segment timing on `transcripts` (added by
//! `20251006000000_add_audio_sync_fields.sql`):
//!
//! - `audio_start_time` / `audio_end_time` — seconds from the start of the
//!   recording, written by the transcription worker and by the import and
//!   retranscription paths. `MAX(audio_end_time)` is therefore the recorded
//!   length of the meeting, and it is what this module prefers.
//! - `duration` — the length of one segment. Summed, that is *speech* time, not
//!   meeting time, so it is only ever a last resort.
//! - `timestamp` — a wall-clock `HH:MM:SS` string (see
//!   `audio/transcription/worker.rs::format_current_timestamp`). No date, so
//!   spans have to be walked forward with a midnight wrap.
//!
//! Every answer comes back labelled with which of those it came from, because a
//! recorded length and a speech-time under-estimate should not look alike on an
//! invoice.

use sqlx::SqlitePool;

use super::rules::{seconds_to_minutes, MinutesSource};

/// A meeting's derived length, and how it was derived.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DerivedMinutes {
    pub minutes: i64,
    pub source: MinutesSource,
}

impl DerivedMinutes {
    pub fn unknown() -> Self {
        Self {
            minutes: 0,
            source: MinutesSource::Unknown,
        }
    }
}

/// Parses a wall-clock `HH:MM:SS` transcript timestamp into seconds past
/// midnight. Tolerates `H:MM:SS`, `HH:MM`, and surrounding whitespace or
/// brackets, because the string has been written by three different code paths
/// over the app's life.
pub fn parse_clock_seconds(value: &str) -> Option<i64> {
    let cleaned = value.trim().trim_start_matches('[').trim_end_matches(']').trim();
    if cleaned.is_empty() {
        return None;
    }
    // An RFC3339-ish value can also appear; take the time part after 'T'.
    let cleaned = match cleaned.split_once('T') {
        Some((_, time)) => time.trim_end_matches('Z'),
        None => cleaned,
    };
    // Drop any fractional seconds and timezone tail.
    let cleaned = cleaned.split(['.', '+']).next().unwrap_or(cleaned);

    let mut parts = cleaned.split(':');
    let hours: i64 = parts.next()?.trim().parse().ok()?;
    let minutes: i64 = parts.next()?.trim().parse().ok()?;
    let seconds: i64 = match parts.next() {
        Some(s) => s.trim().parse().ok()?,
        None => 0,
    };
    if !(0..24).contains(&hours) || !(0..60).contains(&minutes) || !(0..60).contains(&seconds) {
        return None;
    }
    Some(hours * 3600 + minutes * 60 + seconds)
}

/// Total elapsed seconds across a list of wall-clock timestamps in recording
/// order, walking forward so a meeting that crosses midnight still adds up.
///
/// Unparseable entries are skipped rather than treated as zero, which would
/// otherwise wrap the whole span by nearly a day.
pub fn clock_span_seconds(timestamps: &[String]) -> Option<i64> {
    let parsed: Vec<i64> = timestamps
        .iter()
        .filter_map(|t| parse_clock_seconds(t))
        .collect();
    if parsed.len() < 2 {
        return None;
    }
    let mut total = 0i64;
    for window in parsed.windows(2) {
        let step = (window[1] - window[0]).rem_euclid(86_400);
        total += step;
    }
    (total > 0).then_some(total)
}

/// The raw timing a meeting's transcripts carry, as read from the database.
#[derive(Debug, Clone, Default)]
pub struct TranscriptTiming {
    /// `MAX(audio_end_time) - MIN(audio_start_time)` when both are present.
    pub recorded_seconds: Option<f64>,
    /// `SUM(duration)` across segments: speech time only.
    pub speech_seconds: Option<f64>,
    /// Wall-clock timestamps in recording order.
    pub timestamps: Vec<String>,
}

/// Picks the best available length, in the order the module doc explains.
pub fn derive(timing: &TranscriptTiming) -> DerivedMinutes {
    if let Some(seconds) = timing.recorded_seconds.filter(|s| s.is_finite() && *s > 0.0) {
        return DerivedMinutes {
            minutes: seconds_to_minutes(seconds),
            source: MinutesSource::Recorded,
        };
    }
    if let Some(seconds) = clock_span_seconds(&timing.timestamps) {
        return DerivedMinutes {
            minutes: seconds_to_minutes(seconds as f64),
            source: MinutesSource::TranscriptSpan,
        };
    }
    if let Some(seconds) = timing.speech_seconds.filter(|s| s.is_finite() && *s > 0.0) {
        return DerivedMinutes {
            minutes: seconds_to_minutes(seconds),
            source: MinutesSource::SpeechTime,
        };
    }
    DerivedMinutes::unknown()
}

/// Reads one meeting's transcript timing.
pub async fn timing_for_meeting(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<TranscriptTiming, sqlx::Error> {
    let aggregate: Option<(Option<f64>, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT MIN(audio_start_time), MAX(audio_end_time), SUM(duration)
         FROM transcripts WHERE meeting_id = ?",
    )
    .bind(meeting_id)
    .fetch_optional(pool)
    .await?;

    let (start, end, speech) = aggregate.unwrap_or((None, None, None));
    let recorded_seconds = match (start, end) {
        (Some(start), Some(end)) if end > start => Some(end - start),
        // A single segment starting at 0 gives start == end == 0 for silence;
        // an end with no start still tells us how long the recording ran.
        (None, Some(end)) if end > 0.0 => Some(end),
        _ => None,
    };

    let timestamps: Vec<String> = sqlx::query_as::<_, (String,)>(
        "SELECT timestamp FROM transcripts WHERE meeting_id = ?
         ORDER BY audio_start_time IS NULL, audio_start_time ASC, rowid ASC",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(t,)| t)
    .collect();

    Ok(TranscriptTiming {
        recorded_seconds,
        speech_seconds: speech,
        timestamps,
    })
}

/// The derived length for one meeting. Read failures are logged and reported as
/// "unknown" rather than propagated: a database hiccup should leave a report row
/// saying "no length", not fail the whole report.
pub async fn minutes_for_meeting(pool: &SqlitePool, meeting_id: &str) -> DerivedMinutes {
    match timing_for_meeting(pool, meeting_id).await {
        Ok(timing) => derive(&timing),
        Err(e) => {
            log::warn!(
                "[Billing] could not read transcript timing for {} ({}); reporting no length",
                meeting_id,
                e
            );
            DerivedMinutes::unknown()
        }
    }
}

/// Distinct speaker labels for a meeting, used as the honest local stand-in for
/// an attendee count when no calendar attendee list was captured.
pub async fn speaker_count(pool: &SqlitePool, meeting_id: &str) -> Option<i64> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT speaker) FROM transcripts
         WHERE meeting_id = ? AND speaker IS NOT NULL AND TRIM(speaker) <> ''",
    )
    .bind(meeting_id)
    .fetch_one(pool)
    .await
    .ok()?;
    (count > 0).then_some(count)
}

/// How many named attendees the consent log recorded for this meeting.
///
/// Those subjects come from the calendar event via `consent_prefill_attendees`,
/// so when they exist they are a real attendee list rather than a diarization
/// guess. Both confirmations and declines count: the person was in the room
/// either way, which is what a cost estimate is about.
pub async fn consent_attendee_count(pool: &SqlitePool, meeting_id: &str) -> Option<i64> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT subject) FROM consent_events
         WHERE meeting_id = ?
           AND event_type IN ('attendee_confirmed', 'attendee_declined')
           AND subject IS NOT NULL AND TRIM(subject) <> ''",
    )
    .bind(meeting_id)
    .fetch_one(pool)
    .await
    .ok()?;
    (count > 0).then_some(count)
}

/// Where an attendee count came from, so the cost chip can say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttendeeSource {
    /// Named attendees from the calendar event, via the consent log.
    CalendarAttendees,
    /// Distinct diarized speakers heard in the recording.
    DiarizedSpeakers,
    None,
}

/// The best attendee count available locally: the calendar attendee list first,
/// then diarized speakers, then nothing.
pub async fn attendee_count(pool: &SqlitePool, meeting_id: &str) -> (Option<i64>, AttendeeSource) {
    if let Some(count) = consent_attendee_count(pool, meeting_id).await {
        return (Some(count), AttendeeSource::CalendarAttendees);
    }
    if let Some(count) = speaker_count(pool, meeting_id).await {
        return (Some(count), AttendeeSource::DiarizedSpeakers);
    }
    (None, AttendeeSource::None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamps(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn clock_timestamps_parse_in_the_shapes_the_app_has_written() {
        assert_eq!(parse_clock_seconds("14:30:05"), Some(52_205));
        assert_eq!(parse_clock_seconds("9:05:00"), Some(32_700));
        assert_eq!(parse_clock_seconds(" 00:00:00 "), Some(0));
        assert_eq!(parse_clock_seconds("[10:15]"), Some(36_900));
        assert_eq!(parse_clock_seconds("2026-08-01T10:30:00Z"), Some(37_800));
        assert_eq!(parse_clock_seconds("10:30:00.500"), Some(37_800));
    }

    #[test]
    fn junk_timestamps_are_rejected_rather_than_read_as_zero() {
        assert_eq!(parse_clock_seconds(""), None);
        assert_eq!(parse_clock_seconds("later"), None);
        assert_eq!(parse_clock_seconds("25:00:00"), None);
        assert_eq!(parse_clock_seconds("10:70:00"), None);
        assert_eq!(parse_clock_seconds("10"), None);
    }

    #[test]
    fn a_span_is_the_forward_walk_between_timestamps() {
        assert_eq!(
            clock_span_seconds(&stamps(&["10:00:00", "10:15:00", "10:50:00"])),
            Some(3_000)
        );
    }

    #[test]
    fn a_span_across_midnight_does_not_go_negative() {
        assert_eq!(
            clock_span_seconds(&stamps(&["23:50:00", "00:05:00"])),
            Some(900)
        );
    }

    #[test]
    fn a_span_needs_two_readable_timestamps() {
        assert_eq!(clock_span_seconds(&[]), None);
        assert_eq!(clock_span_seconds(&stamps(&["10:00:00"])), None);
        assert_eq!(clock_span_seconds(&stamps(&["10:00:00", "junk"])), None);
        // Every reading identical: a real zero, reported as no span.
        assert_eq!(clock_span_seconds(&stamps(&["10:00:00", "10:00:00"])), None);
    }

    #[test]
    fn unparseable_entries_are_skipped_not_treated_as_midnight() {
        // Without the filter, "junk" as 0 would add ~24h of billable time.
        assert_eq!(
            clock_span_seconds(&stamps(&["10:00:00", "junk", "10:10:00"])),
            Some(600)
        );
    }

    #[test]
    fn the_recorded_length_wins_when_it_exists() {
        let derived = derive(&TranscriptTiming {
            recorded_seconds: Some(3_000.0),
            speech_seconds: Some(600.0),
            timestamps: stamps(&["10:00:00", "10:05:00"]),
        });
        assert_eq!(derived.source, MinutesSource::Recorded);
        assert_eq!(derived.minutes, 50);
    }

    #[test]
    fn the_clock_span_is_the_first_fallback() {
        let derived = derive(&TranscriptTiming {
            recorded_seconds: None,
            speech_seconds: Some(600.0),
            timestamps: stamps(&["10:00:00", "10:40:00"]),
        });
        assert_eq!(derived.source, MinutesSource::TranscriptSpan);
        assert_eq!(derived.minutes, 40);
    }

    #[test]
    fn speech_time_is_the_last_resort_and_says_so() {
        let derived = derive(&TranscriptTiming {
            recorded_seconds: None,
            speech_seconds: Some(605.0),
            timestamps: stamps(&["10:00:00"]),
        });
        assert_eq!(derived.source, MinutesSource::SpeechTime);
        assert_eq!(derived.minutes, 11, "605s rounds up to 11 minutes");
    }

    #[test]
    fn nothing_at_all_is_unknown_not_zero_minutes_billable() {
        let derived = derive(&TranscriptTiming::default());
        assert_eq!(derived.source, MinutesSource::Unknown);
        assert_eq!(derived.minutes, 0);
    }

    #[test]
    fn a_nonsense_recorded_length_falls_through_to_the_next_source() {
        let derived = derive(&TranscriptTiming {
            recorded_seconds: Some(f64::NAN),
            speech_seconds: None,
            timestamps: stamps(&["10:00:00", "10:20:00"]),
        });
        assert_eq!(derived.source, MinutesSource::TranscriptSpan);
        assert_eq!(derived.minutes, 20);
    }
}
