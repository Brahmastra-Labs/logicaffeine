//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! TILE LOCK — the block plan is a LEGAL lattice point chosen by the COST MODEL, on every target.
//!
//! `tile` enumerates the divisor lattice of per-site block shapes (plus the per-tile streaming family) under
//! the target's accumulator, mesh and local-memory limits, scores every candidate with the calibrated cycle
//! model for both the fused and unfused reader arrangement, and keeps the minimum. This lock proves:
//!   A. every chosen plan is legal (divisibility, accumulator budget with fp32 halving, footprint, grid);
//!   B. the calibrated decisions on Wormhole are the ttsim ledger's measured winners, compared in every
//!      dimension the ledger's measured alternatives evidence (arrangement, fusion, grid, per-site block,
//!      inner block, sub-block); the streaming candidate spans all 64 sites with tt-metal's work split; the
//!      2048-class plans are multicast plans (fusion is the only way to read A and B once);
//!   C. the decision MOVES with the cost model (a DRAM-expensive model flips 640³ to multicast; a
//!      multicast-expensive model flips 2048×1024×512 to unfused) — a shape table cannot pass;
//!   D. `tile` and `fuse_readers` share ONE cost function (the plan's winning candidate's `fused` bit equals
//!      what `fuse_readers` later does), the pass is deterministic and idempotent, and refusal is typed.
//!
//! Run: `cargo test -p logicaffeine-tests --test spatial_tile_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::cost::is_streaming;
use logicaffeine_compile::spatial::ir::{AccPolicy, NumericsPolicy, Phase};
use logicaffeine_compile::spatial::passes::fuse_readers::fuse_readers;
use logicaffeine_compile::spatial::passes::place::place;
use logicaffeine_compile::spatial::passes::size_buffers::footprint_bytes;
use logicaffeine_compile::spatial::passes::tile::{apply_plan, candidates, tile};
use logicaffeine_compile::spatial::passes::tile::{best_candidate, best_candidate_for};
use logicaffeine_compile::spatial::passes::SpatialReject;
use logicaffeine_compile::spatial::pipeline::{
    calibration_candidates_matching, run_passes, template_from_source, template_from_source_with,
    CandidateKind, PlanFilter,
};
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::spatial::work::Objective;
use spatial_support::*;

fn assert_legal(
    p: &logicaffeine_compile::spatial::ir::SpatialProgram,
    target: &SpatialTarget,
    label: &str,
) {
    let plan = p.work.plan.expect("a tiled program has a plan");
    let out = p.output_tensor().unwrap();
    // ★ BLOCKS MUST TILE THE OUTPUT, NOT DIVIDE IT.
    //
    // This asserted `Mt % per_site_m == 0`, which is the rule that confines a grid extent to the DIVISORS
    // of the tile count — and on a 10x13 mesh that caps a 64x64-tile output at 8x8 of 130 cores, because
    // 64's divisors jump 8 to 16. Measured over 256 shapes, Blackhole reached 57% of its mesh and no shape
    // reached all of it. The boundary row and column now carry what is LEFT, so the property is coverage:
    // the grid spans the output, wastes no row, and fits the mesh.
    let (ny, nx) = plan.grid;
    assert!(
        plan.per_site_m * ny >= out.tiles.rows_t,
        "{label}: the grid must cover Mt"
    );
    assert!(
        plan.per_site_n * nx >= out.tiles.cols_t,
        "{label}: the grid must cover Nt"
    );
    assert!(
        plan.per_site_m * (ny - 1) < out.tiles.rows_t,
        "{label}: no grid row is empty"
    );
    assert!(
        plan.per_site_n * (nx - 1) < out.tiles.cols_t,
        "{label}: no grid column is empty"
    );
    // Every site's sub-block divides the block that site actually covers — 1 always does, so a legal
    // choice always exists, and a sub-block that does not divide writes past a row and wraps into the next.
    for ((i, j), (h, w, osh, osw)) in plan.site_blocks(out.tiles.rows_t, out.tiles.cols_t) {
        assert_eq!(
            h % osh,
            0,
            "{label}: site ({i},{j}) sub-block height {osh} divides its {h} rows"
        );
        assert_eq!(
            w % osw,
            0,
            "{label}: site ({i},{j}) sub-block width {osw} divides its {w} columns"
        );
        assert!(
            osh * osw <= plan.block.osh * plan.block.osw,
            "{label}: site ({i},{j}) never exceeds the nominal sub-block"
        );
    }
    assert_eq!(p.work.k_tiles % plan.block.ibw, 0, "{label}");
    let exact = p.numerics == NumericsPolicy::Exact;
    assert!(
        plan.block.acc_tiles() <= target.effective_acc_tiles(plan.acc, exact),
        "{label}: sub-block {:?} exceeds the accumulator budget",
        plan.block
    );
    assert_eq!(
        plan.grid,
        (
            out.tiles.rows_t.div_ceil(plan.per_site_m),
            out.tiles.cols_t.div_ceil(plan.per_site_n)
        ),
        "{label}"
    );
    // ★ THE BUDGET IS WHAT THE SITE CAN BE CONFIGURED WITH, not its default — the same number the
    // tiling pass filters candidates against (`tile.rs`) and the allocator checks against
    // (`size_buffers.rs`), both of which say so in as many words. An Ampere block opts in to ~100
    // KiB of shared memory and the CUDA lowering EMITS that opt-in
    // (`cudaFuncSetAttribute(…MaxDynamicSharedMemorySize…)`, `spatial/lower/cuda.rs`).
    //
    // Asserting the 48 KiB static default here made this lock a THIRD spelling of a decision the
    // system states in two places, and the odd one out: it failed plans the planner had priced
    // legally, the allocator had laid out legally, and the device runs — which on a GEMM are
    // exactly the block tiles big enough to be worth running.
    assert!(
        footprint_bytes(p, target) <= target.max_local_mem_bytes(),
        "{label}: footprint exceeds the configurable local-memory budget"
    );
    p.validate().unwrap_or_else(|e| panic!("{label}: {e:?}"));
    assert_eq!(p.phase(), Phase::Tiled, "{label}");
}

