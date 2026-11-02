//! The Samba book's cell ledger, `benchmarks/silicon/samba_cells.json` (section 2.3 of
//! `benchmarks/PREPARE_FOR_SAMBA.md`), and the rules of section 2.4 that decide what a row may claim.
//!
//! The ledger is self-describing: its `measures` are its columns (each with a unit, the direction that is better, a
//! definition, and either the section 2.1 measure and the sources it comes from, or the exam regimes that define it),
//! and its `rows` are results. A row is one side of one cell: ours, or a published result under its citation key. The
//! book is read for everything a row names: the chips, bits and regimes of section 2.3, the measures of section 2.1,
//! the bibliography of section 12.3 and the board of section 11.1.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    pub schema: u32,
    pub approved: Option<Approval>,
    pub measures: Vec<Measure>,
    pub rows: Vec<Row>,
}

/// Tristen's approval of the schema, which comes before the first row.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    pub by: String,
    pub date: String,
    pub line: String,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Better {
    Higher,
    Lower,
    /// A descriptive figure (a FLOP count, a memory size) that no side wins on.
    None,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measure {
    pub id: String,
    pub unit: String,
    pub better: Better,
    pub definition: String,
    /// The bold name of the section 2.1 measure this column records.
    #[serde(default)]
    pub heading: Option<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    /// The section 2.3 regimes that define a column of the exam's own.
    #[serde(default)]
    pub regimes: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Provenance {
    Published,
    Simulated,
    Modeled,
    Measured,
    NotYet,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub cell: String,
    /// `ours`, or the bibliography key of the published result.
    pub side: String,
    pub provenance: Provenance,
    pub date: String,
    #[serde(default)]
    pub values: BTreeMap<String, f64>,
    #[serde(default)]
    pub runs: BTreeMap<String, Vec<f64>>,
    /// The protocol as the source states it: batch, input and output lengths, what the number counts.
    #[serde(default)]
    pub conditions: Option<String>,
    /// Where in the source the number is: a table, a figure, a page, a slide.
    #[serde(default)]
    pub locator: Option<String>,
    pub won: bool,
    #[serde(default)]
    pub quality: Option<Quality>,
    #[serde(default)]
    pub program: Option<Program>,
    #[serde(default)]
    pub bundle_digest: Option<String>,
    #[serde(default)]
    pub time_model: Option<TimeModel>,
    #[serde(default)]
    pub simulator: Option<Simulator>,
    #[serde(default)]
    pub boot: Option<Boot>,
    #[serde(default)]
    pub closers: Vec<String>,
    #[serde(default)]
    pub log: Option<String>,
}

/// Section 2.4's matched quality, against the f32 reference over the golden prompts.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quality {
    pub reference: String,
    pub prompts: u32,
    pub top1_agreement: f64,
    pub reference_top1_agreement: f64,
    pub kl: f64,
    pub kl_bound: f64,
    pub perplexity_delta: f64,
    pub perplexity_bound: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub spec_digest: String,
    pub cut: String,
    pub source_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeModel {
    pub version: String,
    pub calibration: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Simulator {
    pub name: String,
    pub version: String,
    /// The simulator's own stated tolerance, as a fraction of the value.
    pub tolerance: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Boot {
    pub id: String,
    pub host: String,
    pub load_seconds: f64,
}

/// What a row may name, read from the book.
pub struct Book {
    /// Chip name to what a win on it must be (`measured`, `simulated`, or `never`).
    pub chips: BTreeMap<String, String>,
    pub bits: BTreeSet<String>,
    /// Every regime, `SCALE-n` kept as written.
    pub regimes: BTreeSet<String>,
    /// Each section 2.1 measure's bold name and the keys its paragraph cites.
    pub measures: BTreeMap<String, BTreeSet<String>>,
    pub bibliography: BTreeSet<String>,
    pub board: BTreeSet<String>,
}

fn section<'a>(text: &'a str, start: &str) -> &'a str {
    let at = text.find(start).unwrap_or_else(|| panic!("the book has no `{start}`"));
    let rest = &text[at..];
    let end = rest[1..].find("\n### ").or_else(|| rest[1..].find("\n## ")).map(|i| i + 1).unwrap_or(rest.len());
    &rest[..end]
}

/// The rows of the markdown table in `text` whose header's first cell is `first`.
fn table<'a>(text: &'a str, first: &str) -> Vec<Vec<&'a str>> {
    let header = format!("| {first} |");
    let mut rows = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with(&header) {
            inside = true;
            continue;
        }
        if inside {
            if line.starts_with("|---") {
                continue;
            }
            if !line.starts_with('|') {
                break;
            }
            rows.push(line.trim_matches('|').split(" | ").map(str::trim).collect());
        }
    }
    rows
}

