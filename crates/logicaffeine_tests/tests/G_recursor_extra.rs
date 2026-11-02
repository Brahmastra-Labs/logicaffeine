//! Lane **G_recursor** — the fences around the termination guard's LEXICOGRAPHIC widening,
//! the paradox fence around nested inductives, and the derived-eliminator boundary.
//!
//! The guard in `termination.rs` is the thing standing between LOGOS and a proof of `False`,
//! so every capability it gains here is paired with the neighbouring shape it must still
//! REFUSE. A positive-only test is satisfiable by a guard that accepts everything; a
//! positive/negative pair is not.
//!
//! The measure the guard now decides is the LEXICOGRAPHIC product of the structural-subterm
//! order `◁` over an ORDERED list of columns — column 0 is the Giménez structural argument
//! (the binder the body's `match` discriminates on, or the first inductive-typed binder), and
//! the remaining inductive-typed binders follow in positional order. A call is admitted iff
//! some column's argument is a strict subterm of that column's parameter and every EARLIER
//! column's argument is a strict subterm or definitionally that column's parameter. Lex
//! products of well-founded orders are well-founded, so the descent is real.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, recheck, BigInt, Context, Literal, Term, Universe};

// ==== helpers (house style: every kernel test declares its own) ====

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
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn fix(name: &str, body: Term) -> Term {
    Term::Fix {
        name: name.to_string(),
        body: Box::new(body),
    }
}
fn mfix(defs: &[(&str, Term)], index: usize) -> Term {
    Term::MutualFix {
        defs: defs
            .iter()
            .map(|(n, b)| (n.to_string(), b.clone()))
            .collect(),
        index,
    }
}
fn match_(d: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(motive),
        cases,
    }
}
fn nat() -> Term {
    g("Nat")
}
fn succ(t: Term) -> Term {
    app(g("Succ"), t)
}
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn nat_motive() -> Term {
    lam("_", nat(), nat())
}
/// `Eq Nat a b`
fn eq_nat(a: Term, b: Term) -> Term {
    apps(g("Eq"), &[nat(), a, b])
}
/// `pred := λn. match n with | Zero => Zero | Succ k => k` — the δ-transparent definition whose
/// unfolding the guard's subterm analysis has to walk.
fn with_pred(ctx: &mut Context) {
    ctx.add_definition(
        "pred".to_string(),
        arrow(nat(), nat()),
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                nat_motive(),
                vec![g("Zero"), lam("k", nat(), var("k"))],
            ),
        ),
    );
}

/// A two-argument fixpoint `f` over `Nat` whose `m`-match's `Succ` branch nests an `n`-match,
/// with the four branch bodies supplied by the caller. Every lexicographic shape below is one
/// instantiation of this skeleton, so the tests differ ONLY in the recursive call.
fn two_column_fix(m_zero: Term, n_zero: Term, n_succ: Term) -> Term {
    fix(
        "f",
        lam(
            "m",
            nat(),
            lam(
                "n",
                nat(),
                match_(
                    var("m"),
                    nat_motive(),
                    vec![
                        m_zero,
                        lam(
                            "m1",
                            nat(),
                            match_(
                                var("n"),
                                nat_motive(),
                                vec![n_zero, lam("n2", nat(), n_succ)],
                            ),
                        ),
                    ],
                ),
            ),
        ),
    )
}

// ==== THE NEGATIVES — every shape the widening must still refuse ====

/// NEGATIVE 1. Nothing moves: `f m n` holds BOTH columns equal. A lexicographic order needs a
/// STRICT decrease somewhere; "≤ in every column" is exactly the non-terminating loop.
#[test]
fn a_call_that_holds_every_column_equal_is_refused() {
    let ctx = std_ctx();
    let loop_ = two_column_fix(g("Zero"), g("Zero"), apps(var("f"), &[var("m"), var("n")]));
    let r = infer_type(&ctx, &loop_);
    assert!(
        r.is_err(),
        "`f m n` decreases in no column — admitting it is a non-terminating fixpoint at any \
         type, and `Fix` at `False` then inhabits `False`: {r:?}"
    );
    assert!(
        recheck(&ctx, &loop_).is_err(),
        "the second kernel must refuse it too"
    );
}

/// NEGATIVE 2. The REBUILT scrutinee is EQUAL, never SMALLER. Inside `match m with Succ m1`,
/// `Succ m1` is definitionally `m` — so a call passing it at column 0 has held that column
/// equal, and with the second column also equal there is no descent.
#[test]
fn rebuilding_the_scrutinee_holds_a_column_equal_it_does_not_decrease_it() {
    let ctx = std_ctx();
    let loop_ = two_column_fix(
        g("Zero"),
        g("Zero"),
        apps(var("f"), &[succ(var("m1")), var("n")]),
    );
    let r = infer_type(&ctx, &loop_);
    assert!(
        r.is_err(),
        "`f (Succ m1) n` under `match m with Succ m1` is `f m n` spelled differently — the \
         rebuilt constructor application must classify as EQUAL, not smaller: {r:?}"
    );
    assert!(
        recheck(&ctx, &loop_).is_err(),
        "the second kernel must refuse it too"
    );
}

/// NEGATIVE 3. BOTH columns rebuilt. `f (Succ m1) (Succ n2)` under `match m with Succ m1` and
/// `match n with Succ n2` is the identity call written in constructor form.
#[test]
fn rebuilding_every_column_is_the_identity_call_and_is_refused() {
    let ctx = std_ctx();
    let loop_ = two_column_fix(
        g("Zero"),
        g("Zero"),
        apps(var("f"), &[succ(var("m1")), succ(var("n2"))]),
    );
    let r = infer_type(&ctx, &loop_);
    assert!(
        r.is_err(),
        "`f (Succ m1) (Succ n2)` holds both columns equal: {r:?}"
    );
    assert!(
        recheck(&ctx, &loop_).is_err(),
        "the second kernel must refuse it too"
    );
}

/// NEGATIVE 4 — THE CROSS-COLUMN TRAP, and the sharpest fence on the whole widening. `n2` is a
/// strict subterm of `n`, but it is NOT a subterm of `m`; a measure that let column 0 be
/// discharged by column 1's subterm is not a lexicographic order at all, and
/// `f n2 m1` can grow: from `(1, 100)` it steps to `(99, 0)`.
#[test]
fn a_subterm_of_one_column_may_not_discharge_a_different_column() {
    let ctx = std_ctx();
    let swapped = two_column_fix(
        g("Zero"),
        g("Zero"),
        apps(var("f"), &[var("n2"), var("m1")]),
    );
    let r = infer_type(&ctx, &swapped);
    assert!(
        r.is_err(),
        "`f n2 m1` swaps the columns: `n2 ◁ n` says nothing about `m`, so the lexicographic \
         comparison must stop at column 0 with UNKNOWN rather than reading column 1's subterm \
         set: {r:?}"
    );
    assert!(
        recheck(&ctx, &swapped).is_err(),
        "the second kernel must refuse it too"
    );
}

/// NEGATIVE 5. A column-0 argument that is strictly LARGER than the scrutinee's rebuild —
/// `Succ (Succ m1)` where the branch knows only `m ≡ Succ m1` — must not be mistaken for the
/// rebuild. Column 1 decreasing does not rescue it: lex compares column 0 FIRST.
#[test]
fn a_column_argument_larger_than_the_rebuild_is_not_equal() {
    let ctx = std_ctx();
    let grower = two_column_fix(
        g("Zero"),
        g("Zero"),
        apps(var("f"), &[succ(succ(var("m1"))), var("n2")]),
    );
    let r = infer_type(&ctx, &grower);
    assert!(
        r.is_err(),
        "`f (Succ (Succ m1)) n2` grows column 0; the rebuild is `Succ m1` and nothing else \
         may match it: {r:?}"
    );
    assert!(
        recheck(&ctx, &grower).is_err(),
        "the second kernel must refuse it too"
    );
}

/// NEGATIVE 6. A call that never supplies the column it would have to decrease. Partial
/// application cannot be counted as a descent — `(f (Succ m1))` is a FUNCTION, and what it is
/// eventually applied to is not in the guard's hands.
#[test]
fn a_partially_applied_recursive_call_cannot_decrease_a_column_it_omits() {
    let ctx = std_ctx();
    // n-Succ branch: `(f (Succ m1)) n2` written as a one-argument call whose result is then
    // applied — the recursive head is consumed with ONE argument only in the inner spine.
    let partial = two_column_fix(
        g("Zero"),
        g("Zero"),
        app(app(var("f"), succ(var("m1"))), var("m1")),
    );
    let r = infer_type(&ctx, &partial);
    assert!(
        r.is_err(),
        "column 0 is held equal and column 1 gets `m1`, a subterm of the WRONG column: {r:?}"
    );
    assert!(
        recheck(&ctx, &partial).is_err(),
        "the second kernel must refuse it too"
    );
}

/// NEGATIVE 7. A non-inductive binder is not a column. `g : Nat → Nat` can never be destructed,
/// so a fixpoint holding column 0 equal and "decreasing" `g` has no measure at all.
#[test]
fn a_function_typed_binder_is_not_a_lexicographic_column() {
    let ctx = std_ctx();
    let bogus = fix(
        "f",
        lam(
            "m",
            nat(),
            lam(
                "h",
                arrow(nat(), nat()),
                match_(
                    var("m"),
                    nat_motive(),
                    vec![
                        g("Zero"),
                        lam("m1", nat(), apps(var("f"), &[succ(var("m1")), var("h")])),
                    ],
                ),
            ),
        ),
    );
    let r = infer_type(&ctx, &bogus);
    assert!(
        r.is_err(),
        "`f (Succ m1) h` holds the only column equal; a `Nat → Nat` binder contributes no \
         well-founded order: {r:?}"
    );
    assert!(
        recheck(&ctx, &bogus).is_err(),
        "the second kernel must refuse it too"
    );
}

/// NEGATIVE 8. A constructor field that SHADOWS the column's own binder kills the column: inside
/// `match m with Succ m => …` the name `m` denotes the field, so `m ≡ Succ m` is false and the
/// rebuild must not be recorded under the shadowed name.
#[test]
fn a_field_that_shadows_its_columns_binder_kills_the_column() {
    let ctx = std_ctx();
    let shadowed = fix(
        "f",
        lam(
            "m",
            nat(),
            match_(
                var("m"),
                nat_motive(),
                vec![g("Zero"), lam("m", nat(), app(var("f"), succ(var("m"))))],
            ),
        ),
    );
    let r = infer_type(&ctx, &shadowed);
    assert!(
        r.is_err(),
        "the `Succ` field is named `m`, so `Succ m` rebuilds nothing and the recursion has no \
         decreasing column: {r:?}"
    );
    assert!(
        recheck(&ctx, &shadowed).is_err(),
        "the second kernel must refuse it too"
    );
}

/// NEGATIVE 9 — the δ-unfolding fence WITHOUT a refinement. `pred n` unfolds to
/// `match n with Zero => Zero | Succ k => k`, whose `Zero` branch returns `Zero` — and `Zero`
/// is NOT a strict subterm of `n` when `n` IS `Zero`. With no hypothesis ruling that branch
/// out, the meet over branches is "not a subterm" and the recursion must be refused. This is
/// the shape Coq rejects too, and it is the exact thing `decreasing_by` exists to discharge.
#[test]
fn unfolding_a_definition_takes_the_meet_over_every_reachable_branch() {
    let mut ctx = std_ctx();
    with_pred(&mut ctx);
    let bogus = fix(
        "f",
        lam("n", nat(), succ(app(var("f"), app(g("pred"), var("n"))))),
    );
    let r = infer_type(&ctx, &bogus);
    assert!(
        r.is_err(),
        "`f (pred n)` with NO branch hypothesis loops at `n = Zero` (`pred Zero ≡ Zero`), so \
         the subterm analysis must take the meet over both branches of `pred` and refuse: {r:?}"
    );
    assert!(
        recheck(&ctx, &bogus).is_err(),
        "the second kernel must refuse it too"
    );
}

/// NEGATIVE 10 — the refinement's OWN fence. A `Decidable` discriminant that is a bound
/// VARIABLE, not a kernel-checked decision procedure, may be instantiated with `isFalse h` at
/// `n = Zero` by anyone holding a proof of `False`; the branch is then reachable at `Zero` and
/// the recursion diverges. The guard must read a refinement only off a δ-transparent
/// DEFINITION applied to the recursion's own data.
#[test]
fn a_refinement_is_read_only_off_a_checked_decision_procedure() {
    let ctx = std_ctx();
    let dec_eq0 = |n: Term| app(g("Decidable"), eq_nat(n, g("Zero")));
    let smuggled = fix(
        "f",
        lam(
            "n",
            nat(),
            lam(
                "d",
                dec_eq0(var("n")),
                match_(
                    var("d"),
                    nat(),
                    vec![
                        lam("hp", eq_nat(var("n"), g("Zero")), g("Zero")),
                        lam(
                            "hn",
                            app(g("Not"), eq_nat(var("n"), g("Zero"))),
                            succ(apps(var("f"), &[app(g("pred"), var("n")), var("d")])),
                        ),
                    ],
                ),
            ),
        ),
    );
    let r = infer_type(&ctx, &smuggled);
    assert!(
        r.is_err(),
        "the discriminant is the λ-bound `d`, whose value the caller chooses — a refinement \
         taken from it is only as sound as the caller, so the guard must refuse: {r:?}"
    );
}

/// NEGATIVE 11. A refinement pins a constructor head only when the inductive has exactly TWO
/// constructors and the excluded one is NULLARY. `¬(n = Succ k)` excludes one `Succ`, not the
/// whole `Succ` family, so nothing is pinned and `pred n` stays un-analysable.
#[test]
fn a_negative_refinement_against_a_non_nullary_constructor_pins_nothing() {
    let mut ctx = std_ctx();
    with_pred(&mut ctx);
    let bogus = fix(
        "f",
        lam(
            "k",
            nat(),
            lam(
                "n",
                nat(),
                match_(
                    apps(g("decEqNat"), &[var("n"), succ(var("k"))]),
                    nat(),
                    vec![
                        lam("hp", eq_nat(var("n"), succ(var("k"))), g("Zero")),
                        lam(
                            "hn",
                            app(g("Not"), eq_nat(var("n"), succ(var("k")))),
                            succ(apps(var("f"), &[var("k"), app(g("pred"), var("n"))])),
                        ),
                    ],
                ),
            ),
        ),
    );
    let r = infer_type(&ctx, &bogus);
    assert!(
        r.is_err(),
        "`n ≠ Succ k` leaves `n` free to be `Succ j` for any other `j`, so the `Zero` branch of \
         `pred` is NOT ruled out and `pred n` is not a subterm: {r:?}"
    );
}

/// NEGATIVE 12 — the classic escapes stay closed. Widening the measure must not reopen the
/// higher-order escape or the bare non-decreasing self-call.
#[test]
fn the_classic_escapes_stay_closed_under_the_lexicographic_measure() {
    let ctx = std_ctx();

    let diverge = fix("f", lam("n", nat(), app(var("f"), var("n"))));
    assert!(
        infer_type(&ctx, &diverge).is_err(),
        "`fix f. λn. f n` must stay refused"
    );

    // `f` smuggled as a first-class argument in a two-column fixpoint.
    let smuggle = two_column_fix(
        app(
            lam(
                "k",
                arrow(nat(), arrow(nat(), nat())),
                apps(var("k"), &[g("Zero"), g("Zero")]),
            ),
            var("f"),
        ),
        g("Zero"),
        apps(var("f"), &[var("m1"), var("n2")]),
    );
    assert!(
        infer_type(&ctx, &smuggle).is_err(),
        "the recursive name passed as a value is still the higher-order escape"
    );
}

// ==== THE POSITIVES — the capability the fences are drawn around ====

/// Ackermann, the corpus case, made explicit: the inner call `ack (Succ m1) n2` holds column 0
/// EQUAL (`Succ m1 ≡ m`) and strictly decreases column 1 (`n2 ◁ n`), while the outer calls
/// decrease column 0. Both are lexicographic descents on `(m, n)`.
#[test]
fn a_lexicographic_descent_is_admitted_in_both_of_its_shapes() {
    let ctx = std_ctx();

    // Column 0 decreases, column 1 grows: `f m1 (Succ n)` — admissible, lex only looks left.
    let left_wins = two_column_fix(
        g("Zero"),
        g("Zero"),
        apps(var("f"), &[var("m1"), succ(var("n"))]),
    );
    assert!(
        infer_type(&ctx, &left_wins).is_ok(),
        "a strict decrease at column 0 admits ANY column-1 argument — that is what \
         lexicographic means"
    );

    // Column 0 held equal by the BARE variable, column 1 decreases: `f m n2`.
    let bare_equal = two_column_fix(g("Zero"), g("Zero"), apps(var("f"), &[var("m"), var("n2")]));
    assert!(
        infer_type(&ctx, &bare_equal).is_ok(),
        "passing a column's own parameter unchanged holds it EQUAL, so column 1's descent counts"
    );

    // Column 0 held equal by the REBUILT scrutinee, column 1 decreases — Ackermann's shape.
    let rebuilt_equal = two_column_fix(
        g("Zero"),
        g("Zero"),
        apps(var("f"), &[succ(var("m1")), var("n2")]),
    );
    assert!(
        infer_type(&ctx, &rebuilt_equal).is_ok(),
        "`f (Succ m1) n2` under `match m with Succ m1` is Ackermann's inner call"
    );
}

/// BOUNDARY — one column above and one below. A single-column fixpoint keeps exactly its old
/// answers (column 0 equal ⇒ refused), and a THREE-column one must scan past two equal columns
/// to find the third's descent while still refusing the same shape with the third held equal.
#[test]
fn the_lexicographic_scan_walks_past_equal_columns_and_stops_at_an_unknown_one() {
    let ctx = std_ctx();
    let three = |call: Term| {
        fix(
            "f",
            lam(
                "a",
                nat(),
                lam(
                    "b",
                    nat(),
                    lam(
                        "c",
                        nat(),
                        match_(
                            var("a"),
                            nat_motive(),
                            vec![
                                g("Zero"),
                                lam(
                                    "a1",
                                    nat(),
                                    match_(
                                        var("c"),
                                        nat_motive(),
                                        vec![g("Zero"), lam("c1", nat(), call.clone())],
                                    ),
                                ),
                            ],
                        ),
                    ),
                ),
            ),
        )
    };
    // Columns are [a@0, b@1, c@2]: `f (Succ a1) b c1` is (=, =, ◁).
    assert!(
        infer_type(
            &ctx,
            &three(apps(var("f"), &[succ(var("a1")), var("b"), var("c1")]))
        )
        .is_ok(),
        "two equal columns then a strict decrease is a lexicographic descent"
    );
    // (=, =, =) is the loop.
    assert!(
        infer_type(
            &ctx,
            &three(apps(var("f"), &[succ(var("a1")), var("b"), var("c")]))
        )
        .is_err(),
        "three equal columns decrease nothing"
    );
    // (=, UNKNOWN, ◁) must STOP at column 1 — an un-analysable middle column blocks the scan,
    // because nothing says `Succ b` is not larger and lex needs every earlier column to be ≤.
    assert!(
        infer_type(
            &ctx,
            &three(apps(
                var("f"),
                &[succ(var("a1")), succ(var("b")), var("c1")]
            ))
        )
        .is_err(),
        "an unknown column must halt the scan, not be skipped over"
    );
}

/// The MUTUAL block gets one shared measure. A call to a sibling is compared column-by-column
/// against the CURRENT body's columns, so holding column 0 equal and decreasing column 1 across
/// the block is admitted — and holding every column equal across the block is not.
#[test]
fn a_mutual_block_shares_one_lexicographic_measure() {
    let ctx = std_ctx();
    let member = |me: &str, other: &str, call: Term| {
        (
            me.to_string(),
            lam(
                "m",
                nat(),
                lam(
                    "n",
                    nat(),
                    match_(
                        var("m"),
                        nat_motive(),
                        vec![
                            g("Zero"),
                            lam(
                                "m1",
                                nat(),
                                match_(
                                    var("n"),
                                    nat_motive(),
                                    vec![g("Zero"), lam("n2", nat(), call.clone())],
                                ),
                            ),
                        ],
                    ),
                ),
            ),
            other.to_string(),
        )
    };
    let (n0, b0, _) = member("p", "q", apps(var("q"), &[succ(var("m1")), var("n2")]));
    let (n1, b1, _) = member("q", "p", apps(var("p"), &[var("m1"), var("n")]));
    let ok = mfix(&[(n0.as_str(), b0), (n1.as_str(), b1)], 0);
    assert!(
        infer_type(&ctx, &ok).is_ok(),
        "a sibling call holding column 0 equal and decreasing column 1 is a lex descent for the \
         whole block"
    );

    let (n0, b0, _) = member("p", "q", apps(var("q"), &[succ(var("m1")), var("n")]));
    let (n1, b1, _) = member("q", "p", apps(var("p"), &[var("m"), var("n")]));
    let bad = mfix(&[(n0.as_str(), b0), (n1.as_str(), b1)], 0);
    assert!(
        infer_type(&ctx, &bad).is_err(),
        "a mutual block whose members hand each other an unchanged tuple never terminates"
    );
}

/// The `decreasing_by` capability, stated as the capability rather than as its one test case:
/// under `hn : ¬(n = Zero)` — the refutation the `isFalse` branch of a CHECKED decision
/// procedure binds — `pred n` really is a strict subterm of `n`, because the only branch of
/// `pred` that fails to produce one is the `Zero` branch the refutation excludes.
#[test]
fn a_refuted_constructor_branch_drops_out_of_the_subterm_meet() {
    let mut ctx = std_ctx();
    with_pred(&mut ctx);
    let down_from = fix(
        "f",
        lam(
            "n",
            nat(),
            match_(
                apps(g("decEqNat"), &[var("n"), g("Zero")]),
                nat(),
                vec![
                    lam("hp", eq_nat(var("n"), g("Zero")), g("Zero")),
                    lam(
                        "hn",
                        app(g("Not"), eq_nat(var("n"), g("Zero"))),
                        succ(app(var("f"), app(g("pred"), var("n")))),
                    ),
                ],
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &down_from).is_ok(),
        "`decEqNat` is a kernel-checked definition, so the `isFalse` branch is reachable only \
         where `n` really is not `Zero`; `pred n` is then `n`'s `Succ` field"
    );
}

// ==== THE PARADOX FENCE — nested inductives through an impure container ====

/// A nested inductive's container is judged FIELD BY FIELD, and a field the translation cannot
/// carry soundly is refused by name. A CLOSED extra field (`Nat` in `LCons : Π a. a → Nat →
/// LList a → LList a`) is not such a field: Lean 4.14 accepts
/// `inductive LTree2 | node : Labelled2 LTree2 → LTree2` over exactly this container (see the
/// `inductive_compile` module header, which records the run), and the emitted block is still
/// checked by the trusted mutual door. The hazards are a field that puts the nesting parameter
/// in a DOMAIN (negative) and a field that is itself a SORT; each is refused, nothing is
/// registered, and the refusal names the container and the constructor it could not translate.
#[test]
fn an_impure_container_is_refused_with_a_reason_that_names_the_hazard() {
    use logicaffeine_kernel::inductive_compile::NestedDecl;
    let list_like = |ctx: &mut Context, name: &str, extra: Term| {
        let (nil, cons) = (format!("{name}Nil"), format!("{name}Cons"));
        ctx.add_inductive(name, pi("a", sort(0), sort(0)));
        ctx.add_constructor(&nil, name, pi("a", sort(0), app(g(name), var("a"))));
        ctx.add_constructor(
            &cons,
            name,
            pi(
                "a",
                sort(0),
                arrow(
                    var("a"),
                    arrow(extra, arrow(app(g(name), var("a")), app(g(name), var("a")))),
                ),
            ),
        );
        ctx.set_inductive_params(name, 1);
    };
    let tree_through = |tree: &str, container: &str| NestedDecl {
        name: tree.to_string(),
        sort: sort(0),
        constructors: vec![(
            format!("{tree}Node"),
            arrow(app(g(container), g(tree)), g(tree)),
        )],
    };

    // The closed carried field Lean nests through: accepted, and one sibling is emitted.
    let mut ctx = std_ctx();
    list_like(&mut ctx, "LList", nat());
    let info = ctx
        .add_nested_inductive(&tree_through("LTree", "LList"))
        .expect("a closed `Nat` field rides through the mirror, exactly as Lean accepts it");
    assert_eq!(
        info.siblings,
        vec!["LTree$LList".to_string()],
        "one container level, one sibling"
    );
    assert!(
        ctx.is_inductive("LTree"),
        "the accepted declaration is registered"
    );

    // HAZARD 1: the nesting parameter in a DOMAIN — `(a → Nat)` becomes `LTree → Nat` inside
    // `LTree`'s own constructor, the paradox shape.
    let mut ctx = std_ctx();
    list_like(&mut ctx, "NList", arrow(var("a"), nat()));
    let err = ctx
        .add_nested_inductive(&tree_through("NTree", "NList"))
        .expect_err("a field with the nesting parameter in a domain is refused");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("NList") && msg.contains("NListCons") && msg.contains("negative"),
        "the refusal must name the container, its constructor and the negative occurrence, got {msg}"
    );
    assert!(
        !ctx.is_inductive("NTree"),
        "a refused nesting registers nothing"
    );

    // HAZARD 2: a SORT-valued field, which ι-reduction would count as a parameter.
    let mut ctx = std_ctx();
    list_like(&mut ctx, "QList", Term::Sort(Universe::Prop));
    let err = ctx
        .add_nested_inductive(&tree_through("QTree", "QList"))
        .expect_err("a sort-valued container field is refused");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("QList") && msg.contains("QListCons") && msg.contains("sort"),
        "the refusal must name the container, its constructor and the sort-valued field, got {msg}"
    );
    assert!(
        !ctx.is_inductive("QTree"),
        "a refused nesting registers nothing"
    );
}

/// The control: a PURE unary container still nests. The fence above must be a fence, not a wall.
#[test]
fn a_pure_unary_container_still_nests() {
    use logicaffeine_kernel::inductive_compile::NestedDecl;
    let mut ctx = std_ctx();
    ctx.add_inductive("PList", pi("a", sort(0), sort(0)));
    ctx.add_constructor("PNil", "PList", pi("a", sort(0), app(g("PList"), var("a"))));
    ctx.add_constructor(
        "PCons",
        "PList",
        pi(
            "a",
            sort(0),
            arrow(
                var("a"),
                arrow(app(g("PList"), var("a")), app(g("PList"), var("a"))),
            ),
        ),
    );
    ctx.set_inductive_params("PList", 1);

    let decl = NestedDecl {
        name: "PTree".to_string(),
        sort: sort(0),
        constructors: vec![(
            "PNode".to_string(),
            arrow(app(g("PList"), g("PTree")), g("PTree")),
        )],
    };
    assert!(
        ctx.add_nested_inductive(&decl).is_ok(),
        "a rose tree through a pure unary list is the whole point of nested inductives"
    );
}

// ==== THE DERIVED ELIMINATOR — what still computes after the guard moved ====

/// The derived `Nat` eliminator must keep ι-computing. The guard is what admits its `Fix`, so
/// every widening has to be re-checked against the recursors the prelude builds.
#[test]
fn the_derived_nat_recursor_still_iota_computes() {
    use logicaffeine_kernel::{derive_recursor, normalize};
    let mut ctx = std_ctx();
    let (rec_ty, rec_body) = derive_recursor(&ctx, "Nat").expect("Nat's eliminator must derive");
    ctx.add_definition("G_Nat_rec".to_string(), rec_ty, rec_body);
    let add_rec = apps(
        g("G_Nat_rec"),
        &[
            lam("_", nat(), nat()),
            lit(5),
            lam("_", nat(), lam("ih", nat(), succ(var("ih")))),
            lit(2),
        ],
    );
    assert!(
        infer_type(&ctx, &add_rec).is_ok(),
        "the derived recursor must still type-check"
    );
    assert_eq!(
        normalize(&ctx, &add_rec),
        succ(succ(lit(5))),
        "5 + 2 = 7 through the derived eliminator"
    );
}

// ==== THE GUARD'S OWN EDGES — shapes `infer_type` would also reject, checked directly ====

