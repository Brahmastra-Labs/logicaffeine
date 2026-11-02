use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn app(f: Db, a: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(a))
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_declaration("N", Term::Sort(Universe::Type(0)));
    c.add_declaration("zero", Term::Global("N".into()));
    let ty = Term::Pi {
        param: "x".into(),
        param_type: Box::new(Term::Global("N".into())),
        body_type: Box::new(Term::Global("N".into())),
    };
    c.add_declaration("opaque", ty.clone());
    c.declare("alias", ty, Term::Global("opaque".into()))
        .unwrap();
    c
}

#[test]
fn folded_common_head_only_copies_the_differing_arguments() {
    let mut c = context();
    let mut body = Term::Global("zero".into());
    for i in 0..80 {
        body = Term::Let {
            name: format!("unused{i}"),
            ty: Box::new(Term::Global("N".into())),
            value: Box::new(Term::Global("zero".into())),
            body: Box::new(body),
        };
    }
    c.declare("large_alias", Term::Global("N".into()), body)
        .unwrap();
    let ty = Term::Pi {
        param: "x".into(),
        param_type: Box::new(Term::Global("N".into())),
        body_type: Box::new(Term::Global("N".into())),
    };
    c.declare(
        "identity",
        ty,
        Term::Lambda {
            param: "x".into(),
            param_type: Box::new(Term::Global("N".into())),
            body: Box::new(Term::Var("x".into())),
        },
    )
    .unwrap();
    let a = app(g("identity"), g("large_alias"));
    let b = app(g("identity"), g("zero"));
    WHNF_INPUT_CLONES.with(|v| v.set(Some(0)));
    assert!(def_eq(&c, &[], &a, &b));
    let copies = WHNF_INPUT_CLONES.with(|v| v.replace(None).unwrap());
    assert_eq!(
        copies, 2,
        "only the two differing arguments need ordinary WHNF input copies"
    );
}

#[test]
fn folded_subtyping_only_copies_the_differing_arguments() {
    let mut c = context();
    let mut body = Term::Global("zero".into());
    for i in 0..80 {
        body = Term::Let {
            name: format!("unused{i}"),
            ty: Box::new(Term::Global("N".into())),
            value: Box::new(Term::Global("zero".into())),
            body: Box::new(body),
        };
    }
    c.declare("large_alias", Term::Global("N".into()), body)
        .unwrap();
    let ty = Term::Pi {
        param: "x".into(),
        param_type: Box::new(Term::Global("N".into())),
        body_type: Box::new(Term::Global("N".into())),
    };
    c.declare(
        "identity",
        ty,
        Term::Lambda {
            param: "x".into(),
            param_type: Box::new(Term::Global("N".into())),
            body: Box::new(Term::Var("x".into())),
        },
    )
    .unwrap();
    let a = app(g("identity"), g("large_alias"));
    let b = app(g("identity"), g("zero"));
    WHNF_INPUT_CLONES.with(|v| v.set(Some(0)));
    assert!(is_sub(&c, &[], &a, &b));
    let copies = WHNF_INPUT_CLONES.with(|v| v.replace(None).unwrap());
    assert_eq!(
        copies, 2,
        "only the two differing arguments need ordinary WHNF input copies"
    );
}

#[test]
fn nested_folded_heads_share_argument_only_conversion() {
    let mut c = context();
    let mut body = Term::Global("zero".into());
    for i in 0..80 {
        body = Term::Let {
            name: format!("unused{i}"),
            ty: Box::new(Term::Global("N".into())),
            value: Box::new(Term::Global("zero".into())),
            body: Box::new(body),
        };
    }
    c.declare("large_alias", Term::Global("N".into()), body)
        .unwrap();
    let ty = Term::Pi {
        param: "x".into(),
        param_type: Box::new(Term::Global("N".into())),
        body_type: Box::new(Term::Global("N".into())),
    };
    c.declare(
        "identity",
        ty,
        Term::Lambda {
            param: "x".into(),
            param_type: Box::new(Term::Global("N".into())),
            body: Box::new(Term::Var("x".into())),
        },
    )
    .unwrap();
    let a = app(g("identity"), app(g("identity"), g("large_alias")));
    let b = app(g("identity"), app(g("identity"), g("zero")));
    WHNF_INPUT_CLONES.with(|v| v.set(Some(0)));
    assert!(def_eq(&c, &[], &a, &b));
    let copies = WHNF_INPUT_CLONES.with(|v| v.replace(None).unwrap());
    assert_eq!(
        copies, 2,
        "nested shared heads must reuse the same argument conversion allowance"
    );
}
#[test]
fn small_delta_aliases_align_without_cloning_common_applications() {
    let c = context();
    let a = app(g("opaque"), g("zero"));
    let b = app(g("alias"), g("zero"));
    WHNF_INPUT_CLONES.with(|v| v.set(Some(0)));
    assert!(def_eq(&c, &[], &a, &b));
    assert!(def_eq(&c, &[], &b, &a));
    let copies = WHNF_INPUT_CLONES.with(|v| v.replace(None).unwrap());
    assert_eq!(
        copies, 0,
        "a small closed alias needs no ordinary WHNF input copies"
    );
}
