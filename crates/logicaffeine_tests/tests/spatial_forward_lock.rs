//! spatial_forward_lock — a result only the next statement reads is handed to that statement's compute, never stored
//! and loaded back.
//!
//! A batch-1 decode step is one chain of dependent sections, and each boundary costs a store, a load, a rendezvous
//! and the next section's own setup. `passes::forward` joins two statements' section templates into one where the
//! tensor between them is read by nothing else in the program, and the store and the load agree; `fuse_generic` then
//! folds a map into the map it follows. This lock holds:
//! - who decides: forwarding needs the program's results declared (`compile_program_to_spatial_returning`); a program
//!   whose caller declares nothing stores every tensor a statement names, and computes what its forwarded self does;
//! - the rule: a tensor only the next statement reads is gone from the composed program, and its two sections are one;
//!   a tensor anything else reads — a `Show`, a later statement, the carried state, the caller, a second section of
//!   its reader — is stored as before; a store and a load that read different tiles (a matmul's operand blocks) are
//!   never joined; and no section of any program it composes loads a statement's result that nothing stores;
//! - what a join keeps: every layout bridge of a run of joined sections, and every interval a value is proven in — a
//!   join whose carrier would hold more than a load of the tensor read is not made;
//! - a matmul's epilogue joins it once `tile` has planned both — its template walks a work item per output tile, the
//!   epilogue one for the row — and only under a plan of one item packed once: a product two items wide keeps its
//!   epilogue apart;
//! - the whole-program reads it rests on (`Touches::read_only_by`);
//! - the meaning: every joined program evaluates to the VM's answer, and runs to the evaluator's answer in RTL on the
//!   fabric in its twin's cycles and on the GPU;
//! - the chips: each chip of the split step forwards on its own, moves the same transfers, holds the composed program's
//!   values and runs to the evaluator's answer in RTL;
//! - the point: on Board A's fabric the decode step loses every boundary between its row maps, and its step shortens.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_forward_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/cuda_run.rs"]
mod cuda_run;
#[path = "common/fabric_rtl.rs"]
mod fabric_rtl;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_decode_batch_lg, Qwen38Cfg};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, eval_choreography_every_tensor, Inputs, Value};
use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::{Buffer, BufferId, ComputeOp, NumericsPolicy, SpatialProgram, Stage, StageId, StageKind, TensorId, TensorRole};
use logicaffeine_compile::spatial::lower::cuda::CudaPersistentBackend;
use logicaffeine_compile::spatial::lower::fabric::{fabric_cycle_model, FabricBackend};
use logicaffeine_compile::spatial::link::{link, Form, Prelude};
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::passes::forward::{forward, one_block};
use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
use logicaffeine_compile::spatial::pipeline::{
    compile_program_to_chips, compile_program_to_chips_returning, compile_program_to_spatial, compile_program_to_spatial_returning,
    compile_program_to_spatial_tp, run_passes_through_tile,
};
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::spatial::value_range::program_ranges;
use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
use spatial_support::inputs_from_fills;

fn board_a() -> SpatialTarget {
    let mut t = SpatialTarget::xc7a200t_fabric(16, 64);
    t.site.vector_lanes = 8;
    t
}

/// `src` lowered for `target` with its results declared: nothing outside it reads past what it shows and the state it
/// carries.
fn composed(src: &str, target: &SpatialTarget) -> SpatialProgram {
    composed_returning(src, target, &[])
}

/// `src` lowered for `target`, its caller reading `reads` besides what it shows and the state it carries.
fn composed_returning(src: &str, target: &SpatialTarget, reads: &[&str]) -> SpatialProgram {
    let reads: std::collections::BTreeSet<String> = reads.iter().map(|r| r.to_string()).collect();
    let l = compile_program_to_spatial_returning(src, target, &reads).unwrap_or_else(|e| panic!("a tensor program: {e:?}\n{src}"));
    let p = l.whole().unwrap_or_else(|| panic!("lowers whole on {}: {}\n{src}", target.name, l.refusal())).clone();
    every_load_reads_a_written_value(src, &p, target);
    p
}

/// Every tensor a section of `p` loads is one some section stores, or no statement's result at all — a table the
/// source declares, or one the compiler makes. A statement's result loaded where nothing stores it is a value no
/// section computes: whatever the memory held.
fn every_load_reads_a_written_value(src: &str, p: &SpatialProgram, target: &SpatialTarget) {
    let results: std::collections::BTreeSet<String> =
        recognize_stages(src).expect("the program is recognized").stages.iter().map(|s| s.out.clone()).collect();
    let stored: std::collections::BTreeSet<TensorId> = p
        .stages
        .iter()
        .filter_map(|s| match &s.kind {
            StageKind::Store { tensor, .. } => Some(*tensor),
            _ => None,
        })
        .collect();
    for s in &p.stages {
        if let StageKind::Load { tensor, .. } = &s.kind {
            let t = p.tensor(*tensor);
            assert!(
                stored.contains(tensor) || !results.contains(&t.name),
                "{}: stage {} loads {}, a statement's result no section stores",
                target.name,
                s.name,
                t.name
            );
        }
    }
}

fn section_names(p: &SpatialProgram) -> Vec<String> {
    p.sections.iter().map(|s| s.name.clone()).collect()
}

fn has_tensor(p: &SpatialProgram, name: &str) -> bool {
    p.tensors.iter().any(|t| t.name == name)
}

/// The program's `Show`n values as the VM prints them, and as the evaluator's run of `p` gives them — every tensor's, a
/// stored intermediate as much as a result.
fn vm_and_eval(src: &str, p: &SpatialProgram, shows: &[(&str, usize)]) -> (Vec<i64>, Vec<i64>) {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "the VM runs it: {:?}", vm.error);
    let want: Vec<i64> = vm.output.split_whitespace().map(|v| v.parse().expect("an integer")).collect();
    let out = eval_choreography_every_tensor(p, &inputs_from_fills(p)).expect("the evaluator runs it");
    let got = shows
        .iter()
        .map(|(name, i)| {
            let t = p.tensors.iter().find(|t| t.name == *name).unwrap_or_else(|| panic!("the program keeps {name}"));
            match &out.tensors[&t.id] {
                Value::Int(v) => v[*i],
                Value::Float(v) => panic!("{name} through floats: {v:?}"),
            }
        })
        .collect();
    (want, got)
}

