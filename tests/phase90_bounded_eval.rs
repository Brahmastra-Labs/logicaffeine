//! Phase 90: The Clock (Bounded Evaluation)
//!
//! Implements bounded multi-step reduction on embedded syntax:
//! - syn_eval: Evaluate up to N steps

use logos::interface::Repl;

// =============================================================================
// SYN_EVAL: TYPE CHECK
// =============================================================================

#[test]
fn test_syn_eval_type() {
    let mut repl = Repl::new();

    // syn_eval : Int -> Syntax -> Syntax
    let result = repl.execute("Check syn_eval.").expect("Check syn_eval");
    assert_eq!(result, "syn_eval : Int -> Syntax -> Syntax");
}

// =============================================================================
// SYN_EVAL: ZERO FUEL
// =============================================================================

#[test]
fn test_eval_zero_fuel() {
    let mut repl = Repl::new();

    // With 0 fuel, no reduction happens
    repl.execute(
        "Definition redex : Syntax := SApp (SLam (SSort UProp) (SVar 0)) (SSort (UType 42)).",
    )
    .expect("Define");
    repl.execute("Definition result : Syntax := syn_eval 0 redex.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // Should be unchanged - no fuel
    assert_eq!(
        result,
        "((SApp ((SLam (SSort UProp)) (SVar 0))) (SSort (UType 42)))"
    );
}

#[test]
fn test_eval_negative_fuel() {
    let mut repl = Repl::new();

    // With negative fuel, no reduction happens
    repl.execute(
        "Definition redex : Syntax := SApp (SLam (SSort UProp) (SVar 0)) (SSort (UType 42)).",
    )
    .expect("Define");
    repl.execute("Definition result : Syntax := syn_eval (sub 0 5) redex.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // Should be unchanged - negative fuel treated as zero
    assert_eq!(
        result,
        "((SApp ((SLam (SSort UProp)) (SVar 0))) (SSort (UType 42)))"
    );
}

// =============================================================================
// SYN_EVAL: SINGLE STEP
// =============================================================================

#[test]
fn test_eval_single_step() {
    let mut repl = Repl::new();

    // With 1 fuel, one reduction
    repl.execute(
        "Definition redex : Syntax := SApp (SLam (SSort UProp) (SVar 0)) (SSort (UType 42)).",
    )
    .expect("Define");
    repl.execute("Definition result : Syntax := syn_eval 1 redex.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SSort (UType 42))");
}

// =============================================================================
// SYN_EVAL: MULTI STEP
// =============================================================================

#[test]
fn test_eval_multi_step() {
    let mut repl = Repl::new();

    // ((λx. λy. x) A) B → (λy. A) B → A
    // Requires 2 steps
    repl.execute(
        "Definition term : Syntax := SApp (SApp (SLam (SSort UProp) (SLam (SSort UProp) (SVar 1))) (SSort (UType 1))) (SSort (UType 2)).",
    )
    .expect("Define");
    repl.execute("Definition result : Syntax := syn_eval 10 term.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // After 2 steps: SSort (UType 1)
    assert_eq!(result, "(SSort (UType 1))");
}

#[test]
fn test_eval_partial_fuel() {
    let mut repl = Repl::new();

    // ((λx. λy. x) A) B requires 2 steps
    // With only 1 fuel, we get partial reduction
    repl.execute(
        "Definition term : Syntax := SApp (SApp (SLam (SSort UProp) (SLam (SSort UProp) (SVar 1))) (SSort (UType 1))) (SSort (UType 2)).",
    )
    .expect("Define");
    repl.execute("Definition result : Syntax := syn_eval 1 term.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // After 1 step: (λy. A) B
    assert_eq!(
        result,
        "((SApp ((SLam (SSort UProp)) (SSort (UType 1)))) (SSort (UType 2)))"
    );
}

// =============================================================================
// SYN_EVAL: ALREADY NORMAL
// =============================================================================

#[test]
fn test_eval_already_normal_sort() {
    let mut repl = Repl::new();

    // SSort is already in normal form
    repl.execute("Definition term : Syntax := SSort UProp.")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_eval 100 term.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SSort UProp)");
}

#[test]
fn test_eval_already_normal_var() {
    let mut repl = Repl::new();

    // SVar is stuck (normal form for head reduction)
    repl.execute("Definition term : Syntax := SVar 42.")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_eval 100 term.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SVar 42)");
}

#[test]
fn test_eval_already_normal_lambda() {
    let mut repl = Repl::new();

    // Lambda is a value
    repl.execute("Definition term : Syntax := SLam (SSort UProp) (SVar 0).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_eval 100 term.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "((SLam (SSort UProp)) (SVar 0))");
}

// =============================================================================
// SYN_EVAL: STUCK TERMS
// =============================================================================

#[test]
fn test_eval_stuck_app() {
    let mut repl = Repl::new();

    // (x y) is stuck - x is a variable, not a lambda
    repl.execute("Definition term : Syntax := SApp (SVar 0) (SVar 1).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_eval 100 term.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "((SApp (SVar 0)) (SVar 1))");
}

#[test]
fn test_eval_stuck_global() {
    let mut repl = Repl::new();

    // SGlobal is stuck (would need delta reduction)
    repl.execute("Definition term : Syntax := SApp (SGlobal 0) (SVar 0).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_eval 100 term.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "((SApp (SGlobal 0)) (SVar 0))");
}

// =============================================================================
// SYN_EVAL: CHURCH NUMERALS
// =============================================================================

#[test]
fn test_eval_church_application() {
    let mut repl = Repl::new();

    // Church 1 = λf. λx. f x
    // Apply Church 1 to a function g, then to an argument a
    // Should reduce to (g a)
    repl.execute(
        "Definition church1 : Syntax := SLam (SSort UProp) (SLam (SSort UProp) (SApp (SVar 1) (SVar 0))).",
    )
    .expect("Define");
    repl.execute("Definition g : Syntax := SVar 10.").expect("Define");
    repl.execute("Definition a : Syntax := SVar 20.").expect("Define");
    repl.execute("Definition app : Syntax := SApp (SApp church1 g) a.")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_eval 100 app.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // After full evaluation: (g a) = SApp (SVar 11) (SVar 20)
    // g gets lifted when entering lambdas
    assert_eq!(result, "((SApp (SVar 11)) (SVar 20))");
}

// =============================================================================
// SYN_EVAL: SUFFICIENT FUEL
// =============================================================================

#[test]
fn test_eval_sufficient_fuel() {
    let mut repl = Repl::new();

    // 100 fuel is enough for a simple identity application
    repl.execute(
        "Definition redex : Syntax := SApp (SLam (SSort UProp) (SVar 0)) (SSort UProp).",
    )
    .expect("Define");
    repl.execute("Definition result : Syntax := syn_eval 100 redex.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SSort UProp)");
}

// =============================================================================
// SYN_EVAL: EARLY TERMINATION
// =============================================================================

#[test]
fn test_eval_stops_at_normal_form() {
    let mut repl = Repl::new();

    // Even with 1000 fuel, should stop after 1 step when normal form reached
    repl.execute(
        "Definition redex : Syntax := SApp (SLam (SSort UProp) (SVar 0)) (SSort (UType 99)).",
    )
    .expect("Define");
    repl.execute("Definition result : Syntax := syn_eval 1000 redex.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // 1 step to normal form
    assert_eq!(result, "(SSort (UType 99))");
}

// =============================================================================
// ERROR CASES
// =============================================================================

#[test]
fn test_eval_type_error_fuel() {
    let mut repl = Repl::new();

    // syn_eval expects Int for fuel, not Syntax
    let result = repl.execute("Check (syn_eval (SVar 0) (SVar 0)).");
    assert!(result.is_err(), "Should reject Syntax where Int expected");
}

#[test]
fn test_eval_type_error_term() {
    let mut repl = Repl::new();

    // syn_eval expects Syntax for term, not Int
    let result = repl.execute("Check (syn_eval 100 42).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}
