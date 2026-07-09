//! **Full kernel integration of the `n = ∞` ratchet.**
//!
//! Previously (`no_finite_randomness_infinity`) the base and step were supplied to the kernel as *premises*.
//! Here they are **discharged inside the kernel from the axioms of the recurrence** — the induction proves
//! `∀n. PoU(n) = one` where base and step are *derived*, not assumed. This is the Tarski-style structure: a
//! small equational theory whose axioms are verified in the polynomial *model* (`polycalc`), and a single
//! kernel-certified theorem derived from them.
//!
//! The theory (three axioms, each true in the multilinear-`GF(2)` model — see the model checks below):
//!   - `def_zero  : Eq (PoU Zero) one`                            — the base of the recurrence (`PoU(0)=1`)
//!   - `def_succ  : ∀k. Eq (PoU (Succ k)) (rmul (PoU k) atom)`    — the recurrence (`PoU(n+1)=PoU(n)·atom`)
//!   - `ring_step : ∀k. Eq (rmul (PoU k) atom) (PoU k)`           — `atom` is a right identity (`·atom = id`)
//!
//! The kernel then derives, by `Nat` induction:
//!   - base  `PoU Zero = one`                     ← `def_zero`
//!   - step  `PoU (Succ k) = one`                 ← `def_succ k` ∘ `ring_step k` ∘ IH (transitivity)
//!   ⟹ `∀n. PoU n = one`.
//!
//! Every `∀` is instantiated only at the atomic index `k`, so this stays within the certifier's
//! `UniversalInst`. Full-*full* integration — the polynomial ring itself as a kernel inductive with a
//! computable `·`, so `ring_step` becomes a theorem rather than a model-checked axiom — is the next level.

use std::collections::BTreeSet;

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, Context, Term};
use logicaffeine_proof::certifier::{certify, CertificationContext};
use logicaffeine_proof::polycalc::{partition_of_unity, pou_as_product, pou_atom};
use logicaffeine_proof::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

// ── kernel term helpers ──────────────────────────────────────────────────────────────────────────────
fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn kvar(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn pi(param: &str, ty: Term, body: Term) -> Term {
    Term::Pi { param: param.to_string(), param_type: Box::new(ty), body_type: Box::new(body) }
}
fn nat() -> Term {
    g("Nat")
}
fn entity() -> Term {
    g("Entity")
}
/// `Eq Entity a b` — the propositional equality the certifier maps `Identity(a,b)` to (domain `Entity`).
fn keq(a: Term, b: Term) -> Term {
    app(app(app(g("Eq"), entity()), a), b)
}
fn kpou(n: Term) -> Term {
    app(g("PoU"), n)
}
fn krmul(a: Term, b: Term) -> Term {
    app(app(g("rmul"), a), b)
}
fn ksucc(n: Term) -> Term {
    app(g("Succ"), n)
}

// ── proof-expr helpers ───────────────────────────────────────────────────────────────────────────────
fn id(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(a, b)
}
fn ppou(n: ProofTerm) -> ProofTerm {
    ProofTerm::Function("PoU".to_string(), vec![n])
}
fn prmul(a: ProofTerm, b: ProofTerm) -> ProofTerm {
    ProofTerm::Function("rmul".to_string(), vec![a, b])
}
fn psucc(n: ProofTerm) -> ProofTerm {
    ProofTerm::Function("Succ".to_string(), vec![n])
}
fn pk() -> ProofTerm {
    ProofTerm::Variable("k".to_string())
}
fn pone() -> ProofTerm {
    ProofTerm::Constant("one".to_string())
}
fn patom() -> ProofTerm {
    ProofTerm::Constant("atom".to_string())
}
fn pzero() -> ProofTerm {
    ProofTerm::Constant("Zero".to_string())
}
fn forall(v: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: v.to_string(), body: Box::new(body) }
}
fn named(name: &str) -> DerivationTree {
    DerivationTree::leaf(ProofExpr::Atom(name.to_string()), InferenceRule::PremiseMatch)
}

fn one_poly() -> BTreeSet<u64> {
    [0u64].into_iter().collect()
}

/// The kernel theory: the carrier (`one`, `atom`, `rmul`, `PoU`) and the three recurrence/ring axioms.
fn theory_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.add_declaration("one", entity());
    ctx.add_declaration("atom", entity());
    ctx.add_declaration("rmul", pi("_", entity(), pi("_", entity(), entity())));
    ctx.add_declaration("PoU", pi("_", nat(), entity()));
    ctx.add_declaration("def_zero", keq(kpou(g("Zero")), g("one")));
    ctx.add_declaration(
        "def_succ",
        pi("j", nat(), keq(kpou(ksucc(kvar("j"))), krmul(kpou(kvar("j")), g("atom")))),
    );
    ctx.add_declaration(
        "ring_step",
        pi("j", nat(), keq(krmul(kpou(kvar("j")), g("atom")), kpou(kvar("j")))),
    );
    ctx
}

