//! Shared helpers for the spatial-IR locks (included per lock via `#[path]`, so no other test binary is
//! coupled to the spatial module).
#![allow(dead_code)]

use logicaffeine_compile::spatial::eval::Inputs;
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::ir::{SemId, SemRole, TensorId, TensorRole};

/// The canonical LOGOS matmul the recognizer lifts: `C(m×n) = A(m×k)·B(k×n)` over `Seq of Int` with the
/// deterministic fills `a[p] = (p % 7) - 3`, `b[p] = (p % 5) - 2`, and a `Show checksum` of `C`.
pub fn gemm_program(m: usize, n: usize, k: usize) -> String {
    format!(
        r#"## Main
Let rows be {m}.
Let cols be {n}.
Let mid be {k}.
Let mutable a be a new Seq of Int.
Let mutable b be a new Seq of Int.
Let mutable c be a new Seq of Int.
Let mutable p be 0.
While p is less than rows * mid:
    Push (p % 7) - 3 to a.
    Set p to p + 1.
Set p to 0.
While p is less than mid * cols:
    Push (p % 11) - 5 to b.
    Set p to p + 1.
Set p to 0.
While p is less than rows * cols:
    Push 0 to c.
    Set p to p + 1.
Let mutable i be 0.
While i is less than rows:
    Let mutable k be 0.
    While k is less than mid:
        Let mutable j be 0.
        While j is less than cols:
            Let idx be i * cols + j + 1.
            Set item idx of c to item idx of c + item (i * mid + k + 1) of a * item (k * cols + j + 1) of b.
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Let mutable checksum be 0.
Let mutable q be 1.
While q is at most rows * cols:
    Set checksum to checksum + item q of c.
    Set q to q + 1.
Show checksum.
"#
    )
}

pub fn fill_a(p: usize) -> i64 {
    (p as i64 % 7) - 3
}

pub fn fill_b(p: usize) -> i64 {
    (p as i64 % 11) - 5
}

/// The reference product of the canonical fills, row-major `m×n`, wrapping i64.
pub fn gemm_reference(m: usize, n: usize, k: usize) -> Vec<i64> {
    let mut c = vec![0i64; m * n];
    for i in 0..m {
        for kk in 0..k {
            let a = fill_a(i * k + kk);
            for j in 0..n {
                c[i * n + j] = c[i * n + j].wrapping_add(a.wrapping_mul(fill_b(kk * n + j)));
            }
        }
    }
    c
}

pub fn checksum(v: &[i64]) -> i64 {
    v.iter().fold(0i64, |s, x| s.wrapping_add(*x))
}

/// A deterministic hash-based PRNG (the same one the fabric locks use).
pub fn prng(seed: u64, i: usize, lo: i64, hi: i64) -> i64 {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(i as u64 + 1);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    lo + (x % (hi - lo + 1) as u64) as i64
}

/// Inputs for a program whose tensors carry deterministic fills (the canonical gemm): every Input/Param
/// tensor is materialized from its recorded fill ([`Inputs::from_fills`]).
pub fn inputs_from_fills(p: &SpatialProgram) -> Inputs {
    Inputs::from_fills(p).unwrap_or_else(|e| panic!("deterministic fills: {e}"))
}

pub fn output_id(p: &SpatialProgram) -> TensorId {
    p.tensors
        .iter()
        .find(|t| t.role == TensorRole::Output)
        .expect("an output tensor")
        .id
}

use logicaffeine_compile::spatial::cost::CycleModel;
use logicaffeine_compile::spatial::ir::{ChoreoOp, Schedule, SpatialProgram as Program, TaggedOp};
use logicaffeine_compile::spatial::target::SpatialTarget;

/// A path to one op inside a schedule: `(program index, indices through nested loop bodies)`.
pub type OpPath = (usize, Vec<usize>);

fn walk_paths(
    ops: &[TaggedOp],
    prefix: &mut Vec<usize>,
    prog: usize,
    out: &mut Vec<(OpPath, ChoreoOp)>,
) {
    for (i, t) in ops.iter().enumerate() {
        prefix.push(i);
        out.push(((prog, prefix.clone()), t.op.clone()));
        if let ChoreoOp::Loop { body, .. } = &t.op {
            walk_paths(body, prefix, prog, out);
        }
        prefix.pop();
    }
}

