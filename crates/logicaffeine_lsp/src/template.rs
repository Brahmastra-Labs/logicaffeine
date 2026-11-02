//! The LGX template vocabulary the editor completes and paints with.
//!
//! A `View:` / `Head:` body is HTML whose `{…}` groups are either a TAG — `{if c:}`,
//! `{for each x in xs:}`, `{end}` — or a HOLE: a LOGOS expression, optionally piped through
//! filters (`{price | money}`). The template parser in `logicaffeine_language::lgx::template` owns
//! the grammar; this module holds the two things an editor needs from it, once: the phrase that
//! opens each tag (with the shape `MalformedTag` teaches), and the clause words inside a tag.

/// Every template tag: the phrase that opens it, and its shape as `MalformedTag` spells it.
///
/// A longer phrase precedes any phrase it begins with (`else if` before `else`), so the first
/// match against a tag's text is the right one.
pub const TAGS: &[(&str, &str)] = &[
    ("else if", "{else if condition:}"),
    ("else", "{else:}"),
    ("if", "{if condition:} … {end}"),
    (
        "for each",
        "{for each item in items[, keyed by key][, at index i]:} … {end}",
    ),
    ("inspect", "{inspect value:} {when Pattern:} … {end}"),
    ("when", "{when Pattern:}"),
    ("let", "{let name be expression}"),
    ("extends", "{extends Layout}"),
    ("block", "{block name:} … {end}"),
    ("children", "{children} — the caller's markup"),
    (
        "raw",
        "{raw expression} — unescaped, so only for markup you trust",
    ),
    ("end", "{end}"),
];

/// The clause words a `{for each …:}` tag carries after its pattern.
pub const FOR_EACH_CLAUSES: &[&str] = &["in", "keyed by", "at index"];

/// The tag phrase `inner` (a `{…}` group's text, braces excluded) opens with, and the byte length
/// of the phrase within `inner` including any leading whitespace — `None` for a hole.
pub fn tag_phrase(inner: &str) -> Option<(&'static str, usize)> {
    let trimmed = inner.trim_start();
    let lead = inner.len() - trimmed.len();
    TAGS.iter().map(|(phrase, _)| *phrase).find_map(|phrase| {
        let rest = trimmed.strip_prefix(phrase)?;
        let ends_word = rest
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_');
        ends_word.then_some((phrase, lead + phrase.len()))
    })
}

/// A `{# … #}` template comment.
pub fn is_comment(inner: &str) -> bool {
    inner.trim_start().starts_with('#')
}

/// The byte ranges of `text`'s top-level `|` separators — the filter pipes of a hole. A `|` inside
/// a string or a bracketed group is part of an expression, not a pipe.
pub fn filter_pipes(text: &str) -> Vec<usize> {
    let mut pipes = Vec::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '|' if depth == 0 => pipes.push(i),
            _ => {}
        }
    }
    pipes
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/template/tests.rs"]
mod tests;
