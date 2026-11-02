use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies, defeq_for_test, infer_type, int_lit, recheck, BigInt, Context, Term,
};
use logicaffeine_proof::arith::prove_int_eq;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn number(n: &str) -> Term {
    Term::Lit(int_lit(BigInt::parse_decimal(n).unwrap()))
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn op(n: &str, x: Term, y: Term) -> Term {
    app(app(g(n), x), y)
}
fn scaled(n: &str, x: Term) -> Term {
    op("mul", number(n), x)
}
fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c.extend("x", g("Int")).extend("y", g("Int"))
}
fn certifies(c: &Context, l: Term, r: Term) {
    let p =
        prove_int_eq(c, &l, &r).expect("exact symbolic coefficient identity must produce a proof");
    let expected = app(app(app(g("Eq"), g("Int")), l), r);
    assert!(defeq_for_test(c, &infer_type(c, &p).unwrap(), &expected));
    // recheck independently lowers closed terms; Context locals are not its
    // de Bruijn binder environment. Close the actual generated proof and goal.
    let closed_proof = ["y", "x"]
        .into_iter()
        .fold(p.clone(), |body, name| Term::Lambda {
            param: name.into(),
            param_type: Box::new(g("Int")),
            body: Box::new(body),
        });
    let closed_goal = ["y", "x"]
        .into_iter()
        .fold(expected, |body, name| Term::Pi {
            param: name.into(),
            param_type: Box::new(g("Int")),
            body_type: Box::new(body),
        });
    assert!(defeq_for_test(
        c,
        &infer_type(c, &closed_proof).unwrap(),
        &closed_goal
    ));
    assert!(defeq_for_test(
        c,
        &recheck(c, &closed_proof).unwrap(),
        &closed_goal
    ));
    let usage = axiom_dependencies(c, &p);
    assert!(usage.unresolved.is_empty(), "{usage:?}");
    assert!(
        usage.axioms.iter().all(|n| matches!(
            n.as_str(),
            "add_assoc" | "mul_assoc" | "mul_distrib_add" | "sub_elim"
        )),
        "{usage:?}"
    );
}
#[test]
fn multiplying_symbolic_coefficients_promotes_exactly_without_a_closed_subtree() {
    let c = ctx();
    certifies(
        &c,
        op(
            "mul",
            scaled("10000000000", v("x")),
            scaled("10000000000", v("y")),
        ),
        scaled("100000000000000000000", op("mul", v("x"), v("y"))),
    );
}
#[test]
fn adding_coefficients_crosses_the_positive_i64_boundary() {
    let c = ctx();
    certifies(
        &c,
        op("add", scaled("9223372036854775807", v("x")), v("x")),
        scaled("9223372036854775808", v("x")),
    );
}
#[test]
fn negating_the_minimum_i64_symbolic_coefficient_is_exact() {
    let c = ctx();
    certifies(
        &c,
        op("sub", number("0"), scaled("-9223372036854775808", v("x"))),
        scaled("9223372036854775808", v("x")),
    );
}
#[test]
fn large_opposite_coefficients_cancel_to_zero() {
    let c = ctx();
    certifies(
        &c,
        op(
            "add",
            scaled("1000000000000000000000000000000", v("x")),
            scaled("-1000000000000000000000000000000", v("x")),
        ),
        number("0"),
    );
}
#[test]
fn large_coefficients_summing_to_one_recover_the_bare_monomial() {
    let c = ctx();
    certifies(
        &c,
        op(
            "add",
            scaled("1000000000000000000000000000000", v("x")),
            scaled("-999999999999999999999999999999", v("x")),
        ),
        v("x"),
    );
}
#[test]
fn a_closed_large_subexpression_remains_exact_inside_a_symbolic_product() {
    let c = ctx();
    certifies(
        &c,
        op(
            "mul",
            op("mul", number("10000000000"), number("10000000000")),
            v("x"),
        ),
        scaled("100000000000000000000", v("x")),
    );
}
#[test]
fn nearby_but_unequal_large_symbolic_coefficients_are_refused() {
    let c = ctx();
    let lhs = op(
        "mul",
        scaled("10000000000", v("x")),
        scaled("10000000000", v("y")),
    );
    let rhs = scaled("100000000000000000001", op("mul", v("x"), v("y")));
    assert!(prove_int_eq(&c, &lhs, &rhs).is_none());
}
