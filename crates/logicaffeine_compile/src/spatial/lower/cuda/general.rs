//! The spatial IR as a general persistent CUDA program: every op the evaluator runs, each site a resident block.
//!
//! The tile GEMM kernel ([`super::CudaPersistentBackend`]'s own path) is one plan's matmul with its accumulators in
//! WMMA fragments. A program the evaluator runs whole — a composition of sections under their own plans, Generic bodies
//! with their folds and contractions, a gather that reads an index from data, sites that hand off through semaphores —
//! prints here instead, as the evaluator's semantics op by op:
//!
//! - each engine of a site is a warp group of the site's block, running its own program as the schedule wrote it —
//!   loops and all — beside the site's other engines, as each engine is its own state machine on the fabric;
//! - a buffer is a shared-memory ring of `depth` pages at its own precision, between the engine that fills it and the
//!   engine that drains it: two counters in shared memory, the pages pushed and the pages popped, carry the evaluator's
//!   flow control — a reserve waits for a free page, a wait for a filled one — and a group's own named barrier makes
//!   its whole page land before the push publishes it;
//! - the accumulator is shared memory at 64 bits, zeroed when a region is acquired; a tile matmul adds each output
//!   element's dot product over the operands truncated to the operand precision, a Generic writes its plane as the
//!   evaluator's integer arm computes it, and a pack copies the region into the reserved page;
//! - a tile address's indirect term reads the index at element 0 of the index buffer's front page;
//! - a semaphore is a word of global memory per site: an increment is a fenced atomic add after the group's barrier, a
//!   wait spins one thread on the word and then crosses the group's barrier. A spin — on a semaphore or on a ring — that
//!   outlasts its bound records what it waited on in a fault word and the kernel stops, so a schedule that cannot
//!   finish reports rather than hangs.
//!
//! Every tensor lives in global memory at its precision; a load that may read what another block wrote goes around
//! the SM's L1 (`__ldcg`), which no other SM keeps coherent. The host initializes every tensor from its fill (or
//! from `t<id>.txt`, one value a line, where a run supplies one), launches one block per site, and prints every
//! `Output` and `State` tensor as `OUT <tensor> <element> <value>`, the line the fabric's testbench prints.

use std::collections::{BTreeMap, BTreeSet};

use super::{Emit, CudaArtifact, PersistentBudget};

mod flat;
use crate::codegen_sva::fabric_target::Precision;
use crate::spatial::ir::*;
use crate::spatial::lower::{Census, SpatialLowerError};
use crate::spatial::passes::{LV_ITEM, LV_ITEM_X, LV_ITEM_Y, LV_SUB_H, LV_SUB_W};
use crate::spatial::target::{precision_int_bits, SpatialTarget};

/// Threads per engine's warp group.
const GSZ: usize = 128;

/// Spins a semaphore wait makes before it reports the schedule stuck.
const SPIN_BOUND: u64 = 1 << 31;

/// Whether `p` needs the general program rather than the tile GEMM kernel: an op other than a tile matmul, sections
/// under different plans, a data-dependent address or a semaphore.
pub(super) fn wants_general(p: &SpatialProgram) -> bool {
    let only_matmuls = p.stages.iter().all(|s| match &s.kind {
        StageKind::Compute(op) => matches!(op.shape(), Ok(ComputeShape::Tiles { .. })),
        StageKind::Scalar(_) | StageKind::AllReduce { .. } | StageKind::Recv { .. } => false,
        StageKind::Load { .. } | StageKind::Store { .. } => true,
    });
    !only_matmuls || p.sections.iter().any(|s| s.plan != p.work.plan) || p.has_indirect_access() || !p.semaphores.is_empty()
}

/// The C type of an integer precision.
fn c_int(p: Precision) -> Result<&'static str, SpatialLowerError> {
    match p {
        Precision::Int8 => Ok("signed char"),
        Precision::Int16 => Ok("short"),
        Precision::Int32 => Ok("int"),
        Precision::Int64 => Ok("long long"),
        _ => Err(SpatialLowerError::Unsupported(
            "a float tensor or buffer in the general GPU program (its rounding is not printed yet)",
        )),
    }
}

fn bytes_of(ty: &str) -> usize {
    match ty {
        "signed char" => 1,
        "short" => 2,
        "int" => 4,
        _ => 8,
    }
}

struct Ctx<'a> {
    p: &'a SpatialProgram,
    t: usize,
    te: usize,
    /// Ring template → (depth, page elements, C type).
    rings: BTreeMap<BufferId, (usize, usize, &'static str)>,
    /// Block index of each site.
    block: BTreeMap<Site, usize>,
    /// Item coordinates by (section, site): the table an item loop indexes.
    coords: BTreeMap<(Option<usize>, Site), Vec<(usize, usize)>>,
}

impl Ctx<'_> {
    fn ring(&self, b: BufferId) -> usize {
        self.p.buffer(b).origin.unwrap_or(b).idx()
    }
    fn page(&self, b: BufferId) -> usize {
        self.rings[&self.p.buffer(b).origin.unwrap_or(b)].1
    }
    /// The element `elem` of the front page of `b`.
    fn front(&self, b: BufferId, elem: &str) -> String {
        let k = self.ring(b);
        format!("ring_{k}[rslot_{k} * PAGE_{k} + ({elem})]")
    }
}

fn var(v: LoopVar) -> String {
    match v {
        x if x == LV_ITEM => "item".into(),
        x if x == LV_SUB_H => "sh".into(),
        x if x == LV_SUB_W => "sw".into(),
        x => format!("lv{}", x.0),
    }
}

/// A tile address as C: its base, each loop's step over the variables in scope, the item's coordinates from its
/// section's table, and an indirect term read from its index buffer's front page.
fn address(ctx: &Ctx, at: &TileAddr, scope: &BTreeSet<LoopVar>, stage: StageId, site: Site) -> String {
    let mut terms = vec![format!("{}LL", at.base)];
    let table = coord_table(ctx, stage, site);
    for (v, stride) in &at.steps {
        if *stride == 0 {
            continue;
        }
        let item = if scope.contains(&LV_ITEM) { "item" } else { "0" };
        let name = if *v == LV_ITEM_Y {
            format!("{table}_y[{item}]")
        } else if *v == LV_ITEM_X {
            format!("{table}_x[{item}]")
        } else if scope.contains(v) {
            var(*v)
        } else {
            continue;
        };
        terms.push(format!("(long long){name} * {stride}"));
    }
    if let Some(ind) = &at.indirect {
        terms.push(format!(
            "((long long){} / {}) * {}",
            ctx.front(ind.index, "0"),
            ind.elems_per_tile.max(1),
            ind.tile_stride
        ));
    }
    terms.join(" + ")
}

