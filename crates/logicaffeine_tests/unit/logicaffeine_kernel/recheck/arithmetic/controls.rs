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
fn lit(n: u64) -> Db {
    Db::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
#[test]
fn names_and_context_clones_are_not_certificates() {
    let c = context();
    let mut changed = c.clone();
    let ty = c.get_definition_type("Nat_pow").unwrap().clone();
    changed
        .declare(
            "different.power",
            ty,
            c.get_definition_body("Nat_pow").unwrap().clone(),
        )
        .unwrap();
    let _scope = budget(20_000);
    assert_eq!(
        reduce(&changed, &[], &binary("different.power", lit(2), lit(32))),
        Some(lit(4_294_967_296))
    );
    let ty = changed.get_definition_type("Nat_mul").unwrap().clone();
    let zero = Term::Lambda {
        param: "a".into(),
        param_type: Box::new(Term::Global("Nat".into())),
        body: Box::new(Term::Lambda {
            param: "b".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body: Box::new(Term::Global("Zero".into())),
        }),
    };
    let kept = changed
        .refused_meaning_change_for_test(|c| c.declare("Nat_mul", ty.clone(), zero.clone()))
        .expect("the gate keeps `Nat_mul` under its dependents");
    assert_eq!(
        reduce(&kept, &[], &binary("different.power", lit(2), lit(32))),
        Some(lit(4_294_967_296))
    );
    changed.permit_meaning_changes_for_cache_tests();
    changed.declare("Nat_mul", ty, zero).unwrap();
    assert_eq!(
        reduce(&changed, &[], &binary("different.power", lit(2), lit(32))),
        None
    );
    assert_eq!(
        reduce(&c, &[], &binary("Nat_pow", lit(2), lit(32))),
        Some(lit(4_294_967_296))
    );
}
#[test]
fn ordinary_small_naturals_and_constructor_operands_agree() {
    let c = context();
    for name in ["Nat_add", "Nat_mul", "Nat_pow"] {
        for a in 0..4 {
            for b in 0..4 {
                let _scope = budget(20_000);
                let t = binary(name, lit(a), lit(b));
                let actual = reduce(&c, &[], &t).expect("recurrence certificate");
                let expected = match name {
                    "Nat_add" => a + b,
                    "Nat_mul" => a * b,
                    _ => a.pow(b as u32),
                };
                assert_eq!(actual, lit(expected));
                let plain = c.order_recurrence_context();
                assert!(def_eq(&plain, &[], &t, &actual));
            }
        }
    }
    let _scope = budget(20_000);
    assert_eq!(
        reduce(
            &c,
            &[],
            &binary(
                "Nat_pow",
                apply(global("Succ"), apply(global("Succ"), global("Zero"))),
                lit(32)
            )
        ),
        Some(lit(4_294_967_296))
    );
}
#[test]
fn unknown_and_negative_operands_cannot_become_a_number() {
    let mut c = context();
    c.add_declaration("unknown", Term::Global("Nat".into()));
    for t in [
        global("unknown"),
        Db::Var(0),
        Db::Lit(Literal::Nat(crate::BigInt::from_i64(-1))),
    ] {
        let _scope = budget(20_000);
        assert_eq!(reduce(&c, &[], &binary("Nat_pow", t, lit(32))), None);
    }
}
#[test]
fn equation_probe_cannot_use_its_own_shortcut_and_budget_is_not_reset() {
    let c = context();
    let _scope = budget(20_000);
    let t = binary("Nat_pow", lit(2), lit(32));
    assert_eq!(reduce(&c.order_recurrence_context(), &[], &t), None);
    let _empty = budget(0);
    assert_eq!(reduce(&c, &[], &t), None);
    assert_eq!(WHNF_FUEL.with(|x| x.get()), 0);
}
fn nat() -> Term {
    Term::Global("Nat".into())
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
fn var(n: &str) -> Term {
    Term::Var(n.into())
}
fn call(f: Term, args: &[Term]) -> Term {
    args.iter()
        .cloned()
        .fold(f, |f, a| Term::App(Box::new(f), Box::new(a)))
}
/// A checked binary Nat function whose value is its first operand after
/// `rounds` structural steps. Its base-equation probes (`burn x Zero ≡ x`,
/// `≡ Zero`, `≡ Succ Zero`) must each walk every round even for an open `x`,
/// exactly like a well-founded definition such as `Nat.gcd` unfolding its
/// accessibility proof.
fn burning(rounds: u64) -> Context {
    let mut c = context();
    let motive = lam("_", nat(), nat());
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
                    motive: Box::new(motive),
                    cases: vec![
                        var("acc"),
                        lam("k", nat(), call(var("walk"), &[var("k"), var("acc")])),
                    ],
                },
            ),
        )),
    };
    let body = lam(
        "a",
        nat(),
        lam(
            "b",
            nat(),
            call(
                walk,
                &[
                    Term::Lit(Literal::Nat(crate::BigInt::from_u64(rounds))),
                    var("a"),
                ],
            ),
        ),
    );
    c.declare("burn", pi("a", nat(), pi("b", nat(), nat())), body)
        .unwrap();
    c
}
fn fuel() -> usize {
    WHNF_FUEL.with(|x| x.get())
}
fn exhausted() -> bool {
    WHNF_EXHAUSTED.with(|x| x.get())
}
#[test]
fn an_open_operand_is_refused_before_any_certification_probe() {
    let c = burning(50_000);
    let locals = vec![global("Nat")];
    let _scope = budget(20_000);
    assert_eq!(
        reduce(&c, &locals, &binary("burn", Db::Var(0), lit(0))),
        None
    );
    assert!(
        !exhausted(),
        "an open operand must not exhaust the caller's allowance"
    );
    // One base-equation probe alone would walk all 50_000 rounds.
    let first = 20_000 - fuel();
    assert!(
        first <= 16,
        "operand admission is constant work, spent {first}"
    );
    assert_eq!(
        reduce(&c, &locals, &binary("burn", lit(3), Db::Var(0))),
        None
    );
    let second = 20_000 - fuel() - first;
    assert!(
        !exhausted() && second <= 16,
        "second operand is admitted before probing too, spent {second}"
    );
}
#[test]
fn a_non_numeral_closed_operand_is_refused_before_any_certification_probe() {
    let mut c = burning(50_000);
    c.add_declaration("opaque_nat", nat());
    let _scope = budget(20_000);
    assert_eq!(
        reduce(&c, &[], &binary("burn", global("opaque_nat"), lit(0))),
        None
    );
    assert!(
        !exhausted(),
        "an opaque operand is not a numeral and needs no certificate"
    );
    assert!(20_000 - fuel() <= 16, "spent {}", 20_000 - fuel());
}
#[test]
fn a_non_nat_signature_is_refused_before_operands_are_evaluated() {
    let mut c = context();
    c.add_declaration("P", Term::Sort(Universe::Prop));
    c.declare(
        "pick",
        pi("a", Term::Sort(Universe::Prop), pi("b", nat(), nat())),
        lam("a", Term::Sort(Universe::Prop), lam("b", nat(), var("b"))),
    )
    .unwrap();
    let _scope = budget(20_000);
    let expensive = binary("Nat_pow", lit(3), lit(40));
    assert_eq!(
        reduce(&c, &[], &binary("pick", global("P"), expensive)),
        None
    );
    assert!(
        !exhausted() && 20_000 - fuel() <= 2,
        "no operand is evaluated for a non-arithmetic signature"
    );
}
#[test]
fn closed_numeral_operands_still_reach_the_certified_shortcut() {
    let c = burning(50_000);
    for name in ["Nat_add", "Nat_mul", "Nat_pow"] {
        let _scope = budget(20_000);
        assert!(
            reduce(&c, &[], &binary(name, lit(7), lit(5))).is_some(),
            "{name} keeps its certificate"
        );
        assert!(!exhausted());
    }
    let _scope = budget(20_000);
    assert_eq!(
        reduce(
            &c,
            &[],
            &binary("Nat_add", lit(7), apply(global("Succ"), lit(5)))
        ),
        Some(lit(13))
    );
    assert_eq!(
        reduce(
            &c,
            &[],
            &binary("Nat_mul", apply(global("Succ"), global("Zero")), lit(9))
        ),
        Some(lit(9))
    );
    let cheap = burning(8);
    let _small = budget(20_000);
    assert_eq!(
        reduce(&cheap, &[], &binary("burn", lit(2), lit(3))),
        None,
        "closed operands still require a certified recurrence, never a name"
    );
    assert!(!exhausted());
}
/// Lean compiles structural recursion through a helper (`Nat.mul := fun a b => Nat.brecOn b
/// Nat.mul._f a`), so the addition the step equation uses is named inside the helper's body,
/// not the operation's own body. The dependency search must look through such helpers.
fn product_through_a_helper() -> Context {
    let mut c = context();
    let motive = lam("_", nat(), nat());
    let helper = Term::Fix {
        name: "go".into(),
        body: Box::new(lam(
            "a",
            nat(),
            lam(
                "b",
                nat(),
                Term::Match {
                    discriminant: Box::new(var("b")),
                    motive: Box::new(motive),
                    cases: vec![
                        Term::Global("Zero".into()),
                        lam(
                            "k",
                            nat(),
                            call(
                                Term::Global("Nat_add".into()),
                                &[call(var("go"), &[var("a"), var("k")]), var("a")],
                            ),
                        ),
                    ],
                },
            ),
        )),
    };
    c.declare(
        "helper_mul_f",
        pi("a", nat(), pi("b", nat(), nat())),
        helper,
    )
    .unwrap();
    c.declare(
        "helper_mul",
        pi("a", nat(), pi("b", nat(), nat())),
        lam(
            "a",
            nat(),
            lam(
                "b",
                nat(),
                call(Term::Global("helper_mul_f".into()), &[var("a"), var("b")]),
            ),
        ),
    )
    .unwrap();
    c
}
#[test]
fn a_product_whose_addition_is_named_in_a_helper_is_certified() {
    let c = product_through_a_helper();
    let _scope = budget(200_000);
    assert_eq!(
        reduce(&c, &[], &binary("helper_mul", lit(6), lit(7))),
        Some(lit(42))
    );
    let _scope = budget(200_000);
    let big = crate::BigInt::parse_decimal("4294967296").unwrap();
    assert_eq!(
        reduce(&c, &[], &binary("helper_mul", lit(65536), lit(65536))),
        Some(Db::Lit(Literal::Nat(big))),
        "a 2^32 product needs the certified shortcut: unary evaluation exceeds the allowance"
    );
}
#[test]
fn a_helper_that_is_not_addition_does_not_certify_a_product() {
    let mut c = product_through_a_helper();
    let ty = c.get_definition_type("Nat_mul").unwrap().clone();
    // `go a (k+1) = go a k` everywhere: its base is 0 but its step never adds.
    let motive = lam("_", nat(), nat());
    let stuck = Term::Fix {
        name: "go".into(),
        body: Box::new(lam(
            "a",
            nat(),
            lam(
                "b",
                nat(),
                Term::Match {
                    discriminant: Box::new(var("b")),
                    motive: Box::new(motive),
                    cases: vec![
                        Term::Global("Zero".into()),
                        lam(
                            "k",
                            nat(),
                            call(
                                Term::Global("Nat_sub".into()),
                                &[call(var("go"), &[var("a"), var("k")]), var("a")],
                            ),
                        ),
                    ],
                },
            ),
        )),
    };
    c.declare("zero_f", ty.clone(), stuck).unwrap();
    c.declare(
        "zero_mul",
        ty,
        lam(
            "a",
            nat(),
            lam(
                "b",
                nat(),
                call(Term::Global("zero_f".into()), &[var("a"), var("b")]),
            ),
        ),
    )
    .unwrap();
    let _scope = budget(200_000);
    assert_eq!(reduce(&c, &[], &binary("zero_mul", lit(6), lit(7))), None);
}
#[test]
fn zero_exponents_multiword_results_and_resource_limits_are_exact() {
    let _scope = budget(20_000);
    let z = crate::BigInt::zero();
    let one = crate::BigInt::from_u64(1);
    let two = crate::BigInt::from_u64(2);
    let enormous = crate::BigInt::parse_decimal("18446744073709551616").unwrap();
    assert_eq!(calculate(Op::Power, &z, &z), Some(one.clone()));
    assert_eq!(calculate(Op::Power, &z, &enormous), Some(z));
    assert_eq!(calculate(Op::Power, &one, &enormous), Some(one));
    assert_eq!(
        calculate(Op::Power, &two, &crate::BigInt::from_u64(64)),
        Some(enormous.clone())
    );
    assert_eq!(calculate(Op::Power, &two, &enormous), None);
    assert_eq!(
        calculate(Op::Power, &two, &crate::BigInt::from_u64(1_000_000_000)),
        None
    );
}

