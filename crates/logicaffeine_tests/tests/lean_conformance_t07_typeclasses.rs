//! TIER 7 — TYPECLASSES. The Lean-4 conformance corpus, re-read as a capability
//! matrix (`crates/logicaffeine_tests/tests/fixtures/lean_conformance/cases/t07_typeclasses/`, 14 cases; verdicts in
//! `crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/t07_typeclasses.json`).
//!
//! There is no `class` declaration in LOGOS's vernacular. A class is a STRUCTURE plus entries in an
//! instance table: `Context::add_structure` builds an indexed inductive, a `{name}_mk`, and one
//! projection DEFINITION per field; `Context::add_instance(ty, value)` registers an instance that
//! `resolve_instance` finds, and a typeclass method is exactly a field projection.
//!
//! Every case of the tier is native, each pinned by its test: a class with no instances is still a
//! class (T07-001); an instance is a definition plus a search entry and stays structured (T07-002);
//! resolution waits until the explicit arguments and the expected type pin the class variable
//! (T07-003); recursive and polymorphic instances nest and compute (T07-004); a child class
//! discharges its parent's obligation (T07-005); an output parameter takes no part in matching
//! (T07-006); a default instance fires when nothing pins the class variable (T07-007); the later
//! declared instance wins a tie, as in Lean (T07-008); decidable equality is synthesised for a user
//! inductive (T07-009); the prelude carries `Inhabited` and `Nonempty` (T07-010); one class
//! declaration serves two universe levels (T07-011); a bundled structure is declared at the sort its
//! carrier forces (T07-012); an incoherent diamond is refused rather than resolved silently
//! (T07-013); and a method call is the projection applied parameters-first, ι-computing through a
//! literal dictionary, with structure eta round-tripping (T07-014).
//!
//! Every test is named for the capability it decides, and each decides exactly one corpus case.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, double_check, elaborate_app, elaborate_app_against, infer_type, normalize,
    resolve_instance, Context, DoubleCheck, MetaCtx, ParamKind, Term, Universe,
};

// ============================================================================================
// HELPERS — per-file by house rule; there is no shared kernel-test helper module.
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
fn fix(name: &str, body: Term) -> Term {
    Term::Fix {
        name: name.to_string(),
        body: Box::new(body),
    }
}
fn match_(d: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(motive),
        cases,
    }
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
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

/// Does `t` still carry an elaborator metavariable? Those are `Term::Var`s whose name begins
/// with `?`; to the kernel they are ordinary variables, so one surviving into an emitted term
/// is at best `UnboundVariable` and at worst captured by a same-named binder.
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

/// Register `natAdd : Nat → Nat → Nat` by structural recursion on the first argument — the
/// corpus's size functions add, and `Succ`-only stand-ins would not exercise the recursion.
fn register_nat_add(ctx: &mut Context) {
    let body = fix(
        "f",
        lam(
            "a",
            nat(),
            lam(
                "b",
                nat(),
                match_(
                    var("a"),
                    lam("_", nat(), nat()),
                    vec![
                        var("b"),
                        lam(
                            "a'",
                            nat(),
                            app(g("Succ"), apps(var("f"), &[var("a'"), var("b")])),
                        ),
                    ],
                ),
            ),
        ),
    );
    ctx.add_definition(
        "natAdd".to_string(),
        arrow(nat(), arrow(nat(), nat())),
        body,
    );
}
fn nadd(a: Term, b: Term) -> Term {
    apps(g("natAdd"), &[a, b])
}

/// `Box (α : Type) := Box_mk (α)` — the unary type constructor the recursive-instance cases
/// recurse through.
fn register_box(ctx: &mut Context) {
    ctx.add_indexed_inductive("Box", arrow(ty0(), ty0()), 1);
    ctx.add_constructor(
        "Box_mk",
        "Box",
        pi("a", ty0(), arrow(var("a"), app(g("Box"), var("a")))),
    );
}

