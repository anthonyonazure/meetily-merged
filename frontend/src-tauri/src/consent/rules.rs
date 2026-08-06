//! Pure consent logic: level parsing, level resolution, and the blocking-rule
//! matcher. No database, no Tauri, no I/O — everything here is a total function
//! over its inputs so it can be unit tested without a build of the app.

use serde::{Deserialize, Serialize};

/// How much the operator does before a recording may start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentLevel {
    /// The operator consents for themselves. Nothing is announced or prompted.
    SelfOnly,
    /// The operator is handed a disclaimer to paste and, optionally, an
    /// announcement to play, then confirms notice was given.
    Notify,
    /// The operator ticks off each named attendee before recording can start.
    Affirmative,
    /// Every distinct speaker is confirmed individually as they are identified.
    PerSpeaker,
}

impl ConsentLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelfOnly => "self_only",
            Self::Notify => "notify",
            Self::Affirmative => "affirmative",
            Self::PerSpeaker => "per_speaker",
        }
    }

    /// Unknown values fall back to the least-friction level rather than
    /// failing a recording start on a typo in the database.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "notify" => Self::Notify,
            "affirmative" => Self::Affirmative,
            "per_speaker" => Self::PerSpeaker,
            _ => Self::SelfOnly,
        }
    }

    /// Whether the level demands operator action in a sheet before recording
    /// may start. `per_speaker` prompts during the meeting, not before it, and
    /// `self_only` never prompts.
    pub fn requires_pre_record_sheet(self) -> bool {
        matches!(self, Self::Notify | Self::Affirmative)
    }

    /// How much the level asks of the operator, as an orderable rank. Used by
    /// privacy profiles, where a client's level is a floor the operator may
    /// raise for one recording but not drop below.
    pub fn strictness(self) -> u8 {
        match self {
            Self::SelfOnly => 0,
            Self::Notify => 1,
            Self::Affirmative => 2,
            Self::PerSpeaker => 3,
        }
    }
}

/// What `per_speaker` does with a speaker who has not been confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementMode {
    /// Keep the text, mark the segments as unconsented.
    FlagOnly,
    /// Withhold the text from summaries, agents, chat context, and exports.
    Strict,
}

impl EnforcementMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FlagOnly => "flag_only",
            Self::Strict => "strict",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "strict" => Self::Strict,
            _ => Self::FlagOnly,
        }
    }
}

/// Why a recording was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum BlockReason {
    /// The meeting title contains this blocked keyword.
    TitleKeyword(String),
    /// An attendee's email domain is on the blocked list.
    AttendeeDomain(String),
}

impl BlockReason {
    /// Plain mechanics, no legal framing — this string reaches the UI and the
    /// consent log.
    pub fn describe(&self) -> String {
        match self {
            Self::TitleKeyword(keyword) => {
                format!("Meeting title contains the blocked word \"{}\"", keyword)
            }
            Self::AttendeeDomain(domain) => {
                format!("An attendee is on the blocked domain \"{}\"", domain)
            }
        }
    }
}

/// Splits a stored comma-separated list into trimmed, non-empty entries.
pub fn split_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|entry| entry.trim())
        .filter(|entry| !entry.is_empty())
        .map(|entry| entry.to_string())
        .collect()
}

/// Rejoins a list for storage, dropping blanks and duplicates.
pub fn join_list(entries: &[String]) -> String {
    let mut seen: Vec<String> = Vec::new();
    for entry in entries {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(trimmed))
        {
            continue;
        }
        seen.push(trimmed.to_string());
    }
    seen.join(",")
}

/// True when `haystack` contains `needle` as a whole word, case-insensitively.
///
/// Whole-word matching is the whole point: a substring match on "hr" would
/// block "Thursday sync" and "Chris 1:1", which trains operators to override
/// the block reflexively and defeats the feature.
fn contains_word(haystack: &str, needle: &str) -> bool {
    let haystack = haystack.to_ascii_lowercase();
    let needle = needle.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return false;
    }

    // Multi-word keywords ("board review") are matched as a run of characters
    // bounded by non-alphanumerics on both sides.
    let hay: Vec<char> = haystack.chars().collect();
    let pat: Vec<char> = needle.chars().collect();
    if pat.len() > hay.len() {
        return false;
    }

    let boundary = |c: Option<&char>| match c {
        None => true,
        Some(c) => !c.is_alphanumeric(),
    };

    for start in 0..=(hay.len() - pat.len()) {
        if hay[start..start + pat.len()] != pat[..] {
            continue;
        }
        let before = if start == 0 {
            None
        } else {
            hay.get(start - 1)
        };
        let after = hay.get(start + pat.len());
        if boundary(before) && boundary(after) {
            return true;
        }
    }
    false
}