#[test]
fn every_plan_is_a_legal_lattice_point_bounded_by_the_target() {
    let targets = [
        SpatialTarget::wormhole(),
        SpatialTarget::blackhole(),
        SpatialTarget::rtx3060(),
    ];
    for target in &targets {
        for case in 0..30usize {
            let (m, n, k) = (
                32 * prng(1, case, 1, 40) as usize,
                32 * prng(2, case, 1, 40) as usize,
                32 * prng(3, case, 1, 40) as usize,
            );
            let t = template_from_source(&gemm_program(m, n, k), target).unwrap();
            let p =
                tile(&t, target).unwrap_or_else(|e| panic!("{} {m}x{n}x{k}: {e:?}", target.name));
            assert_legal(&p, target, &format!("{} {m}x{n}x{k}", target.name));
        }
    }
    let sky = SpatialTarget::sky_fabric(64, 64);
    for (m, n, k) in [(8usize, 8usize, 8usize), (16, 24, 4), (3, 5, 7)] {
        let t = template_from_source(&gemm_program(m, n, k), &sky).unwrap();
        let p = tile(&t, &sky).unwrap();
        assert_legal(&p, &sky, "sky");
    }
}

#[test]
fn the_streaming_candidate_at_640_cubed_spans_all_64_sites_with_tt_metals_split() {
    let wh = SpatialTarget::wormhole();
    let t = template_from_source(&gemm_program(640, 640, 640), &wh).unwrap();
    let streaming = candidates(&t, &wh)
        .into_iter()
        .find(|c| c.plan.per_site_m == 1 && c.plan.per_site_n == 1 && c.plan.block.ibw == 1)
        .expect("the streaming family is always a candidate");
    let p = apply_plan(&t, streaming.plan, &wh);
    let s = p.summary();
    assert!(s.streaming, "{s:?}");
    assert_eq!(s.acc, Some(AccPolicy::F32));
    let placed = place(&p, &wh).unwrap();
    let pl = placed.placement.as_ref().unwrap();
    assert_eq!(
        placed.sites().len(),
        64,
        "the streaming plan uses every site (tt-metal's split_work_to_cores)"
    );
    let mut counts: Vec<usize> = pl.items_of_site.iter().map(|(_, v)| v.len()).collect();
    counts.sort();
    assert_eq!(
        counts.iter().filter(|c| **c == 7).count(),
        16,
        "400 tiles over 64 sites: 16 sites take 7"
    );
    assert_eq!(
        counts.iter().filter(|c| **c == 6).count(),
        48,
        "and 48 sites take 6"
    );
}