/// `class Sized (α : Type) where size : α → Nat`, plus `sizeOf : Π{α}. [Sized α] → α → Nat`.
/// Returns `sizeOf`'s type, which the elaboration tests need to drive `elaborate_app`.
fn register_sized(ctx: &mut Context) -> Term {
    ctx.add_structure(
        "Sized",
        &[("a", ty0())],
        &[("size", arrow(var("a"), nat()))],
    );
    let size_of_ty = pi(
        "a",
        ty0(),
        arrow(app(g("Sized"), var("a")), arrow(var("a"), nat())),
    );
    let size_of_body = lam(
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
    ctx.add_definition("sizeOf".to_string(), size_of_ty.clone(), size_of_body);
    size_of_ty
}

/// One `add_definition` + one `add_instance` — the two-call shape T07-002 pins.
fn register_instance(ctx: &mut Context, name: &str, ty: Term, value: Term) {
    ctx.add_definition(name.to_string(), ty.clone(), value);
    ctx.add_instance(ty, g(name));
}

/// Resolve `goal` against the instance database from a fresh metavariable context.
fn resolve(ctx: &Context, goal: &Term) -> Option<Term> {
    let mut mctx = MetaCtx::new();
    resolve_instance(ctx, &mut mctx, goal)
}

// ============================================================================================
// T07-001 · `class` declaration — a class is a structure plus entries in a flat vector
// ============================================================================================

/// A class with NO registered instance is not a class at all. `class_heads` is folded over
/// `ctx.instances()`, so a class whose instances have not been registered contributes no head,
/// and `try_instance` then classifies a `[Lonely α]` PARAMETER as an ordinary type parameter to
/// be solved by unifying the conclusion — which never mentions it. The premise's metavariable
/// is left unsolved, the instance "resolves", and `elaborate_app` returns `Ok` with a live `?`
/// in the emitted term. The kernel then rejects it, far from the omission, as an unbound
/// variable — or, worse, the `?`-name collides with a binder and is captured.
#[test]
fn a_class_with_no_instances_is_still_a_class() {
    let mut ctx = std_ctx();
    let size_of_ty = register_sized(&mut ctx);
    register_box(&mut ctx);
    // class Lonely (α : Type) where witness : α → α  — declared, and never instantiated.
    ctx.add_structure(
        "Lonely",
        &[("a", ty0())],
        &[("witness", arrow(var("a"), var("a")))],
    );

    register_instance(
        &mut ctx,
        "sizedNat",
        app(g("Sized"), nat()),
        apps(g("Sized_mk"), &[nat(), lam("n", nat(), var("n"))]),
    );
    // instance {α} [Lonely α] : Sized (Box α) — the premise's head is the EMPTY class.
    let box_a = app(g("Box"), var("a"));
    let sized_box_ty = pi(
        "a",
        ty0(),
        arrow(app(g("Lonely"), var("a")), app(g("Sized"), box_a.clone())),
    );
    let sized_box_body = lam(
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
    register_instance(&mut ctx, "sizedBox", sized_box_ty, sized_box_body);

    // `def useLonely (α) [Lonely α] …` — a consumer whose binder kinds put the instance in the
    // right position. Recording them is the one part of "declaring a class" LOGOS does have.
    ctx.set_binder_kinds(
        "sizeOf",
        vec![
            ParamKind::Implicit,
            ParamKind::Instance,
            ParamKind::Explicit,
        ],
    );
    assert_eq!(
        ctx.binder_kinds("sizeOf"),
        Some(
            [
                ParamKind::Implicit,
                ParamKind::Instance,
                ParamKind::Explicit
            ]
            .as_slice()
        ),
        "a consumer's `[C α]` position must be recorded as ParamKind::Instance",
    );

    // `sizeOf (Box_mk Nat 5)`: the `Sized (Box Nat)` goal matches `sizedBox`, whose `[Lonely α]`
    // premise cannot be discharged — there is no `Lonely` instance anywhere.
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
        Ok((term, _)) => assert!(
            !has_meta(term),
            "an undischargeable class premise must be reported, not silently demoted to a type \
             parameter: elaboration returned a term with a live metavariable in it: {term}",
        ),
    }
    // And whatever the elaborator hands back must be a term the kernel accepts.
    if let Ok((term, _)) = &elaborated {
        assert!(
            infer_type(&ctx, term).is_ok(),
            "an elaborated term must type-check; got {:?} for {term}",
            infer_type(&ctx, term),
        );
    }
}

// ============================================================================================
// T07-002 · `instance` declaration — one `add_definition` plus one `add_instance`
// ============================================================================================

/// The two-call shape, and what each call buys. The definition gives the instance a NAME (so
/// it appears in a certificate's dependency set rather than being inlined); the `add_instance`
/// entry — whose value must be the `Global`, never the unfolded dictionary — is the only thing
/// resolution ever scans. Omit either and the failure surfaces far from the omission.
#[test]
fn an_instance_is_a_definition_plus_a_search_entry() {
    let mut ctx = std_ctx();
    ctx.add_structure("Pointed", &[("a", ty0())], &[("point", var("a"))]);

    // The DEFINITION alone. Nothing scans definitions during resolution, and `class_heads` is
    // folded over the instance vector, so the class is invisible too.
    ctx.add_definition(
        "pointedNat".to_string(),
        app(g("Pointed"), nat()),
        apps(g("Pointed_mk"), &[nat(), g("Zero")]),
    );
    assert!(
        resolve(&ctx, &app(g("Pointed"), nat())).is_none(),
        "a defined-but-unregistered instance must be invisible to resolution",
    );

    // Now the search entry, carrying the `Global` — not the body.
    ctx.add_instance(app(g("Pointed"), nat()), g("pointedNat"));
    let resolved =
        resolve(&ctx, &app(g("Pointed"), nat())).expect("the registered instance resolves");
    assert_eq!(
        resolved,
        g("pointedNat"),
        "the resolved instance must stay STRUCTURED — a `Global`, not the δ-unfolded dictionary, \
         or every use site carries its own copy of the whole instance",
    );

    // A named instance at a SECOND type: two vector entries, both reachable, each selected by
    // its own carrier.
    register_instance(
        &mut ctx,
        "pointedBool",
        app(g("Pointed"), g("Bool")),
        apps(g("Pointed_mk"), &[g("Bool"), g("true")]),
    );
    assert_eq!(
        resolve(&ctx, &app(g("Pointed"), g("Bool"))),
        Some(g("pointedBool"))
    );
    assert_eq!(
        resolve(&ctx, &app(g("Pointed"), nat())),
        Some(g("pointedNat")),
        "adding an instance at Bool must not disturb the one at Nat",
    );

    // `instance sizedNat : Sized Nat := ⟨fun n => n + n + n⟩` — a non-trivial value, so the
    // definition/registration split is load-bearing for term size, and the projection computes.
    register_nat_add(&mut ctx);
    let _ = register_sized(&mut ctx);
    register_instance(
        &mut ctx,
        "sizedNat",
        app(g("Sized"), nat()),
        apps(
            g("Sized_mk"),
            &[
                nat(),
                lam("n", nat(), nadd(nadd(var("n"), var("n")), var("n"))),
            ],
        ),
    );
    let measured = apps(g("sizeOf"), &[nat(), g("sizedNat"), peano(2)]);
    assert!(
        infer_type(&ctx, &measured).is_ok(),
        "the instance's use site type-checks"
    );
    assert_eq!(
        normalize(&ctx, &measured),
        peano(6),
        "measure Nat 2 = 6 — the registered instance is the one that computes",
    );

    // The instance's TYPE is matched by unification, so it must be the fully applied class
    // type. A partially applied head never matches a ground goal.
    let mut partial = std_ctx();
    partial.add_structure("Pointed", &[("a", ty0())], &[("point", var("a"))]);
    partial.add_definition(
        "pointedNat".to_string(),
        app(g("Pointed"), nat()),
        apps(g("Pointed_mk"), &[nat(), g("Zero")]),
    );
    partial.add_instance(g("Pointed"), g("pointedNat"));
    assert!(
        resolve(&partial, &app(g("Pointed"), nat())).is_none(),
        "a partially applied class head must not satisfy a fully applied goal",
    );
}

// ============================================================================================
// T07-003 · Simple instance resolution — deferred until the arguments pin the class variable
// ============================================================================================

/// Resolution runs AFTER the explicit arguments and after the expected-type pass, never before.
/// The discriminating case is a decoy: the first vector entry is at `Bool` and the call is at
/// `Nat`, so an eager resolver — seeing the goal `Sized ?α` — would unify `?α := Bool` and take
/// the wrong instance while still type-checking.
#[test]
fn instance_resolution_waits_for_the_explicit_arguments() {
    let mut ctx = std_ctx();
    let size_of_ty = register_sized(&mut ctx);
    ctx.add_structure("Pointed", &[("a", ty0())], &[("point", var("a"))]);

    // Registered FIRST — the decoy.
    register_instance(
        &mut ctx,
        "sizedBool",
        app(g("Sized"), g("Bool")),
        apps(
            g("Sized_mk"),
            &[
                g("Bool"),
                lam(
                    "b",
                    g("Bool"),
                    match_(
                        var("b"),
                        lam("_", g("Bool"), nat()),
                        vec![peano(1), peano(0)],
                    ),
                ),
            ],
        ),
    );
    // Registered SECOND.
    register_instance(
        &mut ctx,
        "sizedNat",
        app(g("Sized"), nat()),
        apps(g("Sized_mk"), &[nat(), lam("n", nat(), var("n"))]),
    );

    // `earlyResolution (n : Nat) := Sized.size n` — α is pinned only by the explicit argument.
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
        &[peano(7)],
    )
    .expect("the explicit argument pins α := Nat, then Sized Nat resolves");
    assert_eq!(
        term,
        apps(g("sizeOf"), &[nat(), g("sizedNat"), peano(7)]),
        "the Nat instance must win — the Bool entry is first in the vector",
    );
    assert_eq!(ty, nat());
    assert!(
        !has_meta(&term),
        "no `?`-metavariable may survive into the kernel: {term}"
    );
    assert!(
        infer_type(&ctx, &term).is_ok(),
        "the kernel certifies the resolved term"
    );
    assert_eq!(normalize(&ctx, &term), peano(7), "earlyResolution 7 = 7");

    // `fromExpectedType : Bool := Pointed.point` — NO explicit argument at all; only the
    // expected type pins α, and the expected-type pass runs before resolution.
    register_instance(
        &mut ctx,
        "pointedNat",
        app(g("Pointed"), nat()),
        apps(g("Pointed_mk"), &[nat(), g("Zero")]),
    );
    register_instance(
        &mut ctx,
        "pointedBool",
        app(g("Pointed"), g("Bool")),
        apps(g("Pointed_mk"), &[g("Bool"), g("true")]),
    );
    let point_ty = pi("a", ty0(), arrow(app(g("Pointed"), var("a")), var("a")));
    let mut mctx = MetaCtx::new();
    let (term, ty) = elaborate_app_against(
        &ctx,
        &mut mctx,
        &g("Pointed_point"),
        &point_ty,
        &[ParamKind::Implicit, ParamKind::Instance],
        &[],
        Some(&g("Bool")),
    )
    .expect("the expected type pins α := Bool with no argument to read it from");
    assert_eq!(
        term,
        apps(g("Pointed_point"), &[g("Bool"), g("pointedBool")])
    );
    assert_eq!(ty, g("Bool"));
    assert_eq!(normalize(&ctx, &term), g("true"), "fromExpectedType = true");

    // `twoPremises (α β) [Sized α] [Pointed β] (a : α) : β` — two obligations resolved
    // independently in one pass, α pinned by the explicit argument and β by the expected type.
    let two_ty = pi(
        "a",
        ty0(),
        pi(
            "b",
            ty0(),
            arrow(
                app(g("Sized"), var("a")),
                arrow(app(g("Pointed"), var("b")), arrow(var("a"), var("b"))),
            ),
        ),
    );
    ctx.add_definition(
        "twoPremises".to_string(),
        two_ty.clone(),
        lam(
            "a",
            ty0(),
            lam(
                "b",
                ty0(),
                lam(
                    "s",
                    app(g("Sized"), var("a")),
                    lam(
                        "p",
                        app(g("Pointed"), var("b")),
                        lam(
                            "x",
                            var("a"),
                            apps(g("Pointed_point"), &[var("b"), var("p")]),
                        ),
                    ),
                ),
            ),
        ),
    );
    let mut mctx = MetaCtx::new();
    let (term, _) = elaborate_app_against(
        &ctx,
        &mut mctx,
        &g("twoPremises"),
        &two_ty,
        &[
            ParamKind::Implicit,
            ParamKind::Implicit,
            ParamKind::Instance,
            ParamKind::Instance,
            ParamKind::Explicit,
        ],
        &[peano(3)],
        Some(&g("Bool")),
    )
    .expect("two independent instance premises resolve in one pass");
    assert_eq!(
        term,
        apps(
            g("twoPremises"),
            &[nat(), g("Bool"), g("sizedNat"), g("pointedBool"), peano(3)]
        ),
        "each premise resolves from its own variable — α from the argument, β from the expected \
         type — and neither obligation disturbs the other",
    );
    assert!(!has_meta(&term));
    assert_eq!(
        normalize(&ctx, &term),
        g("true"),
        "twoPremises Nat Bool 3 = true"
    );

    // A resolution FAILURE is a hard error, not a recoverable hole: there is no Sized Entity.
    ctx.add_declaration("e", g("Entity"));
    let mut mctx = MetaCtx::new();
    let missing = elaborate_app(
        &ctx,
        &mut mctx,
        &g("sizeOf"),
        &size_of_ty,
        &[
            ParamKind::Implicit,
            ParamKind::Instance,
            ParamKind::Explicit,
        ],
        &[g("e")],
    );
    assert!(
        missing.is_err(),
        "an unresolvable premise must fail loudly, not leave a hole"
    );
}

