//! The ELABORATOR's own invariants — the negatives and boundaries that the conformance
//! corpus does not name, written before the fixes that make them true.
//!
//! The corpus tiers assert CAPABILITIES ("a class with no instances is still a class",
//! "coercions compose"). Every one of those is a widening: the elaborator starts accepting
//! something it used to refuse. A widening is only safe if the neighbouring thing it must
//! STILL refuse is pinned, and the corpus does not pin those — so they are pinned here.
//!
//! Five widenings are covered, each with the negative that bounds it:
//!
//! 1. **No metavariable escapes.** The elaborator's documented contract is that no
//!    `?`-prefixed `Term::Var` may reach the kernel. `elaborate_app` used to return `Ok`
//!    carrying a live `?3` whenever an instance premise was silently demoted to a type
//!    parameter. The fix is a rejection, so the negative is "an undischargeable premise is an
//!    ERROR", across every shape that can produce one — not just the empty-class shape the
//!    corpus names.
//! 2. **Implicit insertion.** Leaving an unsolvable implicit un-inserted must not stop the
//!    SOLVABLE ones being inserted; the boundary is exactly "something downstream can
//!    determine it".
//! 3. **Transitive coercions.** Composing a chain must not invent a path that is not there,
//!    must terminate on a cyclic coercion set, and must not double-wrap.
//! 4. **`extends` inference.** Deriving a parent edge from a flattened field prefix must not
//!    fire on a near-miss: a reordered field list, a same-named field at a different type, or
//!    a different parameter telescope are all NOT inheritance.
//! 5. **Instance resolution order.** An underdetermined goal resolving differently from a
//!    determined one is the whole point; the negative is that the DETERMINED case is
//!    unchanged — first match over the registration vector, which is a contract other tests
//!    order against.
//!
//! Every widening that touches a kernel term is also checked with `double_check`: a term the
//! elaborator now emits must satisfy BOTH kernels, not just the one that built it.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, double_check, elaborate_app, elaborate_app_against, infer_type, normalize,
    resolve_coercion, resolve_instance, surface_elaborate, Context, DoubleCheck, MetaCtx,
    ParamKind, Term, Universe,
};

// ============================================================================================
// HELPERS — per-file by house rule.
// ============================================================================================

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
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn nat() -> Term {
    g("Nat")
}
fn peano(n: u32) -> Term {
    (0..n).fold(g("Zero"), |acc, _| app(g("Succ"), acc))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// Whether `t` still carries an elaborator metavariable — a `Term::Var` whose name starts
/// with `?`. The kernel treats one as an ordinary free variable, so a survivor is at best an
/// `UnboundVariable` far from its cause and at worst captured by a same-named binder.
fn has_meta(t: &Term) -> bool {
    match t {
        Term::Var(n) => n.starts_with('?'),
        Term::App(f, a) => has_meta(f) || has_meta(a),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => has_meta(param_type) || has_meta(body_type),
        Term::Lambda {
            param_type, body, ..
        } => has_meta(param_type) || has_meta(body),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => has_meta(discriminant) || has_meta(motive) || cases.iter().any(has_meta),
        Term::Fix { body, .. } => has_meta(body),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| has_meta(b)),
        Term::Let {
            ty, value, body, ..
        } => has_meta(ty) || has_meta(value) || has_meta(body),
        Term::Annotated { term, .. } => has_meta(term),
        Term::Sort(_) | Term::Global(_) | Term::Const { .. } | Term::Lit(_) | Term::Hole => false,
    }
}

fn register_instance(ctx: &mut Context, name: &str, ty: Term, value: Term) {
    ctx.add_definition(name.to_string(), ty.clone(), value);
    ctx.add_instance(ty, g(name));
}

fn resolve(ctx: &Context, goal: &Term) -> Option<Term> {
    let mut mctx = MetaCtx::new();
    resolve_instance(ctx, &mut mctx, goal)
}

/// `Sized (α) where size : α → Nat`, plus `sizeOf : Π{α}. [Sized α] → α → Nat`. Returns
/// `sizeOf`'s type so the elaboration tests can drive `elaborate_app` with it.
fn register_sized(ctx: &mut Context) -> Term {
    ctx.add_structure(
        "Sized",
        &[("a", ty0())],
        &[("size", arrow(var("a"), nat()))],
    );
    let ty = pi(
        "a",
        ty0(),
        arrow(app(g("Sized"), var("a")), arrow(var("a"), nat())),
    );
    let body = lam(
        "a",
        ty0(),
        lam(
            "s",
            app(g("Sized"), var("a")),
            lam(
                "x",
                var("a"),
                apps(g("Sized_size"), &[var("a"), var("s"), var("x")]),
            ),
        ),
    );
    ctx.add_definition("sizeOf".to_string(), ty.clone(), body);
    ty
}

/// `Box (α : Type) := Box_mk (α)` — the unary type constructor the recursive instances go
/// through.
fn register_box(ctx: &mut Context) {
    ctx.add_indexed_inductive("Box", arrow(ty0(), ty0()), 1);
    ctx.add_constructor(
        "Box_mk",
        "Box",
        pi("a", ty0(), arrow(var("a"), app(g("Box"), var("a")))),
    );
}

// ============================================================================================
// 1 · NO METAVARIABLE ESCAPES — the elaborator's own documented contract
// ============================================================================================

