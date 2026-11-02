//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! AN EXPERT MIXTURE WEIGHTED BY THE SOFTMAX OVER EVERY EXPERT.
//!
//! DeepSeek-V2 keeps its top `K` experts and weighs each by its softmax over all `E`, without renormalizing the kept
//! weights to sum to one (`norm_topk_prob` false). The `moe` sugar takes it as `without renormalizing` after
//! `choosing K of E`; with the clause absent the kept weights are renormalized, as every other mixture's are.
//!
//!   • every tier computes the `f64` formula, on inputs where the clause moves the answer;
//!   • the tensor front end lifts it into the op's spec (`MoeSpec::renormalize`), and an option-free mixture keeps it on;
//!   • the clause composes with the clamped activation, and a malformed clause is refused;
//!   • the spatial IR weighs by the whole softmax (`spatial_moe_options_lock` holds it to the VM), and the CUDA forward,
//!     whose router renormalizes, refuses it by name.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;

use logicaffeine_compile::compile::{compile_to_cuda_forward, tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;

const D: usize = 2;
const E: usize = 4;
const F: usize = 3;
const K: usize = 2;
const X: [f64; D] = [0.7, -1.1];

fn wr(t: usize, e: usize) -> f64 {
    [[0.9, -0.3, 0.4, 0.1], [-0.5, 0.8, 0.2, -0.6]][t][e]
}
fn wg(e: usize, o: usize, t: usize) -> f64 {
    0.6 * ((e * 7 + o * 3 + t * 5) % 9) as f64 / 4.0 - 0.7
}
fn wu(e: usize, o: usize, t: usize) -> f64 {
    0.5 * ((e * 5 + o * 7 + t * 3) % 8) as f64 / 3.0 - 0.6
}
fn wd(e: usize, o: usize, t: usize) -> f64 {
    0.4 * ((e * 3 + o * 5 + t * 7) % 7) as f64 / 2.0 - 0.5
}

/// The program: `X` and the weights filled with `Real` literals, `Y` bound by the `moe` sugar with `clauses` after
/// `choosing K of E`, every entry of `Y` shown as `round(y·1e9)`.
fn program(clauses: &str) -> String {
    let mut s = String::from("## Main\n");
    let mut fill = |name: &str, rows: usize, cols: usize, v: &dyn Fn(usize) -> f64| {
        s.push_str(&format!("Let {name} be a {rows} by {cols} tensor of Real.\n"));
        for i in 0..rows * cols {
            s.push_str(&format!("Set item {} of {name} to {:?}.\n", i + 1, v(i)));
        }
    };
    fill("X", 1, D, &|i| X[i]);
    fill("Wr", D, E, &|i| wr(i / E, i % E));
    fill("Wg", E * F, D, &|i| wg(i / (F * D), (i / D) % F, i % D));
    fill("Wu", E * F, D, &|i| wu(i / (F * D), (i / D) % F, i % D));
    fill("Wd", E * D, F, &|i| wd(i / (D * F), (i / F) % D, i % F));
    s.push_str(&format!("Let Y be moe of X with router Wr gate Wg up Wu down Wd choosing {K} of {E}{clauses}.\n"));
    for i in 1..=D {
        s.push_str(&format!("Show round(item {i} of Y * 1000000000).\n"));
    }
    s
}

/// The mixture in `f64`: the router's softmax, the top `K` kept — renormalized over the kept when `renormalize` — each
/// kept expert's SwiGLU through its down projection, summed by its weight.
fn oracle(renormalize: bool) -> Vec<i64> {
    let logits: Vec<f64> = (0..E).map(|e| (0..D).map(|t| X[t] * wr(t, e)).sum::<f64>()).collect();
    let m = logits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let z: f64 = logits.iter().map(|l| (l - m).exp()).sum();
    let p: Vec<f64> = logits.iter().map(|l| (l - m).exp() / z).collect();
    let mut order: Vec<usize> = (0..E).collect();
    order.sort_by(|&a, &b| p[b].partial_cmp(&p[a]).unwrap().then(a.cmp(&b)));
    let kept = &order[..K];
    let total: f64 = if renormalize { kept.iter().map(|&e| p[e]).sum() } else { 1.0 };
    let mut y = [0f64; D];
    for &e in kept {
        let h: Vec<f64> = (0..F)
            .map(|r| {
                let g = (0..D).map(|t| X[t] * wg(e, r, t)).sum::<f64>();
                let u = (0..D).map(|t| X[t] * wu(e, r, t)).sum::<f64>();
                g / (1.0 + (-g).exp()) * u
            })
            .collect();
        for (r, slot) in y.iter_mut().enumerate() {
            *slot += p[e] / total * (0..F).map(|t| h[t] * wd(e, r, t)).sum::<f64>();
        }
    }
    y.iter().map(|v| (v * 1e9).round() as i64).collect()
}

fn shown(src: &str) -> Vec<i64> {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "VM error on:\n{src}\n{:?}", vm.error);
    let tw = tw_outcome(src);
    assert!(tw.error.is_none(), "tree-walker error: {:?}", tw.error);
    assert_eq!(tw.output.trim_end(), vm.output.trim_end(), "the tree-walker agrees with the VM");
    vm.output.split_whitespace().map(|t| t.parse().expect("an integer")).collect()
}

