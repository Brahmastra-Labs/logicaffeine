//! ★ TRANSLINEAR RATIO LOCK ★ — division joins the plane: the `FDivF` refusal seam became
//! the Gilbert-loop lowering, with the divisor's certified box clearing zero or refusing
//! typed.
//!
//! What this file locks:
//!   1. THE SEAM LOWERS — `a / b` in a float region produces a `Div` tile, schedules at unit
//!      scale, certifies through the independent checker, and runs ≡VM on the ideal tier.
//!   2. STARVATION REFUSES — a divisor whose certified box crosses (or touches) zero is the
//!      typed `DivisorRangeSpansZero`, at the producer AND the checker.
//!   3. `sqrt(x)` LOWERS through the same family (the `FSqrtF` seam).
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. The zero-clearing demand is the translinear
//! loop's physics, not a tolerance.

mod analog_common;

use analog_common::vm_float;
use logicaffeine_compile::codegen_ana::{
    analog_artifact_from_source, eval_analog_dc, to_range_claim, AnaLowerError, AnaUnplaceable,
    AnalogCell, AnalogTarget,
};
use logicaffeine_proof::interval_replay::check_range_claim;

const RATIO: &str = "## To r (a: Float, b: Float) -> Float:\n    Assume a is between -0.5 and 0.5.\n    Assume b is between 0.5 and 0.7.\n    Return (a * 0.25) / b.\n\n## Main\n    Show r(0.4, 0.6).\n";

#[test]
fn division_lowers_certifies_and_runs_vm_identical_on_ideal() {
    let target = AnalogTarget::ideal();
    let art = analog_artifact_from_source(RATIO, "r", &target).expect("the ratio lowers");
    assert!(
        art.netlist
            .cells
            .iter()
            .any(|c| matches!(c, AnalogCell::Div { .. })),
        "vacuity guard: the Gilbert-loop tile is really there"
    );
    assert_eq!(
        art.schedule.scale_log2, 0,
        "translinear pins the unit scale"
    );
    let (cells, claim) = to_range_claim(&art.schedule, &art.cert);
    assert!(
        check_range_claim(&cells, &claim),
        "the ratio certificate certifies"
    );
    assert!(art.budget.recheck());

    for (a, b) in [(0.4, 0.6), (-0.5, 0.5), (0.0, 0.7), (0.25, 0.55)] {
        let vm = vm_float(&format!(
            "## To r (a: Float, b: Float) -> Float:\n    Assume a is between -0.5 and 0.5.\n    Assume b is between 0.5 and 0.7.\n    Return (a * 0.25) / b.\n\n## Main\n    Show r({a}, {b}).\n"
        ));
        let ideal = eval_analog_dc(&art.netlist, &[a, b]);
        assert_eq!(ideal, vm, "ideal ≡ VM at ({a}, {b})");
    }
}

#[test]
fn a_zero_straddling_divisor_refuses_typed_at_producer_and_checker() {
    let target = AnalogTarget::ideal();
    let straddle = "## To r (a: Float, b: Float) -> Float:\n    Assume a is between -0.5 and 0.5.\n    Assume b is between -0.5 and 0.5.\n    Return (a * 0.25) / b.\n\n## Main\n    Show r(0.4, 0.3).\n";
    match analog_artifact_from_source(straddle, "r", &target) {
        Err(AnaLowerError::Unplaceable(AnaUnplaceable::DivisorRangeSpansZero {
            lo, hi, ..
        })) => {
            assert!(
                lo <= 0.0 && hi >= 0.0,
                "the refusal carries the offending box: [{lo}, {hi}]"
            );
        }
        other => panic!("a straddling divisor must refuse DivisorRangeSpansZero, got {other:?}"),
    }

    // The checker enforces the same gate on a good artifact whose claim is tampered into a
    // straddling divisor box.
    let art = analog_artifact_from_source(RATIO, "r", &target).expect("lowers");
    let (cells, mut claim) = to_range_claim(&art.schedule, &art.cert);
    claim.input_lo[1] = -0.5; // the divisor input's box now straddles zero
    assert!(
        !check_range_claim(&cells, &claim),
        "the checker rejects the straddling divisor"
    );
}

#[test]
fn sqrt_lowers_through_the_same_family() {
    let target = AnalogTarget::ideal();
    let src = "## To s (x: Float) -> Float:\n    Assume x is between 0.1 and 0.5.\n    Return sqrt(x).\n\n## Main\n    Show s(0.25).\n";
    let art = analog_artifact_from_source(src, "s", &target).expect("sqrt lowers");
    assert!(
        art.netlist
            .cells
            .iter()
            .any(|c| matches!(c, AnalogCell::Sqrt { .. })),
        "the FSqrtF seam became the tile"
    );
    for x in [0.1, 0.25, 0.5] {
        assert_eq!(
            eval_analog_dc(&art.netlist, &[x]),
            x.sqrt(),
            "ideal IS the math at {x}"
        );
    }
}
