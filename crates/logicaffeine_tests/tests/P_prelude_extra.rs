//! Lane P (prelude/stateability) — the neighbouring behaviour the corpus never asks about.
//!
//! Every case this lane closes adds something to the trusted library. A library addition is
//! satisfiable by a weak implementation unless the REFUSALS are pinned too, so each block here is
//! a pair: the thing that must now work, and the neighbouring thing that must still be refused.
//!
//! House rules honoured throughout:
//!   * a new eliminator must COMPUTE, not merely register;
//!   * a new constant that Lean PROVES must be `add_definition`, never `add_declaration` — the
//!     trusted base is a downward ratchet and a derived constant costs it nothing;
//!   * anything that changes kernel semantics is asserted through `double_check`, because one
//!     kernel agreeing with itself proves nothing.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, double_check, infer_type, normalize, resolve_instance, BigInt, Context,
    DoubleCheck, Literal, MetaCtx, Term, Universe,
};

// ==== per-file helpers (house style) ====

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
fn type0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn nat() -> Term {
    g("Nat")
}
fn natl(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn peano(n: usize) -> Term {
    (0..n).fold(g("Zero"), |acc, _| app(g("Succ"), acc))
}
fn succ(t: Term) -> Term {
    app(g("Succ"), t)
}
fn eq_at(ty: Term, a: Term, b: Term) -> Term {
    apps(g("Eq"), &[ty, a, b])
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn resolve(ctx: &Context, goal: &Term) -> Option<Term> {
    let mut mctx = MetaCtx::new();
    resolve_instance(ctx, &mut mctx, goal)
}
/// Every name the prelude registers as an ASSUMPTION. A constant that Lean proves must never
/// appear here.
fn axioms(ctx: &Context) -> Vec<String> {
    ctx.iter_declarations()
        .map(|(n, _)| n.to_string())
        .collect()
}

// ============================================================================================
// WellFounded.fix — T04-015's neighbourhood.
// ============================================================================================

/// The empty relation on `Nat`: every element is accessible, and the accessibility proof needs no
/// arithmetic. Shared by the `WellFounded_fix` block.
fn empty_rel() -> Term {
    lam("_a", nat(), lam("_b", nat(), g("False")))
}

/// `accAll : Π(x:Nat). Acc Nat R x` for the empty relation `R`.
fn acc_all() -> Term {
    let rel = empty_rel();
    let acc_at = |x: Term| apps(g("Acc"), &[nat(), rel.clone(), x]);
    lam(
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
                        Term::Match {
                            discriminant: Box::new(var("hr")),
                            motive: Box::new(acc_at(var("y"))),
                            cases: Vec::new(),
                        },
                    ),
                ),
            ],
        ),
    )
}

/// THE POSITIVE. `WellFounded_fix` must ι-COMPUTE through to a value — an eliminator that only
/// type-checks is an axiom wearing a recursor's name.
#[test]
fn a_well_founded_fixpoint_computes_and_is_not_an_axiom() {
    let ctx = std_ctx();

    assert!(
        ctx.is_definition("WellFounded_fix"),
        "`WellFounded.fix` is DERIVED from `Acc_rec` in Lean and must be a proven definition here \
         — assuming it would put well-founded recursion in the trusted base"
    );
    assert!(
        !axioms(&ctx).contains(&"WellFounded_fix".to_string()),
        "a derived combinator must not appear in `iter_declarations`"
    );

    // step := λx. λih. Succ Zero   (a constant step: it never uses the induction hypothesis)
    let rel = empty_rel();
    let motive = lam("_x", nat(), nat());
    let ih_ty = |x: Term| pi("y", nat(), arrow(apps(rel.clone(), &[var("y"), x]), nat()));
    let step = lam("x", nat(), lam("ih", ih_ty(var("x")), succ(g("Zero"))));
    let applied = apps(
        g("WellFounded_fix"),
        &[nat(), rel.clone(), motive, acc_all(), step, natl(3)],
    );

    let ty = infer_type(&ctx, &applied).expect("`WellFounded_fix` applied at Nat must type-check");
    assert!(
        defeq_for_test(&ctx, &ty, &nat()),
        "`WellFounded_fix … 3 : C 3 = Nat`, got {ty}"
    );
    assert_eq!(
        normalize(&ctx, &applied),
        peano(1),
        "the fixpoint must ι-COMPUTE to `Succ Zero`, not sit stuck on `Acc_rec`"
    );
    assert_eq!(
        double_check(&ctx, &applied),
        DoubleCheck::Agreed,
        "both kernels must certify the well-founded fixpoint"
    );
}

/// THE NEGATIVE. A step function at the WRONG motive must be refused — the combinator may not be
/// so loosely typed that any step fits.
#[test]
fn a_well_founded_fixpoint_refuses_a_step_at_the_wrong_motive() {
    let ctx = std_ctx();
    let rel = empty_rel();
    // The motive says `C x = Nat`; the step returns a `Bool`.
    let motive = lam("_x", nat(), nat());
    let ih_ty = |x: Term| pi("y", nat(), arrow(apps(rel.clone(), &[var("y"), x]), nat()));
    let bad_step = lam("x", nat(), lam("ih", ih_ty(var("x")), g("true")));
    let applied = apps(
        g("WellFounded_fix"),
        &[nat(), rel.clone(), motive, acc_all(), bad_step, natl(3)],
    );
    assert!(
        infer_type(&ctx, &applied).is_err(),
        "a step whose result type is not the motive must not type-check"
    );
}

// ============================================================================================
// The `Nat` order decision procedure — T04-019's neighbourhood.
// ============================================================================================

/// THE POSITIVE, by COMPUTATION on both sides of the boundary. `decLtNat`/`decLeNat` must decide,
/// not merely exist: `decide` on the instance has to reduce to a literal `Bool`.
#[test]
fn the_nat_order_decision_procedure_computes_on_both_sides_of_its_boundary() {
    let ctx = std_ctx();

    for name in ["decLtNat", "decLeNat"] {
        assert!(
            ctx.is_definition(name),
            "`{name}` must be a PROVEN definition — Lean's `Nat.decLt` is derived, and assuming a \
             decision procedure is the trusted-base bug in miniature"
        );
    }

    // `decide (Nat_lt a b) (decLtNat a b)` — one rung either side of every boundary.
    let decide_lt = |a: u64, b: u64| {
        normalize(
            &ctx,
            &apps(
                g("decide"),
                &[
                    apps(g("Nat_lt"), &[natl(a), natl(b)]),
                    apps(g("decLtNat"), &[natl(a), natl(b)]),
                ],
            ),
        )
    };
    let decide_le = |a: u64, b: u64| {
        normalize(
            &ctx,
            &apps(
                g("decide"),
                &[
                    apps(g("Nat_le"), &[natl(a), natl(b)]),
                    apps(g("decLeNat"), &[natl(a), natl(b)]),
                ],
            ),
        )
    };

    // `<` : one below the boundary, at it, and one above.
    for (a, b, want) in [
        (2u64, 3u64, "true"),
        (3, 3, "false"),
        (4, 3, "false"),
        (0, 1, "true"),
    ] {
        assert_eq!(
            decide_lt(a, b),
            g(want),
            "decide ({a} < {b}) must compute to `{want}`"
        );
    }
    // `≤` : the boundary belongs to the other side.
    for (a, b, want) in [
        (2u64, 3u64, "true"),
        (3, 3, "true"),
        (4, 3, "false"),
        (0, 0, "true"),
    ] {
        assert_eq!(
            decide_le(a, b),
            g(want),
            "decide ({a} ≤ {b}) must compute to `{want}`"
        );
    }
}

/// THE NEGATIVE, and the trap the corpus case names explicitly: closing the NAT ordering gap must
/// not be done by pointing at the `Int` builtins. What makes that impossible is their TYPE, which
/// no body can widen: all four stay `Int → Int → Bool` and refuse a `Nat` in both kernels.
///
/// Since `Int` became Lean's `ofNat | negSucc` inductive, all four are checked DEFINITIONS
/// (`le := Int_le`, a match over the two constructors bottoming out in `natLe`; `lt`/`ge`/`gt`
/// are Lean's bodies over `le`) and none of them rests on an assumption. A re-opaqued order would
/// take a proved fact back on trust, so that is pinned too.
#[test]
fn the_int_comparison_builtins_keep_their_int_type_and_are_checked_definitions() {
    use logicaffeine_kernel::prelude::axiom_dependencies_of;
    let ctx = std_ctx();
    let int_bool = pi("_", g("Int"), pi("_", g("Int"), g("Bool")));
    for name in ["lt", "le", "gt", "ge"] {
        let ty = infer_type(&ctx, &g(name)).unwrap_or_else(|e| panic!("`{name}` exists: {e:?}"));
        assert!(
            defeq_for_test(&ctx, &ty, &int_bool),
            "`{name}` must still be the `Int → Int → Bool` builtin, not `{ty}`"
        );
        assert!(
            ctx.is_definition(name) && !axioms(&ctx).contains(&name.to_string()),
            "`{name}` must be a checked definition over Lean's `Int`, never an assumption"
        );
        let usage = axiom_dependencies_of(&ctx, name).expect("registered");
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "`{name}` must rest on no assumption at all: {usage:?}"
        );
        let on_nat = apps(g(name), &[g("Zero"), g("Zero")]);
        assert!(
            infer_type(&ctx, &on_nat).is_err()
                && logicaffeine_kernel::recheck(&ctx, &on_nat).is_err(),
            "`{name} Zero Zero` must be ill-typed in BOTH kernels — a body never widens the \
             `Int` domain, so the `Int` order cannot stand in for the `Nat` one"
        );
    }
}

/// THE BOUNDARY, in the direction that matters for a `Decidable`: the `isFalse` side must carry a
/// REAL refutation, so the branch that receives `Not p` can actually be used.
#[test]
fn the_false_branch_of_the_nat_order_carries_a_usable_refutation() {
    let ctx = std_ctx();
    // `match decLtNat 4 3 return Nat with | isTrue _ => 0 | isFalse h => 1` — the false branch
    // binds a proof of `Not (Nat_lt 4 3)`; if the instance did not really decide, the match would
    // be stuck rather than selecting the second case.
    let lt43 = apps(g("Nat_lt"), &[natl(4), natl(3)]);
    let m = Term::Match {
        discriminant: Box::new(apps(g("decLtNat"), &[natl(4), natl(3)])),
        motive: Box::new(nat()),
        cases: vec![
            lam("_hp", lt43.clone(), g("Zero")),
            lam("_hn", app(g("Not"), lt43), succ(g("Zero"))),
        ],
    };
    assert!(
        infer_type(&ctx, &m).is_ok(),
        "the Decidable match must type-check"
    );
    assert_eq!(
        normalize(&ctx, &m),
        peano(1),
        "`4 < 3` is false, so the match must select the `isFalse` branch and compute"
    );
    assert_eq!(
        double_check(&ctx, &m),
        DoubleCheck::Agreed,
        "both kernels must agree about the decided branch"
    );
}

// ============================================================================================
// `Inhabited Nat` as a FALLBACK instance — T07-010's neighbourhood.
// ============================================================================================

/// THE POSITIVE: the prelude instance resolves, and to the right value.
#[test]
fn the_prelude_inhabited_instance_resolves_to_zero() {
    let ctx = std_ctx();
    let found = resolve(&ctx, &app(g("Inhabited"), nat()))
        .expect("`Inhabited Nat` must resolve out of the box, as it does in Lean");
    assert!(
        defeq_for_test(&ctx, &found, &apps(g("Inhabited_mk"), &[nat(), g("Zero")])),
        "`Inhabited Nat` must resolve to `⟨0⟩`, got `{found}`"
    );
    // …and the projection COMPUTES, which is the difference between an instance and a name.
    assert_eq!(
        normalize(&ctx, &apps(g("Inhabited_default"), &[nat(), found])),
        g("Zero"),
        "`(default : Nat)` must reduce to `Zero`"
    );
}

/// THE NEGATIVE, and the whole reason the prelude refused this for a wave: a library instance must
/// never outrank a user's. A prelude entry in the main first-match vector would be entry zero
/// forever.
#[test]
fn a_user_instance_outranks_the_prelude_one_and_the_prelude_one_stays_out_of_the_record() {
    let stock = std_ctx();
    assert_eq!(
        stock.instances().len(),
        0,
        "`instances()` is the faithful record of what was EXPLICITLY declared; a library instance \
         belongs in the fallback tier, or every count taken against it silently shifts"
    );
    assert!(
        !stock.fallback_instances().is_empty(),
        "the prelude's library instances live in the fallback tier"
    );

    let mut user = std_ctx();
    let mine = apps(g("Inhabited_mk"), &[nat(), succ(g("Zero"))]);
    user.add_instance(app(g("Inhabited"), nat()), mine.clone());
    let found = resolve(&user, &app(g("Inhabited"), nat())).expect("resolution still succeeds");
    assert!(
        defeq_for_test(&user, &found, &mine),
        "a user instance declared AFTER the prelude's must win — a fallback tier that can be \
         outranked is just a priority number with extra steps: got `{found}`"
    );
}

// ============================================================================================
// Heterogeneous equality — T06-016's neighbourhood.
// ============================================================================================

/// THE POSITIVE: `HEq` is an INDUCTIVE (so it eliminates), not another axiom, and the bridge to
/// `Eq` is a proven definition that COMPUTES.
#[test]
fn heterogeneous_equality_is_an_inductive_and_its_bridge_is_proven() {
    let ctx = std_ctx();

    assert!(
        ctx.is_inductive("HEq"),
        "`HEq` must be an inductive, not an opaque declaration"
    );
    assert!(
        !axioms(&ctx).contains(&"HEq".to_string())
            && !axioms(&ctx).contains(&"HEq_refl".to_string())
            && !axioms(&ctx).contains(&"eq_of_heq".to_string()),
        "heterogeneous equality costs the trusted base NOTHING — it is an inductive family with a \
         derived eliminator, exactly as in Lean"
    );
    assert!(
        ctx.is_definition("HEq_rec"),
        "`HEq`'s eliminator must be derived and registered, or the bridge has nothing to be \
         proved from"
    );
    assert!(
        ctx.is_definition("heq_of_eq"),
        "`heq_of_eq : Eq A a b → HEq A A a b` — the direction that IS derivable at a `Type 0`-pinned \
         `Eq` — must be PROVED by J, never assumed"
    );

    // THE LOCK THAT MATTERS MOST HERE. `eq_of_heq` is the converse, and it is NOT derivable while
    // `Eq`'s carrier is pinned to `Type 0`: Lean proves it by `cast`ing along an equation BETWEEN
    // CARRIERS (`α = β` for `α β : Sort u`) and then discharging `cast h a ≡ a` by K-like
    // reduction on the subsingleton `Eq`. LOGOS has neither yet. So the one thing that must never
    // happen is for the gap to be closed by ASSUMING it — that is Coq's `JMeq_eq`, it makes the
    // capability gap vanish from the matrix, and it reappears as an axiom nobody chose.
    assert!(
        !axioms(&ctx).contains(&"eq_of_heq".to_string()),
        "`eq_of_heq` must NEVER be an `add_declaration`. It is a THEOREM in Lean; assuming it here \
         would hide two real dependencies (a `Sort u`-carried `Eq`, and K-like iota reduction for \
         subsingleton `Prop`s) behind a green test"
    );

    // `HEq` is `Prop`-valued, at two carriers.
    let heq_ty = infer_type(&ctx, &g("HEq")).expect("`HEq` is typed");
    assert!(
        defeq_for_test(
            &ctx,
            &heq_ty,
            &pi(
                "A",
                type0(),
                pi("B", type0(), arrow(var("A"), arrow(var("B"), prop())))
            )
        ),
        "`HEq : Π(A B:Type). A → B → Prop`, got {heq_ty}"
    );

    // The derivable bridge, kernel-checked in BOTH kernels — and it must COMPUTE, so `HEq` is a
    // real family and not a name with a recursor bolted on.
    let bridge = apps(
        g("heq_of_eq"),
        &[nat(), natl(3), natl(3), apps(g("refl"), &[nat(), natl(3)])],
    );
    let ty = infer_type(&ctx, &bridge).expect("`heq_of_eq` at a common carrier type-checks");
    assert!(
        defeq_for_test(
            &ctx,
            &ty,
            &apps(g("HEq"), &[nat(), nat(), natl(3), natl(3)])
        ),
        "`heq_of_eq Nat 3 3 (refl 3) : HEq Nat Nat 3 3`, got {ty}"
    );
    assert_eq!(
        normalize(&ctx, &bridge).to_string(),
        apps(g("HEq_refl"), &[nat(), natl(3)]).to_string(),
        "`heq_of_eq … (refl a)` must ι-COMPUTE to `HEq_refl a` — J on `refl` selects the base case"
    );
    assert_eq!(
        double_check(&ctx, &bridge),
        DoubleCheck::Agreed,
        "both kernels must certify the Eq→HEq bridge"
    );

    // `HEq_rec` must ELIMINATE, not merely exist: a constant motive over the empty field list
    // recovers a value from a heterogeneous equation and reduces.
    let elim = apps(
        g("HEq_rec"),
        &[
            nat(),
            lam(
                "B",
                type0(),
                lam(
                    "x",
                    nat(),
                    lam(
                        "y",
                        var("B"),
                        lam(
                            "_h",
                            apps(g("HEq"), &[nat(), var("B"), var("x"), var("y")]),
                            nat(),
                        ),
                    ),
                ),
            ),
            lam("a", nat(), var("a")),
            nat(),
            natl(3),
            natl(3),
            apps(g("HEq_refl"), &[nat(), natl(3)]),
        ],
    );
    assert!(
        infer_type(&ctx, &elim).is_ok(),
        "`HEq_rec` at a Type-0 motive must type-check — `HEq` is a single-constructor `Prop` whose \
         only field reappears in its indices, so large elimination is admissible: {:?}",
        infer_type(&ctx, &elim).err()
    );
    assert_eq!(
        normalize(&ctx, &elim),
        natl(3),
        "`HEq_rec … (HEq_refl 3)` must ι-COMPUTE to `3`"
    );
}

/// THE NEGATIVE. `HEq_refl` relates a value only to ITSELF; the whole point of the family is that
/// an equation ACROSS two carriers is stateable but not free.
#[test]
fn heterogeneous_equality_refuses_a_refl_across_two_carriers() {
    let ctx = std_ctx();

    // Stateable: `HEq Nat Bool 3 true` is a well-formed Prop…
    let cross = apps(g("HEq"), &[nat(), g("Bool"), natl(3), g("true")]);
    assert_eq!(
        infer_type(&ctx, &cross).expect("a heterogeneous equation is STATEABLE"),
        prop(),
        "`HEq Nat Bool 3 true` must be a Prop — being able to write it down is the capability"
    );
    // …and uninhabited by `refl`, which is the reason it is not an unsound `Eq`.
    assert!(
        infer_type(&ctx, &apps(g("HEq_refl"), &[nat(), natl(3)]))
            .map(|t| !defeq_for_test(&ctx, &t, &cross))
            .unwrap_or(false),
        "`HEq.refl 3 : HEq Nat Nat 3 3` must NOT inhabit `HEq Nat Bool 3 true`"
    );
    // And the EASY direction's bridge must not accept two different carriers either.
    let bogus = apps(
        g("heq_of_eq"),
        &[
            nat(),
            natl(3),
            g("true"),
            apps(g("refl"), &[nat(), natl(3)]),
        ],
    );
    assert!(
        infer_type(&ctx, &bogus).is_err(),
        "`heq_of_eq` is stated at ONE carrier; feeding it a `Bool` on the right must be refused"
    );
}