#[test]
fn full_kernel_integration_derives_no_finite_randomness_for_all_n() {
    // ── The MODEL satisfies the three axioms (multilinear GF(2) ring; polycalc) ─────────────────────────
    // def_zero: PoU(0) = 1.
    assert_eq!(partition_of_unity(0), one_poly(), "model ⊨ def_zero: PoU(0) = 1");
    // ring_step: `atom` is the ring identity, so multiplying by it is the identity map — the atom is 1.
    for v in 0..8 {
        assert_eq!(pou_atom(v), one_poly(), "model ⊨ ring_step: atom (=(1+x)+x) is the identity 1");
    }
    // def_succ ∘ ring_step: appending a coordinate (·atom) leaves PoU unchanged — PoU(n+1) = PoU(n).
    for n in 0..12 {
        assert_eq!(partition_of_unity(n + 1), partition_of_unity(n), "model ⊨ recurrence: PoU(n+1) = PoU(n)·atom = PoU(n)");
        assert_eq!(partition_of_unity(n), pou_as_product(n), "the factorization the axioms abstract");
    }

    // ── The kernel THEORY: declare the carrier, the recurrence, and the ring fact as axioms ─────────────
    let ctx = theory_context();

    // ── The kernel DERIVATION: ∀n. PoU(n) = one, by Nat induction ───────────────────────────────────────
    // Step, at eigen-index k with IH `PoU k = one`:
    //   def_succ k :  PoU(Succ k) = rmul (PoU k) atom
    let ui_succ = DerivationTree::new(
        id(ppou(psucc(pk())), prmul(ppou(pk()), patom())),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("def_succ")],
    );
    //   ring_step k :  rmul (PoU k) atom = PoU k
    let ui_ring = DerivationTree::new(
        id(prmul(ppou(pk()), patom()), ppou(pk())),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("ring_step")],
    );
    //   transitivity :  PoU(Succ k) = PoU k
    let step_eq = DerivationTree::new(
        id(ppou(psucc(pk())), ppou(pk())),
        InferenceRule::EqualityTransitivity,
        vec![ui_succ, ui_ring],
    );
    //   IH :  PoU k = one   (resolves to the recursive call)
    let ih = DerivationTree::leaf(id(ppou(pk()), pone()), InferenceRule::PremiseMatch);
    //   transitivity :  PoU(Succ k) = one
    let step = DerivationTree::new(
        id(ppou(psucc(pk())), pone()),
        InferenceRule::EqualityTransitivity,
        vec![step_eq, ih],
    );

    let tree = DerivationTree::new(
        forall("n", id(ppou(ProofTerm::Variable("n".to_string())), pone())),
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        vec![named("def_zero"), step],
    );

    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("the ∀n induction certifies to a Fix/Match term");
    let inferred =
        infer_type(&ctx, &term).expect("the certified induction term must type-check in the kernel");
    // The theorem's type is ∀n:Nat. Eq Entity (PoU n) one — a Pi over Nat.
    assert!(
        matches!(inferred, Term::Pi { .. }),
        "expected ∀n:Nat. Eq Entity (PoU n) one, got {inferred}"
    );
}

/// **Negative control — the kernel is not rubber-stamping.** Skip `ring_step` and try to chain the step
/// directly: `def_succ k : PoU(Succ k) = rmul (PoU k) atom` composed with the IH `PoU k = one`. The
/// transitivity middle terms do not line up (`rmul (PoU k) atom ≠ PoU k` without the ring axiom), so the
/// assembled term must FAIL the kernel's type check. Soundness is kernel-enforced, not asserted by us.
#[test]
fn skipping_the_ring_axiom_fails_the_kernel_type_check() {
    let ctx = theory_context();

    // def_succ k : PoU(Succ k) = rmul (PoU k) atom
    let ui_succ = DerivationTree::new(
        id(ppou(psucc(pk())), prmul(ppou(pk()), patom())),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("def_succ")],
    );
    // IH : PoU k = one
    let ih = DerivationTree::leaf(id(ppou(pk()), pone()), InferenceRule::PremiseMatch);
    // BROKEN transitivity: claims PoU(Succ k) = one from `= rmul(PoU k) atom` and `PoU k = one` — the middle
    // terms `rmul (PoU k) atom` and `PoU k` differ, so this does not compose.
    let broken_step = DerivationTree::new(
        id(ppou(psucc(pk())), pone()),
        InferenceRule::EqualityTransitivity,
        vec![ui_succ, ih],
    );
    let tree = DerivationTree::new(
        forall("n", id(ppou(ProofTerm::Variable("n".to_string())), pone())),
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        vec![named("def_zero"), broken_step],
    );

    let cert_ctx = CertificationContext::new(&ctx);
    // Either certification rejects it, or the kernel's type check does — never a verified theorem.
    let verified = matches!(
        certify(&tree, &cert_ctx).and_then(|term| infer_type(&ctx, &term)),
        Ok(Term::Pi { .. })
    );
    assert!(!verified, "a step that skips ring_step must NOT yield a kernel-checked ∀n theorem");
}
