//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! FOLD — a compute chain folds its K blocks into CARRIED state, and reads what does not move once.
//!
//! A reduction whose operand streams in K blocks (a GEMM's inner dimension, an attention's cache positions) keeps
//! its running value ON CHIP from block to block: the chain's stages sit in three levels, derived from the dataflow —
//!   • the PROLOGUE, once per item: every stage that reads nothing that moves with the K block — an operand loaded
//!     once (its load does not step `LV_KBLOCK`), and the INIT of each carried value;
//!   • the BODY, once per K block: every stage that reads a block, and each carried value's COMBINE;
//!   • the EPILOGUE, once per item after the blocks: every stage that reads only carried values and prologue data.
//! A CARRY is a scratch buffer one stage both reads and writes (its combine); a second stage writes its first page
//! (its init). Every other reader of a carry in the body sees the value from BEFORE this block's update — it is
//! ordered ahead of the combine — and the epilogue sees the value after the last block.
//!
//! Pinned:
//!   1. HOISTING. An operand that does not move with the blocks is read ONCE per item, not once per block, and the
//!      fold still computes its value.
//!   2. CARRIES FOLD. A carried sum and a carried maximum over K blocks equal the f64 reference exactly, under every
//!      scheduler, and the program certifies (its certificate rechecks).
//!   3. ORDER. A reader of a carry in the body sees the value before the block's update: `q = Σ_k (Σ_{j<k} B_j)·B_k`
//!      comes out as the EXCLUSIVE prefix product, never the inclusive one; and a carry updated from itself alone —
//!      a counter, the index a block's positions are computed from — runs once per block, and so does a value
//!      computed from the carry alone when the blocks read it: `y = Σ_k (k+1)·B_k`.
//!   4. REFUSALS BY NAME. A carry no stage initializes, and a carry two stages update, are structural violations.
//!   5. EVERY BACKEND. The RDU lane ships the fold; every other printer lowers it or refuses it by name.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_fold_lock`
//!
//! ★ MUTATION PROBES (each run 2026-09-27 in a private copy — break, observe red; logs `logs/sn50/probe_fold_*.txt`):
//!   • a carry's body readers ranked after its update (the anti-dependence dropped) → the prefix product comes out
//!     INCLUSIVE and the counter weights block `k` by `k + 1`: both order tests red on their values;
//!   • the reader issues its once-per-item loads after its K-block loop → every fold with a prologue operand
//!     DEADLOCKS at certification (the reader waits for space in the block buffer, the compute for the operand);
//!   • the carry's last page popped after the blocks without a wait → `PopBeforeWait` on the carry;
//!   • carries not validated → both refusal tests red: the uninitialized carry and the twice-updated carry certify;
//!   • an update that reads no block not forced into the body → the counter is read as prologue work and the
//!     schedule faults (`PopBeforeWait` on the counter).
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::eval::{
    eval_choreography, eval_choreography_with, Inputs, Scheduler, Value,
};
use logicaffeine_compile::spatial::ir::*;
use logicaffeine_compile::spatial::lower::{
    cuda::CudaPersistentBackend, fabric::FabricBackend, rdu::RduSpatialBackend,
    tensix::TensixBackend, SpatialArtifact, SpatialBackend, SpatialLowerError,
};
use logicaffeine_compile::spatial::passes::from_tensor_op::{ident_map, per_row_map, Template};
use logicaffeine_compile::spatial::passes::{
    SpatialReject, LV_GEN_COL, LV_GEN_ROW, LV_ITEM_Y, LV_KBLOCK,
};
use logicaffeine_compile::spatial::pipeline::run_passes;
use logicaffeine_compile::spatial::target::SpatialTarget;

/// K blocks per item.
const N: usize = 3;
/// Work items.
const ITEMS: usize = 2;

fn sn50() -> SpatialTarget {
    SpatialTarget::sn50_rdu()
}

fn op(i: usize) -> Box<TensorBody> {
    Box::new(TensorBody::Operand(i))
}

