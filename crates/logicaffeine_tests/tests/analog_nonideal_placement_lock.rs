//! ★ ANALOG NON-IDEALITY-AWARE PLACEMENT LOCK ★ — the placer's cost function EARNS ITS KEEP.
//!
//! The Pelgrom distance gradient makes per-leg gain error grow with placed operand separation,
//! so placement changes the PHYSICAL BUDGET, not just wire length. Anti-vacuity: on a deep
//! chain, the deliberate maximum-stride scatter (`place_blind`) must cost strictly more wire
//! AND a strictly larger ε than the operand-adjacent placer — and both placements must remain
//! SOUND (their own Monte-Carlo batteries inside their own budgets). If aware and blind ever
//! stop differing, the distance model went vacuous and this file goes red.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.

use logicaffeine_compile::codegen_ana::{
    analog_netlist_from_source, assumed_ranges, error_budget_nonideal, eval_analog_dc,
    eval_analog_instance, place_analog, place_blind, sample_instance, scale_pass, sky130_nonideal,
    AnalogTarget,
};

/// A deep gain/sum chain — every stage's operand distance matters.
const CHAIN: &str = "## To chain (x: Float) -> Float:\n    Assume x is between -1.0 and 1.0.\n    Return ((((((x * 0.5 + 0.25) * 0.5 + 0.125) * 0.5 + 0.0625) * 0.5 + 0.03125) * 0.5 + 0.015625) * 0.5 + 0.0078125) * 0.5.\n\n## Main\n    Show chain(0.5).\n";

const BASE_SEED: u64 = 0xA11A_00C0_FFEE_5EED;

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn point_in(state: &mut u64, lo: f64, hi: f64) -> f64 {
    let u = (splitmix(state) >> 11) as f64 / (1u64 << 53) as f64;
    lo + u * (hi - lo)
}

#[test]
fn aware_placement_beats_blind_on_wire_and_epsilon_and_both_stay_sound() {
    let target = AnalogTarget::sky130_fpaa();
    let model = sky130_nonideal();
    let netlist = analog_netlist_from_source(CHAIN, "chain", &target).expect("lowers");
    let ranges = assumed_ranges(CHAIN);
    let (schedule, cert) = scale_pass(&netlist, &ranges, &target).expect("schedules");
    let q = &schedule.quantized;

    let aware = place_analog(q, &target).expect("aware places");
    let blind = place_blind(q, &target).expect("blind places");

    // 1. The objective: strictly less wire.
    let (wa, wb) = (aware.wire_cost(q), blind.wire_cost(q));
    assert!(wa < wb, "aware wire {wa} must beat blind {wb}");

    // 2. The physics: strictly smaller ε — placement is a BUDGET lever, not cosmetics.
    let ba = error_budget_nonideal(&netlist, &cert, &schedule, &target, &model, &aware);
    let bb = error_budget_nonideal(&netlist, &cert, &schedule, &target, &model, &blind);
    assert!(
        ba.eps_math < bb.eps_math,
        "aware ε {:e} must beat blind ε {:e}",
        ba.eps_math,
        bb.eps_math
    );
    // The distance-sensitive term itself must bite hard (the TOTAL is diluted by the
    // placement-independent converter terms — that dilution is honest, so the ratio claim
    // belongs on the term the placer actually controls).
    let tile_gain = |b: &logicaffeine_compile::codegen_ana::OutputBudget| {
        b.terms
            .iter()
            .find(|t| {
                matches!(
                    t.source,
                    logicaffeine_compile::codegen_ana::ErrorSource::TileGain
                )
            })
            .map(|t| t.eps_math)
            .expect("the deep chain must carry a TileGain term")
    };
    let (ga, gb) = (tile_gain(&ba), tile_gain(&bb));
    assert!(
        gb > 1.5 * ga,
        "the deep chain must make the distance term BITE (blind TileGain {gb:e} vs aware {ga:e})"
    );

    // 3. Soundness under BOTH placements: each battery inside its own budget.
    for (label, placement, budget) in [("aware", &aware, &ba), ("blind", &blind, &bb)] {
        let mut worst = 0.0_f64;
        for die in 0..128u64 {
            let inst = sample_instance(&q.cells, placement, &model, BASE_SEED ^ (die << 16));
            let mut seed = BASE_SEED ^ die;
            for _ in 0..2 {
                let xs: Vec<f64> = ranges
                    .iter()
                    .map(|r| point_in(&mut seed, r.lo, r.hi))
                    .collect();
                let dev = (eval_analog_instance(&schedule, &target, &inst, &xs)
                    - eval_analog_dc(&netlist, &xs))
                .abs();
                worst = worst.max(dev);
                assert!(
                    dev <= budget.eps_math,
                    "`{label}` die {die}: {dev:e} exceeds its own ε {:e}",
                    budget.eps_math
                );
            }
        }
        assert!(worst > 0.0, "`{label}`: vacuous battery");
    }
}