/// Every op of a schedule with its path (loop bodies included, pre-order).
pub fn enumerate_ops(s: &Schedule) -> Vec<(OpPath, ChoreoOp)> {
    let mut out = Vec::new();
    for (pi, p) in s.programs.iter().enumerate() {
        walk_paths(&p.ops, &mut Vec::new(), pi, &mut out);
    }
    out
}

fn body_at<'a>(ops: &'a mut Vec<TaggedOp>, path: &[usize]) -> &'a mut Vec<TaggedOp> {
    if path.is_empty() {
        return ops;
    }
    match &mut ops[path[0]].op {
        ChoreoOp::Loop { body, .. } => body_at(body, &path[1..]),
        _ => panic!("path descends into a non-loop op"),
    }
}

/// Apply `f` to the tagged op list containing the op at `path` (the last path element indexes into it).
pub fn edit_op<F: FnOnce(&mut Vec<TaggedOp>, usize)>(p: &mut Program, path: &OpPath, f: F) {
    let sched = p.schedule.as_mut().expect("a scheduled program");
    let prog = &mut sched.programs[path.0];
    let (parent, last) = path.1.split_at(path.1.len() - 1);
    let list = body_at(&mut prog.ops, parent);
    f(list, last[0]);
    p.certificate = None;
}

/// The program with the op at `path` removed (certificate cleared).
pub fn without_op(p: &Program, path: &OpPath) -> Program {
    let mut q = p.clone();
    edit_op(&mut q, path, |list, i| {
        list.remove(i);
    });
    q
}

/// The program with the op at `path` duplicated in place (certificate cleared).
pub fn with_op_duplicated(p: &Program, path: &OpPath) -> Program {
    let mut q = p.clone();
    edit_op(&mut q, path, |list, i| {
        let t = list[i].clone();
        list.insert(i, t);
    });
    q
}

/// The program with the op at `path` replaced (certificate cleared).
pub fn with_op_replaced(p: &Program, path: &OpPath, new: ChoreoOp) -> Program {
    let mut q = p.clone();
    edit_op(&mut q, path, |list, i| {
        list[i].op = new;
    });
    q
}

/// A Wormhole whose cost model makes multicast free and DRAM traffic expensive — a legitimate descriptor
/// knob that forces reader fusion at small shapes so the semantics/handshake locks can exercise fused
/// programs cheaply. (The decision must still come from the cost model: the locks that pin decisions use the
/// calibrated preset.)
pub fn fusing_wormhole() -> SpatialTarget {
    let mut t = SpatialTarget::wormhole();
    t.cycle = CycleModel {
        c_mcast_block: 0.0,
        c_noc_tile: 0.0,
        c_dram_tile: 1.0,
        c_dram_shared: 400.0,
        c_kernel: 0.0,
        c_sem_setup: 0.0,
        ..CycleModel::uncalibrated()
    };
    t
}

/// A Wormhole whose cost model makes every multicast prohibitively expensive.
/// A Wormhole that CANNOT multicast — the interconnect is gone, so reader fusion is not an option the
/// planner may take.
///
/// It used to be a Wormhole with `c_mcast_block` set to 2e7: multicast priced out of reach in the fitted
/// cycle model. That model no longer ranks plans (see `spatial_work_model_lock`), so pricing it did
/// nothing and the target quietly started fusing again. Removing the capability says the same thing
/// through a mechanism the compiler actually consults, and says it about the MACHINE rather than about a
/// constant.
pub fn non_fusing_wormhole() -> SpatialTarget {
    let mut t = SpatialTarget::wormhole();
    t.interconnect
        .remove(&logicaffeine_compile::spatial::target::Interconnect::MulticastRect);
    t
}

/// Parse the integer a `Show` printed.
pub fn shown_int(output: &str) -> i64 {
    output
        .trim()
        .lines()
        .last()
        .expect("a shown line")
        .trim()
        .parse()
        .expect("an integer")
}

// ── The ttsim ledger as evidence ───────────────────────────────────────────────────────────────────
//
// A ledger row is a measured (shape, knobs, plan, cycles). Two rows at the same (chip, shape, knobs) are
// measured ALTERNATIVES; the dimensions on which their plans differ are the dimensions the ledger
// evidences a decision in. A lock compares the compiler's choice to the measured winner only in those
// dimensions: a winner's unmeasured details (a sub-block nobody compared against) are not a spec.