/// THE SOUNDNESS PROBE the corpus never asks for. `HEq A A a b` is exactly the shape a
/// non-dependent `Match` on an indexed family would forge a proof of: if the matcher checked each
/// case against the motive WITHOUT specialising the constructor's indices, the `HEq_refl` case
/// would be allowed to return `refl A a : Eq A a a` at the goal `Eq A a b` and `eq_of_heq` would
/// be "derivable" — unsoundly, since nothing connects `b` to `a`.
///
/// Both kernels must refuse it. This is a REFUSAL test: it is the reason `eq_of_heq` is reported
/// as blocked rather than quietly obtained.
#[test]
fn a_non_dependent_match_on_a_heterogeneous_equation_cannot_forge_an_equation() {
    let ctx = std_ctx();
    // λA a b hh. match hh return (Eq A a b) with | HEq_refl => λA'. λa'. refl A a
    let forged = lam(
        "A",
        type0(),
        lam(
            "a",
            var("A"),
            lam(
                "b",
                var("A"),
                lam(
                    "hh",
                    apps(g("HEq"), &[var("A"), var("A"), var("a"), var("b")]),
                    Term::Match {
                        discriminant: Box::new(var("hh")),
                        motive: Box::new(eq_at(var("A"), var("a"), var("b"))),
                        cases: vec![lam("a2", var("A"), apps(g("refl"), &[var("A"), var("a")]))],
                    },
                ),
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &forged).is_err(),
        "a match on `HEq A A a b` whose motive does not abstract the indices must be REFUSED — \
         accepting it proves `a = b` from nothing"
    );
}

// ============================================================================================
// The length-indexed bitvector — T03-005's neighbourhood.
// ============================================================================================

/// `BVCons b n t : BVec (Succ n)` over the given bits, least-significant first in the order the
/// prelude's own tests use.
fn bvec(bits: &[&str]) -> Term {
    let mut out = g("BVNil");
    let mut len = g("Zero");
    for b in bits.iter().rev() {
        out = apps(g("BVCons"), &[g(b), len.clone(), out]);
        len = succ(len);
    }
    out
}
fn bvec_len(n: usize) -> Term {
    (0..n).fold(g("Zero"), |acc, _| succ(acc))
}

/// THE POSITIVE. The index is what a length-indexed vector is FOR, and the shape a match takes
/// over it is the shape that proves it.
#[test]
fn the_bitvector_length_is_an_index_and_a_length_abstracting_match_computes() {
    let ctx = std_ctx();
    assert_eq!(
        ctx.inductive_num_params("BVec"),
        0,
        "`BVec` has no uniform parameter"
    );
    assert_eq!(
        ctx.inductive_num_indices("BVec"),
        1,
        "its one arity slot is its index"
    );

    // A motive that abstracts the LENGTH as well as the vector — untypeable while the length was
    // recorded as a parameter.
    let m = Term::Match {
        discriminant: Box::new(bvec(&["B1", "B0"])),
        motive: Box::new(lam("k", nat(), lam("_v", app(g("BVec"), var("k")), nat()))),
        cases: vec![
            g("Zero"),
            lam(
                "b",
                g("Bit"),
                lam(
                    "k",
                    nat(),
                    lam("t", app(g("BVec"), var("k")), succ(g("Zero"))),
                ),
            ),
        ],
    };
    assert_eq!(
        infer_type(&ctx, &m).expect("a length-abstracting match must type-check"),
        nat()
    );
    assert_eq!(
        normalize(&ctx, &m),
        peano(1),
        "the match must ι-compute into the BVCons branch"
    );
    assert_eq!(
        double_check(&ctx, &m),
        DoubleCheck::Agreed,
        "both kernels must agree about the indexed match"
    );
}

/// THE REGRESSION GUARD, and the reason the bitwise operations had to be rewritten rather than
/// merely re-typed: they must STILL compute, bit for bit, at every length including zero.
#[test]
fn the_bitwise_operations_still_compute_over_the_indexed_bitvector() {
    let ctx = std_ctx();

    // Types unchanged: Π(n:Nat). BVec n → BVec n → BVec n.
    let bvec_n = || app(g("BVec"), var("n"));
    let binop = pi("n", nat(), arrow(bvec_n(), arrow(bvec_n(), bvec_n())));
    for name in ["bv_and", "bv_or", "bv_xor"] {
        let ty = infer_type(&ctx, &g(name)).unwrap_or_else(|e| panic!("`{name}`: {e:?}"));
        assert!(
            defeq_for_test(&ctx, &ty, &binop),
            "`{name} : Π(n). BVec n → BVec n → BVec n`, got {ty}"
        );
    }
    let unop = pi("n", nat(), arrow(bvec_n(), bvec_n()));
    let ty = infer_type(&ctx, &g("bv_not")).expect("bv_not");
    assert!(
        defeq_for_test(&ctx, &ty, &unop),
        "`bv_not : Π(n). BVec n → BVec n`, got {ty}"
    );

    // The empty vector — the branch whose result length the index now pins.
    for name in ["bv_and", "bv_or", "bv_xor"] {
        let e = apps(g(name), &[g("Zero"), g("BVNil"), g("BVNil")]);
        assert!(
            infer_type(&ctx, &e).is_ok(),
            "`{name} 0 [] []` must type-check"
        );
        assert_eq!(normalize(&ctx, &e), g("BVNil"), "`{name} 0 [] []` = []");
    }
    assert_eq!(
        normalize(&ctx, &apps(g("bv_not"), &[g("Zero"), g("BVNil")])),
        g("BVNil"),
        "`bv_not 0 []` = []"
    );

    // …and the two-bit truth table, which is what actually catches a wrong head/tail pairing.
    let cases: &[(&str, [&str; 2], [&str; 2], [&str; 2])] = &[
        ("bv_and", ["B1", "B0"], ["B1", "B1"], ["B1", "B0"]),
        ("bv_and", ["B0", "B1"], ["B1", "B1"], ["B0", "B1"]),
        ("bv_or", ["B1", "B0"], ["B0", "B0"], ["B1", "B0"]),
        ("bv_xor", ["B1", "B0"], ["B1", "B1"], ["B0", "B1"]),
    ];
    for (op, a, b, want) in cases {
        let e = apps(g(op), &[bvec_len(2), bvec(a), bvec(b)]);
        let got = normalize(&ctx, &e);
        assert_eq!(
            got.to_string(),
            bvec(want).to_string(),
            "`{op} {a:?} {b:?}` must be `{want:?}`, got {got}"
        );
        assert_eq!(
            double_check(&ctx, &e),
            DoubleCheck::Agreed,
            "both kernels must certify `{op}` over the indexed bitvector"
        );
    }
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("bv_not"), &[bvec_len(2), bvec(&["B1", "B0"])])
        )
        .to_string(),
        bvec(&["B0", "B1"]).to_string(),
        "`bv_not [B1,B0]` = [B0,B1]"
    );

    // The ORDER matters and only computing catches a swap: `bv_and` is symmetric, `bv_xor` is
    // not commutative in its BIT positions if head and tail are crossed.
    assert_eq!(
        normalize(
            &ctx,
            &apps(
                g("bv_xor"),
                &[
                    bvec_len(3),
                    bvec(&["B1", "B1", "B0"]),
                    bvec(&["B1", "B0", "B0"])
                ]
            )
        )
        .to_string(),
        bvec(&["B0", "B1", "B0"]).to_string(),
        "a crossed head/tail pairing shows up as a rotated result at width 3"
    );
}

/// THE NEGATIVE. The index must be REAL: a vector at one length must not be accepted where
/// another length is required. Before the fix the length was a parameter, so nothing checked it.
#[test]
fn the_bitvector_index_refuses_a_length_mismatch() {
    let ctx = std_ctx();
    // `bv_and 2 [B1] [B1,B0]` — a one-bit vector where a two-bit one is required.
    let bad = apps(
        g("bv_and"),
        &[bvec_len(2), bvec(&["B1"]), bvec(&["B1", "B0"])],
    );
    assert!(
        infer_type(&ctx, &bad).is_err(),
        "a `BVec 1` may not be passed where a `BVec 2` is required — the length is an index, not \
         decoration"
    );
    // …and a constructor may not claim a length its tail does not have.
    let forged = apps(g("BVCons"), &[g("B1"), bvec_len(2), bvec(&["B0"])]);
    assert!(
        infer_type(&ctx, &forged).is_err(),
        "`BVCons b 2 [B0]` must be refused: the tail is a `BVec 1`, not a `BVec 2`"
    );
}

// ============================================================================================
// The classical axioms — T06-009/010/011/018's neighbourhood.
// ============================================================================================

/// THE CENSUS OF THE ADDITION. The trusted base grew by EXACTLY the three axioms Lean also
/// assumes, and by nothing Lean proves. This is the assertion that makes the whole block
/// defensible: an axiom added to close a capability gap is the worst move available, so the
/// boundary between "assumed" and "derived" is pinned here, both directions.
#[test]
fn the_classical_layer_adds_exactly_leans_three_axioms_and_derives_the_rest() {
    let ctx = std_ctx();
    let declared = axioms(&ctx);

    // Assumed — Lean assumes exactly these three (`propext`, `Classical.choice`, `Quot.sound`).
    for name in ["propext", "Classical_choice", "Quot_sound"] {
        assert!(
            declared.contains(&name.to_string()),
            "`{name}` is one of Lean 4's core axioms and must be a DECLARED assumption"
        );
        assert!(
            infer_type(&ctx, &g(name)).is_ok(),
            "an ill-typed axiom is worse than a missing one: `{name}` must itself be well-typed"
        );
    }

    // DERIVED — Lean proves every one of these, so assuming any of them would be a trusted-base
    // regression wearing a capability win's clothes.
    for name in [
        "funext",
        "dne",
        "Classical_em",
        "Classical_propDecidable",
        "Classical_axiomOfChoice",
        "nonemptyNat",
        "Or_rec",
        "Ex_rec",
    ] {
        assert!(
            ctx.is_definition(name),
            "`{name}` must be a PROVEN definition — Lean derives it, and an axiom here would hide \
             the derivation we owe"
        );
        assert!(
            !declared.contains(&name.to_string()),
            "`{name}` must not appear in `iter_declarations`"
        );
        let body = ctx
            .get_definition_body(name)
            .unwrap_or_else(|| panic!("`{name}` has a body"));
        let inferred = infer_type(&ctx, body).unwrap_or_else(|e| {
            panic!("`{name}`'s BODY must type-check, not just its type: {e:?}")
        });
        let declared_ty = ctx.get_global(name).expect("the stored type");
        assert!(
            defeq_for_test(&ctx, &inferred, declared_ty),
            "`{name}`'s body proves `{inferred}` but it is registered at `{declared_ty}` — \
             `add_definition` stores the declared type without checking it, so this is the only \
             place the two are held together"
        );
        assert_eq!(
            double_check(&ctx, body),
            DoubleCheck::Agreed,
            "both kernels must certify `{name}`'s body"
        );
    }
}

/// THE NEGATIVE that matters for `funext`: the axiom must arrive as a TERM you apply, never as a
/// conversion rule the kernel silently gained. If `def_eq` ever became extensional, every proof
/// that cites `funext` would still typecheck and the axiom would have become invisible.
#[test]
fn function_extensionality_is_a_term_and_not_a_new_conversion_rule() {
    let mut ctx = std_ctx();
    ctx.add_declaration("addz", pi("n", nat(), nat()));
    ctx.add_declaration(
        "addz_spec",
        pi("n", nat(), eq_at(nat(), app(g("addz"), var("n")), var("n"))),
    );
    let f = lam("n", nat(), app(g("addz"), var("n")));
    let id = lam("n", nat(), var("n"));
    assert!(
        !defeq_for_test(&ctx, &f, &id),
        "`λn. addz n` and `λn. n` agree only by a HYPOTHESIS — `def_eq`'s Lambda/Lambda arm must \
         stay α-equivalence"
    );

    // …and with the axiom applied, the equation closes, at the real type.
    let closed = apps(
        g("funext"),
        &[
            nat(),
            lam("_x", nat(), nat()),
            f.clone(),
            id.clone(),
            g("addz_spec"),
        ],
    );
    let ty = infer_type(&ctx, &closed).expect("funext applied pointwise");
    assert!(
        defeq_for_test(&ctx, &ty, &eq_at(arrow(nat(), nat()), f, id)),
        "`funext … addz_spec : Eq (Nat → Nat) (λn. addz n) (λn. n)`, got {ty}"
    );
    assert_eq!(
        double_check(&ctx, &closed),
        DoubleCheck::Agreed,
        "both kernels must certify the funext application"
    );
}

/// THE FIREWALL, re-asserted after the fact. `Classical_propDecidable` is what makes DATA out of a
/// proposition — and it must remain the ONLY way. Eliminating `p ∨ ¬p` into `Decidable p` must
/// still be refused, so the new capability is a term you apply and not a loosened eliminator.
#[test]
fn choice_arrives_as_a_term_and_never_as_a_loosened_eliminator() {
    let ctx = std_ctx();

    // The capability: a blanket `Decidable` instance, at a real type, kernel-checked.
    let dec = apps(
        g("Classical_propDecidable"),
        &[eq_at(nat(), natl(2), natl(3))],
    );
    let ty = infer_type(&ctx, &dec).expect("`Classical.propDecidable` applies to any Prop");
    assert!(
        defeq_for_test(
            &ctx,
            &ty,
            &app(g("Decidable"), eq_at(nat(), natl(2), natl(3)))
        ),
        "`Classical_propDecidable p : Decidable p`, got {ty}"
    );
    assert_eq!(
        double_check(&ctx, &dec),
        DoubleCheck::Agreed,
        "both kernels must certify the classical Decidable instance"
    );

    // The firewall: `Or_rec` at a `Decidable` motive is DATA out of a two-constructor `Prop`.
    let or_pp = apps(g("Or"), &[var("p"), app(g("Not"), var("p"))]);
    let smuggle = lam(
        "p",
        prop(),
        apps(
            g("Or_rec"),
            &[
                var("p"),
                app(g("Not"), var("p")),
                lam("_", or_pp.clone(), app(g("Decidable"), var("p"))),
                lam("hp", var("p"), apps(g("isTrue"), &[var("p"), var("hp")])),
                lam(
                    "hn",
                    app(g("Not"), var("p")),
                    apps(g("isFalse"), &[var("p"), var("hn")]),
                ),
            ],
        ),
    );
    assert!(
        infer_type(&ctx, &smuggle).is_err(),
        "eliminating `p ∨ ¬p` into `Decidable p` must STILL be refused — the motive's sort is the \
         gate, and shipping `Classical_choice` must not have widened it"
    );
    // The same firewall on `Ex`: extracting the witness is choice.
    let pred = lam("n", nat(), eq_at(nat(), var("n"), g("Zero")));
    let into_data = apps(
        g("Ex_rec"),
        &[
            nat(),
            pred.clone(),
            lam("_", apps(g("Ex"), &[nat(), pred.clone()]), nat()),
            lam("n", nat(), lam("_", app(pred, var("n")), var("n"))),
        ],
    );
    assert!(
        infer_type(&ctx, &into_data).is_err(),
        "`Exists.elim` into `Type` must stay structurally blocked"
    );
}

/// `Classical.axiomOfChoice` must COMPUTE its function where the input is concrete — an existence
/// statement that cannot be run is a name, not a capability.
#[test]
fn skolemisation_produces_a_function_and_its_specification() {
    let ctx = std_ctx();
    // r x y := Eq Nat y x, and h x := ⟨x, refl⟩ — so the chosen function must be the identity.
    let rel = lam(
        "x",
        nat(),
        lam("y", nat(), eq_at(nat(), var("y"), var("x"))),
    );
    let h = lam(
        "x",
        nat(),
        apps(
            g("witness"),
            &[
                nat(),
                app(rel.clone(), var("x")),
                var("x"),
                apps(g("refl"), &[nat(), var("x")]),
            ],
        ),
    );
    let applied = apps(
        g("Classical_axiomOfChoice"),
        &[nat(), lam("_x", nat(), nat()), rel.clone(), h],
    );
    let ty = infer_type(&ctx, &applied).expect("Skolemisation applies");
    let want = apps(
        g("Ex"),
        &[
            arrow(nat(), nat()),
            lam(
                "f",
                arrow(nat(), nat()),
                pi("x", nat(), apps(rel, &[var("x"), app(var("f"), var("x"))])),
            ),
        ],
    );
    assert!(
        defeq_for_test(&ctx, &ty, &want),
        "`axiomOfChoice … : ∃ f, ∀ x, r x (f x)`, got {ty}"
    );
    assert_eq!(
        double_check(&ctx, &applied),
        DoubleCheck::Agreed,
        "both kernels must certify Skolemisation"
    );
}

/// `propext` must be stateable AND its hypothesis must be the conjunction a translator already
/// has — a fresh `Iff` inductive would be a proposition nothing could bridge back to `And`.
#[test]
fn propositional_extensionality_is_stated_over_the_transparent_iff() {
    let ctx = std_ctx();
    assert!(
        ctx.is_definition("Iff"),
        "`Iff` must stay δ-transparent to `And`"
    );

    // The conclusion's carrier: `Eq Prop True True` really is typeable.
    assert_eq!(
        infer_type(&ctx, &eq_at(prop(), g("True"), g("True"))).expect("an equation between Props"),
        prop(),
        "`Eq Prop p q` must be a Prop — `Sort Prop : Type 0`, so `Eq`'s carrier accepts it"
    );

    // Applied at a real pair, with the `And` the `Iff` unfolds to.
    let iff_tt = apps(g("Iff"), &[g("True"), g("True")]);
    let mp = lam("h", g("True"), var("h"));
    let proof = apps(
        g("conj"),
        &[
            arrow(g("True"), g("True")),
            arrow(g("True"), g("True")),
            mp.clone(),
            mp,
        ],
    );
    assert!(
        infer_type(&ctx, &proof)
            .map(|t| defeq_for_test(&ctx, &t, &iff_tt))
            .unwrap_or(false),
        "an `And (True→True) (True→True)` must inhabit `Iff True True` by δ-transparency"
    );
    let applied = apps(g("propext"), &[g("True"), g("True"), proof]);
    let ty = infer_type(&ctx, &applied).expect("propext applies");
    assert!(
        defeq_for_test(&ctx, &ty, &eq_at(prop(), g("True"), g("True"))),
        "`propext True True h : Eq Prop True True`, got {ty}"
    );
    assert_eq!(
        double_check(&ctx, &applied),
        DoubleCheck::Agreed,
        "both kernels must certify the propext application"
    );

    // THE NEGATIVE: `propext` is not a conversion rule. Two DIFFERENT propositions that happen to
    // be inter-derivable must still not be definitionally equal.
    assert!(
        !defeq_for_test(&ctx, &g("True"), &arrow(g("True"), g("True"))),
        "`True` and `True → True` are equal only by `propext` — never definitionally"
    );
}

// ============================================================================================
// The axiom audit trail — T06-017 / T11-014's neighbourhood, and `#print axioms`.
// ============================================================================================

/// THE TRANSITIVE CASE, which is the whole point. Lean warns on the declaration that CONTAINS the
/// `sorry` and says nothing about its dependants, so a theorem three lemmas downstream of an
/// admission looks finished. The closure has to be a graph walk, and the exactness NEGATIVE — a
/// clean theorem's report must be EMPTY — is what proves it is a walk and not a file-wide scan.
#[test]
fn an_admission_taints_every_definition_that_transitively_cites_it() {
    use logicaffeine_kernel::prelude::{axiom_dependencies, axiom_dependencies_of};

    let mut ctx = std_ctx();
    // The hazard: one `add_declaration` is a proof of every proposition.
    ctx.add_declaration("sorryAx", pi("A", type0(), var("A")));
    let false_eq = eq_at(nat(), natl(2), natl(3));
    ctx.add_definition(
        "taintedLemma".to_string(),
        false_eq.clone(),
        app(g("sorryAx"), false_eq.clone()),
    );
    // `taintedUser`'s body is the BARE NAME — it never mentions the admission, so a keyword or
    // warning scan cannot see it.
    ctx.add_definition(
        "taintedUser".to_string(),
        false_eq.clone(),
        g("taintedLemma"),
    );
    ctx.add_definition(
        "cleanThm".to_string(),
        eq_at(nat(), natl(3), natl(3)),
        apps(g("refl"), &[nat(), natl(3)]),
    );

    let tainted = axiom_dependencies_of(&ctx, "taintedUser").expect("registered");
    assert!(
        tainted.axioms.contains("sorryAx"),
        "the report must follow the dependency graph of ELABORATED terms and name `sorryAx` for a \
         definition that contains no admission of its own: {:?}",
        tainted
    );
    assert!(
        tainted.is_admitted(),
        "`taintedUser` is ADMITTED, not proved"
    );
    assert_eq!(
        tainted.admitted().into_iter().collect::<Vec<_>>(),
        vec!["sorryAx".to_string()],
        "the taint names exactly what was admitted"
    );
    assert!(
        tainted.render("taintedUser").contains("ADMITTED"),
        "the rendered report must LEAD with the admission: {}",
        tainted.render("taintedUser")
    );

    // THE EXACTNESS NEGATIVE. A clean theorem in the same environment must rest on nothing.
    let clean = axiom_dependencies_of(&ctx, "cleanThm").expect("registered");
    assert!(
        !clean.axioms.contains("sorryAx"),
        "the taint must be exact, not environment-wide: `cleanThm` rests on {:?}",
        clean
    );
    assert!(!clean.is_admitted(), "a proof by `refl` is not admitted");

    // …and an axiom reports ITSELF, or a `sorry`-shaped declaration would read as axiom-free.
    let itself = axiom_dependencies_of(&ctx, "sorryAx").expect("registered");
    assert!(
        itself.axioms.contains("sorryAx"),
        "an axiom rests on itself: {:?}",
        itself
    );

    // A term citing a name this base does not have is DRIFT, reported as drift.
    let drifted = axiom_dependencies(&ctx, &app(g("notInAnyBase"), nat()));
    assert!(
        drifted.unresolved.contains("notInAnyBase"),
        "a name the trusted base does not have must be reported as trusted-base drift"
    );
    assert!(
        drifted.axioms.is_empty(),
        "an unresolved name is not an axiom of this base"
    );
}

/// The closure must follow a definition's TYPE as well as its BODY: a lemma whose statement
/// mentions an axiom rests on it even when the proof term does not.
#[test]
fn the_axiom_closure_follows_a_statement_as_well_as_a_proof() {
    use logicaffeine_kernel::prelude::axiom_dependencies_of;
    let mut ctx = std_ctx();
    // `usesChoiceInItsType : Nonempty Nat → Nat := Classical_choice Nat` — body and type both
    // cite it; then a downstream definition whose BODY is a bare name.
    ctx.add_definition(
        "pickNat".to_string(),
        nat(),
        apps(g("Classical_choice"), &[nat(), g("nonemptyNat")]),
    );
    let u = axiom_dependencies_of(&ctx, "pickNat").expect("registered");
    assert!(
        u.axioms.contains("Classical_choice"),
        "`pickNat` rests on choice: {:?}",
        u
    );
    assert!(
        !u.is_admitted(),
        "citing `Classical_choice` is not an ADMISSION — it is a declared, visible assumption"
    );
    assert!(
        u.render("pickNat").contains("Classical_choice"),
        "the report must name the axiom: {}",
        u.render("pickNat")
    );
    // The `beyond` filter is what a policy is built from: a caller can permit choice and still
    // refuse everything else.
    assert!(
        u.beyond(&["Classical_choice"]).is_empty(),
        "with choice permitted, `pickNat` rests on nothing further: {:?}",
        u.beyond(&["Classical_choice"])
    );
}

