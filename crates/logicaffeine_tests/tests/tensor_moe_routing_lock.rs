//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! AN EXPERT MIXTURE SCORED BY SIGMOIDS, ITS WEIGHTS SCALING THE EXPERTS' INPUTS.
//!
//! Llama 4 scores each expert by the sigmoid of its own router logit, not a softmax over all of them, keeps the top
//! `K` without renormalizing, and multiplies the INPUT of each kept expert by its weight; the expert's output joins
//! the sum unweighted (llama.cpp's `weight_before_ffn`). The `moe` sugar takes the scoring as `by sigmoid` after
//! `choosing K of E` and the weighting as `weighting the inputs` after the renormalizing clause:
//!
//!   • the VM and the tree-walker compute the `f64` formula for every combination of the two clauses with the
//!     renormalizing one, with and without biases, on inputs where each clause moves the answer;
//!   • the compiled tiers (C, Rust) agree with the VM;
//!   • the tensor graph's evaluator computes each combination as the VM does;
//!   • a malformed clause is refused;
//!   • the CUDA forward and the CUDA lane never lower such a mixture as a softmax-weighted one.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane::{compile, LaneOptions};
use logicaffeine_compile::compile::{compile_to_cuda_forward, tw_outcome, vm_outcome};
use logicaffeine_compile::model::spec::{Ffn, Gating};
use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
use logicaffeine_compile::tensor_graph::eval::{eval, Data};
use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
use std::collections::BTreeMap;
use tiny_models::{write_checkpoint, Tiny};

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
fn br(e: usize) -> f64 {
    [0.3, -0.2, 0.05, 0.4][e]
}
fn bg(e: usize, o: usize) -> f64 {
    0.1 * ((e * 3 + o) % 5) as f64 - 0.2
}
fn bu(e: usize, o: usize) -> f64 {
    0.15 * ((e + o * 2) % 4) as f64 - 0.25
}
fn bd(e: usize, o: usize) -> f64 {
    0.2 * ((e * 2 + o * 3) % 5) as f64 - 0.3
}

/// The clauses a mixture is written with.
#[derive(Clone, Copy, Debug)]
struct Routing {
    sigmoid: bool,
    renormalize: bool,
    inputs: bool,
    biases: bool,
}

impl Routing {
    fn clauses(self) -> String {
        let mut s = String::new();
        if self.sigmoid {
            s.push_str(" by sigmoid");
        }
        if !self.renormalize {
            s.push_str(" without renormalizing");
        }
        if self.inputs {
            s.push_str(" weighting the inputs");
        }
        s
    }
}

/// The program: `X` and the weights filled with `Real` literals, `Y` bound by the `moe` sugar written with `r`'s
/// clauses (and every bias when `r.biases`), every entry of `Y` shown as `round(y·1e9)`.
fn program(r: Routing) -> String {
    program_with(r, &r.clauses())
}

fn program_with(r: Routing, clauses: &str) -> String {
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
    let weights = if r.biases {
        fill("Br", 1, E, &br);
        fill("Bg", E, F, &|i| bg(i / F, i % F));
        fill("Bu", E, F, &|i| bu(i / F, i % F));
        fill("Bd", E, D, &|i| bd(i / D, i % D));
        "router Wr plus Br gate Wg plus Bg up Wu plus Bu down Wd plus Bd"
    } else {
        "router Wr gate Wg up Wu down Wd"
    };
    s.push_str(&format!("Let Y be moe of X with {weights} choosing {K} of {E}{clauses}.\n"));
    for i in 1..=D {
        s.push_str(&format!("Show round(item {i} of Y * 1000000000).\n"));
    }
    s
}