/// The contract, stated in `elaborate.rs`'s header: elaboration produces a FULLY EXPLICIT,
/// metavariable-free kernel term. `elaborate_app` violated it whenever an instance's telescope
/// carried a parameter nothing solved — the parameter was freshened to `?n`, never bound, and
/// the `Ok` term carried it into `infer_type`, which reported `UnboundVariable` for a name the
/// user never wrote.
///
/// The corpus names ONE shape (a premise whose class has no registered instance). This pins
/// the contract for every shape that produces an undetermined parameter, because a fix aimed
/// at the named shape alone would leave the others silently broken:
///
/// * a premise headed by a declared class that has no instance (the corpus's shape),
/// * a premise headed by something that is not a class at all,
/// * a PHANTOM parameter — one the instance's conclusion never mentions, so unifying the
///   conclusion cannot possibly determine it.
///
/// In every case the demand is the same: `Err`, or an `Ok` whose term is metavariable-free AND
/// certified by both kernels. There is no third answer.
#[test]
fn no_shape_of_undetermined_parameter_escapes_as_a_live_metavariable() {
    let shapes: Vec<(&str, Box<dyn Fn(&mut Context, &Term)>)> = vec![
        (
            "a premise headed by a class with no registered instance",
            Box::new(|ctx: &mut Context, box_a: &Term| {
                ctx.add_structure(
                    "Lonely",
                    &[("a", ty0())],
                    &[("witness", arrow(var("a"), var("a")))],
                );
                let ty = pi(
                    "a",
                    ty0(),
                    arrow(app(g("Lonely"), var("a")), app(g("Sized"), box_a.clone())),
                );
                let body = lam(
                    "a",
                    ty0(),
                    lam(
                        "l",
                        app(g("Lonely"), var("a")),
                        apps(
                            g("Sized_mk"),
                            &[box_a.clone(), lam("_", box_a.clone(), peano(1))],
                        ),
                    ),
                );
                register_instance(ctx, "sizedBoxLonely", ty, body);
            }),
        ),
        (
            "a premise headed by a global that is not a class at all",
            Box::new(|ctx: &mut Context, box_a: &Term| {
                ctx.add_inductive("Token", ty0());
                let ty = pi(
                    "a",
                    ty0(),
                    arrow(g("Token"), app(g("Sized"), box_a.clone())),
                );
                let body = lam(
                    "a",
                    ty0(),
                    lam(
                        "t",
                        g("Token"),
                        apps(
                            g("Sized_mk"),
                            &[box_a.clone(), lam("_", box_a.clone(), peano(1))],
                        ),
                    ),
                );
                register_instance(ctx, "sizedBoxToken", ty, body);
            }),
        ),
        (
            "a PHANTOM parameter the conclusion never mentions",
            Box::new(|ctx: &mut Context, box_a: &Term| {
                // `instance {α} {β} : Sized (Box α)` — β appears nowhere in the conclusion, so
                // unifying `Sized (Box Nat)` cannot determine it.
                let ty = pi("a", ty0(), pi("b", ty0(), app(g("Sized"), box_a.clone())));
                let body = lam(
                    "a",
                    ty0(),
                    lam(
                        "b",
                        ty0(),
                        apps(
                            g("Sized_mk"),
                            &[box_a.clone(), lam("_", box_a.clone(), peano(1))],
                        ),
                    ),
                );
                register_instance(ctx, "sizedBoxPhantom", ty, body);
            }),
        ),
    ];

    for (what, build) in shapes {
        let mut ctx = std_ctx();
        let size_of_ty = register_sized(&mut ctx);
        register_box(&mut ctx);
        register_instance(
            &mut ctx,
            "sizedNat",
            app(g("Sized"), nat()),
            apps(g("Sized_mk"), &[nat(), lam("n", nat(), var("n"))]),
        );
        let box_a = app(g("Box"), var("a"));
        build(&mut ctx, &box_a);

        let mut mctx = MetaCtx::new();
        let elaborated = elaborate_app(
            &ctx,
            &mut mctx,
            &g("sizeOf"),
            &size_of_ty,
            &[
                ParamKind::Implicit,
                ParamKind::Instance,
                ParamKind::Explicit,
            ],
            &[apps(g("Box_mk"), &[nat(), peano(5)])],
        );
        match &elaborated {
            Err(_) => {}
            Ok((term, _)) => {
                assert!(
                    !has_meta(term),
                    "{what}: elaboration returned Ok with a live metavariable in it: {term}",
                );
                assert!(
                    infer_type(&ctx, term).is_ok(),
                    "{what}: an elaborated term must be certified by the kernel, got {:?} for \
                     {term}",
                    infer_type(&ctx, term),
                );
                assert!(
                    matches!(double_check(&ctx, term), DoubleCheck::Agreed),
                    "{what}: and by BOTH kernels: {term}",
                );
            }
        }
    }
}

/// The boundary of the rejection: it must not swallow the instances that DO resolve. The same
/// `Box` instance with a premise the database can discharge still resolves, still nests
/// structurally, and still computes — so the guard is a filter on undetermined parameters, not
/// on premise-bearing instances as a class.
#[test]
fn rejecting_undetermined_parameters_does_not_reject_dischargeable_premises() {
    let mut ctx = std_ctx();
    let size_of_ty = register_sized(&mut ctx);
    register_box(&mut ctx);
    register_instance(
        &mut ctx,
        "sizedNat",
        app(g("Sized"), nat()),
        apps(g("Sized_mk"), &[nat(), lam("n", nat(), var("n"))]),
    );
    let box_a = app(g("Box"), var("a"));
    register_instance(
        &mut ctx,
        "sizedBox",
        pi(
            "a",
            ty0(),
            arrow(app(g("Sized"), var("a")), app(g("Sized"), box_a.clone())),
        ),
        lam(
            "a",
            ty0(),
            lam(
                "s",
                app(g("Sized"), var("a")),
                apps(
                    g("Sized_mk"),
                    &[box_a.clone(), lam("_", box_a.clone(), peano(1))],
                ),
            ),
        ),
    );

    let mut mctx = MetaCtx::new();
    let (term, ty) = elaborate_app(
        &ctx,
        &mut mctx,
        &g("sizeOf"),
        &size_of_ty,
        &[
            ParamKind::Implicit,
            ParamKind::Instance,
            ParamKind::Explicit,
        ],
        &[apps(g("Box_mk"), &[nat(), peano(5)])],
    )
    .expect("a premise the database CAN discharge must still resolve");
    assert_eq!(
        term,
        apps(
            g("sizeOf"),
            &[
                app(g("Box"), nat()),
                apps(g("sizedBox"), &[nat(), g("sizedNat")]),
                apps(g("Box_mk"), &[nat(), peano(5)]),
            ],
        ),
        "the resolved premise stays structured — the base instance appears by name",
    );
    assert_eq!(ty, nat());
    assert!(
        !has_meta(&term),
        "and nothing undetermined is left behind: {term}"
    );
    assert!(
        matches!(double_check(&ctx, &term), DoubleCheck::Agreed),
        "both kernels agree"
    );
}

