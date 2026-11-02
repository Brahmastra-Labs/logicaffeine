//! Tier 4 of the Lean-4 conformance corpus — **recursion and pattern matching**,
//! re-read as a CAPABILITY matrix (`crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/t04_recursion.json`).
//!
//! Two facts shape every test below:
//!
//! 1. **`Term::Match` is positional and has no patterns at all.** `cases: Vec<Term>` carries no
//!    constructor tag, no binder list, no guard, no default; the kernel zips the cases **by
//!    index** against the registered constructors (for an indexed family, against the
//!    constructors its indices allow), so the i-th case *is* the i-th constructor. Everything
//!    Lean's equation compiler produces — multiple discriminants, nested patterns, wildcards,
//!    overlapping first-match equations, impossible-case discharge — compiles down to that:
//!    `interface::match_compile` is LOGOS's equation compiler (column-based pattern-matrix
//!    compilation with Lean's first-match semantics), and `Consider … When …` reaches it.
//!    T04-007…T04-014 hold it.
//!
//! 2. **Well-founded recursion is the prelude's.** `Acc` is a registered indexed inductive,
//!    `Acc_intro` passes nested strict positivity, `Acc_rec` is a derived eliminator carrying the
//!    functional induction hypothesis, `WellFounded A R := Π(x:A). Acc A R x`, and
//!    `WellFounded_fix` is Lean's `WellFounded.fix`. The Giménez guard's applied-smaller rule
//!    admits `rec (h y hr)` in both kernels, its lexicographic measure stands for
//!    `termination_by` and its refutation refinement for `decreasing_by`, every definition gets
//!    citable equation lemmas, and the `Type 1+` end of a recursor's motive range is reached
//!    through the universe-polymorphic eliminator. T04-004…T04-006 and T04-015…T04-018 hold it.
//!
//! Classification rule used throughout: a case is `Native` when a LOGOS user can obtain the
//! mathematical object the Lean source defines, with the same denotation, out of what exists
//! today — and the test PROVES it by computing, not by `.is_ok()`. It is `Gap` when no LOGOS
//! artefact has that denotation, or when the only route to it is silent and unsound. Every case
//! of this tier is `Native`.

use logicaffeine_kernel::interface::Repl;
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, derive_recursor, derive_recursor_universe_poly, double_check, infer_type,
    normalize, recheck, BigInt, Context, DoubleCheck, Literal, MutualInductive, Term, Universe,
};

// ==== PER-FILE HELPERS (house style: every kernel test declares its own) ====

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
fn prop() -> Term {
    Term::Sort(Universe::Prop)
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
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
/// `Succ^n Zero` — the Peano numeral a `Match` on a `Nat` literal unfolds into.
fn peano(n: usize) -> Term {
    (0..n).fold(g("Zero"), |acc, _| app(g("Succ"), acc))
}
fn succ(t: Term) -> Term {
    app(g("Succ"), t)
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
/// `TList Nat` — the prelude's parametric list at `Nat`.
fn tlist_nat() -> Term {
    app(g("TList"), nat())
}
/// `TCons Nat h t`
fn tcons(h: Term, t: Term) -> Term {
    apps(g("TCons"), &[nat(), h, t])
}
/// `TNil Nat`
fn tnil() -> Term {
    app(g("TNil"), nat())
}
/// `Eq Nat a b`
fn eq_nat(a: Term, b: Term) -> Term {
    apps(g("Eq"), &[nat(), a, b])
}

/// Register `Vect : Π(α:Type 0). Nat → Type 0` with `VNil : Vect α 0` and
/// `VCons : Π(α). Π(n:Nat). α → Vect α n → Vect α (Succ n)` — one parameter, one INDEX. The
/// index is what Lean's matcher unifies away and `infer_indexed_match` cannot.
fn register_vect(ctx: &mut Context) {
    let vect = |a: Term, n: Term| apps(g("Vect"), &[a, n]);
    ctx.add_indexed_inductive("Vect", pi("a", sort(0), arrow(nat(), sort(0))), 1);
    ctx.add_constructor("VNil", "Vect", pi("a", sort(0), vect(var("a"), g("Zero"))));
    ctx.add_constructor(
        "VCons",
        "Vect",
        pi(
            "a",
            sort(0),
            pi(
                "n",
                nat(),
                arrow(
                    var("a"),
                    arrow(vect(var("a"), var("n")), vect(var("a"), succ(var("n")))),
                ),
            ),
        ),
    );
}

// ==== T04-001 · structural recursion on `Nat` (MUST, Native) ====

/// `double`, `addAcc` and the source-order trap in one. The corpus's first `@guard` is that a
/// translator preserving Lean's *source* order (`| n+1 => .. | 0 => ..`) emits `cases` in the
/// wrong constructor slots; `Nat`'s two constructors have different arities, so the kernel
/// catches that one — the silent version of the same bug lives in T04-010.
#[test]
fn structural_recursion_on_nat_computes_in_constructor_registration_order() {
    let ctx = std_ctx();

    // The registration order the positional `cases` vector is zipped against.
    let ctors: Vec<&str> = ctx
        .get_constructors("Nat")
        .iter()
        .map(|(n, _)| *n)
        .collect();
    assert_eq!(
        ctors,
        vec!["Zero", "Succ"],
        "`Nat`'s registration order pins every case vector"
    );

    // double = fix double. λn:Nat. match n return (λ_:Nat. Nat) with
    //            | Zero   => Zero
    //            | Succ k => Succ (Succ (double k))
    let double = fix(
        "double",
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                lam("_", nat(), nat()),
                vec![
                    g("Zero"),
                    lam("k", nat(), succ(succ(app(var("double"), var("k"))))),
                ],
            ),
        ),
    );
    let ty = infer_type(&ctx, &double).expect("`double` must type-check (so the guard passed)");
    assert!(
        defeq_for_test(&ctx, &ty, &arrow(nat(), nat())),
        "double : Nat → Nat, got {ty}"
    );
    assert_eq!(
        normalize(&ctx, &app(double.clone(), lit(3))),
        peano(6),
        "double 3 must compute to 6 through the Nat literal/Peano bridge"
    );
    match double_check(&ctx, &double) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify `double`, got {other:?}"),
    }

    // addAcc: the decreasing argument is NOT the first binder — `m` is an accumulator and the
    // recursion runs on `n`, at argument position 1.
    let add_acc = fix(
        "addAcc",
        lam(
            "m",
            nat(),
            lam(
                "n",
                nat(),
                match_(
                    var("n"),
                    lam("_", nat(), nat()),
                    vec![
                        var("m"),
                        lam("k", nat(), succ(apps(var("addAcc"), &[var("m"), var("k")]))),
                    ],
                ),
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &add_acc).is_ok(),
        "the guard must record the structural ARGUMENT POSITION (1 here), not assume 0"
    );
    assert_eq!(
        normalize(&ctx, &apps(add_acc, &[lit(5), lit(3)])),
        succ(succ(succ(lit(5)))),
        "addAcc 5 3 = 8"
    );

    // half2 written in Lean's source order (`| n+1 => .. | 0 => ..`): the Succ body lands in
    // `Zero`'s slot. `Nat`'s constructors differ in arity, so this one is caught.
    let half2_source_order = fix(
        "half2",
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                lam("_", nat(), nat()),
                vec![
                    lam("k", nat(), succ(app(var("half2"), var("k")))),
                    g("Zero"),
                ],
            ),
        ),
    );
    let r = infer_type(&ctx, &half2_source_order);
    assert!(
        r.is_err(),
        "cases emitted in SOURCE order rather than registration order must be refused: {r:?}"
    );
}

// ==== T04-002 · structural recursion over a parameterised inductive (MUST, Native) ====

