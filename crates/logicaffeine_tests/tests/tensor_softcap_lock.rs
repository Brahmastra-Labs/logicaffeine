//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A SOFT CAP TAKES EVERY SCORE, OR EVERY ELEMENT, AS `C·tanh(x/C)`.
//!
//! Gemma 2 soft-caps each attention score before its softmax and each logit before the arg-max: `x ↦ C·tanh(x/C)`,
//! close to `x` for `|x| ≪ C` and never past `±C`. The sugar takes `capped at C` on every attention form, after its
//! scale and before its sinks, and `Let Y be X capped at C` on any tensor.
//!
//!   • the VM, the tree-walker, the C backend and AOT-Rust compute the `f64` formula, on inputs where the cap moves the
//!     answer, alone and with a scale, a window, a causal block and sinks;
//!   • a capped element reaches `±C` in both tails and is `0` at `0`, with no `∞/∞`;
//!   • a cap that is not a positive number written out is refused by name;
//!   • the tensor front end lifts a capped attention with its cap in its spec (`AttentionSpec::softcap`, apart from its
//!     scale) and a capped tensor as an op of its own (`TensorOpSpec::Softcap`), never either as an uncapped op;
//!   • the spatial path lowers both (`spatial_softcap_lock` holds their values), and the SN50 computes a Gemma 2 shaped
//!     decode step, its scores and its logits capped: on an fp32 twin of the chip within a relative 2⁻¹⁶ of the VM, on
//!     the SN50 within 2% of the largest logit, and the caps move those logits;
//!   • the tensor graph's evaluator computes every capped op as the VM does, value for value;
//!   • the CUDA forward, with no cap in its kernels, refuses a capped program by name, and so does every shim of a
//!     chip without a cap (the forward, the D5005 engine) and the backward pass, while every spatial chip's shim and the
//!     CUDA lane at every knob point resolve both;
//!   • every file of the compiler that reads an attention's spec is named here with what it does with the cap — lifts,
//!     routes, computes, refuses — and each that lifts, computes or refuses names the cap: a field read silently skipped
//!     is a model computed wrong.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;
#[path = "common/model_harness.rs"]
mod model_harness;
#[path = "common/spatial.rs"]
mod spatial_support;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use logicaffeine_compile::codegen_cuda::forward_shim::ForwardShim;
use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane_shim::LaneShim;
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::{compile_to_cuda_forward, tw_outcome, vm_outcome};
use logicaffeine_compile::model::author;
use logicaffeine_compile::model::reference::{argmax, Reference};
use logicaffeine_compile::model::spec::{Mixer, ModelSpec};
use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
use logicaffeine_compile::spatial::decode_engine::shim::EngineShim;
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Value};
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::shim::SpatialShim;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use logicaffeine_compile::tensor_graph::eval::{eval, Data};
use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
use logicaffeine_compile::tensor_graph::link::resolve;
use logicaffeine_compile::tensor_graph::shim::{ReferenceShim, Shim};
use logicaffeine_compile::tensor_graph::vjp::backward;
use logicaffeine_compile::tensor_recognize::qwen::Window;
use model_harness::{carried, checkpoint, filled, run, CTX, PROMPT};
use spatial_support::inputs_from_fills;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tiny_models::Tiny;

const L: usize = 6;
const HD: usize = 2;

fn key(j: usize, c: usize) -> f64 {
    [0.3 * j as f64 - 1.0, 0.2 * (j % 3) as f64 - 0.1][c]
}

fn value(j: usize, c: usize) -> f64 {
    [0.5 * j as f64 - 1.0, 0.25 * (7 - j) as f64][c]
}

fn query(r: usize, head: usize, c: usize) -> f64 {
    [1.6, -1.2][c] + 0.3 * r as f64 - 0.7 * head as f64
}

fn sink(h: usize) -> f64 {
    [0.9, -0.4][h]
}

