//! The reflected `Forall` has two spellings, and every certifying door reads them as one proposition.
//!
//! Form 1 is `Forall (SLam T body)`; Form 2 is `Forall T (SLam T body)`, whose domain repeats the
//! one the λ already carries. The kernel's readers (`extract_forall_body`) accept both, so a door
//! that compares a case proof against a formula it BUILT in Form 2 must accept the same case
//! proved in Form 1, and must still refuse a Form-1 case whose λ binds a different domain, a
//! wrong body, or a wrong sibling case.
//!
//! Three doors are locked: `try_destruct` (cases without an induction hypothesis),
//! `try_induction` (cases with one), and the Nat-specific `DInduction`.

use logicaffeine_kernel::interface::Repl;

const ERROR: &str = "(SName \"Error\")";

const MOTIVE: &str = "SLam (SName \"Nat\") (SApp (SName \"P\") (SVar 0))";
const P_ZERO: &str = "SApp (SName \"P\") (SName \"Zero\")";
const P_ONE: &str = "SApp (SName \"P\") (SApp (SName \"Succ\") (SName \"Zero\"))";

const P_SUCC_K: &str = "SApp (SName \"P\") (SApp (SName \"Succ\") (SVar 0))";
const P_K: &str = "SApp (SName \"P\") (SVar 0)";

fn forall_form1(domain: &str, body: &str) -> String {
    format!("SApp (SName \"Forall\") (SLam (SName \"{domain}\") ({body}))")
}

fn forall_form2(domain: &str, lam_domain: &str, body: &str) -> String {
    format!("SApp (SApp (SName \"Forall\") (SName \"{domain}\")) (SLam (SName \"{lam_domain}\") ({body}))")
}

fn implies(a: &str, b: &str) -> String {
    format!("SApp (SApp (SName \"Implies\") ({a})) ({b})")
}

fn repl_with_motive() -> Repl {
    let mut repl = Repl::new();
    repl.execute(&format!("Definition motive : Syntax := {MOTIVE}."))
        .expect("motive");
    repl
}

/// What a door must conclude: `∀ n : Nat, P n`, spelled by the REPL itself.
fn owed_conclusion() -> String {
    let mut repl = repl_with_motive();
    repl.execute("Eval (SApp (SApp (SName \"Forall\") (SName \"Nat\")) motive).")
        .expect("owed conclusion")
}

fn conclude_two_cases(door: &str, zero_case: &str, succ_case: &str) -> String {
    let mut repl = repl_with_motive();
    repl.execute(&format!(
        "Definition case_zero : Derivation := DAxiom ({zero_case})."
    ))
    .expect("case_zero");
    repl.execute(&format!(
        "Definition case_succ : Derivation := DAxiom ({succ_case})."
    ))
    .expect("case_succ");
    repl.execute("Definition cases : Derivation := DCase case_zero (DCase case_succ DCaseEnd).")
        .expect("cases");
    repl.execute(&format!(
        "Definition proof : Derivation := {door} (SName \"Nat\") motive cases."
    ))
    .expect("proof");
    repl.execute("Eval (concludes proof).").expect("concludes")
}

fn destruct(zero_case: &str, succ_case: &str) -> String {
    conclude_two_cases("try_destruct", zero_case, succ_case)
}

fn induction(zero_case: &str, step_case: &str) -> String {
    conclude_two_cases("try_induction", zero_case, step_case)
}

fn nat_induction(base: &str, step: &str) -> String {
    let mut repl = repl_with_motive();
    repl.execute(&format!("Definition base : Derivation := DAxiom ({base})."))
        .expect("base");
    repl.execute(&format!("Definition step : Derivation := DAxiom ({step})."))
        .expect("step");
    repl.execute("Definition proof : Derivation := DInduction motive base step.")
        .expect("proof");
    repl.execute("Eval (concludes proof).").expect("concludes")
}

#[test]
fn the_step_formula_the_kernel_owes_is_the_form_two_spelling_these_tests_write() {
    let mut repl = repl_with_motive();
    let built = repl
        .execute("Eval (induction_step_goal (SName \"Nat\") motive (Succ Zero)).")
        .expect("kernel-built step goal");
    let written = repl
        .execute(&format!(
            "Eval ({}).",
            forall_form2("Nat", "Nat", &implies(P_K, P_SUCC_K))
        ))
        .expect("written step goal");
    assert_eq!(
        built, written,
        "the Form-2 step formula must be exactly the one the kernel builds"
    );
}

