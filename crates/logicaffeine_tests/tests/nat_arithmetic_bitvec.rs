//! Checked arithmetic foundations for the original T10-010 BitVec obligation.
use logicaffeine_kernel::prelude::{axiom_dependencies, StandardLibrary};
use logicaffeine_kernel::{
    defeq_for_test, double_check, infer_type, normalize_checked, BigInt, Context, DoubleCheck,
    Literal, Term, Universe,
};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apps(n: &str, args: &[Term]) -> Term {
    args.iter().cloned().fold(g(n), app)
}
fn nat(n: u32) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_i64(n.into())))
}
fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn value(c: &Context, name: &str, a: u32, b: u32, expected: u32) {
    let t = apps(name, &[nat(a), nat(b)]);
    assert_eq!(infer_type(c, &t).unwrap(), g("Nat"));
    let reduced = normalize_checked(c, &t).unwrap();
    assert!(
        defeq_for_test(c, &reduced, &nat(expected)),
        "{name} {a} {b}: {reduced}"
    );
}
#[test]
fn natural_arithmetic_definitions_are_checked_and_axiom_free() {
    let c = ctx();
    for name in [
        "Nat_add",
        "Nat_sub",
        "Nat_mul",
        "Nat_pow",
        "Nat_divMod",
        "Nat_div",
        "Nat_mod",
        "BitVec",
    ] {
        let body = c
            .get_definition_body(name)
            .unwrap_or_else(|| panic!("missing {name}"));
        infer_type(&c, body).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(double_check(&c, body), DoubleCheck::Agreed, "{name}");
        assert!(
            axiom_dependencies(&c, body).axioms.is_empty()
                && axiom_dependencies(&c, body).unresolved.is_empty(),
            "{name} introduces axioms"
        );
    }
}
#[test]
fn natural_addition_multiplication_and_truncated_subtraction() {
    let c = ctx();
    for a in 0..8 {
        for b in 0..8 {
            value(&c, "Nat_add", a, b, a + b);
            value(&c, "Nat_mul", a, b, a * b);
            value(&c, "Nat_sub", a, b, a.saturating_sub(b));
        }
    }
}
#[test]
fn natural_power_handles_zero_base_and_zero_exponent() {
    let c = ctx();
    for a in 0_u32..5 {
        for b in 0..4 {
            value(&c, "Nat_pow", a, b, a.pow(b));
        }
    }
}
#[test]
fn natural_division_remainder_and_zero_divisor_match_lean() {
    let c = ctx();
    for a in 0..18 {
        for b in 0..7 {
            value(&c, "Nat_div", a, b, if b == 0 { 0 } else { a / b });
            value(&c, "Nat_mod", a, b, if b == 0 { a } else { a % b });
        }
    }
}
#[test]
fn natural_arithmetic_has_open_definitional_equations() {
    let c = ctx().extend("n", g("Nat"));
    assert!(defeq_for_test(
        &c,
        &apps("Nat_add", &[v("n"), nat(0)]),
        &v("n")
    ));
    assert!(defeq_for_test(
        &c,
        &apps("Nat_mul", &[v("n"), nat(0)]),
        &nat(0)
    ));
    assert!(defeq_for_test(
        &c,
        &apps("Nat_sub", &[v("n"), nat(0)]),
        &v("n")
    ));
    assert!(defeq_for_test(
        &c,
        &apps("Nat_pow", &[v("n"), nat(0)]),
        &nat(1)
    ));
}
#[test]
fn bitvec_carrier_is_fin_two_to_width_including_width_zero() {
    let c = ctx();
    for width in 0..9 {
        let t = app(g("BitVec"), nat(width));
        assert_eq!(infer_type(&c, &t).unwrap(), Term::Sort(Universe::Type(0)));
        assert!(defeq_for_test(&c, &t, &app(g("Fin"), nat(1 << width))));
    }
    assert!(!defeq_for_test(
        &c,
        &app(g("BitVec"), nat(8)),
        &app(g("BVec"), nat(8))
    ));
}
#[test]
fn bitvec_and_machine_word_carriers_agree_at_eight_bits() {
    let c = ctx();
    assert!(defeq_for_test(&c, &app(g("BitVec"), nat(8)), &g("UInt8")));
}
