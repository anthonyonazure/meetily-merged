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
}

/// Inputs available to prompt builders.
pub struct AgentContext {
    pub meeting_title: String,
    pub transcript: String,
    pub summary_markdown: Option<String>,
}

pub struct AgentDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub output_kind: AgentOutputKind,
    /// Whether this agent runs automatically after a summary completes when
    /// the user has not saved an explicit setting yet.
    pub auto_run_default: bool,
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

pub const AGENTS: &[AgentDefinition] = &[
    AgentDefinition {
        id: "followup_drafter",
        name: "Follow-up Drafter",
        description: "Drafts a follow-up email from the transcript and summary. Nothing is ever sent; copy it into your mail client.",
        output_kind: AgentOutputKind::Markdown,
        auto_run_default: false,
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
        system_prompt: "You extract decisions from meeting transcripts. \
Output markdown: a \"## Decisions\" heading followed by one bullet per decision in the form \
\"- **Decision** — context (one line) — driven by <person or 'group'>\". \
Only include decisions actually reached in the transcript; skip open questions. \
If no decisions were made, output \"## Decisions\" followed by \"- No decisions were recorded in this meeting.\" \
Output only the markdown, no preamble.",
        build_user_prompt: decision_log_prompt,
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
    fn test_prompts_include_transcript() {
        let context = AgentContext {
            meeting_title: "Standup".to_string(),
            transcript: "hello world transcript".to_string(),
            summary_markdown: Some("## Summary\nfoo".to_string()),
        };
        for agent in AGENTS {
            let prompt = (agent.build_user_prompt)(&context);
            assert!(prompt.contains("hello world transcript"));
            assert!(prompt.contains("Standup"));
        }
    }
}
