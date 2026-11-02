//! Locks for the independent kernel's lazy δ: it proves by unfolding the later definition first,
//! it never refuses what the reduction-based conversion accepts, and every answer it gives agrees
//! with that conversion wherever the conversion decides.
use super::*;
use crate::prelude::StandardLibrary;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apps(f: Term, xs: impl IntoIterator<Item = Term>) -> Term {
    xs.into_iter().fold(f, app)
}
fn lam(n: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn pi(n: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}
fn eq(a: Term, b: Term) -> Term {
    apps(g("Eq"), [g("Nat"), a, b])
}
fn refl(a: Term) -> Term {
    apps(g("refl"), [g("Nat"), a])
}
fn claim(proof: Term, statement: Term) -> Term {
    Term::Let {
        name: "claim".into(),
        ty: Box::new(statement),
        value: Box::new(proof),
        body: Box::new(v("claim")),
    }
}

/// `countdown n = Zero` one structural step per unit — at `2²¹` its value lies beyond this
/// kernel's reduction budget — and `alias`, a later definition that merely names it.
fn context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    let body = lam(
        "n",
        g("Nat"),
        Term::Match {
            discriminant: Box::new(v("n")),
            motive: Box::new(lam("_", g("Nat"), g("Nat"))),
            cases: vec![g("Zero"), lam("k", g("Nat"), app(v("countdown"), v("k")))],
        },
    );
    let nat_to_nat = pi("_", g("Nat"), g("Nat"));
    c.add_definition(
        "countdown".into(),
        nat_to_nat.clone(),
        Term::Fix {
            name: "countdown".into(),
            body: Box::new(body),
        },
    );
    c.add_definition(
        "alias".into(),
        nat_to_nat.clone(),
        lam("n", g("Nat"), app(g("countdown"), v("n"))),
    );
    c.add_definition(
        "alias_of_alias".into(),
        nat_to_nat,
        lam("n", g("Nat"), app(g("alias"), v("n"))),
    );
    c.add_declaration("P", Term::Sort(Universe::Prop));
    c.add_declaration("p1", g("P"));
    c.add_declaration("p2", g("P"));
    c
}
fn beyond_budget() -> Term {
    nat(1 << 21)
}

/// The comparisons that reached the reduction-based conversion during `f`.
fn reduction_conversions<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = WHNF_CONVERSIONS.with(|n| n.get());
    let result = f();
    (result, WHNF_CONVERSIONS.with(|n| n.get()) - before)
}

#[test]
fn a_later_definition_is_unfolded_first_and_the_common_call_is_never_evaluated() {
    // `alias L ≡ countdown L` with `L = 2²¹`: unfolding `alias` (the greater height) exposes
    // `countdown L` on both sides, so the loop decides the pair without evaluating the call. The
    // reduction-based conversion evaluates `countdown L` on both sides and is cut short by its
    // budget: it decides nothing.
    let c = context();
    let db = |t: &Term| to_db(t, &mut Vec::new()).unwrap();
    for (left, right) in [
        ("alias", "countdown"),
        ("countdown", "alias"),
        ("alias_of_alias", "countdown"),
    ] {
        let (a, b) = (
            db(&app(g(left), beyond_budget())),
            db(&app(g(right), beyond_budget())),
        );
        let (answer, fallbacks) = reduction_conversions(|| conversion(&c, &[], &a, &b));
        assert_eq!(answer, Lazy::Decided(true), "{left} ≡ {right}");
        assert_eq!(
            fallbacks, 0,
            "{left} ≡ {right}: decided lazily, with no reduction-based comparison"
        );
        let exhausted_before = WHNF_EXHAUSTED.with(|e| e.replace(false));
        let old = def_eq_by_whnf(&c, &[], &a, &b);
        let cut_short = WHNF_EXHAUSTED.with(|e| e.replace(exhausted_before));
        assert!(cut_short || !old, "the reduction-based conversion alone cannot decide {left} ≡ {right}: it evaluates the call");
    }
    assert!(height(&c, "alias").unwrap() > height(&c, "countdown").unwrap());
    assert!(height(&c, "alias_of_alias").unwrap() > height(&c, "alias").unwrap());
    // And a whole check that depends on it is accepted by the independent kernel.
    let goal = claim(
        refl(app(g("countdown"), beyond_budget())),
        eq(
            app(g("alias"), beyond_budget()),
            app(g("countdown"), beyond_budget()),
        ),
    );
    crate::recheck(&c, &goal).expect("alias L = countdown L by one δ step");
}

#[test]
fn a_false_claim_is_still_refused() {
    let c = context();
    // `alias 3` computes to `Zero`; claimed at `Succ Zero`.
    let goal = claim(
        refl(app(g("Succ"), g("Zero"))),
        eq(app(g("alias"), nat(3)), app(g("Succ"), g("Zero"))),
    );
    assert!(crate::recheck(&c, &goal).is_err());
    assert!(crate::infer_type(&c, &goal).is_err());
    // A common constructor with different arguments is refused by the rigid rule.
    let goal = claim(
        refl(app(g("Succ"), app(g("alias"), nat(5)))),
        eq(
            app(g("Succ"), app(g("alias"), nat(5))),
            app(g("Succ"), app(g("Succ"), g("Zero"))),
        ),
    );
    assert!(crate::recheck(&c, &goal).is_err());
}

