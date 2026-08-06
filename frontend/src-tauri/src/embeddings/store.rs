//! Persistence for the embedding index.
//!
//! Kept in the feature module rather than `database::repositories` because every
//! query here is specific to vector retrieval and has no other caller.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use super::vector;

/// What a stored vector was made from.
pub const KIND_TRANSCRIPT_CHUNK: &str = "transcript_chunk";
pub const KIND_SUMMARY: &str = "summary";
pub const KIND_MEMORY_FACT: &str = "memory_fact";

/// One row of the index, with its vector already decoded.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: String,
    pub source_kind: String,
    pub source_id: String,
    pub meeting_id: String,
    pub client_id: Option<String>,
    pub chunk_text: String,
    pub vector: Vec<f32>,
}

/// A passage waiting to be written.
#[derive(Debug, Clone)]
pub struct NewEmbedding {
    pub source_kind: String,
    pub source_id: String,
    pub meeting_id: String,
    pub client_id: Option<String>,
    pub chunk_text: String,
    pub vector: Vec<f32>,
}

/// How much of the corpus is indexed, and by which model.
#[derive(Debug, Clone, Serialize)]
pub struct IndexCounts {
    pub transcript_chunks: i64,
    pub summaries: i64,
    pub memory_facts: i64,
    pub total: i64,
    /// Meetings with at least one indexed passage.
    pub meetings_indexed: i64,
    /// Meetings that have a transcript at all.
    pub meetings_with_transcripts: i64,
    /// Rows written by a model other than the one configured now. Non-zero means
    /// the index is mixed and needs a reindex to be trustworthy.
    pub rows_from_other_models: i64,
    pub last_indexed_at: Option<DateTime<Utc>>,
}

pub struct EmbeddingsStore;