/// A `match` with MORE cases than the inductive has constructors must not let the extra case go
/// unvisited. The positional zip that pairs cases with constructors silently DROPS the surplus,
/// and a non-decreasing recursive call inside one would then never be guarded. The type checker
/// rejects such a term on its own, but a guard that is only safe because something else is
/// watching is not a guard — so this calls `check_termination` directly.
#[test]
fn a_surplus_match_case_is_still_guarded() {
    use logicaffeine_kernel::termination::check_termination;
    let ctx = std_ctx();
    let body = lam(
        "n",
        nat(),
        match_(
            var("n"),
            nat_motive(),
            vec![
                g("Zero"),
                lam("k", nat(), app(var("f"), var("k"))),
                // The surplus third case: `Nat` has two constructors, so nothing pairs with it.
                app(var("f"), var("n")),
            ],
        ),
    );
    assert!(
        check_termination(&ctx, "f", &body).is_err(),
        "the surplus case holds `f n`, which decreases nothing — dropping it from the walk is a \
         hole in the guard whatever the type checker does afterwards"
    );
}

/// A `let` ALIAS inherits exactly the smallness of what it names, at the same column — and
/// nothing more. `let m := k` where `k` is a field is admissible because ζ makes the two the same
/// term; `let m := n` where `n` is the column's parameter is not a decrease.
#[test]
fn a_let_alias_inherits_smallness_and_not_more() {
    use logicaffeine_kernel::termination::check_termination;
    let ctx = std_ctx();
    let with_alias = |aliased: &str| {
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                nat_motive(),
                vec![
                    g("Zero"),
                    lam(
                        "k",
                        nat(),
                        Term::Let {
                            name: "m".to_string(),
                            ty: Box::new(nat()),
                            value: Box::new(var(aliased)),
                            body: Box::new(app(var("f"), var("m"))),
                        },
                    ),
                ],
            ),
        )
    };
    assert!(
        check_termination(&ctx, "f", &with_alias("k")).is_ok(),
        "`let m := k in f m` is `f k` after ζ, and `k` is the matched field"
    );
    assert!(
        check_termination(&ctx, "f", &with_alias("n")).is_err(),
        "`let m := n in f m` is `f n` after ζ, which decreases nothing"
    );
}

/// The paradox fence, stated where it bites: strict positivity must refuse a NEGATIVE recursive
/// occurrence however it is dressed, and accept the FUNCTIONAL positive one `Acc_intro` needs.
/// `Acc` is the control — if the fence ever tightened to "the inductive only as a bare domain",
/// well-founded recursion would stop existing.
#[test]
fn the_positivity_fence_refuses_negative_and_admits_functional_positive() {
    use logicaffeine_kernel::check_positivity_for_test as positivity;

    // NEGATIVE, direct: `(Bad → False) → Bad`.
    let bad_to_false = arrow(g("Bad"), g("False"));
    assert!(
        positivity("Bad", "mk", &arrow(bad_to_false.clone(), g("Bad"))).is_err(),
        "`(Bad → False) → Bad` is Russell's paradox and must be refused"
    );
    // NEGATIVE, buried one arrow deeper: `((Bad → Nat) → Nat) → Bad`.
    assert!(
        positivity(
            "Bad",
            "mk",
            &arrow(arrow(arrow(g("Bad"), nat()), nat()), g("Bad"))
        )
        .is_err(),
        "a negative occurrence two arrows deep is still negative"
    );
    // NEGATIVE, in a LATER domain of a functional field: `(Nat → Bad → Bad) → Bad`.
    assert!(
        positivity(
            "Bad",
            "mk",
            &arrow(arrow(nat(), arrow(g("Bad"), g("Bad"))), g("Bad"))
        )
        .is_err(),
        "a functional field may place the inductive only in its RESULT, never in a domain"
    );
    // POSITIVE, functional — the `Acc_intro` shape `Π(y:A). R y x → Acc A R y`.
    assert!(
        positivity(
            "Acc2",
            "intro",
            &arrow(pi("y", nat(), arrow(g("Bool"), g("Acc2"))), g("Acc2"))
        )
        .is_ok(),
        "the functional positive field is what makes well-founded recursion expressible"
    );
    // POSITIVE, direct, alongside data: `Nat → Bad → Bad`.
    assert!(
        positivity("Bad", "cons", &arrow(nat(), arrow(g("Bad"), g("Bad")))).is_ok(),
        "an ordinary recursive constructor must still pass"
    );
}

/// A recursive occurrence in the DOMAIN of a λ-telescope binder that precedes the structural
/// argument must be caught. The guard peels the whole telescope before walking the body, so those
/// annotations are only visited if something explicitly visits them — and `f Zero` sitting in the
/// type of a binder the recursion never destructs is a recursive occurrence in a position no
/// decrease was ever checked for.
#[test]
fn a_recursive_occurrence_in_a_preceding_binders_domain_is_caught() {
    use logicaffeine_kernel::termination::check_termination;
    let ctx = std_ctx();
    // fix f. λ(h : f Zero). λn:Nat. match n with | Zero => Zero | Succ k => f h k
    // The structural column is `n` at position 1, so `h`'s own domain annotation sits BEFORE it.
    let body = lam(
        "h",
        app(var("f"), g("Zero")),
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                nat_motive(),
                vec![
                    g("Zero"),
                    lam("k", nat(), apps(var("f"), &[var("h"), var("k")])),
                ],
            ),
        ),
    );
    assert!(
        check_termination(&ctx, "f", &body).is_err(),
        "`f Zero` in a telescope binder's domain is an unguarded recursive occurrence — the \
         recursion never decreases anything to reach it"
    );
}

// =======================================================================================
// WAVE 2 — THE MOTIVE-SORT RULE, BOTH DECLARATIONS
// =======================================================================================
//
// The rule, stated once here and once in `recursor.rs::motive_target`:
//
//   * `derive_recursor` builds the MONOMORPHIC eliminator, whose motive targets the
//     inductive's OWN sort — concretely. `Nat : Type 0` gets `Type 0`; an inductive declared
//     at `Type 1` gets `Type 1`, which is the level its own constructor fields live at and
//     the whole reason it was declared one level up.
//   * `derive_recursor_universe_poly` builds the UNIVERSE-POLYMORPHIC one, Lean's
//     `Nat.rec.{u}`, whose motive targets a level PARAMETER.
//
// They are two DECLARATIONS, not two spellings of one: a level variable in the motive's
// target makes `Type 0 ⋠ u` (a variable ranges over `Prop` too), so claiming the polymorphic
// type loses the concrete instance. Wave 1 landed the polymorphic spelling inside the
// monomorphic door and four tests went red on exactly that loss.

/// `Prop`, spelled once.
fn prop_() -> Term {
    Term::Sort(Universe::Prop)
}

/// The motive binder's target sort, read off a derived recursor's first `Π` whose parameter is
/// a telescope ending in a `Sort` over `head`.
fn motive_sort_of(rec_ty: &Term, head: &str) -> Term {
    let mut cur = rec_ty;
    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = cur
    {
        let mut inner = param_type.as_ref();
        let mut last_dom: Option<&Term> = None;
        while let Term::Pi {
            param_type: d,
            body_type: b,
            ..
        } = inner
        {
            last_dom = Some(d);
            inner = b;
        }
        if let (Term::Sort(_), Some(dom)) = (inner, last_dom) {
            let mut h = dom;
            while let Term::App(f, _) = h {
                h = f;
            }
            if matches!(h, Term::Global(n) if n == head) {
                return inner.clone();
            }
        }
        cur = body_type;
    }
    panic!("no motive binder over `{head}` in {rec_ty}");
}

#[test]
fn the_monomorphic_eliminator_targets_its_own_sort_and_the_polymorphic_one_a_parameter() {
    use logicaffeine_kernel::recursor::{derive_recursor, derive_recursor_universe_poly};

    // The monomorphic door: `Nat`'s own sort, concretely, with NO level parameter anywhere.
    let ctx = std_ctx();
    let (mono_ty, _) = derive_recursor(&ctx, "Nat").expect("Nat's eliminator derives");
    assert_eq!(
        motive_sort_of(&mono_ty, "Nat"),
        sort(0),
        "the monomorphic eliminator's motive is `Nat → Type 0`, concretely — a level VARIABLE \
         here is the wave-1 regression, and it costs `Type 0 ≤ u` which is false"
    );

    // The polymorphic door: one level parameter, and it is the motive's target.
    let (levels, poly_ty, _) =
        derive_recursor_universe_poly(&ctx, "Nat").expect("Nat's polymorphic eliminator derives");
    assert_eq!(
        levels.len(),
        1,
        "one motive, one level parameter, got {levels:?}"
    );
    assert_eq!(
        motive_sort_of(&poly_ty, "Nat"),
        Term::Sort(Universe::Var(levels[0].clone())),
        "the polymorphic eliminator's motive is `Nat → Sort u`"
    );
    assert!(
        !levels[0].starts_with('?'),
        "a DECLARATION's levels are parameters, not metavariables awaiting a solution: {levels:?}"
    );
}

#[test]
fn the_polymorphic_nat_eliminator_iota_computes_and_reaches_type_one() {
    use logicaffeine_kernel::recursor::derive_recursor_universe_poly;
    use logicaffeine_kernel::{double_check, normalize, DoubleCheck};

    let mut ctx = std_ctx();
    let (_levels, ty_, tm) =
        derive_recursor_universe_poly(&ctx, "Nat").expect("the polymorphic eliminator derives");
    assert_eq!(
        double_check(&ctx, &tm),
        DoubleCheck::Agreed,
        "both kernels must certify the polymorphic eliminator's TERM, got {:?}",
        double_check(&ctx, &tm)
    );
    ctx.add_definition("GNatRecU".to_string(), ty_, tm);

    // It still ι-computes at the ordinary level: 5 + 2 = 7.
    let add_rec = apps(
        g("GNatRecU"),
        &[
            lam("_", nat(), nat()),
            lit(5),
            lam("_", nat(), lam("ih", nat(), succ(var("ih")))),
            lit(2),
        ],
    );
    assert!(
        infer_type(&ctx, &add_rec).is_ok(),
        "the `Type 0` instance must still type-check"
    );
    assert_eq!(
        normalize(&ctx, &add_rec),
        succ(succ(lit(5))),
        "5 + 2 = 7, polymorphically"
    );

    // …AND it reaches `Type 1`, which is the whole point: `Nat.rec (motive := fun _ => Type 0)`
    // builds a `Type 1`-valued family by recursion, ordinary mathematics that the monomorphic
    // eliminator has no image for.
    let tower = apps(
        g("GNatRecU"),
        &[
            lam("_", nat(), sort(0)),
            nat(),
            lam("_", nat(), lam("ih", sort(0), var("ih"))),
            lit(2),
        ],
    );
    let inferred = infer_type(&ctx, &tower);
    assert!(
        inferred.is_ok(),
        "a `Type 1`-valued family by recursion over `Nat` must type-check through the \
         polymorphic eliminator: {inferred:?}"
    );
    assert_eq!(
        normalize(&ctx, &tower),
        nat(),
        "…and compute: two applications of the identity to `Nat` is `Nat`"
    );
}

#[test]
fn the_polymorphic_eliminator_also_answers_at_explicit_levels() {
    use logicaffeine_kernel::recursor::derive_recursor_universe_poly;

    // Lean's `Nat.rec.{1}`: the level telescope named outright rather than inferred. The
    // `Const` door is checked positionally by the kernel, so a wrong arity is refused rather
    // than silently padded — the boundary on either side is asserted.
    let mut ctx = std_ctx();
    let (levels, ty_, tm) =
        derive_recursor_universe_poly(&ctx, "Nat").expect("the polymorphic eliminator derives");
    ctx.add_universe_poly("GNatRecConst", levels.clone(), ty_, tm);

    let at = |n: u32| Term::Const {
        name: "GNatRecConst".to_string(),
        levels: vec![Universe::Type(n)],
    };
    let tower = apps(
        at(1),
        &[
            lam("_", nat(), sort(0)),
            nat(),
            lam("_", nat(), lam("ih", sort(0), var("ih"))),
            lit(2),
        ],
    );
    assert!(
        infer_type(&ctx, &tower).is_ok(),
        "`Nat_rec.{{1}}` at an explicit level must eliminate into `Type 1`: {:?}",
        infer_type(&ctx, &tower)
    );

    // THE NEGATIVE. A reference carrying the WRONG number of levels is a statement about a
    // declaration that does not exist, and must be refused — never padded, never truncated.
    for wrong in [vec![], vec![Universe::Type(0), Universe::Type(1)]] {
        let bad = Term::Const {
            name: "GNatRecConst".to_string(),
            levels: wrong.clone(),
        };
        assert!(
            infer_type(&ctx, &bad).is_err(),
            "a {}-level reference to a 1-level declaration must be refused",
            wrong.len()
        );
    }
}

#[test]
fn a_type_one_inductive_gets_its_own_level_monomorphically_and_one_above_it_polymorphically() {
    use logicaffeine_kernel::recursor::{derive_recursor, derive_recursor_universe_poly};

    // THE BOUNDARY, one rung either side. A `Type 1` inductive's monomorphic eliminator must
    // reach `Type 1` — the level its own field lives at — and its polymorphic one must reach
    // `Type 2` as well, without either answer disturbing the other.
    let mut ctx = std_ctx();
    ctx.add_inductive("GBoxed", sort(1));
    ctx.add_constructor_checked("g_boxed", "GBoxed", arrow(sort(0), g("GBoxed")))
        .expect("a `Type 0` field in a `Type 1` inductive is consistent");

    let (mono_ty, mono_tm) = derive_recursor(&ctx, "GBoxed").expect("the eliminator derives");
    assert_eq!(
        motive_sort_of(&mono_ty, "GBoxed"),
        sort(1),
        "its OWN sort, so unboxing the `Type 0` it stores type-checks"
    );
    ctx.add_definition("GBoxedRec".to_string(), mono_ty, mono_tm);
    let unbox = apps(
        g("GBoxedRec"),
        &[lam("_", g("GBoxed"), sort(0)), lam("a", sort(0), var("a"))],
    );
    assert!(
        infer_type(&ctx, &unbox).is_ok(),
        "recovering the stored `Type 0` must check: {:?}",
        infer_type(&ctx, &unbox)
    );

    let (levels, poly_ty, poly_tm) =
        derive_recursor_universe_poly(&ctx, "GBoxed").expect("the polymorphic eliminator derives");
    assert_eq!(
        motive_sort_of(&poly_ty, "GBoxed"),
        Term::Sort(Universe::Var(levels[0].clone())),
        "and the polymorphic one takes the level as a parameter"
    );
    ctx.add_definition("GBoxedRecU".to_string(), poly_ty, poly_tm);
    let one_above = apps(
        g("GBoxedRecU"),
        &[lam("_", g("GBoxed"), sort(1)), lam("_", sort(0), nat())],
    );
    assert!(
        infer_type(&ctx, &one_above).is_ok(),
        "one rung ABOVE its own sort is what the parameter buys: {:?}",
        infer_type(&ctx, &one_above)
    );
}

#[test]
fn a_multi_constructor_prop_gains_no_level_parameter_in_either_door() {
    use logicaffeine_kernel::recursor::{derive_recursor, derive_recursor_universe_poly};

    // THE NEGATIVE THAT MATTERS. Large elimination out of a non-subsingleton `Prop` extracts
    // computational content from a proof. The polymorphic door must not be a way around the
    // gate: asking for a level parameter on a 2-constructor `Prop` must return NO parameter and
    // a `Prop` motive, and a `Type 0` motive must still be refused.
    let mut ctx = std_ctx();
    ctx.add_inductive("GFlip", prop_());
    ctx.add_constructor("g_fa", "GFlip", g("GFlip"));
    ctx.add_constructor("g_fb", "GFlip", g("GFlip"));

    let (mono_ty, _) = derive_recursor(&ctx, "GFlip").expect("the eliminator derives");
    assert_eq!(
        motive_sort_of(&mono_ty, "GFlip"),
        prop_(),
        "a 2-constructor Prop keeps `Prop`"
    );

    let (levels, poly_ty, poly_tm) =
        derive_recursor_universe_poly(&ctx, "GFlip").expect("the polymorphic door answers");
    assert!(
        levels.is_empty(),
        "no level may be offered where none is used — a caller could otherwise instantiate a \
         parameter that appears nowhere: {levels:?}"
    );
    assert_eq!(
        motive_sort_of(&poly_ty, "GFlip"),
        prop_(),
        "the gate is not parameterised: asking for a level must not widen elimination"
    );

    ctx.add_definition("GFlipRecU".to_string(), poly_ty, poly_tm);
    let large = apps(
        g("GFlipRecU"),
        &[lam("_", g("GFlip"), sort(0)), nat(), nat()],
    );
    assert!(
        infer_type(&ctx, &large).is_err(),
        "a `Type 0` motive over a 2-constructor `Prop` must stay refused in BOTH doors — this \
         is large elimination out of a proposition: {:?}",
        infer_type(&ctx, &large)
    );
}

#[test]
fn an_sprop_eliminator_stays_in_sprop_in_both_doors() {
    use logicaffeine_kernel::recursor::{derive_recursor, derive_recursor_universe_poly};

    // THE OTHER GATE. `SProp`'s definitional irrelevance equates every inhabitant, so a match
    // out of one into a relevant sort equates its branches. Neither door may parameterise it.
    let mut ctx = std_ctx();
    ctx.add_inductive("GStrict", Term::Sort(Universe::SProp));
    ctx.add_constructor("g_strict", "GStrict", g("GStrict"));

    let (mono_ty, _) = derive_recursor(&ctx, "GStrict").expect("the eliminator derives");
    assert_eq!(
        motive_sort_of(&mono_ty, "GStrict"),
        Term::Sort(Universe::SProp),
        "`SProp` eliminates only into `SProp`"
    );
    let (levels, poly_ty, _) =
        derive_recursor_universe_poly(&ctx, "GStrict").expect("the polymorphic door answers");
    assert!(
        levels.is_empty(),
        "no parameter where the gate allows none: {levels:?}"
    );
    assert_eq!(
        motive_sort_of(&poly_ty, "GStrict"),
        Term::Sort(Universe::SProp),
        "and the polymorphic door does not reopen it"
    );
}

#[test]
fn a_universe_polymorphic_inductive_keeps_its_own_level_and_gains_no_second_one() {
    use logicaffeine_kernel::recursor::{derive_recursor, derive_recursor_universe_poly};

    // THE CAPTURE FENCE. `GCell.{u} : Sort u → Sort u` already HAS a level, supplied with the
    // inductive. A motive at a second, independently-instantiable level could be solved at
    // `Type 0` while the inductive's own `u` is solved at `Prop` — large elimination out of a
    // proposition, reached by an instantiation rather than by a rule. So the motive keeps the
    // inductive's OWN variable, in both doors.
    let mut ctx = std_ctx();
    ctx.add_inductive(
        "GCell",
        pi(
            "a",
            Term::Sort(Universe::Var("u".to_string())),
            Term::Sort(Universe::Var("u".to_string())),
        ),
    );
    ctx.add_constructor(
        "g_cell",
        "GCell",
        pi(
            "a",
            Term::Sort(Universe::Var("u".to_string())),
            arrow(var("a"), app(g("GCell"), var("a"))),
        ),
    );

    let (mono_ty, _) = derive_recursor(&ctx, "GCell").expect("the eliminator derives");
    assert_eq!(
        motive_sort_of(&mono_ty, "GCell"),
        Term::Sort(Universe::Var("u".to_string())),
        "the inductive's own level, not a fresh one"
    );
    let (levels, poly_ty, _) =
        derive_recursor_universe_poly(&ctx, "GCell").expect("the polymorphic door answers");
    assert_eq!(
        levels,
        vec!["u".to_string()],
        "the telescope is the inductive's OWN level and nothing else — a declaration already \
         carrying its own level must not acquire a second"
    );
    assert_eq!(
        motive_sort_of(&poly_ty, "GCell"),
        Term::Sort(Universe::Var("u".to_string())),
        "…and the motive still rides on the inductive's own variable"
    );
}

#[test]
fn a_subsingleton_prop_eliminates_at_every_level_in_the_polymorphic_door() {
    use logicaffeine_kernel::recursor::{derive_recursor, derive_recursor_universe_poly};

    // `False`, `Eq` and `And` are subsingletons: Lean's `False.rec.{u}`/`Eq.rec.{u}`/`And.rec.{u}`
    // take the motive's sort as a parameter, and the LICENCE for that is the subsingleton test —
    // the very same test the monomorphic rule already spends on `Type 0`. Parameterising it
    // grants no elimination that was not already granted; it grants the LEVELS of it.
    let ctx = std_ctx();
    for ind in ["False", "Eq"] {
        let (mono_ty, _) = derive_recursor(&ctx, ind).expect("the eliminator derives");
        assert_eq!(
            motive_sort_of(&mono_ty, ind),
            sort(0),
            "`{ind}` is a subsingleton, so the monomorphic rule spends the licence on `Type 0`"
        );
        let (levels, poly_ty, _) =
            derive_recursor_universe_poly(&ctx, ind).expect("the polymorphic door answers");
        // The telescope covers the eliminator's type, in Lean's order: the motive's level first,
        // then the levels the inductive's own arity binds (`Eq.{u}`'s carrier; `False` has none).
        let own: Vec<String> = if ind == "Eq" {
            vec!["u".to_string()]
        } else {
            Vec::new()
        };
        assert_eq!(
            levels.len(),
            1 + own.len(),
            "`{ind}` earns exactly one level parameter beside its own {own:?}, got {levels:?}"
        );
        assert_eq!(
            &levels[1..],
            own.as_slice(),
            "…and keeps its own levels after the motive's"
        );
        assert!(
            !own.contains(&levels[0]),
            "the motive's level is a NEW one: {levels:?}"
        );
        assert_eq!(
            motive_sort_of(&poly_ty, ind),
            Term::Sort(Universe::Var(levels[0].clone())),
            "…and the polymorphic rule spends it on a parameter"
        );
    }

    // The payoff, made concrete: ex falso into `Type 1`.
    let mut c = std_ctx();
    let (levels, ty_, tm) =
        derive_recursor_universe_poly(&c, "False").expect("False's polymorphic eliminator derives");
    assert_eq!(levels.len(), 1);
    c.add_definition("GFalseRecU".to_string(), ty_, tm);
    c.add_declaration("g_absurd", g("False"));
    let ex_falso = apps(
        g("GFalseRecU"),
        &[lam("_", g("False"), sort(1)), g("g_absurd")],
    );
    assert!(
        infer_type(&c, &ex_falso).is_ok(),
        "ex falso must reach `Type 1`, which is the level Lean's `False.rec.{{u}}` reaches: {:?}",
        infer_type(&c, &ex_falso)
    );
}

// =======================================================================================
// WAVE 2 — NESTED INDUCTIVES: THE FENCE IS CLOSEDNESS, NOT "ELEMENT OR SPINE"
// =======================================================================================
//
// Wave 1 refused every container field that was neither the element nor the spine. The
// REASONING was right — such a field must not enter the specialised `Type 0` sibling with its
// strict positivity and its universe unchecked — but the answer was wrong, because the check
// already exists: `Context::add_mutual_inductives` runs `check_positivity_mutual` over the whole
// emitted block and `check_constructor_universes` on every member, transactionally. So a CLOSED
// carried field is carried and CHECKED; the shapes the mirror genuinely cannot express are
// refused, each by name. The tests below are the pair: what is now carried, and every neighbour
// that must still be refused.

/// A container `Π(A:Type 0)` with the given extra value fields before the element and the spine.
fn register_container(ctx: &mut Context, name: &str, extra_before: &[Term]) {
    ctx.add_inductive(name, pi("A", sort(0), sort(0)));
    ctx.add_constructor(
        &format!("{name}_nil"),
        name,
        pi("A", sort(0), app(g(name), var("A"))),
    );
    let mut ty_ = arrow(
        var("A"),
        arrow(app(g(name), var("A")), app(g(name), var("A"))),
    );
    for e in extra_before.iter().rev() {
        ty_ = arrow(e.clone(), ty_);
    }
    ctx.add_constructor(&format!("{name}_cons"), name, pi("A", sort(0), ty_));
}

/// `NestedDecl` for `name := node : Container name → name`.
fn nested_through(name: &str, container: &str) -> logicaffeine_kernel::NestedDecl {
    logicaffeine_kernel::NestedDecl {
        name: name.to_string(),
        sort: sort(0),
        constructors: vec![(
            format!("{name}_node"),
            arrow(app(g(container), g(name)), g(name)),
        )],
    }
}

#[test]
fn a_closed_carried_field_rides_through_and_its_isos_round_trip_by_computation() {
    use logicaffeine_kernel::{double_check, normalize, DoubleCheck};

    // `KCons : Π(A). Nat → A → KList A → KList A` — the counted container Lean nests through
    // every day. Registering is not computing, so the round trip is taken by REDUCTION on a
    // closed value, which is the only thing that shows the carried slot is not silently dropped
    // or reordered.
    let mut ctx = std_ctx();
    register_container(&mut ctx, "KList", &[nat()]);
    let info = ctx
        .add_nested_inductive(&nested_through("KTree", "KList"))
        .expect("a closed carried field is inside the fragment");
    assert_eq!(info.siblings.len(), 1, "one container level, one sibling");
    let sib = info.siblings[0].clone();

    let ctors: Vec<(String, Term)> = ctx
        .get_constructors(&sib)
        .iter()
        .map(|(n, t)| (n.to_string(), (*t).clone()))
        .collect();
    assert_eq!(
        ctors.len(),
        2,
        "both of `KList`'s constructors are mirrored"
    );
    assert_eq!(
        ctors[1].1,
        arrow(nat(), arrow(g("KTree"), arrow(g(&sib), g(&sib)))),
        "the `Nat` rides through UNSPECIALISED and IN PLACE: `Nat → KTree → {sib} → {sib}`"
    );

    let iso = &info.isos[0];
    for name in [&iso.to_generic, &iso.from_generic] {
        assert_eq!(
            double_check(&ctx, &g(name)),
            DoubleCheck::Agreed,
            "both kernels must certify iso {name}, got {:?}",
            double_check(&ctx, &g(name))
        );
    }

    // leaf : KTree = KTree_node (empty specialised list)
    let leaf = app(g("KTree_node"), g(&ctors[0].0));
    // spec : sib = sib_cons 7 leaf sib_nil — the carried `7` is the value the round trip must
    // return unchanged and in its own slot.
    let spec = apps(g(&ctors[1].0), &[lit(7), leaf.clone(), g(&ctors[0].0)]);
    let to = g(&iso.to_generic);
    let from = g(&iso.from_generic);
    assert_eq!(
        normalize(&ctx, &app(from.clone(), app(to.clone(), spec.clone()))),
        normalize(&ctx, &spec),
        "from_generic (to_generic spec) = spec — the carried `7` survives both directions"
    );

    // …and the generic direction, through the container's own constructor.
    let gen = apps(
        g("KList_cons"),
        &[g("KTree"), lit(7), leaf, app(g("KList_nil"), g("KTree"))],
    );
    assert_eq!(
        normalize(&ctx, &app(to, app(from, gen.clone()))),
        normalize(&ctx, &gen),
        "to_generic (from_generic gen) = gen"
    );
}

#[test]
fn a_carried_field_above_the_siblings_universe_is_refused_by_the_trusted_door() {
    // THE NEGATIVE THE OLD BLANKET REFUSAL WAS PROTECTING, kept as a property rather than as a
    // refusal of the whole shape. `Type 0 → Nat` is a CLOSED field, so the mirror can express
    // it — but its sort is `Type 1`, and the specialised sibling is emitted at `Type 0`. The
    // trusted mutual door must refuse it, and the refusal must be TRANSACTIONAL: nothing of the
    // half-built block may survive in the context.
    let mut ctx = std_ctx();
    register_container(&mut ctx, "ULarge", &[arrow(sort(0), nat())]);
    let before_inductive = ctx.is_inductive("UTree");
    let out = ctx.add_nested_inductive(&nested_through("UTree", "ULarge"));
    assert!(
        out.is_err(),
        "a `Type 1`-sorted carried field cannot live in a `Type 0` sibling: {out:?}"
    );
    assert_eq!(
        ctx.is_inductive("UTree"),
        before_inductive,
        "the refusal must leave NOTHING behind — a half-registered nested inductive is a type \
         that can be matched on and never inducted over"
    );
    assert!(
        ctx.get_constructors("UTree").is_empty(),
        "…and no constructor may survive either"
    );
}

