//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A STACKED BATCH IS A MATRIX — ITS ROW WRITES, ROW READS AND ROW NORMS AGREE WITH THE MATMUL THAT READS IT.
//!
//! Batched decode stacks `B` hidden states into a `B × d` matrix so every weight is read once for all of them. A
//! matmul reads that stack in `tile × tile` blocks; a row write (`Set row b of X to x`), a row read (`row b of Q`)
//! and a row norm (`rmsnorm of X with weight w`) touch the same tensor, and a composition joins them only when
//! all of them lay it out alike. The layout is decided once per table (`pipeline::dense_tables`): a tensor any op
//! reads or writes as a multi-row matrix is DENSE, and a row op addresses a dense table's row `r` as tile row
//! `⌊r / tile⌋`, in-tile row `r mod tile`.
//!
//! Pinned, each against the VM:
//!   1. A STACK WRITTEN ROW BY ROW, NORMALIZED AND MULTIPLIED emits the VM's values, every tensor at the program's
//!      shape.
//!   2. A ROW OF A MATRIX TALLER THAN A TILE ROW is read from its tile row — literal and named rows, both sides of
//!      the tile boundary, exactly.
//!   3. A ROW WRITTEN INTO A MATRIX TALLER THAN A TILE ROW lands in its tile row, read back whole, exactly.
//!   4. A STACK WIDER THAN A SITE HOLDS normalizes in the blocked sections over the matrix — the per-row sum a
//!      `B × d` by `d × 1` matmul.
//!   5. A ROW OUTSIDE A DENSE TABLE FAULTS, as it does on every tier.
//!   6. A PER-HEAD NORM KEEPS THE FLAT ROW: the stack rule changes nothing about a row viewed as heads.
//!   7. A TABLE WRITTEN ROW BY ROW IN SEVERAL SECTIONS — dense or flat — holds every row, each write ordered after
//!      the one before by the barrier between their sections, and cutting that barrier is refused (`StateOrder`).
//!   8. THE EVALUATOR HOLDS THOSE WRITES IN ORDER TOO: a read of the table hoisted between its first and second
//!      writes faults `StaleStateRead` under every scheduler, naming a write that had not run.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_stacked_rows_lock`
//!
//! ★ MUTATION PROBES (run 2026-09-25, each in a private copy of the workspace, each red):
//!   - `dense_tables` without its matmul arm → 1 (`TensorShapeConflict`: the stack flat, the matmul's operand not).
//!   - the row-norm tie in `dense_tables_with` disabled → 1, 7.
//!   - `dense_row`'s literal in-tile row fixed at 0 → 1, 2, 3, 7.
//!   - `dense_row_gather`'s in-tile row term at stride 0 → 2 (the named rows).
//!   - `validate_plan` checking a composed program against its first result and every item → 7 (`GridMismatch`).
//!   - carried-state tiles counted across the whole program → 1, 7, 8 (`OutputOverlap`).
//!   - `rescale_rows` reading row 0's scale for every row → 4.
//!   - the certificate without the write-after-write order → 7 (the whole-tile table, which no section reads).
//!   - the certificate refusing any second writer of carried state → 1, 7, 8.
//!   - `barriers_between` always holding → 7.
//!   - the evaluator knowing only a carried tensor's first writer → 8 (the half-written table is answered).
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::certificate::{certify_choreography, CertifyFault};
use logicaffeine_compile::spatial::eval::{
    eval_choreography, eval_choreography_with, ChoreoFault, Outputs, Scheduler, Value,
};
use logicaffeine_compile::spatial::ir::{ChoreoOp, SpatialProgram, StageKind, TaggedOp};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::{inputs_from_fills, without_barrier_after};

/// The stated bf16 bound for a few sections of float arithmetic, relative to the result's largest magnitude: each
/// bf16 rounding is within 2⁻⁹, and a norm then a matmul round at most eight times on any element's path.
const BF16_FEW_SECTIONS_REL: f64 = 1.0 / 64.0;

/// Bf16-exact values: `k / 64` for `|k| < 128`.
fn exact_at(salt: usize, i: usize) -> f64 {
    (((i * 37 + salt * 11) % 61) as f64 - 30.0) / 64.0
}

fn declare(name: &str, rows: usize, cols: usize, salt: usize) -> String {
    declare_scaled(name, rows, cols, salt, |_| 1.0)
}

