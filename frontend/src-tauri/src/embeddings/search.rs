//! Hybrid retrieval: literal word matches and meaning matches, merged.
//!
//! Neither half is sufficient on its own. Keyword search finds an exact phrase a
//! semantic model would rank fifth; semantic search finds "when did we agree that
//! deadline" in a transcript where the client said "let's lock the go-live for the
//! 14th". So both run, results are deduplicated by their source row, and the score
//! components stay visible in the payload rather than being blended into one
//! opaque number.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::model;
use super::settings;
use super::store::{
    Candidate, EmbeddingsStore, KIND_MEMORY_FACT, KIND_SUMMARY, KIND_TRANSCRIPT_CHUNK,
};
use super::vector;

/// Score assigned to a hit found only by literal word match. There is no vector to
/// score it with, so it needs a fixed rank, and that rank has to sit inside the
/// actual similarity range of the model in use rather than a guessed one.
///
/// Measured on all-MiniLM-L6-v2 with real meeting sentences: a genuinely relevant
/// pair that shares almost no vocabulary scores about 0.31, a paraphrase about 0.29,
/// and an unrelated sentence 0.03 or below. Cosine similarities from this model are
/// compressed low; picking a round-sounding 0.5 would have put every word match above
/// every semantic match and quietly disabled the feature. So 0.25 — above the noise
/// floor, below a real semantic hit.
const KEYWORD_ONLY_SCORE: f32 = 0.25;

/// Nudge for a hit both halves agree on. Small: agreement is evidence, not proof.
const BOTH_AGREE_BONUS: f32 = 0.05;

/// Hard ceiling on how many indexed rows a single query will decode and scan.
/// Bounds the cost of the brute-force pass regardless of corpus size.
const MAX_CANDIDATES: i64 = 20_000;

/// How many keyword rows are considered before merging.
const KEYWORD_LIMIT: i64 = 100;

/// Where to look.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchScope {
    pub meeting_id: Option<String>,
    pub client_id: Option<String>,
    /// Only consider material indexed on or after this instant.
    pub since: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub source_kind: String,
    pub source_id: String,
    pub meeting_id: String,
    pub meeting_title: String,
    pub client_id: Option<String>,
    pub text: String,
    /// Cosine similarity, present only when the vector index produced this hit.
    pub semantic_score: Option<f32>,
    /// True when the query's words appear literally in the source.
    pub keyword_match: bool,
    /// The number the results are ordered by. Equal to `semantic_score` for a
    /// semantic hit, `KEYWORD_ONLY_SCORE` for a word-only hit, plus a small bonus
    /// when both halves agree.
    pub score: f32,
    /// `semantic` | `keyword` | `both`
    pub match_kind: String,
}

/// Results plus an honest account of what the index could and could not do.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    /// True when the semantic half actually ran.
    pub semantic_used: bool,
    /// Plain-English reason the semantic half did not run, when it did not.
    pub semantic_unavailable_reason: Option<String>,
    /// Meetings with a transcript that have nothing in the index. Non-zero means
    /// results are incomplete.
    pub unindexed_meetings: i64,
    /// Rows left over from a different embedding model.
    pub stale_rows: i64,
}