#[test]
fn a_field_using_a_nesting_parameter_compoundly_is_specialised_and_a_negative_one_refused() {
    use logicaffeine_kernel::{double_check, normalize, DoubleCheck};

    // `Inner A` INSIDE another container's field. Lean's nested-inductive elaborator specialises
    // it recursively — `OTree`'s block gains an `Inner OTree` auxiliary beside the `Outer OTree`
    // one — and so does this compiler now: the inner field is rewritten by the same
    // specialisation as an explicit nested argument, with its own checked conversion pair. The
    // hazard the old blanket refusal named (a mis-mirrored sibling whose values do not
    // correspond to the generic container's) is taken here BY COMPUTATION: both isos are
    // certified by both kernels and a closed value survives the round trip in each direction.
    let mut ctx = std_ctx();
    register_container(&mut ctx, "Inner", &[]);
    ctx.add_inductive("Outer", pi("A", sort(0), sort(0)));
    ctx.add_constructor(
        "Outer_nil",
        "Outer",
        pi("A", sort(0), app(g("Outer"), var("A"))),
    );
    ctx.add_constructor(
        "Outer_cons",
        "Outer",
        pi(
            "A",
            sort(0),
            arrow(
                app(g("Inner"), var("A")),
                arrow(
                    var("A"),
                    arrow(app(g("Outer"), var("A")), app(g("Outer"), var("A"))),
                ),
            ),
        ),
    );
    let info = ctx
        .add_nested_inductive(&nested_through("OTree", "Outer"))
        .expect("a compound use of the nesting parameter is specialised, as Lean specialises it");
    assert_eq!(
        info.siblings,
        vec!["OTree$Inner".to_string(), "OTree$Outer".to_string()],
        "the inner container gets its own sibling, emitted before the outer one that uses it"
    );
    for iso in &info.isos {
        for name in [&iso.to_generic, &iso.from_generic] {
            assert_eq!(
                double_check(&ctx, &g(name)),
                DoubleCheck::Agreed,
                "both kernels must certify iso {name}, got {:?}",
                double_check(&ctx, &g(name))
            );
        }
    }
    let outer = info
        .isos
        .iter()
        .find(|i| i.container == "Outer")
        .expect("an Outer iso");
    let (to, from) = (g(&outer.to_generic), g(&outer.from_generic));
    let sib_nil = |sibling: &str| {
        let ctors = ctx.get_constructors(sibling);
        let (nil, _) = ctors.first().expect("a sibling has constructors");
        g(nil)
    };
    // leaf : OTree = OTree_node (the empty specialised Outer)
    let leaf = app(g("OTree_node"), sib_nil("OTree$Outer"));
    // gen : Outer OTree = Outer_cons OTree (Inner_cons OTree leaf (Inner_nil OTree)) leaf (Outer_nil OTree)
    let inner = apps(
        g("Inner_cons"),
        &[g("OTree"), leaf.clone(), app(g("Inner_nil"), g("OTree"))],
    );
    let gen = apps(
        g("Outer_cons"),
        &[g("OTree"), inner, leaf, app(g("Outer_nil"), g("OTree"))],
    );
    let there_and_back = app(to.clone(), app(from.clone(), gen.clone()));
    infer_type(&ctx, &there_and_back).expect("the generic round trip is well-typed");
    assert_eq!(
        normalize(&ctx, &there_and_back),
        normalize(&ctx, &gen),
        "to_generic (from_generic gen) = gen — the inner `Inner` field is mirrored, not dropped"
    );
    let spec = normalize(&ctx, &app(from.clone(), gen));
    assert_eq!(
        normalize(&ctx, &app(from, app(to, spec.clone()))),
        spec,
        "from_generic (to_generic spec) = spec"
    );

    // THE NEGATIVE the old refusal was standing in for: the same compound shape through an
    // inner container that is NOT positive in its parameter must be refused, transactionally.
    let mut bad = std_ctx();
    register_container(&mut bad, "NegInner", &[arrow(var("A"), nat())]);
    bad.add_inductive("NegOuter", pi("A", sort(0), sort(0)));
    bad.add_constructor(
        "NegOuter_nil",
        "NegOuter",
        pi("A", sort(0), app(g("NegOuter"), var("A"))),
    );
    bad.add_constructor(
        "NegOuter_cons",
        "NegOuter",
        pi(
            "A",
            sort(0),
            arrow(
                app(g("NegInner"), var("A")),
                arrow(app(g("NegOuter"), var("A")), app(g("NegOuter"), var("A"))),
            ),
        ),
    );
    let out = bad.add_nested_inductive(&nested_through("NTree", "NegOuter"));
    let err = out.expect_err("a compound field whose inner container is negative must be refused");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("negative") || msg.contains("Positivity"),
        "the refusal must say the occurrence is negative, got {msg}"
    );
    assert!(!bad.is_inductive("NTree"), "and nothing may be registered");
    assert!(
        bad.get_global("NTree$NegInner").is_none(),
        "…not even a sibling"
    );
}

#[test]
fn a_field_in_a_domain_position_over_the_nesting_parameter_is_still_refused() {
    // THE PARADOX SHAPE, arriving through the container rather than through the declaration.
    // `(A → Nat) → C A` puts the specialised element in a DOMAIN, which is a negative
    // occurrence — `positivity.rs` never sees it, because it only ever inspects the nested
    // inductive's own constructors, not the container's.
    let mut ctx = std_ctx();
    register_container(&mut ctx, "FList", &[arrow(var("A"), nat())]);
    let out = ctx.add_nested_inductive(&nested_through("FTree", "FList"));
    let err = out.expect_err("a negative occurrence through the container must be refused");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("FList") && msg.contains("negative"),
        "the refusal must name the container and say the occurrence is negative, got {msg}"
    );
    assert!(!ctx.is_inductive("FTree"), "and nothing may be registered");
}

#[test]
fn a_field_mentioning_the_container_other_than_as_its_spine_is_refused() {
    // The sibling has ONE fixed argument list, so a self-occurrence at DIFFERENT arguments
    // (`PList Nat` inside `PList A`) has no sibling to name. It is closed over the parameter
    // telescope, so the closedness fence alone would carry it — the container check is what
    // stops it, and this is that check's pin.
    let mut ctx = std_ctx();
    register_container(&mut ctx, "SList", &[]);
    ctx.add_constructor(
        "SList_odd",
        "SList",
        pi(
            "A",
            sort(0),
            arrow(app(g("SList"), nat()), app(g("SList"), var("A"))),
        ),
    );
    let out = ctx.add_nested_inductive(&nested_through("STree", "SList"));
    let err = out.expect_err("a differently-applied self-occurrence must be refused");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("SList") && msg.contains("SList_odd"),
        "the refusal must name the container and the constructor, got {msg}"
    );
}

#[test]
fn the_carried_field_still_recurses_through_the_container() {
    use logicaffeine_kernel::recursor::derive_recursor;
    use logicaffeine_kernel::{double_check, DoubleCheck};

    // The payoff, and the thing a mis-compiled sibling would lose: the nested type's eliminator
    // recurses THROUGH the container, carried field and all, and both kernels certify it.
    let mut ctx = std_ctx();
    register_container(&mut ctx, "MList", &[nat()]);
    ctx.add_nested_inductive(&nested_through("MTreeG", "MList"))
        .expect("it compiles");
    let (_, rec_term) = derive_recursor(&ctx, "MTreeG").expect("MTreeG's recursor derives");
    assert_eq!(
        double_check(&ctx, &rec_term),
        DoubleCheck::Agreed,
        "both kernels must certify the mutual recursor, got {:?}",
        double_check(&ctx, &rec_term)
    );
}

#[test]
fn a_declared_parameter_split_the_constructors_do_not_have_is_still_caught_by_the_compiler() {
    // THE FAIL-CLOSED LAYER. `inductive_num_params` INFERS the params/indices split, so the
    // previous test's non-uniform self-occurrence is caught one gate earlier, as an indexed
    // family. But an EXPLICIT `set_inductive_params` is the authority and overrides the
    // inference — a caller can therefore ASSERT a uniformity the constructors do not have, and
    // the mirror would then emit a sibling whose values do not correspond to the container's.
    // The nested compiler must refuse on its own evidence rather than trusting the claim.
    let mut ctx = std_ctx();
    register_container(&mut ctx, "DList", &[]);
    ctx.add_constructor(
        "DList_odd",
        "DList",
        pi(
            "A",
            sort(0),
            arrow(app(g("DList"), nat()), app(g("DList"), var("A"))),
        ),
    );
    ctx.set_inductive_params("DList", 1);
    let out = ctx.add_nested_inductive(&nested_through("DTree", "DList"));
    let err = out.expect_err("a claimed uniformity the constructors lack must still be refused");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("DList_odd"),
        "the refusal must name the constructor whose self-occurrence is not the spine, got {msg}"
    );
    assert!(!ctx.is_inductive("DTree"), "and nothing may be registered");
}

#[test]
fn the_indexed_family_refusal_names_the_constructor_that_costs_the_uniform_split() {
    // A BETTER DIAGNOSTIC THAN THE COUNT. "1 of its 1 arguments are indices" is true and
    // unactionable: the split is inferred from the constructors, so the repair is always in one
    // of them. Two shapes, two culprits, each named — a permuted result and a non-uniform
    // recursive occurrence.
    let mut ctx = std_ctx();
    ctx.add_inductive("PermC", pi("A", sort(0), pi("B", sort(0), sort(0))));
    ctx.add_constructor(
        "PermC_mk",
        "PermC",
        pi(
            "A",
            sort(0),
            pi("B", sort(0), apps(g("PermC"), &[var("B"), var("A")])),
        ),
    );
    let out = ctx.add_nested_inductive(&logicaffeine_kernel::NestedDecl {
        name: "PermTree".to_string(),
        sort: sort(0),
        constructors: vec![(
            "PermTree_node".to_string(),
            arrow(apps(g("PermC"), &[nat(), g("PermTree")]), g("PermTree")),
        )],
    });
    let msg = format!(
        "{:?}",
        out.expect_err("a permuted result is not a uniform telescope")
    );
    assert!(
        msg.contains("PermC_mk") && msg.contains("returns"),
        "the refusal must name the constructor and what it returns, got {msg}"
    );
}

// =======================================================================================
// WAVE 2 — `Int` AS A REAL INDUCTIVE (T11-016 / T10-004), BUILT AND PROVED HERE
// =======================================================================================
//
// `Int` in the standard prelude is an inductive with ZERO constructors, so it cannot be matched
// on or inducted over, and every one of the twenty-six entries in `tcb_ledger_lock.rs`'s
// `Definable` block — nine operations and seventeen ring/order laws — is an ASSUMPTION. Lean
// assumes none of them: it declares
//
//     inductive Int | ofNat : Nat → Int | negSucc : Nat → Int
//
// and PROVES the lot. `prelude.rs` is not this lane's file, so the declaration itself is a
// one-line ask reported back — but the ask is worth nothing without evidence that the recipe
// WORKS, so the whole construction is built here under `G`-prefixed names, on the same kernel,
// and the three laws that fall out of ι alone are proved rather than described.
//
// Everything below is ordinary kernel material: two constructors, `derive_recursor` for the
// eliminator, definitions by structural recursion, and `refl` at a motive instance. Nothing is
// assumed, nothing is axiomatised, and both kernels certify each artefact.

/// `Nat`'s `Succ`, and the peano numeral, are already above. These are the `G`-prefixed `Int`.
fn gint() -> Term {
    g("GInt")
}
fn gof(n: Term) -> Term {
    app(g("GIntOfNat"), n)
}
fn gneg_succ(n: Term) -> Term {
    app(g("GIntNegSucc"), n)
}
/// `Eq GInt a b`
fn eq_gint(a: Term, b: Term) -> Term {
    apps(g("Eq"), &[gint(), a, b])
}
/// `refl GInt a`
fn refl_gint(a: Term) -> Term {
    apps(g("refl"), &[gint(), a])
}

/// The whole construction: `GInt` with Lean's two constructors, the `Nat` arithmetic it needs,
/// and `GInt` negation / addition / subtraction / multiplication as DEFINITIONS by structural
/// recursion. Returns the context; every definition is registered under a `G`-prefixed name so
/// nothing here can collide with the prelude's opaque `Int` or its primitive `add`/`sub`/`mul`.
fn with_gint(ctx: &mut Context) {
    // ---- the inductive ----------------------------------------------------------------
    ctx.add_inductive("GInt", sort(0));
    ctx.add_constructor_checked("GIntOfNat", "GInt", arrow(nat(), gint()))
        .expect("`ofNat : Nat → Int` is strictly positive and fits `Type 0`");
    ctx.add_constructor_checked("GIntNegSucc", "GInt", arrow(nat(), gint()))
        .expect("`negSucc : Nat → Int` likewise");

    // ---- Nat arithmetic, by structural recursion on the SECOND argument (Lean's choice) --
    // `gnadd m 0 = m`, `gnadd m (n+1) = (gnadd m n) + 1`
    ctx.add_definition(
        "gnadd".to_string(),
        arrow(nat(), arrow(nat(), nat())),
        fix(
            "f",
            lam(
                "m",
                nat(),
                lam(
                    "n",
                    nat(),
                    match_(
                        var("n"),
                        nat_motive(),
                        vec![
                            var("m"),
                            lam("k", nat(), succ(apps(var("f"), &[var("m"), var("k")]))),
                        ],
                    ),
                ),
            ),
        ),
    );
    // `gnpred 0 = 0`, `gnpred (k+1) = k`
    ctx.add_definition(
        "gnpred".to_string(),
        arrow(nat(), nat()),
        lam(
            "x",
            nat(),
            match_(
                var("x"),
                nat_motive(),
                vec![g("Zero"), lam("p", nat(), var("p"))],
            ),
        ),
    );
    // `gnsub m 0 = m`, `gnsub m (n+1) = gnpred (gnsub m n)` — truncated, exactly Lean's `Nat.sub`
    ctx.add_definition(
        "gnsub".to_string(),
        arrow(nat(), arrow(nat(), nat())),
        fix(
            "f",
            lam(
                "m",
                nat(),
                lam(
                    "n",
                    nat(),
                    match_(
                        var("n"),
                        nat_motive(),
                        vec![
                            var("m"),
                            lam(
                                "k",
                                nat(),
                                app(g("gnpred"), apps(var("f"), &[var("m"), var("k")])),
                            ),
                        ],
                    ),
                ),
            ),
        ),
    );
    // `gnmul m 0 = 0`, `gnmul m (n+1) = gnadd (gnmul m n) m`
    ctx.add_definition(
        "gnmul".to_string(),
        arrow(nat(), arrow(nat(), nat())),
        fix(
            "f",
            lam(
                "m",
                nat(),
                lam(
                    "n",
                    nat(),
                    match_(
                        var("n"),
                        nat_motive(),
                        vec![
                            g("Zero"),
                            lam(
                                "k",
                                nat(),
                                apps(
                                    g("gnadd"),
                                    &[apps(var("f"), &[var("m"), var("k")]), var("m")],
                                ),
                            ),
                        ],
                    ),
                ),
            ),
        ),
    );

    // ---- the two `Int` smart constructors Lean's arithmetic is stated through -----------
    // `gnegOfNat 0 = ofNat 0`, `gnegOfNat (k+1) = negSucc k` — the only place `-0 = 0` is decided.
    ctx.add_definition(
        "gnegOfNat".to_string(),
        arrow(nat(), gint()),
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                lam("_", nat(), gint()),
                vec![gof(g("Zero")), lam("k", nat(), gneg_succ(var("k")))],
            ),
        ),
    );
    // `gsubNatNat m n = match n - m with | 0 => ofNat (m - n) | k+1 => negSucc k`
    ctx.add_definition(
        "gsubNatNat".to_string(),
        arrow(nat(), arrow(nat(), gint())),
        lam(
            "m",
            nat(),
            lam(
                "n",
                nat(),
                match_(
                    apps(g("gnsub"), &[var("n"), var("m")]),
                    lam("_", nat(), gint()),
                    vec![
                        gof(apps(g("gnsub"), &[var("m"), var("n")])),
                        lam("k", nat(), gneg_succ(var("k"))),
                    ],
                ),
            ),
        ),
    );

    // ---- `Int` negation, addition, subtraction, multiplication --------------------------
    let gint_motive = || lam("_", gint(), gint());
    ctx.add_definition(
        "gineg".to_string(),
        arrow(gint(), gint()),
        lam(
            "a",
            gint(),
            match_(
                var("a"),
                gint_motive(),
                vec![
                    lam("m", nat(), app(g("gnegOfNat"), var("m"))),
                    lam("m", nat(), gof(succ(var("m")))),
                ],
            ),
        ),
    );
    let bin = || arrow(gint(), arrow(gint(), gint()));
    ctx.add_definition(
        "giadd".to_string(),
        bin(),
        lam(
            "a",
            gint(),
            lam(
                "b",
                gint(),
                match_(
                    var("a"),
                    gint_motive(),
                    vec![
                        lam(
                            "m",
                            nat(),
                            match_(
                                var("b"),
                                gint_motive(),
                                vec![
                                    lam("n", nat(), gof(apps(g("gnadd"), &[var("m"), var("n")]))),
                                    lam(
                                        "n",
                                        nat(),
                                        apps(g("gsubNatNat"), &[var("m"), succ(var("n"))]),
                                    ),
                                ],
                            ),
                        ),
                        lam(
                            "m",
                            nat(),
                            match_(
                                var("b"),
                                gint_motive(),
                                vec![
                                    lam(
                                        "n",
                                        nat(),
                                        apps(g("gsubNatNat"), &[var("n"), succ(var("m"))]),
                                    ),
                                    lam(
                                        "n",
                                        nat(),
                                        gneg_succ(succ(apps(g("gnadd"), &[var("m"), var("n")]))),
                                    ),
                                ],
                            ),
                        ),
                    ],
                ),
            ),
        ),
    );
    // `gisub a b := giadd a (gineg b)` — so `sub_elim` is `refl`, definitionally, which is the
    // shape Lean's `Int.sub` has too.
    ctx.add_definition(
        "gisub".to_string(),
        bin(),
        lam(
            "a",
            gint(),
            lam(
                "b",
                gint(),
                apps(g("giadd"), &[var("a"), app(g("gineg"), var("b"))]),
            ),
        ),
    );
    ctx.add_definition(
        "gimul".to_string(),
        bin(),
        lam(
            "a",
            gint(),
            lam(
                "b",
                gint(),
                match_(
                    var("a"),
                    gint_motive(),
                    vec![
                        lam(
                            "m",
                            nat(),
                            match_(
                                var("b"),
                                gint_motive(),
                                vec![
                                    lam("n", nat(), gof(apps(g("gnmul"), &[var("m"), var("n")]))),
                                    lam(
                                        "n",
                                        nat(),
                                        app(
                                            g("gnegOfNat"),
                                            apps(g("gnmul"), &[var("m"), succ(var("n"))]),
                                        ),
                                    ),
                                ],
                            ),
                        ),
                        lam(
                            "m",
                            nat(),
                            match_(
                                var("b"),
                                gint_motive(),
                                vec![
                                    lam(
                                        "n",
                                        nat(),
                                        app(
                                            g("gnegOfNat"),
                                            apps(g("gnmul"), &[succ(var("m")), var("n")]),
                                        ),
                                    ),
                                    lam(
                                        "n",
                                        nat(),
                                        gof(apps(g("gnmul"), &[succ(var("m")), succ(var("n"))])),
                                    ),
                                ],
                            ),
                        ),
                    ],
                ),
            ),
        ),
    );
}

/// `Succ^n Zero`, spelled out. The explicit Peano form rather than `Literal::Nat`, so every
/// comparison below is on the STRUCTURE the constructors produce: the two are definitionally
/// equal, which makes a `Lit`-vs-`Succ` mismatch invisible to `def_eq` and visible to `assert_eq!`,
/// and it is the structure that shows the recursion actually ran.
fn gpeano(n: u32) -> Term {
    (0..n).fold(g("Zero"), |acc, _| succ(acc))
}

/// The `GInt` value of `n` in Lean's representation: `ofNat n` for `n ≥ 0`, `negSucc (-n-1)`
/// otherwise — the CANONICAL form every computation below is checked against.
fn gint_of(n: i64) -> Term {
    if n >= 0 {
        gof(gpeano(n as u32))
    } else {
        gneg_succ(gpeano((-n - 1) as u32))
    }
}

#[test]
fn int_as_a_real_inductive_has_two_constructors_a_recursor_and_disjointness() {
    use logicaffeine_kernel::recursor::{derive_auxiliaries, derive_recursor};
    use logicaffeine_kernel::{double_check, DoubleCheck};

    let mut ctx = std_ctx();
    with_gint(&mut ctx);

    // The premise T10-004 asserts and the prelude's `Int` fails: two constructors.
    let ctors: Vec<String> = ctx
        .get_constructors("GInt")
        .iter()
        .map(|(n, _)| n.to_string())
        .collect();
    assert_eq!(
        ctors,
        vec!["GIntOfNat".to_string(), "GIntNegSucc".to_string()],
        "Lean's two constructors, in Lean's order"
    );

    // The eliminator derives, and both kernels certify it — so induction over `Int` is a
    // THEOREM-producing tool, not an assumption.
    let (rec_ty, rec_tm) = derive_recursor(&ctx, "GInt").expect("GInt's eliminator derives");
    assert_eq!(
        double_check(&ctx, &rec_tm),
        DoubleCheck::Agreed,
        "both kernels must certify `GInt_rec`, got {:?}",
        double_check(&ctx, &rec_tm)
    );
    assert_eq!(
        motive_sort_of(&rec_ty, "GInt"),
        sort(0),
        "a `Type 0` inductive's motive targets `Type 0`, so a `Prop` motive passes by \
         cumulativity — which is what an induction PROOF needs"
    );

    // Disjointness and injectivity come from the same declaration, derived and kernel-checked,
    // so `Int`'s `noConfusion` is not an axiom either.
    let auxs: Vec<String> = derive_auxiliaries(&ctx, "GInt")
        .into_iter()
        .map(|a| a.name)
        .collect();
    for want in [
        "GInt_rec",
        "GInt_noConfusionType",
        "GInt_noConfusion",
        "GIntOfNat_inj",
        "GIntNegSucc_inj",
    ] {
        assert!(
            auxs.contains(&want.to_string()),
            "`{want}` must be derivable, got {auxs:?}"
        );
    }
}

#[test]
fn the_inductive_int_arithmetic_computes_the_right_answers() {
    use logicaffeine_kernel::normalize;

    // The whole point of constructors: `add`/`sub`/`mul` become DEFINITIONS that ι-COMPUTE,
    // where the prelude's are declarations whose only behaviour is a literal fast path. Every
    // sign combination and both zeros, because `negOfNat` is the one place `-0 = 0` is decided
    // and an off-by-one there is invisible on positives.
    let mut ctx = std_ctx();
    with_gint(&mut ctx);

    let cases: &[(i64, i64)] = &[
        (0, 0),
        (0, 1),
        (1, 0),
        (3, 4),
        (4, 3),
        (-1, 0),
        (0, -1),
        (-1, -1),
        (-3, -4),
        (-4, -3),
        (3, -4),
        (-4, 3),
        (4, -3),
        (-3, 4),
        (2, -2),
        (-2, 2),
    ];
    for &(a, b) in cases {
        let ta = gint_of(a);
        let tb = gint_of(b);
        assert_eq!(
            normalize(&ctx, &apps(g("giadd"), &[ta.clone(), tb.clone()])),
            gint_of(a + b),
            "giadd {a} {b} = {}",
            a + b
        );
        assert_eq!(
            normalize(&ctx, &apps(g("gisub"), &[ta.clone(), tb.clone()])),
            gint_of(a - b),
            "gisub {a} {b} = {}",
            a - b
        );
        assert_eq!(
            normalize(&ctx, &apps(g("gimul"), &[ta.clone(), tb.clone()])),
            gint_of(a * b),
            "gimul {a} {b} = {}",
            a * b
        );
        assert_eq!(
            normalize(&ctx, &app(g("gineg"), ta)),
            gint_of(-a),
            "gineg {a} = {}",
            -a
        );
    }

    // `-0 = 0` on the nose: the canonical form of zero is `ofNat 0`, never `negSucc` of
    // anything, or two equal integers would have two normal forms and `rfl` would break.
    assert_eq!(
        normalize(&ctx, &app(g("gineg"), gof(g("Zero")))),
        gof(g("Zero")),
        "negation must return zero to its canonical `ofNat 0`"
    );
}

