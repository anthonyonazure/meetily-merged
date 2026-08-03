//! Export meetings to Markdown, DOCX, or print-ready HTML (the PDF path).
//!
//! Transcripts and summaries live in SQLite, which is right for the app's access
//! patterns (per-meeting queries, search, in-place summary updates) but useless if you
//! want your notes in a folder, in Git, or synced to iCloud. Exporting sidesteps that:
//! the database stays the source of truth, and the files are a plain-text copy you own.
//!
//! Formats:
//! - Markdown: the original export, unchanged.
//! - DOCX: native Word documents via the pure-Rust `docx-rs` crate.
//! - PDF: print-styled standalone HTML opened in the browser for
//!   print-to-PDF. See `html.rs` for why this beat `genpdf`/`printpdf`
//!   (Unicode fidelity; no bundled fonts).

pub mod docx;
pub mod html;
pub mod markdown_ast;

use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use crate::database::repositories::{meeting::MeetingsRepository, summary::SummaryProcessesRepository};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct ExportResult {
    /// Folder the files were written to.
    pub folder: String,
    pub exported: usize,
    /// Meetings that produced no file (no transcript and no summary).
    pub skipped: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportFormat {
    Markdown,
    Docx,
    /// Print-styled HTML, opened for browser print-to-PDF.
    HtmlPrint,
}

impl ExportFormat {
    fn parse(format: &str) -> Result<Self, String> {
        match format.to_lowercase().as_str() {
            "markdown" | "md" => Ok(Self::Markdown),
            "docx" => Ok(Self::Docx),
            "pdf" | "html" => Ok(Self::HtmlPrint),
            other => Err(format!("Unsupported export format: {}", other)),
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Docx => "docx",
            Self::HtmlPrint => "html",
        }
    }
}

const HEADING_DATE: &str = "Date";
const HEADING_SUMMARY: &str = "Summary";
const HEADING_TRANSCRIPT: &str = "Transcript";
const NO_SUMMARY_PLACEHOLDER: &str = "_No summary generated._";

/// Make a title safe to use as a filename, without mangling non-ASCII characters.
///
/// Only the characters that actually break filesystems are replaced — stripping
/// everything non-ASCII would reduce a non-English meeting title to an empty string.
fn safe_filename(title: &str) -> String {
    const FORBIDDEN: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

    let cleaned: String = title
        .chars()
        .map(|c| {
            if FORBIDDEN.contains(&c) || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();

    let cleaned = cleaned.trim().trim_matches('.').trim();

    if cleaned.is_empty() {
        "untitled".to_string()
    } else {
        // Leave room for the date prefix and extension within the usual 255-byte limit.
        cleaned.chars().take(120).collect()
    }
}

/// Pull the summary markdown out of the stored process row.
///
/// `summary_processes.result` is a JSON blob whose `markdown` field holds the rendered
/// summary (see summary/service.rs).
fn summary_markdown(result_json: Option<&str>) -> Option<String> {
    let raw = result_json?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let markdown = value.get("markdown")?.as_str()?.trim();
    (!markdown.is_empty()).then(|| markdown.to_string())
}

fn build_markdown(
    title: &str,
    created_at: &str,
    summary: Option<&str>,
    transcripts: &[(String, String)],
) -> String {
    let mut out = String::new();

    out.push_str(&format!("# {}\n\n", title));
    out.push_str(&format!("- {}: {}\n\n", HEADING_DATE, created_at));

    out.push_str(&format!("## {}\n\n", HEADING_SUMMARY));
    match summary {
        Some(md) => {
            out.push_str(md);
            out.push_str("\n\n");
        }
        None => out.push_str(&format!("{}\n\n", NO_SUMMARY_PLACEHOLDER)),
    }

    out.push_str(&format!("## {}\n\n", HEADING_TRANSCRIPT));
    for (timestamp, text) in transcripts {
        if text.trim().is_empty() {
            continue;
        }
        out.push_str(&format!("**[{}]** {}\n\n", timestamp, text.trim()));
    }

    out
}

/// Opens a file or folder with the platform's default handler (Finder /
/// Explorer / browser). Used after an HTML-print export so the user lands one
/// keystroke away from "Save as PDF". Failures are logged, never fatal.
fn open_with_default_app(path: &Path) {
    let path_str = path.to_string_lossy().to_string();
    let result = if cfg!(target_os = "windows") {
        std::process::Command::new("explorer").arg(&path_str).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&path_str).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(&path_str).spawn()
    };
    if let Err(e) = result {
        log::warn!("Failed to open {} with default app: {}", path_str, e);
    }
}

/// Export meetings as Markdown. Kept as its own command for backward
/// compatibility; delegates to the format-aware export.
#[tauri::command]
pub async fn export_meetings_markdown<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    meeting_ids: Option<Vec<String>>,
) -> Result<ExportResult, String> {
    run_export(app, state, ExportFormat::Markdown, meeting_ids).await
}

/// Export meetings in the requested format: "markdown", "docx", or "pdf"
/// (print-styled HTML that the browser saves as PDF). `meeting_ids = None`
/// exports everything.
#[tauri::command]
pub async fn export_meetings<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    format: String,
    meeting_ids: Option<Vec<String>>,
) -> Result<ExportResult, String> {
    let format = ExportFormat::parse(&format)?;
    run_export(app, state, format, meeting_ids).await
}

