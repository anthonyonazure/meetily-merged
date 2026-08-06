//! Meeting types, tolerant parsing of the classifier's answer, and the
//! type-to-template mapping.
//!
//! Pure: the LLM call lives in `classify.rs` and the persistence in the
//! repository, so the parsing of a small local model's loosely-formatted reply can
//! be tested against every shape it actually produces.

use serde::{Deserialize, Serialize};

/// Below this, the classification is recorded but not allowed to pick a template.
///
/// The number is a judgement about consequences rather than about the model: a
/// wrong type is a wrong-shaped summary, which costs a regeneration. Two thirds
/// confident is the point where that trade stops being worth it.
pub const MIN_CONFIDENCE_FOR_TEMPLATE: f64 = 0.6;

/// The workspace scope sentinel for `meeting_type_templates.client_id`. Empty
/// string rather than NULL, because SQLite treats NULLs in a unique index as
/// distinct and would accept conflicting workspace rows.
pub const WORKSPACE_SCOPE: &str = "";

/// What kind of meeting this was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingType {
    Discovery,
    Status,
    Planning,
    Incident,
    Review,
    OneOnOne,
    Sales,
    Other,
}

impl MeetingType {
    pub const ALL: &'static [MeetingType] = &[
        Self::Discovery,
        Self::Status,
        Self::Planning,
        Self::Incident,
        Self::Review,
        Self::OneOnOne,
        Self::Sales,
        Self::Other,
    ];

    /// The stored form. Stable: it is a database value and a mapping key.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Discovery => "discovery",
            Self::Status => "status",
            Self::Planning => "planning",
            Self::Incident => "incident",
            Self::Review => "review",
            Self::OneOnOne => "one_on_one",
            Self::Sales => "sales",
            Self::Other => "other",
        }
    }

    /// What the UI shows.
    pub fn label(self) -> &'static str {
        match self {
            Self::Discovery => "Discovery",
            Self::Status => "Status / check-in",
            Self::Planning => "Planning",
            Self::Incident => "Incident / troubleshooting",
            Self::Review => "Review",
            Self::OneOnOne => "One-on-one",
            Self::Sales => "Sales",
            Self::Other => "Other",
        }
    }

    /// One line the classifier prompt uses, and the settings screen reuses.
    pub fn description(self) -> &'static str {
        match self {
            Self::Discovery => "a first or exploratory conversation, gathering requirements or scoping work",
            Self::Status => "a recurring check-in on progress, tickets, or open work",
            Self::Planning => "deciding what to do and when: roadmaps, projects, schedules",
            Self::Incident => "diagnosing something broken, an outage, or a live troubleshooting session",
            Self::Review => "looking back at a period or a deliverable: a service review, retrospective, or QBR",
            Self::OneOnOne => "a private conversation between two people about their work",
            Self::Sales => "a commercial conversation: pricing, proposals, renewals, negotiation",
            Self::Other => "anything that does not fit the other types",
        }
    }

    /// Parses a stored or model-supplied label. Accepts the synonyms a small
    /// local model reaches for, and the loose punctuation it uses.
    pub fn parse(value: &str) -> Option<Self> {
        let normalized: String = value
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        let normalized = normalized.trim_matches('_').to_string();

        // Exact stored forms first.
        if let Some(found) = Self::ALL.iter().find(|t| t.as_str() == normalized) {
            return Some(*found);
        }

        // Then the synonyms. Ordered so more specific phrases win: "check_in"
        // must not be reached through a substring of something else.
        const SYNONYMS: &[(&str, MeetingType)] = &[
            ("one_on_one", MeetingType::OneOnOne),
            ("1_on_1", MeetingType::OneOnOne),
            ("1_1", MeetingType::OneOnOne),
            ("one_to_one", MeetingType::OneOnOne),
            ("oneonone", MeetingType::OneOnOne),
            ("check_in", MeetingType::Status),
            ("checkin", MeetingType::Status),
            ("standup", MeetingType::Status),
            ("stand_up", MeetingType::Status),
            ("status_update", MeetingType::Status),
            ("troubleshooting", MeetingType::Incident),
            ("troubleshoot", MeetingType::Incident),
            ("outage", MeetingType::Incident),
            ("postmortem", MeetingType::Incident),
            ("post_mortem", MeetingType::Incident),
            ("retrospective", MeetingType::Review),
            ("retro", MeetingType::Review),
            ("qbr", MeetingType::Review),
            ("service_review", MeetingType::Review),
            ("kickoff", MeetingType::Discovery),
            ("kick_off", MeetingType::Discovery),
            ("scoping", MeetingType::Discovery),
            ("requirements", MeetingType::Discovery),
            ("intro", MeetingType::Discovery),
            ("discovery_call", MeetingType::Discovery),
            ("roadmap", MeetingType::Planning),
            ("sprint_planning", MeetingType::Planning),
            ("plan", MeetingType::Planning),
            ("sales_call", MeetingType::Sales),
            ("pricing", MeetingType::Sales),
            ("proposal", MeetingType::Sales),
            ("renewal", MeetingType::Sales),
            ("negotiation", MeetingType::Sales),
            ("unknown", MeetingType::Other),
            ("none", MeetingType::Other),
        ];

        if let Some((_, found)) = SYNONYMS.iter().find(|(word, _)| *word == normalized) {
            return Some(*found);
        }

        // Last resort: the label appears somewhere in a longer answer, e.g.
        // "type_incident_troubleshooting". Longest match wins so "review" inside
        // "service_review" does not beat a more specific synonym.
        let mut best: Option<(usize, MeetingType)> = None;
        for (word, kind) in SYNONYMS
            .iter()
            .map(|(w, k)| (*w, *k))
            .chain(Self::ALL.iter().map(|t| (t.as_str(), *t)))
        {
            if normalized.contains(word) && best.map(|(len, _)| word.len() > len).unwrap_or(true) {
                best = Some((word.len(), kind));
            }
        }
        best.map(|(_, kind)| kind)
    }
}

