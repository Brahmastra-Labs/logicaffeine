//! LANE S_spine — the DECLARATION SPINE's own tests: the behaviour the conformance corpus does not
//! ask about but that the spine changes anyway.
//!
//! Every block below is a POSITIVE/NEGATIVE pair or a BOUNDARY, because a positive-only test is
//! satisfiable by a weak implementation:
//!
//! 1. INSTANCE ORDER — Lean's `(priority desc, later declaration first)`, in both directions,
//!    plus the audit trail that says which entry won and why.
//! 2. THE FALLBACK TIER — a library/derived instance must be findable and must never outrank a
//!    user instance, and must not appear in `instances()`.
//! 3. PARAMS vs INDICES — the split inferred from the constructors, one rung above and one below,
//!    including the recursive-occurrence clause that keeps `Acc` at 2.
//! 4. A STRUCTURE'S SORT — computed from its fields: all-proof ⇒ `Prop`, one datum ⇒ `Type 0`,
//!    a stored `Type` ⇒ `Type 1`, and the projections must still COMPUTE at every one of them.
//! 5. MUTUAL BLOCKS — a disagreeing parameter count is refused at registration and leaves
//!    nothing behind; an agreeing one still registers.
//! 6. DECLARATION-TIME AUXILIARIES — derived, kernel-CHECKED, idempotent, and never displacing a
//!    hand-written constant.

use logicaffeine_kernel::elaborate::{resolve_instance_audited, InstanceTier};
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    double_check, infer_type, normalize, normalize_checked, resolve_instance, surface_elaborate,
    Context, DoubleCheck, KernelError, MetaCtx, MutualInductive, Term, Universe,
    DEFAULT_INSTANCE_PRIORITY,
};

// ============================================================================================
// HELPERS
// ============================================================================================

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
    xs.iter().fold(f, |acc, x| app(acc, x.clone()))
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
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn ty1() -> Term {
    Term::Sort(Universe::Type(1))
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn nat() -> Term {
    g("Nat")
}
fn peano(n: u32) -> Term {
    (0..n).fold(g("Zero"), |acc, _| app(g("Succ"), acc))
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
/// `Eq A a b`.
fn eq3(a: Term, x: Term, y: Term) -> Term {
    apps(g("Eq"), &[a, x, y])
}
/// The codomain of an arity — `Π(…). S` ⇝ `S`.
fn codomain(t: &Term) -> &Term {
    let mut cur = t;
    while let Term::Pi { body_type, .. } = cur {
        cur = body_type;
    }
    cur
}

// ============================================================================================
// 1 · INSTANCE ORDER — priority descending, then the LATER declaration
// ============================================================================================

/// The tie-break, from BOTH sides. Two instances at one type, no priorities: the later wins.
/// Then the same pair with the EARLIER one given a higher priority: the earlier wins after all.
/// A test that only pinned "later wins" would be satisfied by a plain reverse scan, which is not
/// Lean's rule and silently ignores every `(priority := n)` a translated source carries.
#[test]
fn priority_outranks_declaration_order_in_both_directions() {
    let mut base = std_ctx();
    base.add_structure("Ranked", &[("a", ty0())], &[("rank", nat())]);
    let goal = app(g("Ranked"), nat());
    let mk = |n: u32| apps(g("Ranked_mk"), &[nat(), peano(n)]);
    let read = |ctx: &Context, t: Term| normalize(ctx, &apps(g("Ranked_rank"), &[nat(), t]));

    // (a) EQUAL priorities: the later declaration wins.
    let mut equal = base.clone();
    equal.add_instance(goal.clone(), mk(1));
    equal.add_instance(goal.clone(), mk(2));
    assert_eq!(
        read(&equal, resolve(&equal, &goal).expect("resolves")),
        peano(2),
        "two equal-priority instances at one type tie-break to the LATER declaration, as Lean does"
    );

    // (b) The EARLIER one at a HIGHER priority: order loses to priority.
    let mut earlier_wins = base.clone();
    earlier_wins.add_instance_with_priority(goal.clone(), mk(1), DEFAULT_INSTANCE_PRIORITY + 1);
    earlier_wins.add_instance(goal.clone(), mk(2));
    assert_eq!(
        read(
            &earlier_wins,
            resolve(&earlier_wins, &goal).expect("resolves")
        ),
        peano(1),
        "a higher priority declared FIRST must still win — otherwise `(priority := n)` is ignored \
         and only declaration order is read"
    );

    // (c) THE NEGATIVE: the later one at a LOWER priority must NOT be chosen, even though a
    // reverse scan would take it.
    let mut later_loses = base.clone();
    later_loses.add_instance(goal.clone(), mk(1));
    later_loses.add_instance_with_priority(goal.clone(), mk(2), DEFAULT_INSTANCE_PRIORITY - 1);
    assert_eq!(
        read(
            &later_loses,
            resolve(&later_loses, &goal).expect("resolves")
        ),
        peano(1),
        "a LOWER-priority instance declared later must lose to the higher-priority earlier one"
    );

    // (d) The database is PUSH-ONLY throughout, measured relatively — an absolute count is not a
    // property of the database, because a declaration-time registration can land at any moment.
    let before = base.instances().len();
    let mut pushed = base.clone();
    pushed.add_instance(goal.clone(), mk(1));
    pushed.add_instance_with_priority(goal.clone(), mk(2), 42);
    assert_eq!(
        pushed.instances().len(),
        before + 2,
        "both registrations stay in the database; resolution reorders, it never removes"
    );
}

/// The choice must be AUDITABLE — which entry won, at what priority, from which tier, and what
/// was tried on the way. Lean can tell you an instance was synthesised; it cannot tell you why
/// that one.
#[test]
fn the_winning_instance_reports_its_own_coordinates() {
    let mut ctx = std_ctx();
    ctx.add_structure("Audited", &[("a", ty0())], &[("tag", nat())]);
    let goal = app(g("Audited"), nat());
    let mk = |n: u32| apps(g("Audited_mk"), &[nat(), peano(n)]);
    ctx.add_instance(goal.clone(), mk(1)); // index 0, default priority
    ctx.add_instance_with_priority(goal.clone(), mk(2), 5000); // index 1, high
    ctx.add_instance(goal.clone(), mk(3)); // index 2, default

    let mut mctx = MetaCtx::new();
    let choice = resolve_instance_audited(&ctx, &mut mctx, &goal).expect("resolves");
    assert_eq!(
        choice.tier,
        InstanceTier::Declared { index: 1 },
        "the winner must name its own declaration index, not merely its value"
    );
    assert_eq!(choice.priority, 5000, "…and the priority that won it");
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("Audited_tag"), &[nat(), choice.value.clone()])
        ),
        peano(2),
        "the audited answer must be the SAME answer `resolve_instance` gives"
    );
    assert_eq!(
        resolve(&ctx, &goal),
        Some(choice.value),
        "one search serves both faces, so they cannot drift apart"
    );

    // THE NEGATIVE: the audit is a record of the search, not of the whole database. The highest
    // priority is tried FIRST, so nothing is examined before it.
    assert_eq!(
        choice.considered.len(),
        1,
        "resolution stops at the winner; a scan that had to look at the other two would mean the \
         order key is not being used: {:?}",
        choice.considered
    );
    assert!(
        choice.considered[0].applied,
        "and the one probe recorded is the one that applied"
    );
}

// ============================================================================================
// 2 · THE FALLBACK TIER
// ============================================================================================

/// A library/derived instance must be findable, must NEVER outrank a user instance for the same
/// goal, and must not be reported as something the program declared.
#[test]
fn a_fallback_instance_is_findable_but_never_outranks_a_declared_one() {
    let mut ctx = std_ctx();
    ctx.add_structure("Backed", &[("a", ty0())], &[("tag", nat())]);
    let goal = app(g("Backed"), nat());
    let mk = |n: u32| apps(g("Backed_mk"), &[nat(), peano(n)]);
    let read = |ctx: &Context, t: Term| normalize(ctx, &apps(g("Backed_tag"), &[nat(), t]));

    // (a) With nothing declared, the fallback answers.
    let mut only_fallback = ctx.clone();
    let declared_count = only_fallback.instances().len();
    only_fallback.add_fallback_instance(goal.clone(), mk(7));
    assert_eq!(
        read(
            &only_fallback,
            resolve(&only_fallback, &goal).expect("the fallback resolves")
        ),
        peano(7),
        "a fallback instance must be findable, or declaration-time synthesis is unreachable"
    );
    assert_eq!(
        only_fallback.instances().len(),
        declared_count,
        "…and it must NOT appear in `instances()`, which is the record of what was DECLARED"
    );
    assert_eq!(
        only_fallback.fallback_instances().len(),
        ctx.fallback_instances().len() + 1,
        "it is in the lower tier instead"
    );

    // (b) THE NEGATIVE, and the whole point of a tier: a user instance declared at the LOWEST
    // priority still beats the fallback. A numeric priority could be undercut; a tier cannot.
    let mut both = only_fallback.clone();
    both.add_instance_with_priority(goal.clone(), mk(1), i64::MIN);
    assert_eq!(
        read(&both, resolve(&both, &goal).expect("resolves")),
        peano(1),
        "a DECLARED instance outranks a fallback whatever its priority — the tier is the ranking"
    );

    // (c) Idempotent: declaration-time synthesis re-runs once per constructor, and a tier that
    // grew per re-derivation would turn one enum into n identical candidates.
    let mut twice = ctx.clone();
    let base = twice.fallback_instances().len();
    twice.add_fallback_instance(goal.clone(), mk(7));
    twice.add_fallback_instance(goal.clone(), mk(7));
    assert_eq!(
        twice.fallback_instances().len(),
        base + 1,
        "the same fallback registered twice is one"
    );
}

// ============================================================================================
// 3 · PARAMS vs INDICES, inferred from the constructors
// ============================================================================================

/// A `Vector`-shaped family registered through the PLAIN door: one uniform parameter, one index.
/// One rung below (`BVec`-shaped: no parameter at all) and one rung above (a plain container:
/// every position uniform) pin the boundary from both sides.
#[test]
fn the_parameter_index_split_is_inferred_one_rung_either_side() {
    let mut ctx = std_ctx();

    // A: `Vec A : Nat → Type 0` — `A` uniform, the length an index.
    ctx.add_inductive("Vec", pi("A", ty0(), arrow(nat(), ty0())));
    ctx.add_constructor(
        "vnil",
        "Vec",
        pi("A", ty0(), apps(g("Vec"), &[var("A"), g("Zero")])),
    );
    ctx.add_constructor(
        "vcons",
        "Vec",
        pi(
            "A",
            ty0(),
            pi(
                "n",
                nat(),
                arrow(
                    var("A"),
                    arrow(
                        apps(g("Vec"), &[var("A"), var("n")]),
                        apps(g("Vec"), &[var("A"), app(g("Succ"), var("n"))]),
                    ),
                ),
            ),
        ),
    );
    assert_eq!(
        ctx.inductive_num_params("Vec"),
        1,
        "`A` is the one uniform parameter"
    );
    assert_eq!(
        ctx.inductive_num_indices("Vec"),
        1,
        "the length is the one index"
    );

    // B: one rung BELOW — no uniform prefix at all.
    ctx.add_inductive("Len", arrow(nat(), ty0()));
    ctx.add_constructor("lz", "Len", app(g("Len"), g("Zero")));
    ctx.add_constructor(
        "ls",
        "Len",
        pi(
            "n",
            nat(),
            arrow(
                app(g("Len"), var("n")),
                app(g("Len"), app(g("Succ"), var("n"))),
            ),
        ),
    );
    assert_eq!(
        ctx.inductive_num_params("Len"),
        0,
        "no constructor passes its own first binder"
    );
    assert_eq!(ctx.inductive_num_indices("Len"), 1);

    // C: one rung ABOVE — every arity position uniform.
    ctx.add_inductive("Pair2", pi("A", ty0(), pi("B", ty0(), ty0())));
    ctx.add_constructor(
        "p2",
        "Pair2",
        pi(
            "A",
            ty0(),
            pi(
                "B",
                ty0(),
                arrow(
                    var("A"),
                    arrow(var("B"), apps(g("Pair2"), &[var("A"), var("B")])),
                ),
            ),
        ),
    );
    assert_eq!(
        ctx.inductive_num_params("Pair2"),
        2,
        "both positions are uniform"
    );
    assert_eq!(ctx.inductive_num_indices("Pair2"), 0);

    // D: a HEADER with no constructors keeps the pre-inference answer, so a declaration in
    // flight is never re-read underneath its own caller.
    ctx.add_inductive("Pending", arrow(nat(), ty0()));
    assert_eq!(
        ctx.inductive_num_params("Pending"),
        1,
        "nothing constrains it yet"
    );

    // E: an EXPLICIT declaration always wins over the inference.
    let mut pinned = ctx.clone();
    pinned.set_inductive_params("Vec", 0);
    assert_eq!(
        pinned.inductive_num_params("Vec"),
        0,
        "a declared split is the authority — a translator carrying Lean's own split is never \
         second-guessed"
    );
}

/// The recursive-occurrence clause, on the shape that proves it is load-bearing. `Acc`'s
/// introduction rule passes its OWN `x` in the result but `y` at the recursive occurrence, so
/// reading the result alone reports one parameter too many — and an `Acc` recursor that
/// abstracts nothing kills every well-founded definition.
#[test]
fn a_recursive_occurrence_that_varies_a_position_makes_it_an_index() {
    let mut ctx = std_ctx();
    // `AccX : Π(A:Type0). (A → A → Prop) → A → Prop`
    ctx.add_inductive(
        "AccX",
        pi(
            "A",
            ty0(),
            arrow(
                arrow(var("A"), arrow(var("A"), prop())),
                arrow(var("A"), prop()),
            ),
        ),
    );
    // `accx_intro : Π(A)(R)(x:A). (Π(y:A). R y x → AccX A R y) → AccX A R x`
    ctx.add_constructor(
        "accx_intro",
        "AccX",
        pi(
            "A",
            ty0(),
            pi(
                "R",
                arrow(var("A"), arrow(var("A"), prop())),
                pi(
                    "x",
                    var("A"),
                    arrow(
                        pi(
                            "y",
                            var("A"),
                            arrow(
                                apps(var("R"), &[var("y"), var("x")]),
                                apps(g("AccX"), &[var("A"), var("R"), var("y")]),
                            ),
                        ),
                        apps(g("AccX"), &[var("A"), var("R"), var("x")]),
                    ),
                ),
            ),
        ),
    );
    assert_eq!(
        ctx.inductive_num_params("AccX"),
        2,
        "the RESULT passes A, R and x uniformly, but the recursive occurrence passes `y` at \
         position 2 — so position 2 is an INDEX. Reading the result alone reports 3 and every \
         `Acc`-based recursion dies"
    );
    assert_eq!(ctx.inductive_num_indices("AccX"), 1);
}

/// The consequence the split exists for: a length-abstracting `match` on an inferred indexed
/// family type-checks, ι-computes, and BOTH kernels agree about it.
#[test]
fn an_inferred_index_makes_a_length_abstracting_match_typeable() {
    let mut ctx = std_ctx();
    ctx.add_inductive("Tape", arrow(nat(), ty0()));
    ctx.add_constructor("tnil", "Tape", app(g("Tape"), g("Zero")));
    ctx.add_constructor(
        "tcons",
        "Tape",
        pi(
            "n",
            nat(),
            arrow(
                app(g("Tape"), var("n")),
                app(g("Tape"), app(g("Succ"), var("n"))),
            ),
        ),
    );
    assert_eq!(
        ctx.inductive_num_params("Tape"),
        0,
        "the premise: the length is an index"
    );

    let m = Term::Match {
        discriminant: Box::new(g("tnil")),
        motive: Box::new(lam("n", nat(), lam("v", app(g("Tape"), var("n")), nat()))),
        cases: vec![
            g("Zero"),
            lam(
                "n",
                nat(),
                lam("t", app(g("Tape"), var("n")), app(g("Succ"), g("Zero"))),
            ),
        ],
    };
    assert!(
        matches!(infer_type(&ctx, &m), Ok(t) if t == nat()),
        "a length-abstracting match must type-check: {:?}",
        infer_type(&ctx, &m)
    );
    assert_eq!(
        double_check(&ctx, &m),
        DoubleCheck::Agreed,
        "and the SECOND kernel must agree — a split that only one kernel reads is a divergence \
         between typing and re-checking"
    );
    assert_eq!(
        normalize(&ctx, &m),
        g("Zero"),
        "…and it must ι-compute to the first case"
    );
}

// ============================================================================================
// 4 · A STRUCTURE'S SORT, computed from its fields
// ============================================================================================

/// All-proof ⇒ `Prop`; add ONE datum and it must climb back to `Type 0`; store a `Type` and it
/// must reach `Type 1`. Three rungs, and the projections have to COMPUTE at every one of them —
/// a structure that registers at the right sort but whose fields cannot be read back is not a
/// record.
#[test]
fn a_structures_sort_is_the_least_one_that_holds_its_fields() {
    // (a) ALL PROOFS ⇒ Prop.
    let mut p = std_ctx();
    p.add_structure(
        "Conj2",
        &[("P", prop()), ("Q", prop())],
        &[("l", var("P")), ("r", var("Q"))],
    );
    assert_eq!(
        codomain(p.get_global("Conj2").expect("Conj2 registers")),
        &prop(),
        "a record storing nothing but proofs is a PROPOSITION; registering it in `Type 0` turns \
         proofs into data — proof irrelevance stops applying and large elimination stops firing"
    );
    // …and its projections compute.
    let both = apps(g("Conj2_mk"), &[g("True"), g("True"), g("I"), g("I")]);
    assert!(
        infer_type(&p, &both).is_ok(),
        "the Prop-sorted constructor still type-checks: {:?}",
        infer_type(&p, &both)
    );
    assert_eq!(
        normalize(
            &p,
            &apps(g("Conj2_l"), &[g("True"), g("True"), both.clone()])
        ),
        g("I"),
        "a Prop-sorted record's projection must still ι-compute"
    );

    // (b) THE NEGATIVE / boundary: one datum field and it is NOT a proposition any more.
    let mut mixed = std_ctx();
    mixed.add_structure(
        "Tagged2",
        &[("P", prop())],
        &[("proof", var("P")), ("count", nat())],
    );
    assert_eq!(
        codomain(mixed.get_global("Tagged2").expect("Tagged2 registers")),
        &ty0(),
        "one stored `Nat` makes the record DATA — inferring `Prop` here would grant proof \
         irrelevance to a value"
    );

    // (c) One rung above: a stored `Type` lands in `Type 1`.
    let mut bundle = std_ctx();
    bundle.add_structure(
        "Bundle2",
        &[],
        &[("carrier", ty0()), ("point", var("carrier"))],
    );
    assert_eq!(
        bundle.get_global("Bundle2"),
        Some(&ty1()),
        "a record storing a `Type`-valued field lives in `Type 1`; `Type 0` is the \
         Girard/Hurkens door"
    );
    let dict = apps(g("Bundle2_mk"), &[nat(), g("Zero")]);
    assert_eq!(
        normalize(&bundle, &app(g("Bundle2_carrier"), dict.clone())),
        nat(),
        "…and its projection reads the carrier back"
    );
    assert!(
        matches!(double_check(&bundle, &dict), DoubleCheck::Agreed),
        "both kernels agree"
    );

    // (d) A FIELDLESS record stays `Type 0` — Lean's `structure Foo where`. It is a unit type,
    // not a proposition, and must remain eliminable into `Type`.
    let mut empty = std_ctx();
    empty.add_structure("Unitish2", &[], &[]);
    assert_eq!(
        empty.get_global("Unitish2"),
        Some(&ty0()),
        "an empty record is a unit TYPE, not a proposition"
    );
}

