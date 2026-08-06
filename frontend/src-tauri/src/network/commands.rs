//! Tauri commands for the network activity panel.
//!
//! Read-only, and deliberately so: this panel must not itself add an outbound
//! host. Everything it shows comes from the local database, the in-memory session
//! ring, and the static inventory in `hosts`.

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, Runtime};
use tauri_plugin_dialog::DialogExt;

use crate::state::AppState;

use super::export;
use super::hosts::{self, ExpectedHost};
use super::store::{HostTally, NetworkEventRow, NetworkEventsStore};

/// How many rows the panel loads at a time.
const RECENT_LIMIT: i64 = 500;

#[derive(Debug, Clone, Serialize)]
pub struct NetworkActivity {
    pub session_id: String,
    pub session_events: Vec<NetworkEventRow>,
    pub historical_events: Vec<NetworkEventRow>,
    pub session_tallies: Vec<HostTally>,
    pub all_time_tallies: Vec<HostTally>,
    pub session_request_count: usize,
    pub session_host_count: usize,
    pub total_request_count: i64,
    /// Hosts the app reached that are not in its own inventory. Should be empty;
    /// anything here is worth investigating.
    pub unexpected_hosts: Vec<String>,
    /// The headline line for the panel, written for a reader who is checking a
    /// privacy claim rather than debugging.
    pub headline: String,
    /// The standing caveat: this is the app's own record, not a packet capture.
    pub caveat: String,
}

const CAVEAT: &str = "These are the requests this app recorded itself making. They are not a network capture, and they cannot show traffic from anything else on this machine. To confirm independently, watch this app's traffic with any network monitor and compare it against the expected hosts list below.";

fn tally(rows: Vec<(String, i64, i64, i64)>) -> Vec<HostTally> {
    rows.into_iter()
        .map(|(host, requests, bytes_out, bytes_in)| HostTally {
            expected: hosts::is_expected(&host),
            on_device: hosts::is_on_device(&host),
            host,
            requests,
            bytes_out,
            bytes_in,
        })
        .collect()
}

/// Everything the panel needs, in one call.
#[tauri::command]
pub async fn network_events_recent(
    state: tauri::State<'_, AppState>,
) -> Result<NetworkActivity, String> {
    let pool = state.db_manager.pool();
    let session = super::session_id();

    // The in-memory ring is the source for "this session" so the panel is honest
    // even if a database write failed; the database supplies the history.
    let session_events = super::session_events();
    let historical_events = NetworkEventsStore::recent(pool, None, RECENT_LIMIT)
        .await
        .map_err(|e| format!("Failed to read the network log: {}", e))?;
    let session_tallies = tally(
        NetworkEventsStore::tallies(pool, Some(session))
            .await
            .map_err(|e| format!("Failed to total this session: {}", e))?,
    );
    let all_time_tallies = tally(
        NetworkEventsStore::tallies(pool, None)
            .await
            .map_err(|e| format!("Failed to total the network log: {}", e))?,
    );
    let total_request_count = NetworkEventsStore::total_count(pool)
        .await
        .map_err(|e| format!("Failed to count the network log: {}", e))?;

    let mut session_hosts: Vec<String> = session_events
        .iter()
        .map(|event| event.host.clone())
        .collect();
    session_hosts.sort();
    session_hosts.dedup();

    let unexpected_hosts: Vec<String> = all_time_tallies
        .iter()
        .filter(|entry| !entry.expected)
        .map(|entry| entry.host.clone())
        .collect();

    let headline = if session_events.is_empty() {
        "Nothing has left this machine since the app started. No outbound requests have been made this session.".to_string()
    } else {
        let off_device = session_hosts.iter().filter(|h| !hosts::is_on_device(h)).count();
        format!(
            "This session: {} request(s) to {} host(s), {} of them off this machine.",
            session_events.len(),
            session_hosts.len(),
            off_device
        )
    };

    Ok(NetworkActivity {
        session_id: session.to_string(),
        session_request_count: session_events.len(),
        session_host_count: session_hosts.len(),
        session_events,
        historical_events,
        session_tallies,
        all_time_tallies,
        total_request_count,
        unexpected_hosts,
        headline,
        caveat: CAVEAT.to_string(),
    })
}

