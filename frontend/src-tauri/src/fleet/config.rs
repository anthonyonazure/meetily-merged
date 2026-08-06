//! Parsing a managed-configuration file pushed by an MDM or RMM.
//!
//! Pure: a string in, a config plus a list of warnings out. No file system, no
//! database, no Tauri, so every rule about what a policy file may say is testable
//! without a build of the app.
//!
//! ## Parsing posture
//!
//! A policy file arrives from a fleet tool, gets edited by whoever is on shift,
//! and lands on a machine nobody is watching. So parsing is forgiving in one
//! direction only: a value the app does not understand is **ignored with a
//! warning**, never guessed at, and never allowed to make the app less strict than
//! it would have been on its own. A malformed file leaves the machine on its local
//! settings and says so in `warnings`, which the settings panel shows.

use serde::Serialize;

use crate::consent::rules::{ConsentLevel, EnforcementMode};

/// Keys an administrator may list in `locked`.
pub const LOCKABLE_KEYS: &[&str] = &[
    "default_privacy_profile",
    "consent_level_floor",
    "consent_enforcement",
    "blocked_title_keywords",
    "blocked_domains",
    "retention_days",
    "allowed_transcription_providers",
    "allowed_llm_providers",
    "updates_enabled",
];

/// A parsed managed configuration. Every policy field is optional: absent means
/// "the org has no opinion, local settings govern".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ManagedConfig {
    /// Privacy profile applied as the workspace default, by name or by id.
    pub default_privacy_profile: Option<String>,
    /// The least the operator may do before recording. A floor, not a fixed value.
    pub consent_level_floor: Option<ConsentLevel>,
    /// What per-speaker consent does with an unconfirmed speaker.
    pub consent_enforcement: Option<EnforcementMode>,
    /// Title words that block a recording. Added to whatever is set locally.
    pub blocked_title_keywords: Option<Vec<String>>,
    /// Attendee email domains that block a recording. Added to the local list.
    pub blocked_domains: Option<Vec<String>>,
    /// Longest a meeting may be kept. A ceiling: a shorter local window still wins.
    pub retention_days: Option<i64>,
    /// Transcription providers the operator may select. Absent means all.
    pub allowed_transcription_providers: Option<Vec<String>>,
    /// LLM providers the operator may select. Absent means all.
    pub allowed_llm_providers: Option<Vec<String>>,
    /// Whether the app may check for updates.
    pub updates_enabled: Option<bool>,
    /// Which of the keys above the local user cannot change.
    pub locked: Vec<String>,
    /// Everything the parser could not use, in plain English, for the panel.
    pub warnings: Vec<String>,
}

impl ManagedConfig {
    pub fn is_locked(&self, key: &str) -> bool {
        self.locked.iter().any(|entry| entry == key)
    }

    /// True when the file sets no policy at all.
    pub fn is_empty(&self) -> bool {
        self.default_privacy_profile.is_none()
            && self.consent_level_floor.is_none()
            && self.consent_enforcement.is_none()
            && self.blocked_title_keywords.is_none()
            && self.blocked_domains.is_none()
            && self.retention_days.is_none()
            && self.allowed_transcription_providers.is_none()
            && self.allowed_llm_providers.is_none()
            && self.updates_enabled.is_none()
    }

