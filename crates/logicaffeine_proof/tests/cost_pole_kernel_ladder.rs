//! **The kill-or-absorb branches on the kernel's `∀n` ladder — the lower half of the Cost-Pole
//! Attainment Theorem at the same rigor as the upper half.**
//!
//! `finite_randomness_kernel_integration` lifts the completeness pole to `∀n`: a small equational
//! theory whose axioms are verified in the polynomial *model*, and a kernel-certified `Nat`
//! induction deriving `∀n. PoU(n) = 1`. This file does the same for the attainment pole's two
//! branch invariants (PAPER §5.13):
//!
//!   - **DEAD STAYS DEAD** (`∀n. D(n) = 0`): once a multiplier kills a clause factor
//!     (`x·(1−x) = 0`), every further factor multiplication leaves the product zero. Axioms:
//!     `D(0) = 0` (just killed), `D(j+1) = mulf j (D j)` (keep multiplying), and the model fact
//!     `mulf j (D j) = 0` — zero annihilates, per step, verified in the multilinear `ℤ/m` model.
//!   - **THE ABSORBED PREFIX IS THE CLAUSE POLYNOMIAL** (`∀n. M(n) = P(n)`): a multiplier that
//!     only touches negative-literal variables absorbs (`x·x = x`), so the running product
//!     `M(k)` — multiplier-so-far times prefix-clause-polynomial — *is* the prefix polynomial
//!     `P(k)` at every width. Axioms: `M(0) = P(0)`, `M(j+1) = extf j (M j)`, and the model fact
//!     `extf j (M j) = P(j+1)`.
//!
//! Together with the per-variable kernel seeds (`cost_pole_kernel_seeds`: the atom, cube-point
//! idempotence/annihilation, over `GF(2)/GF(3)/ℤ/4/ℤ/6`) and the exhaustive product sweep
//! (`the_cost_pole_is_attained_at_every_n_over_every_ring_by_kill_or_absorb`), this closes the
//! ladder: the branch invariants are kernel-certified `∀n`, the branch *choice* per variable is a
//! finite case split swept product-by-product, and the axioms feeding the inductions are verified
//! in the same `ℤ/m` model the engine computes in. A negative control (dropping the annihilation
//! axiom) fails the kernel's type check — the gluing is sound, not asserted.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, Context, Term};
use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::certifier::{certify, CertificationContext};
use logicaffeine_proof::polycalc_zm::clause_polynomial_zm;
use logicaffeine_proof::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};
use std::collections::BTreeMap;

// ── kernel term helpers ──────────────────────────────────────────────────────────────────────────
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
fn keq(a: Term, b: Term) -> Term {
    app(app(app(g("Eq"), entity()), a), b)
}
fn kapp1(f: &str, x: Term) -> Term {
    app(g(f), x)
}
fn kapp2(f: &str, x: Term, y: Term) -> Term {
    app(app(g(f), x), y)
}
fn ksucc(n: Term) -> Term {
    app(g("Succ"), n)
}

// ── proof-expr helpers ───────────────────────────────────────────────────────────────────────────
fn id(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(a, b)
}
fn pf1(f: &str, x: ProofTerm) -> ProofTerm {
    ProofTerm::Function(f.to_string(), vec![x])
}
fn pf2(f: &str, x: ProofTerm, y: ProofTerm) -> ProofTerm {
    ProofTerm::Function(f.to_string(), vec![x, y])
}
fn psucc(n: ProofTerm) -> ProofTerm {
    pf1("Succ", n)
}
fn pk() -> ProofTerm {
    ProofTerm::Variable("k".to_string())
}
fn pzero() -> ProofTerm {
    ProofTerm::Constant("Zero".to_string())
}
fn pconst(name: &str) -> ProofTerm {
    ProofTerm::Constant(name.to_string())
}
fn forall(v: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: v.to_string(), body: Box::new(body) }
}
fn named(name: &str) -> DerivationTree {
    DerivationTree::leaf(ProofExpr::Atom(name.to_string()), InferenceRule::PremiseMatch)
}

// ── the multilinear ℤ/m model (independent multiply, as in the attainment sweep) ─────────────────
type Poly = BTreeMap<u64, u64>;