// ============================================================================================
// T07-004 · Recursive / polymorphic instances — resolved recursively, depth-bounded at 64
// ============================================================================================

/// The instance's registered `ty` is its WHOLE `Π`-type including its premises; `try_instance`
/// peels that telescope, classifies a parameter as a premise iff its head is a known class
/// head, unifies the conclusion with the goal, and resolves each premise at `depth + 1`. The
/// nesting must stay structured — the inner instance is the base instance's `Global`, not its
/// unfolded body.
#[test]
fn a_recursive_instance_chain_resolves_and_stays_structured() {
    let mut ctx = std_ctx();
    register_nat_add(&mut ctx);
    let size_of_ty = register_sized(&mut ctx);
    register_box(&mut ctx);
    // inductive Pair2 (α : Type) where mk : α → α → Pair2 α
    ctx.add_indexed_inductive("Pair2", arrow(ty0(), ty0()), 1);
    ctx.add_constructor(
        "Pair2_mk",
        "Pair2",
        pi(
            "a",
            ty0(),
            arrow(var("a"), arrow(var("a"), app(g("Pair2"), var("a")))),
        ),
    );

    // BASE instance first, so `Sized` is a known class head when the recursive instances'
    // premises are classified.
    register_instance(
        &mut ctx,
        "sizedNat",
        app(g("Sized"), nat()),
        apps(g("Sized_mk"), &[nat(), lam("n", nat(), var("n"))]),
    );

    // instance sizedBox {α} [Sized α] : Sized (Box α) := ⟨fun b => match b with
    //   | Box.mk a => Sized.size a + 1⟩
    let box_a = app(g("Box"), var("a"));
    let sized_box_ty = pi(
        "a",
        ty0(),
        arrow(app(g("Sized"), var("a")), app(g("Sized"), box_a.clone())),
    );
    let sized_box_body = lam(
        "a",
        ty0(),
        lam(
            "s",
            app(g("Sized"), var("a")),
            apps(
                g("Sized_mk"),
                &[
                    box_a.clone(),
                    lam(
                        "b",
                        box_a.clone(),
                        match_(
                            var("b"),
                            lam("_", box_a.clone(), nat()),
                            vec![lam(
                                "x",
                                var("a"),
                                app(
                                    g("Succ"),
                                    apps(g("Sized_size"), &[var("a"), var("s"), var("x")]),
                                ),
                            )],
                        ),
                    ),
                ],
            ),
        ),
    );
    register_instance(&mut ctx, "sizedBox", sized_box_ty, sized_box_body);

    // instance sizedPair2 {α} [Sized α] : Sized (Pair2 α)
    let pair_a = app(g("Pair2"), var("a"));
    let sized_pair_ty = pi(
        "a",
        ty0(),
        arrow(app(g("Sized"), var("a")), app(g("Sized"), pair_a.clone())),
    );
    let sized_pair_body = lam(
        "a",
        ty0(),
        lam(
            "s",
            app(g("Sized"), var("a")),
            apps(
                g("Sized_mk"),
                &[
                    pair_a.clone(),
                    lam(
                        "p",
                        pair_a.clone(),
                        match_(
                            var("p"),
                            lam("_", pair_a.clone(), nat()),
                            vec![lam(
                                "x",
                                var("a"),
                                lam(
                                    "y",
                                    var("a"),
                                    nadd(
                                        apps(g("Sized_size"), &[var("a"), var("s"), var("x")]),
                                        apps(g("Sized_size"), &[var("a"), var("s"), var("y")]),
                                    ),
                                ),
                            )],
                        ),
                    ),
                ],
            ),
        ),
    );
    register_instance(&mut ctx, "sizedPair2", sized_pair_ty, sized_pair_body);

    // `sizeOf' (Box Nat) (Box.mk 5) = 6`.
    let one_deep = apps(g("Box_mk"), &[nat(), peano(5)]);
    let mut mctx = MetaCtx::new();
    let (term, _) = elaborate_app(
        &ctx,
        &mut mctx,
        &g("sizeOf"),
        &size_of_ty,
        &[
            ParamKind::Implicit,
            ParamKind::Instance,
            ParamKind::Explicit,
        ],
        &[one_deep],
    )
    .expect("the recursive instance resolves through Box");
    assert!(
        !has_meta(&term),
        "the recursive resolution must be fully instantiated: {term}"
    );
    assert_eq!(
        normalize(&ctx, &term),
        peano(6),
        "sizeOf' (Box Nat) (Box.mk 5) = 6"
    );

    // TWO levels: `boxedBoxedNat = 7`, and the inner instance must be the base instance's
    // `Global`, kept structured by the direct `mctx.solve` rather than unified into place.
    let two_deep = apps(
        g("Box_mk"),
        &[app(g("Box"), nat()), apps(g("Box_mk"), &[nat(), peano(5)])],
    );
    let mut mctx = MetaCtx::new();
    let (term, _) = elaborate_app(
        &ctx,
        &mut mctx,
        &g("sizeOf"),
        &size_of_ty,
        &[
            ParamKind::Implicit,
            ParamKind::Instance,
            ParamKind::Explicit,
        ],
        &[two_deep],
    )
    .expect("two levels of nesting resolve");
    assert_eq!(
        term,
        apps(
            g("sizeOf"),
            &[
                app(g("Box"), app(g("Box"), nat())),
                apps(
                    g("sizedBox"),
                    &[
                        app(g("Box"), nat()),
                        apps(g("sizedBox"), &[nat(), g("sizedNat")])
                    ]
                ),
                apps(
                    g("Box_mk"),
                    &[app(g("Box"), nat()), apps(g("Box_mk"), &[nat(), peano(5)])]
                ),
            ],
        ),
        "the nested instance must be `sizedBox (Box Nat) (sizedBox Nat sizedNat)` — structured, \
         with the base instance appearing by name",
    );
    assert_eq!(normalize(&ctx, &term), peano(7), "boxedBoxedNat = 7");

    // Two recursive instances at once: resolution picks by unifying the CONCLUSION, not by
    // class head. `sizeOf' (Pair2 Nat) (Pair2.mk 2 3) = 5`.
    let pair = apps(g("Pair2_mk"), &[nat(), peano(2), peano(3)]);
    let mut mctx = MetaCtx::new();
    let (term, _) = elaborate_app(
        &ctx,
        &mut mctx,
        &g("sizeOf"),
        &size_of_ty,
        &[
            ParamKind::Implicit,
            ParamKind::Instance,
            ParamKind::Explicit,
        ],
        &[pair],
    )
    .expect("the Pair2 instance is selected by its conclusion");
    assert_eq!(
        normalize(&ctx, &term),
        peano(5),
        "sizeOf' (Pair2 Nat) (Pair2.mk 2 3) = 5"
    );

    // MIXED nesting — `Box (Pair2 (Box Nat))` exercises three instances in one resolution.
    let inner = |n: u32| apps(g("Box_mk"), &[nat(), peano(n)]);
    let mixed = apps(
        g("Box_mk"),
        &[
            app(g("Pair2"), app(g("Box"), nat())),
            apps(g("Pair2_mk"), &[app(g("Box"), nat()), inner(1), inner(2)]),
        ],
    );
    let mut mctx = MetaCtx::new();
    let (term, _) = elaborate_app(
        &ctx,
        &mut mctx,
        &g("sizeOf"),
        &size_of_ty,
        &[
            ParamKind::Implicit,
            ParamKind::Instance,
            ParamKind::Explicit,
        ],
        &[mixed],
    )
    .expect("three instances resolve in one goal");
    assert_eq!(
        normalize(&ctx, &term),
        peano(6),
        "Box (Pair2 (Box 1) (Box 2)) has size 6"
    );

    // A 6-deep chain — well inside MAX_INSTANCE_DEPTH = 64, and the shape a monomorphised
    // hierarchy exceeds.
    let mut ty_chain = nat();
    let mut val_chain = g("Zero");
    for _ in 0..6 {
        val_chain = apps(g("Box_mk"), &[ty_chain.clone(), val_chain]);
        ty_chain = app(g("Box"), ty_chain);
    }
    let mut mctx = MetaCtx::new();
    let (term, _) = elaborate_app(
        &ctx,
        &mut mctx,
        &g("sizeOf"),
        &size_of_ty,
        &[
            ParamKind::Implicit,
            ParamKind::Instance,
            ParamKind::Explicit,
        ],
        &[val_chain],
    )
    .expect("a 6-deep chain is inside the depth bound");
    assert!(!has_meta(&term));
    assert_eq!(normalize(&ctx, &term), peano(6), "deepChain = 6");
}

