//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! spatial_roofline_lock — a program's roofline is its work over its target's roofs.
//!
//! `spatial::roofs::roofline` divides each count of the work model by the roof of the resource it loads:
//! DRAM bytes by the shared memory's bandwidth, the busiest movement engine's bytes by one network link,
//! the matrix arithmetic by the engine's rate. The largest quotient binds, and no schedule of that work
//! finishes sooner. This lock holds what does not depend on which plan the planner chose:
//!
//!   - the arithmetic is conserved: a GEMM's roofline counts `2 · M · N · K` operations on every preset;
//!   - the bounds are the published rates: one Tensix MVMUL per cycle, Wormhole's 288 bytes per cycle;
//!   - a resource with no roof has no bound (the SN50 publishes no INT8 rate; the fabric's array is
//!     sized per program), and a bound is no stronger than the clock it was converted through;
//!   - the roofline is a lower bound of a time we know exactly: the fabric's cycle twin, which the RTL
//!     keeps, never beats it.
//!
//! `Objective::Roofline` ranks plans by that bound, the slowest resource first, with `Balanced`'s key as the
//! exact tiebreak. Over every candidate it never picks a plan with a worse bound than `Balanced`'s, never
//! one worse on all four work counts, and on the fabric, the one target whose time is exact, never a slower
//! one. Where it parts from `Balanced` it is on a ragged grid that `Balanced` refuses for want of a clock:
//! the roofline prices that spread, the simulator cannot, so it stays a named objective until silicon does.
//!
//!   cargo nextest run -p logicaffeine-tests --test spatial_roofline_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::ir::{ComputeOp, NumericsPolicy, SpatialProgram, StageKind};
use logicaffeine_compile::spatial::lower::fabric::fabric_cycle_model;
use logicaffeine_compile::spatial::passes::tile::{
    admissible, best_candidate_for, candidates, Candidate,
};
use logicaffeine_compile::spatial::pipeline::{
    calibration_candidates_matching, compile_to_spatial, template_from_source, PlanFilter,
};
use logicaffeine_compile::spatial::programs::gemm_program;
use logicaffeine_compile::spatial::rdu_chip::Provenance;
use logicaffeine_compile::spatial::roofs::{roofline, Resource, Roofline};
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::spatial::work::{work_of, Objective, Work};

fn operand_of(p: &SpatialProgram) -> Precision {
    p.stages
        .iter()
        .find_map(|s| match &s.kind {
            StageKind::Compute(ComputeOp::MatmulTiles { operand, .. }) => Some(*operand),
            _ => None,
        })
        .expect("a GEMM program has a tile matmul")
}

/// The planned GEMM `m` x `n` x `k` on `t`, its work and its roofline.
fn planned(
    t: &SpatialTarget,
    (m, n, k): (usize, usize, usize),
) -> (SpatialProgram, Work, Roofline) {
    let p = compile_to_spatial(&gemm_program(m, n, k), t)
        .unwrap_or_else(|e| panic!("{} plans {m}x{n}x{k}: {e:?}", t.name));
    let w = work_of(&p, t);
    let r = roofline(&w, t, operand_of(&p));
    (p, w, r)
}

fn bound(r: &Roofline, resource: Resource) -> Option<&logicaffeine_compile::spatial::roofs::Bound> {
    r.bounds.iter().find(|b| b.resource == resource)
}

const SHAPES: [(usize, usize, usize); 3] = [(64, 64, 64), (128, 96, 160), (256, 256, 128)];

fn presets() -> [SpatialTarget; 5] {
    [
        SpatialTarget::wormhole_ttsim(),
        SpatialTarget::blackhole(),
        SpatialTarget::rtx3060(),
        SpatialTarget::sn50_rdu(),
        SpatialTarget::sky_fabric(16, 16),
    ]
}

#[test]
fn a_gemms_roofline_counts_its_arithmetic_on_every_preset() {
    for t in presets() {
        for (m, n, k) in [(64, 64, 64), (128, 96, 160)] {
            let (_, _, r) = planned(&t, (m, n, k));
            assert_eq!(
                r.ops,
                2.0 * (m * n * k) as f64,
                "{} {m}x{n}x{k}: the arithmetic, whatever the fidelity",
                t.name
            );
            assert_eq!(
                r.intensity(),
                Some(r.ops / r.dram_bytes),
                "{}: intensity is operations per DRAM byte",
                t.name
            );
        }
    }
}

