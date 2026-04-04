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

// ═══════════════════════════════════════════════════════════════════════════
// EXTENDED SECURITY TESTS — SPEC DELTA (14 additional)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn security_constant_time_no_leak() {
    // Operations take same path regardless of secret
    let transition = VerifyExpr::iff(
        VerifyExpr::var("output@1"),
        VerifyExpr::var("input@0"),
    );
    let signals = vec![
        TaintedSignal { name: "input".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "output".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "secret".into(), label: SecurityLabel::Secret },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::NonInterference),
        "Secret not used → no leak. Got: {:?}", result);
}

#[test]
fn security_multiple_secrets() {
    // Two secrets, one leaks
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("out@1"), VerifyExpr::var("key1@0")),
        VerifyExpr::bool(true), // key2 not used
    );
    let signals = vec![
        TaintedSignal { name: "out".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "key1".into(), label: SecurityLabel::Secret },
        TaintedSignal { name: "key2".into(), label: SecurityLabel::Secret },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::InformationLeak { .. }),
        "Key1 leaks to out. Got: {:?}", result);
}

#[test]
fn security_mask_blocks_taint() {
    // output = secret AND 0 → always 0, no leak
    let transition = VerifyExpr::iff(
        VerifyExpr::var("output@1"),
        VerifyExpr::bool(false), // masked to 0
    );
    let signals = vec![
        TaintedSignal { name: "output".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "secret".into(), label: SecurityLabel::Secret },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::NonInterference),
        "Masked secret should not leak. Got: {:?}", result);
}

#[test]
fn security_two_public_no_secret() {
    // No secrets at all
    let transition = VerifyExpr::iff(
        VerifyExpr::var("b@1"),
        VerifyExpr::var("a@0"),
    );
    let signals = vec![
        TaintedSignal { name: "a".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "b".into(), label: SecurityLabel::Public },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::NonInterference));
}

#[test]
fn security_all_secret_no_public() {
    // Only secrets, no public outputs → trivially non-interfering
    let transition = VerifyExpr::bool(true);
    let signals = vec![
        TaintedSignal { name: "key".into(), label: SecurityLabel::Secret },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::NonInterference),
        "No public signals → no leak possible. Got: {:?}", result);
}

#[test]
fn security_identity_preserves() {
    // Identity: output@1 = output@0 (no secret involvement)
    let transition = VerifyExpr::iff(
        VerifyExpr::var("output@1"),
        VerifyExpr::var("output@0"),
    );
    let signals = vec![
        TaintedSignal { name: "output".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "secret".into(), label: SecurityLabel::Secret },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::NonInterference),
        "Identity transition should not leak. Got: {:?}", result);
}

#[test]
fn security_and_gate_leak() {
    // output = input AND secret
    let transition = VerifyExpr::iff(
        VerifyExpr::var("output@1"),
        VerifyExpr::and(VerifyExpr::var("input@0"), VerifyExpr::var("secret@0")),
    );
    let signals = vec![
        TaintedSignal { name: "input".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "output".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "secret".into(), label: SecurityLabel::Secret },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::InformationLeak { .. }),
        "AND with secret should leak. Got: {:?}", result);
}

#[test]
fn security_or_gate_leak() {
    // output = input OR secret
    let transition = VerifyExpr::iff(
        VerifyExpr::var("output@1"),
        VerifyExpr::or(VerifyExpr::var("input@0"), VerifyExpr::var("secret@0")),
    );
    let signals = vec![
        TaintedSignal { name: "input".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "output".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "secret".into(), label: SecurityLabel::Secret },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::InformationLeak { .. }),
        "OR with secret should leak. Got: {:?}", result);
}

#[test]
fn security_empty_signals() {
    let result = check_non_interference(&VerifyExpr::bool(true), &[]);
    assert!(matches!(result, SecurityResult::NonInterference),
        "Empty signals → trivially safe. Got: {:?}", result);
}

#[test]
fn security_deterministic() {
    let transition = VerifyExpr::iff(VerifyExpr::var("out@1"), VerifyExpr::var("key@0"));
    let signals = vec![
        TaintedSignal { name: "out".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "key".into(), label: SecurityLabel::Secret },
    ];
    let r1 = check_non_interference(&transition, &signals);
    let r2 = check_non_interference(&transition, &signals);
    let both_leak = matches!(r1, SecurityResult::InformationLeak { .. })
        && matches!(r2, SecurityResult::InformationLeak { .. });
    assert!(both_leak, "Same input → same result");
}

#[test]
fn security_not_gate_leak() {
    // output = NOT secret
    let transition = VerifyExpr::iff(
        VerifyExpr::var("output@1"),
        VerifyExpr::not(VerifyExpr::var("secret@0")),
    );
    let signals = vec![
        TaintedSignal { name: "output".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "secret".into(), label: SecurityLabel::Secret },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::InformationLeak { .. }),
        "NOT of secret still leaks. Got: {:?}", result);
}

#[test]
fn security_multi_stage_propagation() {
    // Chain: secret → a → b → output
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("a@1"), VerifyExpr::var("secret@0")),
        VerifyExpr::and(
            VerifyExpr::iff(VerifyExpr::var("b@1"), VerifyExpr::var("a@1")),
            VerifyExpr::iff(VerifyExpr::var("output@1"), VerifyExpr::var("b@1")),
        ),
    );
    let signals = vec![
        TaintedSignal { name: "output".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "a".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "b".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "secret".into(), label: SecurityLabel::Secret },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::InformationLeak { .. }),
        "Multi-stage secret propagation should be detected. Got: {:?}", result);
}

#[test]
fn security_implies_gate_safe() {
    // output = (secret → public) — this leaks because secret=false makes output=true regardless
    let transition = VerifyExpr::iff(
        VerifyExpr::var("output@1"),
        VerifyExpr::implies(VerifyExpr::var("secret@0"), VerifyExpr::var("pub@0")),
    );
    let signals = vec![
        TaintedSignal { name: "output".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "pub".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "secret".into(), label: SecurityLabel::Secret },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::InformationLeak { .. }),
        "Implication involving secret still leaks. Got: {:?}", result);
}

#[test]
fn security_three_public_one_secret() {
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("o1@1"), VerifyExpr::var("i1@0")),
        VerifyExpr::and(
            VerifyExpr::iff(VerifyExpr::var("o2@1"), VerifyExpr::var("i2@0")),
            VerifyExpr::iff(VerifyExpr::var("o3@1"), VerifyExpr::var("i3@0")),
        ),
    );
    let signals = vec![
        TaintedSignal { name: "i1".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "i2".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "i3".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "o1".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "o2".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "o3".into(), label: SecurityLabel::Public },
        TaintedSignal { name: "key".into(), label: SecurityLabel::Secret },
    ];
    let result = check_non_interference(&transition, &signals);
    assert!(matches!(result, SecurityResult::NonInterference),
        "Secret key not used → no leak. Got: {:?}", result);
}
