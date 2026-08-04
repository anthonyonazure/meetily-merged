//! Tauri commands for the Client Memory feature.

use crate::clients::follow_through::{self, FollowThroughResult};
use crate::clients::suggest::{self, ClientSuggestion};
use crate::database::models::{Client, ClientWithCounts, MemoryFact, MemoryFactWithMeeting};
use crate::database::repositories::{
    chat::{ChatMessagesRepository, ChatScope},
    client::{ClientsRepository, MeetingClientsRepository},
    meeting::MeetingsRepository,
    memory::MemoryFactsRepository,
};
use crate::m365;
use crate::state::AppState;
use log::{info as log_info, warn as log_warn};

/// Cap for memory_search results, to keep the IPC payload bounded.
const MEMORY_SEARCH_LIMIT: i64 = 200;

/// Window around a meeting's creation time used when matching calendar-event
/// attendees to the meeting.
const ATTENDEE_WINDOW_HOURS: i64 = 2;

fn validate_client_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Client name cannot be empty".to_string());
    }
    if name.chars().count() > 200 {
        return Err("Client name is too long".to_string());
    }
    Ok(name.to_string())
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Lists all clients with meeting and open-commitment counts.
#[tauri::command]
pub async fn clients_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ClientWithCounts>, String> {
    ClientsRepository::list_with_counts(state.db_manager.pool())
        .await
        .map_err(|e| format!("Failed to load clients: {}", e))
}

/// Creates a client. `domain` is the email domain used for attendee matching
/// (e.g. "acme.com"); optional.
#[tauri::command]
pub async fn client_create(
    state: tauri::State<'_, AppState>,
    name: String,
    domain: Option<String>,
    notes: Option<String>,
) -> Result<Client, String> {
    let name = validate_client_name(&name)?;
    let domain = normalize_optional(domain);
    let notes = notes.unwrap_or_default();
    ClientsRepository::create(
        state.db_manager.pool(),
        &name,
        domain.as_deref(),
        notes.trim(),
    )
    .await
    .map_err(|e| format!("Failed to create client: {}", e))
}

/// Updates a client's name, domain, and notes.
#[tauri::command]
pub async fn client_update(
    state: tauri::State<'_, AppState>,
    client_id: String,
    name: String,
    domain: Option<String>,
    notes: Option<String>,
) -> Result<Client, String> {
    let name = validate_client_name(&name)?;
    let domain = normalize_optional(domain);
    let notes = notes.unwrap_or_default();
    let pool = state.db_manager.pool();
    let updated = ClientsRepository::update(
        pool,
        &client_id,
        &name,
        domain.as_deref(),
        notes.trim(),
    )
    .await
    .map_err(|e| format!("Failed to update client: {}", e))?;
    if !updated {
        return Err("Client not found".to_string());
    }
    ClientsRepository::get(pool, &client_id)
        .await
        .map_err(|e| format!("Failed to reload client: {}", e))?
        .ok_or_else(|| "Client not found".to_string())
}

/// Deletes a client. Tagged meetings are unlinked but kept, and extracted
/// memory facts stay on their meetings (client link cleared).
#[tauri::command]
pub async fn client_delete(
    state: tauri::State<'_, AppState>,
    client_id: String,
) -> Result<bool, String> {
    log_info!("client_delete called: {}", client_id);
    let pool = state.db_manager.pool();
    MemoryFactsRepository::unlink_client(pool, &client_id)
        .await
        .map_err(|e| format!("Failed to unlink client facts: {}", e))?;
    // The client's chat thread has no home once the client is gone.
    ChatMessagesRepository::clear(pool, &ChatScope::Client(client_id.clone()))
        .await
        .map_err(|e| format!("Failed to clear client chat: {}", e))?;
    ClientsRepository::delete(pool, &client_id)
        .await
        .map_err(|e| format!("Failed to delete client: {}", e))
}