fn keys(text: &str) -> BTreeSet<String> {
    text.split('[')
        .skip(1)
        .filter_map(|s| s.split(']').next())
        .filter(|k| {
            k.len() >= 2
                && k.starts_with(|c: char| c.is_ascii_uppercase())
                && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        })
        .map(str::to_string)
        .collect()
}

impl Book {
    pub fn read(book: &str) -> Book {
        let cells = section(book, "### 2.3 Cells and regimes");
        let mut chips = BTreeMap::new();
        for row in table(cells, "Chip") {
            for chip in row[0].split(", ") {
                let win = row[2].split(':').next().unwrap().trim().to_string();
                chips.insert(chip.to_string(), win);
            }
        }
        let bits = table(cells, "Bits").iter().map(|r| r[0].to_string()).collect();
        let regimes = table(cells, "Regime").iter().flat_map(|r| r[0].split(", ").map(str::to_string)).collect();
        let mut measures = BTreeMap::new();
        let mut current: Option<String> = None;
        for line in section(book, "### 2.1 How SambaNova measures a compiler").lines() {
            if let Some(rest) = line.strip_prefix("- **") {
                let name = rest.split("**").next().unwrap().trim_end_matches('.').to_string();
                current = Some(name.clone());
                measures.insert(name, keys(line));
            } else if line.starts_with("  ") {
                if let Some(name) = &current {
                    measures.get_mut(name).unwrap().extend(keys(line));
                }
            } else {
                current = None;
            }
        }
        let bibliography = section(book, "### 12.3 Bibliography")
            .lines()
            .filter_map(|l| l.strip_prefix("- ["))
            .map(|l| l.split(']').next().unwrap().to_string())
            .collect();
        let board = section(book, "### 11.1 Board index")
            .lines()
            .filter_map(|l| l.strip_prefix("- [ ] **").or_else(|| l.strip_prefix("- [x] **")))
            .map(|l| l.split("**").next().unwrap().to_string())
            .collect();
        Book { chips, bits, regimes, measures, bibliography, board }
    }
}

/// A cell id read against the book: `C-<chip>-<model>-<regime>[-<bits>]`.
#[derive(Debug, PartialEq, Eq)]
pub struct Cell {
    pub chip: String,
    pub count: u32,
    pub model: String,
    pub regime: String,
    pub bits: Option<String>,
}

fn regime_matches(written: &str, regimes: &BTreeSet<String>) -> bool {
    if regimes.contains(written) {
        return true;
    }
    written
        .strip_prefix("SCALE-")
        .is_some_and(|n| regimes.contains("SCALE-n") && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && n != "0")
}

pub fn parse_cell(id: &str, book: &Book) -> Result<Cell, String> {
    let rest = id.strip_prefix("C-").ok_or_else(|| format!("`{id}` does not start with `C-`"))?;
    let mut parts: Vec<&str> = rest.split('-').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(format!("`{id}` has an empty part"));
    }
    let chip_part = parts.remove(0);
    let (chip, count) = match chip_part.rsplit_once('x') {
        Some((base, n)) if book.chips.contains_key(base) && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
            let count: u32 = n.parse().map_err(|_| format!("`{id}`: socket count {n}"))?;
            if count < 2 {
                return Err(format!("`{id}`: a count of {count} is written without one"));
            }
            (base.to_string(), count)
        }
        _ if book.chips.contains_key(chip_part) => (chip_part.to_string(), 1),
        _ => return Err(format!("`{id}`: `{chip_part}` is no chip of section 2.3")),
    };
    let bits = match parts.last() {
        Some(last) if book.bits.contains(*last) => {
            let b = last.to_string();
            parts.pop();
            Some(b)
        }
        _ => None,
    };
    let mut regime = None;
    for take in (1..=parts.len().saturating_sub(1).min(2)).rev() {
        let written = parts[parts.len() - take..].join("-");
        if regime_matches(&written, &book.regimes) {
            regime = Some(written);
            parts.truncate(parts.len() - take);
            break;
        }
    }
    let regime = regime.ok_or_else(|| format!("`{id}` names no regime of section 2.3"))?;
    if parts.is_empty() {
        return Err(format!("`{id}` names no model"));
    }
    let model = parts.join("-");
    if !model.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-' || c == '.') {
        return Err(format!("`{id}`: the model `{model}` is not a short name in capitals"));
    }
    Ok(Cell { chip, count, model, regime, bits })
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b[4] == b'-' && b[7] == b'-' && s.chars().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The published row a cell is barred by: the latest of its published rows, every one of them kept.
pub fn bar<'a>(ledger: &'a Ledger, cell: &str) -> Option<&'a Row> {
    ledger
        .rows
        .iter()
        .filter(|r| r.cell == cell && r.provenance == Provenance::Published)
        .max_by(|a, b| a.date.cmp(&b.date))
}

