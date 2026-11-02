use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies, axiom_dependencies_of, defeq_for_test, eval_bool_tree, infer_type, int_lit,
    native_compile_decide, normalize_checked, normalize_checked_at, recheck, BigInt, Context,
    Literal, Term, Transparency,
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
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().cloned().fold(f, app)
}
fn op(n: &str, a: Term, b: Term) -> Term {
    apps(g(n), &[a, b])
}
fn int(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn pi(n: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn lam(n: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn binary_type(carrier: &str, result: Term) -> Term {
    pi("_", g(carrier), pi("_", g(carrier), result))
}
fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}

/// A rebinding of a name the prelude's integer laws rest on is refused by the stable-meaning
/// gate, typed, naming what depends on it. (The attestation caches that must notice such a
/// change are tested with the change permitted in the kernel's own unit tests,
/// `int_inductive::checked_*_attestation_controls`.)
fn refused_as_a_change_of_meaning(
    result: Result<(), logicaffeine_kernel::KernelError>,
    name: &str,
) {
    match result {
        Err(logicaffeine_kernel::KernelError::StableMeaningViolation {
            name: refused,
            dependents,
            ..
        }) => {
            assert_eq!(refused, name);
            assert!(!dependents.is_empty());
        }
        other => panic!("rebinding `{name}` under its dependents must be refused: {other:?}"),
    }
}
fn checked(c: &Context, term: &Term, ty: &Term) {
    assert!(defeq_for_test(c, &infer_type(c, term).unwrap(), ty));
    assert!(defeq_for_test(c, &recheck(c, term).unwrap(), ty));
}
fn actual(name: &str, ty: Term) {
    let c = ctx();
    let body = c
        .get_definition_body(name)
        .expect("actual checked division body");
    assert_ne!(body, &g(name));
    assert!(defeq_for_test(&c, c.get_global(name).unwrap(), &ty));
    checked(&c, body, &ty);
    for usage in [
        axiom_dependencies(&c, body),
        axiom_dependencies_of(&c, name).unwrap(),
    ] {
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{name}: {usage:?}"
        );
    }
}
fn all_paths(c: &Context, expr: Term, want: Term) {
    let got = normalize_checked(c, &expr)
        .expect("ground arithmetic stays within existing normalization budget");
    assert!(
        defeq_for_test(c, &got, &want),
        "{expr} -> {got}; expected {want}"
    );
    let eq = apps(g("Eq"), &[g("Int"), expr.clone(), want.clone()]);
    checked(
        c,
        &app(
            lam("h", eq, g("true")),
            apps(g("refl"), &[g("Int"), want.clone()]),
        ),
        &g("Bool"),
    );
    for bool_expr in [op("le", expr.clone(), want.clone()), op("le", want, expr)] {
        assert_eq!(eval_bool_tree(c, &bool_expr), Some(true));
        assert_eq!(native_compile_decide(c, &bool_expr), Some(true));
    }
}
#[test]
fn integer_division_pair_has_a_checked_closed_structural_body() {
    actual(
        "Int_divMod",
        binary_type("Int", apps(g("Prod"), &[g("Int"), g("Int")])),
    );
}
#[test]
fn integer_quotient_has_a_checked_closed_structural_body() {
    actual("Int_div", binary_type("Int", g("Int")));
}
#[test]
fn integer_remainder_has_a_checked_closed_structural_body() {
    actual("Int_mod", binary_type("Int", g("Int")));
}
#[test]
fn public_division_and_remainder_are_exact_checked_aliases() {
    let c = ctx();
    for (name, target) in [("div", "Int_div"), ("mod", "Int_mod")] {
        assert_eq!(c.get_definition_body(name), Some(&g(target)));
        assert_eq!(c.get_global(name), Some(&binary_type("Int", g("Int"))));
        checked(&c, &g(target), &binary_type("Int", g("Int")));
        let usage = axiom_dependencies_of(&c, name).unwrap();
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{name}: {usage:?}"
        );
    }
}
#[test]
fn checked_division_preserves_euclidean_signs_and_total_zero_on_all_four_paths() {
    let c = ctx();
    assert!(c.is_definition("div") && c.is_definition("mod"));
    for (a, b, q, r) in [
        (7, 3, 2, 1),
        (-7, 3, -3, 2),
        (7, -3, -2, 1),
        (-7, -3, 3, 2),
        (-6, 3, -2, 0),
        (-6, -3, 2, 0),
        (0, 3, 0, 0),
        (0, -3, 0, 0),
        (7, 0, 0, 7),
        (-7, 0, 0, -7),
        (0, 0, 0, 0),
        (-1, 2, -1, 1),
        (1, -2, 0, 1),
    ] {
        for (name, want) in [("div", q), ("mod", r)] {
            all_paths(
                &c,
                op(
                    name,
                    logicaffeine_kernel::int_inductive::canonical_int(a),
                    logicaffeine_kernel::int_inductive::canonical_int(b),
                ),
                int(want),
            );
        }
    }
}
#[test]
fn the_structural_division_body_computes_small_cases_without_public_literal_rules() {
    let c = ctx();
    assert!(c.is_definition("Int_divMod"));
    for a in -4i64..=4 {
        for b in -3i64..=3 {
            let (q, r) = if b == 0 {
                (0, a)
            } else {
                (a.div_euclid(b), a.rem_euclid(b))
            };
            for (name, want) in [("Int_div", q), ("Int_mod", r)] {
                let expr = op(
                    name,
                    logicaffeine_kernel::int_inductive::canonical_int(a),
                    logicaffeine_kernel::int_inductive::canonical_int(b),
                );
                let got = normalize_checked(&c, &expr).unwrap();
                assert!(
                    defeq_for_test(&c, &got, &int(want)),
                    "{a} {name} {b}: {got}"
                );
                checked(&c, &got, &g("Int"));
            }
        }
    }
}
#[test]
fn exact_literal_division_handles_machine_overflow_and_arbitrary_precision() {
    let c = ctx();
    assert!(c.is_definition("div") && c.is_definition("mod"));
    all_paths(
        &c,
        op("div", int(i64::MIN), int(-1)),
        Term::Lit(int_lit(
            BigInt::parse_decimal("9223372036854775808").unwrap(),
        )),
    );
    all_paths(&c, op("mod", int(i64::MIN), int(-1)), int(0));
    let h = BigInt::parse_decimal("1180591620717411303449").unwrap();
    let magnitude = h.mul(&BigInt::from_i64(3)).add(&BigInt::from_i64(1));
    for b in [3, -3] {
        let q = if b > 0 {
            h.add(&BigInt::from_i64(1)).negated()
        } else {
            h.add(&BigInt::from_i64(1))
        };
        all_paths(
            &c,
            op("div", Term::Lit(int_lit(magnitude.negated())), int(b)),
            Term::Lit(int_lit(q)),
        );
        all_paths(
            &c,
            op("mod", Term::Lit(int_lit(magnitude.negated())), int(b)),
            int(2),
        );
    }
    for a in [
        h.clone(),
        h.negated(),
        BigInt::from_i64(i64::MIN),
        BigInt::from_i64(i64::MAX),
    ] {
        let x = Term::Lit(int_lit(a));
        all_paths(&c, op("div", x.clone(), int(0)), int(0));
        all_paths(&c, op("mod", x.clone(), int(0)), x);
    }
}
#[test]
fn rebinding_a_natural_division_program_is_refused_and_division_is_unchanged() {
    let mut parent = ctx();
    assert!(parent.is_definition("div"));
    all_paths(&parent, op("div", int(7), int(3)), int(2));
    all_paths(&parent, op("mod", int(7), int(3)), int(1));
    let pair_ty = apps(g("Prod"), &[g("Nat"), g("Nat")]);
    let pair = |q, r| apps(g("Prod_mk"), &[g("Nat"), g("Nat"), nat(q), nat(r)]);
    let mut child = parent.clone();
    refused_as_a_change_of_meaning(
        child.declare(
            "Nat_divMod",
            binary_type("Nat", pair_ty.clone()),
            lam("n", g("Nat"), lam("d", g("Nat"), pair(4, 0))),
        ),
        "Nat_divMod",
    );
    all_paths(&child, op("div", int(7), int(3)), int(2));
    all_paths(&child, op("mod", int(7), int(3)), int(1));
    all_paths(&parent, op("div", int(7), int(3)), int(2));
    drop(child);
    refused_as_a_change_of_meaning(
        parent.declare(
            "Nat_divModSucc",
            binary_type("Nat", pair_ty),
            lam("n", g("Nat"), lam("d", g("Nat"), pair(1, 0))),
        ),
        "Nat_divModSucc",
    );
    all_paths(&parent, op("div", int(-7), int(3)), int(-3));
    all_paths(&parent, op("mod", int(-7), int(3)), int(2));
}
#[test]
fn a_structural_division_rebinding_is_refused_and_a_public_one_disables_native_meaning() {
    let mut c = ctx();
    assert!(c.is_definition("div"));
    all_paths(&c, op("div", int(7), int(3)), int(2));
    refused_as_a_change_of_meaning(
        c.declare(
            "Int_div",
            binary_type("Int", g("Int")),
            lam("a", g("Int"), lam("b", g("Int"), v("b"))),
        ),
        "Int_div",
    );
    all_paths(&c, op("div", int(7), int(3)), int(2));
    // Nothing depends on the public `mod`, so it may still be rebound, and the rebinding
    // disables its native meaning.
    c.declare(
        "mod",
        binary_type("Int", g("Int")),
        lam("a", g("Int"), lam("b", g("Int"), v("b"))),
    )
    .unwrap();
    all_paths(&c, op("mod", int(7), int(3)), int(3));
}
#[test]
fn checked_division_keeps_unfolding_boundaries_visible() {
    let c = ctx();
    assert!(c.is_definition("div") && c.is_definition("mod"));
    for (operation, blocked) in [
        ("div", "div"),
        ("div", "Int_div"),
        ("div", "Int_divMod"),
        ("div", "Prod_fst"),
        ("div", "Nat_divMod"),
        ("div", "Nat_divModSucc"),
        ("div", "natLt"),
        ("div", "natLe"),
        ("mod", "mod"),
        ("mod", "Int_mod"),
        ("mod", "Prod_snd"),
        ("mod", "Nat_sub"),
        ("mod", "Nat_pred"),
    ] {
        let exclude = |n: &str| n == blocked;
        let expr = op(operation, int(-7), int(3));
        let reduced = normalize_checked_at(&c, &expr, Transparency::Excluding(&exclude)).unwrap();
        assert!(
            reduced.to_string().contains(blocked),
            "{operation} blocks {blocked}: {reduced}"
        );
    }
}
#[test]
fn wrong_carriers_false_equations_and_negative_naturals_cannot_enter_checked_division() {
    let c = ctx();
    for name in ["div", "mod"] {
        for (a, b) in [
            (nat(7), int(3)),
            (int(7), nat(3)),
            (
                app(
                    g("Int_ofNat"),
                    Term::Lit(Literal::Nat(BigInt::from_i64(-1))),
                ),
                int(3),
            ),
        ] {
            let bad = op(name, a, b);
            assert!(infer_type(&c, &bad).is_err());
            assert!(recheck(&c, &bad).is_err());
        }
    }
    let wrong = apps(g("Eq"), &[g("Int"), op("div", int(-7), int(3)), int(-2)]);
    let proof = apps(g("refl"), &[g("Int"), int(-2)]);
    let consume = app(lam("h", wrong, g("true")), proof);
    assert!(infer_type(&c, &consume).is_err());
    assert!(recheck(&c, &consume).is_err());
}
#[test]
fn closed_downstream_division_functions_have_no_assumption_dependencies() {
    let mut c = ctx();
    for name in ["div", "mod"] {
        let proof = lam("a", g("Int"), lam("b", g("Int"), op(name, v("a"), v("b"))));
        let ty = binary_type("Int", g("Int"));
        checked(&c, &proof, &ty);
        let alias = format!("closed_checked_{name}");
        c.declare(&alias, ty, proof).unwrap();
        let usage = axiom_dependencies_of(&c, &alias).unwrap();
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{usage:?}"
        );
    }
}

#[test]
fn constructed_large_euclidean_pairs_recover_independently_known_quotients_and_remainders() {
    let c = ctx();
    assert!(c.is_definition("div") && c.is_definition("mod"));
    let huge = BigInt::parse_decimal("1180591620717411303449").unwrap();
    for q in [huge.clone(), huge.negated(), BigInt::zero()] {
        for b in [
            BigInt::from_i64(3),
            BigInt::from_i64(-3),
            huge.clone(),
            huge.negated(),
        ] {
            for r in [
                BigInt::zero(),
                BigInt::from_i64(1),
                b.abs().sub(&BigInt::from_i64(1)),
            ] {
                let a = b.mul(&q).add(&r);
                let x = Term::Lit(int_lit(a));
                let y = Term::Lit(int_lit(b.clone()));
                all_paths(
                    &c,
                    op("div", x.clone(), y.clone()),
                    Term::Lit(int_lit(q.clone())),
                );
                all_paths(&c, op("mod", x, y), Term::Lit(int_lit(r)));
            }
        }
    }
}
