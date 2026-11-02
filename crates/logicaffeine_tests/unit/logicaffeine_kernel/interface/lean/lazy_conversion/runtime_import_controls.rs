use super::*;

#[test]
#[ignore = "kernel-pending: the app-lam original publishes only through a certified normal form (scripts/run-kernel-pending-tests.sh)"]
fn the_entire_original_app_lam_imports_and_publishes_a_checked_definition() {
    let input = include_str!("../../../../../tests/fixtures/lean_arena/app-lam-original.ndjson");
    let (context, report) = check_import(input, false)
        .unwrap_or_else(|e| panic!("entire original app-lam import: {e:?}"));
    let name = "/s:\"dag_app_binder\"";
    let body = context
        .get_definition_body(name)
        .expect("final definition was published");
    let declared = context.get_global(name).expect("final type was published");
    assert!(definitionally_equal(
        &context,
        declared,
        &Term::Global("Nat".into())
    ));
    assert!(definitionally_equal(
        &context,
        body,
        &Term::Global("Zero".into())
    ));
    let check = Term::Let {
        name: "published.value".into(),
        ty: Box::new(declared.clone()),
        value: Box::new(body.clone()),
        body: Box::new(Term::Var("published.value".into())),
    };
    lean_type(&context, &check).unwrap();
    crate::recheck(&context, &check).unwrap();
    assert!(context.pending_obligations().is_empty());
    assert!(report.runtime_declarations.is_empty());
}