#[test]
fn destruct_accepts_a_case_proved_in_either_forall_spelling() {
    let owed = owed_conclusion();
    assert_ne!(owed, ERROR);
    assert_eq!(
        destruct(P_ZERO, &forall_form2("Nat", "Nat", P_SUCC_K)),
        owed,
        "Form 2 succ case"
    );
    assert_eq!(
        destruct(P_ZERO, &forall_form1("Nat", P_SUCC_K)),
        owed,
        "Form 1 succ case"
    );
}

#[test]
fn destruct_still_refuses_what_neither_spelling_proves() {
    assert_eq!(
        destruct(P_ZERO, &forall_form1("Nat", P_K)),
        ERROR,
        "Form 1 with the wrong body"
    );
    assert_eq!(
        destruct(P_ZERO, &forall_form1("Bool", P_SUCC_K)),
        ERROR,
        "Form 1 whose λ binds Bool"
    );
    assert_eq!(
        destruct(P_ZERO, &forall_form2("Bool", "Nat", P_SUCC_K)),
        ERROR,
        "Form 2 whose domain is Bool"
    );
    assert_eq!(
        destruct(P_ZERO, &forall_form2("Nat", "Bool", P_SUCC_K)),
        ERROR,
        "Form 2 whose λ binds Bool"
    );
    assert_eq!(
        destruct(P_ONE, &forall_form1("Nat", P_SUCC_K)),
        ERROR,
        "a wrong zero case beside a right Form 1 succ case"
    );
}

#[test]
fn induction_accepts_a_step_proved_in_either_forall_spelling() {
    let owed = owed_conclusion();
    let step_body = implies(P_K, P_SUCC_K);
    assert_eq!(
        induction(P_ZERO, &forall_form2("Nat", "Nat", &step_body)),
        owed,
        "Form 2 step case"
    );
    assert_eq!(
        induction(P_ZERO, &forall_form1("Nat", &step_body)),
        owed,
        "Form 1 step case"
    );
}

#[test]
fn induction_still_refuses_what_neither_spelling_proves() {
    let step_body = implies(P_K, P_SUCC_K);
    assert_eq!(
        induction(P_ZERO, &forall_form1("Nat", P_SUCC_K)),
        ERROR,
        "Form 1 step case without its hypothesis"
    );
    assert_eq!(
        induction(P_ZERO, &forall_form1("Bool", &step_body)),
        ERROR,
        "Form 1 step case whose λ binds Bool"
    );
    assert_eq!(
        induction(P_ZERO, &forall_form2("Bool", "Nat", &step_body)),
        ERROR,
        "Form 2 whose domain is Bool"
    );
    assert_eq!(
        induction(P_ONE, &forall_form1("Nat", &step_body)),
        ERROR,
        "a wrong base case beside a right Form 1 step"
    );
}

#[test]
fn nat_induction_accepts_a_step_proved_in_either_forall_spelling() {
    let owed = owed_conclusion();
    let step_body = implies(P_K, P_SUCC_K);
    assert_eq!(
        nat_induction(P_ZERO, &forall_form2("Nat", "Nat", &step_body)),
        owed,
        "Form 2 step"
    );
    assert_eq!(
        nat_induction(P_ZERO, &forall_form1("Nat", &step_body)),
        owed,
        "Form 1 step"
    );
}

#[test]
fn nat_induction_still_refuses_what_neither_spelling_proves() {
    let step_body = implies(P_K, P_SUCC_K);
    assert_eq!(
        nat_induction(P_ZERO, &forall_form1("Nat", P_SUCC_K)),
        ERROR,
        "Form 1 step without its hypothesis"
    );
    assert_eq!(
        nat_induction(P_ZERO, &forall_form1("Bool", &step_body)),
        ERROR,
        "Form 1 step whose λ binds Bool"
    );
    assert_eq!(
        nat_induction(P_ONE, &forall_form1("Nat", &step_body)),
        ERROR,
        "a wrong base beside a right Form 1 step"
    );
}
