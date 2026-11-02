//! A transpose of ANY shape reaches the SN50 on the program path, and computes exactly what the VM computes.
//!
//! A stage's result is placed by its ITERATION SPACE, so a flat `1 × n` result must iterate the flat output:
//! element `c = j·rows + i` of new row `j` reads `X[c·cols + j·(1 − rows·cols)]`, affine inside that row's
//! band. A matrix transpose is therefore `cols` bands of one read each, and a vector — a single row or a
//! single column, the same flat sequence either way — is one plain read.
//!
//! ★ MUTATION PROBE: iterate a 2-D `cols × rows` space instead (writing the transposed PLANE and hoping its
//! row-major read is the flat result) → `every_shape_transposes_exactly` reds on `1x6` with
//! `[500000, 0, 0, 0, 0, 0]`: the flat view reads only the plane's first row.
//!
//! A transpose moves values and computes none, so the check is EXACT: every value here is a multiple of 0.5
//! below 64, which bf16 holds exactly, and a wrong element is a mismatch, not a rounding.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_transpose_any_shape_lock`
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::ir::Phase;
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

/// A `rows × cols` Real tensor filled with `0.5·(p + 1)` at flat position `p`, transposed, every element shown.
fn program(rows: usize, cols: usize) -> String {
    let mut s = format!("## Main\nLet K be a {rows} by {cols} tensor of Real.\n");
    for p in 0..rows * cols {
        s += &format!("Set item {} of K to {:?}.\n", p + 1, 0.5 * (p + 1) as f64);
    }
    s += "Let Kt be transpose of K.\n";
    for p in 0..rows * cols {
        s += &format!("Show round(item {} of Kt * 1000000).\n", p + 1);
    }
    s
}

/// The spatial answer for a shape, the VM's, and the lowered program's name.
fn run(rows: usize, cols: usize) -> (Vec<i64>, Vec<i64>, String) {
    let src = program(rows, cols);
    let target = SpatialTarget::sn50_rdu();
    let pl = compile_program_to_spatial(&src, &target)
        .unwrap_or_else(|e| panic!("{rows}x{cols}: walk refused: {e:?}"));
    let sp = pl
        .whole()
        .unwrap_or_else(|| panic!("{rows}x{cols}: partial lowering: {}", pl.refusal()))
        .clone();
    assert_eq!(
        sp.phase(),
        Phase::Verified,
        "{rows}x{cols}: the composed program certifies"
    );
    let out = eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills"))
        .unwrap_or_else(|e| panic!("{rows}x{cols}: eval: {e:?}"));
    let spatial = shown_from_outputs(&src, &sp, &out).expect("shown");
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "{rows}x{cols}: VM: {:?}", vm.error);
    (
        spatial,
        oracle_ints(&vm.output).expect("vm ints"),
        sp.name.clone(),
    )
}

#[test]
fn every_shape_transposes_exactly() {
    let tile = SpatialTarget::sn50_rdu().tile.cols;
    let shapes = [
        (2, 3),
        (3, 2),
        (3, 5),
        (5, 3),
        (4, 4),
        (7, 2),
        (2, 7),
        (1, 6),
        (6, 1),
        (tile, 2),
        (3, 11),
    ];
    for (rows, cols) in shapes {
        let (spatial, vm, name) = run(rows, cols);
        assert_eq!(
            spatial.len(),
            rows * cols,
            "{rows}x{cols}: every element is shown"
        );
        assert_eq!(
            spatial, vm,
            "{rows}x{cols}: the transpose ({name}) disagrees with the VM"
        );
    }
}

/// The one compute stage of a lowered transpose, and how many band predicates its body carries.
fn band_count(rows: usize, cols: usize) -> usize {
    use logicaffeine_compile::spatial::ir::{ComputeOp, StageKind, TensorBody};
    fn selects(b: &TensorBody) -> usize {
        match b {
            TensorBody::Select {
                then, otherwise, ..
            } => 1 + selects(then) + selects(otherwise),
            _ => 0,
        }
    }
    let src = program(rows, cols);
    let pl = compile_program_to_spatial(&src, &SpatialTarget::sn50_rdu()).expect("walks");
    let sp = pl.whole().expect("whole").clone();
    let bodies: Vec<&TensorBody> = sp
        .stages
        .iter()
        .filter_map(|s| match &s.kind {
            StageKind::Compute(ComputeOp::Generic { body, .. }) => Some(body),
            _ => None,
        })
        .collect();
    assert_eq!(
        bodies.len(),
        1,
        "{rows}x{cols}: a transpose is ONE compute stage"
    );
    selects(bodies[0])
}

#[test]
fn a_vector_transpose_is_one_plain_read_and_a_matrix_has_one_band_per_new_row() {
    for (rows, cols) in [(1, 6), (6, 1)] {
        assert_eq!(
            band_count(rows, cols),
            0,
            "{rows}x{cols}: a vector transposes to itself — no bands"
        );
    }
    for (rows, cols) in [(2, 3), (3, 5), (4, 4), (7, 2)] {
        assert_eq!(
            band_count(rows, cols),
            cols - 1,
            "{rows}x{cols}: one band per new row ({cols} rows → {} predicates)",
            cols - 1
        );
    }
}
