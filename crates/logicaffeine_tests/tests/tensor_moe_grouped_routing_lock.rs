//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! DEEPSEEK-V3'S ROUTING: A SELECTION BIAS, GROUP LIMITS, A CLAMPED RENORMALIZATION AND A ROUTED SCALE.
//!
//! DeepSeek-V3 (and Kimi K2, GLM-4.5) scores each expert by its logit's sigmoid and picks with each score plus a
//! learned per-expert bias that never weighs; it first keeps the top `G` of `N` expert groups, a group scored by the sum
//! of its two best biased scores, and leaves every other group's experts out; then it keeps the top `K` of the rest,
//! renormalizes the kept unbiased scores — their sum clamped at 2⁻¹⁴, as llama.cpp's `build_moe_ffn` clamps it — and
//! multiplies them by the routed scale. The `moe` sugar takes the bias as `biased by Bs` after the scoring, the groups as
//! `within the top G of N groups`, and the scale as `scaled by S` after the renormalizing clause:
//!
//!   • the VM and the tree-walker compute the formula for every combination of the clauses with the scoring and the
//!     renormalizing one, on inputs where each clause moves the answer;
//!   • the bias changes which experts are kept and never their weight; a sum below 2⁻¹⁴ is clamped; the scale reaches
//!     a weight wherever it weighs, the input it scales included;
//!   • the compiled tiers (C, Rust) agree with the VM; the tensor graph's evaluator computes every combination as the
//!     VM does;
//!   • a malformed clause is refused by name;
//!   • the CUDA forward and the CUDA lane never lower such a mixture as a softmax weighting its experts' outputs.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane::{compile, LaneOptions};
use logicaffeine_compile::compile::{compile_to_cuda_forward, tw_outcome, vm_outcome};
use logicaffeine_compile::model::spec::Ffn;
use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
use logicaffeine_compile::tensor_graph::eval::{eval, Data};
use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
use std::collections::BTreeMap;
use tiny_models::{write_checkpoint, Tiny};

const D: usize = 3;
const E: usize = 8;
const F: usize = 2;
const K: usize = 3;
/// Four groups of two experts, the top two kept.
const GROUPS: usize = 4;
const KEPT: usize = 2;
const SCALE: f64 = 2.5;
const X: [f64; D] = [1.0, 0.5, -0.25];
/// The router logits `X·Wr`: under sigmoids the groups of experts 2–3 and 4–5 outrank the group of the best expert 0;
/// under the softmax the groups of 0–1 and 2–3 outrank that of 4–5.
const LOGITS: [f64; E] = [3.0, -3.0, 1.5, 1.4, 1.45, 1.42, -2.0, -2.5];
/// The selection bias: expert 6 lifted past every score, its group past every group.
const BS: [f64; E] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.75, 0.0];

