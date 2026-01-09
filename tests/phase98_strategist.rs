//! Phase 98: The Strategist (Tactic Combinators)
//!
//! Implements tactic combinators in the deep embedding:
//! - tact_orelse: Try first tactic, if it fails try second
//! - tact_fail: A tactic that always fails

use logos::interface::Repl;

// =============================================================================
// TACT_FAIL: TYPE CHECK
// =============================================================================

#[test]
fn test_tact_fail_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check tact_fail.").expect("Check tact_fail");
    assert_eq!(result, "tact_fail : Syntax -> Derivation");
}

#[test]
fn test_tact_fail_returns_error() {
    let mut repl = Repl::new();

    // Any goal
    repl.execute(r#"Definition goal : Syntax := SName "anything"."#)
        .expect("Define goal");

    repl.execute("Definition d : Derivation := tact_fail goal.")
        .expect("Define d");

    let result = repl.execute("Eval (concludes d).").expect("Eval");
    assert_eq!(result, "(SName \"Error\")", "tact_fail should always return Error");
}

// =============================================================================
// TACT_ORELSE: TYPE CHECK
// =============================================================================

#[test]
fn test_tact_orelse_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check tact_orelse.").expect("Check tact_orelse");
    // Note: The kernel prints function types without parentheses for parameters
    assert_eq!(
        result,
        "tact_orelse : Syntax -> Derivation -> Syntax -> Derivation -> Syntax -> Derivation"
    );
}

// =============================================================================
// TACT_ORELSE: FIRST TACTIC SUCCEEDS
// =============================================================================

