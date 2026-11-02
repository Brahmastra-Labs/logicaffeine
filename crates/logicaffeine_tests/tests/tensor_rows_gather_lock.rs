//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ROWS OF A TABLE — a block gather: the embedding of every token of a chunk at once.
//!
//! `Let X be rows I of T` — `I` an Int tensor of `B` row indices (1-based), `T` a declared table of `R` rows — binds `X`,
//! `B` rows of `T`'s width: row `k` of `X` is row `I[k]` of `T`. A prefill chunk's embedding is `rows toks of Emb`: its
//! `B` token ids in one tensor, looked up in one go.
//!
//! Pinned, on the VM and the tree-walker (which must agree):
//!   1. Row `k` is exactly `row I[k] of T` — for Real and Int tables, indices laid out as a row and as a column,
//!      repeated and unordered indices, and one index.
//!   2. An index outside the table fails at run time with one message on every tier, naming the table and its rows;
//!      indices that are not Int, and a table that is not declared, are refused by name.
//!   3. THE SN50 GATHERS THE BLOCK IN ONE SECTION. Rows of whole tiles and rows narrower than a tile, a few indices and
//!      more than a tile of them, lower whole to ONE section and evaluate to the VM's values exactly on the fp32 twin and
//!      the bf16 chip alike — a gather moves values, and every value is a multiple of 1/64, which both widths hold.
//!   4. THE INDICES ARE DATA: one compiled program, evaluated with other indices, gathers the VM's rows for those
//!      indices; an index outside the table faults at its guard, against the table's rows.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test tensor_rows_gather_lock`
//!
//! ★ MUTATION PROBES (private copy, every one RED, 2026-09-26):
//!   • the desugar reads the first index for every row → `row k is row I[k]`, and `index 0 must fail`.
//!   • the whole-tile form reads element 0 of the indices for every item, or reads its item number from tile 0 → the
//!     gathered rows and `the indices are read at run time`.
//!   • the narrow form reads row 0's offset in every band → `the fp32 twin gathers the VM's rows`.
//!   • either form's guard admits one row past the table → `index 0 is checked against the table's rows`.
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::eval::{eval_choreography, ChoreoFault};
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram, TensorInit};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

fn val(seed: usize) -> f64 {
    (((seed.wrapping_mul(7919).wrapping_add(13)) % 129) as f64 - 64.0) / 64.0
}

fn lines(s: &str) -> Vec<String> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Tree-walker and VM agree; returns (output, error).
fn tiers(src: &str) -> (Vec<String>, Option<String>) {
    let (tw, vm) = (tw_outcome(src), vm_outcome(src));
    assert_eq!(
        (lines(&tw.output), &tw.error),
        (lines(&vm.output), &vm.error),
        "the VM and the tree-walker disagree on:\n{src}"
    );
    (lines(&tw.output), tw.error)
}

fn ran(src: &str, what: &str) -> Vec<String> {
    let (out, err) = tiers(src);
    assert_eq!(err, None, "{what}: {err:?}\n{src}");
    out
}

/// A table of `rows × cols`, Real (multiples of 1/64) or Int.
fn table(name: &str, rows: usize, cols: usize, int: bool) -> String {
    let kind = if int { "Int" } else { "Real" };
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of {kind}.\n");
    for i in 0..rows * cols {
        let v = if int {
            format!("{}", (i * 37 + 11) % 1000)
        } else {
            format!("{:?}", val(i * 31 + 5))
        };
        s += &format!("Set item {} of {name} to {v}.\n", i + 1);
    }
    s
}

/// The indices `idx` as an Int tensor, a row (`1 × B`) or a column (`B × 1`).
fn indices(idx: &[usize], column: bool) -> String {
    let (r, c) = if column {
        (idx.len(), 1)
    } else {
        (1, idx.len())
    };
    let mut s = format!("Let I be a {r} by {c} tensor of Int.\n");
    for (k, i) in idx.iter().enumerate() {
        s += &format!("Set item {} of I to {i}.\n", k + 1);
    }
    s
}

fn show_all(name: &str, n: usize) -> String {
    (1..=n)
        .map(|i| format!("Show item {i} of {name}.\n"))
        .collect()
}

