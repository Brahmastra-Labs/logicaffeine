//! "The butler" reaches the prover as a lowercase constant spelled as its noun, `butler`, and a
//! definite description presupposes what it describes: the butler is a butler. That fact is a
//! premise like any other — stated once, where the engine searches and the certifier checks alike
//! — so a rule guarded by the noun (`∀x(butler(x) ∧ do(x) → seen(x))`) applies to the butler
//! without anyone saying so. A proper name (`Butler`) presupposes nothing, and a guard the
//! premises do not meet still blocks the rule (see `the_certifier_reads_premises_as_the_engine_does`).
//!
//! Refuting one conjunct of a rule's antecedent is modus tollens with the other conjuncts known:
//! from `(A ∧ C) → Q`, `¬Q` and `A`, conclude `¬C`.

use logicaffeine_proof::verify::prove_certify_check;
use logicaffeine_proof::{BackwardChainer, ProofExpr, ProofGoal, ProofTerm};

fn predicate(name: &str, arg: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate { name: name.into(), args: vec![arg], world: None }
}

fn constant(name: &str) -> ProofTerm {
    ProofTerm::Constant(name.into())
}

fn x() -> ProofTerm {
    ProofTerm::Variable("x".into())
}

fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}

fn not(e: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(e))
}

fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}

fn every_x(body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: "x".into(), body: Box::new(body) }
}

fn some_x(body: ProofExpr) -> ProofExpr {
    ProofExpr::Exists { variable: "x".into(), body: Box::new(body) }
}

fn relation(name: &str, subject: ProofTerm, object: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate { name: name.into(), args: vec![subject, object], world: None }
}

/// `∀x(butler(x) → loyal(x))` — every butler is loyal.
fn every_butler_is_loyal() -> ProofExpr {
    every_x(implies(predicate("butler", x()), predicate("loyal", x())))
}

/// `∀x(butler(x) ∧ do(x) → seen(x))` — the rule a definite description's conditional becomes.
fn guarded_rule(noun: &str) -> ProofExpr {
    every_x(implies(and(predicate(noun, x()), predicate("do", x())), predicate("seen", x())))
}

fn engine_proves(premises: &[ProofExpr], goal: &ProofExpr) -> bool {
    let mut engine = BackwardChainer::new();
    for premise in premises.iter().cloned() {
        engine.add_axiom(premise);
    }
    engine.prove(goal.clone()).is_ok()
}

#[test]
fn a_known_conjunct_of_a_refuted_ground_rule_leaves_the_other_refuted() {
    let rule = implies(and(predicate("dog", constant("Rex")), predicate("bark", constant("Rex"))), predicate("run", constant("Rex")));
    let premises = [rule, not(predicate("run", constant("Rex"))), predicate("dog", constant("Rex"))];
    let proof = prove_certify_check(&premises, &not(predicate("bark", constant("Rex"))));
    assert!(proof.verified, "{:?}", proof.verification_error);
}

#[test]
fn a_known_conjunct_of_a_refuted_universal_rule_leaves_the_other_refuted() {
    let rule = every_x(implies(and(predicate("dog", x()), predicate("bark", x())), predicate("run", x())));
    let premises = [rule, not(predicate("run", constant("Rex"))), predicate("dog", constant("Rex"))];
    let proof = prove_certify_check(&premises, &not(predicate("bark", constant("Rex"))));
    assert!(proof.verified, "{:?}", proof.verification_error);
}

/// Without `dog(Rex)` the rule says nothing about Rex: `¬run(Rex)` refutes only the whole
/// antecedent, and Rex may bark as long as he is no dog.
#[test]
fn an_unknown_conjunct_refutes_nothing() {
    let rule = every_x(implies(and(predicate("dog", x()), predicate("bark", x())), predicate("run", x())));
    let premises = [rule, not(predicate("run", constant("Rex")))];
    let goal = not(predicate("bark", constant("Rex")));
    assert!(!engine_proves(&premises, &goal), "the engine refuted a conjunct the premises leave open");
    assert!(!prove_certify_check(&premises, &goal).verified);
}