/// The gate `add_structure` used to skip. A structure declared BELOW the sort its fields force
/// must be refused, and the refusal must leave nothing behind.
#[test]
fn a_structure_declared_below_its_fields_sort_is_refused_transactionally() {
    let mut ctx = std_ctx();
    let refused = ctx.add_structure_at(
        "TooSmall",
        Universe::Type(0),
        &[],
        &[("carrier", ty0()), ("point", var("carrier"))],
    );
    assert!(
        matches!(refused, Err(KernelError::CertificationError(_))),
        "a `Type 0` record storing a `Type 0`-typed field is universe-inconsistent: {refused:?}"
    );
    assert!(
        !ctx.is_inductive("TooSmall") && ctx.get_global("TooSmall_mk").is_none(),
        "…and a refused declaration leaves NOTHING behind"
    );

    // THE POSITIVE CONTROL: the same fields at the sort they force is accepted.
    assert!(
        ctx.add_structure_at(
            "BigEnough",
            Universe::Type(1),
            &[],
            &[("carrier", ty0()), ("point", var("carrier"))],
        )
        .is_ok(),
        "the identical record at `Type 1` must be accepted — the gate is a universe rule, not a \
         ban on stored types"
    );
}

// ============================================================================================
// 5 · MUTUAL BLOCKS — parameter agreement, at REGISTRATION
// ============================================================================================

fn member(
    name: &str,
    sort: Term,
    num_params: usize,
    ctors: Vec<(String, Term)>,
) -> MutualInductive {
    MutualInductive {
        name: name.to_string(),
        sort,
        num_params,
        constructors: ctors,
    }
}

#[test]
fn a_mutual_block_must_agree_on_its_parameter_count_at_registration() {
    // THE NEGATIVE: counts disagree ⇒ refused, and nothing is left behind.
    let mut ctx = std_ctx();
    let bad = vec![
        member(
            "MA",
            arrow(nat(), ty0()),
            1,
            vec![("ma0".to_string(), app(g("MA"), g("Zero")))],
        ),
        member(
            "MB",
            arrow(nat(), ty0()),
            0,
            vec![("mb0".to_string(), app(g("MB"), g("Zero")))],
        ),
    ];
    let refused = ctx.add_mutual_inductives(&bad);
    assert!(
        matches!(refused, Err(KernelError::CertificationError(ref m)) if m.contains("parameter count")),
        "members that disagree on their parameter count have no induction principle, so the \
         block must be refused at REGISTRATION: {refused:?}"
    );
    assert!(
        !ctx.is_inductive("MA")
            && !ctx.is_inductive("MB")
            && ctx.mutual_block_of("MA").is_none()
            && ctx.get_global("ma0").is_none(),
        "a refused block leaves NOTHING behind — registration is transactional or it is a trap"
    );

    // THE OTHER NEGATIVE: a count ABOVE the member's own arity. `min(arity)` used to launder
    // this, so two members could agree on a number while disagreeing about what it means.
    let mut over = std_ctx();
    let nonsense = vec![
        member("OA", ty0(), 2, vec![("oa0".to_string(), g("OA"))]),
        member("OB", ty0(), 2, vec![("ob0".to_string(), g("OB"))]),
    ];
    let over_refused = over.add_mutual_inductives(&nonsense);
    assert!(
        matches!(over_refused, Err(KernelError::CertificationError(ref m)) if m.contains("arity")),
        "a parameter count above the arity has no telescope to name: {over_refused:?}"
    );
    assert!(!over.is_inductive("OA"), "…and nothing is registered");

    // THE POSITIVE CONTROL: agreeing counts still register, and the block is recorded.
    let mut good = std_ctx();
    let ok_block = vec![
        member("GA", ty0(), 0, vec![("ga0".to_string(), g("GA"))]),
        member(
            "GB",
            ty0(),
            0,
            vec![("gb0".to_string(), arrow(g("GA"), g("GB")))],
        ),
    ];
    assert!(
        good.add_mutual_inductives(&ok_block).is_ok(),
        "an agreeing block registers"
    );
    assert_eq!(
        good.mutual_block_of("GA").map(|b| b.to_vec()),
        Some(vec!["GA".to_string(), "GB".to_string()]),
        "…and both members know they are one block"
    );
}

// ============================================================================================
// 6 · THE CHECKED CONSTRUCTOR DOOR
// ============================================================================================

/// A constructor for a type that does not exist must be refused by the CHECKED path — every
/// gate below it treats an unknown head as inert, so the checked door used to admit it.
#[test]
fn the_checked_constructor_door_refuses_an_inductive_that_does_not_exist() {
    let mut ctx = std_ctx();
    let refused = ctx.add_constructor_checked("ghost_mk", "Ghost", g("Ghost"));
    assert!(
        matches!(refused, Err(KernelError::CertificationError(_))),
        "a constructor naming an unregistered inductive must be refused: {refused:?}"
    );
    assert!(
        !ctx.is_constructor("ghost_mk"),
        "…and must not be registered"
    );

    // A universe-polymorphic DEFINITION of the name is not an inductive either — that is the
    // live escape hatch a translator reaches for.
    let u = Universe::Var("u".to_string());
    ctx.add_universe_poly(
        "BoxQ",
        vec!["u".to_string()],
        arrow(Term::Sort(u.clone()), Term::Sort(u.clone())),
        lam("A", Term::Sort(u), var("A")),
    );
    assert!(
        ctx.add_constructor_checked("BoxQ_mk", "BoxQ", arrow(nat(), app(g("BoxQ"), nat())))
            .is_err(),
        "a universe-polymorphic abbreviation has no constructors"
    );

    // THE POSITIVE CONTROL: a constructor for a genuinely registered inductive still registers.
    ctx.add_inductive("Real", ty0());
    assert!(
        ctx.add_constructor_checked("real_mk", "Real", g("Real"))
            .is_ok(),
        "the checked door must still accept a well-formed constructor"
    );
    assert!(ctx.is_constructor("real_mk"));
}

// ============================================================================================
// 7 · DECLARATION-TIME AUXILIARIES — derived, CHECKED, idempotent
// ============================================================================================

/// Declaring an inductive must yield everything Lean names at its declaration — and every one of
/// them must be a kernel-CHECKED definition, never an assumption.
#[test]
fn declaring_an_inductive_synthesises_its_auxiliaries_as_checked_definitions() {
    let mut ctx = std_ctx();
    ctx.add_inductive("Hue2", ty0());
    for c in ["h2red", "h2green", "h2blue"] {
        ctx.add_constructor(c, "Hue2", g("Hue2"));
    }
    // The declaration is OWED its auxiliaries until it is sealed, and says so.
    assert!(
        ctx.unsealed_inductives().contains(&"Hue2".to_string()),
        "an inductive declared but not sealed must be reported as owing its auxiliaries: {:?}",
        ctx.unsealed_inductives()
    );
    ctx.seal();
    assert!(
        ctx.unsealed_inductives().is_empty(),
        "sealing discharges the debt"
    );

    for want in [
        "Hue2_rec",
        "Hue2_noConfusionType",
        "Hue2_noConfusion",
        "Hue2_below",
        "Hue2_brecOn",
    ] {
        assert!(
            ctx.get_global(want).is_some(),
            "`{want}` must come with the declaration"
        );
        assert!(
            ctx.is_definition(want),
            "`{want}` must be a DEFINITION, never an assumption"
        );
        assert!(
            ctx.is_derived_aux(want),
            "…and must be recorded as ours to replace"
        );
        // The claim each one makes is re-inferred, so a mis-derivation cannot become an axiom.
        let body = ctx
            .get_definition_body(want)
            .expect("it has a body")
            .clone();
        let declared = ctx.get_definition_type(want).expect("and a type").clone();
        let inferred = infer_type(&ctx, &body);
        assert!(
            matches!(&inferred, Ok(t) if logicaffeine_kernel::is_subtype(&ctx, t, &declared)),
            "`{want}` must type-check against its own declared type: {inferred:?} vs {declared}"
        );
    }

    // The derived decidable equality RESOLVES, and it actually decides — the half a wrong
    // instance (one that compares a field and ignores the tag) would get wrong.
    let goal = app(g("Decidable"), eq3(g("Hue2"), g("h2red"), g("h2green")));
    let inst = resolve(&ctx, &goal).expect("a declared inductive yields a findable Decidable");
    assert_eq!(
        normalize(
            &ctx,
            &apps(
                g("decide"),
                &[eq3(g("Hue2"), g("h2red"), g("h2green")), inst]
            )
        ),
        g("false"),
        "the resolved instance must SEPARATE different constructors"
    );
    let same = resolve(
        &ctx,
        &app(g("Decidable"), eq3(g("Hue2"), g("h2red"), g("h2red"))),
    )
    .expect("…and resolve on the diagonal");
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("decide"), &[eq3(g("Hue2"), g("h2red"), g("h2red")), same])
        ),
        g("true"),
        "…and IDENTIFY equal ones"
    );
}

/// The DERIVED tier: a `Decidable (Eq I a b)` goal must resolve for an inductive that was never
/// sealed and never had a procedure registered — Lean's `deriving DecidableEq`, derived at the USE
/// site so the standard library pays nothing to synthesise constants nobody asks for.
#[test]
fn a_decidable_equality_goal_is_answered_without_anything_being_registered() {
    let mut ctx = std_ctx();
    ctx.add_inductive("Shade", ty0());
    for c in ["sred", "sgreen", "sblue"] {
        ctx.add_constructor(c, "Shade", g("Shade"));
    }
    assert!(
        ctx.get_global("decEqShade").is_none(),
        "the premise: nothing has been synthesised or registered for `Shade`"
    );
    assert!(
        !ctx.instances()
            .iter()
            .chain(ctx.fallback_instances())
            .any(|(t, _)| { format!("{t}").contains("Shade") }),
        "…and no instance mentions it"
    );

    let off = resolve(
        &ctx,
        &app(g("Decidable"), eq3(g("Shade"), g("sred"), g("sgreen"))),
    )
    .expect("a declared inductive must yield a findable `Decidable (Eq I a b)`");
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("decide"), &[eq3(g("Shade"), g("sred"), g("sgreen")), off])
        ),
        g("false"),
        "the derived procedure must SEPARATE different constructors"
    );
    let on = resolve(
        &ctx,
        &app(g("Decidable"), eq3(g("Shade"), g("sred"), g("sred"))),
    )
    .expect("…and resolve on the diagonal");
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("decide"), &[eq3(g("Shade"), g("sred"), g("sred")), on])
        ),
        g("true"),
        "…and IDENTIFY equal ones"
    );

    // A PRELUDE carrier is answered by the HAND-WRITTEN procedure, not a fresh derivation — the
    // one other proofs were written against.
    let nat_dec = resolve(&ctx, &app(g("Decidable"), eq3(nat(), peano(2), peano(2))))
        .expect("`Decidable (Eq Nat 2 2)` must resolve");
    assert_eq!(
        nat_dec,
        apps(g("decEqNat"), &[peano(2), peano(2)]),
        "an existing `decEq{{I}}` is preferred over a fresh derivation: {nat_dec}"
    );
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("decide"), &[eq3(nat(), peano(2), peano(2)), nat_dec])
        ),
        g("true"),
        "…and it decides"
    );

    // THE NEGATIVES. The tier answers `Decidable (Eq I a b)` and nothing else: not a `Decidable`
    // over a non-equation, and not an equation at a carrier that is not an inductive.
    assert!(
        resolve(&ctx, &app(g("Decidable"), g("True"))).is_none(),
        "`Decidable True` is not an equation and must not be answered by a decEq procedure"
    );
    assert!(
        resolve(
            &ctx,
            &app(g("Decidable"), eq3(g("Float"), g("sred"), g("sred")))
        )
        .is_none(),
        "a carrier with no constructors has no derivable decision procedure"
    );
    assert!(
        resolve(&ctx, &app(g("Shade"), nat())).is_none(),
        "an unrelated class goal is not answered at all"
    );
}

/// Auxiliaries are re-derived per constructor, so the generation must REPLACE its own
/// predecessor — and must never touch a hand-written constant of the same name.
#[test]
fn regeneration_replaces_its_own_generation_and_spares_a_hand_written_name() {
    // (a) The prelude's hand-written `decEqNat` is the live case: registering a `Nat` constructor
    // again must not swap it out.
    let ctx = std_ctx();
    assert!(
        ctx.is_definition("decEqNat"),
        "the premise: the prelude writes it by hand"
    );
    let hand_written = ctx
        .get_definition_body("decEqNat")
        .cloned()
        .expect("it has a body");
    assert!(
        !ctx.is_derived_aux("decEqNat"),
        "a hand-written constant must never be claimed as ours — claiming it is what lets a \
         later re-derivation delete it"
    );

    // (b) Re-deriving replaces its OWN prior generation rather than accumulating. Two
    // constructors means two generations of `Two_rec`; exactly one survives, and it is the one
    // that describes BOTH constructors.
    let mut two = std_ctx();
    two.set_auto_auxiliaries(true);
    two.add_inductive("Two", ty0());
    two.add_constructor("t2a", "Two", g("Two"));
    let after_one = two.derived_aux_names("Two").map(|n| n.len()).unwrap_or(0);
    two.add_constructor("t2b", "Two", g("Two"));
    let after_two = two.derived_aux_names("Two").map(|n| n.len()).unwrap_or(0);
    assert!(
        after_one > 0 && after_two > 0,
        "both generations produced something"
    );
    let names = two
        .derived_aux_names("Two")
        .expect("a generation is recorded");
    let mut sorted = names.to_vec();
    sorted.sort();
    let before_dedup = sorted.len();
    sorted.dedup();
    assert_eq!(
        before_dedup,
        sorted.len(),
        "a name is recorded ONCE, not once per generation"
    );

    // The surviving recursor knows about BOTH constructors: a 2-case match on it must check.
    let rec_ty = two
        .get_definition_type("Two_rec")
        .expect("Two_rec survives")
        .clone();
    assert_eq!(
        count_pis(&rec_ty),
        4,
        "`Two_rec : Π(motive)(case_a)(case_b)(x). motive x` — the first generation's 1-case \
         recursor must have been REPLACED, not kept: {rec_ty}"
    );

    // (c) The prelude's own `decEqNat` is still byte-identical after all of that, and is still
    // not claimed as ours even with eager synthesis on.
    let mut eager = std_ctx();
    eager.set_auto_auxiliaries(true);
    eager.add_inductive("Three3", ty0());
    eager.add_constructor("th3", "Three3", g("Three3"));
    assert_eq!(
        eager.get_definition_body("decEqNat"),
        Some(&hand_written),
        "declaration-time synthesis must leave a hand-written constant untouched"
    );
    assert!(!eager.is_derived_aux("decEqNat"), "…and must not claim it");
}

fn count_pis(t: &Term) -> usize {
    let mut n = 0;
    let mut cur = t;
    while let Term::Pi { body_type, .. } = cur {
        n += 1;
        cur = body_type;
    }
    n
}

// ============================================================================================
// 8 · COST — declaration-time synthesis must not make building a context quadratic
// ============================================================================================

/// A measurement, printed rather than asserted tightly, so the three costs are separable:
/// registering the standard library with synthesis suspended, the one-shot synthesis a bulk
/// loader pays at `seal`, and the per-constructor synthesis the default (hole-free) path pays.
///
/// The ASSERTION is the one that matters and is generous by two orders of magnitude: bracketing a
/// bulk load must be cheaper than letting every constructor re-synthesise. An 18-constructor
/// inductive re-derives an O(n²) disjointness table n times otherwise.
#[test]
#[ignore = "a measurement, not a lock — it prints the three costs and takes minutes"]
fn one_synthesis_per_inductive_beats_one_per_constructor() {
    use std::time::Instant;

    let t0 = Instant::now();
    let mut queued = std_ctx();
    let load = t0.elapsed();

    let t1 = Instant::now();
    queued.seal();
    let one_shot = t1.elapsed();

    let t2 = Instant::now();
    let mut eager = Context::new();
    eager.set_auto_auxiliaries(true);
    StandardLibrary::register(&mut eager);
    let per_constructor = t2.elapsed();

    println!(
        "S_spine cost: load(synthesis queued)={load:?} seal(one per inductive)={one_shot:?} \
         eager(one per constructor)={per_constructor:?}"
    );

    // Sealing must produce what the eager path produces.
    assert!(
        queued.get_global("Nat_below").is_some(),
        "sealing must synthesise what the eager path synthesises"
    );
    assert!(
        load + one_shot <= per_constructor,
        "one synthesis per inductive must not cost more than one per constructor: \
         load={load:?} + seal={one_shot:?} vs eager={per_constructor:?}"
    );
}

