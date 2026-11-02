//! ★ ANALOG SELECT LOCK ★ — data-dependent control ON the analog plane: the comparator-driven
//! switch is a real tile, the refusal seam became the lowering, and the comparator's
//! ambiguity is PRICED, never hidden.
//!
//! What this file locks:
//!   1. THE SEAM IS A TILE — an `If a < b:` region lowers (difference Sum + `Select`), runs
//!      ≡VM on the ideal tier at probes on BOTH branches, and its certificate passes the
//!      independent checker's hull-union Select rule.
//!   2. THE BAND IS PRICED — with the sky130 model, a select whose decision box straddles
//!      the comparator's ±k·σ offset band carries an itemized `CompareBand` term equal to
//!      the branch hull it might wander across; a decision box that CLEARS the band carries
//!      none (the certificate takes the decided branch, not the hull).
//!   3. HONEST REFUSALS SURVIVE — equality selects still refuse (a continuous comparator has
//!      no measure-zero decision).
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. The band term never disappears while the physics
//! that causes it remains; ≡VM at the ideal tier is the floor, never widened.

mod analog_common;

use analog_common::vm_float;
use logicaffeine_compile::codegen_ana::{
    analog_artifact_from_source, error_budget_nonideal, eval_analog_dc, place_analog,
    sky130_nonideal, to_range_claim, AnalogCell, AnalogTarget, ErrorSource,
};
use logicaffeine_proof::interval_replay::check_range_claim;

/// The supported if-conversion shape (Let/Set); the decision box straddles zero (a − b spans
/// [−2, 2]).
const PICK: &str = "## To pick (a: Float, b: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Let r be b * 0.25.\n    If a < b:\n        Set r to a * 0.5.\n    Return r.\n\n## Main\n    Show pick(0.5, 0.25).\n";

/// The decision clears the band: a − (−3) ∈ [2, 4] — never the Set branch, and the
/// comparator knows it at certification time.
const DECIDED: &str = "## To sure (a: Float, b: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Let r be b * 0.25.\n    If a < -3.0:\n        Set r to a * 0.5.\n    Return r.\n\n## Main\n    Show sure(0.5, 0.25).\n";

#[test]
fn the_refusal_seam_became_a_tile_and_runs_vm_identical_on_ideal() {
    let target = AnalogTarget::ideal();
    let art = analog_artifact_from_source(PICK, "pick", &target).expect("the select lowers");
    assert!(
        art.netlist
            .cells
            .iter()
            .any(|c| matches!(c, AnalogCell::Select { .. })),
        "vacuity guard: the Select tile is really in the netlist"
    );
    let (cells, claim) = to_range_claim(&art.schedule, &art.cert);
    assert!(
        check_range_claim(&cells, &claim),
        "the hull-union certificate certifies"
    );

    // ≡VM at probes on BOTH branches and near the boundary.
    for (a, b) in [
        (0.5, 0.25),
        (-0.75, 0.5),
        (0.3, 0.30001),
        (1.0, -1.0),
        (0.0, 0.0),
    ] {
        let vm = vm_float(&format!(
            "## To pick (a: Float, b: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Let r be b * 0.25.\n    If a < b:\n        Set r to a * 0.5.\n    Return r.\n\n## Main\n    Show pick({a}, {b}).\n"
        ));
        let ideal = eval_analog_dc(&art.netlist, &[a, b]);
        assert_eq!(ideal, vm, "ideal tier ≡ VM at ({a}, {b})");
    }
}

#[test]
fn the_ambiguity_band_is_priced_iff_the_decision_cannot_clear_it() {
    let target = AnalogTarget::sky130_fpaa();
    let model = sky130_nonideal();

    let straddle = analog_artifact_from_source(PICK, "pick", &target).expect("lowers");
    let placement = place_analog(&straddle.schedule.quantized, &target).expect("places");
    let budget = error_budget_nonideal(
        &straddle.netlist,
        &straddle.cert,
        &straddle.schedule,
        &target,
        &model,
        &placement,
    );
    assert!(budget.recheck());
    let band = budget
        .terms
        .iter()
        .find(|t| t.source == ErrorSource::CompareBand);
    let band = band.expect("a straddling decision must carry the CompareBand term");
    assert!(band.eps_math > 0.0);

    let decided = analog_artifact_from_source(DECIDED, "sure", &target).expect("lowers");
    let placement2 = place_analog(&decided.schedule.quantized, &target).expect("places");
    let budget2 = error_budget_nonideal(
        &decided.netlist,
        &decided.cert,
        &decided.schedule,
        &target,
        &model,
        &placement2,
    );
    assert!(
        !budget2
            .terms
            .iter()
            .any(|t| t.source == ErrorSource::CompareBand),
        "a decision that clears the band costs nothing: {:?}",
        budget2.terms
    );
    // And the decided certificate is branch-tight, proven by the checker (hull would also
    // pass, so pin tightness directly: the output box is the Otherwise branch's box).
    let out_bound = decided.cert.per_net.last().unwrap().math;
    assert!(
        out_bound.lo >= -0.2500001 && out_bound.hi <= 0.2500001,
        "b*0.25 box, not the hull: {out_bound:?}"
    );
}

#[test]
fn equality_selects_still_refuse_by_name() {
    let eq = "## To eq (a: Float, b: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Let r be b * 0.25.\n    If a is b:\n        Set r to a * 0.5.\n    Return r.\n\n## Main\n    Show eq(0.5, 0.25).\n";
    match std::panic::catch_unwind(|| analog_artifact_from_source(eq, "eq", &AnalogTarget::ideal()))
    {
        Ok(Err(e)) => {
            let msg = e.to_string();
            assert!(
                msg.contains("equality") || msg.contains("measure-zero"),
                "refusal names the physics: {msg}"
            );
        }
        Ok(Ok(_)) => panic!("a continuous comparator has no measure-zero decision"),
        Err(_) => {} // an upstream shape panic is also fail-closed — never a wrong circuit
    }
}

#[test]
fn early_return_diamonds_fail_closed_instead_of_dropping_a_branch() {
    // THE HOLE THIS PHASE CLOSED: cutting the body at the first Return used to compile the
    // then-branch ALONE — a silent miscompile. Now the escaping branch is refused loudly.
    let early = "## To pick (a: Float, b: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    If a < b:\n        Return a * 0.5.\n    Otherwise:\n        Return b * 0.25.\n\n## Main\n    Show pick(0.5, 0.25).\n";
    let out = std::panic::catch_unwind(|| {
        analog_artifact_from_source(early, "pick", &AnalogTarget::ideal())
    });
    match out {
        Err(_) => {}     // the loud refusal
        Ok(Err(_)) => {} // a typed refusal is equally fail-closed
        Ok(Ok(art)) => panic!(
            "an early-return diamond must never compile by dropping a branch: {:?}",
            art.netlist.cells
        ),
    }
}
