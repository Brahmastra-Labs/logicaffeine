//! Tier 10 of the Lean-4 conformance corpus — literals and primitive types.
//!
//! The spine of this tier is one fact: LOGOS's `Nat` and LOGOS's `Int` are different KINDS
//! of thing. `Nat` is a genuine Peano inductive (`Zero`/`Succ`) with an accelerated
//! `Literal::Nat(BigInt)` that the kernel treats as *definitionally* `Succ^n Zero` — it
//! computes by ι-reduction and it is arbitrary-precision end to end. `Int` is an inductive
//! with ZERO constructors: an opaque carrier with sixteen axiomatised ring/order laws and an
//! arithmetic that computes only by a primitive hook on literal operands. It cannot be
//! matched on, it cannot be inducted over, and none of its laws is proved. Lean's `Int` is
//! `ofNat`/`negSucc` and *proves* every one of them.
//!
//! Around that spine sit two kinds of case. The first is absence: `Char`, `String`
//! operations, every `UInt*`, `BitVec`, `Array`, Lean's `List`, `Option`, `Prod`, `Sum`,
//! `Sigma`, `Subtype`, `Fin` — none of them is in the prelude, and several are blocked three
//! layers deep (a `Char` needs `UInt32` needs `Subtype`). Those tests assert the capability
//! and name what is missing.
//!
//! The second kind is worse than absence: a SILENT WRONG ANSWER. LOGOS's `div`/`mod`
//! truncate toward zero with the remainder carrying the dividend's sign; Lean 4's `/` and `%`
//! on `Int` are EUCLIDEAN. They agree on every non-negative dividend, which is exactly why
//! testing misses it. And `Literal::Float`'s definitional equality is Rust's IEEE `PartialEq`,
//! under which `NaN ≠ NaN` (so `def_eq` is not reflexive on a well-typed term) and
//! `0.0 = -0.0` (so two distinct values are definitionally identified). Those two tests
//! enumerate every disagreement they find.

use logicaffeine_kernel::interface::Repl;
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, double_check, infer_type, int_lit, is_subtype, normalize,
    surface_elaborate_against, BigInt, Context, DoubleCheck, Literal, Term, Universe,
};

// ==== HELPERS ====

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |a, x| app(a, x.clone()))
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn pi(p: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn fix(name: &str, body: Term) -> Term {
    Term::Fix {
        name: name.to_string(),
        body: Box::new(body),
    }
}
fn mat(d: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(motive),
        cases,
    }
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

fn nat() -> Term {
    g("Nat")
}
fn succ(x: Term) -> Term {
    app(g("Succ"), x)
}
/// The genuine unary Peano numeral `Succ^n Zero`.
fn peano(n: u64) -> Term {
    (0..n).fold(g("Zero"), |acc, _| succ(acc))
}
/// The accelerated `Nat` literal.
fn natl(n: i64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_i64(n)))
}
fn natl_dec(s: &str) -> Term {
    Term::Lit(Literal::Nat(
        BigInt::parse_decimal(s).expect("decimal Nat literal"),
    ))
}
fn intl(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn textl(s: &str) -> Term {
    Term::Lit(Literal::Text(s.to_string()))
}
fn floatl(x: f64) -> Term {
    Term::Lit(Literal::Float(x))
}
fn bin(op: &str, a: Term, b: Term) -> Term {
    app(app(g(op), a), b)
}
fn eq_at(ty: Term, a: Term, b: Term) -> Term {
    apps(g("Eq"), &[ty, a, b])
}
/// The globals of `names` that the prelude does not register.
fn absent<'a>(ctx: &Context, names: &[&'a str]) -> Vec<&'a str> {
    names
        .iter()
        .copied()
        .filter(|n| ctx.get_global(n).is_none())
        .collect()
}
/// Run `f`, reporting `Err(())` when the kernel PANICS instead of answering. The panic hook is
/// silenced for the duration so a caught crash does not spray a backtrace across the run.
fn without_panicking<T>(f: impl FnOnce() -> T) -> Result<T, ()> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    std::panic::set_hook(previous);
    out.map_err(|_| ())
}

/// `Nat → Nat → Nat`, the type of every emitted `Nat` binary operation.
fn nat_binop_ty() -> Term {
    pi("_", nat(), pi("_", nat(), nat()))
}
/// `natAdd m n` by structural recursion on `m` — the translation Lean's `Nat.add` needs,
/// since the LOGOS prelude ships NO `Nat` arithmetic at all.
fn emitted_nat_add() -> Term {
    fix(
        "rec",
        lam(
            "m",
            nat(),
            lam(
                "n",
                nat(),
                mat(
                    var("m"),
                    lam("_", nat(), nat()),
                    vec![
                        var("n"),
                        lam("k", nat(), succ(apps(var("rec"), &[var("k"), var("n")]))),
                    ],
                ),
            ),
        ),
    )
}
/// `natSub m n` — Lean's TRUNCATING `Nat` subtraction, written as the structural recursion
/// `sub n 0 = n`, `sub 0 (Succ _) = Zero`, `sub (Succ m) (Succ n) = sub m n`.
fn emitted_nat_sub() -> Term {
    fix(
        "rec",
        lam(
            "m",
            nat(),
            lam(
                "n",
                nat(),
                mat(
                    var("m"),
                    lam("_", nat(), nat()),
                    vec![
                        g("Zero"),
                        lam(
                            "mk",
                            nat(),
                            mat(
                                var("n"),
                                lam("_", nat(), nat()),
                                vec![
                                    succ(var("mk")),
                                    lam("nk", nat(), apps(var("rec"), &[var("mk"), var("nk")])),
                                ],
                            ),
                        ),
                    ],
                ),
            ),
        ),
    )
}

// ==== SECTION: Nat — the accelerated Peano literal ====

/// T10-001 · A small `Nat` literal is `Literal::Nat`, types as the INDUCTIVE `Nat`, is
/// definitionally its Peano spelling, and drives a `Match`. The tempting bug is emitting it
/// as `Literal::Int` because the value fits an `i64` — that lands in the opaque zero-
/// constructor `Int` where every downstream elimination is stuck.
#[test]
fn a_small_nat_literal_is_a_nat_not_an_int() {
    let mut ctx = std_ctx();

    // `def zeroLit : Nat := 0`, `fiveLit := 5`, `largeButMachine := 9223372036854775807`.
    for n in [0i64, 5, i64::MAX] {
        assert_eq!(
            infer_type(&ctx, &natl(n)).expect("a Nat literal type-checks"),
            nat(),
            "a Lean `Nat` literal must land in the INDUCTIVE Nat, not the opaque Int: {n}"
        );
    }

    // The literal is NOT an `Int`, and the two carriers are not interchangeable in either
    // direction — this is what keeps the `Nat ↦ Int` mistranslation from being silent.
    assert_eq!(
        infer_type(&ctx, &intl(5)).expect("an Int literal type-checks"),
        g("Int"),
        "an Int literal must land in Int"
    );
    assert!(
        !is_subtype(&ctx, &nat(), &g("Int")),
        "Nat must not be a subtype of Int — a Nat literal mis-emitted as Int must be caught"
    );

    // `example : (3 : Nat) = Nat.succ (Nat.succ (Nat.succ Nat.zero)) := rfl`
    assert!(
        defeq_for_test(&ctx, &natl(3), &peano(3)),
        "the literal 3 must be definitionally `Succ (Succ (Succ Zero))`"
    );

    // `def isZero (n : Nat) : Bool := match n with | 0 => true | _ + 1 => false`
    let is_zero = lam(
        "n",
        nat(),
        mat(
            var("n"),
            lam("_", nat(), g("Bool")),
            vec![g("true"), lam("_", nat(), g("false"))],
        ),
    );
    assert_eq!(
        normalize(&ctx, &app(is_zero.clone(), natl(0))),
        g("true"),
        "isZero 0 = true — a Nat literal is constructor-form"
    );
    assert_eq!(
        normalize(&ctx, &app(is_zero, natl(7))),
        g("false"),
        "isZero 7 = false"
    );

    // `example : (2 : Nat) + 3 = 5 := rfl`. LOGOS ships no `Nat` addition, so the sum is a
    // translator-emitted structural recursion computing through the literal bridge.
    ctx.add_definition("natAdd".to_string(), nat_binop_ty(), emitted_nat_add());
    let two_plus_three = apps(g("natAdd"), &[natl(2), natl(3)]);
    assert_eq!(
        infer_type(&ctx, &two_plus_three).expect("emitted Nat addition type-checks"),
        nat(),
        "2 + 3 must be a Nat"
    );
    assert!(
        defeq_for_test(&ctx, &two_plus_three, &natl(5)),
        "2 + 3 = 5 at Nat: {:?}",
        normalize(&ctx, &two_plus_three)
    );
}

