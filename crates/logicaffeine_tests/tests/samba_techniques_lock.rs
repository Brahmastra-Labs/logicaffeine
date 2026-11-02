//! samba_techniques_lock — every technique the Samba book's pinned sources describe is a row of
//! `benchmarks/samba_techniques.tsv`, and every row is answered by a ticket on the book's board.
//!
//! A row is one source's account of one technique: the source's key, the technique's name, its kind (`compiler`,
//! `mapper`, `runtime` or `hardware`), what we have of it today, the repository path that shows it, the tickets that
//! copy it, where in the source it is described and a quote of at most 25 words from the pinned text, and the
//! mechanism in our words. Two sources describing one technique are two rows under one name, and the rows of a name
//! give it one answer. What we have is `none` (no evidence), `partial` or `have` (a path that shows it), or `past` (we
//! go beyond the source; the evidence is the lock that shows it). A hardware feature is recorded with the chip facts:
//! one of its tickets is a CHIP ticket. Section 5.4 of the book names the pinned sources that describe no technique;
//! every other pinned source has rows, and the tickets of section 5's summary tables are each on a row of the sources
//! that row cites.
//!
//! The quotes are checked against the captures under the gitignored `external/papers/`; without them that test records
//! an honest skip (`samba-papers`). `SAMBA_TECHNIQUES`, `SAMBA_BOOK` and `SAMBA_PAPERS` point a mutation run at mutated
//! copies.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test samba_techniques_lock`

mod common;
#[path = "common/samba_papers.rs"]
mod samba_papers;

use samba_papers::{pins, read, repo};
use std::collections::{BTreeMap, BTreeSet};

const COLUMNS: [&str; 9] = ["key", "technique", "kind", "ours", "evidence", "tickets", "locator", "quote", "mechanism"];
const KINDS: [&str; 4] = ["compiler", "mapper", "runtime", "hardware"];
const OURS: [&str; 4] = ["none", "partial", "have", "past"];

/// One row of the ledger, by column name.
type Row = BTreeMap<&'static str, String>;

fn ledger() -> Vec<Row> {
    let table = read("SAMBA_TECHNIQUES", "benchmarks/samba_techniques.tsv");
    let mut lines = table.lines();
    let head: Vec<&str> = lines.next().expect("a header").split('\t').collect();
    assert_eq!(head, COLUMNS, "the ledger's columns");
    lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let cells: Vec<&str> = l.split('\t').collect();
            assert_eq!(cells.len(), COLUMNS.len(), "a row of {} cells: {l}", cells.len());
            COLUMNS.iter().zip(cells).map(|(h, c)| (*h, c.to_string())).collect()
        })
        .collect()
}

fn book() -> String {
    read("SAMBA_BOOK", "benchmarks/PREPARE_FOR_SAMBA.md")
}

fn tickets(row: &Row) -> Vec<&str> {
    row["tickets"].split_whitespace().collect()
}

/// The text after the heading line `start`, up to the next heading of the same or a higher level.
fn section<'a>(text: &'a str, start: &str) -> &'a str {
    let at = text
        .match_indices(start)
        .map(|(i, _)| i)
        .find(|&i| i == 0 || text.as_bytes()[i - 1] == b'\n')
        .unwrap_or_else(|| panic!("no heading {start:?}"));
    let level = start.chars().take_while(|c| *c == '#').count();
    let rest = &text[at + start.len()..];
    let end = rest
        .match_indices('\n')
        .map(|(i, _)| i + 1)
        .find(|&i| {
            let hashes = rest[i..].chars().take_while(|c| *c == '#').count();
            hashes > 0 && hashes <= level && rest[i + hashes..].starts_with(' ')
        })
        .unwrap_or(rest.len());
    &rest[..end]
}

/// The ids on a board index: `- [ ] **ID** · …`.
fn board_ids(index: &str) -> BTreeSet<String> {
    index
        .lines()
        .filter_map(|l| l.strip_prefix("- [ ] **").or_else(|| l.strip_prefix("- [x] **")))
        .filter_map(|r| r.split_once("**").map(|(id, _)| id.to_string()))
        .collect()
}

fn board(book: &str) -> BTreeSet<String> {
    board_ids(section(book, "### 11.1 Board index"))
}

fn fusions_board() -> BTreeSet<String> {
    let fusions = read("FUSIONS_BOOK", "benchmarks/FUSIONS_WE_NEED.md");
    board_ids(section(&fusions, "### 12.1 Board index"))
}

/// The keys section 5.4 names as describing no technique: its bullets `- [KEY] why`.
fn describing_none(book: &str) -> BTreeSet<String> {
    let whole = section(book, "### 5.4 The whole set");
    let list = whole
        .split_once("**Sources that describe no technique.**")
        .expect("section 5.4 names the sources that describe no technique")
        .1;
    list.lines()
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| l.starts_with("- [") || l.starts_with("  "))
        .filter_map(|l| l.strip_prefix("- [").and_then(|r| r.split_once(']')).map(|(k, _)| k.to_string()))
        .collect()
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn every_technique_row_names_a_ticket_on_the_board() {
    let book = book();
    let board = board(&book);
    let fusions = fusions_board();
    let rows = ledger();
    assert!(rows.len() >= 400, "the ledger holds {} rows", rows.len());
    let mut unknown = BTreeSet::new();
    for row in &rows {
        let named = tickets(row);
        assert!(!named.is_empty(), "[{}] {}: no ticket copies it", row["key"], row["technique"]);
        for t in named {
            let known = match t.strip_prefix("FUSIONS:") {
                Some(f) => fusions.contains(f),
                None => board.contains(t),
            };
            if !known {
                unknown.insert(format!("{t} ([{}] {})", row["key"], row["technique"]));
            }
        }
    }
    assert!(unknown.is_empty(), "rows name tickets no board holds:\n{}", unknown.into_iter().collect::<Vec<_>>().join("\n"));
}

