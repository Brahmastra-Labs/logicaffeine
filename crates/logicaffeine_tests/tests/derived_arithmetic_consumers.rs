use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, infer_type, recheck, Context, KernelError, Literal, Term,
};
use logicaffeine_proof::arith::prove_int_eq;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn lit(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn op(n: &str, x: Term, y: Term) -> Term {
    app(app(g(n), x), y)
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    for n in ["x", "y", "z"] {
        c.add_declaration(n, g("Int"));
    }
    c
}
fn certifies(c: &Context, l: Term, r: Term) {
    let p = prove_int_eq(c, &l, &r)
        .expect("existing polynomial identities must survive checked operator definitions");
    let expected = app(app(app(g("Eq"), g("Int")), l), r);
    assert!(defeq_for_test(c, &infer_type(c, &p).unwrap(), &expected));
    assert!(defeq_for_test(c, &recheck(c, &p).unwrap(), &expected));
}
#[test]
fn symbolic_zero_identity_uses_checked_addition() {
    let c = ctx();
    certifies(&c, op("add", g("x"), lit(0)), g("x"));
}
#[test]
fn symbolic_distributivity_survives_operator_delta_bodies() {
    let c = ctx();
    certifies(
        &c,
        op("mul", g("x"), op("add", g("y"), g("z"))),
        op("add", op("mul", g("x"), g("y")), op("mul", g("x"), g("z"))),
    );
}
#[test]
fn farkas_coefficient_cancellation_survives_checked_operators() {
    let c = ctx();
    certifies(
        &c,
        op("add", op("mul", lit(2), g("x")), op("mul", lit(-1), g("x"))),
        g("x"),
    );
}
#[test]
fn definitional_wrappers_expose_the_same_polynomial_vocabulary() {
    let mut c = ctx();
    let ty = c.get_global("add").unwrap().clone();
    c.declare(
        "wrapped_sum",
        ty,
        lam("a", g("Int"), lam("b", g("Int"), op("add", v("a"), v("b")))),
    )
    .unwrap();
    certifies(
        &c,
        op("wrapped_sum", g("x"), g("y")),
        op("add", g("y"), g("x")),
    );
}
/// The raw context cannot rebind an operator the integer laws rest on — the stable-meaning gate
/// refuses it — so the ring interpretation stays the operator's own. Where an operator NAME can be
/// rebound (a document's own `add`, under a fresh identity), the oracle never reads the rebinding as
/// ring arithmetic: `shadowing_surfaces::a_rebound_arithmetic_name_never_inherits_the_ring_interpretation`.
#[test]
fn rebound_operator_names_do_not_inherit_ring_interpretation() {
    for name in ["add", "mul"] {
        let mut c = ctx();
        let ty = c.get_global(name).unwrap().clone();
        assert!(
            matches!(
                c.declare(name, ty, lam("a", g("Int"), lam("b", g("Int"), v("a")))),
                Err(KernelError::StableMeaningViolation { name: refused, .. }) if refused == name
            ),
            "`{name}` keeps its meaning under the integer laws"
        );
        certifies(&c, op(name, g("x"), g("y")), op(name, g("y"), g("x")));
    }
}
/// A transitive dependency of the arithmetic (`Nat_add`, under `add`) cannot be rebound under the
/// attestation the oracle warmed on it: the gate refuses the change, and the warmed interpretation
/// is still the true one.
#[test]
fn changed_transitive_arithmetic_dependency_invalidates_oracle_interpretation() {
    let mut c = ctx();
    let _ = prove_int_eq(&c, &op("add", g("x"), lit(0)), &g("x"));
    let ty = c.get_global("Nat_add").unwrap().clone();
    assert!(
        matches!(
            c.declare("Nat_add", ty, lam("a", g("Nat"), lam("b", g("Nat"), v("a")))),
            Err(KernelError::StableMeaningViolation { name, .. }) if name == "Nat_add"
        ),
        "`Nat_add` keeps its meaning under `add`"
    );
    certifies(&c, op("add", g("x"), g("y")), op("add", g("y"), g("x")));
}
#[test]
fn literal_computation_and_false_polynomial_decisions_remain_distinct() {
    let c = ctx();
    certifies(&c, op("add", op("mul", lit(-7), lit(6)), lit(50)), lit(8));
    assert!(prove_int_eq(&c, &op("mul", g("x"), g("y")), &op("add", g("x"), g("y"))).is_none());
}
