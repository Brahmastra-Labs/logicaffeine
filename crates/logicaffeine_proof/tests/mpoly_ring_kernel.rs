//! **The n-variable multilinear GF(2) polynomial ring, recursively, in the kernel — Stage 1: the type family.**
//!
//! `MPoly(n+1) ≅ MPoly(n) × MPoly(n)` (a polynomial in `n+1` variables is `p₀ + X_{n+1}·p₁`, a pair of
//! `n`-variable polynomials). This stage builds that recursive TYPE FAMILY in the kernel — a `Type`-level
//! product `Prod` and `MPoly : Nat → Type` defined by large-elimination recursion on the variable count — and
//! verifies it type-checks and COMPUTES (`MPoly 0 ⇝ Bool`, `MPoly 2 ⇝ Prod (Prod Bool Bool) (Prod Bool
//! Bool)`). This is the feasibility gate for the full lift; the ring operations and the `∀n. p·one = p`
//! induction build on top of it.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, is_subtype, normalize, Context, Term, Universe};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn kvar(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn app2(f: Term, x: Term, y: Term) -> Term {
    app(app(f, x), y)
}
fn lam(param: &str, ty: Term, body: Term) -> Term {
    Term::Lambda { param: param.to_string(), param_type: Box::new(ty), body: Box::new(body) }
}
fn pi(param: &str, ty: Term, body: Term) -> Term {
    Term::Pi { param: param.to_string(), param_type: Box::new(ty), body_type: Box::new(body) }
}
fn mtch(disc: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match { discriminant: Box::new(disc), motive: Box::new(motive), cases }
}
fn fix(name: &str, body: Term) -> Term {
    Term::Fix { name: name.to_string(), body: Box::new(body) }
}
fn type0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn nat() -> Term {
    g("Nat")
}
fn boolt() -> Term {
    g("Bool")
}
fn succ(n: Term) -> Term {
    app(g("Succ"), n)
}
fn prod(a: Term, b: Term) -> Term {
    app2(g("Prod"), a, b)
}
/// `mkp A B a b : Prod A B`.
fn mkp(ta: Term, tb: Term, a: Term, b: Term) -> Term {
    app(app(app(app(g("mkp"), ta), tb), a), b)
}
fn mpoly(n: Term) -> Term {
    app(g("MPoly"), n)
}
fn appn(f: Term, args: Vec<Term>) -> Term {
    args.into_iter().fold(f, app)
}
/// `Eq T x y`.
fn keq(t: Term, x: Term, y: Term) -> Term {
    appn(g("Eq"), vec![t, x, y])
}
/// `refl T x : Eq T x x`.
fn krefl(t: Term, x: Term) -> Term {
    appn(g("refl"), vec![t, x])
}
fn mpaddn(n: Term, x: Term, y: Term) -> Term {
    appn(g("mpadd"), vec![n, x, y])
}
fn mpmuln(n: Term, x: Term, y: Term) -> Term {
    appn(g("mpmul"), vec![n, x, y])
}
fn mzeron(n: Term) -> Term {
    app(g("mzero"), n)
}
fn mponen(n: Term) -> Term {
    app(g("mpone"), n)
}
fn mkpcong(ta: Term, tb: Term, a0: Term, a1: Term, b0: Term, b1: Term, e0: Term, e1: Term) -> Term {
    appn(g("mkp_cong"), vec![ta, tb, a0, a1, b0, b1, e0, e1])
}
/// The recursive-call (induction hypothesis) `rec k p` inside a Fix named "rec".
fn ihk(p: Term) -> Term {
    app(app(kvar("rec"), kvar("k")), p)
}
fn eqrec(t: Term, x: Term, motive: Term, base: Term, y: Term, h: Term) -> Term {
    appn(g("Eq_rec"), vec![t, x, motive, base, y, h])
}
fn eqtrans(t: Term, x: Term, y: Term, z: Term, p: Term, q: Term) -> Term {
    appn(g("Eq_trans"), vec![t, x, y, z, p, q])
}
/// Build a Nat induction term `fix rec. λn. match n return motive { base, λk. step }`.
fn nat_induction(motive: Term, base: Term, step_at_k: Term) -> Term {
    fix("rec", lam("n", nat(), mtch(kvar("n"), motive, vec![base, lam("k", nat(), step_at_k)])))
}
/// Nest λ-binders over a body (right fold), avoiding hand-matched parentheses.
fn lams(binders: &[(&str, Term)], body: Term) -> Term {
    binders.iter().rev().fold(body, |acc, (n, t)| lam(n, t.clone(), acc))
}
/// Nest Π-binders over a body (right fold).
fn pis(binders: &[(&str, Term)], body: Term) -> Term {
    binders.iter().rev().fold(body, |acc, (n, t)| pi(n, t.clone(), acc))
}

