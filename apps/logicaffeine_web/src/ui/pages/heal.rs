//! LLM Heal: the fairy that mends a reader's English (work/LLM_HEAL.md, DAT-108).
//!
//! A person types English straight into the Studio's editor, typos and loose phrasing and all,
//! and presses Heal. The text goes to the heal worker (`services/heal`), which asks a model for the
//! same text as clear, idiomatic LOGOS or logic sentences; the fairy flies to the lines the mend
//! touched and a card shows both versions with Keep and Undo. Nothing lands in the editor until
//! Keep is pressed. This module is the pure core: the wire shapes, the line diff, the flight's
//! state machine, the worker's address, the pixelation schedule and the dock bar's words. The
//! workbench draws it.

use std::ops::Range;

use serde::{Deserialize, Serialize};

/// Where a heal is sent: `LOGOS_HEAL_URL` at build time (nas2's dev build points at a `wrangler
/// dev` of the worker), else production.
pub const ENDPOINT: &str = match option_env!("LOGOS_HEAL_URL") {
    Some(url) => url,
    None => "https://api.logicaffeine.com/heal",
};

/// The fairy, served from the site (the L of Z, her quest form).
pub const FAIRY: &str = "/assets/heal-fairy.png";

/// What the dock bar says when Heal is pressed over an empty editor.
pub const EMPTY_EDITOR: &str = "Nothing to heal yet. Write some English in the editor first.";

/// What the dock bar says when nothing has changed since the file opened or was last kept.
pub const NOTHING_NEW: &str = "Nothing new to heal. Change some lines first, then press Heal.";

/// The two Studio modes that heal, as the worker names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Code,
    Logic,
}

/// The body the worker reads: `{ "mode": "code" | "logic", "text": "...", "fragment": true }`, the
/// text as written. `fragment` says the text is the lines a person just wrote inside a program that
/// is already right, so the model mends those lines only and answers with them alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub mode: Mode,
    pub text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fragment: bool,
}

/// The worker's answer: `{ ok: true, healed, model }` or `{ ok: false, error }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub healed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The healed text from the worker's status and body, or the sentence the card shows instead.
pub fn outcome(status: u16, body: &str) -> Result<String, String> {
    let answer: Answer = match serde_json::from_str(body) {
        Ok(answer) => answer,
        Err(_) if status == 429 => return Err(reason("rate_limited")),
        Err(_) => return Err(reason("not_json")),
    };
    match (status, answer.ok, answer.healed) {
        (200, true, Some(healed)) if !healed.trim().is_empty() => Ok(healed),
        (200, true, _) => Err(reason("empty_answer")),
        (429, _, _) => Err(reason("rate_limited")),
        _ => Err(reason(answer.error.as_deref().unwrap_or("failed"))),
    }
}

/// The worker's named error as the sentence the card shows.
pub fn reason(error: &str) -> String {
    match error {
        "text_too_long" => "That is more than one heal takes (8 KB). Heal a part of it.".to_string(),
        "text_missing" => EMPTY_EDITOR.to_string(),
        "rate_limited" => "Too many heals in a minute. Try again shortly.".to_string(),
        "upstream_failed" => "The model did not answer. Try again.".to_string(),
        "empty_answer" | "no_text" => "The model answered with no text. Try again.".to_string(),
        "origin_not_allowed" => "This page may not heal from here.".to_string(),
        "unreachable" => "The heal service did not answer. Check the connection and try again.".to_string(),
        other => format!("Heal failed ({other})."),
    }
}

/// The line ranges of `after` the mend touched, by a line diff: identical texts give none, a
/// one-word fix gives that line alone, lines added give their run, and lines deleted give an
/// empty range where they were. Spaces at a line's end do not count as a change.
pub fn changed_lines(before: &str, after: &str) -> Vec<Range<usize>> {
    line_diff(before, after).after
}

/// What a line diff touched on each side: the ranges of `before` lines the mend replaced or
/// removed, and of `after` lines it wrote or added, by the longest common subsequence of lines
/// (spaces at a line's end ignored). The card marks both sides from this.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineDiff {
    pub before: Vec<Range<usize>>,
    pub after: Vec<Range<usize>>,
}