/// `X` = rows `I` of `T`, every element shown.
fn block(rows: usize, cols: usize, int: bool, idx: &[usize], column: bool) -> String {
    let mut s = String::from("## Main\n");
    s += &table("T", rows, cols, int);
    s += &indices(idx, column);
    s += "Let X be rows I of T.\n";
    s += &show_all("X", idx.len() * cols);
    s
}

/// The block as its rows: `row I[k] of T` one at a time.
fn one_by_one(rows: usize, cols: usize, int: bool, idx: &[usize]) -> String {
    let mut s = String::from("## Main\n");
    s += &table("T", rows, cols, int);
    for (k, i) in idx.iter().enumerate() {
        s += &format!("Let x{k} be row {i} of T.\n");
    }
    for k in 0..idx.len() {
        s += &show_all(&format!("x{k}"), cols);
    }
    s
}

// ───────────────────────────── 1. the rows it gathers ─────────────────────────────

#[test]
fn row_k_of_the_block_is_row_i_k_of_the_table() {
    for (rows, cols, int) in [(9, 4, false), (9, 4, true), (6, 3, false)] {
        for idx in [vec![3, 1, 9.min(rows), 3], vec![2], vec![rows, 1]] {
            let want = ran(&one_by_one(rows, cols, int, &idx), "one by one");
            for column in [false, true] {
                let got = ran(
                    &block(rows, cols, int, &idx, column),
                    &format!("{rows}×{cols} rows {idx:?}"),
                );
                assert_eq!(
                    got,
                    want,
                    "{rows}×{cols} (Int {int}) rows {idx:?} ({}): row k is row I[k]",
                    if column { "column" } else { "row" }
                );
            }
        }
    }
}

// ───────────────────────────── 2. what is refused ─────────────────────────────

#[test]
fn an_index_outside_the_table_fails_with_one_message_on_every_tier() {
    for bad in [0usize, 10] {
        let err = tiers(&block(9, 4, false, &[2, bad, 5], false))
            .1
            .unwrap_or_else(|| panic!("index {bad} must fail"));
        assert!(
            err.contains("out of range") && err.contains("T has 9 rows"),
            "index {bad}: {err}"
        );
    }
}

#[test]
fn indices_that_are_not_int_or_an_undeclared_table_are_refused_by_name() {
    let mut real = String::from("## Main\n");
    real += &table("T", 9, 4, false);
    real += "Let I be a 1 by 2 tensor of Real.\nLet X be rows I of T.\nShow item 1 of X.\n";
    let err = tiers(&real)
        .1
        .unwrap_or_else(|| panic!("Real indices must be refused"));
    assert!(err.contains("rows I of T") && err.contains("Int"), "{err}");
    let mut missing = String::from("## Main\n");
    missing += &indices(&[1], false);
    missing += "Let X be rows I of U.\nShow item 1 of X.\n";
    let err = tiers(&missing)
        .1
        .unwrap_or_else(|| panic!("an undeclared table must be refused"));
    assert!(err.contains('U'), "{err}");
}

// ───────────────────────────── 3. the SN50 gathers the block in one section ─────────────────────────────

fn sn50() -> SpatialTarget {
    SpatialTarget::sn50_rdu()
}

/// The SN50 with fp32 datapaths: fp32 in place of bf16 on the matmul, fp32 beside it on the pointwise and row units.
fn fp32_twin() -> SpatialTarget {
    let mut t = sn50();
    for c in &mut t.compute {
        match c {
            ComputeCap::MatmulTile { precisions, .. } => {
                if precisions.remove(&Precision::Bf16) {
                    precisions.insert(Precision::Fp32);
                }
            }
            ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } => {
                if precisions.contains(&Precision::Bf16) {
                    precisions.insert(Precision::Fp32);
                }
            }
            _ => {}
        }
    }
    t.precisions.insert(Precision::Fp32);
    t
}

fn composed(src: &str, target: &SpatialTarget, what: &str) -> (SpatialProgram, usize) {
    let pl = compile_program_to_spatial(src, target)
        .unwrap_or_else(|e| panic!("{what}: walk refused: {e:?}"));
    let sp = pl
        .whole()
        .unwrap_or_else(|| panic!("{what}: not lowered whole: {}", pl.refusal()))
        .clone();
    assert_eq!(sp.phase(), Phase::Verified, "{what}: the program certifies");
    assert!(
        sp.certificate.as_ref().is_some_and(|c| c.recheck(&sp)),
        "{what}: its certificate rechecks"
    );
    (sp, pl.sections)
}

