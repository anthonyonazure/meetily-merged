//! Markdown-subset → print-styled standalone HTML.
//!
//! PDF decision, logged honestly: we evaluated the pure-Rust PDF crates and
//! chose the print pipeline instead. `genpdf` needs external TTF font files
//! shipped with the app, and `printpdf`'s built-in PDF fonts are
//! WinAnsi-only, which silently mangles any non-Latin meeting content (this
//! codebase explicitly supports non-English titles and transcripts). A
//! browser's print-to-PDF engine handles Unicode, wrapping, and pagination
//! correctly, so "PDF export" writes a self-contained print-styled HTML file
//! the user prints to PDF (Cmd+P / Ctrl+P). No external assets, no network:
//! all CSS is inlined.

use crate::branding::Branding;
use crate::export::markdown_ast::{parse_markdown, Block, Inline};

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn inlines_to_html(inlines: &[Inline]) -> String {
    inlines
        .iter()
        .map(|inline| match inline {
            Inline::Text(text) => escape_html(text),
            Inline::Bold(text) => format!("<strong>{}</strong>", escape_html(text)),
        })
        .collect()
}

/// Renders markdown blocks, opening/closing list tags around contiguous list
/// items. Nested depth renders via CSS margin classes instead of nested tags
/// (simpler, prints identically for our subset).
/// `pub(crate)`: also reused by the M365 integration to render summary
/// markdown into Outlook draft bodies.
pub(crate) fn blocks_to_html(blocks: &[Block]) -> String {
    let mut html = String::new();
    let mut open_list: Option<&'static str> = None;

    let close_list = |html: &mut String, open_list: &mut Option<&'static str>| {
        if let Some(tag) = open_list.take() {
            html.push_str(&format!("</{}>\n", tag));
        }
    };

    for block in blocks {
        match block {
            Block::Heading { level, inlines } => {
                close_list(&mut html, &mut open_list);
                // Shift heading levels down one so the document h1 stays unique.
                let tag_level = (*level as usize + 1).min(4);
                html.push_str(&format!(
                    "<h{level}>{content}</h{level}>\n",
                    level = tag_level,
                    content = inlines_to_html(inlines)
                ));
            }
            Block::Paragraph(inlines) => {
                close_list(&mut html, &mut open_list);
                html.push_str(&format!("<p>{}</p>\n", inlines_to_html(inlines)));
            }
            Block::Bullet { depth, inlines } => {
                if open_list != Some("ul") {
                    close_list(&mut html, &mut open_list);
                    html.push_str("<ul>\n");
                    open_list = Some("ul");
                }
                html.push_str(&format!(
                    "<li class=\"d{}\">{}</li>\n",
                    depth,
                    inlines_to_html(inlines)
                ));
            }
            Block::Numbered { depth, inlines } => {
                if open_list != Some("ol") {
                    close_list(&mut html, &mut open_list);
                    html.push_str("<ol>\n");
                    open_list = Some("ol");
                }
                html.push_str(&format!(
                    "<li class=\"d{}\">{}</li>\n",
                    depth,
                    inlines_to_html(inlines)
                ));
            }
        }
    }
    close_list(&mut html, &mut open_list);
    html
}