/// T10-002 · A `Nat` literal past `i64` keeps every digit. `Literal::Nat` always carries a
/// `BigInt`, so small and large are the same constructor and no widening decision arises;
/// the parallel INTEGER representation is canonical (`int_lit` demotes whatever fits), which
/// is what keeps definitional equality of literals sound.
#[test]
fn a_nat_literal_past_the_machine_word_keeps_every_digit() {
    let ctx = std_ctx();

    // `justPastI64 := 2^63`, `justPastU64 := 2^64`, `wideDecimal := 1234…7890`.
    let two63 = natl_dec("9223372036854775808");
    let two64 = natl_dec("18446744073709551616");
    let wide = natl_dec("123456789012345678901234567890");
    for (name, t) in [("2^63", &two63), ("2^64", &two64), ("wideDecimal", &wide)] {
        assert_eq!(
            infer_type(&ctx, t).expect("a wide Nat literal type-checks"),
            nat(),
            "{name} must type-check as a Nat without truncation"
        );
    }

    // A truncating emitter (`as i64` / `as u64`) collapses distinct values onto one. These
    // two differ only in their last digit and must stay distinct.
    let wide_plus = natl_dec("123456789012345678901234567891");
    assert!(
        !defeq_for_test(&ctx, &wide, &wide_plus),
        "two 30-digit literals differing in the last digit must not be definitionally equal"
    );
    assert!(
        !defeq_for_test(&ctx, &two63, &two64),
        "2^63 and 2^64 must stay distinct"
    );

    // `example : (18446744073709551616 : Nat) = 18446744073709551615 + 1 := by decide` —
    // through the Peano bridge, which peels exactly one `Succ` off a 20-digit literal.
    assert!(
        defeq_for_test(&ctx, &two64, &succ(natl_dec("18446744073709551615"))),
        "2^64 must be definitionally Succ (2^64 - 1)"
    );

    // The canonicality invariant `int_lit` exists to hold: nothing that fits an `i64` may be
    // stored wide, or two spellings of one value stop being `==`-equal and `rfl` fails.
    for n in [0i64, 1, -1, i64::MAX, i64::MIN] {
        assert!(
            matches!(int_lit(BigInt::from_i64(n)), Literal::Int(x) if x == n),
            "int_lit must keep {n} in the machine form"
        );
    }
    for wide_dec in [
        "9223372036854775808",
        "-9223372036854775809",
        "10000000000000000000000",
    ] {
        let lit = int_lit(BigInt::parse_decimal(wide_dec).expect("decimal"));
        match lit {
            Literal::BigInt(ref b) => assert!(
                b.to_i64().is_none(),
                "a wide literal must hold only values i64 cannot: {wide_dec}"
            ),
            other => panic!("int_lit must widen {wide_dec}, got {other:?}"),
        }
    }

    // Arbitrary precision is not just representable — it COMPUTES exactly. `2^63 + 2^63` has
    // no `i64` answer, so the kernel must fall through to the exact path rather than wrap.
    let sum = bin(
        "add",
        Term::Lit(int_lit(
            BigInt::parse_decimal("9223372036854775807").unwrap(),
        )),
        intl(1),
    );
    assert_eq!(
        normalize(&ctx, &sum),
        Term::Lit(int_lit(
            BigInt::parse_decimal("9223372036854775808").unwrap()
        )),
        "i64::MAX + 1 must promote to the exact representation, never wrap"
    );
}

/// T10-015 · `Literal::Nat(n)` is definitionally `Succ^n Zero` — in BOTH kernels — and the
/// bridge is what makes a `Match` on a 50000 peel ONE `Succ` rather than materialise 50000
/// heap nodes and exhaust `normalize`'s fuel.
#[test]
fn a_nat_literal_peels_one_succ_per_step_however_large_it_is() {
    let ctx = std_ctx();

    // `example : (0 : Nat) = Nat.zero := rfl` and the successor spellings, both orientations.
    for n in 0u64..32 {
        assert!(
            defeq_for_test(&ctx, &natl(n as i64), &peano(n)),
            "Nat({n}) ≡ Succ^{n} Zero"
        );
        assert!(
            defeq_for_test(&ctx, &peano(n), &natl(n as i64)),
            "…and symmetrically at {n}"
        );
    }

    // `def pred : Nat → Nat | 0 => 0 | n + 1 => n`
    let pred = lam(
        "n",
        nat(),
        mat(
            var("n"),
            lam("_", nat(), nat()),
            vec![g("Zero"), lam("k", nat(), var("k"))],
        ),
    );
    assert_eq!(
        normalize(&ctx, &app(pred.clone(), natl(0))),
        g("Zero"),
        "pred 0 = 0"
    );
    assert!(
        defeq_for_test(
            &ctx,
            &normalize(&ctx, &app(pred.clone(), natl(5))),
            &natl(4)
        ),
        "pred 5 = 4"
    );

    // `def bigLit : Nat := 50000` / `example : pred bigLit = 49999 := rfl`. The ACCELERATION
    // claim: the peel is O(1), so the result is still a compact literal — a Peano spelling
    // would have blown the 10000-step fuel long before here.
    assert_eq!(
        normalize(&ctx, &app(pred, natl(50000))),
        natl(49999),
        "pred 50000 must peel one Succ and stay a compact literal"
    );

    // `minusThree` — three peels off one literal, the incremental behaviour of the bridge.
    let minus_three = lam(
        "n",
        nat(),
        mat(
            var("n"),
            lam("_", nat(), nat()),
            vec![
                g("Zero"),
                lam(
                    "a",
                    nat(),
                    mat(
                        var("a"),
                        lam("_", nat(), nat()),
                        vec![
                            g("Zero"),
                            lam(
                                "b",
                                nat(),
                                mat(
                                    var("b"),
                                    lam("_", nat(), nat()),
                                    vec![g("Zero"), lam("c", nat(), var("c"))],
                                ),
                            ),
                        ],
                    ),
                ),
            ],
        ),
    );
    assert!(
        defeq_for_test(
            &ctx,
            &normalize(&ctx, &app(minus_three, natl(10))),
            &natl(7)
        ),
        "minusThree 10 = 7"
    );

    // Literal-versus-literal equality is decided by comparing `BigInt`s, not by peeling.
    assert!(
        defeq_for_test(&ctx, &natl(50000), &natl(50000)),
        "50000 = 50000"
    );
    assert!(
        !defeq_for_test(&ctx, &natl(50000), &natl(50001)),
        "50000 ≠ 50001"
    );

    // "in BOTH kernels": a term that type-checks ONLY through the bridge must satisfy the
    // independent re-checker too, or the double-kernel guarantee is empty here.
    let coerce = app(
        lam("h", eq_at(nat(), natl(3), peano(3)), g("Zero")),
        apps(g("refl"), &[nat(), peano(3)]),
    );
    assert!(
        matches!(double_check(&ctx, &coerce), DoubleCheck::Agreed),
        "both kernels must accept the literal/Peano bridge: {:?}",
        double_check(&ctx, &coerce)
    );
}

