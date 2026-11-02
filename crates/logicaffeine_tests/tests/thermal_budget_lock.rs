//! ★ THERMAL BUDGET LOCK ★ — temperature is priced into ε: the certified thermal map's
//! rises and gradients become itemized drift terms in the physical budget.
//!
//! What this file locks:
//!   1. COUPLING — the thermally-coupled stacked budget carries `ThermalOffset` (absolute
//!      rise × tempco) whenever the map is above ambient, replays exactly (`recheck`), and
//!      is STRICTLY larger than the temperature-blind stacked budget: heat costs ε, never
//!      the reverse.
//!   2. ITEMIZATION — the thermal terms are named `ErrorSource`s (taxonomy-classified in the
//!      effect grid), not smeared into other figures.
//!   3. DETERMINISM — same stack, same placement, same map, same budget, forever.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. A hotter configuration never gets a smaller ε.

use logicaffeine_compile::codegen_ana::stack::StackTarget;
use logicaffeine_compile::codegen_ana::stack_place::{
    error_budget_nonideal_stacked, error_budget_thermal_stacked, place_stack,
};
use logicaffeine_compile::codegen_ana::thermal::{thermal_default_opts, thermal_map};
use logicaffeine_compile::codegen_ana::{
    analog_artifact_from_source, sky130_nonideal, AnalogTarget, ErrorSource,
};

const DOT3: &str = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";

const HOP_COST: f64 = 4.0;

#[test]
fn the_thermal_budget_prices_the_map_and_replays() {
    let target = AnalogTarget::sky130_fpaa();
    let art = analog_artifact_from_source(DOT3, "dot3", &target).expect("lowers");
    let stack = StackTarget::sky130_stack_v1();
    let model = sky130_nonideal();
    let opts = thermal_default_opts();
    let placement = place_stack(&art.netlist, &stack, HOP_COST).expect("places");
    let map = thermal_map(&stack, &art.netlist, &placement, &opts).expect("solves");

    let blind_to_heat = error_budget_nonideal_stacked(
        &art.netlist,
        &art.cert,
        &art.schedule,
        &target,
        &model,
        &placement,
    );
    let coupled = error_budget_thermal_stacked(
        &art.netlist,
        &art.cert,
        &art.schedule,
        &target,
        &model,
        &placement,
        &map,
        &stack,
        &opts,
    );
    assert!(coupled.recheck(), "the coupled budget replays term-exactly");
    assert!(
        coupled.eps_math > blind_to_heat.eps_math,
        "a die above ambient must cost ε: coupled {} vs blind {}",
        coupled.eps_math,
        blind_to_heat.eps_math
    );
    assert!(
        coupled
            .terms
            .iter()
            .any(|t| t.source == ErrorSource::ThermalOffset && t.eps_math > 0.0),
        "the offset-drift term is itemized by name: {:?}",
        coupled.terms
    );

    let again = error_budget_thermal_stacked(
        &art.netlist,
        &art.cert,
        &art.schedule,
        &target,
        &model,
        &placement,
        &map,
        &stack,
        &opts,
    );
    assert_eq!(again, coupled, "deterministic forever");
}