/// Where a classification came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeSource {
    Model,
    /// Set by a person. Never overwritten by a later model run.
    Manual,
}

impl TypeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Manual => "manual",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "manual" => Self::Manual,
            _ => Self::Model,
        }
    }
}

/// A classification: the type, how sure the model said it was, and who decided.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Classification {
    pub meeting_type: MeetingType,
    pub confidence: f64,
    pub source: TypeSource,
}

impl Classification {
    /// Whether this classification is trusted enough to choose a template.
    pub fn is_confident(&self) -> bool {
        self.source == TypeSource::Manual || self.confidence >= MIN_CONFIDENCE_FOR_TEMPLATE
    }
}

/// Clamps a confidence into 0.0-1.0, treating a percentage as a percentage.
pub fn clamp_confidence(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    // A model asked for 0.0-1.0 sometimes answers "85".
    let value = if value > 1.0 && value <= 100.0 {
        value / 100.0
    } else {
        value
    };
    value.clamp(0.0, 1.0)
}

/// Parses the classifier's reply.
///
/// Deliberately forgiving, in three descending steps, because a 3-billion-parameter
/// local model asked for JSON will sometimes answer with prose:
///
/// 1. JSON with a type/label/category field.
/// 2. A `key: value` line anywhere in the text.
/// 3. Any recognisable type word in the whole reply.
///
/// Returns None only when no type word appears at all, which the caller treats as
/// "not classified" rather than as `Other` — a failed classification and a genuine
/// "other" are different facts.
pub fn parse_reply(raw: &str) -> Option<Classification> {
    let cleaned = strip_code_fences(raw);

    // 1. JSON.
    if let Some(found) = parse_json_reply(&cleaned) {
        return Some(found);
    }

    // 2. A labelled line.
    let confidence = extract_confidence(&cleaned);
    for line in cleaned.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_lowercase();
        if matches!(key.as_str(), "type" | "label" | "category" | "meeting_type" | "meeting type") {
            if let Some(meeting_type) = MeetingType::parse(value) {
                return Some(Classification {
                    meeting_type,
                    confidence: confidence.unwrap_or(0.5),
                    source: TypeSource::Model,
                });
            }
        }
    }

    // 3. Any type word in the reply. A single bare word is the common case.
    MeetingType::parse(&cleaned).map(|meeting_type| Classification {
        meeting_type,
        confidence: confidence.unwrap_or(0.5),
        source: TypeSource::Model,
    })
}

