//! Tier 6 of the Lean-4 conformance corpus — propositions, equality, classical
//! reasoning. Eighteen cases, and the tier where the head-to-head is decided by what we
//! *trust* rather than by what we can parse.
//!
//! Lean 4's trusted base is three axioms: `propext`, `Classical.choice`, `Quot.sound`.
//! LOGOS registers **eighty-nine** declarations, of which exactly one is classical (`dne`)
//! and five are the `Quot` family. The overlap is `Quot.sound` alone. Read in one direction
//! that is a win — excluded middle is *derived* here, not assumed, and every equality lemma
//! (`Eq_rec_dep`, `Eq_rec`, `Eq_sym`, `Eq_trans`), the `Decidable` eliminator, `decide` and
//! `of_decide_eq_true` are kernel-CHECKED definitions rather than axioms. Read in the other
//! it is the tier's whole gap list: no `propext`, no `funext`, no `Classical.choice`, no
//! `HEq`, no `Setoid`/`Quotient`, no `Subsingleton`, no `Iff`.
//!
//! Three things these tests insist on beyond mere presence:
//!
//! 1. **Tracking.** Adding a missing axiom only reaches parity if the system can say which
//!    axioms a proof actually used, the way `#print axioms` does. LOGOS has no such report
//!    at any surface, so every "add the axiom" test here also demands the report.
//! 2. **Transitive taint.** A `sorry` makes every downstream theorem vacuously verified, and
//!    a per-declaration warning does not propagate. The taint must be the transitive closure
//!    of the elaborated environment.
//! 3. **Diagnostics that say what they mean.** `certificate::recheck` rebuilds the prelude
//!    from `StandardLibrary::register`, so a proof citing an invented axiom fails with a bare
//!    `UnboundVariable` — a message that reads like a name-resolution bug and invites the one
//!    catastrophic repair (patching the prelude). Re-checking is reproduced here inline (a
//!    fresh `Context` + `StandardLibrary::register` + `infer_type` is exactly what `recheck`
//!    does) so the tier runs without the optional `serde` feature.
//!
//! A proof is asserted REAL, never merely `Ok`: every constructed term is `infer_type`d, its
//! inferred type checked with `is_subtype` against the proposition it claims, and — where it
//! is the load-bearing claim — corroborated by the second, independently-written de Bruijn
//! kernel via `double_check`.

use logicaffeine_kernel::interface::Repl;
use logicaffeine_kernel::prelude::{axiom_dependencies_of, StandardLibrary};
use logicaffeine_kernel::{
    defeq_for_test, derive_recursor, double_check, infer_type, is_subtype, normalize,
    resolve_instance, Context, DoubleCheck, KernelError, MetaCtx, Term, Universe,
};

// ============================================================================
// Per-file helpers (house style: every kernel test declares its own).
// ============================================================================

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: Vec<Term>) -> Term {
    xs.into_iter().fold(f, app)
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
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn type0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
/// `Eq A a b`.
fn eq(ty: Term, a: Term, b: Term) -> Term {
    apps(g("Eq"), vec![ty, a, b])
}
/// `refl A a`.
fn refl(ty: Term, a: Term) -> Term {
    apps(g("refl"), vec![ty, a])
}
/// Unary `Nat` numeral — the prelude's `Succ`/`Zero`, not a `Literal`.
fn nat(n: u32) -> Term {
    let mut t = g("Zero");
    for _ in 0..n {
        t = app(g("Succ"), t);
    }
    t
}
fn not(p: Term) -> Term {
    app(g("Not"), p)
}
fn or(p: Term, q: Term) -> Term {
    apps(g("Or"), vec![p, q])
}
fn and(p: Term, q: Term) -> Term {
    apps(g("And"), vec![p, q])
}
fn ex(dom: Term, pred: Term) -> Term {
    apps(g("Ex"), vec![dom, pred])
}

/// Kernel-verify `term` and require its inferred type to be the claimed proposition —
/// then corroborate with the SECOND de Bruijn kernel. `Ok` from `infer_type` alone only
/// says a term was produced; agreement says two independently-written checkers accepted it.
fn assert_proves(ctx: &Context, term: &Term, claimed: &Term, what: &str) {
    let inferred = match infer_type(ctx, term) {
        Ok(t) => t,
        Err(e) => panic!("{what} must kernel-check, but the kernel rejected it: {e:?}"),
    };
    assert!(
        is_subtype(ctx, &inferred, claimed),
        "{what} must prove `{claimed}`, but its type is `{inferred}`"
    );
    assert!(
        matches!(double_check(ctx, term), DoubleCheck::Agreed),
        "{what} must be corroborated by the second de Bruijn kernel: {:?}",
        double_check(ctx, term)
    );
}

/// Reproduce `certificate::recheck`'s integrity rule: rebuild the trusted context from
/// `StandardLibrary::register` alone and re-infer. A proof citing anything the prelude does
/// not contain fails here — this is the only thing that catches an added axiom.
fn recheck_against_a_rebuilt_prelude(term: &Term) -> Result<Term, KernelError> {
    let fresh = std_ctx();
    infer_type(&fresh, term)
}

/// `inductive`'s derived eliminator, published as `{inductive}_rec`. The prelude already
/// publishes some of them (`Or_rec`); a published one must BE the derived one, and an absent one
/// is installed.
fn with_recursor(ctx: &mut Context, inductive: &str) {
    let (ty, body) = derive_recursor(ctx, inductive)
        .unwrap_or_else(|e| panic!("{inductive}'s eliminator must derive: {e:?}"));
    let name = format!("{inductive}_rec");
    if ctx.get_global(&name).is_none() {
        ctx.add_definition(name.clone(), ty.clone(), body);
    }
    let published = ctx
        .get_global(&name)
        .cloned()
        .unwrap_or_else(|| panic!("`{name}` must register: {:?}", ctx.refused_declarations()));
    assert!(
        defeq_for_test(ctx, &published, &ty),
        "`{name}` is {inductive}'s derived eliminator, but it is published as `{published}`"
    );
}

/// `λP. dne (Or P (Not P)) (λk. k (right P ¬P (λhp. k (left P ¬P hp))))` — excluded middle
/// from double-negation elimination, the standard intuitionistic derivation. Used by three
/// cases, so it is built once.
fn classical_em_body() -> Term {
    let p = var("P");
    let goal = or(p.clone(), not(p.clone()));
    let inner = lam(
        "hp",
        p.clone(),
        app(
            var("k"),
            apps(g("left"), vec![p.clone(), not(p.clone()), var("hp")]),
        ),
    );
    lam(
        "P",
        prop(),
        apps(
            g("dne"),
            vec![
                goal.clone(),
                lam(
                    "k",
                    arrow(goal, g("False")),
                    app(
                        var("k"),
                        apps(g("right"), vec![p.clone(), not(p.clone()), inner]),
                    ),
                ),
            ],
        ),
    )
}
fn classical_em_type() -> Term {
    pi("P", prop(), or(var("P"), not(var("P"))))
}

// ============================================================================
// A. EQUALITY — `Eq`, J, and the congruence lemmas.
// ============================================================================

/// T06-001 · MUST · Lean's `Eq` is `{α : Sort u}`; LOGOS pins the carrier to `Type 0`, so
/// `True = True` — ordinary Lean, and the statement `propext` concludes — does not typecheck.
#[test]
fn an_equation_is_stateable_at_every_carrier_lean_allows_not_only_type_zero() {
    let ctx = std_ctx();

    // The baseline works, and it is a real proof in both kernels.
    assert_proves(
        &ctx,
        &refl(g("Nat"), nat(3)),
        &eq(g("Nat"), nat(3), nat(3)),
        "`rfl : 3 = 3`",
    );

    // An equation at a function type — still a `Type 0` inhabitant, still fine.
    let idf = lam("n", g("Nat"), var("n"));
    assert_proves(
        &ctx,
        &refl(arrow(g("Nat"), g("Nat")), idf.clone()),
        &eq(arrow(g("Nat"), g("Nat")), idf.clone(), idf),
        "`rfl` at a function carrier",
    );

    // THE GAP: `p = q` between two propositions, and `α = β` between two
    // types, are both ordinary Lean and must be stateable here.
    let prop_eq = eq(prop(), g("True"), g("True"));
    match infer_type(&ctx, &prop_eq) {
        Ok(sort) => assert!(
            matches!(sort, Term::Sort(Universe::Prop)),
            "`True = True` must be a Prop, not `{sort}`"
        ),
        Err(e) => panic!(
            "an equation between two propositions is ordinary Lean (it is the conclusion of \
             `propext`) and must typecheck: {e:?}"
        ),
    }
    let type_eq = eq(type0(), g("Nat"), g("Nat"));
    match infer_type(&ctx, &type_eq) {
        Ok(sort) => assert!(
            matches!(sort, Term::Sort(Universe::Prop)),
            "`Nat = Nat` at `Type 0` must be a Prop, not `{sort}`"
        ),
        Err(e) => panic!("an equation between two types must typecheck: {e:?}"),
    }

    // …and widening the carrier must never be done by passing a hole. `types_equal` matches
    // `Hole` against anything, so `Eq _ Zero true` — an equation between a `Nat` and a
    // `Bool` — is currently a well-formed `Prop`, which is a worse answer than rejection.
    assert!(
        infer_type(&ctx, &eq(Term::Hole, g("Zero"), g("true"))).is_err(),
        "an equation whose carrier is a hole relates two values of unrelated types and must \
         be rejected, not inferred as a Prop"
    );
}

/// T06-002 · MUST · The dependent eliminator is auto-DERIVED and kernel-checked — it is not
/// an axiom, which is strictly better than Lean. But its motive target is the literal
/// `Sort(Type 0)`, where Lean's `Eq.rec` motive is `Sort u`, so a transport of a type family
/// has no eliminator at all.
#[test]
fn the_derived_j_eliminator_carries_a_motive_into_any_sort() {
    let ctx = std_ctx();

    // J is a DEFINITION. Axiomatising it would be a silent trusted-base expansion.
    assert!(
        ctx.is_definition("Eq_rec_dep"),
        "`Eq_rec_dep` must be a kernel-checked definition, never an axiom — the prelude \
         proves the equality eliminator so it stays out of the trusted base"
    );
    assert!(
        ctx.is_definition("Eq_rec"),
        "`Eq_rec` must be derived from J as a definition, never axiomatised"
    );

    // Non-dependent transport (`subst` / `▸`) at a `Prop` motive.
    let subst_ty = pi(
        "P",
        arrow(g("Nat"), prop()),
        pi(
            "a",
            g("Nat"),
            pi(
                "b",
                g("Nat"),
                arrow(
                    eq(g("Nat"), var("a"), var("b")),
                    arrow(app(var("P"), var("a")), app(var("P"), var("b"))),
                ),
            ),
        ),
    );
    let subst = lam(
        "P",
        arrow(g("Nat"), prop()),
        lam(
            "a",
            g("Nat"),
            lam(
                "b",
                g("Nat"),
                lam(
                    "h",
                    eq(g("Nat"), var("a"), var("b")),
                    lam(
                        "pa",
                        app(var("P"), var("a")),
                        apps(
                            g("Eq_rec"),
                            vec![g("Nat"), var("a"), var("P"), var("pa"), var("b"), var("h")],
                        ),
                    ),
                ),
            ),
        ),
    );
    assert_proves(
        &ctx,
        &subst,
        &subst_ty,
        "transport along an equation (`Eq_rec`)",
    );

    // Fully dependent J: the motive mentions the equation proof itself.
    let motive_ty = pi(
        "b",
        g("Nat"),
        arrow(eq(g("Nat"), var("a"), var("b")), prop()),
    );
    let j_ty = pi(
        "a",
        g("Nat"),
        pi(
            "motive",
            motive_ty.clone(),
            arrow(
                apps(var("motive"), vec![var("a"), refl(g("Nat"), var("a"))]),
                pi(
                    "b",
                    g("Nat"),
                    pi(
                        "h",
                        eq(g("Nat"), var("a"), var("b")),
                        apps(var("motive"), vec![var("b"), var("h")]),
                    ),
                ),
            ),
        ),
    );
    let j = lam(
        "a",
        g("Nat"),
        lam(
            "motive",
            motive_ty,
            lam(
                "base",
                apps(var("motive"), vec![var("a"), refl(g("Nat"), var("a"))]),
                lam(
                    "b",
                    g("Nat"),
                    lam(
                        "h",
                        eq(g("Nat"), var("a"), var("b")),
                        apps(
                            g("Eq_rec_dep"),
                            vec![
                                g("Nat"),
                                var("a"),
                                var("motive"),
                                var("base"),
                                var("b"),
                                var("h"),
                            ],
                        ),
                    ),
                ),
            ),
        ),
    );
    assert_proves(&ctx, &j, &j_ty, "fully dependent J (`Eq_rec_dep`)");

    // THE GAP. Lean's `Eq.rec` motive is `Sort u`. Transporting a TYPE FAMILY — `F a → F b`
    // for `F : Nat → Type` — needs a motive landing in `Type 1`, and the derived eliminator's
    // motive target is hard-coded to `Type 0`, so there is no level to instantiate.
    let big_motive = lam(
        "idx",
        g("Nat"),
        lam("_h", eq(g("Nat"), var("a"), var("idx")), type0()),
    );
    let big_transport = lam(
        "a",
        g("Nat"),
        lam(
            "b",
            g("Nat"),
            lam(
                "h",
                eq(g("Nat"), var("a"), var("b")),
                apps(
                    g("Eq_rec_dep"),
                    vec![g("Nat"), var("a"), big_motive, g("Nat"), var("b"), var("h")],
                ),
            ),
        ),
    );
    let big_transport_ty = pi(
        "a",
        g("Nat"),
        pi(
            "b",
            g("Nat"),
            arrow(eq(g("Nat"), var("a"), var("b")), type0()),
        ),
    );
    assert_proves(
        &ctx,
        &big_transport,
        &big_transport_ty,
        "J at a `Type 1` motive (transporting a type family, `Eq.rec` with `motive : … → Type`)",
    );
}

/// T06-003 · MUST · `Eq.symm`/`Eq.trans` are prelude DEFINITIONS derived from J, and
/// `congrArg`/`congrFun`/`congr` derive from J too. Nothing here needs an axiom.
#[test]
fn congruence_lemmas_derive_from_j_and_reuse_the_prelude_symm_and_trans() {
    let ctx = std_ctx();

    // Symmetry and transitivity are DERIVED, in the prelude, under these exact names.
    for name in ["Eq_sym", "Eq_trans"] {
        assert!(
            ctx.is_definition(name),
            "`{name}` must be a derived definition in the prelude — reused, never re-derived \
             under the same flat name and never axiomatised"
        );
    }
    let sym_use = apps(
        g("Eq_sym"),
        vec![g("Nat"), nat(3), nat(3), refl(g("Nat"), nat(3))],
    );
    assert_proves(
        &ctx,
        &sym_use,
        &eq(g("Nat"), nat(3), nat(3)),
        "`Eq_sym` applied",
    );
    let trans_use = apps(
        g("Eq_trans"),
        vec![
            g("Nat"),
            nat(3),
            nat(3),
            nat(3),
            refl(g("Nat"), nat(3)),
            refl(g("Nat"), nat(3)),
        ],
    );
    assert_proves(
        &ctx,
        &trans_use,
        &eq(g("Nat"), nat(3), nat(3)),
        "`Eq_trans` applied",
    );

    // `congrArg` across two carriers, derived from `Eq_rec_dep`. It is NOT a primitive in
    // Lean either, and it must not become an axiom here.
    let congr_arg_ty = pi(
        "A",
        type0(),
        pi(
            "B",
            type0(),
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
                            eq(var("A"), var("x"), var("y")),
                            eq(var("B"), app(var("f"), var("x")), app(var("f"), var("y"))),
                        ),
                    ),
                ),
            ),
        ),
    );
    let congr_motive = lam(
        "idx",
        var("A"),
        lam(
            "_h",
            eq(var("A"), var("x"), var("idx")),
            eq(var("B"), app(var("f"), var("x")), app(var("f"), var("idx"))),
        ),
    );
    let congr_core = apps(
        g("Eq_rec_dep"),
        vec![
            var("A"),
            var("x"),
            congr_motive,
            refl(var("B"), app(var("f"), var("x"))),
            var("y"),
            var("h"),
        ],
    );
    let congr_arg = lam(
        "A",
        type0(),
        lam(
            "B",
            type0(),
            lam(
                "f",
                arrow(var("A"), var("B")),
                lam(
                    "x",
                    var("A"),
                    lam(
                        "y",
                        var("A"),
                        lam("h", eq(var("A"), var("x"), var("y")), congr_core),
                    ),
                ),
            ),
        ),
    );
    assert_proves(&ctx, &congr_arg, &congr_arg_ty, "`congrArg` derived from J");

    // `congrFun` — the SOUND direction (`f = g → ∀ a, f a = g a`). Its converse is `funext`,
    // which is a different, absent axiom; a translator must not conflate them (T06-011).
    let fn_ty = || arrow(g("Nat"), g("Nat"));
    let congr_fun_ty = pi(
        "f",
        fn_ty(),
        pi(
            "g",
            fn_ty(),
            arrow(
                eq(fn_ty(), var("f"), var("g")),
                pi(
                    "a",
                    g("Nat"),
                    eq(g("Nat"), app(var("f"), var("a")), app(var("g"), var("a"))),
                ),
            ),
        ),
    );
    let congr_fun_motive = lam(
        "idx",
        fn_ty(),
        lam(
            "_h",
            eq(fn_ty(), var("f"), var("idx")),
            pi(
                "a",
                g("Nat"),
                eq(g("Nat"), app(var("f"), var("a")), app(var("idx"), var("a"))),
            ),
        ),
    );
    let congr_fun = lam(
        "f",
        fn_ty(),
        lam(
            "g",
            fn_ty(),
            lam(
                "h",
                eq(fn_ty(), var("f"), var("g")),
                apps(
                    g("Eq_rec_dep"),
                    vec![
                        fn_ty(),
                        var("f"),
                        congr_fun_motive,
                        lam("a", g("Nat"), refl(g("Nat"), app(var("f"), var("a")))),
                        var("g"),
                        var("h"),
                    ],
                ),
            ),
        ),
    );
    assert_proves(&ctx, &congr_fun, &congr_fun_ty, "`congrFun` derived from J");
}

