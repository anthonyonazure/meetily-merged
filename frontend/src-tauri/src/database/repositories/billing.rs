//! Billing persistence: the single workspace settings row, per-client rates, and
//! per-meeting overrides.
//!
//! Writes here never invent a rate. `set_client` and `set_default_rate` take
//! `Option<f64>` and store NULL for None, so "clear this rate" and "charge
//! nothing" stay different things all the way down to the column.

use crate::database::models::{
    BillingSettingsRow, ClientBillingRow, MeetingBillingOverrideRow,
};
use sqlx::SqlitePool;

pub struct BillingSettingsRepository;

impl BillingSettingsRepository {
    /// Reads the settings row. None only if the migration seed did not run;
    /// callers substitute defaults.
    pub async fn get(pool: &SqlitePool) -> Result<Option<BillingSettingsRow>, sqlx::Error> {
        sqlx::query_as::<_, BillingSettingsRow>(
            "SELECT default_hourly_rate, currency, rounding_minutes,
                    min_billable_minutes, include_internal
             FROM billing_settings WHERE id = 1",
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn save(
        pool: &SqlitePool,
        default_hourly_rate: Option<f64>,
        currency: &str,
        rounding_minutes: i64,
        min_billable_minutes: i64,
        include_internal: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO billing_settings
                 (id, default_hourly_rate, currency, rounding_minutes,
                  min_billable_minutes, include_internal)
             VALUES (1, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 default_hourly_rate = excluded.default_hourly_rate,
                 currency = excluded.currency,
                 rounding_minutes = excluded.rounding_minutes,
                 min_billable_minutes = excluded.min_billable_minutes,
                 include_internal = excluded.include_internal",
        )
        .bind(default_hourly_rate)
        .bind(currency)
        .bind(rounding_minutes)
        .bind(min_billable_minutes)
        .bind(include_internal)
        .execute(pool)
        .await?;
        Ok(())
    }
}

pub struct ClientBillingRepository;

impl ClientBillingRepository {
    pub async fn get(
        pool: &SqlitePool,
        client_id: &str,
    ) -> Result<Option<ClientBillingRow>, sqlx::Error> {
        sqlx::query_as::<_, ClientBillingRow>(
            "SELECT client_id, hourly_rate, billable FROM client_billing WHERE client_id = ?",
        )
        .bind(client_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn list(pool: &SqlitePool) -> Result<Vec<ClientBillingRow>, sqlx::Error> {
        sqlx::query_as::<_, ClientBillingRow>(
            "SELECT client_id, hourly_rate, billable FROM client_billing",
        )
        .fetch_all(pool)
        .await
    }

    /// Upserts a client's rate and billable flag. `hourly_rate = None` stores
    /// NULL, which means "fall back to the workspace rate".
    pub async fn set(
        pool: &SqlitePool,
        client_id: &str,
        hourly_rate: Option<f64>,
        billable: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO client_billing (client_id, hourly_rate, billable)
             VALUES (?, ?, ?)
             ON CONFLICT(client_id) DO UPDATE SET
                 hourly_rate = excluded.hourly_rate,
                 billable = excluded.billable",
        )
        .bind(client_id)
        .bind(hourly_rate)
        .bind(billable)
        .execute(pool)
        .await?;
        Ok(())
    }
}

pub struct MeetingBillingOverridesRepository;

impl MeetingBillingOverridesRepository {
    pub async fn get(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<MeetingBillingOverrideRow>, sqlx::Error> {
        sqlx::query_as::<_, MeetingBillingOverrideRow>(
            "SELECT meeting_id, billable, minutes_override, note
             FROM meeting_billing_overrides WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
    }

    /// Every override in one read, for the report. Cheaper than a query per row
    /// and small enough to hold: one row per corrected meeting, not per meeting.
    pub async fn all(pool: &SqlitePool) -> Result<Vec<MeetingBillingOverrideRow>, sqlx::Error> {
        sqlx::query_as::<_, MeetingBillingOverrideRow>(
            "SELECT meeting_id, billable, minutes_override, note
             FROM meeting_billing_overrides",
        )
        .fetch_all(pool)
        .await
    }

    pub async fn set(
        pool: &SqlitePool,
        meeting_id: &str,
        billable: Option<bool>,
        minutes_override: Option<i64>,
        note: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO meeting_billing_overrides
                 (meeting_id, billable, minutes_override, note)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(meeting_id) DO UPDATE SET
                 billable = excluded.billable,
                 minutes_override = excluded.minutes_override,
                 note = excluded.note",
        )
        .bind(meeting_id)
        .bind(billable)
        .bind(minutes_override)
        .bind(note)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Removes an override, putting the meeting back on inherited behaviour.
    pub async fn clear(pool: &SqlitePool, meeting_id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM meeting_billing_overrides WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

/// One meeting as the report needs it: identity, date, and its client tag.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BillableMeetingRow {
    pub id: String,
    pub title: String,
    pub created_at: crate::database::models::DateTimeUtc,
    pub client_id: Option<String>,
    pub client_name: Option<String>,
}

pub struct BillingReportRepository;

impl BillingReportRepository {
    /// Every meeting with its client tag joined in, newest first.
    ///
    /// Deliberately unfiltered by date. `meetings.created_at` has been written
    /// both as an RFC3339 string with an offset (`...T12:00:00+00:00`) and as a
    /// naive datetime (`... 12:00:00`) by different write paths over the app's
    /// life, so a `WHERE created_at >= ?` string comparison would silently drop
    /// rows depending on which path saved them. `profiles::retention` reached the
    /// same conclusion and also filters in Rust, on the decoded value.
    pub async fn all_meetings_with_client(
        pool: &SqlitePool,
        client_id: Option<&str>,
    ) -> Result<Vec<BillableMeetingRow>, sqlx::Error> {
        let base = "SELECT m.id, m.title, m.created_at,
                           mc.client_id AS client_id, c.name AS client_name
                    FROM meetings m
                    LEFT JOIN meeting_clients mc ON mc.meeting_id = m.id
                    LEFT JOIN clients c ON c.id = mc.client_id";
        match client_id {
            Some(id) => {
                sqlx::query_as::<_, BillableMeetingRow>(&format!(
                    "{base} WHERE mc.client_id = ? ORDER BY m.created_at DESC"
                ))
                .bind(id)
                .fetch_all(pool)
                .await
            }
            None => {
                sqlx::query_as::<_, BillableMeetingRow>(&format!(
                    "{base} ORDER BY m.created_at DESC"
                ))
                .fetch_all(pool)
                .await
            }
        }
    }
}
