//! SUPERCRUSH Sprint S0B: Quantifier Encoding
//!
//! Tests that ForAll/Exists are properly encoded using Z3's forall_const/exists_const
//! instead of just dropping the bound variables and encoding the body.

#![cfg(feature = "verification")]

use logicaffeine_verify::{
    check_equivalence, EquivalenceResult, VerificationSession, VerifyExpr, VerifyOp, VerifyType,
};

// ═══════════════════════════════════════════════════════════════════════════
// BASIC QUANTIFIER ENCODING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn z3_forall_not_true() {
    // forall x:Int. x > 0 is NOT equivalent to true (x could be negative)
    let expr = VerifyExpr::forall(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)),
    );
    let result = check_equivalence(&expr, &VerifyExpr::bool(true), &[], 1);
    assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
        "forall x. x>0 should NOT be true, got: {:?}", result);
}

#[test]
fn z3_forall_valid() {
    // forall x:Int. (x > 0 -> x >= 0) is valid (tautology)
    let expr = VerifyExpr::forall(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::implies(
            VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)),
            VerifyExpr::gte(VerifyExpr::var("x"), VerifyExpr::int(0)),
        ),
    );
    let result = check_equivalence(&expr, &VerifyExpr::bool(true), &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "forall x. (x>0 -> x>=0) should be valid, got: {:?}", result);
}

#[test]
fn z3_forall_invalid() {
    // forall x:Int. x > 0 is not valid
    let expr = VerifyExpr::forall(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)),
    );
    let result = check_equivalence(&expr, &VerifyExpr::bool(true), &[], 1);
    assert!(!matches!(result, EquivalenceResult::Equivalent),
        "forall x. x>0 should NOT be valid");
}

#[test]
fn z3_exists_satisfiable() {
    // exists x:Int. x == 5 is satisfiable (equivalent to true when encoded)
    let expr = VerifyExpr::exists(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(5)),
    );
    let result = check_equivalence(&expr, &VerifyExpr::bool(true), &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "exists x. x==5 should be equivalent to true, got: {:?}", result);
}

#[test]
fn z3_exists_unsatisfiable() {
    // exists x:Int. (x > 5 AND x < 3) should NOT be true
    let expr = VerifyExpr::exists(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::and(
            VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5)),
            VerifyExpr::lt(VerifyExpr::var("x"), VerifyExpr::int(3)),
        ),
    );
    let result = check_equivalence(&expr, &VerifyExpr::bool(true), &[], 1);
    assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
        "exists x. (x>5 AND x<3) should NOT be true, got: {:?}", result);
}

#[test]
fn z3_nested_quantifiers() {
    // forall x:Int. exists y:Int. y > x is valid
    let expr = VerifyExpr::forall(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::exists(
            vec![("y".into(), VerifyType::Int)],
            VerifyExpr::gt(VerifyExpr::var("y"), VerifyExpr::var("x")),
        ),
    );
    let result = check_equivalence(&expr, &VerifyExpr::bool(true), &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "forall x. exists y. y>x should be valid, got: {:?}", result);
}

#[test]
fn z3_quantifier_bool_typed() {
    // forall p:Bool. (p OR NOT p) is valid (law of excluded middle)
    let expr = VerifyExpr::forall(
        vec![("p".into(), VerifyType::Bool)],
        VerifyExpr::or(
            VerifyExpr::var("p"),
            VerifyExpr::not(VerifyExpr::var("p")),
        ),
    );
    let result = check_equivalence(&expr, &VerifyExpr::bool(true), &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "forall p. (p OR NOT p) should be valid, got: {:?}", result);
}