// ============================================================================
// B. DEFINITIONAL PROOF IRRELEVANCE.
// ============================================================================

/// T06-004 · MUST · Lean's proof irrelevance is a total definitional rule. LOGOS's is a
/// last-resort predicate reached only after structural congruence fails, and it answers
/// "not equal" whenever any of its four `infer_type` calls errors. These are the shapes the
/// corpus names; every one of them must close.
#[test]
fn two_proofs_of_one_proposition_are_definitionally_equal_at_every_shape() {
    let mut ctx = std_ctx();

    // Two opaque proofs of `True`.
    ctx.add_declaration("p", g("True"));
    ctx.add_declaration("q", g("True"));
    assert!(
        defeq_for_test(&ctx, &g("p"), &g("q")),
        "two proofs of `True` must be definitionally equal by proof irrelevance"
    );
    assert!(
        defeq_for_test(&ctx, &g("p"), &g("I")),
        "an opaque proof of `True` and its canonical constructor must be definitionally equal"
    );

    // A proposition with two genuinely different derivations.
    ctx.add_declaration("h", or(g("True"), g("True")));
    assert!(
        defeq_for_test(
            &ctx,
            &g("h"),
            &apps(g("left"), vec![g("True"), g("True"), g("I")])
        ),
        "two derivations of `True ∨ True` must be definitionally equal by proof irrelevance"
    );

    // Propositions equal only up to REDUCTION: `Small 3` δβ-reduces to `3 = 3`, so the two
    // inferred types meet only after `normalize` runs under its fuel.
    ctx.add_definition(
        "Small".to_string(),
        pi("n", g("Nat"), prop()),
        lam("n", g("Nat"), eq(g("Nat"), var("n"), var("n"))),
    );
    ctx.add_declaration("small3", app(g("Small"), nat(3)));
    assert!(
        defeq_for_test(&ctx, &g("small3"), &refl(g("Nat"), nat(3))),
        "proofs of `Small 3` and of `3 = 3` must be definitionally equal — the two \
         propositions are equal after reduction, which irrelevance must reach"
    );

    // Irrelevance UNDER a binder: the proof's type mentions the bound variable, so it is
    // only typeable in the extended context.
    let l1 = lam(
        "n",
        g("Nat"),
        lam("h1", eq(g("Nat"), var("n"), var("n")), var("h1")),
    );
    let l2 = lam(
        "n",
        g("Nat"),
        lam(
            "h1",
            eq(g("Nat"), var("n"), var("n")),
            refl(g("Nat"), var("n")),
        ),
    );
    assert!(
        defeq_for_test(&ctx, &l1, &l2),
        "two proofs of `n = n` under a `Π(n:Nat)` binder must be definitionally equal — \
         irrelevance runs against the EXTENDED context, not the outer one"
    );

    // A structure whose proof field is erased: two records differing only in that field.
    let mut s = std_ctx();
    s.add_structure(
        "Bounded",
        &[],
        &[
            ("val", g("Nat")),
            ("isSmall", eq(g("Nat"), var("val"), var("val"))),
        ],
    );
    s.add_declaration("h1", eq(g("Nat"), nat(3), nat(3)));
    s.add_declaration("h2", eq(g("Nat"), nat(3), nat(3)));
    let b1 = apps(g("Bounded_mk"), vec![nat(3), g("h1")]);
    let b2 = apps(g("Bounded_mk"), vec![nat(3), g("h2")]);
    assert!(
        defeq_for_test(&s, &b1, &b2),
        "two records agreeing on their data field and differing only in an erased proof \
         field must be definitionally equal"
    );

    // …and irrelevance must stay a PROOF rule: it may never equate two propositions'
    // proofs across different propositions, nor two pieces of data.
    ctx.add_declaration("pnat", eq(g("Nat"), nat(2), nat(2)));
    assert!(
        !defeq_for_test(&ctx, &g("p"), &g("pnat")),
        "irrelevance must not equate proofs of DIFFERENT propositions"
    );
    assert!(
        !defeq_for_test(&ctx, &nat(0), &nat(1)),
        "irrelevance must never reach data: `Zero` and `Succ Zero` are distinct"
    );
}