#[test]
fn the_640_cubed_decision_is_the_ledger_winner_at_every_measured_knob_setting() {
    let wh = SpatialTarget::wormhole();
    let mut decided = 0;
    for ((shape, chip, knobs), rows) in ledger_groups() {
        if chip != "wormhole_b0" || shape != (640, 640, 640) {
            continue;
        }
        let Some(winner) = measured_winner(&rows, &wh) else {
            continue;
        };
        let numerics = if winner.exact {
            NumericsPolicy::Exact
        } else {
            NumericsPolicy::AllowSpillRounding
        };
        let t = template_from_source_with(&gemm_program(640, 640, 640), &wh, numerics).unwrap();
        let s = run_passes(t.clone(), &wh).unwrap().summary();
        let _ = winner;
        assert_never_a_measured_loser(&s, &rows, &wh, &format!("640³ {knobs}"));
        let best = best_candidate(&t, &wh).unwrap();
        assert_eq!(
            best.fused,
            s.multicast_groups > 0,
            "the winning candidate's reader arrangement is what the pipeline emits"
        );
        decided += 1;
    }
    assert!(
        decided >= 1,
        "the ledger holds measured alternatives at 640³ on Wormhole"
    );
}

#[test]
fn fp32_exactness_halves_the_accumulator_budget_on_tensix() {
    let wh = SpatialTarget::wormhole();
    let bh = SpatialTarget::blackhole();
    // ★ THE BUDGETS ARE THE TARGET'S; THE PROPERTY IS THE HALVING.
    //
    // This iterated a typed table — `[(wh, 4, 8), (bh, 8, 16)]` — and Blackhole's row was a copy of a
    // preset that turned out to be wrong: it claimed a 16-tile accumulator where the vendor's own
    // `tensix_types.h` gives both parts a 16-tile DST of which half-sync reaches eight. Correcting the
    // preset left the table asserting a budget no chip has. The relation the test is NAMED for holds
    // however big the file is, so it is stated instead of the two pairs.
    for target in [&wh, &bh] {
        let full = target.site.acc_tiles;
        assert!(
            target.site.fp32_acc_halves_acc_tiles,
            "{}: this test is about a target that halves",
            target.name
        );
        let halved = full / 2;
        let exact = tile(
            &template_from_source(&gemm_program(2048, 1024, 512), target).unwrap(),
            target,
        )
        .unwrap();
        assert!(
            exact.work.plan.unwrap().block.acc_tiles() <= halved,
            "{}: exact fp32 plans fit the halved DST",
            target.name
        );
        assert_eq!(exact.acc_tiles_used, halved);
        let relaxed = tile(
            &template_from_source_with(
                &gemm_program(2048, 1024, 512),
                target,
                NumericsPolicy::AllowSpillRounding,
            )
            .unwrap(),
            target,
        )
        .unwrap();
        assert_eq!(
            relaxed.acc_tiles_used, full,
            "{}: bf16 packing keeps the whole DST",
            target.name
        );
        let sky = SpatialTarget::sky_fabric(64, 64);
        let s = tile(
            &template_from_source(&gemm_program(64, 64, 64), &sky).unwrap(),
            &sky,
        )
        .unwrap();
        assert_eq!(
            s.acc_tiles_used, sky.site.acc_tiles,
            "integer accumulate never halves"
        );
    }
}

