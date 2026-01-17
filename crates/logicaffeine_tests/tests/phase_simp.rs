//! Phase Simp: Simplifier Tactic
//!
//! Tests for the simp tactic which normalizes goals using
//! rewrite rules from the context.

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// TYPE CHECKS
// =============================================================================

#[test]
fn test_dsimpsolve_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DSimpSolve.");
    assert!(result.is_ok(), "DSimpSolve should exist: {:?}", result);
}

#[test]
fn test_try_simp_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_simp.");
    assert!(result.is_ok(), "try_simp should exist: {:?}", result);
}

// =============================================================================
// REFLEXIVE EQUALITIES (simp handles these via refl)
// =============================================================================

#[test]
fn test_simp_reflexivity() {
    let mut repl = Repl::new();
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute(
        "Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) x) x.",
    )
    .unwrap();
    repl.execute("Definition d : Derivation := try_simp goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "simp should prove x = x");
}

// =============================================================================
// CONSTANT FOLDING (arithmetic simplification)
// =============================================================================

#[test]
fn test_simp_constant_fold() {
    let mut repl = Repl::new();
    // Goal: 2 + 3 = 5
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"add\") (SLit 2)) (SLit 3).")
        .unwrap();
    repl.execute("Definition rhs : Syntax := SLit 5.").unwrap();
    repl.execute(
        "Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) lhs) rhs.",
    )
    .unwrap();
    repl.execute("Definition d : Derivation := try_simp goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "simp should prove 2 + 3 = 5");
}

#[test]
fn test_simp_nested_arithmetic() {
    let mut repl = Repl::new();
    // Goal: (1 + 1) * 3 = 6
    repl.execute(
        "Definition one_plus_one : Syntax := SApp (SApp (SName \"add\") (SLit 1)) (SLit 1).",
    )
    .unwrap();
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"mul\") one_plus_one) (SLit 3).")
        .unwrap();
    repl.execute("Definition rhs : Syntax := SLit 6.").unwrap();
    repl.execute(
        "Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) lhs) rhs.",
    )
    .unwrap();
    repl.execute("Definition d : Derivation := try_simp goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "simp should prove (1+1)*3 = 6");
}

#[test]
fn test_simp_subtraction() {
    let mut repl = Repl::new();
    // Goal: 10 - 3 = 7
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"sub\") (SLit 10)) (SLit 3).")
        .unwrap();
    repl.execute("Definition rhs : Syntax := SLit 7.").unwrap();
    repl.execute(
        "Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) lhs) rhs.",
    )
    .unwrap();
    repl.execute("Definition d : Derivation := try_simp goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "simp should prove 10 - 3 = 7");
}

// =============================================================================
// CONDITIONAL REWRITING (implications)
// =============================================================================

#[test]
fn test_simp_with_hypothesis() {
    let mut repl = Repl::new();
    // Given: x = 0
    // Goal: x + 1 = 1
    // After substituting x → 0, we get 0 + 1 = 1, which simplifies
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition zero : Syntax := SLit 0.").unwrap();
    repl.execute("Definition one : Syntax := SLit 1.").unwrap();
    repl.execute("Definition x_plus_1 : Syntax := SApp (SApp (SName \"add\") x) one.")
        .unwrap();
    repl.execute(
        "Definition hyp : Syntax := SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) x) zero.",
    )
    .unwrap();
    repl.execute(
        "Definition concl : Syntax := SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) x_plus_1) one.",
    )
    .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"implies\") hyp) concl.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_simp goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "simp should prove x=0 -> x+1=1");
}

#[test]
fn test_simp_chained_hypotheses() {
    let mut repl = Repl::new();
    // Given: x = 1, y = 2
    // Goal: x + y = 3
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition y : Syntax := SVar 1.").unwrap();
    repl.execute("Definition x_plus_y : Syntax := SApp (SApp (SName \"add\") x) y.")
        .unwrap();
    repl.execute(
        "Definition h1 : Syntax := SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) x) (SLit 1).",
    )
    .unwrap();
    repl.execute(
        "Definition h2 : Syntax := SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) y) (SLit 2).",
    )
    .unwrap();
    repl.execute(
        "Definition concl : Syntax := SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) x_plus_y) (SLit 3).",
    )
    .unwrap();
    // h1 -> (h2 -> concl)
    repl.execute("Definition inner : Syntax := SApp (SApp (SName \"implies\") h2) concl.")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"implies\") h1) inner.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_simp goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(
        concluded, original,
        "simp should prove x=1 -> y=2 -> x+y=3"
    );
}

// =============================================================================
// FAILURE CASES
// =============================================================================

#[test]
fn test_simp_fails_on_false_equality() {
    let mut repl = Repl::new();
    // Goal: 2 = 3 (cannot be proved)
    repl.execute(
        "Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) (SLit 2)) (SLit 3).",
    )
    .unwrap();
    repl.execute("Definition d : Derivation := try_simp goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let result = repl.execute("Eval result.").unwrap();
    assert!(result.contains("Error"), "simp should fail on 2 = 3");
}

#[test]
fn test_simp_fails_without_hypothesis() {
    let mut repl = Repl::new();
    // Goal: x + 1 = 1 (without x=0 hypothesis, this is not provable)
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition one : Syntax := SLit 1.").unwrap();
    repl.execute("Definition x_plus_1 : Syntax := SApp (SApp (SName \"add\") x) one.")
        .unwrap();
    repl.execute(
        "Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) x_plus_1) one.",
    )
    .unwrap();
    repl.execute("Definition d : Derivation := try_simp goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let result = repl.execute("Eval result.").unwrap();
    assert!(
        result.contains("Error"),
        "simp should fail on x+1=1 without hypothesis"
    );
}