pub fn line_diff(before: &str, after: &str) -> LineDiff {
    let a: Vec<&str> = before.lines().map(str::trim_end).collect();
    let b: Vec<&str> = after.lines().map(str::trim_end).collect();
    // The longest common subsequence of lines, by the usual table.
    let (n, m) = (a.len(), b.len());
    let mut table = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if a[i] == b[j] { table[i + 1][j + 1] + 1 } else { table[i + 1][j].max(table[i][j + 1]) };
        }
    }
    fn push(ranges: &mut Vec<Range<usize>>, range: Range<usize>) {
        match ranges.last_mut() {
            Some(last) if last.end == range.start => last.end = range.end,
            _ => ranges.push(range),
        }
    }
    let mut diff = LineDiff::default();
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            i += 1;
            j += 1;
        } else if j < m && (i >= n || table[i][j + 1] >= table[i + 1][j]) {
            push(&mut diff.after, j..j + 1);
            push(&mut diff.before, i..i);
            j += 1;
        } else {
            push(&mut diff.after, j..j);
            push(&mut diff.before, i..i + 1);
            i += 1;
        }
    }
    diff
}

/// The regions of `current` a person wrote since `baseline` (the text the file opened with, or was
/// last kept at): the runs of lines that differ, by the line diff, neighbours within one line
/// merged into one region, and nothing for lines only removed. Only these go to the worker, so a
/// heal touches what the person typed and never the code that was already there (the user,
/// 2026-11-02).
pub fn edited_regions(baseline: &str, current: &str) -> Vec<Range<usize>> {
    let mut regions: Vec<Range<usize>> = Vec::new();
    for range in line_diff(baseline, current).after {
        if range.is_empty() {
            continue;
        }
        match regions.last_mut() {
            Some(last) if range.start <= last.end + 1 => last.end = last.end.max(range.end),
            _ => regions.push(range),
        }
    }
    regions
}

/// The lines of `text` in `region`, as one text to send.
pub fn region_text(text: &str, region: &Range<usize>) -> String {
    text.lines().skip(region.start).take(region.len()).collect::<Vec<_>>().join("\n")
}