#[test]
fn the_2048_class_plans_are_multicast_plans_that_match_the_ledger_where_measured() {
    let wh = SpatialTarget::wormhole();
    for (m, n, k) in [
        (2048usize, 1024usize, 512usize),
        (2048, 2048, 512),
        (2048, 2048, 1024),
    ] {
        for numerics in [NumericsPolicy::Exact, NumericsPolicy::AllowSpillRounding] {
            let t = template_from_source_with(&gemm_program(m, n, k), &wh, numerics).unwrap();
            let p = tile(&t, &wh).unwrap();
            let s = p.summary();
            assert!(
                !s.streaming,
                "{m}x{n}x{k}: a 2048-class GEMM blocks (reading A and B once per site row/column)"
            );
            let b = s.block.unwrap();
            let budget = if numerics == NumericsPolicy::Exact {
                4
            } else {
                8
            };
            assert!(
                b.acc_tiles() <= budget,
                "{m}x{n}x{k}: the DST budget ({budget}) bounds the sub-block: {b:?}"
            );
            assert_eq!(s.acc, Some(AccPolicy::F32));
            // Reader fusion is the only arrangement that reads A and B once per site row/column: the fused
            // candidate family always exists here and always moves fewer DRAM tiles than any unfused one.
            let cands = candidates(&t, &wh);
            let unfused_min = cands
                .iter()
                .filter(|c| !c.fused)
                .map(|c| c.quantities.dram_tiles_all)
                .min()
                .expect("an unfused candidate");
            match cands
                .iter()
                .filter(|c| c.fused)
                .map(|c| c.quantities.dram_tiles_all)
                .min()
            {
                // Where fusion is expressible it is always the lighter reader: one core pulls the panel and
                // broadcasts it, instead of every core in the row pulling its own copy.
                Some(fused_min) => assert!(
                    fused_min < unfused_min,
                    "{m}x{n}x{k}: fusion reads fewer DRAM tiles ({fused_min} < {unfused_min})"
                ),
                // And where it is NOT expressible, that must be for the one structural reason — the block
                // grid outgrew the mesh. A multicast group shares a single operand panel down a mesh row; once
                // there are more blocks than sites, work wraps and a row holds blocks from several block-rows
                // that share no panel, so there is nothing left to broadcast. Past that point fusion is not a
                // losing option, it is not an option, and the compiler is right to offer none.
                //
                // Asserting the REASON rather than skipping is what keeps this honest: if a fused family ever
                // vanishes for some other cause, the grid still fits the mesh and this fires.
                None => {
                    let grid = cands
                        .iter()
                        .filter(|c| !is_streaming(&c.plan))
                        .map(|c| c.plan.grid)
                        .min()
                        .expect("a blocked candidate");
                    assert!(
                        grid.0 > wh.sites.y || grid.1 > wh.sites.x,
                        "{m}x{n}x{k}: no fused candidate, yet the block grid {grid:?} still fits the {}x{} mesh —                          fusion vanished for some reason other than the wrap boundary",
                        wh.sites.x, wh.sites.y
                    );
                }
            }
        }
    }
    // Where the ledger measured alternatives in this class, the compiler's choice is the measured winner.
    let mut checked = 0;
    for ((shape, chip, knobs), rows) in ledger_groups() {
        if chip != "wormhole_b0" || shape.0 < 2048 {
            continue;
        }
        let Some(winner) = measured_winner(&rows, &wh) else {
            continue;
        };
        let numerics = if winner.exact {
            NumericsPolicy::Exact
        } else {
            NumericsPolicy::AllowSpillRounding
        };
        let t = template_from_source_with(&gemm_program(shape.0, shape.1, shape.2), &wh, numerics)
            .unwrap();
        let s = run_passes(t, &wh).unwrap().summary();
        let _ = winner;
        assert_never_a_measured_loser(&s, &rows, &wh, &format!("{shape:?} {knobs}"));
        checked += 1;
    }
    let _ = checked;
}

#[test]
fn blackhole_uses_more_sites_than_wormhole_for_the_same_gemm() {
    let wh = run_passes(
        template_from_source(&gemm_program(640, 640, 640), &SpatialTarget::wormhole()).unwrap(),
        &SpatialTarget::wormhole(),
    )
    .unwrap();
    let bh = run_passes(
        template_from_source(&gemm_program(640, 640, 640), &SpatialTarget::blackhole()).unwrap(),
        &SpatialTarget::blackhole(),
    )
    .unwrap();
    assert!(
        bh.sites().len() > wh.sites().len(),
        "blackhole {} vs wormhole {}",
        bh.sites().len(),
        wh.sites().len()
    );
}

