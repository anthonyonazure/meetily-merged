//! The consent gate: the single place a recording is allowed to begin.
//!
//! The gate lives in the Rust start path rather than in the UI on purpose.
//! There are five ways a recording starts (home button, sidebar auto-start,
//! sidebar direct event, tray toggle, and a direct IPC call), and they all
//! converge on `audio::recording_commands`. Gating there means no entry point
//! can skip consent, and the pre-record sheet in the UI becomes a convenience
//! rather than the enforcement.
//!
//! Flow: the UI calls `consent_prepare_recording` to learn what this meeting
//! needs, collects it, then calls `consent_grant_clearance`, which parks a
//! `Clearance`. `enforce` reads that clearance, applies the blocking rules,
//! promotes the clearance to the active consent session, and only then lets the
//! recording proceed.

use chrono::{DateTime, Duration, Utc};
use once_cell::sync::Lazy;
use serde::Serialize;
use sqlx::SqlitePool;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use uuid::Uuid;

use crate::database::repositories::consent::ConsentEventsRepository;
use crate::state::AppState;

use super::rules::{find_block, resolve_level, ConsentLevel, EnforcementMode};
use super::settings;

/// How long a clearance collected in the pre-record sheet stays usable. Long
/// enough to pick devices and read the sheet, short enough that yesterday's
/// confirmation cannot authorise today's recording.
const CLEARANCE_TTL_MINUTES: i64 = 15;

/// Error prefixes the frontend matches on. Kept stable and machine-readable so
/// the UI can open the right sheet instead of parsing prose.
pub const ERR_CONSENT_REQUIRED: &str = "CONSENT_REQUIRED";
pub const ERR_CONSENT_BLOCKED: &str = "CONSENT_BLOCKED";

/// Consent collected for one upcoming (or running) recording.
#[derive(Debug, Clone, Serialize)]
pub struct Clearance {
    /// Id the consent log is keyed by until the meeting row exists.
    pub session_id: String,
    pub meeting_title: String,
    pub level: ConsentLevel,
    pub enforcement: EnforcementMode,
    /// The operator explicitly overrode a blocking rule for this recording.
    pub override_confirmed: bool,
    /// Attendee identifiers (emails or names) the blocking rules were run over.
    pub attendees: Vec<String>,
    pub granted_at: DateTime<Utc>,
}

impl Clearance {
    fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        now.signed_duration_since(self.granted_at) < Duration::minutes(CLEARANCE_TTL_MINUTES)
    }

    /// Titles are compared loosely: the UI mints the title before opening the
    /// sheet and passes the same string to the start command, but a clearance
    /// granted without a title should still apply.
    fn matches_title(&self, title: &str) -> bool {
        let granted = self.meeting_title.trim();
        granted.is_empty() || granted.eq_ignore_ascii_case(title.trim())
    }
}

/// Clearance waiting to be used by the next recording start.
static PENDING: Lazy<Mutex<Option<Clearance>>> = Lazy::new(|| Mutex::new(None));
/// The consent session for the current (or most recent) recording. Kept after
/// the recording stops so the meeting row can be bound to it at save time.
static SESSION: Lazy<Mutex<Option<Clearance>>> = Lazy::new(|| Mutex::new(None));

pub fn new_session_id() -> String {
    format!("consent-session-{}", Uuid::new_v4())
}

pub fn park_pending(clearance: Clearance) {
    if let Ok(mut guard) = PENDING.lock() {
        *guard = Some(clearance);
    }
}

fn read_pending() -> Option<Clearance> {
    PENDING.lock().ok().and_then(|guard| guard.clone())
}

fn clear_pending() {
    if let Ok(mut guard) = PENDING.lock() {
        *guard = None;
    }
}

fn set_session(clearance: Clearance) {
    if let Ok(mut guard) = SESSION.lock() {
        *guard = Some(clearance);
    }
}

/// The consent session in force for the current or most recent recording.
pub fn current_session() -> Option<Clearance> {
    SESSION.lock().ok().and_then(|guard| guard.clone())
}

/// Attendees from the clearance the operator just granted, if any. Privacy
/// profiles use them to work out which client a not-yet-saved recording belongs
/// to, before there is a meeting row to read a tag from.
pub fn pending_attendees() -> Vec<String> {
    read_pending()
        .map(|clearance| clearance.attendees)
        .unwrap_or_default()
}

/// Resolves the level for a recording.
///
/// Precedence: the client's privacy profile sets a floor, the operator's
/// per-meeting choice can raise it but not drop below it, and the global default
/// applies when no profile resolves. The profile resolver is asked rather than
/// re-deriving any of this here, so the gate and the pre-record sheet can never
/// disagree about what this recording needs.
async fn resolve_for(
    pool: &SqlitePool,
    title: &str,
    pending: Option<&Clearance>,
) -> (ConsentLevel, EnforcementMode, Vec<String>, Vec<String>) {
    let settings = settings::load(pool).await;
    let attendees = pending
        .map(|c| c.attendees.clone())
        .unwrap_or_default();
    let requested = pending.map(|c| resolve_level(settings.consent_level, Some(c.level.as_str())));
    let (level, enforcement, _) = crate::profiles::resolver::effective_consent_for_recording(
        pool,
        title,
        &attendees,
        settings.consent_level,
        settings.per_speaker_enforcement,
        requested,
    )
    .await;
    (
        level,
        enforcement,
        settings.blocked_title_keywords,
        settings.blocked_domains,
    )
}