const A: &str = "Let A be a 1 by 16 tensor of Int.\nSet item 1 of A to 400.\nSet item 2 of A to -300.\nSet item 5 of A to 77.\nSet item 9 of A to -5000.\nSet item 16 of A to 1234.\n";
const B: &str = "Let B be a 1 by 16 tensor of Int.\nSet item 1 of B to -7.\nSet item 3 of B to 90.\nSet item 16 of B to -128.\n";

/// requant → clamp → relu → plus B: four statements, each result read only by the next.
fn chain(show_r: bool) -> String {
    let mut s = format!("## Main\n{A}{B}Let R be A times 104593 shifted right by 16.\nLet Q be R clamped between -128 and 127.\nLet P be relu of Q.\nLet C be P plus B.\n");
    for i in 1..=16 {
        s += &format!("Show item {i} of C.\n");
    }
    if show_r {
        s += "Show item 1 of R.\n";
    }
    s
}

#[test]
fn the_whole_program_names_what_reads_each_value() {
    let lifted = TensorGraph::lift_source(&chain(false), &|_| Storage::Values).expect("the chain is recognized");
    let g = &lifted.graph;
    let touches = g.touches();
    let at = |name: &str| g.ops.iter().position(|o| o.out == name).expect("its op");
    let (r, q, p, c) = (at("R"), at("Q"), at("P"), at("C"));
    assert!(touches.read_only_by("R", r, q), "R is read by Q alone");
    assert!(touches.read_only_by("Q", q, p) && touches.read_only_by("P", p, c), "each link of the chain is read by the next alone");
    assert!(!touches.read_only_by("C", c, usize::MAX), "C is shown");
    assert!(!touches.read_only_by("R", r, p), "R's reader is Q, not P");
    let shown = TensorGraph::lift_source(&chain(true), &|_| Storage::Values).expect("the chain is recognized");
    assert!(!shown.graph.touches().read_only_by("R", at("R"), at("Q")), "a Show reads R");
    assert!(shown.program.opaque.is_empty(), "the chain runs no foreign code");
    // A value a later op writes again is not its reader's alone: the second writer touches it, reading it or not.
    let rebound = format!("## Main\n{A}{B}Let R be A plus B.\nLet Q be R plus B.\nLet R be A plus A.\nShow item 1 of Q.\n");
    let rebound = TensorGraph::lift_source(&rebound, &|_| Storage::Values).expect("the rebinding walks").graph;
    assert_eq!(rebound.ops.iter().filter(|o| o.out == "R").count(), 2, "both writes of R are ops");
    assert!(!rebound.touches().read_only_by("R", 0, 1), "R is written again after Q reads it");
}

#[test]
fn a_result_only_the_next_statement_reads_never_leaves_the_site() {
    for target in [board_a(), SpatialTarget::sky_fabric(16, 64), SpatialTarget::rtx3060()] {
        let src = chain(false);
        let p = composed(&src, &target);
        for gone in ["R", "Q", "P"] {
            assert!(!has_tensor(&p, gone), "{}: {gone} is forwarded, never stored", target.name);
        }
        assert_eq!(p.sections.len(), 1, "{}: the four statements are one section: {:?}", target.name, section_names(&p));
        // On a fabric's one-element tile every map walks the row; on the GPU's 16×16 tile the add walks its tile's 16
        // rows where the maps walk the tensor's one, and spaces that differ do not fold.
        let compute = p.stages.iter().filter(|s| matches!(s.kind, logicaffeine_compile::spatial::ir::StageKind::Compute(_))).count();
        let bodies = if target.tile.rows == 1 { 1 } else { 2 };
        assert_eq!(compute, bodies, "{}: the four maps fold into {bodies} body(ies)", target.name);
        let shows: Vec<(&str, usize)> = (0..16).map(|i| ("C", i)).collect();
        let (want, got) = vm_and_eval(&src, &p, &shows);
        assert_eq!(got, want, "{}: the joined program is the VM's", target.name);
    }
}

/// A region of pointwise ops whose arms nothing outside it reads — two arms off one pair of operands, joined
/// (PARITY-ELEMENTWISE-REGIONS, the spatial chips' half) — is one section whose arms never leave the site, and one
/// compute body where the chip's tile walks the row as the maps do, evaluating to the VM's values.
#[test]
fn a_pointwise_region_is_one_section_and_one_body() {
    let mut src = format!("## Main\n{A}{B}Let S be A plus B.\nLet P be A hadamard B.\nLet Y be S plus P.\n");
    for i in 1..=16 {
        src += &format!("Show item {i} of Y.\n");
    }
    for target in [board_a(), SpatialTarget::sky_fabric(16, 64), SpatialTarget::rtx3060()] {
        let p = composed(&src, &target);
        for gone in ["S", "P"] {
            assert!(!has_tensor(&p, gone), "{}: the arm {gone} is never stored", target.name);
        }
        assert_eq!(p.sections.len(), 1, "{}: the region is one section: {:?}", target.name, section_names(&p));
        if target.tile.rows == 1 {
            let compute = p.stages.iter().filter(|s| matches!(s.kind, StageKind::Compute(_))).count();
            assert_eq!(compute, 1, "{}: the region's maps fold into one body", target.name);
        }
        let shows: Vec<(&str, usize)> = (0..16).map(|i| ("Y", i)).collect();
        let (want, got) = vm_and_eval(&src, &p, &shows);
        assert_eq!(got, want, "{}: the region is the VM's", target.name);
    }
}

/// The requant's and the clamp's section templates, as the pipeline makes them from the chain.
fn chain_templates(target: &SpatialTarget) -> (SpatialProgram, SpatialProgram) {
    let g = TensorGraph::lift_source(&chain(false), &|_| Storage::Values).expect("the chain is recognized").graph;
    let template = |name: &str| {
        let k = g.ops.iter().position(|o| o.out == name).expect("its op");
        link(&g, k, &Prelude::default(), Form::Section { floats: false }, NumericsPolicy::Exact, target).expect("its templates").remove(0)
    };
    (template("R"), template("Q"))
}

