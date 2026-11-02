//! ★ ANALOG DC LOCK ★ — memoryless float dataflow lowers to analog tiles and the ideal Tier-1
//! evaluator reproduces the VM exactly (up to f64 formatting round-trip).
//!
//! The lowering chain under lock: a float FUNCTION (the analog region unit — its params are the
//! DAC inputs, its `Assume` statements the declared ranges) → the digital fabric's certified
//! if-conversion front-end (`decision_fabric_from_source`, imported read-only) → `codegen_ana`
//! tile netlist (Kirchhoff `Sum` with signed weights, `Mul` for variable×variable, `GainConst`
//! for constant gains) → `eval_analog_dc` on the `ideal()` target.
//!
//! The IDEAL tier is the analog analogue of `eval_grid ≡ VM`: no non-ideality, unit scale, so the
//! DAG computes the same f64 operations in the same order as the VM — the comparison is
//! `logos_approx_eq` (and in practice bit-exact). Tolerance-widening starts only where physics
//! enters (the error-budget phases), never here.
//!
//! Fail-closed refusals under lock: an undeclared float param (`UnboundedInput` naming it), a
//! division (`UnloweredOp` — no translinear divider witnessed), an integer-only function
//! (`NoAnalogRegion`), and target gating (`MissingCapability` / `TooSmall` on a starved target).
//! NOTE: `AlgebraicLoop` cannot fire on this path — a decision fabric is acyclic SSA by
//! construction; the refusal arrives with the phases that introduce feedback (integrators).
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.

use logicaffeine_compile::codegen_ana::eval_analog_dc;
use logicaffeine_compile::codegen_ana::{
    analog_netlist_from_source, ASrc, AnaLowerError, AnalogCell, AnalogTarget,
};
use logicaffeine_compile::compile::vm_outcome;

// ============================================================
//  Corpus — function-shaped analog regions
// ============================================================

const AFFINE: &str = "## To f (x: Float) -> Float:\n    Assume x is between -1.0 and 1.0.\n    Return x * 0.5 + 0.25.\n\n## Main\n    Show f(0.5).\n";

const DOT3: &str = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";

const HORNER: &str = "## To poly (x: Float) -> Float:\n    Assume x is between -1.0 and 1.0.\n    Return ((x * 0.125 + 0.25) * x + 0.5) * x + 0.0625.\n\n## Main\n    Show poly(0.5).\n";

/// variable × variable — the four-quadrant `Mul` tile (not a constant gain).
const VARMUL: &str = "## To vm2 (a: Float, b: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Return a * b + 0.25.\n\n## Main\n    Show vm2(0.5, 0.5).\n";

/// Float subtraction — a signed-weight Kirchhoff sum (differential negation is free).
const SUB: &str = "## To d (a: Float, b: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Return a - b.\n\n## Main\n    Show d(0.75, 0.5).\n";

const CORPUS: &[(&str, &str, &[f64])] = &[
    ("affine", AFFINE, &[0.5]),
    ("dot3", DOT3, &[0.5, 0.25, 0.75]),
    ("horner", HORNER, &[0.5]),
    ("varmul", VARMUL, &[0.5, 0.5]),
    ("sub", SUB, &[0.75, 0.5]),
];

/// The corpus functions' names, in corpus order.
const FN_NAMES: &[&str] = &["f", "dot3", "poly", "vm2", "d"];

// ============================================================
//  Helpers
// ============================================================

/// The VM's answer for the corpus program's single `Show` — parsed back to f64 (LOGOS float
/// formatting is shortest-round-trip, so the parse is exact).
fn vm_float(src: &str) -> f64 {
    let out = vm_outcome(src);
    assert_eq!(out.error, None, "corpus program must run on the VM:\n{src}");
    out.output
        .trim()
        .parse::<f64>()
        .unwrap_or_else(|_| panic!("VM output is not a float: {:?}", out.output))
}

