//! Calendar integration v1 (local-first).
//!
//! Reads the OS calendar so upcoming meetings can be listed in-app, meeting
//! names prefilled, and meeting links joined with one click. macOS only for
//! now (EventKit via objc2-event-kit); other platforms get a clear
//! "not supported" error. No OAuth and no network: whatever calendars the OS
//! already syncs (iCloud, Google, Exchange) are read locally through EventKit.
//!
//! Requires NSCalendars(FullAccess)UsageDescription in Info.plist and the
//! com.apple.security.personal-information.calendars entitlement.

#[cfg(target_os = "macos")]
mod macos;

use serde::Serialize;

#[cfg(not(target_os = "macos"))]
const NOT_SUPPORTED: &str = "Calendar integration is not supported on this platform yet";

/// One upcoming calendar event, ready for the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct CalendarEvent {
    pub id: String,
    pub title: String,
    /// RFC 3339 timestamps in UTC.
    pub start: String,
    pub end: String,
    pub organizer: Option<String>,
    /// First Zoom / Teams / Google Meet / Webex link found in the event's
    /// URL, location, or notes.
    pub meeting_url: Option<String>,
}

/// Video-conference domains whose links count as "meeting URLs".
const MEETING_DOMAINS: &[&str] = &[
    "zoom.us",
    "teams.microsoft.com",
    "teams.live.com",
    "meet.google.com",
    "webex.com",
];

fn host_is_meeting_domain(host: &str) -> bool {
    let host = host.to_lowercase();
    MEETING_DOMAINS
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{}", domain)))
}

/// Extracts the first meeting link from free text (event URL, location, or
/// notes). Candidate URLs are parsed with the `url` crate so the domain check
/// runs against the real host, not a substring anywhere in the string.
pub(crate) fn extract_meeting_url(texts: &[Option<&str>]) -> Option<String> {
    // Lazily compiled once; the pattern grabs http(s) URLs up to whitespace
    // or common delimiters.
    static URL_PATTERN: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
        regex::Regex::new(r#"https?://[^\s<>"'\)\]\}]+"#).expect("valid URL regex")
    });

    for text in texts.iter().flatten() {
        for candidate in URL_PATTERN.find_iter(text) {
            let cleaned = candidate.as_str().trim_end_matches(['.', ',', ';', ':']);
            if let Ok(parsed) = url::Url::parse(cleaned) {
                if let Some(host) = parsed.host_str() {
                    if host_is_meeting_domain(host) {
                        return Some(parsed.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Converts a Unix timestamp (seconds) to an RFC 3339 UTC string.
#[cfg(target_os = "macos")]
fn unix_to_rfc3339(seconds: f64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(seconds as i64, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

/// Returns the calendar permission state: "not_determined", "denied",
/// "restricted", "write_only", or "full_access". Errors on unsupported
/// platforms.
#[tauri::command]
pub async fn calendar_permission_status() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(macos::permission_status().to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(NOT_SUPPORTED.to_string())
    }
}

/// Prompts for calendar access (no-op if the user already decided). Returns
/// whether access is granted.
#[tauri::command]
pub async fn calendar_request_access() -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        tokio::task::spawn_blocking(macos::request_access_blocking)
            .await
            .map_err(|e| format!("Calendar permission task failed: {}", e))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(NOT_SUPPORTED.to_string())
    }
}

/// Lists events starting in the next 24 hours (including ones already in
/// progress), sorted by start time.
#[tauri::command]
pub async fn calendar_upcoming_events() -> Result<Vec<CalendarEvent>, String> {
    #[cfg(target_os = "macos")]
    {
        let raw_events = tokio::task::spawn_blocking(|| macos::upcoming_events_blocking(24.0))
            .await
            .map_err(|e| format!("Calendar query task failed: {}", e))??;

        let mut events: Vec<CalendarEvent> = raw_events
            .into_iter()
            .map(|raw| {
                let meeting_url = extract_meeting_url(&[
                    raw.url.as_deref(),
                    raw.location.as_deref(),
                    raw.notes.as_deref(),
                ]);
                CalendarEvent {
                    id: raw.id,
                    title: raw.title,
                    start: unix_to_rfc3339(raw.start_unix),
                    end: unix_to_rfc3339(raw.end_unix),
                    organizer: raw.organizer,
                    meeting_url,
                }
            })
            .collect();
        events.sort_by(|a, b| a.start.cmp(&b.start));
        Ok(events)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(NOT_SUPPORTED.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_teams_meet_webex_links_are_detected() {
        assert_eq!(
            extract_meeting_url(&[Some("Join: https://us02web.zoom.us/j/123?pwd=x")]).as_deref(),
            Some("https://us02web.zoom.us/j/123?pwd=x")
        );
        assert!(extract_meeting_url(&[Some(
            "https://teams.microsoft.com/l/meetup-join/abc"
        )])
        .is_some());
        assert!(extract_meeting_url(&[Some("https://meet.google.com/abc-defg-hij")]).is_some());
        assert!(extract_meeting_url(&[Some("https://company.webex.com/meet/room")]).is_some());
    }

    #[test]
    fn non_meeting_urls_and_lookalike_hosts_are_rejected() {
        assert!(extract_meeting_url(&[Some("https://example.com/zoom.us/fake")]).is_none());
        assert!(extract_meeting_url(&[Some("https://notzoom.usa.com/j/1")]).is_none());
        assert!(extract_meeting_url(&[Some("no links here")]).is_none());
        assert!(extract_meeting_url(&[None, None]).is_none());
    }

    #[test]
    fn first_source_with_a_link_wins_and_trailing_punctuation_is_trimmed() {
        let url = extract_meeting_url(&[
            Some("nothing"),
            Some("In notes: https://zoom.us/j/999."),
        ]);
        assert_eq!(url.as_deref(), Some("https://zoom.us/j/999"));
    }
}