#[test]
fn three_of_the_seventeen_assumed_ring_laws_are_proved_outright_from_the_constructors() {
    use logicaffeine_kernel::recursor::derive_recursor;
    use logicaffeine_kernel::{double_check, DoubleCheck};

    // THE PAYOFF, and the thing that cannot be faked: these are PROOF TERMS the kernel checks.
    // `tcb_ledger_lock.rs` classes `sub_elim`, `add_zero` and `mul_zero` as `Definable` — "Lean
    // PROVES this; we assume it". With constructors they are proved by ι alone: one `GInt_rec`
    // and a `refl` per case. No axiom, no `sorry`, and the second kernel agrees.
    let mut ctx = std_ctx();
    with_gint(&mut ctx);
    let (rec_ty, rec_tm) = derive_recursor(&ctx, "GInt").expect("GInt's eliminator derives");
    ctx.add_definition("GInt_rec".to_string(), rec_ty, rec_tm);
    let (nat_rec_ty, nat_rec_tm) = derive_recursor(&ctx, "Nat").expect("Nat's eliminator derives");

    let zero = || gof(g("Zero"));
    let one = || gof(succ(g("Zero")));

    // (1) `sub_elim : Π(a b). a - b = a + (-b)` — definitional, because `gisub` IS that.
    let sub_elim_ty = pi(
        "a",
        gint(),
        pi(
            "b",
            gint(),
            eq_gint(
                apps(g("gisub"), &[var("a"), var("b")]),
                apps(g("giadd"), &[var("a"), app(g("gineg"), var("b"))]),
            ),
        ),
    );
    let sub_elim_tm = lam(
        "a",
        gint(),
        lam(
            "b",
            gint(),
            refl_gint(apps(g("gisub"), &[var("a"), var("b")])),
        ),
    );
    let got = infer_type(&ctx, &sub_elim_tm).expect("the `sub_elim` proof term infers");
    assert!(
        logicaffeine_kernel::is_subtype(&ctx, &got, &sub_elim_ty),
        "`sub_elim` must be proved, not assumed: inferred {got}"
    );

    // (2) `add_zero : Π(a). a + 0 = a` — by induction on the INTEGER, and in the `ofNat` case by
    //     a nested induction on the `Nat` inside it. Every leaf is `refl`, so there is no
    //     appeal to an IH anywhere; the nested induction is there only to put a CONSTRUCTOR in
    //     the recursion's structural position:
    //
    //       `ofNat 0     + ofNat 0` ⤳ `ofNat (gnadd 0 0)`       ⤳ `ofNat 0`
    //       `ofNat (k+1) + ofNat 0` ⤳ `ofNat (gnadd (k+1) 0)`   ⤳ `ofNat (k+1)`
    //       `negSucc m   + ofNat 0` ⤳ `gsubNatNat 0 (Succ m)`   ⤳ `negSucc m`
    //
    //     In Lean the `ofNat` case is plain `rfl` at a VARIABLE, because `Nat.add`'s ι-rule
    //     fires on its SECOND argument — `n + 0 = n` is `rfl` there. Here `reduction.rs` unrolls
    //     a `Fix` only when its FIRST argument is constructor-headed, so `gnadd m 0` is stuck at
    //     a free `m` and the case split is the price. That divergence is a reducer defect, not a
    //     property of the representation: `termination.rs::structural_columns` is the guard's own
    //     answer to which argument ι is on, and the test below pins that the two kernels do not
    //     currently agree about it.
    ctx.add_definition("GNat_rec".to_string(), nat_rec_ty, nat_rec_tm);
    let add_zero_of_nat = lam(
        "m",
        nat(),
        apps(
            g("GNat_rec"),
            &[
                lam(
                    "m",
                    nat(),
                    eq_gint(apps(g("giadd"), &[gof(var("m")), zero()]), gof(var("m"))),
                ),
                refl_gint(gof(g("Zero"))),
                lam(
                    "k",
                    nat(),
                    lam(
                        "ih",
                        eq_gint(apps(g("giadd"), &[gof(var("k")), zero()]), gof(var("k"))),
                        refl_gint(gof(succ(var("k")))),
                    ),
                ),
                var("m"),
            ],
        ),
    );
    let add_zero_tm = lam(
        "a",
        gint(),
        apps(
            g("GInt_rec"),
            &[
                lam(
                    "a",
                    gint(),
                    eq_gint(apps(g("giadd"), &[var("a"), zero()]), var("a")),
                ),
                add_zero_of_nat,
                lam("m", nat(), refl_gint(gneg_succ(var("m")))),
                var("a"),
            ],
        ),
    );
    let add_zero_ty = pi(
        "a",
        gint(),
        eq_gint(apps(g("giadd"), &[var("a"), zero()]), var("a")),
    );
    let got = infer_type(&ctx, &add_zero_tm)
        .unwrap_or_else(|e| panic!("the `add_zero` induction must check: {e:?}"));
    assert!(
        logicaffeine_kernel::is_subtype(&ctx, &got, &add_zero_ty),
        "`add_zero` must be proved, not assumed: inferred {got}"
    );
    ctx.add_definition("gadd_zero".to_string(), add_zero_ty, add_zero_tm);
    assert_eq!(
        double_check(&ctx, &g("gadd_zero")),
        DoubleCheck::Agreed,
        "both kernels must certify the `add_zero` proof, got {:?}",
        double_check(&ctx, &g("gadd_zero"))
    );

    // (3) `mul_zero : Π(a). a * 0 = 0` — likewise, and it needs the `negOfNat` arm to compute:
    //     `negSucc m * ofNat 0` ⤳ `gnegOfNat (gnmul (Succ m) 0)` ⤳ `gnegOfNat 0` ⤳ `ofNat 0`.
    let mul_zero_ty = pi(
        "a",
        gint(),
        eq_gint(apps(g("gimul"), &[var("a"), zero()]), zero()),
    );
    let mul_zero_of_nat = lam(
        "m",
        nat(),
        apps(
            g("GNat_rec"),
            &[
                lam(
                    "m",
                    nat(),
                    eq_gint(apps(g("gimul"), &[gof(var("m")), zero()]), zero()),
                ),
                refl_gint(zero()),
                lam(
                    "k",
                    nat(),
                    lam(
                        "ih",
                        eq_gint(apps(g("gimul"), &[gof(var("k")), zero()]), zero()),
                        refl_gint(zero()),
                    ),
                ),
                var("m"),
            ],
        ),
    );
    let mul_zero_tm = lam(
        "a",
        gint(),
        apps(
            g("GInt_rec"),
            &[
                lam(
                    "a",
                    gint(),
                    eq_gint(apps(g("gimul"), &[var("a"), zero()]), zero()),
                ),
                mul_zero_of_nat,
                lam("m", nat(), refl_gint(zero())),
                var("a"),
            ],
        ),
    );
    let got = infer_type(&ctx, &mul_zero_tm)
        .unwrap_or_else(|e| panic!("the `mul_zero` induction must check: {e:?}"));
    assert!(
        logicaffeine_kernel::is_subtype(&ctx, &got, &mul_zero_ty),
        "`mul_zero` must be proved, not assumed: inferred {got}"
    );
    ctx.add_definition("gmul_zero".to_string(), mul_zero_ty, mul_zero_tm);
    assert_eq!(
        double_check(&ctx, &g("gmul_zero")),
        DoubleCheck::Agreed,
        "both kernels must certify the `mul_zero` proof, got {:?}",
        double_check(&ctx, &g("gmul_zero"))
    );

    // THE NEGATIVE, and it is the one that matters: having constructors must not make a FALSE
    // law provable. `mul_one` is NOT free — `gnmul m 1` ⤳ `gnadd (gnmul m 0) m` ⤳ `gnadd 0 m`,
    // which does not reduce for a variable `m` (Lean's `Nat.add` recurses on its SECOND
    // argument), so it wants `Nat.zero_add` by induction. Asserting the `refl` shape FAILS,
    // which is exactly how a genuine proof obligation is supposed to behave.
    let mul_one_by_refl = lam(
        "a",
        gint(),
        apps(
            g("GInt_rec"),
            &[
                lam(
                    "a",
                    gint(),
                    eq_gint(apps(g("gimul"), &[var("a"), one()]), var("a")),
                ),
                lam("m", nat(), refl_gint(gof(var("m")))),
                lam("m", nat(), refl_gint(gneg_succ(var("m")))),
                var("a"),
            ],
        ),
    );
    assert!(
        infer_type(&ctx, &mul_one_by_refl).is_err(),
        "`mul_one` must NOT fall out of ι — it needs `Nat.zero_add`. A kernel that accepted \
         this `refl` would be deciding an equation it has not computed"
    );

    // …and neither may a FALSE equation sneak through: `a + 1 = a` is refutable, and the same
    // induction shape must be refused.
    let false_law = lam(
        "a",
        gint(),
        apps(
            g("GInt_rec"),
            &[
                lam(
                    "a",
                    gint(),
                    eq_gint(apps(g("giadd"), &[var("a"), one()]), var("a")),
                ),
                lam("m", nat(), refl_gint(gof(var("m")))),
                lam("m", nat(), refl_gint(gneg_succ(var("m")))),
                var("a"),
            ],
        ),
    );
    assert!(
        infer_type(&ctx, &false_law).is_err(),
        "`a + 1 = a` is FALSE and must be refused — constructors buy proofs, not permission"
    );
}

#[test]
fn the_inductive_int_equations_hold_for_a_free_variable_not_only_for_closed_values() {
    use logicaffeine_kernel::{defeq_for_test, normalize};

    // Computing on CLOSED values is the easy half — a definition that only ever reduces when
    // both arguments are numerals is exactly what the prelude's literal fast path already does,
    // and it proves no law. A law is a statement about a FREE variable, so these are the
    // reductions every proof below actually rests on, pinned one at a time. When `add_zero`
    // fails to check, this is the test that says which rung gave way.
    let mut ctx = std_ctx();
    with_gint(&mut ctx);
    let open = ctx.extend("m", nat());

    // The reductions that DO fire at a free variable: every one puts a CONSTRUCTOR in the
    // recursion's structural position, which is the whole difference.
    assert_eq!(
        normalize(&open, &apps(g("gnadd"), &[g("Zero"), g("Zero")])),
        g("Zero"),
        "`0 + 0 ⤳ 0`"
    );
    assert_eq!(
        normalize(&open, &apps(g("gnadd"), &[succ(var("m")), g("Zero")])),
        succ(var("m")),
        "`(m+1) + 0 ⤳ m+1` — the first argument is constructor-headed, so the fixpoint unrolls"
    );
    assert_eq!(
        normalize(&open, &apps(g("gnsub"), &[succ(var("m")), g("Zero")])),
        succ(var("m")),
        "`(m+1) - 0 ⤳ m+1`"
    );
    assert_eq!(
        normalize(&open, &apps(g("gsubNatNat"), &[g("Zero"), succ(var("m"))])),
        gneg_succ(var("m")),
        "`subNatNat 0 (m+1) ⤳ negSucc m` — the arm `add_zero`'s negative case needs, and it \
         fires at a FREE `m` because `Succ m` is constructor-headed"
    );
    assert_eq!(
        normalize(
            &open,
            &apps(g("giadd"), &[gneg_succ(var("m")), gof(g("Zero"))])
        ),
        gneg_succ(var("m")),
        "`negSucc m + ofNat 0 ⤳ negSucc m`"
    );
    assert_eq!(
        normalize(
            &open,
            &apps(g("gimul"), &[gneg_succ(var("m")), gof(g("Zero"))])
        ),
        gof(g("Zero")),
        "`negSucc m * ofNat 0 ⤳ ofNat 0` — through `negOfNat`, the `-0 = 0` arm"
    );

    // …and the same facts as CONVERSIONS, which is the form a proof term is checked against.
    assert!(
        defeq_for_test(
            &open,
            &apps(g("giadd"), &[gof(succ(var("m"))), gof(g("Zero"))]),
            &gof(succ(var("m")))
        ),
        "the equation must be decided by `def_eq`, not merely reached by `normalize`"
    );
    assert!(
        defeq_for_test(
            &open,
            &apps(g("giadd"), &[gneg_succ(var("m")), gof(g("Zero"))]),
            &gneg_succ(var("m"))
        ),
        "…in the negative case too"
    );

    // THE BOUNDARY, recorded rather than wished away: at a BARE variable the same equation does
    // NOT reduce, because `reduction.rs` unrolls a `Fix` only on its FIRST argument while
    // `gnadd` — like Lean's `Nat.add` — recurses on its second. `termination.rs::structural_columns`
    // is the guard's own answer (column 0 at position 1), and the reducer does not ask it. In
    // Lean `n + 0 = n` is `rfl`; here it needs a case split. Pinned so that a reducer fix has to
    // confront this line, and so that nobody reads the case splits above as a property of the
    // representation.
    let gnadd_abs = match ctx.get_definition_body("gnadd").expect("gnadd has a body") {
        Term::Fix { body, .. } => body.as_ref().clone(),
        other => panic!("gnadd must be a Fix, got {other}"),
    };
    let columns = logicaffeine_kernel::termination::structural_columns(&ctx, &gnadd_abs);
    assert_eq!(
        columns[0], 1,
        "the guard admitted `gnadd`'s recursion on its SECOND argument, so THAT is where ι is. \
         A reducer keyed on argument 0 is computing a different rule from the one that was \
         checked, and the case splits in the proofs above are the whole visible cost of the \
         difference: got columns {columns:?}"
    );
    assert_eq!(
        columns,
        vec![1usize, 0usize],
        "…and the lexicographic measure's remaining columns follow in positional order"
    );
}

#[test]
fn the_two_kernels_must_agree_about_which_argument_unrolls_a_fixpoint() {
    use logicaffeine_kernel::{double_check, DoubleCheck};

    // BOTH KERNELS, FROM ONE SPECIFICATION — the assertion this campaign has already been
    // burned by seven times. `reduction.rs` unrolls `(fix f. …) a₀ … aₙ` when `a₀` is
    // constructor-headed; `recheck.rs::whnf` unrolls when ANY argument in the spine is. Neither
    // is the specification, which is `termination.rs::structural_columns` — the column the guard
    // actually admitted — and the two rules do not agree on `gnadd m Zero`, where the structural
    // column is argument 1 and argument 0 is a bare variable.
    //
    // The consequence is a term whose typing the two kernels answer differently: the coercion
    // below needs `gnadd m 0 ≡ m`, which the re-checker's rule reaches and the main kernel's
    // does not. `double_check` is the instrument, and it must say `Agreed`.
    let mut ctx = std_ctx();
    with_gint(&mut ctx);
    ctx.add_declaration("gm", nat());

    // The coercion needs `gnadd gm 0 ≡ gm`, which is `rfl` in Lean and which the STRUCTURAL
    // column licenses here too. Whichever way the kernels decide it, they must decide it
    // TOGETHER — this assertion holds today (both refuse, both being keyed on argument 0) and
    // must still hold the day a reducer starts asking `structural_columns`, which is exactly
    // what makes it worth having: it is the tripwire on a one-sided reducer fix.
    let needs_the_conversion = app(
        lam(
            "h",
            apps(
                g("Eq"),
                &[nat(), apps(g("gnadd"), &[g("gm"), g("Zero")]), g("gm")],
            ),
            var("h"),
        ),
        apps(g("refl"), &[nat(), g("gm")]),
    );
    assert_eq!(
        double_check(&ctx, &needs_the_conversion),
        DoubleCheck::Agreed,
        "the two kernels must decide `gnadd gm 0 ≡ gm` the SAME way — a fixpoint ι-rule changed \
         in one kernel and not the other is the shape this campaign has been burned by seven \
         times. Got {:?}",
        double_check(&ctx, &needs_the_conversion)
    );

    // …and the same for the SPINE shape a mutual block produces, where the structural column is
    // again not argument 0.
    let spine_shape = apps(g("gsubNatNat"), &[g("gm"), g("Zero")]);
    assert_eq!(
        double_check(&ctx, &spine_shape),
        DoubleCheck::Agreed,
        "both kernels must agree on `subNatNat gm 0`, got {:?}",
        double_check(&ctx, &spine_shape)
    );
}

/// Read a normalised `GInt` back to an `i64`, or `None` if it is not in canonical form. The
/// inverse of [`gint_of`], and the instrument the fast-path equivalence is measured with.
fn read_gint(t: &Term) -> Option<i64> {
    fn nat_of(t: &Term) -> Option<i64> {
        match t {
            Term::Global(n) if n == "Zero" => Some(0),
            Term::App(f, x) => match f.as_ref() {
                Term::Global(n) if n == "Succ" => nat_of(x).map(|k| k + 1),
                _ => None,
            },
            Term::Lit(Literal::Nat(b)) => b.to_i64(),
            _ => None,
        }
    }
    match t {
        Term::App(f, x) => match f.as_ref() {
            Term::Global(n) if n == "GIntOfNat" => nat_of(x),
            Term::Global(n) if n == "GIntNegSucc" => nat_of(x).map(|k| -k - 1),
            _ => None,
        },
        _ => None,
    }
}

#[test]
fn the_literal_fast_path_and_the_inductive_definition_agree_on_every_small_integer() {
    use logicaffeine_kernel::normalize;

    // THE EQUIVALENCE THE FAST PATH OWES. Lean ships an inductive `Int` AND a kernel-level
    // literal accelerator, and the accelerator is only legitimate because it computes the SAME
    // function the inductive definition does. LOGOS today has the accelerator and not the
    // inductive — `reduction.rs::try_primitive_reduce` fires on two `Lit`s and there is nothing
    // to compare it against, so "the same function" is not a claim anyone can check.
    //
    // With constructors there is something to compare against, and this is the comparison:
    // exhaustively over a square of small integers, the prelude's accelerated `add`/`sub`/`mul`
    // on `Literal::Int` must agree with the structural recursion on `ofNat`/`negSucc`. Anything
    // the accelerator gets wrong — a sign, a `-0`, an off-by-one in `negSucc` — shows up here
    // and nowhere else.
    let mut ctx = std_ctx();
    with_gint(&mut ctx);
    let lit_int = |n: i64| Term::Lit(Literal::Int(n));
    let as_lit = |t: &Term| match t {
        Term::Lit(Literal::Int(n)) => Some(*n),
        _ => None,
    };

    let mut compared = 0usize;
    for a in -4i64..=4 {
        for b in -4i64..=4 {
            let inductive = normalize(&ctx, &apps(g("giadd"), &[gint_of(a), gint_of(b)]));
            let structural = read_gint(&inductive)
                .unwrap_or_else(|| panic!("giadd {a} {b} left a non-canonical {inductive}"));
            let accelerated = normalize(&ctx, &apps(g("add"), &[lit_int(a), lit_int(b)]));
            assert_eq!(
                as_lit(&accelerated),
                Some(structural),
                "the accelerated `add {a} {b}` must equal the structural one ({structural})"
            );

            let inductive = normalize(&ctx, &apps(g("gimul"), &[gint_of(a), gint_of(b)]));
            let structural = read_gint(&inductive)
                .unwrap_or_else(|| panic!("gimul {a} {b} left a non-canonical {inductive}"));
            let accelerated = normalize(&ctx, &apps(g("mul"), &[lit_int(a), lit_int(b)]));
            assert_eq!(
                as_lit(&accelerated),
                Some(structural),
                "the accelerated `mul {a} {b}` must equal the structural one ({structural})"
            );

            let inductive = normalize(&ctx, &apps(g("gisub"), &[gint_of(a), gint_of(b)]));
            let structural = read_gint(&inductive)
                .unwrap_or_else(|| panic!("gisub {a} {b} left a non-canonical {inductive}"));
            let accelerated = normalize(&ctx, &apps(g("sub"), &[lit_int(a), lit_int(b)]));
            assert_eq!(
                as_lit(&accelerated),
                Some(structural),
                "the accelerated `sub {a} {b}` must equal the structural one ({structural})"
            );
            compared += 3;
        }
    }
    assert_eq!(compared, 243, "9 × 9 × three operations, all compared");
}

#[test]
#[ignore = "MEASUREMENT, not a lock: a stopwatch comparing the unary structural recursion \
            against the literal fast path. Timings flap on a loaded box, so it prints rather \
            than asserts — run it on purpose with --ignored."]
fn measure_what_the_literal_fast_path_is_worth_against_the_inductive_definition() {
    use logicaffeine_kernel::normalize;
    use std::time::Instant;

    // WHY THE FAST PATH EXISTS, measured rather than asserted. The structural definition is
    // UNARY: `ofNat n` is `Succ^n Zero`, so adding is linear in the operand and multiplying is
    // quadratic, while the accelerated path is one machine add on an `i64`. Lean makes exactly
    // this trade — `Int` is an inductive AND `Nat`/`Int` literals are GMP-backed in the kernel —
    // and the trade is only sound because the two compute the same function, which
    // `the_literal_fast_path_and_the_inductive_definition_agree_on_every_small_integer` is the
    // check for.
    let mut ctx = std_ctx();
    with_gint(&mut ctx);
    let lit_int = |n: i64| Term::Lit(Literal::Int(n));

    for n in [8i64, 16, 32, 64] {
        let structural = apps(g("giadd"), &[gint_of(n), gint_of(n)]);
        let t0 = Instant::now();
        let out = normalize(&ctx, &structural);
        let structural_ns = t0.elapsed().as_nanos();
        assert_eq!(
            read_gint(&out),
            Some(n + n),
            "the structural answer must still be right"
        );

        let accelerated = apps(g("add"), &[lit_int(n), lit_int(n)]);
        let t1 = Instant::now();
        let _ = normalize(&ctx, &accelerated);
        let accelerated_ns = t1.elapsed().as_nanos();

        println!(
            "n = {n:>3}: structural {structural_ns:>9} ns · accelerated {accelerated_ns:>9} ns \
             · ratio {:>8.1}x",
            structural_ns as f64 / accelerated_ns.max(1) as f64
        );
    }
}

#[test]
fn a_carried_field_that_already_names_the_inductive_being_declared_is_refused() {
    // THE NAME-COLLISION FENCE. A container's constructor is registered long before the nested
    // declaration, so it normally cannot mention the new inductive — unless the author picks a
    // name that is already taken. Then a field the mirror reads as CLOSED would in fact carry an
    // occurrence of the very type being specialised, into the block, unspecialised. The
    // occurrence check is cheap, fail-closed and asked before anything is carried.
    let mut ctx = std_ctx();
    // `Marked` is an ordinary declared type… which the nested declaration then re-uses as ITS
    // name, so `mcons`'s `Marked` field is an occurrence of the inductive under construction.
    ctx.add_inductive("Marked", sort(0));
    ctx.add_constructor_checked("marked_here", "Marked", g("Marked"))
        .expect("it registers");
    register_container(&mut ctx, "NList", &[g("Marked")]);

    let out = ctx.add_nested_inductive(&logicaffeine_kernel::NestedDecl {
        name: "Marked".to_string(),
        sort: sort(0),
        constructors: vec![(
            "marked_node".to_string(),
            arrow(app(g("NList"), g("Marked")), g("Marked")),
        )],
    });
    let msg = format!(
        "{:?}",
        out.expect_err("a carried occurrence of the declared name is refused")
    );
    assert!(
        msg.contains("Marked") && msg.contains("NList_cons"),
        "the refusal must name the collision and the constructor carrying it, got {msg}"
    );
}

#[test]
fn a_prop_sorted_carried_field_rides_through_and_the_block_still_checks() {
    use logicaffeine_kernel::{double_check, DoubleCheck};

    // THE BOUNDARY BELOW the universe fence: `True : Prop` is a perfectly good carried field for
    // a `Type 0` sibling, because `Prop ≤ Type 0`. The fence is `check_constructor_universes`'s
    // cumulative order, not an equality, and refusing everything that is not literally `Type 0`
    // would lose every proof-carrying container.
    let mut ctx = std_ctx();
    register_container(&mut ctx, "WList", &[g("True")]);
    let info = ctx
        .add_nested_inductive(&nested_through("WTree", "WList"))
        .expect("a `Prop`-sorted carried field fits a `Type 0` sibling by cumulativity");
    let sib = info.siblings[0].clone();
    let ctors: Vec<(String, Term)> = ctx
        .get_constructors(&sib)
        .iter()
        .map(|(n, t)| (n.to_string(), (*t).clone()))
        .collect();
    assert_eq!(
        ctors[1].1,
        arrow(g("True"), arrow(g("WTree"), arrow(g(&sib), g(&sib)))),
        "the proof field rides through in place"
    );
    for name in [&info.isos[0].to_generic, &info.isos[0].from_generic] {
        assert_eq!(
            double_check(&ctx, &g(name)),
            DoubleCheck::Agreed,
            "both kernels must certify iso {name}, got {:?}",
            double_check(&ctx, &g(name))
        );
    }
}

// =======================================================================================
// WAVE 2 — DECLARATION-TIME AUXILIARIES (T03-018 / T04-016), AND THE PROVENANCE THAT
// MAKES THEM SAFE
// =======================================================================================

#[test]
fn the_auxiliary_umbrella_offers_every_constant_lean_generates_at_a_declaration() {
    use logicaffeine_kernel::recursor::derive_auxiliaries;

    // The CAPABILITY half, and it is cheap: `derive_auxiliaries` already builds every constant
    // T03-018 and T04-016 name — the eliminator, the disjointness pair, per-constructor
    // injectivity, the course-of-values package and the decision procedure. What is missing is
    // only the WIRING: `Context::add_constructor_checked` and `interface/repl.rs`'s `Inductive`
    // handler register `{I}_rec` and stop. Pinned here so the wiring is a one-line change to
    // somebody else's file rather than a re-derivation.
    let mut ctx = std_ctx();
    ctx.add_inductive("Tally", sort(0));
    ctx.add_constructor_checked("Nought", "Tally", g("Tally"))
        .expect("Nought registers");
    ctx.add_constructor_checked("Tick", "Tally", arrow(g("Tally"), g("Tally")))
        .expect("Tick is strictly positive");

    let names: Vec<String> = derive_auxiliaries(&ctx, "Tally")
        .into_iter()
        .map(|a| a.name)
        .collect();
    for want in [
        "Tally_rec",
        "Tally_noConfusionType",
        "Tally_noConfusion",
        "Tick_inj",
        "Tally_below",
        "Tally_brecOn",
        "decEqTally",
    ] {
        assert!(
            names.contains(&want.to_string()),
            "`{want}` must be offered, got {names:?}"
        );
    }
    // A nullary constructor contributes no injectivity lemma — there is nothing to inject.
    assert!(
        !names.contains(&"Nought_inj".to_string()),
        "a nullary constructor has none: {names:?}"
    );
}

#[test]
fn declaration_time_synthesis_replaces_its_own_generation_and_never_a_hand_written_one() {
    // THE PROVENANCE PROPERTY, which is what blocks declaration-time synthesis from being turned
    // on. `add_constructor` has no "declaration finished" signal, so eager synthesis runs after
    // EVERY constructor — and the auxiliaries derived after the first describe an inductive that
    // is not finished yet. The umbrella's rule used to be a blanket "never displace a name
    // already in scope", which protects the prelude's hand-written `decEqNat` and, once
    // auxiliaries are registered at the declaration, freezes a ONE-constructor `Tally_rec` in
    // front of a two-constructor `Tally`. Now the rule is keyed on PROVENANCE: replace what this
    // umbrella generated, never what somebody wrote.
    //
    // This test seals the whole standard prelude once, which is the measured cost of eager
    // synthesis and the reason it is a switch rather than a policy — see `Context::auto_aux`.
    let mut ctx = std_ctx();
    assert!(
        !ctx.unsealed_inductives().is_empty(),
        "the prelude declares inductives whose auxiliaries are owed — that is the premise"
    );
    let hand_written = ctx
        .get_definition_body("decEqNat")
        .cloned()
        .expect("the prelude's hand-written decision procedure is the thing being protected");

    ctx.add_inductive("Tally", sort(0));
    ctx.add_constructor_checked("Nought", "Tally", g("Tally"))
        .expect("Nought registers");
    ctx.seal();
    assert!(
        ctx.get_global("Tally_rec").is_some(),
        "one constructor, one eliminator"
    );
    assert!(
        ctx.is_derived_aux("Tally_rec"),
        "and it is recorded as OURS"
    );
    assert!(
        !ctx.is_derived_aux("decEqNat"),
        "the prelude's hand-written constant is not ours and must never be claimed"
    );

    // The declaration is not finished: a second constructor arrives.
    ctx.add_constructor_checked("Tick", "Tally", arrow(g("Tally"), g("Tally")))
        .expect("Tick is strictly positive");
    ctx.seal();

    // The eliminator must now cover BOTH constructors. Under the blanket rule it would still be
    // the one-constructor one — a recursor that cannot eliminate a `Tick`, silently.
    let rec_ty = ctx
        .get_global("Tally_rec")
        .expect("the eliminator survives the second pass")
        .clone();
    let mut minors = 0usize;
    let mut cur = &rec_ty;
    while let Term::Pi { body_type, .. } = cur {
        minors += 1;
        cur = body_type;
    }
    assert_eq!(
        minors, 4,
        "motive + two minor premises + the major premise: the eliminator must be REGENERATED \
         for the finished inductive, got {rec_ty}"
    );
    assert!(
        ctx.get_global("Tick_inj").is_some(),
        "…and the second constructor's injectivity lemma must appear with it"
    );

    // T04-016's two constants, at the declaration, with no caller asking for them.
    for want in ["Tally_below", "Tally_brecOn"] {
        assert!(
            ctx.get_global(want).is_some(),
            "`{want}` must exist once the declaration is sealed — course-of-values recursion is \
             what every `| n+2 =>` definition compiles to"
        );
    }
    // T03-018's, likewise.
    assert!(
        ctx.get_global("Tally_noConfusion").is_some(),
        "constructor disjointness must come with the declaration"
    );

    // And the hand-written constant is untouched, byte for byte.
    assert_eq!(
        ctx.get_definition_body("decEqNat"),
        Some(&hand_written),
        "two seals must not have swapped the prelude's `decEqNat` for a same-intent, \
         different term"
    );
}

#[test]
#[ignore = "MEASUREMENT, not a lock: a stopwatch on what declaration-time auxiliary synthesis \
            actually costs over the standard prelude. Timings flap on a loaded box, so it prints \
            rather than asserts — run it on purpose with --ignored."]
fn measure_what_sealing_the_prelude_costs() {
    use std::time::Instant;

    // The `auto_aux` switch is OFF by default for a MEASURED reason, and the measurement is
    // quoted in `Context::auto_aux`'s own note: "0.18 s with synthesis QUEUED, 17 s synthesising
    // once per INDUCTIVE". That number is what keeps T03-018 and T04-016 closed, so it is worth
    // re-taking rather than inheriting — a stale cost argument is how a capability stays out of
    // reach after the cost has gone.
    let t0 = Instant::now();
    let ctx = std_ctx();
    let build_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let owed = ctx.unsealed_inductives().len();

    let mut sealed = ctx.clone();
    let t1 = Instant::now();
    sealed.seal();
    let seal_ms = t1.elapsed().as_secs_f64() * 1000.0;

    println!(
        "prelude build {build_ms:>8.1} ms · seal {seal_ms:>8.1} ms · {owed} inductive(s) owed \
         · {} auxiliaries registered",
        ctx.unsealed_inductives()
            .iter()
            .filter_map(|i| sealed.derived_aux_names(i).map(<[String]>::len))
            .sum::<usize>()
    );
    assert!(
        sealed.unsealed_inductives().is_empty(),
        "a seal must leave nothing owed"
    );
}

// =======================================================================================
// WAVE 3 — THE δ-SHAPED HOLE IN THE PARADOX FENCE
// =======================================================================================
//
// `positivity.rs` decided strict positivity by looking at the constructor type AS WRITTEN.
// "Does the inductive occur here" was answered by comparing NAMES, so an occurrence behind a
// transparent definition was no occurrence at all:
//
//     Inductive GBad : Type 0
//     Definition GNegBad : Prop := GBad -> False
//     Constructor gbadIn : GNegBad -> GBad
//
// `GNegBad` is `GBad → False` up to δ, so `gbadIn` is Russell's constructor and `False` follows
// with no fixpoint, no recursor and no axiom — one `Term::Match` does it. Nothing downstream can
// catch it, because a constructor that is in the context is in the context.
//
// [`check_positivity_resolved`] is the fence with references RESOLVED. The tests below are the
// negative (the alias must be refused), the boundaries either side (an alias for a LEGITIMATE
// recursive occurrence must still be admitted, and an alias that has nothing to do with the
// block must cost nothing), and a differential pinning that the blind fence's answers are
// unchanged everywhere no reference hides the inductive.

/// `Prop`-sorted `Not p`, spelled through the prelude's own `False`.
fn g_neg(p: Term) -> Term {
    arrow(p, g("False"))
}