/// The mixture in `f64`: each expert scored by the softmax over every logit or by its own logit's sigmoid, the top `K`
/// kept, each kept weight renormalized over the kept when `renormalize`, then either each kept expert's output summed
/// by its weight or its input scaled by its weight and its output summed as it is.
fn oracle(r: Routing) -> Vec<i64> {
    let b = |v: f64| if r.biases { v } else { 0.0 };
    let logits: Vec<f64> = (0..E).map(|e| (0..D).map(|t| X[t] * wr(t, e)).sum::<f64>() + b(br(e))).collect();
    let p: Vec<f64> = if r.sigmoid {
        logits.iter().map(|l| 1.0 / (1.0 + (-l).exp())).collect()
    } else {
        let m = logits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let z: f64 = logits.iter().map(|l| (l - m).exp()).sum();
        logits.iter().map(|l| (l - m).exp() / z).collect()
    };
    let mut order: Vec<usize> = (0..E).collect();
    order.sort_by(|&a, &c| p[c].partial_cmp(&p[a]).unwrap().then(a.cmp(&c)));
    let kept = &order[..K];
    let total: f64 = if r.renormalize { kept.iter().map(|&e| p[e]).sum() } else { 1.0 };
    let mut y = [0f64; D];
    for &e in kept {
        let w = p[e] / total;
        let scale = if r.inputs { w } else { 1.0 };
        let h: Vec<f64> = (0..F)
            .map(|o| {
                let g = scale * (0..D).map(|t| X[t] * wg(e, o, t)).sum::<f64>() + b(bg(e, o));
                let u = scale * (0..D).map(|t| X[t] * wu(e, o, t)).sum::<f64>() + b(bu(e, o));
                g / (1.0 + (-g).exp()) * u
            })
            .collect();
        for (o, slot) in y.iter_mut().enumerate() {
            let down = (0..F).map(|t| h[t] * wd(e, o, t)).sum::<f64>() + b(bd(e, o));
            *slot += if r.inputs { down } else { w * down };
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

/// Every combination of the three clauses, with and without biases.
fn every_routing() -> Vec<Routing> {
    let mut out = Vec::new();
    for bits in 0..16u8 {
        out.push(Routing { sigmoid: bits & 1 != 0, renormalize: bits & 2 != 0, inputs: bits & 4 != 0, biases: bits & 8 != 0 });
    }
    out
}

const LLAMA4: Routing = Routing { sigmoid: true, renormalize: false, inputs: true, biases: false };

#[test]
fn llama4s_mixture_scores_by_sigmoid_and_weighs_the_inputs() {
    close(&shown(&program(LLAMA4)), &oracle(LLAMA4), "by sigmoid without renormalizing weighting the inputs");
}

#[test]
fn every_combination_of_the_clauses_computes_its_formula() {
    for r in every_routing() {
        close(&shown(&program(r)), &oracle(r), &format!("{r:?}"));
    }
}

#[test]
fn each_clause_moves_the_answer_on_these_inputs() {
    let moved = |a: Routing, b: Routing| oracle(a).iter().zip(oracle(b)).any(|(x, y)| (x - y).abs() > 1000);
    for r in every_routing() {
        assert!(moved(r, Routing { sigmoid: !r.sigmoid, ..r }), "scoring by sigmoid moves {r:?}");
        assert!(moved(r, Routing { inputs: !r.inputs, ..r }), "weighting the inputs moves {r:?}");
        assert!(moved(r, Routing { renormalize: !r.renormalize, ..r }), "renormalizing moves {r:?}");
    }
}

#[test]
fn a_kept_expert_with_biases_is_the_only_one_its_input_weighting_reaches() {
    // With biases an expert's output on a zero input is not zero, so an expert left out must add nothing at all.
    let r = Routing { biases: true, ..LLAMA4 };
    close(&shown(&program(r)), &oracle(r), "biased");
}

#[test]
fn the_compiled_tiers_route_by_sigmoid_and_weigh_the_inputs() {
    for r in [LLAMA4, Routing { biases: true, renormalize: true, ..LLAMA4 }] {
        let src = program(r);
        let want = vm_outcome(&src).output;
        common::assert_c_output(&src, want.trim());
        common::assert_exact_output(&src, want.trim());
    }
}

#[test]
fn a_malformed_routing_clause_is_refused() {
    for bad in [" by sigmod", " by", " weighting the input", " weighting inputs", " weighting the outputs"] {
        let err = vm_outcome(&program_with(LLAMA4, bad)).error.unwrap_or_else(|| panic!("{bad:?} is refused"));
        assert!(err.contains("moe") || err.contains("sigmoid") || err.contains("inputs"), "{bad:?}: {err}");
    }
}

#[test]
fn the_cuda_forward_never_lowers_it_as_a_softmax_weighted_mixture() {
    // Renormalized, so the forward's refusal of `without renormalizing` is not what refuses them.
    let renormalized = Routing { renormalize: true, ..LLAMA4 };
    for r in [renormalized, Routing { inputs: false, ..renormalized }, Routing { sigmoid: false, ..renormalized }] {
        assert!(compile_to_cuda_forward(&program(r)).is_err(), "{r:?}: the CUDA forward's router is a softmax weighting outputs");
    }
}

#[test]
fn the_tensor_graph_evaluates_every_routing_as_the_vm_does() {
    for r in every_routing() {
        let src = program(r);
        let rp = recognize_stages(&src).expect("parses");
        assert!(rp.unrecognized.is_empty(), "{r:?}: {:?}", rp.unrecognized);
        let g = TensorGraph::lift(&rp, None, None, &|_| Storage::Values).unwrap_or_else(|e| panic!("{r:?}: the graph: {e}"));
        let env = eval(&g, &BTreeMap::new()).unwrap_or_else(|e| panic!("{r:?}: the evaluator refuses: {e}"));
        let Some(Data::Real(y)) = env.get("Y") else { panic!("{r:?}: the evaluator leaves no Y") };
        let got: Vec<i64> = y.iter().map(|v| (v * 1e9).round() as i64).collect();
        close(&got, &shown(&src), &format!("{r:?} on the tensor graph"));
    }
}

#[test]
fn the_cuda_lane_refuses_a_sigmoid_routing_by_name() {
    for gating in [Gating::SigmoidScaledInput, Gating::Sigmoid { bias: false }] {
        let mut spec = Tiny::Qwen3Moe.spec(16);
        for layer in &mut spec.layers {
            if let Ffn::Moe(m) = &mut layer.ffn {
                m.gating = gating.clone();
            }
        }
        let path = std::env::temp_dir().join(format!("moe-routing-{}-{gating:?}.gguf", std::process::id()));
        write_checkpoint(&spec, &path);
        let opts = LaneOptions { ctx: 16, knobs: CudaKnobs::default(), prefill: 1 };
        let err = compile(&spec, &path, &opts).err().unwrap_or_else(|| panic!("{gating:?}: the lane lowered a sigmoid routing"));
        let _ = std::fs::remove_file(&path);
        assert!(err.to_string().contains("sigmoids"), "{gating:?}: the lane names what it refuses: {err}");
    }
}