/// [`declare`] with row `r` (0-based) scaled by `scale(r)` — rows whose magnitudes differ, so a per-row quantity
/// taken from the wrong row cannot hide inside a tolerance.
fn declare_scaled(
    name: &str,
    rows: usize,
    cols: usize,
    salt: usize,
    scale: fn(usize) -> f64,
) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    for i in 1..=rows * cols {
        s.push_str(&format!(
            "Set item {i} of {name} to {:?}.\n",
            exact_at(salt, i) * scale((i - 1) / cols)
        ));
    }
    s
}

fn composed(src: &str) -> SpatialProgram {
    let pl = compile_program_to_spatial(src, &RduChip::Sn50.facts().spatial_target())
        .unwrap_or_else(|e| panic!("the walk lifts it: {e:?}"));
    let p = pl
        .whole()
        .unwrap_or_else(|| panic!("the program lowers whole: {}", pl.refusal()))
        .clone();
    assert!(
        p.certificate.as_ref().is_some_and(|c| c.recheck(&p)),
        "the composed program certifies"
    );
    p
}

fn evaluated(p: &SpatialProgram) -> Result<Outputs, ChoreoFault> {
    eval_choreography(p, &inputs_from_fills(p))
}

fn tensor(p: &SpatialProgram, out: &Outputs, name: &str) -> Vec<f64> {
    let id = p
        .tensors
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("{name} is a tensor of the program"))
        .id;
    match out
        .tensors
        .get(&id)
        .unwrap_or_else(|| panic!("{name} is written"))
    {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

fn shape(p: &SpatialProgram, name: &str) -> (usize, usize) {
    let t = p
        .tensors
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("{name} is a tensor of the program"));
    (t.elems.rows, t.elems.cols)
}

/// Every element of `names` the VM shows, in order.
fn vm_values(src: &str, names: &[(&str, usize)]) -> Vec<Vec<f64>> {
    let mut shown = src.to_string();
    for (name, n) in names {
        for i in 1..=*n {
            shown.push_str(&format!("Show item {i} of {name}.\n"));
        }
    }
    let vm = vm_outcome(&shown);
    assert!(vm.error.is_none(), "the VM runs it: {:?}", vm.error);
    let all: Vec<f64> = vm
        .output
        .split_whitespace()
        .map(|t| t.parse().expect("a number"))
        .collect();
    let mut at = 0;
    names
        .iter()
        .map(|(_, n)| {
            let v = all[at..at + n].to_vec();
            at += n;
            v
        })
        .collect()
}

fn within(got: &[f64], want: &[f64], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: one value per element");
    let scale = want
        .iter()
        .fold(0.0f64, |m, x| m.max(x.abs()))
        .max(f64::MIN_POSITIVE);
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!((g - w).abs() <= BF16_FEW_SECTIONS_REL * scale, "{what}[{i}]: {g} vs the VM's {w} (bound {} of {scale})\n  ours: {got:?}\n  vm:   {want:?}", BF16_FEW_SECTIONS_REL);
    }
}

/// ── 1. A STACK WRITTEN ROW BY ROW, NORMALIZED AND MULTIPLIED: three rows of eight, the tiny batch's shape.
#[test]
fn a_stack_written_row_by_row_normalizes_and_multiplies_as_the_vm() {
    let mut src = String::from("## Main\n");
    src += &declare("X", 3, 8, 1);
    src += &declare("W", 8, 16, 2);
    src += &declare("g", 1, 8, 3);
    for b in 0..3 {
        src += &declare(&format!("x{b}"), 1, 8, 4 + b);
        src += &format!("Set row {} of X to x{b}.\n", b + 1);
    }
    src += "Let Xn be rmsnorm of X with weight g.\nLet Y be Xn matmul W.\nLet Z be Y plus Y.\n";
    let p = composed(&src);
    for (name, want) in [
        ("X", (3, 8)),
        ("Xn", (3, 8)),
        ("Y", (3, 16)),
        ("Z", (3, 16)),
    ] {
        assert_eq!(
            shape(&p, name),
            want,
            "{name} is laid out at the program's shape"
        );
    }
    let out = evaluated(&p).expect("the stack evaluates");
    let vm = vm_values(&src, &[("X", 24), ("Z", 48)]);
    assert_eq!(
        tensor(&p, &out, "X"),
        vm[0],
        "the stacked rows are the VM's, exactly"
    );
    within(&tensor(&p, &out, "Z"), &vm[1], "Z");
}

