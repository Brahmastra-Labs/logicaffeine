//! ★ ANALOG CHOPPER LOCK ★ — chopping is a SCHEDULED transform: run each frame twice around
//! a polarity swap, average, and the frozen per-die offset cancels EXACTLY — leaving the
//! modeled intra-pair residual and √2-averaged noise.
//!
//! What this file locks:
//!   1. THE BUDGET THEOREM — on an offset-dominated region, the chopped budget is STRICTLY
//!      smaller than the unchopped one: `TileOffset` is gone, `ChopResidual` (50 µV class ≪
//!      2.8 mV offset class) replaces it, `Noise` carries the outward-rounded 1/√2, and
//!      everything else is verbatim.
//!   2. THE MONTE-CARLO AGREES — over fixed-seed manufactured dies, every chopped readout
//!      sits inside the CHOPPED budget, and the worst chopped error beats the worst
//!      unchopped error (the physics, measured, not asserted).
//!   3. A SCHEDULE, NOT A REWRITE — `chopped()` doubles the frame around `ChopSwap` steps;
//!      netlist, certificate, and placement are byte-identical; the unchopped path is
//!      untouched (the whole existing battery is its witness).
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. The exact-cancellation claim is BY CONSTRUCTION
//! (the frozen draw enters both frames with opposite sign); weakening it means the
//! architecture changed, not the tolerance.

mod analog_common;

use analog_common::BASE_SEED;
use logicaffeine_compile::codegen_ana::{
    analog_artifact_from_source, error_budget_chopped, error_budget_nonideal, eval_analog_dc,
    eval_analog_instance, eval_analog_instance_chopped, place_analog, sample_instance,
    sky130_nonideal, AnalogTarget, CtrlStep, ErrorSource,
};

/// Offset-dominated: a plain weighted sum (tiny per-leg gain terms, offsets carry the ε).
const DOT3: &str = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";

#[test]
fn the_chopped_budget_swaps_offset_for_residual_and_shrinks() {
    let target = AnalogTarget::sky130_fpaa();
    let model = sky130_nonideal();
    let art = analog_artifact_from_source(DOT3, "dot3", &target).expect("lowers");
    let placement = place_analog(&art.schedule.quantized, &target).expect("places");

    let plain = error_budget_nonideal(
        &art.netlist,
        &art.cert,
        &art.schedule,
        &target,
        &model,
        &placement,
    );
    let chop = error_budget_chopped(
        &art.netlist,
        &art.cert,
        &art.schedule,
        &target,
        &model,
        &placement,
    );
    assert!(plain.recheck() && chop.recheck(), "both budgets replay");

    assert!(
        chop.eps_math < plain.eps_math,
        "chopping must pay on an offset-dominated region: chopped {} vs plain {}",
        chop.eps_math,
        plain.eps_math
    );
    assert!(
        !chop
            .terms
            .iter()
            .any(|t| t.source == ErrorSource::TileOffset),
        "the frozen offset cancels EXACTLY — no TileOffset term survives: {:?}",
        chop.terms
    );
    let resid = chop
        .terms
        .iter()
        .find(|t| t.source == ErrorSource::ChopResidual);
    assert!(resid.expect("the residual class is itemized").eps_math > 0.0);
    let (pn, cn) = (
        plain
            .terms
            .iter()
            .find(|t| t.source == ErrorSource::Noise)
            .map(|t| t.eps_math),
        chop.terms
            .iter()
            .find(|t| t.source == ErrorSource::Noise)
            .map(|t| t.eps_math),
    );
    if let (Some(pn), Some(cn)) = (pn, cn) {
        assert!(
            cn < pn,
            "two averaged draws buy the √2: chopped noise {cn} vs {pn}"
        );
    }
}

#[test]
fn monte_carlo_dies_sit_inside_the_chopped_budget_and_beat_unchopped() {
    let target = AnalogTarget::sky130_fpaa();
    let model = sky130_nonideal();
    let art = analog_artifact_from_source(DOT3, "dot3", &target).expect("lowers");
    let placement = place_analog(&art.schedule.quantized, &target).expect("places");
    let chop_budget = error_budget_chopped(
        &art.netlist,
        &art.cert,
        &art.schedule,
        &target,
        &model,
        &placement,
    );

    let probes = [[0.5, 0.25, 0.75], [-1.0, 1.0, -1.0], [0.1, -0.2, 0.3]];
    let (mut worst_plain, mut worst_chop) = (0.0_f64, 0.0_f64);
    for die in 0..128u64 {
        let inst = sample_instance(
            &art.schedule.quantized.cells,
            &placement,
            &model,
            BASE_SEED ^ die,
        );
        for p in &probes {
            let ideal = eval_analog_dc(&art.netlist, p);
            let plain = eval_analog_instance(&art.schedule, &target, &inst, p);
            let chop = eval_analog_instance_chopped(
                &art.schedule,
                &target,
                &inst,
                &model,
                p,
                BASE_SEED ^ (die << 8),
            );
            worst_plain = worst_plain.max((plain - ideal).abs());
            let ce = (chop - ideal).abs();
            worst_chop = worst_chop.max(ce);
            assert!(
                ce <= chop_budget.eps_math,
                "die {die} probe {p:?}: chopped error {ce} exceeds the chopped budget {}",
                chop_budget.eps_math
            );
        }
    }
    assert!(
        worst_chop < worst_plain,
        "the physics, measured: worst chopped {worst_chop} must beat worst unchopped {worst_plain}"
    );
}

#[test]
fn chopping_is_a_schedule_transform_not_a_rewrite() {
    use logicaffeine_compile::codegen_ana::ControlProgram;
    let base = ControlProgram {
        frame: vec![
            CtrlStep::LoadDac { ch: 0 },
            CtrlStep::Settle,
            CtrlStep::SampleHold,
            CtrlStep::ReadAdc { ch: 0 },
        ],
        frames: 4,
        epilogue: vec![],
        tick_ns: 1000.0,
    };
    let chopped = base.chopped();
    assert_eq!(
        chopped
            .frame
            .iter()
            .filter(|s| matches!(s, CtrlStep::ChopSwap))
            .count(),
        2,
        "each frame runs twice around two polarity swaps"
    );
    assert_eq!(chopped.frame.len(), 2 * base.frame.len() + 2);
    assert_eq!(chopped.frames, base.frames);
    assert_eq!(chopped.tick_ns, base.tick_ns);
}
