//! Phase 88: The Engine of Syntax (Substitution)
//!
//! Implements capture-avoiding substitution for De Bruijn indices:
//! - syn_lift: Shift free variable indices
//! - syn_subst: Replace a variable with a term

use logos::interface::Repl;

// =============================================================================
// SYN_LIFT: TYPE CHECK
// =============================================================================

#[test]
fn test_syn_lift_type() {
    let mut repl = Repl::new();

    // syn_lift : Int -> Int -> Syntax -> Syntax
    let result = repl.execute("Check syn_lift.").expect("Check syn_lift");
    assert_eq!(result, "syn_lift : Int -> Int -> Syntax -> Syntax");
}

// =============================================================================
// SYN_LIFT: VARIABLES
// =============================================================================

#[test]
fn test_lift_var_free() {
    let mut repl = Repl::new();

    // syn_lift 1 0 (SVar 0) = SVar 1
    // Variable 0 is free at cutoff 0, so shift by 1
    repl.execute("Definition lifted : Syntax := syn_lift 1 0 (SVar 0).")
        .expect("Define");
    repl.execute("Definition expected : Syntax := SVar 1.")
        .expect("Define");

    let result = repl.execute("Eval lifted.").expect("Eval");
    assert_eq!(result, "(SVar 1)");
}

#[test]
fn test_lift_var_free_larger() {
    let mut repl = Repl::new();

    // syn_lift 2 0 (SVar 3) = SVar 5
    repl.execute("Definition lifted : Syntax := syn_lift 2 0 (SVar 3).")
        .expect("Define");

    let result = repl.execute("Eval lifted.").expect("Eval");
    assert_eq!(result, "(SVar 5)");
}

#[test]
fn test_lift_var_bound() {
    let mut repl = Repl::new();

    // syn_lift 1 1 (SVar 0) = SVar 0
    // Variable 0 is bound at cutoff 1 (0 < 1), so no shift
    repl.execute("Definition lifted : Syntax := syn_lift 1 1 (SVar 0).")
        .expect("Define");

    let result = repl.execute("Eval lifted.").expect("Eval");
    assert_eq!(result, "(SVar 0)");
}

#[test]
fn test_lift_var_at_cutoff() {
    let mut repl = Repl::new();

    // syn_lift 1 1 (SVar 1) = SVar 2
    // Variable 1 is free at cutoff 1 (1 >= 1), so shift
    repl.execute("Definition lifted : Syntax := syn_lift 1 1 (SVar 1).")
        .expect("Define");

    let result = repl.execute("Eval lifted.").expect("Eval");
    assert_eq!(result, "(SVar 2)");
}

// =============================================================================
// SYN_LIFT: NON-BINDING CONSTRUCTS
// =============================================================================

#[test]
fn test_lift_global() {
    let mut repl = Repl::new();

    // syn_lift 1 0 (SGlobal 42) = SGlobal 42
    // Globals don't have free variables
    repl.execute("Definition lifted : Syntax := syn_lift 1 0 (SGlobal 42).")
        .expect("Define");

    let result = repl.execute("Eval lifted.").expect("Eval");
    assert_eq!(result, "(SGlobal 42)");
}

#[test]
fn test_lift_sort() {
    let mut repl = Repl::new();

    // syn_lift 1 0 (SSort UProp) = SSort UProp
    // Sorts don't have free variables
    repl.execute("Definition lifted : Syntax := syn_lift 1 0 (SSort UProp).")
        .expect("Define");

    let result = repl.execute("Eval lifted.").expect("Eval");
    assert_eq!(result, "(SSort UProp)");
}

#[test]
fn test_lift_app() {
    let mut repl = Repl::new();

    // syn_lift 1 0 (SApp (SVar 0) (SVar 1)) = SApp (SVar 1) (SVar 2)
    repl.execute("Definition lifted : Syntax := syn_lift 1 0 (SApp (SVar 0) (SVar 1)).")
        .expect("Define");

    let result = repl.execute("Eval lifted.").expect("Eval");
    assert_eq!(result, "((SApp (SVar 1)) (SVar 2))");
}

// =============================================================================
// SYN_LIFT: BINDERS (CRITICAL)
// =============================================================================

#[test]
fn test_lift_lambda_body_bound() {
    let mut repl = Repl::new();

    // syn_lift 1 0 (SLam (SSort UProp) (SVar 0))
    //   = SLam (SSort UProp) (SVar 0)
    // The SVar 0 in the body is bound by the lambda, so it doesn't shift
    // Cutoff increments to 1 when we go under the binder
    repl.execute(
        "Definition lifted : Syntax := syn_lift 1 0 (SLam (SSort UProp) (SVar 0)).",
    )
    .expect("Define");

    let result = repl.execute("Eval lifted.").expect("Eval");
    assert_eq!(result, "((SLam (SSort UProp)) (SVar 0))");
}

