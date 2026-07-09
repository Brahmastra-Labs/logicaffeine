//! **Martin-Löf randomness at the limit, kernel-certified — the incompressibility pole made
//! infinite.** `work/PAPER.md` §2.3 (second bullet, "the poles split") delivered as CoC kernel terms.
//!
//! The finite Chaitin theorem (`ait_kolmogorov.rs`) already certifies that incompressibility is real
//! and unprovable of any *finite* object. This file carries it to the concrete infinite witness —
//! Chaitin's halting probability Ω, a Martin-Löf-random real — and pins the coexistence of the two
//! poles on that one object: **Ω is random in the limit (no structure to compress) while every one
//! of its finite prefixes has structure**. Both are kernel theorems here; together they are the
//! precise "structure exists pointwise, not uniformly" content of the two-poles thesis.
//!
//! Style follows `ait_kolmogorov.rs` exactly: an axiomatic (shallow) development — opaque `Global`
//! constants for the description objects, `Π`-typed axioms for the theorems of algorithmic-
//! randomness (Levin–Schnorr, the Ω incompressibility bound, computable ⇒ compressible), and proof
//! terms assembled and checked by `infer_type` + `is_subtype`, with a negative control that must be
//! REJECTED by the kernel. The axioms are the genuine theorems of the field (each provable, cited);
//! importing them opaquely and deriving their consequences by kernel-checked term application is the
//! same discipline the invariance/Berry development uses.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, is_subtype, Context, Term, Universe};

