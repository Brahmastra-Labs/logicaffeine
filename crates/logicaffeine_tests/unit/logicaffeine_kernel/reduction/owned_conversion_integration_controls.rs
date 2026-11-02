use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn ap(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
#[test]
fn closed_substitution_retains_body_boxes_and_last_replacement_name() {
    let body = ap(g("stable_operator"), v("x"));
    let replacement = g(&"payload".repeat(50));
    let expected = substitute(&body, "x", &replacement);
    let Term::App(f, a) = &body else { panic!() };
    let pointers = (f.as_ref() as *const Term, a.as_ref() as *const Term);
    let Term::Global(n) = &replacement else {
        panic!()
    };
    let payload = n.as_ptr();
    let result = bounded_conversion_substitute_owned(body, "x", replacement).unwrap();
    assert_eq!(result, expected);
    let Term::App(f, a) = &result else { panic!() };
    assert_eq!(
        (f.as_ref() as *const Term, a.as_ref() as *const Term),
        pointers
    );
    let Term::Global(n) = a.as_ref() else {
        panic!()
    };
    assert_eq!(n.as_ptr(), payload);
}
#[test]
fn open_replacement_preserves_capture_avoidance_and_mutual_scopes() {
    let terms = [
        lam("y", g("Nat"), ap(v("x"), v("y"))),
        Term::MutualFix {
            defs: vec![
                ("y".into(), ap(v("x"), v("y"))),
                ("z".into(), ap(v("x"), v("y"))),
            ],
            index: 1,
        },
        Term::Let {
            name: "y".into(),
            ty: Box::new(v("x")),
            value: Box::new(v("x")),
            body: Box::new(ap(v("x"), v("y"))),
        },
    ];
    for body in terms {
        let expected = substitute(&body, "x", &v("y"));
        assert_eq!(
            bounded_conversion_substitute_owned(body, "x", v("y")),
            Some(expected)
        );
    }
}
#[test]
fn beta_and_let_head_reduction_do_not_recopy_owned_terms() {
    let input = Term::Let {
        name: "x".into(),
        ty: Box::new(g("Nat")),
        value: Box::new(ap(g("Succ"), g("Zero"))),
        body: Box::new(ap(lam("y", g("Nat"), v("y")), v("x"))),
    };
    crate::type_checker::take_substitution_copy_work();
    let mut fuel = 100;
    let result = beta_iota_zeta_head(&Context::new(), &input, &mut fuel);
    assert_eq!(result, Some(ap(g("Succ"), g("Zero"))));
    assert_eq!(crate::type_checker::take_substitution_copy_work(), (0, 0));
    assert!(fuel > 0 && fuel < 100);
}
#[test]
fn output_growth_boundary_and_deep_fallback_match_existing_substitution() {
    let mut wide = v("x");
    let mut value = g("leaf");
    for _ in 0..100 {
        wide = ap(wide, v("x"));
        value = ap(value, g("leaf"));
    }
    assert!(bounded_conversion_substitute(&wide, "x", &value).is_none());
    assert!(bounded_conversion_substitute_owned(wide, "x", value).is_none());
    let mut deep = v("x");
    for _ in 0..80 {
        deep = lam("other", g("Nat"), deep);
    }
    let expected = bounded_conversion_substitute(&deep, "x", &g("value"));
    assert_eq!(
        bounded_conversion_substitute_owned(deep, "x", g("value")),
        expected
    );
}