/// The name of the coordinate table of `stage`'s section at `site`.
fn coord_table(ctx: &Ctx, stage: StageId, site: Site) -> String {
    let sec = ctx.p.section_of_stage(stage);
    format!("ITEM_b{}_s{}", ctx.block[&site], sec.map_or("n".to_string(), |k| k.to_string()))
}

/// The C body of a Generic's or an accumulator map's scalar body over operand values `opnd(i)` and axis values `axis(v)`,
/// in 64 bits, its sums, differences and products wrapping.
fn body_c(b: &TensorBody, opnd: &dyn Fn(usize) -> String, axis: &dyn Fn(LoopVar) -> String) -> Result<String, SpatialLowerError> {
    let two = |x: &TensorBody, y: &TensorBody| -> Result<(String, String), SpatialLowerError> {
        Ok((body_c(x, opnd, axis)?, body_c(y, opnd, axis)?))
    };
    Ok(match b {
        TensorBody::Operand(i) => opnd(*i),
        TensorBody::Const(bits) => format!("({}LL)", f64::from_bits(*bits) as i64),
        TensorBody::Add(x, y) => {
            let (a, c) = two(x, y)?;
            format!("add_ll({a}, {c})")
        }
        TensorBody::Sub(x, y) => {
            let (a, c) = two(x, y)?;
            format!("sub_ll({a}, {c})")
        }
        TensorBody::Mul(x, y) => {
            let (a, c) = two(x, y)?;
            format!("mul_ll({a}, {c})")
        }
        TensorBody::Min(x, y) => {
            let (a, c) = two(x, y)?;
            format!("min_ll({a}, {c})")
        }
        TensorBody::Max(x, y) => {
            let (a, c) = two(x, y)?;
            format!("max_ll({a}, {c})")
        }
        TensorBody::FloorDiv(x, y) => {
            let (a, c) = two(x, y)?;
            format!("floordiv_ll({a}, {c})")
        }
        TensorBody::Shr { arg, by } => format!("({} >> {by})", body_c(arg, opnd, axis)?),
        TensorBody::Select { cond, then, otherwise } => {
            let rhs = match cond.rhs() {
                PredRhs::Const(k) => format!("{k}LL"),
                PredRhs::Operand(i) => opnd(i),
            };
            let op = match cond {
                Pred::AxisLt(..) => "<",
                Pred::AxisGe(..) => ">=",
            };
            format!(
                "(((long long){} {op} {rhs}) ? {} : {})",
                axis(cond.var()),
                body_c(then, opnd, axis)?,
                body_c(otherwise, opnd, axis)?
            )
        }
        TensorBody::Unary { .. } => {
            return Err(SpatialLowerError::Unsupported("a transcendental in an integer body on the general GPU program"))
        }
    })
}

/// The fault code a spin on ring `k` records: past every semaphore's.
fn ring_fault(ctx: &Ctx, k: usize) -> usize {
    ctx.p.semaphores.len() + 1 + k
}

/// Print one engine's ops.
fn emit_ops(
    ctx: &Ctx,
    e: &mut Emit,
    ops: &[TaggedOp],
    scope: &mut BTreeSet<LoopVar>,
    site: Site,
) -> Result<(), SpatialLowerError> {
    for top in ops {
        match &top.op {
            ChoreoOp::Loop { count: LoopCount::Trip { .. }, .. } => {
                return Err(SpatialLowerError::Unsupported("a loop whose count the program reads from its data (a trip) on the general GPU program"))
            }
            ChoreoOp::Loop { var: v, count, body } => {
                let name = var(*v);
                e.open(format!("for (int {name} = 0; {name} < {}; ++{name})", count.bound()));
                let fresh = scope.insert(*v);
                emit_ops(ctx, e, body, scope, site)?;
                if fresh {
                    scope.remove(v);
                }
                e.close();
            }
            _ => emit_op(ctx, e, top, scope, site)?,
        }
    }
    Ok(())
}