fn rounded(src: &str) -> String {
    src.lines()
        .map(|l| {
            match l
                .strip_prefix("Show item ")
                .and_then(|r| r.strip_suffix('.'))
            {
                Some(item) => format!("Show round(item {item} * 1000000).\n"),
                None => format!("{l}\n"),
            }
        })
        .collect()
}

/// The program's shown values, its indices replaced by `idx` when given.
fn shown(src: &str, sp: &SpatialProgram, idx: Option<&[usize]>, what: &str) -> Vec<i64> {
    let mut sp = sp.clone();
    if let Some(idx) = idx {
        let t = sp
            .tensors
            .iter_mut()
            .find(|t| t.name == "I")
            .unwrap_or_else(|| panic!("{what}: the indices are a tensor of the program"));
        t.init = TensorInit::Values(idx.iter().map(|i| *i as i64).collect());
    }
    let out = eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills"))
        .unwrap_or_else(|e| panic!("{what}: eval: {e:?}"));
    shown_from_outputs(src, &sp, &out).expect("shown")
}

/// The block's shapes: whole-tile rows and rows narrower than a tile; a few indices and more than a tile of them.
fn shapes() -> Vec<(usize, usize, Vec<usize>)> {
    let many: Vec<usize> = (0..40).map(|k| (k * 7) % 50 + 1).collect();
    vec![
        (50, 64, vec![7, 1, 50]),
        (50, 64, many.clone()),
        (9, 4, vec![3, 1, 9, 3]),
        (50, 4, many),
    ]
}

#[test]
fn the_block_gather_lowers_on_sn50_to_one_section_and_gathers_the_vms_rows() {
    for (rows, cols, idx) in shapes() {
        let what = format!("{rows}×{cols}, {} indices", idx.len());
        let src = rounded(&block(rows, cols, false, &idx, false));
        let want = oracle_ints(&ran(&src, &what).join("\n")).expect("ints");
        for (target, name) in [(fp32_twin(), "fp32 twin"), (sn50(), "bf16 chip")] {
            let (sp, sections) = composed(&src, &target, &format!("{what} on the {name}"));
            assert_eq!(sections, 1, "{what}: the block is one section");
            // Every value is a multiple of 1/64 in (−1, 1]: exact in bf16 and fp32 alike, so a gather matches exactly.
            assert_eq!(
                shown(&src, &sp, None, &what),
                want,
                "{what}: the {name} gathers the VM's rows"
            );
        }
    }
}

// ───────────────────────────── 4. the indices are data ─────────────────────────────

#[test]
fn one_compiled_gather_fetches_whatever_rows_it_is_handed() {
    for (rows, cols, idx) in shapes() {
        let src = rounded(&block(rows, cols, false, &idx, false));
        let (sp, _) = composed(&src, &sn50(), &format!("{rows}×{cols}"));
        let other: Vec<usize> = idx.iter().rev().map(|i| rows + 1 - i).collect();
        let what = format!("{rows}×{cols} compiled for {idx:?}, run with {other:?}");
        let want =
            oracle_ints(&ran(&rounded(&block(rows, cols, false, &other, false)), &what).join("\n"))
                .expect("ints");
        assert_eq!(
            shown(&src, &sp, Some(&other), &what),
            want,
            "{what}: the indices are read at run time"
        );
    }
}

#[test]
fn an_index_outside_the_table_faults_at_its_guard() {
    for (rows, cols) in [(50usize, 64usize), (9, 4)] {
        let idx = [3usize, 1, 2];
        let (sp, _) = composed(
            &rounded(&block(rows, cols, false, &idx, false)),
            &sn50(),
            &format!("{rows}×{cols}"),
        );
        for bad in [0usize, rows + 1] {
            let mut t = sp.clone();
            t.tensors
                .iter_mut()
                .filter(|t| t.name == "I")
                .for_each(|t| t.init = TensorInit::Values(vec![3, bad as i64, 2]));
            match eval_choreography(&t, &inputs_from_fills(&t).expect("fills")) {
                Err(ChoreoFault::GatherOutOfRange { extent, .. }) => assert_eq!(extent, rows, "{rows}×{cols}: index {bad} is checked against the table's rows"),
                other => panic!("{rows}×{cols}: index {bad} is outside the table and must fault at its guard, not {other:?}"),
            }
        }
    }
}
