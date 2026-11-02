//! The structural types Lean core ships, held to a standard the corpus does not state.
//!
//! The corpus's tier-10 cases ask only whether a NAME is in the context. A name is not a type:
//! a `Prod` whose `fst` does not reduce, a `Fin` that accepts an out-of-range index, or a
//! `Nonempty` that large-eliminates into `Type` would all satisfy `get_global(..).is_some()`
//! and would all be worse than the absence they replaced. So every type registered for T10-012 /
//! T10-013 / T10-006 / T10-009 / T10-011 / T06-005 / T06-015 / T07-010 is pinned here by a
//! POSITIVE and a NEGATIVE: the thing it must now accept and compute, and the neighbouring thing
//! it must still refuse.
//!
//! Three invariants run through the whole file.
//!
//! 1. **Nothing here is an axiom.** Every type is an inductive, a structure or a definition with
//!    a body; the trusted base is unchanged. `the_structural_types_cost_the_trusted_base_nothing`
//!    is the lock.
//! 2. **`Nat` order is a computing `Bool`, not a second axiom set.** The prelude's `lt`/`le`
//!    primitives are `Int → Int → Bool` and every order axiom is about `Int`. `Fin`'s bound is
//!    instead the prelude's own shallow idiom — `Nat_lt i n` is `Eq Bool (natLt i n) true` — so
//!    the bound proof is `refl` when the bound holds and is UNINHABITED when it does not,
//!    decided by ι-reduction rather than assumed.
//! 3. **Both kernels or neither.** Each new type gets a `double_check` on a representative term.
//!    Three do not, and that is a finding rather than an omission: any element whose type needs a
//!    recursion over a `Nat` LITERAL (`Subtype`'s bound, `Fin`, `Char`) is accepted by the main
//!    kernel and STUCK in the re-checker, because `recheck.rs`'s `ctor_headed` does not know that
//!    `Literal::Nat(n)` is `Zero`/`Succ _`. That wall is isolated, with a pre-existing `decEqNat`
//!    reproduction, in `the_re_checker_computes_a_recursion_over_a_nat_literal`.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, double_check, infer_type, normalize, BigInt, Context, DoubleCheck, Literal,
    Term, Universe,
};