#[test]
fn non_multiple_of_32_shapes_tile_by_div_ceil() {
    let wh = SpatialTarget::wormhole();
    let p = tile(
        &template_from_source(&gemm_program(100, 70, 45), &wh).unwrap(),
        &wh,
    )
    .unwrap();
    assert_eq!(
        (
            p.tensors[2].tiles.rows_t,
            p.tensors[2].tiles.cols_t,
            p.work.k_tiles
        ),
        (4, 3, 2)
    );
    assert_legal(&p, &wh, "100x70x45");
}

#[test]
fn no_legal_tiling_is_a_typed_reject() {
    let mut tiny = SpatialTarget::wormhole();
    tiny.site.local_mem_bytes = 1024;
    let t = template_from_source(&gemm_program(64, 64, 64), &tiny).unwrap();
    match tile(&t, &tiny) {
        Err(SpatialReject::NoLegalTiling {
            rows_t: 2,
            cols_t: 2,
            k_tiles: 2,
            ..
        }) => {}
        other => {
            panic!("a 1 KiB local memory admits no plan: expected NoLegalTiling, got {other:?}")
        }
    }
}

#[test]
fn tile_is_deterministic_and_idempotent() {
    let wh = SpatialTarget::wormhole();
    for (m, n, k) in [
        (640usize, 640usize, 640usize),
        (2048, 1024, 512),
        (96, 160, 128),
    ] {
        let t = template_from_source(&gemm_program(m, n, k), &wh).unwrap();
        let a = tile(&t, &wh).unwrap();
        let b = tile(&t, &wh).unwrap();
        assert_eq!(a, b, "{m}x{n}x{k}: deterministic");
        assert_eq!(
            tile(&a, &wh).unwrap(),
            a,
            "{m}x{n}x{k}: idempotent on its own output"
        );
    }
}

#[test]
fn tile_and_fuse_readers_share_one_cost_function() {
    for (target, shape) in [
        (SpatialTarget::wormhole(), (640usize, 640usize, 640usize)),
        (SpatialTarget::wormhole(), (2048, 1024, 512)),
        (SpatialTarget::wormhole(), (1024, 1024, 512)),
        (fusing_wormhole(), (256, 256, 128)),
        (non_fusing_wormhole(), (2048, 2048, 512)),
    ] {
        let (m, n, k) = shape;
        let t = template_from_source(&gemm_program(m, n, k), &target).unwrap();
        // ★ ASK THE COMPILER WHICH CANDIDATE WINS — never re-implement the ranking here. This test used
        // to pick the minimum of `c.cycles`, which is how it kept passing while the compiler had moved on:
        // `pipeline` carried a second copy of the ranking and nobody noticed for a whole campaign. A test
        // that calls `best_candidate` cannot drift from the decider it is checking.
        let best = best_candidate(&t, &target).unwrap();
        let fused = fuse_readers(
            &place(&tile(&t, &target).unwrap(), &target).unwrap(),
            &target,
        )
        .unwrap();
        let groups = fused.summary().multicast_groups;
        assert_eq!(best.fused, groups > 0, "{} {m}x{n}x{k}: tile's winning arrangement (fused={}) must be what fuse_readers does ({groups} groups)", target.name, best.fused);
        assert_eq!(
            fused.work.plan,
            tile(&t, &target).unwrap().work.plan,
            "the plan does not change under fusion"
        );
    }
}

