//! =============================================================================
//! PROOF CERTIFICATES — G13's independent-evidence gate
//! =============================================================================
//!
//! A `largo prove` ✓ is only trustworthy if you trust the prover. This lock pins
//! the stronger guarantee: every kernel-proved theorem yields a PORTABLE,
//! INDEPENDENTLY RE-CHECKABLE certificate — the kernel proof term, the goal it
//! establishes, and the theorem's DISCLOSED premises — that a third party can
//! re-verify with nothing but the kernel, by TYPE-CHECKING the term (never by
//! re-running proof search).
//!
//! The invariants under lock:
//!   A. A real theorem's certificate re-checks GREEN via the independent checker,
//!      and does so with TWO-KERNEL agreement (the de Bruijn re-checker concurs).
//!   B. INDEPENDENCE — a hand-TAMPERED proof term FAILS the checker. The checker
//!      is not rubber-stamping the prover's ✓; it re-derives the type itself.
//!   C. A claim swapped to `False` while keeping an honest term FAILS.
//!   D. A DELETED premise (a hypothesis the proof cites, dropped from the disclosed
//!      environment) FAILS — the checker really replays the disclosed delta.
//!   E. HONESTY — an AXIOM/assumption carries NO certificate (it is an assumption,
//!      not a proof), and an UNPROVED theorem carries none either.
//!   F. The certificate round-trips losslessly through JSON — it is a portable
//!      artifact you could email.
//!   G. A certificate minted against a FOREIGN prelude version is refused.
//!
//! There is no finite-enumeration path into these certificates: only a genuine
//! kernel derivation (a proof term) mints one.

use logicaffeine_compile::testrun::{Inventory, ProvenClaim};
use logicaffeine_compile::theorem_certificate;
use logicaffeine_kernel::certificate::{recheck_theorem, TheoremCertificate};
use logicaffeine_kernel::Term;

/// A genuine English theorem: Socrates is a man, every man is mortal ⊢ Socrates is
/// mortal. Its kernel proof is `h2 Socrates h1`, citing the premise hypotheses.
const SOCRATES: &str = r#"
## Theorem: Socrates_Mortality
Given: Socrates is a man.
Given: Every man is mortal.
Prove: Socrates is mortal.
Proof: Auto.
"#;

/// A theorem that is NOT provable (the "every man is mortal" premise is missing),
/// so the proof engine cannot certify it — no proof term, hence no certificate.
const UNPROVABLE: &str = r#"
## Theorem: Incomplete
Given: Every man is mortal.
Prove: Socrates is mortal.
Proof: Auto.
"#;

fn socrates_certificate() -> TheoremCertificate {
    theorem_certificate(SOCRATES)
        .expect("the theorem block parses")
        .expect("a proved theorem yields a certificate")
}

// =============================================================================
// A. A real theorem's certificate re-checks GREEN — with two-kernel agreement.
// =============================================================================
#[test]
fn a_proved_theorem_certificate_rechecks_green() {
    let cert = socrates_certificate();
    assert_eq!(cert.theorem, "Socrates_Mortality");

    let out = recheck_theorem(&cert);
    assert!(
        out.rechecks,
        "a genuine kernel proof's certificate must re-check GREEN: {:?}",
        out.error
    );
    assert!(
        out.two_kernel_agreement,
        "the INDEPENDENT de Bruijn kernel must corroborate the proof term (full de Bruijn criterion)"
    );
    assert!(
        out.error.is_none(),
        "a green re-check carries no error: {:?}",
        out.error
    );

    // The certificate discloses its premises IN THE OPEN — the reader can see exactly
    // which assumptions the theorem rests on (the two Given: premises + its vocabulary).
    assert!(
        cert.declarations.iter().any(|d| d.ty
            == Term::App(
                Box::new(Term::Global("man".into())),
                Box::new(Term::Global("Socrates".into()))
            )),
        "the premise `Socrates is a man` must be disclosed as a hypothesis"
    );
    assert!(
        !cert.declarations.is_empty(),
        "a theorem proved FROM premises discloses them; a certificate that hid its \
         assumptions would be dishonest"
    );
}

// =============================================================================
// B. INDEPENDENCE — a hand-tampered proof term FAILS the checker.
//    This is the whole point: the checker re-derives the type, it does not trust
//    the prover's verdict. A wrong sub-term must be caught.
// =============================================================================
#[test]
fn a_tampered_proof_term_fails_the_checker() {
    let good = socrates_certificate();
    // The honest proof is `h2 Socrates h1`. Tamper the argument: apply `h2 Socrates`
    // to the CONSTANT `Socrates` instead of the hypothesis `h1 : man Socrates`. The
    // term now type-errors (an Entity where a proof of `man Socrates` is required), so
    // an independent type-checker MUST reject it — the prover's ✓ cannot save it.
    let tampered_term = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("h2".into())),
            Box::new(Term::Global("Socrates".into())),
        )),
        Box::new(Term::Global("Socrates".into())), // was Global("h1")
    );
    let mut bad = good.clone();
    bad.proof_term = tampered_term;

    let out = recheck_theorem(&bad);
    assert!(
        !out.rechecks,
        "a TAMPERED proof term must FAIL the independent checker — otherwise the \
         certificate is just the prover's rubber stamp, proving nothing"
    );
    assert!(out.error.is_some(), "a rejection explains why");

    // And the ORIGINAL still re-checks: the checker is discriminating, not always-no.
    assert!(
        recheck_theorem(&good).rechecks,
        "the untouched certificate must still re-check — the failure above is the \
         tamper, not a checker that rejects everything"
    );
}