#[test]
fn test_lift_lambda_body_free() {
    let mut repl = Repl::new();

    // syn_lift 1 0 (SLam (SSort UProp) (SVar 1))
    //   = SLam (SSort UProp) (SVar 2)
    // The SVar 1 in the body is free (refers to outside the lambda)
    // Under the binder, cutoff is 1, so SVar 1 >= 1 means shift
    repl.execute(
        "Definition lifted : Syntax := syn_lift 1 0 (SLam (SSort UProp) (SVar 1)).",
    )
    .expect("Define");

    let result = repl.execute("Eval lifted.").expect("Eval");
    assert_eq!(result, "((SLam (SSort UProp)) (SVar 2))");
}

#[test]
fn test_lift_pi() {
    let mut repl = Repl::new();

    // syn_lift 1 0 (SPi (SVar 0) (SVar 0))
    //   = SPi (SVar 1) (SVar 0)
    // First SVar 0 is in param type (not under binder) -> shifts to 1
    // Second SVar 0 is in body (under binder, cutoff=1) -> bound, no shift
    repl.execute("Definition lifted : Syntax := syn_lift 1 0 (SPi (SVar 0) (SVar 0)).")
        .expect("Define");

    let result = repl.execute("Eval lifted.").expect("Eval");
    assert_eq!(result, "((SPi (SVar 1)) (SVar 0))");
}

#[test]
fn test_lift_nested_lambda() {
    let mut repl = Repl::new();

    // λ. λ. var2 (free, refers to outside both lambdas)
    // syn_lift 1 0 (SLam (SSort UProp) (SLam (SSort UProp) (SVar 2)))
    //   = SLam (SSort UProp) (SLam (SSort UProp) (SVar 3))
    // Under 2 binders, cutoff=2, SVar 2 >= 2 means shift
    repl.execute(
        "Definition lifted : Syntax := syn_lift 1 0 (SLam (SSort UProp) (SLam (SSort UProp) (SVar 2))).",
    )
    .expect("Define");

    let result = repl.execute("Eval lifted.").expect("Eval");
    assert_eq!(
        result,
        "((SLam (SSort UProp)) ((SLam (SSort UProp)) (SVar 3)))"
    );
}

// =============================================================================
// SYN_SUBST: TYPE CHECK
// =============================================================================

#[test]
fn test_syn_subst_type() {
    let mut repl = Repl::new();

    // syn_subst : Syntax -> Int -> Syntax -> Syntax
    let result = repl.execute("Check syn_subst.").expect("Check syn_subst");
    assert_eq!(result, "syn_subst : Syntax -> Int -> Syntax -> Syntax");
}

// =============================================================================
// SYN_SUBST: VARIABLES
// =============================================================================

#[test]
fn test_subst_var_match() {
    let mut repl = Repl::new();

    // syn_subst (SSort UProp) 0 (SVar 0) = SSort UProp
    // Variable 0 matches index 0, substitute
    repl.execute("Definition result : Syntax := syn_subst (SSort UProp) 0 (SVar 0).")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SSort UProp)");
}

#[test]
fn test_subst_var_no_match_greater() {
    let mut repl = Repl::new();

    // syn_subst (SSort UProp) 0 (SVar 1) = SVar 1
    // Variable 1 doesn't match index 0, no change
    repl.execute("Definition result : Syntax := syn_subst (SSort UProp) 0 (SVar 1).")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SVar 1)");
}

#[test]
fn test_subst_var_no_match_less() {
    let mut repl = Repl::new();

    // syn_subst (SSort UProp) 2 (SVar 0) = SVar 0
    // Variable 0 doesn't match index 2
    repl.execute("Definition result : Syntax := syn_subst (SSort UProp) 2 (SVar 0).")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SVar 0)");
}

// =============================================================================
// SYN_SUBST: NON-BINDING CONSTRUCTS
// =============================================================================

#[test]
fn test_subst_global() {
    let mut repl = Repl::new();

    // syn_subst A 0 (SGlobal 42) = SGlobal 42
    repl.execute("Definition result : Syntax := syn_subst (SVar 99) 0 (SGlobal 42).")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SGlobal 42)");
}

#[test]
fn test_subst_sort() {
    let mut repl = Repl::new();

    // syn_subst A 0 (SSort UProp) = SSort UProp
    repl.execute("Definition result : Syntax := syn_subst (SVar 99) 0 (SSort UProp).")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SSort UProp)");
}

#[test]
fn test_subst_app() {
    let mut repl = Repl::new();

    // syn_subst A 0 (SApp (SVar 0) (SVar 1))
    //   = SApp A (SVar 1)
    repl.execute(
        "Definition result : Syntax := syn_subst (SSort UProp) 0 (SApp (SVar 0) (SVar 1)).",
    )
    .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "((SApp (SSort UProp)) (SVar 1))");
}

// =============================================================================
// SYN_SUBST: BINDERS (CRITICAL - CAPTURE AVOIDANCE)
// =============================================================================

