//! Tauri command surface for Recording Consent.
//!
//! Everything here is local: SQLite for the log and settings, the OS speech
//! synthesiser for the announcement, and the existing read-only Microsoft Graph
//! calendar call for attendee prefill. No new network endpoints.

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use crate::database::models::ConsentEvent;
use crate::database::repositories::consent::{
    ConsentEventsRepository, ConsentSessionsRepository,
};
use crate::m365;
use crate::state::AppState;

use super::announce;
use super::export;
use super::filter::{self, RedactionState};
use super::gate::{self, Clearance};
use super::rules::{self, ConsentLevel, EnforcementMode};
use super::settings::{self, ConsentSettings};

/// Window around "now" used when asking the calendar who is in this meeting.
const ATTENDEE_WINDOW_MINUTES: i64 = 90;

/// Event types the log accepts. Anything else is rejected so a typo in a caller
/// cannot quietly create a category nobody reads.
const EVENT_TYPES: &[&str] = &[
    "self",
    "notice_given",
    "attendee_confirmed",
    "attendee_declined",
    "speaker_confirmed",
    "speaker_declined",
    "recording_blocked",
    "level_overridden",
];

const METHODS: &[&str] = &[
    "chat_paste",
    "spoken_announcement",
    "verbal",
    "in_person",
    "other",
];

/// Free-text fields in the log are bounded; the log is a record, not a store.
const MAX_TEXT: usize = 2000;

fn bounded(value: &str) -> String {
    value.trim().chars().take(MAX_TEXT).collect()
}