// =============================================================================
// C. A term that does not prove the CLAIMED goal fails, even if it type-checks.
//    Keep the honest proof term, but claim it establishes `False`.
// =============================================================================
#[test]
fn a_swapped_claim_fails_the_checker() {
    let mut bad = socrates_certificate();
    bad.claimed_type = Term::Global("False".into());
    assert!(
        !recheck_theorem(&bad).rechecks,
        "claiming an honest proof of `Socrates is mortal` establishes `False` must be REJECTED"
    );
}

// =============================================================================
// D. DELETED-PREMISE independence — dropping a hypothesis the proof cites from the
//    disclosed environment makes the term unbindable, so the checker fails. This
//    proves the checker really REPLAYS the disclosed delta, rather than trusting it.
// =============================================================================
#[test]
fn deleting_a_cited_premise_fails_the_checker() {
    let mut bad = socrates_certificate();
    // `h1 : man Socrates` is cited by the proof term. Remove it from the disclosed
    // declarations — now `h1` is an unbound free variable in the rebuilt context.
    bad.declarations.retain(|d| d.name != "h1");
    let out = recheck_theorem(&bad);
    assert!(
        !out.rechecks,
        "a proof citing a hypothesis dropped from the disclosed premises must FAIL — \
         the checker rebuilds the environment from the delta, it does not assume it holds"
    );
}

// =============================================================================
// E. HONESTY in the INVENTORY — a proved theorem carries a re-checking certificate;
//    an axiom/assumption carries NONE (it is an assumption, not a proof).
// =============================================================================
#[test]
fn axioms_carry_no_certificate_theorems_do() {
    let cert = socrates_certificate();
    let theorems = vec![ProvenClaim {
        name: "Socrates_Mortality".to_string(),
        verified: true,
        finite_bound: None,
        certificate: Some(cert),
    }];
    let axioms = vec!["some_assumption".to_string()];
    let admitted = vec!["an_admitted_lemma".to_string()];

    let inv = Inventory::from_prove(&theorems, &axioms, &admitted).expect("no name collision");

    let thm = inv
        .items
        .iter()
        .find(|i| i.name == "Socrates_Mortality")
        .unwrap();
    assert!(thm.proof, "a kernel-certified unbounded theorem IS a proof");
    assert!(
        thm.certificate.is_some(),
        "a proved theorem carries its independent certificate in the inventory"
    );
    assert_eq!(
        thm.certificate_rechecks,
        Some(true),
        "the inventory re-checks the certificate independently and records that it holds"
    );

    let ax = inv
        .items
        .iter()
        .find(|i| i.name == "some_assumption")
        .unwrap();
    assert!(!ax.proof, "an axiom is an assumption, never a proof");
    assert!(
        ax.certificate.is_none(),
        "an AXIOM is an assumption, NOT a proof — it must never carry a certificate"
    );
    assert_eq!(
        ax.certificate_rechecks, None,
        "no certificate ⇒ no re-check verdict"
    );

    let adm = inv
        .items
        .iter()
        .find(|i| i.name == "an_admitted_lemma")
        .unwrap();
    assert!(
        adm.certificate.is_none(),
        "an ADMITTED assumption is not a proof — it carries no certificate"
    );
}

// =============================================================================
// E2. An UNPROVED theorem yields NO certificate — there is no proof term to certify.
// =============================================================================
#[test]
fn an_unproved_theorem_has_no_certificate() {
    let cert = theorem_certificate(UNPROVABLE).expect("the block parses");
    assert!(
        cert.is_none(),
        "an unproved goal has no proof term, hence no certificate — never a fabricated one"
    );
}

// =============================================================================
// F. The certificate round-trips losslessly through JSON — it is portable evidence.
// =============================================================================
#[test]
fn certificate_roundtrips_through_json() {
    let cert = socrates_certificate();
    let json = serde_json::to_string(&cert).expect("certificate serializes");
    let received: TheoremCertificate =
        serde_json::from_str(&json).expect("certificate deserializes");
    assert_eq!(received, cert, "the JSON round-trip must be lossless");
    assert!(
        recheck_theorem(&received).rechecks,
        "a certificate re-checks after crossing the wire as JSON — the third-party path"
    );
}

// =============================================================================
// G. A certificate minted against a FOREIGN prelude version is refused.
// =============================================================================
#[test]
fn a_foreign_prelude_version_is_refused() {
    let mut cert = socrates_certificate();
    cert.prelude_version = "some-other-logic".to_string();
    let out = recheck_theorem(&cert);
    assert!(
        !out.rechecks,
        "a certificate minted against a DIFFERENT axiom set must be refused, not silently accepted"
    );
    assert!(out.error.is_some());
}
