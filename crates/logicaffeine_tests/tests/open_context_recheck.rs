//! Additive open-context requirements; no existing failed test is rewritten.
use logicaffeine_kernel::{
    defeq_for_test, double_check, infer_type, recheck, BigInt, Context, DoubleCheck, Literal,
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
fn base() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", sort());
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    c.add_inductive("Bool", sort());
    c.add_constructor("false", "Bool", g("Bool"));
    c.add_constructor("true", "Bool", g("Bool"));
    c
}
fn accepted(c: &Context, t: &Term, expected: &Term) {
    let main = infer_type(c, t).expect("valid main term");
    let independent = recheck(c, t).expect("independent checked active local telescope");
    assert!(defeq_for_test(c, &main, expected));
    assert!(defeq_for_test(c, &independent, expected));
    assert_eq!(double_check(c, t), DoubleCheck::Agreed);
}
fn ill(c: &Context, t: &Term) {
    assert!(matches!(recheck(c, t), Err(ReCheckError::Ill(_))));
}
#[test]
fn a_free_local_uses_its_checked_active_type() {
    let c = base().extend("n", g("Nat"));
    accepted(&c, &v("n"), &g("Nat"));
}
#[test]
fn dependency_order_is_recovered_independently_of_insertion_order() {
    let mut c = base();
    c.add("x", v("A"));
    c.add("A", sort());
    assert_eq!(recheck(&c, &v("x")).unwrap(), v("A"));
    accepted(&c, &v("x"), &v("A"));
}
#[test]
fn dependent_local_functions_use_the_same_checked_telescope() {
    let c = base()
        .extend("A", sort())
        .extend("x", v("A"))
        .extend("f", pi("argument", v("A"), v("A")));
    accepted(&c, &app(v("f"), v("x")), &v("A"));
}
#[test]
fn nearest_lambda_binder_shadows_an_external_value() {
    let c = base().extend("x", g("Nat"));
    accepted(
        &c,
        &lam("x", g("Bool"), v("x")),
        &pi("b", g("Bool"), g("Bool")),
    );
}
#[test]
fn let_shadowing_preserves_the_outer_value_used_in_its_initializer() {
    let c = base().extend("x", g("Nat"));
    let t = Term::Let {
        name: "x".into(),
        ty: Box::new(g("Nat")),
        value: Box::new(v("x")),
        body: Box::new(v("x")),
    };
    accepted(&c, &t, &g("Nat"));
}
#[test]
fn active_name_map_replacement_does_not_recover_a_displaced_binding() {
    let c = base().extend("x", g("Nat")).extend("x", g("Bool"));
    assert_eq!(recheck(&c, &v("x")).unwrap(), g("Bool"));
    accepted(&c, &v("x"), &g("Bool"));
}
#[test]
fn invalid_local_types_are_not_trusted_as_assumptions() {
    let c = base().extend("bad", g("Zero"));
    ill(&c, &v("bad"));
    let c = base().extend("bad", Term::Lit(Literal::Nat(BigInt::from_u64(2))));
    ill(&c, &v("bad"));
}
#[test]
fn missing_and_cyclic_type_dependencies_are_refused() {
    let c = base().extend("x", v("Missing"));
    ill(&c, &v("x"));
    let c = base()
        .extend("x", v("A"))
        .extend("A", v("B"))
        .extend("B", v("A"));
    ill(&c, &v("x"));
}
#[test]
fn inactive_invalid_bindings_do_not_poison_closed_terms() {
    let c = base().extend("bad", Term::Hole).extend("cycle", v("cycle"));
    accepted(&c, &g("Zero"), &g("Nat"));
}
#[test]
fn explicit_local_universe_instances_keep_header_arity_checks() {
    let mut c = base();
    c.add_universe_poly_inductive(
        "Carrier",
        vec!["u".into()],
        Term::Sort(Universe::Var("u".into())),
        0,
    );
    let ty = Term::Const {
        name: "Carrier".into(),
        levels: vec![Universe::Type(0)],
    };
    c.add("x", ty.clone());
    accepted(&c, &v("x"), &ty);
    c.add(
        "bad",
        Term::Const {
            name: "Carrier".into(),
            levels: vec![],
        },
    );
    ill(&c, &v("bad"));
}
#[test]
fn global_and_local_references_with_the_same_spelling_remain_distinct() {
    let c = base().extend("Zero", g("Bool"));
    accepted(&c, &v("Zero"), &g("Bool"));
    accepted(&c, &g("Zero"), &g("Nat"));
}
#[test]
fn raised_internal_binders_never_capture_external_v_number_names() {
    let c = base().extend("v0", sort()).extend("v1", v("v0"));
    let term = lam("argument", g("Nat"), v("v1"));
    let ty = recheck(&c, &term).unwrap();
    let Term::Pi {
        param,
        param_type,
        body_type,
    } = ty
    else {
        panic!("function type")
    };
    assert_ne!(param, "v0");
    assert_ne!(param, "v1");
    assert_eq!(*param_type, g("Nat"));
    assert_eq!(*body_type, v("v0"));
    assert_eq!(double_check(&c, &term), DoubleCheck::Agreed);
}
#[test]
fn both_checkers_preserve_the_outer_dependent_type() {
    let c = base().extend("T", sort()).extend("x", v("T"));
    let term = lam("T", sort(), v("x"));
    let ty = recheck(&c, &term).unwrap();
    let Term::Pi {
        param,
        param_type,
        body_type,
    } = ty
    else {
        panic!("function type")
    };
    assert_ne!(param, "T");
    assert_eq!(*param_type, sort());
    assert_eq!(*body_type, v("T"));
    assert_eq!(double_check(&c, &term), DoubleCheck::Agreed);
}
#[test]
fn ill_typed_open_application_still_has_two_actual_rejections() {
    let c = base()
        .extend("f", pi("n", g("Nat"), g("Nat")))
        .extend("b", g("Bool"));
    let term = app(v("f"), v("b"));
    assert!(infer_type(&c, &term).is_err());
    ill(&c, &term);
    assert_eq!(double_check(&c, &term), DoubleCheck::Agreed);
}
#[test]
fn hole_in_a_demanded_local_type_is_incompleteness_not_acceptance() {
    let c = base().extend("x", Term::Hole);
    assert!(matches!(
        recheck(&c, &v("x")),
        Err(ReCheckError::Unsupported(_))
    ));
}