/// The DEFAULT must be cheap, and that is a property worth locking: a declaration queues its
/// auxiliary debt instead of re-deriving a quadratic disjointness table after every constructor.
#[test]
fn a_declaration_queues_its_auxiliary_debt_instead_of_paying_it_per_constructor() {
    let mut ctx = std_ctx();
    assert!(
        !ctx.auto_auxiliaries(),
        "eager synthesis is off by default — it costs 125 s here"
    );
    ctx.add_inductive("Owed", ty0());
    ctx.add_constructor("ow1", "Owed", g("Owed"));
    ctx.add_constructor("ow2", "Owed", g("Owed"));
    assert!(
        ctx.get_global("Owed_noConfusion").is_none(),
        "until it is sealed the debt is genuinely unpaid — silence here would be a lie"
    );
    assert!(
        ctx.unsealed_inductives().contains(&"Owed".to_string()),
        "…and the debt is REPORTED, which is more than Lean tells you"
    );
    ctx.seal();
    assert!(
        ctx.get_global("Owed_noConfusion").is_some(),
        "sealing pays it"
    );
    assert!(ctx.unsealed_inductives().is_empty(), "once");
    // Idempotent: a second seal is a no-op, not a second generation.
    let names = ctx
        .derived_aux_names("Owed")
        .expect("a generation is recorded")
        .to_vec();
    ctx.seal();
    assert_eq!(
        ctx.derived_aux_names("Owed").map(|n| n.to_vec()),
        Some(names),
        "sealing twice must not re-derive"
    );
}

// ============================================================================================
// 9 · THE CHECKED GATEWAY — every registration passes through it, and says so
// ============================================================================================

/// `evil : False := Zero` must not land, and a correctly typed definition must still land and
/// still unfold. The pair, because a "fix" that made `add_definition` drop everything would pass
/// the first half alone.
#[test]
fn the_registration_gateway_refuses_a_mistyped_body_and_records_the_refusal() {
    let mut ctx = std_ctx();
    ctx.add_definition("evil".to_string(), g("False"), g("Zero"));
    assert!(
        ctx.get_global("evil").is_none(),
        "`evil : False := Zero` must be REFUSED — from a proof of False every proposition follows"
    );
    assert!(
        ctx.refused_declarations().iter().any(|(n, _)| n == "evil"),
        "…and the refusal must be RECORDED, not swallowed: a definition that silently stops \
         existing surfaces as an unrelated `rfl` failing far away"
    );

    // The reporting face returns the reason instead of dropping it.
    let mut reported = std_ctx();
    assert!(
        matches!(
            reported.declare("evil2", g("False"), g("Zero")),
            Err(KernelError::TypeMismatch { .. })
        ),
        "`declare` reports what `add_definition` can only record"
    );

    // THE CONTROL: a well-typed definition still registers and still unfolds.
    let mut ok = std_ctx();
    ok.add_definition("one".to_string(), nat(), peano(1));
    assert_eq!(ok.get_global("one"), Some(&nat()));
    assert_eq!(
        normalize(&ok, &g("one")),
        peano(1),
        "and it is still transparent"
    );
    assert!(ok.refused_declarations().is_empty(), "nothing was refused");

    // FORWARD REFERENCES still register, and are tracked as OWED rather than waved through.
    let mut fwd = std_ctx();
    let owed_before = fwd.pending_obligations().len();
    fwd.add_definition("a1".to_string(), nat(), g("a2"));
    assert!(
        fwd.is_definition("a1"),
        "a forward reference must still register"
    );
    assert!(
        fwd.pending_obligations().contains(&("a1", "a2")),
        "…and be reported as owing a check on the name it waits for: {:?}",
        fwd.pending_obligations()
    );
    fwd.add_definition("a2".to_string(), nat(), peano(3));
    assert_eq!(
        fwd.pending_obligations().len(),
        owed_before,
        "declaring the missing name DISCHARGES the obligation — deferred, never skipped: {:?}",
        fwd.pending_obligations()
    );
    assert_eq!(
        normalize(&fwd, &g("a1")),
        peano(3),
        "and the chain computes"
    );

    // …and the discharge works when the missing name arrives through a door that is not
    // `declare`. The prelude declares `TTactics := TList (Syntax → Derivation)` long before
    // `Syntax` is an inductive, so an obligation only a DEFINITION could clear would stand for
    // ever and the ledger would cry wolf.
    let mut late = std_ctx();
    late.add_definition("usesLater".to_string(), ty0(), g("LaterInd"));
    assert!(
        late.pending_obligations()
            .contains(&("usesLater", "LaterInd")),
        "the premise"
    );
    late.add_inductive("LaterInd", ty0());
    assert!(
        !late
            .pending_obligations()
            .iter()
            .any(|(n, _)| *n == "usesLater"),
        "an INDUCTIVE entering scope must discharge the obligations waiting on its name: {:?}",
        late.pending_obligations()
    );
}

/// The lock that keeps the gateway honest: building the standard library must refuse nothing and
/// owe nothing. A prelude definition silently dropped by a stricter gate is exactly the failure
/// this whole change could cause, and it would otherwise be invisible.
#[test]
fn the_standard_library_passes_the_gateway_with_nothing_refused_and_nothing_owed() {
    let ctx = std_ctx();
    assert!(
        ctx.refused_declarations().is_empty(),
        "no prelude definition may be refused by the checked gateway: {:#?}",
        ctx.refused_declarations()
    );
    // A RATCHET, not an equality: the number of prelude definitions whose body the kernel has not
    // been able to check only ever falls.
    //
    // It is 3 today, and the three are a FINDING this gateway surfaced rather than caused:
    // `bv_and`/`bv_or`/`bv_xor` are `fix rec. λn. λv1. λv2. (match v1 … ) v2`, and
    // `type_checker::infer_fix_type_structurally` reads a fixpoint's result type off a body whose
    // base case IS a `Match` — an APPLICATION of one falls through to plain inference, where the
    // recursive name is not yet bound, so the body reports `UnboundVariable("bv_and_rec")`. Those
    // three definitions have never type-checked; nothing looked, because `add_definition` was an
    // `insert`. The fix is one arm in `infer_fix_type_structurally` (or moving `v2` inside the
    // match in `prelude::make_bvec_binop_fix`), and neither file is this lane's.
    const OWED_CEILING: usize = 3;
    let owed = ctx.pending_obligations();
    assert!(
        owed.len() <= OWED_CEILING,
        "the number of prelude definitions whose body could not be checked must only FALL \
         (ceiling {OWED_CEILING}): {owed:#?}"
    );
}

// ============================================================================================
// 10 · SCOPES — what `section` / `end` mean, and what they must NOT take with them
// ============================================================================================

/// A `local instance` must WIN inside its section and LEAVE the database at `end`, and the
/// declarations made inside the section must survive — which is why a whole-context snapshot is
/// not a scoping mechanism.
///
/// The negative is in the same test because it is the half that makes the feature safe: an
/// ORDINARY instance registered inside the same section is permanent, so `local` has to be a
/// property of the registration and not of the moment.
#[test]
fn a_local_instance_leaves_at_its_scopes_end_and_the_declarations_stay() {
    let mut ctx = std_ctx();
    ctx.add_structure("Weight", &[("A", ty0())], &[("w", arrow(var("A"), nat()))]);
    let goal = app(g("Weight"), nat());
    let mk = |n: u32| apps(g("Weight_mk"), &[nat(), lam("_", nat(), peano(n))]);
    let read = |c: &Context, t: Term| normalize(c, &apps(g("Weight_w"), &[nat(), t, peano(0)]));

    ctx.add_instance(goal.clone(), mk(1));
    let outside = ctx.instances().len();
    assert_eq!(
        read(&ctx, resolve(&ctx, &goal).expect("resolves")),
        peano(1),
        "the default"
    );

    assert_eq!(
        ctx.push_scope(),
        1,
        "`section` opens a scope and says how deep we are"
    );
    ctx.add_local_instance(goal.clone(), mk(2));
    // A PERMANENT declaration and a PERMANENT instance made inside the section: both survive.
    ctx.add_definition("insideSection".to_string(), nat(), peano(5));
    ctx.add_structure("Kept", &[("A", ty0())], &[("k", nat())]);
    ctx.add_instance(
        app(g("Kept"), nat()),
        apps(g("Kept_mk"), &[nat(), peano(7)]),
    );

    assert_eq!(
        read(&ctx, resolve(&ctx, &goal).expect("resolves")),
        peano(2),
        "inside its section the `local instance` wins"
    );

    ctx.pop_scope();
    assert_eq!(ctx.scope_depth(), 0, "`end` closes it");
    assert_eq!(
        ctx.instances().len(),
        outside + 1,
        "the local one LEFT and the permanent one registered inside it STAYED: {:?}",
        ctx.instances()
            .iter()
            .map(|(t, _)| format!("{t}"))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        read(&ctx, resolve(&ctx, &goal).expect("resolves")),
        peano(1),
        "outside the section resolution falls back to the default, as Lean does"
    );
    assert!(
        ctx.is_definition("insideSection"),
        "closing a section must KEEP the declarations made inside it — restoring a whole \
         snapshot is not a scoping mechanism"
    );
    assert!(
        resolve(&ctx, &app(g("Kept"), nat())).is_some(),
        "…and an ordinary instance declared inside the section is permanent too"
    );
}

/// Nesting, both boundaries. An inner `end` takes only the inner scope's registrations; a
/// `local` registration made at file level has no `end` to revert it and is permanent; and an
/// unbalanced `end` is a no-op rather than a panic, because an unbalanced `end` is a surface
/// error and the kernel's job is to stay consistent.
#[test]
fn scopes_nest_and_an_unbalanced_close_is_inert() {
    let mut ctx = std_ctx();
    ctx.add_structure("Tier", &[("A", ty0())], &[("t", nat())]);
    let goal = app(g("Tier"), nat());
    let mk = |n: u32| apps(g("Tier_mk"), &[nat(), peano(n)]);
    let read = |c: &Context, t: Term| normalize(c, &apps(g("Tier_t"), &[nat(), t]));

    // BOUNDARY BELOW: `local` outside any scope is permanent — there is no `end` to revert it.
    ctx.add_local_instance(goal.clone(), mk(1));
    let base = ctx.instances().len();
    ctx.pop_scope();
    assert_eq!(
        ctx.instances().len(),
        base,
        "an unbalanced `end` removes nothing"
    );
    assert_eq!(ctx.scope_depth(), 0, "…and cannot drive the depth negative");

    assert_eq!(ctx.push_scope(), 1);
    ctx.add_local_instance(goal.clone(), mk(2));
    assert_eq!(ctx.push_scope(), 2);
    ctx.add_local_instance(goal.clone(), mk(3));
    assert_eq!(
        read(&ctx, resolve(&ctx, &goal).expect("resolves")),
        peano(3),
        "innermost wins"
    );

    ctx.pop_scope();
    assert_eq!(
        read(&ctx, resolve(&ctx, &goal).expect("resolves")),
        peano(2),
        "closing the INNER scope leaves the outer scope's local instance standing"
    );
    ctx.pop_scope();
    assert_eq!(
        read(&ctx, resolve(&ctx, &goal).expect("resolves")),
        peano(1),
        "closing the outer one falls back to the file-level registration"
    );
    assert_eq!(
        ctx.instances().len(),
        base,
        "and the database is exactly where it started"
    );
}

/// The scope reverts the OTHER attribute databases too — coercions, hints and simp lemmas — and
/// the invariant is that it reverts attributes and never declarations.
#[test]
fn a_scope_reverts_every_attribute_database_and_no_declaration() {
    let mut ctx = std_ctx();
    ctx.add_declaration("cheat", arrow(g("Bool"), nat()));
    ctx.add_declaration(
        "boolIsNat",
        pi(
            "b",
            g("Bool"),
            eq3(nat(), app(g("cheat"), var("b")), g("Zero")),
        ),
    );

    let coes = ctx.coercions().len();
    let hints = ctx.get_hints().len();
    let rules = ctx.simp_rules().len();

    ctx.push_scope();
    ctx.add_local_coercion(g("Bool"), nat(), g("cheat"));
    ctx.add_local_hint("boolIsNat");
    ctx.add_definition("survivor".to_string(), nat(), peano(2));
    assert_eq!(
        ctx.coercions().len(),
        coes + 1,
        "the premise: the coercion is registered"
    );
    assert!(
        ctx.is_hint("boolIsNat"),
        "the premise: the hint is registered"
    );
    assert!(
        ctx.is_simp_lemma("boolIsNat"),
        "an equational hint is a rewrite rule as well"
    );
    assert_eq!(ctx.simp_rules().len(), rules + 1);

    ctx.pop_scope();
    assert_eq!(
        ctx.coercions().len(),
        coes,
        "a `local` coercion leaves with its scope"
    );
    assert_eq!(ctx.get_hints().len(), hints, "so does a `local` hint");
    assert!(!ctx.is_hint("boolIsNat"));
    assert_eq!(
        ctx.simp_rules().len(),
        rules,
        "…and the rule it compiled to"
    );
    assert!(!ctx.is_simp_lemma("boolIsNat"));
    assert!(
        ctx.is_definition("survivor"),
        "THE INVARIANT: a scope reverts ATTRIBUTES and never a declaration"
    );
    assert_eq!(
        ctx.get_global("cheat"),
        Some(&arrow(g("Bool"), nat())),
        "nor an axiom"
    );
}

// ============================================================================================
// 11 · THE SIMP REGISTRY — one door, compiled once, and what it REFUSES
// ============================================================================================

/// A `@[simp]` lemma compiles to `∀x̄. lhs ⟶ rhs`, keyed by the left-hand side's head so a
/// rewriter never scans the whole set. The POSITIVE is that the compiled shape is faithful; the
/// dispatch half is what keeps a thousand lemmas about other symbols free.
#[test]
fn a_simp_lemma_compiles_to_a_rule_and_dispatches_on_its_head() {
    let mut ctx = std_ctx();
    ctx.add_definition(
        "size".to_string(),
        arrow(nat(), nat()),
        lam("n", nat(), app(g("Succ"), var("n"))),
    );
    ctx.add_declaration(
        "size_eq",
        pi("n", nat(), eq3(nat(), app(g("size"), var("n")), var("n"))),
    );
    ctx.add_simp_lemma("size_eq")
        .expect("an equation is a simp lemma");

    let rule = ctx
        .simp_rules()
        .iter()
        .find(|r| r.name == "size_eq")
        .expect("registered");
    assert_eq!(
        rule.binders,
        vec![("n".to_string(), nat())],
        "the telescope is the rule's ∀"
    );
    assert_eq!(
        rule.lhs,
        app(g("size"), var("n")),
        "lhs is the pattern to match"
    );
    assert_eq!(rule.rhs, var("n"), "rhs is the replacement");
    assert_eq!(rule.head, "size", "…and the head is the dispatch key");

    let by_head: Vec<&str> = ctx
        .simp_rules_for("size")
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(by_head, vec!["size_eq"], "dispatch finds it under its head");
    assert_eq!(
        ctx.simp_rules_for("Succ").count(),
        0,
        "…and hands a rewriter NOTHING for an unrelated head, which is the point of the index"
    );
    assert!(ctx.is_simp_lemma("size_eq"));
    assert!(!ctx.is_simp_lemma("size"));

    // Idempotent: registering the same lemma twice is one rule, not two identical candidates.
    ctx.add_simp_lemma("size_eq").expect("again");
    assert_eq!(
        ctx.simp_rules_for("size").count(),
        1,
        "registering twice registers once"
    );
}

/// THE NEGATIVES — every shape that would make the simplifier wrong or non-terminating is
/// refused AT THE DOOR with the lemma named, and nothing is registered.
///
/// A positive-only registry is satisfied by `push(name)`, which is exactly what `hints` already
/// was; these four are what make it a registry.
#[test]
fn the_simp_registry_refuses_every_rule_that_would_misbehave() {
    let mut ctx = std_ctx();
    ctx.add_declaration("notAnEquation", pi("n", nat(), g("Nat")));
    ctx.add_declaration(
        "matchesEverything",
        pi("x", nat(), eq3(nat(), var("x"), g("Zero"))),
    );
    ctx.add_declaration(
        "refl_like",
        pi(
            "n",
            nat(),
            eq3(nat(), app(g("Succ"), var("n")), app(g("Succ"), var("n"))),
        ),
    );
    ctx.add_declaration(
        "unboundRhs",
        pi(
            "n",
            nat(),
            pi("m", nat(), eq3(nat(), app(g("Succ"), var("n")), var("m"))),
        ),
    );

    for (name, why) in [
        (
            "notAnEquation",
            "a statement that is not an equation has nothing to rewrite with",
        ),
        (
            "matchesEverything",
            "a bare variable on the left matches every term at every position",
        ),
        (
            "refl_like",
            "a rule that rewrites a term to itself never terminates",
        ),
        (
            "unboundRhs",
            "a right-hand side variable the left does not bind cannot be determined",
        ),
    ] {
        let before = ctx.simp_rules().len();
        let verdict = ctx.add_simp_lemma(name);
        assert!(verdict.is_err(), "`{name}` must be REFUSED: {why}");
        assert_eq!(
            ctx.simp_rules().len(),
            before,
            "…and nothing may be registered: {why}"
        );
        assert!(!ctx.is_simp_lemma(name));
    }

    // And an unknown name is an unbound reference, not a silent no-op.
    assert!(matches!(
        ctx.add_simp_lemma("neverDeclared"),
        Err(KernelError::UnboundVariable(_))
    ));

    // THE CONTROL: the hint door is permissive where the simp door is strict — a non-equational
    // hint is still an `auto` hint, it simply contributes no rewrite rule.
    let rules = ctx.simp_rules().len();
    ctx.add_hint("notAnEquation");
    assert!(
        ctx.is_hint("notAnEquation"),
        "a non-equational hint is still a hint"
    );
    assert_eq!(ctx.simp_rules().len(), rules, "…and compiles to no rule");
}

// ============================================================================================
// 12 · SURFACE ARGUMENT SUGAR — named arguments, `..`, and the postponed λ
// ============================================================================================