// ============================================================================================
// T07-005 · `class … extends` — no parent subobject, no `toParent`, no automatic forget
// ============================================================================================

/// Lean's `class Bar α extends Foo α` gives a `Bar.toFoo` projection AND makes every `[Foo α]`
/// goal dischargeable from a `Bar α` in scope. `add_structure` has no parent parameter at all:
/// it takes a flat field list, so the parent's identity is gone and there is nothing to
/// project. The second half of this test shows the gap is in the DECLARATION api, not in
/// resolution — a hand-synthesised forget-definition plus its own `add_instance` does work,
/// which is exactly the per-`extends`-edge emission a translator must do by hand.
#[test]
fn a_child_class_discharges_its_parents_obligation() {
    // `class Foo (α) where base : α` / `class Bar (α) extends Foo α where left : α → α`, in
    // the only encoding available: Bar's field list is the FLATTENED union.
    let mut ctx = std_ctx();
    ctx.add_structure("Foo", &[("a", ty0())], &[("base", var("a"))]);
    ctx.add_structure(
        "Bar",
        &[("a", ty0())],
        &[("base", var("a")), ("left", arrow(var("a"), var("a")))],
    );
    register_instance(
        &mut ctx,
        "barNat",
        app(g("Bar"), nat()),
        apps(
            g("Bar_mk"),
            &[nat(), g("Zero"), lam("n", nat(), app(g("Succ"), var("n")))],
        ),
    );

    // `def parentOfBar (α) [inst : Bar α] : Foo α := inst.toFoo` — Lean generates `Bar.toFoo`;
    // `add_structure` generates one projection per FIELD and nothing else.
    assert!(
        ctx.get_global("Bar_toFoo").is_some(),
        "declaring `Bar extends Foo` must yield the parent projection `Bar_toFoo`",
    );

    // `def useFoo (α) [Foo α] : α := Foo.base`, called with only a `Bar` instance in scope —
    // most of a hierarchy's surface is functions over the PARENT class.
    assert!(
        resolve(&ctx, &app(g("Foo"), nat())).is_some(),
        "a `Foo α` obligation must be dischargeable from a registered `Bar α` instance",
    );

    // The encoding a translator must emit BY HAND, and the proof that resolution itself is
    // able to carry it: one forget-definition and one `add_instance` per `extends` edge,
    // emitted AFTER every base instance so the maximally-general conclusion cannot shadow one.
    let mut manual = std_ctx();
    manual.add_structure("Foo", &[("a", ty0())], &[("base", var("a"))]);
    manual.add_structure(
        "Bar",
        &[("a", ty0())],
        &[("base", var("a")), ("left", arrow(var("a"), var("a")))],
    );
    register_instance(
        &mut manual,
        "fooNat",
        app(g("Foo"), nat()),
        apps(g("Foo_mk"), &[nat(), g("Zero")]),
    );
    register_instance(
        &mut manual,
        "barBool",
        app(g("Bar"), g("Bool")),
        apps(
            g("Bar_mk"),
            &[g("Bool"), g("true"), lam("x", g("Bool"), var("x"))],
        ),
    );
    let forget_ty = pi(
        "a",
        ty0(),
        arrow(app(g("Bar"), var("a")), app(g("Foo"), var("a"))),
    );
    let forget_body = lam(
        "a",
        ty0(),
        lam(
            "b",
            app(g("Bar"), var("a")),
            apps(
                g("Foo_mk"),
                &[var("a"), apps(g("Bar_base"), &[var("a"), var("b")])],
            ),
        ),
    );
    register_instance(&mut manual, "Bar_toFoo", forget_ty, forget_body);
    let through_forget = resolve(&manual, &app(g("Foo"), g("Bool")))
        .expect("the hand-emitted forget discharges Foo Bool");
    assert_eq!(
        through_forget,
        apps(g("Bar_toFoo"), &[g("Bool"), g("barBool")]),
        "the parent instance must be reconstructed through the forget edge",
    );
    assert!(
        infer_type(&manual, &through_forget).is_ok(),
        "and the reconstruction type-checks"
    );
    assert_eq!(
        normalize(&manual, &apps(g("Foo_base"), &[g("Bool"), through_forget])),
        g("true"),
        "the parent's field reads through the forget edge",
    );
    // The base instance is NOT shadowed by the (later) maximally-general forget entry.
    assert_eq!(
        resolve(&manual, &app(g("Foo"), nat())),
        Some(g("fooNat")),
        "a forget-instance emitted after the base instances must not capture their goals",
    );
}

// ============================================================================================
// T07-006 · `outParam` — resolution unifies the WHOLE conclusion, output positions included
// ============================================================================================

/// `outParam` says: match on the inputs, let the instance DETERMINE the output. LOGOS has no
/// per-argument annotation anywhere — an instance is an unadorned `(Term, Term)` pair, and
/// `try_instance` unifies the entire conclusion against the goal. The discriminating
/// consequence is that two goals differing ONLY in the output position select DIFFERENT
/// instances, which is precisely what `outParam` forbids: `HAdd`, `HMul`, `Membership`,
/// `GetElem`, `Coe` and every `Bind` in Lean core depend on it.
#[test]
fn an_output_parameter_takes_no_part_in_instance_matching() {
    let mut ctx = std_ctx();
    // class HPlus (α) (β) (γ : outParam Type) where hPlus : α → β → γ
    ctx.add_structure(
        "HPlus",
        &[("a", ty0()), ("b", ty0()), ("c", ty0())],
        &[("hPlus", arrow(var("a"), arrow(var("b"), var("c"))))],
    );
    let hplus = |a: Term, b: Term, c: Term| apps(g("HPlus"), &[a, b, c]);
    register_instance(
        &mut ctx,
        "hplusNatNatNat",
        hplus(nat(), nat(), nat()),
        apps(
            g("HPlus_mk"),
            &[
                nat(),
                nat(),
                nat(),
                lam("x", nat(), lam("y", nat(), var("x"))),
            ],
        ),
    );
    register_instance(
        &mut ctx,
        "hplusNatNatBool",
        hplus(nat(), nat(), g("Bool")),
        apps(
            g("HPlus_mk"),
            &[
                nat(),
                nat(),
                g("Bool"),
                lam("x", nat(), lam("y", nat(), g("true"))),
            ],
        ),
    );

    let at_nat = resolve(&ctx, &hplus(nat(), nat(), nat()));
    let at_bool = resolve(&ctx, &hplus(nat(), nat(), g("Bool")));
    assert_eq!(
        at_nat, at_bool,
        "γ is an output position: two goals whose INPUTS agree must select the same instance, \
         and the instance then determines γ. Today the output drives the match, so the two \
         disagree ({at_nat:?} vs {at_bool:?}) and a `γ` supplied by the surrounding context \
         silently reinterprets which instance the source meant.",
    );

    // `Decode (α) (β : outParam Type)` — the output must come FROM the instance, so a goal
    // with the output still unknown must resolve, and a goal whose output CONTRADICTS the
    // instance must fail rather than find another entry.
    ctx.add_structure(
        "Decode",
        &[("a", ty0()), ("b", ty0())],
        &[("decode", arrow(var("a"), var("b")))],
    );
    register_instance(
        &mut ctx,
        "decodeBoolNat",
        apps(g("Decode"), &[g("Bool"), nat()]),
        apps(
            g("Decode_mk"),
            &[
                g("Bool"),
                nat(),
                lam(
                    "b",
                    g("Bool"),
                    match_(
                        var("b"),
                        lam("_", g("Bool"), nat()),
                        vec![peano(1), peano(0)],
                    ),
                ),
            ],
        ),
    );
    // A second instance at a DIFFERENT input, so nothing here turns on priority: the only
    // question is whether the output position takes part in matching.
    register_instance(
        &mut ctx,
        "decodeNatBool",
        apps(g("Decode"), &[nat(), g("Bool")]),
        apps(
            g("Decode_mk"),
            &[nat(), g("Bool"), lam("n", nat(), g("true"))],
        ),
    );
    let mut mctx = MetaCtx::new();
    let unknown_output = mctx.fresh();
    let from_instance = resolve_instance(
        &ctx,
        &mut mctx,
        &apps(g("Decode"), &[g("Bool"), unknown_output]),
    );
    assert_eq!(
        from_instance,
        Some(g("decodeBoolNat")),
        "with β an outParam the INSTANCE determines it: a goal whose output is still unknown \
         must resolve on its input alone",
    );
    assert_eq!(
        resolve(&ctx, &apps(g("Decode"), &[g("Bool"), g("Bool")])),
        Some(g("decodeBoolNat")),
        "the ascribed output must not participate: resolution keys on the input `Bool`, finds \
         `decodeBoolNat`, and the β it yields is then checked against the ascription — an \
         ordinary type error at the use site. Letting the output filter candidates instead \
         turns that error into a different instance, or (here) into `no instance found` for a \
         class that plainly has one",
    );
}

