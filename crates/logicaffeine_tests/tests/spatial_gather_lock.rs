//! The data-indexed ops on the SN50's program path hold the three promises the gather contract makes:
//!
//!   1. AN OUT-OF-RANGE INDEX FAULTS, NEVER WRAPS. `row i of T` reads `T` through an `IndirectTerm` whose
//!      extent is the table; a row below 1 or past the last is `ChoreoFault::GatherOutOfRange` in the
//!      evaluator — where every tier's run-time check fails — and never somebody else's row.
//!   2. A ROW WRITE CHECKS ITS ROW. `Set row i of T to S` only reads `S` where the row is selected, so an
//!      out-of-range row would select nothing and silently write nothing; its offset is checked first by a
//!      gather of stride zero, which faults the same way.
//!   3. INDEX ARITHMETIC IS EXACT. A stage whose operands are all integer runs on the integer datapath, even
//!      inside a float program: an `Int` table holding values past 2²⁴ (where `f32` stops holding every
//!      integer) gathers them exactly.
//!   4. THE OFFSET CANNOT WRAP. The row is guarded before `(row − 1)·cols` is formed, so a row whose offset
//!      would wrap the 32-bit index datapath into the table faults instead of reading another element.
//!
//! ★ MUTATION PROBES (each run 2026-09-23 — break, observe red, restore): see the bottom of this file.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_gather_lock`
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, ChoreoFault};
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram, TensorInit};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

fn composed(src: &str) -> SpatialProgram {
    let pl = compile_program_to_spatial(src, &SpatialTarget::sn50_rdu())
        .unwrap_or_else(|e| panic!("walk refused: {e:?}\n{src}"));
    let sp = pl
        .whole()
        .unwrap_or_else(|| panic!("partial lowering: {}\n{src}", pl.refusal()))
        .clone();
    assert_eq!(
        sp.phase(),
        Phase::Verified,
        "the composed program certifies:\n{src}"
    );
    sp
}

/// Evaluate a composed program over its recorded fills.
fn eval(sp: &SpatialProgram) -> Result<logicaffeine_compile::spatial::eval::Outputs, ChoreoFault> {
    eval_choreography(sp, &inputs_from_fills(sp).expect("deterministic fills"))
}

/// The spatial answer and the VM's, for a program that must succeed on both.
fn agree(src: &str) -> (Vec<i64>, Vec<i64>) {
    let sp = composed(src);
    let out = eval(&sp).unwrap_or_else(|e| panic!("eval: {e:?}\n{src}"));
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "VM: {:?}\n{src}", vm.error);
    (
        shown_from_outputs(src, &sp, &out).expect("shown"),
        oracle_ints(&vm.output).expect("vm ints"),
    )
}

fn gather_src(tok: i64) -> String {
    format!(
        "## Main\nLet Emb be a 3 by 2 tensor of Real.\n\
         Set item 1 of Emb to 1.0.\nSet item 2 of Emb to 2.0.\nSet item 3 of Emb to 3.0.\n\
         Set item 4 of Emb to 4.0.\nSet item 5 of Emb to 5.0.\nSet item 6 of Emb to 6.0.\n\
         Let tok be {tok}.\nLet x be row tok of Emb.\n\
         Show round(item 1 of x * 1000000).\nShow round(item 2 of x * 1000000).\n"
    )
}

fn set_row_src(p: i64) -> String {
    let mut s = format!(
        "## Main\nLet Kc be a 3 by 2 tensor of Real.\n\
         Let kp be a 1 by 2 tensor of Real.\nSet item 1 of kp to 7.0.\nSet item 2 of kp to 8.0.\n\
         Let p be {p}.\nSet row p of Kc to kp.\n"
    );
    for i in 1..=6 {
        s += &format!("Show round(item {i} of Kc * 1000000).\n");
    }
    s
}

// ── 1. an out-of-range index faults ──────────────────────────────────────────────────────────────────

#[test]
fn every_row_in_range_gathers_exactly() {
    for tok in 1..=3 {
        let (spatial, vm) = agree(&gather_src(tok));
        assert_eq!(spatial, vm, "row {tok}");
    }
}

#[test]
fn a_row_outside_the_table_faults_in_the_evaluator_and_never_wraps() {
    // The flat offset is `(tok − 1)·2`: row 0 is −2, row 4 is 6 — both outside `[0, 6)`.
    for (tok, offset) in [(0i64, -2.0f64), (4, 6.0)] {
        let sp = composed(&gather_src(tok));
        match eval(&sp) {
            Err(ChoreoFault::GatherOutOfRange { index, extent, .. }) => {
                assert_eq!(
                    (index, extent),
                    (offset, 6),
                    "row {tok}: the fault names the offending offset and the extent"
                );
            }
            other => panic!("row {tok}: an out-of-range gather must fault, not {other:?}"),
        }
        let vm = vm_outcome(&gather_src(tok));
        assert!(
            vm.error
                .as_deref()
                .is_some_and(|e| e.contains("row index out of range")),
            "row {tok}: the VM fails too: {:?}",
            vm.error
        );
    }
}

