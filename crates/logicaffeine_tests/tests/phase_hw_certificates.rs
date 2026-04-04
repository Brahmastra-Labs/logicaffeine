//! SUPERCRUSH Sprint S3B: Self-Certifying Proof Certificates

#![cfg(feature = "verification")]

use logicaffeine_verify::certificate::*;

#[test]
fn cert_generated_for_equivalence() {
    let cert = generate_equivalence_certificate("A", "B", true);
    assert!(matches!(cert.claim, VerifyClaim::Equivalent { .. }));
}

#[test]
fn cert_generated_for_safety() {
    let cert = generate_safety_certificate("G(p)", 5, true);
    assert!(matches!(cert.claim, VerifyClaim::Safe { .. }));
}

#[test]
fn cert_verifiable() {
    let cert = generate_equivalence_certificate("A", "B", true);
    assert!(verify_certificate(&cert), "Valid certificate should verify");
}

#[test]
fn cert_rejects_tampered() {
    let mut cert = generate_equivalence_certificate("A", "B", true);
    cert.steps[0].premises.push(999); // Invalid forward reference
    assert!(!verify_certificate(&cert), "Tampered certificate should fail");
}

#[test]
fn cert_includes_axioms() {
    let cert = generate_equivalence_certificate("A", "B", true);
    assert!(!cert.axioms.is_empty(), "Certificate should list axioms");
}

#[test]
fn cert_steps_valid() {
    let cert = generate_safety_certificate("G(p)", 3, true);
    for (i, step) in cert.steps.iter().enumerate() {
        for &premise in &step.premises {
            assert!(premise < i, "Premise must reference earlier step");
        }
    }
}

#[test]
fn cert_unsafe_has_counterexample() {
    let cert = generate_safety_certificate("G(p)", 3, false);
    assert!(matches!(cert.claim, VerifyClaim::Unsafe { .. }));
}

#[test]
fn cert_serializable_json() {
    let cert = generate_equivalence_certificate("A", "B", true);
    let json = serde_json::to_string(&cert).unwrap();
    let deser: ProofCertificate = serde_json::from_str(&json).unwrap();
    assert!(verify_certificate(&deser));
}

#[test]
fn cert_empty_fails() {
    let cert = ProofCertificate {
        claim: VerifyClaim::Equivalent { description: "test".into() },
        steps: vec![],
        axioms: vec![],
        checkable: true,
    };
    assert!(!verify_certificate(&cert), "Empty certificate should fail");
}

#[test]
fn cert_root_matches_claim() {
    let cert = generate_safety_certificate("always safe", 10, true);
    let last_step = cert.steps.last().unwrap();
    assert!(!last_step.conclusion.is_empty());
}

#[test]
fn cert_rejects_tampered_conclusion() {
    let mut cert = generate_equivalence_certificate("A", "B", true);
    // The claim says Equivalent (expects UNSAT), but we tamper the conclusion to say SAT
    cert.steps[0].conclusion = "SAT: distinguishing assignment found".into();
    assert!(
        !verify_certificate(&cert),
        "Certificate with tampered conclusion (UNSAT->SAT) must be rejected"
    );
}

#[test]
fn cert_rejects_wrong_claim() {
    let mut cert = generate_equivalence_certificate("A", "B", true);
    // Certificate was generated for A≡B, tamper claim to A≡C
    cert.claim = VerifyClaim::Equivalent {
        description: "A ≡ C".into(),
    };
    assert!(
        !verify_certificate(&cert),
        "Certificate whose claim was changed from A≡B to A≡C must be rejected"
    );
}

#[test]
fn cert_steps_reference_actual_formulas() {
    let cert = generate_equivalence_certificate("A", "B", true);
    let step = &cert.steps[0];
    assert!(
        step.formula.is_some(),
        "Proof steps must contain structured formula references, not just description strings"
    );
    let formula = step.formula.as_ref().unwrap();
    let formula_str = format!("{:?}", formula);
    assert!(
        formula_str.contains("Var") && formula_str.contains("A") && formula_str.contains("B"),
        "Formula must reference the actual signals A and B, got: {}",
        formula_str
    );
}
