use super::*;

struct Restore(usize, usize, bool, bool);
impl Drop for Restore {
    fn drop(&mut self) {
        WHNF_DEPTH.with(|x| x.set(self.0));
        WHNF_FUEL.with(|x| x.set(self.1));
        WHNF_EXHAUSTED.with(|x| x.set(self.2));
        OBSERVE.with(|x| x.set(self.3));
    }
}
fn budget(n: usize) -> Restore {
    Restore(
        WHNF_DEPTH.with(|x| x.replace(1)),
        WHNF_FUEL.with(|x| x.replace(n)),
        WHNF_EXHAUSTED.with(|x| x.replace(false)),
        OBSERVE.with(|x| x.replace(true)),
    )
}
fn fuel() -> usize {
    WHNF_FUEL.with(|x| x.get())
}
fn exhausted() -> bool {
    WHNF_EXHAUSTED.with(|x| x.get())
}
fn lit(n: u64) -> Db {
    Db::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}
fn nat() -> Term {
    Term::Global("Nat".into())
}
fn global_term(n: &str) -> Term {
    Term::Global(n.into())
}
fn var(n: &str) -> Term {
    Term::Var(n.into())
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn call(f: Term, args: &[Term]) -> Term {
    args.iter()
        .cloned()
        .fold(f, |f, a| Term::App(Box::new(f), Box::new(a)))
}
fn natural(n: u64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}
fn binary_type() -> Term {
    pi("a", nat(), pi("b", nat(), nat()))
}
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
fn branch(base: Term, successor: Term) -> Term {
    lam(
        "a",
        nat(),
        lam(
            "b",
            nat(),
            Term::Match {
                discriminant: Box::new(var("b")),
                motive: Box::new(lam("_", nat(), nat())),
                cases: vec![base, lam("k", nat(), successor)],
            },
        ),
    )
}
fn right_zero_context(name: &str) -> Context {
    let mut c = context();
    c.declare(name, binary_type(), branch(var("a"), global_term("Zero")))
        .unwrap();
    c
}
fn burning_term() -> Term {
    let walk = Term::Fix {
        name: "walk".into(),
        body: Box::new(lam(
            "n",
            nat(),
            lam(
                "acc",
                nat(),
                Term::Match {
                    discriminant: Box::new(var("n")),
                    motive: Box::new(lam("_", nat(), nat())),
                    cases: vec![
                        var("acc"),
                        lam("k", nat(), call(var("walk"), &[var("k"), var("acc")])),
                    ],
                },
            ),
        )),
    };
    lam("a", nat(), call(walk, &[natural(50_000), var("a")]))
}

#[test]
fn a_renamed_definition_needs_only_its_universal_right_zero_equation() {
    let c = right_zero_context("unrelated.name");
    let _scope = budget(20_000);
    assert_eq!(
        reduce(&c, &[], &binary("unrelated.name", lit(37), lit(0))),
        Some(lit(37))
    );
    assert!(
        !exhausted() && fuel() < 20_000,
        "proof work must be charged"
    );
    let plain = c.order_recurrence_context();
    assert!(def_eq(
        &plain,
        &[],
        &binary("unrelated.name", lit(37), lit(0)),
        &lit(37)
    ));
}

#[test]
fn an_arithmetic_spelling_does_not_prove_a_false_base_equation() {
    let mut c = context();
    c.declare(
        "Nat.shiftRight",
        binary_type(),
        branch(call(global_term("Succ"), &[var("a")]), global_term("Zero")),
    )
    .unwrap();
    let _scope = budget(20_000);
    assert_eq!(
        reduce(&c, &[], &binary("Nat.shiftRight", lit(2), lit(0))),
        None
    );
    assert!(!exhausted());
}

#[test]
fn an_expensive_successor_is_not_probed_for_an_admitted_zero_operand() {
    let mut c = context();
    c.declare("costly", pi("a", nat(), nat()), burning_term())
        .unwrap();
    c.declare(
        "base_then_costly",
        binary_type(),
        branch(var("a"), call(global_term("costly"), &[var("a")])),
    )
    .unwrap();
    {
        let _scope = budget(20_000);
        assert_eq!(
            reduce(&c, &[], &binary("base_then_costly", lit(2), lit(0))),
            Some(lit(2))
        );
        assert!(!exhausted());
    }
    let _scope = budget(20_000);
    assert_eq!(
        certify(&c, "base_then_costly", 0, &mut HashMap::new()),
        None
    );
    assert!(
        exhausted(),
        "the deliberately expensive full recurrence probe exhausts this allowance"
    );
}

#[test]
fn a_false_successor_cannot_become_a_full_or_nonzero_certificate() {
    let c = right_zero_context("partial");
    let _scope = budget(20_000);
    assert_eq!(
        reduce(&c, &[], &binary("partial", lit(2), lit(0))),
        Some(lit(2))
    );
    assert_eq!(reduce(&c, &[], &binary("partial", lit(2), lit(1))), None);
    let mut seen = HashMap::new();
    assert_eq!(certify(&c, "partial", 0, &mut seen), None);
    assert_eq!(seen.get("partial"), Some(&None));
}

#[test]
fn changed_dependencies_are_reproved_in_the_current_context() {
    let mut c = context();
    c.declare(
        "identity_dependency",
        pi("a", nat(), nat()),
        lam("a", nat(), var("a")),
    )
    .unwrap();
    c.declare(
        "dependent",
        binary_type(),
        branch(
            call(global_term("identity_dependency"), &[var("a")]),
            global_term("Zero"),
        ),
    )
    .unwrap();
    {
        let _scope = budget(20_000);
        assert_eq!(
            reduce(&c, &[], &binary("dependent", lit(5), lit(0))),
            Some(lit(5))
        );
    }
    let mut changed = c.clone();
    changed.permit_meaning_changes_for_cache_tests();
    changed
        .declare(
            "identity_dependency",
            pi("a", nat(), nat()),
            lam("a", nat(), call(global_term("Succ"), &[var("a")])),
        )
        .unwrap();
    let _scope = budget(20_000);
    assert_eq!(
        reduce(&changed, &[], &binary("dependent", lit(5), lit(0))),
        None
    );
    assert_eq!(
        reduce(&c, &[], &binary("dependent", lit(5), lit(0))),
        Some(lit(5))
    );
}

#[test]
fn zero_and_tiny_allowances_are_not_replenished() {
    let c = right_zero_context("limited");
    for allowance in [0, 1, 2] {
        let _scope = budget(allowance);
        assert_eq!(reduce(&c, &[], &binary("limited", lit(2), lit(0))), None);
        assert_eq!(fuel(), 0);
        assert!(exhausted());
    }
}

#[test]
fn preexisting_exhaustion_stays_sticky() {
    let c = right_zero_context("sticky");
    let _scope = budget(20_000);
    WHNF_EXHAUSTED.with(|x| x.set(true));
    assert_eq!(reduce(&c, &[], &binary("sticky", lit(2), lit(0))), None);
    assert!(exhausted());
    assert_eq!(fuel(), 20_000);
}

#[test]
fn exhausted_depth_is_not_bypassed() {
    let c = right_zero_context("deep");
    let _scope = budget(20_000);
    WHNF_DEPTH.with(|x| x.set(MAX_RECHECK_WHNF_DEPTH));
    assert_eq!(reduce(&c, &[], &binary("deep", lit(2), lit(0))), None);
    assert_eq!(fuel(), 20_000);
}

#[test]
fn observation_and_recurrence_guards_remain_required() {
    let c = right_zero_context("guarded");
    let _scope = budget(20_000);
    assert_eq!(
        reduce(
            &c.order_recurrence_context(),
            &[],
            &binary("guarded", lit(2), lit(0))
        ),
        None
    );
    OBSERVE.with(|x| x.set(false));
    assert_eq!(reduce(&c, &[], &binary("guarded", lit(2), lit(0))), None);
    assert_eq!(fuel(), 20_000);
}

#[test]
fn open_negative_and_unknown_operands_do_not_use_the_base_equation() {
    let mut c = right_zero_context("closed_only");
    c.add_declaration("unknown_nat", nat());
    for a in [
        Db::Var(0),
        global("unknown_nat"),
        Db::Lit(Literal::Nat(crate::BigInt::from_i64(-1))),
    ] {
        let _scope = budget(20_000);
        assert_eq!(
            reduce(&c, &[global("Nat")], &binary("closed_only", a, lit(0))),
            None
        );
    }
    let _scope = budget(20_000);
    assert_eq!(
        reduce(
            &c,
            &[global("Nat")],
            &binary("closed_only", lit(2), Db::Var(0))
        ),
        None
    );
}

#[test]
fn constructor_zero_and_large_identity_results_keep_exact_values() {
    let c = right_zero_context("wide_identity");
    let huge = crate::BigInt::parse_decimal("18446744073709551616").unwrap();
    let _scope = budget(20_000);
    assert_eq!(
        reduce(
            &c,
            &[],
            &binary(
                "wide_identity",
                Db::Lit(Literal::Nat(huge.clone())),
                global("Zero")
            )
        ),
        Some(Db::Lit(Literal::Nat(huge)))
    );
}

#[test]
fn a_structural_shift_right_at_two_zero_uses_the_base_equation() {
    let mut c = context();
    // Same defining recurrence as the imported shift: shift a 0 = a;
    // shift a (k+1) = (shift a k) / 2. The original brecOn export remains
    // an integration control; this fixture uses the independent Fix encoding.
    let body = Term::Fix {
        name: "shift".into(),
        body: Box::new(branch(
            var("a"),
            call(
                global_term("Nat_div"),
                &[call(var("shift"), &[var("a"), var("k")]), natural(2)],
            ),
        )),
    };
    c.declare("shift_fixture", binary_type(), body).unwrap();
    let _scope = budget(20_000);
    assert_eq!(
        reduce(&c, &[], &binary("shift_fixture", lit(2), lit(0))),
        Some(lit(2))
    );
    assert!(!exhausted());
}

#[test]
fn full_arithmetic_at_zero_and_nonzero_remains_exact() {
    let c = context();
    for (name, b, expected) in [
        ("Nat_add", 0, 7),
        ("Nat_add", 3, 10),
        ("Nat_mul", 0, 0),
        ("Nat_mul", 3, 21),
        ("Nat_pow", 0, 1),
        ("Nat_pow", 3, 343),
    ] {
        let _scope = budget(20_000);
        assert_eq!(
            reduce(&c, &[], &binary(name, lit(7), lit(b))),
            Some(lit(expected))
        );
        assert!(!exhausted());
    }
}

#[test]
fn agreement_only_at_the_concrete_operand_is_not_a_universal_certificate() {
    let mut c = context();
    c.declare(
        "constant_two",
        binary_type(),
        branch(natural(2), global_term("Zero")),
    )
    .unwrap();
    let _scope = budget(20_000);
    assert_eq!(
        reduce(&c, &[], &binary("constant_two", lit(2), lit(0))),
        None
    );
    assert!(!exhausted());
}

#[test]
fn repeated_base_observations_keep_spending_the_shared_allowance() {
    let c = right_zero_context("charged");
    let _scope = budget(20_000);
    assert_eq!(
        reduce(&c, &[], &binary("charged", lit(2), lit(0))),
        Some(lit(2))
    );
    let remaining = fuel();
    assert!(remaining < 20_000);
    assert_eq!(
        reduce(&c, &[], &binary("charged", lit(2), lit(0))),
        Some(lit(2))
    );
    assert!(fuel() < remaining);
    assert!(!exhausted());
}