fn optional(value: Option<String>) -> Option<String> {
    value
        .map(|v| bounded(&v))
        .filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Consent settings plus the capabilities the UI needs to render honestly.
#[derive(Debug, Serialize)]
pub struct ConsentSettingsPayload {
    #[serde(flatten)]
    pub settings: ConsentSettings,
    /// False on platforms with no speech path, so the UI can hide the control
    /// rather than offer a Test button that always fails.
    pub spoken_announcement_supported: bool,
}

fn payload(settings: ConsentSettings) -> ConsentSettingsPayload {
    ConsentSettingsPayload {
        settings,
        spoken_announcement_supported: announce::is_supported(),
    }
}

#[tauri::command]
pub async fn consent_get_settings(
    state: State<'_, AppState>,
) -> Result<ConsentSettingsPayload, String> {
    Ok(payload(settings::load(state.db_manager.pool()).await))
}

#[derive(Debug, Deserialize)]
pub struct ConsentSettingsInput {
    pub consent_level: String,
    pub per_speaker_enforcement: String,
    pub spoken_announcement_enabled: bool,
    pub announcement_text: String,
    pub disclaimer_text: String,
    pub blocked_title_keywords: Vec<String>,
    pub blocked_domains: Vec<String>,
}

#[tauri::command]
pub async fn consent_save_settings(
    state: State<'_, AppState>,
    input: ConsentSettingsInput,
) -> Result<ConsentSettingsPayload, String> {
    let resolved = ConsentSettings {
        consent_level: ConsentLevel::parse(&input.consent_level),
        per_speaker_enforcement: EnforcementMode::parse(&input.per_speaker_enforcement),
        spoken_announcement_enabled: input.spoken_announcement_enabled,
        announcement_text: bounded(&input.announcement_text),
        disclaimer_text: bounded(&input.disclaimer_text),
        blocked_title_keywords: input.blocked_title_keywords,
        blocked_domains: input.blocked_domains,
    };
    let pool = state.db_manager.pool();
    settings::save(pool, &resolved).await?;
    Ok(payload(settings::load(pool).await))
}

// ---------------------------------------------------------------------------
// Pre-record plan and clearance
// ---------------------------------------------------------------------------

/// Everything the pre-record sheet needs in one round trip.
#[derive(Debug, Serialize)]
pub struct ConsentPlan {
    /// Id to pass back to `consent_grant_clearance`.
    pub session_id: String,
    pub meeting_title: String,
    pub level: ConsentLevel,
    pub enforcement: EnforcementMode,
    /// Name of the privacy profile that set this level, when one applied. Lets
    /// the sheet say why it is asking for more than the global default does.
    pub profile_name: Option<String>,
    /// True when the operator must act before recording can start.
    pub requires_sheet: bool,
    /// Present when a blocking rule matched; recording is refused until the
    /// operator explicitly overrides.
    pub blocked_reason: Option<String>,
    pub disclaimer_text: String,
    pub announcement_text: String,
    pub spoken_announcement_enabled: bool,
    pub spoken_announcement_supported: bool,
    /// Attendees found on the calendar, for the affirmative checklist.
    pub attendees: Vec<String>,
}

/// Asks what this recording needs before it starts.
///
/// Read-only apart from logging `recording_blocked` when a rule matches: the
/// refusal is the decision worth recording, and recording it here means the log
/// shows the attempt even if the operator gives up rather than overriding.
#[tauri::command]
pub async fn consent_prepare_recording(
    state: State<'_, AppState>,
    meeting_title: String,
    level_override: Option<String>,
    attendees: Option<Vec<String>>,
) -> Result<ConsentPlan, String> {
    let pool = state.db_manager.pool();
    let loaded = settings::load(pool).await;
    let title = bounded(&meeting_title);
    let requested = rules::resolve_level(loaded.consent_level, level_override.as_deref());

    let mut attendee_list: Vec<String> = attendees
        .unwrap_or_default()
        .into_iter()
        .map(|a| bounded(&a))
        .filter(|a| !a.is_empty())
        .collect();

    // The client's privacy profile sets a floor the operator can raise but not
    // drop below; with no profile in force the global default applies. Resolved
    // through the same function the gate uses, so the sheet the operator sees
    // and the gate that runs at start agree.
    let (level, enforcement, effective_profile) =
        crate::profiles::resolver::effective_consent_for_recording(
            pool,
            &title,
            &attendee_list,
            loaded.consent_level,
            loaded.per_speaker_enforcement,
            Some(requested),
        )
        .await;

    // Only the affirmative checklist needs a prefilled roster, and only when
    // the caller did not already supply one.
    if attendee_list.is_empty() && level == ConsentLevel::Affirmative {
        attendee_list = calendar_attendees().await;
    }

    let session_id = gate::new_session_id();
    let block = rules::find_block(
        &title,
        &attendee_list,
        &loaded.blocked_title_keywords,
        &loaded.blocked_domains,
    );

    if let Some(reason) = &block {
        let detail = reason.describe();
        if let Err(e) = ConsentEventsRepository::append(
            pool,
            &session_id,
            level.as_str(),
            "recording_blocked",
            if title.is_empty() { None } else { Some(title.as_str()) },
            None,
            &detail,
        )
        .await
        {
            log::error!("[Consent] failed to log recording_blocked: {}", e);
        }
    }

    Ok(ConsentPlan {
        session_id,
        meeting_title: title,
        level,
        enforcement,
        profile_name: effective_profile.profile_name().map(str::to_string),
        requires_sheet: level.requires_pre_record_sheet() || block.is_some(),
        blocked_reason: block.map(|r| r.describe()),
        disclaimer_text: loaded.disclaimer_text,
        announcement_text: loaded.announcement_text,
        spoken_announcement_enabled: loaded.spoken_announcement_enabled,
        spoken_announcement_supported: announce::is_supported(),
        attendees: attendee_list,
    })
}

/// One attendee's state on the affirmative checklist.
#[derive(Debug, Deserialize)]
pub struct AttendeeDecision {
    pub name: String,
    /// consented | declined | unknown
    pub state: String,
}

/// Records what the operator did in the pre-record sheet and parks the
/// clearance the gate will read when recording starts.
#[tauri::command]
pub async fn consent_grant_clearance(
    state: State<'_, AppState>,
    session_id: String,
    meeting_title: String,
    level: String,
    attendees: Option<Vec<AttendeeDecision>>,
    notice_method: Option<String>,
    override_block: Option<bool>,
    override_reason: Option<String>,
) -> Result<(), String> {
    let pool = state.db_manager.pool();
    let loaded = settings::load(pool).await;
    let session_id = bounded(&session_id);
    if session_id.is_empty() {
        return Err("Missing consent session id".to_string());
    }
    let title = bounded(&meeting_title);
    let resolved = rules::resolve_level(loaded.consent_level, Some(level.as_str()));
    let override_block = override_block.unwrap_or(false);

    // A per-meeting level that differs from the global default is itself a
    // decision worth logging.
    if resolved != loaded.consent_level {
        append(
            pool,
            &session_id,
            resolved.as_str(),
            "level_overridden",
            None,
            None,
            &format!(
                "Level for this meeting set to {} (global default is {})",
                resolved.as_str(),
                loaded.consent_level.as_str()
            ),
        )
        .await;
    }

    match resolved {
        ConsentLevel::Notify => {
            let method = optional(notice_method)
                .filter(|m| METHODS.contains(&m.as_str()))
                .unwrap_or_else(|| "other".to_string());
            append(
                pool,
                &session_id,
                resolved.as_str(),
                "notice_given",
                if title.is_empty() { None } else { Some(title.as_str()) },
                Some(method.as_str()),
                "Operator confirmed notice was given before recording started",
            )
            .await;
        }
        ConsentLevel::Affirmative => {
            let decisions = attendees.unwrap_or_default();
            if decisions.is_empty() {
                return Err(
                    "Add at least one attendee before starting at the affirmative level"
                        .to_string(),
                );
            }
            let mut confirmed = 0usize;
            for decision in &decisions {
                let name = bounded(&decision.name);
                if name.is_empty() {
                    continue;
                }
                let (event_type, detail) = match decision.state.trim() {
                    "consented" => {
                        confirmed += 1;
                        ("attendee_confirmed", "Told about recording, did not object")
                    }
                    "declined" => ("attendee_declined", "Objected to being recorded"),
                    _ => ("attendee_declined", "Not reached; treated as not confirmed"),
                };
                append(
                    pool,
                    &session_id,
                    resolved.as_str(),
                    event_type,
                    Some(name.as_str()),
                    Some("verbal"),
                    detail,
                )
                .await;
            }
            if confirmed == 0 {
                return Err(
                    "No attendee was confirmed, so recording cannot start at this level"
                        .to_string(),
                );
            }
        }
        ConsentLevel::SelfOnly | ConsentLevel::PerSpeaker => {
            let detail = if resolved == ConsentLevel::PerSpeaker {
                "Operator consented for themselves; each identified speaker is confirmed separately"
            } else {
                "Operator consented, no other parties notified"
            };
            append(
                pool,
                &session_id,
                resolved.as_str(),
                "self",
                None,
                None,
                detail,
            )
            .await;
        }
    }

    if override_block {
        let reason = optional(override_reason)
            .unwrap_or_else(|| "Operator overrode a blocking rule for this recording".to_string());
        append(
            pool,
            &session_id,
            resolved.as_str(),
            "level_overridden",
            if title.is_empty() { None } else { Some(title.as_str()) },
            None,
            &reason,
        )
        .await;
    }

    let attendee_ids: Vec<String> = Vec::new();
    gate::park_pending(Clearance {
        session_id,
        meeting_title: title,
        level: resolved,
        enforcement: loaded.per_speaker_enforcement,
        override_confirmed: override_block,
        // The rules already ran in `consent_prepare_recording`; carrying the
        // roster forward would re-block a recording the operator just cleared.
        attendees: attendee_ids,
        granted_at: Utc::now(),
    });

    Ok(())
}

/// The consent session in force for the current or most recent recording, for
/// the while-recording indicator.
#[tauri::command]
pub async fn consent_active_session() -> Result<Option<Clearance>, String> {
    Ok(gate::current_session())
}

/// Binds the active consent session to the meeting row the recording produced.
/// Called once the meeting id exists (recording stop / save).
#[tauri::command]
pub async fn consent_bind_meeting(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<bool, String> {
    let Some(session) = gate::current_session() else {
        return Ok(false);
    };
    ConsentSessionsRepository::bind(state.db_manager.pool(), &session.session_id, &meeting_id)
        .await
        .map_err(|e| format!("Failed to bind consent session: {}", e))
}

// ---------------------------------------------------------------------------
// Log
// ---------------------------------------------------------------------------

async fn append(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    level: &str,
    event_type: &str,
    subject: Option<&str>,
    method: Option<&str>,
    detail: &str,
) {
    if let Err(e) = ConsentEventsRepository::append(
        pool, meeting_id, level, event_type, subject, method, detail,
    )
    .await
    {
        log::error!("[Consent] failed to append {} event: {}", event_type, e);
    }
}

/// Appends one event to the log. Used for decisions made after the pre-record
/// sheet: per-speaker confirmations, and corrections (which are new rows, never
/// edits — the log is append-only).
///
/// `meeting_id` may be omitted, in which case the event lands on the active
/// consent session.
#[tauri::command]
pub async fn consent_record_event(
    state: State<'_, AppState>,
    event_type: String,
    meeting_id: Option<String>,
    level: Option<String>,
    subject: Option<String>,
    method: Option<String>,
    detail: Option<String>,
) -> Result<ConsentEvent, String> {
    let event_type = event_type.trim().to_string();
    if !EVENT_TYPES.contains(&event_type.as_str()) {
        return Err(format!("Unknown consent event type: {}", event_type));
    }

    let session = gate::current_session();
    let target = optional(meeting_id)
        .or_else(|| session.as_ref().map(|s| s.session_id.clone()))
        .ok_or_else(|| "No meeting or active consent session to log against".to_string())?;

    let level = match optional(level) {
        Some(raw) => ConsentLevel::parse(&raw),
        None => match session.as_ref() {
            Some(s) => s.level,
            None => settings::load(state.db_manager.pool()).await.consent_level,
        },
    };

    let method = optional(method).filter(|m| METHODS.contains(&m.as_str()));

    ConsentEventsRepository::append(
        state.db_manager.pool(),
        &target,
        level.as_str(),
        &event_type,
        optional(subject).as_deref(),
        method.as_deref(),
        &optional(detail).unwrap_or_default(),
    )
    .await
    .map_err(|e| format!("Failed to record consent event: {}", e))
}

/// The consent log for one meeting, oldest first.
#[tauri::command]
pub async fn consent_log_for_meeting(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<ConsentEvent>, String> {
    ConsentEventsRepository::for_meeting(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|e| format!("Failed to load consent log: {}", e))
}

/// One speaker in a meeting and where their consent stands.
#[derive(Debug, Serialize)]
pub struct SpeakerConsentStatus {
    pub speaker: String,
    /// consented | declined | unknown
    pub state: String,
    /// True when this label is the operator's own microphone, which is covered
    /// by the operator's own consent at every level.
    pub is_operator: bool,
}

/// Per-speaker consent status for a meeting, driven by the speaker labels the
/// diarization pass actually produced.
#[tauri::command]
pub async fn consent_speakers_for_meeting(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<SpeakerConsentStatus>, String> {
    let pool = state.db_manager.pool();
    let labels = filter::speaker_labels_for_meeting(pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to read speaker labels: {}", e))?;
    let events = ConsentEventsRepository::for_meeting(pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to load consent log: {}", e))?
        .into_iter()
        .map(|e| (e.event_type, e.subject.unwrap_or_default()))
        .collect::<Vec<_>>();
    let decisions = filter::latest_speaker_decisions(&events);

    Ok(labels
        .into_iter()
        .map(|speaker| {
            let is_operator = speaker.eq_ignore_ascii_case(filter::OPERATOR_SPEAKER);
            let state = if is_operator {
                "consented".to_string()
            } else {
                match decisions
                    .iter()
                    .find(|(label, _)| label.eq_ignore_ascii_case(&speaker))
                {
                    Some((_, true)) => "consented".to_string(),
                    Some((_, false)) => "declined".to_string(),
                    None => "unknown".to_string(),
                }
            };
            SpeakerConsentStatus {
                speaker,
                state,
                is_operator,
            }
        })
        .collect())
}

/// Whether and what strict mode is withholding for a meeting. The frontend
/// summary payload builder uses this; chat, agents, and exports apply the same
/// state in Rust.
#[tauri::command]
pub async fn consent_redaction_state(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<RedactionState, String> {
    Ok(filter::state_for_meeting(state.db_manager.pool(), &meeting_id).await)
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ConsentExportResult {
    pub folder: String,
    pub csv_path: String,
    pub markdown_path: String,
    pub events: usize,
}

/// Accepts either a plain date ("2026-08-01") or a full RFC 3339 timestamp.
/// A plain date resolves to the start of that day when `end_of_day` is false
/// and to its last moment when true, so an inclusive range does what the
/// operator means.
fn parse_boundary(raw: &str, end_of_day: bool) -> Result<DateTime<Utc>, String> {
    let raw = raw.trim();
    if let Ok(parsed) = DateTime::parse_from_rfc3339(raw) {
        return Ok(parsed.with_timezone(&Utc));
    }
    let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| format!("Could not read the date \"{}\"", raw))?;
    let time = if end_of_day {
        NaiveTime::from_hms_micro_opt(23, 59, 59, 999_999).expect("valid time")
    } else {
        NaiveTime::from_hms_opt(0, 0, 0).expect("valid time")
    };
    Ok(Utc.from_utc_datetime(&date.and_time(time)))
}

/// Writes the consent log for a date range to a folder the operator picks, as
/// both CSV and Markdown. A cancelled picker reports the literal error
/// "cancelled" so the UI can stay quiet, matching the meeting export.
#[tauri::command]
pub async fn consent_log_export<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    from: String,
    to: String,
    meeting_id: Option<String>,
    client_id: Option<String>,
) -> Result<ConsentExportResult, String> {
    let start = parse_boundary(&from, false)?;
    let end = parse_boundary(&to, true)?;
    if end < start {
        return Err("The end of the range is before its start".to_string());
    }

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

    let pool = state.db_manager.pool();
    let meeting_id = optional(meeting_id);
    let client_id = optional(client_id);
    let events = ConsentEventsRepository::in_range(
        pool,
        start,
        end,
        meeting_id.as_deref(),
        client_id.as_deref(),
    )
    .await
    .map_err(|e| format!("Failed to read the consent log: {}", e))?;

    let titles = ConsentEventsRepository::titles_for_log(pool)
        .await
        .unwrap_or_default();

    let from_label = start.format("%Y-%m-%d").to_string();
    let to_label = end.format("%Y-%m-%d").to_string();
    let stem = format!("consent-log-{}-to-{}", from_label, to_label);

    let csv_path = folder.join(format!("{}.csv", stem));
    let markdown_path = folder.join(format!("{}.md", stem));

    std::fs::write(&csv_path, export::to_csv(&events, &titles))
        .map_err(|e| format!("Failed to write {}: {}", csv_path.display(), e))?;
    std::fs::write(
        &markdown_path,
        export::to_markdown(&events, &titles, &from_label, &to_label),
    )
    .map_err(|e| format!("Failed to write {}: {}", markdown_path.display(), e))?;

    log::info!(
        "[Consent] exported {} event(s) to {}",
        events.len(),
        folder.display()
    );

    Ok(ConsentExportResult {
        folder: folder.to_string_lossy().to_string(),
        csv_path: csv_path.to_string_lossy().to_string(),
        markdown_path: markdown_path.to_string_lossy().to_string(),
        events: events.len(),
    })
}

// ---------------------------------------------------------------------------
// Announcement and attendee prefill
// ---------------------------------------------------------------------------

/// Speaks the announcement through the current output device. `text` defaults to
/// the saved announcement. Runs on a blocking thread: speech synthesis takes
/// seconds and must not stall the async runtime.
#[tauri::command]
pub async fn consent_speak_announcement(
    state: State<'_, AppState>,
    text: Option<String>,
) -> Result<(), String> {
    let spoken = match optional(text) {
        Some(value) => value,
        None => settings::load(state.db_manager.pool()).await.announcement_text,
    };
    tokio::task::spawn_blocking(move || announce::speak(&spoken))
        .await
        .map_err(|e| format!("Speech task failed: {}", e))?
}

/// Attendee email addresses for meetings happening around now, from the
/// calendar the app already reads. Empty when Microsoft 365 is not connected —
/// the affirmative checklist falls back to typed entry.
async fn calendar_attendees() -> Vec<String> {
    if !matches!(m365::auth::read_tokens().await, Ok(Some(_))) {
        return Vec::new();
    }
    let now = Utc::now();
    let start = now - chrono::Duration::minutes(ATTENDEE_WINDOW_MINUTES);
    let end = now + chrono::Duration::minutes(ATTENDEE_WINDOW_MINUTES);
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
            log::warn!("[Consent] attendee prefill skipped ({})", e);
            Vec::new()
        }
    }
}

/// Attendees for the affirmative checklist, from the calendar when available.
#[tauri::command]
pub async fn consent_prefill_attendees() -> Result<Vec<String>, String> {
    Ok(calendar_attendees().await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_dates_become_inclusive_boundaries() {
        let start = parse_boundary("2026-08-01", false).unwrap();
        let end = parse_boundary("2026-08-01", true).unwrap();
        assert_eq!(start.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-08-01 00:00:00");
        assert_eq!(end.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-08-01 23:59:59");
        assert!(end > start);
    }

    #[test]
    fn rfc3339_boundaries_are_taken_as_given() {
        let parsed = parse_boundary("2026-08-01T10:30:00Z", true).unwrap();
        assert_eq!(parsed.format("%H:%M").to_string(), "10:30");
    }

    #[test]
    fn unreadable_dates_are_rejected() {
        assert!(parse_boundary("08/01/2026", false).is_err());
        assert!(parse_boundary("", false).is_err());
    }

    #[test]
    fn free_text_is_trimmed_and_bounded() {
        assert_eq!(bounded("  hello  "), "hello");
        assert_eq!(bounded(&"x".repeat(MAX_TEXT + 50)).chars().count(), MAX_TEXT);
        assert_eq!(optional(Some("   ".to_string())), None);
        assert_eq!(optional(Some(" a ".to_string())).as_deref(), Some("a"));
    }

    #[test]
    fn the_event_type_and_method_vocabularies_are_closed() {
        assert!(EVENT_TYPES.contains(&"speaker_declined"));
        assert!(!EVENT_TYPES.contains(&"speaker_maybe"));
        assert!(METHODS.contains(&"chat_paste"));
        assert!(!METHODS.contains(&"telepathy"));
    }
}
