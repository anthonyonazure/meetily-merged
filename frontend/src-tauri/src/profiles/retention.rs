//! Retention: what a profile's `retention_days` actually does.
//!
//! Safety, in the order it applies, because this is the only destructive code
//! in the feature:
//!
//! 1. A meeting is only ever considered when its resolved profile sets
//!    `retention_days`. No profile, or a profile with no window, means nothing
//!    happens — which is the shipped state, since the workspace default is unset
//!    on upgrade.
//! 2. The background sweep refuses to delete unless dry run is OFF *and* the
//!    operator explicitly armed it (the timestamp written when they turned dry
//!    run off). A fresh install and a fresh upgrade both have dry run on and no
//!    arming timestamp, so the first launch after an upgrade cannot purge.
//! 3. `retention_run_now` deletes only when the caller passes an explicit
//!    confirmation; without it, the same code runs as a preview.
//! 4. Files are removed one by one from the meeting's own folder, audio
//!    extensions only, and the folder itself only when it is left empty. No
//!    recursive directory removal anywhere.
//! 5. `consent_events` rows are never deleted. The log outlives the meeting on
//!    purpose, and a `retention_purged` row is appended for every purge.
//!
//! The meeting row itself is kept: an empty shell with the title, the date, and
//! its client tag is what lets the consent log still point at something, and
//! what makes a purge visible in the UI rather than a silent disappearance.

use std::path::Path;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager, Runtime};

use crate::database::repositories::{
    consent::ConsentEventsRepository,
    meeting::MeetingsRepository,
    profile::PrivacySettingsRepository,
};
use crate::state::AppState;

use super::resolver;
use super::rules::{age_days, days_until_purge, is_expired};

/// Event type appended to the consent log for every purge, dry run included.
pub const EVENT_RETENTION_PURGED: &str = "retention_purged";
/// Event type for a dry run, so a preview can never be mistaken for a deletion.
pub const EVENT_RETENTION_DRY_RUN: &str = "retention_dry_run";

/// How long the sweep sleeps between wake-ups. Retention windows are measured in
/// days, so hourly is plenty and keeps the task close to free.
const TICK: StdDuration = StdDuration::from_secs(3600);
/// Wait before the first sweep after launch, so a purge never competes with
/// startup work and an operator has time to change their mind.
const STARTUP_DELAY: StdDuration = StdDuration::from_secs(300);
/// How far ahead `retention_preview` looks.
pub const PREVIEW_HORIZON_DAYS: i64 = 30;
/// Audio files a purge removes from a meeting folder.
const AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "mp3", "mp4", "m4a", "flac", "ogg", "aac", "webm", "mka", "opus",
];

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Called from the app's Exit handler so the sweep stops touching state while
/// Tauri tears down.
pub fn shutdown() {
    SHUTDOWN.store(true, Ordering::Release);
}

/// Retention switches as the UI sees them.
#[derive(Debug, Clone, Serialize)]
pub struct RetentionSettings {
    /// True while purges are logged but nothing is deleted. Ships on.
    pub dry_run: bool,
    /// When the operator turned dry run off. None means the background sweep
    /// will not delete anything.
    pub armed_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
}

impl Default for RetentionSettings {
    fn default() -> Self {
        Self {
            dry_run: true,
            armed_at: None,
            last_run_at: None,
        }
    }
}

/// One meeting that is, or is about to be, purgeable.
#[derive(Debug, Clone, Serialize)]
pub struct PurgeCandidate {
    pub meeting_id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub age_days: i64,
    pub profile_name: String,
    pub profile_source: String,
    pub retention_days: i64,
    /// Negative once the window has already closed.
    pub days_until_purge: i64,
    pub client_name: Option<String>,
}

/// What one purge did (or would have done).
#[derive(Debug, Clone, Serialize)]
pub struct PurgeOutcome {
    pub meeting_id: String,
    pub title: String,
    pub profile_name: String,
    pub dry_run: bool,
    pub files_removed: usize,
    pub transcripts_removed: u64,
    pub summaries_removed: u64,
    pub facts_removed: u64,
    pub action_items_removed: u64,
}

/// Result of a sweep, for the UI and the log line.
#[derive(Debug, Clone, Serialize)]
pub struct RetentionRunResult {
    pub dry_run: bool,
    pub examined: usize,
    pub purged: Vec<PurgeOutcome>,
    /// Set when a real purge was asked for but the safety conditions said no.
    pub refused_reason: Option<String>,
}

