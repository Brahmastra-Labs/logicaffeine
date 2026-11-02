use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn tg(n: &str) -> Term {
    Term::Global(n.into())
}
fn ty() -> Term {
    Term::Pi {
        param: "x".into(),
        param_type: Box::new(tg("N")),
        body_type: Box::new(tg("N")),
    }
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_declaration("N", Term::Sort(Universe::Type(0)));
    for n in ["zero", "one"] {
        c.add_declaration(n, tg("N"));
    }
    for n in ["opaque", "different"] {
        c.add_declaration(n, ty());
    }
    c.declare("alias", ty(), tg("opaque")).unwrap();
    c
}
fn equal(c: &Context, a: &Db, b: &Db) -> bool {
    borrowed_congruence_in(Some(c), a, b, &mut 256)
}
#[test]
fn delta_alignment_preserves_arguments_and_rebinding() {
    let c = context();
    assert!(equal(&c, &g("alias"), &g("opaque")));
    assert!(!equal(
        &c,
        &Db::App(DbBox::new(g("alias")), DbBox::new(g("zero"))),
        &Db::App(DbBox::new(g("opaque")), DbBox::new(g("one")))
    ));
    let mut changed = c.clone();
    changed.declare("alias", ty(), tg("different")).unwrap();
    assert!(!equal(&changed, &g("alias"), &g("opaque")));
    assert!(equal(&changed, &g("alias"), &g("different")));
    assert!(equal(&c, &g("alias"), &g("opaque")));
}
#[test]
fn closed_delta_under_binders_preserves_external_indices() {
    let c = context();
    for index in [0, 1, 7] {
        let call = |name: &str| Db::App(DbBox::new(g(name)), DbBox::new(Db::Var(index)));
        let a = Db::Lam(DbBox::new(g("N")), DbBox::new(call("alias")));
        let b = Db::Lam(DbBox::new(g("N")), DbBox::new(call("opaque")));
        assert!(equal(&c, &a, &b));
        assert!(equal(&c, &b, &a));
    }
}
#[test]
fn pending_open_definitions_do_not_capture_caller_locals() {
    let mut c = context();
    c.add_definition("open".into(), tg("N"), Term::Var("caller".into()));
    assert!(reduce(&c, &g("open"), &mut 256).is_none());
    assert!(!equal(&c, &g("open"), &Db::Var(0)));
}
#[test]
fn exhaustion_and_large_bodies_are_inconclusive_without_resetting_work() {
    let mut c = context();
    let mut body = tg("zero");
    let mut annotation = tg("N");
    for _ in 0..80 {
        body = Term::Lambda {
            param: "x".into(),
            param_type: Box::new(tg("N")),
            body: Box::new(body),
        };
        annotation = Term::Pi {
            param: "x".into(),
            param_type: Box::new(tg("N")),
            body_type: Box::new(annotation),
        };
    }
    c.declare("large", annotation, body).unwrap();
    let mut none = 0;
    assert!(reduce(&c, &g("alias"), &mut none).is_none());
    assert_eq!(none, 0);
    let previous = WHNF_EXHAUSTED.with(|f| f.replace(false));
    let mut fuel = 256;
    assert!(reduce(&c, &g("large"), &mut fuel).is_none());
    assert!(fuel > 0 && fuel < 256);
    assert!(!WHNF_EXHAUSTED.with(|f| f.get()));
    WHNF_EXHAUSTED.with(|f| f.set(previous));
}
#[test]
fn long_alias_chains_do_not_replenish_the_shared_allowance() {
    let mut c = context();
    let mut previous = "opaque".to_string();
    for i in 0..100 {
        let name = format!("chain.{i}");
        c.declare(&name, ty(), tg(&previous)).unwrap();
        previous = name;
    }
    let mut fuel = 256;
    assert!(!borrowed_congruence_in(
        Some(&c),
        &g(&previous),
        &g("opaque"),
        &mut fuel
    ));
    assert!(
        fuel < 8,
        "one shared allowance must bound the complete chain"
    );
}
#[test]
fn explicit_universes_follow_declared_order_and_arity() {
    let mut c = Context::new();
    let u = Universe::Var("u".into());
    c.declare_universe_poly(
        "poly",
        vec!["u".into(), "v".into()],
        Term::Sort(Universe::Succ(Box::new(u.clone()))),
        Term::Sort(u),
    )
    .unwrap();
    let term = |levels| Db::Const {
        name: "poly".into(),
        levels,
    };
    assert_eq!(
        reduce(
            &c,
            &term(vec![Universe::Type(0), Universe::Type(1)]),
            &mut 256
        ),
        Some(Db::Sort(Universe::Type(0)))
    );
    assert_eq!(
        reduce(
            &c,
            &term(vec![Universe::Type(1), Universe::Type(0)]),
            &mut 256
        ),
        Some(Db::Sort(Universe::Type(1)))
    );
    assert!(reduce(&c, &term(vec![Universe::Type(0)]), &mut 256).is_none());
    assert!(reduce(&c, &g("poly"), &mut 256).is_none());
    let mut huge = Universe::Type(0);
    for _ in 0..80 {
        huge = Universe::Succ(Box::new(huge));
    }
    let mut fuel = 256;
    assert!(reduce(&c, &term(vec![huge, Universe::Type(0)]), &mut fuel).is_none());
    assert!(fuel < 256);
}
#[test]
fn native_and_quotient_heads_remain_with_their_ordinary_dispatcher() {
    let c = context();
    for name in [
        "reduceBool",
        "try_decide",
        "Quot_lift",
        "Quot_rec",
        "kernel.quot.type",
        "kernel.quot.mk",
        "kernel.quot.lift",
        "kernel.quot.ind",
    ] {
        assert!(reduce(&c, &g(name), &mut 256).is_none());
    }
}
