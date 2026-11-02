use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, infer_type, recheck, BigInt, Context, Literal, Term, Universe,
};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().cloned().fold(f, app)
}
fn pi(n: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn nn(n: i64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_i64(n)))
}
fn power(n: i64) -> Term {
    apps(g("Nat_pow"), &[nn(2), nn(n)])
}
fn context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn indexed(c: &mut Context) {
    c.add_indexed_inductive(
        "ComputedIndex",
        pi("n", g("Nat"), Term::Sort(Universe::Type(0))),
        0,
    );
    c.add_declaration("large_index_value", app(g("ComputedIndex"), power(8)));
}
#[test]
fn computed_unequal_indices_refuse_a_shallow_application_in_both_kernels() {
    let mut c = context();
    indexed(&mut c);
    c.declare(
        "need_small",
        pi("x", app(g("ComputedIndex"), power(4)), g("Nat")),
        lam("x", app(g("ComputedIndex"), power(4)), g("Zero")),
    )
    .unwrap();
    let term = app(g("need_small"), g("large_index_value"));
    assert!(infer_type(&c, &term).is_err());
    assert!(recheck(&c, &term).is_err());
}
#[test]
fn computed_equal_indices_accept_a_shallow_application_in_both_kernels() {
    let mut c = context();
    indexed(&mut c);
    c.declare(
        "need_large",
        pi("x", app(g("ComputedIndex"), nn(256)), g("Nat")),
        lam("x", app(g("ComputedIndex"), nn(256)), g("Zero")),
    )
    .unwrap();
    let term = app(g("need_large"), g("large_index_value"));
    assert_eq!(infer_type(&c, &term).unwrap(), g("Nat"));
    assert_eq!(recheck(&c, &term).unwrap(), g("Nat"));
}
#[test]
fn custom_data_constructors_keep_unequal_computed_fields_distinct() {
    let mut c = context();
    c.add_inductive("DataBox", Term::Sort(Universe::Type(0)));
    c.add_constructor_checked("data_box", "DataBox", pi("value", g("Nat"), g("DataBox")))
        .unwrap();
    let a = app(g("data_box"), power(4));
    let b = app(g("data_box"), power(8));
    assert_eq!(infer_type(&c, &a).unwrap(), g("DataBox"));
    assert_eq!(recheck(&c, &b).unwrap(), g("DataBox"));
    assert!(!defeq_for_test(&c, &a, &b));
    assert!(!defeq_for_test(&c, &b, &a));
}
#[test]
fn propositional_constructors_remain_proof_irrelevant_with_distinct_witnesses() {
    let mut c = context();
    c.add_inductive("ProofBox", Term::Sort(Universe::Prop));
    c.add_constructor_checked(
        "proof_box",
        "ProofBox",
        pi("witness", g("Nat"), g("ProofBox")),
    )
    .unwrap();
    let a = app(g("proof_box"), power(4));
    let b = app(g("proof_box"), power(8));
    assert_eq!(infer_type(&c, &a).unwrap(), g("ProofBox"));
    assert_eq!(recheck(&c, &b).unwrap(), g("ProofBox"));
    assert!(defeq_for_test(&c, &a, &b));
    assert!(defeq_for_test(&c, &b, &a));
}
#[test]
fn data_constructor_proof_fields_still_use_proof_irrelevance() {
    let mut c = context();
    c.add_inductive("ProofBox", Term::Sort(Universe::Prop));
    c.add_constructor_checked(
        "proof_box",
        "ProofBox",
        pi("witness", g("Nat"), g("ProofBox")),
    )
    .unwrap();
    c.add_inductive("DataWithProof", Term::Sort(Universe::Type(0)));
    c.add_constructor_checked(
        "data_with_proof",
        "DataWithProof",
        pi(
            "value",
            g("Nat"),
            pi("proof", g("ProofBox"), g("DataWithProof")),
        ),
    )
    .unwrap();
    let a = apps(
        g("data_with_proof"),
        &[nn(7), app(g("proof_box"), power(4))],
    );
    let b = apps(
        g("data_with_proof"),
        &[nn(7), app(g("proof_box"), power(8))],
    );
    assert_eq!(infer_type(&c, &a).unwrap(), g("DataWithProof"));
    assert_eq!(recheck(&c, &b).unwrap(), g("DataWithProof"));
    assert!(defeq_for_test(&c, &a, &b));
    assert!(defeq_for_test(&c, &b, &a));
}
