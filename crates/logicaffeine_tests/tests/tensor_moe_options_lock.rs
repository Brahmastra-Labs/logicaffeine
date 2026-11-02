//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! AN EXPERT MIXTURE WITH BIASES AND A CLAMPED GATE.
//!
//! gpt-oss's mixture adds a bias to its router's logits and to each expert's gate, up and down projections, and gates
//! with a clamped SwiGLU: `min(g, L)·σ(A·min(g, L))·(clamp(u, −L, L) + 1)` (ggml's `swiglu_oai`, `A` 1.702, `L` 7).
//! The `moe` sugar takes each bias as `plus B` after its weight and the activation as
//! `using a swiglu clamped at L with slope A` after `choosing K of E`.
//!
//!   • every tier computes the `f64` formula, with every option and with each alone, on inputs where each moves the
//!     answer;
//!   • the tensor front end lifts the options into the op's spec (`MoeSpec::{router_bias, biases, act}`) and keeps
//!     `X, Wr, Wg, Wu, Wd` its operands;
//!   • each expert bias alone is lifted into its own projection's slot, whichever others are present;
//!   • a bias of the wrong shape, or a malformed activation clause, is refused by name;
//!   • the spatial IR lowers every option (`spatial_moe_options_lock` holds it to the VM), and the CUDA forward, whose
//!     mixture has no biases, refuses it by name.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;