// ==== HELPERS ====

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
fn mat(d: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(motive),
        cases,
    }
}
fn type0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn nat() -> Term {
    g("Nat")
}
fn bool_ty() -> Term {
    g("Bool")
}
fn natl(n: i64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_i64(n)))
}
fn natl_dec(s: &str) -> Term {
    Term::Lit(Literal::Nat(
        BigInt::parse_decimal(s).expect("decimal Nat literal"),
    ))
}
fn eq_at(ty: Term, a: Term, b: Term) -> Term {
    apps(g("Eq"), &[ty, a, b])
}
fn refl(ty: Term, a: Term) -> Term {
    apps(g("refl"), &[ty, a])
}
/// `refl Bool true` — the canonical inhabitant of every `Nat_lt`/`Nat_le` bound that HOLDS.
fn bound_proof() -> Term {
    refl(bool_ty(), g("true"))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
/// `Nat_lt i n` — the shallow bound proposition a `Fin` element must discharge.
fn nat_lt(i: Term, n: Term) -> Term {
    apps(g("Nat_lt"), &[i, n])
}
/// `λ(i:Nat). Nat_lt i n` — the `Fin n` predicate, as `Subtype` sees it.
fn fin_pred(n: Term) -> Term {
    lam("i", nat(), nat_lt(var("i"), n))
}
/// The `Subtype` spelling of `⟨i, proof⟩ : Fin n`.
fn fin_mk(n: Term, i: Term, pf: Term) -> Term {
    apps(g("Subtype_mk"), &[nat(), fin_pred(n), i, pf])
}
/// `List A` built from a slice of elements.
fn list_of(elem_ty: Term, xs: &[Term]) -> Term {
    xs.iter()
        .rev()
        .fold(app(g("nil"), elem_ty.clone()), |acc, x| {
            apps(g("cons"), &[elem_ty.clone(), x.clone(), acc])
        })
}
/// Both kernels must agree that `t` is well typed — a fix that only the primary kernel accepts
/// is not a fix.
fn agreed(ctx: &Context, t: &Term, what: &str) {
    assert!(
        matches!(double_check(ctx, t), DoubleCheck::Agreed),
        "both kernels must accept {what}: {:?}",
        double_check(ctx, t)
    );
}

// ============================================================================================
// THE TRUSTED BASE — the lock that makes every other test in this file honest.
// ============================================================================================

/// Every structural type below is an inductive, a structure or a definition WITH A BODY. Not one
/// is `add_declaration`. The count is the live trusted-base size `TCB.md` reports (70 since the
/// `Int` laws became theorems and `Quot_rec` a definition), restated here so that a future "just
/// axiomatise `Array_get`" repair goes red in the file that made the promise.
#[test]
fn the_structural_types_cost_the_trusted_base_nothing() {
    let ctx = std_ctx();
    let axioms: Vec<&str> = ctx.iter_declarations().map(|(n, _)| n).collect();

    for name in [
        "Option",
        "some",
        "none",
        "Prod",
        "Prod_mk",
        "Prod_fst",
        "Prod_snd",
        "Sum",
        "Sigma",
        "Sigma_mk",
        "Sigma_fst",
        "Sigma_snd",
        "Subtype",
        "Subtype_mk",
        "Subtype_val",
        "Subtype_property",
        "Fin",
        "List",
        "nil",
        "cons",
        "List_rec",
        "List_length",
        "List_append",
        "List_get",
        "Array",
        "Array_mk",
        "Array_size",
        "Array_push",
        "Array_get",
        "String",
        "String_mk",
        "String_data",
        "Char",
        "UInt8",
        "UInt16",
        "UInt32",
        "UInt64",
        "USize",
        "Iff",
        "Subsingleton",
        "Subsingleton_elim",
        "Squash",
        "Inhabited",
        "Inhabited_mk",
        "Inhabited_default",
        "Nonempty",
        "Nonempty_intro",
        "natLe",
        "natLt",
        "Nat_le",
        "Nat_lt",
        "PSigma",
        "PSigma_mk",
        "PSigma_fst",
        "PSigma_snd",
    ] {
        assert!(
            ctx.get_global(name).is_some(),
            "`{name}` must be registered"
        );
        assert!(
            !axioms.contains(&name),
            "`{name}` was closed with an AXIOM. A declared type-former that never reduces makes \
             the capability gap invisible while leaving every `rfl` over it false"
        );
    }

    assert_eq!(
        axioms.len(),
        63,
        "registering Lean's structural types must not grow the trusted base by one: {axioms:?}"
    );
}

// ============================================================================================
// Nat ORDER — the bound `Fin`, `UInt*` and `Array_get` are built from.
// ============================================================================================

/// `natLe`/`natLt` decide by ι-reduction, in BOTH directions, and they are `Nat` relations — the
/// prelude's `le`/`lt` primitives are `Int → Int → Bool` and stay that way.
#[test]
fn nat_order_computes_in_both_directions_and_is_not_the_int_order() {
    let ctx = std_ctx();
    let le = |a: i64, b: i64| normalize(&ctx, &apps(g("natLe"), &[natl(a), natl(b)]));
    let lt = |a: i64, b: i64| normalize(&ctx, &apps(g("natLt"), &[natl(a), natl(b)]));

    assert_eq!(le(0, 0), g("true"), "0 ≤ 0");
    assert_eq!(le(2, 3), g("true"), "2 ≤ 3");
    assert_eq!(
        le(3, 3),
        g("true"),
        "3 ≤ 3 — `le` is reflexive at the boundary"
    );
    assert_eq!(le(3, 2), g("false"), "3 ≰ 2");
    assert_eq!(lt(2, 3), g("true"), "2 < 3");
    assert_eq!(
        lt(3, 3),
        g("false"),
        "3 ≮ 3 — the boundary is the whole point of a Fin bound"
    );
    assert_eq!(lt(0, 0), g("false"), "0 ≮ 0");
    assert_eq!(
        normalize(
            &ctx,
            &apps(
                g("natLt"),
                &[natl_dec("340282366920938463463374607431768211456"), natl(1)]
            )
        ),
        g("false"),
        "a Nat literal past every machine width still decides — the order is arbitrary-precision"
    );

    // `natLt i Zero` reduces to `false` for an OPEN `i`, because the match is on `Succ i`, which
    // is already a constructor application. That is what makes `Fin 0` uninhabited by COMPUTATION
    // rather than by an appeal to a lemma nobody proved.
    let open = ctx.extend("i", nat());
    assert_eq!(
        normalize(&open, &apps(g("natLt"), &[var("i"), g("Zero")])),
        g("false"),
        "`i < 0` is false for every i, decided without instantiating i"
    );

    // The Int order is untouched and still refuses Nat arguments (T10-013 pins the same fact).
    assert_eq!(
        ctx.get_global("lt").expect("the prelude declares lt"),
        &arrow(g("Int"), arrow(g("Int"), bool_ty())),
        "`lt` stays the Int primitive"
    );
    assert!(
        infer_type(
            &ctx,
            &apps(
                g("natLe"),
                &[Term::Lit(Literal::Int(1)), Term::Lit(Literal::Int(2))]
            )
        )
        .is_err(),
        "`natLe` is a Nat relation and must refuse Int literals — two orders that silently accept \
         each other's arguments is how a Fin bound ends up proved about the wrong carrier"
    );

    // The Prop layer the bound proof actually inhabits.
    assert!(
        defeq_for_test(
            &ctx,
            &nat_lt(natl(2), natl(3)),
            &eq_at(bool_ty(), g("true"), g("true"))
        ),
        "`2 < 3` unfolds to a Bool equation that `refl` discharges"
    );
    assert!(
        infer_type(&ctx, &nat_lt(natl(2), natl(3)))
            .is_ok_and(|s| matches!(s, Term::Sort(Universe::Prop))),
        "`Nat_lt` is Prop-valued"
    );
}

// ============================================================================================
// Prod / Sum / Sigma / Subtype — T10-013's record-like half.
// ============================================================================================

/// `(3, true).1 ⤳ 3`, `.2 ⤳ true`, eta holds, and the swapped pair is REFUSED.
#[test]
fn a_prod_projects_both_components_and_refuses_a_swapped_pair() {
    let ctx = std_ctx();
    let pair = apps(g("Prod_mk"), &[nat(), bool_ty(), natl(3), g("true")]);

    assert_eq!(
        infer_type(&ctx, &pair).expect("a pair is well typed"),
        apps(g("Prod"), &[nat(), bool_ty()]),
        "`Prod_mk A B a b : Prod A B`"
    );
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("Prod_fst"), &[nat(), bool_ty(), pair.clone()])
        ),
        natl(3),
        "`.1` must COMPUTE, not merely type"
    );
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("Prod_snd"), &[nat(), bool_ty(), pair.clone()])
        ),
        g("true"),
        "`.2` must COMPUTE"
    );

    // Structure eta — the reason `Prod` goes through `add_structure` and not `add_inductive`.
    let pctx = ctx.extend("p", apps(g("Prod"), &[nat(), bool_ty()]));
    let rebuilt = apps(
        g("Prod_mk"),
        &[
            nat(),
            bool_ty(),
            apps(g("Prod_fst"), &[nat(), bool_ty(), var("p")]),
            apps(g("Prod_snd"), &[nat(), bool_ty(), var("p")]),
        ],
    );
    assert!(
        defeq_for_test(&pctx, &rebuilt, &var("p")),
        "`⟨p.1, p.2⟩ ≡ p`"
    );

    // THE NEGATIVE. A pair whose components are swapped must not type-check; a `Prod` that
    // accepts it is a two-field box, not a product.
    assert!(
        infer_type(
            &ctx,
            &apps(g("Prod_mk"), &[nat(), bool_ty(), g("true"), natl(3)])
        )
        .is_err(),
        "`Prod_mk Nat Bool true 3` must be refused — the fields are typed by the parameters"
    );
    assert!(
        infer_type(&ctx, &apps(g("Prod_fst"), &[nat(), bool_ty(), natl(3)])).is_err(),
        "a projection applied to a non-pair must be refused"
    );

    agreed(&ctx, &pair, "a Prod pair");
}

