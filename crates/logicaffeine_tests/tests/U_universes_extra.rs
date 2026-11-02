//! Lane U — universes and the two kernels. The behaviour the conformance corpus does NOT ask
//! about, written as positive/negative pairs so a weak implementation cannot satisfy them.
//!
//! The corpus asks "does `Box.{0}` resolve?". It never asks what happens at the WRONG arity,
//! at a monomorphic name, at a declaration whose level variables no parameter list covers, or
//! whether the SECOND kernel agrees about any of it. Those are where the bugs live: six
//! soundness bugs this campaign were miscounted identically in both kernels, so `double_check`
//! returned `Agreed` throughout.
//!
//! The shared specification these tests pin (both kernels implement it independently):
//!
//! 1. **Level parameters of a name.** An explicitly universe-polymorphic definition carries its
//!    own `params`. Every other declaration — inductive, constructor, definition, axiom —
//!    auto-binds the free level variables of its STORED type, in first-occurrence order. That is
//!    Lean's auto-bound-implicit rule, and it is derived rather than stored, so one registration
//!    path cannot disagree with another about what a name's level telescope is.
//! 2. **Level closure.** `free_level_vars(ty) ⊆ params`. A declaration that carries a level
//!    variable nothing binds is REFUSED at every reference, never instantiated with a partial
//!    substitution that silently retains the free level.
//! 3. **Arity.** `|levels| = |params|`, positionally zipped. A level list on a monomorphic name
//!    is a lie about the declaration and is refused.
//! 4. **Implicit instantiation.** A bare `Global` naming a declaration with level parameters is
//!    typed at FRESH level metavariables (`?`-prefixed, the elaborator's own convention), solved
//!    from the argument's sort by the application rule — never defaulted to `Type 0`, never left
//!    to leak into a reported type.
//! 5. **Sharing.** Substituting an argument into a dependent telescope writes it ONCE.
//! 6. **Diagnostics.** A name missing from the trusted base is not the same event as a name
//!    missing from the local context, and an uninstantiated metavariable is neither.
//! 7. **A bare reference that is not a head.** Anywhere but an application's head, a bare
//!    reference stands at its declaration's OWN level parameters. A stored term writes it so, the
//!    declaration around it binds those levels, and conversion equates the bare reference with
//!    exactly that instance. A bare HEAD is the instance its arguments solve, which congruence
//!    never confuses with the own-levels one.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, double_check, infer_type, instantiate, is_subtype, normalize, recheck, Context,
    DoubleCheck, KernelError, Literal, MetaCtx, ReCheckError, Term, Universe,
};

// ==== PER-FILE HELPERS (house style) ====

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
    xs.iter().cloned().fold(f, app)
}
fn lam(p: &str, ty: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(b),
    }
}
fn pi(p: &str, ty: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(b),
    }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn sort(u: Universe) -> Term {
    Term::Sort(u)
}
fn ty(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn uvar(n: &str) -> Universe {
    Universe::Var(n.to_string())
}
fn konst(n: &str, levels: Vec<Universe>) -> Term {
    Term::Const {
        name: n.to_string(),
        levels,
    }
}
fn natl(n: i64) -> Term {
    Term::Lit(Literal::Nat(logicaffeine_base::BigInt::from_i64(n)))
}
fn eq_at(t: Term, a: Term, b: Term) -> Term {
    apps(g("Eq"), &[t, a, b])
}
fn refl(t: Term, a: Term) -> Term {
    apps(g("refl"), &[t, a])
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
/// `Box.{u} : Sort u → Sort u`, an ordinary inductive whose arity MENTIONS a level variable —
/// the auto-bound shape. Registered through the plain (monomorphic) entry points on purpose:
/// the level telescope must be DERIVED from what the author wrote, not stored beside it.
fn register_box(ctx: &mut Context) {
    ctx.add_indexed_inductive("Box", pi("A", sort(uvar("u")), sort(uvar("u"))), 1);
    ctx.add_constructor(
        "Box_mk",
        "Box",
        pi(
            "A",
            sort(uvar("u")),
            arrow(var("A"), app(g("Box"), var("A"))),
        ),
    );
}
/// The printed size of a term — a deterministic proxy for node count, stable under the load a
/// twelve-agent box puts on wall-clock.
fn printed_size(t: &Term) -> usize {
    format!("{t}").len()
}

// ============================================================================================
// 1 · LEVEL PARAMETERS ARE DERIVED FROM THE DECLARATION'S OWN TYPE
// ============================================================================================

#[test]
fn a_declarations_level_parameters_are_the_level_variables_it_was_written_with() {
    let mut ctx = std_ctx();
    ctx.add_declaration("polyAxiom", arrow(sort(uvar("u")), prop()));

    // POSITIVE — it resolves, and the resolved type COMPUTES: the level argument is actually
    // substituted, not merely counted.
    assert_eq!(
        infer_type(&ctx, &konst("polyAxiom", vec![Universe::Type(0)]))
            .expect("`polyAxiom.{0}` resolves"),
        arrow(ty(0), prop()),
        "`polyAxiom.{{0}}` must have `u` REPLACED by `Type 0`, not merely accepted"
    );
    assert_eq!(
        infer_type(&ctx, &konst("polyAxiom", vec![Universe::Type(5)]))
            .expect("`polyAxiom.{5}` resolves"),
        arrow(ty(5), prop()),
        "…and the SAME declaration serves level 5 — one declaration, every instantiation"
    );
    assert_eq!(
        infer_type(&ctx, &konst("polyAxiom", vec![Universe::Prop]))
            .expect("`polyAxiom.{Prop}` resolves"),
        arrow(prop(), prop()),
        "…including `Prop`, which is what a level VARIABLE ranges over"
    );

    // NEGATIVE — the arity, in both directions. A level list that does not match the
    // declaration's telescope is a lie about the declaration.
    assert!(
        infer_type(&ctx, &konst("polyAxiom", vec![])).is_err(),
        "too FEW levels must be refused — an empty list leaves `u` free"
    );
    assert!(
        infer_type(
            &ctx,
            &konst("polyAxiom", vec![Universe::Type(0), Universe::Type(1)])
        )
        .is_err(),
        "too MANY levels must be refused — the surplus is silently dropped otherwise"
    );

    // NEGATIVE — a MONOMORPHIC name has no level telescope, so it is not reachable through the
    // `Const` form AT ALL, with levels or without. `Const` MEANS "instantiate a telescope"; Lean
    // conflates `Expr.const n []` with the constant itself and LOGOS deliberately does not
    // (`lean_conformance_t00_terms`'s T00-003 is the lock), so the two registries stay disjoint and the
    // category error is named rather than papered over.
    assert!(
        matches!(
            infer_type(&ctx, &konst("Nat", vec![Universe::Type(0)])),
            Err(KernelError::UnboundVariable(_))
        ),
        "`Nat.{{0}}` must be refused: `Nat` binds no universe parameter, so a level argument \
         describes a declaration that does not exist"
    );
    assert!(
        matches!(
            infer_type(&ctx, &konst("Nat", vec![])),
            Err(KernelError::UnboundVariable(_))
        ),
        "…and `Nat.{{}}` is not a second spelling of `Global(\"Nat\")` either"
    );
    assert!(
        infer_type(&ctx, &g("Nat")).is_ok(),
        "…which is what `Term::Global` is for"
    );

    // NEGATIVE — an unknown name stays unknown, whatever levels are attached.
    assert!(
        infer_type(&ctx, &konst("noSuchName", vec![Universe::Type(0)])).is_err(),
        "a level list does not conjure a declaration"
    );
}

#[test]
fn the_derived_level_telescope_is_positional_and_in_first_occurrence_order() {
    let mut ctx = std_ctx();
    // `asym.{u,v} : Sort u → Sort v → Sort v` — `u` occurs first, `v` second, and only `v`
    // reaches the result, so a swapped instantiation is OBSERVABLE.
    ctx.add_declaration(
        "asym",
        pi(
            "A",
            sort(uvar("u")),
            pi("B", sort(uvar("v")), sort(uvar("v"))),
        ),
    );

    assert_eq!(
        infer_type(
            &ctx,
            &konst("asym", vec![Universe::Type(0), Universe::Type(1)])
        )
        .expect("the ordered instantiation"),
        pi("A", ty(0), pi("B", ty(1), ty(1))),
        "first-occurrence order: `u` is parameter 0 and `v` is parameter 1"
    );
    assert_eq!(
        infer_type(
            &ctx,
            &konst("asym", vec![Universe::Type(1), Universe::Type(0)])
        )
        .expect("the swapped instantiation is well-formed, just DIFFERENT"),
        pi("A", ty(1), pi("B", ty(0), ty(0))),
        "a swap must change the type — a positional telescope that ignores order is not one"
    );
    // …and the swap is OBSERVABLE, which is the property that makes a positional telescope
    // worth having: `Type 0` fits parameter A only at level 1, so exactly one of the two
    // instantiations accepts the same arguments.
    let swap_args = [ty(0), g("Nat")];
    assert!(
        infer_type(
            &ctx,
            &apps(
                konst("asym", vec![Universe::Type(1), Universe::Type(0)]),
                &swap_args
            )
        )
        .is_ok(),
        "`asym.{{1,0}} (Type 0) Nat` type-checks — A at level 1 admits `Type 0`"
    );
    assert!(
        infer_type(
            &ctx,
            &apps(
                konst("asym", vec![Universe::Type(0), Universe::Type(1)]),
                &swap_args
            )
        )
        .is_err(),
        "…and `asym.{{0,1}}` must REFUSE the identical arguments: A is at level 0 there, and \
         `Type 0 : Type 1` does not fit. A telescope that ignores order cannot tell these apart"
    );
}

#[test]
fn an_inductive_carries_its_level_parameter_and_stays_an_inductive() {
    let mut ctx = std_ctx();
    register_box(&mut ctx);

    // POSITIVE — one declaration, two instantiations, each computing its own type.
    assert_eq!(
        infer_type(&ctx, &konst("Box", vec![Universe::Type(0)])).expect("`Box.{0}` resolves"),
        pi("A", ty(0), ty(0)),
        "`Box.{{0}} : Type 0 → Type 0`"
    );
    assert_eq!(
        infer_type(&ctx, &konst("Box", vec![Universe::Type(2)])).expect("`Box.{2}` resolves"),
        pi("A", ty(2), ty(2)),
        "`Box.{{2}} : Type 2 → Type 2` — the SAME declaration"
    );
    assert_eq!(
        infer_type(&ctx, &app(konst("Box", vec![Universe::Type(1)]), ty(0)))
            .expect("`Box.{1} (Type 0)` resolves"),
        ty(1),
        "an application of the instantiated inductive lands at its own level"
    );

    // NEGATIVE — instantiating does not make it a DEFINITION: there is no body to unfold.
    assert_eq!(
        normalize(&ctx, &konst("Box", vec![Universe::Type(0)])),
        konst("Box", vec![Universe::Type(0)]),
        "a universe-polymorphic INDUCTIVE is a value: `Box.{{0}}` must not δ-unfold to anything"
    );
    // NEGATIVE — and the level argument is checked, so the wrong arity is still refused.
    assert!(
        infer_type(
            &ctx,
            &konst("Box", vec![Universe::Type(0), Universe::Type(1)])
        )
        .is_err(),
        "`Box.{{0,1}}` must be refused — `Box` binds ONE level"
    );

    // …and it is still an inductive: constructors, and a constructor that TYPES.
    assert!(
        !ctx.get_constructors("Box").is_empty(),
        "`Box` keeps its constructors"
    );
    assert!(
        infer_type(&ctx, &apps(g("Box_mk"), &[g("Nat"), g("Zero")])).is_ok(),
        "`Box_mk Nat Zero` must type-check — the constructor's own `Sort u` binds too"
    );
}

// ============================================================================================
// 2 · LEVEL CLOSURE — a level variable nothing binds is refused, in BOTH kernels
// ============================================================================================

#[test]
fn a_declaration_whose_level_variables_its_parameters_do_not_cover_is_refused() {
    let mut ctx = std_ctx();
    // `λ(A : Sort w). A : Π(A : Sort w). Sort w` — well typed, so the ONLY thing that can refuse
    // `unbound` is its level telescope, and the ONLY thing `covered` has to pass is the check.
    let family = || pi("A", sort(uvar("w")), sort(uvar("w")));
    let identity = || lam("A", sort(uvar("w")), var("A"));
    // `unbound` declares NO universe parameters while its type mentions `w`.
    ctx.add_universe_poly("unbound", vec![], family(), identity());
    // `covered` declares `w` and uses it — the neighbouring thing that must still work.
    ctx.add_universe_poly("covered", vec!["w".to_string()], family(), identity());
    // `spare` declares a parameter it does not use — legal (⊆, not =), as in Lean.
    ctx.add_universe_poly("spare", vec!["z".to_string()], g("Nat"), g("Zero"));

    assert!(
        infer_type(&ctx, &konst("unbound", vec![])).is_err(),
        "a free level variable no parameter binds must be REFUSED, not silently retained"
    );
    let reasons: Vec<&str> = ctx
        .refused_declarations()
        .iter()
        .map(|(name, reason)| {
            assert_eq!(name, "unbound", "only `unbound` may be refused: {reason}");
            reason.as_str()
        })
        .collect();
    assert!(
        reasons.len() == 1 && reasons[0].contains("unbound universe parameter"),
        "…and refused for its COVERAGE, not for its body: {reasons:?}"
    );
    // A body whose type is not the declared one is refused by the checked door too, which is
    // why the family above is `Π(A : Sort w). Sort w`: `Π(A : Sort w). A` would say that
    // `λA. A` inhabits every type.
    let mut lying = std_ctx();
    lying.add_universe_poly(
        "lying",
        vec!["w".to_string()],
        pi("A", sort(uvar("w")), var("A")),
        identity(),
    );
    assert!(
        lying.get_universe_poly("lying").is_none(),
        "`Π(A : Sort w). A` is not the type of `λA. A`, and the door must say so"
    );
    assert!(
        infer_type(&ctx, &konst("covered", vec![Universe::Type(0)])).is_ok(),
        "the covered neighbour must still resolve — the refusal is about COVERAGE, not about \
         having level variables at all"
    );
    assert_eq!(
        normalize(
            &ctx,
            &app(konst("covered", vec![Universe::Type(0)]), g("Nat"))
        ),
        g("Nat"),
        "…and it must still δ-unfold: the refusal must not cost the legitimate case its body"
    );
    assert!(
        infer_type(&ctx, &konst("spare", vec![Universe::Type(0)])).is_ok(),
        "an UNUSED parameter is legal — the rule is `free levels ⊆ params`, not equality"
    );

    // BOTH KERNELS. The second kernel must refuse the same declaration for the same reason;
    // a rule that lives in one kernel only is a disagreement waiting to be discovered.
    match recheck(&ctx, &konst("unbound", vec![])) {
        Err(ReCheckError::Ill(_)) => {}
        other => panic!(
            "the re-checker must independently refuse a level-open declaration as ILL-TYPED, \
             got {other:?}"
        ),
    }
    assert!(
        recheck(&ctx, &konst("covered", vec![Universe::Type(0)])).is_ok(),
        "…and independently ACCEPT the covered one"
    );
}

// ============================================================================================
// 3 · THE SECOND KERNEL MUST UNFOLD A UNIVERSE-POLYMORPHIC REFERENCE
// ============================================================================================

#[test]
fn both_kernels_agree_about_what_a_universe_polymorphic_reference_means() {
    let mut ctx = std_ctx();
    // `zeroU.{u} : Nat := Zero` — a polymorphic reference whose MEANING (its body) is needed to
    // decide a conversion. If the second kernel never δ-unfolds a `Const`, it reports a
    // disagreement on a term the main kernel is right to accept.
    ctx.add_universe_poly("zeroU", vec!["u".to_string()], g("Nat"), g("Zero"));
    let at0 = konst("zeroU", vec![Universe::Type(0)]);

    assert!(
        defeq_for_test(&ctx, &at0, &g("Zero")),
        "the main kernel δ-unfolds `zeroU.{{0}}` to `Zero`"
    );
    // A proof that RESTS on that conversion: `refl Nat Zero` cited at the polymorphic spelling.
    let cited = app(
        lam("h", eq_at(g("Nat"), at0.clone(), g("Zero")), g("Zero")),
        refl(g("Nat"), g("Zero")),
    );
    assert!(
        infer_type(&ctx, &cited).is_ok(),
        "the main kernel accepts it"
    );
    assert_eq!(
        double_check(&ctx, &cited),
        DoubleCheck::Agreed,
        "and the SECOND kernel must reach the same verdict — a `Const` that is inert in one \
         kernel and transparent in the other means the two disagree about what a polymorphic \
         reference IS: {:?}",
        double_check(&ctx, &cited)
    );

    // NEGATIVE — unfolding must not launder a FALSE conversion.
    let lie = app(
        lam(
            "h",
            eq_at(g("Nat"), at0, app(g("Succ"), g("Zero"))),
            g("Zero"),
        ),
        refl(g("Nat"), g("Zero")),
    );
    assert!(
        infer_type(&ctx, &lie).is_err(),
        "`zeroU.{{0}} ≡ Succ Zero` is false and must stay refused"
    );
}

// ============================================================================================
// 4 · IMPLICIT LEVEL INSTANTIATION AT A BARE `Global`
// ============================================================================================

#[test]
fn a_bare_reference_to_a_polymorphic_declaration_solves_its_level_from_the_argument() {
    let mut ctx = std_ctx();
    register_box(&mut ctx);

    // POSITIVE — the same bare `Box` at two DIFFERENT levels in one context, each reporting the
    // level it was actually used at. A defaulting implementation reports `Type 0` twice.
    assert_eq!(
        infer_type(&ctx, &app(g("Box"), g("Nat"))).expect("`Box Nat` types"),
        ty(0),
        "`Nat : Type 0`, so `Box Nat : Type 0`"
    );
    assert_eq!(
        infer_type(&ctx, &app(g("Box"), ty(0))).expect("`Box (Type 0)` types"),
        ty(1),
        "`Type 0 : Type 1`, so `Box (Type 0) : Type 1` — the level is SOLVED, not defaulted"
    );
    assert_eq!(
        infer_type(&ctx, &app(g("Box"), g("True"))).expect("`Box True` types"),
        prop(),
        "`True : Prop`, so `Box True : Prop` — a level variable ranges over `Prop` too"
    );

    // NEGATIVE — no level metavariable may LEAK into a reported type. A `?`-named level in an
    // answer is an unsolved obligation presented as a result.
    for t in [
        app(g("Box"), g("Nat")),
        app(g("Box"), ty(0)),
        app(g("Box"), g("True")),
    ] {
        let inferred = infer_type(&ctx, &t).expect("types");
        assert!(
            !format!("{inferred}").contains('?'),
            "a solved application must report a CONCRETE level, got `{inferred}`"
        );
    }

    // NEGATIVE — solving is not accepting. `Zero : Nat` is not a sort, so there is no level to
    // solve and the application must be refused rather than instantiated with a guess.
    assert!(
        infer_type(&ctx, &app(g("Box"), g("Zero"))).is_err(),
        "`Box Zero` must be refused — `Zero` is a value, not a type, and a level metavariable \
         must never be solved by inventing one"
    );

    // NEGATIVE — cumulativity must not be widened. `Box`'s parameter is `Sort u`, so a `Type 1`
    // argument gives `Box _ : Type 2`, NOT `Type 1`.
    assert_eq!(
        infer_type(&ctx, &app(g("Box"), ty(1))).expect("`Box (Type 1)` types"),
        ty(2),
        "the solved level tracks the argument's ACTUAL sort"
    );

    // BOTH KERNELS agree about the implicitly instantiated form.
    let elem = apps(g("Box_mk"), &[g("Nat"), g("Zero")]);
    assert_eq!(
        double_check(&ctx, &elem),
        DoubleCheck::Agreed,
        "an element of a universe-polymorphic container must be verified TWICE: {:?}",
        double_check(&ctx, &elem)
    );
}

#[test]
fn two_independent_level_parameters_are_solved_independently() {
    let mut ctx = std_ctx();
    // `Pair.{u,v} : Sort u → Sort v → Type (max u v)` — the shape that defeats
    // monomorphisation, because the two levels vary independently.
    ctx.add_indexed_inductive(
        "Pair",
        pi(
            "A",
            sort(uvar("u")),
            pi("B", sort(uvar("v")), sort(uvar("u").max(&uvar("v")))),
        ),
        2,
    );

    assert_eq!(
        infer_type(&ctx, &apps(g("Pair"), &[g("Nat"), g("Bool")])).expect("`Pair Nat Bool` types"),
        ty(0),
        "both carriers at `Type 0` ⇒ `max(0,0) = Type 0`"
    );
    assert_eq!(
        infer_type(&ctx, &apps(g("Pair"), &[g("Nat"), ty(0)])).expect("`Pair Nat (Type 0)` types"),
        ty(1),
        "`R` at level 0 and `M` at level 1 ⇒ `max = Type 1`, from ONE declaration"
    );
    assert_eq!(
        infer_type(&ctx, &apps(g("Pair"), &[ty(0), g("Nat")])).expect("the mirrored application"),
        ty(1),
        "…and the mirror image, which is what makes the two levels genuinely independent"
    );
    assert_eq!(
        infer_type(&ctx, &apps(g("Pair"), &[ty(2), g("Nat")])).expect("`Pair (Type 2) Nat` types"),
        ty(3),
        "`max(3, 0) = Type 3`"
    );

    // …and the explicit spelling agrees with the solved one.
    assert_eq!(
        infer_type(
            &ctx,
            &apps(
                konst("Pair", vec![Universe::Type(0), Universe::Type(1)]),
                &[g("Nat"), ty(0)]
            )
        )
        .expect("the explicit instantiation"),
        ty(1),
        "`Pair.{{0,1}} Nat (Type 0)` must agree with the implicitly solved form"
    );
}

// ============================================================================================
// 5 · SHARING — substitution writes an argument ONCE
// ============================================================================================

#[test]
fn a_dependent_telescope_shares_its_argument_instead_of_copying_it_per_binder() {
    // `f : Π(x:Nat). Π(y0 : Eq Nat x x). … Nat` — dependent, so the non-occurrence fast path is
    // defeated at every level.
    let telescope = |n: usize| {
        let mut t = g("Nat");
        for i in 0..n {
            t = pi(&format!("y{i}"), eq_at(g("Nat"), var("x"), var("x")), t);
        }
        pi("x", g("Nat"), t)
    };
    let mut argument = g("Zero");
    for _ in 0..40 {
        argument = app(g("Succ"), argument);
    }

    let ty64 = telescope(64);
    let mut ctx = std_ctx();
    ctx.add_declaration("f", ty64.clone());
    let inferred = infer_type(&ctx, &app(g("f"), argument.clone())).expect("`f arg` types");
    assert!(
        printed_size(&inferred) <= 2 * (printed_size(&ty64) + printed_size(&argument)),
        "one argument into a 64-binder dependent telescope must be written ONCE: {} chars from \
         a {}-char telescope and a {}-char argument",
        printed_size(&inferred),
        printed_size(&ty64),
        printed_size(&argument)
    );

    // THE HEAD — a shared type must still PRESENT as a Π. Every consumer of an inferred type
    // reads its head, so a `Let` handed out at the top breaks all of them.
    assert!(
        matches!(inferred, Term::Pi { .. }),
        "the shared form must still be Π-headed: `{}`",
        &format!("{inferred}")[..60.min(format!("{inferred}").len())]
    );

    // THE NEGATIVE — sharing must not change what the type MEANS. Checked on a SMALL telescope,
    // because deciding it expands the shared form back out to the copied one (which is exactly
    // the work the shared form exists to defer).
    let small = telescope(6);
    let mut sctx = std_ctx();
    sctx.add_declaration("h", small.clone());
    let sinferred = infer_type(&sctx, &app(g("h"), argument.clone())).expect("`h arg` types");
    let eager = {
        let mut t = g("Nat");
        for i in 0..6 {
            t = pi(
                &format!("y{i}"),
                eq_at(g("Nat"), argument.clone(), argument.clone()),
                t,
            );
        }
        t
    };
    assert!(
        defeq_for_test(&sctx, &sinferred, &eager),
        "the shared form must be definitionally equal to the copied one"
    );
    assert!(
        is_subtype(&sctx, &sinferred, &eager) && is_subtype(&sctx, &eager, &sinferred),
        "…and mutually a subtype of it, so nothing downstream can tell them apart"
    );

    // THE BOUNDARY — a further application through the shared form still works, and still
    // REFUSES a wrong argument. A delayed substitution that cannot be applied through is a
    // different bug wearing the first one's clothes.
    let proof = refl(g("Nat"), argument.clone());
    assert!(
        infer_type(&sctx, &apps(g("h"), &[argument.clone(), proof])).is_ok(),
        "applying the next argument through the shared telescope must type-check"
    );
    assert!(
        infer_type(
            &sctx,
            &apps(g("h"), &[argument.clone(), refl(g("Nat"), g("Zero"))])
        )
        .is_err(),
        "…and a proof of the WRONG equation must still be refused: sharing must not make the \
         dependency invisible"
    );
}

#[test]
fn a_shared_binding_whose_variable_is_unused_leaves_no_trace_in_the_reported_type() {
    let ctx = std_ctx();
    // `Eq : Π(A:Type 0). A → A → Prop`. Applying it fully must report `Prop` EXACTLY — a
    // sharing scheme that leaves a spent `let` around changes every structural comparison
    // downstream.
    let e = eq_at(
        app(g("List"), g("Nat")),
        app(g("nil"), g("Nat")),
        app(g("nil"), g("Nat")),
    );
    assert_eq!(
        infer_type(&ctx, &e).expect("the equation types"),
        prop(),
        "a fully-applied `Eq` reports `Prop`, not a `let` whose body is `Prop`"
    );
    let partial = infer_type(&ctx, &apps(g("Eq"), &[app(g("List"), g("Nat"))]))
        .expect("the partial application types");
    assert!(
        defeq_for_test(
            &ctx,
            &partial,
            &pi(
                "x",
                app(g("List"), g("Nat")),
                pi("y", app(g("List"), g("Nat")), prop())
            ),
        ),
        "the PARTIAL application still means `List Nat -> List Nat -> Prop`, got `{partial}`"
    );
    assert!(
        matches!(partial, Term::Pi { .. }),
        "…and it still PRESENTS as a Π: every consumer that reads a function type off \
         `infer_type` matches on the head, so a shared form must be peeled before it is \
         returned, not handed out: `{partial}`"
    );
}

// ============================================================================================
// 6 · DIAGNOSTICS — three different events must not share one error
// ============================================================================================

#[test]
fn an_untrusted_axiom_is_named_as_one_while_an_ordinary_missing_name_is_not() {
    let ctx = std_ctx();

    // POSITIVE — a name the trusted base does NOT carry, but which Lean has as an axiom, must be
    // reported as a TRUSTED-BASE event that names the axiom. (A name the prelude legitimately
    // carries as a TRACKED axiom resolves instead, and is skipped: which of these LOGOS chooses
    // to trust is the prelude's decision, not this rule's.)
    let mut exercised = 0;
    for axiom in [
        "sorryAx",
        "Classical_choice",
        "propext",
        "Quot_sound",
        "em",
        "choice",
    ] {
        if ctx.get_global(axiom).is_some() {
            continue;
        }
        exercised += 1;
        let err = infer_type(&ctx, &g(axiom)).expect_err("the trusted base carries no such axiom");
        assert!(
            !matches!(err, KernelError::UnboundVariable(_)),
            "`{axiom}` must not be reported as an unbound variable — that reads as a \
             name-resolution bug and invites patching the prelude: {err:?}"
        );
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("axiom") && msg.contains(&axiom.to_lowercase()),
            "the diagnostic must say AXIOM and name it: `{err}`"
        );
    }
    assert!(
        exercised > 0,
        "at least one of Lean's axioms must stay OUTSIDE the trusted base — if the prelude has \
         grown to carry every one of them, the rule this test locks has nothing left to protect"
    );

    // NEGATIVE — every other missing name stays an ordinary unbound variable. Widening the new
    // diagnostic to all unknown globals is the wrong fix and breaks four corpus tests.
    for name in ["id", "base", "Bind_bind", "thisNameDoesNotExist"] {
        assert!(
            matches!(
                infer_type(&ctx, &g(name)),
                Err(KernelError::UnboundVariable(_))
            ),
            "`{name}` is not an axiom of anyone's trusted base — it must stay `UnboundVariable`"
        );
    }

    // …and a name the trusted base DOES carry is not a diagnostic at all.
    assert!(infer_type(&ctx, &g("Nat")).is_ok(), "`Nat` resolves");
}

#[test]
fn an_uninstantiated_metavariable_is_diagnosed_as_one_in_both_kernels() {
    let ctx = std_ctx();

    // POSITIVE — `?m` is the elaborator's own convention for an unsolved metavariable, and a
    // metavariable reaching the kernel is a different event from a typo.
    let err = infer_type(&ctx, &var("?m")).expect_err("`?m` is not in the local context");
    assert!(
        !matches!(err, KernelError::UnboundVariable(_)),
        "an uninstantiated metavariable must be reported AS ONE: {err:?}"
    );
    assert!(
        err.to_string().to_lowercase().contains("metavariable") && err.to_string().contains("?m"),
        "…and must NAME it: `{err}`"
    );

    // NEGATIVE — a `?`-named name that IS bound is an ordinary variable and must type normally.
    // `instantiate` already treats a `?`-prefixed BINDER as capturable, so the kernel must not
    // refuse one just for its spelling.
    assert_eq!(
        infer_type(&ctx, &lam("?m", g("Nat"), var("?m"))).expect("a bound `?m` is ordinary"),
        pi("?m", g("Nat"), g("Nat")),
        "a `?`-named BINDER is a binder"
    );
    // NEGATIVE — an ordinary unbound local stays an unbound variable.
    assert!(
        matches!(
            infer_type(&ctx, &var("zzz")),
            Err(KernelError::UnboundVariable(_))
        ),
        "an ordinary unbound local is still `UnboundVariable`"
    );
    // The elaborator's own convention is unchanged: an unsolved `?m` passes through
    // `instantiate` untouched, which is what makes the kernel the backstop.
    assert_eq!(instantiate(&var("?m"), &MetaCtx::new()), var("?m"));

    // BOTH KERNELS — the re-checker must refuse it too, so `double_check` cannot report a term
    // with a live metavariable as independently verified.
    match recheck(&ctx, &var("?m")) {
        Err(ReCheckError::Ill(m)) => assert!(
            m.to_lowercase().contains("metavariable"),
            "the second kernel must give the same diagnosis: {m}"
        ),
        other => panic!("the re-checker must refuse a live metavariable: {other:?}"),
    }
}

// ============================================================================================
// 7 · THE SECOND KERNEL RUNS A RECURSION OVER A `Nat` LITERAL — AND STAYS BOUNDED
// ============================================================================================

#[test]
fn the_re_checker_runs_a_literal_recursion_and_still_terminates_on_a_stuck_one() {
    let ctx = std_ctx();

    // POSITIVE — `decEqNat 2 2` is a structural recursion over two `Nat` LITERALS. Both kernels
    // must run it, or every `Fin`/`Char`/bound proof is single-kernel evidence.
    let decided = apps(
        g("decide"),
        &[
            eq_at(g("Nat"), natl(2), natl(2)),
            apps(g("decEqNat"), &[natl(2), natl(2)]),
        ],
    );
    let cited = app(
        lam("h", eq_at(g("Bool"), decided, g("true")), g("Zero")),
        refl(g("Bool"), g("true")),
    );
    assert_eq!(
        double_check(&ctx, &cited),
        DoubleCheck::Agreed,
        "a recursion over a Nat literal must compute in BOTH kernels: {:?}",
        double_check(&ctx, &cited)
    );

    // NEGATIVE — and the answer must still be DECIDED, not merely produced: `2 = 3` is false.
    let wrong = apps(
        g("decide"),
        &[
            eq_at(g("Nat"), natl(2), natl(3)),
            apps(g("decEqNat"), &[natl(2), natl(3)]),
        ],
    );
    let lie = app(
        lam("h", eq_at(g("Bool"), wrong, g("true")), g("Zero")),
        refl(g("Bool"), g("true")),
    );
    assert!(
        infer_type(&ctx, &lie).is_err(),
        "`decide (2 = 3) ⤳ false`, so citing `true` must be refused by the main kernel"
    );
    assert!(
        recheck(&ctx, &lie).is_err(),
        "…and the second kernel must not certify it either: {:?}",
        recheck(&ctx, &lie)
    );
    // Both kernels rejecting IS agreement, and the strongest rejection the API reports — so the
    // verdict must be `Agreed`-by-rejection, never an acceptance by either side.
    assert_eq!(
        double_check(&ctx, &lie),
        DoubleCheck::Agreed,
        "two independent rejections are an agreement"
    );

    // THE BOUNDARY — letting the guard go must not let the RE-CHECKER run forever. A recursion
    // over a large literal has to come back with a verdict (either an acceptance, or an honest
    // `Unsupported` because the meter stopped it) rather than exhausting the native stack.
    //
    // Asked of `recheck` alone, not `double_check`: this is a claim about the SECOND kernel's
    // bounds, and mixing in the main kernel — whose own depth residue `MAX_TERM_DEPTH` documents
    // and `lean_conformance_t00_terms::a_deep_spine_costs_linearly…` still reproduces — would measure
    // two different things with one assertion.
    let big = apps(
        g("decide"),
        &[
            eq_at(g("Nat"), natl(400), natl(400)),
            apps(g("decEqNat"), &[natl(400), natl(400)]),
        ],
    );
    let big_cited = app(
        lam("h", eq_at(g("Bool"), big, g("true")), g("Zero")),
        refl(g("Bool"), g("true")),
    );
    match recheck(&ctx, &big_cited) {
        Ok(_) => {}
        Err(ReCheckError::Unsupported(_)) => {}
        Err(ReCheckError::Ill(why)) => panic!(
            "a 400-step literal recursion must not be turned into a soundness alarm — a \
             reduction the meter cut short is `Unsupported`, not ill-typed: {why}"
        ),
    }
}

// ============================================================================================
// 8 · THE RE-CHECKER MUST NEVER ANSWER "THE PROCESS IS GONE"
// ============================================================================================

/// A checker whose answer to a hard input is a `SIGABRT` cannot be part of a trusted base: the
/// caller cannot tell it from a crash in its own code, and there is no error channel to report.
/// So the re-checker's reduction is METERED, and this is the lock: the heaviest verification the
/// prelude asks for — a `String`, whose element type `Char` is a `Subtype` whose bound is a
/// structural recursion over six-figure `Nat` literals — must come back with a VERDICT.
///
/// Run on a 2 MiB thread — the stack a spawned Rust thread gets by default, and therefore the
/// budget the kernel must live inside wherever it is embedded, not the roomy main thread the
/// test harness happens to provide.
#[test]
fn the_re_checker_returns_a_verdict_on_a_string_instead_of_exhausting_the_stack() {
    let handle = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let ctx = std_ctx();
            let empty = app(g("String_mk"), app(g("nil"), g("Char")));
            let verdict = double_check(&ctx, &empty);
            format!("{verdict:?}")
        })
        .expect("spawn a small-stack checker thread");
    let verdict = handle
        .join()
        .expect("the re-checker must RETURN on a small stack, never abort the process");
    assert!(
        !verdict.starts_with("Disagree"),
        "…and the verdict must not be a false alarm: {verdict}"
    );
}

