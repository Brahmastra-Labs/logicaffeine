//! The Lean-4 conformance corpus — tier 5, the ELABORATION SURFACE.
//!
//! Twenty cases covering everything that stands between what a mathematician writes and the
//! fully-explicit term the kernel certifies: implicit / strict-implicit / instance binders,
//! `@`-application, named and default and tactic-filled arguments, auto-bound implicits,
//! `⟨…⟩`, dot notation, coercions (`Coe`, `CoeFun`, `CoeSort`), `_` holes, higher-order
//! pattern unification, elaboration postponement, `..`, `OfNat` literals, type ascription,
//! and `do`/`IO`.
//!
//! Each `#[test]` decides exactly one corpus case and is named for the BEHAVIOUR the case
//! demands, not for the Lean keyword. The verdicts live in
//! `crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/t05_elaboration.json`; `lean_parity_matrix.rs` pins the two
//! together. A test that fails here is a capability Lean has and we do not — the work queue.
//!
//! Seven cases are already green, and they are the tier's floor: implicit binders (including
//! interleaved and dependent ones), instance implicits (including recursive instances and
//! expected-type-driven resolution), `@`-application, anonymous constructors, dot notation,
//! `_` holes, and Miller-pattern / η unification.

use logicaffeine_kernel::interface::{Repl, TermParser};
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, elaborate_anon_ctor, elaborate_app, elaborate_app_against, elaborate_dot,
    infer_type, normalize, surface_elaborate, surface_elaborate_against, unify, unify_in, Context,
    KernelError, Literal, MetaCtx, ParamKind, Term, Universe,
};

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
fn pi(p: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn lam(p: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn nat(n: i64) -> Term {
    Term::Lit(Literal::Nat(n.into()))
}
fn int(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// Whether `t` still carries an elaborator METAVARIABLE — a `Term::Var` whose name begins
/// with `?`. The kernel has no idea these are special (it treats them as ordinary free
/// variables), so one reaching a certified term is the corpus's recurring failure mode.
fn has_meta(t: &Term) -> bool {
    walk(t, &|x| matches!(x, Term::Var(n) if n.starts_with('?')))
}

/// Whether `t` still carries a `Term::Hole`. `types_equal` matches a `Hole` against
/// anything, so a hole surviving into a declaration's type makes it typecheck against any
/// type at all.
fn has_hole(t: &Term) -> bool {
    walk(t, &|x| matches!(x, Term::Hole))
}

fn walk(t: &Term, p: &dyn Fn(&Term) -> bool) -> bool {
    if p(t) {
        return true;
    }
    match t {
        Term::Pi {
            param_type,
            body_type,
            ..
        } => walk(param_type, p) || walk(body_type, p),
        Term::Lambda {
            param_type, body, ..
        } => walk(param_type, p) || walk(body, p),
        Term::App(f, a) => walk(f, p) || walk(a, p),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => walk(discriminant, p) || walk(motive, p) || cases.iter().any(|c| walk(c, p)),
        Term::Fix { body, .. } => walk(body, p),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| walk(b, p)),
        Term::Let {
            ty, value, body, ..
        } => walk(ty, p) || walk(value, p) || walk(body, p),
        _ => false,
    }
}

/// `idf : Π{A:Type}. Π(a:A). A` — the tier's running example, declared with its binder kinds.
fn with_idf(ctx: &mut Context) {
    ctx.add_declaration("idf", pi("A", ty0(), pi("a", var("A"), var("A"))));
    ctx.set_binder_kinds("idf", vec![ParamKind::Implicit, ParamKind::Explicit]);
}

/// A `Sizeable` class (a structure plus registered instances) — Lean's `class Sizeable (α)`.
fn with_sizeable(ctx: &mut Context) {
    ctx.add_structure(
        "Sizeable",
        &[("A", ty0())],
        &[("size", arrow(var("A"), g("Nat")))],
    );
    ctx.add_instance(
        app(g("Sizeable"), g("Nat")),
        apps(g("Sizeable_mk"), &[g("Nat"), lam("n", g("Nat"), var("n"))]),
    );
    ctx.add_instance(
        app(g("Sizeable"), g("Bool")),
        apps(
            g("Sizeable_mk"),
            &[g("Bool"), lam("b", g("Bool"), g("Zero"))],
        ),
    );
}

// ===========================================================================
// T05-001 · IMPLICIT BINDERS — a ParamKind, never a term-level distinction
// ===========================================================================

#[test]
fn implicit_binders_interleave_and_are_solved_in_their_real_positions() {
    let mut ctx = std_ctx();
    with_idf(&mut ctx);

    // `idf {α} (a : α) : α` — implicitness is invisible in the TERM: the declared type is an
    // ordinary Pi chain, identical to what a fully-explicit declaration would carry.
    assert_eq!(
        ctx.get_global("idf").unwrap(),
        &pi("A", ty0(), pi("a", var("A"), var("A"))),
        "an implicit Pi and an explicit Pi are the same term — binder info lives only in the Context"
    );
    assert_eq!(
        ctx.binder_kinds("idf").unwrap(),
        &[ParamKind::Implicit, ParamKind::Explicit],
        "the binder-kind vector is Lean's binderInfo, position for position"
    );

    // `idf 3` — the type argument is omitted and solved from the value.
    let e = surface_elaborate(&ctx, &app(g("idf"), g("Zero"))).expect("idf Zero elaborates");
    assert_eq!(
        e,
        apps(g("idf"), &[g("Nat"), g("Zero")]),
        "the implicit is inserted in position"
    );
    assert!(
        !has_meta(&e),
        "no metavariable may survive into a kernel term: {e}"
    );
    assert_eq!(infer_type(&ctx, &e).unwrap(), g("Nat"), "idf Zero : Nat");

    // `constMid (_a : Nat) {β} (b : β) : β` — the implicit sits BETWEEN two explicits, which
    // is where an off-by-one in the positional kinds vector shows up.
    ctx.add_declaration(
        "constMid",
        pi("a", g("Nat"), pi("B", ty0(), pi("b", var("B"), var("B")))),
    );
    ctx.set_binder_kinds(
        "constMid",
        vec![
            ParamKind::Explicit,
            ParamKind::Implicit,
            ParamKind::Explicit,
        ],
    );
    let e = surface_elaborate(&ctx, &apps(g("constMid"), &[g("Zero"), g("true")]))
        .expect("constMid elaborates");
    assert_eq!(
        e,
        apps(g("constMid"), &[g("Zero"), g("Bool"), g("true")]),
        "a NON-leading implicit is inserted at its own position, not at the front"
    );
    assert_eq!(infer_type(&ctx, &e).unwrap(), g("Bool"));

    // `emptyOf {α} : List α` — no value argument pins α; it comes from the EXPECTED type.
    let empty_ty = pi("A", ty0(), app(g("TList"), var("A")));
    ctx.add_declaration("emptyOf", empty_ty.clone());
    let mut m = MetaCtx::new();
    let (e, _) = elaborate_app_against(
        &ctx,
        &mut m,
        &g("emptyOf"),
        &empty_ty,
        &[ParamKind::Implicit],
        &[],
        Some(&app(g("TList"), g("Nat"))),
    )
    .expect("emptyOf is solved from the expected type");
    assert_eq!(
        e,
        app(g("emptyOf"), g("Nat")),
        "the expected type pins an unconstrained implicit"
    );
    assert!(
        !has_meta(&e),
        "expected-type solving must leave no metavariable: {e}"
    );

    // `pairUp {α} {β} (a : α) (b : β) : α` — two implicits, each pinned by its own argument.
    ctx.add_declaration(
        "pairUp",
        pi(
            "A",
            ty0(),
            pi("B", ty0(), pi("a", var("A"), pi("b", var("B"), var("A")))),
        ),
    );
    ctx.set_binder_kinds(
        "pairUp",
        vec![
            ParamKind::Implicit,
            ParamKind::Implicit,
            ParamKind::Explicit,
            ParamKind::Explicit,
        ],
    );
    let e = surface_elaborate(&ctx, &apps(g("pairUp"), &[g("Zero"), g("true")]))
        .expect("pairUp elaborates");
    assert_eq!(
        e,
        apps(g("pairUp"), &[g("Nat"), g("Bool"), g("Zero"), g("true")])
    );

    // `applyTo {α} {β : α → Type} (f : (x:α) → β x) (a : α) : β a` — an implicit whose TYPE
    // mentions an earlier implicit, so the telescope is dependent and the metavariables must
    // be solved in order.
    ctx.add_declaration(
        "applyTo",
        pi(
            "A",
            ty0(),
            pi(
                "B",
                arrow(var("A"), ty0()),
                pi(
                    "f",
                    pi("x", var("A"), app(var("B"), var("x"))),
                    pi("a", var("A"), app(var("B"), var("a"))),
                ),
            ),
        ),
    );
    ctx.set_binder_kinds(
        "applyTo",
        vec![
            ParamKind::Implicit,
            ParamKind::Implicit,
            ParamKind::Explicit,
            ParamKind::Explicit,
        ],
    );
    let e = surface_elaborate(&ctx, &apps(g("applyTo"), &[g("Succ"), g("Zero")]))
        .expect("a dependent implicit telescope elaborates");
    assert_eq!(
        e,
        apps(
            g("applyTo"),
            &[g("Nat"), lam("_", g("Nat"), g("Nat")), g("Succ"), g("Zero")]
        ),
        "the higher-order implicit β is solved as a real λ, not left as a metavariable"
    );
    assert!(
        !has_meta(&e),
        "dependent solving must leave no metavariable: {e}"
    );
    assert_eq!(
        normalize(&ctx, &infer_type(&ctx, &e).unwrap()),
        g("Nat"),
        "the dependent result type `β a` reduces to Nat"
    );
}

// ===========================================================================
// T05-002 · STRICT IMPLICIT `⦃x : A⦄` — there is no fourth ParamKind
// ===========================================================================

#[test]
fn a_strict_implicit_is_not_instantiated_until_an_explicit_argument_follows() {
    let mut ctx = std_ctx();

    // `memOf ⦃α⦄ (a : α) : α` — FULLY APPLIED, a strict implicit and an eager one are
    // indistinguishable, which is exactly why the demotion is silent.
    ctx.add_declaration("memOf", pi("A", ty0(), pi("a", var("A"), var("A"))));
    ctx.set_binder_kinds("memOf", vec![ParamKind::Implicit, ParamKind::Explicit]);
    let applied = surface_elaborate(&ctx, &app(g("memOf"), g("Zero"))).expect("memOf Zero");
    assert_eq!(
        applied,
        apps(g("memOf"), &[g("Nat"), g("Zero")]),
        "with an explicit argument supplied, ⦃α⦄ and {{α}} produce the same term"
    );

    // `nilOf ⦃α⦄ : List α` — NO explicit argument follows, so Lean never inserts it from an
    // application: `nilOf` stays a function of its strict implicit. LOGOS's `ParamKind` has
    // exactly `Explicit`/`Implicit`/`Instance`, so the position can only be demoted to the
    // EAGER implicit — and `elaborate_app` then inserts a metavariable nothing constrains.
    let nil_ty = pi("A", ty0(), app(g("TList"), var("A")));
    ctx.add_declaration("nilOf", nil_ty.clone());
    let mut m = MetaCtx::new();
    let (e, _) = elaborate_app(
        &ctx,
        &mut m,
        &g("nilOf"),
        &nil_ty,
        &[ParamKind::Implicit],
        &[],
    )
    .expect("an unapplied strict implicit elaborates");
    assert!(
        !has_meta(&e),
        "a strict implicit must not be instantiated when no explicit argument follows — \
         it left an unsolved metavariable instead: {e}"
    );
    assert_eq!(
        e,
        g("nilOf"),
        "an unapplied ⦃α⦄ global is still the function awaiting its argument"
    );
}

// ===========================================================================
// T05-003 · INSTANCE IMPLICIT `[Foo A]` — deferred resolution over a flat list
// ===========================================================================

#[test]
fn instance_implicits_resolve_after_the_explicit_arguments_and_recurse() {
    let mut ctx = std_ctx();
    with_sizeable(&mut ctx);

    // `sizeOf' {α} [Sizeable α] (a : α) : Nat` — the class's type variable is pinned by the
    // explicit argument, so resolution must be DEFERRED past the explicit pass.
    ctx.add_declaration(
        "sizeOfx",
        pi(
            "A",
            ty0(),
            pi(
                "i",
                app(g("Sizeable"), var("A")),
                pi("a", var("A"), g("Nat")),
            ),
        ),
    );
    ctx.set_binder_kinds(
        "sizeOfx",
        vec![
            ParamKind::Implicit,
            ParamKind::Instance,
            ParamKind::Explicit,
        ],
    );

    let at_nat = surface_elaborate(&ctx, &app(g("sizeOfx"), g("Zero"))).expect("sizeOf' Zero");
    assert_eq!(
        at_nat,
        apps(
            g("sizeOfx"),
            &[
                g("Nat"),
                apps(g("Sizeable_mk"), &[g("Nat"), lam("n", g("Nat"), var("n"))]),
                g("Zero")
            ]
        ),
        "the Nat instance is synthesised into the instance position"
    );
    assert!(
        !has_meta(&at_nat),
        "instance resolution must leave no metavariable: {at_nat}"
    );
    assert_eq!(infer_type(&ctx, &at_nat).unwrap(), g("Nat"));

    // A DIFFERENT argument type must select a different instance from the same flat list.
    let at_bool = surface_elaborate(&ctx, &app(g("sizeOfx"), g("true"))).expect("sizeOf' true");
    assert_ne!(
        at_bool, at_nat,
        "a Bool argument must select the Bool instance, not the Nat one"
    );
    assert_eq!(infer_type(&ctx, &at_bool).unwrap(), g("Nat"));

    // A RECURSIVE instance: `Sizeable (TList α)` needs `Sizeable α` resolved first, so the
    // flat list is scanned more than once and the premise is discharged recursively.
    ctx.add_instance(
        pi(
            "A",
            ty0(),
            pi(
                "i",
                app(g("Sizeable"), var("A")),
                app(g("Sizeable"), app(g("TList"), var("A"))),
            ),
        ),
        lam(
            "A",
            ty0(),
            lam(
                "i",
                app(g("Sizeable"), var("A")),
                apps(
                    g("Sizeable_mk"),
                    &[
                        app(g("TList"), var("A")),
                        lam("xs", app(g("TList"), var("A")), g("Zero")),
                    ],
                ),
            ),
        ),
    );
    let at_list = surface_elaborate(&ctx, &app(g("sizeOfx"), app(g("TNil"), g("Nat"))))
        .expect("a recursive instance resolves");
    assert!(
        !has_meta(&at_list),
        "recursive resolution must leave no metavariable: {at_list}"
    );
    assert_eq!(
        infer_type(&ctx, &at_list).unwrap(),
        g("Nat"),
        "Sizeable (TList Nat) was built from Sizeable Nat"
    );

    // `defaultOf {α} [Defaultable α] : α` — NO explicit argument mentions α, so the class
    // type is fixed only by the EXPECTED type. The expected-type pass must run BEFORE
    // instance resolution or this is unsolvable.
    ctx.add_structure("Defaultable", &[("A", ty0())], &[("dflt", var("A"))]);
    ctx.add_instance(
        app(g("Defaultable"), g("Nat")),
        apps(g("Defaultable_mk"), &[g("Nat"), g("Zero")]),
    );
    ctx.add_instance(
        app(g("Defaultable"), g("Bool")),
        apps(g("Defaultable_mk"), &[g("Bool"), g("false")]),
    );
    let dflt_ty = pi(
        "A",
        ty0(),
        pi("i", app(g("Defaultable"), var("A")), var("A")),
    );
    ctx.add_declaration("defaultOf", dflt_ty.clone());
    let mut m = MetaCtx::new();
    let (e, ty) = elaborate_app_against(
        &ctx,
        &mut m,
        &g("defaultOf"),
        &dflt_ty,
        &[ParamKind::Implicit, ParamKind::Instance],
        &[],
        Some(&g("Bool")),
    )
    .expect("the expected type fixes the class variable before resolution");
    assert_eq!(ty, g("Bool"), "(defaultOf : Bool) : Bool");
    assert_eq!(
        e,
        apps(
            g("defaultOf"),
            &[
                g("Bool"),
                apps(g("Defaultable_mk"), &[g("Bool"), g("false")])
            ]
        ),
        "the Bool instance was chosen from the expected type alone"
    );

    // `sizeBoth {α} (a : α) [Sizeable α] (b : α) : Nat` — the instance sits BETWEEN two
    // explicit parameters: kinds `[Implicit, Explicit, Instance, Explicit]`.
    ctx.add_declaration(
        "sizeBoth",
        pi(
            "A",
            ty0(),
            pi(
                "a",
                var("A"),
                pi(
                    "i",
                    app(g("Sizeable"), var("A")),
                    pi("b", var("A"), g("Nat")),
                ),
            ),
        ),
    );
    ctx.set_binder_kinds(
        "sizeBoth",
        vec![
            ParamKind::Implicit,
            ParamKind::Explicit,
            ParamKind::Instance,
            ParamKind::Explicit,
        ],
    );
    let e = surface_elaborate(&ctx, &apps(g("sizeBoth"), &[g("Zero"), g("Zero")]))
        .expect("an instance between two explicits elaborates");
    assert!(!has_meta(&e), "no metavariable survives: {e}");
    assert_eq!(infer_type(&ctx, &e).unwrap(), g("Nat"));
}

// ===========================================================================
// T05-004 · `@f` — explicit-all application vanishes in the elaborated term
// ===========================================================================

#[test]
fn explicit_all_application_yields_the_same_left_nested_spine() {
    let mut ctx = std_ctx();
    with_idf(&mut ctx);

    // `@idf Nat 3` and `idf 3` are the same term. There is nothing to emit for the `@`.
    let inferred = surface_elaborate(&ctx, &app(g("idf"), g("Zero"))).expect("idf Zero");
    let written = apps(g("idf"), &[g("Nat"), g("Zero")]);
    assert_eq!(inferred, written, "@idf Nat 3 and idf 3 are the same term");

    // `@` is a property of a USE SITE, never of the declaration: writing the type argument
    // out must not change the callee's binder kinds.
    assert_eq!(
        ctx.binder_kinds("idf").unwrap(),
        &[ParamKind::Implicit, ParamKind::Explicit],
        "an @-application must not rewrite the declaration's binder info"
    );

    // The spine is binary and LEFT-nested.
    match &written {
        Term::App(f, _) => assert!(
            matches!(**f, Term::App(..)),
            "an application spine is left-nested: App(App(f,a),b)"
        ),
        other => panic!("expected an application spine, got {other}"),
    }

    // `natId : Nat → Nat := @idf Nat` — a PARTIAL @-application is a value in its own right
    // and must typecheck without being completed or eta-expanded.
    let partial = app(g("idf"), g("Nat"));
    assert_eq!(
        infer_type(&ctx, &partial).unwrap(),
        pi("a", g("Nat"), g("Nat")),
        "@idf Nat : Nat → Nat, un-eta-expanded"
    );

    // `@compose Nat Nat Nat (·+1) (·*2) 3` — several arguments are themselves FUNCTIONS, so
    // a right-nested fold of the same argument list is the classic silent mistranslation.
    ctx.add_declaration(
        "compose",
        pi(
            "A",
            ty0(),
            pi(
                "B",
                ty0(),
                pi(
                    "C",
                    ty0(),
                    pi(
                        "gf",
                        arrow(var("B"), var("C")),
                        pi("f", arrow(var("A"), var("B")), pi("a", var("A"), var("C"))),
                    ),
                ),
            ),
        ),
    );
    let left = apps(
        g("compose"),
        &[
            g("Nat"),
            g("Nat"),
            g("Nat"),
            g("Succ"),
            g("Succ"),
            g("Zero"),
        ],
    );
    assert_eq!(
        infer_type(&ctx, &left).unwrap(),
        g("Nat"),
        "the left-nested fold typechecks"
    );
    let right = app(
        g("compose"),
        app(
            g("Nat"),
            app(
                g("Nat"),
                app(g("Nat"), app(g("Succ"), app(g("Succ"), g("Zero")))),
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &right).is_err(),
        "a right-nested fold of the same arguments must not be accepted"
    );

    // `@polyId.{1} Nat 3` — `@` on a universe-polymorphic constant supplies LEVELS too, which
    // only `Term::Const { name, levels }` can express, and only for an `add_universe_poly`
    // definition.
    let u = Universe::Var("v".to_string());
    ctx.add_universe_poly(
        "polyId",
        vec!["v".to_string()],
        pi("A", Term::Sort(u.clone()), pi("a", var("A"), var("A"))),
        lam("A", Term::Sort(u), lam("a", var("A"), var("a"))),
    );
    let at_level_1 = Term::Const {
        name: "polyId".to_string(),
        levels: vec![Universe::Type(1)],
    };
    assert_eq!(
        infer_type(&ctx, &at_level_1).unwrap(),
        pi(
            "A",
            Term::Sort(Universe::Type(1)),
            pi("a", var("A"), var("A"))
        ),
        "@polyId at level 1 instantiates the level parameter"
    );
    let at_type = apps(at_level_1, &[ty0(), g("Nat")]);
    assert_eq!(
        normalize(&ctx, &at_type),
        g("Nat"),
        "@polyId at level 1 applied to Type and Nat reduces to Nat"
    );

    // `@Pair.fst Nat Bool ⟨1, true⟩` — a projection's own parameters become explicit.
    ctx.add_structure(
        "Pair",
        &[("A", ty0()), ("B", ty0())],
        &[("fst", var("A")), ("snd", var("B"))],
    );
    let p = apps(g("Pair_mk"), &[g("Nat"), g("Bool"), g("Zero"), g("true")]);
    let proj = apps(g("Pair_fst"), &[g("Nat"), g("Bool"), p]);
    assert_eq!(
        normalize(&ctx, &proj),
        g("Zero"),
        "@Pair.fst Nat Bool ⟨0,true⟩ ⇝ 0"
    );
}

// ===========================================================================
// T05-005 · NAMED ARGUMENTS `f (x := 1)` — resolved into the declaration's order
// ===========================================================================

#[test]
fn named_arguments_resolve_into_the_declarations_parameter_order() {
    let mut repl = Repl::new();
    // `sub3 (x y z : Nat) : Nat := x*100 + y*10 + z` — every parameter has the SAME type, so
    // a source-ordered spine typechecks perfectly and computes a different number. Nothing in
    // the kernel can notice: `Term::App` carries no names to compare against.
    repl.execute(
        "Definition sub3 : Int -> Int -> Int -> Int := \
         fun x : Int => fun y : Int => fun z : Int => add (mul x 100) (add (mul y 10) z).",
    )
    .expect("define sub3");
    assert_eq!(
        repl.execute("Eval (sub3 1 2 3).").unwrap().trim(),
        "123",
        "positional baseline"
    );

    // Written out of order by NAME, the elaborated spine must still be `sub3 1 2 3`.
    let named = TermParser::parse("sub3 (y := 2) (x := 1) (z := 3)");
    assert!(
        named.is_ok(),
        "named arguments must be expressible: `sub3 (y := 2) (x := 1) (z := 3)` — got {named:?}"
    );
    let elab = surface_elaborate(repl.context(), &named.unwrap())
        .expect("a named-argument application elaborates");
    assert_eq!(
        normalize(repl.context(), &elab),
        int(123),
        "named arguments are resolved into the DECLARATION's order, not the source's"
    );

    // A named argument that SKIPS an earlier explicit parameter eta-expands: Lean's elaborated
    // term for `sub3 (y := 2) (z := 9)` is `fun x => sub3 x 2 9`, a LAMBDA, not a partial
    // application.
    let skipping = TermParser::parse("sub3 (y := 2) (z := 9)").expect("skipping form parses");
    let elab = surface_elaborate(repl.context(), &skipping).expect("the skipping form elaborates");
    assert!(
        matches!(elab, Term::Lambda { .. }),
        "a named argument that skips an explicit parameter eta-expands into a λ: {elab}"
    );

    // A named argument naming an IMPLICIT parameter is the same term as the @-form.
    repl.execute("Definition idf : {A : Type} -> A -> A := fun A : Type => fun a : A => a.")
        .expect("define idf");
    let by_name = TermParser::parse("idf (A := Int) 5").expect("named implicit parses");
    assert_eq!(
        surface_elaborate(repl.context(), &by_name).expect("elaborates"),
        apps(g("idf"), &[g("Int"), int(5)]),
        "naming an implicit supplies it exactly where @ would"
    );
}

// ===========================================================================
// T05-006 · DEFAULT ARGUMENTS `(n : Nat := 3)` — no carrier for the default
// ===========================================================================

#[test]
fn an_omitted_default_argument_is_filled_from_the_declaration() {
    let mut repl = Repl::new();
    // `withDefault (n : Nat) (m : Nat := 3) : Nat := n + m`. LOGOS's `Term::Pi` has three
    // fields and no default slot, and `ParamKind` has no `Optional` kind — so the default has
    // nowhere to be recorded at all.
    let declared = repl.execute(
        "Definition withDefault : Int -> (m : Int := 3) -> Int := \
         fun n : Int => fun m : Int => add n m.",
    );
    assert!(
        declared.is_ok(),
        "a parameter's default value must be expressible in a declaration: {declared:?}"
    );
    // Omitting the default must produce the SAME term as supplying it, not a shorter spine.
    assert_eq!(
        repl.execute("Eval (withDefault 1).")
            .expect("the default is filled in")
            .trim(),
        "4",
        "an omitted default is inlined at the call site: withDefault 1 = withDefault 1 3"
    );

    // A default that MENTIONS an earlier parameter must be inlined with that parameter's
    // ACTUAL argument substituted, never left as an open term with a free `n`.
    repl.execute(
        "Definition sized : Int -> (m : Int := mul n 2) -> Int := \
         fun n : Int => fun m : Int => add n m.",
    )
    .expect("a dependent default is expressible");
    assert_eq!(
        repl.execute("Eval (sized 5).")
            .expect("the dependent default is substituted")
            .trim(),
        "15",
        "sized 5 = sized 5 10 — the default is `n * 2` with n := 5"
    );

    // Structure FIELD defaults: `add_structure`'s `{name}_mk` takes every field, so a literal
    // that omits a defaulted field must still emit the full-arity constructor.
    let mut ctx = std_ctx();
    ctx.add_structure("Wrap", &[], &[("val", g("Nat")), ("tag", g("Nat"))]);
    assert_eq!(
        infer_type(&ctx, &app(g("Wrap_mk"), g("Zero"))).unwrap(),
        pi("tag", g("Nat"), g("Wrap")),
        "the short form is a partial application, not a Wrap — the default must be supplied"
    );
}

// ===========================================================================
// T05-007 · AUTO PARAMS `(h : P := by tac)` — the tactic must run per call site
// ===========================================================================

#[test]
fn an_auto_param_obligation_is_discharged_at_each_call_site() {
    let mut ctx = std_ctx();
    // `safeDiv (a b : Nat) (_h : b ≠ 0 := by decide) : Nat` — the proposition MENTIONS an
    // earlier parameter, so `by decide` produces a DIFFERENT proof term at every call site.
    let ne_zero = |b: Term| app(g("Not"), apps(g("Eq"), &[g("Nat"), b, g("Zero")]));
    ctx.add_declaration(
        "safeDiv",
        pi(
            "a",
            g("Nat"),
            pi("b", g("Nat"), pi("h", ne_zero(var("b")), g("Nat"))),
        ),
    );

    // The proof position is filled by running the tactic AT THIS CALL SITE, so the whole
    // application is a `Nat` — not a function still awaiting a proof.
    let call = surface_elaborate(&ctx, &apps(g("safeDiv"), &[nat(10), nat(2)]))
        .expect("safeDiv 10 2 elaborates");
    assert_eq!(
        infer_type(&ctx, &call).unwrap(),
        g("Nat"),
        "an auto param must be discharged at the call site, leaving a Nat: {call}"
    );
    assert!(
        !has_hole(&call),
        "the proof position must not be a Term::Hole — a hole matches ANY type: {call}"
    );
    assert!(
        !has_meta(&call),
        "the proof position must not be an unsolved metavariable: {call}"
    );

    // A second call site has a different `b`, hence a different proposition and a different
    // proof — reusing one proof term for every call site proves the wrong proposition.
    let other = surface_elaborate(&ctx, &apps(g("safeDiv"), &[nat(9), nat(3)]))
        .expect("safeDiv 9 3 elaborates");
    assert_ne!(
        call, other,
        "each call site's tactic proof is about its own proposition"
    );

    // And the escape hatches the corpus forbids: the obligation must not be axiomatised away.
    assert!(
        ctx.get_global("autoParam").is_none(),
        "`autoParam` must not leak into the context as an opaque type former"
    );
}

// ===========================================================================
// T05-008 · AUTO-BOUND IMPLICITS — binders Lean invents, at `Sort u`
// ===========================================================================

#[test]
fn an_auto_bound_implicit_is_universe_polymorphic_like_lean() {
    let mut repl = Repl::new();

    // `def idf (a : α) : α := a` — `α` is never written as a binder; it is auto-bound at the
    // FRONT of the telescope and then inferred at each use.
    repl.execute("Definition idf : A -> A := fun a : A => a.")
        .expect("A is auto-bound");
    assert_eq!(
        repl.execute("Eval (idf 0).").unwrap().trim(),
        "0",
        "idf 0 ⇝ 0 (A := Int)"
    );
    assert_eq!(
        repl.execute("Eval (idf true).").unwrap().trim(),
        "true",
        "idf true ⇝ true"
    );

    // Two auto-bound variables, in FIRST-OCCURRENCE order — the order is not derivable from
    // the surface text, so the emitted telescope has to be the elaborated one.
    repl.execute("Definition constf : A -> B -> A := fun a : A => fun b : B => a.")
        .expect("A and B auto-bound");
    assert_eq!(repl.execute("Eval (constf 0 true).").unwrap().trim(), "0");
    let constf_ty = repl
        .context()
        .get_definition_type("constf")
        .expect("constf is defined");
    match constf_ty {
        Term::Pi {
            param, body_type, ..
        } => {
            assert_eq!(param, "A", "the first-occurring variable binds outermost");
            assert!(
                matches!(&**body_type, Term::Pi { param, .. } if param == "B"),
                "the second auto-bound binder follows: {body_type}"
            );
        }
        other => panic!("constf's type must open with the auto-bound telescope, got {other}"),
    }

    // An auto-bound variable whose binder must be inserted before another it depends on.
    repl.execute("Definition applyF : (A -> B) -> A -> B := fun f : A -> B => fun a : A => f a.")
        .expect("a dependency-ordered auto-bind");
    assert_eq!(
        repl.execute("Eval (applyF (fun n : Int => add n 1) 2).")
            .unwrap()
            .trim(),
        "3"
    );

    // With `autoImplicit` on, Lean binds `α` at `Sort u` for a FRESH auto-bound universe — not
    // at `Type 0`. Pinning it to `Type 0` is a silent monomorphisation: the declaration still
    // typechecks at most use sites and is wrong wherever the source instantiated it higher.
    let at_a_type = repl.execute("Check (idf Nat).");
    assert!(
        at_a_type.is_ok(),
        "an auto-bound implicit is universe-polymorphic (Lean's `Sort u`), so it accepts a \
         TYPE argument — pinning it to Type 0 monomorphises it: {at_a_type:?}"
    );
}

// ===========================================================================
// T05-009 · ANONYMOUS CONSTRUCTOR `⟨…⟩` — resolved against the expected type
// ===========================================================================

#[test]
fn an_anonymous_constructor_resolves_against_its_expected_inductive() {
    let mut ctx = std_ctx();
    ctx.add_structure("Point", &[], &[("x", g("Nat")), ("y", g("Nat"))]);
    ctx.add_structure(
        "Pair",
        &[("A", ty0()), ("B", ty0())],
        &[("fst", var("A")), ("snd", var("B"))],
    );
    ctx.add_structure(
        "Triple",
        &[],
        &[("a", g("Nat")), ("b", g("Nat")), ("c", g("Nat"))],
    );
    ctx.add_structure(
        "NestedPair",
        &[],
        &[("head", g("Nat")), ("rest", g("Point"))],
    );

    // `(⟨1, 2⟩ : Point)` is the named constructor, term for term.
    let mut m = MetaCtx::new();
    let p = elaborate_anon_ctor(&ctx, &mut m, &g("Point"), &[g("Zero"), g("Zero")])
        .expect("⟨0,0⟩ : Point");
    assert_eq!(
        p,
        apps(g("Point_mk"), &[g("Zero"), g("Zero")]),
        "⟨…⟩ is Point.mk"
    );

    // PARAMETERISED: the structure's parameters are read off the expected type and applied
    // BEFORE the fields — `⟨0, true⟩ : Pair Nat Bool` is `Pair_mk Nat Bool 0 true`.
    let pair_ty = apps(g("Pair"), &[g("Nat"), g("Bool")]);
    let pr = elaborate_anon_ctor(&ctx, &mut m, &pair_ty, &[g("Zero"), g("true")])
        .expect("⟨0,true⟩ : Pair Nat Bool");
    assert_eq!(
        pr,
        apps(g("Pair_mk"), &[g("Nat"), g("Bool"), g("Zero"), g("true")])
    );
    assert_eq!(infer_type(&ctx, &pr).unwrap(), pair_ty);

    // Three components, FLATTENED into a three-field constructor …
    let t = elaborate_anon_ctor(
        &ctx,
        &mut m,
        &g("Triple"),
        &[g("Zero"), g("Zero"), g("Zero")],
    )
    .expect("⟨a,b,c⟩ : Triple");
    assert_eq!(t, apps(g("Triple_mk"), &[g("Zero"), g("Zero"), g("Zero")]));
    // … and the SAME surface text RIGHT-ASSOCIATED into a two-field one whose second field is
    // itself a structure. Always-flatten and always-nest are both arity errors.
    let inner = apps(g("Point_mk"), &[g("Zero"), g("Zero")]);
    let n = elaborate_anon_ctor(&ctx, &mut m, &g("NestedPair"), &[g("Zero"), inner.clone()])
        .expect("⟨a, ⟨b,c⟩⟩ : NestedPair");
    assert_eq!(n, apps(g("NestedPair_mk"), &[g("Zero"), inner]));
    assert_ne!(
        t, n,
        "flattened and right-associated ⟨…⟩ are different terms"
    );

    // `⟨h1, h2⟩ : A ∧ B` resolves to the LOGOS prelude's own `conj`, not `And.intro`.
    let and_ty = apps(g("And"), &[g("True"), g("True")]);
    let c =
        elaborate_anon_ctor(&ctx, &mut m, &and_ty, &[g("I"), g("I")]).expect("⟨I,I⟩ : True ∧ True");
    assert_eq!(c, apps(g("conj"), &[g("True"), g("True"), g("I"), g("I")]));
    assert_eq!(infer_type(&ctx, &c).unwrap(), and_ty);

    // An implicit parameter that NOTHING in the components pins, solved from the expected type.
    let empty_ty = apps(g("Pair"), &[g("Nat"), app(g("TList"), g("Bool"))]);
    let e = elaborate_anon_ctor(
        &ctx,
        &mut m,
        &empty_ty,
        &[g("Zero"), app(g("TNil"), g("Bool"))],
    )
    .expect("⟨0, []⟩ : Pair Nat (TList Bool)");
    assert!(
        !has_meta(&e) && !has_hole(&e),
        "no hole and no metavariable survive: {e}"
    );
    assert_eq!(infer_type(&ctx, &e).unwrap(), empty_ty);

    // `⟨…⟩` at a MULTI-constructor type is ambiguous and must be refused, not guessed.
    assert!(
        elaborate_anon_ctor(&ctx, &mut m, &g("Nat"), &[g("Zero")]).is_err(),
        "⟨…⟩ at a type with two constructors must be rejected"
    );
}

// ===========================================================================
// T05-010 · DOT NOTATION — `H_field <params…> receiver`
// ===========================================================================

#[test]
fn dot_notation_projects_with_the_structures_parameters_first() {
    let mut ctx = std_ctx();
    ctx.add_structure(
        "Pair",
        &[("A", ty0()), ("B", ty0())],
        &[("fst", var("A")), ("snd", var("B"))],
    );
    ctx.add_structure("Point", &[], &[("x", g("Nat")), ("y", g("Nat"))]);
    ctx.add_structure("Outer", &[], &[("hd", g("Nat")), ("inner", g("Point"))]);

    // PARAMETERISED: the parameters come off the RECEIVER'S TYPE and are applied before the
    // receiver — `p.fst` with `p : Pair A B` is `Pair_fst A B p`, never `Pair_fst p`.
    let p = apps(g("Pair_mk"), &[g("Nat"), g("Bool"), g("Zero"), g("true")]);
    let mut m = MetaCtx::new();
    let fst = elaborate_dot(&ctx, &mut m, &p, "fst").expect("p.fst");
    assert_eq!(fst, apps(g("Pair_fst"), &[g("Nat"), g("Bool"), p.clone()]));
    assert_eq!(infer_type(&ctx, &fst).unwrap(), g("Nat"));
    assert_eq!(
        normalize(&ctx, &fst),
        g("Zero"),
        "p.fst ⇝ the first component"
    );
    let snd = elaborate_dot(&ctx, &mut m, &p, "snd").expect("p.snd");
    assert_eq!(snd, apps(g("Pair_snd"), &[g("Nat"), g("Bool"), p]));
    assert_eq!(normalize(&ctx, &snd), g("true"));

    // NON-parameterised: `Point_x p`, and the flat `{H}_{field}` name convention (LOGOS keys
    // have no namespaces, so Lean's `Point.x` must be emitted as `Point_x`).
    let pt = apps(g("Point_mk"), &[g("Zero"), app(g("Succ"), g("Zero"))]);
    let x = elaborate_dot(&ctx, &mut m, &pt, "x").expect("pt.x");
    assert_eq!(x, app(g("Point_x"), pt.clone()));
    assert_eq!(normalize(&ctx, &x), g("Zero"));

    // CHAINED: each step re-normalises the receiver's type.
    let o = apps(g("Outer_mk"), &[g("Zero"), pt]);
    let inner = elaborate_dot(&ctx, &mut m, &o, "inner").expect("o.inner");
    let chained = elaborate_dot(&ctx, &mut m, &inner, "y").expect("o.inner.y");
    assert_eq!(
        normalize(&ctx, &chained),
        app(g("Succ"), g("Zero")),
        "a chained projection reduces to the nested field"
    );

    // A receiver whose type is a definitional ALIAS: the head is read off the NORMALISED type,
    // so `MyPoint := Point` still finds `Point_x`.
    ctx.add_definition("MyPoint".to_string(), ty0(), g("Point"));
    ctx.add_definition(
        "mp".to_string(),
        g("MyPoint"),
        apps(g("Point_mk"), &[app(g("Succ"), g("Zero")), g("Zero")]),
    );
    let through_alias = elaborate_dot(&ctx, &mut m, &g("mp"), "x").expect("mp.x through the alias");
    assert_eq!(through_alias, app(g("Point_x"), g("mp")));
    assert_eq!(normalize(&ctx, &through_alias), app(g("Succ"), g("Zero")));

    // An unknown projection is a HARD error, never a silently bad term.
    assert!(
        elaborate_dot(&ctx, &mut m, &g("mp"), "nope").is_err(),
        "an unknown field must be refused"
    );
}

// ===========================================================================
// T05-011 · `Coe` — one-step insertion works; the transitive chain does not
// ===========================================================================

#[test]
fn coercions_compose_transitively_like_lean_coe() {
    let mut ctx = std_ctx();
    ctx.add_declaration("boolToNat", arrow(g("Bool"), g("Nat")));
    ctx.add_coercion(g("Bool"), g("Nat"), g("boolToNat"));
    ctx.add_declaration("takesNat", arrow(g("Nat"), g("Nat")));

    // One step, driven purely by the expected parameter type — the source says no `↑`.
    let one = surface_elaborate(&ctx, &app(g("takesNat"), g("true"))).expect("takesNat true");
    assert_eq!(
        one,
        app(g("takesNat"), app(g("boolToNat"), g("true"))),
        "the registered coercion is inserted around the argument"
    );
    assert_eq!(infer_type(&ctx, &one).unwrap(), g("Nat"));

    // An argument that already has the expected type must NOT be double-wrapped.
    let already = app(g("boolToNat"), g("true"));
    assert_eq!(
        surface_elaborate(&ctx, &app(g("takesNat"), already.clone())).unwrap(),
        app(g("takesNat"), already),
        "a well-typed argument is left untouched"
    );

    // A coercion is never invented: with none registered, the mismatch stays a mismatch.
    let mut bare = std_ctx();
    bare.add_declaration("takesNat", arrow(g("Nat"), g("Nat")));
    let unrepairable = surface_elaborate(&bare, &app(g("takesNat"), g("true")))
        .map(|t| infer_type(&bare, &t).is_ok())
        .unwrap_or(false);
    assert!(
        !unrepairable,
        "without a coercion the mismatch must not become a well-typed term"
    );

    // TRANSITIVITY. Lean's `CoeT` chains head/body/tail, so `Toggle → Bool` and `Bool → Nat`
    // compose with no `Toggle → Nat` instance declared. LOGOS's `resolve_coercion` is a single
    // linear scan over `(from, to, coe)` triples with no transitive closure.
    ctx.add_inductive("Toggle", ty0());
    ctx.add_constructor("bone", "Toggle", g("Toggle"));
    ctx.add_declaration("toggleToBool", arrow(g("Toggle"), g("Bool")));
    ctx.add_coercion(g("Toggle"), g("Bool"), g("toggleToBool"));
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:?}",
        ctx.refused_declarations()
    );
    let chained =
        surface_elaborate(&ctx, &app(g("takesNat"), g("bone"))).expect("the chain elaborates");
    assert!(
        infer_type(&ctx, &chained).is_ok(),
        "Toggle → Bool → Nat must compose from the two one-step coercions; instead the argument \
         was left un-coerced and the emitted term does not typecheck: {chained}"
    );
    assert_eq!(
        chained,
        app(
            g("takesNat"),
            app(g("boolToNat"), app(g("toggleToBool"), g("bone")))
        ),
        "the composite path is the two coercions applied in order"
    );
}

// ===========================================================================
// T05-012 · `CoeFun` / `CoeSort` — coercion into a function or a sort position
// ===========================================================================

#[test]
fn a_structure_can_be_applied_as_a_function_and_used_as_a_sort() {
    let mut ctx = std_ctx();
    // `structure Fn where f : Nat → Nat` + `instance : CoeFun Fn (fun _ => Nat → Nat)`.
    ctx.add_structure("Fn", &[], &[("f", arrow(g("Nat"), g("Nat")))]);
    let gv = app(g("Fn_mk"), g("Succ"));

    // `g 3` APPLIES a structure. LOGOS inserts coercions only at ARGUMENT positions: the
    // `App` rule whnfs the head's type and demands a `Pi`, with no coercion hook.
    let applied = surface_elaborate(&ctx, &app(gv.clone(), g("Zero"))).expect("g 0 elaborates");
    let applied_ty = infer_type(&ctx, &applied);
    assert!(
        matches!(&applied_ty, Ok(t) if *t == g("Nat")),
        "a structure with a CoeFun instance must be applicable as a function — `g 0` should \
         elaborate to `Fn_f g 0 : Nat`, got {applied} of type {applied_ty:?}"
    );

    // A MULTI-FIELD structure with a `CoeFun` — "just unfold it to the function field" loses
    // the other fields and every theorem quantifying over the structure, so the coercion has
    // to be a real mechanism, not a rewrite.
    ctx.add_structure(
        "Named",
        &[],
        &[("name", g("Nat")), ("run", arrow(g("Nat"), g("Nat")))],
    );
    let nv = apps(g("Named_mk"), &[g("Zero"), g("Succ")]);
    let n_applied = surface_elaborate(&ctx, &app(nv, g("Zero"))).expect("elaborates");
    assert!(
        infer_type(&ctx, &n_applied).is_ok(),
        "a multi-field structure with a CoeFun is applicable too: {n_applied}"
    );

    // `CoeSort`: a VALUE used in TYPE position. As in Lean, the ELABORATOR inserts the coercion
    // (`ensureType` calls `coerceToSort?`, `Lean/Elab/Term.lean:1859`) and the kernel checks
    // what it emits; the kernel itself never coerces, so the raw term stays a type error there —
    // Lean's kernel refuses it too.
    ctx.add_structure("Ty", &[], &[("carrier", ty0())]);
    let tv = app(g("Ty_mk"), g("Nat"));
    let raw = pi("x", tv.clone(), tv.clone());
    assert!(
        infer_type(&ctx, &raw).is_err(),
        "the kernel refuses a structure VALUE in a type position; coercion is the elaborator's"
    );
    let elaborated = surface_elaborate(&ctx, &raw).expect("`Π(x : t). t` elaborates");
    let carrier = app(g("Ty_carrier"), tv.clone());
    assert_eq!(
        elaborated,
        pi("x", carrier.clone(), carrier),
        "a CoeSort instance must let `t : Ty` stand in a type position: Π(x : t). t becomes \
         Π(x : Ty.carrier t). Ty.carrier t"
    );
    assert!(
        matches!(infer_type(&ctx, &elaborated), Ok(Term::Sort(_))),
        "…and the elaborated binder is a type the kernel accepts"
    );
    assert_eq!(
        normalize(&ctx, &elaborated),
        pi("x", g("Nat"), g("Nat")),
        "…namely `Π(x : Nat). Nat`"
    );
    // With TWO sort-typed fields there is no canonical coercion, and none is guessed: the value
    // is left as written, so it still fails where it always did.
    ctx.add_structure("TwoTy", &[], &[("left", ty0()), ("right", ty0())]);
    let two = apps(g("TwoTy_mk"), &[g("Nat"), g("Bool")]);
    let ambiguous = surface_elaborate(&ctx, &pi("x", two.clone(), g("Nat")))
        .expect("elaboration leaves an uncoercible binder as written");
    assert!(
        infer_type(&ctx, &ambiguous).is_err(),
        "a structure with two sort-typed fields has no CoeSort, so it is not silently projected"
    );

    // An ORDINARY coercion into a function type, Lean's `instance : Coe Fn (Nat → Nat)`, acts
    // where that function type is EXPECTED: Lean 4.14 elaborates `takesFun gv` to
    // `takesFun gv.f` (as it does under a `CoeFun` instance). It reaches exactly its own target:
    // `takesBoolFun gv` stays a type mismatch in Lean, and here.
    ctx.add_coercion(g("Fn"), arrow(g("Nat"), g("Nat")), g("Fn_f"));
    ctx.add_declaration("takesFun", arrow(arrow(g("Nat"), g("Nat")), g("Nat")));
    ctx.add_declaration(
        "takesBoolFun",
        arrow(arrow(g("Bool"), g("Bool")), g("Bool")),
    );
    let coerced = surface_elaborate(&ctx, &app(g("takesFun"), gv.clone()))
        .expect("an argument of the coercion's source type elaborates");
    assert_eq!(
        coerced,
        app(g("takesFun"), app(g("Fn_f"), gv.clone())),
        "the registered coercion is inserted around the argument, as Lean inserts `gv.f`"
    );
    assert_eq!(infer_type(&ctx, &coerced).unwrap(), g("Nat"));
    let elsewhere = surface_elaborate(&ctx, &app(g("takesBoolFun"), gv))
        .map(|t| infer_type(&ctx, &t).is_ok())
        .unwrap_or(false);
    assert!(
        !elsewhere,
        "a coercion into `Nat → Nat` must not make an `Fn` acceptable where a `Bool → Bool` is \
         expected — Lean rejects `takesBoolFun gv`"
    );
}

// ===========================================================================
// T05-013 · `_` HOLES — solved by unification, never reaching the kernel
// ===========================================================================

#[test]
fn underscore_holes_are_solved_and_never_reach_the_kernel() {
    let mut ctx = std_ctx();
    ctx.add_declaration("idf", pi("A", ty0(), pi("a", var("A"), var("A"))));

    // `@idf _ 3` — the hole is in an explicit type-argument position, solved by the value.
    let e = surface_elaborate(&ctx, &apps(g("idf"), &[Term::Hole, g("Zero")]))
        .expect("idf _ Zero elaborates");
    assert_eq!(
        e,
        apps(g("idf"), &[g("Nat"), g("Zero")]),
        "the hole is solved to Nat"
    );
    assert!(
        !has_hole(&e) && !has_meta(&e),
        "no hole and no metavariable survive: {e}"
    );
    assert!(
        infer_type(&ctx, &e).is_ok(),
        "infer_type — not merely check_type — must succeed"
    );

    // `@List.nil _` — the solution comes from the EXPECTED type, not from any argument.
    let from_expected = surface_elaborate_against(
        &ctx,
        &app(g("TNil"), Term::Hole),
        Some(&app(g("TList"), g("Nat"))),
    )
    .expect("TNil _ : TList Nat");
    assert_eq!(
        from_expected,
        app(g("TNil"), g("Nat")),
        "a later constraint solves the hole"
    );

    // Several holes in one spine, each solved by a different constraint.
    ctx.add_declaration(
        "compose",
        pi(
            "A",
            ty0(),
            pi(
                "B",
                ty0(),
                pi(
                    "C",
                    ty0(),
                    pi(
                        "gf",
                        arrow(var("B"), var("C")),
                        pi("f", arrow(var("A"), var("B")), pi("a", var("A"), var("C"))),
                    ),
                ),
            ),
        ),
    );
    let e = surface_elaborate(
        &ctx,
        &apps(
            g("compose"),
            &[
                Term::Hole,
                Term::Hole,
                Term::Hole,
                g("Succ"),
                g("Succ"),
                g("Zero"),
            ],
        ),
    )
    .expect("three holes elaborate");
    assert!(
        !has_hole(&e) && !has_meta(&e),
        "every hole in the spine is solved: {e}"
    );
    assert_eq!(infer_type(&ctx, &e).unwrap(), g("Nat"));

    // A hole in a structure literal's PARAMETER position.
    ctx.add_structure(
        "Pair",
        &[("A", ty0()), ("B", ty0())],
        &[("fst", var("A")), ("snd", var("B"))],
    );
    let e = surface_elaborate(
        &ctx,
        &apps(
            g("Pair_mk"),
            &[Term::Hole, Term::Hole, g("Zero"), g("true")],
        ),
    )
    .expect("Pair.mk _ _ 0 true elaborates");
    assert_eq!(
        e,
        apps(g("Pair_mk"), &[g("Nat"), g("Bool"), g("Zero"), g("true")])
    );

    // FAIL-CLOSED: a hole nothing can solve must not become a term the kernel accepts. A
    // surviving metavariable is an ordinary free `Var` to the kernel, so this must be caught.
    let unsolvable = surface_elaborate(&ctx, &app(g("idf"), Term::Hole)).expect("elaborates");
    assert!(
        infer_type(&ctx, &unsolvable).is_err(),
        "an unsolved hole must not produce a kernel-accepted term: {unsolvable}"
    );
}

// ===========================================================================
// T05-014 · HIGHER-ORDER PATTERN UNIFICATION — Miller patterns, both ways, + η
// ===========================================================================

#[test]
fn higher_order_pattern_unification_solves_motives_in_both_directions() {
    let ctx = std_ctx();
    let lctx = |ps: &[(&str, Term)]| -> Vec<(String, Term)> {
        ps.iter().map(|(n, t)| (n.to_string(), t.clone())).collect()
    };

    // `?P a =?= t` — the motive constraint `▸`/`Eq.subst`/`congrArg` raise. The solution
    // `?P := λx. t` is unique inside the pattern fragment.
    let l = lctx(&[("x", g("Nat"))]);
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    assert!(
        unify_in(
            &ctx,
            &mut m,
            &l,
            &app(mv.clone(), var("x")),
            &app(g("Succ"), var("x"))
        ),
        "?P x =?= Succ x must solve as the motive λx. Succ x"
    );
    assert_eq!(
        normalize(
            &ctx,
            &logicaffeine_kernel::instantiate(&app(mv, g("Zero")), &m)
        ),
        app(g("Succ"), g("Zero")),
        "the solved motive computes"
    );

    // The SAME constraint with the sides swapped must solve too — `try_pattern` runs in both
    // directions, which is what lets a motive on either side of `=?=` be recovered.
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    assert!(
        unify_in(&ctx, &mut m, &l, &var("x"), &app(mv, var("x"))),
        "a pattern on the RIGHT is solved as well as one on the left"
    );

    // TWO distinct bound variables — still inside the fragment, and the permutation is kept.
    let l2 = lctx(&[("x", g("Nat")), ("y", g("Bool"))]);
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    assert!(unify_in(
        &ctx,
        &mut m,
        &l2,
        &app(app(mv.clone(), var("x")), var("y")),
        &app(app(g("f"), var("y")), var("x"))
    ));
    let applied = app(app(mv, g("Zero")), g("true"));
    assert_eq!(
        normalize(&ctx, &logicaffeine_kernel::instantiate(&applied, &m)),
        app(app(g("f"), g("true")), g("Zero")),
        "the argument permutation is preserved by the solution"
    );

    // η: a function and its expansion unify, in both directions, and η-expansion under a
    // binder solves a metavariable (`λx. ?M x =?= Succ`).
    let eta = lam("x", g("Nat"), app(g("Succ"), var("x")));
    let mut m = MetaCtx::new();
    assert!(unify(&ctx, &mut m, &eta, &g("Succ")), "λx. Succ x ≡ Succ");
    let mut m = MetaCtx::new();
    assert!(unify(&ctx, &mut m, &g("Succ"), &eta), "η is symmetric");
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    assert!(
        unify(
            &ctx,
            &mut m,
            &lam("x", g("Nat"), app(mv.clone(), var("x"))),
            &g("Succ")
        ),
        "η + pattern solves ?M"
    );
    if let Term::Var(name) = &mv {
        let sol = m.solution(name).expect("?M is solved");
        assert!(
            defeq_for_test(&ctx, sol, &g("Succ")),
            "?M ≡ Succ, got {sol}"
        );
    }
    // η must not equate genuinely different functions.
    let mut m = MetaCtx::new();
    assert!(
        !unify(&ctx, &mut m, &eta, &g("Zero")),
        "η must not unify Succ with Zero"
    );

    // The occurs-check aborts a cyclic assignment rather than looping or guessing.
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    assert!(
        !unify_in(
            &ctx,
            &mut m,
            &l,
            &app(mv.clone(), var("x")),
            &app(g("Succ"), app(mv, var("x")))
        ),
        "?M x := Succ (?M x) is cyclic and must be refused"
    );

    // An out-of-scope right-hand side is unsolvable: `λx. y` would leave `y` free.
    let l3 = lctx(&[("x", g("Nat")), ("y", g("Nat"))]);
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    assert!(
        !unify_in(&ctx, &mut m, &l3, &app(mv, var("x")), &var("y")),
        "a solution that would leave a free variable must be refused"
    );

    // Non-patterns (a repeated argument, a non-variable argument) are DECLINED rather than
    // guessed — outside the fragment there is no unique solution to commit to.
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    assert!(
        !unify_in(
            &ctx,
            &mut m,
            &l,
            &app(app(mv, var("x")), var("x")),
            &var("x")
        ),
        "?M x x is not a Miller pattern and must not be solved by guessing"
    );

    // And the motive of a DEPENDENT match is the same Miller solution of `?P disc =?= T`,
    // recovered by abstracting the discriminant out of the expected type.
    let mut repl = Repl::new();
    repl.execute("Inductive Cnt := z : Cnt | s : Cnt -> Cnt.")
        .expect("declare Cnt");
    repl.execute(
        "Definition P : Cnt -> Type := fun n : Cnt => match n with | z => Cnt | s k => Bool end.",
    )
    .expect("declare the dependent family");
    repl.execute(
        "Definition elim_p : forall n : Cnt, P n := \
         fun n : Cnt => match n with | z => z | s k => true end.",
    )
    .expect("the dependent motive λn. P n is inferred");
    assert_eq!(repl.execute("Eval (elim_p z).").unwrap().trim(), "z");
    assert_eq!(repl.execute("Eval (elim_p (s z)).").unwrap().trim(), "true");
}

// ===========================================================================
// T05-015 · POSTPONEMENT — fixed phases, no queue, no default_instance
// ===========================================================================

#[test]
fn a_postponed_elaboration_problem_is_retried_when_its_type_is_known() {
    let mut ctx = std_ctx();
    with_sizeable(&mut ctx);

    // What LOGOS *does* have: three fixed phases. Instance resolution runs after the explicit
    // arguments AND after the expected type, so a class variable pinned by either is solved
    // before the instance database is consulted.
    ctx.add_structure("Defaultable", &[("A", ty0())], &[("dflt", var("A"))]);
    ctx.add_instance(
        app(g("Defaultable"), g("Nat")),
        apps(g("Defaultable_mk"), &[g("Nat"), g("Zero")]),
    );
    ctx.add_instance(
        app(g("Defaultable"), g("Bool")),
        apps(g("Defaultable_mk"), &[g("Bool"), g("false")]),
    );
    let dflt_ty = pi(
        "A",
        ty0(),
        pi("i", app(g("Defaultable"), var("A")), var("A")),
    );
    ctx.add_declaration("defaultOf", dflt_ty.clone());
    let mut m = MetaCtx::new();
    let (_, ty) = elaborate_app_against(
        &ctx,
        &mut m,
        &g("defaultOf"),
        &dflt_ty,
        &[ParamKind::Implicit, ParamKind::Instance],
        &[],
        Some(&g("Bool")),
    )
    .expect("the expected-type phase runs before instance resolution");
    assert_eq!(
        ty,
        g("Bool"),
        "(defaultOf : Bool) resolves through the third phase"
    );

    // What Lean's QUEUE has and LOGOS does not: a lambda elaborated against an unknown
    // expected type, POSTPONED, and re-run once the expected type arrives. `fun x => x + 1`
    // has no type at all until the queue drains.
    let unannotated = TermParser::parse("fun x => Succ x");
    assert!(
        unannotated.is_ok(),
        "a lambda whose parameter type is postponed until the expected type is known must be \
         expressible: `fun x => Succ x` — got {unannotated:?}"
    );
    let elab = surface_elaborate_against(
        &ctx,
        &unannotated.unwrap(),
        Some(&arrow(g("Nat"), g("Nat"))),
    )
    .expect("the postponed lambda is re-run against its expected type");
    assert_eq!(
        elab,
        lam("x", g("Nat"), app(g("Succ"), var("x"))),
        "the postponed parameter type is filled in from the expected type"
    );

    // And `@[default_instance]`: with nothing constraining it, Lean's defaulting table lands a
    // bare numeral at `Nat`. LOGOS's instance system is a flat first-match `Vec` with no
    // default-instance notion at all.
    let mut repl = Repl::new();
    repl.execute("Definition addOne : Int -> Int := fun x : Int => add x 1.")
        .expect("addOne");
    let defaulted = repl.execute("Definition bare := 5.");
    assert!(
        defaulted.is_ok(),
        "a numeral with no ascription must land somewhere by defaulting: {defaulted:?}"
    );
}

// ===========================================================================
// T05-016 · `..` ELLIPSIS — sugar for "fill the rest with `_`"
// ===========================================================================

#[test]
fn an_ellipsis_fills_the_remaining_fields_with_solved_holes() {
    let mut ctx = std_ctx();
    ctx.add_structure(
        "Three",
        &[],
        &[("x", g("Nat")), ("y", g("Nat")), ("z", g("Nat"))],
    );

    // The invariant the ellipsis must respect: `{name}_mk` takes EVERY field, so a short spine
    // is a partial application of a function type, not a value of the structure.
    assert_eq!(
        infer_type(&ctx, &apps(g("Three_mk"), &[g("Zero"), g("Zero")])).unwrap(),
        pi("z", g("Nat"), g("Three")),
        "a two-argument spine against a three-field constructor is a function, not a Three"
    );

    // A trailing `..` with nothing left to fill is a no-op: the same term as the ellipsis-free
    // literal, full arity, no `Term::Hole`.
    let ellipsis = TermParser::parse("⟨Zero, Zero, Zero, ..⟩");
    assert!(
        ellipsis.is_ok(),
        "a structure literal's trailing `..` must be expressible: {ellipsis:?}"
    );
    let elab = surface_elaborate_against(&ctx, &ellipsis.unwrap(), Some(&g("Three")))
        .expect("the ellipsis form elaborates");
    assert_eq!(
        elab,
        apps(g("Three_mk"), &[g("Zero"), g("Zero"), g("Zero")]),
        "`..` expands to the full-arity constructor application"
    );
    assert!(
        !has_hole(&elab),
        "no Term::Hole may survive an ellipsis: {elab}"
    );

    // A trailing APPLICATION ellipsis whose holes are solvable from the expected type —
    // Lean's `@List.nil ..`.
    let trailing = TermParser::parse("TNil ..").expect("`f ..` parses");
    let elab = surface_elaborate_against(&ctx, &trailing, Some(&app(g("TList"), g("Nat"))))
        .expect("elaborates");
    assert_eq!(
        elab,
        app(g("TNil"), g("Nat")),
        "`TNil ..` is solved from the expected type"
    );
}

// ===========================================================================
// T05-017 · `OfNat` — LOGOS literals are typed by KIND, not by instance
// ===========================================================================

#[test]
fn a_numeral_elaborates_at_a_user_type_through_its_instance() {
    let mut ctx = std_ctx();

    // The built-in kinds: a `Literal::Nat` is a `Nat` and a `Literal::Int` an `Int`, decided
    // by the literal's KIND with the surrounding context never consulted. And `Nat`'s Peano
    // bridge makes `Literal::Nat(n)` DEFINITIONALLY `Succ^n Zero` — so a LOGOS numeral
    // computes in the kernel where Lean's `OfNat.ofNat` is an instance projection.
    assert_eq!(
        infer_type(&ctx, &nat(5)).unwrap(),
        g("Nat"),
        "a Nat literal is a Nat"
    );
    assert_eq!(
        infer_type(&ctx, &int(5)).unwrap(),
        g("Int"),
        "an Int literal is an Int"
    );
    assert!(
        defeq_for_test(&ctx, &nat(2), &app(g("Succ"), app(g("Succ"), g("Zero")))),
        "the Peano bridge: Nat(2) ≡ Succ (Succ Zero)"
    );
    // A literal beyond i64 still works — `Literal::Nat` is BigInt-backed.
    let huge = logicaffeine_kernel::BigInt::parse_decimal("123456789012345678901234567890")
        .expect("a literal beyond i64");
    assert_eq!(
        infer_type(&ctx, &Term::Lit(Literal::Nat(huge))).unwrap(),
        g("Nat")
    );

    // A numeral at a USER type. `structure MyNum where v : Nat` + `instance : OfNat MyNum n`
    // makes `(3 : MyNum)` elaborate to `@OfNat.ofNat MyNum 3 inst`. LOGOS has no `OfNat` hook
    // on `Term::Lit`, so the expected type cannot retype a literal.
    ctx.add_structure("MyNum", &[], &[("v", g("Nat"))]);
    let at_user = surface_elaborate_against(&ctx, &nat(3), Some(&g("MyNum")))
        .expect("`3 : MyNum` elaborates");
    assert_eq!(
        infer_type(&ctx, &at_user).unwrap(),
        g("MyNum"),
        "a numeral must elaborate at a user type through its instance, not stay a Nat: {at_user}"
    );
    assert_eq!(
        normalize(&ctx, &app(g("MyNum_v"), at_user)),
        nat(3),
        "and the instance carries the literal's value into the user type"
    );

    // A negative `Nat` literal has no meaning and must be refused outright, never collapsed.
    assert!(
        infer_type(&ctx, &Term::Lit(Literal::Nat((-1i64).into()))).is_err(),
        "a negative Nat literal has no meaning and must be rejected, never collapsed to Zero"
    );
}

// ===========================================================================
// T05-018 · TYPE ASCRIPTION — erased, but it CONSTRAINS what it ascribes
// ===========================================================================

#[test]
fn a_type_ascription_constrains_the_term_it_ascribes() {
    // The ascription mechanism that DOES work: the declared type is an INPUT to elaboration,
    // and it leaves no node behind.
    let mut repl = Repl::new();
    repl.execute("Definition tnil : {A : Type} -> TList A := fun A : Type => TNil A.")
        .expect("define tnil");
    repl.execute("Definition empty : TList Int := tnil.")
        .expect("the ascribed type pins an implicit nothing else determines");
    let body = repl
        .context()
        .get_definition_body("empty")
        .expect("empty is defined")
        .clone();
    assert_eq!(
        body,
        app(g("tnil"), g("Int")),
        "the ascription leaves no wrapper node — no Global(\"id\"), no mdata analogue"
    );

    // An ascription is also a CHECK. `(true : Nat)` is a type error in Lean; the whole point
    // of writing the type is that the term must have it.
    let mistyped = repl.execute("Definition bad : Nat := true.");
    assert!(
        mistyped.is_err(),
        "an ascribed type must be CHECKED against the term: `Definition bad : Nat := true.` \
         was accepted and `bad` is now registered at a type its body does not have"
    );

    // The same hole at a Prop, where it is not a nuisance but an inconsistency: `I : True`
    // ascribed at `False` would put a `False` inhabitant into the environment.
    let mut fresh = Repl::new();
    assert!(
        fresh.execute("Definition oops : False := I.").is_err(),
        "an ascription must never register a term at a proposition it does not prove"
    );

    // And an ascription DRIVES coercion insertion: `(↑true : Nat)` must contain the coercion,
    // not the bare `Bool`.
    let mut coe = Repl::new();
    coe.execute("Definition boolToNat : Bool -> Nat := fun b : Bool => Zero.")
        .expect("define");
    coe.context_mut()
        .add_coercion(g("Bool"), g("Nat"), g("boolToNat"));
    coe.execute("Definition coerced : Nat := true.")
        .expect("the ascription inserts the coercion");
    assert_eq!(
        coe.context()
            .get_definition_body("coerced")
            .expect("coerced is defined"),
        &app(g("boolToNat"), g("true")),
        "the ascribed type is what forces the coercion in"
    );
}

// ===========================================================================
// T05-019 · `do` NOTATION — OUT OF SCOPE, and refused cleanly
// ===========================================================================

/// **OUT.** `do` is an effectful *programming* surface, and LOGOS deliberately keeps effects
/// out of the CIC core: the kernel is the mathematics engine, and the imperative language
/// (interpreter / VM / JIT / AOT / WASM tiers) is where sequencing, mutation and `for` loops
/// already live. Lean's do-elaborator is a CPS transformation whose OUTPUT is a tower of
/// `Bind.bind` / `Pure.pure` / `ForIn.forIn` applications over a monad hierarchy that has no
/// place in a proof kernel. So the obligation here is not to implement it but to refuse it
/// without half-working: no `Global` for the absent operations, and above all no axiom
/// standing in for one — an axiomatised `bind`/`pure` never reduces, so every `rfl` the source
/// proved by running the block silently stops holding. This test pins the clean refusal, and
/// pins that the refusal does not over-reach: an ordinary pure `let` is untouched.
#[test]
fn do_notation_is_refused_and_never_half_translated() {
    let ctx = std_ctx();

    // None of the operations a `do` block elaborates to exist, under any spelling.
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
            "`{name}` must not exist — a do-block operation smuggled in as a global or an axiom"
        );
    }
    // `Option` is not one of them: it is Lean's ordinary `Init/Prelude` DATA type, and the
    // prelude has it as exactly that — an inductive with Lean's two constructors, no axiom.
    assert!(
        ctx.is_inductive("Option"),
        "`Option` is data, not a do-block operation"
    );
    assert_eq!(
        ctx.get_constructors("Option")
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        vec!["none", "some"],
        "`Option` has Lean's `none` and `some`"
    );
    assert!(
        !ctx.iter_declarations().any(|(name, _)| name == "Option"),
        "`Option` is not an assumption"
    );

    // And a term that mentions one is REFUSED, not silently accepted as a free variable.
    let block = apps(g("Bind_bind"), &[g("Zero"), lam("a", g("Nat"), g("Zero"))]);
    assert!(
        matches!(
            infer_type(&ctx, &block),
            Err(KernelError::UnboundVariable(_))
        ),
        "an elaborated do-block must fail loudly on its absent globals, got {:?}",
        infer_type(&ctx, &block)
    );

    // The refusal must NOT over-reach: an ordinary pure `let` — which a `do` block also
    // desugars through `pure` — is a first-class, zeta-transparent LOGOS term.
    let pure_let = Term::Let {
        name: "a".to_string(),
        ty: Box::new(g("Nat")),
        value: Box::new(nat(1)),
        body: Box::new(app(g("Succ"), var("a"))),
    };
    assert_eq!(
        infer_type(&ctx, &pure_let).unwrap(),
        g("Nat"),
        "a pure let is not a do block"
    );
    assert!(
        defeq_for_test(&ctx, &normalize(&ctx, &pure_let), &nat(2)),
        "and it computes: let a := 1 in Succ a ≡ 2"
    );
}