// ============================================================================
// C. CONNECTIVES AND QUANTIFIERS.
// ============================================================================

/// T06-005 · MUST · `And`/`Or`/`Not`/`True`/`False` are in the prelude (with `Or`'s
/// constructors named `left`/`right`, not `inl`/`inr`, and `Not` a delta-transparent
/// definition). `Iff` is not there at all, so every Lean `↔` has nowhere to land.
#[test]
fn the_five_connectives_and_the_biconditional_are_all_in_the_prelude() {
    let mut ctx = std_ctx();

    // Conjunction: introduction, and both eliminations through the derived recursor.
    let conj = apps(g("conj"), vec![g("True"), g("True"), g("I"), g("I")]);
    assert_proves(&ctx, &conj, &and(g("True"), g("True")), "`And.intro`");

    // Disjunction: LOGOS spells the constructors `left` and `right`.
    let inl = apps(g("left"), vec![g("True"), g("False"), g("I")]);
    assert_proves(
        &ctx,
        &inl,
        &or(g("True"), g("False")),
        "`Or.inl` (LOGOS `left`)",
    );
    assert!(
        ctx.is_constructor("left") && ctx.is_constructor("right"),
        "`Or`'s constructors are `left`/`right`; `inl`/`inr` resolve to nothing"
    );

    // `Not` is a DEFINITION, so `¬P` and `P → False` are definitionally interchangeable
    // with no coercion. Axiomatising it would leave `¬p` an opaque Prop with no elimination.
    assert!(
        ctx.is_definition("Not"),
        "`Not` must be a delta-transparent definition"
    );
    assert!(
        defeq_for_test(&ctx, &not(g("True")), &arrow(g("True"), g("False"))),
        "`¬P` and `P → False` must be definitionally equal"
    );

    // `False` has zero constructors, so it large-eliminates into `Type` — `exFalso` at a
    // data motive, which Lean also allows.
    with_recursor(&mut ctx, "False");
    let ex_falso_large = lam(
        "h",
        g("False"),
        apps(
            g("False_rec"),
            vec![lam("_", g("False"), g("Nat")), var("h")],
        ),
    );
    assert_proves(
        &ctx,
        &ex_falso_large,
        &arrow(g("False"), g("Nat")),
        "`False.elim` at a DATA motive (large elimination of a zero-constructor Prop)",
    );

    // `Or` has TWO constructors, so it is not a subsingleton and its eliminator's motive is
    // pinned to `Prop`. Lean agrees; this is the firewall that keeps `em` from yielding data.
    with_recursor(&mut ctx, "Or");
    let or_elim = lam(
        "r",
        prop(),
        lam(
            "hp",
            arrow(g("True"), var("r")),
            lam(
                "hq",
                arrow(g("False"), var("r")),
                lam(
                    "h",
                    or(g("True"), g("False")),
                    apps(
                        g("Or_rec"),
                        vec![
                            g("True"),
                            g("False"),
                            lam("_", or(g("True"), g("False")), var("r")),
                            var("hp"),
                            var("hq"),
                            var("h"),
                        ],
                    ),
                ),
            ),
        ),
    );
    let or_elim_ty = pi(
        "r",
        prop(),
        arrow(
            arrow(g("True"), var("r")),
            arrow(
                arrow(g("False"), var("r")),
                arrow(or(g("True"), g("False")), var("r")),
            ),
        ),
    );
    assert_proves(&ctx, &or_elim, &or_elim_ty, "`Or.elim` at a Prop motive");

    // THE GAP. `Iff` is absent from the kernel entirely. Lean core has it, `simp` is built on
    // it, and it must be a DEFINITION (`And (P→Q) (Q→P)`) so that `.mp`/`.mpr` are projections
    // — an opaque axiom or a fresh inductive would be a different proposition that nothing,
    // `propext` included, can bridge back.
    assert!(
        ctx.get_global("Iff").is_some(),
        "`Iff` must be in the prelude — every Lean `p ↔ q` needs somewhere to land"
    );
    assert!(
        ctx.is_definition("Iff"),
        "`Iff` must be a DEFINITION unfolding to `And (P→Q) (Q→P)`, never an axiom and never \
         a fresh inductive — otherwise `.mp`/`.mpr` are not projections of the conjunction"
    );
    let iff_tt = apps(g("Iff"), vec![g("True"), g("True")]);
    assert!(
        defeq_for_test(
            &ctx,
            &iff_tt,
            &and(arrow(g("True"), g("True")), arrow(g("True"), g("True")))
        ),
        "`p ↔ q` must be definitionally `(p → q) ∧ (q → p)`"
    );
}

/// T06-006 · MUST · `Ex` is in the prelude with the constructor spelled `witness`, but its
/// domain is pinned to `Type 0`, so `∃ p : Prop, …` and `∃ α : Type, …` — both ordinary
/// Lean — cannot be stated.
#[test]
fn an_existential_quantifies_over_every_domain_lean_allows() {
    let mut ctx = std_ctx();

    // The predicate is a LAMBDA argument, not a second binder.
    let pred = lam("n", g("Nat"), eq(g("Nat"), var("n"), var("n")));
    let wit = apps(
        g("witness"),
        vec![g("Nat"), pred.clone(), nat(3), refl(g("Nat"), nat(3))],
    );
    assert_proves(
        &ctx,
        &wit,
        &ex(g("Nat"), pred.clone()),
        "`⟨3, rfl⟩ : ∃ n, n = n`",
    );
    assert!(
        ctx.is_constructor("witness"),
        "`Exists.intro` is spelled `witness` here; lookup is by flat name"
    );

    // Nested existentials: the inner `Ex` lives inside the outer predicate's lambda body.
    let inner_pred = lam("n", g("Nat"), eq(g("Nat"), var("m"), var("n")));
    let outer_pred = lam("m", g("Nat"), ex(g("Nat"), inner_pred.clone()));
    let nested = apps(
        g("witness"),
        vec![
            g("Nat"),
            outer_pred.clone(),
            nat(2),
            apps(
                g("witness"),
                vec![
                    g("Nat"),
                    lam("n", g("Nat"), eq(g("Nat"), nat(2), var("n"))),
                    nat(2),
                    refl(g("Nat"), nat(2)),
                ],
            ),
        ],
    );
    assert_proves(
        &ctx,
        &nested,
        &ex(g("Nat"), outer_pred),
        "a nested `∃ m, ∃ n, …`",
    );

    // Elimination is available at a `Prop` motive — and NOT at a data motive, because that
    // would be choice (T06-009). The negative direction is asserted too: it is the firewall.
    with_recursor(&mut ctx, "Ex");
    let elim = lam(
        "Q",
        prop(),
        lam(
            "k",
            pi("n", g("Nat"), arrow(app(pred.clone(), var("n")), var("Q"))),
            lam(
                "h",
                ex(g("Nat"), pred.clone()),
                apps(
                    g("Ex_rec"),
                    vec![
                        g("Nat"),
                        pred.clone(),
                        lam("_", ex(g("Nat"), pred.clone()), var("Q")),
                        var("k"),
                        var("h"),
                    ],
                ),
            ),
        ),
    );
    let elim_ty = pi(
        "Q",
        prop(),
        arrow(
            pi("n", g("Nat"), arrow(app(pred.clone(), var("n")), var("Q"))),
            arrow(ex(g("Nat"), pred.clone()), var("Q")),
        ),
    );
    assert_proves(&ctx, &elim, &elim_ty, "`Exists.elim` at a Prop motive");

    let into_data = apps(
        g("Ex_rec"),
        vec![
            g("Nat"),
            pred.clone(),
            lam("_", ex(g("Nat"), pred.clone()), g("Nat")),
            lam(
                "n",
                g("Nat"),
                lam("_", app(pred.clone(), var("n")), var("n")),
            ),
        ],
    );
    assert!(
        infer_type(&ctx, &into_data).is_err(),
        "`Exists.elim` into `Type` extracts data from a proposition — that is choice, and it \
         must stay structurally blocked"
    );

    // THE GAP. Lean's `Exists` is `{α : Sort u}`.
    let prop_dom = ex(prop(), lam("p", prop(), var("p")));
    match infer_type(&ctx, &prop_dom) {
        Ok(sort) => assert!(
            matches!(sort, Term::Sort(Universe::Prop)),
            "`∃ p : Prop, p` must be a Prop, not `{sort}`"
        ),
        Err(e) => panic!("`∃ p : Prop, p` is ordinary Lean and must be stateable: {e:?}"),
    }
    let type_dom = ex(type0(), lam("a", type0(), eq(type0(), var("a"), g("Nat"))));
    match infer_type(&ctx, &type_dom) {
        Ok(sort) => assert!(
            matches!(sort, Term::Sort(Universe::Prop)),
            "`∃ α : Type, α = Nat` must be a Prop, not `{sort}`"
        ),
        Err(e) => panic!("`∃ α : Type, …` is ordinary Lean and must be stateable: {e:?}"),
    }
}

