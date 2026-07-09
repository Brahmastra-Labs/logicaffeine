//! R4 — the elaborator, locked in by TDD.
//!
//! Two layers, each pinned to absurdity. (1) The UNIFICATION engine: metavariables solve
//! to concrete terms, decompose through application and `Π`, respect the occurs-check,
//! chain transitively, and stay consistent once solved. (2) IMPLICIT-ARGUMENT inference:
//! the user omits a type argument and the elaborator infers it from the value's type,
//! producing a fully-explicit term that the KERNEL then certifies — elaboration is never
//! trusted, it only constructs. A wrong inference would either fail to type-check or solve
//! the metavariable to the wrong thing; both are caught here.

use logicaffeine_kernel::elaborate::is_meta;
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    elaborate, elaborate_app, infer_type, instantiate, normalize, unify, Context, MetaCtx,
    ParamKind, Term, Universe,
};

// --- builders ---
fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn lam(p: &str, t: Term, b: Term) -> Term {
    Term::Lambda { param: p.to_string(), param_type: Box::new(t), body: Box::new(b) }
}
fn pi(p: &str, t: Term, b: Term) -> Term {
    Term::Pi { param: p.to_string(), param_type: Box::new(t), body_type: Box::new(b) }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
/// Whether a term still contains any metavariable (`?n`) — an un-elaborated leftover.
fn has_meta(t: &Term) -> bool {
    match t {
        Term::Var(n) => is_meta(n),
        Term::App(f, a) => has_meta(f) || has_meta(a),
        Term::Pi { param_type, body_type, .. } => has_meta(param_type) || has_meta(body_type),
        Term::Lambda { param_type, body, .. } => has_meta(param_type) || has_meta(body),
        Term::Match { discriminant, motive, cases } => {
            has_meta(discriminant) || has_meta(motive) || cases.iter().any(has_meta)
        }
        Term::Fix { body, .. } => has_meta(body),
        _ => false,
    }
}

/// Register `id : Π(A:Type0). A → A := λA. λa. a` as a definition, returning its type.
fn register_id(ctx: &mut Context) -> Term {
    let id_ty = pi("A", ty0(), arrow(var("A"), var("A")));
    let id_body = lam("A", ty0(), lam("a", var("A"), var("a")));
    ctx.add_definition("id".to_string(), id_ty.clone(), id_body);
    id_ty
}

// ===========================================================================
// UNIFICATION ENGINE
// ===========================================================================

#[test]
fn metavariable_unifies_with_a_concrete_term() {
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    assert!(unify(&ctx, &mut m, &mv, &g("Nat")), "?0 =?= Nat");
    // The solution is recorded, and instantiation realizes it.
    assert_eq!(instantiate(&mv, &m), g("Nat"));
}

#[test]
fn unification_is_symmetric() {
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    assert!(unify(&ctx, &mut m, &g("Nat"), &mv), "Nat =?= ?0 solves the metavariable");
    assert_eq!(instantiate(&mv, &m), g("Nat"));
}

#[test]
fn equal_concretes_unify_and_distinct_ones_do_not() {
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    assert!(unify(&ctx, &mut m, &g("Nat"), &g("Nat")), "Nat =?= Nat");
    assert!(!unify(&ctx, &mut m, &g("Nat"), &g("Bool")), "Nat ≠ Bool");
}

#[test]
fn unification_descends_through_application() {
    // f ?0 =?= f Nat  solves ?0 := Nat (structural decomposition).
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    let lhs = app(g("Succ"), mv.clone());
    let rhs = app(g("Succ"), g("Zero"));
    assert!(unify(&ctx, &mut m, &lhs, &rhs));
    assert_eq!(instantiate(&mv, &m), g("Zero"));
}

#[test]
fn unification_descends_through_pi() {
    // (?0 → ?0) =?= (Nat → Nat)  solves ?0 := Nat.
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    let lhs = arrow(mv.clone(), mv.clone());
    let rhs = arrow(g("Nat"), g("Nat"));
    assert!(unify(&ctx, &mut m, &lhs, &rhs));
    assert_eq!(instantiate(&mv, &m), g("Nat"));
}

#[test]
fn occurs_check_rejects_a_cyclic_solution() {
    // ?0 =?= Succ ?0  would make ?0 = Succ (Succ (… )), an infinite term. Reject.
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    let cyclic = app(g("Succ"), mv.clone());
    assert!(!unify(&ctx, &mut m, &mv, &cyclic), "the occurs-check must reject ?0 := Succ ?0");
}

#[test]
fn metavariables_chain_transitively() {
    // ?0 =?= ?1, then ?1 =?= Nat  ⇒  ?0 resolves to Nat.
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    let a = m.fresh();
    let b = m.fresh();
    assert!(unify(&ctx, &mut m, &a, &b));
    assert!(unify(&ctx, &mut m, &b, &g("Nat")));
    assert_eq!(instantiate(&a, &m), g("Nat"), "?0 follows ?1 to Nat");
}

#[test]
fn a_solved_metavariable_stays_consistent() {
    // Once ?0 := Nat, unifying ?0 with Bool must FAIL (no clobbering).
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    assert!(unify(&ctx, &mut m, &mv, &g("Nat")));
    assert!(!unify(&ctx, &mut m, &mv, &g("Bool")), "a solved metavariable cannot be re-bound");
    assert!(unify(&ctx, &mut m, &mv, &g("Nat")), "but re-confirming the same solution is fine");
}

#[test]
fn sorts_unify_by_universe_equivalence() {
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    assert!(unify(&ctx, &mut m, &ty0(), &ty0()), "Type 0 =?= Type 0");
    assert!(!unify(&ctx, &mut m, &ty0(), &Term::Sort(Universe::Type(1))), "Type 0 ≠ Type 1");
}

// ===========================================================================
// IMPLICIT-ARGUMENT INFERENCE — the headline
// ===========================================================================

#[test]
fn implicit_type_argument_is_inferred_from_the_value() {
    // The user writes `id 0`; `id : Π{A:Type}. A → A`. The elaborator must infer A := Nat
    // and produce the fully-explicit `id Nat Zero : Nat`, which the kernel certifies.
    let mut ctx = std_ctx();
    let id_ty = register_id(&mut ctx);
    let mut m = MetaCtx::new();

    // mask: first parameter (A) implicit, second (the value) explicit.
    let (term, ty) =
        elaborate_app(&ctx, &mut m, &g("id"), &id_ty, &[ParamKind::Implicit, ParamKind::Explicit], &[g("Zero")]).expect("elab");

    assert_eq!(term, app(app(g("id"), g("Nat")), g("Zero")), "must reconstruct `id Nat Zero`");
    assert_eq!(ty, g("Nat"), "its type is Nat");
    assert!(!has_meta(&term), "the elaborated term has no leftover metavariables");
    assert!(infer_type(&ctx, &term).is_ok(), "and the KERNEL certifies it");
}

#[test]
fn the_same_function_infers_a_different_implicit_per_use() {
    // `id true` must infer A := Bool — the same definition, a different solution.
    let mut ctx = std_ctx();
    let id_ty = register_id(&mut ctx);
    let mut m = MetaCtx::new();

    let (term, ty) =
        elaborate_app(&ctx, &mut m, &g("id"), &id_ty, &[ParamKind::Implicit, ParamKind::Explicit], &[g("true")]).expect("elab");
    assert_eq!(term, app(app(g("id"), g("Bool")), g("true")));
    assert_eq!(ty, g("Bool"));
    assert!(infer_type(&ctx, &term).is_ok());
}

#[test]
fn the_elaborated_application_computes() {
    // `id` is a real definition (λA.λa.a), so the elaborated `id Nat Zero` reduces to Zero.
    let mut ctx = std_ctx();
    let id_ty = register_id(&mut ctx);
    let mut m = MetaCtx::new();
    let (term, _) =
        elaborate_app(&ctx, &mut m, &g("id"), &id_ty, &[ParamKind::Implicit, ParamKind::Explicit], &[g("Zero")]).expect("elab");
    assert_eq!(normalize(&ctx, &term), g("Zero"), "id Nat Zero ⇝ Zero");
}

#[test]
fn an_explicit_argument_of_the_wrong_type_is_rejected() {
    // `pair : Π{A}. A → A → A` applied to `Zero` then `true`: the first arg solves A := Nat,
    // so the second (Bool) cannot unify with A = Nat. Elaboration must FAIL — the implicit
    // ties the two arguments together.
    let mut ctx = std_ctx();
    let pair_ty = pi("A", ty0(), arrow(var("A"), arrow(var("A"), var("A"))));
    ctx.add_declaration("pair", pair_ty.clone());
    let mut m = MetaCtx::new();

    let result = elaborate_app(
        &ctx,
        &mut m,
        &g("pair"),
        &pair_ty,
        &[ParamKind::Implicit, ParamKind::Explicit, ParamKind::Explicit],
        &[g("Zero"), g("true")],
    );
    assert!(result.is_err(), "Bool cannot unify with the already-inferred A = Nat");
}

#[test]
fn a_homogeneous_pair_with_matching_arguments_elaborates() {
    // The positive companion: `pair 0 0` infers A := Nat and both arguments check.
    let mut ctx = std_ctx();
    let pair_ty = pi("A", ty0(), arrow(var("A"), arrow(var("A"), var("A"))));
    ctx.add_declaration("pair", pair_ty.clone());
    let mut m = MetaCtx::new();
    let (term, ty) = elaborate_app(
        &ctx,
        &mut m,
        &g("pair"),
        &pair_ty,
        &[ParamKind::Implicit, ParamKind::Explicit, ParamKind::Explicit],
        &[g("Zero"), g("Zero")],
    )
    .expect("elab");
    assert_eq!(term, app(app(app(g("pair"), g("Nat")), g("Zero")), g("Zero")));
    assert_eq!(ty, g("Nat"));
    assert!(infer_type(&ctx, &term).is_ok());
}

// ===========================================================================
// HOLE-FILLING ELABORATION — the user writes `_`
// ===========================================================================

#[test]
fn an_explicit_hole_is_filled_by_elaboration() {
    // The user writes `id _ Zero`; the elaborator fills `_` with Nat.
    let mut ctx = std_ctx();
    register_id(&mut ctx);
    let mut m = MetaCtx::new();
    let surface = app(app(g("id"), Term::Hole), g("Zero"));

    let (term, ty) = elaborate(&ctx, &mut m, &surface, None).expect("elab");
    let term = instantiate(&term, &m);
    assert_eq!(term, app(app(g("id"), g("Nat")), g("Zero")), "the hole becomes Nat");
    assert_eq!(instantiate(&ty, &m), g("Nat"));
    assert!(!has_meta(&term));
    assert!(infer_type(&ctx, &term).is_ok());
}

// ===========================================================================
// TYPECLASS / INSTANCE RESOLUTION
// ===========================================================================

/// Set up a tiny `Inhabited` typeclass: `Inhabited : Type → Type`, constructor
/// `mk : Π(A). A → Inhabited A`, instances for Nat and Bool, and the eliminator
/// `default_of : Π(A). Inhabited A → A := λA. λi. match i with mk a => a`. Returns
/// (`default_of`'s type, the Nat instance term, the Bool instance term).
fn setup_inhabited(ctx: &mut Context) -> (Term, Term, Term) {
    ctx.add_inductive("Inhabited", pi("A", ty0(), ty0()));
    ctx.add_constructor(
        "mk",
        "Inhabited",
        pi("A", ty0(), arrow(var("A"), app(g("Inhabited"), var("A")))),
    );
    let nat_inst = app(app(g("mk"), g("Nat")), g("Zero")); // : Inhabited Nat
    let bool_inst = app(app(g("mk"), g("Bool")), g("true")); // : Inhabited Bool
    ctx.add_instance(app(g("Inhabited"), g("Nat")), nat_inst.clone());
    ctx.add_instance(app(g("Inhabited"), g("Bool")), bool_inst.clone());

    // default_of : Π(A). Inhabited A → A := λA. λi. match i return (λ_. A) with | mk a => a
    let default_ty = pi("A", ty0(), arrow(app(g("Inhabited"), var("A")), var("A")));
    let default_body = lam(
        "A",
        ty0(),
        lam(
            "i",
            app(g("Inhabited"), var("A")),
            Term::Match {
                discriminant: Box::new(var("i")),
                motive: Box::new(lam("_", app(g("Inhabited"), var("A")), var("A"))),
                cases: vec![lam("a", var("A"), var("a"))],
            },
        ),
    );
    ctx.add_definition("default_of".to_string(), default_ty.clone(), default_body);
    (default_ty, nat_inst, bool_inst)
}

#[test]
fn instance_is_resolved_from_the_database_and_the_result_computes() {
    // `default_of Nat [?inst]`: the elaborator must RESOLVE the instance-implicit argument
    // to `mk Nat Zero` (the registered `Inhabited Nat`), yielding `default_of Nat (mk Nat
    // Zero)`, which the kernel certifies and which COMPUTES to `Zero`.
    let mut ctx = std_ctx();
    let (default_ty, nat_inst, _) = setup_inhabited(&mut ctx);
    let mut m = MetaCtx::new();

    let (term, ty) = elaborate_app(
        &ctx,
        &mut m,
        &g("default_of"),
        &default_ty,
        &[ParamKind::Explicit, ParamKind::Instance],
        &[g("Nat")],
    )
    .expect("elaboration resolves the instance");

    assert_eq!(
        term,
        app(app(g("default_of"), g("Nat")), nat_inst),
        "the Inhabited-Nat instance must be selected"
    );
    assert_eq!(ty, g("Nat"));
    assert!(!has_meta(&term));
    assert!(infer_type(&ctx, &term).is_ok(), "the kernel certifies the resolved term");
    assert_eq!(normalize(&ctx, &term), g("Zero"), "default_of Nat ⇝ Zero");
}

#[test]
fn resolution_selects_the_instance_matching_the_type() {
    // The SAME function with `Bool` must select the Bool instance and compute to `true`.
    let mut ctx = std_ctx();
    let (default_ty, _, bool_inst) = setup_inhabited(&mut ctx);
    let mut m = MetaCtx::new();
    let (term, ty) = elaborate_app(
        &ctx,
        &mut m,
        &g("default_of"),
        &default_ty,
        &[ParamKind::Explicit, ParamKind::Instance],
        &[g("Bool")],
    )
    .expect("elab");
    assert_eq!(term, app(app(g("default_of"), g("Bool")), bool_inst));
    assert_eq!(ty, g("Bool"));
    assert_eq!(normalize(&ctx, &term), g("true"), "default_of Bool ⇝ true");
}

#[test]
fn instance_resolution_is_deferred_until_the_type_variable_is_solved() {
    // `pick : Π{A}. [Inhabited A]. A → A`. The instance parameter precedes the value, so
    // when it is reached `A` is still a metavariable — resolution MUST be deferred until
    // the explicit value `Zero` pins `A := Nat`, then `Inhabited Nat` resolves. This is the
    // whole point of deferring: `pick 0` works without the user naming the type.
    let mut ctx = std_ctx();
    let (_default_ty, nat_inst, _) = setup_inhabited(&mut ctx);
    // pick : Π(A). Inhabited A → A → A := λA. λi. λa. a
    let pick_ty =
        pi("A", ty0(), arrow(app(g("Inhabited"), var("A")), arrow(var("A"), var("A"))));
    let pick_body =
        lam("A", ty0(), lam("i", app(g("Inhabited"), var("A")), lam("a", var("A"), var("a"))));
    ctx.add_definition("pick".to_string(), pick_ty.clone(), pick_body);

    let mut m = MetaCtx::new();
    let (term, ty) = elaborate_app(
        &ctx,
        &mut m,
        &g("pick"),
        &pick_ty,
        &[ParamKind::Implicit, ParamKind::Instance, ParamKind::Explicit],
        &[g("Zero")],
    )
    .expect("deferred resolution succeeds");

    assert_eq!(
        term,
        app(app(app(g("pick"), g("Nat")), nat_inst), g("Zero")),
        "A inferred Nat from the value, THEN the Inhabited Nat instance resolved"
    );
    assert_eq!(ty, g("Nat"));
    assert!(!has_meta(&term));
    assert!(infer_type(&ctx, &term).is_ok());
}

#[test]
fn an_unresolvable_instance_is_an_error() {
    // No `Inhabited Entity` instance is registered, so resolving it must fail loudly
    // rather than silently leaving a hole.
    let mut ctx = std_ctx();
    let (default_ty, _, _) = setup_inhabited(&mut ctx);
    let mut m = MetaCtx::new();
    let result = elaborate_app(
        &ctx,
        &mut m,
        &g("default_of"),
        &default_ty,
        &[ParamKind::Explicit, ParamKind::Instance],
        &[g("Entity")],
    );
    assert!(result.is_err(), "no Inhabited Entity instance ⇒ resolution must fail");
}

// ===========================================================================
// POLYMORPHIC / RECURSIVE INSTANCES
// ===========================================================================

/// Extend the `Inhabited` setup with a POLYMORPHIC instance:
/// `list_inst : Π(A). Inhabited A → Inhabited (TList A)
///    := λA. λia. mk (TList A) (TCons A (default_of A ia) (TNil A))`
/// — i.e. "TList A is inhabited (by the singleton [default A]) whenever A is."
fn add_list_instance(ctx: &mut Context) {
    let list_ty = pi(
        "A",
        ty0(),
        arrow(app(g("Inhabited"), var("A")), app(g("Inhabited"), app(g("TList"), var("A")))),
    );
    let list_body = lam(
        "A",
        ty0(),
        lam(
            "ia",
            app(g("Inhabited"), var("A")),
            app(
                app(g("mk"), app(g("TList"), var("A"))),
                app(
                    app(
                        app(g("TCons"), var("A")),
                        app(app(g("default_of"), var("A")), var("ia")),
                    ),
                    app(g("TNil"), var("A")),
                ),
            ),
        ),
    );
    ctx.add_definition("list_inst".to_string(), list_ty.clone(), list_body);
    ctx.add_instance(list_ty, g("list_inst"));
}

#[test]
fn a_polymorphic_instance_resolves_its_premise_recursively() {
    // Resolving `Inhabited (TList Nat)` must find `list_inst`, RECURSIVELY resolve its
    // `Inhabited A` premise (A := Nat) to the Nat instance, and assemble
    // `list_inst Nat (mk Nat Zero)`. The kernel certifies it, and it computes to `[Zero]`.
    let mut ctx = std_ctx();
    let (default_ty, nat_inst, _) = setup_inhabited(&mut ctx);
    add_list_instance(&mut ctx);

    let tlist_nat = app(g("TList"), g("Nat"));
    let mut m = MetaCtx::new();
    let (term, ty) = elaborate_app(
        &ctx,
        &mut m,
        &g("default_of"),
        &default_ty,
        &[ParamKind::Explicit, ParamKind::Instance],
        &[tlist_nat.clone()],
    )
    .expect("recursive resolution succeeds");

    assert_eq!(
        term,
        app(app(g("default_of"), tlist_nat.clone()), app(app(g("list_inst"), g("Nat")), nat_inst)),
        "list_inst applied to Nat and the recursively-resolved Inhabited-Nat instance"
    );
    assert_eq!(ty, tlist_nat);
    assert!(!has_meta(&term));
    assert!(infer_type(&ctx, &term).is_ok(), "the kernel certifies the recursively-resolved term");
    // The default of `TList Nat` is the singleton list `[Zero]`.
    let singleton = app(app(app(g("TCons"), g("Nat")), g("Zero")), app(g("TNil"), g("Nat")));
    assert_eq!(normalize(&ctx, &term), singleton, "default_of (TList Nat) ⇝ [Zero]");
}

#[test]
fn instance_resolution_recurses_through_two_levels() {
    // `Inhabited (TList (TList Nat))` needs `list_inst` TWICE: the outer premise
    // `Inhabited (TList Nat)` is itself resolved by `list_inst`, whose premise
    // `Inhabited Nat` bottoms out at the Nat instance. A genuine two-level search.
    let mut ctx = std_ctx();
    let (default_ty, nat_inst, _) = setup_inhabited(&mut ctx);
    add_list_instance(&mut ctx);

    let tlist_nat = app(g("TList"), g("Nat"));
    let tlist_tlist_nat = app(g("TList"), tlist_nat.clone());
    let mut m = MetaCtx::new();
    let (term, ty) = elaborate_app(
        &ctx,
        &mut m,
        &g("default_of"),
        &default_ty,
        &[ParamKind::Explicit, ParamKind::Instance],
        &[tlist_tlist_nat.clone()],
    )
    .expect("two-level resolution succeeds");

    let inner = app(app(g("list_inst"), g("Nat")), nat_inst); // Inhabited (TList Nat)
    let outer = app(app(g("list_inst"), tlist_nat), inner); // Inhabited (TList (TList Nat))
    assert_eq!(term, app(app(g("default_of"), tlist_tlist_nat.clone()), outer));
    assert_eq!(ty, tlist_tlist_nat);
    assert!(!has_meta(&term));
    assert!(infer_type(&ctx, &term).is_ok());
}

#[test]
fn a_polymorphic_instance_does_not_fire_without_its_premise() {
    // `TList Entity`: `list_inst` matches the conclusion (A := Entity), but its premise
    // `Inhabited Entity` has no instance, so resolution must FAIL rather than fabricate one.
    let mut ctx = std_ctx();
    let (default_ty, _, _) = setup_inhabited(&mut ctx);
    add_list_instance(&mut ctx);
    let mut m = MetaCtx::new();
    let result = elaborate_app(
        &ctx,
        &mut m,
        &g("default_of"),
        &default_ty,
        &[ParamKind::Explicit, ParamKind::Instance],
        &[app(g("TList"), g("Entity"))],
    );
    assert!(result.is_err(), "no Inhabited Entity ⇒ the List premise cannot be discharged");
}

// ===========================================================================
// MOTIVE INFERENCE — the payoff of higher-order pattern unification
// ===========================================================================

#[test]
fn an_eliminator_motive_is_inferred_by_pattern_unification() {
    // elim : {P : Nat -> Type} -> (forall n : Nat, P n) -> P Zero
    // Applied to `f : forall n : Nat, Vec n`, the MOTIVE P is a higher-order metavariable.
    // Unifying f's type `forall n. Vec n` against the domain `forall n. ?P n` descends
    // under the `n` binder and hits the pattern `?P n =?= Vec n` ⇒ ?P := λn. Vec n. The
    // result type is then `?P Zero` ⇝ `Vec Zero`. THIS is what pattern unification buys.
    let mut ctx = std_ctx();
    ctx.add_inductive("Vec", pi("_", g("Nat"), ty0())); // Vec : Nat -> Type0

    let elim_ty = pi(
        "P",
        pi("_", g("Nat"), ty0()), // P : Nat -> Type
        arrow(
            pi("n", g("Nat"), app(var("P"), var("n"))), // forall n. P n
            app(var("P"), g("Zero")),                   // P Zero
        ),
    );
    ctx.add_declaration("elim", elim_ty.clone());
    ctx.add_declaration("f", pi("n", g("Nat"), app(g("Vec"), var("n")))); // f : forall n. Vec n

    let mut m = MetaCtx::new();
    let (term, ty) = elaborate_app(
        &ctx,
        &mut m,
        &g("elim"),
        &elim_ty,
        &[ParamKind::Implicit, ParamKind::Explicit],
        &[g("f")],
    )
    .expect("the motive is inferred");

    assert_eq!(
        normalize(&ctx, &ty),
        app(g("Vec"), g("Zero")),
        "motive P := λn. Vec n inferred ⇒ result type P Zero ⇝ Vec Zero"
    );
    assert!(!has_meta(&term), "no leftover metavariables");
    assert!(infer_type(&ctx, &term).is_ok(), "the kernel certifies the motive-inferred term");
}

#[test]
fn a_motive_is_inferred_under_a_lambda_binder() {
    // Elaborate `fun n : Nat => g_vec n` against the expected type `forall m : Nat. ?P m`.
    // Threading `n` into the local context turns the body's reconciliation
    // `Vec n =?= ?P n` into a Miller pattern ⇒ ?P := λn. Vec n — a motive inferred UNDER a
    // binder, which only works because `elaborate` now carries the context.
    let mut ctx = std_ctx();
    ctx.add_inductive("Vec", pi("_", g("Nat"), ty0()));
    ctx.add_declaration("g_vec", pi("n", g("Nat"), app(g("Vec"), var("n")))); // forall n. Vec n

    let mut m = MetaCtx::new();
    let pmeta = m.fresh(); // ?P : Nat -> Type
    let expected = pi("mm", g("Nat"), app(pmeta.clone(), var("mm"))); // forall mm. ?P mm
    let lam_term = lam("n", g("Nat"), app(g("g_vec"), var("n"))); // fun n => g_vec n

    let (term, _ty) =
        elaborate(&ctx, &mut m, &lam_term, Some(&expected)).expect("elaborate under the binder");

    // The motive was solved by pattern unification under the lambda: ?P := λn. Vec n.
    let p_zero = normalize(&ctx, &instantiate(&app(pmeta, g("Zero")), &m));
    assert_eq!(p_zero, app(g("Vec"), g("Zero")), "?P Zero ⇝ Vec Zero (motive inferred)");
    assert!(infer_type(&ctx, &term).is_ok());
}