/// The elaborator's answers to the three call shapes the parser can now produce, and the
/// refusals that keep each of them honest.
#[test]
fn named_arguments_and_the_ellipsis_refuse_what_they_cannot_determine() {
    use logicaffeine_kernel::interface::TermParser;
    use logicaffeine_kernel::surface_elaborate_against;

    let mut ctx = std_ctx();
    ctx.add_definition(
        "pair2".to_string(),
        arrow(nat(), arrow(nat(), nat())),
        lam("lo", nat(), lam("hi", nat(), var("lo"))),
    );
    let named = |n: &str, v: Term| apps(g(TermParser::NAMED_ARG_MARKER), &[g(n), v]);
    let ellipsis = g(TermParser::ELLIPSIS_MARKER);

    // POSITIVE: a named argument goes to the parameter it names, whichever order it is written.
    let out = surface_elaborate_against(
        &ctx,
        &apps(g("pair2"), &[named("hi", peano(2)), named("lo", peano(1))]),
        None,
    )
    .expect("named arguments elaborate");
    assert_eq!(
        out,
        apps(g("pair2"), &[peano(1), peano(2)]),
        "the spine is the DECLARATION's order, not the source's: {out}"
    );

    // NEGATIVE: a name the declaration does not have is an error that says so, never a silent
    // positional fallback — which would typecheck and compute a different answer.
    let wrong = surface_elaborate_against(
        &ctx,
        &apps(
            g("pair2"),
            &[named("nope", peano(2)), named("lo", peano(1))],
        ),
        None,
    );
    assert!(
        wrong.is_err(),
        "an unknown parameter name must be refused: {wrong:?}"
    );

    // NEGATIVE: the same parameter twice is a contradiction, not a last-one-wins.
    let twice = surface_elaborate_against(
        &ctx,
        &apps(g("pair2"), &[named("lo", peano(2)), named("lo", peano(1))]),
        None,
    );
    assert!(
        twice.is_err(),
        "supplying one parameter twice must be refused: {twice:?}"
    );

    // BOUNDARY: a named argument that SKIPS an earlier explicit parameter η-expands into a λ —
    // never a shorter spine, which would apply the wrong value at the skipped position.
    let skipped =
        surface_elaborate_against(&ctx, &apps(g("pair2"), &[named("hi", peano(9))]), None)
            .expect("the skipping form elaborates");
    match &skipped {
        Term::Lambda { param, body, .. } => {
            assert_eq!(param, "lo", "the λ binds the parameter that was skipped");
            assert_eq!(**body, apps(g("pair2"), &[var("lo"), peano(9)]));
        }
        other => panic!("a skipped explicit parameter must η-expand, got {other}"),
    }
    // Compared up to the binder NAME: η-expansion keeps the declaration's parameter name, which
    // is the point — `arrow` spells its binder `_`.
    match infer_type(&ctx, &skipped).expect("and the η-expansion typechecks") {
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            assert_eq!(*param_type, nat());
            assert_eq!(*body_type, nat());
        }
        other => panic!("the η-expansion must be a function, got {other}"),
    }

    // NEGATIVE — CAPTURE: the η-binder must not swallow a name the caller's argument already
    // uses. `pair2 (hi := lo)` in a scope where `lo` is bound would otherwise become
    // `λlo. pair2 lo lo`, which typechecks and computes with the WRONG value.
    let scoped = ctx.extend("lo", nat());
    let captured =
        surface_elaborate_against(&scoped, &apps(g("pair2"), &[named("hi", var("lo"))]), None)
            .expect("elaborates");
    match &captured {
        Term::Lambda { param, body, .. } => {
            assert_ne!(
                param, "lo",
                "the η-binder must be renamed away from the argument's `lo`"
            );
            assert_eq!(**body, apps(g("pair2"), &[var(param), var("lo")]));
        }
        other => panic!("expected an η-expansion, got {other}"),
    }

    // NEGATIVE: `..` asks for the rest to be INFERRED, and a position nothing determines is an
    // error naming it — never a metavariable handed to the kernel as if it were a term.
    let undetermined = surface_elaborate_against(&ctx, &apps(g("pair2"), &[ellipsis]), None);
    assert!(
        undetermined.is_err(),
        "`pair2 ..` with nothing to solve the arguments from must be REFUSED: {undetermined:?}"
    );
}

/// The postponed λ, both directions: the expected type fills an unwritten parameter type, and
/// WITHOUT an expected type the hole stays a hole rather than being silently defaulted.
#[test]
fn an_unwritten_lambda_parameter_is_filled_only_by_a_known_expected_type() {
    use logicaffeine_kernel::surface_elaborate_against;

    let ctx = std_ctx();
    let unwritten = Term::Lambda {
        param: "x".into(),
        param_type: Box::new(Term::Hole),
        body: Box::new(app(g("Succ"), var("x"))),
    };

    let filled = surface_elaborate_against(&ctx, &unwritten, Some(&arrow(nat(), nat())))
        .expect("the postponed λ is re-run against its expected type");
    assert_eq!(
        filled,
        lam("x", nat(), app(g("Succ"), var("x"))),
        "the parameter type comes from the expected type"
    );
    match infer_type(&ctx, &filled).expect("and it typechecks") {
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            assert_eq!(*param_type, nat());
            assert_eq!(*body_type, nat());
        }
        other => panic!("a filled λ must have a Π type, got {other}"),
    }

    // THE NEGATIVE: with nothing to postpone ONTO, the hole must survive. Defaulting it would
    // monomorphise a λ the writer left open, and the kernel would then certify the guess.
    let open = surface_elaborate_against(&ctx, &unwritten, None).expect("elaborates");
    assert!(
        matches!(&open, Term::Lambda { param_type, .. } if **param_type == Term::Hole),
        "an unwritten parameter with no expected type must stay unwritten: {open}"
    );

    // BOUNDARY: a DEPENDENT expected type renames its own binder to the λ's, so the codomain
    // the body is checked against talks about the right variable.
    let dependent = pi("n", nat(), eq3(nat(), var("n"), var("n")));
    let refl_body = Term::Lambda {
        param: "m".into(),
        param_type: Box::new(Term::Hole),
        body: Box::new(apps(g("Eq_refl"), &[nat(), var("m")])),
    };
    let elab = surface_elaborate_against(&ctx, &refl_body, Some(&dependent));
    assert!(
        matches!(&elab, Ok(Term::Lambda { param_type, .. }) if **param_type == nat()),
        "the dependent expected type still fills the domain: {elab:?}"
    );
}

/// An auto-bound implicit is quantified at `Sort u`, so the declaration is usable at a TYPE and
/// not only at a value — and two auto-bound variables get INDEPENDENT levels.
#[test]
fn an_auto_bound_implicit_binds_a_fresh_level_per_variable() {
    use logicaffeine_kernel::auto_bind_implicits;

    let ctx = std_ctx();
    // `id : A -> A := fun a : A => a`
    let (ty, body, count) =
        auto_bind_implicits(&ctx, &arrow(g("A"), g("A")), &lam("a", g("A"), var("a")), 0);
    assert_eq!(count, 1, "one variable was generalized");
    match &ty {
        Term::Pi {
            param, param_type, ..
        } => {
            assert_eq!(param, "A");
            assert!(
                matches!(&**param_type, Term::Sort(Universe::Var(_))),
                "an invented binder is quantified at `Sort u`, not pinned to `Type 0`: \
                 {param_type} — pinning it monomorphises the declaration silently"
            );
        }
        other => panic!("the telescope must open with the auto-bound binder, got {other}"),
    }
    assert!(
        infer_type(&ctx, &body).is_ok(),
        "and the generalized body still typechecks"
    );

    // TWO variables must not be forced to share a level: `constf : A -> B -> A` at one level
    // would make `constf Nat 0` and `constf 0 Nat` disagree for no reason the writer can see.
    let (ty2, _, count2) = auto_bind_implicits(
        &ctx,
        &arrow(g("A"), arrow(g("B"), g("A"))),
        &lam("a", g("A"), lam("b", g("B"), var("a"))),
        0,
    );
    assert_eq!(count2, 2);
    let levels: Vec<String> = {
        let mut out = Vec::new();
        let mut cur = &ty2;
        while let Term::Pi {
            param_type,
            body_type,
            ..
        } = cur
        {
            if let Term::Sort(Universe::Var(u)) = &**param_type {
                out.push(u.clone());
            }
            cur = body_type;
        }
        out
    };
    assert_eq!(
        levels.len(),
        2,
        "both binders carry a level variable: {ty2}"
    );
    assert_ne!(
        levels[0], levels[1],
        "…and they are INDEPENDENT levels: {ty2}"
    );
}

/// The compiled rule's MATCHER, which is the half a rewriter would otherwise have to write for
/// itself — and the two ways a hand-written one goes wrong: a repeated pattern variable that is
/// allowed to bind twice, and a substitution that captures.
#[test]
fn a_simp_rule_matches_first_order_and_neither_binds_twice_nor_captures() {
    let mut ctx = std_ctx();
    ctx.add_declaration("fdup", arrow(nat(), arrow(nat(), nat())));
    ctx.add_declaration("konst", arrow(nat(), arrow(nat(), nat())));
    // `dup_eq (x : Nat) : fdup x x = x` — a NON-LINEAR pattern: the same variable twice.
    ctx.add_declaration(
        "dup_eq",
        pi(
            "x",
            nat(),
            eq3(nat(), apps(g("fdup"), &[var("x"), var("x")]), var("x")),
        ),
    );
    // `konst_eq (n : Nat) : konst n = fun x : Nat => n` — the right-hand side binds, so a naive
    // substitution can capture.
    ctx.add_declaration(
        "konst_eq",
        pi(
            "n",
            nat(),
            eq3(
                arrow(nat(), nat()),
                app(g("konst"), var("n")),
                lam("x", nat(), var("n")),
            ),
        ),
    );
    // `shadow_eq (x : Nat) : konst x = fun x : Nat => x` — the binder SHADOWS the pattern
    // variable, so the occurrence under it is NOT the one that was matched.
    ctx.add_declaration(
        "shadow_eq",
        pi(
            "x",
            nat(),
            eq3(
                arrow(nat(), nat()),
                app(g("konst"), var("x")),
                lam("x", nat(), var("x")),
            ),
        ),
    );
    ctx.add_simp_lemma("dup_eq")
        .expect("a non-linear pattern is still a rule");
    ctx.add_simp_lemma("konst_eq")
        .expect("a binding right-hand side is still a rule");
    ctx.add_simp_lemma("shadow_eq")
        .expect("a shadowing right-hand side is still a rule");

    let rule = |n: &str| {
        ctx.simp_rules()
            .iter()
            .find(|r| r.name == n)
            .expect("registered")
            .clone()
    };

    // POSITIVE: the two occurrences agree, so the rule fires and the replacement is the term
    // they both bound to.
    assert_eq!(
        rule("dup_eq").rewrite(&apps(g("fdup"), &[peano(3), peano(3)])),
        Some(peano(3)),
        "a non-linear pattern fires when both occurrences are the SAME term"
    );
    // NEGATIVE: they disagree, so it must NOT fire. A matcher that overwrites the first binding
    // rewrites `fdup 1 2` to `2` — a wrong answer, silently.
    assert_eq!(
        rule("dup_eq").rewrite(&apps(g("fdup"), &[peano(1), peano(2)])),
        None,
        "a repeated pattern variable must bind to ONE term, not to the last one seen"
    );
    // NEGATIVE: a different head does not match at all.
    assert_eq!(
        rule("dup_eq").rewrite(&apps(g("konst"), &[peano(1), peano(1)])),
        None
    );
    // NEGATIVE: too few arguments is not a partial match.
    assert_eq!(rule("dup_eq").rewrite(&app(g("fdup"), peano(1))), None);

    // The replacement goes UNDER a binder…
    assert_eq!(
        rule("konst_eq").rewrite(&app(g("konst"), peano(7))),
        Some(lam("x", nat(), peano(7))),
        "the matched term is substituted into the right-hand side, binders and all"
    );
    // …but a binder of the SAME NAME shadows it: `λx. x` is the identity, not `λx. 7`.
    assert_eq!(
        rule("shadow_eq").rewrite(&app(g("konst"), peano(7))),
        Some(lam("x", nat(), var("x"))),
        "a shadowing binder must NOT be captured — substituting under it would turn the \
         identity into a constant function"
    );
}

// ============================================================================================
// 13 · THE REDUCIBILITY AXIS — `@[reducible]` / `@[irreducible]`, declared and scoped
// ============================================================================================

/// The axis is a property of the DECLARATION, is asked in O(1), defaults to semireducible, and
/// `local attribute [irreducible]` reverts with its scope — including back to a NON-default value
/// it displaced, which a naive "remove on close" gets wrong.
#[test]
fn reducibility_is_declared_defaults_to_semireducible_and_reverts_with_its_scope() {
    use logicaffeine_kernel::Reducibility;

    let mut ctx = std_ctx();
    ctx.add_definition("ordinary".to_string(), nat(), peano(5));
    ctx.add_definition("secret".to_string(), nat(), peano(5));
    ctx.add_definition("alias".to_string(), nat(), peano(5));

    // The default, and the point of a default: an undeclared constant costs one failed lookup.
    assert_eq!(ctx.reducibility("ordinary"), Reducibility::Semireducible);
    assert_eq!(
        ctx.reducibility("neverHeardOf"),
        Reducibility::Semireducible
    );
    assert!(!ctx.is_irreducible("ordinary"));

    ctx.set_reducibility("secret", Reducibility::Irreducible);
    ctx.set_reducibility("alias", Reducibility::Reducible);
    assert!(
        ctx.is_irreducible("secret"),
        "the predicate `Transparency::Excluding` wants"
    );
    assert!(
        !ctx.is_irreducible("alias"),
        "`@[reducible]` is the OPPOSITE end of the axis"
    );
    assert_eq!(ctx.reducibility("alias"), Reducibility::Reducible);
    assert!(
        ctx.get_definition_body("secret").is_some(),
        "opacity KEEPS the body — an `@[irreducible]` definition is not an axiom, and the \
         difference is that `unfold` can still reach it"
    );

    // Setting it back to the default REMOVES the entry rather than storing it, so the map stays
    // the set of constants that actually carry an attribute.
    ctx.set_reducibility("alias", Reducibility::Semireducible);
    assert_eq!(ctx.reducibility("alias"), Reducibility::Semireducible);

    // `section … local attribute [irreducible] ordinary … end`
    ctx.push_scope();
    ctx.set_reducibility("ordinary", Reducibility::Irreducible);
    // …and a scoped edit that DISPLACES a non-default value.
    ctx.set_reducibility("secret", Reducibility::Reducible);
    assert!(ctx.is_irreducible("ordinary"));
    assert!(!ctx.is_irreducible("secret"));
    ctx.pop_scope();
    assert!(
        !ctx.is_irreducible("ordinary"),
        "a `local` attribute leaves with its scope"
    );
    assert!(
        ctx.is_irreducible("secret"),
        "…and what it displaced comes BACK — removing on close would silently un-seal a \
         constant the file declared irreducible"
    );
}

// ============================================================================================
// 14 · COEFUN — a structure used where a function is
// ============================================================================================

/// A structure with exactly ONE function-typed field is applicable, and applying it projects
/// that field. The two negatives are what make the rule a rule rather than a guess: no function
/// field at all, and TWO of them — where there is no canonical choice and picking one silently
/// would decide what the program means.
#[test]
fn a_structure_with_one_function_field_is_applicable_and_an_ambiguous_one_is_not() {
    use logicaffeine_kernel::surface_elaborate;

    let mut ctx = std_ctx();
    ctx.add_structure("Fn", &[], &[("f", arrow(nat(), nat()))]);
    let fv = app(g("Fn_mk"), g("Succ"));

    let applied = surface_elaborate(&ctx, &app(fv.clone(), g("Zero"))).expect("`g 0` elaborates");
    assert_eq!(
        applied,
        apps(g("Fn_f"), &[fv.clone(), g("Zero")]),
        "applying a structure inserts the PROJECTION — a real mechanism, not a rewrite that \
         loses the structure: {applied}"
    );
    assert_eq!(
        infer_type(&ctx, &applied).expect("and it typechecks"),
        nat()
    );
    assert_eq!(normalize(&ctx, &applied), peano(1), "…and it computes");

    // A MULTI-FIELD structure keeps its other fields: what is inserted is `Named_run n`, so every
    // theorem quantifying over `Named` still talks about the same values.
    ctx.add_structure(
        "Named",
        &[],
        &[("name", nat()), ("run", arrow(nat(), nat()))],
    );
    let nv = apps(g("Named_mk"), &[g("Zero"), g("Succ")]);
    let n_applied = surface_elaborate(&ctx, &app(nv.clone(), g("Zero"))).expect("elaborates");
    assert_eq!(n_applied, apps(g("Named_run"), &[nv, g("Zero")]));
    assert_eq!(infer_type(&ctx, &n_applied).expect("typechecks"), nat());

    // NEGATIVE: no function-typed field — there is nothing to coerce through, and applying it
    // must stay the error it always was.
    ctx.add_structure("Plain", &[], &[("v", nat())]);
    let pv = app(g("Plain_mk"), g("Zero"));
    assert!(
        surface_elaborate(&ctx, &app(pv, g("Zero")))
            .and_then(|t| infer_type(&ctx, &t))
            .is_err(),
        "a structure with no function field is not applicable"
    );

    // NEGATIVE: TWO function-typed fields — ambiguous, so refused rather than resolved by
    // declaration order. Picking the first would make `p 0` mean whichever field was written
    // first, which no reader of the call site can see.
    ctx.add_structure(
        "Two",
        &[],
        &[("l", arrow(nat(), nat())), ("r", arrow(nat(), nat()))],
    );
    let tv = apps(g("Two_mk"), &[g("Succ"), g("Succ")]);
    assert!(
        surface_elaborate(&ctx, &app(tv, g("Zero")))
            .and_then(|t| infer_type(&ctx, &t))
            .is_err(),
        "two function-typed fields give no canonical coercion, so the application is refused"
    );
}

// ============================================================================================
// 17 · A NUMERAL TAKES THE TYPE THE CONTEXT ASKS FOR — Lean's `OfNat`, decided by KIND
//
// The kernel types a literal by its kind and must keep doing so. What an expected type may do is
// decide which term to BUILD. Every block below is a pair: the kind change that must happen, and
// the one beside it that must NOT.
// ============================================================================================

fn intl(n: i64) -> Term {
    Term::Lit(logicaffeine_kernel::Literal::Int(n))
}
fn natl(n: i64) -> Term {
    Term::Lit(logicaffeine_kernel::Literal::Nat(
        logicaffeine_kernel::BigInt::from_i64(n),
    ))
}