/// T10-017 · Lean's `Nat` subtraction saturates at zero and is a GMP primitive. LOGOS ships
/// no `Nat` arithmetic at all, and its `sub` is `Int_sub` (the checked alias `sub := Int_sub`), which goes negative. The
/// translator must emit a structural recursion — which truncates correctly and then walks
/// straight into `normalize`'s 10000-step fuel wall, where a TRUE equation silently stops
/// being provable.
#[test]
fn nat_subtraction_truncates_at_zero_and_scales() {
    let mut ctx = std_ctx();

    // The prelude's `sub` is the checked alias `sub := Int_sub`. Applied to `Nat` literals it is
    // ill-typed AND computes no value — that double refusal is what keeps the `Nat ↦ Int`
    // conflation loud.
    assert_eq!(
        ctx.get_global("sub").expect("the prelude declares sub"),
        &pi("_", g("Int"), pi("_", g("Int"), g("Int"))),
        "the prelude's only subtraction is Int → Int → Int"
    );
    let bad = bin("sub", natl(3), natl(8));
    assert!(
        infer_type(&ctx, &bad).is_err(),
        "sub on Nat literals must be ill-typed"
    );
    assert!(
        logicaffeine_kernel::recheck(&ctx, &bad).is_err(),
        "…in the independent kernel too"
    );
    // …and must not compute either. Unfolding the alias is delta, not a value: the `Int` match
    // inside `Int_sub` must stay stuck on the `Nat` literal, because a `Nat` constructor never
    // selects an `Int` branch (no iota fires).
    let stuck = normalize(&ctx, &bad);
    assert_ne!(
        stuck,
        app(g("Int_ofNat"), natl(5)),
        "3 - 8 must not become the Int 5"
    );
    let mut head = &stuck;
    while let Term::App(f, _) = head {
        head = f;
    }
    assert!(
        !matches!(head, Term::Lit(_)) && !matches!(head, Term::Global(c) if ctx.is_constructor(c)),
        "sub on Nat literals must not compute to a literal or constructor value: {stuck:?}"
    );
    assert!(
        matches!(&stuck, Term::Match { discriminant, .. } if **discriminant == natl(3)),
        "no iota fired: the Int match is still waiting on the Nat literal 3: {stuck:?}"
    );
    assert_eq!(
        normalize(&ctx, &bin("sub", intl(3), intl(8))),
        intl(-5),
        "at Int the same numerals give -5, where Lean's Nat gives 0"
    );

    // `def natSub : Nat → Nat → Nat` — the emitted recursion. It type-checks (the Giménez
    // guard accepts the decrease) and it truncates exactly as Lean does.
    let sub_def = emitted_nat_sub();
    let sub_ty = infer_type(&ctx, &sub_def).expect("the emitted Nat subtraction type-checks");
    assert!(
        defeq_for_test(&ctx, &sub_ty, &nat_binop_ty()),
        "natSub : Nat → Nat → Nat, got {sub_ty:?}"
    );
    ctx.add_definition("natSub".to_string(), nat_binop_ty(), sub_def);
    let sub = |a: i64, b: i64| normalize(&ctx, &apps(g("natSub"), &[natl(a), natl(b)]));
    for (a, b, want) in [
        (3i64, 8i64, 0i64),
        (0, 1, 0),
        (8, 3, 5),
        (5, 5, 0),
        (9, 0, 9),
    ] {
        assert!(
            defeq_for_test(&ctx, &sub(a, b), &natl(want)),
            "Lean: ({a} : Nat) - {b} = {want}; LOGOS emitted: {:?}",
            sub(a, b)
        );
    }

    // `example : (50000 : Nat) - 1 = 49999 := rfl`. Lean answers in one GMP step. The
    // emitted recursion costs one ι-step per unit of `min(a, b)`, and past the fuel the
    // kernel returns a PARTIALLY reduced term and reports nothing: a true equation becomes
    // unprovable with no diagnostic anywhere.
    assert!(
        defeq_for_test(&ctx, &sub(5007, 5000), &natl(7)),
        "Lean: (5007 : Nat) - 5000 = 7 — the emitted recursion must not silently run out of fuel"
    );
}

// ==== SECTION: Int — Lean's ofNat/negSucc inductive, with a compact literal ====

/// T10-003 · A negative `Int` literal is `int_lit`-canonical, types as `Int`, and computes
/// through the primitive hook. `Int` is Lean's own `ofNat | negSucc` inductive, and the literal
/// is DEFINITIONALLY its constructor spelling, so a structurally faithful `Int.negSucc`
/// translation lands on the same `Int` as the literal — there is no second, shadow encoding.
#[test]
fn a_negative_int_literal_canonicalises_at_the_i64_boundary() {
    let ctx = std_ctx();

    // `def minusFive : Int := -5`
    assert_eq!(
        infer_type(&ctx, &intl(-5)).expect("a negative Int literal type-checks"),
        g("Int"),
        "-5 must be an Int"
    );

    // `example : (-0 : Int) = (0 : Int) := rfl` — one representation per value.
    assert_eq!(
        int_lit(BigInt::from_i64(0)),
        Literal::Int(0),
        "-0 canonicalises to 0"
    );

    // `i64Min` fits the machine form and must NOT widen; `belowI64Min` does not fit and MUST.
    assert_eq!(
        int_lit(BigInt::parse_decimal("-9223372036854775808").unwrap()),
        Literal::Int(i64::MIN),
        "i64::MIN must stay in the machine form"
    );
    match int_lit(BigInt::parse_decimal("-9223372036854775809").unwrap()) {
        Literal::BigInt(b) => assert!(b.to_i64().is_none(), "one below i64::MIN must widen"),
        other => panic!("one below i64::MIN must widen, got {other:?}"),
    }

    // `example : (3 : Int) - 8 = -5 := by decide`, and the harness oracle
    // `normalize(add (-5) 3) = Lit(Int(-2))`.
    assert_eq!(
        normalize(&ctx, &bin("sub", intl(3), intl(8))),
        intl(-5),
        "3 - 8 = -5 at Int"
    );
    assert_eq!(
        normalize(&ctx, &bin("add", intl(-5), intl(3))),
        intl(-2),
        "(-5) + 3 = -2"
    );
    assert_eq!(
        normalize(&ctx, &bin("mul", intl(-5), intl(3))),
        intl(-15),
        "(-5) * 3 = -15"
    );

    // The results re-canonicalise, so a wide computation that lands back inside `i64` is the
    // SAME literal as the small spelling — otherwise `rfl` fails between two equal values.
    let wide = Term::Lit(int_lit(
        BigInt::parse_decimal("-9223372036854775809").unwrap(),
    ));
    assert_eq!(
        normalize(&ctx, &bin("add", wide, intl(1))),
        intl(i64::MIN),
        "a wide result that fits must demote back to the machine form"
    );

    // The constructor encoding is `Int`'s OWN — Lean's two constructors, in Lean's order — and
    // the literal is definitionally it, so either spelling denotes the same value of the same
    // `Int`, and the literal stays the compact canonical form.
    assert_eq!(
        ctx.get_constructors("Int")
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        vec!["Int_ofNat", "Int_negSucc"],
        "`Int` is Lean's `ofNat | negSucc`"
    );
    assert!(
        defeq_for_test(&ctx, &intl(-5), &app(g("Int_negSucc"), natl(4))),
        "`-5` is definitionally `Int.negSucc 4`"
    );
    assert!(
        defeq_for_test(&ctx, &intl(5), &app(g("Int_ofNat"), natl(5))),
        "`5 : Int` is definitionally `Int.ofNat 5`"
    );
    assert!(
        !defeq_for_test(&ctx, &intl(-5), &app(g("Int_negSucc"), natl(5))),
        "…and exactly: `Int.negSucc 5` is `-6`, not `-5`"
    );
    // One encoding, under one spelling: no bare `ofNat`/`negSucc` second copy.
    let names = ["negSucc", "ofNat"];
    assert_eq!(
        absent(&ctx, &names).len(),
        names.len(),
        "the constructor encoding exists once, as `Int_ofNat`/`Int_negSucc`, but also \
         registers {:?}",
        names
            .iter()
            .filter(|n| ctx.get_global(n).is_some())
            .collect::<Vec<_>>()
    );
}

/// T10-004 · THE tier-10 rejection. Lean's `Int` is `inductive Int | ofNat | negSucc` and
/// PROVES its ring and order laws; LOGOS's `Int` is an inductive with ZERO constructors plus
/// sixteen axioms. So `signOf` — a case split on an integer — has no LOGOS form at all, and
/// `Int.natAbs`/`Int.toNat`/`Nat.cast` have nothing to recurse over.
#[test]
fn an_int_can_be_case_split_the_way_lean_case_splits_on_it() {
    let ctx = std_ctx();

    // What DOES hold today, and must keep holding: the two types are genuinely distinct, so
    // the `Nat ↦ Int` conflation is caught rather than silent.
    assert!(
        !is_subtype(&ctx, &nat(), &g("Int")),
        "Nat is not a subtype of Int"
    );
    assert!(
        !is_subtype(&ctx, &g("Int"), &nat()),
        "Int is not a subtype of Nat"
    );
    assert_eq!(
        ctx.get_constructors("Nat")
            .iter()
            .map(|(c, _)| *c)
            .collect::<Vec<_>>(),
        vec!["Zero", "Succ"],
        "Nat is a genuine Peano inductive"
    );
    // A `Match` on an `Int` scrutinee is refused LOUDLY (zero cases expected) rather than
    // accepted and stuck — the one mercy of the zero-constructor encoding.
    let case_split = mat(intl(-5), lam("_", g("Int"), nat()), vec![g("Zero")]);
    assert!(
        infer_type(&ctx, &case_split).is_err(),
        "a Match on Int must be refused, not silently accepted"
    );

    // THE CAPABILITY. `def signOf (i : Int) : Int := match i with | Int.ofNat 0 => 0 | …`
    // needs `Int` to have constructors to eliminate. It has none.
    let ctors: Vec<&str> = ctx
        .get_constructors("Int")
        .iter()
        .map(|(c, _)| *c)
        .collect();
    assert!(
        !ctors.is_empty(),
        "Lean's Int is an INDUCTIVE (ofNat / negSucc) that can be matched on and inducted \
         over; LOGOS's Int is an opaque carrier with zero constructors, so `signOf`, \
         `Int.natAbs`, `Int.toNat` and every proof by cases on an integer are unstatable: {ctors:?}"
    );
}

