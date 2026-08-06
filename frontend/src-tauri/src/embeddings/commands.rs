//! Tauri commands for local semantic search.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Runtime};

use crate::state::AppState;

use super::index;
use super::model;
use super::search::{self, SearchResults, SearchScope};
use super::settings::{self, EmbeddingsSettings};
use super::store::{EmbeddingsStore, IndexCounts};

/// Guard against two reindex runs racing each other over the same rows.
static REINDEX_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Everything the settings panel needs to tell the truth about the index.
#[derive(Debug, Clone, Serialize)]
pub struct IndexStatus {
    pub settings: EmbeddingsSettings,
    pub counts: IndexCounts,
    pub model_downloaded: bool,
    pub model_loaded: bool,
    pub model_id: String,
    pub dimensions: usize,
    pub download_size_mb: u32,
    pub models_dir: Option<String>,
    pub reindex_running: bool,
    /// One plain-English line for the panel: what state the index is actually in.
    pub summary: String,
}

/// Searches meetings by meaning and by word, merged.
#[tauri::command]
pub async fn search_semantic(
    state: tauri::State<'_, AppState>,
    query: String,
    meeting_id: Option<String>,
    client_id: Option<String>,
    top_k: Option<i64>,
) -> Result<SearchResults, String> {
    let scope = SearchScope {
        meeting_id,
        client_id,
        since: None,
    };
    search::hybrid(state.db_manager.pool(), &query, &scope, top_k).await
}

/// How much is indexed, by which model, and whether it can be trusted.
#[tauri::command]
pub async fn embeddings_index_status(
    state: tauri::State<'_, AppState>,
) -> Result<IndexStatus, String> {
    let pool = state.db_manager.pool();
    let settings = settings::load(pool).await;
    let counts = EmbeddingsStore::counts(pool, &settings.model)
        .await
        .map_err(|e| format!("Failed to read index status: {}", e))?;
    let downloaded = model::files_present();

    let unindexed = (counts.meetings_with_transcripts - counts.meetings_indexed).max(0);
    let summary = if !settings.enabled {
        "Semantic search is off. Searches use word matching only.".to_string()
    } else if !downloaded {
        format!(
            "Semantic search is on but the {} MB model has not finished downloading, so searches use word matching only.",
            model::DOWNLOAD_SIZE_MB
        )
    } else if counts.total == 0 {
        "The model is ready but nothing is indexed yet. Reindex to make past meetings searchable by meaning.".to_string()
    } else if unindexed > 0 {
        format!(
            "{} passage(s) indexed across {} meeting(s). {} meeting(s) with transcripts are not indexed yet, so results are incomplete.",
            counts.total, counts.meetings_indexed, unindexed
        )
    } else if counts.rows_from_other_models > 0 {
        format!(
            "{} passage(s) indexed across {} meeting(s), plus {} left over from a different model. Reindex to clear the leftovers.",
            counts.total, counts.meetings_indexed, counts.rows_from_other_models
        )
    } else {
        format!(
            "{} passage(s) indexed across {} meeting(s). Every meeting with a transcript is covered.",
            counts.total, counts.meetings_indexed
        )
    };

    Ok(IndexStatus {
        settings,
        counts,
        model_downloaded: downloaded,
        model_loaded: model::is_loaded(),
        model_id: model::MODEL_ID.to_string(),
        dimensions: model::DIM,
        download_size_mb: model::DOWNLOAD_SIZE_MB,
        models_dir: model::models_directory().ok().map(|p| p.display().to_string()),
        reindex_running: REINDEX_IN_PROGRESS.load(Ordering::SeqCst),
        summary,
    })
}

/// Rebuilds the whole index in the background, reporting progress through the
/// `embeddings-reindex-progress` event.
#[tauri::command]
pub async fn embeddings_reindex<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    let pool = state.db_manager.pool().clone();
    let settings = settings::load(&pool).await;
    if !settings.enabled {
        return Err("Turn semantic search on first".to_string());
    }
    if !model::files_present() {
        return Err("The embedding model is still downloading".to_string());
    }
    if REINDEX_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return Err("A reindex is already running".to_string());
    }

    tauri::async_runtime::spawn(async move {
        let outcome = index::reindex_all(app.clone(), pool).await;
        REINDEX_IN_PROGRESS.store(false, Ordering::SeqCst);
        match outcome {
            Ok(result) => log::info!(
                "[Embeddings] reindex finished: {} meeting(s), {} passage(s), {} segment(s) withheld for consent",
                result.meetings_indexed,
                result.passages_written,
                result.skipped_for_consent
            ),
            Err(e) => {
                log::error!("[Embeddings] reindex failed: {}", e);
                use tauri::Emitter;
                let _ = app.emit(
                    index::REINDEX_PROGRESS_EVENT,
                    index::ReindexProgress {
                        phase: "failed".to_string(),
                        meetings_done: 0,
                        meetings_total: 0,
                        passages_written: 0,
                        current_meeting_title: None,
                        error: Some(e),
                    },
                );
            }
        }
    });

    Ok(true)
}

#[tauri::command]
pub async fn embeddings_settings_get(
    state: tauri::State<'_, AppState>,
) -> Result<EmbeddingsSettings, String> {
    Ok(settings::load(state.db_manager.pool()).await)
}

/// Saves the settings. Turning the feature on starts the model download in the
/// background (progress arrives on `embeddings-model-download-progress`); turning
/// it off releases the model's memory.
#[tauri::command]
pub async fn embeddings_settings_set<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    enabled: bool,
    model_name: Option<String>,
    top_k: Option<i64>,
) -> Result<EmbeddingsSettings, String> {
    // Resolve the models directory here as well as at startup, so a first-run
    // enable never depends on setup ordering.
    if model::models_directory().is_err() {
        model::set_models_directory(&app);
    }

    let pool = state.db_manager.pool();
    let current = settings::load(pool).await;
    let saved = settings::save(
        pool,
        EmbeddingsSettings {
            enabled,
            model: model_name.unwrap_or(current.model),
            top_k: top_k.unwrap_or(current.top_k),
        },
    )
    .await?;

    if saved.enabled && !model::files_present() {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = model::ensure_downloaded(&handle).await {
                log::error!("[Embeddings] model download failed: {}", e);
            }
        });
    }
    if !saved.enabled {
        model::unload();
    }

    Ok(saved)
}
