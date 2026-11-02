use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(n: &str, ty: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(b),
    }
}
fn function() -> Term {
    Term::Fix {
        name: "f".into(),
        body: Box::new(lam(
            "unused",
            g("Nat"),
            lam(
                "n",
                g("Nat"),
                Term::Match {
                    discriminant: Box::new(v("n")),
                    motive: Box::new(lam("n", g("Nat"), g("Nat"))),
                    cases: vec![
                        g("Zero"),
                        lam("p", g("Nat"), app(app(v("f"), v("unused")), v("p"))),
                    ],
                },
            ),
        )),
    }
}
#[test]
fn bounded_head_unrolls_the_guarded_column_without_forcing_open_arguments() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    let c = c.extend("opaque", g("Nat"));
    let term = app(app(function(), v("opaque")), g("Zero"));
    crate::infer_type(&c, &term).unwrap();
    assert_eq!(crate::normalize_checked(&c, &term).unwrap(), g("Zero"));
    assert_eq!(beta_iota_zeta_head(&c, &term, &mut 256), Some(g("Zero")));
    let recursive = app(app(function(), v("opaque")), app(g("Succ"), g("Zero")));
    assert_eq!(
        beta_iota_zeta_head(&c, &recursive, &mut 256),
        Some(g("Zero"))
    );
}
#[test]
fn bounded_head_preserves_stuck_and_unguarded_fixpoints_and_budget() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    let stuck = app(app(function(), g("Zero")), v("n"));
    assert_eq!(
        beta_iota_zeta_head(&c, &stuck, &mut 256),
        Some(stuck.clone())
    );
    let bad = Term::Fix {
        name: "f".into(),
        body: Box::new(lam("n", g("Nat"), app(v("f"), v("n")))),
    };
    let bad = app(bad, g("Zero"));
    assert_eq!(beta_iota_zeta_head(&c, &bad, &mut 256), Some(bad.clone()));
    assert_eq!(beta_iota_zeta_head(&c, &bad, &mut 0), None);
}
