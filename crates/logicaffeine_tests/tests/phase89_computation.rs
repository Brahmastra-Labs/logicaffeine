//! Phase 89: The Pulse (Computation)
//!
//! Implements beta reduction on embedded syntax:
//! - syn_beta: Substitute argument for variable 0 in body
//! - syn_step: Single-step head reduction

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// SYN_BETA: TYPE CHECK
// =============================================================================

#[test]
fn test_syn_beta_type() {
    let mut repl = Repl::new();

    // syn_beta : Syntax -> Syntax -> Syntax
    let result = repl.execute("Check syn_beta.").expect("Check syn_beta");
    assert_eq!(result, "syn_beta : Syntax -> Syntax -> Syntax");
}

// =============================================================================
// SYN_BETA: IDENTITY FUNCTION
// =============================================================================

#[test]
fn test_beta_identity() {
    let mut repl = Repl::new();

    // (λx.x) A → A
    // Body is SVar 0, arg is A, result is A
    repl.execute("Definition body : Syntax := SVar 0.")
        .expect("Define");
    repl.execute("Definition arg : Syntax := SSort UProp.")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_beta body arg.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SSort UProp)");
}

#[test]
fn test_beta_identity_complex_arg() {
    let mut repl = Repl::new();

    // (λx.x) (SLam T (SVar 0)) → SLam T (SVar 0)
    repl.execute("Definition body : Syntax := SVar 0.")
        .expect("Define");
    repl.execute("Definition arg : Syntax := SLam (SSort UProp) (SVar 0).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_beta body arg.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "((SLam (SSort UProp)) (SVar 0))");
}

// =============================================================================
// SYN_BETA: CONSTANT FUNCTION
// =============================================================================

#[test]
fn test_beta_const() {
    let mut repl = Repl::new();

    // (λx.y) A → y  where y is a free variable (SVar 1 in body)
    // SVar 1 doesn't match index 0, so it's unchanged
    repl.execute("Definition body : Syntax := SVar 1.")
        .expect("Define");
    repl.execute("Definition arg : Syntax := SSort (UType 42).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_beta body arg.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SVar 1)");
}

// =============================================================================
// SYN_BETA: NESTED LAMBDAS
// =============================================================================

#[test]
fn test_beta_nested_bound() {
    let mut repl = Repl::new();

    // (λx. λy. y) A → λy. y
    // Body is SLam T (SVar 0), the inner SVar 0 is bound by inner lambda
    repl.execute("Definition body : Syntax := SLam (SSort UProp) (SVar 0).")
        .expect("Define");
    repl.execute("Definition arg : Syntax := SSort (UType 99).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_beta body arg.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // Inner SVar 0 is bound, doesn't get substituted
    assert_eq!(result, "((SLam (SSort UProp)) (SVar 0))");
}

#[test]
fn test_beta_nested_free() {
    let mut repl = Repl::new();

    // (λx. λy. x) A → λy. A
    // Body is SLam T (SVar 1), the SVar 1 refers to outer lambda's param
    repl.execute("Definition body : Syntax := SLam (SSort UProp) (SVar 1).")
        .expect("Define");
    repl.execute("Definition arg : Syntax := SSort (UType 99).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_beta body arg.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // SVar 1 in body matches index 1 under binder, gets A
    assert_eq!(result, "((SLam (SSort UProp)) (SSort (UType 99)))");
}

// =============================================================================
// SYN_BETA: CAPTURE AVOIDANCE
// =============================================================================

#[test]
fn test_beta_capture_avoidance() {
    let mut repl = Repl::new();

    // (λx. λy. x) (SVar 0) → λy. (SVar 1)
    // Body is SLam T (SVar 1)
    // Arg is SVar 0 (a free variable)
    // When we go under the binder, arg gets lifted to SVar 1
    repl.execute("Definition body : Syntax := SLam (SSort UProp) (SVar 1).")
        .expect("Define");
    repl.execute("Definition arg : Syntax := SVar 0.")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_beta body arg.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // The free SVar 0 in arg gets lifted when entering the binder
    assert_eq!(result, "((SLam (SSort UProp)) (SVar 1))");
}

// =============================================================================
// SYN_STEP: TYPE CHECK
// =============================================================================

#[test]
fn test_syn_step_type() {
    let mut repl = Repl::new();

    // syn_step : Syntax -> Syntax
    let result = repl.execute("Check syn_step.").expect("Check syn_step");
    assert_eq!(result, "syn_step : Syntax -> Syntax");
}

// =============================================================================
// SYN_STEP: DIRECT BETA REDEX
// =============================================================================

#[test]
fn test_step_beta_redex() {
    let mut repl = Repl::new();

    // (λx.x) A → A
    // SApp (SLam T (SVar 0)) A → A
    repl.execute(
        "Definition redex : Syntax := SApp (SLam (SSort UProp) (SVar 0)) (SSort (UType 42)).",
    )
    .expect("Define");
    repl.execute("Definition result : Syntax := syn_step redex.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SSort (UType 42))");
}

#[test]
fn test_step_const_redex() {
    let mut repl = Repl::new();

    // (λx. λy. x) A → λy. A
    repl.execute(
        "Definition redex : Syntax := SApp (SLam (SSort UProp) (SLam (SSort UProp) (SVar 1))) (SSort (UType 42)).",
    )
    .expect("Define");
    repl.execute("Definition result : Syntax := syn_step redex.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "((SLam (SSort UProp)) (SSort (UType 42)))");
}

// =============================================================================
// SYN_STEP: NESTED APPLICATIONS
// =============================================================================

#[test]
fn test_step_nested_app_left() {
    let mut repl = Repl::new();

    // ((λx.x) A) B → A B
    // Step the left side first
    repl.execute(
        "Definition term : Syntax := SApp (SApp (SLam (SSort UProp) (SVar 0)) (SSort (UType 1))) (SSort (UType 2)).",
    )
    .expect("Define");
    repl.execute("Definition result : Syntax := syn_step term.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // After one step: (A B) where A = SSort (UType 1)
    assert_eq!(result, "((SApp (SSort (UType 1))) (SSort (UType 2)))");
}

// =============================================================================
// SYN_STEP: VALUES (NO REDUCTION)
// =============================================================================

#[test]
fn test_step_var_stuck() {
    let mut repl = Repl::new();

    // SVar 0 is stuck (no redex)
    repl.execute("Definition term : Syntax := SVar 0.")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_step term.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SVar 0)");
}

#[test]
fn test_step_lambda_value() {
    let mut repl = Repl::new();

    // A lambda by itself is a value (no head redex)
    repl.execute("Definition term : Syntax := SLam (SSort UProp) (SVar 0).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_step term.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "((SLam (SSort UProp)) (SVar 0))");
}

#[test]
fn test_step_sort_value() {
    let mut repl = Repl::new();

    // A sort is a value
    repl.execute("Definition term : Syntax := SSort UProp.")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_step term.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SSort UProp)");
}

#[test]
fn test_step_global_stuck() {
    let mut repl = Repl::new();

    // SGlobal is stuck (would need delta reduction)
    repl.execute("Definition term : Syntax := SGlobal 42.")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_step term.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SGlobal 42)");
}

// =============================================================================
// SYN_STEP: APPLICATION WITH NON-LAMBDA FUNCTION
// =============================================================================

#[test]
fn test_step_app_var_stuck() {
    let mut repl = Repl::new();

    // (x y) where x is a variable - stuck
    repl.execute("Definition term : Syntax := SApp (SVar 0) (SVar 1).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_step term.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // No redex found, returns unchanged
    assert_eq!(result, "((SApp (SVar 0)) (SVar 1))");
}

#[test]
fn test_step_app_global_stuck() {
    let mut repl = Repl::new();

    // (f x) where f is a global - stuck (would need delta)
    repl.execute("Definition term : Syntax := SApp (SGlobal 0) (SVar 0).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_step term.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "((SApp (SGlobal 0)) (SVar 0))");
}

// =============================================================================
// SYN_STEP: PI TYPES (NO HEAD REDUCTION)
// =============================================================================

#[test]
fn test_step_pi_value() {
    let mut repl = Repl::new();

    // Pi types are values (we don't reduce under binders)
    repl.execute("Definition term : Syntax := SPi (SSort UProp) (SVar 0).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_step term.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "((SPi (SSort UProp)) (SVar 0))");
}

// =============================================================================
// SYN_STEP: COMPLEX REDUCTION CHAINS
// =============================================================================

#[test]
fn test_step_church_numeral() {
    let mut repl = Repl::new();

    // Church 1 = λf. λx. f x
    // (Church 1) g → λx. g x
    // We apply the outer lambda to g
    repl.execute(
        "Definition church1 : Syntax := SLam (SSort UProp) (SLam (SSort UProp) (SApp (SVar 1) (SVar 0))).",
    )
    .expect("Define");
    repl.execute("Definition g : Syntax := SVar 42.")
        .expect("Define");
    repl.execute("Definition app : Syntax := SApp church1 g.")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_step app.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // After beta: λx. (g x) where g is now SVar 43 (lifted)
    // Body is SLam T (SApp (SVar 1) (SVar 0))
    // We substitute SVar 42 for index 0 in body
    // In the inner lambda, we substitute at index 1
    // SVar 1 matches! Gets replaced by lift(SVar 42) = SVar 43
    assert_eq!(
        result,
        "((SLam (SSort UProp)) ((SApp (SVar 43)) (SVar 0)))"
    );
}

// =============================================================================
// ERROR CASES
// =============================================================================

#[test]
fn test_beta_type_error() {
    let mut repl = Repl::new();

    // syn_beta expects Syntax for body, not Int
    let result = repl.execute("Check (syn_beta 42 (SVar 0)).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

#[test]
fn test_step_type_error() {
    let mut repl = Repl::new();

    // syn_step expects Syntax, not Int
    let result = repl.execute("Check (syn_step 42).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}
