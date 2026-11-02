use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn lam(n: &str, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(body),
    }
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}

#[test]
fn closed_delta_exposes_wrappers_but_not_distinct_arguments() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c.declare(
        "wrapper",
        Term::Pi {
            param: "x".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Nat")),
        },
        lam("x", v("x")),
    )
    .unwrap();
    assert!(bounded_conversion(
        &c,
        &app(g("wrapper"), g("Zero")),
        &g("Zero"),
        &mut 256
    ));
    assert!(bounded_conversion(
        &c,
        &g("Zero"),
        &app(g("wrapper"), g("Zero")),
        &mut 256
    ));
    assert!(!bounded_conversion(
        &c,
        &app(g("wrapper"), g("Zero")),
        &app(g("Succ"), g("Zero")),
        &mut 256
    ));
    assert!(!bounded_conversion(
        &c,
        &app(g("wrapper"), g("Zero")),
        &g("Zero"),
        &mut 0
    ));
}

#[test]
fn bounded_delta_does_not_capture_open_bodies_or_expand_cycles() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    assert!(
        c.declare("open", g("Nat"), v("x")).is_err(),
        "the gateway refuses an open body"
    );
    let local = c.extend("x", g("Nat"));
    assert!(!bounded_conversion(&local, &g("open"), &v("x"), &mut 256));
    assert!(
        c.declare("loop", g("Nat"), g("loop")).is_err(),
        "the gateway refuses a δ-cycle"
    );
    assert!(!bounded_conversion(&c, &g("loop"), &g("Zero"), &mut 256));
    let length = 300;
    c.declare(&format!("hop{length}"), g("Nat"), g("Zero"))
        .unwrap();
    for i in (0..length).rev() {
        c.declare(&format!("hop{i}"), g("Nat"), g(&format!("hop{}", i + 1)))
            .unwrap();
    }
    let mut budget = 64;
    assert!(!bounded_conversion(&c, &g("hop0"), &g("Zero"), &mut budget));
    assert_eq!(
        budget, 0,
        "a δ-chain longer than the allowance exhausts it and stops"
    );
    let mut budget = 1 << 16;
    assert!(
        bounded_conversion(&c, &g("hop0"), &g("Zero"), &mut budget),
        "the chain does reach `Zero`"
    );
}

#[test]
fn polymorphic_delta_preserves_universe_instances() {
    let mut c = Context::new();
    c.declare_universe_poly(
        "alias",
        vec!["u".into()],
        Term::Sort(Universe::Succ(Box::new(Universe::Var("u".into())))),
        Term::Sort(Universe::Var("u".into())),
    )
    .unwrap();
    let instance = Term::Const {
        name: "alias".into(),
        levels: vec![Universe::Type(1)],
    };
    assert!(bounded_conversion(
        &c,
        &instance,
        &Term::Sort(Universe::Type(1)),
        &mut 256
    ));
    assert!(!bounded_conversion(
        &c,
        &instance,
        &Term::Sort(Universe::Type(0)),
        &mut 256
    ));
    let malformed = Term::Const {
        name: "alias".into(),
        levels: vec![],
    };
    assert!(!bounded_conversion(
        &c,
        &malformed,
        &Term::Sort(Universe::Type(1)),
        &mut 256
    ));
}
