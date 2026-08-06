//! Building the index: after a summary completes, and on demand for everything
//! already recorded.
//!
//! ## Privacy
//!
//! Nothing here sends text anywhere — the model runs in-process on this machine.
//! But "local" is not the same as "allowed": under strict per-speaker consent an
//! unconfirmed speaker's words must not become searchable, because a semantic
//! index is exactly the kind of durable derived copy that consent is supposed to
//! prevent. So withheld segments are dropped **before** chunking rather than
//! replaced with the withheld marker: the marker text itself never enters the
//! index either, and a later confirmation plus a reindex brings the speech back.

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Runtime};

use crate::database::repositories::{
    client::MeetingClientsRepository, meeting::MeetingsRepository, memory::MemoryFactsRepository,
    summary::SummaryProcessesRepository,
};

use super::chunk::{self, SourceSegment};
use super::model;
use super::settings;
use super::store::{
    EmbeddingsStore, NewEmbedding, KIND_MEMORY_FACT, KIND_SUMMARY, KIND_TRANSCRIPT_CHUNK,
};

/// Progress event for a full reindex.
pub const REINDEX_PROGRESS_EVENT: &str = "embeddings-reindex-progress";

#[derive(Debug, Clone, Serialize)]
pub struct ReindexProgress {
    /// `preparing` | `indexing` | `complete` | `failed`
    pub phase: String,
    pub meetings_done: usize,
    pub meetings_total: usize,
    pub passages_written: usize,
    pub current_meeting_title: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReindexResult {
    pub meetings_indexed: usize,
    pub passages_written: usize,
    pub skipped_for_consent: usize,
}

/// Indexes one meeting's transcript, summary, and memory facts.
///
/// Returns the number of passages written and the number of transcript segments
/// dropped for consent, so the caller can report both honestly.
pub async fn index_meeting(
    pool: &SqlitePool,
    meeting_id: &str,
    model_id: &str,
) -> Result<(usize, usize), String> {
    let client_id = MeetingClientsRepository::client_for_meeting(pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to read the meeting's client: {}", e))?
        .map(|client| client.id);

    let mut written = 0usize;
    let mut skipped = 0usize;

    // --- Transcript passages -------------------------------------------------
    let (transcripts, total) =
        MeetingsRepository::get_meeting_transcripts_paginated(pool, meeting_id, i64::MAX, 0)
            .await
            .map_err(|e| format!("Failed to load transcripts: {}", e))?;

    if total > 0 {
        let consent = crate::consent::filter::state_for_meeting(pool, meeting_id).await;
        let mut segments: Vec<SourceSegment> = Vec::with_capacity(transcripts.len());
        for row in &transcripts {
            if consent.withholds(row.speaker.as_deref()) {
                skipped += 1;
                continue;
            }
            segments.push(SourceSegment {
                id: row.id.clone(),
                speaker: row.speaker.clone(),
                text: row.transcript.clone(),
            });
        }
        if skipped > 0 {
            log::info!(
                "[Embeddings] left {} unconsented segment(s) out of the index for meeting {}",
                skipped,
                meeting_id
            );
        }

        let passages = chunk::chunk_segments(&segments, chunk::TRANSCRIPT_TARGET_CHARS);
        EmbeddingsStore::delete_for_meeting_kind(pool, meeting_id, KIND_TRANSCRIPT_CHUNK)
            .await
            .map_err(|e| format!("Failed to clear stale transcript vectors: {}", e))?;

        if !passages.is_empty() {
            let texts: Vec<String> = passages.iter().map(|p| p.text.clone()).collect();
            let vectors = model::embed(texts).await?;
            for (passage, vector) in passages.iter().zip(vectors) {
                EmbeddingsStore::upsert(
                    pool,
                    model_id,
                    &NewEmbedding {
                        source_kind: KIND_TRANSCRIPT_CHUNK.to_string(),
                        source_id: passage.source_id.clone(),
                        meeting_id: meeting_id.to_string(),
                        client_id: client_id.clone(),
                        chunk_text: passage.text.clone(),
                        vector,
                    },
                )
                .await
                .map_err(|e| format!("Failed to store a transcript vector: {}", e))?;
                written += 1;
            }
        }
    }

    // --- Summary passages ----------------------------------------------------
    let summary = match SummaryProcessesRepository::get_summary_data(pool, meeting_id).await {
        Ok(Some(process)) => {
            crate::chat::service::summary_markdown_from_result(process.result.as_deref())
        }
        _ => None,
    };
    EmbeddingsStore::delete_for_meeting_kind(pool, meeting_id, KIND_SUMMARY)
        .await
        .map_err(|e| format!("Failed to clear stale summary vectors: {}", e))?;
    if let Some(markdown) = summary {
        let passages = chunk::split_document(&markdown, chunk::SUMMARY_TARGET_CHARS);
        if !passages.is_empty() {
            let vectors = model::embed(passages.clone()).await?;
            for (index, (text, vector)) in passages.into_iter().zip(vectors).enumerate() {
                EmbeddingsStore::upsert(
                    pool,
                    model_id,
                    &NewEmbedding {
                        source_kind: KIND_SUMMARY.to_string(),
                        // A summary has no row id of its own, so passages are keyed
                        // by meeting id plus their position.
                        source_id: format!("{}#{}", meeting_id, index),
                        meeting_id: meeting_id.to_string(),
                        client_id: client_id.clone(),
                        chunk_text: text,
                        vector,
                    },
                )
                .await
                .map_err(|e| format!("Failed to store a summary vector: {}", e))?;
                written += 1;
            }
        }
    }

    // --- Memory facts --------------------------------------------------------
    let facts = MemoryFactsRepository::for_meeting(pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to load memory facts: {}", e))?;
    EmbeddingsStore::delete_for_meeting_kind(pool, meeting_id, KIND_MEMORY_FACT)
        .await
        .map_err(|e| format!("Failed to clear stale memory-fact vectors: {}", e))?;
    let indexable: Vec<_> = facts
        .into_iter()
        .filter(|fact| fact.status != "dismissed")
        .collect();
    if !indexable.is_empty() {
        let texts: Vec<String> = indexable.iter().map(fact_text).collect();
        let vectors = model::embed(texts.clone()).await?;
        for ((fact, text), vector) in indexable.iter().zip(texts).zip(vectors) {
            EmbeddingsStore::upsert(
                pool,
                model_id,
                &NewEmbedding {
                    source_kind: KIND_MEMORY_FACT.to_string(),
                    source_id: fact.id.clone(),
                    meeting_id: meeting_id.to_string(),
                    client_id: fact.client_id.clone().or_else(|| client_id.clone()),
                    chunk_text: text,
                    vector,
                },
            )
            .await
            .map_err(|e| format!("Failed to store a memory-fact vector: {}", e))?;
            written += 1;
        }
    }

    Ok((written, skipped))
}

/// Renders one memory fact as a searchable sentence. Kept close to how the fact
/// reads in the client timeline so a search for the words a user remembers seeing
/// matches.
fn fact_text(fact: &crate::database::models::MemoryFact) -> String {
    let mut text = format!("{}: {} — {}", fact.kind, fact.subject, fact.detail);
    if let Some(owner) = fact.owner.as_deref() {
        text.push_str(&format!(" (owner: {})", owner));
    }
    if let Some(due) = fact.due_hint.as_deref() {
        text.push_str(&format!(" (due: {})", due));
    }
    if let Some(amount) = fact.amount.as_deref() {
        text.push_str(&format!(" ({})", amount));
    }
    text
}

/// Indexes one meeting after its summary completes. Fire-and-forget: an indexing
/// failure logs and stops there, because a summary the user is waiting on must
/// never be held up or failed by search bookkeeping.
pub async fn index_after_summary(pool: &SqlitePool, meeting_id: &str) {
    let settings = settings::load(pool).await;
    if !settings.enabled {
        return;
    }
    if !model::files_present() {
        log::info!(
            "[Embeddings] semantic search is on but the model is not downloaded yet; skipping the index pass for {}",
            meeting_id
        );
        return;
    }
    match index_meeting(pool, meeting_id, &settings.model).await {
        Ok((written, skipped)) => log::info!(
            "[Embeddings] indexed {} passage(s) for meeting {} ({} segment(s) withheld for consent)",
            written,
            meeting_id,
            skipped
        ),
        Err(e) => log::warn!("[Embeddings] indexing meeting {} failed: {}", meeting_id, e),
    }
}

/// Rebuilds the whole index, emitting progress as it goes.
pub async fn reindex_all<R: Runtime>(
    app: AppHandle<R>,
    pool: SqlitePool,
) -> Result<ReindexResult, String> {
    let settings = settings::load(&pool).await;
    if !settings.enabled {
        return Err("Semantic search is switched off".to_string());
    }
    if !model::files_present() {
        return Err("The embedding model has not been downloaded yet".to_string());
    }

    let emit = |progress: ReindexProgress| {
        if let Err(e) = app.emit(REINDEX_PROGRESS_EVENT, &progress) {
            log::warn!("[Embeddings] failed to emit reindex progress: {}", e);
        }
    };

    emit(ReindexProgress {
        phase: "preparing".to_string(),
        meetings_done: 0,
        meetings_total: 0,
        passages_written: 0,
        current_meeting_title: None,
        error: None,
    });

    // Everything is rebuilt rather than topped up: a reindex is what an operator
    // reaches for when they do not trust the index, so leaving old rows in place
    // would defeat the point.
    EmbeddingsStore::delete_all(&pool)
        .await
        .map_err(|e| format!("Failed to clear the existing index: {}", e))?;

    let meetings = EmbeddingsStore::meetings_with_transcripts(&pool)
        .await
        .map_err(|e| format!("Failed to list meetings: {}", e))?;
    let total = meetings.len();

    let mut written = 0usize;
    let mut skipped = 0usize;
    let mut done = 0usize;
    for meeting_id in meetings {
        let title = MeetingsRepository::get_meeting_metadata(&pool, &meeting_id)
            .await
            .ok()
            .flatten()
            .map(|meeting| meeting.title);
        match index_meeting(&pool, &meeting_id, &settings.model).await {
            Ok((count, withheld)) => {
                written += count;
                skipped += withheld;
            }
            // One bad meeting must not abandon the rest of the rebuild.
            Err(e) => log::warn!(
                "[Embeddings] reindex skipped meeting {} after an error: {}",
                meeting_id,
                e
            ),
        }
        done += 1;
        emit(ReindexProgress {
            phase: "indexing".to_string(),
            meetings_done: done,
            meetings_total: total,
            passages_written: written,
            current_meeting_title: title,
            error: None,
        });
    }

    emit(ReindexProgress {
        phase: "complete".to_string(),
        meetings_done: done,
        meetings_total: total,
        passages_written: written,
        current_meeting_title: None,
        error: None,
    });

    Ok(ReindexResult {
        meetings_indexed: done,
        passages_written: written,
        skipped_for_consent: skipped,
    })
}
