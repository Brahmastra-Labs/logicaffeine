//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE LANES' GATE: A SUM IN EACH LANE, AND A SIGNED ROOT.
//!
//!     `Let G be the lane sums of X over L lanes`     G[t, l] = Σ_c X[t, l·C + c]              X `R by L·C`, G `R by L`
//!     `Let Y be the signed root of X`                Y = sign(x)·√max(|x|, E)                 E 1e-6 unless given
//!
//! Qwen3.8-Flash-Next's n-gram embedding gates each lane by `σ(sign(g)·√|g|)` of its lane's agreement `g`, the sum over
//! the lane of its normed key times its normed query (`the lane sums of K hadamard Q`, then `the signed root`).
//!
//!   • the VM and the tree-walker compute both formulas: every lane count from one to five, a root at zero, below its floor
//!     and on both sides of zero;
//!   • the front end lifts them as `TensorOpSpec::LaneSums` and `TensorOpSpec::SignedRoot`, and the graph's evaluator
//!     computes them as the VM does, value for value (`tensor_graph_vjp_lock` holds their gradients);
//!   • the SN50 and Wormhole compute them as the formula does, entry by entry, alone and as a section of a composition —
//!     a root below its floor `±√ε` by its sign (`SfpuKind::Sign`), and `0` at zero;
//!   • a lane count that does not divide the width, and a floor that is not positive, are refused by name;
//!   • the sign is one primitive every printer speaks: the oracle, `sign_tile` and its golden on Tensix, a `mux` of two
//!     comparisons in the Spatial language;
//!   • every shim without them refuses them by name, and every spatial shim and the reference resolve them;
//!   • every file of the compiler that names either is named here with what it does with it.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;
#[path = "common/spatial.rs"]
mod spatial_support;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use logicaffeine_compile::codegen_cuda::forward_shim::ForwardShim;
use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane_shim::LaneShim;
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
use logicaffeine_compile::spatial::decode_engine::shim::EngineShim;
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Value};
use logicaffeine_compile::spatial::ir::{
    AccPolicy, ComputeOp, IterKind, IterSpec, NumericsClaim, SfpuKind, SpatialProgram, StageKind, TensorBody, TensorInit,
    TensorRole, TileRect, WorkItem, WorkItemId,
};
use logicaffeine_compile::spatial::lower::spatial_lang::{emit_spatial_program_checked, SpatialCheck, SpatialFlavor};
use logicaffeine_compile::spatial::lower::tensix::TensixBackend;
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::passes::from_tensor_op::{ident_map, Template};
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
use logicaffeine_compile::spatial::passes::{LV_GEN_COL, LV_GEN_ROW};
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, run_passes};
use logicaffeine_compile::spatial::shim::SpatialShim;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_graph::eval::{eval, Data};
use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
use logicaffeine_compile::tensor_graph::link::resolve;
use logicaffeine_compile::tensor_graph::shim::{ReferenceShim, Shim};
use spatial_support::inputs_from_fills;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tiny_models::Tiny;

/// Entry `k` of the input: on both sides of zero, one exactly zero and one below the root's floor.
fn v(k: usize) -> f64 {
    match k % 11 {
        3 => 0.0,
        7 => 4e-7,
        _ => ((k * 17 + 5) % 23) as f64 / 23.0 * 3.0 - 1.4,
    }
}

/// `## Main` and `X` declared `rows` by `cols`, filled from [`v`].
fn declared(rows: usize, cols: usize) -> String {
    let mut s = format!("## Main\nLet X be a {rows} by {cols} tensor of Real.\n");
    for k in 0..rows * cols {
        s.push_str(&format!("Set item {} of X to {:?}.\n", k + 1, v(k)));
    }
    s
}

#[derive(Clone, Copy, Debug)]
enum Gate {
    Sums { r: usize, l: usize, c: usize },
    Root { rows: usize, cols: usize, eps: Option<f64> },
}

impl Gate {
    fn input(self) -> (usize, usize) {
        match self {
            Gate::Sums { r, l, c } => (r, l * c),
            Gate::Root { rows, cols, .. } => (rows, cols),
        }
    }

