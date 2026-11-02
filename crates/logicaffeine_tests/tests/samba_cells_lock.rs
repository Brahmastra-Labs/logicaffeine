//! samba_cells_lock — the Samba book's cell ledger keeps the rules of a fair win.
//!
//! `benchmarks/silicon/samba_cells.json` is the one home of every score in `benchmarks/PREPARE_FOR_SAMBA.md`: one row
//! per cell and side, our side simulated, modeled, measured or not yet, the other side a published result under its
//! citation (section 2.3). Its columns are the measures of section 2.1, each with its definition and source, and the
//! exam's own. Every row is held to section 2.4: a modeled row never wins, a won row carries its quality gates and is
//! ahead of every rival on its cell, a Plasticine cell is won only by a simulated row and an RDU cell only by a measured
//! one, our rows carry their program identity, and a cell's Copy bar is its latest published row with every earlier
//! one kept. The rules live in `common/samba_cells.rs`; each is also run here against a ledger built to break it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test samba_cells_lock`

#[path = "common/samba_cells.rs"]
mod samba_cells;

use samba_cells::{bar, parse_cell, violations, Book, Cell, Ledger};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn book_text() -> String {
    std::fs::read_to_string(repo().join("benchmarks/PREPARE_FOR_SAMBA.md")).expect("the Samba book")
}

