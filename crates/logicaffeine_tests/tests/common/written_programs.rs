//! Programs an author writes as the VM's own loops — a scatter, a novel activation, a novel mask, a novel score — built
//! from seeded tensors, and the check every suite holds them to: lowered whole for the SN50 and evaluated on its
//! spatial evaluator, every `Show` within the bf16 datapath's two percent of the largest value the VM prints.

use crate::tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, ChoreoFault};
use logicaffeine_compile::spatial::ir::{ComputeOp, SpatialProgram, StageKind};
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, RecognizedProgram};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;

/// A value in `[-1, 1]` in steps of 1/64 for element `i` of tensor `name`: every one exact in bf16.
pub fn seeded(name: &str, i: usize) -> f64 {
    let salt = name.bytes().fold(7u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64));
    let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29);
    ((h >> 40) % 129) as f64 / 64.0 - 1.0
}

/// `name` declared `rows by cols` of Real and filled with `value(i)`.
pub fn filled(name: &str, rows: usize, cols: usize, value: &dyn Fn(usize) -> f64) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    for i in 0..rows * cols {
        s.push_str(&format!("Set item {} of {name} to {:?}.\n", i + 1, value(i)));
    }
    s
}

/// `name` declared `rows by cols` of Int and filled with `values`, row by row.
pub fn ints_shaped(name: &str, rows: usize, cols: usize, values: &[i64]) -> String {
    assert_eq!(values.len(), rows * cols, "`{name}`: a value an element");
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Int.\n");
    for (k, v) in values.iter().enumerate() {
        s.push_str(&format!("Set item {} of {name} to {v}.\n", k + 1));
    }
    s
}

/// `name` declared `1 by n` of Int and filled with `values`.
pub fn ints(name: &str, values: &[i64]) -> String {
    ints_shaped(name, 1, values.len(), values)
}

/// Every entry of `show` (`n` of them) shown as `round(v·1e6)`.
pub fn shows(show: &str, n: usize) -> String {
    (1..=n).map(|i| format!("Show round(item {i} of {show} * 1000000).\n")).collect()
}

/// Every entry of `show` (`n` of them) shown as it is: an Int tensor's exact values.
pub fn shows_exact(show: &str, n: usize) -> String {
    (1..=n).map(|i| format!("Show item {i} of {show}.\n")).collect()
}

/// The front end's walk of `src`.
pub fn walk(src: &str) -> RecognizedProgram {
    recognize_stages(src).unwrap_or_else(|e| panic!("the walk refuses the program: {e:?}"))
}

/// The kinds the walk lifts, each with the statement it lifts.
pub fn lifted(src: &str) -> Vec<(&'static str, usize)> {
    walk(src).stages.iter().map(|s| (s.op.kind(), s.stmt_index)).collect()
}

/// `src` lowered whole for `target`, or why not.
pub fn lowered(src: &str, target: &SpatialTarget) -> Result<SpatialProgram, String> {
    let pl = compile_program_to_spatial(src, target).map_err(|e| format!("the walk: {e:?}"))?;
    pl.whole().cloned().ok_or_else(|| pl.refusal())
}

/// What the SN50 shows for `src` against what the VM shows, every value within the bf16 datapath's 2% of the largest;
/// with `exact`, every value the VM's to the last digit. The program the SN50 ran.
pub fn on_the_sn50(src: &str, what: &str, exact: bool) -> SpatialProgram {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "{what}: the VM: {:?}", vm.error);
    let want = oracle_ints(&vm.output).expect("the VM shows integers");
    let sp = lowered(src, &SpatialTarget::sn50_rdu()).unwrap_or_else(|why| panic!("{what}: lowered whole for the SN50: {why}"));
    let out = eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills")).unwrap_or_else(|e| panic!("{what}: {e:?}"));
    let got = shown_from_outputs(src, &sp, &out).expect("shown");
    assert_eq!(got.len(), want.len(), "{what}: one Show a value");
    let scale = want.iter().fold(1i64, |m, v| m.max(v.abs())) as f64;
    for (i, (g, v)) in got.iter().zip(&want).enumerate() {
        if exact {
            assert_eq!(g, v, "{what}: Show #{} on the SN50 is {g}, the VM's {v}", i + 1);
        } else {
            assert!(((g - v).abs() as f64) <= 0.02 * scale, "{what}: Show #{} on the SN50 is {g}, the VM's {v}", i + 1);
        }
    }
    sp
}

/// The fault the SN50's evaluation of `src` stops at: a program whose every tier fails at run time, lowered whole.
pub fn faults_on_the_sn50(src: &str, what: &str) -> ChoreoFault {
    let sp = lowered(src, &SpatialTarget::sn50_rdu()).unwrap_or_else(|why| panic!("{what}: lowered whole for the SN50: {why}"));
    match eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills")) {
        Ok(_) => panic!("{what}: the SN50 evaluates it to values where the VM fails"),
        Err(e) => e,
    }
}

/// The names of `p`'s compute stages that are not the general form: a product's tiles, a requant, a bias, a ReLU.
pub fn special_compute(p: &SpatialProgram) -> Vec<String> {
    p.stages
        .iter()
        .filter(|s| matches!(&s.kind, StageKind::Compute(op) if !matches!(op, ComputeOp::Generic { .. })))
        .map(|s| s.name.clone())
        .collect()
}
