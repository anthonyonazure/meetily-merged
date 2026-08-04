//! Follow-through: turns a client's stale open commitments into nudges and
//! suggested chase messages.
//!
//! Shared between two entry points: the `follow_through` registry agent
//! (meeting Agents panel, markdown output) and the `client_follow_through`
//! command (Clients page, structured output with per-commitment chase text
//! for the draft-email button). Both build the same commitments block.

use crate::database::models::MemoryFactWithMeeting;
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::Serialize;

/// A commitment has to be at least this old before follow-through chases it,
/// unless a parseable due date says it is already overdue.
pub const CHASE_MIN_AGE_DAYS: i64 = 3;

/// One open commitment that deserves a chase.
#[derive(Debug, Clone, Serialize)]
pub struct StaleCommitment {
    pub fact_id: String,
    pub subject: String,
    pub detail: String,
    pub owner: Option<String>,
    pub due_hint: Option<String>,
    pub meeting_title: String,
    pub age_days: i64,
}

/// Best-effort parse of a due hint into a date. Handles absolute forms
/// ("2026-08-01", "8/1/2026", "August 1", "Aug 1"); relative wording like
/// "by Friday" is not parseable and returns None (age-based staleness then
/// applies).
pub fn parse_due_hint(hint: &str, now: DateTime<Utc>) -> Option<NaiveDate> {
    let cleaned: String = hint
        .trim()
        .trim_start_matches("by ")
        .trim_start_matches("By ")
        .trim_start_matches("on ")
        .trim_start_matches("On ")
        .trim_end_matches(['.', ',', '!'])
        .to_string();
    let candidates = [cleaned.as_str()];
    for text in candidates {
        if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
            return Some(date);
        }
        if let Ok(date) = NaiveDate::parse_from_str(text, "%m/%d/%Y") {
            return Some(date);
        }
        for format in ["%B %d %Y", "%b %d %Y"] {
            if let Ok(date) = NaiveDate::parse_from_str(text, format) {
                return Some(date);
            }
        }
        // Month + day without a year ("August 1"): assume the current year.
        let with_year = format!("{} {}", text.replace(',', ""), now.year());
        for format in ["%B %d %Y", "%b %d %Y"] {
            if let Ok(date) = NaiveDate::parse_from_str(&with_year, format) {
                return Some(date);
            }
        }
    }
    None
}

/// Filters open commitments down to the chase-worthy set:
/// - a parseable due date in the past always qualifies;
/// - a parseable due date in the future never does (not yet due);
/// - otherwise the commitment qualifies once it is `CHASE_MIN_AGE_DAYS` old.
pub fn stale_commitments(
    facts: Vec<MemoryFactWithMeeting>,
    now: DateTime<Utc>,
) -> Vec<StaleCommitment> {
    let today = now.date_naive();
    facts
        .into_iter()
        .filter_map(|fact| {
            let age_days = (now - fact.created_at).num_days();
            let due = fact.due_hint.as_deref().and_then(|h| parse_due_hint(h, now));
            let qualifies = match due {
                Some(date) => date < today,
                None => age_days >= CHASE_MIN_AGE_DAYS,
            };
            qualifies.then(|| StaleCommitment {
                fact_id: fact.id,
                subject: fact.subject,
                detail: fact.detail,
                owner: fact.owner,
                due_hint: fact.due_hint,
                meeting_title: fact.meeting_title,
                age_days,
            })
        })
        .collect()
}