/// Runs both halves and merges them.
pub async fn hybrid(
    pool: &SqlitePool,
    query: &str,
    scope: &SearchScope,
    top_k: Option<i64>,
) -> Result<SearchResults, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(SearchResults {
            hits: Vec::new(),
            semantic_used: false,
            semantic_unavailable_reason: None,
            unindexed_meetings: 0,
            stale_rows: 0,
        });
    }

    let config = settings::load(pool).await;
    let limit = top_k.unwrap_or(config.top_k).clamp(1, settings::MAX_TOP_K);

    let counts = EmbeddingsStore::counts(pool, &config.model)
        .await
        .map_err(|e| format!("Failed to read index status: {}", e))?;
    let unindexed_meetings =
        (counts.meetings_with_transcripts - counts.meetings_indexed).max(0);

    let mut hits: Vec<SearchHit> = keyword_hits(pool, query, scope).await?;

    let (semantic_used, reason) = if !config.enabled {
        (false, Some("Semantic search is switched off in settings.".to_string()))
    } else if !model::files_present() {
        (
            false,
            Some("The embedding model has not been downloaded yet.".to_string()),
        )
    } else if counts.total == 0 {
        (
            false,
            Some("Nothing has been indexed yet, so only word matches are shown.".to_string()),
        )
    } else {
        match semantic_hits(pool, query, scope, &config.model, limit).await {
            Ok(semantic) => {
                merge(&mut hits, semantic);
                (true, None)
            }
            Err(e) => {
                log::warn!("[Embeddings] semantic pass failed, keyword results only: {}", e);
                (false, Some(format!("The semantic pass failed: {}", e)))
            }
        }
    };

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(limit as usize);

    Ok(SearchResults {
        hits,
        semantic_used,
        semantic_unavailable_reason: reason,
        unindexed_meetings,
        stale_rows: counts.rows_from_other_models,
    })
}

/// Folds semantic hits into the keyword list, marking overlaps as `both` rather
/// than listing the same passage twice.
fn merge(hits: &mut Vec<SearchHit>, semantic: Vec<SearchHit>) {
    for candidate in semantic {
        match hits.iter_mut().find(|existing| {
            existing.source_kind == candidate.source_kind
                && existing.source_id == candidate.source_id
        }) {
            Some(existing) => {
                existing.semantic_score = candidate.semantic_score;
                existing.keyword_match = true;
                existing.match_kind = "both".to_string();
                existing.score =
                    candidate.semantic_score.unwrap_or(KEYWORD_ONLY_SCORE) + BOTH_AGREE_BONUS;
                // Prefer the indexed passage text: it carries surrounding context
                // rather than one bare row.
                existing.text = candidate.text;
            }
            None => hits.push(candidate),
        }
    }
}

/// Top-k cosine over the narrowed candidate set.
async fn semantic_hits(
    pool: &SqlitePool,
    query: &str,
    scope: &SearchScope,
    model_id: &str,
    limit: i64,
) -> Result<Vec<SearchHit>, String> {
    let mut query_vectors = model::embed(vec![query.to_string()]).await?;
    let query_vector = query_vectors
        .pop()
        .ok_or_else(|| "The embedding model returned no vector for the query".to_string())?;

    let candidates = EmbeddingsStore::candidates(
        pool,
        model_id,
        scope.meeting_id.as_deref(),
        scope.client_id.as_deref(),
        scope.since,
        MAX_CANDIDATES,
    )
    .await
    .map_err(|e| format!("Failed to read the index: {}", e))?;

    let mut scored: Vec<(f32, Candidate)> = candidates
        .into_iter()
        .map(|candidate| (vector::cosine(&query_vector, &candidate.vector), candidate))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit as usize);

    let titles = meeting_titles(pool).await?;
    Ok(scored
        .into_iter()
        .map(|(score, candidate)| SearchHit {
            meeting_title: titles
                .iter()
                .find(|(id, _)| *id == candidate.meeting_id)
                .map(|(_, title)| title.clone())
                .unwrap_or_else(|| "(untitled meeting)".to_string()),
            source_kind: candidate.source_kind,
            source_id: candidate.source_id,
            meeting_id: candidate.meeting_id,
            client_id: candidate.client_id,
            text: candidate.chunk_text,
            semantic_score: Some(score),
            keyword_match: false,
            score,
            match_kind: "semantic".to_string(),
        })
        .collect())
}