// ===========================================================================
// T05-020 · MONADIC BIND and `IO` — OUT OF SCOPE, for a deeper reason
// ===========================================================================

/// **OUT.** T05-019 refuses because the do-elaborator's output names things LOGOS lacks; this
/// case refuses because they cannot be *supplied*. A `Monad` class is parameterised by
/// `m : Type u → Type v` and therefore lives above `Type 0`, while `add_structure` hard-codes
/// every structure's sort to `Type 0` — so declaring it would register a type whose stated
/// sort is a lie, and LOGOS's cumulativity would let the lie propagate silently. `IO` is worse
/// still: it is `EStateM IO.Error IO.RealWorld` over an opaque world token whose primitives
/// are `@[extern]` runtime calls with no defining equations — there is no term to translate,
/// not merely a term LOGOS cannot hold. Axiomatising it would assert inhabitants of arbitrary
/// types. A mathematics kernel loses nothing by declining; it would lose its consistency by
/// pretending. The refusal must stay precise, so this test also pins that `Decidable` — a
/// `Type 0`-valued, one-parameter class that IS in the prelude — is not swept up by it.
#[test]
fn monadic_bind_and_io_are_refused_rather_than_axiomatised() {
    let ctx = std_ctx();

    for name in [
        "Monad",
        "Bind",
        "Pure",
        "Functor",
        "Seq",
        "IO",
        "EStateM",
        "IO_RealWorld",
    ] {
        assert!(
            ctx.get_global(name).is_none(),
            "`{name}` must not exist in the kernel context"
        );
    }

    // Nothing in the standard library smuggles one in under a flattened name, as an axiom, a
    // definition or an inductive — the guard the corpus cares about most.
    let banned = [
        "Monad",
        "Bind",
        "Pure",
        "Functor",
        "Applicative",
        "EStateM",
        "RealWorld",
    ];
    let mentions = |n: &str| banned.iter().any(|b| n.contains(b));
    for (n, _) in ctx.iter_declarations() {
        assert!(
            !mentions(n),
            "the prelude must declare no monad axiom, found `{n}`"
        );
    }
    for (n, _, _) in ctx.iter_definitions() {
        assert!(
            !mentions(n),
            "the prelude must define no monad operation, found `{n}`"
        );
    }
    for (n, _) in ctx.iter_inductives() {
        assert!(
            !mentions(n),
            "the prelude must declare no monad inductive, found `{n}`"
        );
    }
    assert!(
        !ctx.iter_definitions()
            .any(|(n, _, _)| n == "IO" || n.starts_with("IO_")),
        "there is no `IO` and no `IO.*` primitive"
    );

    // `x >>= f` and `f <$> x` used OUTSIDE a do block must be refused for the same reason —
    // the rejection is keyed on the absent head constants, not on a `do` keyword.
    let bind_ex = apps(g("Bind_bind"), &[g("Zero"), g("Succ")]);
    assert!(
        matches!(
            infer_type(&ctx, &bind_ex),
            Err(KernelError::UnboundVariable(_))
        ),
        "`>>=` outside a do block is refused too"
    );

    // NOT over-rejected: `Decidable` is a real, `Type 0`-valued, one-parameter prelude class,
    // and `decide` computes. A rule that rejects "classes over type constructors" must not
    // catch it.
    assert!(
        ctx.get_global("Decidable").is_some(),
        "Decidable is in the prelude"
    );
    assert!(ctx.get_global("isTrue").is_some() && ctx.get_global("isFalse").is_some());
    let decided = app(
        app(
            g("decide"),
            apps(g("Eq"), &[g("Nat"), g("Zero"), g("Zero")]),
        ),
        apps(g("decEqNat"), &[g("Zero"), g("Zero")]),
    );
    assert_eq!(
        normalize(&ctx, &decided),
        g("true"),
        "decide (0 = 0) ⇝ true — the prelude's own decidability, untouched by the monad refusal"
    );
}
