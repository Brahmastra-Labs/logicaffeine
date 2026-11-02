//! ★ ANALOG ERROR-BUDGET LOCK ★ — ε is a COMPILER OUTPUT, and it is SOUND.
//!
//! The budget's whole claim: `|physical-tier eval − ideal eval| ≤ eps_math`, where the physical
//! tier is the converter path over the QUANTIZED netlist (DAC rounding in, volt-domain settle,
//! ADC rounding out) and ε composes itemized, named terms (DAC/ADC half-LSBs, the weight-snap
//! deviation, the Mul interaction cross-term, an explicit f64 guard) in directed-outward f64.
//!
//! What this file locks:
//!   1. `recheck()` — the total is EXACTLY the outward sum of the itemized terms, and dropping
//!      ANY term is detected (the `PlacementCertificate` tamper discipline).
//!   2. THE BEHAVIORAL CORE — over a deterministic fixed-seed battery of in-range points,
//!      the quantized-converter evaluation NEVER deviates from the ideal evaluation by more
//!      than the compiler's own ε. No hand-typed tolerance appears anywhere in this file.
//!   3. Physics direction — fewer converter bits ⇒ strictly larger ε; a non-dyadic weight
//!      program carries a positive `WeightQuant` term, a dyadic one carries none.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.

use logicaffeine_compile::codegen_ana::{
    analog_netlist_from_source, assumed_ranges, error_budget, eval_analog_dc,
    eval_analog_quantized, scale_pass, AnalogTarget, ErrorSource,
};

const AFFINE: &str = "## To f (x: Float) -> Float:\n    Assume x is between -1.0 and 1.0.\n    Return x * 0.5 + 0.25.\n\n## Main\n    Show f(0.5).\n";

const DOT3: &str = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";

const VARMUL: &str = "## To vm2 (a: Float, b: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Return a * b + 0.25.\n\n## Main\n    Show vm2(0.5, 0.5).\n";

const NONDYADIC: &str = "## To g (x: Float) -> Float:\n    Assume x is between -1.0 and 1.0.\n    Return x * 0.3 + 0.25.\n\n## Main\n    Show g(0.5).\n";

const CORPUS: &[(&str, &str)] = &[
    ("affine", AFFINE),
    ("dot3", DOT3),
    ("varmul", VARMUL),
    ("nondyadic", NONDYADIC),
];
const FN_NAMES: &[&str] = &["f", "dot3", "vm2", "g"];

/// Fixed forever — the deterministic point battery's seed.
const BASE_SEED: u64 = 0xA11A_00C0_FFEE_5EED;
const POINTS_PER_PROGRAM: usize = 256;

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A deterministic point inside `[lo, hi]`.
fn point_in(state: &mut u64, lo: f64, hi: f64) -> f64 {
    let u = (splitmix(state) >> 11) as f64 / (1u64 << 53) as f64;
    lo + u * (hi - lo)
}

struct Built {
    netlist: logicaffeine_compile::codegen_ana::AnalogNetlist,
    schedule: logicaffeine_compile::codegen_ana::GainSchedule,
    budget: logicaffeine_compile::codegen_ana::OutputBudget,
    ranges: Vec<logicaffeine_compile::codegen_ana::AssumedRange>,
}

fn build(src: &str, fn_name: &str, target: &AnalogTarget) -> Built {
    let netlist = analog_netlist_from_source(src, fn_name, target).expect("lowers");
    let ranges = assumed_ranges(src);
    let (schedule, cert) = scale_pass(&netlist, &ranges, target).expect("schedules");
    let budget = error_budget(&netlist, &cert, &schedule, target);
    Built {
        netlist,
        schedule,
        budget,
        ranges,
    }
}

// ============================================================
//  1. recheck + tamper
// ============================================================

#[test]
fn budgets_recheck_and_every_dropped_term_is_detected() {
    let target = AnalogTarget::ideal();
    for (i, (label, src)) in CORPUS.iter().enumerate() {
        let b = build(src, FN_NAMES[i], &target);
        assert!(
            b.budget.recheck(),
            "`{label}`: the budget must replay from its own terms"
        );
        assert!(!b.budget.terms.is_empty());
        for idx in 0..b.budget.terms.len() {
            assert!(
                !b.budget.with_term_removed(idx).recheck(),
                "`{label}`: dropping term {idx} ({:?}) must be detected",
                b.budget.terms[idx].source
            );
        }
    }
}

// ============================================================
//  2. THE BEHAVIORAL CORE — the physical tier stays inside ε
// ============================================================

#[test]
fn quantized_evaluation_never_exceeds_the_compilers_own_epsilon() {
    for target in [AnalogTarget::ideal(), AnalogTarget::sky130_fpaa()] {
        for (i, (label, src)) in CORPUS.iter().enumerate() {
            let b = build(src, FN_NAMES[i], &target);
            let mut seed = BASE_SEED ^ (i as u64) ^ ((target.dac_bits as u64) << 32);
            let mut worst = 0.0_f64;
            for _ in 0..POINTS_PER_PROGRAM {
                let xs: Vec<f64> = b
                    .ranges
                    .iter()
                    .map(|r| point_in(&mut seed, r.lo, r.hi))
                    .collect();
                let ideal = eval_analog_dc(&b.netlist, &xs);
                let physical = eval_analog_quantized(&b.schedule, &target, &xs);
                let dev = (physical - ideal).abs();
                worst = worst.max(dev);
                assert!(
                    dev <= b.budget.eps_math,
                    "`{label}` on `{}`: |physical − ideal| = {dev:e} exceeds the compiler's ε = {:e} at {xs:?}",
                    target.name,
                    b.budget.eps_math
                );
            }
            // The battery must be exercising something real, not comparing zeros.
            assert!(
                worst > 0.0,
                "`{label}` on `{}`: the battery observed no deviation at all — vacuous",
                target.name
            );
        }
    }
}

// ============================================================
//  3. Physics direction
// ============================================================

#[test]
fn fewer_converter_bits_cost_strictly_more_epsilon() {
    let fine = AnalogTarget::ideal();
    let mut coarse = AnalogTarget::ideal();
    coarse.dac_bits = 6;
    coarse.adc_bits = 6;
    let f = build(AFFINE, "f", &fine);
    let c = build(AFFINE, "f", &coarse);
    assert!(
        c.budget.eps_math > f.budget.eps_math,
        "6-bit converters must cost more ε than 12-bit: {:e} vs {:e}",
        c.budget.eps_math,
        f.budget.eps_math
    );
}

#[test]
fn weight_quantization_terms_appear_exactly_when_weights_snap() {
    let target = AnalogTarget::sky130_fpaa();
    let dyadic = build(AFFINE, "f", &target);
    assert!(
        !dyadic
            .budget
            .terms
            .iter()
            .any(|t| matches!(t.source, ErrorSource::WeightQuant)),
        "0.5/0.25 sit on the unit-element grid — no snap, no term"
    );
    let snapped = build(NONDYADIC, "g", &target);
    let wq = snapped
        .budget
        .terms
        .iter()
        .find(|t| matches!(t.source, ErrorSource::WeightQuant))
        .expect("0.3 must snap and be itemized");
    assert!(wq.eps_math > 0.0);
}
