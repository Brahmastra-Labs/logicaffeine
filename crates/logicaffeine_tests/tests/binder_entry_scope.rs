//! Closed dependent-binder capture regressions. No free-context bridge is required.
use logicaffeine_kernel::{
    defeq_for_test, double_check, infer_type, recheck, Context, DoubleCheck, KernelError,
    ReCheckError, Term, Universe,
};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn sort() -> Term {
    Term::Sort(Universe::Type(0))
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn context() -> Context {
    let mut c = Context::new();
    logicaffeine_kernel::prelude::StandardLibrary::register(&mut c);
    c
}
fn repeated() -> Term {
    lam("A", sort(), lam("x", v("A"), lam("A", sort(), v("x"))))
}
fn correct() -> Term {
    pi(
        "Outer",
        sort(),
        pi("x", v("Outer"), pi("Inner", sort(), v("Outer"))),
    )
}
fn incorrect() -> Term {
    pi(
        "Outer",
        sort(),
        pi("x", v("Outer"), pi("Inner", sort(), v("Inner"))),
    )
}
fn goal(body: Term, ty: Term) -> Term {
    Term::Let {
        name: "checked".into(),
        ty: Box::new(ty),
        value: Box::new(body),
        body: Box::new(v("checked")),
    }
}
fn instantiated() -> Term {
    let body = lam(
        "A",
        sort(),
        lam("x", v("A"), app(lam("A", sort(), v("x")), g("Nat"))),
    );
    app(app(body, g("Bool")), g("true"))
}
fn let_capture() -> Term {
    lam(
        "A",
        sort(),
        lam(
            "x",
            v("A"),
            Term::Let {
                name: "A".into(),
                ty: Box::new(sort()),
                value: Box::new(g("Nat")),
                body: Box::new(v("x")),
            },
        ),
    )
}

fn checked(c: &Context, t: &Term, ty: &Term) {
    assert!(defeq_for_test(c, &infer_type(c, t).unwrap(), ty));
    assert!(defeq_for_test(c, &recheck(c, t).unwrap(), ty));
    assert_eq!(double_check(c, t), DoubleCheck::Agreed);
}
#[test]
fn pi_shadowing_cannot_apply_an_outer_family_to_an_inner_element() {
    let c = context();
    let t = pi(
        "A",
        sort(),
        pi(
            "F",
            pi("x", v("A"), sort()),
            pi("A", sort(), pi("y", v("A"), app(v("F"), v("y")))),
        ),
    );
    assert!(matches!(
        infer_type(&c, &t),
        Err(KernelError::TypeMismatch { .. })
    ));
    assert!(matches!(recheck(&c, &t), Err(ReCheckError::Ill(_))));
}
#[test]
fn pi_shadowing_keeps_the_legal_outer_family_argument() {
    let c = context();
    let t = pi(
        "A",
        sort(),
        pi(
            "F",
            pi("x", v("A"), sort()),
            pi("x", v("A"), pi("A", sort(), app(v("F"), v("x")))),
        ),
    );
    checked(&c, &t, &Term::Sort(Universe::Type(1)));
}
#[test]
fn structural_fix_lambda_preserves_its_outer_result_type() {
    let c = context();
    let t = lam(
        "A",
        sort(),
        lam(
            "x",
            v("A"),
            Term::Fix {
                name: "rec".into(),
                body: Box::new(lam("n", g("Nat"), lam("A", sort(), v("x")))),
            },
        ),
    );
    let ty = pi(
        "Outer",
        sort(),
        pi(
            "x",
            v("Outer"),
            pi("n", g("Nat"), pi("Inner", sort(), v("Outer"))),
        ),
    );
    checked(&c, &t, &ty);
}
#[test]
fn fix_self_name_does_not_replace_an_external_type_dependency() {
    let c = context();
    let t = lam(
        "A",
        sort(),
        lam(
            "x",
            v("A"),
            Term::Fix {
                name: "A".into(),
                body: Box::new(lam("n", g("Nat"), v("x"))),
            },
        ),
    );
    let ty = pi(
        "Outer",
        sort(),
        pi("x", v("Outer"), pi("n", g("Nat"), v("Outer"))),
    );
    checked(&c, &t, &ty);
}
#[test]
fn mutual_self_names_preserve_external_types_and_uniform_sibling_scope() {
    let c = context();
    let first = lam("n", g("Nat"), v("x"));
    let second = lam(
        "n",
        g("Nat"),
        Term::Match {
            discriminant: Box::new(v("n")),
            motive: Box::new(lam("k", g("Nat"), g("Nat"))),
            cases: vec![g("Zero"), lam("k", g("Nat"), app(v("A'"), v("k")))],
        },
    );
    // A collides with an outer type, and A' is an existing sibling name. Fresh
    // selection must avoid both before any simultaneous substitution begins.
    let t = lam(
        "A",
        sort(),
        lam(
            "x",
            v("A"),
            Term::MutualFix {
                defs: vec![("A".into(), first), ("A'".into(), second)],
                index: 0,
            },
        ),
    );
    let ty = pi(
        "Outer",
        sort(),
        pi("x", v("Outer"), pi("n", g("Nat"), v("Outer"))),
    );
    checked(&c, &t, &ty);
}
#[test]
fn match_motive_shadowing_retains_a_let_named_outer_type() {
    let c = context();
    let matched = Term::Match {
        discriminant: Box::new(g("Zero")),
        motive: Box::new(lam("A", g("Nat"), v("B"))),
        cases: vec![v("x"), lam("k", g("Nat"), v("x"))],
    };
    let t = lam(
        "A",
        sort(),
        lam(
            "x",
            v("A"),
            Term::Let {
                name: "B".into(),
                ty: Box::new(sort()),
                value: Box::new(v("A")),
                body: Box::new(matched),
            },
        ),
    );
    checked(
        &c,
        &t,
        &pi("Outer", sort(), pi("x", v("Outer"), v("Outer"))),
    );
}
#[test]
fn recursive_nondecrease_is_still_rejected_after_self_freshening() {
    let c = context();
    let t = lam(
        "A",
        sort(),
        Term::Fix {
            name: "A".into(),
            body: Box::new(lam(
                "n",
                g("Nat"),
                Term::Match {
                    discriminant: Box::new(v("n")),
                    motive: Box::new(lam("k", g("Nat"), g("Nat"))),
                    cases: vec![g("Zero"), lam("k", g("Nat"), app(v("A"), v("n")))],
                },
            )),
        },
    );
    assert!(matches!(
        infer_type(&c, &t),
        Err(KernelError::TerminationViolation { .. })
    ));
    assert!(matches!(recheck(&c, &t), Err(ReCheckError::Ill(_))));
}
