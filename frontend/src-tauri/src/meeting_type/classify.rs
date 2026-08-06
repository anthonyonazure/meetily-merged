//! The classifier: one tight prompt to the user's already-configured summary
//! model, asking for a single label.
//!
//! It reuses `agents::runner::resolve_llm_settings` and
//! `summary::llm_client::generate_summary`, so it adds no network endpoints, no
//! new credentials, and no new provider handling. It also goes through the privacy
//! profile guard, so a client whose profile forbids cloud models does not get its
//! meetings classified by one.

use sqlx::SqlitePool;
use std::path::PathBuf;

use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::meeting_type::MeetingTypesRepository;
use crate::summary::llm_client::generate_summary;

use super::rules::{parse_reply, Classification, MeetingType, TypeSource};

/// How much transcript the classifier sees.
///
/// A meeting's kind is established in its first few minutes, and a small local
/// model given 40,000 characters will happily lose the question. Two thousand
/// characters is roughly the opening exchange, which is where the answer lives, and
/// it keeps the call fast enough to run automatically after every summary.
pub const CLASSIFIER_CONTEXT_CHARS: usize = 2_000;

/// The system prompt. Written to constrain the answer to one word, with the
/// vocabulary and its definitions inlined so the model does not have to guess what
/// "review" means here.
pub fn system_prompt() -> String {
    let mut prompt = String::from(
        "You classify business meetings into exactly one type. Answer with a single JSON object \
         and nothing else, in this shape:\n\
         {\"type\": \"<one of the labels below>\", \"confidence\": <0.0 to 1.0>}\n\n\
         The labels, and what each means:\n",
    );
    for kind in MeetingType::ALL {
        prompt.push_str(&format!("- {}: {}\n", kind.as_str(), kind.description()));
    }
    prompt.push_str(
        "\nRules: pick the single best label. Use \"other\" only when none of the others fit. \
         Set confidence to how sure you are. Do not explain. Do not add any text outside the JSON.",
    );
    prompt
}

/// The user prompt: the meeting's title and the opening of its transcript.
pub fn user_prompt(title: &str, transcript: &str) -> String {
    let excerpt: String = transcript.chars().take(CLASSIFIER_CONTEXT_CHARS).collect();
    format!(
        "Meeting title: {}\n\nOpening of the transcript:\n{}\n\nClassify this meeting.",
        title.trim(),
        excerpt.trim()
    )
}