use logicaffeine_compile::spatial::cost::{
    load_calibration_points, quantities_for_point, CalibrationPoint, PlanDesc, CALIBRATION_JSON,
};
use logicaffeine_compile::spatial::ir::PlanSummary;
use std::collections::BTreeMap;

pub fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

pub fn ledger_points() -> Vec<CalibrationPoint> {
    let text = std::fs::read_to_string(repo_root().join(CALIBRATION_JSON))
        .expect("the ttsim GEMM ledger exists");
    load_calibration_points(&text).expect("the ledger parses into calibration points")
}

/// The ledger row ids the ledger itself marks as the COMPILER'S CHOICE, per shape.
///
/// The ledger is a Pareto SWEEP: 142 Wormhole rows across five candidate strategies (`blocked-split`,
/// `streamed`, `multicast`, `multicast-split`, `blocked`), 97 of them superseded. Only one row per
/// (shape, knobs) is the plan the default pipeline picks — the rest were measured precisely BECAUSE the
/// compiler did not pick them, so that the cost model's ranking could be checked against the simulator's.
/// Comparing every measured candidate against the one plan the pipeline emits is therefore unsatisfiable
/// by construction, whatever the compiler does.
pub fn compiler_chosen_rows() -> std::collections::BTreeSet<String> {
    let text = std::fs::read_to_string(repo_root().join(CALIBRATION_JSON))
        .expect("the ttsim GEMM ledger exists");
    let v: serde_json::Value = serde_json::from_str(&text).expect("the ledger is JSON");
    v.get("frontier")
        .and_then(|f| f.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r.get("compiler_chosen"))
                .filter_map(|c| c.get("row").and_then(|s| s.as_str()))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Ledger rows grouped by (shape, chip, knobs) — the only groups whose cycles may be compared.
pub fn ledger_groups() -> BTreeMap<((usize, usize, usize), String, String), Vec<CalibrationPoint>> {
    let mut g: BTreeMap<_, Vec<CalibrationPoint>> = Default::default();
    for p in ledger_points()
        .into_iter()
        .filter(|p| p.stack != "ttnn_production")
    {
        g.entry((p.shape, p.chip.clone(), p.knobs.clone()))
            .or_default()
            .push(p);
    }
    g
}

/// The rows of a group the model can quantify on `target` (our emitted programs and the tt-metal
/// streaming example, whose choreography is ours, at the knobs the compiler emits).
pub fn quantified<'a>(
    rows: &'a [CalibrationPoint],
    target: &SpatialTarget,
) -> Vec<&'a CalibrationPoint> {
    rows.iter()
        .filter(|r| quantities_for_point(r, target).is_some())
        .collect()
}

/// The measured winner among the quantified rows, when at least two DIFFERENT plans were measured.
pub fn measured_winner<'a>(
    rows: &'a [CalibrationPoint],
    target: &SpatialTarget,
) -> Option<&'a CalibrationPoint> {
    let q = quantified(rows, target);
    let plans: std::collections::BTreeSet<&PlanDesc> = q.iter().map(|r| &r.plan).collect();
    if plans.len() < 2 {
        return None;
    }
    // ★ BY WHAT THE MACHINE DID, NOT BY ITS CLOCK. Ranking measured plans by `cycles` ranks them by
    // ttsim's bracket counter, and `ttsim_instrument_audit.json` shows that counter is not a function of
    // the program: eight times the reduction depth moves it 0.08%. A "measured winner" chosen that way
    // is a coin toss wearing a measurement's clothes, and at 512^3 it named the plan that issues 2.9x
    // the instructions and moves 3.4x the bytes. Rows without the work meter cannot be ranked at all.
    q.into_iter().filter(|r| r.work.is_some()).min_by_key(|r| {
        let w = r.work.unwrap();
        (w.noc_bytes, w.noc_txns, w.tensix_inst)
    })
}