fn konst(v: f64) -> TensorBody {
    TensorBody::Const(v.to_bits())
}

/// A value exact in bf16 and in every sum and product the folds form: a multiple of 1/8 in `[-2, 2]`.
fn val(seed: usize) -> f64 {
    (((seed.wrapping_mul(7919).wrapping_add(13)) % 33) as f64 - 16.0) / 8.0
}

/// One fold's tensors and element dtype, on `target`'s tile.
struct Shape {
    t: usize,
    dtype: Precision,
}

impl Shape {
    fn of(target: &SpatialTarget) -> Shape {
        let dtype = if target.name.contains("fabric") {
            Precision::Int64
        } else {
            Precision::Fp32
        };
        Shape {
            t: target.tile.rows,
            dtype,
        }
    }
    fn elems(&self) -> usize {
        self.t * self.t
    }
    fn fill(&self, salt: usize, tiles: usize) -> (TensorInit, Vec<f64>) {
        let v: Vec<f64> = (0..tiles * self.elems())
            .map(|p| {
                if self.dtype == Precision::Int64 {
                    ((p * 7 + salt) % 5) as f64 - 2.0
                } else {
                    val(p * 31 + salt)
                }
            })
            .collect();
        let init = if self.dtype == Precision::Int64 {
            TensorInit::Values(v.iter().map(|x| *x as i64).collect())
        } else {
            TensorInit::FloatValues(v.iter().map(|x| x.to_bits()).collect())
        };
        (init, v)
    }
}

/// A pointwise `Generic` over one tile.
fn pointwise(s: &Shape, n_ins: usize, body: TensorBody) -> StageKind {
    let iters = vec![
        IterSpec {
            var: LV_GEN_ROW,
            extent: s.t,
            kind: IterKind::Parallel,
        },
        IterSpec {
            var: LV_GEN_COL,
            extent: s.t,
            kind: IterKind::Parallel,
        },
    ];
    StageKind::Compute(ComputeOp::Generic {
        iters,
        ins: vec![ident_map(s.t); n_ins],
        out: ident_map(s.t),
        body,
    })
}

/// Each row's maximum, written across the row.
fn row_max(s: &Shape) -> StageKind {
    let iters = vec![
        IterSpec {
            var: LV_GEN_ROW,
            extent: s.t,
            kind: IterKind::Parallel,
        },
        IterSpec {
            var: LV_GEN_COL,
            extent: s.t,
            kind: IterKind::Reduce {
                pool: ReducePool::Max,
            },
        },
    ];
    StageKind::Compute(ComputeOp::Generic {
        iters,
        ins: vec![ident_map(s.t)],
        out: per_row_map(s.t),
        body: TensorBody::Operand(0),
    })
}

/// `max(x, y)` of two per-row values (read from each row's first column): a `Max` over two columns that picks `x`,
/// then `y`, written across the row.
fn pairwise_max(s: &Shape) -> StageKind {
    let iters = vec![
        IterSpec {
            var: LV_GEN_ROW,
            extent: s.t,
            kind: IterKind::Parallel,
        },
        IterSpec {
            var: LV_GEN_COL,
            extent: 2,
            kind: IterKind::Reduce {
                pool: ReducePool::Max,
            },
        },
    ];
    let body = TensorBody::Select {
        cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Const(1)),
        then: op(0),
        otherwise: op(1),
    };
    StageKind::Compute(ComputeOp::Generic {
        iters,
        ins: vec![per_row_map(s.t); 2],
        out: per_row_map(s.t),
        body,
    })
}

/// A load of block `k` of item `y` — tile `y·N + k` of a tensor `ITEMS·N` tiles tall.
fn block_load(t: &mut Template, name: &str, tensor: TensorId, into: BufferId) -> StageId {
    let origin = TileAddr {
        base: 0,
        steps: vec![(LV_ITEM_Y, N), (LV_KBLOCK, 1)],
        indirect: None,
    };
    t.stage(
        name,
        StageKind::Load {
            tensor,
            pattern: AccessPattern {
                origin,
                block_h: 1,
                block_w: 1,
                stride_w: 1,
                stride_h: 1,
            },
            num_blocks: N,
            multicast: None,
        },
        vec![],
        vec![into],
    )
}