#[test]
fn stored_let_types_are_checked_by_independent_zeta_conversion() {
    let ty = Term::Let {
        name: "Alias".into(),
        ty: Box::new(sort()),
        value: Box::new(g("Nat")),
        body: Box::new(v("Alias")),
    };
    let c = base().extend("x", ty);
    accepted(&c, &v("x"), &g("Nat"));
}
#[test]
fn active_replacement_cannot_recover_the_old_dependency_type() {
    let c = base()
        .extend("A", sort())
        .extend("x", v("A"))
        .extend("A", g("Nat"));
    // Active A is now a Nat-valued local, so x:A is not a valid local declaration.
    ill(&c, &v("x"));
}
#[test]
fn lexical_bound_occurrences_do_not_demand_an_invalid_external_binding() {
    let c = base().extend("x", Term::Hole);
    accepted(
        &c,
        &lam("x", g("Nat"), v("x")),
        &pi("n", g("Nat"), g("Nat")),
    );
}
#[test]
fn demanded_local_depth_is_admitted_before_recursive_lowering() {
    let mut ty = g("Nat");
    for i in 0..256 {
        ty = pi(&format!("n{i}"), g("Nat"), ty);
    }
    let c = base().extend("f", ty.clone());
    accepted(&c, &v("f"), &ty);
    // Existing public kernel depth ceiling is1600; this local type is one over.
    for i in 256..1601 {
        ty = pi(&format!("n{i}"), g("Nat"), ty);
    }
    let c = base().extend("f", ty);
    assert!(matches!(
        recheck(&c, &v("f")),
        Err(ReCheckError::Unsupported(_))
    ));
}

#[test]
fn invalid_demanded_assumption_is_disagreement_when_main_actually_accepts() {
    let c = base().extend("bad", g("Zero"));
    assert_eq!(infer_type(&c, &v("bad")).unwrap(), g("Zero"));
    assert!(matches!(recheck(&c, &v("bad")), Err(ReCheckError::Ill(_))));
    assert!(matches!(
        double_check(&c, &v("bad")),
        DoubleCheck::Disagree(_)
    ));
}
#[test]
fn demanded_hole_never_becomes_double_checker_agreement() {
    let c = base().extend("bad", Term::Hole);
    assert!(matches!(
        recheck(&c, &v("bad")),
        Err(ReCheckError::Unsupported(_))
    ));
    assert!(matches!(
        double_check(&c, &v("bad")),
        DoubleCheck::MainOnlyReCheckerIncomplete(_)
    ));
}
#[test]
fn demanded_depth_refusal_never_becomes_double_checker_agreement() {
    let mut ty = g("Nat");
    for i in 0..1601 {
        ty = pi(&format!("n{i}"), g("Nat"), ty);
    }
    let c = base().extend("deep", ty);
    assert!(matches!(
        double_check(&c, &v("deep")),
        DoubleCheck::MainOnlyReCheckerIncomplete(_)
    ));
}