// ============================================================================================
// T07-007 · `@[default_instance]` — one resolution tier, no fallback
// ============================================================================================

/// Lean's defaulting fires on a metavariable that is still unsolved at the END of elaboration,
/// and only then. LOGOS has a single flat vector with no tier: `resolve_instance_at` makes one
/// linear pass and returns at the first candidate that unifies. A goal whose class variable is
/// still open therefore does not default — it is captured by whichever entry happens to be
/// first, binding the class variable to THAT entry's carrier.
#[test]
fn a_default_instance_fires_when_nothing_pins_the_class_variable() {
    let mut ctx = std_ctx();
    ctx.add_structure("Zeroish", &[("a", ty0())], &[("zero", var("a"))]);
    // The specific instance, declared first.
    register_instance(
        &mut ctx,
        "zeroishBool",
        app(g("Zeroish"), g("Bool")),
        apps(g("Zeroish_mk"), &[g("Bool"), g("false")]),
    );
    // `@[default_instance] instance zeroishNat : Zeroish Nat := ⟨0⟩`. There is no API to mark
    // it, so it is an ordinary second entry.
    register_instance(
        &mut ctx,
        "zeroishNat",
        app(g("Zeroish"), nat()),
        apps(g("Zeroish_mk"), &[nat(), g("Zero")]),
    );

    // `notDefaulted : Bool := Zeroish.zero` — the expected type pins α, so the SPECIFIC
    // instance must win. This half works.
    assert_eq!(
        resolve(&ctx, &app(g("Zeroish"), g("Bool"))),
        Some(g("zeroishBool")),
        "a pinned class variable must select the specific instance, never the default",
    );

    // `def defaulted := (Zeroish.zero, 1)` — nothing pins α, so Lean falls back to the
    // default instance and α := Nat.
    let mut mctx = MetaCtx::new();
    let open = mctx.fresh();
    let defaulted = resolve_instance(&ctx, &mut mctx, &app(g("Zeroish"), open));
    assert_eq!(
        defaulted,
        Some(g("zeroishNat")),
        "an undetermined class variable must fall back to the DEFAULT instance, not be captured \
         by whichever entry sits first in the vector",
    );
}

// ============================================================================================
// T07-008 · Instance priority — first-match-wins in INSERTION order
// ============================================================================================

/// Lean's rule is lexicographic: higher priority first, then LATER declaration first. LOGOS
/// has exactly one lever, the sequence of `add_instance` calls, and it resolves EARLIEST-first
/// — so source-ordered emission inverts every tie-break, and nothing at all expresses a
/// priority. All three shapes below type-check either way and compute different numbers, which
/// is why this is a silent mistranslation rather than an error.
#[test]
fn the_later_declared_instance_wins_the_tie() {
    let mut ctx = std_ctx();
    ctx.add_structure("Tagged", &[("a", ty0())], &[("tag", nat())]);
    register_instance(
        &mut ctx,
        "firstDeclared",
        app(g("Tagged"), nat()),
        apps(g("Tagged_mk"), &[nat(), peano(1)]),
    );
    register_instance(
        &mut ctx,
        "secondDeclared",
        app(g("Tagged"), nat()),
        apps(g("Tagged_mk"), &[nat(), peano(2)]),
    );
    let tag = resolve(&ctx, &app(g("Tagged"), nat())).expect("Tagged Nat resolves");
    assert_eq!(
        normalize(&ctx, &apps(g("Tagged_tag"), &[nat(), tag])),
        peano(2),
        "laterWins = 2: two equal-priority instances at one type must tie-break to the LATER \
         declaration, as Lean does",
    );

    // Explicit priorities: the higher wins regardless of declaration order, in both directions.
    ctx.add_structure("Ranked", &[("a", ty0())], &[("rank", nat())]);
    register_instance(
        &mut ctx,
        "lowPriority",
        app(g("Ranked"), g("Bool")),
        apps(g("Ranked_mk"), &[g("Bool"), peano(10)]),
    );
    register_instance(
        &mut ctx,
        "highPriority",
        app(g("Ranked"), g("Bool")),
        apps(g("Ranked_mk"), &[g("Bool"), peano(90)]),
    );
    let ranked = resolve(&ctx, &app(g("Ranked"), g("Bool"))).expect("Ranked Bool resolves");
    assert_eq!(
        normalize(&ctx, &apps(g("Ranked_rank"), &[g("Bool"), ranked])),
        peano(90),
        "priorityWins = 90: `(priority := 900)` must beat `(priority := 100)` declared before it",
    );

    // A GENERAL instance declared before a SPECIFIC one. Specificity plays no part in
    // resolution: the general instance's `α` is freshened to a metavariable and unifies.
    register_box(&mut ctx);
    ctx.add_structure(
        "Render",
        &[("a", ty0())],
        &[("render", arrow(var("a"), nat()))],
    );
    register_instance(
        &mut ctx,
        "renderNat",
        app(g("Render"), nat()),
        apps(g("Render_mk"), &[nat(), lam("n", nat(), var("n"))]),
    );
    let box_a = app(g("Box"), var("a"));
    register_instance(
        &mut ctx,
        "generalBox",
        pi(
            "a",
            ty0(),
            arrow(app(g("Render"), var("a")), app(g("Render"), box_a.clone())),
        ),
        lam(
            "a",
            ty0(),
            lam(
                "r",
                app(g("Render"), var("a")),
                apps(
                    g("Render_mk"),
                    &[
                        box_a.clone(),
                        lam(
                            "b",
                            box_a.clone(),
                            match_(
                                var("b"),
                                lam("_", box_a.clone(), nat()),
                                vec![lam(
                                    "x",
                                    var("a"),
                                    apps(g("Render_render"), &[var("a"), var("r"), var("x")]),
                                )],
                            ),
                        ),
                    ],
                ),
            ),
        ),
    );
    register_instance(
        &mut ctx,
        "specificBox",
        app(g("Render"), app(g("Box"), nat())),
        apps(
            g("Render_mk"),
            &[
                app(g("Box"), nat()),
                lam("_", app(g("Box"), nat()), peano(9)),
            ],
        ),
    );
    let chosen =
        resolve(&ctx, &app(g("Render"), app(g("Box"), nat()))).expect("Render (Box Nat) resolves");
    assert_eq!(
        chosen,
        g("specificBox"),
        "generalBeatsSpecific: Lean takes the later-declared specific instance; the earlier \
         general one must not capture the goal",
    );
}

// ============================================================================================
// T07-009 · `DecidableEq` synthesis — two hand-written procedures, and no synthesis at all
// ============================================================================================