/// A load of item `y`'s one tile — the same for every block.
fn item_load(t: &mut Template, name: &str, tensor: TensorId, into: BufferId) -> StageId {
    t.load_block_at(name, tensor, 0, vec![(LV_ITEM_Y, 1)], 1, 1, 1, into)
}

/// The fold's plan: one output tile per item, `N` K blocks each.
fn finish(
    t: Template,
    name: &str,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let items = (0..ITEMS)
        .map(|y| WorkItem {
            id: WorkItemId::new(y),
            coord: (y, 0),
            out: TileRect {
                row0_t: y,
                col0_t: 0,
                h_t: 1,
                w_t: 1,
            },
        })
        .collect();
    let mut p = t.finish(
        name.to_string(),
        items,
        AccPolicy::F32,
        NumericsClaim::Exact,
        target,
    )?;
    p.work.plan = Some(BlockPlan {
        per_site_m: 1,
        per_site_n: 1,
        grid: (ITEMS, 1),
        inner: InnerLoop {
            num_blocks: N,
            block_k_tiles: 1,
        },
        block: BlockShape {
            osh: 1,
            osw: 1,
            ibw: 1,
        },
        acc: AccPolicy::F32,
        pack_accumulate: false,
        engines: EnginePacking::LoadsTogether,
        granularity: Granularity::PerTile,
    });
    p.work.k_tiles = N;
    Ok(p)
}

/// The folds, `a` loaded once per item: the carried SUM `y[i] = Σ_k a[i] ⊙ b[i, k]`, or the carried ROW MAXIMUM —
/// each row's `max_k max_c (a[i] ⊙ b[i, k])` across the row (a column fold writes its scalar across the output
/// row), the running `m` of a softmax. Returns the program and, per item, the f64 reference tile.
fn fold(target: &SpatialTarget, max: bool) -> (SpatialProgram, Vec<f64>) {
    let s = Shape::of(target);
    let mut t = Template::new(target).expect("a template");
    let (a_init, a) = s.fill(1, ITEMS);
    let (b_init, b) = s.fill(2, ITEMS * N);
    let at = t.tensor("a", TensorRole::Input, ITEMS * s.t, s.t, s.dtype, a_init);
    let bt = t.tensor(
        "b",
        TensorRole::Input,
        ITEMS * N * s.t,
        s.t,
        s.dtype,
        b_init,
    );
    let yt = t.tensor(
        "y",
        TensorRole::Output,
        ITEMS * s.t,
        s.t,
        s.dtype,
        TensorInit::Zero,
    );
    let a_in = t.buffer_deep("in_a", 1, 1, s.dtype);
    let b_in = t.buffer("in_b", 1, s.dtype);
    let prod = t.scratch_deep("prod", 1, 1, s.dtype);
    let acc = t.scratch_deep("acc", 2, 1, s.dtype);
    let out = t.buffer("out", 1, s.dtype);
    let la = item_load(&mut t, "load_a", at, a_in);
    let lb = block_load(&mut t, "load_b", bt, b_in);
    let init = if max { -1.0e30 } else { 0.0 };
    let s_init = t.stage("acc_init", pointwise(&s, 0, konst(init)), vec![], vec![acc]);
    let s_prod = t.stage(
        "prod",
        pointwise(&s, 2, TensorBody::Mul(op(0), op(1))),
        vec![a_in, b_in],
        vec![prod],
    );
    let (s_fold, fold_from) = if max {
        let blk = t.scratch_deep("block_max", 1, 1, s.dtype);
        let s_blk = t.stage("block_max", row_max(&s), vec![prod], vec![blk]);
        t.edge(s_prod, prod, s_blk);
        (
            t.stage("acc_fold", pairwise_max(&s), vec![acc, blk], vec![acc]),
            (s_blk, blk),
        )
    } else {
        (
            t.stage(
                "acc_fold",
                pointwise(&s, 2, TensorBody::Add(op(0), op(1))),
                vec![acc, prod],
                vec![acc],
            ),
            (s_prod, prod),
        )
    };
    let s_out = t.stage(
        "acc_out",
        pointwise(&s, 1, TensorBody::Operand(0)),
        vec![acc],
        vec![out],
    );
    let st = t.store_block("store_y", yt, 0, vec![(LV_ITEM_Y, 1)], 1, 1, 1, out);
    t.edge(la, a_in, s_prod);
    t.edge(lb, b_in, s_prod);
    t.edge(fold_from.0, fold_from.1, s_fold);
    t.edge(s_init, acc, s_fold);
    t.edge(s_out, out, st);
    let p = finish(t, if max { "fold_max" } else { "fold_sum" }, target)
        .unwrap_or_else(|e| panic!("the fold template validates: {e:?}"));
    let n = s.elems();
    let mut want = vec![init; ITEMS * n];
    for y in 0..ITEMS {
        for e in 0..n {
            let r = e / s.t;
            want[y * n + e] = if max {
                (0..N)
                    .flat_map(|k| (0..s.t).map(move |cc| (k, cc)))
                    .map(|(k, cc)| a[y * n + r * s.t + cc] * b[(y * N + k) * n + r * s.t + cc])
                    .fold(init, f64::max)
            } else {
                (0..N).map(|k| a[y * n + e] * b[(y * N + k) * n + e]).sum()
            };
        }
    }
    (p, want)
}

