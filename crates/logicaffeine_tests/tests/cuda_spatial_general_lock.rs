//! cuda_spatial_general_lock — the GPU programs of a spatial program, run on the RTX 3060 against the evaluator.
//!
//! A program the tile GEMM kernel cannot hold — Generic bodies with their folds, contractions and selects, gathers
//! that read an index from data, compositions of sections under their own plans, sites that hand off through
//! semaphores — prints two ways (`spatial::lower::cuda`'s general path):
//! - the general program: one block per site, every buffer a shared-memory ring at its own precision, the accumulator
//!   in shared memory, each op the evaluator's, the engines handing off through rings and semaphores as they run;
//! - where every ring and accumulator fits one block, the one-block program: every engine's ops in the order running
//!   their flow control at compile time fixes, the program the backend hands back, the general one beside it.
//!
//! This lock compiles each with nvcc for sm_86, runs it on the GPU, and holds every result it prints to
//! `eval_choreography`:
//!
//! - each integer op's own program — requant, clamp, relu, a sum, a hadamard, an arg-max, a matmul, a row gather, a
//!   row write and a LUT attention — on operands drawn across their ranges;
//! - an int8 GEMM's fused epilogue, the accumulator requantized, biased and clipped by the epilogue ops themselves;
//! - the int8 decode step at every position of the prompt and the tokens after it, the cache carried from step to step:
//!   the next token and the whole cache the evaluator's, a 38-section program over three sites and their semaphores;
//! - the decode step planned for single-element tiles, whose matmuls pack a whole block into an output the plan declares
//!   one tile per accumulator lane (the tile GEMM kernel's register epilogue never stages it);
//! - folds split over a warp's lanes: an arg-max whose tied maxima sit in two lanes with the lower index in the higher
//!   lane, in one lane, everywhere, and at the last element, and a LUT attention whose sum and max fold five positions
//!   over four lanes.
//!
//! And without a GPU: a page holds what the op that fills it writes, so every ring of both programs is laid out at
//! `SpatialProgram::page_extents` — at least each buffer's declared page, and each pack, tile walk and reload it carries.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test cuda_spatial_general_lock` (nvcc- and GPU-gated)

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/spatial.rs"]
mod spatial_support;

use std::collections::BTreeMap;

use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::codegen_sva::systolic::{FusedBlock, Systolic};
use logicaffeine_compile::spatial::ir::{walk_tagged_ops, BufferKind, ChoreoOp, ComputeOp, SpatialProgram, StageKind, TensorInit, TensorRole};
use logicaffeine_compile::spatial::passes::from_tensor_op::from_fused_block;
use logicaffeine_compile::spatial::pipeline::run_passes;
use logicaffeine_compile::spatial::lower::cuda::{CudaArtifact, CudaPersistentBackend};
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{SpatialTarget, TileShape};
use spatial_support::inputs_from_fills;

fn model() -> (Int8Model, Vec<usize>, Vec<usize>) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let tokens = |k: &str| -> Vec<usize> {
        j["golden_trace"][k].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as usize).collect()
    };
    (Int8Model::from_json(&j).expect("the model"), tokens("prompt_tokens"), tokens("generated_tokens"))
}

/// The RTX 3060 planning for single-element tiles: every matmul a block of scalar tiles, packed whole into an output the
/// plan declares one tile per accumulator lane.
fn rtx3060_scalar() -> SpatialTarget {
    let mut t = SpatialTarget::rtx3060();
    t.tile = TileShape::SCALAR;
    t
}

fn scheduled(src: &str) -> SpatialProgram {
    scheduled_on(src, &SpatialTarget::rtx3060())
}

fn scheduled_on(src: &str, target: &SpatialTarget) -> SpatialProgram {
    let l = compile_program_to_spatial(src, target).unwrap_or_else(|e| panic!("a tensor program: {e:?}\n{src}"));
    l.program.unwrap_or_else(|| panic!("the program lowers whole on {}: {:?}\n{src}", target.name, l.missing.first()))
}

fn lower(p: &SpatialProgram, target: &SpatialTarget) -> CudaArtifact {
    match CudaPersistentBackend::new(target.clone()).lower(p) {
        Ok(SpatialArtifact::Cuda(a)) => a,
        other => panic!("the GPU prints the program: {other:?}"),
    }
}

