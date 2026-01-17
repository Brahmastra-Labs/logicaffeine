//! Phase 92: The Law (Deep Embedding of Inference Rules)
//!
//! Teaches the kernel to represent proof trees as data.
//! - Derivation: The type of proof trees
//! - concludes: Extract what a derivation proves

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// DERIVATION TYPE
// =============================================================================

#[test]
fn test_derivation_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check Derivation.").expect("Check Derivation");
    assert_eq!(result, "Derivation : Type0");
}

#[test]
fn test_daxiom_constructor() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DAxiom.").expect("Check DAxiom");
    assert_eq!(result, "DAxiom : Syntax -> Derivation");
}

#[test]
fn test_dmodusponens_constructor() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DModusPonens.").expect("Check DModusPonens");
    assert_eq!(result, "DModusPonens : Derivation -> Derivation -> Derivation");
}

#[test]
fn test_dunivintro_constructor() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DUnivIntro.").expect("Check DUnivIntro");
    assert_eq!(result, "DUnivIntro : Derivation -> Derivation");
}

#[test]
fn test_dunivelim_constructor() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DUnivElim.").expect("Check DUnivElim");
    assert_eq!(result, "DUnivElim : Derivation -> Syntax -> Derivation");
}

#[test]
fn test_concludes_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check concludes.").expect("Check concludes");
    assert_eq!(result, "concludes : Derivation -> Syntax");
}

// =============================================================================
// DAXIOM - AXIOM INTRODUCTION
// =============================================================================

#[test]
fn test_concludes_daxiom_simple() {
    let mut repl = Repl::new();

    repl.execute("Definition P : Syntax := SName \"P\".")
        .expect("Define P");
    repl.execute("Definition d : Derivation := DAxiom P.")
        .expect("Define d");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SName \"P\")");
}

#[test]
fn test_concludes_daxiom_complex() {
    let mut repl = Repl::new();

    repl.execute("Definition A : Syntax := SName \"A\".")
        .expect("Define A");
    repl.execute("Definition B : Syntax := SName \"B\".")
        .expect("Define B");
    repl.execute("Definition impl : Syntax := SApp (SApp (SName \"Implies\") A) B.")
        .expect("Define impl");
    repl.execute("Definition d : Derivation := DAxiom impl.")
        .expect("Define d");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(
        result,
        "((SApp ((SApp (SName \"Implies\")) (SName \"A\"))) (SName \"B\"))"
    );
}

// =============================================================================
// DMODUSPONENS - MODUS PONENS
// =============================================================================

#[test]
fn test_concludes_modus_ponens_valid() {
    let mut repl = Repl::new();

    repl.execute("Definition A : Syntax := SName \"A\".")
        .expect("Define A");
    repl.execute("Definition B : Syntax := SName \"B\".")
        .expect("Define B");
    repl.execute("Definition impl : Syntax := SApp (SApp (SName \"Implies\") A) B.")
        .expect("Define impl");

    repl.execute("Definition d_impl : Derivation := DAxiom impl.")
        .expect("Define d_impl");
    repl.execute("Definition d_ant : Derivation := DAxiom A.")
        .expect("Define d_ant");
    repl.execute("Definition d_mp : Derivation := DModusPonens d_impl d_ant.")
        .expect("Define d_mp");
    repl.execute("Definition result : Syntax := concludes d_mp.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SName \"B\")");
}

#[test]
fn test_concludes_modus_ponens_chained() {
    let mut repl = Repl::new();

    repl.execute("Definition A : Syntax := SName \"A\".")
        .expect("Define A");
    repl.execute("Definition B : Syntax := SName \"B\".")
        .expect("Define B");
    repl.execute("Definition C : Syntax := SName \"C\".")
        .expect("Define C");
    repl.execute("Definition impl_ab : Syntax := SApp (SApp (SName \"Implies\") A) B.")
        .expect("Define impl_ab");
    repl.execute("Definition impl_bc : Syntax := SApp (SApp (SName \"Implies\") B) C.")
        .expect("Define impl_bc");

    repl.execute("Definition d1 : Derivation := DModusPonens (DAxiom impl_ab) (DAxiom A).")
        .expect("Define d1");
    repl.execute("Definition d2 : Derivation := DModusPonens (DAxiom impl_bc) d1.")
        .expect("Define d2");
    repl.execute("Definition result : Syntax := concludes d2.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SName \"C\")");
}