/// The foreign-axiom registry is the ONLY thing that can tell `sorryAx` from a typo inside
/// `infer_type`, so it must stay disjoint from what the prelude actually registers: a name in both
/// places would produce a diagnostic saying an axiom is absent while it is present.
#[test]
fn the_foreign_axiom_registry_is_disjoint_from_the_trusted_base() {
    use logicaffeine_kernel::prelude::{foreign_axiom_note, is_admitted_axiom, FOREIGN_AXIOMS};
    let ctx = std_ctx();
    for (name, note) in FOREIGN_AXIOMS {
        assert!(
            ctx.get_global(name).is_none(),
            "`{name}` is listed as an axiom LOGOS does NOT have, but the prelude registers it — \
             delete the row in the same edit that registered the axiom"
        );
        assert!(
            !note.is_empty(),
            "every foreign axiom must carry its diagnostic sentence"
        );
        assert_eq!(
            foreign_axiom_note(name),
            Some(*note),
            "the lookup must find every row"
        );
    }
    // The discriminator: an ordinary missing name must NOT be dressed up as trusted-base drift.
    for ordinary in ["id", "Bind_bind", "base", "notAName"] {
        assert!(
            foreign_axiom_note(ordinary).is_none(),
            "`{ordinary}` is an ordinary unresolved name — reporting it as a foreign axiom would \
             turn every typo into a trusted-base alarm"
        );
    }
    assert!(is_admitted_axiom("sorryAx") && !is_admitted_axiom("Classical_choice"));
}

// ============================================================================================
// Quotients — T03-024 / T06-012's neighbourhood.
// ============================================================================================

/// `Quot_rec` must be STATEABLE with its respect obligation up to transport, and it must be a
/// DEFINITION over the other five — Lean's `Quot.rec` is one (`Init/Core.lean:1581-1611`) — while
/// the five stay opaque: not inductives (which `Match` could take apart) and not definitions.
#[test]
fn the_dependent_quotient_eliminator_is_a_definition_with_a_transport_obligation() {
    let ctx = std_ctx();
    for name in ["Quot", "Quot_mk", "Quot_lift", "Quot_sound", "Quot_ind"] {
        assert!(
            ctx.get_global(name).is_some(),
            "`{name}` must be in the prelude"
        );
        assert!(
            !ctx.is_inductive(name) && !ctx.is_definition(name),
            "`{name}` must stay an opaque primitive — an inductive `Quot` would let `Match` \
             distinguish classes that `Quot_sound` identifies"
        );
    }
    assert!(
        ctx.is_definition("Quot_rec")
            && !ctx.is_inductive("Quot_rec")
            && !ctx.iter_declarations().any(|(n, _)| n == "Quot_rec"),
        "`Quot_rec` must be a checked DEFINITION derived from the primitives, never an assumption"
    );

    // It must type-check, which for a transport-stated obligation is the real test: a respect
    // clause written as a bare `Eq (f a) (f b)` is ILL-TYPED, because the two sides live in
    // different instances of the motive.
    let ty =
        infer_type(&ctx, &g("Quot_rec")).expect("`Quot_rec`'s stated type must be well-formed");
    assert!(
        matches!(ty, Term::Pi { .. }),
        "`Quot_rec` is a Π-telescope, got {ty}"
    );

    // Applied at a concrete quotient, the WHOLE telescope must accept — including the respect
    // clause, which is the part a naive spelling gets wrong.
    //
    // The motive, the branch and the obligation are declared OPAQUE rather than written as
    // lambdas, and deliberately: a λ-motive makes every occurrence of `motive q` a β-redex, and
    // `def_eq` is currently NOT a congruence under a stuck `Eq_rec_dep` (see the lane report), so
    // a λ-motive would make this test fail for a reason that has nothing to do with `Quot_rec`.
    // Opaque constants are also the honest shape: a caller of `Quot.rec` has a motive it cannot
    // unfold.
    let rel = lam("a", nat(), lam("b", nat(), g("True")));
    let quot_ty = apps(g("Quot"), &[nat(), rel.clone()]);
    let mk = |x: Term| apps(g("Quot_mk"), &[nat(), rel.clone(), x]);
    let mut env = ctx.clone();
    env.add_declaration("M", arrow(quot_ty.clone(), type0()));
    env.add_declaration("branch", pi("a", nat(), app(g("M"), mk(var("a")))));
    // h : Π(a b:A). Π(p : r a b).
    //       Eq (M (mk b)) (Eq_rec_dep … (branch a) (mk b) (Quot_sound … p)) (branch b)
    let transport = apps(
        g("Eq_rec_dep"),
        &[
            quot_ty.clone(),
            mk(var("a")),
            lam(
                "q",
                quot_ty.clone(),
                lam(
                    "_",
                    eq_at(quot_ty.clone(), mk(var("a")), var("q")),
                    app(g("M"), var("q")),
                ),
            ),
            app(g("branch"), var("a")),
            mk(var("b")),
            apps(
                g("Quot_sound"),
                &[nat(), rel.clone(), var("a"), var("b"), var("p")],
            ),
        ],
    );
    env.add_declaration(
        "respectsIt",
        pi(
            "a",
            nat(),
            pi(
                "b",
                nat(),
                pi(
                    "p",
                    apps(rel.clone(), &[var("a"), var("b")]),
                    eq_at(
                        app(g("M"), mk(var("b"))),
                        transport,
                        app(g("branch"), var("b")),
                    ),
                ),
            ),
        ),
    );
    let applied = apps(
        g("Quot_rec"),
        &[
            nat(),
            rel.clone(),
            g("M"),
            g("branch"),
            g("respectsIt"),
            mk(g("Zero")),
        ],
    );
    let got = infer_type(&env, &applied)
        .unwrap_or_else(|e| panic!("`Quot_rec` must accept a transport-stated obligation: {e:?}"));
    assert!(
        defeq_for_test(&env, &got, &app(g("M"), mk(g("Zero")))),
        "`Quot_rec … (Quot_mk … 0) : motive (Quot_mk … 0)`, got {got}"
    );

    // THE NEGATIVE: a respect proof of the NAIVE shape — `Eq ? (f a) (f b)` with no transport —
    // must be refused. It is the single most natural wrong spelling, and it is ill-typed precisely
    // because the two sides live in different instances of the motive.
    let mut naive = env.clone();
    naive.add_declaration(
        "respectsNaively",
        pi(
            "a",
            nat(),
            pi(
                "b",
                nat(),
                pi(
                    "p",
                    apps(rel.clone(), &[var("a"), var("b")]),
                    eq_at(
                        app(g("M"), mk(var("b"))),
                        app(g("branch"), var("a")),
                        app(g("branch"), var("b")),
                    ),
                ),
            ),
        ),
    );
    let bad = apps(
        g("Quot_rec"),
        &[
            nat(),
            rel.clone(),
            g("M"),
            g("branch"),
            g("respectsNaively"),
            mk(g("Zero")),
        ],
    );
    assert!(
        infer_type(&naive, &bad).is_err(),
        "a respect proof stated WITHOUT the transport must be refused — `f a` and `f b` do not \
         inhabit a common type, and accepting it would make the obligation decoration"
    );
}

/// `Setoid` and `Quotient` are the layer Lean's whole quotient API is phrased in, and they must
/// cost the trusted base NOTHING: a structure whose projections ι-reduce, and a δ-transparent
/// definition over the existing primitives.
#[test]
fn the_setoid_layer_is_structural_and_its_quotient_is_transparent() {
    let ctx = std_ctx();
    let declared = axioms(&ctx);
    for name in ["Setoid", "Quotient", "Quotient_mk"] {
        assert!(ctx.get_global(name).is_some(), "`{name}` must exist");
        assert!(
            !declared.contains(&name.to_string()),
            "`{name}` must not be an axiom — Lean's are a structure and two definitions"
        );
    }
    assert!(
        ctx.struct_info("Setoid").is_some(),
        "`Setoid` must be a STRUCTURE (eta, projections)"
    );
    for proj in [
        "Setoid_r",
        "Setoid_refl",
        "Setoid_symm",
        "Setoid_trans",
        "Setoid_mk",
    ] {
        assert!(
            ctx.get_global(proj).is_some(),
            "`{proj}` must be generated by `add_structure`"
        );
    }

    // A real Setoid, and its projections COMPUTE.
    let eq_rel = lam(
        "x",
        nat(),
        lam("y", nat(), eq_at(nat(), var("x"), var("y"))),
    );
    let refl_pf = lam("x", nat(), apps(g("refl"), &[nat(), var("x")]));
    let symm_pf = lam(
        "x",
        nat(),
        lam(
            "y",
            nat(),
            lam(
                "h",
                eq_at(nat(), var("x"), var("y")),
                apps(g("Eq_sym"), &[nat(), var("x"), var("y"), var("h")]),
            ),
        ),
    );
    let trans_pf = lam(
        "x",
        nat(),
        lam(
            "y",
            nat(),
            lam(
                "z",
                nat(),
                lam(
                    "h1",
                    eq_at(nat(), var("x"), var("y")),
                    lam(
                        "h2",
                        eq_at(nat(), var("y"), var("z")),
                        apps(
                            g("Eq_trans"),
                            &[nat(), var("x"), var("y"), var("z"), var("h1"), var("h2")],
                        ),
                    ),
                ),
            ),
        ),
    );
    let s = apps(
        g("Setoid_mk"),
        &[nat(), eq_rel.clone(), refl_pf, symm_pf, trans_pf],
    );
    let s_ty = infer_type(&ctx, &s).expect("a Setoid over Nat's equality must type-check");
    assert!(
        defeq_for_test(&ctx, &s_ty, &app(g("Setoid"), nat())),
        "`⟨(=), refl, symm, trans⟩ : Setoid Nat`, got {s_ty}"
    );
    assert_eq!(
        double_check(&ctx, &s).to_owned(),
        DoubleCheck::Agreed,
        "both kernels must certify the Setoid"
    );
    // The relation projection reduces back to the relation.
    assert_eq!(
        normalize(&ctx, &apps(g("Setoid_r"), &[nat(), s.clone()])).to_string(),
        eq_rel.to_string(),
        "`Setoid_r ⟨r, …⟩` must ι-compute to `r`"
    );

    // `Quotient Nat s` is δ-transparent to `Quot Nat (Setoid_r Nat s)`, and `Quotient_mk` really
    // is `Quot_mk` — so the ONE computation rule quotients have still fires through the wrapper.
    let q_ty = infer_type(&ctx, &apps(g("Quotient"), &[nat(), s.clone()])).expect("Quotient Nat s");
    assert!(
        matches!(q_ty, Term::Sort(Universe::Type(0))),
        "`Quotient Nat s : Type 0`, got {q_ty}"
    );
    assert!(
        defeq_for_test(
            &ctx,
            &apps(g("Quotient"), &[nat(), s.clone()]),
            &apps(
                g("Quot"),
                &[nat(), apps(g("Setoid_r"), &[nat(), s.clone()])]
            )
        ),
        "`Quotient a s` must be δ-transparent to `Quot a s.r`"
    );
    let mk = apps(g("Quotient_mk"), &[nat(), s.clone(), natl(2)]);
    assert!(infer_type(&ctx, &mk).is_ok(), "`Quotient_mk` forms a class");
    let lifted = apps(
        g("Quot_lift"),
        &[
            nat(),
            apps(g("Setoid_r"), &[nat(), s.clone()]),
            nat(),
            lam("n", nat(), var("n")),
            g("respectsAnything"),
            mk,
        ],
    );
    let mut with_resp = ctx.clone();
    with_resp.add_declaration(
        "respectsAnything",
        pi(
            "a",
            nat(),
            pi(
                "b",
                nat(),
                arrow(
                    eq_at(nat(), var("a"), var("b")),
                    eq_at(nat(), var("a"), var("b")),
                ),
            ),
        ),
    );
    assert_eq!(
        normalize(&with_resp, &lifted).to_string(),
        natl(2).to_string(),
        "`Quot_lift … (Quotient_mk … 2)` must still reduce to `2` — the wrapper must not block the \
         one computation rule quotients have"
    );
}

// ============================================================================================
// `Float` — T10-007's neighbourhood.
// ============================================================================================

/// `Float` must have an arithmetic and comparison surface, at the right types, and the `Int`
/// primitives must NOT have been widened to cover it.
#[test]
fn float_has_an_opaque_operation_surface_and_the_int_primitives_stay_int_only() {
    let ctx = std_ctx();
    let fl = || g("Float");
    let bin = pi("_", fl(), pi("_", fl(), fl()));
    let cmp = pi("_", fl(), pi("_", fl(), g("Bool")));
    for name in ["Float_add", "Float_sub", "Float_mul", "Float_div"] {
        let ty = infer_type(&ctx, &g(name)).unwrap_or_else(|e| panic!("`{name}`: {e:?}"));
        assert!(
            defeq_for_test(&ctx, &ty, &bin),
            "`{name} : Float → Float → Float`, got {ty}"
        );
    }
    for name in ["Float_lt", "Float_le"] {
        let ty = infer_type(&ctx, &g(name)).unwrap_or_else(|e| panic!("`{name}`: {e:?}"));
        assert!(
            defeq_for_test(&ctx, &ty, &cmp),
            "`{name} : Float → Float → Bool`, got {ty}"
        );
    }
    // Applied at literals it is well-typed (which is the capability) …
    let sum = apps(g("Float_add"), &[floatl(1.5), floatl(2.5)]);
    assert_eq!(
        infer_type(&ctx, &sum).expect("Float_add applies to Float literals"),
        fl()
    );
    // … and the `Int` ALU must still REFUSE Floats, or `add 1.5 2.5` would reduce with integer
    // semantics and be silently wrong.
    for name in ["add", "sub", "mul", "div", "mod"] {
        assert!(
            infer_type(&ctx, &apps(g(name), &[floatl(1.5), floatl(2.5)])).is_err(),
            "`{name}` is `Int → Int → Int`; applying it to Floats must stay ill-typed"
        );
    }
    for name in ["Float_add", "Float_lt"] {
        assert!(
            infer_type(&ctx, &apps(g(name), &[natl(1), natl(2)])).is_err(),
            "`{name}` is Float-only; applying it to Nats must be refused"
        );
    }
}

fn floatl(x: f64) -> Term {
    Term::Lit(Literal::Float(x))
}

// ============================================================================================
// `funext` — a THEOREM, not an axiom. T06-011's neighbourhood, and the trusted-base ratchet.
// ============================================================================================

/// `F := Π(x:A). B x` for the monomorphic instance `A := Nat`, `B := λ_. Nat` used below.
fn nn() -> Term {
    arrow(nat(), nat())
}

/// Pointwise equality on `Nat → Nat`, as a RELATION — the thing `funext`'s proof quotients by.
/// Written here from the same specification the prelude uses, never read back out of it: a test
/// that fetches the implementation's own relation proves only that the implementation equals
/// itself.
fn pointwise_rel() -> Term {
    lam(
        "u",
        nn(),
        lam(
            "w",
            nn(),
            pi(
                "pt",
                nat(),
                eq_at(nat(), app(var("u"), var("pt")), app(var("w"), var("pt"))),
            ),
        ),
    )
}

/// `ext : Quot (Nat→Nat) eqv → (Nat→Nat)` — the pointwise `Quot_lift` the proof turns on.
fn ext_fn() -> Term {
    let quot_ty = apps(g("Quot"), &[nn(), pointwise_rel()]);
    lam(
        "q",
        quot_ty,
        lam(
            "ix",
            nat(),
            apps(
                g("Quot_lift"),
                &[
                    nn(),
                    pointwise_rel(),
                    nat(),
                    lam("u2", nn(), app(var("u2"), var("ix"))),
                    lam(
                        "a2",
                        nn(),
                        lam(
                            "b2",
                            nn(),
                            lam(
                                "hab",
                                pi(
                                    "pt2",
                                    nat(),
                                    eq_at(
                                        nat(),
                                        app(var("a2"), var("pt2")),
                                        app(var("b2"), var("pt2")),
                                    ),
                                ),
                                app(var("hab"), var("ix")),
                            ),
                        ),
                    ),
                    var("q"),
                ],
            ),
        ),
    )
}

/// THE RATCHET. `funext` is one of the four constants a Lean user reaches for and it is NOT one
/// of Lean's three axioms — Lean PROVES it from `Quot.sound`. Assuming it buys, at a permanent
/// cost to the trusted base, something the reference implementation gets for free.
///
/// This is the assertion that stops a future wave from taking the easy road back.
#[test]
fn funext_is_a_kernel_checked_theorem_and_never_an_axiom() {
    let ctx = std_ctx();

    assert!(
        !axioms(&ctx).iter().any(|n| n == "funext"),
        "`funext` must NOT be in the trusted base: Lean proves it from `Quot.sound` and so do we. \
         Assuming it is the one move that makes the 98-vs-3 comparison worse for nothing"
    );
    assert!(
        ctx.is_definition("funext"),
        "`funext` must be a DEFINITION with a kernel-checked body"
    );

    // The stated type is the DEPENDENT one. A monomorphic `Nat → Nat` version would be strictly
    // weaker at the same length, and would not serve `ext`/`Set`/`Matrix`.
    let dep_fn = pi("x", var("A"), app(var("B"), var("x")));
    let stated = pi(
        "A",
        type0(),
        pi(
            "B",
            pi("_x", var("A"), type0()),
            pi(
                "f",
                dep_fn.clone(),
                pi(
                    "g",
                    dep_fn.clone(),
                    arrow(
                        pi(
                            "pt",
                            var("A"),
                            eq_at(
                                app(var("B"), var("pt")),
                                app(var("f"), var("pt")),
                                app(var("g"), var("pt")),
                            ),
                        ),
                        eq_at(dep_fn.clone(), var("f"), var("g")),
                    ),
                ),
            ),
        ),
    );
    let declared = infer_type(&ctx, &g("funext")).expect("`funext` has a type");
    assert!(
        defeq_for_test(&ctx, &declared, &stated),
        "`funext` must be stated in the dependent form, got {declared}"
    );

    // THE PART THAT MAKES IT A THEOREM: the BODY must check to the stated type. `add_definition`
    // does not check, so a definition whose body is nonsense registers exactly as happily as one
    // that proves something — which is the difference between a theorem and an axiom with a
    // decorative body.
    let body = ctx
        .get_definition_body("funext")
        .cloned()
        .expect("`funext` is a definition and has a body");
    let inferred = infer_type(&ctx, &body)
        .unwrap_or_else(|e| panic!("`funext`'s proof term must kernel-check: {e:?}"));
    assert!(
        defeq_for_test(&ctx, &inferred, &declared),
        "`funext`'s body proves {inferred}, but it is declared at {declared}"
    );

    // BOTH KERNELS. One kernel agreeing with itself proves nothing.
    assert_eq!(
        double_check(&ctx, &body),
        DoubleCheck::Agreed,
        "the second, independently-written de Bruijn kernel must corroborate the proof"
    );
}

/// THE DEPENDENCY. A theorem's worth is what it rests on, and `funext` must rest on `Quot.sound`
/// and NOTHING classical — no `dne`, no `Classical_choice`, no `propext`. If the closure ever
/// widens, the proof was replaced by a shortcut. The quotient formers the proof goes through are
/// primitive identities, not assumption leaves, so the closure is Lean's `#print axioms funext`.
#[test]
fn funext_rests_on_quot_sound_and_nothing_classical() {
    use logicaffeine_kernel::prelude::axiom_dependencies_of;
    let ctx = std_ctx();
    let usage = axiom_dependencies_of(&ctx, "funext").expect("`funext` is registered");
    let expected: std::collections::BTreeSet<String> =
        ["Quot_sound"].iter().map(|s| s.to_string()).collect();
    assert_eq!(
        usage.axioms, expected,
        "`funext` must rest on exactly `Quot_sound` — Lean's `#print axioms funext` is `[Quot.sound]`"
    );
    assert!(
        usage.unresolved.is_empty(),
        "`funext` cites names the trusted base does not have: {:?}",
        usage.unresolved
    );
    for classical in ["dne", "Classical_choice", "propext"] {
        assert!(
            !usage.axioms.contains(classical),
            "`funext` must not reach for `{classical}` — extensionality is not classical"
        );
    }
}