// ============================================================================================
// 2 · IMPLICIT INSERTION — inserted exactly when something can determine it
// ============================================================================================

/// Not inserting an implicit that nothing can solve is a widening of what elaboration ACCEPTS
/// (the application stops early instead of emitting `f ?0`), so the negative is that the
/// solvable positions are untouched. Four positions, one rule: an implicit is filled iff an
/// explicit argument still follows it or an expected type is supplied.
#[test]
fn an_implicit_is_filled_exactly_when_something_can_determine_it() {
    let mut ctx = std_ctx();
    // `idf : Π{A:Type}. A → A` and `nilOf : Π{A:Type}. TList A`.
    let idf_ty = pi("A", ty0(), arrow(var("A"), var("A")));
    ctx.add_declaration("idf", idf_ty.clone());
    let nil_ty = pi("A", ty0(), app(g("TList"), var("A")));
    ctx.add_declaration("nilOf", nil_ty.clone());

    // SOLVABLE, from a following explicit argument.
    let mut m = MetaCtx::new();
    let (e, t) = elaborate_app(
        &ctx,
        &mut m,
        &g("idf"),
        &idf_ty,
        &[ParamKind::Implicit, ParamKind::Explicit],
        &[g("Zero")],
    )
    .expect("an explicit argument follows, so the implicit is inserted and solved");
    assert_eq!(
        e,
        apps(g("idf"), &[nat(), g("Zero")]),
        "the implicit is solved from the argument"
    );
    assert_eq!(t, nat());
    assert!(!has_meta(&e));

    // SOLVABLE, from the expected type, with no value argument at all.
    let mut m = MetaCtx::new();
    let (e, t) = elaborate_app_against(
        &ctx,
        &mut m,
        &g("nilOf"),
        &nil_ty,
        &[ParamKind::Implicit],
        &[],
        Some(&app(g("TList"), nat())),
    )
    .expect("the expected type determines the implicit");
    assert_eq!(e, app(g("nilOf"), nat()), "the expected type supplies it");
    assert_eq!(t, app(g("TList"), nat()));
    assert!(!has_meta(&e));

    // UNSOLVABLE — nothing follows and no expected type. The term must be the function itself,
    // NOT `nilOf ?0`: a metavariable nothing can ever solve is strictly worse than a value that
    // is still waiting for its argument.
    let mut m = MetaCtx::new();
    let (e, t) = elaborate_app(
        &ctx,
        &mut m,
        &g("nilOf"),
        &nil_ty,
        &[ParamKind::Implicit],
        &[],
    )
    .expect(
        "an implicit nothing can determine leaves the function polymorphic rather than failing",
    );
    assert_eq!(
        e,
        g("nilOf"),
        "the bare function, with its implicit still to come"
    );
    assert!(!has_meta(&e), "and certainly no metavariable: {e}");
    assert_eq!(
        t, nil_ty,
        "the reported type is still the full Π, so a caller can go on applying it",
    );

    // UNSOLVABLE IN TRAILING POSITION — the explicit arguments are exhausted before the
    // implicit is reached. Same rule, and the same reason.
    // `pairOf : Π(a:Nat). Π{B:Type}. Nat` — B follows the explicit argument.
    let pair_ty = pi("a", nat(), pi("B", ty0(), nat()));
    ctx.add_declaration("pairOf", pair_ty.clone());
    let mut m = MetaCtx::new();
    let (e, _) = elaborate_app(
        &ctx,
        &mut m,
        &g("pairOf"),
        &pair_ty,
        &[ParamKind::Explicit, ParamKind::Implicit],
        &[g("Zero")],
    )
    .expect("the explicit argument is consumed; the trailing implicit is left");
    assert_eq!(
        e,
        app(g("pairOf"), g("Zero")),
        "the trailing unsolvable implicit is not inserted"
    );
    assert!(!has_meta(&e));

    // …and it IS inserted once an expected type can pin it.
    let mut m = MetaCtx::new();
    let (e, _) = elaborate_app_against(
        &ctx,
        &mut m,
        &g("pairOf"),
        &pair_ty,
        &[ParamKind::Explicit, ParamKind::Implicit],
        &[g("Zero")],
        Some(&nat()),
    )
    .expect("an expected type reaches the trailing implicit");
    assert!(
        matches!(&e, Term::App(f, _) if matches!(&**f, Term::App(..))),
        "with an expected type the trailing implicit IS inserted: {e}",
    );

    // And the rule must not fire on an EXPLICIT position: too few explicit arguments is still a
    // hard error, never a quiet partial application.
    let mut m = MetaCtx::new();
    assert!(
        elaborate_app(
            &ctx,
            &mut m,
            &g("idf"),
            &idf_ty,
            &[ParamKind::Implicit, ParamKind::Explicit],
            &[]
        )
        .is_err(),
        "a MISSING EXPLICIT argument is an error — only implicits may be left unfilled",
    );
}

// ============================================================================================
// 3 · TRANSITIVE COERCIONS — composed, never invented, always terminating
// ============================================================================================