/// Print one op that is not a loop: its flow control as the rings' and the semaphores' handshakes, its data movement
/// and its compute as loops over the engine's threads.
fn emit_op(ctx: &Ctx, e: &mut Emit, top: &TaggedOp, scope: &BTreeSet<LoopVar>, site: Site) -> Result<(), SpatialLowerError> {
    let te = ctx.te;
    {
        let stage = top.stage;
        match &top.op {
            ChoreoOp::Reserve { buf, pages } | ChoreoOp::Push { buf, pages } | ChoreoOp::Wait { buf, pages } | ChoreoOp::Pop { buf, pages }
                if *pages != 1 =>
            {
                let _ = buf;
                return Err(SpatialLowerError::Unsupported("a FIFO op that moves more than one page at once on the general GPU program"));
            }
            // The page this engine fills next waits until the drain has freed it.
            ChoreoOp::Reserve { buf, .. } => {
                let k = ctx.ring(*buf);
                e.l(format!(
                    "if (gtid == 0) {{ unsigned long long spins = 0; while (wres_{k} - *(volatile int*)&popped_{k} >= DEPTH_{k}) {{ if (++spins > {SPIN_BOUND}ULL || *(volatile int*)fault) {{ atomicCAS(fault, 0, {}); break; }} }} }} GBAR(); if (*(volatile int*)fault) return; wslot_{k} = wres_{k} % DEPTH_{k}; ++wres_{k};",
                    ring_fault(ctx, k) + 1
                ));
            }
            ChoreoOp::Push { buf, .. } => {
                let k = ctx.ring(*buf);
                e.l(format!("__threadfence(); GBAR(); if (gtid == 0) {{ __threadfence_block(); atomicAdd(&pushed_{k}, 1); }}"));
            }
            ChoreoOp::Wait { buf, .. } => {
                let k = ctx.ring(*buf);
                e.l(format!(
                    "if (gtid == 0) {{ unsigned long long spins = 0; while (*(volatile int*)&pushed_{k} - rctr_{k} < 1) {{ if (++spins > {SPIN_BOUND}ULL || *(volatile int*)fault) {{ atomicCAS(fault, 0, {}); break; }} }} __threadfence_block(); }} GBAR(); if (*(volatile int*)fault) return; rslot_{k} = rctr_{k} % DEPTH_{k};",
                    ring_fault(ctx, k) + 1
                ));
            }
            ChoreoOp::Pop { buf, .. } => {
                let k = ctx.ring(*buf);
                e.l(format!("GBAR(); if (gtid == 0) {{ __threadfence_block(); atomicAdd(&popped_{k}, 1); }} ++rctr_{k};"));
            }
            ChoreoOp::ReadBarrier | ChoreoOp::WriteBarrier | ChoreoOp::Flush { .. } => {}
            ChoreoOp::ReadTiles { tensor, at, block_h, block_w, stride_w, stride_h, into } => {
                let k = ctx.ring(*into);
                let base = address(ctx, at, scope, stage, site);
                e.open("");
                e.l(format!("const long long base = {base};"));
                e.open(format!("for (int c = gtid; c < {block_h} * {block_w} * {te}; c += GSZ)"));
                e.l(format!("const int tl = c / {te}, el = c % {te}, bh = tl / {block_w}, bw = tl % {block_w};"));
                e.l(format!("const long long tile = base + (long long)bh * {stride_h} + (long long)bw * {stride_w};"));
                e.l(format!("ring_{k}[wslot_{k} * PAGE_{k} + tl * {te} + el] = __ldcg(&t{}[tile * {te} + el]);", tensor.idx()));
                e.close();
                e.close();
            }
            ChoreoOp::WriteTiles { from, tensor, at, block_h, block_w, stride_w, stride_h } => {
                let k = ctx.ring(*from);
                let base = address(ctx, at, scope, stage, site);
                e.open("");
                e.l(format!("const long long base = {base};"));
                e.open(format!("for (int c = gtid; c < {block_h} * {block_w} * {te}; c += GSZ)"));
                e.l(format!("const int tl = c / {te}, el = c % {te}, bh = tl / {block_w}, bw = tl % {block_w};"));
                e.l(format!("const long long tile = base + (long long)bh * {stride_h} + (long long)bw * {stride_w};"));
                e.l(format!("t{}[tile * {te} + el] = ring_{k}[rslot_{k} * PAGE_{k} + tl * {te} + el];", tensor.idx()));
                e.close();
                e.close();
                e.l("GBAR();");
            }
            ChoreoOp::AcquireAcc { tiles } => {
                e.l(format!("for (int c = gtid; c < {tiles} * {te}; c += GSZ) acc[c] = 0;"));
                e.l(format!("acc_tiles = {tiles};"));
                e.l("GBAR();");
            }
            ChoreoOp::ReleaseAcc => {}
            ChoreoOp::Compute { op, ins, acc_reload } => {
                if let Some(r) = acc_reload {
                    e.l(format!("for (int c = gtid; c < acc_tiles * {te}; c += GSZ) acc[c] = {};", ctx.front(*r, "c")));
                    e.l("GBAR();");
                }
                emit_compute(ctx, e, op, ins, scope)?;
                e.l("GBAR();");
            }
            ChoreoOp::PackAcc { to, tiles, convert, accumulate } => {
                let k = ctx.ring(*to);
                if precision_int_bits(*convert).is_none() {
                    return Err(SpatialLowerError::Unsupported("a float pack on the general GPU program"));
                }
                let i32 = ctx.p.plan_of_stage(stage).is_some_and(|pl| pl.acc == AccPolicy::I32);
                let v = if i32 { "(long long)(int)acc[c]" } else { "acc[c]" };
                let dst = format!("ring_{k}[wslot_{k} * PAGE_{k} + c]");
                if *accumulate {
                    e.l(format!("for (int c = gtid; c < {tiles} * {te}; c += GSZ) {dst} = (long long){dst} + {v};"));
                } else {
                    e.l(format!("for (int c = gtid; c < {tiles} * {te}; c += GSZ) {dst} = {v};"));
                }
                e.l("GBAR();");
            }
            ChoreoOp::SemSet { sem, value } => {
                let v = match value {
                    SemValue::Invalid => 0,
                    SemValue::Valid => 1,
                    SemValue::Count(n) => *n,
                };
                e.l(format!("__threadfence(); GBAR(); if (gtid == 0) atomicExch(&sems[{} * NSITE + {}], {v});", sem.idx(), ctx.block[&site]));
            }
            ChoreoOp::SemInc { sem, at, by } => {
                let to = ctx.block.get(at).ok_or(SpatialLowerError::Unsupported("a semaphore increment at a site no block runs"))?;
                e.l(format!("__threadfence(); GBAR(); if (gtid == 0) atomicAdd(&sems[{} * NSITE + {to}], {by});", sem.idx()));
            }
            ChoreoOp::SemWait { sem, until } => {
                let cond = match until {
                    SemCond::Equals(n) => format!("== {n}"),
                    SemCond::AtLeast(n) => format!(">= {n}"),
                };
                e.l(format!(
                    "if (gtid == 0) {{ unsigned long long spins = 0; while (!(atomicAdd(&sems[{0} * NSITE + {1}], 0) {cond})) {{ if (++spins > {SPIN_BOUND}ULL || *(volatile int*)fault) {{ atomicCAS(fault, 0, {0} + 1); break; }} }} __threadfence(); }} GBAR(); if (*(volatile int*)fault) return;",
                    sem.idx(),
                    ctx.block[&site]
                ));
            }
            ChoreoOp::McastSend { .. } | ChoreoOp::McastSetValid { .. } => {
                return Err(SpatialLowerError::Unsupported("a multicast on the general GPU program (sm_86 has no cluster multicast)"))
            }
            ChoreoOp::Barrier { .. } => return Err(SpatialLowerError::Unsupported("a group barrier on the general GPU program")),
            ChoreoOp::Loop { .. } => unreachable!("emit_ops walks a loop's body"),
        }
    }
    Ok(())
}

