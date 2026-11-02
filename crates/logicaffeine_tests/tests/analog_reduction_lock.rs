//! ★ ANALOG REDUCTION LOCK ★ — the flagship streamed-MAC lane: a float-accumulator loop over a
//! sequence becomes ONE analog tick datapath framed by the digital control plane, with the
//! accumulator RESIDENT IN ANALOG between frames (per-element DAC, one final ADC, zero
//! mid-stream conversions — the asymmetry that is the analog win).
//!
//! What this file locks:
//!   1. Lowering shapes — `acc + x` is a two-input Kirchhoff sum; `acc + x·x` streams through
//!      the four-quadrant `Mul`; the control program schedules exactly the declared maximum
//!      frames of [LoadDac → Settle → SampleHold] and one epilogue ADC read.
//!   2. The ideal sampled tier reproduces the VM's loop trajectory BIT-EXACTLY.
//!   3. The iterated certificate passes the INDEPENDENT reduction checker
//!      (`check_reduction_claim` — the same frame iteration, replayed by the proof crate).
//!   4. The iterated budget is SOUND: over deterministic fixed-seed batteries of random
//!      lengths and elements, the physical tier never leaves the compiler's own ε.
//!   5. Contracts are load-bearing: a missing length range or element range refuses
//!      `UnboundedInput`; a `Mul` stream whose accumulator range outgrows every realizable
//!      reference refuses `MulReferenceUnrealizable` (physics, not vibes).
//!   6. The crossbar canonicalization: an unrolled dot product folds to ONE summing column,
//!      and columns wider than the physical fan-in tree back down.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.

use logicaffeine_compile::codegen_ana::{
    analog_netlist_from_source, analog_reduction_from_source, eval_analog_sampled,
    eval_analog_sampled_quantized, fold_crossbar_for, reduction_error_budget, reduction_scale_pass,
    to_range_claim, ASrc, AnaLowerError, AnaUnplaceable, AnalogCell, AnalogTarget, CtrlStep,
};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_proof::interval_replay::{check_reduction_claim, RSrc};

const SUM: &str = "## To total (xs: Seq of Float) -> Float:\n    Assume length of xs is between 1 and 64.\n    Let mutable acc be 0.0.\n    Repeat for each x in xs:\n        Assume x is between -1.0 and 1.0.\n        Set acc to acc + x.\n    Return acc.\n\n## Main\n    Let mutable xs be a new Seq of Float.\n    Push 0.5 to xs.\n    Push 0.25 to xs.\n    Push -0.75 to xs.\n    Show total(xs).\n";

const SCALED: &str = "## To total (xs: Seq of Float) -> Float:\n    Assume length of xs is between 1 and 64.\n    Let mutable acc be 0.0.\n    Repeat for each x in xs:\n        Assume x is between -1.0 and 1.0.\n        Set acc to acc + x * 0.5.\n    Return acc.\n\n## Main\n    Let mutable xs be a new Seq of Float.\n    Push 0.5 to xs.\n    Push 0.25 to xs.\n    Push -0.75 to xs.\n    Show total(xs).\n";

/// Energy accumulation — the streamed four-quadrant multiplier. The 1..32 length contract is
/// LOAD-BEARING: 32 frames of `x²` bound the accumulator to 32, which the smallest realizable
/// reference can still scale into the rails (64 could not — see the refusal test).
const ENERGY: &str = "## To energy (xs: Seq of Float) -> Float:\n    Assume length of xs is between 1 and 32.\n    Let mutable acc be 0.0.\n    Repeat for each x in xs:\n        Assume x is between -1.0 and 1.0.\n        Set acc to acc + x * x.\n    Return acc.\n\n## Main\n    Let mutable xs be a new Seq of Float.\n    Push 0.5 to xs.\n    Push 0.25 to xs.\n    Push -0.75 to xs.\n    Show energy(xs).\n";

const CORPUS: &[(&str, &str, &str)] = &[
    ("sum", SUM, "total"),
    ("scaled", SCALED, "total"),
    ("energy", ENERGY, "energy"),
];

/// The Main-block stream every corpus program pushes.
const MAIN_XS: &[f64] = &[0.5, 0.25, -0.75];

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

fn vm_float(src: &str) -> f64 {
    let out = vm_outcome(src);
    assert_eq!(out.error, None, "corpus program must run on the VM:\n{src}");
    out.output
        .trim()
        .parse::<f64>()
        .unwrap_or_else(|_| panic!("VM output is not a float: {:?}", out.output))
}

// ============================================================
//  1. Lowering shapes + control framing
// ============================================================

