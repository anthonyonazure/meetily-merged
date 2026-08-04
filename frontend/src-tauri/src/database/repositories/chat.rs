use crate::database::models::ChatMessageRecord;
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Which conversation thread a chat operation addresses. Exactly one shape:
/// a meeting thread, a client thread, or the shared "all meetings" thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatScope {
    All,
    Meeting(String),
    Client(String),
}

impl ChatScope {
    /// Builds a scope from the optional ids a command receives. A client id
    /// wins over a meeting id (the frontend only ever sends one).
    pub fn from_ids(meeting_id: Option<String>, client_id: Option<String>) -> Self {
        match (client_id, meeting_id) {
            (Some(client), _) => ChatScope::Client(client),
            (None, Some(meeting)) => ChatScope::Meeting(meeting),
            (None, None) => ChatScope::All,
        }
    }

    pub fn meeting_id(&self) -> Option<&str> {
        match self {
            ChatScope::Meeting(id) => Some(id),
            _ => None,
        }
    }

    pub fn client_id(&self) -> Option<&str> {
        match self {
            ChatScope::Client(id) => Some(id),
            _ => None,
        }
    }

    /// Human-readable label for logs.
    pub fn label(&self) -> String {
        match self {
            ChatScope::All => "all-meetings".to_string(),
            ChatScope::Meeting(id) => format!("meeting:{}", id),
            ChatScope::Client(id) => format!("client:{}", id),
        }
    }
}

pub struct ChatMessagesRepository;

impl ChatMessagesRepository {
    /// Inserts one chat message into a scope and returns the stored record.
    pub async fn insert(
        pool: &SqlitePool,
        scope: &ChatScope,
        role: &str,
        content: &str,
    ) -> Result<ChatMessageRecord, sqlx::Error> {
        let record = ChatMessageRecord {
            id: format!("chatmsg-{}", Uuid::new_v4()),
            meeting_id: scope.meeting_id().map(str::to_string),
            client_id: scope.client_id().map(str::to_string),
            role: role.to_string(),
            content: content.to_string(),
            created_at: Utc::now(),
        };
        sqlx::query(
            "INSERT INTO chat_messages (id, meeting_id, client_id, role, content, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&record.id)
        .bind(record.meeting_id.as_deref())
        .bind(record.client_id.as_deref())
        .bind(&record.role)
        .bind(&record.content)
        .bind(record.created_at)
        .execute(pool)
        .await?;
        Ok(record)
    }

    /// Full history for a scope, oldest first.
    pub async fn history(
        pool: &SqlitePool,
        scope: &ChatScope,
    ) -> Result<Vec<ChatMessageRecord>, sqlx::Error> {
        const COLUMNS: &str = "id, meeting_id, client_id, role, content, created_at";
        match scope {
            ChatScope::Meeting(id) => {
                sqlx::query_as::<_, ChatMessageRecord>(&format!(
                    "SELECT {} FROM chat_messages WHERE meeting_id = ? ORDER BY created_at ASC, id ASC",
                    COLUMNS
                ))
                .bind(id)
                .fetch_all(pool)
                .await
            }
            ChatScope::Client(id) => {
                sqlx::query_as::<_, ChatMessageRecord>(&format!(
                    "SELECT {} FROM chat_messages WHERE client_id = ? ORDER BY created_at ASC, id ASC",
                    COLUMNS
                ))
                .bind(id)
                .fetch_all(pool)
                .await
            }
            ChatScope::All => {
                sqlx::query_as::<_, ChatMessageRecord>(&format!(
                    "SELECT {} FROM chat_messages WHERE meeting_id IS NULL AND client_id IS NULL ORDER BY created_at ASC, id ASC",
                    COLUMNS
                ))
                .fetch_all(pool)
                .await
            }
        }
    }

    /// Deletes the history for a scope; returns the number of removed rows.
    pub async fn clear(pool: &SqlitePool, scope: &ChatScope) -> Result<u64, sqlx::Error> {
        let result = match scope {
            ChatScope::Meeting(id) => {
                sqlx::query("DELETE FROM chat_messages WHERE meeting_id = ?")
                    .bind(id)
                    .execute(pool)
                    .await?
            }
            ChatScope::Client(id) => {
                sqlx::query("DELETE FROM chat_messages WHERE client_id = ?")
                    .bind(id)
                    .execute(pool)
                    .await?
            }
            ChatScope::All => {
                sqlx::query("DELETE FROM chat_messages WHERE meeting_id IS NULL AND client_id IS NULL")
                    .execute(pool)
                    .await?
            }
        };
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_from_ids_prefers_client_then_meeting() {
        assert_eq!(
            ChatScope::from_ids(Some("m1".into()), Some("c1".into())),
            ChatScope::Client("c1".into())
        );
        assert_eq!(
            ChatScope::from_ids(Some("m1".into()), None),
            ChatScope::Meeting("m1".into())
        );
        assert_eq!(ChatScope::from_ids(None, None), ChatScope::All);
    }

    #[test]
    fn scope_ids_are_shape_specific() {
        let client = ChatScope::Client("c1".into());
        assert_eq!(client.client_id(), Some("c1"));
        assert_eq!(client.meeting_id(), None);
        let meeting = ChatScope::Meeting("m1".into());
        assert_eq!(meeting.meeting_id(), Some("m1"));
        assert_eq!(meeting.client_id(), None);
    }
}