/// T10-018 · A SILENT WRONG ANSWER, the worst outcome in the corpus. Lean 4's `/` and `%` on
/// `Int` are `Int.ediv`/`Int.emod` — EUCLIDEAN, remainder always non-negative. LOGOS's
/// `div`/`mod` are Rust's `checked_div`/`checked_rem` — TRUNCATED toward zero, remainder
/// carrying the dividend's sign. They agree on every non-negative dividend, which is exactly
/// why testing misses it; and where Lean totalises division by zero, LOGOS leaves the term
/// stuck with no error at all.
#[test]
fn int_division_and_remainder_follow_leans_euclidean_convention() {
    let ctx = std_ctx();

    // Lean 4's `Int.ediv` / `Int.emod`, transcribed. Pinned first against the exact values
    // the corpus case states, so the oracle itself cannot drift.
    fn lean_ediv(a: i64, b: i64) -> i64 {
        if b == 0 {
            return 0;
        }
        let (q, r) = (a / b, a % b);
        if r < 0 {
            if b > 0 {
                q - 1
            } else {
                q + 1
            }
        } else {
            q
        }
    }
    fn lean_emod(a: i64, b: i64) -> i64 {
        if b == 0 {
            return a;
        }
        let r = a % b;
        if r < 0 {
            if b > 0 {
                r + b
            } else {
                r - b
            }
        } else {
            r
        }
    }
    for (a, b, q, r) in [
        (-7i64, 2i64, -4i64, 1i64),
        (7, 2, 3, 1),
        (7, -2, -3, 1),
        (-7, -2, 4, 1),
        (5, 0, 0, 5),
    ] {
        assert_eq!(lean_ediv(a, b), q, "Lean: ({a}) / ({b}) = {q}");
        assert_eq!(lean_emod(a, b), r, "Lean: ({a}) % ({b}) = {r}");
    }

    // The whole sign grid, including the divide-by-zero column Lean totalises.
    let mut wrong: Vec<String> = Vec::new();
    for a in -20i64..=20 {
        for b in -6i64..=6 {
            for (op, want) in [("div", lean_ediv(a, b)), ("mod", lean_emod(a, b))] {
                let got = normalize(&ctx, &bin(op, intl(a), intl(b)));
                if got != intl(want) {
                    let shown = match &got {
                        Term::Lit(Literal::Int(x)) => format!("{x}"),
                        _ => "STUCK".to_string(),
                    };
                    wrong.push(format!("{op}({a}, {b}): Lean {want}, LOGOS {shown}"));
                }
            }
        }
    }

    // `example (a b : Int) (h : b ≠ 0) : 0 ≤ a % b := Int.emod_nonneg a h` — the defining
    // property of the Euclidean remainder, checked wherever LOGOS answers at all.
    let mut sign_violations: Vec<String> = Vec::new();
    for a in -20i64..=20 {
        for b in [-6i64, -3, -2, -1, 1, 2, 3, 6] {
            if let Term::Lit(Literal::Int(r)) = normalize(&ctx, &bin("mod", intl(a), intl(b))) {
                if r < 0 {
                    sign_violations.push(format!("mod({a}, {b}) = {r} < 0"));
                }
            }
        }
    }

    assert!(
        wrong.is_empty() && sign_violations.is_empty(),
        "Lean's `/` and `%` on Int are Euclidean and total; LOGOS's div/mod truncate toward \
         zero and get stuck on a zero divisor. {} value disagreements (first 12: {:?}) and {} \
         violations of `0 ≤ a % b` (first 6: {:?})",
        wrong.len(),
        &wrong[..wrong.len().min(12)],
        sign_violations.len(),
        &sign_violations[..sign_violations.len().min(6)]
    );
}

// ==== SECTION: Bool — the one primitive that maps across ====

/// T10-008 · `Bool` is the prelude's own two-constructor inductive, and every comparison
/// primitive and every `Int` order axiom is stated about ITS `true`/`false`. The trap is the
/// registration order: LOGOS registers `true` then `false`, Lean declares `false` then
/// `true`, and `Match` is POSITIONAL — so a translator that keeps Lean's order emits a term
/// that type-checks and computes the negation of what the source said.
#[test]
fn bool_is_the_preludes_own_two_constructor_inductive() {
    let ctx = std_ctx();

    assert_eq!(
        ctx.get_constructors("Bool")
            .iter()
            .map(|(c, _)| *c)
            .collect::<Vec<_>>(),
        vec!["true", "false"],
        "the prelude registers `true` FIRST — the opposite of Lean's `false`-then-`true`, \
         and `Match` case order follows registration, not the source"
    );

    // `def toNat (b : Bool) : Nat := match b with | false => 0 | true => 1`, emitted in
    // REGISTRATION order: cases[0] is the `true` branch.
    let to_nat = lam(
        "b",
        g("Bool"),
        mat(var("b"), lam("_", g("Bool"), nat()), vec![natl(1), natl(0)]),
    );
    assert!(
        defeq_for_test(
            &ctx,
            &normalize(&ctx, &app(to_nat.clone(), g("true"))),
            &natl(1)
        ),
        "toNat true = 1"
    );
    assert!(
        defeq_for_test(&ctx, &normalize(&ctx, &app(to_nat, g("false"))), &natl(0)),
        "toNat false = 0"
    );

    // `example : ((3 : Int) ≤ 4) := by decide` — the comparison primitives return the
    // PRELUDE's bare `true`/`false` globals, which is what ties arithmetic computation to
    // the order axioms (`a ≤ b` is `Eq Bool (le a b) true`).
    assert_eq!(
        normalize(&ctx, &bin("le", intl(3), intl(4))),
        g("true"),
        "le 3 4 = true"
    );
    assert_eq!(
        normalize(&ctx, &bin("lt", intl(4), intl(3))),
        g("false"),
        "lt 4 3 = false"
    );
    assert_eq!(
        normalize(&ctx, &bin("ge", intl(4), intl(4))),
        g("true"),
        "ge 4 4 = true"
    );
    assert!(
        ctx.is_constructor("true") && ctx.is_constructor("false"),
        "the values `le` returns must be the prelude's own Bool constructors"
    );

    // `example : decide (2 < 3) = true := rfl` — the `Prop`→`Bool` bridge, present in both
    // systems. There is no `eqNat : Nat → Nat → Bool`; the decidable instance is the route.
    let decide_eq = |a: Term, b: Term| {
        apps(
            g("decide"),
            &[
                eq_at(nat(), a.clone(), b.clone()),
                apps(g("decEqNat"), &[a, b]),
            ],
        )
    };
    assert_eq!(
        normalize(&ctx, &decide_eq(natl(2), natl(2))),
        g("true"),
        "decide (2 = 2) = true"
    );
    assert_eq!(
        normalize(&ctx, &decide_eq(natl(2), natl(3))),
        g("false"),
        "decide (2 = 3) = false"
    );

    // `Bool` and `Prop` stay separate, as in Lean — collapsing them would make `if`
    // untranslatable.
    assert_eq!(
        infer_type(&ctx, &g("Bool")).expect("Bool has a sort"),
        Term::Sort(Universe::Type(0)),
        "Bool is a Type 0 inductive, not a Prop"
    );
}

// ==== SECTION: Text and Float — carriers with nothing attached ====