// ============================================================================================
// 9 · SPROP LARGE ELIMINATION — the gate, at every door, in both kernels
// ============================================================================================

/// `SProp` is LOGOS's own bottom sort: definitional irrelevance identifies ANY two inhabitants of
/// an `SProp` type. That makes large elimination out of one an INCONSISTENCY rather than a
/// precision loss — a `match` from a two-constructor `SProp` into `Nat` equates `0` and `1`, and
/// transporting along the identification proves `0 = 1`.
///
/// The gate lives at three doors — the non-indexed `Match`, the INDEXED `Match`, and the second
/// kernel's own `Match` — and this is the lock that all three are shut, with the neighbouring
/// legal elimination (into `SProp` itself, and out of a `Prop`) still open so the gate cannot be
/// satisfied by refusing everything.
#[test]
fn sprop_large_elimination_is_refused_at_every_door_and_by_both_kernels() {
    let mut ctx = std_ctx();
    // `STwo : SProp` with TWO constructors — a multi-constructor proposition in the
    // definitionally-irrelevant bottom sort.
    ctx.add_inductive("STwo", sort(Universe::SProp));
    ctx.add_constructor("sfirst", "STwo", g("STwo"));
    ctx.add_constructor("ssecond", "STwo", g("STwo"));
    // …and a ONE-constructor one, the subsingleton temptation: for `Prop` a single constructor
    // with propositional fields earns large elimination, and for `SProp` it must NOT, because
    // irrelevance is definitional there rather than derived.
    ctx.add_inductive("SUnit", sort(Universe::SProp));
    ctx.add_constructor("sunit", "SUnit", g("SUnit"));

    let discriminate = |ind: &str, cases: Vec<Term>| {
        lam(
            "s",
            g(ind),
            Term::Match {
                discriminant: Box::new(var("s")),
                motive: Box::new(lam("_", g(ind), g("Nat"))),
                cases,
            },
        )
    };

    // The premise that makes it an inconsistency rather than a coarseness.
    assert!(
        defeq_for_test(&ctx, &g("sfirst"), &g("ssecond")),
        "`SProp` irrelevance identifies the two constructors"
    );

    // DOOR 1 — the non-indexed `Match`, two constructors.
    let two = discriminate("STwo", vec![g("Zero"), app(g("Succ"), g("Zero"))]);
    assert!(
        infer_type(&ctx, &two).is_err(),
        "large elimination out of a two-constructor `SProp` must be refused: {:?}",
        infer_type(&ctx, &two).map(|t| t.to_string())
    );
    // DOOR 1b — one constructor is no exception.
    let one = discriminate("SUnit", vec![g("Zero")]);
    assert!(
        infer_type(&ctx, &one).is_err(),
        "…and a ONE-constructor `SProp` is no exception — irrelevance is definitional, so the \
         subsingleton reasoning that licenses `And`/`Eq` in `Prop` does not transfer: {:?}",
        infer_type(&ctx, &one).map(|t| t.to_string())
    );

    // DOOR 2 — the INDEXED `Match` arm is a separate code path with its own copy of the rule.
    ctx.add_indexed_inductive("SIdx", pi("n", g("Nat"), sort(Universe::SProp)), 0);
    ctx.add_constructor("sidx", "SIdx", app(g("SIdx"), g("Zero")));
    let indexed = Term::Match {
        discriminant: Box::new(app(g("sidx"), g("Zero"))),
        motive: Box::new(lam(
            "i",
            g("Nat"),
            lam("_", app(g("SIdx"), var("i")), g("Nat")),
        )),
        cases: vec![g("Zero")],
    };
    assert!(
        infer_type(&ctx, &indexed).is_err(),
        "the INDEXED match arm carries its own copy of the gate and must shut too: {:?}",
        infer_type(&ctx, &indexed).map(|t| t.to_string())
    );

    // DOOR 3 — the second kernel, independently. A gate in one kernel only is a gate that a
    // certificate can walk around.
    for (what, term) in [("two-constructor", &two), ("one-constructor", &one)] {
        assert!(
            recheck(&ctx, term).is_err(),
            "the re-checker must refuse the {what} `SProp` elimination independently: {:?}",
            recheck(&ctx, term)
        );
        assert_eq!(
            double_check(&ctx, term),
            DoubleCheck::Agreed,
            "two independent refusals are an agreement"
        );
    }

    // THE NEGATIVE — the gate must not refuse what is legal, or it is satisfied by a checker
    // that says no to everything. Eliminating an `SProp` into `SProp` is fine…
    ctx.add_declaration("sproofOf", arrow(g("STwo"), g("SUnit")));
    let into_sprop = lam(
        "s",
        g("STwo"),
        Term::Match {
            discriminant: Box::new(var("s")),
            motive: Box::new(lam("_", g("STwo"), g("SUnit"))),
            cases: vec![g("sunit"), g("sunit")],
        },
    );
    assert!(
        infer_type(&ctx, &into_sprop).is_ok(),
        "`SProp → SProp` elimination is legal and must stay legal: {:?}",
        infer_type(&ctx, &into_sprop)
    );
    // …and so is the `Prop` subsingleton large elimination the gate must not touch: `False` has
    // no constructors, so `ex falso` into `Type 0` is exactly Lean's `False.elim`.
    ctx.add_declaration("hf", g("False"));
    let ex_falso = Term::Match {
        discriminant: Box::new(g("hf")),
        motive: Box::new(lam("_", g("False"), ty(0))),
        cases: vec![],
    };
    assert!(
        infer_type(&ctx, &ex_falso).is_ok(),
        "`False.elim` into `Type 0` must still work — the gate is about `SProp`, not about \
         large elimination in general: {:?}",
        infer_type(&ctx, &ex_falso)
    );
}

// ============================================================================================
// 10 · THE `Nat`-ORDER SHORTCUT IS AN ACCELERATION, NOT A SECOND DEFINITION
// ============================================================================================

