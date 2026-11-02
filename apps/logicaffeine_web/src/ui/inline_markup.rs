//! Inline markup for the site's prose — curriculum, Syntax Guide and news: `**strong**`,
//! `*emphasis*`, `` `code` `` and `[links](/path)`.
//!
//! Text is HTML-escaped before any markup is recognised, so lesson prose can say `x < y` or
//! `A & B` and never inject markup. Stars follow CommonMark's flanking rules: a run of `*`
//! opens emphasis only at the start of a word and closes it only at the end of one, so the
//! star test's literal stars (`A* is B*`, `S*-D`) and arithmetic (`2 * 3`) stay text, and an
//! unmatched marker is printed as written instead of swallowing the rest of the paragraph.

/// Render one paragraph of the site's prose to HTML.
pub fn inline_markup_html(text: &str) -> String {
    emphasis_html(text)
}

/// The end of the backtick run of exactly `ticks` that closes a code span opened just before
/// `chars`, as an index into `chars`.
fn closing_ticks(chars: &[char], ticks: usize) -> Option<usize> {
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            let run = chars[i..].iter().take_while(|&&c| c == '`').count();
            if run == ticks {
                return Some(i);
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

enum Piece {
    Text(String),
    Stars(Stars),
    /// `[text](url)`: one unit to the emphasis around it, its text marked up on its own.
    Link { text: String, url: String },
    /// A code span: one unit to the emphasis around it, its text shown as written.
    Code(String),
}

struct Stars {
    left: usize,
    original: usize,
    can_open: bool,
    can_close: bool,
    opens: Vec<&'static str>,
    closes: Vec<&'static str>,
}

pub(crate) fn emphasis_html(text: &str) -> String {
    let mut pieces = split_star_runs(text);
    match_emphasis(&mut pieces);

    let mut out = String::with_capacity(text.len());
    for piece in &pieces {
        match piece {
            Piece::Text(t) => out.push_str(&escape(t)),
            Piece::Code(code) => {
                out.push_str("<code>");
                out.push_str(&escape(code.trim()));
                out.push_str("</code>");
            }
            Piece::Link { text, url } => {
                out.push_str("<a href=\"");
                out.push_str(&escape(url));
                out.push_str("\">");
                out.push_str(&emphasis_html(text));
                out.push_str("</a>");
            }
            Piece::Stars(s) => {
                for tag in &s.closes {
                    out.push_str("</");
                    out.push_str(tag);
                    out.push('>');
                }
                out.extend(std::iter::repeat('*').take(s.left));
                for tag in s.opens.iter().rev() {
                    out.push('<');
                    out.push_str(tag);
                    out.push('>');
                }
            }
        }
    }
    out
}

fn split_star_runs(text: &str) -> Vec<Piece> {
    let chars: Vec<char> = text.chars().collect();
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            let ticks = chars[i..].iter().take_while(|&&c| c == '`').count();
            match closing_ticks(&chars[i + ticks..], ticks) {
                Some(end) => {
                    if !literal.is_empty() {
                        pieces.push(Piece::Text(std::mem::take(&mut literal)));
                    }
                    pieces.push(Piece::Code(chars[i + ticks..i + ticks + end].iter().collect()));
                    i += ticks + end + ticks;
                }
                None => {
                    literal.extend(&chars[i..i + ticks]);
                    i += ticks;
                }
            }
            continue;
        }
        if chars[i] == '[' {
            if let Some((text, url, len)) = link_at(&chars[i..]) {
                if !literal.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut literal)));
                }
                pieces.push(Piece::Link { text, url });
                i += len;
                continue;
            }
        }
        if chars[i] != '*' {
            literal.push(chars[i]);
            i += 1;
            continue;
        }
        let run = chars[i..].iter().take_while(|&&c| c == '*').count();
        let before = if i == 0 { None } else { Some(chars[i - 1]) };
        let after = chars.get(i + run).copied();
        if !literal.is_empty() {
            pieces.push(Piece::Text(std::mem::take(&mut literal)));
        }
        pieces.push(Piece::Stars(Stars {
            left: run,
            original: run,
            can_open: left_flanking(before, after),
            can_close: right_flanking(before, after),
            opens: Vec::new(),
            closes: Vec::new(),
        }));
        i += run;
    }
    if !literal.is_empty() {
        pieces.push(Piece::Text(literal));
    }
    pieces
}