/// "If the butler did it, he was seen. He was not seen." — so the butler did not do it.
#[test]
fn the_butler_is_a_butler() {
    let premises = [guarded_rule("butler"), not(predicate("seen", constant("butler")))];
    let goal = not(predicate("do", constant("butler")));
    assert!(engine_proves(&premises, &goal), "the engine did not read the butler as a butler");
    let proof = prove_certify_check(&premises, &goal);
    assert!(proof.verified, "{:?}", proof.verification_error);
}

/// A proper name is not a description: nothing says the man called Butler is a butler.
#[test]
fn a_proper_name_presupposes_nothing() {
    let premises = [guarded_rule("butler"), not(predicate("seen", constant("Butler")))];
    let goal = not(predicate("do", constant("Butler")));
    assert!(!engine_proves(&premises, &goal), "the engine read a proper name as a description");
    assert!(!prove_certify_check(&premises, &goal).verified);
}

/// The description presupposes its own noun and no other: the butler is not thereby a gardener.
#[test]
fn a_description_presupposes_only_its_own_noun() {
    let premises = [guarded_rule("gardener"), not(predicate("seen", constant("butler"))), predicate("butler", constant("Jeeves"))];
    let goal = not(predicate("do", constant("butler")));
    assert!(!engine_proves(&premises, &goal), "the engine presupposed a noun the description does not name");
    assert!(!prove_certify_check(&premises, &goal).verified);
}

/// A noun the problem also uses as a relation ("Jeeves is Wooster's butler") says nothing of one
/// argument about what "the butler" names: a relational description presupposes the relation, and
/// the kernel types `butler` by its widest use, where `butler(butler)` is no proposition at all.
#[test]
fn a_noun_used_as_a_relation_presupposes_nothing() {
    let premises = [
        guarded_rule("butler"),
        not(predicate("seen", constant("butler"))),
        relation("butler", constant("Jeeves"), constant("Wooster")),
    ];
    let goal = not(predicate("do", constant("butler")));
    assert!(!engine_proves(&premises, &goal), "the engine presupposed a one-place fact of a relational noun");
    assert!(!prove_certify_check(&premises, &goal).verified);
}

/// "The butler is loyal" follows from "every butler is loyal": the goal's own description
/// presupposes its noun as the premises' descriptions do.
#[test]
fn the_goals_own_description_presupposes_its_noun() {
    let premises = [every_butler_is_loyal()];
    let goal = predicate("loyal", constant("butler"));
    assert!(engine_proves(&premises, &goal), "the engine did not read the goal's butler as a butler");
    let proof = prove_certify_check(&premises, &goal);
    assert!(proof.verified, "{:?}", proof.verification_error);
}

/// A goal's local assumptions are part of the problem, so a description there presupposes its
/// noun too: every butler is loyal, the butler served the tea, so someone is loyal.
#[test]
fn a_description_in_the_goals_context_presupposes_its_noun() {
    let mut engine = BackwardChainer::new();
    engine.add_axiom(every_butler_is_loyal());
    let served = relation("serve", constant("butler"), constant("Tea"));
    let goal = ProofGoal::with_context(some_x(predicate("loyal", x())), vec![served]);
    assert!(engine.prove_with_goal(goal).is_ok(), "the engine did not read the context's butler as a butler");
}

/// What a goal's description presupposes belongs to that goal's proof. Asked next on the same
/// engine, a goal that describes no one finds no butler: "every butler is loyal" alone does not
/// say that someone is.
#[test]
fn a_goals_presupposition_ends_with_its_proof() {
    let mut engine = BackwardChainer::new();
    engine.add_axiom(every_butler_is_loyal());
    assert!(engine.prove(predicate("loyal", constant("butler"))).is_ok(), "the butler is a butler, so the butler is loyal");
    assert!(engine.prove(some_x(predicate("loyal", x()))).is_err(), "the last goal's butler outlived its proof");
}
