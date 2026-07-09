//! Dot / projection notation (E4) — `p.field` elaborates to `Struct_field params p`, the
//! projection of `p`'s structure applied to `p` (with the structure's parameters filled
//! from `p`'s type). This is the elaboration logic the surface `x.f` desugars to.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{elaborate_dot, infer_type, Context, MetaCtx, Term, Universe};

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
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
/// A `Prod` structure with `fst`/`snd` projections, registered via K4's `add_structure`.
fn prod_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    // Prod (A B : Type) := mk (fst : A) (snd : B)
    ctx.add_structure("Prod", &[("A", ty0()), ("B", ty0())], &[("fst", v("A")), ("snd", v("B"))]);
    ctx
}

#[test]
fn dot_projects_a_structure_field() {
    let ctx = prod_ctx();
    // p := Prod_mk Nat Bool Zero true : Prod Nat Bool
    let p = apps(g("Prod_mk"), &[g("Nat"), g("Bool"), g("Zero"), g("true")]);
    let mut mctx = MetaCtx::new();

    // p.fst  ⇒  Prod_fst Nat Bool p   :  Nat
    let fst = elaborate_dot(&ctx, &mut mctx, &p, "fst").expect("p.fst elaborates");
    assert_eq!(fst, apps(g("Prod_fst"), &[g("Nat"), g("Bool"), p.clone()]));
    assert_eq!(infer_type(&ctx, &fst).unwrap(), g("Nat"), "p.fst : Nat");

    // p.snd  ⇒  Prod_snd Nat Bool p   :  Bool
    let snd = elaborate_dot(&ctx, &mut mctx, &p, "snd").expect("p.snd elaborates");
    assert_eq!(snd, apps(g("Prod_snd"), &[g("Nat"), g("Bool"), p]));
    assert_eq!(infer_type(&ctx, &snd).unwrap(), g("Bool"), "p.snd : Bool");
}

#[test]
fn dot_on_an_unknown_field_fails() {
    let ctx = prod_ctx();
    let p = apps(g("Prod_mk"), &[g("Nat"), g("Bool"), g("Zero"), g("true")]);
    let mut mctx = MetaCtx::new();
    assert!(
        elaborate_dot(&ctx, &mut mctx, &p, "nope").is_err(),
        "an unknown projection must fail, not silently produce a bad term"
    );
}
