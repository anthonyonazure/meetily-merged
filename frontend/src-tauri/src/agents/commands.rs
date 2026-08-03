//! Tauri commands for the Meeting Agents feature.

use crate::agents::registry::{self, AgentOutputKind};
use crate::agents::runner;
use crate::database::models::{ActionItem, ActionItemWithMeeting, AgentRun};
use crate::database::repositories::agent::{
    ActionItemsRepository, AgentRunsRepository, AgentSettingsRepository,
};
use crate::state::AppState;
use log::info as log_info;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

/// Registry entry merged with the user's saved settings.
#[derive(Debug, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub output_kind: AgentOutputKind,
    pub enabled: bool,
    pub auto_run: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentRunStarted {
    pub run_id: String,
}

async fn agent_info_list(pool: &sqlx::SqlitePool) -> Vec<AgentInfo> {
    let mut agents = Vec::with_capacity(registry::all().len());
    for agent in registry::all() {
        let (enabled, auto_run) = runner::effective_settings(pool, agent).await;
        agents.push(AgentInfo {
            id: agent.id.to_string(),
            name: agent.name.to_string(),
            description: agent.description.to_string(),
            output_kind: agent.output_kind,
            enabled,
            auto_run,
        });
    }
    agents
}

/// Lists the built-in agents with their effective settings.
#[tauri::command]
pub async fn agents_list(state: tauri::State<'_, AppState>) -> Result<Vec<AgentInfo>, String> {
    Ok(agent_info_list(state.db_manager.pool()).await)
}

/// Starts an agent run for a meeting and returns the run id immediately.
/// The run executes in the background; poll `agent_runs_for_meeting`.
#[tauri::command]
pub async fn agent_run<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    agent_id: String,
    model_provider: String,
    model_name: String,
) -> Result<AgentRunStarted, String> {
    log_info!(
        "agent_run called: agent={}, meeting={}, provider={}",
        agent_id,
        meeting_id,
        model_provider
    );
    let pool = state.db_manager.pool().clone();
    let app_data_dir = app.path().app_data_dir().ok();
    let run_id = runner::start_agent_run(
        pool,
        meeting_id,
        agent_id,
        model_provider,
        model_name,
        app_data_dir,
    )
    .await?;
    Ok(AgentRunStarted { run_id })
}

/// Returns all agent runs for a meeting, newest first.
#[tauri::command]
pub async fn agent_runs_for_meeting(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<AgentRun>, String> {
    AgentRunsRepository::runs_for_meeting(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|e| format!("Failed to load agent runs: {}", e))
}

/// Returns the per-agent settings (same shape as `agents_list`).
#[tauri::command]
pub async fn agents_get_settings(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AgentInfo>, String> {
    Ok(agent_info_list(state.db_manager.pool()).await)
}

/// Saves enable/auto-run flags for one agent. Omitted flags keep their
/// current effective value.
#[tauri::command]
pub async fn agents_set_enabled(
    state: tauri::State<'_, AppState>,
    agent_id: String,
    enabled: Option<bool>,
    auto_run: Option<bool>,
) -> Result<AgentInfo, String> {
    let agent = registry::get(&agent_id).ok_or_else(|| format!("Unknown agent: {}", agent_id))?;
    let pool = state.db_manager.pool();

    let (current_enabled, current_auto_run) = runner::effective_settings(pool, agent).await;
    let next_enabled = enabled.unwrap_or(current_enabled);
    let next_auto_run = auto_run.unwrap_or(current_auto_run);

    AgentSettingsRepository::upsert(pool, &agent_id, next_enabled, next_auto_run)
        .await
        .map_err(|e| format!("Failed to save agent settings: {}", e))?;

    Ok(AgentInfo {
        id: agent.id.to_string(),
        name: agent.name.to_string(),
        description: agent.description.to_string(),
        output_kind: agent.output_kind,
        enabled: next_enabled,
        auto_run: next_auto_run,
    })
}

/// Lists action items across all meetings (with meeting titles).
#[tauri::command]
pub async fn actions_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ActionItemWithMeeting>, String> {
    ActionItemsRepository::list_all(state.db_manager.pool())
        .await
        .map_err(|e| format!("Failed to load action items: {}", e))
}

/// Lists action items for one meeting.
#[tauri::command]
pub async fn actions_for_meeting(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<ActionItem>, String> {
    ActionItemsRepository::list_for_meeting(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|e| format!("Failed to load action items: {}", e))
}

/// Marks an action item open or done.
#[tauri::command]
pub async fn action_set_status(
    state: tauri::State<'_, AppState>,
    action_id: String,
    status: String,
) -> Result<bool, String> {
    if status != "open" && status != "done" {
        return Err(format!("Invalid action status: {}", status));
    }
    ActionItemsRepository::set_status(state.db_manager.pool(), &action_id, &status)
        .await
        .map_err(|e| format!("Failed to update action item: {}", e))
}

/// Deletes an action item.
#[tauri::command]
pub async fn action_delete(
    state: tauri::State<'_, AppState>,
    action_id: String,
) -> Result<bool, String> {
    ActionItemsRepository::delete(state.db_manager.pool(), &action_id)
        .await
        .map_err(|e| format!("Failed to delete action item: {}", e))
}