/// A compute op over the accumulator region the schedule acquired.
fn emit_compute(ctx: &Ctx, e: &mut Emit, op: &ComputeOp, ins: &[BufferId], scope: &BTreeSet<LoopVar>) -> Result<(), SpatialLowerError> {
    let (t, te) = (ctx.t, ctx.te);
    match op.shape().map_err(SpatialLowerError::Unsupported)? {
        ComputeShape::Tiles { block, sub_blocks, operand } => {
            let bits = precision_int_bits(operand)
                .ok_or(SpatialLowerError::Unsupported("a float tile matmul on the general GPU program"))?;
            let (osh, osw, ibw) = (block.osh, block.osw, block.ibw);
            let per_site_n = sub_blocks.1 * osw;
            let sh = if scope.contains(&LV_SUB_H) { "sh" } else { "0" };
            let sw = if scope.contains(&LV_SUB_W) { "sw" } else { "0" };
            let (ka, kb) = (ctx.ring(ins[0]), ctx.ring(ins[1]));
            e.open(format!("for (int c = gtid; c < {osh} * {osw} * {te}; c += GSZ)"));
            e.l(format!("const int ot = c / {te}, el = c % {te}, i = ot / {osw}, j = ot % {osw}, r = el / {t}, cc = el % {t};"));
            e.l("long long s = acc[c];");
            e.open(format!("for (int kk = 0; kk < {ibw}; ++kk)"));
            e.l(format!("const int at = (({sh} * {osh} + i) * {ibw} + kk) * {te}, bt = (kk * {per_site_n} + ({sw} * {osw} + j)) * {te};"));
            e.open(format!("for (int x = 0; x < {t}; ++x)"));
            e.l(format!(
                "s += trunc_ll((long long)ring_{ka}[rslot_{ka} * PAGE_{ka} + at + r * {t} + x], {bits}) * trunc_ll((long long)ring_{kb}[rslot_{kb} * PAGE_{kb} + bt + x * {t} + cc], {bits});"
            ));
            e.close();
            e.close();
            e.l("acc[c] = s;");
            e.close();
        }
        ComputeShape::AccMap { body, reads } => {
            // Element `c` of the acquired accumulator, `c / te` its tile and `c % t` its column.
            let opnd = |i: usize| -> String {
                match reads[i] {
                    AccRead::Tile => format!("((long long){})", ctx.front(ins[i], "c")),
                    AccRead::RowOfTile => {
                        let k = ctx.ring(ins[i]);
                        let n = (ctx.page(ins[i]) / te).max(1);
                        format!("((long long)ring_{k}[rslot_{k} * PAGE_{k} + ((c / {te}) % {n}) * {te} + (c % {t})])")
                    }
                }
            };
            let point = body_c(&body, &opnd, &|_| unreachable!("an accumulator map reads no axis"))?;
            e.l(format!("for (int c = gtid; c < acc_tiles * {te}; c += GSZ) acc[c] = {point};"));
        }
        ComputeShape::Scalar(_) => return Err(SpatialLowerError::Unsupported("a scalar stage on the general GPU program")),
        ComputeShape::Generic { iters, ins: maps, out, body } => {
            if out.indirect.is_some() {
                return Err(SpatialLowerError::Unsupported("a scatter (an indirect output map) on the general GPU program"));
            }
            if !body.integral() {
                return Err(SpatialLowerError::Unsupported("a float Generic on the general GPU program"));
            }
            let (row, col, k) = match iters {
                [r, c] => (*r, *c, None),
                [r, c, k] => (*r, *c, Some(*k)),
                _ => return Err(SpatialLowerError::Unsupported("a Generic whose space is not (rows, cols) or (rows, cols, contraction)")),
            };
            if k.is_some_and(|k| !matches!(k.kind, IterKind::Reduce { pool: ReducePool::Sum })) {
                return Err(SpatialLowerError::Unsupported("a Generic whose contraction axis reduces by a pool other than a sum"));
            }
            let k_var = k.map(|k| k.var);
            // An operand at a point: its map's offset into the page, read as a flat plane of `t` rows; outside the
            // plane, zero. A gather's offset adds the value its index operand reads at the same point.
            let offset = |i: usize| -> String {
                let m = &maps[i];
                let mut terms = vec![format!("{}LL", m.base)];
                for (v, stride) in &m.terms {
                    let a = if *v == row.var {
                        "r"
                    } else if *v == col.var {
                        "cc"
                    } else if Some(*v) == k_var {
                        "x"
                    } else {
                        continue;
                    };
                    terms.push(format!("(long long){a} * {stride}"));
                }
                terms.join(" + ")
            };
            let read = |i: usize| -> String {
                let kb = ctx.ring(ins[i]);
                let width = ctx.page(ins[i]) / te * t;
                let off = match maps[i].indirect {
                    Some(ind) => {
                        let ki = ctx.ring(ins[ind.operand]);
                        let wi = ctx.page(ins[ind.operand]) / te * t;
                        format!(
                            "({}) + read_plane(&ring_{ki}[rslot_{ki} * PAGE_{ki}], {}, {wi}, {t}) * {}",
                            offset(i),
                            offset(ind.operand),
                            ind.stride
                        )
                    }
                    None => offset(i),
                };
                format!("read_plane(&ring_{kb}[rslot_{kb} * PAGE_{kb}], {off}, {width}, {t})")
            };
            let axis = |v: LoopVar| -> String {
                if v == row.var {
                    "r".into()
                } else if v == col.var {
                    "cc".into()
                } else if Some(v) == k_var {
                    "x".into()
                } else {
                    "0".into()
                }
            };
            let opnd = |i: usize| format!("v{i}");
            let point = body_c(body, &opnd, &axis)?;
            let reads: Vec<String> = (0..maps.len()).map(|i| format!("const long long v{i} = {};", read(i))).collect();
            let value = |e: &mut Emit| {
                match k {
                    None => {
                        e.l("long long value;");
                        e.open("");
                        e.l("const int x = 0; (void)x;");
                        for r in &reads {
                            e.l(r);
                        }
                        e.l(format!("value = {point};"));
                        e.close();
                    }
                    Some(k) => {
                        e.l("long long value = 0;");
                        e.open(format!("for (int x = 0; x < {}; ++x)", k.extent));
                        for r in &reads {
                            e.l(r);
                        }
                        e.l(format!("value += {point};"));
                        e.close();
                    }
                }
            };
            let out_tiles = if matches!(col.kind, IterKind::Reduce { .. }) { 1 } else { col.extent.div_ceil(t) };
            e.l(format!("for (int c = gtid; c < {out_tiles} * {te}; c += GSZ) acc[c] = 0;"));
            e.l("GBAR();");
            let fold = |pool: ReducePool| -> (&'static str, &'static str) {
                match pool {
                    ReducePool::Sum => ("acc_v = first ? value : acc_v + value;", "acc_v"),
                    ReducePool::Max => ("acc_v = (first || value > acc_v) ? value : acc_v;", "acc_v"),
                    ReducePool::ArgMax => ("if (first || value > acc_v) { acc_v = value; acc_i = at; }", "(long long)acc_i"),
                }
            };
            match (row.kind, col.kind) {
                (IterKind::Parallel, IterKind::Parallel) => {
                    e.open(format!("for (int p = gtid; p < {} * {}; p += GSZ)", row.extent, col.extent));
                    e.l(format!("const int r = p / {0}, cc = p % {0};", col.extent));
                    value(e);
                    e.l(format!("acc[(cc / {t}) * {te} + (r % {t}) * {t} + (cc % {t})] = value;"));
                    e.close();
                }
                (IterKind::Parallel, IterKind::Reduce { pool }) => {
                    let store = format!("for (int q = 0; q < {t}; ++q) acc[(r % {t}) * {t} + q] = {};", fold_result(pool));
                    lane_fold(e, pool, ("r", row.extent), ("cc", col.extent), &value, &store);
                }
                (IterKind::Reduce { pool }, IterKind::Parallel) => {
                    let store = format!("for (int q = 0; q < {t}; ++q) acc[(cc / {t}) * {te} + q * {t} + (cc % {t})] = {};", fold_result(pool));
                    lane_fold(e, pool, ("cc", col.extent), ("r", row.extent), &value, &store);
                }
                (IterKind::Reduce { pool: row_pool }, IterKind::Reduce { pool: col_pool }) => {
                    let (inner, inner_result) = fold(col_pool);
                    let outer = match row_pool {
                        ReducePool::Sum => "out_v = (r == 0) ? got : out_v + got;",
                        ReducePool::Max => "out_v = (r == 0 || got > out_v) ? got : out_v;",
                        ReducePool::ArgMax => "if (r == 0 || got > out_v) { out_v = got; out_i = r; }",
                    };
                    let outer_result = if row_pool == ReducePool::ArgMax { "(long long)out_i" } else { "out_v" };
                    e.open("if (gtid == 0)");
                    e.l("long long out_v = 0; int out_i = 0; (void)out_i;");
                    e.open(format!("for (int r = 0; r < {}; ++r)", row.extent));
                    e.l("long long acc_v = 0; int acc_i = 0; (void)acc_i;");
                    e.open(format!("for (int cc = 0; cc < {}; ++cc)", col.extent));
                    e.l("const bool first = cc == 0; const int at = cc; (void)at;");
                    value(e);
                    e.l(inner);
                    e.close();
                    e.l(format!("const long long got = {inner_result};"));
                    e.l(outer);
                    e.close();
                    e.l(format!("for (int q = 0; q < {te}; ++q) acc[q] = {outer_result};"));
                    e.close();
                }
            }
        }
    }
    Ok(())
}