/// `[text](url)` at the start of `chars`: its text, its address and how many chars it spans. A
/// link needs text, and an address the site links to — the web, mail, or a path on the site;
/// anything else (a `javascript:` or `data:` URL, brackets with no address) stays text.
fn link_at(chars: &[char]) -> Option<(String, String, usize)> {
    let close = chars.iter().position(|&c| c == ']')?;
    let text: String = chars[1..close].iter().collect();
    if text.is_empty() || text.contains('[') || chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = close + 2 + chars[close + 2..].iter().position(|&c| c == ')')?;
    let url: String = chars[close + 2..end].iter().collect();
    let linkable = ["https://", "http://", "mailto:", "/", "#"].iter().any(|scheme| url.starts_with(scheme));
    (linkable && !url.chars().any(char::is_whitespace)).then_some((text, url, end + 1))
}

fn is_space(c: Option<char>) -> bool {
    c.map_or(true, char::is_whitespace)
}

fn is_punct(c: Option<char>) -> bool {
    c.is_some_and(|c| !c.is_alphanumeric() && !c.is_whitespace())
}

fn left_flanking(before: Option<char>, after: Option<char>) -> bool {
    !is_space(after) && (!is_punct(after) || is_space(before) || is_punct(before))
}

fn right_flanking(before: Option<char>, after: Option<char>) -> bool {
    !is_space(before) && (!is_punct(before) || is_space(after) || is_punct(after))
}

fn stars(pieces: &mut [Piece], i: usize) -> &mut Stars {
    match &mut pieces[i] {
        Piece::Stars(s) => s,
        Piece::Text(_) | Piece::Link { .. } | Piece::Code(_) => unreachable!("index points at a star run"),
    }
}

/// CommonMark's "process emphasis", for `*` only: each closer pairs with the nearest usable
/// opener before it; runs strictly between a matched pair can no longer pair with anything
/// outside it.
fn match_emphasis(pieces: &mut [Piece]) {
    let runs: Vec<usize> = pieces
        .iter()
        .enumerate()
        .filter_map(|(i, p)| matches!(p, Piece::Stars(_)).then_some(i))
        .collect();

    for (closer_pos, &closer) in runs.iter().enumerate() {
        loop {
            let (closer_left, closer_original, closer_can_open, closer_can_close) = {
                let c = stars(pieces, closer);
                (c.left, c.original, c.can_open, c.can_close)
            };
            if closer_left == 0 || !closer_can_close {
                break;
            }
            let opener = runs[..closer_pos].iter().rev().copied().find(|&o| {
                let s = stars(pieces, o);
                if s.left == 0 || !s.can_open {
                    return false;
                }
                let odd_match = (s.can_close || closer_can_open)
                    && (s.original + closer_original) % 3 == 0
                    && !(s.original % 3 == 0 && closer_original % 3 == 0);
                !odd_match
            });
            let Some(opener) = opener else { break };

            let width = if stars(pieces, opener).left >= 2 && closer_left >= 2 { 2 } else { 1 };
            let tag = if width == 2 { "strong" } else { "em" };
            {
                let o = stars(pieces, opener);
                o.left -= width;
                o.opens.push(tag);
            }
            {
                let c = stars(pieces, closer);
                c.left -= width;
                c.closes.push(tag);
            }
            for &between in runs.iter().filter(|&&r| r > opener && r < closer) {
                let s = stars(pieces, between);
                s.can_open = false;
                s.can_close = false;
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/inline_markup/tests.rs"]
mod tests;
