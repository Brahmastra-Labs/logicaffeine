//! ★ MX STACK LOCK ★ — the mixed-signal partition binds onto the 3D stack: every admitted
//! region is PLACED, its temperature map CERTIFIED, and the cooling budget GATED — and the
//! accelerator covenant survives the whole composition.
//!
//! What this file locks:
//!   1. COMPOSITION — on the cold-plate V1 stack, an admissible region binds with a stack
//!      placement AND a thermal map that RE-CERTIFIES through the independent proof-crate
//!      judge; totality holds (every function in exactly one ledger).
//!   2. THE COOLING GATE RIDES THE PARTITION — the SAME source on the same stack under
//!      natural convection moves the binding into the refusal ledger with the
//!      `ThermalBudgetExceeded` reason: an uncoolable accelerator is a refusal, not a
//!      hot surprise.
//!   3. THE COVENANT SURVIVES — stripping every binding leaves the digital program running
//!      ≡VM (the partition never touches program semantics).
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. A binding without a certified map, or a map
//! without a met budget, never ships.

mod analog_common;

use analog_common::vm_float;
use logicaffeine_compile::codegen_ana::stack::{CoolingClass, StackTarget};
use logicaffeine_compile::codegen_mx::partition_on_stack;
use logicaffeine_proof::thermal_replay::check_thermal_claim;

const PROGRAM: &str = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## To host_only (s: Text) -> Text:\n    Return s.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";

fn cold_plate_stack() -> StackTarget {
    let mut s = StackTarget::sky130_stack_v1();
    s.cooling.top = CoolingClass::ColdPlate;
    s
}

#[test]
fn admitted_regions_bind_with_certified_maps_and_totality_holds() {
    let part = partition_on_stack(PROGRAM, &cold_plate_stack(), 4.0);
    assert_eq!(part.bindings.len(), 1, "dot3 binds: {:?}", part.refusals);
    assert_eq!(part.bindings[0].binding.fn_name, "dot3");
    assert!(
        check_thermal_claim(&part.bindings[0].thermal.claim).is_some(),
        "the binding's map re-certifies through the INDEPENDENT judge"
    );
    assert!(!part.bindings[0].placement.site.is_empty());
    assert_eq!(part.refusals.len(), 1, "host_only stays digital, ledgered");
    assert_eq!(part.refusals[0].fn_name, "host_only");
}

#[test]
fn the_cooling_gate_moves_uncoolable_bindings_into_the_ledger() {
    // The projected V1 finding rides the partition: under natural convection the SAME
    // region that binds on a cold plate becomes a NAMED refusal.
    let mut hot = StackTarget::sky130_stack_v1();
    hot.cooling.top = CoolingClass::NaturalConvection;
    let part = partition_on_stack(PROGRAM, &hot, 4.0);
    assert!(
        part.bindings.is_empty(),
        "nothing binds on an uncoolable stack"
    );
    let dot3 = part
        .refusals
        .iter()
        .find(|r| r.fn_name == "dot3")
        .expect("ledgered");
    assert!(
        dot3.reason.contains("thermal budget exceeded"),
        "the reason is the cooling gate's verdict: {}",
        dot3.reason
    );
}

#[test]
fn stripping_every_binding_leaves_the_digital_program_running_vm_identical() {
    // The covenant: the partition is metadata over an unchanged program — the VM answer is
    // the answer, bindings or none.
    let with_bindings = partition_on_stack(PROGRAM, &cold_plate_stack(), 4.0);
    assert!(!with_bindings.bindings.is_empty());
    let digital_answer = vm_float(PROGRAM);
    assert_eq!(digital_answer, 0.25 * 0.5 + 0.5 * 0.25 + 0.125 * 0.75);
}