/// The lanes one output's fold is split over: the largest power of two up to a warp that its reduced axis fills, so every
/// lane folds at least one point.
fn fold_lanes(reduced: usize) -> usize {
    let mut lanes = 1;
    while lanes * 2 <= reduced.min(32) {
        lanes *= 2;
    }
    lanes
}

/// What a fold leaves for its output: the folded value, or an arg-max's index.
fn fold_result(pool: ReducePool) -> &'static str {
    match pool {
        ReducePool::ArgMax => "(long long)acc_i",
        _ => "acc_v",
    }
}

/// A fold of `reduced` points into each of `outs` outputs, every output's points split over its lanes
/// ([`fold_lanes`]): a lane folds its points in order, then the lanes of an output — one segment of a warp — combine by
/// shuffles, and lane 0 runs `store` with the output's result in `acc_v` and `acc_i`. A sum and a max are associative,
/// and an arg-max's lanes carry the first index of their greatest value and keep the lower index on a tie, so the
/// result is the sequential fold's exactly. The loop over groups of the block is uniform, so every thread of a warp
/// takes part in each shuffle.
fn lane_fold(
    e: &mut Emit,
    pool: ReducePool,
    (out_var, outs): (&str, usize),
    (red_var, reduced): (&str, usize),
    value: &dyn Fn(&mut Emit),
    store: &str,
) {
    let lanes = fold_lanes(reduced);
    e.open(format!("for (int g0 = 0; g0 < {outs} * {lanes}; g0 += GSZ)"));
    e.l(format!("const int g = g0 + gtid; const int {out_var} = g / {lanes}, lane = g % {lanes}; const bool live = {out_var} < {outs};"));
    e.l("long long acc_v = 0; int acc_i = 0; bool any = false;");
    e.open("if (live)");
    e.open(format!("for (int {red_var} = lane; {red_var} < {reduced}; {red_var} += {lanes})"));
    e.l(format!("const bool first = !any; const int at = {red_var}; (void)at;"));
    value(e);
    e.l(match pool {
        ReducePool::Sum => "acc_v = first ? value : acc_v + value;",
        ReducePool::Max => "acc_v = (first || value > acc_v) ? value : acc_v;",
        ReducePool::ArgMax => "if (first || value > acc_v) { acc_v = value; acc_i = at; }",
    });
    e.l("any = true;");
    e.close();
    e.close();
    if lanes > 1 {
        e.open(format!("for (int s = {} ; s > 0; s >>= 1)", lanes / 2));
        e.l(format!("const long long ov = __shfl_down_sync(0xffffffffu, acc_v, s, {lanes});"));
        e.l(format!("const int oi = __shfl_down_sync(0xffffffffu, acc_i, s, {lanes});"));
        e.l(format!("const bool oany = __shfl_down_sync(0xffffffffu, any, s, {lanes});"));
        e.l(match pool {
            ReducePool::Sum => "if (oany) acc_v = any ? acc_v + ov : ov;",
            ReducePool::Max => "if (oany && (!any || ov > acc_v)) acc_v = ov;",
            ReducePool::ArgMax => "if (oany && (!any || ov > acc_v || (ov == acc_v && oi < acc_i))) { acc_v = ov; acc_i = oi; }",
        });
        e.l("any = any || oany;");
        e.close();
    }
    e.open("if (live && lane == 0)");
    e.l(store);
    e.close();
    e.close();
}

