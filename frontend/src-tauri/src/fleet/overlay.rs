//! How a managed configuration combines with what the local user chose.
//!
//! Pure functions over a `ManagedConfig` and the local value, so the interaction
//! rules can be tested exhaustively without a database.
//!
//! ## The rule, stated once
//!
//! Managed values are authoritative, but authority here means "a bound the user
//! cannot escape", not "a value that overwrites the user's". Every setting has a
//! direction of strictness, and the overlay always moves toward the stricter of
//! the two:
//!
//! | Setting | Managed value means | Local value can |
//! | --- | --- | --- |
//! | consent level | the floor | be stricter, never looser |
//! | per-speaker enforcement | the floor | be stricter, never looser |
//! | blocked keywords / domains | entries that must be present | add more, never remove these |
//! | retention days | the ceiling | be shorter, never longer |
//! | allowed providers | the whole permitted set | be a subset, never wider |
//!
//! `locked` removes the local half entirely: the managed value applies exactly, and
//! the control is read-only. That is the only case where a managed setting can make
//! a machine *less* strict than the local user asked for, and it is the case an
//! administrator explicitly opted into by naming the key.

use crate::consent::rules::{ConsentLevel, EnforcementMode};
use crate::profiles::rules::{clamp_enforcement, clamp_level};

use super::config::ManagedConfig;

/// The consent level actually in force.
pub fn effective_consent_level(managed: &ManagedConfig, local: ConsentLevel) -> ConsentLevel {
    match managed.consent_level_floor {
        None => local,
        Some(floor) if managed.is_locked("consent_level_floor") => floor,
        // The same floor semantics privacy profiles already use: the stricter of
        // the two wins, so a technician can ask for more but never less.
        Some(floor) => clamp_level(floor, Some(local)),
    }
}

/// The per-speaker enforcement actually in force.
pub fn effective_enforcement(managed: &ManagedConfig, local: EnforcementMode) -> EnforcementMode {
    match managed.consent_enforcement {
        None => local,
        Some(floor) if managed.is_locked("consent_enforcement") => floor,
        Some(floor) => clamp_enforcement(floor, Some(local)),
    }
}

/// Blocking lists: the managed entries plus whatever the operator added, with
/// case-insensitive duplicates collapsed. Locked means the managed list exactly.
pub fn merged_list(
    managed_entries: Option<&Vec<String>>,
    local: &[String],
    locked: bool,
) -> Vec<String> {
    let Some(managed_entries) = managed_entries else {
        return local.to_vec();
    };
    if locked {
        return managed_entries.clone();
    }
    let mut out = managed_entries.clone();
    for entry in local {
        if !out
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(entry))
        {
            out.push(entry.clone());
        }
    }
    out
}

/// Retention: the managed value is a ceiling, so the shorter window wins. A local
/// `None` means "keep forever", which the ceiling overrides.
pub fn effective_retention_days(managed: &ManagedConfig, local: Option<i64>) -> Option<i64> {
    match managed.retention_days {
        None => local,
        Some(managed_days) if managed.is_locked("retention_days") => Some(managed_days),
        Some(managed_days) => Some(match local {
            Some(local_days) if local_days > 0 => local_days.min(managed_days),
            _ => managed_days,
        }),
    }
}

/// Whether a provider may be used at all.
pub fn provider_allowed(allowed: Option<&Vec<String>>, provider: &str) -> bool {
    match allowed {
        None => true,
        Some(list) => list
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(provider.trim())),
    }
}

