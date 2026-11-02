use super::*;
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
fn lam(n: &str, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(b),
    }
}
fn certificate(c: &Context, name: &str) -> Option<Arithmetic> {
    let mut r = HeadReducer::new(c, 10_000);
    let id = r.store.intern(&g(name));
    r.certified_arithmetic(id)
}
#[test]
fn all_three_equations_are_recognized_without_name_authority() {
    let mut c = context();
    for (old, new, kind) in [
        ("Nat_add", "unfamiliar.a", Arithmetic::Add),
        ("Nat_mul", "unfamiliar.b", Arithmetic::Mul),
        ("Nat_pow", "unfamiliar.c", Arithmetic::Pow),
    ] {
        c.declare(
            new,
            c.get_definition_type(old).unwrap().clone(),
            c.get_definition_body(old).unwrap().clone(),
        )
        .unwrap();
        assert_eq!(certificate(&c, new), Some(kind));
    }
    assert!(constructor_equivalent(
        &c,
        &call(&g("unfamiliar.c"), lit(2), lit(64)),
        &Term::Lit(Literal::Nat(
            crate::BigInt::parse_decimal("18446744073709551616").unwrap()
        )),
        1000
    ));
}
#[test]
fn both_kinds_of_operand_and_small_ordinary_reduction_agree() {
    let c = context();
    let ordinary = c.order_recurrence_context();
    for name in ["Nat_add", "Nat_mul", "Nat_pow"] {
        for a in 0..4 {
            for b in 0..4 {
                let t = call(&g(name), lit(a), lit(b));
                let normal = crate::normalize_checked(&ordinary, &t).unwrap();
                assert!(
                    constructor_equivalent(&c, &t, &normal, 1000),
                    "{name} {a} {b}"
                );
            }
        }
    }
    let unary = app(g("Succ"), app(g("Succ"), g("Zero")));
    assert!(constructor_equivalent(
        &c,
        &call(&g("Nat_pow"), unary, lit(32)),
        &lit(4_294_967_296),
        1000
    ));
}
#[test]
fn changing_a_dependency_cannot_reuse_a_certificate_from_a_clone() {
    let c = context();
    assert_eq!(certificate(&c, "Nat_pow"), Some(Arithmetic::Pow));
    let mut changed = c.clone();
    let ty = changed.get_definition_type("Nat_mul").unwrap().clone();
    let kept = changed
        .refused_meaning_change_for_test(|c| {
            c.declare("Nat_mul", ty.clone(), lam("x", lam("y", g("Zero"))))
        })
        .expect("the gate keeps `Nat_mul` under its dependents");
    assert_eq!(
        certificate(&kept, "Nat_pow"),
        Some(Arithmetic::Pow),
        "a refused change leaves the certificate valid"
    );
    changed.permit_meaning_changes_for_cache_tests();
    changed
        .declare("Nat_mul", ty, lam("x", lam("y", g("Zero"))))
        .unwrap();
    assert_eq!(certificate(&changed, "Nat_pow"), None);
    assert!(!constructor_equivalent(
        &changed,
        &call(&g("Nat_pow"), lit(2), lit(32)),
        &lit(4_294_967_296),
        1000
    ));
    assert_eq!(certificate(&c, "Nat_pow"), Some(Arithmetic::Pow));
}
#[test]
fn matching_zero_case_alone_is_not_a_certificate() {
    let mut c = context();
    let ty = c.get_definition_type("Nat_pow").unwrap().clone();
    c.declare(
        "pretend.power",
        ty,
        lam("x", lam("y", app(g("Succ"), g("Zero")))),
    )
    .unwrap();
    assert_eq!(certificate(&c, "pretend.power"), None);
    assert!(!constructor_equivalent(
        &c,
        &call(&g("pretend.power"), lit(2), lit(32)),
        &lit(4_294_967_296),
        1000
    ));
}
#[test]
fn arithmetic_work_is_bounded_before_allocation_and_handles_zero_power() {
    let c = context();
    let mut r = HeadReducer::new(&c, 1000);
    let zero = crate::BigInt::zero();
    let one = crate::BigInt::from_u64(1);
    let two = crate::BigInt::from_u64(2);
    let huge = two.pow(128);
    assert_eq!(
        r.arithmetic_result(Arithmetic::Pow, &zero, &zero),
        Some(one.clone())
    );
    assert_eq!(
        r.arithmetic_result(Arithmetic::Pow, &zero, &huge),
        Some(zero)
    );
    assert_eq!(r.arithmetic_result(Arithmetic::Pow, &one, &huge), Some(one));
    assert_eq!(r.arithmetic_result(Arithmetic::Pow, &two, &huge), None);
    assert_eq!(
        r.arithmetic_result(
            Arithmetic::Pow,
            &two,
            &crate::BigInt::from_u64(1_000_000_000)
        ),
        None
    );
    let mut r = HeadReducer::new(&c, 0);
    assert_eq!(r.arithmetic_result(Arithmetic::Mul, &two, &two), None);
    assert_eq!(
        r.arithmetic_result(Arithmetic::Add, &two, &crate::BigInt::from_i64(-1)),
        None
    );
}
#[test]
fn recurrence_probes_and_exact_readback_cannot_use_the_shortcut() {
    let c = context();
    let probe = c.order_recurrence_context();
    assert_eq!(certificate(&probe, "Nat_pow"), None);
    let mut r = HeadReducer::new(&c, 1000);
    r.preserve_nat_syntax = true;
    let id = r.store.intern(&g("Nat_pow"));
    assert_eq!(r.certified_arithmetic(id), None);
}
