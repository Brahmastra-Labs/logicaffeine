//! Lean's `Nat.ble` on literals: a CERTIFIED order acceleration in both kernels.
//!
//! `isValidChar_UInt32` (42 open roots) and `LF_NB.charBound : (1114112 : Nat) < 4294967296 :=
//! by decide` reduce to Lean's imported `Nat.ble` on literals. Lean's definition matches on BOTH
//! arguments, so its first equation is two cases, `ble 0 0 = true` and `ble 0 (m+1) = true`; the
//! kernels' certified order recognizer only accepted the one-case `f 0 m = yes` form and fell back
//! to unary reduction (about 1.1M steps, past the 2^20 budget). A program is accelerated only when
//! its own equations are proved by conversion in the kernel that uses it; nothing is recognized by
//! name, and a program with a different equation is never accelerated.
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, recheck, BigInt, Context, Literal, Term};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn nat() -> Term {
    g("Nat")
}
fn bool_() -> Term {
    g("Bool")
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
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn on_nat(d: Term, zero: Term, succ: Term) -> Term {
    Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(lam("_", nat(), bool_())),
        cases: vec![zero, succ],
    }
}
/// Lean's shape: `| 0, 0 => a | 0, _+1 => b | _+1, 0 => c | n+1, m+1 => f n m`.
fn two_sided(name: &str, a: &str, b: &str, c: &str) -> Term {
    let rec = format!("{name}.rec");
    Term::Fix {
        name: rec.clone(),
        body: Box::new(lam(
            "n",
            nat(),
            lam(
                "m",
                nat(),
                on_nat(
                    v("n"),
                    on_nat(v("m"), g(a), lam("m1", nat(), g(b))),
                    lam(
                        "n1",
                        nat(),
                        on_nat(
                            v("m"),
                            g(c),
                            lam("m1", nat(), call(v(&rec), &[v("n1"), v("m1")])),
                        ),
                    ),
                ),
            ),
        )),
    }
}
fn environment() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let ty = pi("n", nat(), pi("m", nat(), bool_()));
    ctx.declare(
        "lean_ble",
        ty.clone(),
        two_sided("lean_ble", "true", "true", "false"),
    )
    .unwrap();
    // Differs only in `0, m+1`: it is `n = m` on the diagonal, not `≤`.
    ctx.declare(
        "not_ble",
        ty,
        two_sided("not_ble", "true", "false", "false"),
    )
    .unwrap();
    ctx
}
/// `refl : Eq Bool (f a b) answer`, which checks only if `f a b` reduces to `answer`.
fn claim(f: &str, a: u64, b: u64, answer: &str) -> Term {
    let statement = call(
        g("Eq"),
        &[bool_(), call(g(f), &[lit(a), lit(b)]), g(answer)],
    );
    Term::Let {
        name: "claim".into(),
        ty: Box::new(statement),
        value: Box::new(call(g("refl"), &[bool_(), g(answer)])),
        body: Box::new(v("claim")),
    }
}

#[test]
fn leans_two_sided_ble_decides_the_char_bound_in_both_kernels() {
    let ctx = environment();
    for (a, b, answer) in [
        (1_114_112, 4_294_967_296, "true"),
        (4_294_967_296, 1_114_112, "false"),
        (4_294_967_296, 4_294_967_296, "true"),
        (0, 0, "true"),
        (3, 5, "true"),
    ] {
        let t = claim("lean_ble", a, b, answer);
        infer_type(&ctx, &t).unwrap_or_else(|e| panic!("primary kernel on {a} ≤ {b}: {e:?}"));
        recheck(&ctx, &t).unwrap_or_else(|e| panic!("independent kernel on {a} ≤ {b}: {e:?}"));
    }
}

#[test]
fn the_accelerated_answer_is_never_the_wrong_constructor() {
    let ctx = environment();
    let t = claim("lean_ble", 1_114_112, 4_294_967_296, "false");
    assert!(
        infer_type(&ctx, &t).is_err(),
        "the primary kernel refuses a false bound"
    );
    assert!(
        recheck(&ctx, &t).is_err(),
        "the independent kernel refuses a false bound"
    );
}

#[test]
fn a_program_with_a_different_equation_is_not_accelerated() {
    let ctx = environment();
    // Small inputs still compute by ordinary reduction.
    for (a, b, answer) in [(3, 5, "false"), (5, 5, "true"), (5, 3, "false")] {
        let t = claim("not_ble", a, b, answer);
        infer_type(&ctx, &t).unwrap_or_else(|e| panic!("small not_ble {a} {b}: {e:?}"));
        recheck(&ctx, &t).unwrap_or_else(|e| panic!("small not_ble {a} {b}: {e:?}"));
    }
    // A large input is never decided as an order: `true` would be the `≤` answer.
    let t = claim("not_ble", 1_114_112, 4_294_967_296, "true");
    assert!(
        infer_type(&ctx, &t).is_err(),
        "not_ble is not an order and must not be accelerated as one"
    );
    assert!(recheck(&ctx, &t).is_err());
}

#[test]
fn the_original_char_bound_imports_through_leans_own_nat_ble() {
    let input = include_str!("fixtures/lean_semantics/LF_NatBle.charBound.ndjson");
    let (ctx, _) = logicaffeine_kernel::interface::lean::import_audited(input)
        .unwrap_or_else(|e| panic!("the original charBound import: {e:?}"));
    let name = r#"/s:"LF_NB"/s:"charBound""#;
    let ty = ctx.get_global(name).expect("the theorem is published");
    assert!(
        matches!(ty, Term::App(..)),
        "its statement is the original `1114112 < 4294967296`: {ty}"
    );
}