/// The re-checker decides the `Nat` order by arithmetic where the prelude decides it by structural
/// recursion. That is the second kernel doing the same maths INDEPENDENTLY — which is the whole
/// point of a second kernel — but it is only sound if the two agree everywhere the recursion
/// reaches. This is that differential, over the whole small grid plus the boundaries, against the
/// MAIN kernel's reduction of the prelude's own definition.
///
/// If `natLe` is ever redefined, this goes red — which is the lock the shortcut needs, since
/// nothing else in the suite compares the two routes.
#[test]
fn the_nat_order_shortcut_agrees_with_the_structural_recursion_everywhere_it_answers() {
    let ctx = std_ctx();

    // The grid: every ordered pair below 12, which covers `m < n`, `m = n`, `m > n`, and both
    // zeroes (the two base cases of the recursion).
    for m in 0..12i64 {
        for n in 0..12i64 {
            for (op, expected) in [("natLe", m <= n), ("natLt", m < n)] {
                let want = g(if expected { "true" } else { "false" });
                // The MAIN kernel, running the prelude's structural recursion.
                assert_eq!(
                    normalize(&ctx, &apps(g(op), &[natl(m), natl(n)])),
                    want,
                    "`{op} {m} {n}` by structural recursion"
                );
                // The SECOND kernel, deciding the same thing by arithmetic: a proof citing the
                // computed answer must re-check.
                let cited = app(
                    lam(
                        "h",
                        eq_at(g("Bool"), apps(g(op), &[natl(m), natl(n)]), want.clone()),
                        g("Zero"),
                    ),
                    refl(g("Bool"), want.clone()),
                );
                assert!(
                    infer_type(&ctx, &cited).is_ok(),
                    "the main kernel accepts `{op} {m} {n} = {want}`"
                );
                assert_eq!(
                    double_check(&ctx, &cited),
                    DoubleCheck::Agreed,
                    "…and the re-checker must reach the SAME answer for `{op} {m} {n}`: {:?}",
                    double_check(&ctx, &cited)
                );
                // THE NEGATIVE — the opposite answer must be refused by both, or the shortcut is
                // returning a constant rather than deciding anything.
                let opposite = g(if expected { "false" } else { "true" });
                let lie = app(
                    lam(
                        "h",
                        eq_at(g("Bool"), apps(g(op), &[natl(m), natl(n)]), opposite),
                        g("Zero"),
                    ),
                    refl(g("Bool"), want),
                );
                assert!(
                    infer_type(&ctx, &lie).is_err(),
                    "the wrong answer for `{op} {m} {n}` must be refused"
                );
                assert!(recheck(&ctx, &lie).is_err(), "…by the second kernel too");
            }
        }
    }

    // THE BOUNDARY the shortcut exists for: a small index into a HUGE range. The structural
    // recursion costs `min(m, n)`, so this is cheap in both kernels — and the answer is the same.
    let huge = Term::Lit(Literal::Nat(
        logicaffeine_base::BigInt::parse_decimal("18446744073709551616").expect("2^64"),
    ));
    assert_eq!(
        normalize(&ctx, &apps(g("natLt"), &[natl(5), huge.clone()])),
        g("true"),
        "`5 < 2^64` decides in steps proportional to 5"
    );
    let bounded = app(
        lam(
            "h",
            eq_at(
                g("Bool"),
                apps(g("natLt"), &[natl(5), huge.clone()]),
                g("true"),
            ),
            g("Zero"),
        ),
        refl(g("Bool"), g("true")),
    );
    assert_eq!(
        double_check(&ctx, &bounded),
        DoubleCheck::Agreed,
        "…and both kernels agree about it: {:?}",
        double_check(&ctx, &bounded)
    );

    // THE CEILING, and it is the load-bearing half: past the structural recursion's reach the two
    // kernels must answer in exactly the same places. Both accelerate the order (section 17, and
    // Lean's `Nat.ble` reduction), so `2^64 < 2^64` is DECIDED by both — and the one thing that
    // may never happen is one kernel deciding a bound its partner abandons, which `double_check`
    // would report as a soundness alarm about a valid proof.
    let both_huge = apps(g("natLt"), &[huge.clone(), huge.clone()]);
    assert_eq!(
        normalize(&ctx, &both_huge),
        g("false"),
        "the main kernel decides `2^64 < 2^64`, past the recursion's reach"
    );
    let cite = |answer: &str| {
        app(
            lam(
                "h",
                eq_at(g("Bool"), both_huge.clone(), g(answer)),
                g("Zero"),
            ),
            refl(g("Bool"), g(answer)),
        )
    };
    assert!(
        infer_type(&ctx, &cite("false")).is_ok(),
        "so a proof citing the answer is accepted"
    );
    assert_eq!(
        double_check(&ctx, &cite("false")),
        DoubleCheck::Agreed,
        "…by the second kernel too, which decides the same bound: {:?}",
        double_check(&ctx, &cite("false"))
    );
    assert!(
        infer_type(&ctx, &cite("true")).is_err(),
        "the wrong answer is refused"
    );
    assert!(
        recheck(&ctx, &cite("true")).is_err(),
        "…by the second kernel too"
    );
    assert!(
        !matches!(double_check(&ctx, &cite("true")), DoubleCheck::Disagree(_)),
        "…and neither kernel reaches past its partner: {:?}",
        double_check(&ctx, &cite("true"))
    );
}

// ============================================================================================
// 11 · INTEGER DIVISION — ONE SPECIFICATION, TWO INDEPENDENT IMPLEMENTATIONS
// ============================================================================================
//
// The shared specification, restated here so a wrong implementation cannot quietly agree with a
// wrong oracle:
//
//   For a dividend `a` and a divisor `b ≠ 0`, `div a b` and `mod a b` are the unique `q` and
//   `r` with  `a = b·q + r`  and  `0 ≤ r < |b|`.  The remainder is NEVER negative.
//   At `b = 0` the rule is totalised as Lean totalises it: `q = 0`, `r = a` — the only pair
//   that keeps `b·q + r = a` when `b` is zero.
//
// Truncating division (Rust's `/` and `%`, and `BigInt::div_rem`) gives the remainder the
// DIVIDEND's sign instead, so the two conventions agree on every non-negative dividend and part
// company on every negative one. That is exactly why a truncating second kernel survived until
// something asked it about `-7 / 2`.
//
// These tests are ORACLE-FREE where it matters: rather than comparing the kernels' answer to a
// transcribed table, they assert that the kernels' OWN answer satisfies the defining property,
// checked BY THE KERNELS THEMSELVES, and that the truncated answer is REFUSED wherever the two
// conventions differ. A weak implementation cannot satisfy both halves.

/// An integer literal in the kernel's canonical spelling — `Int` when it fits a machine word,
/// `BigInt` above it — so every value has ONE literal and definitional equality of literals
/// stays sound. `i128` is the test's own arithmetic, wide enough to hold both the `i64`
/// boundary and the beyond-`i64` grid without itself overflowing.
fn intl(v: i128) -> Term {
    Term::Lit(logicaffeine_kernel::int_lit(
        logicaffeine_base::BigInt::parse_decimal(&v.to_string()).expect("decimal literal"),
    ))
}

/// The integer a literal denotes, whichever width it is stored at.
fn int_value(t: &Term) -> Option<i128> {
    match t {
        Term::Lit(Literal::Int(n)) => Some(*n as i128),
        Term::Lit(Literal::BigInt(n)) => n.to_string().parse::<i128>().ok(),
        _ => None,
    }
}

fn bin(op: &str, x: Term, y: Term) -> Term {
    apps(g(op), &[x, y])
}

/// `(λ(h : Eq ty redex result). h) (refl ty result)` — a term that is well-typed exactly when
/// the kernel doing the checking reduces `redex` to `result`. Applying it through BOTH kernels
/// is how a reduction rule is compared without either kernel being asked to grade itself.
fn cites(ty: Term, redex: Term, result: Term) -> Term {
    let claim = eq_at(ty.clone(), redex, result.clone());
    app(lam("h", claim, var("h")), refl(ty, result))
}

/// Both operands' signs, both sides of the quotient boundary, an exact division, the zero
/// dividend, the zero DIVISOR Lean totalises, the whole `i64` boundary (including the one
/// division that overflows the machine quotient), and values that only the arbitrary-precision
/// path can represent.
fn division_grid() -> Vec<(i128, i128)> {
    let min = i64::MIN as i128;
    let max = i64::MAX as i128;
    let big = 1i128 << 70;
    let mut out = vec![
        (-7, 2),
        (7, 2),
        (7, -2),
        (-7, -2),
        (-1, 3),
        (1, 3),
        (-6, 3),
        (6, 3),
        (6, -3),
        (-6, -3),
        (0, 3),
        (0, -3),
        (5, 0),
        (-5, 0),
        (0, 0),
        (min, 1),
        (min, 2),
        (min, -2),
        (min, -1),
        (min, min),
        (max, 1),
        (max, 2),
        (max, -2),
        (max, -1),
        (max, min),
        (1, min),
        (-1, min),
        (min, 0),
        (big, 3),
        (-big, 3),
        (big, -3),
        (-big, -3),
        (big, 0),
        (-big, 0),
        (big + 1, big),
        (-(big + 1), big),
        (-(big + 1), -big),
    ];
    out.dedup();
    out
}

/// The whole rule, from both ends, in both kernels.
///
/// For every cell: the main kernel's own answer is read out, the RE-CHECKER is made to verify
/// the defining property about that answer (so the second kernel does the arithmetic itself
/// rather than being handed a verdict), and the TRUNCATED answer is refused wherever the two
/// conventions differ. The last clause is the load-bearing one — a positive-only test is
/// satisfied by a truncating implementation on two thirds of the grid.
#[test]
fn the_two_kernels_compute_one_euclidean_division_over_the_whole_sign_and_width_grid() {
    let ctx = std_ctx();
    let int = || g("Int");

    for (a, b) in division_grid() {
        let (at, bt) = (intl(a), intl(b));
        let quotient = normalize(&ctx, &bin("div", at.clone(), bt.clone()));
        let remainder = normalize(&ctx, &bin("mod", at.clone(), bt.clone()));
        let q = int_value(&quotient)
            .unwrap_or_else(|| panic!("`div {a} {b}` must COMPUTE, got {quotient}"));
        let r = int_value(&remainder)
            .unwrap_or_else(|| panic!("`mod {a} {b}` must COMPUTE, got {remainder}"));

        // THE DEFINING PROPERTY, decided inside the kernels rather than in the test: the
        // dividend is reconstructed from the kernel's own quotient and remainder.
        let reconstructed = bin(
            "add",
            bin("mul", bt.clone(), quotient.clone()),
            remainder.clone(),
        );
        let identity = cites(int(), reconstructed, at.clone());
        assert!(
            infer_type(&ctx, &identity).is_ok(),
            "`{b}·({q}) + ({r}) = {a}` must hold in the main kernel"
        );
        assert_eq!(
            double_check(&ctx, &identity),
            DoubleCheck::Agreed,
            "…and the SECOND kernel must reconstruct the same dividend for ({a}, {b}): {:?}",
            double_check(&ctx, &identity)
        );

        // …and the two kernels must agree about the quotient and the remainder THEMSELVES,
        // not merely about a sum that could absorb two compensating errors.
        for (op, want) in [("div", quotient.clone()), ("mod", remainder.clone())] {
            let cited = cites(int(), bin(op, at.clone(), bt.clone()), want.clone());
            assert_eq!(
                double_check(&ctx, &cited),
                DoubleCheck::Agreed,
                "both kernels must reduce `{op} {a} {b}` to {want}: {:?}",
                double_check(&ctx, &cited)
            );
        }

        if b == 0 {
            // THE TOTALISATION, pinned as a value rather than inferred from the property
            // above (which every pair `(q, a - 0·q)` would satisfy).
            assert_eq!(
                (q, r),
                (0, a),
                "Lean totalises `{a} / 0 = 0` and `{a} % 0 = {a}`"
            );
            continue;
        }

        // THE BOUND `0 ≤ r < |b|`, decided by the kernels' own order primitives.
        let magnitude = intl(if b < 0 { -b } else { b });
        for (claim, redex) in [
            ("0 ≤ r", bin("le", intl(0), remainder.clone())),
            ("r < |b|", bin("lt", remainder.clone(), magnitude)),
        ] {
            let bound = cites(g("Bool"), redex, g("true"));
            assert!(
                infer_type(&ctx, &bound).is_ok(),
                "`{claim}` must hold for ({a}, {b}), where the kernel answered r = {r}"
            );
            assert_eq!(
                double_check(&ctx, &bound),
                DoubleCheck::Agreed,
                "…and both kernels must decide `{claim}` for ({a}, {b}): {:?}",
                double_check(&ctx, &bound)
            );
        }

        // THE NEGATIVE. Wherever truncation and the Euclidean rule part company, the truncated
        // answer must be REFUSED — by both kernels. Without this clause a second kernel that
        // truncates passes every cell with a non-negative dividend.
        let (tq, tr) = (a / b, a % b);
        for (op, truncated, euclidean) in [("div", tq, q), ("mod", tr, r)] {
            if truncated == euclidean {
                continue;
            }
            let lie = cites(int(), bin(op, at.clone(), bt.clone()), intl(truncated));
            assert!(
                infer_type(&ctx, &lie).is_err(),
                "the truncated `{op} {a} {b} = {truncated}` must be refused — the Euclidean \
                 answer is {euclidean}"
            );
            assert!(
                recheck(&ctx, &lie).is_err(),
                "…and refused by the second kernel too, or a truncating re-checker is \
                 certifying `{op} {a} {b} = {truncated}` behind the main kernel's back"
            );
        }
    }
}

/// The zero divisor on its own, from both directions, because it is the one cell where the
/// defining property alone does not pin an answer and where a truncating implementation does
/// not merely disagree — it gets STUCK, which reads as "no verdict" rather than "wrong".
#[test]
fn a_zero_divisor_computes_in_both_kernels_and_only_to_leans_answer() {
    let ctx = std_ctx();
    let int = || g("Int");

    for a in [
        0i128,
        5,
        -5,
        i64::MIN as i128,
        i64::MAX as i128,
        1i128 << 70,
    ] {
        for (op, want) in [("div", 0i128), ("mod", a)] {
            let redex = bin(op, intl(a), intl(0));
            assert_eq!(
                normalize(&ctx, &redex),
                intl(want),
                "`{op} {a} 0` is total and computes to {want}"
            );
            let cited = cites(int(), redex.clone(), intl(want));
            assert_eq!(
                double_check(&ctx, &cited),
                DoubleCheck::Agreed,
                "…and the second kernel must COMPUTE it too, not leave it stuck: {:?}",
                double_check(&ctx, &cited)
            );
            // THE NEGATIVE, one rung either side: totality must not become "anything goes".
            for wrong in [want + 1, want - 1] {
                let lie = cites(int(), redex.clone(), intl(wrong));
                assert!(
                    infer_type(&ctx, &lie).is_err(),
                    "`{op} {a} 0 = {wrong}` must be refused"
                );
                assert!(recheck(&ctx, &lie).is_err(), "…by the second kernel too");
            }
        }
    }
}

/// THE WIDTH BOUNDARY. The narrow (`i64`) and wide (`BigInt`) paths are two implementations of
/// the same rule inside each kernel, and the seam between them is where a promotion bug hides:
/// `i64::MIN / -1` has no machine quotient at all, so it must PROMOTE rather than wrap, stick or
/// panic — and the promoted answer must be the one the wide path would have given anyway.
#[test]
fn the_division_that_overflows_the_machine_quotient_promotes_in_both_kernels() {
    let ctx = std_ctx();
    let min = i64::MIN as i128;

    let quotient = normalize(&ctx, &bin("div", intl(min), intl(-1)));
    assert_eq!(
        int_value(&quotient),
        Some(-min),
        "`i64::MIN / -1` is 2^63 — outside `i64`, so the kernel must widen instead of wrapping"
    );
    assert!(
        matches!(quotient, Term::Lit(Literal::BigInt(_))),
        "…and the result must be stored at the width that can hold it, got {quotient}"
    );
    assert_eq!(
        normalize(&ctx, &bin("mod", intl(min), intl(-1))),
        intl(0),
        "`i64::MIN % -1` is 0 under every convention"
    );

    for (op, want) in [("div", -min), ("mod", 0)] {
        let cited = cites(g("Int"), bin(op, intl(min), intl(-1)), intl(want));
        assert_eq!(
            double_check(&ctx, &cited),
            DoubleCheck::Agreed,
            "the second kernel must promote at the same seam for `{op}`: {:?}",
            double_check(&ctx, &cited)
        );
    }

    // THE NEGATIVE: the WRAPPED answer — what an unchecked `i64` multiply/divide would produce —
    // must be refused by both, since it is the value a promotion bug would hand back.
    let wrapped = cites(g("Int"), bin("div", intl(min), intl(-1)), intl(min));
    assert!(
        infer_type(&ctx, &wrapped).is_err(),
        "the wrapped quotient must be refused"
    );
    assert!(
        recheck(&ctx, &wrapped).is_err(),
        "…by the second kernel too"
    );
}

// ============================================================================================
// 12 · THE PRIMITIVE TABLE IS A SPECIFICATION, NOT A PER-KERNEL ACCIDENT
// ============================================================================================
//
// A second kernel earns its keep by reaching the same verdict INDEPENDENTLY. A rule the main
// kernel computes and the re-checker does not is not a safe shortfall: `double_check` reports
// `Disagree` — a SOUNDNESS ALARM — on a proof that is perfectly valid, and an alarm that fires
// on valid proofs is an alarm nobody reads. The re-checker's step budget carries exactly this
// argument in its own comment; the primitive TABLE needs it just as much.

/// A context in which the `Text` operations are declared — the shape a user reaches for, and
/// the shape `lean_conformance_t10_literals::string_concatenation_computes` asks the prelude for.
fn text_ctx() -> Context {
    let mut ctx = std_ctx();
    let t = || g("Text");
    ctx.add_declaration("Text_append", arrow(t(), arrow(t(), t())));
    ctx.add_declaration("Text_beq", arrow(t(), arrow(t(), g("Bool"))));
    ctx.add_declaration("Text_isPrefixOf", arrow(t(), arrow(t(), g("Bool"))));
    ctx.add_declaration("Text_length", arrow(t(), g("Nat")));
    ctx
}

fn textl(s: &str) -> Term {
    Term::Lit(Literal::Text(s.to_string()))
}

/// Every computation rule the main kernel has, the second kernel must have — or the second
/// kernel manufactures a disagreement about a valid proof.
#[test]
fn every_primitive_the_main_kernel_computes_the_second_kernel_computes_too() {
    let ctx = text_ctx();

    // (carrier, redex, value) — one row per rule in the main kernel's table.
    let rows: Vec<(Term, Term, Term)> = vec![
        (
            g("Text"),
            bin("Text_append", textl("ab"), textl("cd")),
            textl("abcd"),
        ),
        (
            g("Text"),
            bin("Text_append", textl(""), textl("x")),
            textl("x"),
        ),
        (
            g("Bool"),
            bin("Text_beq", textl("ab"), textl("ab")),
            g("true"),
        ),
        (
            g("Bool"),
            bin("Text_beq", textl("ab"), textl("ac")),
            g("false"),
        ),
        (
            g("Bool"),
            bin("Text_isPrefixOf", textl("ab"), textl("abc")),
            g("true"),
        ),
        (
            g("Bool"),
            bin("Text_isPrefixOf", textl("b"), textl("abc")),
            g("false"),
        ),
        (g("Nat"), app(g("Text_length"), textl("abc")), natl(3)),
        // Characters, not bytes — the same rule Lean's `String.length` states.
        (g("Nat"), app(g("Text_length"), textl("héllo")), natl(5)),
        (g("Nat"), app(g("Text_length"), textl("")), natl(0)),
    ];

    for (carrier, redex, value) in rows {
        assert_eq!(
            normalize(&ctx, &redex),
            value,
            "the main kernel computes `{redex}`"
        );
        let cited = cites(carrier.clone(), redex.clone(), value.clone());
        assert!(
            infer_type(&ctx, &cited).is_ok(),
            "…so a proof citing it type-checks in the main kernel"
        );
        assert_eq!(
            double_check(&ctx, &cited),
            DoubleCheck::Agreed,
            "…and the SECOND kernel must compute `{redex}` too. A rule only one kernel has \
             makes `double_check` report a soundness alarm about a valid proof: {:?}",
            double_check(&ctx, &cited)
        );
        // THE NEGATIVE, in the same context: the rule must DECIDE, not merely accept. A
        // re-checker that returned the first literal it saw would pass the clause above.
        let lie = cites(carrier, redex.clone(), textl("\u{0}not the answer"));
        assert!(
            recheck(&ctx, &lie).is_err(),
            "the second kernel must refuse a wrong value for `{redex}`"
        );
    }
}

/// THE BOUNDARY, one rung either side of "two text literals": the rules must stay STUCK on
/// anything else, in BOTH kernels. A primitive that fires on a non-literal or on the wrong
/// literal kind would read an `Int` as a `Text`.
#[test]
fn the_text_primitives_stay_stuck_on_the_wrong_operands_in_both_kernels() {
    let mut ctx = text_ctx();
    ctx.add_declaration("t", g("Text"));

    let stuck = [
        bin("Text_append", textl("ab"), var("t")),
        bin("Text_append", g("t"), textl("cd")),
        bin("Text_beq", intl(1), intl(1)),
        bin("Text_isPrefixOf", natl(1), textl("a")),
        app(g("Text_length"), g("t")),
        app(g("Text_length"), intl(3)),
    ];
    for redex in stuck {
        assert_eq!(
            normalize(&ctx, &redex),
            redex,
            "`{redex}` has no computation rule and must stay stuck in the main kernel"
        );
        // …and the second kernel must not invent one either: a term whose head is a stuck
        // primitive is only convertible with itself.
        let self_citation = cites(g("Text"), redex.clone(), redex.clone());
        assert!(
            !matches!(double_check(&ctx, &self_citation), DoubleCheck::Disagree(_)),
            "the second kernel must treat `{redex}` as stuck, exactly as the first does: {:?}",
            double_check(&ctx, &self_citation)
        );
    }
}

