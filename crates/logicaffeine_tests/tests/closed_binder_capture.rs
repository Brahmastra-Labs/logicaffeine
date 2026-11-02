//! Closed dependent-binder capture regressions. No free-context bridge is required.
use logicaffeine_kernel::{
    defeq_for_test, double_check, infer_type, recheck, Context, DoubleCheck, KernelError,
    ReCheckError, Term, Universe,
};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn sort() -> Term {
    Term::Sort(Universe::Type(0))
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
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
    logicaffeine_kernel::prelude::StandardLibrary::register(&mut c);
    c
}
fn repeated() -> Term {
    lam("A", sort(), lam("x", v("A"), lam("A", sort(), v("x"))))
}
fn correct() -> Term {
    pi(
        "Outer",
        sort(),
        pi("x", v("Outer"), pi("Inner", sort(), v("Outer"))),
    )
}
fn incorrect() -> Term {
    pi(
        "Outer",
        sort(),
        pi("x", v("Outer"), pi("Inner", sort(), v("Inner"))),
    )
}
fn goal(body: Term, ty: Term) -> Term {
    Term::Let {
        name: "checked".into(),
        ty: Box::new(ty),
        value: Box::new(body),
        body: Box::new(v("checked")),
    }
}
fn instantiated() -> Term {
    let body = lam(
        "A",
        sort(),
        lam("x", v("A"), app(lam("A", sort(), v("x")), g("Nat"))),
    );
    app(app(body, g("Bool")), g("true"))
}
fn let_capture() -> Term {
    lam(
        "A",
        sort(),
        lam(
            "x",
            v("A"),
            Term::Let {
                name: "A".into(),
                ty: Box::new(sort()),
                value: Box::new(g("Nat")),
                body: Box::new(v("x")),
            },
        ),
    )
}
#[test]
fn main_inference_preserves_the_outer_type_under_repeated_lambda_name() {
    let c = context();
    let ty = infer_type(&c, &repeated()).unwrap();
    assert!(
        defeq_for_test(&c, &ty, &correct()),
        "last result must remain the outer type"
    );
    assert!(!defeq_for_test(&c, &ty, &incorrect()));
}
#[test]
fn independent_inference_confirms_the_correct_closed_telescope() {
    let c = context();
    let ty = recheck(&c, &repeated()).unwrap();
    assert!(defeq_for_test(&c, &ty, &correct()));
    assert!(!defeq_for_test(&c, &ty, &incorrect()));
}
#[test]
fn main_rejects_an_annotated_goal_claiming_the_inner_type() {
    let c = context();
    infer_type(&c, &incorrect()).unwrap();
    assert!(matches!(
        infer_type(&c, &goal(repeated(), incorrect())),
        Err(KernelError::TypeMismatch { .. })
    ));
}
#[test]
fn independent_rejects_an_annotated_goal_claiming_the_inner_type() {
    let c = context();
    recheck(&c, &incorrect()).unwrap();
    assert!(matches!(
        recheck(&c, &goal(repeated(), incorrect())),
        Err(ReCheckError::Ill(_))
    ));
}
#[test]
fn instantiated_capture_has_bool_type_in_both_checkers() {
    let c = context();
    let value = instantiated();
    assert!(defeq_for_test(
        &c,
        &infer_type(&c, &value).unwrap(),
        &g("Bool")
    ));
    assert!(defeq_for_test(
        &c,
        &recheck(&c, &value).unwrap(),
        &g("Bool")
    ));
    assert_eq!(double_check(&c, &value), DoubleCheck::Agreed);
}
#[test]
fn instantiated_capture_cannot_certify_true_as_nat() {
    let c = context();
    let claim = goal(instantiated(), g("Nat"));
    assert!(matches!(
        infer_type(&c, &claim),
        Err(KernelError::TypeMismatch { .. })
    ));
    assert!(matches!(recheck(&c, &claim), Err(ReCheckError::Ill(_))));
}
#[test]
fn both_checkers_accept_the_correct_goal_and_agree() {
    let c = context();
    let claim = goal(repeated(), correct());
    infer_type(&c, &claim).unwrap();
    recheck(&c, &claim).unwrap();
    assert_eq!(double_check(&c, &repeated()), DoubleCheck::Agreed);
}
#[test]
fn alpha_renaming_does_not_change_the_inferred_type() {
    let c = context();
    let renamed = lam(
        "Outer",
        sort(),
        lam("x", v("Outer"), lam("Inner", sort(), v("x"))),
    );
    let a = infer_type(&c, &repeated()).unwrap();
    let b = infer_type(&c, &renamed).unwrap();
    assert!(defeq_for_test(&c, &a, &b));
    assert!(defeq_for_test(
        &c,
        &recheck(&c, &repeated()).unwrap(),
        &recheck(&c, &renamed).unwrap()
    ));
}
#[test]
fn let_binding_cannot_capture_an_existing_local_type() {
    let c = context();
    let expected = pi("Outer", sort(), pi("x", v("Outer"), v("Outer")));
    assert!(defeq_for_test(
        &c,
        &infer_type(&c, &let_capture()).unwrap(),
        &expected
    ));
    assert!(defeq_for_test(
        &c,
        &recheck(&c, &let_capture()).unwrap(),
        &expected
    ));
    assert_eq!(double_check(&c, &let_capture()), DoubleCheck::Agreed);
}
#[test]
fn ordinary_nearest_value_shadowing_stays_legal() {
    let c = context();
    let t = lam("x", g("Nat"), lam("x", g("Bool"), v("x")));
    let expected = pi("n", g("Nat"), pi("b", g("Bool"), g("Bool")));
    assert!(defeq_for_test(&c, &infer_type(&c, &t).unwrap(), &expected));
    assert!(defeq_for_test(&c, &recheck(&c, &t).unwrap(), &expected));
    assert_eq!(double_check(&c, &t), DoubleCheck::Agreed);
}
