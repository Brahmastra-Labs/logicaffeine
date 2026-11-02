//! **A general ∀n linear-algebra theorem, by kernel induction: trace additivity.**
//!
//! `linalg_certify` proves a solve for a SPECIFIC numeric instance. This closes the other half of
//! "prove, not just run": a theorem that holds for ALL sizes `n`, by natural-number induction the
//! kernel type-checks — the same machinery as `konig_compactness_kernel.rs`.
//!
//! The theorem is `trace(A + B) = trace(A) + trace(B)` for every n×n pair. Trace is the running sum of
//! the diagonal, so it is a `Nat`-recursion; abstracting the diagonals as opaque `Nat → Int` sequences
//! `s` (of A), `u` (of B) — the (i,i) entry of `A+B` is `s i + u i` — the three traces satisfy
//!   - `tr : Nat → Int`  trace of A+B      `ts`  trace of A      `tu`  trace of B
//!   - base   `tr Zero = ts Zero + tu Zero`                         (0×0 trace is 0 = 0 + 0)
//!   - `tr (Succ k) = tr k + (s k + u k)`   `ts (Succ k) = ts k + s k`   `tu (Succ k) = tu k + u k`
//!
//! The kernel derives, by induction on n:
//!   base  `tr Zero = ts Zero + tu Zero`                                   ← the base axiom
//!   step  from IH `tr k = ts k + tu k`:
//!         `tr (Succ k) = (ts k + tu k) + (s k + u k)`                     (tr_succ, then REWRITE the IH)
//!                     = (ts k + s k) + (tu k + u k)                        (Int RING rearrange — the crux)
//!                     = ts (Succ k) + tu (Succ k)                          (ts_succ, tu_succ, flipped)
//!   ⟹ `∀n. tr n = ts n + tu n` — trace additivity at every size, internalized as a `Fix`/`Match`
//!   the kernel re-checks. The ring rearrangement is discharged by the arithmetic oracle over the
//!   opaque atoms `ts k, tu k, s k, u k` (holds for ALL entries), then kernel-verified.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, is_subtype, Context, Literal, Term};
use logicaffeine_proof::certifier::{certify, CertificationContext};
use logicaffeine_proof::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