/// What every GPU program of `p` is printed against — its rings (each buffer template's depth, page and C type), each
/// site's block, each section's item coordinates by site — and its sites in order; refused by name where the GPU
/// programs cannot run `p`.
fn context(p: &SpatialProgram) -> Result<(Ctx<'_>, Vec<Site>), SpatialLowerError> {
    let pl = p.placement.as_ref().ok_or(SpatialLowerError::Unsupported("a placed program"))?;
    p.schedule.as_ref().ok_or(SpatialLowerError::Unsupported("a scheduled program"))?;
    if !pl.groups.is_empty() {
        return Err(SpatialLowerError::Unsupported("a multicast group on the general GPU program (sm_86 has no cluster multicast)"));
    }
    let (t, te) = (p.tile.rows, p.tile.elems());
    if p.tile.rows != p.tile.cols {
        return Err(SpatialLowerError::Unsupported("a non-square tile on the general GPU program"));
    }
    for tensor in &p.tensors {
        c_int(tensor.dtype)?;
    }
    let mut rings: BTreeMap<BufferId, (usize, usize, &'static str)> = BTreeMap::new();
    let extent = p.page_extents();
    for b in &p.buffers {
        let depth = match b.kind {
            BufferKind::Fifo { depth_pages, .. } | BufferKind::Interm { depth_pages, .. } => depth_pages,
            BufferKind::Accumulator { .. } => continue,
        };
        let page = extent[b.id.idx()] * te;
        let ty = c_int(b.dtype)?;
        let key = b.origin.unwrap_or(b.id);
        let entry = rings.entry(key).or_insert((depth, page, ty));
        if bytes_of(ty) > bytes_of(entry.2) {
            entry.2 = ty;
        }
        entry.0 = entry.0.max(depth);
        entry.1 = entry.1.max(page);
    }
    let sites: Vec<Site> = pl.items_of_site.iter().map(|(s, _)| *s).collect();
    let block: BTreeMap<Site, usize> = sites.iter().enumerate().map(|(i, s)| (*s, i)).collect();
    let items = p.site_items();
    let mut coords: BTreeMap<(Option<usize>, Site), Vec<(usize, usize)>> = BTreeMap::new();
    let sections: Vec<Option<usize>> = if p.sections.is_empty() { vec![None] } else { (0..p.sections.len()).map(Some).collect() };
    for s in &sites {
        for sec in &sections {
            coords.insert((*sec, *s), items.at(*sec, *s).iter().map(|it| p.work.items[it.idx()].coord).collect());
        }
    }
    Ok((Ctx { p, t, te, rings, block, coords }, sites))
}

/// The GPU program of `p`: one block in a fixed order where its rings and accumulators fit one block's shared memory
/// ([`flat`]) — a program's sections run one after another, and a block's own barrier is the cheapest handshake there is
/// — with the general program beside it; the general program, one block per site, where they do not.
pub(super) fn lower(p: &SpatialProgram, target: &SpatialTarget) -> Result<CudaArtifact, SpatialLowerError> {
    let general = lower_general(p, target);
    match flat::lower_flat(p, target)? {
        Some(mut one) => {
            one.general = general.ok().map(Box::new);
            Ok(one)
        }
        None => general,
    }
}

/// The general program of `p`.
pub(super) fn lower_general(p: &SpatialProgram, target: &SpatialTarget) -> Result<CudaArtifact, SpatialLowerError> {
    let (ctx, sites) = context(p)?;
    let sched = p.schedule.as_ref().expect("a context is of a scheduled program");
    // Each site's engine programs, each its own warp group.
    let mut site_progs: Vec<(Site, Vec<Vec<TaggedOp>>)> = Vec::new();
    for s in &sites {
        let progs: Vec<Vec<TaggedOp>> = sched.programs.iter().filter(|pr| pr.site == *s).map(|pr| pr.ops.clone()).collect();
        site_progs.push((*s, progs));
    }
    let groups = site_progs.iter().map(|(_, g)| g.len()).max().unwrap_or(1).max(1);
    if groups > 15 {
        return Err(SpatialLowerError::Unsupported("a site of more than fifteen engines on the general GPU program (a block has fifteen named barriers)"));
    }
    let (per_site, acc_tiles_max) = site_memory(&ctx, &site_progs);
    let acc_bytes = align16(acc_tiles_max * ctx.te * 8);
    let dyn_bytes = per_site.iter().map(|m| m.arena + m.counters).max().unwrap_or(0) + acc_bytes * groups;
    if dyn_bytes > target.max_local_mem_bytes() {
        return Err(SpatialLowerError::TooBig { what: "a site's rings and accumulators in shared memory", need: dyn_bytes, have: target.max_local_mem_bytes() });
    }
    let budget = PersistentBudget {
        smem_bytes: dyn_bytes,
        dynamic_opt_in: dyn_bytes > 48 * 1024,
        rings_dynamic: true,
        dyn_bytes,
        regs_est: 64,
        blocks_per_sm: 1,
        threads: GSZ * groups,
    };
    let program_cu = print(&ctx, &site_progs, &per_site, acc_bytes, groups, &budget, false)?;
    let bench_cu = print(&ctx, &site_progs, &per_site, acc_bytes, groups, &budget, true)?;
    Ok(CudaArtifact { program_cu, bench_cu, budget, census: Census::of(p), general: None })
}

/// A site's shared memory past its accumulators: each touched ring's offset in the arena its phase shares, the arena's
/// size, and the counters' — one pair per ring, never shared, so a phase's rings start counting from zero.
struct SiteMemory {
    layout: BTreeMap<usize, usize>,
    arena: usize,
    counters: usize,
}

/// Each site's shared memory past its accumulators, and the most tiles any engine acquires.
///
/// ★ RINGS OF DIFFERENT PHASES SHARE THEIR MEMORY. A section's rendezvous holds every op after it until every op up to
/// it is done, so the rings of sections a rendezvous separates are never live together: each phase — the run of sections
/// between two rendezvous — lays its rings from the same origin, and a site needs the largest phase's.
fn site_memory(ctx: &Ctx, site_progs: &[(Site, Vec<Vec<TaggedOp>>)]) -> (Vec<SiteMemory>, usize) {
    let p = ctx.p;
    let mut phase_of_section = Vec::new();
    let mut phase = 0usize;
    for sec in &p.sections {
        phase_of_section.push(phase);
        if sec.barrier_after.is_some() {
            phase += 1;
        }
    }
    let phase_of_ring = |k: usize| -> usize {
        p.sections.iter().position(|sec| (sec.buffers.0..sec.buffers.1).contains(&k)).map_or(0, |i| phase_of_section[i])
    };
    let mut per_site = Vec::new();
    let mut acc_tiles_max = 1usize;
    for (_, progs) in site_progs {
        let mut touched = BTreeSet::new();
        for ops in progs {
            walk_tagged_ops(ops, &mut |op| match op {
                ChoreoOp::Reserve { buf, .. } | ChoreoOp::Push { buf, .. } | ChoreoOp::Wait { buf, .. } | ChoreoOp::Pop { buf, .. } => {
                    touched.insert(ctx.ring(*buf));
                }
                ChoreoOp::ReadTiles { into, at, .. } => {
                    touched.insert(ctx.ring(*into));
                    touched.extend(at.indirect.iter().map(|i| ctx.ring(i.index)));
                }
                ChoreoOp::WriteTiles { from, at, .. } => {
                    touched.insert(ctx.ring(*from));
                    touched.extend(at.indirect.iter().map(|i| ctx.ring(i.index)));
                }
                ChoreoOp::PackAcc { to, .. } => {
                    touched.insert(ctx.ring(*to));
                }
                ChoreoOp::Compute { ins, acc_reload, .. } => {
                    touched.extend(ins.iter().map(|b| ctx.ring(*b)));
                    touched.extend(acc_reload.iter().map(|b| ctx.ring(*b)));
                }
                ChoreoOp::AcquireAcc { tiles } => acc_tiles_max = acc_tiles_max.max(*tiles),
                _ => {}
            });
        }
        let mut layout: BTreeMap<usize, usize> = BTreeMap::new();
        let mut phase_end: BTreeMap<usize, usize> = BTreeMap::new();
        for (b, (d, pg, ty)) in ctx.rings.iter().filter(|(b, _)| touched.contains(&b.idx())) {
            let end = phase_end.entry(phase_of_ring(b.idx())).or_insert(0);
            layout.insert(b.idx(), *end);
            *end += align16(d * pg * bytes_of(ty));
        }
        let arena = phase_end.values().copied().max().unwrap_or(0);
        let counters = 16 * layout.len();
        per_site.push(SiteMemory { layout, arena, counters });
    }
    (per_site, acc_tiles_max)
}

fn align16(n: usize) -> usize {
    (n + 15) & !15
}

fn walk_tagged_ops(ops: &[TaggedOp], f: &mut dyn FnMut(&ChoreoOp)) {
    for t in ops {
        f(&t.op);
        if let ChoreoOp::Loop { body, .. } = &t.op {
            walk_tagged_ops(body, f);
        }
    }
}

fn print(
    ctx: &Ctx,
    site_progs: &[(Site, Vec<Vec<TaggedOp>>)],
    per_site: &[SiteMemory],
    acc_bytes: usize,
    groups: usize,
    budget: &PersistentBudget,
    bench: bool,
) -> Result<String, SpatialLowerError> {
    let p = ctx.p;
    let nsite = site_progs.len();
    let mut e = Emit { s: String::new(), ind: 0 };
    e.l(format!("// {} — a general persistent CUDA program printed from the spatial IR; each block is one IR site.", p.name));
    for inc in ["cstdio", "cstdlib", "cstring", "vector"] {
        e.l(format!("#include <{inc}>"));
    }
    e.l(format!("#define GSZ {GSZ}"));
    e.l(format!("#define NTHREAD {}", GSZ * groups));
    // A group's own barrier: named barrier `grp + 1` over its threads alone.
    e.l("#define GBAR() asm volatile(\"bar.sync %0, %1;\" :: \"r\"(grp + 1), \"r\"(GSZ) : \"memory\")");
    e.l(format!("#define NSITE {nsite}"));
    for (b, (depth, page, _)) in &ctx.rings {
        e.l(format!("#define DEPTH_{} {depth}", b.idx()));
        e.l(format!("#define PAGE_{} {page}", b.idx()));
    }
    print_helpers(ctx, &mut e);
    let mut params: Vec<String> = Vec::new();
    for tensor in &p.tensors {
        params.push(format!("{}* __restrict__ t{}", c_int(tensor.dtype)?, tensor.id.idx()));
    }
    params.push("int* sems".into());
    params.push("int* fault".into());
    e.open(format!("__global__ __launch_bounds__(NTHREAD, 1) void logos_spatial_general({})", params.join(", ")));
    e.l("const int grp = threadIdx.x / GSZ, gtid = threadIdx.x % GSZ;");
    e.l("extern __shared__ __align__(16) char logos_dyn_smem[];");
    e.l(format!("long long* acc = reinterpret_cast<long long*>(logos_dyn_smem + grp * {acc_bytes});"));
    e.l("int acc_tiles = 0; (void)acc_tiles;");
    e.open("switch (blockIdx.x)");
    for (bi, (site, progs)) in site_progs.iter().enumerate() {
        e.open(format!("case {bi}:"));
        let mem = &per_site[bi];
        let arena = acc_bytes * groups;
        let mut counter_at = arena + mem.arena;
        let mut counters = Vec::new();
        for (b, (_, _, ty)) in &ctx.rings {
            let k = b.idx();
            let Some(at) = mem.layout.get(&k) else { continue };
            e.l(format!("{ty}* ring_{k} = reinterpret_cast<{ty}*>(logos_dyn_smem + {});", arena + at));
            e.l(format!(
                "int& pushed_{k} = *reinterpret_cast<int*>(logos_dyn_smem + {counter_at}); int& popped_{k} = *reinterpret_cast<int*>(logos_dyn_smem + {});",
                counter_at + 4
            ));
            counter_at += 16;
            e.l(format!("int wres_{k} = 0, rctr_{k} = 0, wslot_{k} = 0, rslot_{k} = 0; (void)wres_{k}; (void)rctr_{k}; (void)wslot_{k}; (void)rslot_{k};"));
            counters.push(k);
        }
        // Every counter starts at zero before any engine runs.
        let zero: Vec<String> = counters.iter().map(|k| format!("pushed_{k} = 0; popped_{k} = 0;")).collect();
        e.l(format!("if (threadIdx.x == 0) {{ {} }} __syncthreads();", zero.join(" ")));
        for (g, ops) in progs.iter().enumerate() {
            e.open(format!("if (grp == {g})"));
            emit_ops(ctx, &mut e, ops, &mut BTreeSet::new(), *site)?;
            e.close();
        }
        e.l("break;");
        e.close();
    }
    e.close();
    e.close();
    e.l("");
    // Host.
    e.open("int main()");
    e.l("int *dsems, *dfault;");
    let nsem = p.semaphores.len().max(1);
    e.l(format!("cudaMalloc(&dsems, {nsem} * NSITE * sizeof(int)); cudaMalloc(&dfault, sizeof(int)); cudaMemset(dfault, 0, sizeof(int));"));
    e.l(format!("std::vector<int> sem_init({nsem} * NSITE, 0);"));
    for s in &p.semaphores {
        for site in &s.sites {
            if let Some(b) = ctx.block.get(site) {
                e.l(format!("sem_init[{} * NSITE + {b}] = {};", s.id.idx(), s.init));
            }
        }
    }
    host_tensors(ctx, &mut e)?;
    let args: Vec<String> = p.tensors.iter().map(|t| format!("d{}", t.id.idx())).chain(["dsems".into(), "dfault".into()]).collect();
    let args = args.join(", ");
    e.l(format!("const int DYN = {};", budget.dyn_bytes));
    if budget.dynamic_opt_in {
        e.l("cudaFuncSetAttribute(logos_spatial_general, cudaFuncAttributeMaxDynamicSharedMemorySize, DYN);");
    }
    e.l("auto launch = [&]() { cudaMemcpy(dsems, sem_init.data(), sem_init.size() * sizeof(int), cudaMemcpyHostToDevice); logos_spatial_general<<<NSITE, NTHREAD, DYN>>>(" .to_string() + &args + "); };");
    if bench {
        e.l("launch(); cudaDeviceSynchronize();");
        e.l("cudaEvent_t e0, e1; cudaEventCreate(&e0); cudaEventCreate(&e1);");
        e.l("const int ITERS = 200; float best = 1e30f;");
        e.open("for (int it = 0; it < ITERS; ++it)");
        e.l("cudaMemcpy(dsems, sem_init.data(), sem_init.size() * sizeof(int), cudaMemcpyHostToDevice);");
        e.l(format!("cudaEventRecord(e0); logos_spatial_general<<<NSITE, NTHREAD, DYN>>>({args}); cudaEventRecord(e1); cudaEventSynchronize(e1);"));
        e.l("float ms = 0; cudaEventElapsedTime(&ms, e0, e1); if (ms < best) best = ms;");
        e.close();
        e.l("{ cudaError_t err = cudaGetLastError(); if (err != cudaSuccess) { fprintf(stderr, \"%s\\n\", cudaGetErrorString(err)); return 1; } }");
        e.l("printf(\"US_PER_STEP %.3f\\n\", best * 1000.0f);");
    } else {
        e.l("launch();");
        e.l("cudaError_t err = cudaDeviceSynchronize();");
        e.l("if (err != cudaSuccess) { fprintf(stderr, \"%s\\n\", cudaGetErrorString(err)); return 1; }");
        e.l("int hfault = 0; cudaMemcpy(&hfault, dfault, sizeof(int), cudaMemcpyDeviceToHost);");
        e.l(format!(
            "if (hfault) {{ if (hfault - 1 < {0}) fprintf(stderr, \"a semaphore wait never finished: semaphore %d\\n\", hfault - 1); else fprintf(stderr, \"a ring wait never finished: ring %d\\n\", hfault - 1 - {0} - 1); return 3; }}",
            p.semaphores.len()
        ));
        host_outputs(ctx, &mut e)?;
    }
    e.l("return 0;");
    e.close();
    Ok(e.s)
}

/// The device helpers every GPU program of `ctx` calls, and its item coordinate tables.
fn print_helpers(ctx: &Ctx, e: &mut Emit) {
    // The ring's sum, difference and product wrap at 64 bits, as the evaluator's epilogues do; a body the evaluator runs
    // never leaves ±2⁶³, so on it they are the exact results.
    e.l("__device__ __forceinline__ long long add_ll(long long a, long long b) { return (long long)((unsigned long long)a + (unsigned long long)b); }");
    e.l("__device__ __forceinline__ long long sub_ll(long long a, long long b) { return (long long)((unsigned long long)a - (unsigned long long)b); }");
    e.l("__device__ __forceinline__ long long mul_ll(long long a, long long b) { return (long long)((unsigned long long)a * (unsigned long long)b); }");
    e.l("__device__ __forceinline__ long long min_ll(long long a, long long b) { return a < b ? a : b; }");
    e.l("__device__ __forceinline__ long long max_ll(long long a, long long b) { return a > b ? a : b; }");
    e.l("__device__ __forceinline__ long long floordiv_ll(long long a, long long b) { long long q = a / b; return (a % b != 0 && ((a < 0) != (b < 0))) ? q - 1 : q; }");
    e.l("__device__ __forceinline__ long long trunc_ll(long long v, int bits) { return bits >= 64 ? v : (long long)((unsigned long long)v << (64 - bits)) >> (64 - bits); }");
    // The evaluator's flat plane: `t` rows by the page's width; outside it, zero.
    e.l("template <typename T> __device__ __forceinline__ long long read_plane(const T* page, long long off, long long width, int t) {");
    e.l("    if (off < 0 || width == 0 || off >= (long long)t * width) return 0;");
    e.l("    const long long r = off / width, c = off % width;");
    e.l("    return (long long)page[(c / t) * (long long)t * t + r * t + (c % t)];");
    e.l("}");
    // Item coordinate tables, per block and section.
    for ((sec, site), list) in &ctx.coords {
        let name = format!("ITEM_b{}_s{}", ctx.block[site], sec.map_or("n".to_string(), |k| k.to_string()));
        let ys: Vec<String> = list.iter().map(|c| c.0.to_string()).chain(std::iter::once("0".into())).collect();
        let xs: Vec<String> = list.iter().map(|c| c.1.to_string()).chain(std::iter::once("0".into())).collect();
        e.l(format!("__device__ __constant__ int {name}_y[{}] = {{{}}};", ys.len(), ys.join(", ")));
        e.l(format!("__device__ __constant__ int {name}_x[{}] = {{{}}};", xs.len(), xs.join(", ")));
    }
}

/// The host's copy of every tensor, initialized from its fill or from `t<id>.txt` (one value a line) where a run supplies
/// one, in the tile-major order the device reads, and its device copy `d<id>`.
fn host_tensors(ctx: &Ctx, e: &mut Emit) -> Result<(), SpatialLowerError> {
    let (t, te) = (ctx.t, ctx.te);
    for tensor in &ctx.p.tensors {
        let id = tensor.id.idx();
        let ty = c_int(tensor.dtype)?;
        let words = tensor.tiles.rows_t * tensor.tiles.cols_t * te;
        let (rows, cols) = (tensor.elems.rows, tensor.elems.cols);
        e.l(format!("std::vector<{ty}> h{id}({words}, 0); {ty}* d{id};"));
        let fill: Vec<String> = match &tensor.init {
            TensorInit::Values(v) => v.iter().map(|x| x.to_string()).collect(),
            TensorInit::Pattern { modulus, shift } => (0..rows * cols).map(|i| ((i as i64 % modulus) + shift).to_string()).collect(),
            _ => Vec::new(),
        };
        if !fill.is_empty() {
            e.l(format!("static const long long fill{id}[{}] = {{{}}};", fill.len(), fill.join(", ")));
        }
        e.open("");
        e.l(format!("std::vector<long long> v({rows} * {cols}, 0);"));
        if !fill.is_empty() {
            e.l(format!("for (size_t i = 0; i < {} && i < v.size(); ++i) v[i] = fill{id}[i];", fill.len()));
        }
        e.l(format!("FILE* f = fopen(\"t{id}.txt\", \"r\"); if (f) {{ for (size_t i = 0; i < v.size(); ++i) {{ long long x; if (fscanf(f, \"%lld\", &x) != 1) break; v[i] = x; }} fclose(f); }}"));
        e.l(format!("for (int i = 0; i < {rows}; ++i) for (int j = 0; j < {cols}; ++j) h{id}[((long long)(i / {t}) * {} + j / {t}) * {te} + (i % {t}) * {t} + (j % {t})] = ({ty})v[(size_t)i * {cols} + j];", tensor.tiles.cols_t));
        e.close();
        e.l(format!("cudaMalloc(&d{id}, {words} * sizeof({ty})); cudaMemcpy(d{id}, h{id}.data(), {words} * sizeof({ty}), cudaMemcpyHostToDevice);"));
    }
    Ok(())
}

/// Every `Output` and `State` tensor copied back and printed as `OUT <tensor> <element> <value>`, row-major.
fn host_outputs(ctx: &Ctx, e: &mut Emit) -> Result<(), SpatialLowerError> {
    let (t, te) = (ctx.t, ctx.te);
    for tensor in ctx.p.tensors.iter().filter(|t| matches!(t.role, TensorRole::Output | TensorRole::State)) {
        let id = tensor.id.idx();
        let ty = c_int(tensor.dtype)?;
        let words = tensor.tiles.rows_t * tensor.tiles.cols_t * te;
        let (rows, cols) = (tensor.elems.rows, tensor.elems.cols);
        e.l(format!("cudaMemcpy(h{id}.data(), d{id}, {words} * sizeof({ty}), cudaMemcpyDeviceToHost);"));
        e.l(format!(
            "for (int i = 0; i < {rows}; ++i) for (int j = 0; j < {cols}; ++j) printf(\"OUT {id} %d %lld\\n\", i * {cols} + j, (long long)h{id}[((long long)(i / {t}) * {} + j / {t}) * {te} + (i % {t}) * {t} + (j % {t})]);",
            tensor.tiles.cols_t
        ));
    }
    Ok(())
}
