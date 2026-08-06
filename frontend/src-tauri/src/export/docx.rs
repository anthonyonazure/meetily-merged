//! Markdown-subset → DOCX rendering via the `docx-rs` crate (pure Rust,
//! crates.io). Headings, paragraphs, bullet/numbered lists, and bold runs map
//! to native Word constructs; anything else arrives as plain paragraphs
//! because the parser already degraded it.

use crate::branding::Branding;
use crate::export::markdown_ast::{parse_markdown, Block, Inline};
use docx_rs::{
    AbstractNumbering, Docx, Footer, Header, IndentLevel, Level, LevelJc, LevelText, NumberFormat,
    Numbering, NumberingId, Paragraph, Run, SpecialIndentType, Start,
};
use std::path::Path;

// Font sizes are in half-points (Word's unit): 22 = 11pt body text.
const SIZE_BODY: usize = 22;
const SIZE_TITLE: usize = 40;
const SIZE_H1: usize = 36;
const SIZE_H2: usize = 30;
const SIZE_H3: usize = 26;
const SIZE_FOOTER: usize = 18;

/// Abstract numbering ids: 1 = bullets, 2 = decimal.
const ABSTRACT_BULLET: usize = 1;
const ABSTRACT_DECIMAL: usize = 2;
/// The single concrete bullet list instance (bullets have no visible counter,
/// so sharing one instance across the document is fine).
const NUMBERING_BULLET: usize = 1;

fn runs_from_inlines(inlines: &[Inline], size: usize, force_bold: bool) -> Vec<Run> {
    inlines
        .iter()
        .map(|inline| match inline {
            Inline::Text(text) => {
                let run = Run::new().add_text(text.as_str()).size(size);
                if force_bold {
                    run.bold()
                } else {
                    run
                }
            }
            Inline::Bold(text) => Run::new().add_text(text.as_str()).size(size).bold(),
        })
        .collect()
}

fn paragraph_with_runs(runs: Vec<Run>) -> Paragraph {
    let mut paragraph = Paragraph::new();
    for run in runs {
        paragraph = paragraph.add_run(run);
    }
    paragraph
}

/// A heading, tinted with the firm's accent when branding is configured.
fn heading_paragraph(level: u8, inlines: &[Inline], accent: Option<&str>) -> Paragraph {
    let size = match level {
        1 => SIZE_H1,
        2 => SIZE_H2,
        _ => SIZE_H3,
    };
    let runs = runs_from_inlines(inlines, size, true)
        .into_iter()
        .map(|run| match accent {
            Some(color) => run.color(color),
            None => run,
        })
        .collect();
    paragraph_with_runs(runs)
}

/// Word's running header: the firm name in the accent colour.
///
/// NOTE: no logo. This crate is deliberately built with
/// `docx-rs = { default-features = false }` so the `image` feature and its whole
/// decoder stack stay out of the binary. Embedding a picture in a DOCX needs that
/// feature, so the Word deliverable is branded by name, colour, and footer only.
/// The print-HTML (PDF) deliverable does carry the logo.
fn branded_header(branding: &Branding) -> Header {
    let accent = crate::branding::rules::docx_color(&branding.accent_hex);
    Header::new().add_paragraph(paragraph_with_runs(vec![Run::new()
        .add_text(branding.firm_name.trim())
        .size(SIZE_BODY)
        .bold()
        .color(accent)]))
}

/// Word's running footer: the firm's footer line, on every page.
fn branded_footer(text: &str) -> Footer {
    Footer::new().add_paragraph(paragraph_with_runs(vec![Run::new()
        .add_text(text)
        .size(SIZE_FOOTER)
        .italic()]))
}

fn list_level(level: usize, format: &str, text: &str) -> Level {
    Level::new(
        level,
        Start::new(1),
        NumberFormat::new(format),
        LevelText::new(text),
        LevelJc::new("left"),
    )
    .indent(
        Some(720 * (level as i32 + 1)),
        Some(SpecialIndentType::Hanging(320)),
        None,
        None,
    )
}