#[test]
fn test_subst_lambda_body_bound() {
    let mut repl = Repl::new();

    // syn_subst A 0 (SLam T (SVar 0))
    //   = SLam (subst A 0 T) (SVar 0)
    // SVar 0 in body is NOW bound by the lambda (it's the lambda's param)
    // We substitute at index 1 in the body, so SVar 0 doesn't match
    repl.execute(
        "Definition result : Syntax := syn_subst (SSort (UType 1)) 0 (SLam (SSort UProp) (SVar 0)).",
    )
    .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "((SLam (SSort UProp)) (SVar 0))");
}

#[test]
fn test_subst_lambda_body_free() {
    let mut repl = Repl::new();

    // syn_subst A 0 (SLam T (SVar 1))
    //   = SLam (subst A 0 T) (subst (lift A) 1 (SVar 1))
    // SVar 1 in body refers to index 0 outside the lambda
    // Under the binder, we substitute at index 1, so SVar 1 matches!
    //   -> becomes (lift A 1 0) = A (lifted by 1)
    // If A = SSort UProp, lift doesn't change it
    repl.execute(
        "Definition result : Syntax := syn_subst (SSort UProp) 0 (SLam (SSort (UType 0)) (SVar 1)).",
    )
    .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "((SLam (SSort (UType 0))) (SSort UProp))");
}

#[test]
fn test_subst_lambda_replacement_lifted() {
    let mut repl = Repl::new();

    // Key test: replacement contains free variable that would be captured
    // syn_subst (SVar 0) 0 (SLam (SSort UProp) (SVar 1))
    //   = SLam (SSort UProp) (SVar 1)
    // The replacement (SVar 0) must be lifted when going under binder
    //   lift 1 0 (SVar 0) = SVar 1
    // So we substitute SVar 1 for index 1 in body, getting SVar 1
    repl.execute(
        "Definition result : Syntax := syn_subst (SVar 0) 0 (SLam (SSort UProp) (SVar 1)).",
    )
    .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "((SLam (SSort UProp)) (SVar 1))");
}

#[test]
fn test_subst_pi_both_positions() {
    let mut repl = Repl::new();

    // syn_subst A 0 (SPi (SVar 0) (SVar 0))
    //   = SPi (subst A 0 (SVar 0)) (subst (lift A) 1 (SVar 0))
    //   = SPi A (SVar 0)
    // First SVar 0 matches (in param type, not under binder)
    // Second SVar 0 is bound by Pi (we substitute at index 1)
    repl.execute(
        "Definition result : Syntax := syn_subst (SSort UProp) 0 (SPi (SVar 0) (SVar 0)).",
    )
    .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "((SPi (SSort UProp)) (SVar 0))");
}

// =============================================================================
// SYN_SUBST: COMPLEX EXAMPLES
// =============================================================================

#[test]
fn test_subst_nested_lambda() {
    let mut repl = Repl::new();

    // λ. λ. var2 (refers to outside both lambdas)
    // syn_subst A 0 (SLam T (SLam T (SVar 2)))
    // Under 2 binders, we substitute at index 2, so SVar 2 matches
    repl.execute(
        "Definition result : Syntax := syn_subst (SSort UProp) 0 (SLam (SSort (UType 0)) (SLam (SSort (UType 0)) (SVar 2))).",
    )
    .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // A = SSort UProp, lifted twice: still SSort UProp (no free vars)
    assert_eq!(
        result,
        "((SLam (SSort (UType 0))) ((SLam (SSort (UType 0))) (SSort UProp)))"
    );
}

#[test]
fn test_subst_identity_application() {
    let mut repl = Repl::new();

    // Beta reduction simulation: (λx.x) A → A
    // The body is SVar 0, substituting A for 0 gives A
    // syn_subst A 0 (SVar 0) = A
    repl.execute("Definition body : Syntax := SVar 0.")
        .expect("Define");
    repl.execute("Definition arg : Syntax := SSort (UType 42).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_subst arg 0 body.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SSort (UType 42))");
}

#[test]
fn test_subst_const_function() {
    let mut repl = Repl::new();

    // (λx.λy.x) applied: body is (SLam T (SVar 1))
    // syn_subst A 0 (SLam T (SVar 1))
    // Under the inner lambda, index becomes 1, SVar 1 matches
    repl.execute("Definition body : Syntax := SLam (SSort UProp) (SVar 1).")
        .expect("Define");
    repl.execute("Definition arg : Syntax := SSort (UType 99).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_subst arg 0 body.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // A lifted once is still A (no free vars), substituted for SVar 1
    assert_eq!(result, "((SLam (SSort UProp)) (SSort (UType 99)))");
}

// =============================================================================
// ERROR CASES
// =============================================================================

#[test]
fn test_lift_type_error() {
    let mut repl = Repl::new();

    // syn_lift expects Int for amount, not Nat
    let result = repl.execute("Check (syn_lift Zero 0 (SVar 0)).");
    assert!(result.is_err(), "Should reject Nat where Int expected");
}

#[test]
fn test_subst_type_error() {
    let mut repl = Repl::new();

    // syn_subst expects Syntax for replacement, not Int
    let result = repl.execute("Check (syn_subst 42 0 (SVar 0)).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}
