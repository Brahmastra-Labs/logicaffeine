//! Sprint 4: Z3 Equivalence Engine
//!
//! Tests for the hardware verification pipeline: structural equivalence,
//! bounded translation, and the full pipeline API.
//! Z3-dependent tests are behind #[cfg(feature = "verification")].

use logicaffeine_compile::codegen_sva::hw_pipeline::{
    check_structural_equivalence, check_bounded_equivalence,
    translate_sva_to_bounded, translate_spec_to_bounded,
    compile_hw_spec, emit_hw_sva, EquivalenceResult,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
use logicaffeine_compile::codegen_sva::SvaAssertionKind;

// ═══════════════════════════════════════════════════════════════════════════
// STRUCTURAL EQUIVALENCE (no Z3)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn structural_equiv_identical_mutex() {
    let result = check_structural_equivalence(
        "!(grant_a && grant_b)",
        "!(grant_a && grant_b)",
    )
    .unwrap();
    assert!(result, "Identical mutex SVAs should be structurally equivalent");
}

#[test]
fn structural_equiv_different_signals_fail() {
    let result = check_structural_equivalence(
        "req |-> ack",
        "req |-> done",
    )
    .unwrap();
    assert!(!result, "Different signals should not be equivalent");
}

#[test]
fn structural_equiv_different_operators_fail() {
    let result = check_structural_equivalence(
        "req |-> ack",
        "req |=> ack",
    )
    .unwrap();
    assert!(!result, "Different implication types should not be equivalent");
}

// ═══════════════════════════════════════════════════════════════════════════
// BOUNDED TRANSLATION (no Z3)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn translate_sva_to_bounded_mutex() {
    let result = translate_sva_to_bounded("!(grant_a && grant_b)", 5).unwrap();
    // Should produce a conjunction of 5 timesteps, each with !(ga && gb)
    let leaf_count = logicaffeine_compile::codegen_sva::sva_to_verify::count_and_leaves(&result.expr);
    assert_eq!(leaf_count, 5, "Mutex at bound=5 should have 5 And-leaves");
}

#[test]
fn translate_spec_to_bounded_always() {
    let result = translate_spec_to_bounded("Always, every dog runs.", 3).unwrap();
    let leaf_count = logicaffeine_compile::codegen_sva::sva_to_verify::count_and_leaves(&result.expr);
    assert!(leaf_count >= 3, "G at bound=3 should have >= 3 And-leaves, got {}", leaf_count);
}

// ═══════════════════════════════════════════════════════════════════════════
// BOUNDED EQUIVALENCE (structural, no Z3)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn bounded_equiv_identical_exprs() {
    let a = BoundedExpr::And(
        Box::new(BoundedExpr::Var("req@0".into())),
        Box::new(BoundedExpr::Var("req@1".into())),
    );
    let b = BoundedExpr::And(
        Box::new(BoundedExpr::Var("req@0".into())),
        Box::new(BoundedExpr::Var("req@1".into())),
    );
    let result = check_bounded_equivalence(&a, &b, 2);
    assert!(result.equivalent, "Identical bounded exprs should be equivalent");
}

#[test]
fn bounded_equiv_different_exprs() {
    let a = BoundedExpr::Var("req@0".into());
    let b = BoundedExpr::Var("ack@0".into());
    let result = check_bounded_equivalence(&a, &b, 1);
    assert!(!result.equivalent, "Different vars should not be equivalent");
}

// ═══════════════════════════════════════════════════════════════════════════
// COMPILE + EMIT PUBLIC API
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn compile_hw_spec_returns_fol() {
    let fol = compile_hw_spec("Every dog runs.").unwrap();
    assert!(fol.contains("Run") || fol.contains("run"), "FOL should contain Run predicate");
}

#[test]
fn emit_hw_sva_generates_property() {
    let sva = emit_hw_sva("Mutex", "clk", "!(grant_a && grant_b)", SvaAssertionKind::Assert);
    assert!(sva.contains("property p_mutex"), "Should contain property name");
    assert!(sva.contains("@(posedge clk)"), "Should contain clock edge");
    assert!(sva.contains("assert property"), "Should have assert wrapper");
}

// ═══════════════════════════════════════════════════════════════════════════
// END-TO-END: SPEC → BOUNDED → SVA → BOUNDED → COMPARE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_spec_and_sva_translate_to_bounded() {
    // Both sides should translate without error
    let spec_bounded = translate_spec_to_bounded("Always, John runs.", 3);
    assert!(spec_bounded.is_ok(), "Spec should translate: {:?}", spec_bounded.err());

    let sva_bounded = translate_sva_to_bounded("req |-> ack", 3);
    assert!(sva_bounded.is_ok(), "SVA should translate: {:?}", sva_bounded.err());
}

#[test]
fn e2e_sva_parse_error_propagates() {
    let result = translate_sva_to_bounded("|||invalid|||", 5);
    assert!(result.is_err(), "Invalid SVA should error");
}