#[test]
fn the_tensix_bounds_are_its_published_rates() {
    for (m, n, k) in SHAPES {
        let t = SpatialTarget::wormhole_ttsim();
        let (_, w, r) = planned(&t, (m, n, k));
        let matrix = bound(&r, Resource::Matrix).expect("a matrix bound");
        assert_eq!(
            matrix.cycles, w.site_matmul_retired as f64,
            "{m}x{n}x{k}: one MVMUL per cycle on the busiest site"
        );
        let dram = bound(&r, Resource::Dram).expect("a DRAM bound");
        assert_eq!(
            dram.cycles,
            w.dram_bytes as f64 / 288.0,
            "{m}x{n}x{k}: 288 GB/s at 1 GHz"
        );
        let link = bound(&r, Resource::NocLink).expect("a link bound");
        assert_eq!(
            link.cycles,
            w.engine_noc_bytes as f64 / 32.0,
            "{m}x{n}x{k}: one 32-byte flit per cycle"
        );
        assert!(
            [matrix, dram, link]
                .iter()
                .all(|b| b.provenance == Provenance::Published),
            "every Wormhole rate is published"
        );
        assert_eq!(
            r.binding().map(|b| b.cycles),
            Some(matrix.cycles.max(dram.cycles).max(link.cycles)),
            "the largest bound binds"
        );
        assert_eq!(r.cycles(), r.binding().unwrap().cycles);
    }
}

#[test]
fn a_chip_wide_rate_bounds_the_whole_programs_arithmetic_and_a_per_second_rate_goes_through_the_clock(
) {
    let (m, n, k) = (512, 512, 128);
    let (_, w, r) = planned(&SpatialTarget::rtx3060(), (m, n, k));
    assert!(
        w.sites_used > 1 && w.site_matmul_retired < w.matmul_retired,
        "the plan spreads over blocks, so a chip-wide rate and a per-site one differ"
    );
    let tensor = bound(&r, Resource::Matrix).expect("a tensor bound");
    assert_eq!(
        tensor.cycles,
        2.0 * (m * n * k) as f64 / (28.0 * 2048.0),
        "INT8 arithmetic over 28 SMs x 2048 per cycle, chip-wide"
    );
    let (_, w, r) = planned(&SpatialTarget::blackhole(), (m, n, k));
    let dram = bound(&r, Resource::Dram).expect("a DRAM bound");
    let expected = w.dram_bytes as f64 / (512e9 / 1.35e9);
    assert!(
        (dram.cycles - expected).abs() <= 1e-12 * expected,
        "512 GB/s at 1.35 GHz: {} cycles, expected {expected}",
        dram.cycles
    );
}

#[test]
fn a_resource_without_a_roof_has_no_bound() {
    let (p, _, r) = planned(&SpatialTarget::sn50_rdu(), (128, 96, 160));
    assert_eq!(
        operand_of(&p),
        Precision::Int8,
        "the SN50 GEMM computes in INT8"
    );
    assert!(
        bound(&r, Resource::Matrix).is_none(),
        "the SN50 publishes no INT8 rate, so its arithmetic is not priced"
    );
    assert!(
        bound(&r, Resource::NocLink).is_none(),
        "nor its switch fabric"
    );
    assert_eq!(
        bound(&r, Resource::Dram).map(|b| b.provenance),
        Some(Provenance::Modeled),
        "its HBM rate is modeled"
    );
    let (_, _, r) = planned(&SpatialTarget::sky_fabric(16, 16), (16, 16, 8));
    assert_eq!(
        r.bounds.iter().map(|b| b.resource).collect::<Vec<_>>(),
        vec![Resource::Dram],
        "the fabric's roofline is its lane"
    );
    let (_, _, r) = planned(&SpatialTarget::rtx3060(), (128, 96, 160));
    assert_eq!(
        bound(&r, Resource::Matrix).map(|b| b.provenance),
        Some(Provenance::Derived),
        "a derived per-cycle tensor rate stays derived"
    );
    assert_eq!(
        bound(&r, Resource::Dram).map(|b| b.provenance),
        Some(Provenance::Modeled),
        "a modeled bandwidth over a published clock is modeled"
    );
}