// ── kernel term helpers ──────────────────────────────────────────────────────────────────────────
fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn pi(param: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: param.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn kvar(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn nat() -> Term {
    g("Nat")
}
fn intt() -> Term {
    g("Int")
}
fn keqi(a: Term, b: Term) -> Term {
    app(app(app(g("Eq"), intt()), a), b)
}
fn kadd(a: Term, b: Term) -> Term {
    app(app(g("add"), a), b)
}
fn kmul(a: Term, b: Term) -> Term {
    app(app(g("mul"), a), b)
}
fn kf(name: &str, x: Term) -> Term {
    app(g(name), x)
}
fn ksucc(n: Term) -> Term {
    app(g("Succ"), n)
}
fn klit(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}

// ── proof-expr / proof-term helpers ───────────────────────────────────────────────────────────────
fn id(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(a, b)
}
fn pf(name: &str, x: ProofTerm) -> ProofTerm {
    ProofTerm::Function(name.to_string(), vec![x])
}
fn padd(a: ProofTerm, b: ProofTerm) -> ProofTerm {
    ProofTerm::Function("add".to_string(), vec![a, b])
}
fn pmul(a: ProofTerm, b: ProofTerm) -> ProofTerm {
    ProofTerm::Function("mul".to_string(), vec![a, b])
}
fn pc() -> ProofTerm {
    ProofTerm::Constant("c".to_string())
}
fn palpha() -> ProofTerm {
    ProofTerm::Constant("alpha".to_string())
}
fn pbeta() -> ProofTerm {
    ProofTerm::Constant("beta".to_string())
}
fn psucc(n: ProofTerm) -> ProofTerm {
    ProofTerm::Function("Succ".to_string(), vec![n])
}
fn pone() -> ProofTerm {
    ProofTerm::Constant("1".to_string())
}
fn pk() -> ProofTerm {
    ProofTerm::Variable("k".to_string())
}
fn pn() -> ProofTerm {
    ProofTerm::Variable("n".to_string())
}
fn forall(v: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll {
        variable: v.to_string(),
        body: Box::new(body),
    }
}
fn named(name: &str) -> DerivationTree {
    DerivationTree::leaf(
        ProofExpr::Atom(name.to_string()),
        InferenceRule::PremiseMatch,
    )
}

/// The kernel theory: three trace sequences and the two diagonal sequences, with the base fact and
/// the three `Succ` recurrences that define trace as the running diagonal sum.
fn trace_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    for f in ["tr", "ts", "tu", "s", "u"] {
        ctx.add_declaration(f, pi("_", nat(), intt()));
    }
    // base: tr Zero = ts Zero + tu Zero
    ctx.add_declaration(
        "base",
        keqi(
            kf("tr", g("Zero")),
            kadd(kf("ts", g("Zero")), kf("tu", g("Zero"))),
        ),
    );
    // tr (Succ k) = tr k + (s k + u k)   — the k-th diagonal of A+B is s k + u k
    ctx.add_declaration(
        "tr_succ",
        pi(
            "k",
            nat(),
            keqi(
                kf("tr", ksucc(kvar("k"))),
                kadd(
                    kf("tr", kvar("k")),
                    kadd(kf("s", kvar("k")), kf("u", kvar("k"))),
                ),
            ),
        ),
    );
    // ts (Succ k) = ts k + s k
    ctx.add_declaration(
        "ts_succ",
        pi(
            "k",
            nat(),
            keqi(
                kf("ts", ksucc(kvar("k"))),
                kadd(kf("ts", kvar("k")), kf("s", kvar("k"))),
            ),
        ),
    );
    // tu (Succ k) = tu k + u k
    ctx.add_declaration(
        "tu_succ",
        pi(
            "k",
            nat(),
            keqi(
                kf("tu", ksucc(kvar("k"))),
                kadd(kf("tu", kvar("k")), kf("u", kvar("k"))),
            ),
        ),
    );
    ctx
}

/// The induction STEP: from IH `tr k = ts k + tu k`, derive `tr (Succ k) = ts (Succ k) + tu (Succ k)`.
fn trace_step() -> DerivationTree {
    let tr_k = pf("tr", pk());
    let ts_k = pf("ts", pk());
    let tu_k = pf("tu", pk());
    let s_k = pf("s", pk());
    let u_k = pf("u", pk());
    let tr_sk = pf("tr", psucc(pk()));
    let ts_sk = pf("ts", psucc(pk()));
    let tu_sk = pf("tu", psucc(pk()));

    // tr_succ k :  tr (Succ k) = tr k + (s k + u k)
    let t1 = DerivationTree::new(
        id(
            tr_sk.clone(),
            padd(tr_k.clone(), padd(s_k.clone(), u_k.clone())),
        ),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("tr_succ")],
    );
    // IH :  tr k = ts k + tu k   (resolved to the recursive call by the certifier)
    let ih = DerivationTree::leaf(
        id(tr_k.clone(), padd(ts_k.clone(), tu_k.clone())),
        InferenceRule::PremiseMatch,
    );
    // REWRITE the IH into t1's RHS:  tr (Succ k) = (ts k + tu k) + (s k + u k)
    let t2 = DerivationTree::new(
        id(
            tr_sk.clone(),
            padd(
                padd(ts_k.clone(), tu_k.clone()),
                padd(s_k.clone(), u_k.clone()),
            ),
        ),
        InferenceRule::Rewrite {
            from: tr_k.clone(),
            to: padd(ts_k.clone(), tu_k.clone()),
        },
        vec![ih, t1],
    );
    // RING rearrange:  (ts k + tu k) + (s k + u k) = (ts k + s k) + (tu k + u k)
    let ring = DerivationTree::leaf(
        id(
            padd(
                padd(ts_k.clone(), tu_k.clone()),
                padd(s_k.clone(), u_k.clone()),
            ),
            padd(
                padd(ts_k.clone(), s_k.clone()),
                padd(tu_k.clone(), u_k.clone()),
            ),
        ),
        InferenceRule::ArithDecision,
    );
    // transitivity:  tr (Succ k) = (ts k + s k) + (tu k + u k)
    let t4 = DerivationTree::new(
        id(
            tr_sk.clone(),
            padd(
                padd(ts_k.clone(), s_k.clone()),
                padd(tu_k.clone(), u_k.clone()),
            ),
        ),
        InferenceRule::EqualityTransitivity,
        vec![t2, ring],
    );
    // ts_succ k, flipped:  (ts k + s k) = ts (Succ k)
    let ts_flip = DerivationTree::new(
        id(padd(ts_k.clone(), s_k.clone()), ts_sk.clone()),
        InferenceRule::EqualitySymmetry,
        vec![DerivationTree::new(
            id(ts_sk.clone(), padd(ts_k.clone(), s_k.clone())),
            InferenceRule::UniversalInst("k".to_string()),
            vec![named("ts_succ")],
        )],
    );
    // REWRITE:  tr (Succ k) = ts (Succ k) + (tu k + u k)
    let t5 = DerivationTree::new(
        id(
            tr_sk.clone(),
            padd(ts_sk.clone(), padd(tu_k.clone(), u_k.clone())),
        ),
        InferenceRule::Rewrite {
            from: padd(ts_k.clone(), s_k.clone()),
            to: ts_sk.clone(),
        },
        vec![ts_flip, t4],
    );
    // tu_succ k, flipped:  (tu k + u k) = tu (Succ k)
    let tu_flip = DerivationTree::new(
        id(padd(tu_k.clone(), u_k.clone()), tu_sk.clone()),
        InferenceRule::EqualitySymmetry,
        vec![DerivationTree::new(
            id(tu_sk.clone(), padd(tu_k.clone(), u_k.clone())),
            InferenceRule::UniversalInst("k".to_string()),
            vec![named("tu_succ")],
        )],
    );
    // REWRITE:  tr (Succ k) = ts (Succ k) + tu (Succ k)   — this is P(Succ k)
    DerivationTree::new(
        id(tr_sk, padd(ts_sk, tu_sk)),
        InferenceRule::Rewrite {
            from: padd(tu_k, u_k),
            to: pf("tu", psucc(pk())),
        },
        vec![tu_flip, t5],
    )
}