#[test]
fn lowering_shapes_and_control_framing_are_pinned() {
    let target = AnalogTarget::ideal();

    let sum = analog_reduction_from_source(SUM, "total", &target).expect("sum lowers");
    assert_eq!(sum.tick.n_in, 2, "element + held accumulator");
    assert_eq!(sum.tick.cells.len(), 1);
    assert!(matches!(&sum.tick.cells[0], AnalogCell::Sum { inputs, .. } if inputs.len() == 2));
    assert_eq!((sum.min_len, sum.max_len), (1, 64));
    assert_eq!(sum.init, 0.0);
    assert_eq!((sum.elem.lo, sum.elem.hi), (-1.0, 1.0));

    assert_eq!(
        sum.control.frames, 64,
        "the control plane schedules the DECLARED maximum"
    );
    assert_eq!(
        sum.control.frame,
        vec![
            CtrlStep::LoadDac { ch: 0 },
            CtrlStep::Settle,
            CtrlStep::SampleHold
        ]
    );
    assert_eq!(sum.control.epilogue, vec![CtrlStep::ReadAdc { ch: 0 }]);
    assert_eq!(sum.control.total_steps(), 64 * 3 + 1);

    let energy = analog_reduction_from_source(ENERGY, "energy", &target).expect("energy lowers");
    assert!(
        energy
            .tick
            .cells
            .iter()
            .any(|c| matches!(c, AnalogCell::Mul { .. })),
        "x·x must stream through the four-quadrant Mul: {:?}",
        energy.tick.cells
    );
}

// ============================================================
//  2. Ideal sampled tier ≡ VM, bit-exact
// ============================================================

#[test]
fn ideal_sampled_trajectory_is_bit_exact_with_the_vm() {
    let target = AnalogTarget::ideal();
    for (label, src, fn_name) in CORPUS {
        let r = analog_reduction_from_source(src, fn_name, &target).expect("lowers");
        let got = eval_analog_sampled(&r, MAIN_XS);
        let want = vm_float(src);
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "`{label}`: the ideal sampled tier mirrors the VM loop op-for-op — bit-exact"
        );
    }
}

// ============================================================
//  3. The iterated certificate passes the independent checker
// ============================================================

#[test]
fn reduction_certificates_pass_the_independent_checker() {
    let target = AnalogTarget::ideal();
    for (label, src, fn_name) in CORPUS {
        let r = analog_reduction_from_source(src, fn_name, &target).expect("lowers");
        let (schedule, cert) = reduction_scale_pass(&r, &target).expect("schedules");
        let (cells, claim) = to_range_claim(&schedule, &cert);
        let out = match schedule.quantized.out {
            ASrc::In(k) => RSrc::In(k),
            ASrc::Val(v) => RSrc::Val(v),
            ASrc::Const(c) => RSrc::Const(c),
        };
        assert!(
            check_reduction_claim(&cells, out, r.init, r.max_len, &claim),
            "`{label}`: the independent frame-iteration checker must accept the claim"
        );
        // Tamper: an accumulator box that hides part of the trajectory is rejected.
        let mut lying = claim.clone();
        lying.input_hi[1] = r.init + (lying.input_hi[1] - r.init) / 1.0e3;
        assert!(
            !check_reduction_claim(&cells, out, r.init, r.max_len, &lying),
            "`{label}`: a trajectory-hiding accumulator claim must be rejected"
        );
    }
}

// ============================================================
//  4. The iterated budget is sound
// ============================================================

#[test]
fn sampled_quantized_evaluation_never_exceeds_the_compilers_own_epsilon() {
    for target in [AnalogTarget::ideal(), AnalogTarget::sky130_fpaa()] {
        for (i, (label, src, fn_name)) in CORPUS.iter().enumerate() {
            let r = analog_reduction_from_source(src, fn_name, &target).expect("lowers");
            let (schedule, cert) = match reduction_scale_pass(&r, &target) {
                Ok(sc) => sc,
                // The V1 chip's references stop at 0.125: a 32-frame energy stream honestly
                // outgrows it — the typed refusal IS the correct behavior for this pairing.
                Err(AnaLowerError::Unplaceable(AnaUnplaceable::MulReferenceUnrealizable {
                    ..
                })) if target.name == "sky130-fpaa" && *label == "energy" => {
                    continue;
                }
                Err(other) => panic!("`{label}` on `{}` must schedule: {other}", target.name),
            };
            let budget = reduction_error_budget(&r, &cert, &schedule, &target);
            assert!(
                budget.recheck(),
                "`{label}`: budget must replay from its terms"
            );

            let mut seed = BASE_SEED ^ (i as u64) ^ ((target.dac_bits as u64) << 40);
            let mut worst = 0.0_f64;
            for _ in 0..64 {
                let len = r.min_len + (splitmix(&mut seed) as usize) % (r.max_len - r.min_len + 1);
                let xs: Vec<f64> = (0..len)
                    .map(|_| point_in(&mut seed, r.elem.lo, r.elem.hi))
                    .collect();
                let ideal = eval_analog_sampled(&r, &xs);
                let physical = eval_analog_sampled_quantized(&r, &schedule, &target, &xs);
                let dev = (physical - ideal).abs();
                worst = worst.max(dev);
                assert!(
                    dev <= budget.eps_math,
                    "`{label}` on `{}`: |physical − ideal| = {dev:e} exceeds ε = {:e} at len {len}",
                    target.name,
                    budget.eps_math
                );
            }
            assert!(
                worst > 0.0,
                "`{label}` on `{}`: battery observed no deviation — vacuous",
                target.name
            );
        }
    }
}

