//! Elimination tactics (ROOT R5): `cases`/`destruct` takes a hypothesis APART —
//! `∧` by projection, `∨` by case split, `∃` by introducing a fresh witness. With
//! these the tactic language can reason from compound hypotheses, not just build
//! compound goals. Every proof is kernel-certified.

use logicaffeine_proof::tactic::combinators::{
    assumption, auto, cases, intro, left, right, seq, split,
};
use logicaffeine_proof::tactic::ProofState;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn pr(name: &str, who: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args: vec![who], world: None }
}
fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}
fn or(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(l), Box::new(r))
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn exists(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::Exists { variable: var.to_string(), body: Box::new(body) }
}

#[test]
fn cases_conjunction_proves_commutativity() {
    // happy(Bob) ∧ tall(Bob) ⊢ tall(Bob) ∧ happy(Bob), by destructing the hypothesis
    // into its conjuncts and re-assembling them the other way round.
    let premises = vec![and(pr("happy", k("Bob")), pr("tall", k("Bob")))];
    let goal = and(pr("tall", k("Bob")), pr("happy", k("Bob")));
    let mut st = ProofState::start(premises, goal);
    // hyp is "hp0"; cases gives hp0_1 : happy(Bob), hp0_2 : tall(Bob).
    st.run(&seq(vec![cases("hp0"), split(), assumption(), assumption()])).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "cases ∧ (commutativity): {:?}", r.verification_error);
}

#[test]
fn cases_disjunction_proves_commutativity() {
    // ⊢ (happy(Bob) ∨ tall(Bob)) → (tall(Bob) ∨ happy(Bob)), by intro then case split:
    // in the happy branch prove the RIGHT disjunct, in the tall branch the LEFT.
    let goal = implies(
        or(pr("happy", k("Bob")), pr("tall", k("Bob"))),
        or(pr("tall", k("Bob")), pr("happy", k("Bob"))),
    );
    let mut st = ProofState::start(vec![], goal);
    st.run(&seq(vec![
        intro("h"),
        cases("h"),
        // branch h_l : happy(Bob), goal tall ∨ happy → prove right (happy)
        right(),
        assumption(),
        // branch h_r : tall(Bob), goal tall ∨ happy → prove left (tall)
        left(),
        assumption(),
    ]))
    .unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "cases ∨ (commutativity): {:?}", r.verification_error);
}

#[test]
fn cases_existential_opens_the_witness() {
    // ∃x. mortal(x) ⊢ ∃z. mortal(z): destruct the existential to a fresh witness with
    // `mortal(W)` in scope, then let `auto` re-introduce the existential. The witness
    // does not escape the goal, so `ExistentialElim` certifies.
    let premises = vec![exists("x", pr("mortal", v("x")))];
    let goal = exists("z", pr("mortal", v("z")));
    let mut st = ProofState::start(premises, goal);
    st.run(&seq(vec![cases("hp0"), auto()])).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "cases ∃ (open witness): {:?}", r.verification_error);
}

#[test]
fn cases_existential_of_conjunction_projects_through() {
    // ∃x. (P(x) ∧ Q(x)) ⊢ ∃y. P(y): open the witness, destruct the conjunction it
    // carries, and re-introduce the existential from the P-conjunct via `auto`.
    let premises = vec![exists("x", and(pr("P", v("x")), pr("Q", v("x"))))];
    let goal = exists("y", pr("P", v("y")));
    let mut st = ProofState::start(premises, goal);
    st.run(&seq(vec![cases("hp0"), cases("hp0_w"), auto()])).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "cases ∃ over ∧: {:?}", r.verification_error);
}

#[test]
fn cases_on_non_eliminable_fails() {
    // `cases` on an atomic hypothesis does not apply.
    let premises = vec![pr("happy", k("Bob"))];
    let goal = pr("happy", k("Bob"));
    let mut st = ProofState::start(premises, goal);
    assert!(st.cases("hp0").is_err(), "cases on an atom must fail");
}