/// **THE THEOREM — trace additivity at every size.** `∀n. tr n = ts n + tu n`, derived by `Nat`
/// induction: base is the axiom, the step genuinely consumes the IH (Leibniz rewrite) and the Int
/// ring rearrangement (arithmetic oracle), and the whole thing certifies to a `Fix` over a `Match`
/// the kernel type-checks — `infer_type` yielding the universally-quantified `Term::Pi`.
#[test]
fn trace_additivity_holds_at_every_size_kernel_certified() {
    let ctx = trace_context();
    let tree = DerivationTree::new(
        forall(
            "n",
            id(pf("tr", pn()), padd(pf("ts", pn()), pf("tu", pn()))),
        ),
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        vec![named("base"), trace_step()],
    );
    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("the ∀n trace-additivity induction certifies");
    let inferred = infer_type(&ctx, &term).expect("the certified trace term must type-check");
    assert!(
        matches!(inferred, Term::Pi { .. }),
        "the certified term is universally quantified"
    );
    // the inferred type is EXACTLY the theorem, not merely some Π.
    let goal = pi(
        "n",
        nat(),
        keqi(
            kf("tr", kvar("n")),
            kadd(kf("ts", kvar("n")), kf("tu", kvar("n"))),
        ),
    );
    assert!(
        is_subtype(&ctx, &inferred, &goal),
        "inferred type must be ∀n:Nat. Eq Int (tr n) (add (ts n) (tu n)): {inferred} ⊄ {goal}"
    );
}

/// **Negative control — drop the ring rearrangement and the kernel rejects it.** Skip the
/// `ArithDecision` step and chain the rewritten IH straight into the ts/tu flips; the middle terms
/// `(ts k + tu k)+(s k + u k)` and `(ts k + s k)+(tu k + u k)` are not definitionally equal, so the
/// step cannot type-check. This is what forces the proof to actually use the ring algebra.
#[test]
fn skipping_the_ring_rearrangement_fails_the_kernel_type_check() {
    let ctx = trace_context();
    let tr_k = pf("tr", pk());
    let ts_k = pf("ts", pk());
    let tu_k = pf("tu", pk());
    let s_k = pf("s", pk());
    let u_k = pf("u", pk());
    let tr_sk = pf("tr", psucc(pk()));
    let ts_sk = pf("ts", psucc(pk()));
    let tu_sk = pf("tu", psucc(pk()));

    let t1 = DerivationTree::new(
        id(
            tr_sk.clone(),
            padd(tr_k.clone(), padd(s_k.clone(), u_k.clone())),
        ),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("tr_succ")],
    );
    let ih = DerivationTree::leaf(
        id(tr_k.clone(), padd(ts_k.clone(), tu_k.clone())),
        InferenceRule::PremiseMatch,
    );
    let t2 = DerivationTree::new(
        id(
            tr_sk.clone(),
            padd(
                padd(ts_k.clone(), tu_k.clone()),
                padd(s_k.clone(), u_k.clone()),
            ),
        ),
        InferenceRule::Rewrite {
            from: tr_k,
            to: padd(ts_k.clone(), tu_k.clone()),
        },
        vec![ih, t1],
    );
    // BROKEN: rewrite (ts k + s k) → ts(Succ k) in t2 WITHOUT first rearranging — `(ts k + s k)` is
    // not a subterm of t2's RHS `(ts k + tu k)+(s k + u k)`, so the rewrite cannot land.
    let ts_flip = DerivationTree::new(
        id(padd(ts_k.clone(), s_k.clone()), ts_sk.clone()),
        InferenceRule::EqualitySymmetry,
        vec![DerivationTree::new(
            id(ts_sk.clone(), padd(ts_k.clone(), s_k.clone())),
            InferenceRule::UniversalInst("k".to_string()),
            vec![named("ts_succ")],
        )],
    );
    let broken = DerivationTree::new(
        id(tr_sk, padd(ts_sk, tu_sk)),
        InferenceRule::Rewrite {
            from: padd(ts_k, s_k),
            to: pf("ts", psucc(pk())),
        },
        vec![ts_flip, t2],
    );
    let tree = DerivationTree::new(
        forall(
            "n",
            id(pf("tr", pn()), padd(pf("ts", pn()), pf("tu", pn()))),
        ),
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        vec![named("base"), broken],
    );
    let cert_ctx = CertificationContext::new(&ctx);
    let verified = matches!(
        certify(&tree, &cert_ctx).and_then(|t| infer_type(&ctx, &t)),
        Ok(Term::Pi { .. })
    );
    assert!(
        !verified,
        "skipping the ring rearrangement must NOT yield a kernel-checked ∀n theorem"
    );
}

// ══ Trace scalar-homogeneity: trace(c·A) = c·trace(A) for every size, by induction. ═════════════════
// The other half of linearity. Trace of c·A accumulates c times each diagonal entry, so the step
// needs the DISTRIBUTIVE law `c·(tr k) + c·(d k) = c·(tr k + d k)` — a different ring identity than
// additivity's rearrangement — discharged by the same arithmetic oracle over the opaque atoms.

fn scale_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    for f in ["trs", "tr", "d"] {
        ctx.add_declaration(f, pi("_", nat(), intt()));
    }
    ctx.add_declaration("c", intt()); // the opaque scalar
                                      // base: trs Zero = c · tr Zero
    ctx.add_declaration(
        "base_s",
        keqi(kf("trs", g("Zero")), kmul(g("c"), kf("tr", g("Zero")))),
    );
    // trs (Succ k) = trs k + c · d k
    ctx.add_declaration(
        "trs_succ",
        pi(
            "k",
            nat(),
            keqi(
                kf("trs", ksucc(kvar("k"))),
                kadd(kf("trs", kvar("k")), kmul(g("c"), kf("d", kvar("k")))),
            ),
        ),
    );
    // tr (Succ k) = tr k + d k
    ctx.add_declaration(
        "tr_succ",
        pi(
            "k",
            nat(),
            keqi(
                kf("tr", ksucc(kvar("k"))),
                kadd(kf("tr", kvar("k")), kf("d", kvar("k"))),
            ),
        ),
    );
    ctx
}