fn mpoly_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // GF(2) = (Bool, xor, and2) — the coefficient field (base case of the recursion). Constructor order [true, false].
    let not_b = mtch(kvar("b"), lam("_", boolt(), boolt()), vec![g("false"), g("true")]);
    ctx.add_definition(
        "xor".to_string(),
        pi("a", boolt(), pi("b", boolt(), boolt())),
        lam("a", boolt(), lam("b", boolt(), mtch(kvar("a"), lam("_", boolt(), boolt()), vec![not_b, kvar("b")]))),
    );
    ctx.add_definition(
        "and2".to_string(),
        pi("a", boolt(), pi("b", boolt(), boolt())),
        lam("a", boolt(), lam("b", boolt(), mtch(kvar("a"), lam("_", boolt(), boolt()), vec![kvar("b"), g("false")]))),
    );

    // Prod : Type → Type → Type, with mkp : Π A B. A → B → Prod A B, and projections pfst / psnd.
    ctx.add_inductive("Prod", pi("A", type0(), pi("B", type0(), type0())));
    ctx.set_inductive_params("Prod", 2);
    ctx.add_constructor(
        "mkp",
        "Prod",
        pi("A", type0(), pi("B", type0(), pi("a", kvar("A"), pi("b", kvar("B"), prod(kvar("A"), kvar("B")))))),
    );
    // pfst A B p = match p { mkp a b => a }
    ctx.add_definition(
        "pfst".to_string(),
        pi("A", type0(), pi("B", type0(), pi("p", prod(kvar("A"), kvar("B")), kvar("A")))),
        lam(
            "A",
            type0(),
            lam(
                "B",
                type0(),
                lam(
                    "p",
                    prod(kvar("A"), kvar("B")),
                    mtch(
                        kvar("p"),
                        lam("_", prod(kvar("A"), kvar("B")), kvar("A")),
                        vec![lam("a", kvar("A"), lam("b", kvar("B"), kvar("a")))],
                    ),
                ),
            ),
        ),
    );
    // psnd A B p = match p { mkp a b => b }
    ctx.add_definition(
        "psnd".to_string(),
        pi("A", type0(), pi("B", type0(), pi("p", prod(kvar("A"), kvar("B")), kvar("B")))),
        lam(
            "A",
            type0(),
            lam(
                "B",
                type0(),
                lam(
                    "p",
                    prod(kvar("A"), kvar("B")),
                    mtch(
                        kvar("p"),
                        lam("_", prod(kvar("A"), kvar("B")), kvar("B")),
                        vec![lam("a", kvar("A"), lam("b", kvar("B"), kvar("b")))],
                    ),
                ),
            ),
        ),
    );

    // MPoly : Nat → Type.  MPoly 0 = Bool,  MPoly (Succ k) = Prod (MPoly k) (MPoly k).
    // Large-elimination recursion on the variable count (Nat is a Type, so this is allowed).
    ctx.add_definition(
        "MPoly".to_string(),
        pi("_", nat(), type0()),
        fix(
            "mp",
            lam(
                "n",
                nat(),
                mtch(
                    kvar("n"),
                    lam("_", nat(), type0()),
                    vec![boolt(), lam("k", nat(), prod(app(kvar("mp"), kvar("k")), app(kvar("mp"), kvar("k"))))],
                ),
            ),
        ),
    );

    // The binary-op type Π n. MPoly n → MPoly n → MPoly n, and the dependent motive for its recursion.
    let binop_ty = pi("n", nat(), pi("_", mpoly(kvar("n")), pi("_", mpoly(kvar("n")), mpoly(kvar("n")))));
    let binop_motive =
        lam("n", nat(), pi("_", mpoly(kvar("n")), pi("_", mpoly(kvar("n")), mpoly(kvar("n")))));
    // Projections / recursive calls at level k (inside a Succ case where `k`, and the self `f`, are in scope).
    let mpk = || mpoly(kvar("k"));
    let fstk = |p: Term| app(app2(g("pfst"), mpk(), mpk()), p);
    let sndk = |p: Term| app(app2(g("psnd"), mpk(), mpk()), p);
    let fk = |x: Term, y: Term| app(app(app(kvar("f"), kvar("k")), x), y); // self-recursion
    let addk = |x: Term, y: Term| app(app(app(g("mpadd"), kvar("k")), x), y);
    let sk = || succ(kvar("k"));

    // mzero : Π n. MPoly n.  mzero 0 = false; mzero (S k) = (mzero k, mzero k).
    ctx.add_definition(
        "mzero".to_string(),
        pi("n", nat(), mpoly(kvar("n"))),
        fix(
            "f",
            lam(
                "n",
                nat(),
                mtch(
                    kvar("n"),
                    lam("n", nat(), mpoly(kvar("n"))),
                    vec![
                        g("false"),
                        lam("k", nat(), mkp(mpk(), mpk(), app(kvar("f"), kvar("k")), app(kvar("f"), kvar("k")))),
                    ],
                ),
            ),
        ),
    );

    // mpone : Π n. MPoly n.  mpone 0 = true; mpone (S k) = (mpone k, mzero k)  — the polynomial 1.
    ctx.add_definition(
        "mpone".to_string(),
        pi("n", nat(), mpoly(kvar("n"))),
        fix(
            "f",
            lam(
                "n",
                nat(),
                mtch(
                    kvar("n"),
                    lam("n", nat(), mpoly(kvar("n"))),
                    vec![
                        g("true"),
                        lam("k", nat(), mkp(mpk(), mpk(), app(kvar("f"), kvar("k")), app(g("mzero"), kvar("k")))),
                    ],
                ),
            ),
        ),
    );

    // mpadd : Π n. MPoly n → MPoly n → MPoly n — coefficient-wise, componentwise on the pair.
    ctx.add_definition(
        "mpadd".to_string(),
        binop_ty.clone(),
        fix(
            "f",
            lam(
                "n",
                nat(),
                mtch(
                    kvar("n"),
                    binop_motive.clone(),
                    vec![
                        g("xor"),
                        lam(
                            "k",
                            nat(),
                            lam(
                                "p",
                                mpoly(sk()),
                                lam(
                                    "q",
                                    mpoly(sk()),
                                    mkp(
                                        mpk(),
                                        mpk(),
                                        fk(fstk(kvar("p")), fstk(kvar("q"))),
                                        fk(sndk(kvar("p")), sndk(kvar("q"))),
                                    ),
                                ),
                            ),
                        ),
                    ],
                ),
            ),
        ),
    );

    // mpmul : Π n. MPoly n → MPoly n → MPoly n.  Using X² = X: (p₀,p₁)(q₀,q₁) = (p₀q₀, p₀q₁ + p₁q₀ + p₁q₁).
    ctx.add_definition(
        "mpmul".to_string(),
        binop_ty,
        fix(
            "f",
            lam(
                "n",
                nat(),
                mtch(
                    kvar("n"),
                    binop_motive,
                    vec![
                        g("and2"),
                        lam(
                            "k",
                            nat(),
                            lam(
                                "p",
                                mpoly(sk()),
                                lam(
                                    "q",
                                    mpoly(sk()),
                                    mkp(
                                        mpk(),
                                        mpk(),
                                        fk(fstk(kvar("p")), fstk(kvar("q"))),
                                        addk(
                                            addk(
                                                fk(fstk(kvar("p")), sndk(kvar("q"))),
                                                fk(sndk(kvar("p")), fstk(kvar("q"))),
                                            ),
                                            fk(sndk(kvar("p")), sndk(kvar("q"))),
                                        ),
                                    ),
                                ),
                            ),
                        ),
                    ],
                ),
            ),
        ),
    );

    // Pair congruence:  mkp_cong A B a0 a1 b0 b1 (e0:a0=a1) (e1:b0=b1) : mkp a0 b0 = mkp a1 b1.
    // Congruence in each argument via Eq_rec (transport of refl), chained by Eq_trans — no primitive needed.
    let pab = prod(kvar("A"), kvar("B"));
    let mk = |a: Term, b: Term| mkp(kvar("A"), kvar("B"), a, b);
    let p1 = lam("x", kvar("A"), keq(pab.clone(), mk(kvar("a0"), kvar("b0")), mk(kvar("x"), kvar("b0"))));
    let cong1 = appn(
        g("Eq_rec"),
        vec![kvar("A"), kvar("a0"), p1, krefl(pab.clone(), mk(kvar("a0"), kvar("b0"))), kvar("a1"), kvar("e0")],
    );
    let p2 = lam("y", kvar("B"), keq(pab.clone(), mk(kvar("a1"), kvar("b0")), mk(kvar("a1"), kvar("y"))));
    let cong2 = appn(
        g("Eq_rec"),
        vec![kvar("B"), kvar("b0"), p2, krefl(pab.clone(), mk(kvar("a1"), kvar("b0"))), kvar("b1"), kvar("e1")],
    );
    let cong_body = appn(
        g("Eq_trans"),
        vec![pab.clone(), mk(kvar("a0"), kvar("b0")), mk(kvar("a1"), kvar("b0")), mk(kvar("a1"), kvar("b1")), cong1, cong2],
    );
    let mk_binders = |inner: Term| {
        lam("A", type0(), lam("B", type0(), lam("a0", kvar("A"), lam("a1", kvar("A"),
            lam("b0", kvar("B"), lam("b1", kvar("B"),
                lam("e0", keq(kvar("A"), kvar("a0"), kvar("a1")),
                    lam("e1", keq(kvar("B"), kvar("b0"), kvar("b1")), inner))))))))
    };
    let pi_binders = |inner: Term| {
        pi("A", type0(), pi("B", type0(), pi("a0", kvar("A"), pi("a1", kvar("A"),
            pi("b0", kvar("B"), pi("b1", kvar("B"),
                pi("e0", keq(kvar("A"), kvar("a0"), kvar("a1")),
                    pi("e1", keq(kvar("B"), kvar("b0"), kvar("b1")), inner))))))))
    };
    let cong_concl = keq(pab, mk(kvar("a0"), kvar("b0")), mk(kvar("a1"), kvar("b1")));
    ctx.add_definition("mkp_cong".to_string(), pi_binders(cong_concl), mk_binders(cong_body));

    // A polynomial-in-p statement `Π n. Π p:MPoly n. Eq (MPoly n) (lhs n p) p`, its motive, and the Succ-step
    // scaffold: destructure p = mkp p0 p1, then apply mkp_cong to the two induction-hypothesis component
    // equalities. `lhs` maps (n, p) to the left side; the right side is always `p`.
    // add_zero_l : Π n p. mpadd n (mzero n) p = p.  Base: xor false p ⇝ p (refl). Step: mkp_cong ∘ IH.
    let azl_lhs = |n: Term, p: Term| mpaddn(n.clone(), mzeron(n), p);
    let azl_ty = pi("n", nat(), pi("p", mpoly(kvar("n")), keq(mpoly(kvar("n")), azl_lhs(kvar("n"), kvar("p")), kvar("p"))));
    let azl_motive = lam("n", nat(), pi("p", mpoly(kvar("n")), keq(mpoly(kvar("n")), azl_lhs(kvar("n"), kvar("p")), kvar("p"))));
    let azl_base = lam("p", mpoly(g("Zero")), krefl(mpoly(g("Zero")), kvar("p")));
    // The Match motive parameter must match the NORMALIZED discriminant type (normalize unfolds `MPoly k` to
    // its raw fix body), so compute that normal form and use it as the binder type.
    let disc_nf = normalize(&ctx, &mpoly(succ(kvar("k"))));
    let azl_step = lam(
        "p",
        mpoly(succ(kvar("k"))),
        mtch(
            kvar("p"),
            lam("p", disc_nf.clone(), keq(mpoly(succ(kvar("k"))), azl_lhs(succ(kvar("k")), kvar("p")), kvar("p"))),
            vec![lam(
                "p0",
                mpoly(kvar("k")),
                lam(
                    "p1",
                    mpoly(kvar("k")),
                    mkpcong(
                        mpoly(kvar("k")),
                        mpoly(kvar("k")),
                        azl_lhs(kvar("k"), kvar("p0")),
                        kvar("p0"),
                        azl_lhs(kvar("k"), kvar("p1")),
                        kvar("p1"),
                        ihk(kvar("p0")),
                        ihk(kvar("p1")),
                    ),
                ),
            )],
        ),
    );
    ctx.add_definition("add_zero_l".to_string(), azl_ty, nat_induction(azl_motive, azl_base, azl_step));

    // fun_cong2 : Π T (f:T→T→T) a a' b b'. a=a' → b=b' → f a b = f a' b'  (congruence of a binary op).
    let fap = |x: Term, y: Term| app(app(kvar("f"), x), y);
    let fc1 = eqrec(
        kvar("T"),
        kvar("a"),
        lam("x", kvar("T"), keq(kvar("T"), fap(kvar("a"), kvar("b")), fap(kvar("x"), kvar("b")))),
        krefl(kvar("T"), fap(kvar("a"), kvar("b"))),
        kvar("a2"),
        kvar("e0"),
    );
    let fc2 = eqrec(
        kvar("T"),
        kvar("b"),
        lam("y", kvar("T"), keq(kvar("T"), fap(kvar("a2"), kvar("b")), fap(kvar("a2"), kvar("y")))),
        krefl(kvar("T"), fap(kvar("a2"), kvar("b"))),
        kvar("b2"),
        kvar("e1"),
    );
    let fc_body_core = eqtrans(kvar("T"), fap(kvar("a"), kvar("b")), fap(kvar("a2"), kvar("b")), fap(kvar("a2"), kvar("b2")), fc1, fc2);
    let tbin = pi("_", kvar("T"), pi("_", kvar("T"), kvar("T")));
    let fbinders = vec![
        ("T", type0()),
        ("f", tbin),
        ("a", kvar("T")),
        ("a2", kvar("T")),
        ("b", kvar("T")),
        ("b2", kvar("T")),
        ("e0", keq(kvar("T"), kvar("a"), kvar("a2"))),
        ("e1", keq(kvar("T"), kvar("b"), kvar("b2"))),
    ];
    let fc_concl = keq(kvar("T"), fap(kvar("a"), kvar("b")), fap(kvar("a2"), kvar("b2")));
    ctx.add_definition("fun_cong2".to_string(), pis(&fbinders, fc_concl), lams(&fbinders, fc_body_core));

    // add_zero_r : Π n p. mpadd n p (mzero n) = p.  Base: case analysis on p:Bool (xor p false); step: mkp_cong ∘ IH.
    let azr_lhs = |n: Term, p: Term| mpaddn(n.clone(), p, mzeron(n));
    let azr_ty = pi("n", nat(), pi("p", mpoly(kvar("n")), keq(mpoly(kvar("n")), azr_lhs(kvar("n"), kvar("p")), kvar("p"))));
    let azr_motive = lam("n", nat(), pi("p", mpoly(kvar("n")), keq(mpoly(kvar("n")), azr_lhs(kvar("n"), kvar("p")), kvar("p"))));
    let azr_base = lam(
        "p",
        mpoly(g("Zero")),
        mtch(
            kvar("p"),
            lam("p", boolt(), keq(mpoly(g("Zero")), azr_lhs(g("Zero"), kvar("p")), kvar("p"))),
            vec![krefl(mpoly(g("Zero")), g("true")), krefl(mpoly(g("Zero")), g("false"))],
        ),
    );
    let azr_step = lam(
        "p",
        mpoly(succ(kvar("k"))),
        mtch(
            kvar("p"),
            lam("p", disc_nf.clone(), keq(mpoly(succ(kvar("k"))), azr_lhs(succ(kvar("k")), kvar("p")), kvar("p"))),
            vec![lam("p0", mpoly(kvar("k")), lam("p1", mpoly(kvar("k")),
                mkpcong(mpoly(kvar("k")), mpoly(kvar("k")), azr_lhs(kvar("k"), kvar("p0")), kvar("p0"),
                    azr_lhs(kvar("k"), kvar("p1")), kvar("p1"), ihk(kvar("p0")), ihk(kvar("p1")))))],
        ),
    );
    ctx.add_definition("add_zero_r".to_string(), azr_ty, nat_induction(azr_motive, azr_base, azr_step));

    // Shared step machinery at level k: the zero, the partial op `mpadd k`, its congruence, and `k`-addition.
    let mpk = || mpoly(kvar("k"));
    let z = mzeron(kvar("k"));
    let addk = |x: Term, y: Term| mpaddn(kvar("k"), x, y);
    let addk_f = app(g("mpadd"), kvar("k"));
    let funcong = |a: Term, a2: Term, b: Term, b2: Term, e0: Term, e1: Term| {
        appn(g("fun_cong2"), vec![mpk(), addk_f.clone(), a, a2, b, b2, e0, e1])
    };
    // mpadd k z z = z, from add_zero_l k z.
    let azz = app(app(g("add_zero_l"), kvar("k")), z.clone());

    // mul_zero : Π n p. mpmul n p (mzero n) = mzero n.  Base: and2 p false ⇝ false. Step: components → 0.
    let mz_lhs = |n: Term, p: Term| mpmuln(n.clone(), p, mzeron(n));
    let mz_stmt = |n: Term, p: Term| keq(mpoly(n.clone()), mz_lhs(n.clone(), p), mzeron(n));
    let mz_ty = pis(&[("n", nat()), ("p", mpoly(kvar("n")))], mz_stmt(kvar("n"), kvar("p")));
    let mz_motive = lam("n", nat(), pis(&[("p", mpoly(kvar("n")))], mz_stmt(kvar("n"), kvar("p"))));
    let mz_base = lam(
        "p",
        mpoly(g("Zero")),
        mtch(kvar("p"), lam("p", boolt(), mz_stmt(g("Zero"), kvar("p"))),
            vec![krefl(mpoly(g("Zero")), g("false")), krefl(mpoly(g("Zero")), g("false"))]),
    );
    // Step, at p = mkp p0 p1:  C0 = p0·0 = 0 (IH);  C1 = (p0·0 + p1·0) + p1·0 = 0.
    let a_ = mpmuln(kvar("k"), kvar("p0"), z.clone()); // p0·0
    let b_ = mpmuln(kvar("k"), kvar("p1"), z.clone()); // p1·0  (= the C term too)
    let mz_step_i = funcong(a_.clone(), z.clone(), b_.clone(), z.clone(), ihk(kvar("p0")), ihk(kvar("p1"))); // A+B = z+z
    let mz_inner = addk(a_.clone(), b_.clone());
    let mz_inner_eq = eqtrans(mpk(), mz_inner.clone(), addk(z.clone(), z.clone()), z.clone(), mz_step_i, azz.clone());
    let mz_c1 = addk(mz_inner.clone(), b_.clone());
    let mz_step_o = funcong(mz_inner.clone(), z.clone(), b_.clone(), z.clone(), mz_inner_eq, ihk(kvar("p1")));
    let mz_c1_eq = eqtrans(mpk(), mz_c1.clone(), addk(z.clone(), z.clone()), z.clone(), mz_step_o, azz.clone());
    let mz_step_body = mkpcong(mpk(), mpk(), a_.clone(), z.clone(), mz_c1, z.clone(), ihk(kvar("p0")), mz_c1_eq);
    let mz_step = lam(
        "p",
        mpoly(succ(kvar("k"))),
        mtch(kvar("p"), lam("p", disc_nf.clone(), mz_stmt(succ(kvar("k")), kvar("p"))),
            vec![lam("p0", mpk(), lam("p1", mpk(), mz_step_body))]),
    );
    ctx.add_definition("mul_zero".to_string(), mz_ty, nat_induction(mz_motive, mz_base, mz_step));

    // mul_one : Π n p. mpmul n p (mpone n) = p.  Base: and2 p true ⇝ p. Step: C0 = p0·1 = p0 (IH);
    // C1 = (p0·0 + p1·1) + p1·0  →  (0 + p1) + 0 = p1  (via mul_zero + IH + add_zero).
    let mo_lhs = |n: Term, p: Term| mpmuln(n.clone(), p, mponen(n));
    let mo_stmt = |n: Term, p: Term| keq(mpoly(n.clone()), mo_lhs(n.clone(), p.clone()), p);
    let mo_ty = pis(&[("n", nat()), ("p", mpoly(kvar("n")))], mo_stmt(kvar("n"), kvar("p")));
    let mo_motive = lam("n", nat(), pis(&[("p", mpoly(kvar("n")))], mo_stmt(kvar("n"), kvar("p"))));
    let mo_base = lam(
        "p",
        mpoly(g("Zero")),
        mtch(kvar("p"), lam("p", boolt(), mo_stmt(g("Zero"), kvar("p"))),
            vec![krefl(mpoly(g("Zero")), g("true")), krefl(mpoly(g("Zero")), g("false"))]),
    );
    let o = mponen(kvar("k"));
    let d_ = mpmuln(kvar("k"), kvar("p0"), z.clone()); // p0·0
    let e_ = mpmuln(kvar("k"), kvar("p1"), o.clone()); // p1·1
    let f_ = mpmuln(kvar("k"), kvar("p1"), z.clone()); // p1·0
    let c0 = mpmuln(kvar("k"), kvar("p0"), o.clone()); // p0·1
    let mz_at = |x: Term| app(app(g("mul_zero"), kvar("k")), x); // mpmul k x 0 = 0
    let azl_p1 = app(app(g("add_zero_l"), kvar("k")), kvar("p1")); // 0 + p1 = p1
    let azr_p1 = app(app(g("add_zero_r"), kvar("k")), kvar("p1")); // p1 + 0 = p1
    let mo_inner = addk(d_.clone(), e_.clone()); // p0·0 + p1·1
    let mo_step_i = funcong(d_.clone(), z.clone(), e_.clone(), kvar("p1"), mz_at(kvar("p0")), ihk(kvar("p1"))); // = 0 + p1
    let mo_inner_eq = eqtrans(mpk(), mo_inner.clone(), addk(z.clone(), kvar("p1")), kvar("p1"), mo_step_i, azl_p1); // = p1
    let mo_c1 = addk(mo_inner.clone(), f_.clone());
    let mo_step_o = funcong(mo_inner.clone(), kvar("p1"), f_.clone(), z.clone(), mo_inner_eq, mz_at(kvar("p1"))); // = p1 + 0
    let mo_c1_eq = eqtrans(mpk(), mo_c1.clone(), addk(kvar("p1"), z.clone()), kvar("p1"), mo_step_o, azr_p1); // = p1
    let mo_step_body = mkpcong(mpk(), mpk(), c0, kvar("p0"), mo_c1, kvar("p1"), ihk(kvar("p0")), mo_c1_eq);
    let mo_step = lam(
        "p",
        mpoly(succ(kvar("k"))),
        mtch(kvar("p"), lam("p", disc_nf.clone(), mo_stmt(succ(kvar("k")), kvar("p"))),
            vec![lam("p0", mpk(), lam("p1", mpk(), mo_step_body))]),
    );
    ctx.add_definition("mul_one".to_string(), mo_ty, nat_induction(mo_motive, mo_base, mo_step));
    ctx
}

