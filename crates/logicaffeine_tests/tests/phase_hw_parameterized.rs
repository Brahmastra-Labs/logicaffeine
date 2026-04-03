//! SUPERCRUSH Sprint S2F: Parameterized Verification

#![cfg(feature = "verification")]

use logicaffeine_verify::parameterized::{verify_parameterized, ParameterizedResult};
use logicaffeine_verify::{VerifyExpr, VerifyOp, VerifyType};

#[test]
fn param_universally_valid() {
    // forall N:Int. N + 0 == N → universally valid
    let property = VerifyExpr::eq(
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("N"), VerifyExpr::int(0)),
        VerifyExpr::var("N"),
    );
    let result = verify_parameterized(&property, "N", VerifyType::Int, None);
    assert!(matches!(result, ParameterizedResult::UniversallyValid),
        "N+0==N should be universally valid. Got: {:?}", result);
}

#[test]
fn param_with_constraint() {
    // forall N:Int. N > 0 → N >= 1 → valid
    let constraint = VerifyExpr::gt(VerifyExpr::var("N"), VerifyExpr::int(0));
    let property = VerifyExpr::gte(VerifyExpr::var("N"), VerifyExpr::int(1));
    let result = verify_parameterized(&property, "N", VerifyType::Int, Some(&constraint));
    assert!(matches!(result, ParameterizedResult::UniversallyValid),
        "N>0 → N>=1 should be valid. Got: {:?}", result);
}

#[test]
fn param_counterexample() {
    // forall N:Int. N > 5 → not universally valid (N=0 fails)
    let property = VerifyExpr::gt(VerifyExpr::var("N"), VerifyExpr::int(5));
    let result = verify_parameterized(&property, "N", VerifyType::Int, None);
    assert!(!matches!(result, ParameterizedResult::UniversallyValid),
        "N>5 should NOT be universally valid. Got: {:?}", result);
}

#[test]
fn param_bool_property() {
    // forall p:Bool. p OR NOT p → valid (law of excluded middle)
    let property = VerifyExpr::or(VerifyExpr::var("p"), VerifyExpr::not(VerifyExpr::var("p")));
    let result = verify_parameterized(&property, "p", VerifyType::Bool, None);
    assert!(matches!(result, ParameterizedResult::UniversallyValid),
        "p OR NOT p should be valid. Got: {:?}", result);
}

#[test]
fn param_arithmetic_identity() {
    // forall x:Int. x * 1 == x → valid
    let property = VerifyExpr::eq(
        VerifyExpr::binary(VerifyOp::Mul, VerifyExpr::var("x"), VerifyExpr::int(1)),
        VerifyExpr::var("x"),
    );
    let result = verify_parameterized(&property, "x", VerifyType::Int, None);
    assert!(matches!(result, ParameterizedResult::UniversallyValid),
        "x*1==x should be valid. Got: {:?}", result);
}

#[test]
fn param_implication_valid() {
    // forall x:Int. x > 10 → x > 5
    let constraint = VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(10));
    let property = VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5));
    let result = verify_parameterized(&property, "x", VerifyType::Int, Some(&constraint));
    assert!(matches!(result, ParameterizedResult::UniversallyValid),
        "x>10 → x>5 should be valid. Got: {:?}", result);
}