/// `forward` joins two sections through a tensor only where no other stage of either names it: a second stage that
/// stores, loads or reduces the tensor would address one the join removed. Such a pair is refused, never mistranslated.
#[test]
fn forward_refuses_a_tensor_another_stage_of_either_section_names() {
    let target = board_a();
    let (r, q) = chain_templates(&target);
    assert!(forward(&r, &q, "R").is_some(), "the requant and the clamp join through R");
    let load = q.stages.iter().find(|s| matches!(s.kind, StageKind::Load { .. })).expect("the clamp loads R").clone();
    let StageKind::Load { tensor, pattern, num_blocks, multicast } = load.kind.clone() else { unreachable!() };
    // The clamp's section storing R again, from the buffer its load fills.
    let mut q2 = q.clone();
    q2.stages.push(Stage {
        id: StageId::new(q2.stages.len()),
        name: "store_R_again".into(),
        kind: StageKind::Store { tensor, pattern: pattern.clone(), sub_blocks: (1, 1) },
        ins: load.outs.clone(),
        outs: vec![],
        ..load.clone()
    });
    assert!(forward(&r, &q2, "R").is_none(), "a consumer that also stores R is not joined");
    // The requant's section loading R back into a buffer of its own.
    let mut r2 = r.clone();
    let rid = r2.tensors.iter().find(|t| t.name == "R").expect("the requant declares R").id;
    let back = BufferId::new(r2.buffers.len());
    r2.buffers.push(Buffer { id: back, name: "r_back".into(), ..q.buffer(load.outs[0]).clone() });
    r2.stages.push(Stage {
        id: StageId::new(r2.stages.len()),
        name: "load_R_back".into(),
        kind: StageKind::Load { tensor: rid, pattern, num_blocks, multicast },
        ins: vec![],
        outs: vec![back],
        ..load.clone()
    });
    assert!(forward(&r2, &q, "R").is_none(), "a producer that also loads R is not joined");
}

/// `forward` joins two sections only where the producer's store and the consumer's load address the same tiles in the
/// same order for every item: a load that walks the tensor at another stride reads tiles the packed page does not hold.
#[test]
fn forward_refuses_a_load_of_other_tiles() {
    let target = board_a();
    let (r, q) = chain_templates(&target);
    assert!(forward(&r, &q, "R").is_some(), "the requant and the clamp join through R");
    let mut q2 = q.clone();
    let load = q2.stages.iter_mut().find(|s| matches!(s.kind, StageKind::Load { .. })).expect("the clamp loads R");
    let StageKind::Load { pattern, .. } = &mut load.kind else { unreachable!() };
    pattern.stride_w += 1;
    assert!(forward(&r, &q2, "R").is_none(), "a load of R at another stride is not joined");
}

/// A planned join holds only under a plan of one item whose every product packs once: a product planned over two
/// sub-blocks packs a page a sub-block, and the epilogue joined to it would read the last alone.
#[test]
fn forward_refuses_a_planned_join_a_plan_of_one_block_does_not_hold() {
    let target = board_a();
    let g = TensorGraph::lift_source(&epilogue(16), &|_| Storage::Values).expect("the product and its epilogue are recognized").graph;
    let planned = |name: &str| {
        let k = g.ops.iter().position(|o| o.out == name).expect("its op");
        let t = link(&g, k, &Prelude::default(), Form::Section { floats: false }, NumericsPolicy::Exact, &target).expect("its templates").remove(0);
        run_passes_through_tile(t, &target).expect("its plan")
    };
    let (y, r) = (planned("Y"), planned("R"));
    let joint = forward(&y, &r, "Y").expect("the planned product and its requant join through Y");
    assert!(one_block(&joint.program), "under a plan of one block");
    let mut y2 = y.clone();
    for s in &mut y2.stages {
        if let StageKind::Compute(ComputeOp::MatmulTiles { sub_blocks, .. }) = &mut s.kind {
            *sub_blocks = (1, 2);
        }
    }
    assert!(forward(&y2, &r, "Y").is_none(), "a product of two sub-blocks keeps its epilogue apart");
}

/// A float result only the next statement reads is forwarded as an integer one is: no interval is proven for it, and
/// none is needed — its join is the program apart, on the GPU exactly and on the SN50 within its bf16.
#[test]
fn a_float_result_only_the_next_statement_reads_is_forwarded() {
    let fill = |t: &str| -> String {
        let mut s = format!("Let {t} be a 1 by 16 tensor of Real.\n");
        for i in 1..=16 {
            s += &format!("Set item {i} of {t} to {}.\n", ((i as i64 * 37) % 200 - 100) as f64 / 64.0);
        }
        s
    };
    let src = format!("## Main\n{}{}Let S be A plus B.\nLet T be silu of S.\n{}", fill("A"), fill("B"), (1..=16).map(|i| format!("Show item {i} of T.\n")).collect::<String>());
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "the VM runs it: {:?}", vm.error);
    let want: Vec<f64> = vm.output.split_whitespace().map(|v| v.parse().expect("a number")).collect();
    for (target, rel) in [(SpatialTarget::rtx3060(), 1e-6), (SpatialTarget::sn50_rdu(), 1e-2)] {
        let p = composed(&src, &target);
        assert_eq!(section_names(&p), ["elementwise_+_16+silu_16"], "{}: the sum is forwarded into the silu", target.name);
        let out = eval_choreography_every_tensor(&p, &inputs_from_fills(&p)).expect("the evaluator runs it");
        let Value::Float(got) = &out.tensors[&p.tensors.iter().find(|t| t.name == "T").expect("T").id] else { panic!("T through integers") };
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert!((g - w).abs() <= rel * w.abs().max(1.0), "{}: T[{i}] {g} joined, {w} on the VM", target.name);
        }
    }
}