fn approx(a: f64, b: f64) -> bool {
    logicaffeine_data::ops::logos_approx_eq(a, b)
}

// ============================================================
//  1. The corpus lowers and the ideal evaluator ≡ VM
// ============================================================

#[test]
fn corpus_lowers_and_ideal_eval_matches_the_vm() {
    let target = AnalogTarget::ideal();
    for (i, (label, src, inputs)) in CORPUS.iter().enumerate() {
        let netlist = analog_netlist_from_source(src, FN_NAMES[i], &target)
            .unwrap_or_else(|e| panic!("`{label}` must lower: {e}"));
        assert_eq!(netlist.n_in, inputs.len(), "`{label}` input arity");
        assert!(
            !netlist.cells.is_empty(),
            "`{label}` must produce analog tiles"
        );

        let got = eval_analog_dc(&netlist, inputs);
        let want = vm_float(src);
        assert!(
            approx(got, want),
            "`{label}` ideal analog eval must match the VM: analog={got} vm={want}"
        );
    }
}

/// The tile vocabulary is honest: a constant gain is a `GainConst` (programmed conductance), a
/// variable×variable product is a four-quadrant `Mul`, and add/sub are weighted `Sum` tiles.
#[test]
fn tile_selection_matches_the_analog_vocabulary() {
    let target = AnalogTarget::ideal();

    let affine = analog_netlist_from_source(AFFINE, "f", &target).expect("affine lowers");
    assert!(
        affine
            .cells
            .iter()
            .any(|c| matches!(c, AnalogCell::GainConst { .. })),
        "x * 0.5 must be a programmed constant gain, got {:?}",
        affine.cells
    );
    assert!(
        !affine
            .cells
            .iter()
            .any(|c| matches!(c, AnalogCell::Mul { .. })),
        "no variable×variable product exists in the affine program"
    );

    let varmul = analog_netlist_from_source(VARMUL, "vm2", &target).expect("varmul lowers");
    assert!(
        varmul
            .cells
            .iter()
            .any(|c| matches!(c, AnalogCell::Mul { .. })),
        "a * b must be the four-quadrant Mul tile, got {:?}",
        varmul.cells
    );

    let sub = analog_netlist_from_source(SUB, "d", &target).expect("sub lowers");
    let has_negative_weight = sub.cells.iter().any(|c| match c {
        AnalogCell::Sum { weights, .. } => weights.iter().any(|w| *w < 0.0),
        _ => false,
    });
    assert!(
        has_negative_weight,
        "a - b must be a signed-weight Kirchhoff sum, got {:?}",
        sub.cells
    );
}

/// Deep-graph sanity: the evaluator follows the SSA order, so Horner nests exactly like the VM.
#[test]
fn ideal_eval_is_bit_exact_on_the_corpus() {
    let target = AnalogTarget::ideal();
    for (i, (label, src, inputs)) in CORPUS.iter().enumerate() {
        let netlist = analog_netlist_from_source(src, FN_NAMES[i], &target).expect("lowers");
        let got = eval_analog_dc(&netlist, inputs);
        let want = vm_float(src);
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "`{label}`: the IDEAL tier mirrors the VM's f64 ops in SSA order — bit-exact, \
             not merely approximate (tolerance begins where physics enters, not here)"
        );
    }
}

// ============================================================
//  2. Fail-closed refusals
// ============================================================

#[test]
fn undeclared_float_param_is_refused_by_name() {
    let src = "## To f (x: Float, y: Float) -> Float:\n    Assume x is between -1.0 and 1.0.\n    Return x * y.\n\n## Main\n    Show f(0.5, 0.5).\n";
    match analog_netlist_from_source(src, "f", &AnalogTarget::ideal()) {
        Err(AnaLowerError::UnboundedInput(msg)) => {
            assert!(
                msg.contains('y'),
                "the refusal must NAME the undeclared param: {msg}"
            );
        }
        other => panic!("undeclared float param must refuse UnboundedInput, got {other:?}"),
    }
}

