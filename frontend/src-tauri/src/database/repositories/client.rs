use crate::database::models::{Client, ClientWithCounts};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct ClientsRepository;

impl ClientsRepository {
    /// Lists clients with meeting counts, alphabetical by name.
    pub async fn list_with_counts(pool: &SqlitePool) -> Result<Vec<ClientWithCounts>, sqlx::Error> {
        sqlx::query_as::<_, ClientWithCounts>(
            "SELECT c.id, c.name, c.domain, c.notes, c.created_at,
                    (SELECT COUNT(*) FROM meeting_clients mc WHERE mc.client_id = c.id) AS meeting_count,
                    0 AS open_commitments
             FROM clients c
             ORDER BY c.name COLLATE NOCASE ASC",
        )
        .fetch_all(pool)
        .await
    }

    pub async fn get(pool: &SqlitePool, client_id: &str) -> Result<Option<Client>, sqlx::Error> {
        sqlx::query_as::<_, Client>(
            "SELECT id, name, domain, notes, created_at FROM clients WHERE id = ?",
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

    /// The client a meeting is tagged with, if any.
    pub async fn client_for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<Client>, sqlx::Error> {
        sqlx::query_as::<_, Client>(
            "SELECT c.id, c.name, c.domain, c.notes, c.created_at
             FROM meeting_clients mc JOIN clients c ON c.id = mc.client_id
             WHERE mc.meeting_id = ? LIMIT 1",
        )
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
    }
}
