//! CSV rendering of the network log, for comparing against a firewall log.

use super::store::NetworkEventRow;

/// Quotes every field, RFC 4180 style, matching `consent::export`. Uniform quoting
/// means a URL containing a comma cannot shift the columns of an exported file
/// someone is diffing by eye.
fn csv_field(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

pub const HEADER: &str = "timestamp,host,purpose,method,outcome,bytes_sent,bytes_received,carried_audio,carried_transcript,privacy_profile,meeting_id,url,detail";

pub fn to_csv(rows: &[NetworkEventRow]) -> String {
    let mut out = String::from(HEADER);
    out.push('\n');
    for row in rows {
        let fields = [
            row.created_at.to_rfc3339(),
            row.host.clone(),
            row.purpose.clone(),
            row.method.clone(),
            row.outcome.clone(),
            row.bytes_out.to_string(),
            row.bytes_in.to_string(),
            if row.carried_audio { "yes" } else { "no" }.to_string(),
            if row.carried_transcript { "yes" } else { "no" }.to_string(),
            row.profile_name.clone().unwrap_or_default(),
            row.meeting_id.clone().unwrap_or_default(),
            row.url.clone(),
            row.detail.clone(),
        ];
        out.push_str(
            &fields
                .iter()
                .map(|field| csv_field(field))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn row() -> NetworkEventRow {
        NetworkEventRow {
            id: "net-1".to_string(),
            created_at: Utc.with_ymd_and_hms(2026, 8, 6, 12, 0, 0).unwrap(),
            session_id: "session-1".to_string(),
            host: "api.anthropic.com".to_string(),
            url: "https://api.anthropic.com/v1/messages".to_string(),
            method: "POST".to_string(),
            purpose: "llm_call".to_string(),
            outcome: "ok".to_string(),
            bytes_out: 4_096,
            bytes_in: 2_048,
            meeting_id: Some("meeting-1".to_string()),
            profile_name: Some("Standard".to_string()),
            carried_audio: false,
            carried_transcript: true,
            detail: String::new(),
        }
    }

    #[test]
    fn the_header_lists_every_column_the_rows_write() {
        let csv = to_csv(&[row()]);
        let header_columns = HEADER.split(',').count();
        let row_columns = csv.lines().nth(1).unwrap().matches("\",\"").count() + 1;
        assert_eq!(header_columns, row_columns);
    }

    #[test]
    fn a_row_renders_its_facts_including_what_it_carried() {
        let csv = to_csv(&[row()]);
        let line = csv.lines().nth(1).unwrap();
        assert!(line.contains("\"api.anthropic.com\""));
        assert!(line.contains("\"llm_call\""));
        assert!(line.contains("\"4096\""));
        assert!(line.contains("\"Standard\""));
        // carried_audio no, carried_transcript yes, in that column order.
        assert!(line.contains("\"no\",\"yes\""));
    }

    #[test]
    fn quotes_inside_a_field_are_doubled_rather_than_breaking_the_row() {
        let mut row = row();
        row.detail = "said \"blocked\"".to_string();
        let csv = to_csv(&[row]);
        assert!(csv.contains("\"said \"\"blocked\"\"\""));
        assert_eq!(csv.lines().count(), 2);
    }

    #[test]
    fn an_empty_log_still_exports_a_header_so_the_file_is_never_ambiguous() {
        let csv = to_csv(&[]);
        assert_eq!(csv.trim(), HEADER);
    }

    #[test]
    fn a_missing_profile_or_meeting_becomes_an_empty_field_not_the_word_none() {
        let mut row = row();
        row.profile_name = None;
        row.meeting_id = None;
        let csv = to_csv(&[row]);
        assert!(!csv.contains("None"));
        assert!(csv.contains("\"\",\"\","));
    }
}