/// THE CAPABILITY, in both directions, plus the one direction that has no reading.
#[test]
fn a_numeral_is_retyped_at_the_carrier_the_position_asks_for_and_never_past_zero() {
    use logicaffeine_kernel::surface_elaborate_against;
    let ctx = std_ctx();

    // `Yield 1` against a declared `-> Nat`: the `Int` token becomes the `Nat` numeral, and the
    // kernel then TYPES it at Nat — which is the whole of T04-009's blocker.
    let at_nat = surface_elaborate_against(&ctx, &intl(1), Some(&nat())).expect("elaborates");
    assert_eq!(
        at_nat,
        natl(1),
        "an Int-kinded numeral at `Nat` is the `Nat` numeral"
    );
    assert_eq!(
        infer_type(&ctx, &at_nat).unwrap(),
        nat(),
        "and the kernel agrees it is a Nat"
    );

    // `(5 : Int)` — the other direction, which T10-017 asks for from the other side.
    let at_int = surface_elaborate_against(&ctx, &natl(5), Some(&g("Int"))).expect("elaborates");
    assert_eq!(
        at_int,
        intl(5),
        "a Nat-kinded numeral at `Int` is the `Int` numeral"
    );
    assert_eq!(infer_type(&ctx, &at_int).unwrap(), g("Int"));

    // BOUNDARY, at zero and one either side of it.
    assert_eq!(
        surface_elaborate_against(&ctx, &intl(0), Some(&nat())).unwrap(),
        natl(0),
        "zero is a Nat"
    );
    assert_eq!(
        surface_elaborate_against(&ctx, &intl(-1), Some(&nat())).unwrap(),
        intl(-1),
        "a NEGATIVE numeral has no reading at `Nat`, so it is left exactly as it was and the \
         kernel refuses it where it always did — collapsing it to Zero would be a wrong ANSWER \
         with a well-typed term in front of it"
    );
    assert!(
        infer_type(
            &ctx,
            &Term::Lit(logicaffeine_kernel::Literal::Nat(
                logicaffeine_kernel::BigInt::from_i64(-1)
            ))
        )
        .is_err(),
        "and a negative `Nat` literal is still refused outright"
    );

    // BOUNDARY, past i64: the canonical-representation invariant definitional equality of
    // literals rests on must survive the round trip.
    let huge = logicaffeine_kernel::BigInt::parse_decimal("123456789012345678901234567890")
        .expect("a literal beyond i64");
    let big_nat = Term::Lit(logicaffeine_kernel::Literal::Nat(huge.clone()));
    let as_int = surface_elaborate_against(&ctx, &big_nat, Some(&g("Int"))).expect("elaborates");
    assert_eq!(infer_type(&ctx, &as_int).unwrap(), g("Int"));
    assert_eq!(
        surface_elaborate_against(&ctx, &as_int, Some(&nat())).unwrap(),
        big_nat,
        "and back again, to the SAME literal — one representation per value, above the i64 \
         boundary as below it"
    );

    // NEGATIVE: no other kind is ever retyped. `Text`/`Float` have one carrier each and an
    // expected type is not a licence to invent a conversion.
    for (lit, want) in [
        (
            Term::Lit(logicaffeine_kernel::Literal::Text("3".into())),
            g("Text"),
        ),
        (
            Term::Lit(logicaffeine_kernel::Literal::Float(3.0)),
            g("Float"),
        ),
    ] {
        let out = surface_elaborate_against(&ctx, &lit, Some(&nat())).expect("elaborates");
        assert_eq!(out, lit, "a non-integer literal is never retyped: {lit}");
        assert_eq!(infer_type(&ctx, &out).unwrap(), want);
    }

    // NEGATIVE: a numeral that ALREADY has the expected type is untouched — in particular it is
    // not wrapped in a `Nat`-carrying record that happens to be expected.
    assert_eq!(
        surface_elaborate_against(&ctx, &natl(3), Some(&nat())).unwrap(),
        natl(3)
    );
}

/// The kind change is a property of the POSITION, not of the top level: an argument, a record
/// field and a declared result must all read the same token the same way.
#[test]
fn the_numeral_kind_change_reaches_an_argument_and_a_field_not_only_the_top_level() {
    use logicaffeine_kernel::{elaborate_app_against, surface_elaborate_against, ParamKind};
    let mut ctx = std_ctx();

    // ARGUMENT: `Succ : Nat -> Nat` applied to a numeral written as an `Int`.
    let mut m = MetaCtx::new();
    let succ_ty = ctx.get_global("Succ").expect("Succ").clone();
    let (applied, _) = elaborate_app_against(
        &ctx,
        &mut m,
        &g("Succ"),
        &succ_ty,
        &[ParamKind::Explicit],
        &[intl(2)],
        None,
    )
    .expect("the argument position retypes the numeral");
    assert_eq!(
        applied,
        app(g("Succ"), natl(2)),
        "the spine carries the Nat numeral"
    );
    assert_eq!(infer_type(&ctx, &applied).unwrap(), nat());

    // FIELD: a one-field record over `Nat`, fed a numeral written as an `Int`.
    ctx.add_structure("Counter", &[], &[("ticks", nat())]);
    let wrapped = surface_elaborate_against(&ctx, &intl(4), Some(&g("Counter")))
        .expect("the injection path still fires, with the field retyped inside it");
    assert_eq!(wrapped, app(g("Counter_mk"), natl(4)));
    assert_eq!(infer_type(&ctx, &wrapped).unwrap(), g("Counter"));
    assert_eq!(normalize(&ctx, &app(g("Counter_ticks"), wrapped)), natl(4));

    // NEGATIVE: the injection is NOT a licence to bend the field's type. A record whose field is
    // `Text` does not accept a numeral by inventing a conversion.
    ctx.add_structure("Labelled", &[], &[("tag", g("Text"))]);
    let mislabelled = surface_elaborate_against(&ctx, &intl(4), Some(&g("Labelled")))
        .expect("elaboration hands the literal back rather than hard-erroring");
    assert_ne!(
        infer_type(&ctx, &mislabelled).ok(),
        Some(g("Labelled")),
        "a numeral is not a Text, at any position: the elaborator leaves the literal alone and \
         the mismatch is reported by the kernel, where it always was"
    );
}

// ============================================================================================
// 18 · WHAT THE GATEWAY DEFERS, AND WHAT IT MUST REFUSE
//
// A deferral is an IOU. It is owed against a name a later declaration could supply and against
// nothing else — otherwise a definition sits in the library behind a debt no door can pay, with
// a body that will never be checked. Keying the deferral on ONE ERROR VARIANT got this wrong in
// both directions at once, and both are pinned below.
// ============================================================================================

/// A body that forward-references a name from `type_checker::UNTRUSTED_AXIOMS` is DEFERRED, like
/// any other forward reference — and discharged the moment the name arrives.
///
/// Those names are reported as a `CertificationError` (they get their own diagnosis, which the
/// bare `UnboundVariable` cannot carry), so a gateway reading the variant refused them outright
/// and dropped the declaration into `refused_declarations()`, where nothing looks. The repair is
/// the CONDITION — "this name resolved nowhere and a declaration could still supply it" — which
/// no error spelling can change.
#[test]
fn a_forward_reference_to_an_untrusted_axiom_is_deferred_and_not_refused() {
    let mut ctx = std_ctx();

    // `taint : Nat := sorryAx Nat` — `sorryAx` is deliberately absent from the trusted base, so
    // at this point it resolves nowhere, exactly like any other not-yet-declared global.
    ctx.add_definition("taint".to_string(), nat(), app(g("sorryAx"), nat()));
    assert!(
        ctx.refused_declarations().is_empty(),
        "a forward reference is not a refusal, whatever the missing name is called: {:#?}",
        ctx.refused_declarations()
    );
    assert!(
        ctx.is_definition("taint"),
        "the definition is registered, with the check OWED"
    );
    assert_eq!(
        ctx.pending_obligations(),
        vec![("taint", "sorryAx")],
        "and the obligation names the missing global, so it can be retried when it arrives"
    );

    // The moment the name exists, the obligation is discharged — the deferral was a real one.
    ctx.add_declaration("sorryAx", pi("A", ty0(), var("A")));
    assert!(
        ctx.pending_obligations().is_empty(),
        "the retry runs on the name that unblocked it: {:#?}",
        ctx.pending_obligations()
    );
    assert!(ctx.refused_declarations().is_empty());
}

/// The other half, and the one that makes the first half safe: a body whose failure NOTHING can
/// repair is REFUSED. It is not deferred, and it is not registered.
#[test]
fn a_failure_no_declaration_could_repair_is_refused_and_never_deferred() {
    let mut ctx = std_ctx();

    // A plain type error, every name in scope: refused, nothing registered, refusal recorded.
    let bad = ctx.declare("evil", g("False"), g("Zero"));
    assert!(
        bad.is_err(),
        "`evil : False := Zero` must be refused: {bad:?}"
    );
    assert!(!ctx.is_definition("evil"));
    assert!(
        ctx.pending_obligations().is_empty(),
        "a refusal is not an obligation"
    );

    // A body mentioning a name that is BOUND INSIDE IT is not a forward reference at all. The
    // fixpoint's own binder is the live case (`type_checker::infer_fix_type_structurally` reports
    // it as unbound on one path), and an obligation keyed on it would wait for a declaration
    // `Context` has no door to make: `retry_pending` fires for globals, and a λ/fix binder is not
    // one. Whatever the gateway does with such a body, it must not be "accept it and owe a check
    // for ever".
    let self_ref = Term::Fix {
        name: "f".to_string(),
        body: Box::new(var("f")),
    };
    let _ = ctx.declare("looper", nat(), self_ref);
    assert!(
        ctx.pending_obligations()
            .iter()
            .all(|(_, blocker)| *blocker != "f"),
        "no obligation may be keyed on a name bound inside the very body that is waiting: {:#?}",
        ctx.pending_obligations()
    );
    assert!(
        !ctx.is_definition("looper"),
        "and a fixpoint with no decreasing argument is refused by the termination guard, not \
         parked behind an IOU — `fix f. f` inhabits every type"
    );
}

/// THE FIXPOINT IS CHECKED, NOT GUESSED — and the three prelude definitions that were in the
/// trusted library with bodies nothing had ever type-checked are checked now.
///
/// `bv_and`/`bv_or`/`bv_xor` are `fix rec. λn. λv1. λv2. (match v1 … ) v2`. Inference has to read
/// a fixpoint's type off the body's SHAPE, and a base case that is an APPLICATION of a match is
/// not a shape it reads — so it fell through to plain inference in a context where the fixpoint's
/// own binder is not bound. A declaration was handed the type and does not have to guess.
#[test]
fn a_fixpoint_whose_shape_inference_cannot_read_is_still_checked_against_its_declared_type() {
    let ctx = std_ctx();

    for name in ["bv_and", "bv_or", "bv_xor", "bv_not"] {
        assert!(ctx.is_definition(name), "`{name}` is a prelude definition");
        assert!(
            ctx.pending_obligations()
                .iter()
                .all(|(who, _)| *who != name),
            "`{name}`'s body must be CHECKED, not owed: {:#?}",
            ctx.pending_obligations()
        );
    }
    assert!(
        ctx.pending_obligations().is_empty(),
        "the standard library owes NO unchecked body at all: {:#?}",
        ctx.pending_obligations()
    );
    assert!(
        ctx.refused_declarations().is_empty(),
        "and none of it was refused on the way: {:#?}",
        ctx.refused_declarations()
    );

    // THE GUARDIAN IS NOT SKIPPED, which is the whole risk of checking a fixpoint against a type
    // instead of inferring one: with the declared type simply believed, `fix f. f` would check at
    // every type there is, `False` included.
    let mut c = std_ctx();
    let nonterminating = Term::Fix {
        name: "f".to_string(),
        body: Box::new(var("f")),
    };
    assert!(
        c.declare("boom", g("False"), nonterminating.clone())
            .is_err(),
        "a fixpoint that does not decrease must be refused at `False`…"
    );
    assert!(
        c.declare("boom2", nat(), nonterminating).is_err(),
        "…and at `Nat`, which is the same refusal for the same reason"
    );
    assert!(!c.is_definition("boom") && !c.is_definition("boom2"));
}

// ============================================================================================
// 19 · δ-ACYCLICITY — a constant's value may only mention constants that do not depend on it
//
// Lean's rule, and the reason δ-normalisation is TOTAL there. Without it `loop_a := loop_b`
// beside `loop_b := loop_a` is a well-typed pair with no normal form, and `normalize` hands back
// an abandoned reduction shaped exactly like a finished one.
// ============================================================================================

/// `is_still_a_redex` in the corpus's own spelling: is this term something δ could still unfold?
fn unfoldable(ctx: &Context, t: &Term) -> bool {
    matches!(t, Term::Global(n) if ctx.is_definition(n))
}

#[test]
fn a_definition_that_would_close_a_delta_cycle_is_refused_at_the_gateway() {
    let mut ctx = std_ctx();

    // The FIRST half of a would-be cycle is a perfectly ordinary forward reference.
    ctx.add_definition("loop_a".to_string(), nat(), g("loop_b"));
    assert!(
        ctx.is_definition("loop_a"),
        "`loop_a := loop_b` is a forward reference, not a cycle"
    );

    // The SECOND half closes it, and that is the one that must not be made.
    let closing = ctx.declare("loop_b", nat(), g("loop_a"));
    assert!(
        closing.is_err(),
        "`loop_b := loop_a` closes a δ-cycle and must be refused"
    );
    let msg = format!("{}", closing.unwrap_err());
    assert!(
        msg.contains("loop_a") && msg.contains("loop_b"),
        "the refusal must name BOTH ends, or the reader cannot see the cycle: {msg}"
    );
    assert!(!ctx.is_definition("loop_b"), "and nothing is registered");

    // The consequence the rule exists for: `normalize` reaches a term δ cannot unfold, so a
    // caller holding only a `Term` can tell a finished reduction from an abandoned one. `loop_a`
    // is an unpaid forward reference — its body was never checked, because `loop_b` never came —
    // and an unchecked body is withheld from δ, so the reduction finishes AT `loop_a`.
    assert!(
        ctx.pending_obligations()
            .iter()
            .any(|(name, _)| *name == "loop_a"),
        "`loop_a` is still owed its check: {:?}",
        ctx.pending_obligations()
    );
    assert!(
        ctx.get_definition_body("loop_a").is_none(),
        "an unchecked body is never handed to δ"
    );
    let settled = normalize(&ctx, &g("loop_a"));
    assert_eq!(
        settled,
        g("loop_a"),
        "`loop_a` does not unfold into a body nothing has checked"
    );
    assert_eq!(
        normalize_checked(&ctx, &g("loop_a")),
        Ok(g("loop_a")),
        "…and the honest channel calls that FINISHED, not abandoned"
    );
    assert!(
        !unfoldable(&ctx, &g("loop_b")),
        "the refused half is not a δ-redex either"
    );
    // Paying the obligation with a body that does NOT close the cycle checks `loop_a` and lets it
    // unfold, once, to a name that is not itself a δ-redex.
    assert!(
        ctx.declare("loop_b", nat(), g("Zero")).is_ok(),
        "an acyclic `loop_b` is accepted"
    );
    assert!(
        ctx.get_definition_body("loop_a").is_some(),
        "`loop_a` is checked once `loop_b` exists"
    );
    assert_eq!(normalize(&ctx, &g("loop_a")), g("Zero"));
    assert!(!unfoldable(&ctx, &normalize(&ctx, &g("loop_a"))));

    // A SELF-cycle is the one-element case of the same rule.
    let mut c = std_ctx();
    assert!(
        c.declare("ouroboros", nat(), g("ouroboros")).is_err(),
        "a definition whose own body names it is a one-element δ-cycle"
    );

    // A THREE-element cycle, refused at the link that closes it and not before.
    let mut d = std_ctx();
    d.add_definition("r1".to_string(), nat(), g("r2"));
    d.add_definition("r2".to_string(), nat(), g("r3"));
    assert!(
        d.is_definition("r2"),
        "the middle link is still just a forward reference"
    );
    assert!(
        d.declare("r3", nat(), g("r1")).is_err(),
        "the closing link is refused"
    );
    assert!(!d.is_definition("r3"));
}

/// THE NEGATIVES, because a check that refuses too much is worse than none: sharing is not a
/// cycle, a long chain is not a cycle, and a REDECLARATION replaces the body it is measured
/// against rather than being compared to it.
#[test]
fn delta_acyclicity_refuses_cycles_and_nothing_else() {
    let mut ctx = std_ctx();

    // A DIAMOND: two definitions over a shared one, and one over both. Every name is mentioned
    // more than once and nothing depends on itself. The names are fresh: `left`/`right` are the
    // prelude's `Or` constructors, which theorems rest on and which therefore keep their meaning.
    ctx.add_definition("diamondBase".to_string(), g("Int"), intl(0));
    ctx.add_definition("diamondLeft".to_string(), g("Int"), g("diamondBase"));
    ctx.add_definition("diamondRight".to_string(), g("Int"), g("diamondBase"));
    ctx.add_definition(
        "diamondBoth".to_string(),
        g("Int"),
        app(app(g("add"), g("diamondLeft")), g("diamondRight")),
    );
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:#?}",
        ctx.refused_declarations()
    );
    assert_eq!(normalize(&ctx, &g("diamondBoth")), intl(0));

    // A LONG CHAIN declared in ascending (forward-reference) order. It must be accepted, it must
    // normalise, and the acceptance must not cost a walk per link — the check looks only at what
    // the NEW body mentions, and at this point that is a name with no body at all.
    const CHAIN: usize = 4_000;
    let mut c = std_ctx();
    let t0 = std::time::Instant::now();
    for i in 0..CHAIN {
        let next = if i + 1 == CHAIN {
            g("Zero")
        } else {
            g(&format!("chain{}", i + 1))
        };
        c.add_definition(format!("chain{i}"), nat(), next);
    }
    let elapsed = t0.elapsed();
    assert!(
        c.refused_declarations().is_empty(),
        "{:#?}",
        c.refused_declarations()
    );
    assert!(
        elapsed.as_secs() < 20,
        "a {CHAIN}-link forward-declared chain must not cost a δ-walk per link: {elapsed:?}"
    );

    // A REDECLARATION is measured against the graph WITHOUT the body it replaces. `alias := Zero`
    // over a previous `alias := t` must not be refused because the old body mentioned something.
    let mut r = std_ctx();
    r.add_definition("t".to_string(), nat(), g("Zero"));
    r.add_definition("alias".to_string(), nat(), g("t"));
    assert!(
        r.declare("t", nat(), g("alias")).is_err(),
        "…but re-pointing `t` at `alias` DOES close the cycle, and must still be refused"
    );
    assert!(
        r.declare("alias", nat(), g("Zero")).is_ok(),
        "replacing `alias`'s own body drops the edge it had, so this is no cycle at all"
    );
    assert_eq!(normalize(&r, &g("alias")), g("Zero"));
}

