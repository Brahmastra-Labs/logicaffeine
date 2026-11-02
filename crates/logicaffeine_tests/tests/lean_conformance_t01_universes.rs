//! The Lean-4 conformance corpus, tier 1 — UNIVERSES.
//!
//! Fourteen cases covering the level layer: the `Prop`/`Type n` constructor split, the
//! `max`/`imax` algebra, level variables and where they may be BOUND, universe-polymorphic
//! definitions (which LOGOS has) versus universe-polymorphic INDUCTIVES (which it does not),
//! cumulativity, and the two sorts whose behaviour diverges from Lean's — `SProp`, which Lean
//! lacks entirely, and `Prop`, whose own sort LOGOS computes one too high.
//!
//! Two cases are decided elsewhere and are NOT reproduced here: T01-010 (`Prop : Type 1`, the
//! `Universe::succ` off-by-one) and T01-014 (the `is_subsingleton_prop` arity skip) are the
//! confirmed kernel defects, whose reproductions live in `lean_conformance_defects.rs`.
//!
//! The classification of each case — `MUST` (Lean can, we should), `DIVERGE` (we differ on
//! purpose), `OUT` (out of scope) — lives in `crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/t01_universes.json`.
//! RED here is the deliverable: a test that fails because we cannot yet do the thing names the
//! work. Every `DIVERGE` test carries its argument in a doc comment.

use std::collections::HashMap;

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, derive_recursor, derive_recursor_universe_poly, double_check, elaborate,
    infer_type, instantiate_universes, is_subtype, normalize, Context, DoubleCheck, MetaCtx, Term,
    Universe,
};

// ==== PER-FILE HELPERS (house style: every kernel test declares its own) ====

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
    xs.iter().fold(f, |acc, x| app(acc, x.clone()))
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
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn levels(pairs: &[(&str, Universe)]) -> HashMap<String, Universe> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

/// The sort a derived recursor's motive targets. `derive_recursor` names the motive binder
/// `P`; its type is `Π(indices…). I args… → <target>`, and `<target>` is what decides whether
/// the eliminator is a computation principle (`Type`) or only an induction principle (`Prop`).
fn motive_target(rec_ty: &Term) -> Term {
    let mut outer = rec_ty;
    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = outer
    {
        if param == "P" {
            let mut inner = param_type.as_ref();
            while let Term::Pi { body_type, .. } = inner {
                inner = body_type;
            }
            return inner.clone();
        }
        outer = body_type;
    }
    panic!("no motive binder `P` in the recursor type {rec_ty}");
}