    /// One line naming what the policy actually does, for the consent log.
    pub fn describe(&self) -> String {
        if self.is_empty() {
            return "Managed configuration found but it sets no policy".to_string();
        }
        let mut parts: Vec<String> = Vec::new();
        if let Some(profile) = &self.default_privacy_profile {
            parts.push(format!("default privacy profile \"{}\"", profile));
        }
        if let Some(level) = self.consent_level_floor {
            parts.push(format!("consent floor {}", level.as_str()));
        }
        if let Some(mode) = self.consent_enforcement {
            parts.push(format!("per-speaker enforcement {}", mode.as_str()));
        }
        if let Some(days) = self.retention_days {
            parts.push(format!("retention at most {} day(s)", days));
        }
        if let Some(providers) = &self.allowed_transcription_providers {
            parts.push(format!(
                "transcription limited to [{}]",
                providers.join(", ")
            ));
        }
        if let Some(providers) = &self.allowed_llm_providers {
            parts.push(format!("models limited to [{}]", providers.join(", ")));
        }
        if let Some(keywords) = &self.blocked_title_keywords {
            parts.push(format!("{} blocked title keyword(s)", keywords.len()));
        }
        if let Some(domains) = &self.blocked_domains {
            parts.push(format!("{} blocked domain(s)", domains.len()));
        }
        if let Some(enabled) = self.updates_enabled {
            parts.push(format!(
                "update checks {}",
                if enabled { "allowed" } else { "disabled" }
            ));
        }
        if !self.locked.is_empty() {
            parts.push(format!("locked: [{}]", self.locked.join(", ")));
        }
        parts.join("; ")
    }
}

fn string_list(
    value: &serde_json::Value,
    key: &str,
    warnings: &mut Vec<String>,
) -> Option<Vec<String>> {
    let Some(array) = value.as_array() else {
        warnings.push(format!("\"{}\" must be a list of text values; it was ignored.", key));
        return None;
    };
    let mut out: Vec<String> = Vec::new();
    for entry in array {
        match entry.as_str() {
            Some(text) if !text.trim().is_empty() => {
                let text = text.trim().to_string();
                if !out.iter().any(|existing| existing == &text) {
                    out.push(text);
                }
            }
            _ => warnings.push(format!(
                "\"{}\" contained an entry that is not text; that entry was ignored.",
                key
            )),
        }
    }
    Some(out)
}

