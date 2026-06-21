//! EXODIA Phase 2 gate — the FORGE's Z3 layer (D11 a + b, M15).
//!
//! Specs over 64-bit bitvectors for the JIT's integer micro-ops, proved
//! satisfiable and algebraically lawful, then GROUNDED: Z3-chosen witness
//! inputs (plus the adversarial corner battery) run through the actual
//! copy-and-patch machine code, the forge's reference interpreter, and
//! the spec itself — three independent evaluators that must agree. A
//! deliberate-bug canary proves the harness can fail.
//!
//! Z3-backed: behind the `verification` feature like every solver test.

#![cfg(all(not(target_arch = "wasm32"), feature = "verification"))]

use logicaffeine_synth::spec::{
    all_specs, prove_all_satisfiable, prove_commutative, prove_min_div_wraps,
};
use logicaffeine_synth::witness::check_spec_with_witnesses;

/// Every spec is inhabited: some (a, b, r) satisfies pre ∧ post.
#[test]
fn all_specs_satisfiable() {
    let n = prove_all_satisfiable().unwrap_or_else(|e| panic!("{e}"));
    assert!(n >= 13, "expected the full integer spec family, got {n}");
}

/// The algebra the Architect's kernel-certified rules lean on, re-proved
/// at the BITVECTOR level: add/mul/and/or/xor/eq commute.
#[test]
fn commutativity_proofs() {
    for name in ["add", "mul", "and", "or", "xor", "eq"] {
        prove_commutative(name).unwrap_or_else(|e| panic!("{e}"));
    }
}

/// Subtraction must NOT prove commutative — the prover is not a rubber
/// stamp.
#[test]
fn sub_is_not_commutative() {
    assert!(prove_commutative("sub").is_err(), "sub commuting would be a prover bug");
}

/// The locked wrapping rim: ⌊MIN / −1⌋ wraps to MIN in the spec model,
/// exactly as `wrapping_div` does at runtime.
#[test]
fn min_div_minus_one_wraps_in_the_model() {
    prove_min_div_wraps().unwrap_or_else(|e| panic!("{e}"));
}

/// THE GROUNDING GATE: for every spec, Z3 witnesses + the corner battery
/// agree across machine code, reference interpreter, and spec — including
/// the side-exit agreement for checked ops at excluded inputs.
#[test]
fn witness_three_way_agreement() {
    for spec in all_specs() {
        let report = check_spec_with_witnesses(&spec, 8)
            .unwrap_or_else(|e| panic!("witness harness: {e}"));
        assert!(
            report.inputs_checked >= 100,
            "spec '{}' checked only {} inputs",
            report.spec,
            report.inputs_checked
        );
    }
}

/// The canary: a deliberately WRONG spec (add claiming subtraction) must
/// be caught by the harness — proof the three-way comparison can fail.
#[test]
fn harness_catches_a_deliberate_bug() {
    let wrong = logicaffeine_synth::spec::deliberately_wrong_spec_for_canary();
    assert!(
        check_spec_with_witnesses(&wrong, 4).is_err(),
        "the harness accepted a spec that contradicts the machine code"
    );
}