/// Shared export flow. Prompts for a destination folder; a cancelled picker is
/// reported as the literal error string "cancelled" so the UI can stay quiet
/// about it.
async fn run_export<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    format: ExportFormat,
    meeting_ids: Option<Vec<String>>,
) -> Result<ExportResult, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |folder| {
        let _ = tx.send(folder);
    });

    let folder = rx
        .await
        .map_err(|_| "Folder picker closed unexpectedly".to_string())?
        .ok_or_else(|| "cancelled".to_string())?;

    let folder = folder
        .into_path()
        .map_err(|e| format!("Invalid destination folder: {}", e))?;

    let pool = state.db_manager.pool();

    // Resolve which meetings to export.
    let ids: Vec<String> = match meeting_ids {
        Some(ids) => ids,
        None => MeetingsRepository::get_meetings(pool)
            .await
            .map_err(|e| format!("Failed to list meetings: {}", e))?
            .into_iter()
            .map(|m| m.id)
            .collect(),
    };

    let mut exported = 0usize;
    let mut skipped = 0usize;
    let mut written_paths: Vec<std::path::PathBuf> = Vec::new();

    for id in ids {
        let details = match MeetingsRepository::get_meeting(pool, &id).await {
            Ok(Some(d)) => d,
            Ok(None) => {
                skipped += 1;
                continue;
            }
            Err(e) => {
                log::warn!("Skipping meeting {} — failed to load: {}", id, e);
                skipped += 1;
                continue;
            }
        };

        let summary = SummaryProcessesRepository::get_summary_data(pool, &id)
            .await
            .ok()
            .flatten()
            .and_then(|p| summary_markdown(p.result.as_deref()));

        let transcripts: Vec<(String, String)> = details
            .transcripts
            .iter()
            .map(|t| (t.timestamp.clone(), t.text.clone()))
            .collect();

        // A meeting with neither a transcript nor a summary has nothing to write.
        if transcripts.is_empty() && summary.is_none() {
            skipped += 1;
            continue;
        }

        let created_date = details.created_at.split('T').next().unwrap_or("").to_string();
        let filename = format!(
            "{}_{}.{}",
            if created_date.is_empty() {
                "undated".to_string()
            } else {
                created_date
            },
            safe_filename(&details.title),
            format.extension()
        );
        let path = folder.join(&filename);

        let write_result: Result<(), String> = match format {
            ExportFormat::Markdown => {
                let markdown = build_markdown(
                    &details.title,
                    &details.created_at,
                    summary.as_deref(),
                    &transcripts,
                );
                std::fs::write(&path, markdown).map_err(|e| e.to_string())
            }
            ExportFormat::Docx => docx::write_meeting_docx(
                &path,
                &details.title,
                &details.created_at,
                summary.as_deref(),
                &transcripts,
            ),
            ExportFormat::HtmlPrint => {
                let html = html::build_meeting_html(
                    &details.title,
                    &details.created_at,
                    summary.as_deref(),
                    &transcripts,
                );
                std::fs::write(&path, html).map_err(|e| e.to_string())
            }
        };

        match write_result {
            Ok(()) => {
                exported += 1;
                written_paths.push(path);
            }
            Err(e) => {
                log::error!("Failed to write {}: {}", path.display(), e);
                skipped += 1;
            }
        }
    }

    // For the PDF path, put the user one step from "Save as PDF": open the
    // single exported page directly, or the folder when there are several.
    if format == ExportFormat::HtmlPrint && exported > 0 {
        if exported == 1 {
            open_with_default_app(&written_paths[0]);
        } else {
            open_with_default_app(&folder);
        }
    }

    log::info!(
        "Exported {} meeting(s) to {} as {:?} ({} skipped)",
        exported,
        folder.display(),
        format,
        skipped
    );

    Ok(ExportResult {
        folder: folder.to_string_lossy().to_string(),
        exported,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_ascii_titles_survive_filename_sanitisation() {
        // Stripping non-ASCII would leave a non-English meeting with an empty filename.
        assert_eq!(safe_filename("产品评审会议"), "产品评审会议");
        assert_eq!(safe_filename("Q3 规划/复盘"), "Q3 规划-复盘");
    }

    #[test]
    fn path_breaking_characters_are_replaced() {
        assert_eq!(safe_filename("a/b\\c:d*e?f\"g<h>i|j"), "a-b-c-d-e-f-g-h-i-j");
        assert_eq!(safe_filename("   "), "untitled");
        assert_eq!(safe_filename(""), "untitled");
    }

    #[test]
    fn summary_markdown_is_read_from_the_result_json() {
        assert_eq!(
            summary_markdown(Some("{\"markdown\":\"# Notes\\nBody\"}")).as_deref(),
            Some("# Notes\nBody")
        );
        assert_eq!(summary_markdown(Some(r#"{"markdown":"   "}"#)), None);
        assert_eq!(summary_markdown(Some("not json")), None);
        assert_eq!(summary_markdown(None), None);
    }

    #[test]
    fn markdown_has_summary_and_transcript_sections() {
        let md = build_markdown(
            "Weekly sync",
            "2026-07-12T10:00:00Z",
            Some("A few key points"),
            &[("00:01".to_string(), "Hello everyone".to_string())],
        );
        assert!(md.starts_with("# Weekly sync\n"));
        assert!(md.contains("## Summary"));
        assert!(md.contains("A few key points"));
        assert!(md.contains("## Transcript"));
        assert!(md.contains("**[00:01]** Hello everyone"));
    }

    #[test]
    fn a_meeting_without_a_summary_says_so_rather_than_leaving_a_hole() {
        let md = build_markdown("Standup", "2026-07-12T10:00:00Z", None, &[]);
        assert!(md.contains("_No summary generated._"));
    }

    #[test]
    fn export_formats_parse_and_map_to_extensions() {
        assert_eq!(ExportFormat::parse("markdown").unwrap(), ExportFormat::Markdown);
        assert_eq!(ExportFormat::parse("DOCX").unwrap(), ExportFormat::Docx);
        assert_eq!(ExportFormat::parse("pdf").unwrap(), ExportFormat::HtmlPrint);
        assert_eq!(ExportFormat::parse("html").unwrap(), ExportFormat::HtmlPrint);
        assert!(ExportFormat::parse("odt").is_err());
        assert_eq!(ExportFormat::Docx.extension(), "docx");
        assert_eq!(ExportFormat::HtmlPrint.extension(), "html");
    }
}
