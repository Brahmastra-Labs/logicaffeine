//! The spatial IR as a PERSISTENT CUDA kernel (sm_80+).
//!
//! A site is a resident thread block that loops over ITS work items — the same static placement every other
//! target runs, so no atomic work counter appears. A FIFO buffer is a shared-memory ring whose pages are
//! `cp.async` commit groups: `Reserve` names the next slot, a load is every thread's share of the page in
//! 16-byte `cp.async` copies, `Push` commits them as one group, `Wait` waits until nothing but the prefetched
//! stage is still in flight and then crosses a block barrier (a thread's wait completes only its own copies;
//! the barrier publishes everyone's), and `Pop` crosses a block barrier so no thread refills a slot another
//! still reads. The ring has no run-time flow control, so the lowering proves that no ring is filled past its
//! depth before it prints (`ring_occupancy`). A tile matmul is WMMA fragments held per warp across the whole
//! reduction. Every statement is the image of one choreography op, which is what the census asserts.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    require_one_plan, require_verified, Census, SpatialArtifact, SpatialBackend, SpatialLowerError,
};
use crate::codegen_sva::fabric_target::Precision;
use crate::spatial::ir::*;
use crate::spatial::passes::{LV_ITEM, LV_ITEM_X, LV_ITEM_Y, LV_SUB_H, LV_SUB_W};
use crate::spatial::target::SpatialTarget;

