//! Regression pin for Bug Report #1, BUG-013.
//!
//! The proof-kernel beta reduction must be capture-avoiding: a free variable in
//! the argument must not be captured by an inner binder of the function body.

use logicaffeine_proof::unify::beta_reduce;
use logicaffeine_proof::{ProofExpr, ProofTerm};

#[test]
fn beta_reduction_must_alpha_rename_to_avoid_capture() {
    // (λx. ∀y. Loves(x, y))  applied to the FREE variable y.
    // The outer y is FREE in the argument; the inner ∀y is a DIFFERENT binder.
    let redex = ProofExpr::App(
        Box::new(ProofExpr::Lambda {
            variable: "x".to_string(),
            body: Box::new(ProofExpr::ForAll {
                variable: "y".to_string(),
                body: Box::new(ProofExpr::Predicate {
                    name: "Loves".to_string(),
                    args: vec![
                        ProofTerm::Variable("x".to_string()),
                        ProofTerm::Variable("y".to_string()),
                    ],
                    world: None,
                }),
            }),
        }),
        Box::new(ProofExpr::Term(ProofTerm::Variable("y".to_string()))),
    );

    let reduced = beta_reduce(&redex);

    // NEGATIVE: the buggy kernel produces exactly "∀y Loves(y, y)" (capture).
    let out = format!("{}", reduced);
    assert_ne!(
        out, "∀y Loves(y, y)",
        "free variable y was captured by the inner ∀y: {}",
        out
    );

    // POSITIVE / structural: after a capture-avoiding reduction the result is a
    // ForAll whose Loves predicate has TWO DISTINCT arguments — the first is the
    // substituted free `y`, the second is the (renamed) bound variable.
    match &reduced {
        ProofExpr::ForAll { variable, body } => match body.as_ref() {
            ProofExpr::Predicate { name, args, .. } => {
                assert_eq!(name, "Loves");
                assert_eq!(args.len(), 2);
                let first = format!("{}", args[0]); // must be the free "y"
                let second = format!("{}", args[1]); // must be the bound binder
                assert_ne!(
                    first, second,
                    "the two arguments of Loves were collapsed to one variable by capture: \
                     first={}, second={}",
                    first, second
                );
                assert_ne!(
                    variable, "y",
                    "inner binder still named y, capturing the free argument y"
                );
                assert_eq!(first, "y", "first arg must remain the free outer y");
                assert_eq!(
                    &second, variable,
                    "second arg must be the (renamed) bound variable {}",
                    variable
                );
            }
            other => panic!("expected ForAll over a Loves predicate, got body {:?}", other),
        },
        other => panic!("expected the reduct to be a ForAll, got {:?}", other),
    }
}
