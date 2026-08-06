//! Typed view over the single `consent_settings` row.
//!
//! Everything here degrades to the least-friction defaults rather than
//! erroring: a database hiccup must not be able to block a recording, and it
//! must not be able to silently disable the blocking rules either — so the
//! defaults keep the shipped keyword list.

use crate::database::models::ConsentSettingsRow;
use crate::database::repositories::consent::ConsentSettingsRepository;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::rules::{join_list, split_list, ConsentLevel, EnforcementMode};

pub const DEFAULT_ANNOUNCEMENT: &str =
    "This meeting is being transcribed for notes. Please say so now if you object.";
pub const DEFAULT_DISCLAIMER: &str = "Heads up: I am transcribing this meeting so I have accurate notes. Let me know if you would rather I did not.";
pub const DEFAULT_BLOCKED_KEYWORDS: &[&str] = &[
    "HR",
    "legal",
    "board",
    "review",
    "termination",
    "disciplinary",
    "therapy",
    "medical",
    "privileged",
];

/// Consent settings as the UI and the gate see them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsentSettings {
    pub consent_level: ConsentLevel,
    pub per_speaker_enforcement: EnforcementMode,
    pub spoken_announcement_enabled: bool,
    pub announcement_text: String,
    pub disclaimer_text: String,
    pub blocked_title_keywords: Vec<String>,
    pub blocked_domains: Vec<String>,
}

impl Default for ConsentSettings {
    fn default() -> Self {
        Self {
            consent_level: ConsentLevel::SelfOnly,
            per_speaker_enforcement: EnforcementMode::FlagOnly,
            spoken_announcement_enabled: false,
            announcement_text: DEFAULT_ANNOUNCEMENT.to_string(),
            disclaimer_text: DEFAULT_DISCLAIMER.to_string(),
            blocked_title_keywords: DEFAULT_BLOCKED_KEYWORDS
                .iter()
                .map(|k| k.to_string())
                .collect(),
            blocked_domains: Vec::new(),
        }
    }
}

impl ConsentSettings {
    fn from_row(row: ConsentSettingsRow) -> Self {
        let announcement_text = if row.announcement_text.trim().is_empty() {
            DEFAULT_ANNOUNCEMENT.to_string()
        } else {
            row.announcement_text
        };
        let disclaimer_text = if row.disclaimer_text.trim().is_empty() {
            DEFAULT_DISCLAIMER.to_string()
        } else {
            row.disclaimer_text
        };
        Self {
            consent_level: ConsentLevel::parse(&row.consent_level),
            per_speaker_enforcement: EnforcementMode::parse(&row.per_speaker_enforcement),
            spoken_announcement_enabled: row.spoken_announcement_enabled,
            announcement_text,
            disclaimer_text,
            blocked_title_keywords: split_list(&row.blocked_title_keywords),
            blocked_domains: split_list(&row.blocked_domains),
        }
    }

    fn to_row(&self) -> ConsentSettingsRow {
        ConsentSettingsRow {
            consent_level: self.consent_level.as_str().to_string(),
            per_speaker_enforcement: self.per_speaker_enforcement.as_str().to_string(),
            spoken_announcement_enabled: self.spoken_announcement_enabled,
            announcement_text: self.announcement_text.trim().to_string(),
            disclaimer_text: self.disclaimer_text.trim().to_string(),
            blocked_title_keywords: join_list(&self.blocked_title_keywords),
            blocked_domains: join_list(&self.blocked_domains),
        }
    }
}

/// Loads the settings row, substituting defaults when it is missing or the read
/// fails. Never returns an error: the gate calls this on the recording start
/// path and must always reach a decision.
pub async fn load(pool: &SqlitePool) -> ConsentSettings {
    match ConsentSettingsRepository::get(pool).await {
        Ok(Some(row)) => ConsentSettings::from_row(row),
        Ok(None) => {
            log::warn!("[Consent] settings row missing; using defaults");
            ConsentSettings::default()
        }
        Err(e) => {
            log::warn!("[Consent] failed to read settings ({}); using defaults", e);
            ConsentSettings::default()
        }
    }
}

pub async fn save(pool: &SqlitePool, settings: &ConsentSettings) -> Result<(), String> {
    ConsentSettingsRepository::save(pool, &settings.to_row())
        .await
        .map_err(|e| format!("Failed to save consent settings: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_texts_fall_back_to_the_shipped_defaults() {
        let row = ConsentSettingsRow {
            consent_level: "notify".to_string(),
            per_speaker_enforcement: "strict".to_string(),
            spoken_announcement_enabled: true,
            announcement_text: "   ".to_string(),
            disclaimer_text: String::new(),
            blocked_title_keywords: "HR, legal".to_string(),
            blocked_domains: String::new(),
        };
        let settings = ConsentSettings::from_row(row);
        assert_eq!(settings.consent_level, ConsentLevel::Notify);
        assert_eq!(settings.per_speaker_enforcement, EnforcementMode::Strict);
        assert_eq!(settings.announcement_text, DEFAULT_ANNOUNCEMENT);
        assert_eq!(settings.disclaimer_text, DEFAULT_DISCLAIMER);
        assert_eq!(settings.blocked_title_keywords, vec!["HR", "legal"]);
        assert!(settings.blocked_domains.is_empty());
    }

    #[test]
    fn settings_round_trip_through_the_row_shape() {
        let settings = ConsentSettings {
            consent_level: ConsentLevel::PerSpeaker,
            per_speaker_enforcement: EnforcementMode::Strict,
            spoken_announcement_enabled: true,
            announcement_text: "Recording now.".to_string(),
            disclaimer_text: "Recording now.".to_string(),
            blocked_title_keywords: vec!["HR".to_string(), "hr".to_string()],
            blocked_domains: vec!["clinic.example".to_string()],
        };
        let row = settings.to_row();
        assert_eq!(row.consent_level, "per_speaker");
        assert_eq!(row.per_speaker_enforcement, "strict");
        // Duplicates collapse on the way to storage.
        assert_eq!(row.blocked_title_keywords, "HR");
        let back = ConsentSettings::from_row(row);
        assert_eq!(back.consent_level, ConsentLevel::PerSpeaker);
        assert_eq!(back.blocked_domains, vec!["clinic.example"]);
    }

    #[test]
    fn defaults_keep_the_blocking_list_populated() {
        // A failed settings read must not silently turn blocking off.
        let defaults = ConsentSettings::default();
        assert!(defaults.blocked_title_keywords.contains(&"HR".to_string()));
        assert_eq!(defaults.consent_level, ConsentLevel::SelfOnly);
    }
}
