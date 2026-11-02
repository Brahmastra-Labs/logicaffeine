use super::*;
use crate::prelude::StandardLibrary;

fn context() -> Context {
    let mut context = Context::new();
    StandardLibrary::register(&mut context);
    context
}

fn global(name: &str) -> Term {
    Term::Global(name.to_owned())
}
fn app(function: Term, argument: Term) -> Term {
    Term::App(Box::new(function), Box::new(argument))
}
fn six() -> Term {
    Term::Lit(Literal::Int(6))
}
fn equality(left: Term, right: Term) -> Term {
    term_to_syntax(&app(app(app(global("Eq"), global("Int")), left), right))
        .expect("integer equality is reflectable")
}
fn defer(context: &mut Context, name: &str) {
    context
        .declare(name, global("Int"), global("not_registered_yet"))
        .unwrap();
    assert!(context
        .pending_obligations()
        .iter()
        .any(|(pending, _)| *pending == name));
    assert!(context.get_global(name).is_none());
}

#[test]
fn pending_definition_cannot_be_certified_as_a_finished_normal_form() {
    let mut context = context();
    defer(&mut context, "waiting");
    assert!(!simp_compute_check(
        &context,
        &equality(global("waiting"), global("waiting"))
    ));
}

#[test]
fn erasing_a_pending_argument_does_not_admit_it_to_a_computation_proof() {
    let mut context = context();
    defer(&mut context, "waiting");
    let discard = Term::Lambda {
        param: "unused".into(),
        param_type: Box::new(global("Int")),
        body: Box::new(six()),
    };
    let expression = app(discard, global("waiting"));
    assert_eq!(normalize_checked(&context, &expression).unwrap(), six());
    assert!(!simp_compute_check(&context, &equality(expression, six())));
}

#[test]
fn admitted_definition_cannot_hide_a_dependency_that_became_pending() {
    let mut context = context();
    context.declare("later", global("Int"), six()).unwrap();
    context
        .declare("reader", global("Int"), global("later"))
        .unwrap();
    assert!(simp_compute_check(
        &context,
        &equality(global("reader"), six())
    ));
    // The stable-meaning gate refuses to re-point `later` under `reader`, and `reader` keeps
    // computing; the cache under test is reached only when the change is permitted.
    let kept = context
        .refused_meaning_change_for_test(|c| {
            c.declare("later", global("Int"), global("not_registered_yet"))
        })
        .expect("`reader` depends on `later`");
    assert!(simp_compute_check(
        &kept,
        &equality(global("reader"), six())
    ));
    context.permit_meaning_changes_for_cache_tests();
    defer(&mut context, "later");
    assert!(!simp_compute_check(
        &context,
        &equality(global("reader"), global("reader"))
    ));
}

#[test]
fn resolving_a_pending_definition_restores_computation_without_stale_refusal() {
    let mut context = context();
    defer(&mut context, "waiting");
    assert!(!simp_compute_check(
        &context,
        &equality(global("waiting"), global("waiting"))
    ));
    context
        .declare("not_registered_yet", global("Int"), six())
        .unwrap();
    assert!(context.pending_obligations().is_empty());
    assert!(simp_compute_check(
        &context,
        &equality(global("waiting"), six())
    ));
}

#[test]
fn unrelated_pending_definition_does_not_disable_valid_computation() {
    let mut context = context();
    defer(&mut context, "waiting");
    context.declare("ready", global("Int"), six()).unwrap();
    assert!(simp_compute_check(
        &context,
        &equality(global("ready"), six())
    ));
    assert!(!simp_compute_check(
        &context,
        &equality(global("ready"), Term::Lit(Literal::Int(7)))
    ));
}