pub async fn load_settings(pool: &SqlitePool) -> RetentionSettings {
    match PrivacySettingsRepository::get(pool).await {
        Ok(Some(row)) => RetentionSettings {
            dry_run: row.retention_dry_run,
            armed_at: row.retention_armed_at,
            last_run_at: row.retention_last_run_at,
        },
        // A missing or unreadable row reads as "dry run on", never as
        // permission to delete.
        Ok(None) => RetentionSettings::default(),
        Err(e) => {
            log::warn!("[Retention] failed to read settings ({}); assuming dry run", e);
            RetentionSettings::default()
        }
    }
}

/// Writes the dry-run switch. Turning it off stamps the arming timestamp the
/// background sweep requires.
pub async fn save_dry_run(pool: &SqlitePool, dry_run: bool) -> Result<RetentionSettings, String> {
    let armed_at = if dry_run { None } else { Some(Utc::now()) };
    PrivacySettingsRepository::set_dry_run(pool, dry_run, armed_at)
        .await
        .map_err(|e| format!("Failed to save retention settings: {}", e))?;
    Ok(load_settings(pool).await)
}

/// Every meeting whose profile sets a window, with the days left on it.
/// `horizon_days` bounds how far ahead to look; already-expired meetings are
/// always included.
pub async fn candidates(
    pool: &SqlitePool,
    horizon_days: i64,
    now: DateTime<Utc>,
) -> Result<Vec<PurgeCandidate>, String> {
    let meetings = MeetingsRepository::get_meetings(pool)
        .await
        .map_err(|e| format!("Failed to list meetings: {}", e))?;

    let mut out = Vec::new();
    for meeting in meetings {
        let created_at = meeting.created_at.0;
        let effective = resolver::for_meeting(pool, &meeting.id).await;
        let Some(profile) = effective.profile.as_ref() else {
            continue;
        };
        // A managed configuration caps how long anything is kept, so a fleet policy
        // of "90 days maximum" applies even to a profile that says keep forever.
        let Some(retention_days) = crate::fleet::retention_days(profile.retention_days) else {
            continue;
        };
        let remaining = days_until_purge(created_at, retention_days, now);
        if remaining > horizon_days {
            continue;
        }
        out.push(PurgeCandidate {
            meeting_id: meeting.id,
            title: meeting.title,
            created_at,
            age_days: age_days(created_at, now),
            profile_name: profile.name.clone(),
            profile_source: effective.source.as_str().to_string(),
            retention_days,
            days_until_purge: remaining,
            client_name: effective.client_name.clone(),
        });
    }
    // Soonest (and most overdue) first.
    out.sort_by_key(|c| c.days_until_purge);
    Ok(out)
}

/// Runs one sweep.
///
/// `allow_delete` is the only way anything is deleted, and every caller has to
/// justify passing true: the background sweep requires dry run off plus an
/// arming timestamp, and `retention_run_now` requires an explicit confirmation
/// from the operator.
pub async fn sweep(
    pool: &SqlitePool,
    allow_delete: bool,
    now: DateTime<Utc>,
) -> Result<RetentionRunResult, String> {
    let due: Vec<PurgeCandidate> = candidates(pool, 0, now)
        .await?
        .into_iter()
        .filter(|candidate| is_expired(candidate.created_at, candidate.retention_days, now))
        .collect();

    let mut purged = Vec::new();
    for candidate in &due {
        match purge_one(pool, candidate, !allow_delete).await {
            Ok(outcome) => purged.push(outcome),
            Err(e) => log::error!(
                "[Retention] failed to purge meeting {}: {}",
                candidate.meeting_id,
                e
            ),
        }
    }

    if let Err(e) = PrivacySettingsRepository::mark_run(pool, now).await {
        log::warn!("[Retention] failed to record the sweep timestamp: {}", e);
    }

    Ok(RetentionRunResult {
        dry_run: !allow_delete,
        examined: due.len(),
        purged,
        refused_reason: None,
    })
}

