//! Congruence closure over uninterpreted functions — "equals added to equals",
//! Euclid's most-used move. `F(a)=F(b)` follows from `a=b`, closed through
//! transitivity and nested/multi-argument applications. The engine decides via a
//! saturated equality graph and reconstructs the proof from `Reflexivity`,
//! `EqualitySymmetry`, `EqualityTransitivity`, and `Rewrite` (Leibniz) — every
//! step kernel-certified. Capitalized names are Global constants (lowercase lowers
//! to variables).

use logicaffeine_proof::verify::prove_certify_check;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn f(n: &str, args: Vec<ProofTerm>) -> ProofTerm {
    ProofTerm::Function(n.to_string(), args)
}
fn eq(l: ProofTerm, r: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(l, r)
}

/// a=b ⊢ F(a)=F(b)
#[test]
fn congruence_unary() {
    let r = prove_certify_check(&[eq(k("A"), k("B"))], &eq(f("F", vec![k("A")]), f("F", vec![k("B")])));
    assert!(r.verified, "F(A)=F(B) from A=B: {:?}", r.verification_error);
}

/// a=b, b=c ⊢ F(a)=F(c)   (congruence through transitivity)
#[test]
fn congruence_with_transitivity() {
    let r = prove_certify_check(
        &[eq(k("A"), k("B")), eq(k("B"), k("C"))],
        &eq(f("F", vec![k("A")]), f("F", vec![k("C")])),
    );
    assert!(r.verified, "F(A)=F(C) from A=B,B=C: {:?}", r.verification_error);
}

/// a=b ⊢ G(F(a))=G(F(b))   (nested congruence)
#[test]
fn congruence_nested() {
    let r = prove_certify_check(
        &[eq(k("A"), k("B"))],
        &eq(f("G", vec![f("F", vec![k("A")])]), f("G", vec![f("F", vec![k("B")])])),
    );
    assert!(r.verified, "G(F(A))=G(F(B)) from A=B: {:?}", r.verification_error);
}

/// a=b, c=d ⊢ F(a,c)=F(b,d)   (binary congruence, two arguments rewritten)
#[test]
fn congruence_binary() {
    let r = prove_certify_check(
        &[eq(k("A"), k("B")), eq(k("C"), k("D"))],
        &eq(f("F", vec![k("A"), k("C")]), f("F", vec![k("B"), k("D")])),
    );
    assert!(r.verified, "F(A,C)=F(B,D) from A=B,C=D: {:?}", r.verification_error);
}

/// F(a)=b, a=c ⊢ F(c)=b   (congruence + transitivity through an atomic side)
#[test]
fn congruence_substitution_closure() {
    let r = prove_certify_check(
        &[eq(f("F", vec![k("A")]), k("B")), eq(k("A"), k("C"))],
        &eq(f("F", vec![k("C")]), k("B")),
    );
    assert!(r.verified, "F(C)=B from F(A)=B,A=C: {:?}", r.verification_error);
}

/// Soundness: F(a)=F(b) must NOT follow when the argument equality is absent.
#[test]
fn congruence_without_argument_equality_fails() {
    let r = prove_certify_check(&[eq(k("X"), k("Y"))], &eq(f("F", vec![k("A")]), f("F", vec![k("B")])));
    assert!(!r.verified, "F(A)=F(B) must not follow from an unrelated X=Y");
}
