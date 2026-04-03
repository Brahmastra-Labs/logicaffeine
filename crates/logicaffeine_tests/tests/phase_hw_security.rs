//! SUPERCRUSH Sprint S2D: Security Non-Interference

#![cfg(feature = "verification")]

use logicaffeine_verify::security::{check_non_interference, SecurityResult, SecurityLabel, TaintedSignal};
use logicaffeine_verify::VerifyExpr;

#[test]
fn security_simple_noninterference() {
    // Public output independent of secret → NonInterference
    let transition = VerifyExpr::iff(
        VerifyExpr::var("output@1"),
        VerifyExpr::var("input@0"),
    );
    let signals = vec![
        TaintedSignal { name: "input".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "output".into(), label: SecurityLabel::Public },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::NonInterference),
        "No secrets → non-interference. Got: {:?}", result);
}

#[test]
fn security_no_secret_signals() {
    // All public → trivially NonInterference
    let transition = VerifyExpr::bool(true);
    let signals = vec![
        TaintedSignal { name: "a".into(), label: SecurityLabel::Public },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::NonInterference));
}

#[test]
fn security_direct_leak() {
    // output = secret XOR mask → InformationLeak
    let transition = VerifyExpr::iff(
        VerifyExpr::var("output@1"),
        VerifyExpr::var("secret@0"),
    );
    let signals = vec![
        TaintedSignal { name: "output".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "secret".into(), label: SecurityLabel::Secret },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::InformationLeak { .. }),
        "Secret directly to output should leak. Got: {:?}", result);
}

#[test]
fn security_taint_propagation() {
    // Secret influences output through intermediate
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("mid@1"), VerifyExpr::var("secret@0")),
        VerifyExpr::iff(VerifyExpr::var("output@1"), VerifyExpr::var("mid@1")),
    );
    let signals = vec![
        TaintedSignal { name: "output".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "mid".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "secret".into(), label: SecurityLabel::Secret },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::InformationLeak { .. }),
        "Transitive taint should be detected. Got: {:?}", result);
}

#[test]
fn security_constant_output_no_leak() {
    // Output is always true, independent of any input
    // With only a secret input, there should be no leak since
    // the transition doesn't reference the secret at all
    let transition = VerifyExpr::bool(true); // constant transition
    let signals = vec![
        TaintedSignal { name: "output".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "secret".into(), label: SecurityLabel::Secret },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::NonInterference),
        "Constant transition should not leak. Got: {:?}", result);
}

#[test]
fn security_leak_path_reported() {
    let transition = VerifyExpr::iff(VerifyExpr::var("out@1"), VerifyExpr::var("key@0"));
    let signals = vec![
        TaintedSignal { name: "out".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "key".into(), label: SecurityLabel::Secret },
    ];
    let result = check_non_interference(&transition, &signals);
    match result {
        SecurityResult::InformationLeak { path } => {
            assert!(!path.is_empty(), "Path should not be empty");
        }
        other => panic!("Expected InformationLeak, got: {:?}", other),
    }
}
