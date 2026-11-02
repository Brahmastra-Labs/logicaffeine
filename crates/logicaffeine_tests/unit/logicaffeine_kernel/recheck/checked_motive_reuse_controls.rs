use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn context() -> Context {
    let mut ctx = Context::new();
    ctx.add_inductive("Flag", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("on", "Flag", Term::Global("Flag".into()));
    ctx.add_constructor("off", "Flag", Term::Global("Flag".into()));
    ctx
}
fn measured(f: impl FnOnce() -> RResult<Db>) -> (RResult<Db>, usize) {
    MOTIVE_CHECKS.with(|n| n.set(Some(0)));
    let result = f();
    let checks = MOTIVE_CHECKS.with(|n| n.replace(None).unwrap());
    (result, checks)
}

#[test]
fn nonindexed_match_reuses_its_checked_function_or_raw_motive() {
    let ctx = context();
    for motive in [
        g("Flag"),
        Db::Lam(DbBox::new(g("Flag")), DbBox::new(g("Flag"))),
    ] {
        let (result, checks) =
            measured(|| infer_match(&ctx, &[], &g("on"), &motive, &[g("off"), g("on")]));
        assert_eq!(result.unwrap(), g("Flag"));
        assert_eq!(
            checks, 1,
            "return-type construction must reuse the already checked motive"
        );
    }
}

#[test]
fn checked_motive_retains_outer_dependent_scope() {
    let ctx = context();
    let local = vec![Db::Sort(Universe::Type(0)), Db::Var(0)];
    let motive = Db::Lam(DbBox::new(g("Flag")), DbBox::new(Db::Var(2)));
    let (result, checks) =
        measured(|| infer_match(&ctx, &local, &g("on"), &motive, &[Db::Var(0), Db::Var(0)]));
    assert_eq!(result.unwrap(), Db::Var(1));
    assert_eq!(checks, 1, "the checked open motive must not be re-inferred");
}

#[test]
fn checked_motive_reuse_still_rejects_wrong_domains_and_branches() {
    let ctx = context();
    let wrong_domain = Db::Lam(
        DbBox::new(Db::Sort(Universe::Type(0))),
        DbBox::new(g("Flag")),
    );
    assert!(infer_match(&ctx, &[], &g("on"), &wrong_domain, &[g("on"), g("off")]).is_err());
    assert!(infer_match(
        &ctx,
        &[],
        &g("on"),
        &g("Flag"),
        &[g("on"), Db::Sort(Universe::Type(0))]
    )
    .is_err());
}

#[test]
fn checked_motive_reuse_preserves_large_elimination_restrictions() {
    let mut ctx = context();
    ctx.add_inductive("TwoProofs", Term::Sort(Universe::Prop));
    ctx.add_constructor("proofA", "TwoProofs", Term::Global("TwoProofs".into()));
    ctx.add_constructor("proofB", "TwoProofs", Term::Global("TwoProofs".into()));
    assert!(infer_match(&ctx, &[], &g("proofA"), &g("Flag"), &[g("on"), g("off")]).is_err());
}
