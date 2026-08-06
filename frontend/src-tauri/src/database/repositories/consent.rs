//! Recording-consent persistence: the single settings row, the append-only
//! consent event log, and the session-to-meeting bridge.
//!
//! The log is append-only by contract. There is deliberately no `update` or
//! `delete` on `ConsentEventsRepository`: a correction is a new row, so the
//! record of what the operator actually did at the time survives.

use crate::database::models::{ConsentEvent, ConsentSettingsRow};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct ConsentSettingsRepository;

impl ConsentSettingsRepository {
    /// Reads the settings row. Returns None only if the seed insert in the
    /// migration somehow did not run; callers substitute defaults.
    pub async fn get(pool: &SqlitePool) -> Result<Option<ConsentSettingsRow>, sqlx::Error> {
        sqlx::query_as::<_, ConsentSettingsRow>(
            "SELECT consent_level, per_speaker_enforcement, spoken_announcement_enabled,
                    announcement_text, disclaimer_text, blocked_title_keywords, blocked_domains
             FROM consent_settings WHERE id = 1",
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn save(pool: &SqlitePool, row: &ConsentSettingsRow) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO consent_settings (
                 id, consent_level, per_speaker_enforcement, spoken_announcement_enabled,
                 announcement_text, disclaimer_text, blocked_title_keywords, blocked_domains
             ) VALUES (1, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 consent_level = excluded.consent_level,
                 per_speaker_enforcement = excluded.per_speaker_enforcement,
                 spoken_announcement_enabled = excluded.spoken_announcement_enabled,
                 announcement_text = excluded.announcement_text,
                 disclaimer_text = excluded.disclaimer_text,
                 blocked_title_keywords = excluded.blocked_title_keywords,
                 blocked_domains = excluded.blocked_domains",
        )
        .bind(&row.consent_level)
        .bind(&row.per_speaker_enforcement)
        .bind(row.spoken_announcement_enabled)
        .bind(&row.announcement_text)
        .bind(&row.disclaimer_text)
        .bind(&row.blocked_title_keywords)
        .bind(&row.blocked_domains)
        .execute(pool)
        .await?;
        Ok(())
    }
}

pub struct ConsentEventsRepository;

impl ConsentEventsRepository {
    /// Appends one event. The only write path for `consent_events`.
    #[allow(clippy::too_many_arguments)]
    pub async fn append(
        pool: &SqlitePool,
        meeting_id: &str,
        level: &str,
        event_type: &str,
        subject: Option<&str>,
        method: Option<&str>,
        detail: &str,
    ) -> Result<ConsentEvent, sqlx::Error> {
        let event = ConsentEvent {
            id: format!("consent-{}", Uuid::new_v4()),
            meeting_id: meeting_id.to_string(),
            level: level.to_string(),
            event_type: event_type.to_string(),
            subject: subject.map(str::to_string),
            method: method.map(str::to_string),
            detail: detail.to_string(),
            created_at: Utc::now(),
        };
        sqlx::query(
            "INSERT INTO consent_events
                 (id, meeting_id, level, event_type, subject, method, detail, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&event.id)
        .bind(&event.meeting_id)
        .bind(&event.level)
        .bind(&event.event_type)
        .bind(event.subject.as_deref())
        .bind(event.method.as_deref())
        .bind(&event.detail)
        .bind(event.created_at)
        .execute(pool)
        .await?;
        Ok(event)
    }

    /// Every event for a meeting, oldest first. Includes events logged against
    /// the pre-recording session id that later bound to this meeting.
    pub async fn for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<ConsentEvent>, sqlx::Error> {
        sqlx::query_as::<_, ConsentEvent>(
            "SELECT id, meeting_id, level, event_type, subject, method, detail, created_at
             FROM consent_events
             WHERE meeting_id = ?
                OR meeting_id IN (
                     SELECT session_id FROM consent_session_meetings WHERE meeting_id = ?
                   )
             ORDER BY created_at ASC, id ASC",
        )
        .bind(meeting_id)
        .bind(meeting_id)
        .fetch_all(pool)
        .await
    }

    /// Events in a date range, newest first, optionally narrowed to one
    /// meeting or one client's meetings. Used by the org-wide export.
    pub async fn in_range(
        pool: &SqlitePool,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        meeting_id: Option<&str>,
        client_id: Option<&str>,
    ) -> Result<Vec<ConsentEvent>, sqlx::Error> {
        let mut sql = String::from(
            "SELECT id, meeting_id, level, event_type, subject, method, detail, created_at
             FROM consent_events
             WHERE created_at >= ? AND created_at <= ?",
        );
        if meeting_id.is_some() {
            sql.push_str(
                " AND (meeting_id = ? OR meeting_id IN (
                        SELECT session_id FROM consent_session_meetings WHERE meeting_id = ?
                      ))",
            );
        }
        if client_id.is_some() {
            sql.push_str(
                " AND (meeting_id IN (SELECT meeting_id FROM meeting_clients WHERE client_id = ?)
                       OR meeting_id IN (
                            SELECT s.session_id FROM consent_session_meetings s
                            JOIN meeting_clients mc ON mc.meeting_id = s.meeting_id
                            WHERE mc.client_id = ?
                          ))",
            );
        }
        sql.push_str(" ORDER BY created_at DESC, id ASC");

        let mut query = sqlx::query_as::<_, ConsentEvent>(&sql).bind(from).bind(to);
        if let Some(id) = meeting_id {
            query = query.bind(id).bind(id);
        }
        if let Some(id) = client_id {
            query = query.bind(id).bind(id);
        }
        query.fetch_all(pool).await
    }

    /// Meeting titles for the ids in the log, so the export can name meetings
    /// instead of printing bare ids. Sessions resolve through the bridge.
    pub async fn titles_for_log(
        pool: &SqlitePool,
    ) -> Result<Vec<(String, String)>, sqlx::Error> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT m.id, m.title FROM meetings m
             UNION ALL
             SELECT s.session_id, m.title FROM consent_session_meetings s
             JOIN meetings m ON m.id = s.meeting_id",
        )
        .fetch_all(pool)
        .await
    }
}

pub struct ConsentSessionsRepository;

impl ConsentSessionsRepository {
    /// Binds a pre-recording consent session to the meeting row the recording
    /// produced. Insert-only: the first binding for a session wins, so a
    /// re-save cannot silently repoint an existing consent trail.
    pub async fn bind(
        pool: &SqlitePool,
        session_id: &str,
        meeting_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO consent_session_meetings (session_id, meeting_id, created_at)
             VALUES (?, ?, ?)",
        )
        .bind(session_id)
        .bind(meeting_id)
        .bind(Utc::now())
        .execute(pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
