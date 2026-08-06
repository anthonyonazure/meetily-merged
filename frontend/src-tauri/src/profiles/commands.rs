//! Tauri command surface for privacy profiles and retention.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::database::repositories::profile::{
    ClientProfileRepository, PrivacyProfilesRepository, PrivacySettingsRepository,
};
use crate::state::AppState;

use super::redaction::{self, RedactionReport};
use super::resolver::{self, EffectiveProfile};
use super::rules::{is_builtin_id, PrivacyProfile, ProcessingMode};
use crate::consent::rules::{ConsentLevel, EnforcementMode};

/// Trim and bound any operator-supplied string before it reaches the database.
fn bounded(value: &str, max: usize) -> String {
    let trimmed = value.trim();
    trimmed.chars().take(max).collect()
}

fn validate_name(name: &str) -> Result<String, String> {
    let name = bounded(name, 120);
    if name.is_empty() {
        return Err("Profile name cannot be empty".to_string());
    }
    Ok(name)
}

/// A retention window has to be a positive number of days. Null means "keep".
fn validate_retention(days: Option<i64>) -> Result<Option<i64>, String> {
    match days {
        None => Ok(None),
        Some(days) if days <= 0 => {
            Err("A retention window has to be at least one day. Leave it empty to keep meetings.".to_string())
        }
        Some(days) if days > 36_500 => {
            Err("That retention window is longer than a hundred years; leave it empty to keep meetings.".to_string())
        }
        Some(days) => Ok(Some(days)),
    }
}

/// Editable fields of a profile, as the settings editor sends them.
#[derive(Debug, Deserialize)]
pub struct PrivacyProfileInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub transcription_mode: String,
    pub llm_mode: String,
    pub consent_level: String,
    pub consent_enforcement: String,
    #[serde(default)]
    pub retention_days: Option<i64>,
    #[serde(default)]
    pub redact_pii: bool,
    #[serde(default = "default_true")]
    pub allow_sharing: bool,
}

fn default_true() -> bool {
    true
}

/// Workspace-level privacy settings for the settings screen.
#[derive(Debug, Serialize)]
pub struct PrivacySettings {
    /// None means no profile governs untagged meetings, and the app's global
    /// transcription, model, and consent settings apply as before.
    pub default_profile_id: Option<String>,
}

/// The resolved profile for a meeting plus the client it came through, for the
/// chip on meeting details.
#[derive(Debug, Serialize)]
pub struct MeetingProfileView {
    #[serde(flatten)]
    pub effective: EffectiveProfile,
    /// The one-line description that also went into the consent log.
    pub summary: String,
}

// ---------------------------------------------------------------------------
// Profiles
// ---------------------------------------------------------------------------

/// Every profile, built-ins first.
#[tauri::command]
pub async fn privacy_profiles_list(
    state: State<'_, AppState>,
) -> Result<Vec<PrivacyProfile>, String> {
    let rows = PrivacyProfilesRepository::list(state.db_manager.pool())
        .await
        .map_err(|e| format!("Failed to load privacy profiles: {}", e))?;
    Ok(rows.into_iter().map(PrivacyProfile::from_row).collect())
}

/// Creates a custom profile. Used both by "New profile" and by the duplicate
/// action, which sends a copy of an existing profile's fields.
#[tauri::command]
pub async fn privacy_profile_create(
    state: State<'_, AppState>,
    input: PrivacyProfileInput,
) -> Result<PrivacyProfile, String> {
    let name = validate_name(&input.name)?;
    let retention_days = validate_retention(input.retention_days)?;
    let row = PrivacyProfilesRepository::create(
        state.db_manager.pool(),
        &name,
        &bounded(&input.description, 500),
        ProcessingMode::parse(&input.transcription_mode).as_str(),
        ProcessingMode::parse(&input.llm_mode).as_str(),
        ConsentLevel::parse(&input.consent_level).as_str(),
        EnforcementMode::parse(&input.consent_enforcement).as_str(),
        retention_days,
        input.redact_pii,
        input.allow_sharing,
    )
    .await
    .map_err(|e| format!("Failed to create privacy profile: {}", e))?;
    Ok(PrivacyProfile::from_row(row))
}

