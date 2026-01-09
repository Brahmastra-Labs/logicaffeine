//! Phase 87: The Mirror (Deep Embedding)
//!
//! Teaches the Kernel to represent its own syntax.
//! - Univ: Universe representation (Prop, Type n)
//! - Syntax: Term representation with De Bruijn indices
//! - Verified operations over syntax trees

use logos::interface::Repl;

// =============================================================================
// UNIVERSE TYPE
// =============================================================================

#[test]
fn test_univ_type() {
    let mut repl = Repl::new();

    // Univ : Type 0
    let result = repl.execute("Check Univ.").expect("Check Univ");
    assert_eq!(result, "Univ : Type0");
}

#[test]
fn test_uprop_constructor() {
    let mut repl = Repl::new();

    // UProp : Univ
    let result = repl.execute("Check UProp.").expect("Check UProp");
    assert_eq!(result, "UProp : Univ");
}

#[test]
fn test_utype_constructor() {
    let mut repl = Repl::new();

    // UType : Int -> Univ
    let result = repl.execute("Check UType.").expect("Check UType");
    assert_eq!(result, "UType : Int -> Univ");
}

#[test]
fn test_utype_application() {
    let mut repl = Repl::new();

    // UType 0 : Univ (Type at level 0)
    let result = repl.execute("Check (UType 0).").expect("Check UType 0");
    assert_eq!(result, "(UType 0) : Univ");
}

// =============================================================================
// SYNTAX TYPE
// =============================================================================

#[test]
fn test_syntax_type() {
    let mut repl = Repl::new();

    // Syntax : Type 0
    let result = repl.execute("Check Syntax.").expect("Check Syntax");
    assert_eq!(result, "Syntax : Type0");
}

#[test]
fn test_svar_constructor() {
    let mut repl = Repl::new();

    // SVar : Int -> Syntax (De Bruijn index)
    let result = repl.execute("Check SVar.").expect("Check SVar");
    assert_eq!(result, "SVar : Int -> Syntax");
}

#[test]
fn test_sglobal_constructor() {
    let mut repl = Repl::new();

    // SGlobal : Int -> Syntax (global reference by ID)
    let result = repl.execute("Check SGlobal.").expect("Check SGlobal");
    assert_eq!(result, "SGlobal : Int -> Syntax");
}

#[test]
fn test_ssort_constructor() {
    let mut repl = Repl::new();

    // SSort : Univ -> Syntax
    let result = repl.execute("Check SSort.").expect("Check SSort");
    assert_eq!(result, "SSort : Univ -> Syntax");
}

#[test]
fn test_sapp_constructor() {
    let mut repl = Repl::new();

    // SApp : Syntax -> Syntax -> Syntax
    let result = repl.execute("Check SApp.").expect("Check SApp");
    assert_eq!(result, "SApp : Syntax -> Syntax -> Syntax");
}

#[test]
fn test_slam_constructor() {
    let mut repl = Repl::new();

    // SLam : Syntax -> Syntax -> Syntax (param_type, body)
    let result = repl.execute("Check SLam.").expect("Check SLam");
    assert_eq!(result, "SLam : Syntax -> Syntax -> Syntax");
}

#[test]
fn test_spi_constructor() {
    let mut repl = Repl::new();

    // SPi : Syntax -> Syntax -> Syntax (param_type, body_type)
    let result = repl.execute("Check SPi.").expect("Check SPi");
    assert_eq!(result, "SPi : Syntax -> Syntax -> Syntax");
}

// =============================================================================
// BUILDING SYNTAX TERMS
// =============================================================================

#[test]
fn test_build_var() {
    let mut repl = Repl::new();

    // Build SVar 0 (innermost bound variable)
    repl.execute("Definition var0 : Syntax := SVar 0.")
        .expect("Define var0");

    let result = repl.execute("Check var0.").expect("Check var0");
    assert_eq!(result, "var0 : Syntax");
}

#[test]
fn test_build_identity() {
    let mut repl = Repl::new();

    // Build λ(x:Type0). x  as:  SLam (SSort (UType 0)) (SVar 0)
    repl.execute("Definition syn_id : Syntax := SLam (SSort (UType 0)) (SVar 0).")
        .expect("Define syn_id");

    let result = repl.execute("Check syn_id.").expect("Check syn_id");
    assert_eq!(result, "syn_id : Syntax");
}

#[test]
fn test_build_application() {
    let mut repl = Repl::new();

    // Build f x as: SApp (SVar 1) (SVar 0)
    repl.execute("Definition syn_app : Syntax := SApp (SVar 1) (SVar 0).")
        .expect("Define syn_app");

    let result = repl.execute("Check syn_app.").expect("Check syn_app");
    assert_eq!(result, "syn_app : Syntax");
}