/// Whether anything left the device for one meeting, and if so what and why.
#[derive(Debug, Clone, Serialize)]
pub struct MeetingNetworkReport {
    pub meeting_id: String,
    pub events: Vec<NetworkEventRow>,
    pub audio_left_device: bool,
    pub transcript_left_device: bool,
    pub hosts: Vec<String>,
    /// The answer in one sentence.
    pub verdict: String,
}

#[tauri::command]
pub async fn network_events_for_meeting(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<MeetingNetworkReport, String> {
    let events = NetworkEventsStore::for_meeting(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|e| format!("Failed to read the network log for this meeting: {}", e))?;

    // Loopback traffic (a local Ollama or the built-in sidecar) is recorded but
    // does not count as leaving the device, which is the distinction the operator
    // actually cares about.
    let left_device = |event: &NetworkEventRow| !hosts::is_on_device(&event.host);
    let audio_left_device = events
        .iter()
        .any(|event| event.carried_audio && left_device(event));
    let transcript_left_device = events
        .iter()
        .any(|event| event.carried_transcript && left_device(event));

    let mut host_list: Vec<String> = events
        .iter()
        .filter(|event| left_device(event))
        .map(|event| event.host.clone())
        .collect();
    host_list.sort();
    host_list.dedup();

    let verdict = match (audio_left_device, transcript_left_device) {
        (false, false) if events.is_empty() => {
            "No audio or transcript from this meeting was sent off this machine.".to_string()
        }
        (false, false) => format!(
            "No audio or transcript from this meeting was sent off this machine. {} request(s) were recorded, none of them carrying meeting content.",
            events.len()
        ),
        (true, false) => format!(
            "Audio from this meeting was sent off this machine to {}.",
            host_list.join(", ")
        ),
        (false, true) => format!(
            "Transcript or summary text from this meeting was sent off this machine to {}.",
            host_list.join(", ")
        ),
        (true, true) => format!(
            "Both audio and transcript text from this meeting were sent off this machine to {}.",
            host_list.join(", ")
        ),
    };

    Ok(MeetingNetworkReport {
        meeting_id,
        events,
        audio_left_device,
        transcript_left_device,
        hosts: host_list,
        verdict,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ExpectedHostsReport {
    pub hosts: Vec<ExpectedHost>,
    pub note: String,
}

/// The complete inventory of hosts this build can ever contact.
#[tauri::command]
pub async fn network_expected_hosts() -> Result<ExpectedHostsReport, String> {
    Ok(ExpectedHostsReport {
        hosts: hosts::expected(),
        note: "This is the full set of hosts this build can contact, kept by hand in the app's source. Most are only reached if you turn on the feature that needs them. Compare it against your own firewall or DNS log: anything there that is not here is worth asking about.".to_string(),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkExportResult {
    pub events: usize,
    pub csv_path: String,
    pub folder: String,
}

/// Writes the log to a CSV in a folder the operator picks.
#[tauri::command]
pub async fn network_events_export<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    from: Option<String>,
    to: Option<String>,
) -> Result<NetworkExportResult, String> {
    let pool = state.db_manager.pool();
    let rows = match (from.as_deref(), to.as_deref()) {
        (Some(from), Some(to)) => {
            let from: DateTime<Utc> = from
                .parse()
                .map_err(|_| "Start date is not a valid timestamp".to_string())?;
            let to: DateTime<Utc> = to
                .parse()
                .map_err(|_| "End date is not a valid timestamp".to_string())?;
            NetworkEventsStore::in_range(pool, from, to).await
        }
        _ => NetworkEventsStore::recent(pool, None, i64::MAX).await,
    }
    .map_err(|e| format!("Failed to read the network log: {}", e))?;

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |folder| {
        let _ = tx.send(folder);
    });
    let folder = rx
        .await
        .map_err(|_| "Folder picker closed unexpectedly".to_string())?
        .ok_or_else(|| "cancelled".to_string())?
        .into_path()
        .map_err(|e| format!("Invalid destination folder: {}", e))?;

    let csv_path = folder.join(format!(
        "network-activity-{}.csv",
        Utc::now().format("%Y-%m-%d")
    ));
    std::fs::write(&csv_path, export::to_csv(&rows))
        .map_err(|e| format!("Failed to write {}: {}", csv_path.display(), e))?;

    Ok(NetworkExportResult {
        events: rows.len(),
        csv_path: csv_path.display().to_string(),
        folder: folder.display().to_string(),
    })
}