/// A join keeps what the round trip it removes kept. Across a page narrower than f32 — bf16 pages — the gate
/// and the product stay two computes inside the one joined section, the page between them rounding the gate as the
/// store did, and the joined program computes what the program with every result stored computes, bit for bit; across
/// an f32 page the product folds the gate in, within the chip's declared bound of its program apart. A chip that does
/// not lower the program is passed over.
#[test]
fn a_joined_section_rounds_what_its_store_rounded() {
    use logicaffeine_compile::codegen_sva::fabric_target::Precision;
    use logicaffeine_compile::spatial::target::ComputeCap;
    use logicaffeine_compile::spatial::shim::SpatialShim;
    use logicaffeine_compile::tensor_graph::shim::Shim;
    let fill = |t: &str, stride: i64| -> String {
        let mut s = format!("Let {t} be a 2 by 8 tensor of Real.\n");
        for i in 1..=16 {
            s += &format!("Set item {i} of {t} to {}.\n", ((i as i64 * stride) % 199 - 99) as f64 / 67.0);
        }
        s
    };
    let src = format!("## Main\n{}{}Let S be sigmoid of G.\nLet Y be T hadamard S.\n{}", fill("G", 37), fill("T", 53), (1..=16).map(|i| format!("Show item {i} of Y.\n")).collect::<String>());
    let (mut narrow, mut wide) = (0, 0);
    // The SN50's SIMD stages take fp32 beside bf16; with their bf16 alone its pages are narrower than its sums.
    let bf16_pages = {
        let mut t = SpatialTarget::sn50_rdu();
        for c in &mut t.compute {
            if let ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } = c {
                precisions.remove(&Precision::Fp32);
            }
        }
        t.precisions.remove(&Precision::Fp32);
        t
    };
    for target in [SpatialTarget::wormhole(), SpatialTarget::sky_fabric(64, 64), SpatialTarget::sn50_rdu(), SpatialTarget::rtx3060(), bf16_pages] {
        let stored = compile_program_to_spatial(&src, &target).unwrap_or_else(|e| panic!("{}: {e:?}", target.name));
        let joined = compile_program_to_spatial_returning(&src, &target, &std::collections::BTreeSet::new()).unwrap_or_else(|e| panic!("{}: {e:?}", target.name));
        let (Some(stored), Some(joined)) = (stored.whole(), joined.whole()) else { continue };
        assert!(joined.sections.len() < stored.sections.len(), "{}: the gate is forwarded into the product: {:?}", target.name, section_names(joined));
        let value = |p: &SpatialProgram| -> Vec<f64> {
            let out = eval_choreography_every_tensor(p, &inputs_from_fills(p)).unwrap_or_else(|e| panic!("{}: the evaluator: {e:?}", target.name));
            let Value::Float(v) = &out.tensors[&p.tensors.iter().find(|t| t.name == "Y").expect("Y").id] else { panic!("Y through integers") };
            v.clone()
        };
        let (apart, together) = (value(stored), value(joined));
        let computes = joined.stages.iter().filter(|s| matches!(s.kind, StageKind::Compute(ComputeOp::Generic { .. }))).count();
        if joined.buffers.iter().any(|b| matches!(b.dtype, Precision::Bf16 | Precision::Fp16)) {
            narrow += 1;
            assert_eq!(computes, 2, "{}: across a narrower page the gate and the product stay two computes", target.name);
            let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<u64>>();
            assert_eq!(bits(&together), bits(&apart), "{}: the joined product, bit for bit, the stored one", target.name);
        } else {
            wide += 1;
            assert_eq!(computes, 1, "{}: across an f32 page the product folds the gate in", target.name);
            let bound = SpatialShim { target: &target }.numerics().expect("a spatial chip declares its contract").fused;
            let max = apart.iter().fold(0.0f64, |m, v| m.max(v.abs()));
            for (i, (a, j)) in apart.iter().zip(&together).enumerate() {
                assert!(bound.holds(*a, *j, max), "{}: Y[{i}] {j:e} joined, {a:e} stored, past {bound:?}", target.name);
            }
        }
    }
    assert!(narrow > 0 && wide > 0, "a chip of each page width lowers the program: {narrow} narrower, {wide} f32");
}

/// A rotation reads each element's partner across the half of its head, three reads of its input a point: joined to
/// the norm before it, it still reads every element as the norm wrote it. The joined program computes what the program
/// with the norm stored computes, bit for bit, and the VM's answer.
#[test]
fn a_joined_norm_and_rotation_reads_every_element_the_norm_wrote() {
    let fill = |t: &str, n: usize| -> String {
        let mut s = format!("Let {t} be a 1 by {n} tensor of Real.\n");
        for i in 1..=n {
            s += &format!("Set item {i} of {t} to {}.\n", ((i as i64 * 37 + t.len() as i64 * 11) % 200 - 100) as f64 / 64.0);
        }
        s
    };
    let src = format!(
        "## Main\n{}{}{}{}Let M be rmsnorm of Q with weight N across 2 heads.\nLet P be rope of M with cos RC and sin RS.\n{}",
        fill("Q", 16),
        fill("N", 8),
        fill("RC", 4),
        fill("RS", 4),
        (1..=16).map(|i| format!("Show item {i} of P.\n")).collect::<String>()
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "the VM runs it: {:?}", vm.error);
    let want: Vec<f64> = vm.output.split_whitespace().map(|v| v.parse().expect("a number")).collect();
    let p_of = |p: &SpatialProgram| -> Vec<f64> {
        let out = eval_choreography_every_tensor(p, &inputs_from_fills(p)).expect("the evaluator runs it");
        let Value::Float(v) = &out.tensors[&p.tensors.iter().find(|t| t.name == "P").expect("P").id] else { panic!("P through integers") };
        v.clone()
    };
    for target in [SpatialTarget::wormhole(), SpatialTarget::rtx3060()] {
        let l = compile_program_to_spatial(&src, &target).expect("a tensor program");
        let stored = l.whole().unwrap_or_else(|| panic!("{}: lowers whole: {}", target.name, l.refusal())).clone();
        let joined = composed(&src, &target);
        assert!(joined.sections.len() < stored.sections.len(), "{}: the norm joins the rotation: {:?}", target.name, section_names(&joined));
        let (apart, together) = (p_of(&stored), p_of(&joined));
        assert_eq!(apart.len(), want.len(), "{}: every element of P", target.name);
        for (i, ((a, j), w)) in apart.iter().zip(&together).zip(&want).enumerate() {
            assert_eq!(a.to_bits(), j.to_bits(), "{}: P[{i}] {j} joined, {a} with the norm stored", target.name);
            assert!((a - w).abs() <= 1e-6 * w.abs().max(1.0), "{}: P[{i}] {a}, {w} on the VM", target.name);
        }
    }
}

#[test]
fn a_result_anything_else_reads_is_stored() {
    let src = chain(true);
    let p = composed(&src, &board_a());
    assert!(has_tensor(&p, "R"), "R is shown, so it is stored");
    assert!(!has_tensor(&p, "Q") && !has_tensor(&p, "P"), "the rest of the chain still forwards");
    assert_eq!(section_names(&p).len(), 2, "R's section, then the rest: {:?}", section_names(&p));
    let mut shows: Vec<(&str, usize)> = (0..16).map(|i| ("C", i)).collect();
    shows.push(("R", 0));
    let (want, got) = vm_and_eval(&src, &p, &shows);
    assert_eq!(got, want, "the program is the VM's");
}

