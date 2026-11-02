//! Independent controls for the emitted recursion in corpus case T10-017.
//! The historical test stops before these computations; its assertions remain unchanged.
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies_of, defeq_for_test, infer_type, normalize_checked, recheck, BigInt, Context,
    Literal, Term,
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

fn fix(name: &str, body: Term) -> Term {
    Term::Fix {
        name: name.to_string(),
        body: Box::new(body),
    }
}

fn mat(d: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(motive),
        cases,
    }
}

fn nat() -> Term {
    g("Nat")
}

fn succ(x: Term) -> Term {
    app(g("Succ"), x)
}

fn natl(n: i64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_i64(n)))
}

fn nat_binop_ty() -> Term {
    pi("_", nat(), pi("_", nat(), nat()))
}

fn emitted_nat_sub() -> Term {
    fix(
        "rec",
        lam(
            "m",
            nat(),
            lam(
                "n",
                nat(),
                mat(
                    var("m"),
                    lam("_", nat(), nat()),
                    vec![
                        g("Zero"),
                        lam(
                            "mk",
                            nat(),
                            mat(
                                var("n"),
                                lam("_", nat(), nat()),
                                vec![
                                    succ(var("mk")),
                                    lam("nk", nat(), apps(var("rec"), &[var("mk"), var("nk")])),
                                ],
                            ),
                        ),
                    ],
                ),
            ),
        ),
    )
}

fn context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let body = emitted_nat_sub();
    assert!(defeq_for_test(
        &ctx,
        &infer_type(&ctx, &body).unwrap(),
        &nat_binop_ty()
    ));
    assert!(defeq_for_test(
        &ctx,
        &recheck(&ctx, &body).unwrap(),
        &nat_binop_ty()
    ));
    ctx.declare("emittedNatSub", nat_binop_ty(), body).unwrap();
    let usage = axiom_dependencies_of(&ctx, "emittedNatSub").unwrap();
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "{usage:?}"
    );
    ctx
}
fn computes(ctx: &Context, a: i64, b: i64, expected: i64) {
    let term = apps(g("emittedNatSub"), &[natl(a), natl(b)]);
    assert_eq!(infer_type(ctx, &term).unwrap(), nat());
    assert_eq!(recheck(ctx, &term).unwrap(), nat());
    let result = normalize_checked(ctx, &term).expect("checked emitted Nat subtraction finishes");
    assert_eq!(infer_type(ctx, &result).unwrap(), nat());
    assert_eq!(recheck(ctx, &result).unwrap(), nat());
    assert!(
        defeq_for_test(ctx, &result, &natl(expected)),
        "{a} minus {b}: {result:?}"
    );
    assert!(!defeq_for_test(ctx, &result, &natl(expected + 1)));
}
#[test]
fn original_emitted_subtraction_preserves_saturation_and_carrier() {
    let ctx = context();
    for (a, b, expected) in [(3, 8, 0), (0, 1, 0), (8, 3, 5), (5, 5, 0), (9, 0, 9)] {
        computes(&ctx, a, b, expected);
    }
}
#[test]
fn original_emitted_subtraction_5007_minus_5000_computes_seven() {
    computes(&context(), 5007, 5000, 7);
}
#[test]
fn original_lean_large_subtraction_50000_minus_one_computes_49999() {
    computes(&context(), 50000, 1, 49999);
}