fn ledger_json() -> Value {
    let path = repo().join("benchmarks/silicon/samba_cells.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn ledger() -> Ledger {
    serde_json::from_value(ledger_json()).expect("the ledger reads under schema 1")
}

/// The ledger's own columns with `rows` in place of its rows, approved so that rows may stand.
fn with_rows(rows: Value) -> Ledger {
    let mut l = ledger_json();
    l["approved"] = json!({"by": "Tristen", "date": "2026-10-29", "line": "fixture"});
    l["rows"] = rows;
    serde_json::from_value(l).expect("a fixture ledger reads under schema 1")
}

const HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A published Copy bar: Llama-3.1-8B at batch 1 on eight SN40L sockets.
fn published(date: &str, value: f64) -> Value {
    json!({
        "cell": "C-SN40Lx8-LLAMA31-8B-D1-BF16", "side": "KL24", "provenance": "published", "date": date,
        "values": {"tok_s_user": value}, "conditions": "batch 1, decode", "locator": "Table 3", "won": false
    })
}

/// Our side of a cell, a winner unless the test takes something away.
fn ours(cell: &str, provenance: &str, value: f64) -> Value {
    let mut r = json!({
        "cell": cell, "side": "ours", "provenance": provenance, "date": "2026-10-29",
        "values": {"tok_s_user": value}, "won": true,
        "quality": {
            "reference": "transformers f32", "prompts": 10, "top1_agreement": 0.99, "reference_top1_agreement": 0.98,
            "kl": 0.001, "kl_bound": 0.01, "perplexity_delta": 0.01, "perplexity_bound": 0.05
        },
        "program": {"spec_digest": HEX, "cut": "decode", "source_sha256": HEX},
        "bundle_digest": HEX, "log": "logs/samba/fixture.log"
    });
    match provenance {
        "measured" => {
            r["boot"] = json!({"id": "boot-1", "host": "fixture", "load_seconds": 30.0});
            r["runs"] = json!({"tok_s_user": [value, value + 1.0, value + 2.0, value + 3.0, value + 4.0]});
        }
        "simulated" => r["simulator"] = json!({"name": "Tungsten", "version": "fixture", "tolerance": 0.01}),
        "modeled" => r["time_model"] = json!({"version": "fixture", "calibration": "fixture"}),
        _ => {}
    }
    r
}

fn complaints(l: &Ledger) -> Vec<String> {
    violations(l, &Book::read(&book_text()))
}

fn assert_refused(l: &Ledger, needle: &str) {
    let v = complaints(l);
    assert!(v.iter().any(|m| m.contains(needle)), "expected a complaint containing `{needle}`, got {v:#?}");
}

fn assert_clean(l: &Ledger) {
    let v = complaints(l);
    assert!(v.is_empty(), "{v:#?}");
}

#[test]
fn the_ledger_keeps_every_rule() {
    assert_clean(&ledger());
}

#[test]
fn a_won_row_is_never_modeled() {
    let l = ledger();
    assert!(
        !l.rows.iter().any(|r| r.won && r.provenance == samba_cells::Provenance::Modeled),
        "the ledger holds a won modeled row"
    );
    let rows = json!([published("2024-10-31", 584.0), ours("C-SN40Lx8-LLAMA31-8B-D1-BF16", "modeled", 700.0)]);
    assert_refused(&with_rows(rows), "a won row is modeled");
}

#[test]
fn a_won_row_carries_its_quality_gates() {
    let l = ledger();
    assert!(!l.rows.iter().any(|r| r.won && r.quality.is_none()), "the ledger holds a won row without its quality gates");
    let mut row = ours("C-SN40Lx8-LLAMA31-8B-D1-BF16", "measured", 700.0);
    row.as_object_mut().unwrap().remove("quality");
    assert_refused(&with_rows(json!([published("2024-10-31", 584.0), row])), "carries its quality gates");
    let mut low = ours("C-SN40Lx8-LLAMA31-8B-D1-BF16", "measured", 700.0);
    low["quality"]["top1_agreement"] = json!(0.97);
    assert_refused(&with_rows(json!([published("2024-10-31", 584.0), low])), "under the reference implementation's own");
    let mut far = ours("C-SN40Lx8-LLAMA31-8B-D1-BF16", "measured", 700.0);
    far["quality"]["kl"] = json!(0.5);
    assert_refused(&with_rows(json!([published("2024-10-31", 584.0), far])), "past its bound");
    assert_clean(&with_rows(json!([published("2024-10-31", 584.0), ours("C-SN40Lx8-LLAMA31-8B-D1-BF16", "measured", 700.0)])));
}

#[test]
fn a_cell_with_only_a_published_side_is_a_bar_and_no_claim() {
    let l = with_rows(json!([published("2024-10-31", 584.0)]));
    assert_clean(&l);
    let b = bar(&l, "C-SN40Lx8-LLAMA31-8B-D1-BF16").expect("the cell's bar");
    assert_eq!(b.values["tok_s_user"], 584.0);
    assert!(!l.rows.iter().any(|r| r.won));
}

#[test]
fn two_published_sides_are_both_kept_and_the_later_is_the_bar() {
    let l = with_rows(json!([published("2025-06-01", 600.0), published("2024-10-31", 584.0)]));
    assert_clean(&l);
    assert_eq!(l.rows.len(), 2, "both published rows are kept");
    assert_eq!(bar(&l, "C-SN40Lx8-LLAMA31-8B-D1-BF16").unwrap().date, "2025-06-01", "the later publication is the bar");
    let won = with_rows(json!([
        published("2025-06-01", 600.0),
        published("2024-10-31", 584.0),
        ours("C-SN40Lx8-LLAMA31-8B-D1-BF16", "measured", 590.0)
    ]));
    assert_refused(&won, "is not ahead of [KL24]'s 600");
}

#[test]
fn a_plasticine_cell_is_won_only_by_a_simulated_row_and_an_rdu_cell_only_by_a_measured_one() {
    let plast = json!({
        "cell": "C-PLAST-RANDOMWALK-TRACE", "side": "PTUT", "provenance": "published", "date": "2019-06-01",
        "values": {"cycles": 95592587.0}, "conditions": "the kit's Tungsten configuration", "locator": "README", "won": false
    });
    let mut sim = ours("C-PLAST-RANDOMWALK-TRACE", "simulated", 0.0);
    sim["values"] = json!({"cycles": 90000000.0});
    assert_clean(&with_rows(json!([plast.clone(), sim.clone()])));
    let mut measured = ours("C-PLAST-RANDOMWALK-TRACE", "measured", 0.0);
    measured["values"] = json!({"cycles": 90000000.0});
    measured["runs"] = json!({"cycles": [9.0e7, 9.0e7, 9.0e7, 9.0e7, 9.0e7]});
    assert_refused(&with_rows(json!([plast, measured])), "a PLAST cell is won only by a simulated row");
    let rdu = ours("C-SN40Lx8-LLAMA31-8B-D1-BF16", "simulated", 700.0);
    assert_refused(&with_rows(json!([published("2024-10-31", 584.0), rdu])), "a SN40L cell is won only by a measured row");
    let cloud = json!({
        "cell": "C-SAMBACLOUD-OSS120B-D", "side": "AA26", "provenance": "published", "date": "2026-10-29",
        "values": {"tok_s_user": 740.0}, "conditions": "output speed", "locator": "the provider table", "won": false
    });
    let ours_cloud = ours("C-SAMBACLOUD-OSS120B-D", "measured", 800.0);
    assert_refused(&with_rows(json!([cloud, ours_cloud])), "a SAMBACLOUD cell is never won");
}

#[test]
fn a_win_is_ahead_of_every_rival_on_its_slowest_run() {
    let mut slow = ours("C-SN40Lx8-LLAMA31-8B-D1-BF16", "measured", 700.0);
    slow["runs"] = json!({"tok_s_user": [700.0, 701.0, 702.0, 703.0, 580.0]});
    assert_refused(&with_rows(json!([published("2024-10-31", 584.0), slow])), "580 is not ahead of [KL24]'s 584");
    let mut few = ours("C-SN40Lx8-LLAMA31-8B-D1-BF16", "measured", 700.0);
    few["runs"] = json!({"tok_s_user": [700.0, 701.0, 702.0]});
    assert_refused(&with_rows(json!([published("2024-10-31", 584.0), few])), "at least five runs");
    assert_refused(&with_rows(json!([ours("C-SN40Lx8-LLAMA31-8B-D1-BF16", "measured", 700.0)])), "no rival");
    let mut near = ours("C-PLAST-RANDOMWALK-TRACE", "simulated", 0.0);
    near["values"] = json!({"cycles": 95000000.0});
    let plast = json!({
        "cell": "C-PLAST-RANDOMWALK-TRACE", "side": "PTUT", "provenance": "published", "date": "2019-06-01",
        "values": {"cycles": 95592587.0}, "conditions": "the kit's Tungsten configuration", "locator": "README", "won": false
    });
    assert_refused(&with_rows(json!([plast, near])), "is not ahead of [PTUT]");
}

#[test]
fn our_rows_carry_their_identity_and_provenance() {
    for (field, needle) in [
        ("program", "carries its program identity"),
        ("bundle_digest", "carries its bundle's digest"),
        ("log", "names its log"),
    ] {
        let mut row = ours("C-SN40Lx8-LLAMA31-8B-D1-BF16", "measured", 700.0);
        row["won"] = json!(false);
        row.as_object_mut().unwrap().remove(field);
        assert_refused(&with_rows(json!([row])), needle);
    }
    for (provenance, field, needle) in [
        ("measured", "boot", "names its boot and load"),
        ("simulated", "simulator", "names its simulator"),
        ("modeled", "time_model", "time model's version"),
    ] {
        let mut row = ours("C-PLAST-RANDOMWALK-TRACE", provenance, 0.0);
        row["values"] = json!({"cycles": 1.0});
        row.as_object_mut().unwrap().remove("runs");
        row["won"] = json!(false);
        row.as_object_mut().unwrap().remove(field);
        assert_refused(&with_rows(json!([row])), needle);
    }
    let mut published_ours = ours("C-SN40Lx8-LLAMA31-8B-D1-BF16", "measured", 700.0);
    published_ours["provenance"] = json!("published");
    published_ours["won"] = json!(false);
    assert_refused(&with_rows(json!([published_ours])), "our side is never published");
    let mut modeled_rival = published("2024-10-31", 584.0);
    modeled_rival["provenance"] = json!("modeled");
    assert_refused(&with_rows(json!([modeled_rival])), "a cited side is published");
    let mut unlocated = published("2024-10-31", 584.0);
    unlocated.as_object_mut().unwrap().remove("locator");
    assert_refused(&with_rows(json!([unlocated])), "says where in its source");
    let mut uncited = published("2024-10-31", 584.0);
    uncited["side"] = json!("NOSUCHKEY");
    assert_refused(&with_rows(json!([uncited])), "neither ours nor a bibliography key");
    let mut no_column = published("2024-10-31", 584.0);
    no_column["values"] = json!({"furlongs": 1.0});
    assert_refused(&with_rows(json!([no_column])), "is no column of the ledger");
}

#[test]
fn a_not_yet_row_names_its_closers_on_the_board() {
    let not_yet = |closers: Value| {
        json!({
            "cell": "C-SN50-LLAMA33-70B-D-FP8", "side": "ours", "provenance": "not-yet", "date": "2026-10-29",
            "won": false, "closers": closers
        })
    };
    assert_clean(&with_rows(json!([not_yet(json!(["TIME-1", "NUM-2"]))])));
    assert_refused(&with_rows(json!([not_yet(json!([]))])), "names its closers");
    assert_refused(&with_rows(json!([not_yet(json!(["TIME-99"]))])), "closer `TIME-99` is not on the board");
    let mut valued = not_yet(json!(["TIME-1"]));
    valued["values"] = json!({"tok_s_user": 1.0});
    assert_refused(&with_rows(json!([valued])), "carries no value");
}

#[test]
fn the_schema_is_approved_before_the_first_row() {
    let l = ledger();
    assert!(l.rows.is_empty() || l.approved.is_some(), "the ledger holds rows before Tristen approved its schema");
    let mut unapproved = ledger_json();
    unapproved["rows"] = json!([published("2024-10-31", 584.0)]);
    let unapproved: Ledger = serde_json::from_value(unapproved).unwrap();
    assert_refused(&unapproved, "before Tristen approved the schema");
}

#[test]
fn every_measure_of_section_2_1_is_a_column_with_its_definition_and_source() {
    let book = Book::read(&book_text());
    let l = ledger();
    assert!(book.measures.len() >= 8, "section 2.1 names {} measures", book.measures.len());
    for (heading, cited) in &book.measures {
        let columns: Vec<_> = l.measures.iter().filter(|m| m.heading.as_deref() == Some(heading.as_str())).collect();
        assert!(!columns.is_empty(), "section 2.1's `{heading}` is no column of the ledger");
        for m in columns {
            assert!(!m.definition.trim().is_empty() && !m.sources.is_empty(), "`{}` has no definition or no source", m.id);
            assert!(m.sources.iter().all(|k| cited.contains(k)), "`{}` cites {:?}; `{heading}` cites {cited:?}", m.id, m.sources);
        }
    }
}

/// The cell ids of section 2.2's Copy bars.
fn copy_bar_cells(book: &str) -> Vec<String> {
    let at = book.find("### 2.2 The Copy bars").expect("section 2.2");
    let rest = &book[at..];
    let end = rest[1..].find("\n### ").map(|i| i + 1).unwrap_or(rest.len());
    rest[..end]
        .lines()
        .filter_map(|l| l.strip_prefix("| C-"))
        .map(|l| format!("C-{}", l.split(" | ").next().unwrap().trim()))
        .collect()
}

#[test]
fn every_copy_bar_cell_id_parses() {
    let text = book_text();
    let book = Book::read(&text);
    let cells = copy_bar_cells(&text);
    assert!(cells.len() >= 15, "section 2.2 holds {} Copy bars", cells.len());
    let mut seen = std::collections::BTreeSet::new();
    for id in &cells {
        if let Err(e) = parse_cell(id, &book) {
            panic!("a Copy bar's cell does not read under section 2.3: {e}");
        }
        assert!(seen.insert(id.clone()), "section 2.2 names `{id}` twice");
    }
}

#[test]
fn the_cell_grammar_reads_ids_as_written() {
    let book = Book::read(&book_text());
    let cell = |id: &str| parse_cell(id, &book);
    assert_eq!(
        cell("C-SN40Lx16-LLAMA31-70B-SPEC-K9-BF16").unwrap(),
        Cell { chip: "SN40L".into(), count: 16, model: "LLAMA31-70B".into(), regime: "SPEC-K9".into(), bits: Some("BF16".into()) }
    );
    assert_eq!(
        cell("C-SN50-LLAMA33-70B-D-FP8").unwrap(),
        Cell { chip: "SN50".into(), count: 1, model: "LLAMA33-70B".into(), regime: "D".into(), bits: Some("FP8".into()) }
    );
    assert_eq!(
        cell("C-SN40Lx8-LLAMA31-8B-SYNC").unwrap(),
        Cell { chip: "SN40L".into(), count: 8, model: "LLAMA31-8B".into(), regime: "SYNC".into(), bits: None }
    );
    assert_eq!(cell("C-SN50x16-QWEN38FN-SCALE-16-BF16").unwrap().regime, "SCALE-16");
    assert_eq!(cell("C-PLAST64x4-SQUEEZENET-INFER").unwrap().count, 4);
    for bad in [
        "SN50-LLAMA31-8B-D1",
        "C-SN50x1-LLAMA31-8B-D1",
        "C-SN50-D1",
        "C-H100-LLAMA31-8B-D1",
        "C-SN50-LLAMA31-8B-D2",
        "C-SN50-LLAMA31-8B-SCALE-0",
        "C-SN50-llama-D1",
        "C-SN50--D1",
    ] {
        assert!(cell(bad).is_err(), "`{bad}` reads as a cell");
    }
}
