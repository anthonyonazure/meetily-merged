//! Markdown-subset → DOCX rendering via the `docx-rs` crate (pure Rust,
//! crates.io). Headings, paragraphs, bullet/numbered lists, and bold runs map
//! to native Word constructs; anything else arrives as plain paragraphs
//! because the parser already degraded it.

use crate::export::markdown_ast::{parse_markdown, Block, Inline};
use docx_rs::{
    AbstractNumbering, Docx, IndentLevel, Level, LevelJc, LevelText, NumberFormat, Numbering,
    NumberingId, Paragraph, Run, SpecialIndentType, Start,
};
use std::path::Path;

// Font sizes are in half-points (Word's unit): 22 = 11pt body text.
const SIZE_BODY: usize = 22;
const SIZE_TITLE: usize = 40;
const SIZE_H1: usize = 36;
const SIZE_H2: usize = 30;
const SIZE_H3: usize = 26;

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

fn heading_paragraph(level: u8, inlines: &[Inline]) -> Paragraph {
    let size = match level {
        1 => SIZE_H1,
        2 => SIZE_H2,
        _ => SIZE_H3,
    };
    paragraph_with_runs(runs_from_inlines(inlines, size, true))
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
/// timestamped paragraphs.
pub fn write_meeting_docx(
    path: &Path,
    title: &str,
    created_at: &str,
    summary_markdown: Option<&str>,
    transcripts: &[(String, String)],
) -> Result<(), String> {
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

    // Document header: title and date.
    docx = docx.add_paragraph(paragraph_with_runs(vec![Run::new()
        .add_text(title)
        .size(SIZE_TITLE)
        .bold()]));
    docx = docx.add_paragraph(paragraph_with_runs(vec![Run::new()
        .add_text(format!("Date: {}", created_at))
        .size(SIZE_BODY)]));

    // Summary section.
    docx = docx.add_paragraph(heading_paragraph(1, &[Inline::Text("Summary".to_string())]));
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
                        docx = docx.add_paragraph(heading_paragraph(level, &inlines));
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
