//! Meeting-type persistence: the per-meeting classification and the
//! type-to-template mapping.

use crate::database::models::{MeetingTypeRow, MeetingTypeTemplateRow};
use chrono::Utc;
use sqlx::SqlitePool;

pub struct MeetingTypesRepository;

impl MeetingTypesRepository {
    pub async fn get(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<MeetingTypeRow>, sqlx::Error> {
        sqlx::query_as::<_, MeetingTypeRow>(
            "SELECT meeting_id, meeting_type, confidence, source, created_at, updated_at
             FROM meeting_types WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
    }

    /// Upserts a classification.
    ///
    /// A manual correction is protected here rather than only in the command
    /// layer: the WHERE clause on the update refuses to let a later model run
    /// overwrite a row a person set, so no future caller can undo a correction by
    /// forgetting to check.
    pub async fn set(
        pool: &SqlitePool,
        meeting_id: &str,
        meeting_type: &str,
        confidence: f64,
        source: &str,
    ) -> Result<(), sqlx::Error> {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO meeting_types
                 (meeting_id, meeting_type, confidence, source, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(meeting_id) DO UPDATE SET
                 meeting_type = excluded.meeting_type,
                 confidence = excluded.confidence,
                 source = excluded.source,
                 updated_at = excluded.updated_at
             WHERE meeting_types.source <> 'manual' OR excluded.source = 'manual'",
        )
        .bind(meeting_id)
        .bind(meeting_type)
        .bind(confidence)
        .bind(source)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn clear(pool: &SqlitePool, meeting_id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM meeting_types WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

pub struct MeetingTypeTemplatesRepository;

impl MeetingTypeTemplatesRepository {
    /// Every mapping, workspace and per-client.
    pub async fn list(pool: &SqlitePool) -> Result<Vec<MeetingTypeTemplateRow>, sqlx::Error> {
        sqlx::query_as::<_, MeetingTypeTemplateRow>(
            "SELECT meeting_type, client_id, template_id FROM meeting_type_templates
             ORDER BY client_id ASC, meeting_type ASC",
        )
        .fetch_all(pool)
        .await
    }

    /// The mappings that could apply to one meeting: the workspace ones plus this
    /// client's.
    pub async fn for_scope(
        pool: &SqlitePool,
        client_id: Option<&str>,
    ) -> Result<Vec<MeetingTypeTemplateRow>, sqlx::Error> {
        sqlx::query_as::<_, MeetingTypeTemplateRow>(
            "SELECT meeting_type, client_id, template_id FROM meeting_type_templates
             WHERE client_id = '' OR client_id = ?",
        )
        .bind(client_id.unwrap_or(""))
        .fetch_all(pool)
        .await
    }

    /// Sets one mapping. An empty `template_id` removes it, so "no mapping" is
    /// expressible without a separate command.
    pub async fn set(
        pool: &SqlitePool,
        meeting_type: &str,
        client_id: &str,
        template_id: &str,
    ) -> Result<(), sqlx::Error> {
        if template_id.trim().is_empty() {
            sqlx::query(
                "DELETE FROM meeting_type_templates WHERE meeting_type = ? AND client_id = ?",
            )
            .bind(meeting_type)
            .bind(client_id)
            .execute(pool)
            .await?;
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO meeting_type_templates (meeting_type, client_id, template_id)
             VALUES (?, ?, ?)
             ON CONFLICT(meeting_type, client_id) DO UPDATE SET
                 template_id = excluded.template_id",
        )
        .bind(meeting_type)
        .bind(client_id)
        .bind(template_id.trim())
        .execute(pool)
        .await?;
        Ok(())
    }
}
