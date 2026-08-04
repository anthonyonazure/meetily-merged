use crate::database::models::{MemoryFact, MemoryFactWithMeeting};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

const FACT_COLUMNS: &str = "id, meeting_id, client_id, agent_run_id, kind, subject, detail, owner, due_hint, amount, status, created_at, updated_at";

pub struct MemoryFactsRepository;

#[allow(clippy::too_many_arguments)]
impl MemoryFactsRepository {
    pub async fn insert(
        pool: &SqlitePool,
        meeting_id: &str,
        client_id: Option<&str>,
        agent_run_id: Option<&str>,
        kind: &str,
        subject: &str,
        detail: &str,
        owner: Option<&str>,
        due_hint: Option<&str>,
        amount: Option<&str>,
    ) -> Result<String, sqlx::Error> {
        let id = format!("fact-{}", Uuid::new_v4());
        let now = Utc::now();
        // Commitments have a lifecycle; every other kind is 'na'.
        let status = if kind == "commitment" { "open" } else { "na" };
        sqlx::query(
            "INSERT INTO memory_facts (id, meeting_id, client_id, agent_run_id, kind, subject, detail, owner, due_hint, amount, status, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(meeting_id)
        .bind(client_id)
        .bind(agent_run_id)
        .bind(kind)
        .bind(subject)
        .bind(detail)
        .bind(owner)
        .bind(due_hint)
        .bind(amount)
        .bind(status)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
        Ok(id)
    }

    /// Removes agent-extracted facts that are still in their initial state
    /// ('open' commitments and 'na' facts) before a fresh extraction, so
    /// re-running the extractor replaces instead of duplicating. Facts the
    /// user marked done/dismissed, and manually created facts (no
    /// agent_run_id), are preserved.
    pub async fn clear_replaceable_agent_facts(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            "DELETE FROM memory_facts
             WHERE meeting_id = ? AND agent_run_id IS NOT NULL AND status IN ('open', 'na')",
        )
        .bind(meeting_id)
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<MemoryFact>, sqlx::Error> {
        sqlx::query_as::<_, MemoryFact>(&format!(
            "SELECT {} FROM memory_facts WHERE meeting_id = ? ORDER BY kind ASC, created_at ASC",
            FACT_COLUMNS
        ))
        .bind(meeting_id)
        .fetch_all(pool)
        .await
    }

    /// All facts for a client, newest meeting first, joined with meeting info
    /// for the timeline view.
    pub async fn for_client(
        pool: &SqlitePool,
        client_id: &str,
    ) -> Result<Vec<MemoryFactWithMeeting>, sqlx::Error> {
        sqlx::query_as::<_, MemoryFactWithMeeting>(
            "SELECT f.id, f.meeting_id, f.client_id, f.agent_run_id, f.kind, f.subject, f.detail,
                    f.owner, f.due_hint, f.amount, f.status, f.created_at, f.updated_at,
                    m.title AS meeting_title, m.created_at AS meeting_created_at
             FROM memory_facts f
             JOIN meetings m ON m.id = f.meeting_id
             WHERE f.client_id = ?
             ORDER BY m.created_at DESC, f.kind ASC, f.created_at ASC",
        )
        .bind(client_id)
        .fetch_all(pool)
        .await
    }

    pub async fn set_status(
        pool: &SqlitePool,
        fact_id: &str,
        status: &str,
    ) -> Result<bool, sqlx::Error> {
        let now = Utc::now();
        let result = sqlx::query("UPDATE memory_facts SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(now)
            .bind(fact_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete(pool: &SqlitePool, fact_id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM memory_facts WHERE id = ?")
            .bind(fact_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// LIKE search over subject + detail, optionally scoped to one client.
    pub async fn search(
        pool: &SqlitePool,
        query: &str,
        client_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<MemoryFactWithMeeting>, sqlx::Error> {
        // Escape LIKE wildcards so user input matches literally.
        let escaped = query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        let pattern = format!("%{}%", escaped);
        let base = "SELECT f.id, f.meeting_id, f.client_id, f.agent_run_id, f.kind, f.subject, f.detail,
                    f.owner, f.due_hint, f.amount, f.status, f.created_at, f.updated_at,
                    m.title AS meeting_title, m.created_at AS meeting_created_at
             FROM memory_facts f
             JOIN meetings m ON m.id = f.meeting_id
             WHERE (f.subject LIKE ? ESCAPE '\\' OR f.detail LIKE ? ESCAPE '\\')";
        match client_id {
            Some(client) => {
                sqlx::query_as::<_, MemoryFactWithMeeting>(&format!(
                    "{} AND f.client_id = ? ORDER BY m.created_at DESC LIMIT ?",
                    base
                ))
                .bind(&pattern)
                .bind(&pattern)
                .bind(client)
                .bind(limit)
                .fetch_all(pool)
                .await
            }
            None => {
                sqlx::query_as::<_, MemoryFactWithMeeting>(&format!(
                    "{} ORDER BY m.created_at DESC LIMIT ?",
                    base
                ))
                .bind(&pattern)
                .bind(&pattern)
                .bind(limit)
                .fetch_all(pool)
                .await
            }
        }
    }

    /// Open commitments for one client created more than `min_age_days` days
    /// ago (0 = all open commitments), newest meeting first.
    pub async fn open_commitments_for_client(
        pool: &SqlitePool,
        client_id: &str,
        min_age_days: i64,
    ) -> Result<Vec<MemoryFactWithMeeting>, sqlx::Error> {
        let cutoff = Utc::now() - chrono::Duration::days(min_age_days);
        sqlx::query_as::<_, MemoryFactWithMeeting>(
            "SELECT f.id, f.meeting_id, f.client_id, f.agent_run_id, f.kind, f.subject, f.detail,
                    f.owner, f.due_hint, f.amount, f.status, f.created_at, f.updated_at,
                    m.title AS meeting_title, m.created_at AS meeting_created_at
             FROM memory_facts f
             JOIN meetings m ON m.id = f.meeting_id
             WHERE f.client_id = ? AND f.kind = 'commitment' AND f.status = 'open'
               AND f.created_at <= ?
             ORDER BY f.created_at ASC",
        )
        .bind(client_id)
        .bind(cutoff)
        .fetch_all(pool)
        .await
    }

    /// Count of open commitments older than `min_age_days` days across all
    /// clients (untagged facts excluded — the badge lives on the Clients nav).
    pub async fn stale_open_count(
        pool: &SqlitePool,
        min_age_days: i64,
    ) -> Result<i64, sqlx::Error> {
        let cutoff = Utc::now() - chrono::Duration::days(min_age_days);
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM memory_facts
             WHERE client_id IS NOT NULL AND kind = 'commitment' AND status = 'open'
               AND created_at <= ?",
        )
        .bind(cutoff)
        .fetch_one(pool)
        .await?;
        Ok(row.0)
    }

    /// Clears the client link on facts when a client is deleted (facts and
    /// meetings are kept).
    pub async fn unlink_client(pool: &SqlitePool, client_id: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("UPDATE memory_facts SET client_id = NULL WHERE client_id = ?")
            .bind(client_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}
