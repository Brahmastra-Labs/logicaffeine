//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! BUFFERS LOCK — buffering is a GENERAL pass: depths from a rate model under the target's memory budget.
//!
//! `size_buffers` fixes every FIFO's depth (double-buffered by default, deeper when the rate model says
//! movement outruns compute), sizes the spill buffer to the accumulator width, lays buffers out in local
//! memory deterministically without overlap, and refuses typed when a site's budget or accumulator file is
//! exceeded. This lock proves:
//!   A. depths: in0 = 2 pages of (per-site rows × inner block) tiles, in1 = 2 pages of (inner block × per-site
//!      columns), out = one page per sub-block; streamed plans use 2 single-tile pages; a spill buffer exists
//!      iff the plan has more than one K block, in the accumulator's width;
//!   B. a shrunken budget is refused with the exact bytes needed; an oversized sub-block is refused naming the
//!      accumulator policy; depth never drops below 2 and rises to 3 under a movement-slow model;
//!   C. offsets are deterministic, non-overlapping per site, identical across group members;
//!   D. the footprint equals the circular-buffer bytes the vendor host allocates for the same configuration;
//!   E. the evaluated occupancy never exceeds any buffer's depth (the choreography respects the sizes).
//!
//! Run: `cargo test -p logicaffeine-tests --test spatial_buffers_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::eval::eval_choreography_traced;
use logicaffeine_compile::spatial::ir::{
    AccPolicy, BlockShape, BufferKind, ComputeOp, NumericsPolicy, StageKind,
};
use logicaffeine_compile::spatial::passes::fuse_readers::fuse_readers;
use logicaffeine_compile::spatial::passes::place::place;
use logicaffeine_compile::spatial::passes::size_buffers::{footprint_bytes, size_buffers};
use logicaffeine_compile::spatial::passes::tile::tile;
use logicaffeine_compile::spatial::passes::SpatialReject;
use logicaffeine_compile::spatial::pipeline::{
    compile_to_spatial, run_passes, template_from_source,
};
use logicaffeine_compile::spatial::target::{CreditLatency, SpatialTarget};
use spatial_support::*;

fn placed(
    m: usize,
    n: usize,
    k: usize,
    target: &SpatialTarget,
) -> logicaffeine_compile::spatial::ir::SpatialProgram {
    let t = template_from_source(&gemm_program(m, n, k), target).unwrap();
    fuse_readers(&place(&tile(&t, target).unwrap(), target).unwrap(), target).unwrap()
}

fn buffer_by_origin_name<'a>(
    p: &'a logicaffeine_compile::spatial::ir::SpatialProgram,
    name: &str,
) -> Vec<&'a logicaffeine_compile::spatial::ir::Buffer> {
    p.buffers
        .iter()
        .filter(|b| b.name.starts_with(name))
        .collect()
}

#[test]
fn fifos_are_double_buffered_by_default_with_block_sized_pages() {
    let wh = SpatialTarget::wormhole();
    let p = compile_to_spatial(&gemm_program(2048, 1024, 512), &wh).unwrap();
    let plan = p.work.plan.unwrap();
    let (osh, osw, ibw) = (plan.block.osh, plan.block.osw, plan.block.ibw);
    for b in buffer_by_origin_name(&p, "in0") {
        assert_eq!(
            b.kind,
            BufferKind::Fifo {
                depth_pages: 2,
                page_tiles: plan.per_site_m * ibw
            },
            "{}",
            b.name
        );
    }
    for b in buffer_by_origin_name(&p, "in1") {
        assert_eq!(
            b.kind,
            BufferKind::Fifo {
                depth_pages: 2,
                page_tiles: ibw * plan.per_site_n
            },
            "{}",
            b.name
        );
    }
    let sub_blocks = (plan.per_site_m / osh) * (plan.per_site_n / osw);
    for b in buffer_by_origin_name(&p, "out") {
        assert_eq!(
            b.kind,
            BufferKind::Fifo {
                depth_pages: sub_blocks,
                page_tiles: osh * osw
            },
            "{}",
            b.name
        );
    }
    let streamed = streamed_program(640, 640, 640, &wh);
    assert!(streamed.summary().streaming);
    for b in &streamed.buffers {
        assert_eq!(
            b.kind,
            BufferKind::Fifo {
                depth_pages: 2,
                page_tiles: 1
            },
            "streamed {}",
            b.name
        );
    }
}

