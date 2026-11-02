//! W06 step 4 — the Init `Nat.gcd.eq_def` independent-recheck decline, reduced to its mechanism.
//!
//! The regenerated Init export declined at `_private.Init.Data.Nat.Gcd.0.Nat.gcd.eq_def` after
//! ~177.8 s: independent rechecking spent its 1,048,576-step allowance on the arithmetic
//! recognizer's base-equation probes (`gcd x 0 ≡ x`, `≡ 0`, `≡ 1`) for an application whose
//! operands were the theorem's own bound variables. An open operand can never become a numeral,
//! so no certificate could ever have been used; the probes only burned the caller's allowance.
//!
//! `burn` reproduces that shape without Lean: a checked `Nat → Nat → Nat` definition whose base
//! probes must each walk `ROUNDS` structural steps even for an open first operand (exactly as a
//! well-founded definition unfolds its accessibility proof). Its equation theorem is stated over
//! open operands and needs only one δ-step, so it must be rechecked within the unchanged allowance.
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, recheck, BigInt, Context, Literal, ReCheckError, Term};

/// Three probes of this many steps exceed the independent kernel's 2^20-step allowance.
const ROUNDS: u64 = 400_000;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn nat() -> Term {
    g("Nat")
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
fn motive() -> Term {
    lam("_", nat(), nat())
}

/// `walk k acc` returns `acc` after `k` structural steps.
fn walk() -> Term {
    Term::Fix {
        name: "walk".into(),
        body: Box::new(lam(
            "k",
            nat(),
            lam(
                "acc",
                nat(),
                Term::Match {
                    discriminant: Box::new(v("k")),
                    motive: Box::new(motive()),
                    cases: vec![
                        v("acc"),
                        lam("j", nat(), call(v("walk"), &[v("j"), v("acc")])),
                    ],
                },
            ),
        )),
    }
}
/// `burn a b := match b with | 0 => walk ROUNDS a | _ + 1 => a` (the unfolded equation).
fn unfolded(a: Term, b: Term) -> Term {
    Term::Match {
        discriminant: Box::new(b),
        motive: Box::new(motive()),
        cases: vec![call(walk(), &[lit(ROUNDS), a.clone()]), lam("j", nat(), a)],
    }
}
fn environment() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.declare(
        "burn",
        pi("a", nat(), pi("b", nat(), nat())),
        lam("a", nat(), lam("b", nat(), unfolded(v("a"), v("b")))),
    )
    .expect("checked burn");
    ctx
}
fn equation(lhs: Term, rhs: Term) -> Term {
    call(g("Eq"), &[nat(), lhs, rhs])
}
/// `eq_def : ∀ m n, burn m n = <unfolded body>` proved by `rfl`, as Lean's equation lemma is.
fn eq_def(rhs: Term) -> Term {
    let statement = pi(
        "m",
        nat(),
        pi(
            "n",
            nat(),
            equation(call(g("burn"), &[v("m"), v("n")]), rhs),
        ),
    );
    let proof = lam(
        "m",
        nat(),
        lam(
            "n",
            nat(),
            call(g("refl"), &[nat(), call(g("burn"), &[v("m"), v("n")])]),
        ),
    );
    Term::Let {
        name: "eq_def".into(),
        ty: Box::new(statement),
        value: Box::new(proof),
        body: Box::new(v("eq_def")),
    }
}

#[test]
fn an_open_operand_equation_is_independently_rechecked_within_the_unchanged_allowance() {
    let ctx = environment();
    let theorem = eq_def(unfolded(v("m"), v("n")));
    infer_type(&ctx, &theorem).expect("the primary kernel accepts the equation lemma");
    let independent = recheck(&ctx, &theorem)
        .unwrap_or_else(|e| panic!("independent recheck must not exhaust its allowance: {e:?}"));
    assert!(
        matches!(independent, Term::Pi { .. }),
        "the rechecked type is the stated equation"
    );
}

#[test]
fn a_false_open_operand_equation_is_still_refused_by_the_independent_kernel() {
    let ctx = environment();
    // `burn m (j + 1)` is `m`; stating it as `m + 1` is not definitionally true. The zero
    // arm is kept verbatim so the refusal needs no walk, only the admitted unfolding.
    let wrong = Term::Match {
        discriminant: Box::new(v("n")),
        motive: Box::new(motive()),
        cases: vec![
            call(walk(), &[lit(ROUNDS), v("m")]),
            lam("j", nat(), call(g("Succ"), &[v("m")])),
        ],
    };
    let theorem = eq_def(wrong);
    assert!(
        infer_type(&ctx, &theorem).is_err(),
        "the primary kernel refuses the false equation"
    );
    let independent = recheck(&ctx, &theorem);
    assert!(matches!(independent, Err(ReCheckError::Ill(_))),
        "the independent kernel refuses it as ill-typed, not as an exhausted allowance: {independent:?}");
}

#[test]
fn closed_arithmetic_in_the_same_environment_keeps_its_certified_shortcut() {
    let ctx = environment();
    // 3^40 has 64 bits of value; Peano evaluation would need ~1.2e19 steps.
    let big = BigInt::parse_decimal("12157665459056928801").unwrap();
    let theorem = Term::Let {
        name: "pow".into(),
        ty: Box::new(equation(
            call(g("Nat_pow"), &[lit(3), lit(40)]),
            Term::Lit(Literal::Nat(big.clone())),
        )),
        value: Box::new(call(g("refl"), &[nat(), Term::Lit(Literal::Nat(big))])),
        body: Box::new(v("pow")),
    };
    recheck(&ctx, &theorem).expect("valid closed arithmetic is still accelerated and accepted");
}
