//! Typed view over the single `embeddings_settings` row.
//!
//! Reads never fail the caller: semantic search is an enhancement over keyword
//! search, so a database hiccup degrades to "semantic search off" rather than
//! breaking a search the operator is waiting on.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::model;

/// Bounds on `top_k`. Below 1 the feature does nothing; above 50 the extra
/// results are noise and the prompt budget suffers.
pub const MIN_TOP_K: i64 = 1;
pub const MAX_TOP_K: i64 = 50;
pub const DEFAULT_TOP_K: i64 = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingsSettings {
    pub enabled: bool,
    /// Model identifier. Only `all-MiniLM-L6-v2` ships today; the field exists so
    /// a second model can be added without a migration.
    pub model: String,
    pub top_k: i64,
}

impl Default for EmbeddingsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            model: model::MODEL_ID.to_string(),
            top_k: DEFAULT_TOP_K,
        }
    }
}

/// Clamps `top_k` into range and falls back to the shipped model on an unknown
/// name, so a hand-edited database cannot put the feature into a state the UI
/// cannot represent.
pub fn sanitize(mut settings: EmbeddingsSettings) -> EmbeddingsSettings {
    settings.top_k = settings.top_k.clamp(MIN_TOP_K, MAX_TOP_K);
    if settings.model.trim() != model::MODEL_ID {
        settings.model = model::MODEL_ID.to_string();
    }
    settings
}

pub async fn load(pool: &SqlitePool) -> EmbeddingsSettings {
    let row: Result<Option<(bool, String, i64)>, sqlx::Error> =
        sqlx::query_as("SELECT enabled, model, top_k FROM embeddings_settings WHERE id = 1")
            .fetch_optional(pool)
            .await;
    match row {
        Ok(Some((enabled, model, top_k))) => sanitize(EmbeddingsSettings {
            enabled,
            model,
            top_k,
        }),
        Ok(None) => {
            log::warn!("[Embeddings] settings row missing; using defaults");
            EmbeddingsSettings::default()
        }
        Err(e) => {
            log::warn!("[Embeddings] failed to read settings ({}); using defaults", e);
            EmbeddingsSettings::default()
        }
    }
}

pub async fn save(
    pool: &SqlitePool,
    settings: EmbeddingsSettings,
) -> Result<EmbeddingsSettings, String> {
    let settings = sanitize(settings);
    sqlx::query(
        "INSERT INTO embeddings_settings (id, enabled, model, top_k)
         VALUES (1, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
             enabled = excluded.enabled,
             model = excluded.model,
             top_k = excluded.top_k",
    )
    .bind(settings.enabled)
    .bind(&settings.model)
    .bind(settings.top_k)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to save semantic search settings: {}", e))?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_search_is_off_until_asked_for() {
        // The model is a 90 MB download; an upgrade must not start one silently.
        assert!(!EmbeddingsSettings::default().enabled);
    }

    #[test]
    fn top_k_is_clamped_into_a_usable_range() {
        let low = sanitize(EmbeddingsSettings {
            top_k: 0,
            ..Default::default()
        });
        assert_eq!(low.top_k, MIN_TOP_K);
        let high = sanitize(EmbeddingsSettings {
            top_k: 5_000,
            ..Default::default()
        });
        assert_eq!(high.top_k, MAX_TOP_K);
    }

    #[test]
    fn an_unknown_model_name_falls_back_to_the_shipped_one() {
        let settings = sanitize(EmbeddingsSettings {
            model: "some-model-we-do-not-have".to_string(),
            ..Default::default()
        });
        assert_eq!(settings.model, model::MODEL_ID);
    }
}
