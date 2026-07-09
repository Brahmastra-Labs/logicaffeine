//! =============================================================================
//! TELESCOPED DEFINITE DESCRIPTIONS — free anaphoric variables in axioms
//! =============================================================================
//!
//! Theorem premises share discourse state: after "The barber is a man."
//! introduces ∃x((barber(x) ∧ ∀y(barber(y) → y = x)) ∧ man(x)), a later
//! premise "The barber shaves ..." telescopes to the SAME referent and is
//! emitted with a free `x` — e.g. ∀z((man(z) ∧ ¬shave(z, z)) → shave(x, z)).
//!
//! The engine's definite-description unification must bind those free
//! anaphoric occurrences to the unified constant (`the_barber`), or the
//! telescoped premises are inert and the Barber paradox loses its derivation.

use logicaffeine_proof::{BackwardChainer, ProofExpr, ProofTerm};

fn var(s: &str) -> ProofTerm {
    ProofTerm::Variable(s.into())
}

fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.into(),
        args,
        world: None,
    }
}

/// ∃x((barber(x) ∧ ∀y(barber(y) → y = x)) ∧ man(x)) — the Russellian
/// definite description introduced by the first premise.
fn barber_description() -> ProofExpr {
    ProofExpr::Exists {
        variable: "x".into(),
        body: Box::new(ProofExpr::And(
            Box::new(ProofExpr::And(
                Box::new(pred("barber", vec![var("x")])),
                Box::new(ProofExpr::ForAll {
                    variable: "y".into(),
                    body: Box::new(ProofExpr::Implies(
                        Box::new(pred("barber", vec![var("y")])),
                        Box::new(ProofExpr::Identity(var("y"), var("x"))),
                    )),
                }),
            )),
            Box::new(pred("man", vec![var("x")])),
        )),
    }
}

/// The full telescoped Barber: premises 2 and 3 reference the description's
/// variable `x` FREE, exactly as the theorem door emits them.
#[test]
fn telescoped_free_variable_premises_prove_barber() {
    let mut engine = BackwardChainer::new();
    engine.set_max_depth(50);

    engine.add_axiom(barber_description());

    // ∀z((man(z) ∧ ¬shave(z, z)) → shave(x, z))   — x free (anaphoric)
    engine.add_axiom(ProofExpr::ForAll {
        variable: "z".into(),
        body: Box::new(ProofExpr::Implies(
            Box::new(ProofExpr::And(
                Box::new(pred("man", vec![var("z")])),
                Box::new(ProofExpr::Not(Box::new(pred(
                    "shave",
                    vec![var("z"), var("z")],
                )))),
            )),
            Box::new(pred("shave", vec![var("x"), var("z")])),
        )),
    });

    // ∀w ¬((man(w) ∧ shave(w, w)) ∧ shave(x, w))   — x free (anaphoric)
    engine.add_axiom(ProofExpr::ForAll {
        variable: "w".into(),
        body: Box::new(ProofExpr::Not(Box::new(ProofExpr::And(
            Box::new(ProofExpr::And(
                Box::new(pred("man", vec![var("w")])),
                Box::new(pred("shave", vec![var("w"), var("w")])),
            )),
            Box::new(pred("shave", vec![var("x"), var("w")])),
        )))),
    });

    // Goal: ¬∃v barber(v)
    let goal = ProofExpr::Not(Box::new(ProofExpr::Exists {
        variable: "v".into(),
        body: Box::new(pred("barber", vec![var("v")])),
    }));

    let result = engine.prove(goal);
    assert!(
        result.is_ok(),
        "telescoped Barber premises should yield a derivation: {:?}",
        result.err()
    );
}

/// Binding free anaphoric variables must NOT capture bound variables that
/// happen to share the name: ∀x(man(x) → mortal(x)) is about every x, not
/// about the barber.
#[test]
fn bound_variables_sharing_the_name_are_untouched() {
    let mut engine = BackwardChainer::new();

    engine.add_axiom(barber_description());

    // An unrelated universal whose binder shadows the description's variable.
    engine.add_axiom(ProofExpr::ForAll {
        variable: "x".into(),
        body: Box::new(ProofExpr::Implies(
            Box::new(pred("man", vec![var("x")])),
            Box::new(pred("mortal", vec![var("x")])),
        )),
    });

    // Trigger preprocessing (the goal itself is irrelevant).
    let _ = engine.prove(ProofExpr::Atom("dummy".into()));

    let shadowed_intact = engine.knowledge_base().iter().any(|axiom| {
        matches!(
            axiom,
            ProofExpr::ForAll { variable, body }
                if variable == "x"
                    && matches!(
                        body.as_ref(),
                        ProofExpr::Implies(l, r)
                            if **l == pred("man", vec![var("x")])
                                && **r == pred("mortal", vec![var("x")])
                    )
        )
    });
    assert!(
        shadowed_intact,
        "∀x(man(x) → mortal(x)) must survive unification untouched; KB: {:#?}",
        engine.knowledge_base()
    );
}
