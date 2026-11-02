//! The block structure of the Markdown the site writes by hand: news articles and the Syntax
//! Guide's prose. Lines group the way CommonMark groups them. Consecutive lines of text are one
//! paragraph, so a line break in the source is a space on the page and a blank line ends the
//! paragraph; a list item and a quote continue across their wrapped lines; a line of `---` is a
//! rule. Each block's text is marked up inline by `ui::inline_markup`, the one renderer every
//! page's prose goes through.

/// One block of a Markdown document, in document order.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// `#` to `######`, and the heading's text.
    Heading { level: usize, text: String },
    /// Consecutive lines of text, joined by spaces.
    Paragraph(String),
    /// `-` or `*` items, or `1.`-numbered items when `ordered`, each item's wrapped lines joined.
    List { ordered: bool, items: Vec<String> },
    /// Consecutive `>` lines, as the quote's paragraphs: a bare `>` separates them.
    Quote(Vec<String>),
    /// `| a | b |` rows; `header` is the row above a `|---|` separator, when there is one.
    Table { header: Option<Vec<String>>, rows: Vec<Vec<String>> },
    /// A fenced block: the language its opening fence names (empty when it names none) and its
    /// lines exactly as written.
    Fence { tag: String, code: String },
    /// A thematic break: a line of three or more `-`, `*` or `_`, and nothing else.
    Rule,
}

/// The blocks `markdown` is made of.
pub fn blocks(markdown: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut open: Option<Block> = None;
    let mut fence: Option<(String, Vec<&str>)> = None;

    for line in markdown.lines() {
        let trimmed = line.trim();

        if let Some((tag, mut lines)) = fence.take() {
            if trimmed.starts_with("```") {
                out.push(fenced(tag, lines));
            } else {
                lines.push(line);
                fence = Some((tag, lines));
            }
            continue;
        }
        if let Some(tag) = trimmed.strip_prefix("```") {
            out.extend(open.take());
            fence = Some((tag.trim().to_string(), Vec::new()));
            continue;
        }
        if trimmed.is_empty() {
            out.extend(open.take());
            continue;
        }
        if let Some(block) = heading(trimmed) {
            out.extend(open.take());
            out.push(block);
            continue;
        }
        if is_rule(trimmed) {
            out.extend(open.take());
            out.push(Block::Rule);
            continue;
        }
        if let Some(text) = quoted(trimmed) {
            match &mut open {
                Some(Block::Quote(paragraphs)) if text.is_empty() => paragraphs.push(String::new()),
                Some(Block::Quote(paragraphs)) => join(paragraphs.last_mut().expect("a quote holds a paragraph"), text),
                _ => {
                    out.extend(open.take());
                    open = Some(Block::Quote(vec![text.to_string()]));
                }
            }
            continue;
        }
        if let Some(cells) = table_row(trimmed) {
            let separator = cells.iter().all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':')));
            match &mut open {
                Some(Block::Table { header, rows }) if separator => {
                    if header.is_none() && rows.len() == 1 {
                        *header = rows.pop();
                    }
                }
                Some(Block::Table { rows, .. }) => rows.push(cells),
                _ => {
                    out.extend(open.take());
                    if !separator {
                        open = Some(Block::Table { header: None, rows: vec![cells] });
                    }
                }
            }
            continue;
        }
        if let Some((ordered, item)) = list_item(trimmed) {
            match &mut open {
                Some(Block::List { ordered: current, items }) if *current == ordered => items.push(item.to_string()),
                _ => {
                    out.extend(open.take());
                    open = Some(Block::List { ordered, items: vec![item.to_string()] });
                }
            }
            continue;
        }
        match &mut open {
            Some(Block::Paragraph(text)) => join(text, trimmed),
            Some(Block::List { items, .. }) => join(items.last_mut().expect("a list holds an item"), trimmed),
            _ => {
                out.extend(open.take());
                open = Some(Block::Paragraph(trimmed.to_string()));
            }
        }
    }

    out.extend(fence.map(|(tag, lines)| fenced(tag, lines)));
    out.extend(open);
    out.into_iter()
        .filter_map(|block| match block {
            Block::Quote(paragraphs) => {
                let paragraphs: Vec<String> = paragraphs.into_iter().filter(|p| !p.is_empty()).collect();
                (!paragraphs.is_empty()).then_some(Block::Quote(paragraphs))
            }
            other => Some(other),
        })
        .collect()
}

/// A wrapped line continues the text before it after a space.
fn join(text: &mut String, more: &str) {
    if !text.is_empty() {
        text.push(' ');
    }
    text.push_str(more);
}

fn fenced(tag: String, lines: Vec<&str>) -> Block {
    Block::Fence { tag, code: lines.join("\n").trim_end_matches('\n').to_string() }
}

/// `### Title` as a level-3 heading; seven or more `#`, or none followed by a space, is text.
fn heading(line: &str) -> Option<Block> {
    let level = line.chars().take_while(|&c| c == '#').count();
    let text = line[level..].strip_prefix(' ')?;
    (1..=6).contains(&level).then(|| Block::Heading { level, text: text.trim().to_string() })
}

/// `---`, `***` or `___`, three or more of one mark with any spaces between, and nothing else.
fn is_rule(line: &str) -> bool {
    let mut marks = line.chars().filter(|c| !c.is_whitespace());
    let Some(mark) = marks.next().filter(|m| matches!(m, '-' | '*' | '_')) else { return false };
    let rest: Vec<char> = marks.collect();
    rest.len() >= 2 && rest.iter().all(|&c| c == mark)
}

/// The text of a quote line: `> text`, or a bare `>` between the quote's paragraphs.
fn quoted(line: &str) -> Option<&str> {
    if line == ">" {
        Some("")
    } else {
        line.strip_prefix("> ").map(str::trim)
    }
}

fn table_row(line: &str) -> Option<Vec<String>> {
    let inner = line.strip_prefix('|')?.strip_suffix('|')?;
    Some(inner.split('|').map(|cell| cell.trim().to_string()).collect())
}

/// `- item` and `* item` are bullets; `12. item` is numbered.
fn list_item(line: &str) -> Option<(bool, &str)> {
    if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        return Some((false, item.trim()));
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    let item = line[digits..].strip_prefix(". ")?;
    (digits > 0).then(|| (true, item.trim()))
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/markdown_blocks/tests.rs"]
mod tests;
