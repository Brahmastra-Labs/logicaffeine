//! **`largo prove` certifies ARITHMETIC ring identities written in a `.lg` development.**
//!
//! The end of the "theorem-prove them, not only run them" thread: a `## Theory` block states a ring
//! identity in formal `Eq(Mul(a, b), Mul(b, a))` syntax, and the whole pipeline — formal parser →
//! `logic_expr_to_proof_expr` (which lowers an arithmetic `Eq` to a ring `Identity`) → backward chainer
//! (`try_arithmetic`) → the proof-producing oracle → kernel `infer_type` — certifies it. Every ✓ is
//! kernel-checked; a derivation alone never counts.

use logicaffeine_compile::ui_bridge::compile_theory_for_ui;

#[test]
fn largo_prove_certifies_ring_identities_from_a_development() {
    let program = "\
## Theory Ring

Theorem commutativity: prove for all a b, Eq(Mul(a, b), Mul(b, a)).
Theorem distributivity: prove for all a b c, Eq(Mul(a, Add(b, c)), Add(Mul(a, b), Mul(a, c))).
Theorem additive_commutativity: prove for all a b, Eq(Add(a, b), Add(b, a)).
";
    let result = compile_theory_for_ui(program);
    assert!(
        result.parse_error.is_none(),
        "development did not parse: {:?}",
        result.parse_error
    );
    assert_eq!(
        result.theorems.len(),
        3,
        "expected three theorems, got {:?}",
        result.theorems.len()
    );
    for t in &result.theorems {
        assert!(
            t.verified,
            "theorem `{}` was not kernel-certified: {:?}",
            t.name, t.error
        );
    }
}

#[test]
fn largo_prove_rejects_a_false_ring_identity() {
    // a·b = a + b is false for general a, b — the engine must not fabricate a proof (fail-closed).
    let program = "\
## Theory Ring

Theorem bogus: prove for all a b, Eq(Mul(a, b), Add(a, b)).
";
    let result = compile_theory_for_ui(program);
    assert!(
        result.parse_error.is_none(),
        "development did not parse: {:?}",
        result.parse_error
    );
    assert!(
        result.theorems.iter().all(|t| !t.verified),
        "a false ring identity must NOT verify: {:?}",
        result.theorems
    );
}