/// The kernel VALIDATES a definition's body against its declared type (`add_definition` only stores; this
/// re-checks). A `true` verdict means the body is a genuine proof/term of the declared type.
fn validates(ctx: &Context, name: &str) -> bool {
    let body = ctx.get_definition_body(name).expect("definition body");
    let ty = ctx.get_definition_type(name).expect("definition type");
    match infer_type(ctx, body) {
        Ok(inferred) => is_subtype(ctx, &inferred, ty) && is_subtype(ctx, ty, &inferred),
        Err(_) => false,
    }
}

#[test]
fn every_ring_definition_body_typechecks_against_its_declaration() {
    let ctx = mpoly_context();
    for name in ["xor", "and2", "pfst", "psnd", "MPoly", "mzero", "mpone", "mpadd", "mpmul", "mkp_cong", "add_zero_l", "fun_cong2", "add_zero_r", "mul_zero", "mul_one"] {
        assert!(validates(&ctx, name), "the body of `{name}` must type-check against its declared type");
    }
}

#[test]
fn multiplicative_identity_for_all_n_and_all_polynomials_is_a_kernel_theorem() {
    // THE LAST GAP, CLOSED. `mul_one : Π n:Nat. Π p:MPoly n. Eq (MPoly n) (mpmul n p (mpone n)) p` — the
    // multiplicative identity of the full recursive n-variable multilinear GF(2) polynomial ring, proven by
    // induction on the variable count (base = the coefficient field; step = mkp_cong ∘ IH ∘ mul_zero ∘
    // add_zero). No axioms: `ring_step` is no longer assumed — it is a theorem of the constructed ring.
    let ctx = mpoly_context();
    assert!(validates(&ctx, "mul_one"), "mul_one's body proves its declared type");
    let declared = ctx.get_definition_type("mul_one").expect("mul_one type").clone();
    let expected = pi(
        "n",
        nat(),
        pi("p", mpoly(kvar("n")), keq(mpoly(kvar("n")), mpmuln(kvar("n"), kvar("p"), mponen(kvar("n"))), kvar("p"))),
    );
    assert!(
        is_subtype(&ctx, &declared, &expected) && is_subtype(&ctx, &expected, &declared),
        "the theorem is exactly ∀n. ∀p:MPoly n. p · one = p"
    );
}

