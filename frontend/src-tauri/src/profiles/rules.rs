//! Pure privacy-profile logic: mode parsing, provider classification, refusal
//! wording, and retention arithmetic. No database, no Tauri, no I/O — every
//! function here is total over its inputs so it can be unit tested without a
//! build of the app.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::consent::rules::{ConsentLevel, EnforcementMode};
use crate::database::models::PrivacyProfileRow;

/// Error prefix the frontend matches on so it can explain the refusal instead
/// of parsing prose. Mirrors the consent gate's `CONSENT_BLOCKED`.
pub const ERR_PROFILE_BLOCKED: &str = "PROFILE_BLOCKED";

/// Ids of the three shipped profiles. These rows cannot be deleted; renaming
/// and editing them is allowed, and duplicating one produces an ordinary
/// custom profile.
pub const BUILTIN_STRICT: &str = "profile-builtin-strict";
pub const BUILTIN_STANDARD: &str = "profile-builtin-standard";
pub const BUILTIN_OPEN: &str = "profile-builtin-open";
pub const BUILTIN_IDS: &[&str] = &[BUILTIN_STRICT, BUILTIN_STANDARD, BUILTIN_OPEN];

pub fn is_builtin_id(id: &str) -> bool {
    BUILTIN_IDS.contains(&id)
}

/// Where processing is allowed to happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingMode {
    /// Only providers that run on this machine.
    LocalOnly,
    /// Cloud providers are allowed as well.
    CloudAllowed,
}

impl ProcessingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalOnly => "local_only",
            Self::CloudAllowed => "cloud_allowed",
        }
    }

    /// Unknown values parse as the restrictive mode. A typo in the database
    /// must not quietly widen what a profile permits.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "cloud_allowed" | "cloud" | "allowed" => Self::CloudAllowed,
            _ => Self::LocalOnly,
        }
    }
}

// ---------------------------------------------------------------------------
// Provider classification
// ---------------------------------------------------------------------------

/// Transcription providers that run entirely on this machine. Matched against
/// the `provider` value stored in `transcript_settings`.
const LOCAL_TRANSCRIPTION: &[&str] = &["localwhisper", "whisper", "parakeet", "qwenasr"];

/// LLM providers that run entirely on this machine. Matched against the
/// provider strings `summary::llm_client::LLMProvider::from_str` accepts.
const LOCAL_LLM: &[&str] = &["ollama", "builtin-ai", "local-llama", "localllama", "builtin"];

fn normalize(provider: &str) -> String {
    provider
        .trim()
        .to_ascii_lowercase()
        .replace(['_', ' '], "-")
}

/// True when this transcription provider keeps audio on the machine.
///
/// Unrecognised providers are treated as cloud: a provider this build does not
/// know about is exactly the case where refusing is the safe answer, and the
/// operator sees a message naming the provider.
pub fn transcription_is_local(provider: &str) -> bool {
    let key = normalize(provider).replace('-', "");
    LOCAL_TRANSCRIPTION.contains(&key.as_str())
}

/// True when this LLM provider runs on the machine. `custom-openai` is treated
/// as cloud even when it points at localhost, because nothing here can verify
/// where that endpoint actually is.
pub fn llm_is_local(provider: &str) -> bool {
    let key = normalize(provider);
    LOCAL_LLM.contains(&key.as_str())
}

// ---------------------------------------------------------------------------
// Profile
// ---------------------------------------------------------------------------

