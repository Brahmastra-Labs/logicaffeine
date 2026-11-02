//! `size_buffers` — buffering as a GENERAL pass: depths from a rate model under the local-memory budget,
//! a deterministic non-overlapping layout, typed refusals.
//!
//! A FIFO's depth is what Little's law asks for — the pages its producer must keep in flight to cover the
//! own latency at the rate its consumer drains them, plus the one being filled — floored at two and capped
//! by the site's memory, which is spent on whichever buffer is shortest. The output FIFO holds one page per
//! sub-block; the spill buffer holds every sub-block's partial in the accumulator's width. Offsets
//! are first-fit in buffer order per site, so group members (identical buffer sets) land at identical
//! offsets — a multicast writes one local address on every receiver.

use super::SpatialReject;
use crate::codegen_sva::fabric_target::Precision;
use crate::spatial::cost::{is_streaming, quantities_for_gemm};
use crate::spatial::ir::*;
use crate::spatial::target::{precision_bytes, SpatialTarget};

/// Bytes a plan's buffers need on one site (the legality formula `tile` uses; conservative: depth 2).
pub fn footprint_bytes_for_plan(
    p: &SpatialProgram,
    plan: &BlockPlan,
    target: &SpatialTarget,
) -> usize {
    let out = p
        .output_tensor()
        .map(|t| t.dtype)
        .unwrap_or(Precision::Bf16);
    let operand = p
        .stages
        .iter()
        .find_map(|s| match &s.kind {
            StageKind::Compute(ComputeOp::MatmulTiles { operand, .. }) => Some(*operand),
            _ => None,
        })
        .unwrap_or(out);
    let exact = p.numerics == NumericsPolicy::Exact;
    let in_place = super::tile::finishes_in_place(p, plan, target);
    let tile = target.tile.elems();
    let ob = tile * precision_bytes(operand);
    let outb = tile * precision_bytes(out);
    if is_streaming(plan) {
        return 2 * ob + 2 * ob + 2 * outb;
    }
    let (pm, pn) = (plan.per_site_m, plan.per_site_n);
    let BlockShape { osh, osw, ibw } = plan.block;
    let sub = (pm / osh.max(1)) * (pn / osw.max(1));
    // Matches the ring depth `apply_plan` lays out: three stages on a register-accumulator target,
    // where one block per site leaves no other warps to hide a load behind.
    let depth = 2;
    let in0 = depth * pm * ibw * ob;
    let in1 = depth * ibw * pn * ob;
    // ★ THE EPILOGUE COSTS A FIXED SCRATCH, NOT THE BLOCK, where the accumulator is register-
    // resident: each warp writes its own tiles straight to global through one tile-sized slot, so
    // the output never stages through a ring at all. Charging the block's whole output —
    // `osh · osw` tiles at the accumulator's width, double-buffered — is what stopped the planner
    // affording a block tile worth running: 16 tiles is 32 KiB of a 99 KiB budget, and the 64 a
    // competitive GEMM wants would be 128 KiB.
    let outbuf =
        if target.accumulator_in_registers() && target.accumulator_lane_grid(osh, osw).is_some() {
            // One tile per accumulator lane — charged only where the epilogue really fuses, so the
            // plan the planner PRICED is the plan the allocator lays out.
            target.accumulator_lanes() * outb
        } else if in_place {
            sub * osh * osw * outb
        } else {
            sub.max(2) * osh * osw * outb
        };
    // ★ AND THE K-BLOCK SPILL COSTS NOTHING WHERE THE ACCUMULATOR NEVER LEAVES THE REGISTERS —
    // the same reasoning as the epilogue above, one buffer along, and it was missing.
    //
    // `tile::apply_plan` creates this buffer only `if nb > 1 && !is_streaming(&plan) &&
    // !target.accumulator_in_registers()`: a GPU's `wmma` fragments hold the partial sums across the
    // whole K loop, so there is no spill to stage. This function priced it on `num_blocks > 1` alone,
    // so the two disagreed by exactly one condition — and the planner charged 64 KiB, out of a 99 KiB
    // budget, for a buffer the very next pass declines to create.
    //
    // The cost was a K depth. At 2048³ on an RTX 3060 that left the 128×128 output tile affordable at
    // exactly ONE inner width (16 elements) when four times that fits, so the persistent kernel ran its
    // K loop 128 times where the knob lattice's best rung runs it 64 — for identical arithmetic.
    // (`is_streaming` is already handled by the early return at the top of this function.)
    // An intermediate that shares the output's memory costs none of its own.
    let interm = if plan.inner.num_blocks > 1 && !target.accumulator_in_registers() && !in_place {
        let dt = if exact {
            plan.acc.interm_precision()
        } else {
            operand
        };
        sub * osh * osw * tile * precision_bytes(dt)
    } else {
        0
    };
    in0 + in1 + outbuf + interm
}

