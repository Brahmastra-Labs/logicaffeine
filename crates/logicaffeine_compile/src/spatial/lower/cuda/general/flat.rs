//! The spatial IR as ONE block: every engine of every site, in an order fixed when the program is compiled.
//!
//! A program whose rings and accumulators fit one block's shared memory needs no handshake at run time: its flow control
//! can be decided when it is compiled. The engines' choreographies run here, at compile time, under the evaluator's
//! discipline — each engine runs ahead until a guard holds it (a reserve with no free page, a wait with no filled one, a
//! semaphore wait not yet met), then the next engine runs — which fixes an order in which every guard holds, and with it
//! each page's ring slot and each loop counter. The kernel is that order: each op that moves or computes data printed as
//! the general program prints it, over every thread of the block, its slots and counters constants; a reserve, a push, a
//! wait, a pop and a semaphore print nothing, since the order they decided is the order the kernel runs in. A block
//! barrier follows each op whose writes the next one may read.

use std::collections::{BTreeMap, BTreeSet};

use super::{align16, c_int, context, emit_op, host_outputs, host_tensors, print_helpers, site_memory, var, Ctx};
use crate::spatial::ir::*;
use crate::spatial::lower::cuda::{CudaArtifact, Emit, PersistentBudget};
use crate::spatial::lower::{Census, SpatialLowerError};
use crate::spatial::target::SpatialTarget;

/// Threads of the one block.
const THREADS: usize = 512;

/// One op of one engine, its loops unrolled: the loop counters it runs under.
#[derive(Clone)]
struct Inst {
    top: TaggedOp,
    bind: Vec<(LoopVar, usize)>,
}

/// An engine's ops with every loop unrolled, each op beside the counters it runs under.
fn unroll(ops: &[TaggedOp], bind: &mut Vec<(LoopVar, usize)>, out: &mut Vec<Inst>) -> Result<(), SpatialLowerError> {
    for top in ops {
        match &top.op {
            ChoreoOp::Loop { count: LoopCount::Trip { .. }, .. } => {
                return Err(SpatialLowerError::Unsupported("a loop whose count the program reads from its data (a trip) on the GPU"))
            }
            ChoreoOp::Loop { var, count, body } => {
                for i in 0..count.bound() {
                    bind.push((*var, i));
                    unroll(body, bind, out)?;
                    bind.pop();
                }
            }
            _ => out.push(Inst { top: top.clone(), bind: bind.clone() }),
        }
    }
    Ok(())
}

/// A data op in the block's order: the engine that runs it, the op, and the slot each of its engine's rings writes and
/// reads when it runs.
struct Step {
    engine: usize,
    inst: Inst,
    slots: BTreeMap<usize, (usize, usize)>,
}