#[test]
fn what_the_lazy_loop_leaves_open_the_reduction_based_conversion_still_proves() {
    let c = context();
    // Distinct heads that are equal only through the literal bridge: `Succ Zero ≡ 1`.
    let goal = claim(refl(nat(1)), eq(app(g("Succ"), g("Zero")), nat(1)));
    crate::recheck(&c, &goal).expect("Succ Zero is the literal 1");
    // Proofs of one proposition. Distinct constructors are left to the full conversion, which proves
    // them equal by proof irrelevance; one constructor at two proofs is compared by irrelevance first.
    let db = |t: &Term| to_db(t, &mut Vec::new()).unwrap();
    let left = db(&apps(g("left"), [g("P"), g("P"), g("p1")]));
    let right = db(&apps(g("right"), [g("P"), g("P"), g("p2")]));
    let other = db(&apps(g("left"), [g("P"), g("P"), g("p2")]));
    assert_eq!(
        conversion(&c, &[], &left, &right),
        Lazy::Open,
        "distinct constructors are not the lazy loop's to refute"
    );
    assert!(def_eq(&c, &[], &left, &right), "proof irrelevance");
    assert!(
        matches!(conversion(&c, &[], &left, &other), Lazy::Decided(true)),
        "irrelevance before argument comparison"
    );
}

#[test]
fn every_lazy_answer_agrees_with_the_reduction_based_conversion() {
    let c = context();
    DIFFERENTIAL.with(|d| d.set(true));
    let before = DIFFERENTIAL_COMPARISONS.with(|n| n.get());
    let goals = [
        claim(
            refl(app(g("countdown"), nat(7))),
            eq(app(g("alias"), nat(7)), app(g("countdown"), nat(7))),
        ),
        claim(
            refl(g("Zero")),
            eq(app(g("alias_of_alias"), nat(9)), g("Zero")),
        ),
        claim(
            refl(nat(1)),
            eq(app(g("Succ"), app(g("alias"), nat(2))), nat(1)),
        ),
        claim(
            refl(app(g("Succ"), g("Zero"))),
            eq(
                app(g("Succ"), app(g("alias"), nat(4))),
                app(g("Succ"), g("Zero")),
            ),
        ),
    ];
    for goal in &goals {
        crate::recheck(&c, goal).unwrap_or_else(|e| panic!("{goal}: {e:?}"));
    }
    let refused = claim(
        refl(app(g("Succ"), g("Zero"))),
        eq(
            app(g("Succ"), app(g("alias"), nat(4))),
            app(g("Succ"), app(g("Succ"), g("Zero"))),
        ),
    );
    assert!(crate::recheck(&c, &refused).is_err());
    DIFFERENTIAL.with(|d| d.set(false));
    assert!(
        DIFFERENTIAL_COMPARISONS.with(|n| n.get()) > before,
        "the differential compared the lazy answers"
    );
}

#[test]
fn the_certified_nat_order_decides_inside_the_lazy_loop_and_in_the_whole_check() {
    // KB's reproduction of the regression the first version of this loop introduced: the order on
    // two closed naturals is decided by this kernel's certified acceleration, which fires on the
    // spine that still NAMES the operation. Unfolding `natLe` first erases the name, and the
    // unary recursion then runs out of budget. Both a bare conversion and a whole check must
    // decide it.
    let c = context();
    let bool_eq = |a: Term, b: Term| apps(g("Eq"), [g("Bool"), a, b]);
    let bool_refl = |a: Term| apps(g("refl"), [g("Bool"), a]);
    let order = |op: &str, x: Term, y: Term| apps(g(op), [x, y]);
    let big = || {
        Term::Lit(Literal::Nat(
            crate::BigInt::parse_decimal("18446744073709551616").unwrap(),
        ))
    };
    for (claim_of, value) in [
        (order("natLe", nat(262_144), nat(262_144)), "true"),
        (order("natLt", big(), big()), "false"),
        (order("natLt", nat(262_143), nat(262_144)), "true"),
        (order("natLe", nat(262_145), nat(262_144)), "false"),
    ] {
        let (x, y) = (
            to_db(&claim_of, &mut Vec::new()).unwrap(),
            to_db(&g(value), &mut Vec::new()).unwrap(),
        );
        assert_eq!(
            conversion(&c, &[], &x, &y),
            Lazy::Decided(true),
            "{claim_of} ≡ {value}"
        );
        let goal = app(
            lam("h", bool_eq(claim_of.clone(), g(value)), g("Zero")),
            bool_refl(g(value)),
        );
        crate::recheck(&c, &goal).unwrap_or_else(|e| panic!("{claim_of} = {value}: {e:?}"));
        let wrong = if value == "true" { "false" } else { "true" };
        let refused = app(
            lam("h", bool_eq(claim_of.clone(), g(wrong)), g("Zero")),
            bool_refl(g(value)),
        );
        assert!(
            matches!(crate::recheck(&c, &refused), Err(ReCheckError::Ill(_))),
            "{claim_of} = {wrong} is refused as ill-typed"
        );
    }
}

