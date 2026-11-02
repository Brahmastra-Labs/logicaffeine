//! The reader behind every lock that holds a document's per-block silicon numbers to the signoff
//! ledgers (the `signoff` class of `benchmarks/silicon/sky130_ledgers.json`). Included per-lock via
//! `#[path = "common/silicon_docs.rs"] mod silicon_docs;`. Every function takes its roots explicitly:
//! the ledgers live in the product tree, and the documents a lock walks may live elsewhere.
//!
//! Two quoted forms are held, both anchored on a backticked block name the ledgers know:
//!   * a markdown table row whose first cell opens with the block — each column is judged by its HEADER
//!     (`cells`, `area`/`die`/`mm²`, `crit`/`path`, `period`/`clock`), so a column of anything else is
//!     never mistaken for a ledger field;
//!   * the inline `` `block` <area> mm² / <cells>[ cells][/<n> ns[ clock]] `` summary form (spaces
//!     around the slash optional; `ns clock` is the signoff clock, a bare `ns` the critical path).
//!
//! A number is compared at the precision the document chose to print it: `0.428 mm²` agrees with
//! 0.4281 and `11.07 ns` with 11.07; `65,432` never agrees with 69,433.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Block {
    pub cells: Option<i64>,
    pub area_mm2: Option<f64>,
    pub crit_ns: Option<f64>,
    pub period_ns: Option<f64>,
    pub setup_slack_ns: Option<f64>,
}

/// The signoff-class ledgers the registry `benchmarks/silicon/sky130_ledgers.json` names, by file name.
fn signoff_ledgers(product: &Path) -> Vec<(String, serde_json::Value)> {
    let registry = product.join("benchmarks/silicon/sky130_ledgers.json");
    let reg: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&registry).unwrap_or_else(|e| panic!("read {}: {e}", registry.display())),
    )
    .unwrap_or_else(|e| panic!("bad JSON in {}: {e}", registry.display()));
    let mut out = Vec::new();
    for l in reg["ledgers"].as_array().expect("the registry's ledgers") {
        if l["class"] != "signoff" {
            continue;
        }
        let path = product.join(l["path"].as_str().expect("a ledger's path"));
        let n = path.file_name().unwrap().to_string_lossy().into_owned();
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|err| panic!("bad JSON in {n}: {err}"));
        out.push((n, doc));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Every block the signoff ledgers under `product` record, by name.
pub fn ledger_blocks(product: &Path) -> BTreeMap<String, Block> {
    let mut out = BTreeMap::new();
    for (_, doc) in signoff_ledgers(product) {
        let Some(blocks) = doc["blocks"].as_object() else {
            continue;
        };
        for (name, b) in blocks {
            out.insert(
                name.clone(),
                Block {
                    cells: b["std_cells"].as_i64(),
                    area_mm2: b["die_area_mm2"].as_f64(),
                    crit_ns: b["critical_path_ns"].as_f64(),
                    period_ns: b["clock_period_ns"].as_f64(),
                    setup_slack_ns: b["setup_worst_slack_ns"].as_f64(),
                },
            );
        }
    }
    assert!(
        out.len() >= 19,
        "found only {} ledger blocks — discovery is broken",
        out.len()
    );
    out
}

/// The block counts the ledgers produce: the total of clean blocks, and each ledger's own clean count
/// (the chip roadmap's "ten" is `sky130_signoff.json`).
pub fn allowed_block_counts(product: &Path) -> Vec<usize> {
    let per_ledger: Vec<usize> = signoff_ledgers(product)
        .iter()
        .filter_map(|(_, doc)| doc["blocks"].as_object())
        .map(|blocks| {
            blocks
                .values()
                .filter(|b| b["signoff_clean"].as_bool() == Some(true))
                .count()
        })
        .collect();
    let total: usize = per_ledger.iter().sum();
    std::iter::once(total)
        .chain(per_ledger.iter().copied().filter(|&c| c > 1))
        .collect()
}