/// Whether the app may check for updates. Absent policy means yes, which is the
/// app's own default.
pub fn updates_allowed(managed: &ManagedConfig) -> bool {
    managed.updates_enabled.unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn managed(json: &str) -> ManagedConfig {
        super::super::config::parse(json).unwrap()
    }

    #[test]
    fn no_policy_leaves_every_local_choice_alone() {
        let none = managed("{}");
        assert_eq!(
            effective_consent_level(&none, ConsentLevel::SelfOnly),
            ConsentLevel::SelfOnly
        );
        assert_eq!(
            effective_enforcement(&none, EnforcementMode::FlagOnly),
            EnforcementMode::FlagOnly
        );
        assert_eq!(effective_retention_days(&none, None), None);
        assert_eq!(effective_retention_days(&none, Some(400)), Some(400));
        assert!(provider_allowed(None, "openai"));
        assert!(updates_allowed(&none));
    }

    #[test]
    fn a_consent_floor_raises_a_looser_local_choice() {
        let policy = managed(r#"{"consent_level_floor": "affirmative"}"#);
        assert_eq!(
            effective_consent_level(&policy, ConsentLevel::SelfOnly),
            ConsentLevel::Affirmative
        );
    }

    #[test]
    fn a_stricter_local_choice_survives_an_unlocked_floor() {
        let policy = managed(r#"{"consent_level_floor": "notify"}"#);
        assert_eq!(
            effective_consent_level(&policy, ConsentLevel::PerSpeaker),
            ConsentLevel::PerSpeaker
        );
    }

    #[test]
    fn locking_the_floor_pins_it_even_against_a_stricter_local_choice() {
        let policy =
            managed(r#"{"consent_level_floor": "notify", "locked": ["consent_level_floor"]}"#);
        assert_eq!(
            effective_consent_level(&policy, ConsentLevel::PerSpeaker),
            ConsentLevel::Notify
        );
    }

    #[test]
    fn a_managed_config_can_never_loosen_an_unlocked_floor() {
        // Exhaustive over the level pairs: with the key unlocked, the result is
        // never less strict than the local choice.
        let levels = [
            ConsentLevel::SelfOnly,
            ConsentLevel::Notify,
            ConsentLevel::Affirmative,
            ConsentLevel::PerSpeaker,
        ];
        for floor in levels {
            let policy = managed(&format!(
                r#"{{"consent_level_floor": "{}"}}"#,
                floor.as_str()
            ));
            for local in levels {
                let effective = effective_consent_level(&policy, local);
                assert!(
                    effective.strictness() >= local.strictness(),
                    "floor {:?} weakened local {:?} to {:?}",
                    floor,
                    local,
                    effective
                );
                assert!(effective.strictness() >= floor.strictness());
            }
        }
    }

    #[test]
    fn strict_enforcement_cannot_be_relaxed_when_the_policy_sets_it() {
        let policy = managed(r#"{"consent_enforcement": "strict"}"#);
        assert_eq!(
            effective_enforcement(&policy, EnforcementMode::FlagOnly),
            EnforcementMode::Strict
        );
    }

    #[test]
    fn an_unlocked_flag_only_policy_still_allows_a_stricter_local_choice() {
        let policy = managed(r#"{"consent_enforcement": "flag_only"}"#);
        assert_eq!(
            effective_enforcement(&policy, EnforcementMode::Strict),
            EnforcementMode::Strict
        );
    }

    #[test]
    fn blocking_lists_are_the_union_and_managed_entries_cannot_be_dropped() {
        let policy = managed(r#"{"blocked_title_keywords": ["HR", "legal"]}"#);
        let merged = merged_list(
            policy.blocked_title_keywords.as_ref(),
            &["standup".to_string(), "hr".to_string()],
            false,
        );
        // "hr" is already covered by the managed "HR", case-insensitively.
        assert_eq!(merged, vec!["HR", "legal", "standup"]);
    }

    #[test]
    fn a_locked_blocking_list_is_exactly_the_managed_one() {
        let policy = managed(
            r#"{"blocked_domains": ["clinic.example"], "locked": ["blocked_domains"]}"#,
        );
        let merged = merged_list(
            policy.blocked_domains.as_ref(),
            &["other.example".to_string()],
            true,
        );
        assert_eq!(merged, vec!["clinic.example"]);
    }

    #[test]
    fn retention_takes_the_shorter_window_and_overrides_keep_forever() {
        let policy = managed(r#"{"retention_days": 90}"#);
        assert_eq!(effective_retention_days(&policy, Some(365)), Some(90));
        assert_eq!(effective_retention_days(&policy, Some(30)), Some(30));
        assert_eq!(effective_retention_days(&policy, None), Some(90));
    }

    #[test]
    fn a_locked_retention_window_applies_exactly() {
        let policy = managed(r#"{"retention_days": 90, "locked": ["retention_days"]}"#);
        assert_eq!(effective_retention_days(&policy, Some(30)), Some(90));
    }

    #[test]
    fn a_provider_allowlist_refuses_anything_outside_it() {
        let policy = managed(r#"{"allowed_llm_providers": ["ollama", "builtin-ai"]}"#);
        let allowed = policy.allowed_llm_providers.as_ref();
        assert!(provider_allowed(allowed, "ollama"));
        assert!(provider_allowed(allowed, "Builtin-AI"));
        assert!(!provider_allowed(allowed, "openai"));
    }

    #[test]
    fn an_empty_provider_allowlist_permits_nothing() {
        // An administrator who writes [] has said "none of them", which is a real
        // if drastic policy; treating it as "all of them" would invert it.
        let policy = managed(r#"{"allowed_llm_providers": []}"#);
        assert!(!provider_allowed(policy.allowed_llm_providers.as_ref(), "ollama"));
    }

    #[test]
    fn update_checks_can_be_switched_off() {
        assert!(!updates_allowed(&managed(r#"{"updates_enabled": false}"#)));
        assert!(updates_allowed(&managed(r#"{"updates_enabled": true}"#)));
    }
}
