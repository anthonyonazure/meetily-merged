//! Privacy-profile persistence: the profile rows and the single workspace
//! settings row.
//!
//! Built-in profiles are protected here rather than only in the command layer,
//! so no future caller can delete one by accident.

use crate::database::models::{PrivacyProfileRow, PrivacySettingsRow};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

const COLUMNS: &str = "id, name, description, transcription_mode, llm_mode, consent_level, \
     consent_enforcement, retention_days, redact_pii, allow_sharing, created_at, updated_at, is_builtin";

pub struct PrivacyProfilesRepository;

impl PrivacyProfilesRepository {
    /// All profiles, built-ins first and then alphabetically.
    pub async fn list(pool: &SqlitePool) -> Result<Vec<PrivacyProfileRow>, sqlx::Error> {
        sqlx::query_as::<_, PrivacyProfileRow>(&format!(
            "SELECT {COLUMNS} FROM privacy_profiles
             ORDER BY is_builtin DESC, name COLLATE NOCASE ASC"
        ))
        .fetch_all(pool)
        .await
    }

    pub async fn get(
        pool: &SqlitePool,
        profile_id: &str,
    ) -> Result<Option<PrivacyProfileRow>, sqlx::Error> {
        sqlx::query_as::<_, PrivacyProfileRow>(&format!(
            "SELECT {COLUMNS} FROM privacy_profiles WHERE id = ?"
        ))
        .bind(profile_id)
        .fetch_optional(pool)
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        pool: &SqlitePool,
        name: &str,
        description: &str,
        transcription_mode: &str,
        llm_mode: &str,
        consent_level: &str,
        consent_enforcement: &str,
        retention_days: Option<i64>,
        redact_pii: bool,
        allow_sharing: bool,
    ) -> Result<PrivacyProfileRow, sqlx::Error> {
        let now: DateTime<Utc> = Utc::now();
        let row = PrivacyProfileRow {
            id: format!("profile-{}", Uuid::new_v4()),
            name: name.to_string(),
            description: description.to_string(),
            transcription_mode: transcription_mode.to_string(),
            llm_mode: llm_mode.to_string(),
            consent_level: consent_level.to_string(),
            consent_enforcement: consent_enforcement.to_string(),
            retention_days,
            redact_pii,
            allow_sharing,
            created_at: now,
            updated_at: now,
            is_builtin: false,
        };
        sqlx::query(
            "INSERT INTO privacy_profiles
                 (id, name, description, transcription_mode, llm_mode, consent_level,
                  consent_enforcement, retention_days, redact_pii, allow_sharing,
                  created_at, updated_at, is_builtin)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0)",
        )
        .bind(&row.id)
        .bind(&row.name)
        .bind(&row.description)
        .bind(&row.transcription_mode)
        .bind(&row.llm_mode)
        .bind(&row.consent_level)
        .bind(&row.consent_enforcement)
        .bind(row.retention_days)
        .bind(row.redact_pii)
        .bind(row.allow_sharing)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(pool)
        .await?;
        Ok(row)
    }

    /// Updates every editable field. `is_builtin` is deliberately not editable,
    /// so a built-in stays undeletable however it is renamed.
    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        pool: &SqlitePool,
        profile_id: &str,
        name: &str,
        description: &str,
        transcription_mode: &str,
        llm_mode: &str,
        consent_level: &str,
        consent_enforcement: &str,
        retention_days: Option<i64>,
        redact_pii: bool,
        allow_sharing: bool,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE privacy_profiles SET
                 name = ?, description = ?, transcription_mode = ?, llm_mode = ?,
                 consent_level = ?, consent_enforcement = ?, retention_days = ?,
                 redact_pii = ?, allow_sharing = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(name)
        .bind(description)
        .bind(transcription_mode)
        .bind(llm_mode)
        .bind(consent_level)
        .bind(consent_enforcement)
        .bind(retention_days)
        .bind(redact_pii)
        .bind(allow_sharing)
        .bind(Utc::now())
        .bind(profile_id)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Deletes a custom profile and detaches it from any client and from the
    /// workspace default. Built-in rows are never deleted: the WHERE clause
    /// refuses them even if a caller forgets to check.
    pub async fn delete(pool: &SqlitePool, profile_id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM privacy_profiles WHERE id = ? AND is_builtin = 0")
            .bind(profile_id)
            .execute(pool)
            .await?;
        if result.rows_affected() == 0 {
            return Ok(false);
        }
        sqlx::query("UPDATE clients SET privacy_profile_id = NULL WHERE privacy_profile_id = ?")
            .bind(profile_id)
            .execute(pool)
            .await?;
        sqlx::query(
            "UPDATE privacy_settings SET default_profile_id = NULL
             WHERE id = 1 AND default_profile_id = ?",
        )
        .bind(profile_id)
        .execute(pool)
        .await?;
        Ok(true)
    }

    /// How many clients point at a profile, for the delete confirmation.
    pub async fn client_usage(pool: &SqlitePool, profile_id: &str) -> Result<i64, sqlx::Error> {
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM clients WHERE privacy_profile_id = ?")
                .bind(profile_id)
                .fetch_one(pool)
                .await?;
        Ok(count)
    }
}

pub struct PrivacySettingsRepository;

impl PrivacySettingsRepository {
    pub async fn get(pool: &SqlitePool) -> Result<Option<PrivacySettingsRow>, sqlx::Error> {
        sqlx::query_as::<_, PrivacySettingsRow>(
            "SELECT default_profile_id, retention_dry_run, retention_armed_at, retention_last_run_at
             FROM privacy_settings WHERE id = 1",
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn set_default_profile(
        pool: &SqlitePool,
        profile_id: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO privacy_settings (id, default_profile_id) VALUES (1, ?)
             ON CONFLICT(id) DO UPDATE SET default_profile_id = excluded.default_profile_id",
        )
        .bind(profile_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Writes the dry-run switch. `armed_at` is stamped the first time dry run
    /// is turned off and cleared when it is turned back on, which is what makes
    /// "cannot purge on the first launch after an upgrade" true by
    /// construction: a fresh row has dry run on and no arming timestamp.
    pub async fn set_dry_run(
        pool: &SqlitePool,
        dry_run: bool,
        armed_at: Option<DateTime<Utc>>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO privacy_settings (id, retention_dry_run, retention_armed_at)
             VALUES (1, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 retention_dry_run = excluded.retention_dry_run,
                 retention_armed_at = excluded.retention_armed_at",
        )
        .bind(dry_run)
        .bind(armed_at)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn mark_run(pool: &SqlitePool, at: DateTime<Utc>) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO privacy_settings (id, retention_last_run_at) VALUES (1, ?)
             ON CONFLICT(id) DO UPDATE SET retention_last_run_at = excluded.retention_last_run_at",
        )
        .bind(at)
        .execute(pool)
        .await?;
        Ok(())
    }
}

pub struct ClientProfileRepository;

impl ClientProfileRepository {
    /// Attaches a profile to a client, or clears it with None.
    pub async fn set(
        pool: &SqlitePool,
        client_id: &str,
        profile_id: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("UPDATE clients SET privacy_profile_id = ? WHERE id = ?")
            .bind(profile_id)
            .bind(client_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