/// A program whose caller declares nothing of what it reads forwards nothing: a host may read any tensor it is handed,
/// and a program of a deployment has readers in the next program. Every tensor a statement names is stored, a section a
/// statement, and the program computes what the forwarded one does.
#[test]
fn a_program_whose_results_are_not_declared_forwards_nothing() {
    let src = chain(false);
    for target in [board_a(), SpatialTarget::sky_fabric(16, 64), SpatialTarget::rtx3060()] {
        let l = compile_program_to_spatial(&src, &target).expect("a tensor program");
        let plain = l.whole().unwrap_or_else(|| panic!("{}: lowers whole: {}", target.name, l.refusal())).clone();
        for kept in ["R", "Q", "P", "C"] {
            assert!(has_tensor(&plain, kept), "{}: {kept} is stored", target.name);
        }
        assert_eq!(plain.sections.len(), 4, "{}: a section a statement: {:?}", target.name, section_names(&plain));
        let joined = composed(&src, &target);
        let c = |p: &SpatialProgram| -> Vec<i64> {
            let out = eval_choreography_every_tensor(p, &inputs_from_fills(p)).expect("the evaluator runs it");
            match &out.tensors[&p.tensors.iter().find(|t| t.name == "C").expect("C").id] {
                Value::Int(v) => v.clone(),
                Value::Float(v) => panic!("C through floats: {v:?}"),
            }
        };
        assert_eq!(c(&joined), c(&plain), "{}: forwarding changes no value", target.name);
    }
}

/// A result the caller reads is stored however its next statement reads it, and the rest of the chain still forwards.
#[test]
fn a_declared_result_is_stored_however_its_next_statement_reads_it() {
    let src = chain(false);
    let p = composed_returning(&src, &board_a(), &["R"]);
    assert!(has_tensor(&p, "R"), "the caller reads R: it is stored");
    assert!(!has_tensor(&p, "Q") && !has_tensor(&p, "P"), "the rest of the chain forwards");
    assert_eq!(section_names(&p).len(), 2, "R's section, then the rest: {:?}", section_names(&p));
    let shows: Vec<(&str, usize)> = (0..16).map(|i| ("C", i)).collect();
    let (want, got) = vm_and_eval(&src, &p, &shows);
    assert_eq!(got, want, "the program is the VM's");
}

#[test]
fn a_matmul_operand_block_is_never_joined_to_the_row_that_feeds_it() {
    let src = format!(
        "## Main\n{A}Let W be a 16 by 16 tensor of Int.\nSet item 1 of W to 3.\nSet item 18 of W to -2.\nSet item 256 of W to 1.\nLet R be A times 104593 shifted right by 16.\nLet Y be R matmul W.\nShow item 1 of Y.\nShow item 2 of Y.\nShow item 16 of Y.\n"
    );
    let p = composed(&src, &board_a());
    assert!(has_tensor(&p, "R"), "the matmul loads R in operand blocks, not the page the requant packs");
    assert_eq!(p.sections.len(), 2, "two sections: {:?}", section_names(&p));
    let (want, got) = vm_and_eval(&src, &p, &[("Y", 0), ("Y", 1), ("Y", 15)]);
    assert_eq!(got, want);
}

/// `A·W` requantized and clamped, every intermediate read only by the next statement.
fn epilogue(w_cols: usize) -> String {
    let mut s = format!("## Main\n{A}Let W be a 16 by {w_cols} tensor of Int.\nSet item 1 of W to 3.\nSet item 18 of W to -2.\nSet item {} of W to 1.\n", 16 * w_cols);
    s += "Let Y be A matmul W.\nLet R be Y times 104593 shifted right by 16.\nLet C be R clamped between -128 and 127.\n";
    for i in 1..=w_cols {
        s += &format!("Show item {i} of C.\n");
    }
    s
}

#[test]
fn a_matmul_s_epilogue_joins_it_under_a_plan_of_one_block() {
    let src = epilogue(16);
    let p = composed(&src, &board_a());
    assert_eq!(section_names(&p), ["matmul_1x16x16+int_requant_16+int_clamp_16"], "the product and its epilogue are one section");
    assert!(!has_tensor(&p, "Y") && !has_tensor(&p, "R"), "the product and its requant are forwarded");
    let shows: Vec<(&str, usize)> = (0..16).map(|i| ("C", i)).collect();
    let (want, got) = vm_and_eval(&src, &p, &shows);
    assert_eq!(got, want, "the joined product is the VM's");
}

#[test]
fn a_product_two_items_wide_keeps_its_epilogue_apart() {
    let src = epilogue(32);
    let target = board_a();
    let p = composed(&src, &target);
    let names = section_names(&p);
    assert_eq!(names.len(), 2, "the product alone, then its epilogue: {names:?}");
    assert!(names[0].starts_with("matmul_") && !names[0].contains('+'), "the product's section is its own: {names:?}");
    assert_eq!(names[1], "int_requant_32+int_clamp_32", "the epilogue's maps still fold together");
    let shows: Vec<(&str, usize)> = (0..32).map(|i| ("C", i)).collect();
    let (want, got) = vm_and_eval(&src, &p, &shows);
    assert_eq!(got, want);
}

/// `A·W` for `A` of `rows` rows, then requantized: every intermediate read only by the next statement.
fn product_epilogue(rows: usize, k: usize, n: usize) -> String {
    let fill = |t: &str, r: usize, c: usize| -> String {
        let mut s = format!("Let {t} be a {r} by {c} tensor of Int.\n");
        for i in 1..=r * c {
            s += &format!("Set item {i} of {t} to {}.\n", (i as i64 * 37) % 200 - 100);
        }
        s
    };
    let mut s = format!("## Main\n{}{}Let Y be A matmul W.\nLet R be Y times 104593 shifted right by 16.\n", fill("A", rows, k), fill("W", k, n));
    for i in 1..=rows * n {
        s += &format!("Show item {i} of R.\n");
    }
    s
}