/// T10-005 · Lean's `String` is a structure over `List Char`, and its literals are definitionally
/// their `String.mk` spelling. LOGOS carries both halves: the prelude registers `String` as that
/// structure over `List Char` (with `Char`), and the Lean reader elaborates every Lean 4.14 string
/// literal to exactly that spelling, `String.mk [Char.ofNat c₁, …]` — the ORIGINAL T10-005 export
/// below. LOGOS's own `Text` literal is a different carrier: an atom with literal equality and
/// nothing else, which is all a `Text` needs and all this test asks of it.
#[test]
fn a_string_literal_is_its_character_list_spelling() {
    let ctx = std_ctx();

    // The residual semantics, which really does work: the literal injects, types, and
    // compares. `example : "hello" = "hello" := rfl` / `"hello" ≠ "world"`.
    for s in ["hello", "", "line\none\ttab", "λ ∀ 日本"] {
        assert_eq!(
            infer_type(&ctx, &textl(s)).expect("a Text literal type-checks"),
            g("Text"),
            "the literal {s:?} must type-check as Text"
        );
    }
    assert!(
        defeq_for_test(&ctx, &textl("hello"), &textl("hello")),
        "\"hello\" = \"hello\""
    );
    assert!(
        !defeq_for_test(&ctx, &textl("hello"), &textl("world")),
        "\"hello\" ≠ \"world\""
    );
    // Escapes must be DECODED, so the one-byte newline and the two-character spelling differ.
    assert!(
        !defeq_for_test(&ctx, &textl("\n"), &textl("\\n")),
        "a decoded newline is not the two-character escape"
    );

    // `def echo (s : String) : String := s` — an opaque parameter passed along translates.
    let echo = lam("s", g("Text"), var("s"));
    assert_eq!(
        normalize(&ctx, &app(echo, textl("hi"))),
        textl("hi"),
        "an opaque Text parameter passes through"
    );

    // THE CAPABILITY, half one: `String` is a structure over a character list, so `.data` of a
    // `String.mk` computes — `example : "ab".data = ['a', 'b'] := rfl` — in both kernels.
    let missing = absent(&ctx, &["String", "String_mk", "String_data", "Char"]);
    assert!(
        missing.is_empty(),
        "Lean's `String` layer must be registered: {missing:?}"
    );
    assert_eq!(
        ctx.get_constructors("String")
            .iter()
            .map(|(c, _)| *c)
            .collect::<Vec<_>>(),
        ["String_mk"],
        "`String` is a structure: exactly one constructor"
    );
    let chars = app(g("List"), g("Char"));
    let lctx = ctx.extend("l", chars.clone());
    let data = app(g("String_data"), app(g("String_mk"), var("l")));
    assert!(
        defeq_for_test(&lctx, &data, &var("l")),
        "`(String.mk l).data ≡ l`"
    );
    let claim = Term::Let {
        name: "h".to_string(),
        ty: Box::new(apps(g("Eq"), &[chars.clone(), data.clone(), var("l")])),
        value: Box::new(apps(g("refl"), &[chars.clone(), var("l")])),
        body: Box::new(var("h")),
    };
    assert_eq!(
        double_check(&lctx, &claim),
        DoubleCheck::Agreed,
        "…by `rfl`, in both kernels"
    );
    assert!(infer_type(&lctx, &claim).is_ok());
    let mctx = lctx.extend("m", chars.clone());
    assert!(
        !defeq_for_test(&mctx, &data, &var("m")),
        "…and only to its own list: `(String.mk l).data` is not an unrelated `m`"
    );

    // Half two: Lean's string LITERAL is that spelling. The original T10-005 roots, exported by
    // Lean 4.14.0, import through the production reader as `String.mk` applied to the literal's
    // code points — decoded (`\n` is one code point, 10), never a `Literal::Text`.
    let export = include_str!("fixtures/lean_scale/original/T10-005.strings.ndjson");
    let (imported, _) = logicaffeine_kernel::interface::lean::import(export)
        .unwrap_or_else(|e| panic!("the original T10-005 export must import: {e:?}"));
    for (root, text) in [
        ("greeting", "hello"),
        ("empty", ""),
        ("escaped", "line\none\ttab"),
        ("unicode", "λ ∀ 日本"),
    ] {
        let key = format!("/s:\"T10_005\"/s:\"{root}\"");
        let body = imported
            .get_definition_body(&key)
            .unwrap_or_else(|| panic!("`T10_005.{root}` is published with its body"));
        assert_eq!(
            lean_string_code_points(body),
            Some(text.chars().map(u32::from).collect::<Vec<_>>()),
            "`T10_005.{root}` must be `String.mk` of the code points of {text:?}, got {body}"
        );
    }
    // The spelling is read, not assumed: an altered code point is a different list. (That the
    // KERNEL keeps two different literals apart is `lean_scale_string_literals::
    // a_literal_is_never_equal_to_a_different_list_of_code_points`, in both kernels.)
    let greeting = imported
        .get_definition_body("/s:\"T10_005\"/s:\"greeting\"")
        .unwrap()
        .clone();
    let misspelt =
        replace_first_code_point(&greeting, 104, 106).expect("`h` is the first code point");
    assert_eq!(
        lean_string_code_points(&misspelt),
        Some("jello".chars().map(u32::from).collect::<Vec<_>>())
    );
    assert_ne!(misspelt, greeting, "\"jello\" is not \"hello\"");
}

/// The head constant of `t`'s application spine and its arguments.
fn spine(t: &Term) -> (&Term, Vec<&Term>) {
    let mut args = Vec::new();
    let mut head = t;
    while let Term::App(f, a) = head {
        args.push(a.as_ref());
        head = f;
    }
    args.reverse();
    (head, args)
}

fn is_constant(t: &Term, key: &str) -> bool {
    matches!(t, Term::Global(n) | Term::Const { name: n, .. } if n == key)
}

/// The code points of Lean 4.14's literal spelling `String.mk [Char.ofNat c₁, …, Char.ofNat cₙ]`,
/// or `None` when `t` is not exactly that spelling.
fn lean_string_code_points(t: &Term) -> Option<Vec<u32>> {
    let (head, args) = spine(t);
    if !is_constant(head, "/s:\"String\"/s:\"mk\"") || args.len() != 1 {
        return None;
    }
    let mut points = Vec::new();
    let mut list = args[0];
    loop {
        let (head, args) = spine(list);
        if is_constant(head, "/s:\"List\"/s:\"nil\"") && args.len() == 1 {
            return Some(points);
        }
        if !is_constant(head, "/s:\"List\"/s:\"cons\"") || args.len() != 3 {
            return None;
        }
        let (char_head, char_args) = spine(args[1]);
        if !is_constant(char_head, "/s:\"Char\"/s:\"ofNat\"") || char_args.len() != 1 {
            return None;
        }
        let Term::Lit(Literal::Nat(n)) = char_args[0] else {
            return None;
        };
        points.push(u32::try_from(n.to_u64()?).ok()?);
        list = args[2];
    }
}

/// `t` with the first `Char.ofNat from` replaced by `Char.ofNat to`.
fn replace_first_code_point(t: &Term, from: u64, to: u64) -> Option<Term> {
    match t {
        Term::Lit(Literal::Nat(n)) if n.to_u64() == Some(from) => {
            Some(Term::Lit(Literal::Nat(BigInt::from_u64(to))))
        }
        Term::App(f, a) => match replace_first_code_point(f, from, to) {
            Some(f) => Some(app(f, (**a).clone())),
            None => replace_first_code_point(a, from, to).map(|a| app((**f).clone(), a)),
        },
        _ => None,
    }
}

/// T10-016 · The prelude declares no `Text` operation. Lean's `++`, `length`, `get`,
/// `isPrefixOf` and `decEq` are all defined by recursion over `String.data`; LOGOS instead carries
/// LITERAL PRIMITIVES for `Text_append`, `Text_length`, `Text_beq` and `Text_isPrefixOf` in the
/// reducer, bound to a declaration of that name made at its CANONICAL type. So declaring
/// `Text_append : Text → Text → Text` computes on literals and the source's `rfl` holds; the same
/// name declared at any other type stays inert, so the primitive can never be reached through a
/// type it does not have.
#[test]
fn string_concatenation_computes() {
    let mut ctx = std_ctx();

    // No declared global anywhere in the prelude is an operation over `Text`. A proposition that
    // mentions `Text` (an equation lemma of a definition over `Syntax`, whose names are `Text`) is
    // no operation.
    let is_proposition = |ty: &Term| matches!(infer_type(&ctx, ty), Ok(Term::Sort(u)) if u == Universe::Prop || u == Universe::SProp);
    let text_ops: Vec<String> = ctx
        .iter_declarations()
        .chain(ctx.iter_definitions().map(|(n, t, _)| (n, t)))
        .filter(|(_, ty)| mentions(ty, "Text") && !is_proposition(ty))
        .map(|(n, _)| n.to_string())
        .collect();

    assert!(
        text_ops.is_empty(),
        "the prelude declares no operation over `Text`: {text_ops:?}"
    );
    let cat = bin("Text_append", textl("ab"), textl("cd"));
    assert!(
        infer_type(&ctx, &cat).is_err(),
        "…so `\"ab\" ++ \"cd\"` is not even a term until `Text_append` is declared"
    );

    // Declared at its CANONICAL type, the name binds the kernel's literal primitive.
    let text = g("Text");
    ctx.add_declaration(
        "Text_append",
        pi("_", text.clone(), pi("_", text.clone(), text)),
    );
    assert_eq!(
        infer_type(&ctx, &cat).expect("the declared append type-checks"),
        g("Text"),
        "declaring the operation is accepted by the kernel…"
    );
    assert_eq!(
        normalize(&ctx, &cat),
        textl("abcd"),
        "…and it computes on literals: the declaration binds the primitive"
    );

    // THE NEGATIVE: the same name at a type the primitive does not have stays inert, so the
    // literal primitive is never reached through a mistyped declaration.
    let mut misdeclared = std_ctx();
    misdeclared.add_declaration("Text_append", pi("_", g("Text"), pi("_", g("Text"), nat())));
    assert_eq!(
        normalize(&misdeclared, &cat),
        cat,
        "`Text_append : Text → Text → Nat` must not compute to a `Text` literal"
    );

    // THE CAPABILITY. `example : "ab" ++ "cd" = "abcd" := rfl`.
    assert!(
        defeq_for_test(&ctx, &cat, &textl("abcd")),
        "Lean proves `\"ab\" ++ \"cd\" = \"abcd\"` by rfl, and so must LOGOS once \
         `Text_append` is declared at its canonical type"
    );
}