#[test]
fn an_undecided_pair_is_decided_by_the_conversion_that_existed_before_the_loop() {
    // Distinct heads the loop leaves open: the caller's conversion decides the ORIGINAL pair, on
    // the fuel it would have had, so the loop cannot change its verdict.
    let c = context();
    let (a, b) = (
        to_db(&app(g("alias"), nat(3)), &mut Vec::new()).unwrap(),
        to_db(&app(g("Succ"), g("Zero")), &mut Vec::new()).unwrap(),
    );
    assert!(!def_eq(&c, &[], &a, &b));
    let fuel = WHNF_FUEL.with(|f| f.get());
    assert_eq!(
        conversion(&c, &[], &a, &b),
        Lazy::Open,
        "Zero against Succ Zero: distinct heads are the caller's"
    );
    assert_eq!(
        WHNF_FUEL.with(|f| f.get()),
        fuel,
        "the loop never spends the caller's fuel"
    );
    assert!(
        !WHNF_EXHAUSTED.with(|e| e.get()),
        "…nor records an exhaustion in the caller's check"
    );
}

#[test]
fn nested_comparisons_share_one_allowance() {
    // `Succ¹² (alias 3) ≡ Succ¹² (countdown 3)`: the common constructor's arguments are compared
    // INSIDE the loop, twelve levels deep, and every level shares the outermost scope — one δ
    // allowance and one fuel budget — which closes when the outermost comparison returns and
    // hands the caller back its fuel untouched.
    let c = context();
    let chain = |leaf: Term| (0..12).fold(leaf, |t, _| app(g("Succ"), t));
    let db = |t: &Term| to_db(t, &mut Vec::new()).unwrap();
    let (a, b) = (
        db(&chain(app(g("alias"), nat(3)))),
        db(&chain(app(g("countdown"), nat(3)))),
    );
    let fuel = WHNF_FUEL.with(|f| f.get());
    let opened = SCOPES_OPENED.with(|n| n.get());
    assert_eq!(conversion(&c, &[], &a, &b), Lazy::Decided(true));
    assert_eq!(
        SCOPES_OPENED.with(|n| n.get()) - opened,
        1,
        "twelve nested comparisons, one scope"
    );
    assert!(
        ALLOWANCE.with(|a| a.get()).is_none(),
        "the allowance closes with the outermost comparison"
    );
    assert_eq!(
        WHNF_FUEL.with(|f| f.get()),
        fuel,
        "the caller's fuel is untouched"
    );
    // A refutation deep in the chain is a decision of the rigid rule, reached the same way.
    let (a, b) = (
        db(&chain(app(g("alias"), nat(3)))),
        db(&chain(app(g("Succ"), g("Zero")))),
    );
    assert_eq!(conversion(&c, &[], &a, &b), Lazy::Decided(false));
}

#[test]
fn a_theorem_is_never_unfolded_where_proof_irrelevance_decides() {
    // Two proofs of one proposition, one of them a theorem whose proof is expensive to expose:
    // irrelevance decides before the theorem is unfolded.
    let mut c = context();
    let statement = eq(app(g("alias"), nat(4)), g("Zero"));
    c.declare_theorem_checked("four_to_zero", vec![], statement.clone(), refl(g("Zero")))
        .unwrap();
    let (a, b) = (
        to_db(&g("four_to_zero"), &mut Vec::new()).unwrap(),
        to_db(&refl(g("Zero")), &mut Vec::new()).unwrap(),
    );
    assert!(matches!(conversion(&c, &[], &a, &b), Lazy::Decided(true)));
    // Folding never makes different data equal: a theorem is still unfolded where its value decides.
    let (x, y) = (
        to_db(&app(g("alias"), nat(4)), &mut Vec::new()).unwrap(),
        to_db(&app(g("Succ"), g("Zero")), &mut Vec::new()).unwrap(),
    );
    assert!(!def_eq(&c, &[], &x, &y));
}

#[test]
fn a_deep_comparison_never_overflows_and_never_invents_an_equality() {
    // 1200 nested constructor levels (inside the kernel's depth door), compared by the loop's rigid
    // rule one level per nested comparison. Checked through the independent kernel's entry point,
    // which sizes its stack from the term's depth as every kernel entry does: the process keeps
    // its stack, a chain equals itself, and two different chains are never called equal.
    let c = context();
    let chain = |leaf: Term| (0..1200).fold(leaf, |t, _| app(g("Succ"), t));
    let same = claim(
        refl(chain(g("Zero"))),
        eq(chain(g("Zero")), chain(g("Zero"))),
    );
    crate::recheck(&c, &same).expect("a chain equals itself");
    let different = claim(
        refl(chain(g("Zero"))),
        eq(chain(g("Zero")), chain(app(g("Succ"), g("Zero")))),
    );
    assert!(
        crate::recheck(&c, &different).is_err(),
        "Succ¹²⁰⁰ Zero is not Succ¹²⁰¹ Zero"
    );
}