/// T06-007 · MUST · `∀` IS `Term::Pi`; there is no separate `Forall`. The result sort comes
/// from `imax`, so quantification over any domain — `Prop` included — still lands in `Prop`.
/// That is impredicativity, and the prelude's own `dne` depends on it.
#[test]
fn universal_quantification_is_pi_and_its_sort_comes_from_imax() {
    let ctx = std_ctx();

    let is_prop = |t: &Term, what: &str| {
        let sort = infer_type(&ctx, t).unwrap_or_else(|e| panic!("{what} must typecheck: {e:?}"));
        assert!(
            matches!(sort, Term::Sort(Universe::Prop)),
            "{what} must land in Prop by `imax`, not `{sort}` — computing `max` instead \
             promotes a proposition out of Prop and silently disables proof irrelevance"
        );
    };

    // Quantification over `Prop` itself — the shape of `dne`'s own type.
    is_prop(
        &pi("P", prop(), arrow(var("P"), var("P"))),
        "`∀ p : Prop, p → p`",
    );
    // Quantification over `Type 0`, whose own type is `Type 1`. `imax (Type 1) Prop = Prop`.
    is_prop(
        &pi(
            "a",
            type0(),
            pi("x", var("a"), eq(var("a"), var("x"), var("x"))),
        ),
        "`∀ α : Type, ∀ x : α, x = x`",
    );
    // A plain dependent `∀` over data.
    is_prop(
        &pi("n", g("Nat"), eq(g("Nat"), var("n"), var("n"))),
        "`∀ n : Nat, n = n`",
    );
    // Nested binders sharing a name: legal in Lean (de Bruijn), and the inner must shadow.
    is_prop(
        &pi(
            "n",
            g("Nat"),
            pi("n", g("Nat"), eq(g("Nat"), var("n"), var("n"))),
        ),
        "`∀ n : Nat, ∀ n : Nat, n = n`",
    );
    // A `∀` whose DOMAIN is itself a `∀`, so lowering recurses into `param_type`.
    is_prop(
        &arrow(
            pi("n", g("Nat"), eq(g("Nat"), var("n"), var("n"))),
            eq(g("Nat"), nat(0), nat(0)),
        ),
        "`(∀ n, n = n) → 0 = 0`",
    );
    // Alternating `∀`/`∃`, so the emitted `Pi` and the `Ex` predicate lambda interleave.
    is_prop(
        &pi(
            "n",
            g("Nat"),
            ex(
                g("Nat"),
                lam(
                    "m",
                    g("Nat"),
                    pi(
                        "k",
                        g("Nat"),
                        and(
                            eq(g("Nat"), var("m"), var("n")),
                            eq(g("Nat"), var("k"), var("k")),
                        ),
                    ),
                ),
            ),
        ),
        "`∀ n, ∃ m, ∀ k, m = n ∧ k = k`",
    );

    // The shadowed binder must really be inhabited, not merely well-sorted — this is where a
    // named representation loses to de Bruijn if it does not freshen.
    let shadowed = lam("n", g("Nat"), lam("n", g("Nat"), refl(g("Nat"), var("n"))));
    assert_proves(
        &ctx,
        &shadowed,
        &pi(
            "n",
            g("Nat"),
            pi("n", g("Nat"), eq(g("Nat"), var("n"), var("n"))),
        ),
        "an inhabitant of `∀ n, ∀ n, n = n` (the inner binder shadows)",
    );

    // A non-dependent `∀` — implication — is the ONLY place `param == "_"` is right.
    let imp = lam("h", g("True"), var("h"));
    assert_proves(
        &ctx,
        &imp,
        &arrow(g("True"), g("True")),
        "implication as a non-dependent Pi",
    );
}

// ============================================================================
// D. THE CLASSICAL BASE — one axiom, and everything that is not derivable from it.
// ============================================================================

/// T06-008 · MUST · Classical logic rests on Lean's own classical base: `dne` is a THEOREM,
/// derived — as Lean derives `Classical.em` — from `Classical.choice` with `propext` and the
/// quotient primitives, so it costs no axiom of its own. Excluded middle, `byContradiction`, De
/// Morgan and case analysis all derive from it — nothing here may mint another axiom.
#[test]
fn excluded_middle_is_derived_from_dne_and_not_axiomatised() {
    let mut ctx = std_ctx();

    // `dne : Π(P:Prop). ((P → False) → False) → P`, exactly — and it is a theorem.
    let dne_ty = pi(
        "P",
        prop(),
        arrow(arrow(arrow(var("P"), g("False")), g("False")), var("P")),
    );
    let have = ctx
        .get_global("dne")
        .unwrap_or_else(|| panic!("`dne` must be in the prelude"))
        .clone();
    assert!(
        defeq_for_test(&ctx, &have, &dne_ty),
        "`dne` must be `Π(P:Prop). ¬¬P → P`, but it is `{have}`"
    );
    assert!(
        ctx.is_definition("dne") && ctx.get_definition_body("dne").is_some(),
        "`dne` is a checked theorem with a body — derived, not assumed"
    );
    assert!(
        !ctx.iter_declarations().any(|(name, _)| name == "dne"),
        "`dne` is not an axiom of the trusted base"
    );
    // It rests on Lean's classical axioms, as `#print axioms Classical.em` lists `propext` and
    // `Classical.choice` (with `Quot.sound`), and on nothing outside the foundational base.
    let rests_on = axiom_dependencies_of(&ctx, "dne")
        .expect("`dne` is registered")
        .axioms;
    for lean_axiom in ["Classical_choice", "propext"] {
        assert!(
            rests_on.contains(lean_axiom),
            "`dne` is derived from Lean's `{lean_axiom}`: {rests_on:?}"
        );
    }
    let foundational = [
        "Quot",
        "Quot_mk",
        "Quot_lift",
        "Quot_ind",
        "Quot_sound",
        "propext",
        "Classical_choice",
    ];
    assert!(
        rests_on
            .iter()
            .all(|axiom| foundational.contains(&axiom.as_str())),
        "`dne` rests on the foundational base alone: {rests_on:?}"
    );

    // `¬¬p` spelled with the `Not` global must reach `dne`'s arrow-chain type through delta.
    let dne_via_not = lam(
        "P",
        prop(),
        lam(
            "h",
            not(not(var("P"))),
            apps(g("dne"), vec![var("P"), var("h")]),
        ),
    );
    assert_proves(
        &ctx,
        &dne_via_not,
        &pi("P", prop(), arrow(not(not(var("P"))), var("P"))),
        "`dne` applied to a `¬¬p` written with the `Not` global",
    );

    // Double-negation INTRODUCTION is intuitionistic — it must not route through `dne`.
    let dni = lam(
        "P",
        prop(),
        lam(
            "hp",
            var("P"),
            lam("k", not(var("P")), app(var("k"), var("hp"))),
        ),
    );
    assert_proves(
        &ctx,
        &dni,
        &pi("P", prop(), arrow(var("P"), not(not(var("P"))))),
        "`¬¬`-introduction (intuitionistic, no classical axiom)",
    );

    // Excluded middle, DERIVED. Both kernels must accept the derivation.
    assert_proves(
        &ctx,
        &classical_em_body(),
        &classical_em_type(),
        "`Classical.em` from `dne`",
    );

    // …and case analysis on it, at the `Prop` motive its eliminator allows. (The prelude
    // publishes Lean's `Classical_em` itself; this derivation is registered under its own name,
    // so the case analysis below runs on the derivation just checked.)
    ctx.add_definition(
        "emFromDne".to_string(),
        classical_em_type(),
        classical_em_body(),
    );
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:?}",
        ctx.refused_declarations()
    );
    with_recursor(&mut ctx, "Or");
    let by_cases = lam(
        "p",
        prop(),
        lam(
            "q",
            prop(),
            lam(
                "hp",
                arrow(var("p"), var("q")),
                lam(
                    "hnp",
                    arrow(not(var("p")), var("q")),
                    apps(
                        g("Or_rec"),
                        vec![
                            var("p"),
                            not(var("p")),
                            lam("_", or(var("p"), not(var("p"))), var("q")),
                            var("hp"),
                            var("hnp"),
                            app(g("emFromDne"), var("p")),
                        ],
                    ),
                ),
            ),
        ),
    );
    let by_cases_ty = pi(
        "p",
        prop(),
        pi(
            "q",
            prop(),
            arrow(
                arrow(var("p"), var("q")),
                arrow(arrow(not(var("p")), var("q")), var("q")),
            ),
        ),
    );
    assert_proves(
        &ctx,
        &by_cases,
        &by_cases_ty,
        "`Classical.byCases` from derived `em`",
    );

    // Nothing above may have grown the trusted base.
    let base = std_ctx();
    let added: Vec<&str> = ctx
        .iter_declarations()
        .map(|(n, _)| n)
        .filter(|n| !base.iter_declarations().any(|(m, _)| m == *n))
        .collect();
    assert!(
        added.is_empty(),
        "deriving classical logic must add ZERO axioms, but these appeared: {added:?}"
    );
}