/// `Option` injects and MATCHES: `some`'s payload comes back out, `none` takes the other branch.
#[test]
fn an_option_matches_on_both_constructors_and_refuses_a_mistyped_payload() {
    let ctx = std_ctx();
    let opt_nat = app(g("Option"), nat());
    let some3 = apps(g("some"), &[nat(), natl(3)]);
    let non = app(g("none"), nat());

    assert_eq!(
        infer_type(&ctx, &some3).expect("`some 3` types"),
        opt_nat,
        "`some A a : Option A`"
    );
    assert_eq!(
        infer_type(&ctx, &non).expect("`none` types"),
        opt_nat,
        "`none A : Option A`"
    );

    // `Option.getD o 0`, written directly as a match. `none` is registered FIRST, as in Lean.
    let get_d = |o: Term| {
        mat(
            o,
            lam("_", opt_nat.clone(), nat()),
            vec![g("Zero"), lam("x", nat(), var("x"))],
        )
    };
    assert_eq!(
        normalize(&ctx, &get_d(some3.clone())),
        natl(3),
        "`(some 3).getD 0 ⤳ 3`"
    );
    assert_eq!(
        normalize(&ctx, &get_d(non.clone())),
        g("Zero"),
        "`none.getD 0 ⤳ 0`"
    );
    assert_eq!(
        ctx.get_constructors("Option")
            .iter()
            .map(|(c, _)| *c)
            .collect::<Vec<_>>(),
        vec!["none", "some"],
        "Lean registers `none` before `some`; a translator that keeps source order must be safe"
    );

    // THE NEGATIVE. `Option` is a container, not a monad: registering it must not conjure the
    // do-block operation surface that `lean_conformance_t05_elaboration` refuses.
    for name in [
        "Monad",
        "Bind",
        "Pure",
        "Functor",
        "Applicative",
        "Seq",
        "ForIn",
        "Id",
        "bind",
        "pure",
        "Bind_bind",
        "Pure_pure",
        "ForIn_forIn",
        "Id_run",
    ] {
        assert!(
            ctx.get_global(name).is_none(),
            "`{name}` must stay absent — `Option` is Lean's core inductive, and registering it \
             must not drag in a monad hierarchy whose operations would never reduce"
        );
    }
    assert!(
        infer_type(&ctx, &apps(g("some"), &[nat(), g("true")])).is_err(),
        "`some Nat true` must be refused"
    );

    agreed(&ctx, &some3, "an Option payload");
}

/// `Sum` injects on both sides and its match dispatches on which injection was used.
#[test]
fn a_sum_dispatches_on_the_injection_and_refuses_the_wrong_side() {
    let ctx = std_ctx();
    let sum = apps(g("Sum"), &[nat(), bool_ty()]);
    let left = apps(g("Sum_inl"), &[nat(), bool_ty(), natl(7)]);
    let right = apps(g("Sum_inr"), &[nat(), bool_ty(), g("true")]);

    assert_eq!(
        infer_type(&ctx, &left).expect("inl types"),
        sum,
        "`inl a : Sum A B`"
    );
    assert_eq!(
        infer_type(&ctx, &right).expect("inr types"),
        sum,
        "`inr b : Sum A B`"
    );

    // `fun | .inl _ => true | .inr b => b`
    let which = |s: Term| {
        mat(
            s,
            lam("_", sum.clone(), bool_ty()),
            vec![lam("_", nat(), g("false")), lam("b", bool_ty(), var("b"))],
        )
    };
    assert_eq!(
        normalize(&ctx, &which(left.clone())),
        g("false"),
        "the inl branch runs"
    );
    assert_eq!(
        normalize(&ctx, &which(right.clone())),
        g("true"),
        "the inr branch runs"
    );

    // THE NEGATIVE. The injections are typed by their own side only.
    assert!(
        infer_type(&ctx, &apps(g("Sum_inl"), &[nat(), bool_ty(), g("true")])).is_err(),
        "`inl` must take an `A`, never a `B` — a Sum whose injections both accept everything is \
         a pair of unrelated boxes"
    );

    agreed(&ctx, &right, "a Sum injection");
}

/// The dependent pair: `s.2`'s TYPE mentions `s.1`'s VALUE, and a witness that does not match the
/// family is refused.
#[test]
fn a_sigma_projects_dependently_and_refuses_a_witness_from_another_fibre() {
    let ctx = std_ctx();
    let fam = lam("n", nat(), eq_at(nat(), var("n"), var("n")));
    let dep = apps(
        g("Sigma_mk"),
        &[nat(), fam.clone(), natl(3), refl(nat(), natl(3))],
    );

    assert_eq!(
        infer_type(&ctx, &dep).expect("a dependent pair is well typed"),
        apps(g("Sigma"), &[nat(), fam.clone()]),
        "the dependent pair inhabits its own family"
    );
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("Sigma_fst"), &[nat(), fam.clone(), dep.clone()])
        ),
        natl(3),
        "`⟨3, rfl⟩.1 ⤳ 3`"
    );
    let snd_ty = infer_type(
        &ctx,
        &apps(g("Sigma_snd"), &[nat(), fam.clone(), dep.clone()]),
    )
    .expect("the dependent projection is well typed");
    assert!(
        defeq_for_test(&ctx, &snd_ty, &eq_at(nat(), natl(3), natl(3))),
        "`s.2 : B s.1` — erasing the dependency gives a constant motive and the wrong type: {snd_ty}"
    );

    // THE NEGATIVE. The second component is checked against `B` applied to the FIRST, so a proof
    // about a different index must not slip through.
    assert!(
        infer_type(
            &ctx,
            &apps(
                g("Sigma_mk"),
                &[nat(), fam.clone(), natl(3), refl(nat(), natl(4))]
            )
        )
        .is_err(),
        "`⟨3, rfl : 4 = 4⟩` must be refused — a Sigma that accepts it is an ordinary Prod"
    );

    agreed(&ctx, &dep, "a dependent pair");
}

/// A `Subtype` carries a PROOF, and the proof is checked against the predicate at the value.
#[test]
fn a_subtype_carries_a_checked_proof_and_refuses_a_false_one() {
    let ctx = std_ctx();
    let pred = fin_pred(natl(3));
    let elem = fin_mk(natl(3), natl(2), bound_proof());

    assert_eq!(
        infer_type(&ctx, &elem).expect("a bounded element is well typed"),
        apps(g("Subtype"), &[nat(), pred.clone()]),
        "`⟨2, _⟩ : {{i : Nat // i < 3}}`"
    );
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("Subtype_val"), &[nat(), pred.clone(), elem.clone()])
        ),
        natl(2),
        "`.val` must COMPUTE"
    );
    let prop_ty = infer_type(
        &ctx,
        &apps(g("Subtype_property"), &[nat(), pred.clone(), elem.clone()]),
    )
    .expect("`.property` is well typed");
    assert!(
        defeq_for_test(&ctx, &prop_ty, &nat_lt(natl(2), natl(3))),
        "`.property : p s.val`, not `p` at some other value: {prop_ty}"
    );

    // THE NEGATIVE — the case that decides whether `Subtype` is a subtype or a labelled pair.
    // `3 < 3` reduces to `Eq Bool false true`, which `refl Bool true` does not inhabit.
    assert!(
        infer_type(&ctx, &fin_mk(natl(3), natl(3), bound_proof())).is_err(),
        "`⟨3, refl⟩ : {{i // i < 3}}` must be REFUSED: the bound is decided by reduction, so a \
         Subtype that accepts it carries no invariant at all"
    );
    assert!(
        infer_type(&ctx, &fin_mk(natl(3), natl(9), bound_proof())).is_err(),
        "…and so must a wildly out-of-range one"
    );

    // The re-checker cannot run the bound's recursion over `Nat` literals; that wall is
    // isolated in `the_re_checker_computes_a_recursion_over_a_nat_literal`.
}

