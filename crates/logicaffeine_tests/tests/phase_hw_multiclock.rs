//! SUPERCRUSH Sprint S2A: Multi-Clock Domain Modeling

#![cfg(feature = "verification")]

use logicaffeine_verify::multiclock::{verify_multiclock, MultiClockModel, MultiClockResult, ClockDomain};
use logicaffeine_verify::VerifyExpr;
use std::collections::HashMap;

#[test]
fn multiclock_single_domain_fallback() {
    let mut transitions = HashMap::new();
    transitions.insert("clk".into(), VerifyExpr::iff(
        VerifyExpr::var("p@t1"), VerifyExpr::var("p@t"),
    ));
    let model = MultiClockModel {
        domains: vec![ClockDomain { name: "clk".into(), frequency: Some(100_000_000), ratio: None }],
        init: VerifyExpr::var("p@0"),
        transitions,
        property: VerifyExpr::var("p@t"),
    };
    let result = verify_multiclock(&model, 5);
    assert!(matches!(result, MultiClockResult::Safe),
        "Single domain should fall back to standard. Got: {:?}", result);
}

#[test]
fn multiclock_two_domains() {
    let mut transitions = HashMap::new();
    transitions.insert("fast".into(), VerifyExpr::var("f@t1"));
    transitions.insert("slow".into(), VerifyExpr::var("s@t1"));
    let model = MultiClockModel {
        domains: vec![
            ClockDomain { name: "fast".into(), frequency: Some(200_000_000), ratio: Some((2, 1)) },
            ClockDomain { name: "slow".into(), frequency: Some(100_000_000), ratio: Some((1, 1)) },
        ],
        init: VerifyExpr::and(VerifyExpr::var("f@0"), VerifyExpr::var("s@0")),
        transitions,
        property: VerifyExpr::var("f@t"),
    };
    let result = verify_multiclock(&model, 5);
    assert!(matches!(result, MultiClockResult::Safe),
        "Two-domain safe property. Got: {:?}", result);
}

#[test]
fn multiclock_unsafe() {
    let mut transitions = HashMap::new();
    transitions.insert("clk".into(), VerifyExpr::not(VerifyExpr::var("ok@t1")));
    let model = MultiClockModel {
        domains: vec![ClockDomain { name: "clk".into(), frequency: None, ratio: None }],
        init: VerifyExpr::var("ok@0"),
        transitions,
        property: VerifyExpr::var("ok@t"),
    };
    let result = verify_multiclock(&model, 5);
    assert!(matches!(result, MultiClockResult::Unsafe { .. }),
        "Transition to false should be unsafe. Got: {:?}", result);
}

#[test]
fn multiclock_no_domains_safe() {
    let model = MultiClockModel {
        domains: vec![],
        init: VerifyExpr::bool(true),
        transitions: HashMap::new(),
        property: VerifyExpr::bool(true),
    };
    let result = verify_multiclock(&model, 3);
    assert!(matches!(result, MultiClockResult::Safe),
        "Empty model should be safe. Got: {:?}", result);
}

#[test]
fn multiclock_three_domains() {
    let mut transitions = HashMap::new();
    transitions.insert("a".into(), VerifyExpr::var("x@t1"));
    transitions.insert("b".into(), VerifyExpr::var("y@t1"));
    transitions.insert("c".into(), VerifyExpr::var("z@t1"));
    let model = MultiClockModel {
        domains: vec![
            ClockDomain { name: "a".into(), frequency: None, ratio: None },
            ClockDomain { name: "b".into(), frequency: None, ratio: None },
            ClockDomain { name: "c".into(), frequency: None, ratio: None },
        ],
        init: VerifyExpr::and(VerifyExpr::var("x@0"), VerifyExpr::and(VerifyExpr::var("y@0"), VerifyExpr::var("z@0"))),
        transitions,
        property: VerifyExpr::var("x@t"),
    };
    let result = verify_multiclock(&model, 3);
    assert!(matches!(result, MultiClockResult::Safe),
        "Three-domain safe. Got: {:?}", result);
}
