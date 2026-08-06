//! Fleet configuration: central policy without a vendor cloud.
//!
//! An MSP deploys this app to technician machines and needs to set policy once
//! rather than per laptop. The mechanism is deliberately the dullest one available:
//! a JSON file at a platform-conventional path that any MDM or RMM can already
//! push. No enrolment, no account, no server of ours in the middle — which also
//! means this feature adds no outbound host.
//!
//! Layout:
//! * `config` — parsing the file. Pure.
//! * `overlay` — how managed values combine with local ones. Pure.
//! * this file — reading the file from disk and holding the result.
//! * `commands` — the Tauri surface.

pub mod commands;
pub mod config;
pub mod overlay;

use once_cell::sync::Lazy;
use serde::Serialize;
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::RwLock;

pub use config::ManagedConfig;

/// The meeting id the startup provenance row is filed under. `consent_events`
/// requires one and this event belongs to the machine rather than any recording,
/// so it gets a reserved, obviously-not-a-meeting id.
pub const SYSTEM_SUBJECT_ID: &str = "system:managed-config";

/// The event type appended to the consent log at startup.
pub const PROVENANCE_EVENT: &str = "managed_config_applied";

const FILE_NAME: &str = "managed-config.json";

/// What was found on disk, if anything.
#[derive(Debug, Clone, Serialize)]
pub struct ManagedState {
    pub config: ManagedConfig,
    /// Where the app looked.
    pub path: String,
    /// Whether a file was there.
    pub found: bool,
    /// Set when a file was there but could not be read or parsed. Local settings
    /// govern in that case, and the panel says so rather than pretending policy
    /// applied.
    pub error: Option<String>,
    /// Keys an administrator is allowed to lock, so the UI can explain the field.
    pub lockable_keys: Vec<String>,
}

impl ManagedState {
    fn absent(path: PathBuf) -> Self {
        Self {
            config: ManagedConfig::default(),
            path: path.display().to_string(),
            found: false,
            error: None,
            lockable_keys: config::LOCKABLE_KEYS.iter().map(|k| k.to_string()).collect(),
        }
    }
}

static STATE: Lazy<RwLock<Option<ManagedState>>> = Lazy::new(|| RwLock::new(None));

/// Where an MDM or RMM should put the file on this platform.
pub fn managed_config_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/Library/Application Support/meetily++").join(FILE_NAME)
    }
    #[cfg(target_os = "windows")]
    {
        let program_data = std::env::var("ProgramData")
            .unwrap_or_else(|_| "C:\\ProgramData".to_string());
        PathBuf::from(program_data).join("meetily++").join(FILE_NAME)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        PathBuf::from("/etc/meetily++").join(FILE_NAME)
    }
}

/// Reads the file from disk and replaces the held state.
pub fn reload() -> ManagedState {
    reload_from(managed_config_path())
}

/// Reads a policy file from an explicit path and replaces the held state.
///
/// The path is a parameter rather than always the platform one so the policy rules
/// can be tested against a real file. There is deliberately no environment variable
/// or setting that redirects it in the shipped app: a fleet policy that a local
/// process could point somewhere else would not be a policy.
pub fn reload_from(path: PathBuf) -> ManagedState {
    let state = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::info!(
                "[Fleet] no managed configuration at {}; local settings govern",
                path.display()
            );
            ManagedState::absent(path)
        }
        Err(e) => {
            log::warn!(
                "[Fleet] managed configuration at {} could not be read: {}",
                path.display(),
                e
            );
            ManagedState {
                error: Some(format!("The file could not be read: {}", e)),
                found: true,
                ..ManagedState::absent(path)
            }
        }
        Ok(text) => match config::parse(&text) {
            Ok(parsed) => {
                log::info!("[Fleet] managed configuration applied: {}", parsed.describe());
                for warning in &parsed.warnings {
                    log::warn!("[Fleet] {}", warning);
                }
                ManagedState {
                    config: parsed,
                    path: path.display().to_string(),
                    found: true,
                    error: None,
                    lockable_keys: config::LOCKABLE_KEYS.iter().map(|k| k.to_string()).collect(),
                }
            }
            Err(e) => {
                log::error!("[Fleet] managed configuration is unusable: {}", e);
                ManagedState {
                    error: Some(e),
                    found: true,
                    ..ManagedState::absent(path)
                }
            }
        },
    };

    if let Ok(mut held) = STATE.write() {
        *held = Some(state.clone());
    }
    state
}

