use crate::database::models::{ActionItem, ActionItemWithMeeting, AgentRun, AgentSettingRow};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct AgentRunsRepository;

impl AgentRunsRepository {
    /// Inserts a new agent run in `running` state and returns its id.
    pub async fn create_run(
        pool: &SqlitePool,
        agent_id: &str,
        meeting_id: &str,
    ) -> Result<String, sqlx::Error> {
        let run_id = format!("agentrun-{}", Uuid::new_v4());
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO agent_runs (id, agent_id, meeting_id, status, output_md, error, created_at)
             VALUES (?, ?, ?, 'running', NULL, NULL, ?)",
        )
        .bind(&run_id)
        .bind(agent_id)
        .bind(meeting_id)
        .bind(now)
        .execute(pool)
        .await?;
        Ok(run_id)
    }

    pub async fn complete_run(
        pool: &SqlitePool,
        run_id: &str,
        output_md: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE agent_runs SET status = 'completed', output_md = ?, error = NULL WHERE id = ?",
        )
        .bind(output_md)
        .bind(run_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn fail_run(pool: &SqlitePool, run_id: &str, error: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE agent_runs SET status = 'error', error = ? WHERE id = ?")
            .bind(error)
            .bind(run_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn runs_for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<AgentRun>, sqlx::Error> {
        sqlx::query_as::<_, AgentRun>(
            "SELECT id, agent_id, meeting_id, status, output_md, error, created_at
             FROM agent_runs WHERE meeting_id = ? ORDER BY created_at DESC",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await
    }

    /// True when the meeting has at least one run for the given agent that is
    /// currently running (guards against double auto-runs).
    pub async fn has_running_run(
        pool: &SqlitePool,
        meeting_id: &str,
        agent_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM agent_runs WHERE meeting_id = ? AND agent_id = ? AND status = 'running' LIMIT 1",
        )
        .bind(meeting_id)
        .bind(agent_id)
        .fetch_optional(pool)
        .await?;
        Ok(row.is_some())
    }
}

pub struct ActionItemsRepository;

impl ActionItemsRepository {
    pub async fn insert(
        pool: &SqlitePool,
        meeting_id: &str,
        agent_run_id: Option<&str>,
        description: &str,
        owner: Option<&str>,
        due_hint: Option<&str>,
    ) -> Result<String, sqlx::Error> {
        let id = format!("action-{}", Uuid::new_v4());
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO action_items (id, meeting_id, agent_run_id, description, owner, due_hint, status, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, 'open', ?, ?)",
        )
        .bind(&id)
        .bind(meeting_id)
        .bind(agent_run_id)
        .bind(description)
        .bind(owner)
        .bind(due_hint)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
        Ok(id)
    }

    /// Removes still-open agent-extracted items for a meeting before a fresh
    /// extraction, so re-running the Action Tracker replaces instead of
    /// duplicating. Items marked done and manually created items (no
    /// agent_run_id) are preserved.
    pub async fn clear_open_agent_items(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            "DELETE FROM action_items WHERE meeting_id = ? AND status = 'open' AND agent_run_id IS NOT NULL",
        )
        .bind(meeting_id)
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn list_all(pool: &SqlitePool) -> Result<Vec<ActionItemWithMeeting>, sqlx::Error> {
        sqlx::query_as::<_, ActionItemWithMeeting>(
            "SELECT a.id, a.meeting_id, a.agent_run_id, a.description, a.owner, a.due_hint,
                    a.status, a.created_at, a.updated_at, m.title AS meeting_title
             FROM action_items a
             JOIN meetings m ON m.id = a.meeting_id
             ORDER BY m.created_at DESC, a.created_at ASC",
        )
        .fetch_all(pool)
        .await
    }

    pub async fn list_for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<ActionItem>, sqlx::Error> {
        sqlx::query_as::<_, ActionItem>(
            "SELECT id, meeting_id, agent_run_id, description, owner, due_hint, status, created_at, updated_at
             FROM action_items WHERE meeting_id = ? ORDER BY created_at ASC",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await
    }

    pub async fn set_status(
        pool: &SqlitePool,
        action_id: &str,
        status: &str,
    ) -> Result<bool, sqlx::Error> {
        let now = Utc::now();
        let result = sqlx::query("UPDATE action_items SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(now)
            .bind(action_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete(pool: &SqlitePool, action_id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM action_items WHERE id = ?")
            .bind(action_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

pub struct AgentSettingsRepository;

impl AgentSettingsRepository {
    pub async fn get_all(pool: &SqlitePool) -> Result<Vec<AgentSettingRow>, sqlx::Error> {
        sqlx::query_as::<_, AgentSettingRow>(
            "SELECT agent_id, enabled, auto_run FROM agent_settings",
        )
        .fetch_all(pool)
        .await
    }

    pub async fn get(
        pool: &SqlitePool,
        agent_id: &str,
    ) -> Result<Option<AgentSettingRow>, sqlx::Error> {
        sqlx::query_as::<_, AgentSettingRow>(
            "SELECT agent_id, enabled, auto_run FROM agent_settings WHERE agent_id = ?",
        )
        .bind(agent_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn upsert(
        pool: &SqlitePool,
        agent_id: &str,
        enabled: bool,
        auto_run: bool,
    ) -> Result<(), sqlx::Error> {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO agent_settings (agent_id, enabled, auto_run, updated_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(agent_id) DO UPDATE SET
                 enabled = excluded.enabled,
                 auto_run = excluded.auto_run,
                 updated_at = excluded.updated_at",
        )
        .bind(agent_id)
        .bind(enabled)
        .bind(auto_run)
        .bind(now)
        .execute(pool)
        .await?;
        Ok(())
    }
}
