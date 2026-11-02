//! Constructor proofs for integer ring, subtraction, and basic order laws.
//!
//! The proof terms reuse the Nat induction, equality transport, and signed
//! constructor proofs already exercised in G_recursor_extra. They target the
//! canonical Int carrier and checked public arithmetic aliases. Every context
//! checks the proof bodies; only their immutable syntax is shared.
use crate::{Context, KernelResult, Term, Universe};

pub(crate) fn register(ctx: &mut Context) -> KernelResult<()> {
    crate::int_inductive::register_public_order_alias(ctx)?;
    // Reuse the existing constructor-disjointness proof generator. Its Prop
    // instances discharge impossible Boolean order premises without an axiom.
    if ctx.get_definition_body("Bool_noConfusionType").is_none() {
        let (ty, body) = crate::recursor::derive_no_confusion_type(ctx, "Bool")?;
        ctx.declare_checked_auxiliary("Bool_noConfusionType", ty, body)?;
    }
    if ctx.get_definition_body("Bool_noConfusion").is_none() {
        let (ty, body) = crate::recursor::derive_no_confusion(ctx, "Bool")?;
        ctx.declare_checked_auxiliary("Bool_noConfusion", ty, body)?;
    }
    // The recursor synthesizer already produces these elimination principles;
    // publish its actual checked definitions before the proofs that use them.
    for family in ["Nat", "Int"] {
        let (ty, body) = crate::recursor::derive_recursor(ctx, family)?;
        ctx.declare_checked_auxiliary(&format!("{family}_rec"), ty, body)?;
    }
    for (name, generated_type, proof) in proofs() {
        // Preserve the exact existing public header (including literal zero
        // and one); constructor forms in the proof convert to those literals.
        let declared_type = match *name {
            "add_zero" | "mul_zero" | "add_comm" | "mul_one" | "mul_comm" | "mul_assoc"
            | "add_assoc" | "mul_distrib_add" | "sub_elim" | "le_refl" | "le_trans"
            | "le_total" | "lt_add1_le" | "le_add_mono" | "le_sub" | "le_mul_nonneg" => ctx
                .get_global(name)
                .cloned()
                .unwrap_or_else(|| generated_type.clone()),
            _ => generated_type.clone(),
        };
        // Reuse the checked declaration gateway with pending admission disabled:
        // a missing dependency must stop initialization, never create a stub.
        ctx.declare_checked_auxiliary(name, declared_type, proof.clone())?;
    }
    Ok(())
}

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}

fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}

fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}

fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |a, x| app(a, x.clone()))
}

fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}

fn pi(p: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}

fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}

fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}

fn nat() -> Term {
    g("Nat")
}

fn succ(t: Term) -> Term {
    app(g("Succ"), t)
}

fn gint() -> Term {
    g("Int")
}

fn gof(n: Term) -> Term {
    app(g("Int_ofNat"), n)
}

fn gneg_succ(n: Term) -> Term {
    app(g("Int_negSucc"), n)
}

fn eq_gint(a: Term, b: Term) -> Term {
    apps(g("Eq"), &[gint(), a, b])
}

fn refl_gint(a: Term) -> Term {
    apps(g("refl"), &[gint(), a])
}

fn eq_at(a: Term, x: Term, y: Term) -> Term {
    apps(g("Eq"), &[a, x, y])
}

fn refl_at(a: Term, x: Term) -> Term {
    apps(g("refl"), &[a, x])
}

fn gcongr(a: Term, b: Term, f: Term, x: Term, y: Term, h: Term) -> Term {
    apps(g("IntLaw_congrArg"), &[a, b, f, x, y, h])
}

fn gsym(a: Term, x: Term, y: Term, h: Term) -> Term {
    apps(g("Eq_sym"), &[a, x, y, h])
}

fn gtrans(a: Term, x: Term, y: Term, z: Term, hxy: Term, hyz: Term) -> Term {
    apps(g("Eq_trans"), &[a, x, y, z, hxy, hyz])
}

fn nat_ind(motive: Term, base: Term, step: Term, n: Term) -> Term {
    apps(g("Nat_rec"), &[motive, base, step, n])
}

fn gint_ind(motive: Term, of_nat: Term, neg_succ: Term, a: Term) -> Term {
    apps(g("Int_rec"), &[motive, of_nat, neg_succ, a])
}

