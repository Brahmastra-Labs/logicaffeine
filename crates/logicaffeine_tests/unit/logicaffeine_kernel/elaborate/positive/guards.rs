use super::*;
use crate::prelude::StandardLibrary;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
#[test]
fn zero_work_and_depth_boundary_are_inconclusive() {
    let c = context();
    let t = g("Nat");
    let mut fuel = 0;
    assert!(!same(&c, &t, &t, Transparency::All, &mut fuel, 0));
    let mut fuel = 32;
    assert!(!same(&c, &t, &t, Transparency::All, &mut fuel, 64));
    assert_eq!(fuel, 32);
    assert!(same(&c, &t, &t, Transparency::All, &mut fuel, 0));
}
#[test]
fn hidden_holes_open_names_and_oversized_inputs_are_inconclusive() {
    let c = context();
    for t in [
        Term::Hole,
        Term::Var("?0".into()),
        Term::Var("free".into()),
        app(g("Succ"), Term::Hole),
        g(&"n".repeat(257)),
    ] {
        assert!(!equivalent(&c, &t, &t));
    }
    let wide = Term::Match {
        discriminant: Box::new(g("Zero")),
        motive: Box::new(g("Nat")),
        cases: vec![g("Zero"); 4096],
    };
    assert!(!equivalent(&c, &wide, &wide));
    let small = Term::Sort(Universe::Succ(Box::new(Universe::Var("u".into()))));
    assert!(admitted(&small).is_some());
    let mut too_deep = Universe::Var("u".into());
    for _ in 0..18 {
        too_deep = Universe::Succ(Box::new(too_deep));
    }
    let complex = Term::Sort(too_deep);
    assert!(!equivalent(&c, &complex, &complex));
}
#[test]
fn const_delta_respects_irreducibility_and_retains_all_wrapper_behavior() {
    let mut c = context();
    c.add_universe_poly("Guard.poly", vec!["u".into()], g("Nat"), g("Zero"));
    assert!(c.get_universe_poly("Guard.poly").is_some());
    let t = Term::Const {
        name: "Guard.poly".into(),
        levels: vec![Universe::Type(0)],
    };
    assert!(equivalent(&c, &t, &g("Zero")));
    c.set_reducibility("Guard.poly", crate::Reducibility::Irreducible);
    assert!(!equivalent(&c, &t, &g("Zero")));
    // Kernel All wrapper is unchanged, even when elaborator exposure refuses.
    assert!(reduction::bounded_delta_head(&c, &t).is_some());
}
#[test]
fn pre_copy_admission_rejects_oversized_delta_payloads() {
    let mut c = context();
    let value = Term::Lit(Literal::Text("x".repeat(257)));
    c.declare("Guard.text", g("Text"), value).unwrap();
    // Small source input, inadmissible stored body: no expansion is returned.
    assert!(admitted(&g("Guard.text")).is_some());
    assert!(
        reduction::bounded_delta_head_with(&c, &g("Guard.text"), Transparency::All, admitted)
            .is_none()
    );
    assert!(reduction::bounded_delta_head(&c, &g("Guard.text")).is_some());
}
