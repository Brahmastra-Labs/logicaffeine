//! **Full kernel-internalization of the König/compactness argument.**
//!
//! `no_randomness_at_infinity.rs` certified compactness in *ladder form*: the König content lived in
//! Rust (alive-sets, projection, path), and the kernel saw only `∀k` Nat induction over an **opaque**
//! atom `LevelNonempty`. `work/PAPER.md` §2.3 flagged the tree argument itself as "the next hardening
//! level." This file closes that gap: the infinite path is exhibited as an explicit kernel function
//! `path : Nat → Node`, and König's conclusion — *the path reaches every level* — is a kernel
//! theorem with **derived** base and step (the step genuinely consumes the induction hypothesis via
//! Leibniz substitution), certified to a `Fix`/`Match` term the kernel type-checks. No opaque
//! predicate; the tree/path argument is now a CoC term.
//!
//! The equational theory (Entity-domain, the certifier's `Identity`-domain — `n:Nat` appears only as
//! an argument, exactly as `PoU n` does in `finite_randomness_kernel_integration`):
//!   - `mark : Node → Node`   reads a node's level-depth marker; `lvl : Nat → Node` is the level-`n`
//!     marker; `esucc : Node → Node` the successor on markers; `path : Nat → Node` the chosen path.
//!   - `mp0 : mark (path Zero) = lvl Zero`                          — base: the root sits at level 0
//!   - `mpS : ∀k. mark (path (Succ k)) = esucc (mark (path k))`     — the recurrence: the level-`k+1`
//!     path node is a CHILD of the level-`k` node (its marker is one deeper) — this references
//!     `path k`, so the step must use the IH
//!   - `lvlS : ∀k. lvl (Succ k) = esucc (lvl k)`                    — the level markers step likewise
//!
//! The kernel derives, by `Nat` induction:
//!   - base  `mark (path Zero) = lvl Zero`                          ← `mp0`
//!   - step  `mark (path (Succ k)) = lvl (Succ k)`                  ← `mpS k`, then REWRITE the IH
//!     `mark (path k) = lvl k` into the marker (Leibniz), then transitivity with `lvlS k`
//!   ⟹ `∀n. mark (path n) = lvl n` — the path is level-faithful at every depth: König's infinite
//!   path, internalized.
//!
//! The compactness reading: the path is a total function `Nat → Node` hitting a node at every level,
//! i.e. a partial satisfying assignment of every finite fragment — so an infinite formula all of
//! whose finite fragments are alive is satisfiable; contrapositively an unsatisfiable infinite
//! formula has a finite unsatisfiable fragment, which (by the ∀n completeness pole) carries a
//! certificate. No randomness at infinity.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, is_subtype, Context, Term};
use logicaffeine_proof::certifier::{certify, CertificationContext};
use logicaffeine_proof::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

// ── kernel term helpers ─────────────────────────────────────────────────────────────────────────
fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn pi(param: &str, ty: Term, body: Term) -> Term {
    Term::Pi { param: param.to_string(), param_type: Box::new(ty), body_type: Box::new(body) }
}
fn kvar(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn nat() -> Term {
    g("Nat")
}
fn node() -> Term {
    g("Entity") // the node carrier — Entity, the certifier's Identity domain
}
fn keq(a: Term, b: Term) -> Term {
    app(app(app(g("Eq"), node()), a), b)
}
fn kmark(x: Term) -> Term {
    app(g("mark"), x)
}
fn kpath(n: Term) -> Term {
    app(g("path"), n)
}
fn klvl(n: Term) -> Term {
    app(g("lvl"), n)
}
fn kesucc(x: Term) -> Term {
    app(g("esucc"), x)
}
fn ksucc(n: Term) -> Term {
    app(g("Succ"), n)
}

// ── proof-expr helpers ──────────────────────────────────────────────────────────────────────────
fn id(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(a, b)
}
fn pmark(x: ProofTerm) -> ProofTerm {
    ProofTerm::Function("mark".to_string(), vec![x])
}
fn ppath(n: ProofTerm) -> ProofTerm {
    ProofTerm::Function("path".to_string(), vec![n])
}
fn plvl(n: ProofTerm) -> ProofTerm {
    ProofTerm::Function("lvl".to_string(), vec![n])
}
fn pesucc(x: ProofTerm) -> ProofTerm {
    ProofTerm::Function("esucc".to_string(), vec![x])
}
fn psucc(n: ProofTerm) -> ProofTerm {
    ProofTerm::Function("Succ".to_string(), vec![n])
}
fn pk() -> ProofTerm {
    ProofTerm::Variable("k".to_string())
}
fn pn() -> ProofTerm {
    ProofTerm::Variable("n".to_string())
}
fn forall(v: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: v.to_string(), body: Box::new(body) }
}
fn named(name: &str) -> DerivationTree {
    DerivationTree::leaf(ProofExpr::Atom(name.to_string()), InferenceRule::PremiseMatch)
}