/// Register `GBad`, `GNegBad := GBad → False`, and return the constructor type
/// `GNegBad → GBad` — Russell's constructor, spelled so that the string "GBad" never appears
/// in a domain.
fn with_aliased_paradox(ctx: &mut Context) -> Term {
    ctx.add_inductive("GBad", sort(0));
    ctx.add_definition(
        "GNegBad".to_string(),
        Term::Sort(Universe::Prop),
        g_neg(g("GBad")),
    );
    arrow(g("GNegBad"), g("GBad"))
}

#[test]
fn a_definitional_alias_cannot_smuggle_a_negative_occurrence_past_the_paradox_fence() {
    use logicaffeine_kernel::double_check;

    // THE DOOR, end to end. `add_constructor_checked` is the gate every checked declaration
    // goes through, and it is the only gate: a constructor it admits is in the context, and
    // `Term::Match` alone turns this one into a closed proof of `False`.
    let mut ctx = std_ctx();
    let ctor_ty = with_aliased_paradox(&mut ctx);
    let admitted = ctx.add_constructor_checked("gbadIn", "GBad", ctor_ty.clone());

    if admitted.is_ok() {
        // …and here is what it costs, computed rather than asserted about: `False` is inhabited.
        let un = lam(
            "x",
            g("GBad"),
            match_(
                var("x"),
                lam("_", g("GBad"), g("False")),
                vec![lam(
                    "f",
                    g("GNegBad"),
                    app(var("f"), app(g("gbadIn"), var("f"))),
                )],
            ),
        );
        let omega = app(un.clone(), app(g("gbadIn"), un.clone()));
        let inferred = infer_type(&ctx, &omega);
        assert!(
            !matches!(&inferred, Ok(t) if *t == g("False")),
            "RUSSELL: `gbadIn : GNegBad → GBad` was admitted and `False` is now inhabited by a \
             term with no fixpoint, no recursor and no axiom in it — {inferred:?}, both kernels \
             {:?}",
            double_check(&ctx, &omega)
        );
    }

    assert!(
        admitted.is_err(),
        "`GNegBad` is `GBad → False` up to δ, so `gbadIn` places `GBad` in a DOMAIN and must be \
         refused exactly as the literal spelling `(GBad → False) → GBad` is.\n\
         \n\
         THE FIX IS TWO LINES, IN `context.rs`, WHICH THIS LANE DOES NOT OWN:\n\
         * `Context::add_constructor_checked` — `crate::positivity::check_positivity(inductive, \
         name, &ty)?` becomes `crate::positivity::check_positivity_resolved(self, inductive, \
         name, &ty)?`\n\
         * `Context::add_mutual_inductives` — \
         `crate::positivity::check_positivity_mutual(&names, cname, cty)?` becomes \
         `crate::positivity::check_positivity_mutual_resolved(self, &names, cname, cty)?`\n\
         \n\
         Both new entry points are in `positivity.rs` and are locked by \
         `the_resolved_fence_refuses_the_alias_and_still_admits_every_legitimate_shape` and by a \
         census over every prelude constructor. `inductive_compile.rs::compile_nested` already \
         calls the mutual one, so the nested door is closed; this is the plain door."
    );
    let msg = format!("{:?}", admitted.unwrap_err());
    assert!(
        msg.contains("GBad") && msg.to_lowercase().contains("negative"),
        "the refusal must name the inductive and the hazard: {msg}"
    );
}

#[test]
fn the_resolved_fence_refuses_the_alias_and_still_admits_every_legitimate_shape() {
    use logicaffeine_kernel::positivity::{
        check_positivity, check_positivity_mutual_resolved, check_positivity_resolved,
    };

    let mut ctx = std_ctx();
    let paradox = with_aliased_paradox(&mut ctx);

    // THE NEGATIVE — and the premise, stated so the test cannot pass for the wrong reason: the
    // BLIND fence accepts it, which is the defect.
    assert!(
        check_positivity("GBad", "gbadIn", &paradox).is_ok(),
        "the premise of this whole section: the name-comparing fence sees no occurrence"
    );
    let refused = check_positivity_resolved(&ctx, "GBad", "gbadIn", &paradox);
    assert!(
        refused.is_err(),
        "resolving `GNegBad` to `GBad → False` puts `GBad` in a domain: {refused:?}"
    );

    // THE SAME HAZARD ONE LEVEL DEEPER — a chain of aliases must not out-run the resolution.
    ctx.add_definition(
        "GNegBad2".to_string(),
        Term::Sort(Universe::Prop),
        g("GNegBad"),
    );
    assert!(
        check_positivity_resolved(&ctx, "GBad", "gbadIn2", &arrow(g("GNegBad2"), g("GBad")))
            .is_err(),
        "an alias for an alias is still the same domain occurrence"
    );

    // …AND UNDER A λ, which is how a paradox is written parametrically: `GNegOf X := X → False`
    // applied to the inductive is `GBad → False` after one β.
    ctx.add_definition(
        "GNegOf".to_string(),
        arrow(sort(0), Term::Sort(Universe::Prop)),
        lam("X", sort(0), g_neg(var("X"))),
    );
    assert!(
        check_positivity_resolved(
            &ctx,
            "GBad",
            "gbadIn3",
            &arrow(app(g("GNegOf"), g("GBad")), g("GBad"))
        )
        .is_err(),
        "`GNegOf GBad` is `GBad → False`; the literal occurrence in the argument is caught by \
         the blind fence too, so this one must stay refused either way"
    );

    // THE BOUNDARY, ONE RUNG EITHER SIDE.
    //
    // (a) An alias for a LEGITIMATE recursive occurrence must still be ADMITTED, or the fence
    //     has bought soundness by refusing ordinary code. `GSelf := GList` in a domain is the
    //     recursive argument of `gcons`.
    let mut ok_ctx = std_ctx();
    ok_ctx.add_inductive("GList", sort(0));
    ok_ctx.add_definition("GSelf".to_string(), sort(0), g("GList"));
    assert!(
        check_positivity_resolved(
            &ok_ctx,
            "GList",
            "gcons",
            &arrow(nat(), arrow(g("GSelf"), g("GList")))
        )
        .is_ok(),
        "an alias for the inductive itself is a recursive argument, not a negative occurrence"
    );

    // (b) A FUNCTIONAL alias — `GTo := λX. Nat → X` — is strictly positive at the inductive,
    //     the `Acc`/rose-tree shape, and must be admitted.
    ok_ctx.add_definition(
        "GTo".to_string(),
        arrow(sort(0), sort(0)),
        lam("X", sort(0), arrow(nat(), var("X"))),
    );
    assert!(
        check_positivity_resolved(
            &ok_ctx,
            "GList",
            "gnode",
            &arrow(app(g("GTo"), g("GList")), g("GList"))
        )
        .is_ok(),
        "`Nat → GList` is a strictly-positive functional field however it is spelled"
    );

    // (c) An alias that has nothing to do with the block must cost nothing and change nothing.
    ok_ctx.add_definition("GPair".to_string(), sort(0), arrow(nat(), nat()));
    assert!(
        check_positivity_resolved(&ok_ctx, "GList", "gplain", &arrow(g("GPair"), g("GList")))
            .is_ok(),
        "an unrelated definition in a domain is an unrelated definition"
    );

    // (d) THE MUTUAL DOOR, same hazard: a SIBLING hidden behind an alias is a cross-block
    //     paradox and must be refused with exactly the same reasoning.
    let mut mut_ctx = std_ctx();
    mut_ctx.add_inductive("GEven", sort(0));
    mut_ctx.add_inductive("GOdd", sort(0));
    mut_ctx.add_definition(
        "GNegEven".to_string(),
        Term::Sort(Universe::Prop),
        g_neg(g("GEven")),
    );
    assert!(
        check_positivity_mutual_resolved(
            &mut_ctx,
            &["GEven", "GOdd"],
            "godd",
            &arrow(g("GNegEven"), g("GOdd"))
        )
        .is_err(),
        "a sibling in a domain is the cross-block paradox, alias or not"
    );
    assert!(
        check_positivity_mutual_resolved(
            &mut_ctx,
            &["GEven", "GOdd"],
            "godd_ok",
            &arrow(g("GEven"), g("GOdd"))
        )
        .is_ok(),
        "…while a sibling in a RECURSIVE position is the whole point of a mutual block"
    );
}

#[test]
fn the_blind_and_the_resolved_fence_agree_wherever_no_reference_hides_the_inductive() {
    use logicaffeine_kernel::positivity::{check_positivity, check_positivity_resolved};

    // THE DIFFERENTIAL. Resolving references may only change the answer for a type that
    // MENTIONS a transparent definition whose unfolding reaches the block. Everywhere else the
    // two fences must agree verdict for verdict — otherwise this widening has quietly become a
    // different positivity rule, and the prelude's own inductives would be the first casualties.
    let mut ctx = std_ctx();
    ctx.add_inductive("GT", sort(0));
    ctx.add_definition("GAlias".to_string(), sort(0), arrow(nat(), nat()));

    let shapes: Vec<(&str, Term)> = vec![
        ("nullary", g("GT")),
        ("direct recursive", arrow(g("GT"), g("GT"))),
        (
            "plain field then recursive",
            arrow(nat(), arrow(g("GT"), g("GT"))),
        ),
        ("functional positive", arrow(arrow(nat(), g("GT")), g("GT"))),
        ("negative", arrow(g_neg(g("GT")), g("GT"))),
        (
            "doubly negative",
            arrow(arrow(arrow(g("GT"), nat()), nat()), g("GT")),
        ),
        ("unrelated alias", arrow(g("GAlias"), g("GT"))),
        ("indexed result", pi("n", nat(), app(g("GT"), var("n")))),
        // An APPLICATION in RESULT position whose head is not a block member and whose
        // ARGUMENT is: the shape that separates "resolve a reference" from "refuse anything
        // applied to the inductive". `List` has no body, so there is nothing to resolve and
        // both fences must walk it structurally.
        (
            "opaque head over the inductive",
            pi("x", nat(), apps(g("List"), &[g("GT")])),
        ),
        (
            "opaque head, nested",
            arrow(nat(), apps(g("List"), &[apps(g("List"), &[g("GT")])])),
        ),
        (
            "recursive under a telescope",
            arrow(pi("n", nat(), g("GT")), g("GT")),
        ),
        ("sort field", arrow(sort(0), g("GT"))),
        ("let-bound field", arrow(nat(), arrow(nat(), g("GT")))),
    ];
    for (what, ty) in &shapes {
        let blind = check_positivity("GT", "gc", ty).is_ok();
        let resolved = check_positivity_resolved(&ctx, "GT", "gc", ty).is_ok();
        assert_eq!(
            blind, resolved,
            "the two fences must answer the same for `{what}` — no reference in it hides `GT`"
        );
    }

    // A NESTED occurrence is the one place the context legitimately changes the verdict, and
    // it changes it in the safe direction only. `List GT` in a domain hides `GT` behind a
    // reference to a REGISTERED CONTAINER; the resolved fence reads `List`'s constructors, finds
    // it strictly positive in its parameter, and admits the nesting as Lean does. The blind
    // fence cannot read them, so it refuses — as an INCONCLUSIVE certification failure, never as
    // a claimed paradox.
    let nested = arrow(apps(g("List"), &[g("GT")]), g("GT"));
    assert!(
        check_positivity_resolved(&ctx, "GT", "gc", &nested).is_ok(),
        "`List GT → GT` nests through a strictly-positive registered container"
    );
    let blind = check_positivity("GT", "gc", &nested);
    assert!(
        matches!(
            blind,
            Err(logicaffeine_kernel::KernelError::CertificationError(_))
        ),
        "without a context the container is unreadable: an inconclusive refusal, got {blind:?}"
    );
    // …and a container that is NOT positive in its parameter is refused by the resolved fence
    // too: reading the container's meaning is a check, never a pass.
    ctx.add_inductive("GNeg", pi("A", sort(0), sort(0)));
    ctx.add_constructor(
        "gneg_mk",
        "GNeg",
        pi(
            "A",
            sort(0),
            arrow(arrow(var("A"), nat()), app(g("GNeg"), var("A"))),
        ),
    );
    ctx.set_inductive_params("GNeg", 1);
    let through_negative = arrow(app(g("GNeg"), g("GT")), g("GT"));
    assert!(
        matches!(
            check_positivity_resolved(&ctx, "GT", "gc", &through_negative),
            Err(logicaffeine_kernel::KernelError::PositivityViolation { .. })
        ),
        "`GNeg GT` puts `GT` in a domain once `GNeg` is read: {:?}",
        check_positivity_resolved(&ctx, "GT", "gc", &through_negative)
    );

    // …and the prelude's OWN constructors are the census that matters: every one of them must
    // still be admitted by the resolved fence, or the widening has broken the standard library.
    let base = std_ctx();
    let mut checked = 0usize;
    let inds: Vec<String> = base.iter_inductives().map(|(n, _)| n.to_string()).collect();
    for ind in &inds {
        for (cname, cty) in base.get_constructors(ind) {
            let r = check_positivity_resolved(&base, ind, cname, cty);
            assert!(
                r.is_ok(),
                "the prelude's `{cname} : {ind}` must stay strictly positive under δ: {r:?}"
            );
            checked += 1;
        }
    }
    assert!(
        checked > 40,
        "the census must actually cover the prelude, saw {checked}"
    );
}

// =======================================================================================
// WAVE 3 — FIVE OF THE SEVENTEEN, AND THE `Nat` LEMMAS UNDER THEM
// =======================================================================================
//
// Wave 2 built `GInt` and proved three of `tcb_ledger_lock.rs`'s seventeen assumed ring/order
// laws — the three that fall out of ι alone. This section proves the next two, `add_comm` and
// `mul_one`, which do NOT fall out of ι: they need a small `Nat` library underneath, and that
// library needs congruence, and congruence is itself a theorem here rather than an axiom (J,
// applied at a motive that transports an equation along `f`).
//
// Every constant below is a DEFINITION whose term the kernel checks and the re-checker agrees
// with. Nothing is assumed. When `prelude.rs` declares `Int` with Lean's two constructors,
// these transcribe verbatim — which is the whole point of building them on the real kernel
// under `G`-prefixed names rather than describing them.
//
// The `Nat` lemmas are also the record of where the TWO KERNELS stand on the fixpoint ι-rule.
// `gnadd` recurses on its SECOND argument (Lean's `Nat.add`). `reduction.rs` now asks
// `termination::structural_columns` which column that is, so the main kernel reduces
// `gnadd m 0` to `m` for a free `m`, exactly as Lean does. `recheck.rs::whnf` still keys on
// argument 0, so the SECOND kernel does not — and every lemma here is `admit`ted, which
// demands `DoubleCheck::Agreed`. So each one is written as an induction that puts a
// constructor in argument 0 and is therefore a theorem BOTH kernels certify: `rfl` alone would
// pass the main kernel and be rejected by the re-checker. Three of them are `rfl` in Lean and
// an induction here, and that difference is `recheck.rs` waiting to ask the same question
// `reduction.rs` now asks.

/// `Eq A x y` and `refl A x` at an arbitrary carrier.
fn eq_at(a: Term, x: Term, y: Term) -> Term {
    apps(g("Eq"), &[a, x, y])
}
fn refl_at(a: Term, x: Term) -> Term {
    apps(g("refl"), &[a, x])
}
/// `gcongr A B f x y h : Eq B (f x) (f y)`
fn gcongr(a: Term, b: Term, f: Term, x: Term, y: Term, h: Term) -> Term {
    apps(g("gcongr"), &[a, b, f, x, y, h])
}
/// `Eq_sym A x y h : Eq A y x`
fn gsym(a: Term, x: Term, y: Term, h: Term) -> Term {
    apps(g("Eq_sym"), &[a, x, y, h])
}
/// `Eq_trans A x y z hxy hyz : Eq A x z`
fn gtrans(a: Term, x: Term, y: Term, z: Term, hxy: Term, hyz: Term) -> Term {
    apps(g("Eq_trans"), &[a, x, y, z, hxy, hyz])
}
/// `GNat_rec motive base step n` / `GInt_rec motive ofNat_case negSucc_case a`.
fn nat_ind(motive: Term, base: Term, step: Term, n: Term) -> Term {
    apps(g("GNat_rec"), &[motive, base, step, n])
}
fn gint_ind(motive: Term, of_nat: Term, neg_succ: Term, a: Term) -> Term {
    apps(g("GInt_rec"), &[motive, of_nat, neg_succ, a])
}
/// `ofNat 1`, the multiplicative unit.
fn gone() -> Term {
    gof(succ(g("Zero")))
}

/// Register `GNat_rec`, `GInt_rec`, `gcongr` and the `Nat` lemmas the `GInt` laws stand on —
/// each one kernel-checked against the type claimed for it before it is admitted, so a lemma
/// that does not actually hold cannot become the premise of a later one.
fn with_gint_laws(ctx: &mut Context) {
    use logicaffeine_kernel::recursor::derive_recursor;

    let (nat_rec_ty, nat_rec_tm) = derive_recursor(ctx, "Nat").expect("Nat's eliminator derives");
    ctx.add_definition("GNat_rec".to_string(), nat_rec_ty, nat_rec_tm);
    let (int_rec_ty, int_rec_tm) = derive_recursor(ctx, "GInt").expect("GInt's eliminator derives");
    ctx.add_definition("GInt_rec".to_string(), int_rec_ty, int_rec_tm);

    // ---- congruence, DERIVED from J ---------------------------------------------------
    // `congrArg : Π(A B : Type 0)(f : A → B)(x y : A). x = y → f x = f y`, the one lemma every
    // induction step below needs. Transport `Eq B (f x) (f _)` along `h`, base `refl B (f x)`.
    let congr_ty = pi(
        "A",
        sort(0),
        pi(
            "B",
            sort(0),
            pi(
                "f",
                arrow(var("A"), var("B")),
                pi(
                    "x",
                    var("A"),
                    pi(
                        "y",
                        var("A"),
                        arrow(
                            eq_at(var("A"), var("x"), var("y")),
                            eq_at(var("B"), app(var("f"), var("x")), app(var("f"), var("y"))),
                        ),
                    ),
                ),
            ),
        ),
    );
    let congr_tm = lam(
        "A",
        sort(0),
        lam(
            "B",
            sort(0),
            lam(
                "f",
                arrow(var("A"), var("B")),
                lam(
                    "x",
                    var("A"),
                    lam(
                        "y",
                        var("A"),
                        lam(
                            "h",
                            eq_at(var("A"), var("x"), var("y")),
                            apps(
                                g("Eq_rec_dep"),
                                &[
                                    var("A"),
                                    var("x"),
                                    lam(
                                        "y2",
                                        var("A"),
                                        lam(
                                            "_",
                                            eq_at(var("A"), var("x"), var("y2")),
                                            eq_at(
                                                var("B"),
                                                app(var("f"), var("x")),
                                                app(var("f"), var("y2")),
                                            ),
                                        ),
                                    ),
                                    refl_at(var("B"), app(var("f"), var("x"))),
                                    var("y"),
                                    var("h"),
                                ],
                            ),
                        ),
                    ),
                ),
            ),
        ),
    );
    admit(ctx, "gcongr", congr_ty, congr_tm);

    let eqn = |a: Term, b: Term| eq_at(nat(), a, b);
    let add = |a: Term, b: Term| apps(g("gnadd"), &[a, b]);
    let mul = |a: Term, b: Term| apps(g("gnmul"), &[a, b]);
    let congr_succ = |x: Term, y: Term, h: Term| gcongr(nat(), nat(), g("Succ"), x, y, h);

    // ---- `gnadd_zero : m + 0 = m` -----------------------------------------------------
    // `rfl` in Lean, and `rfl` in the main kernel now that `reduction.rs` asks which column ι
    // is on. The induction is still what the RE-CHECKER needs — it puts a constructor in
    // argument 0, which is the only shape its `Db::Fix` arm unrolls — and both branches are
    // then `refl`, so this lemma carries no mathematical content: it is the price of the two
    // kernels not yet agreeing.
    admit(
        ctx,
        "gnadd_zero",
        pi("m", nat(), eqn(add(var("m"), g("Zero")), var("m"))),
        lam(
            "m",
            nat(),
            nat_ind(
                lam("mm", nat(), eqn(add(var("mm"), g("Zero")), var("mm"))),
                refl_at(nat(), g("Zero")),
                lam(
                    "k",
                    nat(),
                    lam(
                        "ih",
                        eqn(add(var("k"), g("Zero")), var("k")),
                        refl_at(nat(), succ(var("k"))),
                    ),
                ),
                var("m"),
            ),
        ),
    );

    // ---- `gnzero_add : 0 + n = n` -----------------------------------------------------
    admit(
        ctx,
        "gnzero_add",
        pi("n", nat(), eqn(add(g("Zero"), var("n")), var("n"))),
        lam(
            "n",
            nat(),
            nat_ind(
                lam("nn", nat(), eqn(add(g("Zero"), var("nn")), var("nn"))),
                refl_at(nat(), g("Zero")),
                lam(
                    "j",
                    nat(),
                    lam(
                        "ih",
                        eqn(add(g("Zero"), var("j")), var("j")),
                        congr_succ(add(g("Zero"), var("j")), var("j"), var("ih")),
                    ),
                ),
                var("n"),
            ),
        ),
    );

    // ---- `gnadd_succ_right : m + (n+1) = (m + n) + 1` ---------------------------------
    // Both branches `refl`: the induction is on `m`, again only to unstick argument 0.
    admit(
        ctx,
        "gnadd_succ_right",
        pi(
            "m",
            nat(),
            pi(
                "n",
                nat(),
                eqn(add(var("m"), succ(var("n"))), succ(add(var("m"), var("n")))),
            ),
        ),
        lam(
            "m",
            nat(),
            lam(
                "n",
                nat(),
                nat_ind(
                    lam(
                        "mm",
                        nat(),
                        eqn(
                            add(var("mm"), succ(var("n"))),
                            succ(add(var("mm"), var("n"))),
                        ),
                    ),
                    refl_at(nat(), succ(add(g("Zero"), var("n")))),
                    lam(
                        "k",
                        nat(),
                        lam(
                            "ih",
                            eqn(add(var("k"), succ(var("n"))), succ(add(var("k"), var("n")))),
                            refl_at(nat(), succ(add(succ(var("k")), var("n")))),
                        ),
                    ),
                    var("m"),
                ),
            ),
        ),
    );

    // ---- `gnadd_succ_left : (m+1) + n = (m + n) + 1` ----------------------------------
    admit(
        ctx,
        "gnadd_succ_left",
        pi(
            "m",
            nat(),
            pi(
                "n",
                nat(),
                eqn(add(succ(var("m")), var("n")), succ(add(var("m"), var("n")))),
            ),
        ),
        lam(
            "m",
            nat(),
            lam(
                "n",
                nat(),
                nat_ind(
                    lam(
                        "nn",
                        nat(),
                        eqn(
                            add(succ(var("m")), var("nn")),
                            succ(add(var("m"), var("nn"))),
                        ),
                    ),
                    // `(m+1) + 0` ⤳ `m+1`, while `(m + 0) + 1` is STUCK — `gnadd_zero` is what
                    // closes the gap, and it is needed in the SYMMETRIC direction.
                    gsym(
                        nat(),
                        succ(add(var("m"), g("Zero"))),
                        succ(var("m")),
                        congr_succ(
                            add(var("m"), g("Zero")),
                            var("m"),
                            app(g("gnadd_zero"), var("m")),
                        ),
                    ),
                    lam(
                        "j",
                        nat(),
                        lam(
                            "ih",
                            eqn(add(succ(var("m")), var("j")), succ(add(var("m"), var("j")))),
                            gtrans(
                                nat(),
                                succ(add(succ(var("m")), var("j"))),
                                succ(succ(add(var("m"), var("j")))),
                                succ(add(var("m"), succ(var("j")))),
                                congr_succ(
                                    add(succ(var("m")), var("j")),
                                    succ(add(var("m"), var("j"))),
                                    var("ih"),
                                ),
                                congr_succ(
                                    succ(add(var("m"), var("j"))),
                                    add(var("m"), succ(var("j"))),
                                    gsym(
                                        nat(),
                                        add(var("m"), succ(var("j"))),
                                        succ(add(var("m"), var("j"))),
                                        apps(g("gnadd_succ_right"), &[var("m"), var("j")]),
                                    ),
                                ),
                            ),
                        ),
                    ),
                    var("n"),
                ),
            ),
        ),
    );

    // ---- `gnadd_comm : m + n = n + m` -------------------------------------------------
    admit(
        ctx,
        "gnadd_comm",
        pi(
            "m",
            nat(),
            pi(
                "n",
                nat(),
                eqn(add(var("m"), var("n")), add(var("n"), var("m"))),
            ),
        ),
        lam(
            "m",
            nat(),
            lam(
                "n",
                nat(),
                nat_ind(
                    lam(
                        "mm",
                        nat(),
                        eqn(add(var("mm"), var("n")), add(var("n"), var("mm"))),
                    ),
                    gtrans(
                        nat(),
                        add(g("Zero"), var("n")),
                        var("n"),
                        add(var("n"), g("Zero")),
                        app(g("gnzero_add"), var("n")),
                        gsym(
                            nat(),
                            add(var("n"), g("Zero")),
                            var("n"),
                            app(g("gnadd_zero"), var("n")),
                        ),
                    ),
                    lam(
                        "k",
                        nat(),
                        lam(
                            "ih",
                            eqn(add(var("k"), var("n")), add(var("n"), var("k"))),
                            gtrans(
                                nat(),
                                add(succ(var("k")), var("n")),
                                succ(add(var("n"), var("k"))),
                                add(var("n"), succ(var("k"))),
                                gtrans(
                                    nat(),
                                    add(succ(var("k")), var("n")),
                                    succ(add(var("k"), var("n"))),
                                    succ(add(var("n"), var("k"))),
                                    apps(g("gnadd_succ_left"), &[var("k"), var("n")]),
                                    congr_succ(
                                        add(var("k"), var("n")),
                                        add(var("n"), var("k")),
                                        var("ih"),
                                    ),
                                ),
                                gsym(
                                    nat(),
                                    add(var("n"), succ(var("k"))),
                                    succ(add(var("n"), var("k"))),
                                    apps(g("gnadd_succ_right"), &[var("n"), var("k")]),
                                ),
                            ),
                        ),
                    ),
                    var("m"),
                ),
            ),
        ),
    );

    // ---- `gnmul_zero_right : m * 0 = 0` and `gnmul_one : m * 1 = m` -------------------
    admit(
        ctx,
        "gnmul_zero_right",
        pi("m", nat(), eqn(mul(var("m"), g("Zero")), g("Zero"))),
        lam(
            "m",
            nat(),
            nat_ind(
                lam("mm", nat(), eqn(mul(var("mm"), g("Zero")), g("Zero"))),
                refl_at(nat(), g("Zero")),
                lam(
                    "k",
                    nat(),
                    lam(
                        "ih",
                        eqn(mul(var("k"), g("Zero")), g("Zero")),
                        refl_at(nat(), g("Zero")),
                    ),
                ),
                var("m"),
            ),
        ),
    );
    // ---- `gnadd_assoc : (a + b) + c = a + (b + c)` -------------------------------------
    admit(
        ctx,
        "gnadd_assoc",
        pi(
            "a",
            nat(),
            pi(
                "b",
                nat(),
                pi(
                    "c",
                    nat(),
                    eqn(
                        add(add(var("a"), var("b")), var("c")),
                        add(var("a"), add(var("b"), var("c"))),
                    ),
                ),
            ),
        ),
        lam(
            "a",
            nat(),
            lam(
                "b",
                nat(),
                lam(
                    "c",
                    nat(),
                    nat_ind(
                        lam(
                            "cc",
                            nat(),
                            eqn(
                                add(add(var("a"), var("b")), var("cc")),
                                add(var("a"), add(var("b"), var("cc"))),
                            ),
                        ),
                        refl_at(nat(), add(var("a"), var("b"))),
                        lam(
                            "j",
                            nat(),
                            lam(
                                "ih",
                                eqn(
                                    add(add(var("a"), var("b")), var("j")),
                                    add(var("a"), add(var("b"), var("j"))),
                                ),
                                congr_succ(
                                    add(add(var("a"), var("b")), var("j")),
                                    add(var("a"), add(var("b"), var("j"))),
                                    var("ih"),
                                ),
                            ),
                        ),
                        var("c"),
                    ),
                ),
            ),
        ),
    );

    // ---- `gnmul_zero_left : 0 * n = 0` ------------------------------------------------
    admit(
        ctx,
        "gnmul_zero_left",
        pi("n", nat(), eqn(mul(g("Zero"), var("n")), g("Zero"))),
        lam(
            "n",
            nat(),
            nat_ind(
                lam("nn", nat(), eqn(mul(g("Zero"), var("nn")), g("Zero"))),
                refl_at(nat(), g("Zero")),
                // `0 * (j+1)` ⤳ `0*j + 0` ⤳ `0*j`, so the hypothesis IS the goal.
                lam(
                    "j",
                    nat(),
                    lam("ih", eqn(mul(g("Zero"), var("j")), g("Zero")), var("ih")),
                ),
                var("n"),
            ),
        ),
    );

    // ---- `gnmul_succ_left : (m+1) * n = m*n + n` --------------------------------------
    // The one lemma with real content. Its step is four rewrites: the hypothesis under
    // `_ + m`, associativity out, commutativity of the inner pair, associativity back.
    let add_r = |k: Term| lam("x", nat(), apps(g("gnadd"), &[var("x"), k]));
    let add_l = |k: Term| lam("x", nat(), apps(g("gnadd"), &[k, var("x")]));
    let assoc = |a: Term, b: Term, c: Term| apps(g("gnadd_assoc"), &[a, b, c]);
    let comm = |a: Term, b: Term| apps(g("gnadd_comm"), &[a, b]);
    let p = || mul(var("m"), var("j"));
    admit(
        ctx,
        "gnmul_succ_left",
        pi(
            "m",
            nat(),
            pi(
                "n",
                nat(),
                eqn(
                    mul(succ(var("m")), var("n")),
                    add(mul(var("m"), var("n")), var("n")),
                ),
            ),
        ),
        lam(
            "m",
            nat(),
            lam(
                "n",
                nat(),
                nat_ind(
                    lam(
                        "nn",
                        nat(),
                        eqn(
                            mul(succ(var("m")), var("nn")),
                            add(mul(var("m"), var("nn")), var("nn")),
                        ),
                    ),
                    refl_at(nat(), g("Zero")),
                    lam(
                        "j",
                        nat(),
                        lam(
                            "ih",
                            eqn(
                                mul(succ(var("m")), var("j")),
                                add(mul(var("m"), var("j")), var("j")),
                            ),
                            congr_succ(
                                add(mul(succ(var("m")), var("j")), var("m")),
                                add(add(p(), var("m")), var("j")),
                                gtrans(
                                    nat(),
                                    add(mul(succ(var("m")), var("j")), var("m")),
                                    add(p(), add(var("m"), var("j"))),
                                    add(add(p(), var("m")), var("j")),
                                    gtrans(
                                        nat(),
                                        add(mul(succ(var("m")), var("j")), var("m")),
                                        add(p(), add(var("j"), var("m"))),
                                        add(p(), add(var("m"), var("j"))),
                                        gtrans(
                                            nat(),
                                            add(mul(succ(var("m")), var("j")), var("m")),
                                            add(add(p(), var("j")), var("m")),
                                            add(p(), add(var("j"), var("m"))),
                                            gcongr(
                                                nat(),
                                                nat(),
                                                add_r(var("m")),
                                                mul(succ(var("m")), var("j")),
                                                add(p(), var("j")),
                                                var("ih"),
                                            ),
                                            assoc(p(), var("j"), var("m")),
                                        ),
                                        gcongr(
                                            nat(),
                                            nat(),
                                            add_l(p()),
                                            add(var("j"), var("m")),
                                            add(var("m"), var("j")),
                                            comm(var("j"), var("m")),
                                        ),
                                    ),
                                    gsym(
                                        nat(),
                                        add(add(p(), var("m")), var("j")),
                                        add(p(), add(var("m"), var("j"))),
                                        assoc(p(), var("m"), var("j")),
                                    ),
                                ),
                            ),
                        ),
                    ),
                    var("n"),
                ),
            ),
        ),
    );

    // ---- `gnmul_comm : m * n = n * m` -------------------------------------------------
    admit(
        ctx,
        "gnmul_comm",
        pi(
            "m",
            nat(),
            pi(
                "n",
                nat(),
                eqn(mul(var("m"), var("n")), mul(var("n"), var("m"))),
            ),
        ),
        lam(
            "m",
            nat(),
            lam(
                "n",
                nat(),
                nat_ind(
                    lam(
                        "mm",
                        nat(),
                        eqn(mul(var("mm"), var("n")), mul(var("n"), var("mm"))),
                    ),
                    app(g("gnmul_zero_left"), var("n")),
                    lam(
                        "k",
                        nat(),
                        lam(
                            "ih",
                            eqn(mul(var("k"), var("n")), mul(var("n"), var("k"))),
                            gtrans(
                                nat(),
                                mul(succ(var("k")), var("n")),
                                add(mul(var("k"), var("n")), var("n")),
                                add(mul(var("n"), var("k")), var("n")),
                                apps(g("gnmul_succ_left"), &[var("k"), var("n")]),
                                gcongr(
                                    nat(),
                                    nat(),
                                    add_r(var("n")),
                                    mul(var("k"), var("n")),
                                    mul(var("n"), var("k")),
                                    var("ih"),
                                ),
                            ),
                        ),
                    ),
                    var("m"),
                ),
            ),
        ),
    );

    admit(
        ctx,
        "gnmul_one",
        pi("m", nat(), eqn(mul(var("m"), succ(g("Zero"))), var("m"))),
        lam(
            "m",
            nat(),
            nat_ind(
                lam("mm", nat(), eqn(mul(var("mm"), succ(g("Zero"))), var("mm"))),
                refl_at(nat(), g("Zero")),
                lam(
                    "k",
                    nat(),
                    lam(
                        "ih",
                        eqn(mul(var("k"), succ(g("Zero"))), var("k")),
                        congr_succ(
                            add(g("Zero"), var("k")),
                            var("k"),
                            app(g("gnzero_add"), var("k")),
                        ),
                    ),
                ),
                var("m"),
            ),
        ),
    );
}