/// Parses a managed-configuration file.
///
/// Returns an error only when the file is not JSON at all — at that point there is
/// no policy to apply and the caller must fall back to local settings loudly.
pub fn parse(json: &str) -> Result<ManagedConfig, String> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| format!("The managed configuration file is not valid JSON: {}", e))?;
    let Some(object) = value.as_object() else {
        return Err("The managed configuration file must contain a JSON object".to_string());
    };

    let mut config = ManagedConfig::default();
    let mut warnings: Vec<String> = Vec::new();

    for (key, value) in object {
        match key.as_str() {
            "default_privacy_profile" => match value.as_str() {
                Some(name) if !name.trim().is_empty() => {
                    config.default_privacy_profile = Some(name.trim().to_string())
                }
                _ => warnings.push(
                    "\"default_privacy_profile\" must be the name or id of a privacy profile; it was ignored."
                        .to_string(),
                ),
            },
            "consent_level_floor" => match value.as_str() {
                // ConsentLevel::parse falls back to the least strict level on an
                // unknown value, which would silently *weaken* policy here. So the
                // string is checked against the known set first and a typo is
                // reported rather than accepted.
                Some(text) if is_known_consent_level(text) => {
                    config.consent_level_floor = Some(ConsentLevel::parse(text))
                }
                _ => warnings.push(
                    "\"consent_level_floor\" must be one of self_only, notify, affirmative, per_speaker; it was ignored."
                        .to_string(),
                ),
            },
            "consent_enforcement" => match value.as_str() {
                Some(text) if matches!(text.trim().to_ascii_lowercase().as_str(), "flag_only" | "strict") => {
                    config.consent_enforcement = Some(EnforcementMode::parse(text))
                }
                _ => warnings.push(
                    "\"consent_enforcement\" must be flag_only or strict; it was ignored."
                        .to_string(),
                ),
            },
            "blocked_title_keywords" => {
                config.blocked_title_keywords = string_list(value, key, &mut warnings)
            }
            "blocked_domains" => config.blocked_domains = string_list(value, key, &mut warnings),
            "retention_days" => match value.as_i64() {
                Some(days) if days > 0 => config.retention_days = Some(days),
                _ => warnings.push(
                    "\"retention_days\" must be a whole number of days greater than zero; it was ignored."
                        .to_string(),
                ),
            },
            "allowed_transcription_providers" => {
                config.allowed_transcription_providers = string_list(value, key, &mut warnings)
            }
            "allowed_llm_providers" => {
                config.allowed_llm_providers = string_list(value, key, &mut warnings)
            }
            "telemetry" | "telemetry_enabled" => match value.as_bool() {
                Some(false) | None => {}
                Some(true) => warnings.push(
                    "\"telemetry\" was set to true. This app has no telemetry to switch on, so the setting was ignored and nothing is being sent."
                        .to_string(),
                ),
            },
            "updates_enabled" | "update_channel_enabled" => match value.as_bool() {
                Some(enabled) => config.updates_enabled = Some(enabled),
                None => warnings.push(
                    "\"updates_enabled\" must be true or false; it was ignored.".to_string(),
                ),
            },
            "locked" => {
                if let Some(keys) = string_list(value, key, &mut warnings) {
                    for entry in keys {
                        if LOCKABLE_KEYS.contains(&entry.as_str()) {
                            config.locked.push(entry);
                        } else {
                            warnings.push(format!(
                                "\"{}\" cannot be locked because it is not a managed setting; it was ignored.",
                                entry
                            ));
                        }
                    }
                }
            }
            other => warnings.push(format!(
                "\"{}\" is not a setting this version understands; it was ignored.",
                other
            )),
        }
    }

    // Locking a key the file does not set would make a control read-only with
    // nothing behind it, which reads to the user as a bug.
    config.locked.retain(|key| {
        let set = match key.as_str() {
            "default_privacy_profile" => config.default_privacy_profile.is_some(),
            "consent_level_floor" => config.consent_level_floor.is_some(),
            "consent_enforcement" => config.consent_enforcement.is_some(),
            "blocked_title_keywords" => config.blocked_title_keywords.is_some(),
            "blocked_domains" => config.blocked_domains.is_some(),
            "retention_days" => config.retention_days.is_some(),
            "allowed_transcription_providers" => config.allowed_transcription_providers.is_some(),
            "allowed_llm_providers" => config.allowed_llm_providers.is_some(),
            "updates_enabled" => config.updates_enabled.is_some(),
            _ => false,
        };
        if !set {
            warnings.push(format!(
                "\"{}\" is listed as locked but the file does not set it; the lock was ignored.",
                key
            ));
        }
        set
    });

    config.warnings = warnings;
    Ok(config)
}