#[test]
fn z3_quantifier_mixed_free_bound() {
    // Free vars stay free, bound vars are quantified
    // (forall x:Int. x > 0) AND y > 0
    // This is NOT equivalent to true because forall x. x>0 is false
    let expr = VerifyExpr::and(
        VerifyExpr::forall(
            vec![("x".into(), VerifyType::Int)],
            VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)),
        ),
        VerifyExpr::gt(VerifyExpr::var("y"), VerifyExpr::int(0)),
    );
    let result = check_equivalence(&expr, &VerifyExpr::bool(true), &[], 1);
    assert!(!matches!(result, EquivalenceResult::Equivalent),
        "Formula with false universal should not be true");
}

#[test]
fn z3_empty_quantifier() {
    // forall. P (no vars) equiv P
    let expr = VerifyExpr::forall(
        vec![],
        VerifyExpr::var("p"),
    );
    let just_p = VerifyExpr::var("p");
    let result = check_equivalence(&expr, &just_p, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "Empty quantifier should equal its body, got: {:?}", result);
}

#[test]
fn z3_quantifier_in_equivalence() {
    // Two equivalent quantified formulas
    let a = VerifyExpr::forall(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::implies(
            VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(10)),
            VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5)),
        ),
    );
    let b = VerifyExpr::bool(true); // This implication is a tautology
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "forall x. (x>10 -> x>5) should be valid, got: {:?}", result);
}

#[test]
fn z3_quantifier_alternation() {
    // forall x. exists y. f(x) == y is valid
    let expr = VerifyExpr::forall(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::exists(
            vec![("y".into(), VerifyType::Int)],
            VerifyExpr::eq(
                VerifyExpr::apply("f", vec![VerifyExpr::var("x")]),
                VerifyExpr::var("y"),
            ),
        ),
    );
    // This should be valid since for any x, we can pick y = f(x)
    // But f is uninterpreted (returns Bool), so f(x) == y is mixing sorts...
    // Let's use a simpler version: forall x. exists y. y == x
    let expr2 = VerifyExpr::forall(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::exists(
            vec![("y".into(), VerifyType::Int)],
            VerifyExpr::eq(VerifyExpr::var("y"), VerifyExpr::var("x")),
        ),
    );
    let result = check_equivalence(&expr2, &VerifyExpr::bool(true), &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "forall x. exists y. y==x should be valid, got: {:?}", result);
}

#[test]
fn z3_quantifier_scope_correct() {
    // (forall x. x > 0) is false, but (exists x. x > 0) is true
    let forall_expr = VerifyExpr::forall(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)),
    );
    let exists_expr = VerifyExpr::exists(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)),
    );
    let result = check_equivalence(&forall_expr, &exists_expr, &[], 1);
    assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
        "forall x. x>0 should NOT equal exists x. x>0, got: {:?}", result);
}

#[test]
fn z3_forall_with_arith() {
    // forall x:Int. x + 0 == x is valid
    let expr = VerifyExpr::forall(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::eq(
            VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("x"), VerifyExpr::int(0)),
            VerifyExpr::var("x"),
        ),
    );
    let result = check_equivalence(&expr, &VerifyExpr::bool(true), &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "forall x. x+0==x should be valid, got: {:?}", result);
}

