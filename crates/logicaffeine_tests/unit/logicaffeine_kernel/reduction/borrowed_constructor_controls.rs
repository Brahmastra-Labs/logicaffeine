use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn ap(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn nat_context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(crate::Universe::Type(0)));
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
fn count() -> usize {
    CONSTRUCTOR_MATERIALIZATIONS.with(|n| n.get())
}
fn reset() {
    CONSTRUCTOR_MATERIALIZATIONS.with(|n| n.set(0));
}
#[test]
fn named_recognition_never_materializes_discarded_fields() {
    let c = nat_context();
    let t = ap(g("Succ"), ap(g("large"), g(&"x".repeat(2000))));
    reset();
    assert!(is_constructor_form(&c, &t));
    assert_eq!(count(), 0);
    assert_eq!(
        extract_constructor(&c, &t),
        Some((1, vec![ap(g("large"), g(&"x".repeat(2000)))]))
    );
}
#[test]
fn natural_recognition_never_constructs_a_predecessor() {
    let c = nat_context();
    for n in [-1, 0, 1, 1000000] {
        let t = Term::Lit(Literal::Nat(crate::BigInt::from_i64(n)));
        reset();
        assert!(is_constructor_form(&c, &t));
        assert_eq!(count(), 0);
        let (index, fields) = extract_constructor(&c, &t).unwrap();
        assert_eq!(index, usize::from(n > 0));
        assert_eq!(fields.len(), usize::from(n > 0));
        if n > 0 {
            assert_eq!(
                fields[0],
                Term::Lit(Literal::Nat(crate::BigInt::from_i64(n - 1)))
            );
        }
    }
}
#[test]
fn partial_const_and_nonconstructor_spines_preserve_existing_decisions() {
    let c = nat_context();
    let cases = [
        g("Zero"),
        g("Succ"),
        Term::Const {
            name: "Succ".into(),
            levels: vec![crate::Universe::Type(2)],
        },
        ap(g("Zero"), g("extra")),
        ap(g("unknown"), g("payload")),
        Term::Var("Succ".into()),
        ap(
            Term::Lit(Literal::Nat(crate::BigInt::from_i64(0))),
            g("extra"),
        ),
        Term::Hole,
    ];
    for (t, expected) in cases
        .into_iter()
        .zip([true, true, true, true, false, false, false, false])
    {
        assert_eq!(is_constructor_form(&c, &t), expected);
        assert_eq!(extract_constructor(&c, &t).is_some(), expected);
    }
    assert!(!is_constructor_form(
        &Context::new(),
        &Term::Lit(Literal::Nat(crate::BigInt::from_i64(0)))
    ));
}
#[test]
fn shared_classifier_preserves_registration_order_and_parameter_erasure() {
    let mut c = Context::new();
    c.add_inductive("Box", Term::Sort(crate::Universe::Type(0)));
    c.add_constructor("First", "Box", g("Box"));
    c.add_constructor(
        "Pack",
        "Box",
        Term::Pi {
            param: "A".into(),
            param_type: Box::new(Term::Sort(crate::Universe::Type(0))),
            body_type: Box::new(g("Box")),
        },
    );
    let t = ap(ap(g("Pack"), g("TypeArg")), g("field"));
    assert_eq!(extract_constructor(&c, &t), Some((1, vec![g("field")])));
    c.set_inductive_params("Box", 2);
    assert_eq!(extract_constructor(&c, &t), Some((1, vec![])));
    reset();
    assert!(is_constructor_form(&c, &t));
    assert_eq!(count(), 0);
}
#[test]
fn integer_literal_bridge_keeps_its_canonical_environment_gate() {
    let mut c = nat_context();
    c.add_inductive("Int", Term::Sort(crate::Universe::Type(0)));
    crate::int_inductive::register_int_constructors(&mut c).unwrap();
    for n in [i64::MIN, -1, 0, i64::MAX] {
        let t = Term::Lit(Literal::Int(n));
        assert_eq!(
            is_constructor_form(&c, &t),
            extract_constructor(&c, &t).is_some()
        );
        assert!(is_constructor_form(&c, &t));
    }
    c.add_constructor("extra", "Int", g("Int"));
    assert!(!is_constructor_form(&c, &Term::Lit(Literal::Int(1))));
}
