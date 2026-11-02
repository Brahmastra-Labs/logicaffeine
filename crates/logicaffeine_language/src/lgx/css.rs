//! Component-scoped CSS for the `Style:` section — a real tokenizer, not string surgery.
//!
//! [`scope`] stamps the component's scope class onto the SUBJECT compound of every complex
//! selector (`.card h1` → `.card h1.lgx-todos`). Every element the component's own view
//! renders carries that class (stamped at compile time), and a child component's elements
//! do not, so a scoped rule can match only this component's elements — no wrapper element,
//! no leakage in either direction. `:global(…)` opts a compound out of scoping.
//!
//! Group at-rules (`@media`, `@supports`, `@container`, `@layer`, `@scope`,
//! `@starting-style`) have a nested RULE LIST for a body, so scoping recurses into them;
//! every other block at-rule (`@keyframes`, `@font-face`, `@page`, `@property`, …) passes
//! through VERBATIM (a keyframe's `0%`/`to` are offsets, not selectors), and a statement
//! at-rule (`@import …;`) is untouched. A prefixed at-rule would be invalid CSS a browser
//! silently drops — which is why the prelude of an at-rule is never treated as a selector.
//! Declarations are copied byte for byte; comments are removed.
//!
//! Every offset an error reports is a byte offset into the stylesheet as written, at any depth:
//! comments are blanked in place rather than cut out, and a group at-rule's body is parsed at its
//! own position.

/// Conditional-group at-rules whose body is a nested rule list.
pub const GROUP_AT_RULES: &[&str] = &[
    "media",
    "supports",
    "container",
    "layer",
    "scope",
    "starting-style",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CssErrorKind {
    /// A `{` without its `}` or a `}` without its `{`.
    UnbalancedBraces,
    /// A `"…"`/`'…'` string or a `(`…`)` that never closes.
    UnterminatedString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssError {
    pub kind: CssErrorKind,
    /// Byte offset of the offending character.
    pub at: usize,
}

impl CssError {
    pub fn message(&self) -> String {
        match self.kind {
            CssErrorKind::UnbalancedBraces => "The braces in this stylesheet do not balance: every rule is `selector { declarations }` and every `{` needs its `}`.".into(),
            CssErrorKind::UnterminatedString => "A string or `(` in this stylesheet never closes.".into(),
        }
    }
}

/// One item of a rule list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CssItem {
    /// `prelude { body }` — a style rule; `body` is the raw declaration text.
    Rule { prelude: String, body: String },
    /// `@name prelude { body }` or `@name prelude;` (`body: None`). `body_at` is the byte offset
    /// of the body's first character in the stylesheet. The body keeps its comments BLANKED (each
    /// byte replaced by NUL) rather than removed, so the offsets of anything parsed out of it are
    /// still offsets into the stylesheet; [`parse_items`] of the body reads it.
    AtRule {
        name: String,
        prelude: String,
        body: Option<String>,
        body_at: usize,
    },
}

/// Split a stylesheet into its top-level items (comments removed).
pub fn parse_items(css: &str) -> Result<Vec<CssItem>, CssError> {
    parse_items_at(css, 0)
}

/// [`parse_items`] for text that begins at byte `base` of the stylesheet, so every offset it
/// reports is an offset into the whole stylesheet.
fn parse_items_at(css: &str, base: usize) -> Result<Vec<CssItem>, CssError> {
    let at = |e: CssError| CssError {
        kind: e.kind,
        at: e.at + base,
    };
    let items = parse_blanked(&blank_comments(css).map_err(at)?, base).map_err(at)?;
    Ok(items.into_iter().map(|item| unblank(item)).collect())
}

/// The items of `stripped` (comments already blanked), with each at-rule body's offset absolute.
fn parse_blanked(stripped: &str, base: usize) -> Result<Vec<CssItem>, CssError> {
    let stripped = stripped.to_string();
    let b = stripped.as_bytes();
    let mut items = Vec::new();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == BLANK as u8) {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        if b[i] == b'}' {
            return Err(CssError {
                kind: CssErrorKind::UnbalancedBraces,
                at: i,
            });
        }
        // Prelude runs to the first top-level `{` (a rule / block at-rule) or `;` (a statement at-rule).
        let prelude_start = i;
        let mut depth_paren = 0usize;
        loop {
            if i >= b.len() {
                // A prelude with no block and no `;` — a bare `@import …` at EOF, or garbage.
                let text = trim_blank(&stripped[prelude_start..]);
                if text.is_empty() {
                    break;
                }
                if let Some(rest) = text.strip_prefix('@') {
                    let (name, prelude) = split_at_name(rest);
                    items.push(CssItem::AtRule {
                        name,
                        prelude,
                        body: None,
                        body_at: base + i,
                    });
                    break;
                }
                return Err(CssError {
                    kind: CssErrorKind::UnbalancedBraces,
                    at: prelude_start,
                });
            }
            match b[i] {
                b'"' | b'\'' => i = skip_string(&stripped, i)?,
                b'(' => {
                    depth_paren += 1;
                    i += 1;
                }
                b')' => {
                    depth_paren = depth_paren.saturating_sub(1);
                    i += 1;
                }
                b';' if depth_paren == 0 => {
                    let text = trim_blank(&stripped[prelude_start..i]);
                    if let Some(rest) = text.strip_prefix('@') {
                        let (name, prelude) = split_at_name(rest);
                        items.push(CssItem::AtRule {
                            name,
                            prelude,
                            body: None,
                            body_at: base + i,
                        });
                    }
                    // A stray `;` between rules is dropped.
                    i += 1;
                    break;
                }
                b'{' if depth_paren == 0 => {
                    let prelude = trim_blank(&stripped[prelude_start..i]).to_string();
                    let body_start = i + 1;
                    let body_end = matching_brace(&stripped, i)?;
                    let body = stripped[body_start..body_end].to_string();
                    if let Some(rest) = prelude.strip_prefix('@') {
                        let (name, prelude) = split_at_name(rest);
                        items.push(CssItem::AtRule {
                            name,
                            prelude,
                            body: Some(body),
                            body_at: base + body_start,
                        });
                    } else {
                        items.push(CssItem::Rule { prelude, body });
                    }
                    i = body_end + 1;
                    break;
                }
                _ => i += 1,
            }
        }
    }
    Ok(items)
}

