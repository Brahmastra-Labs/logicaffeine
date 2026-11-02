//! What the evaluator certifies a program computes — the table Lean's `reduce_nat` reads in the
//! lazy loop — is established once per registry generation, refusals included, and names grant
//! nothing: each answer is a certificate the evaluator proved.
use super::*;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn lam(n: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "_".into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn nat() -> Term {
    g("Nat")
}

fn standard() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    // `first a b := a`: a binary program on naturals that is no arithmetic operation.
    c.declare(
        "first",
        arrow(nat(), arrow(nat(), nat())),
        lam("a", nat(), lam("b", nat(), v("a"))),
    )
    .unwrap();
    c
}

fn certifications() -> usize {
    PROGRAM_CERTIFICATIONS.with(|n| n.get())
}

#[test]
fn the_prelude_arithmetic_is_certified_by_its_recurrences() {
    let c = standard();
    assert_eq!(
        certified_program(&c, "Nat_add"),
        Some(CertifiedProgram::Arithmetic(Arithmetic::Add))
    );
    assert_eq!(
        certified_program(&c, "Nat_mul"),
        Some(CertifiedProgram::Arithmetic(Arithmetic::Mul))
    );
    assert_eq!(
        certified_program(&c, "first"),
        None,
        "a binary program that computes no operation"
    );
    assert_eq!(
        certified_program(&c, "Nat"),
        None,
        "an inductive type is no program"
    );
}

#[test]
fn a_conclusion_is_reached_once_per_registry_generation_refusals_included() {
    let mut c = standard();
    let before = certifications();
    assert_eq!(certified_program(&c, "first"), None);
    assert_eq!(
        certified_program(&c, "Nat_add"),
        Some(CertifiedProgram::Arithmetic(Arithmetic::Add))
    );
    let settled = certifications();
    assert_eq!(settled, before + 2, "each program is certified once");
    assert_eq!(certified_program(&c, "first"), None);
    assert_eq!(
        certified_program(&c, "Nat_add"),
        Some(CertifiedProgram::Arithmetic(Arithmetic::Add))
    );
    assert_eq!(
        certifications(),
        settled,
        "a refusal and a certificate are both reused while the registries stand"
    );
    c.declare("unrelated", nat(), g("Zero")).unwrap();
    assert_eq!(
        certified_program(&c, "Nat_add"),
        Some(CertifiedProgram::Arithmetic(Arithmetic::Add))
    );
    assert_eq!(
        certifications(),
        settled + 1,
        "a registry that moved certifies again"
    );
}

#[test]
fn a_certified_application_computes_what_the_recurrence_computes() {
    let big = |n: &str| crate::BigInt::parse_decimal(n).unwrap();
    let lit = |n: &str| Term::Lit(Literal::Nat(big(n)));
    let add = CertifiedProgram::Arithmetic(Arithmetic::Add);
    assert_eq!(
        certified_application(&add, &big("18446744073709551615"), &big("1")),
        Some(lit("18446744073709551616"))
    );
    let sub = CertifiedProgram::Arithmetic(Arithmetic::Sub);
    assert_eq!(
        certified_application(&sub, &big("3"), &big("5")),
        Some(lit("0")),
        "truncated"
    );
    let order = CertifiedProgram::Order {
        no: "no".into(),
        yes: "yes".into(),
    };
    assert_eq!(
        certified_application(&order, &big("5"), &big("5")),
        Some(g("yes"))
    );
    assert_eq!(
        certified_application(&order, &big("6"), &big("5")),
        Some(g("no"))
    );
    let equality = CertifiedProgram::Equality {
        no: "no".into(),
        yes: "yes".into(),
    };
    assert_eq!(
        certified_application(&equality, &big("5"), &big("6")),
        Some(g("no"))
    );
    let modulus = CertifiedProgram::Division {
        op: Arithmetic::Mod,
        pair: "mk".into(),
        parameters: 0,
    };
    assert_eq!(
        certified_application(&modulus, &big("17"), &big("0")),
        Some(lit("17")),
        "Lean's n % 0 = n"
    );
    let quotient = CertifiedProgram::Division {
        op: Arithmetic::Div,
        pair: "mk".into(),
        parameters: 0,
    };
    assert_eq!(
        certified_application(&quotient, &big("17"), &big("0")),
        Some(lit("0")),
        "Lean's n / 0 = 0"
    );
    assert_eq!(
        certified_application(
            &quotient,
            &big("340282366920938463463374607431768211456"),
            &big("18446744073709551616")
        ),
        Some(lit("18446744073709551616"))
    );
}
