//! Built-in agent registry.
//!
//! Adding a new agent means adding one `AgentDefinition` entry to `AGENTS`.
//! The frontend renders whatever `agents_list` reports, so no per-agent UI
//! wiring is needed beyond this file.

use serde::{Deserialize, Serialize};

/// What the runner does with the LLM output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentOutputKind {
    /// Output is stored and rendered as markdown.
    Markdown,
    /// Output is parsed as a JSON array of action items; parsed items are
    /// inserted into `action_items` (raw output is kept when parsing fails).
    ActionItems,
    /// Output is parsed as a JSON array of Client Memory facts; parsed facts
    /// are inserted into `memory_facts` (raw output is kept when parsing
    /// fails), tagged with the meeting's client at extraction time.
    MemoryFacts,
}

/// Inputs available to prompt builders.
pub struct AgentContext {
    pub meeting_title: String,
    pub transcript: String,
    pub summary_markdown: Option<String>,
    /// Name of the client the meeting is tagged with, when any.
    pub client_name: Option<String>,
    /// Preformatted block of the client's stale open commitments (populated
    /// only for agents with `needs_client`).
    pub client_commitments: Option<String>,
}

pub struct AgentDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub output_kind: AgentOutputKind,
    /// Whether this agent runs automatically after a summary completes when
    /// the user has not saved an explicit setting yet.
    pub auto_run_default: bool,
    /// Whether the agent operates on the meeting's client (its run fails with
    /// a friendly message when the meeting is untagged).
    pub needs_client: bool,
    pub system_prompt: &'static str,
    pub build_user_prompt: fn(&AgentContext) -> String,
}

fn shared_context_block(context: &AgentContext) -> String {
    let mut block = format!("Meeting title: {}\n\n", context.meeting_title);
    if let Some(summary) = context
        .summary_markdown
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        block.push_str("Meeting summary:\n");
        block.push_str(summary);
        block.push_str("\n\n");
    }
    block.push_str("Full transcript:\n");
    block.push_str(&context.transcript);
    block
}

fn followup_drafter_prompt(context: &AgentContext) -> String {
    format!(
        "Draft the follow-up email for this meeting.\n\n{}",
        shared_context_block(context)
    )
}

fn action_tracker_prompt(context: &AgentContext) -> String {
    format!(
        "Extract the action items from this meeting.\n\n{}",
        shared_context_block(context)
    )
}

fn decision_log_prompt(context: &AgentContext) -> String {
    format!(
        "Extract the decisions made in this meeting.\n\n{}",
        shared_context_block(context)
    )
}

fn memory_extractor_prompt(context: &AgentContext) -> String {
    format!(
        "Extract the durable memory facts from this meeting.\n\n{}",
        shared_context_block(context)
    )
}

fn follow_through_prompt(context: &AgentContext) -> String {
    // Client-scoped: the input is the client's stale open commitments, not
    // this meeting's transcript. The runner guarantees both fields are set
    // (needs_client) before this builder is called.
    format!(
        "Write follow-through nudges for the open commitments with the client \"{}\".\n\n\
Open commitments that have gone quiet:\n{}",
        context.client_name.as_deref().unwrap_or("(unknown client)"),
        context
            .client_commitments
            .as_deref()
            .unwrap_or("(none provided)")
    )
}