/// Compile `cu` for sm_86 and run it beside `files`; what it printed.
fn run_with(label: &str, cu: &str, files: &[(&str, String)]) -> String {
    let dir = std::env::temp_dir().join(format!("cuda_general_{}_{}", std::process::id(), label.replace(' ', "_")));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, bin) = (dir.join("program.cu"), dir.join("program"));
    std::fs::write(&src, cu).unwrap();
    for (name, body) in files {
        std::fs::write(dir.join(name), body).unwrap();
    }
    let build = std::process::Command::new("nvcc").args(["-O3", "-arch=sm_86", "-std=c++17", "-o"]).arg(&bin).arg(&src).output().expect("nvcc");
    assert!(build.status.success(), "{label}: nvcc failed:\n{}", String::from_utf8_lossy(&build.stderr));
    let out = std::process::Command::new(&bin).current_dir(&dir).output().expect("run");
    assert!(out.status.success(), "{label}: the program failed: {}", String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Compile `cu` for sm_86, run it, and read every `OUT <tensor> <element> <value>` it prints, by tensor.
fn run(label: &str, cu: &str) -> BTreeMap<usize, Vec<i64>> {
    let stdout = run_with(label, cu, &[]);
    let mut got: BTreeMap<usize, Vec<(usize, i64)>> = BTreeMap::new();
    for line in stdout.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.first() == Some(&"OUT") {
            got.entry(t[1].parse().unwrap()).or_default().push((t[2].parse().unwrap(), t[3].parse().unwrap()));
        }
    }
    got.into_iter()
        .map(|(id, mut v)| {
            v.sort();
            (id, v.into_iter().map(|(_, x)| x).collect())
        })
        .collect()
}

/// Each GPU program's results for every `Output` and `State` tensor against the evaluator's — the program the backend
/// hands back and, beside a one-block program, the general one — and the results of the one handed back.
fn gpu_equals_evaluator(label: &str, p: &SpatialProgram) -> BTreeMap<usize, Vec<i64>> {
    gpu_equals_evaluator_on(label, p, &SpatialTarget::rtx3060())
}

fn gpu_equals_evaluator_on(label: &str, p: &SpatialProgram, target: &SpatialTarget) -> BTreeMap<usize, Vec<i64>> {
    let art = lower(p, target);
    let got = program_equals_evaluator(label, p, &art.program_cu);
    if let Some(general) = &art.general {
        program_equals_evaluator(&format!("{label} (one block per site)"), p, &general.program_cu);
    }
    got
}

fn program_equals_evaluator(label: &str, p: &SpatialProgram, cu: &str) -> BTreeMap<usize, Vec<i64>> {
    let inputs = inputs_from_fills(p);
    let want = eval_choreography(p, &inputs).unwrap_or_else(|e| panic!("{label}: the evaluator: {e:?}"));
    let cu = cu.to_string();
    let got = if cu.contains("fopen(\"a.txt\"") {
        // A lone tile matmul is the tile GEMM kernel's: its operands from `a.txt` and `b.txt`, its product printed a
        // value a line.
        let text = |i: usize| match &inputs.tensors[&p.tensors[i].id] {
            Value::Int(v) => v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("\n"),
            Value::Float(v) => panic!("{label}: a float operand {v:?}"),
        };
        let out = p.output_tensor().expect("an output");
        let values: Vec<i64> = run_with(label, &cu, &[("a.txt", text(0)), ("b.txt", text(1))])
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        BTreeMap::from([(out.id.idx(), values)])
    } else {
        run(label, &cu)
    };
    for t in p.tensors.iter().filter(|t| matches!(t.role, TensorRole::Output | TensorRole::State)) {
        let Some(Value::Int(w)) = want.tensors.get(&t.id) else { continue };
        let g = got.get(&t.id.idx()).unwrap_or_else(|| panic!("{label}: the GPU prints {}", t.name));
        assert_eq!(g, w, "{label}: {} on the GPU is the evaluator's", t.name);
    }
    got
}

fn tensor(name: &str, cols: usize, vals: &[i64]) -> String {
    let mut s = format!("Let {name} be a 1 by {cols} tensor of Int.\n");
    for (i, v) in vals.iter().enumerate() {
        if *v != 0 {
            s.push_str(&format!("Set item {} of {name} to {v}.\n", i + 1));
        }
    }
    s
}

