//! Per-binder implicit/explicit/instance info (E2) — implicit and explicit parameters may
//! INTERLEAVE freely (Lean's `BinderInfo`), not merely "all implicits first". `f {A} (x:A)
//! {B} (y:B) : A` applied to `f a b` must infer `A` from `a` and `B` from `b`, with the
//! implicits inserted in their real positions.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, surface_elaborate, Context, ParamKind, Term, Universe};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |a, x| app(a, x.clone()))
}
fn pi(p: &str, t: Term, b: Term) -> Term {
    Term::Pi { param: p.to_string(), param_type: Box::new(t), body_type: Box::new(b) }
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

#[test]
fn interleaved_implicit_and_explicit_parameters_are_inferred() {
    let mut ctx = std_ctx();
    // f : Π{A:Type}. Π(x:A). Π{B:Type}. Π(y:B). A   — implicit A, explicit x, implicit B, explicit y.
    let f_ty = pi("A", ty0(), pi("x", v("A"), pi("B", ty0(), pi("y", v("B"), v("A")))));
    ctx.add_declaration("f", f_ty);
    ctx.set_binder_kinds(
        "f",
        vec![ParamKind::Implicit, ParamKind::Explicit, ParamKind::Implicit, ParamKind::Explicit],
    );

    // `f Zero true` — Zero : Nat pins A, true : Bool pins B (an implicit BETWEEN the two
    // explicit arguments). The old "leading implicits" model could not express this.
    let elab = surface_elaborate(&ctx, &apps(g("f"), &[g("Zero"), g("true")]))
        .expect("interleaved implicits elaborate");
    // Both implicits inserted in position: f Nat Zero Bool true.
    assert_eq!(elab, apps(g("f"), &[g("Nat"), g("Zero"), g("Bool"), g("true")]));
    assert_eq!(infer_type(&ctx, &elab).unwrap(), g("Nat"), "result type is A = Nat");
}

#[test]
fn trailing_implicit_after_explicit_is_still_inferred() {
    // g : Π(x:Nat). Π{A:Type}. A → A   — an implicit AFTER an explicit. `g Zero true` infers
    // A = Bool from the final explicit.
    let mut ctx = std_ctx();
    let g_ty = pi("x", g("Nat"), pi("A", ty0(), pi("y", v("A"), v("A"))));
    ctx.add_declaration("gmix", g_ty);
    ctx.set_binder_kinds(
        "gmix",
        vec![ParamKind::Explicit, ParamKind::Implicit, ParamKind::Explicit],
    );
    let elab = surface_elaborate(&ctx, &apps(g("gmix"), &[g("Zero"), g("true")]))
        .expect("trailing implicit elaborates");
    assert_eq!(elab, apps(g("gmix"), &[g("Zero"), g("Bool"), g("true")]));
    assert_eq!(infer_type(&ctx, &elab).unwrap(), g("Bool"));
}