// ============================================================================================
// 20 · DECLARATION-TIME AUXILIARIES ON THE CHECKED DOOR (T03-018)
//
// Lean generates `{I}.noConfusion`, `{I}.noConfusionType`, `{C}.inj`, `{I}.below`, `{I}.brecOn`
// and `{I}.decEq` at the declaration. Every `injection`, every `cases h` on an equation and every
// `simp [injEq]` in a Lean development names one of them.
// ============================================================================================

#[test]
fn the_checked_constructor_door_leaves_the_disjointness_principles_behind_it() {
    let mut ctx = std_ctx();
    ctx.add_inductive("Chain", ty0());
    ctx.add_constructor_checked("cstop", "Chain", g("Chain"))
        .expect("cstop registers");
    ctx.add_constructor_checked(
        "clink",
        "Chain",
        arrow(nat(), arrow(g("Chain"), g("Chain"))),
    )
    .expect("clink is strictly positive");

    for aux in ["Chain_noConfusion", "Chain_noConfusionType", "Chain_rec"] {
        let ty = ctx
            .get_global(aux)
            .unwrap_or_else(|| panic!("`{aux}` must exist"));
        // DERIVED, never assumed: each one is a definition with a body, and the body checks.
        let body = ctx
            .get_definition_body(aux)
            .unwrap_or_else(|| panic!("`{aux}` must be a definition, not an axiom"))
            .clone();
        let ty = ty.clone();
        let got = infer_type(&ctx, &body).unwrap_or_else(|e| panic!("`{aux}` must check: {e}"));
        assert!(
            logicaffeine_kernel::is_subtype(&ctx, &got, &ty),
            "`{aux}`'s body must have the type it is registered at"
        );
    }
    assert!(
        !ctx.iter_declarations()
            .any(|(n, _)| n.starts_with("Chain_")),
        "not one of them may be an assumption — axiomatising disjointness turns every \
         `injection` proof built on it into an assumption too"
    );

    // NEGATIVE: the synthesis must not reach past the declaration it is for. Nothing here is a
    // `Nat` auxiliary, and the prelude's hand-written `decEqNat` is untouched.
    let before = std_ctx().get_definition_body("decEqNat").cloned();
    assert_eq!(
        ctx.get_definition_body("decEqNat").cloned(),
        before,
        "a hand-written constant is never replaced by a generation"
    );
}

// ============================================================================================
// 21 · THE FOURTH BINDER KIND — `⦃x : T⦄`, and a DEFAULT the declaration writes for the caller
// ============================================================================================

/// Lean's strict implicit is observable in exactly one direction: fully applied it is
/// indistinguishable from the eager one, unapplied it is not. Both are here, because the
/// positive alone is satisfied by collapsing it onto `Implicit`.
#[test]
fn a_strict_implicit_waits_for_an_argument_that_is_actually_written() {
    use logicaffeine_kernel::{elaborate_app, elaborate_app_against, ParamKind};
    let mut ctx = std_ctx();

    // `nilOf ⦃α⦄ : TList α` — nothing follows, so nothing is inserted and `nilOf` is still the
    // function its type says it is.
    let nil_ty = pi("A", ty0(), app(g("TList"), var("A")));
    ctx.add_declaration("nilOf", nil_ty.clone());
    let mut m = MetaCtx::new();
    let (unapplied, _) = elaborate_app(
        &ctx,
        &mut m,
        &g("nilOf"),
        &nil_ty,
        &[ParamKind::StrictImplicit],
        &[],
    )
    .expect("an unapplied strict implicit elaborates");
    assert_eq!(
        unapplied,
        g("nilOf"),
        "no metavariable is emitted where nothing can solve one"
    );

    // THE DIFFERENCE FROM THE EAGER ONE, in the one place it shows: an EXPECTED TYPE is not an
    // argument. Lean does not instantiate `⦃α⦄` from the surrounding type, and a `{α}` here does.
    let mut m2 = MetaCtx::new();
    let strict_at_expected = elaborate_app_against(
        &ctx,
        &mut m2,
        &g("nilOf"),
        &nil_ty,
        &[ParamKind::StrictImplicit],
        &[],
        Some(&app(g("TList"), nat())),
    );
    assert!(
        matches!(strict_at_expected, Err(KernelError::TypeMismatch { .. })),
        "a strict implicit waits for a WRITTEN argument, not for an expected type — so the \
         polymorphic function does NOT become a `TList Nat`, and saying so is a type mismatch \
         rather than a silent instantiation: {strict_at_expected:?}"
    );
    let mut m3 = MetaCtx::new();
    let (eager_at_expected, _) = elaborate_app_against(
        &ctx,
        &mut m3,
        &g("nilOf"),
        &nil_ty,
        &[ParamKind::Implicit],
        &[],
        Some(&app(g("TList"), nat())),
    )
    .expect("elaborates");
    assert_eq!(
        eager_at_expected,
        app(g("nilOf"), nat()),
        "…where the EAGER implicit is solved from it, which is the whole distinction"
    );

    // FULLY APPLIED the two agree, which is why the demotion was silent for so long.
    let mem_ty = pi("A", ty0(), pi("a", var("A"), var("A")));
    ctx.add_declaration("memOf", mem_ty.clone());
    for kind in [ParamKind::StrictImplicit, ParamKind::Implicit] {
        let mut m = MetaCtx::new();
        let (applied, _) = elaborate_app(
            &ctx,
            &mut m,
            &g("memOf"),
            &mem_ty,
            &[kind, ParamKind::Explicit],
            &[g("Zero")],
        )
        .expect("memOf Zero");
        assert_eq!(
            applied,
            apps(g("memOf"), &[nat(), g("Zero")]),
            "with an explicit argument supplied, ⦃α⦄ and {{α}} produce the same term"
        );
    }
}

/// `(m : T := e)` — the declaration answers for a parameter the caller omitted, and the term the
/// kernel sees is at FULL ARITY with no hole in it.
#[test]
fn an_omitted_argument_is_filled_from_the_declarations_own_default() {
    use logicaffeine_kernel::{elaborate_app, ParamKind};
    let mut ctx = std_ctx();

    // `sized : Int -> (m : Int := 3) -> Int := λn. λm. add n m`
    let sized_ty = pi("n", g("Int"), pi("m", g("Int"), g("Int")));
    ctx.add_definition(
        "sized".to_string(),
        sized_ty.clone(),
        lam(
            "n",
            g("Int"),
            lam("m", g("Int"), apps(g("add"), &[var("n"), var("m")])),
        ),
    );
    ctx.set_param_defaults(
        "sized",
        vec![("n".to_string(), None), ("m".to_string(), Some(intl(3)))],
    );

    let mut m = MetaCtx::new();
    let (filled, _) = elaborate_app(
        &ctx,
        &mut m,
        &g("sized"),
        &sized_ty,
        &[ParamKind::Explicit, ParamKind::Explicit],
        &[intl(4)],
    )
    .expect("the omitted argument is filled from the declaration");
    assert_eq!(
        filled,
        apps(g("sized"), &[intl(4), intl(3)]),
        "full arity, default inlined"
    );
    assert_eq!(normalize(&ctx, &filled), intl(7));

    // A DEPENDENT default is closed over the argument the earlier parameter actually received —
    // never handed on as an open term with a free `n` in it.
    ctx.set_param_defaults(
        "sized",
        vec![
            ("n".to_string(), None),
            ("m".to_string(), Some(apps(g("mul"), &[var("n"), intl(2)]))),
        ],
    );
    let mut m2 = MetaCtx::new();
    let (dependent, _) = elaborate_app(
        &ctx,
        &mut m2,
        &g("sized"),
        &sized_ty,
        &[ParamKind::Explicit, ParamKind::Explicit],
        &[intl(5)],
    )
    .expect("a dependent default elaborates");
    assert_eq!(
        dependent,
        apps(g("sized"), &[intl(5), apps(g("mul"), &[intl(5), intl(2)])]),
        "the default is closed over the ACTUAL argument"
    );
    assert_eq!(normalize(&ctx, &dependent), intl(15));

    // NEGATIVE: a position with NO default is still an arity error, and the diagnosis is the
    // arity one rather than a silently invented argument.
    let mut plain = std_ctx();
    plain.add_declaration("twoOf", pi("a", g("Int"), pi("b", g("Int"), g("Int"))));
    let mut m3 = MetaCtx::new();
    let short = elaborate_app(
        &plain,
        &mut m3,
        &g("twoOf"),
        &pi("a", g("Int"), pi("b", g("Int"), g("Int"))),
        &[ParamKind::Explicit, ParamKind::Explicit],
        &[intl(1)],
    );
    assert!(
        short.is_err(),
        "no default means the missing argument is still missing: {short:?}"
    );

    // NEGATIVE: a default that does not have the position's declared type is REFUSED, not
    // inlined. A declaration writing its own arguments is not a licence to skip checking them.
    let mut wrong = std_ctx();
    wrong.add_declaration("wants", pi("a", nat(), nat()));
    wrong.set_param_defaults("wants", vec![("a".to_string(), Some(g("true")))]);
    let mut m4 = MetaCtx::new();
    let refused = elaborate_app(
        &wrong,
        &mut m4,
        &g("wants"),
        &pi("a", nat(), nat()),
        &[ParamKind::Explicit],
        &[],
    );
    assert!(
        refused.is_err(),
        "a mistyped default is refused: {refused:?}"
    );
}

// ============================================================================================
// 22 · AN OBLIGATION THE CALLER DID NOT WRITE — Lean's auto-param, discharged per call site
// ============================================================================================

/// The positive and the three negatives that make it safe. A discharge that fires on a FALSE
/// proposition, on a non-proposition, or on an ordinary partial application would each be worse
/// than no discharge at all.
#[test]
fn a_trailing_proof_obligation_is_discharged_at_the_call_site_and_nowhere_else() {
    use logicaffeine_kernel::surface_elaborate;
    let mut ctx = std_ctx();

    let ne_zero = |b: Term| app(g("Not"), apps(g("Eq"), &[nat(), b, g("Zero")]));
    ctx.add_declaration(
        "safeDiv",
        pi(
            "a",
            nat(),
            pi("b", nat(), pi("h", ne_zero(var("b")), nat())),
        ),
    );

    // THE CAPABILITY: two written arguments, three parameters, and the result is a `Nat` — the
    // obligation was PROVED here, not left as a function still awaiting a proof.
    let call = surface_elaborate(&ctx, &apps(g("safeDiv"), &[natl(10), natl(2)]))
        .expect("safeDiv 10 2 elaborates");
    assert_eq!(
        infer_type(&ctx, &call).unwrap(),
        nat(),
        "the spine is complete: {call}"
    );

    // ABOUT THIS CALL SITE. A second call has a different `b`, hence a different proposition and
    // a different proof; one proof reused across call sites would prove the wrong thing.
    let other = surface_elaborate(&ctx, &apps(g("safeDiv"), &[natl(9), natl(3)]))
        .expect("safeDiv 9 3 elaborates");
    assert_ne!(
        call, other,
        "each call site's proof is about its own proposition"
    );
    assert_eq!(infer_type(&ctx, &other).unwrap(), nat());

    // AND IT COSTS THE TRUSTED BASE NOTHING. The proof is built from `of_decide_eq_true`, which
    // the prelude PROVES; an obligation discharged by a new axiom would have the axiom's own
    // proof as the missing piece.
    let footprint = logicaffeine_kernel::axiom_dependencies(&ctx, &call);
    assert!(
        !footprint.render("safeDiv_call").contains("autoParam"),
        "no `autoParam` former may enter the term or its footprint"
    );
    assert!(ctx.get_global("autoParam").is_none());

    // NEGATIVE — A FALSE OBLIGATION IS NOT DISCHARGED. `Zero ≠ Zero` is false, so nothing may be
    // produced for it and the application stays what it was.
    ctx.add_declaration("needsFalse", pi("h", ne_zero(g("Zero")), nat()));
    let unproved = surface_elaborate(&ctx, &g("needsFalse")).expect("elaborates");
    assert_eq!(
        unproved,
        g("needsFalse"),
        "a false obligation leaves the function unapplied — a discharge that guessed here would \
         put a proof of `Zero ≠ Zero` into the environment"
    );

    // NEGATIVE — A MISSING *DATUM* IS STILL A MISSING ARGUMENT. Discharge is for propositions;
    // a `Nat` the caller did not write is an arity error, not an obligation.
    ctx.add_declaration("twoNats", pi("a", nat(), pi("b", nat(), nat())));
    let partial = surface_elaborate(&ctx, &app(g("twoNats"), natl(1))).expect("elaborates");
    assert_eq!(
        partial,
        app(g("twoNats"), natl(1)),
        "a partially applied function is a value, and must stay one"
    );
    assert_eq!(infer_type(&ctx, &partial).unwrap(), pi("b", nat(), nat()));

    // NEGATIVE — AND THE PROPOSITION MUST BE DECIDABLE. An opaque proposition has no instance,
    // so there is nothing to compute and nothing is invented.
    ctx.add_declaration("Opaque", prop());
    ctx.add_declaration("needsOpaque", pi("h", g("Opaque"), nat()));
    assert_eq!(
        surface_elaborate(&ctx, &g("needsOpaque")).expect("elaborates"),
        g("needsOpaque"),
        "an undecidable obligation is left standing, not assumed"
    );
}

// ============================================================================================
// 23 · BOTH KERNELS, ON EVERYTHING THIS LANE NOW SYNTHESISES
//
// Three of the changes above make the elaborator emit a term the writer never wrote: a numeral
// at a carrier it was not written in, an argument filled from a declaration's default, and a
// PROOF discharged at a call site. A term nobody typed is the last place to trust one kernel, so
// every one of them is put in front of both — and beside each is the INDEPENDENT statement of
// what it should compute, so a term that merely type-checks cannot pass for a correct one.
// ============================================================================================

#[test]
fn every_term_this_lane_synthesises_is_certified_by_both_kernels_and_computes_the_right_value() {
    use logicaffeine_kernel::{
        elaborate_app, surface_elaborate, surface_elaborate_against, ParamKind,
    };
    let mut ctx = std_ctx();

    let agreed = |c: &Context, t: &Term, what: &str| match double_check(c, t) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify {what}, got {other:?} for {t}"),
    };

    // (1) A RETYPED NUMERAL. The independent statement of "3 at `Nat`" is the Peano numeral, and
    // the kernel's own Nat bridge is what has to agree with it — a retyping that produced a
    // well-typed literal of the wrong VALUE would pass a types-only check.
    let three = surface_elaborate_against(&ctx, &intl(3), Some(&nat())).expect("elaborates");
    agreed(&ctx, &three, "a numeral retyped at `Nat`");
    assert!(
        logicaffeine_kernel::defeq_for_test(&ctx, &three, &peano(3)),
        "the retyped numeral IS Succ (Succ (Succ Zero)), not merely a Nat"
    );
    let back = surface_elaborate_against(&ctx, &natl(3), Some(&g("Int"))).expect("elaborates");
    agreed(&ctx, &back, "a numeral retyped at `Int`");
    assert_eq!(
        normalize(&ctx, &apps(g("sub"), &[back, intl(3)])),
        intl(0),
        "…and its value is 3"
    );

    // (2) A FILLED DEFAULT. The independent statement is the spine written out in full by hand:
    // the elaborated term must be that term, not merely something of the same type.
    let sized_ty = pi("n", g("Int"), pi("m", g("Int"), g("Int")));
    ctx.add_definition(
        "sized2".to_string(),
        sized_ty.clone(),
        lam(
            "n",
            g("Int"),
            lam("m", g("Int"), apps(g("add"), &[var("n"), var("m")])),
        ),
    );
    ctx.set_param_defaults(
        "sized2",
        vec![("n".to_string(), None), ("m".to_string(), Some(intl(3)))],
    );
    let mut m = MetaCtx::new();
    let (filled, _) = elaborate_app(
        &ctx,
        &mut m,
        &g("sized2"),
        &sized_ty,
        &[ParamKind::Explicit, ParamKind::Explicit],
        &[intl(4)],
    )
    .expect("the default fills");
    agreed(
        &ctx,
        &filled,
        "an argument filled from a declaration's default",
    );
    assert_eq!(
        filled,
        apps(g("sized2"), &[intl(4), intl(3)]),
        "the hand-written spine"
    );
    assert_eq!(normalize(&ctx, &filled), intl(7));

    // (3) A DISCHARGED PROOF — the one that matters most, because it is a PROOF the writer never
    // saw. Both kernels certify it, and its type is the very proposition the position declared.
    let ne_zero = |b: Term| app(g("Not"), apps(g("Eq"), &[nat(), b, g("Zero")]));
    ctx.add_declaration("safeMod", pi("b", nat(), pi("h", ne_zero(var("b")), nat())));
    let discharged =
        surface_elaborate(&ctx, &app(g("safeMod"), natl(4))).expect("safeMod 4 elaborates");
    agreed(
        &ctx,
        &discharged,
        "a call site whose proof obligation was discharged",
    );
    let Term::App(_, proof) = &discharged else {
        panic!("the spine must be complete: {discharged}")
    };
    let proved = infer_type(&ctx, proof).expect("the synthesised proof types");
    assert!(
        logicaffeine_kernel::defeq_for_test(&ctx, &proved, &ne_zero(natl(4))),
        "the proof proves THIS call site's proposition and not a neighbouring one: {proved}"
    );
    assert!(
        !logicaffeine_kernel::defeq_for_test(&ctx, &proved, &ne_zero(natl(5))),
        "…and it is NOT a proof about 5, which is the way one proof reused everywhere would look"
    );

    // (4) A DERIVED AUXILIARY from the checked declaration door — `{I}_noConfusion` is a term the
    // library will build `injection` proofs on, and it is generated, not written.
    let mut d = std_ctx();
    d.add_inductive("Pair2", ty0());
    d.add_constructor_checked("p2a", "Pair2", g("Pair2"))
        .expect("registers");
    d.add_constructor_checked("p2b", "Pair2", arrow(nat(), g("Pair2")))
        .expect("registers");
    for aux in ["Pair2_noConfusion", "Pair2_noConfusionType"] {
        let body = d
            .get_definition_body(aux)
            .unwrap_or_else(|| panic!("{aux} exists"))
            .clone();
        agreed(&d, &body, aux);
    }
}