/// THE LOAD-BEARING CONVERSION, pinned on its own. The proof closes only because
/// `ext (Quot_mk F eqv u)` is DEFINITIONALLY `u`: one quotient ι-step, one β-step, then η. Take
/// η away and the term is `λx. u x`, the motive does not land on `Eq F f g`, and the theorem does
/// not close. So this is the step to pin, and its absence from `normalize` is what proves η is
/// doing the work rather than reduction.
#[test]
fn the_quotient_section_of_funext_is_definitional_but_invisible_to_normalize() {
    let ctx = std_ctx();
    let fctx = ctx.extend("u", nn());
    let mk_u = apps(g("Quot_mk"), &[nn(), pointwise_rel(), var("u")]);
    let round_trip = app(ext_fn(), mk_u);

    assert!(
        defeq_for_test(&fctx, &round_trip, &var("u")),
        "`ext (Quot_mk F eqv u) ≡ u` — ι then β then η; this conversion IS the proof of funext"
    );
    assert_ne!(
        normalize(&fctx, &round_trip),
        normalize(&fctx, &var("u")),
        "η is invisible to `normalize`, so the conversion above is decided by `def_eq` and not by \
         reduction — which is exactly why `funext` needs a proof rather than a rewrite"
    );

    // THE BOUNDARY on the other side: the same lift at a DIFFERENT representative is not
    // convertible to `u` — the section is not a constant function dressed up.
    let wctx = fctx.extend("w", nn());
    let mk_w = apps(g("Quot_mk"), &[nn(), pointwise_rel(), var("w")]);
    assert!(
        !defeq_for_test(&wctx, &app(ext_fn(), mk_w), &var("u")),
        "`ext (Quot_mk F eqv w)` is `w`, not `u` — the ι-rule must carry the representative \
         through, not discard it"
    );
}

/// THE NEGATIVE, and the one the whole design turns on: gaining `funext` must NOT have made the
/// kernel extensional. It arrives as a term you APPLY; conversion is untouched.
#[test]
fn funext_is_a_term_you_apply_and_never_a_conversion_rule_the_kernel_gained() {
    let ctx = std_ctx();

    // Two pointwise-equal functions that are not convertible: `λn. n + 0` and `λn. n`.
    let mut env = ctx.clone();
    env.add_declaration("addz", nn());
    env.add_declaration(
        "addz_spec",
        pi("n", nat(), eq_at(nat(), app(g("addz"), var("n")), var("n"))),
    );
    let lhs = lam("n", nat(), app(g("addz"), var("n")));
    let rhs = lam("n", nat(), var("n"));
    assert!(
        !defeq_for_test(&env, &lhs, &rhs),
        "`def_eq`'s Lambda/Lambda arm is α-equivalence. If these ever convert, the kernel became \
         extensional and `funext` stopped being a theorem about it"
    );

    // Two OPAQUE functions likewise stay unconvertible, even with a pointwise hypothesis in scope:
    // the hypothesis is data you must cite, not a rule the checker consults.
    let opaque = env.extend("p", nn()).extend("q", nn());
    assert!(
        !defeq_for_test(&opaque, &var("p"), &var("q")),
        "η must not be mistaken for extensionality"
    );

    // Applied properly, it closes exactly that equation — and both kernels agree.
    let proof = apps(
        g("funext"),
        &[
            nat(),
            lam("_", nat(), nat()),
            lhs.clone(),
            rhs.clone(),
            g("addz_spec"),
        ],
    );
    let got = infer_type(&env, &proof)
        .unwrap_or_else(|e| panic!("`funext` must close a pointwise equality: {e:?}"));
    assert!(
        defeq_for_test(&env, &got, &eq_at(nn(), lhs.clone(), rhs.clone())),
        "`funext … addz_spec : Eq (Nat→Nat) (λn. addz n) (λn. n)`, got {got}"
    );
    assert_eq!(
        double_check(&env, &proof),
        DoubleCheck::Agreed,
        "both kernels must accept an applied `funext`"
    );

    // THE REFUSAL. A hypothesis at the WRONG carrier must not be accepted — the theorem is
    // dependent, and a `B` that does not match the functions' codomain is the natural wrong call.
    let mismatched = apps(
        g("funext"),
        &[nat(), lam("_", nat(), g("Bool")), lhs, rhs, g("addz_spec")],
    );
    assert!(
        infer_type(&env, &mismatched).is_err(),
        "`funext` at a codomain family that does not match the functions must be refused"
    );
}

// ============================================================================================
// `propext` — T06-010's neighbourhood: an axiom nothing cites is an axiom nothing can REPORT.
// ============================================================================================

/// THE POSITIVE. `propext` is one of Lean's three, it is stated in Lean's shape, and it has REAL
/// users in the library — `eq_true`/`eq_false`, the proposition normal form every `simp` works in.
/// Both are kernel-checked definitions, so the capability costs the trusted base nothing beyond
/// the one axiom Lean also assumes.
#[test]
fn propositional_extensionality_has_kernel_checked_users_that_cost_nothing() {
    let ctx = std_ctx();

    let stated = pi(
        "p",
        prop(),
        pi(
            "q",
            prop(),
            arrow(
                apps(g("Iff"), &[var("p"), var("q")]),
                eq_at(prop(), var("p"), var("q")),
            ),
        ),
    );
    let got = infer_type(&ctx, &g("propext")).expect("`propext` is registered");
    assert!(
        defeq_for_test(&ctx, &got, &stated),
        "`propext : Π(p q:Prop). (p ↔ q) → p = q`, got {got}"
    );

    for (name, ty) in [
        (
            "eq_true",
            pi(
                "p",
                prop(),
                arrow(var("p"), eq_at(prop(), var("p"), g("True"))),
            ),
        ),
        (
            "eq_false",
            pi(
                "p",
                prop(),
                arrow(app(g("Not"), var("p")), eq_at(prop(), var("p"), g("False"))),
            ),
        ),
    ] {
        assert!(
            ctx.is_definition(name),
            "`{name}` must be a kernel-checked DEFINITION — it is one application of `propext`, \
             and assuming it would put a derivation on the trusted base"
        );
        let declared = infer_type(&ctx, &g(name)).unwrap_or_else(|e| panic!("`{name}`: {e:?}"));
        assert!(
            defeq_for_test(&ctx, &declared, &ty),
            "`{name}` is stated as {declared}"
        );
        let body = ctx.get_definition_body(name).cloned().expect("has a body");
        let inferred = infer_type(&ctx, &body)
            .unwrap_or_else(|e| panic!("`{name}`'s proof term must kernel-check: {e:?}"));
        assert!(
            defeq_for_test(&ctx, &inferred, &declared),
            "`{name}`'s body proves {inferred} but it is declared at {declared}"
        );
        assert_eq!(
            double_check(&ctx, &body),
            DoubleCheck::Agreed,
            "both kernels must corroborate `{name}`"
        );
    }
}

/// THE TRACKING, which is the half T06-010 is actually about. `#print axioms` is Lean's single
/// most important trust tool, and an axiom with no user in the library has nothing to be reported
/// FROM. `eq_true` gives `propext` a citation, and the closure must find it through the
/// definition — one hop, but the same walk that finds a `sorry` three lemmas up.
#[test]
fn a_propext_using_theorem_reports_the_axiom_it_rests_on() {
    use logicaffeine_kernel::prelude::{axiom_dependencies, axiom_dependencies_of};
    let ctx = std_ctx();

    for name in ["eq_true", "eq_false"] {
        let usage = axiom_dependencies_of(&ctx, name).expect("registered");
        assert!(
            usage.axioms.contains("propext"),
            "`{name}` rests on `propext` and the report must say so, got {:?}",
            usage.axioms
        );
        assert!(
            usage.unresolved.is_empty(),
            "`{name}` cites {:?}",
            usage.unresolved
        );
        assert!(
            usage.render(name).contains("propext"),
            "the rendered `#print axioms` line must name the axiom"
        );
    }

    // TRANSITIVELY, through a user's user: a theorem that only ever mentions `eq_true` still
    // rests on `propext`, and a one-hop scan of its proof term would not say so.
    let mut env = ctx.clone();
    env.add_declaration("hypP", prop());
    env.add_declaration("proofOfP", g("hypP"));
    let downstream = apps(g("eq_true"), &[g("hypP"), g("proofOfP")]);
    let usage = axiom_dependencies(&env, &downstream);
    assert!(
        usage.axioms.contains("propext"),
        "the closure must follow `eq_true`'s body to `propext`; a one-hop scan sees only \
         `eq_true` and reports a proof that rests on an axiom as resting on none"
    );

    // THE NEGATIVE, and the whole value of the report: a theorem that does NOT use the axiom must
    // NOT name it. A report that says "propext" about everything says nothing.
    let clean = apps(g("refl"), &[nat(), peano(2)]);
    let clean_usage = axiom_dependencies(&ctx, &clean);
    assert!(
        !clean_usage.axioms.contains("propext"),
        "`refl Nat 2` rests on no axioms at all; naming `propext` here would make the report a \
         file-wide scan wearing a graph walk's name. Got {:?}",
        clean_usage.axioms
    );
    assert!(
        clean_usage.axioms.is_empty(),
        "a proof by `refl` on an inductive rests on NOTHING, got {:?}",
        clean_usage.axioms
    );
}

// ============================================================================================
// Certificates and the audit trail — T06-017 / T11-014's neighbourhood.
//
// The corpus asks for two things here and they are different questions. "Can the axioms a proof
// rests on be REPORTED?" is `axiom_dependencies`, tested above. "Can a proof that rests on an
// ADMISSION be mistaken for a finished one?" is this block, and it is the one that bites: a
// `sorryAx`-backed proof type-checks, BOTH kernels agree on it, and `verified == true` genuinely
// holds. Lean warns on the declaration that contains the `sorry` and says nothing about its
// dependants; the taint has to survive a rename, a re-export and three lemmas of distance, so it
// is folded into the re-check verdict rather than offered beside it.
// ============================================================================================

#[cfg(feature = "serde")]
mod audit_trail {
    use super::*;
    use logicaffeine_kernel::certificate::{
        recheck, recheck_theorem, CertDeclaration, CertDefinition, Certificate, TheoremCertificate,
    };

    /// `Π(P:Prop). P → P`, proved by the identity — the canonical axiom-free certificate.
    fn identity_cert() -> Certificate {
        let ty = pi("P", prop(), arrow(var("P"), var("P")));
        let term = lam("P", prop(), lam("h", var("P"), var("h")));
        Certificate::new(term, ty)
    }

    /// THE POSITIVE, and the control for everything below: a clean certificate re-checks and
    /// reports an EMPTY axiom set. A report that names something here would make every later
    /// assertion vacuous.
    #[test]
    fn a_clean_certificate_rechecks_and_rests_on_no_axioms_at_all() {
        let cert = identity_cert();
        assert!(
            recheck(&cert).is_ok(),
            "`λP h. h : Π(P:Prop). P → P` re-checks"
        );
        let usage = cert.axioms_used();
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "the identity proof rests on NOTHING; got {usage:?}"
        );
        assert!(!usage.is_admitted());
        assert!(
            usage.render("identity").contains("depends on no axioms"),
            "the `#print axioms` line for a clean proof must say so outright"
        );
    }

    /// TRACKING IS NOT BANNING. A proof that genuinely rests on `Classical_choice` — one of Lean's
    /// own three — is a perfectly good proof: it must re-check, and the axiom must be NAMED.
    /// Without this assertion, "refuse anything with an axiom" would pass every other test here
    /// and destroy the library.
    #[test]
    fn a_proof_resting_on_a_declared_axiom_rechecks_and_the_axiom_is_named() {
        let witness = apps(g("Classical_choice"), &[nat(), g("nonemptyNat")]);
        let cert = Certificate::new(witness, nat());
        assert!(
            recheck(&cert).is_ok(),
            "citing a DECLARED axiom is legitimate; refusing it would ban the trusted base"
        );
        let usage = cert.axioms_used();
        assert!(
            usage.axioms.contains("Classical_choice"),
            "the report must name `Classical_choice`, got {:?}",
            usage.axioms
        );
        assert!(
            !usage.is_admitted(),
            "an axiom is not an admission — `Classical_choice` is assumed by Lean too"
        );
        // The closure was computed against the base the certificate would be RE-CHECKED in — no
        // context was passed in and none could be, which is exactly the property that makes the
        // answer worth anything.
    }

    /// THE HOLE, in the shape T11-014 names. A caller reaches past `StandardLibrary::register`,
    /// declares `sorryAx : Π(a:Type). a`, and proves `1 = 0`. It type-checks. Both kernels agree.
    /// The certificate must nevertheless FAIL to re-check, and the error must name the admission.
    #[test]
    fn an_admitted_proof_does_not_recheck_and_the_admission_is_the_headline() {
        let sorry_ax = pi("a", type0(), var("a"));
        let false_thm = eq_at(nat(), peano(1), peano(0));
        let cert = TheoremCertificate::new(
            "oneEqZero",
            app(g("sorryAx"), false_thm.clone()),
            false_thm.clone(),
            vec![CertDeclaration {
                name: "sorryAx".to_string(),
                ty: sorry_ax.clone(),
            }],
            vec![],
        );

        // It really does type-check — that is the whole danger, and asserting it here keeps the
        // refusal below from being satisfied by an unrelated failure.
        let mut env = std_ctx();
        env.add_declaration("sorryAx", sorry_ax.clone());
        let proof = app(g("sorryAx"), false_thm.clone());
        assert!(
            infer_type(&env, &proof).is_ok(),
            "a `sorryAx`-backed proof of `1 = 0` type-checks; if it did not, this test would be \
             passing for the wrong reason"
        );
        assert_eq!(
            double_check(&env, &proof),
            DoubleCheck::Agreed,
            "and BOTH kernels agree on it — a second kernel cannot see an assumption the first \
             one was handed"
        );

        let out = recheck_theorem(&cert);
        assert!(
            !out.rechecks,
            "an ADMITTED proof must not re-check. `rechecks == true` is what every caller reads, \
             so the taint belongs in it and not beside it"
        );
        let err = out.error.expect("a refusal must say why");
        assert!(
            err.contains("sorryAx"),
            "the error must NAME the admission — `1 = 0` is provable from it and nothing else in \
             the certificate says so: `{err}`"
        );
        assert!(
            err.starts_with("'oneEqZero' is ADMITTED"),
            "the admission is the headline, not a footnote a reader scrolls past: `{err}`"
        );
        assert!(out.axioms.is_admitted() && out.axioms.admitted().contains("sorryAx"));
    }

    /// TRANSITIVITY, which is the part a one-hop scan gets wrong and the reason this is a graph
    /// walk. The proof term names only `taintedLemma`; the `sorry` is one definition further in.
    /// A theorem three lemmas downstream of an admission looks finished, and that is precisely the
    /// failure Lean's `#print axioms`-as-a-command leaves to the reader.
    #[test]
    fn an_admission_taints_every_theorem_that_transitively_cites_it() {
        let sorry_ax = pi("a", type0(), var("a"));
        let lemma_ty = eq_at(nat(), peano(1), peano(0));
        let cert = TheoremCertificate::new(
            "downstreamUser",
            g("taintedLemma"),
            lemma_ty.clone(),
            vec![CertDeclaration {
                name: "sorryAx".to_string(),
                ty: sorry_ax,
            }],
            vec![CertDefinition {
                name: "taintedLemma".to_string(),
                ty: lemma_ty.clone(),
                body: app(g("sorryAx"), lemma_ty.clone()),
            }],
        );
        let out = recheck_theorem(&cert);
        assert!(
            !out.rechecks,
            "the proof term is the bare name `taintedLemma` — the admission is one hop away, and \
             one hop is all it takes to look finished"
        );
        assert!(
            out.axioms.admitted().contains("sorryAx"),
            "the closure must follow the DEFINITION's body: got {:?}",
            out.axioms
        );

        // THE ORDER THE MINTING SIDE HAPPENED TO EMIT IN is not something a re-checker may depend
        // on, and this is the assertion that says so. `add_definition` is a CHECKED gateway, so a
        // definition replayed BEFORE the premise its body cites can be refused and dropped — and
        // a dropped definition reads back as an unresolved name, which is a different verdict
        // about a certificate that is not different at all. Both orderings must rebuild the same
        // context and reach the same answer.
        let swapped = TheoremCertificate::new(
            "downstreamUser",
            g("taintedLemma"),
            lemma_ty.clone(),
            cert.declarations.clone(),
            cert.definitions.clone(),
        );
        let swapped_out = recheck_theorem(&swapped);
        assert_eq!(
            swapped_out.axioms, out.axioms,
            "the disclosed delta is a SET, not a script: replaying it must not depend on whether              a definition or the premise it cites came first"
        );

        // THE BOUNDARY: the same certificate with an honest lemma re-checks. The refusal must be
        // about the admission and not about disclosed definitions in general.
        let honest = TheoremCertificate::new(
            "downstreamUser",
            g("honestLemma"),
            eq_at(nat(), peano(1), peano(1)),
            vec![],
            vec![CertDefinition {
                name: "honestLemma".to_string(),
                ty: eq_at(nat(), peano(1), peano(1)),
                body: apps(g("refl"), &[nat(), peano(1)]),
            }],
        );
        let good = recheck_theorem(&honest);
        assert!(
            good.rechecks,
            "an honest disclosed lemma re-checks: {:?}",
            good.error
        );
        assert!(
            good.axioms.axioms.is_empty(),
            "and rests on nothing, got {:?}",
            good.axioms.axioms
        );
    }

    /// THE RENAME. The taint is keyed on the ADMITTED-name registry, so every spelling a
    /// translated or hand-edited environment can acquire is caught — not just `sorryAx`.
    #[test]
    fn every_admitted_spelling_is_caught_and_an_ordinary_name_is_not() {
        let any_ty = pi("a", type0(), var("a"));
        let goal = eq_at(nat(), peano(1), peano(0));
        for name in ["sorryAx", "sorry", "admit", "lcProof", "Lean.trustCompiler"] {
            let cert = TheoremCertificate::new(
                "t",
                app(g(name), goal.clone()),
                goal.clone(),
                vec![CertDeclaration {
                    name: name.to_string(),
                    ty: any_ty.clone(),
                }],
                vec![],
            );
            let out = recheck_theorem(&cert);
            assert!(
                !out.rechecks,
                "`{name}` marks a proof as ADMITTED and must be refused"
            );
            assert!(out.error.unwrap_or_default().contains(name));
        }
        // THE NEGATIVE: an ordinary disclosed premise is not an admission. A theorem proved FROM
        // stated hypotheses is the normal case, and refusing it would make the whole surface
        // useless.
        let premise = TheoremCertificate::new(
            "fromPremises",
            g("h1"),
            g("mortalSocrates"),
            vec![
                CertDeclaration {
                    name: "mortalSocrates".to_string(),
                    ty: prop(),
                },
                CertDeclaration {
                    name: "h1".to_string(),
                    ty: g("mortalSocrates"),
                },
            ],
            vec![],
        );
        let out = recheck_theorem(&premise);
        assert!(
            out.rechecks,
            "a disclosed premise is a premise, not an admission: {:?}",
            out.error
        );
        assert!(
            out.axioms.axioms.contains("h1") && !out.axioms.is_admitted(),
            "it is REPORTED as an assumption — which is the honest judgement `premises ⊢ goal` — \
             without being treated as an admission: {:?}",
            out.axioms
        );
    }

    /// The version string is part of the claim. A certificate says "checked against THESE
    /// assumptions", and wave 2 removed one (`funext` became a theorem), so a `logos-coc-2`
    /// certificate describes a base this checker no longer is. The exact current tag is pinned
    /// by `prelude_version_wave3`; this case pins that the wave-2 tag is RETIRED and refused.
    #[test]
    fn the_prelude_version_moves_with_the_axiom_set() {
        use logicaffeine_kernel::certificate::{PRELUDE_VERSION, RETIRED_PRELUDE_VERSIONS};
        assert_ne!(
            PRELUDE_VERSION, "logos-coc-2",
            "the version moved past the wave-2 axiom set"
        );
        assert!(
            RETIRED_PRELUDE_VERSIONS.contains(&"logos-coc-2"),
            "the wave-2 tag is a retired axiom set: {RETIRED_PRELUDE_VERSIONS:?}"
        );
        assert!(
            recheck(&identity_cert()).is_ok(),
            "a certificate minted now re-checks now"
        );
        let mut stale = identity_cert();
        stale.prelude_version = "logos-coc-2".to_string();
        assert!(
            recheck(&stale).is_err(),
            "a certificate minted against a different axiom set must be refused, not quietly \
             re-checked against this one"
        );
    }

    /// THE INTEGRITY RULE, and a hole that was open until wave 2 — **not in the corpus**.
    ///
    /// `Context::add_declaration` and `Context::add_definition` both begin by DISPLACING the name,
    /// so a disclosed delta entry named after a prelude constant does not sit beside it, it
    /// REPLACES it. A certificate could therefore disclose `dne : Π(P:Prop). P` — "every
    /// proposition holds" — prove `False` from it, and re-check GREEN with both kernels agreeing,
    /// while its axiom report named `dne`, a constant that IS in the trusted base. A reader
    /// auditing that certificate would see a proof resting on double-negation elimination.
    ///
    /// Disclosure is only honest if the names in it mean what the reader's own prelude says they
    /// mean. So the delta may EXTEND the trusted base and may never replace part of it.
    #[test]
    fn a_disclosed_delta_may_never_redefine_a_constant_the_trusted_base_owns() {
        // `dne : Π(P:Prop). P` — double-negation elimination with the hypothesis removed, which
        // proves everything. The proof term is honest-looking and type-checks against it.
        let forged = TheoremCertificate::new(
            "falseIsProvable",
            app(g("dne"), g("False")),
            g("False"),
            vec![CertDeclaration {
                name: "dne".to_string(),
                ty: pi("P", prop(), var("P")),
            }],
            vec![],
        );
        let out = recheck_theorem(&forged);
        assert!(
            !out.rechecks,
            "a certificate that REPLACES a trusted-base axiom must not re-check. It would \
             otherwise prove `False`, agree in both kernels, and report an axiom set naming only \
             constants the reader believes they already trust"
        );
        let err = out.error.expect("a refusal says why");
        assert!(
            err.contains("dne") && err.contains("REDEFINES"),
            "the refusal must NAME the constant that was shadowed: `{err}`"
        );

        // A DEFINITION is the same hazard — `Not` unfolds inside half the prelude.
        let redefined_not = TheoremCertificate::new(
            "notIsWhateverISay",
            g("h"),
            app(g("Not"), g("False")),
            vec![CertDeclaration {
                name: "h".to_string(),
                ty: app(g("Not"), g("False")),
            }],
            vec![CertDefinition {
                name: "Not".to_string(),
                ty: pi("P", prop(), prop()),
                body: lam("P", prop(), g("True")),
            }],
        );
        assert!(
            !recheck_theorem(&redefined_not).rechecks,
            "a disclosed DEFINITION may not replace a prelude definition either — `Not` is \
             δ-transparent and unfolds inside half the library"
        );

        // THE NEGATIVE, so this is a rule about COLLISION and not a ban on deltas: a delta of
        // fresh names is the ordinary, honest case and must still re-check.
        let honest = TheoremCertificate::new(
            "fromAFreshPremise",
            g("hFresh"),
            g("myOwnProp"),
            vec![
                CertDeclaration {
                    name: "myOwnProp".to_string(),
                    ty: prop(),
                },
                CertDeclaration {
                    name: "hFresh".to_string(),
                    ty: g("myOwnProp"),
                },
            ],
            vec![],
        );
        let good = recheck_theorem(&honest);
        assert!(
            good.rechecks,
            "a delta of fresh names is legitimate: {:?}",
            good.error
        );
    }

    /// THE REPLAY ORDER, which is a correctness property of the re-checker and not a detail.
    ///
    /// `Context::add_definition` is a CHECKED gateway: it infers the body's type and REFUSES a
    /// body it can neither check nor defer, recording the refusal where a caller reading only
    /// `rechecks` never sees it. A disclosed definition whose body cites a disclosed PREMISE —
    /// the ordinary shape of a theorem proved from hypotheses — is therefore uncheckable until
    /// that premise is in scope. The disclosed delta is a SET, not a script, so the re-checker
    /// imposes the one order that always works: declarations, then definitions.
    #[test]
    fn a_disclosed_definition_may_cite_a_disclosed_premise() {
        let cert = TheoremCertificate::new(
            "fromAPremiseThroughALemma",
            g("viaLemma"),
            g("P"),
            vec![
                CertDeclaration {
                    name: "P".to_string(),
                    ty: prop(),
                },
                CertDeclaration {
                    name: "assumed".to_string(),
                    ty: g("P"),
                },
            ],
            vec![CertDefinition {
                name: "viaLemma".to_string(),
                ty: g("P"),
                body: g("assumed"),
            }],
        );
        let out = recheck_theorem(&cert);
        assert!(
            out.rechecks,
            "a lemma standing on a disclosed premise must re-check: {:?}",
            out.error
        );
        assert!(
            out.axioms.axioms.contains("assumed"),
            "and every premise it rests on is REPORTED — that is the honest judgement \
             `premises ⊢ goal`: {:?}",
            out.axioms
        );
        assert!(
            out.axioms.unresolved.is_empty(),
            "nothing may be left unresolved: an unresolved name here means the definition was \
             refused during the replay and silently dropped, got {:?}",
            out.axioms.unresolved
        );
    }
}

