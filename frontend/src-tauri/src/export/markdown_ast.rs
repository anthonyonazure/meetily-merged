//! Tiny markdown parser for the subset our summaries actually use:
//! h1-h3 headings, paragraphs, bullet and numbered lists, and `**bold**`
//! inline runs. Anything fancier degrades gracefully to plain paragraphs,
//! which is exactly the fallback behavior the DOCX/HTML exporters want.

/// One inline run of text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Bold(String),
}

/// One block-level element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// level is clamped to 1..=3; deeper headings render as level 3.
    Heading { level: u8, inlines: Vec<Inline> },
    Paragraph(Vec<Inline>),
    /// depth 0 = top level; clamped to 0..=2.
    Bullet { depth: u8, inlines: Vec<Inline> },
    Numbered { depth: u8, inlines: Vec<Inline> },
}

/// Splits `**bold**` spans out of a line. Unbalanced markers stay literal.
pub fn parse_inlines(text: &str) -> Vec<Inline> {
    let mut inlines = Vec::new();
    let mut rest = text;
    loop {
        match rest.find("**") {
            Some(open) => {
                let after_open = &rest[open + 2..];
                match after_open.find("**") {
                    Some(close) if close > 0 => {
                        if open > 0 {
                            inlines.push(Inline::Text(rest[..open].to_string()));
                        }
                        inlines.push(Inline::Bold(after_open[..close].to_string()));
                        rest = &after_open[close + 2..];
                    }
                    _ => {
                        // No closing marker (or empty bold): keep everything literal.
                        if !rest.is_empty() {
                            inlines.push(Inline::Text(rest.to_string()));
                        }
                        break;
                    }
                }
            }
            None => {
                if !rest.is_empty() {
                    inlines.push(Inline::Text(rest.to_string()));
                }
                break;
            }
        }
    }
    if inlines.is_empty() {
        inlines.push(Inline::Text(String::new()));
    }
    inlines
}

fn list_depth(indent_spaces: usize) -> u8 {
    ((indent_spaces / 2) as u8).min(2)
}

/// Returns Some((depth, content)) when the line is a bullet item.
fn parse_bullet(line: &str) -> Option<(u8, &str)> {
    let indent = line.len() - line.trim_start().len();
    let trimmed = line.trim_start();
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return Some((list_depth(indent), rest.trim_start()));
        }
    }
    None
}

/// Returns Some((depth, content)) when the line is a numbered item.
fn parse_numbered(line: &str) -> Option<(u8, &str)> {
    let indent = line.len() - line.trim_start().len();
    let trimmed = line.trim_start();
    let digits: usize = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 4 {
        return None;
    }
    let after = &trimmed[digits..];
    for marker in [". ", ") "] {
        if let Some(rest) = after.strip_prefix(marker) {
            return Some((list_depth(indent), rest.trim_start()));
        }
    }
    None
}

/// Returns Some((level, content)) when the line is an ATX heading.
fn parse_heading(line: &str) -> Option<(u8, &str)> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = trimmed[hashes..].strip_prefix(' ')?;
    Some(((hashes as u8).min(3), rest.trim()))
}

/// Parses markdown into blocks. Consecutive plain lines merge into one
/// paragraph; blank lines separate blocks.
pub fn parse_markdown(markdown: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<String> = Vec::new();

    let flush_paragraph = |paragraph: &mut Vec<String>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            let text = paragraph.join(" ");
            blocks.push(Block::Paragraph(parse_inlines(&text)));
            paragraph.clear();
        }
    };

    for line in markdown.lines() {
        if line.trim().is_empty() {
            flush_paragraph(&mut paragraph, &mut blocks);
            continue;
        }
        if let Some((level, content)) = parse_heading(line) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(Block::Heading {
                level,
                inlines: parse_inlines(content),
            });
            continue;
        }
        if let Some((depth, content)) = parse_bullet(line) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(Block::Bullet {
                depth,
                inlines: parse_inlines(content),
            });
            continue;
        }
        if let Some((depth, content)) = parse_numbered(line) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(Block::Numbered {
                depth,
                inlines: parse_inlines(content),
            });
            continue;
        }
        paragraph.push(line.trim().to_string());
    }
    flush_paragraph(&mut paragraph, &mut blocks);
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_bullets_numbers_and_paragraphs_parse() {
        let md = "# Title\n\nSome **bold** text\nsecond line\n\n- item one\n  - nested\n1. first\n2) second\n\n#### Deep heading";
        let blocks = parse_markdown(md);
        assert_eq!(
            blocks[0],
            Block::Heading { level: 1, inlines: vec![Inline::Text("Title".into())] }
        );
        assert_eq!(
            blocks[1],
            Block::Paragraph(vec![
                Inline::Text("Some ".into()),
                Inline::Bold("bold".into()),
                Inline::Text(" text second line".into()),
            ])
        );
        assert_eq!(
            blocks[2],
            Block::Bullet { depth: 0, inlines: vec![Inline::Text("item one".into())] }
        );
        assert_eq!(
            blocks[3],
            Block::Bullet { depth: 1, inlines: vec![Inline::Text("nested".into())] }
        );
        assert_eq!(
            blocks[4],
            Block::Numbered { depth: 0, inlines: vec![Inline::Text("first".into())] }
        );
        assert_eq!(
            blocks[5],
            Block::Numbered { depth: 0, inlines: vec![Inline::Text("second".into())] }
        );
        // Headings deeper than h3 clamp to level 3.
        assert_eq!(
            blocks[6],
            Block::Heading { level: 3, inlines: vec![Inline::Text("Deep heading".into())] }
        );
    }

    #[test]
    fn unbalanced_bold_markers_stay_literal() {
        assert_eq!(
            parse_inlines("no **closing marker"),
            vec![Inline::Text("no **closing marker".into())]
        );
    }

    #[test]
    fn plain_text_without_markup_is_one_paragraph() {
        let blocks = parse_markdown("just words\n");
        assert_eq!(
            blocks,
            vec![Block::Paragraph(vec![Inline::Text("just words".into())])]
        );
    }

    #[test]
    fn non_list_number_lines_stay_paragraphs() {
        // "2026 was a big year" must not become a numbered item.
        let blocks = parse_markdown("2026 was a big year");
        assert!(matches!(blocks[0], Block::Paragraph(_)));
    }
}