#[test]
fn prod_type_and_projections_compute() {
    let ctx = mpoly_context();
    assert!(matches!(infer_type(&ctx, &g("Prod")), Ok(_)), "Prod : Type → Type → Type");
    // A ground pair and its projections.
    let p = mkp(boolt(), boolt(), g("true"), g("false"));
    assert!(matches!(infer_type(&ctx, &p), Ok(_)), "mkp Bool Bool true false : Prod Bool Bool");
    let fst = app(app2(g("pfst"), boolt(), boolt()), p.clone());
    let snd = app(app2(g("psnd"), boolt(), boolt()), p);
    assert_eq!(normalize(&ctx, &fst), g("true"), "pfst (mkp _ _ true false) = true");
    assert_eq!(normalize(&ctx, &snd), g("false"), "psnd (mkp _ _ true false) = false");
}

#[test]
fn mpoly_type_family_computes() {
    let ctx = mpoly_context();
    assert!(matches!(infer_type(&ctx, &g("MPoly")), Ok(_)), "MPoly : Nat → Type");
    // MPoly 0 = Bool.
    assert!(is_subtype(&ctx, &mpoly(g("Zero")), &boolt()) && is_subtype(&ctx, &boolt(), &mpoly(g("Zero"))), "MPoly 0 = Bool");
    // MPoly 1 = Prod Bool Bool.
    let one = succ(g("Zero"));
    let expect1 = prod(boolt(), boolt());
    assert!(is_subtype(&ctx, &mpoly(one.clone()), &expect1) && is_subtype(&ctx, &expect1, &mpoly(one)), "MPoly 1 = Prod Bool Bool");
    // MPoly 2 = Prod (Prod Bool Bool) (Prod Bool Bool).
    let two = succ(succ(g("Zero")));
    let expect2 = prod(prod(boolt(), boolt()), prod(boolt(), boolt()));
    assert!(is_subtype(&ctx, &mpoly(two.clone()), &expect2) && is_subtype(&ctx, &expect2, &mpoly(two)), "MPoly 2 = (Bool²)²");
}