/// A `40 × 8` matrix — taller than one tile row — that a pointwise op reads whole, so it is dense.
fn tall(rows_src: &str) -> String {
    format!(
        "## Main\n{}{}Let Q be X plus X.\n{rows_src}",
        declare("X", 40, 8, 5),
        declare("s", 1, 8, 6)
    )
}

/// ── 2. A ROW OF A MATRIX TALLER THAN A TILE ROW, read from its tile row: rows either side of the boundary.
#[test]
fn a_row_of_a_tall_matrix_is_read_from_its_tile_row() {
    for r in [1usize, 2, 32, 33, 40] {
        for named in [false, true] {
            let read = if named {
                format!("Let r be {r}.\nLet y be row r of Q.\n")
            } else {
                format!("Let y be row {r} of Q.\n")
            };
            let src = tall(&read);
            let p = composed(&src);
            assert_eq!(shape(&p, "Q"), (40, 8), "Q is dense");
            let out = evaluated(&p).unwrap_or_else(|e| panic!("row {r}: {e:?}"));
            let vm = vm_values(&src, &[("y", 8)]);
            assert_eq!(
                tensor(&p, &out, "y"),
                vm[0],
                "row {r} ({}): the VM's row, exactly",
                if named { "named" } else { "literal" }
            );
        }
    }
}

/// ── 3. A ROW WRITTEN INTO A MATRIX TALLER THAN A TILE ROW lands in its tile row, read back whole.
#[test]
fn a_row_written_into_a_tall_matrix_lands_in_its_tile_row() {
    for r in [1usize, 31, 32, 33, 40] {
        for named in [false, true] {
            let write = if named {
                format!("Let r be {r}.\nSet row r of X to s.\n")
            } else {
                format!("Set row {r} of X to s.\n")
            };
            let src = format!(
                "## Main\n{}{}{write}Let Q be X plus X.\n",
                declare("X", 40, 8, 5),
                declare("s", 1, 8, 6)
            );
            let p = composed(&src);
            assert_eq!(shape(&p, "X"), (40, 8), "X is dense");
            let out = evaluated(&p).unwrap_or_else(|e| panic!("row {r}: {e:?}"));
            let vm = vm_values(&src, &[("Q", 320)]);
            assert_eq!(
                tensor(&p, &out, "Q"),
                vm[0],
                "row {r} ({}): every element the VM's, exactly",
                if named { "named" } else { "literal" }
            );
        }
    }
}

/// ── 4. A STACK WIDER THAN A SITE HOLDS normalizes in the blocked sections: two rows of 320 (ten tiles each, past
/// the eight a site accumulates), the fused residual included — the second row eight times the first, so each row's
/// scale is its own.
#[test]
fn a_wide_stack_normalizes_in_the_blocked_sections_over_the_matrix() {
    let src = format!(
        "## Main\n{}{}{}Let Xn be rmsnorm of X plus R with weight g.\nLet Z be Xn plus Xn.\n",
        declare_scaled("X", 2, 320, 7, |r| if r == 0 { 1.0 } else { 8.0 }),
        declare_scaled("R", 2, 320, 8, |r| if r == 0 { 1.0 } else { 8.0 }),
        declare("g", 1, 320, 9)
    );
    let p = composed(&src);
    assert_eq!(
        shape(&p, "Xn"),
        (2, 320),
        "the normalized stack is a matrix"
    );
    let sums: Vec<_> = p
        .stages
        .iter()
        .filter_map(|s| match &s.kind {
            StageKind::Compute(logicaffeine_compile::spatial::ir::ComputeOp::MatmulTiles {
                ..
            }) => Some(s.name.clone()),
            _ => None,
        })
        .collect();
    assert!(
        !sums.is_empty(),
        "the row sums are a matmul — the blocked form, not the resident row"
    );
    assert_eq!(shape(&p, "Xn__ssq"), (2, 1), "one sum per row of the stack");
    let out = evaluated(&p).expect("the wide stack evaluates");
    let vm = vm_values(&src, &[("Z", 640)]);
    within(&tensor(&p, &out, "Z"), &vm[0], "Z");
}