/// The mended lines at the indentation the region had: a model that drops a fragment's leading
/// spaces (the first line of an indented block comes back at the margin) gets them put back, on
/// every mended line shallower than the region's first line, so the splice keeps the block.
pub fn reindent(region: &str, mended: &str) -> String {
    let indent_of = |line: &str| line.len() - line.trim_start().len();
    let base = region.lines().next().map_or(0, indent_of);
    let prefix = " ".repeat(base);
    mended
        .lines()
        .map(|line| {
            if line.trim().is_empty() || indent_of(line) >= base {
                line.to_string()
            } else {
                format!("{prefix}{}", line.trim_start())
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `text` with each region's lines replaced by the mended lines it got back, last region first so
/// earlier line numbers stay true; the rest of the text is untouched, byte for byte.
pub fn splice(text: &str, regions: &[Range<usize>], mended: &[String]) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for (region, replacement) in regions.iter().zip(mended).rev() {
        let new_lines: Vec<String> = replacement.lines().map(str::to_string).collect();
        lines.splice(region.clone(), new_lines);
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Whether two texts say the same thing line for line, spaces at a line's end and blank lines at
/// the end aside: a mend that only moved whitespace is no mend.
pub fn same_text(before: &str, after: &str) -> bool {
    let norm = |t: &str| t.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim_end().to_string();
    norm(before) == norm(after)
}

/// A line as pieces, each marked whether it differs from the other line: the common prefix and
/// suffix of words stay plain, what lies between is the mend. For the card to show the word that
/// changed, not only the line.
pub fn word_marks<'a>(line: &'a str, other: &str) -> Vec<(&'a str, bool)> {
    let a: Vec<&str> = line.split_inclusive(' ').collect();
    let b: Vec<&str> = other.split_inclusive(' ').collect();
    let prefix = a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(x, y)| x == y).count();
    let mut out = Vec::new();
    let mut piece = |range: Range<usize>, marked: bool| {
        if range.start < range.end {
            let start = a[..range.start].iter().map(|s| s.len()).sum::<usize>();
            let end = a[..range.end].iter().map(|s| s.len()).sum::<usize>();
            out.push((&line[start..end], marked));
        }
    };
    piece(0..prefix, false);
    piece(prefix..a.len() - suffix, true);
    piece(a.len() - suffix..a.len(), false);
    out
}

/// Where the fairy is and what the card shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Flight {
    /// On her button.
    Idle,
    /// On her way to line `to` while the worker thinks.
    Flying { to: usize },
    /// At the line, mending.
    Mending,
    /// The mend is back: both texts, and the lines of `after` it touched (`diff` has both sides).
    Review { before: String, after: String, lines: Vec<Range<usize>>, diff: LineDiff },
    /// No mend: the reason, in the card's place.
    Failed(String),
}

impl Flight {
    /// Whether Heal is busy (the button waits).
    pub fn busy(&self) -> bool {
        matches!(self, Flight::Flying { .. } | Flight::Mending)
    }

    /// Heal pressed over `text`: the flight begins, or the dock says the editor is empty.
    pub fn begin(text: &str) -> Flight {
        if text.trim().is_empty() {
            Flight::Failed(EMPTY_EDITOR.to_string())
        } else {
            Flight::Flying { to: 0 }
        }
    }

    /// The fairy reached the line.
    pub fn arrived(self) -> Flight {
        match self {
            Flight::Flying { .. } => Flight::Mending,
            other => other,
        }
    }

    /// The worker answered: a review of both texts, or the reason there is none. A mend that
    /// changed nothing is reported as such rather than reviewed.
    pub fn answered(before: String, outcome: Result<String, String>) -> Flight {
        match outcome {
            Ok(after) if same_text(&before, &after) => Flight::Failed("Nothing to mend. The text already reads well.".to_string()),
            Ok(after) => {
                let diff = line_diff(&before, &after);
                let lines = diff.after.clone();
                Flight::Review { before, after, lines, diff }
            }
            Err(reason) => Flight::Failed(reason),
        }
    }

    /// Keep: the healed text to write into the editor, and the fairy goes home.
    pub fn keep(self) -> (Flight, Option<String>) {
        match self {
            Flight::Review { after, .. } => (Flight::Idle, Some(after)),
            _ => (Flight::Idle, None),
        }
    }

    /// Undo (or closing a failure): the editor is untouched, and the fairy goes home.
    pub fn undo(self) -> Flight {
        Flight::Idle
    }

    /// The line the fairy stands at: the first the mend touched, else where she was sent.
    pub fn line(&self) -> usize {
        match self {
            Flight::Flying { to } => *to,
            Flight::Review { lines, .. } => lines.first().map_or(0, |r| r.start),
            _ => 0,
        }
    }
}

/// What the dock bar says while a heal is under way; nothing when the fairy is home.
pub fn narration(flight: &Flight) -> Option<String> {
    match flight {
        Flight::Idle => None,
        Flight::Flying { .. } | Flight::Mending => Some("Healing\u{2026}".to_string()),
        Flight::Review { lines, .. } => {
            let n: usize = lines.iter().map(|r| r.len().max(1)).sum();
            Some(format!("Healed {n} line{}. Keep or Undo.", if n == 1 { "" } else { "s" }))
        }
        Flight::Failed(reason) => Some(reason.clone()),
    }
}

/// The order in which `len` characters dissolve, deterministic from `seed`, so the two halves of
/// the animation mirror: the new text forms in the reverse of this order.
pub fn pixel_steps(len: usize, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).max(1);
    for i in (1..len).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = (state % (i as u64 + 1)) as usize;
        order.swap(i, j);
    }
    order
}

/// POST the text to the worker. On the web, the answer as [`outcome`] reads it; on the server
/// there is nothing to send to.
pub async fn send(mode: Mode, text: &str, fragment: bool) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let body = Request { mode, text: text.to_string(), fragment };
        let Ok(request) = gloo_net::http::Request::post(ENDPOINT).json(&body) else {
            return Err(reason("unreachable"));
        };
        match request.send().await {
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                outcome(status, &body)
            }
            Err(_) => Err(reason("unreachable")),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (mode, text, fragment);
        Err(reason("unreachable"))
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/heal/tests.rs"]
mod tests;
