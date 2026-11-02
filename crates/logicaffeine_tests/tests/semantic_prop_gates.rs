//! Universe-equivalent Prop forms must obey the same elimination and equality rules.
use logicaffeine_kernel::{Context, DoubleCheck, KernelError, Term, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
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
fn ty() -> Term {
    Term::Sort(Universe::Type(0))
}
fn raw_prop() -> Term {
    Term::Sort(Universe::IMax(
        Box::new(Universe::Type(4)),
        Box::new(Universe::Prop),
    ))
}
fn proof_context() -> Context {
    let mut c = Context::new();
    c.add_inductive("P", raw_prop());
    c.add_constructor_checked("left", "P", g("P")).unwrap();
    c.add_constructor_checked("right", "P", g("P")).unwrap();
    c
}
#[test]
fn raw_prop_data_projection_is_forbidden_by_both_kernels() {
    let mut c = Context::new();
    c.add_inductive("Data", ty());
    c.add_constructor_checked("data", "Data", g("Data"))
        .unwrap();
    c.add_inductive("P", raw_prop());
    c.add_constructor_checked("pack", "P", pi("d", g("Data"), g("P")))
        .unwrap();
    c.add_declaration("proof", g("P"));
    let projection = Term::Match {
        discriminant: Box::new(g("proof")),
        motive: Box::new(lam("p", g("P"), g("Data"))),
        cases: vec![lam("d", g("Data"), v("d"))],
    };
    assert!(matches!(
        logicaffeine_kernel::infer_type(&c, &projection),
        Err(KernelError::InvalidMotive(_))
    ));
    assert!(logicaffeine_kernel::recheck(&c, &projection).is_err());
}
#[test]
fn proof_irrelevance_recognizes_raw_prop_in_both_kernels() {
    let mut c = proof_context();
    assert!(logicaffeine_kernel::defeq_for_test(
        &c,
        &g("left"),
        &g("right")
    ));
    c.add_declaration("Family", pi("p", g("P"), ty()));
    c.add_declaration("value", app(g("Family"), g("left")));
    let use_value = app(lam("x", app(g("Family"), g("right")), v("x")), g("value"));
    assert!(matches!(
        logicaffeine_kernel::double_check(&c, &use_value),
        DoubleCheck::Agreed
    ));
}
#[test]
fn raw_prop_constructors_cannot_justify_an_unreachable_index_branch() {
    let mut c = proof_context();
    c.add_indexed_inductive("Indexed", pi("p", g("P"), ty()), 0);
    c.add_constructor_checked("indexed_left", "Indexed", app(g("Indexed"), g("left")))
        .unwrap();
    c.add_declaration("at_right", app(g("Indexed"), g("right")));
    c.add_inductive("False", Term::Sort(Universe::Prop));
    let empty = Term::Match {
        discriminant: Box::new(g("at_right")),
        motive: Box::new(lam(
            "p",
            g("P"),
            lam("x", app(g("Indexed"), v("p")), g("False")),
        )),
        cases: vec![],
    };
    assert!(logicaffeine_kernel::infer_type(&c, &empty).is_err());
    assert!(logicaffeine_kernel::recheck(&c, &empty).is_err());
}
#[test]
fn monomorphic_recursor_for_raw_empty_prop_keeps_large_elimination() {
    let mut c = Context::new();
    c.add_inductive("Empty", raw_prop());
    let (rec_ty, body) = logicaffeine_kernel::derive_recursor(&c, "Empty").unwrap();
    let Term::Pi { param_type, .. } = rec_ty else {
        panic!("motive")
    };
    let Term::Pi { body_type, .. } = *param_type else {
        panic!("motive telescope")
    };
    assert_eq!(*body_type, ty());
    assert!(matches!(
        logicaffeine_kernel::double_check(&c, &body),
        DoubleCheck::Agreed
    ));
}
#[test]
fn raw_prop_proof_field_keeps_singleton_large_elimination() {
    let mut c = proof_context();
    c.add_inductive("Wrapper", Term::Sort(Universe::Prop));
    c.add_constructor_checked("wrap", "Wrapper", pi("p", g("P"), g("Wrapper")))
        .unwrap();
    let (levels, _, body) =
        logicaffeine_kernel::derive_recursor_universe_poly(&c, "Wrapper").unwrap();
    assert_eq!(
        levels.len(),
        1,
        "proof-only constructor is eligible for large elimination"
    );
    assert!(matches!(
        logicaffeine_kernel::double_check(&c, &body),
        DoubleCheck::Agreed
    ));
}