/// Renders the commitments block placed in follow-through prompts. Each line
/// carries the fact id in brackets so structured outputs can echo it back.
pub fn commitments_block(commitments: &[StaleCommitment]) -> String {
    commitments
        .iter()
        .map(|c| {
            let mut line = format!(
                "- [{}] {} — {} (from meeting \"{}\", open {} day{})",
                c.fact_id,
                c.subject,
                c.detail,
                c.meeting_title,
                c.age_days,
                if c.age_days == 1 { "" } else { "s" }
            );
            if let Some(owner) = c.owner.as_deref() {
                line.push_str(&format!(" (owner: {})", owner));
            }
            if let Some(due) = c.due_hint.as_deref() {
                line.push_str(&format!(" (due: {})", due));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// System prompt for the structured (Clients page) variant: same job as the
/// registry agent, but JSON so each chase message can power a draft button.
const CHASE_JSON_SYSTEM_PROMPT: &str = "You help the user follow through on commitments made to or by a client. \
You are given open commitments that have gone quiet; each line starts with the commitment id in brackets. \
Respond with ONLY a JSON array inside a fenced code block. One element per commitment, each an object with: \
\"id\" (string, required, the commitment id copied exactly from the brackets), \
\"nudge\" (string, one line: who owes what, how long it has been open, any due wording), \
\"chase_subject\" (string, a short suggested email subject), \
\"chase_message\" (string, a friendly, professional 2-4 sentence chase email body written from the user's perspective, plain text). \
Only use the provided commitments; do not invent new ones or new facts.";

/// One chase suggestion, aligned to a stored commitment.
#[derive(Debug, Clone, Serialize)]
pub struct ChaseSuggestion {
    pub fact_id: String,
    pub subject: String,
    pub owner: Option<String>,
    pub due_hint: Option<String>,
    pub age_days: i64,
    pub nudge: String,
    pub chase_subject: String,
    pub chase_message: String,
}

/// What the Clients page renders: markdown always, plus structured chases
/// when the model's JSON parsed (each powers a "Draft chase email" button).
#[derive(Debug, Clone, Serialize)]
pub struct FollowThroughResult {
    pub markdown: String,
    pub chases: Vec<ChaseSuggestion>,
}

#[derive(Debug, serde::Deserialize)]
struct ParsedChase {
    id: String,
    #[serde(default)]
    nudge: Option<String>,
    #[serde(default, alias = "subject")]
    chase_subject: Option<String>,
    #[serde(default, alias = "message", alias = "chase", alias = "body")]
    chase_message: Option<String>,
}

/// Aligns parsed chases to the stale commitments by id. Elements with unknown
/// ids are dropped; commitments the model skipped simply have no chase.
pub(crate) fn align_chases(
    commitments: &[StaleCommitment],
    raw_output: &str,
) -> Vec<ChaseSuggestion> {
    let cleaned = crate::summary::processor::clean_llm_markdown_output(raw_output);
    let Some(candidate) = crate::agents::runner::extract_first_json_array(&cleaned)
        .or_else(|| crate::agents::runner::extract_first_json_array(raw_output))
    else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<Vec<ParsedChase>>(&candidate) else {
        return Vec::new();
    };
    parsed
        .into_iter()
        .filter_map(|chase| {
            let commitment = commitments.iter().find(|c| c.fact_id == chase.id)?;
            let message = chase.chase_message.as_deref().unwrap_or("").trim().to_string();
            if message.is_empty() {
                return None;
            }
            Some(ChaseSuggestion {
                fact_id: commitment.fact_id.clone(),
                subject: commitment.subject.clone(),
                owner: commitment.owner.clone(),
                due_hint: commitment.due_hint.clone(),
                age_days: commitment.age_days,
                nudge: chase
                    .nudge
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                chase_subject: chase
                    .chase_subject
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| format!("Following up: {}", commitment.subject)),
                chase_message: message,
            })
        })
        .collect()
}

/// Renders the parsed chases as the markdown nudge list.
pub(crate) fn chases_to_markdown(client_name: &str, chases: &[ChaseSuggestion]) -> String {
    let mut md = format!("## Follow-through — {}\n", client_name);
    for chase in chases {
        md.push_str(&format!("\n### {}\n\n", chase.subject));
        if !chase.nudge.is_empty() {
            md.push_str(&format!("{}\n\n", chase.nudge));
        } else {
            md.push_str(&format!(
                "Open {} day{}.\n\n",
                chase.age_days,
                if chase.age_days == 1 { "" } else { "s" }
            ));
        }
        md.push_str("Suggested chase message:\n");
        for line in chase.chase_message.lines() {
            md.push_str(&format!("> {}\n", line));
        }
    }
    md
}

/// Runs follow-through for a client: gathers stale open commitments, asks the
/// configured LLM for nudges + chase messages (JSON), and returns markdown
/// plus the structured chases. Falls back to the model's raw markdown when
/// the JSON does not parse.
pub async fn run_for_client(
    pool: &sqlx::SqlitePool,
    client_id: &str,
    model_provider: &str,
    model_name: &str,
    app_data_dir: Option<std::path::PathBuf>,
) -> Result<FollowThroughResult, String> {
    use crate::database::repositories::{
        client::ClientsRepository, memory::MemoryFactsRepository,
    };

    let client = ClientsRepository::get(pool, client_id)
        .await
        .map_err(|e| format!("Failed to load client: {}", e))?
        .ok_or_else(|| "Client not found".to_string())?;

    let facts = MemoryFactsRepository::open_commitments_for_client(pool, client_id, 0)
        .await
        .map_err(|e| format!("Failed to load open commitments: {}", e))?;
    let stale = stale_commitments(facts, Utc::now());
    if stale.is_empty() {
        return Ok(FollowThroughResult {
            markdown: format!(
                "## Follow-through — {}\n\nNo open commitments ready to chase: nothing is older than {} days or overdue. Nice work.",
                client.name, CHASE_MIN_AGE_DAYS
            ),
            chases: Vec::new(),
        });
    }

    let settings =
        crate::agents::runner::resolve_llm_settings(pool, model_provider).await?;
    let user_prompt = format!(
        "Write follow-through nudges for the open commitments with the client \"{}\".\n\n\
Open commitments that have gone quiet:\n{}",
        client.name,
        commitments_block(&stale)
    );

    let http = reqwest::Client::new();
    let raw = crate::summary::llm_client::generate_summary(
        &http,
        &settings.provider,
        model_name,
        &settings.api_key,
        CHASE_JSON_SYSTEM_PROMPT,
        &user_prompt,
        settings.ollama_endpoint.as_deref(),
        settings.custom_openai_endpoint.as_deref(),
        settings.max_tokens,
        settings.temperature,
        settings.top_p,
        app_data_dir.as_ref(),
        None,
    )
    .await?;

    let chases = align_chases(&stale, &raw);
    let markdown = if chases.is_empty() {
        // JSON did not parse: keep the model's work as plain markdown.
        crate::summary::processor::clean_llm_markdown_output(&raw)
    } else {
        chases_to_markdown(&client.name, &chases)
    };
    Ok(FollowThroughResult { markdown, chases })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fact(
        id: &str,
        created_days_ago: i64,
        due_hint: Option<&str>,
        now: DateTime<Utc>,
    ) -> MemoryFactWithMeeting {
        MemoryFactWithMeeting {
            id: id.to_string(),
            meeting_id: "m1".to_string(),
            client_id: Some("c1".to_string()),
            agent_run_id: None,
            kind: "commitment".to_string(),
            subject: format!("Subject {}", id),
            detail: "Do the thing".to_string(),
            owner: Some("You".to_string()),
            due_hint: due_hint.map(str::to_string),
            amount: None,
            status: "open".to_string(),
            created_at: now - chrono::Duration::days(created_days_ago),
            updated_at: now - chrono::Duration::days(created_days_ago),
            meeting_title: "Kickoff".to_string(),
            meeting_created_at: crate::database::models::DateTimeUtc(
                now - chrono::Duration::days(created_days_ago),
            ),
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 4, 12, 0, 0).unwrap()
    }

    #[test]
    fn due_hint_absolute_forms_parse() {
        let n = now();
        assert_eq!(
            parse_due_hint("2026-08-01", n),
            NaiveDate::from_ymd_opt(2026, 8, 1)
        );
        assert_eq!(
            parse_due_hint("by 8/1/2026", n),
            NaiveDate::from_ymd_opt(2026, 8, 1)
        );
        assert_eq!(
            parse_due_hint("August 1", n),
            NaiveDate::from_ymd_opt(2026, 8, 1)
        );
        assert_eq!(
            parse_due_hint("by Aug 20", n),
            NaiveDate::from_ymd_opt(2026, 8, 20)
        );
        assert_eq!(parse_due_hint("by Friday", n), None);
        assert_eq!(parse_due_hint("next week", n), None);
    }

    #[test]
    fn staleness_uses_age_when_due_is_unparseable() {
        let n = now();
        let stale = stale_commitments(
            vec![fact("old", 5, Some("by Friday"), n), fact("new", 1, None, n)],
            n,
        );
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].fact_id, "old");
        assert_eq!(stale[0].age_days, 5);
    }

    #[test]
    fn parseable_due_date_overrides_age_both_ways() {
        let n = now();
        let stale = stale_commitments(
            vec![
                // Old but not yet due: skipped.
                fact("future", 10, Some("2026-09-01"), n),
                // Fresh but overdue: chased.
                fact("overdue", 1, Some("2026-08-01"), n),
            ],
            n,
        );
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].fact_id, "overdue");
    }

    #[test]
    fn chases_align_by_id_and_tolerate_noise() {
        let n = now();
        let stale = stale_commitments(vec![fact("f-1", 5, None, n), fact("f-2", 6, None, n)], n);
        let raw = r#"Here you go:
```json
[{"id": "f-1", "nudge": "Quote is 5 days old", "chase_subject": "Quick nudge", "chase_message": "Hi — just checking in on the quote."},
 {"id": "f-404", "chase_message": "orphan"},
 {"id": "f-2", "chase_message": ""}]
```"#;
        let chases = align_chases(&stale, raw);
        assert_eq!(chases.len(), 1, "unknown ids and empty messages are dropped");
        assert_eq!(chases[0].fact_id, "f-1");
        assert_eq!(chases[0].chase_subject, "Quick nudge");

        assert!(align_chases(&stale, "no json here").is_empty());
    }

    #[test]
    fn chase_subject_falls_back_to_commitment_subject() {
        let n = now();
        let stale = stale_commitments(vec![fact("f-1", 5, None, n)], n);
        let raw = r#"[{"id": "f-1", "chase_message": "Checking in."}]"#;
        let chases = align_chases(&stale, raw);
        assert_eq!(chases[0].chase_subject, "Following up: Subject f-1");
    }

    #[test]
    fn markdown_renders_nudges_and_blockquotes() {
        let chases = vec![ChaseSuggestion {
            fact_id: "f-1".to_string(),
            subject: "Quote".to_string(),
            owner: Some("You".to_string()),
            due_hint: None,
            age_days: 5,
            nudge: "Quote owed for 5 days".to_string(),
            chase_subject: "Quick nudge".to_string(),
            chase_message: "Hi,\nchecking in.".to_string(),
        }];
        let md = chases_to_markdown("Acme", &chases);
        assert!(md.contains("## Follow-through — Acme"));
        assert!(md.contains("### Quote"));
        assert!(md.contains("> Hi,\n> checking in."));
    }

    #[test]
    fn block_includes_ids_and_context() {
        let n = now();
        let stale = stale_commitments(vec![fact("f-42", 5, None, n)], n);
        let block = commitments_block(&stale);
        assert!(block.contains("[f-42]"));
        assert!(block.contains("Kickoff"));
        assert!(block.contains("open 5 days"));
        assert!(block.contains("(owner: You)"));
    }
}
