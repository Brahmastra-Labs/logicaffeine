//! Phase Ring: Ring Tactic for Polynomial Equality
//!
//! The ring tactic proves polynomial equalities by:
//! 1. Reifying Syntax terms to internal polynomial representation
//! 2. Normalizing both sides to canonical form
//! 3. Checking if normalized forms are equal
//!
//! Implements:
//! - DRingSolve : Syntax -> Derivation (trusted oracle for ring proofs)
//! - try_ring : Syntax -> Derivation (ring tactic)

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// DRINGSSOLVE: TYPE CHECK
// =============================================================================

#[test]
fn test_dringssolve_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DRingSolve.").expect("Check DRingSolve");
    assert_eq!(result, "DRingSolve : Syntax -> Derivation");
}

#[test]
fn test_dringssolve_construction() {
    let mut repl = Repl::new();

    // Build a simple equality goal: Eq Int (SLit 1) (SLit 1)
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition one : Syntax := SLit 1.")
        .expect("Define one");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) one) one.")
        .expect("Define goal");
    repl.execute("Definition d : Derivation := DRingSolve goal.")
        .expect("Define d");

    let result = repl.execute("Check d.").expect("Check d");
    assert_eq!(result, "d : Derivation");
}

// =============================================================================
// TRY_RING: TYPE CHECK
// =============================================================================

#[test]
fn test_try_ring_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_ring.").expect("Check try_ring");
    assert_eq!(result, "try_ring : Syntax -> Derivation");
}

// =============================================================================
// TRY_RING: REFLEXIVITY (TRIVIAL CASE)
// =============================================================================

#[test]
fn test_try_ring_reflexive_constant() {
    let mut repl = Repl::new();

    // Goal: Eq Int 42 42
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition a : Syntax := SLit 42.")
        .expect("Define a");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) a) a.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Ring should prove reflexive equality");
}

#[test]
fn test_try_ring_reflexive_variable() {
    let mut repl = Repl::new();

    // Goal: Eq Int x x (where x is SVar 0)
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define x");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) x) x.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original);
}

// =============================================================================
// TRY_RING: CONSTANT ARITHMETIC
// =============================================================================

#[test]
fn test_try_ring_constant_add() {
    let mut repl = Repl::new();

    // Goal: Eq Int (add 1 2) 3
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"add\") (SLit 1)) (SLit 2).")
        .expect("Define lhs");
    repl.execute("Definition rhs : Syntax := SLit 3.")
        .expect("Define rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Ring should prove 1+2 = 3");
}

#[test]
fn test_try_ring_constant_mul() {
    let mut repl = Repl::new();

    // Goal: Eq Int (mul 3 4) 12
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"mul\") (SLit 3)) (SLit 4).")
        .expect("Define lhs");
    repl.execute("Definition rhs : Syntax := SLit 12.")
        .expect("Define rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Ring should prove 3*4 = 12");
}

// =============================================================================
// TRY_RING: COMMUTATIVITY
// =============================================================================

#[test]
fn test_try_ring_commutativity_add() {
    let mut repl = Repl::new();

    // Goal: Eq Int (add x y) (add y x)
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define x");
    repl.execute("Definition y : Syntax := SVar 1.")
        .expect("Define y");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"add\") x) y.")
        .expect("Define lhs");
    repl.execute("Definition rhs : Syntax := SApp (SApp (SName \"add\") y) x.")
        .expect("Define rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Ring should prove x+y = y+x");
}

#[test]
fn test_try_ring_commutativity_mul() {
    let mut repl = Repl::new();

    // Goal: Eq Int (mul x y) (mul y x)
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define x");
    repl.execute("Definition y : Syntax := SVar 1.")
        .expect("Define y");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"mul\") x) y.")
        .expect("Define lhs");
    repl.execute("Definition rhs : Syntax := SApp (SApp (SName \"mul\") y) x.")
        .expect("Define rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Ring should prove x*y = y*x");
}

// =============================================================================
// TRY_RING: ASSOCIATIVITY
// =============================================================================

#[test]
fn test_try_ring_associativity_add() {
    let mut repl = Repl::new();

    // Goal: Eq Int (add (add x y) z) (add x (add y z))
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define x");
    repl.execute("Definition y : Syntax := SVar 1.")
        .expect("Define y");
    repl.execute("Definition z : Syntax := SVar 2.")
        .expect("Define z");
    repl.execute("Definition xy : Syntax := SApp (SApp (SName \"add\") x) y.")
        .expect("Define xy");
    repl.execute("Definition yz : Syntax := SApp (SApp (SName \"add\") y) z.")
        .expect("Define yz");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"add\") xy) z.")
        .expect("Define lhs");
    repl.execute("Definition rhs : Syntax := SApp (SApp (SName \"add\") x) yz.")
        .expect("Define rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Ring should prove (x+y)+z = x+(y+z)");
}

