use crate::database::models::ChatMessageRecord;
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct ChatMessagesRepository;

impl ChatMessagesRepository {
    /// Inserts one chat message and returns the stored record.
    /// `meeting_id = None` addresses the "all meetings" scope.
    pub async fn insert(
        pool: &SqlitePool,
        meeting_id: Option<&str>,
        role: &str,
        content: &str,
    ) -> Result<ChatMessageRecord, sqlx::Error> {
        let record = ChatMessageRecord {
            id: format!("chatmsg-{}", Uuid::new_v4()),
            meeting_id: meeting_id.map(str::to_string),
            role: role.to_string(),
            content: content.to_string(),
            created_at: Utc::now(),
        };
        sqlx::query(
            "INSERT INTO chat_messages (id, meeting_id, role, content, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&record.id)
        .bind(record.meeting_id.as_deref())
        .bind(&record.role)
        .bind(&record.content)
        .bind(record.created_at)
        .execute(pool)
        .await?;
        Ok(record)
    }

    /// Full history for a scope, oldest first. `meeting_id = None` returns the
    /// all-meetings thread (rows whose meeting_id IS NULL), not every row.
    pub async fn history(
        pool: &SqlitePool,
        meeting_id: Option<&str>,
    ) -> Result<Vec<ChatMessageRecord>, sqlx::Error> {
        match meeting_id {
            Some(id) => {
                sqlx::query_as::<_, ChatMessageRecord>(
                    "SELECT id, meeting_id, role, content, created_at
                     FROM chat_messages WHERE meeting_id = ? ORDER BY created_at ASC, id ASC",
                )
                .bind(id)
                .fetch_all(pool)
                .await
            }
            None => {
                sqlx::query_as::<_, ChatMessageRecord>(
                    "SELECT id, meeting_id, role, content, created_at
                     FROM chat_messages WHERE meeting_id IS NULL ORDER BY created_at ASC, id ASC",
                )
                .fetch_all(pool)
                .await
            }
        }
    }

    /// Deletes the history for a scope; returns the number of removed rows.
    pub async fn clear(
        pool: &SqlitePool,
        meeting_id: Option<&str>,
    ) -> Result<u64, sqlx::Error> {
        let result = match meeting_id {
            Some(id) => {
                sqlx::query("DELETE FROM chat_messages WHERE meeting_id = ?")
                    .bind(id)
                    .execute(pool)
                    .await?
            }
            None => {
                sqlx::query("DELETE FROM chat_messages WHERE meeting_id IS NULL")
                    .execute(pool)
                    .await?
            }
        };
        Ok(result.rows_affected())
    }
}
