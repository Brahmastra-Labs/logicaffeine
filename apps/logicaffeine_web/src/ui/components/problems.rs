//! The language server's diagnostics as the Studio's editor shows them: a squiggle under the
//! words each one names, and the problem at the top of the hover — VS Code's Problems, in the
//! page, from the same analysis (`logicaffeine_lsp`) the server publishes.

use logicaffeine_lsp::document::DocumentState;

use super::hover_hint::{escape, markdown_to_html};

/// How loud a problem is, quietest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Hint,
    Info,
    Warning,
    Error,
}

/// One diagnostic, placed in the source by byte offsets.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    /// Never empty, and always covers at least one visible character, so there is always
    /// something to draw the squiggle under.
    pub start: usize,
    pub end: usize,
    pub severity: Severity,
    /// Code that does nothing (an unused binding): drawn faded instead of squiggled.
    pub unnecessary: bool,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
    /// Where the code is explained.
    pub docs: Option<String>,
}

/// Every problem the language server finds in the document.
pub fn problems(doc: &DocumentState) -> Vec<Problem> {
    use logicaffeine_lsp::lsp_types::{DiagnosticSeverity, DiagnosticTag, NumberOrString};
    doc.diagnostics
        .iter()
        .map(|d| {
            let (start, end) = word_span(
                &doc.source,
                doc.line_index.offset(d.range.start),
                doc.line_index.offset(d.range.end),
            );
            Problem {
                start,
                end,
                // A diagnostic without a severity is an error — VS Code reads it the same way.
                severity: match d.severity {
                    Some(DiagnosticSeverity::WARNING) => Severity::Warning,
                    Some(DiagnosticSeverity::INFORMATION) => Severity::Info,
                    Some(DiagnosticSeverity::HINT) => Severity::Hint,
                    _ => Severity::Error,
                },
                unnecessary: d.tags.as_ref().is_some_and(|t| t.contains(&DiagnosticTag::UNNECESSARY)),
                message: d.message.clone(),
                source: d.source.clone(),
                code: d.code.as_ref().map(|c| match c {
                    NumberOrString::String(s) => s.clone(),
                    NumberOrString::Number(n) => n.to_string(),
                }),
                docs: d.code_description.as_ref().map(|c| c.href.to_string()),
            }
        })
        .collect()
}

/// The span a problem is drawn under: its own range when that shows anything, otherwise the
/// word it touches — the one at its start, or else the nearest one before it, or after it.
fn word_span(source: &str, start: usize, end: usize) -> (usize, usize) {
    let floor = |mut i: usize| {
        i = i.min(source.len());
        while !source.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let ceil = |mut i: usize| {
        i = i.min(source.len());
        while !source.is_char_boundary(i) {
            i += 1;
        }
        i
    };
    let empty = end <= start;
    let start = floor(start);
    let end = if empty { start } else { ceil(end) };
    if !source[start..end].trim().is_empty() {
        return (start, end);
    }
    let word = |at: usize| {
        let from = source[..at].rfind(char::is_whitespace).map_or(0, |i| i + source[i..].chars().next().map_or(1, char::len_utf8));
        let to = source[at..].find(char::is_whitespace).map_or(source.len(), |i| at + i);
        (from, to)
    };
    let visible_at = |i: usize| source[i..].chars().next().is_some_and(|c| !c.is_whitespace());
    if visible_at(start) {
        return word(start);
    }
    if let Some((i, _)) = source[..start].char_indices().rev().find(|(_, c)| !c.is_whitespace()) {
        return word(i);
    }
    if let Some((i, _)) = source[start..].char_indices().find(|(_, c)| !c.is_whitespace()) {
        return word(start + i);
    }
    let next = source[start..].chars().next().map_or(0, char::len_utf8);
    (start, start + next)
}

/// The classes for the text in `start..end`: the most severe squiggle over it, and a fade
/// when any code there does nothing.
pub fn squiggle_class(problems: &[Problem], start: usize, end: usize) -> String {
    let over: Vec<&Problem> = problems.iter().filter(|p| p.start < end && start < p.end).collect();
    // Unnecessary code is shown by its fade; only its hints go without a squiggle.
    let squiggle = over
        .iter()
        .filter(|p| !(p.unnecessary && p.severity == Severity::Hint))
        .map(|p| p.severity)
        .max()
        .map(|s| match s {
            Severity::Error => "sq-error",
            Severity::Warning => "sq-warning",
            Severity::Info => "sq-info",
            Severity::Hint => "sq-hint",
        });
    let fade = over.iter().any(|p| p.unnecessary).then_some("sq-unnecessary");
    [squiggle, fade].into_iter().flatten().collect::<Vec<_>>().join(" ")
}

/// The problems at `offset` as the top of a hover: each message, then where it comes from,
/// its code linked to the explanation.
pub fn problems_html(problems: &[Problem], offset: usize) -> Option<String> {
    let mut here: Vec<&Problem> = problems.iter().filter(|p| p.start <= offset && offset < p.end).collect();
    if here.is_empty() {
        return None;
    }
    here.sort_by(|a, b| b.severity.cmp(&a.severity));
    Some(
        here.iter()
            .map(|p| {
                let code = p.code.as_deref().map(|code| match p.docs.as_deref() {
                    Some(url) if url.starts_with("https://") || url.starts_with("http://") => format!(
                        "<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a>",
                        escape(url),
                        escape(code)
                    ),
                    _ => escape(code),
                });
                let origin = match (p.source.as_deref().map(escape), code) {
                    (Some(source), Some(code)) => format!("{source}({code})"),
                    (Some(only), None) | (None, Some(only)) => only,
                    (None, None) => String::new(),
                };
                let origin = if origin.is_empty() { origin } else { format!("<p class=\"hint-source\">{origin}</p>") };
                format!("<div class=\"hint-problem\">{}{origin}</div>", markdown_to_html(&p.message))
            })
            .collect(),
    )
}

/// Problems found in `old`, carried into `new` until it is analysed afresh: the text before
/// and after an edit keeps its problems, moved with it; a problem the edit touched is dropped.
pub fn rebase(problems: &[Problem], old: &str, new: &str) -> Vec<Problem> {
    let mut prefix = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let room = old.len().min(new.len()) - prefix;
    let mut suffix = old.bytes().rev().zip(new.bytes().rev()).take_while(|(a, b)| a == b).count().min(room);
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    let edited_end = old.len() - suffix;
    let moved_by = new.len() as isize - old.len() as isize;
    problems
        .iter()
        .filter_map(|p| {
            if p.end <= prefix {
                Some(p.clone())
            } else if p.start >= edited_end {
                let shift = |i: usize| (i as isize + moved_by) as usize;
                Some(Problem { start: shift(p.start), end: shift(p.end), ..p.clone() })
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/problems/tests.rs"]
mod tests;