/// `Q` (`rows` by `heads·2`), `K` and `V` (6 by 2) and `S` (1 by `heads`) filled with `Real` literals, `C` bound by
/// `op`, every entry of `C` shown as `round(c·1e9)`.
fn program(op: &str, rows: usize, heads: usize) -> String {
    let mut s = format!("## Main\nLet Q be a {rows} by {} tensor of Real.\n", heads * HD);
    for r in 1..=rows {
        for h in 0..heads {
            for c in 0..HD {
                s.push_str(&format!("Set item {} of Q to {:?}.\n", (r - 1) * heads * HD + h * HD + c + 1, query(r, h, c)));
            }
        }
    }
    s.push_str(&format!("Let K be a {L} by {HD} tensor of Real.\nLet V be a {L} by {HD} tensor of Real.\n"));
    for j in 1..=L {
        for c in 0..HD {
            s.push_str(&format!("Set item {} of K to {:?}.\n", (j - 1) * HD + c + 1, key(j, c)));
            s.push_str(&format!("Set item {} of V to {:?}.\n", (j - 1) * HD + c + 1, value(j, c)));
        }
    }
    s.push_str(&format!("Let S be a 1 by {heads} tensor of Real.\n"));
    for h in 0..heads {
        s.push_str(&format!("Set item {} of S to {:?}.\n", h + 1, sink(h)));
    }
    s.push_str(&format!("Let pos be 5.\nLet C be {op}.\n"));
    for i in 1..=rows * heads * HD {
        s.push_str(&format!("Show round(item {i} of C * 1000000000).\n"));
    }
    s
}

/// What an attention's clauses ask of each score and its softmax.
#[derive(Clone, Copy)]
struct Clauses {
    window: Option<usize>,
    scale: Option<f64>,
    cap: Option<f64>,
    sinks: bool,
}

const PLAIN: Clauses = Clauses { window: None, scale: None, cap: None, sinks: false };

/// The `f64` attention of query row `r` at position `n`: each score `q·k` times the scale, capped as `C·tanh(s/C)`, over
/// `max(1, n − w + 1) ..= n`, each head's sink in its softmax.
fn oracle(r: usize, heads: usize, n: usize, k: Clauses) -> Vec<i64> {
    let lo = k.window.map_or(1, |w| n.saturating_sub(w - 1).max(1));
    let scale = k.scale.unwrap_or(1.0 / (HD as f64).sqrt());
    (0..heads)
        .flat_map(|h| {
            let scores: Vec<f64> = (lo..=n)
                .map(|j| {
                    let s = (0..HD).map(|c| query(r, h, c) * key(j, c)).sum::<f64>() * scale;
                    k.cap.map_or(s, |cap| cap * (s / cap).tanh())
                })
                .collect();
            let m = scores.iter().copied().fold(if k.sinks { sink(h) } else { f64::NEG_INFINITY }, f64::max);
            let den: f64 = scores.iter().map(|s| (s - m).exp()).sum::<f64>() + if k.sinks { (sink(h) - m).exp() } else { 0.0 };
            (0..HD)
                .map(|c| {
                    let a: f64 = (lo..=n).zip(&scores).map(|(j, s)| (s - m).exp() / den * value(j, c)).sum();
                    (a * 1e9).round() as i64
                })
                .collect::<Vec<_>>()
        })
        .collect()
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
        assert!((g - w).abs() <= 1, "{what}: {got:?} vs the formula's {want:?}");
    }
}

#[test]
fn a_capped_score_is_its_tanh_before_the_softmax() {
    let capped = Clauses { cap: Some(0.5), ..PLAIN };
    let got = shown(&program("attention of Q over K and V with 2 heads 1 groups up to pos capped at 0.5", 1, 2));
    close(&got, &oracle(1, 2, 5, capped), "two heads, each score capped at 0.5");
    let uncapped = oracle(1, 2, 5, PLAIN);
    assert!(got.iter().zip(&uncapped).any(|(a, b)| (a - b).abs() > 1_000_000), "the cap moves the answer on these inputs");
    // A cap far past every score leaves the answer within the formula's own rounding of the uncapped one.
    let wide = shown(&program("attention of Q over K and V with 2 heads 1 groups up to pos capped at 50.0", 1, 2));
    close(&wide, &oracle(1, 2, 5, Clauses { cap: Some(50.0), ..PLAIN }), "a cap of 50");
    assert!(wide.iter().zip(&uncapped).all(|(a, b)| (a - b).abs() < 1_000_000), "a cap of 50 barely moves these scores");
}