// ============================================================
//  5. Contracts are load-bearing
// ============================================================

#[test]
fn missing_contracts_refuse_unbounded_input() {
    let target = AnalogTarget::ideal();

    let no_len = "## To total (xs: Seq of Float) -> Float:\n    Let mutable acc be 0.0.\n    Repeat for each x in xs:\n        Assume x is between -1.0 and 1.0.\n        Set acc to acc + x.\n    Return acc.\n\n## Main\n    Let mutable xs be a new Seq of Float.\n    Push 0.5 to xs.\n    Show total(xs).\n";
    match analog_reduction_from_source(no_len, "total", &target) {
        Err(AnaLowerError::UnboundedInput(msg)) => {
            assert!(
                msg.contains("length"),
                "must name the missing length contract: {msg}"
            )
        }
        other => panic!("missing length range must refuse, got {other:?}"),
    }

    let no_elem = "## To total (xs: Seq of Float) -> Float:\n    Assume length of xs is between 1 and 64.\n    Let mutable acc be 0.0.\n    Repeat for each x in xs:\n        Set acc to acc + x.\n    Return acc.\n\n## Main\n    Let mutable xs be a new Seq of Float.\n    Push 0.5 to xs.\n    Show total(xs).\n";
    match analog_reduction_from_source(no_elem, "total", &target) {
        Err(AnaLowerError::UnboundedInput(msg)) => {
            assert!(msg.contains('x'), "must name the undeclared element: {msg}")
        }
        other => panic!("missing element range must refuse, got {other:?}"),
    }
}

/// 64 frames of `x²` bound the accumulator to 64; the V1 chip's smallest realizable reference
/// (125 mV) cannot scale that into the rails — the physics says no, and the compiler says it
/// TYPED. (The ideal preset's deeper reference ladder CAN schedule it — which is exactly the
/// difference between the abstract certification vehicle and the deliberately-boring chip.)
#[test]
fn energy_over_64_frames_outgrows_the_v1_chips_references() {
    let target = AnalogTarget::sky130_fpaa();
    let hot = ENERGY.replace("between 1 and 32", "between 1 and 64");
    let r = analog_reduction_from_source(&hot, "energy", &target).expect("lowers");
    match reduction_scale_pass(&r, &target) {
        Err(AnaLowerError::Unplaceable(AnaUnplaceable::MulReferenceUnrealizable { .. })) => {}
        other => panic!("a 64-frame energy stream must outgrow the V1 Vref options, got {other:?}"),
    }
}

// ============================================================
//  6. The crossbar canonicalization
// ============================================================

#[test]
fn dot_product_folds_to_one_summing_column_and_wide_columns_tree() {
    let target = AnalogTarget::ideal();
    let dot3 = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";
    let netlist = analog_netlist_from_source(dot3, "dot3", &target).expect("lowers");
    let folded = fold_crossbar_for(&netlist, target.sum_fanin);
    assert_eq!(
        folded.cells.len(),
        1,
        "an unrolled dot product IS one crossbar column: {:?}",
        folded.cells
    );
    match &folded.cells[0] {
        AnalogCell::Sum { inputs, weights } => {
            assert_eq!(inputs.len(), 3);
            assert_eq!(
                weights,
                &vec![0.25, 0.5, 0.125],
                "the programmed conductances ARE the weight vector"
            );
        }
        other => panic!("expected the crossbar column, got {other:?}"),
    }

    // A 12-term column trees down to the physical fan-in.
    let wide_terms: Vec<String> = (0..12).map(|i| format!("x{i} * 0.0625")).collect();
    let params: Vec<String> = (0..12).map(|i| format!("x{i}: Float")).collect();
    let assumes: Vec<String> = (0..12)
        .map(|i| format!("    Assume x{i} is between -1.0 and 1.0."))
        .collect();
    let wide = format!(
        "## To wide ({}) -> Float:\n{}\n    Return {}.\n\n## Main\n    Show wide({}).\n",
        params.join(", "),
        assumes.join("\n"),
        wide_terms.join(" + "),
        vec!["0.5"; 12].join(", ")
    );
    let mut roomy = AnalogTarget::ideal();
    roomy.n_dac = 16;
    let netlist = analog_netlist_from_source(&wide, "wide", &roomy).expect("wide lowers");
    let folded = fold_crossbar_for(&netlist, roomy.sum_fanin);
    assert!(
        folded.cells.len() > 1,
        "12 terms cannot fit one fan-in-8 node"
    );
    for cell in &folded.cells {
        if let AnalogCell::Sum { inputs, .. } = cell {
            assert!(
                inputs.len() <= roomy.sum_fanin,
                "every physical node respects the fan-in: {:?}",
                folded.cells
            );
        }
    }
}
