//! Tauri command surface for the Microsoft 365 integration.

use serde::Serialize;
use tauri::{AppHandle, Runtime};

use crate::calendar::CalendarEvent;

use super::{auth, graph, M365Config};

#[derive(Debug, Clone, Serialize)]
pub struct M365AuthStatus {
    pub connected: bool,
    pub account_name: Option<String>,
    pub account_email: Option<String>,
}

#[tauri::command]
pub async fn m365_get_config<R: Runtime>(app: AppHandle<R>) -> Result<M365Config, String> {
    Ok(super::load_config(&app))
}

/// Saves client id / tenant overrides (empty resets to defaults). Changing
/// the registration invalidates any existing session, so tokens are cleared.
#[tauri::command]
pub async fn m365_set_config<R: Runtime>(
    app: AppHandle<R>,
    client_id: Option<String>,
    tenant: Option<String>,
) -> Result<M365Config, String> {
    let before = super::load_config(&app);
    let after = super::save_config(&app, client_id, tenant)?;
    if before.client_id != after.client_id || before.tenant != after.tenant {
        auth::clear_tokens().await?;
    }
    Ok(after)
}

#[tauri::command]
pub async fn m365_auth_status() -> Result<M365AuthStatus, String> {
    let tokens = auth::read_tokens().await?;
    Ok(match tokens {
        Some(t) => M365AuthStatus {
            connected: true,
            account_name: Some(t.account_name),
            account_email: Some(t.account_email),
        },
        None => M365AuthStatus {
            connected: false,
            account_name: None,
            account_email: None,
        },
    })
}

/// Starts a device-code sign-in. Returns the code + verification URL for the
/// UI; completion is announced via the `m365-connected` / `m365-auth-failed`
/// events emitted by the background poll task.
#[tauri::command]
pub async fn m365_begin_device_login<R: Runtime>(
    app: AppHandle<R>,
) -> Result<auth::DeviceLoginStart, String> {
    let config = super::load_config(&app);
    auth::begin_device_login(app, config).await
}

#[tauri::command]
pub async fn m365_cancel_device_login() -> Result<(), String> {
    auth::cancel_device_login();
    Ok(())
}

#[tauri::command]
pub async fn m365_disconnect() -> Result<(), String> {
    auth::clear_tokens().await
}

/// Internal helper shared with the autojoin scheduler: fetch upcoming events
/// with one forced-refresh retry on 401.
pub(crate) async fn upcoming_events_with_refresh() -> Result<Vec<CalendarEvent>, String> {
    let token = auth::access_token().await?;
    match graph::upcoming_events(&token).await {
        Err(e) if e.contains("HTTP 401") => {
            let token = auth::force_refresh().await?;
            graph::upcoming_events(&token).await
        }
        other => other,
    }
}

#[tauri::command]
pub async fn m365_upcoming_events() -> Result<Vec<CalendarEvent>, String> {
    upcoming_events_with_refresh().await
}

/// Creates an Outlook DRAFT of a meeting summary (explicit share action).
/// Returns a URL that opens the draft in Outlook on the web for review; the
/// user presses send there. Never sends mail itself.
#[tauri::command]
pub async fn m365_create_summary_draft(
    state: tauri::State<'_, crate::state::AppState>,
    subject: String,
    markdown: String,
    recipients: Option<Vec<String>>,
    meeting_id: Option<String>,
    client_id: Option<String>,
) -> Result<String, String> {
    if markdown.trim().is_empty() {
        return Err("There is no summary content to share".to_string());
    }

    // Privacy profile: refuse the draft when the governing profile has sharing
    // off, and mask obvious secrets in the body otherwise.
    let pool = state.db_manager.pool();
    let non_empty = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
    };
    // A meeting draft is governed by that meeting's profile; a client-level draft
    // (the follow-through chase) by the client's.
    let scope = match (
        non_empty(meeting_id.as_deref()),
        non_empty(client_id.as_deref()),
    ) {
        (Some(id), _) => crate::profiles::enforce::Scope::meeting(id),
        (None, Some(id)) => crate::profiles::enforce::Scope::client(id),
        (None, None) => crate::profiles::enforce::Scope::Workspace,
    };
    let effective = crate::profiles::enforce::guard_sharing(pool, &scope).await?;
    let (markdown, _) = crate::profiles::enforce::redact_for(&effective, &markdown);
    let recipients = recipients.unwrap_or_default();
    let blocks = crate::export::markdown_ast::parse_markdown(&markdown);
    let body_html = format!(
        "<div style=\"font-family: -apple-system, Segoe UI, sans-serif; font-size: 14px; line-height: 1.5;\">{}</div>",
        crate::export::html::blocks_to_html(&blocks)
    );

    let token = auth::access_token().await?;
    let result = graph::create_draft(&token, &subject, &body_html, &recipients).await;
    match result {
        Err(e) if e.contains("HTTP 401") => {
            let token = auth::force_refresh().await?;
            graph::create_draft(&token, &subject, &body_html, &recipients).await
        }
        other => other,
    }
}