/// `f x 0 = x` and `f x (k+1) = step (f x k)` by structural recursion on the second operand, with a
/// named unary `step` — Lean's `Nat.sub` is exactly this shape with `step = Nat.pred`.
fn stepping(c: &mut Context, name: &str, step: &str) {
    let motive = lam("_", nat(), nat());
    let body = Term::Fix {
        name: "f".into(),
        body: Box::new(lam(
            "x",
            nat(),
            lam(
                "y",
                nat(),
                Term::Match {
                    discriminant: Box::new(var("y")),
                    motive: Box::new(motive),
                    cases: vec![
                        var("x"),
                        lam(
                            "k",
                            nat(),
                            call(
                                Term::Global(step.into()),
                                &[call(var("f"), &[var("x"), var("k")])],
                            ),
                        ),
                    ],
                },
            ),
        )),
    };
    c.declare(name, pi("x", nat(), pi("y", nat(), nat())), body)
        .unwrap();
}
fn with_subtractions() -> Context {
    let mut c = context();
    let motive = lam("_", nat(), nat());
    c.declare(
        "my_pred",
        pi("n", nat(), nat()),
        lam(
            "n",
            nat(),
            Term::Match {
                discriminant: Box::new(var("n")),
                motive: Box::new(motive),
                cases: vec![Term::Global("Zero".into()), lam("k", nat(), var("k"))],
            },
        ),
    )
    .unwrap();
    c.declare(
        "my_double",
        pi("n", nat(), nat()),
        lam(
            "n",
            nat(),
            call(Term::Global("Nat_add".into()), &[var("n"), var("n")]),
        ),
    )
    .unwrap();
    stepping(&mut c, "my_sub", "my_pred");
    stepping(&mut c, "not_sub", "my_double");
    c
}
/// Truncated subtraction is certified by its base equation and a step through a program this kernel
/// proves is the predecessor — never by a step through another unary program.
#[test]
fn a_subtraction_is_certified_only_through_a_proved_predecessor() {
    let c = with_subtractions();
    for (a, b, expected) in [(7, 3, 4), (3, 7, 0), (5, 5, 0), (9, 0, 9)] {
        let _scope = budget(200_000);
        assert_eq!(
            reduce(&c, &[], &binary("my_sub", lit(a), lit(b))),
            Some(lit(expected)),
            "my_sub {a} {b}"
        );
    }
    let _scope = budget(200_000);
    let big = crate::BigInt::parse_decimal("340282366920938463463374607431768211456").unwrap();
    let two_64 = crate::BigInt::parse_decimal("18446744073709551616").unwrap();
    assert_eq!(
        reduce(
            &c,
            &[],
            &binary(
                "my_sub",
                Db::Lit(Literal::Nat(big.clone())),
                Db::Lit(Literal::Nat(two_64.clone()))
            )
        ),
        Some(Db::Lit(Literal::Nat(big.sub(&two_64)))),
        "a difference no unrolling reaches"
    );
    let _scope = budget(200_000);
    assert_eq!(
        reduce(&c, &[], &binary("not_sub", lit(7), lit(3))),
        None,
        "a step through doubling is no subtraction"
    );
    let _scope = budget(200_000);
    assert!(certifies_difference(&c, "my_sub"));
    assert!(!certifies_difference(&c, "not_sub"));
}

