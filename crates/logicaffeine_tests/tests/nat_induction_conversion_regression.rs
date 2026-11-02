use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{defeq_for_test, infer_type, recheck, Context, Term};
fn g(s: &str) -> Term {
    Term::Global(s.into())
}
fn v(s: &str) -> Term {
    Term::Var(s.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
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
fn proposition(x: Term) -> Term {
    apps(
        g("Eq"),
        &[
            g("Nat"),
            apps(g("Nat_mul"), &[x.clone(), app(g("Succ"), g("Zero"))]),
            x,
        ],
    )
}
#[test]
fn independent_checker_accepts_valid_multiplication_one_induction() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let zero_add_k = apps(g("Nat_add"), &[g("Zero"), v("k")]);
    let step = lam(
        "k",
        g("Nat"),
        lam(
            "ih",
            proposition(v("k")),
            apps(
                g("IntLaw_congrArg"),
                &[
                    g("Nat"),
                    g("Nat"),
                    g("Succ"),
                    zero_add_k,
                    v("k"),
                    app(g("Nat_zero_add"), v("k")),
                ],
            ),
        ),
    );
    let body = lam(
        "m",
        g("Nat"),
        apps(
            g("Nat_rec"),
            &[
                lam("mm", g("Nat"), proposition(v("mm"))),
                apps(g("refl"), &[g("Nat"), g("Zero")]),
                step,
                v("m"),
            ],
        ),
    );
    let expected = Term::Pi {
        param: "m".into(),
        param_type: Box::new(g("Nat")),
        body_type: Box::new(proposition(v("m"))),
    };
    assert!(defeq_for_test(
        &ctx,
        &infer_type(&ctx, &body).unwrap(),
        &expected
    ));
    let independent = recheck(&ctx, &body)
        .expect("valid constructor induction must receive an independent verdict");
    assert!(defeq_for_test(&ctx, &independent, &expected));
}

#[test]
fn first_structural_column_and_false_equations_keep_both_kernel_verdicts() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let ty = Term::Pi {
        param: "n".into(),
        param_type: Box::new(g("Nat")),
        body_type: Box::new(g("Nat")),
    };
    let body = Term::Fix {
        name: "f".into(),
        body: Box::new(lam(
            "n",
            g("Nat"),
            Term::Match {
                discriminant: Box::new(v("n")),
                motive: Box::new(lam("_", g("Nat"), g("Nat"))),
                cases: vec![g("Zero"), lam("k", g("Nat"), app(v("f"), v("k")))],
            },
        )),
    };
    ctx.declare("first_column_zero", ty, body).unwrap();
    let three = app(g("Succ"), app(g("Succ"), app(g("Succ"), g("Zero"))));
    let expression = app(g("first_column_zero"), three);
    let proof = apps(g("refl"), &[g("Nat"), g("Zero")]);
    let goal = apps(g("Eq"), &[g("Nat"), expression.clone(), g("Zero")]);
    let consume = app(lam("h", goal, g("true")), proof.clone());
    assert_eq!(infer_type(&ctx, &consume).unwrap(), g("Bool"));
    assert_eq!(recheck(&ctx, &consume).unwrap(), g("Bool"));
    let false_goal = apps(g("Eq"), &[g("Nat"), expression, app(g("Succ"), g("Zero"))]);
    let false_consume = app(lam("h", false_goal, g("true")), proof);
    assert!(infer_type(&ctx, &false_consume).is_err());
    assert!(recheck(&ctx, &false_consume).is_err());
}
