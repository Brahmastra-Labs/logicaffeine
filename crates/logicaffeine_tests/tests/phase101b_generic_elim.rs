//! Phase 101b: Generic Elimination (DElim)
//!
//! Implements generic induction/elimination for ANY inductive type.
//!
//! New constructors:
//! - DCase : Derivation -> Derivation -> Derivation (Cons cell for case proofs)
//! - DCaseEnd : Derivation (Nil for case proofs)
//! - DElim : Syntax -> Syntax -> Derivation -> Derivation
//!
//! DElim introspects the inductive type's constructors and validates
//! that each case proof in the DCase chain matches the expected goal.

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// DCASE / DCASEEND: TYPE CHECKING
// =============================================================================

#[test]
fn test_dcase_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DCase.").expect("Check DCase");
    assert_eq!(result, "DCase : Derivation -> Derivation -> Derivation");
}

#[test]
fn test_dcaseend_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DCaseEnd.").expect("Check DCaseEnd");
    assert_eq!(result, "DCaseEnd : Derivation");
}

#[test]
fn test_delim_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DElim.").expect("Check DElim");
    // DElim : Syntax -> Syntax -> Derivation -> Derivation
    // (ind_type, motive, cases)
    assert_eq!(result, "DElim : Syntax -> Syntax -> Derivation -> Derivation");
}

// =============================================================================
// DCASE CHAIN CONSTRUCTION
// =============================================================================