/// THE TRANSITIVE TAINT, at the KERNEL surface and with no REPL in sight — T06-017's real
/// demand, minus the `Print Axioms` command that carries it to a user.
///
/// This is the shape that defeats every cheaper check. `taintedUser`'s proof term is the bare
/// name `taintedLemma`: it contains no `sorry`, no keyword a scan could find, and no warning was
/// ever attached to it. Only a walk of the dependency graph of ELABORATED terms reaches the
/// admission, and only an EXACT walk is worth anything — which is why `cleanThm`, registered in
/// the same context, must come back empty.
#[test]
fn an_admission_is_reported_through_a_lemma_that_never_mentions_it() {
    use logicaffeine_kernel::prelude::axiom_dependencies_of;
    let mut ctx = std_ctx();
    let goal = eq_at(
        g("Int"),
        Term::Lit(Literal::Int(2)),
        Term::Lit(Literal::Int(3)),
    );

    // The hazard, reproduced: a caller reaches past `StandardLibrary::register`.
    ctx.add_declaration("sorryAx", pi("A", type0(), var("A")));
    ctx.add_definition(
        "taintedLemma".to_string(),
        goal.clone(),
        app(g("sorryAx"), goal.clone()),
    );
    ctx.add_definition("taintedUser".to_string(), goal.clone(), g("taintedLemma"));
    let clean_goal = eq_at(
        g("Int"),
        Term::Lit(Literal::Int(3)),
        Term::Lit(Literal::Int(3)),
    );
    ctx.add_definition(
        "cleanThm".to_string(),
        clean_goal.clone(),
        apps(g("refl"), &[g("Int"), Term::Lit(Literal::Int(3))]),
    );

    assert_eq!(
        ctx.get_definition_body("taintedUser")
            .map(|b| b.to_string())
            .as_deref(),
        Some("taintedLemma"),
        "the downstream proof term must be the bare lemma name, or this test is not testing \
         transitivity"
    );

    let tainted = axiom_dependencies_of(&ctx, "taintedUser").expect("registered");
    assert!(
        tainted.admitted().contains("sorryAx"),
        "`taintedUser` rests on an admission one hop away and the report must say so: {:?}",
        tainted
    );
    assert!(tainted.is_admitted());
    assert!(
        tainted
            .render("taintedUser")
            .starts_with("'taintedUser' is ADMITTED"),
        "the rendered report leads with the admission"
    );

    // EXACTNESS, which is the property that makes the report usable at all. A file-wide scan
    // would name `sorryAx` here too, and a report that taints everything taints nothing.
    let clean = axiom_dependencies_of(&ctx, "cleanThm").expect("registered");
    assert!(
        !clean.is_admitted() && clean.axioms.is_empty(),
        "`cleanThm` is proved by `refl` in the SAME context and must rest on nothing: {clean:?}"
    );

    // And the lemma itself reports it directly — the one-hop case must not have been lost while
    // making the two-hop case work.
    let lemma = axiom_dependencies_of(&ctx, "taintedLemma").expect("registered");
    assert!(lemma.admitted().contains("sorryAx"));
}

/// WHY T06-016's OPENING ASSERTION CANNOT HOLD, pinned as the behaviour it actually is.
///
/// The corpus case opens by re-registering `HEq` through the unchecked doors and requires
/// `derive_recursor` to REFUSE it — "an invented `HEq` has no derivable eliminator". That was true
/// when the kernel took an inductive's whole arity as parameters. It is not true now. The
/// prelude's `HEq` has theorems resting on it, so re-registering the identical family KEEPS it
/// (`Context::admit_meaning`: a declared name never changes meaning under what depends on it), and
/// the same family under a fresh name has no previous generation to inherit from.
/// `add_constructor` then INFERS the split by CIC's own rule — the rule Lean applies to
/// `inductive … where` — and reaches `1`, the same split the prelude declares, because that is the
/// split the constructor forces. A family whose eliminator derives is exactly what `HEq` is.
///
/// So the assertion pins a pre-fix limitation rather than a property, and Lean derives `HEq.rec`.
/// This test states the truth in its place, and it fails the moment inference and declaration
/// disagree about one family — which is the property worth having.
#[test]
fn re_registering_heq_infers_the_same_split_the_prelude_declares() {
    use logicaffeine_kernel::derive_recursor;
    let heq_arity = pi(
        "A",
        type0(),
        pi("B", type0(), arrow(var("A"), arrow(var("B"), prop()))),
    );
    let prelude_split = std_ctx().inductive_num_params("HEq");
    let refl_of = |family: &str| {
        pi(
            "A",
            type0(),
            pi(
                "a",
                var("A"),
                apps(g(family), &[var("A"), var("A"), var("a"), var("a")]),
            ),
        )
    };

    // The prelude's own `HEq` has theorems resting on it, and a declared name never changes
    // meaning under what depends on it: re-registering the identical header and constructor is
    // KEPT as it is, so the "one name, two answers" defect has no way in at all.
    let mut again = std_ctx();
    again.add_inductive("HEq", heq_arity.clone());
    again.add_constructor("HEq_refl", "HEq", refl_of("HEq"));
    assert!(
        again.refused_declarations().is_empty(),
        "{:?}",
        again.refused_declarations()
    );
    assert_eq!(
        again.inductive_num_params("HEq"),
        prelude_split,
        "the identical re-registration keeps the prelude's split"
    );

    // The INFERENCE half, on the same family under a fresh name, where there is no previous
    // generation to inherit from.
    let mut adhoc = std_ctx();
    adhoc.add_inductive("HEqAgain", heq_arity);

    // Immediately after the header, before any constructor: the whole arity.
    assert_eq!(
        adhoc.inductive_num_params("HEqAgain"),
        4,
        "a header with no constructor yet has no params/indices split to report"
    );

    adhoc.add_constructor("HEqAgain_refl", "HEqAgain", refl_of("HEqAgain"));
    assert_eq!(
        adhoc.inductive_num_params("HEqAgain"),
        prelude_split,
        "the constructor forces the split, so registering the SAME family through the unchecked \
         door reaches the SAME answer the prelude declares — inference and declaration must not \
         disagree about one type"
    );
    assert!(
        derive_recursor(&adhoc, "HEqAgain").is_ok(),
        "and a family with a correct split has a derivable eliminator; refusing here would mean \
         the kernel could state `HEq` and never eliminate it"
    );

    // THE NEGATIVE, so this is a statement about the SPLIT and not about the name. A constructor
    // that does NOT pass its own binders through unchanged forces zero parameters, and a family
    // with no uniform parameter at all has no eliminator to derive.
    let mut mismatched = std_ctx();
    mismatched.add_inductive(
        "HEqish",
        pi(
            "A",
            type0(),
            pi("B", type0(), arrow(var("A"), arrow(var("B"), prop()))),
        ),
    );
    mismatched.add_constructor(
        "HEqish_odd",
        "HEqish",
        pi(
            "A",
            type0(),
            pi(
                "a",
                var("A"),
                apps(g("HEqish"), &[nat(), var("A"), peano(0), var("a")]),
            ),
        ),
    );
    assert_eq!(
        mismatched.inductive_num_params("HEqish"),
        0,
        "a constructor whose result puts `Nat` where its own first binder should go makes \
         position 0 an INDEX — guessing higher is how an unsound split gets in"
    );
}

/// THE PRELUDE ITSELF MUST BE CLEAN — the lock that catches a definition silently dropped.
///
/// `Context::add_definition` is the `()`-returning face of a CHECKED gateway: a body whose
/// inferred type is not a subtype of the declared one is REFUSED and nothing is registered, with
/// the reason recorded in `refused_declarations` where no caller looks. That is the right
/// behaviour and the wrong failure mode for a library: a prelude constant that quietly failed to
/// register reads downstream as an unbound name, hundreds of lines from the definition that was
/// dropped.
///
/// `pending_obligations` is the other half: a body that mentions a name not yet in scope is
/// DEFERRED rather than skipped, and the deferral is retried when the name arrives. An obligation
/// still standing after `StandardLibrary::register` has finished is a claim nobody discharged.
#[test]
fn the_standard_prelude_registers_with_nothing_refused_and_nothing_still_owed() {
    let ctx = std_ctx();
    assert!(
        ctx.refused_declarations().is_empty(),
        "the CHECKED registration gateway refused these prelude definitions, and a refusal here \
         is invisible until something downstream cites the missing name:\n  {:?}",
        ctx.refused_declarations()
    );
    // THE DEFERRAL RATCHET. A body mentioning a name not yet in scope is DEFERRED rather than
    // skipped, and retried when the name arrives — which is right, and strictly better than Lean's
    // `addDecl`, which refuses a forward reference outright. An obligation still standing after
    // `StandardLibrary::register` has finished is different: nobody is ever going to discharge it,
    // so that definition's BODY WAS NEVER TYPE-CHECKED and sits in the trusted library unverified.
    //
    // Three do, and the cause is named rather than tolerated: `bv_and`/`bv_or`/`bv_xor` are
    // `Term::Fix` bodies whose blocker is the FIXPOINT'S OWN BINDER (`bv_and_rec`), not a global,
    // so no later declaration can ever unblock them. `infer_type`'s `Fix` arm binds that name
    // before the sanity check but `infer_fix_type_structurally` runs FIRST, against the
    // un-extended context, and these three are the shapes whose structural inference has to look
    // inside a branch that makes the recursive call. See the lane report.
    //
    // SUBSET, never equality: the list may only SHRINK. A fourth entry means a new prelude
    // definition went into the library unchecked.
    const NEVER_DISCHARGED: &[&str] = &["bv_and", "bv_or", "bv_xor"];
    let owed = ctx.pending_obligations();
    let unexpected: Vec<_> = owed
        .iter()
        .filter(|(who, _)| !NEVER_DISCHARGED.contains(who))
        .collect();
    assert!(
        unexpected.is_empty(),
        "these prelude definitions were DEFERRED and never discharged — the body was accepted on \
         the promise that the name would arrive, and it did not, so it was never type-checked at \
         all:\n  {unexpected:?}\n(the three in NEVER_DISCHARGED are the known `Fix`-binder case; \
         anything else is new)"
    );
    if owed.is_empty() {
        eprintln!(
            "\n  ✓ every prelude obligation is discharged — delete NEVER_DISCHARGED to bank it.\n"
        );
    }
}

/// `Eq` and `Ex` at EVERY carrier Lean allows, and the boundary one rung either side.
///
/// The universe variable is solved from the ARGUMENT at the application, so the same constant
/// serves `Type 0` and `Type 1` carriers. That must not become "serves anything": choosing an
/// instantiation is not accepting one, and a genuinely ill-typed application still fails.
#[test]
fn equality_and_the_existential_take_their_level_from_the_carrier() {
    let ctx = std_ctx();

    // Rung 0: an ordinary equation between values.
    let at_values = eq_at(nat(), peano(2), peano(2));
    assert!(matches!(
        infer_type(&ctx, &at_values).expect("`Eq Nat 2 2` is a Prop"),
        Term::Sort(Universe::Prop)
    ));

    // Rung 1: an equation between TYPES — `Sort (Type 0) : Type 1`, so this is the instantiation
    // a `Type 0`-pinned carrier refuses. It is what `eq_of_heq` casts along.
    let at_types = eq_at(type0(), nat(), g("Bool"));
    assert!(
        matches!(
            infer_type(&ctx, &at_types).expect("`Eq Type0 Nat Bool` must be stateable"),
            Term::Sort(Universe::Prop)
        ),
        "an equation is a PROPOSITION whatever its carrier lives in — computing `max` for the \
         codomain instead would promote equations out of `Prop` and disable proof irrelevance"
    );
    // …and it is inhabited only where it should be: `Nat = Bool` is stateable, not provable.
    assert!(
        infer_type(&ctx, &apps(g("refl"), &[type0(), nat()])).is_ok(),
        "`refl Type0 Nat : Eq Type0 Nat Nat`"
    );
    let forged = app(
        lam("h", at_types.clone(), g("Zero")),
        apps(g("refl"), &[type0(), nat()]),
    );
    assert!(
        infer_type(&ctx, &forged).is_err(),
        "`refl Type0 Nat` does not prove `Eq Type0 Nat Bool` — stateable is not provable"
    );

    // The existential over a `Type` domain, which is ordinary Lean and was unstateable.
    let over_types = apps(
        g("Ex"),
        &[type0(), lam("a", type0(), eq_at(type0(), var("a"), nat()))],
    );
    assert!(
        matches!(
            infer_type(&ctx, &over_types).expect("`∃ α : Type, α = Nat` must be stateable"),
            Term::Sort(Universe::Prop)
        ),
        "`Ex` lands in `Prop` over ANY domain — that is the impredicativity `imax` gives, and it \
         is what keeps `∃ α : Type, …` a proposition instead of a `Type 1` datum"
    );

    // THE REFUSAL. The level is solved from the argument, and a solution still has to CHECK: a
    // predicate over the wrong domain must not be accepted because the level was flexible.
    assert!(
        infer_type(&ctx, &apps(g("Ex"), &[nat(), lam("a", type0(), g("True"))])).is_err(),
        "`Ex Nat (λα:Type. True)` must be refused — solving `u` from `Nat` does not excuse a \
         predicate whose domain is `Type`"
    );
    assert!(
        infer_type(&ctx, &apps(g("Eq"), &[nat(), g("true"), peano(1)])).is_err(),
        "`Eq Nat true 1` must be refused — `true` is not a `Nat` at any level"
    );

    // And the QUOTIENT carrier, which is the same primitive: a quotient of a `Type 0` carrier
    // lives one rung up.
    let big_rel = lam("a", type0(), lam("b", type0(), g("True")));
    assert!(
        matches!(
            infer_type(&ctx, &apps(g("Quot"), &[type0(), big_rel])).expect("quotienting a Type"),
            Term::Sort(Universe::Type(1))
        ),
        "`Quot Type0 r : Type 1` — the codomain carries the carrier's own level"
    );
    let small_rel = lam("a", nat(), lam("b", nat(), g("True")));
    assert!(
        matches!(
            infer_type(&ctx, &apps(g("Quot"), &[nat(), small_rel]))
                .expect("quotienting a value type"),
            Term::Sort(Universe::Type(0))
        ),
        "…and a quotient of `Nat` stays in `Type 0`; the level is the carrier's, not a constant"
    );
}

// ============================================================================================
// WAVE 3 — THE `Int` ORDER LAYER IS DEFINED, NOT ASSUMED.
//
// `le` is the one opaque order primitive. `lt`, `ge` and `gt` are DEFINITIONS written from it,
// exactly as Lean writes them (`Int.lt a b := a + 1 ≤ b`, `GE.ge a b := LE.le b a`,
// `GT.gt a b := LT.lt b a`), and both directions of the discreteness bridge are theorems whose
// proof is the identity. Four names left the trusted base, and the capability T08-021 needs —
// an order law that CONCLUDES a strict `<` — arrived with them.
//
// The wave-1 assertion this replaces (`the_opaque_int_comparison_builtins_are_left_exactly_as_
// they_were`, `!ctx.is_definition(name)` for all four) was broader than the trap it cites:
// T04-019's trap is "do not close the NAT ordering gap by reusing the opaque Int builtin", and
// that trap is pinned below, by name, without forbidding `lt` a body.
// ============================================================================================

fn intl(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}

/// `(λ(h : claimed). h) proof` — a term that type-checks ONLY when `proof`'s type is
/// definitionally `claimed`. Applying it is what makes the conversion load-bearing, so
/// `double_check` on the result is a differential on the decided VALUE and not on syntax.
fn ascribe(claimed: Term, proof: Term) -> Term {
    app(lam("h", claimed, var("h")), proof)
}

/// The INDEPENDENT model: Rust's own `i64` order. It shares no line of code with either kernel,
/// which is the whole point — `lt`'s answer now comes from `le`'s primitive rule through a δβ
/// step, and a differential against the implementation that produced it would prove nothing.
fn model_order(op: &str, a: i64, b: i64) -> bool {
    match op {
        "le" => a <= b,
        "lt" => a < b,
        "ge" => a >= b,
        "gt" => a > b,
        other => panic!("no model for `{other}`"),
    }
}

/// The adversarial grid: both `i64` ends, both their neighbours, and the sign boundary. `lt`'s
/// body is `le (a + 1) b`, so `a = i64::MAX` is the case where the `+1` LEAVES `i64` — it
/// promotes to `BigInt` rather than wrapping, and a wrap would make `lt i64::MAX b` answer TRUE
/// for every `b`. That single row is why the grid contains the name that breaks it.
const ORDER_GRID: &[i64] = &[i64::MIN, i64::MIN + 1, -1, 0, 1, i64::MAX - 1, i64::MAX];

/// THE POSITIVE, as a two-kernel differential against the ℕ-model of `i64`: every `Int` order
/// operation decides every pair of the grid, both kernels agree on the decision, and the answer
/// is Rust's.
#[test]
fn the_int_order_operations_decide_the_whole_grid_and_agree_with_the_machine() {
    let ctx = std_ctx();
    for op in ["le", "lt", "ge", "gt"] {
        for &a in ORDER_GRID {
            for &b in ORDER_GRID {
                let want = if model_order(op, a, b) {
                    "true"
                } else {
                    "false"
                };
                let applied = apps(g(op), &[intl(a), intl(b)]);
                assert_eq!(
                    normalize(&ctx, &applied),
                    g(want),
                    "`{op} {a} {b}` must compute to `{want}`"
                );
                // DEFINITIONAL, not merely reducible: the kernel has to accept `refl Bool
                // {want}` at `Eq Bool ({op} a b) {want}`, which is what every `Int` order
                // certificate in `logicaffeine_proof` is built out of.
                let term = ascribe(
                    eq_at(g("Bool"), applied.clone(), g(want)),
                    apps(g("refl"), &[g("Bool"), g(want)]),
                );
                assert!(
                    infer_type(&ctx, &term).is_ok(),
                    "`{op} {a} {b}` must be DEFINITIONALLY `{want}`: {:?}",
                    infer_type(&ctx, &term)
                );
                assert_eq!(
                    double_check(&ctx, &term),
                    DoubleCheck::Agreed,
                    "the second de Bruijn kernel must decide `{op} {a} {b}` the same way — a \
                     definition that only the main kernel unfolds is a two-answers bug"
                );
            }
        }
    }
}

