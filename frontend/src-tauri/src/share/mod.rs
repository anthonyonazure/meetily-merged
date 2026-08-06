//! Slack / Teams summary sharing via user-supplied incoming webhooks.
//!
//! Privacy contract: webhook URLs are secrets and live in the OS keychain.
//! A post happens ONLY when the user presses a per-meeting share button —
//! there is no automatic posting of anything, ever.

use std::time::Duration;

use serde::Serialize;
use serde_json::json;

const KEYRING_SERVICE: &str = "meetily.integrations";

/// The two supported webhook kinds. Anything else is rejected before it can
/// touch the keychain.
fn keyring_user(kind: &str) -> Result<&'static str, String> {
    match kind {
        "slack" => Ok("slack-webhook"),
        "teams" => Ok("teams-webhook"),
        other => Err(format!("Unknown share target '{}'", other)),
    }
}

fn entry(kind: &str) -> Result<keyring::Entry, String> {
    let user = keyring_user(kind)?;
    keyring::Entry::new(KEYRING_SERVICE, user)
        .map_err(|e| format!("Keychain unavailable: {}", e))
}

async fn read_webhook(kind: &'static str) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        let entry = entry(kind)?;
        match entry.get_password() {
            Ok(url) => Ok(Some(url)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("Failed to read {} webhook: {}", kind, e)),
        }
    })
    .await
    .map_err(|e| format!("Keychain task failed: {}", e))?
}

/// Webhooks must be HTTPS URLs with a real host. (Both Slack and Teams only
/// issue HTTPS webhook URLs; anything else is a paste error or worse.)
fn validate_webhook_url(url: &str) -> Result<String, String> {
    let parsed = url::Url::parse(url.trim())
        .map_err(|e| format!("That does not look like a valid URL: {}", e))?;
    if parsed.scheme() != "https" {
        return Err("Webhook URLs must start with https://".to_string());
    }
    if parsed.host_str().map(str::is_empty).unwrap_or(true) {
        return Err("Webhook URL is missing a host".to_string());
    }
    Ok(parsed.to_string())
}

/// Applies the governing privacy profile to a share action: refuses it outright
/// when the profile has sharing off, and masks obvious secrets in the markdown
/// otherwise. `meeting_id` is the meeting whose summary is being shared; without
/// it the workspace default profile applies.
async fn apply_profile(
    state: &tauri::State<'_, crate::state::AppState>,
    meeting_id: Option<&str>,
    markdown: &str,
) -> Result<String, String> {
    let pool = state.db_manager.pool();
    let scope = match meeting_id {
        Some(id) if !id.trim().is_empty() => {
            crate::profiles::enforce::Scope::meeting(id.trim().to_string())
        }
        _ => crate::profiles::enforce::Scope::Workspace,
    };
    let effective = crate::profiles::enforce::guard_sharing(pool, &scope).await?;
    let (masked, _) = crate::profiles::enforce::redact_for(&effective, markdown);
    Ok(masked)
}

#[derive(Debug, Clone, Serialize)]
pub struct ShareTargets {
    pub slack: bool,
    pub teams: bool,
}

/// Reports which share targets have a stored webhook (never the URLs
/// themselves — they stay in the keychain).
#[tauri::command]
pub async fn share_get_targets() -> Result<ShareTargets, String> {
    Ok(ShareTargets {
        slack: read_webhook("slack").await?.is_some(),
        teams: read_webhook("teams").await?.is_some(),
    })
}

/// Stores or clears (empty url) the webhook for a share target.
#[tauri::command]
pub async fn share_set_webhook(kind: String, url: String) -> Result<(), String> {
    keyring_user(&kind)?; // validate kind up front
    let trimmed = url.trim().to_string();
    tokio::task::spawn_blocking(move || {
        let entry = entry(&kind)?;
        if trimmed.is_empty() {
            return match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(format!("Failed to remove webhook: {}", e)),
            };
        }
        let valid = validate_webhook_url(&trimmed)?;
        entry
            .set_password(&valid)
            .map_err(|e| format!("Failed to store webhook: {}", e))
    })
    .await
    .map_err(|e| format!("Keychain task failed: {}", e))?
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

async fn post_json(url: &str, payload: &serde_json::Value, target: &str) -> Result<(), String> {
    // Serialised once so the byte count in the network log is exact rather than an
    // estimate; the header keeps this equivalent to `.json()`.
    let body = serde_json::to_vec(payload)
        .map_err(|e| format!("Failed to encode the {} payload: {}", target, e))?;
    let bytes_out = body.len() as u64;
    let outcome = http_client()?
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await;
    crate::network::observe(
        crate::network::Purpose::ShareWebhook,
        url,
        "POST",
        bytes_out,
        &outcome,
    );
    let response =
        outcome.map_err(|e| format!("Could not reach the {} webhook: {}", target, e))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "{} webhook rejected the post (HTTP {}): {}",
            target,
            status.as_u16(),
            body.chars().take(200).collect::<String>()
        ));
    }
    Ok(())
}