#[test]
fn ring_operations_typecheck_and_compute() {
    let ctx = mpoly_context();
    // Each operation is well-typed at its dependent type.
    for op in ["mzero", "mpone", "mpadd", "mpmul"] {
        assert!(matches!(infer_type(&ctx, &g(op)), Ok(_)), "{op} type-checks at its dependent type");
    }
    let one = succ(g("Zero"));
    let mp1 = || app(g("mpone"), one.clone()); // mpone 1 : MPoly 1
    // mpone 1 = (true, false) = mk true false, a real 1-variable polynomial.
    assert_eq!(
        normalize(&ctx, &mp1()),
        normalize(&ctx, &mkp(boolt(), boolt(), g("true"), g("false"))),
        "mpone 1 = (1, 0) = the constant polynomial 1"
    );
    // A ground polynomial p = 1 + X = (true, true), and p · one = p by computation (Stage-2 evidence the
    // multiplicative identity holds; Stage 3 proves it ∀p ∀n by induction).
    let p = mkp(boolt(), boolt(), g("true"), g("true"));
    let prod_p_one = app(app(app(g("mpmul"), one.clone()), p.clone()), mp1());
    assert_eq!(normalize(&ctx, &prod_p_one), normalize(&ctx, &p), "(1+X) · 1 = (1+X) at n=1, by computation");
    // And at n = 2 on a ground element.
    let two = succ(one.clone());
    let mp2one = app(g("mpone"), two.clone());
    let q = mkp(
        prod(boolt(), boolt()),
        prod(boolt(), boolt()),
        mkp(boolt(), boolt(), g("true"), g("false")),
        mkp(boolt(), boolt(), g("false"), g("true")),
    );
    let prod_q_one = app(app(app(g("mpmul"), two), q.clone()), mp2one);
    assert_eq!(normalize(&ctx, &prod_q_one), normalize(&ctx, &q), "q · 1 = q at n=2, by computation");
}