/// THE BOUNDARY the grid exists for, on its own so a failure names it: `lt`'s `a + 1` at
/// `i64::MAX` must PROMOTE, not wrap. `lt i64::MAX i64::MAX` is false and
/// `lt (i64::MAX - 1) i64::MAX` is true, and one wrapping `add` inverts the first.
#[test]
fn the_strict_order_promotes_at_the_i64_ceiling_instead_of_wrapping() {
    let ctx = std_ctx();
    assert_eq!(
        normalize(&ctx, &apps(g("lt"), &[intl(i64::MAX), intl(i64::MAX)])),
        g("false"),
        "`MAX < MAX` is false; a wrapping `MAX + 1` would make it `le MIN MAX` and answer true"
    );
    assert_eq!(
        normalize(&ctx, &apps(g("lt"), &[intl(i64::MAX), intl(i64::MIN)])),
        g("false"),
        "`MAX < MIN` is false for the same reason, and this is the row a wrap gets wrong loudest"
    );
    assert_eq!(
        normalize(&ctx, &apps(g("lt"), &[intl(i64::MAX - 1), intl(i64::MAX)])),
        g("true"),
        "…while the rung below the ceiling still decides TRUE, so the fix is not `always false`"
    );
    assert_eq!(
        normalize(&ctx, &apps(g("gt"), &[intl(i64::MIN), intl(i64::MIN)])),
        g("false"),
        "`gt` is `lt` with the arguments swapped, so it inherits the same boundary"
    );
}

/// THE SHAPE. All four `Int` orders carry checked bodies and none is an assumption: `le` is
/// `Int_le`, a match over Lean's two `Int` constructors that bottoms out in `natLe`, and the
/// other three are Lean's own bodies over `le`. A future wave that re-opaques any of them takes a
/// proved fact back on trust.
#[test]
fn every_int_order_carries_a_checked_body_and_the_strict_three_are_leans_own() {
    let ctx = std_ctx();
    let int_bool = pi("_", g("Int"), pi("_", g("Int"), g("Bool")));

    for name in ["le", "lt", "ge", "gt"] {
        let ty = infer_type(&ctx, &g(name)).unwrap_or_else(|e| panic!("`{name}` exists: {e:?}"));
        assert!(
            defeq_for_test(&ctx, &ty, &int_bool),
            "`{name}` must still be `Int → Int → Bool`, not `{ty}` — a body may not change a \
             constant's TYPE, only stop it being an assumption"
        );
        assert!(
            ctx.is_definition(name),
            "`{name}` is DEFINABLE over Lean's `Int` constructors, and Lean defines it"
        );
        assert!(
            !axioms(&ctx).contains(&name.to_string()),
            "`{name}` must be gone from the trusted base, not merely given a body beside its \
             declaration"
        );
    }
    assert_eq!(
        ctx.get_definition_body("le"),
        Some(&g("Int_le")),
        "`le` is the checked public alias of the constructor-matching `Int_le`"
    );
    assert!(
        ctx.is_inductive("Int") && ctx.get_constructors("Int").len() == 2,
        "…which exists because `Int` has Lean's two constructors"
    );
    assert!(
        !defeq_for_test(
            &ctx,
            &apps(g("lt"), &[intl(3), intl(3)]),
            &apps(g("le"), &[intl(3), intl(3)]),
        ),
        "`lt` and `le` must still differ on the diagonal — a body that collapsed the strict order \
         onto `le` would make `3 < 3` true"
    );

    // The bodies are the ones named in the argument, checked by δ-unfolding rather than by
    // reading the source: `lt a b ≡ le (a+1) b`, `ge a b ≡ le b a`, `gt a b ≡ le (b+1) a`.
    let a = || intl(7);
    let b = || intl(-3);
    let le_of = |x: Term, y: Term| apps(g("le"), &[x, y]);
    for (spelt, meant) in [
        (
            apps(g("lt"), &[a(), b()]),
            le_of(apps(g("add"), &[a(), intl(1)]), b()),
        ),
        (apps(g("ge"), &[a(), b()]), le_of(b(), a())),
        (
            apps(g("gt"), &[a(), b()]),
            le_of(apps(g("add"), &[b(), intl(1)]), a()),
        ),
    ] {
        assert!(
            defeq_for_test(&ctx, &spelt, &meant),
            "`{spelt}` must be DEFINITIONALLY `{meant}` — the body is the specification, and a \
             body that merely agrees on literals is a second implementation"
        );
    }
}

/// THE TRAP T04-019 ACTUALLY NAMES, pinned by name instead of by "no body anywhere": the `Nat`
/// ordering gap must not be closed by pointing at the `Int` builtin. `Int`'s `lt` is not
/// applicable to a `Nat` at all, and `Nat`'s own order is a separate, proved, computing layer.
#[test]
fn the_nat_order_is_still_its_own_proved_layer_and_the_int_builtin_cannot_stand_in() {
    let ctx = std_ctx();

    // `lt Zero Zero` is ILL-TYPED: `Zero : Nat`, and `lt : Int → Int → Bool`. This is the
    // assertion that makes "renaming closed the gap" impossible, and it survives `lt` having a
    // body because a body cannot widen a domain.
    assert!(
        infer_type(&ctx, &apps(g("lt"), &[g("Zero"), g("Zero")])).is_err(),
        "the `Int` strict order must not accept a `Nat` — that substitution IS the trap"
    );
    assert!(
        infer_type(&ctx, &apps(g("Nat_lt"), &[intl(0), intl(1)])).is_err(),
        "…and the reverse substitution is refused too: `Nat_lt` is not an `Int` relation"
    );

    // `Nat`'s order is a separate, PROVEN layer that decides.
    for name in ["Nat_le", "Nat_lt", "decLeNat", "decLtNat", "natLe", "natLt"] {
        assert!(
            ctx.is_definition(name),
            "`{name}` must stay a proved definition over `Nat`'s constructors"
        );
    }
    assert_eq!(
        normalize(
            &ctx,
            &apps(
                g("decide"),
                &[
                    apps(g("Nat_lt"), &[natl(2), natl(3)]),
                    apps(g("decLtNat"), &[natl(2), natl(3)]),
                ],
            ),
        ),
        g("true"),
        "and it still computes — closing the Int order gap must not have moved the Nat one"
    );
}

/// THE BRIDGE, both directions, and the point of the whole exercise: an order law that
/// CONCLUDES a strict `<`. Wave 1 assumed the forward direction and the backward one did not
/// exist, which is why `calc` could not compose a mixed `≤`/`<` chain (T08-021).
#[test]
fn the_discreteness_bridge_is_a_theorem_in_both_directions_and_rests_on_the_alu_alone() {
    use logicaffeine_kernel::axiom_dependencies_of;
    let ctx = std_ctx();

    for name in ["lt_succ_le", "le_succ_lt"] {
        assert!(
            ctx.is_definition(name),
            "`{name}` must be a kernel-checked THEOREM: `lt a b` IS `le (a+1) b`, so the \
             identity inhabits each direction from the other and assuming either is assuming a \
             conversion"
        );
        assert!(
            !axioms(&ctx).contains(&name.to_string()),
            "`{name}` must be out of the trusted base"
        );
        let body = ctx
            .get_definition_body(name)
            .cloned()
            .unwrap_or_else(|| panic!("`{name}` has a body"));
        let declared = ctx
            .get_definition_type(name)
            .cloned()
            .unwrap_or_else(|| panic!("`{name}` has a declared type"));
        let inferred = infer_type(&ctx, &body)
            .unwrap_or_else(|e| panic!("`{name}`'s proof term must kernel-check: {e:?}"));
        assert!(
            logicaffeine_kernel::is_subtype(&ctx, &inferred, &declared),
            "`{name}`'s body proves `{inferred}` but it is declared at `{declared}`"
        );
        assert_eq!(
            double_check(&ctx, &body),
            DoubleCheck::Agreed,
            "the second de Bruijn kernel must corroborate `{name}` — the conversion it turns on \
             is δ into the new `lt` body, which is exactly the kind of step two kernels can \
             disagree about"
        );
        // What it rests on: NOTHING. Its statement mentions `add` and `le`, and since `Int`
        // gained Lean's constructors both are checked definitions, so the whole closure is
        // proved. In particular one direction must not be proved by citing the other.
        let usage = axiom_dependencies_of(&ctx, name).expect("registered");
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "`{name}` must rest on no assumption at all: {usage:?}"
        );
        let other = if name == "lt_succ_le" {
            "le_succ_lt"
        } else {
            "lt_succ_le"
        };
        assert!(
            !format!("{body:?}").contains(&format!("Global({other:?})")),
            "`{name}` must be proved directly, never by citing its converse `{other}`: {body}"
        );
    }

    // THE CAPABILITY, spelled as the step T08-021's mixed chain needs: from `(a + 1) ≤ b`
    // conclude `a < b`, at symbolic arguments rather than literals.
    let sym = ctx.extend("x", g("Int")).extend("y", g("Int"));
    let le_prop = |x: Term, y: Term| eq_at(g("Bool"), apps(g("le"), &[x, y]), g("true"));
    let lt_prop = |x: Term, y: Term| eq_at(g("Bool"), apps(g("lt"), &[x, y]), g("true"));
    let hyp = le_prop(apps(g("add"), &[var("x"), intl(1)]), var("y"));
    let step = lam(
        "h",
        hyp.clone(),
        apps(g("le_succ_lt"), &[var("x"), var("y"), var("h")]),
    );
    assert!(
        logicaffeine_kernel::is_subtype(
            &sym,
            &infer_type(&sym, &step).expect("the untightening step type-checks"),
            &arrow(hyp, lt_prop(var("x"), var("y"))),
        ),
        "`le_succ_lt x y : (x+1) ≤ y → x < y` — the one step no assumed order law could take"
    );

    // THE NEGATIVE, one rung either side: the bridge is between `a < b` and `(a+1) ≤ b`, NOT
    // between `a < b` and `a ≤ b`. If the identity inhabited THAT, `lt` would be `le` and the
    // strict order would have collapsed.
    let collapse = lam("h", le_prop(var("x"), var("y")), var("h"));
    let collapse_claim = arrow(le_prop(var("x"), var("y")), lt_prop(var("x"), var("y")));
    assert!(
        !logicaffeine_kernel::is_subtype(
            &sym,
            &infer_type(&sym, &collapse).expect("the identity type-checks at its own type"),
            &collapse_claim,
        ),
        "`x ≤ y → x < y` must NOT hold by conversion — `lt` is `le` SHIFTED BY ONE, and a \
         definition that lost the shift would make every non-strict bound strict"
    );
    assert!(
        normalize(&ctx, &apps(g("lt"), &[intl(3), intl(3)])) == g("false")
            && normalize(&ctx, &apps(g("le"), &[intl(3), intl(3)])) == g("true"),
        "…witnessed at the boundary where they differ: `3 ≤ 3` and `¬(3 < 3)`"
    );
}

// ============================================================================================
// WAVE 3 — `Decidable` INSTANCES. The prelude's decision procedures must be FINDABLE.
// ============================================================================================

/// THE POSITIVE: the goals an auto-param obligation `(h : b ≠ 0 := by decide)` actually raises
/// resolve, and the proof they produce kernel-checks in both kernels. This is T05-007's prelude
/// half, end to end — the elaborator's half is `elaborate.rs`'s.
#[test]
fn the_decidable_instances_resolve_nat_equality_and_its_negation_and_the_proof_checks() {
    let ctx = std_ctx();
    let dec = |p: Term| app(g("Decidable"), p);
    let not = |p: Term| app(g("Not"), p);
    let ne_zero = |n: Term| not(eq_at(nat(), n, g("Zero")));

    // `Decidable (Eq Nat 2 0)` — the inner goal.
    let inner = resolve(&ctx, &dec(eq_at(nat(), peano(2), g("Zero"))))
        .expect("`Decidable (Eq Nat 2 0)` must resolve — `decEqNat` is proved and must be found");
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("decide"), &[eq_at(nat(), peano(2), g("Zero")), inner])
        ),
        g("false"),
        "…and it DECIDES: `2 = 0` is false"
    );

    // `Decidable (2 ≠ 0)` — the goal `by decide` is actually handed, which needs a premise to be
    // discharged recursively.
    let goal = ne_zero(peano(2));
    let inst = resolve(&ctx, &dec(goal.clone()))
        .expect("`Decidable (2 ≠ 0)` must resolve through `instDecidableNot`");
    assert_eq!(
        normalize(&ctx, &apps(g("decide"), &[goal.clone(), inst.clone()])),
        g("true"),
        "`2 ≠ 0` decides TRUE — an instance that resolves without deciding is a placeholder"
    );

    // THE PROOF. `of_decide_eq_true p inst (refl Bool true) : p` is what the discharged
    // obligation must be, and it is proved from `Decidable_rec` with zero axioms.
    let proof = apps(
        g("of_decide_eq_true"),
        &[
            goal.clone(),
            inst.clone(),
            apps(g("refl"), &[g("Bool"), g("true")]),
        ],
    );
    let proved = infer_type(&ctx, &proof)
        .unwrap_or_else(|e| panic!("the discharged obligation must kernel-check: {e:?}"));
    assert!(
        logicaffeine_kernel::is_subtype(&ctx, &proved, &goal),
        "the obligation must prove `2 ≠ 0`, got `{proved}`"
    );
    assert_eq!(
        double_check(&ctx, &proof),
        DoubleCheck::Agreed,
        "both kernels must accept the discharged obligation"
    );

    // THE BOUNDARY, one rung the other side: the SAME construction at a TRUE equation must be
    // refused. `2 ≠ 2` decides false, so `refl Bool true` does not fit and no proof of `2 ≠ 2`
    // can be built this way. Without this, an instance that always answered `isTrue` would pass
    // everything above.
    let false_goal = ne_zero(g("Zero"));
    let false_inst = resolve(&ctx, &dec(false_goal.clone()))
        .expect("`Decidable (0 ≠ 0)` resolves — it is decidable, and it decides FALSE");
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("decide"), &[false_goal.clone(), false_inst.clone()])
        ),
        g("false"),
        "`0 ≠ 0` decides FALSE"
    );
    assert!(
        infer_type(
            &ctx,
            &apps(
                g("of_decide_eq_true"),
                &[
                    false_goal,
                    false_inst,
                    apps(g("refl"), &[g("Bool"), g("true")])
                ],
            ),
        )
        .is_err(),
        "a FALSE obligation must not be dischargeable — `decide` is `false` there and \
         `refl Bool true` cannot be coerced into it"
    );
}

/// THE COST: nothing. `instDecidableNot` is a kernel-checked definition, not an assumption —
/// which matters more than it looks, because an axiom of type `Π(p:Prop). Decidable p` inhabits
/// the class for EVERY proposition and `of_decide_eq_true` would turn it into a proof of
/// anything.
#[test]
fn the_decidable_layer_costs_the_trusted_base_nothing_and_stays_out_of_the_user_record() {
    use logicaffeine_kernel::axiom_dependencies_of;
    let ctx = std_ctx();

    assert!(
        ctx.is_definition("instDecidableNot"),
        "`instDecidableNot` must carry a body — it is `Decidable_rec` with the constructors \
         swapped, and Lean proves it too"
    );
    assert!(
        !axioms(&ctx).contains(&"instDecidableNot".to_string()),
        "…so it is not in the trusted base"
    );
    let usage = axiom_dependencies_of(&ctx, "instDecidableNot").expect("registered");
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "`instDecidableNot` must rest on NOTHING — `Decidable`, `Not`, `False` and the two \
         constructors are all inductive: {usage:?}"
    );

    // THE TIER. These are FALLBACK entries: `Context::instances()` is the faithful record of
    // what a USER declared and is resolved first-match, so a prelude entry there would be entry
    // zero and would outrank every instance anyone ever writes for the same goal.
    let declared: Vec<String> = ctx
        .instances()
        .iter()
        .map(|(ty, _)| format!("{ty}"))
        .collect();
    for needle in ["Decidable"] {
        assert!(
            !declared.iter().any(|t| t.contains(needle)),
            "the prelude's `{needle}` instances must live in the FALLBACK tier, not in the \
             declared record: {declared:?}"
        );
    }

    // …and a user instance for the same goal still wins, which is what the tier is FOR.
    let mut user = std_ctx();
    let goal = app(g("Decidable"), eq_at(nat(), g("Zero"), g("Zero")));
    let mine = apps(
        g("isTrue"),
        &[
            eq_at(nat(), g("Zero"), g("Zero")),
            apps(g("refl"), &[nat(), g("Zero")]),
        ],
    );
    user.add_instance(goal.clone(), mine.clone());
    assert_eq!(
        resolve(&user, &goal).expect("the user instance resolves"),
        mine,
        "a declared instance must outrank the prelude's fallback — otherwise the prelude has \
         quietly taken the decision away from the user"
    );
}

// ============================================================================================
// WAVE 3 — `Option` IS AN ORDINARY INDUCTIVE, AND NO MONAD OPERATION IS SMUGGLED IN.
//
// `lean_conformance_t05_elaboration::do_notation_is_refused_and_never_half_translated` lists `Option`
// among fifteen names that "must not exist — a do-block operation smuggled in as a global or an
// axiom". `Option` is not a do-block operation: it is an ordinary two-constructor inductive in
// Lean's own `Init.Prelude`, and the thing that case is actually guarding — that no monad
// vocabulary was axiomatised to make `do` half-work — is stated here at class level, where
// `Option` being a checked inductive is EVIDENCE FOR the guard rather than a breach of it.
// ============================================================================================

#[test]
fn option_is_a_checked_inductive_and_the_monad_vocabulary_is_still_absent() {
    let ctx = std_ctx();

    // `Option` is an inductive with Lean's two constructors, in Lean's order — not a
    // declaration, and above all not an axiom.
    assert!(
        ctx.is_inductive("Option"),
        "`Option` is an inductive, not an opaque constant"
    );
    assert!(
        !axioms(&ctx).contains(&"Option".to_string()),
        "`Option` must cost the trusted base nothing — an AXIOMATISED `Option` would be the \
         smuggling the do-notation case is warning about"
    );
    assert_eq!(
        ctx.get_constructors("Option")
            .iter()
            .map(|(c, _)| *c)
            .collect::<Vec<_>>(),
        vec!["none", "some"],
        "Lean's `Option` is `none | some`, in that order"
    );
    for c in ["none", "some"] {
        assert!(ctx.is_constructor(c), "`{c}` is a constructor of `Option`");
    }
    // It COMPUTES, which no axiomatised stand-in could — and THAT is the distinction the
    // do-notation case is actually drawing: an axiomatised `bind`/`pure` type-checks and never
    // reduces, so every `rfl` the source proved by running the block silently stops holding.
    // `Option` is the opposite shape: a real inductive whose match runs.
    let opt_nat = app(g("Option"), nat());
    let get_or_zero = |o: Term| Term::Match {
        discriminant: Box::new(o),
        motive: Box::new(lam("_", opt_nat.clone(), nat())),
        cases: vec![g("Zero"), lam("x", nat(), var("x"))],
    };
    assert_eq!(
        normalize(&ctx, &get_or_zero(apps(g("some"), &[nat(), peano(1)]))),
        peano(1),
        "`(some 1).getD 0 ⤳ 1` — an axiom could never do this"
    );
    assert_eq!(
        normalize(&ctx, &get_or_zero(app(g("none"), nat()))),
        g("Zero"),
        "…and `none` takes the other branch"
    );

    // THE GUARD THE CASE IS ACTUALLY ABOUT, restated so it cannot be satisfied by `Option`
    // existing or not existing: no monad CLASS and no `do` OPERATION, under any spelling, as a
    // global, an axiom, a definition or an inductive.
    for name in [
        "Monad",
        "Bind",
        "Pure",
        "Functor",
        "Applicative",
        "ForIn",
        "bind",
        "pure",
        "Bind_bind",
        "Pure_pure",
        "ForIn_forIn",
        "Id_run",
        "SeqLeft",
        "SeqRight",
    ] {
        assert!(
            ctx.get_global(name).is_none(),
            "`{name}` must not exist — a do-block operation smuggled in as a global or an axiom"
        );
    }
    for (n, _) in ctx.iter_declarations() {
        assert!(
            ![
                "Monad",
                "Bind",
                "Pure",
                "Functor",
                "Applicative",
                "EStateM",
                "RealWorld"
            ]
            .iter()
            .any(|b| n.contains(b)),
            "the prelude must declare no monad axiom, found `{n}`"
        );
    }
}

// ============================================================================================
// WAVE 3 — THE AXIOM CLOSURE SEES AN INDUCTIVE'S TYPES, AND A CERTIFICATE'S STATEMENT.
// ============================================================================================

