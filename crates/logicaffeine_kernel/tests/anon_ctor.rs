//! Anonymous-constructor notation (E3) — `⟨f₀, …, fₙ⟩` elaborated against an expected
//! structure type builds the constructor application with the type parameters filled from
//! the expected type. This is what the surface `⟨…⟩` desugars to.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{elaborate_anon_ctor, infer_type, Context, MetaCtx, Term, Universe};

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
fn prod_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.add_structure("Prod", &[("A", ty0()), ("B", ty0())], &[("fst", v("A")), ("snd", v("B"))]);
    ctx
}

#[test]
fn anonymous_constructor_builds_the_structure() {
    let ctx = prod_ctx();
    let mut mctx = MetaCtx::new();
    // ⟨Zero, true⟩ expected at `Prod Nat Bool`  ⇒  Prod_mk Nat Bool Zero true.
    let expected = apps(g("Prod"), &[g("Nat"), g("Bool")]);
    let anon = elaborate_anon_ctor(&ctx, &mut mctx, &expected, &[g("Zero"), g("true")])
        .expect("anonymous constructor elaborates");
    assert_eq!(anon, apps(g("Prod_mk"), &[g("Nat"), g("Bool"), g("Zero"), g("true")]));
    assert_eq!(infer_type(&ctx, &anon).unwrap(), expected, "⟨Zero,true⟩ : Prod Nat Bool");
}

#[test]
fn anonymous_constructor_needs_the_expected_type() {
    // Without a single-constructor expected type it cannot know what to build.
    let ctx = prod_ctx();
    let mut mctx = MetaCtx::new();
    // `Nat` has TWO constructors (Zero/Succ) — ambiguous for ⟨…⟩.
    assert!(
        elaborate_anon_ctor(&ctx, &mut mctx, &g("Nat"), &[g("Zero")]).is_err(),
        "an anonymous constructor at a multi-constructor type must be rejected"
    );
}
