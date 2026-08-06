//! Device-code OAuth against Microsoft identity platform, with token
//! storage in the OS keychain via the `keyring` crate.
//!
//! Flow: `begin_device_login` POSTs to the /devicecode endpoint, hands the
//! user code + verification URL back to the UI, and spawns a background task
//! that polls the /token endpoint until the user finishes signing in (or the
//! code expires / the attempt is cancelled). On success the tokens are
//! written to the keychain and an `m365-connected` event fires; on terminal
//! failure `m365-auth-failed` fires with a readable message.
//!
//! Refresh is transparent: `access_token()` refreshes when the stored token
//! is within a minute of expiry, and callers hitting a 401 anyway can call
//! `force_refresh()` once and retry.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};

use super::M365Config;

/// Keychain coordinates. One JSON blob per connected account (single
/// account by design).
const KEYRING_SERVICE: &str = "meetily.integrations";
const KEYRING_USER: &str = "m365-tokens";

/// Bumped on every login attempt and on cancel/disconnect so stale poll
/// tasks notice they have been superseded and exit quietly.
static LOGIN_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Serializes token reads + refreshes so two concurrent commands cannot
/// both spend the same refresh token.
static TOKEN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds after which `access_token` is no longer trusted.
    pub expires_at: i64,
    /// The registration the tokens were minted against; refresh uses these,
    /// not the (possibly since-edited) settings values.
    pub client_id: String,
    pub tenant: String,
    pub account_name: String,
    pub account_email: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceLoginStart {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectedAccount {
    pub name: String,
    pub email: String,
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| format!("Keychain unavailable: {}", e))
}

/// Reads the stored token set. `Ok(None)` means "not connected".
pub async fn read_tokens() -> Result<Option<TokenSet>, String> {
    tokio::task::spawn_blocking(|| {
        let entry = entry()?;
        match entry.get_password() {
            Ok(json) => serde_json::from_str::<TokenSet>(&json)
                .map(Some)
                .map_err(|e| format!("Stored Microsoft 365 tokens are corrupt: {}", e)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("Failed to read Microsoft 365 tokens: {}", e)),
        }
    })
    .await
    .map_err(|e| format!("Keychain task failed: {}", e))?
}

async fn write_tokens(tokens: TokenSet) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let entry = entry()?;
        let json = serde_json::to_string(&tokens)
            .map_err(|e| format!("Failed to serialize tokens: {}", e))?;
        entry
            .set_password(&json)
            .map_err(|e| format!("Failed to store Microsoft 365 tokens: {}", e))
    })
    .await
    .map_err(|e| format!("Keychain task failed: {}", e))?
}

pub async fn clear_tokens() -> Result<(), String> {
    // Also invalidate any in-flight login poll task.
    LOGIN_GENERATION.fetch_add(1, Ordering::SeqCst);
    tokio::task::spawn_blocking(|| {
        let entry = entry()?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("Failed to remove Microsoft 365 tokens: {}", e)),
        }
    })
    .await
    .map_err(|e| format!("Keychain task failed: {}", e))?
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: Option<u64>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: i64,
}

#[derive(Deserialize)]
struct OAuthError {
    error: String,
    error_description: Option<String>,
}

/// Starts a device-code login and spawns the background token poll.
pub async fn begin_device_login<R: Runtime>(
    app: AppHandle<R>,
    config: M365Config,
) -> Result<DeviceLoginStart, String> {
    let client = http_client()?;
    let device_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/devicecode",
        config.tenant
    );
    let outcome = client
        .post(&device_url)
        .form(&[("client_id", config.client_id.as_str()), ("scope", SCOPES_WITH_OFFLINE)])
        .send()
        .await;
    crate::network::observe(
        crate::network::Purpose::GraphApi,
        &device_url,
        "POST",
        0,
        &outcome,
    );
    let response = outcome.map_err(|e| format!("Could not reach Microsoft sign-in: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Microsoft sign-in rejected the request (HTTP {}): {}",
            status,
            oauth_error_message(&body)
        ));
    }
    let device: DeviceCodeResponse = response
        .json()
        .await
        .map_err(|e| format!("Unexpected device-code response: {}", e))?;

    let start = DeviceLoginStart {
        user_code: device.user_code.clone(),
        verification_uri: device.verification_uri.clone(),
        expires_in: device.expires_in,
    };

    let generation = LOGIN_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(poll_for_tokens(
        app,
        config,
        device,
        generation,
    ));

    Ok(start)
}

/// Cancels any in-flight device login (the poll task exits on its next tick).
pub fn cancel_device_login() {
    LOGIN_GENERATION.fetch_add(1, Ordering::SeqCst);
}

const SCOPES_WITH_OFFLINE: &str = super::SCOPES;

