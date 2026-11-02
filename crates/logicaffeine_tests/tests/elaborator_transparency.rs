use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    infer_type, normalize, surface_elaborate, unify, Context, MetaCtx, Reducibility, Term, Universe,
};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.declare("sealedZero", g("Nat"), g("Zero")).unwrap();
    ctx
}

#[test]
fn elaboration_respects_irreducibility_while_kernel_conversion_keeps_the_body() {
    let mut ctx = context();
    ctx.set_reducibility("sealedZero", Reducibility::Irreducible);
    assert!(!unify(
        &ctx,
        &mut MetaCtx::new(),
        &g("sealedZero"),
        &g("Zero")
    ));
    assert_eq!(normalize(&ctx, &g("sealedZero")), g("Zero"));
    assert!(unify(
        &ctx,
        &mut MetaCtx::new(),
        &g("sealedZero"),
        &g("sealedZero")
    ));
}

#[test]
fn elaboration_does_not_unfold_an_irreducible_function_type_alias() {
    let mut ctx = context();
    let function_type = Term::Pi {
        param: "_".into(),
        param_type: Box::new(g("Nat")),
        body_type: Box::new(g("Nat")),
    };
    ctx.declare(
        "HiddenFunction",
        Term::Sort(Universe::Type(0)),
        function_type,
    )
    .unwrap();
    ctx.add_declaration("f", g("HiddenFunction"));
    ctx.set_reducibility("HiddenFunction", Reducibility::Irreducible);
    let applied = Term::App(Box::new(g("f")), Box::new(g("Zero")));
    assert!(surface_elaborate(&ctx, &applied).is_err());
    assert_eq!(infer_type(&ctx, &applied).unwrap(), g("Nat"));
}

#[test]
fn ending_a_local_irreducible_attribute_restores_elaboration() {
    let mut ctx = context();
    assert!(unify(
        &ctx,
        &mut MetaCtx::new(),
        &g("sealedZero"),
        &g("Zero")
    ));
    ctx.push_scope();
    ctx.set_reducibility("sealedZero", Reducibility::Irreducible);
    assert!(!unify(
        &ctx,
        &mut MetaCtx::new(),
        &g("sealedZero"),
        &g("Zero")
    ));
    ctx.pop_scope();
    assert!(unify(
        &ctx,
        &mut MetaCtx::new(),
        &g("sealedZero"),
        &g("Zero")
    ));
}
