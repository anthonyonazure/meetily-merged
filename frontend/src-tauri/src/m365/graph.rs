//! Minimal Microsoft Graph client: /me, next-24h calendarView, and draft
//! message creation. All requests run in Rust (the webview has no network
//! access by CSP design).

use std::time::Duration;

use serde_json::{json, Value};

use crate::calendar::{extract_meeting_url, CalendarEvent};

const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

/// Sends a Graph request and records it in the network log.
///
/// Every Graph call goes through here rather than each call site remembering to
/// record itself: instrumentation that depends on being remembered is
/// instrumentation that goes stale the next time a call is added.
async fn send_observed(
    builder: reqwest::RequestBuilder,
    method: &str,
    url: &str,
    bytes_out: u64,
) -> Result<reqwest::Response, String> {
    let outcome = builder.send().await;
    crate::network::observe(
        crate::network::Purpose::GraphApi,
        url,
        method,
        bytes_out,
        &outcome,
    );
    outcome.map_err(|e| format!("Could not reach Microsoft Graph: {}", e))
}

/// Maps a non-success Graph response into an error string. The "HTTP 401"
/// marker is load-bearing: command wrappers use it to trigger one forced
/// token refresh + retry.
async fn error_for(response: reqwest::Response) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(String::from))
        .unwrap_or_else(|| body.chars().take(200).collect());
    format!("Graph request failed: HTTP {} — {}", status.as_u16(), message)
}

/// Returns (display name, email) for the signed-in user.
pub async fn me(access_token: &str) -> Result<(String, String), String> {
    let url = format!("{}/me", GRAPH_BASE);
    let response = send_observed(
        http_client()?.get(&url).bearer_auth(access_token),
        "GET",
        &url,
        0,
    )
    .await?;
    if !response.status().is_success() {
        return Err(error_for(response).await);
    }
    let profile: Value = response
        .json()
        .await
        .map_err(|e| format!("Unexpected /me response: {}", e))?;
    let name = profile["displayName"]
        .as_str()
        .unwrap_or("Microsoft 365 account")
        .to_string();
    let email = profile["mail"]
        .as_str()
        .or_else(|| profile["userPrincipalName"].as_str())
        .unwrap_or_default()
        .to_string();
    Ok((name, email))
}

/// Graph returns naive timestamps like "2026-08-04T18:00:00.0000000" in the
/// timezone we asked for (UTC via the Prefer header). Normalize to RFC 3339
/// UTC to match the EventKit path.
fn graph_time_to_rfc3339(value: &Value) -> String {
    let raw = value["dateTime"].as_str().unwrap_or_default();
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(raw) {
        return parsed.with_timezone(&chrono::Utc).to_rfc3339();
    }
    chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f")
        .map(|naive| {
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc)
                .to_rfc3339()
        })
        .unwrap_or_default()
}

/// Lists events starting in the next 24 hours (including in-progress ones),
/// in the same shape as the EventKit path so the sidebar can merge them.
pub async fn upcoming_events(access_token: &str) -> Result<Vec<CalendarEvent>, String> {
    let now = chrono::Utc::now();
    let start = now - chrono::Duration::hours(8); // catch in-progress meetings
    let end = now + chrono::Duration::hours(24);

    let url = format!("{}/me/calendarView", GRAPH_BASE);
    let request = http_client()?
        .get(&url)
        .query(&[
            ("startDateTime", start.to_rfc3339()),
            ("endDateTime", end.to_rfc3339()),
            (
                "$select",
                "subject,start,end,organizer,onlineMeeting,body,location,isCancelled".to_string(),
            ),
            ("$orderby", "start/dateTime".to_string()),
            ("$top", "50".to_string()),
        ])
        .header("Prefer", "outlook.timezone=\"UTC\"")
        .bearer_auth(access_token);
    let response = send_observed(request, "GET", &url, 0).await?;
    if !response.status().is_success() {
        return Err(error_for(response).await);
    }
    let payload: Value = response
        .json()
        .await
        .map_err(|e| format!("Unexpected calendarView response: {}", e))?;

    let now_rfc = now.to_rfc3339();
    let mut events: Vec<CalendarEvent> = payload["value"]
        .as_array()
        .map(|items| items.iter().filter_map(|item| parse_event(item)).collect())
        .unwrap_or_default();
    // The widened window exists only to catch still-running meetings; drop
    // anything already over.
    events.retain(|event| event.end > now_rfc || event.end.is_empty());
    events.sort_by(|a, b| a.start.cmp(&b.start));
    Ok(events)
}