    fn len(self) -> usize {
        match self {
            Gate::Sums { r, l, .. } => r * l,
            Gate::Root { rows, cols, .. } => rows * cols,
        }
    }

    fn program_with(self, tail: &str) -> String {
        let (rows, cols) = self.input();
        let mut s = declared(rows, cols);
        s.push_str(&match self {
            Gate::Sums { l, .. } => format!("Let Y be the lane sums of X over {l} lanes.\n"),
            Gate::Root { eps: None, .. } => "Let Y be the signed root of X.\n".to_string(),
            Gate::Root { eps: Some(e), .. } => format!("Let Y be the signed root of X with epsilon {e:?}.\n"),
        });
        s.push_str(tail);
        s
    }

    fn program(self) -> String {
        self.program_with("")
    }

    fn oracle(self) -> Vec<f64> {
        self.oracle_with_magnitude().into_iter().map(|(y, _)| y).collect()
    }

    /// Each entry of `Y` with the magnitude its rounding scales by: the sum of its lane's `|x|`, or the root's own size.
    fn oracle_with_magnitude(self) -> Vec<(f64, f64)> {
        match self {
            Gate::Sums { r, l, c } => (0..r * l)
                .map(|u| {
                    let (mut s, mut m) = (0.0f64, 0.0f64);
                    for e in 0..c {
                        s = s + v(u * c + e);
                        m += v(u * c + e).abs();
                    }
                    (s, m)
                })
                .collect(),
            Gate::Root { rows, cols, eps } => (0..rows * cols)
                .map(|k| {
                    let x = v(k);
                    let sign = if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 };
                    let y = sign * x.abs().max(eps.unwrap_or(1e-6)).sqrt();
                    (y, y.abs())
                })
                .collect(),
        }
    }
}

const CASES: [Gate; 8] = [
    Gate::Sums { r: 1, l: 4, c: 3 },
    Gate::Sums { r: 2, l: 4, c: 3 },
    Gate::Sums { r: 3, l: 2, c: 5 },
    Gate::Sums { r: 2, l: 5, c: 2 },
    Gate::Sums { r: 1, l: 1, c: 4 },
    Gate::Root { rows: 1, cols: 12, eps: None },
    Gate::Root { rows: 2, cols: 11, eps: None },
    Gate::Root { rows: 1, cols: 12, eps: Some(0.01) },
];

fn on_the_vm(src: &str, n: usize) -> Vec<f64> {
    let mut prog = src.to_string();
    for i in 1..=n {
        prog.push_str(&format!("Show item {i} of Y.\n"));
    }
    let vm = vm_outcome(&prog);
    assert!(vm.error.is_none(), "the VM runs the program: {:?}", vm.error);
    let tw = tw_outcome(&prog);
    assert!(tw.error.is_none(), "the tree-walker runs the program: {:?}", tw.error);
    assert_eq!(tw.output.trim_end(), vm.output.trim_end(), "the tree-walker agrees with the VM");
    vm.output.split_whitespace().map(|t| t.parse().expect("a number")).collect()
}

#[test]
fn the_vm_and_the_tree_walker_compute_both_formulas() {
    for k in CASES {
        let (got, want) = (on_the_vm(&k.program(), k.len()), k.oracle());
        assert_eq!(got.len(), want.len(), "{k:?}: Y's size");
        for (i, (g, o)) in got.iter().zip(&want).enumerate() {
            assert!((g - o).abs() <= 1e-12 * o.abs().max(1.0), "{k:?}: Y[{i}] is {g} on the VM, the formula's {o}");
        }
    }
}

