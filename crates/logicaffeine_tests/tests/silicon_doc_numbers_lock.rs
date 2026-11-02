//! silicon_doc_numbers_lock — a document that quotes a hardened block's cells, area, critical path
//! or clock quotes the ledger's number for it.
//!
//! Per-block silicon figures are copied by hand into the guides, the benchmark READMEs and the
//! README. Re-hardening a block gives it a new netlist and turns every copy stale at once, and no
//! other lock reads the prose, so this one reads every markdown document under `docs/` and
//! `benchmarks/` plus the README against the signoff ledgers (the `signoff` class of `benchmarks/silicon/sky130_ledgers.json`),
//! along with the flow invocations the documents and the driver scripts teach. The reader and its
//! rules live in `common/silicon_docs.rs`.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/silicon_docs.rs"]
mod silicon_docs;

use silicon_docs::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every markdown document a reader might be handed a silicon number in.
fn documents() -> Vec<PathBuf> {
    let mut out = files_under(&root().join("docs"), &[".md"]);
    out.extend(files_under(&root().join("benchmarks"), &[".md"]));
    out.push(root().join("README.md"));
    out
}

#[test]
fn every_quoted_block_number_is_the_ledgers() {
    let blocks = ledger_blocks(&root());
    let (seen, wrong) = quoted_number_mismatches(&documents(), &root(), &blocks);
    assert!(
        seen > 0,
        "no document names any ledger block — the document walk is broken"
    );
    assert!(
        wrong.is_empty(),
        "{} quoted silicon number(s) disagree with the ledgers:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// The parser reads what it claims to: header-driven columns, printed precision, both row forms.
#[test]
fn the_reader_judges_by_header_and_printed_precision() {
    let mut blocks = BTreeMap::new();
    blocks.insert(
        "blk".to_string(),
        Block {
            cells: Some(69_433),
            area_mm2: Some(0.5923),
            crit_ns: Some(7.72),
            period_ns: Some(15.0),
            setup_slack_ns: Some(0.44),
        },
    );
    let table = [
        "| Block | Die area | Std cells | Crit. path | Notes |",
        "|---|---|---|---|---|",
        "| `blk` | 0.592 mm² | 69,433 | 7.72 ns | 1,234 things |",
        "| `blk` | 0.428 mm² | 65,432 | 11.07 ns | ok |",
        "| `other` | 9 mm² | 1 | 1 ns | not a ledger block |",
    ];
    let m = table_row_mismatches(&table, &blocks);
    assert_eq!(
        m.iter().filter(|(l, _)| *l == 3).count(),
        0,
        "a correct row must pass (the Notes column is not a field): {m:?}"
    );
    assert_eq!(
        m.iter().filter(|(l, _)| *l == 4).count(),
        3,
        "a stale row must fail on area, cells and path: {m:?}"
    );
    assert_eq!(m.len(), 3, "{m:?}");

    let mut half = BTreeMap::new();
    half.insert(
        "h".to_string(),
        Block {
            cells: None,
            area_mm2: Some(0.1225),
            crit_ns: None,
            period_ns: None,
            setup_slack_ns: None,
        },
    );
    let at_half = [
        "| Block | Area |",
        "|--|--|",
        "| `h` | 0.123 mm² |",
        "| `h` | 0.122 mm² |",
        "| `h` | 0.121 mm² |",
    ];
    let m = table_row_mismatches(&at_half, &half);
    assert_eq!(
        m.iter().map(|(l, _)| *l).collect::<Vec<_>>(),
        vec![5],
        "at an exact half both neighbours agree, nothing else: {m:?}"
    );

    let reordered = [
        "| Block | Std cells | Die area (mm²) | Crit path |",
        "|--|--|--|--|",
        "| `blk` | 69,433 | 0.5923 | 7.72 ns |",
    ];
    assert!(
        table_row_mismatches(&reordered, &blocks).is_empty(),
        "columns are read by header, not position"
    );

    let fmax = [
        "| Block | Fmax (every corner) |",
        "|--|--|",
        "| `blk` | 68.7 MHz |",
        "| `blk` | 129.5 MHz |",
    ];
    assert_eq!(table_row_mismatches(&fmax, &blocks).iter().map(|(l, _)| *l).collect::<Vec<_>>(), vec![4],
        "an Fmax is the frequency every corner meets, 1000 / (period - worst slack); a typical-corner one is caught");

    let described = [
        "| Block | Std cells | Signoff clock (ns) |",
        "|--|--|--|",
        "| `blk` (the CPU) | 65,432 | 11.07 |",
    ];
    assert_eq!(
        table_row_mismatches(&described, &blocks).len(),
        2,
        "a first cell that opens with the block, then describes it, is still that block's row"
    );

    let inline = "`blk` 0.592 mm²/69,433 cells/7.72 ns · fine\n\
                  `blk` 0.428 mm²/65,432 cells/11.07 ns · stale\n\
                  `blk` 0.59 mm²/69,433 · short form, then a wrapped one: `blk`\n  0.428 mm²/69,433\n\
                  `blk` (spaced) 0.5923 mm² / 65,432 cells\n\
                  `blk` 0.592 mm²/69,433 cells/15 ns clock, and `blk` 0.592 mm²/69,433 cells/7.72 ns clock";
    let m = inline_mismatches(inline, &blocks);
    assert_eq!(
        m.iter().filter(|(l, _)| *l == 2).count(),
        3,
        "area, cells and path all caught inline: {m:?}"
    );
    assert_eq!(
        m.iter().filter(|(l, _)| *l == 3).count(),
        1,
        "a summary wrapped onto the next line is still read: {m:?}"
    );
    assert_eq!(
        m.iter().filter(|(l, _)| *l == 5).count(),
        1,
        "the spaced slash form is read too: {m:?}"
    );
    assert_eq!(
        m.iter().filter(|(l, _)| *l == 6).count(),
        1,
        "`ns clock` is judged against the signoff clock: {m:?}"
    );
    assert_eq!(m.len(), 6, "{m:?}");
}

/// Every documented flow invocation for a registered block names the block's registered period.
///
/// The flow-config emitter refuses any other period for a registered block, so a stale period in a
/// usage line or a driver script is a command that no longer runs. A script should ask
/// `emit_flow_config --registered-period`; a document quoting a literal must quote the right one.
/// Every `.sh` under `dir`, at any depth.
fn shell_scripts(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "sh") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_documented_flow_invocation_uses_the_registered_period() {
    use logicaffeine_compile::codegen_sva::signoff_blocks::registered_period_ns;
    let mut files = documents();
    files.extend(shell_scripts(&root().join("scripts")));
    let (seen, wrong) = flow_invocation_mismatches(&files, &root(), registered_period_ns);
    assert!(
        seen > 0,
        "no documented invocation found — the scan is broken"
    );
    assert!(
        wrong.is_empty(),
        "documented flow invocations the emitter would refuse:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn the_count_reader_reads_through_markup_and_stops_at_clauses() {
    let counts = |t: &str| {
        stated_block_counts(t)
            .into_iter()
            .map(|(_, n, _)| n)
            .collect::<Vec<_>>()
    };
    assert_eq!(counts("**18 LOGOS blocks** signoff-clean"), vec![18]);
    assert_eq!(
        counts("Seventeen of our blocks are signoff-clean on Sky130"),
        vec![17]
    );
    assert_eq!(
        counts("<text x=\"393\" fill=\"#eef2f7\">17 Sky130 blocks</text>"),
        vec![17]
    );
    assert_eq!(
        counts("(\"115-op ISA\",\"S\"),(\"17 Sky130 blocks\",\"S\")"),
        vec![17]
    );
    assert_eq!(
        counts("across seven signoff ledgers; only blocks recorded clean"),
        Vec::<usize>::new()
    );
    assert_eq!(
        counts("routing = 0), joining our other signoff blocks"),
        Vec::<usize>::new()
    );
    assert_eq!(counts("17 blocks of cheese"), Vec::<usize>::new());
    assert_eq!(
        stated_block_counts("GDSII: a line\nwhere 18 signoff blocks sit")[0].0,
        2
    );
}