/// The literal-word half: transcripts, summaries, and memory facts.
async fn keyword_hits(
    pool: &SqlitePool,
    query: &str,
    scope: &SearchScope,
) -> Result<Vec<SearchHit>, String> {
    // Wildcards are escaped so a query containing % or _ matches literally.
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{}%", escaped.to_lowercase());

    let mut sql = String::from(
        "SELECT t.id, t.meeting_id, m.title, t.transcript, t.speaker
         FROM transcripts t JOIN meetings m ON m.id = t.meeting_id
         WHERE LOWER(t.transcript) LIKE ? ESCAPE '\\'",
    );
    if scope.meeting_id.is_some() {
        sql.push_str(" AND t.meeting_id = ?");
    }
    if scope.client_id.is_some() {
        sql.push_str(
            " AND t.meeting_id IN (SELECT meeting_id FROM meeting_clients WHERE client_id = ?)",
        );
    }
    sql.push_str(" ORDER BY m.created_at DESC LIMIT ?");

    let mut rows = sqlx::query_as::<_, (String, String, String, String, Option<String>)>(&sql)
        .bind(&pattern);
    if let Some(id) = scope.meeting_id.as_deref() {
        rows = rows.bind(id);
    }
    if let Some(id) = scope.client_id.as_deref() {
        rows = rows.bind(id);
    }
    let rows = rows
        .bind(KEYWORD_LIMIT)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Keyword search failed: {}", e))?;

    // Strict per-speaker consent applies to search results exactly as it applies
    // to chat and exports: a withheld voice is not quotable through a search box.
    let mut hits = Vec::with_capacity(rows.len());
    for (id, meeting_id, title, text, speaker) in rows {
        let consent = crate::consent::filter::state_for_meeting(pool, &meeting_id).await;
        if consent.withholds(speaker.as_deref()) {
            continue;
        }
        let text = match speaker.as_deref().map(str::trim) {
            Some(label) if !label.is_empty() => format!("[{}] {}", label, text),
            _ => text,
        };
        hits.push(SearchHit {
            source_kind: KIND_TRANSCRIPT_CHUNK.to_string(),
            source_id: id,
            meeting_id,
            meeting_title: title,
            client_id: None,
            text,
            semantic_score: None,
            keyword_match: true,
            score: KEYWORD_ONLY_SCORE,
            match_kind: "keyword".to_string(),
        });
    }

    // Memory facts, through the existing repository search so there is one
    // definition of what a fact match means.
    let facts = crate::database::repositories::memory::MemoryFactsRepository::search(
        pool,
        query,
        scope.client_id.as_deref(),
        KEYWORD_LIMIT,
    )
    .await
    .map_err(|e| format!("Memory search failed: {}", e))?;
    for fact in facts {
        if scope
            .meeting_id
            .as_deref()
            .is_some_and(|id| id != fact.meeting_id)
        {
            continue;
        }
        hits.push(SearchHit {
            source_kind: KIND_MEMORY_FACT.to_string(),
            source_id: fact.id,
            meeting_id: fact.meeting_id,
            meeting_title: fact.meeting_title,
            client_id: fact.client_id,
            text: format!("{}: {} — {}", fact.kind, fact.subject, fact.detail),
            semantic_score: None,
            keyword_match: true,
            score: KEYWORD_ONLY_SCORE,
            match_kind: "keyword".to_string(),
        });
    }

    Ok(hits)
}

async fn meeting_titles(pool: &SqlitePool) -> Result<Vec<(String, String)>, String> {
    sqlx::query_as::<_, (String, String)>("SELECT id, title FROM meetings")
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Failed to read meeting titles: {}", e))
}