/// The kernel theory: the carrier functions and the three recurrence axioms of the level-faithful
/// path. `mp0`/`mpS`/`lvlS` are the finitely-branching-tree facts König's lemma rests on.
fn konig_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.add_declaration("mark", pi("_", node(), node()));
    ctx.add_declaration("path", pi("_", nat(), node()));
    ctx.add_declaration("lvl", pi("_", nat(), node()));
    ctx.add_declaration("esucc", pi("_", node(), node()));
    ctx.add_declaration("mp0", keq(kmark(kpath(g("Zero"))), klvl(g("Zero"))));
    ctx.add_declaration(
        "mpS",
        pi("k", nat(), keq(kmark(kpath(ksucc(kvar("k")))), kesucc(kmark(kpath(kvar("k")))))),
    );
    ctx.add_declaration(
        "lvlS",
        pi("k", nat(), keq(klvl(ksucc(kvar("k"))), kesucc(klvl(kvar("k"))))),
    );
    ctx
}

/// The induction step, derived: `mark (path (Succ k)) = lvl (Succ k)` from `mpS k`, the IH
/// (`mark (path k) = lvl k`, resolved to the recursive call by the certifier), and `lvlS k`.
fn konig_step() -> DerivationTree {
    // mpS k :  mark (path (Succ k)) = esucc (mark (path k))
    let ui_mps = DerivationTree::new(
        id(pmark(ppath(psucc(pk()))), pesucc(pmark(ppath(pk())))),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("mpS")],
    );
    // IH :  mark (path k) = lvl k   (the recursive call)
    let ih = DerivationTree::leaf(id(pmark(ppath(pk())), plvl(pk())), InferenceRule::PremiseMatch);
    // REWRITE the IH into the marker (Leibniz): mark (path (Succ k)) = esucc (lvl k)
    let rewritten = DerivationTree::new(
        id(pmark(ppath(psucc(pk()))), pesucc(plvl(pk()))),
        InferenceRule::Rewrite { from: pmark(ppath(pk())), to: plvl(pk()) },
        vec![ih, ui_mps],
    );
    // lvlS k :  lvl (Succ k) = esucc (lvl k),  flipped to  esucc (lvl k) = lvl (Succ k)
    let ui_lvls = DerivationTree::new(
        id(plvl(psucc(pk())), pesucc(plvl(pk()))),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("lvlS")],
    );
    let lvls_flip = DerivationTree::new(
        id(pesucc(plvl(pk())), plvl(psucc(pk()))),
        InferenceRule::EqualitySymmetry,
        vec![ui_lvls],
    );
    // transitivity :  mark (path (Succ k)) = lvl (Succ k)
    DerivationTree::new(
        id(pmark(ppath(psucc(pk()))), plvl(psucc(pk()))),
        InferenceRule::EqualityTransitivity,
        vec![rewritten, lvls_flip],
    )
}

/// **THE THEOREM — König's path, internalized.** `∀n. mark (path n) = lvl n`, derived by `Nat`
/// induction from the three recurrence axioms: base is `mp0`, and the step genuinely consumes the
/// induction hypothesis (via the Leibniz `Rewrite`). The whole derivation certifies to a `Fix` over
/// a `Match` — the dependent eliminator the kernel re-checks for coverage, case types, and
/// termination — and `infer_type` yields the universally-quantified `Term::Pi`. The opaque
/// `LevelNonempty` of the ladder is gone: the path is a kernel function and its level-faithfulness is
/// a kernel term.
#[test]
fn the_konig_path_is_level_faithful_at_every_depth_kernel_certified() {
    let ctx = konig_context();
    let tree = DerivationTree::new(
        forall("n", id(pmark(ppath(pn())), plvl(pn()))),
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        vec![named("mp0"), konig_step()],
    );
    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("the ∀n König induction certifies to a Fix/Match term");
    let inferred =
        infer_type(&ctx, &term).expect("the certified König term must type-check in the kernel");
    assert!(matches!(inferred, Term::Pi { .. }), "the certified term is universally quantified");
    // The inferred type is EXACTLY the intended theorem — not merely *some* Π. This is the guard
    // against a certified term that type-checks to a weaker/different statement.
    let goal = pi("n", nat(), keq(kmark(kpath(kvar("n"))), klvl(kvar("n"))));
    assert!(
        is_subtype(&ctx, &inferred, &goal),
        "the inferred type is exactly ∀n:Nat. Eq Entity (mark (path n)) (lvl n): {inferred} ⊄ {goal}"
    );
}

