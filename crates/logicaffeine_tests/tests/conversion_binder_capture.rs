use logicaffeine_kernel::{defeq_for_test, infer_type, recheck, Context, Term, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn ty() -> Term {
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
fn env() -> Context {
    let mut c = Context::new();
    c.add_declaration("A", ty());
    c.add("x", g("A"));
    c.add_declaration("a", g("A"));
    c
}
#[test]
fn lambda_alpha_equivalence_cannot_capture_a_free_variable() {
    let c = env();
    let bound = lam("x", g("A"), v("x"));
    let free = lam("y", g("A"), v("x"));
    assert!(!defeq_for_test(&c, &bound, &free));
    assert!(!defeq_for_test(&c, &free, &bound));
}
#[test]
fn pi_conversion_cannot_capture_a_free_index_in_either_kernel() {
    let mut c = env();
    c.add_declaration("P", pi("z", g("A"), ty()));
    let bound = pi("x", g("A"), app(g("P"), v("x")));
    let free = pi("y", g("A"), app(g("P"), v("x")));
    let proof = lam(
        "x",
        g("A"),
        lam(
            "f",
            bound,
            Term::Let {
                name: "bad".into(),
                ty: Box::new(free),
                value: Box::new(v("f")),
                body: Box::new(v("bad")),
            },
        ),
    );
    assert!(infer_type(&c, &proof).is_err());
    assert!(recheck(&c, &proof).is_err());
}
#[test]
fn let_alpha_equivalence_cannot_capture_a_free_variable() {
    let c = env();
    let binding = |name: &str, body: Term| Term::Let {
        name: name.into(),
        ty: Box::new(g("A")),
        value: Box::new(g("a")),
        body: Box::new(body),
    };
    let bound = binding("x", v("x"));
    let free = binding("y", v("x"));
    assert!(!defeq_for_test(&c, &bound, &free));
    assert!(!defeq_for_test(&c, &free, &bound));
}
#[test]
fn different_arguments_of_a_constant_function_still_convert() {
    let mut c = env();
    c.add_declaration("b", g("A"));
    c.declare(
        "constant",
        pi("z", g("A"), g("A")),
        lam("z", g("A"), g("a")),
    )
    .unwrap();
    assert!(defeq_for_test(
        &c,
        &app(g("constant"), g("a")),
        &app(g("constant"), g("b"))
    ));
}
