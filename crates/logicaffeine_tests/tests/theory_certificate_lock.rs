//! ★ LOCK — every theorem a `## Theory` development proves carries a re-checkable certificate. ★
//!
//! The development path proves each theorem with the backward chainer and type-checks the proof term in the kernel,
//! as the English path does; the English path hands that term out as a [`TheoremCertificate`] a third party re-checks
//! with nothing but the kernel. A development's theorem gets the same: the term, the goal, and the axioms and cited
//! lemmas it assumed, disclosed as hypotheses. So:
//!
//! - every proved theorem carries a certificate, named after it, that re-checks with two-kernel agreement;
//! - the certificate claims the theorem's own goal;
//! - another theorem's proof term, or a certificate missing an axiom the proof cites, fails the re-check;
//! - an unproved theorem carries none;
//! - `largo prove --certificate` reports every one of them, re-checked.
//!
//! The development is the Andersen points-to analysis as Datalog rules (Whaley and Lam's PLDI 2004 formulation): its
//! facts and four rules are axioms, each derived fact a theorem.

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr, stdout};
use logicaffeine_compile::ui_bridge::compile_theory_for_ui;
use logicaffeine_kernel::certificate::{recheck_theorem, TheoremCertificate};

const RULES: &str = "\
Axiom new_a: VP0(A, H1).
Axiom new_b: VP0(B, H2).
Axiom copy_c: Assign(C, A).
Axiom store_c: Store(C, Next, B).
Axiom load_d: Load(A, Next, D).
Axiom new_e: VP0(E, H3).
Axiom store_a: Store(A, Next, E).

Axiom alloc: for all v h, if VP0(v, h) then VP(v, h).
Axiom assign: for all v1 v2 h, if Assign(v1, v2) and VP(v2, h) then VP(v1, h).
Axiom store: for all v1 f v2 h1 h2, if Store(v1, f, v2) and VP(v1, h1) and VP(v2, h2) then HP(h1, f, h2).
Axiom load: for all v1 f v2 h1 h2, if Load(v1, f, v2) and VP(v1, h1) and HP(h1, f, h2) then VP(v2, h2).
";

const DERIVED: &str = "\
Theorem c_points_to_h1: prove VP(C, H1).
Theorem h1_next_holds_h2: prove HP(H1, Next, H2).
Theorem d_points_to_h2: prove VP(D, H2).
Theorem d_points_to_h3: prove VP(D, H3).
";

const NOT_DERIVED: &str = "Theorem b_points_to_h1: prove VP(B, H1).\n";

fn development(theorems: &str) -> String {
    format!("## Theory PointsTo\n\n{RULES}\n{theorems}")
}

/// Each theorem's name, whether it was proved, and its certificate.
fn proved(source: &str) -> Vec<(String, bool, Option<TheoremCertificate>)> {
    let result = compile_theory_for_ui(source);
    assert!(result.parse_error.is_none(), "the development parses: {:?}", result.parse_error);
    result.theorems.into_iter().map(|t| (t.name, t.verified, t.certificate)).collect()
}

fn certificate_of(source: &str, name: &str) -> TheoremCertificate {
    proved(source)
        .into_iter()
        .find(|(n, _, _)| n == name)
        .and_then(|(_, _, c)| c)
        .unwrap_or_else(|| panic!("`{name}` carries a certificate"))
}

#[test]
fn every_proved_theorem_carries_a_certificate_that_rechecks() {
    let results = proved(&development(DERIVED));
    assert_eq!(results.len(), 4);
    for (name, verified, certificate) in results {
        assert!(verified, "`{name}` follows from the rules");
        let certificate = certificate.unwrap_or_else(|| panic!("the proved `{name}` carries a certificate"));
        assert_eq!(certificate.theorem, name);
        let out = recheck_theorem(&certificate);
        assert!(out.rechecks, "`{name}`'s certificate re-checks: {:?}", out.error);
        assert!(out.two_kernel_agreement, "the de Bruijn kernel agrees on `{name}`");
        assert!(out.error.is_none(), "`{name}`: {:?}", out.error);
    }
}