#[test]
fn the_roofline_is_a_lower_bound_of_the_fabrics_exact_time() {
    let t = SpatialTarget::sky_fabric(16, 16);
    for (m, n, k) in [(4, 4, 4), (8, 6, 10), (16, 16, 8)] {
        let (p, _, r) = planned(&t, (m, n, k));
        let exact = fabric_cycle_model(&p, &t);
        assert!(exact > 0, "{m}x{n}x{k}: the twin times the program");
        assert!(
            r.cycles() > 0.0,
            "{m}x{n}x{k}: the roofline bounds something"
        );
        assert!(
            exact as f64 >= r.cycles(),
            "{m}x{n}x{k}: the twin's {exact} cycles beat the roofline's {}",
            r.cycles()
        );
    }
}

#[test]
fn the_roofline_objective_leads_with_the_slowest_resource_and_breaks_ties_as_balanced_does() {
    let t = SpatialTarget::wormhole_ttsim();
    let rates = Objective::roofline(&t, Precision::Bf16);
    let (_, w, r) = planned(&t, (128, 96, 160));
    let key = rates.key(&w);
    assert_eq!(
        key[0],
        r.cycles().ceil() as u128,
        "the leading term is the binding bound, in whole cycles"
    );
    let balanced = Objective::Balanced.key(&w);
    assert_eq!(
        (key[1], key[2]),
        (balanced[0], balanced[1]),
        "then the shared memory's bytes and the busiest site's arithmetic"
    );
    let mut fewer_txns = w;
    fewer_txns.noc_txns -= 1;
    assert!(
        rates.key(&fewer_txns) < key,
        "at an equal bound, Balanced's own order decides"
    );
    let mut slower = w;
    slower.dram_bytes += 288 * 1_000_000;
    assert!(
        rates.key(&slower) > key,
        "a slower bound ranks after, whatever else it saves"
    );
    let one_byte_over = Work {
        dram_bytes: 289,
        ..Work::default()
    };
    assert_eq!(
        rates.key(&one_byte_over)[0],
        2,
        "289 bytes at 288 per cycle take two whole cycles; a bound is never rounded below itself"
    );
}

/// The four work counts the plans answer for on a simulator with no clock.
fn counts(c: &Candidate) -> [u64; 4] {
    [
        c.work.dram_bytes,
        c.work.site_matmul_retired,
        c.work.engine_noc_bytes,
        c.work.noc_txns,
    ]
}

#[test]
fn the_roofline_objective_never_chooses_a_slower_bound_or_a_plan_worse_on_every_count() {
    for t in [SpatialTarget::wormhole_ttsim(), SpatialTarget::blackhole()] {
        for (m, n, k) in [
            (128, 128, 128),
            (512, 512, 512),
            (640, 640, 640),
            (1024, 1024, 1024),
            (2048, 1024, 512),
            (384, 768, 1024),
        ] {
            let template = template_from_source(&gemm_program(m, n, k), &t)
                .unwrap_or_else(|e| panic!("{} {m}x{n}x{k}: {e:?}", t.name));
            let element = operand_of(&template);
            let objective = Objective::roofline(&t, element);
            let balanced =
                best_candidate_for(&template, &t, Objective::Balanced).expect("a balanced plan");
            let roof = best_candidate_for(&template, &t, objective).expect("a roofline plan");
            let (rb, rr) = (
                roofline(&balanced.work, &t, element),
                roofline(&roof.work, &t, element),
            );
            assert!(
                rr.cycles() <= rb.cycles(),
                "{} {m}x{n}x{k}: the roofline plan's bound {} exceeds Balanced's {}",
                t.name,
                rr.cycles(),
                rb.cycles()
            );
            let (cb, cr) = (counts(&balanced), counts(&roof));
            let worse_everywhere = cr.iter().zip(cb.iter()).all(|(r, b)| r >= b) && cr != cb;
            assert!(!worse_everywhere, "{} {m}x{n}x{k}: the roofline plan {cr:?} is worse than Balanced's {cb:?} on every count", t.name);
        }
    }
}

/// The compiled program of candidate `c` for `m` x `n` x `k` on `t`.
fn program_of(
    t: &SpatialTarget,
    (m, n, k): (usize, usize, usize),
    c: &Candidate,
) -> SpatialProgram {
    let filter = PlanFilter {
        pack_accumulate: Some(c.plan.pack_accumulate),
        engines: Some(c.plan.engines),
        per_site: Some((c.plan.per_site_m, c.plan.per_site_n)),
        block: Some(c.plan.block),
    };
    calibration_candidates_matching(&gemm_program(m, n, k), t, NumericsPolicy::Exact, filter)
        .unwrap_or_else(|e| panic!("{m}x{n}x{k}: {e:?}"))
        .into_iter()
        .map(|(_, p)| p)
        .find(|p| p.work.plan == Some(c.plan))
        .unwrap_or_else(|| panic!("{m}x{n}x{k}: no program for the chosen plan {:?}", c.plan))
}

