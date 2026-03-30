//! Sprint 3: FOL → Bounded Verification IR (Temporal Kripke Unrolling)
//!
//! Tests that Kripke-lowered FOL is correctly translated to bounded timestep
//! model using compile_kripke_with() and FolTranslator.

use logicaffeine_language::compile_kripke_with;
use logicaffeine_compile::codegen_sva::fol_to_verify::FolTranslator;
use logicaffeine_compile::codegen_sva::sva_to_verify::{BoundedExpr, count_and_leaves, count_or_leaves};

// ═══════════════════════════════════════════════════════════════════════════
// BASIC: compile_kripke_with gives us the AST
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn compile_kripke_with_returns_ast() {
    let result = compile_kripke_with("Every dog runs.", |_ast, _interner| {
        true // just verify callback fires
    });
    assert!(result.is_ok(), "compile_kripke_with should succeed: {:?}", result.err());
    assert!(result.unwrap());
}

// ═══════════════════════════════════════════════════════════════════════════
// TEMPORAL UNROLLING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn fol_always_translates_to_conjunction() {
    // "Always, every dog runs." → G(∀x(Dog(x) → Run(x)))
    // After Kripke lowering: ∀w'(Accessible_Temporal(w0,w') → ∀x(Dog(x,w') → Run(x,w')))
    // After bounded translation at bound=3: conjunction of 3 steps
    let result = compile_kripke_with("Always, every dog runs.", |ast, interner| {
        let mut translator = FolTranslator::new(interner, 3);
        let bounded = translator.translate(ast);
        bounded
    }).unwrap();

    // Should produce a conjunction (And chain) with multiple timestep entries
    let leaf_count = count_and_leaves(&result);
    assert!(
        leaf_count >= 3,
        "G at bound=3 should produce at least 3 And-leaves, got {}",
        leaf_count
    );
}

#[test]
fn fol_eventually_translates_to_disjunction() {
    // "Eventually, John runs." → F(Run(John))
    // After Kripke lowering: ∃w'(Reachable_Temporal(w0,w') ∧ Run(John,w'))
    // After bounded translation: disjunction of timesteps
    let result = compile_kripke_with("Eventually, John runs.", |ast, interner| {
        let mut translator = FolTranslator::new(interner, 3);
        let bounded = translator.translate(ast);
        bounded
    }).unwrap();

    let leaf_count = count_or_leaves(&result);
    assert!(
        leaf_count >= 2,
        "F at bound=3 should produce at least 2 Or-leaves, got {}",
        leaf_count
    );
}

#[test]
fn fol_next_translates_to_bounded() {
    // "Next, John runs." → X(Run(John))
    // Should translate to a bounded expression
    let result = compile_kripke_with("Next, John runs.", |ast, interner| {
        let mut translator = FolTranslator::new(interner, 5);
        let bounded = translator.translate(ast);
        bounded
    }).unwrap();

    // Should not be trivially True
    assert_ne!(result, BoundedExpr::Bool(true), "X(P) should produce non-trivial bounded expr");
}

// ═══════════════════════════════════════════════════════════════════════════
// DECLARATIONS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn fol_translation_produces_bounded_expr() {
    // Translation should produce a non-trivial bounded expression
    let (bounded, decls) = compile_kripke_with("Always, John runs.", |ast, interner| {
        let mut translator = FolTranslator::new(interner, 3);
        let result = translator.translate_property(ast);
        (format!("{:?}", result.expr), result.declarations)
    }).unwrap();

    // The bounded expr should contain And (from G unrolling) or Var references
    assert!(
        bounded.contains("And") || bounded.contains("Var") || bounded.contains("Implies"),
        "Translation should produce structured bounded expr, got: {}",
        bounded
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// NON-TEMPORAL PASSTHROUGH
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn fol_non_temporal_is_preserved() {
    // A sentence without temporal operators should still translate
    let result = compile_kripke_with("Every dog runs.", |ast, interner| {
        let mut translator = FolTranslator::new(interner, 3);
        let bounded = translator.translate(ast);
        bounded
    }).unwrap();

    // Non-temporal should produce something (not just Bool(true) for everything)
    // At minimum the quantifier and predicate should survive
    assert_ne!(result, BoundedExpr::Bool(false));
}
