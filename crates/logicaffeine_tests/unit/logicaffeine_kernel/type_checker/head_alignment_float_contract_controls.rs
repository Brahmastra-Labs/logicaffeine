use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_declaration("Nat", Term::Sort(Universe::Type(0)));
    c.add_declaration("Zero", g("Nat"));
    c.add_declaration("Float", Term::Sort(Universe::Type(0)));
    c
}
#[test]
fn typed_beta_erasure_preserves_bit_exact_float_literal_semantics() {
    let c = context();
    let constant = Term::Lambda {
        param: "unused".into(),
        param_type: Box::new(g("Float")),
        body: Box::new(g("Zero")),
    };
    let term = app(constant, Term::Lit(crate::Literal::Float(f64::NAN)));
    assert_eq!(infer_type(&c, &term).unwrap(), g("Nat"));
    assert!(conversion_by_unfolded_head(&c, &term, &g("Zero")));
    assert!(def_eq(&c, &term, &g("Zero")));
}
#[test]
fn opaque_float_arguments_preserve_distinct_nan_payloads() {
    let mut c = context();
    c.add_declaration(
        "observe",
        Term::Pi {
            param: "x".into(),
            param_type: Box::new(g("Float")),
            body_type: Box::new(g("Nat")),
        },
    );
    let at = |bits| {
        app(
            g("observe"),
            Term::Lit(crate::Literal::Float(f64::from_bits(bits))),
        )
    };
    let a = at(0x7ff8000000000001);
    let b = at(0x7ff8000000000002);
    assert_eq!(infer_type(&c, &a).unwrap(), g("Nat"));
    assert_eq!(infer_type(&c, &b).unwrap(), g("Nat"));
    assert!(!alpha_eq(&a, &b));
    let mut budget = 256;
    assert!(!bounded_conversion(&c, &a, &b, &mut budget));
    assert!(!def_eq(&c, &a, &b));
}