/// `len` over the prelude's `TList`, plus the arity boundary the guard rides on.
///
/// `TCons : Π(A:Type 0). A → TList A → TList A` has THREE `Π` binders but a case of a match on
/// `TList Nat` binds only TWO — the parameter `A` is fixed by the discriminant, and
/// `compute_case_type` gives the case the type `Π(h:Nat). Π(t:TList Nat). motive (TCons Nat h t)`.
/// `termination.rs`'s `check_match_cases_guarded` nonetheless peels `count_pi_params(TCons) = 3`
/// λs and marks every one of them structurally smaller. When the match's motive is a FUNCTION
/// type the case legitimately has a third λ — and that third binder, which is not a constructor
/// field at all, is marked smaller. A recursive call on it is then admitted and the fixpoint
/// loops forever. The second kernel's `guard_match_cases` peels by the same over-count.
#[test]
fn a_list_case_binds_only_the_constructor_fields_the_guard_may_mark_smaller() {
    let ctx = std_ctx();

    // len = fix len. λl:TList Nat. match l return (λ_:TList Nat. Nat) with
    //         | TNil      => Zero
    //         | TCons h t => Succ (len t)
    let len = fix(
        "len",
        lam(
            "l",
            tlist_nat(),
            match_(
                var("l"),
                lam("_", tlist_nat(), nat()),
                vec![
                    g("Zero"),
                    lam(
                        "h",
                        nat(),
                        lam("t", tlist_nat(), succ(app(var("len"), var("t")))),
                    ),
                ],
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &len).is_ok(),
        "`len` over a parameterised inductive must type-check"
    );
    assert_eq!(
        normalize(&ctx, &app(len, tcons(lit(1), tcons(lit(2), tnil())))),
        peano(2),
        "len [1, 2] = 2"
    );

    // THE BOUNDARY. The motive is `λ_:TList Nat. TList Nat → Nat`, so the `TCons` case is
    // `λh. λt. λm. …` — three λs, of which only the first two are constructor fields. `m` is an
    // ordinary function parameter and recursing on it does not decrease: `f [x] ys` steps to
    // `f ys ys`, which steps to itself forever.
    let looping = fix(
        "f",
        lam(
            "l",
            tlist_nat(),
            match_(
                var("l"),
                lam("_", tlist_nat(), arrow(tlist_nat(), nat())),
                vec![
                    lam("m", tlist_nat(), g("Zero")),
                    lam(
                        "h",
                        nat(),
                        lam(
                            "t",
                            tlist_nat(),
                            lam("m", tlist_nat(), apps(var("f"), &[var("m"), var("m")])),
                        ),
                    ),
                ],
            ),
        ),
    );
    let main_accepts = infer_type(&ctx, &looping).is_ok();
    let recheck_accepts = recheck(&ctx, &looping).is_ok();

    // THE CONSEQUENCE, stated as sharply as it can be. Give the same match a `False`-valued
    // motive: `TNil`'s case gets one over-peeled λ and `TCons`'s gets one too, so BOTH case
    // bodies are the looping call and neither has to inhabit anything. The fixpoint then has
    // type `TList Nat → TList Nat → False`, and applying it to two lists is a closed proof of
    // `False` built entirely out of the standard library.
    let absurd = fix(
        "f",
        lam(
            "l",
            tlist_nat(),
            match_(
                var("l"),
                lam("_", tlist_nat(), arrow(tlist_nat(), g("False"))),
                vec![
                    lam("m", tlist_nat(), apps(var("f"), &[var("m"), var("m")])),
                    lam(
                        "h",
                        nat(),
                        lam(
                            "t",
                            tlist_nat(),
                            lam("m", tlist_nat(), apps(var("f"), &[var("m"), var("m")])),
                        ),
                    ),
                ],
            ),
        ),
    );
    let paradox = apps(absurd, &[tnil(), tnil()]);
    let paradox_ty = infer_type(&ctx, &paradox);

    assert!(
        !main_accepts && !recheck_accepts && paradox_ty.is_err(),
        "the over-peel is a SOUNDNESS hole, not a convenience. Main kernel accepted the \
         non-terminating fixpoint: {main_accepts}. Independent re-checker accepted it: \
         {recheck_accepts} — its `guard_match_cases` peels by the same over-count, so the \
         two-kernel guarantee gives zero protection here. Type of a CLOSED term built only from \
         the standard library: {paradox_ty:?} — a proof of `False`. The fix is to peel the \
         constructor's VALUE fields (its Π-telescope past the inductive's parameters), which is \
         exactly what `compute_case_type` already computes."
    );
}

/// A recursion written as a `match` applied to further arguments, the convoy pattern
/// `acc = fix f. λn l. (match l return (λ_. Nat → Nat) with (λx. x) | (λh t x. f (Succ x) t)) n`,
/// recurses on the `match`'s scrutinee `l`. Both kernels read the scrutinee and the fixpoint's type
/// through the application spine: the guard certifies the recursion on `l` (the threaded `x` is not
/// a field and is never smaller), the type is the motive's applied to the spine's arguments, and
/// the reducer unrolls on `l`, so an open list leaves the fixpoint stuck rather than unrolling it
/// on `n`.
#[test]
fn a_recursion_through_a_convoy_is_structural_on_the_matched_argument() {
    let ctx = std_ctx();
    let acc = fix(
        "f",
        lam(
            "n",
            nat(),
            lam(
                "l",
                tlist_nat(),
                app(
                    match_(
                        var("l"),
                        lam("_", tlist_nat(), arrow(nat(), nat())),
                        vec![
                            lam("x", nat(), var("x")),
                            lam(
                                "h",
                                nat(),
                                lam(
                                    "t",
                                    tlist_nat(),
                                    lam("x", nat(), apps(var("f"), &[succ(var("x")), var("t")])),
                                ),
                            ),
                        ],
                    ),
                    var("n"),
                ),
            ),
        ),
    );
    let expected = arrow(nat(), arrow(tlist_nat(), nat()));
    let main = infer_type(&ctx, &acc);
    assert!(
        matches!(&main, Ok(t) if defeq_for_test(&ctx, t, &expected)),
        "the convoy is structural on its list: {main:?}"
    );
    let independent = recheck(&ctx, &acc);
    assert!(
        independent.is_ok(),
        "the independent kernel reads it the same way: {independent:?}"
    );
    assert_eq!(
        normalize(
            &ctx,
            &apps(
                acc.clone(),
                &[g("Zero"), tcons(lit(7), tcons(lit(9), tnil()))]
            )
        ),
        peano(2),
        "acc 0 [7, 9] = 2"
    );
    let open = normalize(
        &ctx,
        &lam("l", tlist_nat(), apps(acc, &[g("Zero"), var("l")])),
    );
    let Term::Lambda { body, .. } = &open else {
        panic!("an abstraction normalizes to one: {open}")
    };
    let mut head = body.as_ref();
    while let Term::App(f, _) = head {
        head = f;
    }
    assert!(
        matches!(head, Term::Fix { .. }),
        "on an open list the fixpoint does not unroll: {open}"
    );

    // The threaded argument is not a field: recursing on it is refused.
    let threaded = fix(
        "f",
        lam(
            "l",
            tlist_nat(),
            app(
                match_(
                    var("l"),
                    lam("_", tlist_nat(), arrow(tlist_nat(), nat())),
                    vec![
                        lam("m", tlist_nat(), g("Zero")),
                        lam(
                            "h",
                            nat(),
                            lam(
                                "t",
                                tlist_nat(),
                                lam("m", tlist_nat(), app(var("f"), var("m"))),
                            ),
                        ),
                    ],
                ),
                var("l"),
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &threaded).is_err() && recheck(&ctx, &threaded).is_err(),
        "a recursion on the convoy's threaded argument does not decrease"
    );
}

/// A fixpoint whose result type no motive states, even through an application spine, is typed
/// against the type its position expects. Here the body under the binders is a `let` around the
/// `match`, so inference cannot read the type; as an argument and as a definition's value, both
/// kernels check it with the fixpoint bound at the expected type.
#[test]
fn a_fixpoint_is_checked_against_the_type_its_position_expects() {
    let ctx = std_ctx();
    let lf = fix(
        "f",
        lam(
            "n",
            nat(),
            Term::Let {
                name: "z".to_string(),
                ty: Box::new(nat()),
                value: Box::new(g("Zero")),
                body: Box::new(match_(
                    var("n"),
                    lam("_", nat(), nat()),
                    vec![var("z"), lam("k", nat(), app(var("f"), var("k")))],
                )),
            },
        ),
    );
    let ty = arrow(nat(), nat());
    assert!(
        infer_type(&ctx, &lf).is_err(),
        "no motive states its type, so inference cannot read it"
    );
    let applied = app(lam("g", ty.clone(), var("g")), lf.clone());
    let main = infer_type(&ctx, &applied);
    assert!(
        matches!(&main, Ok(t) if defeq_for_test(&ctx, t, &ty)),
        "checked against the domain it is passed at: {main:?}"
    );
    let independent = recheck(&ctx, &applied);
    assert!(
        independent.is_ok(),
        "the independent kernel checks it the same way: {independent:?}"
    );
    let mut defined = ctx.clone();
    let declared = defined.declare("lf", ty, lf);
    assert!(
        declared.is_ok(),
        "as a definition's value it checks against the declared type: {declared:?}"
    );
    assert_eq!(
        normalize(&defined, &app(g("lf"), peano(3))),
        g("Zero"),
        "lf 3 = 0"
    );
}

// ==== T04-003 · mutual recursion (MUST, Native) ====

/// One `MutualFix`, one shared guard: every member's structural POSITION is computed
/// separately, and a call to any member must decrease at *that* member's position.
#[test]
fn a_mutual_block_guards_each_member_at_its_own_structural_position() {
    let ctx = std_ctx();
    let bool_motive = lam("_", nat(), g("Bool"));

    let is_even_body = lam(
        "n",
        nat(),
        match_(
            var("n"),
            bool_motive.clone(),
            vec![g("true"), lam("k", nat(), app(var("isOdd"), var("k")))],
        ),
    );
    let is_odd_body = lam(
        "n",
        nat(),
        match_(
            var("n"),
            bool_motive.clone(),
            vec![g("false"), lam("k", nat(), app(var("isEven"), var("k")))],
        ),
    );
    let defs = [("isEven", is_even_body), ("isOdd", is_odd_body)];

    let even = mfix(&defs, 0);
    let odd = mfix(&defs, 1);
    assert!(
        infer_type(&ctx, &even).is_ok(),
        "the mutual block must type-check"
    );
    assert_eq!(
        normalize(&ctx, &app(even, lit(4))),
        g("true"),
        "isEven 4 = true"
    );
    // `index` is the only thing that tells the two apart — both have type `Nat → Bool`, so a
    // wrong index is invisible to the type-checker and visible only by computing.
    assert_eq!(
        normalize(&ctx, &app(odd, lit(4))),
        g("false"),
        "isOdd 4 = false"
    );

    // Members whose structural argument sits at position 1, behind an accumulator.
    let from_body = |sibling: &str, bump: u32| {
        lam(
            "acc",
            nat(),
            lam(
                "n",
                nat(),
                match_(
                    var("n"),
                    lam("_", nat(), nat()),
                    vec![
                        var("acc"),
                        lam(
                            "k",
                            nat(),
                            apps(
                                var(sibling),
                                &[(0..bump).fold(var("acc"), |a, _| succ(a)), var("k")],
                            ),
                        ),
                    ],
                ),
            ),
        )
    };
    let from_defs = [
        ("evenFrom", from_body("oddFrom", 1)),
        ("oddFrom", from_body("evenFrom", 2)),
    ];
    let even_from = mfix(&from_defs, 0);
    assert!(
        infer_type(&ctx, &even_from).is_ok(),
        "a block whose members decrease at position 1 must be admitted"
    );
    assert_eq!(
        normalize(&ctx, &apps(even_from, &[lit(0), lit(1)])),
        succ(lit(0)),
        "evenFrom 0 1 = 1"
    );

    // THE FENCE: the mutual guard is what closes the hole two separate `Fix`es would leave. A
    // sibling call that passes the structural parameter UNCHANGED is an infinite loop and must
    // be refused by both kernels.
    let loop_a = lam(
        "n",
        nat(),
        match_(
            var("n"),
            bool_motive.clone(),
            vec![g("true"), lam("_k", nat(), app(var("loopB"), var("n")))],
        ),
    );
    let loop_b = lam(
        "n",
        nat(),
        match_(
            var("n"),
            bool_motive,
            vec![g("false"), lam("_k", nat(), app(var("loopA"), var("n")))],
        ),
    );
    let looping = mfix(&[("loopA", loop_a), ("loopB", loop_b)], 0);
    assert!(
        infer_type(&ctx, &looping).is_err(),
        "a sibling call that does not decrease must be a TerminationViolation"
    );
    assert!(
        recheck(&ctx, &looping).is_err(),
        "the second kernel must reject the non-decreasing mutual block independently"
    );

    // `Tree`/`Forest`: a block whose members recurse on genuinely DIFFERENT structural types, so
    // the guard must carry one `struct_type` per member. Their eliminators are one shared
    // `MutualFix`, which is what lets `treeSize` call `forestSize` on the sub-forest.
    let mut mctx = std_ctx();
    mctx.add_mutual_inductives(&[
        MutualInductive {
            name: "Tree".to_string(),
            sort: sort(0),
            num_params: 0,
            constructors: vec![("Node".to_string(), arrow(g("Forest"), g("Tree")))],
        },
        MutualInductive {
            name: "Forest".to_string(),
            sort: sort(0),
            num_params: 0,
            constructors: vec![
                ("FNil".to_string(), g("Forest")),
                (
                    "FCons".to_string(),
                    arrow(g("Tree"), arrow(g("Forest"), g("Forest"))),
                ),
            ],
        },
    ])
    .expect("the mutual block must register");
    let (rec_ty, rec_body) = derive_recursor(&mctx, "Tree").expect("Tree's recursor must derive");
    let mut core = &rec_body;
    while let Term::Lambda { body, .. } = core {
        core = body;
    }
    assert!(
        matches!(core, Term::MutualFix { defs, .. } if defs.len() == 2),
        "the two members share ONE MutualFix, so the guard sees both bodies: {rec_body}"
    );
    let inferred = infer_type(&mctx, &rec_body).expect("the mutual recursor must type-check");
    assert!(
        defeq_for_test(&mctx, &inferred, &rec_ty),
        "the derived mutual recursor's body must have its derived type"
    );
}

// ==== T04-004 · `termination_by` — a measure (MUST, Native) ====

/// Ackermann. Its measure is LEXICOGRAPHIC `(m, n)`: neither argument decreases on its own; the
/// outer call decreases `m`, and the inner call `ack (m+1) n` keeps `m` and decreases `n`. The guard
/// measures a fixpoint's inductive binders lexicographically, so it admits the recursion Lean
/// admits with `termination_by m n => (m, n)`, read off the recursion itself.
#[test]
fn a_lexicographic_measure_admits_a_recursion_no_single_argument_decreases() {
    let ctx = std_ctx();

    // ack = fix ack. λm:Nat. λn:Nat. match m return (λ_:Nat. Nat) with
    //         | Zero    => Succ n
    //         | Succ m' => match n return (λ_:Nat. Nat) with
    //             | Zero     => ack m' (Succ Zero)
    //             | Succ n'' => ack m' (ack (Succ m') n'')
    let ack = fix(
        "ack",
        lam(
            "m",
            nat(),
            lam(
                "n",
                nat(),
                match_(
                    var("m"),
                    lam("_", nat(), nat()),
                    vec![
                        succ(var("n")),
                        lam(
                            "m1",
                            nat(),
                            match_(
                                var("n"),
                                lam("_", nat(), nat()),
                                vec![
                                    apps(var("ack"), &[var("m1"), succ(g("Zero"))]),
                                    lam(
                                        "n2",
                                        nat(),
                                        apps(
                                            var("ack"),
                                            &[
                                                var("m1"),
                                                apps(var("ack"), &[succ(var("m1")), var("n2")]),
                                            ],
                                        ),
                                    ),
                                ],
                            ),
                        ),
                    ],
                ),
            ),
        ),
    );
    let r = infer_type(&ctx, &ack);
    assert!(
        r.is_ok(),
        "Ackermann is total, and the guard's lexicographic measure admits it as Lean's \
         `termination_by m n => (m, n)` does: {r:?}"
    );

    // The capability we are asking for must NOT be a hole: `fix f. λn. f n` stays refused.
    let diverge = fix("f", lam("n", nat(), app(var("f"), var("n"))));
    assert!(
        infer_type(&ctx, &diverge).is_err(),
        "admitting measures must never admit a bare non-decreasing loop"
    );
}

// ==== T04-005 · `decreasing_by` — the proof obligation (MUST, Native) ====

/// `downFrom n = if h : n = 0 then 0 else downFrom (n - 1) + 1`. The decrease `n - 1 < n` holds
/// only because of `h : ¬(n = 0)`, which the DEPENDENT branch puts in scope. The guard's
/// refutation refinement reads that refutation off the kernel-checked `decEqNat` and drops `Zero`
/// from the branch's subterm meet, so the recursion on `pred n` is admitted with the obligation
/// Lean's `decreasing_by` states discharged by the decision procedure itself.
#[test]
fn a_termination_proof_obligation_has_somewhere_to_be_discharged() {
    let mut ctx = std_ctx();
    // pred := λn:Nat. match n return (λ_:Nat. Nat) with | Zero => Zero | Succ k => k
    ctx.add_definition(
        "pred".to_string(),
        arrow(nat(), nat()),
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                lam("_", nat(), nat()),
                vec![g("Zero"), lam("k", nat(), var("k"))],
            ),
        ),
    );

    let is_zero = |n: Term| eq_nat(n, g("Zero"));
    // downFrom = fix f. λn:Nat. match decEqNat n Zero return Nat with
    //              | isTrue  hp => Zero
    //              | isFalse hn => Succ (f (pred n))
    let down_from = fix(
        "f",
        lam(
            "n",
            nat(),
            match_(
                apps(g("decEqNat"), &[var("n"), g("Zero")]),
                nat(),
                vec![
                    lam("hp", is_zero(var("n")), g("Zero")),
                    lam(
                        "hn",
                        app(g("Not"), is_zero(var("n"))),
                        succ(app(var("f"), app(g("pred"), var("n")))),
                    ),
                ],
            ),
        ),
    );
    let r = infer_type(&ctx, &down_from);
    assert!(
        r.is_ok(),
        "the `isFalse` branch binds `hn : ¬(n = 0)`, Lean's `decreasing_by` hypothesis, and the \
         refutation refinement admits the recursion on `pred n`: {r:?}"
    );

    // The anti-workaround the corpus names: padding the call with a "certificate" argument must
    // not let the guard inspect the certificate instead of the recursive argument. The structural
    // position is read off the λ-telescope, so a swapped call is still caught.
    let padded = fix(
        "f",
        lam(
            "cert",
            nat(),
            lam(
                "n",
                nat(),
                match_(
                    var("n"),
                    lam("_", nat(), nat()),
                    vec![
                        g("Zero"),
                        lam("k", nat(), apps(var("f"), &[var("k"), var("cert")])),
                    ],
                ),
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &padded).is_err(),
        "a certificate argument must not shift which argument the guard inspects"
    );
}

// ==== T04-006 · `Nat.rec` applied directly (MUST, Native) ====

/// The recursor is DERIVED here and primitive in Lean — the inverse of Lean 4, and defensible.
/// Lean's `Nat.rec` is `{motive : Nat → Sort u}`. The monomorphic `derive_recursor` targets the
/// inductive's own sort by design (a `Prop` motive still passes by cumulativity), so the LARGE end
/// of the range — where `tower : Nat → Type 1` lives — is reached through
/// `derive_recursor_universe_poly`, whose motive carries a level of its own.
#[test]
fn a_derived_nat_recursor_iota_computes_and_its_polymorphic_form_reaches_type_one() {
    let mut ctx = std_ctx();
    let (rec_ty, rec_body) = derive_recursor(&ctx, "Nat").expect("Nat's eliminator must derive");
    ctx.add_definition("Nat_rec".to_string(), rec_ty, rec_body);

    // addRec m n = Nat_rec (λ_:Nat. Nat) m (λ_:Nat. λih:Nat. Succ ih) n — the `Type 0` motive,
    // which must ι-compute (the derived recursor is an ordinary delta-transparent definition,
    // so it unfolds rather than staying in recursor form).
    let add_rec = apps(
        g("Nat_rec"),
        &[
            lam("_", nat(), nat()),
            lit(5),
            lam("_", nat(), lam("ih", nat(), succ(var("ih")))),
            lit(2),
        ],
    );
    assert!(
        infer_type(&ctx, &add_rec).is_ok(),
        "the Type 0 motive must type-check"
    );
    assert_eq!(
        normalize(&ctx, &add_rec),
        succ(succ(lit(5))),
        "Nat_rec must ι-compute: 5 + 2 = 7"
    );

    // towerRec : Type 1 built by recursion — `Nat.rec (motive := fun _ => Type) Nat …`. The motive
    // `λ_:Nat. Type 0` inhabits `Nat → Type 1`. Lean's `Nat.rec` is universe-polymorphic in its
    // motive, so a `Type 1`-valued family built by recursion is ordinary mathematics — and the
    // polymorphic eliminator is its counterpart.
    let (_, poly_ty, poly_body) = derive_recursor_universe_poly(&ctx, "Nat")
        .expect("Nat's polymorphic eliminator must derive");
    assert_eq!(
        double_check(&ctx, &poly_body),
        DoubleCheck::Agreed,
        "both kernels certify the polymorphic eliminator"
    );
    ctx.add_definition("NatRecU".to_string(), poly_ty, poly_body);
    let tower_rec_u = apps(
        g("NatRecU"),
        &[
            lam("_", nat(), sort(0)),
            nat(),
            lam("_", nat(), lam("ih", sort(0), var("ih"))),
            lit(2),
        ],
    );
    let r = infer_type(&ctx, &tower_rec_u);
    assert!(
        r.is_ok(),
        "`Nat.rec (motive := fun _ => Type) Nat (fun _ ih => ih) 2` must type-check through the \
         polymorphic eliminator: {r:?}"
    );
    assert_eq!(
        normalize(&ctx, &tower_rec_u),
        nat(),
        "…and compute: two steps of the identity"
    );

    // The level is solved, not waived: a step that returns a `Nat` where the motive asks for a
    // `Type 0` is refused.
    let mistyped_step = apps(
        g("NatRecU"),
        &[
            lam("_", nat(), sort(0)),
            nat(),
            lam("_", nat(), lam("ih", sort(0), lit(0))),
            lit(2),
        ],
    );
    assert!(
        infer_type(&ctx, &mistyped_step).is_err(),
        "a step of the wrong type is refused at the polymorphic instance too"
    );
}

// ==== T04-007 · several discriminants (MUST, Native) ====

/// `Term::Match` has ONE discriminant, so a Lean multi-column match becomes a nest of unary
/// ones. Two things the corpus warns about turn out NOT to bite: the outer motive may be an
/// ARROW that still abstracts the remaining scrutinees, and `extract_structural_param` picks
/// whichever binder the outermost match discriminates on — so both nesting orders of `zipLen`
/// are admitted and compute the same function.
#[test]
fn a_multi_column_match_nests_into_unary_matches_in_either_scrutinee_order() {
    let ctx = std_ctx();
    let bool_ = || g("Bool");
    let bb = |t: Term, f: Term| match_(var("b"), lam("_", bool_(), bool_()), vec![t, f]);

    // eqBool: the 2×2 grid, nested with `b` already λ-bound.
    let eq_bool = lam(
        "a",
        bool_(),
        lam(
            "b",
            bool_(),
            match_(
                var("a"),
                lam("_", bool_(), bool_()),
                vec![bb(g("true"), g("false")), bb(g("false"), g("true"))],
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &eq_bool).is_ok(),
        "the nested encoding must type-check"
    );
    for (a, b, want) in [
        ("true", "true", "true"),
        ("true", "false", "false"),
        ("false", "true", "false"),
        ("false", "false", "true"),
    ] {
        assert_eq!(
            normalize(&ctx, &apps(eq_bool.clone(), &[g(a), g(b)])),
            g(want),
            "eqBool {a} {b} = {want}"
        );
    }

    // The scrutinee-abstracting shape: the OUTER motive is `λ_:Bool. Bool → Bool`, so each outer
    // case is itself a function of the second scrutinee. This is the encoding a translator that
    // lifts the columns must emit, and it must type-check too.
    let inner = |t: Term, f: Term| {
        lam(
            "b",
            bool_(),
            match_(var("b"), lam("_", bool_(), bool_()), vec![t, f]),
        )
    };
    let eq_bool_lifted = lam(
        "a",
        bool_(),
        match_(
            var("a"),
            lam("_", bool_(), arrow(bool_(), bool_())),
            vec![inner(g("true"), g("false")), inner(g("false"), g("true"))],
        ),
    );
    assert!(
        infer_type(&ctx, &eq_bool_lifted).is_ok(),
        "the outer motive must be allowed to abstract the remaining scrutinees"
    );
    assert_eq!(
        normalize(&ctx, &apps(eq_bool_lifted, &[g("false"), g("false")])),
        g("true"),
        "the lifted encoding computes the same function"
    );

    // zipLen under recursion, both nesting orders. The list `[1,2,3]` against `[4,5]` is
    // asymmetric, so a swapped nesting that changed the denotation would show. The recursive
    // call always passes (xs-tail, ys-tail); which of those is the OUTER match's binder flips
    // with the nesting, and the guard follows it because `extract_structural_param` reads the
    // structural parameter off the outermost match's own scrutinee rather than assuming
    // argument 0.
    let zip = |outer: &str, inner_v: &str, call: [&str; 2]| {
        fix(
            "f",
            lam(
                "xs",
                tlist_nat(),
                lam(
                    "ys",
                    tlist_nat(),
                    match_(
                        var(outer),
                        lam("_", tlist_nat(), nat()),
                        vec![
                            g("Zero"),
                            lam(
                                "h1",
                                nat(),
                                lam(
                                    "t1",
                                    tlist_nat(),
                                    match_(
                                        var(inner_v),
                                        lam("_", tlist_nat(), nat()),
                                        vec![
                                            g("Zero"),
                                            lam(
                                                "h2",
                                                nat(),
                                                lam(
                                                    "t2",
                                                    tlist_nat(),
                                                    succ(apps(
                                                        var("f"),
                                                        &[var(call[0]), var(call[1])],
                                                    )),
                                                ),
                                            ),
                                        ],
                                    ),
                                ),
                            ),
                        ],
                    ),
                ),
            ),
        )
    };
    let three = tcons(lit(1), tcons(lit(2), tcons(lit(3), tnil())));
    let two = tcons(lit(4), tcons(lit(5), tnil()));
    for (outer, inner_v, call) in [("xs", "ys", ["t1", "t2"]), ("ys", "xs", ["t2", "t1"])] {
        let z = zip(outer, inner_v, call);
        assert!(
            infer_type(&ctx, &z).is_ok(),
            "zipLen nested outer-on-{outer} must be admitted — the guard records the structural \
             parameter from the outermost match's own scrutinee"
        );
        assert_eq!(
            normalize(&ctx, &apps(z, &[three.clone(), two.clone()])),
            peano(2),
            "zipLen [1,2,3] [4,5] = 2 whichever column is nested outermost"
        );
    }
}

// ==== T04-008 · nested constructor patterns (MUST, Native) ====

/// `| 0 => 10 | 1 => 20 | n+2 => n` is three equations over a two-constructor inductive: a TREE
/// of unary positional matches, one layer per constructor the pattern inspected. The tree is
/// buildable and computes, and the pattern surface (`Consider x: When C p…:`) BUILDS it: nested
/// constructor patterns compile to exactly the tree this test writes by hand, and — as in Lean —
/// a set of equations that leaves a constructor uncovered is refused, naming the missing case.
#[test]
fn nested_constructor_patterns_compile_to_a_tree_of_positional_matches() {
    let ctx = std_ctx();

    // classify = λn. match n with | Zero => 10 | Succ m => match m with | Zero => 20 | Succ k => k
    let classify = lam(
        "n",
        nat(),
        match_(
            var("n"),
            lam("_", nat(), nat()),
            vec![
                lit(10),
                lam(
                    "m",
                    nat(),
                    match_(
                        var("m"),
                        lam("_", nat(), nat()),
                        vec![lit(20), lam("k", nat(), var("k"))],
                    ),
                ),
            ],
        ),
    );
    assert!(
        infer_type(&ctx, &classify).is_ok(),
        "the two-level match tree must type-check"
    );
    for (input, want) in [(0u64, lit(10)), (1, lit(20)), (5, lit(3))] {
        assert_eq!(
            normalize(&ctx, &app(classify.clone(), lit(input))),
            want,
            "classify {input}"
        );
    }
    // Dropping the inner layer's exhaustiveness is caught: `cases.len()` must equal the
    // constructor count at EVERY node, not just the outermost one.
    let inner_short = lam(
        "n",
        nat(),
        match_(
            var("n"),
            lam("_", nat(), nat()),
            vec![
                lit(10),
                lam(
                    "m",
                    nat(),
                    match_(
                        var("m"),
                        lam("_", nat(), nat()),
                        vec![lam("k", nat(), var("k"))],
                    ),
                ),
            ],
        ),
    );
    assert!(
        infer_type(&ctx, &inner_short).is_err(),
        "an inner match with one case for a two-constructor inductive must be refused"
    );

    // THE CAPABILITY. The case's three equations, written as nested patterns, compile to the
    // tree above.
    let mut repl = Repl::new();
    let r = repl.execute(
        "## To classify (n: Nat) -> Nat: Consider n: When Zero: Yield 10. \
         When Succ Zero: Yield 20. When Succ (Succ k): Yield k.",
    );
    assert!(
        r.is_ok(),
        "LOGOS must accept a NESTED constructor pattern and compile it to the match tree this \
         test builds by hand: {r:?}"
    );
    let compiled = repl
        .context()
        .get_definition_body("classify")
        .cloned()
        .expect("the compiled definition is registered with its body");
    assert!(
        defeq_for_test(&ctx, &compiled, &classify),
        "the equations compile to exactly the hand-built tree: {compiled}"
    );
    for (input, want) in [(0u64, "10"), (1, "20"), (5, "3")] {
        assert_eq!(
            repl.execute(&format!("Eval classify {input}."))
                .unwrap()
                .trim(),
            want,
            "the compiled classify {input}"
        );
    }

    // …and equations that leave a constructor uncovered are refused, as Lean refuses them
    // ("missing cases: (Nat.succ Nat.zero)"), naming the case and registering nothing.
    let partial = repl.execute(
        "## To partialClassify (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ (Succ k): Yield k.",
    );
    assert!(
        matches!(&partial, Err(e) if e.to_string().contains("not exhaustive")
            && e.to_string().contains("Succ Zero")),
        "a non-exhaustive set of equations must be refused, naming `Succ Zero`: {partial:?}"
    );
    assert!(
        repl.context().get_global("partialClassify").is_none(),
        "the refused definition registers nothing"
    );
}

// ==== T04-009 · overlapping patterns, first-match semantics (MUST, Native) ====

/// `pick 0 _ = 1 | _ 0 = 2 | _ _ = 3`. Three equations, two constructors, and `pick 0 0` is
/// matched by two of them — Lean takes the first. `cases` is indexed by CONSTRUCTOR, so N
/// overlapping equations cannot be represented as N cases, and there is no notion of order among
/// alternatives for one constructor: the equation compiler splits the overlap into the disjoint
/// tree Lean's produces. The tree is built by hand here and computes with first-match semantics,
/// one case per equation is refused, and overlapping equations written through `Consider` are
/// accepted. The two wrong splits (keep the first equation per constructor, or keep the last)
/// would both yield well-typed terms that are silently a different function.
#[test]
fn overlapping_equations_must_be_split_into_disjoint_constructor_cases() {
    let ctx = std_ctx();

    // The compiled target: match on the first column, and only in its `Zero` branch is the
    // second column irrelevant.
    //   pick = λa. λb. match a with
    //            | Zero   => 1
    //            | Succ _ => match b with | Zero => 2 | Succ _ => 3
    let pick = lam(
        "a",
        nat(),
        lam(
            "b",
            nat(),
            match_(
                var("a"),
                lam("_", nat(), nat()),
                vec![
                    lit(1),
                    lam(
                        "_a",
                        nat(),
                        match_(
                            var("b"),
                            lam("_", nat(), nat()),
                            vec![lit(2), lam("_b", nat(), lit(3))],
                        ),
                    ),
                ],
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &pick).is_ok(),
        "the disjoint tree must type-check"
    );
    for (a, b, want) in [(0u64, 0u64, 1u64), (0, 5, 1), (5, 0, 2), (5, 5, 3)] {
        assert_eq!(
            normalize(&ctx, &apps(pick.clone(), &[lit(a), lit(b)])),
            lit(want),
            "first-match semantics: pick {a} {b} = {want}"
        );
    }

    // One case per EQUATION is refused when the counts differ — this is the visible half of the
    // hazard. (`bucket : | 0 => 100 | 1 => 200 | _ => 300` is three equations over `Nat`.)
    let bucket_flat = lam(
        "n",
        nat(),
        match_(
            var("n"),
            lam("_", nat(), nat()),
            vec![lit(100), lit(200), lit(300)],
        ),
    );
    assert!(
        infer_type(&ctx, &bucket_flat).is_err(),
        "three cases for a two-constructor inductive must be WrongNumberOfCases"
    );

    // THE CAPABILITY. Overlapping equations with top-to-bottom first-match order must be
    // writable, and the splitting must be done by the compiler — never by the author, and never
    // by dropping the equations that do not fit the case vector.
    let mut repl = Repl::new();
    let r = repl.execute(
        "## To pick (a: Nat) and (b: Nat) -> Nat: Consider a: When Zero: Yield 1. \
         When Zero: Yield 2. When Succ k: Yield 3.",
    );
    assert!(
        r.is_ok(),
        "LOGOS must accept overlapping equations and resolve them top-to-bottom the way Lean \
         does; `Consider` has one clause per constructor and no ordering at all: {r:?}"
    );
}

// ==== T04-010 · the wildcard `_` (MUST, Native) ====

/// Two requirements meet here.
///
/// (a) A wildcard `| _ => e` expands into one case per remaining constructor, with the body
///     replicated and re-abstracted over each constructor's fields. The expansion written by hand
///     agrees with Lean on every constructor, one that misses a constructor is refused, and
///     `When _:` is fanned out over the remaining constructors by the equation compiler.
/// (b) `Consider`'s `When <ctor>` puts its clause in THAT constructor's slot, whatever the source
///     order. For an inductive whose constructors have equal arity — every enumeration, and
///     `Bool` itself — clauses taken in source order would silently compute a different
///     function: the wildcard-first shape of the corpus's `isNil` hazard.
#[test]
fn a_catch_all_expands_over_the_remaining_constructors_and_case_order_is_not_source_order() {
    let ctx = std_ctx();

    // (a) The hand-expanded target over a five-constructor enumeration.
    let mut cctx = ctx.clone();
    cctx.add_inductive("Color", sort(0));
    for c in ["Red", "Green", "Blue", "Cyan", "Magenta"] {
        cctx.add_constructor(c, "Color", g("Color"));
    }
    let is_red = lam(
        "c",
        g("Color"),
        match_(
            var("c"),
            lam("_", g("Color"), g("Bool")),
            vec![g("true"), g("false"), g("false"), g("false"), g("false")],
        ),
    );
    assert!(
        infer_type(&cctx, &is_red).is_ok(),
        "the expanded catch-all must type-check"
    );
    for (c, want) in [
        ("Red", "true"),
        ("Green", "false"),
        ("Blue", "false"),
        ("Cyan", "false"),
        ("Magenta", "false"),
    ] {
        assert_eq!(
            normalize(&cctx, &app(is_red.clone(), g(c))),
            g(want),
            "the expansion must agree with Lean on EVERY constructor, not only the enumerated one"
        );
    }
    // An expansion that misses a constructor is caught by the arity check — the loud half.
    let short = lam(
        "c",
        g("Color"),
        match_(
            var("c"),
            lam("_", g("Color"), g("Bool")),
            vec![g("true"), g("false")],
        ),
    );
    assert!(
        infer_type(&cctx, &short).is_err(),
        "two cases for five constructors must be WrongNumberOfCases"
    );

    // (b) THE SILENT HALF. `Bool` is registered `true` then `false`. Writing the clauses the
    // other way round must not silently produce the other function.
    assert_eq!(
        ctx.get_constructors("Bool")
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>(),
        vec!["true", "false"],
        "`Bool`'s registration order is what a positional case vector is zipped against"
    );
    let mut repl = Repl::new();
    repl.execute(
        "## To flip (b: Bool) -> Bool: Consider b: When false: Yield true. When true: Yield false.",
    )
    .expect("the definition must be accepted (the constructor names are well-formed)");
    let flipped = repl.execute("Eval flip true.").expect("flip must evaluate");
    let order_honoured = flipped.contains("false");

    // …and the catch-all itself must be writable, on an inductive where it genuinely has to fan
    // out: three constructors, two clauses. (On `Bool` a trailing `_` happens to land in the
    // one remaining slot, which is coincidence, not expansion.)
    repl.execute("A Hue is either Ruby or Jade or Slate.")
        .expect("the literate inductive must register");
    let catch_all = repl.execute(
        "## To isRuby (h: Hue) -> Bool: Consider h: When Ruby: Yield true. When _: Yield false.",
    );

    assert!(
        order_honoured && catch_all.is_ok(),
        "two holes, reported together. (1) `When <ctor>` must put the clause in THAT \
         constructor's slot — the name is parsed and thrown away, so clauses land in SOURCE \
         order and `flip true` returned {flipped:?} instead of `false`, a silent wrong answer \
         for every equal-arity inductive. (2) a `_` catch-all must be accepted and fanned out \
         over the remaining constructors: {catch_all:?}"
    );
}

// ==== T04-011 · dependent `match` with an explicit motive (MUST, Native) ====

/// A genuinely type-valued family built by matching, eliminated at a dependent motive. The
/// motive is a mandatory field of every `Match`, and a `Term::Hole` in it is `CannotInferHole`
/// at the kernel boundary — inference is the elaborator's job, and it is Miller-pattern only,
/// so a discriminant that is not a bare variable needs the motive supplied. Both shapes work.
#[test]
fn a_dependent_match_eliminates_at_a_type_valued_family() {
    let mut ctx = std_ctx();
    // Carrier : Bool → Type 0 := λb. match b return Type 0 with | true => Nat | false => Bool
    ctx.add_definition(
        "Carrier".to_string(),
        arrow(g("Bool"), sort(0)),
        lam(
            "b",
            g("Bool"),
            match_(var("b"), sort(0), vec![nat(), g("Bool")]),
        ),
    );
    let carrier = |t: Term| app(g("Carrier"), t);

    // mkCarrier : Π(b:Bool). Carrier b — the branches have DIFFERENT types.
    let mk = lam(
        "b",
        g("Bool"),
        match_(
            var("b"),
            lam("x", g("Bool"), carrier(var("x"))),
            vec![g("Zero"), g("false")],
        ),
    );
    let ty = infer_type(&ctx, &mk).expect("the dependent match must type-check");
    assert!(
        defeq_for_test(&ctx, &ty, &pi("b", g("Bool"), carrier(var("b")))),
        "mkCarrier : Π(b:Bool). Carrier b, got {ty}"
    );
    assert_eq!(
        normalize(&ctx, &app(mk.clone(), g("true"))),
        g("Zero"),
        "mkCarrier true : Nat"
    );
    assert_eq!(
        normalize(&ctx, &app(mk, g("false"))),
        g("false"),
        "mkCarrier false : Bool"
    );

    // A discriminant that is NOT a bare variable — the Miller condition fails, so the motive
    // must be supplied rather than inferred. With it supplied, the kernel is content.
    let not_var = lam(
        "f",
        arrow(nat(), g("Bool")),
        lam(
            "n",
            nat(),
            match_(
                app(var("f"), var("n")),
                lam("x", g("Bool"), carrier(var("x"))),
                vec![g("Zero"), g("false")],
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &not_var).is_ok(),
        "an explicit motive must work over a non-variable discriminant"
    );

    // A `Hole` motive must never reach a declaration: the kernel refuses it outright.
    let holed = match_(g("true"), Term::Hole, vec![g("Zero"), g("false")]);
    assert!(
        infer_type(&ctx, &holed).is_err(),
        "a Hole motive is CannotInferHole — motive inference belongs to the elaborator"
    );
}

// ==== T04-012 · `match h : e with` (MUST, Native) ====

/// The corpus calls this unrepresentable. It is not: the CONVOY encoding works. Generalise the
/// motive over the scrutinee, give every case the equation as an extra binder, and apply the
/// whole match to `refl` — `(match e return (λx. Eq T e x → C x) with …) (refl T e)`. Each case
/// then receives a proof at ITS OWN constructor, which is exactly what Lean's `match h : e with`
/// binds, and the proof is load-bearing: the `Zero` case below uses it to build an existential
/// witness that could not be built without it.
#[test]
fn the_match_equation_hypothesis_is_available_through_the_convoy_motive() {
    let ctx = std_ctx();
    // P := λz:Nat. Eq Nat n z   (with `n` the outer binder)
    let p = || lam("z", nat(), eq_nat(var("n"), var("z")));
    let ex_p = || apps(g("Ex"), &[nat(), p()]);

    // witnessOf = λn:Nat.
    //   (match n return (λx:Nat. Eq Nat n x → Ex Nat P) with
    //      | Zero   => λh : Eq Nat n Zero.     witness Nat P Zero h
    //      | Succ k => λh : Eq Nat n (Succ k). witness Nat P (Succ k) h)
    //   (refl Nat n)
    let body = match_(
        var("n"),
        lam("x", nat(), arrow(eq_nat(var("n"), var("x")), ex_p())),
        vec![
            lam(
                "h",
                eq_nat(var("n"), g("Zero")),
                apps(g("witness"), &[nat(), p(), g("Zero"), var("h")]),
            ),
            lam(
                "k",
                nat(),
                lam(
                    "h",
                    eq_nat(var("n"), succ(var("k"))),
                    apps(g("witness"), &[nat(), p(), succ(var("k")), var("h")]),
                ),
            ),
        ],
    );
    let witness_of = lam("n", nat(), app(body, apps(g("refl"), &[nat(), var("n")])));

    let ty = infer_type(&ctx, &witness_of).expect(
        "the convoy encoding must type-check: each case receives `Eq Nat n <its constructor>`, \
         the equation hypothesis `match h : e with` binds",
    );
    assert!(
        defeq_for_test(&ctx, &ty, &pi("n", nat(), ex_p().clone())),
        "witnessOf : Π(n:Nat). Ex Nat (λz. Eq Nat n z), got {ty}"
    );
    match double_check(&ctx, &witness_of) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify the convoy encoding, got {other:?}"),
    }

    // The hypothesis is genuinely per-constructor: dropping it (a constant motive) leaves the
    // `Zero` case unable to build `P Zero`, because there is no proof that `n` IS `Zero`.
    let dropped = lam(
        "n",
        nat(),
        match_(
            var("n"),
            lam("_", nat(), ex_p()),
            vec![
                apps(g("witness"), &[nat(), p(), g("Zero"), g("I")]),
                lam(
                    "k",
                    nat(),
                    apps(g("witness"), &[nat(), p(), succ(var("k")), g("I")]),
                ),
            ],
        ),
    );
    assert!(
        infer_type(&ctx, &dropped).is_err(),
        "dropping the equation hypothesis must not be patchable with an unrelated proof"
    );
}

// ==== T04-013 · incomplete match (MUST, Native) ====

/// Lean accepts `vhead : Vect α (n+1) → α` with ONE branch, because index unification rules
/// `nil` out. `infer_indexed_match` owes a branch only for the constructors the discriminant's
/// indices allow (`constructors_the_indices_allow`, which rules a constructor out only on a
/// definite clash), so the one-branch `vhead` checks, and the `VNil` case, which would have to
/// inhabit an arbitrary `α`, is never asked for. A `Hole` padding the case vector stays refused,
/// and `ex falso` (zero constructors, zero cases) keeps working.
#[test]
fn an_index_refined_match_needs_no_branch_that_cannot_be_inhabited() {
    let mut ctx = std_ctx();
    register_vect(&mut ctx);

    let vect = |a: Term, n: Term| apps(g("Vect"), &[a, n]);
    // vhead : Π(a:Type 0). Π(n:Nat). Vect a (Succ n) → a, with only the `VCons` branch.
    let vhead_one_case = lam(
        "a",
        sort(0),
        lam(
            "n",
            nat(),
            lam(
                "v",
                vect(var("a"), succ(var("n"))),
                match_(
                    var("v"),
                    lam("i", nat(), lam("_", vect(var("a"), var("i")), var("a"))),
                    vec![lam(
                        "m",
                        nat(),
                        lam("x", var("a"), lam("_t", vect(var("a"), var("m")), var("x"))),
                    )],
                ),
            ),
        ),
    );
    let r = infer_type(&ctx, &vhead_one_case);
    assert!(
        r.is_ok(),
        "the index `Succ n` rules `VNil` out, so Lean's one-branch `vhead` is total. LOGOS \
         demands a `VNil` case that would have to inhabit an arbitrary `α`: {r:?}"
    );

    // Padding with a `Hole` to make the arity check pass must stay refused: `check_type` accepts
    // a `Hole` only where a Sort is expected, and the missing branch's type is the variable `a`.
    let vhead_holed = lam(
        "a",
        sort(0),
        lam(
            "n",
            nat(),
            lam(
                "v",
                vect(var("a"), succ(var("n"))),
                match_(
                    var("v"),
                    lam("i", nat(), lam("_", vect(var("a"), var("i")), var("a"))),
                    vec![
                        Term::Hole,
                        lam(
                            "m",
                            nat(),
                            lam("x", var("a"), lam("_t", vect(var("a"), var("m")), var("x"))),
                        ),
                    ],
                ),
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &vhead_holed).is_err(),
        "padding the case vector with a Hole must never be a way past the arity check"
    );

    // The over-rejection fence: a zero-constructor inductive genuinely HAS zero cases, so
    // `ex falso` must keep working — `cases.len() == 0 == ctor count`.
    let ex_falso = lam(
        "a",
        sort(0),
        lam("h", g("False"), match_(var("h"), var("a"), Vec::new())),
    );
    assert!(
        infer_type(&ctx, &ex_falso).is_ok(),
        "`False.elim` must not be collateral damage of the coverage rule"
    );
}

// ==== T04-014 · impossible-case discharge (MUST, Native) ====

/// The mirror of T04-013, where a filler IS constructible. Lean omits `VCons` from a match on
/// `Vect α 0` because `0` cannot unify with `n+1`, and so does LOGOS. Supplying the unreachable
/// branch anyway stays legal: `compute_indexed_case_type` gives it a well-formed type at the
/// CONSTRUCTOR's own result indices, so a filler type-checks and is dead weight.
#[test]
fn a_constructor_the_indices_rule_out_needs_no_synthesised_filler() {
    let mut ctx = std_ctx();
    register_vect(&mut ctx);
    let vect = |a: Term, n: Term| apps(g("Vect"), &[a, n]);
    let motive = || lam("i", nat(), lam("_", vect(nat(), var("i")), nat()));

    // The FULL vector with a synthesised filler for the impossible `VCons` branch type-checks —
    // so the case's expected type is well-formed and the kernel is not the obstacle.
    let vnil_filled = lam(
        "v",
        vect(nat(), g("Zero")),
        match_(
            var("v"),
            motive(),
            vec![
                g("Zero"),
                lam(
                    "m",
                    nat(),
                    lam("x", nat(), lam("_t", vect(nat(), var("m")), var("x"))),
                ),
            ],
        ),
    );
    assert!(
        infer_type(&ctx, &vnil_filled).is_ok(),
        "a filler at the constructor's own result indices must type-check"
    );
    assert_eq!(
        normalize(&ctx, &app(vnil_filled, app(g("VNil"), nat()))),
        g("Zero"),
        "the reachable branch computes; the filler is dead weight"
    );

    // THE CAPABILITY: with index unification the `VCons` branch is not written at all.
    let vnil_only = lam(
        "v",
        vect(nat(), g("Zero")),
        match_(var("v"), motive(), vec![g("Zero")]),
    );
    let r = infer_type(&ctx, &vnil_only);
    assert!(
        r.is_ok(),
        "`0` cannot unify with `Succ n`, so Lean's one-branch match on `Vect α 0` is exhaustive. \
         `infer_indexed_match` has no index unifier and demands the unreachable case: {r:?}"
    );
}

// ==== T04-015 · well-founded recursion (MUST, Native) ====

/// `Acc` is a registered indexed inductive with nested strict positivity, `Acc_rec` a derived
/// dependent eliminator carrying the FUNCTIONAL induction hypothesis, `WellFounded A R :=
/// Π(x:A). Acc A R x`, and the guard's applied-smaller rule admits recursion through an
/// accessibility proof in both kernels. The test inhabits `WellFounded Nat R` for the empty
/// relation, feeds it to `Acc_rec`, and ι-COMPUTES the result — not `.is_ok()`, an actual value,
/// certified by both kernels — and `WellFounded_fix`, the combinator every Lean well-founded
/// definition goes through (`WellFounded.fix`), is the prelude's, defined by `Acc_rec`.
#[test]
fn a_well_founded_recursion_over_acc_computes_end_to_end() {
    let ctx = std_ctx();
    // R := λ_:Nat. λ_:Nat. False — the empty relation. Every `Nat` is accessible under it, and
    // the accessibility proof is constructible without any arithmetic.
    let rel = lam("_a", nat(), lam("_b", nat(), g("False")));
    let acc_at = |x: Term| apps(g("Acc"), &[nat(), rel.clone(), x]);

    // accAll : Π(x:Nat). Acc Nat R x
    //   := λx. Acc_intro Nat R x (λy. λhr : R y x. match hr return (Acc Nat R y) with ⟨⟩)
    let acc_all = lam(
        "x",
        nat(),
        apps(
            g("Acc_intro"),
            &[
                nat(),
                rel.clone(),
                var("x"),
                lam(
                    "y",
                    nat(),
                    lam(
                        "hr",
                        apps(rel.clone(), &[var("y"), var("x")]),
                        match_(var("hr"), acc_at(var("y")), Vec::new()),
                    ),
                ),
            ],
        ),
    );
    let wf_ty = infer_type(&ctx, &acc_all).expect("the accessibility proof must type-check");
    assert!(
        defeq_for_test(&ctx, &wf_ty, &apps(g("WellFounded"), &[nat(), rel.clone()])),
        "accAll inhabits `WellFounded Nat R` — the abbreviation Π(x:A). Acc A R x, got {wf_ty}"
    );

    // depth := Acc_rec Nat R (λx. λ_. Nat) (λx. λh. λih. Succ Zero) — a recursion whose step
    // receives the functional induction hypothesis, applied to a real proof, and COMPUTED.
    let motive = lam("x", nat(), lam("_", acc_at(var("x")), nat()));
    let h_ty = |x: Term| {
        pi(
            "y",
            nat(),
            arrow(apps(rel.clone(), &[var("y"), x]), acc_at(var("y"))),
        )
    };
    let ih_ty = |x: Term| {
        pi(
            "y",
            nat(),
            pi("hr", apps(rel.clone(), &[var("y"), x]), nat()),
        )
    };
    let step = lam(
        "x",
        nat(),
        lam(
            "h",
            h_ty(var("x")),
            lam("ih", ih_ty(var("x")), succ(g("Zero"))),
        ),
    );
    let applied = apps(
        g("Acc_rec"),
        &[
            nat(),
            rel.clone(),
            motive,
            step,
            lit(3),
            app(acc_all, lit(3)),
        ],
    );
    assert!(
        infer_type(&ctx, &applied).is_ok(),
        "the Acc_rec application must type-check"
    );
    assert_eq!(
        normalize(&ctx, &applied),
        peano(1),
        "well-founded recursion through `Acc_rec` must ι-COMPUTE, not merely type-check"
    );
    match double_check(&ctx, &applied) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify the Acc recursion, got {other:?}"),
    }

    // THE COMBINATOR. `WellFounded.fix` is what every Lean well-founded definition goes through:
    // it turns a well-foundedness proof into a fixpoint.
    let r = infer_type(&ctx, &g("WellFounded_fix"));
    assert!(
        r.is_ok(),
        "`WellFounded_fix : Π(A)(R)(C). WellFounded A R → (Π(x). (Π(y). R y x → C y) → C x) → \
         Π(x). C x` must exist — `Acc_rec` alone leaves every caller to reconstruct it: {r:?}"
    );
}

// ==== T04-016 · `brecOn` / `below` (MUST, Native) ====

/// Declaring an inductive derives its dependent eliminator `I_rec` and, beside it, Lean's
/// course-of-values pair: `I_below` packages every subterm reachable below `x`, and `I_brecOn`
/// recurses over it (`recursor::derive_below`, `recursor::derive_brec_on`). `brecOn` is what
/// Lean's equation compiler emits for every `| n+2 =>` definition; without it each would have to
/// be re-expressed by hand as a different definition.
#[test]
fn declaring_an_inductive_must_also_give_a_course_of_values_recursor() {
    let mut repl = Repl::new();
    repl.execute("Inductive Tally := Nought : Tally | Tick : Tally -> Tally.")
        .expect("the inductive must register");

    // What we DO generate, automatically, on the user-facing path.
    let derived = repl.execute("Check Tally_rec.");
    assert!(
        derived.is_ok(),
        "declaring an inductive must give its dependent eliminator for free: {derived:?}"
    );

    // The course-of-values pair. `below` packages "every subterm reachable below x"; `brecOn` is
    // the recursion over it. Without them, course-of-values recursion has to be re-expressed by
    // hand as a mutual block, which is a different definition, not a translation.
    for name in ["Tally_below", "Tally_brecOn"] {
        let r = repl.execute(&format!("Check {name}."));
        assert!(
            r.is_ok(),
            "`{name}` must be derivable alongside `Tally_rec` — `derive_recursor` emits only the \
             plain eliminator, so every course-of-values definition is stranded: {r:?}"
        );
    }
}

// ==== T04-017 · equation lemmas (MUST, Native) ====

/// Lean generates `f.eq_def` and `f.eq_1 … f.eq_n` as real, citable constants; `rw [f]`,
/// `simp [f]` and `unfold f` are how a proof appeals to a definition's behaviour. LOGOS generates
/// them for every definition (`equations::generate`, spelled `f_eq_def` and `f_eq_1 … f_eq_n`),
/// each a checked theorem. The content also holds definitionally — `add_definition` is
/// delta-transparent and `Fix`/`Match` ι-reduce — closed and at a free argument; the lemma is the
/// NAME a proof cites.
#[test]
fn a_definitions_equations_must_be_citable_and_not_only_definitional() {
    let mut ctx = std_ctx();
    let dbl_body = fix(
        "dbl",
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                lam("_", nat(), nat()),
                vec![
                    g("Zero"),
                    lam("k", nat(), succ(succ(app(var("dbl"), var("k"))))),
                ],
            ),
        ),
    );
    ctx.add_definition("dbl".to_string(), arrow(nat(), nat()), dbl_body);

    // The content: both equations hold DEFINITIONALLY, closed and open.
    assert!(
        defeq_for_test(&ctx, &app(g("dbl"), lit(3)), &peano(6)),
        "dbl 3 ≡ 6 by delta + iota"
    );
    let open_ctx = ctx.extend("n", nat());
    assert!(
        defeq_for_test(
            &open_ctx,
            &app(g("dbl"), succ(var("n"))),
            &succ(succ(app(g("dbl"), var("n"))))
        ),
        "dbl (n+1) ≡ dbl n + 2 holds for a FREE n — the `eq_2` content, with no lemma to name it"
    );

    // The name a proof that rewrites by one equation cites.
    let r = infer_type(&ctx, &g("dbl_eq_def"));
    assert!(
        r.is_ok(),
        "defining `dbl` must also generate its equation lemma(s); a Lean proof that says \
         `rw [dbl]` has no LOGOS counterpart to name: {r:?}"
    );
}

// ==== T04-018 · recursion Lean accepts that the Giménez guard rejects (MUST, Native) ====

/// `halve`/`fib`/`pairSum` — every `| n+2 => …` definition in every library. Lean compiles them
/// structurally through `brecOn`, which hands the body every subterm at once. LOGOS's guard reads
/// them directly: a `match` on a variable already known smaller contributes its own constructor
/// fields as smaller still, so the `k` of the inner `match m` is a subterm of a subterm of `n`,
/// and a `let` whose value is a smaller variable inherits its smallness (`termination.rs`), so a
/// lowering pass that introduces a `let` for sharing keeps a definition admitted. The fence
/// stands: a call on `Succ k` and a call on the structural parameter itself are refused.
#[test]
fn recursion_two_constructors_deep_must_be_admitted_without_becoming_a_hole() {
    let ctx = std_ctx();

    // halve = fix h. λn. match n with | Zero => Zero
    //                                 | Succ m => match m with | Zero => Zero
    //                                                          | Succ k => Succ (h k)
    let halve = fix(
        "h",
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                lam("_", nat(), nat()),
                vec![
                    g("Zero"),
                    lam(
                        "m",
                        nat(),
                        match_(
                            var("m"),
                            lam("_", nat(), nat()),
                            vec![g("Zero"), lam("k", nat(), succ(app(var("h"), var("k"))))],
                        ),
                    ),
                ],
            ),
        ),
    );
    let r = infer_type(&ctx, &halve);
    assert!(
        r.is_ok(),
        "`k` is a subterm of a subterm of `n` and the recursion on it plainly decreases; the \
         guard only marks binders of a match on the structural parameter itself, so every \
         `| n+2 => …` definition is refused: {r:?}"
    );

    // The `let`-alias form: identical denotation, admitted because the alias inherits smallness.
    let let_alias = fix(
        "h",
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                lam("_", nat(), nat()),
                vec![
                    g("Zero"),
                    lam(
                        "k",
                        nat(),
                        Term::Let {
                            name: "smaller".to_string(),
                            ty: Box::new(nat()),
                            value: Box::new(var("k")),
                            body: Box::new(succ(app(var("h"), var("smaller")))),
                        },
                    ),
                ],
            ),
        ),
    );
    let r2 = infer_type(&ctx, &let_alias);
    assert!(
        r2.is_ok(),
        "a `let` alias of a structurally smaller variable is still structurally smaller; today \
         the value is not registered as smaller, so naming it for readability breaks the \
         definition: {r2:?}"
    );

    // THE FENCE. Whatever admits the two above must not admit these: a bare non-decreasing call
    // (which inhabits every type), and the recursive name escaping as a first-class value.
    let diverge = fix(
        "h",
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                lam("_", nat(), nat()),
                vec![g("Zero"), lam("k", nat(), app(var("h"), succ(var("k"))))],
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &diverge).is_err(),
        "a recursive call on `Succ k` is LARGER and must stay refused"
    );
    assert!(
        recheck(&ctx, &diverge).is_err(),
        "…in the second kernel too"
    );
    let escaping = fix(
        "h",
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                lam("_", nat(), nat()),
                vec![g("Zero"), lam("_k", nat(), app(var("h"), var("n")))],
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &escaping).is_err(),
        "a call on the structural parameter itself must stay refused"
    );
}