fn parse_event(item: &Value) -> Option<CalendarEvent> {
    if item["isCancelled"].as_bool().unwrap_or(false) {
        return None;
    }
    let title = item["subject"].as_str().unwrap_or("(no title)").to_string();
    let organizer = item["organizer"]["emailAddress"]["name"]
        .as_str()
        .or_else(|| item["organizer"]["emailAddress"]["address"].as_str())
        .map(String::from);

    // Prefer the structured Teams join URL; otherwise scrape the body and
    // location like the EventKit path does. Graph bodies are HTML, where
    // querystring ampersands arrive as &amp; — undo that before scanning.
    let join_url = item["onlineMeeting"]["joinUrl"].as_str().map(String::from);
    let meeting_url = join_url.or_else(|| {
        let body = item["body"]["content"]
            .as_str()
            .unwrap_or_default()
            .replace("&amp;", "&");
        let location = item["location"]["displayName"].as_str().unwrap_or_default();
        extract_meeting_url(&[Some(body.as_str()), Some(location)])
    });

    Some(CalendarEvent {
        id: item["id"].as_str().unwrap_or_default().to_string(),
        title,
        start: graph_time_to_rfc3339(&item["start"]),
        end: graph_time_to_rfc3339(&item["end"]),
        organizer,
        meeting_url,
    })
}

/// Lists attendee email addresses for events overlapping the given UTC window.
/// Used by client suggestion: attendee domains are matched against client
/// domains. Read-only calendar access, same scope as `upcoming_events`.
pub async fn attendee_emails_between(
    access_token: &str,
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<String>, String> {
    let url = format!("{}/me/calendarView", GRAPH_BASE);
    let request = http_client()?
        .get(&url)
        .query(&[
            ("startDateTime", start.to_rfc3339()),
            ("endDateTime", end.to_rfc3339()),
            ("$select", "subject,attendees,organizer,isCancelled".to_string()),
            ("$top", "25".to_string()),
        ])
        .header("Prefer", "outlook.timezone=\"UTC\"")
        .bearer_auth(access_token);
    let response = send_observed(request, "GET", &url, 0).await?;
    if !response.status().is_success() {
        return Err(error_for(response).await);
    }
    let payload: Value = response
        .json()
        .await
        .map_err(|e| format!("Unexpected calendarView response: {}", e))?;

    let mut emails = Vec::new();
    for item in payload["value"].as_array().into_iter().flatten() {
        if item["isCancelled"].as_bool().unwrap_or(false) {
            continue;
        }
        if let Some(address) = item["organizer"]["emailAddress"]["address"].as_str() {
            emails.push(address.to_string());
        }
        for attendee in item["attendees"].as_array().into_iter().flatten() {
            if let Some(address) = attendee["emailAddress"]["address"].as_str() {
                emails.push(address.to_string());
            }
        }
    }
    Ok(emails)
}

/// Creates a DRAFT message (never sends) and returns a URL that opens the
/// draft for review — the Graph-provided webLink when present, otherwise the
/// OWA drafts folder.
pub async fn create_draft(
    access_token: &str,
    subject: &str,
    html_body: &str,
    recipients: &[String],
) -> Result<String, String> {
    let mut message = json!({
        "subject": subject,
        "body": { "contentType": "HTML", "content": html_body },
    });
    if !recipients.is_empty() {
        message["toRecipients"] = Value::Array(
            recipients
                .iter()
                .map(|address| json!({ "emailAddress": { "address": address } }))
                .collect(),
        );
    }

    let url = format!("{}/me/messages", GRAPH_BASE);
    let body = serde_json::to_vec(&message)
        .map_err(|e| format!("Failed to encode the draft message: {}", e))?;
    let bytes_out = body.len() as u64;
    let response = send_observed(
        http_client()?
            .post(&url)
            .bearer_auth(access_token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body),
        "POST",
        &url,
        bytes_out,
    )
    .await?;
    if !response.status().is_success() {
        return Err(error_for(response).await);
    }
    let created: Value = response
        .json()
        .await
        .map_err(|e| format!("Unexpected draft response: {}", e))?;
    Ok(created["webLink"]
        .as_str()
        .unwrap_or("https://outlook.office.com/mail/drafts")
        .to_string())
}
