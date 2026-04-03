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