use logicaffeine_compile::compile::{compile_to_cuda_forward, tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_recognize::qwen::MoeAct;

const D: usize = 2;
const E: usize = 3;
const F: usize = 3;
const K: usize = 2;
const X: [f64; D] = [0.7, -1.1];

fn wr(t: usize, e: usize) -> f64 {
    [[0.9, -0.3, 0.4], [-0.5, 0.8, 0.2]][t][e]
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
    [0.25, -0.6, 0.9][e]
}
fn bg(e: usize, o: usize) -> f64 {
    0.3 * (e as f64 - 1.0) + 0.2 * o as f64
}
fn bu(e: usize, o: usize) -> f64 {
    -0.25 * e as f64 + 0.35 * o as f64
}
fn bdn(e: usize, o: usize) -> f64 {
    0.15 * (e + 1) as f64 - 0.4 * o as f64
}

/// Which options a program takes.
#[derive(Clone, Copy, Default)]
struct Options {
    router_bias: bool,
    biases: bool,
    clamp: Option<(f64, f64)>,
}

/// The program: `X`, the weights, and every bias declared and filled with `Real` literals, `Y` bound by the `moe` sugar
/// with `options`, every entry of `Y` shown as `round(y·1e9)`.
fn program(o: Options) -> String {
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
    fill("Br", 1, E, &|i| br(i));
    fill("Bg", E, F, &|i| bg(i / F, i % F));
    fill("Bu", E, F, &|i| bu(i / F, i % F));
    fill("Bd", E, D, &|i| bdn(i / D, i % D));
    let b = |name: &str, on: bool| if on { format!(" plus {name}") } else { String::new() };
    let act = o.clamp.map_or(String::new(), |(l, a)| format!(" using a swiglu clamped at {l:?} with slope {a:?}"));
    s.push_str(&format!(
        "Let Y be moe of X with router Wr{} gate Wg{} up Wu{} down Wd{} choosing {K} of {E}{act}.\n",
        b("Br", o.router_bias),
        b("Bg", o.biases),
        b("Bu", o.biases),
        b("Bd", o.biases)
    ));
    for i in 1..=D {
        s.push_str(&format!("Show round(item {i} of Y * 1000000000).\n"));
    }
    s
}

/// The mixture in `f64`: the router's logits (plus its bias), softmax, the top `K` renormalized, each kept expert's
/// gate and up (plus their biases) through the activation, its down projection (plus its bias), summed by weight.
fn oracle(o: Options) -> Vec<i64> {
    let logits: Vec<f64> =
        (0..E).map(|e| (0..D).map(|t| X[t] * wr(t, e)).sum::<f64>() + if o.router_bias { br(e) } else { 0.0 }).collect();
    let m = logits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let z: f64 = logits.iter().map(|l| (l - m).exp()).sum();
    let p: Vec<f64> = logits.iter().map(|l| (l - m).exp() / z).collect();
    let mut order: Vec<usize> = (0..E).collect();
    order.sort_by(|&a, &b| p[b].partial_cmp(&p[a]).unwrap().then(a.cmp(&b)));
    let kept = &order[..K];
    let total: f64 = kept.iter().map(|&e| p[e]).sum();
    let mut y = [0f64; D];
    for &e in kept {
        let w = p[e] / total;
        let bias = |v: f64| if o.biases { v } else { 0.0 };
        let h: Vec<f64> = (0..F)
            .map(|r| {
                let g = (0..D).map(|t| X[t] * wg(e, r, t)).sum::<f64>() + bias(bg(e, r));
                let u = (0..D).map(|t| X[t] * wu(e, r, t)).sum::<f64>() + bias(bu(e, r));
                match o.clamp {
                    Some((l, a)) => {
                        let gc = g.min(l);
                        gc / (1.0 + (-a * gc).exp()) * (u.clamp(-l, l) + 1.0)
                    }
                    None => g / (1.0 + (-g).exp()) * u,
                }
            })
            .collect();
        for (r, slot) in y.iter_mut().enumerate() {
            *slot += w * ((0..F).map(|t| h[t] * wd(e, r, t)).sum::<f64>() + bias(bdn(e, r)));
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

const ALL: Options = Options { router_bias: true, biases: true, clamp: Some((7.0, 1.702)) };

#[test]
fn every_option_computes_the_formula() {
    let plain = oracle(Options::default());
    close(&shown(&program(Options::default())), &plain, "the plain mixture");
    for (what, o) in [
        ("every option", ALL),
        ("the router's bias", Options { router_bias: true, ..Options::default() }),
        ("the experts' biases", Options { biases: true, ..Options::default() }),
        ("the clamped gate", Options { clamp: Some((0.5, 1.702)), ..Options::default() }),
    ] {
        let want = oracle(o);
        close(&shown(&program(o)), &want, what);
        assert!(want.iter().zip(&plain).any(|(a, b)| (a - b).abs() > 1000), "{what} moves the answer on these inputs");
    }
}

#[test]
fn the_compiled_tiers_take_every_option() {
    let src = program(ALL);
    let want = vm_outcome(&src).output;
    common::assert_c_output(&src, want.trim());
    common::assert_exact_output(&src, want.trim());
}

#[test]
fn the_front_end_lifts_the_options_into_the_spec() {
    let lifted = |o: Options| {
        let rp = recognize_stages(&program(o)).expect("parses");
        assert!(rp.unrecognized.is_empty(), "{:?}", rp.unrecognized);
        let st = rp.stages.into_iter().find(|s| matches!(s.op, TensorOpSpec::Moe(_))).expect("a mixture");
        let TensorOpSpec::Moe(m) = st.op else { unreachable!() };
        (m, st.ins)
    };
    let (m, ins) = lifted(ALL);
    assert_eq!(&ins[..5], ["X", "Wr", "Wg", "Wu", "Wd"], "the operands, in order: {ins:?}");
    assert!(m.router_bias.is_some() && m.biases.iter().all(Option::is_some), "every bias is lifted");
    assert_eq!(m.act, MoeAct::ClampedSwiglu { limit: 7.0f64.to_bits(), slope: 1.702f64.to_bits() });
    assert_eq!((m.d, m.dff, m.ne, m.k), (D, F, E, K));
    let (plain, _) = lifted(Options::default());
    assert!(plain.router_bias.is_none() && plain.biases.iter().all(Option::is_none));
    assert_eq!(plain.act, MoeAct::Swiglu);
}

#[test]
fn each_expert_bias_alone_is_lifted_into_its_own_slot() {
    // The gate's and the up's biases are told apart by the weight their accumulator sums, not by their order: an up
    // bias with no gate bias is the up's.
    let all = program(ALL);
    for (keep, slot) in [("Bg", 0), ("Bu", 1), ("Bd", 2)] {
        let mut src = all.clone();
        for b in ["Bg", "Bu", "Bd"].into_iter().filter(|b| *b != keep) {
            src = src.replace(&format!(" plus {b} "), " ");
        }
        let rp = recognize_stages(&src).expect("parses");
        let st = rp.stages.into_iter().find(|s| matches!(s.op, TensorOpSpec::Moe(_))).expect("a mixture");
        let TensorOpSpec::Moe(m) = st.op else { unreachable!() };
        let lifted: Vec<Option<&str>> = m.biases.iter().map(|b| b.map(|_| keep)).collect();
        let mut want = vec![None; 3];
        want[slot] = Some(keep);
        assert_eq!(lifted, want, "`{keep}` alone is lifted into slot {slot}");
        assert_eq!(st.ins[5..], ["Br", keep], "the operands past the weights: the router's bias, then `{keep}`");
    }
}

#[test]
fn a_bias_of_the_wrong_shape_or_a_malformed_activation_is_refused() {
    let src = program(ALL);
    for (from, to, why) in [
        ("router Wr plus Br", "router Wr plus Bg", "1×3"),
        ("gate Wg plus Bg", "gate Wg plus Bd", "3×3"),
        ("down Wd plus Bd", "down Wd plus Bg", "3×2"),
        ("clamped at 7.0 with slope 1.702", "clamped at Wr with slope 1.702", "number"),
        ("using a swiglu clamped", "using a gelu clamped", "swiglu"),
    ] {
        let bad = src.replace(from, to);
        let err = vm_outcome(&bad).error.unwrap_or_else(|| panic!("`{to}` is refused"));
        assert!(err.contains("moe") && err.contains(why), "`{to}` is refused naming the {why}: {err}");
    }
}

#[test]
fn the_spatial_ir_lowers_every_option_and_the_cuda_forward_refuses_it_by_name() {
    let src = program(ALL);
    let spatial = compile_program_to_spatial(&src, &SpatialTarget::sn50_rdu()).expect("the tensor program walks");
    assert!(spatial.whole().is_some(), "the spatial IR lowers the biased, clamped mixture whole: {}", spatial.refusal());
    let forward = compile_to_cuda_forward(&src).expect_err("the CUDA forward's mixture has no biases");
    assert!(forward.contains("bias"), "the CUDA forward names what it refuses: {forward}");
}
