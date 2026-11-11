//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A HYPER-CONNECTED RESIDUAL'S LANES MIX INTO ONE ROW, AND THE ROW JOINS EVERY LANE BY THE LANE'S SHARE.
//!
//!     `Let M be the mean of X over L lanes`      M[t, c] = (Σ_l X[t, l·C + c]) / L      X `R by L·C`, M `R by C`
//!     `Let P be O spread over L lanes [by S]`    P[t, l·C + c] = O[t, c] · S[t, l]       O `R by C`, S `R by L`
//!
//! Qwen3.8-Flash-Next reads each sublayer's input as the gated mean of its residual's four lanes and adds the sublayer's
//! output back into each lane by the lane's share (`qwen4_exp_hyper_lock` holds the model they write).
//!
//!   • the VM and the tree-walker compute both formulas, at one row and several, from one lane to five, the spread with
//!     its weights and without (a copy in every lane);
//!   • a lane count that does not divide the width, and weights that are not one a lane of each row, are refused by name;
//!   • the front end lifts them as `TensorOpSpec::LaneMean` and `TensorOpSpec::LaneSpread`, with their operands;
//!   • the tensor graph's evaluator computes them as the VM does, value for value (`tensor_graph_vjp_lock` holds their
//!     gradients);
//!   • the spatial chips compute them as the VM does: alone, as a section of a composition, and at Qwen3.8's width;
//!   • every shim without them refuses them by name — the CUDA lane at every knob point, the forward at each of its
//!     activations, the D5005 engine — and every spatial shim and the reference resolve them;
//!   • every file of the compiler that names either op is named here with what it does with it.
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
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
use logicaffeine_compile::spatial::decode_engine::shim::EngineShim;
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Value};
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
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

/// Entry `k` of the tensor `seed` fills: every value its own, on both sides of zero.
fn v(seed: usize, k: usize) -> f64 {
    ((seed * 31 + k * 17) % 37) as f64 / 37.0 * 2.6 - 1.25
}

/// `name` declared `rows` by `cols` and filled from `seed`.
fn declared(name: &str, rows: usize, cols: usize, seed: usize) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    for k in 0..rows * cols {
        s.push_str(&format!("Set item {} of {name} to {:?}.\n", k + 1, v(seed, k)));
    }
    s
}

/// One lane op: `R` rows of `L` lanes `C` wide; a spread `weighted` by `S` or a copy, or a mean.
#[derive(Clone, Copy, Debug)]
enum Lane {
    Mean { r: usize, l: usize, c: usize },
    Spread { r: usize, l: usize, c: usize, weighted: bool },
}

impl Lane {
    /// Its program binding `Y`, then `tail`.
    fn program_with(self, tail: &str) -> String {
        let mut s = String::from("## Main\n");
        match self {
            Lane::Mean { r, l, c } => {
                s.push_str(&declared("X", r, l * c, 1));
                s.push_str(&format!("Let Y be the mean of X over {l} lanes.\n"));
            }
            Lane::Spread { r, l, c, weighted } => {
                s.push_str(&declared("O", r, c, 2));
                if weighted {
                    s.push_str(&declared("S", r, l, 3));
                    s.push_str(&format!("Let Y be O spread over {l} lanes by S.\n"));
                } else {
                    s.push_str(&format!("Let Y be O spread over {l} lanes.\n"));
                }
            }
        }
        s.push_str(tail);
        s
    }

    fn program(self) -> String {
        self.program_with("")
    }

    /// `Y`'s entries.
    fn len(self) -> usize {
        match self {
            Lane::Mean { r, c, .. } => r * c,
            Lane::Spread { r, l, c, .. } => r * l * c,
        }
    }

    /// The formula, in the order the sugar computes it.
    fn oracle(self) -> Vec<f64> {
        match self {
            Lane::Mean { r, l, c } => (0..r)
                .flat_map(|t| {
                    (0..c).map(move |j| {
                        let mut s = 0.0f64;
                        for lane in 0..l {
                            s = s + v(1, t * l * c + lane * c + j);
                        }
                        s / l as f64
                    })
                })
                .collect(),
            Lane::Spread { r, l, c, weighted } => (0..r)
                .flat_map(|t| {
                    (0..l).flat_map(move |lane| {
                        (0..c).map(move |j| v(2, t * c + j) * if weighted { v(3, t * l + lane) } else { 1.0 })
                    })
                })
                .collect(),
        }
    }
}