#[test]
fn test_build_pi_type() {
    let mut repl = Repl::new();

    // Build Π(x:Prop). Prop as: SPi (SSort UProp) (SSort UProp)
    repl.execute("Definition syn_prop_to_prop : Syntax := SPi (SSort UProp) (SSort UProp).")
        .expect("Define syn_prop_to_prop");

    let result = repl.execute("Check syn_prop_to_prop.").expect("Check");
    assert_eq!(result, "syn_prop_to_prop : Syntax");
}

// =============================================================================
// SIZE FUNCTION
// =============================================================================

#[test]
fn test_size_var() {
    let mut repl = Repl::new();

    // Size of a variable is 1
    repl.execute("Definition test_var : Syntax := SVar 0.")
        .expect("Define");
    repl.execute("Definition var_size : Int := syn_size test_var.")
        .expect("Define");

    let result = repl.execute("Eval var_size.").expect("Eval");
    assert_eq!(result, "1");
}

#[test]
fn test_size_sort() {
    let mut repl = Repl::new();

    // Size of a sort is 1
    repl.execute("Definition test_sort : Syntax := SSort UProp.")
        .expect("Define");
    repl.execute("Definition sort_size : Int := syn_size test_sort.")
        .expect("Define");

    let result = repl.execute("Eval sort_size.").expect("Eval");
    assert_eq!(result, "1");
}

#[test]
fn test_size_app() {
    let mut repl = Repl::new();

    // Size of (f x) = 1 + size(f) + size(x) = 1 + 1 + 1 = 3
    repl.execute("Definition test_app : Syntax := SApp (SVar 0) (SVar 1).")
        .expect("Define");
    repl.execute("Definition app_size : Int := syn_size test_app.")
        .expect("Define");

    let result = repl.execute("Eval app_size.").expect("Eval");
    assert_eq!(result, "3");
}

#[test]
fn test_size_lambda() {
    let mut repl = Repl::new();

    // Size of λ(x:A).x = 1 + size(A) + size(body) = 1 + 1 + 1 = 3
    repl.execute("Definition test_lam : Syntax := SLam (SSort UProp) (SVar 0).")
        .expect("Define");
    repl.execute("Definition lam_size : Int := syn_size test_lam.")
        .expect("Define");

    let result = repl.execute("Eval lam_size.").expect("Eval");
    assert_eq!(result, "3");
}

#[test]
fn test_size_nested() {
    let mut repl = Repl::new();

    // λ(x:Prop). λ(y:Prop). x
    // SLam (SSort UProp) (SLam (SSort UProp) (SVar 1))
    // Size = 1 + 1 + (1 + 1 + 1) = 5
    repl.execute(
        "Definition nested : Syntax := SLam (SSort UProp) (SLam (SSort UProp) (SVar 1)).",
    )
    .expect("Define");
    repl.execute("Definition nested_size : Int := syn_size nested.")
        .expect("Define");

    let result = repl.execute("Eval nested_size.").expect("Eval");
    assert_eq!(result, "5");
}

// =============================================================================
// IS_CLOSED PREDICATE
// =============================================================================

#[test]
fn test_is_closed_true() {
    let mut repl = Repl::new();

    // λ(x:Prop). x is closed (SVar 0 is bound by the lambda)
    // Under 1 binder, SVar 0 is valid
    repl.execute("Definition closed_term : Syntax := SLam (SSort UProp) (SVar 0).")
        .expect("Define");
    repl.execute("Definition closed_result : Int := syn_max_var closed_term.")
        .expect("Define");

    // max_var returns the highest unbound var index, or -1 if closed
    // For λx.x, max_var = -1 (all vars bound)
    let result = repl.execute("Eval closed_result.").expect("Eval");
    // This checks the maximum free variable index; -1 means closed
    assert_eq!(result, "-1");
}

#[test]
fn test_is_closed_false() {
    let mut repl = Repl::new();

    // Just SVar 0 with no enclosing binder is open
    repl.execute("Definition open_term : Syntax := SVar 0.")
        .expect("Define");
    repl.execute("Definition open_result : Int := syn_max_var open_term.")
        .expect("Define");

    let result = repl.execute("Eval open_result.").expect("Eval");
    // SVar 0 is free, so max_var = 0
    assert_eq!(result, "0");
}

// =============================================================================
// ERROR HANDLING
// =============================================================================

#[test]
fn test_syntax_type_error() {
    let mut repl = Repl::new();

    // SVar expects Int, not Nat
    let result = repl.execute("Check (SVar Zero).");
    assert!(result.is_err(), "Should reject Nat where Int expected");
}