/// Builds a DOCX for one meeting and writes it to `path`.
///
/// `summary_markdown` renders through the markdown walker; the transcript is
/// timestamped paragraphs. `branding` adds a running header with the firm name, a
/// running footer, and the accent colour on headings; None leaves the document
/// exactly as it was before branding existed.
pub fn write_meeting_docx(
    path: &Path,
    title: &str,
    created_at: &str,
    summary_markdown: Option<&str>,
    transcripts: &[(String, String)],
    branding: Option<&Branding>,
) -> Result<(), String> {
    let branding = branding.filter(|b| b.is_configured());
    let accent = branding.map(|b| crate::branding::rules::docx_color(&b.accent_hex));
    let accent = accent.as_deref();

    let bullet_abstract = AbstractNumbering::new(ABSTRACT_BULLET)
        .add_level(list_level(0, "bullet", "•"))
        .add_level(list_level(1, "bullet", "◦"))
        .add_level(list_level(2, "bullet", "▪"));
    let decimal_abstract = AbstractNumbering::new(ABSTRACT_DECIMAL)
        .add_level(list_level(0, "decimal", "%1."))
        .add_level(list_level(1, "decimal", "%2."))
        .add_level(list_level(2, "decimal", "%3."));

    let mut docx = Docx::new()
        .add_abstract_numbering(bullet_abstract)
        .add_abstract_numbering(decimal_abstract)
        .add_numbering(Numbering::new(NUMBERING_BULLET, ABSTRACT_BULLET));

    // The firm's letterhead and footer, as real running header/footer sections so
    // they repeat on every page rather than only on the first.
    if let Some(branding) = branding {
        docx = docx.header(branded_header(branding));
        if let Some(footer_text) = branding.footer_to_print() {
            docx = docx.footer(branded_footer(footer_text));
        }
    }

    // Document header: title and date.
    docx = docx.add_paragraph(paragraph_with_runs(vec![{
        let run = Run::new().add_text(title).size(SIZE_TITLE).bold();
        match accent {
            Some(color) => run.color(color),
            None => run,
        }
    }]));
    docx = docx.add_paragraph(paragraph_with_runs(vec![Run::new()
        .add_text(format!("Date: {}", created_at))
        .size(SIZE_BODY)]));

    // Summary section.
    docx = docx.add_paragraph(heading_paragraph(
        1,
        &[Inline::Text("Summary".to_string())],
        accent,
    ));
    // Each contiguous numbered-list group gets its own concrete numbering
    // instance so counters restart at 1 instead of continuing document-wide.
    let mut next_numbering_id = NUMBERING_BULLET + 1;
    let mut open_numbered_group: Option<usize> = None;
    match summary_markdown {
        Some(markdown) => {
            for block in parse_markdown(markdown) {
                if !matches!(block, Block::Numbered { .. }) {
                    open_numbered_group = None;
                }
                match block {
                    Block::Heading { level, inlines } => {
                        docx = docx.add_paragraph(heading_paragraph(level, &inlines, accent));
                    }
                    Block::Paragraph(inlines) => {
                        docx = docx.add_paragraph(paragraph_with_runs(runs_from_inlines(
                            &inlines, SIZE_BODY, false,
                        )));
                    }
                    Block::Bullet { depth, inlines } => {
                        docx = docx.add_paragraph(
                            paragraph_with_runs(runs_from_inlines(&inlines, SIZE_BODY, false))
                                .numbering(
                                    NumberingId::new(NUMBERING_BULLET),
                                    IndentLevel::new(depth as usize),
                                ),
                        );
                    }
                    Block::Numbered { depth, inlines } => {
                        let numbering_id = match open_numbered_group {
                            Some(id) => id,
                            None => {
                                let id = next_numbering_id;
                                next_numbering_id += 1;
                                docx = docx.add_numbering(Numbering::new(id, ABSTRACT_DECIMAL));
                                open_numbered_group = Some(id);
                                id
                            }
                        };
                        docx = docx.add_paragraph(
                            paragraph_with_runs(runs_from_inlines(&inlines, SIZE_BODY, false))
                                .numbering(
                                    NumberingId::new(numbering_id),
                                    IndentLevel::new(depth as usize),
                                ),
                        );
                    }
                }
            }
        }
        None => {
            docx = docx.add_paragraph(paragraph_with_runs(vec![Run::new()
                .add_text("No summary generated.")
                .size(SIZE_BODY)
                .italic()]));
        }
    }

    // Transcript section.
    docx = docx.add_paragraph(heading_paragraph(
        1,
        &[Inline::Text("Transcript".to_string())],
        accent,
    ));
    for (timestamp, text) in transcripts {
        if text.trim().is_empty() {
            continue;
        }
        docx = docx.add_paragraph(paragraph_with_runs(vec![
            Run::new()
                .add_text(format!("[{}] ", timestamp))
                .size(SIZE_BODY)
                .bold(),
            Run::new().add_text(text.trim()).size(SIZE_BODY),
        ]));
    }

    let file = std::fs::File::create(path)
        .map_err(|e| format!("Failed to create {}: {}", path.display(), e))?;
    docx.build()
        .pack(file)
        .map_err(|e| format!("Failed to write DOCX {}: {}", path.display(), e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branding(firm: &str) -> Branding {
        Branding {
            firm_name: firm.to_string(),
            logo_path: Some("/tmp/logo.png".to_string()),
            footer_text: "Confidential".to_string(),
            accent_hex: "#2D5F8B".to_string(),
            include_logo: true,
            include_footer: true,
        }
    }

    /// docx-rs elements derive Serialize, so the built structure can be inspected
    /// without unzipping a file.
    fn as_json<T: serde::Serialize>(value: &T) -> String {
        serde_json::to_string(value).expect("serializable")
    }

    #[test]
    fn a_branded_header_carries_the_firm_name_and_accent_without_a_hash() {
        let json = as_json(&branded_header(&branding("Vortex MSP")));
        assert!(json.contains("Vortex MSP"));
        // Word wants RRGGBB, lowercased by our normalizer, never "#2D5F8B".
        assert!(json.contains("2d5f8b"), "was: {}", json);
        assert!(!json.contains("#2d5f8b"));
    }

    #[test]
    fn a_branded_footer_carries_its_line() {
        assert!(as_json(&branded_footer("Confidential")).contains("Confidential"));
    }

    #[test]
    fn headings_are_tinted_only_when_an_accent_is_given() {
        let inlines = [Inline::Text("Summary".to_string())];
        let plain = as_json(&heading_paragraph(1, &inlines, None));
        let tinted = as_json(&heading_paragraph(1, &inlines, Some("2d5f8b")));
        assert!(!plain.contains("2d5f8b"));
        assert!(tinted.contains("2d5f8b"));
    }

    #[test]
    fn an_unbranded_document_is_written_and_a_branded_one_differs() {
        let dir = tempfile::tempdir().unwrap();
        let plain_path = dir.path().join("plain.docx");
        let branded_path = dir.path().join("branded.docx");

        write_meeting_docx(
            &plain_path,
            "Q3 review",
            "2026-08-06",
            Some("## Topics\n- one\n\n1. first"),
            &[("00:01".to_string(), "hello".to_string())],
            None,
        )
        .unwrap();
        write_meeting_docx(
            &branded_path,
            "Q3 review",
            "2026-08-06",
            Some("## Topics\n- one\n\n1. first"),
            &[("00:01".to_string(), "hello".to_string())],
            Some(&branding("Vortex MSP")),
        )
        .unwrap();

        let plain = std::fs::read(&plain_path).unwrap();
        let branded = std::fs::read(&branded_path).unwrap();
        // Both are real zip containers.
        assert_eq!(&plain[..2], b"PK");
        assert_eq!(&branded[..2], b"PK");
        assert_ne!(plain, branded, "branding must change the document");
    }

    #[test]
    fn unconfigured_branding_produces_the_same_document_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.docx");
        let b = dir.path().join("b.docx");

        write_meeting_docx(&a, "T", "2026-01-01", None, &[], None).unwrap();
        write_meeting_docx(&b, "T", "2026-01-01", None, &[], Some(&Branding::default())).unwrap();

        // Zip metadata includes timestamps, so compare sizes rather than bytes:
        // an added header/footer part would change the length substantially.
        let a_len = std::fs::metadata(&a).unwrap().len();
        let b_len = std::fs::metadata(&b).unwrap().len();
        assert_eq!(a_len, b_len);
    }
}