// ==== T04-019 · `if then else` (MUST, Native) ====

/// A `Bool` condition is a two-case `Match` in `Bool`'s registration order (`true`, `false`);
/// a `Prop` condition is a `Match` on the resolved `Decidable p` instance, which is the only
/// way to keep the proofs `isTrue`/`isFalse` carry. Both work and both compute, and the prelude
/// decides the `Nat` orderings as well as equality (`decLeNat`, `decLtNat` beside `decEqBool`
/// and `decEqNat`), so `if n < 10 then … else …` resolves as `if n = 0 then … else …` does.
#[test]
fn boolean_and_decidable_branching_compute_but_ordering_needs_a_decision_procedure() {
    let ctx = std_ctx();

    // `if b then x else y` — the cases go true-first, matching Bool's registration order.
    let pick_b = lam(
        "b",
        g("Bool"),
        lam(
            "x",
            nat(),
            lam(
                "y",
                nat(),
                match_(
                    var("b"),
                    lam("_", g("Bool"), nat()),
                    vec![var("x"), var("y")],
                ),
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &pick_b).is_ok(),
        "the Bool branch must type-check"
    );
    assert_eq!(
        normalize(&ctx, &apps(pick_b.clone(), &[g("true"), lit(1), lit(2)])),
        lit(1)
    );
    assert_eq!(
        normalize(&ctx, &apps(pick_b, &[g("false"), lit(1), lit(2)])),
        lit(2)
    );

    // `if !b then x else y` — source order and registration order now DISAGREE, and the cases
    // must be swapped. Both branches have type `Nat`, so only computing catches a copy of the
    // source order.
    let pick_not_b = lam(
        "b",
        g("Bool"),
        lam(
            "x",
            nat(),
            lam(
                "y",
                nat(),
                match_(
                    var("b"),
                    lam("_", g("Bool"), nat()),
                    vec![var("y"), var("x")],
                ),
            ),
        ),
    );
    assert_eq!(
        normalize(&ctx, &apps(pick_not_b, &[g("true"), lit(1), lit(2)])),
        lit(2),
        "pickNotB true 1 2 = 2"
    );

    // A `Prop` condition: match on the INSTANCE, not on `decide p inst`.
    let is_zero = lam(
        "n",
        nat(),
        match_(
            apps(g("decEqNat"), &[var("n"), g("Zero")]),
            nat(),
            vec![
                lam("_hp", eq_nat(var("n"), g("Zero")), lit(1)),
                lam("_hn", app(g("Not"), eq_nat(var("n"), g("Zero"))), lit(0)),
            ],
        ),
    );
    assert!(
        infer_type(&ctx, &is_zero).is_ok(),
        "the Decidable branch must type-check"
    );
    assert_eq!(
        normalize(&ctx, &app(is_zero.clone(), lit(0))),
        lit(1),
        "isZero 0 = 1"
    );
    assert_eq!(
        normalize(&ctx, &app(is_zero, lit(3))),
        lit(0),
        "isZero 3 = 0"
    );

    // ORDERING. `clamp n = if n < 10 then n else 10` needs a decision procedure for `<` on
    // `Nat`. The prelude's `lt` is the `Int` ordering, `Int → Int → Bool`, and `decLtNat` decides
    // `Nat_lt`. Ordering is not a corner case: it is most of what an `if` in mathematics
    // branches on.
    assert!(
        matches!(infer_type(&ctx, &g("lt")), Ok(t) if !defeq_for_test(&ctx, &t, &pi("a", nat(), pi("b", nat(), prop())))),
        "`lt` is the Int ordering, not a Nat one — pinned so the two are never confused"
    );
    let r = infer_type(&ctx, &g("decLtNat"));
    assert!(
        r.is_ok(),
        "`decLtNat : Π(a b : Nat). Decidable (Nat_lt a b)` must exist so `if n < 10 then … \
         else …` resolves: {r:?}"
    );
}

// ==== T04-020 · `dite` — the decision's proof in each branch (MUST, Native) ====

/// `if h : c then a h else b h`. `Decidable` is `Type 0`-valued, so eliminating it into `Type`
/// is unrestricted — which is exactly why `dite` may return data — and `isTrue`/`isFalse` carry
/// `p` and `Not p` as ordinary constructor fields, so `compute_indexed_case_type` makes each
/// branch a λ over the very proof Lean's `h` binds. Routing through `decide` instead would throw
/// both proofs away; this test builds the branch that USES them, on both sides.
#[test]
fn a_dependent_if_binds_the_decisions_proof_in_both_branches() {
    let ctx = std_ctx();
    let is_zero = |n: Term| eq_nat(n, g("Zero"));
    let not_zero = |n: Term| app(g("Not"), is_zero(n));
    let disj = |n: Term| apps(g("Or"), &[is_zero(n.clone()), not_zero(n)]);

    // decide0 : Π(n:Nat). Or (n = 0) (¬ n = 0)
    //   := λn. match decEqNat n Zero return Or … with
    //            | isTrue  hp => left  (n = 0) (¬ n = 0) hp
    //            | isFalse hn => right (n = 0) (¬ n = 0) hn
    let decide0 = lam(
        "n",
        nat(),
        match_(
            apps(g("decEqNat"), &[var("n"), g("Zero")]),
            disj(var("n")),
            vec![
                lam(
                    "hp",
                    is_zero(var("n")),
                    apps(
                        g("left"),
                        &[is_zero(var("n")), not_zero(var("n")), var("hp")],
                    ),
                ),
                lam(
                    "hn",
                    not_zero(var("n")),
                    apps(
                        g("right"),
                        &[is_zero(var("n")), not_zero(var("n")), var("hn")],
                    ),
                ),
            ],
        ),
    );
    let ty = infer_type(&ctx, &decide0).expect(
        "both branches must receive their proof: `isTrue` carries `p`, `isFalse` carries `Not p`",
    );
    assert!(
        defeq_for_test(&ctx, &ty, &pi("n", nat(), disj(var("n")))),
        "decide0 : Π(n:Nat). Or (n = 0) (¬ n = 0), got {ty}"
    );
    // …and it COMPUTES: the instance reduces, so the branch is selected, not left stuck.
    let head_of = |t: &Term| {
        let mut h = t;
        while let Term::App(f, _) = h {
            h = f;
        }
        format!("{h}")
    };
    for (n, want) in [(0u64, "left"), (3, "right")] {
        let reduced = normalize(&ctx, &app(decide0.clone(), lit(n)));
        assert_eq!(
            head_of(&reduced),
            want,
            "decide0 {n} must reduce into the `{want}` branch, got {reduced}"
        );
    }
    match double_check(&ctx, &decide0) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify the dependent branch, got {other:?}"),
    }

    // Erasing the proof binder — lowering `dite` the way `ite` is lowered — must be refused.
    let erased = lam(
        "n",
        nat(),
        match_(
            apps(g("decEqNat"), &[var("n"), g("Zero")]),
            disj(var("n")),
            vec![
                apps(g("left"), &[is_zero(var("n")), not_zero(var("n")), g("I")]),
                apps(g("right"), &[is_zero(var("n")), not_zero(var("n")), g("I")]),
            ],
        ),
    );
    assert!(
        infer_type(&ctx, &erased).is_err(),
        "dropping the λ that binds `h` must not typecheck with an unrelated proof in its place"
    );
}