/// Whether `ours` is ahead of `theirs` on `measure` in its better direction, by more than `margin` (a fraction).
fn ahead(better: Better, ours: f64, theirs: f64, margin: f64) -> bool {
    match better {
        Better::Higher => ours > theirs * (1.0 + margin),
        Better::Lower => ours < theirs * (1.0 - margin),
        Better::None => false,
    }
}

/// Every way `ledger` breaks the rules, one sentence each.
pub fn violations(ledger: &Ledger, book: &Book) -> Vec<String> {
    let mut out = Vec::new();
    if ledger.schema != 1 {
        out.push(format!("schema {} (this reader knows schema 1)", ledger.schema));
    }
    if !ledger.rows.is_empty() && ledger.approved.is_none() {
        out.push(format!("{} rows before Tristen approved the schema", ledger.rows.len()));
    }
    if let Some(a) = &ledger.approved {
        if a.by != "Tristen" || !is_date(&a.date) || a.line.trim().is_empty() {
            out.push(format!("the approval {a:?} names Tristen, a date and his line"));
        }
    }

    let mut columns: BTreeMap<&str, &Measure> = BTreeMap::new();
    for m in &ledger.measures {
        if columns.insert(m.id.as_str(), m).is_some() {
            out.push(format!("measure `{}` is declared twice", m.id));
        }
        if m.unit.trim().is_empty() || m.definition.trim().is_empty() {
            out.push(format!("measure `{}` has no unit or no definition", m.id));
        }
        match &m.heading {
            Some(h) => match book.measures.get(h) {
                None => out.push(format!("measure `{}` records `{h}`, which is no measure of section 2.1", m.id)),
                Some(cited) => {
                    if m.sources.is_empty() {
                        out.push(format!("measure `{}` names no source", m.id));
                    }
                    for k in &m.sources {
                        if !cited.contains(k) {
                            out.push(format!("measure `{}` cites [{k}], which section 2.1's `{h}` does not", m.id));
                        }
                    }
                }
            },
            None => {
                if m.regimes.is_empty() {
                    out.push(format!("measure `{}` names neither a section 2.1 measure nor a regime", m.id));
                }
                for k in &m.sources {
                    if !book.bibliography.contains(k) {
                        out.push(format!("measure `{}` cites [{k}], which the bibliography does not hold", m.id));
                    }
                }
            }
        }
        for r in &m.regimes {
            if !regime_matches(r, &book.regimes) && r != "SCALE-n" {
                out.push(format!("measure `{}` names regime `{r}`, which section 2.3 does not", m.id));
            }
        }
    }

    for (i, row) in ledger.rows.iter().enumerate() {
        let at = format!("row {i} ({} {})", row.cell, row.side);
        let cell = match parse_cell(&row.cell, book) {
            Ok(c) => Some(c),
            Err(e) => {
                out.push(format!("{at}: {e}"));
                None
            }
        };
        if !is_date(&row.date) {
            out.push(format!("{at}: the date `{}` is not YYYY-MM-DD", row.date));
        }
        let named: BTreeSet<&String> = row.values.keys().chain(row.runs.keys()).collect();
        for measure in named {
            if !columns.contains_key(measure.as_str()) {
                out.push(format!("{at}: `{measure}` is no column of the ledger"));
            }
        }
        for (measure, runs) in &row.runs {
            if runs.is_empty() {
                out.push(format!("{at}: `{measure}` has an empty list of runs"));
            }
            if !row.values.contains_key(measure) {
                out.push(format!("{at}: runs of `{measure}` with no value"));
            }
        }
        for closer in &row.closers {
            if !book.board.contains(closer) {
                out.push(format!("{at}: closer `{closer}` is not on the board"));
            }
        }
        let ours = row.side == "ours";
        if !ours && !book.bibliography.contains(&row.side) {
            out.push(format!("{at}: the side `{}` is neither ours nor a bibliography key", row.side));
        }
        match row.provenance {
            Provenance::Published => {
                if ours {
                    out.push(format!("{at}: our side is never published; it is simulated, modeled, measured or not yet"));
                }
                if row.locator.as_deref().is_none_or(|l| l.trim().is_empty()) {
                    out.push(format!("{at}: a published row says where in its source the number is"));
                }
                if row.conditions.as_deref().is_none_or(|c| c.trim().is_empty()) {
                    out.push(format!("{at}: a published row states its conditions"));
                }
                if row.values.is_empty() {
                    out.push(format!("{at}: a published row carries a value"));
                }
            }
            Provenance::NotYet => {
                if !ours {
                    out.push(format!("{at}: only our side is ever not yet"));
                }
                if !row.values.is_empty() || !row.runs.is_empty() {
                    out.push(format!("{at}: a not-yet row carries no value"));
                }
                if row.closers.is_empty() {
                    out.push(format!("{at}: a not-yet row names its closers"));
                }
            }
            p => {
                if !ours {
                    out.push(format!("{at}: a cited side is published, not {p:?}"));
                }
                if row.values.is_empty() {
                    out.push(format!("{at}: a {p:?} row carries a value"));
                }
                match &row.program {
                    Some(prog) => {
                        if !is_hex(&prog.spec_digest, 64) || !is_hex(&prog.source_sha256, 64) || prog.cut.trim().is_empty() {
                            out.push(format!("{at}: the program identity {prog:?} is a spec digest, a cut and a source sha256"));
                        }
                    }
                    None => out.push(format!("{at}: our row carries its program identity")),
                }
                if row.bundle_digest.as_deref().is_none_or(|d| !is_hex(d, 64)) {
                    out.push(format!("{at}: our row carries its bundle's digest"));
                }
                if row.log.as_deref().is_none_or(|l| !(l.starts_with("logs/") || l.starts_with("benchmarks/"))) {
                    out.push(format!("{at}: our row names its log under logs/ or benchmarks/"));
                }
                match p {
                    Provenance::Modeled if row.time_model.is_none() => {
                        out.push(format!("{at}: a modeled row names the time model's version and calibration set"))
                    }
                    Provenance::Simulated if row.simulator.is_none() => {
                        out.push(format!("{at}: a simulated row names its simulator and tolerance"))
                    }
                    Provenance::Measured if row.boot.is_none() => out.push(format!("{at}: a measured row names its boot and load")),
                    _ => {}
                }
            }
        }

        if !row.won {
            continue;
        }
        if !ours {
            out.push(format!("{at}: only our side wins"));
        }
        if row.provenance == Provenance::Modeled {
            out.push(format!("{at}: a won row is modeled; a modeled row ranks plans and predicts, it never wins"));
        }
        if matches!(row.provenance, Provenance::NotYet | Provenance::Published) {
            out.push(format!("{at}: a {:?} row cannot win", row.provenance));
        }
        if let Some(cell) = &cell {
            let need = book.chips.get(&cell.chip).map(String::as_str).unwrap_or("never");
            let have = match row.provenance {
                Provenance::Simulated => "simulated",
                Provenance::Measured => "measured",
                _ => "",
            };
            if need == "never" {
                out.push(format!("{at}: a {} cell is never won: it is a Copy bar only", cell.chip));
            } else if need != have {
                out.push(format!("{at}: a {} cell is won only by a {need} row, not a {:?} one", cell.chip, row.provenance));
            }
        }
        match &row.quality {
            None => out.push(format!("{at}: a won row carries its quality gates")),
            Some(q) => {
                if q.reference.trim().is_empty() || q.prompts == 0 {
                    out.push(format!("{at}: the quality gates name their reference and prompts"));
                }
                if q.top1_agreement < q.reference_top1_agreement {
                    out.push(format!(
                        "{at}: top-1 agreement {} under the reference implementation's own {}",
                        q.top1_agreement, q.reference_top1_agreement
                    ));
                }
                if q.kl > q.kl_bound || q.perplexity_delta.abs() > q.perplexity_bound {
                    out.push(format!("{at}: KL {} or perplexity change {} past its bound", q.kl, q.perplexity_delta));
                }
            }
        }
        let rivals: Vec<&Row> = ledger.rows.iter().filter(|r| r.cell == row.cell && r.side != "ours").collect();
        if rivals.is_empty() {
            out.push(format!("{at}: a won row with no rival on its cell is not a claim"));
        }
        let margin = row.simulator.as_ref().map_or(0.0, |s| s.tolerance);
        for (measure, ours_value) in &row.values {
            let Some(m) = columns.get(measure.as_str()) else { continue };
            for rival in &rivals {
                let Some(theirs) = rival.values.get(measure) else { continue };
                // A measured win counts its slowest run against the rival's number.
                let worst = row.runs.get(measure).map_or(*ours_value, |runs| match m.better {
                    Better::Higher => runs.iter().copied().fold(f64::INFINITY, f64::min),
                    _ => runs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                });
                if m.better != Better::None && !ahead(m.better, worst, *theirs, margin) {
                    out.push(format!("{at}: `{measure}` {worst} is not ahead of [{}]'s {theirs}", rival.side));
                }
            }
        }
        if row.provenance == Provenance::Measured && row.runs.values().any(|r| r.len() < 5) {
            out.push(format!("{at}: a measured win stands on at least five runs"));
        }
    }
    out
}
