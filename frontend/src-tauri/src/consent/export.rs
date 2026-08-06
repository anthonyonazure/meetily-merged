//! Renders the consent log as CSV and Markdown.
//!
//! CSV is the machine-readable copy (spreadsheets, archival); Markdown is the
//! readable copy. Both are produced from the same rows in one pass so they can
//! never disagree about what the log said.

use crate::database::models::ConsentEvent;

/// One CSV field, RFC 4180 quoted. Every field is quoted rather than only the
/// ones that need it: the log carries operator-typed text, and "quote only when
/// necessary" is exactly the rule that gets a comma or a newline wrong.
fn csv_field(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn csv_row(fields: &[&str]) -> String {
    fields
        .iter()
        .map(|f| csv_field(f))
        .collect::<Vec<_>>()
        .join(",")
}

/// Human-readable label for an event type. Describes the mechanism only.
pub fn event_label(event_type: &str) -> &str {
    match event_type {
        "self" => "Operator self-consent",
        "notice_given" => "Notice given",
        "attendee_confirmed" => "Attendee confirmed",
        "attendee_declined" => "Attendee declined",
        "speaker_confirmed" => "Speaker confirmed",
        "speaker_declined" => "Speaker declined",
        "recording_blocked" => "Recording blocked",
        "level_overridden" => "Level overridden",
        other => other,
    }
}

/// Human-readable label for how notice was given.
pub fn method_label(method: &str) -> &str {
    match method {
        "chat_paste" => "Pasted in meeting chat",
        "spoken_announcement" => "Spoken announcement",
        "verbal" => "Said out loud",
        "in_person" => "In person",
        "other" => "Other",
        other => other,
    }
}

fn title_for(titles: &[(String, String)], meeting_id: &str) -> String {
    titles
        .iter()
        .find(|(id, _)| id == meeting_id)
        .map(|(_, title)| title.clone())
        .unwrap_or_else(|| "(recording not saved)".to_string())
}

/// CSV with a header row. `titles` maps meeting ids and consent session ids to
/// meeting titles.
pub fn to_csv(events: &[ConsentEvent], titles: &[(String, String)]) -> String {
    let mut out = String::new();
    out.push_str(&csv_row(&[
        "timestamp_utc",
        "meeting_title",
        "meeting_id",
        "consent_level",
        "event",
        "subject",
        "method",
        "detail",
    ]));
    out.push('\n');

    for event in events {
        let title = title_for(titles, &event.meeting_id);
        let method = event.method.as_deref().map(method_label).unwrap_or("");
        out.push_str(&csv_row(&[
            &event.created_at.to_rfc3339(),
            &title,
            &event.meeting_id,
            &event.level,
            event_label(&event.event_type),
            event.subject.as_deref().unwrap_or(""),
            method,
            &event.detail,
        ]));
        out.push('\n');
    }
    out
}

/// Markdown, grouped by meeting, newest meeting first.
pub fn to_markdown(
    events: &[ConsentEvent],
    titles: &[(String, String)],
    from_label: &str,
    to_label: &str,
) -> String {
    let mut out = String::new();
    out.push_str("# Recording consent log\n\n");
    out.push_str(&format!("Range: {} to {}\n\n", from_label, to_label));
    out.push_str(&format!("Events: {}\n\n", events.len()));

    if events.is_empty() {
        out.push_str("_No consent events in this range._\n");
        return out;
    }

    // Preserve the incoming order of first appearance for each meeting id.
    let mut groups: Vec<(String, Vec<&ConsentEvent>)> = Vec::new();
    for event in events {
        match groups.iter_mut().find(|(id, _)| *id == event.meeting_id) {
            Some((_, bucket)) => bucket.push(event),
            None => groups.push((event.meeting_id.clone(), vec![event])),
        }
    }

    for (meeting_id, bucket) in groups {
        out.push_str(&format!("## {}\n\n", title_for(titles, &meeting_id)));
        out.push_str(&format!("`{}`\n\n", meeting_id));
        out.push_str("| Time (UTC) | Level | Event | Subject | Method | Detail |\n");
        out.push_str("| --- | --- | --- | --- | --- | --- |\n");
        for event in bucket {
            let method = event.method.as_deref().map(method_label).unwrap_or("");
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                event.created_at.to_rfc3339(),
                escape_cell(&event.level),
                escape_cell(event_label(&event.event_type)),
                escape_cell(event.subject.as_deref().unwrap_or("")),
                escape_cell(method),
                escape_cell(&event.detail),
            ));
        }
        out.push('\n');
    }
    out
}

/// Keeps operator-typed pipes and newlines from breaking the table.
fn escape_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn event(event_type: &str, subject: Option<&str>, detail: &str) -> ConsentEvent {
        ConsentEvent {
            id: "consent-1".to_string(),
            meeting_id: "meeting-1".to_string(),
            level: "notify".to_string(),
            event_type: event_type.to_string(),
            subject: subject.map(str::to_string),
            method: Some("chat_paste".to_string()),
            detail: detail.to_string(),
            created_at: Utc.with_ymd_and_hms(2026, 8, 6, 12, 0, 0).unwrap(),
        }
    }

    fn titles() -> Vec<(String, String)> {
        vec![("meeting-1".to_string(), "Weekly Sync".to_string())]
    }

    #[test]
    fn csv_quotes_every_field_and_doubles_inner_quotes() {
        let events = vec![event(
            "notice_given",
            Some("dana@example.com"),
            "said \"go ahead\", then, paused",
        )];
        let csv = to_csv(&events, &titles());
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("\"timestamp_utc\",\"meeting_title\""));
        assert!(lines[1].contains("\"said \"\"go ahead\"\", then, paused\""));
        assert!(lines[1].contains("\"Weekly Sync\""));
        assert!(lines[1].contains("\"Pasted in meeting chat\""));
    }

    #[test]
    fn csv_of_an_empty_log_is_just_the_header() {
        let csv = to_csv(&[], &titles());
        assert_eq!(csv.lines().count(), 1);
    }

    #[test]
    fn unsaved_recordings_are_named_rather_than_shown_as_bare_ids() {
        let mut ev = event("self", None, "");
        ev.meeting_id = "consent-session-abc".to_string();
        let csv = to_csv(&[ev], &titles());
        assert!(csv.contains("(recording not saved)"));
    }

    #[test]
    fn markdown_groups_by_meeting_and_escapes_pipes() {
        let events = vec![
            event("self", None, "a | b"),
            event("speaker_declined", Some("Speaker 2"), "line one\nline two"),
        ];
        let md = to_markdown(&events, &titles(), "2026-08-01", "2026-08-31");
        assert!(md.contains("# Recording consent log"));
        assert!(md.contains("## Weekly Sync"));
        assert!(md.contains("a \\| b"));
        assert!(md.contains("line one line two"));
        assert!(md.contains("Speaker declined"));
        // One heading only: both events belong to the same meeting.
        assert_eq!(md.matches("## ").count(), 1);
    }

    #[test]
    fn empty_markdown_says_so_instead_of_rendering_an_empty_table() {
        let md = to_markdown(&[], &titles(), "2026-08-01", "2026-08-31");
        assert!(md.contains("_No consent events in this range._"));
        assert!(!md.contains("| Time (UTC) |"));
    }
}