/// Kernel-check `term` against `ty`, refuse to register anything that does not check, and
/// require BOTH kernels to certify it — the discipline that makes every lemma above a theorem
/// rather than a hopeful name.
fn admit(ctx: &mut Context, name: &str, ty: Term, term: Term) {
    use logicaffeine_kernel::{double_check, is_subtype, DoubleCheck};

    let got = infer_type(ctx, &term)
        .unwrap_or_else(|e| panic!("`{name}`'s proof term must type-check: {e:?}"));
    assert!(
        is_subtype(ctx, &got, &ty),
        "`{name}` must inhabit the type claimed for it\n  claimed: {ty}\n  inferred: {got}"
    );
    ctx.add_definition(name.to_string(), ty, term);
    match double_check(ctx, &g(name)) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify `{name}`, got {other:?}"),
    }
    // AND IT COSTS THE TRUSTED BASE NOTHING. The whole argument for giving `Int` constructors
    // is that its laws stop being assumptions; a lemma that closed a ledger entry by leaning on
    // a DIFFERENT axiom would have moved the debt rather than paid it. The closure is
    // transitive through every definition the proof cites, and `unresolved` is checked too —
    // a name that resolves nowhere means the proof was minted against a larger base than the
    // one re-checking it.
    let usage = logicaffeine_kernel::axiom_dependencies(ctx, &g(name));
    assert!(
        usage.axioms.is_empty(),
        "`{name}` must rest on NO axiom — the point of the construction: {:?}",
        usage.axioms
    );
    assert!(
        usage.unresolved.is_empty(),
        "`{name}` cites names the context does not have: {:?}",
        usage.unresolved
    );
}

#[test]
fn the_nat_lemmas_under_the_int_ring_laws_are_proved_and_certified_by_both_kernels() {
    let mut ctx = std_ctx();
    with_gint(&mut ctx);
    with_gint_laws(&mut ctx);

    // `admit` already refused anything that did not check. What is asserted here is that the
    // lemmas are DEFINITIONS with bodies — not declarations, not axioms — because a lemma that
    // became an assumption on the way in would have closed the same gap for free.
    for name in [
        "gcongr",
        "gnadd_zero",
        "gnzero_add",
        "gnadd_succ_right",
        "gnadd_succ_left",
        "gnadd_comm",
        "gnadd_assoc",
        "gnmul_zero_right",
        "gnmul_zero_left",
        "gnmul_succ_left",
        "gnmul_comm",
        "gnmul_one",
    ] {
        assert!(
            ctx.is_definition(name),
            "`{name}` must be a definition, not an assumption"
        );
    }

    // …and they COMPUTE where they are supposed to: an instance of a proved commutativity at
    // closed numerals normalises to `refl`-shaped evidence about the same numeral both ways.
    use logicaffeine_kernel::normalize;
    for (a, b) in [(0u32, 0u32), (1, 0), (0, 1), (2, 3), (3, 2), (4, 1)] {
        let lhs = normalize(&ctx, &apps(g("gnadd"), &[gpeano(a), gpeano(b)]));
        let rhs = normalize(&ctx, &apps(g("gnadd"), &[gpeano(b), gpeano(a)]));
        assert_eq!(
            lhs, rhs,
            "`gnadd {a} {b}` and `gnadd {b} {a}` must compute to one value"
        );
        assert_eq!(lhs, gpeano(a + b), "…and it must be {}", a + b);
    }

    // THE NEGATIVE. The toolkit must not make a FALSE `Nat` equation provable: `m + 1 = m` has
    // exactly the same induction shape as `gnadd_zero` and must be refused at the step.
    let bogus = lam(
        "m",
        nat(),
        nat_ind(
            lam(
                "mm",
                nat(),
                eq_at(
                    nat(),
                    apps(g("gnadd"), &[var("mm"), succ(g("Zero"))]),
                    var("mm"),
                ),
            ),
            refl_at(nat(), g("Zero")),
            lam(
                "k",
                nat(),
                lam(
                    "ih",
                    eq_at(
                        nat(),
                        apps(g("gnadd"), &[var("k"), succ(g("Zero"))]),
                        var("k"),
                    ),
                    refl_at(nat(), succ(var("k"))),
                ),
            ),
            var("m"),
        ),
    );
    assert!(
        infer_type(&ctx, &bogus).is_err(),
        "`m + 1 = m` is false and the same shape must not check"
    );
}