#[test]
fn the_cap_follows_the_scale_and_composes_with_a_window_a_causal_block_and_sinks() {
    let all = Clauses { window: Some(3), scale: Some(0.9), cap: Some(0.4), sinks: true };
    close(
        &shown(&program("attention of Q over K and V with 2 heads 1 groups up to pos within a window of 3 scaled by 0.9 capped at 0.4 with sinks S", 1, 2)),
        &oracle(1, 2, 5, all),
        "a windowed, scaled, capped attention with sinks",
    );
    let got = shown(&program("causal attention of Q over K and V with 1 heads 1 groups from 3 within a window of 2 capped at 0.3", 3, 1));
    let want: Vec<i64> = (1..=3).flat_map(|r| oracle(r, 1, 3 + r - 1, Clauses { window: Some(2), cap: Some(0.3), ..PLAIN })).collect();
    close(&got, &want, "each row of a causal block over its window, its scores capped");
}

/// `Let Y be X capped at C` over `xs`, each element shown as `round(y·1e9)`.
fn capped_tensor(xs: &[f64], cap: &str) -> String {
    let mut s = format!("## Main\nLet X be a 1 by {} tensor of Real.\n", xs.len());
    for (i, x) in xs.iter().enumerate() {
        s.push_str(&format!("Set item {} of X to {x:?}.\n", i + 1));
    }
    s.push_str(&format!("Let Y be X capped at {cap}.\n"));
    for i in 1..=xs.len() {
        s.push_str(&format!("Show round(item {i} of Y * 1000000000).\n"));
    }
    s
}

#[test]
fn a_capped_tensor_is_its_tanh_elementwise_and_reaches_the_cap_in_both_tails() {
    let xs = [0.0, 0.5, -0.5, 3.0, -3.0, 30.0, -30.0, 1e6, -1e6, 1e300, -1e300];
    let got = shown(&capped_tensor(&xs, "2.0"));
    let want: Vec<i64> = xs.iter().map(|x| ((2.0 * (x / 2.0).tanh()) * 1e9).round() as i64).collect();
    close(&got, &want, "every element capped at 2");
    assert_eq!(got[0], 0, "zero stays zero");
    assert_eq!((got[7], got[8], got[9], got[10]), (2_000_000_000, -2_000_000_000, 2_000_000_000, -2_000_000_000), "the tails reach ±C, with no ∞/∞");
}

#[test]
fn the_compiled_tiers_take_the_caps() {
    for src in [
        program("causal attention of Q over K and V with 2 heads 1 groups from 3 scaled by 0.8 capped at 0.5 with sinks S", 3, 2),
        capped_tensor(&[0.0, 0.75, -4.0, 1e6, -1e300], "3.0"),
    ] {
        let want = vm_outcome(&src).output;
        common::assert_c_output(&src, want.trim());
        common::assert_exact_output(&src, want.trim());
    }
}

#[test]
fn a_cap_that_is_not_a_positive_number_is_refused_by_name() {
    for cap in ["0.0", "-1.0", "pos", "inf"] {
        let err = vm_outcome(&program(&format!("attention of Q over K and V with 2 heads 1 groups up to pos capped at {cap}"), 1, 2))
            .error
            .unwrap_or_else(|| panic!("an attention capped at `{cap}` is refused"));
        assert!(err.contains("capped at C") && err.contains("positive number"), "the refusal names the clause: {err}");
        let err = vm_outcome(&capped_tensor(&[1.0], cap)).error.unwrap_or_else(|| panic!("a tensor capped at `{cap}` is refused"));
        assert!(err.contains("capped at C") && err.contains("positive number"), "the refusal names the sugar: {err}");
    }
    let err = vm_outcome(&program("attention of Q over K and V with 2 heads 1 groups up to pos capped 0.5", 1, 2))
        .error
        .expect("`capped` without `at` is refused");
    assert!(err.contains("expected `at`"), "{err}");
}