/// T06-009 · MUST · `Classical.choice` is one of Lean 4's three core axioms and essentially
/// all of Mathlib depends on it. `Ex`'s derived eliminator is `Prop`-motived, so no `Type`-level
/// witness escapes without it — and the prelude declares it, as Lean does. That reaches parity
/// ONLY because it is TRACKED: an axiom a translator invents with `add_declaration` is a silent
/// trusted-base expansion, and the alarm for it must be a named refusal at re-check time. The
/// demonstration invents Lean's bare `choice`, a name the kernel's closed registry of axioms
/// outside the trusted base knows (`type_checker::UNTRUSTED_AXIOMS`); the real `Classical_choice`
/// is a legitimate member of the trusted base. An arbitrary unknown name stays an unbound
/// variable — it is overwhelmingly a typo, and calling it an axiom would train the reader to
/// ignore the message that matters.
#[test]
fn choice_is_a_tracked_axiom_not_an_untracked_context_edit() {
    let ctx = std_ctx();

    // First, the demonstration that the danger is real. A translator's one-line edit
    // typechecks locally, and the SECOND de Bruijn kernel agrees with the first — the
    // double-kernel guarantee gives zero protection against an added axiom.
    let mut invented = std_ctx();
    invented.add_declaration("choice", pi("A", type0(), var("A")));
    assert!(
        invented.refused_declarations().is_empty(),
        "the trusted base has no `choice`"
    );
    let smuggled = app(g("choice"), g("Nat"));
    assert!(
        infer_type(&invented, &smuggled).is_ok(),
        "an invented axiom typechecks in the translator's own context — that is the hazard"
    );
    assert!(
        matches!(double_check(&invented, &smuggled), DoubleCheck::Agreed),
        "both kernels accept a term built on an invented axiom: two kernels do not protect \
         the trusted base, only rebuilding it does"
    );

    // Re-checking against a rebuilt prelude IS the alarm, and it fires.
    let err = match recheck_against_a_rebuilt_prelude(&smuggled) {
        Err(e) => e,
        Ok(t) => panic!(
            "re-checking rebuilds the trusted context from `StandardLibrary::register`, so a \
             proof citing an invented axiom must NOT re-check — it inferred `{t}`"
        ),
    };
    // …but it must say what it means. A bare `UnboundVariable` reads like a name-resolution
    // bug and invites the one catastrophic repair: adding the name to the prelude.
    assert!(
        !matches!(err, KernelError::UnboundVariable(_)),
        "a proof resting on an axiom outside the trusted base must be reported as exactly \
         that, not as `{err}` — an unbound-variable message invites patching the prelude, \
         which grows the trusted base while `PRELUDE_VERSION` still names the old one"
    );
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("axiom") && msg.contains("'choice'"),
        "the re-check diagnostic must name the offending axiom and say it is not in the \
         trusted base: `{err}`"
    );

    // Now the capability. `Nonempty α` is a Prop and `α` is data; eliminating one into the
    // other IS choice, and it must be a declared, well-typed axiom of the trusted base.
    assert!(
        ctx.get_global("Nonempty").is_some(),
        "`Nonempty` must exist — it is the domain of `Classical.choice`"
    );
    let choice = ctx
        .get_global("Classical_choice")
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "`Classical.choice` must be in the trusted base: it is one of Lean 4's three core \
             axioms, and `#print axioms` on almost any Mathlib theorem lists it"
            )
        });
    let choice_ty = pi("A", type0(), arrow(app(g("Nonempty"), var("A")), var("A")));
    assert!(
        defeq_for_test(&ctx, &choice, &choice_ty),
        "`Classical.choice` must be `Π(A:Type). Nonempty A → A`, but it is `{choice}`"
    );

    // …and tracked. Parity with Lean is the axiom PLUS the report that names it.
    let mut repl = Repl::new();
    repl.execute("Definition pickNat : Nat := Classical_choice Nat nonemptyNat.")
        .unwrap_or_else(|e| panic!("a proof may use `Classical.choice`: {e}"));
    let listed = match repl.execute("Print Axioms pickNat.") {
        Ok(out) => out,
        Err(e) => panic!(
            "the axioms a declaration rests on must be reportable, the way `#print axioms` \
             is — `Print Axioms pickNat.` failed with {e}"
        ),
    };
    assert!(
        listed.contains("Classical_choice"),
        "the axiom report must name `Classical_choice`: `{listed}`"
    );
}

/// T06-018 · MUST · The `Prop`-only classical fragment (`byCases`, `not_forall`) derives from
/// `dne` and is asserted here to work. Everything that produces DATA — `propDecidable`,
/// Skolemisation, the blanket `Decidable` instance — needs choice, and LOGOS blocks it
/// structurally rather than by name, which is the right design and also the gap.
#[test]
fn classical_reasoning_past_dne_is_available_and_never_smuggled_in() {
    let mut ctx = std_ctx();
    ctx.add_definition(
        "Classical_em".to_string(),
        classical_em_type(),
        classical_em_body(),
    );
    with_recursor(&mut ctx, "Or");

    // The firewall: `p ∨ ¬p` cannot be eliminated into `Decidable p`, because `Or` has two
    // constructors and so is not a subsingleton. Keying on the MOTIVE's sort — not on a
    // `Classical.` name prefix — is what makes this structural.
    let smuggle = lam(
        "p",
        prop(),
        apps(
            g("Or_rec"),
            vec![
                var("p"),
                not(var("p")),
                lam(
                    "_",
                    or(var("p"), not(var("p"))),
                    app(g("Decidable"), var("p")),
                ),
                lam("hp", var("p"), apps(g("isTrue"), vec![var("p"), var("hp")])),
                lam(
                    "hn",
                    not(var("p")),
                    apps(g("isFalse"), vec![var("p"), var("hn")]),
                ),
            ],
        ),
    );
    assert!(
        infer_type(&ctx, &smuggle).is_err(),
        "turning `p ∨ ¬p` into `Decidable p` crosses Prop→Type and needs CHOICE, not DNE — \
         it must stay blocked by the eliminator's Prop-pinned motive"
    );

    // De Morgan's classical direction, re-proved from `dne` rather than transcribed from
    // Lean's proof term (which drags `Classical.choice` in through `propDecidable`).
    let de_morgan_ty = pi(
        "p",
        prop(),
        pi(
            "q",
            prop(),
            arrow(
                not(and(var("p"), var("q"))),
                or(not(var("p")), not(var("q"))),
            ),
        ),
    );
    let de_morgan = lam(
        "p",
        prop(),
        lam(
            "q",
            prop(),
            lam(
                "h",
                not(and(var("p"), var("q"))),
                apps(
                    g("Or_rec"),
                    vec![
                        var("p"),
                        not(var("p")),
                        lam(
                            "_",
                            or(var("p"), not(var("p"))),
                            or(not(var("p")), not(var("q"))),
                        ),
                        lam(
                            "hp",
                            var("p"),
                            apps(
                                g("right"),
                                vec![
                                    not(var("p")),
                                    not(var("q")),
                                    lam(
                                        "hq",
                                        var("q"),
                                        app(
                                            var("h"),
                                            apps(
                                                g("conj"),
                                                vec![var("p"), var("q"), var("hp"), var("hq")],
                                            ),
                                        ),
                                    ),
                                ],
                            ),
                        ),
                        apps(g("left"), vec![not(var("p")), not(var("q"))]),
                        app(g("Classical_em"), var("p")),
                    ],
                ),
            ),
        ),
    );
    assert_proves(
        &ctx,
        &de_morgan,
        &de_morgan_ty,
        "De Morgan re-proved from `dne`",
    );

    // THE GAP. `Classical.propDecidable` — the step from `em` to DATA — is the constant that
    // makes choice strictly stronger than `dne`, and `Classical.axiomOfChoice`/`skolem`
    // produce a FUNCTION, so they need `funext` as well (T06-011). All three must exist, and
    // the diagnostic for the fragment that is unreachable must distinguish "needs choice"
    // from "needs a Decidable instance the user can supply".
    assert!(
        ctx.get_global("Classical_propDecidable").is_some(),
        "`Classical.propDecidable : Π(p:Prop). Decidable p` must exist — `by classical` and \
         every `open Classical in` in Mathlib rests on it"
    );
    assert!(
        ctx.get_global("Classical_axiomOfChoice").is_some(),
        "`Classical.axiomOfChoice` must exist — Skolemisation produces a function, so the \
         Prop-only `dne` fragment cannot reach it"
    );
}

// ============================================================================
// E. THE TWO MISSING CORE AXIOMS, AND THE MISSING RELATION.
// ============================================================================

/// T06-010 · MUST · `propext` is Lean's second core axiom. It must be stateable — its
/// CONCLUSION is an equation between two `Prop`s, so `Eq`'s carrier must reach `Prop` — declared
/// with exactly Lean's type, and TRACKED: a theorem proved with it reports it.
#[test]
fn propositional_extensionality_is_stateable_and_tracked() {
    let ctx = std_ctx();

    // The statement must be expressible before the axiom can be stated at all.
    let prop_equation = eq(prop(), g("True"), g("True"));
    if let Err(e) = infer_type(&ctx, &prop_equation) {
        let msg = e.to_string();
        assert!(
            msg.contains("Eq"),
            "`propext`'s conclusion is an equation between propositions; rejecting it must \
             name `Eq`'s carrier restriction rather than report `{msg}`, which reads like a \
             universe bug and invites widening `Eq` by re-registering it or by passing a hole"
        );
    }

    // `Iff` must exist for the hypothesis (T06-005), and `propext` for the conclusion.
    assert!(
        ctx.get_global("Iff").is_some(),
        "`propext`'s hypothesis is `p ↔ q`; without `Iff` the axiom cannot be stated"
    );
    let propext = ctx.get_global("propext").cloned().unwrap_or_else(|| {
        panic!(
            "`propext` must be in the trusted base: it is one of Lean 4's three core axioms, \
             and `Set`, quotient extensionality and `simp`'s handling of `↔` all rest on it"
        )
    });
    let propext_ty = pi(
        "p",
        prop(),
        pi(
            "q",
            prop(),
            arrow(
                apps(g("Iff"), vec![var("p"), var("q")]),
                eq(prop(), var("p"), var("q")),
            ),
        ),
    );
    assert!(
        defeq_for_test(&ctx, &propext, &propext_ty),
        "`propext` must be `Π(p q:Prop). (p ↔ q) → p = q`, but it is `{propext}`"
    );

    // Widening `Eq` must NOT have been done by re-registering it: the already-derived
    // `Eq_rec_dep`/`Eq_rec`/`Eq_sym`/`Eq_trans` carry bodies elaborated against the original,
    // so a second registration leaves the environment internally inconsistent.
    for name in ["Eq_rec_dep", "Eq_rec", "Eq_sym", "Eq_trans"] {
        let body = ctx
            .get_definition_body(name)
            .unwrap_or_else(|| panic!("`{name}` must still be a prelude definition"));
        assert!(
            infer_type(&ctx, body).is_ok(),
            "`{name}`'s body must still typecheck — a re-registered `Eq` silently invalidates \
             every lemma already elaborated against the first registration"
        );
    }

    // And tracked: a theorem proved by extensionality must say so.
    let mut repl = Repl::new();
    repl.execute(
        "Definition propextUser : (Iff True True) -> (Eq Prop True True) := propext True True.",
    )
    .expect("`propext True True` is a theorem about `True`, discharged from an `Iff`");
    let listed = match repl.execute("Print Axioms propextUser.") {
        Ok(out) => out,
        Err(e) => panic!(
            "a `propext`-using theorem must be able to report the axiom, the way \
             `#print axioms` does — `Print Axioms` failed with {e}"
        ),
    };
    assert!(
        listed.contains("propext"),
        "the axiom report must name `propext`: `{listed}`"
    );
    // The report is a graph walk, not a session-wide scan: a theorem that does not use
    // extensionality does not name it.
    repl.execute("Definition cleanThm : Eq Nat Zero Zero := refl Nat Zero.")
        .expect("a clean theorem in the same session");
    let clean = repl
        .execute("Print Axioms cleanThm.")
        .expect("reportable too");
    assert!(
        !clean.contains("propext"),
        "`cleanThm` rests on nothing: `{clean}`"
    );
}

