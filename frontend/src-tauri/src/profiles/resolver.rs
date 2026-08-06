//! One place that answers "which profile governs this?".
//!
//! Every enforcement point resolves through here, so transcription, models,
//! consent, sharing, and retention can never disagree about which policy is in
//! force. Resolution order is: the meeting's client tag, then the workspace
//! default, then nothing.
//!
//! "Nothing" is a real answer and the shipped one: on upgrade the workspace
//! default is unset, so an existing install keeps behaving exactly as it did
//! until an operator picks a profile.

use serde::Serialize;
use sqlx::SqlitePool;

use crate::consent::rules::{ConsentLevel, EnforcementMode};
use crate::database::repositories::{
    client::{ClientsRepository, MeetingClientsRepository},
    profile::{PrivacyProfilesRepository, PrivacySettingsRepository},
};

use super::rules::{clamp_enforcement, clamp_level, PrivacyProfile};

/// How a profile came to apply. Recorded in the consent log so the decision can
/// be checked after the fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSource {
    /// The meeting is tagged with a client that has a profile.
    ClientTag,
    /// No client profile applied, so the workspace default did.
    WorkspaceDefault,
    /// No profile applies; the app's global settings govern.
    None,
}

impl ProfileSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ClientTag => "client_tag",
            Self::WorkspaceDefault => "workspace_default",
            Self::None => "none",
        }
    }
}

/// The resolved answer: which profile (if any), and how it was reached.
#[derive(Debug, Clone, Serialize)]
pub struct EffectiveProfile {
    pub profile: Option<PrivacyProfile>,
    pub source: ProfileSource,
    /// Set when resolution went through a client tag.
    pub client_id: Option<String>,
    pub client_name: Option<String>,
}

impl EffectiveProfile {
    pub fn none() -> Self {
        Self {
            profile: None,
            source: ProfileSource::None,
            client_id: None,
            client_name: None,
        }
    }

    pub fn profile_name(&self) -> Option<&str> {
        self.profile.as_ref().map(|p| p.name.as_str())
    }

    /// One line for the consent log and the app log, naming the profile and the
    /// route it took.
    pub fn describe(&self) -> String {
        match (&self.profile, &self.source) {
            (Some(profile), ProfileSource::ClientTag) => format!(
                "Profile \"{}\" applied from the client tag ({})",
                profile.name,
                self.client_name.as_deref().unwrap_or("client")
            ),
            (Some(profile), _) => {
                format!("Profile \"{}\" applied as the workspace default", profile.name)
            }
            (None, _) => "No privacy profile applies; global settings govern".to_string(),
        }
    }

    pub fn check_transcription(&self, provider: &str) -> Result<(), String> {
        match &self.profile {
            Some(profile) => profile.check_transcription(provider),
            None => Ok(()),
        }
    }

    pub fn check_llm(&self, provider: &str) -> Result<(), String> {
        match &self.profile {
            Some(profile) => profile.check_llm(provider),
            None => Ok(()),
        }
    }

    pub fn check_sharing(&self) -> Result<(), String> {
        match &self.profile {
            Some(profile) => profile.check_sharing(),
            None => Ok(()),
        }
    }

    /// True when this profile asks for secret masking on the copy that leaves
    /// the app.
    pub fn redact_pii(&self) -> bool {
        self.profile.as_ref().is_some_and(|p| p.redact_pii)
    }

    pub fn allow_sharing(&self) -> bool {
        self.profile.as_ref().map_or(true, |p| p.allow_sharing)
    }

    pub fn retention_days(&self) -> Option<i64> {
        self.profile.as_ref().and_then(|p| p.retention_days)
    }

    /// The consent level and enforcement this profile imposes, if any. The
    /// consent gate asks for this instead of reading the global settings.
    pub fn consent_floor(&self) -> Option<(ConsentLevel, EnforcementMode)> {
        self.profile
            .as_ref()
            .map(|p| (p.consent_level, p.consent_enforcement))
    }

    /// Combines the profile floor, the global default, and any per-recording
    /// request into the level that actually applies.
    pub fn effective_consent(
        &self,
        global_level: ConsentLevel,
        global_enforcement: EnforcementMode,
        requested_level: Option<ConsentLevel>,
    ) -> (ConsentLevel, EnforcementMode) {
        match self.consent_floor() {
            Some((floor_level, floor_enforcement)) => (
                clamp_level(floor_level, requested_level),
                clamp_enforcement(floor_enforcement, Some(global_enforcement)),
            ),
            None => (
                requested_level.unwrap_or(global_level),
                global_enforcement,
            ),
        }
    }
}