const CASES: [Lane; 9] = [
    Lane::Mean { r: 1, l: 4, c: 3 },
    Lane::Mean { r: 2, l: 4, c: 3 },
    Lane::Mean { r: 3, l: 2, c: 5 },
    Lane::Mean { r: 2, l: 5, c: 2 },
    Lane::Mean { r: 1, l: 1, c: 4 },
    Lane::Spread { r: 1, l: 4, c: 3, weighted: true },
    Lane::Spread { r: 3, l: 2, c: 5, weighted: true },
    Lane::Spread { r: 2, l: 5, c: 2, weighted: true },
    Lane::Spread { r: 2, l: 3, c: 4, weighted: false },
];

/// `Y`'s `n` entries as the VM computes them, the tree-walker agreeing.
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
        let got = on_the_vm(&k.program(), k.len());
        let want = k.oracle();
        assert_eq!(got.len(), want.len(), "{k:?}: Y's size");
        for (i, (g, o)) in got.iter().zip(&want).enumerate() {
            assert!((g - o).abs() <= 1e-12 * o.abs().max(1.0), "{k:?}: Y[{i}] is {g} on the VM, the formula's {o}");
        }
    }
}

#[test]
fn a_lane_count_or_weights_that_do_not_fit_are_refused_by_name() {
    for (src, what, names) in [
        (format!("## Main\n{}Let Y be the mean of X over 3 lanes.\n", declared("X", 1, 8, 1)), "a mean over lanes that do not divide 8", "lanes"),
        (format!("## Main\n{}Let Y be the mean of X over 0 lanes.\n", declared("X", 1, 8, 1)), "a mean over no lanes", "lanes"),
        (format!("## Main\n{}{}Let Y be O spread over 3 lanes by S.\n", declared("O", 2, 4, 2), declared("S", 2, 2, 3)), "weights for two lanes of three", "lane"),
        (format!("## Main\n{}{}Let Y be O spread over 2 lanes by S.\n", declared("O", 2, 4, 2), declared("S", 1, 2, 3)), "weights for one row of two", "rows"),
    ] {
        let refusal = format!("{:?}", recognize_stages(&src).err().unwrap_or_else(|| panic!("{what} is refused")));
        assert!(refusal.contains(names), "{what}: the refusal names its {names}: {refusal}");
    }
}

#[test]
fn the_front_end_lifts_them_with_their_operands() {
    for k in CASES {
        let rp = recognize_stages(&k.program()).expect("parses");
        assert!(rp.unrecognized.is_empty(), "{k:?} lifts whole: {:?}", rp.unrecognized);
        let st = rp.stages.into_iter().find(|s| s.out == "Y").expect("a stage writes Y");
        match (k, st.op) {
            (Lane::Mean { r, l, c }, TensorOpSpec::LaneMean(m)) => {
                assert_eq!((m.rows, m.lanes, m.cols), (r, l, c), "{k:?}: its shape");
                assert_eq!(st.ins, ["X"], "{k:?}: X its operand");
            }
            (Lane::Spread { r, l, c, weighted }, TensorOpSpec::LaneSpread(p)) => {
                assert_eq!((p.rows, p.lanes, p.cols, p.by.is_some()), (r, l, c, weighted), "{k:?}: its shape and weights");
                let want: &[&str] = if weighted { &["O", "S"] } else { &["O"] };
                assert_eq!(st.ins, want, "{k:?}: its operands");
            }
            (k, other) => panic!("{k:?} lifts as {other:?}"),
        }
    }
}

#[test]
fn the_graph_evaluator_computes_them_as_the_vm_does() {
    for k in CASES {
        let src = k.program();
        let g = TensorGraph::lift(&recognize_stages(&src).expect("parses"), None, None, &|_| Storage::Values)
            .unwrap_or_else(|why| panic!("{k:?} lifts: {why:?}"));
        let env = eval(&g, &BTreeMap::new()).unwrap_or_else(|why| panic!("the evaluator computes {k:?}: {why}"));
        let Some(Data::Real(values)) = env.get("Y") else { panic!("{k:?}: the evaluator has no real `Y`") };
        let printed: Vec<String> = on_the_vm(&src, k.len()).iter().map(|x| format!("{x}")).collect();
        let evaluated: Vec<String> = values.iter().map(|x| format!("{x}")).collect();
        assert_eq!(evaluated, printed, "{k:?}: the evaluator's `Y` is the VM's, value for value");
    }
}