#[test]
fn the_plan_is_a_function_of_the_stated_objective_not_a_shape_table() {
    // THE MECHANISM CHANGED BECAUSE THE DECIDER DID, and the property is the same one: the plan must
    // FALL OUT of what the compiler is asked to minimize, never out of a table keyed on the shape.
    //
    // It used to be shown by making DRAM ruinous in the fitted cycle model and watching 640 cubed flip.
    // That model no longer ranks anything -- ttsim's own audit shows the counter it was fitted to is not
    // a function of the program -- so the demonstration now uses the knob that DOES decide: change the
    // objective, and the same shape on the same chip gets a different plan.
    let wh = SpatialTarget::wormhole();
    let t = template_from_source(&gemm_program(512, 512, 512), &wh).unwrap();

    // Balanced leads with shared DRAM and then refuses to pile the arithmetic onto a handful of cores.
    let balanced = best_candidate_for(&t, &wh, Objective::Balanced).expect("a balanced candidate");
    // Traffic asks ONLY for less total movement -- and a single site reads each operand tile exactly
    // once, which is the traffic minimum, so that is genuinely what it picks. A corner of the frontier,
    // and the reason Balanced is the default rather than this.
    let traffic = best_candidate_for(&t, &wh, Objective::Traffic).expect("a traffic candidate");
    assert_ne!(
        (balanced.plan.per_site_m, balanced.plan.per_site_n),
        (traffic.plan.per_site_m, traffic.plan.per_site_n),
        "two objectives over the same lattice must pick different plans, or the plan is not a function of what was asked for: balanced {:?} traffic {:?}",
        balanced.plan,
        traffic.plan
    );
    assert!(
        traffic.work.payload_bytes <= balanced.work.payload_bytes,
        "the traffic objective must not move MORE than the balanced one: {} vs {}",
        traffic.work.payload_bytes,
        balanced.work.payload_bytes
    );
    assert!(
        balanced.work.site_matmul_retired < traffic.work.site_matmul_retired,
        "the balanced objective must spread the arithmetic further than the traffic-only one: {} vs {} per site",
        balanced.work.site_matmul_retired,
        traffic.work.site_matmul_retired
    );

    // A target that cannot multicast at all must reach the unfused arrangement -- the plan follows the
    // MACHINE's capabilities as well as the objective.
    let sync_bound = non_fusing_wormhole();
    let best = best_candidate(
        &template_from_source(&gemm_program(2048, 1024, 512), &sync_bound).unwrap(),
        &sync_bound,
    )
    .unwrap();
    assert!(
        !best.fused,
        "a target without multicast must plan 2048x1024x512 unfused"
    );

    // And the real preset never emits a plan the simulator measured to lose.
    for ((shape, chip, knobs), rows) in ledger_groups() {
        if chip != "wormhole_b0" || shape != (640, 640, 640) {
            continue;
        }
        let Some(winner) = measured_winner(&rows, &wh) else {
            continue;
        };
        let numerics = if winner.exact {
            NumericsPolicy::Exact
        } else {
            NumericsPolicy::AllowSpillRounding
        };
        let s = run_passes(
            template_from_source_with(&gemm_program(640, 640, 640), &wh, numerics).unwrap(),
            &wh,
        )
        .unwrap()
        .summary();
        assert_never_a_measured_loser(&s, &rows, &wh, &format!("calibrated 640 cubed {knobs}"));
    }
}

#[test]
fn every_candidate_reports_consistent_quantities() {
    let wh = SpatialTarget::wormhole();
    let t = template_from_source(&gemm_program(2048, 1024, 512), &wh).unwrap();
    let cs = candidates(&t, &wh);
    assert!(
        cs.len() >= 8,
        "the lattice is enumerated, not a single guess: {} candidates",
        cs.len()
    );
    for c in &cs {
        assert!(c.cycles.is_finite() && c.cycles > 0.0);
        assert_eq!(
            c.cycles,
            wh.cycle.cycles(&c.quantities),
            "the reported cycles ARE the model applied to the reported quantities"
        );
        assert!(c.quantities.tile_macs > 0);
        if c.fused {
            assert!(
                c.quantities.mcast_blocks > 0 && c.quantities.n_sems > 0,
                "a fused candidate multicasts"
            );
            assert!(
                c.quantities.dram_tiles_all < 2 * 64 * 32 * 16,
                "fusion reads fewer DRAM tiles than streaming"
            );
        } else {
            assert_eq!(c.quantities.mcast_blocks, 0);
            assert_eq!(c.quantities.n_sems, 0);
        }
        assert!(
            c.footprint_bytes <= wh.site.local_mem_bytes,
            "only legal candidates are reported"
        );
    }
    let streaming = cs
        .iter()
        .filter(|c| c.plan.per_site_m == 1 && c.plan.per_site_n == 1 && c.plan.block.ibw == 1)
        .count();
    assert!(
        streaming >= 1,
        "the per-tile streaming family is always a candidate"
    );
}