#[test]
fn the_certificate_claims_the_theorems_goal() {
    let source = development(DERIVED);
    for (name, predicate, args) in [
        ("c_points_to_h1", "VP", ["C", "H1"].as_slice()),
        ("h1_next_holds_h2", "HP", ["H1", "Next", "H2"].as_slice()),
        ("d_points_to_h3", "VP", ["D", "H3"].as_slice()),
    ] {
        let claimed = certificate_of(&source, name).claimed_type.to_string();
        assert!(claimed.contains(predicate), "`{name}` claims {predicate}: {claimed}");
        for arg in args {
            assert!(claimed.contains(arg), "`{name}` claims a fact about {arg}: {claimed}");
        }
    }
}

#[test]
fn another_theorems_proof_does_not_recheck() {
    let source = development(DERIVED);
    let mut c = certificate_of(&source, "c_points_to_h1");
    c.proof_term = certificate_of(&source, "d_points_to_h3").proof_term;
    assert!(!recheck_theorem(&c).rechecks, "the proof that D points to H3 does not prove that C points to H1");
}

#[test]
fn a_certificate_missing_an_axiom_its_proof_cites_does_not_recheck() {
    let source = development(DERIVED);
    let mut c = certificate_of(&source, "c_points_to_h1");
    let before = c.declarations.len();
    c.declarations.retain(|d| !d.ty.to_string().contains("Assign"));
    assert!(c.declarations.len() < before, "the copy fact and the assign rule are disclosed");
    assert!(!recheck_theorem(&c).rechecks, "C points to H1 only through the copy; without it the proof does not check");
}

#[test]
fn an_unproved_theorem_carries_no_certificate() {
    let results = proved(&development(&format!("{DERIVED}{NOT_DERIVED}")));
    let (_, verified, certificate) = results
        .into_iter()
        .find(|(n, _, _)| n == "b_points_to_h1")
        .expect("the theorem is reported");
    assert!(!verified, "B never points to H1");
    assert!(certificate.is_none(), "an unproved theorem has no proof term, so no certificate");
}

#[test]
fn a_theorem_over_free_variables_discloses_them_and_rechecks() {
    // Lowercase names are variables, free in the theorem: the kernel checks the proof with each one bound as a local, and
    // the certificate must disclose them, or a re-checker meets them unbound.
    let source = "## Theory PointsTo\n\n\
Axiom new_a: VP0(a, h1).\n\
Axiom copy_c: Assign(c, a).\n\
Axiom alloc: for all v h, if VP0(v, h) then VP(v, h).\n\
Axiom assign: for all v1 v2 h, if Assign(v1, v2) and VP(v2, h) then VP(v1, h).\n\n\
Theorem a_points_to_h1: prove VP(a, h1).\n\
Theorem c_points_to_h1: prove VP(c, h1).\n";
    let results = proved(source);
    assert_eq!(results.len(), 2);
    for (name, verified, certificate) in results {
        assert!(verified, "`{name}` follows from the rules");
        let certificate = certificate.unwrap_or_else(|| panic!("the proved `{name}` carries a certificate"));
        let out = recheck_theorem(&certificate);
        assert!(out.rechecks, "`{name}`'s certificate re-checks: {:?}", out.error);
        assert!(out.two_kernel_agreement, "the de Bruijn kernel agrees on `{name}`");
    }
}

#[test]
fn largo_prove_reports_every_theorem_of_a_development_rechecked() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("points_to.lg"), development(DERIVED)).expect("write source");
    let out = largo_in(dir.path(), &["prove", "points_to.lg", "--certificate"]);
    assert!(out.status.success(), "`largo prove --certificate` succeeds:\n{}", stderr(&out));
    let report: serde_json::Value = serde_json::from_str(stdout(&out).trim()).expect("the report is JSON");
    let certificates = report["certificates"].as_array().expect("a certificate list");
    assert_eq!(certificates.len(), 4);
    for entry in certificates {
        let name = entry["name"].as_str().unwrap_or_default();
        assert!(entry["certificate"].is_object(), "`{name}` carries a certificate: {entry}");
        assert_eq!(entry["rechecks"], true, "`{name}` re-checks: {entry}");
    }
}