fn scale_step() -> DerivationTree {
    let trs_k = pf("trs", pk());
    let tr_k = pf("tr", pk());
    let d_k = pf("d", pk());
    let trs_sk = pf("trs", psucc(pk()));
    let tr_sk = pf("tr", psucc(pk()));

    // trs_succ k :  trs (Succ k) = trs k + c · d k
    let t1 = DerivationTree::new(
        id(trs_sk.clone(), padd(trs_k.clone(), pmul(pc(), d_k.clone()))),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("trs_succ")],
    );
    // IH :  trs k = c · tr k   (the recursive call)
    let ih = DerivationTree::leaf(
        id(trs_k.clone(), pmul(pc(), tr_k.clone())),
        InferenceRule::PremiseMatch,
    );
    // REWRITE the IH :  trs (Succ k) = c·tr k + c·d k
    let t2 = DerivationTree::new(
        id(
            trs_sk.clone(),
            padd(pmul(pc(), tr_k.clone()), pmul(pc(), d_k.clone())),
        ),
        InferenceRule::Rewrite {
            from: trs_k,
            to: pmul(pc(), tr_k.clone()),
        },
        vec![ih, t1],
    );
    // DISTRIBUTIVITY :  c·tr k + c·d k = c·(tr k + d k)
    let distrib = DerivationTree::leaf(
        id(
            padd(pmul(pc(), tr_k.clone()), pmul(pc(), d_k.clone())),
            pmul(pc(), padd(tr_k.clone(), d_k.clone())),
        ),
        InferenceRule::ArithDecision,
    );
    let t4 = DerivationTree::new(
        id(trs_sk.clone(), pmul(pc(), padd(tr_k.clone(), d_k.clone()))),
        InferenceRule::EqualityTransitivity,
        vec![t2, distrib],
    );
    // tr_succ k, flipped :  (tr k + d k) = tr (Succ k)
    let tr_flip = DerivationTree::new(
        id(padd(tr_k.clone(), d_k.clone()), tr_sk.clone()),
        InferenceRule::EqualitySymmetry,
        vec![DerivationTree::new(
            id(tr_sk.clone(), padd(tr_k.clone(), d_k.clone())),
            InferenceRule::UniversalInst("k".to_string()),
            vec![named("tr_succ")],
        )],
    );
    // REWRITE (inside c·_) :  trs (Succ k) = c · tr (Succ k)  — this is P(Succ k)
    DerivationTree::new(
        id(trs_sk, pmul(pc(), tr_sk.clone())),
        InferenceRule::Rewrite {
            from: padd(tr_k, d_k),
            to: tr_sk,
        },
        vec![tr_flip, t4],
    )
}

/// **∀n. trace(c·A) = c · trace(A)**, by `Nat` induction with the distributive law in the step.
#[test]
fn trace_scalar_homogeneity_holds_at_every_size_kernel_certified() {
    let ctx = scale_context();
    let tree = DerivationTree::new(
        forall("n", id(pf("trs", pn()), pmul(pc(), pf("tr", pn())))),
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        vec![named("base_s"), scale_step()],
    );
    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("scalar-homogeneity induction certifies");
    let inferred = infer_type(&ctx, &term).expect("the certified term must type-check");
    let goal = pi(
        "n",
        nat(),
        keqi(kf("trs", kvar("n")), kmul(g("c"), kf("tr", kvar("n")))),
    );
    assert!(
        is_subtype(&ctx, &inferred, &goal),
        "inferred type must be ∀n. Eq Int (trs n) (mul c (tr n)): {inferred} ⊄ {goal}"
    );
}

// ══ Full linearity: trace(αA + βB) = α·trace(A) + β·trace(B) for every size. ════════════════════════
// The capstone — additivity AND homogeneity in one theorem. The diagonal of αA+βB is α·saₖ + β·sbₖ,
// so the step's single ring identity `(α·tra+β·trb)+(α·sa+β·sb) = α·(tra+sa)+β·(trb+sb)` folds BOTH a
// rearrangement and a distribution; `prove_int_eq` normalizes both sides to the same 4-monomial
// polynomial in one shot.

fn linear_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    for f in ["trc", "tra", "trb", "sa", "sb"] {
        ctx.add_declaration(f, pi("_", nat(), intt()));
    }
    ctx.add_declaration("alpha", intt());
    ctx.add_declaration("beta", intt());
    // base: trc Zero = α·(tra Zero) + β·(trb Zero)
    ctx.add_declaration(
        "base_l",
        keqi(
            kf("trc", g("Zero")),
            kadd(
                kmul(g("alpha"), kf("tra", g("Zero"))),
                kmul(g("beta"), kf("trb", g("Zero"))),
            ),
        ),
    );
    // trc (Succ k) = trc k + (α·sa k + β·sb k)
    ctx.add_declaration(
        "trc_succ",
        pi(
            "k",
            nat(),
            keqi(
                kf("trc", ksucc(kvar("k"))),
                kadd(
                    kf("trc", kvar("k")),
                    kadd(
                        kmul(g("alpha"), kf("sa", kvar("k"))),
                        kmul(g("beta"), kf("sb", kvar("k"))),
                    ),
                ),
            ),
        ),
    );
    for (t, s) in [("tra", "sa"), ("trb", "sb")] {
        ctx.add_declaration(
            &format!("{t}_succ"),
            pi(
                "k",
                nat(),
                keqi(
                    kf(t, ksucc(kvar("k"))),
                    kadd(kf(t, kvar("k")), kf(s, kvar("k"))),
                ),
            ),
        );
    }
    ctx
}