/// THE POSITIVE, through a CONSTRUCTOR's type: an axiom reachable only there must be reported.
/// A constructor that takes a `Quot A r` over an ASSUMED carrier makes a proof rest on that
/// carrier — the walk passes through the quotient former, which is a primitive identity and no
/// assumption leaf — and Lean's `#print axioms` traverses exactly those types.
#[test]
fn the_axiom_closure_follows_an_inductives_arity_and_its_constructors_types() {
    use logicaffeine_kernel::axiom_dependencies;
    let mut ctx = std_ctx();

    // An axiom reachable ONLY through a constructor's argument type.
    ctx.add_declaration("hiddenCarrier", type0());
    ctx.add_inductive("Box", type0());
    ctx.add_constructor("boxUp", "Box", arrow(g("hiddenCarrier"), g("Box")));
    let boxed = axiom_dependencies(&ctx, &g("Box"));
    assert!(
        boxed.axioms.contains("hiddenCarrier"),
        "an axiom named only in a CONSTRUCTOR's type is still a dependency: {boxed:?}"
    );

    // …and one reachable only through the inductive's ARITY.
    ctx.add_declaration("hiddenIndex", type0());
    ctx.add_indexed_inductive("Tagged", pi("t", g("hiddenIndex"), type0()), 1);
    let tagged = axiom_dependencies(&ctx, &g("Tagged"));
    assert!(
        tagged.axioms.contains("hiddenIndex"),
        "an axiom named only in an inductive's ARITY is still a dependency: {tagged:?}"
    );

    // THE REAL SHAPE this exists for: a constructor over a quotient of an ASSUMED carrier.
    ctx.add_declaration("quotCarrier", type0());
    let rel = || arrow(g("quotCarrier"), arrow(g("quotCarrier"), prop()));
    ctx.add_inductive("QBox", type0());
    ctx.add_constructor(
        "qbox",
        "QBox",
        pi(
            "r",
            rel(),
            arrow(apps(g("Quot"), &[g("quotCarrier"), var("r")]), g("QBox")),
        ),
    );
    let qboxed = axiom_dependencies(&ctx, &g("QBox"));
    assert!(
        qboxed.axioms.contains("quotCarrier"),
        "a constructor taking a `Quot quotCarrier r` makes a proof rest on `quotCarrier`: {qboxed:?}"
    );
    assert!(
        !qboxed.axioms.contains("Quot"),
        "the walk passes THROUGH the quotient former, which is a primitive identity: {qboxed:?}"
    );

    // THE NEGATIVE, which is what stops this being satisfied by naming every constant in sight:
    // a neighbouring inductive whose arity and constructors mention no assumption must report
    // NOTHING — and `Nat` is also the RECURSION check, since `Succ : Nat → Nat` re-enters the
    // name being walked.
    let plain = axiom_dependencies(&ctx, &nat());
    assert!(
        plain.axioms.is_empty() && plain.unresolved.is_empty(),
        "`Nat` rests on nothing and its own recursive constructor must not loop: {plain:?}"
    );
    let boolean = axiom_dependencies(&ctx, &g("Bool"));
    assert!(
        boolean.axioms.is_empty() && boolean.unresolved.is_empty(),
        "`Bool` rests on nothing either: {boolean:?}"
    );
    // And a MUTUALLY recursive pair terminates for the same reason — `seen` is keyed on the
    // name, so each is expanded once.
    ctx.add_inductive("Ping", type0());
    ctx.add_inductive("Pong", type0());
    ctx.add_constructor("ping", "Ping", arrow(g("Pong"), g("Ping")));
    ctx.add_constructor("pong", "Pong", arrow(g("Ping"), g("Pong")));
    let mutual = axiom_dependencies(&ctx, &g("Ping"));
    assert!(
        mutual.axioms.is_empty(),
        "a mutually recursive pair must terminate and rest on nothing: {mutual:?}"
    );
}

/// `Eq Bool ((λ_:Float→Float→Float. true) Float_add) true` — a statement that is true by
/// β-conversion and whose only assumption, the opaque `Float_add`, sits in the STATEMENT.
fn float_statement() -> Term {
    let float_op = arrow(g("Float"), arrow(g("Float"), g("Float")));
    eq_at(
        g("Bool"),
        app(lam("_", float_op, g("true")), g("Float_add")),
        g("true"),
    )
}

/// The two doors must answer the same question the same way. `axiom_dependencies_of` (the
/// REPL's `Print Axioms` door) and `axiom_dependencies` (the certificate door) read one closure,
/// so a constant whose TYPE cites an axiom reports it whichever way you come in.
#[test]
fn both_axiom_doors_agree_about_a_dependency_that_is_only_in_a_type() {
    use logicaffeine_kernel::{axiom_dependencies, axiom_dependencies_of};
    let mut ctx = std_ctx();
    // `bound : Eq Bool ((λ_:Float→Float→Float. true) Float_add) true := refl Bool true` — the
    // BODY names no axiom at all; the STATEMENT names the opaque primitive `Float_add`, inside a
    // redex, so conversion discharges it and only a walk of the type can see it.
    ctx.add_definition(
        "bound".to_string(),
        float_statement(),
        apps(g("refl"), &[g("Bool"), g("true")]),
    );
    assert!(
        ctx.is_definition("bound"),
        "`bound` is well typed: {:?}",
        ctx.refused_declarations()
    );
    let named = axiom_dependencies_of(&ctx, "bound").expect("registered");
    assert!(
        named.axioms.contains("Float_add"),
        "the named-constant door must report `Float_add`: {named:?}"
    );
    // `le` itself is no longer an assumption (it is `Int_le` over Lean's constructors), so a
    // statement about `le` alone rests on nothing through either door.
    ctx.add_definition(
        "leBound".to_string(),
        eq_at(g("Bool"), apps(g("le"), &[intl(2), intl(3)]), g("true")),
        apps(g("refl"), &[g("Bool"), g("true")]),
    );
    let le_named = axiom_dependencies_of(&ctx, "leBound").expect("registered");
    assert!(
        le_named.axioms.is_empty() && le_named.unresolved.is_empty(),
        "`2 ≤ 3` over the defined `le` rests on nothing: {le_named:?}"
    );
    assert_eq!(
        le_named.axioms,
        axiom_dependencies(&ctx, &g("leBound")).axioms
    );
    let termwise = axiom_dependencies(&ctx, &g("bound"));
    assert_eq!(
        named.axioms, termwise.axioms,
        "the two doors must not disagree about one graph"
    );
    // The NEGATIVE: the proof term alone rests on nothing, so the report is not "every prelude
    // axiom".
    let bare = axiom_dependencies(&ctx, &apps(g("refl"), &[g("Bool"), g("true")]));
    assert!(
        bare.axioms.is_empty(),
        "`refl Bool true` on its own rests on nothing: {bare:?}"
    );
}

#[cfg(feature = "serde")]
mod statement_audit {
    use super::*;
    use logicaffeine_kernel::certificate::{
        recheck, recheck_theorem, CertDeclaration, Certificate, TheoremCertificate,
    };

    /// THE POSITIVE. A certificate is its proof term AND its claimed proposition, so an axiom
    /// named only in the STATEMENT is a real dependency. `refl Bool true` proves
    /// `Eq Bool ((λ_. true) Float_add) true` by conversion and mentions no constant at all — a
    /// report that walked only the proof term would say this theorem about the opaque `Float_add`
    /// rests on nothing.
    #[test]
    fn a_certificates_statement_is_part_of_what_it_rests_on() {
        let stated = Certificate::new(apps(g("refl"), &[g("Bool"), g("true")]), float_statement());
        assert!(
            recheck(&stated).is_ok(),
            "the β-redex statement really is proved by `refl Bool true`: {:?}",
            recheck(&stated)
        );
        let usage = stated.axioms_used();
        assert!(
            usage.axioms.contains("Float_add"),
            "the axiom the STATEMENT rests on must be reported: {:?}",
            usage.axioms
        );

        // THE NEGATIVE, so this cannot be satisfied by naming every prelude axiom: a
        // neighbouring certificate whose statement names nothing reports nothing.
        let silent = Certificate::new(
            apps(g("refl"), &[g("Bool"), g("true")]),
            eq_at(g("Bool"), g("true"), g("true")),
        );
        assert!(recheck(&silent).is_ok());
        let quiet = silent.axioms_used();
        assert!(
            quiet.axioms.is_empty() && quiet.unresolved.is_empty(),
            "`true = true` rests on nothing: {quiet:?}"
        );
    }

    /// THE TAINT THAT COULD BE LOST. An admission hidden in the STATEMENT — inside a redex, so
    /// the proposition is definitionally clean and an ordinary proof discharges it — used to
    /// re-check GREEN with `sorryAx` reported nowhere. The audit trail is only worth something
    /// if it cannot be walked around, and the walk-around was "put it in the type".
    #[test]
    fn an_admission_hidden_in_a_statement_is_still_an_admission() {
        // `Eq Nat 0 ((λ_:Nat. 0) (sorryAx Nat))` — β-equal to `Eq Nat 0 0`, so `refl Nat 0`
        // proves it and the proof term never mentions the admission.
        let hidden = app(lam("_", nat(), g("Zero")), app(g("sorryAx"), nat()));
        let claimed = eq_at(nat(), g("Zero"), hidden);
        let cert = TheoremCertificate::new(
            "looksClean",
            apps(g("refl"), &[nat(), g("Zero")]),
            claimed.clone(),
            vec![CertDeclaration {
                name: "sorryAx".to_string(),
                ty: pi("A", type0(), var("A")),
            }],
            vec![],
        );
        let out = recheck_theorem(&cert);
        assert!(
            out.axioms.admitted().contains("sorryAx"),
            "the admission is in the STATEMENT and must be reported: {:?}",
            out.axioms
        );
        assert!(
            !out.rechecks,
            "…and a proposition nobody should accept at face value must not re-check: {:?}",
            out.error
        );
        assert!(
            out.error.unwrap_or_default().contains("sorryAx"),
            "the refusal must name it"
        );

        // THE BOUNDARY: the same certificate with the redex removed is an ordinary, clean
        // theorem and re-checks. The refusal is about the admission, not about redexes.
        let honest = TheoremCertificate::new(
            "reallyClean",
            apps(g("refl"), &[nat(), g("Zero")]),
            eq_at(nat(), g("Zero"), app(lam("_", nat(), g("Zero")), g("Zero"))),
            vec![],
            vec![],
        );
        let good = recheck_theorem(&honest);
        assert!(good.rechecks, "the clean twin re-checks: {:?}", good.error);
        assert!(
            !good.axioms.is_admitted(),
            "and is untainted: {:?}",
            good.axioms
        );
    }
}

// ============================================================================================
// WAVE 3 — A DEFECT THAT IS NOT IN THE CORPUS: the three trust queries disagreed about
// `unresolved`.
//
// `AxiomUsage` answers "what does this rest on?" three ways. `admitted()` chained `axioms` and
// `unresolved`; `beyond()` and `is_tainted_by()` read `axioms` ALONE. The two that read one half
// are the two a trust POLICY is naturally written from — "rests on nothing outside this list",
// "is this tainted?" — and both answered NO for a proof citing `sorryAx`, because a certificate
// re-checked against the clean prelude finds `sorryAx` UNRESOLVED (the clean prelude never
// declared it) rather than in `axioms`. The one shape the audit trail exists to catch was the
// one shape two of its three questions could not see.
// ============================================================================================

#[test]
fn every_way_of_asking_what_a_proof_rests_on_counts_a_name_the_trusted_base_lacks() {
    use logicaffeine_kernel::{axiom_dependencies, ADMITTED_AXIOMS};
    let ctx = std_ctx();

    // The shape: an admitted proof, re-checked against a context that never declared `sorryAx`.
    let admitted = app(g("sorryAx"), eq_at(nat(), peano(1), g("Zero")));
    let u = axiom_dependencies(&ctx, &admitted);
    assert!(
        u.unresolved.contains("sorryAx") && u.axioms.is_empty(),
        "against a CLEAN prelude an admission is `unresolved`, not `axioms` — that is what \
         makes it the dangerous half: {u:?}"
    );
    assert!(u.is_admitted(), "the chained view has always seen it");
    assert!(
        u.is_tainted_by(ADMITTED_AXIOMS),
        "…and so must the taint query — `is_tainted_by(ADMITTED_AXIOMS)` is the obvious spelling \
         of `is_admitted`, and the two may not disagree"
    );
    assert!(
        !u.beyond(&[]).is_empty(),
        "…and so must the policy query: a proof citing a name this base does not have rests on \
         SOMETHING, and `beyond(&[])` reporting nothing is a licence to accept it"
    );
    assert!(
        u.beyond(&["sorryAx"]).is_empty(),
        "permitting a name by spelling must silence it from whichever half it landed in — one \
         list, one rule"
    );

    // THE NEGATIVE: a clean proof answers clean to every one of the three, so this cannot be
    // satisfied by making every query say yes.
    let clean = axiom_dependencies(&ctx, &apps(g("refl"), &[nat(), g("Zero")]));
    assert!(
        !clean.is_admitted()
            && !clean.is_tainted_by(ADMITTED_AXIOMS)
            && clean.beyond(&[]).is_empty(),
        "`refl Nat 0` rests on nothing by every query: {clean:?}"
    );

    // THE BOUNDARY: a DECLARED axiom is reported by the policy query and is NOT an admission —
    // tracking is not banning, and the two halves must not be conflated in the other direction.
    let choice = axiom_dependencies(
        &ctx,
        &apps(g("Classical_choice"), &[nat(), g("nonemptyNat")]),
    );
    assert!(
        !choice.beyond(&[]).is_empty() && choice.beyond(&["Classical_choice"]).is_empty(),
        "`Classical_choice` is reported, and permitted by name: {choice:?}"
    );
    assert!(
        !choice.is_admitted() && !choice.is_tainted_by(ADMITTED_AXIOMS),
        "one of Lean's own three is an assumption, never an admission"
    );
}

/// The same two doors, for an INDUCTIVE. `axiom_dependencies_of` used to walk the inductive's
/// arity by hand while `axiom_dependencies` walks its arity AND its constructors, so the named
/// door under-reported exactly the dependency the term door had just learned to see. Two walks
/// over one graph is two answers; there is now one walk and this pins it.
#[test]
fn the_named_door_and_the_term_door_take_the_same_walk_over_an_inductive() {
    use logicaffeine_kernel::{axiom_dependencies, axiom_dependencies_of};
    let mut ctx = std_ctx();
    ctx.add_declaration("secretCarrier", type0());
    ctx.add_inductive("Crate", type0());
    ctx.add_constructor("crateUp", "Crate", arrow(g("secretCarrier"), g("Crate")));

    let named = axiom_dependencies_of(&ctx, "Crate").expect("registered");
    let termwise = axiom_dependencies(&ctx, &g("Crate"));
    assert_eq!(
        named.axioms, termwise.axioms,
        "`Print Axioms Crate.` and the certificate closure must not disagree"
    );
    assert!(
        named.axioms.contains("secretCarrier"),
        "…and both must see the axiom that is only in a CONSTRUCTOR's type: {named:?}"
    );
    // A CONSTRUCTOR asked by name reports its own inductive's closure, not a bare type walk.
    let ctor = axiom_dependencies_of(&ctx, "crateUp").expect("registered");
    assert!(
        ctor.axioms.contains("secretCarrier"),
        "a constructor rests on what its own argument types rest on: {ctor:?}"
    );
    // NEITHER door may report the inductive itself as an axiom — an inductive is a declaration,
    // not an assumption, and the whole classification turns on that.
    assert!(
        !named.axioms.contains("Crate") && !termwise.axioms.contains("Crate"),
        "an inductive is not an axiom: {named:?}"
    );
    let plain = axiom_dependencies_of(&ctx, "Nat").expect("registered");
    assert!(
        plain.axioms.is_empty() && plain.unresolved.is_empty(),
        "`Nat` rests on nothing through either door: {plain:?}"
    );
}

#[cfg(feature = "serde")]
mod prelude_version_wave3 {
    use super::*;
    use logicaffeine_kernel::certificate::{recheck, Certificate, PRELUDE_VERSION};

    /// The version string is part of the claim, and wave 3 removed four assumptions — `lt`,
    /// `ge`, `gt` and `lt_succ_le` all became definitions — so a `logos-coc-3` certificate
    /// describes a base this checker no longer is.
    #[test]
    fn the_prelude_version_moved_again_when_the_int_order_stopped_being_assumed() {
        assert_ne!(
            PRELUDE_VERSION, "logos-coc-3",
            "the version moved past the wave-3 axiom set"
        );
        assert!(
            logicaffeine_kernel::certificate::RETIRED_PRELUDE_VERSIONS.contains(&"logos-coc-3"),
            "the wave-3 tag is a retired axiom set"
        );
        let honest = Certificate::new(
            lam("P", prop(), lam("h", var("P"), var("h"))),
            pi("P", prop(), arrow(var("P"), var("P"))),
        );
        assert!(
            recheck(&honest).is_ok(),
            "a certificate minted now re-checks now"
        );
        for stale in ["logos-coc-1", "logos-coc-2", "logos-coc-3"] {
            let mut old = honest.clone();
            old.prelude_version = stale.to_string();
            assert!(
                recheck(&old).is_err(),
                "a certificate minted against `{stale}` must be refused, not quietly re-checked \
                 against this base"
            );
        }
    }
}

/// THE GUARD THE NEW BODIES MOVE, stated as the invariant instead of as a shape — and the place
/// the load moved TO, which is the half worth pinning.
///
/// The rule is: an `Int` primitive may never read a `Nat` or a `Text` literal as an integer, or
/// `sub 3 8` at `Nat` computes `-5` and the truncating `Nat` subtraction the prelude emits is
/// bypassed. `R_reduction_extra::the_int_primitives_refuse_nat_and_text_operands` states it as
/// `normalize(t) == t`, which was exact while all nine were opaque. A DEFINED `lt` δ-unfolds its
/// own body first, and `le` and `add` unfold into their matches over `Int`'s constructors, so
/// `lt 7ℕ 2ℕ` is no longer SYNTACTICALLY unchanged — it is `le`'s match waiting on `add`'s match
/// waiting on the `Nat` literal `7` — while being every bit as stuck.
///
/// What the unfolding MOVES is real and is why this test exists rather than just a note: the step
/// that must not happen is now `add 7ℕ 1ℤ`, so the line holding a `Nat` out of the `Int` ALU sits
/// on `add`'s rule as well as on `lt`'s. `term.rs::lit_bigint` answers `None` for `Literal::Nat`
/// in both kernels, so neither can take it — and the assertion below is on the ANSWER (no `Bool`,
/// no literal) rather than on the spelling, which is the form that survives a primitive being
/// given a body and would have survived it being given one in either direction.
#[test]
fn no_int_primitive_decides_or_computes_on_a_nat_or_text_literal() {
    let ctx = std_ctx();
    let textl = |s: &str| Term::Lit(Literal::Text(s.to_string()));
    let bin = |op: &str, a: Term, b: Term| apps(g(op), &[a, b]);

    for op in ["add", "sub", "mul", "div", "mod", "le", "lt", "ge", "gt"] {
        for (kind, a, b) in [
            ("two Nat", natl(7), natl(2)),
            ("mixed Int/Nat", intl(7), natl(2)),
            ("mixed Nat/Int", natl(7), intl(2)),
            ("two Text", textl("7"), textl("2")),
        ] {
            let out = normalize(&ctx, &bin(op, a, b));
            assert!(
                !matches!(out, Term::Lit(_)) && out != g("true") && out != g("false"),
                "`{op}` must not COMPUTE or DECIDE on {kind} literals — it may unfold its own \
                 body and must then be stuck, because `Nat` and `Int` are different types and \
                 reading one as the other is a silent wrong answer: {out}"
            );
        }
    }

    // THE STEP THE UNFOLDING CREATES, pinned on its own so a failure names it: `lt`'s body puts a
    // `Nat` and an `Int` literal under one `add`, and THAT must stay stuck.
    // Since `Int` became Lean's inductive, `add` is itself a checked definition — a match over
    // its first operand's `Int` constructor — so it δ-unfolds too. What must not happen is the
    // ι-step that reads the `Nat` literal `7` as an `Int` constructor: the unfolded `add` must
    // come to rest as a `match` whose discriminant is still the `Nat` literal.
    let carried = bin("add", natl(7), intl(1));
    let stuck_add = normalize(&ctx, &carried);
    assert!(
        matches!(&stuck_add, Term::Match { discriminant, .. } if **discriminant == natl(7)),
        "`add 7ℕ 1ℤ` is the mixed application `lt 7ℕ 2ℕ` now reduces THROUGH; it must stay stuck \
         on the `Nat` literal — if `add` ever accepted a `Nat` operand, the `Int` strict order \
         would start deciding `Nat` comparisons: {stuck_add}"
    );
    let stuck_lt = normalize(&ctx, &bin("lt", natl(7), natl(2)));
    assert!(
        matches!(&stuck_lt, Term::Match { discriminant, .. } if **discriminant == stuck_add),
        "…and that is exactly where `lt 7ℕ 2ℕ` stops: `le`'s match waits on the stuck `add`, \
         unfolded, decided never: {stuck_lt}"
    );

    // THE TYPE LEVEL, which is the real firewall: none of these applications is well-typed at
    // all, whether the head has a body or not.
    for op in ["add", "le", "lt", "ge", "gt"] {
        assert!(
            logicaffeine_kernel::recheck(&ctx, &bin(op, natl(7), natl(2))).is_err(),
            "the re-checker must refuse `{op} 7ℕ 2ℕ` too"
        );
        assert!(
            infer_type(&ctx, &bin(op, natl(7), natl(2))).is_err(),
            "`{op} 7ℕ 2ℕ` must be ill-typed — `{op} : Int → Int → Bool` and a `Nat` literal is \
             not an `Int`"
        );
    }
}

