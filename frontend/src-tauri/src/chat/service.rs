//! Context building and background execution for meeting chat.

use crate::agents::runner::resolve_llm_settings;
use crate::database::models::ChatMessageRecord;
use crate::database::repositories::{
    chat::{ChatMessagesRepository, ChatScope},
    client::{ClientsRepository, MeetingClientsRepository},
    meeting::MeetingsRepository,
    memory::MemoryFactsRepository,
    summary::SummaryProcessesRepository,
};
use crate::summary::llm_client::generate_summary;
use serde::Serialize;
use sqlx::SqlitePool;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Runtime};
use tracing::{error, info};

/// Event emitted when a background chat request finishes (success or failure).
/// The frontend also polls `chat_history` as a fallback, so the stored message
/// is the source of truth and this event is only a wake-up call.
pub const CHAT_RESPONSE_EVENT: &str = "chat-response";

/// Transcripts larger than this many characters are truncated from the middle
/// (head + tail kept) before being placed in the prompt.
const TRANSCRIPT_MAX_CHARS: usize = 26_000;
const TRANSCRIPT_HEAD_CHARS: usize = 16_000;
const TRANSCRIPT_TAIL_CHARS: usize = 10_000;

/// Per-meeting summary budget in all-meetings mode.
const ALL_MEETINGS_SUMMARY_CHARS: usize = 2_000;
/// How many recent meetings the all-meetings scope includes.
const ALL_MEETINGS_LIMIT: usize = 20;
/// How many of the client's recent meetings the client scope includes.
const CLIENT_MEETINGS_LIMIT: usize = 15;

/// How many prior chat turns are replayed into the prompt for continuity.
const HISTORY_TURNS: usize = 12;

/// Above this many characters of grounding context, retrieval replaces the dump.
///
/// Below it, handing the model everything is both cheaper and better: there is no
/// haystack to search. Above it the prompt starts crowding out the question, and a
/// semantic pass that keeps the twelve most relevant passages beats twenty
/// truncated summaries. The threshold is a size, not a meeting count, because one
/// long meeting can outweigh ten short ones.
const RETRIEVAL_THRESHOLD_CHARS: usize = 24_000;

/// How many passages retrieval contributes when it takes over.
const RETRIEVAL_TOP_K: i64 = 14;

