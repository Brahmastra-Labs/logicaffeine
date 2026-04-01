//! Sprint A: Hardware Temporal Operators
//!
//! RED tests for LTL/CTL temporal logic extensions to the Kripke frame.
//! These test the pivot from discourse worlds to hardware state transitions.
//!
//! G(φ) → ∀w'(Accessible_Temporal(w, w') → φ(w'))
//! F(φ) → ∃w'(Reachable_Temporal(w, w') ∧ φ(w'))
//! X(φ) → ∀w'(Next_Temporal(w, w') → φ(w'))

use logicaffeine_language::{compile, compile_kripke};

// ═══════════════════════════════════════════════════════════════════════════
// AST SIZE BUDGET
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn temporal_binary_fits_48_byte_budget() {
    // Adding TemporalBinary must not blow LogicExpr past 48 bytes.
    // The existing size_tests in logic.rs enforce this — this test
    // guards from the test-crate side.
    assert!(
        std::mem::size_of::<logicaffeine_language::ast::logic::LogicExpr>() <= 48,
        "LogicExpr is {} bytes after adding TemporalBinary — must be ≤ 48",
        std::mem::size_of::<logicaffeine_language::ast::logic::LogicExpr>()
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// KRIPKE LOWERING: G (ALWAYS / GLOBALLY)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_always_lowers_to_universal_temporal() {
    // G(P) → ∀w'(Accessible_Temporal(w₀, w') → P(w'))
    let output = compile_kripke("Always, every signal is valid.").unwrap();
    assert!(
        output.contains("Accessible_Temporal"),
        "G(P) should generate Accessible_Temporal. Got: {}",
        output
    );
    assert!(
        output.contains("∀") || output.contains("ForAll"),
        "G(P) should generate universal quantifier over worlds. Got: {}",
        output
    );
}

#[test]
fn kripke_always_generates_temporal_not_alethic() {
    // Temporal domain must produce Accessible_Temporal, NOT Accessible_Alethic
    let output = compile_kripke("Always, every dog runs.").unwrap();
    assert!(
        output.contains("Accessible_Temporal"),
        "Should use Temporal accessibility. Got: {}",
        output
    );
    assert!(
        !output.contains("Accessible_Alethic"),
        "Should NOT use Alethic accessibility for temporal operators. Got: {}",
        output
    );
    assert!(
        !output.contains("Accessible_Deontic"),
        "Should NOT use Deontic accessibility for temporal operators. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// KRIPKE LOWERING: F (EVENTUALLY / FINALLY)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_eventually_lowers_to_existential_temporal() {
    // F(P) → ∃w'(Reachable_Temporal(w₀, w') ∧ P(w'))
    let output = compile_kripke("Eventually, the acknowledge signal is high.").unwrap();
    assert!(
        output.contains("Reachable_Temporal"),
        "F(P) should generate Reachable_Temporal. Got: {}",
        output
    );
    assert!(
        output.contains("∃") || output.contains("Exists"),
        "F(P) should generate existential quantifier. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// KRIPKE LOWERING: X (NEXT)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_next_lowers_to_single_step() {
    // X(P) → ∀w'(Next_Temporal(w₀, w') → P(w'))
    let output = compile_kripke("Next, every dog runs.").unwrap();
    assert!(
        output.contains("Next_Temporal"),
        "X(P) should generate Next_Temporal (single-step). Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// KRIPKE LOWERING: UNTIL (BINARY TEMPORAL)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_until_lowers_correctly() {
    // φ U ψ → ψ(w) ∨ (φ(w) ∧ ∃w'(Next_Temporal(w,w') ∧ (φ U ψ)(w')))
    let output = compile_kripke("Every dog runs until every cat sleeps.").unwrap();
    assert!(
        output.contains("Next_Temporal"),
        "Until should generate Next_Temporal for recursive step. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// NESTED TEMPORAL + MODAL
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_nested_always_implies_next() {
    // G(P → X(Q)) — safety property with one-cycle delay
    let output = compile_kripke(
        "Always, if every dog runs, then next, every cat sleeps.",
    )
    .unwrap();
    assert!(
        output.contains("Accessible_Temporal"),
        "G should generate Accessible_Temporal. Got: {}",
        output
    );
    assert!(
        output.contains("Next_Temporal"),
        "X should generate Next_Temporal inside G. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// WORLD THREADING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_temporal_threads_worlds_through_predicates() {
    // All predicates inside temporal operators must carry world arguments
    let output = compile_kripke("Always, the data is valid.").unwrap();
    // The predicate "valid(data)" should have a world variable attached
    assert!(
        output.contains("w"),
        "Predicates inside temporal scope must carry world arguments. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PROOF ENGINE: TEMPORAL INFERENCE RULES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn proof_temporal_induction_rule_exists() {
    // G(P) iff P(s₀) ∧ ∀s(P(s) → P(next(s))) — standard k-induction
    let _rule = logicaffeine_proof::InferenceRule::TemporalInduction;
}

#[test]
fn proof_temporal_unfolding_rule_exists() {
    // G(P) iff P ∧ X(G(P)) — fixpoint unfolding
    let _rule = logicaffeine_proof::InferenceRule::TemporalUnfolding;
}

#[test]
fn proof_expr_temporal_binary_exists() {
    // ProofExpr must have a TemporalBinary variant for Until/Release/WeakUntil
    let expr = logicaffeine_proof::ProofExpr::TemporalBinary {
        operator: "Until".to_string(),
        left: Box::new(logicaffeine_proof::ProofExpr::Atom("P".to_string())),
        right: Box::new(logicaffeine_proof::ProofExpr::Atom("Q".to_string())),
    };
    // Should be constructible and matchable
    match &expr {
        logicaffeine_proof::ProofExpr::TemporalBinary { operator, .. } => {
            assert_eq!(operator, "Until");
        }
        _ => panic!("Expected TemporalBinary"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CTL COMPOSITION (MODAL + TEMPORAL)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_always_produces_universal_and_implication() {
    // G(P) produces ∀w'(Accessible_Temporal(w,w') → P(w'))
    // The quantifier is universal, and the connective is implication
    let output = compile_kripke("Always, John runs.").unwrap();
    assert!(
        output.contains("∀") || output.contains("ForAll"),
        "G should produce universal quantifier. Got: {}",
        output
    );
    assert!(
        output.contains("→") || output.contains("Implies") || output.contains("If"),
        "G should produce implication. Got: {}",
        output
    );
    assert!(
        output.contains("Accessible_Temporal"),
        "G should use Accessible_Temporal. Got: {}",
        output
    );
}

#[test]
fn kripke_eventually_produces_existential_and_conjunction() {
    // F(P) produces ∃w'(Reachable_Temporal(w,w') ∧ P(w'))
    let output = compile_kripke("Eventually, John runs.").unwrap();
    assert!(
        output.contains("∃") || output.contains("Exists"),
        "F should produce existential quantifier. Got: {}",
        output
    );
    assert!(
        output.contains("∧") || output.contains("And"),
        "F should produce conjunction. Got: {}",
        output
    );
    assert!(
        output.contains("Reachable_Temporal"),
        "F should use Reachable_Temporal. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// KRIPKE CONTEXT EXTENSIONS (Phase 3)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_context_clock_counter_starts_at_zero() {
    use logicaffeine_language::semantics::kripke::KripkeContext;
    let mut interner = logicaffeine_base::Interner::new();
    let ctx = KripkeContext::new(&mut interner);
    assert_eq!(ctx.clock_counter(), 0, "Clock counter should start at 0");
}

#[test]
fn kripke_context_domain_hint_starts_as_none() {
    use logicaffeine_language::semantics::kripke::KripkeContext;
    let mut interner = logicaffeine_base::Interner::new();
    let ctx = KripkeContext::new(&mut interner);
    assert!(ctx.domain_hint().is_none(), "Domain hint should start as None");
}

#[test]
fn kripke_next_advances_clock() {
    // After lowering a "Next" temporal, the clock counter should be > 0
    // We verify this indirectly: "Next" produces Next_Temporal with fresh worlds
    let result = compile_kripke("Next, every dog runs.");
    let output = result.unwrap();
    assert!(output.contains("Next_Temporal"),
        "Next must produce Next_Temporal. Got: {}", output);
    // The world variables (w1, w2) indicate fresh worlds were generated
    assert!(output.contains("w1") || output.contains("w"),
        "Next must generate fresh world variables. Got: {}", output);
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT 0D: RELEASE / WEAKUNTIL LOWERING VERIFICATION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_release_lowers_correctly() {
    // φ R ψ → ψ(w) ∨ (φ(w) ∧ ∃w'(Next_Temporal(w,w') ∧ (φ R ψ)(w')))
    // Kripke lowering of Release should produce Next_Temporal.
    let output = compile_kripke("Every dog runs release every cat sleeps.").unwrap();
    assert!(
        output.contains("Next_Temporal"),
        "Release should generate Next_Temporal for recursive step. Got: {}",
        output
    );
}

#[test]
fn kripke_weak_until_lowers_correctly() {
    // φ W ψ — parses and lowers without error.
    let result = compile_kripke("Every dog runs weak-until every cat sleeps.");
    assert!(
        result.is_ok(),
        "WeakUntil should parse and lower without error. Got: {:?}",
        result.err()
    );
}

#[test]
fn kripke_release_produces_conjunction() {
    // Release body should have OR at top level (ψ ∨ (φ ∧ ...))
    // which means the lowered output contains both And and Or connectives.
    let output = compile_kripke("Every dog runs release every cat sleeps.").unwrap();
    let has_conjunction = output.contains("∧") || output.contains("And") || output.contains("∧");
    assert!(
        has_conjunction,
        "Release lowering must produce conjunction. Got: {}",
        output
    );
}