#[test]
fn mult_identity_base_case_zero_variables_is_a_kernel_theorem() {
    let ctx = mpoly_context();
    // The BASE CASE of the ∀n induction: ∀p:MPoly 0. mpmul 0 p (mpone 0) = p. Since MPoly 0 = Bool,
    // mpmul 0 = and2, mpone 0 = true, this is exactly the GF(2) multiplicative identity — proven over the
    // recursive ring's fibre at n=0, by case analysis (each leaf reduces `and2 _ true`).
    let mp0 = mpoly(g("Zero"));
    let mmul0 = |a: Term, b: Term| app(app(app(g("mpmul"), g("Zero")), a), b);
    let mone0 = app(g("mpone"), g("Zero"));
    let body = |p: Term| app(app2(g("Eq"), mp0.clone(), mmul0(p.clone(), mone0.clone())), p);
    let law = pi("p", mp0.clone(), body(kvar("p")));
    let refl_at = |x: Term| app2(g("refl"), mp0.clone(), x);
    // The match discriminant p : MPoly 0 reduces to Bool, so the match motive is typed over Bool (the fibre).
    let proof = lam(
        "p",
        mp0.clone(),
        mtch(kvar("p"), lam("p", boolt(), body(kvar("p"))), vec![refl_at(g("true")), refl_at(g("false"))]),
    );
    match infer_type(&ctx, &proof) {
        Ok(t) => assert!(
            is_subtype(&ctx, &t, &law) && is_subtype(&ctx, &law, &t),
            "base case ∀p:MPoly 0. p · one = p is kernel-proven"
        ),
        Err(e) => panic!("base case did not certify: {e:?}"),
    }
}
