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
fn pi(n: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn frame_type() -> Term {
    (0..6)
        .rev()
        .fold(g("Nat"), |body, i| pi(&format!("n{i}"), g("Nat"), body))
}
fn frame(head: Term, last: Term) -> Term {
    app((0..5).fold(head, |f, _| app(f, g("Zero"))), last)
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    c.add_declaration("frame", frame_type());
    c.add_declaration("other", frame_type());
    c.add_universe_poly_declaration("poly", vec!["u".into()], frame_type())
        .unwrap();
    c.extend("fresh", g("Nat"))
}
fn late_branch(head: Term) -> (Term, Term) {
    (
        app(lam("n", g("Nat"), frame(head.clone(), v("n"))), v("fresh")),
        frame(head, v("fresh")),
    )
}
fn probe(c: &Context, a: &Term, b: &Term) -> bool {
    bounded_conversion(c, a, b, &mut 256)
}
#[test]
fn late_branch_copies_only_the_reducible_side() {
    let c = context();
    let (a, b) = late_branch(g("frame"));
    assert_eq!(infer_type(&c, &a).unwrap(), g("Nat"));
    assert_eq!(infer_type(&c, &b).unwrap(), g("Nat"));
    crate::reduction::take_bounded_head_clone_nodes();
    assert!(probe(&c, &a, &b));
    // Seventeen syntax nodes in the applied lambda; the fixed RHS has thirteen.
    assert_eq!(crate::reduction::take_bounded_head_clone_nodes(), 17);
}
#[test]
fn late_branch_avoids_actual_fixed_spine_visits_with_the_same_budget() {
    let c = context();
    let (a, b) = late_branch(g("frame"));
    crate::reduction::take_bounded_head_visits();
    let mut budget = 12;
    assert!(bounded_conversion(&c, &a, &b, &mut budget));
    // App + Lambda + the six-argument result spine including its global root.
    assert_eq!(crate::reduction::take_bounded_head_visits(), 9);
    assert_eq!(budget, 2);
    let mut empty = 0;
    assert!(!bounded_conversion(&c, &a, &b, &mut empty));
    assert_eq!(empty, 0);
}
#[test]
fn unequal_opaque_fixed_spines_are_not_copied_or_executed() {
    let c = context();
    let a = frame(g("frame"), v("fresh"));
    let b = frame(g("other"), v("fresh"));
    assert_eq!(infer_type(&c, &a).unwrap(), infer_type(&c, &b).unwrap());
    crate::reduction::take_bounded_head_clone_nodes();
    crate::reduction::take_bounded_head_visits();
    assert!(!probe(&c, &a, &b));
    assert_eq!(crate::reduction::take_bounded_head_clone_nodes(), 0);
    assert_eq!(crate::reduction::take_bounded_head_visits(), 0);
}
#[test]
fn fixed_head_observation_does_not_freeze_delta_or_argument_obligations() {
    let mut c = context();
    let ty = pi("x", g("Nat"), g("Nat"));
    c.with_equations_suspended(|c| c.declare("drop", ty.clone(), lam("x", g("Nat"), g("Zero"))))
        .unwrap();
    let a = app(g("drop"), g("Zero"));
    let b = app(g("drop"), app(g("Succ"), g("Zero")));
    assert!(probe(&c, &a, &b)); // failed argument comparison must retain erasing delta
    c.set_reducibility("drop", crate::context::Reducibility::Irreducible);
    assert!(probe(&c, &a, &b)); // existing full-kernel transparency policy
    let mut changed = c.clone();
    changed
        .with_equations_suspended(|c| c.declare("drop", ty, lam("x", g("Nat"), v("x"))))
        .unwrap();
    assert!(!probe(&changed, &a, &b));
    assert!(probe(&c, &a, &b));
    assert!(!probe(
        &c,
        &frame(g("frame"), g("Zero")),
        &frame(g("frame"), app(g("Succ"), g("Zero")))
    ));
}
#[test]
fn local_heads_and_capture_are_not_assumed_global() {
    let c = context().extend("local", frame_type());
    let (a, b) = late_branch(v("local"));
    assert_eq!(infer_type(&c, &a).unwrap(), g("Nat"));
    assert!(probe(&c, &a, &b));
    let captured = app(
        lam("fresh", g("Nat"), lam("n", g("Nat"), v("fresh"))),
        v("n"),
    );
    let c = c.extend("n", g("Nat"));
    assert!(probe(&c, &captured, &lam("renamed", g("Nat"), v("n"))));
    assert!(!probe(&c, &captured, &lam("n", g("Nat"), v("n"))));
}
#[test]
fn explicit_const_head_keeps_universe_identity() {
    let c = context();
    let at = |u| Term::Const {
        name: "poly".into(),
        levels: vec![u],
    };
    let (a, b) = late_branch(at(Universe::Type(0)));
    assert_eq!(infer_type(&c, &a).unwrap(), g("Nat"));
    assert!(probe(&c, &a, &b));
    assert!(!probe(&c, &b, &frame(at(Universe::Type(1)), v("fresh"))));
    assert!(!probe(
        &c,
        &b,
        &frame(
            Term::Const {
                name: "poly".into(),
                levels: vec![]
            },
            v("fresh")
        )
    ));
}
#[test]
fn let_and_match_function_heads_still_execute() {
    let c = context();
    let let_head = Term::Let {
        name: "f".into(),
        ty: Box::new(frame_type()),
        value: Box::new(g("frame")),
        body: Box::new(v("f")),
    };
    let match_head = Term::Match {
        discriminant: Box::new(g("Zero")),
        motive: Box::new(lam("n", g("Nat"), frame_type())),
        cases: vec![g("frame"), lam("n", g("Nat"), g("frame"))],
    };
    let expected = frame(g("frame"), v("fresh"));
    for head in [let_head, match_head] {
        let a = frame(head, v("fresh"));
        assert_eq!(infer_type(&c, &a).unwrap(), g("Nat"));
        assert!(probe(&c, &a, &expected));
    }
}
#[test]
fn fixed_head_does_not_make_holes_or_oversized_erasing_inputs_admissible() {
    let mut c = context();
    let ty = pi("x", g("Nat"), g("Nat"));
    c.with_equations_suspended(|c| c.declare("erase", ty, lam("x", g("Nat"), g("Zero"))))
        .unwrap();
    assert!(!conversion_by_unfolded_head(
        &c,
        &app(g("erase"), Term::Hole),
        &g("Zero")
    ));
    fn tree(d: usize) -> Term {
        if d == 0 {
            g("Zero")
        } else {
            app(app(g("opaque.tree"), tree(d - 1)), tree(d - 1))
        }
    }
    // Raw oversized syntax is an admission boundary, not a typing fixture.
    let oversized = app(g("erase"), tree(11));
    assert!(!probe(&c, &oversized, &g("Zero")));
}
