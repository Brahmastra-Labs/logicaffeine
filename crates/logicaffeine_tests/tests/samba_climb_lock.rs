//! samba_climb_lock — the Samba book's hill-climb keeps its records.
//!
//! The experiment ledger, `benchmarks/silicon/samba_experiments.json` (CLIMB-3), holds every experiment the climb runs:
//! its hypothesis, the change, the measured delta, the verdict (kept, rejected, or no effect when the delta is within
//! the noise) and the log. A row is written from an `EXPERIMENT SAMBA` line on the lanes board and keeps the line
//! verbatim, or was migrated from an earlier campaign's record in the tree, which its log points at by JSON pointer.
//! The rules live in `common/samba_climb.rs`.
//!
//! The lanes board is read from `SAMBA_LANES` when it names one (tool probe `samba-lanes`); with
//! `LOGOS_UPDATE_SAMBA_EXPERIMENTS=1` as well, every line without a row is appended as one, dated today.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test samba_climb_lock`

mod common;
#[path = "common/samba_climb.rs"]
mod samba_climb;

use samba_climb::{board_lines, parse_line, row_of, violations, Ledger, Line, Verdict};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ledger_path() -> PathBuf {
    repo().join("benchmarks/silicon/samba_experiments.json")
}

fn ledger() -> Ledger {
    let path = ledger_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The board's ticket ids, from the Samba book's index.
fn board() -> BTreeSet<String> {
    let book = std::fs::read_to_string(repo().join("benchmarks/PREPARE_FOR_SAMBA.md")).expect("the Samba book");
    let at = book.find("### 11.1 Board index").expect("section 11.1");
    let end = book[at..].find("### 11.2").map(|i| at + i).expect("section 11.2");
    book[at..end]
        .lines()
        .filter_map(|l| l.strip_prefix("- [ ] **").or_else(|| l.strip_prefix("- [x] **")))
        .map(|l| l.split("**").next().unwrap().to_string())
        .collect()
}

fn today() -> String {
    let out = std::process::Command::new("date").arg("-u").arg("+%Y-%m-%d").output().expect("date");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

const LINE: &str = "EXPERIMENT SAMBA SCHED-1 on: Tungsten, SARA 20x20 HBM2 hypothesis: a FIFO in place of the barrier between \
    the norm and the projection hides the norm's time change: the norm streams into the projection's first PCU delta: -4.1% \
    cycles a layer (1,204,311 to 1,154,936) verdict: kept log: logs/samba/sched1-fifo.log";

#[test]
fn every_experiment_line_becomes_a_row() {
    let mut l = ledger();
    if common::require_tool("samba-lanes") {
        let board_text = std::fs::read_to_string(std::env::var("SAMBA_LANES").unwrap()).expect("the lanes board");
        let lines: Vec<&str> = board_lines(&board_text);
        let have: BTreeSet<String> = l.rows.iter().filter_map(|r| r.line.clone()).collect();
        let missing: Vec<&str> = lines.iter().copied().filter(|line| !have.contains(*line)).collect();
        if std::env::var_os("LOGOS_UPDATE_SAMBA_EXPERIMENTS").is_some() && !missing.is_empty() {
            let date = today();
            for line in &missing {
                let id = format!("X-{:04}", l.rows.len() + 1);
                l.rows.push(row_of(line, &id, &date).unwrap_or_else(|e| panic!("{e}")));
            }
            std::fs::write(ledger_path(), serde_json::to_string_pretty(&l).unwrap() + "\n").unwrap();
        } else {
            assert!(missing.is_empty(), "lanes-board lines with no row: {missing:#?}");
        }
    }
    let v = violations(&l, &board(), &repo());
    assert!(v.is_empty(), "{v:#?}");
    // A line becomes the row it says, and the rules accept it.
    let mut with_line = l.clone();
    let id = format!("X-{:04}", with_line.rows.len() + 1);
    with_line.rows.push(row_of(LINE, &id, "2026-10-29").unwrap());
    let v = violations(&with_line, &board(), &repo());
    assert!(v.is_empty(), "{v:#?}");
}

#[test]
fn the_line_parser_reads_lines_as_written() {
    assert_eq!(
        parse_line(LINE).unwrap(),
        Line {
            ticket: "SCHED-1".into(),
            on: "Tungsten, SARA 20x20 HBM2".into(),
            hypothesis: "a FIFO in place of the barrier between the norm and the projection hides the norm's time".into(),
            change: "the norm streams into the projection's first PCU".into(),
            delta: "-4.1% cycles a layer (1,204,311 to 1,154,936)".into(),
            verdict: Verdict::Kept,
            log: "logs/samba/sched1-fifo.log".into(),
        }
    );
    for bad in [
        "EXPERIMENT SAMBA SCHED-1 hypothesis: x change: y delta: z verdict: kept log: logs/a",
        "EXPERIMENT SAMBA SCHED-1 on: a hypothesis: x change: y delta: z verdict: maybe log: logs/a",
        "EXPERIMENT SAMBA SCHED-1 on: a hypothesis: x change: y delta: z verdict: kept",
        "EXPERIMENT SAMBA SCHED-1 on: a hypothesis:  change: y delta: z verdict: kept log: logs/a",
        "EXPERIMENT SAMBA SCHED-1 on: a change: y hypothesis: x delta: z verdict: kept log: logs/a",
        "RECEIPT SAMBA SCHED-1 \"a box\" receipt: logs/a",
    ] {
        assert!(parse_line(bad).is_err(), "`{bad}` reads as an experiment line");
    }
    let board = "CLAIM SAMBA SCHED-1 cut R7 files: x\n  EXPERIMENT SAMBA SCHED-1 on: a hypothesis: x change: y delta: z verdict: \
                 no-effect log: logs/a\nDONE SAMBA SCHED-1 landed: x suite: y\n";
    assert_eq!(board_lines(board).len(), 1);
}

#[test]
fn a_delta_within_noise_is_no_effect_and_kept() {
    let line = "EXPERIMENT SAMBA SCHED-1 on: Tungsten hypothesis: double-buffering the weight tile hides its load change: \
                two PMU buffers in place of one delta: +0.2% cycles, inside the run-to-run spread of 0.5% verdict: no-effect \
                log: logs/samba/sched1-db.log";
    let mut l = ledger();
    let id = format!("X-{:04}", l.rows.len() + 1);
    let row = row_of(line, &id, "2026-10-29").unwrap();
    assert_eq!(row.verdict, Verdict::NoEffect);
    l.rows.push(row);
    let v = violations(&l, &board(), &repo());
    assert!(v.is_empty(), "a no-effect row is a row like any other: {v:#?}");
}

#[test]
fn a_row_names_its_log() {
    let mut l = ledger();
    let id = format!("X-{:04}", l.rows.len() + 1);
    let mut row = row_of(LINE, &id, "2026-10-29").unwrap();
    row.log = String::new();
    row.line = None;
    row.ticket = None;
    l.rows.push(row);
    let v = violations(&l, &board(), &repo());
    assert!(v.iter().any(|m| m.contains("a row names its log")), "{v:#?}");
}

#[test]
fn the_rules_refuse_what_they_should() {
    let refused = |edit: &dyn Fn(&mut samba_climb::Row), needle: &str| {
        let mut l = ledger();
        let id = format!("X-{:04}", l.rows.len() + 1);
        let mut row = row_of(LINE, &id, "2026-10-29").unwrap();
        edit(&mut row);
        l.rows.push(row);
        let v = violations(&l, &board(), &repo());
        assert!(v.iter().any(|m| m.contains(needle)), "expected `{needle}`: {v:#?}");
    };
    refused(&|r| r.id = "X-9999".into(), "rows are numbered in order");
    refused(&|r| r.date = "29 Oct".into(), "is not YYYY-MM-DD");
    refused(&|r| r.ticket = Some("SCHED-99".into()), "is not on the board");
    refused(&|r| r.delta = "-9% cycles".into(), "the row differs from its line");
    refused(&|r| r.log = "/tmp/x.log".into(), "is not under logs/ or benchmarks/");
    refused(&|r| r.line = None, "comes from its line");
    refused(
        &|r| {
            r.line = None;
            r.ticket = None;
            r.log = "benchmarks/results/qwen38_27b.json".into();
        },
        "points at its record",
    );
    refused(
        &|r| {
            r.line = None;
            r.ticket = None;
            r.log = "benchmarks/results/qwen38_27b.json#/no/such/key".into();
        },
        "names nothing in",
    );
}

#[test]
fn every_migrated_row_points_at_its_record() {
    let l = ledger();
    let migrated: Vec<_> = l.rows.iter().filter(|r| r.line.is_none()).collect();
    assert!(migrated.len() >= 20, "the ledger holds {} rows migrated from earlier campaigns", migrated.len());
    for r in &migrated {
        assert!(r.log.contains('#'), "{} points at no record: {}", r.id, r.log);
        assert!(r.lesson.as_deref().is_some_and(|s| !s.trim().is_empty()), "{} states no lesson", r.id);
    }
    assert!(migrated.iter().any(|r| r.verdict == Verdict::Rejected), "the migrated rows hold the dead ends");
}