/// A product of several rows and its epilogue are the VM's on every target, joined or apart. Their templates walk an
/// item a tile; `tile` plans the product a block of tiles an item, and an epilogue joined to it at its template would
/// still compute one tile an item: on the sky130 fabric `2×16 · 16×32` then a requant computed its first element alone.
/// Two templates join through a store that steps with its item only where each walks one item.
#[test]
fn a_product_s_epilogue_over_several_rows_is_the_vm_s() {
    for (rows, k, n) in [(2, 16, 32), (32, 32, 32)] {
        let src = product_epilogue(rows, k, n);
        for target in [board_a(), SpatialTarget::sky_fabric(16, 64), SpatialTarget::rtx3060(), SpatialTarget::sn50_rdu()] {
            let p = composed(&src, &target);
            let shows: Vec<(&str, usize)> = (0..rows * n).map(|i| ("R", i)).collect();
            let (want, got) = vm_and_eval(&src, &p, &shows);
            assert_eq!(got, want, "{}: {rows}×{k} · {k}×{n} then a requant is the VM's: {:?}", target.name, section_names(&p));
        }
    }
}

#[test]
fn a_joined_product_runs_on_the_fabric_as_the_evaluator_in_its_twin_s_cycles() {
    if !common::require_tool("iverilog") {
        return;
    }
    let target = board_a();
    let p = composed(&epilogue(16), &target);
    let art = match FabricBackend::new(target.clone()).lower(&p) {
        Ok(SpatialArtifact::Verilog(v)) => v,
        other => panic!("the fabric lowers the joined product: {other:?}"),
    };
    fabric_rtl::runs_as_the_evaluator("joined product", &p, &target, &art, &inputs_from_fills(&p));
}

#[test]
fn carried_state_is_stored_however_its_next_reader_reads_it() {
    let (m, prompt, _) = spatial_support_model();
    let p = composed(&m.decode_step_lg(prompt[0], 0, &[], &[]), &board_a());
    for state in ["Kc", "Vc"] {
        assert!(has_tensor(&p, state), "{state}, the cache, is stored");
    }
}

#[test]
fn a_joined_chain_runs_on_the_fabric_as_the_evaluator_in_its_twin_s_cycles() {
    if !common::require_tool("iverilog") {
        return;
    }
    for target in [board_a(), SpatialTarget::sky_fabric(16, 64)] {
        let p = composed(&chain(false), &target);
        let art = match FabricBackend::new(target.clone()).lower(&p) {
            Ok(SpatialArtifact::Verilog(v)) => v,
            other => panic!("{}: the fabric lowers the joined chain: {other:?}", target.name),
        };
        fabric_rtl::runs_as_the_evaluator(&format!("joined chain on {}", target.name), &p, &target, &art, &inputs_from_fills(&p));
    }
}

#[test]
fn a_joined_chain_runs_on_the_gpu_as_the_evaluator() {
    if !common::require_tool("nvcc") || !common::require_gpu() {
        return;
    }
    let target = SpatialTarget::rtx3060();
    let p = composed(&chain(false), &target);
    let art = match CudaPersistentBackend::new(target.clone()).lower(&p) {
        Ok(SpatialArtifact::Cuda(a)) => a,
        other => panic!("the GPU prints the joined chain: {other:?}"),
    };
    cuda_run::runs_as_the_evaluator("joined chain", &p, &art.program_cu, &inputs_from_fills(&p));
    if let Some(general) = &art.general {
        cuda_run::runs_as_the_evaluator("joined chain (one block per site)", &p, &general.program_cu, &inputs_from_fills(&p));
    }
}

/// On Board A's fabric the decode step keeps no boundary between two row maps: every section that maps a row is the
/// whole run of maps from the result before it to the next matmul, attention or table write. And it is shorter than
/// the step of a section per statement.
#[test]
fn the_decode_step_keeps_no_boundary_between_its_row_maps() {
    let (m, prompt, _) = spatial_support_model();
    let src = m.decode_step_lg(prompt[0], 0, &[], &[]);
    let target = board_a();
    let p = composed(&src, &target);
    let rp = recognize_stages(&src).expect("the step is recognized");
    assert!(p.sections.len() < rp.stages.len(), "fewer sections than statements: {} of {}", p.sections.len(), rp.stages.len());
    let row_map = |name: &str| ["int_requant_", "int_clamp_", "relu_", "elementwise_"].iter().any(|k| name.starts_with(k));
    let names = section_names(&p);
    for w in names.windows(2) {
        assert!(!(row_map(&w[0]) && row_map(&w[1])), "two row-map sections in a row: {} then {}", w[0], w[1]);
    }
    let twin = fabric_cycle_model(&p, &target);
    assert!(twin < 1507, "the step at 8 lanes is shorter than the 1,507 cycles of a section per statement: {twin}");
}

/// `X = A + A`, read only by a mixture of experts whose router and experts are sections of their own, each loading `X`.
fn moe_over_a_sum() -> (String, usize) {
    let (d, ne, dff, k) = (8usize, 4usize, 8usize, 2usize);
    let mut s = format!(
        "## Main\nLet A be a 1 by {d} tensor of Real.\nLet Wr be a {d} by {ne} tensor of Real.\nLet Wg be a {} by {d} tensor of Real.\nLet Wu be a {} by {d} tensor of Real.\nLet Wd be a {} by {dff} tensor of Real.\n",
        ne * dff,
        ne * dff,
        ne * d
    );
    let v = |i: usize, a: usize, m: usize, scale: f64, off: f64| ((i * a) % m) as f64 * scale - off;
    for i in 1..=d {
        s += &format!("Set item {i} of A to {}.\n", v(i, 1, 9, 0.25, 1.0));
    }
    for i in 1..=d * ne {
        s += &format!("Set item {i} of Wr to {}.\n", v(i, 7, 11, 0.1, 0.5));
    }
    for i in 1..=ne * dff * d {
        s += &format!(
            "Set item {i} of Wg to {}.\nSet item {i} of Wu to {}.\nSet item {i} of Wd to {}.\n",
            v(i, 5, 13, 0.05, 0.3),
            v(i, 3, 7, 0.1, 0.3),
            v(i, 11, 9, 0.05, 0.2)
        );
    }
    s += &format!("Let X be A plus A.\nLet H be moe of X with router Wr gate Wg up Wu down Wd choosing {k} of {ne}.\n");
    for i in 1..=d {
        s += &format!("Show item {i} of H.\n");
    }
    (s, d)
}