/// T10-007 · `Float` is a labelled atom: the type and the literal exist, and nothing else
/// does — no arithmetic, no comparison, no axiom, no reduction rule. Strictly less than an
/// axiomatised type, which at least has laws. And its definitional equality is Rust's IEEE
/// `PartialEq` on `f64`, which disagrees with Lean's propositional equality in BOTH
/// directions: `def_eq` is not reflexive at `NaN`, and it identifies `0.0` with `-0.0`.
#[test]
fn float_has_arithmetic_and_leans_equality() {
    let ctx = std_ctx();

    // The literal really does inject, with the exact `f64` payload. `def half : Float := 0.5`
    for x in [0.5f64, 0.1, -1.0e-3] {
        assert_eq!(
            infer_type(&ctx, &floatl(x)).expect("a Float literal type-checks"),
            g("Float"),
            "the literal {x} must type-check as a Float"
        );
    }
    // A `Float`-typed signature with no arithmetic survives: `def identity (x : Float) : Float`
    let ident = lam("x", g("Float"), var("x"));
    assert_eq!(
        infer_type(&ctx, &ident).expect("a Float signature type-checks"),
        pi("x", g("Float"), g("Float")),
        "a Float-typed signature translates"
    );
    // Reusing the `Int` primitives is refused, which is the right refusal.
    assert!(
        infer_type(&ctx, &bin("add", floatl(1.5), floatl(2.5))).is_err(),
        "`add : Int → Int → Int` applied to Floats must be ill-typed"
    );

    let mut gaps: Vec<String> = Vec::new();

    // `def sum : Float := 1.5 + 2.5` / `def isSmall (x : Float) : Bool := x < 1.0`. Lean's
    // Float operations are opaque `@[extern]` constants, but they EXIST and `native_decide`
    // evaluates them. LOGOS declares none.
    let float_ops: Vec<String> = ctx
        .iter_declarations()
        .chain(ctx.iter_definitions().map(|(n, t, _)| (n, t)))
        .filter(|(_, ty)| mentions(ty, "Float"))
        .map(|(n, _)| n.to_string())
        .collect();
    if float_ops.is_empty() {
        gaps.push(
            "no Float operation is declared at all — not one of add/sub/mul/div/lt/le".into(),
        );
    }

    // NaN: Lean proves `x = x` by `rfl` for every Float `x`. LOGOS's literal equality is
    // `==` on `f64`, so definitional equality is NOT REFLEXIVE on a well-typed term, and the
    // rejection prints two identical sides.
    let nan = floatl(f64::NAN);
    if !defeq_for_test(&ctx, &nan, &nan) {
        let refl_nan = app(
            lam("h", eq_at(g("Float"), nan.clone(), nan.clone()), g("Zero")),
            apps(g("refl"), &[g("Float"), nan.clone()]),
        );
        gaps.push(format!(
            "def_eq is not reflexive at NaN — `refl Float nan : Eq Float nan nan` is REFUSED, \
             and the rejection prints two identical sides: {:?}",
            infer_type(&ctx, &refl_nan).err()
        ));
    }

    // Signed zero, the other direction and the dangerous one: `0.0` and `-0.0` are DISTINCT
    // Lean Floats (`0.0 = -0.0` is not provable by rfl; only `==` is true of them), and
    // LOGOS identifies them definitionally — so LOGOS PROVES a proposition Lean does not.
    let (pz, nz) = (floatl(0.0), floatl(-0.0));
    if defeq_for_test(&ctx, &pz, &nz) {
        let proof = app(
            lam("h", eq_at(g("Float"), pz.clone(), nz.clone()), g("Zero")),
            apps(g("refl"), &[g("Float"), pz.clone()]),
        );
        gaps.push(format!(
            "0.0 and -0.0 are definitionally identified — `refl : Eq Float 0.0 (-0.0)` \
             type-checks ({}), proving a Float equation Lean does not",
            infer_type(&ctx, &proof).is_ok()
        ));
    }

    assert!(
        gaps.is_empty(),
        "Lean's Float has arithmetic, comparison and a propositional equality that separates \
         0.0 from -0.0 and relates NaN to itself. LOGOS: {gaps:?}"
    );
}

/// Whether a type mentions a given global anywhere — used to enumerate the operations a
/// carrier actually has.
fn mentions(t: &Term, name: &str) -> bool {
    match t {
        Term::Global(n) | Term::Var(n) => n == name,
        Term::Const { name: n, .. } => n == name,
        Term::App(a, b) => mentions(a, name) || mentions(b, name),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => mentions(param_type, name) || mentions(body_type, name),
        Term::Lambda {
            param_type, body, ..
        } => mentions(param_type, name) || mentions(body, name),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            mentions(discriminant, name)
                || mentions(motive, name)
                || cases.iter().any(|c| mentions(c, name))
        }
        Term::Fix { body, .. } => mentions(body, name),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| mentions(b, name)),
        Term::Let {
            ty, value, body, ..
        } => mentions(ty, name) || mentions(value, name) || mentions(body, name),
        _ => false,
    }
}

// ==== SECTION: types absent from the prelude ====

/// T10-006 · `Char` is absent, and the loss is not recoverable. Lean's `Char` is
/// `{ val : UInt32 // val.isValidChar }`, and the encoding is blocked twice over: `UInt32` is
/// absent too, and so is `Subtype`. The `Literal` enum has no character variant either, so a
/// character cannot even be INJECTED.
#[test]
fn a_character_literal_has_a_type_to_land_in() {
    let ctx = std_ctx();

    // The two re-encodings the guards name, and why each is a dead end.
    // A `Nat` encoding makes `Char` and `Nat` the SAME type, so `Nat → Char` becomes
    // `Nat → Nat` and every argument swap type-checks.
    assert_eq!(
        infer_type(&ctx, &natl(97)).expect("97 : Nat"),
        nat(),
        "`'a' ↦ Lit(Nat(97))` would type-check — and would be indistinguishable from 97"
    );
    // A one-character `Text` encoding is opaque: `Text` has zero constructors, so
    // `Char.val`/`Char.toNat` have nothing to project.
    assert!(
        ctx.get_constructors("Text").is_empty(),
        "a one-character Text cannot carry Char.val — Text is opaque"
    );

    // THE CAPABILITY, and the two layers that block the honest encoding.
    let missing = absent(&ctx, &["Char", "UInt32", "Subtype"]);
    assert!(
        missing.is_empty(),
        "Lean's Char is a decidable, ordered type with a validity invariant \
         (`Char.ofNat 0xD800` clamps to '\\0'); LOGOS has neither the type nor a character \
         Literal variant, and the natural encoding `{{ val : UInt32 // val.isValidChar }}` is \
         blocked at every layer. Unregistered: {missing:?}"
    );
}