#[test]
fn the_front_end_lifts_no_capped_op_as_an_uncapped_one_and_the_cuda_forward_refuses_it_by_name() {
    for (src, what, cap) in [
        (program("attention of Q over K and V with 2 heads 1 groups up to pos capped at 0.5", 1, 2), "a capped attention", 0.5),
        (program("causal attention of Q over K and V with 2 heads 1 groups from 3 scaled by 0.8 capped at 0.5", 3, 2), "a capped causal block", 0.5),
        (capped_tensor(&[0.5, -3.0], "2.0"), "a capped tensor", 2.0),
    ] {
        let rp = recognize_stages(&src).expect("parses");
        assert!(rp.unrecognized.is_empty(), "{what} is lifted whole: {:?}", rp.unrecognized);
        let lifted: Vec<&TensorOpSpec> = rp.stages.iter().filter(|s| s.out == "C" || s.out == "Y").map(|s| &s.op).collect();
        assert_eq!(lifted.len(), 1, "{what} is one op: {lifted:?}");
        let carried = match lifted[0] {
            TensorOpSpec::Attention(a) => a.softcap.map(|c| c.get()),
            TensorOpSpec::Softcap(s) => Some(s.cap.get()),
            other => panic!("{what} is lifted as an op with no cap: {other:?}"),
        };
        assert_eq!(carried, Some(cap), "{what} carries its cap");
        for target in [SpatialTarget::sn50_rdu(), SpatialTarget::wormhole()] {
            compile_program_to_spatial(&src, &target).unwrap_or_else(|why| panic!("the spatial IR lowers {what} for {}: {why:?}", target.name));
        }
        let refusal = compile_to_cuda_forward(&src).expect_err("the CUDA forward has no cap in its kernels");
        assert!(refusal.contains("cap"), "the CUDA forward names the cap it refuses in {what}: {refusal}");
    }
}

#[test]
fn the_front_end_lifts_the_cap_into_the_spec_apart_from_the_scale() {
    let lifted = |op: &str, rows: usize| {
        let src = program(op, rows, 2);
        let rp = recognize_stages(&src).expect("parses");
        assert!(rp.unrecognized.is_empty(), "{op}: {:?}", rp.unrecognized);
        let st = rp.stages.into_iter().find(|s| matches!(s.op, TensorOpSpec::Attention(_))).expect("an attention");
        let TensorOpSpec::Attention(a) = st.op else { unreachable!() };
        (a, st.ins)
    };
    let (a, ins) = lifted("attention of Q over K and V with 2 heads 1 groups up to pos capped at 0.5", 1);
    assert_eq!(a.softcap.map(|c| c.get()), Some(0.5), "the cap is lifted");
    assert!(a.scale.is_none(), "the default scale stays the default under a cap");
    assert_eq!(&ins[..3], ["Q", "K", "V"], "Q, K and V stay the operands, in order: {ins:?}");
    let (b, _) = lifted("attention of Q over K and V with 2 heads 1 groups up to pos scaled by 0.9 capped at 0.5", 1);
    assert_eq!((b.scale.map(|s| s.get()), b.softcap.map(|c| c.get())), (Some(0.9), Some(0.5)), "the scale and the cap apart");
    let (c, _) = lifted("attention of Q over K and V with 2 heads 1 groups up to pos", 1);
    assert!(c.softcap.is_none(), "no cap without the clause");
    let (d, _) = lifted("causal attention of Q over K and V with 2 heads 1 groups from 3 within a window of 2 capped at 0.5 with sinks S", 3);
    assert_eq!(
        (d.window, d.sinks.is_some(), d.rows, d.softcap.map(|c| c.get())),
        (Some(Window::Sliding(2)), true, 3, Some(0.5)),
        "a causal block with a window, a cap and sinks"
    );

    let rp = recognize_stages(&capped_tensor(&[-3.0, -0.4, 0.0, 0.2, 0.9, 7.5], "0.5")).expect("parses");
    assert!(rp.unrecognized.is_empty(), "a capped tensor: {:?}", rp.unrecognized);
    let st = rp.stages.iter().find(|s| matches!(s.op, TensorOpSpec::Softcap(_))).expect("a capped tensor lifts as its own op");
    let TensorOpSpec::Softcap(s) = &st.op else { unreachable!() };
    assert_eq!((s.n, s.cap.get()), (6, 0.5), "its six elements, capped at 0.5");
    assert_eq!(st.ins, ["X"], "X its operand");
}

/// The three capped programs, each with the kind of its capped op: an attention, a causal block and a tensor.
fn capped_programs() -> [(String, &'static str, &'static str); 3] {
    [
        (program("attention of Q over K and V with 2 heads 1 groups up to pos capped at 0.5", 1, 2), "a capped attention", "Attention"),
        (program("causal attention of Q over K and V with 2 heads 1 groups from 3 scaled by 0.8 capped at 0.5", 3, 2), "a capped causal block", "Attention"),
        (capped_tensor(&[0.5, -3.0], "2.0"), "a capped tensor", "Softcap"),
    ]
}