/// The held state, loading it on first use so a caller in a code path that runs
/// before startup finished still sees policy.
pub fn state() -> ManagedState {
    if let Ok(held) = STATE.read() {
        if let Some(state) = held.as_ref() {
            return state.clone();
        }
    }
    reload()
}

/// The policy itself, which is what the overlay seams want.
pub fn managed() -> ManagedConfig {
    state().config
}

/// Appends the provenance row to the consent log, so which policy applied at which
/// launch is auditable after the fact.
///
/// Written on every launch rather than only on change: "the policy was still this
/// on Tuesday" is the question an audit actually asks, and the consent log is
/// append-only by design so there is nothing to update.
pub async fn log_provenance(pool: &SqlitePool, state: &ManagedState) {
    let detail = if let Some(error) = &state.error {
        format!(
            "Managed configuration at {} could not be applied ({}); local settings govern",
            state.path, error
        )
    } else if !state.found {
        format!(
            "No managed configuration found at {}; local settings govern",
            state.path
        )
    } else {
        format!("{} (from {})", state.config.describe(), state.path)
    };

    let level = state
        .config
        .consent_level_floor
        .map(|level| level.as_str().to_string())
        .unwrap_or_else(|| "none".to_string());

    if let Err(e) = crate::database::repositories::consent::ConsentEventsRepository::append(
        pool,
        SYSTEM_SUBJECT_ID,
        &level,
        PROVENANCE_EVENT,
        Some(&state.path),
        Some("managed_config"),
        &detail,
    )
    .await
    {
        log::warn!("[Fleet] failed to record managed-config provenance: {}", e);
    }
}

// ---------------------------------------------------------------------------
// The seams other modules call
// ---------------------------------------------------------------------------

/// Refuses an LLM provider the organisation does not permit.
pub fn check_llm_provider(provider: &str) -> Result<(), String> {
    let managed = managed();
    if overlay::provider_allowed(managed.allowed_llm_providers.as_ref(), provider) {
        return Ok(());
    }
    Err(format!(
        "\"{}\" is not one of the model providers your organisation allows ({}).",
        provider,
        managed
            .allowed_llm_providers
            .as_ref()
            .map(|list| if list.is_empty() {
                "none".to_string()
            } else {
                list.join(", ")
            })
            .unwrap_or_default()
    ))
}

/// Refuses a transcription provider the organisation does not permit.
pub fn check_transcription_provider(provider: &str) -> Result<(), String> {
    let managed = managed();
    if overlay::provider_allowed(managed.allowed_transcription_providers.as_ref(), provider) {
        return Ok(());
    }
    Err(format!(
        "\"{}\" is not one of the transcription providers your organisation allows ({}).",
        provider,
        managed
            .allowed_transcription_providers
            .as_ref()
            .map(|list| if list.is_empty() {
                "none".to_string()
            } else {
                list.join(", ")
            })
            .unwrap_or_default()
    ))
}

/// The retention window in force for a profile that asks for `local`.
pub fn retention_days(local: Option<i64>) -> Option<i64> {
    overlay::effective_retention_days(&managed(), local)
}

/// The privacy profile the organisation wants as the workspace default.
pub fn default_privacy_profile() -> Option<String> {
    managed().default_privacy_profile
}

/// Whether update checks are permitted.
pub fn updates_allowed() -> bool {
    overlay::updates_allowed(&managed())
}