mod general;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PersistentBudget {
    pub smem_bytes: usize,
    /// The kernel needs `cudaFuncAttributeMaxDynamicSharedMemorySize` — its DYNAMIC block exceeds 48 KiB.
    pub dynamic_opt_in: bool,
    /// The RINGS could not stay static (ptxas caps static `__shared__` at 48 KiB whatever the attribute
    /// says), so they are carved out of the dynamic block.
    pub rings_dynamic: bool,
    /// Bytes of the dynamic block: the epilogue scratch always, plus the rings when they had to move.
    pub dyn_bytes: usize,
    pub regs_est: usize,
    pub blocks_per_sm: usize,
    pub threads: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CudaArtifact {
    pub program_cu: String,
    pub bench_cu: String,
    pub budget: PersistentBudget,
    pub census: Census,
    /// Where the program runs as one block in a fixed order, the same program as one block per site, its engines
    /// handing off through rings and semaphores at run time.
    pub general: Option<Box<CudaArtifact>>,
}

pub struct CudaPersistentBackend {
    pub target: SpatialTarget,
}

impl CudaPersistentBackend {
    pub fn new(target: SpatialTarget) -> Self {
        CudaPersistentBackend { target }
    }
}

/// Warps per block. Four is the occupancy sweet spot on sm_86 for a WMMA tile block and divides every
/// sub-block count the tile pass produces.
/// Warps per block — the site's accumulator LANES, read from the target so the planner's split and
/// the emitted `NTHREAD` cannot disagree. It was a fixed 4; eight lanes hold twice the accumulator
/// tiles per block, which is what a competitive GEMM's block tile needs.
fn nwarp(target: &SpatialTarget) -> usize {
    target.accumulator_lanes()
}
const WARP: usize = 32;

/// Shared-memory regions start on a 16-byte boundary — wmma loads want one.
fn align16(n: usize) -> usize {
    (n + 15) & !15
}

/// The C type a precision's operand fragments take, and the type its accumulator takes.
fn wmma_types(operand: Precision) -> Option<(&'static str, &'static str, &'static str)> {
    match operand {
        // (operand C type, accumulator C type, the printf/host element type)
        Precision::Int8 => Some(("signed char", "int", "int")),
        Precision::Bf16 => Some(("__nv_bfloat16", "float", "float")),
        Precision::Fp16 => Some(("__half", "float", "float")),
        _ => None,
    }
}

struct Ctx<'a> {
    p: &'a SpatialProgram,
    te: usize,
    tr: usize,
    tc: usize,
    grid_nx: usize,
    /// Buffer template → (depth pages, page elements, C element type).
    rings: BTreeMap<BufferId, (usize, usize, &'static str)>,
    operand: Precision,
    acc_t: &'static str,
    /// The block's `(osh, osw)` output-tile shape, so the multiply and the store read ONE
    /// warp-to-tile map. Two maps would put a warp's accumulators in another warp's tiles.
    acc_shape: Option<(usize, usize)>,
    /// ★ THE OUTPUT BUFFER WHOSE RING IS REPLACED BY A PER-WARP SCRATCH.
    ///
    /// Staging the whole block's output through shared memory costs `osh · osw` tiles at the
    /// ACCUMULATOR's width, double-buffered — the largest ring in the kernel, and the one that
    /// capped how big a block tile the planner could afford. At 16 output tiles it is 32 KiB of a
    /// 99 KiB budget; at the 64 tiles a competitive GEMM wants it would be 128 KiB, which is why
    /// the planner could never get there.
    ///
    /// It buys nothing. Each warp already holds its own tiles in registers, so it can store ONE
    /// fragment to a tile-sized slot of its own and write that straight to global — a scratch of
    /// `NWARP` tiles, independent of the block size. The epilogue's ring, its pipeline, its two
    /// barriers and its block-wide copy all go away with it.
    fused_out: Option<BufferId>,
    /// How the block's tiles split across the site's accumulator lanes — resolved ONCE from the
    /// target, so the multiply, the store and the planner all read the same rule.
    lane_grid: Option<(usize, usize)>,
    /// The site's accumulator lanes (CUDA warps per block).
    lanes: usize,
}

fn ring(ctx: &Ctx, b: BufferId) -> BufferId {
    ctx.p.buffer(b).origin.unwrap_or(b)
}

/// A buffer's ring template, before a `Ctx` exists.
fn ring_of(p: &SpatialProgram, b: BufferId) -> BufferId {
    p.buffer(b).origin.unwrap_or(b)
}

fn var_name(v: LoopVar) -> String {
    match v {
        x if x == LV_ITEM => "item".into(),
        x if x == LV_SUB_H => "sh".into(),
        x if x == LV_SUB_W => "sw".into(),
        x => format!("lv{}", x.0),
    }
}

/// A tile address as C, over the loop variables in scope.
fn addr_expr(at: &TileAddr, scope: &BTreeSet<LoopVar>) -> String {
    let mut terms = vec![format!("{}", at.base)];
    for (v, stride) in &at.steps {
        if *stride == 0 {
            continue;
        }
        let n = if *v == LV_ITEM_Y {
            "oy".to_string()
        } else if *v == LV_ITEM_X {
            "ox".to_string()
        } else if scope.contains(v) {
            var_name(*v)
        } else {
            continue;
        };
        terms.push(format!("{n} * {stride}"));
    }
    terms.join(" + ")
}

struct Emit {
    s: String,
    ind: usize,
}

impl Emit {
    fn l(&mut self, line: impl AsRef<str>) {
        for _ in 0..self.ind {
            self.s.push_str("    ");
        }
        self.s.push_str(line.as_ref());
        self.s.push('\n');
    }
    fn open(&mut self, line: impl AsRef<str>) {
        self.l(format!("{} {{", line.as_ref()));
        self.ind += 1;
    }
    fn close(&mut self) {
        self.ind -= 1;
        self.l("}");
    }
}

/// Is `op` a Wait (or, with `pop`, a Pop) on a ring the kernel keeps — the ops whose run one block barrier
/// closes.
fn ring_op(ctx: &Ctx, op: Option<&TaggedOp>, pop: bool) -> bool {
    match op.map(|t| &t.op) {
        Some(ChoreoOp::Wait { buf, .. }) if !pop => ctx.fused_out != Some(ring(ctx, *buf)),
        Some(ChoreoOp::Pop { buf, .. }) if pop => ctx.fused_out != Some(ring(ctx, *buf)),
        _ => false,
    }
}

/// Print `ops`. `in_flight` is how many commit groups the printed code has issued past the stage a Wait
/// reads: zero in straight-line code, the prefetched producer half's commits inside a rotated consumer half.
fn emit_ops(
    ctx: &Ctx,
    e: &mut Emit,
    ops: &[TaggedOp],
    scope: &mut BTreeSet<LoopVar>,
    in_flight: usize,
) -> Result<(), SpatialLowerError> {
    for (i, t) in ops.iter().enumerate() {
        let next = ops.get(i + 1);
        match &t.op {
            // The fused output has no ring, so its flow-control ops have nothing to control: the
            // accumulators are already where the epilogue reads them.
            ChoreoOp::Reserve { buf, .. } if ctx.fused_out == Some(ring(ctx, *buf)) => {}
            ChoreoOp::Push { buf, .. } if ctx.fused_out == Some(ring(ctx, *buf)) => {}
            ChoreoOp::Wait { buf, .. } if ctx.fused_out == Some(ring(ctx, *buf)) => {}
            ChoreoOp::Pop { buf, .. } if ctx.fused_out == Some(ring(ctx, *buf)) => {}
            ChoreoOp::Reserve { buf, .. } => e.l(format!("const int wslot_{0} = wctr_{0} % DEPTH_{0}; ++wctr_{0};", ring(ctx, *buf).idx())),
            ChoreoOp::Push { .. } => e.l("__pipeline_commit();"),
            // One barrier closes a run of waits: it is what makes every thread's completed copies visible to
            // every other thread, and one crossing does that for the whole run.
            ChoreoOp::Wait { buf, .. } => {
                e.l(format!("__pipeline_wait_prior({in_flight}); const int rslot_{0} = rctr_{0} % DEPTH_{0};", ring(ctx, *buf).idx()));
                if !ring_op(ctx, next, false) {
                    e.l("__syncthreads();");
                }
            }
            ChoreoOp::Pop { buf, .. } => {
                e.l(format!("++rctr_{};", ring(ctx, *buf).idx()));
                if !ring_op(ctx, next, true) {
                    e.l("__syncthreads();");
                }
            }
            ChoreoOp::ReadBarrier | ChoreoOp::WriteBarrier | ChoreoOp::Flush { .. } => {}
            // A page is filled in 16-byte chunks, each thread issuing its own share — the width a single
            // `cp.async` moves. One block-collective copy per tile splits a tile over every thread in pieces
            // too small for that path, and without a 16-byte size guarantee the copy cannot take it at all.
            ChoreoOp::ReadTiles { tensor, at, block_h, block_w, stride_w, stride_h, into } => {
                let r = ring(ctx, *into).idx();
                let te = ctx.te;
                let base = addr_expr(at, scope);
                e.open("");
                e.l(format!("constexpr int chunks = (int)(sizeof(*ring_{r}) * {te} / 16);"));
                e.l(format!("static_assert(sizeof(*ring_{r}) * {te} % 16 == 0, \"a tile is a whole number of 16-byte chunks\");"));
                e.open(format!("for (int c = tid; c < {block_h} * {block_w} * chunks; c += NTHREAD)"));
                e.l(format!("const int tl = c / chunks, off = c % chunks, bh = tl / {block_w}, bw = tl % {block_w};"));
                e.l(format!("const long long tile = ({base}) + (long long)bh * {stride_h} + (long long)bw * {stride_w};"));
                e.l(format!("__pipeline_memcpy_async(reinterpret_cast<char*>(&ring_{r}[wslot_{r} * PAGE_{r} + tl * {te}]) + off * 16, reinterpret_cast<const char*>(&t{}[tile * {te}]) + off * 16, 16);", tensor.idx()));
                e.close();
                e.close();
            }
            // ★ THE FUSED EPILOGUE: each warp's accumulators go straight to global, one tile at a
            // time through a slot of its own. No output ring, no pipeline, no block-wide copy —
            // and the shared memory that ring used to hold is what pays for a bigger block tile.
            ChoreoOp::WriteTiles { from, tensor, at, block_w, stride_w, stride_h, .. }
                if ctx.fused_out == Some(ring(ctx, *from)) =>
            {
                let te = ctx.te;
                let base = addr_expr(at, scope);
                let (osh, osw) = ctx.acc_shape.expect("a fused epilogue knows its block shape");
                let (wm, wn) = ctx.lane_grid.expect("a fused epilogue has an even lane grid");
                let (rm, rn) = (osh / wm, osw / wn);
                let acct = ctx.acc_t;
                // Its OWN names: the multiply declares `wi`/`wj` from the same split, and where the
                // schedule puts both in one scope a shared name is a compile error.
                e.l(format!("const int ewi = warp / {wn}, ewj = warp % {wn};"));
                e.l("const int lane = tid % 32;");
                e.open(format!("for (int i = 0; i < {rm}; ++i)"));
                e.open(format!("for (int j = 0; j < {rn}; ++j)"));
                e.l(format!("const int bh = ewi * {rm} + i, bw = ewj * {rn} + j;"));
                e.l(format!("const long long tile = ({base}) + (long long)bh * {stride_h} + (long long)bw * {stride_w};"));
                e.l(format!("nvcuda::wmma::store_matrix_sync(&epi_scratch[warp * {te}], accf[i * {rn} + j], {}, nvcuda::wmma::mem_row_major);", ctx.tc));
                // The warp wrote the slot with `store_matrix_sync`, which is warp-synchronous but
                // not a memory fence for the reads that follow — the same 32 lanes must see it.
                e.l("__syncwarp();");
                e.open(format!("for (int el = lane; el < {te}; el += 32)"));
                e.l(format!("t{}[tile * {te} + el] = (long long) epi_scratch[warp * {te} + el];", tensor.idx()));
                e.close();
                e.l("__syncwarp();");
                e.close();
                e.close();
                let _ = (block_w, acct);
            }
            ChoreoOp::WriteTiles { from, tensor, at, block_h, block_w, stride_w, stride_h } => {
                let r = ring(ctx, *from).idx();
                let te = ctx.te;
                let base = addr_expr(at, scope);
                e.open(format!("for (int bh = 0; bh < {block_h}; ++bh)"));
                e.open(format!("for (int bw = 0; bw < {block_w}; ++bw)"));
                e.l(format!("const long long tile = ({base}) + (long long)bh * {stride_h} + (long long)bw * {stride_w};"));
                e.open(format!("for (int el = tid; el < {te}; el += NTHREAD)"));
                e.l(format!("t{}[tile * {te} + el] = (long long)ring_{r}[rslot_{r} * PAGE_{r} + (bh * {block_w} + bw) * {te} + el];", tensor.idx()));
                e.close();
                e.close();
                e.close();
            }
            ChoreoOp::AcquireAcc { .. } => {
                e.open("for (int q = 0; q < PERW; ++q)");
                e.l("nvcuda::wmma::fill_fragment(accf[q], 0);");
                e.close();
            }
            ChoreoOp::ReleaseAcc => {}
            ChoreoOp::Compute { op, ins, acc_reload } => match op {
                ComputeOp::MatmulTiles { block, sub_blocks, .. } => {
                    if acc_reload.is_some() {
                        return Err(SpatialLowerError::Unsupported("an accumulator spill on the GPU (the fragments never leave the registers)"));
                    }
                    let BlockShape { osh, osw, ibw } = *block;
                    let per_site_n = sub_blocks.1 * osw;
                    let (ra, rb) = (ring(ctx, ins[0]).idx(), ring(ctx, ins[1]).idx());
                    let (te, tc) = (ctx.te, ctx.tc);
                    // A single sub-block leaves no loop, so its index is a constant.
                    let sh = if scope.contains(&LV_SUB_H) { "sh" } else { "0" };
                    let sw = if scope.contains(&LV_SUB_W) { "sw" } else { "0" };
                    match ctx.lane_grid {
                        // A warp owns a RECTANGLE: load `rm` A-fragments and `rn` B-fragments once
                        // per K step, then do every product with them. This is the reuse a GEMM is.
                        Some((wm, wn)) => {
                            let (rm, rn) = (osh / wm, osw / wn);
                            e.l(format!("const int wi = warp / {wn}, wj = warp % {wn};"));
                            e.open(format!("for (int kk = 0; kk < {ibw}; ++kk)"));
                            e.l(format!("nvcuda::wmma::fragment<nvcuda::wmma::matrix_a, 16, 16, 16, OPT, nvcuda::wmma::row_major> af[{rm}];"));
                            e.l(format!("nvcuda::wmma::fragment<nvcuda::wmma::matrix_b, 16, 16, 16, OPT, nvcuda::wmma::row_major> bf[{rn}];"));
                            e.open(format!("for (int i = 0; i < {rm}; ++i)"));
                            e.l(format!("nvcuda::wmma::load_matrix_sync(af[i], &ring_{ra}[rslot_{ra} * PAGE_{ra} + (({sh} * {osh} + wi * {rm} + i) * {ibw} + kk) * {te}], {tc});"));
                            e.close();
                            e.open(format!("for (int j = 0; j < {rn}; ++j)"));
                            e.l(format!("nvcuda::wmma::load_matrix_sync(bf[j], &ring_{rb}[rslot_{rb} * PAGE_{rb} + (kk * {per_site_n} + {sw} * {osw} + wj * {rn} + j) * {te}], {tc});"));
                            e.close();
                            e.open(format!("for (int i = 0; i < {rm}; ++i)"));
                            e.open(format!("for (int j = 0; j < {rn}; ++j)"));
                            e.l(format!("nvcuda::wmma::mma_sync(accf[i * {rn} + j], af[i], bf[j], accf[i * {rn} + j]);"));
                            e.close();
                            e.close();
                            e.close();
                        }
                        // No even split — correct and slow rather than wrong.
                        None => {
                            e.open("for (int q = 0; q < PERW; ++q)");
                            e.l(format!("const int ot = warp + q * {};", ctx.lanes));
                            e.l(format!("if (ot >= {}) break;", osh * osw));
                            e.l(format!("const int oi = ot / {osw}, oj = ot % {osw};"));
                            e.open(format!("for (int kk = 0; kk < {ibw}; ++kk)"));
                            e.l("nvcuda::wmma::fragment<nvcuda::wmma::matrix_a, 16, 16, 16, OPT, nvcuda::wmma::row_major> af;");
                            e.l("nvcuda::wmma::fragment<nvcuda::wmma::matrix_b, 16, 16, 16, OPT, nvcuda::wmma::row_major> bf;");
                            e.l(format!("nvcuda::wmma::load_matrix_sync(af, &ring_{ra}[rslot_{ra} * PAGE_{ra} + (({sh} * {osh} + oi) * {ibw} + kk) * {te}], {tc});"));
                            e.l(format!("nvcuda::wmma::load_matrix_sync(bf, &ring_{rb}[rslot_{rb} * PAGE_{rb} + (kk * {per_site_n} + {sw} * {osw} + oj) * {te}], {tc});"));
                            e.l("nvcuda::wmma::mma_sync(accf[q], af, bf, accf[q]);");
                            e.close();
                            e.close();
                        }
                    }
                }
                other => return Err(SpatialLowerError::Unsupported(compute_name(other))),
            },
            ChoreoOp::PackAcc { to, .. } if ctx.fused_out == Some(ring(ctx, *to)) => {}
            ChoreoOp::PackAcc { to, tiles, accumulate, .. } => {
                if *accumulate {
                    return Err(SpatialLowerError::Unsupported("an accumulating epilogue on the GPU (the fragments never leave the registers)"));
                }
                let r = ring(ctx, *to).idx();
                let te = ctx.te;
                // The store must use the SAME warp-to-tile map the multiply used, or a warp writes
                // its accumulators into another warp's tiles — which is a wrong answer, not a slow
                // one. `ctx.acc_shape` carries the block's `(osh, osw)` so both sites read one map.
                match ctx.acc_shape.and_then(|(osh, osw)| ctx.lane_grid.map(|g| (osh, osw, g))) {
                    Some((_osh, osw, (wn_m, wn_n))) if *tiles == _osh * osw => {
                        let (rm, rn) = (_osh / wn_m, osw / wn_n);
                        e.l(format!("const int pwi = warp / {wn_n}, pwj = warp % {wn_n};"));
                        e.open(format!("for (int i = 0; i < {rm}; ++i)"));
                        e.open(format!("for (int j = 0; j < {rn}; ++j)"));
                        e.l(format!("nvcuda::wmma::store_matrix_sync(&ring_{r}[wslot_{r} * PAGE_{r} + ((pwi * {rm} + i) * {osw} + pwj * {rn} + j) * {te}], accf[i * {rn} + j], {}, nvcuda::wmma::mem_row_major);", ctx.tc));
                        e.close();
                        e.close();
                    }
                    _ => {
                        e.open("for (int q = 0; q < PERW; ++q)");
                        e.l(format!("const int ot = warp + q * {};", ctx.lanes));
                        e.l(format!("if (ot >= {tiles}) break;"));
                        e.l(format!("nvcuda::wmma::store_matrix_sync(&ring_{r}[wslot_{r} * PAGE_{r} + ot * {te}], accf[q], {}, nvcuda::wmma::mem_row_major);", ctx.tc));
                        e.close();
                    }
                }
            }
            ChoreoOp::SemSet { .. } | ChoreoOp::SemInc { .. } | ChoreoOp::SemWait { .. } | ChoreoOp::McastSetValid { .. } | ChoreoOp::McastSend { .. } => {
                return Err(SpatialLowerError::Unsupported("a multicast handshake on sm_86 (no cluster-wide multicast)"))
            }
            ChoreoOp::Barrier { .. } => return Err(SpatialLowerError::Unsupported("a group barrier on the GPU lowering")),
            ChoreoOp::Loop { count: LoopCount::Trip { .. }, .. } => {
                return Err(SpatialLowerError::Unsupported("a loop whose count the program reads from its data (a trip) — the persistent kernel's loop bounds are fixed when it is printed"))
            }
            ChoreoOp::Loop { var, count, body } => {
                let v = var_name(*var);
                let bound = if *var == LV_ITEM { "n_items".to_string() } else { count.bound().to_string() };
                // ★ SOFTWARE-PIPELINE THE K LOOP, or the double buffer buys nothing.
                //
                // The choreography reads as acquire → copy → commit → WAIT → compute, and printed
                // literally that is what the kernel does: it issues the loads for this K block and
                // immediately waits for them, so a ~500-cycle global latency is fully exposed every
                // block and `DEPTH = 2` is two slots that are never both in use. The whole point of
                // an async copy is to have the NEXT block's loads in flight while this block's
                // multiply runs.
                //
                // So the producer half is ROTATED one iteration ahead: its copies are issued for
                // block 0 before the loop, and inside the loop the copies for block i+1 are issued
                // before the wait for block i. The address expressions are functions of the loop
                // variable, so a shadowed binding is all the rotation needs — nothing about the
                // choreography changes, only when its producer half is printed.
                //
                // Block i+1's commits are then in flight while block i is awaited, so the consumer half
                // waits for all but those. The last block has no successor to prefetch; it commits as many
                // empty groups instead, which keeps that count exact on every iteration.
                if let Some((prod, cons)) = rotation(ctx, body) {
                    let n = count.bound();
                    let commits = commits_of(ctx, &prod);
                    e.open("");
                    e.l(format!("const int {v} = 0;"));
                    emit_ops(ctx, e, &prod, &mut scope.clone(), 0)?;
                    e.close();
                    e.open(format!("for (int {v} = 0; {v} < {bound}; ++{v})"));
                    scope.insert(*var);
                    e.open(format!("if ({v} + 1 < {n})"));
                    e.l(format!("const int {v}_pf = {v} + 1;"));
                    e.open("");
                    e.l(format!("const int {v} = {v}_pf;"));
                    emit_ops(ctx, e, &prod, &mut scope.clone(), 0)?;
                    e.close();
                    e.close();
                    if commits > 0 {
                        e.open("else");
                        for _ in 0..commits {
                            e.l("__pipeline_commit();");
                        }
                        e.close();
                    }
                    emit_ops(ctx, e, &cons, scope, commits)?;
                    e.close();
                    scope.remove(var);
                    continue;
                }
                e.open(format!("for (int {v} = 0; {v} < {bound}; ++{v})"));
                if *var == LV_ITEM {
                    e.l("const int item_id = ITEM_LIST[item_off + item];");
                    e.l(format!("const int oy = item_id / {}, ox = item_id % {};", ctx.grid_nx.max(1), ctx.grid_nx.max(1)));
                }
                scope.insert(*var);
                emit_ops(ctx, e, body, scope, 0)?;
                scope.remove(var);
                e.close();
            }
        }
    }
    Ok(())
}

/// The producer and consumer halves of a loop body the printer rotates ([`pipelined_halves`]), when every ring
/// the producer fills can hold the two blocks a rotation keeps resident at once — the one being read and the
/// one being prefetched. A shallower ring prints literally, without the prefetch.
fn rotation(ctx: &Ctx, body: &[TaggedOp]) -> Option<(Vec<TaggedOp>, Vec<TaggedOp>)> {
    let (prod, cons) = pipelined_halves(body)?;
    let mut pages: BTreeMap<BufferId, usize> = BTreeMap::new();
    for t in &prod {
        if let ChoreoOp::Reserve { buf, .. } = &t.op {
            *pages.entry(ring(ctx, *buf)).or_default() += 1;
        }
    }
    let fits = pages.iter().all(|(r, n)| {
        ctx.fused_out == Some(*r) || ctx.rings.get(r).is_some_and(|(depth, ..)| 2 * n <= *depth)
    });
    fits.then_some((prod, cons))
}

/// The commit groups `ops` prints: one per Push on a ring the kernel keeps.
fn commits_of(ctx: &Ctx, ops: &[TaggedOp]) -> usize {
    ops.iter().filter(|t| matches!(&t.op, ChoreoOp::Push { buf, .. } if ctx.fused_out != Some(ring(ctx, *buf)))).count()
}

/// ★ NO RING IS FILLED PAST ITS DEPTH. A ring has no run-time flow control — a `Reserve` names the next slot
/// and trusts it is free — so the printed schedule must never hold more pages of a ring than it has slots,
/// or a copy lands on a page a consumer has not read yet. That is a wrong answer, not a hang, so it is
/// proved here, on the same rotation the printer takes, before a line is printed.
///
/// A slot is claimed at its `Reserve` and freed at its `Pop`, one page per op — the printer's slot counters
/// step by one, so an op that moves several pages at once is refused rather than mis-indexed. A loop that runs
/// more than once must leave every ring as it found it, so one iteration from the entry state bounds them all;
/// a rotated loop holds its prefetched block on top of the one being read.
fn ring_occupancy(
    ctx: &Ctx,
    ops: &[TaggedOp],
    held: &mut BTreeMap<BufferId, usize>,
) -> Result<(), SpatialLowerError> {
    for t in ops {
        match &t.op {
            ChoreoOp::Reserve { buf, pages }
            | ChoreoOp::Push { buf, pages }
            | ChoreoOp::Wait { buf, pages }
            | ChoreoOp::Pop { buf, pages }
                if ctx.fused_out != Some(ring(ctx, *buf)) && *pages != 1 =>
            {
                return Err(SpatialLowerError::Unsupported("a FIFO op that moves more than one page at once (the persistent kernel's ring slots are one page each)"));
            }
            ChoreoOp::Reserve { buf, .. } if ctx.fused_out != Some(ring(ctx, *buf)) => {
                let r = ring(ctx, *buf);
                let depth = ctx.rings.get(&r).map(|(d, ..)| *d).unwrap_or(0);
                let n = held.entry(r).or_default();
                *n += 1;
                if *n > depth {
                    return Err(SpatialLowerError::TooBig {
                        what: "pages of a ring held before its consumer drains them",
                        need: *n,
                        have: depth,
                    });
                }
            }
            ChoreoOp::Pop { buf, .. } if ctx.fused_out != Some(ring(ctx, *buf)) => {
                let r = ring(ctx, *buf);
                if let Some(n) = held.get_mut(&r) {
                    *n -= 1;
                    if *n == 0 {
                        held.remove(&r);
                    }
                }
            }
            ChoreoOp::Loop { count, body, .. } => {
                let entry = held.clone();
                let once = matches!(count, LoopCount::Const(1));
                match rotation(ctx, body) {
                    Some((prod, cons)) if !once => {
                        ring_occupancy(ctx, &prod, held)?;
                        let steady = held.clone();
                        ring_occupancy(ctx, &prod, held)?;
                        ring_occupancy(ctx, &cons, held)?;
                        if *held != steady {
                            return Err(SpatialLowerError::Unsupported(
                                "a rotated loop whose iterations do not drain what they fill",
                            ));
                        }
                        ring_occupancy(ctx, &cons, held)?;
                    }
                    _ => ring_occupancy(ctx, body, held)?,
                }
                if !once && *held != entry {
                    return Err(SpatialLowerError::Unsupported("a loop whose iterations do not drain what they fill (a ring with no flow control would overrun)"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn compute_name(op: &ComputeOp) -> &'static str {
    match op {
        ComputeOp::Generic { .. } => {
            "the general compute form on the GPU lowering (S2 gives it an arm)"
        }
        ComputeOp::Requant { .. } => "a requant stage on the GPU lowering",
        ComputeOp::Bias { .. } => "a bias stage on the GPU lowering",
        ComputeOp::Relu => "a relu stage on the GPU lowering",
        ComputeOp::Scalar(_) => "a scalar stage on the GPU lowering",
        ComputeOp::MatmulTiles { .. } => "a matmul stage",
    }
}

/// One CUDA block runs a whole site, so the site's engine programs (which run concurrently on separate
/// engines everywhere else) become ONE sequential program. The merge is the choreography's own dataflow
/// order: loop nests are aligned by variable and merged recursively; inside a segment an engine's head op is
/// emitted unless it waits on a buffer another engine has not pushed yet. That is exactly the interleaving
/// `eval_choreography` produces, with no reordering across a buffer's producer and consumer.
fn merge_programs(progs: &[Vec<TaggedOp>], site: Site) -> Result<Vec<TaggedOp>, SpatialLowerError> {
    // Split each program into (ops before the next loop, the loop, the rest).
    fn next_loop(ops: &[TaggedOp]) -> Option<usize> {
        ops.iter()
            .position(|t| matches!(t.op, ChoreoOp::Loop { .. }))
    }
    let mut heads: Vec<&[TaggedOp]> = progs.iter().map(|v| v.as_slice()).collect();
    let mut out = Vec::new();
    loop {
        // The prefix segment: everything before each program's next loop.
        // Where to cut each program before ordering. A loop that CO-ITERATES with another program's loop
        // (same variable) is a merge point and must be cut at. A loop nobody co-iterates with is just an op,
        // and cutting at it strands any other program waiting on what that loop fills — the block then cannot
        // be serialised at all, though a perfectly good order exists.
        //
        // Absorbing such a loop is only safe when it runs ONCE: emitting a longer loop whole would push more
        // pages than the ring holds before any consumer drains it, trading a compile-time refusal for a
        // runtime hang. A multi-trip loop with no co-iterating partner keeps the refusal, which is honest.
        let firsts: Vec<Option<(usize, LoopVar, LoopCount)>> = heads
            .iter()
            .map(|h| {
                next_loop(h).and_then(|i| match &h[i].op {
                    ChoreoOp::Loop { var, count, .. } => Some((i, *var, *count)),
                    _ => None,
                })
            })
            .collect();
        let shared: std::collections::BTreeSet<LoopVar> = firsts
            .iter()
            .flatten()
            .filter(|(_, v, _)| firsts.iter().flatten().filter(|(_, w, _)| w == v).count() > 1)
            .map(|(_, v, _)| *v)
            .collect();
        let cuts: Vec<usize> = heads
            .iter()
            .zip(&firsts)
            .map(|(h, f)| match f {
                Some((i, v, c)) if shared.contains(v) || !matches!(c, LoopCount::Const(1)) => *i,
                Some((i, _, _)) => next_loop(&h[i + 1..]).map(|j| i + 1 + j).unwrap_or(h.len()),
                None => h.len(),
            })
            .collect();
        let segs: Vec<Vec<TaggedOp>> = heads
            .iter()
            .zip(&cuts)
            .map(|(h, c)| h[..*c].to_vec())
            .collect();
        // The tails are what each program still has to run after this segment — including the loops. A wait
        // must be held back for a push that lives in one of them, not just for one in the segment itself.
        let tails: Vec<&[TaggedOp]> = heads.iter().zip(&cuts).map(|(h, c)| &h[*c..]).collect();
        out.extend(order_segment(&segs, &tails, site)?);
        // Every program that still has ops must now be at a loop; merge the loops that share a variable.
        let loops: Vec<Option<(LoopVar, LoopCount, Vec<TaggedOp>, StageId)>> = heads
            .iter()
            .zip(&cuts)
            .map(|(h, c)| match h.get(*c) {
                Some(TaggedOp {
                    stage,
                    op: ChoreoOp::Loop { var, count, body },
                }) => Some((*var, *count, body.clone(), *stage)),
                _ => None,
            })
            .collect();
        if loops.iter().all(|l| l.is_none()) {
            return Ok(out);
        }
        let var = loops.iter().flatten().map(|(v, ..)| *v).next().unwrap();
        if loops.iter().flatten().any(|(v, ..)| *v != var) {
            return Err(SpatialLowerError::Unsupported(
                "engine programs whose loop nests do not align (the GPU runs one block per site)",
            ));
        }
        let (count, stage) = loops
            .iter()
            .flatten()
            .map(|(_, c, _, s)| (*c, *s))
            .next()
            .unwrap();
        let bodies: Vec<Vec<TaggedOp>> = loops
            .iter()
            .flatten()
            .map(|(_, _, b, _)| b.clone())
            .collect();
        let body = merge_programs(&bodies, site)?;
        out.push(TaggedOp {
            stage,
            op: ChoreoOp::Loop { var, count, body },
        });
        heads = heads
            .iter()
            .zip(&cuts)
            .zip(&loops)
            .map(|((h, c), l)| {
                if l.is_some() {
                    &h[*c + 1..]
                } else {
                    &h[h.len()..]
                }
            })
            .collect();
    }
}

/// Emit a segment's ops so no consumer runs before the producer that fills its buffer.
/// Does this op sequence still push `buf`, at any loop depth?
///
/// The scan has to descend into loop bodies. A producer whose push sits inside a loop is exactly the shape a
/// GEMM's operand feed takes, and a scan that only looks at the current straight-line segment cannot see it —
/// so the consumer's wait is emitted first, and a block that serialises both roles waits for data it has not
/// produced yet. The kernel then runs forever rather than deadlocking visibly, because a persistent kernel
/// spinning on a pipeline looks exactly like one doing work.
fn pushes_buf(ops: &[TaggedOp], buf: BufferId) -> bool {
    ops.iter().any(|t| match &t.op {
        ChoreoOp::Push { buf: b, .. } => *b == buf,
        ChoreoOp::Loop { body, .. } => pushes_buf(body, buf),
        _ => false,
    })
}

/// Does this op sequence still raise semaphore `sem` at `site` — an increment or a set — at any loop depth?
fn raises_sem(ops: &[TaggedOp], sem: SemId, site: Site) -> bool {
    ops.iter().any(|t| match &t.op {
        ChoreoOp::SemInc { sem: s, at, .. } => *s == sem && *at == site,
        ChoreoOp::SemSet { sem: s, .. } => *s == sem,
        ChoreoOp::Loop { body, .. } => raises_sem(body, sem, site),
        _ => false,
    })
}

/// Buffers this program fills, at any loop depth.
fn collect_pushes(ops: &[TaggedOp], out: &mut std::collections::BTreeSet<usize>) {
    for t in ops {
        match &t.op {
            ChoreoOp::Push { buf, .. } => {
                out.insert(buf.idx());
            }
            ChoreoOp::Loop { body, .. } => collect_pushes(body, out),
            _ => {}
        }
    }
}

/// Buffers this program drains, at any loop depth.
fn collect_waits(ops: &[TaggedOp], out: &mut std::collections::BTreeSet<usize>) {
    for t in ops {
        match &t.op {
            ChoreoOp::Wait { buf, .. } => {
                out.insert(buf.idx());
            }
            ChoreoOp::Loop { body, .. } => collect_waits(body, out),
            _ => {}
        }
    }
}

fn order_segment(
    segs: &[Vec<TaggedOp>],
    tails: &[&[TaggedOp]],
    site: Site,
) -> Result<Vec<TaggedOp>, SpatialLowerError> {
    let mut idx = vec![0usize; segs.len()];
    let mut out = Vec::new();
    let pending_push =
        |segs: &[Vec<TaggedOp>], idx: &[usize], buf: BufferId, skip: usize| -> bool {
            segs.iter().enumerate().any(|(i, s)| {
                i != skip
                    && (pushes_buf(&s[idx[i]..], buf)
                        || tails.get(i).map(|t| pushes_buf(t, buf)).unwrap_or(false))
            })
        };
    // A semaphore wait is a wait on the engine of this site that still raises it; a raise from another site is another
    // block's, which runs beside this one.
    let pending_raise = |segs: &[Vec<TaggedOp>], idx: &[usize], sem: SemId, skip: usize| -> bool {
        segs.iter().enumerate().any(|(i, s)| {
            i != skip
                && (raises_sem(&s[idx[i]..], sem, site) || tails.get(i).map(|t| raises_sem(t, sem, site)).unwrap_or(false))
        })
    };
    loop {
        let mut progressed = false;
        for i in 0..segs.len() {
            while idx[i] < segs[i].len() {
                let blocked = match &segs[i][idx[i]].op {
                    ChoreoOp::Wait { buf, .. } => pending_push(segs, &idx, *buf, i),
                    ChoreoOp::SemWait { sem, .. } => pending_raise(segs, &idx, *sem, i),
                    _ => false,
                };
                if blocked {
                    break;
                }
                out.push(segs[i][idx[i]].clone());
                idx[i] += 1;
                progressed = true;
            }
        }
        if idx.iter().zip(segs).all(|(k, s)| *k == s.len()) {
            return Ok(out);
        }
        if !progressed {
            return Err(SpatialLowerError::Unsupported(
                "engine programs the block cannot serialize without deadlock",
            ));
        }
    }
}

/// The largest per-warp accumulator fragment count the schedule needs.
/// ★ HOW THE BLOCK'S OUTPUT TILES ARE SPLIT ACROSS ITS WARPS — the single decision that makes the
/// difference between a GEMM and a fragment-loading loop.
///
/// A warp that owns a RECTANGLE of `(rm × rn)` output tiles loads `rm` A-fragments and `rn`
/// B-fragments per K step and does `rm · rn` multiplies with them. A warp that owns a strided SET
/// of tiles reuses nothing: it loads BOTH fragments for every tile, every step. At 8×8 tiles over
/// 4 warps that is 32 loads per step against 8 — the reason the emitted kernel measured 1,267 GOPS
/// against the knob lattice's 16,500.
///
/// The split is chosen to make the warp's rectangle as SQUARE as possible, because `rm + rn` is
/// what is loaded and `rm · rn` is what is computed: for a fixed area the squarest rectangle has
/// the smallest perimeter, which is exactly the arithmetic-intensity argument. `None` when no
/// split divides the block evenly — then the caller keeps the strided form, which is correct and
/// slow rather than wrong.
/// Split a loop body into its PRODUCER half (reserve/copy/commit — the async loads) and its
/// CONSUMER half (wait/compute/pop), when the body really has that shape: every producer op first,
/// then every consumer op, and at least one of each.
///
/// `None` when the body interleaves them, which is a body whose producer cannot be rotated ahead
/// without reordering something that is not a load. The caller then prints it literally — correct,
/// and without the prefetch.
fn pipelined_halves(body: &[TaggedOp]) -> Option<(Vec<TaggedOp>, Vec<TaggedOp>)> {
    let is_producer = |op: &ChoreoOp| {
        matches!(
            op,
            ChoreoOp::Reserve { .. }
                | ChoreoOp::Push { .. }
                | ChoreoOp::ReadTiles { .. }
                | ChoreoOp::ReadBarrier { .. }
        )
    };
    let is_consumer = |op: &ChoreoOp| {
        matches!(
            op,
            ChoreoOp::Wait { .. } | ChoreoOp::Pop { .. } | ChoreoOp::Compute { .. }
        )
    };
    let split = body.iter().position(|t| !is_producer(&t.op))?;
    if split == 0 || split == body.len() {
        return None;
    }
    if !body[split..].iter().all(|t| is_consumer(&t.op)) {
        return None;
    }
    Some((body[..split].to_vec(), body[split..].to_vec()))
}

fn per_warp_tiles(p: &SpatialProgram, lanes: usize) -> usize {
    let mut max = 0usize;
    if let Some(sch) = &p.schedule {
        for prog in &sch.programs {
            walk_ops(
                &prog.ops.iter().map(|t| t.op.clone()).collect::<Vec<_>>(),
                1,
                &mut |op, _| {
                    if let ChoreoOp::Compute {
                        op: ComputeOp::MatmulTiles { block, .. },
                        ..
                    } = op
                    {
                        max = max.max(block.osh * block.osw);
                    }
                    if let ChoreoOp::PackAcc { tiles, .. } = op {
                        max = max.max(*tiles);
                    }
                },
            );
        }
    }
    max.div_ceil(lanes).max(1)
}

impl SpatialBackend for CudaPersistentBackend {
    fn name(&self) -> &str {
        self.target.name
    }
    fn target(&self) -> &SpatialTarget {
        &self.target
    }
    fn lower(&self, p: &SpatialProgram) -> Result<SpatialArtifact, SpatialLowerError> {
        require_verified(p)?;
        // A program the tile GEMM kernel cannot hold — any op but a tile matmul, plans per section, a data-dependent
        // address, a handshake between sites — prints as the general program.
        if general::wants_general(p) {
            return general::lower(p, &self.target).map(SpatialArtifact::Cuda);
        }
        require_one_plan(p)?;
        if p.has_indirect_access() {
            return Err(SpatialLowerError::Unsupported("an indirect (data-dependent) tile or element access on the GPU lowering (gathers are B-P5)"));
        }
        let pl = p
            .placement
            .as_ref()
            .ok_or(SpatialLowerError::Unsupported("a placed program"))?;
        if let Some(g) = pl.groups.first() {
            return Err(SpatialLowerError::GroupSpansSites { group: g.id });
        }
        if !p.semaphores.is_empty() {
            return Err(SpatialLowerError::Unsupported(
                "a semaphore handshake on sm_86 (no cluster-wide multicast)",
            ));
        }
        let sched = p
            .schedule
            .as_ref()
            .ok_or(SpatialLowerError::Unsupported("a scheduled program"))?;
        let plan = p
            .work
            .plan
            .ok_or(SpatialLowerError::Unsupported("a tiled program"))?;
        if p.tile.rows != 16 || p.tile.cols != 16 {
            return Err(SpatialLowerError::Unsupported(
                "a tile shape other than WMMA's 16×16",
            ));
        }
        // The operand precision the matmul stages agree on.
        let operand = p
            .stages
            .iter()
            .find_map(|s| match &s.kind {
                StageKind::Compute(ComputeOp::MatmulTiles { operand, .. }) => Some(*operand),
                _ => None,
            })
            .ok_or(SpatialLowerError::Unsupported(
                "a program without a tile matmul",
            ))?;
        let (opt, acct, _) =
            wmma_types(operand).ok_or(SpatialLowerError::Precision { needed: operand })?;
        // int8 fragments accumulate in int32: the reduction must not overflow it, else the GPU would disagree
        // with the IR's 64-bit accumulate.
        if operand == Precision::Int8 {
            let k = p.work.k_tiles as i128 * p.tile.cols as i128;
            if k * 127 * 127 >= i32::MAX as i128 {
                return Err(SpatialLowerError::TooBig {
                    what: "an int8 reduction that overflows the int32 fragment accumulator",
                    need: (k * 127 * 127) as usize,
                    have: i32::MAX as usize,
                });
            }
        }
        let mut rings: BTreeMap<BufferId, (usize, usize, &'static str)> = BTreeMap::new();
        for b in &p.buffers {
            let (depth, page) = match b.kind {
                BufferKind::Fifo {
                    depth_pages,
                    page_tiles,
                }
                | BufferKind::Interm {
                    depth_pages,
                    page_tiles,
                } => (depth_pages, page_tiles * p.tile.elems()),
                BufferKind::Accumulator { .. } => continue,
            };
            // Inputs carry operands, outputs carry the accumulator's type.
            let ty = if p
                .stages
                .iter()
                .any(|s| s.outs.contains(&b.id) && matches!(s.kind, StageKind::Compute(_)))
            {
                acct
            } else {
                opt
            };
            rings.insert(b.origin.unwrap_or(b.id), (depth, page, ty));
        }
        let acc_shape = Some((plan.block.osh, plan.block.osw));
        // The epilogue fuses only where the accumulator is register-resident (so there is nothing
        // to drain), the warp grid divides the block (so each warp owns a rectangle it can write
        // itself), and the output really is the matmul's own output buffer.
        let lane_grid = self
            .target
            .accumulator_lane_grid(plan.block.osh, plan.block.osw);
        let fused_out = if self.target.accumulator_in_registers() && lane_grid.is_some() {
            p.output_tensor()
                .and_then(|_| {
                    p.stages.iter().find(|s| {
                        matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. }))
                    })
                })
                .and_then(|s| s.outs.first().copied())
                .map(|b| ring_of(p, b))
        } else {
            None
        };
        let ctx = Ctx {
            p,
            te: p.tile.elems(),
            tr: p.tile.rows,
            tc: p.tile.cols,
            grid_nx: plan.grid.1,
            rings,
            operand,
            acc_t: acct,
            acc_shape,
            fused_out,
            lane_grid,
            lanes: nwarp(&self.target),
        };
        // One engine program per site, all structurally equal (the same static schedule): print the first.
        let sites: Vec<Site> = pl.items_of_site.iter().map(|(s, _)| *s).collect();
        // Producers before consumers. Serialising a site's engine programs onto ONE block makes their order a
        // correctness property: a program that waits on a buffer another program fills must be emitted after
        // it, or the block waits for data it has not produced. Scheduler order is arbitrary with respect to
        // that, so sort by dataflow first and let the merge interleave within it.
        let mut site_progs: Vec<Vec<TaggedOp>> = sched
            .programs
            .iter()
            .filter(|pr| pr.site == sites[0])
            .map(|pr| pr.ops.clone())
            .collect();
        let fills: Vec<std::collections::BTreeSet<usize>> = site_progs
            .iter()
            .map(|ops| {
                let mut f = Default::default();
                collect_pushes(ops, &mut f);
                f
            })
            .collect();
        let drains: Vec<std::collections::BTreeSet<usize>> = site_progs
            .iter()
            .map(|ops| {
                let mut d = Default::default();
                collect_waits(ops, &mut d);
                d
            })
            .collect();
        let mut order: Vec<usize> = (0..site_progs.len()).collect();
        order.sort_by(|a, b| {
            let a_feeds_b = !fills[*a].is_disjoint(&drains[*b]);
            let b_feeds_a = !fills[*b].is_disjoint(&drains[*a]);
            match (a_feeds_b, b_feeds_a) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.cmp(b),
            }
        });
        site_progs = order.into_iter().map(|i| site_progs[i].clone()).collect();
        let ops = merge_programs(&site_progs, sites[0])?;
        ring_occupancy(&ctx, &ops, &mut BTreeMap::new())?;
        let perw = per_warp_tiles(p, nwarp(&self.target));
        let smem: usize = ctx
            .rings
            .values()
            .map(|(d, pg, ty)| d * pg * c_type_bytes(ty))
            .sum();
        // ★ THE EPILOGUE SCRATCH LIVES IN THE DYNAMIC BLOCK, AND THE RINGS ONLY MOVE IF THEY MUST.
        //
        // ptxas caps STATIC `__shared__` at 48 KiB whatever `cudaFuncSetAttribute` says, and the cap covers
        // every static declaration. The rings are the only ones: the epilogue scratch is 8 KiB touched once per
        // epilogue, so it always goes dynamic, and the rings follow only when they alone exceed the cap —
        // sending them dynamic costs 12% (measured, 5,743 GOPS static against 5,056 dynamic at the same bytes
        // and the same two blocks per SM).
        let epi_bytes = if ctx.fused_out.is_some() {
            ctx.lanes * ctx.te * c_type_bytes(acct)
        } else {
            0
        };
        let rings_dynamic = smem > 48 * 1024;
        let dyn_bytes = align16(epi_bytes) + if rings_dynamic { align16(smem) } else { 0 };
        let dynamic = dyn_bytes > 48 * 1024;
        let threads = nwarp(&self.target) * WARP;
        let regs_est = perw * 8 + 48;
        // ONE definition of occupancy, shared with the planner — `tile::best_candidate_for` ranks plans by
        // it, so a copy here would let the emitter and the search disagree about the machine.
        let blocks_per_sm =
            self.target
                .resident_sites_per_unit(smem + epi_bytes, plan.block.osh, plan.block.osw);
        let budget = PersistentBudget {
            smem_bytes: smem,
            dynamic_opt_in: dynamic,
            rings_dynamic,
            dyn_bytes,
            regs_est,
            blocks_per_sm,
            threads,
        };
        let program_cu = print_program(&ctx, &ops, &sites, perw, &budget, false)?;
        let bench_cu = print_program(&ctx, &ops, &sites, perw, &budget, true)?;
        Ok(SpatialArtifact::Cuda(CudaArtifact {
            program_cu,
            bench_cu,
            budget,
            census: Census::of(p),
            general: None,
        }))
    }
}

fn c_type_bytes(ty: &str) -> usize {
    match ty {
        "signed char" => 1,
        "__nv_bfloat16" | "__half" => 2,
        _ => 4,
    }
}

fn print_program(
    ctx: &Ctx,
    ops: &[TaggedOp],
    sites: &[Site],
    perw: usize,
    budget: &PersistentBudget,
    bench: bool,
) -> Result<String, SpatialLowerError> {
    let p = ctx.p;
    let pl = p.placement.as_ref().unwrap();
    let mut e = Emit {
        s: String::new(),
        ind: 0,
    };
    e.l(format!(
        "// {} — a persistent CUDA kernel printed from the spatial IR; each block is one IR site.",
        p.name
    ));
    e.l("#include <cuda_pipeline.h>");
    e.l("#include <mma.h>");
    e.l("#include <cuda_bf16.h>");
    e.l("#include <cuda_fp16.h>");
    e.l("#include <cstdio>");
    e.l("#include <cstdlib>");
    e.l("#include <vector>");
    e.l("#include <cstring>");
    e.l("");
    e.l(format!(
        "typedef {} OPT;",
        wmma_types(ctx.operand).unwrap().0
    ));
    e.l(format!("typedef {} ACCT;", ctx.acc_t));
    e.l(format!("#define NTHREAD {}", ctx.lanes * WARP));
    e.l(format!("#define PERW {perw}"));
    // The epilogue scratch is first in the dynamic block; the rings follow only when they had to leave
    // static memory. Every region starts on a 16-byte boundary because wmma loads want one.
    let mut dyn_off = 0usize;
    if ctx.fused_out.is_some() {
        e.l(format!("#define SMEM_OFF_EPI {dyn_off}"));
        dyn_off += align16(ctx.lanes * ctx.te * c_type_bytes(ctx.acc_t));
    }
    for (b, (depth, page, ty)) in &ctx.rings {
        e.l(format!("#define DEPTH_{} {depth}", b.idx()));
        e.l(format!("#define PAGE_{} {page}", b.idx()));
        if budget.rings_dynamic {
            dyn_off = (dyn_off + 15) & !15;
            e.l(format!("#define SMEM_OFF_{} {dyn_off}", b.idx()));
            dyn_off += depth * page * c_type_bytes(ty);
        }
    }
    if dyn_off > 0 {
        e.l(format!("#define DYN_SMEM {}", (dyn_off + 15) & !15));
    }
    // ★ THE WORK LIST IS THE PLACEMENT ITSELF, item by item — not a start plus a count.
    //
    // `item_start + item` assumes each site's items are a CONTIGUOUS run. They are whenever the
    // block grid fits the mesh; when it does not, `place` wraps items round-robin and a site owns
    // a STRIDED set. Reading a strided set as a contiguous run made site `s` process items
    // `s..s+n` — so sites overlapped, most items were never computed at all, and the output tiles
    // nobody claimed stayed ZERO. (Measured at 256³: 8 sites each claiming 8 items covered 15 of
    // the 64, and 50,175 of 65,536 output elements were wrong.)
    //
    // A flat list indexed by a per-site offset is exact for ANY placement, which is what "the
    // emitted program is a function of the IR" has to mean: the lowering reads the placement
    // rather than assuming its shape.
    let mut flat: Vec<String> = Vec::new();
    let mut offs: Vec<String> = Vec::new();
    let mut counts: Vec<String> = Vec::new();
    for s in sites.iter() {
        offs.push(flat.len().to_string());
        let items = pl.items_at(*s);
        counts.push(items.len().to_string());
        flat.extend(items.iter().map(|i| i.idx().to_string()));
    }
    if flat.is_empty() {
        flat.push("0".to_string());
    }
    e.l(format!(
        "__device__ __constant__ int ITEM_LIST[{}] = {{{}}};",
        flat.len(),
        flat.join(", ")
    ));
    e.l(format!(
        "__device__ __constant__ int ITEM_OFF[{}] = {{{}}};",
        offs.len(),
        offs.join(", ")
    ));
    e.l(format!(
        "__device__ __constant__ int ITEM_COUNT[{}] = {{{}}};",
        sites.len(),
        counts.join(", ")
    ));
    e.l("");
    // Kernel signature: every tensor as a device pointer, in id order.
    let mut params: Vec<String> = Vec::new();
    for t in &p.tensors {
        let ty = if t.role == TensorRole::Output {
            "long long".to_string()
        } else {
            wmma_types(ctx.operand).unwrap().0.to_string()
        };
        let cst = if t.role == TensorRole::Output {
            ""
        } else {
            "const "
        };
        params.push(format!("{cst}{ty}* __restrict__ t{}", t.id.idx()));
    }
    e.l(format!(
        "__global__ __launch_bounds__(NTHREAD, {}) void logos_spatial_persistent({})",
        budget.blocks_per_sm,
        params.join(", ")
    ));
    e.open("");
    e.l("const int tid = threadIdx.x;");
    e.l("const int warp = tid / 32;");
    e.l("const int site = blockIdx.x;");
    e.l("const int item_off = ITEM_OFF[site];");
    e.l("const int n_items = ITEM_COUNT[site];");
    if budget.dyn_bytes > 0 {
        // ptxas caps STATIC __shared__ at 48 KiB regardless of `cudaFuncSetAttribute`, which only
        // raises the DYNAMIC limit. The epilogue scratch always lives here; the rings join it only when
        // they do not fit statically.
        e.l("extern __shared__ __align__(16) char logos_dyn_smem[];");
    }
    for (b, (_, _, ty)) in &ctx.rings {
        let k = b.idx();
        if budget.rings_dynamic {
            e.l(format!(
                "{ty}* ring_{k} = reinterpret_cast<{ty}*>(logos_dyn_smem + SMEM_OFF_{k});"
            ));
        } else {
            e.l(format!(
                "__shared__ __align__(16) {ty} ring_{k}[DEPTH_{k} * PAGE_{k}];"
            ));
        }
        e.l(format!(
            "int wctr_{k} = 0, rctr_{k} = 0; (void)wctr_{k}; (void)rctr_{k};"
        ));
    }
    e.l("nvcuda::wmma::fragment<nvcuda::wmma::accumulator, 16, 16, 16, ACCT> accf[PERW];");
    if ctx.fused_out.is_some() {
        // One tile per warp — the whole epilogue's shared memory, whatever the block tile is.
        e.l("ACCT* epi_scratch = reinterpret_cast<ACCT*>(logos_dyn_smem + SMEM_OFF_EPI);");
    }
    // With one item per site the schedule keeps no item loop, so the item's coordinates are constants.
    let has_item_loop = ops
        .iter()
        .any(|t| matches!(t.op, ChoreoOp::Loop { var, .. } if var == LV_ITEM));
    if !has_item_loop {
        // ★ A SITE WITH NO WORK MUST DO NONE. When every site holds at most one item the schedule
        // keeps no item loop — and without this guard a site holding ZERO items still ran the body
        // once, reading past its (empty) slice of the work list and writing a tile nobody asked
        // for, over a tile some other site had computed correctly. A block grid smaller than the
        // mesh is ordinary: it is what a big block tile on a small problem produces.
        e.l("if (n_items == 0) { return; }");
        e.l("const int item_id = ITEM_LIST[item_off];");
        e.l(format!(
            "const int oy = item_id / {}, ox = item_id % {};",
            ctx.grid_nx.max(1),
            ctx.grid_nx.max(1)
        ));
        e.l("(void)oy; (void)ox;");
    }
    let mut scope = BTreeSet::new();
    emit_ops(ctx, &mut e, ops, &mut scope, 0)?;
    e.close();
    e.l("");
    // Host: read the inputs, tilize, launch, print (or time).
    let a = &p.tensors[0];
    let b = &p.tensors[1];
    let out = p
        .output_tensor()
        .ok_or(SpatialLowerError::Unsupported("an output tensor"))?;
    let (m, k, n) = (a.elems.rows, a.elems.cols, b.elems.cols);
    let (tr, tc, te) = (ctx.tr, ctx.tc, ctx.te);
    e.open("int main()");
    e.l(format!("const int M = {m}, K = {k}, N = {n};"));
    e.l(format!(
        "const long long AW = {}LL * {}LL * {te};",
        a.tiles.rows_t, a.tiles.cols_t
    ));
    e.l(format!(
        "const long long BW = {}LL * {}LL * {te};",
        b.tiles.rows_t, b.tiles.cols_t
    ));
    e.l(format!(
        "const long long CW = {}LL * {}LL * {te};",
        out.tiles.rows_t, out.tiles.cols_t
    ));
    e.l("std::vector<double> ah((size_t)M * K, 0.0), bh((size_t)K * N, 0.0);");
    if bench {
        e.l("for (size_t i = 0; i < ah.size(); ++i) ah[i] = (double)((int)(i % 7) - 3);");
        e.l("for (size_t i = 0; i < bh.size(); ++i) bh[i] = (double)((int)(i % 11) - 5);");
    } else {
        e.l("{ FILE* f = fopen(\"a.txt\", \"r\"); if (!f) { fprintf(stderr, \"a.txt\\n\"); return 2; } for (size_t i = 0; i < ah.size(); ++i) { double v; if (fscanf(f, \"%lf\", &v) != 1) { fprintf(stderr, \"a.txt short\\n\"); return 2; } ah[i] = v; } fclose(f); }");
        e.l("{ FILE* f = fopen(\"b.txt\", \"r\"); if (!f) { fprintf(stderr, \"b.txt\\n\"); return 2; } for (size_t i = 0; i < bh.size(); ++i) { double v; if (fscanf(f, \"%lf\", &v) != 1) { fprintf(stderr, \"b.txt short\\n\"); return 2; } bh[i] = v; } fclose(f); }");
    }
    e.l("std::vector<OPT> at(AW), bt(BW);");
    e.l("memset(at.data(), 0, at.size() * sizeof(OPT)); memset(bt.data(), 0, bt.size() * sizeof(OPT));");
    e.l(format!("for (int i = 0; i < M; ++i) for (int j = 0; j < K; ++j) at[((long long)(i / {tr}) * {} + j / {tc}) * {te} + (i % {tr}) * {tc} + j % {tc}] = (OPT)ah[(size_t)i * K + j];", a.tiles.cols_t));
    e.l(format!("for (int i = 0; i < K; ++i) for (int j = 0; j < N; ++j) bt[((long long)(i / {tr}) * {} + j / {tc}) * {te} + (i % {tr}) * {tc} + j % {tc}] = (OPT)bh[(size_t)i * N + j];", b.tiles.cols_t));
    e.l("OPT *da, *db; long long* dc;");
    e.l("cudaMalloc(&da, AW * sizeof(OPT)); cudaMalloc(&db, BW * sizeof(OPT)); cudaMalloc(&dc, CW * sizeof(long long));");
    e.l("cudaMemcpy(da, at.data(), AW * sizeof(OPT), cudaMemcpyHostToDevice);");
    e.l("cudaMemcpy(db, bt.data(), BW * sizeof(OPT), cudaMemcpyHostToDevice);");
    e.l("cudaMemset(dc, 0, CW * sizeof(long long));");
    let smem_arg = if budget.dyn_bytes > 0 {
        ", DYN_SMEM"
    } else {
        ""
    };
    if budget.dynamic_opt_in {
        e.l("cudaFuncSetAttribute(logos_spatial_persistent, cudaFuncAttributeMaxDynamicSharedMemorySize, DYN_SMEM);");
    }
    e.l(format!("const int GRID = {};", sites.len()));
    if bench {
        e.l("cudaEvent_t e0, e1; cudaEventCreate(&e0); cudaEventCreate(&e1);");
        e.l(format!(
            "logos_spatial_persistent<<<GRID, NTHREAD{}>>>(da, db, dc); cudaDeviceSynchronize();",
            smem_arg
        ));
        e.l("const int ITERS = 20;");
        e.l("cudaEventRecord(e0);");
        e.l(format!("for (int it = 0; it < ITERS; ++it) logos_spatial_persistent<<<GRID, NTHREAD{}>>>(da, db, dc);", smem_arg));
        e.l("cudaEventRecord(e1); cudaEventSynchronize(e1);");
        // A launch that failed times nothing, and `ops / 0` prints an infinite rate that beats any bar.
        e.l("{ cudaError_t err = cudaGetLastError(); if (err != cudaSuccess) { fprintf(stderr, \"%s\\n\", cudaGetErrorString(err)); return 1; } }");
        e.l("float ms = 0; cudaEventElapsedTime(&ms, e0, e1);");
        e.l("const double ops = 2.0 * (double)M * (double)N * (double)K * (double)ITERS;");
        e.l("printf(\"GOPS %.1f\\n\", ops / (ms * 1e6));");
    } else {
        e.l(format!(
            "logos_spatial_persistent<<<GRID, NTHREAD{}>>>(da, db, dc);",
            smem_arg
        ));
        e.l("cudaError_t err = cudaDeviceSynchronize();");
        e.l("if (err != cudaSuccess) { fprintf(stderr, \"%s\\n\", cudaGetErrorString(err)); return 1; }");
        e.l("std::vector<long long> ct(CW);");
        e.l("cudaMemcpy(ct.data(), dc, CW * sizeof(long long), cudaMemcpyDeviceToHost);");
        e.l(format!("for (int i = 0; i < M; ++i) for (int j = 0; j < N; ++j) printf(\"%lld\\n\", ct[((long long)(i / {tr}) * {} + j / {tc}) * {te} + (i % {tr}) * {tc} + j % {tc}]);", out.tiles.cols_t));
    }
    e.l("cudaFree(da); cudaFree(db); cudaFree(dc);");
    e.l("return 0;");
    e.close();
    Ok(e.s)
}