#[test]
fn a_spill_buffer_exists_iff_the_plan_has_more_than_one_k_block_in_the_accumulator_width() {
    let wh = SpatialTarget::wormhole();
    let one_block = fused_2048_program();
    assert_eq!(one_block.summary().num_blocks, 1);
    assert!(
        !one_block
            .buffers
            .iter()
            .any(|b| matches!(b.kind, BufferKind::Interm { .. })),
        "one K block needs no spill"
    );
    let two_blocks = program_with_plan(
        2048,
        2048,
        1024,
        &wh,
        blocked_plan(64, 64, 32, (8, 8), (1, 4, 8), AccPolicy::F32),
        true,
        NumericsPolicy::Exact,
    );
    assert_eq!(two_blocks.summary().num_blocks, 4);
    let plan = two_blocks.work.plan.unwrap();
    let sub_blocks = (plan.per_site_m / plan.block.osh) * (plan.per_site_n / plan.block.osw);
    let interms: Vec<_> = two_blocks
        .buffers
        .iter()
        .filter(|b| matches!(b.kind, BufferKind::Interm { .. }))
        .collect();
    assert_eq!(
        interms.len(),
        two_blocks.sites().len(),
        "one spill buffer per site"
    );
    for b in interms {
        assert_eq!(
            b.kind,
            BufferKind::Interm {
                depth_pages: sub_blocks,
                page_tiles: plan.block.acc_tiles()
            }
        );
        assert_eq!(b.dtype, Precision::Fp32, "fp32 partials spill losslessly");
    }
    for s in &two_blocks.stages {
        if let StageKind::Compute(ComputeOp::MatmulTiles {
            interm, num_blocks, ..
        }) = &s.kind
        {
            assert_eq!(*num_blocks, 4);
            assert!(
                interm.is_some(),
                "the compute stage references its spill buffer"
            );
        }
    }
}

#[test]
fn local_memory_capacity_is_a_typed_reject_with_the_bytes_needed() {
    let wh = SpatialTarget::wormhole();
    let p = placed(2048, 2048, 1024, &wh);
    let needed = footprint_bytes(&p, &wh);
    assert!(needed > 0);
    let mut shrunk = wh.clone();
    shrunk.site.local_mem_bytes = needed - 1;
    match size_buffers(&p, &shrunk) {
        Err(SpatialReject::LocalMemExceeded {
            needed_bytes,
            budget_bytes,
            ..
        }) => {
            assert_eq!(
                needed_bytes, needed,
                "the reject reports the exact footprint"
            );
            assert_eq!(budget_bytes, needed - 1);
        }
        other => panic!("a budget one byte short must be refused typed, got {other:?}"),
    }
    let sized = size_buffers(&p, &wh).unwrap();
    assert_eq!(
        footprint_bytes(&sized, &wh),
        needed,
        "sizing does not change the footprint the legality check used"
    );
}

#[test]
fn accumulator_budget_reject_names_the_policy() {
    let wh = SpatialTarget::wormhole();
    let mut p = placed(2048, 1024, 512, &wh);
    for s in p.stages.iter_mut() {
        if let StageKind::Compute(ComputeOp::MatmulTiles { block, .. }) = &mut s.kind {
            *block = BlockShape {
                osh: 4,
                osw: 4,
                ibw: block.ibw,
            };
        }
    }
    if let Some(plan) = p.work.plan.as_mut() {
        plan.block = BlockShape {
            osh: 4,
            osw: 4,
            ibw: plan.block.ibw,
        };
        plan.per_site_m = 8;
        plan.per_site_n = 4;
    }
    match size_buffers(&p, &wh) {
        Err(SpatialReject::AccTilesExceeded {
            needed: 16,
            available: 4,
            acc: AccPolicy::F32,
        }) => {}
        other => panic!("a 4×4 sub-block cannot fit a halved fp32 DST: {other:?}"),
    }
}

