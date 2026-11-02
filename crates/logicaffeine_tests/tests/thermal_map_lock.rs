//! ★ THERMAL MAP LOCK ★ — the stack's certified temperature map: exactly solved, judged by
//! the independent proof-crate checker, with the TSV heat lever MEASURED, not asserted.
//!
//! What this file locks:
//!   1. CERTIFICATION — the V1 stack's map re-certifies through
//!      `proof::thermal_replay::check_thermal_claim` (Laplacian reassembled independently,
//!      exact equality), and the checker's hotspot agrees with the producer's.
//!   2. THE HOTSPOT IS REAL — the digital core's measured power (sky130_power.json) dwarfs
//!      the modeled analog bias, so the hottest node sits on the DIGITAL tier:
//!      analog-over-hot-digital is a thermal statement the map exposes rather than hides.
//!   3. THE LEVER — replacing the gap's TSVs with bumps-only STRICTLY raises the peak:
//!      vertical copper is a cooling resource the compiler can spend, measured here.
//!   4. FAIL-CLOSED — a stack with no way to shed heat refuses `ThermalModelDisconnected`;
//!      the map is deterministic forever.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. The checker stays independent; the lever stays
//! strict; a hotter configuration never gets a cooler map.

use logicaffeine_compile::codegen_ana::stack::{StackTarget, VerticalClass};
use logicaffeine_compile::codegen_ana::stack_place::place_stack;
use logicaffeine_compile::codegen_ana::thermal::{thermal_default_opts, thermal_map};
use logicaffeine_compile::codegen_ana::{analog_netlist_from_source, AnalogTarget};
use logicaffeine_proof::thermal_replay::check_thermal_claim;

const DOT3: &str = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";

const HOP_COST: f64 = 4.0;

#[test]
fn the_v1_stack_map_certifies_and_the_hotspot_sits_on_the_digital_tier() {
    let stack = StackTarget::sky130_stack_v1();
    let netlist =
        analog_netlist_from_source(DOT3, "dot3", &AnalogTarget::sky130_fpaa()).expect("lowers");
    let placement = place_stack(&netlist, &stack, HOP_COST).expect("places");
    let map = thermal_map(&stack, &netlist, &placement, &thermal_default_opts()).expect("solves");

    let verdict = check_thermal_claim(&map.claim)
        .expect("the exactly-solved map must certify through the INDEPENDENT checker");
    assert_eq!(
        verdict.hottest_node, map.hottest,
        "producer and judge agree on the hotspot"
    );
    assert_eq!(
        map.node_tier[map.hottest], 0,
        "the digital core's measured power is the heat source — the hotspot is on ITS tier"
    );
    assert!(map.delta_t_c[map.hottest] > 0.0, "power in, temperature up");
    assert!(
        map.delta_t_c.iter().all(|t| *t >= 0.0),
        "no node below ambient under non-negative injection"
    );

    // Deterministic forever.
    let again = thermal_map(&stack, &netlist, &placement, &thermal_default_opts()).expect("replay");
    assert_eq!(again.claim, map.claim);
}

#[test]
fn tsvs_are_a_measured_cooling_lever() {
    let netlist =
        analog_netlist_from_source(DOT3, "dot3", &AnalogTarget::sky130_fpaa()).expect("lowers");
    let with_tsvs = StackTarget::sky130_stack_v1();
    let mut bumps_only = StackTarget::sky130_stack_v1();
    bumps_only.gaps[0] = vec![VerticalClass::microbump_v1(256)];

    let pa = place_stack(&netlist, &with_tsvs, HOP_COST).expect("places");
    let pb = place_stack(&netlist, &bumps_only, HOP_COST).expect("places");
    let ma = thermal_map(&with_tsvs, &netlist, &pa, &thermal_default_opts()).expect("solves");
    let mb = thermal_map(&bumps_only, &netlist, &pb, &thermal_default_opts()).expect("solves");
    assert!(check_thermal_claim(&ma.claim).is_some());
    assert!(check_thermal_claim(&mb.claim).is_some());

    let (peak_tsv, peak_bump) = (ma.delta_t_c[ma.hottest], mb.delta_t_c[mb.hottest]);
    assert!(
        peak_tsv < peak_bump,
        "64 TSVs (3.15e-4 W/K each, derived from sourced k_Cu) must strictly lower the peak: \
         {peak_tsv} °C with vs {peak_bump} °C without"
    );
}

#[test]
fn a_stack_with_nowhere_to_shed_heat_would_be_a_refusal_not_a_map() {
    // The refusal path is structural: exact_solve returns None on a singular system. The
    // public surface cannot even EXPRESS an uncooled stack (cooling classes always ground the
    // faces), so this probes the seam the S5 cooling phase will widen: the map must never
    // fabricate an equilibrium. We verify the certifier side: stripping the grounds from an
    // otherwise-true claim is rejected by the judge.
    let stack = StackTarget::sky130_stack_v1();
    let netlist =
        analog_netlist_from_source(DOT3, "dot3", &AnalogTarget::sky130_fpaa()).expect("lowers");
    let placement = place_stack(&netlist, &stack, HOP_COST).expect("places");
    let map = thermal_map(&stack, &netlist, &placement, &thermal_default_opts()).expect("solves");
    let mut adrift = map.claim.clone();
    adrift.grounds.clear();
    assert!(
        check_thermal_claim(&adrift).is_none(),
        "heat with nowhere to go is a rejection, never an equilibrium"
    );
}
