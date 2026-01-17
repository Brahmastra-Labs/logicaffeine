//! Phase 96: The Wizard (Verified Tactics)
//!
//! Implements the first tactic:
//! - DRefl: Reflexivity proof constructor
//! - try_refl: Reflexivity tactic

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// DREFL: TYPE CHECK
// =============================================================================

#[test]
fn test_drefl_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DRefl.").expect("Check DRefl");
    assert_eq!(result, "DRefl : Syntax -> Syntax -> Derivation");
}

#[test]
fn test_drefl_construction() {
    let mut repl = Repl::new();

    repl.execute("Definition T : Syntax := SSort (UType 0).")
        .expect("Define T");
    repl.execute("Definition a : Syntax := SName \"a\".")
        .expect("Define a");
    repl.execute("Definition d : Derivation := DRefl T a.")
        .expect("Define d");

    let result = repl.execute("Check d.").expect("Check d");
    assert_eq!(result, "d : Derivation");
}

// =============================================================================
// CONCLUDES DREFL: COMPUTATION
// =============================================================================

#[test]
fn test_concludes_drefl() {
    let mut repl = Repl::new();

    repl.execute("Definition T : Syntax := SName \"Nat\".")
        .expect("Define T");
    repl.execute("Definition a : Syntax := SName \"Zero\".")
        .expect("Define a");
    repl.execute("Definition d : Derivation := DRefl T a.")
        .expect("Define d");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    // Should be: Eq Nat Zero Zero = SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SName "Zero")) (SName "Zero")
    assert!(
        result.contains("SName \"Eq\""),
        "Should contain Eq: {}",
        result
    );
    assert!(
        result.contains("SName \"Nat\""),
        "Should contain Nat: {}",
        result
    );
    assert!(
        result.contains("SName \"Zero\""),
        "Should contain Zero: {}",
        result
    );
}

#[test]
fn test_concludes_drefl_equals_expected() {
    let mut repl = Repl::new();

    repl.execute("Definition T : Syntax := SName \"Nat\".")
        .expect("Define T");
    repl.execute("Definition a : Syntax := SName \"Zero\".")
        .expect("Define a");
    repl.execute("Definition d : Derivation := DRefl T a.")
        .expect("Define d");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    // Build expected: Eq T a a
    repl.execute("Definition expected : Syntax := SApp (SApp (SApp (SName \"Eq\") T) a) a.")
        .expect("Define expected");

    let result = repl.execute("Eval result.").expect("Eval result");
    let expected = repl.execute("Eval expected.").expect("Eval expected");
    assert_eq!(result, expected);
}

// =============================================================================
// TRY_REFL: TYPE CHECK
// =============================================================================

#[test]
fn test_try_refl_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_refl.").expect("Check try_refl");
    assert_eq!(result, "try_refl : Syntax -> Derivation");
}

// =============================================================================
// TRY_REFL: SUCCESS CASES
// =============================================================================

#[test]
fn test_try_refl_on_equality_same() {
    let mut repl = Repl::new();

    // Goal: Eq Nat Zero Zero (where left == right)
    repl.execute("Definition T : Syntax := SName \"Nat\".")
        .expect("Define T");
    repl.execute("Definition a : Syntax := SName \"Zero\".")
        .expect("Define a");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) a) a.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_refl goal.")
        .expect("Define d");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Tactic should prove the goal");
}

#[test]
fn test_try_refl_on_complex_term() {
    let mut repl = Repl::new();

    // Goal: Eq Nat (S Zero) (S Zero)
    repl.execute("Definition T : Syntax := SName \"Nat\".")
        .expect("Define T");
    repl.execute("Definition succ_zero : Syntax := SApp (SName \"S\") (SName \"Zero\").")
        .expect("Define succ_zero");
    repl.execute(
        "Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) succ_zero) succ_zero.",
    )
    .expect("Define goal");

    repl.execute("Definition d : Derivation := try_refl goal.")
        .expect("Define d");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Tactic should prove complex equality");
}

#[test]
fn test_try_refl_with_type0() {
    let mut repl = Repl::new();

    // Goal: Eq Type0 Nat Nat
    repl.execute("Definition T : Syntax := SSort (UType 0).")
        .expect("Define T");
    repl.execute("Definition a : Syntax := SName \"Nat\".")
        .expect("Define a");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) a) a.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_refl goal.")
        .expect("Define d");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original);
}

// =============================================================================
// TRY_REFL: FAILURE CASES
// =============================================================================

#[test]
fn test_try_refl_on_non_equality() {
    let mut repl = Repl::new();

    // Not an equality goal
    repl.execute("Definition P : Syntax := SName \"P\".")
        .expect("Define P");
    repl.execute("Definition d : Derivation := try_refl P.")
        .expect("Define d");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(
        result, "(SName \"Error\")",
        "Should return error for non-equality"
    );
}

#[test]
fn test_try_refl_on_inequality() {
    let mut repl = Repl::new();

    // Goal: Eq Nat Zero (S Zero) - left != right
    repl.execute("Definition T : Syntax := SName \"Nat\".")
        .expect("Define T");
    repl.execute("Definition a : Syntax := SName \"Zero\".")
        .expect("Define a");
    repl.execute("Definition b : Syntax := SApp (SName \"S\") (SName \"Zero\").")
        .expect("Define b");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) a) b.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_refl goal.")
        .expect("Define d");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(
        result, "(SName \"Error\")",
        "Should return error for inequality"
    );
}

#[test]
fn test_try_refl_on_application() {
    let mut repl = Repl::new();

    // A single application, not an Eq
    repl.execute("Definition goal : Syntax := SApp (SName \"P\") (SName \"x\").")
        .expect("Define goal");
    repl.execute("Definition d : Derivation := try_refl goal.")
        .expect("Define d");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SName \"Error\")");
}

// =============================================================================
// TYPE ERRORS
// =============================================================================

#[test]
fn test_drefl_type_error_first_arg() {
    let mut repl = Repl::new();

    // DRefl expects Syntax, not Int
    let result = repl.execute("Check (DRefl 42 (SName \"a\")).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

#[test]
fn test_drefl_type_error_second_arg() {
    let mut repl = Repl::new();

    // DRefl expects Syntax for both args
    let result = repl.execute("Check (DRefl (SName \"T\") 42).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

#[test]
fn test_try_refl_type_error() {
    let mut repl = Repl::new();

    // try_refl expects Syntax, not Int
    let result = repl.execute("Check (try_refl 42).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

// =============================================================================
// INTEGRATION: TACTIC PRODUCES CORRECT PROOF
// =============================================================================

#[test]
fn test_tactic_proof_is_verified() {
    let mut repl = Repl::new();

    // Full workflow: define goal, run tactic, verify conclusion matches goal
    repl.execute("Definition T : Syntax := SName \"Bool\".")
        .expect("Define T");
    repl.execute("Definition true_val : Syntax := SName \"True\".")
        .expect("Define true_val");
    repl.execute(
        "Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) true_val) true_val.",
    )
    .expect("Define goal");

    // Run tactic
    repl.execute("Definition proof : Derivation := try_refl goal.")
        .expect("Define proof");

    // Verify: concludes proof == goal
    repl.execute("Definition conclusion : Syntax := concludes proof.")
        .expect("Define conclusion");

    let concl = repl.execute("Eval conclusion.").expect("Eval conclusion");
    let goal_val = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concl, goal_val, "Tactic proof should conclude the goal");
}