/// T06-011 · MUST · `funext` is Lean's third core axiom. The tempting belief is that LOGOS
/// already has it because `def_eq` compares lambdas pointwise — it does not, and the
/// discriminating instance is asserted here in the negative direction first.
#[test]
fn function_extensionality_is_available_where_normalisation_cannot_reach() {
    let mut ctx = std_ctx();

    // THE DISCRIMINATOR. `f` and `id` agree pointwise only because of a HYPOTHESIS, which
    // `normalize` cannot use. The `Lambda`/`Lambda` arm of `def_eq` is alpha-equivalence, not
    // extensionality, and it must stay that way — anything that closes this is funext.
    ctx.add_declaration("addz", pi("n", g("Nat"), g("Nat")));
    ctx.add_declaration(
        "addz_spec",
        pi(
            "n",
            g("Nat"),
            eq(g("Nat"), app(g("addz"), var("n")), var("n")),
        ),
    );
    let f = lam("n", g("Nat"), app(g("addz"), var("n")));
    let id = lam("n", g("Nat"), var("n"));
    assert!(
        !defeq_for_test(&ctx, &f, &id),
        "`fun n => addz n` and `fun n => n` are equal only by a hypothesis, never \
         definitionally — `def_eq` must not pretend to be extensional"
    );

    // The axiom, in its DEPENDENT form. A monomorphic `(Nat → Nat)` version would be a
    // different, strictly weaker axiom AND would still have expanded the trusted base.
    let funext = ctx.get_global("funext").cloned().unwrap_or_else(|| {
        panic!(
            "`funext` must be in the trusted base: it is one of Lean 4's three core axioms, \
             and every `ext` lemma and every `Set`/`Finset`/`Matrix` equality in Mathlib \
             goes through it"
        )
    });
    let fam = || pi("_x", var("A"), type0());
    let dep_fn = || pi("x", var("A"), app(var("B"), var("x")));
    let funext_ty = pi(
        "A",
        type0(),
        pi(
            "B",
            fam(),
            pi(
                "f",
                dep_fn(),
                pi(
                    "g",
                    dep_fn(),
                    arrow(
                        pi(
                            "x",
                            var("A"),
                            eq(
                                app(var("B"), var("x")),
                                app(var("f"), var("x")),
                                app(var("g"), var("x")),
                            ),
                        ),
                        eq(dep_fn(), var("f"), var("g")),
                    ),
                ),
            ),
        ),
    );
    assert!(
        defeq_for_test(&ctx, &funext, &funext_ty),
        "`funext` must be the DEPENDENT axiom `Π(A)(B:A→Type)(f g:Π(x:A). B x). \
         (Π x, f x = g x) → f = g`, but it is `{funext}`"
    );

    // With it, the discriminating instance closes — and it must close to the real equation.
    let pointwise = apps(
        g("funext"),
        vec![
            g("Nat"),
            lam("_x", g("Nat"), g("Nat")),
            f.clone(),
            id.clone(),
            g("addz_spec"),
        ],
    );
    assert_proves(
        &ctx,
        &pointwise,
        &eq(arrow(g("Nat"), g("Nat")), f, id),
        "`funext` closing a pointwise equality that is not definitional",
    );

    // Substituting `Quot.sound` for `funext` is not a repair: `Quot A r` is an OPAQUE
    // declaration, so a quotiented function can no longer be applied at all.
    assert!(
        !ctx.is_inductive("Quot") && !ctx.is_definition("Quot"),
        "`Quot` is an opaque axiom — quotienting a function type by pointwise equality \
         changes its type and loses application; it is not a substitute for `funext`"
    );
}

/// T06-016 · MUST · `Eq` takes ONE carrier, so `HEq` cannot be spelled as an `Eq` at either side:
/// heterogeneous equality must be its own inductive, relating values at TWO carriers, with
/// `eq_of_heq` a PROVEN bridge back to `Eq` at a common carrier. The prelude has exactly that, and
/// an ad-hoc declaration can neither replace it nor be a dead end: re-registering `HEq` over the
/// prelude's leaves the name meaning what the bridge was proved against, and an invented copy
/// under a fresh name is an ordinary indexed family whose eliminator derives.
#[test]
fn heterogeneous_equality_exists_and_bridges_to_eq_at_a_common_carrier() {
    let ctx = std_ctx();

    // The ad-hoc workaround, over the prelude's own `HEq`.
    let mut adhoc = std_ctx();
    adhoc.add_inductive(
        "HEq",
        pi(
            "A",
            type0(),
            pi("B", type0(), arrow(var("A"), arrow(var("B"), prop()))),
        ),
    );
    adhoc.add_constructor(
        "HEq_refl",
        "HEq",
        pi(
            "A",
            type0(),
            pi(
                "a",
                var("A"),
                apps(g("HEq"), vec![var("A"), var("A"), var("a"), var("a")]),
            ),
        ),
    );
    assert!(
        infer_type(&adhoc, &apps(g("HEq_refl"), vec![g("Nat"), nat(3)])).is_ok(),
        "`HEq_refl Nat 3` typechecks"
    );
    // It is the IDENTICAL declaration, so it changed nothing: the name still means what the
    // prelude's `eq_of_heq` was proved against, and that proof still checks.
    assert!(
        adhoc.refused_declarations().is_empty(),
        "{:?}",
        adhoc.refused_declarations()
    );
    let bridge_body = adhoc
        .get_definition_body("eq_of_heq")
        .cloned()
        .expect("the bridge is proved");
    assert!(
        infer_type(&adhoc, &bridge_body).is_ok(),
        "a re-registered `HEq` must never invalidate the bridge already proved against it"
    );
    // An invented heterogeneous equality under a fresh name is no dead end either: it is an
    // ordinary indexed family, so its eliminator derives and it is not an axiom in all but name.
    adhoc.add_inductive(
        "MyHEq",
        pi(
            "A",
            type0(),
            pi("B", type0(), arrow(var("A"), arrow(var("B"), prop()))),
        ),
    );
    adhoc.add_constructor(
        "MyHEq_refl",
        "MyHEq",
        pi(
            "A",
            type0(),
            pi(
                "a",
                var("A"),
                apps(g("MyHEq"), vec![var("A"), var("A"), var("a"), var("a")]),
            ),
        ),
    );
    let (_, invented_rec) =
        derive_recursor(&adhoc, "MyHEq").expect("an invented `HEq` derives its eliminator");
    assert_eq!(
        double_check(&adhoc, &invented_rec),
        DoubleCheck::Agreed,
        "both kernels certify the invented family's eliminator"
    );

    // The capability. `HEq` must be in the prelude, with both carriers in its statement.
    let heq = ctx.get_global("HEq").cloned().unwrap_or_else(|| {
        panic!(
            "`HEq` must exist: Lean's equation compiler, `Eq.rec` on indexed families, and \
             `cases`/`injection` on a dependent hypothesis all GENERATE heterogeneous \
             equations that the source never types"
        )
    });
    let heq_ty = pi(
        "A",
        type0(),
        pi("B", type0(), arrow(var("A"), arrow(var("B"), prop()))),
    );
    assert!(
        defeq_for_test(&ctx, &heq, &heq_ty),
        "`HEq` must relate values at TWO carriers: `Π(A B:Type). A → B → Prop`, not `{heq}`"
    );

    // `eq_of_heq` is a THEOREM, not a coercion — it must be provable, not assumed.
    assert!(
        ctx.is_definition("eq_of_heq"),
        "`eq_of_heq : HEq A A a b → Eq A a b` must be a proven definition; assuming it makes \
         the bridge between the two equalities an axiom rather than a theorem"
    );
    let bridge = apps(
        g("eq_of_heq"),
        vec![
            g("Nat"),
            nat(3),
            nat(3),
            apps(g("HEq_refl"), vec![g("Nat"), nat(3)]),
        ],
    );
    assert_proves(
        &ctx,
        &bridge,
        &eq(g("Nat"), nat(3), nat(3)),
        "`eq_of_heq` at a common carrier",
    );
}

// ============================================================================
// F. QUOTIENTS.
// ============================================================================

/// T06-012 · MUST · The five `Quot` primitives ARE here — the one place LOGOS's trusted base
/// overlaps Lean's — and the `Quot_lift ∘ Quot_mk` computation rule really fires. What is
/// missing is the dependent eliminator into `Type`, the `Setoid`/`Quotient` wrapper, and any
/// carrier above `Type 0`.
#[test]
fn a_quotient_computes_lifts_data_and_carries_a_setoid_wrapper() {
    let mut ctx = std_ctx();

    // All five are declarations — axioms — with `Quot A r` OPAQUE, not an inductive. A
    // translator that declares it as an inductive with one constructor makes `Quot_sound`
    // inconsistent: `Match` would distinguish classes that `Quot_sound` asserts are equal.
    for name in ["Quot", "Quot_mk", "Quot_lift", "Quot_sound", "Quot_ind"] {
        assert!(
            ctx.get_global(name).is_some(),
            "`{name}` must be in the prelude under this exact underscored spelling"
        );
        assert!(
            !ctx.is_inductive(name) && !ctx.is_definition(name),
            "`{name}` must stay an opaque axiom — making `Quot` an inductive would let \
             `Match` distinguish classes that `Quot_sound` identifies"
        );
    }

    // `Quot_sound` at a concrete relation, kernel-verified.
    let rel = lam(
        "a",
        g("Nat"),
        lam("b", g("Nat"), eq(g("Nat"), var("a"), var("b"))),
    );
    let mk = |n: Term| apps(g("Quot_mk"), vec![g("Nat"), rel.clone(), n]);
    let quot_ty = apps(g("Quot"), vec![g("Nat"), rel.clone()]);
    let sound = apps(
        g("Quot_sound"),
        vec![
            g("Nat"),
            rel.clone(),
            nat(2),
            nat(2),
            refl(g("Nat"), nat(2)),
        ],
    );
    assert_proves(
        &ctx,
        &sound,
        &eq(quot_ty.clone(), mk(nat(2)), mk(nat(2))),
        "`Quot.sound`",
    );

    // THE COMPUTATION RULE — the only reduction quotients have.
    ctx.add_declaration(
        "resp",
        pi(
            "a",
            g("Nat"),
            pi(
                "b",
                g("Nat"),
                arrow(
                    eq(g("Nat"), var("a"), var("b")),
                    eq(g("Nat"), var("a"), var("b")),
                ),
            ),
        ),
    );
    let lifted = apps(
        g("Quot_lift"),
        vec![
            g("Nat"),
            rel.clone(),
            g("Nat"),
            lam("n", g("Nat"), var("n")),
            g("resp"),
            mk(nat(2)),
        ],
    );
    assert_eq!(
        normalize(&ctx, &lifted).to_string(),
        nat(2).to_string(),
        "`Quot_lift … f h (Quot_mk … a)` must reduce to `f a`"
    );

    // THE GAPS. Lean's `Quot.rec` eliminates into `Sort v`; LOGOS's `Quot_ind` motive is
    // `Quot A r → Prop`, so a function DEFINED by cases on a quotient has no counterpart.
    assert!(
        ctx.get_global("Quot_rec").is_some(),
        "`Quot.rec` — the DEPENDENT eliminator into `Sort v` — must exist; `Quot_ind`'s \
         Prop-only motive cannot define a function on a quotient by cases"
    );
    // …and the `Setoid`/`Quotient` layer Lean's whole quotient API is phrased in.
    assert!(
        ctx.get_global("Setoid").is_some(),
        "`Setoid` must exist — Lean's `Quotient s` takes its relation from a `Setoid` instance"
    );
    assert!(
        ctx.get_global("Quotient").is_some(),
        "`Quotient` must exist — silently unfolding it to `Quot s.r` drops the instance that \
         was resolved at translation time and is no longer resolvable at check time"
    );

    // …and a carrier above `Type 0`.
    let big_rel = lam("a", type0(), lam("b", type0(), g("True")));
    match infer_type(&ctx, &apps(g("Quot"), vec![type0(), big_rel])) {
        Ok(sort) => assert!(
            matches!(sort, Term::Sort(Universe::Type(1))),
            "a quotient of a `Type 0` carrier must live in `Type 1`, not `{sort}`"
        ),
        Err(e) => panic!(
            "Lean's `Quot` is `{{α : Sort u}}`; quotienting a `Type` must be possible: {e:?}"
        ),
    }
}