/// The domain part of an email address, lowercased. None when there is no `@`
/// or nothing after it.
pub fn email_domain(email: &str) -> Option<String> {
    let (_, domain) = email.trim().rsplit_once('@')?;
    let domain = domain.trim().trim_end_matches('>').to_ascii_lowercase();
    (!domain.is_empty()).then_some(domain)
}

/// True when `candidate` is the blocked domain or a subdomain of it.
fn domain_matches(candidate: &str, blocked: &str) -> bool {
    let candidate = candidate.trim().to_ascii_lowercase();
    let blocked = blocked.trim().trim_start_matches('@').to_ascii_lowercase();
    if blocked.is_empty() || candidate.is_empty() {
        return false;
    }
    candidate == blocked || candidate.ends_with(&format!(".{}", blocked))
}

/// The blocking-rule matcher. Returns the first reason the recording should be
/// refused, or None when nothing matches.
///
/// `attendees` may be email addresses or bare names; anything without an `@`
/// simply cannot match a domain rule.
pub fn find_block(
    title: &str,
    attendees: &[String],
    blocked_keywords: &[String],
    blocked_domains: &[String],
) -> Option<BlockReason> {
    for keyword in blocked_keywords {
        if contains_word(title, keyword) {
            return Some(BlockReason::TitleKeyword(keyword.trim().to_string()));
        }
    }

    for attendee in attendees {
        let Some(domain) = email_domain(attendee) else {
            continue;
        };
        for blocked in blocked_domains {
            if domain_matches(&domain, blocked) {
                return Some(BlockReason::AttendeeDomain(blocked.trim().to_string()));
            }
        }
    }

    None
}