fn linear_step() -> DerivationTree {
    let (a, b) = (palpha(), pbeta());
    let trc_k = pf("trc", pk());
    let tra_k = pf("tra", pk());
    let trb_k = pf("trb", pk());
    let sa_k = pf("sa", pk());
    let sb_k = pf("sb", pk());
    let trc_sk = pf("trc", psucc(pk()));
    let tra_sk = pf("tra", psucc(pk()));
    let trb_sk = pf("trb", psucc(pk()));
    let diag = padd(pmul(a.clone(), sa_k.clone()), pmul(b.clone(), sb_k.clone()));
    let ih_rhs = padd(
        pmul(a.clone(), tra_k.clone()),
        pmul(b.clone(), trb_k.clone()),
    );

    // trc_succ k :  trc (Succ k) = trc k + (α·sa k + β·sb k)
    let t1 = DerivationTree::new(
        id(trc_sk.clone(), padd(trc_k.clone(), diag.clone())),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("trc_succ")],
    );
    let ih = DerivationTree::leaf(
        id(trc_k.clone(), ih_rhs.clone()),
        InferenceRule::PremiseMatch,
    );
    // REWRITE the IH :  trc (Succ k) = (α·tra k + β·trb k) + (α·sa k + β·sb k)
    let t2 = DerivationTree::new(
        id(trc_sk.clone(), padd(ih_rhs.clone(), diag.clone())),
        InferenceRule::Rewrite {
            from: trc_k,
            to: ih_rhs.clone(),
        },
        vec![ih, t1],
    );
    // RING (rearrange + distribute in one) :  = α·(tra k + sa k) + β·(trb k + sb k)
    let combined = padd(
        pmul(a.clone(), padd(tra_k.clone(), sa_k.clone())),
        pmul(b.clone(), padd(trb_k.clone(), sb_k.clone())),
    );
    let ring = DerivationTree::leaf(
        id(padd(ih_rhs, diag), combined.clone()),
        InferenceRule::ArithDecision,
    );
    let t4 = DerivationTree::new(
        id(trc_sk.clone(), combined),
        InferenceRule::EqualityTransitivity,
        vec![t2, ring],
    );
    // tra_succ k flipped, then REWRITE : trc (Succ k) = α·tra(Succ k) + β·(trb k + sb k)
    let tra_flip = DerivationTree::new(
        id(padd(tra_k.clone(), sa_k.clone()), tra_sk.clone()),
        InferenceRule::EqualitySymmetry,
        vec![DerivationTree::new(
            id(tra_sk.clone(), padd(tra_k.clone(), sa_k.clone())),
            InferenceRule::UniversalInst("k".to_string()),
            vec![named("tra_succ")],
        )],
    );
    let t5 = DerivationTree::new(
        id(
            trc_sk.clone(),
            padd(
                pmul(a.clone(), tra_sk.clone()),
                pmul(b.clone(), padd(trb_k.clone(), sb_k.clone())),
            ),
        ),
        InferenceRule::Rewrite {
            from: padd(tra_k, sa_k),
            to: tra_sk.clone(),
        },
        vec![tra_flip, t4],
    );
    // trb_succ k flipped, then REWRITE : trc (Succ k) = α·tra(Succ k) + β·trb(Succ k)  — P(Succ k)
    let trb_flip = DerivationTree::new(
        id(padd(trb_k.clone(), sb_k.clone()), trb_sk.clone()),
        InferenceRule::EqualitySymmetry,
        vec![DerivationTree::new(
            id(trb_sk.clone(), padd(trb_k.clone(), sb_k.clone())),
            InferenceRule::UniversalInst("k".to_string()),
            vec![named("trb_succ")],
        )],
    );
    DerivationTree::new(
        id(trc_sk, padd(pmul(a, tra_sk), pmul(b, trb_sk.clone()))),
        InferenceRule::Rewrite {
            from: padd(trb_k, sb_k),
            to: trb_sk,
        },
        vec![trb_flip, t5],
    )
}

/// **∀n. trace(αA + βB) = α·trace(A) + β·trace(B)** — full linearity, by `Nat` induction.
#[test]
fn trace_full_linearity_holds_at_every_size_kernel_certified() {
    let ctx = linear_context();
    let tree = DerivationTree::new(
        forall(
            "n",
            id(
                pf("trc", pn()),
                padd(
                    pmul(palpha(), pf("tra", pn())),
                    pmul(pbeta(), pf("trb", pn())),
                ),
            ),
        ),
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        vec![named("base_l"), linear_step()],
    );
    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("full-linearity induction certifies");
    let inferred = infer_type(&ctx, &term).expect("the certified term must type-check");
    let goal = pi(
        "n",
        nat(),
        keqi(
            kf("trc", kvar("n")),
            kadd(
                kmul(g("alpha"), kf("tra", kvar("n"))),
                kmul(g("beta"), kf("trb", kvar("n"))),
            ),
        ),
    );
    assert!(
        is_subtype(&ctx, &inferred, &goal),
        "∀n. Eq Int (trc n) (add (mul alpha (tra n)) (mul beta (trb n))): {inferred} ⊄ {goal}"
    );
}

// ══ Determinant multiplicativity for diagonal matrices: det(D·E) = det(D)·det(E) for every size. ════
// The MULTIPLICATIVE ∀n theorem, unlocked by the oracle's constructive monomial sort. det of a
// diagonal matrix is the product of its diagonal, so det(D)·det(E) for D=diag(f), E=diag(g) reduces
// to `Π(fₖ·gₖ) = (Π fₖ)·(Π gₖ)`. The step's ring identity `(df·dg)·(f·g) = (df·f)·(dg·g)` is a
// degree-4 PURE-MULTIPLICATIVE rearrangement — proven by `prove_int_eq` via the atom-factor sort.

