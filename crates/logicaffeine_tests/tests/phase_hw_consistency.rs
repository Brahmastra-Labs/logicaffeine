//! Sprint 6B: Multi-Property Consistency Checking

#![cfg(feature = "verification")]

use logicaffeine_verify::consistency::{check_consistency, ConsistencyResult};
use logicaffeine_verify::ir::{VerifyExpr, VerifyOp};

#[test]
fn consistent_compatible_properties() {
    // P and Q are independent — should be consistent
    let props = vec![
        VerifyExpr::Var("req@0".into()),
        VerifyExpr::Var("ack@0".into()),
    ];
    let result = check_consistency(&props, &["req".into(), "ack".into()], 1);
    assert!(matches!(result, ConsistencyResult::Consistent),
        "Independent properties should be consistent. Got: {:?}", result);
}

#[test]
fn inconsistent_p_and_not_p() {
    // P and ¬P — contradiction
    let props = vec![
        VerifyExpr::Var("p@0".into()),
        VerifyExpr::not(VerifyExpr::Var("p@0".into())),
    ];
    let result = check_consistency(&props, &["p".into()], 1);
    assert!(matches!(result, ConsistencyResult::Inconsistent { .. }),
        "P and ¬P should be inconsistent. Got: {:?}", result);
}

#[test]
fn inconsistent_identifies_conflicting_pair() {
    let props = vec![
        VerifyExpr::Var("p@0".into()),
        VerifyExpr::not(VerifyExpr::Var("p@0".into())),
    ];
    let result = check_consistency(&props, &["p".into()], 1);
    if let ConsistencyResult::Inconsistent { conflicting, .. } = result {
        assert!(conflicting.contains(&(0, 1)),
            "Should identify pair (0,1). Got: {:?}", conflicting);
    } else {
        panic!("Expected Inconsistent");
    }
}

#[test]
fn consistent_empty_set() {
    let result = check_consistency(&[], &[], 1);
    assert!(matches!(result, ConsistencyResult::Consistent),
        "Empty set should be consistent");
}

#[test]
fn consistent_single_property() {
    let props = vec![VerifyExpr::Var("p@0".into())];
    let result = check_consistency(&props, &["p".into()], 1);
    assert!(matches!(result, ConsistencyResult::Consistent),
        "Single property should be consistent");
}

#[test]
fn inconsistent_mutex_forced_overlap() {
    // ¬(a ∧ b) AND (a ∧ b) — direct contradiction
    let mutex = VerifyExpr::not(VerifyExpr::and(
        VerifyExpr::Var("a@0".into()),
        VerifyExpr::Var("b@0".into()),
    ));
    let overlap = VerifyExpr::and(
        VerifyExpr::Var("a@0".into()),
        VerifyExpr::Var("b@0".into()),
    );
    let props = vec![mutex, overlap];
    let result = check_consistency(&props, &["a".into(), "b".into()], 1);
    assert!(matches!(result, ConsistencyResult::Inconsistent { .. }),
        "Mutex + forced overlap should be inconsistent. Got: {:?}", result);
}

#[test]
fn consistent_implication_pair() {
    // (p → q) AND (q → r) — logically compatible
    let p1 = VerifyExpr::implies(
        VerifyExpr::Var("p@0".into()),
        VerifyExpr::Var("q@0".into()),
    );
    let p2 = VerifyExpr::implies(
        VerifyExpr::Var("q@0".into()),
        VerifyExpr::Var("r@0".into()),
    );
    let props = vec![p1, p2];
    let result = check_consistency(&props, &["p".into(), "q".into(), "r".into()], 1);
    assert!(matches!(result, ConsistencyResult::Consistent),
        "Implication chain should be consistent. Got: {:?}", result);
}