/// A primitive rule fires on a NAME, so it must never outrank a DEFINITION of that name — in
/// EITHER kernel. The main kernel gates on `get_definition_body`; the second kernel must reach
/// the same verdict, or a context that defines `Text_append` is checked against one rule and
/// reduced by another.
#[test]
fn a_definition_outranks_the_primitive_of_the_same_name_in_both_kernels() {
    let mut ctx = std_ctx();
    let t = || g("Text");
    // `Text_append := λa. λb. a` — a definition that DISAGREES with the primitive, so which
    // rule won is observable rather than a matter of taste.
    ctx.add_definition(
        "Text_append".to_string(),
        arrow(t(), arrow(t(), t())),
        lam("a", t(), lam("b", t(), var("a"))),
    );
    let redex = bin("Text_append", textl("ab"), textl("cd"));
    assert_eq!(
        normalize(&ctx, &redex),
        textl("ab"),
        "the DEFINITION's body decides, not the built-in concatenation rule"
    );
    let cited = cites(t(), redex.clone(), textl("ab"));
    assert_eq!(
        double_check(&ctx, &cited),
        DoubleCheck::Agreed,
        "…and both kernels must agree about which rule won: {:?}",
        double_check(&ctx, &cited)
    );
    let primitive_answer = cites(t(), redex, textl("abcd"));
    assert!(
        infer_type(&ctx, &primitive_answer).is_err(),
        "the built-in answer must be refused once the name is defined"
    );
    assert!(
        recheck(&ctx, &primitive_answer).is_err(),
        "…by the second kernel too"
    );
}

// ============================================================================================
// 13 · THE CEILING OF THE LEVEL REPRESENTATION
// ============================================================================================

/// `Universe::Type` holds a `u32`, so at `u32::MAX` the successor has no representable `Type`.
/// Both machine answers are catastrophic and neither is a judgement:
///
/// * **Wrapping** gives `Type u32::MAX : Type 0`. Since `Type 0 ≤ Type u32::MAX` by
///   cumulativity, a universe then contains itself — Girard's paradox, from which a closed term
///   of type `False` is derivable and every proposition follows.
/// * **Panicking** makes the kernel's answer to a malformed input "the process is gone", which
///   is exactly the failure the re-checker's own depth door exists to prevent: a checker that
///   aborts cannot be part of a trusted base.
///
/// There is a third answer, and it is the only one that is both total and sound: the ℕ-model
/// the ordering is decided over (`term::to_lnat`) works in `u64`, so a SYMBOLIC successor is
/// representable, strictly larger, and fully comparable. This pins it from both sides, one rung
/// either way, and in both kernels.
#[test]
fn the_top_representable_universe_still_has_a_strictly_larger_successor() {
    let ctx = std_ctx();
    let top = Universe::Type(u32::MAX);
    let above = top.succ();

    assert_ne!(
        above,
        Universe::Type(0),
        "the successor must not WRAP to the bottom of the tower"
    );
    assert!(
        top.is_subtype_of(&above),
        "`Type u32::MAX ≤ succ(Type u32::MAX)`"
    );
    assert!(
        !above.is_subtype_of(&top),
        "…and STRICTLY so: `succ(Type u32::MAX) ≤ Type u32::MAX` closes the hierarchy into a \
         cycle at the top, and Girard's paradox derives `False` from a universe that contains \
         itself"
    );
    assert!(!top.equiv(&above), "a universe is never its own successor");

    // ONE RUNG BELOW: the ordinary rule is untouched, so the repair is at the ceiling and not a
    // change of the level algebra.
    assert_eq!(
        Universe::Type(u32::MAX - 1).succ(),
        top,
        "below the ceiling the successor is still the next concrete `Type`"
    );
    // ONE RUNG ABOVE: climbing twice stays total and strictly increasing.
    let above2 = above.succ();
    assert!(above.is_subtype_of(&above2), "the tower keeps going up");
    assert!(!above2.is_subtype_of(&above), "…and never comes back down");

    // THE KERNEL'S OWN RULE must reach the same answer, and must not abort reaching it.
    assert_eq!(
        infer_type(&ctx, &sort(top.clone())).expect("a sort at the ceiling still has a type"),
        sort(above.clone()),
        "`Sort u : Sort (succ u)` holds at the ceiling like everywhere else"
    );
    assert!(
        !is_subtype(&ctx, &sort(above.clone()), &sort(top.clone())),
        "no sort may inhabit itself, at the ceiling least of all"
    );
    // …and the SECOND kernel must agree, so the ceiling is not a place the two part company.
    assert_eq!(
        double_check(&ctx, &sort(top)),
        DoubleCheck::Agreed,
        "both kernels must type the top representable sort the same way: {:?}",
        double_check(&ctx, &sort(Universe::Type(u32::MAX)))
    );
}

// ============================================================================================
// 14 · THE KERNEL HALF OF A UNIVERSE-POLYMORPHIC EQUALITY (T06-001)
// ============================================================================================

/// `prelude.rs` pins `Eq`'s carrier to the literal `Sort(Type 0)`, which is the ceiling on the
/// entire disjointness family: `True = True` and `Nat = Nat` — both ordinary Lean, the second
/// being what `propext` concludes — are not merely unproved but UNSTATEABLE.
///
/// The prelude is not this lane's file. What IS this lane's is the machinery the repair needs,
/// and this test is the evidence that it is already in place and does not need building: an
/// equality registered exactly as `Eq` would have to be registered — a `Sort u` carrier, the
/// level AUTO-BOUND from what the author wrote, through the ordinary `add_indexed_inductive`
/// entry point, with no new registry and no new `Universe` variant — states every equation
/// T06-001 asks for, refuses the ones it must, reports a CONCRETE level every time, and is
/// verified by BOTH kernels.
///
/// So the remaining work for T06-001 is one declaration in `prelude.rs`, not a kernel feature.
/// This test is also the lock that keeps it that way: if the level machinery regresses, this
/// goes red without waiting for the prelude change.
#[test]
fn a_universe_polymorphic_equality_states_every_equation_at_every_carrier() {
    let mut ctx = std_ctx();
    // `PolyEq.{u} : Π(A : Sort u). A → A → Prop` — `Eq`'s own shape with the carrier one level
    // up. `A` and `x` are the uniform parameters (`poly_refl`'s result repeats them); the
    // trailing slot is the index.
    ctx.add_indexed_inductive(
        "PolyEq",
        pi(
            "A",
            sort(uvar("u")),
            pi("x", var("A"), pi("y", var("A"), prop())),
        ),
        2,
    );
    ctx.add_constructor(
        "poly_refl",
        "PolyEq",
        pi(
            "A",
            sort(uvar("u")),
            pi(
                "x",
                var("A"),
                apps(g("PolyEq"), &[var("A"), var("x"), var("x")]),
            ),
        ),
    );
    let eq3 = |a: Term, x: Term, y: Term| apps(g("PolyEq"), &[a, x, y]);

    // EVERY CARRIER LEAN ALLOWS, from one declaration. Each equation is a `Prop`, whatever the
    // level its carrier lives at — that is the impredicativity of `Prop` doing its job, and it
    // is what makes `propext`'s conclusion stateable.
    let carriers = [
        (
            g("Nat"),
            g("Zero"),
            "a `Type 0` carrier — the instance that already worked",
        ),
        (
            arrow(g("Nat"), g("Nat")),
            lam("n", g("Nat"), var("n")),
            "a function carrier",
        ),
        (
            prop(),
            g("True"),
            "`Prop` — `True = True`, which is what `propext` concludes",
        ),
        (
            ty(0),
            g("Nat"),
            "`Type 0` — `Nat = Nat`, an equation between two TYPES",
        ),
        (ty(1), ty(0), "`Type 1` — one rung higher again"),
    ];
    for (carrier, witness, what) in carriers {
        let stated = eq3(carrier.clone(), witness.clone(), witness.clone());
        assert_eq!(
            infer_type(&ctx, &stated).unwrap_or_else(|e| panic!("{what} must be stateable: {e:?}")),
            prop(),
            "an equation is a `Prop` at {what}"
        );
        // A level metavariable must never LEAK into a reported type: the level is SOLVED from
        // the carrier, not defaulted and not left as an obligation dressed up as an answer.
        assert!(
            !format!("{stated}").contains('?'),
            "the stated equation must carry a concrete level, got `{stated}`"
        );
        // …and it must be INHABITED by reflexivity, verified by both kernels — a statement
        // nothing can prove is not a repair.
        let proof = apps(g("poly_refl"), &[carrier.clone(), witness.clone()]);
        assert_eq!(
            infer_type(&ctx, &proof).unwrap_or_else(|e| panic!("`rfl` at {what}: {e:?}")),
            stated,
            "`poly_refl` proves the equation at {what}"
        );
        assert_eq!(
            double_check(&ctx, &proof),
            DoubleCheck::Agreed,
            "…and BOTH kernels must verify it at {what}: {:?}",
            double_check(&ctx, &proof)
        );
    }

    // THE NEGATIVES, which are the half that makes the positives mean something.
    //
    // A HETEROGENEOUS equation — two sides of unrelated types — must stay unstateable. Widening
    // the carrier must widen WHICH types can be equated, never relate two different ones.
    assert!(
        infer_type(&ctx, &eq3(g("Nat"), g("Zero"), g("true"))).is_err(),
        "`Zero = true` relates a `Nat` to a `Bool` and must be refused"
    );
    // …and it must not be reachable by passing a hole for the carrier either: a matcher's
    // wildcard must never become the kernel's answer.
    assert!(
        infer_type(&ctx, &eq3(Term::Hole, g("Zero"), g("true"))).is_err(),
        "an equation whose carrier is a hole must be refused, not inferred as a `Prop`"
    );
    // A carrier that is not a TYPE at all has no level to solve, and the level must never be
    // invented to make the application go through.
    assert!(
        infer_type(&ctx, &eq3(g("Zero"), g("Zero"), g("Zero"))).is_err(),
        "`Zero` is a value, not a carrier — there is no level here and none may be guessed"
    );
    // `refl` must still be pinned to ONE inhabitant: reflexivity proves `x = x`, never `x = y`.
    assert!(
        infer_type(
            &ctx,
            &app(
                lam(
                    "h",
                    eq3(g("Nat"), g("Zero"), app(g("Succ"), g("Zero"))),
                    var("h")
                ),
                apps(g("poly_refl"), &[g("Nat"), g("Zero")]),
            )
        )
        .is_err(),
        "`poly_refl` must not prove `0 = 1` at any carrier"
    );
}

// ============================================================================================
// 15 · THE `imax` CASE SPLIT MUST NOT CAPTURE
// ============================================================================================

/// Deciding `a ≤ b` when an `imax`'s right argument is a variable `v` needs a case split: `v` is
/// `Prop` (level 0), or `v ≥ 1`. The second branch is written `v := succ(v′)`, and it means what
/// it says ONLY while `v′` is a variable nothing else constrains. A `v′` that already occurs in
/// either side ties the case-split witness to a real variable, so the branch checks the DIAGONAL
/// instead of every independent pair — and the decision procedure then reports a level ordering
/// that does not hold, at exactly the place cumulativity is read off.
///
/// `imax(Prop, u)` IS `u` (`imax(0, 0) = 0`, and `max(0, u) = u` otherwise), so
/// `imax(Prop, u) ≤ w + 2` asks whether `u ≤ w + 2` for every INDEPENDENT `u` and `w`. That is
/// false at `u = 10, w = 0`, and it must be false however the second variable is SPELLED.
#[test]
fn the_imax_case_split_never_captures_a_variable_the_levels_already_mention() {
    let ctx = std_ctx();
    let imax_prop_u = Universe::IMax(Box::new(Universe::Prop), Box::new(uvar("u")));
    let plus_two = |v: &str| Universe::Succ(Box::new(Universe::Succ(Box::new(uvar(v)))));

    // THE CONTROL first, so the refusals below cannot be read as "it refuses everything":
    // `imax(Prop, u)` and `u` are the SAME level, and the split must still say so.
    assert!(imax_prop_u.is_subtype_of(&uvar("u")), "`imax(Prop, u) ≤ u`");
    assert!(
        uvar("u").is_subtype_of(&imax_prop_u),
        "…and back, so the two are equivalent"
    );
    assert!(imax_prop_u.equiv(&uvar("u")), "`imax(Prop, u) ≡ u`");

    // THE ORDERING ITSELF, under every spelling of the second variable — including the one a
    // split on `u` would otherwise invent for itself.
    for w in ["w", "u✦", "u✦✦", "✦"] {
        assert!(
            !imax_prop_u.is_subtype_of(&plus_two(w)),
            "`imax(Prop, u) ≤ {w} + 2` is false at `u = 10, {w} = 0` and must be refused; a \
             case split that spells its witness `u✦` CAPTURES a variable spelled `u✦` and then \
             only ever checks `u = {w} + 1`"
        );
        assert!(
            !is_subtype(&ctx, &sort(imax_prop_u.clone()), &sort(plus_two(w))),
            "…and the kernel's own conversion door must give the same answer for `{w}`"
        );
    }

    // THE POSITIVE the split exists for, so freshening did not disable the rule: `imax` is
    // bounded by `max` in both branches, and is NOT bounded by the domain alone.
    let imax_t3_u = Universe::IMax(Box::new(Universe::Type(3)), Box::new(uvar("u")));
    assert!(
        imax_t3_u.is_subtype_of(&Universe::Type(3).max(&uvar("u"))),
        "`imax(Type 3, u) ≤ max(Type 3, u)` holds in the `u = Prop` branch and in the `u ≥ 1` one"
    );
    assert!(
        !imax_t3_u.is_subtype_of(&Universe::Type(3)),
        "…and it is NOT bounded by `Type 3`, because `u` may exceed it"
    );
}

/// THE GENERAL LOCK behind the case above, and the one that would have caught it without anyone
/// guessing the shape: a differential of the kernel's level ORDERING against the ℕ-model the
/// ordering is specified over, written here from the specification rather than read out of
/// `term.rs`.
///
/// The specification (`term.rs`, `LNat`): `Prop = 0`, `Type n = n+1`, `succ` is `+1`, `max` is
/// the larger, `imax(a, b)` is `0` when `b` is `0` and `max(a, b)` otherwise, and every universe
/// VARIABLE ranges over all of ℕ — which is what makes `Type 0 ≤ u` false and closes the
/// `Sort u := Nat` unsoundness.
///
/// **Why a differential and not more examples.** The level algebra is the one part of the
/// kernel that `double_check` structurally cannot police: `Universe::succ`, `max`, `imax` and
/// `is_subtype_of` are ONE implementation in `term.rs` and BOTH kernels call it, so a wrong
/// level ordering is miscounted identically on both sides and `double_check` returns `Agreed`
/// throughout. The only thing that can catch it is a test that compares the algebra to its own
/// semantics.
///
/// The variable named `u✦` is in the grid ON PURPOSE: that is the name a case split on `u`
/// invents for itself, so the grid contains the collision rather than hoping someone thinks of
/// it.
#[test]
fn the_level_ordering_never_accepts_an_ordering_the_number_model_refutes() {
    /// The ℕ-model, evaluated under an assignment — the specification, restated.
    fn value(u: &Universe, assign: &dyn Fn(&str) -> u64) -> u64 {
        match u {
            Universe::SProp | Universe::Prop => 0,
            Universe::Type(n) => *n as u64 + 1,
            Universe::Var(v) => assign(v),
            Universe::Succ(l) => value(l, assign) + 1,
            Universe::Max(a, b) => value(a, assign).max(value(b, assign)),
            Universe::IMax(a, b) => {
                let right = value(b, assign);
                if right == 0 {
                    0
                } else {
                    value(a, assign).max(right)
                }
            }
        }
    }

    let atoms = [
        Universe::Prop,
        Universe::Type(0),
        Universe::Type(1),
        uvar("u"),
        uvar("u✦"),
    ];
    let mut grid: Vec<Universe> = atoms.to_vec();
    for a in &atoms {
        grid.push(Universe::Succ(Box::new(a.clone())));
        grid.push(Universe::Succ(Box::new(Universe::Succ(Box::new(
            a.clone(),
        )))));
        for b in &atoms {
            grid.push(Universe::Max(Box::new(a.clone()), Box::new(b.clone())));
            grid.push(Universe::IMax(Box::new(a.clone()), Box::new(b.clone())));
        }
    }
    let assignments: Vec<(u64, u64)> = (0..=5u64)
        .flat_map(|x| (0..=5u64).map(move |y| (x, y)))
        .collect();

    // ── SOUNDNESS: the kernel must never ACCEPT an ordering the model refutes. This is the
    // direction that lets a term through, so it is asserted over every pair.
    let mut unsound: Vec<String> = Vec::new();
    for a in &grid {
        for b in &grid {
            if !a.is_subtype_of(b) {
                continue;
            }
            for (x, y) in &assignments {
                let assign = |name: &str| if name == "u" { *x } else { *y };
                if value(a, &assign) > value(b, &assign) {
                    unsound.push(format!(
                        "{a} ≤ {b} accepted, but at u={x}, u✦={y} it is false"
                    ));
                    break;
                }
            }
        }
    }
    assert!(
        unsound.is_empty(),
        "{} level orderings accepted that the ℕ-model refutes (the first few): {:?}",
        unsound.len(),
        &unsound[..unsound.len().min(6)]
    );

    // ── THE LAWS, from below, so a procedure that simply refused everything could not pass the
    // clause above. Each is true in the model for EVERY level, and needs no oracle.
    for a in &grid {
        assert!(a.is_subtype_of(a), "reflexivity: `{a} ≤ {a}`");
        let up = Universe::Succ(Box::new(a.clone()));
        assert!(a.is_subtype_of(&up), "`{a} ≤ succ {a}`");
        assert!(!up.is_subtype_of(a), "…and strictly: `succ {a} ≰ {a}`");
        assert!(
            Universe::Prop.is_subtype_of(a),
            "`Prop` is the bottom: `Prop ≤ {a}`"
        );
        for b in &grid {
            let joined = Universe::Max(Box::new(a.clone()), Box::new(b.clone()));
            assert!(a.is_subtype_of(&joined), "`{a} ≤ max({a}, {b})`");
            assert!(b.is_subtype_of(&joined), "`{b} ≤ max({a}, {b})`");
            let imaxed = Universe::IMax(Box::new(a.clone()), Box::new(b.clone()));
            assert!(
                imaxed.is_subtype_of(&joined),
                "`imax({a}, {b}) ≤ max({a}, {b})`"
            );
            // `succ` is MONOTONE and reflects the order, which pins the procedure in both
            // directions at once.
            assert_eq!(
                Universe::Succ(Box::new(a.clone()))
                    .is_subtype_of(&Universe::Succ(Box::new(b.clone()))),
                a.is_subtype_of(b),
                "`succ {a} ≤ succ {b}` must agree with `{a} ≤ {b}`"
            );
        }
    }
}

// ============================================================================================
// 16 · THE `Nat`-ORDER ACCELERATION MUST FIRE ON EXACTLY THE SAME OPERANDS IN BOTH KERNELS
// ============================================================================================
//
// An acceleration is a rule that answers a question the definition it stands in for would answer
// more slowly. That makes it sound only while the two kernels accelerate over the SAME set: one
// that reaches further decides what the other abandons, and `double_check` reports that as a
// soundness alarm about a perfectly valid proof; one that clamps differently answers a different
// VALUE, and then one of the two kernels is simply wrong.
//
// Section 10 above compares the shortcut's ANSWER against the structural recursion on the small
// grid. These three compare its DOMAIN, at the three places the two engines can part company:
// how far each reaches, which definitions each stands in for, and how each reads an operand that
// is not a natural number at all.

/// A `Nat` literal at arbitrary precision — including the malformed negative one the kernel can
/// be handed, which is not a natural and which both Peano bridges collapse to `Zero`.
fn natbig(decimal: &str) -> Term {
    Term::Lit(Literal::Nat(
        logicaffeine_base::BigInt::parse_decimal(decimal).expect("decimal Nat literal"),
    ))
}

/// `(λ(h : Eq Bool <redex> <value>). Zero) (refl Bool <value>)` — well-typed exactly when the
/// kernel doing the checking reduces `redex` to the `Bool` constructor `value`.
fn cites_bool(redex: &Term, value: &str) -> Term {
    app(
        lam("h", eq_at(g("Bool"), redex.clone(), g(value)), g("Zero")),
        refl(g("Bool"), g(value)),
    )
}