/// `Fin n` IS that subtype, `Fin 0` is uninhabited, and the boundary index is the one that fails.
#[test]
fn a_fin_admits_every_index_below_its_bound_and_nothing_at_or_above_it() {
    let ctx = std_ctx();

    assert_eq!(
        ctx.get_global("Fin").expect("Fin is registered"),
        &arrow(nat(), type0()),
        "`Fin : Nat → Type 0`"
    );
    assert!(
        defeq_for_test(
            &ctx,
            &app(g("Fin"), natl(3)),
            &apps(g("Subtype"), &[nat(), fin_pred(natl(3))])
        ),
        "`Fin n` unfolds to the bounded-Nat subtype — a `Fin` that is an opaque name has no \
         `.val` and no bound"
    );

    // Every index below the bound, and the boundary just above it.
    for i in 0..3i64 {
        let e = fin_mk(natl(3), natl(i), bound_proof());
        assert!(
            infer_type(&ctx, &e).is_ok(),
            "`⟨{i}, _⟩ : Fin 3` must type-check"
        );
        assert_eq!(
            normalize(
                &ctx,
                &apps(g("Subtype_val"), &[nat(), fin_pred(natl(3)), e])
            ),
            natl(i),
            "…and `.val` returns {i}"
        );
    }
    assert!(
        infer_type(&ctx, &fin_mk(natl(3), natl(3), bound_proof())).is_err(),
        "`Fin 3` must not admit 3"
    );

    // `Fin 0` is EMPTY — the fact `Fin` exists to provide. Not one index inhabits it.
    for i in 0..3i64 {
        assert!(
            infer_type(&ctx, &fin_mk(natl(0), natl(i), bound_proof())).is_err(),
            "`Fin 0` must be uninhabited, and `⟨{i}, _⟩` must be refused"
        );
    }

    // The re-checker cannot run the bound's recursion over `Nat` literals; that wall is
    // isolated in `the_re_checker_computes_a_recursion_over_a_nat_literal`.
}

// ============================================================================================
// List — T10-012.
// ============================================================================================

/// The cons-list computes: its recursor ι-reduces on both constructors, `length` and `append`
/// run, and a heterogeneous cons is refused.
#[test]
fn a_list_recurses_over_both_constructors_and_refuses_a_heterogeneous_cons() {
    let ctx = std_ctx();
    let list_nat = app(g("List"), nat());
    let l = list_of(nat(), &[natl(1), natl(2), natl(3)]);

    assert!(
        defeq_for_test(
            &ctx,
            ctx.get_global("List").expect("List is registered"),
            &arrow(type0(), type0()),
        ),
        "`List : Type 0 → Type 0` — parametric, unlike `EList`"
    );
    assert_eq!(
        ctx.get_constructors("List")
            .iter()
            .map(|(c, _)| *c)
            .collect::<Vec<_>>(),
        vec!["nil", "cons"],
        "Lean's registration order is nil-then-cons, and T10-012 pins the translator to it"
    );
    assert_eq!(
        infer_type(&ctx, &l).expect("[1,2,3] types"),
        list_nat,
        "`[1,2,3] : List Nat`"
    );

    // `List_length` builds its answer with `Succ`, so the normal form is the unary spine and the
    // CLAIM is the value: definitionally 3, and definitionally neither of its neighbours.
    let len3 = normalize(&ctx, &apps(g("List_length"), &[nat(), l.clone()]));
    assert!(
        defeq_for_test(&ctx, &len3, &natl(3)),
        "`length [1,2,3]` must be 3, got {len3} — a List whose length does not reduce is a name, \
         not a list"
    );
    assert!(
        !defeq_for_test(&ctx, &len3, &natl(2)) && !defeq_for_test(&ctx, &len3, &natl(4)),
        "…and it must be 3 EXACTLY — an off-by-one length is what makes a bound proof lie"
    );
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("List_length"), &[nat(), app(g("nil"), nat())])
        ),
        g("Zero"),
        "`length [] ⤳ 0`"
    );
    assert!(
        defeq_for_test(
            &ctx,
            &apps(
                g("List_append"),
                &[
                    nat(),
                    list_of(nat(), &[natl(1)]),
                    list_of(nat(), &[natl(2)])
                ]
            ),
            &list_of(nat(), &[natl(1), natl(2)])
        ),
        "`[1] ++ [2] = [1,2]` by rfl"
    );

    // `List_rec` — the dependent eliminator, at a DATA motive, ι-reducing on both branches.
    let sum_rec = |xs: Term| {
        apps(
            g("List_rec"),
            &[
                nat(),
                lam("_", list_nat.clone(), nat()),
                g("Zero"),
                lam(
                    "h",
                    nat(),
                    lam(
                        "_",
                        list_nat.clone(),
                        lam("acc", nat(), app(g("Succ"), var("acc"))),
                    ),
                ),
                xs,
            ],
        )
    };
    assert_eq!(
        normalize(&ctx, &sum_rec(app(g("nil"), nat()))),
        g("Zero"),
        "List_rec on nil"
    );
    assert!(
        defeq_for_test(&ctx, &normalize(&ctx, &sum_rec(l.clone())), &natl(3)),
        "List_rec must ι-reduce through three conses"
    );

    // THE NEGATIVE.
    assert!(
        infer_type(
            &ctx,
            &apps(g("cons"), &[nat(), g("true"), app(g("nil"), nat())])
        )
        .is_err(),
        "`cons Nat true []` must be refused — the element type is the parameter"
    );
    assert!(
        infer_type(
            &ctx,
            &apps(g("cons"), &[nat(), natl(1), app(g("nil"), bool_ty())])
        )
        .is_err(),
        "`cons Nat 1 ([] : List Bool)` must be refused — the tail carries the same parameter"
    );
    // Registering `List` must not disturb the two lists that were already there.
    assert_eq!(
        ctx.get_constructors("EList")
            .iter()
            .map(|(c, _)| *c)
            .collect::<Vec<_>>(),
        vec!["ENil", "ECons"],
        "the monomorphic EList the induction tactic runs over is untouched"
    );
    assert_eq!(
        ctx.get_constructors("TList")
            .iter()
            .map(|(c, _)| *c)
            .collect::<Vec<_>>(),
        vec!["TNil", "TCons"],
        "and so is TList"
    );

    agreed(&ctx, &l, "a List");
}