fn spread(n: usize, lo: i64, hi: i64, seed: u64) -> Vec<i64> {
    (0..n as u64).map(|i| lo + ((i.wrapping_add(seed).wrapping_mul(2_654_435_761) >> 7) % (hi - lo + 1) as u64) as i64).collect()
}

#[test]
fn each_integer_op_runs_on_the_gpu_as_the_evaluator_runs_it() {
    if !common::require_tool("nvcc") || !common::require_gpu() {
        return;
    }
    let x = tensor("X", 8, &spread(8, -258_064, 258_064, 3));
    let a = tensor("A", 8, &spread(8, -100, 50, 4));
    let b = tensor("B", 8, &spread(8, -7, 90, 5));
    let programs: Vec<(&str, String)> = vec![
        ("requant", format!("## Main\n{x}Let Y be X times 104593 shifted right by 24.\n")),
        ("clamp", format!("## Main\n{x}Let Y be X clamped between -128 and 127.\n")),
        ("relu", format!("## Main\n{a}Let Y be relu of A.\n")),
        ("sum", format!("## Main\n{a}{b}Let C be A plus B.\n")),
        ("hadamard", format!("## Main\n{a}{b}Let C be A hadamard B.\n")),
        ("argmax", format!("## Main\n{a}Let C be argmax of A.\n")),
        (
            "matmul",
            "## Main\nLet A be a 1 by 3 tensor of Int.\nSet item 1 of A to -128.\nSet item 2 of A to 127.\nSet item 3 of A to 9.\n\
             Let W be a 3 by 2 tensor of Int.\nSet item 1 of W to 100.\nSet item 2 of W to -5.\nSet item 5 of W to 77.\nLet C be A matmul W.\n"
                .to_string(),
        ),
        (
            "row gather",
            "## Main\nLet T be a 3 by 2 tensor of Int.\nSet item 1 of T to -9.\nSet item 6 of T to 40.\nLet at be 3.\nLet r be row at of T.\n".to_string(),
        ),
        (
            "row write",
            "## Main\nLet K be a 4 by 2 tensor of Int.\nLet x be a 1 by 2 tensor of Int.\nSet item 1 of x to 300.\nSet item 2 of x to -7.\n\
             Let y be x clamped between -128 and 127.\nLet at be 2.\nSet row at of K to y.\nLet r be row at of K.\n"
                .to_string(),
        ),
        (
            "lut attention",
            format!(
                "## Main\n{}Let K be a 3 by 2 tensor of Int.\nSet item 1 of K to 3.\nSet item 4 of K to -100.\n\
                 Let V be a 3 by 2 tensor of Int.\nSet item 1 of V to -9.\nSet item 4 of V to 11.\nSet item 6 of V to -128.\n{}Let at be 2.\n\
                 Let C be lut attention of Q over K and V with 1 heads 1 groups up to at with table E and scale 21076 shifted right by 6.\n",
                tensor("Q", 2, &[100, -2]),
                tensor("E", 2, &[32768, 1]),
            ),
        ),
    ];
    for (label, src) in &programs {
        gpu_equals_evaluator(label, &scheduled(src));
    }
}

