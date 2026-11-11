//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A GATED DELTANET GATED BY A SIGMOID (QWEN4-8 of `benchmarks/PREPARE_FOR_SAMBA.md`).
//!
//! Qwen 3.5's Gated DeltaNet gates its normed output by `silu(z)`; Qwen3.8-Flash-Next's by `σ(z)`. The sugar takes
//! `with sigmoid gate Z` where `with gate Z` keeps SiLU:
//!
//!   • the VM computes the clean-room f64 step, its output and its mutated memory, at several shapes, and the gate
//!     binds: the sigmoid-gated output is not the SiLU-gated one; the tree-walker, C and AOT-Rust print what it prints;
//!   • the tensor front end lifts the gate into the op (`GatedDeltaSpec::gate`), a decode row and a block of rows, and
//!     the graph's evaluator computes the op as the VM does, value for value;
//!   • the SN50 computes a sigmoid-gated step as the VM does, within the bf16 datapath's error, and its gate moves it;
//!   • the CUDA forward and every shim of a chip without the gate refuse it by name, and every spatial shim resolves it;
//!   • every file of the compiler that reads a gated delta's spec is named here with what it does with the gate, and
//!     each that lifts, computes or refuses names it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test tensor_gated_delta_gate_lock`
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
use logicaffeine_compile::codegen_cuda::qwen38::Qwen38Cfg;
use logicaffeine_compile::compile::{compile_to_cuda_forward, tw_outcome, vm_outcome};
use logicaffeine_compile::model::spec::{GateActivation, Mixer};
use logicaffeine_compile::spatial::decode_engine::capability::spec_refusals;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::shim::SpatialShim;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_graph::eval::{eval, Data};
use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
use logicaffeine_compile::tensor_graph::link::resolve;
use logicaffeine_compile::tensor_graph::shim::{ReferenceShim, Shim};
use logicaffeine_compile::tensor_recognize::qwen::GateAct;
use spatial_support::inputs_from_fills;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// A deterministic, spread pseudo-weight for element `i` of tensor `salt`.
fn wv(salt: u64, i: usize) -> f64 {
    let h = (i as u64).wrapping_mul(2654435761).wrapping_add(salt.wrapping_mul(40503));
    ((h % 97) as f64 / 97.0 - 0.5) * 0.6
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

fn silu(x: f64) -> f64 {
    x * sigmoid(x)
}

/// One gated-delta step's inputs at `HV` value heads over `HK` key heads `D` wide, `rows` rows.
struct Step {
    hv: usize,
    hk: usize,
    d: usize,
    rows: usize,
    q: Vec<f64>,
    k: Vec<f64>,
    v: Vec<f64>,
    z: Vec<f64>,
    b: Vec<f64>,
    a: Vec<f64>,
    s0: Vec<f64>,
    gn: Vec<f64>,
}

fn step(hv: usize, hk: usize, d: usize, rows: usize) -> Step {
    let (qn, vn) = (hk * d, hv * d);
    Step {
        hv,
        hk,
        d,
        rows,
        q: (0..rows * qn).map(|i| wv(1, i)).collect(),
        k: (0..rows * qn).map(|i| wv(2, i)).collect(),
        v: (0..rows * vn).map(|i| wv(3, i)).collect(),
        // Gate inputs far enough from zero that σ(z) and silu(z) part.
        z: (0..rows * vn).map(|i| 4.0 * wv(4, i)).collect(),
        b: (0..rows * hv).map(|i| 0.5 + wv(5, i)).collect(),
        a: (0..rows * hv).map(|i| 0.5 + wv(6, i)).collect(),
        s0: (0..vn * d).map(|i| wv(7, i)).collect(),
        gn: (0..d).map(|i| 1.0 + wv(8, i)).collect(),
    }
}

/// The step from the equations, row by row against one memory: per value head `h` (key head `h mod HK`), `u = S·k`,
/// `S = α·S + β·(v − α·u)·kᵀ`, `o = S·q / √D`, then RMSNorm of `o`, scaled by `GN`, gated by `gate(z)`. Returns the
/// output and the memory after the last row.
fn oracle(s: &Step, gate: fn(f64) -> f64) -> (Vec<f64>, Vec<f64>) {
    let (qn, vn, d) = (s.hk * s.d, s.hv * s.d, s.d);
    let mut st = s.s0.clone();
    let mut out = vec![0.0; s.rows * vn];
    for row in 0..s.rows {
        for h in 0..s.hv {
            let (kb, vb, sb) = (row * qn + (h % s.hk) * d, row * vn + h * d, h * d * d);
            let (alpha, beta) = (s.a[row * s.hv + h], s.b[row * s.hv + h]);
            let u: Vec<f64> = (0..d).map(|r| (0..d).map(|c| st[sb + r * d + c] * s.k[kb + c]).sum()).collect();
            for r in 0..d {
                let coef = beta * (s.v[vb + r] - alpha * u[r]);
                for c in 0..d {
                    st[sb + r * d + c] = alpha * st[sb + r * d + c] + coef * s.k[kb + c];
                }
            }
            let o: Vec<f64> =
                (0..d).map(|r| (0..d).map(|c| st[sb + r * d + c] * s.q[kb + c]).sum::<f64>() / (d as f64).sqrt()).collect();
            let den = (o.iter().map(|x| x * x).sum::<f64>() / d as f64 + 1e-6).sqrt();
            for r in 0..d {
                out[vb + r] = s.gn[r] * o[r] / den * gate(s.z[vb + r]);
            }
        }
    }
    (out, st)
}

/// `Let O be gated delta …` over `s`, its gate clause `gate` (`gate` or `sigmoid gate`).
fn program(s: &Step, gate: &str) -> String {
    let (qn, vn) = (s.hk * s.d, s.hv * s.d);
    let mut src = "## Main\n".to_string();
    let decl = |src: &mut String, name: &str, r: usize, c: usize, vals: &[f64]| {
        src.push_str(&format!("Let {name} be a {r} by {c} tensor of Real.\n"));
        for (i, v) in vals.iter().enumerate() {
            src.push_str(&format!("Set item {} of {name} to {v:?}.\n", i + 1));
        }
    };
    decl(&mut src, "Q", s.rows, qn, &s.q);
    decl(&mut src, "K", s.rows, qn, &s.k);
    decl(&mut src, "V", s.rows, vn, &s.v);
    decl(&mut src, "Z", s.rows, vn, &s.z);
    decl(&mut src, "B", s.rows, s.hv, &s.b);
    decl(&mut src, "A", s.rows, s.hv, &s.a);
    decl(&mut src, "S", vn, s.d, &s.s0);
    decl(&mut src, "GN", 1, s.d, &s.gn);
    src.push_str(&format!(
        "Let O be gated delta of Q over K and V with {gate} Z beta B decay A state S norm GN with {} heads {} groups.\n",
        s.hv, s.hk
    ));
    src
}

/// `src` with every entry of `O`, then of `S`, shown.
fn shown(src: &str, s: &Step) -> String {
    let (vn, d) = (s.hv * s.d, s.d);
    let mut out = src.to_string();
    for i in 1..=s.rows * vn {
        out.push_str(&format!("Show item {i} of O.\n"));
    }
    for i in 1..=vn * d {
        out.push_str(&format!("Show item {i} of S.\n"));
    }
    out
}

fn vm(src: &str) -> Vec<f64> {
    let out = vm_outcome(src);
    assert!(out.error.is_none(), "the VM runs it: {:?}\n{src}", out.error);
    out.output.split_whitespace().map(|t| t.parse().expect("a number")).collect()
}

const SHAPES: [(usize, usize, usize, usize); 5] = [(2, 1, 2, 1), (2, 2, 3, 1), (6, 2, 2, 1), (4, 1, 3, 1), (4, 2, 2, 3)];

#[test]
fn a_sigmoid_gate_takes_the_normed_head_times_sigmoid_of_z() {
    for (hv, hk, d, rows) in SHAPES {
        let s = step(hv, hk, d, rows);
        let got = vm(&shown(&program(&s, "sigmoid gate"), &s));
        let (o, st) = oracle(&s, sigmoid);
        let want: Vec<f64> = o.iter().chain(&st).copied().collect();
        assert_eq!(got.len(), want.len(), "(HV {hv}, HK {hk}, D {d}, {rows} rows): the output and the memory");
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert!((g - w).abs() < 1e-9, "(HV {hv}, HK {hk}, D {d}, {rows} rows) slot {i}: the VM's {g}, the formula's {w}");
        }
        let (silu_o, _) = oracle(&s, silu);
        assert!(o.iter().zip(&silu_o).any(|(a, b)| (a - b).abs() > 1e-3), "the gate moves the output on these inputs");
    }
}