/// The order probe: `s` carries `Σ_{j<k} b_j` into block `k`, and `q = Σ_k s·b_k` reads `s` before its update.
fn prefix_product(target: &SpatialTarget) -> (SpatialProgram, Vec<f64>) {
    let sh = Shape::of(target);
    let mut t = Template::new(target).expect("a template");
    let (b_init, b) = sh.fill(3, ITEMS * N);
    let bt = t.tensor(
        "b",
        TensorRole::Input,
        ITEMS * N * sh.t,
        sh.t,
        sh.dtype,
        b_init,
    );
    let yt = t.tensor(
        "q",
        TensorRole::Output,
        ITEMS * sh.t,
        sh.t,
        sh.dtype,
        TensorInit::Zero,
    );
    let b_in = t.buffer("in_b", 1, sh.dtype);
    let s = t.scratch_deep("s", 2, 1, sh.dtype);
    let q = t.scratch_deep("q", 2, 1, sh.dtype);
    let term = t.scratch_deep("term", 1, 1, sh.dtype);
    let out = t.buffer("out", 1, sh.dtype);
    let lb = block_load(&mut t, "load_b", bt, b_in);
    let s_init = t.stage("s_init", pointwise(&sh, 0, konst(0.0)), vec![], vec![s]);
    let q_init = t.stage("q_init", pointwise(&sh, 0, konst(0.0)), vec![], vec![q]);
    // Declared AHEAD of the update it must precede only by the order the levels impose — ids put it last.
    let s_fold = t.stage(
        "s_fold",
        pointwise(&sh, 2, TensorBody::Add(op(0), op(1))),
        vec![s, b_in],
        vec![s],
    );
    let s_term = t.stage(
        "term",
        pointwise(&sh, 2, TensorBody::Mul(op(0), op(1))),
        vec![s, b_in],
        vec![term],
    );
    let q_fold = t.stage(
        "q_fold",
        pointwise(&sh, 2, TensorBody::Add(op(0), op(1))),
        vec![q, term],
        vec![q],
    );
    let s_out = t.stage(
        "q_out",
        pointwise(&sh, 1, TensorBody::Operand(0)),
        vec![q],
        vec![out],
    );
    let st = t.store_block("store_q", yt, 0, vec![(LV_ITEM_Y, 1)], 1, 1, 1, out);
    t.edge(lb, b_in, s_fold);
    t.edge(s_init, s, s_fold);
    t.edge(q_init, q, q_fold);
    t.edge(s_term, term, q_fold);
    t.edge(s_out, out, st);
    let p = finish(t, "fold_prefix_product", target)
        .unwrap_or_else(|e| panic!("the prefix-product template validates: {e:?}"));
    let n = sh.elems();
    let mut want = vec![0.0; ITEMS * n];
    for y in 0..ITEMS {
        for e in 0..n {
            let mut run = 0.0;
            for k in 0..N {
                let bk = b[(y * N + k) * n + e];
                want[y * n + e] += run * bk;
                run += bk;
            }
        }
    }
    (p, want)
}