/// Composing `Coe` steps widens what elaboration accepts, so the negatives are: a path that
/// does not exist is not invented, a CYCLIC coercion set terminates instead of searching
/// forever, and an argument that already has the expected type is not wrapped at all.
#[test]
fn a_coercion_chain_composes_and_never_invents_a_path() {
    let mut ctx = std_ctx();
    // A four-node chain `D → C → B → Nat`, registered OUT of path order so nothing rides on
    // the registration sequence.
    for (n, sort) in [("Bnode", ty0()), ("Cnode", ty0()), ("Dnode", ty0())] {
        ctx.add_inductive(n, sort);
        ctx.add_constructor(&format!("{n}_mk"), n, g(n));
    }
    ctx.add_declaration("bToNat", arrow(g("Bnode"), nat()));
    ctx.add_declaration("cToB", arrow(g("Cnode"), g("Bnode")));
    ctx.add_declaration("dToC", arrow(g("Dnode"), g("Cnode")));
    ctx.add_coercion(g("Cnode"), g("Bnode"), g("cToB"));
    ctx.add_coercion(g("Dnode"), g("Cnode"), g("dToC"));
    ctx.add_coercion(g("Bnode"), nat(), g("bToNat"));
    ctx.add_declaration("takesNat", arrow(nat(), nat()));

    // THREE steps, applied in path order, innermost first.
    let three = surface_elaborate(&ctx, &app(g("takesNat"), g("Dnode_mk")))
        .expect("a three-step chain elaborates");
    assert_eq!(
        three,
        app(
            g("takesNat"),
            app(g("bToNat"), app(g("cToB"), app(g("dToC"), g("Dnode_mk"))))
        ),
        "the composite is the three coercions applied in order, not a nest of β-redexes",
    );
    assert_eq!(
        infer_type(&ctx, &three).unwrap(),
        nat(),
        "and the composite type-checks"
    );
    assert!(
        matches!(double_check(&ctx, &three), DoubleCheck::Agreed),
        "both kernels agree"
    );

    // NEVER INVENTED: a node with no outgoing path to `Nat` must not be bridged.
    ctx.add_inductive("Island", ty0());
    ctx.add_constructor("Island_mk", "Island", g("Island"));
    let unreachable = surface_elaborate(&ctx, &app(g("takesNat"), g("Island_mk")))
        .map(|t| infer_type(&ctx, &t).is_ok())
        .unwrap_or(false);
    assert!(
        !unreachable,
        "an unreachable node must stay a type error — a chain search must not manufacture a path",
    );

    // TERMINATION: a cycle in the coercion set must not be searched forever. `Loop ⇄ Ring`,
    // with no exit to `Nat`, is the shape that makes a naive transitive closure diverge.
    let mut cyclic = std_ctx();
    for n in ["LoopT", "RingT"] {
        cyclic.add_inductive(n, ty0());
        cyclic.add_constructor(&format!("{n}_mk"), n, g(n));
    }
    cyclic.add_declaration("loopToRing", arrow(g("LoopT"), g("RingT")));
    cyclic.add_declaration("ringToLoop", arrow(g("RingT"), g("LoopT")));
    cyclic.add_coercion(g("LoopT"), g("RingT"), g("loopToRing"));
    cyclic.add_coercion(g("RingT"), g("LoopT"), g("ringToLoop"));
    cyclic.add_declaration("takesNat", arrow(nat(), nat()));
    let mut m = MetaCtx::new();
    assert!(
        resolve_coercion(&cyclic, &mut m, &g("LoopT"), &nat()).is_none(),
        "a cyclic coercion set with no exit must terminate and report no path",
    );
    // The cycle itself still resolves in one step — terminating is not the same as giving up.
    let mut m = MetaCtx::new();
    assert_eq!(
        resolve_coercion(&cyclic, &mut m, &g("LoopT"), &g("RingT")),
        Some(g("loopToRing")),
        "a one-step coercion inside a cycle is still found, and is the function itself",
    );

    // NO DOUBLE WRAP: an argument that already has the expected type is left exactly alone,
    // even though a path to `Nat` exists from further up the chain.
    let already = app(g("bToNat"), g("Bnode_mk"));
    assert_eq!(
        surface_elaborate(&ctx, &app(g("takesNat"), already.clone())).unwrap(),
        app(g("takesNat"), already),
        "a well-typed argument is never wrapped",
    );
}

/// An ASCRIBED type drives coercion insertion exactly as a parameter type does — `(↑b : Nat)`
/// must contain the coercion, not a `Bool` stamped `Nat`. The negatives: a term that already
/// has the ascribed type is untouched, and a mismatch with no registered path stays a mismatch
/// rather than being papered over.
#[test]
fn an_ascribed_type_drives_the_coercion_and_nothing_else() {
    let mut ctx = std_ctx();
    ctx.add_inductive("Flag", ty0());
    ctx.add_constructor("raised", "Flag", g("Flag"));
    ctx.add_declaration("flagToNat", arrow(g("Flag"), nat()));
    ctx.add_coercion(g("Flag"), nat(), g("flagToNat"));

    // The ascription is the only thing that forces the coercion in: nothing else in
    // `raised` mentions `Nat`.
    let coerced = logicaffeine_kernel::surface_elaborate_against(&ctx, &g("raised"), Some(&nat()))
        .expect("the ascribed type inserts the coercion");
    assert_eq!(
        coerced,
        app(g("flagToNat"), g("raised")),
        "the ascription forces the coercion"
    );
    assert_eq!(
        infer_type(&ctx, &coerced).unwrap(),
        nat(),
        "and the result has the ascribed type"
    );
    assert!(
        matches!(double_check(&ctx, &coerced), DoubleCheck::Agreed),
        "both kernels agree"
    );

    // Already the ascribed type — untouched, no double wrap.
    let already = app(g("flagToNat"), g("raised"));
    assert_eq!(
        logicaffeine_kernel::surface_elaborate_against(&ctx, &already, Some(&nat())).unwrap(),
        already,
        "a term that already has the ascribed type is left exactly alone",
    );

    // No registered path — the ascription must not repair it.
    let mut bare = std_ctx();
    bare.add_inductive("Flag", ty0());
    bare.add_constructor("raised", "Flag", g("Flag"));
    let unrepairable =
        logicaffeine_kernel::surface_elaborate_against(&bare, &g("raised"), Some(&nat()))
            .map(|t| matches!(infer_type(&bare, &t), Ok(ty) if ty == nat()))
            .unwrap_or(false);
    assert!(
        !unrepairable,
        "with no coercion registered the ascription must not be satisfiable"
    );
}

