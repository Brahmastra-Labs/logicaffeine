//! Elaborator coercions (E1) — when an argument's type does not match the expected
//! parameter type but a registered coercion bridges them, the elaborator inserts the
//! coercion (Lean's `↑`/`Coe`). This is the classic "makes it pleasant to write" feature:
//! `f n` where `f : Int → Int` and `n : Nat` elaborates to `f (natToInt n)`.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, surface_elaborate, Context, Term};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi { param: "_".to_string(), param_type: Box::new(a), body_type: Box::new(b) }
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
/// Standard prelude + `natToInt : Nat → Int`, `f : Int → Int`, and the coercion `Nat ⇒ Int`.
fn coe_ctx() -> Context {
    let mut ctx = std_ctx();
    ctx.add_declaration("natToInt", arrow(g("Nat"), g("Int")));
    ctx.add_declaration("f", arrow(g("Int"), g("Int")));
    ctx.add_coercion(g("Nat"), g("Int"), g("natToInt"));
    ctx
}

#[test]
fn coercion_inserted_on_argument_type_mismatch() {
    let ctx = coe_ctx();
    // `f Zero` — `Zero : Nat`, but `f` wants `Int`. The elaborator inserts `natToInt`.
    let elab = surface_elaborate(&ctx, &app(g("f"), g("Zero"))).expect("elaborates with coercion");
    assert_eq!(
        elab,
        app(g("f"), app(g("natToInt"), g("Zero"))),
        "the coercion `natToInt` must be inserted around the argument"
    );
    assert_eq!(infer_type(&ctx, &elab).unwrap(), g("Int"), "the coerced application type-checks to Int");
}

#[test]
fn no_coercion_when_types_already_match() {
    // `f (natToInt Zero)` — the argument is already `Int`, so NO extra coercion is inserted
    // (the elaborator must not double-wrap).
    let ctx = coe_ctx();
    let already_int = app(g("natToInt"), g("Zero"));
    let elab = surface_elaborate(&ctx, &app(g("f"), already_int.clone())).expect("elaborates");
    assert_eq!(elab, app(g("f"), already_int), "a well-typed argument is left untouched");
}

#[test]
fn elaboration_fails_without_a_matching_coercion() {
    // Remove the coercion: `f Zero` can no longer be repaired and must fail to elaborate
    // (or produce an ill-typed term) — coercion is not invented out of thin air.
    let mut ctx = std_ctx();
    ctx.add_declaration("f", arrow(g("Int"), g("Int")));
    let res = surface_elaborate(&ctx, &app(g("f"), g("Zero")));
    let ill = res.map(|t| infer_type(&ctx, &t).is_ok()).unwrap_or(false);
    assert!(!ill, "without a coercion, `f Zero` must not elaborate to a well-typed term");
}
