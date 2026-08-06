//! Tauri command surface for client-branded deliverables.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use crate::database::repositories::branding::BrandingRepository;
use crate::state::AppState;

use super::assets;
use super::rules::{self, Branding};

fn bounded(value: &str, max: usize) -> String {
    value.trim().chars().take(max).collect()
}

/// The branding row plus what the UI needs to render a preview of the logo
/// without reaching for the filesystem itself.
#[derive(Debug, Serialize)]
pub struct BrandingView {
    #[serde(flatten)]
    pub branding: Branding,
    /// True when a firm name is set, which is what switches branding on.
    pub is_configured: bool,
    /// The stored logo as a data URI, for the picker's preview. None when there is
    /// no logo or the stored copy has gone bad.
    pub logo_data_uri: Option<String>,
    /// The accent after normalization, and readable ink to sit on it.
    pub accent: String,
    pub accent_ink: String,
}

fn view(branding: Branding) -> BrandingView {
    let logo_data_uri = branding
        .logo_path
        .as_deref()
        .and_then(assets::logo_as_data_uri);
    let accent = branding.accent();
    let accent_ink = rules::contrasting_ink(&accent).to_string();
    BrandingView {
        is_configured: branding.is_configured(),
        logo_data_uri,
        accent,
        accent_ink,
        branding,
    }
}

#[tauri::command]
pub async fn branding_get(state: State<'_, AppState>) -> Result<BrandingView, String> {
    Ok(view(super::load(state.db_manager.pool()).await))
}

#[derive(Debug, Deserialize)]
pub struct BrandingInput {
    #[serde(default)]
    pub firm_name: String,
    #[serde(default)]
    pub footer_text: String,
    #[serde(default)]
    pub accent_hex: String,
    #[serde(default = "default_true")]
    pub include_logo: bool,
    #[serde(default = "default_true")]
    pub include_footer: bool,
}

fn default_true() -> bool {
    true
}

/// Saves the branding fields. The logo is managed separately by
/// `branding_pick_logo` / `branding_clear_logo`, so saving text never risks
/// losing the stored image.
#[tauri::command]
pub async fn branding_set(
    state: State<'_, AppState>,
    input: BrandingInput,
) -> Result<BrandingView, String> {
    let pool = state.db_manager.pool();
    let accent = rules::normalize_hex(&input.accent_hex).ok_or_else(|| {
        "That is not a colour. Use a hex value like #2d5f8b.".to_string()
    })?;
    let current = super::load(pool).await;

    BrandingRepository::save(
        pool,
        &bounded(&input.firm_name, 120),
        current.logo_path.as_deref(),
        &bounded(&input.footer_text, 300),
        &accent,
        input.include_logo,
        input.include_footer,
    )
    .await
    .map_err(|e| format!("Failed to save branding: {}", e))?;

    Ok(view(super::load(pool).await))
}

/// Opens a picker, validates the chosen image by its content, and copies it into
/// the app data directory. Only the copy's path is stored.
#[tauri::command]
pub async fn branding_pick_logo<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<BrandingView, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Image", &["png", "jpg", "jpeg", "gif", "webp", "svg"])
        .pick_file(move |file| {
            let _ = tx.send(file);
        });
    let picked = rx
        .await
        .map_err(|_| "File picker closed unexpectedly".to_string())?
        .ok_or_else(|| "cancelled".to_string())?
        .into_path()
        .map_err(|e| format!("Invalid file: {}", e))?;

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Could not find the app data directory: {}", e))?;

    let stored = assets::install_logo(&app_data_dir, &picked)?;

    let pool = state.db_manager.pool();
    BrandingRepository::set_logo_path(pool, Some(&stored.to_string_lossy()))
        .await
        .map_err(|e| format!("Failed to save the logo path: {}", e))?;

    Ok(view(super::load(pool).await))
}

#[tauri::command]
pub async fn branding_clear_logo<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<BrandingView, String> {
    if let Ok(app_data_dir) = app.path().app_data_dir() {
        assets::remove_logo(&app_data_dir);
    }
    let pool = state.db_manager.pool();
    BrandingRepository::set_logo_path(pool, None)
        .await
        .map_err(|e| format!("Failed to clear the logo: {}", e))?;
    Ok(view(super::load(pool).await))
}

#[derive(Debug, Serialize)]
pub struct BrandingPreviewResult {
    pub path: String,
}

/// Renders a sample deliverable with the current branding and opens it, so the
/// operator sees the real export path rather than a mock of it.
#[tauri::command]
pub async fn branding_preview<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<BrandingPreviewResult, String> {
    let branding = super::load(state.db_manager.pool()).await;
    let html = crate::export::html::build_meeting_html(
        "Sample deliverable — Q3 service review",
        "2026-08-06T10:00:00Z",
        Some(super::SAMPLE_SUMMARY),
        &[
            (
                "10:02:14".to_string(),
                "This is what a transcript line looks like in an exported deliverable."
                    .to_string(),
            ),
            (
                "10:02:41".to_string(),
                "The header, footer, and accent colour above and below come from your Deliverables settings."
                    .to_string(),
            ),
        ],
        Some(&branding),
    );

    let path = std::env::temp_dir().join("meetily-branding-preview.html");
    std::fs::write(&path, html)
        .map_err(|e| format!("Could not write the preview to {}: {}", path.display(), e))?;
    crate::export::open_path_with_default_app(&path);

    Ok(BrandingPreviewResult {
        path: path.to_string_lossy().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_fields_are_trimmed_and_bounded() {
        assert_eq!(bounded("  Vortex MSP  ", 120), "Vortex MSP");
        assert_eq!(bounded(&"x".repeat(400), 300).chars().count(), 300);
    }

    #[test]
    fn the_view_normalizes_the_accent_and_picks_readable_ink() {
        let rendered = view(Branding {
            firm_name: "Firm".to_string(),
            accent_hex: "#FFF".to_string(),
            ..Branding::default()
        });
        assert_eq!(rendered.accent, "#ffffff");
        assert_eq!(rendered.accent_ink, "#1a1c21");
        assert!(rendered.is_configured);
    }

    #[test]
    fn the_view_reports_unconfigured_branding_as_such() {
        let rendered = view(Branding::default());
        assert!(!rendered.is_configured);
        assert_eq!(rendered.logo_data_uri, None);
        assert_eq!(rendered.accent, rules::DEFAULT_ACCENT);
    }
}