fn row1(e: usize) -> f64 {
    0.25 * (e % 3) as f64 - 0.25
}
fn row2(e: usize) -> f64 {
    0.5 * (e % 2) as f64 - 0.25
}
fn wr(t: usize, e: usize) -> f64 {
    match t {
        0 => LOGITS[e] - 0.5 * row1(e) + 0.25 * row2(e),
        1 => row1(e),
        _ => row2(e),
    }
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

/// The clauses a mixture is written with.
#[derive(Clone, Copy, Debug)]
struct Routing {
    sigmoid: bool,
    bias: bool,
    groups: bool,
    renormalize: bool,
    scale: bool,
}

impl Routing {
    fn clauses(self) -> String {
        let mut s = String::new();
        if self.sigmoid {
            s.push_str(" by sigmoid");
        }
        if self.bias {
            s.push_str(" biased by Bs");
        }
        if self.groups {
            s.push_str(&format!(" within the top {KEPT} of {GROUPS} groups"));
        }
        if !self.renormalize {
            s.push_str(" without renormalizing");
        }
        if self.scale {
            s.push_str(&format!(" scaled by {SCALE:?}"));
        }
        s
    }
}

/// DeepSeek-V3's routing: every clause.
const DEEPSEEK_V3: Routing = Routing { sigmoid: true, bias: true, groups: true, renormalize: true, scale: true };

/// The inputs a program is written with: the selection bias, and the router's own bias (`router Wr plus Br`).
#[derive(Clone, Copy)]
struct Inputs {
    bs: [f64; E],
    br: Option<f64>,
}

const PLAIN: Inputs = Inputs { bs: BS, br: None };

/// The program: `X` and the weights filled with `Real` literals, `Y` bound by the `moe` sugar with `clauses`, every
/// entry of `Y` shown as `round(y·1e9)`.
fn program_with(inputs: Inputs, clauses: &str, tail: &str) -> String {
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
    fill("Bs", 1, E, &|i| inputs.bs[i]);
    let router = match inputs.br {
        Some(b) => {
            fill("Br", 1, E, &|_| b);
            "router Wr plus Br"
        }
        None => "router Wr",
    };
    s.push_str(&format!("Let Y be moe of X with {router} gate Wg up Wu down Wd choosing {K} of {E}{clauses}{tail}.\n"));
    for i in 1..=D {
        s.push_str(&format!("Show round(item {i} of Y * 1000000000).\n"));
    }
    s
}

fn program(r: Routing) -> String {
    program_with(PLAIN, &r.clauses(), "")
}

/// The least sum llama.cpp renormalizes over, 2⁻¹⁴.
const FLOOR: f64 = 6.103515625e-5;

/// The mixture in `f64`, as llama.cpp's `build_moe_ffn` computes it: the scores (a softmax over every logit, or each
/// logit's sigmoid); the selection scores, each plus its bias; with groups, each group scored by its two best selection
/// scores and every expert outside the top `KEPT` groups left out; the top `K` selection scores kept; their UNBIASED
/// scores the weights, renormalized over their sum clamped at `floor`, then scaled; each kept expert's output summed by
/// its weight, or — `weigh_inputs` — its input scaled by its weight and its output summed as it is.
fn oracle_with(r: Routing, inputs: Inputs, weigh_inputs: bool, floor: f64) -> Vec<i64> {
    let logits: Vec<f64> = (0..E).map(|e| (0..D).map(|t| X[t] * wr(t, e)).sum::<f64>() + inputs.br.unwrap_or(0.0)).collect();
    let p: Vec<f64> = if r.sigmoid {
        logits.iter().map(|l| 1.0 / (1.0 + (-l).exp())).collect()
    } else {
        let m = logits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let z: f64 = logits.iter().map(|l| (l - m).exp()).sum();
        logits.iter().map(|l| (l - m).exp() / z).collect()
    };
    let mut sel: Vec<f64> = (0..E).map(|e| p[e] + if r.bias { inputs.bs[e] } else { 0.0 }).collect();
    if r.groups {
        let per = E / GROUPS;
        let score = |g: usize| {
            let mut v: Vec<f64> = sel[g * per..(g + 1) * per].to_vec();
            v.sort_by(|a, b| b.partial_cmp(a).unwrap());
            v[0] + v.get(1).copied().unwrap_or(0.0)
        };
        let mut ranked: Vec<usize> = (0..GROUPS).collect();
        ranked.sort_by(|&a, &b| score(b).partial_cmp(&score(a)).unwrap().then(a.cmp(&b)));
        for &g in &ranked[KEPT..] {
            sel[g * per..(g + 1) * per].iter_mut().for_each(|v| *v = f64::NEG_INFINITY);
        }
    }
    let mut order: Vec<usize> = (0..E).collect();
    order.sort_by(|&a, &b| sel[b].partial_cmp(&sel[a]).unwrap().then(a.cmp(&b)));
    let kept = &order[..K];
    let total = if r.renormalize { kept.iter().map(|&e| p[e]).sum::<f64>().max(floor) } else { 1.0 };
    let scale = if r.scale { SCALE } else { 1.0 };
    let mut y = [0f64; D];
    for &e in kept {
        let w = p[e] / total * scale;
        let into = if weigh_inputs { w } else { 1.0 };
        let h: Vec<f64> = (0..F)
            .map(|o| {
                let g = into * (0..D).map(|t| X[t] * wg(e, o, t)).sum::<f64>();
                let u = into * (0..D).map(|t| X[t] * wu(e, o, t)).sum::<f64>();
                g / (1.0 + (-g).exp()) * u
            })
            .collect();
        for (o, slot) in y.iter_mut().enumerate() {
            let down = (0..F).map(|t| h[t] * wd(e, o, t)).sum::<f64>();
            *slot += if weigh_inputs { down } else { w * down };
        }
    }
    y.iter().map(|v| (v * 1e9).round() as i64).collect()
}

fn oracle(r: Routing) -> Vec<i64> {
    oracle_with(r, PLAIN, false, FLOOR)
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

/// Every combination of the scoring, the three clauses and the renormalizing one.
fn every_routing() -> Vec<Routing> {
    (0..32u8)
        .map(|b| Routing { sigmoid: b & 1 != 0, bias: b & 2 != 0, groups: b & 4 != 0, renormalize: b & 8 != 0, scale: b & 16 != 0 })
        .collect()
}

#[test]
fn deepseek_v3s_routing_computes_its_formula() {
    close(&shown(&program(DEEPSEEK_V3)), &oracle(DEEPSEEK_V3), &DEEPSEEK_V3.clauses());
}

#[test]
fn every_combination_of_the_clauses_computes_its_formula() {
    for r in every_routing() {
        close(&shown(&program(r)), &oracle(r), &format!("{r:?}"));
    }
}

#[test]
fn each_clause_moves_the_answer_on_these_inputs() {
    // Not vacuous: every clause, flipped from every combination, moves the formula by far more than `close`'s bound.
    let moved = |a: Routing, b: Routing| oracle(a).iter().zip(oracle(b)).any(|(x, y)| (x - y).abs() > 1000);
    for r in every_routing() {
        assert!(moved(r, Routing { bias: !r.bias, ..r }), "the selection bias moves {r:?}");
        assert!(moved(r, Routing { groups: !r.groups, ..r }), "the group limit moves {r:?}");
        assert!(moved(r, Routing { scale: !r.scale, ..r }), "the routed scale moves {r:?}");
        assert!(moved(r, Routing { renormalize: !r.renormalize, ..r }), "renormalizing moves {r:?}");
    }
}

#[test]
fn the_selection_bias_chooses_and_never_weighs() {
    // A bias the same for every expert moves every selection score and every group's score alike: the same experts are
    // kept, with the same weights, so the mixture is the unbiased one — on the VM, not only in the formula.
    let even = Inputs { bs: [0.625; E], br: None };
    for r in every_routing().into_iter().filter(|r| r.bias) {
        let biased = shown(&program_with(even, &r.clauses(), ""));
        let unbiased = shown(&program(Routing { bias: false, ..r }));
        assert_eq!(biased, unbiased, "{r:?}: an even bias changes nothing");
    }
}

#[test]
fn ties_keep_the_first_groups_and_the_first_experts() {
    // A router that reads nothing ties every score and every group's score: the first two groups are kept and, within
    // them, the first experts — the order the formula breaks ties in (score, then index), on every combination.
    let tied = |src: String| -> String {
        src.lines().filter(|l| !l.contains(" of Wr to ") && !l.contains(" of Bs to ")).map(|l| format!("{l}\n")).collect()
    };
    for r in every_routing() {
        let got = shown(&tied(program(r)));
        // Every score equal: the kept experts are the first `K` of the kept groups, each weighing the one score.
        let p = if r.sigmoid { 0.5 } else { 1.0 / E as f64 };
        let kept: Vec<usize> = (0..K).collect();
        let total = if r.renormalize { (K as f64 * p).max(FLOOR) } else { 1.0 };
        let w = p / total * if r.scale { SCALE } else { 1.0 };
        let mut y = [0f64; D];
        for &e in &kept {
            let h: Vec<f64> = (0..F)
                .map(|o| {
                    let g = (0..D).map(|t| X[t] * wg(e, o, t)).sum::<f64>();
                    let u = (0..D).map(|t| X[t] * wu(e, o, t)).sum::<f64>();
                    g / (1.0 + (-g).exp()) * u
                })
                .collect();
            for (o, slot) in y.iter_mut().enumerate() {
                *slot += w * (0..F).map(|t| h[t] * wd(e, o, t)).sum::<f64>();
            }
        }
        let want: Vec<i64> = y.iter().map(|v| (v * 1e9).round() as i64).collect();
        close(&got, &want, &format!("{r:?} with every score tied"));
    }
}

#[test]
fn a_renormalizing_sum_below_two_to_the_minus_fourteen_is_clamped() {
    // Every logit lowered by 14: the three kept sigmoids sum to about 2.4·10⁻⁵, under 2⁻¹⁴ = 6.1·10⁻⁵, so each weight is
    // its score over 2⁻¹⁴, not over the sum — a mixture far smaller than the unclamped renormalization's.
    let low = Inputs { bs: BS, br: Some(-14.0) };
    let every_score: f64 = (0..E).map(|e| 1.0 / (1.0 + (14.0 - LOGITS[e]).exp())).sum();
    assert!(every_score < FLOOR, "every score together is under 2⁻¹⁴, so any kept sum is: {every_score:e}");
    for r in [Routing { sigmoid: true, bias: false, groups: false, renormalize: true, scale: false }, DEEPSEEK_V3] {
        let want = oracle_with(r, low, false, FLOOR);
        close(&shown(&program_with(low, &r.clauses(), "")), &want, &format!("{r:?} under 2⁻¹⁴"));
        let unclamped = oracle_with(r, low, false, 0.0);
        assert!(want.iter().zip(&unclamped).any(|(a, b)| (a - b).abs() > 1000), "{r:?}: the clamp moves the mixture: {want:?} vs {unclamped:?}");
    }
}

#[test]
fn a_routed_scale_scales_a_weight_wherever_it_weighs() {
    // Weighting the inputs, the scaled weight scales each kept expert's input.
    for r in [Routing { renormalize: false, ..DEEPSEEK_V3 }, Routing { renormalize: false, sigmoid: false, ..DEEPSEEK_V3 }] {
        let src = program_with(PLAIN, &r.clauses(), " weighting the inputs");
        close(&shown(&src), &oracle_with(r, PLAIN, true, FLOOR), &format!("{r:?} weighting the inputs"));
    }
}

#[test]
fn the_compiled_tiers_route_as_the_vm_does() {
    let low = Inputs { bs: BS, br: Some(-14.0) };
    for src in [program(DEEPSEEK_V3), program(Routing { sigmoid: false, ..DEEPSEEK_V3 }), program_with(low, &DEEPSEEK_V3.clauses(), "")] {
        let want = vm_outcome(&src).output;
        common::assert_c_output(&src, want.trim());
        common::assert_exact_output(&src, want.trim());
    }
}

#[test]
fn a_malformed_clause_is_refused_by_name() {
    for (bad, names) in [
        (" by sigmoid biased by", "biased"),
        (" by sigmoid biased Bs", "biased"),
        (" by sigmoid within the top 2 of 3 groups", "groups"),
        (" by sigmoid within the top 5 of 4 groups", "groups"),
        (" by sigmoid within the top 0 of 4 groups", "groups"),
        (" by sigmoid within the top 1 of 4 groups", "groups"),
        (" by sigmoid within the 2 of 4 groups", "groups"),
        (" by sigmoid scaled by", "scaled"),
        (" by sigmoid scaled by 0", "scaled"),
        (" by sigmoid scaled by 2.5 without renormalizing", "moe"),
    ] {
        let err = vm_outcome(&program_with(PLAIN, bad, "")).error.unwrap_or_else(|| panic!("{bad:?} is refused"));
        assert!(err.contains(names), "{bad:?} names `{names}`: {err}");
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
fn the_cuda_forward_never_lowers_it_as_a_softmax_weighting_outputs() {
    // Softmax-scored and renormalized, so the forward's other refusals are not what refuses them.
    let softmax = Routing { sigmoid: false, bias: false, groups: false, renormalize: true, scale: false };
    for r in [Routing { bias: true, ..softmax }, Routing { groups: true, ..softmax }, Routing { scale: true, ..softmax }] {
        let err = compile_to_cuda_forward(&program(r)).err().unwrap_or_else(|| panic!("{r:?}: the CUDA forward lowered it"));
        assert!(err.to_string().contains("softmax weighting"), "{r:?}: the forward names what it refuses: {err}");
    }
}

#[test]
fn the_cuda_lane_refuses_a_grouped_or_scaled_routing_by_name() {
    // The tiny mixture's four experts in two groups of two, one kept; or its kept weights scaled.
    for (what, groups, scale) in [("a group limit", Some((2, 1)), 1.0), ("a routed scale", None, SCALE)] {
        let mut spec = Tiny::Qwen3Moe.spec(16);
        for layer in &mut spec.layers {
            if let Ffn::Moe(m) = &mut layer.ffn {
                m.groups = groups;
                m.weights_scale = scale;
            }
        }
        let path = std::env::temp_dir().join(format!("moe-grouped-{}-{}.gguf", std::process::id(), what.replace(' ', "-")));
        write_checkpoint(&spec, &path);
        let opts = LaneOptions { ctx: 16, knobs: CudaKnobs::default(), prefill: 1 };
        let err = compile(&spec, &path, &opts).err().unwrap_or_else(|| panic!("{what}: the lane lowered it"));
        let _ = std::fs::remove_file(&path);
        assert!(err.to_string().contains("softmax weighting"), "{what}: the lane names what it refuses: {err}");
    }
}