/// Bytes of the busiest site's buffers as the program stands (placed) or as its plan implies (template).
pub fn footprint_bytes(p: &SpatialProgram, target: &SpatialTarget) -> usize {
    if p.placement.is_some() {
        let mut per_site: std::collections::BTreeMap<Site, usize> = Default::default();
        for b in &p.buffers {
            if b.space == MemSpace::Acc || b.alias_of.is_some() {
                continue;
            }
            *per_site
                .entry(b.site.unwrap_or(Site::new(0, 0)))
                .or_default() += p.buffer_bytes(b);
        }
        return per_site.values().copied().max().unwrap_or(0);
    }
    let gemm = p
        .stages
        .iter()
        .any(|s| matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. })));
    match p.work.plan {
        Some(plan) if gemm => footprint_bytes_for_plan(p, &plan, target),
        _ => p.buffers.iter().map(|b| p.buffer_bytes(b)).sum(),
    }
}

/// The `size_buffers` pass.
/// ★ A MULTICAST GROUP'S MEMBER BUFFERS ARE ONE BUFFER, REPLICATED ACROSS A RECTANGLE.
///
/// The hardware lands a broadcast at ONE L1 address on every receiver, so every member must agree on
/// depth and page size — `IrViolation::GroupMemberMismatch` refuses a program where they do not. Sizing
/// them independently is therefore not merely suboptimal, it is invalid, and so is taking a page back
/// from one of them under memory pressure: the shrink loop below does exactly that, one buffer at a
/// time, and on a movement-slow target it produced a program the IR rejected outright.
///
/// The common depth is the MINIMUM, because a shrink happened for a reason — the site could not afford
/// the pages — and every member sits in the same L1 budget.
fn unify_group_depths(p: &mut SpatialProgram) {
    let Some(pl) = p.placement.clone() else {
        return;
    };
    for g in &pl.groups {
        let members: Vec<BufferId> = p
            .stages
            .iter()
            .filter(|s| p.stage_group(s) == Some(g.id))
            .filter_map(|s| s.outs.first().copied())
            .collect();
        let Some(depth) = members
            .iter()
            .filter_map(|b| match p.buffer(*b).kind {
                BufferKind::Fifo { depth_pages, .. } => Some(depth_pages),
                _ => None,
            })
            .min()
        else {
            continue;
        };
        for b in members {
            if let BufferKind::Fifo { page_tiles, .. } = p.buffers[b.idx()].kind {
                p.buffers[b.idx()].kind = BufferKind::Fifo {
                    depth_pages: depth,
                    page_tiles,
                };
            }
        }
    }
}