/// Every file under `dir` whose name ends in one of `exts`, skipping build output; a missing `dir`
/// contributes nothing.
pub fn files_under(dir: &Path, exts: &[&str]) -> Vec<PathBuf> {
    fn walk(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                if !matches!(name.as_str(), "dist" | "node_modules" | "target" | ".git") {
                    walk(&p, exts, out);
                }
            } else if exts.iter().any(|x| name.ends_with(x)) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, exts, &mut out);
    out.sort();
    out
}

/// `text` is `truth` rounded at the precision `text` was printed at: within half a unit of its last
/// printed digit. At an exact half (a ledger area of 0.1225 printed to three places) either neighbour
/// is an honest rounding.
pub fn agrees(text: &str, truth: f64) -> bool {
    let t = text.trim();
    let decimals = t.split_once('.').map_or(0, |(_, frac)| frac.len()) as i32;
    t.parse::<f64>()
        .is_ok_and(|v| (v - truth).abs() <= 0.5 * 10f64.powi(-decimals) + 1e-9)
}

fn plain_number(cell: &str) -> Option<String> {
    let c = cell.trim().trim_matches('*').trim();
    let n: String = c.chars().filter(|ch| *ch != ',').collect();
    (!n.is_empty()
        && n.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
        && n.chars().next()?.is_ascii_digit())
    .then_some(n)
}

/// The number in `cell` stripped of a trailing unit, when the cell is just that number and unit.
fn number_with_unit(cell: &str, unit: &str) -> Option<String> {
    let c = cell.trim().trim_matches('*').trim();
    plain_number(c.strip_suffix(unit).unwrap_or(c).trim())
}

/// The block a row is about: its first cell OPENS with a backticked name (a description may follow).
fn block_of_first_cell(cell: &str) -> Option<&str> {
    let rest = cell
        .trim()
        .trim_start_matches('*')
        .trim_start()
        .strip_prefix('`')?;
    rest.split_once('`').map(|(name, _)| name)
}

#[derive(Clone, Copy, Debug)]
enum Field {
    Cells,
    Area,
    Crit,
    Period,
    Fmax,
}

fn field_of_header(h: &str) -> Option<Field> {
    let h = h.to_lowercase();
    if h.contains("fmax") {
        Some(Field::Fmax)
    } else if h.contains("cell") {
        Some(Field::Cells)
    } else if h.contains("area") || h.contains("die") || h.contains("mm²") || h.contains("mm2") {
        Some(Field::Area)
    } else if h.contains("crit") || h.contains("path") {
        Some(Field::Crit)
    } else if h.contains("period") || h.contains("clock") {
        Some(Field::Period)
    } else {
        None
    }
}

fn check(field: Field, text: &str, b: &Block) -> Option<String> {
    let (want, got) = match field {
        Field::Cells => {
            let v = b.cells?;
            let n = text.split('.').next().unwrap_or("");
            return (n.parse::<i64>().ok() != Some(v))
                .then(|| format!("cells {text} (ledger {v})"));
        }
        Field::Area => (b.area_mm2?, text),
        Field::Crit => (b.crit_ns?, text),
        Field::Period => (b.period_ns?, text),
        Field::Fmax => (1000.0 / (b.period_ns? - b.setup_slack_ns?), text),
    };
    (!agrees(got, want)).then(|| format!("{field:?} {got} (ledger {want})"))
}

pub fn table_row_mismatches(
    lines: &[&str],
    blocks: &BTreeMap<String, Block>,
) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut header: Vec<Option<Field>> = Vec::new();
    let mut in_table = false;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if !t.starts_with('|') {
            in_table = false;
            continue;
        }
        let cells: Vec<&str> = t.trim_matches('|').split('|').collect();
        if !in_table {
            header = cells.iter().map(|h| field_of_header(h)).collect();
            in_table = true;
            continue;
        }
        let Some(name) = cells.first().and_then(|c| block_of_first_cell(c)) else {
            continue;
        };
        let Some(b) = blocks.get(name) else { continue };
        for (col, cell) in cells.iter().enumerate().skip(1) {
            let Some(Some(field)) = header.get(col) else {
                continue;
            };
            let unit = match field {
                Field::Area => "mm²",
                Field::Crit | Field::Period => "ns",
                Field::Fmax => "MHz",
                Field::Cells => "",
            };
            let Some(num) = number_with_unit(cell, unit) else {
                continue;
            };
            if let Some(why) = check(*field, &num, b) {
                out.push((i + 1, format!("`{name}` {why}")));
            }
        }
    }
    out
}

