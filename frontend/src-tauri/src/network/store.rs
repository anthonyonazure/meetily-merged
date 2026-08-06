//! Persistence and aggregation for the network log.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

/// One recorded outbound request, as the panel reads it.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct NetworkEventRow {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub session_id: String,
    pub host: String,
    pub url: String,
    pub method: String,
    pub purpose: String,
    pub outcome: String,
    pub bytes_out: i64,
    pub bytes_in: i64,
    pub meeting_id: Option<String>,
    pub profile_name: Option<String>,
    pub carried_audio: bool,
    pub carried_transcript: bool,
    pub detail: String,
}

/// A host with how much traffic went to it.
#[derive(Debug, Clone, Serialize)]
pub struct HostTally {
    pub host: String,
    pub requests: i64,
    pub bytes_out: i64,
    pub bytes_in: i64,
    /// False when the host is not in the app's own inventory of expected hosts.
    pub expected: bool,
    pub on_device: bool,
}

const COLUMNS: &str = "id, created_at, session_id, host, url, method, purpose, outcome, \
                       bytes_out, bytes_in, meeting_id, profile_name, carried_audio, \
                       carried_transcript, detail";

pub struct NetworkEventsStore;

impl NetworkEventsStore {
    #[allow(clippy::too_many_arguments)]
    pub async fn insert(pool: &SqlitePool, row: &NetworkEventRow) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO network_events
                 (id, created_at, session_id, host, url, method, purpose, outcome,
                  bytes_out, bytes_in, meeting_id, profile_name, carried_audio,
                  carried_transcript, detail)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(row.created_at)
        .bind(&row.session_id)
        .bind(&row.host)
        .bind(&row.url)
        .bind(&row.method)
        .bind(&row.purpose)
        .bind(&row.outcome)
        .bind(row.bytes_out)
        .bind(row.bytes_in)
        .bind(row.meeting_id.as_deref())
        .bind(row.profile_name.as_deref())
        .bind(row.carried_audio)
        .bind(row.carried_transcript)
        .bind(&row.detail)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// The most recent events, newest first. `session_id` narrows to this run of
    /// the app.
    pub async fn recent(
        pool: &SqlitePool,
        session_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<NetworkEventRow>, sqlx::Error> {
        match session_id {
            Some(session) => sqlx::query_as::<_, NetworkEventRow>(&format!(
                "SELECT {} FROM network_events WHERE session_id = ?
                 ORDER BY created_at DESC, id DESC LIMIT ?",
                COLUMNS
            ))
            .bind(session)
            .bind(limit)
            .fetch_all(pool)
            .await,
            None => sqlx::query_as::<_, NetworkEventRow>(&format!(
                "SELECT {} FROM network_events ORDER BY created_at DESC, id DESC LIMIT ?",
                COLUMNS
            ))
            .bind(limit)
            .fetch_all(pool)
            .await,
        }
    }

    pub async fn for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<NetworkEventRow>, sqlx::Error> {
        sqlx::query_as::<_, NetworkEventRow>(&format!(
            "SELECT {} FROM network_events WHERE meeting_id = ?
             ORDER BY created_at ASC, id ASC",
            COLUMNS
        ))
        .bind(meeting_id)
        .fetch_all(pool)
        .await
    }

    pub async fn in_range(
        pool: &SqlitePool,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<NetworkEventRow>, sqlx::Error> {
        sqlx::query_as::<_, NetworkEventRow>(&format!(
            "SELECT {} FROM network_events WHERE created_at >= ? AND created_at <= ?
             ORDER BY created_at DESC, id DESC",
            COLUMNS
        ))
        .bind(from)
        .bind(to)
        .fetch_all(pool)
        .await
    }

    /// Per-host totals. `session_id` narrows to this run of the app.
    pub async fn tallies(
        pool: &SqlitePool,
        session_id: Option<&str>,
    ) -> Result<Vec<(String, i64, i64, i64)>, sqlx::Error> {
        let sql = match session_id {
            Some(_) => {
                "SELECT host, COUNT(*), COALESCE(SUM(bytes_out), 0), COALESCE(SUM(bytes_in), 0)
                 FROM network_events WHERE session_id = ? GROUP BY host ORDER BY COUNT(*) DESC"
            }
            None => {
                "SELECT host, COUNT(*), COALESCE(SUM(bytes_out), 0), COALESCE(SUM(bytes_in), 0)
                 FROM network_events GROUP BY host ORDER BY COUNT(*) DESC"
            }
        };
        let mut query = sqlx::query_as::<_, (String, i64, i64, i64)>(sql);
        if let Some(session) = session_id {
            query = query.bind(session);
        }
        query.fetch_all(pool).await
    }

    pub async fn total_count(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM network_events")
            .fetch_one(pool)
            .await?;
        Ok(count)
    }
}