#[test]
fn every_row_cites_a_pinned_source_in_a_short_quote() {
    let keys: BTreeSet<String> = pins().into_iter().map(|p| p["key"].clone()).collect();
    for row in ledger() {
        let at = format!("[{}] {}", row["key"], row["technique"]);
        assert!(keys.contains(&row["key"]), "{at}: the key is no pinned source");
        assert!(KINDS.contains(&row["kind"].as_str()), "{at}: kind {:?}", row["kind"]);
        for col in ["technique", "locator", "quote", "mechanism"] {
            assert!(!row[col].trim().is_empty(), "{at}: no {col}");
        }
        let words = row["quote"].split_whitespace().count();
        assert!(words <= 25, "{at}: a quote of {words} words");
    }
}

#[test]
fn every_quote_is_in_its_pinned_source() {
    if !common::require_tool("samba-papers") {
        return;
    }
    let mut texts: BTreeMap<String, String> = BTreeMap::new();
    for p in pins() {
        let path = repo().join(&p["text"]);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("[{}] {}: {e}", p["key"], path.display()));
        let e = texts.entry(p["key"].clone()).or_default();
        e.push_str(&squash(&String::from_utf8_lossy(&bytes)));
        e.push('\n');
    }
    let unfound: Vec<String> = ledger()
        .into_iter()
        .filter(|row| !texts[&row["key"]].contains(&squash(&row["quote"])))
        .map(|row| format!("[{}] {}: {:?}", row["key"], row["technique"], row["quote"]))
        .collect();
    assert!(unfound.is_empty(), "quotes not in their pinned source:\n{}", unfound.join("\n"));
}

#[test]
fn one_technique_has_one_answer() {
    let mut answers: BTreeMap<String, (String, String, String, String)> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for row in ledger() {
        assert!(seen.insert((row["key"].clone(), row["technique"].clone())), "[{}] {} is two rows", row["key"], row["technique"]);
        let answer = (row["kind"].clone(), row["ours"].clone(), row["evidence"].clone(), row["tickets"].clone());
        let first = answers.entry(row["technique"].clone()).or_insert_with(|| answer.clone());
        assert_eq!(*first, answer, "{}: [{}] answers it otherwise than an earlier row (kind, ours, evidence, tickets)", row["technique"], row["key"]);
    }
}

#[test]
fn every_status_carries_its_evidence() {
    let root = repo();
    for row in ledger() {
        let at = format!("[{}] {}", row["key"], row["technique"]);
        let (ours, evidence) = (row["ours"].as_str(), row["evidence"].as_str());
        assert!(OURS.contains(&ours), "{at}: ours {ours:?}");
        if ours == "none" {
            assert_eq!(evidence, "-", "{at}: nothing of it is ours, so nothing shows it");
            continue;
        }
        assert!(root.join(evidence).exists(), "{at}: the evidence {evidence} does not exist");
        if ours == "past" {
            assert!(
                evidence.starts_with("crates/logicaffeine_tests/tests/") && evidence.ends_with("_lock.rs"),
                "{at}: going past a source is shown by a lock, not {evidence}"
            );
        }
    }
}

#[test]
fn a_hardware_feature_is_recorded_with_the_chip_facts() {
    for row in ledger().into_iter().filter(|r| r["kind"] == "hardware") {
        assert!(
            tickets(&row).iter().any(|t| t.starts_with("CHIP-")),
            "[{}] {}: a hardware feature names a CHIP ticket, not only {}",
            row["key"],
            row["technique"],
            row["tickets"]
        );
    }
}

#[test]
fn every_pinned_source_is_mined_or_named_as_describing_none() {
    let book = book();
    let none = describing_none(&book);
    let mined: BTreeSet<String> = ledger().into_iter().map(|r| r["key"].clone()).collect();
    for key in pins().into_iter().map(|p| p["key"].clone()).collect::<BTreeSet<_>>() {
        match (mined.contains(&key), none.contains(&key)) {
            (true, true) => panic!("[{key}] has rows, yet section 5.4 says it describes no technique"),
            (false, false) => panic!("[{key}] is pinned, has no row, and section 5.4 does not say why"),
            _ => {}
        }
    }
    let pinned: BTreeSet<String> = pins().into_iter().map(|p| p["key"].clone()).collect();
    for key in &none {
        assert!(pinned.contains(key), "section 5.4 names [{key}], which is no pinned source");
    }
}

#[test]
fn the_summary_tables_draw_on_the_ledger() {
    let book = book();
    let rows = ledger();
    let mut checked = 0;
    for heading in ["### 5.1 SambaNova's published compiler techniques", "### 5.2 The Stanford lineage"] {
        for line in section(&book, heading).lines().filter(|l| l.starts_with("| ") && !l.starts_with("|---")) {
            let cells: Vec<&str> = line.trim_matches('|').split(" | ").map(str::trim).collect();
            let keys: Vec<&str> = cells[1].split('[').skip(1).filter_map(|s| s.split_once(']').map(|(k, _)| k)).collect();
            if keys.is_empty() {
                continue;
            }
            let answered: BTreeSet<&str> = rows.iter().filter(|r| keys.contains(&r["key"].as_str())).flat_map(tickets).collect();
            for t in cells[cells.len() - 1].split(',').map(str::trim) {
                checked += 1;
                assert!(answered.contains(t), "section 5 row {:?} names {t}, which no ledger row of {keys:?} names", cells[0]);
            }
        }
    }
    assert!(checked >= 30, "section 5's tables name {checked} tickets");
}
