//! Agent runner: builds meeting context, calls the same LLM plumbing the
//! summary feature uses (`summary::llm_client::generate_summary`), parses the
//! output, and persists runs and action items.
//!
//! All processing goes through the user's configured summary provider. The
//! runner adds no network endpoints and no telemetry of its own.

use crate::agents::registry::{self, AgentContext, AgentDefinition, AgentOutputKind};
use crate::database::repositories::{
    agent::{ActionItemsRepository, AgentRunsRepository, AgentSettingsRepository},
    client::MeetingClientsRepository,
    meeting::MeetingsRepository,
    memory::MemoryFactsRepository,
    setting::SettingsRepository,
    summary::SummaryProcessesRepository,
};
use crate::summary::llm_client::{generate_summary, LLMProvider};
use serde::Deserialize;
use sqlx::SqlitePool;
use std::path::PathBuf;
use tracing::{error, info, warn};

/// Everything needed to talk to the configured LLM provider.
/// Shared with the chat feature (`crate::chat`), which resolves providers the
/// same way agents do.
pub(crate) struct LlmSettings {
    pub(crate) provider: LLMProvider,
    pub(crate) api_key: String,
    pub(crate) ollama_endpoint: Option<String>,
    pub(crate) custom_openai_endpoint: Option<String>,
    pub(crate) max_tokens: Option<u32>,
    pub(crate) temperature: Option<f32>,
    pub(crate) top_p: Option<f32>,
}

/// Resolves provider credentials/endpoints from the settings table, mirroring
/// the resolution done for summary generation in `summary::service`.
pub(crate) async fn resolve_llm_settings(
    pool: &SqlitePool,
    model_provider: &str,
) -> Result<LlmSettings, String> {
    let provider = LLMProvider::from_str(model_provider)?;

    let api_key = if provider == LLMProvider::Ollama
        || provider == LLMProvider::BuiltInAI
        || provider == LLMProvider::CustomOpenAI
    {
        String::new()
    } else {
        match SettingsRepository::get_api_key(pool, model_provider).await {
            Ok(Some(key)) if !key.is_empty() => key,
            Ok(_) => return Err(format!("API key not found for {}", model_provider)),
            Err(e) => {
                return Err(format!(
                    "Failed to retrieve API key for {}: {}",
                    model_provider, e
                ))
            }
        }
    };

    let ollama_endpoint = if provider == LLMProvider::Ollama {
        match SettingsRepository::get_model_config(pool).await {
            Ok(Some(config)) => config.ollama_endpoint,
            _ => None,
        }
    } else {
        None
    };

    let (custom_openai_endpoint, custom_api_key, max_tokens, temperature, top_p) =
        if provider == LLMProvider::CustomOpenAI {
            match SettingsRepository::get_custom_openai_config(pool).await {
                Ok(Some(config)) => (
                    Some(config.endpoint),
                    config.api_key,
                    config.max_tokens.map(|t| t as u32),
                    config.temperature,
                    config.top_p,
                ),
                Ok(None) => {
                    return Err(
                        "Custom OpenAI provider selected but no configuration found".to_string()
                    )
                }
                Err(e) => return Err(format!("Failed to retrieve custom OpenAI config: {}", e)),
            }
        } else {
            (None, None, None, None, None)
        };

    let final_api_key = if provider == LLMProvider::CustomOpenAI {
        custom_api_key.unwrap_or_default()
    } else {
        api_key
    };

    Ok(LlmSettings {
        provider,
        api_key: final_api_key,
        ollama_endpoint,
        custom_openai_endpoint,
        max_tokens,
        temperature,
        top_p,
    })
}

