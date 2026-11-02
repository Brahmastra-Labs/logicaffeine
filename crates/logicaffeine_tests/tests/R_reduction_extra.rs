//! Lane R_reduction's own locks — reduction, evaluation and honest normalisation.
//!
//! The corpus test for a case is ONE case. These are the neighbours it does not ask about:
//! the negative that a weak implementation would also satisfy, the boundary one rung either
//! side of a door, the narrow path measured against the wide one, and the second engine
//! measured against the first.
//!
//! The spine of the file is Lean 4's `Int.ediv`/`Int.emod`. Its SPECIFICATION — written once,
//! here, and implemented independently by each engine — is:
//!
//! ```text
//!   b ≠ 0  ⟹  a = b·q + r   ∧   0 ≤ r < |b|      (the unique such pair)
//!   b = 0  ⟹  q = 0  ∧  r = a                     (total, and `a = 0·0 + a` still holds)
//! ```
//!
//! Every oracle below is derived from THAT, never transcribed from an implementation's
//! formula: an oracle that is a copy of the code cannot catch the code being wrong.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, eval_bool, infer_type, int_lit, normalize, normalize_at, normalize_checked,
    normalize_checked_at, BigInt, Context, Literal, Term, Transparency, Unreduced,
};

// ==== helpers ====

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn bin(op: &str, a: Term, b: Term) -> Term {
    app(app(g(op), a), b)
}
fn intl(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
/// An integer literal forced onto the WIDE path: `Literal::BigInt` carrying a value that
/// `int_lit` would have canonicalised to `Int`. Not a term the kernel produces — the point is
/// to drive the exact path with an input the fast path would otherwise have taken.
fn wide(n: i64) -> Term {
    Term::Lit(Literal::BigInt(BigInt::from_i64(n)))
}
fn bigl(s: &str) -> Term {
    Term::Lit(Literal::BigInt(
        BigInt::parse_decimal(s).expect("decimal BigInt literal"),
    ))
}
fn natl(n: i64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_i64(n)))
}
fn textl(s: &str) -> Term {
    Term::Lit(Literal::Text(s.to_string()))
}
fn pi(p: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// The prelude's CARRIERS — `Bool`, `Nat`, `Int`, `Text`, spelled as the prelude spells them — and
/// nothing built on the operation names. Every name the primitive table dispatches on (`add` …
/// `gt`, `natLe`, `reduceBool`) is a checked definition or declaration in the prelude with
/// theorems resting on it, and a declared name never changes meaning under what depends on it
/// (`Context::admit_meaning`; Lean's environment is append-only). A test that must bind one of
/// those names to a different program or a different type does it here, where nothing depends on
/// the name, so what it measures is the reducer's name-keyed rules and their guards.
fn carriers_only_ctx() -> Context {
    let ty0 = || Term::Sort(logicaffeine_kernel::Universe::Type(0));
    let mut ctx = Context::new();
    ctx.add_inductive("Int", ty0());
    ctx.add_inductive("Text", ty0());
    ctx.add_inductive("Nat", ty0());
    ctx.add_constructor("Zero", "Nat", g("Nat"));
    ctx.add_constructor("Succ", "Nat", pi("_", g("Nat"), g("Nat")));
    ctx.add_inductive("Bool", ty0());
    ctx.add_constructor("true", "Bool", g("Bool"));
    ctx.add_constructor("false", "Bool", g("Bool"));
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:?}",
        ctx.refused_declarations()
    );
    ctx
}

/// A prelude name something depends on keeps its meaning: rebinding it is refused and recorded,
/// and the prelude's own answer for `probe` is unchanged.
fn assert_prelude_name_keeps_its_meaning(name: &str, ty: Term, body: Option<Term>, probe: &Term) {
    let mut ctx = std_ctx();
    let before = normalize(&ctx, probe);
    match body {
        Some(body) => ctx.add_definition(name.to_string(), ty, body),
        None => ctx.add_declaration(name, ty),
    }
    assert!(
        ctx.refused_declarations().iter().any(|(n, _)| n == name),
        "rebinding the prelude's `{name}` must be refused, since theorems rest on it: {:?}",
        ctx.refused_declarations()
    );
    assert_eq!(
        normalize(&ctx, probe),
        before,
        "…and `{probe}` still means what it meant"
    );
}

/// A rewrite cycle of period `n` through the `@[simp]` registry: `prefix0 x = prefix1 x`, …,
/// `prefix(n-1) x = prefix0 x`, over axioms `prefixK : Nat → Nat`. Every δ door refuses a cycle
/// (Lean's rule, enforced at `Context::declare`), so the registry is the door that still builds a
/// reduction with no normal form — and it is the reducer's cycle detector, not the gateway, that
/// has to answer for it.
fn simp_cycle(ctx: &mut Context, prefix: &str, n: usize) -> Vec<String> {
    let names: Vec<String> = (0..n).map(|i| format!("{prefix}{i}")).collect();
    for f in &names {
        ctx.add_declaration(f, pi("_", g("Nat"), g("Nat")));
    }
    let x = || Term::Var("x".to_string());
    for i in 0..n {
        let (l, r) = (&names[i], &names[(i + 1) % n]);
        let rule = format!("{l}_rot");
        let stmt = pi(
            "x",
            g("Nat"),
            app(app(app(g("Eq"), g("Nat")), app(g(l), x())), app(g(r), x())),
        );
        ctx.add_declaration(&rule, stmt);
        ctx.add_simp_lemma(&rule)
            .unwrap_or_else(|e| panic!("the registry takes `{rule}`: {e:?}"));
    }
    names
}

/// `a := b` then `b := a`: the SECOND definition closes a δ-cycle and must be refused at the
/// gateway, with the refusal recorded; the first stays an unpaid forward reference that nothing
/// can type, so no proof can be written about it.
fn assert_delta_cycle_refused(ctx: &mut Context, a: &str, b: &str) {
    ctx.add_definition(a.to_string(), g("Nat"), g(b));
    ctx.add_definition(b.to_string(), g("Nat"), g(a));
    assert!(
        ctx.get_definition_body(b).is_none(),
        "`{b} := {a}` closes a δ-cycle and must be refused"
    );
    assert!(
        ctx.refused_declarations().iter().any(|(n, _)| n == b),
        "…and the refusal must be recorded: {:?}",
        ctx.refused_declarations()
    );
    assert!(
        infer_type(ctx, &g(a)).is_err(),
        "`{a}` waits on a name that never came, so it has no type"
    );
}

/// `(λx:Nat. x x) (λx:Nat. x x)` — the period-ONE β-cycle: the rule FIRES and leads back to where
/// it started, so the honest answer is "no normal form", never "already normal".
fn omega() -> Term {
    let half = lam(
        "x",
        g("Nat"),
        app(Term::Var("x".to_string()), Term::Var("x".to_string())),
    );
    app(half.clone(), half)
}

/// The scrutinee a stuck term finally rests on: the discriminant of its head `match`, followed
/// through nested head matches. A term with no head `match` rests on itself.
fn resting_scrutinee(t: &Term) -> &Term {
    let mut cur = t;
    while let Term::Match { discriminant, .. } = cur {
        cur = discriminant;
    }
    cur
}

fn int_of(t: &Term) -> Option<i64> {
    match t {
        Term::Lit(Literal::Int(n)) => Some(*n),
        Term::Lit(Literal::BigInt(n)) => n.to_i64(),
        _ => None,
    }
}
fn bool_of(t: &Term) -> Option<bool> {
    match t {
        Term::Global(n) if n == "true" => Some(true),
        Term::Global(n) if n == "false" => Some(false),
        _ => None,
    }
}

/// The Euclidean pair, found by SEARCH from the defining property above rather than by
/// transcribing a formula. Panics rather than guessing if the window is too small.
fn euclid_by_search(a: i64, b: i64) -> (i64, i64) {
    if b == 0 {
        return (0, a);
    }
    let lo = -(a.abs() + 2);
    let hi = a.abs() + 2;
    for q in lo..=hi {
        let r = a - b * q;
        if 0 <= r && r < b.abs() {
            return (q, r);
        }
    }
    panic!("no Euclidean pair for ({a}, {b}) inside the search window");
}

/// The grid every arithmetic test below walks: both signs of both operands, both magnitudes
/// either side of the quotient boundary, and the zero divisor Lean totalises.
fn grid() -> impl Iterator<Item = (i64, i64)> {
    (-40i64..=40).flat_map(|a| (-9i64..=9).map(move |b| (a, b)))
}

// =============================================================================
// T10-018 · Euclidean division — the value grid
// =============================================================================

/// The search oracle is pinned against the five values the corpus case states in prose, so a
/// wrong oracle cannot quietly agree with a wrong implementation.
#[test]
fn the_euclidean_oracle_reproduces_the_values_the_case_states() {
    for (a, b, q, r) in [
        (-7i64, 2i64, -4i64, 1i64),
        (7, 2, 3, 1),
        (7, -2, -3, 1),
        (-7, -2, 4, 1),
        (5, 0, 0, 5),
    ] {
        assert_eq!(
            euclid_by_search(a, b),
            (q, r),
            "Lean: ({a}) / ({b}) = {q} rem {r}"
        );
    }
}

