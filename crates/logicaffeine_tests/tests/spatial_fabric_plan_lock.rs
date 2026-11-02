//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! spatial_fabric_plan_lock — on our fabric, whose time is exact, the planner chooses the fastest GEMM plan
//! it was offered.
//!
//! The fabric's systolic array runs one sub-block over its whole inner block per call, paying the array's
//! fill once per call, and is fed the sub-block's A and B panels once (`BlockApi::Systolic`). The work model
//! counts both: a deeper inner block makes fewer calls, and a square sub-block loads fewer operands than a
//! long one of the same area. The cycle twin, cycle-exact against the RTL (`spatial_fabric_lowering_lock`),
//! times every candidate, so here the planner is held to time itself: its choice takes exactly the cycles of
//! the fastest candidate, at every shape, under both numerics.
//!
//!   cargo nextest run -p logicaffeine-tests --test spatial_fabric_plan_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::spatial::ir::NumericsPolicy;
use logicaffeine_compile::spatial::lower::fabric::fabric_cycle_model;
use logicaffeine_compile::spatial::passes::tile::candidates;
use logicaffeine_compile::spatial::pipeline::{
    build_candidate, compile_to_spatial_with, template_from_source_with,
};
use logicaffeine_compile::spatial::programs::gemm_program;
use logicaffeine_compile::spatial::target::{BlockApi, ComputeCap, SpatialTarget};

#[test]
fn the_fabric_matrix_engine_takes_a_whole_sub_block_per_call() {
    let t = SpatialTarget::sky_fabric(16, 16);
    assert!(
        t.matmul_block_api(),
        "the systolic array runs a sub-block over its whole inner block per call"
    );
    assert!(
        t.compute.iter().any(|c| matches!(
            c,
            ComputeCap::MatmulTile {
                block_api: Some(BlockApi::Systolic),
                ..
            }
        )),
        "its API is the systolic array's: one call per inner block, each operand panel loaded once"
    );
}

const SHAPES: [(usize, usize, usize); 7] = [
    (4, 4, 4),
    (8, 6, 10),
    (16, 16, 8),
    (12, 20, 9),
    (16, 8, 32),
    (20, 12, 6),
    (24, 16, 12),
];

/// The planner's GEMM on the fabric takes exactly the cycles of the fastest candidate it was offered.
fn the_planner_chooses_the_fastest_candidate(numerics: NumericsPolicy) {
    let t = SpatialTarget::sky_fabric(16, 16);
    for (m, n, k) in SHAPES {
        let src = gemm_program(m, n, k);
        let chosen = fabric_cycle_model(
            &compile_to_spatial_with(&src, &t, numerics).expect("the fabric plans the GEMM"),
            &t,
        );
        assert!(
            chosen > 0,
            "{m}x{n}x{k} {numerics:?}: the twin times the chosen plan"
        );
        let template = template_from_source_with(&src, &t, numerics).expect("a fabric template");
        let mut timed = 0;
        let mut fastest: Option<(u64, String)> = None;
        for c in candidates(&template, &t) {
            let built = build_candidate(&template, &c, &t).unwrap_or_else(|e| {
                panic!(
                    "{m}x{n}x{k}: the candidate {:?} does not build: {e:?}",
                    c.plan
                )
            });
            let cycles = fabric_cycle_model(&built, &t);
            assert!(
                cycles > 0,
                "{m}x{n}x{k}: the twin times the candidate {:?}",
                c.plan
            );
            timed += 1;
            if fastest.as_ref().map_or(true, |(f, _)| cycles < *f) {
                fastest = Some((cycles, format!("{:?}", c.plan)));
            }
        }
        assert!(timed > 10, "{m}x{n}x{k}: only {timed} candidates timed");
        let (fastest, plan) = fastest.expect("a timed candidate");
        assert_eq!(chosen, fastest, "{m}x{n}x{k} {numerics:?}: the planner's plan takes {chosen} exact cycles, the fastest candidate {fastest} ({plan})");
    }
}

#[test]
fn under_exact_numerics_the_planner_chooses_the_fastest_fabric_gemm() {
    the_planner_chooses_the_fastest_candidate(NumericsPolicy::Exact);
}

#[test]
fn under_rounding_numerics_the_planner_chooses_the_fastest_fabric_gemm() {
    the_planner_chooses_the_fastest_candidate(NumericsPolicy::AllowSpillRounding);
}