/// `elab_surface`'s application arm falls back to a PLAIN application when typed elaboration
/// does not apply, which is what keeps non-standard shapes working — and which also handed back
/// `Ok(f a)` for an argument whose type was simply wrong, so the failure surfaced later, in
/// `infer_type`, with no mention of the argument that caused it.
///
/// The contract: an elaboration that returns `Ok` returns something the kernel accepts. The
/// boundary the fallback exists for is pinned alongside it — a global application UNDER a
/// binder, whose bound variable `elab_surface` does not carry in its context, must still
/// elaborate rather than be rejected for mentioning a name the kernel cannot see yet.
#[test]
fn an_argument_type_error_is_reported_rather_than_returned_as_a_well_typed_ok() {
    let mut ctx = std_ctx();
    ctx.add_declaration("takesNat", arrow(nat(), nat()));

    let bad = surface_elaborate(&ctx, &app(g("takesNat"), g("true")));
    assert!(
        bad.is_err(),
        "an argument of the wrong type with no coercion must be REPORTED by elaboration, not \
         returned as an `Ok` the kernel rejects later: {bad:?}",
    );

    // THE BOUNDARY. Under a `λ`, the bound variable is not in the kernel context `elab_surface`
    // carries, so its application cannot be type-checked there — and must not be rejected for
    // that. This is the case the permissive fallback exists for.
    let under_binder = surface_elaborate(&ctx, &lam("n", nat(), app(g("takesNat"), var("n"))));
    assert_eq!(
        under_binder.expect("a body mentioning its own binder still elaborates"),
        lam("n", nat(), app(g("takesNat"), var("n"))),
        "an application of a bound variable is not a type error, it is a name the surface pass \
         cannot yet see",
    );

    // And a well-typed application is untouched, coercion or not.
    assert_eq!(
        surface_elaborate(&ctx, &app(g("takesNat"), g("Zero"))).unwrap(),
        app(g("takesNat"), g("Zero")),
        "a correct application elaborates to itself",
    );
}

// ============================================================================================
// 4 · `extends` INFERENCE — a flattened prefix, and nothing weaker
// ============================================================================================

/// Recovering `extends` from a flattened field list is an inference, so it has to be exact:
/// three near-misses must NOT produce a parent edge, or unrelated records would start
/// discharging each other's instance goals. The positive half checks that the derived edge is
/// a real definition satisfying the projection round-trip law — an edge that did not compute
/// would be an axiom in disguise.
#[test]
fn a_parent_edge_is_derived_only_from_a_genuine_flattened_prefix() {
    // POSITIVE: the parent's whole field list is a strict prefix, over the same parameters.
    let mut ctx = std_ctx();
    ctx.add_structure("Par", &[("a", ty0())], &[("base", var("a"))]);
    ctx.add_structure(
        "Chi",
        &[("a", ty0())],
        &[("base", var("a")), ("extra", arrow(var("a"), var("a")))],
    );
    let edge = ctx
        .get_global("Chi_toPar")
        .expect("a flattened prefix derives the parent edge");
    assert_eq!(
        edge,
        &pi(
            "a",
            ty0(),
            arrow(app(g("Chi"), var("a")), app(g("Par"), var("a")))
        ),
        "the edge forgets a Chi to a Par over the shared parameters",
    );
    // It is a DEFINITION, not an axiom: `(c.toPar).base ≡ c.base` must hold by computation.
    let dict = apps(
        g("Chi_mk"),
        &[nat(), peano(3), lam("n", nat(), app(g("Succ"), var("n")))],
    );
    assert_eq!(
        normalize(
            &ctx,
            &apps(
                g("Par_base"),
                &[nat(), apps(g("Chi_toPar"), &[nat(), dict.clone()])]
            )
        ),
        peano(3),
        "(c.toPar).base ≡ c.base — the law that pins the edge as a definition, not an axiom",
    );
    let forgotten = apps(g("Chi_toPar"), &[nat(), dict]);
    assert!(
        infer_type(&ctx, &forgotten).is_ok(),
        "the derived edge type-checks"
    );
    assert!(
        matches!(double_check(&ctx, &forgotten), DoubleCheck::Agreed),
        "and both kernels accept what it builds",
    );

    // NEAR-MISS 1 — the parent's fields are present but NOT as a prefix. Field ORDER is what
    // makes the forget map a projection of the leading fields; a reordered list is a different
    // record that happens to share names.
    let mut reordered = std_ctx();
    reordered.add_structure("Par", &[("a", ty0())], &[("base", var("a"))]);
    reordered.add_structure(
        "Reord",
        &[("a", ty0())],
        &[("extra", arrow(var("a"), var("a"))), ("base", var("a"))],
    );
    assert!(
        reordered.get_global("Reord_toPar").is_none(),
        "a parent's fields present out of order are NOT inheritance — `extends` flattens the \
         parent's fields FIRST, so anything else is a coincidence of names",
    );

    // NEAR-MISS 2 — a field of the same NAME at a different TYPE.
    let mut retyped = std_ctx();
    retyped.add_structure("Par", &[("a", ty0())], &[("base", var("a"))]);
    retyped.add_structure(
        "Retyped",
        &[("a", ty0())],
        &[("base", nat()), ("extra", var("a"))],
    );
    assert!(
        retyped.get_global("Retyped_toPar").is_none(),
        "a same-named field at a different type is not the parent's field — the forget map \
         would not even type-check",
    );

    // NEAR-MISS 3 — a different parameter telescope. `Par (a : Type)` and `Other (a b : Type)`
    // have no shared parameter list to write the edge over.
    let mut reparam = std_ctx();
    reparam.add_structure("Par", &[("a", ty0())], &[("base", var("a"))]);
    reparam.add_structure(
        "Other",
        &[("a", ty0()), ("b", ty0())],
        &[("base", var("a")), ("extra", var("b"))],
    );
    assert!(
        reparam.get_global("Other_toPar").is_none(),
        "a different parameter telescope is a different family, not a child",
    );

    // NEAR-MISS 4 — an identical field list is not inheritance either; a child must EXTEND.
    let mut twin = std_ctx();
    twin.add_structure("Par", &[("a", ty0())], &[("base", var("a"))]);
    twin.add_structure("Twin", &[("a", ty0())], &[("base", var("a"))]);
    assert!(
        twin.get_global("Twin_toPar").is_none(),
        "a structure with exactly the parent's fields adds nothing — `extends` is a STRICT \
         extension, and deriving an edge here would make every pair of like-shaped records \
         interchangeable",
    );

    // NEAR-MISS 5 — a FIELDLESS parent. Every structure trivially extends the empty record, so
    // deriving that edge would connect everything to everything.
    let mut empty = std_ctx();
    empty.add_structure("Unitish", &[("a", ty0())], &[]);
    empty.add_structure("Holder", &[("a", ty0())], &[("base", var("a"))]);
    assert!(
        empty.get_global("Holder_toUnitish").is_none(),
        "a fieldless parent is extended by everything, so it is evidence of nothing",
    );
}

