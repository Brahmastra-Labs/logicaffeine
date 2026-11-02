use super::*;
fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(name: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: name.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn proof_context() -> Context {
    let mut ctx = Context::new();
    ctx.set_auto_auxiliaries(false);
    ctx.add_inductive("P", Term::Sort(Universe::Prop));
    ctx.add_constructor("p", "P", g("P"));
    ctx
}
fn entries<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = PEANO_INFERENCE_ENTRIES.with(|c| c.get());
    let answer = f();
    let after = PEANO_INFERENCE_ENTRIES.with(|c| c.get());
    (answer, after - before)
}
fn absent(ctx: &Context, name: &str) {
    assert!(ctx.get_global(name).is_none());
    assert!(ctx.poly_declaration(name).is_none());
    assert!(ctx.get_definition_body(name).is_none());
    assert!(crate::axiom_dependencies_of(ctx, name).is_none());
}
#[test]
fn opaque_gateway_infers_simple_type_and_body_once_each() {
    let mut ctx = proof_context();
    let (result, count) = entries(|| ctx.declare_opaque_checked("checked", vec![], g("P"), g("p")));
    result.unwrap();
    assert_eq!(
        count, 2,
        "actual main inference entries; no duplicated declared-type inference"
    );
    assert_eq!(ctx.get_global("checked"), Some(&g("P")));
    assert!(ctx.get_definition_body("checked").is_none());
    assert!(crate::axiom_dependencies_of(&ctx, "checked")
        .unwrap()
        .axioms
        .is_empty());
}
#[test]
fn sort_and_type_helper_retains_raw_alias_without_reinferring() {
    let mut ctx = Context::new();
    ctx.set_auto_auxiliaries(false);
    ctx.declare(
        "SortAlias",
        Term::Sort(Universe::Type(1)),
        Term::Sort(Universe::Type(0)),
    )
    .unwrap();
    ctx.add_declaration("A", g("SortAlias"));
    let (result, count) = entries(|| infer_type_as_type(&ctx, &g("A")));
    let raw = result.unwrap();
    assert_eq!(
        raw,
        g("SortAlias"),
        "retain unnormalized result for independent exact agreement"
    );
    assert_eq!(count, 1, "the sort check reuses the actual inferred type");
    assert_eq!(infer_sort(&ctx, &g("A")).unwrap(), Universe::Type(0));
    assert_eq!(crate::recheck(&ctx, &g("A")).unwrap(), g("SortAlias"));
}
#[test]
fn raw_alias_result_still_publishes_checked_opaque_data() {
    let mut ctx = Context::new();
    ctx.set_auto_auxiliaries(false);
    ctx.declare(
        "SortAlias",
        Term::Sort(Universe::Type(1)),
        Term::Sort(Universe::Type(0)),
    )
    .unwrap();
    ctx.add_declaration("A", g("SortAlias"));
    ctx.add_declaration("x", g("A"));
    ctx.declare_opaque_checked("value", vec![], g("A"), g("x"))
        .unwrap();
    assert_eq!(ctx.get_global("value"), Some(&g("A")));
    assert!(ctx.get_definition_body("value").is_none());
    let usage = crate::axiom_dependencies_of(&ctx, "value").unwrap();
    assert_eq!(
        usage.axioms,
        ["A".to_string(), "x".to_string()].into_iter().collect()
    );
}
#[test]
fn non_types_and_ill_typed_type_expressions_are_atomic_refusals() {
    let mut ctx = proof_context();
    for (name, ty) in [
        ("not_type", g("p")),
        ("bad_application", app(g("p"), g("p"))),
    ] {
        assert!(infer_type_as_type(&ctx, &ty).is_err());
        assert!(ctx
            .declare_opaque_checked(name, vec![], ty, g("p"))
            .is_err());
        absent(&ctx, name);
    }
}
#[test]
fn raw_sort_normalization_failure_still_refuses_before_publication() {
    let mut ctx = proof_context();
    let omega_lam = lam("x", Term::Sort(Universe::Prop), app(v("x"), v("x")));
    let omega = app(omega_lam.clone(), omega_lam);
    // The assumption door checks that a declared type is a type, so the raw sort is refused
    // there, recorded, before anything is published.
    ctx.add_declaration("RawBad", omega.clone());
    absent(&ctx, "RawBad");
    assert!(ctx
        .refused_declarations()
        .iter()
        .any(|(name, _)| name == "RawBad"));
    assert!(infer_type_as_type(&ctx, &omega).is_err());
    assert!(ctx
        .declare_opaque_checked("bad", vec![], omega, g("p"))
        .is_err());
    absent(&ctx, "bad");
}
#[test]
fn depth_and_universe_telescope_doors_are_not_bypassed() {
    let mut ctx = proof_context();
    let mut deep = g("P");
    for _ in 0..=MAX_TERM_DEPTH {
        deep = Term::Pi {
            param: "x".into(),
            param_type: Box::new(g("P")),
            body_type: Box::new(deep),
        };
    }
    assert!(infer_type_as_type(&ctx, &deep).is_err());
    assert!(ctx
        .declare_opaque_checked("deep", vec![], deep, g("p"))
        .is_err());
    absent(&ctx, "deep");
    let u = Term::Sort(Universe::Var("u".into()));
    assert!(ctx
        .declare_opaque_checked("missing_u", vec![], u, Term::Sort(Universe::Prop))
        .is_err());
    absent(&ctx, "missing_u");
    assert!(ctx
        .declare_opaque_checked("duplicate_u", vec!["u".into(), "u".into()], g("P"), g("p"))
        .is_err());
    absent(&ctx, "duplicate_u");
}