/// Slack mrkdwn uses *single asterisks* for bold.
fn to_slack_mrkdwn(markdown: &str) -> String {
    markdown.replace("**", "*")
}

/// Splits text into chunks that fit Slack's ~3000-char section-block limit,
/// breaking on line boundaries where possible.
fn chunk_text(text: &str, max_len: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        // A single pathological line longer than the limit gets hard-split.
        let mut line = line;
        while line.len() > max_len {
            let split_at = (0..=max_len).rev().find(|i| line.is_char_boundary(*i)).unwrap_or(0);
            if split_at == 0 {
                // Cannot make progress (limit smaller than one char);
                // fall through and accept an oversized chunk.
                break;
            }
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            chunks.push(line[..split_at].to_string());
            line = &line[split_at..];
        }
        if current.len() + line.len() + 1 > max_len && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    chunks
}

/// Posts a summary to the configured Slack incoming webhook. Explicit
/// per-meeting user action only.
#[tauri::command]
pub async fn share_slack(
    state: tauri::State<'_, crate::state::AppState>,
    title: String,
    markdown: String,
    meeting_id: Option<String>,
) -> Result<(), String> {
    let markdown = apply_profile(&state, meeting_id.as_deref(), &markdown).await?;
    let url = read_webhook("slack")
        .await?
        .ok_or_else(|| "No Slack webhook configured. Add one in Settings → Integrations.".to_string())?;

    let text = to_slack_mrkdwn(markdown.trim());
    if text.is_empty() {
        return Err("There is no summary content to share".to_string());
    }
    let mut blocks = vec![json!({
        "type": "header",
        "text": { "type": "plain_text", "text": title.chars().take(150).collect::<String>(), "emoji": true }
    })];
    for chunk in chunk_text(&text, 2900).into_iter().take(15) {
        blocks.push(json!({
            "type": "section",
            "text": { "type": "mrkdwn", "text": chunk }
        }));
    }
    let payload = json!({ "text": title, "blocks": blocks });
    post_json(&url, &payload, "Slack").await
}

/// Posts a summary to the configured Teams incoming webhook as a simple
/// MessageCard. Explicit per-meeting user action only.
#[tauri::command]
pub async fn share_teams(
    state: tauri::State<'_, crate::state::AppState>,
    title: String,
    markdown: String,
    meeting_id: Option<String>,
) -> Result<(), String> {
    let markdown = apply_profile(&state, meeting_id.as_deref(), &markdown).await?;
    let url = read_webhook("teams")
        .await?
        .ok_or_else(|| "No Teams webhook configured. Add one in Settings → Integrations.".to_string())?;

    let mut text = markdown.trim().to_string();
    if text.is_empty() {
        return Err("There is no summary content to share".to_string());
    }
    // Teams webhook payloads cap out around 28KB; leave headroom.
    if text.len() > 25_000 {
        let cut = (0..=25_000).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
        text.truncate(cut);
        text.push_str("\n\n*(truncated)*");
    }
    let payload = json!({
        "@type": "MessageCard",
        "@context": "https://schema.org/extensions",
        "summary": title,
        "themeColor": "2F5496",
        "title": title,
        "text": text,
    });
    post_json(&url, &payload, "Teams").await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_urls_must_be_https_with_host() {
        assert!(validate_webhook_url("https://hooks.slack.com/services/T/B/x").is_ok());
        assert!(validate_webhook_url("http://hooks.slack.com/services/T/B/x").is_err());
        assert!(validate_webhook_url("file:///etc/passwd").is_err());
        assert!(validate_webhook_url("not a url").is_err());
    }

    #[test]
    fn chunking_respects_line_boundaries_and_limits() {
        let text = "line one\nline two\nline three";
        assert_eq!(chunk_text(text, 100), vec![text.to_string()]);
        let chunks = chunk_text(text, 10);
        assert!(chunks.len() >= 3);
        assert!(chunks.iter().all(|c| c.len() <= 10));
        let long = "x".repeat(25);
        assert!(chunk_text(&long, 10).iter().all(|c| c.len() <= 10));
    }

    #[test]
    fn slack_bold_markers_are_converted() {
        assert_eq!(to_slack_mrkdwn("**Key points**"), "*Key points*");
    }
}