fn strip_code_fences(raw: &str) -> String {
    let trimmed = raw.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }
    let without_open = trimmed.trim_start_matches('`');
    // Drop a language tag on the opening fence.
    let body = match without_open.find('\n') {
        Some(newline) => &without_open[newline + 1..],
        None => without_open,
    };
    body.trim_end_matches('`').trim().to_string()
}

fn parse_json_reply(text: &str) -> Option<Classification> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&text[start..=end]).ok()?;
    let object = value.as_object()?;

    let label = ["type", "meeting_type", "label", "category", "meetingType"]
        .iter()
        .find_map(|key| object.get(*key).and_then(|v| v.as_str()))?;
    let meeting_type = MeetingType::parse(label)?;

    let confidence = ["confidence", "score", "certainty"]
        .iter()
        .find_map(|key| object.get(*key))
        .and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.trim().trim_end_matches('%').parse().ok()))
        })
        .map(clamp_confidence)
        .unwrap_or(0.5);

    Some(Classification {
        meeting_type,
        confidence,
        source: TypeSource::Model,
    })
}

/// Finds a confidence figure in free text: `0.82`, `82%`, `confidence: 0.7`.
fn extract_confidence(text: &str) -> Option<f64> {
    let lowered = text.to_lowercase();
    let anchor = ["confidence", "score", "certainty"]
        .iter()
        .find_map(|key| lowered.find(key).map(|at| at + key.len()));
    let haystack = match anchor {
        Some(at) => &lowered[at..],
        None => lowered.as_str(),
    };

    let mut number = String::new();
    let mut found: Option<f64> = None;
    for ch in haystack.chars() {
        if ch.is_ascii_digit() || (ch == '.' && !number.is_empty()) {
            number.push(ch);
        } else if !number.is_empty() {
            if let Ok(value) = number.parse::<f64>() {
                let value = if ch == '%' { value / 100.0 } else { value };
                found = Some(clamp_confidence(value));
                break;
            }
            number.clear();
        }
    }
    if found.is_none() && !number.is_empty() {
        found = number.parse::<f64>().ok().map(clamp_confidence);
    }
    found
}

/// One row of the type-to-template mapping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeTemplateMapping {
    pub meeting_type: MeetingType,
    /// None for the workspace mapping, Some for a client override.
    pub client_id: Option<String>,
    pub template_id: String,
}

/// How a template came to be chosen, so the UI can always say which and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateChoiceSource {
    /// A mapping for this client and this meeting type.
    ClientMapping,
    /// The workspace mapping for this meeting type.
    WorkspaceMapping,
    /// No mapping applied, so the caller's own choice stands.
    Requested,
    /// A mapping exists but the classification was not confident enough.
    LowConfidence,
    /// The meeting has no classification yet.
    NotClassified,
}

impl TemplateChoiceSource {
    /// The wire form, matching the serde rename so the string a command returns
    /// and the string inside a serialized struct are the same value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClientMapping => "client_mapping",
            Self::WorkspaceMapping => "workspace_mapping",
            Self::Requested => "requested",
            Self::LowConfidence => "low_confidence",
            Self::NotClassified => "not_classified",
        }
    }
}

/// The chosen template and the reasoning behind it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateChoice {
    pub template_id: String,
    pub source: TemplateChoiceSource,
    pub meeting_type: Option<MeetingType>,
    pub confidence: Option<f64>,
}

