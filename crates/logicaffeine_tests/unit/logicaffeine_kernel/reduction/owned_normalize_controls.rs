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
fn redex(t: Term) -> Term {
    ap(lam("x", g("Nat"), v("x")), t)
}
fn differential(c: &Context, input: &Term, tr: Transparency<'_>) {
    let expected = reduce_step_opt(c, input, tr);
    let mut actual = input.clone();
    assert_eq!(
        reduce_step_in_place(c, &mut actual, tr),
        expected.is_some(),
        "input: {input:?}"
    );
    assert_eq!(
        actual,
        expected.unwrap_or_else(|| input.clone()),
        "input: {input:?}"
    );
}
#[test]
fn reducing_one_application_child_keeps_both_existing_boxes() {
    let c = Context::new();
    let mut t = ap(redex(g("f")), g(&"retained_argument".repeat(200)));
    let Term::App(f, a) = &t else { panic!() };
    let boxes = (f.as_ref() as *const Term, a.as_ref() as *const Term);
    let Term::Global(n) = a.as_ref() else {
        panic!()
    };
    let bytes = n.as_ptr();
    assert!(reduce_step_in_place(&c, &mut t, Transparency::All));
    assert_eq!(t, ap(g("f"), g(&"retained_argument".repeat(200))));
    let Term::App(f, a) = &t else { panic!() };
    assert_eq!(
        (f.as_ref() as *const Term, a.as_ref() as *const Term),
        boxes
    );
    let Term::Global(n) = a.as_ref() else {
        panic!()
    };
    assert_eq!(n.as_ptr(), bytes);
}
#[test]
fn reducing_inside_a_shared_let_steps_its_parts_where_they_lie() {
    let c = Context::new();
    let retained = g(&"retained_type".repeat(200));
    // `x` occurs twice, so the binding is shared and only its value steps.
    let mut t = Term::Let {
        name: "x".into(),
        ty: Box::new(retained.clone()),
        value: Box::new(redex(g("Zero"))),
        body: Box::new(ap(v("x"), v("x"))),
    };
    let Term::Let {
        ty, value, body, ..
    } = &t
    else {
        panic!()
    };
    let boxes = (
        ty.as_ref() as *const Term,
        value.as_ref() as *const Term,
        body.as_ref() as *const Term,
    );
    let Term::Global(n) = ty.as_ref() else {
        panic!()
    };
    let bytes = n.as_ptr();
    assert!(reduce_step_in_place(&c, &mut t, Transparency::All));
    assert_eq!(
        t,
        Term::Let {
            name: "x".into(),
            ty: Box::new(retained),
            value: Box::new(g("Zero")),
            body: Box::new(ap(v("x"), v("x")))
        }
    );
    let Term::Let {
        ty, value, body, ..
    } = &t
    else {
        panic!()
    };
    assert_eq!(
        (
            ty.as_ref() as *const Term,
            value.as_ref() as *const Term,
            body.as_ref() as *const Term
        ),
        boxes,
        "a shared binding keeps its boxes: nothing is rebuilt around the step"
    );
    let Term::Global(n) = ty.as_ref() else {
        panic!()
    };
    assert_eq!(n.as_ptr(), bytes, "the untouched type is never copied");
}
#[test]
fn every_let_shape_steps_exactly_as_the_reference_reducer() {
    let c = Context::new();
    let r = redex(v("y"));
    let xx = ap(v("x"), v("x"));
    let shared = |ty: Term, value: Term, body: Term| Term::Let {
        name: "x".into(),
        ty: Box::new(ty),
        value: Box::new(value),
        body: Box::new(body),
    };
    let terms = vec![
        // At most one occurrence: ζ on the unreduced value.
        shared(r.clone(), r.clone(), v("x")),
        shared(g("Nat"), r.clone(), ap(g("f"), r.clone())),
        // Shared: each part steps once, in order, independently.
        shared(r.clone(), g("Zero"), xx.clone()),
        shared(g("Nat"), g("Zero"), ap(xx.clone(), r.clone())),
        shared(r.clone(), r.clone(), ap(xx.clone(), r.clone())),
        // Shared and all parts normal: ζ only when the expansion itself steps.
        shared(g("Nat"), lam("y", g("Nat"), v("y")), xx.clone()),
        shared(g("Nat"), g("opaque"), xx.clone()),
        // A shared binding shadowed inside its own body.
        shared(
            g("Nat"),
            r.clone(),
            ap(v("x"), lam("x", g("Nat"), ap(v("x"), v("x")))),
        ),
        // Nested shared bindings.
        shared(
            g("Nat"),
            shared(g("Nat"), r.clone(), xx.clone()),
            xx.clone(),
        ),
    ];
    for t in &terms {
        differential(&c, t, Transparency::All);
    }
    for t in &terms {
        let mut old = t.clone();
        let mut new = t.clone();
        for _ in 0..12 {
            let next = reduce_step_opt(&c, &old, Transparency::All);
            let changed = reduce_step_in_place(&c, &mut new, Transparency::All);
            assert_eq!(changed, next.is_some(), "{t:?}");
            if let Some(next) = next {
                old = next;
            }
            assert_eq!(old, new, "{t:?}");
            if !changed {
                break;
            }
        }
    }
}
#[test]
fn reducing_match_discriminant_keeps_motive_and_case_vector() {
    let c = Context::new();
    let mut t = Term::Match {
        discriminant: Box::new(redex(v("unknown"))),
        motive: Box::new(g("motive")),
        cases: vec![g("first"), g("second")],
    };
    let Term::Match {
        discriminant,
        motive,
        cases,
    } = &t
    else {
        panic!()
    };
    let pointers = (
        discriminant.as_ref() as *const Term,
        motive.as_ref() as *const Term,
        cases.as_ptr(),
    );
    assert!(reduce_step_in_place(&c, &mut t, Transparency::All));
    let Term::Match {
        discriminant,
        motive,
        cases,
    } = &t
    else {
        panic!()
    };
    assert_eq!(discriminant.as_ref(), &v("unknown"));
    assert_eq!(
        (
            discriminant.as_ref() as *const Term,
            motive.as_ref() as *const Term,
            cases.as_ptr()
        ),
        pointers
    );
}
#[test]
fn all_term_forms_preserve_exact_one_step_order_and_shadowing() {
    let c = Context::new();
    let r = redex(v("y"));
    let terms = vec![
        Term::Hole,
        Term::Sort(crate::Universe::Type(0)),
        g("opaque"),
        v("x"),
        Term::Const {
            name: "missing".into(),
            levels: vec![crate::Universe::Type(0)],
        },
        lam("y", r.clone(), r.clone()),
        Term::Pi {
            param: "y".into(),
            param_type: Box::new(r.clone()),
            body_type: Box::new(r.clone()),
        },
        Term::Fix {
            name: "rec".into(),
            body: Box::new(r.clone()),
        },
        Term::MutualFix {
            defs: vec![("left".into(), r.clone()), ("right".into(), r.clone())],
            index: 1,
        },
        Term::Let {
            name: "x".into(),
            ty: Box::new(r.clone()),
            value: Box::new(r.clone()),
            body: Box::new(ap(v("x"), v("x"))),
        },
        ap(
            lam("x", g("Nat"), lam("y", v("x"), ap(v("x"), v("y")))),
            v("y"),
        ),
        ap(lam("x", g("Nat"), lam("y", g("Nat"), v("y"))), v("y")),
        ap(r.clone(), r.clone()),
        Term::Match {
            discriminant: Box::new(r.clone()),
            motive: Box::new(r.clone()),
            cases: vec![r.clone(), r],
        },
    ];
    for t in &terms {
        differential(&c, t, Transparency::All);
    }
    for a in &terms {
        for b in &terms {
            differential(&c, &ap(a.clone(), b.clone()), Transparency::All);
        }
    }
}
#[test]
fn registered_reductions_literals_and_transparency_match_reference() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c.add_definition("visible".into(), g("Nat"), g("Zero"));
    let hidden = |n: &str| n == "visible";
    let terms = vec![
        g("visible"),
        redex(g("visible")),
        ap(
            ap(g("add"), Term::Lit(Literal::Int(2))),
            Term::Lit(Literal::Int(3)),
        ),
        ap(
            ap(g("add"), redex(Term::Lit(Literal::Int(2)))),
            v("neutral"),
        ),
        ap(g("reduceBool"), g("true")),
        Term::Match {
            discriminant: Box::new(g("Zero")),
            motive: Box::new(g("Nat")),
            cases: vec![redex(g("Zero")), lam("n", g("Nat"), v("n"))],
        },
        Term::Match {
            discriminant: Box::new(ap(g("Succ"), g("Zero"))),
            motive: Box::new(g("Nat")),
            cases: vec![g("Zero"), lam("n", g("Nat"), v("n"))],
        },
        Term::Const {
            name: "Nat".into(),
            levels: vec![crate::Universe::Type(8)],
        },
    ];
    for t in &terms {
        for tr in [Transparency::All, Transparency::Excluding(&hidden)] {
            differential(&c, t, tr);
        }
    }
}
#[test]
fn repeated_transitions_preserve_results_and_step_counts() {
    let mut c = Context::new();
    c.add_definition("cycle".into(), g("Nat"), g("cycle"));
    for input in [
        g("cycle"),
        redex(redex(g("Zero"))),
        lam("z", redex(g("Nat")), redex(v("z"))),
        Term::Let {
            name: "x".into(),
            ty: Box::new(g("Nat")),
            value: Box::new(redex(g("Zero"))),
            body: Box::new(ap(v("x"), v("x"))),
        },
    ] {
        let mut old = input.clone();
        let mut new = input;
        for _ in 0..20 {
            let next = reduce_step_opt(&c, &old, Transparency::All);
            let changed = reduce_step_in_place(&c, &mut new, Transparency::All);
            assert_eq!(changed, next.is_some());
            if let Some(next) = next {
                old = next;
            }
            assert_eq!(old, new);
            if !changed {
                break;
            }
        }
    }
}