pub const AGENTS: &[AgentDefinition] = &[
    AgentDefinition {
        id: "followup_drafter",
        name: "Follow-up Drafter",
        description: "Drafts a follow-up email from the transcript and summary. Nothing is ever sent; copy it into your mail client.",
        output_kind: AgentOutputKind::Markdown,
        auto_run_default: false,
        needs_client: false,
        system_prompt: "You are an assistant that drafts follow-up emails after meetings. \
Write a concise, professional follow-up email in markdown with: a one-line subject suggestion, \
a short recap of what was discussed, commitments made by \"You\" (the microphone speaker), \
asks of other participants, and proposed next steps. \
Only include things actually said in the transcript. Do not invent names, dates, or commitments. \
Output only the email markdown, no preamble.",
        build_user_prompt: followup_drafter_prompt,
    },
    AgentDefinition {
        id: "action_tracker",
        name: "Action Tracker",
        description: "Extracts action items (what, who, due hint) into a checklist you can track across meetings.",
        output_kind: AgentOutputKind::ActionItems,
        auto_run_default: true,
        needs_client: false,
        system_prompt: "You extract action items from meeting transcripts. \
Respond with ONLY a JSON array inside a fenced code block. Each element must be an object with: \
\"description\" (string, required, the concrete task), \
\"owner\" (string or null, who committed to it, e.g. \"You\" or a name from the transcript), \
\"due_hint\" (string or null, any deadline wording exactly as spoken, e.g. \"by Friday\"). \
Only include real commitments from the transcript. If there are none, return an empty array []. \
Example:\n```json\n[{\"description\": \"Send the revised budget\", \"owner\": \"You\", \"due_hint\": \"by Friday\"}]\n```",
        build_user_prompt: action_tracker_prompt,
    },
    AgentDefinition {
        id: "decision_log",
        name: "Decision Log",
        description: "Records the decisions made in the meeting, with one line of context and who drove each.",
        output_kind: AgentOutputKind::Markdown,
        auto_run_default: false,
        needs_client: false,
        system_prompt: "You extract decisions from meeting transcripts. \
Output markdown: a \"## Decisions\" heading followed by one bullet per decision in the form \
\"- **Decision** — context (one line) — driven by <person or 'group'>\". \
Only include decisions actually reached in the transcript; skip open questions. \
If no decisions were made, output \"## Decisions\" followed by \"- No decisions were recorded in this meeting.\" \
Output only the markdown, no preamble.",
        build_user_prompt: decision_log_prompt,
    },
    AgentDefinition {
        id: "memory_extractor",
        name: "Memory Extractor",
        description: "Distills the meeting into durable facts — commitments, decisions, figures, notes — that build the client's running memory.",
        output_kind: AgentOutputKind::MemoryFacts,
        auto_run_default: true,
        needs_client: false,
        system_prompt: "You extract durable facts from meeting transcripts for a client memory system. \
Respond with ONLY a JSON array inside a fenced code block. Each element must be an object with: \
\"kind\" (one of \"commitment\", \"decision\", \"figure\", \"note\"), \
\"subject\" (string, required, a short label like \"Contract renewal\"), \
\"detail\" (string, required, one or two sentences of substance), \
\"owner\" (string or null, who it belongs to, e.g. \"You\" or a name from the transcript), \
\"due_hint\" (string or null, deadline wording exactly as spoken, e.g. \"by end of month\"), \
\"amount\" (string or null, the figure for kind \"figure\", e.g. \"$12,000\" or \"15 seats\"). \
Kinds: a commitment is something someone promised to do; a decision is a choice that was agreed; \
a figure is a number, amount, or date that matters commercially; a note is any other fact worth \
remembering next time (preferences, constraints, context, names). \
Only include things actually said in the transcript. Do not invent anything. \
If there is nothing worth remembering, return an empty array []. \
Example:\n```json\n[{\"kind\": \"commitment\", \"subject\": \"Revised quote\", \"detail\": \"You promised to send the revised quote.\", \"owner\": \"You\", \"due_hint\": \"by Friday\", \"amount\": null}]\n```",
        build_user_prompt: memory_extractor_prompt,
    },
    AgentDefinition {
        id: "follow_through",
        name: "Follow-through",
        description: "Reviews this client's open commitments that have gone quiet and drafts a chase message for each. Nothing is ever sent.",
        output_kind: AgentOutputKind::Markdown,
        auto_run_default: false,
        needs_client: true,
        system_prompt: "You help the user follow through on commitments made to or by a client. \
You are given open commitments that have gone quiet (with owner, age in days, and any due wording). \
Output markdown: a \"## Follow-through\" heading, then for each commitment a \"### <subject>\" section containing \
one nudge line (who owes what, how long it has been open, any due wording) and \
\"Suggested chase message:\" followed by a blockquote with a friendly, professional 2-4 sentence \
chase email body written from the user's perspective. \
Only use the provided commitments; do not invent new ones or new facts. \
Output only the markdown, no preamble.",
        build_user_prompt: follow_through_prompt,
    },
];

pub fn all() -> &'static [AgentDefinition] {
    AGENTS
}

pub fn get(agent_id: &str) -> Option<&'static AgentDefinition> {
    AGENTS.iter().find(|agent| agent.id == agent_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_ids_are_unique() {
        let mut ids: Vec<&str> = AGENTS.iter().map(|a| a.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), AGENTS.len());
    }

    #[test]
    fn test_get_known_and_unknown() {
        assert!(get("action_tracker").is_some());
        assert!(get("nope").is_none());
    }

    #[test]
    fn test_prompts_include_their_inputs() {
        let context = AgentContext {
            meeting_title: "Standup".to_string(),
            transcript: "hello world transcript".to_string(),
            summary_markdown: Some("## Summary\nfoo".to_string()),
            client_name: Some("Acme".to_string()),
            client_commitments: Some("- [c1] Send quote (open 5 days)".to_string()),
        };
        for agent in AGENTS {
            let prompt = (agent.build_user_prompt)(&context);
            if agent.needs_client {
                // Client-scoped agents prompt from the commitments block, not
                // the transcript.
                assert!(prompt.contains("Acme"), "{} misses client name", agent.id);
                assert!(prompt.contains("Send quote"), "{} misses commitments", agent.id);
            } else {
                assert!(prompt.contains("hello world transcript"), "{} misses transcript", agent.id);
                assert!(prompt.contains("Standup"), "{} misses title", agent.id);
            }
        }
    }
}
