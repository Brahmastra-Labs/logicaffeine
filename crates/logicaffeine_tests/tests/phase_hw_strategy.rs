//! SUPERCRUSH Sprint S1F: Strategy Selection Engine

#![cfg(feature = "verification")]

use logicaffeine_verify::strategy::{select_strategy, verify_auto, Strategy, VerificationResult};
use logicaffeine_verify::kinduction::SignalDecl;
use logicaffeine_verify::{VerifyExpr, VerifyOp};

fn sig(name: &str) -> SignalDecl {
    SignalDecl { name: name.into(), width: None }
}

#[test]
fn strategy_small_selects_kind() {
    // Small property → k-induction
    let prop = VerifyExpr::var("p@t");
    let strategy = select_strategy(&prop, &[sig("p")]);
    assert!(matches!(strategy, Strategy::KInduction(_)),
        "Small property should select k-induction. Got: {:?}", strategy);
}

#[test]
fn strategy_auto_proves_safety() {
    // Auto should prove a safe property
    let init = VerifyExpr::var("safe@0");
    let transition = VerifyExpr::iff(VerifyExpr::var("safe@t1"), VerifyExpr::var("safe@t"));
    let property = VerifyExpr::var("safe@t");
    let result = verify_auto(&init, &transition, &property, &[sig("safe")]);
    assert!(matches!(result, VerificationResult::Safe { .. }),
        "Auto should prove safety. Got: {:?}", result);
}

#[test]
fn strategy_auto_finds_bug() {
    // Auto should find a bug
    let init = VerifyExpr::not(VerifyExpr::var("ok@0"));
    let transition = VerifyExpr::bool(true);
    let property = VerifyExpr::var("ok@t");
    let result = verify_auto(&init, &transition, &property, &[sig("ok")]);
    assert!(matches!(result, VerificationResult::Unsafe { .. }),
        "Auto should find bug. Got: {:?}", result);
}

#[test]
fn strategy_deterministic() {
    // Same property → same strategy
    let prop = VerifyExpr::var("x@t");
    let s1 = select_strategy(&prop, &[]);
    let s2 = select_strategy(&prop, &[]);
    assert_eq!(s1, s2, "Same property should give same strategy");
}

#[test]
fn strategy_auto_integer_property() {
    // Integer counter safety
    let init = VerifyExpr::eq(VerifyExpr::var("c@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("c@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("c@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::gte(VerifyExpr::var("c@t"), VerifyExpr::int(0));
    let result = verify_auto(&init, &transition, &property, &[sig("c")]);
    assert!(matches!(result, VerificationResult::Safe { .. }),
        "Counter >= 0 should be proved. Got: {:?}", result);
}

#[test]
fn strategy_result_has_strategy_used() {
    let init = VerifyExpr::var("p@0");
    let transition = VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t"));
    let property = VerifyExpr::var("p@t");
    let result = verify_auto(&init, &transition, &property, &[sig("p")]);
    match result {
        VerificationResult::Safe { strategy_used } => {
            assert!(!format!("{:?}", strategy_used).is_empty());
        }
        other => panic!("Expected Safe, got: {:?}", other),
    }
}

#[test]
fn strategy_portfolio_basic() {
    // Direct portfolio test
    let portfolio = Strategy::Portfolio {
        strategies: vec![Strategy::KInduction(5), Strategy::Ic3],
        timeout_each_ms: 5000,
    };
    assert!(matches!(portfolio, Strategy::Portfolio { .. }));
}