fn det_mult_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    for name in ["dfg", "df", "dg", "f", "g"] {
        ctx.add_declaration(name, pi("_", nat(), intt()));
    }
    ctx.add_declaration(
        "base_d",
        keqi(
            kf("dfg", g("Zero")),
            kmul(kf("df", g("Zero")), kf("dg", g("Zero"))),
        ),
    );
    ctx.add_declaration(
        "dfg_succ",
        pi(
            "k",
            nat(),
            keqi(
                kf("dfg", ksucc(kvar("k"))),
                kmul(
                    kf("dfg", kvar("k")),
                    kmul(kf("f", kvar("k")), kf("g", kvar("k"))),
                ),
            ),
        ),
    );
    for (d, e) in [("df", "f"), ("dg", "g")] {
        ctx.add_declaration(
            &format!("{d}_succ"),
            pi(
                "k",
                nat(),
                keqi(
                    kf(d, ksucc(kvar("k"))),
                    kmul(kf(d, kvar("k")), kf(e, kvar("k"))),
                ),
            ),
        );
    }
    ctx
}

fn det_mult_step() -> DerivationTree {
    let dfg_k = pf("dfg", pk());
    let df_k = pf("df", pk());
    let dg_k = pf("dg", pk());
    let f_k = pf("f", pk());
    let g_k = pf("g", pk());
    let dfg_sk = pf("dfg", psucc(pk()));
    let df_sk = pf("df", psucc(pk()));
    let dg_sk = pf("dg", psucc(pk()));

    let t1 = DerivationTree::new(
        id(
            dfg_sk.clone(),
            pmul(dfg_k.clone(), pmul(f_k.clone(), g_k.clone())),
        ),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("dfg_succ")],
    );
    let ih = DerivationTree::leaf(
        id(dfg_k.clone(), pmul(df_k.clone(), dg_k.clone())),
        InferenceRule::PremiseMatch,
    );
    let t2 = DerivationTree::new(
        id(
            dfg_sk.clone(),
            pmul(
                pmul(df_k.clone(), dg_k.clone()),
                pmul(f_k.clone(), g_k.clone()),
            ),
        ),
        InferenceRule::Rewrite {
            from: dfg_k,
            to: pmul(df_k.clone(), dg_k.clone()),
        },
        vec![ih, t1],
    );
    // degree-4 multiplicative rearrange:  (df·dg)·(f·g) = (df·f)·(dg·g)
    let ring = DerivationTree::leaf(
        id(
            pmul(
                pmul(df_k.clone(), dg_k.clone()),
                pmul(f_k.clone(), g_k.clone()),
            ),
            pmul(
                pmul(df_k.clone(), f_k.clone()),
                pmul(dg_k.clone(), g_k.clone()),
            ),
        ),
        InferenceRule::ArithDecision,
    );
    let t4 = DerivationTree::new(
        id(
            dfg_sk.clone(),
            pmul(
                pmul(df_k.clone(), f_k.clone()),
                pmul(dg_k.clone(), g_k.clone()),
            ),
        ),
        InferenceRule::EqualityTransitivity,
        vec![t2, ring],
    );
    let df_flip = DerivationTree::new(
        id(pmul(df_k.clone(), f_k.clone()), df_sk.clone()),
        InferenceRule::EqualitySymmetry,
        vec![DerivationTree::new(
            id(df_sk.clone(), pmul(df_k.clone(), f_k.clone())),
            InferenceRule::UniversalInst("k".to_string()),
            vec![named("df_succ")],
        )],
    );
    let t5 = DerivationTree::new(
        id(
            dfg_sk.clone(),
            pmul(df_sk.clone(), pmul(dg_k.clone(), g_k.clone())),
        ),
        InferenceRule::Rewrite {
            from: pmul(df_k, f_k),
            to: df_sk.clone(),
        },
        vec![df_flip, t4],
    );
    let dg_flip = DerivationTree::new(
        id(pmul(dg_k.clone(), g_k.clone()), dg_sk.clone()),
        InferenceRule::EqualitySymmetry,
        vec![DerivationTree::new(
            id(dg_sk.clone(), pmul(dg_k.clone(), g_k.clone())),
            InferenceRule::UniversalInst("k".to_string()),
            vec![named("dg_succ")],
        )],
    );
    DerivationTree::new(
        id(dfg_sk, pmul(df_sk, dg_sk.clone())),
        InferenceRule::Rewrite {
            from: pmul(dg_k, g_k),
            to: dg_sk,
        },
        vec![dg_flip, t5],
    )
}

/// **∀n. det(D·E) = det(D)·det(E)** for diagonal D, E — determinant multiplicativity, by induction,
/// now provable thanks to the oracle's high-degree monomial reordering.
#[test]
fn diagonal_det_multiplicativity_holds_at_every_size_kernel_certified() {
    let ctx = det_mult_context();
    let tree = DerivationTree::new(
        forall(
            "n",
            id(pf("dfg", pn()), pmul(pf("df", pn()), pf("dg", pn()))),
        ),
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        vec![named("base_d"), det_mult_step()],
    );
    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("det-multiplicativity induction certifies");
    let inferred = infer_type(&ctx, &term).expect("the certified term must type-check");
    let goal = pi(
        "n",
        nat(),
        keqi(
            kf("dfg", kvar("n")),
            kmul(kf("df", kvar("n")), kf("dg", kvar("n"))),
        ),
    );
    assert!(
        is_subtype(&ctx, &inferred, &goal),
        "∀n. Eq Int (dfg n) (mul (df n) (dg n)): {inferred} ⊄ {goal}"
    );
}