/// A result its reader loads in more than one section is stored: forwarding hands the value to one section's compute,
/// and the others would read a tensor nothing wrote.
#[test]
fn a_result_two_sections_of_its_reader_load_is_stored() {
    let (src, d) = moe_over_a_sum();
    let target = SpatialTarget::sn50_rdu();
    let p = composed(&src, &target);
    assert!(has_tensor(&p, "X"), "the router and the experts both load X: it is stored");
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "the VM runs it: {:?}", vm.error);
    let want: Vec<f64> = vm.output.split_whitespace().map(|v| v.parse().expect("a number")).collect();
    let out = eval_choreography_every_tensor(&p, &inputs_from_fills(&p)).expect("the evaluator runs it");
    let h = p.tensors.iter().find(|t| t.name == "H").expect("H is the program's result");
    let got = match &out.tensors[&h.id] {
        Value::Float(v) => v[..d].to_vec(),
        Value::Int(v) => panic!("H through integers: {v:?}"),
    };
    let scale = want.iter().fold(0.0f64, |m, x| m.max(x.abs())).max(f64::MIN_POSITIVE);
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert!((g - w).abs() <= 0.02 * scale, "H[{i}]: {g} on the SN50, {w} on the VM\n  ours: {got:?}\n  vm:   {want:?}");
    }
}

/// `(tok, pos)` per sequence of Qwen3.8 tiny's batched decode step.
const QWEN_SEQS: [(usize, usize); 3] = [(5, 3), (2, 1), (7, 6)];

/// Qwen3.8 tiny's batched decode step, every declared `Real` tensor filled by a hash of its name and index.
fn qwen_batch() -> String {
    let src = gen_qwen38_decode_batch_lg(&Qwen38Cfg::tiny(), "/w", 6, &QWEN_SEQS).0;
    let mut out = String::new();
    for line in src.lines() {
        out.push_str(line);
        out.push('\n');
        let Some((var, dims)) = line
            .strip_prefix("Let ")
            .and_then(|r| r.split_once(" be a "))
            .and_then(|(v, t)| t.strip_suffix(" tensor of Real.").map(|d| (v, d)))
        else {
            continue;
        };
        let (r, c) = dims.split_once(" by ").expect("R by C");
        let salt = var.split("_s").next().unwrap_or(var).bytes().map(|b| b as u64).sum::<u64>();
        for i in 0..r.parse::<usize>().unwrap() * c.parse::<usize>().unwrap() {
            let h = (i as u64).wrapping_mul(2654435761).wrapping_add(salt.wrapping_mul(40503));
            out.push_str(&format!("Set item {} of {var} to {:?}.\n", i + 1, ((h % 97) as f64 / 97.0 - 0.5) * 0.4));
        }
    }
    out
}

/// A join anywhere in a run of planned sections leaves each statement's layouts agreed: Qwen3.8's batched step stacks
/// its embeddings into `X`, a matrix the norm reads as one row, and the bridge between the two layouts stands however
/// many sections around it join.
#[test]
fn a_join_anywhere_in_a_run_of_sections_leaves_every_layout_agreed() {
    let src = qwen_batch();
    let target = RduChip::Sn50.facts().spatial_target();
    let p = composed_returning(&src, &target, &["next_s0", "next_s1", "next_s2"]);
    assert!(p.sections.iter().any(|s| s.name.contains('+')), "the step forwards: {:?}", section_names(&p));
    assert!(p.certificate.as_ref().is_some_and(|c| c.recheck(&p)), "the joined step certifies");
    let out = eval_choreography(&p, &inputs_from_fills(&p)).expect("the joined step evaluates");
    let mut shown = src.clone();
    for b in 0..QWEN_SEQS.len() {
        shown += &format!("Show next_s{b}.\n");
    }
    let vm = vm_outcome(&shown);
    assert!(vm.error.is_none(), "the VM runs it: {:?}", vm.error);
    let want: Vec<f64> = vm.output.split_whitespace().map(|v| v.parse().expect("a number")).collect();
    for (b, w) in want.iter().enumerate() {
        let t = p.tensors.iter().find(|t| t.name == format!("next_s{b}")).unwrap_or_else(|| panic!("next_s{b} is a result"));
        let got = match out.tensors.get(&t.id).expect("the token is written") {
            Value::Int(v) => v[0] as f64,
            Value::Float(v) => v[0],
        };
        assert_eq!(got, *w, "sequence {b}: the joined step emits the VM's token");
    }
}

/// A join never widens what a value is proven to hold. The front end proves an attention's context within its values'
/// range — a weighted mean of them — which no analysis of the division that computes it can see; joined to the
/// requant that reads it, the context would reach the product unbounded. The decode step's storage fits 32 bits and its
/// arithmetic 48, joined as it was stored.
#[test]
fn a_join_never_widens_what_a_value_is_proven_to_hold() {
    let (m, prompt, _) = spatial_support_model();
    let src = m.decode_step_lg(prompt[0], 0, &[], &[]);
    let fits = |iv: (i64, i64), bits: u32| iv.0 >= -(1i64 << (bits - 1)) && iv.1 < (1i64 << (bits - 1));
    for target in [SpatialTarget::sky_fabric(16, 64), board_a()] {
        let p = composed(&src, &target);
        assert!(p.sections.iter().any(|s| s.name.contains('+')), "{}: the step forwards", target.name);
        let proven = program_ranges(&p);
        for b in &p.buffers {
            let iv = proven.buffers[b.id.idx()].unwrap_or_else(|| panic!("{}: buffer {} has an interval", target.name, b.name));
            assert!(fits(iv, 32), "{}: buffer {} holds {iv:?}", target.name, b.name);
        }
        for (s, st) in p.stages.iter().zip(&proven.stages) {
            for (j, iv) in st.nodes.iter().enumerate() {
                assert!(fits(*iv, 48), "{}: stage {} node {j} computes {iv:?}", target.name, s.name);
            }
        }
    }
}

/// The golden trace's steps: its prompt, then every generated token but the last.
fn trace() -> (Int8Model, Vec<usize>) {
    let (m, prompt, generated) = spatial_support_model();
    let mut trace = prompt;
    trace.extend_from_slice(&generated[..generated.len() - 1]);
    (m, trace)
}

