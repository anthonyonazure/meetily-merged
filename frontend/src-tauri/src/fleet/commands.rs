//! Tauri commands for managed configuration.
//!
//! Read-only from the app's side: policy is written by whatever pushed the file,
//! never by the app. Nothing here reaches the network.

use crate::state::AppState;

use super::ManagedState;

/// The parsed policy, where the app looked for it, and whether it was there.
#[tauri::command]
pub async fn managed_config_get() -> Result<ManagedState, String> {
    Ok(super::state())
}

/// Re-reads the file from disk and re-records provenance, for an administrator who
/// just pushed a change and does not want to make the technician restart.
#[tauri::command]
pub async fn managed_config_reload(
    state: tauri::State<'_, AppState>,
) -> Result<ManagedState, String> {
    let reloaded = super::reload();
    super::log_provenance(state.db_manager.pool(), &reloaded).await;
    Ok(reloaded)
}