/// `List_get` returns the element AT the index and cannot be called out of range, because the
/// bound proof is an argument and an out-of-range bound does not exist.
#[test]
fn list_get_returns_the_indexed_element_and_has_no_out_of_range_call() {
    let ctx = std_ctx();
    let l = list_of(nat(), &[natl(10), natl(20), natl(30)]);
    let get = |i: i64| apps(g("List_get"), &[nat(), l.clone(), natl(i), bound_proof()]);

    assert_eq!(normalize(&ctx, &get(0)), natl(10), "`l[0] ⤳ 10`");
    assert_eq!(normalize(&ctx, &get(1)), natl(20), "`l[1] ⤳ 20`");
    assert_eq!(normalize(&ctx, &get(2)), natl(30), "`l[2] ⤳ 30`");

    // THE NEGATIVE. The index the cons-list substitution would silently answer for.
    assert!(
        infer_type(&ctx, &get(3)).is_err(),
        "`l[3]` on a three-element list must be REFUSED at the type level — an indexing operation \
         that decides what to do out of range is the substitution T10-011 warns about"
    );
    assert!(
        infer_type(
            &ctx,
            &apps(
                g("List_get"),
                &[nat(), app(g("nil"), nat()), natl(0), bound_proof()]
            )
        )
        .is_err(),
        "…and the empty list has no element 0"
    );

    // The re-checker cannot run the bound's recursion over `Nat` literals; that wall is
    // isolated in `the_re_checker_computes_a_recursion_over_a_nat_literal`.
}

// ============================================================================================
// Array — T10-011.
// ============================================================================================

/// `size`, `push` and a bound-proof `get`, all computing.
#[test]
fn an_array_sizes_pushes_and_indexes_and_refuses_an_unproved_index() {
    let ctx = std_ctx();
    let small = app(
        app(g("Array_mk"), nat()),
        list_of(nat(), &[natl(1), natl(2), natl(3)]),
    );

    assert_eq!(
        infer_type(&ctx, &small).expect("an array is well typed"),
        app(g("Array"), nat()),
        "`#[1,2,3] : Array Nat`"
    );
    assert!(
        defeq_for_test(
            &ctx,
            &apps(g("Array_size"), &[nat(), small.clone()]),
            &natl(3)
        ),
        "`small.size` must be 3"
    );
    assert_eq!(
        normalize(
            &ctx,
            &apps(
                g("Array_get"),
                &[nat(), small.clone(), natl(0), bound_proof()]
            )
        ),
        natl(1),
        "`small[0] ⤳ 1`"
    );
    assert_eq!(
        normalize(
            &ctx,
            &apps(
                g("Array_get"),
                &[nat(), small.clone(), natl(2), bound_proof()]
            )
        ),
        natl(3),
        "`small[2] ⤳ 3`"
    );

    let pushed = apps(g("Array_push"), &[nat(), small.clone(), natl(4)]);
    assert!(
        defeq_for_test(
            &ctx,
            &apps(g("Array_size"), &[nat(), pushed.clone()]),
            &natl(4)
        ),
        "`(small.push 4).size` must be 4 — Lean's `size_push` must hold by computation"
    );
    assert_eq!(
        normalize(
            &ctx,
            &apps(
                g("Array_get"),
                &[nat(), pushed.clone(), natl(3), bound_proof()]
            )
        ),
        natl(4),
        "`(small.push 4)[3] ⤳ 4`"
    );

    // THE NEGATIVE.
    assert!(
        infer_type(
            &ctx,
            &apps(
                g("Array_get"),
                &[nat(), small.clone(), natl(3), bound_proof()]
            )
        )
        .is_err(),
        "`small[3]` must be refused — the bound proof is checked against `Array_size`"
    );
    assert!(
        infer_type(
            &ctx,
            &apps(
                g("Array_get"),
                &[nat(), small.clone(), natl(7), bound_proof()]
            )
        )
        .is_err(),
        "…and so must `small[7]`"
    );

    agreed(&ctx, &pushed, "a pushed array");
}

// ============================================================================================
// UInt* / Char / String — T10-009, T10-006, T10-005.
// ============================================================================================

/// The machine widths are bounded `Nat`s: a value below `2^N` inhabits them, the modulus itself
/// never does, and each unfolds to the `Fin` Lean defines it as. Their ARITHMETIC — the wrapping
/// `(255 : UInt8) + 1 = 0` — is a separate capability the corpus still tracks as a gap.
#[test]
fn a_machine_word_admits_exactly_the_values_below_its_width() {
    let ctx = std_ctx();
    let widths: [(&str, &str); 5] = [
        ("UInt8", "256"),
        ("UInt16", "65536"),
        ("UInt32", "4294967296"),
        ("UInt64", "18446744073709551616"),
        ("USize", "18446744073709551616"),
    ];

    for (name, size) in widths {
        let bound = natl_dec(size);
        assert_eq!(
            ctx.get_global(name)
                .unwrap_or_else(|| panic!("`{name}` is registered")),
            &type0(),
            "`{name} : Type 0`"
        );
        assert!(
            defeq_for_test(&ctx, &g(name), &app(g("Fin"), bound.clone())),
            "`{name}` must unfold to `Fin {size}` — an opaque width has no values and no bound"
        );
        // An ordinary value is representable at every width. The bound check costs the SMALLER
        // operand (see `the_nat_order_costs_the_smaller_operand`), so this is cheap even at 2^64.
        assert!(
            infer_type(&ctx, &fin_mk(bound.clone(), natl(7), bound_proof())).is_ok(),
            "`(7 : {name})` must type-check"
        );
        // …and `Fin` is the thing that stops it holding its own modulus: the predicate really is
        // the bound, not a decoration. (The refusal itself is exercised below — the four widths
        // share one mechanism, and re-deciding `2^32 ≤ 2^32` at each costs the step budget for no
        // extra information.)
        assert!(
            defeq_for_test(
                &ctx,
                &app(g("Fin"), bound.clone()),
                &apps(g("Subtype"), &[nat(), fin_pred(bound.clone())])
            ),
            "`{name}`'s values are `Nat`s carrying a proof of the bound"
        );
    }

    // The full range, at the one width whose top the structural order can reach inside the
    // normalisation budget. 255 is in; 256 is out; and that is the whole of `UInt8`.
    assert!(
        infer_type(&ctx, &fin_mk(natl(256), natl(255), bound_proof())).is_ok(),
        "`(255 : UInt8)` — the largest byte — must type-check"
    );
    assert!(
        infer_type(&ctx, &fin_mk(natl(256), natl(256), bound_proof())).is_err(),
        "`(256 : UInt8)` must NOT — a width that admits its own modulus is a Nat with a \
         misleading name"
    );
    assert!(
        infer_type(&ctx, &fin_mk(natl(256), natl(999), bound_proof())).is_err(),
        "…and neither may anything above it"
    );

    // The exactness that makes an Int re-encoding hopeless is unchanged: LOGOS never wraps.
    assert_eq!(
        normalize(
            &ctx,
            &apps(
                g("add"),
                &[
                    Term::Lit(Literal::Int(i64::MAX)),
                    Term::Lit(Literal::Int(1))
                ]
            )
        ),
        Term::Lit(logicaffeine_kernel::int_lit(
            BigInt::parse_decimal("9223372036854775808").unwrap()
        )),
        "Int arithmetic stays exact and unbounded"
    );
}