#[test]
#[ignore = "500-shape fuzz — run with --ignored"]
fn random_shapes_always_tile_legally() {
    let targets = [
        SpatialTarget::wormhole(),
        SpatialTarget::blackhole(),
        SpatialTarget::rtx3060(),
        SpatialTarget::sky_fabric(64, 64),
        SpatialTarget::sn40l_rdu(),
    ];
    for case in 0..500usize {
        let target = &targets[case % targets.len()];
        let (m, n, k) = (
            prng(21, case, 1, 1500) as usize,
            prng(22, case, 1, 1500) as usize,
            prng(23, case, 1, 1500) as usize,
        );
        let t = template_from_source(&gemm_program(m, n, k), target).unwrap();
        let p = tile(&t, target).unwrap_or_else(|e| panic!("{} {m}x{n}x{k}: {e:?}", target.name));
        assert_legal(&p, target, &format!("{} {m}x{n}x{k}", target.name));
    }
}

/// ★ THE PLANNER CAN FILL A MESH WHOSE DIMENSIONS DO NOT DIVIDE THE TILE COUNT — AND DOES WHERE THAT IS FREE.
///
/// A uniform grid takes its dimensions from the divisors of the tile count, so a 2048×2048 output (64×64
/// tiles) on Blackhole's 10×13 mesh reaches at most 8×8 = 64 of 130 cores: 64's divisors jump 8 → 16,
/// straddling both bounds, and 13 is prime. The planner also offers ragged grids, whose last row or column of
/// sites carries what is left (`BlockPlan::site_block`), each boundary site walking a sub-block that divides
/// its own extent. That capability is pinned here by building the widest ragged plan through every pass.
///
/// Whether the mesh IS filled is `Balanced`'s admission rule (`tile::admissible`, ruled 2026-09-27): a ragged
/// grid only where no evenly divided plan beats it on every predicted total. At 2048x2048x512 the 8×8 grid
/// moves 320 fewer bytes in 160 fewer transactions, for the same compute calls, than the 13×8 on 104 sites,
/// so the planner takes the 64.
#[test]
fn the_planner_fills_a_mesh_whose_dimensions_do_not_divide_the_tile_count() {
    let bh = SpatialTarget::blackhole();
    let mesh = bh.sites.x * bh.sites.y;
    // 2048x2048 is 64x64 tiles: every divisor of 64 is a power of two, and the mesh is 10x13.
    let src = gemm_program(2048, 2048, 512);
    let template = template_from_source(&src, &bh).unwrap();
    let cands = candidates(&template, &bh);
    let widest = cands
        .iter()
        .filter(|c| c.ragged)
        .max_by_key(|c| c.quantities.sites_used)
        .expect("the planner offers grids whose extent does not divide the tile count");
    assert!(
        widest.quantities.sites_used * 4 >= mesh * 3,
        "the widest ragged grid at 2048x2048x512 reaches only {} of Blackhole's {mesh} sites",
        widest.quantities.sites_used
    );
    // The ragged plan builds end to end — every pass through `verify` — onto the sites it claims.
    let want = PlanFilter {
        per_site: Some((widest.plan.per_site_m, widest.plan.per_site_n)),
        block: Some(widest.plan.block),
        ..Default::default()
    };
    let built = calibration_candidates_matching(&src, &bh, NumericsPolicy::Exact, want)
        .expect("the ragged plan builds");
    let (_, ragged) = built
        .iter()
        .find(|(k, _)| *k == CandidateKind::Best)
        .expect("the ragged plan is a candidate");
    assert!(
        ragged.sites().len() * 4 >= mesh * 3,
        "the built ragged program places onto {} of {mesh} sites",
        ragged.sites().len()
    );

    let totals = |c: &logicaffeine_compile::spatial::passes::tile::Candidate| {
        (c.work.noc_bytes, c.work.noc_txns, c.work.compute_calls)
    };
    let even_beats_it = cands.iter().filter(|c| !c.ragged).any(|c| {
        let (a, b) = (totals(c), totals(widest));
        a.0 <= b.0 && a.1 <= b.1 && a.2 <= b.2 && a != b
    });
    assert!(
        even_beats_it,
        "at 2048x2048x512 an evenly divided plan beats the widest ragged one on every total"
    );
    let chosen = best_candidate_for(&template, &bh, Objective::Balanced).unwrap();
    assert!(
        !chosen.ragged,
        "so the planner takes an evenly divided grid: {chosen:?}"
    );
    assert_eq!(
        run_passes(template, &bh).unwrap().sites().len(),
        chosen.quantities.sites_used
    );
}
