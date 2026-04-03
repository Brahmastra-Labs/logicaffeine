//! SUPERCRUSH Sprint S1E: Assume-Guarantee Compositional Reasoning

#![cfg(feature = "verification")]

use logicaffeine_verify::compositional::{verify_compositional, ComponentSpec, CompositionalResult};
use logicaffeine_verify::{VerifyExpr, VerifyOp};

#[test]
fn comp_single_module() {
    let comp = ComponentSpec {
        name: "producer".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("valid@t")],
        init: VerifyExpr::var("valid@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("valid@t1"), VerifyExpr::var("valid@t")),
    };
    let result = verify_compositional(&[comp]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "Single verified component. Got: {:?}", result);
}

#[test]
fn comp_two_modules() {
    let producer = ComponentSpec {
        name: "producer".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("data_valid@t")],
        init: VerifyExpr::var("data_valid@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("data_valid@t1"), VerifyExpr::var("data_valid@t")),
    };
    let consumer = ComponentSpec {
        name: "consumer".into(),
        assumes: vec![VerifyExpr::var("data_valid@t")],
        guarantees: vec![VerifyExpr::var("output_ok@t")],
        init: VerifyExpr::var("output_ok@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("output_ok@t1"), VerifyExpr::var("data_valid@t")),
    };
    let result = verify_compositional(&[producer, consumer]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "Producer-consumer should verify. Got: {:?}", result);
}

#[test]
fn comp_failed_component_identified() {
    let bad_comp = ComponentSpec {
        name: "broken".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("always_true@t")],
        init: VerifyExpr::not(VerifyExpr::var("always_true@0")),
        transition: VerifyExpr::bool(true),
    };
    let result = verify_compositional(&[bad_comp]);
    match result {
        CompositionalResult::ComponentFailed { name, .. } => {
            assert_eq!(name, "broken");
        }
        other => panic!("Expected ComponentFailed, got: {:?}", other),
    }
}

#[test]
fn comp_empty_components() {
    let result = verify_compositional(&[]);
    assert!(matches!(result, CompositionalResult::AllVerified));
}

#[test]
fn comp_three_chain() {
    let a = ComponentSpec {
        name: "A".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("a_out@t")],
        init: VerifyExpr::var("a_out@0"),
        transition: VerifyExpr::var("a_out@t1"),
    };
    let b = ComponentSpec {
        name: "B".into(),
        assumes: vec![VerifyExpr::var("a_out@t")],
        guarantees: vec![VerifyExpr::var("b_out@t")],
        init: VerifyExpr::var("b_out@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("b_out@t1"), VerifyExpr::var("a_out@t")),
    };
    let c = ComponentSpec {
        name: "C".into(),
        assumes: vec![VerifyExpr::var("b_out@t")],
        guarantees: vec![VerifyExpr::var("c_out@t")],
        init: VerifyExpr::var("c_out@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("c_out@t1"), VerifyExpr::var("b_out@t")),
    };
    let result = verify_compositional(&[a, b, c]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "Three-chain should verify. Got: {:?}", result);
}

#[test]
fn comp_guarantee_checked() {
    // Component with guarantee that holds
    let comp = ComponentSpec {
        name: "safe".into(),
        assumes: vec![],
        guarantees: vec![
            VerifyExpr::gte(VerifyExpr::var("count@t"), VerifyExpr::int(0)),
        ],
        init: VerifyExpr::eq(VerifyExpr::var("count@0"), VerifyExpr::int(0)),
        transition: VerifyExpr::eq(
            VerifyExpr::var("count@t1"),
            VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("count@t"), VerifyExpr::int(1)),
        ),
    };
    let result = verify_compositional(&[comp]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "Counter >= 0 should verify. Got: {:?}", result);
}