/// An int8 GEMM's fused epilogue — the accumulator requantized, a bias row added down every tile, then a ReLU — runs on
/// the GPU as the evaluator runs it: the block the systolic lane lowers for a fabric of single-element tiles and for the
/// SN50's 32×32 tiles (where the bias row is broadcast down each tile's rows), printed as the general program,
/// activations at int8's two ends and weights of both signs, so each epilogue sees negative and positive values and the
/// ReLU clips some outputs and passes others.
#[test]
fn a_fused_requant_bias_relu_epilogue_runs_on_the_gpu_as_the_evaluator_runs_it() {
    if !common::require_tool("nvcc") || !common::require_gpu() {
        return;
    }
    let (k, n, m) = (4usize, 4usize, 6usize);
    let weights: Vec<i64> = (0..k * n).map(|i| if (i % n) % 2 == 0 { 100 } else { -100 }).collect();
    let fb = FusedBlock { sys: Systolic::new(k, n, weights), requant_m: 181, requant_s: 8, bias: vec![-400, 0, 7, 400] };
    let acts: Vec<i64> = (0..m * k).map(|i| if (i / k) % 2 == 0 { -128 } else { 127 }).collect();
    for target in [SpatialTarget::sky_fabric(16, 64), SpatialTarget::sn50_rdu()] {
        let mut p = run_passes(from_fused_block(&fb, m, &target).expect("the fused block"), &target).expect("the fused block schedules");
        for t in p.tensors.iter_mut().filter(|t| t.role == TensorRole::Input) {
            t.init = TensorInit::Values(acts.clone());
        }
        let kinds: Vec<&str> = p
            .stages
            .iter()
            .filter_map(|s| match &s.kind {
                StageKind::Compute(ComputeOp::Requant { .. }) => Some("requant"),
                StageKind::Compute(ComputeOp::Bias { .. }) => Some("bias"),
                StageKind::Compute(ComputeOp::Relu) => Some("relu"),
                _ => None,
            })
            .collect();
        assert_eq!(kinds, ["requant", "bias", "relu"], "{}: the block carries the three epilogue ops", target.name);
        let got = gpu_equals_evaluator_on(&format!("fused epilogue on {}", target.name), &p, &SpatialTarget::rtx3060());
        let out = p.output_tensor().expect("an output");
        let y = &got[&out.id.idx()];
        assert!(y.iter().any(|v| *v == 0) && y.iter().any(|v| *v > 0), "{}: the ReLU clips some outputs and passes others: {y:?}", target.name);
    }
}

/// A fold of a row is split over up to a warp's lanes, each folding its share in order, the lanes then combined: the
/// result is the sequential fold's. An arg-max's tie goes to the lower index — here the maxima at 20 and 33 of a
/// 40-element row land in lanes 20 and 1, so lane order alone would answer 33.
#[test]
fn a_fold_split_over_lanes_is_the_sequential_fold() {
    if !common::require_tool("nvcc") || !common::require_gpu() {
        return;
    }
    let with = |n: usize, at: &[(usize, i64)]| -> Vec<i64> {
        let mut v = spread(n, -60, 50, 9);
        for (i, x) in at {
            v[*i] = *x;
        }
        v
    };
    let programs: Vec<(&str, String)> = vec![
        ("argmax, ties in lanes 20 and 1", format!("## Main\n{}Let C be argmax of A.\n", tensor("A", 40, &with(40, &[(20, 77), (33, 77)])))),
        ("argmax, ties in one lane", format!("## Main\n{}Let C be argmax of A.\n", tensor("A", 40, &with(40, &[(3, 77), (35, 77)])))),
        ("argmax, all equal", format!("## Main\n{}Let C be argmax of A.\n", tensor("A", 40, &[5; 40]))),
        ("argmax, the last element", format!("## Main\n{}Let C be argmax of A.\n", tensor("A", 40, &with(40, &[(39, 99)])))),
        ("argmax, three elements", format!("## Main\n{}Let C be argmax of A.\n", tensor("A", 3, &[4, 9, 9]))),
        (
            "lut attention over five positions",
            format!(
                "## Main\n{}Let K be a 5 by 2 tensor of Int.\nSet item 1 of K to 3.\nSet item 4 of K to -100.\nSet item 7 of K to 60.\nSet item 10 of K to 2.\n\
                 Let V be a 5 by 2 tensor of Int.\nSet item 1 of V to -9.\nSet item 4 of V to 11.\nSet item 6 of V to -128.\nSet item 9 of V to 127.\n{}Let at be 4.\n\
                 Let C be lut attention of Q over K and V with 1 heads 1 groups up to at with table E and scale 21076 shifted right by 6.\n",
                tensor("Q", 2, &[100, -2]),
                tensor("E", 2, &[32768, 9000]),
            ),
        ),
    ];
    for target in [SpatialTarget::rtx3060(), rtx3060_scalar()] {
        for (label, src) in &programs {
            gpu_equals_evaluator_on(&format!("{label} on {}", target.name), &scheduled_on(src, &target), &target);
        }
    }
}