#[test]
fn buffers_never_overlap_and_offsets_are_deterministic() {
    let wh = SpatialTarget::wormhole();
    for case in 0..30usize {
        let (m, n, k) = (
            32 * prng(61, case, 1, 32) as usize,
            32 * prng(62, case, 1, 32) as usize,
            32 * prng(63, case, 1, 32) as usize,
        );
        let a = compile_to_spatial(&gemm_program(m, n, k), &wh).unwrap();
        let b = compile_to_spatial(&gemm_program(m, n, k), &wh).unwrap();
        assert_eq!(a.memory, b.memory, "{m}x{n}x{k}: deterministic layout");
        let mm = a.memory.as_ref().unwrap();
        let mut by_site: std::collections::BTreeMap<_, Vec<(u32, u32)>> = Default::default();
        for buf in &a.buffers {
            let at = mm.buffer_at[buf.id.idx()];
            assert_eq!(at.bytes as usize, a.buffer_bytes(buf));
            // An alias is a second view of the region it shares (`Buffer::alias_of`), never memory of its own: it
            // lies exactly on that region and within it.
            if let Some(owner) = buf.alias_of {
                let o = mm.buffer_at[owner.idx()];
                assert_eq!(
                    (at.site, at.offset_bytes),
                    (o.site, o.offset_bytes),
                    "{m}x{n}x{k}: {} is laid out on the region it aliases",
                    buf.name
                );
                assert!(
                    at.bytes <= o.bytes,
                    "{m}x{n}x{k}: {} fits inside the region it aliases ({} B in {} B)",
                    buf.name,
                    at.bytes,
                    o.bytes
                );
                continue;
            }
            by_site
                .entry(at.site)
                .or_default()
                .push((at.offset_bytes, at.offset_bytes + at.bytes));
        }
        for (site, mut v) in by_site {
            v.sort();
            for w in v.windows(2) {
                assert!(
                    w[1].0 >= w[0].1,
                    "{m}x{n}x{k}: overlap at {site:?}: {:?}",
                    w
                );
            }
            assert!(v.last().unwrap().1 as usize <= wh.site.local_mem_bytes);
        }
    }
}

#[test]
fn depth_ratchets_never_below_two_and_rises_under_a_movement_slow_model() {
    for target in SpatialTarget::presets() {
        let p = compile_to_spatial(&gemm_program(256, 256, 128), &target).unwrap();
        for b in &p.buffers {
            if let BufferKind::Fifo { depth_pages, .. } = b.kind {
                assert!(
                    depth_pages >= 2,
                    "{}: {} is not double-buffered",
                    target.name,
                    b.name
                );
            }
        }
    }
    // The slow machine is declared where a machine's latencies belong — on the target — rather than as cost
    // constants. Depth is a resource decision and must not be derived from a regression fitted to a cycle
    // counter; the ratio it needs is fetch against service, and both are architectural facts a part can cite.
    // Same hypothetical part as before, stated in the right place. MEASURED response on this program, which
    // is what makes this a live rate model rather than a hand-set number: with the fetch undeclared both input
    // FIFOs sit at 2, at fetch 4 one of them reaches 3, at fetch 8 both do, and past fetch 16 the memory
    // budget starts giving pages back rather than growing without bound.
    let mut slow_dram = SpatialTarget::wormhole();
    slow_dram.site.credit = Some(CreditLatency {
        fetch_tile: 8,
        credit_return: 8,
        service_page: 200,
    });
    let p = compile_to_spatial(&gemm_program(2048, 1024, 512), &slow_dram).unwrap();
    let deep = p
        .buffers
        .iter()
        .filter(|b| {
            b.name.starts_with("in") && matches!(b.kind, BufferKind::Fifo { depth_pages: 3, .. })
        })
        .count();
    assert!(
        deep > 0,
        "when movement outruns compute and the buffer fits, the rate model deepens the input FIFOs"
    );
}

#[test]
fn footprint_matches_the_vendor_circular_buffer_bytes() {
    let wh = SpatialTarget::wormhole();
    let p = compile_to_spatial(&gemm_program(2048, 2048, 1024), &wh).unwrap();
    let plan = p.work.plan.unwrap();
    let tile_bf16 = 2048usize;
    let tile_fp32 = 4096usize;
    let in0 = 2 * plan.per_site_m * plan.block.ibw * tile_bf16;
    let in1 = 2 * plan.block.ibw * plan.per_site_n * tile_bf16;
    let out = plan.per_site_m * plan.per_site_n * tile_fp32;
    let interm = plan.per_site_m * plan.per_site_n * tile_fp32;
    let site = *p.sites().iter().next().unwrap();
    let bytes_of = |name: &str| {
        p.buffers
            .iter()
            .find(|b| b.site == Some(site) && b.name.starts_with(name))
            .map(|b| p.buffer_bytes(b))
            .unwrap()
    };
    assert_eq!(
        bytes_of("in0"),
        in0,
        "in0 CB = per_core_M · in0_block_w · 2 · tile"
    );
    assert_eq!(
        bytes_of("in1"),
        in1,
        "in1 CB = in0_block_w · per_core_N · 2 · tile"
    );
    assert_eq!(
        bytes_of("out"),
        out,
        "out CB = per_core_M · per_core_N tiles"
    );
    // The spill buffer exists exactly when the reduction needs more than one K block — the invariant
    // `a_spill_buffer_exists_iff_the_plan_has_more_than_one_k_block_in_the_accumulator_width` states
    // separately. A single-block plan holds the whole reduction in the accumulator and has nothing to spill,
    // so asserting its bytes would be asserting a buffer that must not exist. Both branches assert.
    if plan.inner.num_blocks > 1 {
        assert_eq!(
            bytes_of("interm"),
            interm,
            "interm CB = per_core_M · per_core_N fp32 tiles"
        );
    } else {
        assert!(
            !p.buffers
                .iter()
                .any(|b| b.site == Some(site) && b.name.starts_with("interm")),
            "a single-K-block plan spills nothing, so it must carry no intermediate buffer"
        );
    }
}

