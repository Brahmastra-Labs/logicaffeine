//! Phase 93: The Diagonal Lemma (Self-Reference)
//!
//! Implements the diagonal function for self-referential constructions:
//! - syn_diag: The diagonal function (syn_subst (syn_quote x) 0 x)
//! - Fixed point construction for any predicate P

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// SYN_DIAG: TYPE CHECK
// =============================================================================

#[test]
fn test_syn_diag_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check syn_diag.").expect("Check syn_diag");
    assert_eq!(result, "syn_diag : Syntax -> Syntax");
}

// =============================================================================
// SYN_DIAG: BASIC COMPUTATION
// =============================================================================

#[test]
fn test_syn_diag_var() {
    let mut repl = Repl::new();

    // syn_diag (SVar 0) = syn_quote (SVar 0) = SApp (SName "SVar") (SLit 0)
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_diag x.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "((SApp (SName \"SVar\")) (SLit 0))");
}

#[test]
fn test_syn_diag_matches_manual() {
    let mut repl = Repl::new();

    // syn_diag x should equal syn_subst (syn_quote x) 0 x
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define");
    repl.execute("Definition diag_builtin : Syntax := syn_diag x.")
        .expect("Define");
    repl.execute("Definition diag_manual : Syntax := syn_subst (syn_quote x) 0 x.")
        .expect("Define");

    let builtin = repl.execute("Eval diag_builtin.").expect("Eval");
    let manual = repl.execute("Eval diag_manual.").expect("Eval");
    assert_eq!(builtin, manual);
}

#[test]
fn test_syn_diag_app_self() {
    let mut repl = Repl::new();

    // syn_diag (SApp (SVar 0) (SVar 0))
    // Both SVar 0s get replaced with the quoted form
    repl.execute("Definition x : Syntax := SApp (SVar 0) (SVar 0).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_diag x.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // The result contains the quoted form twice (once for each SVar 0)
    assert!(result.contains("SApp"));
    assert!(result.contains("SName"));
}

#[test]
fn test_syn_diag_closed_term() {
    let mut repl = Repl::new();

    // syn_diag on a closed term (no SVar 0) returns the term unchanged
    repl.execute("Definition x : Syntax := SSort UProp.")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_diag x.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SSort UProp)");
}

#[test]
fn test_syn_diag_different_var() {
    let mut repl = Repl::new();

    // syn_diag only substitutes for var 0, not other vars
    repl.execute("Definition x : Syntax := SVar 1.")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_diag x.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SVar 1)");
}

// =============================================================================
// FIXED POINT CONSTRUCTION
// =============================================================================

#[test]
fn test_fixed_point_simple_predicate() {
    let mut repl = Repl::new();

    // Let P = SName "Provable" (a predicate on syntax)
    // Template T = SApp P (SVar 0) meaning "Provable(x)"
    // G = syn_diag T = SApp P (syn_quote T) meaning "Provable(⌜T⌝)"
    repl.execute("Definition P : Syntax := SName \"Provable\".")
        .expect("Define");
    repl.execute("Definition T : Syntax := SApp P (SVar 0).")
        .expect("Define");
    repl.execute("Definition G : Syntax := syn_diag T.")
        .expect("Define");

    let result = repl.execute("Eval G.").expect("Eval");
    // G should be SApp P (syn_quote T) = SApp (SName "Provable") (quoted form of T)
    assert!(result.contains("SName \"Provable\""));
    // The quoted form should contain the original structure
    assert!(result.contains("SApp"));
}

#[test]
fn test_fixed_point_godel_sentence() {
    let mut repl = Repl::new();

    // The Gödel sentence: "I am not provable"
    // Let Neg be negation: Neg P = SApp (SName "Not") P
    // Let Prov be provability: Prov x = SApp (SName "Provable") x
    // Template T = SApp (SName "Not") (SApp (SName "Provable") (SVar 0))
    // G = syn_diag T says "Not(Provable(⌜T⌝))"

    repl.execute(
        "Definition T : Syntax := SApp (SName \"Not\") (SApp (SName \"Provable\") (SVar 0)).",
    )
    .expect("Define");
    repl.execute("Definition G : Syntax := syn_diag T.")
        .expect("Define");

    let result = repl.execute("Eval G.").expect("Eval");
    // G should contain Not and Provable
    assert!(result.contains("SName \"Not\""));
    assert!(result.contains("SName \"Provable\""));
}

