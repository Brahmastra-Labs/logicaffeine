//! Both native evaluators must consume the standard kernel's Nat/Int literal bridges.
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    eval_bool_tree, infer_type, native_compile_decide, normalize_checked, BigInt, Context, Literal,
    Term,
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
fn apps(n: &str, xs: &[Term]) -> Term {
    xs.iter().cloned().fold(g(n), app)
}
fn lam(n: &str, ty: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
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
fn agree(c: &Context, t: &Term, want: bool) {
    assert_eq!(infer_type(c, t).unwrap(), g("Bool"));
    assert_eq!(
        normalize_checked(c, t).unwrap(),
        g(if want { "true" } else { "false" })
    );
    assert_eq!(eval_bool_tree(c, t), Some(want), "tree: {t}");
    assert_eq!(native_compile_decide(c, t), Some(want), "compiled: {t}");
}
fn nonnegative(t: Term) -> Term {
    Term::Match {
        discriminant: Box::new(t),
        motive: Box::new(lam("i", g("Int"), g("Bool"))),
        cases: vec![
            lam("n", g("Nat"), g("true")),
            lam("n", g("Nat"), g("false")),
        ],
    }
}
#[test]
fn native_integer_literal_matches_agree_with_kernel_including_bigints() {
    let c = ctx();
    for n in [i64::MIN, -1, 0, 1, i64::MAX] {
        agree(&c, &nonnegative(int(n)), n >= 0);
    }
    for (n, want) in [
        ("18446744073709551617", true),
        ("-18446744073709551617", false),
    ] {
        agree(
            &c,
            &nonnegative(Term::Lit(Literal::BigInt(
                BigInt::parse_decimal(n).unwrap(),
            ))),
            want,
        );
    }
}
#[test]
fn native_structural_integer_arithmetic_feeds_primitive_comparisons() {
    let c = ctx();
    for a in -2..=2 {
        for b in -2..=2 {
            for (name, want) in [("Int_add", a + b), ("Int_sub", a - b), ("Int_mul", a * b)] {
                let value = apps(name, &[int(a), int(b)]);
                agree(&c, &apps("le", &[value.clone(), int(want)]), true);
                agree(&c, &apps("lt", &[value, int(want)]), false);
            }
        }
    }
}
#[test]
fn native_primitives_read_constructor_operands_and_peano_payloads() {
    let c = ctx();
    let positive = apps("Int_ofNat", &[app(g("Succ"), app(g("Succ"), g("Zero")))]);
    let negative = apps("Int_negSucc", &[nat(2)]);
    for (name, want) in [("add", -1), ("sub", 5), ("mul", -6), ("div", 0), ("mod", 2)] {
        let value = apps(name, &[positive.clone(), negative.clone()]);
        agree(&c, &apps("le", &[value.clone(), int(want)]), true);
        agree(&c, &apps("lt", &[value, int(want)]), false);
    }
}
#[test]
fn native_nat_literal_matches_and_recursion_remain_compact() {
    let c = ctx();
    let parity = Term::Fix {
        name: "f".into(),
        body: Box::new(lam(
            "n",
            g("Nat"),
            Term::Match {
                discriminant: Box::new(v("n")),
                motive: Box::new(lam("_", g("Nat"), g("Bool"))),
                cases: vec![
                    g("true"),
                    lam(
                        "p",
                        g("Nat"),
                        Term::Match {
                            discriminant: Box::new(v("p")),
                            motive: Box::new(lam("_", g("Nat"), g("Bool"))),
                            cases: vec![g("false"), lam("k", g("Nat"), app(v("f"), v("k")))],
                        },
                    ),
                ],
            },
        )),
    };
    for n in 0..=20 {
        agree(&c, &app(parity.clone(), nat(n)), n % 2 == 0);
    }
}
#[test]
fn native_int_bridge_is_disabled_when_the_constructor_is_not_canonical() {
    // In the standard environment `Int_ofNat` is relied on, so re-signing it is refused and the
    // bridge keeps working.
    let mut c = ctx();
    c.add_declaration("Int_ofNat", g("Nat"));
    assert!(
        c.refused_declarations()
            .iter()
            .any(|(n, why)| n == "Int_ofNat" && why.starts_with("Stable meaning violation")),
        "{:?}",
        c.refused_declarations()
    );
    agree(&c, &nonnegative(int(0)), true);
    // Where nothing relies on it, `Int_ofNat` may be something other than the canonical
    // constructor, and then neither native evaluator bridges a literal through it.
    let mut off = Context::new();
    off.add_inductive("Nat", Term::Sort(logicaffeine_kernel::Universe::Type(0)));
    off.add_constructor("Zero", "Nat", g("Nat"));
    off.add_constructor(
        "Succ",
        "Nat",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Nat")),
        },
    );
    off.add_inductive("Bool", Term::Sort(logicaffeine_kernel::Universe::Type(0)));
    off.add_constructor("false", "Bool", g("Bool"));
    off.add_constructor("true", "Bool", g("Bool"));
    off.add_inductive("Int", Term::Sort(logicaffeine_kernel::Universe::Type(0)));
    off.add_constructor(
        "Int_negSucc",
        "Int",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Int")),
        },
    );
    off.add_declaration("Int_ofNat", g("Nat"));
    assert!(
        off.refused_declarations().is_empty(),
        "{:?}",
        off.refused_declarations()
    );
    let term = nonnegative(int(0));
    assert_eq!(eval_bool_tree(&off, &term), None);
    assert_eq!(native_compile_decide(&off, &term), None);
}

#[test]
fn native_large_literal_recursion_exhausts_safely_instead_of_overflowing_stack() {
    const CHILD: &str = "LOGOS_NATIVE_LITERAL_STACK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("native_large_literal_recursion_exhausts_safely_instead_of_overflowing_stack")
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "child terminated abnormally: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    let c = ctx();
    let countdown = Term::Fix {
        name: "count".into(),
        body: Box::new(lam(
            "n",
            g("Nat"),
            Term::Match {
                discriminant: Box::new(v("n")),
                motive: Box::new(lam("_", g("Nat"), g("Bool"))),
                cases: vec![g("true"), lam("p", g("Nat"), app(v("count"), v("p")))],
            },
        )),
    };
    let term = app(countdown, nat(1_000_000));
    assert_eq!(infer_type(&c, &term).unwrap(), g("Bool"));
    assert!(matches!(eval_bool_tree(&c, &term), Some(true) | None));
    assert!(matches!(
        native_compile_decide(&c, &term),
        Some(true) | None
    ));
}
