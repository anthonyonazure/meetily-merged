//! Tauri command surface for meeting-type detection and template mapping.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::database::repositories::client::MeetingClientsRepository;
use crate::database::repositories::meeting_type::{
    MeetingTypeTemplatesRepository, MeetingTypesRepository,
};
use crate::state::AppState;

use super::rules::{
    self, Classification, MeetingType, TemplateChoice, TypeSource, TypeTemplateMapping,
    WORKSPACE_SCOPE,
};

/// One entry in the vocabulary the settings screen renders.
#[derive(Debug, Serialize)]
pub struct MeetingTypeOption {
    pub value: String,
    pub label: String,
    pub description: String,
}

/// A meeting's classification, plus the template it would drive.
#[derive(Debug, Serialize)]
pub struct MeetingTypeView {
    pub meeting_id: String,
    /// None when the meeting has not been classified.
    pub meeting_type: Option<MeetingType>,
    pub label: Option<String>,
    pub confidence: Option<f64>,
    pub source: Option<TypeSource>,
    /// True when the classification is trusted enough to pick a template.
    pub is_confident: bool,
    /// The template a summary would use, and why.
    pub template_choice: TemplateChoice,
    pub client_id: Option<String>,
    /// The whole vocabulary, so a correction dropdown needs no second call.
    pub options: Vec<MeetingTypeOption>,
}

fn options() -> Vec<MeetingTypeOption> {
    MeetingType::ALL
        .iter()
        .map(|kind| MeetingTypeOption {
            value: kind.as_str().to_string(),
            label: kind.label().to_string(),
            description: kind.description().to_string(),
        })
        .collect()
}

fn to_mappings(rows: Vec<crate::database::models::MeetingTypeTemplateRow>) -> Vec<TypeTemplateMapping> {
    rows.into_iter()
        .filter_map(|row| {
            MeetingType::parse(&row.meeting_type).map(|meeting_type| TypeTemplateMapping {
                meeting_type,
                client_id: (row.client_id != WORKSPACE_SCOPE).then_some(row.client_id),
                template_id: row.template_id,
            })
        })
        .collect()
}

/// The template that should be used for a meeting, given its classification and
/// the mappings in force. `requested` is the caller's own choice and is always the
/// fallback.
pub async fn resolve_template(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    requested: &str,
) -> TemplateChoice {
    let classification = super::classify::stored_classification(pool, meeting_id).await;
    let client_id = MeetingClientsRepository::client_for_meeting(pool, meeting_id)
        .await
        .ok()
        .flatten()
        .map(|client| client.id);
    let mappings = MeetingTypeTemplatesRepository::for_scope(pool, client_id.as_deref())
        .await
        .map(to_mappings)
        .unwrap_or_default();

    rules::choose_template(requested, classification, client_id.as_deref(), &mappings)
}