/// The counter probe: a carry updated from itself alone — `k ← k + 1` each block, the index a block's position is
/// computed from — and a value computed from that carry alone, `w = k + 1`, weighting each block: `y = Σ_k (k+1)·b_k`.
/// `w` reads nothing that moves with the blocks, yet the blocks read it, so it runs once per block.
fn counted(target: &SpatialTarget) -> (SpatialProgram, Vec<f64>) {
    let sh = Shape::of(target);
    let mut t = Template::new(target).expect("a template");
    let (b_init, b) = sh.fill(4, ITEMS * N);
    let bt = t.tensor(
        "b",
        TensorRole::Input,
        ITEMS * N * sh.t,
        sh.t,
        sh.dtype,
        b_init,
    );
    let yt = t.tensor(
        "y",
        TensorRole::Output,
        ITEMS * sh.t,
        sh.t,
        sh.dtype,
        TensorInit::Zero,
    );
    let b_in = t.buffer("in_b", 1, sh.dtype);
    let k = t.scratch_deep("k", 2, 1, sh.dtype);
    let acc = t.scratch_deep("acc", 2, 1, sh.dtype);
    let weight = t.scratch_deep("weight", 1, 1, sh.dtype);
    let term = t.scratch_deep("term", 1, 1, sh.dtype);
    let out = t.buffer("out", 1, sh.dtype);
    let lb = block_load(&mut t, "load_b", bt, b_in);
    let k_init = t.stage("k_init", pointwise(&sh, 0, konst(0.0)), vec![], vec![k]);
    let acc_init = t.stage("acc_init", pointwise(&sh, 0, konst(0.0)), vec![], vec![acc]);
    let k_next = t.stage(
        "k_next",
        pointwise(&sh, 1, TensorBody::Add(op(0), Box::new(konst(1.0)))),
        vec![k],
        vec![k],
    );
    let s_weight = t.stage(
        "weight",
        pointwise(&sh, 1, TensorBody::Add(op(0), Box::new(konst(1.0)))),
        vec![k],
        vec![weight],
    );
    let s_term = t.stage(
        "term",
        pointwise(&sh, 2, TensorBody::Mul(op(0), op(1))),
        vec![weight, b_in],
        vec![term],
    );
    let acc_fold = t.stage(
        "acc_fold",
        pointwise(&sh, 2, TensorBody::Add(op(0), op(1))),
        vec![acc, term],
        vec![acc],
    );
    let s_out = t.stage(
        "acc_out",
        pointwise(&sh, 1, TensorBody::Operand(0)),
        vec![acc],
        vec![out],
    );
    let st = t.store_block("store_y", yt, 0, vec![(LV_ITEM_Y, 1)], 1, 1, 1, out);
    t.edge(lb, b_in, s_term);
    t.edge(k_init, k, k_next);
    t.edge(s_weight, weight, s_term);
    t.edge(acc_init, acc, acc_fold);
    t.edge(s_term, term, acc_fold);
    t.edge(s_out, out, st);
    let p = finish(t, "fold_counted", target)
        .unwrap_or_else(|e| panic!("the counter template validates: {e:?}"));
    let n = sh.elems();
    let want = (0..ITEMS * n)
        .map(|i| {
            (0..N)
                .map(|kk| (kk + 1) as f64 * b[((i / n) * N + kk) * n + i % n])
                .sum()
        })
        .collect();
    (p, want)
}

