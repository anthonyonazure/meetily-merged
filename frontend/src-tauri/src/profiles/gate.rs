//! The recording-start enforcement point for privacy profiles.
//!
//! Sits next to the consent gate in `audio::recording_commands`, and for the
//! same reason: five UI paths start a recording and they all converge there, so
//! a check placed there cannot be skipped by picking a different button.
//!
//! Two calls, deliberately split:
//!
//! 1. `check_recording_start` runs BEFORE the consent gate and refuses a cloud
//!    transcription provider the profile does not allow — nothing should be
//!    touched, and no consent collected, for a recording that cannot run.
//! 2. `log_applied` runs AFTER the consent gate, once the consent session id
//!    exists, and appends the `profile_applied` row so the log shows which
//!    policy governed and how it was reached.

use tauri::{AppHandle, Manager, Runtime};

use crate::database::repositories::consent::ConsentEventsRepository;
use crate::state::AppState;

use super::resolver::{self, EffectiveProfile};

/// Event type written into the append-only consent log at recording start.
pub const EVENT_PROFILE_APPLIED: &str = "profile_applied";

/// Resolves the profile for a recording that is about to start and refuses it
/// when the configured transcription provider is not allowed.
///
/// Returns the resolved profile so the caller can hand it to `log_applied`
/// without a second round of database reads.
pub async fn check_recording_start<R: Runtime>(
    app: &AppHandle<R>,
    meeting_title: Option<&str>,
) -> Result<EffectiveProfile, String> {
    let Some(state) = app.try_state::<AppState>() else {
        // No database yet (first launch, before setup). Nothing to resolve, and
        // blocking here would brick recording rather than protect anyone.
        log::warn!("[Profiles] database unavailable at recording start; profile check skipped");
        return Ok(EffectiveProfile::none());
    };
    let pool = state.db_manager.pool();
    let title = meeting_title.unwrap_or("").trim();

    // Attendees the operator already confirmed in the pre-record sheet, when
    // there was one. They make client resolution by email domain possible
    // before a meeting row exists.
    let attendees = crate::consent::gate::pending_attendees();

    let effective = resolver::for_recording(pool, title, &attendees).await;

    if effective.profile.is_some() {
        let provider = configured_transcription_provider(app).await;
        effective.check_transcription(&provider).inspect_err(|reason| {
            log::warn!("[Profiles] recording refused: {}", reason);
        })?;
    }

    Ok(effective)
}

/// Appends the `profile_applied` row. Called after the consent gate so the row
/// lands on the same session id as the rest of that recording's consent trail.
pub async fn log_applied<R: Runtime>(
    app: &AppHandle<R>,
    effective: &EffectiveProfile,
    meeting_title: Option<&str>,
) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let pool = state.db_manager.pool();

    let session_id = match crate::consent::gate::current_session() {
        Some(session) => session.session_id,
        None => crate::consent::gate::new_session_id(),
    };
    let level = effective
        .profile
        .as_ref()
        .map(|p| p.consent_level.as_str())
        .unwrap_or("self_only");
    let title = meeting_title.unwrap_or("").trim();
    let subject = (!title.is_empty()).then_some(title);

    let detail = format!(
        "{}. Transcription: {}. Models: {}. Retention: {}. Masking: {}. Sharing: {}.",
        effective.describe(),
        effective
            .profile
            .as_ref()
            .map(|p| p.transcription_mode.as_str())
            .unwrap_or("unrestricted"),
        effective
            .profile
            .as_ref()
            .map(|p| p.llm_mode.as_str())
            .unwrap_or("unrestricted"),
        match effective.retention_days() {
            Some(days) => format!("{} days", days),
            None => "kept".to_string(),
        },
        if effective.redact_pii() { "on" } else { "off" },
        if effective.allow_sharing() { "allowed" } else { "off" },
    );

    if let Err(e) = ConsentEventsRepository::append(
        pool,
        &session_id,
        level,
        EVENT_PROFILE_APPLIED,
        subject,
        Some(effective.source.as_str()),
        &detail,
    )
    .await
    {
        log::error!("[Profiles] failed to log profile_applied: {}", e);
    } else {
        log::info!("[Profiles] {}", detail);
    }
}

/// The transcription provider the app would actually use, read through the same
/// command the recording path reads it through. Falls back to the packaged
/// local default when the config is missing, matching
/// `transcription::validate_transcription_model_ready`.
async fn configured_transcription_provider<R: Runtime>(app: &AppHandle<R>) -> String {
    match crate::api::api::api_get_transcript_config(app.clone(), app.clone().state(), None).await {
        Ok(Some(config)) => config.provider,
        Ok(None) => "parakeet".to_string(),
        Err(e) => {
            log::warn!(
                "[Profiles] could not read the transcript config ({}); assuming the local default",
                e
            );
            "parakeet".to_string()
        }
    }
}
