//! Phase 91: The Quote (Reification)
//!
//! Implements quoting for deep embedding:
//! - SLit: Integer literals in Syntax
//! - SName: Named global references
//! - syn_quote: Convert Syntax value to its construction code

use logos::interface::Repl;

// =============================================================================
// NEW CONSTRUCTORS: TYPE CHECK
// =============================================================================

#[test]
fn test_slit_type() {
    let mut repl = Repl::new();

    // SLit : Int -> Syntax
    let result = repl.execute("Check SLit.").expect("Check SLit");
    assert_eq!(result, "SLit : Int -> Syntax");
}

#[test]
fn test_sname_type() {
    let mut repl = Repl::new();

    // SName : Text -> Syntax
    let result = repl.execute("Check SName.").expect("Check SName");
    assert_eq!(result, "SName : Text -> Syntax");
}

#[test]
fn test_slit_construction() {
    let mut repl = Repl::new();

    // Can construct SLit with an integer
    repl.execute("Definition lit42 : Syntax := SLit 42.")
        .expect("Define");
    let result = repl.execute("Check lit42.").expect("Check");
    assert_eq!(result, "lit42 : Syntax");
}

#[test]
fn test_sname_construction() {
    let mut repl = Repl::new();

    // Can construct SName with a text literal
    repl.execute("Definition name_svar : Syntax := SName \"SVar\".")
        .expect("Define");
    let result = repl.execute("Check name_svar.").expect("Check");
    assert_eq!(result, "name_svar : Syntax");
}

// =============================================================================
// SYN_QUOTE: TYPE CHECK
// =============================================================================

#[test]
fn test_syn_quote_type() {
    let mut repl = Repl::new();

    // syn_quote : Syntax -> Syntax
    let result = repl.execute("Check syn_quote.").expect("Check syn_quote");
    assert_eq!(result, "syn_quote : Syntax -> Syntax");
}

// =============================================================================
// SYN_QUOTE: BASIC CONSTRUCTORS
// =============================================================================

#[test]
fn test_quote_svar() {
    let mut repl = Repl::new();

    // syn_quote (SVar 0) = SApp (SName "SVar") (SLit 0)
    repl.execute("Definition term : Syntax := SVar 0.")
        .expect("Define");
    repl.execute("Definition quoted : Syntax := syn_quote term.")
        .expect("Define");

    let result = repl.execute("Eval quoted.").expect("Eval");
    assert_eq!(result, "((SApp (SName \"SVar\")) (SLit 0))");
}

#[test]
fn test_quote_svar_nonzero() {
    let mut repl = Repl::new();

    // syn_quote (SVar 42) = SApp (SName "SVar") (SLit 42)
    repl.execute("Definition term : Syntax := SVar 42.")
        .expect("Define");
    repl.execute("Definition quoted : Syntax := syn_quote term.")
        .expect("Define");

    let result = repl.execute("Eval quoted.").expect("Eval");
    assert_eq!(result, "((SApp (SName \"SVar\")) (SLit 42))");
}

#[test]
fn test_quote_sglobal() {
    let mut repl = Repl::new();

    // syn_quote (SGlobal 5) = SApp (SName "SGlobal") (SLit 5)
    repl.execute("Definition term : Syntax := SGlobal 5.")
        .expect("Define");
    repl.execute("Definition quoted : Syntax := syn_quote term.")
        .expect("Define");

    let result = repl.execute("Eval quoted.").expect("Eval");
    assert_eq!(result, "((SApp (SName \"SGlobal\")) (SLit 5))");
}

#[test]
fn test_quote_ssort_uprop() {
    let mut repl = Repl::new();

    // syn_quote (SSort UProp) = SApp (SName "SSort") (SName "UProp")
    repl.execute("Definition term : Syntax := SSort UProp.")
        .expect("Define");
    repl.execute("Definition quoted : Syntax := syn_quote term.")
        .expect("Define");

    let result = repl.execute("Eval quoted.").expect("Eval");
    assert_eq!(result, "((SApp (SName \"SSort\")) (SName \"UProp\"))");
}

#[test]
fn test_quote_ssort_utype() {
    let mut repl = Repl::new();

    // syn_quote (SSort (UType 3)) = SApp (SName "SSort") (SApp (SName "UType") (SLit 3))
    repl.execute("Definition term : Syntax := SSort (UType 3).")
        .expect("Define");
    repl.execute("Definition quoted : Syntax := syn_quote term.")
        .expect("Define");

    let result = repl.execute("Eval quoted.").expect("Eval");
    assert_eq!(
        result,
        "((SApp (SName \"SSort\")) ((SApp (SName \"UType\")) (SLit 3)))"
    );
}

// =============================================================================
// SYN_QUOTE: COMPOUND TERMS
// =============================================================================

#[test]
fn test_quote_sapp() {
    let mut repl = Repl::new();

    // syn_quote (SApp (SVar 1) (SVar 0))
    // = SApp (SApp (SName "SApp") (quote (SVar 1))) (quote (SVar 0))
    repl.execute("Definition term : Syntax := SApp (SVar 1) (SVar 0).")
        .expect("Define");
    repl.execute("Definition quoted : Syntax := syn_quote term.")
        .expect("Define");

    let result = repl.execute("Eval quoted.").expect("Eval");
    assert_eq!(
        result,
        "((SApp ((SApp (SName \"SApp\")) ((SApp (SName \"SVar\")) (SLit 1)))) ((SApp (SName \"SVar\")) (SLit 0)))"
    );
}