// ============================================================================
// G. DECIDABILITY AND THE Bool/Prop BRIDGE.
// ============================================================================

/// T06-013 · MUST · `Decidable` is a `Type 0`-valued indexed inductive and both `decide` and
/// `of_decide_eq_true` are PROVEN definitions — the bridge from a computed `Bool` back to a
/// `Prop` costs nothing in the trusted base. What is missing is instance resolution: the
/// prelude never calls `add_instance`, so the instance table is empty even for `Nat`.
#[test]
fn decidable_equality_resolves_as_an_instance_for_nat_bool_and_user_types() {
    let ctx = std_ctx();

    // `Decidable` is `Type 0`-valued so it can be eliminated into `Type` to compute a `Bool`.
    // Placing it in `Prop` would make it a two-constructor non-subsingleton whose eliminator
    // is Prop-pinned, and `decide` would be unwritable.
    let dec_ty = ctx
        .get_global("Decidable")
        .cloned()
        .expect("`Decidable` is in the prelude");
    assert!(
        defeq_for_test(&ctx, &dec_ty, &pi("p", prop(), type0())),
        "`Decidable : Π(p:Prop). Type 0`, not `{dec_ty}`"
    );
    assert_eq!(
        ctx.inductive_num_params("Decidable"),
        1,
        "`Decidable` has exactly one parameter and no indices"
    );

    // Both bridges are DEFINITIONS. Axiomatising either is the trusted-base bug in miniature:
    // the computed side always evaluates to `true` in the cases anyone tests.
    for name in [
        "decide",
        "of_decide_eq_true",
        "Decidable_rec",
        "decEqNat",
        "decEqBool",
    ] {
        assert!(
            ctx.is_definition(name),
            "`{name}` must be a proven definition, never an axiom"
        );
    }

    // `decide` computes, in both directions.
    let dec_at = |a: u32, b: u32| {
        apps(
            g("decide"),
            vec![
                eq(g("Nat"), nat(a), nat(b)),
                apps(g("decEqNat"), vec![nat(a), nat(b)]),
            ],
        )
    };
    assert_eq!(
        normalize(&ctx, &dec_at(2, 2)).to_string(),
        "true",
        "`decide (2 = 2)` must reduce to `true`"
    );
    assert_eq!(
        normalize(&ctx, &dec_at(2, 3)).to_string(),
        "false",
        "`decide (2 = 3)` must reduce to `false`"
    );

    // The sound `Bool → Prop` direction, kernel-verified.
    let bridged = apps(
        g("of_decide_eq_true"),
        vec![
            eq(g("Nat"), nat(3), nat(3)),
            apps(g("decEqNat"), vec![nat(3), nat(3)]),
            refl(g("Bool"), g("true")),
        ],
    );
    assert_proves(
        &ctx,
        &bridged,
        &eq(g("Nat"), nat(3), nat(3)),
        "`of_decide_eq_true`",
    );

    // THE GAP. `decide p` takes its instance as an explicit argument, and the instance table
    // is EMPTY — `add_instance` is never called by the prelude, so even `DecidableEq Nat`
    // does not resolve. Every Lean `if a = b then … else …` needs this.
    let mut m = MetaCtx::default();
    assert!(
        resolve_instance(
            &ctx,
            &mut m,
            &app(g("Decidable"), eq(g("Nat"), nat(2), nat(2)))
        )
        .is_some(),
        "`decEqNat` must be registered as an INSTANCE so `Decidable (a = b)` resolves; the \
         prelude never calls `add_instance`, so today it does not"
    );
    let mut mb = MetaCtx::default();
    assert!(
        resolve_instance(
            &ctx,
            &mut mb,
            &app(g("Decidable"), eq(g("Bool"), g("true"), g("true")))
        )
        .is_some(),
        "`decEqBool` must be registered as an instance too"
    );

    // …and a user inductive must get decidable equality, as Lean's `deriving DecidableEq`
    // gives it. Lean's source shows nothing unusual, so a translator sees an instance name
    // that resolves to nothing.
    let mut enum_ctx = std_ctx();
    enum_ctx.add_inductive("Colour", type0());
    for c in ["Red", "Green", "Blue"] {
        enum_ctx.add_constructor(c, "Colour", g("Colour"));
    }
    let mut mc = MetaCtx::default();
    assert!(
        resolve_instance(
            &enum_ctx,
            &mut mc,
            &app(g("Decidable"), eq(g("Colour"), g("Red"), g("Blue")))
        )
        .is_some(),
        "declaring a finite inductive must yield decidable equality for it — Lean synthesises \
         this from `deriving DecidableEq` and there is no LOGOS equivalent"
    );
}

/// T06-014 · DIVERGE · Native. LOGOS carries the SAME two axioms as Lean's `native_decide`
/// (`reduceBool`, `ofReduceBool`) with the same types, and the bridge term kernel-checks in
/// both kernels. The divergence is WHAT stands behind them, and ours is the better story:
/// Lean's `native_decide` trusts the Lean compiler, its C code generator and the resulting
/// native object code, none of which an independent re-checker re-runs — the artefact is
/// believed, not re-verified. LOGOS's `reduceBool` is discharged by `eval_bool`, a CBV
/// evaluator inside `logicaffeine_kernel` itself, which is exactly the crate a standalone
/// re-check already trusts; so re-checking a `native_decide` certificate RE-RUNS the
/// evaluation rather than taking a compiled artefact's word for it. The trusted base is
/// smaller and, unusually for this feature, auditable in one crate. The cost is real and is
/// recorded: a proposition asserted by Lean's evaluator is not thereby asserted by ours, so a
/// `native_decide` proof is not portable between the two systems on its own terms.
#[test]
fn native_decide_runs_the_kernels_own_evaluator_under_two_declared_axioms() {
    let ctx = std_ctx();

    // Both are genuine axioms — unlike `decide`/`of_decide_eq_true`, which the prelude proves.
    for name in ["reduceBool", "ofReduceBool"] {
        assert!(
            ctx.get_global(name).is_some(),
            "`{name}` must be registered — it is the `native_decide` trust boundary"
        );
        assert!(
            !ctx.is_definition(name),
            "`{name}` must stay an opaque declaration: a delta-transparent `reduceBool` would \
             unfold to the identity BEFORE the evaluator arm of `reduce_step` is reached, and \
             the evaluator would never run"
        );
    }
    let of_rb = ctx.get_global("ofReduceBool").cloned().unwrap();
    let of_rb_ty = pi(
        "a",
        g("Bool"),
        pi(
            "b",
            g("Bool"),
            arrow(
                eq(g("Bool"), app(g("reduceBool"), var("a")), var("b")),
                eq(g("Bool"), var("a"), var("b")),
            ),
        ),
    );
    assert!(
        defeq_for_test(&ctx, &of_rb, &of_rb_ty),
        "`ofReduceBool` must have Lean's exact type `Π(a b:Bool). reduceBool a = b → a = b`, \
         but it is `{of_rb}`"
    );

    // The evaluator really is what closes the gap: `reduceBool t` evaluates `t`.
    let decided = apps(
        g("decide"),
        vec![
            eq(g("Nat"), nat(2), nat(2)),
            apps(g("decEqNat"), vec![nat(2), nat(2)]),
        ],
    );
    assert_eq!(
        normalize(&ctx, &app(g("reduceBool"), decided.clone())).to_string(),
        "true",
        "`reduceBool` must hand its argument to the trusted evaluator during kernel reduction"
    );

    // The whole `native_decide` bridge, kernel-verified in both kernels: the inner proof is
    // `refl Bool true`, and it only typechecks because the evaluator ran.
    let bridge = apps(
        g("ofReduceBool"),
        vec![decided.clone(), g("true"), refl(g("Bool"), g("true"))],
    );
    assert_proves(
        &ctx,
        &bridge,
        &eq(g("Bool"), decided.clone(), g("true")),
        "the `native_decide` bridge `ofReduceBool a true (rfl)`",
    );
    let proved = apps(
        g("of_decide_eq_true"),
        vec![
            eq(g("Nat"), nat(2), nat(2)),
            apps(g("decEqNat"), vec![nat(2), nat(2)]),
            bridge,
        ],
    );
    assert_proves(
        &ctx,
        &proved,
        &eq(g("Nat"), nat(2), nat(2)),
        "a proposition proved through the trusted evaluator",
    );

    // The divergence itself: what stands behind `reduceBool` must be re-run by an independent
    // re-check, not believed. Re-checking rebuilds the prelude and re-normalizes, so it does.
    assert!(
        recheck_against_a_rebuilt_prelude(&proved).is_ok(),
        "a `native_decide` proof must survive a re-check that rebuilds the trusted context \
         from `StandardLibrary::register` — the evaluator is inside the crate being trusted, \
         so the re-check RE-RUNS it instead of trusting a compiled artefact"
    );
}

// ============================================================================
// H. TRUNCATION.
// ============================================================================