/// Tags a meeting with a client, or clears the tag when `client_id` is null.
/// Returns the meeting's client after the change.
#[tauri::command]
pub async fn meeting_set_client(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    client_id: Option<String>,
) -> Result<Option<Client>, String> {
    let pool = state.db_manager.pool();
    if let Some(id) = client_id.as_deref() {
        if ClientsRepository::get(pool, id)
            .await
            .map_err(|e| format!("Failed to look up client: {}", e))?
            .is_none()
        {
            return Err("Client not found".to_string());
        }
    }
    MeetingClientsRepository::set(pool, &meeting_id, client_id.as_deref())
        .await
        .map_err(|e| format!("Failed to tag meeting: {}", e))?;
    MeetingClientsRepository::client_for_meeting(pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to read meeting client: {}", e))
}

/// The client a meeting is tagged with, if any.
#[tauri::command]
pub async fn meeting_get_client(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Option<Client>, String> {
    MeetingClientsRepository::client_for_meeting(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|e| format!("Failed to read meeting client: {}", e))
}

/// Attendee emails from Microsoft 365 calendar events around the meeting's
/// time, when the integration is connected. Any failure degrades to an empty
/// list — suggestions then rely on title matching alone.
async fn m365_attendee_emails_for(
    meeting_time: chrono::DateTime<chrono::Utc>,
) -> Vec<String> {
    let connected = matches!(m365::auth::read_tokens().await, Ok(Some(_)));
    if !connected {
        return Vec::new();
    }
    let start = meeting_time - chrono::Duration::hours(ATTENDEE_WINDOW_HOURS);
    let end = meeting_time + chrono::Duration::hours(ATTENDEE_WINDOW_HOURS);
    let attempt = async {
        let token = m365::auth::access_token().await?;
        match m365::graph::attendee_emails_between(&token, start, end).await {
            Err(e) if e.contains("HTTP 401") => {
                let token = m365::auth::force_refresh().await?;
                m365::graph::attendee_emails_between(&token, start, end).await
            }
            other => other,
        }
    };
    match attempt.await {
        Ok(emails) => emails,
        Err(e) => {
            log_warn!("Client suggestion: attendee lookup skipped ({})", e);
            Vec::new()
        }
    }
}

/// Suggests a client for an untagged meeting from calendar attendee domains
/// (when M365 is connected) and fuzzy title matching. Returns null when the
/// meeting is already tagged or nothing matches. Suggestion only — the UI
/// asks before tagging; nothing is assigned silently.
#[tauri::command]
pub async fn meeting_suggest_client(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Option<ClientSuggestion>, String> {
    let pool = state.db_manager.pool();

    let already_tagged = MeetingClientsRepository::client_for_meeting(pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to read meeting client: {}", e))?
        .is_some();
    if already_tagged {
        return Ok(None);
    }

    let clients: Vec<Client> = ClientsRepository::list_with_counts(pool)
        .await
        .map_err(|e| format!("Failed to load clients: {}", e))?
        .into_iter()
        .map(|c| Client {
            id: c.id,
            name: c.name,
            domain: c.domain,
            notes: c.notes,
            created_at: c.created_at,
        })
        .collect();
    if clients.is_empty() {
        return Ok(None);
    }

    let meeting = MeetingsRepository::get_meeting_metadata(pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to load meeting: {}", e))?
        .ok_or_else(|| format!("Meeting not found: {}", meeting_id))?;

    let attendee_domains: Vec<String> = m365_attendee_emails_for(meeting.created_at.0)
        .await
        .iter()
        .filter_map(|email| suggest::email_domain(email))
        .collect();

    Ok(suggest::suggest(&clients, &meeting.title, &attendee_domains))
}

/// Everything the client timeline view needs in one round trip.
#[derive(Debug, serde::Serialize)]
pub struct ClientTimeline {
    pub client: Client,
    /// Meetings tagged with this client, newest first.
    pub meetings: Vec<crate::database::models::MeetingModel>,
    /// All memory facts for this client, joined with their meetings.
    pub facts: Vec<MemoryFactWithMeeting>,
}

/// The client's timeline: their meetings (newest first) plus every memory
/// fact, for interleaved rendering on the Clients page.
#[tauri::command]
pub async fn client_timeline(
    state: tauri::State<'_, AppState>,
    client_id: String,
) -> Result<ClientTimeline, String> {
    let pool = state.db_manager.pool();
    let client = ClientsRepository::get(pool, &client_id)
        .await
        .map_err(|e| format!("Failed to load client: {}", e))?
        .ok_or_else(|| "Client not found".to_string())?;
    let meetings = MeetingClientsRepository::meetings_for_client(pool, &client_id)
        .await
        .map_err(|e| format!("Failed to load client meetings: {}", e))?;
    let facts = MemoryFactsRepository::for_client(pool, &client_id)
        .await
        .map_err(|e| format!("Failed to load client memory: {}", e))?;
    Ok(ClientTimeline {
        client,
        meetings,
        facts,
    })
}

/// Runs the follow-through agent for a client from the Clients page: stale
/// open commitments become nudges plus suggested chase messages. Awaited by
/// the frontend (a normal LLM round trip); nothing is persisted or sent.
#[tauri::command]
pub async fn client_follow_through<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
    client_id: String,
    model_provider: String,
    model_name: String,
) -> Result<FollowThroughResult, String> {
    log_info!(
        "client_follow_through called: client={}, provider={}",
        client_id,
        model_provider
    );
    let app_data_dir = tauri::Manager::path(&app).app_data_dir().ok();
    follow_through::run_for_client(
        state.db_manager.pool(),
        &client_id,
        &model_provider,
        &model_name,
        app_data_dir,
    )
    .await
}

/// Count of open commitments older than `older_than_days` days across all
/// clients — powers the subtle once-per-session badge on the Clients nav.
#[tauri::command]
pub async fn memory_stale_open_count(
    state: tauri::State<'_, AppState>,
    older_than_days: Option<i64>,
) -> Result<i64, String> {
    let days = older_than_days.unwrap_or(7).max(0);
    MemoryFactsRepository::stale_open_count(state.db_manager.pool(), days)
        .await
        .map_err(|e| format!("Failed to count stale commitments: {}", e))
}

/// Memory facts extracted for one meeting.
#[tauri::command]
pub async fn memory_facts_for_meeting(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<MemoryFact>, String> {
    MemoryFactsRepository::for_meeting(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|e| format!("Failed to load memory facts: {}", e))
}

/// All memory facts for a client, joined with their meetings (newest meeting
/// first) — the raw material of the client timeline.
#[tauri::command]
pub async fn memory_facts_for_client(
    state: tauri::State<'_, AppState>,
    client_id: String,
) -> Result<Vec<MemoryFactWithMeeting>, String> {
    MemoryFactsRepository::for_client(state.db_manager.pool(), &client_id)
        .await
        .map_err(|e| format!("Failed to load client memory: {}", e))
}

/// Sets a commitment's lifecycle status: open, done, or dismissed.
#[tauri::command]
pub async fn memory_fact_set_status(
    state: tauri::State<'_, AppState>,
    fact_id: String,
    status: String,
) -> Result<bool, String> {
    if !matches!(status.as_str(), "open" | "done" | "dismissed") {
        return Err(format!("Invalid memory fact status: {}", status));
    }
    MemoryFactsRepository::set_status(state.db_manager.pool(), &fact_id, &status)
        .await
        .map_err(|e| format!("Failed to update memory fact: {}", e))
}

/// Deletes a memory fact.
#[tauri::command]
pub async fn memory_fact_delete(
    state: tauri::State<'_, AppState>,
    fact_id: String,
) -> Result<bool, String> {
    MemoryFactsRepository::delete(state.db_manager.pool(), &fact_id)
        .await
        .map_err(|e| format!("Failed to delete memory fact: {}", e))
}

/// Case-insensitive substring search over fact subjects and details, optionally
/// scoped to one client.
#[tauri::command]
pub async fn memory_search(
    state: tauri::State<'_, AppState>,
    query: String,
    client_id: Option<String>,
) -> Result<Vec<MemoryFactWithMeeting>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    MemoryFactsRepository::search(
        state.db_manager.pool(),
        query,
        client_id.as_deref(),
        MEMORY_SEARCH_LIMIT,
    )
    .await
    .map_err(|e| format!("Failed to search memory: {}", e))
}