/// Scope a stylesheet to `class` (the bare class name, no leading dot).
/// Copy the whole CHARACTER at byte `at` into `out`, and answer the next character's byte index.
///
/// ★ THIS SCANNER USED TO PUSH `b[i] as char`, WHICH IS LATIN-1. Every byte of a multi-byte
/// character became a `char` of its own, so `.café` came out `.cafÃ©` — VALID CSS that matches
/// nothing. The stylesheet loads, the rule is emitted, the page renders unstyled, and nothing
/// anywhere is an error; the class in the MARKUP is untouched, because markup is copied rather
/// than scanned, so the two halves of one name silently disagree.
///
/// The structure this scanner looks for — `,` `(` `)` `[` `]` `"` `'` — is all ASCII, so it can
/// keep indexing bytes; only the ACCUMULATION had to learn that a character is not a byte.
fn push_char_at(src: &str, at: usize, out: &mut String) -> usize {
    match src[at..].chars().next() {
        Some(c) => {
            out.push(c);
            at + c.len_utf8()
        }
        None => at + 1,
    }
}

pub fn scope(css: &str, class: &str) -> Result<String, CssError> {
    let items = parse_items(css)?;
    Ok(emit_items(&items, class)?.join(" "))
}

fn emit_items(items: &[CssItem], class: &str) -> Result<Vec<String>, CssError> {
    let mut out = Vec::new();
    for item in items {
        match item {
            CssItem::Rule { prelude, body } => {
                let selectors = split_top_level(prelude, ',');
                let scoped: Vec<String> = selectors
                    .iter()
                    .map(|s| scope_selector(s.trim(), class))
                    .collect();
                out.push(format!("{} {{{}}}", scoped.join(", "), body));
            }
            CssItem::AtRule {
                name,
                prelude,
                body: None,
                ..
            } => {
                out.push(if prelude.is_empty() {
                    format!("@{name};")
                } else {
                    format!("@{name} {prelude};")
                });
            }
            CssItem::AtRule {
                name,
                prelude,
                body: Some(body),
                body_at,
            } => {
                let head = if prelude.is_empty() {
                    format!("@{name}")
                } else {
                    format!("@{name} {prelude}")
                };
                if GROUP_AT_RULES.contains(&name.to_ascii_lowercase().as_str()) {
                    let inner = emit_items(&parse_items_at(body, *body_at)?, class)?;
                    out.push(format!("{head} {{ {} }}", inner.join(" ")));
                } else {
                    out.push(format!("{head} {{{}}}", body.replace(BLANK, "")));
                }
            }
        }
    }
    Ok(out)
}