#[test]
fn two_more_of_the_seventeen_assumed_ring_laws_are_proved_from_the_constructors() {
    use logicaffeine_kernel::{double_check, is_subtype, DoubleCheck};

    // `add_comm` and `mul_one` — the fourth and fifth of `tcb_ledger_lock.rs`'s seventeen
    // `Definable` ring/order laws to become theorems. Neither falls out of ι: `add_comm` is a
    // 2×2 case split whose diagonal needs `Nat` commutativity and whose off-diagonal is `refl`
    // because `gsubNatNat m (n+1)` is literally the same term both ways; `mul_one` is a case
    // split over `gnmul_one`, once through `ofNat` and once through the `negOfNat` smart
    // constructor.
    let mut ctx = std_ctx();
    with_gint(&mut ctx);
    with_gint_laws(&mut ctx);

    let add = |a: Term, b: Term| apps(g("giadd"), &[a, b]);
    let nadd = |a: Term, b: Term| apps(g("gnadd"), &[a, b]);

    // ---- add_comm : Π(a b : Int). a + b = b + a --------------------------------------
    let add_comm_ty = pi(
        "a",
        gint(),
        pi(
            "b",
            gint(),
            eq_gint(add(var("a"), var("b")), add(var("b"), var("a"))),
        ),
    );
    let neg_succ_succ = lam("x", nat(), gneg_succ(succ(var("x"))));
    let add_comm_tm = lam(
        "a",
        gint(),
        lam(
            "b",
            gint(),
            gint_ind(
                lam(
                    "aa",
                    gint(),
                    eq_gint(add(var("aa"), var("b")), add(var("b"), var("aa"))),
                ),
                // a = ofNat m: split on b.
                lam(
                    "m",
                    nat(),
                    gint_ind(
                        lam(
                            "bb",
                            gint(),
                            eq_gint(add(gof(var("m")), var("bb")), add(var("bb"), gof(var("m")))),
                        ),
                        lam(
                            "n",
                            nat(),
                            gcongr(
                                nat(),
                                gint(),
                                g("GIntOfNat"),
                                nadd(var("m"), var("n")),
                                nadd(var("n"), var("m")),
                                apps(g("gnadd_comm"), &[var("m"), var("n")]),
                            ),
                        ),
                        // `ofNat m + negSucc n` and `negSucc n + ofNat m` are the SAME term.
                        lam(
                            "n",
                            nat(),
                            refl_gint(apps(g("gsubNatNat"), &[var("m"), succ(var("n"))])),
                        ),
                        var("b"),
                    ),
                ),
                // a = negSucc m: split on b.
                lam(
                    "m",
                    nat(),
                    gint_ind(
                        lam(
                            "bb",
                            gint(),
                            eq_gint(
                                add(gneg_succ(var("m")), var("bb")),
                                add(var("bb"), gneg_succ(var("m"))),
                            ),
                        ),
                        lam(
                            "n",
                            nat(),
                            refl_gint(apps(g("gsubNatNat"), &[var("n"), succ(var("m"))])),
                        ),
                        lam(
                            "n",
                            nat(),
                            gcongr(
                                nat(),
                                gint(),
                                neg_succ_succ.clone(),
                                nadd(var("m"), var("n")),
                                nadd(var("n"), var("m")),
                                apps(g("gnadd_comm"), &[var("m"), var("n")]),
                            ),
                        ),
                        var("b"),
                    ),
                ),
                var("a"),
            ),
        ),
    );
    let got = infer_type(&ctx, &add_comm_tm)
        .unwrap_or_else(|e| panic!("the `add_comm` double induction must check: {e:?}"));
    assert!(
        is_subtype(&ctx, &got, &add_comm_ty),
        "`add_comm` must be proved, not assumed\n  claimed: {add_comm_ty}\n  inferred: {got}"
    );
    ctx.add_definition("gadd_comm".to_string(), add_comm_ty, add_comm_tm);
    assert_eq!(
        double_check(&ctx, &g("gadd_comm")),
        DoubleCheck::Agreed,
        "both kernels must certify `add_comm`, got {:?}",
        double_check(&ctx, &g("gadd_comm"))
    );

    // ---- mul_one : Π(a : Int). a * 1 = a ---------------------------------------------
    let mul_one_ty = pi(
        "a",
        gint(),
        eq_gint(apps(g("gimul"), &[var("a"), gone()]), var("a")),
    );
    let mul_one_tm = lam(
        "a",
        gint(),
        gint_ind(
            lam(
                "aa",
                gint(),
                eq_gint(apps(g("gimul"), &[var("aa"), gone()]), var("aa")),
            ),
            lam(
                "m",
                nat(),
                gcongr(
                    nat(),
                    gint(),
                    g("GIntOfNat"),
                    apps(g("gnmul"), &[var("m"), succ(g("Zero"))]),
                    var("m"),
                    app(g("gnmul_one"), var("m")),
                ),
            ),
            lam(
                "m",
                nat(),
                gcongr(
                    nat(),
                    gint(),
                    g("gnegOfNat"),
                    apps(g("gnmul"), &[succ(var("m")), succ(g("Zero"))]),
                    succ(var("m")),
                    app(g("gnmul_one"), succ(var("m"))),
                ),
            ),
            var("a"),
        ),
    );
    let got = infer_type(&ctx, &mul_one_tm)
        .unwrap_or_else(|e| panic!("the `mul_one` induction must check: {e:?}"));
    assert!(
        is_subtype(&ctx, &got, &mul_one_ty),
        "`mul_one` must be proved, not assumed\n  claimed: {mul_one_ty}\n  inferred: {got}"
    );
    ctx.add_definition("gmul_one".to_string(), mul_one_ty, mul_one_tm);
    assert_eq!(
        double_check(&ctx, &g("gmul_one")),
        DoubleCheck::Agreed,
        "both kernels must certify `mul_one`, got {:?}",
        double_check(&ctx, &g("gmul_one"))
    );

    // THE NEGATIVE, and it is the reason this is a proof rather than a spelling: the SAME
    // shapes at a FALSE law must be refused. `a * 1 = 0` and `a + b = a` have the identical
    // induction skeletons and must not check.
    let bogus_mul = lam(
        "a",
        gint(),
        gint_ind(
            lam(
                "aa",
                gint(),
                eq_gint(apps(g("gimul"), &[var("aa"), gone()]), gof(g("Zero"))),
            ),
            lam("m", nat(), refl_gint(gof(var("m")))),
            lam("m", nat(), refl_gint(gneg_succ(var("m")))),
            var("a"),
        ),
    );
    assert!(
        infer_type(&ctx, &bogus_mul).is_err(),
        "`a * 1 = 0` is false and must be refused"
    );

    let bogus_add = lam(
        "a",
        gint(),
        lam(
            "b",
            gint(),
            gint_ind(
                lam("aa", gint(), eq_gint(add(var("aa"), var("b")), var("aa"))),
                lam("m", nat(), refl_gint(gof(var("m")))),
                lam("m", nat(), refl_gint(gneg_succ(var("m")))),
                var("a"),
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &bogus_add).is_err(),
        "`a + b = a` is false and must be refused"
    );

    // …and the proved laws COMPUTE on closed integers, both ways round, over the sign grid.
    use logicaffeine_kernel::normalize;
    for a in [-3i64, -1, 0, 1, 4] {
        for b in [-2i64, 0, 2, 5] {
            let ab = normalize(&ctx, &add(gint_of(a), gint_of(b)));
            let ba = normalize(&ctx, &add(gint_of(b), gint_of(a)));
            assert_eq!(ab, ba, "`{a} + {b}` and `{b} + {a}` must be one value");
            assert_eq!(ab, gint_of(a + b), "…and it must be {}", a + b);
        }
        assert_eq!(
            normalize(&ctx, &apps(g("gimul"), &[gint_of(a), gone()])),
            gint_of(a),
            "`{a} * 1` must be `{a}`"
        );
    }
}

// =======================================================================================
// WAVE 3 — THE ι-SPECIFICATION, AS ONE NUMBER
// =======================================================================================
//
// `termination::structural_columns` is the guard's own answer to "which argument does this
// fixpoint recurse on", and it is the answer a REDUCER owes agreement to. A reducer that picks
// argument 0 instead computes a DIFFERENT function from the one the guard checked: Lean's
// `Nat.add` recurses on its second argument, so `n + 0 = n` is `rfl` in Lean and stuck here.
//
// `structural_position` is the same answer shaped for a reducer — one position, or `None` when
// the guard admits no column and therefore no ι rule exists for that fixpoint at all.

/// A one-column fixpoint over `Nat` whose `match` is on the binder at `on`, so the structural
/// column is at that position by construction.
fn one_column_fix(on: usize) -> Term {
    let scrutinee = if on == 0 { "p" } else { "q" };
    let other = if on == 0 { "q" } else { "p" };
    fix(
        "f",
        lam(
            "p",
            nat(),
            lam(
                "q",
                nat(),
                match_(
                    var(scrutinee),
                    nat_motive(),
                    vec![
                        g("Zero"),
                        lam(
                            "k",
                            nat(),
                            succ(if on == 0 {
                                apps(var("f"), &[var("k"), var(other)])
                            } else {
                                apps(var("f"), &[var(other), var("k")])
                            }),
                        ),
                    ],
                ),
            ),
        ),
    )
}

#[test]
fn the_guards_own_answer_to_which_argument_unrolls_is_available_as_one_number() {
    use logicaffeine_kernel::termination::{structural_columns, structural_position};

    let ctx = std_ctx();

    // The measure's FIRST column is the ι position, for both spellings of the same recursion.
    for on in [0usize, 1] {
        let Term::Fix { body, .. } = one_column_fix(on) else {
            unreachable!()
        };
        assert_eq!(
            structural_position(&ctx, &body),
            Some(on),
            "a fixpoint whose `match` is on binder {on} recurses on argument {on}"
        );
        assert_eq!(
            structural_columns(&ctx, &body).first().copied(),
            structural_position(&ctx, &body),
            "the two answers are one answer — a reducer asking the short one must not get a \
             different column from the guard's own measure"
        );
    }

    // The lexicographic shape: `ack`'s measure is (m, n) and ι is on column 0's position.
    let Term::Fix { body, .. } = two_column_fix(
        var("n"),
        succ(g("Zero")),
        apps(var("f"), &[var("m1"), var("n2")]),
    ) else {
        unreachable!()
    };
    let cols = structural_columns(&ctx, &body);
    assert!(
        cols.len() >= 2,
        "both `Nat` binders are columns of the measure: {cols:?}"
    );
    assert_eq!(structural_position(&ctx, &body), cols.first().copied());

    // NO inductive binder at all: the guard admits nothing, so there is no ι rule — and the
    // reducer must be told `None` rather than handed an index into an empty measure.
    let no_column = lam("f2", arrow(nat(), nat()), app(var("f2"), g("Zero")));
    assert_eq!(
        structural_position(&ctx, &no_column),
        None,
        "a fixpoint the guard would refuse has no ι rule; unrolling one is how a normaliser \
         diverges"
    );
    assert!(structural_columns(&ctx, &no_column).is_empty());

    // THE DIFFERENTIAL that keeps the two in step: everything `check_termination` ADMITS has a
    // structural position, and that position is one of the measure's own columns.
    use logicaffeine_kernel::termination::check_termination;
    let mut ctx2 = std_ctx();
    with_pred(&mut ctx2);
    let admitted: Vec<Term> = vec![
        one_column_fix(0),
        one_column_fix(1),
        two_column_fix(
            var("n"),
            succ(g("Zero")),
            apps(var("f"), &[var("m1"), var("n2")]),
        ),
    ];
    for t in &admitted {
        let Term::Fix { name, body } = t else {
            unreachable!()
        };
        check_termination(&ctx2, name, body).expect("the fixture must be admitted by the guard");
        let p = structural_position(&ctx2, body)
            .expect("an admitted fixpoint must have a structural column");
        assert!(
            structural_columns(&ctx2, body).contains(&p),
            "the ι position must be a column of the measure the guard checked"
        );
    }
}

#[test]
fn a_fixpoint_that_recurses_on_its_second_argument_must_compute_like_lean() {
    use logicaffeine_kernel::termination::structural_position;
    use logicaffeine_kernel::{double_check, normalize, DoubleCheck};

    // THE REDUCER'S OBLIGATION. `gnadd` is Lean's `Nat.add`: it recurses on its SECOND
    // argument, the guard admitted it on that column, and `structural_position` says so out
    // loud. The MAIN kernel now asks — `reduction.rs::reduce_step_at` reads the column off
    // `termination::structural_columns` — so `gnadd gm 0` reduces to `gm` for a free `gm`,
    // which is the assertion below and which was stuck through waves 1 and 2.
    //
    // `recheck.rs::whnf`'s `Db::Fix` arm still keys on argument 0, so the two kernels no longer
    // agree about this term; the neighbouring
    // `the_two_kernels_must_agree_about_which_argument_unrolls_a_fixpoint` is the tripwire that
    // says so and it is RED. The second kernel must implement the same specification
    // INDEPENDENTLY over de Bruijn levels — never by copying `reduction.rs`, which is how both
    // came to be wrong in the same way and why `double_check` could not see it before.
    let mut ctx = std_ctx();
    with_gint(&mut ctx);
    ctx.add_declaration("gm", nat());

    let Some(Term::Fix { body, .. }) = ctx.get_definition_body("gnadd").cloned() else {
        panic!("`gnadd` is a fixpoint");
    };
    assert_eq!(
        structural_position(&ctx, &body),
        Some(1),
        "the guard admitted `gnadd` on its SECOND argument — that is the ι column"
    );

    let stuck = apps(g("gnadd"), &[g("gm"), g("Zero")]);
    let answer = normalize(&ctx, &stuck);
    assert_eq!(
        double_check(&ctx, &stuck),
        DoubleCheck::Agreed,
        "whatever the two kernels decide, they must decide it together"
    );
    assert_eq!(
        answer,
        g("gm"),
        "`gnadd gm 0` must reduce to `gm`: the ι rule fires on the STRUCTURAL column \
         (`structural_position` = 1, and argument 1 is the constructor `Zero`), not on \
         argument 0. reduction.rs::reduce_step_at's `Term::Fix` arm and recheck.rs::whnf's \
         `Db::Fix` arm both key on argument 0 and leave this stuck; got {answer}"
    );
}

#[test]
fn the_nested_door_resolves_references_before_it_believes_a_field_is_harmless() {
    use logicaffeine_kernel::NestedDecl;

    // THE SAME δ HOLE, AT THE NESTED DOOR — and this one `inductive_compile.rs` owns, so it is
    // closed here rather than reported. Declare the inductive, define an alias for its own
    // negation, then nest through a container while carrying the alias as an ordinary field.
    // Nothing in the constructor SPELLS `LNeg`'s inductive, so a name-comparing fence sees a
    // harmless carried field where the Russell constructor is.
    let mut ctx = std_ctx();
    register_container(&mut ctx, "LBox", &[]);
    ctx.add_inductive("LNeg", sort(0));
    ctx.add_definition(
        "LNegate".to_string(),
        Term::Sort(Universe::Prop),
        g_neg(g("LNeg")),
    );

    let poisoned = NestedDecl {
        name: "LNeg".to_string(),
        sort: sort(0),
        constructors: vec![(
            "lneg_node".to_string(),
            arrow(
                g("LNegate"),
                arrow(apps(g("LBox"), &[g("LNeg")]), g("LNeg")),
            ),
        )],
    };
    let refused = ctx.add_nested_inductive(&poisoned);
    assert!(
        refused.is_err(),
        "`LNegate` is `LNeg → False` up to δ, so `lneg_node` puts `LNeg` in a DOMAIN and the \
         nested door must refuse it exactly as it refuses the literal spelling: {refused:?}"
    );
    let msg = format!("{:?}", refused.unwrap_err());
    assert!(
        msg.contains("LNeg") && msg.to_lowercase().contains("negative"),
        "the refusal must name the inductive and the hazard: {msg}"
    );

    // THE BOUNDARY: the SAME shape with the alias replaced by an unrelated one must still nest.
    // A fence that bought this refusal by rejecting carried fields in general would have broken
    // the capability T11-012 requires.
    let mut ok = std_ctx();
    register_container(&mut ok, "LBox", &[]);
    ok.add_definition("LTag".to_string(), sort(0), nat());
    let clean = NestedDecl {
        name: "LFine".to_string(),
        sort: sort(0),
        constructors: vec![(
            "lfine_node".to_string(),
            arrow(g("LTag"), arrow(apps(g("LBox"), &[g("LFine")]), g("LFine"))),
        )],
    };
    let admitted = ok.add_nested_inductive(&clean);
    assert!(
        admitted.is_ok(),
        "an alias for `Nat` carried alongside the nesting is an ordinary field: {admitted:?}"
    );

    // …and the LITERAL spelling of the hazard was refused before this change and must still be.
    let mut lit = std_ctx();
    register_container(&mut lit, "LBox", &[]);
    lit.add_inductive("LLit", sort(0));
    let literal = NestedDecl {
        name: "LLit".to_string(),
        sort: sort(0),
        constructors: vec![(
            "llit_node".to_string(),
            arrow(
                g_neg(g("LLit")),
                arrow(apps(g("LBox"), &[g("LLit")]), g("LLit")),
            ),
        )],
    };
    assert!(
        lit.add_nested_inductive(&literal).is_err(),
        "the spelled-out negative occurrence was always refused and must stay refused"
    );
}

/// `Bool`, `Eq Bool a b`.
fn bool_() -> Term {
    g("Bool")
}
fn eq_bool(a: Term, b: Term) -> Term {
    apps(g("Eq"), &[bool_(), a, b])
}

/// `GInt`'s ORDER, as Lean has it: a decidable `≤` by structural recursion, and `<`/`≥`/`>`
/// defined from it rather than assumed alongside it.
///
/// `tcb_ledger_lock.rs` classes `le`, `lt`, `ge` and `gt` as `Definable` — four of the nine ALU
/// primitives LOGOS assumes and Lean defines. They are definitions here, and `le_refl` (one of
/// the eight assumed ORDER laws) is a theorem over them.
fn with_gint_order(ctx: &mut Context) {
    let bool_motive = || lam("_", nat(), bool_());
    let gint_bool_motive = || lam("_", gint(), bool_());

    // `gnle 0 n = true`, `gnle (m+1) 0 = false`, `gnle (m+1) (n+1) = gnle m n`.
    ctx.add_definition(
        "gnle".to_string(),
        arrow(nat(), arrow(nat(), bool_())),
        fix(
            "f",
            lam(
                "m",
                nat(),
                lam(
                    "n",
                    nat(),
                    match_(
                        var("m"),
                        bool_motive(),
                        vec![
                            g("true"),
                            lam(
                                "k",
                                nat(),
                                match_(
                                    var("n"),
                                    bool_motive(),
                                    vec![
                                        g("false"),
                                        lam("j", nat(), apps(var("f"), &[var("k"), var("j")])),
                                    ],
                                ),
                            ),
                        ],
                    ),
                ),
            ),
        ),
    );
    // `gile` — the sign case analysis. A negative is below every non-negative, and two
    // negatives compare by their `negSucc` payloads REVERSED (`negSucc m = -m-1`).
    ctx.add_definition(
        "gile".to_string(),
        arrow(gint(), arrow(gint(), bool_())),
        lam(
            "a",
            gint(),
            lam(
                "b",
                gint(),
                match_(
                    var("a"),
                    gint_bool_motive(),
                    vec![
                        lam(
                            "m",
                            nat(),
                            match_(
                                var("b"),
                                gint_bool_motive(),
                                vec![
                                    lam("n", nat(), apps(g("gnle"), &[var("m"), var("n")])),
                                    lam("n", nat(), g("false")),
                                ],
                            ),
                        ),
                        lam(
                            "m",
                            nat(),
                            match_(
                                var("b"),
                                gint_bool_motive(),
                                vec![
                                    lam("n", nat(), g("true")),
                                    lam("n", nat(), apps(g("gnle"), &[var("n"), var("m")])),
                                ],
                            ),
                        ),
                    ],
                ),
            ),
        ),
    );
    // `a < b := a + 1 ≤ b`, exactly the identity `lt_succ_le` states — so that law is `refl`
    // over these definitions rather than an assumption about two unrelated primitives.
    let bin = || arrow(gint(), arrow(gint(), bool_()));
    ctx.add_definition(
        "gilt".to_string(),
        bin(),
        lam(
            "a",
            gint(),
            lam(
                "b",
                gint(),
                apps(
                    g("gile"),
                    &[apps(g("giadd"), &[var("a"), gone()]), var("b")],
                ),
            ),
        ),
    );
    ctx.add_definition(
        "gige".to_string(),
        bin(),
        lam(
            "a",
            gint(),
            lam("b", gint(), apps(g("gile"), &[var("b"), var("a")])),
        ),
    );
    ctx.add_definition(
        "gigt".to_string(),
        bin(),
        lam(
            "a",
            gint(),
            lam("b", gint(), apps(g("gilt"), &[var("b"), var("a")])),
        ),
    );

    admit(
        ctx,
        "gnle_refl",
        pi(
            "m",
            nat(),
            eq_bool(apps(g("gnle"), &[var("m"), var("m")]), g("true")),
        ),
        lam(
            "m",
            nat(),
            nat_ind(
                lam(
                    "mm",
                    nat(),
                    eq_bool(apps(g("gnle"), &[var("mm"), var("mm")]), g("true")),
                ),
                refl_at(bool_(), g("true")),
                lam(
                    "k",
                    nat(),
                    lam(
                        "ih",
                        eq_bool(apps(g("gnle"), &[var("k"), var("k")]), g("true")),
                        var("ih"),
                    ),
                ),
                var("m"),
            ),
        ),
    );
    // ---- `le_total : a ≤ b ∨ b ≤ a` — LINEARITY, the eighth of the seventeen ------------
    // The `Nat` half is a double induction whose inner one is NOT on the second argument's
    // value but on the generalised statement: the motive is `Π n. …`, so the hypothesis is
    // available at `j` rather than only at the `n` the outer induction started with. Without
    // that generalisation the `Succ k` / `Succ j` case has no hypothesis to appeal to.
    let nle = |a: Term, b: Term| apps(g("gnle"), &[a, b]);
    let nle_true = |a: Term, b: Term| eq_bool(nle(a, b), g("true"));
    let or_ = |p: Term, q: Term| apps(g("Or"), &[p, q]);
    let left_ = |p: Term, q: Term, x: Term| apps(g("left"), &[p, q, x]);
    let right_ = |p: Term, q: Term, x: Term| apps(g("right"), &[p, q, x]);
    let tt = || refl_at(bool_(), g("true"));
    let total_at = |m: Term| {
        pi(
            "n",
            nat(),
            or_(nle_true(m.clone(), var("n")), nle_true(var("n"), m)),
        )
    };
    admit(
        ctx,
        "gnle_total",
        pi("m", nat(), total_at(var("m"))),
        lam(
            "m",
            nat(),
            nat_ind(
                lam("mm", nat(), total_at(var("mm"))),
                lam(
                    "n",
                    nat(),
                    left_(
                        nle_true(g("Zero"), var("n")),
                        nle_true(var("n"), g("Zero")),
                        tt(),
                    ),
                ),
                lam(
                    "k",
                    nat(),
                    lam(
                        "ih",
                        total_at(var("k")),
                        lam(
                            "n",
                            nat(),
                            nat_ind(
                                lam(
                                    "nn",
                                    nat(),
                                    or_(
                                        nle_true(succ(var("k")), var("nn")),
                                        nle_true(var("nn"), succ(var("k"))),
                                    ),
                                ),
                                right_(
                                    nle_true(succ(var("k")), g("Zero")),
                                    nle_true(g("Zero"), succ(var("k"))),
                                    tt(),
                                ),
                                lam(
                                    "j",
                                    nat(),
                                    lam(
                                        "_ih2",
                                        or_(
                                            nle_true(succ(var("k")), var("j")),
                                            nle_true(var("j"), succ(var("k"))),
                                        ),
                                        app(var("ih"), var("j")),
                                    ),
                                ),
                                var("n"),
                            ),
                        ),
                    ),
                ),
                var("m"),
            ),
        ),
    );

    admit(
        ctx,
        "gle_refl",
        pi(
            "a",
            gint(),
            eq_bool(apps(g("gile"), &[var("a"), var("a")]), g("true")),
        ),
        lam(
            "a",
            gint(),
            gint_ind(
                lam(
                    "aa",
                    gint(),
                    eq_bool(apps(g("gile"), &[var("aa"), var("aa")]), g("true")),
                ),
                lam("m", nat(), app(g("gnle_refl"), var("m"))),
                lam("m", nat(), app(g("gnle_refl"), var("m"))),
                var("a"),
            ),
        ),
    );

    // The `Int` half: four sign cases, two of which are decided by the SIGNS alone and two of
    // which are the `Nat` totality — the second with its arguments SWAPPED, because
    // `negSucc m = -m-1` reverses the order on the payload.
    let ile_true = |a: Term, b: Term| eq_bool(apps(g("gile"), &[a, b]), g("true"));
    let total_int = |a: Term| or_(ile_true(a.clone(), var("b")), ile_true(var("b"), a));
    admit(
        ctx,
        "gle_total",
        pi("a", gint(), pi("b", gint(), total_int(var("a")))),
        lam(
            "a",
            gint(),
            lam(
                "b",
                gint(),
                gint_ind(
                    lam("aa", gint(), total_int(var("aa"))),
                    lam(
                        "m",
                        nat(),
                        gint_ind(
                            lam(
                                "bb",
                                gint(),
                                or_(
                                    ile_true(gof(var("m")), var("bb")),
                                    ile_true(var("bb"), gof(var("m"))),
                                ),
                            ),
                            lam("n", nat(), apps(g("gnle_total"), &[var("m"), var("n")])),
                            lam(
                                "n",
                                nat(),
                                right_(
                                    ile_true(gof(var("m")), gneg_succ(var("n"))),
                                    ile_true(gneg_succ(var("n")), gof(var("m"))),
                                    tt(),
                                ),
                            ),
                            var("b"),
                        ),
                    ),
                    lam(
                        "m",
                        nat(),
                        gint_ind(
                            lam(
                                "bb",
                                gint(),
                                or_(
                                    ile_true(gneg_succ(var("m")), var("bb")),
                                    ile_true(var("bb"), gneg_succ(var("m"))),
                                ),
                            ),
                            lam(
                                "n",
                                nat(),
                                left_(
                                    ile_true(gneg_succ(var("m")), gof(var("n"))),
                                    ile_true(gof(var("n")), gneg_succ(var("m"))),
                                    tt(),
                                ),
                            ),
                            lam("n", nat(), apps(g("gnle_total"), &[var("n"), var("m")])),
                            var("b"),
                        ),
                    ),
                    var("a"),
                ),
            ),
        ),
    );
}

#[test]
fn int_multiplication_commutes_through_all_four_sign_cases() {
    use logicaffeine_kernel::{double_check, is_subtype, DoubleCheck};

    // The SIXTH of the seventeen. Every one of the four sign cases reduces to `Nat`
    // commutativity — the `ofNat`/`negSucc` split is exactly the structure that makes it so —
    // and `gnmul_comm` is itself a theorem here, proved from `gnadd_assoc` and `gnadd_comm`.
    let mut ctx = std_ctx();
    with_gint(&mut ctx);
    with_gint_laws(&mut ctx);

    let imul = |a: Term, b: Term| apps(g("gimul"), &[a, b]);
    let ncomm = |a: Term, b: Term| apps(g("gnmul_comm"), &[a, b]);
    let nmul = |a: Term, b: Term| apps(g("gnmul"), &[a, b]);

    let mul_comm_ty = pi(
        "a",
        gint(),
        pi(
            "b",
            gint(),
            eq_gint(imul(var("a"), var("b")), imul(var("b"), var("a"))),
        ),
    );
    let mul_comm_tm = lam(
        "a",
        gint(),
        lam(
            "b",
            gint(),
            gint_ind(
                lam(
                    "aa",
                    gint(),
                    eq_gint(imul(var("aa"), var("b")), imul(var("b"), var("aa"))),
                ),
                lam(
                    "m",
                    nat(),
                    gint_ind(
                        lam(
                            "bb",
                            gint(),
                            eq_gint(
                                imul(gof(var("m")), var("bb")),
                                imul(var("bb"), gof(var("m"))),
                            ),
                        ),
                        lam(
                            "n",
                            nat(),
                            gcongr(
                                nat(),
                                gint(),
                                g("GIntOfNat"),
                                nmul(var("m"), var("n")),
                                nmul(var("n"), var("m")),
                                ncomm(var("m"), var("n")),
                            ),
                        ),
                        lam(
                            "n",
                            nat(),
                            gcongr(
                                nat(),
                                gint(),
                                g("gnegOfNat"),
                                nmul(var("m"), succ(var("n"))),
                                nmul(succ(var("n")), var("m")),
                                ncomm(var("m"), succ(var("n"))),
                            ),
                        ),
                        var("b"),
                    ),
                ),
                lam(
                    "m",
                    nat(),
                    gint_ind(
                        lam(
                            "bb",
                            gint(),
                            eq_gint(
                                imul(gneg_succ(var("m")), var("bb")),
                                imul(var("bb"), gneg_succ(var("m"))),
                            ),
                        ),
                        lam(
                            "n",
                            nat(),
                            gcongr(
                                nat(),
                                gint(),
                                g("gnegOfNat"),
                                nmul(succ(var("m")), var("n")),
                                nmul(var("n"), succ(var("m"))),
                                ncomm(succ(var("m")), var("n")),
                            ),
                        ),
                        lam(
                            "n",
                            nat(),
                            gcongr(
                                nat(),
                                gint(),
                                g("GIntOfNat"),
                                nmul(succ(var("m")), succ(var("n"))),
                                nmul(succ(var("n")), succ(var("m"))),
                                ncomm(succ(var("m")), succ(var("n"))),
                            ),
                        ),
                        var("b"),
                    ),
                ),
                var("a"),
            ),
        ),
    );
    let got = infer_type(&ctx, &mul_comm_tm)
        .unwrap_or_else(|e| panic!("the `mul_comm` double induction must check: {e:?}"));
    assert!(
        is_subtype(&ctx, &got, &mul_comm_ty),
        "`mul_comm` must be proved, not assumed\n  claimed: {mul_comm_ty}\n  inferred: {got}"
    );
    ctx.add_definition("gmul_comm".to_string(), mul_comm_ty, mul_comm_tm);
    assert_eq!(
        double_check(&ctx, &g("gmul_comm")),
        DoubleCheck::Agreed,
        "both kernels must certify `mul_comm`, got {:?}",
        double_check(&ctx, &g("gmul_comm"))
    );

    // THE NEGATIVE: the same skeleton at a FALSE law must be refused.
    let bogus = lam(
        "a",
        gint(),
        lam(
            "b",
            gint(),
            gint_ind(
                lam("aa", gint(), eq_gint(imul(var("aa"), var("b")), var("aa"))),
                lam("m", nat(), refl_gint(gof(var("m")))),
                lam("m", nat(), refl_gint(gneg_succ(var("m")))),
                var("a"),
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &bogus).is_err(),
        "`a * b = a` is false and must be refused"
    );

    // …and it computes over the sign grid, both ways round.
    use logicaffeine_kernel::normalize;
    for a in [-3i64, -1, 0, 2, 5] {
        for b in [-4i64, -1, 0, 1, 3] {
            let ab = normalize(&ctx, &imul(gint_of(a), gint_of(b)));
            assert_eq!(ab, normalize(&ctx, &imul(gint_of(b), gint_of(a))));
            assert_eq!(ab, gint_of(a * b), "`{a} * {b}` must be {}", a * b);
        }
    }
}

#[test]
fn the_int_order_is_defined_rather_than_assumed_and_reflexivity_is_proved() {
    use logicaffeine_kernel::normalize;

    // FOUR MORE `Definable` ENTRIES AS DEFINITIONS — `le`, `lt`, `ge`, `gt` — and the SEVENTH
    // of the seventeen laws, `le_refl`, as a theorem over them. `le_prop a b` in `prelude.rs`
    // is `Eq Bool (le a b) true`, so the statement below is the prelude's own, verbatim.
    let mut ctx = std_ctx();
    with_gint(&mut ctx);
    with_gint_laws(&mut ctx);
    with_gint_order(&mut ctx);

    for name in [
        "gnle",
        "gile",
        "gilt",
        "gige",
        "gigt",
        "gnle_refl",
        "gle_refl",
        "gnle_total",
        "gle_total",
    ] {
        assert!(
            ctx.is_definition(name),
            "`{name}` must be a definition, not an assumption"
        );
    }

    // THE ORDER ITSELF, over the whole sign grid — a `≤` that is reflexive but wrong about
    // negatives would satisfy `le_refl` and nothing else.
    let bool_of = |b: bool| if b { g("true") } else { g("false") };
    let grid = [-5i64, -3, -1, 0, 1, 2, 7];
    for a in grid {
        for b in grid {
            assert_eq!(
                normalize(&ctx, &apps(g("gile"), &[gint_of(a), gint_of(b)])),
                bool_of(a <= b),
                "`{a} ≤ {b}`"
            );
            assert_eq!(
                normalize(&ctx, &apps(g("gilt"), &[gint_of(a), gint_of(b)])),
                bool_of(a < b),
                "`{a} < {b}`"
            );
            assert_eq!(
                normalize(&ctx, &apps(g("gige"), &[gint_of(a), gint_of(b)])),
                bool_of(a >= b),
                "`{a} ≥ {b}`"
            );
            assert_eq!(
                normalize(&ctx, &apps(g("gigt"), &[gint_of(a), gint_of(b)])),
                bool_of(a > b),
                "`{a} > {b}`"
            );
        }
    }
    // `lt_succ_le : a < b → (a + 1) ≤ b` is the IDENTITY over these definitions, because `<`
    // is DEFINED as that — which is what it means for the two to be one primitive rather than
    // two assumptions that happen to agree.
    let prem = |a: Term, b: Term| eq_bool(apps(g("gilt"), &[a, b]), g("true"));
    let concl = |a: Term, b: Term| {
        eq_bool(
            apps(g("gile"), &[apps(g("giadd"), &[a, gone()]), b]),
            g("true"),
        )
    };
    let lt_succ_le = lam(
        "a",
        gint(),
        lam("b", gint(), lam("h", prem(var("a"), var("b")), var("h"))),
    );
    let want = pi(
        "a",
        gint(),
        pi(
            "b",
            gint(),
            arrow(prem(var("a"), var("b")), concl(var("a"), var("b"))),
        ),
    );
    let got = infer_type(&ctx, &lt_succ_le).expect("the identity proof infers");
    assert!(
        logicaffeine_kernel::is_subtype(&ctx, &got, &want),
        "`lt_succ_le` is `refl` over a DEFINED `<`\n  claimed: {want}\n  inferred: {got}"
    );

    // THE NEGATIVE. `le_refl` must not have been bought by a `≤` that answers `true` always:
    // the strict order at a reflexive pair must be `false`, and the same identity proof at the
    // REVERSED implication (`(a+1) ≤ b → a < b` is fine, but `a ≤ b → a < b` is not) must fail.
    for a in grid {
        assert_eq!(
            normalize(&ctx, &apps(g("gilt"), &[gint_of(a), gint_of(a)])),
            g("false"),
            "`{a} < {a}` is false"
        );
    }
    let bad = pi(
        "a",
        gint(),
        pi(
            "b",
            gint(),
            arrow(
                eq_bool(apps(g("gile"), &[var("a"), var("b")]), g("true")),
                prem(var("a"), var("b")),
            ),
        ),
    );
    let bad_proof = lam(
        "a",
        gint(),
        lam(
            "b",
            gint(),
            lam(
                "h",
                eq_bool(apps(g("gile"), &[var("a"), var("b")]), g("true")),
                var("h"),
            ),
        ),
    );
    let got_bad = infer_type(&ctx, &bad_proof).expect("it infers something");
    assert!(
        !logicaffeine_kernel::is_subtype(&ctx, &got_bad, &bad),
        "`a ≤ b → a < b` is FALSE and must not be `refl`"
    );
}

// =======================================================================================
// WAVE 4 — `Quot_rec` IS A THEOREM, AND LEAN NEVER ASSUMED IT EITHER
// =======================================================================================
//
// `prelude.rs::register_quot` registers `Quot_rec` with `add_declaration`, and its note says it
// "is a kernel PRIMITIVE: it is not derivable from `Quot_lift` … nor from `Quot_ind`".
// `tcb_ledger_lock.rs` repeated the claim and booked `Quot_rec` as `Primitive` on the strength
// of it; it now books it `Definable`. Lean 4.14.0's own source says why, twice:
//
//   * `Init/Core.lean:1515-1527` — the `init_quot` command introduces FOUR constants, `Quot`,
//     `Quot.mk`, `Quot.lift` and `Quot.ind`, plus the `Quot.sound` axiom. `Quot.rec` is not
//     among them.
//   * `Init/Core.lean:1607-1611` — `protected abbrev Quot.rec … := Eq.ndrecOn
//     (Quot.liftIndepPr1 f h q) ((lift (Quot.indep f) (Quot.indepCoherent f h) q).2)`, a
//     DEFINITION over `Quot.lift`, `Quot.ind`, `Quot.sound` and `PSigma`. Running
//     `#print axioms Quot.rec` under that toolchain answers `[Quot.sound]`.
//
// `recursor::derive_quot_rec` is that construction, transcribed. These tests are the evidence
// that the claim is false and that the axiom can leave the trusted base.

/// `Eq T a b`, spelled through the prelude.
fn eq_at_g(t: Term, a: Term, b: Term) -> Term {
    apps(g("Eq"), &[t, a, b])
}

/// The quotient every test below eliminates: `Nat` by the total relation, with the motive, the
/// branch and the respect obligation left OPAQUE. That is the honest shape — a caller of
/// `Quot.rec` cannot unfold the motive it was handed — and it is also the only shape that works,
/// because `def_eq` is not a congruence under a stuck `Eq_rec_dep`, so a λ-motive would make the
/// test fail for a reason that has nothing to do with the eliminator.
fn with_opaque_quotient(ctx: &mut Context) -> (Term, Term) {
    let rel = lam("a", nat(), lam("b", nat(), g("True")));
    let quot_ty = apps(g("Quot"), &[nat(), rel.clone()]);
    let mk = |x: Term| apps(g("Quot_mk"), &[nat(), rel.clone(), x]);
    ctx.add_declaration("QM", arrow(quot_ty.clone(), sort(0)));
    ctx.add_declaration("Qbranch", pi("a", nat(), app(g("QM"), mk(var("a")))));
    let transported = apps(
        g("Eq_rec_dep"),
        &[
            quot_ty.clone(),
            mk(var("a")),
            lam(
                "q",
                quot_ty.clone(),
                lam(
                    "_",
                    eq_at_g(quot_ty.clone(), mk(var("a")), var("q")),
                    app(g("QM"), var("q")),
                ),
            ),
            app(g("Qbranch"), var("a")),
            mk(var("b")),
            apps(
                g("Quot_sound"),
                &[nat(), rel.clone(), var("a"), var("b"), var("p")],
            ),
        ],
    );
    ctx.add_declaration(
        "Qresp",
        pi(
            "a",
            nat(),
            pi(
                "b",
                nat(),
                pi(
                    "p",
                    apps(rel.clone(), &[var("a"), var("b")]),
                    eq_at_g(
                        app(g("QM"), mk(var("b"))),
                        transported,
                        app(g("Qbranch"), var("b")),
                    ),
                ),
            ),
        ),
    );
    (rel, quot_ty)
}

/// **The re-checker cannot yet corroborate this one, and that is a finding, not a caveat.**
/// `double_check` answers `Disagree` on the derived term with
///
/// ```text
/// have     Eq (motive (mk a)) (Eq_rec_dep … (f a) (mk a) (refl … (mk a))) b2
/// expected Eq (motive (mk a)) (f a) b2
/// ```
///
/// — the re-checker did not ι-reduce `Eq_rec_dep … (refl …)` to its base. That is not a
/// property of this term: `recheck.rs`'s `Db::Fix` arm unrolls a fixpoint only when ARGUMENT 0
/// is in constructor form, and `Eq_rec_dep`'s structural column is the EQUATION, the last
/// argument. Every J-at-`refl` step in the language is invisible to the second kernel for the
/// same reason (the same note stands on `gnadd_zero` above), and `reduction.rs` already asks the
/// guard WHICH column ι is on. Until `recheck.rs` asks it too, the second kernel corroborates
/// nothing that transports along a `refl`. This test therefore pins the MAIN kernel's judgment
/// — which is what "derivable" means — and the negative below pins that the certification is
/// not vacuous.
#[test]
fn the_dependent_quotient_eliminator_is_derivable_and_the_kernel_certifies_it() {
    use logicaffeine_kernel::recursor::derive_quot_rec;

    let ctx = std_ctx();
    let (ty, tm) = derive_quot_rec(&ctx).unwrap_or_else(|e| {
        panic!(
            "`Quot_rec` is Lean's `Quot.lift`-through-`Sigma` construction \
             (Init/Core.lean:1607-1611) and must derive from the four primitives the prelude \
             already has: {e:?}"
        )
    });
    let inferred = infer_type(&ctx, &tm)
        .expect("the derivation is a TERM the kernel checks, not a claim about one");
    assert!(
        logicaffeine_kernel::is_subtype(&ctx, &inferred, &ty)
            && logicaffeine_kernel::is_subtype(&ctx, &ty, &inferred),
        "the reported type must be the certified one: reported {ty}, certified {inferred}"
    );
    // NOT VACUOUS: break the respect obligation's transport and the same door refuses it.
    let broken = match tm.clone() {
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param,
            param_type,
            body: Box::new(app(g("Quot_sound"), *body)),
        },
        other => other,
    };
    assert!(
        infer_type(&ctx, &broken).is_err(),
        "the door that accepted the derivation must reject a mangled one"
    );

    // It is not circular, and it does rest on the axiom Lean also pays for.
    let spelled = format!("{tm}");
    assert!(
        !spelled.contains("Quot_rec"),
        "the derivation must not cite the constant it replaces"
    );
    for cited in [
        "Quot_lift",
        "Quot_ind",
        "Quot_sound",
        "Sigma_mk",
        "Eq_rec_dep",
    ] {
        assert!(
            spelled.contains(cited),
            "Lean's recipe goes through `{cited}`; this derivation does not mention it"
        );
    }
}

#[test]
fn the_derived_quotient_eliminator_eliminates_exactly_where_the_assumed_one_does() {
    use logicaffeine_kernel::recursor::derive_quot_rec;

    let mut ctx = std_ctx();
    let (ty, tm) = derive_quot_rec(&ctx).expect("the dependent quotient eliminator derives");
    ctx.add_definition("G_Quot_rec".to_string(), ty, tm);
    let (rel, _quot_ty) = with_opaque_quotient(&mut ctx);
    let mk = |x: Term| apps(g("Quot_mk"), &[nat(), rel.clone(), x]);

    let args = [
        nat(),
        rel.clone(),
        g("QM"),
        g("Qbranch"),
        g("Qresp"),
        mk(g("Zero")),
    ];
    let derived = infer_type(&ctx, &apps(g("G_Quot_rec"), &args)).unwrap_or_else(|e| {
        panic!("the derived eliminator must accept the transport-stated obligation: {e:?}")
    });
    let assumed = infer_type(&ctx, &apps(g("Quot_rec"), &args))
        .expect("the assumed eliminator accepts it — that is the baseline");
    assert!(
        logicaffeine_kernel::is_subtype(&ctx, &derived, &assumed)
            && logicaffeine_kernel::is_subtype(&ctx, &assumed, &derived),
        "the derived eliminator must conclude exactly what the assumed one concludes, so the \
         swap is type-compatible: derived {derived}, assumed {assumed}"
    );

    // THE NEGATIVE, carried over from the assumed one: a respect proof written WITHOUT the
    // transport is ill-typed — `f a` and `f b` do not inhabit a common type — and a derived
    // eliminator that accepted it would have made the obligation decoration.
    let mut naive = ctx.clone();
    naive.add_declaration(
        "QrespNaive",
        pi(
            "a",
            nat(),
            pi(
                "b",
                nat(),
                pi(
                    "p",
                    apps(rel.clone(), &[var("a"), var("b")]),
                    eq_at_g(
                        app(g("QM"), mk(var("b"))),
                        app(g("Qbranch"), var("a")),
                        app(g("Qbranch"), var("b")),
                    ),
                ),
            ),
        ),
    );
    let bad = apps(
        g("G_Quot_rec"),
        &[
            nat(),
            rel.clone(),
            g("QM"),
            g("Qbranch"),
            g("QrespNaive"),
            mk(g("Zero")),
        ],
    );
    assert!(
        infer_type(&naive, &bad).is_err(),
        "a respect proof stated without the transport must still be refused"
    );
}

// =======================================================================================
// WAVE 4 — THE NESTED MIRROR IS ORDER-FAITHFUL
// =======================================================================================
//
// Every carried-field test wave 2 wrote puts the carried field BEFORE the element
// (`Π(A). Nat → A → C A → C A`). The two tests that demand a refusal — `nested_inductives::
// nesting_in_an_impure_container_is_rejected` and `an_impure_container_is_refused_with_a_reason_
// that_names_the_hazard` — both put it BETWEEN the element and the spine
// (`Π(A). A → Nat → C A → C A`). Nothing in `inductive_compile.rs` reads field POSITION —
// `FieldKind` is assigned by matching a field's TYPE against the parameter telescope — so the two
// shapes are the same shape and no fence can separate them. This test is that claim, taken by
// COMPUTATION rather than by reading the classifier: the carried field must land in the sibling
// at its own index, and the isos must still invert on a closed value.
//
// It is also the executable Lean transcript. Lean 4.14.0, this box:
//
//     inductive Labelled (α : Type) | lnil | lcons : String → α → Labelled α → Labelled α
//     inductive LTree | node : Labelled LTree → LTree
//     inductive Labelled2 (α : Type) | lnil | lcons : α → Nat → Labelled2 α → Labelled2 α
//     inductive LTree2 | node : Labelled2 LTree2 → LTree2
//
// Both are accepted, and `#check @LTree.rec` prints the specialised MUTUAL block this compiler
// emits — `{motive_1 : LTree → Sort u_1} → {motive_2 : Labelled LTree → Sort u_1} → …` with the
// carried `(a : String)` riding through the `lcons` minor premise in its own slot.

/// `Π(A:Type 0). ft₁ → … → A → e₁ → … → C A → C A` — a container whose carried fields sit
/// AFTER the element, which is the half of the space `register_container` never covers.
fn register_container_after(ctx: &mut Context, name: &str, extra_after: &[Term]) {
    ctx.add_inductive(name, pi("A", sort(0), sort(0)));
    ctx.add_constructor(
        &format!("{name}_nil"),
        name,
        pi("A", sort(0), app(g(name), var("A"))),
    );
    let mut ty_ = arrow(app(g(name), var("A")), app(g(name), var("A")));
    for e in extra_after.iter().rev() {
        ty_ = arrow(e.clone(), ty_);
    }
    ty_ = arrow(var("A"), ty_);
    ctx.add_constructor(&format!("{name}_cons"), name, pi("A", sort(0), ty_));
}

#[test]
fn a_carried_field_after_the_element_mirrors_into_the_same_slot_it_came_from() {
    use logicaffeine_kernel::recursor::derive_recursor;
    use logicaffeine_kernel::{double_check, normalize, DoubleCheck};

    let mut ctx = std_ctx();
    register_container_after(&mut ctx, "NList", &[nat()]);
    let info = ctx
        .add_nested_inductive(&nested_through("NTree", "NList"))
        .expect("a closed carried field is a closed carried field wherever it sits");
    let sib = info.siblings[0].clone();
    let ctors: Vec<(String, Term)> = ctx
        .get_constructors(&sib)
        .iter()
        .map(|(n, t)| (n.to_string(), (*t).clone()))
        .collect();
    assert_eq!(
        ctors[1].1,
        arrow(g("NTree"), arrow(nat(), arrow(g(&sib), g(&sib)))),
        "the mirror must be ORDER-FAITHFUL: `A → Nat → NList A → NList A` becomes \
         `NTree → Nat → {sib} → {sib}`, not the permutation"
    );

    // …and the isos still invert, on a value that carries the field.
    let iso = &info.isos[0];
    let to = g(&iso.to_generic);
    let from = g(&iso.from_generic);
    let leaf = app(g("NTree_node"), g(&ctors[0].0));
    let spec = apps(g(&ctors[1].0), &[leaf.clone(), lit(9), g(&ctors[0].0)]);
    assert_eq!(
        normalize(&ctx, &app(from.clone(), app(to.clone(), spec.clone()))),
        normalize(&ctx, &spec),
        "from_generic (to_generic spec) = spec — the carried `9` survives in its own slot"
    );
    let gen = apps(
        g("NList_cons"),
        &[g("NTree"), leaf, lit(9), app(g("NList_nil"), g("NTree"))],
    );
    assert_eq!(
        normalize(&ctx, &app(to, app(from, gen.clone()))),
        normalize(&ctx, &gen),
        "to_generic (from_generic gen) = gen"
    );

    // …and the eliminator the whole exercise is for derives and is certified by both kernels.
    let (_, rec_tm) = derive_recursor(&ctx, "NTree").expect("NTree's recursor derives");
    assert_eq!(
        double_check(&ctx, &rec_tm),
        DoubleCheck::Agreed,
        "both kernels must certify the mutual recursor, got {:?}",
        double_check(&ctx, &rec_tm)
    );
}

// =======================================================================================
// WAVE 4 — THE SURGICAL HALF OF THE POLYMORPHIC ELIMINATOR
// =======================================================================================
//
// Flipping `derive_recursor` wholesale to the polymorphic door was MEASURED this wave and costs
// 35 regressions against 2 closures: every hand-written declaration that pinned a derived
// eliminator's motive to `Type 0` (`prelude.rs`'s `HEq_rec`, `well_founded.rs`'s expected
// `Acc_rec`, `mutual_inductives.rs`'s `forestLength`) stops matching, because monomorphic and
// polymorphic really are two declarations and `Type 0 ⋠ u`. The corpus does not ask for the
// wholesale flip; it asks for THREE constants. These tests take the measurement for the one that
// matters most — `Eq_rec_dep`, the prelude's derived J — on this kernel, so the one-line change
// in `prelude.rs::register_eq_recursor` is a checked fact rather than a hope.
//
// Lean's is `@Eq.rec : {α : Sort u_2} → {a : α} → {motive : (a' : α) → a = a' → Sort u_1} → …`
// — ONE eliminator, with the motive's level a parameter. `Lean/Meta/Constructions/RecOn.lean:31`
// carries the same fact structurally: `recOn` is declared with `recInfo.levelParams`, the
// recursor's own level telescope, because there is only one recursor to inherit them from.

#[test]
fn the_polymorphic_j_transports_a_type_family_where_the_monomorphic_one_has_no_level_to_offer() {
    use logicaffeine_kernel::recursor::{derive_recursor, derive_recursor_universe_poly};
    use logicaffeine_kernel::{double_check, DoubleCheck};

    // `λa b h. Eq_rec_dep Nat a (λi _. Type 0) Nat b h : Π(a b:Nat). a = b → Type 0` — Lean's
    // `Eq.rec` with `motive : … → Type`, the transport of a TYPE FAMILY. Its motive inhabits
    // `Π(i:Nat). Eq Nat a i → Type 1`, one level above what the monomorphic J's binder accepts.
    let big_transport = |j: &str| {
        lam(
            "a",
            nat(),
            lam(
                "b",
                nat(),
                lam(
                    "h",
                    eq_nat(var("a"), var("b")),
                    apps(
                        g(j),
                        &[
                            nat(),
                            var("a"),
                            lam(
                                "idx",
                                nat(),
                                lam("_h", eq_nat(var("a"), var("idx")), sort(0)),
                            ),
                            nat(),
                            var("b"),
                            var("h"),
                        ],
                    ),
                ),
            ),
        )
    };

    // THE PREMISE, stated so the test cannot pass for the wrong reason: the MONOMORPHIC door
    // — which is what `prelude.rs::register_eq_recursor` calls today — refuses it.
    let mut mono = std_ctx();
    let (mono_ty, mono_tm) = derive_recursor(&mono, "Eq").expect("Eq's eliminator derives");
    mono.add_definition("G_J_mono".to_string(), mono_ty, mono_tm);
    assert!(
        infer_type(&mono, &big_transport("G_J_mono")).is_err(),
        "the monomorphic J's motive binder is `… → Type 0`; a `Type 1`-valued family has no \
         level to instantiate and the application must be refused"
    );

    // THE FIX, taken on this kernel: the polymorphic door accepts the very same term, and the
    // independent kernel certifies the eliminator it came from.
    let mut poly = std_ctx();
    let (levels, poly_ty, poly_tm) =
        derive_recursor_universe_poly(&poly, "Eq").expect("Eq's polymorphic eliminator derives");
    assert_eq!(
        levels.len(),
        2,
        "one motive level beside `Eq.{{u}}`'s own carrier level, as Lean's `Eq.rec.{{u_1, u_2}}`, \
         got {levels:?}"
    );
    assert_eq!(
        levels[1], "u",
        "the carrier level is `Eq`'s own, after the motive's: {levels:?}"
    );
    // The telescope COVERS the type, so the explicit door takes the eliminator as it is.
    let mut explicit = std_ctx();
    explicit
        .declare_universe_poly(
            "G_J_explicit",
            levels.clone(),
            poly_ty.clone(),
            poly_tm.clone(),
        )
        .expect("the explicit door registers the polymorphic J with the telescope it came with");
    assert_eq!(
        double_check(&poly, &poly_tm),
        DoubleCheck::Agreed,
        "both kernels must certify the polymorphic J, got {:?}",
        double_check(&poly, &poly_tm)
    );
    poly.add_definition("G_J_poly".to_string(), poly_ty, poly_tm);
    assert!(
        infer_type(&poly, &big_transport("G_J_poly")).is_ok(),
        "Lean's `Eq.rec` takes `motive : (a' : α) → a = a' → Sort u`, so transporting a type \
         family is ordinary mathematics: {:?}",
        infer_type(&poly, &big_transport("G_J_poly"))
    );

    // …and it does not LOSE the `Type 0` instance, which is the whole worry the two-door design
    // was built around. Non-dependent transport at a `Prop` motive still goes through.
    let prop_transport = apps(
        g("G_J_poly"),
        &[
            nat(),
            g("Zero"),
            lam(
                "idx",
                nat(),
                lam(
                    "_h",
                    eq_nat(g("Zero"), var("idx")),
                    Term::Sort(Universe::Prop),
                ),
            ),
            g("True"),
            g("Zero"),
            apps(g("refl"), &[nat(), g("Zero")]),
        ],
    );
    assert!(
        infer_type(&poly, &prop_transport).is_ok(),
        "a `Prop` motive must still pass — `Prop ≤ u` holds for every instantiation: {:?}",
        infer_type(&poly, &prop_transport)
    );
}

// =======================================================================================
// WAVE 4 — THE `Int` CONSTRUCTION, LIFTED OUT OF THIS TEST FILE AND INTO A LIBRARY
// =======================================================================================
//
// Wave 2 built `GInt` here, under `G`-prefixed names, and reported the prelude change as a
// "one-line ask". The ask was never one line, because the recipe lived in a test file nobody
// else could call, so nobody took it. `kernel::int_inductive` is the same construction as a
// LIBRARY — `register_inductive_int(ctx)` — so `prelude.rs::register_primitives` genuinely is
// one line. These tests hold the library to what the `GInt` tests above proved about the recipe.

/// `Nat` with its constructors beside an `Int` CARRIER that has none — the state the prelude
/// was in before it adopted `int_inductive`, rebuilt explicitly now that the prelude's `Int`
/// carries Lean's constructors.
fn carrier_int_ctx() -> Context {
    let mut ctx = Context::new();
    ctx.add_inductive("Nat", sort(0));
    ctx.add_constructor("Zero", "Nat", nat());
    ctx.add_constructor("Succ", "Nat", arrow(nat(), nat()));
    ctx.add_inductive("Int", sort(0));
    ctx
}

#[test]
fn the_library_int_gains_leans_two_constructors_and_a_certified_eliminator() {
    use logicaffeine_kernel::int_inductive::register_inductive_int;
    use logicaffeine_kernel::recursor::{derive_auxiliaries, derive_recursor};
    use logicaffeine_kernel::{double_check, DoubleCheck};

    // The library's own run: a bare `Int` carrier with no constructors GAINS them here.
    let mut bare = carrier_int_ctx();
    assert!(
        bare.get_constructors("Int").is_empty(),
        "the premise: a constructor-less carrier"
    );
    register_inductive_int(&mut bare).expect(
        "Lean's `Int` is an ordinary strictly-positive Type 0 \
                                              inductive and must register through the CHECKED door",
    );
    let gained: Vec<String> = bare
        .get_constructors("Int")
        .iter()
        .map(|(n, _)| n.to_string())
        .collect();
    assert_eq!(
        gained,
        vec!["Int_ofNat".to_string(), "Int_negSucc".to_string()]
    );

    // The adoption has LANDED: `prelude.rs` makes this call, so the standard `Int` already
    // carries Lean's constructors, and running the library again is idempotent — it neither
    // duplicates nor reorders them.
    let mut ctx = std_ctx();
    let prelude_ctors: Vec<String> = ctx
        .get_constructors("Int")
        .iter()
        .map(|(n, _)| n.to_string())
        .collect();
    assert_eq!(
        prelude_ctors, gained,
        "the prelude's `Int` is the library's `Int`, constructor for constructor"
    );
    register_inductive_int(&mut ctx).expect("re-running the library on the prelude is a no-op");

    let ctors: Vec<String> = ctx
        .get_constructors("Int")
        .iter()
        .map(|(n, _)| n.to_string())
        .collect();
    assert_eq!(
        ctors,
        vec!["Int_ofNat".to_string(), "Int_negSucc".to_string()],
        "Lean's two constructors, in Lean's declaration order (Init/Data/Int/Basic.lean:42,:45) \
         — `Match` is positional, so the order IS the declaration"
    );

    let (_, rec_tm) = derive_recursor(&ctx, "Int").expect("`Int`'s eliminator must derive");
    assert_eq!(
        double_check(&ctx, &rec_tm),
        DoubleCheck::Agreed,
        "induction over an integer must be a THEOREM-producing tool both kernels certify, \
         got {:?}",
        double_check(&ctx, &rec_tm)
    );

    // Disjointness and injectivity come with the declaration, derived — so `Int`'s
    // `noConfusion` is not an axiom either.
    // `Int_rec` is already PUBLISHED by the prelude, and it is exactly the certified eliminator
    // above. The prelude registers `Int`'s constructors through the checked door, which
    // synthesises Lean's whole auxiliary family at the declaration, so `Int_rec` is the umbrella's
    // OWN generation: re-deriving may replace it, and must replace it with the identical term.
    assert_eq!(
        ctx.get_definition_body("Int_rec"),
        Some(&rec_tm),
        "the prelude's `Int_rec` is the derived eliminator, not an assumption"
    );
    assert!(
        ctx.is_derived_aux("Int_rec"),
        "…and it was derived at `Int`'s declaration, not written by hand: {:?}",
        ctx.derived_aux_names("Int")
    );
    let auxs = derive_auxiliaries(&ctx, "Int");
    let rederived = auxs
        .iter()
        .find(|a| a.name == "Int_rec")
        .expect("the umbrella re-offers its own eliminator");
    assert_eq!(
        rederived.term, rec_tm,
        "re-deriving `Int_rec` reproduces the published term exactly"
    );
    let auxs: Vec<String> = auxs.into_iter().map(|a| a.name).collect();
    for want in [
        "Int_noConfusionType",
        "Int_noConfusion",
        "Int_ofNat_inj",
        "Int_negSucc_inj",
    ] {
        assert!(
            auxs.contains(&want.to_string()),
            "`{want}` must be derivable, got {auxs:?}"
        );
    }
}

#[test]
fn the_library_int_arithmetic_computes_leans_answers_in_every_sign_combination() {
    use logicaffeine_kernel::int_inductive::{canonical_int, register_inductive_int};
    use logicaffeine_kernel::normalize;

    let mut ctx = std_ctx();
    register_inductive_int(&mut ctx).expect("it registers");

    // Both zeros and all four sign quadrants, because `Int_negOfNat` is the one place `-0 = 0`
    // is decided and an off-by-one there is invisible on positives.
    let cases: &[(i64, i64)] = &[
        (0, 0),
        (0, 1),
        (1, 0),
        (3, 4),
        (4, 3),
        (-1, 0),
        (0, -1),
        (-1, -1),
        (-3, -4),
        (-4, -3),
        (3, -4),
        (-4, 3),
        (4, -3),
        (-3, 4),
        (2, -2),
        (-2, 2),
    ];
    for &(a, b) in cases {
        let (ta, tb) = (canonical_int(a), canonical_int(b));
        for (op, want) in [("Int_add", a + b), ("Int_sub", a - b), ("Int_mul", a * b)] {
            let got = normalize(&ctx, &apps(g(op), &[ta.clone(), tb.clone()]));
            assert!(int_is(&ctx, &got, want), "{op} {a} {b} = {want}, got {got}");
        }
    }
    // Negation, and the `-0 = 0` corner on its own — `Int_negOfNat` is the only place that
    // corner is decided and it is invisible on positive operands.
    for n in [-3i64, -1, 0, 1, 3] {
        let got = normalize(&ctx, &app(g("Int_neg"), canonical_int(n)));
        assert!(int_is(&ctx, &got, -n), "Int_neg {n} = {}, got {got}", -n);
    }
    // The CONSTRUCTOR is exact even where the `Nat` payload is not: a negative answer must be
    // headed by `Int_negSucc` and a non-negative one by `Int_ofNat`, so a sign error cannot hide
    // behind definitional equality.
    for (a, b, head) in [
        (1i64, 2i64, "Int_negSucc"),
        (2, 1, "Int_ofNat"),
        (2, 2, "Int_ofNat"),
    ] {
        let got = normalize(
            &ctx,
            &apps(g("Int_sub"), &[canonical_int(a), canonical_int(b)]),
        );
        assert!(
            matches!(&got, Term::App(f, _) if **f == g(head)),
            "Int_sub {a} {b} must be headed by `{head}`, got {got}"
        );
    }
}

/// `t` is the integer `n`, up to DEFINITIONAL equality.
///
/// Structural equality is the wrong comparison here and the reason is worth stating: the
/// inductive `Int_add` bottoms out in `Nat_add`, which peels ONE `Succ` per ι-step off a
/// `Literal::Nat`, so `Int_add (ofNat 4) (ofNat 3)` normalises to `ofNat (Succ (Succ (Succ
/// (Lit 4))))` rather than to `ofNat (Lit 7)`. The two are definitionally equal and the kernel
/// says so; what they are not is the same tree. Lean does re-compact, because its kernel
/// recognises `Nat.add` BY NAME and answers with one GMP addition
/// (`Init/Data/Int/Basic.lean:126`'s `ofNat (m + n)` arm is one `Nat` add) — the same
/// program-identity shape `reduction.rs::accelerates_nat_order` already uses for `natLe`.
fn int_is(ctx: &Context, t: &Term, n: i64) -> bool {
    use logicaffeine_kernel::int_inductive::canonical_int;
    logicaffeine_kernel::defeq_for_test(ctx, t, &canonical_int(n))
}

#[test]
fn the_library_int_and_the_literal_fast_path_compute_the_same_function() {
    use logicaffeine_kernel::int_inductive::{canonical_int, register_inductive_int};
    use logicaffeine_kernel::normalize;

    // THE EQUIVALENCE THE ACCELERATOR OWES, now against a LIBRARY definition rather than a
    // test-local one. Lean ships an inductive `Int` AND a fast path, and the fast path is only
    // legitimate because it computes the same function the definition does.
    let mut ctx = std_ctx();
    register_inductive_int(&mut ctx).expect("it registers");
    let lit_int = |n: i64| Term::Lit(Literal::Int(n));

    let mut compared = 0usize;
    for a in -4i64..=4 {
        for b in -4i64..=4 {
            for (defined, accelerated, want) in [
                ("Int_add", "add", a + b),
                ("Int_sub", "sub", a - b),
                ("Int_mul", "mul", a * b),
            ] {
                let got = normalize(
                    &ctx,
                    &apps(g(defined), &[canonical_int(a), canonical_int(b)]),
                );
                assert!(
                    int_is(&ctx, &got, want),
                    "the DEFINITION must answer {want} for {defined} {a} {b}, got {got}"
                );
                assert_eq!(
                    normalize(&ctx, &apps(g(accelerated), &[lit_int(a), lit_int(b)])),
                    lit_int(want),
                    "…and the ACCELERATED `{accelerated} {a} {b}` must answer the same"
                );
                compared += 1;
            }
        }
    }
    assert_eq!(compared, 243, "9 × 9 × three operations, all compared");
}

#[test]
fn the_int_library_refuses_to_define_arithmetic_before_the_constructors_exist() {
    use logicaffeine_kernel::int_inductive::{register_int_arithmetic, register_nat_arithmetic};

    // FAIL-CLOSED, and it matters: `Int_add` is defined BY CASES on `ofNat`/`negSucc`, so
    // registering it against a zero-constructor `Int` would emit a `Match` with two cases
    // against no constructors — a definition that type-checks nowhere and reports its failure
    // at the first CALLER instead of here.
    let mut ctx = carrier_int_ctx();
    let err = register_int_arithmetic(&mut ctx)
        .expect_err("arithmetic before constructors must be refused");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("constructors"),
        "the refusal must say what is missing, got {msg}"
    );
    assert!(
        ctx.get_global("Int_add").is_none() && ctx.get_constructors("Int").is_empty(),
        "a refused arithmetic registers nothing and invents no constructors"
    );

    // …and the `Nat` half is independent of `Int` and may be registered alone, without
    // displacing anything a caller already bound to those names.
    let mut solo = std_ctx();
    solo.add_definition(
        "Nat_add".to_string(),
        arrow(nat(), arrow(nat(), nat())),
        lam("a", nat(), lam("b", nat(), var("a"))),
    );
    let mine = solo.get_definition_body("Nat_add").cloned();
    register_nat_arithmetic(&mut solo);
    assert_eq!(
        solo.get_definition_body("Nat_add"),
        mine.as_ref(),
        "a caller's own `Nat_add` must never be silently swapped for this module's"
    );
    assert!(
        solo.get_global("Nat_mul").is_some(),
        "…while the names it did NOT bind are still registered"
    );
}

