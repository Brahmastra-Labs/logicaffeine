//! Whether a line of hand-written Markdown reached the rendered page. The page's text and the
//! line's text are compared as runs of words, so markup (`**`, backticks, link targets, tags,
//! entities) and where the source wrapped its lines never decide the answer.

/// The words of `text`: its runs of letters and digits, lower-cased.
pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// The text a reader sees in `html`: tags dropped, the entities the renderers write decoded.
pub fn visible_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// The words a line of Markdown shows: its block marker, its link targets and its inline
/// markup left out.
pub fn shown_words(line: &str) -> Vec<String> {
    let mut text = line.trim().to_string();
    while let Some(at) = text.find("](") {
        let Some(len) = text[at..].find(')') else { break };
        text.replace_range(at + 1..at + len + 1, "");
    }
    let text = text.trim_start_matches(|c: char| c == '#' || c == '>' || c.is_whitespace());
    let text = text.strip_prefix("- ").or_else(|| text.strip_prefix("* ")).unwrap_or(text);
    let text = match text.find(". ") {
        Some(at) if at > 0 && text[..at].chars().all(|c| c.is_ascii_digit()) => &text[at + 2..],
        _ => text,
    };
    words(text)
}

/// Whether `needle` occurs in `haystack` as one unbroken run.
pub fn contains_run(haystack: &[String], needle: &[String]) -> bool {
    needle.is_empty() || haystack.windows(needle.len()).any(|window| window == needle)
}

/// The prose lines of `markdown` a reader should find on the page: every non-blank line
/// outside a fenced block, except a table's `|---|` separator row.
pub fn prose_lines(markdown: &str) -> Vec<&str> {
    let mut in_fence = false;
    let mut out = Vec::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        let separator = trimmed.starts_with('|') && trimmed.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
        if !in_fence && !trimmed.is_empty() && !separator {
            out.push(line);
        }
    }
    out
}
