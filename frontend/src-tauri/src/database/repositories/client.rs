use crate::database::models::{Client, ClientWithCounts};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct ClientsRepository;

impl ClientsRepository {
    /// Lists clients with meeting and open-commitment counts, alphabetical by
    /// name.
    pub async fn list_with_counts(pool: &SqlitePool) -> Result<Vec<ClientWithCounts>, sqlx::Error> {
        sqlx::query_as::<_, ClientWithCounts>(
            "SELECT c.id, c.name, c.domain, c.notes, c.created_at, c.privacy_profile_id,
                    (SELECT COUNT(*) FROM meeting_clients mc WHERE mc.client_id = c.id) AS meeting_count,
                    (SELECT COUNT(*) FROM memory_facts f
                     WHERE f.client_id = c.id AND f.kind = 'commitment' AND f.status = 'open') AS open_commitments
             FROM clients c
             ORDER BY c.name COLLATE NOCASE ASC",
        )
        .fetch_all(pool)
        .await
    }

    pub async fn get(pool: &SqlitePool, client_id: &str) -> Result<Option<Client>, sqlx::Error> {
        sqlx::query_as::<_, Client>(
            "SELECT id, name, domain, notes, created_at, privacy_profile_id
             FROM clients WHERE id = ?",
        )
        .bind(client_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn create(
        pool: &SqlitePool,
        name: &str,
        domain: Option<&str>,
        notes: &str,
    ) -> Result<Client, sqlx::Error> {
        let client = Client {
            id: format!("client-{}", Uuid::new_v4()),
            name: name.to_string(),
            domain: domain.map(str::to_string),
            notes: notes.to_string(),
            created_at: Utc::now(),
            privacy_profile_id: None,
        };
        sqlx::query(
            "INSERT INTO clients (id, name, domain, notes, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&client.id)
        .bind(&client.name)
        .bind(client.domain.as_deref())
        .bind(&client.notes)
        .bind(client.created_at)
        .execute(pool)
        .await?;
        Ok(client)
    }

    pub async fn update(
        pool: &SqlitePool,
        client_id: &str,
        name: &str,
        domain: Option<&str>,
        notes: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("UPDATE clients SET name = ?, domain = ?, notes = ? WHERE id = ?")
            .bind(name)
            .bind(domain)
            .bind(notes)
            .bind(client_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Deletes a client and its meeting links. Meetings themselves are kept.
    pub async fn delete(pool: &SqlitePool, client_id: &str) -> Result<bool, sqlx::Error> {
        sqlx::query("DELETE FROM meeting_clients WHERE client_id = ?")
            .bind(client_id)
            .execute(pool)
            .await?;
        let result = sqlx::query("DELETE FROM clients WHERE id = ?")
            .bind(client_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

pub struct MeetingClientsRepository;

impl MeetingClientsRepository {
    /// Tags a meeting with a client (replacing any existing tag), or clears the
    /// tag when `client_id` is None.
    pub async fn set(
        pool: &SqlitePool,
        meeting_id: &str,
        client_id: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM meeting_clients WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(pool)
            .await?;
        if let Some(client_id) = client_id {
            sqlx::query("INSERT INTO meeting_clients (meeting_id, client_id) VALUES (?, ?)")
                .bind(meeting_id)
                .bind(client_id)
                .execute(pool)
                .await?;
        }
        Ok(())
    }

    /// All meetings tagged with a client, newest first.
    pub async fn meetings_for_client(
        pool: &SqlitePool,
        client_id: &str,
    ) -> Result<Vec<crate::database::models::MeetingModel>, sqlx::Error> {
        sqlx::query_as::<_, crate::database::models::MeetingModel>(
            "SELECT m.id, m.title, m.created_at, m.updated_at, m.folder_path
             FROM meeting_clients mc JOIN meetings m ON m.id = mc.meeting_id
             WHERE mc.client_id = ?
             ORDER BY m.created_at DESC",
        )
        .bind(client_id)
        .fetch_all(pool)
        .await
    }

    /// The client a meeting is tagged with, if any.
    pub async fn client_for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<Client>, sqlx::Error> {
        sqlx::query_as::<_, Client>(
            "SELECT c.id, c.name, c.domain, c.notes, c.created_at, c.privacy_profile_id
             FROM meeting_clients mc JOIN clients c ON c.id = mc.client_id
             WHERE mc.meeting_id = ? LIMIT 1",
        )
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
    }
}