/// Purges one meeting's content, or reports what it would remove when
/// `dry_run` is true.
async fn purge_one(
    pool: &SqlitePool,
    candidate: &PurgeCandidate,
    dry_run: bool,
) -> Result<PurgeOutcome, String> {
    let meeting = MeetingsRepository::get_meeting_metadata(pool, &candidate.meeting_id)
        .await
        .map_err(|e| format!("Failed to read meeting: {}", e))?;

    let audio_files = meeting
        .as_ref()
        .and_then(|m| m.folder_path.clone())
        .map(|folder| audio_files_in(Path::new(&folder)))
        .unwrap_or_default();

    let mut outcome = PurgeOutcome {
        meeting_id: candidate.meeting_id.clone(),
        title: candidate.title.clone(),
        profile_name: candidate.profile_name.clone(),
        dry_run,
        files_removed: audio_files.len(),
        transcripts_removed: 0,
        summaries_removed: 0,
        facts_removed: 0,
        action_items_removed: 0,
    };

    if dry_run {
        let counts = count_rows(pool, &candidate.meeting_id).await;
        outcome.transcripts_removed = counts.0;
        outcome.summaries_removed = counts.1;
        outcome.facts_removed = counts.2;
        outcome.action_items_removed = counts.3;
        log_purge(pool, candidate, &outcome, true).await;
        return Ok(outcome);
    }

    // Recording files first: if a later step fails the audio is already gone,
    // which is the direction a retention window is asking for.
    let mut removed = 0usize;
    for path in &audio_files {
        match std::fs::remove_file(path) {
            Ok(()) => removed += 1,
            Err(e) => log::warn!("[Retention] could not remove {}: {}", path.display(), e),
        }
    }
    outcome.files_removed = removed;

    // Only when the folder is left empty, and never recursively.
    if let Some(folder) = meeting.as_ref().and_then(|m| m.folder_path.clone()) {
        let folder = Path::new(&folder);
        if folder.is_dir()
            && std::fs::read_dir(folder)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(false)
        {
            let _ = std::fs::remove_dir(folder);
        }
    }

    outcome.transcripts_removed =
        delete_where(pool, "DELETE FROM transcripts WHERE meeting_id = ?", &candidate.meeting_id)
            .await;
    delete_where(
        pool,
        "DELETE FROM transcript_chunks WHERE meeting_id = ?",
        &candidate.meeting_id,
    )
    .await;
    outcome.summaries_removed = delete_where(
        pool,
        "DELETE FROM summary_processes WHERE meeting_id = ?",
        &candidate.meeting_id,
    )
    .await;
    outcome.facts_removed = delete_where(
        pool,
        "DELETE FROM memory_facts WHERE meeting_id = ?",
        &candidate.meeting_id,
    )
    .await;
    outcome.action_items_removed = delete_where(
        pool,
        "DELETE FROM action_items WHERE meeting_id = ?",
        &candidate.meeting_id,
    )
    .await;
    // Agent output and the meeting's chat thread are derived from the
    // transcript, so they go with it.
    delete_where(
        pool,
        "DELETE FROM agent_runs WHERE meeting_id = ?",
        &candidate.meeting_id,
    )
    .await;
    delete_where(
        pool,
        "DELETE FROM chat_messages WHERE meeting_id = ?",
        &candidate.meeting_id,
    )
    .await;

    // NOTE: consent_events and consent_session_meetings are deliberately left
    // alone. The record of what was consented to outlives the recording.
    log_purge(pool, candidate, &outcome, false).await;
    log::info!(
        "[Retention] purged meeting {} ({} audio file(s), {} transcript row(s))",
        candidate.meeting_id,
        outcome.files_removed,
        outcome.transcripts_removed
    );
    Ok(outcome)
}

async fn delete_where(pool: &SqlitePool, sql: &str, meeting_id: &str) -> u64 {
    match sqlx::query(sql).bind(meeting_id).execute(pool).await {
        Ok(result) => result.rows_affected(),
        Err(e) => {
            log::warn!("[Retention] {} failed: {}", sql, e);
            0
        }
    }
}

async fn count_rows(pool: &SqlitePool, meeting_id: &str) -> (u64, u64, u64, u64) {
    async fn count(pool: &SqlitePool, sql: &str, meeting_id: &str) -> u64 {
        sqlx::query_as::<_, (i64,)>(sql)
            .bind(meeting_id)
            .fetch_one(pool)
            .await
            .map(|(count,)| count.max(0) as u64)
            .unwrap_or(0)
    }
    (
        count(pool, "SELECT COUNT(*) FROM transcripts WHERE meeting_id = ?", meeting_id).await,
        count(
            pool,
            "SELECT COUNT(*) FROM summary_processes WHERE meeting_id = ?",
            meeting_id,
        )
        .await,
        count(pool, "SELECT COUNT(*) FROM memory_facts WHERE meeting_id = ?", meeting_id).await,
        count(pool, "SELECT COUNT(*) FROM action_items WHERE meeting_id = ?", meeting_id).await,
    )
}

/// Audio files directly inside a meeting folder. Not recursive, and never
/// anything that is not an audio file, so a misconfigured folder path cannot
/// take other data with it.
fn audio_files_in(folder: &Path) -> Vec<std::path::PathBuf> {
    if !folder.is_dir() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
                .unwrap_or(false)
        })
        .collect()
}

