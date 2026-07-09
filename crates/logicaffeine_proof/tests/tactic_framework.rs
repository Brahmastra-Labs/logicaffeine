//! The tactic framework (ROOT R5): build proofs interactively with intro/apply/
//! exact/split/exists/auto, then `qed` certifies the assembled derivation to the
//! kernel. Every proof here is kernel-checked — the tactics only construct the
//! derivation the kernel validates.

use logicaffeine_proof::tactic::ProofState;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn p(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args, world: None }
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}
fn or(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(l), Box::new(r))
}
fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: var.to_string(), body: Box::new(body) }
}
fn exists(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::Exists { variable: var.to_string(), body: Box::new(body) }
}

#[test]
fn intro_then_assumption_proves_self_implication() {
    // ⊢ man(Socrates) → man(Socrates), by `intro h; assumption`.
    let goal = implies(p("man", vec![k("Socrates")]), p("man", vec![k("Socrates")]));
    let mut st = ProofState::start(vec![], goal);
    st.intro("h").unwrap();
    st.assumption().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "intro;assumption: {:?}", r.verification_error);
}

#[test]
fn nested_intro_proves_constant_implication() {
    // ⊢ P → (Q → P), by `intro hp; intro hq; exact hp`. The classic K combinator.
    let goal = implies(
        p("rains", vec![k("Tuesday")]),
        implies(p("snows", vec![k("Tuesday")]), p("rains", vec![k("Tuesday")])),
    );
    let mut st = ProofState::start(vec![], goal);
    st.intro("hp").unwrap();
    st.intro("hq").unwrap();
    st.exact("hp").unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "K combinator: {:?}", r.verification_error);
}

#[test]
fn universal_intro_proves_reflexive_implication() {
    // ⊢ ∀x. mortal(x) → mortal(x), by `intro x; intro h; assumption`.
    let goal = forall("z", implies(p("mortal", vec![v("z")]), p("mortal", vec![v("z")])));
    let mut st = ProofState::start(vec![], goal);
    st.intro("z").unwrap();
    st.intro("h").unwrap();
    st.assumption().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "∀-intro reflexive: {:?}", r.verification_error);
}

#[test]
fn engine_proves_same_universal_via_prove_certify_check() {
    // Control: the SAME ∀-goal through the engine path (prove + finish_check), to
    // isolate whether the overflow is the tactic tree or the check_derivation path.
    use logicaffeine_proof::verify::prove_certify_check;
    let goal = forall("z", implies(p("mortal", vec![v("z")]), p("mortal", vec![v("z")])));
    let r = prove_certify_check(&[], &goal);
    assert!(r.verified, "engine ∀-proof: {:?}", r.verification_error);
}

#[test]
fn check_derivation_on_engine_universal_tree() {
    // Is the overflow in `check_derivation`, or in the tactic tree? Feed the ENGINE's
    // own ∀-derivation through `check_derivation` (the same door the tactics use).
    use logicaffeine_proof::verify::check_derivation;
    use logicaffeine_proof::BackwardChainer;
    let goal = forall("z", implies(p("mortal", vec![v("z")]), p("mortal", vec![v("z")])));
    let mut eng = BackwardChainer::new();
    let tree = eng.prove(goal.clone()).unwrap();
    let r = check_derivation(&[], &goal, tree);
    assert!(r.verified, "check_derivation on engine ∀-tree: {:?}", r.verification_error);
}

#[test]
fn split_proves_conjunction_from_premises() {
    // happy(Bob), tall(Bob) ⊢ happy(Bob) ∧ tall(Bob), by `split; assumption; assumption`.
    let premises = vec![p("happy", vec![k("Bob")]), p("tall", vec![k("Bob")])];
    let goal = and(p("happy", vec![k("Bob")]), p("tall", vec![k("Bob")]));
    let mut st = ProofState::start(premises, goal);
    st.split().unwrap();
    st.assumption().unwrap(); // happy(Bob)
    st.assumption().unwrap(); // tall(Bob)
    let r = st.qed().unwrap();
    assert!(r.verified, "split: {:?}", r.verification_error);
}

#[test]
fn left_proves_disjunction() {
    // happy(Bob) ⊢ happy(Bob) ∨ sad(Bob), by `left; assumption`.
    let premises = vec![p("happy", vec![k("Bob")])];
    let goal = or(p("happy", vec![k("Bob")]), p("sad", vec![k("Bob")]));
    let mut st = ProofState::start(premises, goal);
    st.left().unwrap();
    st.assumption().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "left: {:?}", r.verification_error);
}

#[test]
fn exists_intro_with_witness() {
    // mortal(Socrates) ⊢ ∃x. mortal(x), by `exists Socrates; assumption`.
    let premises = vec![p("mortal", vec![k("Socrates")])];
    let goal = exists("x", p("mortal", vec![v("x")]));
    let mut st = ProofState::start(premises, goal);
    st.exists(k("Socrates")).unwrap();
    st.assumption().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "exists: {:?}", r.verification_error);
}

#[test]
fn apply_modus_ponens_then_assumption() {
    // (man(Socrates) → mortal(Socrates)), man(Socrates) ⊢ mortal(Socrates),
    // by `apply (man→mortal); assumption`.
    let rule = implies(p("man", vec![k("Socrates")]), p("mortal", vec![k("Socrates")]));
    let premises = vec![rule.clone(), p("man", vec![k("Socrates")])];
    let goal = p("mortal", vec![k("Socrates")]);
    let mut st = ProofState::start(premises, goal);
    st.apply(&rule).unwrap();
    st.assumption().unwrap(); // man(Socrates)
    let r = st.qed().unwrap();
    assert!(r.verified, "apply MP: {:?}", r.verification_error);
}

#[test]
fn auto_closes_with_backward_chainer() {
    // ∀x. man(x) → mortal(x), man(Socrates) ⊢ mortal(Socrates), discharged by `auto`
    // (the backward chainer hosted as a tactic).
    let rule = forall(
        "x",
        implies(p("man", vec![v("x")]), p("mortal", vec![v("x")])),
    );
    let premises = vec![rule, p("man", vec![k("Socrates")])];
    let goal = p("mortal", vec![k("Socrates")]);
    let mut st = ProofState::start(premises, goal);
    st.auto().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "auto: {:?}", r.verification_error);
}

#[test]
fn incomplete_proof_is_rejected() {
    // Leaving a goal open must not yield a verified proof.
    let goal = implies(p("P", vec![k("a")]), p("Q", vec![k("a")]));
    let mut st = ProofState::start(vec![], goal);
    st.intro("h").unwrap();
    // The goal Q(a) is unprovable from P(a); `qed` with the goal still open must error.
    assert!(st.qed().is_err(), "qed with an open goal must fail");
    assert_eq!(st.open_goals(), 1);
}