#[test]
fn test_build_case_chain() {
    let mut repl = Repl::new();

    // Build a chain of two cases using axioms
    repl.execute(r#"Definition case1 : Derivation := DAxiom (SName "P1")."#)
        .expect("Define case1");
    repl.execute(r#"Definition case2 : Derivation := DAxiom (SName "P2")."#)
        .expect("Define case2");

    // Build chain: DCase case1 (DCase case2 DCaseEnd)
    let result = repl.execute("Definition cases : Derivation := DCase case1 (DCase case2 DCaseEnd).");
    assert!(result.is_ok(), "Should build case chain: {:?}", result);

    // Type check the chain
    let output = repl.execute("Check cases.").expect("Check cases");
    assert!(output.contains("Derivation"), "Cases should be Derivation: {}", output);
}

// =============================================================================
// DELIM ON NAT: SHOULD WORK LIKE DINDUCTION
// =============================================================================

#[test]
fn test_delim_nat_type_check() {
    let mut repl = Repl::new();

    // Motive: λn:Nat. Eq Nat n n
    repl.execute(r#"Definition motive : Syntax := SLam (SName "Nat") (SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 0))."#)
        .expect("Define motive");

    // Base case: Eq Nat Zero Zero (by reflexivity)
    repl.execute(r#"Definition base : Derivation := DRefl (SName "Nat") (SName "Zero")."#)
        .expect("Define base");

    // Step case (using axiom for simplicity)
    repl.execute(r#"
        Definition step_formula : Syntax :=
            SApp (SApp (SName "Forall") (SName "Nat"))
                (SLam (SName "Nat")
                    (SApp (SApp (SName "Implies")
                        (SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 0)))
                        (SApp (SApp (SApp (SName "Eq") (SName "Nat"))
                            (SApp (SName "Succ") (SVar 0)))
                            (SApp (SName "Succ") (SVar 0))))).
    "#).expect("Define step_formula");
    repl.execute("Definition step : Derivation := DAxiom step_formula.")
        .expect("Define step");

    // Build case chain: [base, step]
    repl.execute("Definition cases : Derivation := DCase base (DCase step DCaseEnd).")
        .expect("Define cases");

    // DElim (SName "Nat") motive cases
    let result = repl.execute(r#"Definition elim_proof : Derivation := DElim (SName "Nat") motive cases."#);
    assert!(result.is_ok(), "Should type-check DElim on Nat: {:?}", result);
}

#[test]
fn test_delim_nat_concludes() {
    let mut repl = Repl::new();

    // Same setup as above
    repl.execute(r#"Definition motive : Syntax := SLam (SName "Nat") (SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 0))."#)
        .expect("Define motive");
    repl.execute(r#"Definition base : Derivation := DRefl (SName "Nat") (SName "Zero")."#)
        .expect("Define base");
    repl.execute(r#"
        Definition step_formula : Syntax :=
            SApp (SApp (SName "Forall") (SName "Nat"))
                (SLam (SName "Nat")
                    (SApp (SApp (SName "Implies")
                        (SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 0)))
                        (SApp (SApp (SApp (SName "Eq") (SName "Nat"))
                            (SApp (SName "Succ") (SVar 0)))
                            (SApp (SName "Succ") (SVar 0))))).
    "#).expect("Define step_formula");
    repl.execute("Definition step : Derivation := DAxiom step_formula.")
        .expect("Define step");
    repl.execute("Definition cases : Derivation := DCase base (DCase step DCaseEnd).")
        .expect("Define cases");
    repl.execute(r#"Definition elim_proof : Derivation := DElim (SName "Nat") motive cases."#)
        .expect("Define elim_proof");

    // concludes should return Forall Nat motive
    let result = repl.execute("Eval (concludes elim_proof).").expect("Eval concludes");

    assert!(result.contains("Forall") || result.contains("forall"),
        "Should conclude with universal: {}", result);
    assert!(result.contains("Nat"), "Should mention Nat: {}", result);
}

// =============================================================================
// DELIM ON LIST: POLYMORPHIC INDUCTIVE
// =============================================================================

#[test]
fn test_delim_list_type_check() {
    let mut repl = Repl::new();

    // First define List (from Phase 101a)
    repl.execute("Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.")
        .expect("Define List");

    // Motive: λl:List Nat. Eq (List Nat) l l
    repl.execute(r#"
        Definition motive : Syntax :=
            SLam (SApp (SName "List") (SName "Nat"))
                (SApp (SApp (SApp (SName "Eq") (SApp (SName "List") (SName "Nat")))
                    (SVar 0))
                    (SVar 0)).
    "#).expect("Define motive");

    // Nil case: Eq (List Nat) (Nil Nat) (Nil Nat) by reflexivity
    repl.execute(r#"
        Definition nil_case : Derivation :=
            DRefl (SApp (SName "List") (SName "Nat")) (SApp (SName "Nil") (SName "Nat")).
    "#).expect("Define nil_case");

    // Cons case (axiom for now)
    repl.execute(r#"
        Definition cons_formula : Syntax :=
            SApp (SApp (SName "Forall") (SName "Nat"))
                (SLam (SName "Nat")
                    (SApp (SApp (SName "Forall") (SApp (SName "List") (SName "Nat")))
                        (SLam (SApp (SName "List") (SName "Nat"))
                            (SApp (SApp (SName "Implies")
                                (SApp (SApp (SApp (SName "Eq") (SApp (SName "List") (SName "Nat")))
                                    (SVar 0)) (SVar 0)))
                                (SApp (SApp (SApp (SName "Eq") (SApp (SName "List") (SName "Nat")))
                                    (SApp (SApp (SApp (SName "Cons") (SName "Nat")) (SVar 1)) (SVar 0)))
                                    (SApp (SApp (SApp (SName "Cons") (SName "Nat")) (SVar 1)) (SVar 0))))))).
    "#).expect("Define cons_formula");
    repl.execute("Definition cons_case : Derivation := DAxiom cons_formula.")
        .expect("Define cons_case");

    // Build case chain: [nil_case, cons_case]
    repl.execute("Definition cases : Derivation := DCase nil_case (DCase cons_case DCaseEnd).")
        .expect("Define cases");

    // DElim on List Nat
    let result = repl.execute(r#"
        Definition elim_proof : Derivation :=
            DElim (SApp (SName "List") (SName "Nat")) motive cases.
    "#);
    assert!(result.is_ok(), "Should type-check DElim on List: {:?}", result);
}

// =============================================================================
// ERROR CASES
// =============================================================================

#[test]
fn test_delim_wrong_number_of_cases() {
    let mut repl = Repl::new();

    // Motive for Nat
    repl.execute(r#"Definition motive : Syntax := SLam (SName "Nat") (SName "Nat")."#)
        .expect("Define motive");

    // Only one case (Nat has 2 constructors)
    repl.execute(r#"Definition one_case : Derivation := DAxiom (SName "P")."#)
        .expect("Define one_case");
    repl.execute("Definition cases : Derivation := DCase one_case DCaseEnd.")
        .expect("Define cases");

    repl.execute(r#"Definition bad_elim : Derivation := DElim (SName "Nat") motive cases."#)
        .expect("Define bad_elim");

    // concludes should return Error
    let result = repl.execute("Eval (concludes bad_elim).").expect("Eval");
    assert!(result.contains("Error"), "Should error with wrong case count: {}", result);
}

#[test]
fn test_delim_wrong_case_conclusion() {
    let mut repl = Repl::new();

    // Motive: λn:Nat. Eq Nat n n
    repl.execute(r#"Definition motive : Syntax := SLam (SName "Nat") (SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 0))."#)
        .expect("Define motive");

    // Wrong base case (proves something else)
    repl.execute(r#"Definition bad_base : Derivation := DAxiom (SName "Wrong")."#)
        .expect("Define bad_base");

    // Step case (also wrong, but doesn't matter)
    repl.execute(r#"Definition step : Derivation := DAxiom (SName "AlsoWrong")."#)
        .expect("Define step");

    repl.execute("Definition cases : Derivation := DCase bad_base (DCase step DCaseEnd).")
        .expect("Define cases");

    repl.execute(r#"Definition bad_elim : Derivation := DElim (SName "Nat") motive cases."#)
        .expect("Define bad_elim");

    // concludes should return Error (base case doesn't match)
    let result = repl.execute("Eval (concludes bad_elim).").expect("Eval");
    assert!(result.contains("Error"), "Should error with wrong case: {}", result);
}

#[test]
fn test_delim_unknown_inductive() {
    let mut repl = Repl::new();

    repl.execute(r#"Definition motive : Syntax := SLam (SName "Unknown") (SName "Prop")."#)
        .expect("Define motive");
    repl.execute(r#"Definition cases : Derivation := DCaseEnd."#)
        .expect("Define cases");

    repl.execute(r#"Definition bad_elim : Derivation := DElim (SName "Unknown") motive cases."#)
        .expect("Define bad_elim");

    // concludes should return Error (Unknown not an inductive)
    let result = repl.execute("Eval (concludes bad_elim).").expect("Eval");
    assert!(result.contains("Error"), "Should error with unknown inductive: {}", result);
}

// =============================================================================
// TYPE ERRORS
// =============================================================================

#[test]
fn test_dcase_type_error() {
    let mut repl = Repl::new();
    // DCase expects Derivation, not Syntax
    let result = repl.execute(r#"Check (DCase (SName "x") DCaseEnd)."#);
    assert!(result.is_err(), "Should reject Syntax where Derivation expected");
}

#[test]
fn test_delim_type_error_motive() {
    let mut repl = Repl::new();
    // DElim expects Syntax for motive, not Int
    let result = repl.execute("Check (DElim (SName \"Nat\") 42 DCaseEnd).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

#[test]
fn test_delim_type_error_cases() {
    let mut repl = Repl::new();
    // DElim expects Derivation for cases, not Syntax
    let result = repl.execute(r#"Check (DElim (SName "Nat") (SName "motive") (SName "not_cases"))."#);
    assert!(result.is_err(), "Should reject Syntax where Derivation expected");
}