/// Resolves the level actually in force: a per-meeting override wins over the
/// global default, and an unparseable override is ignored rather than silently
/// downgrading the operator's global choice.
pub fn resolve_level(global_default: ConsentLevel, meeting_override: Option<&str>) -> ConsentLevel {
    match meeting_override {
        Some(raw) if !raw.trim().is_empty() => {
            let parsed = ConsentLevel::parse(raw);
            // `parse` funnels unknown strings to SelfOnly. Only honour that as
            // an override when the operator actually asked for self_only.
            if parsed == ConsentLevel::SelfOnly
                && !raw.trim().eq_ignore_ascii_case("self_only")
            {
                global_default
            } else {
                parsed
            }
        }
        _ => global_default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn levels_round_trip_through_strings() {
        for level in [
            ConsentLevel::SelfOnly,
            ConsentLevel::Notify,
            ConsentLevel::Affirmative,
            ConsentLevel::PerSpeaker,
        ] {
            assert_eq!(ConsentLevel::parse(level.as_str()), level);
        }
        assert_eq!(ConsentLevel::parse("PER_SPEAKER"), ConsentLevel::PerSpeaker);
        assert_eq!(ConsentLevel::parse("nonsense"), ConsentLevel::SelfOnly);
        assert_eq!(EnforcementMode::parse("STRICT"), EnforcementMode::Strict);
        assert_eq!(EnforcementMode::parse(""), EnforcementMode::FlagOnly);
    }

    #[test]
    fn levels_rank_from_least_to_most_demanding() {
        assert!(ConsentLevel::SelfOnly.strictness() < ConsentLevel::Notify.strictness());
        assert!(ConsentLevel::Notify.strictness() < ConsentLevel::Affirmative.strictness());
        assert!(ConsentLevel::Affirmative.strictness() < ConsentLevel::PerSpeaker.strictness());
    }

    #[test]
    fn only_notify_and_affirmative_gate_the_start() {
        assert!(!ConsentLevel::SelfOnly.requires_pre_record_sheet());
        assert!(ConsentLevel::Notify.requires_pre_record_sheet());
        assert!(ConsentLevel::Affirmative.requires_pre_record_sheet());
        // per_speaker prompts during the meeting, so it must not block the start.
        assert!(!ConsentLevel::PerSpeaker.requires_pre_record_sheet());
    }

    #[test]
    fn blocked_keywords_match_whole_words_only() {
        let keywords = strings(&["HR", "legal", "board", "review", "termination"]);
        let none: Vec<String> = Vec::new();

        assert_eq!(
            find_block("HR check-in with Dana", &none, &keywords, &none),
            Some(BlockReason::TitleKeyword("HR".to_string()))
        );
        assert_eq!(
            find_block("Quarterly Board Meeting", &none, &keywords, &none),
            Some(BlockReason::TitleKeyword("board".to_string()))
        );
        assert_eq!(
            find_block("design review", &none, &keywords, &none),
            Some(BlockReason::TitleKeyword("review".to_string()))
        );

        // The false-positive cases that would otherwise train operators to
        // override every block.
        assert_eq!(find_block("Thursday sync", &none, &keywords, &none), None);
        assert_eq!(find_block("Chris 1:1", &none, &keywords, &none), None);
        assert_eq!(find_block("Onboarding walkthrough", &none, &keywords, &none), None);
        assert_eq!(find_block("Keyboard shopping", &none, &keywords, &none), None);
    }

    #[test]
    fn multi_word_keywords_match_across_the_space() {
        let keywords = strings(&["board review"]);
        let none: Vec<String> = Vec::new();
        assert_eq!(
            find_block("Annual board review prep", &none, &keywords, &none),
            Some(BlockReason::TitleKeyword("board review".to_string()))
        );
        assert_eq!(find_block("board sync", &none, &keywords, &none), None);
    }

    #[test]
    fn blocked_domains_match_the_domain_and_its_subdomains() {
        let keywords: Vec<String> = Vec::new();
        let domains = strings(&["clientlegal.com", "@hospital.org"]);

        assert_eq!(
            find_block(
                "Weekly sync",
                &strings(&["dana@clientlegal.com"]),
                &keywords,
                &domains
            ),
            Some(BlockReason::AttendeeDomain("clientlegal.com".to_string()))
        );
        // Subdomains count.
        assert_eq!(
            find_block(
                "Weekly sync",
                &strings(&["dana@mail.clientlegal.com"]),
                &keywords,
                &domains
            ),
            Some(BlockReason::AttendeeDomain("clientlegal.com".to_string()))
        );
        // A stored "@domain" entry is normalised.
        assert_eq!(
            find_block(
                "Weekly sync",
                &strings(&["nurse@hospital.org"]),
                &keywords,
                &domains
            ),
            Some(BlockReason::AttendeeDomain("@hospital.org".to_string()))
        );
        // Suffix lookalikes must not match.
        assert_eq!(
            find_block(
                "Weekly sync",
                &strings(&["dana@notclientlegal.com"]),
                &keywords,
                &domains
            ),
            None
        );
        // Bare names cannot match a domain rule.
        assert_eq!(
            find_block("Weekly sync", &strings(&["Dana"]), &keywords, &domains),
            None
        );
    }

    #[test]
    fn title_keywords_are_checked_before_attendee_domains() {
        let reason = find_block(
            "legal review",
            &strings(&["dana@blocked.com"]),
            &strings(&["legal"]),
            &strings(&["blocked.com"]),
        );
        assert_eq!(reason, Some(BlockReason::TitleKeyword("legal".to_string())));
    }

    #[test]
    fn empty_rule_lists_never_block() {
        let none: Vec<String> = Vec::new();
        assert_eq!(
            find_block("HR termination review", &strings(&["a@b.com"]), &none, &none),
            None
        );
    }

    #[test]
    fn per_meeting_override_wins_but_garbage_does_not_downgrade() {
        assert_eq!(
            resolve_level(ConsentLevel::SelfOnly, Some("affirmative")),
            ConsentLevel::Affirmative
        );
        assert_eq!(
            resolve_level(ConsentLevel::Affirmative, Some("self_only")),
            ConsentLevel::SelfOnly
        );
        assert_eq!(resolve_level(ConsentLevel::Notify, None), ConsentLevel::Notify);
        assert_eq!(resolve_level(ConsentLevel::Notify, Some("  ")), ConsentLevel::Notify);
        // A typo must not quietly relax the operator's global setting.
        assert_eq!(
            resolve_level(ConsentLevel::Affirmative, Some("affirmatve")),
            ConsentLevel::Affirmative
        );
    }

    #[test]
    fn lists_round_trip_and_deduplicate() {
        assert_eq!(split_list("HR, legal , ,board"), strings(&["HR", "legal", "board"]));
        assert_eq!(split_list(""), Vec::<String>::new());
        assert_eq!(
            join_list(&strings(&["HR", " hr ", "legal", ""])),
            "HR,legal"
        );
    }

    #[test]
    fn email_domains_are_extracted_lowercased() {
        assert_eq!(email_domain("Dana@Example.COM").as_deref(), Some("example.com"));
        assert_eq!(email_domain("Dana <dana@example.com>").as_deref(), Some("example.com"));
        assert_eq!(email_domain("no-at-sign"), None);
        assert_eq!(email_domain("trailing@"), None);
    }
}