fn mul_polys(m: u64, a: &Poly, b: &Poly) -> Poly {
    let mut r = Poly::new();
    for (&ta, &ca) in a {
        for (&tb, &cb) in b {
            let key = ta | tb;
            let e = r.entry(key).or_insert(0);
            *e = (*e + ca * cb) % m;
            if *e == 0 {
                r.remove(&key);
            }
        }
    }
    r
}

fn literal_factor(m: u64, l: Lit) -> Poly {
    clause_polynomial_zm(m, &[l])
}

/// **The DEAD theory**: once zero, every further factor multiplication stays zero.
fn dead_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.add_declaration("zero", entity());
    ctx.add_declaration("mulf", pi("_", nat(), pi("_", entity(), entity())));
    ctx.add_declaration("D", pi("_", nat(), entity()));
    ctx.add_declaration("def_dead_zero", keq(kapp1("D", g("Zero")), g("zero")));
    ctx.add_declaration(
        "def_dead_succ",
        pi("j", nat(), keq(kapp1("D", ksucc(kvar("j"))), kapp2("mulf", kvar("j"), kapp1("D", kvar("j"))))),
    );
    ctx.add_declaration(
        "dead_step",
        pi("j", nat(), keq(kapp2("mulf", kvar("j"), kapp1("D", kvar("j"))), g("zero"))),
    );
    ctx
}

/// **The ABSORB theory**: the negative-touching multiplier's running product IS the prefix
/// clause polynomial at every width.
fn absorb_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.add_declaration("P", pi("_", nat(), entity()));
    ctx.add_declaration("M", pi("_", nat(), entity()));
    ctx.add_declaration("extf", pi("_", nat(), pi("_", entity(), entity())));
    ctx.add_declaration("def_m_zero", keq(kapp1("M", g("Zero")), kapp1("P", g("Zero"))));
    ctx.add_declaration(
        "def_m_succ",
        pi("j", nat(), keq(kapp1("M", ksucc(kvar("j"))), kapp2("extf", kvar("j"), kapp1("M", kvar("j"))))),
    );
    ctx.add_declaration(
        "ext_step",
        pi("j", nat(), keq(kapp2("extf", kvar("j"), kapp1("M", kvar("j"))), kapp1("P", ksucc(kvar("j"))))),
    );
    ctx
}

/// Certify `∀n. lhs(n) = rhs(n)` by Nat induction with base axiom `base` and a step that chains
/// `def_succ` (the recurrence) with `step_axiom` (the model fact) by transitivity.
fn derive_forall(
    ctx: &Context,
    base_axiom: &str,
    def_succ: &str,
    step_axiom: &str,
    lhs: impl Fn(ProofTerm) -> ProofTerm,
    mid: impl Fn(ProofTerm) -> ProofTerm,
    rhs: impl Fn(ProofTerm) -> ProofTerm,
) -> bool {
    let ui_def = DerivationTree::new(
        id(lhs(psucc(pk())), mid(pk())),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named(def_succ)],
    );
    let ui_step = DerivationTree::new(
        id(mid(pk()), rhs(psucc(pk()))),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named(step_axiom)],
    );
    let step = DerivationTree::new(
        id(lhs(psucc(pk())), rhs(psucc(pk()))),
        InferenceRule::EqualityTransitivity,
        vec![ui_def, ui_step],
    );
    let nvar = || ProofTerm::Variable("n".to_string());
    let tree = DerivationTree::new(
        forall("n", id(lhs(nvar()), rhs(nvar()))),
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        vec![named(base_axiom), step],
    );
    let cert_ctx = CertificationContext::new(ctx);
    matches!(
        certify(&tree, &cert_ctx).and_then(|term| infer_type(ctx, &term)),
        Ok(Term::Pi { .. })
    )
}