/// Whether the compiler's plan is the plan a ledger row measured (streaming plans are one family; a blocked
/// plan must agree on fusion, grid, per-site block and block shape wherever the row records them).
pub fn plan_equals(s: &PlanSummary, w: &PlanDesc) -> bool {
    if s.streaming != w.streaming {
        return false;
    }
    if w.streaming {
        return true;
    }
    if (s.multicast_groups > 0) != w.fused {
        return false;
    }
    if w.grid != (0, 0) && s.grid != w.grid {
        return false;
    }
    if w.per_site != (0, 0) && s.per_site_block != w.per_site {
        return false;
    }
    if let (Some(b), Some(got)) = (w.block, s.block) {
        if got != b {
            return false;
        }
    }
    true
}

/// The evidence rule: the compiler may choose any plan the ledger has not measured, but never one the
/// ledger measured to LOSE against another plan at the same (shape, chip, knobs).
pub fn assert_never_a_measured_loser(
    s: &PlanSummary,
    rows: &[CalibrationPoint],
    target: &SpatialTarget,
    label: &str,
) {
    // ★ DOMINATION, NOT "NOT THE CHEAPEST". Two plans for one GEMM retire identical arithmetic, so what
    // separates them is movement — and movement has more than one axis. A plan that moves fewer bytes by
    // pushing everything through two sites has not beaten a plan that spreads across sixty-four; that is
    // a trade, and a frontier is made of them. What the compiler may never do is pick a plan that another
    // MEASURED plan beats on every axis at once, which would mean it is simply off the frontier.
    //
    // This used to compare `cycles`, which ranks by a counter the instrument audit shows does not track
    // the program at all.
    let q: Vec<&CalibrationPoint> = quantified(rows, target)
        .into_iter()
        .filter(|r| r.work.is_some())
        .collect();
    let Some(mine) = q.iter().find(|r| plan_equals(s, &r.plan)) else {
        return;
    };
    let mw = mine.work.unwrap();
    for other in &q {
        let ow = other.work.unwrap();
        if ow.matmul_retired != mw.matmul_retired {
            continue;
        }
        if ow.noc_bytes < mw.noc_bytes
            && ow.noc_txns < mw.noc_txns
            && ow.tensix_inst < mw.tensix_inst
        {
            panic!(
                "{label}: the compiler chose a plan another MEASURED plan beats on every axis — ours {} (bytes {}, txns {}, instrs {}); theirs {} (bytes {}, txns {}, instrs {})",
                mine.reproduce, mw.noc_bytes, mw.noc_txns, mw.tensix_inst, other.reproduce, ow.noc_bytes, ow.noc_txns, ow.tensix_inst
            );
        }
    }
}

// ── Programs built with an EXPLICIT plan ───────────────────────────────────────────────────────────
//
// The structure locks test what the passes PRODUCE for a given decision (groups, semaphores, buffers,
// schedules); the decision itself is locked against the ledger in the tile and cost-model locks. Building
// the program with an explicit plan keeps a structure lock independent of the calibration.

use logicaffeine_compile::spatial::cost::streaming_plan;
use logicaffeine_compile::spatial::ir::{
    AccPolicy, BlockPlan, BlockShape, EnginePacking, Granularity, InnerLoop, NumericsPolicy,
};
use logicaffeine_compile::spatial::passes::fuse_readers::fuse_readers_forced;
use logicaffeine_compile::spatial::passes::persist::persist;
use logicaffeine_compile::spatial::passes::place::place;
use logicaffeine_compile::spatial::passes::size_buffers::size_buffers;
use logicaffeine_compile::spatial::passes::synthesize_handshakes::synthesize_handshakes;
use logicaffeine_compile::spatial::passes::tile::apply_plan;
use logicaffeine_compile::spatial::passes::verify::verify;
use logicaffeine_compile::spatial::pipeline::template_from_source_with;

/// A blocked plan: per-site block, sub-block and inner width, with the grid and K blocking derived.
pub fn blocked_plan(
    mt: usize,
    nt: usize,
    kt: usize,
    per_site: (usize, usize),
    block: (usize, usize, usize),
    acc: AccPolicy,
) -> BlockPlan {
    let (osh, osw, ibw) = block;
    BlockPlan {
        per_site_m: per_site.0,
        per_site_n: per_site.1,
        grid: (mt / per_site.0, nt / per_site.1),
        inner: InnerLoop {
            num_blocks: kt / ibw,
            block_k_tiles: ibw,
        },
        block: BlockShape { osh, osw, ibw },
        acc,
        pack_accumulate: false,
        engines: EnginePacking::LoadsTogether,
        granularity: Granularity::PerTile,
    }
}

