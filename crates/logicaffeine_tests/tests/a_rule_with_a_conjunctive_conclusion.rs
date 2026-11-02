//! A rule that concludes a conjunction proves each conjunct: "Every man sees Mary and Bill" is
//! `∀x(man(x) → see(x, Mary) ∧ see(x, Bill))`, and with "Tom is a man" it proves "Tom sees
//! Bill" as surely as "Tom sees Mary" — modus ponens, then ∧-elimination. And a goal such a
//! rule does NOT entail is refused promptly: the search for a contradiction among the premises
//! (the ex falso last resort) does not re-search every ordering of the same case splits.

use logicaffeine_proof::{BackwardChainer, ProofExpr, ProofTerm};
use std::time::{Duration, Instant};

fn sees(agent: ProofTerm, theme: &str) -> ProofExpr {
    ProofExpr::NeoEvent {
        event_var: "e".into(),
        verb: "See".into(),
        roles: vec![("Agent".into(), agent), ("Theme".into(), ProofTerm::Constant(theme.into()))],
    }
}

fn predicate(name: &str, arg: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate { name: name.into(), args: vec![arg], world: None }
}

fn tom() -> ProofTerm {
    ProofTerm::Constant("Tom".into())
}

/// `∀x(man(x) → see(x, a) ∧ see(x, b) ∧ …)` — "Every man sees a, b, …".
fn every_man_sees(themes: &[&str]) -> ProofExpr {
    let x = || ProofTerm::Variable("x".into());
    let mut conclusion = sees(x(), themes[0]);
    for theme in &themes[1..] {
        conclusion = ProofExpr::And(Box::new(conclusion), Box::new(sees(x(), theme)));
    }
    ProofExpr::ForAll {
        variable: "x".into(),
        body: Box::new(ProofExpr::Implies(Box::new(predicate("man", x())), Box::new(conclusion))),
    }
}

fn engine_with(premises: Vec<ProofExpr>) -> BackwardChainer {
    let mut engine = BackwardChainer::new();
    for premise in premises {
        engine.add_axiom(premise);
    }
    engine
}

#[test]
fn a_universal_rule_proves_each_conjunct_of_its_conclusion() {
    for theme in ["Mary", "Bill"] {
        let mut engine = engine_with(vec![every_man_sees(&["Mary", "Bill"]), predicate("man", tom())]);
        assert!(engine.prove(sees(tom(), theme)).is_ok(), "Tom is a man, so Tom sees {theme}");
    }
    let mut engine = engine_with(vec![every_man_sees(&["Mary", "Bill", "Sue"]), predicate("man", tom())]);
    assert!(engine.prove(sees(tom(), "Sue")).is_ok(), "the last of three conjuncts");
}

#[test]
fn a_ground_rule_proves_each_conjunct_of_its_conclusion() {
    let fido = || ProofTerm::Constant("Fido".into());
    let rule = ProofExpr::Implies(
        Box::new(predicate("dog", fido())),
        Box::new(ProofExpr::And(Box::new(predicate("bark", fido())), Box::new(predicate("run", fido())))),
    );
    let mut engine = engine_with(vec![rule, predicate("dog", fido())]);
    assert!(engine.prove(predicate("run", fido())).is_ok(), "Fido is a dog, so Fido runs");
}

#[test]
fn the_rule_proves_nothing_its_conclusion_does_not_say() {
    let mut engine = engine_with(vec![every_man_sees(&["Mary", "Bill"]), predicate("man", tom())]);
    assert!(engine.prove(sees(tom(), "Sue")).is_err(), "no premise says Tom sees Sue");
    let mut engine = engine_with(vec![every_man_sees(&["Mary", "Bill"])]);
    assert!(engine.prove(sees(tom(), "Bill")).is_err(), "nothing says Tom is a man");
}

/// The refusal above took 194 s before the ⊥ search remembered the fact sets it had already
/// failed to refute; it is milliseconds now. The bound is generous so a loaded machine cannot
/// flake it — it is here to catch the exponential search coming back, not to time the prover.
#[test]
fn an_unentailed_goal_is_refused_promptly() {
    for themes in [&["Mary", "Bill"][..], &["Mary", "Bill", "Sue"][..]] {
        let mut engine = engine_with(vec![every_man_sees(themes), predicate("man", tom())]);
        let started = Instant::now();
        assert!(engine.prove(sees(tom(), "Ann")).is_err());
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "refusing took {:?} with {} objects",
            started.elapsed(),
            themes.len()
        );
    }
}

/// "A man sees Mary and Bill" is `∃x(man(x) ∧ see(x, Mary) ∧ see(x, Bill))`; opening it names a
/// witness, and that witness is the one "Some man sees Bill" is about.
#[test]
fn an_opened_existential_witnesses_an_existential_goal() {
    let x = || ProofTerm::Variable("x".into());
    let a_man_sees_both = ProofExpr::Exists {
        variable: "x".into(),
        body: Box::new(ProofExpr::And(
            Box::new(predicate("man", x())),
            Box::new(ProofExpr::And(Box::new(sees(x(), "Mary")), Box::new(sees(x(), "Bill")))),
        )),
    };
    let some_man_sees = |theme: &str| ProofExpr::Exists {
        variable: "y".into(),
        body: Box::new(ProofExpr::And(
            Box::new(predicate("man", ProofTerm::Variable("y".into()))),
            Box::new(sees(ProofTerm::Variable("y".into()), theme)),
        )),
    };
    for theme in ["Mary", "Bill"] {
        let mut engine = engine_with(vec![a_man_sees_both.clone()]);
        assert!(engine.prove(some_man_sees(theme)).is_ok(), "some man sees {theme}");
    }
    let mut engine = engine_with(vec![a_man_sees_both]);
    assert!(engine.prove(some_man_sees("Sue")).is_err());
}