/// The cost model of the `Nat` order, pinned because every bound proof in the prelude pays it.
///
/// `natLe` recurses until EITHER operand hits `Zero`, so deciding `m ≤ n` costs `min(m, n)`
/// ι-steps against `normalize`'s budget. A small index into a huge range is therefore cheap —
/// which is what makes `UInt64` usable at all — while REFUTING a bound near the top of a wide
/// range is not reachable. The honest consequence is recorded in the negative: an undecided bound
/// is REFUSED, never assumed, so the failure direction is the safe one.
#[test]
fn the_nat_order_costs_the_smaller_operand() {
    let ctx = std_ctx();
    let huge = natl_dec("18446744073709551616");

    // Cheap because the FIRST operand runs out first — six peels, whatever the second is.
    assert_eq!(
        normalize(&ctx, &apps(g("natLt"), &[natl(5), huge.clone()])),
        g("true"),
        "`5 < 2^64` must decide in steps proportional to 5, not to 2^64"
    );
    // Cheap because the SECOND operand runs out first.
    assert_eq!(
        normalize(&ctx, &apps(g("natLt"), &[huge.clone(), natl(5)])),
        g("false"),
        "`2^64 < 5` must decide in steps proportional to 5"
    );
    // The expensive corner: both operands are huge, so neither bottoms out inside the budget and
    // the reduction is abandoned. The term must NOT be mistaken for `true`.
    let stuck = normalize(&ctx, &apps(g("natLt"), &[huge.clone(), huge.clone()]));
    assert_ne!(
        stuck,
        g("true"),
        "an abandoned order decision must never read as `true`"
    );
    assert!(
        infer_type(&ctx, &fin_mk(huge.clone(), huge.clone(), bound_proof())).is_err(),
        "…and the element it would have justified must be REFUSED. Closing this properly needs an \
         O(1) `Nat` comparison in the reduction primitives, the way Lean's `Nat.decLt` is GMP; \
         until then the wall is a refusal, not an admission"
    );
}

/// A `Char` is a valid Unicode scalar value — a surrogate is refused, which is the invariant the
/// type exists to carry.
#[test]
fn a_char_admits_a_scalar_value_and_refuses_a_surrogate() {
    let ctx = std_ctx();
    let u32_bound = natl_dec("4294967296");
    let word = |n: Term| fin_mk(u32_bound.clone(), n, bound_proof());
    let valid = |n: Term| app(g("Char_isValid"), word(n));
    // `Or.inl` — the low half of `isValidChar`, which is where every ASCII character lands.
    let low = |n: Term| {
        apps(
            g("left"),
            &[
                nat_lt(app(g("UInt32_toNat"), word(n.clone())), natl_dec("55296")),
                apps(
                    g("And"),
                    &[
                        nat_lt(natl_dec("57343"), app(g("UInt32_toNat"), word(n.clone()))),
                        nat_lt(app(g("UInt32_toNat"), word(n.clone())), natl_dec("1114112")),
                    ],
                ),
                bound_proof(),
            ],
        )
    };
    let mk_char = |n: Term| {
        apps(
            g("Subtype_mk"),
            &[g("UInt32"), g("Char_isValid"), word(n.clone()), low(n)],
        )
    };

    assert_eq!(
        ctx.get_global("Char").expect("Char is registered"),
        &type0(),
        "`Char : Type 0`"
    );
    assert!(
        infer_type(&ctx, &valid(natl(97))).is_ok_and(|s| matches!(s, Term::Sort(Universe::Prop))),
        "`Char.isValidChar` is a Prop-valued predicate on a UInt32"
    );
    let a = mk_char(natl(97));
    assert!(
        infer_type(&ctx, &a).is_ok(),
        "`'a'` must type-check: {:?}",
        infer_type(&ctx, &a)
    );
    assert!(
        defeq_for_test(
            &ctx,
            &g("Char"),
            &apps(g("Subtype"), &[g("UInt32"), g("Char_isValid")])
        ),
        "`Char` is `{{ val : UInt32 // val.isValidChar }}` — the honest encoding, not a Nat alias"
    );

    // THE NEGATIVE, three ways. The predicate is a REAL obligation: a proof of the wrong shape is
    // refused, so `Char` is not a UInt32 wearing a label.
    assert!(
        infer_type(
            &ctx,
            &apps(
                g("Subtype_mk"),
                &[
                    g("UInt32"),
                    g("Char_isValid"),
                    word(natl(97)),
                    bound_proof()
                ]
            )
        )
        .is_err(),
        "a Char built with `refl` in place of the validity disjunction must be refused — a \
         predicate that anything inhabits is not an invariant"
    );
    // A surrogate must not come through the low disjunct: `0xD800 < 0xD800` is not true.
    assert!(
        infer_type(&ctx, &mk_char(natl_dec("55296"))).is_err(),
        "`Char.ofNat 0xD800` must not produce a Char through the low disjunct — a surrogate is not \
         a scalar value, and a Char that admits one loses the only invariant it has"
    );
    // …and `Char` must not be `Nat`, or every `Nat → Char` argument swap would type-check.
    assert!(
        infer_type(&ctx, &app(g("Char_isValid"), natl(97))).is_err(),
        "`isValidChar` takes a UInt32, not a bare Nat"
    );

    // The re-checker cannot run the bound's recursion over `Nat` literals; that wall is
    // isolated in `the_re_checker_computes_a_recursion_over_a_nat_literal`.
}