// ── term-building helpers (mirroring ait_kolmogorov.rs) ─────────────────────────────────────────
fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn int() -> Term {
    g("Int")
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn app(f: Term, xs: Vec<Term>) -> Term {
    xs.into_iter().fold(f, |acc, x| Term::App(Box::new(acc), Box::new(x)))
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi { param: "_".to_string(), param_type: Box::new(a), body_type: Box::new(b) }
}
fn pi(param: &str, ty: Term, body: Term) -> Term {
    Term::Pi { param: param.to_string(), param_type: Box::new(ty), body_type: Box::new(body) }
}
fn lam(param: &str, ty: Term, body: Term) -> Term {
    Term::Lambda { param: param.to_string(), param_type: Box::new(ty), body: Box::new(body) }
}
/// `a ≤ b` as the Prop `Eq Bool (le a b) true` — the prelude's order encoding (as in the AIT test).
fn le(a: Term, b: Term) -> Term {
    app(g("Eq"), vec![g("Bool"), app(g("le"), vec![a, b]), g("true")])
}
fn seqt() -> Term {
    g("Seq")
}
fn kpre(s: Term, n: Term) -> Term {
    app(g("Kpre"), vec![s, n])
}
fn slt(a: Term, b: Term) -> Term {
    app(g("StrictLt"), vec![a, b])
}
fn minus(a: Term, b: Term) -> Term {
    app(g("minus"), vec![a, b])
}
fn mlrandom(s: Term) -> Term {
    app(g("MLRandom"), vec![s])
}
fn computable(s: Term) -> Term {
    app(g("Computable"), vec![s])
}
fn has_cert(s: Term, n: Term) -> Term {
    app(g("HasStructureCert"), vec![s, n])
}

/// The algorithmic-randomness axiom system over infinite binary sequences (`Seq`), anchored on the
/// halting probability Ω. Every declaration is a theorem of the field; they are introduced opaquely
/// and their consequences derived by kernel-checked application.
fn register_randomness_system(c: &mut Context) {
    // Objects.
    c.add_declaration("Seq", ty0()); // infinite binary sequences
    c.add_declaration("Omega", seqt()); // Chaitin's Ω
    c.add_declaration("Kpre", arrow(seqt(), arrow(int(), int()))); // K(s ↾ n) — prefix complexity
    c.add_declaration("MLRandom", arrow(seqt(), prop()));
    c.add_declaration("Computable", arrow(seqt(), prop()));
    c.add_declaration("HasStructureCert", arrow(seqt(), arrow(int(), prop()))); // finite-prefix structure
    // Numeric scaffolding (kept opaque, self-contained — the AIT test declares its own too).
    c.add_declaration("minus", arrow(int(), arrow(int(), int())));
    c.add_declaration("StrictLt", arrow(int(), arrow(int(), prop())));
    c.add_declaration("cK", int()); // the incompressibility constant c
    c.add_declaration("nBig", int()); // a scale index with nBig − c ≥ c

    // The theorems of the field, as Π-typed axioms.

    // Levin–Schnorr (one direction): a sequence incompressible at every prefix (K(s↾n) > n − c for
    // all n) is Martin-Löf random.
    c.add_declaration(
        "levin_schnorr",
        pi(
            "s",
            seqt(),
            arrow(pi("n", int(), slt(minus(v("n"), g("cK")), kpre(v("s"), v("n")))), mlrandom(v("s"))),
        ),
    );

    // The Ω incompressibility bound (Chaitin): K(Ω ↾ n) > n − c for every n. This is exactly the
    // finite Chaitin theorem's engine, quantified over all prefix lengths — the limit statement.
    c.add_declaration(
        "omega_incompressible",
        pi("n", int(), slt(minus(v("n"), g("cK")), kpre(g("Omega"), v("n")))),
    );

    // Computable ⇒ boundedly compressible: a computable sequence's prefix complexity is bounded by a
    // constant at the scale index (a program of size c prints it), so K(s ↾ nBig) ≤ c.
    c.add_declaration(
        "computable_bounded",
        pi("s", seqt(), arrow(computable(v("s")), le(kpre(v("s"), g("nBig")), g("cK")))),
    );

    // The scale fact: at nBig the incompressibility threshold clears the compressibility bound,
    // c ≤ nBig − c (true once nBig ≥ 2c). A declared arithmetic fact, as the AIT test declares
    // `berry_length`.
    c.add_declaration("scale_gap", le(g("cK"), minus(g("nBig"), g("cK"))));

    // Strict-vs-nonstrict antisymmetry: a < b and b ≤ a is absurd (as `ord_antisym` in the AIT test).
    c.add_declaration(
        "lt_le_absurd",
        pi("a", int(), pi("b", int(), arrow(slt(v("a"), v("b")), arrow(le(v("b"), v("a")), g("False"))))),
    );

    // The completeness pole, imported: every finite prefix of every sequence has a structure
    // certificate (the ∀n `no_finite_randomness` theorem, kernel-certified in its own file).
    c.add_declaration("finite_completeness", pi("s", seqt(), pi("n", int(), has_cert(v("s"), v("n")))));
}

/// **THEOREM 1 — Ω is Martin-Löf random.** The incompressibility bound feeds Levin–Schnorr directly:
/// `levin_schnorr Omega omega_incompressible : MLRandom Omega`. Kernel-checked — the infinite
/// incompressibility pole, on the concrete witness.
#[test]
fn omega_is_martin_lof_random_is_kernel_certified() {
    let mut c = ctx();
    register_randomness_system(&mut c);
    let proof = app(g("levin_schnorr"), vec![g("Omega"), g("omega_incompressible")]);
    let inferred = infer_type(&c, &proof).expect("the ML-randomness proof term must type-check");
    let goal = mlrandom(g("Omega"));
    assert!(is_subtype(&c, &inferred, &goal), "Ω is Martin-Löf random: {inferred} ⊄ {goal}");
}

/// **THEOREM 2 — Ω is not computable.** A computable sequence is boundedly compressible at every
/// scale, but Ω is incompressible past the scale threshold; the two collide at `nBig`. Assemble
/// `Computable Omega → False`: from `h : Computable Omega`, `computable_bounded` gives
/// `K(Ω↾nBig) ≤ c`, `scale_gap` gives `c ≤ nBig − c`, transitivity gives `K(Ω↾nBig) ≤ nBig − c`,
/// while `omega_incompressible nBig` gives `nBig − c < K(Ω↾nBig)` — `lt_le_absurd` closes it. The
/// same `le_trans`/antisymmetry contradiction shape as the Berry theorem.
#[test]
fn omega_is_not_computable_is_kernel_certified() {
    let mut c = ctx();
    register_randomness_system(&mut c);
    // K(Ω ↾ nBig) ≤ c   (from computability)
    let bounded = app(g("computable_bounded"), vec![g("Omega"), v("h")]);
    // K(Ω ↾ nBig) ≤ nBig − c   (chain with the scale gap)
    let chained = app(
        g("le_trans"),
        vec![kpre(g("Omega"), g("nBig")), g("cK"), minus(g("nBig"), g("cK")), bounded, g("scale_gap")],
    );
    // nBig − c < K(Ω ↾ nBig)   (incompressibility at nBig)
    let incompressible = app(g("omega_incompressible"), vec![g("nBig")]);
    // absurd: (nBig − c < K) ∧ (K ≤ nBig − c)
    let contra = app(
        g("lt_le_absurd"),
        vec![minus(g("nBig"), g("cK")), kpre(g("Omega"), g("nBig")), incompressible, chained],
    );
    let not_computable = lam("h", computable(g("Omega")), contra);
    let inferred = infer_type(&c, &not_computable).expect("the ¬-computability proof must type-check");
    let goal = arrow(computable(g("Omega")), g("False"));
    assert!(is_subtype(&c, &inferred, &goal), "Ω is not computable: {inferred} ⊄ {goal}");
}

/// **THEOREM 3 — the two poles coexist on Ω (the limit "poles split").** Ω is Martin-Löf random
/// (Theorem 1 — incompressibility, no uniform structure) AND every finite prefix of Ω has a
/// structure certificate (the completeness pole). Assemble the conjunction
/// `And (MLRandom Omega) (Π n. HasStructureCert Omega n)` via `conj` — one kernel object carrying
/// both poles: structure exists at every finite prefix, randomness reigns in the limit.
#[test]
fn the_two_poles_coexist_on_omega_is_kernel_certified() {
    let mut c = ctx();
    register_randomness_system(&mut c);
    let random = app(g("levin_schnorr"), vec![g("Omega"), g("omega_incompressible")]);
    let every_prefix_structured =
        lam("n", int(), app(g("finite_completeness"), vec![g("Omega"), v("n")]));
    let p = mlrandom(g("Omega"));
    let q = pi("n", int(), has_cert(g("Omega"), v("n")));
    let proof = app(g("conj"), vec![p.clone(), q.clone(), random, every_prefix_structured]);
    let inferred = infer_type(&c, &proof).expect("the two-poles conjunction must type-check");
    let goal = app(g("And"), vec![p, q]);
    assert!(is_subtype(&c, &inferred, &goal), "both poles hold of Ω: {inferred} ⊄ {goal}");
}

/// **Negative control.** Feeding `computable_bounded` (which needs a `Computable` witness and yields
/// a `≤` fact) where Levin–Schnorr demands the `∀n` incompressibility witness must be REJECTED by
/// the kernel — the sole assertion is that `infer_type` FAILS. Guards the development against a
/// well-typed-by-accident collapse, exactly as the AIT test's Decode-V-for-Decode-U control does.
#[test]
fn a_wrong_witness_does_not_type_check_as_randomness() {
    let mut c = ctx();
    register_randomness_system(&mut c);
    // `computable_bounded Omega` is `Computable Omega → (K(Ω↾nBig) ≤ c)`, NOT the ∀n incompressibility
    // premise Levin–Schnorr requires. Handing it to `levin_schnorr Omega` must not type-check.
    let bogus = app(g("levin_schnorr"), vec![g("Omega"), app(g("computable_bounded"), vec![g("Omega")])]);
    assert!(
        infer_type(&c, &bogus).is_err(),
        "a Computable-bounded fact must be rejected where the ∀n incompressibility witness is required"
    );
}

/// **Negative control — the two-poles conjunction needs BOTH poles honestly.** Assemble `conj` with
/// the correct `MLRandom Omega` proof but a WRONG second argument (the randomness proof again, whose
/// type is `MLRandom Omega`, not the structure-pole `∀n. HasStructureCert Omega n`). The kernel must
/// reject it — the conjunction cannot be forged from one pole twice. Guards against a two-poles
/// claim that is secretly one pole.
#[test]
fn the_two_poles_conjunction_rejects_a_wrong_second_pole() {
    let mut c = ctx();
    register_randomness_system(&mut c);
    let random = app(g("levin_schnorr"), vec![g("Omega"), g("omega_incompressible")]);
    let p = mlrandom(g("Omega"));
    let q = pi("n", int(), has_cert(g("Omega"), v("n")));
    // The 4th argument must prove `q`; handing it `random` (which proves `p`) must not type-check.
    let forged = app(g("conj"), vec![p, q, random.clone(), random]);
    assert!(
        infer_type(&c, &forged).is_err(),
        "the structure pole cannot be forged from the randomness proof — the kernel must reject it"
    );
}

/// **Robustness — the incompressibility contradiction needs the SCALE, not just the bound.** Without
/// `scale_gap` (`c ≤ nBig − c`), the compressibility bound `K(Ω↾nBig) ≤ c` and the incompressibility
/// `nBig − c < K(Ω↾nBig)` cannot be chained into a contradiction (there is no `le_trans` bridge). We
/// confirm the honest structure by rebuilding Theorem 2's contradiction WITHOUT the scale bridge and
/// checking the kernel rejects it — the scale hypothesis is load-bearing, not decorative.
#[test]
fn the_not_computable_contradiction_is_load_bearing_on_the_scale_gap() {
    let mut c = ctx();
    register_randomness_system(&mut c);
    let bounded = app(g("computable_bounded"), vec![g("Omega"), v("h")]);
    let incompressible = app(g("omega_incompressible"), vec![g("nBig")]);
    // SKIP scale_gap: feed the raw `K ≤ c` bound where `lt_le_absurd` needs `K ≤ nBig − c`.
    let contra = app(
        g("lt_le_absurd"),
        vec![minus(g("nBig"), g("cK")), kpre(g("Omega"), g("nBig")), incompressible, bounded],
    );
    let broken = lam("h", computable(g("Omega")), contra);
    assert!(
        infer_type(&c, &broken).is_err(),
        "without the scale gap the compressibility bound cannot close the contradiction — rejected"
    );
}