/// Each board's fabric target, as its images are printed, on two boards joined by a link.
fn board_targets() -> [SpatialTarget; 2] {
    [SpatialTarget::xc7a100t_fabric(16, 64).linked(), SpatialTarget::xc7a200t_fabric(16, 64).linked()]
}

/// The split step at `pos`, as the composed program over both chips evaluates it: each tensor's values by name.
fn composed_split(m: &Int8Model, tok: usize, pos: usize, kc: &[i64], vc: &[i64], target: &SpatialTarget) -> std::collections::BTreeMap<String, Vec<i64>> {
    let (src, manifest) = m.decode_step_tp_lg(tok, pos, kc, vc, 2).unwrap();
    let l = compile_program_to_spatial_tp(&src, target, &manifest).expect("the walk lifts the split step");
    let whole = l.whole().unwrap_or_else(|| panic!("{}: the split step at {pos} lowers whole: {}", target.name, l.refusal())).clone();
    let out = eval_choreography_every_tensor(&whole, &inputs_from_fills(&whole)).expect("the composed program evaluates");
    whole
        .tensors
        .iter()
        .filter_map(|t| match out.tensors.get(&t.id) {
            Some(Value::Int(v)) => Some((t.name.clone(), v.clone())),
            _ => None,
        })
        .collect()
}

/// Chip `c`'s inputs: each tensor it receives as the composed program holds it, every other its own fill.
fn chip_inputs(p: &SpatialProgram, received: &[&str], composed: &std::collections::BTreeMap<String, Vec<i64>>, c: usize) -> Inputs {
    let mut inputs = Inputs::default();
    for t in p.tensors.iter().filter(|t| t.role != TensorRole::Output) {
        let v = if received.contains(&t.name.as_str()) {
            composed[&t.name].clone()
        } else {
            (0..t.elems.rows * t.elems.cols).map(|i| t.init.element(i).unwrap_or_else(|| panic!("chip {c}'s {} has a fill", t.name))).collect()
        };
        inputs.tensors.insert(t.id, Value::Int(v));
    }
    inputs
}

/// The split step's chips, each forwarding what only its next statement reads, at every position of the trace: the
/// tensors that cross are those of the chips that forward nothing, and each chip's program, given what it receives as
/// the composed program holds it, leaves every tensor it holds at the composed program's value.
#[test]
fn each_forwarding_chip_given_what_it_receives_holds_the_composed_values() {
    let (m, trace) = trace();
    let reads: std::collections::BTreeSet<String> = ["next".to_string()].into();
    for target in board_targets() {
        let (mut kc, mut vc) = (Vec::new(), Vec::new());
        for (pos, tok) in trace.iter().enumerate() {
            let (src, manifest) = m.decode_step_tp_lg(*tok, pos, &kc, &vc, 2).unwrap();
            let plain = compile_program_to_chips(&src, &target, &manifest).expect("the walk lifts the split step");
            let lowered = compile_program_to_chips_returning(&src, &target, &manifest, &reads).expect("the walk lifts the split step");
            assert_eq!(lowered.transfers, plain.transfers, "{} position {pos}: forwarding moves no transfer", target.name);
            let composed = composed_split(&m, *tok, pos, &kc, &vc, &target);
            for (c, pl) in lowered.chips.iter().enumerate() {
                let p = pl.whole().unwrap_or_else(|| panic!("{}: chip {c} at {pos} lowers whole: {}", target.name, pl.refusal()));
                assert!(
                    p.sections.len() < plain.chips[c].whole().expect("the plain chip").sections.len(),
                    "{} position {pos}: chip {c} forwards",
                    target.name
                );
                let received: Vec<&str> = lowered.transfers.iter().filter(|t| t.to as usize == c).map(|t| t.tensor.as_str()).collect();
                let out = eval_choreography_every_tensor(p, &chip_inputs(p, &received, &composed, c)).expect("the chip's program evaluates");
                let mut held = 0;
                for t in &p.tensors {
                    let (Some(Value::Int(got)), Some(want)) = (out.tensors.get(&t.id), composed.get(&t.name)) else { continue };
                    assert_eq!(got, want, "{} position {pos}: chip {c}'s {}", target.name, t.name);
                    held += 1;
                }
                assert!(held > 5, "{} position {pos}: chip {c} is held to the composed program on {held} tensors", target.name);
            }
            let w = m.d / 2;
            let row = |name: &str| composed[name][pos * w..(pos + 1) * w].to_vec();
            kc.extend(row("Kc_c0").into_iter().chain(row("Kc_c1")));
            vc.extend(row("Vc_c0").into_iter().chain(row("Vc_c1")));
        }
    }
}

/// Each forwarding chip's program runs in RTL to the evaluator's answer in its twin's cycles, from what it receives at
/// the first step: its datapath, run through without the halts its link points add.
#[test]
fn each_forwarding_chip_runs_on_the_fabric_as_the_evaluator() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (m, trace) = trace();
    let reads: std::collections::BTreeSet<String> = ["next".to_string()].into();
    for target in board_targets() {
        let (src, manifest) = m.decode_step_tp_lg(trace[0], 0, &[], &[], 2).unwrap();
        let lowered = compile_program_to_chips_returning(&src, &target, &manifest, &reads).expect("the walk lifts the split step");
        let composed = composed_split(&m, trace[0], 0, &[], &[], &target);
        for (c, pl) in lowered.chips.iter().enumerate() {
            let mut p = pl.whole().expect("the chip lowers whole").clone();
            p.links.clear();
            let received: Vec<&str> = lowered.transfers.iter().filter(|t| t.to as usize == c).map(|t| t.tensor.as_str()).collect();
            let art = match FabricBackend::new(target.clone()).lower(&p) {
                Ok(SpatialArtifact::Verilog(v)) => v,
                other => panic!("{}: the fabric lowers chip {c}: {other:?}", target.name),
            };
            fabric_rtl::runs_as_the_evaluator(&format!("forwarding chip {c} on {}", target.name), &p, &target, &art, &chip_inputs(&p, &received, &composed, c));
        }
    }
}

fn spatial_support_model() -> (Int8Model, Vec<usize>, Vec<usize>) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let toks = |k: &str| -> Vec<usize> { j["golden_trace"][k].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as usize).collect() };
    (Int8Model::from_json(&j).expect("the model"), toks("prompt_tokens"), toks("generated_tokens"))
}