#[test]
fn the_silu_gate_is_unchanged() {
    let s = step(4, 2, 2, 1);
    let got = vm(&shown(&program(&s, "gate"), &s));
    let (o, st) = oracle(&s, silu);
    for (i, (g, w)) in got.iter().zip(o.iter().chain(&st)).enumerate() {
        assert!((g - w).abs() < 1e-9, "slot {i}: the VM's {g}, the SiLU formula's {w}");
    }
}

#[test]
fn the_compiled_tiers_take_the_sigmoid_gate() {
    for (hv, hk, d, rows) in [(4, 2, 2, 1), (2, 1, 2, 2)] {
        let s = step(hv, hk, d, rows);
        let src = shown(&program(&s, "sigmoid gate"), &s);
        let want = vm_outcome(&src).output;
        let tw = tw_outcome(&src);
        assert!(tw.error.is_none(), "the tree-walker runs it: {:?}", tw.error);
        assert_eq!(tw.output.trim_end(), want.trim_end(), "the tree-walker prints what the VM prints");
        common::assert_c_output(&src, want.trim());
        common::assert_exact_output(&src, want.trim());
    }
}

#[test]
fn the_front_end_lifts_the_gate_into_the_op() {
    for (rows, gate, want) in [(1, "sigmoid gate", GateAct::Sigmoid), (1, "gate", GateAct::Silu), (3, "sigmoid gate", GateAct::Sigmoid), (3, "gate", GateAct::Silu)] {
        let s = step(4, 2, 2, rows);
        let rp = recognize_stages(&program(&s, gate)).expect("parses");
        assert!(rp.unrecognized.is_empty(), "{rows} rows, `{gate}`: {:?}", rp.unrecognized);
        let spec = rp
            .stages
            .iter()
            .find_map(|st| match &st.op {
                TensorOpSpec::GatedDelta(g) => Some(g.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{rows} rows, `{gate}`: a gated delta is lifted"));
        assert_eq!((spec.gate, spec.rows, spec.hv, spec.hk, spec.d), (want, rows, 4, 2, 2), "{rows} rows, `{gate}`");
    }
}

#[test]
fn the_graph_evaluator_computes_the_gated_step_as_the_vm_does() {
    for (gate, rows) in [("sigmoid gate", 1), ("sigmoid gate", 3), ("gate", 1)] {
        let s = step(4, 2, 2, rows);
        let src = program(&s, gate);
        let g = TensorGraph::lift(&recognize_stages(&src).expect("parses"), None, None, &|_| Storage::Values)
            .unwrap_or_else(|why| panic!("it lifts: {why:?}"));
        let env = eval(&g, &BTreeMap::new()).unwrap_or_else(|why| panic!("the evaluator computes it: {why}"));
        let Some(Data::Real(o)) = env.get("O") else { panic!("the evaluator has no real `O`") };
        let mut shown = src.clone();
        for i in 1..=o.len() {
            shown.push_str(&format!("Show item {i} of O.\n"));
        }
        let printed: Vec<String> = vm_outcome(&shown).output.split_whitespace().map(str::to_string).collect();
        let evaluated: Vec<String> = o.iter().map(|x| format!("{x}")).collect();
        assert_eq!(printed, evaluated, "`{gate}`, {rows} rows: the evaluator's `O` is the VM's, value for value");
    }
}

/// `C`'s entries for `O` as the spatial evaluator computes the program lowered whole for the SN50.
fn on_the_chip(src: &str) -> Vec<f64> {
    let prog = compile_program_to_spatial(src, &SpatialTarget::sn50_rdu())
        .expect("a tensor program")
        .into_whole()
        .unwrap_or_else(|why| panic!("{why}"));
    let out = eval_choreography(&prog, &inputs_from_fills(&prog)).expect("the program evaluates");
    let o = prog.tensors.iter().find(|t| t.name == "O").expect("O").id;
    match out.tensors.get(&o).expect("the evaluation hands back O") {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

#[test]
fn the_sn50_computes_a_sigmoid_gated_step() {
    for (hv, hk, d, rows) in [(4, 2, 2, 1), (6, 2, 2, 1), (4, 2, 2, 3)] {
        let s = step(hv, hk, d, rows);
        let sigmoid_src = program(&s, "sigmoid gate");
        let chip = on_the_chip(&sigmoid_src);
        let (want, _) = oracle(&s, sigmoid);
        let scale = want.iter().fold(0.0f64, |m, x| m.max(x.abs()));
        assert_eq!(chip.len(), want.len(), "O's size");
        for (i, (c, w)) in chip.iter().zip(&want).enumerate() {
            assert!((c - w).abs() <= 0.02 * scale, "(HV {hv}, HK {hk}, D {d}, {rows} rows) O[{i}]: the chip's {c}, the VM's {w}");
        }
        let silu_chip = on_the_chip(&program(&s, "gate"));
        assert!(chip.iter().zip(&silu_chip).any(|(a, b)| (a - b).abs() > 0.05 * scale), "the gate moves the chip's output");
    }
}

/// The shims of the chips with no sigmoid gate in their kernels: the CUDA lane at every knob point and the forward.
fn shims_without_the_gate() -> Vec<Box<dyn Shim>> {
    let mut out: Vec<Box<dyn Shim>> = Vec::new();
    for activations in [Q8MatKind::Dequant, Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        for fuse_quant in [true, false] {
            for batch in [false, true] {
                let knobs = CudaKnobs { activations, fuse_quant, ..CudaKnobs::default() };
                out.push(Box::new(LaneShim { knobs, batch, held: Vec::new() }));
            }
        }
        out.push(Box::new(ForwardShim { kind: activations }));
    }
    out
}

#[test]
fn every_chip_without_the_gate_refuses_it_by_name_and_every_spatial_shim_resolves_it() {
    let src = program(&step(4, 2, 2, 1), "sigmoid gate");
    let refusal = compile_to_cuda_forward(&src).expect_err("the CUDA forward has no sigmoid-gated delta kernel");
    assert!(refusal.contains("sigmoid"), "the CUDA forward names the gate it refuses: {refusal}");
    let rp = recognize_stages(&src).expect("parses");
    let g = TensorGraph::lift(&rp, None, None, &|_| Storage::Values).unwrap_or_else(|why| panic!("it lifts: {why:?}"));
    for shim in shims_without_the_gate() {
        let unresolved = resolve(&g, shim.as_ref()).expect_err(&format!("{} refuses the sigmoid gate", shim.name()));
        let refused = unresolved
            .iter()
            .find(|u| u.kind == "GatedDelta")
            .unwrap_or_else(|| panic!("{} refuses the gated delta: {unresolved:?}", shim.name()));
        assert!(refused.why.contains("sigmoid"), "{} names the gate it refuses: {}", shim.name(), refused.why);
    }
    for target in [SpatialTarget::wormhole(), SpatialTarget::sky_fabric(64, 64), SpatialTarget::sn50_rdu(), SpatialTarget::rtx3060()] {
        let shim = SpatialShim { target: Box::leak(Box::new(target)) };
        resolve(&g, &shim).unwrap_or_else(|why| panic!("{} resolves the sigmoid gate: {why:?}", shim.name()));
    }
    resolve(&g, &ReferenceShim).unwrap_or_else(|why| panic!("the reference resolves the sigmoid gate: {why:?}"));
    // The SiLU gate is refused for nothing new: no shim names a gate it refuses on it.
    let silu = recognize_stages(&program(&step(4, 2, 2, 1), "gate")).expect("parses");
    let g = TensorGraph::lift(&silu, None, None, &|_| Storage::Values).unwrap_or_else(|why| panic!("it lifts: {why:?}"));
    for shim in shims_without_the_gate() {
        let unresolved = resolve(&g, shim.as_ref()).err().unwrap_or_default();
        assert!(unresolved.iter().all(|u| !u.why.contains("sigmoid")), "{} refuses the SiLU gate for a sigmoid: {unresolved:?}", shim.name());
    }
}

#[test]
fn a_model_whose_deltanet_a_chip_cannot_gate_is_refused_by_name() {
    // The D5005 engine's delta unit and the CUDA lane's Qwen3.8 configuration gate by SiLU: each refuses a Gated
    // DeltaNet gated by a sigmoid, naming it, and takes the SiLU-gated model it always took.
    let silu = model_harness::qwen35_spec();
    let mut spec = silu.clone();
    for layer in &mut spec.layers {
        if let Mixer::GatedDelta(g) = &mut layer.mixer {
            g.gate = GateActivation::Sigmoid;
        }
    }
    let engine = spec_refusals(&spec).iter().map(|r| r.to_string()).collect::<Vec<_>>();
    assert!(engine.iter().any(|r| r.contains("sigmoid")), "the engine names the sigmoid gate: {engine:?}");
    let before = spec_refusals(&silu).iter().map(|r| r.to_string()).collect::<Vec<_>>();
    assert!(before.iter().all(|r| !r.contains("sigmoid")), "the SiLU gate is not refused for its gate: {before:?}");
    assert!(Qwen38Cfg::from_spec(&spec).is_err(), "the CUDA lane's Qwen3.8 configuration writes only the SiLU gate");
}

/// Every file of the compiler that reads a gated delta's spec, and what it does with the gate. A file that lifts it,
/// computes the op or refuses what it does not compute names the gate; one that only routes the op by its shape need not.
const CONSUMERS: &[(&str, &str)] = &[
    ("tensor_recognize/qwen.rs", "lifts"),
    ("tensor_graph/front.rs", "routes"),
    ("tensor_graph/graph.rs", "routes"),
    ("tensor_graph/numerics.rs", "routes"),
    ("tensor_graph/fuse.rs", "routes"),
    ("tensor_graph/vjp.rs", "routes"),
    ("tensor_graph/eval.rs", "computes"),
    ("spatial/link.rs", "routes"),
    ("spatial/passes/from_qwen_ops.rs", "computes"),
    ("codegen_cpu/link.rs", "computes"),
    ("codegen_cuda/forward_shim.rs", "refuses"),
    ("codegen_cuda/lane_link.rs", "refuses"),
    ("codegen_cuda/lane_shim.rs", "refuses"),
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
fn every_consumer_of_a_gated_delta_spec_is_held_to_its_gate() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let reads = |text: &str| {
        text.contains("GatedDeltaSpec") || text.contains("TensorOpSpec::GatedDelta(") || text.contains(" T::GatedDelta(")
    };
    let found: BTreeSet<String> = files
        .iter()
        .filter(|p| reads(&std::fs::read_to_string(p).unwrap()))
        .map(|p| p.strip_prefix(&src).unwrap().to_string_lossy().to_string())
        .collect();
    let named: BTreeSet<String> = CONSUMERS.iter().map(|(f, _)| f.to_string()).collect();
    assert_eq!(found, named, "the files that read a gated delta's spec are the ones this census names");
    for (file, role) in CONSUMERS {
        let text = std::fs::read_to_string(src.join(file)).unwrap();
        if matches!(*role, "lifts" | "computes" | "refuses") {
            assert!(text.contains("GateAct"), "{file} {role} a gated delta, so it reads its gate: it never names `GateAct`");
        }
    }
}
