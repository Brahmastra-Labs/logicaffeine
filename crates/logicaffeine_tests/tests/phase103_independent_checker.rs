//! =============================================================================
//! PHASE 103: THE INDEPENDENT PROOF CHECKER (De Bruijn criterion)
//! =============================================================================
//!
//! A competitor returns a *verdict* ("Z3 says unsat"). LOGOS returns a
//! *certificate*: a serializable proof term plus the proposition it claims to
//! prove. Anyone can re-validate it against the prelude axioms with NOTHING but
//! the kernel — no parser, no proof engine, no SMT solver.
//!
//! These tests exercise that guarantee end to end:
//!   * a real theorem round-trips through JSON and re-checks (`recheck`);
//!   * tampering with the term or the claim is REJECTED;
//!   * the checker supplies its own axioms, so a forged certificate cannot
//!     smuggle in a free proof of `False`.

use logicaffeine_kernel::certificate::{recheck, Certificate};
use logicaffeine_kernel::{Term, Universe};

// -----------------------------------------------------------------------------
// A genuine, prelude-only theorem: ∀(P:Prop). P → P  (the polymorphic identity).
// Proof term: λ(P:Prop). λ(h:P). h    Type: Π(P:Prop). Π(_:P). P
// It references only the `Prop` sort, so any re-checker that rebuilds the
// standard prelude can validate it with zero domain vocabulary.
// -----------------------------------------------------------------------------

fn prop() -> Term {
    Term::Sort(Universe::Prop)
}

fn identity_proof_term() -> Term {
    Term::Lambda {
        param: "P".into(),
        param_type: Box::new(prop()),
        body: Box::new(Term::Lambda {
            param: "h".into(),
            param_type: Box::new(Term::Var("P".into())),
            body: Box::new(Term::Var("h".into())),
        }),
    }
}

fn identity_claimed_type() -> Term {
    Term::Pi {
        param: "P".into(),
        param_type: Box::new(prop()),
        body_type: Box::new(Term::Pi {
            param: "_".into(),
            param_type: Box::new(Term::Var("P".into())),
            body_type: Box::new(Term::Var("P".into())),
        }),
    }
}

fn identity_certificate() -> Certificate {
    Certificate::new(identity_proof_term(), identity_claimed_type())
}

// =============================================================================
// A. A valid certificate round-trips through JSON and re-checks independently.
// =============================================================================
#[test]
fn certificate_roundtrips_and_rechecks() {
    let cert = identity_certificate();

    // Serialize to JSON — this is the portable artifact you could email.
    let json = serde_json::to_string(&cert).expect("certificate serializes");

    // Re-check in a context that imports ONLY the kernel — simulating an
    // independent verifier with no access to how the proof was found.
    let received: Certificate = serde_json::from_str(&json).expect("certificate deserializes");
    assert_eq!(received, cert, "round-trip must be lossless");

    recheck(&received).expect("a valid certificate must re-check from the axioms");
}

// =============================================================================
// B. Tampering with the PROOF TERM is rejected.
// =============================================================================
#[test]
fn tampered_term_is_rejected() {
    // Corrupt the body: return the type variable `P` instead of the proof `h`.
    // The term still type-checks, but to Π(P:Prop). P → Prop — NOT P → P — so the
    // claimed type no longer matches.
    let bad_term = Term::Lambda {
        param: "P".into(),
        param_type: Box::new(prop()),
        body: Box::new(Term::Lambda {
            param: "h".into(),
            param_type: Box::new(Term::Var("P".into())),
            body: Box::new(Term::Var("P".into())), // <-- was Var("h")
        }),
    };
    let cert = Certificate::new(bad_term, identity_claimed_type());
    assert!(
        recheck(&cert).is_err(),
        "a term that does not prove the claimed type must be REJECTED"
    );
}

// =============================================================================
// C. Tampering with the CLAIM is rejected.
// =============================================================================
#[test]
fn tampered_claim_is_rejected() {
    // Keep the honest identity proof, but claim it proves `False`.
    let cert = Certificate::new(identity_proof_term(), Term::Global("False".into()));
    assert!(
        recheck(&cert).is_err(),
        "claiming a valid proof establishes False must be REJECTED"
    );
}

// =============================================================================
// D. Byte-level forgery in the serialized JSON is rejected.
// =============================================================================
#[test]
fn json_byte_tampering_is_rejected() {
    let json = serde_json::to_string(&identity_certificate()).unwrap();
    // Flip the bound-variable reference "h" in the body to "P" at the text level.
    // (The last `"Var":"h"` is the body that should return the proof.)
    let forged = {
        let idx = json.rfind("\"h\"").expect("body var present");
        let mut s = json.clone();
        s.replace_range(idx..idx + 3, "\"P\"");
        s
    };
    let cert: Certificate = serde_json::from_str(&forged).expect("forged JSON still parses");
    assert!(
        recheck(&cert).is_err(),
        "byte-level forgery must not survive re-checking"
    );
}

// =============================================================================
// E. Anti-smuggling: the checker uses ITS OWN axioms, not the certificate's.
//    A forged certificate that references a fabricated axiom is rejected — you
//    cannot mint `my_false : False` and ride it to a proof of anything.
// =============================================================================
#[test]
fn checker_rejects_smuggled_axiom() {
    // Claim a proof of False whose term is a made-up global the prelude never
    // declares. With no carried context to trust, the kernel finds it unbound.
    let cert = Certificate::new(Term::Global("my_free_false".into()), Term::Global("False".into()));
    assert!(
        recheck(&cert).is_err(),
        "a certificate referencing an undeclared axiom must be REJECTED"
    );
}

// =============================================================================
// F. The version guard: a certificate from a foreign axiom set is refused.
// =============================================================================
#[test]
fn foreign_prelude_version_is_refused() {
    let mut cert = identity_certificate();
    cert.prelude_version = "some-other-logic".into();
    assert!(
        recheck(&cert).is_err(),
        "a certificate minted against a different prelude must be refused"
    );
}