#[test]
fn test_quote_slam() {
    let mut repl = Repl::new();

    // syn_quote (SLam (SSort UProp) (SVar 0))
    repl.execute("Definition term : Syntax := SLam (SSort UProp) (SVar 0).")
        .expect("Define");
    repl.execute("Definition quoted : Syntax := syn_quote term.")
        .expect("Define");

    let result = repl.execute("Eval quoted.").expect("Eval");
    assert_eq!(
        result,
        "((SApp ((SApp (SName \"SLam\")) ((SApp (SName \"SSort\")) (SName \"UProp\")))) ((SApp (SName \"SVar\")) (SLit 0)))"
    );
}

#[test]
fn test_quote_spi() {
    let mut repl = Repl::new();

    // syn_quote (SPi (SSort UProp) (SSort UProp))
    repl.execute("Definition term : Syntax := SPi (SSort UProp) (SSort UProp).")
        .expect("Define");
    repl.execute("Definition quoted : Syntax := syn_quote term.")
        .expect("Define");

    let result = repl.execute("Eval quoted.").expect("Eval");
    assert_eq!(
        result,
        "((SApp ((SApp (SName \"SPi\")) ((SApp (SName \"SSort\")) (SName \"UProp\")))) ((SApp (SName \"SSort\")) (SName \"UProp\")))"
    );
}

// =============================================================================
// SYN_QUOTE: NEW CONSTRUCTORS (SELF-QUOTING)
// =============================================================================

#[test]
fn test_quote_slit() {
    let mut repl = Repl::new();

    // syn_quote (SLit 99) = SApp (SName "SLit") (SLit 99)
    repl.execute("Definition term : Syntax := SLit 99.")
        .expect("Define");
    repl.execute("Definition quoted : Syntax := syn_quote term.")
        .expect("Define");

    let result = repl.execute("Eval quoted.").expect("Eval");
    assert_eq!(result, "((SApp (SName \"SLit\")) (SLit 99))");
}

#[test]
fn test_quote_sname() {
    let mut repl = Repl::new();

    // syn_quote (SName "foo") = SName "foo"  (self-quoting)
    repl.execute("Definition term : Syntax := SName \"foo\".")
        .expect("Define");
    repl.execute("Definition quoted : Syntax := syn_quote term.")
        .expect("Define");

    let result = repl.execute("Eval quoted.").expect("Eval");
    assert_eq!(result, "(SName \"foo\")");
}

// =============================================================================
// SYN_QUOTE: SIZE AFTER QUOTING
// =============================================================================

#[test]
fn test_quote_increases_size() {
    let mut repl = Repl::new();

    // Original: SVar 0 (size 1)
    // Quoted: SApp (SName "SVar") (SLit 0) (size 3)
    repl.execute("Definition term : Syntax := SVar 0.")
        .expect("Define");
    repl.execute("Definition original_size : Int := syn_size term.")
        .expect("Define");
    repl.execute("Definition quoted : Syntax := syn_quote term.")
        .expect("Define");
    repl.execute("Definition quoted_size : Int := syn_size quoted.")
        .expect("Define");

    let orig = repl.execute("Eval original_size.").expect("Eval");
    let quot = repl.execute("Eval quoted_size.").expect("Eval");

    assert_eq!(orig, "1");
    assert_eq!(quot, "3");
}

// =============================================================================
// DIAGONAL FUNCTION FOUNDATION
// =============================================================================

#[test]
fn test_diag_simple() {
    let mut repl = Repl::new();

    // syn_diag x := syn_subst (syn_quote x) 0 x
    // For x = SVar 0:
    //   syn_quote (SVar 0) = SApp (SName "SVar") (SLit 0)
    //   syn_subst that for var 0 in (SVar 0)
    //   = SApp (SName "SVar") (SLit 0)
    repl.execute("Definition x : Syntax := SVar 0.").expect("Define");
    repl.execute("Definition quoted_x : Syntax := syn_quote x.")
        .expect("Define");
    repl.execute("Definition diag_x : Syntax := syn_subst quoted_x 0 x.")
        .expect("Define");

    let result = repl.execute("Eval diag_x.").expect("Eval");
    // The variable gets replaced with the quoted form
    assert_eq!(result, "((SApp (SName \"SVar\")) (SLit 0))");
}

#[test]
fn test_diag_nested() {
    let mut repl = Repl::new();

    // For x = SApp (SVar 0) (SVar 0):
    //   Both SVar 0s get replaced with the quoted form of x
    repl.execute("Definition x : Syntax := SApp (SVar 0) (SVar 0).")
        .expect("Define");
    repl.execute("Definition quoted_x : Syntax := syn_quote x.")
        .expect("Define");
    repl.execute("Definition diag_x : Syntax := syn_subst quoted_x 0 x.")
        .expect("Define");

    let result = repl.execute("Eval diag_x.").expect("Eval");
    // Both occurrences of SVar 0 become syn_quote(SApp (SVar 0) (SVar 0))
    // This is the essence of self-reference!
    assert!(result.contains("SApp"));
    assert!(result.contains("SName"));
}

// =============================================================================
// ERROR CASES
// =============================================================================

#[test]
fn test_quote_type_error() {
    let mut repl = Repl::new();

    // syn_quote expects Syntax, not Int
    let result = repl.execute("Check (syn_quote 42).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

#[test]
fn test_slit_type_error() {
    let mut repl = Repl::new();

    // SLit expects Int, not Syntax
    let result = repl.execute("Check (SLit (SVar 0)).");
    assert!(result.is_err(), "Should reject Syntax where Int expected");
}

#[test]
fn test_sname_type_error() {
    let mut repl = Repl::new();

    // SName expects Text, not Int
    let result = repl.execute("Check (SName 42).");
    assert!(result.is_err(), "Should reject Int where Text expected");
}