/// A structure is declared at a sort that can HOLD its fields, and the constructor it generates
/// is one the CIC universe check accepts at that sort.
///
/// This is the invariant `lean_conformance_t07_typeclasses::a_bundled_structure_is_declared_at_the_sort_its_carrier_forces`
/// is about, stated so that it can be decided. That test reproduces the defect by re-declaring
/// the bundled record at a HARD-CODED `Type 0` and offering `add_structure`'s constructor to
/// `add_constructor_checked`, then asserting the check passes — which it cannot, because a
/// `Type 0` inductive storing a `Type 0` field is exactly the Girard/Hurkens shape the check
/// exists to refuse. The invariant that is actually wanted is the one below: whatever sort
/// `add_structure` CHOSE, the checked path must accept the constructor at THAT sort.
#[test]
fn a_structure_is_declared_at_a_sort_its_own_constructor_survives_the_universe_check() {
    // A record storing a `Type 0` field morally lives in `Type 1`, and must be declared there.
    let mut bundled = std_ctx();
    bundled.add_structure(
        "Bundle",
        &[],
        &[
            ("carrier", ty0()),
            (
                "mul",
                arrow(var("carrier"), arrow(var("carrier"), var("carrier"))),
            ),
            ("one", var("carrier")),
        ],
    );
    let declared = bundled
        .get_global("Bundle")
        .expect("Bundle is registered")
        .clone();
    assert_eq!(
        declared,
        Term::Sort(Universe::Type(1)),
        "a record storing a `Type 0` field lives in `Type 1` — declaring it at `Type 0` is the \
         Girard/Hurkens door",
    );

    // THE INVARIANT: the constructor `add_structure` generated passes the CIC universe check at
    // the sort `add_structure` declared. Re-declaring the inductive at its own registered sort
    // and offering the same constructor to the CHECKED path is the faithful reproduction.
    let ctor = bundled
        .get_global("Bundle_mk")
        .expect("Bundle_mk is registered")
        .clone();
    let mut checked = std_ctx();
    checked.add_indexed_inductive("Bundle", declared, 0);
    assert!(
        checked
            .add_constructor_checked("Bundle_mk", "Bundle", ctor)
            .is_ok(),
        "`add_structure` must never register a constructor the CIC universe check would reject \
         at the sort it chose",
    );

    // …and the same must hold for a record whose fields force nothing, which stays at `Type 0`.
    let mut flat = std_ctx();
    flat.add_structure("Flat", &[], &[("x", nat()), ("y", nat())]);
    assert_eq!(
        flat.get_global("Flat").expect("Flat is registered"),
        &ty0(),
        "a record of `Type 0` values is not pushed up a level for nothing",
    );
    let flat_ctor = flat
        .get_global("Flat_mk")
        .expect("Flat_mk is registered")
        .clone();
    let mut flat_check = std_ctx();
    flat_check.add_indexed_inductive("Flat", ty0(), 0);
    assert!(
        flat_check
            .add_constructor_checked("Flat_mk", "Flat", flat_ctor)
            .is_ok(),
        "and its constructor is accepted at `Type 0`",
    );

    // The projection reads back and both kernels accept what the encoding builds.
    let dict = apps(
        g("Bundle_mk"),
        &[nat(), lam("x", nat(), lam("y", nat(), var("x"))), g("Zero")],
    );
    assert_eq!(
        normalize(&bundled, &app(g("Bundle_carrier"), dict.clone())),
        nat(),
        "(toBundle Nat).carrier = Nat",
    );
    assert!(
        matches!(double_check(&bundled, &dict), DoubleCheck::Agreed),
        "both kernels agree"
    );
}

// ============================================================================================
// 5 · INSTANCE RESOLUTION ORDER — the determined case must not move
// ============================================================================================