#[test]
fn division_lowers_through_the_translinear_ratio_tile() {
    // The O6 promotion (the plan-scheduled grow-only event): a translinear divider IS now
    // witnessed, so the old refusal became a lowering. The divisor's certified box must
    // still clear zero — that starvation refusal lives in `analog_translinear_ratio_lock`.
    let src = "## To f (x: Float) -> Float:\n    Assume x is between 0.5 and 1.0.\n    Return 1.0 / x.\n\n## Main\n    Show f(0.5).\n";
    let netlist = analog_netlist_from_source(src, "f", &AnalogTarget::ideal())
        .expect("float division lowers through the Gilbert loop");
    assert!(
        netlist
            .cells
            .iter()
            .any(|c| matches!(c, logicaffeine_compile::codegen_ana::AnalogCell::Div { .. })),
        "vacuity guard: the ratio tile is really there"
    );
}

#[test]
fn integer_only_function_is_refused() {
    let src = "## To f (x: Int) -> Int:\n    Return x * 2.\n\n## Main\n    Show f(3).\n";
    assert_eq!(
        analog_netlist_from_source(src, "f", &AnalogTarget::ideal()),
        Err(AnaLowerError::NoAnalogRegion),
        "an integer-only function has nothing to map onto the analog plane"
    );
}

#[test]
fn target_gating_refuses_starved_targets() {
    // A target with no Mul tile refuses the variable×variable program by capability.
    let mut no_mul = AnalogTarget::ideal();
    no_mul
        .cells
        .remove(&logicaffeine_compile::codegen_ana::AnalogCellKind::Mul);
    match analog_netlist_from_source(VARMUL, "vm2", &no_mul) {
        Err(AnaLowerError::Unplaceable(u)) => {
            let msg = format!("{u}");
            assert!(
                msg.contains("Mul"),
                "the refusal must name the missing tile: {msg}"
            );
        }
        other => panic!("a Mul-less target must refuse the varmul program, got {other:?}"),
    }

    // A 1×1 array refuses a multi-tile netlist for size.
    let mut tiny = AnalogTarget::ideal();
    tiny.rows = 1;
    tiny.cols = 1;
    match analog_netlist_from_source(DOT3, "dot3", &tiny) {
        Err(AnaLowerError::Unplaceable(u)) => {
            let msg = format!("{u}");
            assert!(
                msg.contains("too small"),
                "the refusal must be the size gate: {msg}"
            );
        }
        other => panic!("a 1x1 target must refuse the dot3 netlist, got {other:?}"),
    }

    // A DAC-starved target refuses a 3-input region by channel count.
    let mut starved = AnalogTarget::ideal();
    starved.n_dac = 2;
    match analog_netlist_from_source(DOT3, "dot3", &starved) {
        Err(AnaLowerError::Unplaceable(u)) => {
            let msg = format!("{u}");
            assert!(
                msg.contains("DacChannels"),
                "the refusal must name the channel capability: {msg}"
            );
        }
        other => panic!("a 2-DAC target must refuse a 3-input region, got {other:?}"),
    }
}

// ============================================================
//  3. Output taps are well-formed
// ============================================================

#[test]
fn netlist_output_references_a_real_value() {
    let target = AnalogTarget::ideal();
    for (i, (label, src, _)) in CORPUS.iter().enumerate() {
        let n = analog_netlist_from_source(src, FN_NAMES[i], &target).expect("lowers");
        match n.out {
            ASrc::Val(v) => assert!(
                v < n.cells.len(),
                "`{label}` output taps cell {v} of {}",
                n.cells.len()
            ),
            ASrc::In(k) => assert!(k < n.n_in, "`{label}` output taps input {k}"),
            ASrc::Const(_) => {} // a fully-folded region is legal, if pointless
        }
    }
}