#[test]
fn test_concludes_modus_ponens_invalid_not_implication() {
    let mut repl = Repl::new();

    repl.execute("Definition P : Syntax := SName \"P\".")
        .expect("Define P");
    repl.execute("Definition Q : Syntax := SName \"Q\".")
        .expect("Define Q");

    repl.execute("Definition d1 : Derivation := DAxiom P.")
        .expect("Define d1");
    repl.execute("Definition d2 : Derivation := DAxiom Q.")
        .expect("Define d2");
    repl.execute("Definition d_bad : Derivation := DModusPonens d1 d2.")
        .expect("Define d_bad");
    repl.execute("Definition result : Syntax := concludes d_bad.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SName \"Error\")");
}

#[test]
fn test_concludes_modus_ponens_invalid_antecedent_mismatch() {
    let mut repl = Repl::new();

    repl.execute("Definition A : Syntax := SName \"A\".")
        .expect("Define A");
    repl.execute("Definition B : Syntax := SName \"B\".")
        .expect("Define B");
    repl.execute("Definition C : Syntax := SName \"C\".")
        .expect("Define C");
    repl.execute("Definition impl : Syntax := SApp (SApp (SName \"Implies\") A) B.")
        .expect("Define impl");

    repl.execute("Definition d_impl : Derivation := DAxiom impl.")
        .expect("Define d_impl");
    repl.execute("Definition d_ant : Derivation := DAxiom C.")
        .expect("Define d_ant");
    repl.execute("Definition d_bad : Derivation := DModusPonens d_impl d_ant.")
        .expect("Define d_bad");
    repl.execute("Definition result : Syntax := concludes d_bad.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SName \"Error\")");
}

// =============================================================================
// DUNIVINTRO - UNIVERSAL INTRODUCTION (GENERALIZATION)
// =============================================================================

#[test]
fn test_concludes_univ_intro() {
    let mut repl = Repl::new();

    repl.execute("Definition P_x : Syntax := SApp (SName \"P\") (SVar 0).")
        .expect("Define P_x");
    repl.execute("Definition d : Derivation := DUnivIntro (DAxiom P_x).")
        .expect("Define d");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert!(result.contains("Forall"));
    assert!(result.contains("SLam"));
}

// =============================================================================
// DUNIVELIM - UNIVERSAL ELIMINATION (INSTANTIATION)
// =============================================================================

#[test]
fn test_concludes_univ_elim() {
    let mut repl = Repl::new();

    repl.execute("Definition type0 : Syntax := SSort (UType 0).")
        .expect("Define type0");
    repl.execute("Definition P_var : Syntax := SApp (SName \"P\") (SVar 0).")
        .expect("Define P_var");
    repl.execute(
        "Definition forall_P : Syntax := SApp (SApp (SName \"Forall\") type0) (SLam type0 P_var).",
    )
    .expect("Define forall_P");

    repl.execute("Definition t : Syntax := SName \"t\".")
        .expect("Define t");

    repl.execute("Definition d : Derivation := DUnivElim (DAxiom forall_P) t.")
        .expect("Define d");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "((SApp (SName \"P\")) (SName \"t\"))");
}

#[test]
fn test_concludes_univ_elim_invalid_not_forall() {
    let mut repl = Repl::new();

    repl.execute("Definition P : Syntax := SName \"P\".")
        .expect("Define P");
    repl.execute("Definition t : Syntax := SName \"t\".")
        .expect("Define t");

    repl.execute("Definition d : Derivation := DUnivElim (DAxiom P) t.")
        .expect("Define d");
    repl.execute("Definition result : Syntax := concludes d.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval");
    assert_eq!(result, "(SName \"Error\")");
}

// =============================================================================
// TYPE ERRORS
// =============================================================================

#[test]
fn test_daxiom_type_error() {
    let mut repl = Repl::new();
    let result = repl.execute("Check (DAxiom 42).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

#[test]
fn test_dmodusponens_type_error() {
    let mut repl = Repl::new();
    let result = repl.execute("Check (DModusPonens (SName \"P\") (SName \"Q\")).");
    assert!(
        result.is_err(),
        "Should reject Syntax where Derivation expected"
    );
}

#[test]
fn test_dunivelim_type_error() {
    let mut repl = Repl::new();
    repl.execute("Definition d : Derivation := DAxiom (SName \"P\").")
        .expect("Define d");
    let result = repl.execute("Check (DUnivElim d d).");
    assert!(
        result.is_err(),
        "Should reject Derivation where Syntax expected"
    );
}

#[test]
fn test_concludes_type_error() {
    let mut repl = Repl::new();
    let result = repl.execute("Check (concludes (SName \"P\")).");
    assert!(
        result.is_err(),
        "Should reject Syntax where Derivation expected"
    );
}