/// The shims of the chips with no cap in their kernels: the forward at each of its activations, the D5005 engine for
/// both of its tiny models.
fn shims_without_a_cap() -> Vec<Box<dyn Shim>> {
    let mut out: Vec<Box<dyn Shim>> = Vec::new();
    for activations in [Q8MatKind::Dequant, Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        out.push(Box::new(ForwardShim { kind: activations }));
    }
    for tiny in [Tiny::Qwen35, Tiny::Qwen3] {
        out.push(Box::new(EngineShim { spec: Box::leak(Box::new(tiny.spec(24))), numerics: EngineNumerics::precise() }));
    }
    out
}

/// The CUDA lane at every knob point: every attention kernel caps its scores, and a capped tensor is one launch.
fn lane_shims() -> Vec<LaneShim> {
    let mut out = Vec::new();
    for activations in [Q8MatKind::Dequant, Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        for fuse_quant in [true, false] {
            for batch in [false, true] {
                let knobs = CudaKnobs { activations, fuse_quant, ..CudaKnobs::default() };
                out.push(LaneShim { knobs, batch, held: Vec::new() });
            }
        }
    }
    out
}

#[test]
fn every_shim_without_the_cap_refuses_the_capped_op_by_name_and_every_spatial_shim_and_the_cuda_lane_resolve_it() {
    for (src, what, kind) in capped_programs() {
        let rp = recognize_stages(&src).expect("parses");
        let g = TensorGraph::lift(&rp, None, None, &|_| Storage::Values).unwrap_or_else(|why| panic!("{what} lifts: {why:?}"));
        assert!(g.ops.iter().any(|o| o.kind() == kind), "{what} lifts as a {kind}");
        for shim in shims_without_a_cap() {
            let unresolved = resolve(&g, shim.as_ref()).expect_err(&format!("{} refuses {what}", shim.name()));
            let refused = unresolved.iter().find(|u| u.kind == kind).unwrap_or_else(|| panic!("{} refuses {what}'s {kind}: {unresolved:?}", shim.name()));
            assert!(refused.why.contains("cap"), "{} names the cap it refuses in {what}: {}", shim.name(), refused.why);
        }
        for target in [SpatialTarget::wormhole(), SpatialTarget::sky_fabric(64, 64), SpatialTarget::sn50_rdu(), SpatialTarget::rtx3060()] {
            let shim = SpatialShim { target: Box::leak(Box::new(target)) };
            resolve(&g, &shim).unwrap_or_else(|why| panic!("{} resolves {what}: {why:?}", shim.name()));
        }
        for shim in lane_shims() {
            resolve(&g, &shim).unwrap_or_else(|why| panic!("{} resolves {what}: {why:?}", shim.name()));
        }
        resolve(&g, &ReferenceShim).unwrap_or_else(|why| panic!("the reference resolves {what}: {why:?}"));
    }
}

/// `src` without its `Show` lines.
fn unshown(src: &str) -> String {
    src.lines().filter(|l| !l.starts_with("Show ")).map(|l| format!("{l}\n")).collect()
}

/// `src` without its `Show` lines, the cotangent `W` (`rows` by `cols`) declared after it and filled with values on both
/// sides of zero.
fn with_cotangent(src: &str, rows: usize, cols: usize) -> String {
    let mut s = unshown(src);
    s.push_str(&format!("Let W be a {rows} by {cols} tensor of Real.\n"));
    for i in 1..=rows * cols {
        s.push_str(&format!("Set item {i} of W to {:?}.\n", 0.3 * i as f64 - 0.5));
    }
    s
}