/// The prelude's `Nat`, `Bool` and `Eq.{u}`, spelled as the prelude spells them, and nothing built
/// on the order names. The prelude's `natLe`/`natLt` have the integer order's laws resting on
/// them, and a declared name never changes meaning under what depends on it
/// (`Context::admit_meaning`; Lean's environment is append-only), so a test that REBINDS the
/// accelerated name does it here, where nothing depends on the name — which is exactly the
/// situation the acceleration's own gate has to handle.
fn order_free_ctx() -> Context {
    let mut ctx = Context::new();
    ctx.add_inductive("Nat", ty(0));
    ctx.add_constructor("Zero", "Nat", g("Nat"));
    ctx.add_constructor("Succ", "Nat", arrow(g("Nat"), g("Nat")));
    ctx.add_inductive("Bool", ty(0));
    ctx.add_constructor("true", "Bool", g("Bool"));
    ctx.add_constructor("false", "Bool", g("Bool"));
    let carrier = || sort(uvar("u"));
    ctx.add_indexed_inductive(
        "Eq",
        pi("A", carrier(), pi("x", var("A"), pi("y", var("A"), prop()))),
        2,
    );
    ctx.add_constructor(
        "refl",
        "Eq",
        pi(
            "A",
            carrier(),
            pi("x", var("A"), eq_at(var("A"), var("x"), var("x"))),
        ),
    );
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:?}",
        ctx.refused_declarations()
    );
    ctx
}

/// The prelude's `natLe` keeps its program: rebinding it where theorems rest on it is refused and
/// recorded, and `natLe 7 3` still decides `false`.
fn assert_prelude_nat_order_keeps_its_program(rebound_body: Term) {
    let mut ctx = std_ctx();
    ctx.add_definition(
        "natLe".to_string(),
        arrow(g("Nat"), arrow(g("Nat"), g("Bool"))),
        rebound_body,
    );
    assert!(
        ctx.refused_declarations().iter().any(|(n, _)| n == "natLe"),
        "rebinding the prelude's `natLe` must be refused: {:?}",
        ctx.refused_declarations()
    );
    assert_eq!(
        normalize(&ctx, &apps(g("natLe"), &[natl(7), natl(3)])),
        g("false")
    );
}

/// The `Bool` constructor a term reduced to, or `None` if the kernel did not decide it.
fn decided_bool(t: &Term) -> Option<&'static str> {
    match t {
        Term::Global(n) if n == "true" => Some("true"),
        Term::Global(n) if n == "false" => Some("false"),
        _ => None,
    }
}

/// HOW FAR EACH REACHES. Wherever the main kernel decides a `Nat` order, the second kernel must
/// decide it too and reach the same answer; wherever the main kernel abandons it, the second
/// must abandon it as well. Neither half is optional: a shortcut that stops short refuses valid
/// proofs, and one that reaches past manufactures alarms about them.
#[test]
fn both_kernels_decide_a_nat_order_over_exactly_the_same_operands() {
    let ctx = std_ctx();
    // Small, either side of an acceleration boundary, and far past anything a structural
    // recursion could walk.
    let operands = [
        (natl(0), natl(0)),
        (natl(3), natl(7)),
        (natl(7), natl(3)),
        (natbig("262144"), natbig("262144")),
        (natbig("262145"), natbig("262144")),
        (natbig("262144"), natbig("262145")),
        (
            natbig("18446744073709551616"),
            natbig("18446744073709551616"),
        ),
        (
            natbig("18446744073709551616"),
            natbig("18446744073709551617"),
        ),
        (natl(5), natbig("18446744073709551616")),
    ];
    for (m, n) in operands {
        for op in ["natLe", "natLt"] {
            let redex = apps(g(op), &[m.clone(), n.clone()]);
            match decided_bool(&normalize(&ctx, &redex)) {
                Some(answer) => {
                    assert_eq!(
                        double_check(&ctx, &cites_bool(&redex, answer)),
                        DoubleCheck::Agreed,
                        "the main kernel decided `{op} {m} {n} = {answer}`, so the second kernel \
                         must decide it too — a shortcut that stops short of the first kernel's \
                         reach makes `double_check` refuse a valid proof: {:?}",
                        double_check(&ctx, &cites_bool(&redex, answer))
                    );
                    let other = if answer == "true" { "false" } else { "true" };
                    assert!(
                        recheck(&ctx, &cites_bool(&redex, other)).is_err(),
                        "…and the second kernel must REFUSE the opposite answer for `{op} {m} {n}`"
                    );
                }
                None => {
                    for guess in ["true", "false"] {
                        assert!(
                            !matches!(
                                double_check(&ctx, &cites_bool(&redex, guess)),
                                DoubleCheck::Disagree(_)
                            ),
                            "the main kernel abandoned `{op} {m} {n}`, so the second kernel must \
                             not decide it either — reaching past the first kernel is what turns \
                             an acceleration into a widening"
                        );
                    }
                }
            }
        }
    }
}

/// WHICH DEFINITION EACH STANDS IN FOR. The shortcut is an override keyed on a NAME, so it is
/// sound only while it fires exactly when the name is bound to the program it accelerates. A
/// gate that checks only the name's TYPE fires on any `Nat → Nat → Bool`, and then a context
/// that rebinds `natLe` gets one kernel's arithmetic and the other kernel's body — two different
/// VALUES for one closed term, which is the worst outcome a second kernel can produce.
#[test]
fn the_nat_order_shortcut_stands_in_only_for_the_program_it_is_named_after() {
    let const_true = || lam("m", g("Nat"), lam("n", g("Nat"), g("true")));
    assert_prelude_nat_order_keeps_its_program(const_true());
    let mut ctx = order_free_ctx();
    // A DIFFERENT `Nat → Nat → Bool` under the accelerated name: constantly `true`, so which
    // rule won is observable rather than a matter of taste.
    ctx.add_definition(
        "natLe".to_string(),
        arrow(g("Nat"), arrow(g("Nat"), g("Bool"))),
        const_true(),
    );
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:?}",
        ctx.refused_declarations()
    );
    let redex = apps(g("natLe"), &[natl(7), natl(3)]);
    assert_eq!(
        normalize(&ctx, &redex),
        g("true"),
        "the rebound BODY decides, not the arithmetic the name used to stand for"
    );
    assert_eq!(
        double_check(&ctx, &cites_bool(&redex, "true")),
        DoubleCheck::Agreed,
        "…and both kernels must agree about which rule won, or one of them is computing a \
         program the other never checked: {:?}",
        double_check(&ctx, &cites_bool(&redex, "true"))
    );
    assert!(
        recheck(&ctx, &cites_bool(&redex, "false")).is_err(),
        "the accelerated answer must be refused once the name is rebound"
    );

    // THE CONTROL: with the prelude's own `natLe` the acceleration is still on, in both kernels.
    let pristine = std_ctx();
    let same = apps(g("natLe"), &[natl(7), natl(3)]);
    assert_eq!(
        normalize(&pristine, &same),
        g("false"),
        "the real `natLe` still decides `7 ≤ 3`"
    );
    assert_eq!(
        double_check(&pristine, &cites_bool(&same, "false")),
        DoubleCheck::Agreed,
        "…in both kernels: {:?}",
        double_check(&pristine, &cites_bool(&same, "false"))
    );
}

/// HOW EACH READS AN OPERAND THAT IS NOT A NATURAL. A `Literal::Nat` carrying a NEGATIVE value is
/// not a natural number, but the kernel can be handed one, and both Peano bridges collapse it to
/// `Zero` so the peel terminates rather than descending toward −∞. An arithmetic shortcut that
/// reads the raw sign instead answers a different question from the recursion it stands in for —
/// and `natLt (-1) 0` is `false` under the bridge and `true` under the raw comparison, so the two
/// kernels return OPPOSITE `Bool` constructors for one closed term.
#[test]
fn a_negative_nat_literal_is_read_as_zero_by_the_shortcut_and_by_the_recursion_alike() {
    let ctx = std_ctx();
    let neg = |d: &str| natbig(d);
    for (m, n, op) in [
        (neg("-1"), natl(0), "natLt"),
        (natl(0), neg("-1"), "natLe"),
        (neg("-1"), neg("-5"), "natLe"),
        (neg("-5"), neg("-1"), "natLt"),
        (neg("-1"), natl(1), "natLt"),
    ] {
        let redex = apps(g(op), &[m.clone(), n.clone()]);
        let main = normalize(&ctx, &redex);
        let answer = decided_bool(&main)
            .unwrap_or_else(|| panic!("`{op} {m} {n}` must still decide, got {main}"));
        assert_eq!(
            double_check(&ctx, &cites_bool(&redex, answer)),
            DoubleCheck::Agreed,
            "both kernels must read the malformed literal the SAME way for `{op} {m} {n}`; the \
             bridge says it is `Zero`, so the shortcut must say so too: {:?}",
            double_check(&ctx, &cites_bool(&redex, answer))
        );
    }

    // THE BOUNDARY, one rung up: a `Succ` applied to a malformed literal is `Succ Zero`, i.e.
    // ONE, under the bridge — so the arithmetic route must not read it as zero-minus-one-plus-one.
    let succ_of_neg = app(g("Succ"), neg("-1"));
    let redex = apps(g("natLt"), &[natl(0), succ_of_neg]);
    let main = normalize(&ctx, &redex);
    let answer = decided_bool(&main).unwrap_or_else(|| panic!("must decide, got {main}"));
    assert_eq!(answer, "true", "`0 < Succ (-1 as Zero)` is `0 < 1`");
    assert_eq!(
        double_check(&ctx, &cites_bool(&redex, answer)),
        DoubleCheck::Agreed,
        "…and both kernels must agree one rung above the clamp too: {:?}",
        double_check(&ctx, &cites_bool(&redex, answer))
    );
}

// ============================================================================================
// 17 · A LITERAL'S EQUALITY IS THE KERNEL'S EQUALITY, SO IT MUST BE AN EQUIVALENCE
// ============================================================================================
//
// `type_checker::types_equal` compares two `Term::Lit` with `==`, and `recheck::def_eq` does the
// same, so ONE `PartialEq` decides literal conversion in BOTH kernels. That is the universe
// algebra's blind spot one field over: the two-kernel guarantee gives ZERO protection here, so
// the property has to be pinned against an INDEPENDENT model instead — IEEE-754's own bit
// encoding, which `f64::to_bits` reports and which is total, reflexive and injective.
//
// IEEE `==` is not an equivalence relation. It is not reflexive at `NaN`, and it identifies the
// two signed zeros, which have distinct encodings. A kernel whose conversion is not reflexive
// cannot prove `x = x` by `refl`; one that identifies two distinct values proves an equation
// Lean refuses. `impl Eq for Literal {}` (term.rs) is a written claim that neither happens.

/// A `Float` literal at an exact bit pattern — the test names the ENCODING, never the printed
/// decimal, because `-0.0` and `0.0` print differently on some paths and identically on others.
fn floatbits(bits: u64) -> Term {
    Term::Lit(Literal::Float(f64::from_bits(bits)))
}

/// Every bit pattern the kernel can be handed, including the two the derived `PartialEq` gets
/// wrong. `NaN` appears twice at DIFFERENT payloads on purpose: they are different encodings and
/// therefore different literals, and a `to_bits` comparison is what makes that statement total.
fn float_bit_patterns() -> Vec<(&'static str, u64)> {
    vec![
        ("0.0", 0.0f64.to_bits()),
        ("-0.0", (-0.0f64).to_bits()),
        ("1.5", 1.5f64.to_bits()),
        ("-1.5", (-1.5f64).to_bits()),
        ("+inf", f64::INFINITY.to_bits()),
        ("-inf", f64::NEG_INFINITY.to_bits()),
        ("NaN", f64::NAN.to_bits()),
        ("NaN(payload 1)", f64::NAN.to_bits() | 1),
        ("min_positive", f64::MIN_POSITIVE.to_bits()),
        ("subnormal", 1u64),
    ]
}

/// REFLEXIVITY, the property `refl` IS. Every float literal must be convertible with itself in
/// both kernels — `NaN` included, where IEEE `==` says a value differs from itself and the
/// kernel consequently cannot retype `refl Float x : Eq Float x x`.
#[test]
fn a_float_literal_is_convertible_with_itself_at_every_bit_pattern_in_both_kernels() {
    let ctx = std_ctx();
    for (name, bits) in float_bit_patterns() {
        let x = floatbits(bits);
        assert!(
            defeq_for_test(&ctx, &x, &x),
            "`{name}` must be definitionally equal to itself — conversion that is not \
             REFLEXIVE is not an equivalence relation, and `impl Eq for Literal` claims it is"
        );
        let proof = refl(g("Float"), x.clone());
        let stated = eq_at(g("Float"), x.clone(), x.clone());
        let cited = app(lam("h", stated, var("h")), proof);
        assert!(
            infer_type(&ctx, &cited).is_ok(),
            "…so `refl Float {name}` must prove `{name} = {name}` in the main kernel"
        );
        assert_eq!(
            double_check(&ctx, &cited),
            DoubleCheck::Agreed,
            "…and in the second kernel, which reads the SAME `PartialEq`: {:?}",
            double_check(&ctx, &cited)
        );
    }
}

/// THE NEGATIVE, and the boundary one bit either side: two DIFFERENT encodings must stay
/// different terms in both kernels. `0.0` and `-0.0` are the pair IEEE `==` identifies, and
/// Lean does not prove `(0.0 : Float) = -0.0` by `rfl`; the two `NaN` payloads are the pair a
/// "canonicalise every NaN" repair would wrongly merge.
#[test]
fn distinct_float_encodings_are_distinct_terms_in_both_kernels() {
    let ctx = std_ctx();
    let patterns = float_bit_patterns();
    for (na, ba) in &patterns {
        for (nb, bb) in &patterns {
            if ba == bb {
                continue;
            }
            let a = floatbits(*ba);
            let b = floatbits(*bb);
            assert!(
                !defeq_for_test(&ctx, &a, &b),
                "`{na}` and `{nb}` have different IEEE encodings and must not be identified"
            );
            // …and the kernel must REFUSE the proof that says otherwise, in both directions.
            let lie = app(
                lam("h", eq_at(g("Float"), a.clone(), b.clone()), var("h")),
                refl(g("Float"), a.clone()),
            );
            assert!(
                infer_type(&ctx, &lie).is_err(),
                "`refl Float {na}` must not prove `{na} = {nb}`"
            );
            assert!(
                recheck(&ctx, &lie).is_err(),
                "…and the second kernel must refuse it too"
            );
        }
    }
}

/// THE CONTRACT ITSELF. `Eq` is a mathematical claim about `PartialEq`, and the kernel relies on
/// it everywhere it compares terms. Stated over the literal grid the kernel actually handles, so
/// a repair aimed only at `Float` cannot break `Int`/`Nat`/`Text` on the way past.
#[test]
fn literal_equality_is_an_equivalence_relation_over_every_literal_kind() {
    let grid: Vec<Literal> = float_bit_patterns()
        .into_iter()
        .map(|(_, b)| Literal::Float(f64::from_bits(b)))
        .chain([
            Literal::Int(0),
            Literal::Int(-1),
            Literal::Int(i64::MAX),
            Literal::Text(String::new()),
            Literal::Text("a".to_string()),
            Literal::Duration(0),
            Literal::Date(0),
            Literal::Moment(-1),
            Literal::BigInt(
                logicaffeine_base::BigInt::parse_decimal("1180591620717411303424").unwrap(),
            ),
            Literal::Nat(logicaffeine_base::BigInt::from_i64(7)),
        ])
        .collect();

    for a in &grid {
        assert!(a == a, "reflexivity fails at {a:?}");
    }
    for a in &grid {
        for b in &grid {
            assert_eq!(a == b, b == a, "symmetry fails at {a:?} / {b:?}");
            for c in &grid {
                if a == b && b == c {
                    assert!(a == c, "transitivity fails at {a:?} / {b:?} / {c:?}");
                }
            }
        }
    }

    // THE INDEPENDENT MODEL: two float literals are equal exactly when their IEEE encodings
    // are. Stated against `to_bits` rather than against the implementation, so a change that
    // re-derives the rule from `==` again cannot satisfy it.
    for (_, ba) in float_bit_patterns() {
        for (_, bb) in float_bit_patterns() {
            assert_eq!(
                Literal::Float(f64::from_bits(ba)) == Literal::Float(f64::from_bits(bb)),
                ba == bb,
                "float literal equality must be encoding equality, not IEEE `==`"
            );
        }
    }
}

// ============================================================================================
// 18 · THE DEPTH DOOR MUST MEAN WHAT IT SAYS, IN BOTH KERNELS
// ============================================================================================
//
// `enter_kernel` advertises a limit — `MAX_TERM_DEPTH` — and REFUSES anything past it, on the
// stated ground that descending further "would exhaust the native stack". A door is only a door
// while the band it admits is a band the kernel survives: if the walk dies at a fraction of the
// advertised depth, the refusal is not a judgement and the process ABORTS instead, which is the
// one outcome a kernel may never produce.
//
// The second kernel has the same obligation and, until this wave, the OPPOSITE discipline: an
// absolute byte budget unrelated to the stack the call actually has. Two kernels that run out of
// room at different places do not corroborate each other across the gap — `double_check` reports
// `MainOnlyReCheckerIncomplete` on a perfectly ordinary telescope, which is single-checking
// wearing a two-kernel badge. The band must be ONE band.

/// `Π(x₀:Nat). … Π(x_{n-1}:Nat). Nat` — the plain telescope, `n` rungs deep.
fn pi_telescope(n: usize) -> Term {
    (0..n).fold(g("Nat"), |acc, i| pi(&format!("x{i}"), g("Nat"), acc))
}

/// `λ(n:Nat). λ(p₀ : n = n). … λ(p_{k-1} : n = n). n` — the DEPENDENT telescope the corpus's own
/// depth case uses, where every later binder's type mentions the first.
fn dependent_telescope(k: usize) -> Term {
    let body = (0..k).rev().fold(var("n"), |acc, i| {
        lam(&format!("p{i}"), eq_at(g("Nat"), var("n"), var("n")), acc)
    });
    lam("n", g("Nat"), body)
}

/// THE DOOR'S OWN CLAIM. Everything the door ADMITS, the main kernel must actually check — it
/// may answer, it may refuse, it may not die. Measured on both shapes, over a grid that spans
/// the band where the bare 2 MiB stack used to abort (~52 rungs of the dependent shape, ~106 of
/// the plain one) and well past it.
#[test]
fn every_depth_the_door_admits_is_a_depth_the_main_kernel_survives() {
    let ctx = std_ctx();
    for rungs in [8usize, 32, 64, 128, 256, 400] {
        let plain = pi_telescope(rungs);
        assert!(
            infer_type(&ctx, &plain).is_ok(),
            "a {rungs}-rung Π telescope is far inside the depth the door advertises, so the \
             kernel must reach a verdict about it rather than exhaust the native stack"
        );
        let dependent = dependent_telescope(rungs);
        assert!(
            infer_type(&ctx, &dependent).is_ok(),
            "…and so is a {rungs}-rung DEPENDENT telescope, whose every binder mentions the first"
        );
    }
}

/// ONE BAND, TWO KERNELS. Wherever the main kernel reaches a verdict, the second must reach one
/// too — `Agreed`, never `MainOnlyReCheckerIncomplete`, which is what a stack budget unrelated
/// to the actual stack produces on an ordinary telescope.
#[test]
fn the_second_kernel_runs_out_of_room_at_the_same_place_the_first_one_does() {
    let ctx = std_ctx();
    for rungs in [8usize, 32, 64, 128, 256] {
        for (shape, term) in [
            ("Π telescope", pi_telescope(rungs)),
            ("dependent λ telescope", dependent_telescope(rungs)),
        ] {
            let verdict = double_check(&ctx, &term);
            assert_eq!(
                verdict,
                DoubleCheck::Agreed,
                "both kernels must reach a verdict about a {rungs}-rung {shape}; a second \
                 kernel that gives up sooner than the first is single-checking with extra \
                 steps: {verdict:?}"
            );
        }
    }
}

/// THE NEGATIVE, one rung past the door: a term DEEPER than the advertised limit must be
/// REFUSED — reported, not answered and not aborted — and the refusal must name the limit, so
/// the caller can tell "I will not look" from "I looked and it is ill-typed".
#[test]
fn a_term_past_the_advertised_limit_is_refused_by_both_kernels_and_never_answered() {
    let ctx = std_ctx();
    // 1600 is the door; 1700 is past it on any reading, and the refusal must not depend on
    // WHICH side of the door the walk would have died on.
    let too_deep = pi_telescope(1700);
    match infer_type(&ctx, &too_deep) {
        Err(KernelError::CertificationError(why)) => assert!(
            why.contains("1600") && why.contains("REFUSAL"),
            "the refusal must say it is a refusal and name the limit, got: {why}"
        ),
        other => panic!("a term past the door must be REFUSED, got {other:?}"),
    }
    match recheck(&ctx, &too_deep) {
        Err(ReCheckError::Unsupported(why)) => assert!(
            why.contains("1600"),
            "the second kernel must refuse at the SAME door and say so, got: {why}"
        ),
        other => panic!("the second kernel must refuse past the door, got {other:?}"),
    }
    assert!(
        matches!(
            double_check(&ctx, &too_deep),
            DoubleCheck::MainOnlyReCheckerIncomplete(_)
        ),
        "a refusal by both is not agreement about a TYPE — neither kernel looked"
    );
}

