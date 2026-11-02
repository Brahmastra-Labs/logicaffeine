use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn t(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Db, a: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(a))
}
fn context() -> Context {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    c.add_inductive("Flag", Term::Sort(Universe::Type(0)));
    c.add_constructor("on", "Flag", t("Flag"));
    c.add_constructor("off", "Flag", t("Flag"));
    c.add_inductive(
        "Tag",
        Term::Pi {
            param: "flag".into(),
            param_type: Box::new(t("Flag")),
            body_type: Box::new(Term::Sort(Universe::Type(0))),
        },
    );
    c.add_constructor(
        "tagOn",
        "Tag",
        Term::App(Box::new(t("Tag")), Box::new(t("on"))),
    );
    c
}
fn measured(f: impl FnOnce() -> RResult<Db>) -> (Db, usize) {
    WHNF_INPUT_CLONES.with(|v| v.set(Some(0)));
    let result = f();
    let copies = WHNF_INPUT_CLONES.with(|v| v.replace(None).unwrap());
    (result.unwrap(), copies)
}
#[test]
fn constructor_case_moves_its_fresh_motive_application_into_reduction() {
    let c = context();
    let motive = Db::Lam(DbBox::new(g("Flag")), DbBox::new(g("Flag")));
    let (result, copies) = measured(|| case_type(&c, &motive, "on", &[], &g("Flag")));
    assert_eq!(result, g("Flag"));
    assert_eq!(copies, 0);
}
#[test]
fn indexed_constructor_case_moves_its_fresh_motive_application_into_reduction() {
    let c = context();
    let motive = Db::Lam(
        DbBox::new(g("Flag")),
        DbBox::new(Db::Lam(
            DbBox::new(app(g("Tag"), Db::Var(0))),
            DbBox::new(g("Flag")),
        )),
    );
    let (result, copies) =
        measured(|| case_type_indexed(&c, &motive, "tagOn", &[], &app(g("Tag"), g("on"))));
    assert_eq!(result, g("Flag"));
    assert_eq!(copies, 0);
}
#[test]
fn indexed_return_type_moves_its_fresh_motive_application_into_reduction() {
    let c = context();
    let motive = Db::Lam(
        DbBox::new(g("Flag")),
        DbBox::new(Db::Lam(
            DbBox::new(app(g("Tag"), Db::Var(0))),
            DbBox::new(g("Flag")),
        )),
    );
    let (result, copies) =
        measured(|| match_return_type(&c, &[], &motive, &g("tagOn"), &app(g("Tag"), g("on"))));
    assert_eq!(result, g("Flag"));
    assert_eq!(copies, 0);
}

#[test]
fn nonindexed_match_and_return_type_move_fresh_motive_applications() {
    let c = context();
    let (result, copies) =
        measured(|| infer_match(&c, &[], &g("on"), &g("Flag"), &[g("off"), g("on")]));
    assert_eq!(result, g("Flag"));
    assert_eq!(copies, 0);
    let (result, copies) =
        measured(|| match_return_type(&c, &[], &g("Flag"), &g("on"), &g("Flag")));
    assert_eq!(result, g("Flag"));
    assert_eq!(copies, 0);
}
#[test]
fn owned_match_applications_preserve_outer_dependent_scope() {
    let c = context();
    let locals = vec![Db::Sort(Universe::Type(0)), Db::Var(0)];
    let (result, copies) = measured(|| {
        infer_match(
            &c,
            &locals,
            &g("on"),
            &Db::Var(1),
            &[Db::Var(0), Db::Var(0)],
        )
    });
    assert_eq!(result, Db::Var(1));
    assert_eq!(copies, 0);
}

#[test]
fn indexed_motive_consumes_each_owned_residual_telescope() {
    for indices in [1usize, 8, 32] {
        let mut c = context();
        let mut arity = Term::Sort(Universe::Type(0));
        for i in (0..indices).rev() {
            arity = Term::Pi {
                param: format!("i{i}"),
                param_type: Box::new(t("Flag")),
                body_type: Box::new(arity),
            };
        }
        c.add_inductive("ManyIndices", arity);
        let mut family = t("ManyIndices");
        let mut disc_ty = g("ManyIndices");
        for i in 0..indices {
            family = Term::App(Box::new(family), Box::new(Term::Var(format!("i{i}"))));
            disc_ty = app(disc_ty, g("on"));
        }
        let mut motive_ty = Term::Pi {
            param: "major".into(),
            param_type: Box::new(family),
            body_type: Box::new(Term::Sort(Universe::Type(0))),
        };
        for i in (0..indices).rev() {
            motive_ty = Term::Pi {
                param: format!("i{i}"),
                param_type: Box::new(t("Flag")),
                body_type: Box::new(motive_ty),
            };
        }
        c.add_declaration("motive", motive_ty);
        WHNF_INPUT_CLONES.with(|v| v.set(Some(0)));
        let result =
            check_indexed_motive(&c, &[], &g("motive"), &disc_ty, 0, &vec![g("on"); indices]);
        let copies = WHNF_INPUT_CLONES.with(|v| v.replace(None).unwrap());
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            copies, 0,
            "owned residual telescopes must not copy their tails at {indices} indices"
        );
    }
}