/// The order every engine's ops run in, decided by running their flow control: each engine runs until a guard holds
/// it, then the next, over and over, until every engine is done — or no engine can move, a deadlock the schedule has.
fn schedule(ctx: &Ctx, engines: &[(Site, Vec<Inst>)]) -> Result<Vec<Step>, SpatialLowerError> {
    let p = ctx.p;
    let depth = |k: usize| ctx.rings[&BufferId::new(k)].0;
    // Per site and ring: pages pushed and popped. Per engine and ring: pages it reserved and pages it read.
    let mut pushed: BTreeMap<(Site, usize), usize> = BTreeMap::new();
    let mut popped: BTreeMap<(Site, usize), usize> = BTreeMap::new();
    let mut wres: Vec<BTreeMap<usize, usize>> = vec![BTreeMap::new(); engines.len()];
    let mut rctr: Vec<BTreeMap<usize, usize>> = vec![BTreeMap::new(); engines.len()];
    let mut slots: Vec<BTreeMap<usize, (usize, usize)>> = vec![BTreeMap::new(); engines.len()];
    let mut sems: BTreeMap<(SemId, Site), usize> = BTreeMap::new();
    for s in &p.semaphores {
        for site in &s.sites {
            sems.insert((s.id, *site), s.init as usize);
        }
    }
    let mut pc = vec![0usize; engines.len()];
    let mut out = Vec::new();
    loop {
        let mut moved = false;
        for (ei, (site, insts)) in engines.iter().enumerate() {
            while let Some(inst) = insts.get(pc[ei]) {
                let ring = |b: &BufferId| ctx.ring(*b);
                let go = match &inst.top.op {
                    ChoreoOp::Reserve { pages, .. } | ChoreoOp::Push { pages, .. } | ChoreoOp::Wait { pages, .. } | ChoreoOp::Pop { pages, .. }
                        if *pages != 1 =>
                    {
                        return Err(SpatialLowerError::Unsupported("a FIFO op that moves more than one page at once on the GPU"))
                    }
                    ChoreoOp::Reserve { buf, .. } => {
                        let k = ring(buf);
                        let w = wres[ei].get(&k).copied().unwrap_or(0);
                        let ok = w - popped.get(&(*site, k)).copied().unwrap_or(0) < depth(k);
                        if ok {
                            slots[ei].entry(k).or_default().0 = w % depth(k);
                            wres[ei].insert(k, w + 1);
                        }
                        ok
                    }
                    ChoreoOp::Push { buf, .. } => {
                        *pushed.entry((*site, ring(buf))).or_default() += 1;
                        true
                    }
                    ChoreoOp::Wait { buf, .. } => {
                        let k = ring(buf);
                        let r = rctr[ei].get(&k).copied().unwrap_or(0);
                        let ok = pushed.get(&(*site, k)).copied().unwrap_or(0) > r;
                        if ok {
                            slots[ei].entry(k).or_default().1 = r % depth(k);
                        }
                        ok
                    }
                    ChoreoOp::Pop { buf, .. } => {
                        let k = ring(buf);
                        *popped.entry((*site, k)).or_default() += 1;
                        *rctr[ei].entry(k).or_default() += 1;
                        true
                    }
                    ChoreoOp::SemSet { sem, value } => {
                        let v = match value {
                            SemValue::Invalid => 0,
                            SemValue::Valid => 1,
                            SemValue::Count(n) => *n,
                        };
                        sems.insert((*sem, *site), v);
                        true
                    }
                    ChoreoOp::SemInc { sem, at, by } => {
                        *sems.entry((*sem, *at)).or_default() += *by;
                        true
                    }
                    ChoreoOp::SemWait { sem, until } => {
                        let v = sems.get(&(*sem, *site)).copied().unwrap_or(0);
                        match until {
                            SemCond::Equals(n) => v == *n,
                            SemCond::AtLeast(n) => v >= *n,
                        }
                    }
                    ChoreoOp::McastSend { .. } | ChoreoOp::McastSetValid { .. } => {
                        return Err(SpatialLowerError::Unsupported("a multicast on the GPU (sm_86 has no cluster multicast)"))
                    }
                    ChoreoOp::Barrier { .. } => return Err(SpatialLowerError::Unsupported("a group barrier on the GPU")),
                    ChoreoOp::ReadBarrier | ChoreoOp::WriteBarrier | ChoreoOp::Flush { .. } | ChoreoOp::ReleaseAcc => true,
                    ChoreoOp::AcquireAcc { .. } | ChoreoOp::ReadTiles { .. } | ChoreoOp::WriteTiles { .. } | ChoreoOp::Compute { .. } | ChoreoOp::PackAcc { .. } => {
                        out.push(Step { engine: ei, inst: inst.clone(), slots: slots[ei].clone() });
                        true
                    }
                    ChoreoOp::Loop { .. } => unreachable!("unrolled"),
                };
                if !go {
                    break;
                }
                pc[ei] += 1;
                moved = true;
            }
        }
        if pc.iter().zip(engines).all(|(pc, (_, insts))| *pc >= insts.len()) {
            return Ok(out);
        }
        if !moved {
            return Err(SpatialLowerError::Unsupported("a schedule whose engines all wait on each other"));
        }
    }
}

/// The rings an op reads or writes.
fn rings_of(ctx: &Ctx, op: &ChoreoOp) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    match op {
        ChoreoOp::ReadTiles { into, at, .. } => {
            out.insert(ctx.ring(*into));
            out.extend(at.indirect.iter().map(|i| ctx.ring(i.index)));
        }
        ChoreoOp::WriteTiles { from, at, .. } => {
            out.insert(ctx.ring(*from));
            out.extend(at.indirect.iter().map(|i| ctx.ring(i.index)));
        }
        ChoreoOp::PackAcc { to, .. } => {
            out.insert(ctx.ring(*to));
        }
        ChoreoOp::Compute { ins, acc_reload, .. } => {
            out.extend(ins.iter().map(|b| ctx.ring(*b)));
            out.extend(acc_reload.iter().map(|b| ctx.ring(*b)));
        }
        _ => {}
    }
    out
}

/// What a step of the one-block program reads and writes: a ring, an engine's accumulator, a tensor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Obj {
    Ring(usize),
    Acc(usize),
    Tensor(usize),
}