#[test]
fn test_try_ring_associativity_mul() {
    let mut repl = Repl::new();

    // Goal: Eq Int (mul (mul x y) z) (mul x (mul y z))
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define x");
    repl.execute("Definition y : Syntax := SVar 1.")
        .expect("Define y");
    repl.execute("Definition z : Syntax := SVar 2.")
        .expect("Define z");
    repl.execute("Definition xy : Syntax := SApp (SApp (SName \"mul\") x) y.")
        .expect("Define xy");
    repl.execute("Definition yz : Syntax := SApp (SApp (SName \"mul\") y) z.")
        .expect("Define yz");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"mul\") xy) z.")
        .expect("Define lhs");
    repl.execute("Definition rhs : Syntax := SApp (SApp (SName \"mul\") x) yz.")
        .expect("Define rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Ring should prove (x*y)*z = x*(y*z)");
}

// =============================================================================
// TRY_RING: DISTRIBUTIVITY
// =============================================================================

#[test]
fn test_try_ring_distributivity_left() {
    let mut repl = Repl::new();

    // Goal: Eq Int (mul x (add y z)) (add (mul x y) (mul x z))
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define x");
    repl.execute("Definition y : Syntax := SVar 1.")
        .expect("Define y");
    repl.execute("Definition z : Syntax := SVar 2.")
        .expect("Define z");
    repl.execute("Definition yz : Syntax := SApp (SApp (SName \"add\") y) z.")
        .expect("Define yz");
    repl.execute("Definition xy : Syntax := SApp (SApp (SName \"mul\") x) y.")
        .expect("Define xy");
    repl.execute("Definition xz : Syntax := SApp (SApp (SName \"mul\") x) z.")
        .expect("Define xz");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"mul\") x) yz.")
        .expect("Define lhs");
    repl.execute("Definition rhs : Syntax := SApp (SApp (SName \"add\") xy) xz.")
        .expect("Define rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Ring should prove x*(y+z) = x*y + x*z");
}

#[test]
fn test_try_ring_distributivity_right() {
    let mut repl = Repl::new();

    // Goal: Eq Int (mul (add x y) z) (add (mul x z) (mul y z))
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define x");
    repl.execute("Definition y : Syntax := SVar 1.")
        .expect("Define y");
    repl.execute("Definition z : Syntax := SVar 2.")
        .expect("Define z");
    repl.execute("Definition xy : Syntax := SApp (SApp (SName \"add\") x) y.")
        .expect("Define xy");
    repl.execute("Definition xz : Syntax := SApp (SApp (SName \"mul\") x) z.")
        .expect("Define xz");
    repl.execute("Definition yz : Syntax := SApp (SApp (SName \"mul\") y) z.")
        .expect("Define yz");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"mul\") xy) z.")
        .expect("Define lhs");
    repl.execute("Definition rhs : Syntax := SApp (SApp (SName \"add\") xz) yz.")
        .expect("Define rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Ring should prove (x+y)*z = x*z + y*z");
}

// =============================================================================
// TRY_RING: THE COLLATZ GOAL
// =============================================================================

#[test]
fn test_try_ring_collatz_algebra() {
    let mut repl = Repl::new();

    // The Grand Challenge: Eq Int (3(2k+1) + 1) (6k + 4)
    // Expanded: add (mul 3 (add (mul 2 k) 1)) 1 = add (mul 6 k) 4
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition k : Syntax := SVar 0.")
        .expect("Define k");

    // LHS: 3(2k+1) + 1
    repl.execute("Definition two_k : Syntax := SApp (SApp (SName \"mul\") (SLit 2)) k.")
        .expect("Define two_k");
    repl.execute("Definition two_k_plus_1 : Syntax := SApp (SApp (SName \"add\") two_k) (SLit 1).")
        .expect("Define two_k_plus_1");
    repl.execute(
        "Definition three_times : Syntax := SApp (SApp (SName \"mul\") (SLit 3)) two_k_plus_1.",
    )
    .expect("Define three_times");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"add\") three_times) (SLit 1).")
        .expect("Define lhs");

    // RHS: 6k + 4
    repl.execute("Definition six_k : Syntax := SApp (SApp (SName \"mul\") (SLit 6)) k.")
        .expect("Define six_k");
    repl.execute("Definition rhs : Syntax := SApp (SApp (SName \"add\") six_k) (SLit 4).")
        .expect("Define rhs");

    // Goal
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Ring should prove 3(2k+1)+1 = 6k+4");
}

// =============================================================================
// TRY_RING: MIXED CONSTANTS AND VARIABLES
// =============================================================================

#[test]
fn test_try_ring_identity_add_zero() {
    let mut repl = Repl::new();

    // Goal: Eq Int (add x 0) x
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define x");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"add\") x) (SLit 0).")
        .expect("Define lhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) x.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Ring should prove x+0 = x");
}

#[test]
fn test_try_ring_identity_mul_one() {
    let mut repl = Repl::new();

    // Goal: Eq Int (mul x 1) x
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define x");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"mul\") x) (SLit 1).")
        .expect("Define lhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) x.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Ring should prove x*1 = x");
}