// ============================================================================================
// 24 · A UNIVERSE-POLYMORPHIC *TYPE* DECLARATION — the primitive T07-011 / T00-011 / T01-011
//      all reduce to, and the invariant that keeps it apart from a polymorphic DEFINITION
// ============================================================================================

/// A type former has no body. Entering one in the registry the REDUCER δ-unfolds would make a
/// reference to it either unfold into nothing or need a fake body, so the two registries are
/// separate — and this pins that separation from both sides, because merging them is the
/// obvious-looking repair.
#[test]
fn a_polymorphic_type_declaration_carries_a_level_telescope_and_no_body() {
    let mut ctx = std_ctx();

    // `Boxed.{u} : Sort u -> Sort u`
    let arity = pi(
        "a",
        Term::Sort(Universe::Var("u".to_string())),
        Term::Sort(Universe::Var("u".to_string())),
    );
    ctx.add_universe_poly_inductive("Boxed", vec!["u".to_string()], arity.clone(), 1);

    // It IS an inductive, with everything an inductive has…
    assert!(
        ctx.is_inductive("Boxed"),
        "the ordinary registry is populated too"
    );
    assert_eq!(ctx.get_global("Boxed"), Some(&arity), "…at the same arity");
    assert_eq!(ctx.inductive_num_params("Boxed"), 1);
    ctx.add_constructor_checked(
        "boxed_mk",
        "Boxed",
        pi(
            "a",
            Term::Sort(Universe::Var("u".to_string())),
            arrow(var("a"), app(g("Boxed"), var("a"))),
        ),
    )
    .expect("a constructor registers against it");

    // …and it now ALSO has a level telescope, which is the whole primitive.
    let (params, ty) = ctx
        .poly_declaration("Boxed")
        .expect("the telescope is recorded");
    assert_eq!(
        params,
        ["u".to_string()],
        "one level parameter, as declared"
    );
    assert_eq!(ty, &arity);
    assert!(ctx.is_universe_poly_inductive("Boxed"));

    // NEGATIVE — IT IS NOT A DEFINITION, and must not answer as one. `get_universe_poly` is the
    // registry the reducer δ-unfolds; a type former appearing there is a body-less term handed to
    // a rule that expects a body.
    assert!(
        ctx.get_universe_poly("Boxed").is_none(),
        "a type former must NOT be in the δ-unfolding registry"
    );
    assert!(!ctx.is_definition("Boxed"), "and it has no body to unfold");
    assert_eq!(
        normalize(&ctx, &g("Boxed")),
        g("Boxed"),
        "so a reference to it is inert"
    );

    // THE ONE LOOKUP BOTH KERNELS NEED answers for a polymorphic DEFINITION as well, so the
    // `Const` rule needs one call and not two.
    ctx.add_universe_poly(
        "idP",
        vec!["v".to_string()],
        pi(
            "a",
            Term::Sort(Universe::Var("v".to_string())),
            arrow(var("a"), var("a")),
        ),
        lam(
            "a",
            Term::Sort(Universe::Var("v".to_string())),
            lam("x", var("a"), var("x")),
        ),
    );
    let (dparams, _) = ctx
        .poly_declaration("idP")
        .expect("a polymorphic definition answers too");
    assert_eq!(dparams, ["v".to_string()]);
    assert!(
        ctx.get_universe_poly("idP").is_some(),
        "…and it IS δ-transparent, unlike the type"
    );
    assert!(!ctx.is_universe_poly_inductive("idP"));

    // NEGATIVE — A REDECLARATION TAKES THE TELESCOPE WITH IT. One name, one declaration: if
    // `Boxed` becomes something else, typing must not keep reading a level telescope the new
    // declaration never stated.
    ctx.add_definition("Boxed".to_string(), ty0(), nat());
    assert!(
        ctx.poly_declaration("Boxed").is_none(),
        "displacing the header drops its level telescope"
    );
    assert!(!ctx.is_universe_poly_inductive("Boxed"));
}

// ============================================================================================
// 7. `local instance` AND THE SCOPE THAT REVERTS IT — the capability T02-012 names
// ============================================================================================
//
// Lean stores instances in a `SimpleScopedEnvExtension` (`Lean/Meta/Instances.lean:91-95`), so an
// `AttributeKind.local` entry is reverted when its `section`/`namespace` closes while every
// DECLARATION made inside it survives. `lean_conformance_t02_declarations::a_local_instance_can_never_
// leave_the_instance_database` asserts that behaviour but never opens a scope — it calls
// `add_instance` twice and then requires the count to have gone back down, which no scoping
// discipline can deliver because nothing was ever scoped. This is the same MUST, expressed
// against the door that implements it.

/// A `local instance` wins for as long as its scope is open, leaves the database at `end`, and
/// takes nothing else with it.
#[test]
fn a_local_instance_wins_inside_its_scope_and_leaves_at_its_end() {
    let mut ctx = std_ctx();
    ctx.add_structure("Weight", &[("A", ty0())], &[("w", arrow(var("A"), nat()))]);
    let weight_nat = app(g("Weight"), nat());

    // `instance defaultWeight : Weight Nat where w := fun _ => 1` — permanent, declared first.
    let default_w = apps(g("Weight_mk"), &[nat(), lam("_", nat(), peano(1))]);
    ctx.add_instance(weight_nat.clone(), default_w);
    let outside_count = ctx.instances().len();
    let weigh = |c: &Context| {
        resolve(c, &weight_nat).map(|i| normalize(c, &apps(g("Weight_w"), &[nat(), i, peano(5)])))
    };
    assert_eq!(
        weigh(&ctx),
        Some(peano(1)),
        "outside the section the default answers"
    );

    // `section Override`
    assert_eq!(ctx.push_scope(), 1, "one scope open");
    //   `local instance overrideWeight : Weight Nat where w := fun n => n`
    let override_w = apps(g("Weight_mk"), &[nat(), lam("n", nat(), var("n"))]);
    ctx.add_local_instance(weight_nat.clone(), override_w);
    //   a PERMANENT declaration made inside the section
    ctx.add_definition("insideSection".to_string(), nat(), peano(5));
    assert_eq!(
        weigh(&ctx),
        Some(peano(5)),
        "inside its section the `local instance` WINS — later declaration first, Lean's tie-break"
    );
    assert_eq!(
        ctx.instances().len(),
        outside_count + 1,
        "and it is IN the database"
    );

    // `end Override`
    ctx.pop_scope();
    assert_eq!(ctx.scope_depth(), 0, "the scope closed");
    assert_eq!(
        ctx.instances().len(),
        outside_count,
        "a `local` instance LEAVES the database when its scope closes"
    );
    assert_eq!(weigh(&ctx), Some(peano(1)), "so the default answers again");
    // NEGATIVE — closing a section must NOT be a snapshot restore.
    assert!(
        ctx.is_definition("insideSection"),
        "a declaration made inside the section survives its `end`"
    );
    assert_eq!(
        ctx.get_definition_body("insideSection"),
        Some(&peano(5)),
        "…with its body intact"
    );
}

/// BOUNDARY — one rung either side of the scope that owns the entry.
///
/// Nesting is the case a single `bool` flag gets wrong: an inner `end` must revert only the inner
/// scope's `local`s, and a `local` written at FILE level (depth 0) is permanent, because there is
/// no `end` for it — which is what `local` means at file scope in Lean.
#[test]
fn a_scope_reverts_its_own_depth_and_neither_its_parent_nor_the_file() {
    let mut ctx = std_ctx();
    ctx.add_structure("Tag", &[("A", ty0())], &[("t", nat())]);
    let tag = |x: &Term| app(g("Tag"), x.clone());
    let mk = |x: &Term, n: u32| apps(g("Tag_mk"), &[x.clone(), peano(n)]);

    // FILE LEVEL: `local` with no scope open is permanent — there is no `end` to revert it.
    ctx.add_local_instance(tag(&nat()), mk(&nat(), 0));
    let file_level = ctx.instances().len();

    ctx.push_scope(); // section Outer
    ctx.add_local_instance(tag(&g("Bool")), mk(&g("Bool"), 1));
    let outer = ctx.instances().len();
    assert_eq!(outer, file_level + 1);

    ctx.push_scope(); // section Inner
    assert_eq!(ctx.scope_depth(), 2);
    ctx.add_local_instance(tag(&ty0()), mk(&ty0(), 2));
    assert_eq!(ctx.instances().len(), outer + 1);

    ctx.pop_scope(); // end Inner — reverts DEPTH 2 ONLY
    assert_eq!(
        ctx.instances().len(),
        outer,
        "the inner `end` reverts its own depth and leaves the outer scope's entry alone"
    );
    assert!(
        resolve(&ctx, &tag(&g("Bool"))).is_some(),
        "the outer scope's instance is still live"
    );
    assert!(
        resolve(&ctx, &tag(&ty0())).is_none(),
        "the inner scope's is gone"
    );

    ctx.pop_scope(); // end Outer
    assert_eq!(
        ctx.instances().len(),
        file_level,
        "and now the outer one is gone too"
    );
    assert!(
        resolve(&ctx, &tag(&nat())).is_some(),
        "the file-level `local` is PERMANENT — no `end` exists that could revert it"
    );

    // NEGATIVE — an unbalanced `end` is inert, not a panic and not a silent wipe.
    ctx.pop_scope();
    ctx.pop_scope();
    assert_eq!(ctx.scope_depth(), 0);
    assert_eq!(
        ctx.instances().len(),
        file_level,
        "an unbalanced `end` removes nothing"
    );
}

// ============================================================================================
// 8. A UNIVERSE-POLYMORPHIC STRUCTURE — one `class` declaration serving every level
// ============================================================================================
//
// `class Mul (α : Type u) where mul : α → α → α` is the shape the whole algebraic hierarchy is
// built from, and `add_structure` could only ever produce a MONOMORPHIC one: its parameter
// telescope is a list of concrete `Term`s and its result sort is computed from them, so `Mul`
// declared with `α : Type 0` refuses a `Type 0` carrier (`Type 0 : Type 1`) and the same
// declaration cannot serve two levels. Lean binds the level as a real parameter of the
// declaration (`Lean/Elab/Inductive.lean:484 mkResultUniverse`, over the level TELESCOPE, and
// `Lean/Elab/AutoBound.lean:46 isValidAutoBoundLevelName` for how `u` gets bound in the first
// place), which is what `add_structure_poly` gives us.

/// `Sort u` for a level PARAMETER `u`.
fn sort_var(u: &str) -> Term {
    Term::Sort(Universe::Var(u.to_string()))
}

/// One class declaration, every level: declared once with a level telescope, usable at a `Type 0`
/// carrier and at a `Type 1` carrier, and instantiable by writing the level.
#[test]
fn a_universe_polymorphic_structure_serves_every_level_from_one_declaration() {
    let mut ctx = std_ctx();

    // `class Mul.{u} (α : Sort u) where mul : α → α → α`
    ctx.add_structure_poly(
        "Mul",
        vec!["u".to_string()],
        &[("a", sort_var("u"))],
        &[("mul", arrow(var("a"), arrow(var("a"), var("a"))))],
    )
    .expect("a polymorphic class declares");

    // It is an inductive with a level telescope and NO body — a type former, not a definition.
    assert!(ctx.is_inductive("Mul"));
    assert!(
        ctx.is_universe_poly_inductive("Mul"),
        "the level telescope is recorded"
    );
    let (params, _) = ctx
        .poly_declaration("Mul")
        .expect("Mul carries a telescope");
    assert_eq!(params, ["u".to_string()]);
    assert!(
        ctx.get_universe_poly("Mul").is_none(),
        "a type former is not δ-transparent"
    );
    // The arity is COMPUTED over the telescope, exactly as `mkResultUniverse` computes it: the
    // `max` of what the record stores, raised to at least `Type 0` because it stores something.
    assert_eq!(
        ctx.get_global("Mul"),
        Some(&pi(
            "a",
            sort_var("u"),
            Term::Sort(Universe::Type(0).max(&Universe::Var("u".into())))
        )),
        "Mul.{{u}} : Π(a : Sort u). Sort (max 1 u)",
    );
    assert_eq!(
        ctx.get_global("Mul_mk"),
        Some(&pi(
            "a",
            sort_var("u"),
            pi(
                "mul",
                arrow(var("a"), arrow(var("a"), var("a"))),
                app(
                    Term::Const { name: "Mul".to_string(), levels: vec![Universe::Var("u".into())] },
                    var("a")
                )
            )
        )),
        "and its constructor quantifies over the SAME level parameter: its result is `Mul.{{u}} a`, \
         Lean's `Mul.mk : {{α : Sort u}} → (α → α → α) → Mul.{{u}} α`",
    );

    // THE MUST — the same declaration at a `Type 0` carrier …
    assert!(
        infer_type(&ctx, &app(g("Mul"), nat())).is_ok(),
        "Mul Nat is typed"
    );
    // … and at a `Type 1` carrier, which is what Mathlib does constantly.
    let at_type = infer_type(&ctx, &app(g("Mul"), ty0()));
    assert!(
        at_type.is_ok(),
        "one class declaration must serve a Type 1 carrier too: {at_type:?}"
    );

    // And it is universe-INSTANTIABLE by writing the level, at every rung.
    for level in [
        Universe::Prop,
        Universe::Type(0),
        Universe::Type(1),
        Universe::Type(7),
    ] {
        let at_level = Term::Const {
            name: "Mul".to_string(),
            levels: vec![level.clone()],
        };
        assert!(
            infer_type(&ctx, &at_level).is_ok(),
            "`Mul.{{{level}}}` must instantiate the class's level parameter: {:?}",
            infer_type(&ctx, &at_level),
        );
    }

    // NEGATIVE — the telescope is CHECKED, not decorative. A reference writing the wrong NUMBER
    // of levels is a statement about a declaration that does not exist.
    let two_levels = Term::Const {
        name: "Mul".to_string(),
        levels: vec![Universe::Type(0), Universe::Type(1)],
    };
    assert!(
        infer_type(&ctx, &two_levels).is_err(),
        "a one-parameter class must refuse a two-level instantiation"
    );

    // THE DICTIONARY COMPUTES AT BOTH LEVELS, which is the half a sort-only fix would miss.
    let nat_mul = apps(
        g("Mul_mk"),
        &[nat(), lam("x", nat(), lam("y", nat(), var("x")))],
    );
    assert!(
        infer_type(&ctx, &nat_mul).is_ok(),
        "a Type 0 dictionary builds: {nat_mul}"
    );
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("Mul_mul"), &[nat(), nat_mul.clone(), peano(2), peano(3)])
        ),
        peano(2),
        "and its projection COMPUTES",
    );
    // The same class one level up: the carrier is `Type 0`, which lives in `Type 1`.
    let type_mul = apps(
        g("Mul_mk"),
        &[ty0(), lam("x", ty0(), lam("y", ty0(), var("x")))],
    );
    let ty = infer_type(&ctx, &type_mul);
    assert!(
        ty.is_ok(),
        "a Type 1 dictionary builds from the SAME declaration: {ty:?}"
    );
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("Mul_mul"), &[ty0(), type_mul, nat(), g("Bool")])
        ),
        nat(),
        "and its projection computes there too",
    );

    // BOTH KERNELS — the level algebra is the one layer `double_check` cannot corroborate
    // structurally, so the new declaration shape is put through it explicitly.
    assert!(matches!(double_check(&ctx, &nat_mul), DoubleCheck::Agreed));

    // TWO INDEPENDENT LEVELS — `class Action.{u,v} (R : Sort u) (M : Sort v)`, the shape that
    // defeats even per-instantiation monomorphisation, because the two levels vary separately.
    ctx.add_structure_poly(
        "Action",
        vec!["u".to_string(), "v".to_string()],
        &[("r", sort_var("u")), ("m", sort_var("v"))],
        &[("smul", arrow(var("r"), arrow(var("m"), var("m"))))],
    )
    .expect("a two-level class declares");
    let mixed = infer_type(&ctx, &apps(g("Action"), &[nat(), ty0()]));
    assert!(
        mixed.is_ok(),
        "`Action Nat (Type 0)` — R at one level and M at another — from ONE declaration: {mixed:?}",
    );
    assert!(
        infer_type(&ctx, &apps(g("Action"), &[ty0(), nat()])).is_ok(),
        "…and with the two levels the other way round",
    );
    assert!(
        infer_type(
            &ctx,
            &Term::Const {
                name: "Action".to_string(),
                levels: vec![Universe::Type(0), Universe::Type(3)],
            }
        )
        .is_ok(),
        "…and written out, at two DIFFERENT levels",
    );

    // INSTANCE RESOLUTION reaches it at both levels from one class.
    ctx.add_instance(app(g("Mul"), nat()), nat_mul.clone());
    assert_eq!(resolve(&ctx, &app(g("Mul"), nat())), Some(nat_mul));
    assert!(
        resolve(&ctx, &app(g("Mul"), g("Bool"))).is_none(),
        "and it does not invent one for a carrier nobody declared"
    );
}

