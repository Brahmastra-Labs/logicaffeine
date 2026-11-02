//! VS Code-style hover hints for the Studio's LOGOS editors.
//!
//! The words come from the language server's own analysis (`logicaffeine_lsp::hover`), linked
//! into the page: the same hover VS Code shows over a `.logos` file, with no server to reach.

use logicaffeine_lsp::document::DocumentState;

/// The language server's hover for the source at byte `offset`, as markdown.
pub fn hover_markdown(doc: &DocumentState, offset: usize) -> Option<String> {
    use logicaffeine_lsp::lsp_types::{HoverContents, MarkedString};
    let hover = logicaffeine_lsp::hover::hover(doc, doc.line_index.position(offset))?;
    let text = |m: MarkedString| match m {
        MarkedString::String(s) => s,
        MarkedString::LanguageString(code) => format!("```{}\n{}\n```", code.language, code.value),
    };
    let markdown = match hover.contents {
        HoverContents::Markup(m) => m.value,
        HoverContents::Scalar(m) => text(m),
        HoverContents::Array(ms) => ms.into_iter().map(text).collect::<Vec<_>>().join("\n\n"),
    };
    (!markdown.trim().is_empty()).then_some(markdown)
}

/// The hover's markdown as HTML for the tooltip: paragraphs, `---` rules, bullet lists, fenced
/// and inline code, bold and emphasis. Everything else is text, escaped.
pub fn markdown_to_html(markdown: &str) -> String {
    let mut html = String::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut bullets: Vec<&str> = Vec::new();
    let flush = |html: &mut String, paragraph: &mut Vec<&str>, bullets: &mut Vec<&str>| {
        if !paragraph.is_empty() {
            let lines: Vec<String> = paragraph.iter().map(|l| inline(l)).collect();
            html.push_str(&format!("<p>{}</p>", lines.join("<br>")));
            paragraph.clear();
        }
        if !bullets.is_empty() {
            html.push_str("<ul>");
            for item in bullets.iter() {
                html.push_str(&format!("<li>{}</li>", inline(item)));
            }
            html.push_str("</ul>");
            bullets.clear();
        }
    };
    let mut lines = markdown.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            flush(&mut html, &mut paragraph, &mut bullets);
            let code: Vec<&str> = lines.by_ref().take_while(|l| !l.trim_start().starts_with("```")).collect();
            html.push_str(&format!("<pre><code>{}</code></pre>", escape(&code.join("\n"))));
        } else if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            flush(&mut html, &mut paragraph, &mut bullets);
            html.push_str("<hr>");
        } else if let Some(item) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            if !paragraph.is_empty() {
                let pending = std::mem::take(&mut bullets);
                flush(&mut html, &mut paragraph, &mut Vec::new());
                bullets = pending;
            }
            bullets.push(item);
        } else if trimmed.is_empty() {
            flush(&mut html, &mut paragraph, &mut bullets);
        } else {
            if !bullets.is_empty() {
                flush(&mut html, &mut Vec::new(), &mut bullets);
            }
            paragraph.push(trimmed);
        }
    }
    flush(&mut html, &mut paragraph, &mut bullets);
    html
}

pub(crate) fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// One line of prose: `code` spans verbatim, then links, bold and emphasis in what is left.
fn inline(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let Some(len) = rest[open + 1..].find('`') else { break };
        out.push_str(&prose(&rest[..open]));
        out.push_str(&format!("<code>{}</code>", escape(&rest[open + 1..open + 1 + len])));
        rest = &rest[open + 1 + len + 1..];
    }
    out.push_str(&prose(rest));
    out
}

/// Text outside code spans: `[label](url)` links — only to web addresses, opened in a new
/// tab; any other target leaves just the label — then bold and emphasis.
fn prose(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(mid) = rest[open..].find("](").map(|i| open + i) else { break };
        let Some(close) = rest[mid + 2..].find(')').map(|i| mid + 2 + i) else { break };
        let (label, url) = (&rest[open + 1..mid], &rest[mid + 2..close]);
        out.push_str(&emphasis(&escape(&rest[..open])));
        if url.starts_with("https://") || url.starts_with("http://") {
            out.push_str(&format!(
                "<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a>",
                escape(url),
                emphasis(&escape(label))
            ));
        } else {
            out.push_str(&emphasis(&escape(label)));
        }
        rest = &rest[close + 1..];
    }
    out.push_str(&emphasis(&escape(rest)));
    out
}

/// `**bold**`, then `_emphasis_` / `*emphasis*` — each only when it opens and closes around
/// text, so `2 ** 3` and a `snake_case` name stay as they are.
fn emphasis(text: &str) -> String {
    let bold = wrap_pairs(text, "**", "strong");
    let em = wrap_pairs(&bold, "_", "em");
    wrap_pairs(&em, "*", "em")
}

fn wrap_pairs(text: &str, marker: &str, tag: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    loop {
        let Some(open) = find_marker(rest, marker, true) else { break };
        let after = &rest[open + marker.len()..];
        let Some(close) = find_marker(after, marker, false) else { break };
        let inner = &after[..close];
        out.push_str(&rest[..open]);
        out.push_str(&format!("<{tag}>{inner}</{tag}>"));
        rest = &after[close + marker.len()..];
    }
    out.push_str(rest);
    out
}

/// Where `marker` can open (followed by text, not after a letter) or close (after text, not
/// before a letter) a span.
fn find_marker(text: &str, marker: &str, opening: bool) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = text[from..].find(marker).map(|i| i + from) {
        let before = text[..i].chars().next_back();
        let after = text[i + marker.len()..].chars().next();
        let ok = if opening {
            after.is_some_and(|c| !c.is_whitespace()) && before.is_none_or(|c| !c.is_alphanumeric())
        } else {
            before.is_some_and(|c| !c.is_whitespace()) && after.is_none_or(|c| !c.is_alphanumeric())
        };
        if ok && !(marker != "**" && text[i..].starts_with("**")) {
            return Some(i);
        }
        from = i + marker.len();
    }
    None
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/hover_hint/tests.rs"]
mod tests;