fn verified(target: &SpatialTarget, p: SpatialProgram) -> SpatialProgram {
    let name = p.name.clone();
    let v = run_passes(p, target)
        .unwrap_or_else(|e| panic!("{name} on {}: the passes must verify: {e:?}", target.name));
    assert_eq!(v.phase(), Phase::Verified, "{name}: verified");
    assert!(
        v.certificate.as_ref().is_some_and(|c| c.recheck(&v)),
        "{name}: its certificate rechecks"
    );
    v
}

fn output(p: &SpatialProgram, sched: Option<Scheduler>) -> Vec<f64> {
    let out = p
        .tensors
        .iter()
        .find(|t| t.role == TensorRole::Output)
        .expect("an output")
        .id;
    let o = match sched {
        None => eval_choreography(p, &Inputs::default()),
        Some(s) => eval_choreography_with(p, &Inputs::default(), s),
    }
    .unwrap_or_else(|e| panic!("{}: the choreography evaluates: {e:?}", p.name));
    match &o.tensors[&out] {
        Value::Float(x) => x.clone(),
        Value::Int(x) => x.iter().map(|i| *i as f64).collect(),
    }
}

/// Tiles of `tensor` the schedule reads, every loop's count multiplied in.
fn tiles_read(p: &SpatialProgram, tensor: &str) -> u64 {
    fn walk(p: &SpatialProgram, ops: &[TaggedOp], mult: u64, tensor: &str, n: &mut u64) {
        for t in ops {
            match &t.op {
                ChoreoOp::Loop { count, body, .. } => {
                    walk(p, body, mult * count.bound() as u64, tensor, n)
                }
                ChoreoOp::ReadTiles {
                    tensor: id,
                    block_h,
                    block_w,
                    ..
                } if p.tensor(*id).name == tensor => *n += mult * (*block_h * *block_w) as u64,
                _ => {}
            }
        }
    }
    let mut n = 0;
    for prog in &p.schedule.as_ref().expect("scheduled").programs {
        walk(p, &prog.ops, 1, tensor, &mut n);
    }
    n
}

// ───────────────────────────── 1. hoisting ─────────────────────────────

#[test]
fn an_operand_that_does_not_move_with_the_blocks_is_read_once_per_item() {
    let t = sn50();
    let (p, want) = fold(&t, false);
    let v = verified(&t, p);
    assert_eq!(
        tiles_read(&v, "a"),
        ITEMS as u64,
        "the item's operand is read once per item, not once per block"
    );
    assert_eq!(
        tiles_read(&v, "b"),
        (ITEMS * N) as u64,
        "every block of the streamed operand is read once"
    );
    assert_eq!(output(&v, None), want, "and the fold computes its value");
}

// ───────────────────────────── 2. carries fold ─────────────────────────────

#[test]
fn a_carried_sum_and_a_carried_max_fold_every_block_under_every_scheduler() {
    let t = sn50();
    for max in [false, true] {
        let (p, want) = fold(&t, max);
        let v = verified(&t, p);
        let what = if max { "max" } else { "sum" };
        assert_eq!(output(&v, None), want, "the carried {what}");
        for s in [
            Scheduler::RoundRobin,
            Scheduler::Reverse,
            Scheduler::Seeded(7),
            Scheduler::MovementFirst,
            Scheduler::ComputeFirst,
        ] {
            assert_eq!(output(&v, Some(s)), want, "the carried {what} under {s:?}");
        }
    }
}

// ───────────────────────────── 3. order ─────────────────────────────

#[test]
fn a_reader_of_a_carry_sees_the_value_before_the_blocks_update() {
    let t = sn50();
    let (p, want) = prefix_product(&t);
    let v = verified(&t, p);
    assert_eq!(
        output(&v, None),
        want,
        "`q` is built from the EXCLUSIVE prefix of `b`"
    );
    for s in [
        Scheduler::Reverse,
        Scheduler::Seeded(11),
        Scheduler::ComputeFirst,
    ] {
        assert_eq!(output(&v, Some(s)), want, "under {s:?}");
    }
}