/// T10-009 · No bounded integer type of any width. LOGOS's arithmetic is exact and unbounded
/// BY CONSTRUCTION — an overflowing `i64` op falls through to the arbitrary-precision path
/// rather than truncating — so there is no configuration of the prelude in which a `Nat`/`Int`
/// encoding of `UInt64` accidentally agrees at the one place that matters.
#[test]
fn machine_words_wrap_at_their_width() {
    let ctx = std_ctx();

    // The positive fact that makes the re-encoding hopeless: LOGOS deliberately never wraps.
    assert_eq!(
        normalize(&ctx, &bin("add", intl(i64::MAX), intl(1))),
        Term::Lit(int_lit(
            BigInt::parse_decimal("9223372036854775808").unwrap()
        )),
        "i64::MAX + 1 promotes to exact arbitrary precision — it does not wrap to i64::MIN"
    );
    assert_eq!(
        normalize(&ctx, &bin("mul", intl(i64::MAX), intl(2))),
        Term::Lit(int_lit(
            BigInt::parse_decimal("18446744073709551614").unwrap()
        )),
        "…and neither does multiplication"
    );

    // THE CAPABILITY. `example : (255 : UInt8) + 1 = 0 := by decide`, and the same for
    // `UInt64`, `USize`, and the mod-2^N literal reduction `(256 : UInt8) = 0`.
    let missing = absent(&ctx, &["UInt8", "UInt16", "UInt32", "UInt64", "USize"]);
    assert!(
        missing.is_empty(),
        "Lean's machine words wrap at their width — `(255 : UInt8) + 1 = 0`, \
         `(0 : UInt8) - 1 = 255`, and a literal is reduced mod 2^N at elaboration. LOGOS has \
         no bounded integer type at all: {missing:?}"
    );
}

/// T10-010 · THE NAME IS THE TRAP. `BVec n` and `BitVec n` have the same arity and the same
/// kind, and nothing else in common: Lean's `BitVec w` is `Fin (2^w)` — a NUMBER with modular
/// arithmetic, shifts and `toNat` — while `BVec n` is a `Bit` cons-list whose complete
/// operation surface is four bitwise recursions.
#[test]
fn a_bitvec_adds_modulo_two_to_the_width() {
    let ctx = std_ctx();

    // What `BVec` actually is: a cons-list of a two-constructor `Bit`.
    assert_eq!(
        ctx.get_constructors("BVec")
            .iter()
            .map(|(c, _)| *c)
            .collect::<Vec<_>>(),
        vec!["BVNil", "BVCons"],
        "BVec is a cons-list, not a machine word"
    );
    assert_eq!(
        ctx.get_constructors("Bit")
            .iter()
            .map(|(c, _)| *c)
            .collect::<Vec<_>>(),
        vec!["B0", "B1"],
        "its elements are a two-constructor inductive"
    );
    // Its COMPLETE operation surface — bitwise only. No add, no mul, no shift, no toNat. Each
    // operation also carries the equation lemma derived from it (`bv_and_eq_def`, Lean's
    // `eq_def`): a theorem ABOUT the operation, owned by it, and not an operation of its own.
    let bv_ops: Vec<&str> = {
        let mut v: Vec<&str> = ctx
            .iter_definitions()
            .map(|(n, _, _)| n)
            .filter(|n| n.starts_with("bv_") && !ctx.is_derived_aux(n))
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        bv_ops,
        vec!["bv_and", "bv_not", "bv_or", "bv_xor"],
        "BVec has four bitwise operations and no arithmetic of any kind"
    );
    for op in &bv_ops {
        assert_eq!(
            ctx.derived_aux_names(op),
            Some(&[format!("{op}_eq_def")][..]),
            "`{op}`'s only derived constant is its equation lemma"
        );
    }
    assert!(
        absent(
            &ctx,
            &["bv_add", "bv_mul", "bv_shl", "bv_ushr", "bv_toNat", "bv_ofNat"]
        )
        .len()
            == 6,
        "…confirmed by name: none of Lean's BitVec arithmetic has a counterpart"
    );

    // THE CAPABILITY. `example : (255 : BitVec 8) + 1 = 0 := by decide`, `<<<`, `>>>`,
    // `.toNat`, `BitVec.ofNat`, and `x ++ y : BitVec (m + n)`.
    let missing = absent(&ctx, &["BitVec"]);
    assert!(
        missing.is_empty(),
        "Lean's `BitVec w` is a number with arithmetic mod 2^w, shifts, extraction and Nat \
         conversion; LOGOS's similarly-named `BVec` is a Bit cons-list with four bitwise \
         recursions, so it is not a translation target. Unregistered: {missing:?}"
    );
}

/// T10-011 · No `Array`, no array literal variant, no indexing primitive. The dangerous
/// substitution is `Array ↦ TList`: the `foldl`/`map`/`size` interface lines up, so most
/// sources translate without a type error, while `Array.get` goes from O(1)-with-a-bound-proof
/// to an O(i) recursion that must decide what to do out of range — and every `Array` lemma
/// stops holding.
#[test]
fn an_array_literal_indexes_in_constant_time() {
    let ctx = std_ctx();

    // The nearest neighbour, and why it is not one: `TList` is a cons-list whose constructors
    // carry the element TYPE as an explicit leading argument, which Lean's `#[…]` does not.
    assert_eq!(
        ctx.get_constructors("TList")
            .iter()
            .map(|(c, _)| *c)
            .collect::<Vec<_>>(),
        vec!["TNil", "TCons"],
        "the only parametric sequence in the prelude is a cons-list"
    );
    assert_eq!(
        ctx.get_global("TList").expect("TList is registered"),
        &pi(
            "A",
            Term::Sort(Universe::Type(0)),
            Term::Sort(Universe::Type(0))
        ),
        "and it is pinned to Type 0 → Type 0, so an Array of types or of Props has no home"
    );

    // THE CAPABILITY. `def small : Array Nat := #[1, 2, 3]`, `small.size = 3`,
    // `small[0] = 1`, `(small.push 4)[3] = 4`, `small[7]? = none`.
    let missing = absent(&ctx, &["Array", "Array_get", "Array_push", "Array_size"]);
    assert!(
        missing.is_empty(),
        "Lean's Array has a literal syntax, O(1) `getElem` taking a bound proof, `push` with \
         sharing, and a lemma set (`size_push`, `getElem_set_eq`, …) none of which holds of a \
         cons-list. LOGOS has no array type, no array Literal variant and no indexing \
         primitive: {missing:?}"
    );
}

/// T10-012 · Lean's `List` is not in the prelude. `TList` is a different global with
/// different constructor names and a `Type 0` pin; `EList` is monomorphic in `Entity`. The
/// prelude's own comment says so: the `E`-prefix exists to avoid the user-space `List` a REPL
/// program is expected to declare.
#[test]
fn leans_list_is_in_the_prelude_with_nil_before_cons() {
    let ctx = std_ctx();

    // `EList` is `List Entity` and nothing else — it is not a parametric list.
    assert_eq!(
        ctx.get_constructors("EList")
            .iter()
            .map(|(c, _)| *c)
            .collect::<Vec<_>>(),
        vec!["ENil", "ECons"],
        "EList is the monomorphic list the induction tactic runs over"
    );
    assert_eq!(
        ctx.get_global("EList").expect("EList is registered"),
        &Term::Sort(Universe::Type(0)),
        "…with no type parameter at all"
    );
    // Unlike `Bool`, `TList`'s registration order MATCHES Lean's `nil`-then-`cons`, so a
    // translator that keeps source order is safe here and catastrophic there.
    let tlist_ctors: Vec<&str> = ctx
        .get_constructors("TList")
        .iter()
        .map(|(c, _)| *c)
        .collect();
    assert_eq!(
        tlist_ctors[0], "TNil",
        "the empty constructor is registered first"
    );

    // THE CAPABILITY. `def small : List Nat := [1, 2, 3]`, `List.rec`, `List.append`, and the
    // ~900 `List` lemmas every Lean source leans on.
    let missing = absent(&ctx, &["List", "nil", "cons", "List_rec"]);
    assert!(
        missing.is_empty(),
        "Lean's List is universe-polymorphic (`Type u → Type u`), ships `nil`/`cons`/`List.rec` \
         and a lemma library; LOGOS's nearest neighbours are a differently-named `TList` \
         pinned to Type 0 and a monomorphic `EList` over Entity. Unregistered: {missing:?}"
    );
}

