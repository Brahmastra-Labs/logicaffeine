use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{defeq_for_test, infer_type, recheck, Context, Term, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn alias(t: Term) -> Term {
    app(lam("T", Term::Sort(Universe::Type(0)), v("T")), t)
}
fn check(domain: Term) -> Term {
    Term::Match {
        discriminant: Box::new(g("Zero")),
        motive: Box::new(lam("n", domain, g("Nat"))),
        cases: vec![g("Zero"), lam("pred", g("Nat"), g("Zero"))],
    }
}
#[test]
fn motive_domain_uses_conversion_in_both_kernels() {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    let term = check(alias(g("Nat")));
    let expected = g("Nat");
    assert!(defeq_for_test(
        &c,
        &infer_type(&c, &term).expect("convertible motive domain"),
        &expected
    ));
    assert!(recheck(&c, &term).is_ok());
}
#[test]
fn motive_domain_does_not_accept_an_unrelated_type() {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    let term = check(alias(g("Bool")));
    assert!(infer_type(&c, &term).is_err());
    assert!(recheck(&c, &term).is_err());
}

fn nat_recursion(with_beta: bool, successor: bool) -> Term {
    let recursive = app(v("rec"), v("pred"));
    let recursive = if with_beta {
        app(lam("z", g("Nat"), v("z")), recursive)
    } else {
        recursive
    };
    let step = if successor {
        app(g("Succ"), recursive)
    } else {
        recursive
    };
    Term::Fix {
        name: "rec".into(),
        body: Box::new(lam(
            "n",
            g("Nat"),
            Term::Match {
                discriminant: Box::new(v("n")),
                motive: Box::new(lam("n", g("Nat"), g("Nat"))),
                cases: vec![g("Zero"), lam("pred", g("Nat"), step)],
            },
        )),
    }
}
#[test]
fn stuck_recursive_applications_compare_convertible_bodies() {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c.add("n", g("Nat"));
    let a = app(nat_recursion(false, true), v("n"));
    let b = app(nat_recursion(true, true), v("n"));
    assert!(infer_type(&c, &a).is_ok());
    assert!(infer_type(&c, &b).is_ok());
    assert!(defeq_for_test(&c, &a, &b));
    assert!(defeq_for_test(&c, &b, &a));
}
#[test]
fn stuck_recursive_applications_keep_different_computation_rules_distinct() {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c.add("n", g("Nat"));
    let a = app(nat_recursion(false, true), v("n"));
    let b = app(nat_recursion(true, false), v("n"));
    assert!(infer_type(&c, &a).is_ok());
    assert!(infer_type(&c, &b).is_ok());
    assert!(!defeq_for_test(&c, &a, &b));
    assert!(!defeq_for_test(&c, &b, &a));
}

#[test]
fn stuck_match_compares_unreduced_branch_bodies() {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c.add("n", g("Nat"));
    let make = |beta| Term::Match {
        discriminant: Box::new(v("n")),
        motive: Box::new(lam("n", g("Nat"), g("Nat"))),
        cases: vec![
            g("Zero"),
            lam(
                "pred",
                g("Nat"),
                if beta {
                    app(lam("z", g("Nat"), v("z")), v("pred"))
                } else {
                    v("pred")
                },
            ),
        ],
    };
    let a = make(false);
    let b = make(true);
    assert!(infer_type(&c, &a).is_ok());
    assert!(infer_type(&c, &b).is_ok());
    assert!(defeq_for_test(&c, &a, &b));
    assert!(defeq_for_test(&c, &b, &a));
}
