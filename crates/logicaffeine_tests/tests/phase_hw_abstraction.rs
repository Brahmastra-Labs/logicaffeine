//! SUPERCRUSH Sprint S3D: Automatic Abstraction/CEGAR

#![cfg(feature = "verification")]

use logicaffeine_verify::abstraction::*;
use logicaffeine_verify::{VerifyExpr, VerifyOp};

#[test]
fn abstract_model_creates_predicates() {
    let init = VerifyExpr::eq(VerifyExpr::var("x@0"), VerifyExpr::int(0));
    let preds = vec![VerifyExpr::gte(VerifyExpr::var("x@t"), VerifyExpr::int(0))];
    let model = abstract_model(&init, &preds);
    assert_eq!(model.predicates.len(), 1);
}

#[test]
fn abstract_model_empty_predicates() {
    let init = VerifyExpr::bool(true);
    let model = abstract_model(&init, &[]);
    assert!(model.predicates.is_empty());
}

#[test]
fn cegar_safe_property() {
    let init = VerifyExpr::eq(VerifyExpr::var("c@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("c@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("c@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::gte(VerifyExpr::var("c@t"), VerifyExpr::int(0));
    let result = cegar_verify(&init, &transition, &property, &[], 5);
    assert!(matches!(result, AbstractionResult::Safe),
        "Counter >= 0 should be safe. Got: {:?}", result);
}

#[test]
fn cegar_unsafe_property() {
    let init = VerifyExpr::eq(VerifyExpr::var("c@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("c@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("c@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::lt(VerifyExpr::var("c@t"), VerifyExpr::int(3));
    let result = cegar_verify(&init, &transition, &property, &[property.clone()], 5);
    assert!(matches!(result, AbstractionResult::Unsafe { .. }),
        "Counter < 3 should be unsafe. Got: {:?}", result);
}

#[test]
fn cegar_spurious_detected() {
    // Property that fails but could be refined
    let init = VerifyExpr::not(VerifyExpr::var("p@0"));
    let transition = VerifyExpr::bool(true);
    let property = VerifyExpr::var("p@t");
    let result = cegar_verify(&init, &transition, &property, &[], 3);
    assert!(matches!(result, AbstractionResult::SpuriousRefined { .. } | AbstractionResult::Unsafe { .. }),
        "Should detect counterexample. Got: {:?}", result);
}

#[test]
fn cegar_max_refinements_respected() {
    let init = VerifyExpr::bool(true);
    let transition = VerifyExpr::bool(true);
    let property = VerifyExpr::bool(true); // trivially true
    let result = cegar_verify(&init, &transition, &property, &[], 0);
    assert!(matches!(result, AbstractionResult::Unknown | AbstractionResult::Safe),
        "Max refinements 0 should be handled. Got: {:?}", result);
}

#[test]
fn cegar_uses_abstract_model() {
    // System: counter starts at 0, increments by 1 each step.
    // Property: counter >= 0 (safe, provable via predicate abstraction).
    //
    // We provide a predicate p: (c >= 0). The abstract model should use
    // an abstract transition (not the concrete one) for its initial check.
    // Specifically, the AbstractModel returned by abstract_model should have
    // an abstract_transition that references the predicates, not Bool(true).
    //
    // The CEGAR loop should build and verify the abstract model first.
    // If the abstract model proves safety, it returns Safe without needing
    // to fall back to concrete model checking at all.
    let init = VerifyExpr::eq(VerifyExpr::var("c@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("c@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("c@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::gte(VerifyExpr::var("c@t"), VerifyExpr::int(0));

    // Predicate: c >= 0
    let pred = VerifyExpr::gte(VerifyExpr::var("c@t"), VerifyExpr::int(0));

    // Build the abstract model (with transition for predicate abstraction)
    let model = abstract_model_full(&init, &transition, &[pred.clone()]);

    // The abstract transition must NOT be trivially true.
    // A real predicate abstraction computes how predicates evolve under the transition.
    assert_ne!(
        model.abstract_transition,
        VerifyExpr::bool(true),
        "Abstract transition must not be trivially Bool(true) — it should encode \
         the transition relation restricted to the predicate space"
    );

    // Verify via CEGAR — should be safe
    let result = cegar_verify(&init, &transition, &property, &[pred], 5);
    assert!(
        matches!(result, AbstractionResult::Safe),
        "Counter >= 0 should be safe via abstract model. Got: {:?}", result
    );
}

#[test]
fn cegar_refinement_adds_new_predicate() {
    // System: a boolean that starts false and stays false.
    // Property: p is true. This fails on the concrete model.
    //
    // When CEGAR finds a counterexample, refinement should add predicates
    // that are DIFFERENT from the original property. The old implementation
    // just pushed property.clone() as the "new predicate", which is useless.
    let init = VerifyExpr::not(VerifyExpr::var("p@0"));
    let transition = VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t"));
    let property = VerifyExpr::var("p@t");

    let result = cegar_verify(&init, &transition, &property, &[], 3);
    match &result {
        AbstractionResult::SpuriousRefined { new_predicates } => {
            // The new predicates must not just be [property].
            // They should contain predicates that help distinguish
            // abstract from concrete counterexamples.
            for pred in new_predicates {
                assert_ne!(
                    pred, &property,
                    "Refinement must add predicates different from the property itself. \
                     Adding the property as a predicate is not real refinement."
                );
            }
            assert!(
                !new_predicates.is_empty(),
                "Refinement should produce at least one new predicate"
            );
        }
        AbstractionResult::Unsafe { .. } => {
            // Also acceptable: the counterexample is real, so Unsafe is correct.
            // The init violates the property directly.
        }
        other => {
            panic!(
                "Expected SpuriousRefined or Unsafe for a property that fails. Got: {:?}",
                other
            );
        }
    }
}

#[test]
fn abstract_transition_not_trivial() {
    // Two-variable system: x starts at 0, y starts at 1.
    // Transition: x' = x + 1, y' = y + x.
    // Predicates: (x >= 0) and (y >= 1).
    //
    // The abstract transition should encode how these predicates evolve
    // under the concrete transition. It must NOT be Bool(true).
    let init = VerifyExpr::and(
        VerifyExpr::eq(VerifyExpr::var("x@0"), VerifyExpr::int(0)),
        VerifyExpr::eq(VerifyExpr::var("y@0"), VerifyExpr::int(1)),
    );
    let transition = VerifyExpr::and(
        VerifyExpr::eq(
            VerifyExpr::var("x@t1"),
            VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("x@t"), VerifyExpr::int(1)),
        ),
        VerifyExpr::eq(
            VerifyExpr::var("y@t1"),
            VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("y@t"), VerifyExpr::var("x@t")),
        ),
    );
    let predicates = vec![
        VerifyExpr::gte(VerifyExpr::var("x@t"), VerifyExpr::int(0)),
        VerifyExpr::gte(VerifyExpr::var("y@t"), VerifyExpr::int(1)),
    ];

    let model = abstract_model_full(&init, &transition, &predicates);

    // The abstract transition must not be Bool(true)
    assert_ne!(
        model.abstract_transition,
        VerifyExpr::bool(true),
        "Abstract transition for a non-trivial system with predicates must not be Bool(true)"
    );

    // The abstract transition should reference predicate-related variables
    // (the abstract boolean variables that represent predicate truth values)
    let trans_str = format!("{:?}", model.abstract_transition);
    assert!(
        trans_str.contains("@t") || trans_str.contains("pred_"),
        "Abstract transition should reference timestep or predicate variables, got: {}",
        trans_str
    );
}