// ══ Trace transpose-invariance: trace(Aᵀ) = trace(A) for every size, by induction. ══════════════════
// Aᵀ and A share a diagonal, so the step is a PURE rewrite (no ring algebra at all). Its point is the
// GOAL SHAPE: `trt n = tr n` is a BARE Int-valued sequence identity — no add/mul on either side — which
// the purely-syntactic domain classifier used to call `Eq Entity`, breaking the induction motive. The
// certifier now resolves the equality domain against the context (`trt, tr : Nat → Int`) to `Eq Int`.

fn transpose_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    for f in ["trt", "tr", "d"] {
        ctx.add_declaration(f, pi("_", nat(), intt()));
    }
    // base: trt Zero = tr Zero   (both 0×0 traces are 0)
    ctx.add_declaration("base_t", keqi(kf("trt", g("Zero")), kf("tr", g("Zero"))));
    // trt (Succ k) = trt k + d k   and   tr (Succ k) = tr k + d k   — Aᵀ shares A's diagonal d
    for t in ["trt", "tr"] {
        ctx.add_declaration(
            &format!("{t}_succ"),
            pi(
                "k",
                nat(),
                keqi(
                    kf(t, ksucc(kvar("k"))),
                    kadd(kf(t, kvar("k")), kf("d", kvar("k"))),
                ),
            ),
        );
    }
    ctx
}

fn transpose_step() -> DerivationTree {
    let trt_k = pf("trt", pk());
    let tr_k = pf("tr", pk());
    let d_k = pf("d", pk());
    let trt_sk = pf("trt", psucc(pk()));
    let tr_sk = pf("tr", psucc(pk()));

    // trt_succ k :  trt (Succ k) = trt k + d k
    let t1 = DerivationTree::new(
        id(trt_sk.clone(), padd(trt_k.clone(), d_k.clone())),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("trt_succ")],
    );
    // IH :  trt k = tr k   (a bare sequence identity — the recursive call)
    let ih = DerivationTree::leaf(id(trt_k.clone(), tr_k.clone()), InferenceRule::PremiseMatch);
    // REWRITE the IH :  trt (Succ k) = tr k + d k
    let t2 = DerivationTree::new(
        id(trt_sk.clone(), padd(tr_k.clone(), d_k.clone())),
        InferenceRule::Rewrite {
            from: trt_k,
            to: tr_k.clone(),
        },
        vec![ih, t1],
    );
    // tr_succ k flipped :  (tr k + d k) = tr (Succ k)
    let tr_flip = DerivationTree::new(
        id(padd(tr_k.clone(), d_k.clone()), tr_sk.clone()),
        InferenceRule::EqualitySymmetry,
        vec![DerivationTree::new(
            id(tr_sk.clone(), padd(tr_k.clone(), d_k.clone())),
            InferenceRule::UniversalInst("k".to_string()),
            vec![named("tr_succ")],
        )],
    );
    // REWRITE :  trt (Succ k) = tr (Succ k)   — P(Succ k), a BARE sequence identity
    DerivationTree::new(
        id(trt_sk, tr_sk.clone()),
        InferenceRule::Rewrite {
            from: padd(tr_k, d_k),
            to: tr_sk,
        },
        vec![tr_flip, t2],
    )
}

/// **∀n. trace(Aᵀ) = trace(A)** — transpose-invariance of the trace, by `Nat` induction. The step is a
/// pure rewrite; the theorem exists to certify a ∀n goal whose body is a BARE `Int`-sequence identity
/// (`trt n = tr n`), which the ctx-resolved equality domain now types as `Eq Int`, not `Eq Entity`.
#[test]
fn trace_transpose_invariance_holds_at_every_size_kernel_certified() {
    let ctx = transpose_context();
    let tree = DerivationTree::new(
        forall("n", id(pf("trt", pn()), pf("tr", pn()))),
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        vec![named("base_t"), transpose_step()],
    );
    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("trace transpose-invariance induction certifies");
    let inferred = infer_type(&ctx, &term).expect("the certified term must type-check");
    let goal = pi("n", nat(), keqi(kf("trt", kvar("n")), kf("tr", kvar("n"))));
    assert!(
        is_subtype(&ctx, &inferred, &goal),
        "∀n. Eq Int (trt n) (tr n): {inferred} ⊄ {goal}"
    );
}

// ══ det(Iₙ) = 1 at every size, by induction. ════════════════════════════════════════════════════════
// The determinant of the n×n identity is 1 for all n: cofactor expansion gives det(I_{k+1}) = det(I_k),
// so from base det(I_0) = 1 the value never moves. A pure-rewrite induction whose goal body `deti n = 1`
// has a CONSTANT literal right-hand side (not a sequence application) — the ctx-resolved equality domain
// still types it `Eq Int`.

fn identity_det_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.add_declaration("deti", pi("_", nat(), intt()));
    // base: deti Zero = 1
    ctx.add_declaration("base_id", keqi(kf("deti", g("Zero")), klit(1)));
    // deti (Succ k) = deti k   (adding a row/column of the identity leaves the determinant unchanged)
    ctx.add_declaration(
        "deti_succ",
        pi(
            "k",
            nat(),
            keqi(kf("deti", ksucc(kvar("k"))), kf("deti", kvar("k"))),
        ),
    );
    ctx
}