// ============================================================================================
// 19 · A REDUCTION RULE ONLY ONE KERNEL HAS IS AN ALARM THAT FIRES ON VALID PROOFS
// ============================================================================================
//
// The prelude DECLARES the reflection and tactic primitives, so they are inside the trusted base
// and a proof may cite what they compute. The main kernel computes them; the second kernel, for
// most of them, walks past and treats the redex as a value. That is not a safe shortfall —
// `double_check` reports `Disagree`, a SOUNDNESS ALARM, about a proof that is perfectly valid.
//
// There are exactly two honest answers for a primitive the second kernel is handed:
//   * compute it INDEPENDENTLY, from the specification, and corroborate; or
//   * say it did not check it, so the verdict is `MainOnlyReCheckerIncomplete` — single-checked,
//     and labelled as such.
// "Quietly treat it as stuck and then disagree" is neither.

fn sname(s: &str) -> Term {
    app(g("SName"), Term::Lit(Literal::Text(s.to_string())))
}
fn svar(k: i64) -> Term {
    app(g("SVar"), Term::Lit(Literal::Int(k)))
}
fn slit(k: i64) -> Term {
    app(g("SLit"), Term::Lit(Literal::Int(k)))
}
fn sapp(f: Term, x: Term) -> Term {
    apps(g("SApp"), &[f, x])
}
fn slam(t: Term, b: Term) -> Term {
    apps(g("SLam"), &[t, b])
}
fn spi(t: Term, b: Term) -> Term {
    apps(g("SPi"), &[t, b])
}
fn intlit(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}

/// The reflection algebra: one row per rule, each a closed redex over the `Syntax` grammar whose
/// value is fixed by the specification alone, so the row states the ANSWER rather than quoting
/// an implementation.
fn reflection_rows() -> Vec<(&'static str, Term, Term)> {
    let id = slam(sname("Nat"), svar(0));
    vec![
        ("syn_size/leaf", app(g("syn_size"), svar(0)), intlit(1)),
        (
            "syn_size/app",
            app(g("syn_size"), sapp(svar(0), svar(1))),
            intlit(3),
        ),
        ("syn_size/lam", app(g("syn_size"), id.clone()), intlit(3)),
        (
            "syn_size/nested",
            app(g("syn_size"), sapp(id.clone(), sapp(svar(2), slit(7)))),
            intlit(7),
        ),
        (
            "syn_max_var/free",
            app(g("syn_max_var"), svar(3)),
            intlit(3),
        ),
        (
            "syn_max_var/bound",
            app(g("syn_max_var"), id.clone()),
            intlit(-1),
        ),
        (
            "syn_max_var/name",
            app(g("syn_max_var"), sname("Nat")),
            intlit(-1),
        ),
        (
            "syn_max_var/under-binder",
            app(g("syn_max_var"), slam(sname("Nat"), svar(2))),
            intlit(1),
        ),
        (
            "syn_lift/free",
            apps(g("syn_lift"), &[intlit(2), intlit(0), svar(0)]),
            svar(2),
        ),
        (
            "syn_lift/below-cutoff",
            apps(g("syn_lift"), &[intlit(2), intlit(1), svar(0)]),
            svar(0),
        ),
        (
            "syn_lift/under-binder",
            apps(
                g("syn_lift"),
                &[intlit(1), intlit(0), slam(sname("Nat"), svar(1))],
            ),
            slam(sname("Nat"), svar(2)),
        ),
        (
            "syn_subst/hit",
            apps(g("syn_subst"), &[slit(9), intlit(0), svar(0)]),
            slit(9),
        ),
        (
            "syn_subst/miss",
            apps(g("syn_subst"), &[slit(9), intlit(1), svar(0)]),
            svar(0),
        ),
        (
            "syn_beta",
            apps(g("syn_beta"), &[sapp(svar(0), sname("f")), slit(4)]),
            sapp(slit(4), sname("f")),
        ),
        (
            "syn_step/beta",
            app(g("syn_step"), sapp(id.clone(), slit(5))),
            slit(5),
        ),
        (
            "syn_step/stuck",
            app(g("syn_step"), sapp(sname("f"), slit(5))),
            sapp(sname("f"), slit(5)),
        ),
        (
            "syn_eval/arith",
            apps(
                g("syn_eval"),
                &[intlit(10), sapp(sapp(sname("add"), slit(2)), slit(3))],
            ),
            slit(5),
        ),
        (
            "syn_eval/beta-then-arith",
            apps(
                g("syn_eval"),
                &[
                    intlit(20),
                    sapp(
                        slam(sname("Int"), sapp(sapp(sname("mul"), svar(0)), slit(3))),
                        slit(4),
                    ),
                ],
            ),
            slit(12),
        ),
        (
            "syn_size/pi",
            app(g("syn_size"), spi(sname("Nat"), sname("Nat"))),
            intlit(3),
        ),
    ]
}

/// The carrier each reflection rule lands in — `Int` for the measurements, `Syntax` for the
/// transformations.
fn reflection_carrier(row: &str) -> Term {
    if row.starts_with("syn_size") || row.starts_with("syn_max_var") {
        g("Int")
    } else {
        g("Syntax")
    }
}

/// THE CLASS. Every reflection rule the main kernel computes, the second kernel computes too —
/// the same answer, reached over its own de Bruijn representation.
#[test]
fn every_reflection_rule_the_main_kernel_computes_the_second_kernel_computes_too() {
    let ctx = std_ctx();
    for (name, redex, value) in reflection_rows() {
        assert_eq!(
            normalize(&ctx, &redex),
            value,
            "the main kernel computes `{name}`"
        );
        let carrier = reflection_carrier(name);
        let cited = cites(carrier.clone(), redex.clone(), value.clone());
        assert!(
            infer_type(&ctx, &cited).is_ok(),
            "…so a proof citing `{name}` type-checks in the main kernel"
        );
        assert_eq!(
            double_check(&ctx, &cited),
            DoubleCheck::Agreed,
            "…and the SECOND kernel must compute `{name}` too: {:?}",
            double_check(&ctx, &cited)
        );
        // THE NEGATIVE: the rule must DECIDE, not merely accept. A re-checker that answered
        // "whatever you say" would pass the clause above.
        let lie = cites(carrier, redex, slit(-424242));
        assert!(
            recheck(&ctx, &lie).is_err(),
            "the second kernel must refuse a wrong value for `{name}`"
        );
    }
}

/// `redex` with its head constant replaced by that constant's definition: δ and nothing else.
fn unfolded(ctx: &Context, redex: &Term) -> Term {
    match redex {
        Term::App(f, x) => Term::App(Box::new(unfolded(ctx, f)), x.clone()),
        Term::Global(name) => ctx
            .get_definition_body(name)
            .unwrap_or_else(|| panic!("`{name}` is a checked definition"))
            .clone(),
        other => panic!("`{other}` is not an application of a constant"),
    }
}

/// THE BOUNDARY: the rules must stay STUCK on an operand outside the grammar, in BOTH kernels.
/// A native rule that fires on a variable, or on the wrong literal kind, has invented a value. The
/// operations are checked definitions, so on such an operand the main kernel may only unfold them:
/// its normal form is the definition's own, stuck on the operand, and never a literal.
#[test]
fn the_reflection_rules_stay_stuck_on_an_operand_outside_the_grammar_in_both_kernels() {
    let mut ctx = std_ctx();
    ctx.add_declaration("s", g("Syntax"));
    let stuck = [
        app(g("syn_size"), g("s")),
        app(g("syn_max_var"), g("s")),
        apps(g("syn_lift"), &[intlit(1), intlit(0), g("s")]),
        apps(g("syn_subst"), &[slit(1), intlit(0), g("s")]),
        apps(g("syn_beta"), &[g("s"), slit(1)]),
        app(g("syn_size"), sapp(svar(0), g("s"))),
    ];
    for redex in stuck {
        let reduced = normalize(&ctx, &redex);
        assert_eq!(
            reduced,
            normalize(&ctx, &unfolded(&ctx, &redex)),
            "`{redex}` has no computation rule: the main kernel may only unfold the definition"
        );
        assert!(
            !matches!(reduced, Term::Lit(_)),
            "`{redex}` must not compute a value, got {reduced}"
        );
        let self_citation = cites(g("Syntax"), redex.clone(), redex.clone());
        assert!(
            !matches!(double_check(&ctx, &self_citation), DoubleCheck::Disagree(_)),
            "the second kernel must treat `{redex}` as stuck too: {:?}",
            double_check(&ctx, &self_citation)
        );
    }
}

/// THE WHOLE CLASS, INCLUDING THE RULES THE SECOND KERNEL DOES NOT IMPLEMENT. A tactic
/// primitive runs an ENGINE — ring normalisation, Fourier–Motzkin, congruence closure — and a
/// second kernel that re-derived one of those would be the same implementation twice, which
/// corroborates nothing. The obligation is therefore not "compute it" but "do not cry wolf":
/// whatever the second kernel does with a registered primitive, it must never report `Disagree`
/// about a term the first kernel reduced by a rule of the trusted base.
#[test]
fn no_registered_primitive_makes_the_second_kernel_report_a_soundness_alarm() {
    let ctx = std_ctx();
    // The tactic primitives are declared `Syntax → Derivation`, so the goal has to be the
    // REFLECTED proposition — `Eq Int 2 2` written in the object language — or the citation is
    // ill-typed and every row skips itself, which is how this test would pass by touching
    // nothing.
    let goal = sapp(sapp(sapp(sname("Eq"), sname("Int")), slit(2)), slit(2));
    let tactic_redexes = vec![
        ("try_refl", app(g("try_refl"), goal.clone())),
        ("try_compute", app(g("try_compute"), goal.clone())),
        ("try_ring", app(g("try_ring"), goal.clone())),
        ("try_lia", app(g("try_lia"), goal.clone())),
        ("try_cc", app(g("try_cc"), goal.clone())),
        ("try_simp", app(g("try_simp"), goal.clone())),
        ("try_omega", app(g("try_omega"), goal.clone())),
        ("try_auto", app(g("try_auto"), goal.clone())),
        ("try_decide", app(g("try_decide"), goal.clone())),
        ("try_assumption", app(g("try_assumption"), goal.clone())),
        ("try_split", app(g("try_split"), goal.clone())),
        ("try_left", app(g("try_left"), goal.clone())),
        ("try_right", app(g("try_right"), goal.clone())),
        ("try_bitblast", app(g("try_bitblast"), goal.clone())),
        ("try_tabulate", app(g("try_tabulate"), goal.clone())),
        ("try_hw_auto", app(g("try_hw_auto"), goal.clone())),
        ("try_inversion", app(g("try_inversion"), goal.clone())),
        ("tact_fail", app(g("tact_fail"), goal.clone())),
        (
            "concludes",
            app(g("concludes"), app(g("DAxiom"), sname("True"))),
        ),
        (
            "induction_num_cases",
            app(g("induction_num_cases"), sname("Nat")),
        ),
    ];
    let mut exercised: Vec<&str> = Vec::new();
    for (name, redex) in tactic_redexes {
        let value = normalize(&ctx, &redex);
        if value == redex {
            continue; // the main kernel did not compute it either — nothing to corroborate
        }
        let carrier = infer_type(&ctx, &value).unwrap_or_else(|_| g("Derivation"));
        let cited = cites(carrier, redex.clone(), value.clone());
        if infer_type(&ctx, &cited).is_err() {
            continue; // the citation itself is not well-typed; that is not this test's subject
        }
        exercised.push(name);
        let verdict = double_check(&ctx, &cited);
        assert!(
            !matches!(verdict, DoubleCheck::Disagree(_)),
            "`{name}` is a rule of the trusted base that the main kernel computed. The second \
             kernel may corroborate it or may say it did not check it — it may NOT report a \
             soundness alarm about a valid proof: {verdict:?}"
        );
    }
    // NOT VACUOUS. Both `continue`s above are escape hatches, and a test made of escape hatches
    // passes by touching nothing — so the rows that actually reached the assertion are counted
    // and the count is pinned.
    assert_eq!(
        exercised.len(),
        20,
        "every one of the twenty tactic rules must actually REACH the assertion; the two \
         `continue`s above are escape hatches, and a test made of escape hatches passes by \
         touching nothing. Exercised: {exercised:?}"
    );
}

/// THE RATCHET. The second kernel must compute or name every primitive the first kernel dispatches
/// on, so a rule added to one kernel cannot silently be missing from the other: its primitive table
/// names the rule, or the name is a checked definition of the standard library, which the second
/// kernel computes by unfolding (the native rule then accelerates an attested definition, and
/// `every_reflection_rule_the_main_kernel_computes_the_second_kernel_computes_too` holds the two to
/// one value). Read straight out of `reduction.rs`'s dispatcher, because a hand-kept list is a list
/// that drifts.
#[test]
fn the_second_kernels_primitive_table_names_every_primitive_the_first_kernel_dispatches_on() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../logicaffeine_kernel/src/reduction.rs"
    ))
    .expect("the first kernel's reducer is readable from its own crate");
    let start = src
        .find("fn try_reflection_reduce")
        .expect("the dispatcher is still called `try_reflection_reduce`");
    let body = &src[start..];
    let end = body[1..].find("\nfn ").map(|i| i + 1).unwrap_or(body.len());
    let dispatcher = &body[..end];

    let mut dispatched: Vec<String> = Vec::new();
    for piece in dispatcher.split('"').skip(1).step_by(2) {
        let looks_like_a_primitive = piece.starts_with("syn_")
            || piece.starts_with("tact_")
            || piece.starts_with("try_")
            || piece.starts_with("induction_")
            || piece == "concludes";
        if looks_like_a_primitive && !dispatched.iter().any(|d| d == piece) {
            dispatched.push(piece.to_string());
        }
    }
    assert!(
        dispatched.len() >= 25,
        "the dispatcher scan found only {} names — the scan itself has broken: {dispatched:?}",
        dispatched.len()
    );

    let ctx = std_ctx();
    let table = logicaffeine_kernel::recheck::primitive_rule_names();
    for name in &dispatched {
        assert!(
            table.contains(&name.as_str()) || ctx.is_definition(name),
            "`{name}` is a reduction rule of the FIRST kernel that the second kernel's table does \
             not name and the standard library does not define. Every such rule must be either \
             implemented independently, declared unchecked, or a checked definition the second \
             kernel unfolds — any other rule is one the second kernel silently treats as a value, \
             and then DISAGREES about: {table:?}"
        );
    }
}

// ============================================================================================
// 20 · A UNIVERSE-POLYMORPHIC TYPE DECLARATION IS A DECLARATION, NOT A DEFINITION
// ============================================================================================
//
// Three corpus cases — Mathlib's `class Mul (α : Type u)`, Lean's `Prod : Type u → Type v →
// Type (max u v)`, and a record referenced at an explicit level — all reduce to one question:
// can a TYPE declaration (which has no body, and so cannot live in the definitions-only
// universe-polymorphic map) carry level parameters and be instantiated at several levels at
// once? These pin the answer from both sides.
//
// The rule, stated once: a declaration's level telescope is DERIVED from the free level
// variables of the type its author wrote, in first-occurrence order. So a structure written
// over `Sort u` is polymorphic and a structure written over `Type 0` is not — which is also
// Lean's answer (`structure Cell (α : Type)` gives `Cell : Type → Type` with an EMPTY
// `levelParams`, and `Cell.{1}` is an error there too). Generalising a level the author wrote
// concretely is the one repair that must never be made: it would silently change the shape of
// every prelude structure, and it would make the level argument unconstrained by anything.

/// `structure S.{u} (A : Sort u) where val : A` — Lean's universe-polymorphic record, in the
/// only spelling that can be polymorphic: the level variable appears in what the author WROTE.
fn register_poly_cell(ctx: &mut Context) {
    ctx.add_structure("Cell", &[("A", sort(uvar("u")))], &[("val", var("A"))]);
}

/// THE CAPABILITY. One declaration, every level — resolved explicitly through `Const`, and
/// implicitly through a bare reference whose level the argument decides.
#[test]
fn a_structure_written_over_sort_u_is_one_declaration_at_every_level() {
    let mut ctx = std_ctx();
    register_poly_cell(&mut ctx);

    for level in [
        Universe::Prop,
        Universe::Type(0),
        Universe::Type(1),
        Universe::Type(7),
    ] {
        let at = konst("Cell", vec![level.clone()]);
        assert!(
            infer_type(&ctx, &at).is_ok(),
            "`Cell.{{{level}}}` must resolve — a structure has to carry its own universe \
             parameter, and it has no body, so the definitions-only polymorphic map cannot \
             hold it: {:?}",
            infer_type(&ctx, &at)
        );
        assert_eq!(
            double_check(&ctx, &at),
            DoubleCheck::Agreed,
            "…and the second kernel must resolve it the same way: {:?}",
            double_check(&ctx, &at)
        );
    }

    // A BARE reference, with the level solved from the argument — the shape Mathlib writes and
    // the shape `Mul Nat` / `Mul (Type 0)` need. ONE declaration serves both.
    for carrier in [g("Nat"), g("True"), ty(0), ty(3)] {
        let applied = app(g("Cell"), carrier.clone());
        assert!(
            infer_type(&ctx, &applied).is_ok(),
            "`Cell {carrier}` must type — the same declaration has to serve a carrier at every \
             level, which is exactly what defeats per-instantiation monomorphisation: {:?}",
            infer_type(&ctx, &applied)
        );
        assert_eq!(
            double_check(&ctx, &applied),
            DoubleCheck::Agreed,
            "…in both kernels: {:?}",
            double_check(&ctx, &applied)
        );
    }

    // …and the constructor and projection travel with it: a `Type 1` value really is STORED.
    let big = apps(g("Cell_mk"), &[ty(0), g("Nat")]);
    assert!(
        infer_type(&ctx, &big).is_ok(),
        "`Cell_mk (Type 0) Nat` must type — a record whose sort tracked only `Type 0` would \
         store a `Type 1` value in a `Type 0` box: {:?}",
        infer_type(&ctx, &big)
    );
    assert_eq!(
        normalize(&ctx, &apps(g("Cell_val"), &[ty(0), big])),
        g("Nat"),
        "…and the projection recovers it"
    );
}

/// THE NEGATIVE, and it is the one that matters: a structure whose parameter the author pinned
/// at a CONCRETE level has NO level telescope, so a reference carrying one is a statement about
/// a declaration that does not exist. Lean answers the same way. Generalising here would make
/// the level argument mean nothing at all.
#[test]
fn a_structure_written_at_a_concrete_level_has_no_level_to_instantiate() {
    let mut ctx = std_ctx();
    ctx.add_structure("Flat", &[("A", ty(0))], &[("val", var("A"))]);

    assert!(
        infer_type(&ctx, &konst("Flat", vec![Universe::Type(1)])).is_err(),
        "`Flat.{{1}}` names a level parameter `Flat` does not have; accepting it would let a \
         level argument be chosen freely and constrain nothing"
    );
    assert!(
        infer_type(&ctx, &app(g("Flat"), ty(0))).is_err(),
        "…and `Flat (Type 0)` must stay refused: the author wrote `Type 0` for the parameter, \
         so a `Type 1` carrier is outside the declaration, not a level to be solved"
    );
    // THE BOUNDARY: at the level it WAS written for, everything still works.
    assert!(
        infer_type(&ctx, &app(g("Flat"), g("Nat"))).is_ok(),
        "a monomorphic structure still serves the level it was declared at"
    );
    // WRONG ARITY, one rung either side, on the polymorphic one.
    register_poly_cell(&mut ctx);
    for levels in [vec![], vec![Universe::Type(0), Universe::Type(1)]] {
        assert!(
            infer_type(&ctx, &konst("Cell", levels.clone())).is_err(),
            "`Cell` binds exactly one level; {} is the wrong arity",
            levels.len()
        );
    }
}

/// TWO INDEPENDENT LEVELS on one declaration — the shape that defeats even per-instantiation
/// monomorphisation, because the two levels vary independently and a flat name would collapse
/// them. `Prod.{u,v} : Sort u → Sort v → Sort (max u v)` is Lean's own.
#[test]
fn one_type_declaration_carries_two_independent_levels() {
    let mut ctx = std_ctx();
    ctx.add_structure(
        "Pair",
        &[("A", sort(uvar("u"))), ("B", sort(uvar("v")))],
        &[("fst", var("A")), ("snd", var("B"))],
    );

    // Each carrier at its own level, in every combination — including the two that differ.
    for (a, b) in [
        (g("Nat"), g("Bool")),
        (ty(0), g("Nat")),
        (g("Nat"), ty(0)),
        (ty(1), ty(0)),
    ] {
        let applied = apps(g("Pair"), &[a.clone(), b.clone()]);
        assert!(
            infer_type(&ctx, &applied).is_ok(),
            "`Pair {a} {b}` must be typed by ONE declaration — the levels vary independently, \
             so no single monomorphisation can serve them: {:?}",
            infer_type(&ctx, &applied)
        );
        assert_eq!(
            double_check(&ctx, &applied),
            DoubleCheck::Agreed,
            "…and both kernels must instantiate the pair the same way: {:?}",
            double_check(&ctx, &applied)
        );
    }

    // POSITIONAL, and the order is OBSERVABLE here because the two carriers sit at different
    // levels: `Pair.{1,0}` accepts `(Type 0) Nat` and `Pair.{0,1}` — the same list swapped —
    // must refuse it. A zip that lost the order would accept both.
    let swapped = konst("Pair", vec![Universe::Type(1), Universe::Type(0)]);
    let forward = konst("Pair", vec![Universe::Type(0), Universe::Type(1)]);
    assert!(
        infer_type(&ctx, &swapped).is_ok(),
        "`Pair.{{1,0}}` resolves"
    );
    assert!(
        infer_type(&ctx, &forward).is_ok(),
        "`Pair.{{0,1}}` resolves"
    );
    assert!(
        infer_type(&ctx, &apps(swapped, &[ty(0), g("Nat")])).is_ok(),
        "`Pair.{{1,0}} (Type 0) Nat` — a `Type 1` carrier first, a `Type 0` carrier second"
    );
    assert!(
        infer_type(&ctx, &apps(forward, &[ty(0), g("Nat")])).is_err(),
        "…and the SWAPPED list must refuse the same arguments, or the positional zip is not \
         positional at all"
    );
}