/// A recurrence is certified once per registry generation, and so is a refusal: an omega proof
/// observes the binary programs on naturals that compute no operation — Lean's `Nat.gcd`, `Nat.div`,
/// `Nat.mod` — at numerals over and over, and every certification probes their base equations.
#[test]
fn a_recurrence_and_a_refusal_are_each_certified_once_per_registry_generation() {
    let mut c = burning(8);
    let certifications = || CERTIFICATIONS.with(|n| n.get());
    let before = certifications();
    for _ in 0..3 {
        let _scope = budget(200_000);
        assert_eq!(
            reduce(&c, &[], &binary("Nat_add", lit(7), lit(5))),
            Some(lit(12))
        );
        assert_eq!(
            reduce(&c, &[], &binary("burn", lit(2), lit(3))),
            None,
            "burn computes no operation"
        );
        assert!(!exhausted());
    }
    assert_eq!(
        certifications(),
        before + 2,
        "one certification each, the refusal included"
    );
    c.declare("unrelated", nat(), Term::Global("Zero".into()))
        .unwrap();
    let _scope = budget(200_000);
    assert_eq!(
        reduce(&c, &[], &binary("Nat_add", lit(7), lit(5))),
        Some(lit(12))
    );
    assert_eq!(reduce(&c, &[], &binary("burn", lit(2), lit(3))), None);
    assert_eq!(
        certifications(),
        before + 4,
        "a registry that moved certifies again"
    );
}

/// A refusal reached on scarce fuel is a resource stop, never a conclusion: it is not retained, and
/// the same program certifies once the fuel suffices.
#[test]
fn a_refusal_on_scarce_fuel_is_not_retained() {
    let c = context();
    {
        let _scope = budget(64);
        assert_eq!(reduce(&c, &[], &binary("Nat_pow", lit(2), lit(10))), None);
    }
    let _scope = budget(200_000);
    assert_eq!(
        reduce(&c, &[], &binary("Nat_pow", lit(2), lit(10))),
        Some(lit(1024))
    );
    assert!(!exhausted());
}