#[test]
fn on_the_fabric_whose_time_is_exact_the_roofline_plan_is_never_slower() {
    let t = SpatialTarget::sky_fabric(16, 16);
    for shape in [(4, 4, 4), (8, 6, 10), (16, 16, 8), (12, 20, 9)] {
        let template = template_from_source(&gemm_program(shape.0, shape.1, shape.2), &t)
            .expect("a fabric template");
        let element = operand_of(&template);
        let balanced =
            best_candidate_for(&template, &t, Objective::Balanced).expect("a balanced plan");
        let roof = best_candidate_for(&template, &t, Objective::roofline(&t, element))
            .expect("a roofline plan");
        let (cb, cr) = (
            fabric_cycle_model(&program_of(&t, shape, &balanced), &t),
            fabric_cycle_model(&program_of(&t, shape, &roof), &t),
        );
        assert!(cb > 0 && cr > 0, "{shape:?}: the twin times both plans");
        assert!(
            cr <= cb,
            "{shape:?}: the roofline plan takes {cr} exact cycles, Balanced's {cb}"
        );
    }
}

#[test]
fn the_roofline_admits_every_plan_and_balanced_only_the_ragged_ones_no_even_plan_beats() {
    let t = SpatialTarget::wormhole_ttsim();
    let template = template_from_source(&gemm_program(640, 640, 640), &t).expect("a template");
    let cands = candidates(&template, &t);
    assert!(
        cands.iter().any(|c| c.ragged),
        "640³ on Wormhole offers ragged grids"
    );
    let objective = Objective::roofline(&t, operand_of(&template));
    assert_eq!(
        admissible(&cands, objective).len(),
        cands.len(),
        "the roofline prices a ragged grid's spread, so it refuses none"
    );
    assert!(
        admissible(&cands, Objective::Balanced).len() < cands.len(),
        "Balanced refuses the ragged grids an even plan beats on every total"
    );
}

#[test]
fn an_element_lane_is_paced_by_the_elements_it_carries_not_their_bytes() {
    let t = SpatialTarget::sky_fabric(16, 16);
    let (_, w, r) = planned(&t, (16, 16, 8));
    assert!(
        w.dram_bytes > w.dram_tiles,
        "the GEMM's wide result makes its bytes outnumber its elements ({} bytes, {} elements)",
        w.dram_bytes,
        w.dram_tiles
    );
    assert!(
        w.dram_tiles >= 16 * 8 + 8 * 16 + 16 * 16,
        "the lane carries at least the elements a 16x16x8 GEMM must read and write, counted {}",
        w.dram_tiles
    );
    let dram = bound(&r, Resource::Dram).expect("the fabric's lane bound");
    assert_eq!(
        (dram.work, dram.per_cycle),
        (w.dram_tiles as f64, 1.0),
        "one scalar tile is one element, one element per cycle"
    );
}

#[test]
fn the_closed_form_counts_the_dram_tiles_the_choreography_moves() {
    use logicaffeine_compile::spatial::work::work_of_plan;
    let t = SpatialTarget::wormhole_ttsim();
    let mut compared = 0;
    for (m, n, k) in [
        (128usize, 128usize, 128usize),
        (512, 512, 512),
        (640, 640, 640),
    ] {
        for (kind, built) in calibration_candidates_matching(
            &gemm_program(m, n, k),
            &t,
            NumericsPolicy::AllowSpillRounding,
            PlanFilter::default(),
        )
        .expect("the candidates build")
        {
            let walked = work_of(&built, &t);
            let plan = built.work.plan.expect("a built program carries its plan");
            let closed = work_of_plan(&built, &plan, built.summary().multicast_groups > 0, &t)
                .expect("a GEMM is priced");
            assert_eq!(closed.dram_tiles, walked.dram_tiles, "{m}x{n}x{k} {kind:?}: the planner's tile count is not the one the choreography moves");
            compared += 1;
        }
    }
    assert!(compared >= 6, "only {compared} candidates compared");
}
