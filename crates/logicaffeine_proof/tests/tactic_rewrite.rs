//! The `rewrite` tactic (ROOT R5): equational reasoning. `rewrite h` with `h : a = b`
//! substitutes `a ↦ b` throughout the goal (Leibniz, certified by `Eq_rec`). With
//! it the tactic language can reason up to equality, not just structurally. Every
//! proof is kernel-certified.

use logicaffeine_proof::tactic::combinators::{assumption, rewrite, seq};
use logicaffeine_proof::tactic::ProofState;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn f(name: &str, arg: ProofTerm) -> ProofTerm {
    ProofTerm::Function(name.to_string(), vec![arg])
}
fn pr(name: &str, who: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args: vec![who], world: None }
}
fn eq(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(a, b)
}

#[test]
fn rewrite_substitutes_equal_for_equal() {
    // MorningStar = EveningStar, planet(EveningStar) ⊢ planet(MorningStar):
    // rewrite the goal's MorningStar to EveningStar, then it's a hypothesis.
    let premises = vec![
        eq(k("MorningStar"), k("EveningStar")),
        pr("planet", k("EveningStar")),
    ];
    let goal = pr("planet", k("MorningStar"));
    let mut st = ProofState::start(premises, goal);
    // hp0 : MorningStar = EveningStar ; hp1 : planet(EveningStar)
    st.run(&seq(vec![rewrite("hp0"), assumption()])).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "rewrite (equal-for-equal): {:?}", r.verification_error);
}

#[test]
fn rewrite_a_compound_term() {
    // f(A) = C, big(C) ⊢ big(f(A)): rewrite the compound term f(A) to C.
    let premises = vec![eq(f("f", k("A")), k("C")), pr("big", k("C"))];
    let goal = pr("big", f("f", k("A")));
    let mut st = ProofState::start(premises, goal);
    st.run(&seq(vec![rewrite("hp0"), assumption()])).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "rewrite (compound term): {:?}", r.verification_error);
}

#[test]
fn rewrite_then_rewrite_chains_equalities() {
    // A = B, B = C, here(C) ⊢ here(A): rewrite A→B, then B→C, then assumption.
    let premises = vec![
        eq(k("A"), k("B")),
        eq(k("B"), k("C")),
        pr("here", k("C")),
    ];
    let goal = pr("here", k("A"));
    let mut st = ProofState::start(premises, goal);
    // hp0 : A=B, hp1 : B=C, hp2 : here(C)
    st.run(&seq(vec![rewrite("hp0"), rewrite("hp1"), assumption()])).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "rewrite chain: {:?}", r.verification_error);
}

#[test]
fn rewrite_fails_with_no_occurrence() {
    // `rewrite` on an equality whose LHS does not appear in the goal does not apply.
    let premises = vec![eq(k("A"), k("B"))];
    let goal = pr("here", k("C"));
    let mut st = ProofState::start(premises, goal);
    assert!(st.rewrite("hp0").is_err(), "rewrite with no occurrence must fail");
}

#[test]
fn rewrite_requires_an_equality_hypothesis() {
    // `rewrite` on a non-equality hypothesis does not apply.
    let premises = vec![pr("happy", k("Bob"))];
    let goal = pr("happy", k("Bob"));
    let mut st = ProofState::start(premises, goal);
    assert!(st.rewrite("hp0").is_err(), "rewrite on a non-equality must fail");
}