/// An UNDETERMINED goal (its class variable still a metavariable) resolving differently from a
/// determined one is deliberate: first-match on an open goal lets the earliest entry INVENT the
/// class variable, which is the bug. The negative is that the determined case is untouched —
/// first match over the registration vector, which `lean_conformance_t02_declarations` pins as the
/// contract a translator orders against, and which this test pins from the other side so the
/// two can never drift apart.
#[test]
fn only_an_undetermined_goal_changes_which_instance_wins() {
    let mut ctx = std_ctx();
    ctx.add_structure("Marker", &[("a", ty0())], &[("mark", nat())]);
    register_instance(
        &mut ctx,
        "markerFirst",
        app(g("Marker"), nat()),
        apps(g("Marker_mk"), &[nat(), peano(1)]),
    );
    register_instance(
        &mut ctx,
        "markerSecond",
        app(g("Marker"), nat()),
        apps(g("Marker_mk"), &[nat(), peano(2)]),
    );

    // DETERMINED: resolves deterministically, by Lean's tie-break — equal priority, so the
    // LATER declaration wins. The point of this test is that a determined goal is decided by
    // the instance ORDER alone and never by the shape of the goal; only the direction of that
    // order changed when we adopted Lean's rule.
    assert_eq!(
        resolve(&ctx, &app(g("Marker"), nat())),
        Some(g("markerSecond")),
        "a goal that pins its class variable resolves by the declaration order, latest first",
    );

    // UNDETERMINED across DIFFERENT carriers: the open variable must not be invented by the
    // earliest entry.
    let mut carriers = std_ctx();
    carriers.add_structure("Opener", &[("a", ty0())], &[("seed", var("a"))]);
    register_instance(
        &mut carriers,
        "openerBool",
        app(g("Opener"), g("Bool")),
        apps(g("Opener_mk"), &[g("Bool"), g("false")]),
    );
    register_instance(
        &mut carriers,
        "openerNat",
        app(g("Opener"), nat()),
        apps(g("Opener_mk"), &[nat(), g("Zero")]),
    );
    // Pinned either way — the specific instance always wins, in both directions.
    assert_eq!(
        resolve(&carriers, &app(g("Opener"), g("Bool"))),
        Some(g("openerBool"))
    );
    assert_eq!(
        resolve(&carriers, &app(g("Opener"), nat())),
        Some(g("openerNat"))
    );
    let mut mctx = MetaCtx::new();
    let open = mctx.fresh();
    assert_eq!(
        resolve_instance(&carriers, &mut mctx, &app(g("Opener"), open)),
        Some(g("openerNat")),
        "an OPEN class variable takes the most recently declared candidate, the position \
         Lean's `@[default_instance]` occupies — never the earliest entry, which would pick \
         the carrier by accident of registration order",
    );

    // A single candidate is not a tie-break at all: an open goal with one match takes it.
    let mut only = std_ctx();
    only.add_structure("Solo", &[("a", ty0())], &[("seed", var("a"))]);
    register_instance(
        &mut only,
        "soloBool",
        app(g("Solo"), g("Bool")),
        apps(g("Solo_mk"), &[g("Bool"), g("true")]),
    );
    let mut mctx = MetaCtx::new();
    let open = mctx.fresh();
    assert_eq!(
        resolve_instance(&only, &mut mctx, &app(g("Solo"), open)),
        Some(g("soloBool")),
        "one candidate resolves an open goal and determines the class variable",
    );

    // And a goal no entry matches is still `None`, open or not — resolution never invents.
    let mut mctx = MetaCtx::new();
    assert!(
        resolve_instance(&only, &mut mctx, &app(g("Marker"), nat())).is_none(),
        "an unmatched goal resolves to nothing, whichever direction the scan runs",
    );
}

// ============================================================================================
// 6 · OUTPUT PARAMETERS — inferred only where an input remains to match on
// ============================================================================================

/// Treating a class parameter as an OUTPUT means resolution stops matching on it. That is
/// exactly right for `HAdd`'s result type and exactly wrong for `Inhabited`'s carrier, so the
/// inference needs a floor: a class must keep at least one input, or resolution has nothing to
/// key on and the first entry wins every goal. The negatives here are the classes that must
/// keep matching on every parameter.
#[test]
fn an_output_parameter_is_inferred_only_where_an_input_remains() {
    // A ONE-PARAMETER class whose only field RETURNS the carrier — `Inhabited`'s shape. If
    // "appears only in result position" alone made a parameter an output, this carrier would
    // become one and every `Inhabited τ` goal would take the first entry.
    let mut carrier = std_ctx();
    carrier.add_structure("Default", &[("a", ty0())], &[("dflt", var("a"))]);
    register_instance(
        &mut carrier,
        "defaultBool",
        app(g("Default"), g("Bool")),
        apps(g("Default_mk"), &[g("Bool"), g("true")]),
    );
    register_instance(
        &mut carrier,
        "defaultNat",
        app(g("Default"), nat()),
        apps(g("Default_mk"), &[nat(), g("Zero")]),
    );
    assert_eq!(
        resolve(&carrier, &app(g("Default"), nat())),
        Some(g("defaultNat")),
        "a one-parameter class must keep matching on its carrier — with no input left there is \
         nothing to resolve against and the first entry would answer every goal",
    );

    // A TWO-PARAMETER class where BOTH parameters are consumed — `Action R M`'s shape. Neither
    // is an output, so both take part in matching.
    let mut both = std_ctx();
    both.add_structure(
        "Acts",
        &[("r", ty0()), ("m", ty0())],
        &[("act", arrow(var("r"), arrow(var("m"), var("m"))))],
    );
    register_instance(
        &mut both,
        "actsNatNat",
        apps(g("Acts"), &[nat(), nat()]),
        apps(
            g("Acts_mk"),
            &[nat(), nat(), lam("x", nat(), lam("y", nat(), var("y")))],
        ),
    );
    assert!(
        resolve(&both, &apps(g("Acts"), &[nat(), g("Bool")])).is_none(),
        "a consumed parameter is an INPUT: a goal that disagrees on it has no instance",
    );

    // A DECLARED override in both directions — the inference is a default, never a ceiling.
    let mut declared = std_ctx();
    declared.add_structure(
        "Tagger",
        &[("a", ty0()), ("b", ty0())],
        &[("tag", arrow(var("a"), var("b")))],
    );
    register_instance(
        &mut declared,
        "tagNatBool",
        apps(g("Tagger"), &[nat(), g("Bool")]),
        apps(
            g("Tagger_mk"),
            &[nat(), g("Bool"), lam("n", nat(), g("true"))],
        ),
    );
    assert_eq!(
        declared.out_params("Tagger"),
        &[1usize],
        "`b` appears only in the field's result position, so it is inferred as an output",
    );
    assert_eq!(
        resolve(&declared, &apps(g("Tagger"), &[nat(), nat()])),
        Some(g("tagNatBool")),
        "an inferred output takes no part in matching — the input `Nat` selects the instance \
         and the mismatched output becomes an ordinary type error at the use site",
    );
    // Declaring NO outputs restores full matching.
    let mut strict = declared.clone();
    strict.set_out_params("Tagger", vec![]);
    assert!(
        resolve(&strict, &apps(g("Tagger"), &[nat(), nat()])).is_none(),
        "declaring the class has no outputs puts every position back into matching",
    );
    // Declaring the FIRST parameter an output makes the input stop discriminating.
    let mut flipped = declared.clone();
    flipped.set_out_params("Tagger", vec![0]);
    assert_eq!(
        flipped.out_params("Tagger"),
        &[0usize],
        "a declared output list replaces the inferred one outright",
    );
    assert_eq!(
        resolve(&flipped, &apps(g("Tagger"), &[g("Bool"), g("Bool")])),
        Some(g("tagNatBool")),
        "with `a` declared an output, the remaining input `Bool` is what matching keys on",
    );
}