/// `` `block` <area> mm²/<cells>[ cells][/<crit> ns] `` — the inline summary form. Read over the
/// whole text, because a wrapped paragraph puts the name at the end of one line and its numbers at
/// the start of the next; the summary runs to the next backtick.
pub fn inline_mismatches(text: &str, blocks: &BTreeMap<String, Block>) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(open) = text[at..].find('`').map(|o| at + o) {
        let Some(close) = text[open + 1..].find('`').map(|c| open + 1 + c) else {
            break;
        };
        let name = &text[open + 1..close];
        at = close + 1;
        let Some(b) = blocks.get(name) else { continue };
        let line = text[..open].matches('\n').count() + 1;
        let tail = text[at..].split('`').next().unwrap_or("");
        let Some(mm) = tail.find(" mm²") else {
            continue;
        };
        let area: String = {
            let mut v: Vec<char> = tail[..mm]
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            v.reverse();
            v.into_iter().collect()
        };
        let Some(after_mm) = tail[mm + " mm²".len()..]
            .trim_start_matches(' ')
            .strip_prefix('/')
        else {
            continue;
        };
        let after_mm = after_mm.trim_start_matches(' ');
        let digits_end = after_mm
            .find(|c: char| !(c.is_ascii_digit() || c == ','))
            .unwrap_or(after_mm.len());
        let cells: String = after_mm[..digits_end]
            .chars()
            .filter(|c| *c != ',')
            .collect();
        if plain_number(&area).is_some() {
            if let Some(why) = check(Field::Area, &area, b) {
                out.push((line, format!("`{name}` {why}")));
            }
        }
        if cells.is_empty() {
            continue;
        }
        if let Some(why) = check(Field::Cells, &cells, b) {
            out.push((line, format!("`{name}` {why}")));
        }
        let after_cells = after_mm[digits_end..].trim_start_matches(" cells");
        if let Some((ns, rest)) = after_cells
            .strip_prefix('/')
            .and_then(|s| s.split_once(" ns"))
        {
            let field = if rest.starts_with(" clock") {
                Field::Period
            } else {
                Field::Crit
            };
            if plain_number(ns).is_some() {
                if let Some(why) = check(field, ns.trim(), b) {
                    out.push((line, format!("`{name}` {why}")));
                }
            }
        }
    }
    out
}

/// Every quoted block number in `docs` that disagrees with the ledgers, as `path:line: why` relative
/// to `base`, and the number of lines that name a ledger block at all.
pub fn quoted_number_mismatches(
    docs: &[PathBuf],
    base: &Path,
    blocks: &BTreeMap<String, Block>,
) -> (usize, Vec<String>) {
    let mut wrong = Vec::new();
    let mut seen = 0usize;
    for doc in docs {
        let Ok(text) = std::fs::read_to_string(doc) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        let rel = doc.strip_prefix(base).unwrap_or(doc).display().to_string();
        let found: Vec<_> = table_row_mismatches(&lines, blocks)
            .into_iter()
            .chain(inline_mismatches(&text, blocks))
            .collect();
        seen += lines
            .iter()
            .filter(|l| blocks.keys().any(|b| l.contains(&format!("`{b}`"))))
            .count();
        wrong.extend(
            found
                .into_iter()
                .map(|(n, why)| format!("  {rel}:{n}: {why}")),
        );
    }
    (seen, wrong)
}