/// The workspace default profile, or none when unset.
pub async fn workspace_default(pool: &SqlitePool) -> EffectiveProfile {
    let settings = match PrivacySettingsRepository::get(pool).await {
        Ok(Some(row)) => row,
        Ok(None) => return EffectiveProfile::none(),
        Err(e) => {
            // A read failure must not invent a policy in either direction; the
            // global settings keep governing and the gap is logged.
            log::warn!("[Profiles] failed to read privacy settings ({}); no profile applied", e);
            return EffectiveProfile::none();
        }
    };
    let Some(profile_id) = settings.default_profile_id else {
        return EffectiveProfile::none();
    };
    match PrivacyProfilesRepository::get(pool, &profile_id).await {
        Ok(Some(row)) => EffectiveProfile {
            profile: Some(PrivacyProfile::from_row(row)),
            source: ProfileSource::WorkspaceDefault,
            client_id: None,
            client_name: None,
        },
        Ok(None) => {
            log::warn!(
                "[Profiles] workspace default profile {} is missing; no profile applied",
                profile_id
            );
            EffectiveProfile::none()
        }
        Err(e) => {
            log::warn!("[Profiles] failed to load default profile ({}); no profile applied", e);
            EffectiveProfile::none()
        }
    }
}

/// The profile for a client, falling back to the workspace default.
pub async fn for_client(pool: &SqlitePool, client_id: &str) -> EffectiveProfile {
    let client = match ClientsRepository::get(pool, client_id).await {
        Ok(Some(client)) => client,
        Ok(None) => return workspace_default(pool).await,
        Err(e) => {
            log::warn!("[Profiles] failed to read client {} ({})", client_id, e);
            return workspace_default(pool).await;
        }
    };
    let Some(profile_id) = client.privacy_profile_id.clone() else {
        return workspace_default(pool).await;
    };
    match PrivacyProfilesRepository::get(pool, &profile_id).await {
        Ok(Some(row)) => EffectiveProfile {
            profile: Some(PrivacyProfile::from_row(row)),
            source: ProfileSource::ClientTag,
            client_id: Some(client.id),
            client_name: Some(client.name),
        },
        _ => workspace_default(pool).await,
    }
}

/// The profile for a meeting: its client tag first, then the workspace default.
pub async fn for_meeting(pool: &SqlitePool, meeting_id: &str) -> EffectiveProfile {
    match MeetingClientsRepository::client_for_meeting(pool, meeting_id).await {
        Ok(Some(client)) => for_client(pool, &client.id).await,
        Ok(None) => workspace_default(pool).await,
        Err(e) => {
            log::warn!("[Profiles] failed to read client tag for {} ({})", meeting_id, e);
            workspace_default(pool).await
        }
    }
}

/// The profile for a recording that has not produced a meeting row yet.
///
/// There is no client tag to read at this point, so the same suggestion logic
/// the meeting-details chip uses (attendee email domains, then the client name
/// appearing in the title) picks the client. When nothing matches, the
/// workspace default applies. Suggestion here only ever tightens or relaxes
/// policy through a profile the operator configured; it never tags the meeting.
pub async fn for_recording(
    pool: &SqlitePool,
    meeting_title: &str,
    attendees: &[String],
) -> EffectiveProfile {
    let clients = match ClientsRepository::list_with_counts(pool).await {
        Ok(clients) => clients,
        Err(e) => {
            log::warn!("[Profiles] failed to list clients ({})", e);
            return workspace_default(pool).await;
        }
    };
    if clients.is_empty() {
        return workspace_default(pool).await;
    }

    let plain: Vec<crate::database::models::Client> = clients
        .into_iter()
        .map(|c| crate::database::models::Client {
            id: c.id,
            name: c.name,
            domain: c.domain,
            notes: c.notes,
            created_at: c.created_at,
            privacy_profile_id: c.privacy_profile_id,
        })
        .collect();

    let domains: Vec<String> = attendees
        .iter()
        .filter_map(|a| crate::clients::suggest::email_domain(a))
        .collect();

    let matched = crate::clients::suggest::suggest_by_domain(&plain, &domains)
        .or_else(|| crate::clients::suggest::suggest_by_title(&plain, meeting_title));

    match matched {
        Some((client, _)) => for_client(pool, &client.id).await,
        None => workspace_default(pool).await,
    }
}