/// T10-013 · Six of the most-used types in Lean core — `Option`, `Prod`, `Sum`, `Sigma`,
/// `Subtype`, `Fin` — and not one is registered. Each must be emitted, each emission is a
/// fresh lemma-free type, and `Fin` is blocked behind two of the others plus a `Nat` order
/// relation the prelude does not have (every order axiom is about `Int`).
#[test]
fn the_six_structural_types_lean_core_ships_are_registered() {
    let ctx = std_ctx();

    // `Fin`'s honest encoding needs a `Nat` `<`. Every order axiom in the prelude is stated
    // about `Int`, and the comparison primitives are `Int → Int → Bool`, so there is no
    // `Nat` order relation to build a bound proof from.
    assert_eq!(
        ctx.get_global("lt").expect("the prelude declares lt"),
        &pi("_", g("Int"), pi("_", g("Int"), g("Bool"))),
        "the only `<` in the prelude is on Int — `i < n` for a Fin bound has no target"
    );
    assert!(
        infer_type(&ctx, &bin("lt", natl(1), natl(3))).is_err(),
        "…and it is ill-typed on Nat arguments"
    );

    // THE CAPABILITY. `(3, true) : Nat × Bool` with eta, `some 5`, `Sum.inl 3`,
    // `⟨2, 0⟩ : Σ n, Fin (n+1)`, `⟨1, _⟩ : {n : Nat // 0 < n}`, and `Fin 0` uninhabited.
    let missing = absent(
        &ctx,
        &[
            "Option", "some", "none", "Prod", "Sum", "Sigma", "Subtype", "Fin",
        ],
    );
    assert!(
        missing.is_empty(),
        "A mathematician cannot state much without pairs, dependent pairs, subtypes and \
         bounded indices; Lean ships all six with lemmas, DecidableEq and Sort-polymorphism, \
         and LOGOS registers none of them (and `add_structure` would pin the three record-like \
         ones to Type 0). Unregistered: {missing:?}"
    );
}

// ==== SECTION: overloading ====

/// T10-014 · In Lean a numeral's meaning is an INSTANCE: `(5 : α)` is `@OfNat.ofNat α 5 inst`,
/// and with `instance (n : Nat) : OfNat Mod7 n := ⟨Mod7.mk (n % 7)⟩`, `(9 : Mod7) = Mod7.mk 2 :=
/// rfl`. The kernel types a literal by its VARIANT, so the elaborator is where a numeral takes
/// the type the context asks for: it resolves the declared `OfNat` dictionary FIRST, and only
/// with none declared falls back to LOGOS's own record injection. The dangerous outcome — a
/// numeral elaborated against an expected type silently keeping its `Nat` value (`Mod7.mk 9`) —
/// must not survive a declared instance.
#[test]
fn a_numeral_takes_the_type_the_context_asks_for() {
    let mut ctx = std_ctx();
    let mut gaps: Vec<String> = Vec::new();

    // There is no `OfNat` class and no default-instance machinery to represent the
    // `@[default_instance]` that makes a bare numeral a `Nat`.
    let cls = absent(&ctx, &["OfNat", "OfNat_ofNat", "instOfNatNat"]);
    if !cls.is_empty() {
        gaps.push(format!(
            "no OfNat class to resolve a numeral against: {cls:?}"
        ));
    }

    // `def asInt : Int := 5`. Elaborated against `Int`, the numeral must become an
    // `Int`-typed term. It comes back as a `Nat` literal, with no diagnostic.
    let against_int = surface_elaborate_against(&ctx, &natl(5), Some(&g("Int")))
        .expect("elaborating a numeral must not hard-error");
    let got = infer_type(&ctx, &against_int).expect("the elaborated numeral type-checks");
    if got != g("Int") {
        gaps.push(format!(
            "`(5 : Int)` elaborates to {against_int:?} of type {got:?} — the numeral keeps the \
             Nat variant and the expected type is silently ignored"
        ));
    }

    // `instance (n : Nat) : OfNat Mod7 n := ⟨Mod7.mk (n % 7)⟩`, so `(9 : Mod7) = Mod7.mk 2`.
    // A translator that emits the raw numeral produces `Mod7.mk 9` — well-typed, wrong value.
    ctx.add_structure("Mod7", &[], &[("rep", nat())]);
    let of_nat_mod7 = |n: Term| {
        app(
            app(
                Term::Const {
                    name: "OfNat".to_string(),
                    levels: vec![Universe::Type(0)],
                },
                g("Mod7"),
            ),
            n,
        )
    };
    let instance = lam(
        "n",
        nat(),
        apps(
            g("OfNat_mk"),
            &[
                g("Mod7"),
                var("n"),
                app(g("Mod7_mk"), apps(g("Nat_mod"), &[var("n"), natl(7)])),
            ],
        ),
    );
    let instance_ty = pi("n", nat(), of_nat_mod7(var("n")));
    assert!(
        matches!(infer_type(&ctx, &instance), Ok(ty) if is_subtype(&ctx, &ty, &instance_ty)),
        "the instance is well-typed at `Π(n : Nat). OfNat Mod7 n`"
    );
    ctx.add_instance(instance_ty, instance);
    let want = app(g("Mod7_mk"), natl(2));
    match without_panicking(|| surface_elaborate_against(&ctx, &natl(9), Some(&g("Mod7")))) {
        Err(()) => gaps.push(
            "elaborating a numeral at a user type CRASHES the kernel rather than resolving \
             or refusing"
                .into(),
        ),
        Ok(Err(e)) => gaps.push(format!(
            "`(9 : Mod7)` is refused rather than resolved: {e:?}"
        )),
        Ok(Ok(t)) if !defeq_for_test(&ctx, &t, &want) => gaps.push(format!(
            "`(9 : Mod7)` with a user OfNat instance is `Mod7.mk 2` in Lean; LOGOS elaborates \
             the numeral to {t:?}"
        )),
        Ok(Ok(_)) => {}
    }

    assert!(
        gaps.is_empty(),
        "Lean resolves every numeral through an OfNat instance, so `(5 : Int)`, `(5 : Nat)` and \
         `(9 : Mod7)` are three different values of the same surface token, and a class-\
         polymorphic `(2 : α)` is statable. LOGOS: {gaps:?}"
    );
}

/// The text/literate surface reaches the same literal layer, so the tier's spine holds there
/// too: the `Repl` door is the only public path an authored LOGOS program takes, and a
/// numeral typed through it must land in `Nat`, not `Int`.
#[test]
fn the_repl_door_agrees_with_the_kernel_on_a_numerals_type() {
    let repl = Repl::new();
    let ctx = repl.context();
    assert_eq!(
        infer_type(ctx, &natl(7)).expect("a Nat literal type-checks through the Repl context"),
        nat(),
        "the literate surface's context must carry the same Nat/Int split as the kernel's"
    );
    assert_eq!(
        ctx.get_constructors("Int")
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        vec!["Int_ofNat", "Int_negSucc"],
        "and the same Int — Lean's `ofNat | negSucc`"
    );
}

// ==== SECTION: a crash found while translating this tier ====

/// NOT a corpus case — a defect the tier turned up. `reduce_step`'s ι-rule indexes the case
/// list with the constructor index it extracted, UNCHECKED, and `extract_constructor`'s Peano
/// bridge reports index 1 (`Succ`) for ANY `Nat` literal regardless of the inductive the
/// `Match` was written for. So a `Match` with fewer than two cases whose discriminant is a
/// `Nat` literal does not fail to type-check — it PANICS the kernel out of `normalize`, which
/// is a public entry point that must be total on every input. The realistic route is a
/// one-field structure's generated projection meeting a numeral during `def_eq`, which is
/// exactly how `a_numeral_takes_the_type_the_context_asks_for` reached it.
#[test]
fn a_match_with_too_few_cases_never_panics_the_kernel() {
    let mut ctx = std_ctx();
    let mut crashes: Vec<&str> = Vec::new();

    // Direct: one case, a `Nat` literal discriminant. `Nat(0)` takes index 0 and survives;
    // `Nat(9)` takes index 1 and walks off the end of a one-element vector.
    let one_case = |d: Term| Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(lam("_", g("Unit"), nat())),
        cases: vec![g("Zero")],
    };
    if without_panicking(|| normalize(&ctx, &one_case(natl(9)))).is_err() {
        crashes.push("normalize of a one-case Match on a positive Nat literal");
    }

    // Realistic: a registered one-field structure's projection — a well-formed global — meets
    // a well-formed `Nat` literal. Both terms type-check on their own; the crash is in the
    // conversion machinery, where heterogeneous comparison is routine.
    ctx.add_structure("Wrapper", &[], &[("unwrap", nat())]);
    if without_panicking(|| normalize(&ctx, &app(g("Wrapper_unwrap"), natl(9)))).is_err() {
        crashes.push("normalize of a structure projection applied to a Nat literal");
    }
    if without_panicking(|| defeq_for_test(&ctx, &natl(9), &app(g("Wrapper_mk"), natl(2)))).is_err()
    {
        crashes.push("def_eq between a Nat literal and a one-field structure value");
    }

    assert!(
        crashes.is_empty(),
        "the kernel must answer or refuse, never abort: {crashes:?}"
    );
}