pub fn size_buffers(
    p: &SpatialProgram,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    if p.memory.is_some() {
        return Ok(p.clone());
    }
    let Some(pl) = &p.placement else {
        return Err(SpatialReject::WrongPhase {
            pass: "size_buffers",
            expected: "Placed",
            found: p.phase(),
        });
    };
    let plan = p.work.plan.expect("placed programs are tiled");
    let exact = p.numerics == NumericsPolicy::Exact;
    // Accumulator budget.
    for s in &p.stages {
        if let StageKind::Compute(ComputeOp::MatmulTiles { block, acc, .. }) = &s.kind {
            let available = target.effective_acc_tiles(*acc, exact);
            if block.acc_tiles() > available {
                return Err(SpatialReject::AccTilesExceeded {
                    needed: block.acc_tiles(),
                    available,
                    acc: *acc,
                });
            }
        }
    }
    let mut q = p.clone();
    // ── Depths from a rate model, per buffer, under the site's memory budget ─────────────────────────────
    //
    // A bounded buffer exists to keep its consumer fed while its producer works. Little's law gives the
    // depth that does it: the producer needs `latency / service` pages in flight, plus the one it is filling.
    // Below that the consumer stalls every round; above it the extra page buys nothing and costs memory that
    // another buffer could use. Two is the floor (a page landing while the previous one is read), never a
    // default.
    //
    // The budget is then spent greedily on whichever buffer's stall the next page removes most, so the
    // memory goes where the pipeline is actually short rather than uniformly.
    if !is_streaming(&plan) {
        if let Some(out) = p.output_tensor() {
            let fidelity = match target.matmul_cap() {
                Some(crate::spatial::target::ComputeCap::MatmulTile { fidelity, .. }) => *fidelity,
                _ => crate::spatial::target::Fidelity::Exact,
            };
            let fused = !pl.groups.is_empty();
            let qn = quantities_for_gemm(
                out.tiles.rows_t,
                out.tiles.cols_t,
                p.work.k_tiles,
                &plan,
                fused,
                fidelity,
                target,
            );
            let m = &target.cycle;
            let nb = plan.inner.num_blocks.max(1) as f64;
            let sub = ((plan.per_site_m / plan.block.osh.max(1))
                * (plan.per_site_n / plan.block.osw.max(1)))
            .max(1) as f64;
            // What the writer spends per output page.
            let compute_per_page = m.compute_cycles(&qn) / (nb * sub);
            // Only a buffer whose producer can RUN AHEAD benefits from depth. The operand FIFOs can: their
            // producer is a movement engine fetching the next block while the compute engine works on this
            // one. The output FIFO cannot: its producer is that same compute engine, which never has more
            // than one block of sub-blocks outstanding, so a page past the sub-block count is memory spent
            // on nothing. The spill buffer's depth is a correctness constraint, not a rate choice.
            let want = |b: &Buffer| -> usize {
                let BufferKind::Fifo { page_tiles, .. } = b.kind else {
                    return 0;
                };
                if !b.name.starts_with("in") {
                    return 0;
                }
                // The producer cannot refill a page until the consumer's release has travelled back to it,
                // so what the depth must cover is the whole credit loop — the forward fetch AND the return
                // trip — divided by the service it is hiding. Sizing on the fetch alone and adding a page is
                // the same number only while the return fits inside one service time; below that the
                // producer stalls every round on a buffer that looks correctly sized. This is the
                // bandwidth-delay product `codegen_sva::choreography::credit_depth` proves two-sided on the
                // other spatial target, so both targets answer this question the same way.
                //
                // The page being filled is not added on top: the loop is acquire credit, fill, push, consume,
                // release, so that page is already inside the round trip and counting it again double-counts.
                //
                // Optimistic where packing is expensive: the true forward leg runs from credit acquired to
                // page visible, so it also carries pack and push, which this DRAM term omits. The floor of
                // two absorbs it at every shape measured; `move_cycles` is where the exact leg lives.
                // The inputs are the TARGET's declared latencies, not the cost model's fitted ones. A depth
                // is a resource decision, and the constants were fitted against a cycle counter that turned
                // out not to track work at all — so a depth derived from them would be a function of noise
                // wearing the costume of a rate model. A part that has not declared its latencies gets the
                // vendor's double-buffering discipline, which is what every program we have run actually used.
                let Some(c) = target.site.credit else {
                    return 2;
                };
                if c.service_page == 0 {
                    return 2;
                }
                let loop_cycles = page_tiles as u64 * c.fetch_tile as u64 + c.credit_return as u64;
                (loop_cycles.div_ceil(c.service_page as u64) as usize).max(2)
            };
            let _ = compute_per_page;
            let mut trial = q.clone();
            for b in trial.buffers.iter_mut() {
                if let BufferKind::Fifo {
                    depth_pages,
                    page_tiles,
                } = b.kind
                {
                    let d = want(b).max(depth_pages);
                    b.kind = BufferKind::Fifo {
                        depth_pages: d,
                        page_tiles,
                    };
                }
            }
            unify_group_depths(&mut trial);
            // Spend only what fits; a buffer whose extra pages do not fit keeps the depth it had.
            while footprint_bytes(&trial, target) > target.site.local_mem_bytes {
                // Give back the page that costs the most bytes per unit of stall it was buying.
                let victim = trial
                    .buffers
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| b.name.starts_with("in") && matches!(b.kind, BufferKind::Fifo { depth_pages, .. } if depth_pages > 2))
                    .max_by_key(|(_, b)| p.buffer_bytes(b) / b.kind.depth_pages().max(1))
                    .map(|(i, _)| i);
                match victim {
                    Some(i) => {
                        if let BufferKind::Fifo {
                            depth_pages,
                            page_tiles,
                        } = trial.buffers[i].kind
                        {
                            trial.buffers[i].kind = BufferKind::Fifo {
                                depth_pages: depth_pages - 1,
                                page_tiles,
                            };
                        }
                        // Whatever we took, take it from every member of the group that buffer belongs to.
                        unify_group_depths(&mut trial);
                    }
                    None => break,
                }
            }
            if footprint_bytes(&trial, target) <= target.site.local_mem_bytes {
                q = trial;
            }
        }
    }
    lay_out(q, target)
}