/// `String` is a structure over `List Char`, and `Text` — the opaque literal carrier — stays
/// opaque. The two are different things and must not be merged.
#[test]
fn a_string_is_a_character_list_while_text_stays_the_opaque_literal_carrier() {
    let ctx = std_ctx();
    let empty = app(g("String_mk"), app(g("nil"), g("Char")));

    assert_eq!(
        ctx.get_global("String").expect("String is registered"),
        &type0(),
        "`String : Type 0`"
    );
    assert_eq!(
        infer_type(&ctx, &empty).expect("`String.mk []` types"),
        g("String"),
        "`String.mk : List Char → String`"
    );
    assert!(
        defeq_for_test(
            &ctx,
            &app(g("String_data"), empty.clone()),
            &app(g("nil"), g("Char"))
        ),
        "`(String.mk l).data ⤳ l` — the projection must COMPUTE"
    );

    // THE NEGATIVE, and the reason T10-005 and T10-006 cannot both be satisfied: `Text` is the
    // carrier `Literal::Text` lands in, and it must stay a zero-constructor opaque type, or the
    // literal has two incompatible spellings.
    assert!(
        ctx.get_constructors("Text").is_empty(),
        "`Text` must stay opaque — giving it constructors would make `Literal::Text` and its \
         cons-list spelling two different normal forms of the same value"
    );
    assert_eq!(
        infer_type(&ctx, &Term::Lit(Literal::Text("hi".into()))).expect("a Text literal types"),
        g("Text"),
        "a string LITERAL is still a `Text`, not a `String`"
    );
    assert!(
        infer_type(&ctx, &app(g("String_mk"), app(g("nil"), nat()))).is_err(),
        "`String.mk ([] : List Nat)` must be refused — a String is a list of CHARACTERS"
    );

    agreed(&ctx, &empty, "a String");
}

// ============================================================================================
// Iff — T06-005.
// ============================================================================================

/// The biconditional is a DEFINITION unfolding to the conjunction of the two implications, so
/// `.mp`/`.mpr` are projections of an `And` and not methods of a fresh opaque proposition.
#[test]
fn the_biconditional_is_the_conjunction_of_two_implications_and_is_not_an_inductive() {
    let ctx = std_ctx();

    assert!(ctx.is_definition("Iff"), "`Iff` must be δ-transparent");
    assert!(
        !ctx.is_inductive("Iff"),
        "…and must NOT be a fresh inductive nothing can bridge to"
    );
    assert!(
        defeq_for_test(
            &ctx,
            ctx.get_global("Iff").expect("Iff is registered"),
            &arrow(prop(), arrow(prop(), prop())),
        ),
        "`Iff : Prop → Prop → Prop`"
    );
    assert!(
        defeq_for_test(
            &ctx,
            &apps(g("Iff"), &[g("True"), g("False")]),
            &apps(
                g("And"),
                &[arrow(g("True"), g("False")), arrow(g("False"), g("True"))]
            )
        ),
        "`p ↔ q` ≡ `(p → q) ∧ (q → p)`"
    );

    // `Iff.intro` is `And.intro`, and it really inhabits the biconditional.
    let id_true = lam("h", g("True"), var("h"));
    let intro = apps(
        g("conj"),
        &[
            arrow(g("True"), g("True")),
            arrow(g("True"), g("True")),
            id_true.clone(),
            id_true.clone(),
        ],
    );
    let iff_ty = apps(g("Iff"), &[g("True"), g("True")]);
    assert!(
        infer_type(&ctx, &intro).is_ok_and(|t| defeq_for_test(&ctx, &t, &iff_ty)),
        "`⟨id, id⟩ : True ↔ True` — the introduction is the conjunction's"
    );

    // THE NEGATIVE. `Iff` is a Prop connective; a data argument must be refused rather than
    // promoted, which is what an `imax`-blind encoding would do.
    assert!(
        infer_type(&ctx, &apps(g("Iff"), &[nat(), nat()])).is_err(),
        "`Iff Nat Nat` must be refused — the biconditional relates PROPOSITIONS"
    );

    agreed(&ctx, &intro, "an Iff introduction");
}

// ============================================================================================
// Subsingleton / Squash / Inhabited / Nonempty — T06-015, T07-010.
// ============================================================================================

/// `Subsingleton` is a `Prop`-valued class whose method really is a proof of equality, and
/// `Squash` is a `Type`-valued truncation — the distinction that separates truncation from choice.
#[test]
fn subsingleton_is_prop_valued_and_squash_stays_in_type() {
    let ctx = std_ctx();

    assert!(
        defeq_for_test(
            &ctx,
            ctx.get_global("Subsingleton")
                .expect("Subsingleton is registered"),
            &arrow(type0(), prop()),
        ),
        "`Subsingleton : Type 0 → Prop` — a Type-valued class would promote every proposition \
         that mentions it through imax"
    );
    assert_eq!(
        infer_type(&ctx, &app(g("Subsingleton"), g("Unit"))).expect("`Subsingleton Unit` types"),
        prop(),
        "…so `Subsingleton Unit` is a Prop"
    );
    // The method: given the instance, any two elements are equal.
    let elim_ty = infer_type(&ctx, &apps(g("Subsingleton_elim"), &[g("Unit")]))
        .expect("`Subsingleton.elim` is applicable");
    assert!(
        defeq_for_test(
            &ctx,
            &elim_ty,
            &pi(
                "s",
                app(g("Subsingleton"), g("Unit")),
                pi(
                    "x",
                    g("Unit"),
                    pi("y", g("Unit"), eq_at(g("Unit"), var("x"), var("y")))
                )
            )
        ),
        "`Subsingleton.elim : [Subsingleton α] → ∀ x y : α, x = y`, got {elim_ty}"
    );

    assert!(
        defeq_for_test(
            &ctx,
            ctx.get_global("Squash").expect("Squash is registered"),
            &arrow(type0(), type0())
        ),
        "`Squash : Type → Type` — a Prop-valued truncation is `Nonempty`, and eliminating THAT \
         into Type is choice"
    );
    assert_eq!(
        infer_type(&ctx, &app(g("Squash"), nat())).expect("`Squash Nat` types"),
        type0(),
        "`Squash Nat : Type 0`, not Prop"
    );

    // THE NEGATIVE. `Squash` truncates through `Quot`, so it must not be matchable — a match
    // recovering the representative is exactly what `Quot_sound` forbids.
    let mut s = ctx.clone();
    s.add_declaration("sq", app(g("Squash"), nat()));
    assert!(
        infer_type(
            &s,
            &mat(
                g("sq"),
                lam("_", app(g("Squash"), nat()), nat()),
                vec![g("Zero")]
            )
        )
        .is_err(),
        "a `Squash` value must not be pattern-matched"
    );
}

