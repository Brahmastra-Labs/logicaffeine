//! ★ ANALOG RANGE-CERTIFICATE LOCK ★ — the gain-scaling pass emits a range-safety certificate
//! that the INDEPENDENT checker (`logicaffeine_proof::interval_replay`, dependency-free, in the
//! proof crate — the RUP-checker trust separation) accepts, and every physical gate refuses
//! typed.
//!
//! What this file locks:
//!   1. For every corpus region: `scale_pass` succeeds on `ideal()`, the produced claim passes
//!      `check_range_claim` — and the claim is over the QUANTIZED netlist (the configuration
//!      silicon actually runs), judged by code the producer cannot influence.
//!   2. Tampered claims FAIL the independent checker: a bound tightened past reality, a rail
//!      wall pulled in, a noise floor the signals cannot clear.
//!   3. The schedule is physical: the block scale is a power of two inside the target's
//!      `scale_log2_range`; the descale is its exact inverse; a `Mul`-bearing graph's scale IS
//!      a member of `mul_vref_options_v` (rail safety does not imply Vref realizability — the
//!      review's catch); weights in the quantized twin sit on the unit-element grid within
//!      half an LSB of the ideal weights.
//!   4. Refusals: no realizable Vref → `MulReferenceUnrealizable`; ranges too big for the
//!      scale window → `DynamicRangeExceeded`; a snapped weight outside the realizable range →
//!      `GainRatioUnrealizable`.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.

use logicaffeine_compile::codegen_ana::{
    analog_netlist_from_source, assumed_ranges, scale_pass, to_range_claim, AnaLowerError,
    AnaUnplaceable, AnalogCell, AnalogTarget,
};
use logicaffeine_proof::interval_replay::check_range_claim;

const AFFINE: &str = "## To f (x: Float) -> Float:\n    Assume x is between -1.0 and 1.0.\n    Return x * 0.5 + 0.25.\n\n## Main\n    Show f(0.5).\n";

const DOT3: &str = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";

const VARMUL: &str = "## To vm2 (a: Float, b: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Return a * b + 0.25.\n\n## Main\n    Show vm2(0.5, 0.5).\n";

/// A NON-dyadic constant gain — exercises real weight quantization (0.3 is not on the grid).
const NONDYADIC: &str = "## To g (x: Float) -> Float:\n    Assume x is between -1.0 and 1.0.\n    Return x * 0.3 + 0.25.\n\n## Main\n    Show g(0.5).\n";

const CORPUS: &[(&str, &str)] = &[
    ("affine", AFFINE),
    ("dot3", DOT3),
    ("varmul", VARMUL),
    ("nondyadic", NONDYADIC),
];
const FN_NAMES: &[&str] = &["f", "dot3", "vm2", "g"];

fn scheduled(
    src: &str,
    fn_name: &str,
    target: &AnalogTarget,
) -> (
    logicaffeine_compile::codegen_ana::AnalogNetlist,
    logicaffeine_compile::codegen_ana::GainSchedule,
    logicaffeine_compile::codegen_ana::RangeCertificate,
) {
    let netlist = analog_netlist_from_source(src, fn_name, target).expect("lowers");
    let ranges = assumed_ranges(src);
    assert_eq!(
        ranges.len(),
        netlist.n_in,
        "one declared range per input, in param order"
    );
    let (schedule, cert) = scale_pass(&netlist, &ranges, target).expect("schedules");
    (netlist, schedule, cert)
}

// ============================================================
//  1. Certificates pass the INDEPENDENT checker
// ============================================================

#[test]
fn corpus_certificates_pass_the_independent_checker() {
    let target = AnalogTarget::ideal();
    for (i, (label, src)) in CORPUS.iter().enumerate() {
        let (_netlist, schedule, cert) = scheduled(src, FN_NAMES[i], &target);
        let (cells, claim) = to_range_claim(&schedule, &cert);
        assert!(
            check_range_claim(&cells, &claim),
            "`{label}`: the independent replay checker must accept the compiler's claim"
        );
        assert_eq!(
            cells.len(),
            schedule.quantized.cells.len(),
            "the claim is over the QUANTIZED netlist"
        );
    }
}

// ============================================================
//  2. Tampered claims are REJECTED by the independent checker
// ============================================================

#[test]
fn tampered_claims_are_rejected() {
    let target = AnalogTarget::ideal();
    let (_n, schedule, cert) = scheduled(AFFINE, "f", &target);
    let (cells, claim) = to_range_claim(&schedule, &cert);
    assert!(check_range_claim(&cells, &claim), "baseline must pass");

    // A node bound tightened past reality.
    let mut tighter = claim.clone();
    let last = tighter.node_hi_v.len() - 1;
    tighter.node_hi_v[last] = tighter.node_hi_v[last] / 2.0;
    assert!(
        !check_range_claim(&cells, &tighter),
        "a claim tighter than reality must be rejected"
    );

    // The rail wall pulled inside a claimed bound.
    let mut walled = claim.clone();
    walled.rail_half_span_v = claim.node_hi_v.iter().cloned().fold(0.0_f64, f64::max) / 2.0;
    assert!(
        !check_range_claim(&cells, &walled),
        "a claim past the rail must be rejected"
    );

    // A noise floor the signals cannot clear.
    let mut starved = claim.clone();
    starved.noise_floor_v = 0.1;
    starved.min_full_scale_over_noise = 1.0e6;
    assert!(
        !check_range_claim(&cells, &starved),
        "a starved dynamic range must be rejected"
    );
}