/// Replaces a large grounding context with the passages most relevant to the
/// question, when retrieval is available and the context is genuinely large.
///
/// Returns the original context unchanged whenever semantic search is off, not yet
/// indexed, or finds nothing — so this can only ever improve on the dump, never
/// leave the model with less to work from than before.
async fn narrow_context_if_large(
    pool: &SqlitePool,
    context: String,
    question: &str,
    scope: crate::embeddings::search::SearchScope,
) -> String {
    if context.chars().count() <= RETRIEVAL_THRESHOLD_CHARS {
        return context;
    }
    match crate::embeddings::search::retrieve_context(pool, question, &scope, RETRIEVAL_TOP_K).await
    {
        Ok(passages) if !passages.is_empty() => {
            info!(
                "Chat context narrowed by retrieval: {} chars dumped -> {} relevant passage(s)",
                context.chars().count(),
                passages.len()
            );
            format!(
                "The most relevant passages from the recorded meetings, found by searching for this question. This is a selection, not everything on record.\n\n{}",
                passages.join("\n\n")
            )
        }
        Ok(_) => context,
        Err(e) => {
            log::warn!("Chat retrieval failed, keeping the full context: {}", e);
            context
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatResponsePayload {
    /// Meeting scope the response belongs to, when any.
    pub meeting_id: Option<String>,
    /// Client scope the response belongs to, when any. Both None = the
    /// all-meetings thread.
    pub client_id: Option<String>,
    /// The stored assistant message (also readable via `chat_history`).
    pub message: ChatMessageRecord,
    /// True when the content is an error report rather than an answer.
    pub is_error: bool,
}

const SYSTEM_PROMPT_SINGLE: &str = "You are an assistant that answers questions about one recorded meeting. \
Answer ONLY from the meeting content provided (title, summary, transcript). \
If the answer is not in the provided content, say so plainly instead of guessing. \
When speaker labels like [You] or [Speaker 2] appear in the transcript, cite them when attributing statements. \
Be concise and answer in plain English markdown.";

const SYSTEM_PROMPT_ALL: &str = "You are an assistant that answers questions across a user's recent recorded meetings. \
You are given each meeting's title, date, and summary. Answer ONLY from that provided content. \
If the answer is not in the provided content, say so plainly instead of guessing; \
mention which meeting(s) your answer comes from. \
Be concise and answer in plain English markdown.";

const SYSTEM_PROMPT_CLIENT: &str = "You are an assistant that answers questions about one client relationship, \
built from the user's recorded meetings with that client. \
You are given the client's recent meetings (title, date, summary) and their memory facts \
(commitments with status, decisions, figures, notes). Answer ONLY from that provided content. \
If the answer is not in the provided content, say so plainly instead of guessing; \
mention which meeting or fact your answer comes from. \
Be concise and answer in plain English markdown.";

/// Truncates a transcript from the middle, keeping head and tail, and notes
/// the truncation inline so the model knows content is missing.
fn truncate_transcript(transcript: &str) -> String {
    if transcript.chars().count() <= TRANSCRIPT_MAX_CHARS {
        return transcript.to_string();
    }
    let chars: Vec<char> = transcript.chars().collect();
    let head: String = chars[..TRANSCRIPT_HEAD_CHARS].iter().collect();
    let tail: String = chars[chars.len() - TRANSCRIPT_TAIL_CHARS..].iter().collect();
    let omitted = chars.len() - TRANSCRIPT_HEAD_CHARS - TRANSCRIPT_TAIL_CHARS;
    format!(
        "{}\n\n[NOTE: transcript truncated here; about {} characters from the middle of the meeting were omitted. The transcript resumes near the end of the meeting.]\n\n{}",
        head, omitted, tail
    )
}

/// Extracts the summary markdown from a stored summary process row.
///
/// `pub(crate)` because the semantic indexer needs exactly the same extraction, and
/// two copies of "where the markdown lives inside that JSON" would drift.
pub(crate) fn summary_markdown_from_result(raw: Option<&str>) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw?).ok()?;
    let markdown = value.get("markdown")?.as_str()?.trim();
    (!markdown.is_empty()).then(|| markdown.to_string())
}

/// Builds the grounding context for a single-meeting question.
async fn build_single_meeting_context(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<String, String> {
    let meeting = MeetingsRepository::get_meeting_metadata(pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to load meeting: {}", e))?
        .ok_or_else(|| format!("Meeting not found: {}", meeting_id))?;

    let (transcripts, total) =
        MeetingsRepository::get_meeting_transcripts_paginated(pool, meeting_id, i64::MAX, 0)
            .await
            .map_err(|e| format!("Failed to load transcripts: {}", e))?;
    if total == 0 {
        return Err("This meeting has no transcript yet".to_string());
    }

    // Strict per-speaker consent withholds unconsented speakers' text from the
    // chat grounding context, so an unconsented voice cannot be quoted back.
    let rows: Vec<(Option<String>, String)> = transcripts
        .iter()
        .map(|t| (t.speaker.clone(), t.transcript.clone()))
        .collect();
    let transcript =
        crate::consent::filter::speaker_prefixed_block(pool, meeting_id, &rows).await;

    let summary = match SummaryProcessesRepository::get_summary_data(pool, meeting_id).await {
        Ok(Some(process)) => summary_markdown_from_result(process.result.as_deref()),
        _ => None,
    };

    let mut context = format!("Meeting title: {}\n\n", meeting.title);
    if let Some(summary) = summary {
        context.push_str("Meeting summary:\n");
        context.push_str(&summary);
        context.push_str("\n\n");
    }
    context.push_str("Meeting transcript:\n");
    context.push_str(&truncate_transcript(&transcript));
    Ok(context)
}

/// Builds the grounding context for the all-meetings scope: recent meeting
/// titles, dates, and summaries (no transcripts, to stay within budget).
async fn build_all_meetings_context(pool: &SqlitePool) -> Result<String, String> {
    let meetings = MeetingsRepository::get_meetings(pool)
        .await
        .map_err(|e| format!("Failed to list meetings: {}", e))?;
    if meetings.is_empty() {
        return Err("There are no recorded meetings yet".to_string());
    }

    let mut context = String::from("Recent meetings (newest first):\n\n");
    for meeting in meetings.iter().take(ALL_MEETINGS_LIMIT) {
        let date = meeting.created_at.0.format("%Y-%m-%d %H:%M UTC");
        context.push_str(&format!("### {} ({})\n", meeting.title, date));
        let summary = match SummaryProcessesRepository::get_summary_data(pool, &meeting.id).await {
            Ok(Some(process)) => summary_markdown_from_result(process.result.as_deref()),
            _ => None,
        };
        match summary {
            Some(md) => {
                let clipped: String = md.chars().take(ALL_MEETINGS_SUMMARY_CHARS).collect();
                context.push_str(&clipped);
                if md.chars().count() > ALL_MEETINGS_SUMMARY_CHARS {
                    context.push_str("\n[summary truncated]");
                }
            }
            None => context.push_str("(no summary generated)"),
        }
        context.push_str("\n\n");
    }
    Ok(context)
}

/// Builds the grounding context for the client scope: the client's most
/// recent meetings (titles, dates, summaries) plus all their memory facts
/// except dismissed ones.
async fn build_client_context(pool: &SqlitePool, client_id: &str) -> Result<String, String> {
    let client = ClientsRepository::get(pool, client_id)
        .await
        .map_err(|e| format!("Failed to load client: {}", e))?
        .ok_or_else(|| "Client not found".to_string())?;

    let meetings = MeetingClientsRepository::meetings_for_client(pool, client_id)
        .await
        .map_err(|e| format!("Failed to load client meetings: {}", e))?;
    if meetings.is_empty() {
        return Err(format!(
            "No meetings are tagged with {} yet",
            client.name
        ));
    }

    let mut context = format!("Client: {}\n", client.name);
    if let Some(domain) = client.domain.as_deref() {
        context.push_str(&format!("Email domain: {}\n", domain));
    }
    if !client.notes.trim().is_empty() {
        context.push_str(&format!("Notes: {}\n", client.notes.trim()));
    }

    context.push_str("\nRecent meetings with this client (newest first):\n\n");
    for meeting in meetings.iter().take(CLIENT_MEETINGS_LIMIT) {
        let date = meeting.created_at.0.format("%Y-%m-%d %H:%M UTC");
        context.push_str(&format!("### {} ({})\n", meeting.title, date));
        let summary = match SummaryProcessesRepository::get_summary_data(pool, &meeting.id).await {
            Ok(Some(process)) => summary_markdown_from_result(process.result.as_deref()),
            _ => None,
        };
        match summary {
            Some(md) => {
                let clipped: String = md.chars().take(ALL_MEETINGS_SUMMARY_CHARS).collect();
                context.push_str(&clipped);
                if md.chars().count() > ALL_MEETINGS_SUMMARY_CHARS {
                    context.push_str("\n[summary truncated]");
                }
            }
            None => context.push_str("(no summary generated)"),
        }
        context.push_str("\n\n");
    }

    let facts = MemoryFactsRepository::for_client(pool, client_id)
        .await
        .map_err(|e| format!("Failed to load client memory: {}", e))?;
    let facts: Vec<_> = facts
        .into_iter()
        .filter(|fact| fact.status != "dismissed")
        .collect();
    if !facts.is_empty() {
        context.push_str("Client memory facts:\n");
        for fact in facts {
            let status = match fact.status.as_str() {
                "na" => String::new(),
                other => format!(", {}", other),
            };
            let mut line = format!(
                "- [{}{}] {} — {}",
                fact.kind, status, fact.subject, fact.detail
            );
            if let Some(owner) = fact.owner.as_deref() {
                line.push_str(&format!(" (owner: {})", owner));
            }
            if let Some(due) = fact.due_hint.as_deref() {
                line.push_str(&format!(" (due: {})", due));
            }
            if let Some(amount) = fact.amount.as_deref() {
                line.push_str(&format!(" ({})", amount));
            }
            line.push_str(&format!(" (from \"{}\")", fact.meeting_title));
            context.push_str(&line);
            context.push('\n');
        }
    }
    Ok(context)
}

/// Assembles the user prompt: grounding context, recent conversation, and the
/// new question. History goes into the user prompt because the shared
/// `generate_summary` plumbing takes a single system + user message pair.
fn build_user_prompt(context: &str, history: &[ChatMessageRecord], question: &str) -> String {
    let mut prompt = String::from("Meeting content to answer from:\n\n");
    prompt.push_str(context);
    prompt.push_str("\n\n");

    let relevant: Vec<&ChatMessageRecord> = history
        .iter()
        .rev()
        .take(HISTORY_TURNS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if !relevant.is_empty() {
        prompt.push_str("Conversation so far:\n");
        for message in relevant {
            let speaker = if message.role == "user" { "User" } else { "Assistant" };
            prompt.push_str(&format!("{}: {}\n", speaker, message.content));
        }
        prompt.push('\n');
    }

    prompt.push_str("New question: ");
    prompt.push_str(question);
    prompt
}

/// Executes one chat request in the background: builds context, calls the
/// configured LLM, persists the assistant message, and emits
/// `chat-response`. Failures are persisted as an assistant message too, so the
/// poll fallback surfaces them without special casing.
pub async fn execute_chat_request<R: Runtime>(
    app: AppHandle<R>,
    pool: SqlitePool,
    scope: ChatScope,
    question: String,
    model_provider: String,
    model_name: String,
    app_data_dir: Option<PathBuf>,
) {
    info!(
        "Chat request started: scope={}, provider={}, model={}",
        scope.label(),
        model_provider,
        model_name
    );

    let outcome = run_chat_llm(
        &pool,
        &scope,
        &question,
        &model_provider,
        &model_name,
        app_data_dir,
    )
    .await;

    let (content, is_error) = match outcome {
        Ok(answer) => (answer, false),
        Err(e) => {
            error!("Chat request failed (scope={}): {}", scope.label(), e);
            (format!("The model request failed: {}", e), true)
        }
    };

    match ChatMessagesRepository::insert(&pool, &scope, "assistant", &content).await {
        Ok(record) => {
            let payload = ChatResponsePayload {
                meeting_id: scope.meeting_id().map(str::to_string),
                client_id: scope.client_id().map(str::to_string),
                message: record,
                is_error,
            };
            if let Err(e) = app.emit(CHAT_RESPONSE_EVENT, &payload) {
                error!("Failed to emit chat-response event: {}", e);
            }
        }
        Err(e) => error!(
            "Failed to persist chat response (scope={}): {}",
            scope.label(),
            e
        ),
    }
}

async fn run_chat_llm(
    pool: &SqlitePool,
    scope: &ChatScope,
    question: &str,
    model_provider: &str,
    model_name: &str,
    app_data_dir: Option<PathBuf>,
) -> Result<String, String> {
    // Privacy profile: a meeting or client chat is governed by that meeting's or
    // client's profile; the all-meetings thread falls back to the workspace
    // default.
    let profile_scope = match scope {
        ChatScope::Meeting(id) => crate::profiles::enforce::Scope::meeting(id.clone()),
        ChatScope::Client(id) => crate::profiles::enforce::Scope::client(id.clone()),
        ChatScope::All => crate::profiles::enforce::Scope::Workspace,
    };
    let effective =
        crate::profiles::enforce::guard_llm(pool, &profile_scope, model_provider).await?;

    let settings = resolve_llm_settings(pool, model_provider).await?;

    // History is read before the new user message was... no: the command
    // inserts the user message first, so drop the trailing user turn (it is
    // passed separately as the question).
    let mut history = ChatMessagesRepository::history(pool, scope)
        .await
        .map_err(|e| format!("Failed to load chat history: {}", e))?;
    if history
        .last()
        .map(|m| m.role == "user" && m.content == question)
        .unwrap_or(false)
    {
        history.pop();
    }

    // All three scopes build their context the same way as before, then hand it to
    // retrieval. Retrieval only takes over when the context is large enough that
    // dumping it is the worse option, so a single short meeting is unaffected.
    let (system_prompt, context) = match scope {
        ChatScope::Meeting(id) => {
            let context = build_single_meeting_context(pool, id).await?;
            let scope = crate::embeddings::search::SearchScope {
                meeting_id: Some(id.clone()),
                client_id: None,
                since: None,
            };
            (
                SYSTEM_PROMPT_SINGLE,
                narrow_context_if_large(pool, context, question, scope).await,
            )
        }
        ChatScope::Client(id) => {
            let context = build_client_context(pool, id).await?;
            let scope = crate::embeddings::search::SearchScope {
                meeting_id: None,
                client_id: Some(id.clone()),
                since: None,
            };
            (
                SYSTEM_PROMPT_CLIENT,
                narrow_context_if_large(pool, context, question, scope).await,
            )
        }
        ChatScope::All => {
            let context = build_all_meetings_context(pool).await?;
            (
                SYSTEM_PROMPT_ALL,
                narrow_context_if_large(
                    pool,
                    context,
                    question,
                    crate::embeddings::search::SearchScope::default(),
                )
                .await,
            )
        }
    };

    let user_prompt = build_user_prompt(&context, &history, question);
    // Mask obvious secrets in the copy handed to the model.
    let (user_prompt, _) = crate::profiles::enforce::redact_for(&effective, &user_prompt);

    let client = reqwest::Client::new();
    let llm_call = generate_summary(
        &client,
        &settings.provider,
        model_name,
        &settings.api_key,
        system_prompt,
        &user_prompt,
        settings.ollama_endpoint.as_deref(),
        settings.custom_openai_endpoint.as_deref(),
        settings.max_tokens,
        settings.temperature,
        settings.top_p,
        app_data_dir.as_ref(),
        None,
    );
    // Attribute the request to the meeting when the question is about one, so the
    // per-meeting network answer covers chat as well as summaries.
    let raw = match scope.meeting_id() {
        Some(meeting_id) => crate::network::with_meeting(meeting_id, llm_call).await?,
        None => llm_call.await?,
    };

    Ok(crate::summary::processor::clean_llm_markdown_output(&raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn record(role: &str, content: &str) -> ChatMessageRecord {
        ChatMessageRecord {
            id: format!("chatmsg-{}", content.len()),
            meeting_id: None,
            client_id: None,
            role: role.to_string(),
            content: content.to_string(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn short_transcripts_pass_through_untouched() {
        let text = "hello world";
        assert_eq!(truncate_transcript(text), text);
    }

    #[test]
    fn long_transcripts_keep_head_and_tail_with_a_note() {
        let text = "a".repeat(TRANSCRIPT_MAX_CHARS + 5_000);
        let result = truncate_transcript(&text);
        assert!(result.contains("transcript truncated"));
        assert!(result.len() < text.len());
        assert!(result.starts_with('a'));
        assert!(result.ends_with('a'));
    }

    #[test]
    fn prompt_includes_context_history_and_question() {
        let history = vec![record("user", "Who attended?"), record("assistant", "You and Dana.")];
        let prompt = build_user_prompt("CONTEXT", &history, "What was decided?");
        assert!(prompt.contains("CONTEXT"));
        assert!(prompt.contains("User: Who attended?"));
        assert!(prompt.contains("Assistant: You and Dana."));
        assert!(prompt.contains("New question: What was decided?"));
    }

    #[test]
    fn prompt_without_history_skips_the_conversation_block() {
        let prompt = build_user_prompt("CONTEXT", &[], "Anything?");
        assert!(!prompt.contains("Conversation so far"));
    }

    #[test]
    fn summary_markdown_is_read_from_result_json() {
        assert_eq!(
            summary_markdown_from_result(Some("{\"markdown\":\"# Hi\"}")).as_deref(),
            Some("# Hi")
        );
        assert_eq!(summary_markdown_from_result(Some("not json")), None);
        assert_eq!(summary_markdown_from_result(None), None);
    }
}