#[test]
fn evaluated_occupancy_never_exceeds_any_depth() {
    let fusing = fusing_wormhole();
    let wh = SpatialTarget::wormhole();
    for (target, m, n, k) in [
        (&fusing, 128usize, 128usize, 64usize),
        (&wh, 96, 64, 64),
        (&wh, 64, 64, 128),
    ] {
        let p = compile_to_spatial(&gemm_program(m, n, k), target).unwrap();
        let (_, trace) = eval_choreography_traced(&p, &inputs_from_fills(&p)).unwrap();
        for b in &p.buffers {
            let hw = trace.high_water.get(&b.id).copied().unwrap_or(0);
            assert!(
                hw <= b.kind.depth_pages(),
                "{m}x{n}x{k}: {} reached {hw} pages of {}",
                b.name,
                b.kind.depth_pages()
            );
        }
    }
}

/// ★ THE PLAN THE PLANNER PRICED IS THE PLAN THE ALLOCATOR LAYS OUT.
///
/// `footprint_bytes_for_plan` decides which plans the tiler may even rank — `tile::candidates` drops any
/// whose footprint exceeds `max_local_mem_bytes`. `footprint_bytes` reports what the placed program
/// actually occupies. If the first over-charges, the search never sees plans the machine can run, and
/// nothing anywhere says so: the compiler simply emits a smaller one, forever, and every measurement
/// agrees with itself.
///
/// That is not hypothetical. `size_buffers` priced the K-block spill buffer whenever `num_blocks > 1`,
/// while `tile::apply_plan` creates it only `if nb > 1 && !is_streaming(&plan) && !accumulator_in_registers()`
/// — the two disagreed by exactly one condition. On the RTX 3060 that charged 64 KiB for a buffer the very
/// next pass declines to create, out of a 99 KiB budget, which left the 128×128 output tile affordable at
/// ONE K depth (16 elements) when the machine can hold four times that. The measured cost was the CUDA
/// persistent kernel running its K loop 128 times where the knob lattice's best rung runs it 64.
///
/// The comment three lines above the bug already stated the rule it broke: *"so the plan the planner
/// PRICED is the plan the allocator lays out."* A stated principle that nothing enforces is a comment.
///
/// NON-VACUITY: Wormhole and Blackhole satisfy this exactly (delta 0) at every shape below, and the GPU
/// did not — a property two of three targets already met, and the third was wrong about.
#[test]
fn the_footprint_the_planner_prices_is_the_footprint_the_allocator_lays_out() {
    use logicaffeine_compile::spatial::passes::size_buffers::footprint_bytes_for_plan;

    let mut checked = 0usize;
    for target in [
        SpatialTarget::rtx3060(),
        SpatialTarget::wormhole(),
        SpatialTarget::blackhole(),
    ] {
        for (m, n, k) in [
            (2048usize, 2048usize, 2048usize),
            (640, 640, 640),
            (512, 512, 512),
            (1024, 1024, 4096),
        ] {
            let Ok(t) = template_from_source(&gemm_program(m, n, k), &target) else {
                continue;
            };
            let Ok(p) = run_passes(t, &target) else {
                continue;
            };
            let Some(plan) = p.work.plan else { continue };
            let priced = footprint_bytes_for_plan(&p, &plan, &target);
            let laid_out = footprint_bytes(&p, &target);
            assert_eq!(
                priced, laid_out,
                "{} at {m}x{n}x{k}: the planner priced {priced} B and the allocator laid out {laid_out} B. \
                 A plan priced ABOVE what it occupies is pruned from a search it should have won; one priced \
                 BELOW overflows the site. Either way the plan that ships is not the plan that was ranked.",
                target.name
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 8,
        "the sweep must actually compile programs on more than one target: {checked}"
    );
}