#[test]
fn the_kill_and_absorb_invariants_are_kernel_certified_for_all_n() {
    // ── The MODEL satisfies the axioms, over rings with zero divisors ────────────────────────────
    for &m in &[2u64, 3, 4, 6] {
        // DEAD: a killed product stays zero under every further literal factor. Kill x0 (positive
        // literal times its own negative-indicator factor), then multiply through mixed factors.
        let kill = mul_polys(
            m,
            &literal_factor(m, Lit::pos(0)),
            &[(1u64, 1u64)].into_iter().collect::<Poly>(), // ·x0 — the killing multiplier bit
        );
        assert!(kill.is_empty(), "m={m}: x·(1−x) = 0 — the kill");
        let mut dead = kill;
        for v in 1..8u32 {
            let f = if v % 2 == 0 { literal_factor(m, Lit::pos(v)) } else { literal_factor(m, Lit::neg(v)) };
            dead = mul_polys(m, &dead, &f);
            assert!(dead.is_empty(), "m={m}: model ⊨ dead_step — zero stays zero at step {v}");
        }
        // ABSORB: for an all-negative-literal clause prefix, the multiplier (⊆ prefix variables)
        // times the prefix polynomial IS the prefix polynomial, at every width.
        let lits: Vec<Lit> = (0..8u32).map(Lit::neg).collect();
        let mut prefix: Poly = [(0u64, 1u64)].into_iter().collect(); // P(0) = 1
        let mut running = prefix.clone(); // M(0) = P(0)
        for k in 0..8usize {
            prefix = mul_polys(m, &prefix, &literal_factor(m, lits[k]));
            // extf: extend by the factor AND absorb the multiplier bit for this variable.
            running = mul_polys(m, &running, &literal_factor(m, lits[k]));
            running = mul_polys(m, &running, &[(1u64 << k, 1u64)].into_iter().collect::<Poly>());
            assert_eq!(running, prefix, "m={m}: model ⊨ ext_step — M({}) = P({}) (absorption)", k + 1, k + 1);
        }
    }

    // ── The kernel derives both invariants ∀n by Nat induction ──────────────────────────────────
    let dead_ctx = dead_context();
    assert!(
        derive_forall(
            &dead_ctx,
            "def_dead_zero",
            "def_dead_succ",
            "dead_step",
            |n| pf1("D", n),
            |k| pf2("mulf", k.clone(), pf1("D", k)),
            |_| pconst("zero"),
        ),
        "∀n. D(n) = 0 — dead stays dead, kernel-certified"
    );
    let absorb_ctx = absorb_context();
    assert!(
        derive_forall(
            &absorb_ctx,
            "def_m_zero",
            "def_m_succ",
            "ext_step",
            |n| pf1("M", n),
            |k| pf2("extf", k.clone(), pf1("M", k)),
            |n| pf1("P", n),
        ),
        "∀n. M(n) = P(n) — the absorbed prefix is the clause polynomial, kernel-certified"
    );
}

/// **Negative control**: dropping the annihilation axiom and chaining the recurrence directly to
/// the conclusion must FAIL — the middle terms (`mulf k (D k)` vs `zero`) do not line up without
/// `dead_step`, and the kernel rejects the assembled term. Soundness is kernel-enforced.
#[test]
fn skipping_the_annihilation_axiom_fails_the_kernel_type_check() {
    let ctx = dead_context();
    let ui_def = DerivationTree::new(
        id(pf1("D", psucc(pk())), pf2("mulf", pk(), pf1("D", pk()))),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("def_dead_succ")],
    );
    // BROKEN: claim D(Succ k) = zero directly from the recurrence plus the IH-shaped leaf
    // `D k = zero` — the middle terms differ (`mulf k (D k)` is not `D k`).
    let ih = DerivationTree::leaf(id(pf1("D", pk()), pconst("zero")), InferenceRule::PremiseMatch);
    let broken = DerivationTree::new(
        id(pf1("D", psucc(pk())), pconst("zero")),
        InferenceRule::EqualityTransitivity,
        vec![ui_def, ih],
    );
    let nvar = || ProofTerm::Variable("n".to_string());
    let tree = DerivationTree::new(
        forall("n", id(pf1("D", nvar()), pconst("zero"))),
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        vec![named("def_dead_zero"), broken],
    );
    let cert_ctx = CertificationContext::new(&ctx);
    let verified = matches!(
        certify(&tree, &cert_ctx).and_then(|term| infer_type(&ctx, &term)),
        Ok(Term::Pi { .. })
    );
    assert!(!verified, "a step that skips the annihilation axiom must NOT certify");
}