#[test]
fn test_try_ring_annihilator_mul_zero() {
    let mut repl = Repl::new();

    // Goal: Eq Int (mul x 0) 0
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define x");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"mul\") x) (SLit 0).")
        .expect("Define lhs");
    repl.execute("Definition rhs : Syntax := SLit 0.")
        .expect("Define rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Ring should prove x*0 = 0");
}

// =============================================================================
// TRY_RING: FAILURE CASES
// =============================================================================

#[test]
fn test_try_ring_fails_on_inequality() {
    let mut repl = Repl::new();

    // Goal: Eq Int x y (where x != y structurally and algebraically)
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define x");
    repl.execute("Definition y : Syntax := SVar 1.")
        .expect("Define y");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) x) y.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(
        result, "(SName \"Error\")",
        "Ring should fail on x = y where x != y"
    );
}

#[test]
fn test_try_ring_fails_on_non_equality() {
    let mut repl = Repl::new();

    // Not an equality goal
    repl.execute("Definition goal : Syntax := SName \"P\".")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(
        result, "(SName \"Error\")",
        "Ring should fail on non-equality"
    );
}

#[test]
fn test_try_ring_fails_on_division() {
    let mut repl = Repl::new();

    // Goal involving division (not a ring operation)
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define x");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"div\") x) (SLit 2).")
        .expect("Define lhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) lhs.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(
        result, "(SName \"Error\")",
        "Ring should fail on division"
    );
}

#[test]
fn test_try_ring_fails_on_wrong_constant() {
    let mut repl = Repl::new();

    // Goal: Eq Int (add 1 2) 4 (wrong answer)
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"add\") (SLit 1)) (SLit 2).")
        .expect("Define lhs");
    repl.execute("Definition rhs : Syntax := SLit 4.")
        .expect("Define rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(
        result, "(SName \"Error\")",
        "Ring should fail on 1+2 = 4"
    );
}

// =============================================================================
// TRY_RING: SUBTRACTION
// =============================================================================

#[test]
fn test_try_ring_subtraction() {
    let mut repl = Repl::new();

    // Goal: Eq Int (sub x x) 0
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define x");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"sub\") x) x.")
        .expect("Define lhs");
    repl.execute("Definition rhs : Syntax := SLit 0.")
        .expect("Define rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Ring should prove x-x = 0");
}

// =============================================================================
// TRY_RING: POLYNOMIAL EXPANSION
// =============================================================================

#[test]
fn test_try_ring_expand_square() {
    let mut repl = Repl::new();

    // Goal: Eq Int (x+y)^2 = x^2 + 2xy + y^2
    // (x+y)*(x+y) = add (add (mul x x) (mul 2 (mul x y))) (mul y y)
    repl.execute("Definition T : Syntax := SName \"Int\".")
        .expect("Define T");
    repl.execute("Definition x : Syntax := SVar 0.")
        .expect("Define x");
    repl.execute("Definition y : Syntax := SVar 1.")
        .expect("Define y");

    // LHS: (x+y)*(x+y)
    repl.execute("Definition xpy : Syntax := SApp (SApp (SName \"add\") x) y.")
        .expect("Define xpy");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"mul\") xpy) xpy.")
        .expect("Define lhs");

    // RHS: x^2 + 2xy + y^2
    repl.execute("Definition xx : Syntax := SApp (SApp (SName \"mul\") x) x.")
        .expect("Define xx");
    repl.execute("Definition yy : Syntax := SApp (SApp (SName \"mul\") y) y.")
        .expect("Define yy");
    repl.execute("Definition xy : Syntax := SApp (SApp (SName \"mul\") x) y.")
        .expect("Define xy");
    repl.execute("Definition two_xy : Syntax := SApp (SApp (SName \"mul\") (SLit 2)) xy.")
        .expect("Define two_xy");
    repl.execute("Definition xx_plus_2xy : Syntax := SApp (SApp (SName \"add\") xx) two_xy.")
        .expect("Define xx_plus_2xy");
    repl.execute("Definition rhs : Syntax := SApp (SApp (SName \"add\") xx_plus_2xy) yy.")
        .expect("Define rhs");

    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("Define goal");

    repl.execute("Definition d : Derivation := try_ring goal.")
        .expect("Apply try_ring");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let concluded = repl.execute("Eval result.").expect("Eval result");
    let original = repl.execute("Eval goal.").expect("Eval goal");
    assert_eq!(concluded, original, "Ring should prove (x+y)^2 = x^2 + 2xy + y^2");
}

// =============================================================================
// TYPE ERRORS
// =============================================================================

#[test]
fn test_dringssolve_type_error() {
    let mut repl = Repl::new();

    // DRingSolve expects Syntax, not Int
    let result = repl.execute("Check (DRingSolve 42).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

#[test]
fn test_try_ring_type_error() {
    let mut repl = Repl::new();

    // try_ring expects Syntax, not Int
    let result = repl.execute("Check (try_ring 42).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}
