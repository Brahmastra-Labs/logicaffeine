//! ★ PER-NET SCALE LOCK ★ — the block-scale weight-amplification limiter is dead: net scales
//! decouple, so one large intermediate no longer drags every net (and the multiplier
//! reference) down with it.
//!
//! What this file locks:
//!   1. THE LIMITER IS DEAD — a Mul-plus-weighted-chain region that a single block scale
//!      cannot serve (the reference must sit in the realizable Vref set while the chain's
//!      intermediates overflow it) COMPILES with genuinely divergent per-net scales, and its
//!      certificate passes the UNTOUCHED independent checker (per-cell scales were always
//!      its native vocabulary).
//!   2. NO CREEP — a uniform-sufficient region stays uniform: `per_net_scale_log2` is
//!      all-equal and the whole existing battery is the byte-identity witness.
//!   3. PHYSICS STILL AGREES — the volt-domain evaluation of the per-net schedule matches
//!      the ideal math within the compiler's own ε budget, which replays (`recheck`) and
//!      itemizes the realized-vs-ideal snap as `WeightQuant`.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. The checker never learns producer tricks; the
//! io/Mul/Const pinning rules are soundness lines, not suggestions.

use logicaffeine_compile::codegen_ana::{
    analog_artifact_from_source, eval_analog_dc, eval_analog_quantized, to_range_claim,
    AnalogTarget, ErrorSource,
};
use logicaffeine_proof::interval_replay::check_range_claim;

/// A Mul whose reference must be realizable PLUS a weighted chain whose intermediates
/// overflow every realizable reference — unschedulable under one block scale on the V1 chip.
const SPIKE: &str = "## To spike (a: Float, b: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Return ((a * b + a * 1.9 + b * 1.9) * 1.9 + (a * b) * 0.1) * 0.05.\n\n## Main\n    Show spike(0.5, 0.25).\n";

const DOT3: &str = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";

#[test]
fn the_block_scale_limiter_is_dead_and_the_untouched_checker_certifies_it() {
    let target = AnalogTarget::sky130_fpaa();
    let art = analog_artifact_from_source(SPIKE, "spike", &target)
        .expect("the per-net pass schedules what one block scale cannot");

    let distinct: std::collections::BTreeSet<i32> =
        art.schedule.per_net_scale_log2.iter().copied().collect();
    assert!(
        distinct.len() >= 2,
        "genuinely divergent scales — the uniform path cannot produce this: {:?}",
        art.schedule.per_net_scale_log2
    );
    assert!(
        target
            .mul_vref_options_v
            .iter()
            .any(|v| v.0 == art.schedule.mul_vref_v),
        "the multiplier reference is still chosen FROM the realizable set: {}",
        art.schedule.mul_vref_v
    );
    // The INDEPENDENT checker (untouched by O4) judges the per-net claim.
    let (cells, claim) = to_range_claim(&art.schedule, &art.cert);
    assert!(
        check_range_claim(&cells, &claim),
        "the per-net certificate must certify"
    );

    // Tamper: shrinking one net's claimed bound must be caught.
    let mut lying = claim.clone();
    lying.node_hi_v[0] = 0.0;
    lying.node_lo_v[0] = 0.0;
    assert!(
        !check_range_claim(&cells, &lying),
        "the checker still bites on per-net claims"
    );
}

#[test]
fn uniform_sufficient_regions_stay_uniform() {
    for target in [AnalogTarget::ideal(), AnalogTarget::sky130_fpaa()] {
        let art = analog_artifact_from_source(DOT3, "dot3", &target).expect("lowers");
        assert!(
            art.schedule
                .per_net_scale_log2
                .iter()
                .all(|e| *e == art.schedule.scale_log2),
            "no creep: a uniform-sufficient schedule stays uniform on {}",
            target.name
        );
    }
}

#[test]
fn the_physical_eval_agrees_with_the_ideal_math_within_the_budget() {
    let target = AnalogTarget::sky130_fpaa();
    let art = analog_artifact_from_source(SPIKE, "spike", &target).expect("lowers");
    assert!(art.budget.recheck(), "the budget replays");
    assert!(art.budget.eps_math > 0.0 && art.budget.eps_math.is_finite());
    assert!(
        art.budget
            .terms
            .iter()
            .any(|t| t.source == ErrorSource::WeightQuant),
        "the realized-vs-ideal snap is itemized: {:?}",
        art.budget.terms
    );
    for probe in [[0.5, 0.25], [-1.0, 1.0], [0.9, -0.7], [0.0, 0.0]] {
        let ideal = eval_analog_dc(&art.netlist, &probe);
        let physical = eval_analog_quantized(&art.schedule, &target, &probe);
        assert!(
            (physical - ideal).abs() <= art.budget.eps_math,
            "probe {probe:?}: physical {physical} vs ideal {ideal} exceeds ε {}",
            art.budget.eps_math
        );
    }
}