#[test]
fn test_fixed_point_equivalence() {
    let mut repl = Repl::new();

    // Verify G = SApp P (syn_quote T) when T = SApp P (SVar 0)
    // This is the core of the diagonal lemma
    repl.execute("Definition P : Syntax := SName \"P\".")
        .expect("Define");
    repl.execute("Definition T : Syntax := SApp P (SVar 0).")
        .expect("Define");
    repl.execute("Definition G : Syntax := syn_diag T.")
        .expect("Define");
    repl.execute("Definition expected : Syntax := SApp P (syn_quote T).")
        .expect("Define");

    let g = repl.execute("Eval G.").expect("Eval G");
    let exp = repl.execute("Eval expected.").expect("Eval expected");
    assert_eq!(g, exp);
}

// =============================================================================
// COMPOSITIONALITY
// =============================================================================

#[test]
fn test_syn_diag_under_lambda() {
    let mut repl = Repl::new();

    // Diagonalizing a term with a binder
    // SLam T (SVar 1) - the SVar 1 is free (refers to outside the lambda)
    // After diag, SVar 1 becomes SVar 0 which gets substituted
    repl.execute("Definition x : Syntax := SLam (SSort UProp) (SVar 1).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_diag x.")
        .expect("Define");

    // The result should have the lambda structure but with var 1 becoming the quoted form
    let result = repl.execute("Eval result.").expect("Eval");
    assert!(result.contains("SLam"));
}

#[test]
fn test_syn_diag_nested_app() {
    let mut repl = Repl::new();

    // Complex nested application with multiple occurrences of SVar 0
    repl.execute("Definition x : Syntax := SApp (SApp (SVar 0) (SVar 0)) (SVar 0).")
        .expect("Define");
    repl.execute("Definition result : Syntax := syn_diag x.")
        .expect("Define");

    let result = repl.execute("Eval result.").expect("Eval");
    // All three SVar 0 occurrences should be replaced
    // Result should be deeply nested with quoted forms
    assert!(result.contains("SApp"));
    assert!(!result.contains("(SVar 0)"));
}

// =============================================================================
// IDEMPOTENCE AND PROPERTIES
// =============================================================================

#[test]
fn test_syn_diag_idempotent_on_closed() {
    let mut repl = Repl::new();

    // For closed terms, syn_diag is idempotent (applying twice gives same result)
    repl.execute("Definition x : Syntax := SName \"constant\".")
        .expect("Define");
    repl.execute("Definition once : Syntax := syn_diag x.")
        .expect("Define");
    repl.execute("Definition twice : Syntax := syn_diag once.")
        .expect("Define");

    let once_result = repl.execute("Eval once.").expect("Eval once");
    let twice_result = repl.execute("Eval twice.").expect("Eval twice");
    assert_eq!(once_result, twice_result);
}

#[test]
fn test_syn_diag_preserves_size_growth() {
    let mut repl = Repl::new();

    // Diagonalization typically increases term size
    repl.execute("Definition x : Syntax := SApp (SVar 0) (SVar 0).")
        .expect("Define");
    repl.execute("Definition original_size : Int := syn_size x.")
        .expect("Define");
    repl.execute("Definition diag_x : Syntax := syn_diag x.")
        .expect("Define");
    repl.execute("Definition diag_size : Int := syn_size diag_x.")
        .expect("Define");

    let orig = repl.execute("Eval original_size.").expect("Eval orig");
    let diag = repl.execute("Eval diag_size.").expect("Eval diag");

    let orig_n: i64 = orig.parse().unwrap();
    let diag_n: i64 = diag.parse().unwrap();
    assert!(diag_n > orig_n, "Diagonalization should increase size");
}

// =============================================================================
// ERROR CASES
// =============================================================================

#[test]
fn test_syn_diag_type_error() {
    let mut repl = Repl::new();

    // syn_diag expects Syntax, not Int
    let result = repl.execute("Check (syn_diag 42).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

// =============================================================================
// QUINE CONSTRUCTION
// =============================================================================

#[test]
fn test_quine_foundation() {
    let mut repl = Repl::new();

    // A quine is a term Q such that syn_eval Q = syn_quote Q
    // Using diag: if T = syn_quote (SVar 0), then syn_diag T gives us
    // syn_subst (syn_quote T) 0 T = syn_subst (syn_quote (syn_quote (SVar 0))) 0 (syn_quote (SVar 0))
    // This is the foundation for quine construction

    repl.execute("Definition inner : Syntax := SVar 0.")
        .expect("Define");
    repl.execute("Definition T : Syntax := syn_quote inner.")
        .expect("Define");
    repl.execute("Definition Q : Syntax := syn_diag T.")
        .expect("Define");

    // Q should be a valid Syntax term
    let result = repl.execute("Check Q.").expect("Check Q");
    assert_eq!(result, "Q : Syntax");
}
