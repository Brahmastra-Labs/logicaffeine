//! The engine reasons over events abstracted to predicates: `∃e(Bark(e) ∧ Agent(e, John))` is
//! `bark(John)`. A binder of the event variable goes with the event — `∀e((Bark(e) ∧
//! Agent(e, John)) → ψ)` is `bark(John) → ψ` — because a vacuous binder left behind is one the
//! search never instantiates, and "If John barks, Mary runs" would prove nothing.

use logicaffeine_proof::{BackwardChainer, ProofExpr, ProofTerm};

fn event(verb: &str, agent: &str) -> ProofExpr {
    ProofExpr::NeoEvent {
        event_var: "e".into(),
        verb: verb.into(),
        roles: vec![("Agent".into(), ProofTerm::Constant(agent.into()))],
    }
}

fn predicate(name: &str, arg: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate { name: name.into(), args: vec![arg], world: None }
}

fn if_john_barks_mary_runs() -> ProofExpr {
    ProofExpr::ForAll {
        variable: "e".into(),
        body: Box::new(ProofExpr::Implies(Box::new(event("Bark", "John")), Box::new(event("Run", "Mary")))),
    }
}

#[test]
fn an_event_binder_is_abstracted_with_its_event() {
    let mut engine = BackwardChainer::new();
    engine.add_axiom(if_john_barks_mary_runs());
    assert_eq!(
        engine.knowledge_base(),
        &[ProofExpr::Implies(
            Box::new(predicate("bark", ProofTerm::Constant("John".into()))),
            Box::new(predicate("run", ProofTerm::Constant("Mary".into()))),
        )]
    );
}

#[test]
fn a_conditional_between_events_licenses_modus_ponens() {
    let mut engine = BackwardChainer::new();
    engine.add_axiom(if_john_barks_mary_runs());
    engine.add_axiom(event("Bark", "John"));
    assert!(engine.prove(event("Run", "Mary")).is_ok(), "John barks, yet Mary does not run");
}

/// A binder of an individual stays: `∀x(dog(x) → bark(x))` quantifies over dogs.
#[test]
fn a_binder_of_an_individual_stays() {
    let every_dog_barks = ProofExpr::ForAll {
        variable: "x".into(),
        body: Box::new(ProofExpr::Implies(
            Box::new(predicate("dog", ProofTerm::Variable("x".into()))),
            Box::new(predicate("bark", ProofTerm::Variable("x".into()))),
        )),
    };
    let mut engine = BackwardChainer::new();
    engine.add_axiom(every_dog_barks.clone());
    assert_eq!(engine.knowledge_base(), &[every_dog_barks]);
}

/// An event variable the abstraction keeps — one still named inside its body — keeps its binder.
#[test]
fn an_event_binder_still_in_use_stays() {
    let still_used = ProofExpr::ForAll {
        variable: "e".into(),
        body: Box::new(ProofExpr::Implies(
            Box::new(predicate("loud", ProofTerm::Variable("e".into()))),
            Box::new(predicate("heard", ProofTerm::Variable("e".into()))),
        )),
    };
    let mut engine = BackwardChainer::new();
    engine.add_axiom(still_used.clone());
    assert_eq!(engine.knowledge_base(), &[still_used]);
}
