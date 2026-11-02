//! ★ STACK ANTI-VACUITY LOCK ★ — 3D-aware placement EARNS its keep, measured three ways.
//!
//! A cost function nobody beats is decoration. `place_stack_blind` is the deliberate straw
//! man (max-stride scatter, tier-alternating); the aware placer must beat it on:
//!   1. WIRE — total operand cost (lateral pitches + hop-weighted crossings);
//!   2. ε — the stacked non-ideality budget (cross-tier legs pay the categorical Pelgrom
//!      ceiling, so scattering matched clusters across dies COSTS accuracy, provably);
//!   3. VERTICALS — consumed scarce vias.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. If a smarter placer lands, it must beat the straw
//! man by MORE, never less — ratchet the margins, don't relax them.

use logicaffeine_compile::codegen_ana::stack::{
    CoolingBoundary, Material, StackTarget, Tier, TierFabric, VerticalClass,
};
use logicaffeine_compile::codegen_ana::stack_place::{
    error_budget_nonideal_stacked, place_stack, place_stack_blind,
};
use logicaffeine_compile::codegen_ana::{
    analog_artifact_from_source, sky130_nonideal, AnalogTarget,
};

const CHAIN: &str = "## To chain (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return ((a * b) * 0.5 + (b * c) * 0.25) * 0.5 + ((a * c) * 0.125 + a * 0.0625) * 0.25.\n\n## Main\n    Show chain(0.5, 0.25, 0.75).\n";

const HOP_COST: f64 = 4.0;

fn two_tier() -> StackTarget {
    let tier = |name: &'static str| {
        let mut t = AnalogTarget::ideal();
        t.rows = 3;
        t.cols = 3;
        Tier {
            name,
            fabric: TierFabric::Analog(t),
            material: Material::silicon(),
            thickness_um: 100.0,
        }
    };
    StackTarget {
        name: "two-tier-antivacuity",
        tiers: vec![tier("analog-lo"), tier("analog-hi")],
        gaps: vec![vec![VerticalClass::tsv_v1(64)]],
        cooling: CoolingBoundary::forced_air_default(),
    }
}

#[test]
fn aware_beats_blind_on_wire_epsilon_and_verticals() {
    let target = AnalogTarget::ideal();
    let art = analog_artifact_from_source(CHAIN, "chain", &target).expect("chain lowers");
    let netlist = &art.netlist;
    let model = sky130_nonideal();
    let stack = two_tier();

    let aware = place_stack(netlist, &stack, HOP_COST).expect("aware places");
    let blind = place_stack_blind(netlist, &stack, HOP_COST).expect("blind places");

    // 1. Wire.
    let (wa, wb) = (
        aware.wire_cost(netlist, HOP_COST),
        blind.wire_cost(netlist, HOP_COST),
    );
    assert!(wa < wb, "aware wire {wa} must beat blind {wb}");

    // 2. ε — the budget teaches the placer to keep matched clusters in-tier.
    let quantized_cells = &art.schedule.quantized;
    let _ = quantized_cells;
    let ba =
        error_budget_nonideal_stacked(netlist, &art.cert, &art.schedule, &target, &model, &aware);
    let bb =
        error_budget_nonideal_stacked(netlist, &art.cert, &art.schedule, &target, &model, &blind);
    assert!(ba.recheck() && bb.recheck(), "both budgets replay");
    assert!(
        ba.eps_math < bb.eps_math,
        "scattering matched clusters across dies must COST accuracy: aware ε {} vs blind ε {}",
        ba.eps_math,
        bb.eps_math
    );

    // 3. Scarce verticals.
    assert!(
        aware.vertical_used[0] < blind.vertical_used[0],
        "aware crossings {} must undercut blind {}",
        aware.vertical_used[0],
        blind.vertical_used[0]
    );
}