/// Decides whether a recording may start, and records the decision.
///
/// Returns Err when the recording must not begin. The error string starts with
/// `CONSENT_BLOCKED` (a blocking rule matched) or `CONSENT_REQUIRED` (the level
/// needs operator confirmation that has not been collected).
pub async fn enforce<R: Runtime>(
    app: &AppHandle<R>,
    meeting_title: Option<&str>,
) -> Result<(), String> {
    let Some(state) = app.try_state::<AppState>() else {
        // No database yet (first launch, before setup). There is nowhere to
        // read settings from and nowhere to write the log, so blocking here
        // would brick recording rather than protect anyone. Logged as a gap.
        log::warn!("[Consent] database unavailable at recording start; consent gate skipped");
        return Ok(());
    };
    let pool = state.db_manager.pool();
    let title = meeting_title.unwrap_or("").trim().to_string();
    let now = Utc::now();

    let pending = read_pending().filter(|c| c.is_fresh(now) && c.matches_title(&title));
    let (level, enforcement, keywords, domains) =
        resolve_for(pool, &title, pending.as_ref()).await;
    let attendees = pending
        .as_ref()
        .map(|c| c.attendees.clone())
        .unwrap_or_default();

    // 1. Blocking rules run at every level, including self_only.
    if let Some(reason) = find_block(&title, &attendees, &keywords, &domains) {
        let overridden = pending.as_ref().is_some_and(|c| c.override_confirmed);
        if !overridden {
            let session_id = pending
                .as_ref()
                .map(|c| c.session_id.clone())
                .unwrap_or_else(new_session_id);
            let detail = reason.describe();
            let subject = if title.is_empty() {
                None
            } else {
                Some(title.as_str())
            };
            if let Err(e) = ConsentEventsRepository::append(
                pool,
                &session_id,
                level.as_str(),
                "recording_blocked",
                subject,
                None,
                &detail,
            )
            .await
            {
                log::error!("[Consent] failed to log recording_blocked: {}", e);
            }
            log::warn!("[Consent] recording refused: {}", detail);
            clear_pending();
            return Err(format!("{}: {}", ERR_CONSENT_BLOCKED, detail));
        }
    }

    // 2. Levels that require operator action before the first sample is
    //    captured. per_speaker prompts during the meeting, self_only never
    //    prompts, so neither of them lands here.
    if level.requires_pre_record_sheet() && pending.is_none() {
        log::info!(
            "[Consent] level {} needs confirmation before recording starts",
            level.as_str()
        );
        return Err(format!(
            "{}: consent level \"{}\" needs confirmation before recording starts",
            ERR_CONSENT_REQUIRED,
            level.as_str()
        ));
    }

    // 3. Cleared. Promote the clearance (or mint a self-consent one) into the
    //    active consent session so events during the recording land on it.
    let clearance = match pending {
        Some(mut clearance) => {
            clearance.enforcement = enforcement;
            clearance
        }
        None => {
            let session_id = new_session_id();
            let detail = match level {
                ConsentLevel::PerSpeaker => {
                    "Operator consented for themselves; each identified speaker is confirmed separately"
                }
                _ => "Operator consented, no other parties notified",
            };
            if let Err(e) = ConsentEventsRepository::append(
                pool,
                &session_id,
                level.as_str(),
                "self",
                None,
                None,
                detail,
            )
            .await
            {
                log::error!("[Consent] failed to log self-consent: {}", e);
            }
            Clearance {
                session_id,
                meeting_title: title.clone(),
                level,
                enforcement,
                override_confirmed: false,
                attendees,
                granted_at: now,
            }
        }
    };

    log::info!(
        "[Consent] recording cleared at level {} (session {})",
        clearance.level.as_str(),
        clearance.session_id
    );
    set_session(clearance.clone());
    clear_pending();

    // Lets the recording indicator and the per-speaker watcher pick up the
    // level without polling.
    let _ = app.emit("consent-session-started", &clearance);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clearance(title: &str, minutes_ago: i64) -> Clearance {
        Clearance {
            session_id: new_session_id(),
            meeting_title: title.to_string(),
            level: ConsentLevel::Notify,
            enforcement: EnforcementMode::FlagOnly,
            override_confirmed: false,
            attendees: Vec::new(),
            granted_at: Utc::now() - Duration::minutes(minutes_ago),
        }
    }

    #[test]
    fn clearances_expire() {
        let now = Utc::now();
        assert!(clearance("Sync", 1).is_fresh(now));
        assert!(clearance("Sync", CLEARANCE_TTL_MINUTES - 1).is_fresh(now));
        assert!(!clearance("Sync", CLEARANCE_TTL_MINUTES + 1).is_fresh(now));
    }

    #[test]
    fn title_matching_is_loose_but_not_blind() {
        let granted = clearance("Weekly Sync", 0);
        assert!(granted.matches_title("weekly sync"));
        assert!(granted.matches_title("  Weekly Sync  "));
        assert!(!granted.matches_title("HR review"));
        // A clearance granted without a title applies to whatever starts.
        assert!(clearance("", 0).matches_title("anything"));
    }

    #[test]
    fn session_ids_are_unique_and_prefixed() {
        let a = new_session_id();
        let b = new_session_id();
        assert_ne!(a, b);
        assert!(a.starts_with("consent-session-"));
    }
}