fn is_known_consent_level(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "self_only" | "notify" | "affirmative" | "per_speaker"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_policy_file_parses_every_key() {
        let config = parse(
            r#"{
                "default_privacy_profile": "Strict",
                "consent_level_floor": "per_speaker",
                "consent_enforcement": "strict",
                "blocked_title_keywords": ["HR", "legal"],
                "blocked_domains": ["clinic.example"],
                "retention_days": 90,
                "allowed_transcription_providers": ["localWhisper"],
                "allowed_llm_providers": ["ollama", "builtin-ai"],
                "telemetry": false,
                "updates_enabled": false,
                "locked": ["consent_level_floor", "allowed_llm_providers"]
            }"#,
        )
        .unwrap();

        assert_eq!(config.default_privacy_profile.as_deref(), Some("Strict"));
        assert_eq!(config.consent_level_floor, Some(ConsentLevel::PerSpeaker));
        assert_eq!(config.consent_enforcement, Some(EnforcementMode::Strict));
        assert_eq!(
            config.blocked_title_keywords,
            Some(vec!["HR".to_string(), "legal".to_string()])
        );
        assert_eq!(config.retention_days, Some(90));
        assert_eq!(config.updates_enabled, Some(false));
        assert!(config.is_locked("consent_level_floor"));
        assert!(config.is_locked("allowed_llm_providers"));
        assert!(!config.is_locked("retention_days"));
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    }

    #[test]
    fn a_file_that_is_not_json_is_an_error_not_an_empty_policy() {
        assert!(parse("this is not json").is_err());
        assert!(parse("[1, 2, 3]").is_err());
    }

    #[test]
    fn an_empty_object_is_a_valid_policy_that_sets_nothing() {
        let config = parse("{}").unwrap();
        assert!(config.is_empty());
        assert!(config.warnings.is_empty());
    }

    #[test]
    fn a_misspelled_consent_level_is_reported_rather_than_silently_weakened() {
        // ConsentLevel::parse would turn "per-speaker" into self_only, which is the
        // loosest level. Accepting that would let a typo in a policy file quietly
        // switch consent off across a fleet.
        let config = parse(r#"{"consent_level_floor": "per-speaker"}"#).unwrap();
        assert_eq!(config.consent_level_floor, None);
        assert_eq!(config.warnings.len(), 1);
        assert!(config.warnings[0].contains("consent_level_floor"));
    }

    #[test]
    fn telemetry_cannot_be_switched_on_and_says_so() {
        let config = parse(r#"{"telemetry": true}"#).unwrap();
        assert_eq!(config.warnings.len(), 1);
        assert!(config.warnings[0].contains("no telemetry"));
        // Setting it to false is simply the state the app is already in.
        assert!(parse(r#"{"telemetry": false}"#).unwrap().warnings.is_empty());
    }

    #[test]
    fn an_unknown_key_is_ignored_with_a_warning_rather_than_failing_the_file() {
        let config = parse(r#"{"future_setting": 1, "retention_days": 30}"#).unwrap();
        assert_eq!(config.retention_days, Some(30));
        assert_eq!(config.warnings.len(), 1);
        assert!(config.warnings[0].contains("future_setting"));
    }

    #[test]
    fn a_nonsense_retention_window_is_ignored() {
        for json in [r#"{"retention_days": 0}"#, r#"{"retention_days": -5}"#, r#"{"retention_days": "90"}"#] {
            let config = parse(json).unwrap();
            assert_eq!(config.retention_days, None, "{}", json);
            assert_eq!(config.warnings.len(), 1);
        }
    }

    #[test]
    fn locking_a_key_the_file_does_not_set_is_dropped() {
        let config = parse(r#"{"locked": ["retention_days"]}"#).unwrap();
        assert!(config.locked.is_empty());
        assert!(config.warnings.iter().any(|w| w.contains("does not set it")));
    }

    #[test]
    fn locking_something_that_is_not_a_managed_setting_is_rejected() {
        let config = parse(r#"{"retention_days": 30, "locked": ["everything"]}"#).unwrap();
        assert!(config.locked.is_empty());
        assert!(config.warnings.iter().any(|w| w.contains("everything")));
    }

    #[test]
    fn duplicate_list_entries_collapse_and_blanks_are_dropped() {
        let config = parse(r#"{"blocked_domains": ["a.test", "a.test", "  ", 7]}"#).unwrap();
        assert_eq!(config.blocked_domains, Some(vec!["a.test".to_string()]));
        // The blank and the number each earn a warning.
        assert_eq!(config.warnings.len(), 2);
    }

    #[test]
    fn a_list_that_is_not_a_list_is_ignored() {
        let config = parse(r#"{"blocked_domains": "a.test"}"#).unwrap();
        assert_eq!(config.blocked_domains, None);
        assert_eq!(config.warnings.len(), 1);
    }

    #[test]
    fn the_description_names_the_policy_in_plain_english() {
        let config = parse(
            r#"{"consent_level_floor": "notify", "retention_days": 30, "locked": ["retention_days"]}"#,
        )
        .unwrap();
        let described = config.describe();
        assert!(described.contains("consent floor notify"));
        assert!(described.contains("retention at most 30 day(s)"));
        assert!(described.contains("locked: [retention_days]"));
    }

    #[test]
    fn an_empty_policy_describes_itself_as_setting_nothing() {
        assert!(parse("{}").unwrap().describe().contains("sets no policy"));
    }
}