#[test]
fn test_tact_orelse_first_succeeds() {
    let mut repl = Repl::new();

    // Goal: Eq Nat Zero Zero (reflexivity will succeed)
    repl.execute(r#"Definition goal : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SName "Zero")) (SName "Zero")."#)
        .expect("Define goal");

    // tact_orelse try_refl tact_fail goal
    // Since try_refl succeeds on Eq Nat Zero Zero, should return try_refl's result
    repl.execute("Definition d : Derivation := tact_orelse try_refl tact_fail goal.")
        .expect("Define d");

    let result = repl.execute("Eval (concludes d).").expect("Eval");
    let expected = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(result, expected, "Should succeed with try_refl's proof");
}

// =============================================================================
// TACT_ORELSE: FIRST FAILS, SECOND SUCCEEDS
// =============================================================================

#[test]
fn test_tact_orelse_first_fails_second_succeeds() {
    let mut repl = Repl::new();

    // Goal: Eq Nat Zero Zero
    repl.execute(r#"Definition goal : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SName "Zero")) (SName "Zero")."#)
        .expect("Define goal");

    // tact_orelse tact_fail try_refl goal
    // tact_fail fails, so try_refl runs and succeeds
    repl.execute("Definition d : Derivation := tact_orelse tact_fail try_refl goal.")
        .expect("Define d");

    let result = repl.execute("Eval (concludes d).").expect("Eval");
    let expected = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(result, expected, "Should fall through to try_refl");
}

// =============================================================================
// TACT_ORELSE: BOTH FAIL
// =============================================================================

#[test]
fn test_tact_orelse_both_fail() {
    let mut repl = Repl::new();

    // Goal: something that try_refl can't prove (inequality)
    repl.execute(r#"Definition goal : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SName "Zero")) (SApp (SName "Succ") (SName "Zero"))."#)
        .expect("Define goal");

    // tact_orelse try_refl tact_fail goal
    // try_refl fails on inequality, tact_fail always fails
    repl.execute("Definition d : Derivation := tact_orelse try_refl tact_fail goal.")
        .expect("Define d");

    let result = repl.execute("Eval (concludes d).").expect("Eval");
    assert_eq!(result, "(SName \"Error\")", "Both tactics should fail");
}

// =============================================================================
// TACT_ORELSE: CHAINING
// =============================================================================

#[test]
fn test_tact_orelse_chaining() {
    let mut repl = Repl::new();

    // Goal: Eq Nat Zero Zero
    repl.execute(r#"Definition goal : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SName "Zero")) (SName "Zero")."#)
        .expect("Define goal");

    // Chain: tact_orelse tact_fail (tact_orelse tact_fail try_refl)
    // First tact_fail fails, then inner tact_orelse tries tact_fail (fails), then try_refl (succeeds)
    repl.execute("Definition d : Derivation := tact_orelse tact_fail (tact_orelse tact_fail try_refl) goal.")
        .expect("Define d");

    let result = repl.execute("Eval (concludes d).").expect("Eval");
    let expected = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(result, expected, "Chained tactics should eventually succeed");
}

// =============================================================================
// TACT_ORELSE: PARTIAL APPLICATION
// =============================================================================

#[test]
fn test_tact_orelse_partial_application() {
    let mut repl = Repl::new();

    // Define a composite tactic via partial application
    repl.execute("Definition solve_eq : Syntax -> Derivation := tact_orelse try_refl tact_fail.")
        .expect("Define solve_eq");

    let result = repl.execute("Check solve_eq.").expect("Check");
    assert_eq!(result, "solve_eq : Syntax -> Derivation");

    // Use the composite tactic
    repl.execute(r#"Definition goal : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SName "Zero")) (SName "Zero")."#)
        .expect("Define goal");

    repl.execute("Definition d : Derivation := solve_eq goal.")
        .expect("Apply solve_eq");

    let result = repl.execute("Eval (concludes d).").expect("Eval");
    let expected = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(result, expected);
}

// =============================================================================
// TYPE ERRORS
// =============================================================================

#[test]
fn test_tact_orelse_type_error_first_arg() {
    let mut repl = Repl::new();
    // First arg should be (Syntax -> Derivation), not Int
    let result = repl.execute(r#"Check (tact_orelse 42 try_refl (SName "x"))."#);
    assert!(result.is_err(), "Should reject Int where tactic expected");
}

#[test]
fn test_tact_orelse_type_error_second_arg() {
    let mut repl = Repl::new();
    // Second arg should be (Syntax -> Derivation), not Int
    let result = repl.execute(r#"Check (tact_orelse try_refl 42 (SName "x"))."#);
    assert!(result.is_err(), "Should reject Int where tactic expected");
}

#[test]
fn test_tact_orelse_type_error_goal() {
    let mut repl = Repl::new();
    // Third arg should be Syntax, not Int
    let result = repl.execute("Check (tact_orelse try_refl tact_fail 42).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

#[test]
fn test_tact_fail_type_error() {
    let mut repl = Repl::new();
    // tact_fail expects Syntax, not Int
    let result = repl.execute("Check (tact_fail 42).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

// =============================================================================
// INTEGRATION: COMPOSITE TACTIC DEFINITION
// =============================================================================

#[test]
fn test_solve_trivial_definition() {
    let mut repl = Repl::new();

    // Define a composite tactic that tries reflexivity
    repl.execute("Definition solve_trivial : Syntax -> Derivation := tact_orelse try_refl tact_fail.")
        .expect("Define solve_trivial");

    // Test on reflexive equality
    repl.execute(r#"Definition goal1 : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Bool")) (SName "True")) (SName "True")."#)
        .expect("Define goal1");

    repl.execute("Definition d1 : Derivation := solve_trivial goal1.")
        .expect("Apply solve_trivial");

    let result = repl.execute("Eval (concludes d1).").expect("Eval");
    let expected = repl.execute("Eval goal1.").expect("Eval goal1");
    assert_eq!(result, expected, "solve_trivial should prove Eq Bool True True");
}

#[test]
fn test_solve_trivial_fails_on_inequality() {
    let mut repl = Repl::new();

    repl.execute("Definition solve_trivial : Syntax -> Derivation := tact_orelse try_refl tact_fail.")
        .expect("Define solve_trivial");

    // Test on non-reflexive equality (should fail)
    repl.execute(r#"Definition goal2 : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Bool")) (SName "True")) (SName "False")."#)
        .expect("Define goal2");

    repl.execute("Definition d2 : Derivation := solve_trivial goal2.")
        .expect("Apply solve_trivial");

    let result = repl.execute("Eval (concludes d2).").expect("Eval");
    assert_eq!(result, "(SName \"Error\")", "solve_trivial should fail on True != False");
}
