//! The Samba book's experiment ledger, `benchmarks/silicon/samba_experiments.json` (section 9.2 of
//! `benchmarks/PREPARE_FOR_SAMBA.md`): every experiment the climb runs, kept, rejected or of no effect, so that no dead
//! end is walked twice.
//!
//! A row comes from an `EXPERIMENT SAMBA` line on the lanes board, which it keeps verbatim, or was migrated from an
//! earlier campaign's record in the tree, which its log points at (`<file>#<JSON pointer>`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    pub schema: u32,
    pub rows: Vec<Row>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    Kept,
    Rejected,
    /// Measured within the noise: recorded, and the change not taken.
    NoEffect,
}

impl Verdict {
    fn parse(s: &str) -> Option<Verdict> {
        match s {
            "kept" => Some(Verdict::Kept),
            "rejected" => Some(Verdict::Rejected),
            "no-effect" => Some(Verdict::NoEffect),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub id: String,
    /// The board ticket the experiment served; none for a row migrated from before the board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    pub date: String,
    /// What it ran on: a chip, a simulator, or a host.
    pub on: String,
    pub hypothesis: String,
    pub change: String,
    pub delta: String,
    pub verdict: Verdict,
    pub log: String,
    /// What the result teaches the next attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lesson: Option<String>,
    /// The lanes-board line the row was written from, verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
}

/// One `EXPERIMENT SAMBA` line, read.
#[derive(Debug, PartialEq, Eq)]
pub struct Line {
    pub ticket: String,
    pub on: String,
    pub hypothesis: String,
    pub change: String,
    pub delta: String,
    pub verdict: Verdict,
    pub log: String,
}

const FIELDS: [&str; 6] = ["on:", "hypothesis:", "change:", "delta:", "verdict:", "log:"];

/// `EXPERIMENT SAMBA <TICKET> on: <chip> hypothesis: <text> change: <text> delta: <measured> verdict:
/// <kept|rejected|no-effect> log: <path>`, each field present once and in that order.
pub fn parse_line(line: &str) -> Result<Line, String> {
    let rest = line.trim().strip_prefix("EXPERIMENT SAMBA ").ok_or_else(|| format!("not an experiment line: {line}"))?;
    let (ticket, mut rest) = rest.split_once(' ').ok_or_else(|| format!("no fields: {line}"))?;
    let mut values = Vec::new();
    for (i, field) in FIELDS.iter().enumerate() {
        let body = rest.strip_prefix(field).ok_or_else(|| format!("`{field}` expected at `{rest}`"))?;
        let (value, next) = match FIELDS.get(i + 1) {
            Some(next) => {
                let at = body.find(&format!(" {next} ")).ok_or_else(|| format!("no `{next}` after `{field}`: {line}"))?;
                (&body[..at], &body[at + 1..])
            }
            None => (body, ""),
        };
        let value = value.trim();
        if value.is_empty() {
            return Err(format!("`{field}` is empty: {line}"));
        }
        values.push(value.to_string());
        rest = next;
    }
    let verdict = Verdict::parse(&values[4]).ok_or_else(|| format!("verdict `{}` is not kept, rejected or no-effect", values[4]))?;
    Ok(Line {
        ticket: ticket.to_string(),
        on: values[0].clone(),
        hypothesis: values[1].clone(),
        change: values[2].clone(),
        delta: values[3].clone(),
        verdict,
        log: values[5].clone(),
    })
}

/// Every experiment line of a lanes board.
pub fn board_lines(board: &str) -> Vec<&str> {
    board.lines().map(str::trim).filter(|l| l.starts_with("EXPERIMENT SAMBA ")).collect()
}

/// The row a line becomes, numbered `id` and dated `date`.
pub fn row_of(line: &str, id: &str, date: &str) -> Result<Row, String> {
    let l = parse_line(line)?;
    Ok(Row {
        id: id.to_string(),
        ticket: Some(l.ticket),
        date: date.to_string(),
        on: l.on,
        hypothesis: l.hypothesis,
        change: l.change,
        delta: l.delta,
        verdict: l.verdict,
        log: l.log,
        lesson: None,
        line: Some(line.trim().to_string()),
    })
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b[4] == b'-' && b[7] == b'-' && s.chars().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// Every way `ledger` breaks the rules, one sentence each. `board` is the set of ticket ids; `repo` is where a log in
/// the tree is looked up.
pub fn violations(ledger: &Ledger, board: &BTreeSet<String>, repo: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if ledger.schema != 1 {
        out.push(format!("schema {} (this reader knows schema 1)", ledger.schema));
    }
    let mut ids = BTreeSet::new();
    let mut lines = BTreeSet::new();
    for (i, row) in ledger.rows.iter().enumerate() {
        let at = format!("row {i} ({})", row.id);
        let want = format!("X-{:04}", i + 1);
        if row.id != want {
            out.push(format!("{at}: rows are numbered in order; this one is {want}"));
        }
        if !ids.insert(row.id.clone()) {
            out.push(format!("{at}: the id is used twice"));
        }
        if !is_date(&row.date) {
            out.push(format!("{at}: the date `{}` is not YYYY-MM-DD", row.date));
        }
        for (name, text) in [("on", &row.on), ("hypothesis", &row.hypothesis), ("change", &row.change), ("delta", &row.delta)] {
            if text.trim().is_empty() {
                out.push(format!("{at}: no {name}"));
            }
        }
        if let Some(t) = &row.ticket {
            if !board.contains(t) {
                out.push(format!("{at}: ticket `{t}` is not on the board"));
            }
        }
        if row.log.trim().is_empty() {
            out.push(format!("{at}: a row names its log"));
        } else if !(row.log.starts_with("logs/") || row.log.starts_with("benchmarks/")) {
            out.push(format!("{at}: the log `{}` is not under logs/ or benchmarks/", row.log));
        } else if let Some((file, pointer)) = row.log.split_once('#') {
            match std::fs::read_to_string(repo.join(file)) {
                Err(e) => out.push(format!("{at}: the record `{file}`: {e}")),
                Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
                    Err(e) => out.push(format!("{at}: the record `{file}` is not JSON: {e}")),
                    Ok(v) => {
                        if v.pointer(pointer).is_none() {
                            out.push(format!("{at}: `{pointer}` names nothing in `{file}`"));
                        }
                    }
                },
            }
        } else if row.log.starts_with("benchmarks/") && !repo.join(&row.log).exists() {
            out.push(format!("{at}: the log `{}` does not exist", row.log));
        }
        match &row.line {
            Some(line) => {
                if !lines.insert(line.clone()) {
                    out.push(format!("{at}: the line is written twice"));
                }
                match row_of(line, &row.id, &row.date) {
                    Ok(from_line) => {
                        let mut same = row.clone();
                        same.lesson = None;
                        if same != from_line {
                            out.push(format!("{at}: the row differs from its line"));
                        }
                    }
                    Err(e) => out.push(format!("{at}: {e}")),
                }
            }
            None => {
                if row.ticket.is_some() {
                    out.push(format!("{at}: a row of a board ticket comes from its line"));
                }
                if !row.log.contains('#') {
                    out.push(format!("{at}: a migrated row points at its record (`<file>#<JSON pointer>`)"));
                }
            }
        }
    }
    out
}
