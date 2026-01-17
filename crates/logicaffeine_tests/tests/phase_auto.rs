//! Phase Auto: The Meta-Tactic
//!
//! Tests for the auto tactic which combines all decision procedures.

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// TYPE CHECKS
// =============================================================================

#[test]
fn test_dautosolve_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DAutoSolve.");
    assert!(result.is_ok(), "DAutoSolve should exist: {:?}", result);
}

#[test]
fn test_try_auto_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_auto.");
    assert!(result.is_ok(), "try_auto should exist: {:?}", result);
}

// =============================================================================
// SIMP-SOLVABLE (auto delegates to simp)
// =============================================================================

#[test]
fn test_auto_true() {
    let mut repl = Repl::new();
    repl.execute("Definition goal : Syntax := SName \"True\".")
        .unwrap();
    repl.execute("Definition d : Derivation := try_auto goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();
    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "auto should prove True via simp");
}

// =============================================================================
// RING-SOLVABLE (auto delegates to ring)
// =============================================================================

#[test]
fn test_auto_ring_comm() {
    let mut repl = Repl::new();
    // xx + yy = yy + xx
    repl.execute("Definition xx : Syntax := SVar 0.").unwrap();
    repl.execute("Definition yy : Syntax := SVar 1.").unwrap();
    repl.execute("Definition left_side : Syntax := SApp (SApp (SName \"add\") xx) yy.")
        .unwrap();
    repl.execute("Definition right_side : Syntax := SApp (SApp (SName \"add\") yy) xx.")
        .unwrap();
    repl.execute(
        "Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) left_side) right_side.",
    )
    .unwrap();
    repl.execute("Definition d : Derivation := try_auto goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();
    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "auto should prove xx+yy=yy+xx via ring");
}

// =============================================================================
// CC-SOLVABLE (auto delegates to cc)
// =============================================================================

#[test]
fn test_auto_cc_reflexive() {
    let mut repl = Repl::new();
    // f(x) = f(x)
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition f_x : Syntax := SApp (SName \"f\") x.")
        .unwrap();
    repl.execute(
        "Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") (SName \"T\")) f_x) f_x.",
    )
    .unwrap();
    repl.execute("Definition d : Derivation := try_auto goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();
    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "auto should prove f(x)=f(x) via cc");
}

// =============================================================================
// OMEGA-SOLVABLE (auto delegates to omega)
// =============================================================================

#[test]
fn test_auto_omega_lt() {
    let mut repl = Repl::new();
    // 2 < 5
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Lt\") (SLit 2)) (SLit 5).")
        .unwrap();
    repl.execute("Definition d : Derivation := try_auto goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();
    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "auto should prove 2 < 5 via omega");
}

#[test]
fn test_auto_omega_integer_specific() {
    let mut repl = Repl::new();
    // x > 0 -> x >= 1 (integer-specific)
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition hyp : Syntax := SApp (SApp (SName \"Gt\") x) (SLit 0).")
        .unwrap();
    repl.execute("Definition concl : Syntax := SApp (SApp (SName \"Ge\") x) (SLit 1).")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"implies\") hyp) concl.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_auto goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();
    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(
        concluded, original,
        "auto should prove x>0 -> x>=1 via omega"
    );
}

// =============================================================================
// LIA-SOLVABLE (auto delegates to lia)
// =============================================================================

#[test]
fn test_auto_lia_le() {
    let mut repl = Repl::new();
    // x <= x
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Le\") x) x.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_auto goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();
    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "auto should prove x <= x");
}

// =============================================================================
// FAILURE CASES
// =============================================================================

#[test]
fn test_auto_fails_false() {
    let mut repl = Repl::new();
    // 5 < 2 (false)
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Lt\") (SLit 5)) (SLit 2).")
        .unwrap();
    repl.execute("Definition d : Derivation := try_auto goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();
    let result = repl.execute("Eval result.").unwrap();
    assert!(result.contains("Error"), "auto should fail on 5 < 2");
}

#[test]
fn test_auto_fails_unprovable() {
    let mut repl = Repl::new();
    // x = y (unprovable without assumptions)
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition y : Syntax := SVar 1.").unwrap();
    repl.execute(
        "Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) x) y.",
    )
    .unwrap();
    repl.execute("Definition d : Derivation := try_auto goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();
    let result = repl.execute("Eval result.").unwrap();
    assert!(result.contains("Error"), "auto should fail on x = y");
}
