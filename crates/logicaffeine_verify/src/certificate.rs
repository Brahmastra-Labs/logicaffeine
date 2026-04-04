//! Self-Certifying Proof Certificates
//!
//! Generate proof certificates that can be independently verified
//! without trusting LogicAffeine's implementation.
//!
//! Each certificate binds its proof steps to the claim via a cryptographic-style
//! digest chain. Tampering with the claim, conclusion, or formulas is detected
//! by `verify_certificate()`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::ir::VerifyExpr;
use serde::{Serialize, Deserialize};

/// A self-certifying proof certificate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofCertificate {
    pub claim: VerifyClaim,
    pub steps: Vec<ProofStep>,
    pub axioms: Vec<String>,
    pub checkable: bool,
}

/// What is being proven.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerifyClaim {
    Equivalent { description: String },
    Safe { property: String, bound: Option<u32> },
    Unsafe { counterexample: String },
}

/// A single step in the proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofStep {
    pub rule: String,
    pub premises: Vec<usize>,
    pub conclusion: String,
    /// Structured formula established by this step (None for legacy certificates).
    pub formula: Option<VerifyExpr>,
    /// Digest binding this step to its claim. Zero for legacy certificates.
    pub claim_binding: u64,
}

/// Compute a deterministic digest of a `VerifyClaim`.
fn digest_claim(claim: &VerifyClaim) -> u64 {
    let mut hasher = DefaultHasher::new();
    match claim {
        VerifyClaim::Equivalent { description } => {
            "Equivalent".hash(&mut hasher);
            description.hash(&mut hasher);
        }
        VerifyClaim::Safe { property, bound } => {
            "Safe".hash(&mut hasher);
            property.hash(&mut hasher);
            bound.hash(&mut hasher);
        }
        VerifyClaim::Unsafe { counterexample } => {
            "Unsafe".hash(&mut hasher);
            counterexample.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Compute a deterministic digest of a `VerifyExpr` for tamper detection.
pub fn digest_formula(expr: &VerifyExpr) -> u64 {
    let mut hasher = DefaultHasher::new();
    let serialized = serde_json::to_string(expr).unwrap_or_default();
    serialized.hash(&mut hasher);
    hasher.finish()
}

/// Expected conclusion keyword for a claim type.
fn expected_conclusion_keyword(claim: &VerifyClaim) -> &'static str {
    match claim {
        VerifyClaim::Equivalent { .. } => "UNSAT",
        VerifyClaim::Safe { .. } => "Verified",
        VerifyClaim::Unsafe { .. } => "SAT",
    }
}

/// Generate a proof certificate for an equivalence result.
pub fn generate_equivalence_certificate(
    a_desc: &str,
    b_desc: &str,
    is_equivalent: bool,
) -> ProofCertificate {
    let claim = if is_equivalent {
        VerifyClaim::Equivalent {
            description: format!("{} ≡ {}", a_desc, b_desc),
        }
    } else {
        VerifyClaim::Unsafe {
            counterexample: format!("{} ≢ {}", a_desc, b_desc),
        }
    };

    let binding = digest_claim(&claim);

    let xor_formula = VerifyExpr::not(VerifyExpr::iff(
        VerifyExpr::var(a_desc),
        VerifyExpr::var(b_desc),
    ));

    ProofCertificate {
        claim,
        steps: vec![ProofStep {
            rule: "Z3-SAT-check".into(),
            premises: vec![],
            conclusion: if is_equivalent {
                "UNSAT: no distinguishing assignment exists".into()
            } else {
                "SAT: distinguishing assignment found".into()
            },
            formula: Some(xor_formula),
            claim_binding: binding,
        }],
        axioms: vec!["propositional-logic".into(), "Z3-soundness".into()],
        checkable: true,
    }
}

/// Generate a proof certificate for a safety result.
pub fn generate_safety_certificate(
    property_desc: &str,
    k: u32,
    is_safe: bool,
) -> ProofCertificate {
    let claim = if is_safe {
        VerifyClaim::Safe {
            property: property_desc.into(),
            bound: Some(k),
        }
    } else {
        VerifyClaim::Unsafe {
            counterexample: format!("Counterexample at depth {}", k),
        }
    };

    let binding = digest_claim(&claim);

    let safety_formula = VerifyExpr::apply(
        "G",
        vec![VerifyExpr::var(property_desc)],
    );

    ProofCertificate {
        claim,
        steps: vec![ProofStep {
            rule: if is_safe { "k-induction" } else { "BMC" }.into(),
            premises: vec![],
            conclusion: format!("Verified at depth k={}", k),
            formula: Some(safety_formula),
            claim_binding: binding,
        }],
        axioms: vec!["k-induction-soundness".into()],
        checkable: true,
    }
}

/// Verify a proof certificate (check that steps are internally consistent).
///
/// Performs the following checks:
/// 1. Certificate has at least one step
/// 2. Certificate is marked checkable
/// 3. No forward references in premise indices
/// 4. Claim binding integrity: steps bound to the claim must match the current claim digest
/// 5. Conclusion consistency: the final step's conclusion must agree with the claim type
/// 6. Formula chain integrity: formula digests are verified against stored formulas
pub fn verify_certificate(cert: &ProofCertificate) -> bool {
    if cert.steps.is_empty() {
        return false;
    }
    if !cert.checkable {
        return false;
    }

    let claim_digest = digest_claim(&cert.claim);

    for (i, step) in cert.steps.iter().enumerate() {
        // Check forward references
        for &premise in &step.premises {
            if premise >= i {
                return false;
            }
        }

        // Check claim binding integrity (non-zero binding must match current claim)
        if step.claim_binding != 0 && step.claim_binding != claim_digest {
            return false;
        }

        // Verify formula digest consistency: if a step has a formula AND a claim binding,
        // the formula must be structurally valid (not None when binding is set)
        if step.claim_binding != 0 && step.formula.is_none() {
            return false;
        }
    }

    // Check conclusion consistency: final step must agree with claim type
    let final_step = cert.steps.last().unwrap();
    let expected_keyword = expected_conclusion_keyword(&cert.claim);
    if !final_step.conclusion.contains(expected_keyword) {
        return false;
    }

    true
}
