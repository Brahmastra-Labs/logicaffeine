use logicaffeine_kernel::{defeq_for_test, infer_type, recheck, Context, Term, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn pi(n: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn ty() -> Term {
    Term::Sort(Universe::Type(0))
}
fn cn(n: &str, u: Universe) -> Term {
    Term::Const {
        name: n.into(),
        levels: vec![u],
    }
}
fn use_conversion(c: &mut Context, carrier: Term, a: Term, b: Term) -> Term {
    c.add_declaration("Family", pi("x", carrier, ty()));
    c.add_declaration("witness", app(g("Family"), a));
    Term::Let {
        name: "checked".into(),
        ty: Box::new(app(g("Family"), b)),
        value: Box::new(g("witness")),
        body: Box::new(v("checked")),
    }
}
fn simple_unit() -> Context {
    let mut c = Context::new();
    c.add_inductive("LocalUnit", ty());
    c.add_constructor_checked("local_unit", "LocalUnit", g("LocalUnit"))
        .unwrap();
    c
}
#[test]
fn two_neutral_unit_inhabitants_remain_convertible_in_both_kernels() {
    let mut c = simple_unit();
    c.add_declaration("first", g("LocalUnit"));
    c.add_declaration("second", g("LocalUnit"));
    assert!(defeq_for_test(&c, &g("first"), &g("second")));
    let t = use_conversion(&mut c, g("LocalUnit"), g("first"), g("second"));
    infer_type(&c, &t).unwrap();
    recheck(&c, &t).unwrap();
}
#[test]
fn constructor_and_neutral_unit_inhabitant_remain_convertible() {
    for reverse in [false, true] {
        let mut c = simple_unit();
        c.add_declaration("neutral", g("LocalUnit"));
        let (a, b) = if reverse {
            (g("neutral"), g("local_unit"))
        } else {
            (g("local_unit"), g("neutral"))
        };
        assert!(defeq_for_test(&c, &a, &b));
        let t = use_conversion(&mut c, g("LocalUnit"), a, b);
        infer_type(&c, &t).unwrap();
        recheck(&c, &t).unwrap();
    }
}
#[test]
fn parameterized_fieldless_singletons_retain_unit_eta() {
    let mut c = Context::new();
    c.add_inductive("Payload", ty());
    c.add_indexed_inductive("Token", pi("A", ty(), ty()), 1);
    c.add_constructor_checked("token", "Token", pi("A", ty(), app(g("Token"), v("A"))))
        .unwrap();
    let carrier = app(g("Token"), g("Payload"));
    let constructor = app(g("token"), g("Payload"));
    c.add_declaration("neutral", carrier.clone());
    assert!(defeq_for_test(&c, &constructor, &g("neutral")));
    let t = use_conversion(&mut c, carrier, constructor, g("neutral"));
    infer_type(&c, &t).unwrap();
    recheck(&c, &t).unwrap();
}
#[test]
fn explicit_universe_singletons_retain_unit_eta() {
    for level in [Universe::Prop, Universe::Type(0), Universe::Type(1)] {
        let mut c = Context::new();
        c.add_universe_poly_inductive(
            "PolyUnit",
            vec!["u".into()],
            Term::Sort(Universe::Var("u".into())),
            0,
        );
        c.add_constructor_checked(
            "poly_unit",
            "PolyUnit",
            cn("PolyUnit", Universe::Var("u".into())),
        )
        .unwrap();
        let carrier = cn("PolyUnit", level.clone());
        let constructor = cn("poly_unit", level);
        c.add_declaration("neutral", carrier.clone());
        assert!(defeq_for_test(&c, &constructor, &g("neutral")));
        let t = use_conversion(&mut c, carrier, constructor, g("neutral"));
        infer_type(&c, &t).unwrap();
        recheck(&c, &t).unwrap();
    }
}
#[test]
fn two_constructor_data_is_not_mistaken_for_unit() {
    let mut c = Context::new();
    c.add_inductive("Choice", ty());
    c.add_constructor_checked("left", "Choice", g("Choice"))
        .unwrap();
    c.add_constructor_checked("right", "Choice", g("Choice"))
        .unwrap();
    c.add_declaration("neutral", g("Choice"));
    assert!(!defeq_for_test(&c, &g("left"), &g("right")));
    assert!(!defeq_for_test(&c, &g("left"), &g("neutral")));
    let t = use_conversion(&mut c, g("Choice"), g("left"), g("right"));
    assert!(infer_type(&c, &t).is_err());
    assert!(recheck(&c, &t).is_err());
}