impl EmbeddingsStore {
    /// Upserts one passage. The unique index on (source_kind, source_id, model)
    /// means re-indexing replaces rather than duplicating.
    pub async fn upsert(
        pool: &SqlitePool,
        model: &str,
        item: &NewEmbedding,
    ) -> Result<(), sqlx::Error> {
        let bytes = vector::encode(&item.vector);
        sqlx::query(
            "INSERT INTO embeddings
                 (id, source_kind, source_id, meeting_id, client_id, chunk_text,
                  vector, dim, model, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(source_kind, source_id, model) DO UPDATE SET
                 meeting_id = excluded.meeting_id,
                 client_id = excluded.client_id,
                 chunk_text = excluded.chunk_text,
                 vector = excluded.vector,
                 dim = excluded.dim,
                 created_at = excluded.created_at",
        )
        .bind(format!("emb-{}", Uuid::new_v4()))
        .bind(&item.source_kind)
        .bind(&item.source_id)
        .bind(&item.meeting_id)
        .bind(item.client_id.as_deref())
        .bind(&item.chunk_text)
        .bind(bytes)
        .bind(item.vector.len() as i64)
        .bind(model)
        .bind(Utc::now())
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Removes every passage for one meeting and kind, so a re-index of a meeting
    /// whose transcript shrank does not leave orphans behind.
    pub async fn delete_for_meeting_kind(
        pool: &SqlitePool,
        meeting_id: &str,
        source_kind: &str,
    ) -> Result<u64, sqlx::Error> {
        let result =
            sqlx::query("DELETE FROM embeddings WHERE meeting_id = ? AND source_kind = ?")
                .bind(meeting_id)
                .bind(source_kind)
                .execute(pool)
                .await?;
        Ok(result.rows_affected())
    }

    pub async fn delete_all(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM embeddings").execute(pool).await?;
        Ok(result.rows_affected())
    }

    /// Keeps the client link in step when a meeting is tagged or a client is
    /// deleted, so client-scoped retrieval does not go stale.
    pub async fn resync_client_links(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE embeddings SET client_id = (
                 SELECT mc.client_id FROM meeting_clients mc
                 WHERE mc.meeting_id = embeddings.meeting_id LIMIT 1
             )",
        )
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// The candidate set for a cosine scan, narrowed in SQL first.
    ///
    /// This is where the "brute force is correct at this scale" decision lives:
    /// SQLite has no vector index and this tree adds no extension, so the
    /// candidate set is narrowed by client, meeting, and date in SQL and then
    /// scanned linearly in Rust. `limit` caps how many rows are ever decoded, so
    /// the worst case is bounded rather than open-ended.
    pub async fn candidates(
        pool: &SqlitePool,
        model: &str,
        meeting_id: Option<&str>,
        client_id: Option<&str>,
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<Candidate>, sqlx::Error> {
        let mut sql = String::from(
            "SELECT id, source_kind, source_id, meeting_id, client_id, chunk_text, vector, dim
             FROM embeddings WHERE model = ?",
        );
        if meeting_id.is_some() {
            sql.push_str(" AND meeting_id = ?");
        }
        if client_id.is_some() {
            sql.push_str(" AND client_id = ?");
        }
        if since.is_some() {
            sql.push_str(" AND created_at >= ?");
        }
        sql.push_str(" ORDER BY created_at DESC LIMIT ?");

        let mut query = sqlx::query(&sql).bind(model);
        if let Some(id) = meeting_id {
            query = query.bind(id);
        }
        if let Some(id) = client_id {
            query = query.bind(id);
        }
        if let Some(since) = since {
            query = query.bind(since);
        }
        let rows = query.bind(limit).fetch_all(pool).await?;

        let mut candidates = Vec::with_capacity(rows.len());
        for row in rows {
            let bytes: Vec<u8> = row.try_get("vector")?;
            let dim: i64 = row.try_get("dim")?;
            let Some(decoded) = vector::decode(&bytes, dim.max(0) as usize) else {
                // A row whose blob does not match its declared width is skipped
                // rather than scored on garbage; a reindex repairs it.
                log::warn!("[Embeddings] skipping a row with a malformed vector blob");
                continue;
            };
            candidates.push(Candidate {
                id: row.try_get("id")?,
                source_kind: row.try_get("source_kind")?,
                source_id: row.try_get("source_id")?,
                meeting_id: row.try_get("meeting_id")?,
                client_id: row.try_get("client_id")?,
                chunk_text: row.try_get("chunk_text")?,
                vector: decoded,
            });
        }
        Ok(candidates)
    }

    /// Meeting ids that have transcripts, newest first — the work list for a
    /// full reindex.
    pub async fn meetings_with_transcripts(
        pool: &SqlitePool,
    ) -> Result<Vec<String>, sqlx::Error> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT m.id FROM meetings m
             WHERE EXISTS (SELECT 1 FROM transcripts t WHERE t.meeting_id = m.id)
             ORDER BY m.created_at DESC",
        )
        .fetch_all(pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    pub async fn counts(pool: &SqlitePool, model: &str) -> Result<IndexCounts, sqlx::Error> {
        let by_kind: Vec<(String, i64)> = sqlx::query_as(
            "SELECT source_kind, COUNT(*) FROM embeddings WHERE model = ? GROUP BY source_kind",
        )
        .bind(model)
        .fetch_all(pool)
        .await?;

        let find = |kind: &str| -> i64 {
            by_kind
                .iter()
                .find(|(k, _)| k == kind)
                .map(|(_, n)| *n)
                .unwrap_or(0)
        };

        let (meetings_indexed,): (i64,) =
            sqlx::query_as("SELECT COUNT(DISTINCT meeting_id) FROM embeddings WHERE model = ?")
                .bind(model)
                .fetch_one(pool)
                .await?;
        let (meetings_with_transcripts,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM meetings m
             WHERE EXISTS (SELECT 1 FROM transcripts t WHERE t.meeting_id = m.id)",
        )
        .fetch_one(pool)
        .await?;
        let (rows_from_other_models,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM embeddings WHERE model <> ?")
                .bind(model)
                .fetch_one(pool)
                .await?;
        // MAX() over an empty set yields a row holding NULL, so the column is
        // decoded as an Option rather than fetched optionally.
        let (last_indexed_at,): (Option<DateTime<Utc>>,) =
            sqlx::query_as("SELECT MAX(created_at) FROM embeddings WHERE model = ?")
                .bind(model)
                .fetch_one(pool)
                .await?;

        let transcript_chunks = find(KIND_TRANSCRIPT_CHUNK);
        let summaries = find(KIND_SUMMARY);
        let memory_facts = find(KIND_MEMORY_FACT);
        Ok(IndexCounts {
            transcript_chunks,
            summaries,
            memory_facts,
            total: transcript_chunks + summaries + memory_facts,
            meetings_indexed,
            meetings_with_transcripts,
            rows_from_other_models,
            last_indexed_at,
        })
    }
}