/// ── 5. A ROW OUTSIDE A DENSE TABLE FAULTS — read or written — where every tier's run-time check fails.
#[test]
fn a_row_outside_a_dense_table_faults() {
    for r in [0i64, 41] {
        let read = tall(&format!("Let r be {r}.\nLet y be row r of Q.\n"));
        let write = format!(
            "## Main\n{}{}Let r be {r}.\nSet row r of X to s.\nLet Q be X plus X.\n",
            declare("X", 40, 8, 5),
            declare("s", 1, 8, 6)
        );
        for (what, src) in [("read", read), ("write", write)] {
            let p = composed(&src);
            let got = evaluated(&p);
            assert!(
                got.is_err(),
                "row {r} ({what}): an out-of-range row must fault, not return {:?}",
                got.map(|_| "a result")
            );
            let vm = vm_outcome(&src);
            assert!(vm.error.is_some(), "row {r} ({what}): the VM fails too");
        }
    }
}

/// ── 6. A PER-HEAD NORM KEEPS THE FLAT ROW beside a stacked norm in the same program.
#[test]
fn a_per_head_norm_keeps_the_flat_row_beside_a_stacked_norm() {
    let src = format!(
        "## Main\n{}{}{}{}Let qn be rmsnorm of q with weight h across 2 heads.\nLet Xn be rmsnorm of X with weight g.\nLet u be qn plus qn.\nLet Z be Xn plus Xn.\n",
        declare("q", 1, 8, 10),
        declare("h", 1, 4, 11),
        declare("X", 2, 8, 12),
        declare("g", 1, 8, 13)
    );
    let p = composed(&src);
    assert_eq!(
        shape(&p, "qn"),
        (1, 8),
        "the per-head norm's result is the flat row the program binds"
    );
    assert_eq!(
        shape(&p, "Xn"),
        (2, 8),
        "the stacked norm's result is the matrix the program binds"
    );
    let out = evaluated(&p).expect("both norms evaluate");
    let vm = vm_values(&src, &[("u", 8), ("Z", 16)]);
    within(&tensor(&p, &out, "u"), &vm[0], "u");
    within(&tensor(&p, &out, "Z"), &vm[1], "Z");
}

/// ── 7. A TABLE WRITTEN ROW BY ROW IN SEVERAL SECTIONS holds every row — a narrow table no matrix reads (flat) and a
/// stack a norm reads (dense) — and each later write is ordered after the earlier by a barrier the certificate
/// demands: with the chain between two writing sections cut, the program does not certify.
#[test]
fn a_table_written_row_by_row_holds_every_row_and_orders_its_writes() {
    let flat = format!(
        "## Main\n{}{}{}Set row 1 of T to a.\nSet row 3 of T to b.\nSet row 1 of T to b.\n",
        declare("T", 3, 2, 14),
        declare("a", 1, 2, 15),
        declare("b", 1, 2, 16)
    );
    let dense = format!(
        "## Main\n{}{}{}{}Set row 2 of X to a.\nSet row 3 of X to b.\nLet Xn be rmsnorm of X with weight g.\nLet Z be Xn plus Xn.\n",
        declare("X", 3, 8, 17),
        declare("a", 1, 8, 18),
        declare("b", 1, 8, 19),
        declare("g", 1, 8, 20)
    );
    // Whole-tile rows are written by moving tiles, never reading the table: only the order between the writes
    // themselves stands between two sections storing the same row.
    let whole_tile = format!(
        "## Main\n{}{}{}Set row 1 of U to a.\nSet row 3 of U to b.\nSet row 1 of U to b.\n",
        declare("U", 3, 32, 21),
        declare("a", 1, 32, 22),
        declare("b", 1, 32, 23)
    );
    for (what, src, table, n) in [
        ("flat", &flat, "T", 6),
        ("dense", &dense, "X", 24),
        ("whole-tile", &whole_tile, "U", 96),
    ] {
        let p = composed(src);
        let out = evaluated(&p).unwrap_or_else(|e| panic!("{what}: {e:?}"));
        let vm = vm_values(src, &[(table, n)]);
        assert_eq!(
            tensor(&p, &out, table),
            vm[0],
            "{what}: every row the VM's, the last write standing"
        );
        let t = p
            .tensors
            .iter()
            .find(|t| t.name == table)
            .expect("the table")
            .id;
        let mut writers: Vec<usize> = p
            .stages
            .iter()
            .filter(|s| matches!(&s.kind, StageKind::Store { tensor, .. } if *tensor == t))
            .filter_map(|s| p.section_of_stage(s.id))
            .collect();
        writers.sort_unstable();
        writers.dedup();
        assert!(
            writers.len() >= 2,
            "{what}: the table is written by more than one section ({writers:?})"
        );
        let cut = without_barrier_after(&p, writers[0]);
        assert!(
            cut.validate().is_ok(),
            "{what}: the cut program is well-formed, so what refuses it is the ordering: {:?}",
            cut.validate()
        );
        match certify_choreography(&cut) {
            Err(CertifyFault::StateOrder { reason, .. }) => {
                if what == "whole-tile" {
                    assert!(reason.contains("two sections that write"), "{what}: no section reads the table, so the order between its writes is what is missing: `{reason}`");
                }
            }
            other => panic!("{what}: two writes of one table with no barrier between them must be refused `StateOrder`, not {:?}", other.map(|_| "a certificate")),
        }
    }
}