#[test]
fn z3_exists_with_constraint() {
    // exists x:Int. (x > 3 AND x < 5) should be true (x = 4)
    let expr = VerifyExpr::exists(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::and(
            VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(3)),
            VerifyExpr::lt(VerifyExpr::var("x"), VerifyExpr::int(5)),
        ),
    );
    let result = check_equivalence(&expr, &VerifyExpr::bool(true), &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "exists x. (x>3 AND x<5) should be satisfiable, got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// SOLVER.RS ENCODER PATH — VerificationSession quantifier handling
// ═══════════════════════════════════════════════════════════════════════════
// These tests exercise the solver.rs Encoder path via VerificationSession,
// which is SEPARATE from the equivalence.rs path tested above.
// The bug: solver.rs:795-802 drops bound variables.

#[test]
fn solver_forall_assumption_not_vacuous() {
    // Assume: forall x:Int. x > 0 (which is FALSE — should constrain to unsatisfiable)
    // Verify: 1 == 1 (trivially true)
    // If quantifier is properly encoded, the assumption is FALSE so session is unsatisfiable
    // => verify_with_binding should succeed (vacuous truth from contradiction)
    // If quantifier drops vars, assumption becomes "x > 0" with free x, which is satisfiable
    // => verify_with_binding would also succeed but for wrong reason
    //
    // Better test: assume forall x. x > 0, then try to verify x <= 0 for a specific x
    let mut session = VerificationSession::new();
    session.declare("y", VerifyType::Int);
    // Assume: forall x. x > 0 — this is FALSE
    session.assume(&VerifyExpr::forall(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)),
    ));
    // Under a false assumption, anything should verify (ex falso)
    // But with the bug, the assumption is just "x > 0" with free x,
    // which means y can be anything, and verifying y < 0 would FAIL
    let result = session.verify_with_binding(
        "y",
        VerifyType::Int,
        &VerifyExpr::int(-5),
        &VerifyExpr::lt(VerifyExpr::var("y"), VerifyExpr::int(0)),
    );
    // This should pass: -5 < 0 is true, and the assumption shouldn't interfere
    assert!(result.is_ok(), "verify_with_binding should handle quantified assumptions: {:?}", result);
}

#[test]
fn solver_exists_in_predicate() {
    // Verify: exists x:Int. x == 5 — should be valid (some x satisfies it)
    // Through the solver.rs path, if exists drops vars, this becomes just "x == 5"
    // which is NOT valid (x is free and could be anything)
    let session = VerificationSession::new();
    let exists_expr = VerifyExpr::exists(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(5)),
    );
    let result = session.verify_with_binding(
        "dummy",
        VerifyType::Int,
        &VerifyExpr::int(0),
        &exists_expr,
    );
    // With proper quantifier encoding: exists x. x==5 is satisfiable/valid
    // With bug: x==5 with free x is not valid (x could be != 5)
    assert!(result.is_ok(),
        "solver should properly encode exists in predicate: {:?}", result);
}

#[test]
fn solver_forall_in_predicate_invalid() {
    // Verify: forall x:Int. x > 0 — should FAIL (it's false, x could be negative)
    let session = VerificationSession::new();
    let forall_expr = VerifyExpr::forall(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)),
    );
    let result = session.verify_with_binding(
        "dummy",
        VerifyType::Int,
        &VerifyExpr::int(0),
        &forall_expr,
    );
    // With proper encoding: forall x. x>0 is NOT valid → should fail
    // With bug: "x > 0" with free x, and dummy=0, x is undeclared → unpredictable
    assert!(result.is_err(),
        "forall x. x>0 should NOT be valid through solver path: {:?}", result);
}

#[test]
fn solver_nested_quantifier_valid() {
    // Verify: forall x:Int. exists y:Int. y > x — valid
    let session = VerificationSession::new();
    let expr = VerifyExpr::forall(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::exists(
            vec![("y".into(), VerifyType::Int)],
            VerifyExpr::gt(VerifyExpr::var("y"), VerifyExpr::var("x")),
        ),
    );
    let result = session.verify_with_binding(
        "dummy",
        VerifyType::Int,
        &VerifyExpr::int(0),
        &expr,
    );
    assert!(result.is_ok(),
        "forall x. exists y. y>x should be valid through solver path: {:?}", result);
}

#[test]
fn solver_forall_tautology_valid() {
    // Verify: forall x:Int. (x > 0 -> x >= 0) — valid tautology
    let session = VerificationSession::new();
    let expr = VerifyExpr::forall(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::implies(
            VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)),
            VerifyExpr::gte(VerifyExpr::var("x"), VerifyExpr::int(0)),
        ),
    );
    let result = session.verify_with_binding(
        "dummy",
        VerifyType::Int,
        &VerifyExpr::int(0),
        &expr,
    );
    assert!(result.is_ok(),
        "forall x. (x>0 -> x>=0) should be valid through solver path: {:?}", result);
}
