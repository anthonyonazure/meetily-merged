//! Enforcement helpers for everything that happens after a recording: models,
//! sharing, and the redacted copy handed to either.
//!
//! Each helper resolves through `profiles::resolver` and returns the resolved
//! profile, so a caller that needs both the refusal and the redaction setting
//! gets them from one call.

use sqlx::SqlitePool;

use super::redaction::{self, RedactionReport};
use super::resolver::{self, EffectiveProfile};

/// What the caller is acting on, so the resolver can find the right profile.
#[derive(Debug, Clone)]
pub enum Scope {
    Meeting(String),
    Client(String),
    /// Nothing narrower than the workspace (the all-meetings chat, for example).
    Workspace,
}

impl Scope {
    pub fn meeting(id: impl Into<String>) -> Self {
        Self::Meeting(id.into())
    }

    pub fn client(id: impl Into<String>) -> Self {
        Self::Client(id.into())
    }
}

pub async fn resolve(pool: &SqlitePool, scope: &Scope) -> EffectiveProfile {
    match scope {
        Scope::Meeting(id) => resolver::for_meeting(pool, id).await,
        Scope::Client(id) => resolver::for_client(pool, id).await,
        Scope::Workspace => resolver::workspace_default(pool).await,
    }
}

/// Refuses an LLM provider the governing profile does not allow.
///
/// Returns the resolved profile on success so the caller can immediately ask it
/// whether the prompt needs masking.
pub async fn guard_llm(
    pool: &SqlitePool,
    scope: &Scope,
    provider: &str,
) -> Result<EffectiveProfile, String> {
    let effective = resolve(pool, scope).await;
    effective.check_llm(provider)?;
    Ok(effective)
}

/// Refuses a share action when the governing profile has sharing off.
pub async fn guard_sharing(
    pool: &SqlitePool,
    scope: &Scope,
) -> Result<EffectiveProfile, String> {
    let effective = resolve(pool, scope).await;
    effective.check_sharing()?;
    Ok(effective)
}

/// Masks obvious secrets when the profile asks for it. The stored transcript is
/// never touched: this is applied to the copy on its way to a model, an export,
/// or a share action.
pub fn redact_for(effective: &EffectiveProfile, text: &str) -> (String, RedactionReport) {
    let (masked, report) = redaction::redact_if(effective.redact_pii(), text);
    if !report.is_empty() {
        log::info!(
            "[Profiles] masked {} item(s) before handing the text on (profile {})",
            report.total(),
            effective.profile_name().unwrap_or("none")
        );
    }
    (masked, report)
}

/// Resolve, refuse, and mask in one step, for the callers that always do all
/// three (summary generation, agents, chat).
pub async fn guard_llm_and_redact(
    pool: &SqlitePool,
    scope: &Scope,
    provider: &str,
    text: &str,
) -> Result<(String, EffectiveProfile), String> {
    let effective = guard_llm(pool, scope, provider).await?;
    let (masked, _) = redact_for(&effective, text);
    Ok((masked, effective))
}