fn gone() -> Term {
    gof(succ(g("Zero")))
}
type Proof = (&'static str, Term, Term);
fn proofs() -> &'static [Proof] {
    static PROOFS: std::sync::OnceLock<Vec<Proof>> = std::sync::OnceLock::new();
    PROOFS.get_or_init(|| {
        let mut proofs = Vec::new();
        // ---- congruence, DERIVED from J ---------------------------------------------------
        // `congrArg : Π(A B : Type 0)(f : A → B)(x y : A). x = y → f x = f y`, the one lemma every
        // induction step below needs. Transport `Eq B (f x) (f _)` along `h`, base `refl B (f x)`.
        let congr_ty = pi(
            "A",
            sort(0),
            pi(
                "B",
                sort(0),
                pi(
                    "f",
                    arrow(var("A"), var("B")),
                    pi(
                        "x",
                        var("A"),
                        pi(
                            "y",
                            var("A"),
                            arrow(
                                eq_at(var("A"), var("x"), var("y")),
                                eq_at(var("B"), app(var("f"), var("x")), app(var("f"), var("y"))),
                            ),
                        ),
                    ),
                ),
            ),
        );
        let congr_tm = lam(
            "A",
            sort(0),
            lam(
                "B",
                sort(0),
                lam(
                    "f",
                    arrow(var("A"), var("B")),
                    lam(
                        "x",
                        var("A"),
                        lam(
                            "y",
                            var("A"),
                            lam(
                                "h",
                                eq_at(var("A"), var("x"), var("y")),
                                apps(
                                    g("Eq_rec_dep"),
                                    &[
                                        var("A"),
                                        var("x"),
                                        lam(
                                            "y2",
                                            var("A"),
                                            lam(
                                                "_",
                                                eq_at(var("A"), var("x"), var("y2")),
                                                eq_at(
                                                    var("B"),
                                                    app(var("f"), var("x")),
                                                    app(var("f"), var("y2")),
                                                ),
                                            ),
                                        ),
                                        refl_at(var("B"), app(var("f"), var("x"))),
                                        var("y"),
                                        var("h"),
                                    ],
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        );
        proofs.push(("IntLaw_congrArg", congr_ty, congr_tm));

        let eqn = |a: Term, b: Term| eq_at(nat(), a, b);
        let add = |a: Term, b: Term| apps(g("Nat_add"), &[a, b]);
        let mul = |a: Term, b: Term| apps(g("Nat_mul"), &[a, b]);
        let congr_succ = |x: Term, y: Term, h: Term| gcongr(nat(), nat(), g("Succ"), x, y, h);

        // ---- `Nat_add_zero : m + 0 = m` -----------------------------------------------------
        // Both kernels now unfold the guarded structural column, so the
        // defining zero equations are reflexivity proofs without induction.
        proofs.push((
            "Nat_add_zero",
            pi("m", nat(), eqn(add(var("m"), g("Zero")), var("m"))),
            lam("m", nat(), refl_at(nat(), var("m"))),
        ));

        // ---- `Nat_zero_add : 0 + n = n` -----------------------------------------------------
        proofs.push((
            "Nat_zero_add",
            pi("n", nat(), eqn(add(g("Zero"), var("n")), var("n"))),
            lam(
                "n",
                nat(),
                nat_ind(
                    lam("nn", nat(), eqn(add(g("Zero"), var("nn")), var("nn"))),
                    refl_at(nat(), g("Zero")),
                    lam(
                        "j",
                        nat(),
                        lam(
                            "ih",
                            eqn(add(g("Zero"), var("j")), var("j")),
                            congr_succ(add(g("Zero"), var("j")), var("j"), var("ih")),
                        ),
                    ),
                    var("n"),
                ),
            ),
        ));

        // ---- `Nat_add_succ_right : m + (n+1) = (m + n) + 1` ---------------------------------
        // The successor equation follows directly from the defining recursion.
        proofs.push((
            "Nat_add_succ_right",
            pi(
                "m",
                nat(),
                pi(
                    "n",
                    nat(),
                    eqn(add(var("m"), succ(var("n"))), succ(add(var("m"), var("n")))),
                ),
            ),
            lam(
                "m",
                nat(),
                lam("n", nat(), refl_at(nat(), succ(add(var("m"), var("n"))))),
            ),
        ));

        // ---- `Nat_add_succ_left : (m+1) + n = (m + n) + 1` ----------------------------------
        proofs.push((
            "Nat_add_succ_left",
            pi(
                "m",
                nat(),
                pi(
                    "n",
                    nat(),
                    eqn(add(succ(var("m")), var("n")), succ(add(var("m"), var("n")))),
                ),
            ),
            lam(
                "m",
                nat(),
                lam(
                    "n",
                    nat(),
                    nat_ind(
                        lam(
                            "nn",
                            nat(),
                            eqn(
                                add(succ(var("m")), var("nn")),
                                succ(add(var("m"), var("nn"))),
                            ),
                        ),
                        // Transport the defining zero equation through Succ.
                        gsym(
                            nat(),
                            succ(add(var("m"), g("Zero"))),
                            succ(var("m")),
                            congr_succ(
                                add(var("m"), g("Zero")),
                                var("m"),
                                app(g("Nat_add_zero"), var("m")),
                            ),
                        ),
                        lam(
                            "j",
                            nat(),
                            lam(
                                "ih",
                                eqn(add(succ(var("m")), var("j")), succ(add(var("m"), var("j")))),
                                gtrans(
                                    nat(),
                                    succ(add(succ(var("m")), var("j"))),
                                    succ(succ(add(var("m"), var("j")))),
                                    succ(add(var("m"), succ(var("j")))),
                                    congr_succ(
                                        add(succ(var("m")), var("j")),
                                        succ(add(var("m"), var("j"))),
                                        var("ih"),
                                    ),
                                    congr_succ(
                                        succ(add(var("m"), var("j"))),
                                        add(var("m"), succ(var("j"))),
                                        gsym(
                                            nat(),
                                            add(var("m"), succ(var("j"))),
                                            succ(add(var("m"), var("j"))),
                                            apps(g("Nat_add_succ_right"), &[var("m"), var("j")]),
                                        ),
                                    ),
                                ),
                            ),
                        ),
                        var("n"),
                    ),
                ),
            ),
        ));

        // ---- `Nat_add_comm : m + n = n + m` -------------------------------------------------
        proofs.push((
            "Nat_add_comm",
            pi(
                "m",
                nat(),
                pi(
                    "n",
                    nat(),
                    eqn(add(var("m"), var("n")), add(var("n"), var("m"))),
                ),
            ),
            lam(
                "m",
                nat(),
                lam(
                    "n",
                    nat(),
                    nat_ind(
                        lam(
                            "mm",
                            nat(),
                            eqn(add(var("mm"), var("n")), add(var("n"), var("mm"))),
                        ),
                        gtrans(
                            nat(),
                            add(g("Zero"), var("n")),
                            var("n"),
                            add(var("n"), g("Zero")),
                            app(g("Nat_zero_add"), var("n")),
                            gsym(
                                nat(),
                                add(var("n"), g("Zero")),
                                var("n"),
                                app(g("Nat_add_zero"), var("n")),
                            ),
                        ),
                        lam(
                            "k",
                            nat(),
                            lam(
                                "ih",
                                eqn(add(var("k"), var("n")), add(var("n"), var("k"))),
                                gtrans(
                                    nat(),
                                    add(succ(var("k")), var("n")),
                                    succ(add(var("n"), var("k"))),
                                    add(var("n"), succ(var("k"))),
                                    gtrans(
                                        nat(),
                                        add(succ(var("k")), var("n")),
                                        succ(add(var("k"), var("n"))),
                                        succ(add(var("n"), var("k"))),
                                        apps(g("Nat_add_succ_left"), &[var("k"), var("n")]),
                                        congr_succ(
                                            add(var("k"), var("n")),
                                            add(var("n"), var("k")),
                                            var("ih"),
                                        ),
                                    ),
                                    gsym(
                                        nat(),
                                        add(var("n"), succ(var("k"))),
                                        succ(add(var("n"), var("k"))),
                                        apps(g("Nat_add_succ_right"), &[var("n"), var("k")]),
                                    ),
                                ),
                            ),
                        ),
                        var("m"),
                    ),
                ),
            ),
        ));

        // ---- `Nat_mul_zero_right : m * 0 = 0` and `Nat_mul_one : m * 1 = m` -------------------
        proofs.push((
            "Nat_mul_zero_right",
            pi("m", nat(), eqn(mul(var("m"), g("Zero")), g("Zero"))),
            lam("m", nat(), refl_at(nat(), g("Zero"))),
        ));
        // ---- `Nat_add_assoc : (a + b) + c = a + (b + c)` -------------------------------------
        proofs.push((
            "Nat_add_assoc",
            pi(
                "a",
                nat(),
                pi(
                    "b",
                    nat(),
                    pi(
                        "c",
                        nat(),
                        eqn(
                            add(add(var("a"), var("b")), var("c")),
                            add(var("a"), add(var("b"), var("c"))),
                        ),
                    ),
                ),
            ),
            lam(
                "a",
                nat(),
                lam(
                    "b",
                    nat(),
                    lam(
                        "c",
                        nat(),
                        nat_ind(
                            lam(
                                "cc",
                                nat(),
                                eqn(
                                    add(add(var("a"), var("b")), var("cc")),
                                    add(var("a"), add(var("b"), var("cc"))),
                                ),
                            ),
                            refl_at(nat(), add(var("a"), var("b"))),
                            lam(
                                "j",
                                nat(),
                                lam(
                                    "ih",
                                    eqn(
                                        add(add(var("a"), var("b")), var("j")),
                                        add(var("a"), add(var("b"), var("j"))),
                                    ),
                                    congr_succ(
                                        add(add(var("a"), var("b")), var("j")),
                                        add(var("a"), add(var("b"), var("j"))),
                                        var("ih"),
                                    ),
                                ),
                            ),
                            var("c"),
                        ),
                    ),
                ),
            ),
        ));

        // ---- `Nat_mul_zero_left : 0 * n = 0` ------------------------------------------------
        proofs.push((
            "Nat_mul_zero_left",
            pi("n", nat(), eqn(mul(g("Zero"), var("n")), g("Zero"))),
            lam(
                "n",
                nat(),
                nat_ind(
                    lam("nn", nat(), eqn(mul(g("Zero"), var("nn")), g("Zero"))),
                    refl_at(nat(), g("Zero")),
                    // `0 * (j+1)` ⤳ `0*j + 0` ⤳ `0*j`, so the hypothesis IS the goal.
                    lam(
                        "j",
                        nat(),
                        lam("ih", eqn(mul(g("Zero"), var("j")), g("Zero")), var("ih")),
                    ),
                    var("n"),
                ),
            ),
        ));

        // ---- `Nat_mul_succ_left : (m+1) * n = m*n + n` --------------------------------------
        // The one lemma with real content. Its step is four rewrites: the hypothesis under
        // `_ + m`, associativity out, commutativity of the inner pair, associativity back.
        let add_r = |k: Term| lam("x", nat(), apps(g("Nat_add"), &[var("x"), k]));
        let add_l = |k: Term| lam("x", nat(), apps(g("Nat_add"), &[k, var("x")]));
        let assoc = |a: Term, b: Term, c: Term| apps(g("Nat_add_assoc"), &[a, b, c]);
        let comm = |a: Term, b: Term| apps(g("Nat_add_comm"), &[a, b]);
        let p = || mul(var("m"), var("j"));
        proofs.push((
            "Nat_mul_succ_left",
            pi(
                "m",
                nat(),
                pi(
                    "n",
                    nat(),
                    eqn(
                        mul(succ(var("m")), var("n")),
                        add(mul(var("m"), var("n")), var("n")),
                    ),
                ),
            ),
            lam(
                "m",
                nat(),
                lam(
                    "n",
                    nat(),
                    nat_ind(
                        lam(
                            "nn",
                            nat(),
                            eqn(
                                mul(succ(var("m")), var("nn")),
                                add(mul(var("m"), var("nn")), var("nn")),
                            ),
                        ),
                        refl_at(nat(), g("Zero")),
                        lam(
                            "j",
                            nat(),
                            lam(
                                "ih",
                                eqn(
                                    mul(succ(var("m")), var("j")),
                                    add(mul(var("m"), var("j")), var("j")),
                                ),
                                congr_succ(
                                    add(mul(succ(var("m")), var("j")), var("m")),
                                    add(add(p(), var("m")), var("j")),
                                    gtrans(
                                        nat(),
                                        add(mul(succ(var("m")), var("j")), var("m")),
                                        add(p(), add(var("m"), var("j"))),
                                        add(add(p(), var("m")), var("j")),
                                        gtrans(
                                            nat(),
                                            add(mul(succ(var("m")), var("j")), var("m")),
                                            add(p(), add(var("j"), var("m"))),
                                            add(p(), add(var("m"), var("j"))),
                                            gtrans(
                                                nat(),
                                                add(mul(succ(var("m")), var("j")), var("m")),
                                                add(add(p(), var("j")), var("m")),
                                                add(p(), add(var("j"), var("m"))),
                                                gcongr(
                                                    nat(),
                                                    nat(),
                                                    add_r(var("m")),
                                                    mul(succ(var("m")), var("j")),
                                                    add(p(), var("j")),
                                                    var("ih"),
                                                ),
                                                assoc(p(), var("j"), var("m")),
                                            ),
                                            gcongr(
                                                nat(),
                                                nat(),
                                                add_l(p()),
                                                add(var("j"), var("m")),
                                                add(var("m"), var("j")),
                                                comm(var("j"), var("m")),
                                            ),
                                        ),
                                        gsym(
                                            nat(),
                                            add(add(p(), var("m")), var("j")),
                                            add(p(), add(var("m"), var("j"))),
                                            assoc(p(), var("m"), var("j")),
                                        ),
                                    ),
                                ),
                            ),
                        ),
                        var("n"),
                    ),
                ),
            ),
        ));

        // ---- `Nat_mul_comm : m * n = n * m` -------------------------------------------------
        proofs.push((
            "Nat_mul_comm",
            pi(
                "m",
                nat(),
                pi(
                    "n",
                    nat(),
                    eqn(mul(var("m"), var("n")), mul(var("n"), var("m"))),
                ),
            ),
            lam(
                "m",
                nat(),
                lam(
                    "n",
                    nat(),
                    nat_ind(
                        lam(
                            "mm",
                            nat(),
                            eqn(mul(var("mm"), var("n")), mul(var("n"), var("mm"))),
                        ),
                        app(g("Nat_mul_zero_left"), var("n")),
                        lam(
                            "k",
                            nat(),
                            lam(
                                "ih",
                                eqn(mul(var("k"), var("n")), mul(var("n"), var("k"))),
                                gtrans(
                                    nat(),
                                    mul(succ(var("k")), var("n")),
                                    add(mul(var("k"), var("n")), var("n")),
                                    add(mul(var("n"), var("k")), var("n")),
                                    apps(g("Nat_mul_succ_left"), &[var("k"), var("n")]),
                                    gcongr(
                                        nat(),
                                        nat(),
                                        add_r(var("n")),
                                        mul(var("k"), var("n")),
                                        mul(var("n"), var("k")),
                                        var("ih"),
                                    ),
                                ),
                            ),
                        ),
                        var("m"),
                    ),
                ),
            ),
        ));

        // Multiplication by one unfolds to zero plus m. Reuse the proved
        // left-zero equation; no additional induction is necessary.
        proofs.push((
            "Nat_mul_one",
            pi("m", nat(), eqn(mul(var("m"), succ(g("Zero"))), var("m"))),
            lam("m", nat(), app(g("Nat_zero_add"), var("m"))),
        ));
        let zero = || gof(g("Zero"));
        let add_zero_of_nat = lam(
            "m",
            nat(),
            apps(
                g("Nat_rec"),
                &[
                    lam(
                        "m",
                        nat(),
                        eq_gint(apps(g("add"), &[gof(var("m")), zero()]), gof(var("m"))),
                    ),
                    refl_gint(gof(g("Zero"))),
                    lam(
                        "k",
                        nat(),
                        lam(
                            "ih",
                            eq_gint(apps(g("add"), &[gof(var("k")), zero()]), gof(var("k"))),
                            refl_gint(gof(succ(var("k")))),
                        ),
                    ),
                    var("m"),
                ],
            ),
        );
        let add_zero_tm = lam(
            "a",
            gint(),
            apps(
                g("Int_rec"),
                &[
                    lam(
                        "a",
                        gint(),
                        eq_gint(apps(g("add"), &[var("a"), zero()]), var("a")),
                    ),
                    add_zero_of_nat,
                    lam("m", nat(), refl_gint(gneg_succ(var("m")))),
                    var("a"),
                ],
            ),
        );
        let add_zero_ty = pi(
            "a",
            gint(),
            eq_gint(apps(g("add"), &[var("a"), zero()]), var("a")),
        );
        proofs.push(("add_zero", add_zero_ty, add_zero_tm));
        let mul_zero_ty = pi(
            "a",
            gint(),
            eq_gint(apps(g("mul"), &[var("a"), zero()]), zero()),
        );
        let mul_zero_of_nat = lam(
            "m",
            nat(),
            apps(
                g("Nat_rec"),
                &[
                    lam(
                        "m",
                        nat(),
                        eq_gint(apps(g("mul"), &[gof(var("m")), zero()]), zero()),
                    ),
                    refl_gint(zero()),
                    lam(
                        "k",
                        nat(),
                        lam(
                            "ih",
                            eq_gint(apps(g("mul"), &[gof(var("k")), zero()]), zero()),
                            refl_gint(zero()),
                        ),
                    ),
                    var("m"),
                ],
            ),
        );
        let mul_zero_tm = lam(
            "a",
            gint(),
            apps(
                g("Int_rec"),
                &[
                    lam(
                        "a",
                        gint(),
                        eq_gint(apps(g("mul"), &[var("a"), zero()]), zero()),
                    ),
                    mul_zero_of_nat,
                    lam("m", nat(), refl_gint(zero())),
                    var("a"),
                ],
            ),
        );
        proofs.push(("mul_zero", mul_zero_ty, mul_zero_tm));
        let add = |a: Term, b: Term| apps(g("add"), &[a, b]);
        let nadd = |a: Term, b: Term| apps(g("Nat_add"), &[a, b]);

        // ---- add_comm : Π(a b : Int). a + b = b + a --------------------------------------
        let add_comm_ty = pi(
            "a",
            gint(),
            pi(
                "b",
                gint(),
                eq_gint(add(var("a"), var("b")), add(var("b"), var("a"))),
            ),
        );
        let neg_succ_succ = lam("x", nat(), gneg_succ(succ(var("x"))));
        let add_comm_tm = lam(
            "a",
            gint(),
            lam(
                "b",
                gint(),
                gint_ind(
                    lam(
                        "aa",
                        gint(),
                        eq_gint(add(var("aa"), var("b")), add(var("b"), var("aa"))),
                    ),
                    // a = ofNat m: split on b.
                    lam(
                        "m",
                        nat(),
                        gint_ind(
                            lam(
                                "bb",
                                gint(),
                                eq_gint(
                                    add(gof(var("m")), var("bb")),
                                    add(var("bb"), gof(var("m"))),
                                ),
                            ),
                            lam(
                                "n",
                                nat(),
                                gcongr(
                                    nat(),
                                    gint(),
                                    g("Int_ofNat"),
                                    nadd(var("m"), var("n")),
                                    nadd(var("n"), var("m")),
                                    apps(g("Nat_add_comm"), &[var("m"), var("n")]),
                                ),
                            ),
                            // `ofNat m + negSucc n` and `negSucc n + ofNat m` are the SAME term.
                            lam(
                                "n",
                                nat(),
                                refl_gint(apps(g("Int_subNatNat"), &[var("m"), succ(var("n"))])),
                            ),
                            var("b"),
                        ),
                    ),
                    // a = negSucc m: split on b.
                    lam(
                        "m",
                        nat(),
                        gint_ind(
                            lam(
                                "bb",
                                gint(),
                                eq_gint(
                                    add(gneg_succ(var("m")), var("bb")),
                                    add(var("bb"), gneg_succ(var("m"))),
                                ),
                            ),
                            lam(
                                "n",
                                nat(),
                                refl_gint(apps(g("Int_subNatNat"), &[var("n"), succ(var("m"))])),
                            ),
                            lam(
                                "n",
                                nat(),
                                gcongr(
                                    nat(),
                                    gint(),
                                    neg_succ_succ.clone(),
                                    nadd(var("m"), var("n")),
                                    nadd(var("n"), var("m")),
                                    apps(g("Nat_add_comm"), &[var("m"), var("n")]),
                                ),
                            ),
                            var("b"),
                        ),
                    ),
                    var("a"),
                ),
            ),
        );
        proofs.push(("add_comm", add_comm_ty, add_comm_tm));
        let mul_one_ty = pi(
            "a",
            gint(),
            eq_gint(apps(g("mul"), &[var("a"), gone()]), var("a")),
        );
        let mul_one_tm = lam(
            "a",
            gint(),
            gint_ind(
                lam(
                    "aa",
                    gint(),
                    eq_gint(apps(g("mul"), &[var("aa"), gone()]), var("aa")),
                ),
                lam(
                    "m",
                    nat(),
                    gcongr(
                        nat(),
                        gint(),
                        g("Int_ofNat"),
                        apps(g("Nat_mul"), &[var("m"), succ(g("Zero"))]),
                        var("m"),
                        app(g("Nat_mul_one"), var("m")),
                    ),
                ),
                lam(
                    "m",
                    nat(),
                    gcongr(
                        nat(),
                        gint(),
                        g("Int_negOfNat"),
                        apps(g("Nat_mul"), &[succ(var("m")), succ(g("Zero"))]),
                        succ(var("m")),
                        app(g("Nat_mul_one"), succ(var("m"))),
                    ),
                ),
                var("a"),
            ),
        );
        proofs.push(("mul_one", mul_one_ty, mul_one_tm));
        let imul = |a: Term, b: Term| apps(g("mul"), &[a, b]);
        let ncomm = |a: Term, b: Term| apps(g("Nat_mul_comm"), &[a, b]);
        let nmul = |a: Term, b: Term| apps(g("Nat_mul"), &[a, b]);

        let mul_comm_ty = pi(
            "a",
            gint(),
            pi(
                "b",
                gint(),
                eq_gint(imul(var("a"), var("b")), imul(var("b"), var("a"))),
            ),
        );
        let mul_comm_tm = lam(
            "a",
            gint(),
            lam(
                "b",
                gint(),
                gint_ind(
                    lam(
                        "aa",
                        gint(),
                        eq_gint(imul(var("aa"), var("b")), imul(var("b"), var("aa"))),
                    ),
                    lam(
                        "m",
                        nat(),
                        gint_ind(
                            lam(
                                "bb",
                                gint(),
                                eq_gint(
                                    imul(gof(var("m")), var("bb")),
                                    imul(var("bb"), gof(var("m"))),
                                ),
                            ),
                            lam(
                                "n",
                                nat(),
                                gcongr(
                                    nat(),
                                    gint(),
                                    g("Int_ofNat"),
                                    nmul(var("m"), var("n")),
                                    nmul(var("n"), var("m")),
                                    ncomm(var("m"), var("n")),
                                ),
                            ),
                            lam(
                                "n",
                                nat(),
                                gcongr(
                                    nat(),
                                    gint(),
                                    g("Int_negOfNat"),
                                    nmul(var("m"), succ(var("n"))),
                                    nmul(succ(var("n")), var("m")),
                                    ncomm(var("m"), succ(var("n"))),
                                ),
                            ),
                            var("b"),
                        ),
                    ),
                    lam(
                        "m",
                        nat(),
                        gint_ind(
                            lam(
                                "bb",
                                gint(),
                                eq_gint(
                                    imul(gneg_succ(var("m")), var("bb")),
                                    imul(var("bb"), gneg_succ(var("m"))),
                                ),
                            ),
                            lam(
                                "n",
                                nat(),
                                gcongr(
                                    nat(),
                                    gint(),
                                    g("Int_negOfNat"),
                                    nmul(succ(var("m")), var("n")),
                                    nmul(var("n"), succ(var("m"))),
                                    ncomm(succ(var("m")), var("n")),
                                ),
                            ),
                            lam(
                                "n",
                                nat(),
                                gcongr(
                                    nat(),
                                    gint(),
                                    g("Int_ofNat"),
                                    nmul(succ(var("m")), succ(var("n"))),
                                    nmul(succ(var("n")), succ(var("m"))),
                                    ncomm(succ(var("m")), succ(var("n"))),
                                ),
                            ),
                            var("b"),
                        ),
                    ),
                    var("a"),
                ),
            ),
        );
        proofs.push(("mul_comm", mul_comm_ty, mul_comm_tm));
        append_multiplication_associativity(&mut proofs);
        append_addition_associativity(&mut proofs);
        append_integer_distributivity(&mut proofs);
        append_integer_subtraction(&mut proofs);
        append_integer_order(&mut proofs);
        append_integer_totality(&mut proofs);
        append_integer_successor_order(&mut proofs);
        append_integer_ordered_addition(&mut proofs);
        append_integer_multiplication_order(&mut proofs);

        proofs
    })
}

// Multiplication associativity uses two Nat induction facts and the existing
// canonical integer constructors. The eight signed cases are equality chains;
// no ring prover or assumed integer associativity participates in construction.
fn append_multiplication_associativity(proofs: &mut Vec<Proof>) {
    let add = |a: Term, b: Term| apps(g("Nat_add"), &[a, b]);
    let mul = |a: Term, b: Term| apps(g("Nat_mul"), &[a, b]);
    let eqn = |a: Term, b: Term| eq_at(nat(), a, b);
    let a = var("a");
    let b = var("b");
    let c = var("c");
    let j = var("j");
    let forall3 =
        |ty: Term, body: Term| pi("a", ty.clone(), pi("b", ty.clone(), pi("c", ty, body)));
    let lambda3 =
        |ty: Term, body: Term| lam("a", ty.clone(), lam("b", ty.clone(), lam("c", ty, body)));
    let distrib = |c: Term| {
        eqn(
            mul(a.clone(), add(b.clone(), c.clone())),
            add(mul(a.clone(), b.clone()), mul(a.clone(), c)),
        )
    };
    let x = mul(a.clone(), add(b.clone(), j.clone()));
    let y = add(mul(a.clone(), b.clone()), mul(a.clone(), j.clone()));
    proofs.push((
        "Nat_mul_add",
        forall3(nat(), distrib(c.clone())),
        lambda3(
            nat(),
            nat_ind(
                lam("cc", nat(), distrib(var("cc"))),
                refl_at(nat(), mul(a.clone(), b.clone())),
                lam(
                    "j",
                    nat(),
                    lam(
                        "ih",
                        distrib(j.clone()),
                        gtrans(
                            nat(),
                            add(x.clone(), a.clone()),
                            add(y.clone(), a.clone()),
                            add(
                                mul(a.clone(), b.clone()),
                                add(mul(a.clone(), j.clone()), a.clone()),
                            ),
                            gcongr(
                                nat(),
                                nat(),
                                lam("x", nat(), add(var("x"), a.clone())),
                                x,
                                y,
                                var("ih"),
                            ),
                            apps(
                                g("Nat_add_assoc"),
                                &[
                                    mul(a.clone(), b.clone()),
                                    mul(a.clone(), j.clone()),
                                    a.clone(),
                                ],
                            ),
                        ),
                    ),
                ),
                c.clone(),
            ),
        ),
    ));
    let assoc = |c: Term| {
        eqn(
            mul(mul(a.clone(), b.clone()), c.clone()),
            mul(a.clone(), mul(b.clone(), c)),
        )
    };
    let x = mul(mul(a.clone(), b.clone()), j.clone());
    let y = mul(a.clone(), mul(b.clone(), j.clone()));
    let after = add(y.clone(), mul(a.clone(), b.clone()));
    let right = mul(a.clone(), add(mul(b.clone(), j.clone()), b.clone()));
    proofs.push((
        "Nat_mul_assoc",
        forall3(nat(), assoc(c.clone())),
        lambda3(
            nat(),
            nat_ind(
                lam("cc", nat(), assoc(var("cc"))),
                refl_at(nat(), g("Zero")),
                lam(
                    "j",
                    nat(),
                    lam(
                        "ih",
                        assoc(j.clone()),
                        gtrans(
                            nat(),
                            add(x.clone(), mul(a.clone(), b.clone())),
                            after.clone(),
                            right.clone(),
                            gcongr(
                                nat(),
                                nat(),
                                lam("x", nat(), add(var("x"), mul(a.clone(), b.clone()))),
                                x,
                                y,
                                var("ih"),
                            ),
                            gsym(
                                nat(),
                                right,
                                after,
                                apps(
                                    g("Nat_mul_add"),
                                    &[a.clone(), mul(b.clone(), j.clone()), b.clone()],
                                ),
                            ),
                        ),
                    ),
                ),
                c.clone(),
            ),
        ),
    ));

    let neg = |n: Term| app(g("Int_negOfNat"), n);
    let imul = |a: Term, b: Term| apps(g("mul"), &[a, b]);
    let of_neg_goal =
        |a: Term, b: Term| eq_gint(imul(gof(a.clone()), neg(b.clone())), neg(mul(a, b)));
    proofs.push((
        "IntLaw_mul_ofNat_negOfNat",
        pi(
            "a",
            nat(),
            pi("b", nat(), of_neg_goal(a.clone(), b.clone())),
        ),
        lam(
            "a",
            nat(),
            lam(
                "b",
                nat(),
                nat_ind(
                    lam("bb", nat(), of_neg_goal(a.clone(), var("bb"))),
                    refl_gint(gof(g("Zero"))),
                    lam(
                        "j",
                        nat(),
                        lam(
                            "ih",
                            of_neg_goal(a.clone(), j.clone()),
                            refl_gint(neg(mul(a.clone(), succ(j.clone())))),
                        ),
                    ),
                    b.clone(),
                ),
            ),
        ),
    ));
    let left = imul(neg(a.clone()), gof(b.clone()));
    let mid = imul(gof(b.clone()), neg(a.clone()));
    let flipped = neg(mul(b.clone(), a.clone()));
    let right = neg(mul(a.clone(), b.clone()));
    proofs.push((
        "IntLaw_mul_negOfNat_ofNat",
        pi(
            "a",
            nat(),
            pi("b", nat(), eq_gint(left.clone(), right.clone())),
        ),
        lam(
            "a",
            nat(),
            lam(
                "b",
                nat(),
                gtrans(
                    gint(),
                    left,
                    mid.clone(),
                    right.clone(),
                    apps(g("mul_comm"), &[neg(a.clone()), gof(b.clone())]),
                    gtrans(
                        gint(),
                        mid,
                        flipped.clone(),
                        right,
                        apps(g("IntLaw_mul_ofNat_negOfNat"), &[b.clone(), a.clone()]),
                        gcongr(
                            nat(),
                            gint(),
                            g("Int_negOfNat"),
                            mul(b.clone(), a.clone()),
                            mul(a.clone(), b.clone()),
                            apps(g("Nat_mul_comm"), &[b.clone(), a.clone()]),
                        ),
                    ),
                ),
            ),
        ),
    ));
    let neg_neg_goal =
        |a: Term, b: Term| eq_gint(imul(neg(a.clone()), neg(b.clone())), gof(mul(a, b)));
    let lhs0 = imul(gof(g("Zero")), neg(b.clone()));
    let mid0 = imul(neg(b.clone()), gof(g("Zero")));
    let rhs0 = gof(mul(g("Zero"), b.clone()));
    let zero_case = lam(
        "b",
        nat(),
        gtrans(
            gint(),
            lhs0,
            mid0.clone(),
            rhs0.clone(),
            apps(g("mul_comm"), &[gof(g("Zero")), neg(b.clone())]),
            gtrans(
                gint(),
                mid0,
                gof(g("Zero")),
                rhs0.clone(),
                app(g("mul_zero"), neg(b.clone())),
                gsym(
                    gint(),
                    rhs0,
                    gof(g("Zero")),
                    gcongr(
                        nat(),
                        gint(),
                        g("Int_ofNat"),
                        mul(g("Zero"), b.clone()),
                        g("Zero"),
                        app(g("Nat_mul_zero_left"), b.clone()),
                    ),
                ),
            ),
        ),
    );
    let k = var("k");
    let succ_case = lam(
        "k",
        nat(),
        lam(
            "ih",
            pi("b", nat(), neg_neg_goal(k.clone(), b.clone())),
            lam(
                "b",
                nat(),
                nat_ind(
                    lam("bb", nat(), neg_neg_goal(succ(k.clone()), var("bb"))),
                    refl_gint(gof(g("Zero"))),
                    lam(
                        "j",
                        nat(),
                        lam(
                            "ihb",
                            neg_neg_goal(succ(k.clone()), j.clone()),
                            refl_gint(gof(mul(succ(k.clone()), succ(j.clone())))),
                        ),
                    ),
                    b.clone(),
                ),
            ),
        ),
    );
    proofs.push((
        "IntLaw_mul_negOfNat_negOfNat",
        pi(
            "a",
            nat(),
            pi("b", nat(), neg_neg_goal(a.clone(), b.clone())),
        ),
        lam(
            "a",
            nat(),
            nat_ind(
                lam(
                    "aa",
                    nat(),
                    pi("b", nat(), neg_neg_goal(var("aa"), b.clone())),
                ),
                zero_case,
                succ_case,
                a.clone(),
            ),
        ),
    ));

    let int_assoc = integer_assoc_goal(a.clone(), b.clone(), c.clone());
    proofs.push((
        "mul_assoc",
        forall3(gint(), int_assoc),
        lambda3(
            gint(),
            integer_assoc_cases(0, [a, b, c], [false; 3], [g("Zero"), g("Zero"), g("Zero")]),
        ),
    ));
}

fn signed_natural(negative: bool, n: Term) -> Term {
    app(
        g(if negative {
            "Int_negOfNat"
        } else {
            "Int_ofNat"
        }),
        n,
    )
}

fn signed_product(negative_a: bool, negative_b: bool, a: Term, b: Term) -> Term {
    let name = match (negative_a, negative_b) {
        (false, false) => return refl_gint(gof(apps(g("Nat_mul"), &[a, b]))),
        (false, true) => "IntLaw_mul_ofNat_negOfNat",
        (true, false) => "IntLaw_mul_negOfNat_ofNat",
        (true, true) => "IntLaw_mul_negOfNat_negOfNat",
    };
    apps(g(name), &[a, b])
}

fn integer_assoc_goal(a: Term, b: Term, c: Term) -> Term {
    eq_gint(
        apps(
            g("mul"),
            &[apps(g("mul"), &[a.clone(), b.clone()]), c.clone()],
        ),
        apps(g("mul"), &[a, apps(g("mul"), &[b, c])]),
    )
}

fn integer_assoc_cases(
    depth: usize,
    values: [Term; 3],
    signs: [bool; 3],
    magnitudes: [Term; 3],
) -> Term {
    if depth == 3 {
        return integer_assoc_leaf(signs, magnitudes);
    }
    let binder = ["case_a", "case_b", "case_c"][depth];
    let nat_binder = ["na", "nb", "nc"][depth];
    let mut motive_values = values.clone();
    motive_values[depth] = var(binder);
    let motive = lam(
        binder,
        gint(),
        integer_assoc_goal(
            motive_values[0].clone(),
            motive_values[1].clone(),
            motive_values[2].clone(),
        ),
    );
    let branches: [Term; 2] = std::array::from_fn(|branch| {
        let mut values = values.clone();
        let mut signs = signs;
        let mut magnitudes = magnitudes.clone();
        signs[depth] = branch == 1;
        magnitudes[depth] = if branch == 0 {
            var(nat_binder)
        } else {
            succ(var(nat_binder))
        };
        values[depth] = signed_natural(signs[depth], magnitudes[depth].clone());
        lam(
            nat_binder,
            nat(),
            integer_assoc_cases(depth + 1, values, signs, magnitudes),
        )
    });
    gint_ind(
        motive,
        branches[0].clone(),
        branches[1].clone(),
        values[depth].clone(),
    )
}

fn integer_assoc_leaf(signs: [bool; 3], magnitudes: [Term; 3]) -> Term {
    let [a, b, c] = magnitudes;
    let [sa, sb, sc] = signs;
    let imul = |a: Term, b: Term| apps(g("mul"), &[a, b]);
    let nmul = |a: Term, b: Term| apps(g("Nat_mul"), &[a, b]);
    let ia = signed_natural(sa, a.clone());
    let ib = signed_natural(sb, b.clone());
    let ic = signed_natural(sc, c.clone());
    let ab = nmul(a.clone(), b.clone());
    let bc = nmul(b.clone(), c.clone());
    let iab = signed_natural(sa ^ sb, ab.clone());
    let ibc = signed_natural(sb ^ sc, bc.clone());
    let left = imul(imul(ia.clone(), ib.clone()), ic.clone());
    let left_mid = imul(iab.clone(), ic.clone());
    let left_nat = nmul(ab.clone(), c.clone());
    let right_nat = nmul(a.clone(), bc.clone());
    let left_normal = signed_natural(sa ^ sb ^ sc, left_nat.clone());
    let right_normal = signed_natural(sa ^ sb ^ sc, right_nat.clone());
    let right = imul(ia.clone(), imul(ib.clone(), ic.clone()));
    let right_mid = imul(ia.clone(), ibc.clone());
    let left_proof = gtrans(
        gint(),
        left.clone(),
        left_mid,
        left_normal.clone(),
        gcongr(
            gint(),
            gint(),
            lam("x", gint(), imul(var("x"), ic.clone())),
            imul(ia.clone(), ib.clone()),
            iab,
            signed_product(sa, sb, a.clone(), b.clone()),
        ),
        signed_product(sa ^ sb, sc, ab, c.clone()),
    );
    let right_proof = gtrans(
        gint(),
        right.clone(),
        right_mid,
        right_normal.clone(),
        gcongr(
            gint(),
            gint(),
            lam("x", gint(), imul(ia.clone(), var("x"))),
            imul(ib, ic),
            ibc,
            signed_product(sb, sc, b.clone(), c.clone()),
        ),
        signed_product(sa, sb ^ sc, a.clone(), bc),
    );
    let natural_step = gcongr(
        nat(),
        gint(),
        g(if sa ^ sb ^ sc {
            "Int_negOfNat"
        } else {
            "Int_ofNat"
        }),
        left_nat,
        right_nat,
        apps(g("Nat_mul_assoc"), &[a, b, c]),
    );
    gtrans(
        gint(),
        left,
        left_normal.clone(),
        right.clone(),
        left_proof,
        gtrans(
            gint(),
            left_normal,
            right_normal.clone(),
            right.clone(),
            natural_step,
            gsym(gint(), right, right_normal, right_proof),
        ),
    )
}

// The proof builders below only construct ordinary equality/recursor terms.
// In particular none searches with, or refers to, the assumed add_assoc.
fn law_chain(ty: Term, start: Term, steps: Vec<(Term, Term)>) -> Term {
    let mut iter = steps.into_iter();
    let Some((mut end, mut proof)) = iter.next() else {
        return refl_at(ty, start);
    };
    for (next, step) in iter {
        proof = gtrans(ty.clone(), start.clone(), end, next.clone(), proof, step);
        end = next;
    }
    proof
}
fn nadd(a: Term, b: Term) -> Term {
    apps(g("Nat_add"), &[a, b])
}
fn nsub(a: Term, b: Term) -> Term {
    apps(g("Nat_sub"), &[a, b])
}
fn idiff(a: Term, b: Term) -> Term {
    apps(g("Int_subNatNat"), &[a, b])
}
fn iadd(a: Term, b: Term) -> Term {
    apps(g("add"), &[a, b])
}
fn all2(ty: Term, body: Term) -> Term {
    pi("a", ty.clone(), pi("b", ty, body))
}
fn abs2(ty: Term, body: Term) -> Term {
    lam("a", ty.clone(), lam("b", ty, body))
}
fn all3(ty: Term, body: Term) -> Term {
    pi("a", ty.clone(), pi("b", ty.clone(), pi("c", ty, body)))
}
fn abs3(ty: Term, body: Term) -> Term {
    lam("a", ty.clone(), lam("b", ty.clone(), lam("c", ty, body)))
}
fn diff_match(negative: Term, positive: Term) -> Term {
    Term::Match {
        discriminant: Box::new(negative),
        motive: Box::new(lam("_", nat(), gint())),
        cases: vec![gof(positive), lam("dk", nat(), gneg_succ(var("dk")))],
    }
}
fn diff_zero_right(a: Term) -> Term {
    gcongr(
        nat(),
        gint(),
        lam("r", nat(), diff_match(var("r"), a.clone())),
        nsub(g("Zero"), a.clone()),
        g("Zero"),
        app(g("Nat_sub_zero_left"), a),
    )
}
fn diff_cancel(a: Term, b: Term) -> Term {
    apps(g("IntLaw_subNatNat_succ_succ"), &[a, b])
}
fn append_addition_associativity(proofs: &mut Vec<Proof>) {
    let (a, b, c, k, j) = (var("a"), var("b"), var("c"), var("k"), var("j"));
    let zero = g("Zero");
    let zero_goal = |x: Term| eq_at(nat(), nsub(zero.clone(), x), zero.clone());
    proofs.push((
        "Nat_sub_zero_left",
        pi("a", nat(), zero_goal(a.clone())),
        lam(
            "a",
            nat(),
            nat_ind(
                lam("n", nat(), zero_goal(var("n"))),
                refl_at(nat(), zero.clone()),
                lam(
                    "k",
                    nat(),
                    lam(
                        "ih",
                        zero_goal(k.clone()),
                        gcongr(
                            nat(),
                            nat(),
                            g("Nat_pred"),
                            nsub(zero.clone(), k.clone()),
                            zero.clone(),
                            var("ih"),
                        ),
                    ),
                ),
                a.clone(),
            ),
        ),
    ));
    let sub_ss =
        |x: Term, y: Term| eq_at(nat(), nsub(succ(x.clone()), succ(y.clone())), nsub(x, y));
    proofs.push((
        "Nat_sub_succ_succ",
        all2(nat(), sub_ss(a.clone(), b.clone())),
        abs2(
            nat(),
            nat_ind(
                lam("n", nat(), sub_ss(a.clone(), var("n"))),
                refl_at(nat(), a.clone()),
                lam(
                    "k",
                    nat(),
                    lam(
                        "ih",
                        sub_ss(a.clone(), k.clone()),
                        gcongr(
                            nat(),
                            nat(),
                            g("Nat_pred"),
                            nsub(succ(a.clone()), succ(k.clone())),
                            nsub(a.clone(), k.clone()),
                            var("ih"),
                        ),
                    ),
                ),
                b.clone(),
            ),
        ),
    ));
    let ds = idiff(succ(a.clone()), succ(b.clone()));
    let mid = diff_match(
        nsub(succ(b.clone()), succ(a.clone())),
        nsub(a.clone(), b.clone()),
    );
    let end = idiff(a.clone(), b.clone());
    proofs.push((
        "IntLaw_subNatNat_succ_succ",
        all2(nat(), eq_gint(ds.clone(), end.clone())),
        abs2(
            nat(),
            law_chain(
                gint(),
                ds,
                vec![
                    (
                        mid,
                        gcongr(
                            nat(),
                            gint(),
                            lam(
                                "r",
                                nat(),
                                diff_match(nsub(succ(b.clone()), succ(a.clone())), var("r")),
                            ),
                            nsub(succ(a.clone()), succ(b.clone())),
                            nsub(a.clone(), b.clone()),
                            apps(g("Nat_sub_succ_succ"), &[a.clone(), b.clone()]),
                        ),
                    ),
                    (
                        end,
                        gcongr(
                            nat(),
                            gint(),
                            lam("r", nat(), diff_match(var("r"), nsub(a.clone(), b.clone()))),
                            nsub(succ(b.clone()), succ(a.clone())),
                            nsub(b.clone(), a.clone()),
                            apps(g("Nat_sub_succ_succ"), &[b.clone(), a.clone()]),
                        ),
                    ),
                ],
            ),
        ),
    ));
    // Difference boundary proofs cancel one successor on each side per step.
    let pos_goal = |x: Term, y: Term| eq_gint(idiff(nadd(x.clone(), y.clone()), x), gof(y));
    let neg_goal =
        |x: Term, y: Term| eq_gint(idiff(x.clone(), nadd(x, succ(y.clone()))), gneg_succ(y));
    let pos0 = lam(
        "b",
        nat(),
        law_chain(
            gint(),
            idiff(nadd(zero.clone(), b.clone()), zero.clone()),
            vec![
                (
                    idiff(b.clone(), zero.clone()),
                    gcongr(
                        nat(),
                        gint(),
                        lam("r", nat(), idiff(var("r"), zero.clone())),
                        nadd(zero.clone(), b.clone()),
                        b.clone(),
                        app(g("Nat_zero_add"), b.clone()),
                    ),
                ),
                (gof(b.clone()), diff_zero_right(b.clone())),
            ],
        ),
    );
    let pos_step = lam(
        "k",
        nat(),
        lam(
            "ih",
            pi("b", nat(), pos_goal(k.clone(), b.clone())),
            lam(
                "b",
                nat(),
                law_chain(
                    gint(),
                    idiff(nadd(succ(k.clone()), b.clone()), succ(k.clone())),
                    vec![
                        (
                            idiff(succ(nadd(k.clone(), b.clone())), succ(k.clone())),
                            gcongr(
                                nat(),
                                gint(),
                                lam("r", nat(), idiff(var("r"), succ(k.clone()))),
                                nadd(succ(k.clone()), b.clone()),
                                succ(nadd(k.clone(), b.clone())),
                                apps(g("Nat_add_succ_left"), &[k.clone(), b.clone()]),
                            ),
                        ),
                        (
                            idiff(nadd(k.clone(), b.clone()), k.clone()),
                            diff_cancel(nadd(k.clone(), b.clone()), k.clone()),
                        ),
                        (gof(b.clone()), app(var("ih"), b.clone())),
                    ],
                ),
            ),
        ),
    );
    proofs.push((
        "IntLaw_subNatNat_add_left",
        all2(nat(), pos_goal(a.clone(), b.clone())),
        lam(
            "a",
            nat(),
            nat_ind(
                lam("n", nat(), pi("b", nat(), pos_goal(var("n"), b.clone()))),
                pos0,
                pos_step,
                a.clone(),
            ),
        ),
    ));
    let neg0 = lam(
        "b",
        nat(),
        gcongr(
            nat(),
            gint(),
            lam("r", nat(), idiff(zero.clone(), var("r"))),
            nadd(zero.clone(), succ(b.clone())),
            succ(b.clone()),
            app(g("Nat_zero_add"), succ(b.clone())),
        ),
    );
    let neg_step = lam(
        "k",
        nat(),
        lam(
            "ih",
            pi("b", nat(), neg_goal(k.clone(), b.clone())),
            lam(
                "b",
                nat(),
                law_chain(
                    gint(),
                    idiff(succ(k.clone()), nadd(succ(k.clone()), succ(b.clone()))),
                    vec![
                        (
                            idiff(succ(k.clone()), succ(nadd(k.clone(), succ(b.clone())))),
                            gcongr(
                                nat(),
                                gint(),
                                lam("r", nat(), idiff(succ(k.clone()), var("r"))),
                                nadd(succ(k.clone()), succ(b.clone())),
                                succ(nadd(k.clone(), succ(b.clone()))),
                                apps(g("Nat_add_succ_left"), &[k.clone(), succ(b.clone())]),
                            ),
                        ),
                        (
                            idiff(k.clone(), nadd(k.clone(), succ(b.clone()))),
                            diff_cancel(k.clone(), nadd(k.clone(), succ(b.clone()))),
                        ),
                        (gneg_succ(b.clone()), app(var("ih"), b.clone())),
                    ],
                ),
            ),
        ),
    );
    proofs.push((
        "IntLaw_subNatNat_add_right",
        all2(nat(), neg_goal(a.clone(), b.clone())),
        lam(
            "a",
            nat(),
            nat_ind(
                lam("n", nat(), pi("b", nat(), neg_goal(var("n"), b.clone()))),
                neg0,
                neg_step,
                a.clone(),
            ),
        ),
    ));
    // F(a,b,c): (a+b)-c = a+(b-c). Induct on c with a,b generalized.
    let fgoal = |x: Term, y: Term, z: Term| {
        eq_gint(
            idiff(nadd(x.clone(), y.clone()), z.clone()),
            iadd(gof(x), idiff(y, z)),
        )
    };
    let f0 = abs2(
        nat(),
        law_chain(
            gint(),
            idiff(nadd(a.clone(), b.clone()), zero.clone()),
            vec![
                (
                    gof(nadd(a.clone(), b.clone())),
                    diff_zero_right(nadd(a.clone(), b.clone())),
                ),
                (
                    iadd(gof(a.clone()), idiff(b.clone(), zero.clone())),
                    gsym(
                        gint(),
                        iadd(gof(a.clone()), idiff(b.clone(), zero.clone())),
                        iadd(gof(a.clone()), gof(b.clone())),
                        gcongr(
                            gint(),
                            gint(),
                            lam("r", gint(), iadd(gof(a.clone()), var("r"))),
                            idiff(b.clone(), zero.clone()),
                            gof(b.clone()),
                            diff_zero_right(b.clone()),
                        ),
                    ),
                ),
            ],
        ),
    );
    let f_step = lam(
        "k",
        nat(),
        lam(
            "ih",
            all2(nat(), fgoal(a.clone(), b.clone(), k.clone())),
            lam(
                "a",
                nat(),
                lam(
                    "b",
                    nat(),
                    nat_ind(
                        lam("n", nat(), fgoal(a.clone(), var("n"), succ(k.clone()))),
                        refl_gint(idiff(a.clone(), succ(k.clone()))),
                        lam(
                            "j",
                            nat(),
                            lam(
                                "unused",
                                fgoal(a.clone(), j.clone(), succ(k.clone())),
                                law_chain(
                                    gint(),
                                    idiff(nadd(a.clone(), succ(j.clone())), succ(k.clone())),
                                    vec![
                                        (
                                            idiff(nadd(a.clone(), j.clone()), k.clone()),
                                            diff_cancel(nadd(a.clone(), j.clone()), k.clone()),
                                        ),
                                        (
                                            iadd(gof(a.clone()), idiff(j.clone(), k.clone())),
                                            apps(var("ih"), &[a.clone(), j.clone()]),
                                        ),
                                        (
                                            iadd(
                                                gof(a.clone()),
                                                idiff(succ(j.clone()), succ(k.clone())),
                                            ),
                                            gcongr(
                                                gint(),
                                                gint(),
                                                lam("r", gint(), iadd(gof(a.clone()), var("r"))),
                                                idiff(j.clone(), k.clone()),
                                                idiff(succ(j.clone()), succ(k.clone())),
                                                gsym(
                                                    gint(),
                                                    idiff(succ(j.clone()), succ(k.clone())),
                                                    idiff(j.clone(), k.clone()),
                                                    diff_cancel(j.clone(), k.clone()),
                                                ),
                                            ),
                                        ),
                                    ],
                                ),
                            ),
                        ),
                        b.clone(),
                    ),
                ),
            ),
        ),
    );
    proofs.push((
        "IntLaw_subNatNat_add",
        all3(nat(), fgoal(a.clone(), b.clone(), c.clone())),
        abs3(
            nat(),
            apps(
                nat_ind(
                    lam(
                        "n",
                        nat(),
                        all2(nat(), fgoal(a.clone(), b.clone(), var("n"))),
                    ),
                    f0,
                    f_step,
                    c.clone(),
                ),
                &[a.clone(), b.clone()],
            ),
        ),
    ));
    // G(a,b,c): (a-b)+(-c-1) = a-(b+c+1). Induct on b, split a.
    let ggoal = |x: Term, y: Term, z: Term| {
        eq_gint(
            iadd(idiff(x.clone(), y.clone()), gneg_succ(z.clone())),
            idiff(x, nadd(y, succ(z))),
        )
    };
    let g0 = lam(
        "a",
        nat(),
        lam(
            "c",
            nat(),
            law_chain(
                gint(),
                iadd(idiff(a.clone(), zero.clone()), gneg_succ(c.clone())),
                vec![
                    (
                        idiff(a.clone(), succ(c.clone())),
                        gcongr(
                            gint(),
                            gint(),
                            lam("r", gint(), iadd(var("r"), gneg_succ(c.clone()))),
                            idiff(a.clone(), zero.clone()),
                            gof(a.clone()),
                            diff_zero_right(a.clone()),
                        ),
                    ),
                    (
                        idiff(a.clone(), nadd(zero.clone(), succ(c.clone()))),
                        gcongr(
                            nat(),
                            gint(),
                            lam("r", nat(), idiff(a.clone(), var("r"))),
                            succ(c.clone()),
                            nadd(zero.clone(), succ(c.clone())),
                            gsym(
                                nat(),
                                nadd(zero.clone(), succ(c.clone())),
                                succ(c.clone()),
                                app(g("Nat_zero_add"), succ(c.clone())),
                            ),
                        ),
                    ),
                ],
            ),
        ),
    );
    let g_step = lam(
        "k",
        nat(),
        lam(
            "ih",
            pi(
                "a",
                nat(),
                pi("c", nat(), ggoal(a.clone(), k.clone(), c.clone())),
            ),
            lam(
                "a",
                nat(),
                lam(
                    "c",
                    nat(),
                    nat_ind(
                        lam("n", nat(), ggoal(var("n"), succ(k.clone()), c.clone())),
                        gsym(
                            gint(),
                            idiff(zero.clone(), nadd(succ(k.clone()), succ(c.clone()))),
                            gneg_succ(succ(nadd(k.clone(), c.clone()))),
                            gcongr(
                                nat(),
                                gint(),
                                lam("r", nat(), idiff(zero.clone(), var("r"))),
                                nadd(succ(k.clone()), succ(c.clone())),
                                succ(nadd(k.clone(), succ(c.clone()))),
                                apps(g("Nat_add_succ_left"), &[k.clone(), succ(c.clone())]),
                            ),
                        ),
                        lam(
                            "j",
                            nat(),
                            lam(
                                "unused",
                                ggoal(j.clone(), succ(k.clone()), c.clone()),
                                law_chain(
                                    gint(),
                                    iadd(
                                        idiff(succ(j.clone()), succ(k.clone())),
                                        gneg_succ(c.clone()),
                                    ),
                                    vec![
                                        (
                                            iadd(idiff(j.clone(), k.clone()), gneg_succ(c.clone())),
                                            gcongr(
                                                gint(),
                                                gint(),
                                                lam(
                                                    "r",
                                                    gint(),
                                                    iadd(var("r"), gneg_succ(c.clone())),
                                                ),
                                                idiff(succ(j.clone()), succ(k.clone())),
                                                idiff(j.clone(), k.clone()),
                                                diff_cancel(j.clone(), k.clone()),
                                            ),
                                        ),
                                        (
                                            idiff(j.clone(), nadd(k.clone(), succ(c.clone()))),
                                            apps(var("ih"), &[j.clone(), c.clone()]),
                                        ),
                                        (
                                            idiff(
                                                succ(j.clone()),
                                                succ(nadd(k.clone(), succ(c.clone()))),
                                            ),
                                            gsym(
                                                gint(),
                                                idiff(
                                                    succ(j.clone()),
                                                    succ(nadd(k.clone(), succ(c.clone()))),
                                                ),
                                                idiff(j.clone(), nadd(k.clone(), succ(c.clone()))),
                                                diff_cancel(
                                                    j.clone(),
                                                    nadd(k.clone(), succ(c.clone())),
                                                ),
                                            ),
                                        ),
                                        (
                                            idiff(
                                                succ(j.clone()),
                                                nadd(succ(k.clone()), succ(c.clone())),
                                            ),
                                            gcongr(
                                                nat(),
                                                gint(),
                                                lam("r", nat(), idiff(succ(j.clone()), var("r"))),
                                                succ(nadd(k.clone(), succ(c.clone()))),
                                                nadd(succ(k.clone()), succ(c.clone())),
                                                gsym(
                                                    nat(),
                                                    nadd(succ(k.clone()), succ(c.clone())),
                                                    succ(nadd(k.clone(), succ(c.clone()))),
                                                    apps(
                                                        g("Nat_add_succ_left"),
                                                        &[k.clone(), succ(c.clone())],
                                                    ),
                                                ),
                                            ),
                                        ),
                                    ],
                                ),
                            ),
                        ),
                        a.clone(),
                    ),
                ),
            ),
        ),
    );
    proofs.push((
        "IntLaw_subNatNat_add_negSucc",
        all3(nat(), ggoal(a.clone(), b.clone(), c.clone())),
        abs3(
            nat(),
            apps(
                nat_ind(
                    lam(
                        "n",
                        nat(),
                        pi(
                            "a",
                            nat(),
                            pi("c", nat(), ggoal(a.clone(), var("n"), c.clone())),
                        ),
                    ),
                    g0,
                    g_step,
                    b.clone(),
                ),
                &[a.clone(), c.clone()],
            ),
        ),
    ));
    proofs.push((
        "add_assoc",
        all3(gint(), add_assoc_goal(a.clone(), b.clone(), c.clone())),
        abs3(
            gint(),
            add_assoc_cases(0, [a, b, c], [false; 3], [zero.clone(), zero.clone(), zero]),
        ),
    ));
}
fn add_assoc_goal(a: Term, b: Term, c: Term) -> Term {
    eq_gint(
        iadd(iadd(a.clone(), b.clone()), c.clone()),
        iadd(a, iadd(b, c)),
    )
}
fn add_assoc_cases(
    depth: usize,
    values: [Term; 3],
    negative: [bool; 3],
    fields: [Term; 3],
) -> Term {
    if depth == 3 {
        return add_assoc_leaf(negative, fields);
    }
    let binder = ["ca", "cb", "cc"][depth];
    let field = ["na", "nb", "nc"][depth];
    let mut motive = values.clone();
    motive[depth] = var(binder);
    let branches: [Term; 2] = std::array::from_fn(|branch| {
        let mut values = values.clone();
        let mut negative = negative;
        let mut fields = fields.clone();
        negative[depth] = branch == 1;
        fields[depth] = var(field);
        values[depth] = if branch == 0 {
            gof(var(field))
        } else {
            gneg_succ(var(field))
        };
        lam(
            field,
            nat(),
            add_assoc_cases(depth + 1, values, negative, fields),
        )
    });
    gint_ind(
        lam(
            binder,
            gint(),
            add_assoc_goal(motive[0].clone(), motive[1].clone(), motive[2].clone()),
        ),
        branches[0].clone(),
        branches[1].clone(),
        values[depth].clone(),
    )
}
fn add_comm_proof(a: Term, b: Term) -> Term {
    apps(g("add_comm"), &[a, b])
}
// Base constructor shapes: positive,positive,any and negative,negative,positive.
fn add_assoc_base(signs: [bool; 3], fields: [Term; 3]) -> Term {
    let [m, n, k] = fields;
    match signs {
        [false, false, false] => gcongr(
            nat(),
            gint(),
            g("Int_ofNat"),
            nadd(nadd(m.clone(), n.clone()), k.clone()),
            nadd(m.clone(), nadd(n.clone(), k.clone())),
            apps(g("Nat_add_assoc"), &[m, n, k]),
        ),
        [false, false, true] => apps(g("IntLaw_subNatNat_add"), &[m, n, succ(k)]),
        [true, true, false] => {
            let a = gneg_succ(m.clone());
            let b = gneg_succ(n.clone());
            let c = gof(k.clone());
            let old = succ(succ(nadd(m.clone(), n.clone())));
            let new = nadd(succ(n.clone()), succ(m.clone()));
            let rearrange = law_chain(
                nat(),
                old.clone(),
                vec![
                    (
                        succ(succ(nadd(n.clone(), m.clone()))),
                        gcongr(
                            nat(),
                            nat(),
                            lam("r", nat(), succ(succ(var("r")))),
                            nadd(m.clone(), n.clone()),
                            nadd(n.clone(), m.clone()),
                            apps(g("Nat_add_comm"), &[m.clone(), n.clone()]),
                        ),
                    ),
                    (
                        new.clone(),
                        gsym(
                            nat(),
                            new.clone(),
                            succ(succ(nadd(n.clone(), m.clone()))),
                            gcongr(
                                nat(),
                                nat(),
                                g("Succ"),
                                nadd(succ(n.clone()), m.clone()),
                                succ(nadd(n.clone(), m.clone())),
                                apps(g("Nat_add_succ_left"), &[n.clone(), m.clone()]),
                            ),
                        ),
                    ),
                ],
            );
            let middle = iadd(idiff(k.clone(), succ(n.clone())), a.clone());
            law_chain(
                gint(),
                iadd(iadd(a.clone(), b.clone()), c.clone()),
                vec![
                    (
                        idiff(k.clone(), new.clone()),
                        gcongr(
                            nat(),
                            gint(),
                            lam("r", nat(), idiff(k.clone(), var("r"))),
                            old,
                            new.clone(),
                            rearrange,
                        ),
                    ),
                    (
                        middle.clone(),
                        gsym(
                            gint(),
                            middle.clone(),
                            idiff(k.clone(), new),
                            apps(
                                g("IntLaw_subNatNat_add_negSucc"),
                                &[k.clone(), succ(n.clone()), m],
                            ),
                        ),
                    ),
                    (
                        iadd(a.clone(), iadd(b, c)),
                        add_comm_proof(idiff(k.clone(), succ(n)), a),
                    ),
                ],
            )
        }
        _ => unreachable!(),
    }
}
fn add_assoc_leaf(signs: [bool; 3], fields: [Term; 3]) -> Term {
    let vals: [Term; 3] = std::array::from_fn(|i| {
        if signs[i] {
            gneg_succ(fields[i].clone())
        } else {
            gof(fields[i].clone())
        }
    });
    let [a, b, c] = vals;
    if matches!(
        signs,
        [false, false, false] | [false, false, true] | [true, true, false]
    ) {
        return add_assoc_base(signs, fields);
    }
    if matches!(signs, [true, false, false] | [false, true, true]) {
        // Reverse an already-proved association using only commutativity.
        let start = iadd(iadd(a.clone(), b.clone()), c.clone());
        return law_chain(
            gint(),
            start,
            vec![
                (
                    iadd(c.clone(), iadd(a.clone(), b.clone())),
                    add_comm_proof(iadd(a.clone(), b.clone()), c.clone()),
                ),
                (
                    iadd(c.clone(), iadd(b.clone(), a.clone())),
                    gcongr(
                        gint(),
                        gint(),
                        lam("r", gint(), iadd(c.clone(), var("r"))),
                        iadd(a.clone(), b.clone()),
                        iadd(b.clone(), a.clone()),
                        add_comm_proof(a.clone(), b.clone()),
                    ),
                ),
                (
                    iadd(iadd(c.clone(), b.clone()), a.clone()),
                    gsym(
                        gint(),
                        iadd(iadd(c.clone(), b.clone()), a.clone()),
                        iadd(c.clone(), iadd(b.clone(), a.clone())),
                        add_assoc_base(
                            [signs[2], signs[1], signs[0]],
                            [fields[2].clone(), fields[1].clone(), fields[0].clone()],
                        ),
                    ),
                ),
                (
                    iadd(iadd(b.clone(), c.clone()), a.clone()),
                    gcongr(
                        gint(),
                        gint(),
                        lam("r", gint(), iadd(var("r"), a.clone())),
                        iadd(c.clone(), b.clone()),
                        iadd(b.clone(), c.clone()),
                        add_comm_proof(c.clone(), b.clone()),
                    ),
                ),
                (
                    iadd(a.clone(), iadd(b.clone(), c.clone())),
                    add_comm_proof(iadd(b, c), a),
                ),
            ],
        );
    }
    if matches!(signs, [false, true, false] | [true, false, true]) {
        return law_chain(
            gint(),
            iadd(iadd(a.clone(), b.clone()), c.clone()),
            vec![
                (
                    iadd(c.clone(), iadd(a.clone(), b.clone())),
                    add_comm_proof(iadd(a.clone(), b.clone()), c.clone()),
                ),
                (
                    iadd(iadd(c.clone(), a.clone()), b.clone()),
                    gsym(
                        gint(),
                        iadd(iadd(c.clone(), a.clone()), b.clone()),
                        iadd(c.clone(), iadd(a.clone(), b.clone())),
                        add_assoc_base(
                            [signs[2], signs[0], signs[1]],
                            [fields[2].clone(), fields[0].clone(), fields[1].clone()],
                        ),
                    ),
                ),
                (
                    iadd(iadd(a.clone(), c.clone()), b.clone()),
                    gcongr(
                        gint(),
                        gint(),
                        lam("r", gint(), iadd(var("r"), b.clone())),
                        iadd(c.clone(), a.clone()),
                        iadd(a.clone(), c.clone()),
                        add_comm_proof(c.clone(), a.clone()),
                    ),
                ),
                (
                    iadd(a.clone(), iadd(c.clone(), b.clone())),
                    add_assoc_base(
                        [signs[0], signs[2], signs[1]],
                        [fields[0].clone(), fields[2].clone(), fields[1].clone()],
                    ),
                ),
                (
                    iadd(a.clone(), iadd(b.clone(), c.clone())),
                    gcongr(
                        gint(),
                        gint(),
                        lam("r", gint(), iadd(a.clone(), var("r"))),
                        iadd(c.clone(), b.clone()),
                        iadd(b.clone(), c.clone()),
                        add_comm_proof(c, b),
                    ),
                ),
            ],
        );
    }
    let [m, n, k] = fields;
    let inner = nadd(succ(nadd(m.clone(), n.clone())), k.clone());
    let end = nadd(m.clone(), succ(nadd(n.clone(), k.clone())));
    let proof = law_chain(
        nat(),
        inner.clone(),
        vec![
            (
                succ(nadd(nadd(m.clone(), n.clone()), k.clone())),
                apps(
                    g("Nat_add_succ_left"),
                    &[nadd(m.clone(), n.clone()), k.clone()],
                ),
            ),
            (
                end.clone(),
                gcongr(
                    nat(),
                    nat(),
                    g("Succ"),
                    nadd(nadd(m.clone(), n.clone()), k.clone()),
                    nadd(m.clone(), nadd(n.clone(), k.clone())),
                    apps(g("Nat_add_assoc"), &[m, n, k]),
                ),
            ),
        ],
    );
    gcongr(
        nat(),
        gint(),
        lam("r", nat(), gneg_succ(succ(var("r")))),
        inner,
        end,
        proof,
    )
}

fn nmul(a: Term, b: Term) -> Term {
    apps(g("Nat_mul"), &[a, b])
}
fn imul(a: Term, b: Term) -> Term {
    apps(g("mul"), &[a, b])
}
fn neg_nat(a: Term) -> Term {
    app(g("Int_negOfNat"), a)
}
fn diff_zero_left(a: Term) -> Term {
    app(g("IntLaw_subNatNat_zero_left"), a)
}
fn signed_sum(negative: bool, a: Term, b: Term) -> Term {
    if negative {
        apps(g("IntLaw_add_negOfNat_negOfNat"), &[a, b])
    } else {
        refl_gint(gof(nadd(a, b)))
    }
}
fn mixed_sum(negative_left: bool, a: Term, b: Term) -> Term {
    if !negative_left {
        return apps(g("IntLaw_add_ofNat_negOfNat"), &[a, b]);
    }
    law_chain(
        gint(),
        iadd(neg_nat(a.clone()), gof(b.clone())),
        vec![
            (
                iadd(gof(b.clone()), neg_nat(a.clone())),
                add_comm_proof(neg_nat(a.clone()), gof(b.clone())),
            ),
            (
                idiff(b.clone(), a.clone()),
                apps(g("IntLaw_add_ofNat_negOfNat"), &[b, a]),
            ),
        ],
    )
}
fn append_integer_distributivity(proofs: &mut Vec<Proof>) {
    let (a, b, c, k, j) = (var("a"), var("b"), var("c"), var("k"), var("j"));
    let z = g("Zero");
    let common = |x: Term, y: Term, t: Term| {
        eq_gint(
            idiff(nadd(x.clone(), t.clone()), nadd(y.clone(), t)),
            idiff(x, y),
        )
    };
    proofs.push((
        "IntLaw_subNatNat_add_add",
        all3(nat(), common(a.clone(), b.clone(), c.clone())),
        abs3(
            nat(),
            nat_ind(
                lam("n", nat(), common(a.clone(), b.clone(), var("n"))),
                refl_gint(idiff(a.clone(), b.clone())),
                lam(
                    "k",
                    nat(),
                    lam(
                        "ih",
                        common(a.clone(), b.clone(), k.clone()),
                        law_chain(
                            gint(),
                            idiff(
                                nadd(a.clone(), succ(k.clone())),
                                nadd(b.clone(), succ(k.clone())),
                            ),
                            vec![
                                (
                                    idiff(nadd(a.clone(), k.clone()), nadd(b.clone(), k.clone())),
                                    diff_cancel(
                                        nadd(a.clone(), k.clone()),
                                        nadd(b.clone(), k.clone()),
                                    ),
                                ),
                                (idiff(a.clone(), b.clone()), var("ih")),
                            ],
                        ),
                    ),
                ),
                c.clone(),
            ),
        ),
    ));
    let zero_left = |x: Term| eq_gint(idiff(z.clone(), x.clone()), neg_nat(x));
    proofs.push((
        "IntLaw_subNatNat_zero_left",
        pi("a", nat(), zero_left(a.clone())),
        lam(
            "a",
            nat(),
            nat_ind(
                lam("n", nat(), zero_left(var("n"))),
                refl_gint(gof(z.clone())),
                lam(
                    "k",
                    nat(),
                    lam("ih", zero_left(k.clone()), refl_gint(gneg_succ(k.clone()))),
                ),
                a.clone(),
            ),
        ),
    ));
    let mixed = |x: Term, y: Term| eq_gint(iadd(gof(x.clone()), neg_nat(y.clone())), idiff(x, y));
    proofs.push((
        "IntLaw_add_ofNat_negOfNat",
        all2(nat(), mixed(a.clone(), b.clone())),
        abs2(
            nat(),
            nat_ind(
                lam("n", nat(), mixed(a.clone(), var("n"))),
                gsym(
                    gint(),
                    idiff(a.clone(), z.clone()),
                    gof(a.clone()),
                    diff_zero_right(a.clone()),
                ),
                lam(
                    "k",
                    nat(),
                    lam(
                        "ih",
                        mixed(a.clone(), k.clone()),
                        refl_gint(idiff(a.clone(), succ(k.clone()))),
                    ),
                ),
                b.clone(),
            ),
        ),
    ));
    let negative_sum = |x: Term, y: Term| {
        eq_gint(
            iadd(neg_nat(x.clone()), neg_nat(y.clone())),
            neg_nat(nadd(x, y)),
        )
    };
    let ns_zero = lam(
        "b",
        nat(),
        law_chain(
            gint(),
            iadd(gof(z.clone()), neg_nat(b.clone())),
            vec![
                (
                    iadd(neg_nat(b.clone()), gof(z.clone())),
                    add_comm_proof(gof(z.clone()), neg_nat(b.clone())),
                ),
                (neg_nat(b.clone()), app(g("add_zero"), neg_nat(b.clone()))),
                (
                    neg_nat(nadd(z.clone(), b.clone())),
                    gcongr(
                        nat(),
                        gint(),
                        g("Int_negOfNat"),
                        b.clone(),
                        nadd(z.clone(), b.clone()),
                        gsym(
                            nat(),
                            nadd(z.clone(), b.clone()),
                            b.clone(),
                            app(g("Nat_zero_add"), b.clone()),
                        ),
                    ),
                ),
            ],
        ),
    );
    let ns_step = lam(
        "k",
        nat(),
        lam(
            "ih",
            pi("b", nat(), negative_sum(k.clone(), b.clone())),
            lam(
                "b",
                nat(),
                nat_ind(
                    lam("n", nat(), negative_sum(succ(k.clone()), var("n"))),
                    app(g("add_zero"), gneg_succ(k.clone())),
                    lam(
                        "j",
                        nat(),
                        lam(
                            "unused",
                            negative_sum(succ(k.clone()), j.clone()),
                            gcongr(
                                nat(),
                                gint(),
                                g("Int_negSucc"),
                                succ(nadd(k.clone(), j.clone())),
                                nadd(succ(k.clone()), j.clone()),
                                gsym(
                                    nat(),
                                    nadd(succ(k.clone()), j.clone()),
                                    succ(nadd(k.clone(), j.clone())),
                                    apps(g("Nat_add_succ_left"), &[k.clone(), j.clone()]),
                                ),
                            ),
                        ),
                    ),
                    b.clone(),
                ),
            ),
        ),
    );
    proofs.push((
        "IntLaw_add_negOfNat_negOfNat",
        all2(nat(), negative_sum(a.clone(), b.clone())),
        lam(
            "a",
            nat(),
            nat_ind(
                lam(
                    "n",
                    nat(),
                    pi("b", nat(), negative_sum(var("n"), b.clone())),
                ),
                ns_zero,
                ns_step,
                a.clone(),
            ),
        ),
    ));

    for negative in [false, true] {
        let name = if negative {
            "IntLaw_mul_negOfNat_subNatNat"
        } else {
            "IntLaw_mul_ofNat_subNatNat"
        };
        let result = |x: Term, y: Term, t: Term| {
            let p = nmul(x.clone(), y);
            let n = nmul(x, t);
            if negative {
                idiff(n, p)
            } else {
                idiff(p, n)
            }
        };
        let goal = |x: Term, y: Term, t: Term| {
            eq_gint(
                imul(
                    signed_natural(negative, x.clone()),
                    idiff(y.clone(), t.clone()),
                ),
                result(x, y, t),
            )
        };
        let ab = nmul(a.clone(), b.clone());
        let base = abs2(
            nat(),
            law_chain(
                gint(),
                imul(
                    signed_natural(negative, a.clone()),
                    idiff(b.clone(), z.clone()),
                ),
                vec![
                    (
                        imul(signed_natural(negative, a.clone()), gof(b.clone())),
                        gcongr(
                            gint(),
                            gint(),
                            lam(
                                "r",
                                gint(),
                                imul(signed_natural(negative, a.clone()), var("r")),
                            ),
                            idiff(b.clone(), z.clone()),
                            gof(b.clone()),
                            diff_zero_right(b.clone()),
                        ),
                    ),
                    (
                        signed_natural(negative, ab.clone()),
                        signed_product(negative, false, a.clone(), b.clone()),
                    ),
                    (
                        result(a.clone(), b.clone(), z.clone()),
                        if negative {
                            gsym(
                                gint(),
                                idiff(z.clone(), ab.clone()),
                                neg_nat(ab.clone()),
                                diff_zero_left(ab.clone()),
                            )
                        } else {
                            gsym(
                                gint(),
                                idiff(ab.clone(), z.clone()),
                                gof(ab.clone()),
                                diff_zero_right(ab.clone()),
                            )
                        },
                    ),
                ],
            ),
        );
        let ak = nmul(a.clone(), succ(k.clone()));
        let left0 = imul(
            signed_natural(negative, a.clone()),
            idiff(z.clone(), succ(k.clone())),
        );
        let step0 = law_chain(
            gint(),
            left0,
            vec![
                (
                    signed_natural(!negative, ak.clone()),
                    signed_product(negative, true, a.clone(), succ(k.clone())),
                ),
                (
                    result(a.clone(), z.clone(), succ(k.clone())),
                    if negative {
                        gsym(
                            gint(),
                            idiff(ak.clone(), z.clone()),
                            gof(ak.clone()),
                            diff_zero_right(ak.clone()),
                        )
                    } else {
                        gsym(
                            gint(),
                            idiff(z.clone(), ak.clone()),
                            neg_nat(ak.clone()),
                            diff_zero_left(ak.clone()),
                        )
                    },
                ),
            ],
        );
        let aj = nmul(a.clone(), j.clone());
        let akk = nmul(a.clone(), k.clone());
        let (p, n) = if negative {
            (akk.clone(), aj.clone())
        } else {
            (aj.clone(), akk.clone())
        };
        let step_succ = lam(
            "j",
            nat(),
            lam(
                "unused",
                goal(a.clone(), j.clone(), succ(k.clone())),
                law_chain(
                    gint(),
                    imul(
                        signed_natural(negative, a.clone()),
                        idiff(succ(j.clone()), succ(k.clone())),
                    ),
                    vec![
                        (
                            imul(
                                signed_natural(negative, a.clone()),
                                idiff(j.clone(), k.clone()),
                            ),
                            gcongr(
                                gint(),
                                gint(),
                                lam(
                                    "r",
                                    gint(),
                                    imul(signed_natural(negative, a.clone()), var("r")),
                                ),
                                idiff(succ(j.clone()), succ(k.clone())),
                                idiff(j.clone(), k.clone()),
                                diff_cancel(j.clone(), k.clone()),
                            ),
                        ),
                        (
                            result(a.clone(), j.clone(), k.clone()),
                            apps(var("ih"), &[a.clone(), j.clone()]),
                        ),
                        (
                            result(a.clone(), succ(j.clone()), succ(k.clone())),
                            gsym(
                                gint(),
                                idiff(nadd(p.clone(), a.clone()), nadd(n.clone(), a.clone())),
                                idiff(p.clone(), n.clone()),
                                apps(g("IntLaw_subNatNat_add_add"), &[p, n, a.clone()]),
                            ),
                        ),
                    ],
                ),
            ),
        );
        let step = lam(
            "k",
            nat(),
            lam(
                "ih",
                all2(nat(), goal(a.clone(), b.clone(), k.clone())),
                abs2(
                    nat(),
                    nat_ind(
                        lam("n", nat(), goal(a.clone(), var("n"), succ(k.clone()))),
                        step0,
                        step_succ,
                        b.clone(),
                    ),
                ),
            ),
        );
        proofs.push((
            name,
            all3(nat(), goal(a.clone(), b.clone(), c.clone())),
            abs3(
                nat(),
                apps(
                    nat_ind(
                        lam(
                            "n",
                            nat(),
                            all2(nat(), goal(a.clone(), b.clone(), var("n"))),
                        ),
                        base,
                        step,
                        c.clone(),
                    ),
                    &[a.clone(), b.clone()],
                ),
            ),
        ));
    }
    proofs.push((
        "mul_distrib_add",
        all3(
            gint(),
            integer_distrib_goal(a.clone(), b.clone(), c.clone()),
        ),
        abs3(
            gint(),
            integer_distrib_cases(0, [a, b, c], [false; 3], [z.clone(), z.clone(), z]),
        ),
    ));
}
fn integer_distrib_goal(a: Term, b: Term, c: Term) -> Term {
    eq_gint(
        imul(a.clone(), iadd(b.clone(), c.clone())),
        iadd(imul(a.clone(), b), imul(a, c)),
    )
}
fn integer_distrib_cases(
    depth: usize,
    values: [Term; 3],
    signs: [bool; 3],
    magnitudes: [Term; 3],
) -> Term {
    if depth == 3 {
        return integer_distrib_leaf(signs, magnitudes);
    }
    let binder = ["da", "db", "dc"][depth];
    let field = ["ma", "mb", "mc"][depth];
    let mut mv = values.clone();
    mv[depth] = var(binder);
    let branches: [Term; 2] = std::array::from_fn(|branch| {
        let mut values = values.clone();
        let mut signs = signs;
        let mut mags = magnitudes.clone();
        signs[depth] = branch == 1;
        mags[depth] = if branch == 0 {
            var(field)
        } else {
            succ(var(field))
        };
        values[depth] = signed_natural(signs[depth], mags[depth].clone());
        lam(
            field,
            nat(),
            integer_distrib_cases(depth + 1, values, signs, mags),
        )
    });
    gint_ind(
        lam(
            binder,
            gint(),
            integer_distrib_goal(mv[0].clone(), mv[1].clone(), mv[2].clone()),
        ),
        branches[0].clone(),
        branches[1].clone(),
        values[depth].clone(),
    )
}
fn integer_distrib_leaf(signs: [bool; 3], magnitudes: [Term; 3]) -> Term {
    let [sa, sb, sc] = signs;
    let [a, b, c] = magnitudes;
    let ia = signed_natural(sa, a.clone());
    let ib = signed_natural(sb, b.clone());
    let ic = signed_natural(sc, c.clone());
    let ab = nmul(a.clone(), b.clone());
    let ac = nmul(a.clone(), c.clone());
    let pab = signed_natural(sa ^ sb, ab.clone());
    let pac = signed_natural(sa ^ sc, ac.clone());
    let left = imul(ia.clone(), iadd(ib.clone(), ic.clone()));
    let right = iadd(imul(ia.clone(), ib.clone()), imul(ia.clone(), ic.clone()));
    let right_mid = iadd(pab.clone(), imul(ia.clone(), ic.clone()));
    let right_products = iadd(pab.clone(), pac.clone());
    let mut right_steps = vec![
        (
            right_mid,
            gcongr(
                gint(),
                gint(),
                lam("r", gint(), iadd(var("r"), imul(ia.clone(), ic.clone()))),
                imul(ia.clone(), ib.clone()),
                pab.clone(),
                signed_product(sa, sb, a.clone(), b.clone()),
            ),
        ),
        (
            right_products.clone(),
            gcongr(
                gint(),
                gint(),
                lam("r", gint(), iadd(pab.clone(), var("r"))),
                imul(ia.clone(), ic.clone()),
                pac.clone(),
                signed_product(sa, sc, a.clone(), c.clone()),
            ),
        ),
    ];
    let (normal, left_proof) = if sb == sc {
        let sum = signed_natural(sb, nadd(b.clone(), c.clone()));
        let natural_product = nmul(a.clone(), nadd(b.clone(), c.clone()));
        let normal = signed_natural(sa ^ sb, nadd(ab.clone(), ac.clone()));
        let proof = law_chain(
            gint(),
            left.clone(),
            vec![
                (
                    imul(ia.clone(), sum.clone()),
                    gcongr(
                        gint(),
                        gint(),
                        lam("r", gint(), imul(ia.clone(), var("r"))),
                        iadd(ib.clone(), ic.clone()),
                        sum,
                        signed_sum(sb, b.clone(), c.clone()),
                    ),
                ),
                (
                    signed_natural(sa ^ sb, natural_product.clone()),
                    signed_product(sa, sb, a.clone(), nadd(b.clone(), c.clone())),
                ),
                (
                    normal.clone(),
                    gcongr(
                        nat(),
                        gint(),
                        g(if sa ^ sb { "Int_negOfNat" } else { "Int_ofNat" }),
                        natural_product,
                        nadd(ab.clone(), ac.clone()),
                        apps(g("Nat_mul_add"), &[a.clone(), b.clone(), c.clone()]),
                    ),
                ),
            ],
        );
        right_steps.push((normal.clone(), signed_sum(sa ^ sb, ab.clone(), ac.clone())));
        (normal, proof)
    } else {
        let (positive, negative) = if sb {
            (c.clone(), b.clone())
        } else {
            (b.clone(), c.clone())
        };
        let sum = idiff(positive.clone(), negative.clone());
        let np = nmul(a.clone(), positive.clone());
        let nn = nmul(a.clone(), negative.clone());
        let normal = if sa { idiff(nn, np) } else { idiff(np, nn) };
        let proof = law_chain(
            gint(),
            left.clone(),
            vec![
                (
                    imul(ia.clone(), sum.clone()),
                    gcongr(
                        gint(),
                        gint(),
                        lam("r", gint(), imul(ia.clone(), var("r"))),
                        iadd(ib, ic),
                        sum,
                        mixed_sum(sb, b, c),
                    ),
                ),
                (
                    normal.clone(),
                    apps(
                        g(if sa {
                            "IntLaw_mul_negOfNat_subNatNat"
                        } else {
                            "IntLaw_mul_ofNat_subNatNat"
                        }),
                        &[a, positive, negative],
                    ),
                ),
            ],
        );
        right_steps.push((normal.clone(), mixed_sum(sa ^ sb, ab, ac)));
        (normal, proof)
    };
    let right_proof = law_chain(gint(), right.clone(), right_steps);
    gtrans(
        gint(),
        left,
        normal.clone(),
        right.clone(),
        left_proof,
        gsym(gint(), right, normal, right_proof),
    )
}

// Negation is multiplication by negative one because the existing signed
// products reduce to the same magnitude; no subtraction/ring oracle is used.
fn append_integer_subtraction(proofs: &mut Vec<Proof>) {
    let (a, b, n) = (var("a"), var("b"), var("n"));
    let one = succ(g("Zero"));
    let minus_one = neg_nat(one.clone());
    let goal = |x: Term| eq_gint(app(g("Int_neg"), x.clone()), imul(minus_one.clone(), x));
    let branch = |negative: bool| {
        let magnitude = if negative { succ(n.clone()) } else { n.clone() };
        let input = signed_natural(negative, magnitude.clone());
        let output = signed_natural(!negative, magnitude.clone());
        let natural_product = nmul(one.clone(), magnitude.clone());
        let product = imul(minus_one.clone(), input);
        let magnitude_proof = law_chain(
            nat(),
            natural_product.clone(),
            vec![
                (
                    nmul(magnitude.clone(), one.clone()),
                    apps(g("Nat_mul_comm"), &[one.clone(), magnitude.clone()]),
                ),
                (magnitude.clone(), app(g("Nat_mul_one"), magnitude.clone())),
            ],
        );
        let product_proof = law_chain(
            gint(),
            product.clone(),
            vec![
                (
                    signed_natural(!negative, natural_product.clone()),
                    signed_product(true, negative, one.clone(), magnitude.clone()),
                ),
                (
                    output.clone(),
                    gcongr(
                        nat(),
                        gint(),
                        g(if negative {
                            "Int_ofNat"
                        } else {
                            "Int_negOfNat"
                        }),
                        natural_product,
                        magnitude,
                        magnitude_proof,
                    ),
                ),
            ],
        );
        lam("n", nat(), gsym(gint(), product, output, product_proof))
    };
    proofs.push((
        "IntLaw_neg_eq_mul_neg_one",
        pi("b", gint(), goal(b.clone())),
        lam(
            "b",
            gint(),
            gint_ind(
                lam("x", gint(), goal(var("x"))),
                branch(false),
                branch(true),
                b.clone(),
            ),
        ),
    ));
    proofs.push((
        "sub_elim",
        all2(
            gint(),
            eq_gint(
                apps(g("sub"), &[a.clone(), b.clone()]),
                iadd(a.clone(), imul(minus_one.clone(), b.clone())),
            ),
        ),
        abs2(
            gint(),
            gcongr(
                gint(),
                gint(),
                lam("r", gint(), iadd(a.clone(), var("r"))),
                app(g("Int_neg"), b.clone()),
                imul(minus_one, b.clone()),
                app(g("IntLaw_neg_eq_mul_neg_one"), b),
            ),
        ),
    ));
}

fn order_fact(operation: &str, a: Term, b: Term) -> Term {
    eq_at(g("Bool"), apps(g(operation), &[a, b]), g("true"))
}
fn order_trans_tail(operation: &str, a: Term, b: Term, c: Term) -> Term {
    pi(
        "ab",
        order_fact(operation, a.clone(), b.clone()),
        pi(
            "bc",
            order_fact(operation, b, c.clone()),
            order_fact(operation, a, c),
        ),
    )
}
fn order_trans_at(operation: &str, carrier: Term, a: Term) -> Term {
    pi(
        "b",
        carrier.clone(),
        pi(
            "c",
            carrier,
            order_trans_tail(operation, a, var("b"), var("c")),
        ),
    )
}
fn order_evidence_lambdas(operation: &str, a: Term, b: Term, c: Term, proof: Term) -> Term {
    lam(
        "ab",
        order_fact(operation, a, b.clone()),
        lam("bc", order_fact(operation, b, c), proof),
    )
}
fn impossible_boolean_order(goal: Term, evidence: Term) -> Term {
    apps(
        g("Bool_noConfusion"),
        &[goal, g("false"), g("true"), evidence],
    )
}
fn order_match(
    discriminant: Term,
    parameter: &str,
    carrier: Term,
    result: Term,
    cases: Vec<Term>,
) -> Term {
    Term::Match {
        discriminant: Box::new(discriminant),
        motive: Box::new(lam(parameter, carrier, result)),
        cases,
    }
}

fn append_integer_order(proofs: &mut Vec<Proof>) {
    let nrel = |a: Term, b: Term| order_fact("natLe", a, b);
    let rel = |a: Term, b: Term| order_fact("le", a, b);
    let truth = || refl_at(g("Bool"), g("true"));

    proofs.push((
        "Nat_le_refl",
        pi("a", nat(), nrel(var("a"), var("a"))),
        lam(
            "a",
            nat(),
            nat_ind(
                lam("n", nat(), nrel(var("n"), var("n"))),
                truth(),
                lam("n", nat(), lam("ih", nrel(var("n"), var("n")), var("ih"))),
                var("a"),
            ),
        ),
    ));

    let first = succ(var("m"));
    let second = succ(var("n"));
    let third = succ(var("k"));
    let c_zero = order_evidence_lambdas(
        "natLe",
        first.clone(),
        second.clone(),
        g("Zero"),
        impossible_boolean_order(nrel(first.clone(), g("Zero")), var("bc")),
    );
    let c_succ = lam(
        "k",
        nat(),
        order_evidence_lambdas(
            "natLe",
            first.clone(),
            second.clone(),
            third,
            apps(var("ih"), &[var("n"), var("k"), var("ab"), var("bc")]),
        ),
    );
    let b_succ = lam(
        "n",
        nat(),
        lam(
            "c",
            nat(),
            order_match(
                var("c"),
                "cc",
                nat(),
                order_trans_tail("natLe", first.clone(), second, var("cc")),
                vec![c_zero, c_succ],
            ),
        ),
    );
    let b_zero = lam(
        "c",
        nat(),
        order_evidence_lambdas(
            "natLe",
            first.clone(),
            g("Zero"),
            var("c"),
            impossible_boolean_order(nrel(first.clone(), var("c")), var("ab")),
        ),
    );
    let step = lam(
        "m",
        nat(),
        lam(
            "ih",
            order_trans_at("natLe", nat(), var("m")),
            lam(
                "b",
                nat(),
                order_match(
                    var("b"),
                    "bb",
                    nat(),
                    pi(
                        "c",
                        nat(),
                        order_trans_tail("natLe", first, var("bb"), var("c")),
                    ),
                    vec![b_zero, b_succ],
                ),
            ),
        ),
    );
    let base = lam(
        "b",
        nat(),
        lam(
            "c",
            nat(),
            order_evidence_lambdas("natLe", g("Zero"), var("b"), var("c"), truth()),
        ),
    );
    proofs.push((
        "Nat_le_trans",
        pi("a", nat(), order_trans_at("natLe", nat(), var("a"))),
        lam(
            "a",
            nat(),
            nat_ind(
                lam("aa", nat(), order_trans_at("natLe", nat(), var("aa"))),
                base,
                step,
                var("a"),
            ),
        ),
    ));

    proofs.push((
        "le_refl",
        pi("a", gint(), rel(var("a"), var("a"))),
        lam(
            "a",
            gint(),
            gint_ind(
                lam("aa", gint(), rel(var("aa"), var("aa"))),
                lam("m", nat(), app(g("Nat_le_refl"), var("m"))),
                lam("m", nat(), app(g("Nat_le_refl"), var("m"))),
                var("a"),
            ),
        ),
    ));

    let signed = |positive: bool, n: Term| if positive { gof(n) } else { gneg_succ(n) };
    let first_cases = [true, false]
        .into_iter()
        .map(|ap| {
            let a = signed(ap, var("m"));
            let second_cases = [true, false]
                .into_iter()
                .map(|bp| {
                    let b = signed(bp, var("n"));
                    let third_cases = [true, false]
                        .into_iter()
                        .map(|cp| {
                            let c = signed(cp, var("k"));
                            let goal = rel(a.clone(), c.clone());
                            let proof = if ap && !bp {
                                impossible_boolean_order(goal, var("ab"))
                            } else if bp && !cp {
                                impossible_boolean_order(goal, var("bc"))
                            } else if ap && bp && cp {
                                apps(
                                    g("Nat_le_trans"),
                                    &[var("m"), var("n"), var("k"), var("ab"), var("bc")],
                                )
                            } else if !ap && !bp && !cp {
                                apps(
                                    g("Nat_le_trans"),
                                    &[var("k"), var("n"), var("m"), var("bc"), var("ab")],
                                )
                            } else {
                                truth()
                            };
                            lam(
                                "k",
                                nat(),
                                order_evidence_lambdas("le", a.clone(), b.clone(), c, proof),
                            )
                        })
                        .collect();
                    lam(
                        "n",
                        nat(),
                        lam(
                            "c",
                            gint(),
                            order_match(
                                var("c"),
                                "cc",
                                gint(),
                                order_trans_tail("le", a.clone(), b, var("cc")),
                                third_cases,
                            ),
                        ),
                    )
                })
                .collect();
            lam(
                "m",
                nat(),
                lam(
                    "b",
                    gint(),
                    order_match(
                        var("b"),
                        "bb",
                        gint(),
                        pi("c", gint(), order_trans_tail("le", a, var("bb"), var("c"))),
                        second_cases,
                    ),
                ),
            )
        })
        .collect();
    proofs.push((
        "le_trans",
        pi("a", gint(), order_trans_at("le", gint(), var("a"))),
        lam(
            "a",
            gint(),
            order_match(
                var("a"),
                "aa",
                gint(),
                order_trans_at("le", gint(), var("aa")),
                first_cases,
            ),
        ),
    ));
}

fn order_totality(operation: &str, a: Term, b: Term) -> Term {
    apps(
        g("Or"),
        &[
            order_fact(operation, a.clone(), b.clone()),
            order_fact(operation, b, a),
        ],
    )
}
fn order_totality_witness(operation: &str, a: Term, b: Term, left: bool) -> Term {
    apps(
        g(if left { "left" } else { "right" }),
        &[
            order_fact(operation, a.clone(), b.clone()),
            order_fact(operation, b, a),
            refl_at(g("Bool"), g("true")),
        ],
    )
}
fn append_integer_totality(proofs: &mut Vec<Proof>) {
    let natural_statement = |a: Term| pi("b", nat(), order_totality("natLe", a, var("b")));
    let base = lam(
        "b",
        nat(),
        order_totality_witness("natLe", g("Zero"), var("b"), true),
    );
    let step = lam(
        "m",
        nat(),
        lam(
            "ih",
            natural_statement(var("m")),
            lam(
                "b",
                nat(),
                order_match(
                    var("b"),
                    "bb",
                    nat(),
                    order_totality("natLe", succ(var("m")), var("bb")),
                    vec![
                        order_totality_witness("natLe", succ(var("m")), g("Zero"), false),
                        lam("n", nat(), app(var("ih"), var("n"))),
                    ],
                ),
            ),
        ),
    );
    proofs.push((
        "Nat_le_total",
        pi("a", nat(), natural_statement(var("a"))),
        lam(
            "a",
            nat(),
            nat_ind(
                lam("aa", nat(), natural_statement(var("aa"))),
                base,
                step,
                var("a"),
            ),
        ),
    ));

    let signed = |positive: bool, n: Term| if positive { gof(n) } else { gneg_succ(n) };
    let cases = [true, false]
        .into_iter()
        .map(|ap| {
            let a = signed(ap, var("m"));
            let inner = [true, false]
                .into_iter()
                .map(|bp| {
                    let b = signed(bp, var("n"));
                    let witness = match (ap, bp) {
                        (true, true) => apps(g("Nat_le_total"), &[var("m"), var("n")]),
                        (false, false) => apps(g("Nat_le_total"), &[var("n"), var("m")]),
                        (true, false) => order_totality_witness("le", a.clone(), b, false),
                        (false, true) => order_totality_witness("le", a.clone(), b, true),
                    };
                    lam("n", nat(), witness)
                })
                .collect();
            lam(
                "m",
                nat(),
                lam(
                    "b",
                    gint(),
                    order_match(
                        var("b"),
                        "bb",
                        gint(),
                        order_totality("le", a, var("bb")),
                        inner,
                    ),
                ),
            )
        })
        .collect();
    proofs.push((
        "le_total",
        pi(
            "a",
            gint(),
            pi("b", gint(), order_totality("le", var("a"), var("b"))),
        ),
        lam(
            "a",
            gint(),
            order_match(
                var("a"),
                "aa",
                gint(),
                pi("b", gint(), order_totality("le", var("aa"), var("b"))),
                cases,
            ),
        ),
    ));
}

fn increment_int(a: Term) -> Term {
    iadd(a, gone())
}
fn increment_comparison(a: Term, b: Term) -> Term {
    eq_at(
        g("Bool"),
        apps(
            g("le"),
            &[increment_int(a.clone()), increment_int(b.clone())],
        ),
        apps(g("le"), &[a, b]),
    )
}
fn signed_constructor(positive: bool, n: Term) -> Term {
    if positive {
        gof(n)
    } else {
        gneg_succ(n)
    }
}
fn incremented_constructor(positive: bool, n: Term) -> Term {
    if positive {
        gof(succ(n))
    } else {
        neg_nat(n)
    }
}
fn normalized_increment_comparison(ap: bool, bp: bool, m: Term, n: Term) -> Term {
    eq_at(
        g("Bool"),
        apps(
            g("le"),
            &[
                incremented_constructor(ap, m.clone()),
                incremented_constructor(bp, n.clone()),
            ],
        ),
        apps(
            g("le"),
            &[signed_constructor(ap, m), signed_constructor(bp, n)],
        ),
    )
}
fn increment_boundary_proof(ap: bool, bp: bool) -> Term {
    let yes = || refl_at(g("Bool"), g("true"));
    let no = || refl_at(g("Bool"), g("false"));
    match (ap, bp) {
        (true, true) => refl_at(g("Bool"), apps(g("natLe"), &[var("m"), var("n")])),
        (true, false) => order_match(
            var("n"),
            "ni",
            nat(),
            normalized_increment_comparison(ap, bp, var("m"), var("ni")),
            vec![no(), lam("k", nat(), no())],
        ),
        (false, true) => order_match(
            var("m"),
            "mi",
            nat(),
            normalized_increment_comparison(ap, bp, var("mi"), var("n")),
            vec![yes(), lam("j", nat(), yes())],
        ),
        (false, false) => order_match(
            var("m"),
            "mi",
            nat(),
            normalized_increment_comparison(ap, bp, var("mi"), var("n")),
            vec![
                order_match(
                    var("n"),
                    "ni",
                    nat(),
                    normalized_increment_comparison(ap, bp, g("Zero"), var("ni")),
                    vec![yes(), lam("k", nat(), no())],
                ),
                lam(
                    "j",
                    nat(),
                    order_match(
                        var("n"),
                        "ni",
                        nat(),
                        normalized_increment_comparison(ap, bp, succ(var("j")), var("ni")),
                        vec![
                            yes(),
                            lam(
                                "k",
                                nat(),
                                refl_at(g("Bool"), apps(g("natLe"), &[var("k"), var("j")])),
                            ),
                        ],
                    ),
                ),
            ],
        ),
    }
}
fn append_integer_successor_order(proofs: &mut Vec<Proof>) {
    let n = var("n");
    let start = increment_int(gneg_succ(n.clone()));
    proofs.push((
        "IntLaw_add_one_negSucc",
        pi("n", nat(), eq_gint(start.clone(), neg_nat(n.clone()))),
        lam(
            "n",
            nat(),
            law_chain(
                gint(),
                start,
                vec![
                    (
                        idiff(g("Zero"), n.clone()),
                        diff_cancel(g("Zero"), n.clone()),
                    ),
                    (neg_nat(n.clone()), diff_zero_left(n)),
                ],
            ),
        ),
    ));

    let cases = [true, false]
        .into_iter()
        .map(|ap| {
            let a = signed_constructor(ap, var("m"));
            let first = increment_int(a.clone());
            let first_reduced = incremented_constructor(ap, var("m"));
            let first_proof = if ap {
                refl_gint(first_reduced.clone())
            } else {
                app(g("IntLaw_add_one_negSucc"), var("m"))
            };
            let inner = [true, false]
                .into_iter()
                .map(|bp| {
                    let b = signed_constructor(bp, var("n"));
                    let second = increment_int(b.clone());
                    let second_reduced = incremented_constructor(bp, var("n"));
                    let second_proof = if bp {
                        refl_gint(second_reduced.clone())
                    } else {
                        app(g("IntLaw_add_one_negSucc"), var("n"))
                    };
                    let left = apps(g("le"), &[first.clone(), second.clone()]);
                    let mid = apps(g("le"), &[first_reduced.clone(), second.clone()]);
                    let reduced = apps(g("le"), &[first_reduced.clone(), second_reduced.clone()]);
                    let right = apps(g("le"), &[a.clone(), b]);
                    lam(
                        "n",
                        nat(),
                        law_chain(
                            g("Bool"),
                            left,
                            vec![
                                (
                                    mid,
                                    gcongr(
                                        gint(),
                                        g("Bool"),
                                        lam(
                                            "x",
                                            gint(),
                                            apps(g("le"), &[var("x"), second.clone()]),
                                        ),
                                        first.clone(),
                                        first_reduced.clone(),
                                        first_proof.clone(),
                                    ),
                                ),
                                (
                                    reduced,
                                    gcongr(
                                        gint(),
                                        g("Bool"),
                                        lam(
                                            "y",
                                            gint(),
                                            apps(g("le"), &[first_reduced.clone(), var("y")]),
                                        ),
                                        second,
                                        second_reduced,
                                        second_proof,
                                    ),
                                ),
                                (right, increment_boundary_proof(ap, bp)),
                            ],
                        ),
                    )
                })
                .collect();
            lam(
                "m",
                nat(),
                lam(
                    "b",
                    gint(),
                    order_match(
                        var("b"),
                        "bb",
                        gint(),
                        increment_comparison(a, var("bb")),
                        inner,
                    ),
                ),
            )
        })
        .collect();
    proofs.push((
        "IntLaw_le_add_one",
        pi(
            "a",
            gint(),
            pi("b", gint(), increment_comparison(var("a"), var("b"))),
        ),
        lam(
            "a",
            gint(),
            order_match(
                var("a"),
                "aa",
                gint(),
                pi("b", gint(), increment_comparison(var("aa"), var("b"))),
                cases,
            ),
        ),
    ));

    let a = var("a");
    let b = var("b");
    let premise = order_fact("lt", a.clone(), increment_int(b.clone()));
    let left = apps(
        g("le"),
        &[increment_int(a.clone()), increment_int(b.clone())],
    );
    let right = apps(g("le"), &[a.clone(), b.clone()]);
    let proof = apps(
        g("Eq_rec_dep"),
        &[
            g("Bool"),
            left.clone(),
            lam(
                "out",
                g("Bool"),
                lam(
                    "eq",
                    eq_at(g("Bool"), left, var("out")),
                    eq_at(g("Bool"), var("out"), g("true")),
                ),
            ),
            var("h"),
            right,
            apps(g("IntLaw_le_add_one"), &[a.clone(), b.clone()]),
        ],
    );
    proofs.push((
        "lt_add1_le",
        pi(
            "a",
            gint(),
            pi("b", gint(), arrow(premise.clone(), order_fact("le", a, b))),
        ),
        lam("a", gint(), lam("b", gint(), lam("h", premise, proof))),
    ));
}

fn order_value(a: Term, b: Term) -> Term {
    apps(g("le"), &[a, b])
}
fn additive_translation(a: Term, b: Term, c: Term) -> Term {
    eq_at(
        g("Bool"),
        order_value(iadd(a.clone(), c.clone()), iadd(b.clone(), c)),
        order_value(a, b),
    )
}
fn order_pair_congr(a: Term, b: Term, x: Term, y: Term, pa: Term, pb: Term) -> Term {
    law_chain(
        g("Bool"),
        order_value(a.clone(), b.clone()),
        vec![
            (
                order_value(x.clone(), b.clone()),
                gcongr(
                    gint(),
                    g("Bool"),
                    lam("q", gint(), order_value(var("q"), b.clone())),
                    a,
                    x.clone(),
                    pa,
                ),
            ),
            (
                order_value(x.clone(), y.clone()),
                gcongr(
                    gint(),
                    g("Bool"),
                    lam("q", gint(), order_value(x, var("q"))),
                    b,
                    y,
                    pb,
                ),
            ),
        ],
    )
}
fn order_evidence_transport(p: Term, q: Term, equality: Term, witness: Term) -> Term {
    apps(
        g("Eq_rec_dep"),
        &[
            g("Bool"),
            p.clone(),
            lam(
                "out",
                g("Bool"),
                lam(
                    "eq",
                    eq_at(g("Bool"), p, var("out")),
                    eq_at(g("Bool"), var("out"), g("true")),
                ),
            ),
            witness,
            q,
            equality,
        ],
    )
}
fn inverse_translation_return(a: Term, n: Term) -> Term {
    let negative = gneg_succ(n.clone());
    let positive = gof(succ(n.clone()));
    let zero = gof(g("Zero"));
    let sum = iadd(negative.clone(), positive.clone());
    law_chain(
        gint(),
        iadd(iadd(a.clone(), negative.clone()), positive.clone()),
        vec![
            (
                iadd(a.clone(), sum.clone()),
                apps(g("add_assoc"), &[a.clone(), negative, positive]),
            ),
            (
                iadd(a.clone(), zero.clone()),
                gcongr(
                    gint(),
                    gint(),
                    lam("q", gint(), iadd(a.clone(), var("q"))),
                    sum,
                    zero,
                    app(g("IntLaw_subNatNat_self"), succ(n)),
                ),
            ),
            (a.clone(), app(g("add_zero"), a)),
        ],
    )
}
fn append_integer_ordered_addition(proofs: &mut Vec<Proof>) {
    let zero = gof(g("Zero"));
    let self_goal = |n: Term| eq_gint(idiff(n.clone(), n), zero.clone());
    proofs.push((
        "IntLaw_subNatNat_self",
        pi("n", nat(), self_goal(var("n"))),
        lam(
            "n",
            nat(),
            nat_ind(
                lam("k", nat(), self_goal(var("k"))),
                refl_gint(zero.clone()),
                lam(
                    "k",
                    nat(),
                    lam(
                        "ih",
                        self_goal(var("k")),
                        law_chain(
                            gint(),
                            idiff(succ(var("k")), succ(var("k"))),
                            vec![
                                (idiff(var("k"), var("k")), diff_cancel(var("k"), var("k"))),
                                (zero.clone(), var("ih")),
                            ],
                        ),
                    ),
                ),
                var("n"),
            ),
        ),
    ));
    let neg = |a: Term| app(g("Int_neg"), a);
    let inverse = |a: Term| eq_gint(iadd(a.clone(), neg(a)), zero.clone());
    proofs.push((
        "IntLaw_add_neg_right",
        pi("a", gint(), inverse(var("a"))),
        lam(
            "a",
            gint(),
            order_match(
                var("a"),
                "aa",
                gint(),
                inverse(var("aa")),
                vec![
                    lam(
                        "n",
                        nat(),
                        law_chain(
                            gint(),
                            iadd(gof(var("n")), neg(gof(var("n")))),
                            vec![
                                (
                                    idiff(var("n"), var("n")),
                                    mixed_sum(false, var("n"), var("n")),
                                ),
                                (zero.clone(), app(g("IntLaw_subNatNat_self"), var("n"))),
                            ],
                        ),
                    ),
                    lam("n", nat(), app(g("IntLaw_subNatNat_self"), succ(var("n")))),
                ],
            ),
        ),
    ));
    let a = var("a");
    let b = var("b");
    let n = var("n");
    let natural = gof(n.clone());
    let positive_goal = |n: Term| all2(gint(), additive_translation(var("a"), var("b"), gof(n)));
    let pos_base = abs2(
        gint(),
        order_pair_congr(
            iadd(a.clone(), zero.clone()),
            iadd(b.clone(), zero.clone()),
            a.clone(),
            b.clone(),
            app(g("add_zero"), a.clone()),
            app(g("add_zero"), b.clone()),
        ),
    );
    let an = iadd(a.clone(), natural.clone());
    let bn = iadd(b.clone(), natural.clone());
    let next = gof(succ(n.clone()));
    let anext = iadd(a.clone(), next.clone());
    let bnext = iadd(b.clone(), next.clone());
    let ai = increment_int(an.clone());
    let bi = increment_int(bn.clone());
    let pos_step = lam(
        "n",
        nat(),
        lam(
            "ih",
            positive_goal(n.clone()),
            abs2(
                gint(),
                law_chain(
                    g("Bool"),
                    order_value(anext.clone(), bnext.clone()),
                    vec![
                        (
                            order_value(ai.clone(), bi.clone()),
                            order_pair_congr(
                                anext.clone(),
                                bnext.clone(),
                                ai.clone(),
                                bi.clone(),
                                gsym(
                                    gint(),
                                    ai.clone(),
                                    anext,
                                    apps(g("add_assoc"), &[a.clone(), natural.clone(), gone()]),
                                ),
                                gsym(
                                    gint(),
                                    bi.clone(),
                                    bnext,
                                    apps(g("add_assoc"), &[b.clone(), natural.clone(), gone()]),
                                ),
                            ),
                        ),
                        (
                            order_value(an.clone(), bn.clone()),
                            apps(g("IntLaw_le_add_one"), &[an, bn]),
                        ),
                        (
                            order_value(a.clone(), b.clone()),
                            apps(var("ih"), &[a.clone(), b.clone()]),
                        ),
                    ],
                ),
            ),
        ),
    );
    proofs.push((
        "IntLaw_le_add_ofNat",
        pi("n", nat(), positive_goal(n.clone())),
        lam(
            "n",
            nat(),
            nat_ind(
                lam("k", nat(), positive_goal(var("k"))),
                pos_base,
                pos_step,
                n.clone(),
            ),
        ),
    ));
    let c = var("c");
    let nc = gneg_succ(n.clone());
    let pc = gof(succ(n.clone()));
    let x = iadd(a.clone(), nc.clone());
    let y = iadd(b.clone(), nc.clone());
    let xback = iadd(x.clone(), pc.clone());
    let yback = iadd(y.clone(), pc.clone());
    let neg_case = lam(
        "n",
        nat(),
        law_chain(
            g("Bool"),
            order_value(x.clone(), y.clone()),
            vec![
                (
                    order_value(xback.clone(), yback.clone()),
                    gsym(
                        g("Bool"),
                        order_value(xback.clone(), yback.clone()),
                        order_value(x.clone(), y.clone()),
                        apps(g("IntLaw_le_add_ofNat"), &[succ(n.clone()), x, y]),
                    ),
                ),
                (
                    order_value(a.clone(), b.clone()),
                    order_pair_congr(
                        xback,
                        yback,
                        a.clone(),
                        b.clone(),
                        inverse_translation_return(a.clone(), n.clone()),
                        inverse_translation_return(b.clone(), n.clone()),
                    ),
                ),
            ],
        ),
    );
    proofs.push((
        "IntLaw_le_add_right",
        all3(
            gint(),
            additive_translation(a.clone(), b.clone(), c.clone()),
        ),
        abs3(
            gint(),
            order_match(
                c.clone(),
                "cc",
                gint(),
                additive_translation(a.clone(), b.clone(), var("cc")),
                vec![
                    lam(
                        "n",
                        nat(),
                        apps(g("IntLaw_le_add_ofNat"), &[n.clone(), a.clone(), b.clone()]),
                    ),
                    neg_case,
                ],
            ),
        ),
    ));
    // Translate each premise, commute the second translation, and apply checked transitivity.
    let d = var("d");
    let ac = iadd(a.clone(), c.clone());
    let bc = iadd(b.clone(), c.clone());
    let bd = iadd(b.clone(), d.clone());
    let cb = iadd(c.clone(), b.clone());
    let db = iadd(d.clone(), b.clone());
    let h1 = order_evidence_transport(
        order_value(a.clone(), b.clone()),
        order_value(ac.clone(), bc.clone()),
        gsym(
            g("Bool"),
            order_value(ac.clone(), bc.clone()),
            order_value(a.clone(), b.clone()),
            apps(g("IntLaw_le_add_right"), &[a.clone(), b.clone(), c.clone()]),
        ),
        var("hab"),
    );
    let h2 = order_evidence_transport(
        order_value(c.clone(), d.clone()),
        order_value(bc.clone(), bd.clone()),
        law_chain(
            g("Bool"),
            order_value(c.clone(), d.clone()),
            vec![
                (
                    order_value(cb.clone(), db.clone()),
                    gsym(
                        g("Bool"),
                        order_value(cb.clone(), db.clone()),
                        order_value(c.clone(), d.clone()),
                        apps(g("IntLaw_le_add_right"), &[c.clone(), d.clone(), b.clone()]),
                    ),
                ),
                (
                    order_value(bc.clone(), bd.clone()),
                    order_pair_congr(
                        cb,
                        db,
                        bc.clone(),
                        bd.clone(),
                        apps(g("add_comm"), &[c.clone(), b.clone()]),
                        apps(g("add_comm"), &[d.clone(), b.clone()]),
                    ),
                ),
            ],
        ),
        var("hcd"),
    );
    let mut mono_ty = arrow(
        order_fact("le", a.clone(), b.clone()),
        arrow(
            order_fact("le", c.clone(), d.clone()),
            order_fact("le", ac.clone(), bd.clone()),
        ),
    );
    let mut mono_body = lam(
        "hab",
        order_fact("le", a.clone(), b.clone()),
        lam(
            "hcd",
            order_fact("le", c.clone(), d.clone()),
            apps(g("le_trans"), &[ac, bc, bd, h1, h2]),
        ),
    );
    for name in ["d", "c", "b", "a"] {
        mono_ty = pi(name, gint(), mono_ty);
        mono_body = lam(name, gint(), mono_body);
    }
    proofs.push(("le_add_mono", mono_ty, mono_body));
    // Translating by the additive inverse converts a<=b into 0<=b+(-1)*a.
    let na = neg(a.clone());
    let ca = iadd(a.clone(), na.clone());
    let cb = iadd(b.clone(), na.clone());
    let product = imul(gneg_succ(g("Zero")), a.clone());
    let result = iadd(b.clone(), product.clone());
    let transport = law_chain(
        g("Bool"),
        order_value(a.clone(), b.clone()),
        vec![
            (
                order_value(ca.clone(), cb.clone()),
                gsym(
                    g("Bool"),
                    order_value(ca.clone(), cb.clone()),
                    order_value(a.clone(), b.clone()),
                    apps(
                        g("IntLaw_le_add_right"),
                        &[a.clone(), b.clone(), na.clone()],
                    ),
                ),
            ),
            (
                order_value(zero.clone(), result.clone()),
                order_pair_congr(
                    ca,
                    cb,
                    zero.clone(),
                    result.clone(),
                    app(g("IntLaw_add_neg_right"), a.clone()),
                    gcongr(
                        gint(),
                        gint(),
                        lam("q", gint(), iadd(b.clone(), var("q"))),
                        na,
                        product,
                        app(g("IntLaw_neg_eq_mul_neg_one"), a.clone()),
                    ),
                ),
            ),
        ],
    );
    proofs.push((
        "le_sub",
        all2(
            gint(),
            arrow(
                order_fact("le", a.clone(), b.clone()),
                order_fact("le", zero.clone(), result.clone()),
            ),
        ),
        abs2(
            gint(),
            lam(
                "h",
                order_fact("le", a.clone(), b.clone()),
                order_evidence_transport(
                    order_value(a, b),
                    order_value(zero, result),
                    transport,
                    var("h"),
                ),
            ),
        ),
    ));
}

fn multiplied_order(k: Term, a: Term, b: Term) -> Term {
    order_fact("le", imul(k.clone(), a), imul(k, b))
}
fn left_zero_product(a: Term) -> Term {
    let z = gof(g("Zero"));
    law_chain(
        gint(),
        imul(z.clone(), a.clone()),
        vec![
            (
                imul(a.clone(), z.clone()),
                apps(g("mul_comm"), &[z.clone(), a.clone()]),
            ),
            (z, app(g("mul_zero"), a)),
        ],
    )
}
fn append_integer_multiplication_order(proofs: &mut Vec<Proof>) {
    let a = var("a");
    let b = var("b");
    let n = var("n");
    let z = gof(g("Zero"));
    let k = gof(n.clone());
    let left = imul(gof(succ(n.clone())), a.clone());
    let swapped = imul(a.clone(), iadd(k.clone(), gone()));
    let distribute = iadd(imul(a.clone(), k.clone()), imul(a.clone(), gone()));
    let middle = iadd(imul(k.clone(), a.clone()), imul(a.clone(), gone()));
    let result = iadd(imul(k.clone(), a.clone()), a.clone());
    proofs.push((
        "IntLaw_mul_ofNat_succ",
        pi(
            "n",
            nat(),
            pi("a", gint(), eq_gint(left.clone(), result.clone())),
        ),
        lam(
            "n",
            nat(),
            lam(
                "a",
                gint(),
                law_chain(
                    gint(),
                    left,
                    vec![
                        (
                            swapped,
                            apps(g("mul_comm"), &[iadd(k.clone(), gone()), a.clone()]),
                        ),
                        (
                            distribute,
                            apps(g("mul_distrib_add"), &[a.clone(), k.clone(), gone()]),
                        ),
                        (
                            middle,
                            gcongr(
                                gint(),
                                gint(),
                                lam("q", gint(), iadd(var("q"), imul(a.clone(), gone()))),
                                imul(a.clone(), k.clone()),
                                imul(k.clone(), a.clone()),
                                apps(g("mul_comm"), &[a.clone(), k.clone()]),
                            ),
                        ),
                        (
                            result,
                            gcongr(
                                gint(),
                                gint(),
                                lam("q", gint(), iadd(imul(k.clone(), a.clone()), var("q"))),
                                imul(a.clone(), gone()),
                                a.clone(),
                                app(g("mul_one"), a.clone()),
                            ),
                        ),
                    ],
                ),
            ),
        ),
    ));
    let goal = |n: Term| {
        all2(
            gint(),
            arrow(
                order_fact("le", var("a"), var("b")),
                multiplied_order(gof(n), var("a"), var("b")),
            ),
        )
    };
    let az = imul(z.clone(), a.clone());
    let bz = imul(z.clone(), b.clone());
    let base = abs2(
        gint(),
        lam(
            "hab",
            order_fact("le", a.clone(), b.clone()),
            order_evidence_transport(
                order_value(z.clone(), z.clone()),
                order_value(az.clone(), bz.clone()),
                gsym(
                    g("Bool"),
                    order_value(az.clone(), bz.clone()),
                    order_value(z.clone(), z.clone()),
                    order_pair_congr(
                        az,
                        bz,
                        z.clone(),
                        z.clone(),
                        left_zero_product(a.clone()),
                        left_zero_product(b.clone()),
                    ),
                ),
                app(g("le_refl"), z.clone()),
            ),
        ),
    );
    let ka = imul(k.clone(), a.clone());
    let kb = imul(k.clone(), b.clone());
    let sa = iadd(ka.clone(), a.clone());
    let sb = iadd(kb.clone(), b.clone());
    let na = imul(gof(succ(n.clone())), a.clone());
    let nb = imul(gof(succ(n.clone())), b.clone());
    let step = lam(
        "n",
        nat(),
        lam(
            "ih",
            goal(n.clone()),
            abs2(
                gint(),
                lam(
                    "hab",
                    order_fact("le", a.clone(), b.clone()),
                    order_evidence_transport(
                        order_value(sa.clone(), sb.clone()),
                        order_value(na.clone(), nb.clone()),
                        gsym(
                            g("Bool"),
                            order_value(na.clone(), nb.clone()),
                            order_value(sa.clone(), sb.clone()),
                            order_pair_congr(
                                na,
                                nb,
                                sa,
                                sb,
                                apps(g("IntLaw_mul_ofNat_succ"), &[n.clone(), a.clone()]),
                                apps(g("IntLaw_mul_ofNat_succ"), &[n.clone(), b.clone()]),
                            ),
                        ),
                        apps(
                            g("le_add_mono"),
                            &[
                                ka,
                                kb,
                                a.clone(),
                                b.clone(),
                                apps(var("ih"), &[a.clone(), b.clone(), var("hab")]),
                                var("hab"),
                            ],
                        ),
                    ),
                ),
            ),
        ),
    );
    proofs.push((
        "IntLaw_le_mul_ofNat",
        pi("n", nat(), goal(n.clone())),
        lam(
            "n",
            nat(),
            nat_ind(lam("nn", nat(), goal(var("nn"))), base, step, n.clone()),
        ),
    ));
    let public_goal = |k: Term| {
        all2(
            gint(),
            arrow(
                order_fact("le", z.clone(), k.clone()),
                arrow(
                    order_fact("le", var("a"), var("b")),
                    multiplied_order(k, var("a"), var("b")),
                ),
            ),
        )
    };
    let cases = [true, false]
        .into_iter()
        .map(|positive| {
            let k = if positive {
                gof(n.clone())
            } else {
                gneg_succ(n.clone())
            };
            let proof = if positive {
                apps(
                    g("IntLaw_le_mul_ofNat"),
                    &[n.clone(), a.clone(), b.clone(), var("hab")],
                )
            } else {
                impossible_boolean_order(
                    multiplied_order(k.clone(), a.clone(), b.clone()),
                    var("hk"),
                )
            };
            lam(
                "n",
                nat(),
                abs2(
                    gint(),
                    lam(
                        "hk",
                        order_fact("le", z.clone(), k),
                        lam("hab", order_fact("le", a.clone(), b.clone()), proof),
                    ),
                ),
            )
        })
        .collect();
    proofs.push((
        "le_mul_nonneg",
        pi("k", gint(), public_goal(var("k"))),
        lam(
            "k",
            gint(),
            order_match(var("k"), "kk", gint(), public_goal(var("kk")), cases),
        ),
    ));
}
