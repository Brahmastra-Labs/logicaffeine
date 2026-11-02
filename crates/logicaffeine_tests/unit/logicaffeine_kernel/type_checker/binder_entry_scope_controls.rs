use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn sort() -> Term {
    Term::Sort(Universe::Type(0))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
#[test]
fn placeholder_lambda_accepts_its_outer_result_type() {
    let c = context().extend("A", sort()).extend("x", v("A"));
    assert!(check_type(&c, &lam("A", g("_"), v("x")), &pi("B", sort(), v("A"))).is_ok());
}
#[test]
fn placeholder_lambda_rejects_a_false_inner_result_type() {
    let c = context().extend("A", sort()).extend("x", v("A"));
    assert!(matches!(
        check_type(&c, &lam("A", g("_"), v("x")), &pi("B", sort(), v("B"))),
        Err(KernelError::TypeMismatch { .. })
    ));
}
#[test]
fn structural_lambda_inference_keeps_the_external_annotation_dependency() {
    let c = context().extend("A", sort()).extend("x", v("A"));
    let t = lam("n", g("Nat"), lam("A", sort(), v("x")));
    let expected = pi("n", g("Nat"), pi("B", sort(), v("A")));
    assert!(def_eq(
        &c,
        &infer_fix_type_structurally(&c, &t).unwrap(),
        &expected
    ));
}
#[test]
fn generated_conversion_local_avoids_free_names_stored_only_in_local_types() {
    // Raw incomplete context deliberately tests name hygiene only. No
    // assertion claims this local type is well formed or kernel accepted.
    let c = context().extend("reserved", v("__conversion'"));
    let name = fresh_conversion_name(Some(&c), &[&g("Nat")]);
    assert_ne!(name, "__conversion'");
}
#[test]
fn generated_conversion_local_checks_body_names_again_after_suffixing() {
    let c = context().extend("__conversion'", g("Nat"));
    let free = v("__conversion'_");
    let name = fresh_conversion_name(Some(&c), &[&free]);
    assert_ne!(name, "__conversion'");
    assert_ne!(name, "__conversion'_");
}
#[test]
fn own_parameter_annotation_is_checked_before_opening_its_scope() {
    let c = context().extend("A", sort());
    // Inner value A has type OUTER A; the domain must never become the
    // freshly introduced value itself.
    let t = lam("A", v("A"), v("A"));
    let expected = pi("value", v("A"), v("A"));
    assert!(def_eq(&c, &infer_type(&c, &t).unwrap(), &expected));
}
