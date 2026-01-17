//! Phase 99: The Solver (Computational Reflection)
//!
//! Implements computational reflection in the deep embedding:
//! - DCompute: Proof by computation
//! - try_compute: Computation tactic
//! - syn_step extended for arithmetic

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// SYN_EVAL ARITHMETIC: Basic Operations
// =============================================================================

#[test]
fn test_syn_eval_add() {
    let mut repl = Repl::new();

    // SApp (SApp (SName "add") (SLit 1)) (SLit 1) should reduce to SLit 2
    repl.execute(r#"Definition expr : Syntax := SApp (SApp (SName "add") (SLit 1)) (SLit 1)."#)
        .expect("Define expr");

    let result = repl.execute("Eval (syn_eval 100 expr).").expect("Eval");
    assert_eq!(result, "(SLit 2)");
}

#[test]
fn test_syn_eval_sub() {
    let mut repl = Repl::new();

    repl.execute(r#"Definition expr : Syntax := SApp (SApp (SName "sub") (SLit 10)) (SLit 3)."#)
        .expect("Define expr");

    let result = repl.execute("Eval (syn_eval 100 expr).").expect("Eval");
    assert_eq!(result, "(SLit 7)");
}

#[test]
fn test_syn_eval_mul() {
    let mut repl = Repl::new();

    repl.execute(r#"Definition expr : Syntax := SApp (SApp (SName "mul") (SLit 6)) (SLit 7)."#)
        .expect("Define expr");

    let result = repl.execute("Eval (syn_eval 100 expr).").expect("Eval");
    assert_eq!(result, "(SLit 42)");
}

#[test]
fn test_syn_eval_div() {
    let mut repl = Repl::new();

    repl.execute(r#"Definition expr : Syntax := SApp (SApp (SName "div") (SLit 20)) (SLit 4)."#)
        .expect("Define expr");

    let result = repl.execute("Eval (syn_eval 100 expr).").expect("Eval");
    assert_eq!(result, "(SLit 5)");
}

#[test]
fn test_syn_eval_mod() {
    let mut repl = Repl::new();

    repl.execute(r#"Definition expr : Syntax := SApp (SApp (SName "mod") (SLit 17)) (SLit 5)."#)
        .expect("Define expr");

    let result = repl.execute("Eval (syn_eval 100 expr).").expect("Eval");
    assert_eq!(result, "(SLit 2)");
}

#[test]
fn test_syn_eval_nested_arithmetic() {
    let mut repl = Repl::new();

    // (2 * 5) + 2 = 12
    repl.execute(r#"Definition two_times_five : Syntax := SApp (SApp (SName "mul") (SLit 2)) (SLit 5)."#)
        .expect("Define two_times_five");
    repl.execute(r#"Definition expr : Syntax := SApp (SApp (SName "add") two_times_five) (SLit 2)."#)
        .expect("Define expr");

    let result = repl.execute("Eval (syn_eval 100 expr).").expect("Eval");
    assert_eq!(result, "(SLit 12)");
}

// =============================================================================
// DCOMPUTE: TYPE CHECK
// =============================================================================

#[test]
fn test_dcompute_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DCompute.").expect("Check DCompute");
    assert_eq!(result, "DCompute : Syntax -> Derivation");
}

// =============================================================================
// TRY_COMPUTE: TYPE CHECK
// =============================================================================

#[test]
fn test_try_compute_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_compute.").expect("Check try_compute");
    assert_eq!(result, "try_compute : Syntax -> Derivation");
}

// =============================================================================
// TRY_COMPUTE: SUCCESS CASES
// =============================================================================

#[test]
fn test_try_compute_one_plus_one() {
    let mut repl = Repl::new();

    // Goal: Eq Int (add 1 1) 2
    repl.execute(r#"Definition lhs : Syntax := SApp (SApp (SName "add") (SLit 1)) (SLit 1)."#)
        .expect("Define lhs");
    repl.execute(r#"Definition rhs : Syntax := SLit 2."#)
        .expect("Define rhs");
    repl.execute(r#"Definition goal : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Int")) lhs) rhs."#)
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_compute goal.")
        .expect("Define d");

    let result = repl.execute("Eval (concludes d).").expect("Eval");
    let expected = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(result, expected, "try_compute should prove 1 + 1 = 2");
}

#[test]
fn test_try_compute_ten_plus_ten() {
    let mut repl = Repl::new();

    // Goal: Eq Int (add 10 10) 20
    repl.execute(r#"Definition lhs : Syntax := SApp (SApp (SName "add") (SLit 10)) (SLit 10)."#)
        .expect("Define lhs");
    repl.execute(r#"Definition rhs : Syntax := SLit 20."#)
        .expect("Define rhs");
    repl.execute(r#"Definition goal : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Int")) lhs) rhs."#)
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_compute goal.")
        .expect("Define d");

    let result = repl.execute("Eval (concludes d).").expect("Eval");
    let expected = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(result, expected, "try_compute should prove 10 + 10 = 20");
}

#[test]
fn test_try_compute_complex_expression() {
    let mut repl = Repl::new();

    // Goal: Eq Int ((2 * 5) + 2) 12
    repl.execute(r#"Definition two_times_five : Syntax := SApp (SApp (SName "mul") (SLit 2)) (SLit 5)."#)
        .expect("Define two_times_five");
    repl.execute(r#"Definition lhs : Syntax := SApp (SApp (SName "add") two_times_five) (SLit 2)."#)
        .expect("Define lhs");
    repl.execute(r#"Definition rhs : Syntax := SLit 12."#)
        .expect("Define rhs");
    repl.execute(r#"Definition goal : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Int")) lhs) rhs."#)
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_compute goal.")
        .expect("Define d");

    let result = repl.execute("Eval (concludes d).").expect("Eval");
    let expected = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(result, expected, "try_compute should prove (2*5)+2 = 12");
}

#[test]
fn test_try_compute_both_sides_compute() {
    let mut repl = Repl::new();

    // Goal: Eq Int (add 2 3) (sub 10 5)
    // Both sides compute to 5
    repl.execute(r#"Definition lhs : Syntax := SApp (SApp (SName "add") (SLit 2)) (SLit 3)."#)
        .expect("Define lhs");
    repl.execute(r#"Definition rhs : Syntax := SApp (SApp (SName "sub") (SLit 10)) (SLit 5)."#)
        .expect("Define rhs");
    repl.execute(r#"Definition goal : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Int")) lhs) rhs."#)
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_compute goal.")
        .expect("Define d");

    let result = repl.execute("Eval (concludes d).").expect("Eval");
    let expected = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(result, expected, "try_compute should prove (2+3) = (10-5)");
}

// =============================================================================
// TRY_COMPUTE: FAILURE CASES
// =============================================================================

#[test]
fn test_try_compute_fails_on_inequality() {
    let mut repl = Repl::new();

    // Goal: Eq Int (add 1 1) 3  (1 + 1 = 3 is false!)
    repl.execute(r#"Definition lhs : Syntax := SApp (SApp (SName "add") (SLit 1)) (SLit 1)."#)
        .expect("Define lhs");
    repl.execute(r#"Definition rhs : Syntax := SLit 3."#)
        .expect("Define rhs");
    repl.execute(r#"Definition goal : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Int")) lhs) rhs."#)
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_compute goal.")
        .expect("Define d");

    let result = repl.execute("Eval (concludes d).").expect("Eval");
    assert_eq!(result, "(SName \"Error\")", "try_compute should fail on 1 + 1 = 3");
}

#[test]
fn test_try_compute_fails_on_non_equality() {
    let mut repl = Repl::new();

    // Not an equality goal
    repl.execute(r#"Definition goal : Syntax := SName "P"."#)
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_compute goal.")
        .expect("Define d");

    let result = repl.execute("Eval (concludes d).").expect("Eval");
    assert_eq!(result, "(SName \"Error\")", "try_compute should fail on non-equality");
}

// =============================================================================
// SOLVE_ARITH: COMPOSITE TACTIC
// =============================================================================

#[test]
fn test_solve_arith_type() {
    let mut repl = Repl::new();

    repl.execute("Definition solve_arith : Syntax -> Derivation := tact_orelse try_refl try_compute.")
        .expect("Define solve_arith");

    let result = repl.execute("Check solve_arith.").expect("Check");
    assert_eq!(result, "solve_arith : Syntax -> Derivation");
}

#[test]
fn test_solve_arith_reflexive() {
    let mut repl = Repl::new();

    repl.execute("Definition solve_arith : Syntax -> Derivation := tact_orelse try_refl try_compute.")
        .expect("Define solve_arith");

    // Goal: Eq Int 5 5 (reflexivity handles this)
    repl.execute(r#"Definition goal : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Int")) (SLit 5)) (SLit 5)."#)
        .expect("Define goal");

    repl.execute("Definition d : Derivation := solve_arith goal.")
        .expect("Apply solve_arith");

    let result = repl.execute("Eval (concludes d).").expect("Eval");
    let expected = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(result, expected, "solve_arith should prove 5 = 5 via reflexivity");
}

#[test]
fn test_solve_arith_computation() {
    let mut repl = Repl::new();

    repl.execute("Definition solve_arith : Syntax -> Derivation := tact_orelse try_refl try_compute.")
        .expect("Define solve_arith");

    // Goal: Eq Int (add 1 1) 2 (computation handles this)
    repl.execute(r#"Definition lhs : Syntax := SApp (SApp (SName "add") (SLit 1)) (SLit 1)."#)
        .expect("Define lhs");
    repl.execute(r#"Definition goal : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Int")) lhs) (SLit 2)."#)
        .expect("Define goal");

    repl.execute("Definition d : Derivation := solve_arith goal.")
        .expect("Apply solve_arith");

    let result = repl.execute("Eval (concludes d).").expect("Eval");
    let expected = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(result, expected, "solve_arith should prove 1 + 1 = 2 via computation");
}

#[test]
fn test_solve_arith_fails() {
    let mut repl = Repl::new();

    repl.execute("Definition solve_arith : Syntax -> Derivation := tact_orelse try_refl try_compute.")
        .expect("Define solve_arith");

    // Goal: Eq Int (add 1 1) 3 (false!)
    repl.execute(r#"Definition lhs : Syntax := SApp (SApp (SName "add") (SLit 1)) (SLit 1)."#)
        .expect("Define lhs");
    repl.execute(r#"Definition goal : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Int")) lhs) (SLit 3)."#)
        .expect("Define goal");

    repl.execute("Definition d : Derivation := solve_arith goal.")
        .expect("Apply solve_arith");

    let result = repl.execute("Eval (concludes d).").expect("Eval");
    assert_eq!(result, "(SName \"Error\")", "solve_arith should fail on 1 + 1 = 3");
}

// =============================================================================
// TYPE ERRORS
// =============================================================================

#[test]
fn test_dcompute_type_error() {
    let mut repl = Repl::new();
    // DCompute expects Syntax, not Int
    let result = repl.execute("Check (DCompute 42).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

#[test]
fn test_try_compute_type_error() {
    let mut repl = Repl::new();
    // try_compute expects Syntax, not Int
    let result = repl.execute("Check (try_compute 42).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

// =============================================================================
// EDGE CASES
// =============================================================================

#[test]
fn test_syn_eval_div_by_zero() {
    let mut repl = Repl::new();

    // Division by zero should remain stuck (not crash)
    repl.execute(r#"Definition expr : Syntax := SApp (SApp (SName "div") (SLit 10)) (SLit 0)."#)
        .expect("Define expr");

    let result = repl.execute("Eval (syn_eval 100 expr).").expect("Eval");
    // Should remain unreduced (stuck)
    assert!(result.contains("div"), "Division by zero should remain stuck");
}

#[test]
fn test_syn_eval_partial_application() {
    let mut repl = Repl::new();

    // Partial application should remain stuck
    repl.execute(r#"Definition expr : Syntax := SApp (SName "add") (SLit 1)."#)
        .expect("Define expr");

    let result = repl.execute("Eval (syn_eval 100 expr).").expect("Eval");
    // Should remain unreduced (partial application)
    assert!(result.contains("add"), "Partial application should remain stuck");
}

#[test]
fn test_syn_eval_non_literal_args() {
    let mut repl = Repl::new();

    // add x 1 where x is a variable should remain stuck
    repl.execute(r#"Definition expr : Syntax := SApp (SApp (SName "add") (SVar 0)) (SLit 1)."#)
        .expect("Define expr");

    let result = repl.execute("Eval (syn_eval 100 expr).").expect("Eval");
    // Should remain unreduced (variable not a literal)
    assert!(result.contains("add"), "Non-literal args should remain stuck");
}