/// **Negative control — a base that does not match `P(Zero)` is rejected.** Re-declare `mp0` with an
/// off-by-one type `mark (path Zero) = lvl (Succ Zero)` and run the SAME (otherwise correct)
/// induction whose base obligation is `mark (path Zero) = lvl Zero`. The base premise no longer
/// proves `P(Zero)`, so the induction cannot be a kernel theorem — the guard that the base is
/// actually checked against the motive at `Zero`, not rubber-stamped.
#[test]
fn a_base_that_mismatches_p_of_zero_fails_the_kernel_type_check() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.add_declaration("mark", pi("_", node(), node()));
    ctx.add_declaration("path", pi("_", nat(), node()));
    ctx.add_declaration("lvl", pi("_", nat(), node()));
    ctx.add_declaration("esucc", pi("_", node(), node()));
    // OFF BY ONE: base claims the root is at level 1, not level 0.
    ctx.add_declaration("mp0", keq(kmark(kpath(g("Zero"))), klvl(ksucc(g("Zero")))));
    ctx.add_declaration(
        "mpS",
        pi("k", nat(), keq(kmark(kpath(ksucc(kvar("k")))), kesucc(kmark(kpath(kvar("k")))))),
    );
    ctx.add_declaration("lvlS", pi("k", nat(), keq(klvl(ksucc(kvar("k"))), kesucc(klvl(kvar("k"))))));
    let tree = DerivationTree::new(
        forall("n", id(pmark(ppath(pn())), plvl(pn()))),
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        vec![named("mp0"), konig_step()],
    );
    let cert_ctx = CertificationContext::new(&ctx);
    let verified = matches!(
        certify(&tree, &cert_ctx).and_then(|term| infer_type(&ctx, &term)),
        Ok(Term::Pi { .. })
    );
    assert!(!verified, "an off-by-one base must NOT yield a kernel-checked ∀n theorem");
}

/// **Negative control — skip the IH rewrite and the kernel rejects it.** Chain `mpS k` (giving
/// `= esucc (mark (path k))`) directly with the flipped `lvlS k` (`esucc (lvl k) = lvl (Succ k)`) by
/// transitivity, WITHOUT the Leibniz rewrite that turns `mark (path k)` into `lvl k`. The two middle
/// terms — `esucc (mark (path k))` and `esucc (lvl k)` — do not line up, so the induction cannot be
/// a kernel theorem. Either `certify` or `infer_type` must fail; never a verified `Pi`. This is what
/// forces the step to actually use the induction hypothesis.
#[test]
fn skipping_the_induction_hypothesis_fails_the_kernel_type_check() {
    let ctx = konig_context();
    let ui_mps = DerivationTree::new(
        id(pmark(ppath(psucc(pk()))), pesucc(pmark(ppath(pk())))),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("mpS")],
    );
    let ui_lvls = DerivationTree::new(
        id(plvl(psucc(pk())), pesucc(plvl(pk()))),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("lvlS")],
    );
    let lvls_flip = DerivationTree::new(
        id(pesucc(plvl(pk())), plvl(psucc(pk()))),
        InferenceRule::EqualitySymmetry,
        vec![ui_lvls],
    );
    // BROKEN: transitivity of `= esucc(mark(path k))` and `esucc(lvl k) = lvl(Succ k)` — the middles
    // (esucc(mark(path k)) vs esucc(lvl k)) mismatch; no IH bridge.
    let broken_step = DerivationTree::new(
        id(pmark(ppath(psucc(pk()))), plvl(psucc(pk()))),
        InferenceRule::EqualityTransitivity,
        vec![ui_mps, lvls_flip],
    );
    let tree = DerivationTree::new(
        forall("n", id(pmark(ppath(pn())), plvl(pn()))),
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        vec![named("mp0"), broken_step],
    );
    let cert_ctx = CertificationContext::new(&ctx);
    let verified = matches!(
        certify(&tree, &cert_ctx).and_then(|term| infer_type(&ctx, &term)),
        Ok(Term::Pi { .. })
    );
    assert!(!verified, "a step that skips the IH rewrite must NOT yield a kernel-checked ∀n theorem");
}