/// Updates a profile. Built-ins are editable (renaming and retuning them is
/// fine); only deletion is refused.
#[tauri::command]
pub async fn privacy_profile_update(
    state: State<'_, AppState>,
    profile_id: String,
    input: PrivacyProfileInput,
) -> Result<PrivacyProfile, String> {
    let pool = state.db_manager.pool();
    let name = validate_name(&input.name)?;
    let retention_days = validate_retention(input.retention_days)?;
    let updated = PrivacyProfilesRepository::update(
        pool,
        &profile_id,
        &name,
        &bounded(&input.description, 500),
        ProcessingMode::parse(&input.transcription_mode).as_str(),
        ProcessingMode::parse(&input.llm_mode).as_str(),
        ConsentLevel::parse(&input.consent_level).as_str(),
        EnforcementMode::parse(&input.consent_enforcement).as_str(),
        retention_days,
        input.redact_pii,
        input.allow_sharing,
    )
    .await
    .map_err(|e| format!("Failed to update privacy profile: {}", e))?;
    if !updated {
        return Err("Privacy profile not found".to_string());
    }
    PrivacyProfilesRepository::get(pool, &profile_id)
        .await
        .map_err(|e| format!("Failed to reload privacy profile: {}", e))?
        .map(PrivacyProfile::from_row)
        .ok_or_else(|| "Privacy profile not found".to_string())
}

/// Deletes a custom profile. The three shipped profiles cannot be deleted;
/// clients pointing at a deleted profile fall back to the workspace default.
#[tauri::command]
pub async fn privacy_profile_delete(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<bool, String> {
    if is_builtin_id(&profile_id) {
        return Err(
            "The three shipped profiles cannot be deleted. Rename one, or duplicate it and edit the copy."
                .to_string(),
        );
    }
    PrivacyProfilesRepository::delete(state.db_manager.pool(), &profile_id)
        .await
        .map_err(|e| format!("Failed to delete privacy profile: {}", e))
}

/// How many clients a profile is attached to, for the delete confirmation.
#[tauri::command]
pub async fn privacy_profile_usage(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<i64, String> {
    PrivacyProfilesRepository::client_usage(state.db_manager.pool(), &profile_id)
        .await
        .map_err(|e| format!("Failed to count profile use: {}", e))
}

// ---------------------------------------------------------------------------
// Workspace settings and client attachment
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn privacy_settings_get(state: State<'_, AppState>) -> Result<PrivacySettings, String> {
    let row = PrivacySettingsRepository::get(state.db_manager.pool())
        .await
        .map_err(|e| format!("Failed to read privacy settings: {}", e))?;
    Ok(PrivacySettings {
        default_profile_id: row.and_then(|r| r.default_profile_id),
    })
}

/// Sets the profile that governs meetings with no client tag. Null clears it,
/// which puts the app back on its global settings.
#[tauri::command]
pub async fn privacy_settings_set_default(
    state: State<'_, AppState>,
    profile_id: Option<String>,
) -> Result<PrivacySettings, String> {
    let pool = state.db_manager.pool();
    if let Some(id) = profile_id.as_deref() {
        if PrivacyProfilesRepository::get(pool, id)
            .await
            .map_err(|e| format!("Failed to read privacy profile: {}", e))?
            .is_none()
        {
            return Err("That privacy profile no longer exists".to_string());
        }
    }
    PrivacySettingsRepository::set_default_profile(pool, profile_id.as_deref())
        .await
        .map_err(|e| format!("Failed to save the default profile: {}", e))?;
    privacy_settings_get(state).await
}

/// Attaches a profile to a client, or clears it with null.
#[tauri::command]
pub async fn client_set_privacy_profile(
    state: State<'_, AppState>,
    client_id: String,
    profile_id: Option<String>,
) -> Result<EffectiveProfile, String> {
    let pool = state.db_manager.pool();
    if let Some(id) = profile_id.as_deref() {
        if PrivacyProfilesRepository::get(pool, id)
            .await
            .map_err(|e| format!("Failed to read privacy profile: {}", e))?
            .is_none()
        {
            return Err("That privacy profile no longer exists".to_string());
        }
    }
    let updated = ClientProfileRepository::set(pool, &client_id, profile_id.as_deref())
        .await
        .map_err(|e| format!("Failed to set the client's privacy profile: {}", e))?;
    if !updated {
        return Err("Client not found".to_string());
    }
    Ok(resolver::for_client(pool, &client_id).await)
}

/// The profile that governs a meeting, and how it was reached.
#[tauri::command]
pub async fn meeting_privacy_profile(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<MeetingProfileView, String> {
    let effective = resolver::for_meeting(state.db_manager.pool(), &meeting_id).await;
    Ok(MeetingProfileView {
        summary: effective.describe(),
        effective,
    })
}

// ---------------------------------------------------------------------------
// Redaction
// ---------------------------------------------------------------------------

/// Masks a piece of text with the same matchers the LLM, export, and share
/// paths use. Lets the settings screen show an operator exactly what masking
/// does before they turn it on for a client.
#[tauri::command]
pub async fn privacy_redaction_preview(text: String) -> Result<RedactionPreview, String> {
    let (masked, report) = redaction::redact(&text);
    Ok(RedactionPreview { masked, report })
}

#[derive(Debug, Serialize)]
pub struct RedactionPreview {
    pub masked: String,
    pub report: RedactionReport,
}
