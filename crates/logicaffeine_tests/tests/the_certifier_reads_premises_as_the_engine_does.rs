//! The engine and the certifier read every premise and goal the same way. A self-named guard —
//! `dog(Dog)`, a definite description's "the dog is a dog" — is a fact like any other and stays
//! where it is written: dropped from a rule's antecedent it would strengthen the rule into one the
//! premises do not state, and dropped from a goal it would prove less than was asked. A rule that
//! concludes a guard alongside something else still proves the something else, one conjunct of
//! its conclusion, and the certifier checks exactly the derivation the engine built.

use logicaffeine_proof::verify::prove_certify_check;
use logicaffeine_proof::{BackwardChainer, ProofExpr, ProofTerm};

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

fn every_x(antecedent: ProofExpr, consequent: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll {
        variable: "x".into(),
        body: Box::new(ProofExpr::Implies(Box::new(antecedent), Box::new(consequent))),
    }
}

/// `∀x(bark(x) → run(x) ∧ dog(Dog))`, `bark(Fido)` ⊢ `run(Fido)`: the guard is one conjunct of
/// what the rule concludes, and the other conjunct is proved — and certified — without it.
#[test]
fn a_guard_a_premise_asserts_is_a_fact_and_blocks_no_proof() {
    let rule = every_x(predicate("bark", x()), and(predicate("run", x()), predicate("dog", constant("Dog"))));
    let proof = prove_certify_check(&[rule, predicate("bark", constant("Fido"))], &predicate("run", constant("Fido")));
    assert!(proof.verified, "{:?}", proof.verification_error);
}

/// `∀x(dog(Dog) ∧ bark(x) → run(John))`, `bark(Fido)` ⊬ `run(John)`: nothing says the dog is a
/// dog, and dropping that condition from the antecedent would prove what the premises do not say.
#[test]
fn a_guard_a_rule_requires_is_kept() {
    let rule = every_x(and(predicate("dog", constant("Dog")), predicate("bark", x())), predicate("run", constant("John")));
    let premises = [rule, predicate("bark", constant("Fido"))];
    let mut engine = BackwardChainer::new();
    for premise in premises.iter().cloned() {
        engine.add_axiom(premise);
    }
    assert!(engine.prove(predicate("run", constant("John"))).is_err(), "the engine proved what the premises do not say");
    assert!(!prove_certify_check(&premises, &predicate("run", constant("John"))).verified);
}

/// With the guard given, the rule fires — and certifies, because both sides read the antecedent
/// the same, unsimplified way.
#[test]
fn a_guard_a_rule_requires_is_met_by_a_premise() {
    let rule = every_x(and(predicate("dog", constant("Dog")), predicate("bark", x())), predicate("run", constant("John")));
    let proof = prove_certify_check(
        &[rule, predicate("dog", constant("Dog")), predicate("bark", constant("Fido"))],
        &predicate("run", constant("John")),
    );
    assert!(proof.verified, "{:?}", proof.verification_error);
}