#[test]
fn div_and_mod_compute_the_euclidean_pair_on_the_whole_sign_grid() {
    let ctx = std_ctx();
    let mut wrong: Vec<String> = Vec::new();
    for (a, b) in grid() {
        let (q, r) = euclid_by_search(a, b);
        for (op, want) in [("div", q), ("mod", r)] {
            let got = normalize(&ctx, &bin(op, intl(a), intl(b)));
            if got != intl(want) {
                wrong.push(format!("{op}({a},{b}): want {want}, got {got}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} disagreements: {:?}",
        wrong.len(),
        &wrong[..wrong.len().min(10)]
    );
}

/// THE NEGATIVE. Euclidean and truncated division agree on every non-negative dividend — the
/// reason a truncating implementation survives casual testing. So it is not enough that the
/// grid matches: at least one cell must be a cell where the two conventions DIFFER, and the
/// truncated answer must be refused there.
#[test]
fn the_truncated_answer_is_refused_wherever_the_two_conventions_differ() {
    let ctx = std_ctx();
    let mut differing = 0usize;
    for (a, b) in grid() {
        if b == 0 {
            continue;
        }
        let (eq, er) = euclid_by_search(a, b);
        let (tq, tr) = (a / b, a % b);
        if (tq, tr) == (eq, er) {
            continue;
        }
        differing += 1;
        assert_ne!(
            normalize(&ctx, &bin("div", intl(a), intl(b))),
            intl(tq),
            "div({a},{b}) must not be the TRUNCATED quotient {tq}"
        );
        assert_ne!(
            normalize(&ctx, &bin("mod", intl(a), intl(b))),
            intl(tr),
            "mod({a},{b}) must not be the TRUNCATED remainder {tr}"
        );
    }
    assert!(
        differing >= 200,
        "the grid must actually straddle the two conventions; only {differing} cells differ"
    );
}

/// `example (a b : Int) (h : b ≠ 0) : 0 ≤ a % b := Int.emod_nonneg a h`, and its upper half
/// `a % b < |b|` — decided by the KERNEL's own `le`/`lt`, not by Rust, so this is a statement
/// about the object language rather than about the test.
#[test]
fn the_remainder_is_non_negative_and_below_the_divisor_by_the_kernels_own_order() {
    let ctx = std_ctx();
    let mut bad: Vec<String> = Vec::new();
    for (a, b) in grid() {
        if b == 0 {
            continue;
        }
        let r = bin("mod", intl(a), intl(b));
        let non_negative = normalize(&ctx, &bin("le", intl(0), r.clone()));
        let below = normalize(&ctx, &bin("lt", r.clone(), intl(b.abs())));
        if bool_of(&non_negative) != Some(true) {
            bad.push(format!("0 ≤ mod({a},{b}) is {non_negative}"));
        }
        if bool_of(&below) != Some(true) {
            bad.push(format!("mod({a},{b}) < {} is {below}", b.abs()));
        }
    }
    assert!(
        bad.is_empty(),
        "{} violations: {:?}",
        bad.len(),
        &bad[..bad.len().min(10)]
    );
}

/// The other half of the defining property, evaluated INSIDE the kernel: `b·(a/b) + a%b = a`.
/// It holds at `b = 0` too, which is exactly why Lean's totalisation is `(0, a)` and not
/// something tidier.
#[test]
fn the_division_identity_reconstructs_the_dividend_inside_the_kernel() {
    let ctx = std_ctx();
    let mut bad: Vec<String> = Vec::new();
    for (a, b) in grid() {
        let rebuilt = bin(
            "add",
            bin("mul", intl(b), bin("div", intl(a), intl(b))),
            bin("mod", intl(a), intl(b)),
        );
        let got = normalize(&ctx, &rebuilt);
        if got != intl(a) {
            bad.push(format!("b·(a/b) + a%b at ({a},{b}) = {got}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{} violations: {:?}",
        bad.len(),
        &bad[..bad.len().min(10)]
    );
}

/// Lean's `/` and `%` are TOTAL: `Int.ediv_zero : a / 0 = 0` and `Int.emod_zero : a % 0 = a`.
/// A stuck term is not the same answer — it is no answer, and it makes `decide` decline on a
/// goal Lean discharges.
#[test]
fn division_by_zero_is_total_and_computes_rather_than_sticking() {
    let ctx = std_ctx();
    for a in [-9i64, -1, 0, 1, 9, i64::MAX, i64::MIN] {
        assert_eq!(
            normalize(&ctx, &bin("div", intl(a), intl(0))),
            intl(0),
            "({a}) / 0 = 0"
        );
        assert_eq!(
            normalize(&ctx, &bin("mod", intl(a), intl(0))),
            intl(a),
            "({a}) % 0 = {a}"
        );
    }
}

// =============================================================================
// T10-018 · the boundaries — the narrow path against the wide one
// =============================================================================

/// A fast path that disagrees with the exact path is the whole bug class over again. Every
/// cell of the grid is computed twice: once with `Literal::Int` operands (the i64 path) and
/// once with `Literal::BigInt` operands carrying the SAME values (the arbitrary-precision
/// path). They must be the same term.
#[test]
fn the_i64_path_and_the_bigint_path_compute_the_same_quotient_and_remainder() {
    let ctx = std_ctx();
    let mut bad: Vec<String> = Vec::new();
    for (a, b) in grid() {
        for op in ["div", "mod"] {
            let narrow = normalize(&ctx, &bin(op, intl(a), intl(b)));
            let broad = normalize(&ctx, &bin(op, wide(a), wide(b)));
            if narrow != broad {
                bad.push(format!("{op}({a},{b}): i64 {narrow} vs BigInt {broad}"));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} disagreements: {:?}",
        bad.len(),
        &bad[..bad.len().min(10)]
    );
}

/// `i64::MIN / -1` overflows a machine division. It must PROMOTE to the exact path and answer,
/// not get stuck and not wrap.
#[test]
fn the_one_overflowing_division_promotes_instead_of_sticking() {
    let ctx = std_ctx();
    let want = BigInt::from_i64(i64::MIN).abs();
    assert_eq!(
        normalize(&ctx, &bin("div", intl(i64::MIN), intl(-1))),
        Term::Lit(Literal::BigInt(want)),
        "i64::MIN / -1 = 2^63, which only the exact path can represent"
    );
    assert_eq!(
        normalize(&ctx, &bin("mod", intl(i64::MIN), intl(-1))),
        intl(0),
        "i64::MIN % -1 = 0"
    );
}

/// Beyond `i64` in every sign combination, checked by the defining property rather than by a
/// table: the kernel's own `add`/`mul` must rebuild the dividend, and the remainder must sit
/// in `[0, |b|)`.
#[test]
fn the_euclidean_law_holds_beyond_i64_in_every_sign_combination() {
    let ctx = std_ctx();
    let huge = "1180591620717411303449"; // 2^70 + 25, far outside i64
    let mid = "4611686018427387913"; // ~2^62 + 9, inside i64 but near the edge
    let mut bad: Vec<String> = Vec::new();
    for asign in ["", "-"] {
        for bsign in ["", "-"] {
            for (a_s, b_s) in [(huge, mid), (mid, huge), (huge, huge), (huge, "7")] {
                let a = format!("{asign}{a_s}");
                let b = format!("{bsign}{b_s}");
                let (at, bt) = (bigl(&a), bigl(&b));
                let q = normalize(&ctx, &bin("div", at.clone(), bt.clone()));
                let r = normalize(&ctx, &bin("mod", at.clone(), bt.clone()));
                let rebuilt = normalize(
                    &ctx,
                    &bin("add", bin("mul", bt.clone(), q.clone()), r.clone()),
                );
                // The CANONICAL literal for `a`: `int_lit` demotes to `Int` whenever the value
                // fits, and every computed result is canonicalised, so the expected value must
                // be too — `bigl` deliberately builds the non-canonical form to drive the wide
                // path, and comparing against it would fail for the wrong reason.
                let canonical_a = Term::Lit(int_lit(BigInt::parse_decimal(&a).expect("decimal")));
                if rebuilt != canonical_a {
                    bad.push(format!("({a})/({b}): b·q + r = {rebuilt}, not {a}"));
                }
                let non_negative = normalize(&ctx, &bin("le", intl(0), r.clone()));
                if bool_of(&non_negative) != Some(true) {
                    bad.push(format!("({a}) % ({b}) = {r} is negative"));
                }
                let abs_b = bigl(b_s);
                let below = normalize(&ctx, &bin("lt", r.clone(), abs_b));
                if bool_of(&below) != Some(true) {
                    bad.push(format!("({a}) % ({b}) = {r} is not below |{b}|"));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} violations: {:?}",
        bad.len(),
        &bad[..bad.len().min(8)]
    );
}

/// THE NEGATIVE for the operand kinds. `Nat` and `Int` are different types; the `Int`
/// operations must not fire on a `Nat` literal, or `sub 3 8` at `Nat` would compute `-5` and
/// the truncating `Nat` subtraction the prelude emits would be bypassed.
///
/// The operations are Lean's checked definitions over `Int`'s two constructors, and δβ is
/// untyped, so a name may UNFOLD on a foreign operand. What it may never do is decide: the
/// reduct is either the term itself or an `Int` `match` resting on the foreign literal, with no
/// ι step taken on it — and both kernels refuse to type the term at all.
#[test]
fn the_int_primitives_refuse_nat_and_text_operands() {
    let ctx = std_ctx();
    for op in ["add", "sub", "mul", "div", "mod", "le", "lt", "ge", "gt"] {
        for (label, x, y, foreign) in [
            ("Nat literals", natl(7), natl(2), vec![natl(7), natl(2)]),
            ("mixed literals", intl(7), natl(2), vec![natl(2)]),
            (
                "Text literals",
                textl("7"),
                textl("2"),
                vec![textl("7"), textl("2")],
            ),
        ] {
            let t = bin(op, x, y);
            let n = normalize_checked(&ctx, &t)
                .unwrap_or_else(|e| panic!("`{op}` on {label} must finish, got {e:?}"));
            assert!(
                n == t || foreign.contains(resting_scrutinee(&n)),
                "`{op}` must stay stuck on {label}: it may unfold, but only to a `match` resting \
                 on the foreign literal, got {n}"
            );
            assert!(
                infer_type(&ctx, &t).is_err(),
                "`{op}` on {label} is ill-typed"
            );
            assert!(
                logicaffeine_kernel::recheck(&ctx, &t).is_err(),
                "…in the re-checker too"
            );
        }
    }
}

// =============================================================================
// T10-018 · the second engine
// =============================================================================

/// `native_decide`'s evaluator is a SECOND implementation of the same primitives, and the
/// `ofReduceBool` axiom trusts it to agree with reduction. Sandwich each quotient between
/// `le` and `ge` against the value reduction computed, and pin the neighbour as false — so an
/// evaluator that merely declined, or that answered a neighbouring value, is caught.
#[test]
fn the_native_evaluator_computes_the_same_euclidean_pair_as_the_reducer() {
    let ctx = std_ctx();
    let mut bad: Vec<String> = Vec::new();
    for (a, b) in grid() {
        for op in ["div", "mod"] {
            let t = bin(op, intl(a), intl(b));
            let want = int_of(&normalize(&ctx, &t)).expect("the reducer must answer");
            let at = |rel: &str, v: i64| eval_bool(&ctx, &bin(rel, t.clone(), intl(v)));
            if at("le", want) != Some(true) || at("ge", want) != Some(true) {
                bad.push(format!(
                    "{op}({a},{b}): the evaluator does not agree it is {want}"
                ));
            }
            if at("ge", want + 1) != Some(false) || at("le", want - 1) != Some(false) {
                bad.push(format!(
                    "{op}({a},{b}): the evaluator also accepts a neighbour of {want}"
                ));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} disagreements: {:?}",
        bad.len(),
        &bad[..bad.len().min(10)]
    );
}

/// The simp constant folder certifies goals (`try_dsimp_solve_conclude` accepts a proof when
/// `simp::check_goal` succeeds), so a folder that still truncates would CERTIFY a false
/// equation the moment the reducer went Euclidean. Both directions are pinned: simp must
/// prove the Euclidean equation and must refuse the truncated one.
#[test]
fn the_simp_folder_certifies_the_euclidean_equation_and_refuses_the_truncated_one() {
    use logicaffeine_kernel::simp;
    // `simp::check_goal` reads the REFLECTED goal (`SApp`/`SName`/`SLit`), which is the form
    // `try_try_simp_reduce` hands it.
    let slit = |n: i64| app(g("SLit"), intl(n));
    let sname = |s: &str| app(g("SName"), textl(s));
    let sapp = |f: Term, x: Term| app(app(g("SApp"), f), x);
    let seq = |l: Term, r: Term| sapp(sapp(sapp(sname("Eq"), sname("Int")), l), r);
    let sbin = |op: &str, a: i64, b: i64| sapp(sapp(sname(op), slit(a)), slit(b));

    // The control, so a folder that simply refuses everything cannot satisfy the negatives.
    assert!(
        simp::check_goal(&seq(sbin("add", 2, 3), slit(5))),
        "simp folds `2 + 3 = 5`"
    );
    assert!(
        !simp::check_goal(&seq(sbin("add", 2, 3), slit(6))),
        "…and refuses `2 + 3 = 6`"
    );

    let mut bad: Vec<String> = Vec::new();
    for (a, b) in grid() {
        if b == 0 {
            continue;
        }
        let (q, r) = euclid_by_search(a, b);
        let (tq, tr) = (a / b, a % b);
        for (op, e, t) in [("div", q, tq), ("mod", r, tr)] {
            if !simp::check_goal(&seq(sbin(op, a, b), slit(e))) {
                bad.push(format!("simp cannot fold {op}({a},{b}) = {e}"));
            }
            if e != t && simp::check_goal(&seq(sbin(op, a, b), slit(t))) {
                bad.push(format!("simp PROVED the truncated {op}({a},{b}) = {t}"));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} disagreements: {:?}",
        bad.len(),
        &bad[..bad.len().min(10)]
    );
}

// =============================================================================
// A defect the corpus does not name — a builtin outranking a real definition
// =============================================================================

/// `try_primitive_reduce` fires on a NAME. If a context defines that name with a body, the
/// kernel must compute what the BODY says: a primitive rule that outranks δ makes the kernel
/// disagree with the definition it type-checked against, which is a silent wrong answer with
/// no diagnostic anywhere. The prelude's own `add`…`gt` are Lean's checked definitions over
/// `Int`'s constructors, with the integer laws resting on them, so they keep their meaning; the
/// shadowing definition is bound where nothing depends on `add`.
#[test]
fn a_definition_with_a_body_outranks_the_builtin_computation_rule() {
    let int = g("Int");
    let binop = pi("_", int.clone(), pi("_", int.clone(), int.clone()));
    let zero_add = lam("a", int.clone(), lam("b", int.clone(), intl(0)));
    assert_prelude_name_keeps_its_meaning(
        "add",
        binop.clone(),
        Some(zero_add.clone()),
        &bin("add", intl(2), intl(3)),
    );

    let mut ctx = carriers_only_ctx();
    let bool_op = pi("_", int.clone(), pi("_", int.clone(), g("Bool")));
    // The builtin-dispatched siblings, declared at the types their rules compute at.
    ctx.add_declaration("le", bool_op);
    ctx.add_declaration("mul", binop.clone());
    ctx.add_declaration("div", binop.clone());
    // `def add (a b : Int) : Int := 0` — deliberately wrong arithmetic, deliberately shadowing.
    ctx.add_definition("add".to_string(), binop, zero_add);
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:?}",
        ctx.refused_declarations()
    );
    assert_eq!(
        normalize(&ctx, &bin("add", intl(2), intl(3))),
        intl(0),
        "the kernel must compute what the definition says, not what the builtin says"
    );
    // THE DIFFERENTIAL that makes this a defect rather than a preference: the trusted
    // evaluator ALREADY prefers the definition (`eval.rs` consults `get_definition_body`
    // before `prim_name`, and `recheck.rs`'s `whnf` δ-unfolds the head before reaching
    // `try_builtin`), so before this gate the reducer was the only engine of the three that
    // answered `5` — and `native_decide` is TRUSTED to agree with `normalize`.
    assert_eq!(
        eval_bool(&ctx, &bin("le", bin("add", intl(2), intl(3)), intl(0))),
        Some(true),
        "`add 2 3 = 0` under this definition, so `le (add 2 3) 0` is true in the evaluator"
    );
    assert_eq!(
        normalize(&ctx, &bin("le", bin("add", intl(2), intl(3)), intl(0))),
        g("true"),
        "…and the reducer must reach the same verdict, or the two engines disagree"
    );
    // The declared-but-undefined siblings still compute: the gate costs a declaration nothing.
    assert_eq!(normalize(&ctx, &bin("mul", intl(2), intl(3))), intl(6));
    assert_eq!(normalize(&ctx, &bin("div", intl(-7), intl(2))), intl(-4));
    // A shadowing definition of a TEXT primitive is honoured the same way.
    let mut tctx = std_ctx();
    with_text_ops(&mut tctx);
    let text = g("Text");
    tctx.add_definition(
        "Text_append".to_string(),
        pi("_", text.clone(), pi("_", text.clone(), text.clone())),
        lam(
            "a",
            text.clone(),
            lam("b", text, Term::Var("a".to_string())),
        ),
    );
    assert_eq!(
        normalize(&tctx, &bin("Text_append", textl("ab"), textl("cd"))),
        textl("ab"),
        "a defined `Text_append` means what its body says"
    );
}

// =============================================================================
// T10-016 · Text operations that COMPUTE
// =============================================================================

fn with_text_ops(ctx: &mut Context) {
    let text = g("Text");
    let bool_t = g("Bool");
    ctx.add_declaration(
        "Text_append",
        pi("_", text.clone(), pi("_", text.clone(), text.clone())),
    );
    ctx.add_declaration("Text_length", pi("_", text.clone(), g("Nat")));
    ctx.add_declaration(
        "Text_beq",
        pi("_", text.clone(), pi("_", text.clone(), bool_t.clone())),
    );
    ctx.add_declaration(
        "Text_isPrefixOf",
        pi("_", text.clone(), pi("_", text, bool_t)),
    );
}

#[test]
fn text_operations_compute_on_literals() {
    let mut ctx = std_ctx();
    with_text_ops(&mut ctx);

    assert_eq!(
        normalize(&ctx, &bin("Text_append", textl("ab"), textl("cd"))),
        textl("abcd")
    );
    assert_eq!(
        normalize(&ctx, &bin("Text_append", textl(""), textl("cd"))),
        textl("cd")
    );
    assert_eq!(
        normalize(&ctx, &bin("Text_append", textl("ab"), textl(""))),
        textl("ab")
    );
    // Associativity and the unit law, computed rather than axiomatised.
    let left = bin(
        "Text_append",
        bin("Text_append", textl("a"), textl("b")),
        textl("c"),
    );
    let right = bin(
        "Text_append",
        textl("a"),
        bin("Text_append", textl("b"), textl("c")),
    );
    assert_eq!(normalize(&ctx, &left), textl("abc"));
    assert_eq!(normalize(&ctx, &right), textl("abc"));
    assert!(
        defeq_for_test(&ctx, &left, &right),
        "`rfl` proves the associativity instance"
    );

    // `String.length` counts CHARACTERS, not bytes — the trap a `len()` on a Rust `String`
    // walks straight into.
    assert_eq!(
        normalize(&ctx, &app(g("Text_length"), textl("abcd"))),
        natl(4)
    );
    assert_eq!(normalize(&ctx, &app(g("Text_length"), textl(""))), natl(0));
    assert_eq!(
        normalize(&ctx, &app(g("Text_length"), textl("héllo"))),
        natl(5),
        "five characters, six bytes"
    );
    assert_eq!(
        normalize(&ctx, &app(g("Text_length"), textl("日本語"))),
        natl(3),
        "three characters, nine bytes"
    );

    assert_eq!(
        normalize(&ctx, &bin("Text_beq", textl("ab"), textl("ab"))),
        g("true")
    );
    assert_eq!(
        normalize(&ctx, &bin("Text_beq", textl("ab"), textl("ba"))),
        g("false")
    );
    assert_eq!(
        normalize(&ctx, &bin("Text_isPrefixOf", textl("ab"), textl("abcd"))),
        g("true")
    );
    assert_eq!(
        normalize(&ctx, &bin("Text_isPrefixOf", textl("abcd"), textl("ab"))),
        g("false")
    );
    assert_eq!(
        normalize(&ctx, &bin("Text_isPrefixOf", textl(""), textl("ab"))),
        g("true")
    );

    // The declared type is what the kernel infers, so the computed value is well-typed.
    assert_eq!(
        infer_type(&ctx, &bin("Text_append", textl("ab"), textl("cd"))).expect("types"),
        g("Text")
    );
}

/// THE NEGATIVE. The rules fire on Text LITERALS and nothing else: an open term, an Int
/// operand and a partially-applied operation must all stay exactly where they are.
#[test]
fn text_operations_stay_stuck_on_anything_that_is_not_two_text_literals() {
    let mut ctx = std_ctx();
    with_text_ops(&mut ctx);
    let open = ctx.extend("s", g("Text"));

    for stuck in [
        bin("Text_append", Term::Var("s".to_string()), textl("cd")),
        bin("Text_append", textl("ab"), Term::Var("s".to_string())),
        bin("Text_append", intl(1), intl(2)),
        bin("Text_append", textl("ab"), intl(2)),
        bin("Text_beq", textl("ab"), natl(2)),
        app(g("Text_length"), Term::Var("s".to_string())),
        app(g("Text_length"), intl(3)),
    ] {
        assert_eq!(normalize(&open, &stuck), stuck, "must stay stuck: {stuck}");
    }
    // A partial application is a value, not a redex.
    let partial = app(g("Text_append"), textl("ab"));
    assert_eq!(
        normalize(&ctx, &partial),
        partial,
        "a partially applied Text op does not fire"
    );
}

// =============================================================================
// DEFECT-9 / T11-005 · honest normalisation
// =============================================================================

/// The honest channel must distinguish all three ways a reduction can fail to finish from the
/// one way it can succeed.
#[test]
fn normalize_checked_separates_a_normal_form_from_every_way_of_not_reaching_one() {
    let mut ctx = std_ctx();
    assert_eq!(
        normalize_checked(&ctx, &g("Zero")),
        Ok(g("Zero")),
        "a genuine normal form"
    );

    assert_delta_cycle_refused(&mut ctx, "loop_a", "loop_b");
    // The rewrite loop lives in its OWN context. A registered `@[simp]` rule shuts the bounded
    // conversion probes for every term (`bounded_head_conversion`), so registering each rung of
    // the chain below falls back to conversion by normalising the rest of the chain — quadratic
    // in the chain's length, and a measurement of that gate rather than of the honest channel.
    let mut spun = std_ctx();
    let spin = simp_cycle(&mut spun, "loopR", 2);
    assert!(
        matches!(
            normalize_checked(&spun, &app(g(&spin[0]), g("Zero"))),
            Err((_, Unreduced::Cyclic))
        ),
        "a rewrite cycle has NO normal form and must be reported as such, not approximated"
    );

    // A long but TERMINATING δ-chain is not a failure: a budget that stops it is
    // indistinguishable from a normal form, which is the whole defect.
    const CHAIN: usize = 20_000;
    for i in 0..CHAIN {
        let next = if i + 1 == CHAIN {
            g("Zero")
        } else {
            g(&format!("chain{}", i + 1))
        };
        ctx.add_definition(format!("chain{i}"), g("Nat"), next);
    }
    assert_eq!(
        normalize_checked(&ctx, &g("chain0")),
        Ok(g("Zero")),
        "20 000 δ-steps terminate, so the answer is a normal form and must be reported as one"
    );
}

/// A term deeper than the kernel's door is refused BEFORE the first clone, and the refusal is
/// reported rather than returned as "already normal".
#[test]
fn a_term_too_deep_to_walk_is_reported_and_not_mistaken_for_a_normal_form() {
    let ctx = std_ctx();
    // Just past the kernel's door (1600) and nowhere near what a recursive `Drop` of the term
    // itself would cost, so the test measures the DOOR rather than the native stack.
    let deep = (0..2_000).fold(g("Zero"), |acc, _| app(g("Succ"), acc));
    match normalize_checked(&ctx, &deep) {
        Err((_, Unreduced::TooDeep)) => {}
        other => panic!(
            "a 2000-deep spine must be refused, not answered: {:?}",
            other.is_ok()
        ),
    }
    // One rung INSIDE the door still answers — the refusal is a boundary, not a blanket.
    let shallow = (0..100).fold(g("Zero"), |acc, _| app(g("Succ"), acc));
    assert_eq!(normalize_checked(&ctx, &shallow), Ok(shallow.clone()));
}

/// `normalize` hands a cycling term back UNCHANGED rather than returning an arbitrary point on
/// the cycle. That is a deliberate choice and it is the only one that keeps `normalize` a
/// FUNCTION: every term on the cycle is as good an answer as any other, so returning one makes
/// `normalize(a) = b` and `normalize(b) = a`, and every conversion check built on "normalise
/// both sides and compare" inherits the instability. Pin it, and pin the consequence.
///
/// The δ door refuses the cycle this was first written over, so the cycle is built where one can
/// still be built: constants that unfold INTO `@[simp]` rewrite loops. Two points of ONE loop are
/// equal by the loop's own assumed equations, so the conversion that must be refused is the one
/// nothing assumed: between two UNRELATED loops, and between a loop and a value.
#[test]
fn normalize_stays_a_function_on_a_delta_cycle_and_certifies_nothing_about_it() {
    let mut ctx = std_ctx();
    assert_delta_cycle_refused(&mut ctx, "loop_a", "loop_b");

    let spin = simp_cycle(&mut ctx, "stay", 2);
    let other = simp_cycle(&mut ctx, "apart", 2);
    ctx.add_definition("enter_a".to_string(), g("Nat"), app(g(&spin[0]), g("Zero")));
    ctx.add_definition("enter_b".to_string(), g("Nat"), app(g(&spin[1]), g("Zero")));
    ctx.add_definition(
        "enter_c".to_string(),
        g("Nat"),
        app(g(&other[0]), g("Zero")),
    );
    let starts = [
        g("enter_a"),
        g("enter_b"),
        g("enter_c"),
        app(g(&spin[0]), g("Zero")),
        app(g(&spin[1]), g("Zero")),
    ];

    // Stable and idempotent, from every point that reaches a cycle.
    for start in &starts {
        assert!(
            matches!(normalize_checked(&ctx, start), Err((_, Unreduced::Cyclic))),
            "`{start}` reaches a rewrite loop and has no normal form"
        );
        let once = normalize(&ctx, start);
        assert_eq!(&once, start, "`{start}` is handed back unchanged");
        assert_eq!(
            normalize(&ctx, &once),
            once,
            "…and normalize is idempotent on it"
        );
    }
    // Two constants that enter UNRELATED loops must NOT be found definitionally equal by
    // normalising both: nothing finished, and no equation relates them.
    assert!(
        !defeq_for_test(&ctx, &g("enter_a"), &g("enter_c")),
        "an unfinished reduction may not be the evidence for a conversion"
    );
    assert!(
        !defeq_for_test(&ctx, &g("enter_a"), &g("Zero")),
        "…nor may it be identified with a value it never reached"
    );
    // …while a definition that actually terminates still converts.
    ctx.add_definition("settled".to_string(), g("Nat"), natl(3));
    assert!(defeq_for_test(&ctx, &g("settled"), &natl(3)));
}

// =============================================================================
// T00-013 / T02-004 · the δ-transparency axis
//
// Lean's reducibility is an ELABORATOR attribute: its KERNEL unfolds every definition
// regardless, which is why `example : secret = 5 := rfl` on an `@[irreducible] def secret` is
// refused at the source level and accepted by the kernel. So the axis has to be a SETTING on
// the reduction, not a property of the definition — and `normalize` must keep unfolding
// everything, or LOGOS would diverge from Lean's kernel in the other direction.
//
// The one thing the corpus test for these cases cannot exercise is the API, because it marks
// nothing irreducible: it registers `secret` with the same three-argument `add_definition` as
// `ordinary`. These tests exercise the mechanism the cases are about.
// =============================================================================

/// An `@[irreducible]` definition keeps its BODY and still refuses to unfold — the distinction
/// that makes `irreducible` useful rather than opaque. Routing opacity through
/// `add_declaration` instead would throw the body away and manufacture a silent axiom, which
/// is the strongest guard in T00-013's case file.
#[test]
fn an_irreducible_definition_keeps_its_body_and_stays_folded_below_kernel_transparency() {
    let mut ctx = std_ctx();
    ctx.add_definition("ordinary".to_string(), g("Nat"), natl(5));
    ctx.add_definition("secret".to_string(), g("Nat"), natl(5));
    let sealed = |n: &str| n == "secret";
    let elaborator = Transparency::Excluding(&sealed);

    // The body is still there — nothing was turned into an axiom.
    assert_eq!(
        ctx.get_definition_body("secret"),
        Some(&natl(5)),
        "opacity must not cost the body: an axiom cannot satisfy the equations the definition can"
    );

    // At the ELABORATOR's transparency `secret` stays folded and `ordinary` still unfolds.
    assert_eq!(normalize_at(&ctx, &g("secret"), elaborator), g("secret"));
    assert_eq!(normalize_at(&ctx, &g("ordinary"), elaborator), natl(5));

    // At the KERNEL's transparency both unfold — Lean's kernel does exactly this, and
    // `normalize` is the kernel's face.
    assert_eq!(normalize(&ctx, &g("secret")), natl(5));
    assert_eq!(normalize_at(&ctx, &g("secret"), Transparency::All), natl(5));

    // …and the honest channel agrees at either setting.
    assert_eq!(
        normalize_checked_at(&ctx, &g("secret"), elaborator),
        Ok(g("secret"))
    );
    assert_eq!(
        normalize_checked_at(&ctx, &g("secret"), Transparency::All),
        Ok(natl(5))
    );
}

/// THE NEGATIVE, and the reason a name-keyed shortcut cannot pass for this. Opacity must be a
/// property of the SETTING, so the same context answers differently under two transparencies
/// and identically for two structurally identical definitions under one.
#[test]
fn opacity_is_a_property_of_the_reduction_setting_and_not_of_the_name() {
    let mut ctx = std_ctx();
    // Two definitions that differ ONLY in their name, exactly as the corpus test registers them.
    ctx.add_definition("twinA".to_string(), g("Nat"), natl(7));
    ctx.add_definition("twinB".to_string(), g("Nat"), natl(7));

    // Under `All` the two are indistinguishable — anything else would be reading the name.
    for n in ["twinA", "twinB"] {
        assert_eq!(
            normalize(&ctx, &g(n)),
            natl(7),
            "`{n}` unfolds at kernel transparency"
        );
    }
    // Under an opacity set naming ONE of them, exactly that one folds.
    let only_a = |n: &str| n == "twinA";
    assert_eq!(
        normalize_at(&ctx, &g("twinA"), Transparency::Excluding(&only_a)),
        g("twinA")
    );
    assert_eq!(
        normalize_at(&ctx, &g("twinB"), Transparency::Excluding(&only_a)),
        natl(7)
    );
    // Swap the set and the answers swap with it.
    let only_b = |n: &str| n == "twinB";
    assert_eq!(
        normalize_at(&ctx, &g("twinA"), Transparency::Excluding(&only_b)),
        natl(7)
    );
    assert_eq!(
        normalize_at(&ctx, &g("twinB"), Transparency::Excluding(&only_b)),
        g("twinB")
    );
    // An empty opacity set is `All` in every observable way.
    let nothing = |_: &str| false;
    for n in ["twinA", "twinB"] {
        assert_eq!(
            normalize_at(&ctx, &g(n), Transparency::Excluding(&nothing)),
            natl(7)
        );
    }
}

/// THE BOUNDARY. Opacity must stop δ and NOTHING else: β, ζ, ι and the primitive rules all
/// still fire, and an opaque constant blocks only its own unfolding — not reduction around it.
#[test]
fn opacity_blocks_delta_only_and_reduction_continues_around_it() {
    let mut ctx = std_ctx();
    ctx.add_definition("sealed".to_string(), g("Nat"), natl(5));
    let sealed = |n: &str| n == "sealed";
    let tr = Transparency::Excluding(&sealed);

    // β still fires, and the opaque constant travels through it unchanged.
    let applied = app(
        lam("x", g("Nat"), app(g("Succ"), Term::Var("x".to_string()))),
        g("sealed"),
    );
    assert_eq!(
        normalize_at(&ctx, &applied, tr),
        app(g("Succ"), g("sealed"))
    );

    // The primitive rules still fire beside it.
    assert_eq!(
        normalize_at(&ctx, &bin("add", intl(2), intl(3)), tr),
        intl(5)
    );

    // ζ still fires.
    let zeta = Term::Let {
        name: "y".to_string(),
        ty: Box::new(g("Nat")),
        value: Box::new(natl(1)),
        body: Box::new(Term::Var("y".to_string())),
    };
    assert_eq!(normalize_at(&ctx, &zeta, tr), natl(1));

    // ι still fires on a literal scrutinee, with the opaque constant inside a branch.
    let m = Term::Match {
        discriminant: Box::new(natl(0)),
        motive: Box::new(lam("_", g("Nat"), g("Nat"))),
        cases: vec![g("sealed"), lam("k", g("Nat"), g("Nat"))],
    };
    assert_eq!(
        normalize_at(&ctx, &m, tr),
        g("sealed"),
        "ι chose the Zero branch and stopped there"
    );

    // A definition whose BODY mentions the opaque constant unfolds once and then stops.
    ctx.add_definition(
        "usesSealed".to_string(),
        g("Nat"),
        app(g("Succ"), g("sealed")),
    );
    assert_eq!(
        normalize_at(&ctx, &g("usesSealed"), tr),
        app(g("Succ"), g("sealed"))
    );
    assert_eq!(normalize(&ctx, &g("usesSealed")), app(g("Succ"), natl(5)));
}

// =============================================================================
// PERFORMANCE — measured, not guessed
//
// `normalize` is `reduce_step` to a fixpoint, and `reduce_step` walks from the ROOT every
// time. So finding the k-th redex of a term costs O(size), and a term with n nested redexes
// costs O(n²) — plus two whole-term structural comparisons per step (`reduced == current` for
// the fixpoint test and `current == tortoise` for cycle detection) and a rebuild of every
// unchanged subtree along the path.
//
// Lean does not pay this: its `whnf` reduces at the head with an explicit spine, its `Nat`
// literals are GMP-accelerated rather than peeled one `Succ` at a time, and its transparency
// modes let it stop early. This measurement is the evidence for that report — it is
// `#[ignore]`d because it is a stopwatch, and a stopwatch on a twelve-way-contended box is a
// number, not a lock.
// =============================================================================

#[test]
#[ignore = "measurement, not a lock — run with --run-ignored all"]
fn measure_how_normalize_scales_in_the_number_of_nested_redexes() {
    let ctx = std_ctx();
    // `add 1 (add 1 (… (add 1 0)))`, n deep. Every node is a redex, and the INNERMOST one is
    // the only one that can fire (the primitive rules need two literal operands), so each
    // whole-term `reduce_step` walks the entire spine to reach it.
    let build = |n: usize| (0..n).fold(intl(0), |acc, _| bin("add", intl(1), acc));
    let mut rows: Vec<(usize, u128)> = Vec::new();
    // The rungs stop at 200 (term depth ~401) because 400 (depth ~801) ABORTS THE PROCESS with
    // a stack overflow — at a depth `enter_kernel_or_refuse` ADMITS, since `MAX_TERM_DEPTH` is
    // 1600. That is a finding, not a limit of the measurement: see the report.
    for n in [50usize, 100, 150, 200] {
        let t = build(n);
        let start = std::time::Instant::now();
        let got = normalize(&ctx, &t);
        let micros = start.elapsed().as_micros();
        assert_eq!(got, intl(n as i64), "the spine must still compute");
        rows.push((n, micros));
        println!("nested redexes {n:>4}: {micros:>9} µs");
    }
    // Report the observed exponent between the first and last rung rather than asserting one:
    // the claim in the report is a measurement, and the box is shared.
    let (n0, t0) = rows[0];
    let (n1, t1) = *rows.last().unwrap();
    if t0 > 0 {
        let ratio = (t1 as f64) / (t0 as f64);
        let scale = (n1 as f64) / (n0 as f64);
        println!(
            "growth {ratio:.1}× for {scale:.1}× the redexes  →  exponent ≈ {:.2}",
            ratio.ln() / scale.ln()
        );
    }
}

// =============================================================================
// BOTH KERNELS — the half of T10-018 that cannot land in one file
//
// The de Bruijn re-checker has its own copy of the primitive arithmetic
// (`recheck.rs::try_builtin`), and `double_check` is what proves the two agree. A change to
// kernel SEMANTICS that lands in only one of them is exactly the shape the campaign's six
// identically-miscounted bugs had in reverse: here the two DISAGREE, loudly, which is the safe
// direction — `double_check` refuses rather than certifies.
//
// `recheck_reduction_differential.rs` cannot see this: every case it uses has a NON-NEGATIVE
// dividend (`div 20 3`, `mod 20 3`, `10^60 / 10^30`), and Euclidean and truncated division
// agree on every one of those. That is the same blind spot the convention bug itself lives in,
// showing up a second time in the test that was supposed to catch it.
// =============================================================================

/// `(λ(h : Eq Int (div a b) q). h) (refl Int q)` is well-typed only if BOTH kernels reduce
/// `div a b` to `q`. Every sign combination, so the one the two conventions disagree about is
/// in the set rather than adjacent to it.
#[test]
fn both_kernels_agree_on_the_euclidean_quotient_at_every_sign() {
    use logicaffeine_kernel::{double_check, DoubleCheck};
    let ctx = std_ctx();
    let apps = |f: Term, xs: &[Term]| xs.iter().fold(f, |a, x| app(a, x.clone()));
    let coercion = |ty: Term, redex: Term, result: Term| {
        let claim = apps(g("Eq"), &[ty.clone(), redex, result.clone()]);
        let refl_proof = apps(g("refl"), &[ty, result]);
        app(lam("h", claim, Term::Var("h".to_string())), refl_proof)
    };

    let mut disagreed: Vec<String> = Vec::new();
    for (a, b) in [
        (-7i64, 2i64),
        (7, 2),
        (7, -2),
        (-7, -2),
        (-1, 3),
        (1, 3),
        (5, 0),
    ] {
        for (op, want) in [
            ("div", euclid_by_search(a, b).0),
            ("mod", euclid_by_search(a, b).1),
        ] {
            let redex = bin(op, intl(a), intl(b));
            // The main kernel first, so a failure here is never blamed on the re-checker.
            assert_eq!(
                normalize(&ctx, &redex),
                intl(want),
                "the main kernel must reduce {op}({a},{b}) to {want}"
            );
            let term = coercion(g("Int"), redex, intl(want));
            if double_check(&ctx, &term) != DoubleCheck::Agreed {
                disagreed.push(format!("{op}({a},{b}) = {want}"));
            }
        }
    }
    assert!(
        disagreed.is_empty(),
        "the de Bruijn re-checker must compute Lean's TOTAL Euclidean `div`/`mod` too — \
         `recheck.rs::try_builtin` still uses `checked_div`/`checked_rem` and `BigInt::div_rem`, \
         which truncate toward zero and get stuck on a zero divisor. Written from the shared \
         specification in `reduction.rs::ediv_emod_i64`, never copied from it. Disagreements: {disagreed:?}"
    );
}

// =============================================================================
// THE `Nat` ORDER — the kernel's largest MEASURED hotspot, and the wall behind it
//
// `natLe` is a structural recursion that peels one `Succ` per unit, and a `Literal::Nat`
// enters it through the Peano bridge. So deciding `m ≤ n` costs `min(m, n)` whole-term
// reduction steps: `Char`'s validity bound (`< 0x110000`) is 1 114 112 of them against a
// 2^20 budget, which means the answer is not slow, it is UNREACHABLE — the bound is refused
// for want of arithmetic rather than for being false.
//
// Lean does not pay this: `Nat.ble`/`Nat.blt` on two literals are GMP, overridden in the
// kernel by NAME. We can do that AND be safer than Lean, because Lean's override never looks
// at the definition it is standing in for. Ours fires only when the constant is bound to
// EXACTLY the program being accelerated, so the fast answer is the slow answer by
// construction rather than by trust — and if the prelude ever redefines `natLe`, the
// acceleration silently stops applying instead of silently lying.
//
// These are the neighbours of `W_perf_extra::a_nat_order_fast_path_must_agree_with_the_unary_unfolding`,
// which pins the VALUES. These pin the things a value grid cannot see: the pairs that only a
// fast path can reach, the negative that must survive it, and the second engine.
// =============================================================================

/// The oracle, independent of both engines: the order on naturals, with the clamp the Peano
/// bridge imposes (a negative `Literal::Nat` peels to `Zero`, so it compares AS zero).
fn nat_order_oracle(op: &str, a: i64, b: i64) -> bool {
    let (a, b) = (a.max(0), b.max(0));
    match op {
        "natLe" => a <= b,
        "natLt" => a < b,
        other => panic!("not a Nat order: {other}"),
    }
}

/// THE CAPABILITY. Pairs whose unary decision costs more steps than `normalize` has, so the
/// verdict exists only if the literal path does. Both directions of both relations, so a fast
/// path cannot pass by answering one constant.
#[test]
fn the_nat_order_decides_literal_pairs_the_unary_recursion_cannot_reach() {
    let ctx = std_ctx();
    // Unicode's top scalar value against its bound — the exact pair `Char_isValid` asks about.
    for (op, m, n) in [
        ("natLt", 1_114_111i64, 1_114_112i64),
        ("natLe", 1_114_112, 1_114_111),
        ("natLe", 1_114_112, 1_114_112),
        ("natLt", 1_114_112, 1_114_112),
    ] {
        assert_eq!(
            bool_of(&normalize(&ctx, &bin(op, natl(m), natl(n)))),
            Some(nat_order_oracle(op, m, n)),
            "{op} {m} {n} needs min({m},{n}) unary steps against a 2^20 budget — only a \
             literal path can decide it, and it must decide it CORRECTLY"
        );
    }

    // Beyond every budget: 2^64, the bound `UInt64` is built over.
    let two64 = Term::Lit(Literal::Nat(
        BigInt::parse_decimal("18446744073709551616").unwrap(),
    ));
    let two64m1 = Term::Lit(Literal::Nat(
        BigInt::parse_decimal("18446744073709551615").unwrap(),
    ));
    for (op, want) in [("natLe", true), ("natLt", false)] {
        assert_eq!(
            bool_of(&normalize(&ctx, &bin(op, two64.clone(), two64.clone()))),
            Some(want),
            "{op} 2^64 2^64 — arbitrary precision, decided exactly"
        );
    }
    assert_eq!(
        bool_of(&normalize(
            &ctx,
            &bin("natLt", two64m1.clone(), two64.clone())
        )),
        Some(true)
    );
    assert_eq!(
        bool_of(&normalize(&ctx, &bin("natLt", two64.clone(), two64m1))),
        Some(false)
    );
}

/// THE VALUES. A dense grid plus every boundary a fast path is likely to get wrong, decided
/// against an oracle that is not either engine's formula — and, where the unary route can
/// still reach it, against the unary route itself.
#[test]
fn the_nat_order_agrees_with_the_unary_spine_everywhere_the_spine_can_be_reached() {
    let ctx = std_ctx();
    let peano = |n: i64| (0..n.max(0)).fold(g("Zero"), |acc, _| app(g("Succ"), acc));

    for m in 0i64..14 {
        for n in 0i64..14 {
            for op in ["natLe", "natLt"] {
                let want = nat_order_oracle(op, m, n);
                assert_eq!(
                    bool_of(&normalize(&ctx, &bin(op, natl(m), natl(n)))),
                    Some(want),
                    "{op} {m} {n} on two literals"
                );
                // The bridge the acceleration must not break: the literal spelling and the
                // Peano spine are the same value, so they must decide identically.
                assert_eq!(
                    normalize(&ctx, &bin(op, peano(m), peano(n))),
                    normalize(&ctx, &bin(op, natl(m), natl(n))),
                    "{op} {m} {n}: the Peano spine and the literal must agree"
                );
                // …and so must the MIXED spellings, which no single path handles.
                assert_eq!(
                    bool_of(&normalize(&ctx, &bin(op, peano(m), natl(n)))),
                    Some(want),
                    "{op} (peano {m}) {n}"
                );
                assert_eq!(
                    bool_of(&normalize(&ctx, &bin(op, natl(m), peano(n)))),
                    Some(want),
                    "{op} {m} (peano {n})"
                );
            }
        }
    }

    // The boundaries either side of every interesting rung, including the byte and word edges.
    for (m, n) in [
        (0i64, 0i64),
        (0, 1),
        (1, 0),
        (1, 1),
        (254, 255),
        (255, 255),
        (255, 256),
        (256, 255),
        (256, 256),
        (65_535, 65_536),
        (65_536, 65_535),
    ] {
        for op in ["natLe", "natLt"] {
            assert_eq!(
                bool_of(&normalize(&ctx, &bin(op, natl(m), natl(n)))),
                Some(nat_order_oracle(op, m, n)),
                "{op} {m} {n} — the boundary a fast path is most likely to get wrong"
            );
        }
    }

    // A NEGATIVE `Literal::Nat` is not a term the kernel mints, but it is a term the kernel can
    // be HANDED, and the Peano bridge peels it to `Zero`. The fast path must clamp the same way
    // or the two spellings of the same reduction disagree.
    let neg = Term::Lit(Literal::Nat(BigInt::from_i64(-3)));
    assert_eq!(
        bool_of(&normalize(&ctx, &bin("natLe", neg.clone(), natl(0)))),
        Some(true)
    );
    assert_eq!(
        bool_of(&normalize(&ctx, &bin("natLt", neg.clone(), natl(1)))),
        Some(true)
    );
    assert_eq!(
        bool_of(&normalize(&ctx, &bin("natLt", natl(0), neg))),
        Some(false)
    );
}

/// THE NEGATIVE that makes this better than Lean's override. An acceleration that fired on the
/// NAME would answer `true` for `natLe 1 2` in a context where `natLe` has been rebound to a
/// different program — and the kernel would then disagree with the very definition it
/// type-checked the application against, with no diagnostic anywhere. Ours checks the body.
#[test]
fn a_nat_order_name_bound_to_a_different_program_is_not_accelerated() {
    let binop = || pi("_", g("Nat"), pi("_", g("Nat"), g("Bool")));
    let const_false = || lam("m", g("Nat"), lam("n", g("Nat"), g("false")));
    // The prelude's `natLe` has the integer order's laws resting on it, so it keeps its program;
    // the rebinding is made where nothing depends on the name.
    assert_prelude_name_keeps_its_meaning(
        "natLe",
        binop(),
        Some(const_false()),
        &bin("natLe", natl(1), natl(2)),
    );

    let mut ctx = carriers_only_ctx();
    // `natLe` bound to the constant-`false` program, and `natLt` to the prelude's own body over
    // it. Everything about the application still type-checks; only the VALUE changes, and the
    // kernel must report the value of the body.
    ctx.add_definition("natLe".to_string(), binop(), const_false());
    ctx.add_definition(
        "natLt".to_string(),
        binop(),
        lam(
            "m",
            g("Nat"),
            lam(
                "n",
                g("Nat"),
                bin(
                    "natLe",
                    app(g("Succ"), Term::Var("m".to_string())),
                    Term::Var("n".to_string()),
                ),
            ),
        ),
    );
    assert!(
        ctx.refused_declarations().is_empty(),
        "the rebinding must be accepted, or this test is not testing what it says: {:?}",
        ctx.refused_declarations()
    );
    assert_eq!(
        bool_of(&normalize(&ctx, &bin("natLe", natl(1), natl(2)))),
        Some(false),
        "the kernel must compute what the BODY says, not what the name usually means"
    );
    // …and `natLt`, whose own body calls `natLe`, must lose its acceleration with it.
    assert_eq!(
        bool_of(&normalize(&ctx, &bin("natLt", natl(1), natl(2)))),
        Some(false),
        "an acceleration that depends on another constant must check that one too"
    );
}

/// THE OTHER NEGATIVE. A computation rule keyed on a name is a rule for a term the kernel
/// cannot type when nothing declares that name, so it must not fire in a context that has
/// never heard of it.
#[test]
fn the_nat_order_acceleration_never_fires_on_a_name_the_context_does_not_know() {
    let ctx = Context::new();
    assert!(
        ctx.get_global("natLe").is_none(),
        "a bare context declares no `natLe`"
    );
    for op in ["natLe", "natLt"] {
        let t = bin(op, natl(1), natl(2));
        assert_eq!(
            normalize(&ctx, &t),
            t,
            "`{op}` is undeclared here: reducing it would be computing with a term the kernel \
             cannot type"
        );
    }
}

/// THE BOUNDARY on the transparency axis. The acceleration stands in for the δ+ι unfolding of
/// a DEFINITION, so it must obey the same door: at the elaborator's transparency an
/// `@[irreducible]` `natLe` keeps its body and stays folded, fast path included.
#[test]
fn the_nat_order_acceleration_is_delta_and_obeys_the_transparency_door() {
    let ctx = std_ctx();
    let sealed = |n: &str| n == "natLe" || n == "natLt";
    let elaborator = Transparency::Excluding(&sealed);
    for op in ["natLe", "natLt"] {
        let t = bin(op, natl(3), natl(9));
        assert_eq!(
            normalize_at(&ctx, &t, elaborator),
            t,
            "`{op}` is opaque at this transparency — an acceleration that ignored the door \
             would make the kernel prove by `rfl` what the elaborator refuses"
        );
        assert_eq!(
            bool_of(&normalize_at(&ctx, &t, Transparency::All)),
            Some(nat_order_oracle(op, 3, 9)),
            "…and the kernel's own setting still decides it"
        );
    }
}

/// The SECOND ENGINE. `native_decide` trusts `eval` to agree with `normalize`; a `Nat` order
/// the reducer decides and the evaluator declines is a gap in that trust, and one the evaluator
/// decides differently would be a hole. Both spellings, because the evaluator reaches them by
/// different routes (`Fix` unfolds on a constructor, never on a literal).
#[test]
fn the_evaluator_decides_the_nat_order_exactly_as_the_reducer_does() {
    let ctx = std_ctx();
    let peano = |n: i64| (0..n.max(0)).fold(g("Zero"), |acc, _| app(g("Succ"), acc));
    for m in 0i64..10 {
        for n in 0i64..10 {
            for op in ["natLe", "natLt"] {
                let want = nat_order_oracle(op, m, n);
                let lit = bin(op, natl(m), natl(n));
                assert_eq!(
                    eval_bool(&ctx, &lit),
                    Some(want),
                    "the evaluator must decide {op} {m} {n} on two literals"
                );
                assert_eq!(eval_bool(&ctx, &lit), bool_of(&normalize(&ctx, &lit)));
                let spine = bin(op, peano(m), peano(n));
                assert_eq!(
                    eval_bool(&ctx, &spine),
                    Some(want),
                    "…and on the Peano spine, which it decided before the fast path existed"
                );
            }
        }
    }
    // The pair only a literal path can reach, in the evaluator too.
    assert_eq!(
        eval_bool(&ctx, &bin("natLt", natl(1_114_111), natl(1_114_112))),
        Some(true)
    );
}

/// BOTH KERNELS, at a pair the de Bruijn re-checker can still reach by its own recursion — so
/// this measures AGREEMENT rather than the re-checker's own wall. `⟨2, refl⟩ : Fin 3` is the
/// shape every bound proof in the prelude has.
#[test]
fn both_kernels_agree_on_an_accelerated_nat_order() {
    use logicaffeine_kernel::{double_check, DoubleCheck};
    let ctx = std_ctx();
    let apps = |f: Term, xs: &[Term]| xs.iter().fold(f, |a, x| app(a, x.clone()));
    let mut disagreed: Vec<String> = Vec::new();
    for (op, m, n) in [
        ("natLe", 2i64, 3i64),
        ("natLt", 2, 3),
        ("natLe", 3, 2),
        ("natLt", 3, 3),
    ] {
        let want = if nat_order_oracle(op, m, n) {
            g("true")
        } else {
            g("false")
        };
        let redex = bin(op, natl(m), natl(n));
        assert_eq!(normalize(&ctx, &redex), want, "the main kernel first");
        let claim = apps(g("Eq"), &[g("Bool"), redex, want.clone()]);
        let proof = apps(g("refl"), &[g("Bool"), want]);
        let coerced = app(lam("h", claim, Term::Var("h".to_string())), proof);
        if double_check(&ctx, &coerced) != DoubleCheck::Agreed {
            disagreed.push(format!("{op}({m},{n}): {:?}", double_check(&ctx, &coerced)));
        }
    }
    assert!(
        disagreed.is_empty(),
        "an accelerated order must be the SAME order the second kernel computes: {disagreed:?}"
    );
}

// =============================================================================
// THE ELABORATOR'S DOOR — one opacity predicate, not one per caller
//
// `Transparency::Excluding` takes a PREDICATE, which is right: the reducer must not have to
// know how the attribute is stored. The cost is that every caller has to build the closure,
// and two callers that build it differently disagree about whether a constant unfolds — which
// is `def_eq` answering differently depending on who asked. `normalize_elab` is the one
// closure, built from the registry `Context` already keeps.
// =============================================================================

/// The door reads the REGISTRY, so marking a constant `@[irreducible]` through `Context` is
/// enough — and the kernel's own face still unfolds it, exactly as Lean's kernel does.
#[test]
fn the_elaborator_door_folds_what_the_registry_calls_irreducible() {
    use logicaffeine_kernel::{normalize_checked_elab, normalize_elab, Reducibility};
    let mut ctx = std_ctx();
    ctx.add_definition("ordinary".to_string(), g("Nat"), natl(5));
    ctx.add_definition("secret".to_string(), g("Nat"), natl(5));
    ctx.set_reducibility("secret", Reducibility::Irreducible);

    assert_eq!(
        normalize_elab(&ctx, &g("secret")),
        g("secret"),
        "@[irreducible] stays folded"
    );
    assert_eq!(
        normalize_elab(&ctx, &g("ordinary")),
        natl(5),
        "…and nothing else changes"
    );
    assert_eq!(
        ctx.get_definition_body("secret"),
        Some(&natl(5)),
        "the BODY survives: this is opacity, not a manufactured axiom"
    );
    assert_eq!(
        normalize(&ctx, &g("secret")),
        natl(5),
        "the KERNEL's face unfolds it — Lean's kernel ignores the attribute, and an attribute \
         that changed what is TRUE would be a different system"
    );
    assert_eq!(normalize_checked_elab(&ctx, &g("secret")), Ok(g("secret")));

    // THE BOUNDARY: lifting the attribute restores unfolding at the same door, with no other
    // change to the context.
    ctx.set_reducibility("secret", Reducibility::Semireducible);
    assert_eq!(normalize_elab(&ctx, &g("secret")), natl(5));
    ctx.set_reducibility("secret", Reducibility::Reducible);
    assert_eq!(
        normalize_elab(&ctx, &g("secret")),
        natl(5),
        "`@[reducible]` is transparent too"
    );
}

// =============================================================================
// DEFECT 9 / T11-005 — what the honest channel actually guarantees
//
// `normalize` hands back a `Term`, so "this is the normal form" and "this is as far as I got"
// are the same answer. `normalize_checked` separates them. These pin the two halves of the
// contract that a future change could quietly break, and record the reason the BEST-EFFORT
// face is nevertheless CONSERVATIVE rather than unsound:
//
//   `reduce_step` is a FUNCTION of the term. So two reductions that hold the same term at the
//   same step count hold it forever after, and therefore have the same normal form. A
//   conversion check built on "normalise both sides, then compare" can be INCOMPLETE when a
//   budget runs out — never wrong. The δ-cycle answer (hand the input back) is conservative
//   for the same reason: the only term it can be mistaken for is itself.
//
// That argument is the reason DEFECT 9 is a DIAGNOSTIC defect and not a soundness hole, and it
// stops being true the moment `normalize` stops running a fixed number of deterministic steps
// — which is what these lock.
// =============================================================================

/// `Ok(nf)` must mean nf is a FIXPOINT: normalising it again changes nothing, and the honest
/// channel agrees a second time. Anything else is `normalize_checked` saying "finished" about a
/// reduction it did not finish.
#[test]
fn a_reported_normal_form_is_a_fixpoint_of_the_reducer() {
    let mut ctx = std_ctx();
    ctx.add_definition("five".to_string(), g("Nat"), natl(5));
    ctx.add_definition("chained".to_string(), g("Nat"), g("five"));
    let corpus = vec![
        natl(3),
        g("Zero"),
        bin("add", intl(2), intl(3)),
        bin("natLe", natl(2), natl(3)),
        g("chained"),
        app(
            lam("x", g("Nat"), app(g("Succ"), Term::Var("x".to_string()))),
            natl(1),
        ),
        bin("Text_append", textl("ab"), textl("cd")),
        bin("div", intl(-7), intl(2)),
    ];
    for t in &corpus {
        let nf = normalize_checked(&ctx, t).unwrap_or_else(|e| panic!("{t} must normalise: {e:?}"));
        assert_eq!(
            normalize(&ctx, &nf),
            nf,
            "a reported normal form must not reduce further: {t}"
        );
        assert_eq!(
            normalize_checked(&ctx, &nf),
            Ok(nf.clone()),
            "…and must re-report as Ok: {t}"
        );
    }
}

/// Determinism, stated where it can be violated: the same term reduces to the same thing every
/// time and from every starting point ON its own trajectory. This is the property that makes a
/// budget-exhausted comparison conservative instead of wrong, so it is worth a lock of its own.
#[test]
fn reduction_is_a_function_so_a_shared_partial_forces_a_shared_normal_form() {
    let mut ctx = std_ctx();
    // Two DIFFERENT terms whose reductions converge: `f 1` and `f 2` both β-reduce onto the
    // same body, which then reduces alone. Once two trajectories meet they cannot part, so
    // whatever either one is holding at step k, both hold — including when a budget stops them.
    ctx.add_definition(
        "ignoresIt".to_string(),
        pi("_", g("Nat"), g("Int")),
        lam("x", g("Nat"), bin("add", intl(20), intl(22))),
    );
    assert!(
        ctx.refused_declarations().is_empty(),
        "the definition must be ACCEPTED or the convergence below is vacuous: {:?}",
        ctx.refused_declarations()
    );
    let a = app(g("ignoresIt"), natl(1));
    let b = app(g("ignoresIt"), natl(2));
    assert_ne!(a, b, "the two terms really are different");
    assert_eq!(normalize(&ctx, &a), normalize(&ctx, &b), "…and converge");
    assert_eq!(normalize(&ctx, &a), intl(42));

    // Normalising is idempotent and repeatable: no hidden state, no ordering dependence.
    for t in [&a, &b] {
        let once = normalize(&ctx, t);
        assert_eq!(normalize(&ctx, &once), once, "idempotent");
        assert_eq!(normalize(&ctx, t), once, "repeatable");
    }
}

/// The δ-CYCLE answer, pinned from the side that matters: `normalize` stays a FUNCTION (so
/// conversion built on it stays stable), the honest channel says the reduction cycles, and the
/// answer is conservative — the cycle's two points are NOT reported convertible, which is
/// incomplete and safe rather than complete and wrong.
#[test]
fn a_cycle_is_reported_cyclic_and_never_silently_identified_with_its_neighbour() {
    let mut ctx = std_ctx();
    // The δ door refuses the cycle outright — Lean's rule, and the root repair.
    assert_delta_cycle_refused(&mut ctx, "ring_a", "ring_b");

    // The rewrite registry still builds one, so the reducer's half is asked of that.
    let ring = simp_cycle(&mut ctx, "ringR", 2);
    let points = [app(g(&ring[0]), g("Zero")), app(g(&ring[1]), g("Zero"))];

    // The channel reports WHY, and the partial it carries is a point ON the cycle — which point
    // is not a promise, because a cycle has no distinguished one. That is exactly the
    // information `normalize` cannot carry and the reason the honest face exists.
    for start in &points {
        match normalize_checked(&ctx, start) {
            Err((partial, Unreduced::Cyclic)) => assert!(
                points.contains(&partial),
                "the partial must be a point on the cycle, got {partial}"
            ),
            other => panic!("`{start}` cycles and must be reported Cyclic, got {other:?}"),
        }
    }

    // `normalize` stays a function: it does not answer one point for the other and back again,
    // which would make it non-idempotent and every conversion built on it unstable.
    for start in &points {
        assert_eq!(&normalize(&ctx, start), start);
    }
    // Nothing on the cycle is silently identified with a neighbour it was never equated with: not
    // with a value, and not with a point on an unrelated cycle.
    let other = simp_cycle(&mut ctx, "ringS", 2);
    assert!(
        !defeq_for_test(&ctx, &points[0], &app(g(&other[0]), g("Zero"))),
        "two unrelated non-normalising terms may never be found convertible"
    );
    assert!(!defeq_for_test(&ctx, &points[0], &g("Zero")));
    assert!(
        !defeq_for_test(&ctx, &g("ring_a"), &g("ring_b")),
        "an unpaid forward reference proves nothing"
    );
    // A cycle underneath a constructor is found too, at a period the tortoise must catch.
    let wrapped = app(g("Succ"), points[0].clone());
    assert!(matches!(
        normalize_checked(&ctx, &wrapped),
        Err((_, Unreduced::Cyclic))
    ));
}

/// THE END STATE the acceleration buys, recorded where it can be lost: a bound that neither
/// kernel could reach is now decided by BOTH, so the proof that cites it is accepted twice and
/// the proof that cites the opposite is refused twice.
///
/// `U_universes_extra::the_nat_order_shortcut_agrees_with_the_structural_recursion_everywhere_it_answers`
/// pins the PRE-acceleration arrangement — that the main kernel abandons `2^64 < 2^64`, so the
/// re-checker's own shortcut had to stay silent there or manufacture a disagreement about a
/// valid proof. The reason for that silence is gone: the first kernel now reaches it too, and
/// agreement is reached by both DECIDING rather than by both declining, which is the stronger
/// of the two ways to agree.
#[test]
fn a_bound_past_the_unary_budget_is_decided_and_accepted_by_both_kernels() {
    use logicaffeine_kernel::{double_check, DoubleCheck};
    let ctx = std_ctx();
    let apps = |f: Term, xs: &[Term]| xs.iter().fold(f, |a, x| app(a, x.clone()));
    let huge = Term::Lit(Literal::Nat(
        BigInt::parse_decimal("18446744073709551616").unwrap(),
    ));
    let redex = bin("natLt", huge.clone(), huge.clone());

    assert_eq!(
        normalize(&ctx, &redex),
        g("false"),
        "`2^64 < 2^64` is FALSE, and now decided"
    );

    let cite = |answer: Term| {
        let claim = apps(g("Eq"), &[g("Bool"), redex.clone(), answer.clone()]);
        app(
            lam("h", claim, Term::Var("h".to_string())),
            apps(g("refl"), &[g("Bool"), answer]),
        )
    };
    let truth = cite(g("false"));
    assert!(
        infer_type(&ctx, &truth).is_ok(),
        "the true citation must be accepted"
    );
    assert_eq!(
        double_check(&ctx, &truth),
        DoubleCheck::Agreed,
        "…by both kernels: {:?}",
        double_check(&ctx, &truth)
    );

    // THE NEGATIVE: deciding is not agreeing to anything. The opposite answer is refused, and
    // refused by the second kernel too — so the acceleration decides rather than asserts.
    let lie = cite(g("true"));
    assert!(
        infer_type(&ctx, &lie).is_err(),
        "`2^64 < 2^64 = true` must be refused"
    );
    assert!(
        logicaffeine_kernel::recheck(&ctx, &lie).is_err(),
        "…by the re-checker as well"
    );
}

// =============================================================================
// PROGRESS IS A RETURN VALUE — the fixpoint test, and the defect it was hiding
//
// `normalize` used to decide "is this a normal form?" by comparing the reducer's answer to its
// input. That is two mistakes in one line. It pays a Θ(size) structural walk PER STEP on top of
// the walk the step itself already cost — and it is only correct if term equality is
// REFLEXIVE.
//
// It is not. `Literal::Float` derives `PartialEq`, so two `Float` literals compare with IEEE
// `==`, and `NaN != NaN`. A term holding a `NaN` was therefore never equal to itself, the
// fixpoint test could never fire, and `normalize` spent its entire 2^20-step budget walking a
// term that was ALREADY A NORMAL FORM — then reported `Unreduced::BudgetSpent` about it.
// Measured before the fix: 25 ms for the bare literal, 407 ms for `Succ NaN`, growing with the
// term. That is DEFECT 9 from the other side: the honest channel lying in the safe-looking
// direction.
//
// The reducer KNOWS whether a rule fired. Having it say so is exact, free, and reflexivity-
// proof. These lock both halves: every shape stops when it should, and every shape that has a
// redex still fires it.
// =============================================================================

/// A `NaN` literal is a normal form and must be REPORTED as one — in a few microseconds, not a
/// million steps later as an exhausted budget.
#[test]
fn a_float_that_is_not_equal_to_itself_is_still_a_normal_form() {
    let ctx = std_ctx();
    let nan = Term::Lit(Literal::Float(f64::NAN));
    // `assert_eq!` on these terms is itself unusable — `Term`'s `PartialEq` is the very relation
    // under test, and it answers `false` for a NaN against itself. The oracle is therefore the
    // PRINTED form, which is a function of the term and not of `f64`'s comparison rules. That a
    // test cannot use the language's own equality on a well-typed value is the defect, restated.
    let shown = |t: &Term| format!("{t}");
    let finished = |t: &Term| match normalize_checked(&ctx, t) {
        Ok(nf) => shown(&nf),
        Err((partial, why)) => panic!(
            "`{t}` is a normal form and must be reported as one, got {why:?} at `{partial}` — \
             the fixpoint test is reading `==`, which is not reflexive here"
        ),
    };
    assert_eq!(finished(&nan), shown(&nan));
    // …and buried anywhere inside a term, at any arity.
    for wrapped in [
        app(g("Succ"), nan.clone()),
        app(app(g("Eq"), g("Float")), nan.clone()),
        lam("x", g("Float"), nan.clone()),
        pi("x", g("Float"), nan.clone()),
        Term::Fix {
            name: "f".to_string(),
            body: Box::new(nan.clone()),
        },
    ] {
        assert_eq!(
            finished(&wrapped),
            shown(&wrapped),
            "a normal form containing a `NaN` must still be reported as a normal form"
        );
    }
    // The reduction AROUND a NaN still happens, and still terminates.
    assert_eq!(
        shown(&normalize(
            &ctx,
            &app(
                lam("x", g("Float"), Term::Var("x".to_string())),
                nan.clone()
            )
        )),
        shown(&nan),
        "β still fires with a NaN argument, and the result is still recognised as normal"
    );
    // THE BOUNDARY: an ordinary float is unaffected, and so is the rest of the literal family.
    for lit in [
        Term::Lit(Literal::Float(1.5)),
        Term::Lit(Literal::Float(0.0)),
        Term::Lit(Literal::Float(f64::INFINITY)),
        intl(7),
        natl(7),
        textl("hi"),
    ] {
        assert_eq!(
            normalize_checked(&ctx, &lit),
            Ok(lit.clone()),
            "{lit} is a normal form"
        );
    }
}

/// Every `Term` shape, each with the answer it must reduce to and the fact that it STOPS there.
/// A reducer that reported "no redex" too eagerly would silently hand back an unreduced term;
/// one that reported it too late would spin. This walks the whole constructor set so neither can
/// hide in a shape nothing else exercises.
#[test]
fn every_term_shape_reduces_to_its_normal_form_and_is_then_reported_finished() {
    let mut ctx = std_ctx();
    ctx.add_definition("five".to_string(), g("Nat"), natl(5));
    ctx.add_declaration("opaqueNat", g("Nat"));
    with_text_ops(&mut ctx);
    let v = |n: &str| Term::Var(n.to_string());
    // `add k x` on a variable `x`: δ exposes Lean's `Int.add` (`Int_add`), whose `match` on the
    // literal fires and whose `match` on `x` is stuck — no value, and nothing left to reduce.
    let int_add_stuck_on = |k: i64, x: &str| Term::Match {
        discriminant: Box::new(v(x)),
        motive: Box::new(lam("_", g("Int"), g("Int"))),
        cases: vec![
            lam(
                "n",
                g("Nat"),
                app(g("Int_ofNat"), bin("Nat_add", natl(k), v("n"))),
            ),
            lam(
                "n",
                g("Nat"),
                bin("Int_subNatNat", natl(k), app(g("Succ"), v("n"))),
            ),
        ],
    };

    let mutual = Term::MutualFix {
        defs: vec![
            (
                "f".to_string(),
                lam("n", g("Nat"), bin("add", intl(1), intl(1))),
            ),
            ("h".to_string(), lam("n", g("Nat"), v("n"))),
        ],
        index: 1,
    };
    let mutual_done = Term::MutualFix {
        defs: vec![
            ("f".to_string(), lam("n", g("Nat"), intl(2))),
            ("h".to_string(), lam("n", g("Nat"), v("n"))),
        ],
        index: 1,
    };

    let cases: Vec<(&str, Term, Term)> = vec![
        ("an Int literal", intl(7), intl(7)),
        (
            "a sort",
            Term::Sort(logicaffeine_kernel::Universe::Prop),
            Term::Sort(logicaffeine_kernel::Universe::Prop),
        ),
        ("a free variable", v("x"), v("x")),
        ("a hole", Term::Hole, Term::Hole),
        ("an axiom", g("opaqueNat"), g("opaqueNat")),
        ("a constructor", g("Zero"), g("Zero")),
        ("an inductive", g("Nat"), g("Nat")),
        ("δ on a definition", g("five"), natl(5)),
        (
            "β",
            app(lam("x", g("Nat"), app(g("Succ"), v("x"))), g("Zero")),
            app(g("Succ"), g("Zero")),
        ),
        ("a primitive", bin("add", intl(2), intl(3)), intl(5)),
        (
            "a stuck Int operation",
            bin("add", intl(2), v("y")),
            int_add_stuck_on(2, "y"),
        ),
        (
            "a primitive rule",
            bin("Text_append", textl("ab"), textl("cd")),
            textl("abcd"),
        ),
        (
            "a stuck primitive",
            bin("Text_append", textl("ab"), v("s")),
            bin("Text_append", textl("ab"), v("s")),
        ),
        (
            "ζ",
            Term::Let {
                name: "y".to_string(),
                ty: Box::new(g("Nat")),
                value: Box::new(natl(1)),
                body: Box::new(app(g("Succ"), v("y"))),
            },
            app(g("Succ"), natl(1)),
        ),
        (
            "under a λ",
            lam("x", g("Nat"), bin("add", intl(1), intl(2))),
            lam("x", g("Nat"), intl(3)),
        ),
        (
            "in a λ's domain",
            lam("x", g("five"), v("x")),
            lam("x", natl(5), v("x")),
        ),
        (
            "under a Π",
            pi("x", g("Nat"), bin("add", intl(1), intl(2))),
            pi("x", g("Nat"), intl(3)),
        ),
        (
            "in a Π's domain",
            pi("x", g("five"), g("Nat")),
            pi("x", natl(5), g("Nat")),
        ),
        (
            "under an unapplied fix",
            Term::Fix {
                name: "f".to_string(),
                body: Box::new(lam("n", g("Nat"), bin("mul", intl(6), intl(7)))),
            },
            Term::Fix {
                name: "f".to_string(),
                body: Box::new(lam("n", g("Nat"), intl(42))),
            },
        ),
        ("under a mutual fix", mutual, mutual_done),
        (
            "ι on a literal scrutinee",
            Term::Match {
                discriminant: Box::new(natl(0)),
                motive: Box::new(lam("_", g("Nat"), g("Nat"))),
                cases: vec![g("five"), lam("k", g("Nat"), v("k"))],
            },
            natl(5),
        ),
        (
            "ι on a constructor scrutinee",
            Term::Match {
                discriminant: Box::new(app(g("Succ"), g("Zero"))),
                motive: Box::new(lam("_", g("Nat"), g("Nat"))),
                cases: vec![g("Zero"), lam("k", g("Nat"), app(g("Succ"), v("k")))],
            },
            app(g("Succ"), g("Zero")),
        ),
        (
            "a stuck match reduces its discriminant",
            Term::Match {
                discriminant: Box::new(bin("add", intl(1), v("z"))),
                motive: Box::new(lam("_", g("Nat"), g("Nat"))),
                cases: vec![g("Zero"), lam("k", g("Nat"), v("k"))],
            },
            Term::Match {
                discriminant: Box::new(int_add_stuck_on(1, "z")),
                motive: Box::new(lam("_", g("Nat"), g("Nat"))),
                cases: vec![g("Zero"), lam("k", g("Nat"), v("k"))],
            },
        ),
    ];

    for (what, input, want) in cases {
        let got = normalize_checked(&ctx, &input)
            .unwrap_or_else(|e| panic!("{what}: must finish, got {e:?}"));
        assert_eq!(got, want, "{what}: wrong normal form");
        assert_eq!(
            normalize_checked(&ctx, &got),
            Ok(got.clone()),
            "{what}: the normal form must be reported finished a second time"
        );
    }
}

/// Brent's schedule, exercised at a period the one-step cycle cannot reach: a cycle of period
/// THREE entered after a TAIL of two steps, so the tortoise has to be re-saved before the
/// comparison can ever succeed. A cycle detector that only ever compared against step zero would
/// pass every other test in this file and fail here. The δ door refuses a δ-cycle of any period,
/// so the tail is two δ-steps and the cycle is three `@[simp]` rewrites.
#[test]
fn a_cycle_of_period_three_behind_a_tail_is_still_reported_cyclic() {
    let mut ctx = std_ctx();
    // The δ door refuses the 3-cycle at the definition that closes it.
    ctx.add_definition("dcyc0".to_string(), g("Nat"), g("dcyc1"));
    ctx.add_definition("dcyc1".to_string(), g("Nat"), g("dcyc2"));
    ctx.add_definition("dcyc2".to_string(), g("Nat"), g("dcyc0"));
    assert!(
        ctx.get_definition_body("dcyc2").is_none(),
        "`dcyc2` closes a δ-cycle"
    );
    assert!(ctx.refused_declarations().iter().any(|(n, _)| n == "dcyc2"));

    // tail: enter0 → enter1 → cyc0 Zero (two δ-steps); cycle: cyc0 → cyc1 → cyc2 → cyc0 (rewrites)
    let cyc = simp_cycle(&mut ctx, "cyc", 3);
    ctx.add_definition("enter1".to_string(), g("Nat"), app(g(&cyc[0]), g("Zero")));
    ctx.add_definition("enter0".to_string(), g("Nat"), g("enter1"));
    let starts = [
        app(g(&cyc[0]), g("Zero")),
        app(g(&cyc[1]), g("Zero")),
        app(g(&cyc[2]), g("Zero")),
        g("enter0"),
        g("enter1"),
    ];
    for start in &starts {
        match normalize_checked(&ctx, start) {
            Err((_, Unreduced::Cyclic)) => {}
            other => {
                panic!("`{start}` reaches a 3-cycle and must be reported Cyclic, got {other:?}")
            }
        }
        assert_eq!(
            &normalize(&ctx, start),
            start,
            "`normalize` stays a function on it"
        );
    }
    // A δ-chain of the SAME length that terminates must NOT be called cyclic — the boundary
    // either side of the detector.
    ctx.add_definition("lin2".to_string(), g("Nat"), natl(9));
    ctx.add_definition("lin1".to_string(), g("Nat"), g("lin2"));
    ctx.add_definition("lin0".to_string(), g("Nat"), g("lin1"));
    assert_eq!(normalize_checked(&ctx, &g("lin0")), Ok(natl(9)));

    // A self-referential definition is the period-ONE δ case, and the door refuses it.
    ctx.add_definition("selfRef".to_string(), g("Nat"), g("selfRef"));
    assert!(
        ctx.get_definition_body("selfRef").is_none(),
        "`a := a` is a δ-cycle of period one"
    );
    assert!(ctx
        .refused_declarations()
        .iter()
        .any(|(n, _)| n == "selfRef"));
    // The period-ONE reduction the reducer can still meet is β's: the rule fires, and the
    // answer is "no normal form", not "already normal".
    match normalize_checked(&ctx, &omega()) {
        Err((_, Unreduced::Cyclic)) => {}
        other => panic!("ω has no normal form and must say so, got {other:?}"),
    }
    assert_eq!(normalize(&ctx, &omega()), omega());
}

// =============================================================================
// THE REFLECTED OBJECT LANGUAGE MODELS THE KERNEL'S ARITHMETIC — or it certifies lies
//
// `syn_eval` runs a DEEPLY EMBEDDED term (`SApp (SApp (SName "div") (SLit a)) (SLit b)`), and
// `try_dcompute_conclude` accepts proofs through that door: a goal is concluded when both sides
// evaluate to the same reflected value. So the reflected `div` is not a second, independent
// notion of division that may differ by convention — it is a MODEL of the kernel's own `div`,
// and any divergence is a false equation with a kernel-checked proof in front of it.
//
// `logicaffeine_tests::phase99_solver::test_syn_eval_div_by_zero` asserts the opposite for the
// zero divisor ("Division by zero should remain stuck"), which is the pre-Euclidean reading;
// under Lean's total convention `10 / 0 = 0` and the reflected language has to say so too.
// This is the invariant that matters stated positively, in a file that owns the rule.
// =============================================================================

/// Every `div`/`mod` the reflected evaluator computes must be the value the kernel computes —
/// on both signs of both operands, and at the zero divisor the two conventions part company on.
#[test]
fn the_reflected_evaluator_models_the_kernels_own_division() {
    let ctx = std_ctx();
    let sname = |s: &str| app(g("SName"), textl(s));
    let slit = |n: i64| app(g("SLit"), intl(n));
    let sapp = |f: Term, x: Term| app(app(g("SApp"), f), x);
    let eval_syn = |e: Term| normalize(&ctx, &app(app(g("syn_eval"), intl(100)), e));

    let mut divergences: Vec<String> = Vec::new();
    for (a, b) in [
        (-7i64, 2i64),
        (7, 2),
        (7, -2),
        (-7, -2),
        (9, 2),
        (-1, 3),
        (10, 0),
        (-10, 0),
        (0, 0),
    ] {
        for op in ["div", "mod"] {
            let reflected = eval_syn(sapp(sapp(sname(op), slit(a)), slit(b)));
            let kernel = normalize(&ctx, &bin(op, intl(a), intl(b)));
            let want = match op {
                "div" => euclid_by_search(a, b).0,
                _ => euclid_by_search(a, b).1,
            };
            if kernel != intl(want) {
                divergences.push(format!("kernel {op}({a},{b}) = {kernel}, want {want}"));
            }
            if reflected != slit(want) {
                divergences.push(format!(
                    "reflected {op}({a},{b}) = {reflected}, want (SLit {want}) — the reflected \
                     language models the kernel's arithmetic, so a divergence here is a FALSE \
                     equation `try_dcompute_conclude` will certify"
                ));
            }
        }
    }
    assert!(divergences.is_empty(), "{divergences:#?}");

    // THE NEGATIVE: the reflected evaluator is still STUCK on what it genuinely cannot compute,
    // so "it answers everything" is not how it passed the grid above.
    let partial = sapp(sname("add"), slit(1));
    assert_eq!(
        eval_syn(partial.clone()),
        partial,
        "a partial application stays stuck"
    );
    let unknown = sapp(sapp(sname("nonsense"), slit(1)), slit(2));
    assert_eq!(
        eval_syn(unknown.clone()),
        unknown,
        "an unknown operator stays stuck"
    );
}

// =============================================================================
// DEFECT 5 — one kernel, one verdict about a malformed `Const`
//
// `Term::Const { name, levels }` whose `levels` do not match its declaration's universe
// PARAMETERS is not a term of the calculus. `infer_type` says so. The reducer used to fall
// through to `term.clone()`, which is the reducer saying "settled normal form" about a term
// that denotes nothing — the two halves of one kernel returning opposite verdicts, and a
// conversion check built on "normalise both sides, then compare" then comparing two things
// that have no meaning.
//
// The reducer's honest answer is `Term::Hole`: the kernel's own "there is nothing determinate
// here", which `infer_type` refuses by construction (`CannotInferHole`). It cannot be mistaken
// for a normal form of the original, and it cannot be LAUNDERED into a well-typed term — the
// tempting alternative, zipping the level list and dropping the extra, would silently make
// `id.{0,1}` mean `id.{0}` and hand back something that type-checks.
// =============================================================================

/// The whole verdict table: at the right arity the constant unfolds, at any wrong arity — too
/// many levels OR too few — reduction refuses in the same way typing does, and what it produces
/// is never well typed.
#[test]
fn a_universe_level_arity_mismatch_is_refused_by_reduction_as_well_as_typing() {
    use logicaffeine_kernel::Universe;
    let mut ctx = std_ctx();
    let u = Universe::Var("u".to_string());
    let v = |n: &str| Term::Var(n.to_string());
    // `id.{u} : Π(A : Sort u). A → A := λA. λa. a`, with ONE universe parameter.
    ctx.add_universe_poly(
        "polyId",
        vec!["u".to_string()],
        pi("A", Term::Sort(u.clone()), pi("_", v("A"), v("A"))),
        lam("A", Term::Sort(u), lam("a", v("A"), v("a"))),
    );
    let at = |ls: Vec<Universe>| Term::Const {
        name: "polyId".to_string(),
        levels: ls,
    };

    // THE CONTROL: the right arity type-checks AND unfolds. A "fix" that refused everything
    // would pass every negative below and fail here.
    let good = at(vec![Universe::Type(0)]);
    assert!(
        infer_type(&ctx, &good).is_ok(),
        "polyId.{{0}} must type-check"
    );
    assert_ne!(
        normalize(&ctx, &good),
        good,
        "…and must δ-unfold to its instantiated body"
    );
    assert_ne!(
        normalize(&ctx, &good),
        Term::Hole,
        "…to a real body, not to a hole"
    );

    // THE BOUNDARY, one rung either side: too many levels and too few.
    for levels in [
        vec![Universe::Type(0), Universe::Type(1)],
        vec![],
        vec![Universe::Type(0), Universe::Type(0), Universe::Type(0)],
    ] {
        let bad = at(levels.clone());
        assert!(
            infer_type(&ctx, &bad).is_err(),
            "typing must refuse polyId at {} levels",
            levels.len()
        );
        let nf = normalize(&ctx, &bad);
        assert_ne!(
            nf, bad,
            "reduction must not hand a malformed `Const` back as a settled normal form"
        );
        assert!(
            infer_type(&ctx, &nf).is_err(),
            "…and must not launder it into a well-typed term either, got `{nf}`"
        );
        assert_eq!(
            nf,
            Term::Hole,
            "the reducer's answer is the kernel's own `no meaning here`"
        );
        // It is refused the same way at the elaborator's transparency: malformedness is a
        // property of the TERM, not of how much δ the caller asked for.
        let never = |_: &str| false;
        assert_eq!(
            normalize_at(&ctx, &bad, Transparency::Excluding(&never)),
            Term::Hole
        );
    }

    // THE NEGATIVE that keeps the rule narrow: a `Const` naming something that is not
    // universe-polymorphic at all is STUCK, not a hole — reduction has no rule for it, which is
    // a different fact from the term being malformed.
    let plain = Term::Const {
        name: "Nat".to_string(),
        levels: vec![],
    };
    assert_eq!(
        normalize(&ctx, &plain),
        plain,
        "a non-polymorphic `Const` is simply stuck"
    );
    let unknown = Term::Const {
        name: "neverHeardOfIt".to_string(),
        levels: vec![Universe::Type(0)],
    };
    assert_eq!(
        normalize(&ctx, &unknown),
        unknown,
        "and so is an unknown name"
    );

    // …and the malformed constant is refused just as firmly INSIDE a larger term, where a
    // caller would otherwise see a well-shaped application wrapped around nothing.
    let inside = app(g("Succ"), at(vec![Universe::Type(0), Universe::Type(1)]));
    assert_eq!(normalize(&ctx, &inside), app(g("Succ"), Term::Hole));
    assert!(infer_type(&ctx, &normalize(&ctx, &inside)).is_err());
}

/// THE TRAP, locked shut. DEFECT 9 asks that `normalize` never hand back an unfinished
/// reduction as a normal form, and the tempting way to satisfy it for a δ-CYCLE is the move
/// that works for a malformed `Const` above: answer `Term::Hole`, the kernel's "nothing
/// determinate here". For a cycle that is UNSOUND, and the difference is worth a test of its own
/// because the two cases look identical from the reducer's side.
///
/// A mis-levelled `Const` is ILL-TYPED, so no proof can mention it: two of them collapsing to
/// the same `Hole` is unreachable from any theorem. A cyclic constant is WELL-TYPED
/// (`loop_a : Nat`), so if two DISTINCT cycles both normalised to `Hole` they would compare
/// definitionally equal, and `refl` would prove `loop_a = otherLoop` — a false equation between
/// two perfectly well-typed terms.
///
/// So the cycle answer stays conservative, and the real repair belongs where Lean puts it: a
/// definition environment that cannot be made cyclic in the first place. That repair is in
/// (`Context::declare` refuses the closing definition), so the δ half is asserted at the door and
/// the reducer's half is asked of the rewrite cycles the `@[simp]` registry can still build.
#[test]
fn two_distinct_delta_cycles_are_never_definitionally_identified() {
    let mut ctx = std_ctx();
    assert_delta_cycle_refused(&mut ctx, "ringA0", "ringA1");
    assert_delta_cycle_refused(&mut ctx, "ringB0", "ringB1");
    let a = simp_cycle(&mut ctx, "spinA", 2);
    let b = simp_cycle(&mut ctx, "spinB", 2);
    ctx.add_definition("loopA".to_string(), g("Nat"), app(g(&a[0]), g("Zero")));
    ctx.add_definition("loopB".to_string(), g("Nat"), app(g(&b[0]), g("Zero")));

    // Both are well typed, so a proof CAN be written about them — which is the whole hazard.
    for n in ["loopA", "loopB"] {
        assert_eq!(
            infer_type(&ctx, &g(n)).ok(),
            Some(g("Nat")),
            "`{n}` is a well-typed `Nat`, whatever its reduction does"
        );
        assert!(matches!(
            normalize_checked(&ctx, &g(n)),
            Err((_, Unreduced::Cyclic))
        ));
    }
    assert!(
        !defeq_for_test(&ctx, &g("loopA"), &g("loopB")),
        "two unrelated non-normalising constants must NEVER be identified — collapsing every \
         cycle to one answer would make `refl` prove `loopA = loopB`"
    );
    let claim = app(app(app(g("Eq"), g("Nat")), g("loopA")), g("loopB"));
    let forged = app(
        lam("h", claim, Term::Var("h".to_string())),
        app(app(g("refl"), g("Nat")), g("loopA")),
    );
    assert!(
        infer_type(&ctx, &forged).is_err(),
        "…and the proof that would rest on it must be REFUSED"
    );
    // The malformed-`Const` answer stays what it is: those two DO share an answer, and that is
    // harmless precisely because neither can appear in a theorem.
    assert_eq!(
        normalize(
            &ctx,
            &Term::Const {
                name: "polyMissing".to_string(),
                levels: vec![logicaffeine_kernel::Universe::Type(0u32)],
            }
        ),
        Term::Const {
            name: "polyMissing".to_string(),
            levels: vec![logicaffeine_kernel::Universe::Type(0u32)],
        },
        "an unknown `Const` is stuck, not a hole — the hole is for a KNOWN constant at the \
         wrong arity"
    );
}

// =============================================================================
// NOT IN THE CORPUS — a computation rule that fires at a type its declaration denies
//
// `try_primitive_reduce` dispatches on the NAME and on the literal kinds, and it already
// refuses to outrank a DEFINITION of that name. It did not look at the declared TYPE. So a
// context that declares `Text_append : Text → Text → Nat` gets a kernel where
// `infer_type` says `Nat` and `normalize` produces a `Text` literal: SUBJECT REDUCTION broken,
// which is the property that makes a reduct usable in place of the term it came from. The
// codebase already names this shape — a typing/reduction divergence is "conformance defect 3" —
// and this is another instance of it, reached through the primitive table rather than through
// `get_global`'s resolution order.
//
// It is not by itself a route to `False` (the declaration is the user's own axiom), but every
// conversion check in the kernel rests on "a term and its reduct are interchangeable", and
// here they are not even the same type.
// =============================================================================

/// A primitive rule must fire at the type its declaration promises, and stay stuck otherwise —
/// with the well-typed declaration right beside it, so "it never fires" cannot pass for a fix.
#[test]
fn a_primitive_rule_never_fires_at_a_type_its_declaration_denies() {
    let text = || g("Text");
    let int = || g("Int");

    // THE CONTROL: declared at the right type, the rules compute and the value has that type.
    let mut right = std_ctx();
    right.add_declaration("Text_append", pi("_", text(), pi("_", text(), text())));
    let cat = bin("Text_append", textl("ab"), textl("cd"));
    assert_eq!(normalize(&right, &cat), textl("abcd"));
    assert_eq!(
        infer_type(&right, &cat).ok(),
        Some(g("Text")),
        "and the value is a Text"
    );

    // THE DEFECT: the same application, declared at a type the rule's answer does not inhabit.
    let mut wrong = std_ctx();
    wrong.add_declaration("Text_append", pi("_", text(), pi("_", text(), g("Nat"))));
    assert_eq!(
        infer_type(&wrong, &cat).ok(),
        Some(g("Nat")),
        "the declaration is what the kernel types the application at"
    );
    assert_eq!(
        normalize(&wrong, &cat),
        cat,
        "…so the computation rule must NOT fire: its answer is a `Text` literal, and handing \
         that back as the reduct of a `Nat`-typed term breaks subject reduction"
    );

    // The same, on the arithmetic the whole kernel rests on: `add` at a Bool result. The
    // prelude's `add` keeps its meaning (theorems rest on it), so the bent declarations are made
    // where nothing depends on the names.
    assert_prelude_name_keeps_its_meaning(
        "add",
        pi("_", int(), pi("_", int(), g("Bool"))),
        None,
        &bin("add", intl(2), intl(3)),
    );
    let mut bent = carriers_only_ctx();
    bent.add_declaration("add", pi("_", int(), pi("_", int(), g("Bool"))));
    assert_eq!(
        normalize(&bent, &bin("add", intl(2), intl(3))),
        bin("add", intl(2), intl(3)),
        "`add` declared to return Bool must not reduce to an Int literal"
    );
    // …and the comparison the other way: `le` promised an Int result must not produce `true`.
    let mut bent_cmp = carriers_only_ctx();
    bent_cmp.add_declaration("le", pi("_", int(), pi("_", int(), int())));
    assert_eq!(
        normalize(&bent_cmp, &bin("le", intl(2), intl(3))),
        bin("le", intl(2), intl(3)),
        "`le` declared to return Int must not reduce to a Bool constructor"
    );
    // A wrong DOMAIN is refused too, not only a wrong codomain.
    let mut bent_dom = carriers_only_ctx();
    bent_dom.add_declaration("mul", pi("_", g("Nat"), pi("_", g("Nat"), g("Nat"))));
    assert_eq!(
        normalize(&bent_dom, &bin("mul", intl(2), intl(3))),
        bin("mul", intl(2), intl(3)),
        "`mul` declared over Nat must not fold two Int literals"
    );
    // …and in that same carriers-only context, the SAME names declared at the right types DO
    // compute, so the three refusals above are the type guard and not a context in which the
    // rules never fire.
    let mut straight = carriers_only_ctx();
    straight.add_declaration("add", pi("_", int(), pi("_", int(), int())));
    straight.add_declaration("le", pi("_", int(), pi("_", int(), g("Bool"))));
    straight.add_declaration("mul", pi("_", int(), pi("_", int(), int())));
    assert_eq!(normalize(&straight, &bin("add", intl(2), intl(3))), intl(5));
    assert_eq!(
        normalize(&straight, &bin("le", intl(2), intl(3))),
        g("true")
    );
    assert_eq!(normalize(&straight, &bin("mul", intl(2), intl(3))), intl(6));

    // THE BOUNDARY: the untouched standard context still computes everything it did before.
    let ctx = std_ctx();
    assert_eq!(normalize(&ctx, &bin("add", intl(2), intl(3))), intl(5));
    assert_eq!(normalize(&ctx, &bin("mul", intl(6), intl(7))), intl(42));
    assert_eq!(normalize(&ctx, &bin("le", intl(2), intl(3))), g("true"));
    assert_eq!(normalize(&ctx, &bin("gt", intl(2), intl(3))), g("false"));
    assert_eq!(normalize(&ctx, &bin("div", intl(-7), intl(2))), intl(-4));
}

/// The pit-of-success half: every name the primitive table computes for must HAVE a declared
/// signature to be checked against, or the check above is silently vacuous for it. A new
/// primitive that forgets its row turns this red instead of shipping unguarded.
#[test]
fn every_primitive_the_reducer_computes_has_a_signature_it_is_held_to() {
    let mut ctx = std_ctx();
    with_text_ops(&mut ctx);
    // (name, a fully applied instance that MUST compute, a wrong declared type)
    let cases: Vec<(&str, Term, Term)> = vec![
        (
            "add",
            bin("add", intl(1), intl(1)),
            pi("_", g("Int"), pi("_", g("Int"), g("Bool"))),
        ),
        (
            "sub",
            bin("sub", intl(1), intl(1)),
            pi("_", g("Int"), pi("_", g("Int"), g("Bool"))),
        ),
        (
            "mul",
            bin("mul", intl(1), intl(1)),
            pi("_", g("Int"), pi("_", g("Int"), g("Bool"))),
        ),
        (
            "div",
            bin("div", intl(1), intl(1)),
            pi("_", g("Int"), pi("_", g("Int"), g("Bool"))),
        ),
        (
            "mod",
            bin("mod", intl(1), intl(1)),
            pi("_", g("Int"), pi("_", g("Int"), g("Bool"))),
        ),
        (
            "le",
            bin("le", intl(1), intl(1)),
            pi("_", g("Int"), pi("_", g("Int"), g("Int"))),
        ),
        (
            "lt",
            bin("lt", intl(1), intl(1)),
            pi("_", g("Int"), pi("_", g("Int"), g("Int"))),
        ),
        (
            "ge",
            bin("ge", intl(1), intl(1)),
            pi("_", g("Int"), pi("_", g("Int"), g("Int"))),
        ),
        (
            "gt",
            bin("gt", intl(1), intl(1)),
            pi("_", g("Int"), pi("_", g("Int"), g("Int"))),
        ),
        (
            "Text_append",
            bin("Text_append", textl("a"), textl("b")),
            pi("_", g("Text"), pi("_", g("Text"), g("Nat"))),
        ),
        (
            "Text_beq",
            bin("Text_beq", textl("a"), textl("b")),
            pi("_", g("Text"), pi("_", g("Text"), g("Text"))),
        ),
        (
            "Text_isPrefixOf",
            bin("Text_isPrefixOf", textl("a"), textl("b")),
            pi("_", g("Text"), pi("_", g("Text"), g("Text"))),
        ),
        (
            "Text_length",
            app(g("Text_length"), textl("ab")),
            pi("_", g("Text"), g("Int")),
        ),
    ];
    let mut unguarded: Vec<&str> = Vec::new();
    for (name, instance, wrong_ty) in &cases {
        assert_ne!(
            normalize(&ctx, instance),
            *instance,
            "`{name}` must compute at its declared type, or this row proves nothing"
        );
        // The prelude's operation names keep their meaning (theorems rest on them), so the
        // wrong declaration is made where nothing depends on the name — a context in which the
        // rule does compute until the declaration denies its type.
        let mut plain = carriers_only_ctx();
        with_text_ops(&mut plain);
        assert_ne!(
            normalize(&plain, instance),
            *instance,
            "`{name}` computes in the carriers-only context, so a refusal there is the guard"
        );
        let mut bent = carriers_only_ctx();
        with_text_ops(&mut bent);
        bent.add_declaration(name, wrong_ty.clone());
        assert!(
            bent.refused_declarations().is_empty(),
            "{name}: {:?}",
            bent.refused_declarations()
        );
        if normalize(&bent, instance) != *instance {
            unguarded.push(name);
        }
    }
    assert!(
        unguarded.is_empty(),
        "these computation rules fire regardless of the type their declaration promises, so \
         `normalize` can hand back a reduct of a different type than the term it replaced: \
         {unguarded:?}"
    );
}

/// The SECOND ENGINE half of the same rule. `native_decide` trusts the evaluator to agree with
/// `normalize`; an evaluator that folded `add` at a declaration promising `Bool` while the
/// reducer declined would be a disagreement about the value of a well-typed term, and it is the
/// evaluator's verdict that a `reduceBool` proof cites.
#[test]
fn the_evaluator_holds_a_primitive_to_the_same_declared_type_the_reducer_does() {
    // The prelude's `le`/`add` keep their meaning (theorems rest on them), so each bent
    // declaration is made where nothing depends on the name, beside the prelude's carriers.
    let bent = |op: &str, ty: Term| {
        let mut ctx = carriers_only_ctx();
        ctx.add_declaration(op, ty);
        assert!(
            ctx.refused_declarations().is_empty(),
            "{:?}",
            ctx.refused_declarations()
        );
        ctx
    };
    let int = || g("Int");
    // `le 2 3` is a Bool decision, so the evaluator has an opinion about it in every context.
    let decision = bin("le", intl(2), intl(3));
    let ctx = std_ctx();
    assert_eq!(
        eval_bool(&ctx, &decision),
        Some(true),
        "the control: it decides normally"
    );
    assert_eq!(bool_of(&normalize(&ctx, &decision)), Some(true));

    // Declared to return an Int, `le` has no Bool answer — and BOTH engines must say so.
    let wrong = bent("le", pi("_", int(), pi("_", int(), int())));
    assert_eq!(
        normalize(&wrong, &decision),
        decision,
        "the reducer declines"
    );
    assert_eq!(
        eval_bool(&wrong, &decision),
        None,
        "…and so must the evaluator"
    );

    // The same through an `add` the decision depends on: if the evaluator folded `add` where the
    // reducer will not, `native_decide` would certify a value `normalize` never produced.
    let nested = bin("le", bin("add", intl(2), intl(3)), intl(5));
    assert_eq!(eval_bool(&ctx, &nested), Some(true), "the control again");
    let bent_add = bent("add", pi("_", int(), pi("_", int(), g("Bool"))));
    let reduct = normalize(&bent_add, &nested);
    assert_eq!(
        reduct, nested,
        "the reducer will not fold an `add` its declaration types at Bool"
    );
    assert!(bool_of(&reduct).is_none(), "…and so it decides nothing");
    assert_eq!(
        eval_bool(&bent_add, &nested),
        None,
        "…and the evaluator declines with it"
    );
}

// =============================================================================
// WAVE 3 · SECTION 1 — the `@[simp]` registry, CONSULTED
//
// `Context::add_simp_lemma` / `simp_rules_for(head)` landed in wave 2 and nothing read them.
// A registry nothing consults is a list of names, which is the exact shape T02-013 already
// records as the gap. These pin the other half: a registered rule REACHES the reducer, an
// unregistered one does NOT, a rule for a different head never fires, a rule whose pattern
// cannot bind declines rather than guessing, and a SCOPED rule leaves with its scope.
//
// The last is the soundness clause and the reason the rules are read from the `Context` on
// every call rather than cached anywhere: `pop_scope` REMOVES rules, so a conversion decided
// while a scoped rule was in force must stop holding the moment the scope closes.
// =============================================================================

/// `size n := plus n 0` is stuck on a variable, because `plus` recurses on its FIRST argument —
/// exactly the shape a rewrite lemma exists to close, and exactly what β/δ/ι cannot reach.
fn simp_fixture() -> (Context, Term) {
    let mut ctx = std_ctx();
    // `plus` recurses on its FIRST argument, so `plus n 0` is stuck on a free `n`. The recursive
    // call is the FIXPOINT's own binder: a body naming the constant being defined is a δ-cycle,
    // and the gateway refuses it.
    let v = |n: &str| Term::Var(n.to_string());
    let plus_body = Term::Fix {
        name: "plus".to_string(),
        body: Box::new(lam(
            "n",
            g("Nat"),
            lam(
                "m",
                g("Nat"),
                Term::Match {
                    discriminant: Box::new(v("n")),
                    motive: Box::new(lam("_", g("Nat"), g("Nat"))),
                    cases: vec![
                        v("m"),
                        lam(
                            "k",
                            g("Nat"),
                            app(g("Succ"), app(app(v("plus"), v("k")), v("m"))),
                        ),
                    ],
                },
            ),
        )),
    };
    ctx.add_definition(
        "plus".to_string(),
        pi("_", g("Nat"), pi("_", g("Nat"), g("Nat"))),
        plus_body,
    );
    ctx.add_definition(
        "size".to_string(),
        pi("_", g("Nat"), g("Nat")),
        lam(
            "n",
            g("Nat"),
            bin("plus", Term::Var("n".to_string()), g("Zero")),
        ),
    );
    // `@[simp] theorem size_eq (n : Nat) : size n = n`
    let stmt = pi(
        "n",
        g("Nat"),
        app(
            app(
                app(g("Eq"), g("Nat")),
                app(g("size"), Term::Var("n".to_string())),
            ),
            Term::Var("n".to_string()),
        ),
    );
    ctx.add_declaration("size_eq", stmt);
    for name in ["plus", "size"] {
        assert!(
            ctx.get_definition_body(name).is_some(),
            "the fixture's `{name}` must register: {:?}",
            ctx.refused_declarations()
        );
    }
    let scoped = ctx.extend("n", g("Nat"));
    let nested = app(g("size"), app(g("size"), Term::Var("n".to_string())));
    (scoped, nested)
}

#[test]
fn a_registered_rewrite_rule_closes_what_beta_delta_and_iota_leave_stuck() {
    let (mut ctx, nested) = simp_fixture();
    ctx.add_simp_lemma("size_eq")
        .expect("`∀n. size n = n` orients left to right");
    assert_eq!(
        normalize(&ctx, &nested),
        Term::Var("n".to_string()),
        "a registered `@[simp]` rule must be applied by the reducer at every subterm it \
         matches; `size (size n)` is stuck at {} without it",
        normalize(&std_ctx(), &nested)
    );
}

#[test]
fn an_unregistered_equation_leaves_the_reducer_exactly_where_it_was() {
    let (ctx, nested) = simp_fixture();
    // `size_eq` is declared, true, and in scope — and contributes NOTHING, because registration
    // is what `@[simp]` MEANS. A reducer that read every declared equation would make the
    // attribute a decoration and would rewrite with lemmas the author never offered it.
    let without = normalize(&ctx, &nested);
    assert_ne!(
        without,
        Term::Var("n".to_string()),
        "an unregistered lemma must not rewrite, however true it is"
    );
    assert!(
        ctx.simp_rules().is_empty(),
        "…and it must not be in the registry either"
    );
}

#[test]
fn a_rule_registered_for_a_different_head_never_fires() {
    let (mut ctx, nested) = simp_fixture();
    // `bulk n := size n`, and the rule is about `bulk`. Dispatch is by the LEFT-HAND SIDE's head
    // symbol, so a `bulk` rule must not fire on a `size` application even though the two are
    // definitionally equal — a rewriter that matched up to conversion would rewrite everything.
    ctx.add_definition(
        "bulk".to_string(),
        pi("_", g("Nat"), g("Nat")),
        lam("n", g("Nat"), app(g("size"), Term::Var("n".to_string()))),
    );
    let stmt = pi(
        "n",
        g("Nat"),
        app(
            app(
                app(g("Eq"), g("Nat")),
                app(g("bulk"), Term::Var("n".to_string())),
            ),
            g("Zero"),
        ),
    );
    ctx.add_declaration("bulk_eq", stmt);
    ctx.add_simp_lemma("bulk_eq")
        .expect("an equation about `bulk`");
    assert_eq!(
        ctx.simp_rules_for("size").count(),
        0,
        "the registry dispatches on the head symbol and `size` has no rule"
    );
    assert_ne!(
        normalize(&ctx, &nested),
        g("Zero"),
        "a rule keyed on `bulk` must never fire on a `size` application"
    );
}

#[test]
fn a_rule_whose_pattern_cannot_bind_consistently_declines_rather_than_guessing() {
    let mut ctx = std_ctx();
    // `dup : Nat → Nat → Nat` and `@[simp] dup_eq (x : Nat) : dup x x = Zero`. The pattern is
    // NON-LINEAR, so it may only fire when both arguments are the SAME term.
    ctx.add_declaration("dup", pi("_", g("Nat"), pi("_", g("Nat"), g("Nat"))));
    let x = || Term::Var("x".to_string());
    let stmt = pi(
        "x",
        g("Nat"),
        app(app(app(g("Eq"), g("Nat")), bin("dup", x(), x())), g("Zero")),
    );
    ctx.add_declaration("dup_eq", stmt);
    ctx.add_simp_lemma("dup_eq")
        .expect("a non-linear pattern is still a rule");

    let a = ctx.extend("a", g("Nat")).extend("b", g("Nat"));
    let same = bin(
        "dup",
        Term::Var("a".to_string()),
        Term::Var("a".to_string()),
    );
    let different = bin(
        "dup",
        Term::Var("a".to_string()),
        Term::Var("b".to_string()),
    );
    assert_eq!(
        normalize(&a, &same),
        g("Zero"),
        "`dup a a` matches `dup x x`"
    );
    assert_eq!(
        normalize(&a, &different),
        different,
        "`dup a b` does NOT — a matcher that bound `x` twice to two different terms would \
         substitute a variable the rule never determined"
    );
}

#[test]
fn a_scoped_rewrite_rule_stops_rewriting_when_its_scope_closes() {
    let (mut ctx, nested) = simp_fixture();
    let stuck = normalize(&ctx, &nested);
    ctx.push_scope();
    ctx.add_local_simp_lemma("size_eq")
        .expect("a `scoped @[simp]` lemma");
    assert_eq!(
        normalize(&ctx, &nested),
        Term::Var("n".to_string()),
        "inside the scope the rule fires"
    );
    ctx.pop_scope();
    assert_eq!(
        normalize(&ctx, &nested),
        stuck,
        "and OUTSIDE it the reducer is back where it was — which is why the rules are read \
         from the `Context` on every call and never remembered across one"
    );
}

/// BOTH KERNELS, and the honest statement of where they currently stand.
///
/// Wiring the registry into `reduce_step_opt` extends what the FIRST kernel calls a conversion.
/// The SECOND kernel — `recheck.rs::whnf`, a different file and a different lane — does not read
/// the registry yet, so a proof that LEANS on a rewrite is accepted by the main kernel and not
/// confirmed by the re-checker. That is an incompleteness in the second checker, not a
/// contradiction about what is true, and the exact one-line patch that closes it is written out
/// in this wave's hand-off (`Context::simp_rules_for` + `SimpRule::rewrite` over `from_db`).
///
/// What this test pins is the half of the two-kernel invariant that must hold WHATEVER either
/// kernel can reach: the re-checker may never accept what the main kernel refuses, and the
/// registry may never change that. The direction that is currently one-sided is stated in
/// `needs_from_other_lanes` rather than asserted here, because asserting it either way would
/// pin a transient.
#[test]
fn the_registry_never_lets_the_re_checker_accept_what_the_main_kernel_refuses() {
    use logicaffeine_kernel::{double_check, DoubleCheck};
    let (mut ctx, _nested) = simp_fixture();
    ctx.add_simp_lemma("size_eq").expect("registered");
    // `(λp : Π(n:Nat). Eq Nat (size (size n)) n. p) (λn. refl Nat n)` — CLOSED, so the second
    // kernel has the whole term, and the application type-checks ONLY if conversion sees
    // through the rewrite under the binder.
    let n = || Term::Var("n".to_string());
    let stated = pi(
        "n",
        g("Nat"),
        app(
            app(app(g("Eq"), g("Nat")), app(g("size"), app(g("size"), n()))),
            n(),
        ),
    );
    let witness = lam("n", g("Nat"), app(app(g("refl"), g("Nat")), n()));
    let cited = app(
        lam("p", stated.clone(), Term::Var("p".to_string())),
        witness,
    );
    assert!(
        infer_type(&ctx, &cited).is_ok(),
        "the main kernel accepts the citation once the rule is registered"
    );
    let verdict = double_check(&ctx, &cited);
    assert!(
        !matches!(&verdict, DoubleCheck::Disagree(d) if d.contains("re-checker accepted")),
        "the re-checker may never accept what the main kernel refuses: {verdict:?}"
    );

    // …and the registry is what made the difference: WITHOUT it the citation is refused by the
    // main kernel too, so the extension is the rule firing rather than a hole in the checker.
    let (plain, _) = simp_fixture();
    assert!(
        infer_type(&plain, &cited).is_err(),
        "an unregistered lemma must leave the citation unprovable"
    );
    let plain_verdict = double_check(&plain, &cited);
    assert!(
        !matches!(&plain_verdict, DoubleCheck::Disagree(_)),
        "and with no rule registered the two kernels agree about it: {plain_verdict:?}"
    );
}

// =============================================================================
// WAVE 3 · SECTION 2 — DEFECT-5's remaining half: ONE well-formedness test for `Const`
//
// `infer_type` decides whether `name.{levels}` is a term of the calculus with
// `term::resolve_const_levels`, which checks TWO things: that the declaration's type leaves no
// level variable outside its telescope, and that the reference carries exactly as many levels
// as the telescope has. The reducer used to ask only the second question, and only of a
// declaration in the universe-polymorphic map — so there were two well-formedness tests for one
// judgement, and they part company in both directions.
// =============================================================================

/// WHERE THE LINE IS, and why it is there. A reference that does not RESOLVE — an unknown name,
/// or a declaration with no level telescope, which is reached as a `Term::Global` and never as a
/// `Const` — is an UNBOUND reference, and reduction leaves an unbound reference alone in exactly
/// the way it leaves an unbound `Term::Global` alone. Boundness is the type checker's verdict,
/// not the reducer's, and the two have always differed there.
///
/// What may never happen is the other thing: a reference that DOES resolve, is ill-formed
/// against the declaration it resolved to, and is nevertheless reduced — see the two tests
/// below. That is the divergence, and it is a different fact from this one.
#[test]
fn an_unresolvable_const_is_left_alone_exactly_as_an_unbound_global_is() {
    let ctx = std_ctx();
    for name in ["Nat", "neverHeardOfIt"] {
        for levels in [vec![], vec![logicaffeine_kernel::Universe::Type(0u32)]] {
            let reference = Term::Const {
                name: name.to_string(),
                levels: levels.clone(),
            };
            assert!(
                infer_type(&ctx, &reference).is_err(),
                "`{name}` has no level telescope, so `{name}.{{…}}` is unbound"
            );
            assert_eq!(
                normalize(&ctx, &reference),
                reference,
                "…and an unbound reference is inert, as `Global(\"neverHeardOfIt\")` is"
            );
        }
    }
    let unbound_global = g("neverHeardOfIt");
    assert_eq!(
        normalize(&ctx, &unbound_global),
        unbound_global,
        "the behaviour being matched, stated so the two cannot drift apart"
    );
}

/// The OTHER direction, and the one that laundered: a declaration whose TYPE mentions a level
/// variable its telescope does not bind can never be instantiated, so `infer_type` refuses
/// every reference to it. The reducer's own arity test passes (`0 == 0`) and it used to unfold
/// the body — turning a reference the type checker refuses into a well-typed term.
#[test]
fn a_const_whose_declaration_leaves_a_level_free_is_refused_by_the_reducer_too() {
    use logicaffeine_kernel::Universe;
    let mut ctx = std_ctx();
    // `def leaky : Sort v := Nat` with NO universe parameters — `v` is free in the type.
    ctx.add_universe_poly(
        "leaky",
        vec![],
        Term::Sort(Universe::Var("v".to_string())),
        g("Nat"),
    );
    let leaky = Term::Const {
        name: "leaky".to_string(),
        levels: vec![],
    };
    assert!(
        infer_type(&ctx, &leaky).is_err(),
        "`term::resolve_const_levels` refuses a declaration whose type is not level-closed"
    );
    let nf = normalize(&ctx, &leaky);
    assert_ne!(
        nf,
        g("Nat"),
        "the reducer must NOT unfold a reference typing refuses: got {nf}"
    );
    assert!(
        infer_type(&ctx, &nf).is_err(),
        "and it must not produce a well-typed term from one: got {nf}"
    );
}

/// The two halves of the judgement, measured against each other on a grid that contains every
/// shape the resolver distinguishes. The property is not "both succeed" — it is that the
/// reducer NEVER calls a reference a normal form that the type checker refuses, and never
/// produces a well-typed term from one it refuses.
#[test]
fn the_reducer_never_settles_a_const_reference_the_type_checker_refuses() {
    use logicaffeine_kernel::Universe;
    let mut ctx = std_ctx();
    let u = Universe::Var("u".to_string());
    ctx.add_universe_poly(
        "polyId",
        vec!["u".to_string()],
        pi(
            "A",
            Term::Sort(u.clone()),
            pi("_", Term::Var("A".to_string()), Term::Var("A".to_string())),
        ),
        lam(
            "A",
            Term::Sort(u),
            lam("a", Term::Var("A".to_string()), Term::Var("a".to_string())),
        ),
    );
    ctx.add_universe_poly(
        "leaky",
        vec![],
        Term::Sort(Universe::Var("v".to_string())),
        g("Nat"),
    );
    let lv = |ls: &[u32]| ls.iter().map(|n| Universe::Type(*n)).collect::<Vec<_>>();
    let grid: Vec<(&str, Vec<Universe>)> = vec![
        ("polyId", vec![]),
        ("polyId", lv(&[0])),
        ("polyId", lv(&[1])),
        ("polyId", lv(&[0, 0])),
        ("polyId", lv(&[0, 1, 2])),
        ("leaky", vec![]),
        ("leaky", lv(&[0])),
        ("Nat", vec![]),
        ("Nat", lv(&[0])),
        ("Zero", lv(&[0])),
        ("Eq", lv(&[0])),
    ];
    for (name, levels) in grid {
        let t = Term::Const {
            name: name.to_string(),
            levels: levels.clone(),
        };
        let typed = infer_type(&ctx, &t).is_ok();
        let nf = normalize(&ctx, &t);
        if !typed {
            // THE PROPERTY, and it holds for every row whether the reference resolved or not:
            // reduction may never turn a reference typing refuses into a term typing accepts.
            assert!(
                infer_type(&ctx, &nf).is_err(),
                "`{name}.{levels:?}` is refused by typing, so its reduct must be too: {nf}"
            );
        }
        // …and a reference that RESOLVES and is ill-formed against its declaration is never
        // handed back as a settled normal form.
        let resolves = ctx.get_universe_poly(name).is_some();
        if resolves && !typed {
            assert_ne!(
                nf, t,
                "`{name}.{levels:?}` resolves and is ill-formed — not a normal form"
            );
        }
    }
}

// =============================================================================
// WAVE 3 · SECTION 3 — ι for a fixpoint, at the column the GUARD admitted it on
//
// The specification is `termination::structural_columns`': `(fix f. λx₀ … λxₙ. e) a₀ … aₙ`
// unrolls exactly when the argument at the STRUCTURAL position is in constructor form. The
// reducer used to ask the question of whatever argument the INNERMOST `App` happened to carry,
// which is always `a₀` — a strictly smaller rule, and the definition it leaves stuck is the
// commonest one there is.
// =============================================================================

/// `natAdd := fix f. λm. λn. match n { Zero ⇒ m | Succ k ⇒ Succ (f m k) }` — Lean's `Nat.add`,
/// which recurses on its SECOND argument. Returned with the fixpoint body so a test can ask the
/// guard which column it admitted rather than assuming one.
fn nat_add_fixture() -> (Context, Term, Term) {
    let mut ctx = std_ctx();
    let v = |n: &str| Term::Var(n.to_string());
    let body = lam(
        "m",
        g("Nat"),
        lam(
            "n",
            g("Nat"),
            Term::Match {
                discriminant: Box::new(v("n")),
                motive: Box::new(lam("_", g("Nat"), g("Nat"))),
                cases: vec![
                    v("m"),
                    lam(
                        "k",
                        g("Nat"),
                        app(g("Succ"), app(app(v("natAdd"), v("m")), v("k"))),
                    ),
                ],
            },
        ),
    );
    let fix = Term::Fix {
        name: "natAdd".to_string(),
        body: Box::new(body.clone()),
    };
    ctx.add_definition(
        "natAdd".to_string(),
        pi("_", g("Nat"), pi("_", g("Nat"), g("Nat"))),
        fix.clone(),
    );
    (ctx, fix, body)
}

#[test]
fn the_guard_admits_nat_add_on_its_second_column_and_the_reducer_unrolls_there() {
    let (ctx, _fix, body) = nat_add_fixture();
    let columns = logicaffeine_kernel::termination::structural_columns(&ctx, &body);
    assert_eq!(
        columns.first().copied(),
        Some(1),
        "the guard reads the structural column off the body's own `match`, and `natAdd` \
         recurses on argument 1 — if this ever says 0 the reducer below is measuring the \
         wrong thing, not the guard"
    );

    let scoped = ctx.extend("x", g("Nat"));
    let x = Term::Var("x".to_string());
    // `x + 0 = x` — `rfl` in Lean, and the whole point of the column.
    assert_eq!(
        normalize(&scoped, &bin("natAdd", x.clone(), g("Zero"))),
        x,
        "the structural argument is `Zero`, a constructor, so ι fires however opaque `m` is"
    );
    // …and one rung up, to show the recursion really runs rather than the base case alone.
    assert_eq!(
        normalize(
            &scoped,
            &bin("natAdd", x.clone(), app(g("Succ"), g("Zero")))
        ),
        app(g("Succ"), x.clone()),
        "`x + 1` unrolls once and lands on the base case"
    );
}

#[test]
fn a_fixpoint_never_unrolls_on_a_non_structural_argument() {
    let (ctx, _fix, _body) = nat_add_fixture();
    let scoped = ctx.extend("x", g("Nat")).extend("y", g("Nat"));
    let x = Term::Var("x".to_string());
    let y = Term::Var("y".to_string());

    // Whether the recursion UNROLLED, asked of the answer rather than of the input: δ is
    // allowed to replace the name by its fixpoint (that is progress, and it happens), so
    // "unchanged" is the wrong measure. What may not happen is the fixpoint being CONSUMED.
    let never_unrolled = |t: &Term| {
        let mut cur = t;
        while let Term::App(f, _) = cur {
            cur = f;
        }
        matches!(cur, Term::Fix { .. })
    };

    // THE NEGATIVE THAT SEPARATES "the structural column" FROM "some argument". Argument 0 is
    // a constructor and argument 1 — the structural one — is not: unrolling here would make
    // progress the guard never promised, and the recursion would never reach a `match` it can
    // decide.
    let ctor_in_the_wrong_column = bin("natAdd", g("Zero"), y.clone());
    let answer = normalize(&ctx.extend("y", g("Nat")), &ctor_in_the_wrong_column);
    assert!(
        never_unrolled(&answer),
        "`0 + y` is stuck: the STRUCTURAL argument is `y`, and it is not in constructor form —          the constructor in column 0 must not unroll anything. Got {answer}"
    );

    // Neither argument is a constructor: stuck on both counts.
    let neither = bin("natAdd", x, y);
    let answer = normalize(&scoped, &neither);
    assert!(
        never_unrolled(&answer),
        "`x + y` has no constructor anywhere: {answer}"
    );
}

#[test]
fn a_fixpoint_applied_below_its_structural_column_is_left_alone() {
    let (ctx, fix, _body) = nat_add_fixture();
    // A PARTIAL application: the structural column is 1 and only one argument is present, so
    // there is no structural argument to be in constructor form and nothing may unroll.
    let partial = app(fix, g("Zero"));
    assert_eq!(
        normalize(&ctx, &partial),
        partial,
        "a fixpoint applied to fewer arguments than its structural column has no redex"
    );
}

// =============================================================================
// WAVE 3 · SECTION 4 — the quotient eliminators COMPUTE, both of them
//
// `Quot_lift` had its ι-rule from the start. `Quot_rec` — the DEPENDENT eliminator — was first
// declared with none, which is an axiom wearing a recursor's name: it type-checks and never
// computes, so every `rfl` over it is FALSE rather than unproved. It is now Lean's own DEFINITION
// (`Init/Core.lean:1581-1611`) and computes by δ through `Quot_lift`'s rule.
// =============================================================================

/// `Quot_rec A r motive f h (Quot_mk A r a) ⤳ f a`, alongside its `Quot_lift` sibling so the
/// two are measured against each other rather than each against itself.
#[test]
fn both_quotient_eliminators_compute_on_a_class_of_a_representative() {
    let mut ctx = std_ctx();
    ctx.add_declaration(
        "R",
        pi(
            "_",
            g("Nat"),
            pi(
                "_",
                g("Nat"),
                Term::Sort(logicaffeine_kernel::Universe::Prop),
            ),
        ),
    );
    ctx.add_declaration("h", g("Nat"));
    let a = natl(3);
    let mk = |x: Term| app(app(app(g("Quot_mk"), g("Nat")), g("R")), x);
    let f = lam("z", g("Nat"), app(g("Succ"), Term::Var("z".to_string())));
    let apps = |head: &str, args: Vec<Term>| args.into_iter().fold(g(head), |acc, x| app(acc, x));
    let lifted = apps(
        "Quot_lift",
        vec![g("Nat"), g("R"), g("Nat"), f.clone(), g("h"), mk(a.clone())],
    );
    let recursed = apps(
        "Quot_rec",
        vec![
            g("Nat"),
            g("R"),
            lam("_", g("Nat"), g("Nat")),
            f.clone(),
            g("h"),
            mk(a.clone()),
        ],
    );
    let want = app(g("Succ"), a);
    assert_eq!(
        normalize(&ctx, &lifted),
        want,
        "`Quot_lift … (Quot_mk a) ⤳ f a` — the control"
    );
    assert_eq!(
        normalize(&ctx, &recursed),
        want,
        "…and the DEPENDENT eliminator must compute the same way; a declared `Quot_rec` with \
         no ι-rule is an axiom wearing a recursor's name"
    );
}

#[test]
fn a_quotient_eliminator_is_stuck_on_anything_that_is_not_a_class() {
    let mut ctx = std_ctx();
    ctx.add_declaration(
        "R",
        pi(
            "_",
            g("Nat"),
            pi(
                "_",
                g("Nat"),
                Term::Sort(logicaffeine_kernel::Universe::Prop),
            ),
        ),
    );
    ctx.add_declaration("h", g("Nat"));
    ctx.add_declaration("q", g("Nat"));
    let f = lam("z", g("Nat"), app(g("Succ"), Term::Var("z".to_string())));
    let apps = |head: &str, args: Vec<Term>| args.into_iter().fold(g(head), |acc, x| app(acc, x));
    // The major premise is an opaque `q`, not a `Quot_mk`: nothing may fire, in either
    // eliminator. A rule that fired here would compute `f` at a representative nobody chose.
    let lifted = apps(
        "Quot_lift",
        vec![
            g("Nat"),
            g("R"),
            lam("_", g("Nat"), g("Nat")),
            f.clone(),
            g("h"),
            g("q"),
        ],
    );
    assert_eq!(
        normalize(&ctx, &lifted),
        lifted,
        "`Quot_lift` must be stuck on a major premise that is not a class"
    );
    // `Quot_rec` is a DEFINITION, so δ unfolds it and the question moves inside: with its δ
    // withheld the spine is a normal form exactly as it stands (no rule is keyed on its name),
    // and unfolded, the quotient eliminators its body goes through are still stuck on `q`.
    let recursed = apps(
        "Quot_rec",
        vec![
            g("Nat"),
            g("R"),
            lam("_", g("Nat"), g("Nat")),
            f.clone(),
            g("h"),
            g("q"),
        ],
    );
    let folded = |n: &str| n == "Quot_rec";
    assert_eq!(
        normalize_at(&ctx, &recursed, Transparency::Excluding(&folded)),
        recursed,
        "`Quot_rec` must be stuck on a major premise that is not a class"
    );
    let unfolded = normalize(&ctx, &recursed);
    for (eliminator, arity) in [("Quot_lift", 6), ("Quot_ind", 5)] {
        assert!(
            stuck_on(&unfolded, eliminator, arity, &g("q")),
            "unfolded, `Quot_rec` must leave `{eliminator}` stuck on the opaque `q`: {unfolded}"
        );
    }
}

/// Whether `t` contains a saturated `head` spine (`arity` arguments) whose last argument is `major`.
fn stuck_on(t: &Term, head: &str, arity: usize, major: &Term) -> bool {
    let mut args = Vec::new();
    let mut h = t;
    while let Term::App(f, a) = h {
        args.push(a.as_ref());
        h = f;
    }
    if matches!(h, Term::Global(n) if n == head) && args.len() == arity && args[0] == major {
        return true;
    }
    match t {
        Term::App(f, a) => stuck_on(f, head, arity, major) || stuck_on(a, head, arity, major),
        Term::Lambda {
            param_type, body, ..
        } => stuck_on(param_type, head, arity, major) || stuck_on(body, head, arity, major),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => stuck_on(param_type, head, arity, major) || stuck_on(body_type, head, arity, major),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            stuck_on(discriminant, head, arity, major)
                || stuck_on(motive, head, arity, major)
                || cases.iter().any(|c| stuck_on(c, head, arity, major))
        }
        Term::Fix { body, .. } => stuck_on(body, head, arity, major),
        Term::Let {
            ty, value, body, ..
        } => {
            stuck_on(ty, head, arity, major)
                || stuck_on(value, head, arity, major)
                || stuck_on(body, head, arity, major)
        }
        _ => false,
    }
}

// =============================================================================
// WAVE 3 · SECTION 5 — the elimination rules CHECK what they concede
//
// Two derivation formers concede `∀x:Ind, P x`. What they were checking first was strictly
// weaker than what they conceded, in two different ways.
// =============================================================================

/// `A Tag is either Zed or a Val with n : Nat.` — a NON-recursive constructor that carries a
/// field. The case formula for `Val` must quantify over the field; substituting the BARE
/// constructor checks `P Val`, a proposition about a partially applied constructor, while the
/// eliminator then concedes every `P (Val n)` that was never proved.
fn tag_ctx() -> Context {
    let mut ctx = std_ctx();
    ctx.add_inductive("Tag", Term::Sort(logicaffeine_kernel::Universe::Type(0u32)));
    ctx.add_constructor("Zed", "Tag", g("Tag"));
    ctx.add_constructor("Val", "Tag", pi("_", g("Nat"), g("Tag")));
    ctx
}

fn sname(s: &str) -> Term {
    app(g("SName"), textl(s))
}
fn sapp(f: Term, x: Term) -> Term {
    app(app(g("SApp"), f), x)
}
fn slam(ty: Term, body: Term) -> Term {
    app(app(g("SLam"), ty), body)
}
fn svar(n: i64) -> Term {
    app(g("SVar"), intl(n))
}
fn concludes(d: Term) -> Term {
    app(g("concludes"), d)
}
fn dcases(proofs: Vec<Term>) -> Term {
    proofs
        .into_iter()
        .rev()
        .fold(g("DCaseEnd"), |tail, p| app(app(g("DCase"), p), tail))
}

#[test]
fn a_field_carrying_constructors_case_formula_quantifies_over_its_field() {
    let ctx = tag_ctx();
    // The motive `λt. P t`, reflected.
    let motive = slam(sname("Tag"), sapp(sname("P"), svar(0)));
    // `induction_step_goal Tag motive 1` is the `Val` case.
    let goal = app(
        app(app(g("induction_step_goal"), sname("Tag")), motive),
        app(g("Succ"), g("Zero")),
    );
    let formula = normalize(&ctx, &goal);
    let bare = sapp(sname("P"), sname("Val"));
    assert_ne!(
        formula, bare,
        "the `Val` case may not be `P Val` — that is a proposition about a partially applied \
         constructor, and it is strictly weaker than the `∀t:Tag, P t` the eliminator concedes"
    );
    assert_eq!(
        formula,
        sapp(
            sapp(sname("Forall"), sname("Nat")),
            slam(sname("Nat"), sapp(sname("P"), sapp(sname("Val"), svar(0))))
        ),
        "it must be `∀n:Nat. P (Val n)`"
    );
}

/// THE SOUNDNESS HOLE, stated as the derivation that exploited it. `try_ddestruct_conclude`
/// checked the case COUNT and nothing else, then conceded `∀x:Ind, P x`.
#[test]
fn a_destruct_derivation_that_proves_nothing_concludes_nothing() {
    let ctx = std_ctx();
    let motive = slam(sname("Bool"), sname("False"));
    // Two case "proofs" of an unrelated proposition — never looked at.
    let fake = app(
        app(app(g("DDestruct"), sname("Bool")), motive.clone()),
        dcases(vec![
            app(g("DAxiom"), sname("True")),
            app(g("DAxiom"), sname("True")),
        ]),
    );
    let conceded = normalize(&ctx, &concludes(fake));
    let forall_bool_false = sapp(sapp(sname("Forall"), sname("Bool")), motive);
    assert_ne!(
        conceded, forall_bool_false,
        "a `destruct` whose case proofs were never examined must not conclude \
         `∀b:Bool, False` — with `DAxiom` in the vocabulary that is a proof of anything"
    );
}

/// …and the POSITIVE, so the refusal above is not simply "destruct never concludes anything".
#[test]
fn a_destruct_derivation_that_does_prove_every_case_still_concludes() {
    let ctx = std_ctx();
    // `∀b:Bool. True`, whose two cases are `True` and `True` — both discharged by `DAxiom`.
    let motive = slam(sname("Bool"), sname("True"));
    let good = app(
        app(app(g("DDestruct"), sname("Bool")), motive.clone()),
        dcases(vec![
            app(g("DAxiom"), sname("True")),
            app(g("DAxiom"), sname("True")),
        ]),
    );
    let conceded = normalize(&ctx, &concludes(good));
    assert_eq!(
        conceded,
        sapp(sapp(sname("Forall"), sname("Bool")), motive),
        "every case really was proved, so the eliminator concedes the quantification"
    );
}

// =============================================================================
// WAVE 3 · SECTION 6 — ONE implication, however it is spelled
//
// The reflected surface has three spellings of `A → B` — `implies`, `Implies`, and a
// non-dependent `SPi` — and each reader knew a different subset. `intro`, `assumption`,
// `decide` and the simp goal reader all matched the lower case one EXACTLY, so an induction
// step case (built capitalised) and the literate `Statement:` surface's infix `implies` (which
// desugars to a `Pi` and reflects as an `SPi`) were both propositions no tactic could read.
// =============================================================================

fn spi(a: Term, b: Term) -> Term {
    app(app(g("SPi"), a), b)
}

/// The SAME proposition in all three spellings, each handed to the same tactic. A reader that
/// knows one of them answers three different things about one proposition.
#[test]
fn every_spelling_of_one_implication_is_read_by_the_same_tactic() {
    let ctx = std_ctx();
    let t = || sname("True");
    let spellings: Vec<(&str, Term)> = vec![
        ("implies", sapp(sapp(sname("implies"), t()), t())),
        ("Implies", sapp(sapp(sname("Implies"), t()), t())),
        ("SPi", spi(t(), t())),
    ];
    for (name, goal) in &spellings {
        // `assumption` introduces the antecedent and closes on it: `True → True` is its
        // canonical shape, and it must succeed for every spelling of the arrow.
        let derivation = normalize(&ctx, &app(g("try_assumption"), goal.clone()));
        assert_eq!(
            derivation,
            app(g("DAutoSolve"), goal.clone()),
            "`assumption` must prove `True → True` spelled `{name}`, got {derivation}"
        );
    }
}

/// THE NEGATIVE that keeps the widened reader narrow: a DEPENDENT `SPi` is a quantifier, not an
/// implication. Discharging its antecedent does not hand you its conclusion, because the
/// conclusion mentions the thing quantified over — and a tactic that read it as an arrow would
/// "introduce" a hypothesis and then prove an open body.
#[test]
fn a_dependent_spi_is_a_quantifier_and_is_not_read_as_an_implication() {
    let ctx = std_ctx();
    let dependent = spi(sname("Nat"), sapp(sname("P"), svar(0)));
    let derivation = normalize(&ctx, &app(g("try_assumption"), dependent.clone()));
    assert_eq!(
        derivation,
        app(g("DAxiom"), sname("Error")),
        "`∀n:Nat. P n` is not `A → B` and `assumption` must not claim it: {derivation}"
    );
}

// =============================================================================
// WAVE 3 · SECTION 7 — the ζ gate, held ALL the way down
//
// The gate keeps a binding whose name occurs more than once. A NEST of such bindings is the
// shape that separates "the gate fires" from "the gate fires once": every binding in
// `let b0 := … in let b1 := pair b0 b0 in … let b15 := pair b14 b14 in pair b15 b15` is used
// twice, so every one must survive — and its eager ζ-expansion is 2^16 nodes, which is what a
// gate that only held the outermost would still produce underneath.
// =============================================================================

fn let_in(name: &str, ty: Term, value: Term, body: Term) -> Term {
    Term::Let {
        name: name.to_string(),
        ty: Box::new(ty),
        value: Box::new(value),
        body: Box::new(body),
    }
}

#[test]
fn every_binding_in_a_nest_of_shared_values_survives_normalisation() {
    let mut ctx = std_ctx();
    ctx.add_declaration("pair", pi("_", g("Nat"), pi("_", g("Nat"), g("Nat"))));
    let base = ctx.extend("u", g("Nat"));
    let v = |n: &str| Term::Var(n.to_string());

    const NEST: usize = 16;
    let mut body = bin("pair", v("b15"), v("b15"));
    for i in (0..NEST).rev() {
        let value = if i == 0 {
            app(g("Succ"), v("u"))
        } else {
            bin("pair", v(&format!("b{}", i - 1)), v(&format!("b{}", i - 1)))
        };
        body = let_in(&format!("b{i}"), g("Nat"), value, body);
    }

    let nf = normalize(&base, &body);
    let mut depth = 0usize;
    let mut walk = &nf;
    while let Term::Let { body, .. } = walk {
        depth += 1;
        walk = body;
    }
    assert_eq!(
        depth, NEST,
        "every binding is used twice, so the gate must hold every one of them; the normal form \
         is {depth} deep and reads {nf}"
    );
}

// =============================================================================
// WAVE 3 · SECTION 8 — a rewrite may never CAPTURE
//
// The price of wiring a rewriter into a NAMED-variable kernel. `SimpRule::rewrite` substitutes
// the matched terms into the rule's right-hand side with a substitution that tracks shadowing of
// the PATTERN VARIABLES — the right check for the pattern variable's own name, and no check at
// all for the free names of the term it was bound to. A lemma whose right-hand side BINDS a name
// therefore turns a free occurrence of that name into a bound one, which is a wrong ANSWER
// inside `normalize`, therefore inside `def_eq`, therefore inside every `rfl`.
// =============================================================================

/// `∀x. g x = (λn. x)` — the constant function. Applied to a local `n`, a capturing rewriter
/// answers `λn. n`, the IDENTITY, and the two are not equal at any argument.
#[test]
fn a_rewrite_that_would_capture_a_free_name_is_declined() {
    let mut ctx = std_ctx();
    let x = || Term::Var("x".to_string());
    let n = || Term::Var("n".to_string());
    ctx.add_declaration("g", pi("_", g("Nat"), pi("_", g("Nat"), g("Nat"))));
    // `g_eq : Π(x:Nat). Eq (Nat → Nat) (g x) (λn:Nat. x)`
    let statement = pi(
        "x",
        g("Nat"),
        app(
            app(app(g("Eq"), pi("_", g("Nat"), g("Nat"))), app(g("g"), x())),
            lam("n", g("Nat"), x()),
        ),
    );
    ctx.add_declaration("g_eq", statement);
    ctx.add_simp_lemma("g_eq")
        .expect("an equation whose right-hand side binds `n`");

    // The SAFE case: the argument mentions no `n`, so nothing can be captured.
    let safe = app(g("g"), g("Zero"));
    assert_eq!(
        normalize(&ctx, &safe),
        lam("n", g("Nat"), g("Zero")),
        "with nothing free to capture the rule fires normally"
    );

    // THE HAZARD: the argument IS the name the right-hand side binds.
    let scoped = ctx.extend("n", g("Nat"));
    let hazard = app(g("g"), n());
    let answer = normalize(&scoped, &hazard);
    assert_ne!(
        answer,
        lam("n", g("Nat"), n()),
        "`g n` must never become the IDENTITY — that is the free `n` being captured by the \\
         rule's own binder, and it is a wrong answer inside `def_eq` and so inside every `rfl`"
    );
    // Declining leaves the term where it was, which is weaker and never wrong.
    assert_eq!(
        answer, hazard,
        "the rewrite is declined rather than performed wrongly"
    );
}

// =============================================================================
// NOT IN THE CORPUS — WHERE A δ-CYCLE CAN STILL COME FROM, AND THE INVARIANT THAT
// SURVIVES THE REPAIR
//
// `Context::declare` now REFUSES a body that would close a δ-cycle (`delta_cycle_through`).
// That is Lean's own rule and exactly the repair
// `two_distinct_delta_cycles_are_never_definitionally_identified` argued for: the fix belongs
// in an environment that cannot be made cyclic, not in the reducer's ANSWER. Lean gets it for
// free — `Environment.addDecl` admits only constants already in the environment, so the δ-graph
// is acyclic by construction, and `Lean/Meta/WHNF.lean`'s `whnfImp` (WHNF.lean:991-992) carries
// NO cycle detection whatever: just `withIncRecDepth`, a recursion-depth backstop, and
// `unfoldDefinition?` (WHNF.lean:760) which never asks whether it has been here before.
//
// Both DEFINITION doors now carry the gate: `Context::add_universe_poly` goes through the same
// checked `declare_definition` as `add_definition`, and `Context::delta_reaches` follows a
// polymorphic body's edges too. One door still builds a reduction with no normal form:
// `Context::add_declaration` + `add_simp_lemma` register a REWRITE, which `reduce_step_opt`
// tries before β/δ/ι and which no acyclicity check looks at (it refuses only a rule that
// rewrites a term to itself).
//
// So the invariant is stated in the shape that survives every repair: a cycle must be
// IMPOSSIBLE TO BUILD **or** IMPOSSIBLE TO MISTAKE FOR A NORMAL FORM — never neither. The δ
// doors are asserted closed below, and the reducer's half is asked of the rewrite door that is
// still open, so the detector's locks keep exercising the mechanism instead of passing over a
// premise that can no longer be built.
// =============================================================================

/// The polymorphic door refuses the δ-cycle `polyA.{u} := polyB.{u}`, `polyB.{u} := polyA.{u}`
/// exactly as the monomorphic one does, and records the refusal. The reducer's half is then asked
/// through a polymorphic constant whose unfolding ENTERS a `@[simp]` rewrite cycle, at three
/// different levels so the answer cannot be a single memoised accident.
#[test]
fn a_universe_polymorphic_delta_cycle_is_reported_cyclic_at_every_level_it_is_asked_at() {
    use logicaffeine_kernel::Universe;
    let mut ctx = std_ctx();
    let u = || Universe::Var("u".to_string());
    let cref = |n: &str, l: Universe| Term::Const {
        name: n.to_string(),
        levels: vec![l],
    };
    ctx.add_universe_poly(
        "polyA",
        vec!["u".to_string()],
        Term::Sort(u()),
        cref("polyB", u()),
    );
    ctx.add_universe_poly(
        "polyB",
        vec!["u".to_string()],
        Term::Sort(u()),
        cref("polyA", u()),
    );
    assert!(
        ctx.get_universe_poly("polyB").is_none(),
        "the polymorphic door refuses the definition that closes a δ-cycle, as the monomorphic one does"
    );
    assert!(
        ctx.refused_declarations().iter().any(|(n, _)| n == "polyB"),
        "…and records it"
    );

    // The reducer's half, through a polymorphic constant whose unfolding ENTERS a rewrite cycle,
    // asked at three different levels so the answer cannot be a single memoised accident.
    let spin = simp_cycle(&mut ctx, "polySpin", 2);
    let points = [app(g(&spin[0]), g("Zero")), app(g(&spin[1]), g("Zero"))];
    ctx.add_universe_poly(
        "polyEnter",
        vec!["u".to_string()],
        g("Nat"),
        points[0].clone(),
    );
    assert!(
        ctx.get_universe_poly("polyEnter").is_some(),
        "{:?}",
        ctx.refused_declarations()
    );
    for level in [Universe::Prop, Universe::Type(0), Universe::Type(7)] {
        let start = cref("polyEnter", level.clone());
        match normalize_checked(&ctx, &start) {
            Err((partial, Unreduced::Cyclic)) => assert!(
                points.contains(&partial),
                "the partial must be a point ON the cycle, got {partial}"
            ),
            other => panic!(
                "`polyEnter` at {level:?} cycles and must be reported Cyclic, got ok={}",
                other.is_ok()
            ),
        }
        // `normalize` stays a FUNCTION: it hands a cycling term back unchanged rather than an
        // arbitrary point on the cycle, which would make every conversion built on it unstable.
        let once = normalize(&ctx, &start);
        assert_eq!(
            once, start,
            "`normalize` hands a cycling term back unchanged"
        );
        assert_eq!(normalize(&ctx, &once), once, "…and is idempotent on it");
    }

    // A cycle UNDER a constructor is found too — the detector is not a head-position special
    // case, and the tortoise has to travel through the rebuilt spine to see it.
    let wrapped = app(g("Succ"), cref("polyEnter", Universe::Type(0)));
    assert!(
        matches!(
            normalize_checked(&ctx, &wrapped),
            Err((_, Unreduced::Cyclic))
        ),
        "a cycle beneath a constructor has no normal form either"
    );

    // Two UNRELATED cycles must never be identified: collapsing every non-normalising term to
    // one answer would let `refl` prove an equation between two well-typed terms.
    let other = simp_cycle(&mut ctx, "polyTwirl", 2);
    ctx.add_universe_poly(
        "polyEnterC",
        vec!["u".to_string()],
        g("Nat"),
        app(g(&other[0]), g("Zero")),
    );
    assert!(
        !defeq_for_test(
            &ctx,
            &cref("polyEnter", Universe::Type(0)),
            &cref("polyEnterC", Universe::Type(0))
        ),
        "two unrelated non-normalising constants may never be found convertible"
    );
}

/// BRENT'S SCHEDULE, at a period the two-cycle above cannot reach: a cycle of period THREE
/// entered after a TAIL of two steps, so the tortoise must be re-saved before any comparison
/// can succeed. A detector that only ever compared against step zero passes every other test
/// here and fails this one. The polymorphic door refuses a δ 3-cycle, so the tail is two
/// polymorphic δ-steps and the cycle is three `@[simp]` rewrites.
#[test]
fn a_polymorphic_cycle_of_period_three_behind_a_tail_is_still_reported_cyclic() {
    use logicaffeine_kernel::Universe;
    let mut ctx = std_ctx();
    let u = || Universe::Var("u".to_string());
    let cref = |n: &str, l: Universe| Term::Const {
        name: n.to_string(),
        levels: vec![l],
    };
    let poly = |ctx: &mut Context, n: &str, to: &str| {
        ctx.add_universe_poly(n, vec!["u".to_string()], Term::Sort(u()), cref(to, u()));
    };
    // The polymorphic door refuses the definition that closes a δ 3-cycle.
    poly(&mut ctx, "pDcyc0", "pDcyc1");
    poly(&mut ctx, "pDcyc1", "pDcyc2");
    poly(&mut ctx, "pDcyc2", "pDcyc0");
    assert!(
        ctx.get_universe_poly("pDcyc2").is_none(),
        "`pDcyc2` closes a δ-cycle"
    );
    assert!(ctx
        .refused_declarations()
        .iter()
        .any(|(n, _)| n == "pDcyc2"));

    // tail: pEnter0 → pEnter1 → pCyc0 Zero (two polymorphic δ-steps);
    // cycle: pCyc0 → pCyc1 → pCyc2 → pCyc0 (rewrites)
    let cyc = simp_cycle(&mut ctx, "pCyc", 3);
    ctx.add_universe_poly(
        "pEnter1",
        vec!["u".to_string()],
        g("Nat"),
        app(g(&cyc[0]), g("Zero")),
    );
    ctx.add_universe_poly(
        "pEnter0",
        vec!["u".to_string()],
        g("Nat"),
        Term::Const {
            name: "pEnter1".to_string(),
            levels: vec![u()],
        },
    );
    let starts = [
        app(g(&cyc[0]), g("Zero")),
        app(g(&cyc[1]), g("Zero")),
        app(g(&cyc[2]), g("Zero")),
        cref("pEnter0", Universe::Type(0)),
        cref("pEnter1", Universe::Type(0)),
    ];
    for t in &starts {
        match normalize_checked(&ctx, t) {
            Err((_, Unreduced::Cyclic)) => {}
            other => panic!(
                "`{t}` reaches a 3-cycle and must say so, got ok={}",
                other.is_ok()
            ),
        }
        assert_eq!(
            &normalize(&ctx, t),
            t,
            "`normalize` stays a function on `{t}`"
        );
    }

    // THE BOUNDARY, one rung the other side of the detector: a δ-chain of the SAME length that
    // TERMINATES must not be called cyclic, or the answer is a budget wearing a cycle's name. Each
    // rung is `Sort u : Sort (u+1)`, well typed at every level, as the checked door requires.
    let above = || Term::Sort(Universe::Succ(Box::new(u())));
    ctx.add_universe_poly("pLin2", vec!["u".to_string()], above(), Term::Sort(u()));
    ctx.add_universe_poly("pLin1", vec!["u".to_string()], above(), cref("pLin2", u()));
    ctx.add_universe_poly("pLin0", vec!["u".to_string()], above(), cref("pLin1", u()));
    for rung in ["pLin2", "pLin1", "pLin0"] {
        assert!(
            ctx.get_universe_poly(rung).is_some(),
            "`{rung}` is well typed and must register: {:?}",
            ctx.refused_declarations()
        );
    }
    assert_eq!(
        normalize_checked(&ctx, &cref("pLin0", Universe::Type(0))),
        Ok(Term::Sort(Universe::Type(0))),
        "three δ-steps that finish are a NORMAL FORM and must be reported as one"
    );

    // The period-ONE δ case: a body that names itself is refused at the polymorphic door too.
    poly(&mut ctx, "pSelf", "pSelf");
    assert!(
        ctx.get_universe_poly("pSelf").is_none(),
        "`a := a` is a δ-cycle of period one"
    );
    assert!(ctx.refused_declarations().iter().any(|(n, _)| n == "pSelf"));
}

/// THE INVARIANT ITSELF, asked of EVERY door that can register a body — stated so it stays
/// green whichever way each door is repaired. For each door: build a two-cycle, then either the
/// door refused it (and the refusal is RECORDED, not swallowed) or the reducer reports the
/// cycle. "Accepted and reported as a normal form" is the one outcome that is never allowed.
#[test]
fn no_door_that_registers_a_body_can_leave_a_delta_cycle_looking_like_a_normal_form() {
    use logicaffeine_kernel::Universe;

    // ---- door 1: the ordinary definition door, which now refuses ----
    let mut ctx = std_ctx();
    ctx.add_definition("door_a".to_string(), g("Nat"), g("door_b"));
    ctx.add_definition("door_b".to_string(), g("Nat"), g("door_a"));
    match ctx.get_definition_body("door_b") {
        None => assert!(
            ctx.refused_declarations()
                .iter()
                .any(|(n, _)| n == "door_b"),
            "a door that refuses must SAY it refused — a silently dropped definition is a \
             constant that looks like an axiom and is not one: {:?}",
            ctx.refused_declarations()
        ),
        Some(_) => {
            assert!(
                matches!(
                    normalize_checked(&ctx, &g("door_a")),
                    Err((_, Unreduced::Cyclic))
                ),
                "the definition door admitted the cycle, so the reducer must report it"
            );
            // A cycle the door ADMITTED must not then be laundered into a conversion: nothing
            // finished, so nothing certified the two sides equal. (When the door refuses, the
            // question does not arise — `door_a := door_b` is then a one-step δ to an unbound
            // name, and `door_a ≡ door_b` is simply TRUE.)
            assert!(
                !defeq_for_test(&ctx, &g("door_a"), &g("door_b")),
                "an unfinished reduction may not be the evidence for a conversion"
            );
        }
    }
    // …and a chain that TERMINATES is still accepted by that door, which is the boundary the
    // acyclicity check must not overshoot.
    ctx.add_definition("fine1".to_string(), g("Nat"), natl(4));
    ctx.add_definition("fine0".to_string(), g("Nat"), g("fine1"));
    assert_eq!(normalize_checked(&ctx, &g("fine0")), Ok(natl(4)));

    // ---- door 2: the universe-polymorphic door ----
    let mut pctx = std_ctx();
    let u = || Universe::Var("u".to_string());
    let cref = |n: &str, l: Universe| Term::Const {
        name: n.to_string(),
        levels: vec![l],
    };
    pctx.add_universe_poly(
        "dp_a",
        vec!["u".to_string()],
        Term::Sort(u()),
        cref("dp_b", u()),
    );
    pctx.add_universe_poly(
        "dp_b",
        vec!["u".to_string()],
        Term::Sort(u()),
        cref("dp_a", u()),
    );
    if pctx.get_universe_poly("dp_b").is_some() {
        assert!(
            matches!(
                normalize_checked(&pctx, &cref("dp_a", Universe::Type(0))),
                Err((_, Unreduced::Cyclic))
            ),
            "the polymorphic door admitted the cycle, so the reducer must report it"
        );
    }

    // ---- door 3: the `@[simp]` registry, which `reduce_step_opt` consults BEFORE β/δ/ι ----
    // Two axioms that rewrite each other. Neither is a DEFINITION, so no acyclicity check in
    // the world looks at them; the rewrite loop is nevertheless a reduction with no normal form.
    let mut sctx = std_ctx();
    let nat_to_nat = pi("_", g("Nat"), g("Nat"));
    sctx.add_declaration("simpF", nat_to_nat.clone());
    sctx.add_declaration("simpG", nat_to_nat);
    let eqn = |l: &str, r: &str| {
        pi(
            "x",
            g("Nat"),
            app(
                app(
                    app(g("Eq"), g("Nat")),
                    app(g(l), Term::Var("x".to_string())),
                ),
                app(g(r), Term::Var("x".to_string())),
            ),
        )
    };
    sctx.add_declaration("simpFG", eqn("simpF", "simpG"));
    sctx.add_declaration("simpGF", eqn("simpG", "simpF"));
    let registered = sctx.add_simp_lemma("simpFG").is_ok() && sctx.add_simp_lemma("simpGF").is_ok();
    assert!(
        registered,
        "the `@[simp]` registry takes both orientations today — if it ever stops, this door \
         closed and the reducer's half below is what must keep being true"
    );
    if registered {
        let spun = app(g("simpF"), natl(1));
        match normalize_checked(&sctx, &spun) {
            Err((_, Unreduced::Cyclic)) => {}
            other => panic!(
                "two simp lemmas that rewrite each other have NO normal form; the door \
                 registered them, so the reducer must report the cycle, got ok={}",
                other.is_ok()
            ),
        }
        assert_eq!(
            normalize(&sctx, &spun),
            spun,
            "`normalize` stays a function on a rewrite loop too"
        );
    }
}

// =============================================================================
// THE INT PRIMITIVES AND THE OPERAND KINDS — the invariant that is actually about soundness
//
// `the_int_primitives_refuse_nat_and_text_operands` above asks that the nine Int names stay
// SYNTACTICALLY STUCK on a `Nat` or `Text` operand. Six of them do, and must: `add`, `sub`,
// `mul`, `div`, `mod` and `le` are opaque DECLARATIONS, so the only thing that could fire is the
// computation rule, and `term::lit_bigint` refuses a `Literal::Nat` — which is what stops
// `sub 3 8` at `Nat` from computing `-5` instead of the prelude's truncating subtraction.
//
// `lt`, `ge` and `gt` are DEFINITIONS, and Lean defines them the same way:
// `Init/Data/Int/Basic.lean:180` is `protected def lt (a b : Int) : Prop := (a + 1) ≤ b`, and
// `Init/Prelude.lean:1655` is `protected def Nat.lt (n m : Nat) : Prop := n + 1 ≤ m`. A
// definition unfolds, and δβ in Lean's
// kernel is UNTYPED — `Lean/Meta/WHNF.lean:628-632` beta-reduces on `f'.isLambda` alone and
// never looks at the argument's type, because the elaborator has already type-checked. So
// asking the REDUCER to refuse an ill-typed argument is asking it to do the type checker's job,
// and `a_definition_with_a_body_outranks_the_builtin_computation_rule` in this same file says
// the opposite about the same three names.
//
// The invariant that carries the soundness content — and that holds for all nine — is stated
// here: **no Int primitive ever produces an ANSWER from an operand that is not an Int.** A
// stuck term is harmless whatever shape it is stuck in; a computed `-5`, `true` or `false` is
// not. `the_int_primitives_refuse_nat_and_text_operands` is the same claim with an
// over-specified conclusion, and the handoff carries the exact repair.
// =============================================================================

/// No Int primitive computes a VALUE from a non-Int operand — over every name the Int table
/// dispatches on, every mixed pairing, and both directions of the mix.
#[test]
fn no_int_primitive_ever_computes_a_value_from_a_nat_or_text_operand() {
    let ctx = std_ctx();
    let answered = |t: &Term| {
        matches!(t, Term::Lit(Literal::Int(_) | Literal::BigInt(_)))
            || matches!(t, Term::Global(n) if n == "true" || n == "false")
    };
    let mut bad: Vec<String> = Vec::new();
    for op in ["add", "sub", "mul", "div", "mod", "le", "lt", "ge", "gt"] {
        for (label, x, y) in [
            ("nat/nat", natl(7), natl(2)),
            ("nat/nat reversed", natl(2), natl(7)),
            // The exact hazard the sibling spec names: at `Nat` this must NOT be `-5`.
            ("the truncating-subtraction trap", natl(3), natl(8)),
            ("int/nat", intl(7), natl(2)),
            ("nat/int", natl(7), intl(2)),
            ("text/text", textl("7"), textl("2")),
            ("int/text", intl(7), textl("2")),
            ("text/int", textl("7"), intl(2)),
        ] {
            let t = bin(op, x, y);
            let n = normalize(&ctx, &t);
            if answered(&n) {
                bad.push(format!("`{op}` on {label} answered {n}"));
            }
            // …and the honest channel agrees it FINISHED — the term is genuinely stuck, not
            // abandoned, which is the difference between "no rule applies" and "gave up".
            if normalize_checked(&ctx, &t).is_err() {
                bad.push(format!("`{op}` on {label} did not finish"));
            }
        }
        // THE POSITIVE, one rung the other side: on two Int literals the very same name DOES
        // answer, so the refusal above is about the operand kinds and not about the rule being
        // dead.
        let live = normalize(&ctx, &bin(op, intl(7), intl(2)));
        if !answered(&live) {
            bad.push(format!(
                "`{op}` on two Int literals must still compute, got {live}"
            ));
        }
    }
    assert!(bad.is_empty(), "{} leaks: {:?}", bad.len(), bad);
}

/// THE NEGATIVE for the three DEFINED orders, stated exactly: they unfold (they are definitions,
/// and Lean's `Int.lt` is a definition too), and what they unfold TO is stuck rather than
/// decided — at Lean's own `Int` `match`, resting on a `Nat` literal it cannot take apart.
#[test]
fn the_defined_int_orders_unfold_on_a_nat_operand_and_then_get_stuck() {
    let ctx = std_ctx();
    for op in ["lt", "ge", "gt"] {
        let t = bin(op, natl(7), natl(2));
        let n = normalize(&ctx, &t);
        assert_ne!(
            n, t,
            "`{op}` is a DEFINITION and δβ is untyped, so it unfolds"
        );
        assert!(
            !matches!(&n, Term::Global(g) if g == "true" || g == "false"),
            "…and what it unfolds to must be STUCK, not a verdict: {n}"
        );
        // Every order is a checked definition over `Int`'s constructors, so the family comes to
        // rest at an `Int` `match` whose scrutinee is one of the `Nat` literals: no ι step fired.
        assert!(
            [natl(7), natl(2)].contains(resting_scrutinee(&n)),
            "`{op}` on a Nat operand must come to rest on a Nat literal: {n}"
        );
        assert!(
            normalize_checked(&ctx, &t).is_ok(),
            "…and it is stuck, not abandoned"
        );
    }
    // On two INT literals the same three names decide, and decide correctly — the acceleration
    // this arrangement buys, not an accident of getting stuck.
    for (op, want) in [("lt", true), ("ge", false), ("gt", false)] {
        assert_eq!(
            normalize(&ctx, &bin(op, intl(2), intl(7))),
            g(if want { "true" } else { "false" }),
            "`{op} 2 7`"
        );
    }
}

// =============================================================================
// T10-016 · THE CAPABILITY, exactly as `lean_conformance_t10_literals::string_concatenation_computes`
// states it — `example : "ab" ++ "cd" = "abcd" := rfl`.
//
// That corpus case asks for TWO contradictory things in one body: that an axiomatised
// `Text_append` "never reduces", and that the `rfl` Lean proves goes through. The first was the
// GAP being recorded; the second is the capability. They cannot both hold, and the capability is
// the one Lean arbitrates — `Init/Data/String/Basic.lean`'s `String.append` computes. The rule
// now exists (`reduction::text_binary`), so the gap sentence is what has to go; the handoff
// carries the one-assertion edit. This is that case's capability half, in a file that owns the
// rule, so the win cannot be lost while the corpus case is being corrected.
// =============================================================================

/// The corpus's own context — `std_ctx` plus a bare `Text_append` AXIOM, no definition — and the
/// four things it asks of it.
#[test]
fn text_concatenation_is_proved_by_rfl_the_way_lean_proves_it() {
    let mut ctx = std_ctx();
    let text = g("Text");
    ctx.add_declaration(
        "Text_append",
        pi("_", text.clone(), pi("_", text.clone(), text)),
    );
    let cat = bin("Text_append", textl("ab"), textl("cd"));

    assert_eq!(
        infer_type(&ctx, &cat).expect("types"),
        g("Text"),
        "the axiom types the term"
    );
    assert_eq!(
        normalize(&ctx, &cat),
        textl("abcd"),
        "…and the rule COMPUTES it"
    );
    assert!(
        defeq_for_test(&ctx, &cat, &textl("abcd")),
        "`\"ab\" ++ \"cd\" = \"abcd\"` by rfl"
    );
    // …and the `refl` proof is one the kernel actually accepts, which is what `rfl` means.
    let eq = |l: Term, r: Term| app(app(app(g("Eq"), g("Text")), l), r);
    let cite = |answer: Term| {
        let claim = eq(cat.clone(), answer.clone());
        app(
            lam("h", claim, Term::Var("h".to_string())),
            app(app(g("refl"), g("Text")), answer),
        )
    };
    assert!(
        infer_type(&ctx, &cite(textl("abcd"))).is_ok(),
        "the true citation is accepted"
    );

    // THE NEGATIVE — computing is not agreeing to anything. The neighbouring answers are
    // refused, including the two that a `++` implemented backwards would produce.
    for wrong in ["cdab", "abc", "abcde", "abdc", ""] {
        assert!(
            !defeq_for_test(&ctx, &cat, &textl(wrong)),
            "`\"ab\" ++ \"cd\"` is not `\"{wrong}\"`"
        );
        assert!(
            infer_type(&ctx, &cite(textl(wrong))).is_err(),
            "…and `rfl` may not claim it"
        );
    }
}

// =============================================================================
// T00-013 / T02-004 · LEAN'S FOUR TRANSPARENCY MODES, and what they are for
//
// Lean's `TransparencyMode` (`Lean/Meta/TransparencyMode.lean:12-16`) has FOUR settings —
// `.all`, `.default`, `.instances`, `.reducible` — and exactly one function decides δ from
// them, `canUnfoldDefault` (`Lean/Meta/GetUnfoldableConst.lean:11-21`):
//
// ```lean
//   match cfg.transparency with
//   | .all     => return true
//   | .default => return !(← isIrreducible info.name)
//   | m        => if (← isReducible info.name) then return true
//                 else if m == .instances && isGlobalInstance (← getEnv) info.name then true
//                 else return false
// ```
//
// LOGOS's axis is ONE constructor, `Transparency::Excluding(pred)`, and the claim these lock is
// that it is EXPRESSIVELY COMPLETE for Lean's four: each mode is a predicate, and `All` is the
// degenerate one. That is a smaller surface for the same behaviour — and it is also why
// `Context::set_reducibility` stores three `Reducibility` settings rather than a boolean: the
// `.reducible`/`.instances` modes need to tell `@[reducible]` from the default, which a single
// `is_irreducible` flag cannot.
//
// The corpus asks these two cases through `normalize`/`defeq_for_test` with NO reducibility hint
// declared at all, which is the kernel's door — and Lean's kernel ignores the attribute, by the
// `.all` line above. That is the finding, and the capability the cases are really about is
// locked below through the door Lean uses for it.
// =============================================================================

/// The DIFFERENTIAL: our δ door against Lean's `canUnfoldDefault`, transcribed from Lean's
/// source and never from ours, over the whole grid of (mode × declared reducibility ×
/// instance-hood). An engine that merely got the two extremes right fails in the middle.
#[test]
fn every_lean_transparency_mode_is_expressible_and_agrees_with_leans_own_can_unfold() {
    use logicaffeine_kernel::{Reducibility, Transparency};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Mode {
        All,
        Default,
        Instances,
        Reducible,
    }

    /// `Lean/Meta/GetUnfoldableConst.lean:11-21`, line for line. The MODEL — independent of
    /// everything under test, which is the only thing that makes agreement evidence.
    fn lean_can_unfold(m: Mode, reducible: bool, irreducible: bool, instance: bool) -> bool {
        match m {
            Mode::All => true,
            Mode::Default => !irreducible,
            other => {
                if reducible {
                    true
                } else {
                    other == Mode::Instances && instance
                }
            }
        }
    }

    // Six constants with IDENTICAL bodies, differing only in their attributes — so anything the
    // door does differently is the attribute and never the name or the term.
    let mut ctx = std_ctx();
    let rows: [(&str, Reducibility, bool); 6] = [
        ("tmRed", Reducibility::Reducible, false),
        ("tmSemi", Reducibility::Semireducible, false),
        ("tmIrr", Reducibility::Irreducible, false),
        ("tmRedInst", Reducibility::Reducible, true),
        ("tmSemiInst", Reducibility::Semireducible, true),
        ("tmIrrInst", Reducibility::Irreducible, true),
    ];
    for (name, r, _) in rows {
        ctx.add_definition(name.to_string(), g("Nat"), natl(5));
        ctx.set_reducibility(name, r);
    }
    let is_instance = |n: &str| n.ends_with("Inst");

    // Lean's four modes AS LOGOS PREDICATES — the mapping under test.
    let reducible_only = |n: &str| ctx.reducibility(n) != Reducibility::Reducible;
    let instances_mode =
        |n: &str| !(ctx.reducibility(n) == Reducibility::Reducible || is_instance(n));
    let default_mode = |n: &str| ctx.is_irreducible(n);
    let modes: [(Mode, Transparency); 4] = [
        (Mode::All, Transparency::All),
        (Mode::Default, Transparency::Excluding(&default_mode)),
        (Mode::Instances, Transparency::Excluding(&instances_mode)),
        (Mode::Reducible, Transparency::Excluding(&reducible_only)),
    ];

    let mut bad: Vec<String> = Vec::new();
    for (mode, tr) in modes {
        for (name, r, inst) in rows {
            let unfolded = normalize_at(&ctx, &g(name), tr) == natl(5);
            let want = lean_can_unfold(
                mode,
                r == Reducibility::Reducible,
                r == Reducibility::Irreducible,
                inst,
            );
            if unfolded != want {
                bad.push(format!(
                    "{mode:?}/{name}: unfolded={unfolded}, Lean says {want}"
                ));
            }
            // Whatever the door decided, the BODY is still there. Opacity is never an axiom.
            if ctx.get_definition_body(name) != Some(&natl(5)) {
                bad.push(format!("{name} lost its body"));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} disagreements with Lean: {:?}",
        bad.len(),
        bad
    );

    // The grid is not vacuous: every mode answers differently from at least one other, so the
    // agreement above is not four copies of the same verdict.
    let seen: Vec<Vec<bool>> = [Mode::All, Mode::Default, Mode::Instances, Mode::Reducible]
        .iter()
        .map(|m| {
            rows.iter()
                .map(|(_, r, inst)| {
                    lean_can_unfold(
                        *m,
                        *r == Reducibility::Reducible,
                        *r == Reducibility::Irreducible,
                        *inst,
                    )
                })
                .collect()
        })
        .collect();
    for i in 0..seen.len() {
        for j in (i + 1)..seen.len() {
            assert_ne!(
                seen[i], seen[j],
                "two modes must not be the same function on this grid"
            );
        }
    }
}

/// Lean's `TransparencyMode.lt` (`Lean/Meta/TransparencyMode.lean:20-27`) is a hand-written
/// six-row table. It must be the SAME order as "unfolds at least as much as", and that is a
/// property nothing in either implementation states — so derive the order from the observed
/// unfold-sets and compare it to Lean's table, row for row.
#[test]
fn the_transparency_modes_are_ordered_the_way_lean_orders_them() {
    use logicaffeine_kernel::{Reducibility, Transparency};
    let mut ctx = std_ctx();
    for (name, r) in [
        ("ordRed", Reducibility::Reducible),
        ("ordSemi", Reducibility::Semireducible),
        ("ordIrr", Reducibility::Irreducible),
    ] {
        ctx.add_definition(name.to_string(), g("Nat"), natl(5));
        ctx.set_reducibility(name, r);
    }
    let names = ["ordRed", "ordSemi", "ordIrr"];
    let reducible_only = |n: &str| ctx.reducibility(n) != Reducibility::Reducible;
    let instances_mode = |n: &str| ctx.reducibility(n) != Reducibility::Reducible;
    let default_mode = |n: &str| ctx.is_irreducible(n);
    // No global instance among them, so `.instances` coincides with `.reducible` HERE — which is
    // exactly what Lean's own `canUnfoldDefault` does when nothing is an instance, and it is the
    // reason the order is `≤` and not `<` between those two on this grid.
    let modes: [(&str, Transparency); 4] = [
        ("reducible", Transparency::Excluding(&reducible_only)),
        ("instances", Transparency::Excluding(&instances_mode)),
        ("default", Transparency::Excluding(&default_mode)),
        ("all", Transparency::All),
    ];
    let unfold_set = |tr: Transparency| -> Vec<&str> {
        names
            .iter()
            .copied()
            .filter(|n| normalize_at(&ctx, &g(n), tr) == natl(5))
            .collect()
    };
    let sets: Vec<(&str, Vec<&str>)> = modes
        .into_iter()
        .map(|(n, tr)| (n, unfold_set(tr)))
        .collect();

    // Lean's table, transcribed: `lt` is true for exactly these six ordered pairs.
    let lean_lt: [(&str, &str); 6] = [
        ("reducible", "default"),
        ("reducible", "all"),
        ("reducible", "instances"),
        ("instances", "default"),
        ("instances", "all"),
        ("default", "all"),
    ];
    let set_of = |name: &str| -> Vec<&str> {
        sets.iter()
            .find(|(n, _)| *n == name)
            .expect("named mode")
            .1
            .clone()
    };
    for (lo, hi) in lean_lt {
        let (a, b) = (set_of(lo), set_of(hi));
        assert!(
            a.iter().all(|n| b.contains(n)),
            "Lean orders `{lo}` below `{hi}`, so everything `{lo}` unfolds `{hi}` must unfold \
             too: {a:?} ⊄ {b:?}"
        );
    }
    // …and the order is not the discrete one: `all` strictly beats `default`, which strictly
    // beats `reducible`. Containment alone would be satisfied by four equal sets.
    assert!(
        set_of("all").len() > set_of("default").len(),
        "`all` unfolds an irreducible"
    );
    assert!(
        set_of("default").len() > set_of("reducible").len(),
        "`default` unfolds a semireducible and `reducible` does not"
    );
    assert_eq!(
        set_of("all"),
        vec!["ordRed", "ordSemi", "ordIrr"],
        "`all` is the kernel's"
    );
    assert_eq!(
        set_of("reducible"),
        vec!["ordRed"],
        "`.reducible` is abbreviations only"
    );
}

/// T02-004's CAPABILITY, in full: `abbrev`, plain `def` and `@[irreducible] def` must be THREE
/// different things, and the corpus case's complaint is that they "collapse onto one another".
/// One context, three constants with byte-identical bodies, three transparencies — and the 3×3
/// table separates all three, which is the thing that was missing.
#[test]
fn an_abbrev_a_def_and_an_irreducible_def_are_three_different_things() {
    use logicaffeine_kernel::{normalize_elab, Reducibility, Transparency};
    let mut ctx = std_ctx();
    for (name, r) in [
        ("NatAbbrev", Reducibility::Reducible),
        ("NatPlain", Reducibility::Semireducible),
        ("NatSealed", Reducibility::Irreducible),
    ] {
        ctx.add_definition(name.to_string(), g("Nat"), g("Zero"));
        ctx.set_reducibility(name, r);
    }
    let reducible_only = |n: &str| ctx.reducibility(n) != Reducibility::Reducible;
    let elaborator = |n: &str| ctx.is_irreducible(n);

    // rows: the three declarations. columns: `.reducible`, `.default`, `.all`.
    let table = [
        ("NatAbbrev", [true, true, true]),
        ("NatPlain", [false, true, true]),
        ("NatSealed", [false, false, true]),
    ];
    for (name, wants) in table {
        let got = [
            normalize_at(&ctx, &g(name), Transparency::Excluding(&reducible_only)) == g("Zero"),
            normalize_at(&ctx, &g(name), Transparency::Excluding(&elaborator)) == g("Zero"),
            normalize(&ctx, &g(name)) == g("Zero"),
        ];
        assert_eq!(got, wants, "`{name}` across .reducible/.default/.all");
    }
    // No two rows agree — which is what "three different things" means, and what T02-004 says
    // LOGOS could not express.
    let row = |n: &str| table.iter().find(|(m, _)| *m == n).expect("row").1;
    assert_ne!(row("NatAbbrev"), row("NatPlain"));
    assert_ne!(row("NatPlain"), row("NatSealed"));
    assert_ne!(row("NatAbbrev"), row("NatSealed"));

    // T02-004's own shape: `@[irreducible] def sealedZero : Nat := 0` must NOT convert with
    // `Zero` where Lean refuses it. At the elaborator's transparency the two sides have
    // DIFFERENT normal forms, which is what a conversion check there reduces to; at the
    // kernel's they convert, exactly as Lean's kernel converts them.
    assert_ne!(
        normalize_elab(&ctx, &g("NatSealed")),
        normalize_elab(&ctx, &g("Zero")),
        "`@[irreducible]` separates them at the elaborator's transparency"
    );
    assert_eq!(
        normalize_elab(&ctx, &g("NatPlain")),
        normalize_elab(&ctx, &g("Zero")),
        "…and a structurally identical plain `def` still converts there"
    );
    assert!(
        defeq_for_test(&ctx, &g("NatSealed"), &g("Zero")),
        "the KERNEL converts them — Lean's does too, and an attribute may not change what is TRUE"
    );
    // The body survives every one of those verdicts: opacity, never a manufactured axiom.
    for (name, _) in table {
        assert_eq!(
            ctx.get_definition_body(name),
            Some(&g("Zero")),
            "`{name}` kept its body"
        );
    }
}

// =============================================================================
// NOT IN THE CORPUS — THE SECOND ENGINE DOES NOT KNOW ABOUT `Text`
//
// `ofReduceBool` is an AXIOM: a `native_decide` proof trusts `eval::eval_bool` to agree with
// `normalize`, and `the_native_evaluator_computes_the_same_euclidean_pair_as_the_reducer` in
// this file pins that agreement over the Int table. The `Text` rules landed in `reduction.rs`
// (`text_binary`, `unary_prim`) and NOT in `eval.rs`, whose `prim_name` lists nine Int names —
// so the evaluator was `Stuck` on exactly the `Text` decisions the reducer answers.
//
// The divergence is in the SAFE direction (the evaluator declines where the reducer decides, so
// `native_decide` refuses a provable goal rather than proving a false one), but a silent gap in
// the agreement an axiom rests on is the last place to leave one: the next person adds a rule to
// one engine and the two part company the other way round. These pin the two engines TOGETHER
// over `Text`, so a rule added to one and not the other goes red.
// =============================================================================

/// Every `Text` decision the reducer answers, the evaluator must answer identically — and the
/// neighbour must be refused, so agreement is by DECIDING and not by both declining.
#[test]
fn the_native_evaluator_decides_the_text_primitives_exactly_as_the_reducer_does() {
    let mut ctx = std_ctx();
    with_text_ops(&mut ctx);
    let words = ["", "a", "ab", "abc", "ba", "héllo", "日本語"];

    let mut bad: Vec<String> = Vec::new();
    for a in words {
        for b in words {
            for op in ["Text_beq", "Text_isPrefixOf"] {
                let t = bin(op, textl(a), textl(b));
                let reducer = bool_of(&normalize(&ctx, &t));
                let evaluator = eval_bool(&ctx, &t);
                if reducer != evaluator {
                    bad.push(format!(
                        "{op}({a:?},{b:?}): reducer {reducer:?}, evaluator {evaluator:?}"
                    ));
                }
                if reducer.is_none() {
                    bad.push(format!(
                        "{op}({a:?},{b:?}): the reducer must decide two literals"
                    ));
                }
            }
            // `Text_append` is not Bool-valued, so it is observed THROUGH a decision — which is
            // also the shape a real proof has: a nested computation under a comparison.
            let nested = bin(
                "Text_beq",
                bin("Text_append", textl(a), textl(b)),
                textl(&format!("{a}{b}")),
            );
            if eval_bool(&ctx, &nested) != Some(true) {
                bad.push(format!(
                    "the evaluator cannot see through append({a:?},{b:?})"
                ));
            }
            let wrong = bin(
                "Text_beq",
                bin("Text_append", textl(a), textl(b)),
                textl(&format!("{b}{a}!")),
            );
            if eval_bool(&ctx, &wrong) != Some(false) {
                bad.push(format!(
                    "…and it accepts a wrong answer for append({a:?},{b:?})"
                ));
            }
        }
        // `Text_length` is UNARY and `Nat`-valued, observed through the accelerated `Nat` order:
        // `length a ≤ n` is true at the exact character count and false one below it.
        let n = a.chars().count() as i64;
        let at = |rel: &str, k: i64| {
            eval_bool(&ctx, &bin(rel, app(g("Text_length"), textl(a)), natl(k)))
        };
        if at("natLe", n) != Some(true) || at("natLt", n) != Some(false) {
            bad.push(format!(
                "the evaluator does not agree `length {a:?}` is {n}"
            ));
        }
        if n > 0 && at("natLe", n - 1) != Some(false) {
            bad.push(format!("…and it also accepts {} for `length {a:?}`", n - 1));
        }
    }
    assert!(
        bad.is_empty(),
        "{} disagreements: {:?}",
        bad.len(),
        &bad[..bad.len().min(10)]
    );
}

/// THE NEGATIVE, and the guard that keeps the two engines honest for the same reason: the
/// evaluator must hold a `Text` primitive to the same declared type and the same
/// definition-outranks-the-builtin rule the reducer does, or `native_decide` would certify a
/// value `normalize` refuses to produce.
#[test]
fn the_evaluator_gives_up_a_text_primitive_wherever_the_reducer_does() {
    // A context that DEFINES `Text_beq` as the constant `false` — the reducer refuses to let
    // its builtin outrank a definition, and the evaluator must refuse in exactly the same way
    // or the two disagree on every equal pair.
    let mut defined = std_ctx();
    with_text_ops(&mut defined);
    let text = g("Text");
    defined.add_definition(
        "Text_beq".to_string(),
        pi("_", text.clone(), pi("_", text.clone(), g("Bool"))),
        lam("a", text.clone(), lam("b", text.clone(), g("false"))),
    );
    let same = bin("Text_beq", textl("ab"), textl("ab"));
    assert_eq!(
        normalize(&defined, &same),
        g("false"),
        "the definition outranks the builtin"
    );
    assert_eq!(
        eval_bool(&defined, &same),
        Some(false),
        "…and the second engine must follow the definition too, not its own rule"
    );

    // A context that declares `Text_beq` at a type the rule's answer does not inhabit. The
    // reducer leaves it alone (`declaration_agrees_with_the_rule`); so must the evaluator.
    let mut mistyped = std_ctx();
    mistyped.add_declaration("Text_beq", pi("_", text.clone(), pi("_", text, g("Nat"))));
    let t = bin("Text_beq", textl("ab"), textl("ab"));
    assert_eq!(
        normalize(&mistyped, &t),
        t,
        "a rule may not fire at a type its declaration denies"
    );
    assert_eq!(
        eval_bool(&mistyped, &t),
        None,
        "…and the evaluator may not either"
    );

    // A non-literal operand is stuck in both engines — the rules fire on two `Text` LITERALS.
    let mut open = std_ctx();
    with_text_ops(&mut open);
    let open = open.extend("s", g("Text"));
    let stuck = bin("Text_beq", Term::Var("s".to_string()), textl("ab"));
    assert_eq!(normalize(&open, &stuck), stuck);
    assert_eq!(eval_bool(&open, &stuck), None);
}

// =============================================================================
// NOT IN THE CORPUS — THE NATIVE-REDUCTION HOOK HAS NONE OF THE GUARDS EVERY OTHER RULE HAS
//
// `reduceBool t` is the ONE door through which the trusted CBV evaluator enters kernel
// reduction, and `ofReduceBool` is the AXIOM that turns its verdict into a proof. Every other
// computation rule in `try_primitive_reduce` passes two gates before it may fire:
//
//   * `ctx.get_definition_body(op).is_some()` — a DEFINITION of the name outranks the builtin
//     (`a_definition_with_a_body_outranks_the_builtin_computation_rule` in this file), because
//     otherwise the reducer answers something the context says is not what the constant means;
//   * `declaration_agrees_with_the_rule(ctx, op)` — the rule may not produce a value at a type
//     the DECLARATION denies (`a_primitive_rule_never_fires_at_a_type_its_declaration_denies`),
//     because a term and its reduct at different types is subject reduction failing.
//
// The `reduceBool` arm matches a bare `Term::Global("reduceBool")` and asks neither question.
// It is the highest-trust rule in the kernel and the only one with no guard at all.
// =============================================================================

/// The native-reduction hook must pass both gates. `Bool` is what it is declared to return, and
/// a context that says otherwise — by defining the name, or by declaring it at another type —
/// must stop it, exactly as it stops `add` and `Text_append`.
#[test]
fn the_native_reduction_hook_obeys_the_same_two_guards_every_other_primitive_rule_does() {
    // The BASELINE: in the prelude's own context the hook fires and is right.
    let base = std_ctx();
    let decided = app(g("reduceBool"), bin("le", intl(3), intl(2)));
    assert_eq!(
        normalize(&base, &decided),
        g("false"),
        "the hook fires in the trusted context"
    );

    // GATE 1 — a DEFINITION of the name outranks the builtin. A context that says
    // `reduceBool := λb. true` has said what the constant means, and the reducer may not answer
    // something else; every other primitive name already behaves this way.
    let mut defined = std_ctx();
    defined.add_definition(
        "reduceBool".to_string(),
        pi("_", g("Bool"), g("Bool")),
        lam("b", g("Bool"), g("true")),
    );
    assert_eq!(
        normalize(&defined, &decided),
        g("true"),
        "a DEFINITION of `reduceBool` must outrank the native hook — the hook answers `false` \
         here, so the reducer and the context disagree about what the constant means"
    );

    // GATE 2 — the rule may not fire at a type the declaration denies. Re-declared as
    // `Nat → Nat`, the hook still hands back a `Bool` constructor: a term whose inferred type
    // and whose reduct's type are different, which is subject reduction failing.
    // The prelude's `reduceBool` keeps its type (`ofReduceBool` rests on it), so the mistyped
    // declaration is made where nothing depends on the name.
    assert_prelude_name_keeps_its_meaning(
        "reduceBool",
        pi("_", g("Nat"), g("Nat")),
        None,
        &decided,
    );
    let mut mistyped = carriers_only_ctx();
    mistyped.add_declaration("reduceBool", pi("_", g("Nat"), g("Nat")));
    assert!(
        mistyped.refused_declarations().is_empty(),
        "{:?}",
        mistyped.refused_declarations()
    );
    // The hook must not fire; reduction continues AROUND it, so the argument still computes and
    // the head stays exactly where it was. (Asserting the whole term is unchanged would be
    // asserting that `le 3 2` stopped working, which is a different claim.)
    assert_eq!(
        normalize(
            &mistyped,
            &app(g("reduceBool"), bin("le", intl(3), intl(2)))
        ),
        app(g("reduceBool"), g("false")),
        "a rule whose answer is a `Bool` may not fire where the declaration promises a `Nat` — \
         that is a term and its reduct at different types, which is subject reduction failing"
    );
    // …and the honest channel agrees the term FINISHED: it is genuinely stuck, not abandoned.
    assert!(normalize_checked(
        &mistyped,
        &app(g("reduceBool"), bin("le", intl(3), intl(2)))
    )
    .is_ok());
}

/// THE HONEST CHANNEL'S WHOLE CONTRACT, through doors that are all still open: `normalize_checked`
/// must tell a genuine NORMAL FORM from every way of not reaching one — and it must not reach for
/// "did not finish" when the reduction merely took a long time, which is the failure that makes a
/// budget indistinguishable from an answer.
///
/// This carries the 20 000-step δ-chain that
/// `normalize_checked_separates_a_normal_form_from_every_way_of_not_reaching_one` pins, so that
/// case's coverage survives the correction its cycle half needs.
#[test]
fn the_honest_channel_separates_a_normal_form_from_every_way_of_not_reaching_one() {
    use logicaffeine_kernel::Universe;
    let mut ctx = std_ctx();

    // (1) A GENUINE normal form.
    assert_eq!(normalize_checked(&ctx, &g("Zero")), Ok(g("Zero")));
    assert_eq!(normalize_checked(&ctx, &natl(3)), Ok(natl(3)));

    // (2) A LONG BUT TERMINATING δ-chain is not a failure. A budget that stops it is
    // indistinguishable from a normal form, which is the whole defect — so the chain is long
    // enough that a small fuel constant would be caught, and it must still answer `Ok`.
    const CHAIN: usize = 20_000;
    for i in 0..CHAIN {
        let next = if i + 1 == CHAIN {
            g("Zero")
        } else {
            g(&format!("chain{}", i + 1))
        };
        ctx.add_definition(format!("chain{i}"), g("Nat"), next);
    }
    assert!(
        (0..CHAIN).all(|i| ctx.get_definition_body(&format!("chain{i}")).is_some()),
        "a chain registered in ASCENDING order is a forward reference, not a cycle, and the \
         acyclicity gate must not refuse a single rung of it: {:?}",
        ctx.refused_declarations()
    );
    assert_eq!(
        normalize_checked(&ctx, &g("chain0")),
        Ok(g("Zero")),
        "20 000 δ-steps terminate, so the answer is a normal form and must be reported as one"
    );

    // (3) CYCLIC — no normal form exists, and no fuel constant could ever find one. The δ doors
    // refuse to build one (that is Lean's rule), so the cycle comes from the rewrite registry.
    let u = || Universe::Var("u".to_string());
    let cref = |n: &str, l: Universe| Term::Const {
        name: n.to_string(),
        levels: vec![l],
    };
    ctx.add_universe_poly(
        "hcA",
        vec!["u".to_string()],
        Term::Sort(u()),
        cref("hcB", u()),
    );
    ctx.add_universe_poly(
        "hcB",
        vec!["u".to_string()],
        Term::Sort(u()),
        cref("hcA", u()),
    );
    assert!(
        ctx.get_universe_poly("hcB").is_none(),
        "the δ door refuses the closing definition"
    );
    let spin = simp_cycle(&mut ctx, "hcSpin", 2);
    assert!(matches!(
        normalize_checked(&ctx, &app(g(&spin[0]), g("Zero"))),
        Err((_, Unreduced::Cyclic))
    ));

    // (4) TOO DEEP — the term nests past the kernel's door, so the reducer never started, and
    // that is reported rather than returned as "already normal".
    let deep = (0..2_000).fold(g("Zero"), |acc, _| app(g("Succ"), acc));
    assert!(matches!(
        normalize_checked(&ctx, &deep),
        Err((_, Unreduced::TooDeep))
    ));

    // The four verdicts are FOUR: `Ok` on a real normal form, and three distinct refusals. A
    // channel that collapsed any two would be back to a `Term` that cannot say which it is.
    let shallow = (0..100).fold(g("Zero"), |acc, _| app(g("Succ"), acc));
    assert_eq!(
        normalize_checked(&ctx, &shallow),
        Ok(shallow.clone()),
        "one rung inside the door answers"
    );
}

// =============================================================================
// NOT IN THE CORPUS — THE SIMP DOOR CERTIFIED FROM AN ABANDONED SIMPLIFICATION
//
// `simp::check_goal` decides a goal by simplifying BOTH sides and comparing, and
// `reduction::try_dsimp_solve_conclude` ACCEPTS PROOFS through that verdict. Its simplifier
// carried a fuel counter and, on exhaustion, returned the term it had reached — so "they came
// out equal" meant two different things that the door could not tell apart: *both reached the
// same normal form*, and *both were abandoned in the same place*.
//
// That is DEFECT 9 exactly, one layer down: `reduction.rs`'s own
// `the_reflection_door_refuses_a_side_whose_reduction_never_finishes` makes the argument for
// `simp_compute_check`, and `check_goal` — the OTHER half of the same `if` at reduction.rs's
// three `try_*_conclude` sites — did not have it. The budget is reachable: `decompose_goal`
// reads hypotheses off the goal's own implications and never checks that the rewritten variable
// is absent from the right-hand side, so `x = f x` is an accepted hypothesis and `x` then has
// no normal form at all.
//
// `simp::simplify_checked` is now the honest face and `simplify_sterm` its best-effort one —
// ONE implementation with two faces, the same arrangement `normalize`/`normalize_checked` has,
// so the two can never drift apart and answer differently about the same term.
// =============================================================================

/// The simp door must refuse a side whose simplification never finishes — even when the two
/// sides are the SAME term and the equation is true, because the verdict was reached without
/// finishing and that is indistinguishable from two different terms abandoned in one place.
#[test]
fn the_simp_door_refuses_a_side_whose_simplification_never_finishes() {
    use logicaffeine_kernel::simp::check_goal;

    let sname = |s: &str| app(g("SName"), textl(s));
    let svar = |i: i64| app(g("SVar"), intl(i));
    let slit = |n: i64| app(g("SLit"), intl(n));
    let sapp = |f: Term, a: Term| app(app(g("SApp"), f), a);
    let eq = |l: Term, r: Term| sapp(sapp(sname("Eq"), l), r);
    let implies = |h: Term, c: Term| sapp(sapp(sname("implies"), h), c);

    // THE CONTROL — a simplification that FINISHES still decides, in both directions.
    assert!(
        check_goal(&eq(sapp(sapp(sname("add"), slit(2)), slit(3)), slit(5))),
        "2+3 = 5"
    );
    assert!(!check_goal(&eq(slit(2), slit(3))), "…and 2 = 3 is refused");
    assert!(
        check_goal(&eq(svar(0), svar(0))),
        "`x = x` with nothing to rewrite is decided"
    );
    // …including through a hypothesis that DOES settle, which is the door's whole purpose.
    assert!(check_goal(&implies(
        eq(svar(0), slit(0)),
        eq(sapp(sapp(sname("add"), svar(0)), slit(1)), slit(1))
    )));

    // THE HAZARD. `x = f x` is a hypothesis nothing forbids, and it makes `x` a rewrite with no
    // normal form. The conclusion `x = x` is TRUE — and the door must still refuse it, because
    // what it would be agreeing with is two abandoned expansions, and the same code path with a
    // false conclusion is a false positive.
    let never_settles = eq(svar(0), sapp(sname("f"), svar(0)));
    assert!(
        !check_goal(&implies(never_settles.clone(), eq(svar(0), svar(0)))),
        "a side with NO normal form may not be the evidence for a certified goal"
    );
    // …and the plainly-unsound direction, where the two sides are not even the same term.
    assert!(!check_goal(&implies(
        never_settles.clone(),
        eq(svar(0), slit(7))
    )));
    assert!(!check_goal(&implies(
        never_settles,
        eq(sapp(sname("f"), svar(0)), sapp(sname("g"), svar(0)))
    )));

    // THE BOUNDARY, one rung either side: the SAME conclusions without the non-settling
    // hypothesis are decided normally, so the refusal is about the unfinished simplification
    // and not about the shape of the goal.
    assert!(check_goal(&eq(svar(0), svar(0))));
    assert!(!check_goal(&eq(svar(0), slit(7))));
    // A non-settling hypothesis about a DIFFERENT variable does not poison this goal: the
    // refusal is about the side that did not finish, not about the hypothesis list.
    assert!(
        check_goal(&implies(
            eq(svar(1), sapp(sname("f"), svar(1))),
            eq(svar(0), svar(0))
        )),
        "`y = f y` says nothing about `x`, and `x = x` still simplifies to a fixpoint"
    );

    // A term deeper than the budget is refused rather than answered. Both sides are literally
    // the same term, so an implementation that returned how far it got would certify it.
    // Past the 1000-step budget and no further: the point is the BOUNDARY, and a spine deep
    // enough to threaten the reader's own native recursion would be measuring the wrong thing.
    let deep = (0..1_200).fold(slit(1), |acc, _| sapp(sname("f"), acc));
    assert!(
        !check_goal(&eq(deep.clone(), deep)),
        "a simplification that ran out of budget is not a normal form, whatever it reached"
    );
}

// =============================================================================
// NOT IN THE CORPUS — A CERTIFYING DOOR NORMALISED ITS GOAL WITH THE BEST-EFFORT FACE
//
// Every `try_d*_solve_conclude` began `let norm_goal = normalize(ctx, goal);`. `normalize`
// hands back HOW FAR IT GOT, and these are the doors that turn a verdict into a proof: a goal
// whose own normalisation was abandoned reached the checker as a PARTIAL goal, was decided
// about, and was then RETURNED as the concluded statement — which the REPL's soundness gate
// compares against the stated goal. So the door certified something other than what it was
// asked about, and nothing anywhere said so.
//
// The file already made this exact argument twice — `simp_compute_check` insists both SIDES of
// the equality finished, and `try_concludes_reduce`'s script path normalises its conclusion
// through `normalize_checked` "so the gate cannot match two partial terms" — while taking the
// goal itself from the unchecked face. `reduction::goal_to_certify` is now the one door, so the
// only line there is for a new tactic to copy is the honest one.
// =============================================================================

/// A goal that cannot be normalised must not be CONCLUDED, even when the checker would have
/// said yes about the part of it that did settle.
#[test]
fn a_certifying_door_refuses_a_goal_whose_own_normalisation_never_finishes() {
    use logicaffeine_kernel::Universe;
    let mut ctx = std_ctx();
    let sname = |s: &str| app(g("SName"), textl(s));
    let slit = |n: i64| app(g("SLit"), intl(n));
    let sapp = |f: Term, a: Term| app(app(g("SApp"), f), a);
    // `Eq T 1 1` in the reflected language — the TYPE slot is where the hazard hides, because
    // the simplifier reads the two sides and never looks at it, while `normalize` walks it.
    let eq_at = |ty: Term| sapp(sapp(sapp(sname("Eq"), ty), slit(1)), slit(1));
    let conclude =
        |c: &Context, goal: Term| normalize(c, &app(g("concludes"), app(g("DSimpSolve"), goal)));
    let error = app(g("SName"), textl("Error"));

    // THE CONTROL — a goal that normalises finishes, is checked, and is concluded.
    let ordinary = eq_at(sname("Int"));
    assert_eq!(
        conclude(&ctx, ordinary.clone()),
        ordinary,
        "`1 = 1` at an ordinary type still concludes — the refusal below is about the \
         unfinished reduction, not about the shape of the goal"
    );

    // THE HAZARD. A reduction with no normal form in the type slot: `normalize` never finishes
    // on this goal, while the simplifier — which reads only the two SIDES — would happily say
    // `1 = 1`. The δ doors refuse a cycle, so it comes from the rewrite registry.
    let u = || Universe::Var("u".to_string());
    ctx.add_universe_poly(
        "gcA",
        vec!["u".to_string()],
        Term::Sort(u()),
        Term::Const {
            name: "gcB".to_string(),
            levels: vec![u()],
        },
    );
    ctx.add_universe_poly(
        "gcB",
        vec!["u".to_string()],
        Term::Sort(u()),
        Term::Const {
            name: "gcA".to_string(),
            levels: vec![u()],
        },
    );
    assert!(
        ctx.get_universe_poly("gcB").is_none(),
        "the δ door refuses the closing definition"
    );
    let spin = simp_cycle(&mut ctx, "gcSpin", 2);
    let cycling = eq_at(app(g(&spin[0]), g("Zero")));
    assert!(
        matches!(
            normalize_checked(&ctx, &cycling),
            Err((_, Unreduced::Cyclic))
        ),
        "the premise of this test: the goal really does have no normal form"
    );
    assert_eq!(
        conclude(&ctx, cycling),
        error,
        "a door that turns a verdict into a proof may not conclude a goal it never finished \
         normalising — what it would hand back is a PARTIAL goal, and the soundness gate \
         compares that against the stated one"
    );

    // …and the control still concludes in the SAME context, so the refusal is a property of
    // the goal and not of the context having a cycle in it anywhere at all.
    assert_eq!(conclude(&ctx, ordinary.clone()), ordinary);
}

// =============================================================================
// ★ NOT IN THE CORPUS — THE UNIVERSE-POLYMORPHIC DOOR IS AN UNCHECKED MINT ★
//
// `Context::declare` is THE CHECKED REGISTRATION GATEWAY, and its own doc says why it had to
// exist: "`add_definition` used to be an `insert`. That is a hole the size of the whole kernel:
// the type a name is registered AT is the type `infer_type` hands back for it, so an unchecked
// insert lets any caller mint a constant of any type, `False` included."
//
// `Context::add_universe_poly` is still that insert. It writes `(params, ty, body)` straight
// into `universe_polys` — no type check, no δ-acyclicity check — and every reader downstream
// trusts it:
//
//   * `type_checker::lookup_const` reads `Context::poly_declaration`, which reads
//     `universe_polys`, so `infer_type` hands back the DECLARED type;
//   * `term::level_telescope(Some(&[]), ty)` returns `[]` for an empty telescope, so
//     `resolve_const_levels` accepts `Const { name, levels: [] }` at arity 0;
//   * `reduction::const_step` δ-unfolds the body.
//
// So `add_universe_poly("evilPoly", vec![], False, Zero)` puts a CLOSED, WELL-TYPED term of
// type `False` into a context built from `StandardLibrary::register`. Both kernels read the
// same map, so both agree.
//
// This is a `Context` defect and the repair belongs there — route `add_universe_poly` through
// `declare`'s check, exactly as `add_definition` is. It is asserted here because this is the
// lane that found it and because `reduction::const_step` is the δ half of the path.
// =============================================================================

/// A door that registers a BODY must check it against the TYPE it registers it at — whatever
/// the door. Stated as the invariant, so it goes green the moment the polymorphic door joins
/// the checked gateway and stays green afterwards.
#[test]
fn no_door_that_registers_a_body_can_mint_a_constant_at_a_type_the_body_does_not_have() {
    use logicaffeine_kernel::double_check;
    let mut ctx = std_ctx();

    // THE CONTROL — the checked gateway refuses exactly this, which is the whole reason it
    // exists. `Zero : Nat` is not a proof of `False`.
    ctx.add_definition("evilMono".to_string(), g("False"), g("Zero"));
    assert!(
        ctx.get_definition_body("evilMono").is_none(),
        "`add_definition` must refuse `evil : False := Zero` — and it does"
    );
    assert!(
        ctx.refused_declarations()
            .iter()
            .any(|(n, _)| n == "evilMono"),
        "…and say so: {:?}",
        ctx.refused_declarations()
    );

    // THE SAME BODY AT THE SAME TYPE, through the polymorphic door.
    ctx.add_universe_poly("evilPoly", vec![], g("False"), g("Zero"));
    let minted = Term::Const {
        name: "evilPoly".to_string(),
        levels: vec![],
    };

    // Either the door refused it, or the kernel must refuse to type it at `False`. One of the
    // two has to hold, or the context contains a closed term of type `False`.
    let refused_at_the_door = ctx.get_universe_poly("evilPoly").is_none();
    let typed_at_false = infer_type(&ctx, &minted).ok() == Some(g("False"));
    assert!(
        refused_at_the_door || !typed_at_false,
        "`add_universe_poly` registered `evilPoly : False := Zero` with NO check, and \
         `infer_type` answers `False` for it — a closed, well-typed proof of `False` in a \
         context built from `StandardLibrary::register`. The body normalises to {}, the \
         declared type is `False`, and THE SECOND KERNEL IS NO HELP because it reads the same \
         map: double_check says {:?}. The repair belongs at the door — route \
         `add_universe_poly` through `Context::declare`'s check, as `add_definition` is.",
        normalize(&ctx, &minted),
        double_check(&ctx, &minted)
    );

    // …and once the door is checked, the ordinary polymorphic definition it exists for must
    // still go through: the repair is a CHECK, not a closure.
    // `Nat : Sort u` fails at `u = 0`, exactly as Lean refuses `def p.{u} : Sort u := Nat`.
    ctx.add_universe_poly(
        "notPoly",
        vec!["u".to_string()],
        Term::Sort(logicaffeine_kernel::Universe::Var("u".to_string())),
        g("Nat"),
    );
    assert!(
        ctx.get_universe_poly("notPoly").is_none(),
        "`Nat : Sort u` does not hold at every level"
    );
    assert!(ctx
        .refused_declarations()
        .iter()
        .any(|(n, _)| n == "notPoly"));
    let uvar = || logicaffeine_kernel::Universe::Var("u".to_string());
    ctx.add_universe_poly(
        "goodPoly",
        vec!["u".to_string()],
        Term::Sort(logicaffeine_kernel::Universe::Succ(Box::new(uvar()))),
        Term::Sort(uvar()),
    );
    let good = Term::Const {
        name: "goodPoly".to_string(),
        levels: vec![logicaffeine_kernel::Universe::Type(0)],
    };
    assert_eq!(
        normalize(&ctx, &good),
        Term::Sort(logicaffeine_kernel::Universe::Type(0)),
        "a WELL-TYPED polymorphic body still unfolds, at the level it is asked at"
    );
}