// ============================================================================================
// 21 · A VALUE IN A TYPE POSITION IS THE ELABORATOR'S JOB, NOT THE KERNEL'S
// ============================================================================================
//
// `CoeSort` is what lets `t : Ty` stand where a sort is expected. In Lean the ELABORATOR
// inserts it and the kernel never sees the coercion: what reaches the kernel is the PROJECTION,
// `Π(x : Ty.carrier t). …`, an ordinary well-typed Π. Teaching the kernel to insert it instead
// would put a coercion rule inside the trusted base — and a rule that has to GUESS, since a
// structure with two sort-valued fields gives it two answers and a structure with none gives it
// a wrong one.
//
// So the property is a PAIR, and both halves matter: the kernel must REFUSE the un-coerced
// form, and must ACCEPT the projected one. Anything that satisfies only the first half is a
// kernel that cannot express the coercion; anything that satisfies only the second is a kernel
// doing the elaborator's job.

#[test]
fn a_structure_value_is_refused_in_a_type_position_and_its_projection_is_not() {
    let mut ctx = std_ctx();
    // `structure Ty where carrier : Type` — the shape a `CoeSort` instance is written for.
    ctx.add_structure("Ty", &[], &[("carrier", ty(0))]);
    let value = app(g("Ty_mk"), g("Nat"));

    // THE REFUSAL. `Ty_mk Nat : Ty`, and `Ty` is not a sort, so `Π(x : t). t` is not a type.
    let uncoerced = pi("x", value.clone(), value.clone());
    assert!(
        infer_type(&ctx, &uncoerced).is_err(),
        "a VALUE is not a SORT. Accepting `Π(x : Ty_mk Nat). …` would put a coercion rule in \
         the trusted base — one that must GUESS which field to project, and that guesses \
         wrongly for a structure with two sort-valued fields or none: {:?}",
        infer_type(&ctx, &uncoerced)
    );
    assert!(
        recheck(&ctx, &uncoerced).is_err(),
        "…and the second kernel must refuse it independently"
    );
    assert_eq!(
        double_check(&ctx, &uncoerced),
        DoubleCheck::Agreed,
        "both kernels examining it and both refusing IS agreement — the strongest rejection \
         this API reports: {:?}",
        double_check(&ctx, &uncoerced)
    );

    // THE ACCEPTANCE, and it needs nothing new: the projection the elaborator inserts is an
    // ordinary application, and the Π it builds type-checks, computes and double-checks today.
    let projected = app(g("Ty_carrier"), value.clone());
    assert_eq!(
        infer_type(&ctx, &projected).expect("the projection is well typed"),
        ty(0),
        "`Ty.carrier t : Type 0` — a SORT, which is what a type position wants"
    );
    let coerced = pi("x", projected.clone(), projected.clone());
    assert!(
        infer_type(&ctx, &coerced).is_ok(),
        "`Π(x : Ty.carrier t). Ty.carrier t` is what the kernel must see, and it checks: {:?}",
        infer_type(&ctx, &coerced)
    );
    assert_eq!(
        normalize(&ctx, &coerced),
        pi("x", g("Nat"), g("Nat")),
        "…and it computes to the type the coercion was standing for"
    );
    assert_eq!(
        double_check(&ctx, &coerced),
        DoubleCheck::Agreed,
        "…in both kernels: {:?}",
        double_check(&ctx, &coerced)
    );

    // THE BOUNDARY: the projection of a DIFFERENT structure's value is still refused, so the
    // acceptance above is about the projection and not about the shape of an application.
    ctx.add_structure("Boxed", &[], &[("held", g("Nat"))]);
    let wrong = app(g("Boxed_held"), app(g("Boxed_mk"), g("Zero")));
    assert!(
        infer_type(&ctx, &pi("x", wrong.clone(), wrong)).is_err(),
        "a `Nat`-valued projection is still a value, not a sort"
    );
}

// ============================================================================================
// 22 · THE SECOND KERNEL'S ANSWER MUST BE A TERM THE FIRST KERNEL CAN READ
// ============================================================================================
//
// `recheck` reports the type it inferred, and it reports it in the FIRST kernel's named
// representation — so the de Bruijn → named direction is part of the two-kernel contract, not
// an internal convenience. A binder whose name does not match the scheme its own bound
// occurrences get produces a term with a FREE variable where a bound one belongs; the round
// trip back into de Bruijn then fails, and `double_check` quietly reports the term as
// single-checked with a message about a "fragment" it in fact handled perfectly.
//
// A `Fix` is where that bites: it binds one de Bruijn level exactly as a `λ` does.

/// `fix f. λn:Nat. match n with Zero ⇒ Zero | Succ k ⇒ f k` — genuine structural recursion,
/// accepted by both kernels' guards.
fn nat_fixpoint() -> Term {
    Term::Fix {
        name: "f".to_string(),
        body: Box::new(lam(
            "n",
            g("Nat"),
            Term::Match {
                discriminant: Box::new(var("n")),
                motive: Box::new(lam("_", g("Nat"), g("Nat"))),
                cases: vec![g("Zero"), lam("k", g("Nat"), app(var("f"), var("k")))],
            },
        )),
    }
}

#[test]
fn a_type_that_mentions_a_fixpoint_survives_the_second_kernels_report() {
    let ctx = std_ctx();
    // `let fac : Nat → Nat := <fix> in λn:Nat. refl Nat (fac n)`. ζ runs before inference, so
    // the INFERRED TYPE is `Π(n:Nat). Eq Nat ((fix …) n) ((fix …) n)` — a type with a fixpoint
    // inside it, which is the only way a `Fix` reaches the reporting path at all.
    let term = Term::Let {
        name: "fac".to_string(),
        ty: Box::new(arrow(g("Nat"), g("Nat"))),
        value: Box::new(nat_fixpoint()),
        body: Box::new(lam(
            "n",
            g("Nat"),
            refl(g("Nat"), app(var("fac"), var("n"))),
        )),
    };

    let main_ty = infer_type(&ctx, &term).expect("the main kernel types it");
    assert!(
        format!("{main_ty}").contains("fix"),
        "the inferred type must actually mention the fixpoint, or this test is vacuous: {main_ty}"
    );

    let re_ty = recheck(&ctx, &term).expect("the second kernel types it too");
    assert!(
        infer_type(&ctx, &re_ty).is_ok(),
        "the type the SECOND kernel reports must be a term the FIRST kernel can read. A binder \
         named by one scheme whose occurrences are named by another hands back a free variable \
         where a bound one belongs: {re_ty}"
    );
    assert!(
        is_subtype(&ctx, &re_ty, &main_ty) && is_subtype(&ctx, &main_ty, &re_ty),
        "…and it must be the same type the first kernel inferred:\n  main    = {main_ty}\n  \
         recheck = {re_ty}"
    );
    assert_eq!(
        double_check(&ctx, &term),
        DoubleCheck::Agreed,
        "…so the verdict is AGREEMENT, not a report that the term was outside the second \
         kernel's fragment when it was not: {:?}",
        double_check(&ctx, &term)
    );
}

// ============================================================================================
// 23 · A FIXPOINT UNROLLS AT ITS STRUCTURAL COLUMN, IN BOTH KERNELS
// ============================================================================================
//
// The ι rule for a fixpoint fires when the argument at the column the termination guard
// admitted it on is constructor-headed. A reducer that instead asks about argument 0 — which
// is all a per-application-node descent has in hand — never computes a recursion that descends
// on its SECOND argument. That is not a symmetric shortfall the moment ONE kernel is fixed:
// the other then abandons a reduction its partner completes, and `double_check` reports
// `Disagree` about a proof that computes perfectly well.

#[test]
fn a_fixpoint_that_recurses_on_its_second_argument_computes_in_both_kernels() {
    let mut ctx = std_ctx();
    // An OPAQUE `Nat`, so argument 0 is never constructor-headed and an argument-0 guard can
    // never fire. This is the whole fixture: without it the two rules are indistinguishable.
    ctx.add_declaration("a", g("Nat"));

    // `fix f. λacc:Nat. λn:Nat. match n with Zero ⇒ acc | Succ k ⇒ f (Succ acc) k`
    // — an accumulator at column 0 that GROWS, and the recursion at column 1.
    let body = lam(
        "acc",
        g("Nat"),
        lam(
            "n",
            g("Nat"),
            Term::Match {
                discriminant: Box::new(var("n")),
                motive: Box::new(lam("_", g("Nat"), g("Nat"))),
                cases: vec![
                    var("acc"),
                    lam(
                        "k",
                        g("Nat"),
                        apps(var("f"), &[app(g("Succ"), var("acc")), var("k")]),
                    ),
                ],
            },
        ),
    );
    let f = Term::Fix {
        name: "f".to_string(),
        body: Box::new(body),
    };

    // Two rungs of recursion, so the rule has to fire twice.
    let redex = apps(
        f.clone(),
        &[g("a"), app(g("Succ"), app(g("Succ"), g("Zero")))],
    );
    let expected = app(g("Succ"), app(g("Succ"), g("a")));
    assert_eq!(
        normalize(&ctx, &redex),
        expected,
        "the first kernel unrolls at the STRUCTURAL column: argument 0 is an opaque `a` and \
         never constructor-headed, so a rule that asked about argument 0 would abandon this"
    );
    let cited = cites(g("Nat"), redex.clone(), expected.clone());
    assert!(
        infer_type(&ctx, &cited).is_ok(),
        "…so a proof citing the computation type-checks"
    );
    assert_eq!(
        double_check(&ctx, &cited),
        DoubleCheck::Agreed,
        "…and the SECOND kernel must reach the same value. A kernel that stopped at argument 0 \
         here would abandon a reduction its partner completes, and report the difference as a \
         soundness alarm about a valid proof: {:?}",
        double_check(&ctx, &cited)
    );

    // THE NEGATIVE: the rule must DECIDE, not merely accept.
    let lie = cites(g("Nat"), redex, app(g("Succ"), g("a")));
    assert!(
        recheck(&ctx, &lie).is_err(),
        "the second kernel must refuse a wrong value for the accumulated recursion"
    );

    // THE BOUNDARY, one rung either side: with a constructor at BOTH columns the ordinary
    // argument-0 rule would also fire, so the two answers must coincide there too.
    let both = apps(f, &[g("Zero"), app(g("Succ"), g("Zero"))]);
    let both_value = normalize(&ctx, &both);
    assert_eq!(
        both_value,
        app(g("Succ"), g("Zero")),
        "`f Zero 1` accumulates one `Succ`"
    );
    assert_eq!(
        double_check(&ctx, &cites(g("Nat"), both, both_value)),
        DoubleCheck::Agreed,
        "…and both kernels agree where both rules could have fired"
    );
}

// ============================================================================================
// 16 · A UNIVERSE-POLYMORPHIC *TYPE* DECLARATION IS REACHABLE FROM BOTH KERNELS' `Const` RULE
// ============================================================================================
//
// `Context::add_universe_poly_inductive` records a level TELESCOPE beside a type former that has
// no body. `S_spine_extra::a_polymorphic_type_declaration_carries_a_level_telescope_and_no_body`
// pins the registry; this pins what the two KERNELS do with it, which is the half that decides
// whether `T01-011` / `T07-011` / `T00-011` can be written at all.
//
// The distinction the tests below are built to expose: a declaration's telescope may be STATED
// (`add_universe_poly_inductive`'s parameter list) or DERIVED (the free level variables of its
// stored type, in first-occurrence order — `term::free_level_vars`). Those two orders coincide
// for every declaration written in the obvious way, and part company the moment the arity
// mentions its parameters out of order. A kernel that silently derives where a telescope was
// stated instantiates the WRONG parameter, and — because both kernels would derive identically —
// `double_check` reports `Agreed` about it. Lean has no such hazard: `ConstantInfo.levelParams`
// is stored on every declaration and `Level.instantiateParams`
// (`Lean/Level.lean:594`) zips that stored list positionally, never a list recovered from the
// type.

/// `Swapped.{u,v} : Sort v → Sort u → Sort (max u v)` — a stated telescope `[u, v]` whose arity
/// mentions `v` FIRST, so the derived order is `[v, u]` and the two disagree on every
/// instantiation with distinct levels.
fn swapped_arity() -> Term {
    pi(
        "a",
        sort(uvar("v")),
        pi("b", sort(uvar("u")), sort(uvar("u").max(&uvar("v")))),
    )
}

#[test]
fn a_polymorphic_type_declaration_instantiates_the_telescope_it_declared_not_the_one_it_looks_like()
{
    let mut ctx = std_ctx();
    ctx.add_universe_poly_inductive(
        "Swapped",
        vec!["u".to_string(), "v".to_string()],
        swapped_arity(),
        2,
    );

    // THE CAPABILITY. A `Const` reference to a body-less polymorphic TYPE resolves at all.
    let at = konst("Swapped", vec![Universe::Type(0), Universe::Type(3)]);
    let got = infer_type(&ctx, &at).unwrap_or_else(|e| {
        panic!("a polymorphic type declaration must resolve through the `Const` rule: {e:?}")
    });

    // THE DISCRIMINATOR. `u := Type 0`, `v := Type 3` positionally against the DECLARED list, so
    // the domain (`Sort v`) is `Type 3` and the second binder (`Sort u`) is `Type 0`. Deriving
    // the telescope from the arity instead yields `[v, u]` and swaps them — same arity, same
    // level arguments, a different type, and no error anywhere to show for it.
    let want = pi("a", ty(3), pi("b", ty(0), ty(3)));
    assert_eq!(
        got,
        want,
        "the STATED telescope is zipped positionally; deriving `[v, u]` from the arity would \
         give `{}` instead",
        pi("a", ty(0), pi("b", ty(3), ty(3)))
    );

    // BOTH KERNELS, and the check that the two domain levels are the ones claimed rather than
    // merely printed: the reference is APPLIED to a carrier at each level in turn. `Type 2`
    // inhabits `Sort (Type 3)` and `Nat` inhabits `Sort (Type 0)`, so the swapped reading
    // refuses the very arguments the declared one accepts.
    let used = apps(at.clone(), &[ty(2), g("Nat")]);
    assert_eq!(
        infer_type(&ctx, &used).expect("applied at the declared levels"),
        ty(3),
        "`Swapped.{{0,3}} (Type 2) Nat : Sort (max u v) = Type 3`"
    );
    assert!(
        recheck(&ctx, &used).is_ok(),
        "the second kernel must resolve the same reference: {:?}",
        recheck(&ctx, &used)
    );
    assert!(
        infer_type(&ctx, &apps(at.clone(), &[g("Nat"), ty(2)])).is_err(),
        "…and the arguments the DERIVED order would have accepted must be refused"
    );
    assert_eq!(
        double_check(&ctx, &used),
        DoubleCheck::Agreed,
        "…and reach the same answer: {:?}",
        double_check(&ctx, &used)
    );

    // THE NEGATIVE — the telescope is an ARITY, not a suggestion. One level too few is a
    // statement about a declaration that does not exist.
    for wrong in [vec![Universe::Type(0)], vec![], vec![Universe::Type(0); 3]] {
        let bad = konst("Swapped", wrong.clone());
        assert!(
            infer_type(&ctx, &bad).is_err(),
            "`Swapped` at {} level argument(s) must be refused",
            wrong.len()
        );
        assert!(recheck(&ctx, &bad).is_err(), "…by the second kernel too");
        assert_eq!(
            double_check(&ctx, &bad),
            DoubleCheck::Agreed,
            "two independent refusals are an agreement: {:?}",
            double_check(&ctx, &bad)
        );
    }

    // THE BOUNDARY, one rung either side of the swap: at EQUAL levels the two telescopes agree,
    // which is exactly why a suite that only ever instantiates `.{0,0}` cannot see this bug.
    let flat = konst("Swapped", vec![Universe::Type(1), Universe::Type(1)]);
    assert_eq!(
        infer_type(&ctx, &flat).expect("resolves"),
        pi("a", ty(1), pi("b", ty(1), ty(1))),
        "a symmetric instantiation hides the order entirely"
    );

    // AND IT IS STILL A TYPE FORMER: no body, so a reference to it is its own normal form in
    // both kernels. A telescope recorded in the δ-unfolding registry instead would make this
    // unfold into nothing.
    assert_eq!(
        normalize(&ctx, &at),
        at,
        "a polymorphic type former is inert"
    );
}

/// The same primitive at the shape the corpus actually needs: ONE declaration serving two
/// carrier levels, with a constructor that stores a field at the declared level. This is
/// `T07-011`'s class, `T01-011`'s structure and `T00-011`'s `Sigma` reduced to the kernel
/// primitive underneath all three — so it goes green the moment the `Const` rule reads the
/// telescope, regardless of which declaration door the surface eventually uses.
#[test]
fn one_polymorphic_type_declaration_serves_two_carrier_levels_in_both_kernels() {
    let mut ctx = std_ctx();
    // `Cell.{u} : Sort u → Sort u`, with `Cell_mk.{u} : Π(A : Sort u). A → Cell A`.
    ctx.add_universe_poly_inductive(
        "Cell",
        vec!["u".to_string()],
        pi("A", sort(uvar("u")), sort(uvar("u"))),
        1,
    );
    ctx.add_constructor_checked(
        "Cell_mk",
        "Cell",
        pi(
            "A",
            sort(uvar("u")),
            arrow(var("A"), app(g("Cell"), var("A"))),
        ),
    )
    .expect("the constructor registers against the polymorphic header");

    // TWO LEVELS, ONE DECLARATION — the whole point.
    for level in [0u32, 1, 2, 7] {
        let at = konst("Cell", vec![Universe::Type(level)]);
        assert_eq!(
            infer_type(&ctx, &at).unwrap_or_else(|e| panic!("`Cell.{{{level}}}`: {e:?}")),
            pi("A", ty(level), ty(level)),
            "`Cell.{{{level}}}` is the same declaration at another level"
        );
        // Applied to a carrier AT that level, the record lands there too.
        let carrier = if level == 0 { g("Nat") } else { ty(level - 1) };
        let applied = app(at, carrier);
        assert_eq!(
            infer_type(&ctx, &applied).unwrap_or_else(|e| panic!("applied at {level}: {e:?}")),
            ty(level),
            "a `Type {level}` carrier gives a `Type {level}` record — no `ULift` anywhere"
        );
        assert_eq!(
            double_check(&ctx, &applied),
            DoubleCheck::Agreed,
            "…and the second kernel agrees at level {level}: {:?}",
            double_check(&ctx, &applied)
        );
    }

    // THE NEGATIVE that says the level is really CHECKED and not merely carried: a `Type 1`
    // carrier is not a `Type 0` one, so `Cell.{0}` must refuse it.
    assert!(
        infer_type(&ctx, &app(konst("Cell", vec![Universe::Type(0)]), ty(1))).is_err(),
        "`Cell.{{0}}` applied to a `Type 1` carrier must be refused — a level that is carried \
         but never checked is not a level parameter"
    );
}

// ============================================================================================
// 17 · THE `Nat`-ORDER ACCELERATION REACHES PAST THE RECURSION — IN BOTH KERNELS, TOGETHER
// ============================================================================================
//
// ★ THIS SUPERSEDES THE "CEILING" CLAUSE of
// `the_nat_order_shortcut_agrees_with_the_structural_recursion_everywhere_it_answers`, which was
// written when only the SECOND kernel accelerated and therefore asserted that the main kernel
// abandons `2^64 < 2^64`. It does not: `reduction::accelerates_nat_order` gates the identical
// override in the main kernel, so both answer. Lean settles which behaviour is right —
// `Lean/Meta/WHNF.lean:934-956` reduces `Nat.ble` on two literals keyed on the constant's NAME
// (`| ``Nat.ble => reduceBinNatPred Nat.ble a1 a2`, line 951), with no reference to how far the
// reference definition would peel. So an accelerated order that reaches past the structural
// recursion is Lean's behaviour, not a hazard.
//
// What IS load-bearing is the ALIGNMENT: the two kernels must answer in exactly the same places.
// One that reached further would decide a bound its partner abandons, and `double_check` reports
// that as a soundness alarm about a valid proof. So the test below pins both directions — where
// the gate is OPEN both answer, and where the gate is SHUT (a rebound `natLe`, whose structural
// recursion cannot finish on these operands) NEITHER answers and `double_check` never reports a
// disagreement.

/// A `Nat` literal spelled in decimal — the shape the acceleration is keyed on in BOTH kernels.
fn natbig_dec(d: &str) -> Term {
    Term::Lit(Literal::Nat(
        logicaffeine_base::BigInt::parse_decimal(d).expect("decimal `Nat` literal"),
    ))
}