/// T06-015 · MUST · Lean's `Subsingleton` is a `Prop`-valued CLASS with instances, and
/// `Squash : Type u → Type u` is propositional truncation that stays in `Type`. LOGOS has
/// neither, and its own subsingleton notion is a different thing entirely: a compile-time
/// predicate on an inductive DECLARATION, not addressable from a term.
#[test]
fn subsingleton_is_a_class_with_instances_and_squash_truncates_into_type() {
    let ctx = std_ctx();

    // The kernel's own notion, for contrast — it is a property of a declaration, read off the
    // constructor list, and it agrees with the class only for Prop inductives.
    let (false_rec_ty, _) = derive_recursor(&ctx, "False").expect("`False`'s eliminator derives");
    assert!(
        false_rec_ty.to_string().contains("Type0"),
        "a zero-constructor `Prop` is a subsingleton, so its motive reaches `Type 0`: {false_rec_ty}"
    );
    let (or_rec_ty, _) = derive_recursor(&ctx, "Or").expect("`Or`'s eliminator derives");
    assert!(
        !or_rec_ty.to_string().contains("Type0"),
        "a two-constructor `Prop` is NOT a subsingleton, so its motive stays in `Prop`: {or_rec_ty}"
    );

    // THE GAP. `Subsingleton` with its method, and a `Type`-level instance — proof
    // irrelevance does NOT prove `Subsingleton Unit`, because `Unit` is not a `Prop`, so the
    // kernel rule cannot stand in for the class.
    assert!(
        ctx.get_global("Subsingleton").is_some(),
        "`Subsingleton` must exist as a class — Mathlib derives instances of it constantly, \
         and the kernel's declaration-level notion is not addressable from a term"
    );
    assert!(
        ctx.get_global("Subsingleton_elim").is_some(),
        "`Subsingleton.elim` must be a real method: for `Unit`, a `Type`, proof irrelevance \
         does not apply and the proof must be given, not assumed from the kernel rule"
    );

    // …and the encoding route the corpus proposes is not a repair, because it changes the
    // SORT: `add_structure` hard-codes `Type 0`, so a `Prop`-valued class cannot be built
    // that way, and a `Type 0` `Subsingleton α` inside a proposition's statement promotes
    // that statement out of `Prop` through `imax`.
    let mut s = std_ctx();
    s.add_structure(
        "SubS",
        &[("a", type0())],
        &[(
            "elim",
            pi(
                "x",
                var("a"),
                pi("y", var("a"), eq(var("a"), var("x"), var("y"))),
            ),
        )],
    );
    let sub_sort = infer_type(&s, &app(g("SubS"), g("Unit"))).expect("the encoding typechecks");
    assert!(
        matches!(sub_sort, Term::Sort(Universe::Prop)),
        "`Subsingleton α` is a PROP-valued class in Lean; encoding it through `add_structure` \
         yields `{sub_sort}`, which silently promotes every proposition that mentions it"
    );

    // `Squash α` is a `Type`, not a `Prop`; substituting `Nonempty` changes exactly whether
    // you may eliminate into `Type`, which is choice.
    let squash = ctx.get_global("Squash").cloned().unwrap_or_else(|| {
        panic!("`Squash : Type → Type` must exist — truncation that stays in `Type`")
    });
    assert!(
        defeq_for_test(&ctx, &squash, &pi("A", type0(), type0())),
        "`Squash` must be `Type → Type`, not `{squash}` — a `Prop`-valued truncation is \
         `Nonempty`, and eliminating that into `Type` is choice"
    );
}

// ============================================================================
// I. ADMISSION AND TAINT — the most dangerous case in the tier.
// ============================================================================

/// T06-017 · MUST · `sorry` must never reach the context, and the protection cannot be a
/// per-declaration warning: Lean warns on the declaration that CONTAINS the `sorry`, not on
/// its dependants, so the taint must be the transitive closure of the elaborated environment
/// — which is what `#print axioms` reports. LOGOS has no such report at any surface, so a
/// theorem whose proof term cites a lemma that cites an admission is indistinguishable from
/// a clean one.
#[test]
fn an_admitted_proof_taints_every_theorem_that_transitively_cites_it() {
    // The hazard, demonstrated. One `add_declaration` is a proof of every proposition.
    let mut tainted = std_ctx();
    tainted.add_declaration("sorryAx", pi("A", type0(), var("A")));
    let false_equation = eq(g("Nat"), nat(2), nat(3));
    let admitted = app(g("sorryAx"), false_equation.clone());
    assert!(
        infer_type(&tainted, &admitted).is_ok(),
        "an admission is well-typed BY CONSTRUCTION — `verified == true` really does hold, \
         which is why the trust door cannot be the thing that catches it"
    );
    assert!(
        matches!(double_check(&tainted, &admitted), DoubleCheck::Agreed),
        "the SECOND de Bruijn kernel agrees as well: two kernels give zero protection against \
         an added axiom, because both are asked the same question"
    );

    // What DOES catch it: rebuilding the trusted context and re-checking.
    let err = match recheck_against_a_rebuilt_prelude(&admitted) {
        Err(e) => e,
        Ok(_) => panic!(
            "a proof citing an admission must not survive a re-check that rebuilds the trusted \
             context from `StandardLibrary::register`"
        ),
    };

    // …but it must say what it means. `UnboundVariable("sorryAx")` reads like a
    // name-resolution bug, and the obvious repair — adding the name to the prelude — is the
    // catastrophic one, because `PRELUDE_VERSION` would still describe the old axiom set.
    assert!(
        !matches!(err, KernelError::UnboundVariable(_)),
        "re-checking an admitted proof must report an INCOMPLETE PROOF, not `{err}` — an \
         unbound-variable message invites adding the name to the prelude, after which every \
         certificate ever minted claims an axiom set it was not checked against"
    );

    // THE TRANSITIVE CASE, which no warning can reach. `taintedUser`'s proof term is the bare
    // name `taintedLemma`; it never mentions the admission, and a re-check that fails names
    // the lemma rather than the admission behind it.
    let mut repl = Repl::new();
    repl.context_mut()
        .add_declaration("sorryAx", pi("A", type0(), var("A")));
    repl.execute("Definition taintedLemma : Eq Int 2 3 := sorryAx (Eq Int 2 3).")
        .expect("an admitted lemma registers — that is the hazard, not the test");
    repl.execute("Definition taintedUser : Eq Int 2 3 := taintedLemma.")
        .expect("a downstream theorem cites the lemma by name");
    repl.execute("Definition cleanThm : Eq Int 3 3 := refl Int 3.")
        .expect("a clean theorem in the same session");

    let body = repl
        .context_mut()
        .get_definition_body("taintedUser")
        .cloned()
        .expect("`taintedUser` is registered");
    assert_eq!(
        body.to_string(),
        "taintedLemma",
        "the downstream proof term never mentions the admission — a keyword or warning scan \
         cannot see it"
    );

    let listed = match repl.execute("Print Axioms taintedUser.") {
        Ok(out) => out,
        Err(e) => panic!(
            "the axioms a declaration transitively rests on must be reportable, the way \
             `#print axioms` is — without it a `sorry`-tainted theorem is indistinguishable \
             from a proved one. `Print Axioms taintedUser.` failed with {e}"
        ),
    };
    assert!(
        listed.contains("sorryAx"),
        "the report must follow the dependency graph of ELABORATED terms and name `sorryAx` \
         for `taintedUser`, which contains no `sorry` of its own: `{listed}`"
    );
    let clean = repl
        .execute("Print Axioms cleanThm.")
        .expect("the report works for a clean theorem too");
    assert!(
        !clean.contains("sorryAx"),
        "the taint must be exact, not file-wide: `cleanThm` rests on nothing: `{clean}`"
    );
}

// ============================================================================
// Z. THE TRUSTED-BASE CENSUS — a lock, not a case. Green today, and it must only ever
//    change by a deliberate edit that says which axiom was added and why.
// ============================================================================

#[test]
fn the_trusted_base_is_exactly_its_declared_axioms_and_nothing_foundational_is_smuggled_in() {
    let ctx = std_ctx();
    let mut names: Vec<&str> = ctx.iter_declarations().map(|(n, _)| n).collect();
    names.sort();

    // Every declaration is a well-typed assumption — an ill-typed axiom is worse than a
    // missing one, because it is invisible.
    for name in &names {
        assert!(
            infer_type(&ctx, &g(name)).is_ok(),
            "trusted-base axiom `{name}` must itself be well-typed"
        );
    }

    // The FOUNDATIONAL fragment, named exactly: Lean 4's three axioms (`propext`,
    // `Classical_choice`, `Quot_sound`, added deliberately by the classical and quotient waves —
    // `logos-coc-4`), the quotient primitives, and `native_decide`'s `reduceBool`/`ofReduceBool`
    // pair. `Quot_rec` is not here: Lean 4.14 DEFINES `Quot.rec` (from `Quot.lift` over a
    // dependent pair, resting on `Quot.sound`), and `logos-coc-6` defines ours the same way.
    for name in [
        "propext",
        "Classical_choice",
        "Quot",
        "Quot_mk",
        "Quot_lift",
        "Quot_sound",
        "Quot_ind",
        "reduceBool",
        "ofReduceBool",
    ] {
        assert!(
            names.contains(&name),
            "`{name}` is part of the declared foundational base and must be present"
        );
    }

    // Nothing Lean DERIVES may appear as an axiom. `dne` left the base in wave 4 (Diaconescu's
    // `em` from `Classical_choice`, `funext` and `propext`); the rest were never assumed. Each is a
    // definition, an inductive or absent — and re-assuming one must go red here.
    for name in [
        "dne",
        "funext",
        "Classical_em",
        "sorryAx",
        "HEq",
        "Setoid",
        "Quotient",
        "Subsingleton",
        "Squash",
        "Nonempty",
        "Iff",
        "Eq_rec_dep",
        "Eq_rec",
        "Eq_sym",
        "Eq_trans",
        "decide",
        "of_decide_eq_true",
        "Quot_rec",
        "PSigma",
    ] {
        assert!(
            !names.contains(&name),
            "`{name}` appeared in the TRUSTED BASE. If it was added deliberately, update this \
             census and say which wave earned it; if it is a derived lemma (`Eq_*`, `decide`, \
             `of_decide_eq_true`, `dne`, `funext`), it belongs in `add_definition`, not \
             `add_declaration`"
        );
    }

    assert_eq!(
        names.len(),
        63,
        "the trusted base holds 63 declared axioms — the nine above, the six `Float` operations, \
         thirty-seven reflection primitives and eleven temporal operations, the same set \
         `tcb_ledger_lock` classifies. A change here is a change to what LOGOS ASSUMES — update \
         this number deliberately, in the same commit as the axiom: {names:?}"
    );
}