#[test]
fn the_graph_evaluator_computes_every_capped_op_as_the_vm_does() {
    let mut programs = capped_programs().to_vec();
    programs.push((
        program("causal attention of Q over K and V with 2 heads 1 groups from 3 within a window of 2 scaled by 0.9 capped at 0.4 with sinks S", 3, 2),
        "a capped causal block over a window with sinks",
        "Attention",
    ));
    programs.push((capped_tensor(&[0.0, 0.75, -4.0, 1e6, -1e300], "3.0"), "a capped tensor reaching both tails", "Softcap"));
    for (src, what, kind) in programs {
        let bare = unshown(&src);
        let g = TensorGraph::lift(&recognize_stages(&bare).expect("parses"), None, None, &|_| Storage::Values)
            .unwrap_or_else(|why| panic!("{what} lifts: {why:?}"));
        let out = &g.ops.iter().find(|o| o.kind() == kind).unwrap_or_else(|| panic!("{what} lifts as a {kind}")).out;
        let env = eval(&g, &BTreeMap::new()).unwrap_or_else(|why| panic!("the evaluator computes {what}: {why}"));
        let Some(Data::Real(values)) = env.get(out) else { panic!("{what}: the evaluator has no real `{out}`") };
        let mut shown = bare.clone();
        for i in 1..=values.len() {
            shown.push_str(&format!("Show item {i} of {out}.\n"));
        }
        let vm = vm_outcome(&shown);
        assert!(vm.error.is_none(), "{what}: the VM runs it: {:?}", vm.error);
        let printed: Vec<&str> = vm.output.split_whitespace().collect();
        let evaluated: Vec<String> = values.iter().map(|x| format!("{x}")).collect();
        assert_eq!(printed, evaluated, "{what}: the evaluator's `{out}` is the VM's, value for value");
    }
}

#[test]
fn the_backward_pass_takes_both_caps() {
    // The backward pass differentiates through each cap (`tensor_graph_vjp_lock::a_cap_differentiates_through_its_tanh`
    // holds the gradients to central differences); the capped attention's gradient is not the uncapped one's.
    let grad_of = |op: &str| {
        let src = with_cotangent(&program(op, 1, 2), 1, 4);
        let b = backward(&src, "C", "W", &["Q"]).unwrap_or_else(|e| panic!("{op}: {e:?}"));
        let mut shown = src.clone();
        shown.push_str(&b.source);
        for i in 1..=4 {
            shown.push_str(&format!("Show item {i} of {}.\n", b.grads["Q"]));
        }
        let vm = vm_outcome(&shown);
        assert!(vm.error.is_none(), "{op}: the VM runs the backward pass: {:?}", vm.error);
        vm.output.split_whitespace().map(|t| t.parse::<f64>().expect("a number")).collect::<Vec<_>>()
    };
    let capped = grad_of("attention of Q over K and V with 2 heads 1 groups up to pos capped at 0.5");
    let plain = grad_of("attention of Q over K and V with 2 heads 1 groups up to pos");
    assert!(capped.iter().zip(&plain).any(|(a, b)| (a - b).abs() > 1e-6), "the cap moves the gradient: {capped:?} vs {plain:?}");
    let tensor = with_cotangent(&capped_tensor(&[0.5, -3.0], "2.0"), 1, 2);
    backward(&tensor, "Y", "W", &["X"]).unwrap_or_else(|e| panic!("a capped tensor: {e:?}"));
}

/// A chip with fp32 in place of bf16 on the matmul, pointwise and row units: every value of the step is fp32.
fn fp32_twin() -> SpatialTarget {
    let mut t = SpatialTarget::sn50_rdu();
    for c in &mut t.compute {
        if let ComputeCap::MatmulTile { precisions, .. } | ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } = c {
            if precisions.remove(&Precision::Bf16) {
                precisions.insert(Precision::Fp32);
            }
        }
    }
    t.precisions.remove(&Precision::Bf16);
    t.precisions.insert(Precision::Fp32);
    t
}

/// Gemma 2's shape on the tiny Gemma 3: every attention's scores and the logits soft-capped, at caps the tiny model's
/// values reach.
fn gemma2_shaped(caps: Option<(f64, f64)>) -> ModelSpec {
    let mut spec = Tiny::Gemma3.spec(CTX);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.softcap = caps.map(|(s, _)| s);
        }
    }
    spec.final_logit_softcap = caps.map(|(_, l)| l);
    spec
}