/// `DecidableEq α` is an abbreviation for `Π(a b : α). Decidable (Eq α a b)`, so it has no
/// class of its own; what a development needs is a decision PROCEDURE per type and an instance
/// entry pointing at it. The LOGOS prelude defines `decEqBool` and `decEqNat` — and registers
/// neither as an instance, so `StandardLibrary::register` leaves the instance database empty
/// and a `Decidable (Eq Nat a b)` obligation cannot be discharged even for `Nat`. For a user
/// inductive there is no synthesis of any kind.
#[test]
fn decidable_equality_is_synthesised_for_a_user_inductive() {
    let stock = std_ctx();
    let dec = |p: Term| app(g("Decidable"), p);
    let eq = |t: Term, a: Term, b: Term| apps(g("Eq"), &[t, a, b]);

    assert!(
        stock.is_definition("decEqNat") && stock.is_definition("decEqBool"),
        "the prelude's two decision procedures must exist",
    );
    assert!(
        resolve(&stock, &dec(eq(nat(), peano(3), peano(3)))).is_some(),
        "`natEq a b := decide (a = b)`: the prelude's own `decEqNat` must be reachable as an \
         INSTANCE, or nothing can discharge a `Decidable (Eq Nat _ _)` obligation",
    );

    // The mechanism is not the problem — registering the procedure makes it resolve and decide.
    let mut registered = std_ctx();
    registered.add_instance(
        pi(
            "a",
            nat(),
            pi("b", nat(), dec(eq(nat(), var("a"), var("b")))),
        ),
        g("decEqNat"),
    );
    let inst = resolve(&registered, &dec(eq(nat(), peano(3), peano(3))))
        .expect("a registered procedure resolves");
    assert!(
        infer_type(&registered, &inst).is_ok(),
        "and the resolved procedure type-checks"
    );
    assert_eq!(
        normalize(
            &registered,
            &apps(g("decide"), &[eq(nat(), peano(3), peano(3)), inst])
        ),
        g("true"),
        "natEq 3 3 = true",
    );

    // `inductive Colour where | red | green | blue  deriving DecidableEq` — Lean's handler
    // generates the procedure; LOGOS synthesises nothing, for any type.
    let mut colour = std_ctx();
    colour.add_inductive("Colour", ty0());
    for k in ["red", "green", "blue"] {
        colour.add_constructor(k, "Colour", g("Colour"));
    }
    assert!(
        resolve(&colour, &dec(eq(g("Colour"), g("red"), g("blue")))).is_some(),
        "`deriving DecidableEq` on an enum must yield a decision procedure and its instance — \
         today a user inductive gets neither, and `decide` on it can never fire",
    );
}

// ============================================================================================
// T07-010 · `Inhabited` / `Nonempty` — neither is in the prelude
// ============================================================================================

/// The LOGOS source names both constantly — `context.rs`'s `add_instance` doc, and
/// `elaborate.rs`'s worked example `instance {A} [Inhabited A] : Inhabited (List A)` — but
/// those are doc comments illustrating the API, not registrations. `StandardLibrary::register`
/// provides neither, so `Global("Inhabited")` resolves to nothing. The second half shows the
/// shape they must take: `Inhabited` through `add_structure` (only that path registers the
/// `StructInfo` that gates eta), `Nonempty` through `add_indexed_inductive` at `Prop` plus
/// `add_constructor_checked` — because `add_structure` hard-codes `Type 0`, and a `Type 0`
/// `Nonempty` is `Classical.choice` written by hand.
#[test]
fn the_prelude_carries_inhabited_and_nonempty() {
    let stock = std_ctx();
    for name in [
        "Inhabited",
        "Inhabited_mk",
        "Inhabited_default",
        "Nonempty",
        "Nonempty_intro",
    ] {
        assert!(
            stock.get_global(name).is_some(),
            "`{name}` must be in the standard library — Lean's `Inhabited`/`Nonempty` are core, \
             and `opaque`'s inhabitedness obligation has nothing to cite without them",
        );
    }
    assert!(
        resolve(&stock, &app(g("Inhabited"), nat())).is_some(),
        "`Inhabited Nat` must resolve out of the box",
    );

    // The emission shape a translator must produce today, and the proof that it is sound.
    let mut manual = std_ctx();
    manual.add_structure("MyInhabited", &[("a", ty0())], &[("deflt", var("a"))]);
    assert!(
        manual.struct_info("MyInhabited").is_some(),
        "only `add_structure` registers the StructInfo that gates definitional eta",
    );
    register_instance(
        &mut manual,
        "inhabitedNat",
        app(g("MyInhabited"), nat()),
        apps(g("MyInhabited_mk"), &[nat(), g("Zero")]),
    );
    // The projection takes the PARAMETER first: `MyInhabited_deflt α i`, not `… i`.
    let got = apps(g("MyInhabited_deflt"), &[nat(), g("inhabitedNat")]);
    assert_eq!(normalize(&manual, &got), g("Zero"), "getDefault Nat = 0");

    // `Nonempty` must be a Prop, so it cannot go through `add_structure` at all.
    manual.add_indexed_inductive("MyNonempty", pi("a", ty0(), prop()), 1);
    let intro_ty = pi("a", ty0(), arrow(var("a"), app(g("MyNonempty"), var("a"))));
    manual
        .add_constructor_checked("MyNonempty_intro", "MyNonempty", intro_ty)
        .expect("the checked path accepts a Prop-valued Nonempty");
    assert_eq!(
        infer_type(&manual, &app(g("MyNonempty"), nat())).expect("MyNonempty Nat is typed"),
        prop(),
        "Nonempty must land in Prop; a Type-0 Nonempty is eligible for large elimination, which \
         is Classical.choice",
    );
    // Inhabited ⇒ Nonempty, the sound direction.
    let witness = apps(
        g("MyNonempty_intro"),
        &[
            nat(),
            apps(g("MyInhabited_deflt"), &[nat(), g("inhabitedNat")]),
        ],
    );
    assert!(
        infer_type(&manual, &witness).is_ok(),
        "inhabitedNonempty type-checks"
    );
}

// ============================================================================================
// T07-011 · Mathlib's algebraic hierarchy — the case that settles the Mathlib question
// ============================================================================================

