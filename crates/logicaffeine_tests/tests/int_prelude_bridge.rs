//! Original T10-004/T11-016: actual Int eliminations through the standard prelude.
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies, defeq_for_test, double_check, infer_type, normalize_checked, recheck,
    BigInt, Context, DoubleCheck, KernelError, Literal, ReCheckError, Term, Universe,
};
fn g(s: &str) -> Term {
    Term::Global(s.into())
}
fn v(s: &str) -> Term {
    Term::Var(s.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apps(f: &str, xs: &[Term]) -> Term {
    xs.iter().cloned().fold(g(f), app)
}
fn lam(p: &str, ty: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.into(),
        param_type: Box::new(ty),
        body: Box::new(b),
    }
}
fn int(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn nat(n: i64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_i64(n)))
}
fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn equality_proof(c: &Context, a: Term, b: Term) {
    let expected = apps("Eq", &[g("Int"), a.clone(), b]);
    let proof = app(lam("h", expected, v("h")), apps("refl", &[g("Int"), a]));
    infer_type(c, &proof).unwrap();
    assert_eq!(double_check(c, &proof), DoubleCheck::Agreed);
}
fn absolute(t: Term) -> Term {
    Term::Match {
        discriminant: Box::new(t),
        motive: Box::new(lam("i", g("Int"), g("Nat"))),
        cases: vec![
            lam("n", g("Nat"), v("n")),
            lam("n", g("Nat"), app(g("Succ"), v("n"))),
        ],
    }
}
#[test]
fn standard_int_has_checked_constructors_and_axiom_free_structural_operations() {
    let c = ctx();
    assert_eq!(
        c.get_constructors("Int")
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>(),
        ["Int_ofNat", "Int_negSucc"]
    );
    for name in [
        "Int_negOfNat",
        "Int_subNatNat",
        "Int_neg",
        "Int_add",
        "Int_sub",
        "Int_mul",
    ] {
        let b = c
            .get_definition_body(name)
            .unwrap_or_else(|| panic!("missing {name}"));
        infer_type(&c, b).unwrap();
        assert_eq!(double_check(&c, b), DoubleCheck::Agreed, "{name}");
        let usage = axiom_dependencies(&c, b);
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{name}: {usage:?}"
        );
    }
}
#[test]
fn signed_integer_literals_and_constructors_convert_in_both_kernels() {
    let c = ctx();
    for n in [i64::MIN, -6, -1, 0, 1, 7, i64::MAX] {
        let constructor = logicaffeine_kernel::int_inductive::canonical_int(n);
        assert!(defeq_for_test(&c, &int(n), &constructor), "{n}");
        assert!(defeq_for_test(&c, &constructor, &int(n)), "{n}");
        equality_proof(&c, int(n), constructor);
    }
    assert!(!defeq_for_test(&c, &int(-1), &apps("Int_ofNat", &[nat(0)])));
    assert!(!defeq_for_test(
        &c,
        &int(0),
        &apps("Int_negSucc", &[nat(0)])
    ));
}
#[test]
fn integer_matches_extract_natural_fields_including_big_integer_boundaries() {
    let c = ctx();
    for spelling in [
        "0",
        "-1",
        "-9223372036854775808",
        "9223372036854775808",
        "-18446744073709551617",
    ] {
        let n = BigInt::parse_decimal(spelling).unwrap();
        let literal = Term::Lit(Literal::BigInt(n.clone()));
        let expression = absolute(literal);
        assert_eq!(infer_type(&c, &expression).unwrap(), g("Nat"));
        let result = normalize_checked(&c, &expression).unwrap();
        assert!(defeq_for_test(
            &c,
            &result,
            &Term::Lit(Literal::Nat(n.abs()))
        ));
        let proof_type = apps(
            "Eq",
            &[
                g("Nat"),
                expression.clone(),
                Term::Lit(Literal::Nat(n.abs())),
            ],
        );
        let proof = app(
            lam("h", proof_type, v("h")),
            apps("refl", &[g("Nat"), expression]),
        );
        assert_eq!(double_check(&c, &proof), DoubleCheck::Agreed, "{spelling}");
    }
}
#[test]
fn checked_structural_integer_arithmetic_agrees_with_literal_primitives() {
    let c = ctx();
    for a in -3..=3 {
        for b in -3..=3 {
            for (structural, primitive) in
                [("Int_add", "add"), ("Int_sub", "sub"), ("Int_mul", "mul")]
            {
                let expression = apps(structural, &[int(a), int(b)]);
                let accelerated =
                    normalize_checked(&c, &apps(primitive, &[int(a), int(b)])).unwrap();
                assert!(
                    defeq_for_test(&c, &expression, &accelerated),
                    "{structural} {a} {b}"
                );
                equality_proof(&c, expression, accelerated);
            }
        }
    }
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "_".into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
#[test]
fn integer_bridge_requires_registered_canonical_constructor_signatures() {
    // In the standard environment everything built over `Int` relies on its canonical
    // constructors, so re-signing one is refused, and the bridge keeps standing on them.
    let mut c = ctx();
    c.add_declaration("Int_ofNat", g("Nat"));
    assert!(
        c.refused_declarations()
            .iter()
            .any(|(n, why)| n == "Int_ofNat" && why.contains("Stable meaning")),
        "{:?}",
        c.refused_declarations()
    );
    assert_eq!(
        c.get_constructors("Int")
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>(),
        ["Int_ofNat", "Int_negSucc"]
    );
    assert!(defeq_for_test(&c, &int(3), &apps("Int_ofNat", &[nat(3)])));
    // Where nothing relies on it, `Int_ofNat` may be something other than the canonical
    // constructor — and then no literal bridges to it.
    let mut off = Context::new();
    off.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    off.add_constructor("Zero", "Nat", g("Nat"));
    off.add_constructor("Succ", "Nat", arrow(g("Nat"), g("Nat")));
    off.add_inductive("Int", Term::Sort(Universe::Type(0)));
    off.add_constructor("Int_negSucc", "Int", arrow(g("Nat"), g("Int")));
    off.add_declaration("Int_ofNat", g("Nat"));
    assert!(
        off.refused_declarations().is_empty(),
        "{:?}",
        off.refused_declarations()
    );
    let term = absolute(int(3));
    assert_eq!(normalize_checked(&off, &term).unwrap(), term);
    assert!(!defeq_for_test(
        &off,
        &int(3),
        &apps("Int_ofNat", &[nat(3)])
    ));
    let mut wrong_sort = Context::new();
    wrong_sort.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    wrong_sort.add_inductive("Int", Term::Sort(Universe::Prop));
    assert_eq!(
        normalize_checked(&wrong_sort, &absolute(int(0))).unwrap(),
        absolute(int(0))
    );
}
/// An `Int` literal is a value of a two-constructor inductive, so a `match` on it with zero or
/// one case is not `ex falso` and not exhaustive: BOTH kernels must reject it outright — the
/// re-checker with a real verdict, not a decline — and `double_check` then reports the two
/// rejections as agreement (its contract: `Agreed` is two acceptances at equal types or two
/// rejections, and acceptance is read off the inference result).
#[test]
fn integer_bridge_keeps_empty_and_incomplete_matches_invalid() {
    let c = ctx();
    for cases in [vec![], vec![lam("n", g("Nat"), v("n"))]] {
        let expression = Term::Match {
            discriminant: Box::new(int(0)),
            motive: Box::new(lam("i", g("Int"), g("Nat"))),
            cases,
        };
        assert!(
            matches!(
                infer_type(&c, &expression),
                Err(KernelError::WrongNumberOfCases { expected: 2, .. })
            ),
            "the main kernel refuses a match that does not cover `Int`'s two constructors"
        );
        assert!(
            matches!(recheck(&c, &expression), Err(ReCheckError::Ill(_))),
            "the re-checker refuses it too, on its own grounds: {:?}",
            recheck(&c, &expression)
        );
        assert_eq!(
            double_check(&c, &expression),
            DoubleCheck::Agreed,
            "two rejections are agreement — and the rejections above are what make it a refusal"
        );
    }
}

#[test]
fn integer_primitive_acceleration_accepts_canonical_constructor_operands() {
    let c = ctx();
    for a in -2..=2 {
        for b in -2..=2 {
            for name in ["add", "sub", "mul", "div", "mod"] {
                let expression = apps(
                    name,
                    &[
                        logicaffeine_kernel::int_inductive::canonical_int(a),
                        logicaffeine_kernel::int_inductive::canonical_int(b),
                    ],
                );
                let expected = normalize_checked(&c, &apps(name, &[int(a), int(b)])).unwrap();
                assert!(defeq_for_test(&c, &expression, &expected), "{name} {a} {b}");
                equality_proof(&c, expression, expected);
            }
        }
    }
    let with_peano = apps(
        "add",
        &[apps("Int_ofNat", &[app(g("Succ"), g("Zero"))]), int(-1)],
    );
    equality_proof(&c, with_peano, int(0));
}