fn identity_det_step() -> DerivationTree {
    let deti_k = pf("deti", pk());
    let deti_sk = pf("deti", psucc(pk()));
    // deti_succ k :  deti (Succ k) = deti k
    let t1 = DerivationTree::new(
        id(deti_sk.clone(), deti_k.clone()),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("deti_succ")],
    );
    // IH :  deti k = 1
    let ih = DerivationTree::leaf(id(deti_k.clone(), pone()), InferenceRule::PremiseMatch);
    // REWRITE the IH :  deti (Succ k) = 1   — P(Succ k)
    DerivationTree::new(
        id(deti_sk, pone()),
        InferenceRule::Rewrite {
            from: deti_k,
            to: pone(),
        },
        vec![ih, t1],
    )
}

/// **∀n. det(Iₙ) = 1** — the determinant of the identity is 1 at every size, by `Nat` induction with a
/// constant literal right-hand side.
#[test]
fn identity_determinant_is_one_at_every_size_kernel_certified() {
    let ctx = identity_det_context();
    let tree = DerivationTree::new(
        forall("n", id(pf("deti", pn()), pone())),
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        vec![named("base_id"), identity_det_step()],
    );
    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("det(Iₙ)=1 induction certifies");
    let inferred = infer_type(&ctx, &term).expect("the certified term must type-check");
    let goal = pi("n", nat(), keqi(kf("deti", kvar("n")), klit(1)));
    assert!(
        is_subtype(&ctx, &inferred, &goal),
        "∀n. Eq Int (deti n) 1: {inferred} ⊄ {goal}"
    );
}

// ══ trace(Iₙ) = n at every size, by induction. ══════════════════════════════════════════════════════
// The trace of the n×n identity is n: each diagonal entry is 1, so trace accumulates 1 per step. With
// no built-in Nat→Int coercion, `n` is the axiomatized embedding `emb` (emb Zero = 0, emb (Succ k) =
// emb k + 1); the theorem is `tri n = emb n`, i.e. trace(Iₙ) equals the integer embedding of n. The
// step is the same shape as trace-transpose but with the literal `1` diagonal.

fn identity_trace_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    for f in ["tri", "emb"] {
        ctx.add_declaration(f, pi("_", nat(), intt()));
    }
    // base: tri Zero = emb Zero   (both 0)
    ctx.add_declaration("base_it", keqi(kf("tri", g("Zero")), kf("emb", g("Zero"))));
    // tri (Succ k) = tri k + 1   and   emb (Succ k) = emb k + 1
    for t in ["tri", "emb"] {
        ctx.add_declaration(
            &format!("{t}_succ"),
            pi(
                "k",
                nat(),
                keqi(kf(t, ksucc(kvar("k"))), kadd(kf(t, kvar("k")), klit(1))),
            ),
        );
    }
    ctx
}

fn identity_trace_step() -> DerivationTree {
    let tri_k = pf("tri", pk());
    let emb_k = pf("emb", pk());
    let tri_sk = pf("tri", psucc(pk()));
    let emb_sk = pf("emb", psucc(pk()));

    // tri_succ k :  tri (Succ k) = tri k + 1
    let t1 = DerivationTree::new(
        id(tri_sk.clone(), padd(tri_k.clone(), pone())),
        InferenceRule::UniversalInst("k".to_string()),
        vec![named("tri_succ")],
    );
    // IH :  tri k = emb k
    let ih = DerivationTree::leaf(
        id(tri_k.clone(), emb_k.clone()),
        InferenceRule::PremiseMatch,
    );
    // REWRITE the IH :  tri (Succ k) = emb k + 1
    let t2 = DerivationTree::new(
        id(tri_sk.clone(), padd(emb_k.clone(), pone())),
        InferenceRule::Rewrite {
            from: tri_k,
            to: emb_k.clone(),
        },
        vec![ih, t1],
    );
    // emb_succ k flipped :  (emb k + 1) = emb (Succ k)
    let emb_flip = DerivationTree::new(
        id(padd(emb_k.clone(), pone()), emb_sk.clone()),
        InferenceRule::EqualitySymmetry,
        vec![DerivationTree::new(
            id(emb_sk.clone(), padd(emb_k.clone(), pone())),
            InferenceRule::UniversalInst("k".to_string()),
            vec![named("emb_succ")],
        )],
    );
    // REWRITE :  tri (Succ k) = emb (Succ k)   — P(Succ k)
    DerivationTree::new(
        id(tri_sk, emb_sk.clone()),
        InferenceRule::Rewrite {
            from: padd(emb_k, pone()),
            to: emb_sk,
        },
        vec![emb_flip, t2],
    )
}

/// **∀n. trace(Iₙ) = n** — the trace of the identity is n at every size (as the integer embedding of
/// `n`), by `Nat` induction with a literal-`1` diagonal recurrence.
#[test]
fn identity_trace_is_the_size_at_every_size_kernel_certified() {
    let ctx = identity_trace_context();
    let tree = DerivationTree::new(
        forall("n", id(pf("tri", pn()), pf("emb", pn()))),
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(),
        },
        vec![named("base_it"), identity_trace_step()],
    );
    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("trace(Iₙ)=emb(n) induction certifies");
    let inferred = infer_type(&ctx, &term).expect("the certified term must type-check");
    let goal = pi("n", nat(), keqi(kf("tri", kvar("n")), kf("emb", kvar("n"))));
    assert!(
        is_subtype(&ctx, &inferred, &goal),
        "∀n. Eq Int (tri n) (emb n): {inferred} ⊄ {goal}"
    );
}