async fn build_view(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    requested_template: &str,
) -> Result<MeetingTypeView, String> {
    let classification = super::classify::stored_classification(pool, meeting_id).await;
    let client_id = MeetingClientsRepository::client_for_meeting(pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to read the meeting's client: {}", e))?
        .map(|client| client.id);
    let mappings = MeetingTypeTemplatesRepository::for_scope(pool, client_id.as_deref())
        .await
        .map(to_mappings)
        .unwrap_or_default();
    let template_choice = rules::choose_template(
        requested_template,
        classification,
        client_id.as_deref(),
        &mappings,
    );

    Ok(MeetingTypeView {
        meeting_id: meeting_id.to_string(),
        meeting_type: classification.map(|c| c.meeting_type),
        label: classification.map(|c| c.meeting_type.label().to_string()),
        confidence: classification.map(|c| c.confidence),
        source: classification.map(|c| c.source),
        is_confident: classification.map(|c| c.is_confident()).unwrap_or(false),
        template_choice,
        client_id,
        options: options(),
    })
}

/// The classification for a meeting, and the template it drives.
#[tauri::command]
pub async fn meeting_type_get(
    state: State<'_, AppState>,
    meeting_id: String,
    requested_template: Option<String>,
) -> Result<MeetingTypeView, String> {
    let requested = requested_template.unwrap_or_else(|| "standard_meeting".to_string());
    build_view(state.db_manager.pool(), &meeting_id, &requested).await
}

/// A person's correction. Stored with source `manual`, which no later model run
/// overwrites, and with full confidence so it drives the template mapping.
#[tauri::command]
pub async fn meeting_type_set(
    state: State<'_, AppState>,
    meeting_id: String,
    meeting_type: String,
    requested_template: Option<String>,
) -> Result<MeetingTypeView, String> {
    let pool = state.db_manager.pool();
    let parsed = MeetingType::parse(&meeting_type)
        .ok_or_else(|| format!("\"{}\" is not a meeting type", meeting_type))?;

    MeetingTypesRepository::set(pool, &meeting_id, parsed.as_str(), 1.0, TypeSource::Manual.as_str())
        .await
        .map_err(|e| format!("Failed to save the meeting type: {}", e))?;

    let requested = requested_template.unwrap_or_else(|| "standard_meeting".to_string());
    build_view(pool, &meeting_id, &requested).await
}

/// Removes a classification, putting the meeting back to unclassified.
#[tauri::command]
pub async fn meeting_type_clear(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<MeetingTypeView, String> {
    let pool = state.db_manager.pool();
    MeetingTypesRepository::clear(pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to clear the meeting type: {}", e))?;
    build_view(pool, &meeting_id, "standard_meeting").await
}

/// The whole mapping table, workspace and per-client, plus the vocabulary.
#[derive(Debug, Serialize)]
pub struct MeetingTypeMappings {
    pub mappings: Vec<TypeTemplateMapping>,
    pub options: Vec<MeetingTypeOption>,
    pub min_confidence: f64,
}

#[tauri::command]
pub async fn meeting_type_mappings_get(
    state: State<'_, AppState>,
) -> Result<MeetingTypeMappings, String> {
    let rows = MeetingTypeTemplatesRepository::list(state.db_manager.pool())
        .await
        .map_err(|e| format!("Failed to read the template mappings: {}", e))?;
    Ok(MeetingTypeMappings {
        mappings: to_mappings(rows),
        options: options(),
        min_confidence: rules::MIN_CONFIDENCE_FOR_TEMPLATE,
    })
}

#[derive(Debug, Deserialize)]
pub struct MappingInput {
    pub meeting_type: String,
    /// None or empty sets the workspace mapping.
    #[serde(default)]
    pub client_id: Option<String>,
    /// An empty template id removes the mapping.
    #[serde(default)]
    pub template_id: String,
}

/// Sets or clears one mapping. The template id is validated against the templates
/// that actually exist, so a mapping cannot point at a template that was deleted.
#[tauri::command]
pub async fn meeting_type_mappings_set(
    state: State<'_, AppState>,
    input: MappingInput,
) -> Result<MeetingTypeMappings, String> {
    let pool = state.db_manager.pool();
    let parsed = MeetingType::parse(&input.meeting_type)
        .ok_or_else(|| format!("\"{}\" is not a meeting type", input.meeting_type))?;
    let template_id = input.template_id.trim();

    if !template_id.is_empty() {
        let known = crate::summary::templates::list_template_ids();
        if !known.iter().any(|id| id == template_id) {
            return Err(format!(
                "There is no template called \"{}\". Pick one from the template list.",
                template_id
            ));
        }
    }

    let scope = input
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(WORKSPACE_SCOPE);

    MeetingTypeTemplatesRepository::set(pool, parsed.as_str(), scope, template_id)
        .await
        .map_err(|e| format!("Failed to save the template mapping: {}", e))?;

    meeting_type_mappings_get(state).await
}

/// Classifies a meeting now, using the given model. Used by the "detect again"
/// action; the automatic path runs the same function from the summary flow.
#[tauri::command]
pub async fn meeting_type_detect<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    meeting_id: String,
    model: String,
    model_name: String,
    requested_template: Option<String>,
) -> Result<MeetingTypeView, String> {
    use tauri::Manager;
    let pool = state.db_manager.pool().clone();
    let app_data_dir = app.path().app_data_dir().ok();

    let classified: Option<Classification> =
        super::classify::classify_meeting(&pool, &meeting_id, &model, &model_name, app_data_dir)
            .await;
    if classified.is_none() {
        log::info!(
            "[MeetingType] detection produced no answer for {}; leaving it unclassified",
            meeting_id
        );
    }

    let requested = requested_template.unwrap_or_else(|| "standard_meeting".to_string());
    build_view(&pool, &meeting_id, &requested).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::models::MeetingTypeTemplateRow;

    #[test]
    fn the_vocabulary_is_offered_in_full_with_labels_and_descriptions() {
        let all = options();
        assert_eq!(all.len(), MeetingType::ALL.len());
        for option in &all {
            assert!(!option.label.is_empty());
            assert!(!option.description.is_empty());
            assert!(MeetingType::parse(&option.value).is_some());
        }
    }

    #[test]
    fn the_workspace_sentinel_becomes_a_null_client_id() {
        let rows = vec![
            MeetingTypeTemplateRow {
                meeting_type: "status".to_string(),
                client_id: WORKSPACE_SCOPE.to_string(),
                template_id: "daily_standup".to_string(),
            },
            MeetingTypeTemplateRow {
                meeting_type: "review".to_string(),
                client_id: "c1".to_string(),
                template_id: "detailed_discussion".to_string(),
            },
        ];
        let mappings = to_mappings(rows);
        assert_eq!(mappings[0].client_id, None);
        assert_eq!(mappings[1].client_id.as_deref(), Some("c1"));
    }

    #[test]
    fn an_unreadable_stored_type_is_dropped_rather_than_crashing_the_list() {
        let rows = vec![MeetingTypeTemplateRow {
            meeting_type: "nonsense-from-an-older-build".to_string(),
            client_id: WORKSPACE_SCOPE.to_string(),
            template_id: "daily_standup".to_string(),
        }];
        assert!(to_mappings(rows).is_empty());
    }
}
