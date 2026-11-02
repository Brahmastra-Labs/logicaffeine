use super::*;
use crate::MatchArm;

/// The number of `ProofExpr` variants [`variant`] tells apart.
const VARIANTS: usize = 24;

/// Which variant `expr` is. The match is exhaustive, so a new variant does not compile until
/// it has an index here — and then [`one_of_every_variant`] a sample and `VARIANTS` its count.
fn variant(expr: &ProofExpr) -> usize {
    match expr {
        ProofExpr::Predicate { .. } => 0,
        ProofExpr::Identity(..) => 1,
        ProofExpr::Atom(_) => 2,
        ProofExpr::And(..) => 3,
        ProofExpr::Or(..) => 4,
        ProofExpr::Implies(..) => 5,
        ProofExpr::Iff(..) => 6,
        ProofExpr::Not(_) => 7,
        ProofExpr::ForAll { .. } => 8,
        ProofExpr::Exists { .. } => 9,
        ProofExpr::Modal { .. } => 10,
        ProofExpr::Counterfactual { .. } => 11,
        ProofExpr::Temporal { .. } => 12,
        ProofExpr::TemporalBinary { .. } => 13,
        ProofExpr::Lambda { .. } => 14,
        ProofExpr::App(..) => 15,
        ProofExpr::NeoEvent { .. } => 16,
        ProofExpr::Ctor { .. } => 17,
        ProofExpr::Match { .. } => 18,
        ProofExpr::Fixpoint { .. } => 19,
        ProofExpr::TypedVar { .. } => 20,
        ProofExpr::Hole(_) => 21,
        ProofExpr::Term(_) => 22,
        ProofExpr::Unsupported(_) => 23,
    }
}

fn one_of_every_variant() -> Vec<ProofExpr> {
    let rex = || ProofTerm::Constant("Rex".into());
    let dog = || ProofExpr::Predicate { name: "dog".into(), args: vec![rex()], world: None };
    let bark = || ProofExpr::Predicate { name: "bark".into(), args: vec![rex()], world: None };
    let boxed = |e: ProofExpr| Box::new(e);
    vec![
        dog(),
        ProofExpr::Identity(rex(), ProofTerm::Constant("Fido".into())),
        ProofExpr::Atom("P".into()),
        ProofExpr::And(boxed(dog()), boxed(bark())),
        ProofExpr::Or(boxed(dog()), boxed(bark())),
        ProofExpr::Implies(boxed(dog()), boxed(bark())),
        ProofExpr::Iff(boxed(dog()), boxed(bark())),
        ProofExpr::Not(boxed(dog())),
        ProofExpr::ForAll {
            variable: "x".into(),
            body: boxed(ProofExpr::Predicate {
                name: "dog".into(),
                args: vec![ProofTerm::Variable("x".into())],
                world: None,
            }),
        },
        ProofExpr::Exists {
            variable: "x".into(),
            body: boxed(ProofExpr::Predicate {
                name: "dog".into(),
                args: vec![ProofTerm::Variable("x".into())],
                world: None,
            }),
        },
        ProofExpr::Modal { domain: "Alethic".into(), force: 1.0, flavor: "Root".into(), body: boxed(dog()) },
        ProofExpr::Counterfactual { antecedent: boxed(dog()), consequent: boxed(bark()) },
        ProofExpr::Temporal { operator: "Past".into(), body: boxed(bark()) },
        ProofExpr::TemporalBinary { operator: "Until".into(), left: boxed(dog()), right: boxed(bark()) },
        ProofExpr::Lambda { variable: "x".into(), body: boxed(dog()) },
        ProofExpr::App(boxed(dog()), boxed(bark())),
        ProofExpr::NeoEvent {
            event_var: "e".into(),
            verb: "Bark".into(),
            roles: vec![("Agent".into(), rex())],
        },
        ProofExpr::Ctor {
            name: "Succ".into(),
            args: vec![ProofExpr::Ctor { name: "Zero".into(), args: vec![] }],
        },
        ProofExpr::Match {
            scrutinee: boxed(ProofExpr::Term(ProofTerm::Variable("n".into()))),
            arms: vec![MatchArm { ctor: "Zero".into(), bindings: vec![], body: dog() }],
        },
        ProofExpr::Fixpoint { name: "f".into(), body: boxed(dog()) },
        ProofExpr::TypedVar { name: "n".into(), typename: "Nat".into() },
        ProofExpr::Hole("P".into()),
        ProofExpr::Term(rex()),
        ProofExpr::Unsupported("an unparsed construct".into()),
    ]
}

#[test]
fn the_samples_cover_every_variant() {
    let covered: std::collections::BTreeSet<usize> = one_of_every_variant().iter().map(variant).collect();
    assert_eq!(covered, (0..VARIANTS).collect(), "a variant has no sample");
}

/// A fact the search derived is the fact it already knew, whatever connective, quantifier,
/// tense or modality it is built from — or `P` never meets `¬P`.
#[test]
fn every_expression_is_structurally_equal_to_itself() {
    for sample in one_of_every_variant() {
        if matches!(sample, ProofExpr::Unsupported(_)) {
            continue;
        }
        assert!(exprs_structurally_equal(&sample, &sample.clone()), "{sample:?} is not equal to itself");
    }
}

/// A construct the pipeline could not read stands for nothing in particular, so it matches
/// no fact, not even another spelled the same.
#[test]
fn an_unsupported_construct_equals_nothing() {
    let unsupported = ProofExpr::Unsupported("an unparsed construct".into());
    assert!(!exprs_structurally_equal(&unsupported, &unsupported.clone()));
}

#[test]
fn expressions_of_different_variants_are_unequal() {
    let samples = one_of_every_variant();
    for (i, left) in samples.iter().enumerate() {
        for (j, right) in samples.iter().enumerate() {
            if i != j {
                assert!(!exprs_structurally_equal(left, right), "{left:?} equals {right:?}");
            }
        }
    }
}

/// Two tensed propositions are one only when both the tense and the proposition agree.
#[test]
fn a_tense_is_part_of_structural_equality() {
    let ran = |who: &str| ProofExpr::Temporal {
        operator: "Past".into(),
        body: Box::new(ProofExpr::Predicate {
            name: "run".into(),
            args: vec![ProofTerm::Constant(who.into())],
            world: None,
        }),
    };
    let will_run = ProofExpr::Temporal {
        operator: "Future".into(),
        body: Box::new(ProofExpr::Predicate {
            name: "run".into(),
            args: vec![ProofTerm::Constant("Rex".into())],
            world: None,
        }),
    };
    assert!(exprs_structurally_equal(&ran("Rex"), &ran("Rex")));
    assert!(!exprs_structurally_equal(&ran("Rex"), &ran("Fido")));
    assert!(!exprs_structurally_equal(&ran("Rex"), &will_run));
}