/// The consent level and enforcement in force for a recording, with the profile
/// floor applied. The consent gate and `consent_prepare_recording` both call
/// this so the pre-record sheet and the gate can never disagree.
pub async fn effective_consent_for_recording(
    pool: &SqlitePool,
    meeting_title: &str,
    attendees: &[String],
    global_level: ConsentLevel,
    global_enforcement: EnforcementMode,
    requested_level: Option<ConsentLevel>,
) -> (ConsentLevel, EnforcementMode, EffectiveProfile) {
    let effective = for_recording(pool, meeting_title, attendees).await;
    let (level, enforcement) =
        effective.effective_consent(global_level, global_enforcement, requested_level);
    (level, enforcement, effective)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::rules::ProcessingMode;
    use chrono::Utc;

    fn profile(level: ConsentLevel, enforcement: EnforcementMode) -> PrivacyProfile {
        PrivacyProfile {
            id: "profile-test".to_string(),
            name: "Test".to_string(),
            description: String::new(),
            transcription_mode: ProcessingMode::LocalOnly,
            llm_mode: ProcessingMode::LocalOnly,
            consent_level: level,
            consent_enforcement: enforcement,
            retention_days: Some(30),
            redact_pii: true,
            allow_sharing: false,
            is_builtin: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn applied(profile: PrivacyProfile) -> EffectiveProfile {
        EffectiveProfile {
            profile: Some(profile),
            source: ProfileSource::ClientTag,
            client_id: Some("client-1".to_string()),
            client_name: Some("Hale & Dorr".to_string()),
        }
    }

    #[test]
    fn no_profile_means_no_constraint_anywhere() {
        let none = EffectiveProfile::none();
        assert!(none.check_transcription("openai").is_ok());
        assert!(none.check_llm("claude").is_ok());
        assert!(none.check_sharing().is_ok());
        assert!(!none.redact_pii());
        assert!(none.allow_sharing());
        assert_eq!(none.retention_days(), None);
        assert_eq!(none.consent_floor(), None);
        assert_eq!(none.source, ProfileSource::None);
    }

    #[test]
    fn a_client_profile_constrains_every_axis() {
        let effective = applied(profile(ConsentLevel::PerSpeaker, EnforcementMode::Strict));
        assert!(effective.check_transcription("openai").is_err());
        assert!(effective.check_transcription("parakeet").is_ok());
        assert!(effective.check_llm("claude").is_err());
        assert!(effective.check_llm("ollama").is_ok());
        assert!(effective.check_sharing().is_err());
        assert!(effective.redact_pii());
        assert_eq!(effective.retention_days(), Some(30));
    }

    #[test]
    fn the_profile_replaces_the_global_consent_level() {
        let effective = applied(profile(ConsentLevel::PerSpeaker, EnforcementMode::Strict));
        let (level, enforcement) = effective.effective_consent(
            ConsentLevel::SelfOnly,
            EnforcementMode::FlagOnly,
            None,
        );
        assert_eq!(level, ConsentLevel::PerSpeaker);
        assert_eq!(enforcement, EnforcementMode::Strict);
    }

    #[test]
    fn an_operator_cannot_drop_below_the_profile_but_can_go_stricter() {
        let effective = applied(profile(ConsentLevel::Affirmative, EnforcementMode::FlagOnly));
        let (lowered, _) = effective.effective_consent(
            ConsentLevel::SelfOnly,
            EnforcementMode::FlagOnly,
            Some(ConsentLevel::SelfOnly),
        );
        assert_eq!(lowered, ConsentLevel::Affirmative);
        let (raised, _) = effective.effective_consent(
            ConsentLevel::SelfOnly,
            EnforcementMode::FlagOnly,
            Some(ConsentLevel::PerSpeaker),
        );
        assert_eq!(raised, ConsentLevel::PerSpeaker);
    }

    #[test]
    fn global_settings_still_apply_when_no_profile_resolves() {
        let none = EffectiveProfile::none();
        let (level, enforcement) = none.effective_consent(
            ConsentLevel::Notify,
            EnforcementMode::Strict,
            None,
        );
        assert_eq!(level, ConsentLevel::Notify);
        assert_eq!(enforcement, EnforcementMode::Strict);
        let (overridden, _) = none.effective_consent(
            ConsentLevel::Notify,
            EnforcementMode::FlagOnly,
            Some(ConsentLevel::SelfOnly),
        );
        assert_eq!(overridden, ConsentLevel::SelfOnly);
    }

    #[test]
    fn global_strict_enforcement_is_not_relaxed_by_a_flag_only_profile() {
        let effective = applied(profile(ConsentLevel::Notify, EnforcementMode::FlagOnly));
        let (_, enforcement) = effective.effective_consent(
            ConsentLevel::SelfOnly,
            EnforcementMode::Strict,
            None,
        );
        assert_eq!(enforcement, EnforcementMode::Strict);
    }

    #[test]
    fn the_log_line_names_the_profile_and_the_route() {
        let tagged = applied(profile(ConsentLevel::Notify, EnforcementMode::FlagOnly));
        let line = tagged.describe();
        assert!(line.contains("Test"));
        assert!(line.contains("Hale & Dorr"));
        assert_eq!(tagged.source.as_str(), "client_tag");

        let default = EffectiveProfile {
            profile: Some(profile(ConsentLevel::Notify, EnforcementMode::FlagOnly)),
            source: ProfileSource::WorkspaceDefault,
            client_id: None,
            client_name: None,
        };
        assert!(default.describe().contains("workspace default"));
        assert_eq!(default.source.as_str(), "workspace_default");

        assert!(EffectiveProfile::none().describe().contains("No privacy profile"));
    }
}
