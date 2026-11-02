use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn peano(n: usize) -> Term {
    (0..n).fold(g("Zero"), |t, _| {
        Term::App(Box::new(g("Succ")), Box::new(t))
    })
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor(
        "Succ",
        "Nat",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Nat")),
        },
    );
    c
}
#[test]
fn public_main_and_independent_typing_agree_on_deep_peano_values() {
    let c = context();
    for n in [256, 512] {
        let term = peano(n);
        assert_eq!(infer_type(&c, &term).unwrap(), g("Nat"));
        assert_eq!(crate::recheck(&c, &term).unwrap(), g("Nat"));
    }
}
#[test]
fn computed_peano_depth_boundary_accepts_exactly_the_existing_limit() {
    let c = context();
    let shallow = g("Zero");
    let _outer = enter_kernel(&shallow, "exact Peano boundary").unwrap();
    assert_eq!(infer_type(&c, &peano(MAX_TERM_DEPTH)).unwrap(), g("Nat"));
    assert!(matches!(
        infer_type(&c, &peano(MAX_TERM_DEPTH + 1)),
        Err(KernelError::CertificationError(_))
    ));
}