/// Every `openlane-run.sh <block> <port> <period>` invocation in `files` whose period is not the
/// block's registered one, as `path:line: why` relative to `base`, and the number of invocations of a
/// registered block seen.
pub fn flow_invocation_mismatches(
    files: &[PathBuf],
    base: &Path,
    registered: impl Fn(&str) -> Option<f64>,
) -> (usize, Vec<String>) {
    let mut wrong = Vec::new();
    let mut seen = 0usize;
    for f in files {
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        let rel = f.strip_prefix(base).unwrap_or(f).display().to_string();
        for (n, line) in text.lines().enumerate() {
            let Some(at) = line.find("openlane-run.sh") else {
                continue;
            };
            let args: Vec<&str> = line[at + "openlane-run.sh".len()..]
                .split_whitespace()
                .take(3)
                .collect();
            let [block, _port, period] = args[..] else {
                continue;
            };
            let Ok(p) = period.parse::<f64>() else {
                continue;
            };
            let Some(reg) = registered(block) else {
                continue;
            };
            seen += 1;
            if (p - reg).abs() > 1e-9 {
                wrong.push(format!(
                    "  {rel}:{}: `{block}` at {period} ns, registered {reg} ns",
                    n + 1
                ));
            }
        }
    }
    (seen, wrong)
}

pub const COUNT_WORDS: [&str; 21] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
];

/// Every signoff block count `text` states: (line, count, the word that stated it). A count is a
/// number that QUALIFIES "blocks" — only qualifier words between them, and no clause punctuation
/// ("seven signoff ledgers; only blocks" counts ledgers, "= 0), joining our other blocks" counts
/// nothing) — in a context about signoff. Tokens are read by their words, not their markup:
/// `>17 Sky130 blocks</text>` and `("17 Sky130 blocks","S")` state a count as surely as prose.
pub fn stated_block_counts(text: &str) -> Vec<(usize, usize, String)> {
    const QUALIFIERS: [&str; 14] = [
        "logos",
        "sky130",
        "signoff",
        "signoff-clean",
        "sign-off",
        "clean",
        "hardened",
        "of",
        "our",
        "the",
        "more",
        "gdsii",
        "signed",
        "off",
    ];
    let number = |w: &str| -> Option<usize> {
        w.parse()
            .ok()
            .or_else(|| COUNT_WORDS.iter().position(|x| x.eq_ignore_ascii_case(w)))
    };
    let mut toks: Vec<(usize, &str)> = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                toks.push((s, &text[s..i]));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        toks.push((s, &text[s..]));
    }
    let mut out = Vec::new();
    for (k, (at, tok)) in toks.iter().enumerate() {
        let t: String = tok
            .trim_start_matches(|c: char| !c.is_alphanumeric())
            .chars()
            .take_while(|c| c.is_alphanumeric())
            .collect();
        if t != "blocks" && t != "Blocks" {
            continue;
        }
        let bare = |s: &str| {
            s.trim_matches(|c: char| c == '*' || c == '`' || c == '(' || c == '"')
                .to_string()
        };
        for back in 1..=4 {
            let Some((_, cand)) = toks.get(k.wrapping_sub(back)) else {
                break;
            };
            let raw = bare(cand);
            if raw.ends_with([',', ';', ':', '.', ')']) {
                break;
            }
            let c = raw
                .rsplit(|ch: char| !(ch.is_alphanumeric() || ch == '-'))
                .next()
                .unwrap_or("");
            let Some(n) = number(c) else {
                if QUALIFIERS.contains(&c.to_lowercase().as_str()) {
                    continue;
                }
                break;
            };
            let lo = at.saturating_sub(120);
            let hi = (at + 120).min(text.len());
            let ctx = text.get(lo..hi).unwrap_or("").to_lowercase();
            if ["sky130", "signoff", "sign-off", "gdsii", "tape"]
                .iter()
                .any(|w| ctx.contains(w))
            {
                out.push((text[..*at].matches('\n').count() + 1, n, c.to_string()));
            }
            break;
        }
    }
    out
}

/// Every stated signoff block count in `files` the ledgers do not produce, as `path:line: why`
/// relative to `base`, and the number of counts seen.
pub fn block_count_mismatches(
    files: &[PathBuf],
    base: &Path,
    allowed: &[usize],
) -> (usize, Vec<String>) {
    let mut wrong = Vec::new();
    let mut seen = 0usize;
    for f in files {
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        for (line, n, word) in stated_block_counts(&text) {
            seen += 1;
            if !allowed.contains(&n) {
                let rel = f.strip_prefix(base).unwrap_or(f).display().to_string();
                wrong.push(format!(
                    "  {rel}:{line}: \"{word} … blocks\" — the ledgers produce {allowed:?}"
                ));
            }
        }
    }
    (seen, wrong)
}