// ============================================================================================
// 7 · DIAMOND COHERENCE — refuse only where the two paths genuinely disagree
// ============================================================================================

/// Refusing an incoherent diamond widens what resolution REJECTS, so the negative is the
/// diamond that is perfectly coherent: two parent edges whose dictionaries are definitionally
/// equal must still resolve. Refusing those would break every genuine multiple-inheritance
/// hierarchy, which is most of an algebraic library.
#[test]
fn a_coherent_diamond_still_resolves_and_only_a_disagreeing_one_refuses() {
    // COHERENT: both children carry the SAME parent data, so both routes build the same
    // dictionary and there is nothing to disagree about.
    let mut ok = std_ctx();
    ok.add_structure("Root", &[("a", ty0())], &[("seed", nat())]);
    ok.add_structure(
        "LeftK",
        &[("a", ty0())],
        &[("seed", nat()), ("lmark", nat())],
    );
    ok.add_structure(
        "RightK",
        &[("a", ty0())],
        &[("seed", nat()), ("rmark", nat())],
    );
    register_instance(
        &mut ok,
        "leftOk",
        app(g("LeftK"), nat()),
        apps(g("LeftK_mk"), &[nat(), peano(1), peano(7)]),
    );
    register_instance(
        &mut ok,
        "rightOk",
        app(g("RightK"), nat()),
        apps(g("RightK_mk"), &[nat(), peano(1), peano(8)]),
    );
    let picked = resolve(&ok, &app(g("Root"), nat()))
        .expect("two routes that AGREE must still discharge the parent goal");
    assert_eq!(
        normalize(&ok, &apps(g("Root_seed"), &[nat(), picked.clone()])),
        peano(1),
        "and whichever route is taken reads the same parent data",
    );
    assert!(
        infer_type(&ok, &picked).is_ok(),
        "the chosen route type-checks"
    );

    // The coherence really is definitional, not an accident of which route ran.
    let via_left = apps(g("LeftK_toRoot"), &[nat(), g("leftOk")]);
    let via_right = apps(g("RightK_toRoot"), &[nat(), g("rightOk")]);
    assert!(
        defeq_for_test(&ok, &via_left, &via_right),
        "the two routes build definitionally equal dictionaries — that is what makes the \
         hierarchy coherent",
    );

    // INCOHERENT: the same shape with different parent data must be REFUSED, not silently
    // decided by registration order.
    let mut clash = std_ctx();
    clash.add_structure("Root", &[("a", ty0())], &[("seed", nat())]);
    clash.add_structure(
        "LeftK",
        &[("a", ty0())],
        &[("seed", nat()), ("lmark", nat())],
    );
    clash.add_structure(
        "RightK",
        &[("a", ty0())],
        &[("seed", nat()), ("rmark", nat())],
    );
    register_instance(
        &mut clash,
        "leftClash",
        app(g("LeftK"), nat()),
        apps(g("LeftK_mk"), &[nat(), peano(1), peano(7)]),
    );
    register_instance(
        &mut clash,
        "rightClash",
        app(g("RightK"), nat()),
        apps(g("RightK_mk"), &[nat(), peano(2), peano(8)]),
    );
    assert!(
        resolve(&clash, &app(g("Root"), nat())).is_none(),
        "two routes carrying different parent data must be refused — everything type-checks \
         either way, so a silent choice surfaces as an unrelated proof failing def_eq far from \
         the diamond",
    );

    // …and the refusal is scoped to the CLASH. A carrier reachable by only ONE of the two
    // routes still resolves in the very same context.
    register_instance(
        &mut clash,
        "leftOnlyBool",
        app(g("LeftK"), g("Bool")),
        apps(g("LeftK_mk"), &[g("Bool"), peano(3), peano(9)]),
    );
    let single = resolve(&clash, &app(g("Root"), g("Bool")))
        .expect("a carrier with only one route is not a diamond and must resolve");
    assert_eq!(
        normalize(&clash, &apps(g("Root_seed"), &[g("Bool"), single])),
        peano(3),
        "the single route carries its own parent data through",
    );

    // A DIRECT instance is not a route: an ordinary overlapping pair is settled by order, not
    // refused as incoherent — that is the difference between an ambiguous hierarchy and two
    // instances a developer deliberately wrote.
    let mut direct = std_ctx();
    direct.add_structure("Plain", &[("a", ty0())], &[("seed", nat())]);
    register_instance(
        &mut direct,
        "plainA",
        app(g("Plain"), nat()),
        apps(g("Plain_mk"), &[nat(), peano(1)]),
    );
    register_instance(
        &mut direct,
        "plainB",
        app(g("Plain"), nat()),
        apps(g("Plain_mk"), &[nat(), peano(2)]),
    );
    assert_eq!(
        resolve(&direct, &app(g("Plain"), nat())),
        Some(g("plainB")),
        "two DIRECT instances at one type are overlap, not a diamond: resolution PICKS, it does \
         not refuse — and it picks the later declaration, as Lean does",
    );
}
