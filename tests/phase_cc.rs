//! Phase CC: Congruence Closure Tactic
//!
//! Tests for the cc tactic which proves equalities over
//! uninterpreted functions using congruence closure.

use logos::interface::Repl;

// =============================================================================
// TYPE CHECKS
// =============================================================================

#[test]
fn test_dccsolve_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DccSolve.");
    assert!(result.is_ok(), "DccSolve should exist: {:?}", result);
}

#[test]
fn test_try_cc_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_cc.");
    assert!(result.is_ok(), "try_cc should exist: {:?}", result);
}

// =============================================================================
// REFLEXIVE EQUALITIES
// =============================================================================

#[test]
fn test_cc_reflexivity() {
    let mut repl = Repl::new();
    // f(x) = f(x) by reflexivity
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition fx : Syntax := SApp (SName \"f\") x.")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Eq\") fx) fx.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_cc goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "CC should prove f(x) = f(x)");
}

#[test]
fn test_cc_nested_functions() {
    let mut repl = Repl::new();
    // f(g(x)) = f(g(x))
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition gx : Syntax := SApp (SName \"g\") x.")
        .unwrap();
    repl.execute("Definition fgx : Syntax := SApp (SName \"f\") gx.")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Eq\") fgx) fgx.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_cc goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "CC should prove f(g(x)) = f(g(x))");
}

// =============================================================================
// CONGRUENCE WITH HYPOTHESES
// =============================================================================

#[test]
fn test_cc_congruence_from_hypothesis() {
    let mut repl = Repl::new();
    // Given x = y, prove f(x) = f(y)
    // Encoded as: (Eq x y) -> (Eq (f x) (f y))
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition y : Syntax := SVar 1.").unwrap();
    repl.execute("Definition fx : Syntax := SApp (SName \"f\") x.")
        .unwrap();
    repl.execute("Definition fy : Syntax := SApp (SName \"f\") y.")
        .unwrap();
    repl.execute("Definition hyp : Syntax := SApp (SApp (SName \"Eq\") x) y.")
        .unwrap();
    repl.execute("Definition concl : Syntax := SApp (SApp (SName \"Eq\") fx) fy.")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"implies\") hyp) concl.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_cc goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "CC should prove x=y -> f(x)=f(y)");
}

#[test]
fn test_cc_congruence_binary() {
    let mut repl = Repl::new();
    // Given a = b, prove add(a, c) = add(b, c)
    repl.execute("Definition a : Syntax := SVar 0.").unwrap();
    repl.execute("Definition b : Syntax := SVar 1.").unwrap();
    repl.execute("Definition c : Syntax := SVar 2.").unwrap();
    repl.execute("Definition add_ac : Syntax := SApp (SApp (SName \"add\") a) c.")
        .unwrap();
    repl.execute("Definition add_bc : Syntax := SApp (SApp (SName \"add\") b) c.")
        .unwrap();
    repl.execute("Definition hyp : Syntax := SApp (SApp (SName \"Eq\") a) b.")
        .unwrap();
    repl.execute("Definition concl : Syntax := SApp (SApp (SName \"Eq\") add_ac) add_bc.")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"implies\") hyp) concl.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_cc goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(
        concluded, original,
        "CC should prove a=b -> add(a,c)=add(b,c)"
    );
}

#[test]
fn test_cc_transitivity_chain() {
    let mut repl = Repl::new();
    // Given a = b and b = c, prove f(a) = f(c)
    // Encoded as: (Eq a b) -> (Eq b c) -> (Eq (f a) (f c))
    repl.execute("Definition a : Syntax := SVar 0.").unwrap();
    repl.execute("Definition b : Syntax := SVar 1.").unwrap();
    repl.execute("Definition c : Syntax := SVar 2.").unwrap();
    repl.execute("Definition fa : Syntax := SApp (SName \"f\") a.")
        .unwrap();
    repl.execute("Definition fc : Syntax := SApp (SName \"f\") c.")
        .unwrap();
    repl.execute("Definition h1 : Syntax := SApp (SApp (SName \"Eq\") a) b.")
        .unwrap();
    repl.execute("Definition h2 : Syntax := SApp (SApp (SName \"Eq\") b) c.")
        .unwrap();
    repl.execute("Definition concl : Syntax := SApp (SApp (SName \"Eq\") fa) fc.")
        .unwrap();
    // h1 -> (h2 -> concl)
    repl.execute("Definition inner : Syntax := SApp (SApp (SName \"implies\") h2) concl.")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"implies\") h1) inner.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_cc goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(
        concluded, original,
        "CC should prove a=b -> b=c -> f(a)=f(c)"
    );
}

// =============================================================================
// FAILURE CASES
// =============================================================================

#[test]
fn test_cc_fails_different_functions() {
    let mut repl = Repl::new();
    // f(x) != g(x) (different function symbols)
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition fx : Syntax := SApp (SName \"f\") x.")
        .unwrap();
    repl.execute("Definition gx : Syntax := SApp (SName \"g\") x.")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Eq\") fx) gx.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_cc goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let result = repl.execute("Eval result.").unwrap();
    assert!(result.contains("Error"), "CC should fail on f(x) = g(x)");
}

#[test]
fn test_cc_fails_without_hypothesis() {
    let mut repl = Repl::new();
    // f(x) != f(y) (without x=y hypothesis)
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition y : Syntax := SVar 1.").unwrap();
    repl.execute("Definition fx : Syntax := SApp (SName \"f\") x.")
        .unwrap();
    repl.execute("Definition fy : Syntax := SApp (SName \"f\") y.")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Eq\") fx) fy.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_cc goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let result = repl.execute("Eval result.").unwrap();
    assert!(
        result.contains("Error"),
        "CC should fail on f(x) = f(y) without hypothesis"
    );
}