const STYLE: &str = r#"
  body { font-family: -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
         max-width: 46rem; margin: 2rem auto; padding: 0 1.5rem; color: #1a1a1a; line-height: 1.55; }
  h1 { font-size: 1.7rem; margin: 0 0 0.25rem; }
  h2 { font-size: 1.25rem; margin-top: 1.6rem; border-bottom: 1px solid #ddd; padding-bottom: 0.2rem; }
  h3 { font-size: 1.05rem; margin-top: 1.2rem; }
  h4 { font-size: 1rem; margin-top: 1rem; }
  .meta { color: #666; margin-bottom: 1.5rem; }
  li.d1 { margin-left: 1.25rem; }
  li.d2 { margin-left: 2.5rem; }
  .transcript p { margin: 0.35rem 0; }
  .ts { font-weight: 600; color: #444; }
  .print-hint { background: #eef4ff; border: 1px solid #c8d8f8; border-radius: 6px;
                padding: 0.6rem 0.9rem; font-size: 0.9rem; color: #234; margin-bottom: 1.5rem; }
  @media print { .print-hint { display: none; } body { margin: 0 auto; } }
  @page { margin: 18mm; }
"#;

/// The branded additions: a rule under the letterhead, the accent on headings,
/// and a footer that repeats on every printed page.
///
/// Injected only when branding is configured, so an unbranded export produces
/// exactly the same bytes it always has.
fn branded_style(accent: &str) -> String {
    format!(
        r#"
  .letterhead {{ display: flex; align-items: center; gap: 0.9rem;
                 border-bottom: 3px solid {accent}; padding-bottom: 0.75rem; margin-bottom: 1.5rem; }}
  .letterhead img {{ max-height: 56px; max-width: 220px; }}
  .letterhead .firm {{ font-size: 1.15rem; font-weight: 600; color: {accent}; }}
  h1 {{ color: {accent}; }}
  h2 {{ color: {accent}; border-bottom-color: {accent}; }}
  .doc-footer {{ margin-top: 2.5rem; padding-top: 0.75rem; border-top: 1px solid {accent};
                 font-size: 0.8rem; color: #555; }}
  @media print {{ .doc-footer {{ position: fixed; bottom: 0; left: 0; right: 0; }} }}
"#
    )
}

/// The letterhead block: the logo (inlined as a data URI so the file travels
/// alone) and the firm name.
fn letterhead_html(branding: &Branding) -> String {
    let logo = branding
        .logo_to_embed()
        .and_then(crate::branding::assets::logo_as_data_uri)
        .map(|uri| {
            format!(
                "<img src=\"{}\" alt=\"{}\">",
                uri,
                escape_html(branding.firm_name.trim())
            )
        })
        .unwrap_or_default();
    format!(
        "<div class=\"letterhead\">{logo}<div class=\"firm\">{firm}</div></div>\n",
        logo = logo,
        firm = escape_html(branding.firm_name.trim())
    )
}

/// Builds a fully self-contained, print-styled HTML document for one meeting.
///
/// `branding` is the firm's letterhead, footer, and accent colour, or None for an
/// unbranded export. Nothing in this document references the app that produced
/// it: a deliverable a client reads carries the firm's marks, not the tool's.
pub fn build_meeting_html(
    title: &str,
    created_at: &str,
    summary_markdown: Option<&str>,
    transcripts: &[(String, String)],
    branding: Option<&Branding>,
) -> String {
    let summary_html = match summary_markdown {
        Some(markdown) => blocks_to_html(&parse_markdown(markdown)),
        None => "<p><em>No summary generated.</em></p>\n".to_string(),
    };

    let mut transcript_html = String::new();
    for (timestamp, text) in transcripts {
        if text.trim().is_empty() {
            continue;
        }
        transcript_html.push_str(&format!(
            "<p><span class=\"ts\">[{}]</span> {}</p>\n",
            escape_html(timestamp),
            escape_html(text.trim())
        ));
    }

    let branding = branding.filter(|b| b.is_configured());
    let extra_style = branding
        .map(|b| branded_style(&b.accent()))
        .unwrap_or_default();
    let letterhead = branding.map(letterhead_html).unwrap_or_default();
    let footer = branding
        .and_then(|b| b.footer_to_print())
        .map(|text| format!("<div class=\"doc-footer\">{}</div>\n", escape_html(text)))
        .unwrap_or_default();

    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>{title}</title>\n<style>{style}{extra_style}</style>\n</head>\n<body>\n\
         <div class=\"print-hint\">To save this meeting as a PDF, print this page \
         (Cmd+P on macOS, Ctrl+P on Windows/Linux) and choose \"Save as PDF\".</div>\n\
         {letterhead}\
         <h1>{title}</h1>\n<p class=\"meta\">Date: {date}</p>\n\
         <h2>Summary</h2>\n{summary}\
         <h2>Transcript</h2>\n<div class=\"transcript\">\n{transcript}</div>\n\
         {footer}</body>\n</html>\n",
        title = escape_html(title),
        style = STYLE,
        extra_style = extra_style,
        letterhead = letterhead,
        date = escape_html(created_at),
        summary = summary_html,
        transcript = transcript_html,
        footer = footer,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branding(firm: &str) -> Branding {
        Branding {
            firm_name: firm.to_string(),
            logo_path: None,
            footer_text: "Confidential — prepared for the client".to_string(),
            accent_hex: "#2d5f8b".to_string(),
            include_logo: true,
            include_footer: true,
        }
    }

    #[test]
    fn an_unbranded_export_has_no_letterhead_footer_or_accent() {
        let html = build_meeting_html("Standup", "2026-07-12", None, &[], None);
        assert!(!html.contains("letterhead"));
        assert!(!html.contains("doc-footer"));
        assert!(!html.contains("#2d5f8b"));
    }

    #[test]
    fn unconfigured_branding_is_the_same_as_none() {
        // The switch is the firm name, so an all-defaults row must not brand.
        let with_blank = build_meeting_html(
            "Standup",
            "2026-07-12",
            None,
            &[],
            Some(&Branding::default()),
        );
        let without = build_meeting_html("Standup", "2026-07-12", None, &[], None);
        assert_eq!(with_blank, without);
    }

    #[test]
    fn a_branded_export_carries_the_firm_name_footer_and_accent() {
        let html = build_meeting_html(
            "Q3 review",
            "2026-07-12",
            None,
            &[],
            Some(&branding("Vortex MSP")),
        );
        assert!(html.contains("class=\"letterhead\""));
        assert!(html.contains("Vortex MSP"));
        assert!(html.contains("class=\"doc-footer\""));
        assert!(html.contains("Confidential — prepared for the client"));
        assert!(html.contains("#2d5f8b"), "the accent reaches the stylesheet");
    }

    #[test]
    fn a_junk_accent_never_reaches_the_stylesheet() {
        let mut junk = branding("Firm");
        junk.accent_hex = "red; } body { display: none".to_string();
        let html = build_meeting_html("T", "2026-01-01", None, &[], Some(&junk));
        // The injected rule must not appear. (The base stylesheet has its own
        // `display: none` for the print hint, so the assertion names the payload.)
        assert!(!html.contains("body { display: none"));
        assert!(!html.contains("red;"));
        assert!(html.contains("#23252b"), "falls back to the default accent");
    }

    #[test]
    fn a_branded_firm_name_is_escaped_like_any_other_content() {
        let html = build_meeting_html(
            "T",
            "2026-01-01",
            None,
            &[],
            Some(&branding("Smith & <Co>")),
        );
        assert!(html.contains("Smith &amp; &lt;Co&gt;"));
        assert!(!html.contains("<Co>"));
    }

    #[test]
    fn the_footer_can_be_switched_off_without_clearing_its_text() {
        let mut branding = branding("Firm");
        branding.include_footer = false;
        let html = build_meeting_html("T", "2026-01-01", None, &[], Some(&branding));
        assert!(html.contains("letterhead"), "the letterhead is unaffected");
        // The CSS rule may exist; the element must not.
        assert!(!html.contains("class=\"doc-footer\""));
        assert!(!html.contains("Confidential"));
    }

    #[test]
    fn a_client_facing_export_carries_no_app_marks() {
        // The competitive point, enforced: if anyone adds a vendor stamp to the
        // deliverable, this fails.
        let html = build_meeting_html(
            "Q3 review",
            "2026-07-12",
            Some("## Topics\n- one"),
            &[("00:01".to_string(), "hello".to_string())],
            Some(&branding("Vortex MSP")),
        );
        let lowered = html.to_lowercase();
        for mark in ["meetily", "meetily++", "generated by", "powered by"] {
            assert!(!lowered.contains(mark), "export leaked the mark {:?}", mark);
        }
    }

    #[test]
    fn html_escapes_content_and_wraps_lists() {
        let html = build_meeting_html(
            "Q3 <review>",
            "2026-07-12",
            Some("## Topics\n- item **one**\n- two\n\n1. first"),
            &[("00:01".to_string(), "Hello & welcome".to_string())],
            None,
        );
        assert!(html.contains("Q3 &lt;review&gt;"));
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li class=\"d0\">item <strong>one</strong></li>"));
        assert!(html.contains("<ol>"));
        assert!(html.contains("Hello &amp; welcome"));
        // Summary h2 headings shift to h3 so the page keeps a single h1/h2 spine.
        assert!(html.contains("<h3>Topics</h3>"));
        assert!(html.contains("print-hint"));
    }

    #[test]
    fn missing_summary_says_so() {
        let html = build_meeting_html("T", "2026-01-01", None, &[], None);
        assert!(html.contains("No summary generated."));
    }
}
