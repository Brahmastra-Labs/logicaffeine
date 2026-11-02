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
fn pi(n: &str, ty: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(ty),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, ty: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(b),
    }
}
fn fixture() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    c.with_equations_suspended(|c| {
        let mut ty = g("Nat");
        let mut body = v("a5");
        for i in (0..6).rev() {
            let n = format!("a{i}");
            ty = pi(&n, g("Nat"), ty);
            body = lam(&n, g("Nat"), body);
        }
        c.declare("frame", ty, body).unwrap();
        c.declare(
            "drop",
            pi("a", g("Nat"), g("Nat")),
            lam("a", g("Nat"), g("Zero")),
        )
        .unwrap();
    });
    c
}
fn nested(depth: usize, mut tail: Term) -> Term {
    for _ in 0..depth {
        let mut head = g("frame");
        for _ in 0..5 {
            head = app(head, g("Zero"));
        }
        tail = app(head, tail);
    }
    tail
}
fn beta_zero() -> Term {
    app(lam("x", g("Nat"), v("x")), g("Zero"))
}
#[test]
fn nested_common_heads_preserve_budget_for_the_actual_difference() {
    let c = fixture();
    let a = nested(12, beta_zero());
    let b = nested(12, g("Zero"));
    assert_eq!(infer_type(&c, &a).unwrap(), g("Nat"));
    assert_eq!(infer_type(&c, &b).unwrap(), g("Nat"));
    crate::reduction::take_bounded_head_clone_nodes();
    let mut budget = 256;
    assert!(bounded_conversion(&c, &a, &b, &mut budget));
    let clones = crate::reduction::take_bounded_head_clone_nodes();
    assert!(clones <= 20, "actual head-reducer Term clones: {clones}");
    assert!(budget > 0);
}
#[test]
fn even_short_common_spines_do_not_clone_their_shared_prefixes() {
    let c = fixture();
    let a = nested(3, beta_zero());
    let b = nested(3, g("Zero"));
    crate::reduction::take_bounded_head_clone_nodes();
    let mut budget = 256;
    assert!(bounded_conversion(&c, &a, &b, &mut budget));
    let clones = crate::reduction::take_bounded_head_clone_nodes();
    assert!(clones <= 20, "actual head-reducer Term clones: {clones}");
}
#[test]
fn nested_common_heads_still_compare_the_innermost_argument() {
    let c = fixture();
    let a = nested(8, g("Zero"));
    let b = nested(8, app(g("Succ"), g("Zero")));
    let mut budget = 256;
    assert!(!bounded_conversion(&c, &a, &b, &mut budget));
}
#[test]
fn failed_argument_congruence_retains_erasing_delta_fallback() {
    let c = fixture();
    let a = app(g("drop"), g("Zero"));
    let b = app(g("drop"), app(g("Succ"), g("Zero")));
    let mut budget = 256;
    assert!(bounded_conversion(&c, &a, &b, &mut budget));
    assert!(budget > 0 && budget < 256);
}
#[test]
fn common_heads_observe_rebound_definitions_without_a_cache() {
    let original = fixture();
    let mut changed = original.clone();
    changed
        .with_equations_suspended(|c| {
            c.declare(
                "drop",
                pi("a", g("Nat"), g("Nat")),
                lam("a", g("Nat"), v("a")),
            )
        })
        .unwrap();
    let a = app(g("drop"), g("Zero"));
    let b = app(g("drop"), app(g("Succ"), g("Zero")));
    let mut budget = 256;
    assert!(!bounded_conversion(&changed, &a, &b, &mut budget));
    let mut budget = 256;
    assert!(bounded_conversion(&original, &a, &b, &mut budget));
}