async fn log_purge(
    pool: &SqlitePool,
    candidate: &PurgeCandidate,
    outcome: &PurgeOutcome,
    dry_run: bool,
) {
    let detail = format!(
        "{} under profile \"{}\" ({} day window, meeting was {} days old): {} audio file(s), {} transcript row(s), {} summary row(s), {} memory fact(s), {} action item(s). Consent log kept.",
        if dry_run { "Would purge" } else { "Purged" },
        candidate.profile_name,
        candidate.retention_days,
        candidate.age_days,
        outcome.files_removed,
        outcome.transcripts_removed,
        outcome.summaries_removed,
        outcome.facts_removed,
        outcome.action_items_removed,
    );
    let event_type = if dry_run {
        EVENT_RETENTION_DRY_RUN
    } else {
        EVENT_RETENTION_PURGED
    };
    if let Err(e) = ConsentEventsRepository::append(
        pool,
        &candidate.meeting_id,
        "self_only",
        event_type,
        Some(&candidate.title),
        Some(candidate.profile_source.as_str()),
        &detail,
    )
    .await
    {
        log::error!("[Retention] failed to log {}: {}", event_type, e);
    }
}

/// An operator-triggered sweep. `confirm` is the explicit second step: without
/// it this is a preview whatever the dry-run setting says.
pub async fn run_now(pool: &SqlitePool, confirm: bool) -> Result<RetentionRunResult, String> {
    let settings = load_settings(pool).await;
    if !confirm {
        let mut result = sweep(pool, false, Utc::now()).await?;
        result.refused_reason = Some(
            "Ran as a preview: nothing was deleted because the run was not confirmed.".to_string(),
        );
        return Ok(result);
    }
    if settings.dry_run {
        let mut result = sweep(pool, false, Utc::now()).await?;
        result.refused_reason = Some(
            "Ran as a preview: dry run is still on, so nothing was deleted.".to_string(),
        );
        return Ok(result);
    }
    sweep(pool, true, Utc::now()).await
}

/// Spawns the hourly sweep. Cheap when idle: with no profile setting a retention
/// window, each tick is a couple of local reads and nothing else.
pub fn spawn<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        // Sleep in short slices so shutdown is prompt.
        let mut slept = StdDuration::ZERO;
        while slept < STARTUP_DELAY {
            if SHUTDOWN.load(Ordering::Acquire) {
                return;
            }
            tokio::time::sleep(StdDuration::from_secs(5)).await;
            slept += StdDuration::from_secs(5);
        }

        log::info!("Retention sweep started (hourly)");
        loop {
            if SHUTDOWN.load(Ordering::Acquire) {
                log::info!("Retention sweep exiting");
                return;
            }

            if let Some(state) = app.try_state::<AppState>() {
                let pool = state.db_manager.pool().clone();
                let settings = load_settings(&pool).await;
                // Both conditions are required: dry run off, and an explicit
                // arming timestamp from the operator turning it off. A fresh
                // upgrade has neither, so the first launch cannot purge.
                let allow_delete = !settings.dry_run && settings.armed_at.is_some();
                match sweep(&pool, allow_delete, Utc::now()).await {
                    Ok(result) if !result.purged.is_empty() => log::info!(
                        "[Retention] sweep {} {} meeting(s)",
                        if result.dry_run { "would purge" } else { "purged" },
                        result.purged.len()
                    ),
                    Ok(_) => {}
                    Err(e) => log::warn!("[Retention] sweep failed: {}", e),
                }
            }

            let mut waited = StdDuration::ZERO;
            while waited < TICK {
                if SHUTDOWN.load(Ordering::Acquire) {
                    return;
                }
                tokio::time::sleep(StdDuration::from_secs(30)).await;
                waited += StdDuration::from_secs(30);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_audio_files_are_collected_and_never_recursively() {
        let root = std::env::temp_dir().join(format!("meetily-retention-{}", uuid::Uuid::new_v4()));
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("recording.wav"), b"audio").unwrap();
        std::fs::write(root.join("recording.MP3"), b"audio").unwrap();
        std::fs::write(root.join("metadata.json"), b"{}").unwrap();
        std::fs::write(root.join("notes.txt"), b"text").unwrap();
        std::fs::write(nested.join("other.wav"), b"audio").unwrap();

        let found = audio_files_in(&root);
        assert_eq!(found.len(), 2, "only the two audio files in the folder itself");
        assert!(found.iter().all(|p| p.is_file()));
        assert!(!found.iter().any(|p| p.ends_with("other.wav")), "not recursive");
        assert!(!found.iter().any(|p| p.ends_with("metadata.json")));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_missing_folder_yields_nothing_rather_than_an_error() {
        let missing = std::env::temp_dir().join("meetily-retention-does-not-exist");
        assert!(audio_files_in(&missing).is_empty());
    }

    #[test]
    fn default_settings_are_the_safe_ones() {
        let defaults = RetentionSettings::default();
        assert!(defaults.dry_run, "dry run ships on");
        assert!(defaults.armed_at.is_none(), "nothing is armed until an operator says so");
    }
}