async fn poll_for_tokens<R: Runtime>(
    app: AppHandle<R>,
    config: M365Config,
    device: DeviceCodeResponse,
    generation: u64,
) {
    let client = match http_client() {
        Ok(c) => c,
        Err(e) => {
            emit_auth_failed(&app, &e);
            return;
        }
    };
    let token_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        config.tenant
    );
    let mut interval = device.interval.unwrap_or(5).max(1);
    let deadline = now_unix() + device.expires_in as i64;

    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        if LOGIN_GENERATION.load(Ordering::SeqCst) != generation {
            log::info!("M365 device login superseded or cancelled; poll task exiting");
            return;
        }
        if now_unix() > deadline {
            emit_auth_failed(&app, "The sign-in code expired before it was used. Start again to get a new code.");
            return;
        }

        let response = client
            .post(&token_url)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", config.client_id.as_str()),
                ("device_code", device.device_code.as_str()),
            ])
            .send()
            .await;
        crate::network::observe(
            crate::network::Purpose::GraphApi,
            &token_url,
            "POST",
            0,
            &response,
        );

        let response = match response {
            Ok(r) => r,
            Err(e) => {
                // Transient network trouble: keep polling until the code expires.
                log::warn!("M365 token poll failed (will retry): {}", e);
                continue;
            }
        };

        if response.status().is_success() {
            let tokens: TokenResponse = match response.json().await {
                Ok(t) => t,
                Err(e) => {
                    emit_auth_failed(&app, &format!("Unexpected token response: {}", e));
                    return;
                }
            };
            let refresh_token = match tokens.refresh_token {
                Some(t) => t,
                None => {
                    emit_auth_failed(
                        &app,
                        "Microsoft did not return a refresh token. Check that the app registration allows the offline_access scope.",
                    );
                    return;
                }
            };
            // Resolve the signed-in account for the settings UI.
            let (account_name, account_email) =
                match super::graph::me(&tokens.access_token).await {
                    Ok(pair) => pair,
                    Err(e) => {
                        log::warn!("M365 /me lookup failed after login: {}", e);
                        (String::from("Microsoft 365 account"), String::new())
                    }
                };
            let token_set = TokenSet {
                access_token: tokens.access_token,
                refresh_token,
                expires_at: now_unix() + tokens.expires_in.max(60) - 60,
                client_id: config.client_id.clone(),
                tenant: config.tenant.clone(),
                account_name: account_name.clone(),
                account_email: account_email.clone(),
            };
            if let Err(e) = write_tokens(token_set).await {
                emit_auth_failed(&app, &e);
                return;
            }
            log::info!("M365 connected as {}", account_email);
            let _ = app.emit(
                "m365-connected",
                ConnectedAccount {
                    name: account_name,
                    email: account_email,
                },
            );
            return;
        }

        let body = response.text().await.unwrap_or_default();
        let error: Option<OAuthError> = serde_json::from_str(&body).ok();
        match error.as_ref().map(|e| e.error.as_str()) {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval += 5;
                continue;
            }
            _ => {
                emit_auth_failed(&app, &oauth_error_message(&body));
                return;
            }
        }
    }
}

fn emit_auth_failed<R: Runtime>(app: &AppHandle<R>, message: &str) {
    log::warn!("M365 login failed: {}", message);
    let _ = app.emit("m365-auth-failed", message.to_string());
}

/// Pulls the human sentence out of an OAuth error body, falling back to the
/// raw body (truncated) when it is not the JSON shape we expect.
fn oauth_error_message(body: &str) -> String {
    match serde_json::from_str::<OAuthError>(body) {
        Ok(e) => e
            .error_description
            .map(|d| d.lines().next().unwrap_or_default().to_string())
            .filter(|d| !d.is_empty())
            .unwrap_or(e.error),
        Err(_) => body.chars().take(200).collect(),
    }
}

/// Returns a currently-valid access token, refreshing if needed.
pub async fn access_token() -> Result<String, String> {
    let _guard = TOKEN_LOCK.lock().await;
    let tokens = read_tokens()
        .await?
        .ok_or_else(|| "Microsoft 365 is not connected".to_string())?;
    if tokens.expires_at > now_unix() {
        return Ok(tokens.access_token);
    }
    refresh_locked(tokens).await
}

/// Refreshes unconditionally (used after an unexpected 401).
pub async fn force_refresh() -> Result<String, String> {
    let _guard = TOKEN_LOCK.lock().await;
    let tokens = read_tokens()
        .await?
        .ok_or_else(|| "Microsoft 365 is not connected".to_string())?;
    refresh_locked(tokens).await
}

async fn refresh_locked(tokens: TokenSet) -> Result<String, String> {
    let client = http_client()?;
    let token_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        tokens.tenant
    );
    let outcome = client
        .post(&token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", tokens.client_id.as_str()),
            ("refresh_token", tokens.refresh_token.as_str()),
            ("scope", super::SCOPES),
        ])
        .send()
        .await;
    crate::network::observe(
        crate::network::Purpose::GraphApi,
        &token_url,
        "POST",
        0,
        &outcome,
    );
    let response =
        outcome.map_err(|e| format!("Could not reach Microsoft sign-in to refresh: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let is_terminal = serde_json::from_str::<OAuthError>(&body)
            .map(|e| e.error == "invalid_grant")
            .unwrap_or(false);
        if is_terminal {
            // Refresh token revoked/expired: drop the dead session so the UI
            // shows "not connected" instead of failing forever.
            let _ = clear_tokens().await;
            return Err(
                "Your Microsoft 365 session expired. Reconnect in Settings → Integrations."
                    .to_string(),
            );
        }
        return Err(format!(
            "Token refresh failed (HTTP {}): {}",
            status,
            oauth_error_message(&body)
        ));
    }

    let refreshed: TokenResponse = response
        .json()
        .await
        .map_err(|e| format!("Unexpected refresh response: {}", e))?;
    let access_token = refreshed.access_token;
    let new_tokens = TokenSet {
        access_token: access_token.clone(),
        // Microsoft rotates refresh tokens; keep the old one only if no new
        // one was issued.
        refresh_token: refreshed
            .refresh_token
            .unwrap_or_else(|| tokens.refresh_token.clone()),
        expires_at: now_unix() + refreshed.expires_in.max(60) - 60,
        client_id: tokens.client_id,
        tenant: tokens.tenant,
        account_name: tokens.account_name,
        account_email: tokens.account_email,
    };
    write_tokens(new_tokens).await?;
    Ok(access_token)
}