#[test]
fn the_front_end_lifts_them_and_the_evaluator_computes_them_as_the_vm_does() {
    for k in CASES {
        let src = k.program();
        let rp = recognize_stages(&src).expect("parses");
        assert!(rp.unrecognized.is_empty(), "{k:?} lifts whole: {:?}", rp.unrecognized);
        let st = rp.stages.iter().find(|s| s.out == "Y").expect("a stage writes Y");
        match (k, &st.op) {
            (Gate::Sums { r, l, c }, TensorOpSpec::LaneSums(s)) => assert_eq!((s.rows, s.lanes, s.cols), (r, l, c), "{k:?}"),
            (Gate::Root { rows, cols, eps }, TensorOpSpec::SignedRoot(s)) => {
                assert_eq!((s.n, s.eps.get()), (rows * cols, eps.unwrap_or(1e-6)), "{k:?}")
            }
            (k, other) => panic!("{k:?} lifts as {other:?}"),
        }
        assert_eq!(st.ins, ["X"], "{k:?}: X its operand");
        let g = TensorGraph::lift(&rp, None, None, &|_| Storage::Values).unwrap_or_else(|why| panic!("{k:?} lifts: {why:?}"));
        let env = eval(&g, &BTreeMap::new()).unwrap_or_else(|why| panic!("the evaluator computes {k:?}: {why}"));
        let Some(Data::Real(values)) = env.get("Y") else { panic!("{k:?}: the evaluator has no real `Y`") };
        let printed: Vec<String> = on_the_vm(&src, k.len()).iter().map(|x| format!("{x}")).collect();
        let evaluated: Vec<String> = values.iter().map(|x| format!("{x}")).collect();
        assert_eq!(evaluated, printed, "{k:?}: the evaluator's `Y` is the VM's, value for value");
    }
}