/// `Box.{u} (α : Sort u) : Sort u` registered the only way LOGOS offers today:
/// `add_indexed_inductive` takes a FIXED `sort: Term`, so the level variable can be written
/// into the arity but there is no parameter list anywhere to bind it.
fn register_polymorphic_box(ctx: &mut Context) {
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

/// `Box0 : Type 0 → Type 0` and `Box1 : Type 1 → Type 1` — the two monomorphisations a
/// translator would emit for one Lean `Box.{u}`.
fn register_two_monomorphisations(ctx: &mut Context) {
    ctx.add_indexed_inductive("Box0", arrow(ty(0), ty(0)), 1);
    ctx.add_constructor(
        "Box0_mk",
        "Box0",
        pi("A", ty(0), arrow(var("A"), app(g("Box0"), var("A")))),
    );
    ctx.add_indexed_inductive("Box1", arrow(ty(1), ty(1)), 1);
    ctx.add_constructor(
        "Box1_mk",
        "Box1",
        pi("A", ty(1), arrow(var("A"), app(g("Box1"), var("A")))),
    );
}

/// `STwo : SProp` with TWO constructors — a multi-constructor proposition parked in the
/// definitionally-irrelevant bottom sort. `add_inductive` is unchecked, so this registers.
fn register_sprop_two(ctx: &mut Context) {
    ctx.add_inductive("STwo", sort(Universe::SProp));
    ctx.add_constructor("sfirst", "STwo", g("STwo"));
    ctx.add_constructor("ssecond", "STwo", g("STwo"));
}

/// `f : STwo → Nat` by LARGE ELIMINATION out of `STwo` — `f sfirst = 0`, `f ssecond = 1`.
fn sprop_discriminator() -> Term {
    lam(
        "s",
        g("STwo"),
        Term::Match {
            discriminant: Box::new(var("s")),
            motive: Box::new(lam("_", g("STwo"), g("Nat"))),
            cases: vec![g("Zero"), app(g("Succ"), g("Zero"))],
        },
    )
}

// ==== T01-001 · `Prop` ≡ `Sort 0` is its own constructor, never `Type(0)` ====

#[test]
fn prop_is_its_own_universe_constructor_not_type_zero() {
    let ctx = std_ctx();

    // The two are distinct ENUM CONSTRUCTORS, and the level algebra agrees they are distinct
    // LEVELS — `equiv` is mutual `level_leq`, where `Prop` is 0 and `Type 0` is 1.
    assert_ne!(
        Universe::Prop,
        Universe::Type(0),
        "`Prop` and `Type 0` must be different `Universe` constructors"
    );
    assert!(
        !Universe::Prop.equiv(&Universe::Type(0)),
        "`Prop` and `Type 0` must be different LEVELS, not merely different spellings"
    );

    // …and the one-directional subtyping check CANNOT see the difference: `Prop ≤ Type 0`
    // holds by cumulativity, so a harness that compares with `is_subtype_of` in one direction
    // concludes a `Prop ⟼ Type 0` mistranslation is harmless.
    assert!(
        Universe::Prop.is_subtype_of(&Universe::Type(0)),
        "cumulativity gives `Prop ≤ Type 0`"
    );
    assert!(
        !Universe::Type(0).is_subtype_of(&Universe::Prop),
        "`Type 0 ≤ Prop` must NOT hold — the direction that fails loudly"
    );

    // IMPREDICATIVITY is what rides on the constructor: `∀(A : Type 0). A → True` quantifies
    // over every type and is still a PROPOSITION, because `imax(_, Prop) = Prop`.
    let impredicative = pi("A", ty(0), pi("_", var("A"), g("True")));
    assert_eq!(
        infer_type(&ctx, &impredicative).expect("the impredicative ∀ type-checks"),
        prop(),
        "`∀(A : Type 0). A → True` must be a `Prop`, not a `Type`"
    );
    // One level up — so the collapse is `imax`'s doing and not an accident of `Type 0`.
    let impredicative_high = pi("A", ty(1), pi("_", var("A"), g("True")));
    assert_eq!(
        infer_type(&ctx, &impredicative_high).expect("the ∀ over `Type 1` type-checks"),
        prop(),
        "`∀(A : Type 1). A → True` must still be a `Prop`"
    );
    // The PREDICATIVE control: the same domain, a `Type`-valued codomain, no collapse.
    let predicative = pi("A", ty(0), g("Nat"));
    assert_eq!(
        infer_type(&ctx, &predicative).expect("the predicative ∀ type-checks"),
        ty(1),
        "`∀(A : Type 0). Nat` must land in `Type 1` — the control that distinguishes it"
    );

    // Two-kernel: the impredicative Π is re-derived by the independent re-checker.
    assert_eq!(
        double_check(&ctx, &impredicative),
        DoubleCheck::Agreed,
        "the impredicative ∀ must be two-kernel-verified, got {:?}",
        double_check(&ctx, &impredicative)
    );
}

#[test]
fn impredicativity_and_proof_irrelevance_ride_on_the_prop_constructor() {
    // The two consequences a `Prop ⟼ Type 0` mistranslation switches off SILENTLY: definitional
    // proof irrelevance, and the large-elimination gate.

    // (1) Proof irrelevance fires at `Prop`…
    let mut at_prop = std_ctx();
    at_prop.add_declaration("P", prop());
    at_prop.add_declaration("h1", g("P"));
    at_prop.add_declaration("h2", g("P"));
    assert!(
        defeq_for_test(&at_prop, &g("h1"), &g("h2")),
        "two proofs of the same proposition must be definitionally equal"
    );

    // …and NOT at `Type 0`, which is exactly what the mistranslation costs.
    let mut at_type = std_ctx();
    at_type.add_declaration("P", ty(0));
    at_type.add_declaration("h1", g("P"));
    at_type.add_declaration("h2", g("P"));
    assert!(
        !defeq_for_test(&at_type, &g("h1"), &g("h2")),
        "two inhabitants of a `Type 0` must NOT be definitionally equal"
    );

    // (2) The large-elimination gate fires on a MULTI-constructor proposition.
    let mut ctx = std_ctx();
    ctx.add_declaration("p", prop());
    ctx.add_declaration("q", prop());
    let or_pq = apps(g("Or"), &[g("p"), g("q")]);
    ctx.add_declaration("hor", or_pq.clone());
    let eliminate_or = Term::Match {
        discriminant: Box::new(g("hor")),
        motive: Box::new(lam("_", or_pq, g("Nat"))),
        cases: vec![lam("_", g("p"), g("Zero")), lam("_", g("q"), g("Zero"))],
    };
    assert!(
        infer_type(&ctx, &eliminate_or).is_err(),
        "eliminating `Or` into `Nat` extracts computation from a proof and must be refused"
    );

    // (3) …and `derive_recursor` branches on the same syntactic `Sort(Prop)` test: a
    // non-subsingleton proposition gets only its `Prop` induction principle.
    let (or_rec, _) = derive_recursor(&ctx, "Or").expect("`Or` has a derived recursor");
    assert_eq!(
        motive_target(&or_rec),
        prop(),
        "`Or`'s eliminator must target `Prop`, got {}",
        motive_target(&or_rec)
    );
    let (and_rec, _) = derive_recursor(&ctx, "And").expect("`And` has a derived recursor");
    assert_eq!(
        motive_target(&and_rec),
        ty(0),
        "`And` is a subsingleton, so its eliminator reaches `Type 0`, got {}",
        motive_target(&and_rec)
    );
}

// ==== T01-002 · `Type n` ≡ `Sort (n+1)`, and the index shift ====

#[test]
fn type_indices_agree_with_lean_and_the_successor_ladder_holds() {
    let ctx = std_ctx();

    // `Type n : Type (n+1)` — LOGOS's `Type` index is Lean's `Type` index, not Lean's `Sort`
    // index. An off-by-one anywhere in this ladder is a uniform shift of the whole development.
    for n in 0u32..4 {
        assert_eq!(
            infer_type(&ctx, &ty(n))
                .unwrap_or_else(|e| panic!("`Type {n}` must have a sort: {e:?}")),
            ty(n + 1),
            "`Type {n} : Type {}`",
            n + 1
        );
    }

    // `Universe::Type(n)` for distinct `n` are distinct LEVELS…
    assert!(
        !Universe::Type(0).equiv(&Universe::Type(1)),
        "`Type 0` and `Type 1` must be different levels"
    );
    // …but the cumulative order cannot tell an off-by-one from a correct level in the upward
    // direction, which is why the conformance comparison must be `equiv`, never `is_subtype_of`.
    assert!(
        Universe::Type(0).is_subtype_of(&Universe::Type(1)),
        "`Type 0 ≤ Type 1` — the check that CANNOT detect an off-by-one"
    );
    assert!(
        !Universe::Type(1).is_subtype_of(&Universe::Type(0)),
        "`Type 1 ≤ Type 0` must not hold"
    );

    // A function BETWEEN universes: `Type 1 → Type 0` has sort `imax(Type 2, Type 1) = Type 2`.
    assert_eq!(
        infer_type(&ctx, &arrow(ty(1), ty(0))).expect("`Type 1 → Type 0` has a sort"),
        ty(2),
        "the Π's sort is the max of the two operand sorts"
    );
    assert_eq!(
        double_check(&ctx, &arrow(ty(1), ty(0))),
        DoubleCheck::Agreed,
        "the level ladder must be two-kernel-verified"
    );
}

#[test]
fn cumulativity_forgives_an_off_by_one_at_the_top_and_not_one_constructor_deep() {
    // LOGOS's cumulativity is SHALLOW: it fires between two `Sort`s and covariantly under a Π
    // codomain, and NOWHERE else. Inside a type constructor's argument the comparison is
    // INVARIANT — which is why a uniformly-too-high level passes the simple tests and fails one
    // constructor deep.
    let mut ctx = std_ctx();
    // `Cont : Type 2 → Type 2` — big enough that BOTH `Type 0` and `Type 1` are legal arguments.
    ctx.add_indexed_inductive("Cont", arrow(ty(2), ty(2)), 1);

    let at0 = app(g("Cont"), ty(0));
    let at1 = app(g("Cont"), ty(1));
    assert!(
        infer_type(&ctx, &at0).is_ok(),
        "`Cont (Type 0)` is well-formed"
    );
    assert!(
        infer_type(&ctx, &at1).is_ok(),
        "`Cont (Type 1)` is well-formed"
    );

    // At the top: forgiven.
    assert!(
        is_subtype(&ctx, &ty(0), &ty(1)),
        "`Type 0 ≤ Type 1` at the top level"
    );
    // One constructor deep: invariant, in BOTH directions.
    assert!(
        !is_subtype(&ctx, &at0, &at1),
        "`Cont (Type 0)` and `Cont (Type 1)` must NOT be interchangeable — argument positions \
         are invariant"
    );
    assert!(
        !is_subtype(&ctx, &at1, &at0),
        "…and not in the other direction either"
    );
}

// ==== T01-003 · `Sort u` — a level VARIABLE, and where it may be bound ====

/// The level-variable ALGEBRA is sound and Lean-shaped; what is missing is a binding site on
/// anything but a definition. This test asserts the capability Lean has and we do not:
/// `axiom polyAxiom.{u} : Sort u → Prop`, referenced at a chosen level.
#[test]
fn a_universe_parameter_binds_on_an_axiom() {
    // The algebra, which IS right: a variable ranges over all of ℕ including `Prop`, so
    // `Prop ≤ u` holds, `Type 0 ≤ u` does NOT (it would fail at `u := Prop`), and `u ≤ v` for
    // distinct variables does not either.
    assert!(uvar("u").is_subtype_of(&uvar("u")), "`u ≤ u`");
    assert!(
        Universe::Prop.is_subtype_of(&uvar("u")),
        "`Prop ≤ u` — a variable may be `Prop`"
    );
    assert!(
        !Universe::Type(0).is_subtype_of(&uvar("u")),
        "`Type 0 ≤ u` must NOT hold — `u` may be instantiated at `Prop`"
    );
    assert!(
        !uvar("u").is_subtype_of(&uvar("v")),
        "`u ≤ v` for distinct variables must not hold"
    );
    assert!(uvar("u").is_subtype_of(&uvar("u").succ()), "`u ≤ u+1`");

    // THE GAP. `add_declaration` takes no universe-parameter list, so an axiom whose type
    // mentions `Sort u` carries a level variable with no binder. Lean writes this in one line
    // and instantiates it at every level.
    let mut ctx_ax = std_ctx();
    ctx_ax.add_declaration("polyAxiom", arrow(sort(uvar("u")), prop()));
    let at_type0 = konst("polyAxiom", vec![Universe::Type(0)]);
    assert!(
        infer_type(&ctx_ax, &at_type0).is_ok(),
        "`polyAxiom.{{0}}` must resolve — a universe parameter has to bind on an AXIOM too, not \
         only on a definition: {:?}",
        infer_type(&ctx_ax, &at_type0)
    );

    // …and the consequence of the missing binder, which is the quiet half: the free `Var("u")`
    // is not rejected, it is silently retained, and the axiom then applies at `Prop` (where
    // `Prop ≤ u` holds) and NOWHERE else. One declaration, one usable instantiation.
    assert!(
        infer_type(&ctx_ax, &app(g("polyAxiom"), g("Nat"))).is_ok(),
        "an axiom over `Sort u` must accept a `Type 0` argument at some instantiation: {:?}",
        infer_type(&ctx_ax, &app(g("polyAxiom"), g("Nat")))
    );
}

#[test]
fn level_variable_names_are_a_flat_namespace_across_declarations() {
    // `Universe::Var` holds a bare `String` and `equiv` compares through `to_lnat`, which
    // matches variables BY NAME. Two unrelated declarations that both call their parameter `u`
    // therefore have levels the kernel reports EQUAL. Nothing errors; constraints the source
    // imposed silently weaken. Any translator must freshen per declaration.
    assert!(
        uvar("u").equiv(&uvar("u")),
        "two separately-introduced `u`s are reported as the same level"
    );
    assert!(
        !uvar("u").equiv(&uvar("u_1")),
        "distinct names are distinct levels"
    );

    // And the substitution is silent about a variable it does not cover: an unmatched `Var` is
    // returned UNCHANGED rather than raising, so a level with no binder survives instantiation.
    let subst = levels(&[("u", Universe::Type(0))]);
    let unbound = sort(uvar("w"));
    assert_eq!(
        instantiate_universes(&unbound, &subst),
        unbound,
        "an unmatched level variable passes through instantiation untouched"
    );
}

// ==== T01-004 · `max u v` ====

#[test]
fn universe_max_is_decided_by_equiv_never_by_structural_equality() {
    let ctx = std_ctx();

    // `max(u, u)` stays SYMBOLIC — `Universe::max` collapses only CONCRETE operands — so it is
    // structurally different from `u` and `equiv` to it. Every level comparison in a translator,
    // a cache key or a conformance harness must go through `equiv`.
    let max_uu = uvar("u").max(&uvar("u"));
    assert_ne!(max_uu, uvar("u"), "`max(u,u)` is not STRUCTURALLY `u`");
    assert!(
        max_uu.equiv(&uvar("u")),
        "`max(u,u)` must be `equiv` to `u`"
    );

    // Associativity: decided equal, structurally different.
    let left = uvar("u").max(&uvar("v").max(&uvar("w")));
    let right = uvar("u").max(&uvar("v")).max(&uvar("w"));
    assert_ne!(
        left, right,
        "the two associations are structurally different"
    );
    assert!(left.equiv(&right), "`max u (max v w)` ≡ `max (max u v) w`");

    // `max(u, u+1) ≡ u+1` — the identity the doc names.
    assert!(
        uvar("u").max(&uvar("u").succ()).equiv(&uvar("u").succ()),
        "`max(u, u+1)` ≡ `u+1`"
    );

    // The smart constructor collapses what it can: concrete operands fold, and `Prop`/`SProp`
    // are absorbed. A hand-built `Max` reaches the kernel in a shape nothing else produces.
    assert_eq!(
        Universe::Type(0).max(&Universe::Type(2)),
        Universe::Type(2),
        "concrete max folds"
    );
    assert_eq!(
        Universe::Prop.max(&uvar("u")),
        uvar("u"),
        "`max(Prop, u) = u`"
    );
    assert_eq!(
        Universe::SProp.max(&Universe::Prop),
        Universe::Prop,
        "`max(SProp, Prop) = Prop`"
    );
    let hand_built = Universe::Max(Box::new(Universe::Type(0)), Box::new(Universe::Type(0)));
    assert_ne!(
        hand_built,
        Universe::Type(0),
        "a hand-built `Max` is not what `max` produces"
    );
    assert!(
        hand_built.equiv(&Universe::Type(0)),
        "…but it DECIDES equal, so nothing errors"
    );

    // Π formation is where a `max` arises without being written: `Type 0 → Type 1` is
    // `max(Type 1, Type 2) = Type 2`.
    assert_eq!(
        infer_type(&ctx, &arrow(ty(0), ty(1))).expect("`Type 0 → Type 1` has a sort"),
        ty(2),
        "Π formation computes the max of the two operand sorts"
    );
}

// ==== T01-005 · `imax u v` — impredicative Π formation ====

#[test]
fn imax_stays_symbolic_when_the_codomain_level_is_a_variable() {
    // `imax` resolves the cases whose Prop-ness is KNOWN and stays symbolic otherwise, because
    // whether `v` is `Prop` is not decidable before instantiation. Collapsing it to `max` makes
    // an impredicative ∀ predicative, and cumulativity then hides the mistake.
    assert_eq!(
        Universe::Type(0).imax(&Universe::Prop),
        Universe::Prop,
        "`imax(a, Prop) = Prop`"
    );
    assert_eq!(
        Universe::Type(0).imax(&Universe::Type(2)),
        Universe::Type(2),
        "`imax(a, Type n) = max(a, Type n)`"
    );
    assert_eq!(uvar("u").imax(&uvar("u")), uvar("u"), "`imax(a, a) = a`");
    assert_eq!(
        uvar("u").imax(&uvar("v")),
        Universe::IMax(Box::new(uvar("u")), Box::new(uvar("v"))),
        "`imax(u, v)` must stay SYMBOLIC — `v` may be `Prop`"
    );
    assert!(
        !uvar("u").imax(&uvar("v")).equiv(&uvar("u").max(&uvar("v"))),
        "`imax(u,v)` and `max(u,v)` must NOT be equal — they differ at `v := Prop`"
    );

    // The rewrites the decision core performs, checked by `equiv` rather than by shape:
    // `imax(a, imax(b,c)) = imax(max(a,b), c)`.
    assert!(
        uvar("u")
            .imax(&uvar("v").imax(&uvar("w")))
            .equiv(&uvar("u").max(&uvar("v")).imax(&uvar("w"))),
        "nested `imax` must decide equal to its pushed-down form"
    );

    // Π formation with two symbolic operands produces exactly that symbolic `IMax`.
    let mut ctx = std_ctx();
    ctx.add_declaration("A", sort(uvar("u")));
    ctx.add_declaration("B", sort(uvar("v")));
    assert_eq!(
        infer_type(&ctx, &arrow(g("A"), g("B"))).expect("`A → B` has a sort"),
        sort(Universe::IMax(Box::new(uvar("u")), Box::new(uvar("v")))),
        "`Π(_:A). B` must have sort `imax(u, v)`"
    );

    // The `SProp` arm — which no Lean level can reach, since Lean 4 has no `SProp`. It is ours,
    // and it is impredicative the same way `Prop` is.
    assert_eq!(
        Universe::Type(0).imax(&Universe::SProp),
        Universe::SProp,
        "`imax(a, SProp) = SProp` — LOGOS's own bottom sort is impredicative too"
    );

    // Both instantiations of one symbolic `imax`, in one context: into `Prop` it collapses,
    // into `Type` it does not.
    let ctx2 = std_ctx();
    assert_eq!(
        infer_type(&ctx2, &arrow(g("Nat"), g("True"))).expect("`Nat → True` has a sort"),
        prop(),
        "a Π into a proposition is a proposition"
    );
    assert_eq!(
        infer_type(&ctx2, &arrow(g("Nat"), g("Bool"))).expect("`Nat → Bool` has a sort"),
        ty(0),
        "a Π into a type is a type"
    );
}

// ==== T01-006 · universe-polymorphic DEFINITION — the construct LOGOS has ====

#[test]
fn a_universe_polymorphic_definition_serves_every_instantiation() {
    let mut ctx = std_ctx();
    // `polyId.{u} : Π(A : Sort u). A → A := λA. λa. a`
    ctx.add_universe_poly(
        "polyId",
        vec!["u".to_string()],
        pi("A", sort(uvar("u")), arrow(var("A"), var("A"))),
        lam("A", sort(uvar("u")), lam("a", var("A"), var("a"))),
    );

    // One declaration, three genuinely different instantiations — including `Prop`, which is
    // why the parameter is `Sort u` and not `Type u`.
    let at_prop = konst("polyId", vec![Universe::Prop]);
    let at_t0 = konst("polyId", vec![Universe::Type(0)]);
    let at_t1 = konst("polyId", vec![Universe::Type(1)]);
    assert!(
        infer_type(&ctx, &app(at_prop.clone(), g("True"))).is_ok(),
        "`polyId.{{Prop}} True` — the `Prop` instance"
    );
    assert!(
        infer_type(&ctx, &app(at_t0.clone(), g("Nat"))).is_ok(),
        "`polyId.{{0}} Nat` — the `Type 0` instance"
    );
    assert!(
        infer_type(&ctx, &app(at_t1.clone(), ty(0))).is_ok(),
        "`polyId.{{1}} (Type 0)` — the `Type 1` instance, applied to a type-of-types"
    );
    // The decisive contrast: the `Type 0` instance CANNOT do what the `Type 1` instance can.
    assert!(
        infer_type(&ctx, &app(at_t0.clone(), ty(0))).is_err(),
        "`polyId.{{0}}` must reject `Type 0` — the limitation polymorphism removes"
    );

    // It COMPUTES: a `Const` δ-unfolds with the levels substituted.
    assert_eq!(
        normalize(&ctx, &apps(at_t0.clone(), &[g("Nat"), g("Zero")])),
        g("Zero"),
        "`polyId.{{0}} Nat Zero` must reduce to `Zero`"
    );
    assert_eq!(
        double_check(&ctx, &at_t0),
        DoubleCheck::Agreed,
        "a universe-polymorphic reference must be two-kernel-verified"
    );

    // The level list is POSITIONAL. Used asymmetrically, a swap is detectable; the pairing of
    // this declaration with a symmetric one is what makes a swapped `params` catchable at all.
    ctx.add_universe_poly(
        "polySecond",
        vec!["u".to_string(), "v".to_string()],
        pi(
            "A",
            sort(uvar("u")),
            pi("B", sort(uvar("v")), sort(uvar("v"))),
        ),
        lam("A", sort(uvar("u")), lam("B", sort(uvar("v")), var("B"))),
    );
    let ordered = konst("polySecond", vec![Universe::Type(0), Universe::Type(1)]);
    let swapped = konst("polySecond", vec![Universe::Type(1), Universe::Type(0)]);
    assert_eq!(
        infer_type(&ctx, &apps(ordered, &[g("Nat"), ty(0)])).expect("the ordered instantiation"),
        ty(1),
        "`polySecond.{{0,1}} Nat (Type 0) : Type 1`"
    );
    assert!(
        infer_type(&ctx, &apps(swapped, &[g("Nat"), ty(0)])).is_err(),
        "a swapped level list must be caught by the asymmetric use"
    );

    // A universe-polymorphic definition lives in its OWN map: a `Global` naming one is unbound,
    // and a `Const` naming one at the wrong arity is refused. (Whether the REDUCER agrees with
    // that refusal is decided by
    // `lean_conformance_defects::reduction_and_typing_agree_about_a_level_arity_mismatch`.)
    assert!(
        infer_type(&ctx, &g("polyId")).is_err(),
        "`Term::Global` must not resolve a universe-polymorphic definition — the loud half"
    );
    assert!(
        infer_type(&ctx, &konst("polyId", vec![])).is_err(),
        "a `Const` with the wrong number of level arguments must not type-check"
    );
}

// ==== T01-007 · universe-polymorphic INDUCTIVE — the headline gap ====

/// THE case. Lean writes `inductive Box (α : Type u) : Type u` once and instantiates it at
/// every level; Mathlib does this on nearly every container. LOGOS's `universe_polys` map
/// stores `(params, ty, BODY)` — a body, so an inductive cannot live there — and
/// `add_inductive` / `add_indexed_inductive` take a single fixed `sort: Term` with no
/// parameter list. There is no representation at all.
#[test]
fn a_universe_polymorphic_container_survives_two_instantiations() {
    let mut ctx = std_ctx();
    register_polymorphic_box(&mut ctx);

    // The capability, in the order that matters. First: ONE declaration, referenced at a
    // chosen level — the thing `Term::Const` does for definitions and for nothing else.
    let box_at_0 = konst("Box", vec![Universe::Type(0)]);
    assert!(
        infer_type(&ctx, &box_at_0).is_ok(),
        "`Box.{{0}}` must resolve to the inductive at level 0: {:?}",
        infer_type(&ctx, &box_at_0)
    );

    // Second: the SAME declaration at a different level, in the same context — which is what
    // makes monomorphisation at one level unsound rather than merely lossy.
    let box_at_1 = konst("Box", vec![Universe::Type(1)]);
    assert!(
        infer_type(&ctx, &app(box_at_1, ty(0))).is_ok(),
        "`Box.{{1}} (Type 0)` must type-check against the SAME declaration"
    );
    assert!(
        infer_type(&ctx, &app(box_at_0, g("Nat"))).is_ok(),
        "`Box.{{0}} Nat` must type-check against the same declaration"
    );

    // Third: it is still an INDUCTIVE — constructors and a derived eliminator — which the
    // `add_universe_poly` escape gives up.
    assert!(
        !ctx.get_constructors("Box").is_empty(),
        "a universe-polymorphic inductive must still have constructors"
    );
    assert!(
        derive_recursor(&ctx, "Box").is_ok(),
        "a universe-polymorphic inductive must still have a derived eliminator"
    );
}

#[test]
fn monomorphising_an_inductive_cannot_hold_two_levels_at_once() {
    // The headline trap, both halves. Monomorphise LOW and the second instantiation fails
    // LOUDLY; monomorphise HIGH and cumulativity absorbs the first instantiation SILENTLY, so
    // two Lean types that were distinct collapse onto one LOGOS global.
    let mut ctx = std_ctx();
    register_two_monomorphisations(&mut ctx);

    assert!(
        infer_type(&ctx, &app(g("Box0"), ty(0))).is_err(),
        "`Box@0` applied at `Type 1` must fail — the loud, therefore safe, direction"
    );
    assert!(
        infer_type(&ctx, &app(g("Box1"), g("Nat"))).is_ok(),
        "`Box@1 Nat` type-checks by cumulativity — the SILENT collapse a translator must not \
         rely on"
    );

    // And the two instances are unrelated globals: nothing records that they were one Lean
    // declaration, so no lemma proved about one transfers to the other.
    assert!(
        !is_subtype(&ctx, &app(g("Box0"), g("Nat")), &app(g("Box1"), g("Nat"))),
        "`Box@0 Nat` and `Box@1 Nat` are unrelated types"
    );
    assert!(
        !is_subtype(&ctx, &app(g("Box1"), g("Nat")), &app(g("Box0"), g("Nat"))),
        "…in either direction"
    );
}

#[test]
fn an_inductive_registered_as_a_universe_polymorphic_definition_is_not_an_inductive() {
    // The second tempting escape: put the inductive in `universe_polys` so that `Term::Const`
    // resolves. It does resolve — and the thing that comes back is δ-TRANSPARENT, has no
    // constructors, and has no eliminator. It is not an inductive; it is an abbreviation.
    let mut ctx = std_ctx();
    ctx.add_universe_poly(
        "BoxP",
        vec!["u".to_string()],
        arrow(sort(uvar("u")), sort(uvar("u"))),
        lam("A", sort(uvar("u")), var("A")),
    );
    let at0 = konst("BoxP", vec![Universe::Type(0)]);
    assert!(
        infer_type(&ctx, &app(at0.clone(), g("Nat"))).is_ok(),
        "`BoxP.{{0}} Nat` type-checks"
    );
    assert_eq!(
        normalize(&ctx, &app(at0, g("Nat"))),
        g("Nat"),
        "…and unfolds straight to its argument: it is transparent, so it cannot be matched on"
    );
    assert!(
        ctx.get_global("BoxP").is_none(),
        "`get_global` does not see a universe-poly name"
    );
    assert!(
        ctx.get_constructors("BoxP").is_empty(),
        "it has no constructors"
    );
    assert!(
        derive_recursor(&ctx, "BoxP").is_err(),
        "and no derived eliminator"
    );

    // Worse: a constructor attached to it is accepted by BOTH registration paths, including the
    // checked one, because `check_constructor_universes` bails out when the inductive is not a
    // global — so the ill-formed constructor type is never inferred at all.
    let ctor_ty = pi("A", ty(0), arrow(var("A"), app(g("BoxP"), var("A"))));
    assert!(
        infer_type(&ctx, &ctor_ty).is_err(),
        "the constructor's own type does not type-check — `Global(\"BoxP\")` is unbound"
    );
    assert!(
        ctx.add_constructor_checked("BoxP_mk", "BoxP", ctor_ty)
            .is_err(),
        "…so the CHECKED registration path must refuse it, rather than admitting a constructor \
         for a type that does not exist"
    );
}

#[test]
fn an_inductive_over_sort_u_accepts_an_element_type_at_any_level() {
    // The third escape: write the level variable straight into the inductive's arity. It
    // registers — `add_inductive` is unchecked — and then it is instantiable at exactly ONE
    // level, because `Prop ≤ u` holds and `Type n ≤ u` does not. One declaration, one usable
    // instance, and no `Const` form to pick which.
    let mut ctx = std_ctx();
    register_polymorphic_box(&mut ctx);

    assert!(
        infer_type(&ctx, &app(g("Box"), g("True"))).is_ok(),
        "`Box True` works — `Prop ≤ u`"
    );
    assert!(
        infer_type(&ctx, &app(g("Box"), g("Nat"))).is_ok(),
        "`Box Nat` must work too — a container over `Sort u` has to accept a `Type 0` element \
         type: {:?}",
        infer_type(&ctx, &app(g("Box"), g("Nat")))
    );
}

// ==== T01-008 · per-instantiation monomorphisation, and its two losses ====

/// Even after monomorphising, the derived eliminator is pinned: `derive_recursor` hard-codes
/// the motive target to `Sort(Type 0)` for every non-`Prop` inductive. So `Box@1`'s ELEMENTS
/// live at `Type 1` while its recursor eliminates only into `Type 0`, and a Lean proof that
/// recurses over a `Type 1` container into a `Type 1` motive has no image.
#[test]
fn a_monomorphised_instance_keeps_an_eliminator_at_its_own_level() {
    let mut ctx = std_ctx();
    register_two_monomorphisations(&mut ctx);

    let (rec0, _) = derive_recursor(&ctx, "Box0").expect("`Box@0` has a derived recursor");
    assert_eq!(
        motive_target(&rec0),
        ty(0),
        "`Box@0`'s eliminator targets `Type 0`"
    );

    let (rec1, _) = derive_recursor(&ctx, "Box1").expect("`Box@1` has a derived recursor");
    assert_eq!(
        motive_target(&rec1),
        ty(1),
        "`Box@1` lives at `Type 1`, so its eliminator must reach `Type 1` — today it is pinned \
         to `Type 0`, which is the monomorphisation escape failing at the level it was meant to \
         recover: {}",
        motive_target(&rec1)
    );
}

#[test]
fn the_match_primitive_already_eliminates_a_type_one_inductive_into_type_one() {
    // The ceiling above is on the DERIVED recursor, not on elimination itself: `Match` is
    // LOGOS's primitive and it carries its own motive, so case analysis on a `Type 1` inductive
    // into a `Type 1` motive already works. The gap is the auto-derived `.rec`, not the theory.
    let mut ctx = std_ctx();
    register_two_monomorphisations(&mut ctx);
    ctx.add_declaration("b", app(g("Box1"), ty(0)));
    let unbox = Term::Match {
        discriminant: Box::new(g("b")),
        motive: Box::new(lam("_", app(g("Box1"), ty(0)), ty(1))),
        cases: vec![lam("a", ty(0), var("a"))],
    };
    assert_eq!(
        infer_type(&ctx, &unbox).expect("the `Type 1` elimination type-checks"),
        ty(1),
        "a `Match` on a `Type 1` inductive may target a `Type 1` motive"
    );
    assert_eq!(
        double_check(&ctx, &unbox),
        DoubleCheck::Agreed,
        "…and both kernels agree, got {:?}",
        double_check(&ctx, &unbox)
    );
}

// ==== T01-009 · cumulativity — DIVERGE ====

/// **DIVERGE.** LOGOS is CUMULATIVE — `SProp ≤ Prop ≤ Type i ≤ Type j`, plus contravariance in
/// a Π's domain and covariance in its codomain — where Lean 4 has no cumulativity at all and
/// ships `ULift`/`PLift` to move between levels by hand. Ours is the Coq/Matita/Lean-3 rule: a
/// strictly larger typing relation, sound (it is the standard PTS cumulative CIC), and it
/// removes lifting bureaucracy from ordinary mathematics. Crucially it is SHALLOW — it fires
/// only between two `Sort`s and under a Π codomain; every other position, including a type
/// constructor's argument, is invariant — so the type distinctions Lean uses `ULift` to
/// express are still expressible and still enforced exactly where they carry content. The
/// honest cost, asserted below, is auditing precision: a declaration stated one level too high
/// is accepted rather than refused, so LOGOS's acceptance of a term is never by itself evidence
/// that the term's levels are the ones the author meant.
#[test]
fn cumulativity_is_coq_style_shallow_and_the_lift_distinction_survives() {
    let mut ctx = std_ctx();

    // The universe arm, in both directions.
    assert!(is_subtype(&ctx, &prop(), &ty(0)), "`Prop ≤ Type 0`");
    assert!(is_subtype(&ctx, &ty(0), &ty(1)), "`Type 0 ≤ Type 1`");
    assert!(
        !is_subtype(&ctx, &ty(1), &ty(0)),
        "`Type 1 ≤ Type 0` must not hold"
    );

    // The Π arm — contravariant in the domain, covariant in the codomain. Lean has neither.
    assert!(
        is_subtype(&ctx, &arrow(ty(1), g("Nat")), &arrow(ty(0), g("Nat"))),
        "`Type 1 → Nat ≤ Type 0 → Nat` (contravariance)"
    );
    assert!(
        is_subtype(&ctx, &arrow(g("Nat"), ty(0)), &arrow(g("Nat"), ty(1))),
        "`Nat → Type 0 ≤ Nat → Type 1` (covariance)"
    );

    // What that buys: a `Type 1`-domained function applies to a `Type 0` type with no lift, and
    // a proposition may be used where a type is wanted. Both are Lean hard errors.
    ctx.add_declaration("big", arrow(ty(1), ty(1)));
    assert!(
        infer_type(&ctx, &app(g("big"), g("Nat"))).is_ok(),
        "`big Nat` needs no `ULift` in LOGOS"
    );
    assert!(
        infer_type(&ctx, &app(g("TList"), g("True"))).is_ok(),
        "a proposition may inhabit a `Type 0` argument position — `Prop ≤ Type 0`"
    );

    // And what it does NOT cost: a genuine lift is still a genuine, distinct type. `ULift`
    // wraps a `Type 0` carrier into a `Type 1` record, and the wrapper is not interchangeable
    // with its carrier in either direction — so every Lean lemma about `ULift.up`/`.down` has
    // a counterpart here.
    ctx.add_indexed_inductive("ULift", arrow(ty(0), ty(1)), 1);
    ctx.add_constructor(
        "ULift_up",
        "ULift",
        pi("A", ty(0), arrow(var("A"), app(g("ULift"), var("A")))),
    );
    let lifted = app(g("ULift"), g("Nat"));
    assert_eq!(
        infer_type(&ctx, &lifted).expect("`ULift Nat` is well-formed"),
        ty(1)
    );
    assert!(
        !is_subtype(&ctx, &lifted, &g("Nat")),
        "`ULift Nat` is not `Nat`"
    );
    assert!(
        !is_subtype(&ctx, &g("Nat"), &lifted),
        "…and `Nat` is not `ULift Nat`"
    );

    // …and one constructor deep the two systems agree exactly: `Cont (Type 0)` and
    // `Cont (Type 1)` are distinct in LOGOS too.
    ctx.add_indexed_inductive("Cont", arrow(ty(2), ty(2)), 1);
    assert!(
        !is_subtype(&ctx, &app(g("Cont"), ty(0)), &app(g("Cont"), ty(1))),
        "cumulativity is shallow: a constructor argument is invariant"
    );

    // The cost, stated as a fact rather than a feeling: a body at `Type 0` satisfies a type
    // stated at `Type 1`, so a level that is uniformly one too high is never rejected.
    assert!(
        is_subtype(&ctx, &infer_type(&ctx, &g("Nat")).unwrap(), &ty(1)),
        "`Nat`, whose sort is `Type 0`, checks against a declared `Type 1`"
    );
}

// ==== T01-011 · auto-bound universes — a parameter on a NON-definition ====

/// Lean auto-binds a universe parameter on any declaration kind, `structure` included, and
/// Mathlib's `Type*` is exactly this. `add_structure` binds the level variables its parameter and
/// field types mention, in order — Lean's auto-bound rule — so `structure Cell.{u} (α : Sort u)`
/// is ONE declaration referenced at any level, and a structure written over a CONCRETE sort has
/// no level parameter, exactly as in Lean. Its sort is the least one holding its fields, checked
/// transactionally, so a record can no longer be registered universe-inconsistent.
#[test]
fn a_universe_parameter_binds_on_a_structure() {
    let mut ctx = std_ctx();

    // The capability: `structure Cell.{u} (α : Sort u)`, referenced at a level.
    ctx.add_structure(
        "Cell",
        &[("A", Term::Sort(Universe::Var("u".to_string())))],
        &[("val", var("A"))],
    );
    assert_eq!(
        ctx.poly_declaration("Cell")
            .map(|(levels, _)| levels.to_vec()),
        Some(vec!["u".to_string()]),
        "the structure carries exactly the level its parameter was written over"
    );
    let cell_at_1 = konst("Cell", vec![Universe::Type(1)]);
    assert!(
        infer_type(&ctx, &cell_at_1).is_ok(),
        "`Cell.{{1}}` must resolve — a structure has to carry its own universe parameter: {:?}",
        infer_type(&ctx, &cell_at_1)
    );
    let big = app(cell_at_1.clone(), ty(0));
    assert!(
        is_subtype(
            &ctx,
            &infer_type(&ctx, &big).expect("`Cell.{1} (Type 0)` is a type"),
            &ty(1)
        ),
        "`Cell.{{1}}` stores a `Type 0` — a `Type 1` value — in a `Type 1` record"
    );
    let stored = apps(
        konst("Cell_mk", vec![Universe::Type(1)]),
        &[ty(0), g("Nat")],
    );
    assert_eq!(
        normalize(
            &ctx,
            &apps(konst("Cell_val", vec![Universe::Type(1)]), &[ty(0), stored])
        ),
        g("Nat"),
        "…and its projection computes at that level"
    );

    // Written over a CONCRETE sort, a structure has no level parameter to instantiate — Lean
    // refuses `Fixed.{1}` for the same reason.
    ctx.add_structure("Fixed", &[("A", ty(0))], &[("val", var("A"))]);
    assert!(
        infer_type(&ctx, &konst("Fixed", vec![Universe::Type(1)])).is_err(),
        "a monomorphic structure has no level to instantiate"
    );

    // The hazard of the old workaround — writing the higher level into the PARAMETER — is gone:
    // the record's sort is inferred from what it stores, so the sort `add_structure` chose admits
    // its field under the checked constructor gate.
    let mut bad = std_ctx();
    bad.add_structure("BigCell", &[("A", ty(1))], &[("val", var("A"))]);
    assert!(
        bad.refused_declarations().is_empty(),
        "{:?}",
        bad.refused_declarations()
    );
    let sort = bad
        .get_global("BigCell")
        .cloned()
        .expect("BigCell is registered");
    let mut copy = std_ctx();
    copy.add_inductive("BigCellCopy", sort.clone());
    let stored_ctor = pi(
        "A",
        ty(1),
        pi("val", var("A"), app(g("BigCellCopy"), var("A"))),
    );
    assert!(
        copy.add_constructor_checked("BigCellCopy_mk", "BigCellCopy", stored_ctor)
            .is_ok(),
        "the sort `add_structure` inferred for a record storing a `Type 1` field must pass the \
         checked universe gate: {sort}"
    );
}

#[test]
fn the_level_list_is_positional_so_an_asymmetric_use_detects_a_swap() {
    // Lean's `ConstantInfo.levelParams` is an ORDERED list and LOGOS zips it positionally
    // against a reference's `levels`. A swap is undetectable wherever the level expression is
    // symmetric and fatal wherever it is not, so the two shapes must always be checked as a
    // pair — the symmetric one to show the swap is invisible, the asymmetric one to catch it.
    let mut ctx = std_ctx();
    // Symmetric: `max u v`. Both orders accept the same arguments.
    ctx.add_universe_poly(
        "polyArrow",
        vec!["u".to_string(), "v".to_string()],
        pi(
            "A",
            sort(uvar("u")),
            pi("B", sort(uvar("v")), sort(uvar("u").max(&uvar("v")))),
        ),
        lam(
            "A",
            sort(uvar("u")),
            lam("B", sort(uvar("v")), arrow(var("A"), var("B"))),
        ),
    );
    let fwd = konst("polyArrow", vec![Universe::Type(0), Universe::Type(1)]);
    let rev = konst("polyArrow", vec![Universe::Type(1), Universe::Type(0)]);
    assert!(
        infer_type(&ctx, &apps(fwd, &[g("Nat"), ty(0)])).is_ok(),
        "`polyArrow.{{0,1}} Nat (Type 0)` type-checks"
    );
    assert!(
        infer_type(&ctx, &apps(rev, &[ty(0), g("Nat")])).is_ok(),
        "…and so does the mirrored application: a symmetric `max` hides the order entirely"
    );

    // Asymmetric: only `v` appears in the result, so the order becomes observable.
    ctx.add_universe_poly(
        "polySecond",
        vec!["u".to_string(), "v".to_string()],
        pi(
            "A",
            sort(uvar("u")),
            pi("B", sort(uvar("v")), sort(uvar("v"))),
        ),
        lam("A", sort(uvar("u")), lam("B", sort(uvar("v")), var("B"))),
    );
    let ok = konst("polySecond", vec![Universe::Type(0), Universe::Type(1)]);
    let swapped = konst("polySecond", vec![Universe::Type(1), Universe::Type(0)]);
    assert_eq!(
        infer_type(&ctx, &apps(ok, &[g("Nat"), ty(0)])).expect("the ordered instantiation"),
        ty(1),
        "the asymmetric declaration reports the SECOND level"
    );
    assert!(
        infer_type(&ctx, &apps(swapped, &[g("Nat"), ty(0)])).is_err(),
        "a swapped level list must be caught where the level expression is asymmetric"
    );
}

// ==== T01-012 · level metavariables — no representation, and no solver ====

/// `Universe` has `SProp`, `Prop`, `Type(u32)`, `Var`, `Succ`, `Max` and `IMax` — and no
/// metavariable form, no wildcard, and no constraint store. Two things follow, and both are
/// Lean capabilities we owe: an unresolved level must be REFUSED rather than silently retained
/// as a free `Var`, and a level the author did not write must be SOLVED rather than demanded.
#[test]
fn an_unbound_level_is_refused_rather_than_silently_retained() {
    let mut ctx = std_ctx();
    // A declaration whose type mentions `w` while its parameter list is empty. Nothing binds
    // `w`; nothing ever can. Today this registers and type-checks, carrying a level variable
    // that no reference can instantiate.
    ctx.add_universe_poly(
        "unbound",
        vec![],
        pi("A", sort(uvar("w")), var("A")),
        lam("A", sort(uvar("w")), var("A")),
    );
    assert!(
        infer_type(&ctx, &konst("unbound", vec![])).is_err(),
        "a declaration carrying a level variable that is not among its parameters must be \
         refused, not registered with a free level: {:?}",
        infer_type(&ctx, &konst("unbound", vec![]))
    );
}

#[test]
fn an_unwritten_level_is_solved_by_elaboration() {
    // Lean's elaborator solves `Type _` / `Sort _` by unification and only auto-binds what it
    // cannot solve; a Mathlib file writes a level explicitly almost never. LOGOS's `MetaCtx`
    // holds `String -> Term` solutions — TERM metavariables only — so today there is no level
    // to solve with: every reference to a universe-polymorphic definition has to spell its
    // levels out by hand, and one that does not is simply unbound.
    let mut ctx = std_ctx();
    ctx.add_universe_poly(
        "polyId",
        vec!["u".to_string()],
        pi("A", sort(uvar("u")), arrow(var("A"), var("A"))),
        lam("A", sort(uvar("u")), lam("a", var("A"), var("a"))),
    );
    let unwritten = apps(g("polyId"), &[g("Nat"), g("Zero")]);
    assert!(
        elaborate(&ctx, &mut MetaCtx::new(), &unwritten, None).is_ok(),
        "`polyId Nat Zero`, with the level left unwritten, must elaborate — the level is \
         determined by the argument: {:?}",
        elaborate(&ctx, &mut MetaCtx::new(), &unwritten, None).err()
    );
}

// ==== T01-013 · `SProp` — ours, not Lean's, and not yet safe ====

/// The corpus report calls `SProp` vestigial. It is not: it is a real bottom sort with a
/// complete algebra, definitional irrelevance in BOTH kernels, and its own test file. Lean 4
/// has nothing like it. This test pins what we genuinely have.
#[test]
fn sprop_is_a_real_definitionally_irrelevant_bottom_sort() {
    let mut ctx = std_ctx();

    // The lattice: `SProp ≤ Prop ≤ Type n`, and none of the converses.
    assert!(
        Universe::SProp.is_subtype_of(&Universe::Prop),
        "`SProp ≤ Prop`"
    );
    assert!(
        Universe::Prop.is_subtype_of(&Universe::Type(0)),
        "`Prop ≤ Type 0`"
    );
    assert!(
        !Universe::Prop.is_subtype_of(&Universe::SProp),
        "`Prop ≤ SProp` must not hold"
    );
    assert!(
        !Universe::Type(0).is_subtype_of(&Universe::SProp),
        "`Type 0 ≤ SProp` must not hold"
    );

    // Impredicative: a Π into `SProp` is an `SProp`.
    ctx.add_declaration("S", sort(Universe::SProp));
    assert_eq!(
        infer_type(&ctx, &arrow(g("Nat"), g("S"))).expect("`Nat → S` has a sort"),
        sort(Universe::SProp),
        "a Π into `SProp` is impredicatively `SProp`"
    );

    // Definitional irrelevance, and it is a TWO-KERNEL property: the re-checker has its own
    // irrelevance rule, so a term well-typed only by `SProp` conversion is accepted twice.
    ctx.add_declaration("a", g("S"));
    ctx.add_declaration("b", g("S"));
    assert!(
        defeq_for_test(&ctx, &g("a"), &g("b")),
        "any two `SProp` inhabitants are equal"
    );
    ctx.add_declaration("Vec", arrow(g("S"), ty(0)));
    ctx.add_declaration("v", app(g("Vec"), g("b")));
    let coerce = app(lam("x", app(g("Vec"), g("a")), var("x")), g("v"));
    assert_eq!(
        infer_type(&ctx, &coerce).expect("the `SProp`-driven coercion type-checks"),
        app(g("Vec"), g("a")),
        "`Vec a` and `Vec b` are convertible because `a ≡ b`"
    );
    assert_eq!(
        double_check(&ctx, &coerce),
        DoubleCheck::Agreed,
        "`SProp` irrelevance must hold in both kernels, got {:?}",
        double_check(&ctx, &coerce)
    );
}

#[test]
fn an_sprop_inductive_must_obey_the_large_elimination_gate() {
    // The gate at the `Match` rule tests the discriminant's sort for `Sort(Universe::Prop)`
    // SYNTACTICALLY. `Sort(Universe::SProp)` does not match, so a multi-constructor inductive
    // parked in `SProp` may be eliminated into `Type` with no restriction whatsoever — while
    // `proof_irrelevant`, which DOES match `SProp`, makes its two constructors definitionally
    // equal. The gate must cover both sorts.
    let mut ctx = std_ctx();
    register_sprop_two(&mut ctx);
    assert!(
        defeq_for_test(&ctx, &g("sfirst"), &g("ssecond")),
        "`SProp` irrelevance identifies the two constructors"
    );
    assert!(
        infer_type(&ctx, &sprop_discriminator()).is_err(),
        "large elimination out of a two-constructor `SProp` inductive must be refused, exactly \
         as it is for `Prop`: {:?}",
        infer_type(&ctx, &sprop_discriminator()).map(|t| t.to_string())
    );
}

#[test]
fn sprop_large_elimination_must_not_derive_zero_equals_one() {
    // The gate hole above is not a precision loss — it is an INCONSISTENCY, and this is the
    // whole derivation, in six steps, all of them accepted by both kernels today:
    //   1. `STwo : SProp` with two constructors            (`add_inductive` is unchecked)
    //   2. `f : STwo → Nat`, `f sfirst = 0`, `f ssecond = 1` (the ungated large elimination)
    //   3. `sfirst ≡ ssecond`                                (SProp definitional irrelevance)
    //   4. `refl STwo sfirst : Eq STwo sfirst ssecond`       (…so `refl` retypes across them)
    //   5. transport along that equation with `Eq`'s J
    //   6. yields `Eq Nat (f sfirst) (f ssecond)` = `Eq Nat Zero (Succ Zero)`
    let mut ctx = std_ctx();
    register_sprop_two(&mut ctx);
    let f = sprop_discriminator();
    ctx.add_definition("f".to_string(), arrow(g("STwo"), g("Nat")), f);
    let (eq_rec_ty, eq_rec_tm) = derive_recursor(&ctx, "Eq").expect("`Eq`'s J is derivable");
    ctx.add_definition("Eq_rec".to_string(), eq_rec_ty, eq_rec_tm);

    let refl_first = apps(g("refl"), &[g("STwo"), g("sfirst")]);
    let motive = lam(
        "y",
        g("STwo"),
        lam(
            "_h",
            apps(g("Eq"), &[g("STwo"), g("sfirst"), var("y")]),
            apps(
                g("Eq"),
                &[g("Nat"), app(g("f"), g("sfirst")), app(g("f"), var("y"))],
            ),
        ),
    );
    let base = apps(g("refl"), &[g("Nat"), app(g("f"), g("sfirst"))]);
    let transport = apps(
        g("Eq_rec"),
        &[
            g("STwo"),
            g("sfirst"),
            motive,
            base,
            g("ssecond"),
            refl_first,
        ],
    );
    let zero_equals_one = apps(g("Eq"), &[g("Nat"), g("Zero"), app(g("Succ"), g("Zero"))]);

    match infer_type(&ctx, &transport) {
        Err(_) => {}
        Ok(derived) => assert!(
            !is_subtype(&ctx, &derived, &zero_equals_one),
            "`0 = 1` must not be derivable: a two-constructor `SProp` inductive large-eliminates \
             into `Nat` while its constructors are definitionally equal, and transporting along \
             that equality proves {} — and the second kernel agrees ({:?})",
            normalize(&ctx, &derived),
            double_check(&ctx, &transport)
        ),
    }
}

// ==== T01-014 · large elimination from `Prop` — the LEVEL CEILING ====

/// Lean's `False.elim`, `Eq.rec`, `And.rec` and `Acc.rec` are universe-POLYMORPHIC: they
/// eliminate into `Sort u` for any `u`. LOGOS agrees with Lean about WHICH propositions may be
/// large-eliminated — `False` (no constructors) and `And`/`Eq` (one constructor, propositional
/// fields) yes, `Or` (two constructors) no — and `derive_recursor_universe_poly` gives each
/// admitted eliminator a motive LEVEL of its own, so every instance above `Type 0` has its image.
///
/// (The other direction — LOGOS ADMITTING a large elimination Lean denies, the
/// `is_subsingleton_prop` arity skip — is a confirmed kernel defect, reproduced by
/// `lean_conformance_defects::large_elimination_admits_exactly_the_propositional_and_indexed_constructor_fields`.)
#[test]
fn a_prop_eliminator_reaches_every_level_lean_reaches() {
    let mut ctx = std_ctx();

    // The agreement first, so the ceiling is not mistaken for a disagreement about the RULE:
    // `ex falso` out of the empty proposition is legal, at `Type 0`…
    ctx.add_declaration("hf", g("False"));
    let ex_falso_at_0 = Term::Match {
        discriminant: Box::new(g("hf")),
        motive: Box::new(lam("_", g("False"), ty(0))),
        cases: vec![],
    };
    assert_eq!(
        infer_type(&ctx, &ex_falso_at_0).expect("`ex falso` into `Type 0` type-checks"),
        ty(0),
        "`False` has no constructors, so it eliminates into anything"
    );
    // …and at `Type 1` too, because `Match` carries its own motive: the primitive is not the
    // limit here, which is what makes the ceiling below specifically the DERIVED eliminator's.
    let ex_falso_at_1 = Term::Match {
        discriminant: Box::new(g("hf")),
        motive: Box::new(lam("_", g("False"), ty(1))),
        cases: vec![],
    };
    assert_eq!(
        infer_type(&ctx, &ex_falso_at_1).expect("`ex falso` into `Type 1` type-checks"),
        ty(1),
        "a `Match` on `False` may target a `Type 1` motive"
    );

    // THE CEILING, lifted. Lean's `False.elim`, `And.rec` and `Eq.rec` each take the motive's sort
    // as a universe PARAMETER, and so does `derive_recursor_universe_poly`: the motive lands in a
    // level variable of the eliminator's own telescope, instantiable at `Type 1` and above. (The
    // monomorphic `derive_recursor` still targets the inductive's own sort, by design.)
    for ind in ["False", "And", "Eq"] {
        let (telescope, rec_ty, rec_term) = derive_recursor_universe_poly(&ctx, ind)
            .unwrap_or_else(|e| panic!("`{ind}` has a polymorphic eliminator: {e:?}"));
        assert_eq!(
            double_check(&ctx, &rec_term),
            DoubleCheck::Agreed,
            "both kernels certify `{ind}`'s polymorphic eliminator"
        );
        let level = match motive_target(&rec_ty) {
            Term::Sort(Universe::Var(level)) => level,
            other => panic!("`{ind}`'s motive must land in a level variable, got {other}"),
        };
        assert!(
            telescope.contains(&level),
            "`{ind}`'s motive level `{level}` is a parameter of the eliminator: {telescope:?}"
        );
        let at_type_one =
            instantiate_universes(&rec_ty, &levels(&[(level.as_str(), Universe::Type(1))]));
        let target = motive_target(&at_type_one);
        assert!(
            is_subtype(&ctx, &ty(1), &target),
            "`{ind}`'s eliminator must reach at least `Type 1`, as Lean's `{ind}.rec` does: {target}"
        );
    }

    // …and the large-elimination GATE is not parameterised away: `Or` has two constructors, so
    // its eliminator's motive stays in `Prop` at every level, exactly as Lean's `Or.rec` does.
    let (or_levels, or_ty, _) =
        derive_recursor_universe_poly(&ctx, "Or").expect("`Or` has an eliminator");
    assert_eq!(
        motive_target(&or_ty),
        prop(),
        "`Or` eliminates only into `Prop`"
    );
    assert!(
        or_levels.is_empty(),
        "…with no motive level to instantiate: {or_levels:?}"
    );
}
