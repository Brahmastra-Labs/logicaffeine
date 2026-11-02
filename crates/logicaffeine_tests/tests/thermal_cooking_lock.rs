//! ★ THERMAL COOKING LOCK ★ — the crown anti-vacuity of the thermal coupling: on a stack
//! whose analog tiers sit over a hot digital core, the heat-and-adjacency-blind layout
//! EXCEEDS an accuracy budget the aware layout MEETS — and deleting the thermal terms makes
//! the blind layout FALSELY pass, proving those terms are load-bearing.
//!
//! The three strict inequalities (all on measured, deterministic budgets):
//!   1. ε_aware(thermal) < ε_blind(thermal) — hot scattering costs accuracy;
//!   2. ε_blind(no thermal terms) < ε_blind(thermal) — the thermal terms actually bite on
//!      the blind layout (its cross-tier legs straddle a real temperature gradient);
//!   3. therefore a demand θ exists with ε_aware ≤ θ < ε_blind where the blind layout
//!      WITHOUT the thermal terms would sneak under θ — the exact dishonesty the coupling
//!      exists to prevent.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. If a smarter placer or model lands, these margins
//! ratchet — they never relax.

use logicaffeine_compile::codegen_ana::stack::{
    CoolingBoundary, Material, StackTarget, Tier, TierFabric, VerticalClass,
};
use logicaffeine_compile::codegen_ana::stack_place::{
    error_budget_nonideal_stacked, error_budget_thermal_stacked, place_stack, place_stack_blind,
};
use logicaffeine_compile::codegen_ana::thermal::{thermal_default_opts, thermal_map};
use logicaffeine_compile::codegen_ana::{
    analog_artifact_from_source, sky130_nonideal, AnalogTarget,
};
use logicaffeine_proof::thermal_replay::check_thermal_claim;

const CHAIN: &str = "## To chain (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return ((a * b) * 0.5 + (b * c) * 0.25) * 0.5 + ((a * c) * 0.125 + a * 0.0625) * 0.25.\n\n## Main\n    Show chain(0.5, 0.25, 0.75).\n";

const HOP_COST: f64 = 4.0;

/// Digital heat source at the bottom, TWO analog tiers stacked over it — the deliberately
/// cooked configuration.
fn analog_over_hot_digital() -> StackTarget {
    let analog = |name: &'static str| {
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
    let digital = StackTarget::sky130_stack_v1().tiers.remove(0);
    StackTarget {
        name: "analog-over-hot-digital",
        tiers: vec![digital, analog("analog-lo"), analog("analog-hi")],
        gaps: vec![
            vec![VerticalClass::tsv_v1(64), VerticalClass::microbump_v1(128)],
            vec![VerticalClass::tsv_v1(64)],
        ],
        cooling: CoolingBoundary::forced_air_default(),
    }
}

#[test]
fn blind_stacking_over_a_hot_core_exceeds_the_budget_aware_placement_meets() {
    let target = AnalogTarget::ideal();
    let art = analog_artifact_from_source(CHAIN, "chain", &target).expect("lowers");
    let stack = analog_over_hot_digital();
    let model = sky130_nonideal();
    // The 3×3 tiers get a 3×3 thermal grid: site-exact coarsening (finer than the default is
    // pointless here, and smaller exact solves keep the suite fast).
    let mut opts = thermal_default_opts();
    opts.analog_grid = (3, 3);

    let aware = place_stack(&art.netlist, &stack, HOP_COST).expect("aware places");
    let blind = place_stack_blind(&art.netlist, &stack, HOP_COST).expect("blind places");
    let map_aware = thermal_map(&stack, &art.netlist, &aware, &opts).expect("solves");
    let map_blind = thermal_map(&stack, &art.netlist, &blind, &opts).expect("solves");
    assert!(
        check_thermal_claim(&map_aware.claim).is_some(),
        "both maps certify"
    );
    assert!(check_thermal_claim(&map_blind.claim).is_some());

    let eps_aware = error_budget_thermal_stacked(
        &art.netlist,
        &art.cert,
        &art.schedule,
        &target,
        &model,
        &aware,
        &map_aware,
        &stack,
        &opts,
    );
    let eps_blind = error_budget_thermal_stacked(
        &art.netlist,
        &art.cert,
        &art.schedule,
        &target,
        &model,
        &blind,
        &map_blind,
        &stack,
        &opts,
    );
    let eps_blind_no_thermal = error_budget_nonideal_stacked(
        &art.netlist,
        &art.cert,
        &art.schedule,
        &target,
        &model,
        &blind,
    );
    assert!(eps_aware.recheck() && eps_blind.recheck() && eps_blind_no_thermal.recheck());

    // 1. Hot scattering costs accuracy.
    assert!(
        eps_aware.eps_math < eps_blind.eps_math,
        "aware {} must beat blind {} with temperature priced in",
        eps_aware.eps_math,
        eps_blind.eps_math
    );
    // 2. The thermal terms bite on the blind layout.
    assert!(
        eps_blind_no_thermal.eps_math < eps_blind.eps_math,
        "the blind layout's thermal terms must be real: sans {} vs with {}",
        eps_blind_no_thermal.eps_math,
        eps_blind.eps_math
    );
    // 3. The cooking statement: a demand exists that separates them — and deleting the
    //    thermal terms would smuggle the blind layout under it.
    let theta = (eps_aware.eps_math.max(eps_blind_no_thermal.eps_math) + eps_blind.eps_math) / 2.0;
    assert!(eps_aware.eps_math <= theta, "aware meets the demand");
    assert!(eps_blind.eps_math > theta, "blind exceeds it");
    assert!(
        eps_blind_no_thermal.eps_math <= theta,
        "without the thermal terms the blind layout would FALSELY pass — the coupling is \
         load-bearing, not decorative"
    );
}