/// Scope one complex selector: stamp `.class` on the last non-`:global` compound and unwrap
/// every `:global(…)`.
pub fn scope_selector(selector: &str, class: &str) -> String {
    let parts = split_compounds(selector);
    // Index of the compound to stamp: the last one that is not `:global(...)`.
    let subject = parts.iter().rposition(|(compound, _)| !is_global(compound));
    let mut out = String::new();
    for (idx, (compound, combinator)) in parts.iter().enumerate() {
        let text = if is_global(compound) {
            unwrap_global(compound)
        } else if Some(idx) == subject {
            stamp(compound, class)
        } else {
            compound.clone()
        };
        out.push_str(&text);
        if let Some(c) = combinator {
            if c.trim().is_empty() {
                out.push(' ');
            } else {
                out.push(' ');
                out.push_str(c.trim());
                out.push(' ');
            }
        }
    }
    out
}

fn is_global(compound: &str) -> bool {
    compound.starts_with(":global(")
}

fn unwrap_global(compound: &str) -> String {
    let inner = &compound[":global(".len()..];
    match inner.rfind(')') {
        Some(k) => format!("{}{}", &inner[..k], &inner[k + 1..]),
        None => inner.to_string(),
    }
}

/// Insert `.class` into a compound before its first pseudo (`:`/`::`) at the top level.
fn stamp(compound: &str, class: &str) -> String {
    if compound == "*" {
        return format!(".{class}");
    }
    let b = compound.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b':' if depth == 0 => {
                let (head, tail) = compound.split_at(i);
                return format!("{head}.{class}{tail}");
            }
            _ => {}
        }
        i += 1;
    }
    format!("{compound}.{class}")
}

/// Split a complex selector into `(compound, combinator-after)` pairs. Combinators are
/// whitespace (descendant), `>`, `+`, `~`; parens, brackets and strings are opaque.
fn split_compounds(selector: &str) -> Vec<(String, Option<String>)> {
    let b = selector.as_bytes();
    let mut parts: Vec<(String, Option<String>)> = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if depth == 0
            && (c == b' ' || c == b'\t' || c == b'\n' || c == b'>' || c == b'+' || c == b'~')
        {
            // Gather the whole combinator run.
            let start = i;
            while i < b.len()
                && (b[i] == b' '
                    || b[i] == b'\t'
                    || b[i] == b'\n'
                    || b[i] == b'>'
                    || b[i] == b'+'
                    || b[i] == b'~')
            {
                i += 1;
            }
            let comb = selector[start..i].to_string();
            if current.is_empty() {
                // Leading whitespace: ignore.
                continue;
            }
            if i >= b.len() {
                // Trailing whitespace: ignore.
                break;
            }
            parts.push((std::mem::take(&mut current), Some(comb)));
            continue;
        }
        match c {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'"' | b'\'' => {
                let q = c;
                current.push(c as char);
                i += 1;
                while i < b.len() && b[i] != q {
                    i = push_char_at(selector, i, &mut current);
                }
            }
            _ => {}
        }
        if i < b.len() {
            i = push_char_at(selector, i, &mut current);
        } else {
            i += 1;
        }
    }
    if !current.is_empty() {
        parts.push((current, None));
    }
    parts
}