fn close(got: &[i64], want: &[i64], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() <= 2, "{what}: {got:?} vs the formula's {want:?}");
    }
}

#[test]
fn each_kept_expert_weighs_its_softmax_over_every_expert() {
    let want = oracle(false);
    close(&shown(&program(" without renormalizing")), &want, "without renormalizing");
    let renormalized = oracle(true);
    close(&shown(&program("")), &renormalized, "the default renormalizes");
    assert!(want.iter().zip(&renormalized).any(|(a, b)| (a - b).abs() > 1000), "the clause moves the answer on these inputs");
}

#[test]
fn the_compiled_tiers_weigh_by_the_whole_softmax() {
    let src = program(" without renormalizing");
    let want = vm_outcome(&src).output;
    common::assert_c_output(&src, want.trim());
    common::assert_exact_output(&src, want.trim());
}

#[test]
fn the_front_end_lifts_the_weighting_into_the_spec() {
    let lifted = |clauses: &str| {
        let rp = recognize_stages(&program(clauses)).expect("parses");
        assert!(rp.unrecognized.is_empty(), "{:?}", rp.unrecognized);
        let st = rp.stages.into_iter().find(|s| matches!(s.op, TensorOpSpec::Moe(_))).expect("a mixture");
        let TensorOpSpec::Moe(m) = st.op else { unreachable!() };
        assert_eq!(&st.ins[..5], ["X", "Wr", "Wg", "Wu", "Wd"], "the operands, in order: {:?}", st.ins);
        m
    };
    assert!(!lifted(" without renormalizing").renormalize, "the clause is lifted");
    assert!(lifted("").renormalize, "a mixture without the clause renormalizes");
    let both = lifted(" without renormalizing using a swiglu clamped at 7.0 with slope 1.702");
    assert!(!both.renormalize && both.act != logicaffeine_compile::tensor_recognize::qwen::MoeAct::Swiglu, "it composes with the clamp");
    assert_eq!((both.d, both.dff, both.ne, both.k), (D, F, E, K));
}

#[test]
fn a_malformed_weighting_clause_is_refused() {
    let err = vm_outcome(&program(" without renormalising")).error.expect("a misspelled clause is refused");
    assert!(err.contains("without") || err.contains("moe"), "{err}");
}

#[test]
fn the_spatial_ir_lowers_the_weighting_and_the_cuda_forward_refuses_it_by_name() {
    let src = program(" without renormalizing");
    let spatial = compile_program_to_spatial(&src, &SpatialTarget::sn50_rdu()).expect("the tensor program walks");
    assert!(spatial.whole().is_some(), "the spatial IR lowers the weighting whole: {}", spatial.refusal());
    let forward = compile_to_cuda_forward(&src).expect_err("the CUDA forward's router renormalizes");
    assert!(forward.contains("without renormalizing"), "the CUDA forward names what it refuses: {forward}");
}