/// The GEMM through every pass after `tile`, with `plan` and the reader arrangement chosen here.
pub fn program_with_plan(
    m: usize,
    n: usize,
    k: usize,
    target: &SpatialTarget,
    plan: BlockPlan,
    fused: bool,
    numerics: NumericsPolicy,
) -> SpatialProgram {
    let t = template_from_source_with(&gemm_program(m, n, k), target, numerics)
        .unwrap_or_else(|e| panic!("{}: template: {e:?}", target.name));
    let tiled = apply_plan(&t, plan, target);
    let placed = place(&tiled, target).unwrap_or_else(|e| panic!("{}: place: {e:?}", target.name));
    let arranged = fuse_readers_forced(&placed, target, fused)
        .unwrap_or_else(|e| panic!("{}: fuse_readers: {e:?}", target.name));
    let sized = size_buffers(&arranged, target)
        .unwrap_or_else(|e| panic!("{}: size_buffers: {e:?}", target.name));
    let scheduled = synthesize_handshakes(&sized, target)
        .unwrap_or_else(|e| panic!("{}: synthesize_handshakes: {e:?}", target.name));
    let persisted =
        persist(&scheduled, target).unwrap_or_else(|e| panic!("{}: persist: {e:?}", target.name));
    verify(&persisted, target).unwrap_or_else(|e| panic!("{}: verify: {e:?}", target.name))
}

/// The measured 2048×1024×512 multicast program: 8×8 grid, per-site 8×4, inner width 16 (one K block),
/// fp32 accumulate (sub-block 1×4 inside the halved DST), readers fused along rows and columns.
pub fn fused_2048_program() -> SpatialProgram {
    program_with_plan(
        2048,
        1024,
        512,
        &SpatialTarget::wormhole(),
        blocked_plan(64, 32, 16, (8, 4), (1, 4, 16), AccPolicy::F32),
        true,
        NumericsPolicy::Exact,
    )
}

/// The per-tile streaming program (tt-metal's `matmul_multi_core` choreography) on `target`.
pub fn streamed_program(m: usize, n: usize, k: usize, target: &SpatialTarget) -> SpatialProgram {
    let (mt, nt, kt) = (
        m.div_ceil(target.tile.rows),
        n.div_ceil(target.tile.cols),
        k.div_ceil(target.tile.cols),
    );
    program_with_plan(
        m,
        n,
        k,
        target,
        streaming_plan(mt, nt, kt, AccPolicy::F32),
        false,
        NumericsPolicy::Exact,
    )
}

/// `p` with the rendezvous after section `k` removed the way a well-formed program would lack it: the span no longer
/// closes on it, its two semaphores are ordinary flags, and no engine program posts or waits on them. What is left
/// is a program whose ONLY defect is the missing order between section `k` and section `k + 1` — so a refusal of it
/// comes from the ordering obligations, not from the structural check that a dangling barrier semaphore trips first.
pub fn without_barrier_after(p: &SpatialProgram, k: usize) -> SpatialProgram {
    let mut q = p.clone();
    q.certificate = None;
    let r = q.sections[k]
        .barrier_after
        .take()
        .expect("the section closes on a rendezvous");
    let sems = [r.gather, r.release];
    for s in &sems {
        q.semaphores[s.idx()].role = SemRole::ValidFlag;
    }
    fn strip(ops: &mut Vec<TaggedOp>, sems: &[SemId]) {
        ops.retain(|t| !matches!(&t.op, ChoreoOp::SemInc { sem, .. } | ChoreoOp::SemWait { sem, .. } if sems.contains(sem)));
        for t in ops.iter_mut() {
            if let ChoreoOp::Loop { body, .. } = &mut t.op {
                strip(body, sems);
            }
        }
    }
    for prog in &mut q.schedule.as_mut().expect("scheduled").programs {
        strip(&mut prog.ops, &sems);
    }
    // The two phases the rendezvous closed are one now, and a phase's sections may not share a byte: lay the joined
    // phase out as composition lays any phase out, so the cut program is well-formed and only its ORDER is missing.
    logicaffeine_compile::spatial::passes::compose::separate_phase_memory(&mut q);
    q
}