#[test]
fn a_carry_updated_from_itself_alone_counts_the_blocks() {
    let t = sn50();
    let (p, want) = counted(&t);
    // The levels of the template as written — the passes may fuse `weight` into the stage that reads it.
    let levels = p.fold_levels();
    let level = |name: &str| {
        levels[p
            .stages
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("a stage {name}"))
            .id
            .idx()]
    };
    assert_eq!(
        level("k_next"),
        FoldLevel::Body,
        "an update reads no block, and still runs once per block"
    );
    assert_eq!(
        level("weight"),
        FoldLevel::Body,
        "what the blocks read, computed from a carry alone, runs once per block"
    );
    let v = verified(&t, p);
    assert_eq!(
        output(&v, None),
        want,
        "`y = Σ_k (k+1)·b_k`: block `k` sees the count of the blocks before it"
    );
    for s in [Scheduler::Reverse, Scheduler::ComputeFirst] {
        assert_eq!(output(&v, Some(s)), want, "under {s:?}");
    }
}

// ───────────────────────────── 4. refusals by name ─────────────────────────────

#[test]
fn a_carry_nothing_initializes_is_refused_by_name() {
    let t = sn50();
    let s = Shape::of(&t);
    let mut tp = Template::new(&t).expect("a template");
    let (b_init, _) = s.fill(2, ITEMS * N);
    let bt = tp.tensor(
        "b",
        TensorRole::Input,
        ITEMS * N * s.t,
        s.t,
        s.dtype,
        b_init,
    );
    let yt = tp.tensor(
        "y",
        TensorRole::Output,
        ITEMS * s.t,
        s.t,
        s.dtype,
        TensorInit::Zero,
    );
    let b_in = tp.buffer("in_b", 1, s.dtype);
    let acc = tp.scratch_deep("acc", 2, 1, s.dtype);
    let out = tp.buffer("out", 1, s.dtype);
    let lb = block_load(&mut tp, "load_b", bt, b_in);
    let s_fold = tp.stage(
        "acc_fold",
        pointwise(&s, 2, TensorBody::Add(op(0), op(1))),
        vec![acc, b_in],
        vec![acc],
    );
    let s_out = tp.stage(
        "acc_out",
        pointwise(&s, 1, TensorBody::Operand(0)),
        vec![acc],
        vec![out],
    );
    let st = tp.store_block("store_y", yt, 0, vec![(LV_ITEM_Y, 1)], 1, 1, 1, out);
    tp.edge(lb, b_in, s_fold);
    tp.edge(s_out, out, st);
    match finish(tp, "fold_uninitialized", &t) {
        Err(SpatialReject::Invalid(IrViolation::CarryWithoutInit { buffer })) => {
            assert_eq!(buffer, acc, "the violation names the carry")
        }
        other => panic!(
            "a carry no stage initializes must be refused as `CarryWithoutInit`, got {:?}",
            other.map(|p| p.name)
        ),
    }
}