/// The objects a step's op touches, each with whether it writes it.
fn accesses(ctx: &Ctx, op: &ChoreoOp, engine: usize) -> Vec<(Obj, bool)> {
    let index = |at: &TileAddr| at.indirect.iter().map(|i| (Obj::Ring(ctx.ring(i.index)), false)).collect::<Vec<_>>();
    match op {
        ChoreoOp::ReadTiles { tensor, into, at, .. } => {
            let mut v = vec![(Obj::Tensor(tensor.idx()), false), (Obj::Ring(ctx.ring(*into)), true)];
            v.extend(index(at));
            v
        }
        ChoreoOp::WriteTiles { from, tensor, at, .. } => {
            let mut v = vec![(Obj::Ring(ctx.ring(*from)), false), (Obj::Tensor(tensor.idx()), true)];
            v.extend(index(at));
            v
        }
        ChoreoOp::AcquireAcc { .. } => vec![(Obj::Acc(engine), true)],
        ChoreoOp::Compute { ins, acc_reload, .. } => {
            let mut v: Vec<(Obj, bool)> = ins.iter().chain(acc_reload).map(|b| (Obj::Ring(ctx.ring(*b)), false)).collect();
            v.push((Obj::Acc(engine), true));
            v
        }
        ChoreoOp::PackAcc { to, accumulate, .. } => {
            let mut v = vec![(Obj::Acc(engine), false), (Obj::Ring(ctx.ring(*to)), true)];
            if *accumulate {
                v.push((Obj::Ring(ctx.ring(*to)), false));
            }
            v
        }
        _ => Vec::new(),
    }
}