/// `p` with sections `k` and `k + 1` — no rendezvous between them — merged into ONE section: the spans joined, the
/// result the section that has an `Output`, and the second half's local memory laid out past the first half's on each
/// site and tier (a section's live buffers may not share a byte). What is left is one section running both halves'
/// dataflow side by side, ordered by nothing but that dataflow — the shape a program from outside the front end (a
/// vendor bridge, an ingested placement) may have.
pub fn with_sections_merged(p: &SpatialProgram, k: usize) -> SpatialProgram {
    use logicaffeine_compile::spatial::ir::{MemSpace, SectionSpan, Site};
    let mut q = p.clone();
    q.certificate = None;
    let (a, b) = (q.sections[k].clone(), q.sections[k + 1].clone());
    assert!(
        a.barrier_after.is_none(),
        "sections {k} and {} are separated by a rendezvous",
        k + 1
    );
    let output = [a.output, b.output]
        .into_iter()
        .flatten()
        .find(|t| q.tensors[t.idx()].role == TensorRole::Output)
        .or(b.output);
    let merged = SectionSpan {
        name: format!("{}+{}", a.name, b.name),
        stages: (a.stages.0, b.stages.1),
        buffers: (a.buffers.0, b.buffers.1),
        edges: (a.edges.0, b.edges.1),
        semaphores: (a.semaphores.0, b.semaphores.1),
        items: (a.items.0, b.items.1),
        plan: b.plan,
        k_tiles: b.k_tiles,
        output,
        barrier_after: b.barrier_after,
    };
    q.sections.splice(k..=k + 1, [merged]);
    let memory = q.memory.as_mut().expect("laid out");
    let mut end: BTreeMap<(Site, MemSpace), u32> = BTreeMap::new();
    for at in &memory.buffer_at[a.buffers.0..a.buffers.1] {
        let e = end.entry((at.site, at.space)).or_default();
        *e = (*e).max(at.offset_bytes + at.bytes);
    }
    for at in &mut memory.buffer_at[b.buffers.0..b.buffers.1] {
        at.offset_bytes += end.get(&(at.site, at.space)).copied().unwrap_or(0);
    }
    q
}

// ── One hand-built integer `Generic`, for the locks that reach what no front-end program does yet ──

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::ir::{
    ComputeOp, ElemAccess, IterKind, IterSpec, LoopVar, NumericsClaim, ReducePool, StageKind, TensorBody, TensorInit, TileRect, WorkItem,
    WorkItemId,
};
use logicaffeine_compile::spatial::passes::from_tensor_op::Template;
use logicaffeine_compile::spatial::pipeline::run_passes;

pub fn parallel(var: LoopVar, extent: usize) -> IterSpec {
    IterSpec { var, extent, kind: IterKind::Parallel }
}

pub fn sum_fold(var: LoopVar, extent: usize) -> IterSpec {
    IterSpec { var, extent, kind: IterKind::Reduce { pool: ReducePool::Sum } }
}

pub fn elem_map(base: i64, terms: &[(LoopVar, i64)]) -> ElemAccess {
    ElemAccess { base, terms: terms.to_vec(), indirect: None }
}

/// A scheduled program of ONE integer `Generic` over `ins` — each a tensor one tile tall of the given width and values,
/// declared at the hull of its values as the front end declares a fill — writing a `cols`-wide output through `out`.
pub fn one_generic(target: &SpatialTarget, ins: &[(&str, usize, Vec<i64>)], cols: usize, iters: Vec<IterSpec>, maps: Vec<ElemAccess>, out: ElemAccess, body: TensorBody) -> SpatialProgram {
    one_generic_of(target, Precision::Int64, ins, cols, iters, maps, out, body)
}