/// The decode step of `spec` after the prompt, every weight, table and cache written into it, and the VM's logits.
fn decode_step(spec: &ModelSpec) -> (String, Vec<f64>) {
    let ckpt = checkpoint(spec);
    let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
    let prefill = reference.step(PROMPT).expect("reference prefill");
    let tok = argmax(&prefill);
    let caches = carried(spec, &reference);
    let prog = author::decode_step(spec, "/w", CTX, tok as usize + 1, PROMPT.len() + 1);
    let src = filled(spec, &prog, &ckpt, &caches);
    let (vm, _) = run(&src, spec.vocab);
    (src, vm)
}

fn on_the_chip(src: &str, target: &SpatialTarget) -> Vec<f64> {
    let prog = compile_program_to_spatial(src, target).expect("a tensor program").into_whole().unwrap_or_else(|why| panic!("{why}"));
    let out = eval_choreography_every_tensor(&prog, &inputs_from_fills(&prog)).expect("the program evaluates");
    let logits = prog.tensors.iter().find(|t| t.name == "logits").expect("the logits").id;
    match out.tensors.get(&logits).expect("the evaluation hands back the logits") {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

#[test]
fn the_sn50_computes_a_capped_decode_step() {
    let spec = gemma2_shaped(Some((0.25, 0.5)));
    let (src, vm) = decode_step(&spec);
    let (_, uncapped) = decode_step(&gemma2_shaped(None));
    let scale = vm.iter().fold(0.0f64, |m, x| m.max(x.abs())).max(1e-9);
    assert!(
        vm.iter().zip(&uncapped).any(|(a, b)| (a - b).abs() > 0.05 * scale),
        "the caps move the logits on this step"
    );
    for (target, bound, form) in [(fp32_twin(), scale / 65536.0, "WIRING"), (SpatialTarget::sn50_rdu(), 0.02 * scale, "CHIP")] {
        let chip = on_the_chip(&src, &target);
        assert_eq!(chip.len(), vm.len(), "the logits' count");
        for (k, (c, v)) in chip.iter().zip(&vm).enumerate() {
            assert!((c - v).abs() <= bound, "{form} logit {k} is {c} on {}, the VM's {v}", target.name);
        }
    }
}

/// Every file of the compiler that reads an attention's spec, and what it does with the cap. A file that lifts or
/// computes an attention, or refuses what it does not compute, names `softcap`; one that only routes an attention by its
/// shape does not need to.
const CONSUMERS: &[(&str, &str)] = &[
    ("tensor_recognize/qwen.rs", "lifts"),
    ("tensor_graph/front.rs", "routes"),
    ("tensor_graph/graph.rs", "routes"),
    ("tensor_graph/numerics.rs", "routes"),
    ("tensor_graph/fuse.rs", "routes"),
    ("spatial/value_range.rs", "routes"),
    ("codegen_cuda/lane/graph.rs", "routes"),
    ("tensor_graph/eval.rs", "computes"),
    ("tensor_graph/vjp.rs", "computes"),
    ("spatial/link.rs", "computes"),
    ("spatial/passes/from_qwen_ops.rs", "computes"),
    ("codegen_cpu/link.rs", "computes"),
    ("codegen_cuda/forward_shim.rs", "refuses"),
    ("codegen_cuda/lane_shim.rs", "refuses"),
    ("codegen_cuda/lane_link.rs", "refuses"),
    ("codegen_cuda/qwen_emit.rs", "refuses"),
    ("spatial/decode_engine/shim.rs", "refuses"),
];

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn every_consumer_of_an_attention_spec_is_held_to_its_cap() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let reads = |text: &str| {
        text.contains("AttentionSpec") || text.contains("TensorOpSpec::Attention(") || text.contains(" T::Attention(")
    };
    let found: BTreeSet<String> = files
        .iter()
        .filter(|p| reads(&std::fs::read_to_string(p).unwrap()))
        .map(|p| p.strip_prefix(&src).unwrap().to_string_lossy().to_string())
        .collect();
    let named: BTreeSet<String> = CONSUMERS.iter().map(|(f, _)| f.to_string()).collect();
    assert_eq!(found, named, "the files that read an attention's spec are the ones this census names");
    for (file, role) in CONSUMERS {
        let text = std::fs::read_to_string(src.join(file)).unwrap();
        if matches!(*role, "lifts" | "computes" | "refuses") {
            assert!(text.contains("softcap"), "{file} {role} an attention, so it reads its cap: it never names `softcap`");
        }
    }
}
