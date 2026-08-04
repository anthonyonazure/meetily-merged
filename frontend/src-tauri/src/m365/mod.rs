//! Microsoft 365 (Graph) integration.
//!
//! Privacy contract: the only network traffic this module initiates on its
//! own is the OAuth device-code exchange with Microsoft's official login
//! endpoints. Graph reads (calendar, /me) happen only after the user has
//! explicitly connected an account, and the one write surface — creating an
//! Outlook DRAFT of a meeting summary — only runs on an explicit per-meeting
//! user action. Nothing is ever sent automatically, and drafts are never
//! auto-sent: the user reviews and presses send in Outlook.
//!
//! Tokens live in the OS keychain (see `auth`), never in SQLite or the
//! store plugin. The store plugin only holds the non-secret app
//! registration overrides (client id / tenant).

pub mod auth;
pub mod commands;
pub mod graph;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

/// Default Entra app registration (Anthony's tenant). Overridable in
/// Settings → Integrations for other tenants; stored in the integrations
/// store, not hardcoded anywhere else.
pub const DEFAULT_CLIENT_ID: &str = "b2c2095c-f3e0-4a72-bf0e-ab98404b5658";
pub const DEFAULT_TENANT: &str = "52820ba4-0d2c-457f-b117-7b7b0db1180e";
/// Delegated scopes: profile read, calendar read, mail draft creation, and
/// refresh tokens. Mail.ReadWrite (not Mail.Send) on purpose — the app can
/// create drafts but can never send mail.
pub const SCOPES: &str = "User.Read Calendars.Read Mail.ReadWrite offline_access";

/// Store file shared by the integrations settings surfaces (M365 config
/// overrides here; the frontend also keeps the Google client id and the
/// autojoin toggle in the same file).
pub const INTEGRATIONS_STORE: &str = "integrations.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct M365Config {
    pub client_id: String,
    pub tenant: String,
    /// True when both values are the built-in defaults.
    pub is_default: bool,
}

/// Client id / tenant values are embedded in login URLs, so restrict them to
/// the characters real Entra values use (GUIDs, domain names, or the
/// "common" / "organizations" / "consumers" aliases).
fn valid_config_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
}

pub fn load_config<R: Runtime>(app: &AppHandle<R>) -> M365Config {
    let stored = |key: &str| -> Option<String> {
        let store = app.store(INTEGRATIONS_STORE).ok()?;
        let value = store.get(key)?;
        let value = value.as_str()?.trim().to_string();
        valid_config_value(&value).then_some(value)
    };
    let client_id = stored("m365_client_id").unwrap_or_else(|| DEFAULT_CLIENT_ID.to_string());
    let tenant = stored("m365_tenant").unwrap_or_else(|| DEFAULT_TENANT.to_string());
    let is_default = client_id == DEFAULT_CLIENT_ID && tenant == DEFAULT_TENANT;
    M365Config {
        client_id,
        tenant,
        is_default,
    }
}

/// Persists overrides. `None` or empty string resets a field to its default.
pub fn save_config<R: Runtime>(
    app: &AppHandle<R>,
    client_id: Option<String>,
    tenant: Option<String>,
) -> Result<M365Config, String> {
    let store = app
        .store(INTEGRATIONS_STORE)
        .map_err(|e| format!("Failed to open integrations store: {}", e))?;

    let mut apply = |key: &str, value: Option<String>| -> Result<(), String> {
        let value = value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        match value {
            Some(v) if !valid_config_value(&v) => {
                Err(format!("Invalid value for {}: only letters, digits, '-', '.' and '_' are allowed", key))
            }
            Some(v) => {
                store.set(key, serde_json::Value::String(v));
                Ok(())
            }
            None => {
                store.delete(key);
                Ok(())
            }
        }
    };
    apply("m365_client_id", client_id)?;
    apply("m365_tenant", tenant)?;
    store
        .save()
        .map_err(|e| format!("Failed to save integrations store: {}", e))?;
    Ok(load_config(app))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_values_are_restricted_to_entra_shapes() {
        assert!(valid_config_value(DEFAULT_CLIENT_ID));
        assert!(valid_config_value("common"));
        assert!(valid_config_value("contoso.onmicrosoft.com"));
        assert!(!valid_config_value(""));
        assert!(!valid_config_value("evil/../path"));
        assert!(!valid_config_value("a b"));
        assert!(!valid_config_value(&"x".repeat(200)));
    }
}