fn every_tensor(prog: &SpatialProgram, out: &str) -> Vec<f64> {
    let values = eval_choreography_every_tensor(prog, &inputs_from_fills(prog)).expect("the program evaluates");
    let id = prog.tensors.iter().find(|t| t.name == out).unwrap_or_else(|| panic!("the program holds {out}")).id;
    match values.tensors.get(&id).unwrap_or_else(|| panic!("the evaluation hands back {out}")) {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

/// The chip's `Y` against the formula's, entry by entry: each within 2% of its own magnitude, so a root below its floor
/// is held to `±√ε` and not to the size of the largest entry.
fn close(what: &str, chip: &[f64], want: &[(f64, f64)]) {
    assert_eq!(chip.len(), want.len(), "{what}: the size");
    for (i, (a, (b, m))) in chip.iter().zip(want).enumerate() {
        assert!((a - b).abs() <= 0.02 * m + 1e-12, "{what}: entry {i} is {a}, the formula's {b}");
    }
}

#[test]
fn the_spatial_chips_compute_them_as_the_formula_does() {
    for target in [SpatialTarget::sn50_rdu(), SpatialTarget::wormhole()] {
        for k in CASES {
            let want = k.oracle_with_magnitude();
            let alone = compile_program_to_spatial(&k.program(), &target)
                .unwrap_or_else(|why| panic!("{k:?} alone lowers for {}: {why:?}", target.name))
                .into_whole()
                .unwrap_or_else(|why| panic!("{why}"));
            close(&format!("{k:?} alone on {}", target.name), &every_tensor(&alone, "Y"), &want);
            let section = compile_program_to_spatial(&k.program_with("Let Z be Y plus Y.\n"), &target)
                .unwrap_or_else(|why| panic!("{k:?} as a section lowers for {}: {why:?}", target.name))
                .into_whole()
                .unwrap_or_else(|why| panic!("{why}"));
            let doubled: Vec<(f64, f64)> = want.iter().map(|&(y, m)| (y + y, m + m)).collect();
            close(&format!("{k:?} as a section on {}", target.name), &every_tensor(&section, "Z"), &doubled);
        }
    }
}

#[test]
fn a_lane_count_that_does_not_fit_or_a_floor_that_is_not_positive_is_refused_by_name() {
    let x = declared(1, 8);
    for (tail, what, names) in [
        ("Let Y be the lane sums of X over 3 lanes.\n", "lane sums over lanes that do not divide 8", "lanes"),
        ("Let Y be the lane sums of X over 0 lanes.\n", "lane sums over no lanes", "lanes"),
        ("Let Y be the signed root of X with epsilon 0.\n", "a root with no floor", "epsilon"),
        ("Let Y be the signed root of X with epsilon -0.5.\n", "a root with a negative floor", "epsilon"),
    ] {
        let refusal = format!("{:?}", recognize_stages(&format!("{x}{tail}")).err().unwrap_or_else(|| panic!("{what} is refused")));
        assert!(refusal.contains(names), "{what}: the refusal names its {names}: {refusal}");
    }
}

/// `y = sign(x)` alone, one Generic over one tile row: negatives, zero, positives, and a value either side of zero far
/// below any floor.
fn bare_sign(target: &SpatialTarget) -> (SpatialProgram, Vec<f64>) {
    let w = target.tile.cols;
    let xs: Vec<f64> = (0..w)
        .map(|c| match c {
            1 => 1e-30,
            2 => -1e-30,
            _ => (c as f64 - (w / 2) as f64) * 0.37,
        })
        .collect();
    let signs = xs.iter().map(|&x| if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 }).collect();
    let mut tp = Template::new(target).expect("a template builder");
    let xt = tp.tensor("x", TensorRole::Input, 1, w, Precision::Fp32, TensorInit::FloatValues(xs.iter().map(|x| x.to_bits()).collect()));
    let xb = tp.buffer("in_x", 1, Precision::Fp32);
    let l = tp.load("load_x", xt, vec![], 1, xb);
    let yt = tp.tensor("y", TensorRole::Output, 1, w, Precision::Fp32, TensorInit::Zero);
    let yb = tp.buffer("out", 1, Precision::Fp32);
    let iters = vec![
        IterSpec { var: LV_GEN_ROW, extent: 1, kind: IterKind::Parallel },
        IterSpec { var: LV_GEN_COL, extent: w, kind: IterKind::Parallel },
    ];
    let body = TensorBody::Unary { kind: SfpuKind::Sign, arg: Box::new(TensorBody::Operand(0)) };
    let g = tp.stage(
        "sign",
        StageKind::Compute(ComputeOp::Generic { iters, ins: vec![ident_map(w)], out: ident_map(w), body }),
        vec![xb],
        vec![yb],
    );
    let st = tp.store("store_y", yt, vec![], 1, yb);
    tp.edge(l, xb, g);
    tp.edge(g, yb, st);
    let items = vec![WorkItem { id: WorkItemId::new(0), coord: (0, 0), out: TileRect { row0_t: 0, col0_t: 0, h_t: 1, w_t: 1 } }];
    let p = tp.finish("bare_sign".into(), items, AccPolicy::F32, NumericsClaim::Exact, target).expect("the sign validates");
    (run_passes(p, target).expect("the sign verifies"), signs)
}

#[test]
fn the_sign_is_one_primitive_every_printer_speaks() {
    for target in [SpatialTarget::sn50_rdu(), SpatialTarget::wormhole(), SpatialTarget::sn40l_rdu()] {
        let (p, signs) = bare_sign(&target);
        assert_eq!(every_tensor(&p, "y"), signs, "the oracle's sign on {}: 1 above zero, −1 below, 0 at it", target.name);
    }
    let wormhole = SpatialTarget::wormhole();
    let art = match TensixBackend::new(wormhole.clone()).lower(&bare_sign(&wormhole).0) {
        Ok(SpatialArtifact::Tensix(a)) => a,
        other => panic!("the sign lowers to Tensix: {other:?}"),
    };
    let kernels: String = art.kernels.iter().map(|k| k.source.as_str()).collect();
    for needle in ["api/compute/compute_kernel_api.h", "sign_tile_init();", "sign_tile("] {
        assert!(kernels.contains(needle), "the compute kernel names `{needle}`");
    }
    assert!(art.host_cpp.contains("> 0.0) - ("), "the host's golden computes the sign by comparison:\n{}", art.host_cpp);
    let rdu = SpatialTarget::sn40l_rdu();
    let (p, signs) = bare_sign(&rdu);
    let check = [SpatialCheck { tensor: "y".into(), offset: 0, gold: signs, rel_tol: 0.0 }];
    for flavor in [SpatialFlavor::Simulator, SpatialFlavor::Fabric] {
        let src = emit_spatial_program_checked(&p, &rdu, &check, flavor).unwrap_or_else(|why| panic!("the sign prints for {flavor:?}: {why:?}"));
        assert!(src.contains("mux(") && src.contains(" > 0.to[") && src.contains(" < 0.to["), "{flavor:?} spells the sign as two comparisons and a mux");
    }
}

fn shims_without_them() -> Vec<Box<dyn Shim>> {
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
    for tiny in [Tiny::Qwen35, Tiny::Qwen3] {
        out.push(Box::new(EngineShim { spec: Box::leak(Box::new(tiny.spec(24))), numerics: EngineNumerics::precise() }));
    }
    out
}

#[test]
fn every_shim_without_them_refuses_them_by_name_and_every_spatial_shim_resolves_them() {
    for (k, kind) in [(CASES[0], "LaneSums"), (CASES[5], "SignedRoot")] {
        let g = TensorGraph::lift(&recognize_stages(&k.program()).expect("parses"), None, None, &|_| Storage::Values).expect("lifts");
        for shim in shims_without_them() {
            let unresolved = resolve(&g, shim.as_ref()).expect_err(&format!("{} refuses {k:?}", shim.name()));
            let refused = unresolved.iter().find(|u| u.kind == kind).unwrap_or_else(|| panic!("{} refuses the {kind}: {unresolved:?}", shim.name()));
            let named = ["lane sum", "signed root", kind].iter().any(|w| refused.why.contains(w));
            assert!(named, "{} names the {kind} it refuses: {}", shim.name(), refused.why);
        }
        for target in [SpatialTarget::wormhole(), SpatialTarget::sky_fabric(64, 64), SpatialTarget::sn50_rdu(), SpatialTarget::rtx3060()] {
            let shim = SpatialShim { target: Box::leak(Box::new(target)) };
            resolve(&g, &shim).unwrap_or_else(|why| panic!("{} resolves {k:?}: {why:?}", shim.name()));
        }
        resolve(&g, &ReferenceShim).unwrap_or_else(|why| panic!("the reference resolves {k:?}: {why:?}"));
    }
}

const CONSUMERS: &[(&str, &str)] = &[
    ("tensor_recognize/qwen.rs", "lifts"),
    ("tensor_graph/front.rs", "lifts"),
    ("tensor_graph/graph.rs", "routes"),
    ("tensor_graph/eval.rs", "computes"),
    ("tensor_graph/vjp.rs", "computes"),
    ("spatial/link.rs", "computes"),
    ("spatial/passes/from_qwen_ops.rs", "computes"),
    ("codegen_cpu/link.rs", "computes"),
    ("codegen_cuda/forward_shim.rs", "refuses"),
    ("codegen_cuda/lane_link.rs", "refuses"),
    ("codegen_cuda/lane_shim.rs", "refuses"),
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
fn every_consumer_of_a_lane_sum_or_a_signed_root_is_named() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let names = |text: &str| ["LaneSums", "SignedRoot"].iter().any(|n| text.contains(n));
    let found: BTreeSet<String> = files
        .iter()
        .filter(|p| names(&std::fs::read_to_string(p).unwrap()))
        .map(|p| p.strip_prefix(&src).unwrap().to_string_lossy().to_string())
        .collect();
    let named: BTreeSet<String> = CONSUMERS.iter().map(|(f, _)| f.to_string()).collect();
    assert_eq!(found, named, "the files that name a lane sum or a signed root are the ones this census names");
    for (file, role) in CONSUMERS {
        let text = std::fs::read_to_string(src.join(file)).unwrap();
        if matches!(*role, "lifts" | "computes" | "refuses") {
            for op in ["LaneSums", "SignedRoot"] {
                assert!(text.contains(op), "{file} {role} the gate's ops, so it names {op}");
            }
        }
    }
}