#[test]
fn a_carry_two_stages_update_is_refused_by_name() {
    let t = sn50();
    let s = Shape::of(&t);
    let mut tp = Template::new(&t).expect("a template");
    let (b_init, _) = s.fill(2, ITEMS * N);
    let bt = tp.tensor(
        "b",
        TensorRole::Input,
        ITEMS * N * s.t,
        s.t,
        s.dtype,
        b_init,
    );
    let yt = tp.tensor(
        "y",
        TensorRole::Output,
        ITEMS * s.t,
        s.t,
        s.dtype,
        TensorInit::Zero,
    );
    let b_in = tp.buffer("in_b", 1, s.dtype);
    let acc = tp.scratch_deep("acc", 2, 1, s.dtype);
    let out = tp.buffer("out", 1, s.dtype);
    let lb = block_load(&mut tp, "load_b", bt, b_in);
    let s_init = tp.stage("acc_init", pointwise(&s, 0, konst(0.0)), vec![], vec![acc]);
    let s_fold = tp.stage(
        "acc_fold",
        pointwise(&s, 2, TensorBody::Add(op(0), op(1))),
        vec![acc, b_in],
        vec![acc],
    );
    let _ = tp.stage(
        "acc_again",
        pointwise(&s, 2, TensorBody::Mul(op(0), op(1))),
        vec![acc, b_in],
        vec![acc],
    );
    let s_out = tp.stage(
        "acc_out",
        pointwise(&s, 1, TensorBody::Operand(0)),
        vec![acc],
        vec![out],
    );
    let st = tp.store_block("store_y", yt, 0, vec![(LV_ITEM_Y, 1)], 1, 1, 1, out);
    tp.edge(lb, b_in, s_fold);
    tp.edge(s_init, acc, s_fold);
    tp.edge(s_out, out, st);
    match finish(tp, "fold_updated_twice", &t) {
        Err(SpatialReject::Invalid(IrViolation::CarryUpdatedTwice { buffer })) => {
            assert_eq!(buffer, acc, "the violation names the carry")
        }
        other => panic!(
            "a carry two stages update must be refused as `CarryUpdatedTwice`, got {:?}",
            other.map(|p| p.name)
        ),
    }
}

// ───────────────────────────── 5. every backend ─────────────────────────────

#[test]
fn the_rdu_ships_the_fold_and_every_other_printer_lowers_it_or_refuses_it_by_name() {
    let backends: Vec<(&str, Box<dyn SpatialBackend>)> = vec![
        (
            "Tensix",
            Box::new(TensixBackend::new(SpatialTarget::wormhole())),
        ),
        (
            "fabric",
            Box::new(FabricBackend::new(SpatialTarget::sky_fabric(64, 64))),
        ),
        (
            "CUDA",
            Box::new(CudaPersistentBackend::new(SpatialTarget::rtx3060())),
        ),
        ("RDU", Box::new(RduSpatialBackend::new())),
    ];
    let mut matrix = Vec::new();
    for (name, backend) in backends {
        let target = backend.target().clone();
        let (p, want) = fold(&target, false);
        let v = match run_passes(p, &target) {
            Ok(v) => v,
            // A target whose passes cannot build the fold refuses it before any printer runs — by a named reason.
            Err(e) if name != "RDU" => {
                matrix.push(format!("{name}: the passes refuse: {e:?}"));
                continue;
            }
            Err(e) => panic!("RDU: the SN50's passes verify the fold, got {e:?}"),
        };
        match (name, backend.lower(&v)) {
            ("RDU", Ok(SpatialArtifact::SdkGated(art))) => {
                assert!(art.certificate.recheck(&v), "RDU: the shipped certificate rechecks against the fold");
                assert_eq!(output(&v, None), want, "RDU: the program it ships computes the fold");
                matrix.push("RDU: ships (SdkGated)".to_string());
            }
            ("RDU", other) => panic!("RDU: the neutral lane ships the fold, got {other:?}"),
            (_, Ok(_)) => {
                assert_eq!(output(&v, None), want, "{name}: a printer that lowers the fold lowers a program that computes it");
                matrix.push(format!("{name}: lowers"));
            }
            (_, Err(SpatialLowerError::Unsupported(reason))) => {
                assert!(reason.split_whitespace().count() >= 3, "{name}: a refusal names what it refused, got `{reason}`");
                matrix.push(format!("{name}: refuses: {reason}"));
            }
            (_, Err(other)) => panic!("{name}: must lower the fold or refuse it with `Unsupported(<reason>)`, got {other:?}"),
        }
    }
    println!("fold × backend:\n  {}", matrix.join("\n  "));
    assert_eq!(matrix.len(), 4, "every backend answers: {matrix:?}");
}