/// Classifies one meeting and stores the result.
///
/// Returns None when the meeting has no transcript, when the profile forbids the
/// configured model, or when the model's reply contains no recognisable type. All
/// three are logged and none is an error the caller has to handle: an unclassified
/// meeting simply keeps whatever template the caller asked for.
pub async fn classify_meeting(
    pool: &SqlitePool,
    meeting_id: &str,
    model_provider: &str,
    model_name: &str,
    app_data_dir: Option<PathBuf>,
) -> Option<Classification> {
    // A person's correction is final; do not spend a model call to contradict it.
    if let Ok(Some(existing)) = MeetingTypesRepository::get(pool, meeting_id).await {
        if TypeSource::parse(&existing.source) == TypeSource::Manual {
            return MeetingType::parse(&existing.meeting_type).map(|meeting_type| Classification {
                meeting_type,
                confidence: existing.confidence,
                source: TypeSource::Manual,
            });
        }
    }

    let meeting = match MeetingsRepository::get_meeting_metadata(pool, meeting_id).await {
        Ok(Some(meeting)) => meeting,
        Ok(None) => return None,
        Err(e) => {
            log::warn!("[MeetingType] could not load meeting {}: {}", meeting_id, e);
            return None;
        }
    };

    let (transcripts, total) =
        match MeetingsRepository::get_meeting_transcripts_paginated(pool, meeting_id, 200, 0).await {
            Ok(result) => result,
            Err(e) => {
                log::warn!(
                    "[MeetingType] could not load transcripts for {}: {}",
                    meeting_id,
                    e
                );
                return None;
            }
        };
    if total == 0 {
        return None;
    }

    // Strict per-speaker consent and the profile's redaction both apply: the
    // classifier sees the same constrained copy every other model call does.
    let rows: Vec<(Option<String>, String)> = transcripts
        .iter()
        .map(|t| (t.speaker.clone(), t.transcript.clone()))
        .collect();
    let transcript = crate::consent::filter::speaker_prefixed_block(pool, meeting_id, &rows).await;

    let effective = match crate::profiles::enforce::guard_llm(
        pool,
        &crate::profiles::enforce::Scope::meeting(meeting_id),
        model_provider,
    )
    .await
    {
        Ok(effective) => effective,
        Err(e) => {
            log::info!(
                "[MeetingType] skipping classification for {}: {}",
                meeting_id,
                e
            );
            return None;
        }
    };
    let (transcript, _) = crate::profiles::enforce::redact_for(&effective, &transcript);

    // Polish before classifying: the fillers are noise to a small model working
    // from a short excerpt.
    let transcript = crate::polish::polish_block(&transcript);

    let settings = match crate::agents::runner::resolve_llm_settings(pool, model_provider).await {
        Ok(settings) => settings,
        Err(e) => {
            log::info!("[MeetingType] no usable model for classification: {}", e);
            return None;
        }
    };

    let client = reqwest::Client::new();
    let raw = match generate_summary(
        &client,
        &settings.provider,
        model_name,
        &settings.api_key,
        &system_prompt(),
        &user_prompt(&meeting.title, &transcript),
        settings.ollama_endpoint.as_deref(),
        settings.custom_openai_endpoint.as_deref(),
        settings.max_tokens,
        settings.temperature,
        settings.top_p,
        app_data_dir.as_ref(),
        None,
    )
    .await
    {
        Ok(raw) => raw,
        Err(e) => {
            log::warn!("[MeetingType] classification call failed for {}: {}", meeting_id, e);
            return None;
        }
    };

    let Some(classification) = parse_reply(&raw) else {
        log::warn!(
            "[MeetingType] could not read a type out of the model's reply for {}: {:?}",
            meeting_id,
            raw.chars().take(200).collect::<String>()
        );
        return None;
    };

    if let Err(e) = MeetingTypesRepository::set(
        pool,
        meeting_id,
        classification.meeting_type.as_str(),
        classification.confidence,
        classification.source.as_str(),
    )
    .await
    {
        log::warn!(
            "[MeetingType] could not store the classification for {}: {}",
            meeting_id,
            e
        );
    }

    log::info!(
        "[MeetingType] {} classified as {} ({:.2})",
        meeting_id,
        classification.meeting_type.as_str(),
        classification.confidence
    );
    Some(classification)
}

/// Reads a stored classification without calling a model.
pub async fn stored_classification(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Option<Classification> {
    let row = MeetingTypesRepository::get(pool, meeting_id).await.ok()??;
    MeetingType::parse(&row.meeting_type).map(|meeting_type| Classification {
        meeting_type,
        confidence: row.confidence,
        source: TypeSource::parse(&row.source),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_prompt_lists_every_label_with_its_meaning() {
        let prompt = system_prompt();
        for kind in MeetingType::ALL {
            assert!(prompt.contains(kind.as_str()), "missing {}", kind.as_str());
            assert!(prompt.contains(kind.description()));
        }
        assert!(prompt.contains("confidence"));
        assert!(prompt.contains("Do not explain"));
    }

    #[test]
    fn the_user_prompt_carries_the_title_and_bounds_the_transcript() {
        let long = "x".repeat(CLASSIFIER_CONTEXT_CHARS * 3);
        let prompt = user_prompt("  Q3 service review  ", &long);
        assert!(prompt.contains("Q3 service review"));
        assert!(!prompt.contains("  Q3"), "the title is trimmed");
        // The excerpt is capped, so a four-hour meeting is not sent in full.
        assert!(prompt.len() < CLASSIFIER_CONTEXT_CHARS + 300);
    }

    #[test]
    fn a_short_transcript_is_sent_whole() {
        let prompt = user_prompt("Standup", "We shipped the change.");
        assert!(prompt.contains("We shipped the change."));
    }
}
