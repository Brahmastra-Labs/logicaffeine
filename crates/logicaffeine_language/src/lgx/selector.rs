//! The CSS selector subset LGX understands — parsed ONCE, here.
//!
//! Two very different things ask "what does this selector name": the runtime engine that
//! answers `the count of ".row"` against a live page, and the static check that refuses
//! `When "#draft" is clicked:` when no view renders a `#draft`. If those two disagreed, a page
//! would pass its checks and then fail to find its own elements. They cannot disagree, because
//! they read the same parse.
//!
//! The subset is what a page test and a page handler actually need: a descendant chain of
//! compounds, each an optional tag plus any number of `#id`, `.class`, and `[attr]` /
//! `[attr=value]` qualifiers. No sibling combinators, no pseudo-classes — those belong to
//! stylesheets, and a stylesheet says them in `Style:` where real CSS is read verbatim.

/// One compound selector: everything that must hold of a SINGLE element (`li#a.done[x=1]`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Compound {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    /// `[name]` (presence) or `[name=value]` (equality); quotes around the value optional.
    pub attrs: Vec<(String, Option<String>)>,
}

/// A descendant chain: `.card li.done` is "a `li.done` anywhere inside a `.card`".
pub type Selector = Vec<Compound>;

/// The CSS syntax this engine deliberately does not read, and what to write instead.
///
/// ★ A CATALOGUE, NOT A LIST OF CHECKS. Each entry carries its own sentence, so adding a form the
/// engine cannot evaluate means writing the explanation the author will read — there is nowhere to
/// add one silently.
///
/// ★ AND EVERY ONE OF THESE USED TO PARSE AS A NAME. `compound()` takes the leading run up to `#`,
/// `.` or `[` as the TAG, so `>` became a tag named `>` and `li:first-child` a tag named
/// `li:first-child`. Both matched nothing — the same answer a correct selector gives for a page
/// that genuinely has none — so `Expect the count of "ul > li" to be 0.` passed, and would have
/// passed with five hundred items on the page.
pub const UNSUPPORTED_SYNTAX: &[(char, &str)] = &[
    ('>', "uses the child combinator `>`, which this engine does not read. It names elements by tag, `#id`, `.class`, `[attr]` and DESCENDANT nesting, so write `ul li` for a `li` anywhere inside a `ul`. Full CSS, combinators included, belongs in `Style:`, where it is read verbatim"),
    ('+', "uses the adjacent-sibling combinator `+`, which this engine does not read. It names elements by tag, `#id`, `.class`, `[attr]` and descendant nesting; give the element you mean a class and name that. Full CSS belongs in `Style:`, where it is read verbatim"),
    ('~', "uses the general-sibling combinator `~`, which this engine does not read. It names elements by tag, `#id`, `.class`, `[attr]` and descendant nesting; give the element you mean a class and name that. Full CSS belongs in `Style:`, where it is read verbatim"),
    (',', "is TWO selectors joined by a comma, and this engine answers about one. Ask each question on its own, or give both elements a shared class and name that"),
    (':', "uses a pseudo-class, which this engine does not read. A pseudo-class is about how a page is DRAWN, so it belongs in `Style:`, where real CSS is read verbatim; to pick one element out of several here, give it an id or a class"),
    ('*', "uses the universal selector `*`, which this engine does not read. Name the tag, id or class you actually mean"),
];

/// Why `selector` is outside the subset, or `None` when it is inside it.
///
/// Scanned OUTSIDE `[...]`, because an attribute VALUE may legitimately hold any of these —
/// `[href="https://example.com"]` carries a colon and a slash and is perfectly ordinary.
pub fn unsupported(selector: &str) -> Option<String> {
    let mut in_brackets = false;
    for c in selector.chars() {
        match c {
            '[' => in_brackets = true,
            ']' => in_brackets = false,
            _ if in_brackets => {}
            _ => {
                if let Some((_, why)) = UNSUPPORTED_SYNTAX.iter().find(|(k, _)| *k == c) {
                    return Some(format!("`{selector}` {why}"));
                }
            }
        }
    }
    None
}

/// Parse a selector, or `None` when it is empty, malformed, or outside the subset.
pub fn parse(selector: &str) -> Option<Selector> {
    if unsupported(selector).is_some() {
        return None;
    }
    let mut parts = Vec::new();
    for token in selector.split_whitespace() {
        parts.push(compound(token)?);
    }
    (!parts.is_empty()).then_some(parts)
}

/// Parse one compound.
pub fn compound(token: &str) -> Option<Compound> {
    let mut c = Compound::default();
    let chars: Vec<char> = token.chars().collect();
    let mut i = 0;
    // A leading bare word is the tag name.
    while i < chars.len() && !matches!(chars[i], '#' | '.' | '[') {
        i += 1;
    }
    if i > 0 {
        c.tag = Some(chars[..i].iter().collect());
    }
    while i < chars.len() {
        match chars[i] {
            '#' | '.' => {
                let kind = chars[i];
                i += 1;
                let start = i;
                while i < chars.len() && !matches!(chars[i], '#' | '.' | '[') {
                    i += 1;
                }
                if i == start {
                    return None;
                }
                let name: String = chars[start..i].iter().collect();
                match kind {
                    '#' => c.id = Some(name),
                    _ => c.classes.push(name),
                }
            }
            '[' => {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != ']' {
                    i += 1;
                }
                if i >= chars.len() {
                    return None;
                }
                let inner: String = chars[start..i].iter().collect();
                i += 1;
                match inner.split_once('=') {
                    Some((name, value)) => {
                        let value = value.trim().trim_matches(|q| q == '"' || q == '\'');
                        c.attrs
                            .push((name.trim().to_string(), Some(value.to_string())));
                    }
                    None => c.attrs.push((inner.trim().to_string(), None)),
                }
            }
            _ => return None,
        }
    }
    (c.tag.is_some() || c.id.is_some() || !c.classes.is_empty() || !c.attrs.is_empty()).then_some(c)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_language/lgx/selector/tests.rs"]
mod tests;