/// One Generic op whose operand slots all read ONE input, `name` of `w` values, each through its own map: the shape in
/// which slots share a memory port, so the issue schedule may give two of them one read.
#[allow(clippy::too_many_arguments)]
pub fn one_generic_on_one_input(target: &SpatialTarget, name: &str, w: usize, vals: Vec<i64>, cols: usize, iters: Vec<IterSpec>, maps: Vec<ElemAccess>, out: ElemAccess, body: TensorBody) -> SpatialProgram {
    let t = target.tile.rows;
    let mut tp = Template::new(target).expect("a template");
    let hull = vals.iter().fold((i64::MAX, i64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    let id = tp.tensor(name, TensorRole::Input, t, w, Precision::Int64, TensorInit::Values(vals));
    let cols_t = w.div_ceil(t);
    let b = tp.buffer(&format!("in_{name}"), cols_t, Precision::Int64);
    let load = tp.load(&format!("load_{name}"), id, vec![], cols_t, b);
    let y = tp.tensor("y", TensorRole::Output, t, cols, Precision::Int64, TensorInit::Zero);
    let out_ct = cols.div_ceil(t);
    let ob = tp.buffer("out", out_ct, Precision::Int64);
    let slots = maps.len();
    let g = tp.stage("generic", StageKind::Compute(ComputeOp::Generic { iters, ins: maps, out, body }), vec![b; slots], vec![ob]);
    let st = tp.store("store_y", y, vec![], out_ct, ob);
    tp.edge(load, b, g);
    tp.edge(g, ob, st);
    let items = vec![WorkItem { id: WorkItemId::new(0), coord: (0, 0), out: TileRect { row0_t: 0, col0_t: 0, h_t: 1, w_t: out_ct } }];
    let mut p = tp
        .finish("one_generic_on_one_input".into(), items, AccPolicy::I64, NumericsClaim::Approx { reason: "a hand-built port probe".into() }, target)
        .expect("the program validates");
    p.tensors[id.idx()].range = Some(hull);
    run_passes(p, target).expect("the program schedules")
}

/// [`one_generic`] over operands of precision `dtype`, each declared at the hull of the values it is given.
#[allow(clippy::too_many_arguments)]
pub fn one_generic_of(target: &SpatialTarget, dtype: Precision, ins: &[(&str, usize, Vec<i64>)], cols: usize, iters: Vec<IterSpec>, maps: Vec<ElemAccess>, out: ElemAccess, body: TensorBody) -> SpatialProgram {
    let t = target.tile.rows;
    let mut tp = Template::new(target).expect("a template");
    let (mut loads, mut bufs, mut ids) = (Vec::new(), Vec::new(), Vec::new());
    for (name, w, vals) in ins {
        let id = tp.tensor(name, TensorRole::Input, t, *w, dtype, TensorInit::Values(vals.clone()));
        let cols_t = w.div_ceil(t);
        let b = tp.buffer(&format!("in_{name}"), cols_t, dtype);
        loads.push((tp.load(&format!("load_{name}"), id, vec![], cols_t, b), b));
        bufs.push(b);
        ids.push((id, vals.iter().fold((i64::MAX, i64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)))));
    }
    let y = tp.tensor("y", TensorRole::Output, t, cols, Precision::Int64, TensorInit::Zero);
    let out_ct = cols.div_ceil(t);
    let ob = tp.buffer("out", out_ct, Precision::Int64);
    let g = tp.stage("generic", StageKind::Compute(ComputeOp::Generic { iters, ins: maps, out, body }), bufs, vec![ob]);
    let st = tp.store("store_y", y, vec![], out_ct, ob);
    for (l, b) in loads {
        tp.edge(l, b, g);
    }
    tp.edge(g, ob, st);
    let items = vec![WorkItem { id: WorkItemId::new(0), coord: (0, 0), out: TileRect { row0_t: 0, col0_t: 0, h_t: 1, w_t: out_ct } }];
    let mut p = tp
        .finish("one_generic".into(), items, AccPolicy::I64, NumericsClaim::Approx { reason: "a hand-built interval probe".into() }, target)
        .expect("the program validates");
    for (id, hull) in ids {
        p.tensors[id.idx()].range = Some(hull);
    }
    run_passes(p, target).expect("the program schedules")
}

use logicaffeine_compile::spatial::int8_model::Int8Model;

/// The evaluator's next token (0-based) after each prompt position, the decode step lowered for `target` at every
/// position with the cache the steps before it wrote: what a board stepped through the prompt with `S` replies.
pub fn prompt_next_tokens(m: &Int8Model, prompt: &[usize], target: &SpatialTarget) -> Vec<usize> {
    m.prompt_next_tokens(prompt, target).unwrap_or_else(|e| panic!("the prompt steps: {e}"))
}