// =======================================================================================
// WAVE 4 — `casesOn` AND `recOn`, THE TWO CONSTANTS LEAN GENERATES THAT WE DID NOT
// =======================================================================================
//
// `the_auxiliary_umbrella_offers_every_constant_lean_generates_at_a_declaration` above claims
// its title over seven names. Lean generates NINE at `Lean/Elab/Inductive.lean:721-737` —
// `mkRecOn`, `mkCasesOn`, `mkNoConfusion`, `mkBelow`, `mkIBelow`, `mkBRecOn`, `mkBInductionOn`
// — and `recOn` and `casesOn` were the two missing. They are not decoration: `cases h` compiles
// to `casesOn`, `induction … using` and every `match` elaborate through one of them, and a Lean
// proof that names `Foo.casesOn` had no LOGOS counterpart at all.
//
// Both are RESHAPINGS of `derive_recursor`'s answer — `recOn` moves the major premise in front
// of the minors (`Lean/Meta/Constructions/RecOn.lean:20-26` spells the permutation out), and
// `casesOn` additionally drops the induction hypotheses. Neither adds a gate, which is the
// point: a second door onto the same eliminator is a second place for the motive-sort rule to
// drift.

#[test]
fn case_analysis_is_derivable_and_carries_no_induction_hypothesis() {
    use logicaffeine_kernel::recursor::derive_cases_on;
    use logicaffeine_kernel::{double_check, normalize, DoubleCheck};

    let mut ctx = std_ctx();
    let (ty, tm) = derive_cases_on(&ctx, "Nat").expect("`Nat`'s case analysis derives");
    assert_eq!(
        double_check(&ctx, &tm),
        DoubleCheck::Agreed,
        "both kernels must certify `Nat_casesOn`, got {:?}",
        double_check(&ctx, &tm)
    );
    ctx.add_definition("G_Nat_casesOn".to_string(), ty, tm);

    // Lean's `@Nat.casesOn : {motive} → (t : Nat) → motive zero → ((n : Nat) → motive n.succ)
    // → motive t`. The MAJOR premise comes second and the succ branch takes ONE binder.
    let pred = apps(
        g("G_Nat_casesOn"),
        &[nat_motive(), lit(3), g("Zero"), lam("k", nat(), var("k"))],
    );
    assert!(
        infer_type(&ctx, &pred).is_ok(),
        "case analysis on a literal must type-check: {:?}",
        infer_type(&ctx, &pred)
    );
    assert!(
        logicaffeine_kernel::defeq_for_test(&ctx, &normalize(&ctx, &pred), &lit(2)),
        "`pred 3 = 2` by CASES, no recursion: got {}",
        normalize(&ctx, &pred)
    );

    // THE NEGATIVE, and it is what separates `casesOn` from `rec`: the succ branch must NOT be
    // handed an induction hypothesis. A branch written as if it were `rec`'s — two binders, the
    // field and the hypothesis — is a different type and must be refused.
    let as_if_rec = apps(
        g("G_Nat_casesOn"),
        &[
            nat_motive(),
            lit(3),
            g("Zero"),
            lam("k", nat(), lam("ih", nat(), var("ih"))),
        ],
    );
    assert!(
        infer_type(&ctx, &as_if_rec).is_err(),
        "`casesOn` supplies no hypothesis, so a `rec`-shaped branch has the wrong type — \
         accepting it would make the two constants interchangeable and `casesOn` would silently \
         recurse"
    );
}

#[test]
fn the_reordered_eliminator_takes_its_scrutinee_before_its_branches() {
    use logicaffeine_kernel::recursor::derive_rec_on;
    use logicaffeine_kernel::{double_check, normalize, DoubleCheck};

    let mut ctx = std_ctx();
    let (ty, tm) = derive_rec_on(&ctx, "Nat").expect("`Nat`'s reordered eliminator derives");
    assert_eq!(
        double_check(&ctx, &tm),
        DoubleCheck::Agreed,
        "both kernels must certify `Nat_recOn`, got {:?}",
        double_check(&ctx, &tm)
    );
    ctx.add_definition("G_Nat_recOn".to_string(), ty, tm);

    // `recOn motive n base step` — and it still ι-computes, because it IS the eliminator.
    let add = apps(
        g("G_Nat_recOn"),
        &[
            nat_motive(),
            lit(2),
            lit(5),
            lam("_", nat(), lam("ih", nat(), succ(var("ih")))),
        ],
    );
    assert!(
        infer_type(&ctx, &add).is_ok(),
        "the reordered eliminator must type-check"
    );
    assert_eq!(
        normalize(&ctx, &add),
        succ(succ(lit(5))),
        "5 + 2 = 7 through `recOn`"
    );

    // THE NEGATIVE: the ORDER is the whole content. `rec`'s argument order — minors before the
    // major premise — must not also type-check, or the reordering would be decoration.
    let in_rec_order = apps(
        g("G_Nat_recOn"),
        &[
            nat_motive(),
            lit(5),
            lam("_", nat(), lam("ih", nat(), succ(var("ih")))),
            lit(2),
        ],
    );
    assert!(
        infer_type(&ctx, &in_rec_order).is_err(),
        "`recOn` takes the scrutinee second; handing it `rec`'s order must be refused"
    );
}

#[test]
fn every_declaration_now_offers_the_nine_constants_lean_generates() {
    use logicaffeine_kernel::recursor::derive_auxiliaries;

    // The umbrella's own claim, re-measured against Lean's actual generator list at
    // `Lean/Elab/Inductive.lean:721-737`. `mkIBelow` and `mkBInductionOn` are the two still
    // missing, and they are named rather than silently absent.
    let mut ctx = std_ctx();
    ctx.add_inductive("Tally2", sort(0));
    ctx.add_constructor_checked("Nought2", "Tally2", g("Tally2"))
        .expect("Nought2 registers");
    ctx.add_constructor_checked("Tick2", "Tally2", arrow(g("Tally2"), g("Tally2")))
        .expect("Tick2 is strictly positive");

    let names: Vec<String> = derive_auxiliaries(&ctx, "Tally2")
        .into_iter()
        .map(|a| a.name)
        .collect();
    for want in [
        "Tally2_rec",
        "Tally2_recOn",
        "Tally2_casesOn",
        "Tally2_noConfusionType",
        "Tally2_noConfusion",
        "Tick2_inj",
        "Tally2_below",
        "Tally2_brecOn",
        "decEqTally2",
    ] {
        assert!(
            names.contains(&want.to_string()),
            "`{want}` must be offered, got {names:?}"
        );
    }

    // …and an INDEXED family gets them too, which is the case the permutation could quietly get
    // wrong: `recOn`/`casesOn` move the indices in front of the minors, and an off-by-one there
    // would emit a telescope that type-checks against the wrong motive instance.
    let vect = |a: Term, n: Term| apps(g("Vect2"), &[a, n]);
    ctx.add_indexed_inductive("Vect2", pi("A", sort(0), arrow(nat(), sort(0))), 1);
    ctx.add_constructor(
        "vnil2",
        "Vect2",
        pi("A", sort(0), vect(var("A"), g("Zero"))),
    );
    ctx.add_constructor(
        "vcons2",
        "Vect2",
        pi(
            "A",
            sort(0),
            pi(
                "n",
                nat(),
                arrow(
                    var("A"),
                    arrow(vect(var("A"), var("n")), vect(var("A"), succ(var("n")))),
                ),
            ),
        ),
    );
    let vauxs = derive_auxiliaries(&ctx, "Vect2");
    let vnames: Vec<String> = vauxs.iter().map(|a| a.name.clone()).collect();
    for want in ["Vect2_rec", "Vect2_recOn", "Vect2_casesOn"] {
        assert!(
            vnames.contains(&want.to_string()),
            "an INDEXED family must get `{want}` too, got {vnames:?}"
        );
    }

    // …and every reshaping the umbrella hands back must be certified by BOTH kernels and carry
    // the type it reports. `derive_auxiliaries` registers what it returns WITHOUT re-checking
    // it, so a reshaping that only the constructing kernel believes would land as a constant
    // whose declared type is not its own — the one failure mode a telescope permutation can
    // hide.
    use logicaffeine_kernel::{double_check, DoubleCheck};
    for a in derive_auxiliaries(&ctx, "Tally2").into_iter().chain(vauxs) {
        if !(a.name.ends_with("_recOn") || a.name.ends_with("_casesOn")) {
            continue;
        }
        let got = infer_type(&ctx, &a.term)
            .unwrap_or_else(|e| panic!("`{}` must type-check: {e:?}", a.name));
        assert_eq!(
            got, a.ty,
            "`{}`'s reported type must be the inferred one",
            a.name
        );
        assert_eq!(
            double_check(&ctx, &a.term),
            DoubleCheck::Agreed,
            "both kernels must certify `{}`, got {:?}",
            a.name,
            double_check(&ctx, &a.term)
        );
    }
}