/// Re-size every operand FIFO to `depth` pages and lay the site out again.
///
/// Depth is the one buffering choice the cost model cannot yet score, so it is also the one that has to be
/// MEASURED rather than argued. This is the instrument that measures it: it takes a program the passes have
/// already sized and re-runs the layout at a pinned depth, so two runs differ in exactly that number.
pub fn resize_operand_fifos(
    p: &SpatialProgram,
    target: &SpatialTarget,
    depth: usize,
) -> Result<SpatialProgram, SpatialReject> {
    let mut q = p.clone();
    for b in q.buffers.iter_mut() {
        if let BufferKind::Fifo { page_tiles, .. } = b.kind {
            if b.name.starts_with("in") {
                b.kind = BufferKind::Fifo {
                    depth_pages: depth,
                    page_tiles,
                };
            }
        }
    }
    q.memory = None;
    lay_out(q, target)
}

fn lay_out(q: SpatialProgram, target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
    let mut q = q;
    // Layout: first-fit per site in buffer order; aliases share their target's region.
    let mut buffer_at = vec![
        BufferAt {
            site: Site::new(0, 0),
            space: MemSpace::Local,
            offset_bytes: 0,
            bytes: 0
        };
        q.buffers.len()
    ];
    let mut cursor: std::collections::BTreeMap<Site, u32> = Default::default();
    // ★ A MULTICAST GROUP'S MEMBERS ARE PLACED FIRST, AND AT ONE ADDRESS.
    //
    // The hardware writes a broadcast to the SAME L1 offset on every site in the rectangle — there is one
    // destination address in the packet, not one per receiver — and the IR refuses a program where they
    // differ (`GroupMemberMismatch`: "multicast lands at one L1 address"). First-fitting each site
    // independently cannot honour that: a sender site holds a Load's buffer where a receiver holds a
    // Recv's, the cursors diverge, and the group lands at two addresses.
    //
    // So group buffers are laid down before anything else, at an offset every member site can take, and
    // the per-site cursors all advance past it. Deterministic: groups in id order, then buffers in id
    // order, exactly as before.
    let mut placed = vec![false; q.buffers.len()];
    if let Some(pl) = q.placement.clone() {
        for g in &pl.groups {
            let members: Vec<BufferId> = q
                .stages
                .iter()
                .filter(|s| q.stage_group(s) == Some(g.id))
                .filter_map(|s| s.outs.first().copied())
                .collect();
            if members.is_empty() {
                continue;
            }
            let bytes = members
                .iter()
                .map(|b| q.buffer_bytes(q.buffer(*b)) as u32)
                .max()
                .unwrap_or(0);
            let off = members
                .iter()
                .filter_map(|b| q.buffer(*b).site)
                .map(|site| cursor.get(&site).copied().unwrap_or(0))
                .max()
                .unwrap_or(0);
            for b in &members {
                let buf = q.buffer(*b);
                let site = buf.site.unwrap_or(Site::new(0, 0));
                buffer_at[b.idx()] = BufferAt {
                    site,
                    space: buf.space,
                    offset_bytes: off,
                    bytes: q.buffer_bytes(buf) as u32,
                };
                placed[b.idx()] = true;
                cursor.insert(site, off + bytes);
            }
        }
    }
    for b in &q.buffers {
        if placed[b.id.idx()] {
            continue;
        }
        let site = b.site.unwrap_or(Site::new(0, 0));
        let bytes = q.buffer_bytes(b) as u32;
        let offset = match b.alias_of {
            Some(a) => buffer_at[a.idx()].offset_bytes,
            None if b.space == MemSpace::Acc => 0,
            None => {
                let c = cursor.entry(site).or_insert(0);
                let off = *c;
                *c += bytes;
                off
            }
        };
        buffer_at[b.id.idx()] = BufferAt {
            site,
            space: b.space,
            offset_bytes: offset,
            bytes,
        };
    }
    // The budget is what the site can be CONFIGURED with, not its default. An Ampere block opts in
    // to ~100 KiB of shared memory and the CUDA lowering emits that path
    // (`cudaFuncSetAttribute(…MaxDynamicSharedMemorySize…)`), so measuring against the 48 KiB
    // static default rejects plans the device runs perfectly well — and on a GEMM those are exactly
    // the block tiles big enough to be worth running. The tiling pass filters candidates against
    // the same number, so the two cannot disagree about what fits.
    let budget = target.max_local_mem_bytes();
    for (site, used) in &cursor {
        if *used as usize > budget {
            return Err(SpatialReject::LocalMemExceeded {
                site: *site,
                needed_bytes: *used as usize,
                budget_bytes: budget,
            });
        }
    }
    q.memory = Some(MemoryMap { buffer_at });
    q.validate()?;
    Ok(q)
}
