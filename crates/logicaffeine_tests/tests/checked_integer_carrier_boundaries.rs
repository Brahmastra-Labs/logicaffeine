use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, normalize, recheck, BigInt, Context, Literal, Term};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().cloned().fold(f, app)
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn int(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
#[test]
fn both_kernels_reject_natural_operands_before_unfolding_checked_integer_aliases() {
    let ctx = context();
    for name in ["add", "mul", "sub"] {
        for (a, b) in [(nat(3), nat(8)), (int(3), nat(8)), (nat(3), int(8))] {
            let bad = apps(g(name), &[a, b]);
            assert!(
                infer_type(&ctx, &bad).is_err(),
                "{name}: main checker accepted wrong carrier"
            );
            assert!(
                recheck(&ctx, &bad).is_err(),
                "{name}: independent checker accepted wrong carrier"
            );
        }
    }
}
#[test]
fn untyped_beta_reduction_is_not_a_typing_judgment() {
    let ctx = context();
    let bad = app(lam("x", g("Int"), Term::Var("x".into())), nat(3));
    assert_eq!(normalize(&ctx, &bad), nat(3));
    assert!(infer_type(&ctx, &bad).is_err());
    assert!(recheck(&ctx, &bad).is_err());
}
#[test]
fn normalization_of_a_wrong_carrier_application_cannot_certify_an_integer_equation() {
    let ctx = context();
    let bad = apps(g("sub"), &[nat(3), nat(8)]);
    let reduced = normalize(&ctx, &bad);
    let equation = apps(g("Eq"), &[g("Int"), bad, reduced.clone()]);
    let proof = apps(g("refl"), &[g("Int"), reduced]);
    let consume = app(lam("h", equation, g("true")), proof);
    assert!(infer_type(&ctx, &consume).is_err());
    assert!(recheck(&ctx, &consume).is_err());
}