/// THE ONE-LINE REPAIR FOR T06-010, EXECUTED HERE SO THE ORCHESTRATOR DOES NOT HAVE TO GUESS.
///
/// `lean_conformance_t06_propositions::propositional_extensionality_is_stateable_and_tracked` ends with
/// `Repl::new()` and then `Print Axioms propextUser.`, and **never defines `propextUser`** — a
/// fresh `Repl` loads `StandardLibrary::register` and nothing else, so the call fails with
/// `Unbound variable: propextUser` and the case's last assertion has never run. Satisfying it as
/// written would mean shipping a demo constant called `propextUser` inside the trusted library,
/// which is forging; the corpus line is simply missing its `Definition`.
///
/// This is that line, executed: the declaration below is the one to insert immediately before the
/// `Print Axioms` call, and the two assertions after it are the case's own, verbatim. Everything
/// the case wants — a `propext`-using theorem, a report that names the axiom — holds today.
#[test]
fn a_propext_using_theorem_reports_the_axiom_through_the_repl_door() {
    use logicaffeine_kernel::interface::Repl;
    let mut repl = Repl::new();
    repl.execute(
        "Definition propextUser : (Iff True True) -> (Eq Prop True True) := propext True True.",
    )
    .expect("`propext True True` is a theorem about `True`, discharged from an `Iff`");

    let listed = repl
        .execute("Print Axioms propextUser.")
        .expect("the axioms a declaration transitively rests on must be reportable");
    assert!(
        listed.contains("propext"),
        "the axiom report must name `propext`: `{listed}`"
    );

    // THE NEGATIVE, so the report is a graph walk and not a file-wide scan: a theorem in the same
    // session that does NOT use extensionality must not name it.
    repl.execute("Definition cleanThm : Eq Nat Zero Zero := refl Nat Zero.")
        .expect("a clean theorem in the same session");
    let clean = repl
        .execute("Print Axioms cleanThm.")
        .expect("reportable too");
    assert!(
        !clean.contains("propext"),
        "`cleanThm` rests on nothing — the taint must be exact: `{clean}`"
    );
}

// ============================================================================================
// WAVE 4 · THE FOUNDATIONAL FRAGMENT. `dne` left the trusted base.
//
// The head-to-head number a Lean person asks for first is "how many axioms", and the honest
// answer has two halves: a total (ours is larger, and TCB.md says why for every entry) and a
// FOUNDATIONAL fragment — the assumptions that are about the LOGIC rather than about an opaque
// `Int` or a reflection embedding. Lean's foundational fragment is three: `propext`,
// `Classical.choice`, `Quot.sound`. Ours used to be four, because `dne` stood where Lean has a
// proof. It is three now, and the block below is the door shutting behind that.
//
// Lean's sources, read and ported rather than inferred:
//   Init/Classical.lean:32-63 — `theorem em` — DIACONESCU, from choice + funext + propext
//   Init/Classical.lean:75    — `propDecidable a := choice <| match em a with …`
//   Init/Classical.lean:131   — `theorem byContradiction` — `dne`, from `propDecidable`
//   Init/Classical.lean:15,25,28 — `indefiniteDescription` / `choose` / `choose_spec`
//   Init/Core.lean:1890       — `theorem funext` — the wave-3 entry this pattern is copied from
// ============================================================================================

/// THE CENSUS, in the shape Lean's own source forces. Three assumptions, and every other
/// foundational constant is a DEFINITION whose body type-checks at its declared type and which
/// both kernels certify.
///
/// This is the corrected form of `the_classical_layer_adds_exactly_leans_three_axioms_and_derives_the_rest`,
/// which asserts that `funext` is "one of Lean 4's core axioms" — it is not, it is proved at
/// `Init/Core.lean:1890`, and wave 3 made it a theorem here too, which is why that case is red.
#[test]
fn the_foundational_fragment_is_exactly_leans_three_axioms() {
    let ctx = std_ctx();
    let declared = axioms(&ctx);

    // ASSUMED — and exactly these three. Lean assumes them too.
    for name in ["propext", "Classical_choice", "Quot_sound"] {
        assert!(
            declared.contains(&name.to_string()),
            "`{name}` is one of Lean 4's three core axioms and must be a DECLARED assumption"
        );
        assert!(
            infer_type(&ctx, &g(name)).is_ok(),
            "an ill-typed axiom is worse than a missing one: `{name}` must itself be well-typed"
        );
    }

    // PROVED — every one of them, and Lean proves every one of them too. An axiom here would be
    // a capability bought with an assumption, which is the single worst move in this campaign.
    for (name, lean) in [
        ("funext", "Init/Core.lean:1890"),
        ("dne", "Init/Classical.lean:131 (Classical.byContradiction)"),
        ("Classical_em", "Init/Classical.lean:32 (Diaconescu)"),
        ("Classical_propDecidable", "Init/Classical.lean:75"),
        ("Classical_indefiniteDescription", "Init/Classical.lean:15"),
        ("Classical_choose", "Init/Classical.lean:25"),
        ("Classical_choose_spec", "Init/Classical.lean:28"),
        ("Classical_axiomOfChoice", "Init/Classical.lean:115"),
        ("nonemptyNat", "Nonempty_intro Nat Zero"),
        ("Or_rec", "auto-derived eliminator"),
        ("Ex_rec", "auto-derived eliminator"),
    ] {
        assert!(
            ctx.is_definition(name),
            "`{name}` must be a PROVEN definition — Lean derives it at {lean}, and an axiom here \
             would hide the derivation we owe"
        );
        assert!(
            !declared.contains(&name.to_string()),
            "`{name}` must not appear in `iter_declarations`"
        );
        let body = ctx
            .get_definition_body(name)
            .unwrap_or_else(|| panic!("`{name}` has a body"));
        let inferred = infer_type(&ctx, body).unwrap_or_else(|e| {
            panic!("`{name}`'s BODY must type-check, not just its type: {e:?}")
        });
        let declared_ty = ctx.get_global(name).expect("the stored type");
        assert!(
            defeq_for_test(&ctx, &inferred, declared_ty),
            "`{name}`'s body proves `{inferred}` but it is registered at `{declared_ty}` — \
             `add_definition` stores the declared type without checking it, so this is the only \
             place the two are held together"
        );
        assert_eq!(
            double_check(&ctx, body),
            DoubleCheck::Agreed,
            "both kernels must certify `{name}`'s body"
        );
    }

    // THE RATCHET, stated as a set so a later wave cannot re-assume one of them quietly. The
    // foundational fragment — every classical, extensionality and quotient-soundness constant —
    // is EXACTLY Lean's three: one classical (`Classical_choice`) and two that are not
    // classical (`propext`, `Quot_sound`). `funext` and `dne` are proved above and must never
    // reappear here.
    let foundational: std::collections::BTreeSet<&str> = declared
        .iter()
        .map(String::as_str)
        .filter(|n| {
            n.starts_with("Classical_") || matches!(*n, "propext" | "funext" | "dne" | "Quot_sound")
        })
        .collect();
    assert_eq!(
        foundational,
        ["Classical_choice", "Quot_sound", "propext"]
            .into_iter()
            .collect::<std::collections::BTreeSet<&str>>(),
        "the foundational fragment of the trusted base must be exactly Lean's three axioms: \
         {foundational:?}"
    );
    let classical: Vec<&str> = foundational
        .iter()
        .copied()
        .filter(|n| n.starts_with("Classical_"))
        .collect();
    assert_eq!(
        classical,
        vec!["Classical_choice"],
        "the ONLY classical constant the trusted base may assume is `Classical_choice`; \
         everything else in the classical layer is proved: {classical:?}"
    );
}

/// `Classical.em` is DIACONESCU'S THEOREM and its axiom closure says so. This is the assertion
/// that would catch a regression to the old shape most directly: a derivation from `dne` would
/// still typecheck, still be a definition, and still be a capability — and the closure would
/// name `dne` instead of Lean's axioms.
#[test]
fn excluded_middle_rests_on_leans_axioms_and_not_on_a_classical_assumption_of_our_own() {
    use logicaffeine_kernel::prelude::axiom_dependencies_of;
    let ctx = std_ctx();
    let usage = axiom_dependencies_of(&ctx, "Classical_em").expect("`Classical_em` is registered");
    assert!(
        usage.unresolved.is_empty(),
        "`Classical_em` cites names the trusted base does not have: {:?}",
        usage.unresolved
    );
    for needed in ["propext", "Classical_choice", "Quot_sound"] {
        assert!(
            usage.axioms.contains(needed),
            "Diaconescu's proof uses choice, function extensionality and propositional \
             extensionality — `{needed}` must be in the closure: {:?}",
            usage.axioms
        );
    }
    // THE NEGATIVE. Nothing classical of our own may be in there, under any spelling.
    for ours in ["dne", "Classical_em", "sorryAx", "em"] {
        assert!(
            !usage.axioms.contains(ours),
            "`{ours}` must not be an axiom `Classical_em` rests on — the whole point of porting \
             Lean's derivation is that the classical content is `Classical_choice` and nothing \
             else: {:?}",
            usage.axioms
        );
    }
    // Lean reports exactly `propext, Classical.choice, Quot.sound` for `Classical.em`, and so do
    // we: the quotient formers are primitive identities here as `ConstantInfo.quotInfo` is there.
    // Pinned so the closure cannot grow by a single name.
    let extra: std::collections::BTreeSet<&str> = usage
        .axioms
        .iter()
        .map(|s| s.as_str())
        .filter(|n| !matches!(*n, "propext" | "Classical_choice" | "Quot_sound"))
        .collect();
    assert!(
        extra.is_empty(),
        "our closure may add nothing to Lean's three; it adds {extra:?}: {:?}",
        usage.axioms
    );
}

/// `dne` is a theorem now, and it still does everything it used to. Type verbatim unchanged (a
/// weakened statement would be a silent capability loss), the classical firewall still shut, and
/// both kernels certify an application.
#[test]
fn double_negation_elimination_is_proved_and_its_statement_is_unchanged() {
    let ctx = std_ctx();
    assert!(
        ctx.is_definition("dne"),
        "`dne` is Lean's `Classical.byContradiction` (Init/Classical.lean:131) and must be PROVED"
    );
    // THE BOUNDARY: the exact type it had as an axiom, spelled with the arrow chain.
    let want = pi(
        "P",
        prop(),
        arrow(arrow(arrow(var("P"), g("False")), g("False")), var("P")),
    );
    let have = ctx.get_global("dne").expect("registered").clone();
    assert!(
        defeq_for_test(&ctx, &have, &want),
        "`dne : Π(P:Prop). ¬¬P → P`, verbatim — a proof that only closes a weaker statement is a \
         capability loss wearing a trusted-base win's clothes; got `{have}`"
    );

    // It still proves things. `¬¬p` written with the `Not` global must reach the arrow chain.
    let mut c = ctx.clone();
    c.add_declaration("pp", prop());
    c.add_declaration("hnn", arrow(app(g("Not"), g("pp")), g("False")));
    let used = apps(g("dne"), &[g("pp"), g("hnn")]);
    let ty = infer_type(&c, &used).expect("`dne` applies to a `¬¬p` written with `Not`");
    assert!(
        defeq_for_test(&c, &ty, &g("pp")),
        "`dne pp hnn : pp`, got {ty}"
    );
    assert_eq!(
        double_check(&c, &used),
        DoubleCheck::Agreed,
        "both kernels must certify a `dne` application"
    );

    // THE FIREWALL, unchanged: `p ∨ ¬p` still cannot be eliminated into `Decidable p`. Making
    // `em` a theorem must not have loosened the eliminator — the motive's SORT is the gate.
    let or_pp = apps(g("Or"), &[var("p"), app(g("Not"), var("p"))]);
    let smuggle = lam(
        "p",
        prop(),
        apps(
            g("Or_rec"),
            &[
                var("p"),
                app(g("Not"), var("p")),
                lam("_", or_pp, app(g("Decidable"), var("p"))),
                lam("hp", var("p"), apps(g("isTrue"), &[var("p"), var("hp")])),
                lam(
                    "hn",
                    app(g("Not"), var("p")),
                    apps(g("isFalse"), &[var("p"), var("hn")]),
                ),
            ],
        ),
    );
    assert!(
        infer_type(&ctx, &smuggle).is_err(),
        "eliminating `p ∨ ¬p` into `Decidable p` must STILL be refused — proving `em` rather than \
         deriving it from `dne` must not have widened the motive"
    );
}

/// `Classical.choose` is a FUNCTION, not a name: applied to a concrete existence proof its
/// specification is available and the whole thing kernel-checks in both kernels. An
/// `indefiniteDescription` that only type-checks is a declaration with extra steps.
#[test]
fn choosing_from_an_existence_proof_carries_its_specification() {
    let ctx = std_ctx();
    // p := λ(n:Nat). Eq Nat n Zero, and the witness is `Zero` itself.
    let pred = lam("n", nat(), eq_at(nat(), var("n"), g("Zero")));
    let hex = apps(
        g("witness"),
        &[
            nat(),
            pred.clone(),
            g("Zero"),
            apps(g("refl"), &[nat(), g("Zero")]),
        ],
    );
    let chosen = apps(g("Classical_choose"), &[nat(), pred.clone(), hex.clone()]);
    let ty = infer_type(&ctx, &chosen).expect("`choose` applies to an existence proof");
    assert!(
        defeq_for_test(&ctx, &ty, &nat()),
        "`choose … : Nat`, got {ty}"
    );
    assert_eq!(double_check(&ctx, &chosen), DoubleCheck::Agreed);

    let spec = apps(g("Classical_choose_spec"), &[nat(), pred.clone(), hex]);
    let spec_ty = infer_type(&ctx, &spec).expect("`choose_spec` applies");
    assert!(
        defeq_for_test(&ctx, &spec_ty, &app(pred, chosen)),
        "`choose_spec … : p (choose … )` — the specification must be ABOUT the chosen value and \
         not about a fresh one; got {spec_ty}"
    );
    assert_eq!(double_check(&ctx, &spec), DoubleCheck::Agreed);
}

// ============================================================================================
// WAVE 4 · `eq_of_heq`, and the reason it is not `JMeq_eq`.
// ============================================================================================

/// The bridge from `HEq` back to `Eq` is a THEOREM and it costs the trusted base nothing. The
/// kernel reduces `cast` along an opaque `Eq` proof by K-like ι-reduction exactly where Lean's
/// kernel does (convertible indices) and nowhere else, and `cast_refl` still states the same fact
/// as a proposition. All halves are asserted, because the first alone is satisfiable by an axiom
/// and a K rule that ignored the indices would be a loosened reducer.
#[test]
fn the_heterogeneous_bridge_is_proved_and_costs_the_trusted_base_nothing() {
    use logicaffeine_kernel::prelude::axiom_dependencies_of;
    let ctx = std_ctx();
    assert!(
        ctx.is_definition("eq_of_heq"),
        "`eq_of_heq` is Lean's `Init/Prelude.lean:464` and must be PROVED — assuming it is Coq's \
         `JMeq_eq`, an axiom"
    );
    let bridge = apps(
        g("eq_of_heq"),
        &[
            nat(),
            natl(3),
            natl(3),
            apps(g("HEq_refl"), &[nat(), natl(3)]),
        ],
    );
    let ty = infer_type(&ctx, &bridge).expect("the bridge applies at a common carrier");
    assert!(
        defeq_for_test(&ctx, &ty, &eq_at(nat(), natl(3), natl(3))),
        "`eq_of_heq Nat 3 3 (HEq_refl Nat 3) : Eq Nat 3 3`, got {ty}"
    );
    assert_eq!(
        double_check(&ctx, &bridge),
        DoubleCheck::Agreed,
        "both kernels must certify the bridge"
    );

    // IT COSTS NOTHING. The closure is empty — no `Quot_sound`, no `propext`, no invented name.
    let usage = axiom_dependencies_of(&ctx, "eq_of_heq").expect("registered");
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "`eq_of_heq` must rest on NO axiom at all: {:?} / {:?}",
        usage.axioms,
        usage.unresolved
    );

    // K-LIKE ι-REDUCTION, exactly as Lean's kernel performs it. `Eq` is a K-like inductive (a
    // `Prop` with one constructor and no fields), so with an OPAQUE `hc : Eq (Type 0) Nat Nat`
    // the match inside `cast Nat Nat hc x` fires by replacing `hc` with `refl` — which is sound
    // only because the proof's two indices are CONVERTIBLE. Lean's `toCtorWhenK` checks exactly
    // that, and so does ours.
    let mut c = ctx.clone();
    c.add_declaration("hcOpaque", eq_at(type0(), nat(), nat()));
    let stuck = apps(g("cast"), &[nat(), nat(), g("hcOpaque"), g("Zero")]);
    assert_eq!(
        normalize(&c, &stuck),
        g("Zero"),
        "`cast Nat Nat hc Zero` reduces by K exactly as in Lean: the indices `Nat`/`Nat` agree"
    );
    assert_eq!(double_check(&c, &stuck), DoubleCheck::Agreed);
    // THE NEGATIVE that separates K from a loosened reducer: when the indices DIFFER the proof
    // cannot be replaced by `refl`, so a cast along an assumed `Nat = Bool` must stay stuck and
    // must never hand back the `Nat` value at type `Bool`.
    c.add_declaration("hcApart", eq_at(type0(), nat(), g("Bool")));
    let apart = apps(g("cast"), &[nat(), g("Bool"), g("hcApart"), g("Zero")]);
    let apart_nf = normalize(&c, &apart);
    assert!(
        apart_nf != g("Zero") && apart_nf != g("true") && apart_nf != g("false"),
        "K must NOT fire on `Eq (Type 0) Nat Bool`: its indices are not convertible, and firing \
         would coerce `Zero` into `Bool` — a change to what the kernel BELIEVES: {apart_nf}"
    );
    // …and the propositional route closes it, which is the whole argument.
    let closed = apps(g("cast_refl"), &[nat(), g("hcOpaque"), g("Zero")]);
    let closed_ty = infer_type(&c, &closed).expect("`cast_refl` applies at an opaque proof");
    assert!(
        defeq_for_test(&c, &closed_ty, &eq_at(nat(), stuck, g("Zero"))),
        "`cast_refl Nat hc Zero : Eq Nat (cast Nat Nat hc Zero) Zero`, got {closed_ty}"
    );
}

// ============================================================================================
// WAVE 4 · The `Quotient` API Mathlib actually calls.
// ============================================================================================

/// `Quotient.sound`/`.lift`/`.ind`/`.rec` exist, are definitions, and the one COMPUTATION rule a
/// quotient has still fires through the wrapper: `Quotient_lift f h (Quotient_mk s x) ⤳ f x`.
/// A wrapper that type-checks and does not compute is `Quot` with extra names.
#[test]
fn the_quotient_wrapper_computes_through_to_the_primitive() {
    let mut ctx = std_ctx();
    for name in [
        "Quotient",
        "Quotient_mk",
        "Quotient_sound",
        "Quotient_lift",
        "Quotient_ind",
        "Quotient_rec",
        "Setoid",
        "Setoid_r",
        "Setoid_refl",
        "Setoid_symm",
        "Setoid_trans",
    ] {
        assert!(
            ctx.get_global(name).is_some(),
            "`{name}` must exist — Mathlib phrases its ENTIRE quotient API in `Quotient`, never \
             in `Quot` (Init/Core.lean:1657-1725)"
        );
        assert!(
            !axioms(&ctx).contains(&name.to_string()),
            "`{name}` must cost the trusted base nothing — Lean's are one-liners over `Quot`"
        );
    }

    ctx.add_declaration("sNat", app(g("Setoid"), nat()));
    let rel = apps(g("Setoid_r"), &[nat(), g("sNat")]);
    let f = lam("n", nat(), succ(var("n")));
    ctx.add_declaration(
        "respectSucc",
        pi(
            "x",
            nat(),
            pi(
                "y",
                nat(),
                arrow(
                    apps(rel.clone(), &[var("x"), var("y")]),
                    eq_at(nat(), app(f.clone(), var("x")), app(f.clone(), var("y"))),
                ),
            ),
        ),
    );
    let lifted = apps(
        g("Quotient_lift"),
        &[
            nat(),
            g("sNat"),
            nat(),
            f,
            g("respectSucc"),
            apps(g("Quotient_mk"), &[nat(), g("sNat"), peano(2)]),
        ],
    );
    assert_eq!(
        normalize(&ctx, &lifted),
        peano(3),
        "`Quotient_lift f h (Quotient_mk s 2)` must REDUCE to `f 2` — the wrapper is δ-transparent \
         over `Quot_lift`, so the quotient ι-rule fires through it"
    );
    assert_eq!(double_check(&ctx, &lifted), DoubleCheck::Agreed);

    // `Quotient_sound` at a concrete relation, kernel-verified in both kernels.
    let sound = apps(
        g("Quotient_sound"),
        &[nat(), g("sNat"), g("Zero"), succ(g("Zero")), g("hrel")],
    );
    let mut c2 = ctx.clone();
    c2.add_declaration("hrel", apps(rel, &[g("Zero"), succ(g("Zero"))]));
    let ty = infer_type(&c2, &sound).expect("`Quotient_sound` applies");
    assert!(
        defeq_for_test(
            &c2,
            &ty,
            &eq_at(
                apps(g("Quotient"), &[nat(), g("sNat")]),
                apps(g("Quotient_mk"), &[nat(), g("sNat"), g("Zero")]),
                apps(g("Quotient_mk"), &[nat(), g("sNat"), succ(g("Zero"))]),
            )
        ),
        "`Quotient_sound … : Quotient_mk s 0 = Quotient_mk s 1`, got {ty}"
    );
    assert_eq!(double_check(&c2, &sound), DoubleCheck::Agreed);
}