/// `2^64`, far past any peel either kernel's step budget can finish.
fn beyond_every_peel() -> Term {
    natbig_dec("18446744073709551616")
}

#[test]
fn the_accelerated_nat_order_answers_past_the_recursions_reach_in_both_kernels_at_once() {
    let ctx = std_ctx();
    let huge = beyond_every_peel();

    // THE CAPABILITY, at operands no peel reaches. `min(m, n)` ι-steps would be 2^64 of them.
    for (op, m, n, want) in [
        ("natLt", huge.clone(), huge.clone(), "false"),
        ("natLe", huge.clone(), huge.clone(), "true"),
        // One rung either side, written as LITERALS. (`Succ <literal>` is deliberately not in
        // this grid: our reducer has no `Succ`-of-literal fold, so that spelling falls through
        // to a 2^64-step peel in both kernels. Lean folds it — `withNatValue` whnfs its argument
        // and `reduceNat?`'s `Nat.succ` branch turns `Nat.succ <lit>` into a raw literal
        // (`Lean/Meta/WHNF.lean:900-911` and `:936-940`) — so this is a real divergence, and it
        // belongs to the reducer's Peano bridge rather than to the order rule.)
        (
            "natLt",
            huge.clone(),
            natbig_dec("18446744073709551617"),
            "true",
        ),
        (
            "natLe",
            natbig_dec("18446744073709551617"),
            huge.clone(),
            "false",
        ),
    ] {
        let redex = apps(g(op), &[m, n]);
        assert_eq!(
            normalize(&ctx, &redex),
            g(want),
            "the MAIN kernel decides `{op}` at 2^64 — Lean's `reduceNat?` does the same \
             (`Lean/Meta/WHNF.lean:951`)"
        );
        assert_eq!(
            double_check(&ctx, &cites_bool(&redex, want)),
            DoubleCheck::Agreed,
            "…and the second kernel reaches the same answer: {:?}",
            double_check(&ctx, &cites_bool(&redex, want))
        );
        // THE NEGATIVE — it DECIDES, rather than returning a constant that happens to be right
        // half the time.
        let wrong = if want == "true" { "false" } else { "true" };
        assert!(
            infer_type(&ctx, &cites_bool(&redex, wrong)).is_err(),
            "the opposite answer for `{op}` at 2^64 must be refused"
        );
        assert!(
            recheck(&ctx, &cites_bool(&redex, wrong)).is_err(),
            "…by the second kernel too"
        );
    }

    // ★ THE ALIGNMENT AT THE CEILING, which is the clause that actually carries soundness. With
    // `natLe` REBOUND the gate shuts in both kernels, the structural recursion is what is left,
    // and 2^64 peels is past both budgets. Neither kernel may answer — and, decisively, neither
    // may report the other's silence as a DISAGREEMENT. The prelude's `natLe` keeps its program
    // (the integer order's laws rest on it), so the rebinding is made where nothing depends on
    // the name, beside the prelude's own `natLt` body over it.
    let mut rebound = order_free_ctx();
    rebound.add_definition(
        "natLe".to_string(),
        arrow(g("Nat"), arrow(g("Nat"), g("Bool"))),
        // THE SAME RECURSION with ONE token changed: the inner `Zero` arm answers `true` where
        // the order answers `false`. That single token shuts the structural gate
        // (`reduction::is_min_recursion_order` requires `false` there) while leaving the peel
        // exactly as expensive — so this context isolates the ACCELERATION and nothing else.
        // A program that also stopped recursing would prove nothing here, because it could
        // finish inside the budget for reasons that have nothing to do with the gate.
        Term::Fix {
            name: "f".to_string(),
            body: Box::new(lam(
                "m",
                g("Nat"),
                lam(
                    "n",
                    g("Nat"),
                    Term::Match {
                        discriminant: Box::new(var("m")),
                        motive: Box::new(lam("_", g("Nat"), g("Bool"))),
                        cases: vec![
                            g("true"),
                            lam(
                                "m0",
                                g("Nat"),
                                Term::Match {
                                    discriminant: Box::new(var("n")),
                                    motive: Box::new(lam("_", g("Nat"), g("Bool"))),
                                    cases: vec![
                                        g("true"),
                                        lam(
                                            "n0",
                                            g("Nat"),
                                            apps(var("f"), &[var("m0"), var("n0")]),
                                        ),
                                    ],
                                },
                            ),
                        ],
                    },
                ),
            )),
        },
    );
    rebound.add_definition(
        "natLt".to_string(),
        arrow(g("Nat"), arrow(g("Nat"), g("Bool"))),
        lam(
            "m",
            g("Nat"),
            lam(
                "n",
                g("Nat"),
                apps(g("natLe"), &[app(g("Succ"), var("m")), var("n")]),
            ),
        ),
    );
    assert!(
        rebound.refused_declarations().is_empty(),
        "{:?}",
        rebound.refused_declarations()
    );
    assert_prelude_nat_order_keeps_its_program(lam("m", g("Nat"), lam("n", g("Nat"), g("true"))));
    // Both answers. The main kernel must refuse each, and `double_check` may never report a
    // DISAGREEMENT. A rejection the re-checker reached only after its budget ran out is not an
    // independent verdict (`recheck::recheck_closed` reports it `Unsupported`), so the honest
    // verdict for a term both kernels abandon is `MainOnlyReCheckerIncomplete`; `Agreed` would be
    // the verdict of a re-checker that finished. A `Disagree` is the soundness alarm this clause
    // exists to rule out.
    let stuck = apps(g("natLt"), &[huge.clone(), huge.clone()]);
    for answer in ["true", "false"] {
        let cited = cites_bool(&stuck, answer);
        assert!(
            infer_type(&rebound, &cited).is_err(),
            "with the acceleration off, the main kernel may not reach `{answer}` for `2^64 < 2^64`"
        );
        let verdict = double_check(&rebound, &cited);
        assert!(
            matches!(
                verdict,
                DoubleCheck::Agreed | DoubleCheck::MainOnlyReCheckerIncomplete(_)
            ),
            "…and the second kernel may not reach past its partner either — one that did would \
             manufacture a soundness alarm about a proof both refuse: {verdict:?}"
        );
    }

    // THE BOUNDARY, one rung either side of the gate: the rebound program still COMPUTES where
    // its own recursion finishes, and both kernels run the rebound body rather than the
    // arithmetic the name used to stand for. `natLt 0 0` is `natLe (Succ 0) 0`, whose rebound
    // inner `Zero` arm answers `true` — the opposite of the real order's `false`.
    let small = apps(g("natLt"), &[natl(0), natl(0)]);
    assert_eq!(
        normalize(&rebound, &small),
        g("true"),
        "the rebound body decides where it can finish"
    );
    assert_eq!(
        double_check(&rebound, &cites_bool(&small, "true")),
        DoubleCheck::Agreed,
        "…and both kernels run the same rebound program: {:?}",
        double_check(&rebound, &cites_bool(&small, "true"))
    );
    assert_eq!(
        normalize(&std_ctx(), &small),
        g("false"),
        "…which is NOT what the prelude's own order says, so the gate is observable"
    );
}

// ============================================================================================
// 18 · THE MOTIVE'S SORT IS A LEVEL PARAMETER — one eliminator, every target universe
// ============================================================================================
//
// Lean states every core recursor with a motive in `Sort u`:
//
// ```lean
// theorem Eq.rec : {α : Sort u_2} → {a : α} → {motive : (a' : α) → a = a' → Sort u_1} → …
// ```
//
// (`Init/Prelude.lean` declares `Eq`/`Eq.refl` and the auto-generated `Eq.rec` carries that
// telescope; `Init/Core.lean`'s `Eq.mpr`/`Eq.ndrec` are written against it.) A monomorphic
// eliminator whose motive target is pinned to `Type 0` cannot transport a TYPE family, because
// a motive valued in `Type 0` has codomain `Type 1` — and that is exactly the corpus's `T06-002`
// and `T01-014`.
//
// `recursor::derive_recursor_universe_poly` already builds the polymorphic artefact. What this
// pins is the half that lives in the two KERNELS: a bare `Global` naming a declaration whose
// type carries a free level variable must have that variable turned into a metavariable and
// SOLVED from the motive actually supplied (`type_checker::implicit_levels_for_head` +
// `solve_domain_levels`), at more than one level, with the second kernel reaching the same
// answer. Without that, registering the polymorphic eliminator buys nothing.

#[test]
fn one_polymorphic_eliminator_serves_a_prop_motive_and_a_type_motive_alike() {
    let mut ctx = std_ctx();
    let (params, j_ty, body) = logicaffeine_kernel::derive_recursor_universe_poly(&ctx, "Eq")
        .expect("`Eq`'s eliminator must derive universe-polymorphically");
    // Lean's `Eq.rec.{u, u_1}`: ONE level parameter for the motive's sort, then `Eq.{u}`'s own
    // carrier level, so the telescope covers the eliminator's type and the explicit door below
    // can take it as it is.
    assert_eq!(
        params.len(),
        2,
        "the motive's sort is ONE level parameter, beside `Eq`'s own carrier level — got \
         {params:?} for `{j_ty}`"
    );
    let u = params[0].clone();
    assert!(
        crate_mentions(&j_ty, &u),
        "…and the parameter must actually appear in the eliminator's type: {j_ty}"
    );
    assert_eq!(
        params[1], "u",
        "…and the second is `Eq`'s own carrier level: {params:?}"
    );
    // Registered as an ordinary definition: the reference form is then a bare `Global`, and the
    // level is solved at the use site rather than written by the author. That is Lean's
    // `@[elab_as_elim]` experience, not an extra thing a proof has to spell.
    ctx.add_definition("J".to_string(), j_ty.clone(), body.clone());

    // `a = b → P a → P b` at THREE motive levels, in one declaration. `Prop` is what every
    // equality lemma needs; `Type 0` is what a `cast`/`Eq.mpr` needs; `Type 1` is the rung that
    // proves the parameter is a parameter and not a two-case special rule.
    for (what, target) in [("Prop", prop()), ("Type 0", ty(0)), ("Type 1", ty(1))] {
        // motive := λ(idx : Nat). λ(_h : Eq Nat a idx). <target>
        let motive = lam(
            "idx",
            g("Nat"),
            lam("_h", eq_at(g("Nat"), var("a"), var("idx")), target.clone()),
        );
        // λa b h. J Nat a motive <base> b h  :  Π a b. Eq Nat a b → <target>
        let base = match what {
            "Prop" => g("True"),
            _ => g("Nat"),
        };
        let transported = lam(
            "a",
            g("Nat"),
            lam(
                "b",
                g("Nat"),
                lam(
                    "h",
                    eq_at(g("Nat"), var("a"), var("b")),
                    apps(
                        g("J"),
                        &[g("Nat"), var("a"), motive, base, var("b"), var("h")],
                    ),
                ),
            ),
        );
        let claimed = pi(
            "a",
            g("Nat"),
            pi(
                "b",
                g("Nat"),
                arrow(eq_at(g("Nat"), var("a"), var("b")), target.clone()),
            ),
        );
        let inferred = infer_type(&ctx, &transported).unwrap_or_else(|e| {
            panic!(
                "J at a `{what}` motive must check — the level parameter is what makes ONE \
                    eliminator serve every target sort: {e:?}"
            )
        });
        assert!(
            is_subtype(&ctx, &inferred, &claimed),
            "J at a `{what}` motive must prove `{claimed}`, but its type is `{inferred}`"
        );
        assert_eq!(
            double_check(&ctx, &transported),
            DoubleCheck::Agreed,
            "…and the second kernel must solve the same level: {:?}",
            double_check(&ctx, &transported)
        );
    }

    // THE NEGATIVE — the level is SOLVED, not defaulted and not ignored. A base value at the
    // wrong sort is still refused, so the rule is not "accept anything once a level variable is
    // in scope".
    let bad_motive = lam(
        "idx",
        g("Nat"),
        lam("_h", eq_at(g("Nat"), var("a"), var("idx")), ty(0)),
    );
    let mismatched = lam(
        "a",
        g("Nat"),
        lam(
            "b",
            g("Nat"),
            lam(
                "h",
                eq_at(g("Nat"), var("a"), var("b")),
                // base is `Zero : Nat`, but the motive demands a `Type 0`.
                apps(
                    g("J"),
                    &[
                        g("Nat"),
                        var("a"),
                        bad_motive,
                        g("Zero"),
                        var("b"),
                        var("h"),
                    ],
                ),
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &mismatched).is_err(),
        "a base value at the wrong sort must still be refused: {:?}",
        infer_type(&ctx, &mismatched)
    );
    assert!(
        recheck(&ctx, &mismatched).is_err(),
        "…by the second kernel too"
    );

    // AND THE EXPLICIT FORM, which is the other half of the same declaration: registered with
    // its telescope, `J.{1}` names the level rather than solving it.
    let mut explicit = std_ctx();
    explicit.add_universe_poly("Jx", params.clone(), j_ty, body);
    assert!(
        explicit.refused_declarations().is_empty(),
        "{:?}",
        explicit.refused_declarations()
    );
    // The motive at `Type 1`, the carrier at `Type 0`.
    let at_one = konst("Jx", vec![Universe::Type(1), Universe::Type(0)]);
    assert!(
        infer_type(&explicit, &at_one).is_ok(),
        "`Jx.{{1, 0}}` must resolve: {:?}",
        infer_type(&explicit, &at_one)
    );
    assert!(
        infer_type(&explicit, &konst("Jx", vec![])).is_err(),
        "…and `Jx.{{}}` must not — the telescope is an arity"
    );
    assert!(
        infer_type(&explicit, &konst("Jx", vec![Universe::Type(1)])).is_err(),
        "…nor `Jx.{{1}}`, which leaves the carrier's level unwritten"
    );
    assert_eq!(
        double_check(&explicit, &at_one),
        DoubleCheck::Agreed,
        "both kernels resolve the explicit form identically: {:?}",
        double_check(&explicit, &at_one)
    );
}

/// Whether a term's type mentions the named level variable — written here rather than reached
/// for, because the kernel's own `free_level_vars` is the thing under test elsewhere in this
/// file and a test that asks a function about itself proves nothing.
fn crate_mentions(t: &Term, level: &str) -> bool {
    fn walk(t: &Term, level: &str) -> bool {
        fn in_u(u: &Universe, level: &str) -> bool {
            match u {
                Universe::Var(v) => v == level,
                Universe::Succ(a) => in_u(a, level),
                Universe::Max(a, b) | Universe::IMax(a, b) => in_u(a, level) || in_u(b, level),
                _ => false,
            }
        }
        match t {
            Term::Sort(u) => in_u(u, level),
            Term::Const { levels, .. } => levels.iter().any(|u| in_u(u, level)),
            Term::App(f, a) => walk(f, level) || walk(a, level),
            Term::Pi {
                param_type,
                body_type,
                ..
            } => walk(param_type, level) || walk(body_type, level),
            Term::Lambda {
                param_type, body, ..
            } => walk(param_type, level) || walk(body, level),
            Term::Fix { body, .. } => walk(body, level),
            Term::Let {
                ty, value, body, ..
            } => walk(ty, level) || walk(value, level) || walk(body, level),
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                walk(discriminant, level)
                    || walk(motive, level)
                    || cases.iter().any(|c| walk(c, level))
            }
            _ => false,
        }
    }
    walk(t, level)
}

// ============================================================================================
// 19 · A BARE REFERENCE THAT IS NOT A HEAD STANDS AT ITS DECLARATION'S OWN LEVELS
// ============================================================================================

/// `QUnit : Sort u` with `qunit : QUnit`, through the plain entry points: the family's level is
/// auto-bound, and the constructor's type names the family bare.
fn register_qunit(ctx: &mut Context) {
    ctx.add_indexed_inductive("QUnit", sort(uvar("u")), 0);
    ctx.add_constructor("qunit", "QUnit", g("QUnit"));
}

#[test]
fn a_bare_reference_that_is_not_a_head_stands_at_its_declarations_own_levels() {
    let mut ctx = std_ctx();
    register_qunit(&mut ctx);
    let at = |levels: Vec<Universe>| konst("QUnit", levels);

    // STORED — the constructor's type is the family at its own level, so the constructor's
    // telescope is the family's and an instance of one moves the other.
    assert_eq!(
        ctx.get_global("qunit"),
        Some(&at(vec![uvar("u")])),
        "`qunit : QUnit` is stored as `qunit : QUnit.{{u}}`"
    );
    assert_eq!(
        infer_type(&ctx, &konst("qunit", vec![Universe::Type(0)])).expect("`qunit.{1}` types"),
        at(vec![Universe::Type(0)]),
        "`qunit.{{1}} : QUnit.{{1}}`"
    );

    // POSITIVE — the bare family IS the family at its own level, in both kernels.
    assert!(defeq_for_test(&ctx, &g("QUnit"), &at(vec![uvar("u")])));
    let at_own = app(lam("x", g("QUnit"), var("x")), g("qunit"));
    assert!(
        infer_type(&ctx, &at_own).is_ok(),
        "`(fun (x : QUnit) => x) qunit` types: {:?}",
        infer_type(&ctx, &at_own)
    );
    assert!(
        recheck(&ctx, &at_own).is_ok(),
        "and in the independent kernel: {:?}",
        recheck(&ctx, &at_own)
    );

    // NEGATIVE — and no other instance.
    for other in [
        vec![Universe::Prop],
        vec![Universe::Type(0)],
        vec![uvar("v")],
    ] {
        assert!(
            !defeq_for_test(&ctx, &g("QUnit"), &at(other.clone())),
            "the bare `QUnit` is not `QUnit.{{{other:?}}}`"
        );
        let elsewhere = app(lam("x", at(other.clone()), var("x")), g("qunit"));
        assert!(
            infer_type(&ctx, &elsewhere).is_err() && recheck(&ctx, &elsewhere).is_err(),
            "`qunit` is no element of `QUnit.{{{other:?}}}` in either kernel"
        );
    }
}

#[test]
fn a_bare_head_is_the_instance_its_arguments_solve_never_the_one_at_its_own_levels() {
    let mut ctx = std_ctx();
    register_box(&mut ctx);
    ctx.add_declaration("B", prop());
    let boxed = |levels: Vec<Universe>| app(konst("Box", levels), g("B"));

    // `B : Prop` solves `Box B` at `0`: the bare head is `Box.{0}`, never the declaration's own
    // `Box.{u}`, which is a different instance (`B` fits it by cumulativity).
    assert!(defeq_for_test(
        &ctx,
        &app(g("Box"), g("B")),
        &boxed(vec![Universe::Prop])
    ));
    assert!(
        !defeq_for_test(&ctx, &app(g("Box"), g("B")), &boxed(vec![uvar("u")])),
        "`Box B` is `Box.{{0}} B`, not `Box.{{u}} B`"
    );
    for (levels, converts) in [(vec![Universe::Prop], true), (vec![uvar("u")], false)] {
        let through = lam(
            "x",
            boxed(levels.clone()),
            app(lam("y", app(g("Box"), g("B")), var("y")), var("x")),
        );
        assert_eq!(
            infer_type(&ctx, &through).is_ok(),
            converts,
            "an element of `Box.{{{levels:?}}} B` passed where `Box B` is expected: {:?}",
            infer_type(&ctx, &through)
        );
        assert_eq!(
            recheck(&ctx, &through).is_ok(),
            converts,
            "and in the independent kernel: {:?}",
            recheck(&ctx, &through)
        );
    }
}

#[test]
fn a_definition_binds_the_levels_its_stored_body_mentions_after_its_types() {
    let mut ctx = std_ctx();
    register_qunit(&mut ctx);
    // `w : Sort v → Sort v`, whose body mentions `QUnit` at `u`: the telescope is the type's `v`,
    // then the body's `u`, and an instance moves both.
    let body = lam(
        "A",
        sort(uvar("v")),
        app(lam("x", g("QUnit"), var("A")), g("qunit")),
    );
    let ty_w = arrow(sort(uvar("v")), sort(uvar("v")));
    ctx.declare("w", ty_w, body).expect("`w` is admitted");
    assert_eq!(
        ctx.poly_declaration("w").map(|(ps, _)| ps.to_vec()),
        Some(vec!["v".to_string(), "u".to_string()]),
        "the type's level, then the one only the body mentions"
    );
    assert_eq!(
        infer_type(&ctx, &konst("w", vec![Universe::Type(0), Universe::Prop])).expect("types"),
        arrow(ty(0), ty(0)),
        "`w.{{1, 0}} : Type 0 → Type 0`"
    );
    // A stated telescope must bind them too: the body's `u` is not among `[v]`.
    let stated = ctx.declare_universe_poly(
        "w2",
        vec!["v".to_string()],
        arrow(sort(uvar("v")), sort(uvar("v"))),
        lam(
            "A",
            sort(uvar("v")),
            app(lam("x", g("QUnit"), var("A")), g("qunit")),
        ),
    );
    assert!(
        stated.is_err() && ctx.get_global("w2").is_none(),
        "a telescope that does not bind a level its stored body carries is refused: {stated:?}"
    );
}