/// A privacy profile as the resolver, the enforcement points, and the UI see
/// it. Serialised with snake_case enum values, matching `types/privacy.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyProfile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub transcription_mode: ProcessingMode,
    pub llm_mode: ProcessingMode,
    pub consent_level: ConsentLevel,
    pub consent_enforcement: EnforcementMode,
    /// None means meetings are kept until someone deletes them.
    pub retention_days: Option<i64>,
    pub redact_pii: bool,
    pub allow_sharing: bool,
    pub is_builtin: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl PrivacyProfile {
    pub fn from_row(row: PrivacyProfileRow) -> Self {
        // A built-in stays a built-in however it was renamed, and even if the
        // stored flag was lost in a database round trip.
        let is_builtin = row.is_builtin || is_builtin_id(&row.id);
        Self {
            id: row.id,
            name: row.name,
            description: row.description,
            transcription_mode: ProcessingMode::parse(&row.transcription_mode),
            llm_mode: ProcessingMode::parse(&row.llm_mode),
            consent_level: ConsentLevel::parse(&row.consent_level),
            consent_enforcement: EnforcementMode::parse(&row.consent_enforcement),
            // A stored zero or negative day count would mean "purge everything
            // immediately", which is never what an operator meant to type.
            retention_days: row.retention_days.filter(|days| *days > 0),
            redact_pii: row.redact_pii,
            allow_sharing: row.allow_sharing,
            is_builtin,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    /// Refusal for a transcription provider this profile does not permit.
    pub fn check_transcription(&self, provider: &str) -> Result<(), String> {
        if self.transcription_mode == ProcessingMode::CloudAllowed
            || transcription_is_local(provider)
        {
            return Ok(());
        }
        Err(format!(
            "{}: profile \"{}\" allows on-device transcription only, and \"{}\" sends audio to a service off this machine. \
Pick a local model in Settings → Transcription, or change the profile.",
            ERR_PROFILE_BLOCKED, self.name, provider
        ))
    }

    /// Refusal for an LLM provider this profile does not permit.
    pub fn check_llm(&self, provider: &str) -> Result<(), String> {
        if self.llm_mode == ProcessingMode::CloudAllowed || llm_is_local(provider) {
            return Ok(());
        }
        Err(format!(
            "{}: profile \"{}\" allows on-device models only, and \"{}\" runs off this machine. \
Pick Ollama or the built-in model in Settings → Summary, or change the profile.",
            ERR_PROFILE_BLOCKED, self.name, provider
        ))
    }

    /// Refusal for the Slack / Teams / Outlook-draft share actions.
    pub fn check_sharing(&self) -> Result<(), String> {
        if self.allow_sharing {
            return Ok(());
        }
        Err(format!(
            "{}: profile \"{}\" has the share actions turned off. Copy or export the summary instead, or change the profile.",
            ERR_PROFILE_BLOCKED, self.name
        ))
    }
}

// ---------------------------------------------------------------------------
// Consent coordination
// ---------------------------------------------------------------------------

/// Combines a profile's consent level with an operator's per-recording choice.
///
/// The profile acts as a floor, not a fixed value: the operator can ask for a
/// stricter level for one recording but cannot drop below what the client's
/// profile sets. Lowering it silently is the failure mode this whole feature
/// exists to prevent.
pub fn clamp_level(profile_level: ConsentLevel, requested: Option<ConsentLevel>) -> ConsentLevel {
    match requested {
        Some(requested) if requested.strictness() > profile_level.strictness() => requested,
        _ => profile_level,
    }
}

/// Strict enforcement is likewise a floor: a profile that withholds
/// unconfirmed speakers cannot be relaxed to flagging for one recording.
pub fn clamp_enforcement(
    profile_mode: EnforcementMode,
    requested: Option<EnforcementMode>,
) -> EnforcementMode {
    match (profile_mode, requested) {
        (EnforcementMode::Strict, _) => EnforcementMode::Strict,
        (_, Some(requested)) => requested,
        (base, None) => base,
    }
}

// ---------------------------------------------------------------------------
// Retention arithmetic
// ---------------------------------------------------------------------------

/// Age of a meeting in whole days.
pub fn age_days(created_at: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    now.signed_duration_since(created_at).num_days()
}

/// True when a meeting has outlived its profile's retention window.
///
/// The comparison is strictly greater than, so a meeting is purged the day
/// *after* the window closes rather than on its last day.
pub fn is_expired(created_at: DateTime<Utc>, retention_days: i64, now: DateTime<Utc>) -> bool {
    if retention_days <= 0 {
        return false;
    }
    now.signed_duration_since(created_at) > Duration::days(retention_days)
}

/// Days until this meeting becomes purgeable. Negative when it already is.
pub fn days_until_purge(
    created_at: DateTime<Utc>,
    retention_days: i64,
    now: DateTime<Utc>,
) -> i64 {
    retention_days - age_days(created_at, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str, transcription: ProcessingMode, llm: ProcessingMode) -> PrivacyProfile {
        PrivacyProfile {
            id: "profile-test".to_string(),
            name: name.to_string(),
            description: String::new(),
            transcription_mode: transcription,
            llm_mode: llm,
            consent_level: ConsentLevel::SelfOnly,
            consent_enforcement: EnforcementMode::FlagOnly,
            retention_days: None,
            redact_pii: false,
            allow_sharing: true,
            is_builtin: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn modes_parse_and_default_to_the_restrictive_one() {
        assert_eq!(ProcessingMode::parse("cloud_allowed"), ProcessingMode::CloudAllowed);
        assert_eq!(ProcessingMode::parse("  CLOUD_ALLOWED "), ProcessingMode::CloudAllowed);
        assert_eq!(ProcessingMode::parse("local_only"), ProcessingMode::LocalOnly);
        // Typos and blanks must not widen what a profile permits.
        assert_eq!(ProcessingMode::parse("cloudallowd"), ProcessingMode::LocalOnly);
        assert_eq!(ProcessingMode::parse(""), ProcessingMode::LocalOnly);
        assert_eq!(ProcessingMode::LocalOnly.as_str(), "local_only");
    }

    #[test]
    fn local_transcription_providers_are_recognised() {
        for provider in ["localWhisper", "parakeet", "qwenAsr", "QWEN_ASR", " whisper "] {
            assert!(transcription_is_local(provider), "{provider} should be local");
        }
    }

    #[test]
    fn cloud_and_unknown_transcription_providers_are_not_local() {
        for provider in ["openai", "remote", "deepgram", "groq", "brand-new-thing", ""] {
            assert!(!transcription_is_local(provider), "{provider} should not be local");
        }
    }

    #[test]
    fn local_llm_providers_are_recognised() {
        for provider in ["ollama", "OLLAMA", "builtin-ai", "local-llama", "localllama"] {
            assert!(llm_is_local(provider), "{provider} should be local");
        }
    }

    #[test]
    fn cloud_llm_providers_including_custom_endpoints_are_not_local() {
        for provider in ["openai", "claude", "groq", "openrouter", "custom-openai", "gemini", ""] {
            assert!(!llm_is_local(provider), "{provider} should not be local");
        }
    }

    #[test]
    fn local_only_transcription_refuses_cloud_and_names_the_provider() {
        let strict = profile("Strict", ProcessingMode::LocalOnly, ProcessingMode::LocalOnly);
        assert!(strict.check_transcription("parakeet").is_ok());
        let error = strict.check_transcription("openai").unwrap_err();
        assert!(error.starts_with(ERR_PROFILE_BLOCKED));
        assert!(error.contains("Strict"));
        assert!(error.contains("openai"));
    }

    #[test]
    fn cloud_allowed_permits_everything() {
        let open = profile("Open", ProcessingMode::CloudAllowed, ProcessingMode::CloudAllowed);
        assert!(open.check_transcription("openai").is_ok());
        assert!(open.check_transcription("remote").is_ok());
        assert!(open.check_llm("claude").is_ok());
    }

    #[test]
    fn local_only_llm_refuses_every_cloud_provider() {
        let strict = profile("Strict", ProcessingMode::LocalOnly, ProcessingMode::LocalOnly);
        for provider in ["claude", "openai", "groq", "openrouter", "custom-openai"] {
            let error = strict.check_llm(provider).unwrap_err();
            assert!(error.starts_with(ERR_PROFILE_BLOCKED), "{provider}");
            assert!(error.contains(provider), "{provider}");
        }
        assert!(strict.check_llm("ollama").is_ok());
        assert!(strict.check_llm("builtin-ai").is_ok());
    }

    #[test]
    fn sharing_refusal_mentions_the_profile_and_the_alternative() {
        let mut strict = profile("Strict", ProcessingMode::LocalOnly, ProcessingMode::LocalOnly);
        strict.allow_sharing = false;
        let error = strict.check_sharing().unwrap_err();
        assert!(error.starts_with(ERR_PROFILE_BLOCKED));
        assert!(error.contains("Strict"));
        strict.allow_sharing = true;
        assert!(strict.check_sharing().is_ok());
    }

    #[test]
    fn a_profile_level_is_a_floor_not_a_fixed_value() {
        // The operator may go stricter for one recording.
        assert_eq!(
            clamp_level(ConsentLevel::Notify, Some(ConsentLevel::PerSpeaker)),
            ConsentLevel::PerSpeaker
        );
        // ...but cannot drop below the profile.
        assert_eq!(
            clamp_level(ConsentLevel::PerSpeaker, Some(ConsentLevel::SelfOnly)),
            ConsentLevel::PerSpeaker
        );
        assert_eq!(clamp_level(ConsentLevel::Affirmative, None), ConsentLevel::Affirmative);
    }

    #[test]
    fn strict_enforcement_cannot_be_relaxed_per_recording() {
        assert_eq!(
            clamp_enforcement(EnforcementMode::Strict, Some(EnforcementMode::FlagOnly)),
            EnforcementMode::Strict
        );
        assert_eq!(
            clamp_enforcement(EnforcementMode::FlagOnly, Some(EnforcementMode::Strict)),
            EnforcementMode::Strict
        );
        assert_eq!(
            clamp_enforcement(EnforcementMode::FlagOnly, None),
            EnforcementMode::FlagOnly
        );
    }

    #[test]
    fn expiry_needs_the_window_to_be_fully_past() {
        let now = Utc::now();
        let created = now - Duration::days(90);
        assert!(!is_expired(created, 90, now), "on the boundary day nothing is purged");
        assert!(is_expired(now - Duration::days(91), 90, now));
        assert!(!is_expired(now - Duration::days(10), 90, now));
    }

    #[test]
    fn zero_or_negative_retention_never_expires_anything() {
        let now = Utc::now();
        let created = now - Duration::days(4000);
        assert!(!is_expired(created, 0, now));
        assert!(!is_expired(created, -5, now));
    }

    #[test]
    fn zero_retention_days_in_the_database_reads_as_keep_forever() {
        let row = PrivacyProfileRow {
            id: "profile-x".to_string(),
            name: "X".to_string(),
            description: String::new(),
            transcription_mode: "local_only".to_string(),
            llm_mode: "local_only".to_string(),
            consent_level: "notify".to_string(),
            consent_enforcement: "strict".to_string(),
            retention_days: Some(0),
            redact_pii: true,
            allow_sharing: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            is_builtin: false,
        };
        let parsed = PrivacyProfile::from_row(row);
        assert_eq!(parsed.retention_days, None);
        assert_eq!(parsed.consent_level, ConsentLevel::Notify);
        assert_eq!(parsed.consent_enforcement, EnforcementMode::Strict);
        assert!(parsed.redact_pii);
        assert!(!parsed.allow_sharing);
    }

    #[test]
    fn builtin_rows_are_recognised_by_id_even_if_the_flag_is_lost() {
        let row = PrivacyProfileRow {
            id: BUILTIN_STRICT.to_string(),
            name: "Renamed".to_string(),
            description: String::new(),
            transcription_mode: "local_only".to_string(),
            llm_mode: "local_only".to_string(),
            consent_level: "per_speaker".to_string(),
            consent_enforcement: "strict".to_string(),
            retention_days: Some(90),
            redact_pii: true,
            allow_sharing: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            is_builtin: false,
        };
        assert!(PrivacyProfile::from_row(row).is_builtin);
        assert!(is_builtin_id(BUILTIN_OPEN));
        assert!(!is_builtin_id("profile-custom-1"));
    }

    #[test]
    fn countdown_to_purge_is_reported_in_days() {
        let now = Utc::now();
        assert_eq!(days_until_purge(now - Duration::days(80), 90, now), 10);
        assert_eq!(days_until_purge(now - Duration::days(95), 90, now), -5);
        assert_eq!(age_days(now - Duration::days(3), now), 3);
    }
}
