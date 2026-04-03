//! Sprint 6C: Hierarchical Spec Decomposition

#![cfg(feature = "verification")]

use logicaffeine_compile::codegen_sva::decompose::decompose_conjunctive;
use logicaffeine_verify::ir::{VerifyExpr, VerifyOp};

#[test]
fn conjunction_splits() {
    let expr = VerifyExpr::and(VerifyExpr::var("P"), VerifyExpr::var("Q"));
    let parts = decompose_conjunctive(&expr);
    assert_eq!(parts.len(), 2);
}

#[test]
fn nested_conjunction_flattens() {
    let expr = VerifyExpr::and(
        VerifyExpr::and(VerifyExpr::var("P"), VerifyExpr::var("Q")),
        VerifyExpr::var("R"),
    );
    let parts = decompose_conjunctive(&expr);
    assert_eq!(parts.len(), 3);
}

#[test]
fn single_returns_self() {
    let expr = VerifyExpr::var("P");
    let parts = decompose_conjunctive(&expr);
    assert_eq!(parts.len(), 1);
}

#[test]
fn implication_not_split() {
    let expr = VerifyExpr::implies(VerifyExpr::var("P"), VerifyExpr::var("Q"));
    let parts = decompose_conjunctive(&expr);
    assert_eq!(parts.len(), 1, "Implication should not be split");
}

#[test]
fn deep_nesting_flattens() {
    let mut expr = VerifyExpr::var("A");
    for name in ["B", "C", "D", "E"] {
        expr = VerifyExpr::and(expr, VerifyExpr::var(name));
    }
    let parts = decompose_conjunctive(&expr);
    assert_eq!(parts.len(), 5, "5-level nesting should flatten to 5 parts");
}

#[test]
fn preserves_signal_refs() {
    let expr = VerifyExpr::and(
        VerifyExpr::implies(VerifyExpr::var("req"), VerifyExpr::var("ack")),
        VerifyExpr::not(VerifyExpr::and(VerifyExpr::var("ga"), VerifyExpr::var("gb"))),
    );
    let parts = decompose_conjunctive(&expr);
    assert_eq!(parts.len(), 2);
}

#[test]
fn or_not_split() {
    let expr = VerifyExpr::or(VerifyExpr::var("P"), VerifyExpr::var("Q"));
    let parts = decompose_conjunctive(&expr);
    assert_eq!(parts.len(), 1, "Disjunction should not be split");
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT 6C BACKFILL: Z3 Soundness Verification
// ═══════════════════════════════════════════════════════════════════════════

use logicaffeine_compile::codegen_sva::decompose::verify_decomposition_sound;

#[test]
fn z3_conjunction_sound() {
    let p = VerifyExpr::var("P@0");
    let q = VerifyExpr::var("Q@0");
    let original = VerifyExpr::and(p, q);
    let parts = decompose_conjunctive(&original);
    assert!(verify_decomposition_sound(&original, &parts, 1),
        "Decomposing And(P, Q) into [P, Q] must be provably sound");
}

#[test]
fn z3_disjunction_unsound() {
    let p = VerifyExpr::var("P@0");
    let q = VerifyExpr::var("Q@0");
    let original = VerifyExpr::or(p.clone(), q.clone());
    let bogus_parts = vec![p, q];
    assert!(!verify_decomposition_sound(&original, &bogus_parts, 1),
        "Splitting Or(P, Q) into And(P, Q) must NOT be sound");
}

#[test]
fn z3_deep_conjunction_sound() {
    let mut expr = VerifyExpr::var("A@0");
    for name in ["B@0", "C@0", "D@0"] {
        expr = VerifyExpr::and(expr, VerifyExpr::var(name));
    }
    let parts = decompose_conjunctive(&expr);
    assert_eq!(parts.len(), 4);
    assert!(verify_decomposition_sound(&expr, &parts, 1),
        "4-level deep conjunction decomposition must be provably sound");
}