/// A class is a structure, and `add_structure` binds the level variables its parameters are
/// written over — Lean's auto-bound rule — so `class Mul (α : Type u)` is ONE declaration,
/// `Mul.{u}`, serving a `Type 0` carrier and a `Type 1` carrier alike, instantiable by `Const` the
/// way a universe-polymorphic definition is, and two independent levels (`Action.{u,v}`) vary
/// independently. Mathlib is built from universe-polymorphic structures instantiated at several
/// levels at once (`Module R M` with `R : Type u`, `M : Type v`; `CategoryTheory` at
/// `Type (u+1)`), so this one wall decided the whole library.
///
/// Monomorphising at `Type 0` is the tempting wrong move and it is unsound rather than merely
/// lossy: keys are flat `String`s, so the two instantiations collapse into one declaration, and
/// because LOGOS is CUMULATIVE the resulting terms very often still type-check. A class written
/// over a CONCRETE `Type 0` therefore stays monomorphic — the control below.
#[test]
fn one_class_declaration_serves_two_universe_levels() {
    let mut ctx = std_ctx();
    // `Type u` is `Sort (u+1)`.
    let type_at =
        |level: &str| Term::Sort(Universe::Succ(Box::new(Universe::Var(level.to_string()))));
    // `class Mul (α : Type u) where mul : α → α → α`.
    ctx.add_structure(
        "Mul",
        &[("a", type_at("u"))],
        &[("mul", arrow(var("a"), arrow(var("a"), var("a"))))],
    );
    assert_eq!(
        ctx.poly_declaration("Mul")
            .map(|(levels, _)| levels.to_vec()),
        Some(vec!["u".to_string()]),
        "the class carries exactly the level its carrier was written over"
    );
    // At `Type 0` this works: `instance : Mul Nat`.
    assert!(
        infer_type(&ctx, &app(g("Mul"), nat())).is_ok(),
        "Mul Nat is typed"
    );

    // `instance mulType : Mul (Type)` — the SAME class one level up, which Mathlib does
    // constantly. The carrier is `Type 0`, which lives in `Type 1`.
    assert!(
        infer_type(&ctx, &app(g("Mul"), ty0())).is_ok(),
        "one class declaration must serve a `Type 1` carrier too: {:?}",
        infer_type(&ctx, &app(g("Mul"), ty0())),
    );

    // And it must be universe-INSTANTIABLE, the way a universe-polymorphic definition is.
    // `Term::Const { name, levels }` is the only instantiating form and it resolves solely
    // against `Context::universe_polys`, which no type declaration can enter.
    for level in [Universe::Type(0), Universe::Type(1)] {
        let at_level = Term::Const {
            name: "Mul".to_string(),
            levels: vec![level.clone()],
        };
        assert!(
            infer_type(&ctx, &at_level).is_ok(),
            "`Mul.{{{level}}}` must instantiate the class's universe parameter: {:?}",
            infer_type(&ctx, &at_level),
        );
    }

    // `class Action (R : Type u) (M : Type v)` — TWO independent levels, the shape that defeats
    // even per-instantiation monomorphisation because the levels vary independently.
    ctx.add_structure(
        "Action",
        &[("r", type_at("u")), ("m", type_at("v"))],
        &[("smul", arrow(var("r"), arrow(var("m"), var("m"))))],
    );
    assert!(
        infer_type(&ctx, &apps(g("Action"), &[nat(), ty0()])).is_ok(),
        "`Action Nat (Type)` — R at level 0 and M at level 1 — must be typed by one declaration: \
         {:?}",
        infer_type(&ctx, &apps(g("Action"), &[nat(), ty0()])),
    );

    // The machinery a universe-polymorphic STRUCTURE needs already exists and is proven for
    // DEFINITIONS: a level parameter list, `instantiate_universes`, and `Term::Const`. This is
    // the reference point — it must extend to type declarations.
    let tyu = Term::Sort(Universe::Var("u".to_string()));
    ctx.add_universe_poly(
        "idU",
        vec!["u".to_string()],
        pi("a", tyu.clone(), arrow(var("a"), var("a"))),
        lam("a", tyu.clone(), lam("x", var("a"), var("x"))),
    );
    for level in [Universe::Type(0), Universe::Type(1)] {
        assert!(
            infer_type(
                &ctx,
                &Term::Const {
                    name: "idU".to_string(),
                    levels: vec![level]
                }
            )
            .is_ok(),
            "a universe-polymorphic DEFINITION instantiates at any level — the same must hold \
             for a class",
        );
    }

    // THE CONTROL. Written over a CONCRETE `Type 0`, a class is monomorphic — the level variable
    // above is what made `Mul` serve `Type 1`, not cumulativity leaking a higher carrier through.
    ctx.add_structure(
        "MulAtZero",
        &[("a", ty0())],
        &[("mul", arrow(var("a"), arrow(var("a"), var("a"))))],
    );
    assert!(
        infer_type(&ctx, &app(g("MulAtZero"), ty0())).is_err(),
        "a class over a concrete `Type 0` carrier refuses the `Type 1` carrier `Type 0`"
    );
}

// ============================================================================================
// T07-012 · Bundled vs unbundled — the bundled form is registered at a sort it contradicts
// ============================================================================================