/// Retrieval for prompt building: the passages most relevant to a question, as
/// ready-to-paste text blocks with their provenance.
///
/// This is what lets the chat context builders stop dumping the whole corpus into
/// a prompt once the corpus is large.
pub async fn retrieve_context(
    pool: &SqlitePool,
    question: &str,
    scope: &SearchScope,
    top_k: i64,
) -> Result<Vec<String>, String> {
    let results = hybrid(pool, question, scope, Some(top_k)).await?;
    if !results.semantic_used {
        // Without a working semantic pass the keyword hits are too narrow to
        // ground an answer, so the caller keeps its existing behaviour.
        return Ok(Vec::new());
    }
    Ok(results
        .hits
        .into_iter()
        .map(|hit| {
            let label = match hit.source_kind.as_str() {
                KIND_SUMMARY => "summary",
                KIND_MEMORY_FACT => "memory fact",
                _ => "transcript",
            };
            format!(
                "### From \"{}\" ({}, relevance {:.2})\n{}",
                hit.meeting_title, label, hit.score, hit.text
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(kind: &str, id: &str, semantic: Option<f32>) -> SearchHit {
        SearchHit {
            source_kind: kind.to_string(),
            source_id: id.to_string(),
            meeting_id: "m1".to_string(),
            meeting_title: "Standup".to_string(),
            client_id: None,
            text: format!("text for {}", id),
            semantic_score: semantic,
            keyword_match: semantic.is_none(),
            score: semantic.unwrap_or(KEYWORD_ONLY_SCORE),
            match_kind: if semantic.is_some() {
                "semantic".to_string()
            } else {
                "keyword".to_string()
            },
        }
    }

    #[test]
    fn a_passage_both_halves_found_is_listed_once_and_marked_both() {
        let mut hits = vec![hit(KIND_TRANSCRIPT_CHUNK, "t1", None)];
        merge(&mut hits, vec![hit(KIND_TRANSCRIPT_CHUNK, "t1", Some(0.8))]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].match_kind, "both");
        assert!(hits[0].keyword_match);
        assert_eq!(hits[0].semantic_score, Some(0.8));
        assert!((hits[0].score - (0.8 + BOTH_AGREE_BONUS)).abs() < 1e-6);
    }

    #[test]
    fn a_semantic_only_passage_is_appended_rather_than_merged() {
        let mut hits = vec![hit(KIND_TRANSCRIPT_CHUNK, "t1", None)];
        merge(&mut hits, vec![hit(KIND_TRANSCRIPT_CHUNK, "t2", Some(0.7))]);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[1].match_kind, "semantic");
        assert!(!hits[1].keyword_match);
    }

    #[test]
    fn the_same_source_id_in_different_kinds_is_not_treated_as_a_duplicate() {
        let mut hits = vec![hit(KIND_TRANSCRIPT_CHUNK, "shared", None)];
        merge(&mut hits, vec![hit(KIND_MEMORY_FACT, "shared", Some(0.9))]);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn merging_prefers_the_indexed_passage_text_for_its_context() {
        let mut hits = vec![hit(KIND_TRANSCRIPT_CHUNK, "t1", None)];
        let mut semantic = hit(KIND_TRANSCRIPT_CHUNK, "t1", Some(0.6));
        semantic.text = "the whole passage with context".to_string();
        merge(&mut hits, vec![semantic]);
        assert_eq!(hits[0].text, "the whole passage with context");
    }

    #[test]
    fn the_keyword_baseline_sits_inside_the_models_real_similarity_range() {
        // Sanity bounds taken from measured all-MiniLM-L6-v2 behaviour: an unrelated
        // sentence pair scores about 0.03, a relevant one about 0.31. The baseline has
        // to be above the first and below the second, or one half of the hybrid
        // silently wins everything.
        assert!(KEYWORD_ONLY_SCORE > 0.05, "a word match must beat pure noise");
        assert!(
            KEYWORD_ONLY_SCORE < 0.28,
            "a word match must not outrank a genuine semantic hit"
        );
    }

    #[test]
    fn agreement_between_the_halves_moves_a_hit_up_but_not_far() {
        let mut hits = vec![hit(KIND_TRANSCRIPT_CHUNK, "t1", None)];
        merge(&mut hits, vec![hit(KIND_TRANSCRIPT_CHUNK, "t1", Some(0.30))]);
        let agreed = hits[0].score;
        // Above the semantic score alone, but nowhere near enough to leapfrog a much
        // stronger semantic-only hit.
        assert!(agreed > 0.30);
        assert!(agreed < 0.40);
    }
}