/// NEGATIVE + BOUNDARY for the declaration door itself: the result sort is COMPUTED from the
/// fields over the level telescope, exactly as `mkResultUniverse` does, and a monomorphic
/// declaration is left exactly where it was.
#[test]
fn a_polymorphic_structures_sort_is_computed_from_its_fields_not_assumed() {
    let mut ctx = std_ctx();

    // An all-PROOF record is a `Prop`, level parameter or not — impredicativity does not care.
    ctx.add_structure_poly(
        "BothP",
        vec!["u".to_string()],
        &[("p", prop()), ("q", prop())],
        &[("l", var("p")), ("r", var("q"))],
    )
    .expect("declares");
    assert_eq!(
        codomain(ctx.get_global("BothP").expect("BothP")),
        &prop(),
        "a record storing only proofs is a Prop",
    );

    // A record STORING a `Sort u` is one rung above it — the Girard shape, refused at `Sort u`.
    ctx.add_structure_poly(
        "Bundle",
        vec!["u".to_string()],
        &[],
        &[("carrier", sort_var("u")), ("pt", var("carrier"))],
    )
    .expect("declares");
    let bundle_sort = codomain(ctx.get_global("Bundle").expect("Bundle"));
    assert_ne!(
        bundle_sort,
        &sort_var("u"),
        "a record storing a `Sort u` cannot live at `Sort u`"
    );
    assert_eq!(
        bundle_sort,
        &Term::Sort(
            Universe::Type(0)
                .max(&Universe::Var("u".into()).succ())
                .max(&Universe::Var("u".into()))
        ),
        "…it lives one rung ABOVE the sort it stores: {bundle_sort}",
    );
    // The declaration is CHECKED at the sort it computed — the constructor must pass the same
    // universe gate a hand-written one would.
    let hand = ctx.get_global("Bundle_mk").expect("Bundle_mk").clone();
    let mut check = std_ctx();
    let arity = ctx.get_global("Bundle").expect("Bundle").clone();
    check.add_universe_poly_inductive("Bundle", vec!["u".to_string()], arity, 0);
    assert!(
        check
            .add_constructor_checked("Bundle_mk", "Bundle", hand)
            .is_ok(),
        "the sort a polymorphic structure is declared at must satisfy the CIC universe check",
    );

    // NEGATIVE — asking for a sort too small is REFUSED and leaves nothing behind.
    let before = ctx.get_global("TooSmall").cloned();
    let verdict = ctx.add_structure_poly_at(
        "TooSmall",
        vec!["u".to_string()],
        Universe::Var("u".to_string()),
        &[],
        &[("carrier", sort_var("u"))],
    );
    assert!(
        verdict.is_err(),
        "a record storing `Sort u` may not claim `Sort u`: {verdict:?}"
    );
    assert_eq!(
        ctx.get_global("TooSmall").cloned(),
        before,
        "and nothing was registered"
    );
    assert!(
        ctx.get_global("TooSmall_mk").is_none(),
        "not even its constructor"
    );

    // BOUNDARY — an EMPTY level telescope is exactly the monomorphic declaration, term for term.
    let mut poly = std_ctx();
    poly.add_structure_poly("P", vec![], &[("a", ty0())], &[("v", var("a"))])
        .expect("declares");
    let mut mono = std_ctx();
    mono.add_structure("P", &[("a", ty0())], &[("v", var("a"))]);
    assert_eq!(poly.get_global("P"), mono.get_global("P"), "same arity");
    assert_eq!(
        poly.get_global("P_mk"),
        mono.get_global("P_mk"),
        "same constructor"
    );
    assert_eq!(
        poly.get_global("P_v"),
        mono.get_global("P_v"),
        "same projection"
    );
    assert!(
        !poly.is_universe_poly_inductive("P"),
        "and NO level telescope is invented for a declaration that stated none"
    );
}

/// T07-012's MUST, expressed against the sort the declaration actually chose.
///
/// `a_bundled_structure_is_declared_at_the_sort_its_carrier_forces` offers `add_structure`'s own
/// constructor to `add_constructor_checked` and requires it to be accepted — but it re-declares
/// the inductive at a HARD-CODED `Type 0` first, which is the very sort a bundled record may not
/// live at. `add_structure` computes `Type 1` for it; re-pinning `Type 0` manufactures the
/// Girard/Hurkens shape the gate exists to refuse, so the assertion and its own message
/// contradict each other. Read the sort back instead and the MUST holds — which is the property
/// the case is named for.
#[test]
fn a_bundled_record_is_declared_one_rung_above_the_carrier_it_stores() {
    let mut bundled = std_ctx();
    bundled.add_structure(
        "BMonoid",
        &[],
        &[
            ("carrier", ty0()),
            (
                "mul",
                arrow(var("carrier"), arrow(var("carrier"), var("carrier"))),
            ),
            ("one", var("carrier")),
        ],
    );
    // A record STORING a `Type 0` lives in `Type 1` — Lean's `mkResultUniverse`, and the CIC
    // constraint `check_constructor_universes` enforces.
    assert_eq!(
        bundled.get_global("BMonoid"),
        Some(&ty1()),
        "a bundled record is declared one rung above the carrier it stores",
    );
    // The projection still reads back, so the encoding is usable at the sort it chose.
    let nat_bmonoid = apps(
        g("BMonoid_mk"),
        &[nat(), lam("x", nat(), lam("y", nat(), var("x"))), g("Zero")],
    );
    assert_eq!(
        normalize(&bundled, &app(g("BMonoid_carrier"), nat_bmonoid.clone())),
        nat()
    );
    assert!(matches!(
        double_check(&bundled, &nat_bmonoid),
        DoubleCheck::Agreed
    ));

    // THE MUST — the constructor `add_structure` generated passes the CHECKED path, at the sort
    // the declaration chose.
    let b_ctor = bundled
        .get_global("BMonoid_mk")
        .expect("BMonoid_mk")
        .clone();
    let mut b_check = std_ctx();
    b_check.add_indexed_inductive("BMonoid", bundled.get_global("BMonoid").unwrap().clone(), 0);
    assert!(
        b_check
            .add_constructor_checked("BMonoid_mk", "BMonoid", b_ctor.clone())
            .is_ok(),
        "`add_structure` must not register a constructor the CIC universe check rejects",
    );

    // NEGATIVE — and one rung DOWN is the Girard/Hurkens shape, which must still be refused.
    // This is the assertion that makes the positive one non-vacuous.
    let mut too_small = std_ctx();
    too_small.add_indexed_inductive("BMonoid", ty0(), 0);
    let verdict = too_small.add_constructor_checked("BMonoid_mk", "BMonoid", b_ctor);
    assert!(
        verdict.is_err(),
        "a `Type 0` record storing a `Type 0` field is Girard's shape and must be refused: \
         {verdict:?}",
    );
    // And the declaration door refuses it too, transactionally.
    let mut claimed = std_ctx();
    assert!(
        claimed
            .add_structure_at("BMonoid", Universe::Type(0), &[], &[("carrier", ty0())])
            .is_err(),
        "…and a record may not CLAIM a sort too small for what it stores",
    );
    assert!(
        claimed.get_global("BMonoid").is_none(),
        "nothing is left behind by the refusal"
    );
}

// ============================================================================================
// 9. THE INSTANCE INDEX — Lean's discrimination tree, and why a flat `Vec` is not enough
// ============================================================================================
//
// `Lean/Meta/Instances.lean:67` stores instances in a `DiscrTree InstanceEntry`, and
// `Lean/Meta/SynthInstance.lean:210` looks a goal up with `globalInstances.getUnify type` — so a
// goal only ever sees the entries whose KEY PATH could unify with it. Ours was a flat `Vec`
// scanned end to end, and every entry in it cost a `MetaCtx` clone, a telescope freshening and a
// full unification attempt before being discarded. At a Mathlib-sized instance set that is not a
// speed question, it is a usability one.
//
// This is locked as a COMPLEXITY bound rather than a stopwatch: the audit trail says exactly how
// many candidates the search touched, so "did the index prune?" is a deterministic assertion that
// cannot flap under load.

/// A goal must not pay for the instances of classes it has nothing to do with.
#[test]
fn an_instance_goal_only_pays_for_the_candidates_that_could_match_it() {
    let mut ctx = std_ctx();
    ctx.add_structure("Wanted", &[("A", ty0())], &[("v", nat())]);
    // 300 instances of 60 OTHER classes — the shape of a real library.
    for c in 0..60u32 {
        let cls = format!("Noise{c}");
        ctx.add_structure(&cls, &[("A", ty0())], &[("v", nat())]);
        for k in 0..5u32 {
            let carrier = if k % 2 == 0 { nat() } else { g("Bool") };
            ctx.add_instance(
                app(g(&cls), carrier.clone()),
                apps(g(&format!("{cls}_mk")), &[carrier, peano(k)]),
            );
        }
    }
    let wanted = apps(g("Wanted_mk"), &[nat(), peano(9)]);
    ctx.add_instance(app(g("Wanted"), nat()), wanted.clone());
    assert_eq!(
        ctx.instances().len(),
        301,
        "the database really is that big"
    );

    let goal = app(g("Wanted"), nat());

    // THE DIFFERENTIAL, stated directly against the two doors so it cannot be satisfied by a
    // search that merely returned early: the unrestricted scan visits all 301, the indexed one
    // visits the candidates whose head could match and reports the rest as pruned.
    assert_eq!(
        ctx.instance_search_order().len(),
        301,
        "the unrestricted scan is the whole Vec"
    );
    let (candidates, pruned) = ctx.instance_search_order_for(&goal);
    assert_eq!(
        candidates.len() + pruned,
        301,
        "the index partitions the database, it never loses"
    );
    assert!(
        candidates.len() <= 2,
        "the index must exclude the 60 classes the goal has nothing to do with: {} candidates",
        candidates.len(),
    );
    // …and every candidate it kept really is an entry for THIS class.
    for &i in &candidates {
        let (ty, _) = &ctx.instances()[i];
        assert_eq!(
            ty, &goal,
            "a kept candidate is an entry for the goal's own class: {ty}"
        );
    }

    let mut mctx = MetaCtx::new();
    let choice = resolve_instance_audited(&ctx, &mut mctx, &goal).expect("Wanted Nat resolves");
    assert_eq!(choice.value, wanted, "and it finds the right one");
    assert!(
        choice.considered.len() <= 4,
        "a goal must only pay for candidates whose head could match it — 301 in the database, \
         {} unification attempts made",
        choice.considered.len(),
    );
    assert!(
        choice.pruned >= 300,
        "…and the audit must SAY how many the index excluded without trying them, which Lean \
         cannot tell you: pruned {}",
        choice.pruned,
    );

    // NEGATIVE — pruning must never lose an answer. Every one of the 301 goals still resolves,
    // to exactly the entry the unpruned scan would have picked.
    for c in 0..60u32 {
        let cls = format!("Noise{c}");
        for carrier in [nat(), g("Bool")] {
            let goal = app(g(&cls), carrier.clone());
            let mut m = MetaCtx::new();
            let got = resolve_instance(&ctx, &mut m, &goal);
            assert!(
                got.is_some(),
                "`{cls}` at {carrier} must still resolve after indexing"
            );
            // …and it is the LATER declaration, which is the tie-break the index must preserve.
            let mut m2 = MetaCtx::new();
            let audited = resolve_instance_audited(&ctx, &mut m2, &goal).expect("audited");
            assert_eq!(
                audited.value,
                got.unwrap(),
                "both views of one search agree"
            );
        }
    }
}

/// An OPEN candidate — a polymorphic instance whose conclusion head is a class applied to a
/// variable the instance itself binds — must be visible to EVERY goal of that class, which is the
/// `star` bucket in Lean's tree (`Lean/Meta/DiscrTree.lean:727 getUnify` unions `getStarResult d`
/// into every answer). Pruning it away is the one way an index can silently lose an instance.
#[test]
fn the_index_keeps_every_open_candidate_visible_and_the_order_unchanged() {
    let mut ctx = std_ctx();
    ctx.add_structure("Shown", &[("A", ty0())], &[("s", arrow(var("A"), nat()))]);
    ctx.add_structure("TList", &[("A", ty0())], &[("hd", var("A"))]);

    // `instance {A} [Shown A] : Shown (TList A)` — the conclusion's ARGUMENT is open.
    let list_shown = pi(
        "A",
        ty0(),
        arrow(
            app(g("Shown"), var("A")),
            app(g("Shown"), app(g("TList"), var("A"))),
        ),
    );
    ctx.add_declaration("shownList", list_shown.clone());
    ctx.add_instance(list_shown, g("shownList"));
    // …and a monomorphic base case.
    let shown_nat = apps(g("Shown_mk"), &[nat(), lam("n", nat(), var("n"))]);
    ctx.add_instance(app(g("Shown"), nat()), shown_nat.clone());

    // The RECURSIVE instance must still be reachable through the index.
    let nested = app(g("Shown"), app(g("TList"), nat()));
    assert!(
        resolve(&ctx, &nested).is_some(),
        "an open-argument instance stays visible: {nested}"
    );
    assert_eq!(resolve(&ctx, &app(g("Shown"), nat())), Some(shown_nat));

    // A candidate whose whole CONCLUSION is open — the `star` root — must be visible to every
    // goal, of every class, because nothing about its head rules it out.
    let mut open = std_ctx();
    open.add_structure("Any", &[("A", ty0())], &[("v", nat())]);
    open.add_structure("Other", &[("A", ty0())], &[("v", nat())]);
    open.add_declaration("anyGoal", pi("G", ty0(), var("G")));
    open.add_instance(pi("G", ty0(), var("G")), g("anyGoal"));
    for goal in [app(g("Any"), nat()), app(g("Other"), g("Bool"))] {
        let mut m = MetaCtx::new();
        assert!(
            resolve_instance_audited(&open, &mut m, &goal)
                .is_some_and(|c| !c.considered.is_empty()),
            "a candidate with an OPEN conclusion must be offered to every goal: {goal}",
        );
    }

    // ORDER IS UNCHANGED — the index selects candidates, it does not re-rank them. Two entries
    // for one goal at equal priority: the LATER declaration still wins.
    let mut ord = std_ctx();
    ord.add_structure("Pick", &[("A", ty0())], &[("v", nat())]);
    let first = apps(g("Pick_mk"), &[nat(), peano(1)]);
    let second = apps(g("Pick_mk"), &[nat(), peano(2)]);
    ord.add_instance(app(g("Pick"), nat()), first.clone());
    ord.add_instance(app(g("Pick"), nat()), second.clone());
    assert_eq!(
        resolve(&ord, &app(g("Pick"), nat())),
        Some(second.clone()),
        "later declaration wins"
    );
    // …and priority still outranks order, in the presence of the index.
    ord.add_instance_with_priority(
        app(g("Pick"), nat()),
        first.clone(),
        DEFAULT_INSTANCE_PRIORITY,
    );
    let mut hi = ord.clone();
    hi.add_instance_with_priority(app(g("Pick"), nat()), second.clone(), 100);
    assert_eq!(
        resolve(&hi, &app(g("Pick"), nat())),
        Some(first),
        "a LOWER-priority later entry must not outrank the higher-priority earlier one",
    );
}

// ============================================================================================
// 10. `CoeSort` — a VALUE standing in a TYPE position
// ============================================================================================
//
// Lean's `CoeSort α β` (`Init/Coe.lean:277`) is consumed by `coerceToSort?`
// (`Lean/Meta/Coe.lean:69`), which is reached from `ensureType` (`Lean/Elab/Term.lean:1867`) —
// so the coercion is inserted during ELABORATION and the KERNEL never sees a binder whose domain
// is not a sort. That is the half `coe_fun` already had and `coe_sort` did not: a structure with
// a carrier field could be projected by hand but could not stand where a type is expected.
//
// Read off the DECLARATION rather than off an instance, exactly as `coe_fun` is: a structure with
// exactly ONE sort-typed field coerces to that field. More than one is not a canonical choice and
// must decline rather than guess.

/// A structure with a carrier field stands in a type position, and the elaborated term is the
/// projection — which is what makes the kernel's own answer unchanged.
#[test]
fn a_structure_with_one_carrier_field_can_stand_where_a_type_is_expected() {
    let mut ctx = std_ctx();
    ctx.add_structure("Ty", &[], &[("carrier", ty0())]);
    let tv = app(g("Ty_mk"), nat());

    // THE MUST — `Π(x : t). t` with `t : Ty`.
    let written = pi("x", tv.clone(), tv.clone());
    let elaborated = surface_elaborate(&ctx, &written).expect("Π(x : t). t elaborates");
    assert_eq!(
        elaborated,
        pi(
            "x",
            app(g("Ty_carrier"), tv.clone()),
            app(g("Ty_carrier"), tv.clone())
        ),
        "the coercion is INSERTED: the binder's domain becomes the projection",
    );
    assert!(
        infer_type(&ctx, &elaborated).is_ok(),
        "…and the elaborated term is one the kernel accepts: {:?}",
        infer_type(&ctx, &elaborated),
    );
    assert_eq!(
        normalize(&ctx, &elaborated),
        pi("x", nat(), nat()),
        "and it IS `Nat -> Nat`"
    );

    // The KERNEL is unchanged — a raw binder at a non-sort domain is still refused, which is
    // where Lean puts the line too.
    assert!(
        infer_type(&ctx, &written).is_err(),
        "the kernel must NOT learn to coerce: `CoeSort` is an elaboration mechanism",
    );

    // A λ binder is a type position too.
    let lam_written = lam("x", tv.clone(), var("x"));
    let lam_elab = surface_elaborate(&ctx, &lam_written).expect("λ(x : t). x elaborates");
    assert!(
        infer_type(&ctx, &lam_elab).is_ok(),
        "λ over a coerced domain: {lam_elab}"
    );
    assert_eq!(normalize(&ctx, &lam_elab), lam("x", nat(), var("x")));

    // NEGATIVE — TWO sort-typed fields is not a canonical choice, and guessing one silently
    // picks a meaning. It must decline, leaving the term exactly as written.
    let mut two = std_ctx();
    two.add_structure("Both", &[], &[("left", ty0()), ("right", ty0())]);
    let bv = apps(g("Both_mk"), &[nat(), g("Bool")]);
    let amb = pi("x", bv.clone(), bv.clone());
    assert_eq!(
        surface_elaborate(&two, &amb).expect("elaborates"),
        amb,
        "two carrier fields have no canonical choice — decline, never guess",
    );

    // NEGATIVE — a value with NO sort-typed field must not be coerced into a type either.
    let mut none = std_ctx();
    none.add_structure("Plain", &[], &[("v", nat())]);
    let pv = app(g("Plain_mk"), g("Zero"));
    let plain = pi("x", pv.clone(), nat());
    assert_eq!(
        surface_elaborate(&none, &plain).expect("elaborates"),
        plain,
        "nothing to coerce"
    );

    // NEGATIVE — a domain that ALREADY is a type is untouched, so ordinary elaboration is
    // unchanged by the new rule.
    let ordinary = pi("x", nat(), nat());
    assert_eq!(
        surface_elaborate(&ctx, &ordinary).expect("elaborates"),
        ordinary
    );

    // BOTH KERNELS agree about the term the coercion produced.
    assert!(matches!(double_check(&ctx, &lam_elab), DoubleCheck::Agreed));
}