// ── 2. a row write checks its row ────────────────────────────────────────────────────────────────────

#[test]
fn every_row_write_in_range_is_exact() {
    for p in 1..=3 {
        let (spatial, vm) = agree(&set_row_src(p));
        assert_eq!(spatial, vm, "row {p}");
    }
}

#[test]
fn a_row_write_outside_the_table_faults_instead_of_writing_nothing() {
    for p in [0i64, 4] {
        let sp = composed(&set_row_src(p));
        match eval(&sp) {
            Err(ChoreoFault::GatherOutOfRange { extent, .. }) => {
                assert_eq!(extent, 6, "row {p}: checked against the whole table")
            }
            other => panic!("row {p}: an out-of-range row write must fault, not {other:?}"),
        }
    }
}

// ── 3. index arithmetic is exact ─────────────────────────────────────────────────────────────────────

#[test]
fn an_int_table_past_two_to_the_twenty_four_gathers_exactly() {
    // 2²⁴ + 1 is the first integer `f32` cannot hold; each value here is odd and above it.
    let big = [
        16_777_217i64,
        16_777_219,
        33_554_433,
        33_554_435,
        1_000_000_001,
        1_000_000_003,
    ];
    for tok in 1..=3 {
        let mut src = String::from("## Main\nLet T be a 3 by 2 tensor of Int.\n");
        for (i, v) in big.iter().enumerate() {
            src += &format!("Set item {} of T to {v}.\n", i + 1);
        }
        src += &format!(
            "Let tok be {tok}.\nLet x be row tok of T.\nShow item 1 of x.\nShow item 2 of x.\n"
        );
        let (spatial, vm) = agree(&src);
        assert_eq!(spatial, vm, "row {tok} of an Int table past 2^24");
        assert_eq!(
            spatial,
            vec![big[(tok as usize - 1) * 2], big[(tok as usize - 1) * 2 + 1]]
        );
    }
}

// ── 4. the offset cannot wrap ────────────────────────────────────────────────────────────────────────

/// On a 32-bit index datapath `(tok − 1)·cols` wraps once `tok` passes `INT_MAX / cols`, and a wrapped offset can
/// land INSIDE the table: row 1,431,655,767 of a 3-column table is offset `1,431,655,766·3 = 2³² + 2`, which wraps
/// to 2 — item 3, somebody else's element. The row is guarded before the multiply, so that index faults.
#[test]
fn a_row_index_whose_offset_would_wrap_faults_before_the_multiply() {
    let src = "## Main\nLet T be a 3 by 3 tensor of Int.\nSet item 3 of T to 33.\nLet tok be 2.\nLet x be row tok of T.\nShow item 1 of x.\n";
    let sp = composed(src);
    let mut adversarial = sp.clone();
    let wrap = 1_431_655_767i64;
    assert_eq!(
        ((wrap - 1) * 3) as i32,
        2,
        "the offset of this row wraps to element 2 in 32 bits"
    );
    for t in &mut adversarial.tensors {
        if t.name == "tok" {
            t.init = TensorInit::Values(vec![wrap]);
        }
    }
    let bound = (i32::MAX / 3) as usize;
    match eval(&adversarial) {
        Err(ChoreoFault::GatherOutOfRange { extent, .. }) => assert_eq!(
            extent,
            2 * bound,
            "caught by the guard, before any offset is formed"
        ),
        other => panic!("a row whose offset wraps must fault, not {other:?}"),
    }
}

// ★ MUTATION PROBES:
//   • `fuse_generic` without its read-where-written precondition (the consumer's slot map discarded
//     unconditionally) → `a_row_write_outside_the_table_faults_instead_of_writing_nothing` reds: `Set row 0`
//     returns `Ok` with the table untouched — the offset stage fused into the stride-0 check deleted the check.
//     (Observed 2026-09-23; that run is how the pass's missing precondition was found.)
//   • eval accepting an out-of-range gather index (no `GatherOutOfRange`) → both fault tests red: row 0 is
//     SILENTLY `[0.0, 0.0]`, and `Set row 0` returns the table untouched — exactly the wrong answers the
//     contract exists to refuse.
//   • the integer datapath rule reverted (every Generic accumulates in f32) → `an_int_table_past_two_to_the_
//     twenty_four_gathers_exactly` reds with `[16777216, 16777220]` for `[16777217, 16777219]`: f32 rounding.
//   • the row guard's stride-0 gather replaced by a plain read (2026-09-24) → `a_row_index_whose_offset_would_wrap_
//     faults_before_the_multiply` reds with `Ok([33, 0, 0])`: the wrapped offset gathered item 3 — another element.