/// Builds the meeting context (title, transcript, summary markdown, client)
/// an agent prompt operates on.
async fn build_agent_context(
    pool: &SqlitePool,
    meeting_id: &str,
    agent: &AgentDefinition,
) -> Result<AgentContext, String> {
    let meeting = MeetingsRepository::get_meeting_metadata(pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to load meeting: {}", e))?
        .ok_or_else(|| format!("Meeting not found: {}", meeting_id))?;

    // i64::MAX effectively means "all segments"; agents need the full meeting.
    let (transcripts, total) =
        MeetingsRepository::get_meeting_transcripts_paginated(pool, meeting_id, i64::MAX, 0)
            .await
            .map_err(|e| format!("Failed to load transcripts: {}", e))?;

    // Client-scoped agents work from the client's memory, not the transcript,
    // so an empty transcript only blocks transcript-driven agents.
    if total == 0 && !agent.needs_client {
        return Err("This meeting has no transcript yet".to_string());
    }

    // Strict per-speaker consent withholds unconsented speakers' text from the
    // agent's context, so agent output cannot be derived from it.
    let rows: Vec<(Option<String>, String)> = transcripts
        .iter()
        .map(|t| (t.speaker.clone(), t.transcript.clone()))
        .collect();
    let transcript =
        crate::consent::filter::speaker_prefixed_block(pool, meeting_id, &rows).await;

    let summary_markdown = match SummaryProcessesRepository::get_summary_data(pool, meeting_id).await
    {
        Ok(Some(process)) => process.result.and_then(|raw| {
            serde_json::from_str::<serde_json::Value>(&raw)
                .ok()
                .and_then(|value| {
                    value
                        .get("markdown")
                        .and_then(|m| m.as_str())
                        .map(str::to_string)
                })
        }),
        _ => None,
    };

    let client = MeetingClientsRepository::client_for_meeting(pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to read meeting client: {}", e))?;

    let (client_name, client_commitments) = if agent.needs_client {
        let client = client.ok_or_else(|| {
            format!("Tag this meeting with a client to run {}", agent.name)
        })?;
        let facts = MemoryFactsRepository::open_commitments_for_client(pool, &client.id, 0)
            .await
            .map_err(|e| format!("Failed to load open commitments: {}", e))?;
        let stale = crate::clients::follow_through::stale_commitments(facts, chrono::Utc::now());
        if stale.is_empty() {
            return Err(format!(
                "{} has no open commitments ready to chase (older than {} days or overdue)",
                client.name,
                crate::clients::follow_through::CHASE_MIN_AGE_DAYS
            ));
        }
        (
            Some(client.name),
            Some(crate::clients::follow_through::commitments_block(&stale)),
        )
    } else {
        (client.map(|c| c.name), None)
    };

    Ok(AgentContext {
        meeting_title: meeting.title,
        transcript,
        summary_markdown,
        client_name,
        client_commitments,
    })
}

/// One parsed Action Tracker item. Tolerant of near-miss key names.
#[derive(Debug, Deserialize)]
struct ParsedActionItem {
    #[serde(alias = "task", alias = "action", alias = "item")]
    description: String,
    #[serde(default, alias = "assignee", alias = "who")]
    owner: Option<String>,
    #[serde(default, alias = "due", alias = "due_date", alias = "deadline")]
    due_hint: Option<String>,
}

/// Tolerantly parses the LLM output into action items: strips code fences and
/// surrounding prose, then parses the first top-level JSON array found.
/// Returns None when no parsable array exists.
fn parse_action_items(raw: &str) -> Option<Vec<ParsedActionItem>> {
    let cleaned = crate::summary::processor::clean_llm_markdown_output(raw);
    let candidate = extract_first_json_array(&cleaned)
        .or_else(|| extract_first_json_array(raw))?;
    serde_json::from_str::<Vec<ParsedActionItem>>(&candidate)
        .ok()
        .map(|items| {
            items
                .into_iter()
                .filter(|item| !item.description.trim().is_empty())
                .collect()
        })
}

/// Extracts the first balanced top-level `[ ... ]` block, respecting strings.
/// Also used by the follow-through command (`crate::clients::follow_through`).
pub(crate) fn extract_first_json_array(text: &str) -> Option<String> {
    let start = text.find('[')?;
    let bytes = text.as_bytes();
    let mut depth: i64 = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, &byte) in bytes[start..].iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[start..start + offset + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// One parsed Client Memory fact. Tolerant of near-miss key names and value
/// shapes; elements that cannot be salvaged are skipped individually rather
/// than failing the whole array.
#[derive(Debug, PartialEq)]
pub(crate) struct ParsedMemoryFact {
    pub(crate) kind: String,
    pub(crate) subject: String,
    pub(crate) detail: String,
    pub(crate) owner: Option<String>,
    pub(crate) due_hint: Option<String>,
    pub(crate) amount: Option<String>,
}

const MEMORY_FACT_KINDS: &[&str] = &["commitment", "decision", "figure", "note"];

fn value_to_trimmed_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn first_string(object: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| value_to_trimmed_string(&object[*key]))
}

/// Converts one JSON element into a fact, or None when it has no substance.
fn memory_fact_from_value(value: &serde_json::Value) -> Option<ParsedMemoryFact> {
    if !value.is_object() {
        return None;
    }
    let kind_raw = first_string(value, &["kind", "type", "category"])
        .unwrap_or_default()
        .to_lowercase();
    // Tolerate plural/verbose kinds ("commitments", "decision made"); unknown
    // kinds degrade to "note" instead of dropping the fact.
    let kind = MEMORY_FACT_KINDS
        .iter()
        .find(|k| kind_raw.starts_with(**k))
        .copied()
        .unwrap_or("note")
        .to_string();

    let subject = first_string(value, &["subject", "title", "topic"]);
    let detail = first_string(value, &["detail", "description", "text", "content"]);
    let (subject, detail) = match (subject, detail) {
        (Some(s), Some(d)) => (s, d),
        // Only one of the two present: use it for both label and substance.
        (Some(s), None) => (s.clone(), s),
        (None, Some(d)) => {
            let label: String = d.chars().take(80).collect();
            (label, d)
        }
        (None, None) => return None,
    };

    Some(ParsedMemoryFact {
        kind,
        subject,
        detail,
        owner: first_string(value, &["owner", "assignee", "who"]),
        due_hint: first_string(value, &["due_hint", "due", "due_date", "deadline"]),
        amount: first_string(value, &["amount", "value", "figure"]),
    })
}

/// Tolerantly parses the LLM output into memory facts, mirroring
/// `parse_action_items`: strips fences/prose, takes the first balanced JSON
/// array, then salvages each element independently. Returns None when no
/// parsable array exists (the raw output is then stored as the run result).
pub(crate) fn parse_memory_facts(raw: &str) -> Option<Vec<ParsedMemoryFact>> {
    let cleaned = crate::summary::processor::clean_llm_markdown_output(raw);
    let candidate =
        extract_first_json_array(&cleaned).or_else(|| extract_first_json_array(raw))?;
    let values: Vec<serde_json::Value> = serde_json::from_str(&candidate).ok()?;
    Some(values.iter().filter_map(memory_fact_from_value).collect())
}

/// Renders parsed memory facts as the run's markdown output, grouped by kind.
fn memory_facts_to_markdown(facts: &[ParsedMemoryFact]) -> String {
    if facts.is_empty() {
        return "## Client Memory\n\nNothing worth remembering was found in this meeting."
            .to_string();
    }
    let mut md = String::from("## Client Memory\n");
    for (kind, heading) in [
        ("commitment", "Commitments"),
        ("decision", "Decisions"),
        ("figure", "Figures"),
        ("note", "Notes"),
    ] {
        let group: Vec<&ParsedMemoryFact> = facts.iter().filter(|f| f.kind == kind).collect();
        if group.is_empty() {
            continue;
        }
        md.push_str(&format!("\n### {}\n\n", heading));
        for fact in group {
            md.push_str(&format!("- **{}** — {}", fact.subject, fact.detail));
            if let Some(owner) = fact.owner.as_deref() {
                md.push_str(&format!(" (owner: {})", owner));
            }
            if let Some(due) = fact.due_hint.as_deref() {
                md.push_str(&format!(" (due: {})", due));
            }
            if let Some(amount) = fact.amount.as_deref() {
                md.push_str(&format!(" ({})", amount));
            }
            md.push('\n');
        }
    }
    md
}

/// Renders parsed action items as the run's markdown output.
fn action_items_to_markdown(items: &[ParsedActionItem]) -> String {
    if items.is_empty() {
        return "## Action Items\n\nNo action items were found in this meeting.".to_string();
    }
    let mut md = String::from("## Action Items\n\n");
    for item in items {
        md.push_str("- [ ] ");
        md.push_str(item.description.trim());
        if let Some(owner) = item.owner.as_deref().filter(|s| !s.trim().is_empty()) {
            md.push_str(&format!(" (owner: {})", owner.trim()));
        }
        if let Some(due) = item.due_hint.as_deref().filter(|s| !s.trim().is_empty()) {
            md.push_str(&format!(" (due: {})", due.trim()));
        }
        md.push('\n');
    }
    md
}

/// Returns the effective (enabled, auto_run) pair for an agent: the saved
/// setting when one exists, otherwise the registry defaults.
pub async fn effective_settings(
    pool: &SqlitePool,
    agent: &AgentDefinition,
) -> (bool, bool) {
    match AgentSettingsRepository::get(pool, agent.id).await {
        Ok(Some(row)) => (row.enabled, row.auto_run),
        Ok(None) => (true, agent.auto_run_default),
        Err(e) => {
            warn!("Failed to read agent settings for {}: {}", agent.id, e);
            (true, agent.auto_run_default)
        }
    }
}

/// Creates the run row and spawns the background execution task. Returns the
/// run id immediately; the frontend polls `agent_runs_for_meeting` for status.
pub async fn start_agent_run(
    pool: SqlitePool,
    meeting_id: String,
    agent_id: String,
    model_provider: String,
    model_name: String,
    app_data_dir: Option<PathBuf>,
) -> Result<String, String> {
    let agent = registry::get(&agent_id).ok_or_else(|| format!("Unknown agent: {}", agent_id))?;

    let (enabled, _) = effective_settings(&pool, agent).await;
    if !enabled {
        return Err(format!("{} is disabled in agent settings", agent.name));
    }

    if AgentRunsRepository::has_running_run(&pool, &meeting_id, &agent_id)
        .await
        .map_err(|e| format!("Failed to check running agent runs: {}", e))?
    {
        return Err(format!("{} is already running for this meeting", agent.name));
    }

    let run_id = AgentRunsRepository::create_run(&pool, &agent_id, &meeting_id)
        .await
        .map_err(|e| format!("Failed to create agent run: {}", e))?;

    let run_id_for_task = run_id.clone();
    tauri::async_runtime::spawn(async move {
        execute_run(
            pool,
            run_id_for_task,
            agent,
            meeting_id,
            model_provider,
            model_name,
            app_data_dir,
        )
        .await;
    });

    Ok(run_id)
}

async fn execute_run(
    pool: SqlitePool,
    run_id: String,
    agent: &'static AgentDefinition,
    meeting_id: String,
    model_provider: String,
    model_name: String,
    app_data_dir: Option<PathBuf>,
) {
    info!(
        "Agent run {} started: agent={}, meeting={}, provider={}, model={}",
        run_id, agent.id, meeting_id, model_provider, model_name
    );

    let outcome = run_agent_llm(
        &pool,
        agent,
        &meeting_id,
        &model_provider,
        &model_name,
        app_data_dir,
    )
    .await;

    match outcome {
        Ok(raw_output) => {
            let output_md = match agent.output_kind {
                AgentOutputKind::Markdown => {
                    crate::summary::processor::clean_llm_markdown_output(&raw_output)
                }
                AgentOutputKind::ActionItems => match parse_action_items(&raw_output) {
                    Some(items) => {
                        if let Err(e) =
                            ActionItemsRepository::clear_open_agent_items(&pool, &meeting_id).await
                        {
                            error!("Failed to clear previous open action items: {}", e);
                        }
                        for item in &items {
                            if let Err(e) = ActionItemsRepository::insert(
                                &pool,
                                &meeting_id,
                                Some(&run_id),
                                item.description.trim(),
                                item.owner.as_deref(),
                                item.due_hint.as_deref(),
                            )
                            .await
                            {
                                error!("Failed to insert action item: {}", e);
                            }
                        }
                        info!(
                            "Agent run {} extracted {} action item(s)",
                            run_id,
                            items.len()
                        );
                        action_items_to_markdown(&items)
                    }
                    None => {
                        // Parse failure: keep the raw output as the run result
                        // instead of dropping the model's work; no items created.
                        warn!(
                            "Agent run {} output was not parsable as an action item array; storing raw output",
                            run_id
                        );
                        raw_output.trim().to_string()
                    }
                },
                AgentOutputKind::MemoryFacts => match parse_memory_facts(&raw_output) {
                    Some(facts) => {
                        // Denormalize the meeting's client tag onto each fact at
                        // extraction time, so client timelines survive retags.
                        let client_id = match MeetingClientsRepository::client_for_meeting(
                            &pool,
                            &meeting_id,
                        )
                        .await
                        {
                            Ok(client) => client.map(|c| c.id),
                            Err(e) => {
                                warn!("Failed to read meeting client for fact tagging: {}", e);
                                None
                            }
                        };
                        if let Err(e) = MemoryFactsRepository::clear_replaceable_agent_facts(
                            &pool,
                            &meeting_id,
                        )
                        .await
                        {
                            error!("Failed to clear previous memory facts: {}", e);
                        }
                        for fact in &facts {
                            if let Err(e) = MemoryFactsRepository::insert(
                                &pool,
                                &meeting_id,
                                client_id.as_deref(),
                                Some(&run_id),
                                &fact.kind,
                                &fact.subject,
                                &fact.detail,
                                fact.owner.as_deref(),
                                fact.due_hint.as_deref(),
                                fact.amount.as_deref(),
                            )
                            .await
                            {
                                error!("Failed to insert memory fact: {}", e);
                            }
                        }
                        info!(
                            "Agent run {} extracted {} memory fact(s)",
                            run_id,
                            facts.len()
                        );
                        memory_facts_to_markdown(&facts)
                    }
                    None => {
                        warn!(
                            "Agent run {} output was not parsable as a memory fact array; storing raw output",
                            run_id
                        );
                        raw_output.trim().to_string()
                    }
                },
            };

            if let Err(e) = AgentRunsRepository::complete_run(&pool, &run_id, &output_md).await {
                error!("Failed to persist completed agent run {}: {}", run_id, e);
            } else {
                info!("Agent run {} completed", run_id);
            }
        }
        Err(e) => {
            error!("Agent run {} failed: {}", run_id, e);
            if let Err(db_err) = AgentRunsRepository::fail_run(&pool, &run_id, &e).await {
                error!("Failed to persist failed agent run {}: {}", run_id, db_err);
            }
        }
    }
}

async fn run_agent_llm(
    pool: &SqlitePool,
    agent: &AgentDefinition,
    meeting_id: &str,
    model_provider: &str,
    model_name: &str,
    app_data_dir: Option<PathBuf>,
) -> Result<String, String> {
    // Privacy profile: refuse a cloud model this meeting's profile does not
    // allow, then mask obvious secrets in the context handed to the model.
    let effective = crate::profiles::enforce::guard_llm(
        pool,
        &crate::profiles::enforce::Scope::meeting(meeting_id),
        model_provider,
    )
    .await?;

    let settings = resolve_llm_settings(pool, model_provider).await?;
    let context = build_agent_context(pool, meeting_id, agent).await?;
    let (context, _) = crate::profiles::enforce::redact_for(&effective, &context);
    let user_prompt = (agent.build_user_prompt)(&context);

    let client = reqwest::Client::new();
    generate_summary(
        &client,
        &settings.provider,
        model_name,
        &settings.api_key,
        agent.system_prompt,
        &user_prompt,
        settings.ollama_endpoint.as_deref(),
        settings.custom_openai_endpoint.as_deref(),
        settings.max_tokens,
        settings.temperature,
        settings.top_p,
        app_data_dir.as_ref(),
        None,
    )
    .await
}

/// Fire-and-forget hook called after a summary completes: starts every enabled
/// auto-run agent (v1: the Action Tracker) with the same provider/model that
/// produced the summary. Failures are logged, never surfaced to the summary flow.
pub async fn auto_run_after_summary(
    pool: SqlitePool,
    meeting_id: String,
    model_provider: String,
    model_name: String,
    app_data_dir: Option<PathBuf>,
) {
    for agent in registry::all() {
        let (enabled, auto_run) = effective_settings(&pool, agent).await;
        if !(enabled && auto_run) {
            continue;
        }
        info!(
            "Auto-running agent {} after summary completion for meeting {}",
            agent.id, meeting_id
        );
        if let Err(e) = start_agent_run(
            pool.clone(),
            meeting_id.clone(),
            agent.id.to_string(),
            model_provider.clone(),
            model_name.clone(),
            app_data_dir.clone(),
        )
        .await
        {
            warn!("Auto-run of agent {} skipped: {}", agent.id, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plain_array() {
        let raw = r#"[{"description": "Send budget", "owner": "You", "due_hint": "by Friday"}]"#;
        let items = parse_action_items(raw).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].description, "Send budget");
        assert_eq!(items[0].owner.as_deref(), Some("You"));
        assert_eq!(items[0].due_hint.as_deref(), Some("by Friday"));
    }

    #[test]
    fn test_parse_fenced_array_with_prose() {
        let raw = "Here are the action items:\n```json\n[{\"description\": \"Book room\"}]\n```\nLet me know!";
        let items = parse_action_items(raw).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].description, "Book room");
        assert!(items[0].owner.is_none());
    }

    #[test]
    fn test_parse_empty_array() {
        let items = parse_action_items("```json\n[]\n```").unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_parse_aliases() {
        let raw = r#"[{"task": "Review deck", "assignee": "Ana", "due": "next week"}]"#;
        let items = parse_action_items(raw).unwrap();
        assert_eq!(items[0].description, "Review deck");
        assert_eq!(items[0].owner.as_deref(), Some("Ana"));
        assert_eq!(items[0].due_hint.as_deref(), Some("next week"));
    }

    #[test]
    fn test_parse_failure_returns_none() {
        assert!(parse_action_items("Sorry, I could not find any action items.").is_none());
        assert!(parse_action_items("[unclosed").is_none());
    }

    #[test]
    fn test_extract_array_ignores_brackets_in_strings() {
        let raw = r#"noise [{"description": "Handle [edge] cases"}] trailing"#;
        let extracted = extract_first_json_array(raw).unwrap();
        assert_eq!(extracted, r#"[{"description": "Handle [edge] cases"}]"#);
    }

    #[test]
    fn test_blank_descriptions_filtered() {
        let raw = r#"[{"description": "  "}, {"description": "Real task"}]"#;
        let items = parse_action_items(raw).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].description, "Real task");
    }

    #[test]
    fn test_parse_memory_facts_full_shape() {
        let raw = r#"```json
[{"kind": "commitment", "subject": "Quote", "detail": "Send revised quote", "owner": "You", "due_hint": "by Friday", "amount": null},
 {"kind": "figure", "subject": "Budget", "detail": "Annual budget agreed", "amount": "$12,000"}]
```"#;
        let facts = parse_memory_facts(raw).unwrap();
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].kind, "commitment");
        assert_eq!(facts[0].owner.as_deref(), Some("You"));
        assert_eq!(facts[1].amount.as_deref(), Some("$12,000"));
    }

    #[test]
    fn test_parse_memory_facts_tolerates_aliases_and_kinds() {
        let raw = r#"[{"type": "decisions", "title": "Vendor", "description": "Chose vendor A"},
                      {"kind": "something-weird", "subject": "Tidbit", "detail": "Client prefers mornings"},
                      {"kind": "figure", "subject": "Seats", "detail": "Seat count", "amount": 15}]"#;
        let facts = parse_memory_facts(raw).unwrap();
        assert_eq!(facts.len(), 3);
        assert_eq!(facts[0].kind, "decision");
        assert_eq!(facts[0].subject, "Vendor");
        assert_eq!(facts[1].kind, "note", "unknown kinds degrade to note");
        assert_eq!(facts[2].amount.as_deref(), Some("15"), "numeric amounts become strings");
    }

    #[test]
    fn test_parse_memory_facts_salvages_partial_elements() {
        let raw = r#"[{"subject": "Only a label"},
                      {"detail": "Only substance, no subject"},
                      {"owner": "nobody"},
                      "not an object"]"#;
        let facts = parse_memory_facts(raw).unwrap();
        assert_eq!(facts.len(), 2, "empty and non-object elements are skipped");
        assert_eq!(facts[0].subject, "Only a label");
        assert_eq!(facts[0].detail, "Only a label");
        assert_eq!(facts[1].subject, "Only substance, no subject");
    }

    #[test]
    fn test_parse_memory_facts_failure_and_empty() {
        assert!(parse_memory_facts("I found nothing to extract.").is_none());
        assert!(parse_memory_facts("[unclosed").is_none());
        assert!(parse_memory_facts("```json\n[]\n```").unwrap().is_empty());
    }

    #[test]
    fn test_memory_facts_markdown_groups_by_kind() {
        let facts = vec![
            ParsedMemoryFact {
                kind: "note".to_string(),
                subject: "Preference".to_string(),
                detail: "Prefers email".to_string(),
                owner: None,
                due_hint: None,
                amount: None,
            },
            ParsedMemoryFact {
                kind: "commitment".to_string(),
                subject: "Quote".to_string(),
                detail: "Send quote".to_string(),
                owner: Some("You".to_string()),
                due_hint: Some("Friday".to_string()),
                amount: None,
            },
        ];
        let md = memory_facts_to_markdown(&facts);
        let commitments_at = md.find("### Commitments").unwrap();
        let notes_at = md.find("### Notes").unwrap();
        assert!(commitments_at < notes_at, "commitments render before notes");
        assert!(md.contains("**Quote** — Send quote (owner: You) (due: Friday)"));
        assert!(memory_facts_to_markdown(&[]).contains("Nothing worth remembering"));
    }

    #[test]
    fn test_action_items_markdown() {
        let items = vec![ParsedActionItem {
            description: "Send budget".to_string(),
            owner: Some("You".to_string()),
            due_hint: None,
        }];
        let md = action_items_to_markdown(&items);
        assert!(md.contains("- [ ] Send budget (owner: You)"));
    }
}