/// The one-block program of `p`, or `None` where its rings and accumulators do not fit one block's shared memory.
pub(super) fn lower_flat(p: &SpatialProgram, target: &SpatialTarget) -> Result<Option<CudaArtifact>, SpatialLowerError> {
    let (ctx, sites) = context(p)?;
    let sched = p.schedule.as_ref().expect("a context is of a scheduled program");
    let mut site_progs: Vec<(Site, Vec<Vec<TaggedOp>>)> = Vec::new();
    let mut engines: Vec<(Site, Vec<Inst>)> = Vec::new();
    for s in &sites {
        let progs: Vec<Vec<TaggedOp>> = sched.programs.iter().filter(|pr| pr.site == *s).map(|pr| pr.ops.clone()).collect();
        for ops in &progs {
            let mut insts = Vec::new();
            unroll(ops, &mut Vec::new(), &mut insts)?;
            engines.push((*s, insts));
        }
        site_progs.push((*s, progs));
    }
    let (per_site, acc_tiles_max) = site_memory(&ctx, &site_progs);
    let acc_bytes = align16(acc_tiles_max * ctx.te * 8);
    let site_at: Vec<usize> = per_site.iter().scan(0, |at, m| {
        let here = *at;
        *at += m.arena;
        Some(here)
    })
    .collect();
    let arena: usize = per_site.iter().map(|m| m.arena).sum();
    let dyn_bytes = arena + acc_bytes * engines.len();
    if dyn_bytes > target.max_local_mem_bytes() {
        return Ok(None);
    }
    let steps = schedule(&ctx, &engines)?;
    let budget = PersistentBudget {
        smem_bytes: dyn_bytes,
        dynamic_opt_in: dyn_bytes > 48 * 1024,
        rings_dynamic: true,
        dyn_bytes,
        regs_est: 64,
        blocks_per_sm: 1,
        threads: THREADS,
    };
    let block_of: BTreeMap<Site, usize> = sites.iter().enumerate().map(|(i, s)| (*s, i)).collect();
    let print = |bench: bool| -> Result<String, SpatialLowerError> {
        let mut e = Emit { s: String::new(), ind: 0 };
        e.l(format!("// {} — one block, every engine in the order its flow control fixes; printed from the spatial IR.", p.name));
        for inc in ["cstdio", "cstdlib", "cstring", "vector"] {
            e.l(format!("#include <{inc}>"));
        }
        e.l(format!("#define GSZ {THREADS}"));
        e.l("#define GBAR() __syncthreads()");
        for (b, (depth, page, _)) in &ctx.rings {
            e.l(format!("#define DEPTH_{} {depth}", b.idx()));
            e.l(format!("#define PAGE_{} {page}", b.idx()));
        }
        print_helpers(&ctx, &mut e);
        let params: Vec<String> = p
            .tensors
            .iter()
            .map(|t| c_int(t.dtype).map(|ty| format!("{ty}* __restrict__ t{}", t.id.idx())))
            .collect::<Result<_, _>>()?;
        e.open(format!("__global__ __launch_bounds__(GSZ, 1) void logos_spatial_flat({})", params.join(", ")));
        e.l("const int gtid = threadIdx.x;");
        e.l("extern __shared__ __align__(16) char logos_dyn_smem[];");
        for ei in 0..engines.len() {
            e.l(format!("long long* acc_e{ei} = reinterpret_cast<long long*>(logos_dyn_smem + {});", arena + ei * acc_bytes));
            e.l(format!("int acc_tiles_e{ei} = 0; (void)acc_tiles_e{ei};"));
        }
        // ★ A BARRIER ONLY WHERE A STEP DEPENDS ON ONE BEFORE IT. The block's threads each take their own elements of an
        // op, so an op that reads what an earlier one wrote — or writes what it read — since the last barrier must
        // wait for every thread; one that touches none of it need not. Each op's own internal barriers stay.
        let mut pending: BTreeMap<Obj, bool> = BTreeMap::new();
        for step in &steps {
            let touched = accesses(&ctx, &step.inst.top.op, step.engine);
            if touched.iter().any(|(o, w)| pending.get(o).is_some_and(|pw| *w || *pw)) {
                e.l("GBAR();");
                pending.clear();
            }
            for (o, w) in touched {
                *pending.entry(o).or_insert(false) |= w;
            }
            let (site, _) = &engines[step.engine];
            let mem = &per_site[block_of[site]];
            let base = site_at[block_of[site]];
            e.open("");
            e.l(format!("long long* acc = acc_e{0}; int& acc_tiles = acc_tiles_e{0}; (void)acc; (void)acc_tiles;", step.engine));
            for k in rings_of(&ctx, &step.inst.top.op) {
                let ty = ctx.rings[&BufferId::new(k)].2;
                let at = mem.layout.get(&k).copied().ok_or(SpatialLowerError::Unsupported("an op on a ring its site does not lay out"))?;
                let (w, r) = step.slots.get(&k).copied().unwrap_or((0, 0));
                e.l(format!(
                    "{ty}* ring_{k} = reinterpret_cast<{ty}*>(logos_dyn_smem + {}); const int wslot_{k} = {w}, rslot_{k} = {r}; (void)wslot_{k}; (void)rslot_{k};",
                    base + at
                ));
            }
            let mut scope = BTreeSet::new();
            for (v, i) in &step.inst.bind {
                e.l(format!("const int {} = {i};", var(*v)));
                scope.insert(*v);
            }
            // The op's own closing barrier gives way to the dependence above.
            let mut op = Emit { s: String::new(), ind: e.ind };
            emit_op(&ctx, &mut op, &step.inst.top, &scope, *site)?;
            let body = op.s.trim_end_matches('\n');
            let body = match body.rsplit_once('\n') {
                Some((head, last)) if last.trim() == "GBAR();" => head,
                None if body.trim() == "GBAR();" => "",
                _ => body,
            };
            if !body.is_empty() {
                e.s.push_str(body);
                e.s.push('\n');
            }
            e.close();
        }
        e.close();
        e.l("");
        e.open("int main()");
        host_tensors(&ctx, &mut e)?;
        let args: Vec<String> = p.tensors.iter().map(|t| format!("d{}", t.id.idx())).collect();
        let args = args.join(", ");
        e.l(format!("const int DYN = {dyn_bytes};"));
        if budget.dynamic_opt_in {
            e.l("cudaFuncSetAttribute(logos_spatial_flat, cudaFuncAttributeMaxDynamicSharedMemorySize, DYN);");
        }
        if bench {
            e.l(format!("logos_spatial_flat<<<1, GSZ, DYN>>>({args}); cudaDeviceSynchronize();"));
            e.l("cudaEvent_t e0, e1; cudaEventCreate(&e0); cudaEventCreate(&e1);");
            e.l("const int ITERS = 200; float best = 1e30f;");
            e.open("for (int it = 0; it < ITERS; ++it)");
            e.l(format!("cudaEventRecord(e0); logos_spatial_flat<<<1, GSZ, DYN>>>({args}); cudaEventRecord(e1); cudaEventSynchronize(e1);"));
            e.l("float ms = 0; cudaEventElapsedTime(&ms, e0, e1); if (ms < best) best = ms;");
            e.close();
            e.l("{ cudaError_t err = cudaGetLastError(); if (err != cudaSuccess) { fprintf(stderr, \"%s\\n\", cudaGetErrorString(err)); return 1; } }");
            e.l("printf(\"US_PER_STEP %.3f\\n\", best * 1000.0f);");
        } else {
            e.l(format!("logos_spatial_flat<<<1, GSZ, DYN>>>({args});"));
            e.l("cudaError_t err = cudaDeviceSynchronize();");
            e.l("if (err != cudaSuccess) { fprintf(stderr, \"%s\\n\", cudaGetErrorString(err)); return 1; }");
            host_outputs(&ctx, &mut e)?;
        }
        e.l("return 0;");
        e.close();
        Ok(e.s)
    };
    Ok(Some(CudaArtifact { program_cu: print(false)?, bench_cu: print(true)?, budget, census: Census::of(p), general: None }))
}
