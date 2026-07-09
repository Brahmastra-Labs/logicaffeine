//! Tactic combinators (ROOT R5): `seq`, `first`, `try_`, `repeat`, `all_goals`, and
//! `then_all` (`<;>`) compose the primitive tactics into a real proof language. The
//! backtracking combinators speculate on a clone of the state and commit only on
//! success. Every assembled proof is kernel-certified.

use logicaffeine_proof::tactic::combinators::{
    all_goals, assumption, first, intro, repeat, seq, split, then_all, try_,
};
use logicaffeine_proof::tactic::ProofState;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn pr(name: &str, who: &str) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args: vec![k(who)], world: None }
}
fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}

#[test]
fn seq_runs_tactics_in_order() {
    // happy(Bob), tall(Bob) ⊢ happy(Bob) ∧ tall(Bob), by `split; assumption; assumption`.
    let premises = vec![pr("happy", "Bob"), pr("tall", "Bob")];
    let goal = and(pr("happy", "Bob"), pr("tall", "Bob"));
    let mut st = ProofState::start(premises, goal);
    st.run(&seq(vec![split(), assumption(), assumption()])).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "seq: {:?}", r.verification_error);
}

#[test]
fn first_commits_the_first_applicable() {
    // happy(Bob) ⊢ happy(Bob): `split` does not apply (not a ∧), `assumption` does.
    let premises = vec![pr("happy", "Bob")];
    let goal = pr("happy", "Bob");
    let mut st = ProofState::start(premises, goal);
    st.run(&first(vec![split(), assumption()])).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "first: {:?}", r.verification_error);
}

#[test]
fn first_fails_when_nothing_applies() {
    // No alternative proves an unrelated atom from no premises.
    let goal = pr("happy", "Bob");
    let mut st = ProofState::start(vec![], goal);
    assert!(st.run(&first(vec![split(), assumption()])).is_err());
}

#[test]
fn try_is_a_noop_on_failure() {
    // `try split` on an atom leaves the goal untouched; `assumption` then closes it.
    let premises = vec![pr("happy", "Bob")];
    let goal = pr("happy", "Bob");
    let mut st = ProofState::start(premises, goal);
    st.run(&seq(vec![try_(split()), assumption()])).unwrap();
    assert_eq!(st.open_goals(), 0);
    let r = st.qed().unwrap();
    assert!(r.verified, "try: {:?}", r.verification_error);
}

#[test]
fn repeat_decomposes_a_nested_conjunction() {
    // (happy ∧ tall) ∧ smart, all of Bob — repeatedly split-or-assume until done.
    let premises = vec![pr("happy", "Bob"), pr("tall", "Bob"), pr("smart", "Bob")];
    let goal = and(and(pr("happy", "Bob"), pr("tall", "Bob")), pr("smart", "Bob"));
    let mut st = ProofState::start(premises, goal);
    st.run(&repeat(first(vec![split(), assumption()]))).unwrap();
    assert_eq!(st.open_goals(), 0, "repeat should close every goal");
    let r = st.qed().unwrap();
    assert!(r.verified, "repeat: {:?}", r.verification_error);
}

#[test]
fn all_goals_applies_to_every_subgoal() {
    // After `split` there are two goals; `all_goals(assumption)` closes both at once.
    let premises = vec![pr("happy", "Bob"), pr("tall", "Bob")];
    let goal = and(pr("happy", "Bob"), pr("tall", "Bob"));
    let mut st = ProofState::start(premises, goal);
    st.run(&seq(vec![split(), all_goals(assumption())])).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "all_goals: {:?}", r.verification_error);
}

#[test]
fn then_all_is_the_semicolon_combinator() {
    // `split <;> assumption`: split, then assumption on EVERY resulting goal.
    let premises = vec![pr("happy", "Bob"), pr("tall", "Bob")];
    let goal = and(pr("happy", "Bob"), pr("tall", "Bob"));
    let mut st = ProofState::start(premises, goal);
    st.run(&then_all(split(), assumption())).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "then_all: {:?}", r.verification_error);
}

#[test]
fn composed_script_intro_then_split() {
    // happy(Bob), tall(Bob) ⊢ rains(Day) → (happy(Bob) ∧ tall(Bob)), the whole proof
    // as ONE composed tactic: `intro h; (split <;> assumption)`. Exercises `intro`,
    // `seq`, and `then_all` together — the shape of a real scripted proof.
    let premises = vec![pr("happy", "Bob"), pr("tall", "Bob")];
    let goal = implies(pr("rains", "Day"), and(pr("happy", "Bob"), pr("tall", "Bob")));
    let mut st = ProofState::start(premises, goal);
    st.run(&seq(vec![intro("h"), then_all(split(), assumption())])).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "composed: {:?}", r.verification_error);
}