/// Split on a delimiter at paren/bracket/string depth zero.
fn split_top_level(s: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '(' | '[' => {
                depth += 1;
                current.push(c);
            }
            ')' | ']' => {
                depth -= 1;
                current.push(c);
            }
            '"' | '\'' => {
                current.push(c);
                for n in chars.by_ref() {
                    current.push(n);
                    if n == c {
                        break;
                    }
                }
            }
            _ if c == delim && depth == 0 => out.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() || out.is_empty() {
        out.push(current);
    }
    out.into_iter().filter(|s| !s.trim().is_empty()).collect()
}

fn split_at_name(rest: &str) -> (String, String) {
    let rest = rest.trim();
    let end = rest
        .find(|c: char| c.is_ascii_whitespace() || c == '(')
        .unwrap_or(rest.len());
    (rest[..end].to_string(), rest[end..].trim().to_string())
}

/// The byte a blanked comment is filled with. It is not whitespace and not structure, so every
/// scan passes over it exactly as it passed over the comment, and [`unblank`] removes it from what
/// is emitted — the output is what cutting the comment out would have produced.
const BLANK: char = '\u{0}';

/// The stylesheet with every comment's bytes replaced by [`BLANK`], one for one, so an offset in
/// the result is the same offset in the source. An unterminated comment runs to the end.
fn blank_comments(css: &str) -> Result<String, CssError> {
    let mut out = String::with_capacity(css.len());
    let b = css.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if css[i..].starts_with("/*") {
            let end = css[i + 2..].find("*/").map_or(b.len(), |k| i + 2 + k + 2);
            out.extend(std::iter::repeat(BLANK).take(end - i));
            i = end;
            continue;
        }
        if b[i] == b'"' || b[i] == b'\'' {
            let end = skip_string(css, i)?;
            out.push_str(&css[i..end]);
            i = end;
            continue;
        }
        let ch = css[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    Ok(out)
}

/// `s` without the whitespace and blanked comments around it.
fn trim_blank(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == BLANK)
}

/// An item with the blanked comments taken out of its text.
fn unblank(item: CssItem) -> CssItem {
    let clean = |s: String| {
        if s.contains(BLANK) {
            s.replace(BLANK, "")
        } else {
            s
        }
    };
    match item {
        CssItem::Rule { prelude, body } => CssItem::Rule {
            prelude: clean(prelude).trim().to_string(),
            body: clean(body),
        },
        CssItem::AtRule {
            name,
            prelude,
            body,
            body_at,
        } => CssItem::AtRule {
            name: clean(name),
            prelude: clean(prelude).trim().to_string(),
            body,
            body_at,
        },
    }
}

/// Skip the string opening at `at`; returns the index past its closing quote.
fn skip_string(s: &str, at: usize) -> Result<usize, CssError> {
    let b = s.as_bytes();
    let q = b[at];
    let mut i = at + 1;
    while i < b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == q {
            return Ok(i + 1);
        }
        if b[i] == b'\n' {
            break;
        }
        i += 1;
    }
    Err(CssError {
        kind: CssErrorKind::UnterminatedString,
        at,
    })
}

/// Index of the `}` matching the `{` at `open` (strings and parens opaque).
fn matching_brace(s: &str, open: usize) -> Result<usize, CssError> {
    let b = s.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                i = skip_string(s, i)?;
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    Err(CssError {
        kind: CssErrorKind::UnbalancedBraces,
        at: open,
    })
}
