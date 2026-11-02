use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies, defeq_for_test, infer_type, recheck, BigInt, Context, Literal, Term,
};
use logicaffeine_proof::arith::prove_int_eq;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn lit(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn big(n: &str) -> Term {
    Term::Lit(Literal::BigInt(BigInt::parse_decimal(n).unwrap()))
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn op(n: &str, x: Term, y: Term) -> Term {
    app(app(g(n), x), y)
}
fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn computation_proves(c: &Context, l: Term, r: Term) {
    let p = prove_int_eq(c, &l, &r)
        .expect("closed arbitrary-precision computation must reach proof construction");
    let expected = app(app(app(g("Eq"), g("Int")), l), r);
    assert!(defeq_for_test(c, &infer_type(c, &p).unwrap(), &expected));
    assert!(defeq_for_test(c, &recheck(c, &p).unwrap(), &expected));
    let trust = axiom_dependencies(c, &p);
    assert!(
        trust.axioms.is_empty() && trust.unresolved.is_empty(),
        "{trust:?}"
    );
}
#[test]
fn addition_beyond_i64_uses_exact_kernel_computation() {
    let c = ctx();
    computation_proves(
        &c,
        op("add", big("9223372036854775808"), lit(1)),
        big("9223372036854775809"),
    );
}
#[test]
fn overflowing_literal_coefficients_compute_before_polynomial_extraction() {
    let c = ctx();
    computation_proves(
        &c,
        op("mul", lit(10_000_000_000), lit(10_000_000_000)),
        big("100000000000000000000"),
    );
}
#[test]
fn nested_signed_bigint_computation_canonicalizes_back_to_small_int() {
    let c = ctx();
    let positive = big("1000000000000000000000000000000");
    let negative = big("-1000000000000000000000000000000");
    computation_proves(
        &c,
        op(
            "add",
            op("mul", positive, lit(3)),
            op("mul", negative, lit(3)),
        ),
        lit(0),
    );
}
#[test]
fn unequal_ground_bigint_values_never_receive_a_proof() {
    let c = ctx();
    let n = big("9223372036854775808");
    assert!(prove_int_eq(&c, &op("add", n.clone(), lit(1)), &op("add", n, lit(2))).is_none());
}