#[test]
fn the_decode_step_runs_on_the_gpu_at_every_position() {
    if !common::require_tool("nvcc") || !common::require_gpu() {
        return;
    }
    let (m, prompt, golden) = model();
    let (mut kc, mut vc) = (Vec::new(), Vec::new());
    let mut tok = prompt[0];
    let mut generated = Vec::new();
    for pos in 0..prompt.len() + golden.len() - 1 {
        let p = scheduled(&m.decode_step_lg(tok, pos, &kc, &vc));
        let got = gpu_equals_evaluator(&format!("position {pos}"), &p);
        let read = |name: &str| got[&p.tensors.iter().find(|t| t.name == name).unwrap().id.idx()].clone();
        kc = read("Kc")[..(pos + 1) * m.d].to_vec();
        vc = read("Vc")[..(pos + 1) * m.d].to_vec();
        let next = read("next")[0] as usize - 1;
        if pos + 1 >= prompt.len() {
            generated.push(next);
        }
        tok = if pos + 1 < prompt.len() { prompt[pos + 1] } else { next };
    }
    assert_eq!(generated, golden, "the GPU generates the trainer's tokens");
}

#[test]
fn the_decode_step_planned_for_single_element_tiles_runs_on_the_gpu() {
    if !common::require_tool("nvcc") || !common::require_gpu() {
        return;
    }
    let (m, prompt, _) = model();
    let target = rtx3060_scalar();
    gpu_equals_evaluator_on("single-element tiles", &scheduled_on(&m.decode_step_lg(prompt[0], 0, &[], &[]), &target), &target);
}

/// Every `#define PAGE_<ring> <elements>` a GPU program prints.
fn printed_pages(cu: &str) -> BTreeMap<usize, usize> {
    cu.lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("#define PAGE_")?;
            let (k, n) = rest.split_once(' ')?;
            Some((k.parse().ok()?, n.trim().parse().ok()?))
        })
        .collect()
}

#[test]
fn every_ring_holds_the_pages_its_ops_fill() {
    let (m, prompt, _) = model();
    let src = m.decode_step_lg(prompt[0], 0, &[], &[]);
    let mut overfilled = 0;
    for target in [SpatialTarget::rtx3060(), rtx3060_scalar()] {
        let p = scheduled_on(&src, &target);
        let extent = p.page_extents();
        let te = p.tile.elems();
        // The tiles each op moves through one page of a buffer: a pack's, a tile walk's, a reload's — the region its
        // program last acquired.
        let mut moved = vec![0usize; p.buffers.len()];
        for program in &p.schedule.as_ref().expect("a scheduled program").programs {
            let mut acquired = 0;
            walk_tagged_ops(&program.ops, 1, &mut |op, _| match op {
                ChoreoOp::AcquireAcc { tiles } => acquired = *tiles,
                ChoreoOp::PackAcc { to, tiles, .. } => moved[to.idx()] = moved[to.idx()].max(*tiles),
                ChoreoOp::ReadTiles { into, block_h, block_w, .. } => moved[into.idx()] = moved[into.idx()].max(block_h * block_w),
                ChoreoOp::WriteTiles { from, block_h, block_w, .. } => moved[from.idx()] = moved[from.idx()].max(block_h * block_w),
                ChoreoOp::Compute { acc_reload: Some(r), .. } => moved[r.idx()] = moved[r.idx()].max(acquired),
                _ => {}
            });
        }
        for b in &p.buffers {
            let declared = match b.kind {
                BufferKind::Fifo { page_tiles, .. } | BufferKind::Interm { page_tiles, .. } => page_tiles,
                BufferKind::Accumulator { .. } => continue,
            };
            assert_eq!(extent[b.id.idx()], declared.max(moved[b.id.idx()]), "{}@{}: a page is its declared page or the most an op moves through it", b.name, target.name);
            overfilled += usize::from(moved[b.id.idx()] > declared);
        }
        let art = lower(&p, &target);
        let mut programs = vec![("one block", art.program_cu.clone())];
        programs.extend(art.general.as_ref().map(|g| ("one block per site", g.program_cu.clone())));
        for (label, cu) in programs {
            let pages = printed_pages(&cu);
            for b in p.buffers.iter().filter(|b| !matches!(b.kind, BufferKind::Accumulator { .. })) {
                let ring = b.origin.unwrap_or(b.id).idx();
                let Some(page) = pages.get(&ring) else { continue };
                assert!(
                    *page >= extent[b.id.idx()] * te,
                    "{label}@{}: ring {ring}'s page of {page} elements holds {}'s pages of {} tiles",
                    target.name,
                    b.name,
                    extent[b.id.idx()]
                );
            }
        }
    }
    assert!(overfilled > 0, "single-element tiles pack a block into a lane scratch, the page this lock is about");
}