fn every_tensor(prog: &logicaffeine_compile::spatial::ir::SpatialProgram, out: &str) -> Vec<f64> {
    let values = eval_choreography_every_tensor(prog, &inputs_from_fills(prog)).expect("the program evaluates");
    let id = prog.tensors.iter().find(|t| t.name == out).unwrap_or_else(|| panic!("the program holds {out}")).id;
    match values.tensors.get(&id).unwrap_or_else(|| panic!("the evaluation hands back {out}")) {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

fn close(what: &str, chip: &[f64], vm: &[f64]) {
    assert_eq!(chip.len(), vm.len(), "{what}: the size");
    let scale = vm.iter().fold(0.0f64, |m, x| m.max(x.abs())).max(1e-9);
    for (i, (a, b)) in chip.iter().zip(vm).enumerate() {
        assert!((a - b).abs() <= 0.02 * scale, "{what}: entry {i} is {a}, the VM's {b}");
    }
}

#[test]
fn the_spatial_chips_compute_them_as_the_vm_does() {
    // Qwen3.8's width: four lanes of 2,560, the mean's input and the spread's output 10,240 wide, past one site's page.
    let wide = [Lane::Mean { r: 1, l: 4, c: 2560 }, Lane::Spread { r: 1, l: 4, c: 2560, weighted: true }];
    for target in [SpatialTarget::sn50_rdu(), SpatialTarget::wormhole()] {
        let sn50 = target.name == SpatialTarget::sn50_rdu().name;
        for k in CASES.iter().copied().chain(if sn50 { wide.to_vec() } else { Vec::new() }) {
            let vm = on_the_vm(&k.program(), k.len());
            let alone = compile_program_to_spatial(&k.program(), &target)
                .unwrap_or_else(|why| panic!("{k:?} alone lowers for {}: {why:?}", target.name))
                .into_whole()
                .unwrap_or_else(|why| panic!("{why}"));
            close(&format!("{k:?} alone on {}", target.name), &every_tensor(&alone, "Y"), &vm);
            let section = compile_program_to_spatial(&k.program_with("Let Z be Y plus Y.\n"), &target)
                .unwrap_or_else(|why| panic!("{k:?} as a section lowers for {}: {why:?}", target.name))
                .into_whole()
                .unwrap_or_else(|why| panic!("{why}"));
            let doubled: Vec<f64> = vm.iter().map(|y| y + y).collect();
            close(&format!("{k:?} as a section on {}", target.name), &every_tensor(&section, "Z"), &doubled);
        }
    }
}

/// The shims of the chips without a lane mean or spread: the CUDA lane at every knob point, the forward at each of its
/// activations, the D5005 engine for both of its tiny models.
fn shims_without_lanes() -> Vec<Box<dyn Shim>> {
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
    for (k, kind) in [(CASES[0], "LaneMean"), (CASES[5], "LaneSpread"), (CASES[8], "LaneSpread")] {
        let g = TensorGraph::lift(&recognize_stages(&k.program()).expect("parses"), None, None, &|_| Storage::Values).expect("lifts");
        for shim in shims_without_lanes() {
            let unresolved = resolve(&g, shim.as_ref()).expect_err(&format!("{} refuses {k:?}", shim.name()));
            let refused = unresolved.iter().find(|u| u.kind == kind).unwrap_or_else(|| panic!("{} refuses the {kind}: {unresolved:?}", shim.name()));
            assert!(
                refused.why.contains("lanes") || refused.why.contains(kind),
                "{} names the {kind} it refuses: {}",
                shim.name(),
                refused.why
            );
        }
        for target in [SpatialTarget::wormhole(), SpatialTarget::sky_fabric(64, 64), SpatialTarget::sn50_rdu(), SpatialTarget::rtx3060()] {
            let shim = SpatialShim { target: Box::leak(Box::new(target)) };
            resolve(&g, &shim).unwrap_or_else(|why| panic!("{} resolves {k:?}: {why:?}", shim.name()));
        }
        resolve(&g, &ReferenceShim).unwrap_or_else(|why| panic!("the reference resolves {k:?}: {why:?}"));
    }
}

/// Every file of the compiler that names a lane mean or a lane spread, and what it does with them. A file that lifts
/// or computes them, or refuses them, names both.
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
fn every_consumer_of_a_lane_mean_or_spread_is_named() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let names = |text: &str| ["LaneMean", "LaneSpread"].iter().any(|n| text.contains(n));
    let found: BTreeSet<String> = files
        .iter()
        .filter(|p| names(&std::fs::read_to_string(p).unwrap()))
        .map(|p| p.strip_prefix(&src).unwrap().to_string_lossy().to_string())
        .collect();
    let named: BTreeSet<String> = CONSUMERS.iter().map(|(f, _)| f.to_string()).collect();
    assert_eq!(found, named, "the files that name a lane mean or spread are the ones this census names");
    for (file, role) in CONSUMERS {
        let text = std::fs::read_to_string(src.join(file)).unwrap();
        if matches!(*role, "lifts" | "computes" | "refuses") {
            for op in ["LaneMean", "LaneSpread"] {
                assert!(text.contains(op), "{file} {role} the lane ops, so it names {op}");
            }
        }
    }
}
