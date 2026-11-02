//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A TENSOR SCALED BY A NUMBER.
//!
//! `Let Y be X scaled by C.` multiplies every element of `X` by the number `C` — the embedding scale Gemma reads its
//! token rows with (`√d`), and any other constant factor a model applies. `Y` has `X`'s shape and holds `Real`s.
//!
//!   • every tier computes `x·C` element by element, an `Int` tensor's elements too;
//!   • the tensor front end lifts the op with its factor (`ScaleSpec`);
//!   • a factor that is not a number, or a missing `by`, is refused by name;
//!   • the spatial IR lowers it as one multiply by the constant; the single-token CUDA forward, which has no kernel
//!     for it, refuses it by name.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;

use logicaffeine_compile::compile::{compile_to_cuda_forward, tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;

const XS: [f64; 6] = [0.5, -1.25, 3.0, 0.0, -0.75, 2.125];

/// `X` (2 by 3, of `Real` or of `Int`) filled, `Y` bound by `op`, every entry of `Y` shown as `round(y·1e6)`.
fn program(op: &str, int: bool) -> String {
    let ty = if int { "Int" } else { "Real" };
    let mut s = format!("## Main\nLet X be a 2 by 3 tensor of {ty}.\nLet W be a 2 by 3 tensor of Real.\n");
    for (i, x) in XS.iter().enumerate() {
        let v = if int { format!("{}", (x * 4.0) as i64) } else { format!("{x:?}") };
        s.push_str(&format!("Set item {} of X to {v}.\n", i + 1));
    }
    s.push_str(&format!("Let Y be {op}.\n"));
    for i in 1..=6 {
        s.push_str(&format!("Show round(item {i} of Y * 1000000).\n"));
    }
    s
}

fn shown(src: &str) -> Vec<i64> {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "VM error on:\n{src}\n{:?}", vm.error);
    let tw = tw_outcome(src);
    assert!(tw.error.is_none(), "tree-walker error: {:?}", tw.error);
    assert_eq!(tw.output.trim_end(), vm.output.trim_end(), "the tree-walker agrees with the VM");
    vm.output.split_whitespace().map(|t| t.parse().expect("an integer")).collect()
}

fn scaled(xs: impl Iterator<Item = f64>, c: f64) -> Vec<i64> {
    xs.map(|x| (x * c * 1e6).round() as i64).collect()
}

#[test]
fn every_element_is_multiplied_by_the_factor() {
    assert_eq!(shown(&program("X scaled by 2.5", false)), scaled(XS.iter().copied(), 2.5));
    assert_eq!(shown(&program("X scaled by 33.941125496954285", false)), scaled(XS.iter().copied(), 33.941125496954285));
    assert_eq!(shown(&program("X scaled by 1", false)), scaled(XS.iter().copied(), 1.0), "a whole-number factor");
}

#[test]
fn an_int_tensor_scales_to_reals() {
    let ints = XS.iter().map(|x| ((x * 4.0) as i64) as f64);
    assert_eq!(shown(&program("X scaled by 0.5", true)), scaled(ints, 0.5));
}

#[test]
fn the_compiled_tiers_scale() {
    let src = program("X scaled by -1.5", false);
    let want = vm_outcome(&src).output;
    assert_eq!(shown(&src), scaled(XS.iter().copied(), -1.5), "a negative factor");
    common::assert_c_output(&src, want.trim());
    common::assert_exact_output(&src, want.trim());
}

#[test]
fn the_front_end_lifts_the_scale_with_its_factor() {
    let rp = recognize_stages(&program("X scaled by 2.5", false)).expect("parses");
    assert!(rp.unrecognized.is_empty(), "{:?}", rp.unrecognized);
    let ops: Vec<TensorOpSpec> = rp.stages.into_iter().map(|s| s.op).collect();
    match ops.as_slice() {
        [TensorOpSpec::Scale(s)] => {
            assert_eq!(s.n, 6);
            assert_eq!(s.factor.get(), 2.5);
        }
        other => panic!("one scale is lifted, not {other:?}"),
    }
}

#[test]
fn a_factor_that_is_not_a_number_is_refused() {
    for (op, why) in [("X scaled by W", "a number"), ("X scaled 2.5", "`by`"), ("X scaled by", "a number")] {
        let err = vm_outcome(&program(op, false)).error.unwrap_or_else(|| panic!("`{op}` is refused"));
        assert!(err.contains("scaled by") && err.contains(why), "`{op}` is refused naming the {why}: {err}");
    }
}

#[test]
fn the_spatial_ir_lowers_the_scale_and_the_cuda_forward_refuses_it_by_name() {
    let src = program("X scaled by 2.5", false);
    let lowered = compile_program_to_spatial(&src, &SpatialTarget::wormhole()).expect("the spatial IR lowers a scale");
    let shown = format!("{lowered:?}");
    assert!(shown.contains("scale_2x3"), "one section multiplies the 2 by 3 tensor:\n{shown}");
    assert!(shown.contains(&format!("Mul(Operand(0), Const({}))", 2.5f64.to_bits())), "by the program's factor:\n{shown}");
    let forward = compile_to_cuda_forward(&src).expect_err("the CUDA forward has no scale kernel");
    assert!(forward.contains("Scale"), "the CUDA forward names the op it refuses: {forward}");
}