/// UNBUNDLED (`class UMonoid (α : Type)`), the carrier is a PARAMETER: every projection binds
/// it before the receiver, and the class resolves. BUNDLED (`structure BMonoid where carrier :
/// Type; …`), the carrier is a FIELD: nothing about it is an instance and every call site passes
/// it explicitly (a `m.carrier` in type position is the elaborator's `CoeSort`, T05-012).
///
/// The load-bearing question is the SORT. A structure storing a `Type 0` field lives in `Type 1`,
/// and `add_structure` infers exactly that least sort and checks the record transactionally, so
/// its own constructor passes the CHECKED CIC universe gate at that sort — while the same
/// constructor under a `Type 0` header, the Girard/Hurkens shape, is refused as Lean refuses it.
#[test]
fn a_bundled_structure_is_declared_at_the_sort_its_carrier_forces() {
    // UNBUNDLED: the carrier is a parameter, so the fields are all at `Type 0` and the
    // declaration is universe-consistent. This is the control.
    let mut unbundled = std_ctx();
    unbundled.add_structure(
        "UMonoid",
        &[("a", ty0())],
        &[
            ("mul", arrow(var("a"), arrow(var("a"), var("a")))),
            ("one", var("a")),
        ],
    );
    register_instance(
        &mut unbundled,
        "umonoidNat",
        app(g("UMonoid"), nat()),
        apps(
            g("UMonoid_mk"),
            &[nat(), lam("x", nat(), lam("y", nat(), var("x"))), g("Zero")],
        ),
    );
    assert_eq!(
        resolve(&unbundled, &app(g("UMonoid"), nat())),
        Some(g("umonoidNat")),
        "the unbundled form resolves — the carrier is a class parameter",
    );
    let u_ctor = unbundled
        .get_global("UMonoid_mk")
        .expect("UMonoid_mk registered")
        .clone();
    let mut u_check = std_ctx();
    u_check.add_indexed_inductive("UMonoid", arrow(ty0(), ty0()), 1);
    assert!(
        u_check
            .add_constructor_checked("UMonoid_mk", "UMonoid", u_ctor)
            .is_ok(),
        "the unbundled constructor is universe-consistent: its `Type 0` binder is the \
         inductive's own parameter, not a stored field",
    );

    // BUNDLED: the carrier is a FIELD.
    let mut bundled = std_ctx();
    bundled.add_structure(
        "BMonoid",
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
    // The projection reads back, so the encoding itself is usable — `m.carrier` just has to be
    // written explicitly wherever a type is needed.
    let nat_bmonoid = apps(
        g("BMonoid_mk"),
        &[nat(), lam("x", nat(), lam("y", nat(), var("x"))), g("Zero")],
    );
    assert_eq!(
        normalize(&bundled, &app(g("BMonoid_carrier"), nat_bmonoid.clone())),
        nat(),
        "(toBundled Nat).carrier = Nat",
    );
    // Both kernels accept the term, so the disagreement is not detectable downstream.
    assert!(matches!(
        double_check(&bundled, &nat_bmonoid),
        DoubleCheck::Agreed
    ));

    // THE SORT: `add_structure` declared the bundled record where its carrier forces it.
    let b_sort = bundled
        .get_global("BMonoid")
        .cloned()
        .expect("BMonoid registered");
    assert_eq!(
        b_sort,
        Term::Sort(Universe::Type(1)),
        "a record storing a `Type 0` field lives in `Type 1`"
    );
    // Offer `add_structure`'s own constructor to the CHECKED path at that sort: admitted.
    let b_ctor = bundled
        .get_global("BMonoid_mk")
        .expect("BMonoid_mk registered")
        .clone();
    let mut b_check = std_ctx();
    b_check.add_indexed_inductive("BMonoid", b_sort, 0);
    let verdict = b_check.add_constructor_checked("BMonoid_mk", "BMonoid", b_ctor.clone());
    assert!(
        verdict.is_ok(),
        "`add_structure` must not register a constructor that the CIC universe check rejects — \
         at the sort it inferred, its own constructor passes the checked gate: {verdict:?}",
    );
    // …and under a `Type 0` header — the Girard/Hurkens shape — the same constructor is refused,
    // exactly as Lean refuses `structure BMonoid : Type where carrier : Type`.
    let mut pinned = std_ctx();
    pinned.add_indexed_inductive("BMonoid", ty0(), 0);
    let refused = pinned.add_constructor_checked("BMonoid_mk", "BMonoid", b_ctor);
    assert!(
        matches!(&refused, Err(e) if e.to_string().contains("universe inconsistency")),
        "a `Type 0` inductive storing a `Type 0` field is refused by the checked gate: {refused:?}",
    );
    assert!(
        pinned.get_constructors("BMonoid").is_empty(),
        "…and the refused constructor registers nothing"
    );
}

// ============================================================================================
// T07-013 · Instance diamond INCOHERENCE — two paths that disagree, never compared
// ============================================================================================

/// Two paths to one parent carrying different parent data. LOGOS cannot even ask the question:
/// `add_instance` is an unconditional `push` with no lookup, and `resolve_instance_at` returns
/// at the FIRST candidate that fully resolves, so the second path is never constructed and the
/// two dictionaries are never compared. Structure eta does not save it — eta relates a variable
/// to its own reconstruction, not `Base_mk α 1` to `Base_mk α 2` — and proof irrelevance does
/// not apply because the dictionary carries DATA.
///
/// The MUST is refusal rather than an arbitrary pick: everything type-checks either way, so a
/// silent choice is a mistranslation that surfaces as an unrelated proof failing `def_eq` far
/// from the diamond. An unsound answer is worse than no answer.
#[test]
fn an_incoherent_diamond_is_not_silently_resolved() {
    let mut ctx = std_ctx();
    ctx.add_structure("Base", &[("a", ty0())], &[("stamp", nat())]);
    ctx.add_structure(
        "Left",
        &[("a", ty0())],
        &[("stamp", nat()), ("leftMark", nat())],
    );
    ctx.add_structure(
        "Right",
        &[("a", ty0())],
        &[("stamp", nat()), ("rightMark", nat())],
    );
    register_instance(
        &mut ctx,
        "leftNat",
        app(g("Left"), nat()),
        apps(g("Left_mk"), &[nat(), peano(1), peano(10)]),
    );
    register_instance(
        &mut ctx,
        "rightNat",
        app(g("Right"), nat()),
        apps(g("Right_mk"), &[nat(), peano(2), peano(20)]),
    );
    for child in ["Left", "Right"] {
        let name = format!("{child}_toBase");
        let ty = pi(
            "a",
            ty0(),
            arrow(app(g(child), var("a")), app(g("Base"), var("a"))),
        );
        let body = lam(
            "a",
            ty0(),
            lam(
                "c",
                app(g(child), var("a")),
                apps(
                    g("Base_mk"),
                    &[
                        var("a"),
                        apps(g(&format!("{child}_stamp")), &[var("a"), var("c")]),
                    ],
                ),
            ),
        );
        register_instance(&mut ctx, &name, ty, body);
    }

    // The incoherence is real: the two paths disagree, and nothing in the conversion relates
    // them — `bothPaths : viaLeft ≠ viaRight`.
    let via_left = apps(g("Left_toBase"), &[nat(), g("leftNat")]);
    let via_right = apps(g("Right_toBase"), &[nat(), g("rightNat")]);
    assert_eq!(
        normalize(&ctx, &apps(g("Base_stamp"), &[nat(), via_left.clone()])),
        peano(1),
        "viaLeft stamps 1",
    );
    assert_eq!(
        normalize(&ctx, &apps(g("Base_stamp"), &[nat(), via_right.clone()])),
        peano(2),
        "viaRight stamps 2",
    );
    assert!(
        !defeq_for_test(&ctx, &via_left, &via_right),
        "the two parent dictionaries carry data and are genuinely unequal — proof irrelevance \
         does not apply and structure eta does not relate them",
    );

    // `def implicitPath (α) [Both α] : Nat := readStamp α` — what real code writes. Both paths
    // discharge the goal and LOGOS never discovers the second one exists.
    let forget_edges: Vec<&Term> = ctx.instances().iter().map(|(_, value)| value).collect();
    assert!(
        forget_edges.contains(&&g("Left_toBase")) && forget_edges.contains(&&g("Right_toBase")),
        "the diamond must really have two forget edges registered: {forget_edges:?}",
    );
    assert!(
        resolve(&ctx, &app(g("Base"), nat())).is_none(),
        "a `Base Nat` goal reachable by two paths whose dictionaries are not definitionally \
         equal must be REFUSED, not resolved to whichever forget edge was registered first — \
         everything type-checks either way, so a silent choice surfaces as an unrelated proof \
         failing def_eq arbitrarily far from the diamond",
    );
}

// ============================================================================================
// T07-014 · Method projection — the projection applied parameters-first, then the dictionary
// ============================================================================================

/// A typeclass method IS a field projection. `add_structure` emits `{name}_{field}` as an
/// ordinary DEFINITION whose body is a `Term::Match` on the dictionary, with the structure's
/// PARAMETERS bound before the receiver — there is no `Term::Proj`. Because it is a definition
/// and not an axiom, `C_method α (C_mk α f) a` ι-computes, which is what makes every `rfl`
/// through an instance work; and because `add_structure` registers the `StructInfo`, the
/// reconstruction `⟨c.f⟩ ≡ c` holds inside `def_eq`.
#[test]
fn a_method_call_is_a_projection_applied_parameters_first() {
    let mut ctx = std_ctx();
    ctx.add_structure("Pointed", &[("a", ty0())], &[("point", var("a"))]);
    register_instance(
        &mut ctx,
        "pointedNat",
        app(g("Pointed"), nat()),
        apps(g("Pointed_mk"), &[nat(), peano(7)]),
    );

    // `usePoint (α) [Pointed α] : α := Pointed.point`, then `computesThroughInstance : … = 7`.
    let use_point_ty = pi("a", ty0(), arrow(app(g("Pointed"), var("a")), var("a")));
    let mut mctx = MetaCtx::new();
    let (term, ty) = elaborate_app_against(
        &ctx,
        &mut mctx,
        &g("Pointed_point"),
        &use_point_ty,
        &[ParamKind::Implicit, ParamKind::Instance],
        &[],
        Some(&nat()),
    )
    .expect("the expected type pins α := Nat, then the dictionary resolves");
    assert_eq!(
        term,
        apps(g("Pointed_point"), &[nat(), g("pointedNat")]),
        "the method call is the projection applied to the PARAMETER, then the dictionary",
    );
    assert_eq!(ty, nat());
    assert_eq!(
        normalize(&ctx, &term),
        peano(7),
        "computesThroughInstance: the projection unfolds to its Match and ι-reduces against the \
         constructor — an opaque axiom would break every `rfl` through an instance",
    );
    // Directly against a literal dictionary, with no resolution involved.
    assert_eq!(
        normalize(
            &ctx,
            &apps(
                g("Pointed_point"),
                &[nat(), apps(g("Pointed_mk"), &[nat(), peano(7)])]
            )
        ),
        peano(7),
        "computesOnLiteral",
    );

    // TWO class parameters: the projection binds BOTH before the receiver.
    ctx.add_structure(
        "Convert",
        &[("a", ty0()), ("b", ty0())],
        &[("convert", arrow(var("a"), var("b")))],
    );
    let convert_nat_bool = apps(
        g("Convert_mk"),
        &[nat(), g("Bool"), lam("n", nat(), g("true"))],
    );
    let two_params = apps(
        g("Convert_convert"),
        &[nat(), g("Bool"), convert_nat_bool.clone(), peano(3)],
    );
    assert_eq!(
        infer_type(&ctx, &two_params).expect("twoParams type-checks"),
        g("Bool"),
        "twoParams Nat Bool 3 : Bool",
    );
    assert_eq!(
        normalize(&ctx, &two_params),
        g("true"),
        "twoParams Nat Bool 3 = true"
    );
    // Receiver-first is the wrong telescope and must not type-check.
    let receiver_first = apps(
        g("Convert_convert"),
        &[convert_nat_bool, nat(), g("Bool"), peano(3)],
    );
    assert!(
        infer_type(&ctx, &receiver_first).is_err(),
        "the receiver must come AFTER the class parameters, never before",
    );

    // A method with its own explicit arguments, applied after the receiver.
    register_nat_add(&mut ctx);
    ctx.add_structure(
        "Binop",
        &[("a", ty0())],
        &[("op", arrow(var("a"), arrow(var("a"), var("a"))))],
    );
    let binop_nat = apps(g("Binop_mk"), &[nat(), g("natAdd")]);
    assert_eq!(
        normalize(
            &ctx,
            &apps(
                g("Binop_op"),
                &[nat(), binop_nat.clone(), peano(2), peano(3)]
            )
        ),
        peano(5),
        "useBinop Nat 2 3 = 5",
    );
    // `dotOnDictionary (inst : Binop Nat) (a b : Nat) := inst.op a b` — the same projection,
    // reached from the head Global of the receiver's type.
    let mut dotted = ctx.clone();
    dotted.add_declaration("inst", app(g("Binop"), nat()));
    assert!(
        infer_type(
            &dotted,
            &apps(g("Binop_op"), &[nat(), g("inst"), peano(3), peano(4)])
        )
        .is_ok(),
        "dot notation on an explicit dictionary resolves to `{{Class}}_{{field}}`",
    );

    // ETA ROUND TRIP: rebuilding a dictionary from its projections is def-eq to the original.
    let mut eta = ctx.clone();
    eta.add_declaration("p", app(g("Pointed"), nat()));
    let rebuilt = apps(
        g("Pointed_mk"),
        &[nat(), apps(g("Pointed_point"), &[nat(), g("p")])],
    );
    assert!(
        defeq_for_test(&eta, &rebuilt, &g("p")),
        "etaRoundTrip: ⟨Pointed.point⟩ ≡ inst, via the StructInfo `add_structure` registered",
    );
    assert!(
        matches!(double_check(&eta, &rebuilt), DoubleCheck::Agreed),
        "and both kernels agree"
    );
}