/// `Inhabited`'s default COMPUTES, and `Nonempty` stays a `Prop` that cannot be eliminated into
/// `Type` — the firewall that keeps `Classical.choice` an axiom rather than a derivation.
#[test]
fn inhabited_computes_its_default_and_nonempty_cannot_large_eliminate() {
    let ctx = std_ctx();

    assert!(
        defeq_for_test(
            &ctx,
            ctx.get_global("Inhabited")
                .expect("Inhabited is registered"),
            &arrow(type0(), type0()),
        ),
        "`Inhabited : Type 0 → Type 0` — it carries DATA"
    );
    let inst = apps(g("Inhabited_mk"), &[nat(), g("Zero")]);
    assert_eq!(
        normalize(&ctx, &apps(g("Inhabited_default"), &[nat(), inst.clone()])),
        g("Zero"),
        "`(default : Nat) ⤳ 0` — the projection must COMPUTE, parameter first"
    );
    assert!(
        ctx.struct_info("Inhabited").is_some(),
        "`Inhabited` must go through `add_structure`, which is what registers the StructInfo that \
         gates definitional eta"
    );

    assert!(
        defeq_for_test(
            &ctx,
            ctx.get_global("Nonempty").expect("Nonempty is registered"),
            &arrow(type0(), prop()),
        ),
        "`Nonempty : Type 0 → Prop`"
    );
    let witness = apps(g("Nonempty_intro"), &[nat(), g("Zero")]);
    assert_eq!(
        infer_type(&ctx, &witness).expect("`Nonempty.intro 0` types"),
        app(g("Nonempty"), nat()),
        "`Nonempty.intro : α → Nonempty α`"
    );

    // THE NEGATIVE, and the most important assertion in this file. `Nonempty α` has ONE
    // constructor whose argument is DATA, so it is not a subsingleton: eliminating it into `Type`
    // would extract the witness from a proof, which IS `Classical.choice`. It must be refused.
    let mut n = ctx.clone();
    n.add_declaration("ne", app(g("Nonempty"), nat()));
    assert!(
        infer_type(
            &n,
            &mat(
                g("ne"),
                lam("_", app(g("Nonempty"), nat()), nat()),
                vec![lam("x", nat(), var("x"))]
            )
        )
        .is_err(),
        "large elimination of `Nonempty` into `Type` must be REFUSED — a `Nonempty` that yields \
         its witness is choice, derived rather than declared, and the axiom census would never \
         see it"
    );

    agreed(&ctx, &inst, "an Inhabited instance");
    agreed(&ctx, &witness, "a Nonempty witness");
}

// ============================================================================================
// A DEFECT THIS FILE FOUND — the re-checker cannot run a recursion over a `Nat` LITERAL.
// ============================================================================================

/// The second kernel's guarded fix-unfolding asks `ctor_headed` whether some argument is a
/// constructor application. `Literal::Nat(n)` IS one — it is `Zero` or `Succ (n-1)`, and
/// `recheck.rs`'s own `Match` arm already expands it that way — but `ctor_headed` only accepts a
/// `Db::Global` the context calls a constructor, so a literal fails the guard, the fixpoint never
/// unfolds, and every structural recursion over a `Nat` literal is STUCK in the re-checker while
/// it computes in the main kernel.
///
/// This is not caused by the structural types: `decEqNat` is the prelude's own hand-written
/// recursion and it predates them, which is what the first half demonstrates. The consequence is
/// that `double_check` reports `Disagree` on a term the main kernel is right to accept — the safe
/// direction, but it means a `Fin`/`Char`/`Array_get` element is single-kernel-verified today.
///
/// The fix is one clause in `recheck.rs`'s `ctor_headed` (treat a `Nat` literal as
/// constructor-headed, exactly as the `Match` arm does). That file belongs to the kernel-semantics
/// bunch, so this test states the requirement rather than reaching across.
#[test]
fn the_re_checker_computes_a_recursion_over_a_nat_literal() {
    let ctx = std_ctx();

    // PRE-EXISTING, and nothing to do with the structural types: `decEqNat` is the prelude's own
    // `Nat` decision procedure. The main kernel runs it on literals…
    let dec = apps(g("decEqNat"), &[natl(2), natl(2)]);
    let decided = apps(g("decide"), &[eq_at(nat(), natl(2), natl(2)), dec.clone()]);
    assert_eq!(
        normalize(&ctx, &decided),
        g("true"),
        "the main kernel decides `2 = 2` by running decEqNat on two literals"
    );
    // …so a proof that cites the computed answer must survive the SECOND kernel too.
    let cited = app(
        lam("h", eq_at(bool_ty(), decided.clone(), g("true")), g("Zero")),
        refl(bool_ty(), g("true")),
    );
    assert!(
        infer_type(&ctx, &cited).is_ok(),
        "the main kernel accepts a proof resting on `decide (2 = 2) ⤳ true`"
    );
    assert!(
        matches!(double_check(&ctx, &cited), DoubleCheck::Agreed),
        "and so must the re-checker — a recursion over a Nat LITERAL is stuck behind \
         `ctor_headed`, which does not know that `Literal::Nat(n)` is `Zero`/`Succ _`: {:?}",
        double_check(&ctx, &cited)
    );

    // The same wall, reached through the new bound proofs: `⟨2, refl⟩ : Fin 3` needs
    // `natLt 2 3 ⤳ true`, which is a recursion over two literals.
    let elem = fin_mk(natl(3), natl(2), bound_proof());
    assert!(
        infer_type(&ctx, &elem).is_ok(),
        "the main kernel accepts a Fin element"
    );
    assert!(
        matches!(double_check(&ctx, &elem), DoubleCheck::Agreed),
        "a `Fin` element must be verified by BOTH kernels, or the bound is single-kernel \
         evidence: {:?}",
        double_check(&ctx, &elem)
    );
}
