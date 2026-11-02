//! A tensed proposition — `Past(run(Rex))`, "Rex ran" — is one proposition wherever it appears.
//! It conflicts with its own negation; a rule that concludes it is refuted when it is refuted, so
//! refuting the rule's conclusion refutes whichever conjunct of its condition is not known; and
//! the tense is part of the proposition, so "Rex ran" says nothing of whether Rex will run.

use logicaffeine_proof::verify::{detect_conflict, prove_certify_check};
use logicaffeine_proof::{BackwardChainer, ProofExpr, ProofTerm};

fn predicate(name: &str, arg: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate { name: name.into(), args: vec![arg], world: None }
}

fn rex() -> ProofTerm {
    ProofTerm::Constant("Rex".into())
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

fn tensed(operator: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::Temporal { operator: operator.into(), body: Box::new(body) }
}

fn past(name: &str, who: ProofTerm) -> ProofExpr {
    tensed("Past", predicate(name, who))
}

fn engine_proves(premises: &[ProofExpr], goal: &ProofExpr) -> bool {
    let mut engine = BackwardChainer::new();
    for premise in premises.iter().cloned() {
        engine.add_axiom(premise);
    }
    engine.prove(goal.clone()).is_ok()
}

fn proves_and_certifies(premises: &[ProofExpr], goal: &ProofExpr) {
    assert!(engine_proves(premises, goal), "the engine did not prove {goal} from {premises:?}");
    let proof = prove_certify_check(premises, goal);
    assert!(proof.verified, "{:?}", proof.verification_error);
}

/// "Rex ran. Rex did not run."
#[test]
fn a_tensed_fact_conflicts_with_its_negation() {
    let report = detect_conflict(&[past("run", rex()), not(past("run", rex()))]);
    assert!(report.inconsistent, "{:?}", report.error);
}

/// "Rex ran" and "Rex will not run" are both true of a dog who ran once; so are "Rex ran" and
/// "Rex does not run".
#[test]
fn a_tense_is_part_of_its_proposition() {
    for premises in [
        [past("run", rex()), not(tensed("Future", predicate("run", rex())))],
        [past("run", rex()), not(predicate("run", rex()))],
    ] {
        let report = detect_conflict(&premises);
        assert!(!report.inconsistent, "{premises:?} read as a conflict");
    }
}

/// "If Rex is a dog and barks, he ran. He did not run. Rex is a dog." — so Rex does not bark.
#[test]
fn a_refuted_tensed_consequent_of_a_ground_rule_refutes_the_unknown_conjunct() {
    let rule = implies(and(predicate("dog", rex()), predicate("bark", rex())), past("run", rex()));
    proves_and_certifies(&[rule, not(past("run", rex())), predicate("dog", rex())], &not(predicate("bark", rex())));
}

/// "Every dog that barks ran. Rex did not run. Rex is a dog." — so Rex does not bark.
#[test]
fn a_refuted_tensed_consequent_of_a_universal_rule_refutes_the_unknown_conjunct() {
    let rule = every_x(implies(and(predicate("dog", x()), predicate("bark", x())), past("run", x())));
    proves_and_certifies(&[rule, not(past("run", rex())), predicate("dog", rex())], &not(predicate("bark", rex())));
}

/// "Every dog that barks ran; whoever ran tired. Rex did not tire. Rex is a dog." — so Rex does
/// not bark: the refutation runs back through both tensed rules.
#[test]
fn a_refutation_runs_back_through_a_chain_of_tensed_rules() {
    let barked_so_ran = every_x(implies(and(predicate("dog", x()), predicate("bark", x())), past("run", x())));
    let ran_so_tired = every_x(implies(past("run", x()), past("tire", x())));
    proves_and_certifies(
        &[barked_so_ran, ran_so_tired, not(past("tire", rex())), predicate("dog", rex())],
        &not(predicate("bark", rex())),
    );
}