/// ── 8. THE EVALUATOR HOLDS A TABLE'S WRITES IN ORDER. A table written row by row in three sections and then read
/// whole: hoist the reader's load of the table to just after the FIRST write, ahead of the other two, and the run
/// must fault `StaleStateRead` — the tile it reads holds one write of three, which is no step's value. A rule that
/// knew only the table's first writer would see that writer done, and hand the half-written table on as the answer.
#[test]
fn a_read_hoisted_between_two_row_writes_faults_rather_than_answering() {
    let src = format!(
        "## Main\n{}{}{}Set row 1 of T to a.\nSet row 3 of T to b.\nSet row 1 of T to b.\nLet Z be T plus T.\n",
        declare("T", 3, 2, 24),
        declare("a", 1, 2, 25),
        declare("b", 1, 2, 26)
    );
    let ordered = composed(&src);
    let t = ordered
        .tensors
        .iter()
        .find(|t| t.name == "T")
        .expect("the table")
        .id;
    let section_of = |p: &SpatialProgram, stage| {
        p.section_of_stage(stage)
            .expect("a composed program's stages are sectioned")
    };
    let mut writers: Vec<usize> = ordered
        .stages
        .iter()
        .filter(|s| matches!(&s.kind, StageKind::Store { tensor, .. } if *tensor == t))
        .map(|s| section_of(&ordered, s.id))
        .collect();
    writers.sort_unstable();
    writers.dedup();
    assert_eq!(writers.len(), 3, "each row write is its own section");
    let reader = section_of(
        &ordered,
        ordered
            .stages
            .iter()
            .rev()
            .find(|s| matches!(&s.kind, StageKind::Load { tensor, .. } if *tensor == t))
            .expect("Z reads the table")
            .id,
    );
    assert!(
        reader > writers[2],
        "the whole-table read comes after every write"
    );

    let mut hoisted = ordered.clone();
    let programs = &mut hoisted.schedule.as_mut().expect("scheduled").programs;
    let prog = programs
        .iter_mut()
        .find(|pr| {
            pr.ops.iter().any(|o| {
                matches!(o.op, ChoreoOp::ReadTiles { tensor, .. } if tensor == t)
                    && section_of(&ordered, o.stage) == reader
            })
        })
        .expect("an engine program reads the table for Z");
    let load = prog
        .ops
        .iter()
        .find(|o| {
            matches!(o.op, ChoreoOp::ReadTiles { tensor, .. } if tensor == t)
                && section_of(&ordered, o.stage) == reader
        })
        .unwrap()
        .stage;
    let block: Vec<TaggedOp> = prog
        .ops
        .iter()
        .filter(|o| o.stage == load)
        .cloned()
        .collect();
    prog.ops.retain(|o| o.stage != load);
    let at = prog
        .ops
        .iter()
        .position(|o| section_of(&ordered, o.stage) > writers[0])
        .expect("the second write's section is on this engine");
    prog.ops.splice(at..at, block);

    for sched in [
        Scheduler::RoundRobin,
        Scheduler::Reverse,
        Scheduler::MovementFirst,
        Scheduler::ComputeFirst,
        Scheduler::Seeded(7),
    ] {
        match eval_choreography_with(&hoisted, &inputs_from_fills(&hoisted), sched) {
            Err(ChoreoFault::StaleStateRead {
                read_in,
                written_in,
                ..
            }) => {
                assert_eq!(
                    read_in, reader,
                    "{sched:?}: the fault names the reading section"
                );
                assert!(
                    writers[1..].contains(&written_in),
                    "{sched:?}: and a write that had not run yet ({written_in})"
                );
            }
            other => panic!(
                "{sched:?}: a read between two writes of the table must fault, not {:?}",
                other.map(|_| "an answer")
            ),
        }
    }
}