/// Resolves which template a summary should use.
///
/// `requested` is what the caller asked for and is always the fallback, so this
/// function can only ever redirect a choice, never fail to produce one.
///
/// Precedence: a client's mapping beats the workspace mapping, and both are
/// ignored unless the classification is confident (or was set by a person).
pub fn choose_template(
    requested: &str,
    classification: Option<Classification>,
    client_id: Option<&str>,
    mappings: &[TypeTemplateMapping],
) -> TemplateChoice {
    let Some(classification) = classification else {
        return TemplateChoice {
            template_id: requested.to_string(),
            source: TemplateChoiceSource::NotClassified,
            meeting_type: None,
            confidence: None,
        };
    };

    if !classification.is_confident() {
        return TemplateChoice {
            template_id: requested.to_string(),
            source: TemplateChoiceSource::LowConfidence,
            meeting_type: Some(classification.meeting_type),
            confidence: Some(classification.confidence),
        };
    }

    let matching = |scope: Option<&str>| {
        mappings.iter().find(|mapping| {
            mapping.meeting_type == classification.meeting_type
                && mapping.client_id.as_deref() == scope
                && !mapping.template_id.trim().is_empty()
        })
    };

    if let Some(client_id) = client_id.filter(|id| !id.is_empty()) {
        if let Some(mapping) = matching(Some(client_id)) {
            return TemplateChoice {
                template_id: mapping.template_id.clone(),
                source: TemplateChoiceSource::ClientMapping,
                meeting_type: Some(classification.meeting_type),
                confidence: Some(classification.confidence),
            };
        }
    }

    if let Some(mapping) = matching(None) {
        return TemplateChoice {
            template_id: mapping.template_id.clone(),
            source: TemplateChoiceSource::WorkspaceMapping,
            meeting_type: Some(classification.meeting_type),
            confidence: Some(classification.confidence),
        };
    }

    TemplateChoice {
        template_id: requested.to_string(),
        source: TemplateChoiceSource::Requested,
        meeting_type: Some(classification.meeting_type),
        confidence: Some(classification.confidence),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping(kind: MeetingType, client: Option<&str>, template: &str) -> TypeTemplateMapping {
        TypeTemplateMapping {
            meeting_type: kind,
            client_id: client.map(str::to_string),
            template_id: template.to_string(),
        }
    }

    fn model(kind: MeetingType, confidence: f64) -> Classification {
        Classification {
            meeting_type: kind,
            confidence,
            source: TypeSource::Model,
        }
    }

    // ---- the type vocabulary ---------------------------------------------

    #[test]
    fn stored_forms_round_trip() {
        for kind in MeetingType::ALL {
            assert_eq!(MeetingType::parse(kind.as_str()), Some(*kind));
            assert!(!kind.label().is_empty());
            assert!(!kind.description().is_empty());
        }
    }

    #[test]
    fn the_synonyms_a_local_model_reaches_for_are_accepted() {
        assert_eq!(MeetingType::parse("check-in"), Some(MeetingType::Status));
        assert_eq!(MeetingType::parse("standup"), Some(MeetingType::Status));
        assert_eq!(MeetingType::parse("1:1"), Some(MeetingType::OneOnOne));
        assert_eq!(MeetingType::parse("one-to-one"), Some(MeetingType::OneOnOne));
        assert_eq!(MeetingType::parse("Troubleshooting"), Some(MeetingType::Incident));
        assert_eq!(MeetingType::parse("QBR"), Some(MeetingType::Review));
        assert_eq!(MeetingType::parse("retro"), Some(MeetingType::Review));
        assert_eq!(MeetingType::parse("kick-off"), Some(MeetingType::Discovery));
        assert_eq!(MeetingType::parse("renewal"), Some(MeetingType::Sales));
    }

    #[test]
    fn a_type_buried_in_a_longer_answer_is_still_found() {
        assert_eq!(
            MeetingType::parse("Type: incident / troubleshooting"),
            Some(MeetingType::Incident)
        );
        assert_eq!(
            MeetingType::parse("This looks like a status check-in"),
            Some(MeetingType::Status)
        );
    }

    #[test]
    fn an_unrecognisable_label_is_none_not_other() {
        // "Other" is a real answer; "I could not tell" is a different one.
        assert_eq!(MeetingType::parse(""), None);
        assert_eq!(MeetingType::parse("banana"), None);
        assert_eq!(MeetingType::parse("!!!"), None);
    }

    // ---- reply parsing ---------------------------------------------------

    #[test]
    fn a_clean_json_reply_parses() {
        let parsed = parse_reply(r#"{"type":"incident","confidence":0.82}"#).unwrap();
        assert_eq!(parsed.meeting_type, MeetingType::Incident);
        assert!((parsed.confidence - 0.82).abs() < 1e-9);
        assert_eq!(parsed.source, TypeSource::Model);
    }

    #[test]
    fn a_fenced_json_reply_parses() {
        let parsed = parse_reply("```json\n{\"type\": \"sales\", \"confidence\": 0.9}\n```").unwrap();
        assert_eq!(parsed.meeting_type, MeetingType::Sales);
        assert!((parsed.confidence - 0.9).abs() < 1e-9);
    }

    #[test]
    fn json_with_prose_around_it_parses() {
        let parsed =
            parse_reply("Here is my answer:\n{\"label\":\"planning\",\"score\":0.71}\nHope that helps.")
                .unwrap();
        assert_eq!(parsed.meeting_type, MeetingType::Planning);
        assert!((parsed.confidence - 0.71).abs() < 1e-9);
    }

    #[test]
    fn json_with_alternate_key_names_parses() {
        assert_eq!(
            parse_reply(r#"{"meeting_type":"review","certainty":"85%"}"#)
                .unwrap()
                .meeting_type,
            MeetingType::Review
        );
        assert_eq!(
            parse_reply(r#"{"category":"discovery"}"#).unwrap().meeting_type,
            MeetingType::Discovery
        );
    }

    #[test]
    fn a_bare_label_reply_parses_with_a_middling_confidence() {
        let parsed = parse_reply("incident").unwrap();
        assert_eq!(parsed.meeting_type, MeetingType::Incident);
        assert!((parsed.confidence - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_labelled_line_reply_parses() {
        let parsed = parse_reply("Type: one_on_one\nConfidence: 0.95").unwrap();
        assert_eq!(parsed.meeting_type, MeetingType::OneOnOne);
        assert!((parsed.confidence - 0.95).abs() < 1e-9);
    }

    #[test]
    fn a_prose_reply_still_yields_a_type() {
        let parsed = parse_reply("I would call this a status check-in, about 70% sure.").unwrap();
        assert_eq!(parsed.meeting_type, MeetingType::Status);
        assert!((parsed.confidence - 0.7).abs() < 1e-9);
    }

    #[test]
    fn a_reply_with_no_type_at_all_is_none() {
        assert!(parse_reply("").is_none());
        assert!(parse_reply("I am not sure what you mean.").is_none());
        assert!(parse_reply("{}").is_none());
        assert!(parse_reply("{\"confidence\": 0.9}").is_none());
    }

    #[test]
    fn a_percentage_confidence_becomes_a_fraction() {
        assert_eq!(clamp_confidence(85.0), 0.85);
        assert_eq!(clamp_confidence(0.85), 0.85);
        assert_eq!(clamp_confidence(1.0), 1.0);
        assert_eq!(clamp_confidence(-5.0), 0.0);
        assert_eq!(clamp_confidence(500.0), 1.0);
        assert_eq!(clamp_confidence(f64::NAN), 0.0);
    }

    // ---- template choice -------------------------------------------------

    #[test]
    fn with_no_classification_the_requested_template_stands() {
        let choice = choose_template("standard_meeting", None, Some("c1"), &[]);
        assert_eq!(choice.template_id, "standard_meeting");
        assert_eq!(choice.source, TemplateChoiceSource::NotClassified);
        assert_eq!(choice.meeting_type, None);
    }

    #[test]
    fn a_workspace_mapping_redirects_the_template() {
        let mappings = vec![mapping(MeetingType::Status, None, "daily_standup")];
        let choice = choose_template(
            "standard_meeting",
            Some(model(MeetingType::Status, 0.8)),
            None,
            &mappings,
        );
        assert_eq!(choice.template_id, "daily_standup");
        assert_eq!(choice.source, TemplateChoiceSource::WorkspaceMapping);
        assert_eq!(choice.meeting_type, Some(MeetingType::Status));
    }

    #[test]
    fn a_client_mapping_beats_the_workspace_mapping() {
        let mappings = vec![
            mapping(MeetingType::Status, None, "daily_standup"),
            mapping(MeetingType::Status, Some("c1"), "detailed_discussion"),
        ];
        let choice = choose_template(
            "standard_meeting",
            Some(model(MeetingType::Status, 0.8)),
            Some("c1"),
            &mappings,
        );
        assert_eq!(choice.template_id, "detailed_discussion");
        assert_eq!(choice.source, TemplateChoiceSource::ClientMapping);
    }

    #[test]
    fn another_clients_mapping_is_not_applied() {
        let mappings = vec![mapping(MeetingType::Status, Some("c2"), "detailed_discussion")];
        let choice = choose_template(
            "standard_meeting",
            Some(model(MeetingType::Status, 0.8)),
            Some("c1"),
            &mappings,
        );
        assert_eq!(choice.template_id, "standard_meeting");
        assert_eq!(choice.source, TemplateChoiceSource::Requested);
    }

    #[test]
    fn a_low_confidence_classification_does_not_redirect() {
        let mappings = vec![mapping(MeetingType::Status, None, "daily_standup")];
        let choice = choose_template(
            "standard_meeting",
            Some(model(MeetingType::Status, 0.4)),
            None,
            &mappings,
        );
        assert_eq!(choice.template_id, "standard_meeting");
        assert_eq!(choice.source, TemplateChoiceSource::LowConfidence);
        assert_eq!(choice.confidence, Some(0.4));
    }

    #[test]
    fn a_manual_classification_always_counts_as_confident() {
        let mappings = vec![mapping(MeetingType::Status, None, "daily_standup")];
        let manual = Classification {
            meeting_type: MeetingType::Status,
            confidence: 0.0,
            source: TypeSource::Manual,
        };
        let choice = choose_template("standard_meeting", Some(manual), None, &mappings);
        assert_eq!(choice.template_id, "daily_standup");
        assert_eq!(choice.source, TemplateChoiceSource::WorkspaceMapping);
    }

    #[test]
    fn a_mapping_for_a_different_type_is_not_applied() {
        let mappings = vec![mapping(MeetingType::Incident, None, "detailed_discussion")];
        let choice = choose_template(
            "standard_meeting",
            Some(model(MeetingType::Status, 0.9)),
            None,
            &mappings,
        );
        assert_eq!(choice.template_id, "standard_meeting");
        assert_eq!(choice.source, TemplateChoiceSource::Requested);
    }

    #[test]
    fn an_empty_mapped_template_is_ignored() {
        let mappings = vec![mapping(MeetingType::Status, None, "   ")];
        let choice = choose_template(
            "standard_meeting",
            Some(model(MeetingType::Status, 0.9)),
            None,
            &mappings,
        );
        assert_eq!(choice.template_id, "standard_meeting");
        assert_eq!(choice.source, TemplateChoiceSource::Requested);
    }

    #[test]
    fn the_confidence_threshold_boundary_is_inclusive() {
        let mappings = vec![mapping(MeetingType::Status, None, "daily_standup")];
        let at_threshold = choose_template(
            "standard_meeting",
            Some(model(MeetingType::Status, MIN_CONFIDENCE_FOR_TEMPLATE)),
            None,
            &mappings,
        );
        assert_eq!(at_threshold.source, TemplateChoiceSource::WorkspaceMapping);
    }

    #[test]
    fn the_choice_source_wire_form_matches_its_serde_name() {
        for source in [
            TemplateChoiceSource::ClientMapping,
            TemplateChoiceSource::WorkspaceMapping,
            TemplateChoiceSource::Requested,
            TemplateChoiceSource::LowConfidence,
            TemplateChoiceSource::NotClassified,
        ] {
            let serialized = serde_json::to_string(&source).unwrap();
            assert_eq!(serialized, format!("\"{}\"", source.as_str()));
        }
    }

    #[test]
    fn sources_round_trip_through_their_stored_form() {
        assert_eq!(TypeSource::parse("manual"), TypeSource::Manual);
        assert_eq!(TypeSource::parse("model"), TypeSource::Model);
        assert_eq!(TypeSource::parse("nonsense"), TypeSource::Model);
        assert_eq!(TypeSource::Manual.as_str(), "manual");
    }
}