// ============================================================
//  3. The schedule is physical
// ============================================================

#[test]
fn schedule_is_physical_and_exactly_invertible() {
    let target = AnalogTarget::ideal();
    for (i, (label, src)) in CORPUS.iter().enumerate() {
        let (_n, schedule, _c) = scheduled(src, FN_NAMES[i], &target);
        let (lo, hi) = target.scale_log2_range;
        assert!(
            schedule.scale_log2 >= lo && schedule.scale_log2 <= hi,
            "`{label}`: scale {} outside the target window",
            schedule.scale_log2
        );
        assert_eq!(
            schedule.out_descale_log2, -schedule.scale_log2,
            "`{label}`: descale is the exact inverse"
        );
        let s = f64::powi(2.0, schedule.scale_log2);
        let d = f64::powi(2.0, schedule.out_descale_log2);
        assert_eq!(s * d, 1.0, "`{label}`: powers of two invert EXACTLY in f64");
    }
}

#[test]
fn mul_bearing_scale_is_a_realizable_reference() {
    let target = AnalogTarget::ideal();
    let (_n, schedule, _c) = scheduled(VARMUL, "vm2", &target);
    assert!(
        target
            .mul_vref_options_v
            .contains(&logicaffeine_base::si::Volts(schedule.mul_vref_v)),
        "the multiplier reference {} must be chosen FROM the realizable set {:?}",
        schedule.mul_vref_v,
        target.mul_vref_options_v
    );
    assert_eq!(
        schedule.mul_vref_v,
        f64::powi(2.0, schedule.scale_log2),
        "the reference IS the block scale (translinear normalization)"
    );
}

#[test]
fn weights_are_quantized_to_the_unit_element_grid() {
    let target = AnalogTarget::sky130_fpaa();
    let (netlist, schedule, _c) = scheduled(NONDYADIC, "g", &target);
    let lsb = target.weight_lsb();
    let mut saw_snap = false;
    for (orig, q) in netlist.cells.iter().zip(&schedule.quantized.cells) {
        if let (AnalogCell::GainConst { k, .. }, AnalogCell::GainConst { k: kq, .. }) = (orig, q) {
            assert!(
                (kq / lsb).fract() == 0.0,
                "quantized weight {kq} must sit on the {lsb} grid"
            );
            assert!(
                (k - kq).abs() <= lsb / 2.0 + f64::EPSILON,
                "snap must be nearest: |{k} - {kq}| vs lsb {lsb}"
            );
            if k != kq {
                saw_snap = true;
            }
        }
    }
    assert!(
        saw_snap,
        "0.3 is not on the 1/64 grid — a real snap must occur"
    );
}

// ============================================================
//  4. Physical refusals
// ============================================================

#[test]
fn unrealizable_mul_reference_is_refused() {
    let mut target = AnalogTarget::ideal();
    target.mul_vref_options_v = vec![logicaffeine_base::si::Volts(0.3)]; // not a power of two — no valid block scale exists
    let netlist = analog_netlist_from_source(VARMUL, "vm2", &target).expect("lowers");
    let ranges = assumed_ranges(VARMUL);
    match scale_pass(&netlist, &ranges, &target) {
        Err(AnaLowerError::Unplaceable(AnaUnplaceable::MulReferenceUnrealizable {
            options_v,
            ..
        })) => {
            assert_eq!(options_v, vec![0.3]);
        }
        other => panic!("rail safety must not imply Vref realizability, got {other:?}"),
    }
}

#[test]
fn oversized_ranges_are_refused_when_the_scale_window_is_pinned() {
    let mut target = AnalogTarget::ideal();
    target.scale_log2_range = (0, 0); // no attenuation available
    let big = "## To f (x: Float) -> Float:\n    Assume x is between -100.0 and 100.0.\n    Return x * 0.5 + 0.25.\n\n## Main\n    Show f(0.5).\n";
    let netlist = analog_netlist_from_source(big, "f", &target).expect("lowers");
    let ranges = assumed_ranges(big);
    match scale_pass(&netlist, &ranges, &target) {
        Err(AnaLowerError::Unplaceable(AnaUnplaceable::DynamicRangeExceeded { .. })) => {}
        other => panic!("±100 at unit scale must exceed the rails, got {other:?}"),
    }
}

#[test]
fn out_of_range_snapped_weight_is_refused() {
    let target = AnalogTarget::sky130_fpaa(); // weight max = 127/64 < 3.0
    let hot = "## To f (x: Float) -> Float:\n    Assume x is between -1.0 and 1.0.\n    Return x * 3.0 + 0.25.\n\n## Main\n    Show f(0.5).\n";
    let netlist = analog_netlist_from_source(hot, "f", &target).expect("lowers");
    let ranges = assumed_ranges(hot);
    match scale_pass(&netlist, &ranges, &target) {
        Err(AnaLowerError::Unplaceable(AnaUnplaceable::GainRatioUnrealizable {
            needed, ..
        })) => {
            assert_eq!(needed, 3.0, "the refusal names the unrealizable weight");
        }
        other => panic!("a 3.0 weight exceeds the sky130-fpaa unit-element range, got {other:?}"),
    }
}
